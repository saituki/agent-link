//! 研究方針の木（`rt_*` の表）の読み書き。行の型と基本の操作だけを持つ。
//!
//! 毎回計算するもの（Objectivity・Logicality の判定・進み具合）は置かない。
//! 親子の決まりの検査も置かない。DB の CHECK 制約で守れるものだけ表が守る。
//!
//! JSON の文字列の列（`cites` `findings` `path` `hit_papers` `payload`）は Rust では `Vec`・`Value` で持つ。
//! 読めない JSON は空として扱う。

use rusqlite::{Connection, OptionalExtension, Row, params};
use serde::{Deserialize, Serialize};

use super::{Kind, Result, RtError, fmt_id, now_text};

const NODE_COLUMNS: &str = "id, kind, parent_id, guide_key, text, body, answer_kind, starred, \
     target, cites, paper_id, run_id, report_id, mlflow_ref, ord, removed_at, \
     proposal_id, paths, created_by, created_at, updated_at";
const LOGIC_COLUMNS: &str =
    "id, q_id, value, note, findings, f_count, run_id, proposal_id, created_by, created_at";
const LOG_COLUMNS: &str = "id, node_id, action, detail, actor, proposal_id, created_at";
const PROPOSAL_COLUMNS: &str = "id, run_id, block, op, node_id, payload, risk, status, \
     result_ref, decided_by, decided_at, created_at";
const SEARCH_COLUMNS: &str = "id, run_id, node_id, tool, query, scope, hits, \
     hit_papers, set_id, created_at, log_id";

/// 文字列の JSON 配列を読む。読めないときは空。
fn parse_vec<T: for<'de> Deserialize<'de>>(text: &str) -> Vec<T> {
    serde_json::from_str(text).unwrap_or_default()
}

/// 文字列の JSON 配列を書く。
fn write_vec<T: Serialize>(items: &[T]) -> Result<String> {
    Ok(serde_json::to_string(items)?)
}

/// 更新の対象が無かったときの共通の文。
fn expect_changed(changed: usize, id: i64) -> Result<()> {
    if changed == 0 {
        return Err(RtError::NotFound(format!("{id} はありません")));
    }
    Ok(())
}

// ---- Node -----------------------------------------------------------------------

/// `rt_nodes` の 1 行。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct NodeRow {
    pub id: i64,
    pub kind: Kind,
    pub parent_id: Option<i64>,
    pub guide_key: Option<String>,
    pub text: String,
    pub body: String,
    pub answer_kind: Option<String>,
    pub starred: bool,
    pub target: Option<i64>,
    pub cites: Vec<String>,
    pub paper_id: Option<i64>,
    pub run_id: Option<i64>,
    pub report_id: Option<String>,
    pub mlflow_ref: Option<String>,
    pub ord: i64,
    pub removed_at: Option<String>,
    pub proposal_id: Option<i64>,
    /// SPROUT（kind S）の PATH（終点の O の id）。S 以外は空。
    pub paths: Vec<i64>,
    pub created_by: String,
    pub created_at: String,
    pub updated_at: String,
}

/// Node を足すときの値。
#[derive(Debug, Clone)]
pub struct NewNode<'a> {
    pub kind: Kind,
    pub parent_id: Option<i64>,
    pub guide_key: Option<&'a str>,
    pub text: &'a str,
    pub body: &'a str,
    pub answer_kind: Option<&'a str>,
    pub cites: &'a [String],
    pub paper_id: Option<i64>,
    pub run_id: Option<i64>,
    pub report_id: Option<&'a str>,
    pub mlflow_ref: Option<&'a str>,
    pub proposal_id: Option<i64>,
    /// SPROUT（kind S）の PATH。S 以外は空の並び。
    pub paths: &'a [i64],
    pub created_by: &'a str,
}

fn node_from_row(row: &Row<'_>) -> rusqlite::Result<NodeRow> {
    let kind_text: String = row.get(1)?;
    let kind = Kind::from_str(&kind_text).ok_or_else(|| {
        rusqlite::Error::FromSqlConversionFailure(
            1,
            rusqlite::types::Type::Text,
            format!("kind が正しくありません: {kind_text}").into(),
        )
    })?;
    Ok(NodeRow {
        id: row.get(0)?,
        kind,
        parent_id: row.get(2)?,
        guide_key: row.get(3)?,
        text: row.get(4)?,
        body: row.get(5)?,
        answer_kind: row.get(6)?,
        starred: row.get::<_, i64>(7)? != 0,
        target: row.get(8)?,
        cites: parse_vec(&row.get::<_, String>(9)?),
        paper_id: row.get(10)?,
        run_id: row.get(11)?,
        report_id: row.get(12)?,
        mlflow_ref: row.get(13)?,
        ord: row.get(14)?,
        removed_at: row.get(15)?,
        proposal_id: row.get(16)?,
        paths: parse_vec(&row.get::<_, String>(17)?),
        created_by: row.get(18)?,
        created_at: row.get(19)?,
        updated_at: row.get(20)?,
    })
}

