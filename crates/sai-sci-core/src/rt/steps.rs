//! research-tree の時系列（`status.steps`）。成功した op 1 回を 1 step とし、`n` は 1 から振る。
//!
//! 正は `rt_nodes`（作成・消去の時刻と親）と `rt_log`（`move_a`・`move_node`・`insert_q`・`regroup`・`copy`・`remove` の記録）。
//! `add_a`・`add_f`・`add_e`・`add_sprout`・`add_seed` は `rt_log` に書かないので、作成は `rt_nodes` の行から作る（`add_q` は書く）。
//! 本文・★・stance の変更は step に数えず、再生もしない。

use std::collections::{BTreeMap, HashMap};

use rusqlite::Connection;
use serde::Serialize;

use super::compute::{Counts, NodeJson, rfc3339_utc};
use super::store::{self, LogRow, NodeRow};
use super::{Kind, Result, fmt_id, parse_id_of};

/// `status.steps`。
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct StepsJson {
    /// step の数（`steps` の長さ）。
    pub total: usize,
    /// 1 番目の要素が step 1。
    pub list: Vec<StepJson>,
    /// 全 Node（消えた Node も）。ID の数の昇順。
    pub nodes: Vec<NodeStepJson>,
    /// 消えた Node の中身。ID の数の昇順。
    pub removed: Vec<NodeJson>,
}

/// 1 step。`node` は作られた・動いた・消えた Node の ID（消去は根の Node）。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct StepJson {
    pub n: usize,
    pub at: String,
    pub op: String,
    pub by: String,
    pub node: String,
}

/// 1 Node の一生。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct NodeStepJson {
    pub id: String,
    /// 生まれた step の n。
    pub born: usize,
    /// 消えた step の n。生きていれば null。
    pub gone: Option<usize>,
    /// 親の履歴。先頭は {step: born, parent: 生まれたときの親}。その後は付け替えのあった step の昇順。
    pub parents: Vec<ParentAtJson>,
}

/// `step` から親が `parent` になった。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ParentAtJson {
    pub step: usize,
    /// その step 以降の親の ID。top_Q・D・親の無い S は null。
    pub parent: Option<String>,
}

/// 同じ秒の中の出来事。`at` は DB の文字のまま比べる（秒の単位）。`rank` は 作成 0・move_a 1・消去 2。
struct Event<'a> {
    at: &'a str,
    rank: u8,
    tie: i64,
    what: What<'a>,
}

