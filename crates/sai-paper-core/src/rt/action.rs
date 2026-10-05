//! ブロックの Action（task.md を作って実行した run）と、その完了基準・DEBT（C-ACT）。
//!
//! DEBT は「Action の完了基準を満たしていない run」。基準は Agent（researcher の task そのものの完了）と
//! User（そのあとに利用者がすること）の 2 種。釦で動く操作（取得・翻訳・Radio）には付けない。

use std::fs;
use std::path::PathBuf;
use std::time::SystemTime;

use chrono::{DateTime, Local, NaiveDateTime, TimeZone, Utc};
use rusqlite::{Connection, params};
use serde::Serialize;
use serde_json::{Value, json};

use super::store;
use super::{Kind, Result, RtKeys, fmt_id, task_keys};
use crate::paths::{PaperRoots, paper_file_names, paper_folder};
use crate::runs::{list_run_dirs, read_frontmatter, run_block_id};

/// ブロック定義の置き場の候補（`<root>/.sai/blocks/<id>.md` → `<sai_home>/blocks/<id>.md`）。id が `^[a-z0-9-]+$` でなければ空。
#[must_use]
pub fn block_def_paths(roots: &PaperRoots, block: &str) -> Vec<PathBuf> {
    let valid = !block.is_empty()
        && block
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
    if !valid {
        return Vec::new();
    }
    let mut candidates = vec![roots.root.join(".sai").join("blocks").join(format!("{block}.md"))];
    if let Some(dir) = crate::paths::sai_home_path("blocks") {
        candidates.push(dir.join(format!("{block}.md")));
    }
    candidates
}

/// 最初に見つかったブロック定義の frontmatter。
#[must_use]
pub fn block_frontmatter(roots: &PaperRoots, block: &str) -> Option<Vec<(String, String)>> {
    block_def_paths(roots, block)
        .into_iter()
        .find_map(|path| fs::read_to_string(path).ok())
        .map(|text| read_frontmatter(&text))
}

/// `sci_action` の値。
pub const ACTIONS: [&str; 7] = [
    "split",
    "write_o",
    "find_f",
    "check_logic",
    "choose",
    "experiment",
    "other",
];
/// 完了基準の名前（判定と DEBT の並びはこの順）。
pub const CRITERIA: [&str; 13] = [
    "report",
    "review",
    "o_added",
    "f_added",
    "e_added",
    "logic_recorded",
    "q_proposed",
    "seed_created",
    "set_created",
    "set_screened",
    "note",
    "triaged",
    "starred",
];

/// ブロック定義から読んだ Action と完了基準。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct BlockAction {
    pub action: String,
    pub support: bool,
    pub done_agent: Vec<String>,
    pub done_user: Vec<String>,
}

fn split_list(text: &str) -> Vec<String> {
    text.split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

/// ブロックの Action と完了基準。定義が無い・キーが無いときは既定（`other`・`report, review`・`triaged`）。
#[must_use]
pub fn block_action(roots: &PaperRoots, block: &str) -> BlockAction {
    let pairs = block_frontmatter(roots, block).unwrap_or_default();
    let get = |name: &str| pairs.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str());
    BlockAction {
        action: get("sci_action").filter(|v| !v.is_empty()).unwrap_or("other").to_string(),
        support: get("sci_support").is_some_and(|v| v == "true"),
        done_agent: get("sci_done_agent").map_or_else(|| vec!["report".into(), "review".into()], split_list),
        done_user: get("sci_done_user").map_or_else(|| vec!["triaged".into()], split_list),
    }
}

