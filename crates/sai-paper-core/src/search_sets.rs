//! 検索セット: リモート検索結果の永続化と注釈付け。
//!
//! 複数の DB から得たリモート論文候補を保存し、ユーザーやエージェントが
//! 重複排除・keep/drop の判定・ダウンロードを行う。候補はまだ `papers` テーブルに
//! 入らない。DL / import した時点で papers に行ができ、`paper_id` が埋まり
//! `decision='downloaded'` になる。

use crate::config::Config;
use crate::database::Database;
use crate::models::RemotePaper;
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// 検索セット操作のエラー。
#[derive(Debug, Error)]
pub enum SearchSetError {
    /// SQLite 操作に失敗した。
    #[error("database error: {0}")]
    Database(#[from] rusqlite::Error),
    /// [`Database`] の操作に失敗した。
    #[error("database error: {0}")]
    DatabaseOps(#[from] crate::database::DatabaseError),
    /// JSON の変換に失敗した。
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    /// 対象が見つからない。
    #[error("not found: {0}")]
    NotFound(String),
    /// 入力が不正。
    #[error("invalid input: {0}")]
    Invalid(String),
}

/// 検索セット操作の結果型。
pub type Result<T> = std::result::Result<T, SearchSetError>;

/// 検索セットのメタデータ。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SearchSet {
    /// 主キー。
    pub id: i64,
    /// セット名（ユニーク）。
    pub name: String,
    /// 検索に使った DB（カンマ区切り: "s2,pubmed"）。
    pub source_dbs: String,
    /// 検索クエリ（自然言語）。
    pub query: String,
    /// DB 固有構文（raw_query）。
    pub raw_query: Option<String>,
    /// 作成者（"user" / agent ID）。
    pub created_by: String,
    /// メモ。
    pub note: Option<String>,
    /// 作成日時（ISO8601）。
    pub created_at: String,
}

/// 検索セットの候補 1 件。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SearchSetItem {
    /// セット内での位置（0 始まり）。
    pub position: i64,
    /// リモート API から返ってきたメタデータ（JSON）。
    pub remote: RemotePaper,
    /// 重複排除キー（api::dedupe_key の返り値）。
    pub dedupe_key: String,
    /// ダウンロード / import されたときの papers ID。
    pub paper_id: Option<i64>,
    /// ユーザーの判定（"pending" / "keep" / "drop" / "downloaded"）。
    pub decision: String,
    /// 判定の理由（オプション）。
    pub reason: Option<String>,
    /// 追加日時。
    pub added_at: String,
}

/// セット作成時のパラメータ。
pub struct CreateSearchSetParams {
    /// セット名。
    pub name: String,
    /// 検索した DB（カンマ区切り）。
    pub source_dbs: String,
    /// 検索クエリ。
    pub query: String,
    /// DB 固有構文。
    pub raw_query: Option<String>,
    /// 作成者。
    pub created_by: String,
    /// メモ。
    pub note: Option<String>,
}

/// 検索セットを新規作成する。
///
/// # Errors
///
/// セット名が重複している、またはその他 DB エラーの場合。
pub fn create(conn: &Connection, params: CreateSearchSetParams) -> Result<i64> {
    conn.execute(
        "INSERT INTO search_sets (name, source_dbs, query, raw_query, created_by, note, created_at,
                                  origin, searched_at, log_date)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?7, ?9)",
        rusqlite::params![
            params.name,
            params.source_dbs,
            params.query,
            params.raw_query,
            params.created_by,
            params.note,
            chrono::Local::now().to_rfc3339(),
            // 検索の記録は 1 つ（段階 6）: エージェントが作ったものは agent、それ以外は cli。
            if params.created_by == "agent" { "agent" } else { "cli" },
            chrono::Local::now().format("%Y/%m/%d").to_string(),
        ],
    )
    .map_err(|e| {
        if e.to_string().contains("UNIQUE constraint failed") {
            SearchSetError::Invalid(format!("セット名が既に存在します: {}", params.name))
        } else {
            SearchSetError::Database(e)
        }
    })?;
    Ok(conn.last_insert_rowid())
}