fn query_nodes(conn: &Connection, tail: &str, args: impl rusqlite::Params) -> Result<Vec<NodeRow>> {
    let sql = format!("SELECT {NODE_COLUMNS} FROM rt_nodes {tail}");
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(args, node_from_row)?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

/// Node を足す。`ord` は兄弟の最大 + 1。
pub fn insert_node(conn: &Connection, node: &NewNode<'_>) -> Result<i64> {
    let ord: i64 = conn.query_row(
        "SELECT COALESCE(MAX(ord), 0) + 1 FROM rt_nodes WHERE parent_id IS ?1",
        params![node.parent_id],
        |row| row.get(0),
    )?;
    conn.execute(
        "INSERT INTO rt_nodes (kind, parent_id, guide_key, text, body, answer_kind, cites, \
         paper_id, run_id, report_id, mlflow_ref, ord, proposal_id, paths, created_by) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
        params![
            node.kind.as_str(),
            node.parent_id,
            node.guide_key,
            node.text,
            node.body,
            node.answer_kind,
            write_vec(node.cites)?,
            node.paper_id,
            node.run_id,
            node.report_id,
            node.mlflow_ref,
            ord,
            node.proposal_id,
            write_vec(node.paths)?,
            node.created_by,
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

/// Node を 1 つ読む。無ければ NotFound。
pub fn get_node(conn: &Connection, id: i64) -> Result<NodeRow> {
    query_nodes(conn, "WHERE id = ?1", params![id])?
        .into_iter()
        .next()
        .ok_or_else(|| RtError::NotFound(format!("{id} はありません")))
}

/// kind まで合わせて読む。kind が違えば NotFound。
pub fn get_node_of(conn: &Connection, kind: Kind, id: i64) -> Result<NodeRow> {
    match get_node(conn, id) {
        Ok(node) if node.kind == kind => Ok(node),
        Ok(_) | Err(RtError::NotFound(_)) => {
            Err(RtError::NotFound(format!("{} はありません", fmt_id(kind, id))))
        }
        Err(other) => Err(other),
    }
}

/// 全 Node。`(parent_id, ord, id)` の順。
pub fn list_nodes(conn: &Connection, include_removed: bool) -> Result<Vec<NodeRow>> {
    let tail = if include_removed {
        "ORDER BY parent_id, ord, id"
    } else {
        "WHERE removed_at IS NULL ORDER BY parent_id, ord, id"
    };
    query_nodes(conn, tail, [])
}

/// 生きている子を `ord` 順で返す。
pub fn children(conn: &Connection, parent_id: i64) -> Result<Vec<NodeRow>> {
    query_nodes(
        conn,
        "WHERE parent_id = ?1 AND removed_at IS NULL ORDER BY ord, id",
        params![parent_id],
    )
}

/// 根（`parent_id` が NULL の生きた Q。最小の id）。
pub fn root(conn: &Connection) -> Result<Option<NodeRow>> {
    Ok(query_nodes(
        conn,
        "WHERE parent_id IS NULL AND kind = 'Q' AND removed_at IS NULL ORDER BY id LIMIT 1",
        [],
    )?
    .into_iter()
    .next())
}

/// 文と本文を直す（`None` は変えない）。
pub fn update_text(conn: &Connection, id: i64, text: Option<&str>, body: Option<&str>) -> Result<()> {
    let changed = conn.execute(
        "UPDATE rt_nodes SET text = COALESCE(?2, text), body = COALESCE(?3, body), updated_at = ?4 \
         WHERE id = ?1",
        params![id, text, body, now_text()],
    )?;
    expect_changed(changed, id)
}

/// ★ を付け外しする。
pub fn set_starred(conn: &Connection, id: i64, starred: bool) -> Result<()> {
    let changed = conn.execute(
        "UPDATE rt_nodes SET starred = ?2, updated_at = ?3 WHERE id = ?1",
        params![id, i64::from(starred), now_text()],
    )?;
    expect_changed(changed, id)
}

/// O の種類（answer / issue）を変える。
pub fn set_answer_kind(conn: &Connection, id: i64, kind: &str) -> Result<()> {
    let changed = conn.execute(
        "UPDATE rt_nodes SET answer_kind = ?2, updated_at = ?3 WHERE id = ?1",
        params![id, kind, now_text()],
    )?;
    expect_changed(changed, id)
}

/// Q の target（行き先の Node）を変える。
pub fn set_target(conn: &Connection, id: i64, target: Option<i64>) -> Result<()> {
    let changed = conn.execute(
        "UPDATE rt_nodes SET target = ?2, updated_at = ?3 WHERE id = ?1",
        params![id, target, now_text()],
    )?;
    expect_changed(changed, id)
}

/// 自分を含む生きた子孫（深さ優先）。
pub fn subtree_ids(conn: &Connection, id: i64) -> Result<Vec<i64>> {
    let mut out = Vec::new();
    let mut stack = vec![id];
    while let Some(current) = stack.pop() {
        out.push(current);
        let kids = children(conn, current)?;
        // 先頭の子から取り出すよう、逆順に積む。
        stack.extend(kids.iter().rev().map(|kid| kid.id));
    }
    Ok(out)
}

/// 自分と子孫に `removed_at` を入れ、消した id を返す。
pub fn remove_subtree(conn: &Connection, id: i64) -> Result<Vec<i64>> {
    get_node(conn, id)?;
    let ids = subtree_ids(conn, id)?;
    let now = now_text();
    for removed in &ids {
        conn.execute(
            "UPDATE rt_nodes SET removed_at = ?2, updated_at = ?2 WHERE id = ?1",
            params![removed, now],
        )?;
    }
    Ok(ids)
}

// ---- Logic（Logicality の確かめの記録） ---------------------------------------------

/// `rt_logic` の 1 行。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LogicRow {
    pub id: i64,
    pub q_id: i64,
    pub value: String,
    pub note: String,
    pub findings: Vec<i64>,
    pub f_count: i64,
    pub run_id: Option<i64>,
    pub proposal_id: Option<i64>,
    pub created_by: String,
    pub created_at: String,
}

/// 確かめの記録を積む。
#[allow(clippy::too_many_arguments)]
pub fn insert_logic(
    conn: &Connection,
    q_id: i64,
    value: &str,
    note: &str,
    findings: &[i64],
    f_count: i64,
    run_id: Option<i64>,
    proposal_id: Option<i64>,
    created_by: &str,
) -> Result<i64> {
    conn.execute(
        "INSERT INTO rt_logic (q_id, value, note, findings, f_count, run_id, proposal_id, created_by) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![q_id, value, note, write_vec(findings)?, f_count, run_id, proposal_id, created_by],
    )?;
    Ok(conn.last_insert_rowid())
}

/// 確かめの記録（新しい順）。
pub fn list_logic(conn: &Connection, q_id: Option<i64>) -> Result<Vec<LogicRow>> {
    let sql = format!(
        "SELECT {LOGIC_COLUMNS} FROM rt_logic WHERE (?1 IS NULL OR q_id = ?1) ORDER BY id DESC"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params![q_id], |row| {
        Ok(LogicRow {
            id: row.get(0)?,
            q_id: row.get(1)?,
            value: row.get(2)?,
            note: row.get(3)?,
            findings: parse_vec(&row.get::<_, String>(4)?),
            f_count: row.get(5)?,
            run_id: row.get(6)?,
            proposal_id: row.get(7)?,
            created_by: row.get(8)?,
            created_at: row.get(9)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

// ---- Log（追記だけの記録） -----------------------------------------------------------

/// `rt_log` の 1 行。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LogRow {
    pub id: i64,
    pub node_id: i64,
    pub action: String,
    pub detail: String,
    pub actor: String,
    pub proposal_id: Option<i64>,
    pub created_at: String,
}

/// 記録を 1 行足す。
pub fn insert_log(
    conn: &Connection,
    node_id: i64,
    action: &str,
    detail: &str,
    actor: &str,
    proposal_id: Option<i64>,
) -> Result<i64> {
    conn.execute(
        "INSERT INTO rt_log (node_id, action, detail, actor, proposal_id) \
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![node_id, action, detail, actor, proposal_id],
    )?;
    Ok(conn.last_insert_rowid())
}

/// 記録（新しい順）。
pub fn list_log(conn: &Connection, limit: Option<usize>) -> Result<Vec<LogRow>> {
    let sql = format!("SELECT {LOG_COLUMNS} FROM rt_log ORDER BY id DESC LIMIT ?1");
    let mut stmt = conn.prepare(&sql)?;
    let limit = limit.map_or(-1, |n| i64::try_from(n).unwrap_or(i64::MAX));
    let rows = stmt.query_map(params![limit], |row| {
        Ok(LogRow {
            id: row.get(0)?,
            node_id: row.get(1)?,
            action: row.get(2)?,
            detail: row.get(3)?,
            actor: row.get(4)?,
            proposal_id: row.get(5)?,
            created_at: row.get(6)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

// ---- Proposal -------------------------------------------------------------------------

/// `rt_proposals` の 1 行。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ProposalRow {
    pub id: i64,
    pub run_id: Option<i64>,
    pub block: Option<String>,
    pub op: String,
    pub node_id: Option<i64>,
    pub payload: serde_json::Value,
    pub risk: String,
    pub status: String,
    pub result_ref: Option<String>,
    pub decided_by: Option<String>,
    pub decided_at: Option<String>,
    pub created_at: String,
}

fn proposal_from_row(row: &Row<'_>) -> rusqlite::Result<ProposalRow> {
    Ok(ProposalRow {
        id: row.get(0)?,
        run_id: row.get(1)?,
        block: row.get(2)?,
        op: row.get(3)?,
        node_id: row.get(4)?,
        payload: serde_json::from_str(&row.get::<_, String>(5)?).unwrap_or(serde_json::Value::Null),
        risk: row.get(6)?,
        status: row.get(7)?,
        result_ref: row.get(8)?,
        decided_by: row.get(9)?,
        decided_at: row.get(10)?,
        created_at: row.get(11)?,
    })
}

/// 提案を足す。
#[allow(clippy::too_many_arguments)]
pub fn insert_proposal(
    conn: &Connection,
    run_id: Option<i64>,
    block: Option<&str>,
    op: &str,
    node_id: Option<i64>,
    payload: &serde_json::Value,
    risk: &str,
    status: &str,
) -> Result<i64> {
    conn.execute(
        "INSERT INTO rt_proposals (run_id, block, op, node_id, payload, risk, status) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![run_id, block, op, node_id, serde_json::to_string(payload)?, risk, status],
    )?;
    Ok(conn.last_insert_rowid())
}

/// 提案を 1 つ読む。無ければ NotFound。
pub fn get_proposal(conn: &Connection, id: i64) -> Result<ProposalRow> {
    let sql = format!("SELECT {PROPOSAL_COLUMNS} FROM rt_proposals WHERE id = ?1");
    conn.query_row(&sql, params![id], proposal_from_row)
        .optional()?
        .ok_or_else(|| RtError::NotFound(format!("P{id} はありません")))
}

/// 提案の一覧（古い順）。`status` を渡すとその状態だけ。
pub fn list_proposals(conn: &Connection, status: Option<&str>) -> Result<Vec<ProposalRow>> {
    let sql = format!(
        "SELECT {PROPOSAL_COLUMNS} FROM rt_proposals WHERE (?1 IS NULL OR status = ?1) ORDER BY id"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params![status], proposal_from_row)?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

/// 提案の結末を書く。
pub fn finish_proposal(
    conn: &Connection,
    id: i64,
    status: &str,
    result_ref: Option<&str>,
    decided_by: Option<&str>,
) -> Result<()> {
    let changed = conn.execute(
        "UPDATE rt_proposals SET status = ?2, result_ref = ?3, decided_by = ?4, decided_at = ?5 \
         WHERE id = ?1",
        params![id, status, result_ref, decided_by, now_text()],
    )?;
    if changed == 0 {
        return Err(RtError::NotFound(format!("P{id} はありません")));
    }
    Ok(())
}

// ---- Search ---------------------------------------------------------------------------

/// `rt_searches` の 1 行。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SearchRow {
    pub id: i64,
    pub run_id: Option<i64>,
    pub node_id: Option<i64>,
    pub tool: String,
    pub query: String,
    pub scope: String,
    pub hits: i64,
    pub hit_papers: Vec<i64>,
    pub set_id: Option<i64>,
    pub created_at: String,
    /// その 1 回の検索の履歴（`search_sets` の行）。
    pub log_id: Option<i64>,
}

/// 検索の記録を足す。`id` と `created_at` は無視して振る。
pub fn insert_search(conn: &Connection, row: &SearchRow) -> Result<i64> {
    conn.execute(
        "INSERT INTO rt_searches (run_id, node_id, tool, query, scope, hits, \
         hit_papers, set_id, log_id) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            row.run_id,
            row.node_id,
            row.tool,
            row.query,
            row.scope,
            row.hits,
            write_vec(&row.hit_papers)?,
            row.set_id,
            row.log_id,
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

/// 検索の記録（新しい順）。
pub fn list_searches(conn: &Connection, node_id: Option<i64>) -> Result<Vec<SearchRow>> {
    let sql = format!(
        "SELECT {SEARCH_COLUMNS} FROM rt_searches WHERE (?1 IS NULL OR node_id = ?1) ORDER BY id DESC"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params![node_id], |row| {
        Ok(SearchRow {
            id: row.get(0)?,
            run_id: row.get(1)?,
            node_id: row.get(2)?,
            tool: row.get(3)?,
            query: row.get(4)?,
            scope: row.get(5)?,
            hits: row.get(6)?,
            hit_papers: parse_vec(&row.get::<_, String>(7)?),
            set_id: row.get(8)?,
            created_at: row.get(9)?,
            log_id: row.get(10)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

// ---- Meta -----------------------------------------------------------------------------

/// `rt_meta` の値を読む。
pub fn meta_get(conn: &Connection, key: &str) -> Result<Option<String>> {
    Ok(conn
        .query_row("SELECT value FROM rt_meta WHERE key = ?1", params![key], |row| row.get(0))
        .optional()?)
}

/// `rt_meta` の値を書く（あれば上書き）。
pub fn meta_set(conn: &Connection, key: &str, value: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO rt_meta (key, value) VALUES (?1, ?2) \
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![key, value],
    )?;
    Ok(())
}

/// 新しい木があるか（`rt_meta` に `initialized_at` がある）。
pub fn is_initialized(conn: &Connection) -> Result<bool> {
    Ok(meta_get(conn, "initialized_at")?.is_some())
}

/// 古い木だけがあるか（新しい木が無く、`sci_meta` に `initialized_at` がある）。`sci_meta` が無い DB でも false。
pub fn is_legacy_only(conn: &Connection) -> Result<bool> {
    if is_initialized(conn)? {
        return Ok(false);
    }
    let has_table: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'sci_meta')",
        [],
        |row| row.get(0),
    )?;
    if !has_table {
        return Ok(false);
    }
    Ok(conn
        .query_row("SELECT 1 FROM sci_meta WHERE key = 'initialized_at'", [], |_| Ok(()))
        .optional()?
        .is_some())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sai_paper_core::database::Database;

    fn db() -> Database {
        Database::in_memory().expect("メモリ DB")
    }

    fn new_node<'a>(kind: Kind, parent_id: Option<i64>, text: &'a str) -> NewNode<'a> {
        NewNode {
            kind,
            parent_id,
            guide_key: None,
            text,
            body: "",
            answer_kind: (kind == Kind::O).then_some("answer"),
            cites: &[],
            paper_id: None,
            run_id: None,
            report_id: None,
            mlflow_ref: None,
            proposal_id: None,
            paths: &[],
            created_by: "user",
        }
    }

    #[test]
    fn migration_creates_tables() {
        let database = db();
        let conn = database.connection();
        for table in [
            "rt_meta", "rt_nodes", "rt_logic", "rt_log", "rt_proposals", "rt_searches",
        ] {
            let found: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
                    params![table],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(found, 1, "{table}");
        }
        conn.prepare("SELECT rt_node FROM search_sets").expect("search_sets.rt_node");
    }

    #[test]
    fn check_constraints_reject_bad_rows() {
        let database = db();
        let conn = database.connection();
        let q = insert_node(conn, &new_node(Kind::Q, None, "問い")).unwrap();

        // F に answer_kind。
        let mut bad = new_node(Kind::F, Some(q), "根拠");
        bad.answer_kind = Some("answer");
        assert!(insert_node(conn, &bad).is_err());
        // O 以外に target。
        let f = insert_node(conn, &new_node(Kind::F, Some(q), "根拠")).unwrap();
        assert!(set_target(conn, f, Some(q)).is_err());
        // 親の無い O。
        assert!(insert_node(conn, &new_node(Kind::O, None, "候補")).is_err());
        // Q の target は入れられる。
        set_target(conn, q, Some(f)).unwrap();
    }

    #[test]
    fn remove_subtree_hides_only_subtree() {
        let database = db();
        let conn = database.connection();
        let q = insert_node(conn, &new_node(Kind::Q, None, "根")).unwrap();
        let o1 = insert_node(conn, &new_node(Kind::O, Some(q), "候補 1")).unwrap();
        let o2 = insert_node(conn, &new_node(Kind::O, Some(q), "候補 2")).unwrap();
        let f = insert_node(conn, &new_node(Kind::F, Some(o1), "根拠")).unwrap();

        assert_eq!(get_node(conn, o2).unwrap().ord, 2);
        assert_eq!(subtree_ids(conn, q).unwrap(), vec![q, o1, f, o2]);
        assert_eq!(remove_subtree(conn, o1).unwrap(), vec![o1, f]);

        assert!(get_node(conn, o1).unwrap().removed_at.is_some());
        assert!(get_node(conn, f).unwrap().removed_at.is_some());
        assert!(get_node(conn, o2).unwrap().removed_at.is_none());
        let ids: Vec<i64> = children(conn, q).unwrap().iter().map(|n| n.id).collect();
        assert_eq!(ids, vec![o2]);
        let live: Vec<i64> = list_nodes(conn, false).unwrap().iter().map(|n| n.id).collect();
        assert_eq!(live, vec![q, o2]);
        assert_eq!(list_nodes(conn, true).unwrap().len(), 4);
        assert_eq!(root(conn).unwrap().unwrap().id, q);
        assert!(matches!(get_node_of(conn, Kind::Q, o2), Err(RtError::NotFound(_))));
    }

    #[test]
    fn log_is_append_only() {
        let database = db();
        let conn = database.connection();
        let q = insert_node(conn, &new_node(Kind::Q, None, "根")).unwrap();
        let id = insert_log(conn, q, "edit", "", "user", None).unwrap();
        assert!(conn.execute("UPDATE rt_log SET detail = 'x' WHERE id = ?1", params![id]).is_err());
        assert!(conn.execute("DELETE FROM rt_log WHERE id = ?1", params![id]).is_err());
        assert_eq!(list_log(conn, Some(5)).unwrap().len(), 1);
    }

    #[test]
    fn legacy_only_detection() {
        let database = db();
        let conn = database.connection();
        assert!(!is_legacy_only(conn).unwrap());
        conn.execute("INSERT INTO sci_meta (key, value) VALUES ('initialized_at', 'now')", [])
            .unwrap();
        assert!(is_legacy_only(conn).unwrap());
        meta_set(conn, "initialized_at", "now").unwrap();
        assert!(is_initialized(conn).unwrap());
        assert!(!is_legacy_only(conn).unwrap());
        // sci_meta の無い DB。
        let bare = Connection::open_in_memory().unwrap();
        assert!(!is_legacy_only(&bare).unwrap_or(false));
    }

    #[test]
    fn other_tables_round_trip() {
        let database = db();
        let conn = database.connection();
        let q = insert_node(conn, &new_node(Kind::Q, None, "根")).unwrap();
        insert_logic(conn, q, "ok", "", &[1, 2], 2, None, None, "user").unwrap();
        assert_eq!(list_logic(conn, Some(q)).unwrap()[0].findings, vec![1, 2]);
        let p = insert_proposal(
            conn, None, None, "add", Some(q), &serde_json::json!({"a": 1}), "auto", "pending",
        )
        .unwrap();
        finish_proposal(conn, p, "applied", Some("O2"), None).unwrap();
        let got = get_proposal(conn, p).unwrap();
        assert_eq!(got.status, "applied");
        assert_eq!(got.payload["a"], 1);
        assert_eq!(list_proposals(conn, Some("pending")).unwrap().len(), 0);
        let row = SearchRow {
            id: 0, run_id: None, node_id: Some(q), tool: "t".into(), query: "q".into(),
            scope: String::new(), hits: 1, hit_papers: vec![7],
            set_id: None, created_at: String::new(), log_id: None,
        };
        insert_search(conn, &row).unwrap();
        assert_eq!(list_searches(conn, Some(q)).unwrap()[0].hit_papers, vec![7]);
    }
}