/// ブロック定義の書き間違い（知らない値）を 1 行ずつ返す。
#[must_use]
pub fn block_warnings(roots: &PaperRoots) -> Vec<String> {
    let mut dirs = vec![roots.root.join(".sai").join("blocks")];
    dirs.extend(crate::paths::sai_home_path("blocks"));
    let mut out = Vec::new();
    for dir in dirs {
        let Ok(entries) = fs::read_dir(dir) else { continue };
        let mut files: Vec<PathBuf> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == "md"))
            .collect();
        files.sort();
        for path in files {
            let Ok(text) = fs::read_to_string(&path) else { continue };
            let name = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
            for (key, value) in read_frontmatter(&text) {
                match key.as_str() {
                    "sci_action" if !ACTIONS.contains(&value.as_str()) => {
                        out.push(format!("{name}: sci_action に知らない値「{value}」があります"));
                    }
                    "sci_done_agent" | "sci_done_user" => {
                        for item in split_list(&value).iter().filter(|i| !CRITERIA.contains(&i.as_str())) {
                            out.push(format!("{name}: {key} に知らない完了基準「{item}」があります"));
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    out
}

/// task.md に sci キーがある run。
#[derive(Debug, Clone)]
pub struct SciRun {
    pub run_id: i64,
    pub owner: String,
    pub block: String,
    pub runner: String,
    pub keys: RtKeys,
    /// RFC3339。
    pub occurred_at: String,
    /// task.md の `global_round`。空や数でなければ `None`。
    pub global_round: Option<i64>,
    pub path: PathBuf,
}

fn parse_local(text: &str) -> Option<DateTime<Utc>> {
    let naive = NaiveDateTime::parse_from_str(text.trim(), "%Y/%m/%d %H:%M:%S").ok()?;
    Some(Local.from_local_datetime(&naive).earliest()?.with_timezone(&Utc))
}

/// sci の run（新しい順）。
#[must_use]
pub fn sci_runs(roots: &PaperRoots) -> Vec<SciRun> {
    let mut out = Vec::new();
    for run in list_run_dirs(&roots.root) {
        let task_path = run.path.join("task.md");
        let Ok(text) = fs::read_to_string(&task_path) else { continue };
        let keys = task_keys(&text);
        if !keys.is_sci() {
            continue;
        }
        let pairs = read_frontmatter(&text);
        let get = |name: &str| pairs.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str());
        let when = get("execution_date")
            .and_then(parse_local)
            .or_else(|| get("creation_date").and_then(parse_local))
            .or_else(|| {
                let modified = fs::metadata(&task_path).and_then(|m| m.modified()).unwrap_or(SystemTime::UNIX_EPOCH);
                Some(DateTime::<Utc>::from(modified))
            })
            .unwrap_or_default();
        out.push(SciRun {
            run_id: run.run_id,
            owner: run.owner.clone(),
            block: run_block_id(&roots.root, &run).unwrap_or_default(),
            runner: get("runner").filter(|v| !v.is_empty()).unwrap_or("pi").to_string(),
            keys,
            occurred_at: when.to_rfc3339(),
            global_round: get("global_round").and_then(|v| v.trim().parse::<i64>().ok()),
            path: run.path,
        });
    }
    out
}

/// 満たしていない完了基準 1 つ分。
#[derive(Debug, Clone, Serialize)]
pub struct Debt {
    pub run_id: i64,
    pub block: String,
    pub node: Option<String>,
    /// "agent" | "user"
    pub side: String,
    pub criterion: String,
    pub message: String,
    pub fix: Value,
}

fn count(conn: &Connection, sql: &str, run_id: i64) -> i64 {
    conn.query_row(sql, params![run_id], |r| r.get(0)).unwrap_or(0)
}

fn card_fix(run: &SciRun) -> Value {
    let k = &run.keys;
    let mut card = serde_json::Map::new();
    if let Some((kind, id)) = k.node {
        card.insert("node".into(), json!(fmt_id(kind, id)));
    }
    card.insert("block".into(), json!(run.block));
    card.insert("purpose".into(), json!(k.purpose));
    if !k.expect.is_empty() {
        card.insert("expect".into(), json!(k.expect));
    }
    card.insert("runner".into(), json!(run.runner));
    if !k.papers.is_empty() {
        card.insert("papers".into(), json!(k.papers));
    }
    json!({"kind": "card", "card": Value::Object(card)})
}

/// 基準 1 つの判定。満たしていなければ (message, fix) を返す（複数あり得る）。
fn check(conn: &Connection, roots: &PaperRoots, run: &SciRun, name: &str) -> Vec<(String, Value)> {
    let id = run.run_id;
    let run_fix = || json!({"kind": "run", "run_id": id});
    let miss = |ok: bool, msg: &str, fix: Value| if ok { vec![] } else { vec![(msg.to_string(), fix)] };
    let alive = |kind: &str| {
        count(
            conn,
            &format!("SELECT COUNT(*) FROM rt_nodes WHERE run_id = ?1 AND kind = '{kind}' AND removed_at IS NULL"),
            id,
        ) > 0
    };
    match name {
        "report" => {
            let ok = fs::read_dir(&run.path).is_ok_and(|entries| {
                entries.flatten().any(|e| {
                    let file = e.file_name().to_string_lossy().to_string();
                    file.ends_with(".md")
                        && file != "task.md"
                        && file != "review.md"
                        && fs::read_to_string(e.path()).is_ok_and(|t| {
                            read_frontmatter(&t).iter().any(|(k, v)| k == "description" && !v.is_empty())
                        })
                })
            });
            miss(ok, "報告書（description 付き）がありません", run_fix())
        }
        "review" => {
            let ok = fs::read_to_string(run.path.join("review.md")).is_ok_and(|t| {
                crate::attention::review_has_content(crate::attention::strip_frontmatter(&t))
            });
            miss(ok, "振り返り（review.md）が書かれていません", run_fix())
        }
        "o_added" => miss(alive("O"), "O が 1 つも入っていません", card_fix(run)),
        "f_added" => miss(alive("F"), "F が 1 つも入っていません", card_fix(run)),
        "e_added" => miss(alive("E"), "E が 1 つも入っていません", card_fix(run)),
        "logic_recorded" => miss(
            count(
                conn,
                "SELECT COUNT(*) FROM rt_logic l LEFT JOIN rt_proposals p ON p.id = l.proposal_id \
                 WHERE l.run_id = ?1 AND (p.status IS NULL OR p.status != 'reverted')",
                id,
            ) > 0,
            "Logicality が記録されていません",
            card_fix(run),
        ),
        "q_proposed" => miss(
            count(conn, "SELECT COUNT(*) FROM rt_proposals WHERE run_id = ?1 AND op = 'add_q'", id) > 0,
            "Q の案が出ていません",
            card_fix(run),
        ),
        "seed_created" => miss(
            count(conn, "SELECT COUNT(*) FROM rt_seeds WHERE run_id = ?1 AND removed_at IS NULL", id) > 0,
            "SEED が作られていません",
            card_fix(run),
        ),
        "set_created" => miss(
            count(conn, "SELECT COUNT(*) FROM rt_searches WHERE run_id = ?1 AND set_id IS NOT NULL", id) > 0,
            "検索セットが作られていません",
            card_fix(run),
        ),
        "set_screened" => {
            let sql = "SELECT s.id, s.name, COUNT(i.set_id) FROM search_sets s \
                       JOIN search_set_items i ON i.set_id = s.id AND i.decision = 'pending' \
                       WHERE s.id IN (SELECT set_id FROM rt_searches WHERE run_id = ?1 AND set_id IS NOT NULL) \
                       GROUP BY s.id ORDER BY s.id";
            let Ok(mut stmt) = conn.prepare(sql) else { return vec![] };
            let rows = stmt.query_map(params![id], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)?)));
            rows.map(|rows| {
                rows.flatten()
                    .map(|(set_id, name, n)| {
                        (format!("検索セット「{name}」に未選別 {n} 件"), json!({"kind": "set", "set_id": set_id}))
                    })
                    .collect()
            })
            .unwrap_or_default()
        }
        "note" => run
            .keys
            .papers
            .iter()
            .filter(|p| !paper_folder(&roots.data_dir, **p, "").join(paper_file_names::NOTE).is_file())
            .map(|p| (format!("論文 #{p} の note.md がありません"), json!({"kind": "paper", "paper_id": p})))
            .collect(),
        "triaged" => {
            let n = count(conn, "SELECT COUNT(*) FROM rt_proposals WHERE run_id = ?1 AND status = 'pending'", id);
            miss(n == 0, &format!("PROPOSAL {n} 件が仕分け待ちです"), json!({"kind": "triage"}))
        }
        "starred" => {
            let Some((kind, nid)) = run.keys.node else { return vec![] };
            let q_id = if kind == Kind::O {
                store::get_node(conn, nid).ok().and_then(|n| n.parent_id).unwrap_or(nid)
            } else {
                nid
            };
            let starred = count(
                conn,
                "SELECT COUNT(*) FROM rt_nodes WHERE parent_id = ?1 AND kind = 'O' AND starred = 1 AND removed_at IS NULL",
                q_id,
            ) > 0;
            let text = store::get_node(conn, q_id).map(|n| n.text).unwrap_or_else(|_| fmt_id(Kind::Q, q_id));
            miss(
                starred,
                &format!("「{text}」に ★ がありません"),
                json!({"kind": "node", "node": fmt_id(Kind::Q, q_id)}),
            )
        }
        _ => vec![],
    }
}