/// セットに複数の候補を追加する。
///
/// 重複排除キーが既にセット内に存在する候補はスキップされる。
/// 既に存在する候補の paper_id / decision は更新されない。
/// 候補は `pending` で入る（採否は利用者が選別して決める）。
///
/// # Errors
///
/// DB エラーの場合。
pub fn add_items(conn: &Connection, set_id: i64, papers: Vec<RemotePaper>) -> Result<Vec<usize>> {
    let mut stmt = conn
        .prepare("SELECT position FROM search_set_items WHERE set_id = ?1 AND dedupe_key = ?2")?;

    let mut added = Vec::new();
    let now = chrono::Utc::now().to_rfc3339();

    // 最大位置を取得
    let max_pos: i64 = conn
        .query_row(
            "SELECT COALESCE(MAX(position), -1) FROM search_set_items WHERE set_id = ?1",
            [set_id],
            |r| r.get(0),
        )
        .unwrap_or(-1);

    let mut next_pos = max_pos + 1;

    for paper in papers {
        let dedupe_key = crate::api::dedupe_key(&paper);

        // この dedupe_key がセット内に既に存在するか確認
        let exists: bool = stmt
            .query_row(params![set_id, &dedupe_key], |_| Ok(true))
            .optional()?
            .is_some();

        if exists {
            // スキップ
            continue;
        }

        conn.execute(
            "INSERT INTO search_set_items
             (set_id, position, remote_json, dedupe_key, decision, reason, added_at)
             VALUES (?1, ?2, ?3, ?4, 'pending', NULL, ?5)",
            rusqlite::params![
                set_id,
                next_pos,
                serde_json::to_string(&paper)?,
                dedupe_key,
                now,
            ],
        )?;

        added.push(next_pos as usize);
        next_pos += 1;
    }

    Ok(added)
}

/// 検索セットに「何のための検索か」を書く欄（R-R6）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SciSetFields {
    /// 結ぶ節点の番号。
    pub node: Option<i64>,
    /// 結ぶ問いの番号。
    pub question: Option<i64>,
    /// 何を確かめるための検索か。
    pub purpose: Option<String>,
    /// 検索結果から抜き出したい項目。
    pub extract: Option<String>,
}

/// 検索セットに、何のための検索かを書く（Node・目的・抜き出す項目）。`node` は `rt_nodes.id`。
/// `question` は読まない（古い `sci_question` 列は NULL にする）。
///
/// # Errors
///
/// セットが無い、または DB エラーの場合。
pub fn set_sci_fields(conn: &Connection, set_id: i64, fields: &SciSetFields) -> Result<()> {
    let updated = conn.execute(
        "UPDATE search_sets
            SET rt_node = ?1, sci_question = NULL, purpose = ?3, extract = ?4
          WHERE id = ?5",
        params![
            fields.node,
            Option::<i64>::None,
            fields.purpose,
            fields.extract,
            set_id,
        ],
    )?;
    if updated == 0 {
        return Err(SearchSetError::NotFound(format!(
            "セット ID {} が見つかりません",
            set_id
        )));
    }
    Ok(())
}

/// 研究方針の木の Node の番号から、外に出す形の文字（例 `O5`）を作る。
///
/// `sai-sci-core::rt::fmt_id` と同じ形（`<種類><番号>`）を、`papr.db` の `rt_nodes` を
/// SQL で読んで作る。sai-paper は sai-sci に依存しないため、ここに置いてある。
/// 消えた Node（`removed_at` がある）も、`get_node` と同じく返す。
/// 表が無い・行が無いときは `None`。
pub fn rt_label(conn: &Connection, node_id: i64) -> Option<String> {
    let kind: String = conn
        .query_row("SELECT kind FROM rt_nodes WHERE id = ?1", [node_id], |row| {
            row.get(0)
        })
        .ok()?;
    Some(format!("{kind}{node_id}"))
}

/// セットの全候補を取得する。
///
/// # Errors
///
/// セットが見つからない、または DB エラーの場合。
pub fn items(conn: &Connection, set_id: i64) -> Result<Vec<SearchSetItem>> {
    let mut stmt = conn.prepare(
        "SELECT position, remote_json, dedupe_key, paper_id, decision, reason, added_at
         FROM search_set_items WHERE set_id = ?1 ORDER BY position",
    )?;

    let rows: Vec<_> = stmt
        .query_map([set_id], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<i64>>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, Option<String>>(5)?,
                row.get::<_, String>(6)?,
            ))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;

    let mut items = Vec::new();
    for (position, remote_json, dedupe_key, paper_id, decision, reason, added_at) in rows {
        let remote = serde_json::from_str(&remote_json)?;
        items.push(SearchSetItem {
            position,
            remote,
            dedupe_key,
            paper_id,
            decision,
            reason,
            added_at,
        });
    }

    Ok(items)
}

