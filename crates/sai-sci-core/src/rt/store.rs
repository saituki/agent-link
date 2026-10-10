//! research-tree（`rt_*` の表）の読み書き。行の型と基本の操作だけを持つ。
//!
//! 毎回計算するもの（進み具合など）は置かない。
//! 親子の決まりの検査も置かない。DB の CHECK 制約で守れるものだけ表が守る（親子の決まりは `ops::execute` が守る）。
//!
//! JSON の文字列の列（`cites` `paths` `hit_papers`）は Rust では `Vec` で持つ。
//! 読めない JSON は空として扱う。

use rusqlite::{Connection, OptionalExtension, Row, params};
use serde::{Deserialize, Serialize};

use super::{Kind, Result, RtError, fmt_id, now_text};

const NODE_COLUMNS: &str = "id, kind, parent_id, guide_key, text, body, stance, starred, \
     cites, paper_id, run_id, report_id, mlflow_ref, ord, removed_at, paths, \
     created_by, created_at, updated_at";
const LOG_COLUMNS: &str =
    "id, node_id, action, detail, actor, approved_by, qid, question, created_at";
const APPROVAL_COLUMNS: &str = "id, run_id, qid, question, answer, answered_by, created_at, \
     consumed_at";
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
    /// F の立場（support / deny / issue）。F 以外は None。
    pub stance: Option<String>,
    pub starred: bool,
    pub cites: Vec<String>,
    pub paper_id: Option<i64>,
    pub run_id: Option<i64>,
    pub report_id: Option<String>,
    pub mlflow_ref: Option<String>,
    pub ord: i64,
    pub removed_at: Option<String>,
    /// SPROUT（kind S）の PATH（終点の A の id）。S 以外は空。
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
    pub stance: Option<&'a str>,
    pub cites: &'a [String],
    pub paper_id: Option<i64>,
    pub run_id: Option<i64>,
    pub report_id: Option<&'a str>,
    pub mlflow_ref: Option<&'a str>,
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
        stance: row.get(6)?,
        starred: row.get::<_, i64>(7)? != 0,
        cites: parse_vec(&row.get::<_, String>(8)?),
        paper_id: row.get(9)?,
        run_id: row.get(10)?,
        report_id: row.get(11)?,
        mlflow_ref: row.get(12)?,
        ord: row.get(13)?,
        removed_at: row.get(14)?,
        paths: parse_vec(&row.get::<_, String>(15)?),
        created_by: row.get(16)?,
        created_at: row.get(17)?,
        updated_at: row.get(18)?,
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
        "INSERT INTO rt_nodes (kind, parent_id, guide_key, text, body, stance, cites, \
         paper_id, run_id, report_id, mlflow_ref, ord, paths, created_by) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
        params![
            node.kind.as_str(),
            node.parent_id,
            node.guide_key,
            node.text,
            node.body,
            node.stance,
            write_vec(node.cites)?,
            node.paper_id,
            node.run_id,
            node.report_id,
            node.mlflow_ref,
            ord,
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

/// F の立場を変える。
pub fn set_stance(conn: &Connection, id: i64, stance: &str) -> Result<()> {
    let changed = conn.execute(
        "UPDATE rt_nodes SET stance = ?2, updated_at = ?3 WHERE id = ?1",
        params![id, stance, now_text()],
    )?;
    expect_changed(changed, id)
}

/// E の MLflow の結び付けを変える（`None` で外す）。
pub fn set_mlflow(conn: &Connection, id: i64, mlflow: Option<&str>) -> Result<()> {
    let changed = conn.execute(
        "UPDATE rt_nodes SET mlflow_ref = ?2, updated_at = ?3 WHERE id = ?1",
        params![id, mlflow, now_text()],
    )?;
    expect_changed(changed, id)
}

/// 親を付け替える。`ord` は新しい親の子の最後。
pub fn set_parent(conn: &Connection, id: i64, parent_id: i64) -> Result<()> {
    let ord: i64 = conn.query_row(
        "SELECT COALESCE(MAX(ord), 0) + 1 FROM rt_nodes WHERE parent_id = ?1",
        params![parent_id],
        |row| row.get(0),
    )?;
    let changed = conn.execute(
        "UPDATE rt_nodes SET parent_id = ?2, ord = ?3, updated_at = ?4 WHERE id = ?1",
        params![id, parent_id, ord, now_text()],
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

// ---- Log（追記だけの記録） -----------------------------------------------------------

/// `rt_log` の 1 行。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LogRow {
    pub id: i64,
    pub node_id: i64,
    pub action: String,
    pub detail: String,
    pub actor: String,
    /// 承認した人（許可が要る op だけ。それ以外は None）。
    pub approved_by: Option<String>,
    /// 承認の質問の ID（許可が要る op だけ）。
    pub qid: Option<String>,
    /// 承認の質問の文（許可が要る op だけ）。
    pub question: Option<String>,
    pub created_at: String,
}

/// 記録を 1 行足す。`approval` は許可を使った op のときだけ入れる（その行の承認の印）。
pub fn insert_log(
    conn: &Connection,
    node_id: i64,
    action: &str,
    detail: &str,
    actor: &str,
    approval: Option<&ApprovalRow>,
) -> Result<i64> {
    conn.execute(
        "INSERT INTO rt_log (node_id, action, detail, actor, approved_by, qid, question) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            node_id,
            action,
            detail,
            actor,
            approval.map(|a| a.answered_by.as_str()),
            approval.map(|a| a.qid.as_str()),
            approval.map(|a| a.question.as_str()),
        ],
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
            approved_by: row.get(5)?,
            qid: row.get(6)?,
            question: row.get(7)?,
            created_at: row.get(8)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

// ---- Approval（質問で取った許可） -----------------------------------------------------

/// `rt_approvals` の 1 行。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ApprovalRow {
    pub id: i64,
    pub run_id: i64,
    pub qid: String,
    pub question: String,
    pub answer: String,
    pub answered_by: String,
    pub created_at: String,
    pub consumed_at: Option<String>,
}

fn approval_from_row(row: &Row<'_>) -> rusqlite::Result<ApprovalRow> {
    Ok(ApprovalRow {
        id: row.get(0)?,
        run_id: row.get(1)?,
        qid: row.get(2)?,
        question: row.get(3)?,
        answer: row.get(4)?,
        answered_by: row.get(5)?,
        created_at: row.get(6)?,
        consumed_at: row.get(7)?,
    })
}

/// 質問の回答を 1 行足す。同じ `qid` が既にあれば Db エラー（UNIQUE）。
pub fn insert_approval(
    conn: &Connection,
    run_id: i64,
    qid: &str,
    question: &str,
    answer: &str,
    answered_by: &str,
) -> Result<i64> {
    conn.execute(
        "INSERT INTO rt_approvals (run_id, qid, question, answer, answered_by) \
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![run_id, qid, question, answer, answered_by],
    )?;
    Ok(conn.last_insert_rowid())
}

/// その run の未消費の許可を 1 つ読む（最も古いもの。消費はしない）。
/// 回答を空白で切った値が `許可する` の行だけ。
pub fn take_approval(conn: &Connection, run_id: i64) -> Result<Option<ApprovalRow>> {
    let sql = format!(
        "SELECT {APPROVAL_COLUMNS} FROM rt_approvals \
         WHERE run_id = ?1 AND consumed_at IS NULL AND trim(answer) = '許可する' \
         ORDER BY id LIMIT 1"
    );
    Ok(conn.query_row(&sql, params![run_id], approval_from_row).optional()?)
}

/// 許可を使い切る。`ops` は許可で出した op 名の並び。
pub fn consume_approval(conn: &Connection, id: i64, ops: &[String]) -> Result<()> {
    let changed = conn.execute(
        "UPDATE rt_approvals SET consumed_at = ?2, consumed_ops = ?3 WHERE id = ?1",
        params![id, now_text(), write_vec(ops)?],
    )?;
    expect_changed(changed, id)
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
    /// その 1 回の検索の履歴（`search_history` の行）。
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

/// 新しい research-tree があるか（`rt_meta` に `initialized_at` がある）。
pub fn is_initialized(conn: &Connection) -> Result<bool> {
    Ok(meta_get(conn, "initialized_at")?.is_some())
}

/// 古い research-tree だけがあるか（新しい research-tree が無く、`sci_meta` に `initialized_at` がある）。`sci_meta` が無い DB でも false。
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
            stance: (kind == Kind::F).then_some("support"),
            cites: &[],
            paper_id: None,
            run_id: None,
            report_id: None,
            mlflow_ref: None,
            paths: &[],
            created_by: "user",
        }
    }

    #[test]
    fn migration_creates_tables() {
        let database = db();
        let conn = database.connection();
        for table in [
            "rt_meta", "rt_nodes", "rt_log", "rt_approvals", "rt_searches",
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
        // 0056 で消した表は無い。
        for table in ["rt_logic", "rt_proposals"] {
            let found: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
                    params![table],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(found, 0, "{table}");
        }
        conn.prepare("SELECT rt_node FROM search_history").expect("search_history.rt_node");
    }

    #[test]
    fn check_constraints_reject_bad_rows() {
        let database = db();
        let conn = database.connection();
        let q = insert_node(conn, &new_node(Kind::Q, None, "問い")).unwrap();
        let a = insert_node(conn, &new_node(Kind::A, Some(q), "答え")).unwrap();

        // F で stance が NULL は弾く。
        let mut bad = new_node(Kind::F, Some(a), "根拠");
        bad.stance = None;
        assert!(insert_node(conn, &bad).is_err());
        // A に stance があると弾く。
        let mut bad_a = new_node(Kind::A, Some(q), "答え 2");
        bad_a.stance = Some("support");
        assert!(insert_node(conn, &bad_a).is_err());
        // 親の無い A。
        assert!(insert_node(conn, &new_node(Kind::A, None, "候補")).is_err());
        // 親を持つ D は弾く。
        assert!(insert_node(conn, &new_node(Kind::D, Some(q), "seed")).is_err());
        // 親の無い D は入り、その下に S を作れる。
        let d = insert_node(conn, &new_node(Kind::D, None, "seed")).unwrap();
        insert_node(conn, &new_node(Kind::S, Some(d), "sprout")).unwrap();
        // 古い O は弾く。
        assert!(conn
            .execute("INSERT INTO rt_nodes (kind, parent_id, text) VALUES ('O', ?1, 'x')", params![q])
            .is_err());
        // F は入る。
        insert_node(conn, &new_node(Kind::F, Some(a), "根拠")).unwrap();
        // 立場は付け替えられる。
        let f = insert_node(conn, &new_node(Kind::F, Some(a), "根拠 2")).unwrap();
        set_stance(conn, f, "deny").unwrap();
        assert_eq!(get_node(conn, f).unwrap().stance.as_deref(), Some("deny"));
    }

    #[test]
    fn remove_subtree_hides_only_subtree() {
        let database = db();
        let conn = database.connection();
        let q = insert_node(conn, &new_node(Kind::Q, None, "根")).unwrap();
        let a1 = insert_node(conn, &new_node(Kind::A, Some(q), "候補 1")).unwrap();
        let a2 = insert_node(conn, &new_node(Kind::A, Some(q), "候補 2")).unwrap();
        let f = insert_node(conn, &new_node(Kind::F, Some(a1), "根拠")).unwrap();

        assert_eq!(get_node(conn, a2).unwrap().ord, 2);
        assert_eq!(subtree_ids(conn, q).unwrap(), vec![q, a1, f, a2]);
        assert_eq!(remove_subtree(conn, a1).unwrap(), vec![a1, f]);

        assert!(get_node(conn, a1).unwrap().removed_at.is_some());
        assert!(get_node(conn, f).unwrap().removed_at.is_some());
        assert!(get_node(conn, a2).unwrap().removed_at.is_none());
        let ids: Vec<i64> = children(conn, q).unwrap().iter().map(|n| n.id).collect();
        assert_eq!(ids, vec![a2]);
        let live: Vec<i64> = list_nodes(conn, false).unwrap().iter().map(|n| n.id).collect();
        assert_eq!(live, vec![q, a2]);
        assert_eq!(list_nodes(conn, true).unwrap().len(), 4);
        assert!(matches!(get_node_of(conn, Kind::Q, a2), Err(RtError::NotFound(_))));
    }

    #[test]
    fn set_parent_moves_node_to_last() {
        let database = db();
        let conn = database.connection();
        let q1 = insert_node(conn, &new_node(Kind::Q, None, "根")).unwrap();
        let q2 = insert_node(conn, &new_node(Kind::Q, Some(q1), "子 1")).unwrap();
        let q3 = insert_node(conn, &new_node(Kind::Q, Some(q1), "子 2")).unwrap();
        let other = insert_node(conn, &new_node(Kind::Q, None, "別の根")).unwrap();
        set_parent(conn, other, q1).unwrap();
        assert_eq!(get_node(conn, other).unwrap().ord, 3);
        assert_eq!(get_node(conn, q2).unwrap().ord, 1);
        assert_eq!(get_node(conn, q3).unwrap().ord, 2);
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
    fn approvals_take_only_allowed_and_consume() {
        let database = db();
        let conn = database.connection();
        // 許可しない の回答は取れない。
        insert_approval(conn, 61, "a1", "作ってよいか", "許可しない", "saituki").unwrap();
        assert!(take_approval(conn, 61).unwrap().is_none());
        // 許可する は取れる（消費はしない）。
        insert_approval(conn, 61, "a2", "作ってよいか", " 許可する ", "saituki").unwrap();
        let first = take_approval(conn, 61).unwrap().expect("許可する");
        assert_eq!(first.qid, "a2");
        assert!(take_approval(conn, 61).unwrap().is_some());
        // 別の run には効かない。
        assert!(take_approval(conn, 62).unwrap().is_none());
        // 消費した行は 2 回目に返らない。
        consume_approval(conn, first.id, &["add_q".to_string()]).unwrap();
        assert!(take_approval(conn, 61).unwrap().is_none());
        // 同じ qid は入らない。
        assert!(insert_approval(conn, 61, "a2", "x", "許可する", "saituki").is_err());
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
        let row = SearchRow {
            id: 0, run_id: None, node_id: Some(q), tool: "t".into(), query: "q".into(),
            scope: String::new(), hits: 1, hit_papers: vec![7],
            set_id: None, created_at: String::new(), log_id: None,
        };
        insert_search(conn, &row).unwrap();
        assert_eq!(list_searches(conn, Some(q)).unwrap()[0].hit_papers, vec![7]);
        meta_set(conn, "k", "v").unwrap();
        assert_eq!(meta_get(conn, "k").unwrap().as_deref(), Some("v"));
    }
}