fn run_debts(conn: &Connection, roots: &PaperRoots, run: &SciRun) -> Vec<Debt> {
    let def = block_action(roots, &run.block);
    let mut agent = def.done_agent.clone();
    let mut user = if def.support { vec![] } else { def.done_user.clone() };
    if run.runner == "user" {
        let mut moved: Vec<String> = agent.drain(..).filter(|c| c != "report" && c != "review").collect();
        moved.retain(|c| !user.contains(c));
        moved.extend(user);
        user = moved;
    }
    let node = run.keys.node.map(|(k, i)| fmt_id(k, i));
    let mut out = Vec::new();
    for (side, list) in [("agent", agent), ("user", user)] {
        for name in CRITERIA.iter().filter(|c| list.iter().any(|l| l == **c)) {
            for (message, fix) in check(conn, roots, run, name) {
                out.push(Debt {
                    run_id: run.run_id,
                    block: run.block.clone(),
                    node: node.clone(),
                    side: side.to_string(),
                    criterion: (*name).to_string(),
                    message,
                    fix,
                });
            }
        }
    }
    out
}

/// 完了基準を満たしていない run の DEBT（run_id の新しい順、同じ run では Agent → User、基準の表の順）。
pub fn debts(conn: &Connection, roots: &PaperRoots) -> Result<Vec<Debt>> {
    Ok(sci_runs(roots)
        .iter()
        .filter(|r| r.owner == "review" || r.owner == "done")
        .flat_map(|r| run_debts(conn, roots, r))
        .collect())
}