/// セットの 1 件を取得する。
///
/// # Errors
///
/// セットが見つからない、または DB エラーの場合。
pub fn get(conn: &Connection, set_id: i64) -> Result<SearchSet> {
    conn.query_row(
        "SELECT id, name, source_dbs, query, raw_query, created_by, note, created_at
         FROM search_sets WHERE id = ?1",
        [set_id],
        |row| {
            Ok(SearchSet {
                id: row.get(0)?,
                name: row.get(1)?,
                source_dbs: row.get(2)?,
                query: row.get(3)?,
                raw_query: row.get(4)?,
                created_by: row.get(5)?,
                note: row.get(6)?,
                created_at: row.get(7)?,
            })
        },
    )
    .optional()?
    .ok_or_else(|| SearchSetError::NotFound(format!("セット ID {} が見つかりません", set_id)))
}

/// 名前でセットを取得する。
///
/// # Errors
///
/// セットが見つからない、または DB エラーの場合。
pub fn get_by_name(conn: &Connection, name: &str) -> Result<SearchSet> {
    conn.query_row(
        "SELECT id, name, source_dbs, query, raw_query, created_by, note, created_at
         FROM search_sets WHERE name = ?1",
        [name],
        |row| {
            Ok(SearchSet {
                id: row.get(0)?,
                name: row.get(1)?,
                source_dbs: row.get(2)?,
                query: row.get(3)?,
                raw_query: row.get(4)?,
                created_by: row.get(5)?,
                note: row.get(6)?,
                created_at: row.get(7)?,
            })
        },
    )
    .optional()?
    .ok_or_else(|| SearchSetError::NotFound(format!("セット '{}' が見つかりません", name)))
}