enum What<'a> {
    Create(&'a NodeRow),
    Move(&'a LogRow),
    /// 同じ `removed_at` の Node（部分木）。
    Remove(Vec<&'a NodeRow>),
}

/// 付け替え 1 回（`insert_q` の作成も含む）。`exact` は詳細に残った移した A の ID。無ければ `count` の件数で推測する。
struct Move {
    n: usize,
    dest: i64,
    from: i64,
    exact: Option<Vec<i64>>,
    count: Option<usize>,
}

/// `status.steps` を作る。始めている research-tree でだけ呼ぶ。
pub fn build(conn: &Connection) -> Result<StepsJson> {
    let nodes = store::list_nodes(conn, true)?;
    let log = store::list_log(conn, None)?;
    let kinds: HashMap<i64, Kind> = nodes.iter().map(|n| (n.id, n.kind)).collect();
    // 作成の記録（insert_q・copy・regroup）と消去の記録を、Node の ID で引けるようにする。
    // regroup の記録は細分の Q に付く。その親（中間の A）は regroup の作成になる。
    let made_by: HashMap<i64, &LogRow> = log
        .iter()
        .filter(|l| matches!(l.action.as_str(), "insert_q" | "copy" | "regroup"))
        .map(|l| (l.node_id, l))
        .collect();
    let parent_of: HashMap<i64, Option<i64>> = nodes.iter().map(|n| (n.id, n.parent_id)).collect();
    let regroup_a: HashMap<i64, &LogRow> = log
        .iter()
        .filter(|l| l.action == "regroup")
        .filter_map(|l| parent_of.get(&l.node_id).copied().flatten().map(|a| (a, l)))
        .collect();
    let removed_by: HashMap<i64, &LogRow> = log
        .iter()
        .filter(|l| l.action == "remove")
        .map(|l| (l.node_id, l))
        .collect();

    let mut events: Vec<Event<'_>> = nodes
        .iter()
        .map(|n| Event {
            at: n.created_at.as_str(),
            rank: 0,
            tie: n.id,
            what: What::Create(n),
        })
        .collect();
    events.extend(log.iter().filter(|l| matches!(l.action.as_str(), "move_a" | "move_node")).map(|l| Event {
        at: l.created_at.as_str(),
        rank: 1,
        tie: l.id,
        what: What::Move(l),
    }));
    // ponytail: 消去は removed_at の同じ値を 1 step にまとめる。同じ秒に 2 回消すと 1 step になる。秒より細かい時刻を持てば直る。
    let mut groups: BTreeMap<&str, Vec<&NodeRow>> = BTreeMap::new();
    for n in &nodes {
        if let Some(at) = n.removed_at.as_deref() {
            groups.entry(at).or_default().push(n);
        }
    }
    events.extend(groups.into_iter().map(|(at, ns)| Event {
        at,
        rank: 2,
        tie: ns.iter().map(|n| n.id).min().unwrap_or_default(),
        what: What::Remove(ns),
    }));
    events.sort_by(|a, b| (a.at, a.rank, a.tie).cmp(&(b.at, b.rank, b.tie)));

    let mut steps = Vec::with_capacity(events.len());
    let mut born: HashMap<i64, usize> = HashMap::new();
    let mut gone: HashMap<i64, usize> = HashMap::new();
    let mut moves: Vec<Move> = Vec::new();
    for ev in events.iter() {
        // regroup は中間の A と細分の Q を作るが、1 op = 1 step。細分の Q は直前の A の step に畳み、step は増やさない。
        let folded = matches!(&ev.what, What::Create(row)
            if row.kind == Kind::Q && made_by.get(&row.id).is_some_and(|l| l.action == "regroup"));
        let n = if folded { steps.len() } else { steps.len() + 1 };
        let (op, by, node) = match &ev.what {
            What::Create(row) => {
                born.insert(row.id, n);
                let made = made_by.get(&row.id).map(|l| l.action.as_str());
                let op = match made {
                    Some("insert_q") => "insert_q",
                    Some("copy") => "copy",
                    Some("regroup") => "regroup",
                    _ => match row.kind {
                        Kind::Q => "add_q",
                        Kind::A if regroup_a.contains_key(&row.id) => "regroup",
                        Kind::A => "add_a",
                        Kind::F => "add_f",
                        Kind::E => "add_e",
                        Kind::S => "add_sprout",
                        Kind::D => "add_seed",
                    },
                };
                if let Some(l) = made_by.get(&row.id).filter(|l| matches!(l.action.as_str(), "insert_q" | "regroup")) {
                    moves.extend(read_move(&l.detail).map(|(from, exact, count)| Move {
                        n,
                        dest: row.id,
                        from,
                        exact,
                        count,
                    }));
                }
                (op, row.created_by.clone(), fmt_id(row.kind, row.id))
            }
            What::Move(l) => {
                moves.extend(read_move_log(l).map(|(dest, from, exact, count)| Move { n, dest, from, exact, count }));
                if l.action == "move_node" {
                    ("move_node", l.actor.clone(), fmt_id(Kind::A, l.node_id))
                } else {
                    ("move_a", l.actor.clone(), fmt_id(Kind::Q, l.node_id))
                }
            }
            What::Remove(ns) => {
                for m in ns {
                    gone.insert(m.id, n);
                }
                let (node, by) = match ns.iter().find_map(|m| removed_by.get(&m.id)) {
                    Some(l) => (l.node_id, l.actor.clone()),
                    None => (
                        ns.iter().map(|m| m.id).min().unwrap_or_default(),
                        String::new(),
                    ),
                };
                ("remove", by, fmt_id(kinds[&node], node))
            }
        };
        if !folded {
            steps.push(StepJson {
                n,
                at: rfc3339_utc(ev.at),
                op: op.to_string(),
                by,
                node,
            });
        }
    }

    // 親は今の値から時間をさかのぼって求める。付け替えは新しい step から順に戻す。
    let mut cur: HashMap<i64, Option<i64>> = nodes.iter().map(|n| (n.id, n.parent_id)).collect();
    let mut hist: HashMap<i64, Vec<ParentAtJson>> = HashMap::new();
    moves.sort_by_key(|m| std::cmp::Reverse(m.n));
    for m in &moves {
        let moved = match &m.exact {
            Some(ids) => ids.clone(),
            None => {
                // ponytail: 今までの記録は移した ID が無い。移動先にもとからあった A と取り違えうる。ID の小さい順に件数だけ取る（推測）。
                let mut found: Vec<i64> = nodes
                    .iter()
                    .filter(|a| {
                        a.kind == Kind::A
                            && cur[&a.id] == Some(m.dest)
                            && born[&a.id] < m.n
                            && gone.get(&a.id).is_none_or(|g| *g > m.n)
                    })
                    .map(|a| a.id)
                    .collect();
                found.sort_unstable();
                found.truncate(m.count.unwrap_or(usize::MAX));
                found
            }
        };
        for a in moved {
            hist.entry(a).or_default().push(ParentAtJson {
                step: m.n,
                parent: Some(fmt_id(Kind::Q, m.dest)),
            });
            cur.insert(a, Some(m.from));
        }
    }

    let parent_of = |p: Option<i64>| p.and_then(|p| kinds.get(&p).map(|k| fmt_id(*k, p)));
    let mut order: Vec<&NodeRow> = nodes.iter().collect();
    order.sort_by_key(|n| n.id);
    let mut node_steps = Vec::with_capacity(order.len());
    for n in &order {
        let mut moved = hist.remove(&n.id).unwrap_or_default();
        moved.sort_by_key(|p| p.step);
        let mut parents = vec![ParentAtJson {
            step: born[&n.id],
            parent: parent_of(cur[&n.id]),
        }];
        parents.extend(moved);
        node_steps.push(NodeStepJson {
            id: fmt_id(n.kind, n.id),
            born: born[&n.id],
            gone: gone.get(&n.id).copied(),
            parents,
        });
    }
    let removed = order
        .iter()
        .copied()
        .filter(|n| n.removed_at.is_some())
        .map(|n| removed_json(n, &kinds))
        .collect();
    Ok(StepsJson {
        total: steps.len(),
        list: steps,
        nodes: node_steps,
        removed,
    })
}

/// 移動の記録を読む（`(移し先の Q, 元の Q, 移した A, 件数)`）。`move_node` は移した A が `node_id` で、移し先は detail の「Q7 から Q30 へ」。
/// `move_a` は移し先が `node_id`（detail は [`read_move`]）。
fn read_move_log(l: &LogRow) -> Option<(i64, i64, Option<Vec<i64>>, Option<usize>)> {
    if l.action == "move_node" {
        let words: Vec<&str> = l.detail.split_whitespace().collect();
        let from = parse_id_of(words.first()?, Kind::Q).ok()?;
        let dest = parse_id_of(words.get(2)?, Kind::Q).ok()?;
        return Some((dest, from, Some(vec![l.node_id]), None));
    }
    let (from, exact, count) = read_move(&l.detail)?;
    Some((l.node_id, from, exact, count))
}

/// `rt_log.detail` を読む。先頭の語が元の Q。`（A5, A6）` があれば移した A の ID、無ければ `A 2 件` の件数。
fn read_move(detail: &str) -> Option<(i64, Option<Vec<i64>>, Option<usize>)> {
    let from = parse_id_of(detail.split_whitespace().next()?, Kind::Q).ok()?;
    let exact = detail
        .rsplit_once('（')
        .and_then(|(_, rest)| rest.strip_suffix('）'))
        .and_then(|ids| {
            ids.split(", ")
                .map(|t| parse_id_of(t, Kind::A))
                .collect::<Result<Vec<_>>>()
                .ok()
        });
    let words: Vec<&str> = detail.split_whitespace().collect();
    let count = words
        .windows(2)
        .find(|w| w[0] == "A")
        .and_then(|w| w[1].parse().ok());
    Some((from, exact, count))
}

/// 消えた Node の中身（`NodeJson`）。`depth`・`bottom_q`・`range`・`counts` は消えた Node では意味が無いので空にする。
fn removed_json(n: &NodeRow, kinds: &HashMap<i64, Kind>) -> NodeJson {
    NodeJson {
        id: fmt_id(n.kind, n.id),
        kind: n.kind.as_str().to_string(),
        parent: n
            .parent_id
            .and_then(|p| kinds.get(&p).map(|k| fmt_id(*k, p))),
        depth: 0,
        ord: n.ord,
        text: n.text.clone(),
        body: n.body.clone(),
        guide: if n.kind == Kind::Q {
            n.guide_key.clone()
        } else {
            None
        },
        stance: n.stance.clone(),
        starred: n.starred,
        bottom_q: false,
        paths: if n.kind == Kind::S {
            n.paths.iter().map(|a| fmt_id(Kind::A, *a)).collect()
        } else {
            Vec::new()
        },
        range: None,
        counts: Counts::default(),
        cites: n.cites.clone(),
        paper: n.paper_id,
        run: n.run_id,
        mlflow: n.mlflow_ref.clone(),
        dataset: n.dataset_ref.clone(),
        created_by: n.created_by.clone(),
        created_at: n.created_at.clone(),
        // 消えた Node の問いと区分は、問いの表（消えた D の問い）と A の位置が要るので出さない（空）。
        questions: Vec::new(),
        complete: None,
        missing: Vec::new(),
        path_roles: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::store::NewNode;
    use super::*;
    use rusqlite::params;
    use sai_paper_core::database::Database;

    const T0: &str = "2026-10-01 00:00:00";
    const T5: &str = "2026-10-01 00:00:05";

    fn db() -> Database {
        Database::in_memory().expect("メモリ DB")
    }

    /// Node を 1 つ作り、作った時刻を直接書く（テストの時刻を決めるため）。
    fn add(conn: &Connection, kind: Kind, parent: Option<i64>, at: &str) -> i64 {
        let stance = (kind == Kind::F).then_some("support");
        let node = NewNode {
            kind,
            parent_id: parent,
            guide_key: None,
            text: "",
            body: "",
            stance,
            cites: &[],
            paper_id: None,
            run_id: None,
            report_id: None,
            mlflow_ref: None,
            dataset_ref: None,
            paths: &[],
            created_by: "user",
        };
        let id = store::insert_node(conn, &node).unwrap();
        conn.execute(
            "UPDATE rt_nodes SET created_at = ?2 WHERE id = ?1",
            params![id, at],
        )
        .unwrap();
        id
    }

    /// `rt_log` に 1 行入れる（時刻は直接書く。INSERT はトリガで止まらない）。
    fn log(conn: &Connection, node: i64, action: &str, detail: &str, actor: &str, at: &str) {
        conn.execute(
            "INSERT INTO rt_log (node_id, action, detail, actor, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![node, action, detail, actor, at],
        )
        .unwrap();
    }

    fn q(id: i64) -> Option<String> {
        Some(fmt_id(Kind::Q, id))
    }

    /// 親の履歴を (step, 親の ID) の組で返す。
    fn hist(st: &StepsJson, kind: Kind, id: i64) -> Vec<(usize, Option<String>)> {
        let want = fmt_id(kind, id);
        let n = st.nodes.iter().find(|n| n.id == want).unwrap();
        n.parents
            .iter()
            .map(|p| (p.step, p.parent.clone()))
            .collect()
    }

    /// Q1 の下に A2・A3。insert_q で Q4 を挟み、A2・A3 を Q4 へ移した DB。詳細に ID を入れるかどうかだけ変える。
    fn insert_q_db(with_ids: bool) -> (Database, [i64; 4]) {
        let database = db();
        let conn = database.connection();
        let q1 = add(conn, Kind::Q, None, T0);
        let a2 = add(conn, Kind::A, Some(q1), T0);
        let a3 = add(conn, Kind::A, Some(q1), T0);
        let q4 = add(conn, Kind::Q, Some(q1), T0);
        store::set_parent(conn, a2, q4).unwrap();
        store::set_parent(conn, a3, q4).unwrap();
        let head = format!("{} の A 2 件を移した", fmt_id(Kind::Q, q1));
        let detail = if with_ids {
            format!("{head}（{}, {}）", fmt_id(Kind::A, a2), fmt_id(Kind::A, a3))
        } else {
            head
        };
        log(conn, q4, "insert_q", &detail, "user", T0);
        (database, [q1, a2, a3, q4])
    }

    #[test]
    fn creation_steps_follow_time_then_id_with_mixed_kinds() {
        let database = db();
        let conn = database.connection();
        let q1 = add(conn, Kind::Q, None, T0);
        let a2 = add(conn, Kind::A, Some(q1), T0);
        let f3 = add(conn, Kind::F, Some(a2), T0);
        let e4 = add(conn, Kind::E, Some(a2), T0);
        let s5 = add(conn, Kind::S, None, "2026-10-01 00:00:01");
        let d6 = add(conn, Kind::D, None, "2026-10-01 00:00:01");
        let st = build(conn).unwrap();
        assert_eq!(st.total, 6);
        let ops: Vec<&str> = st.list.iter().map(|s| s.op.as_str()).collect();
        assert_eq!(
            ops,
            ["add_q", "add_a", "add_f", "add_e", "add_sprout", "add_seed"]
        );
        let want: Vec<String> = [
            (Kind::Q, q1),
            (Kind::A, a2),
            (Kind::F, f3),
            (Kind::E, e4),
            (Kind::S, s5),
            (Kind::D, d6),
        ]
        .iter()
        .map(|(k, id)| fmt_id(*k, *id))
        .collect();
        assert_eq!(
            st.nodes.iter().map(|n| n.id.clone()).collect::<Vec<_>>(),
            want
        );
        assert_eq!(
            st.nodes.iter().map(|n| n.born).collect::<Vec<_>>(),
            vec![1, 2, 3, 4, 5, 6]
        );
    }

    #[test]
    fn insert_q_is_one_step_and_moves_parent_of_moved_answers() {
        // 新しい形（ID あり）と、今までの形（件数だけ）の両方で同じ履歴になる。
        for with_ids in [true, false] {
            let (database, [q1, a2, a3, q4]) = insert_q_db(with_ids);
            let st = build(database.connection()).unwrap();
            assert_eq!(st.total, 4);
            assert_eq!(st.list[3].op, "insert_q");
            assert_eq!(st.list[3].node, fmt_id(Kind::Q, q4));
            assert_eq!(hist(&st, Kind::A, a2), vec![(2, q(q1)), (4, q(q4))]);
            assert_eq!(hist(&st, Kind::A, a3), vec![(3, q(q1)), (4, q(q4))]);
            assert_eq!(hist(&st, Kind::Q, q4), vec![(4, q(q1))]);
        }
    }

    #[test]
    fn move_a_is_one_step_and_uses_moved_ids() {
        let database = db();
        let conn = database.connection();
        let q1 = add(conn, Kind::Q, None, T0);
        let a2 = add(conn, Kind::A, Some(q1), T0);
        let a3 = add(conn, Kind::A, Some(q1), T0);
        let q5 = add(conn, Kind::Q, None, T0);
        store::set_parent(conn, a2, q5).unwrap();
        store::set_parent(conn, a3, q5).unwrap();
        let detail = format!(
            "{} から A 2 件（{}, {}）",
            fmt_id(Kind::Q, q1),
            fmt_id(Kind::A, a2),
            fmt_id(Kind::A, a3)
        );
        log(conn, q5, "move_a", &detail, "user", T5);
        let st = build(conn).unwrap();
        assert_eq!(st.total, 5);
        assert_eq!(st.list[4].op, "move_a");
        assert_eq!(st.list[4].node, fmt_id(Kind::Q, q5));
        assert_eq!(hist(&st, Kind::A, a2), vec![(2, q(q1)), (5, q(q5))]);
        assert_eq!(hist(&st, Kind::A, a3), vec![(3, q(q1)), (5, q(q5))]);
    }

    #[test]
    fn move_a_without_ids_guesses_smallest_ids_at_destination() {
        // 今までの形の限界を固定する。移動先の q5 にもとからある a_pre（ID が小さい）を、移した a_mv と取り違える。
        // 正しくは a_pre は (2, q5) のまま、a_mv は (4, q1)→(5, q5)。
        let database = db();
        let conn = database.connection();
        let q5 = add(conn, Kind::Q, None, T0);
        let a_pre = add(conn, Kind::A, Some(q5), T0);
        let q1 = add(conn, Kind::Q, None, T0);
        let a_mv = add(conn, Kind::A, Some(q1), T0);
        store::set_parent(conn, a_mv, q5).unwrap();
        log(
            conn,
            q5,
            "move_a",
            &format!("{} から A 1 件", fmt_id(Kind::Q, q1)),
            "user",
            T5,
        );
        let st = build(conn).unwrap();
        assert_eq!(hist(&st, Kind::A, a_pre), vec![(2, q(q1)), (5, q(q5))]);
        assert_eq!(hist(&st, Kind::A, a_mv), vec![(4, q(q5))]);
    }

    #[test]
    fn remove_is_one_step_for_whole_subtree() {
        let database = db();
        let conn = database.connection();
        let q1 = add(conn, Kind::Q, None, T0);
        let a2 = add(conn, Kind::A, Some(q1), T0);
        let f3 = add(conn, Kind::F, Some(a2), T0);
        let _q4 = add(conn, Kind::Q, None, T0);
        store::remove_subtree(conn, a2).unwrap();
        log(conn, a2, "remove", "", "saituki", T0);
        let st = build(conn).unwrap();
        assert_eq!(st.total, 5);
        assert_eq!(st.list[4].op, "remove");
        assert_eq!(st.list[4].by, "saituki");
        assert_eq!(st.list[4].node, fmt_id(Kind::A, a2));
        // nodes は q1・a2・f3・q4 の順。消えた a2 と f3 は同じ step、q1 と q4 は生きたまま。
        assert_eq!(
            st.nodes.iter().map(|n| n.gone).collect::<Vec<_>>(),
            vec![None, Some(5), Some(5), None]
        );
        assert_eq!(
            st.removed.iter().map(|n| n.id.clone()).collect::<Vec<_>>(),
            vec![fmt_id(Kind::A, a2), fmt_id(Kind::F, f3)]
        );
        assert_eq!(st.removed[0].parent, q(q1));
    }

    #[test]
    fn same_second_is_create_then_move_then_remove() {
        let database = db();
        let conn = database.connection();
        let q1 = add(conn, Kind::Q, None, T0);
        let a2 = add(conn, Kind::A, Some(q1), T0);
        let q5 = add(conn, Kind::Q, None, T0);
        let x = add(conn, Kind::Q, None, T0);
        store::set_parent(conn, a2, q5).unwrap();
        let detail = format!(
            "{} から A 1 件（{}）",
            fmt_id(Kind::Q, q1),
            fmt_id(Kind::A, a2)
        );
        log(conn, q5, "move_a", &detail, "user", T0);
        // 同じ秒に消した。消去は作成と移動の後に並ぶ。
        conn.execute(
            "UPDATE rt_nodes SET removed_at = ?2 WHERE id = ?1",
            params![x, T0],
        )
        .unwrap();
        log(conn, x, "remove", "", "user", T0);
        let st = build(conn).unwrap();
        let ops: Vec<&str> = st.list.iter().map(|s| s.op.as_str()).collect();
        assert_eq!(
            ops,
            ["add_q", "add_a", "add_q", "add_q", "move_a", "remove"]
        );
        assert_eq!(hist(&st, Kind::A, a2), vec![(2, q(q1)), (5, q(q5))]);
    }
}