/// Recent Activity に出す Action の行。
#[derive(Debug, Clone, Serialize)]
pub struct ActionItem {
    pub kind: String,
    pub label: String,
    pub occurred_at: String,
    pub paper_id: Option<i64>,
    pub radio_id: Option<String>,
    pub run_id: i64,
    pub node: Option<String>,
    pub block: String,
    pub owner: String,
    pub debt: bool,
    pub debt_messages: Vec<String>,
}

/// `todo` を除く sci の run すべての行。
pub fn activity(conn: &Connection, roots: &PaperRoots) -> Result<Vec<ActionItem>> {
    let all = debts(conn, roots)?;
    Ok(sci_runs(roots)
        .into_iter()
        .filter(|r| r.owner != "todo")
        .map(|r| {
            let node = r.keys.node.map(|(k, i)| fmt_id(k, i));
            let label = match &node {
                Some(n) => format!("{}（{n}）: {}", r.block, r.keys.purpose),
                None => format!("{}: {}", r.block, r.keys.purpose),
            };
            let debt_messages: Vec<String> =
                all.iter().filter(|d| d.run_id == r.run_id).map(|d| d.message.clone()).collect();
            ActionItem {
                kind: "action".into(),
                label,
                occurred_at: r.occurred_at,
                paper_id: None,
                radio_id: None,
                run_id: r.run_id,
                node,
                block: r.block,
                owner: r.owner,
                debt: !debt_messages.is_empty(),
                debt_messages,
            }
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::Database;

    struct Env {
        dir: PathBuf,
        roots: PaperRoots,
        db: Database,
    }

    fn env(tag: &str) -> Env {
        let dir = std::env::temp_dir().join(format!("sai-rt-action-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let data_dir = dir.join(".sai/paper");
        fs::create_dir_all(&data_dir).unwrap();
        fs::create_dir_all(dir.join(".sai/blocks")).unwrap();
        let db = Database::open(&data_dir.join("papr.db")).unwrap();
        Env { roots: PaperRoots { root: dir.clone(), data_dir }, dir, db }
    }

    fn block(e: &Env, id: &str, fm: &str) {
        fs::write(e.dir.join(format!(".sai/blocks/{id}.md")), format!("---\nname: {id}\n{fm}---\n本文\n")).unwrap();
    }

    fn run(e: &Env, owner: &str, id: i64, block: &str, extra: &str) -> PathBuf {
        let p = e.dir.join(format!(".sai/outputs/{owner}/{id}"));
        fs::create_dir_all(&p).unwrap();
        fs::write(
            p.join("task.md"),
            format!("---\nrunner: pi\nblock: {block}\nexecution_date: 2026/10/02 09:16:54\nsci_node: Q1\nsci_purpose: 目的\n{extra}---\n本文\n"),
        )
        .unwrap();
        p
    }

    fn q(e: &Env) {
        let conn = e.db.connection();
        let mut n = store::NewNode {
            kind: Kind::Q, parent_id: None, guide_key: None, text: "データの量は？", body: "", answer_kind: None,
            cites: &[], paper_id: None, run_id: None, report_id: None, mlflow_ref: None, proposal_id: None, created_by: "user",
        };
        store::insert_node(conn, &n).unwrap();
        n.kind = Kind::O;
        n.parent_id = Some(1);
        n.answer_kind = Some("answer");
        n.text = "o";
        n.run_id = Some(10);
        store::insert_node(conn, &n).unwrap();
    }

    fn crit(list: &str, e: &Env, extra: &str) -> Vec<Debt> {
        block(e, "b", &format!("sci_done_agent: {list}\nsci_done_user: \n"));
        run(e, "done", 10, "b", extra);
        debts(e.db.connection(), &e.roots).unwrap()
    }

    #[test]
    fn criteria_each_unmet_and_met() {
        // report / review
        let e = env("rr");
        let d = crit("report, review", &e, "");
        assert_eq!(d.iter().map(|d| d.criterion.as_str()).collect::<Vec<_>>(), ["report", "review"]);
        assert_eq!(d[0].message, "報告書（description 付き）がありません");
        assert_eq!(d[0].fix, json!({"kind":"run","run_id":10}));
        assert_eq!(d[1].message, "振り返り（review.md）が書かれていません");
        let p = e.dir.join(".sai/outputs/done/10");
        fs::write(p.join("report-U1.md"), "---\ndescription: 報告\n---\n").unwrap();
        fs::write(p.join("review.md"), "## 振り返り\n### 良かった\nあり\n").unwrap();
        assert!(debts(e.db.connection(), &e.roots).unwrap().is_empty());

        // O / F / E / logic / q / seed / set(created)
        let e = env("nodes");
        q(&e);
        let d = crit("o_added, f_added, e_added, logic_recorded, q_proposed, seed_created, set_created", &e, "sci_expect: 期待\nsci_papers: 3\n");
        let names: Vec<_> = d.iter().map(|d| d.criterion.as_str()).collect();
        assert_eq!(names, ["f_added", "e_added", "logic_recorded", "q_proposed", "seed_created", "set_created"]);
        assert_eq!(d[0].message, "F が 1 つも入っていません");
        assert_eq!(d[0].fix["kind"], "card");
        assert_eq!(d[0].fix["card"]["node"], "Q1");
        assert_eq!(d[0].fix["card"]["expect"], "期待");
        assert_eq!(d[0].fix["card"]["papers"], json!([3]));
        let conn = e.db.connection();
        store::insert_logic(conn, 1, "ok", "", &[], 0, Some(10), None, "x").unwrap();
        store::insert_seed(conn, "t", "s", &[1], Some(10), "x").unwrap();
        conn.execute("INSERT INTO search_sets (id, name, source_dbs, query) VALUES (1, 'セットA', '[]', 'q')", []).unwrap();
        conn.execute(
            "INSERT INTO rt_searches (run_id, tool, query, set_id) VALUES (10, 't', 'q', 1)", []).unwrap();
        conn.execute("INSERT INTO rt_proposals (run_id, op, payload, risk, status) VALUES (10, 'add_q', '{}', 'auto', 'applied')", []).unwrap();
        let names: Vec<_> = debts(conn, &e.roots).unwrap().into_iter().map(|d| d.criterion).collect();
        assert_eq!(names, ["f_added", "e_added"]);

        // set_screened / note / triaged / starred
        conn.execute("INSERT INTO search_set_items (set_id, position, remote_json, dedupe_key) VALUES (1, 0, '{}', 'k')", []).unwrap();
        conn.execute("INSERT INTO rt_proposals (run_id, op, payload, risk, status) VALUES (10, 'add_o', '{}', 'auto', 'pending')", []).unwrap();
        block(&e, "b", "sci_done_agent: set_screened, note, triaged, starred\nsci_done_user: \n");
        run(&e, "done", 10, "b", "sci_papers: 3\n");
        let d = debts(conn, &e.roots).unwrap();
        let msgs: Vec<_> = d.iter().map(|d| d.message.as_str()).collect();
        assert_eq!(
            msgs,
            ["検索セット「セットA」に未選別 1 件", "論文 #3 の note.md がありません", "PROPOSAL 1 件が仕分け待ちです", "「データの量は？」に ★ がありません"]
        );
        assert_eq!(d[0].fix, json!({"kind":"set","set_id":1}));
        assert_eq!(d[1].fix, json!({"kind":"paper","paper_id":3}));
        assert_eq!(d[2].fix, json!({"kind":"triage"}));
        assert_eq!(d[3].fix, json!({"kind":"node","node":"Q1"}));
        conn.execute("UPDATE search_set_items SET decision='keep'", []).unwrap();
        conn.execute("UPDATE rt_proposals SET status='accepted'", []).unwrap();
        conn.execute("UPDATE rt_nodes SET starred=1 WHERE id=2", []).unwrap();
        let folder = e.roots.data_dir.join("papers/0003_x");
        fs::create_dir_all(&folder).unwrap();
        fs::write(folder.join("note.md"), "n").unwrap();
        assert!(debts(conn, &e.roots).unwrap().is_empty());
    }

    #[test]
    fn support_runner_user_and_owner() {
        let e = env("sup");
        block(&e, "sup", "sci_support: true\nsci_done_agent: report\nsci_done_user: f_added\n");
        run(&e, "done", 10, "sup", "");
        let d = debts(e.db.connection(), &e.roots).unwrap();
        assert!(d.iter().all(|d| d.side == "agent"));
        assert_eq!(d.len(), 1);

        block(&e, "u", "sci_done_agent: report, review, f_added\n");
        run(&e, "done", 11, "u", "");
        fs::write(e.dir.join(".sai/outputs/done/11/task.md"), "---\nrunner: user\nblock: u\nsci_node: Q1\nsci_purpose: p\n---\n").unwrap();
        let d: Vec<_> = debts(e.db.connection(), &e.roots).unwrap().into_iter().filter(|d| d.run_id == 11).collect();
        assert_eq!(d.iter().map(|d| (d.side.as_str(), d.criterion.as_str())).collect::<Vec<_>>(), [("user", "f_added")]);

        run(&e, "running", 12, "u", "");
        run(&e, "todo", 13, "u", "");
        let d = debts(e.db.connection(), &e.roots).unwrap();
        assert!(d.iter().all(|d| d.run_id < 12));
        let a = activity(e.db.connection(), &e.roots).unwrap();
        assert!(a.iter().any(|a| a.run_id == 12 && !a.debt));
        assert!(a.iter().all(|a| a.run_id != 13));
        assert_eq!(a.iter().find(|a| a.run_id == 10).unwrap().label, "sup（Q1）: 目的");
    }

    #[test]
    fn warnings() {
        let e = env("warn");
        block(&e, "x", "sci_action: foo\nsci_done_user: triaged, bar\n");
        block(&e, "ok", "sci_action: split\n");
        let w: Vec<_> = block_warnings(&e.roots).into_iter().filter(|w| w.starts_with("x:") || w.starts_with("ok:")).collect();
        assert_eq!(w.len(), 2, "{w:?}");
        assert!(w[0].contains("foo") && w[1].contains("bar"));
    }

    #[test]
    fn block_id_must_be_lowercase_before_it_becomes_a_file_name() {
        // `^[a-z0-9-]+$` に通る ID だけを読む。外へ出る値をそのままファイル名にしない。
        let e = env("bid");
        for good in ["sci-t", "a", "0", "a-b-9"] {
            assert!(!block_def_paths(&e.roots, good).is_empty(), "{good}");
        }
        for bad in ["", "..", "../sci-t", "a/b", "Sci-T", "sci_t", "sci.t", "a b"] {
            assert!(block_def_paths(&e.roots, bad).is_empty(), "{bad}");
        }
        let _ = fs::remove_dir_all(&e.dir);
    }
}