/// すべてのセットを一覧する。
///
/// # Errors
///
/// DB エラーの場合。
pub fn list(conn: &Connection) -> Result<Vec<SearchSet>> {
    let mut stmt = conn.prepare(
        "SELECT id, name, source_dbs, query, raw_query, created_by, note, created_at
         FROM search_sets ORDER BY created_at DESC",
    )?;

    let sets = stmt
        .query_map([], |row| {
            Ok(SearchSet {
                id: row.get(0)?,
                name: row.get(1)?,
                source_dbs: row.get(2)?,
                query: row.get(3)?,
                raw_query: row.get(4)?,
                created_by: row.get(5)?,
                note: row.get(6)?,
                created_at: row.get(7)?,
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;

    Ok(sets)
}

/// 候補の判定を更新する。
///
/// # Errors
///
/// 候補が見つからない、または DB エラーの場合。
pub fn mark(
    conn: &Connection,
    set_id: i64,
    position: i64,
    decision: &str,
    reason: Option<&str>,
) -> Result<()> {
    let rows = conn.execute(
        "UPDATE search_set_items SET decision = ?1, reason = ?2
         WHERE set_id = ?3 AND position = ?4",
        rusqlite::params![decision, reason, set_id, position],
    )?;

    if rows == 0 {
        return Err(SearchSetError::NotFound(format!(
            "候補 set_id={}, position={} が見つかりません",
            set_id, position
        )));
    }

    Ok(())
}

/// 複数の候補を一括で判定を更新する。
///
/// # Errors
///
/// DB エラーの場合。
pub fn mark_many(
    conn: &Connection,
    set_id: i64,
    positions: Vec<i64>,
    decision: &str,
    reason: Option<&str>,
) -> Result<()> {
    for position in positions {
        mark(conn, set_id, position, decision, reason)?;
    }
    Ok(())
}

/// セットをマージして新しいセットを作成する。
///
/// 複数のセットの候補を新しいセットに集約し、重複を排除する。
///
/// # Errors
///
/// セットが見つからない、または DB エラーの場合。
pub fn merge(
    conn: &Connection,
    source_set_ids: Vec<i64>,
    new_name: String,
    created_by: &str,
) -> Result<i64> {
    // 新セットを作成
    let new_set_id = create(
        conn,
        CreateSearchSetParams {
            name: new_name,
            source_dbs: String::new(),
            query: String::new(),
            raw_query: None,
            created_by: created_by.to_string(),
            note: None,
        },
    )?;

    // 各ソースセットから候補を取得して追加
    let mut all_papers = Vec::new();
    for set_id in source_set_ids {
        let items = items(conn, set_id)?;
        for item in items {
            all_papers.push(item.remote);
        }
    }

    add_items(conn, new_set_id, all_papers)?;

    Ok(new_set_id)
}

/// セットを削除する。
///
/// # Errors
///
/// セットが見つからない、または DB エラーの場合。
pub fn delete(conn: &Connection, set_id: i64) -> Result<()> {
    let rows = conn.execute("DELETE FROM search_sets WHERE id = ?1", [set_id])?;

    if rows == 0 {
        return Err(SearchSetError::NotFound(format!(
            "セット ID {} が見つかりません",
            set_id
        )));
    }

    Ok(())
}

/// 検索セットをもう一度実行した結果（S03・共有契約）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RerunResult {
    /// 実行したセットの ID。
    pub set_id: i64,
    /// 新しく足した候補の数。
    pub new_items: usize,
    /// 重複を除いた検索のヒット数。
    pub total_hits: usize,
    /// 失敗した DB の名前（1 つでも成功していればここに残る）。
    pub failed_dbs: Vec<String>,
}

/// 検索セットをもう一度実行するための検索条件（S03）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RerunPlan {
    /// セットの ID。
    pub set_id: i64,
    /// セットの名前。
    pub name: String,
    /// 検索する DB（`source_dbs` をカンマで分けたもの）。
    pub source_dbs: Vec<String>,
    /// 自然言語のクエリ。
    pub query: String,
    /// DB 固有構文のクエリ。
    pub raw_query: Option<String>,
    /// 発行年の下限。
    pub year_from: Option<u16>,
    /// 発行年の上限。
    pub year_to: Option<u16>,
    /// 1 回の検索で取る件数。
    pub limit: u16,
    /// この再実行の時刻（ISO8601。`searched_at` と新着候補の `added_at` に使う）。
    pub searched_at: String,
}

/// 保存した検索を実行するための条件を組み立てる。
///
/// `set` はセット名または ID。`manual` だけのセット（手で作ったもの）は
/// 検索の条件が無いのでエラーにする。
///
/// # Errors
///
/// セットが見つからない、手で作ったセット、DB エラーのとき。
pub fn plan_rerun(conn: &Connection, set: &str) -> Result<RerunPlan> {
    // ID として解析を試み、だめなら名前で引く（CLI の resolve_set_id と同じ規則）。
    let id = set.parse::<i64>().ok().filter(|id| get(conn, *id).is_ok());
    let set_row = match id {
        Some(id) => get(conn, id)?,
        None => get_by_name(conn, set)?,
    };

    // year / limit_txt は SearchSet に無いので直接引く。
    let (year, limit_txt): (Option<String>, Option<String>) = conn.query_row(
        "SELECT year, limit_txt FROM search_sets WHERE id = ?1",
        [set_row.id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;

    let source_dbs: Vec<String> = set_row
        .source_dbs
        .split(',')
        .map(str::trim)
        .filter(|db| !db.is_empty())
        .map(str::to_string)
        .collect();
    if source_dbs.is_empty() || source_dbs == ["manual".to_string()] {
        return Err(SearchSetError::Invalid(
            "手で作ったセットはもう一度実行できません".to_string(),
        ));
    }

    let (year_from, year_to) = parse_year(year.as_deref().unwrap_or(""));
    let limit = limit_txt
        .as_deref()
        .and_then(|value| value.trim().parse::<u16>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(20);

    Ok(RerunPlan {
        set_id: set_row.id,
        name: set_row.name,
        source_dbs,
        query: set_row.query,
        raw_query: set_row.raw_query,
        year_from,
        year_to,
        limit,
        searched_at: chrono::Local::now().to_rfc3339(),
    })
}

/// `"2025-2026"` → `(2025, 2026)`、`"2025"` → `(2025, 2025)`、空 → 指定なし。
///
/// 数字として読めない側は指定なし（`None`）にする。
#[must_use]
pub fn parse_year(value: &str) -> (Option<u16>, Option<u16>) {
    let value = value.trim();
    if value.is_empty() {
        return (None, None);
    }
    if let Some((from, to)) = value.split_once('-') {
        (from.trim().parse().ok(), to.trim().parse().ok())
    } else {
        let year = value.parse().ok();
        (year, year)
    }
}

/// 検索の結果をセットへ反映する（S03）。
///
/// - 同じ `dedupe_key` の候補は足さない。新しいものは `position` を続き番号で振る。
/// - 新着候補の `added_at` は `plan.searched_at` にする（新しさの判定に使う）。
/// - 全部の DB で失敗したときは、候補も `searched_at` も変えずエラーを返す。
/// - 検索の記録は `activity_log` の `discover` に `{"set_id","rerun":true,"new"}` で残す。
///
/// # Errors
///
/// 全部の DB で失敗した、DB / JSON エラーのとき。
pub fn apply_rerun(
    db: &Database,
    plan: &RerunPlan,
    dbs: &[String],
    papers: Vec<RemotePaper>,
    failed_dbs: &[String],
) -> Result<RerunResult> {
    // 1 つも成功しなかったときは何も変えない。
    if !dbs.is_empty() && !failed_dbs.is_empty() && failed_dbs.len() >= dbs.len() {
        return Err(SearchSetError::Invalid(format!(
            "検索がすべての DB で失敗しました: {}",
            failed_dbs.join(", ")
        )));
    }

    let total_hits = papers.len();
    let conn = db.connection();
    let added = add_items(conn, plan.set_id, papers)?;
    // 新着候補の `added_at` を検索時刻にそろえる（要対応の「新着」判定に使う）。
    if let Some(first) = added.first() {
        conn.execute(
            "UPDATE search_set_items SET added_at = ?1 WHERE set_id = ?2 AND position >= ?3",
            params![plan.searched_at, plan.set_id, *first as i64],
        )?;
    }
    // 検索した時刻と、Library の日付グループを今にそろえる。
    let log_date = chrono::Local::now().format("%Y/%m/%d").to_string();
    conn.execute(
        "UPDATE search_sets SET searched_at = ?1, log_date = ?2 WHERE id = ?3",
        params![plan.searched_at, log_date, plan.set_id],
    )?;

    // 新着の数を `activity_log` の `discover` に残す（新しい表は作らない）。
    let detail = serde_json::json!({
        "set_id": plan.set_id,
        "rerun": true,
        "new": added.len(),
    })
    .to_string();
    db.record_activity("discover", None, Some(&detail))?;

    Ok(RerunResult {
        set_id: plan.set_id,
        new_items: added.len(),
        total_hits,
        failed_dbs: failed_dbs.to_vec(),
    })
}

/// 保存した検索をもう一度実行し、新着だけをセットに足す（S03・共有契約）。
///
/// `set` はセット名または ID。
///
/// # Errors
///
/// セットが見つからない、手で作ったセット、検索が全部失敗、DB エラーのとき。
pub async fn rerun_set(db: &Database, cfg: &Config, set: &str) -> Result<RerunResult> {
    let plan = plan_rerun(db.connection(), set)?;
    let (dbs, papers, failed) = crate::api::run_search(
        cfg,
        &plan.query,
        &plan.source_dbs,
        plan.limit,
        plan.year_from,
        plan.year_to,
        plan.raw_query.as_deref(),
    )
    .await;
    let failed_dbs: Vec<String> = failed.into_iter().map(|(db_id, _)| db_id).collect();
    apply_rerun(db, &plan, &dbs, papers, &failed_dbs)
}

/// ダウンロード済みの論文をセット内の候補にリンクする。
///
/// `paper_id` を設定して `decision` を 'downloaded' に更新する。
///
/// # Errors
///
/// 候補が見つからない、または DB エラーの場合。
pub fn link_paper(conn: &Connection, set_id: i64, position: i64, paper_id: i64) -> Result<()> {
    let rows = conn.execute(
        "UPDATE search_set_items SET paper_id = ?1, decision = ?2
         WHERE set_id = ?3 AND position = ?4",
        rusqlite::params![paper_id, "downloaded", set_id, position],
    )?;

    if rows == 0 {
        return Err(SearchSetError::NotFound(format!(
            "候補 set_id={}, position={} が見つかりません",
            set_id, position
        )));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Database;

    fn create_test_papers() -> Vec<RemotePaper> {
        use chrono::Utc;

        vec![
            RemotePaper {
                id: "s2:1".to_string(),
                title: "Ferroptosis in glioma".to_string(),
                authors: vec!["Smith".to_string()],
                abstract_text: "Abstract 1".to_string(),
                published: Utc::now(),
                updated: Utc::now(),
                categories: vec![],
                pdf_url: None,
                doi: Some("10.1234/example1".to_string()),
                journal_ref: None,
                citation_count: None,
            },
            RemotePaper {
                id: "s2:2".to_string(),
                title: "Ferroptosis signaling".to_string(),
                authors: vec!["Jones".to_string()],
                abstract_text: "Abstract 2".to_string(),
                published: Utc::now(),
                updated: Utc::now(),
                categories: vec![],
                pdf_url: None,
                doi: None,
                journal_ref: None,
                citation_count: None,
            },
        ]
    }

    /// テスト用の論文を 1 件作る。
    fn extra_paper(id: &str, title: &str) -> RemotePaper {
        use chrono::Utc;

        RemotePaper {
            id: id.to_string(),
            title: title.to_string(),
            authors: vec!["Smith".to_string()],
            abstract_text: String::new(),
            published: Utc::now(),
            updated: Utc::now(),
            categories: vec![],
            pdf_url: None,
            doi: None,
            journal_ref: None,
            citation_count: None,
        }
    }

    // ---- S03: もう一度実行する ----

    #[test]
    fn parse_year_reads_range_single_and_empty() {
        assert_eq!(parse_year("2025-2026"), (Some(2025), Some(2026)));
        assert_eq!(parse_year("2025"), (Some(2025), Some(2025)));
        assert_eq!(parse_year(""), (None, None));
        assert_eq!(parse_year("  "), (None, None));
        assert_eq!(parse_year("abc"), (None, None));
    }

    #[test]
    fn rerun_adds_only_new_items_with_continuing_positions() {
        let db = Database::in_memory().unwrap();
        let conn = db.connection();
        let set_id = create(
            conn,
            CreateSearchSetParams {
                name: "rerun-new".to_string(),
                source_dbs: "s2,pubmed".to_string(),
                query: "ferroptosis".to_string(),
                raw_query: None,
                created_by: "user".to_string(),
                note: None,
            },
        )
        .unwrap();
        // 以前の検索時刻を古くしておく（更新されたかを確かめる）。
        conn.execute(
            "UPDATE search_sets SET searched_at = '2026-09-01T00:00:00+09:00' WHERE id = ?1",
            [set_id],
        )
        .unwrap();
        let first = create_test_papers();
        add_items(conn, set_id, first.clone()).unwrap();

        let plan = plan_rerun(conn, "rerun-new").unwrap();
        assert_eq!(plan.source_dbs, vec!["s2", "pubmed"]);
        assert_eq!(plan.limit, 20, "limit_txt が空なら 20");
        assert_eq!(plan.year_from, None);

        // 1 件は既存（同じ dedupe_key）、1 件は新規。
        let papers = vec![first[0].clone(), extra_paper("s2:3", "Ferroptosis review")];
        let result = apply_rerun(&db, &plan, &plan.source_dbs, papers, &[]).unwrap();

        assert_eq!(result.set_id, set_id);
        assert_eq!(result.new_items, 1);
        assert_eq!(result.total_hits, 2);
        assert!(result.failed_dbs.is_empty());

        let items = items(conn, set_id).unwrap();
        assert_eq!(items.len(), 3, "既存は足さず、新規だけ足す");
        assert_eq!(items[2].position, 2, "position は続き番号");
        assert_eq!(items[2].decision, "pending");
        assert_eq!(items[2].added_at, plan.searched_at);

        // searched_at が今の時刻に変わる。
        let searched_at: String = conn
            .query_row(
                "SELECT searched_at FROM search_sets WHERE id = ?1",
                [set_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(searched_at, plan.searched_at);

        // 新着の数が活動の記録に残る。
        let detail: Option<String> = conn
            .query_row(
                "SELECT detail FROM activity_log WHERE kind = 'discover'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let value: serde_json::Value = serde_json::from_str(detail.as_deref().unwrap()).unwrap();
        assert_eq!(value["set_id"], serde_json::json!(set_id));
        assert_eq!(value["rerun"], serde_json::json!(true));
        assert_eq!(value["new"], serde_json::json!(1));
    }

    #[test]
    fn rerun_keeps_everything_when_all_dbs_fail() {
        let db = Database::in_memory().unwrap();
        let conn = db.connection();
        let set_id = create(
            conn,
            CreateSearchSetParams {
                name: "rerun-fail".to_string(),
                source_dbs: "s2,pubmed".to_string(),
                query: "ferroptosis".to_string(),
                raw_query: None,
                created_by: "user".to_string(),
                note: None,
            },
        )
        .unwrap();
        conn.execute(
            "UPDATE search_sets SET searched_at = '2026-09-01T00:00:00+09:00' WHERE id = ?1",
            [set_id],
        )
        .unwrap();
        add_items(conn, set_id, create_test_papers()).unwrap();

        let plan = plan_rerun(conn, "rerun-fail").unwrap();
        // 検索の部分を差し替えたのと同じく、全部失敗の結果を渡す。
        let failed = vec!["s2".to_string(), "pubmed".to_string()];
        let result = apply_rerun(&db, &plan, &plan.source_dbs, Vec::new(), &failed);

        assert!(result.is_err(), "全部失敗ならエラー");
        // 候補も searched_at も変わらない。
        assert_eq!(items(conn, set_id).unwrap().len(), 2);
        let searched_at: String = conn
            .query_row(
                "SELECT searched_at FROM search_sets WHERE id = ?1",
                [set_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(searched_at, "2026-09-01T00:00:00+09:00");
        // rerun の記録も残さない。
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM activity_log WHERE kind = 'discover'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn plan_rerun_rejects_manual_sets_and_reports_missing() {
        let db = Database::in_memory().unwrap();
        let conn = db.connection();
        create(
            conn,
            CreateSearchSetParams {
                name: "manual-set".to_string(),
                source_dbs: "manual".to_string(),
                query: String::new(),
                raw_query: None,
                created_by: "user".to_string(),
                note: None,
            },
        )
        .unwrap();

        let error = plan_rerun(conn, "manual-set").unwrap_err();
        assert!(error.to_string().contains("手で作ったセット"), "{error}");
        assert!(plan_rerun(conn, "no-such-set").is_err());
    }

    #[test]
    fn test_create_and_get() {
        let db = Database::in_memory().unwrap();
        let conn = db.connection();

        let set_id = create(
            conn,
            CreateSearchSetParams {
                name: "test1".to_string(),
                source_dbs: "s2".to_string(),
                query: "ferroptosis".to_string(),
                raw_query: None,
                created_by: "user".to_string(),
                note: None,
            },
        )
        .unwrap();

        let set = get(conn, set_id).unwrap();
        assert_eq!(set.name, "test1");
        assert_eq!(set.query, "ferroptosis");
    }

    #[test]
    fn test_add_items_with_dedupe() {
        let db = Database::in_memory().unwrap();
        let conn = db.connection();

        let set_id = create(
            conn,
            CreateSearchSetParams {
                name: "test2".to_string(),
                source_dbs: "s2".to_string(),
                query: "ferroptosis".to_string(),
                raw_query: None,
                created_by: "user".to_string(),
                note: None,
            },
        )
        .unwrap();

        let papers = create_test_papers();
        let added1 = add_items(conn, set_id, papers.clone()).unwrap();
        assert_eq!(added1.len(), 2);

        // 同じ papers を追加しても重複排除により追加されない
        let added2 = add_items(conn, set_id, papers).unwrap();
        assert!(added2.is_empty());

        let items = items(conn, set_id).unwrap();
        assert_eq!(items.len(), 2);
    }

    #[test]
    fn test_mark() {
        let db = Database::in_memory().unwrap();
        let conn = db.connection();

        let set_id = create(
            conn,
            CreateSearchSetParams {
                name: "test3".to_string(),
                source_dbs: "s2".to_string(),
                query: "ferroptosis".to_string(),
                raw_query: None,
                created_by: "user".to_string(),
                note: None,
            },
        )
        .unwrap();

        let papers = create_test_papers();
        add_items(conn, set_id, papers).unwrap();

        mark(conn, set_id, 0, "keep", Some("Important paper")).unwrap();

        let items = items(conn, set_id).unwrap();
        assert_eq!(items[0].decision, "keep");
        assert_eq!(items[0].reason, Some("Important paper".to_string()));
    }

    #[test]
    fn test_merge() {
        let db = Database::in_memory().unwrap();
        let conn = db.connection();

        let set1_id = create(
            conn,
            CreateSearchSetParams {
                name: "set1".to_string(),
                source_dbs: "s2".to_string(),
                query: "ferroptosis".to_string(),
                raw_query: None,
                created_by: "user".to_string(),
                note: None,
            },
        )
        .unwrap();

        let set2_id = create(
            conn,
            CreateSearchSetParams {
                name: "set2".to_string(),
                source_dbs: "pubmed".to_string(),
                query: "glioma".to_string(),
                raw_query: None,
                created_by: "user".to_string(),
                note: None,
            },
        )
        .unwrap();

        add_items(conn, set1_id, create_test_papers()).unwrap();
        add_items(conn, set2_id, create_test_papers()).unwrap();

        let merged_id = merge(conn, vec![set1_id, set2_id], "merged".to_string(), "user").unwrap();

        let merged_items = items(conn, merged_id).unwrap();
        // 重複排除されるので 2 件
        assert_eq!(merged_items.len(), 2);
    }

    #[test]
    fn test_link_paper() {
        let db = Database::in_memory().unwrap();
        let conn = db.connection();

        // Create a paper first
        conn.execute("INSERT INTO papers (title) VALUES (?1)", ["Test Paper"])
            .unwrap();
        let paper_id = conn.last_insert_rowid();

        let set_id = create(
            conn,
            CreateSearchSetParams {
                name: "test4".to_string(),
                source_dbs: "s2".to_string(),
                query: "ferroptosis".to_string(),
                raw_query: None,
                created_by: "user".to_string(),
                note: None,
            },
        )
        .unwrap();

        let papers = create_test_papers();
        add_items(conn, set_id, papers).unwrap();

        link_paper(conn, set_id, 0, paper_id).unwrap();

        let items = items(conn, set_id).unwrap();
        assert_eq!(items[0].paper_id, Some(paper_id));
        assert_eq!(items[0].decision, "downloaded");
    }

    // ---- 「何のための検索か」 ----

    /// 候補はいつも `pending` で入る（墓標は無い）。
    #[test]
    fn add_items_inserts_pending() {
        let db = Database::in_memory().unwrap();
        let conn = db.connection();
        let set_id = create(
            conn,
            CreateSearchSetParams {
                name: "pend".to_string(),
                source_dbs: "s2".to_string(),
                query: "ferroptosis".to_string(),
                raw_query: None,
                created_by: "user".to_string(),
                note: None,
            },
        )
        .unwrap();
        add_items(conn, set_id, create_test_papers()).unwrap();
        let listed = items(conn, set_id).unwrap();
        assert_eq!(listed.len(), 2);
        assert!(listed.iter().all(|item| item.decision == "pending" && item.reason.is_none()));
    }

    /// `set_sci_fields` は検索セットの `rt_node`・`purpose`・`extract` に書く。無いセットはエラー。
    #[test]
    fn set_sci_fields_writes_columns() {
        let db = Database::in_memory().unwrap();
        let conn = db.connection();
        let set_id = create(
            conn,
            CreateSearchSetParams {
                name: "sci-set".to_string(),
                source_dbs: "s2".to_string(),
                query: "ferroptosis".to_string(),
                raw_query: None,
                created_by: "user".to_string(),
                note: None,
            },
        )
        .unwrap();

        set_sci_fields(
            conn,
            set_id,
            &SciSetFields {
                node: Some(7),
                question: Some(3),
                purpose: Some("手法の候補を探す".to_string()),
                extract: Some("Dataset と Loss".to_string()),
            },
        )
        .unwrap();

        let row: (Option<i64>, Option<i64>, Option<String>, Option<String>) = conn
            .query_row(
                "SELECT rt_node, sci_question, purpose, extract FROM search_sets WHERE id = ?1",
                [set_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();
        assert_eq!(row.0, Some(7));
        // `question` は読まず、古い列は NULL のまま。
        assert_eq!(row.1, None);
        assert_eq!(row.2.as_deref(), Some("手法の候補を探す"));
        assert_eq!(row.3.as_deref(), Some("Dataset と Loss"));

        // 無いセットは Err。
        assert!(set_sci_fields(conn, 9999, &SciSetFields::default()).is_err());
    }
}
