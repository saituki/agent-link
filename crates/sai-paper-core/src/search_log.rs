//! 検索の記録（Library の階層表示・QUERY 画面・phone が読む窓口）。
//!
//! 記録そのものは検索セット（`search_sets` / `search_set_items`）1 つにまとめてある
//! （段階 6）。Discover・phone で検索すると検索セットが 1 つ自動で作られ、CLI・
//! エージェントが作った検索セットも同じ一覧に並ぶ。ここでの「ログ ID」は検索セットの ID。
//!
//! ダウンロード状態は `papers` テーブルと突合して判定する。

use crate::models::RemotePaper;
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// 検索実行ログ操作のエラー。
#[derive(Debug, Error)]
pub enum SearchLogError {
    /// SQLite 操作に失敗した。
    #[error("database error: {0}")]
    Database(#[from] rusqlite::Error),
    /// JSON の変換に失敗した。
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
}

/// 検索実行ログのメタデータ。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SearchLog {
    /// 主キー。
    pub id: i64,
    /// 検索実行日時（ISO8601）。
    pub searched_at: String,
    /// 階層表示の日付グループ（`YYYY/MM/DD`）。
    pub log_date: String,
    /// 検索ワード。
    pub query: String,
    /// 発行年区間（例: `2010-2020`）。
    pub year: String,
    /// 取得件数（入力値そのまま）。
    pub limit_txt: String,
    /// 検索対象 DB（カンマ区切り）。
    pub source_dbs: String,
    /// QUERY 画面で入力する検索理由。
    pub reason: Option<String>,
}

/// 検索実行ログのヒット論文 1 件。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SearchLogItem {
    /// ログ内での位置（0 始まり）。
    pub position: i64,
    /// リモート API が返したメタデータ。
    pub remote: RemotePaper,
    /// 重複排除キー。
    pub dedupe_key: String,
    /// この候補が Library に取り込まれた論文 ID（未取り込みなら `None`）。
    /// `search_set_items.paper_id` を読む。階層表示はこれを先に使う。
    #[serde(default)]
    pub paper_id: Option<i64>,
}

/// 検索実行ログを 1 件作成し、ヒット論文を保存する。作成したログ ID を返す。
///
/// # Errors
///
/// DB / JSON エラーの場合。
pub fn record_search(
    conn: &Connection,
    origin: &str,
    query: &str,
    year: &str,
    limit_txt: &str,
    source_dbs: &str,
    papers: &[RemotePaper],
) -> Result<i64, SearchLogError> {
    let now = chrono::Local::now();
    let searched_at = now.to_rfc3339();
    let log_date = now.format("%Y/%m/%d").to_string();
    // 名前は重ならないよう、窓口と日時（ナノ秒まで）から作る。
    conn.execute(
        "INSERT INTO search_sets (name, source_dbs, query, created_by, created_at,
                                  year, limit_txt, origin, searched_at, log_date)
         VALUES (?1, ?2, ?3, 'user', ?4, ?5, ?6, ?7, ?4, ?8)",
        params![
            format!("{origin}-{searched_at}"),
            source_dbs,
            query,
            searched_at,
            year,
            limit_txt,
            origin,
            log_date
        ],
    )?;
    let log_id = conn.last_insert_rowid();
    append_items(conn, log_id, papers)?;
    Ok(log_id)
}

/// 検索を実行した sai-link の実行 ID を記録に残す（Agent の検索。`None` なら何もしない）。
///
/// 実行 ID があれば `origin` も `agent` にする（窓口が人の検索と見分けるため）。
///
/// # Errors
///
/// DB エラーの場合。
pub fn set_run_id(conn: &Connection, log_id: i64, run_id: Option<i64>) -> Result<(), SearchLogError> {
    if let Some(run_id) = run_id {
        conn.execute(
            "UPDATE search_sets SET run_id = ?1, origin = 'agent' WHERE id = ?2",
            params![run_id, log_id],
        )?;
    }
    Ok(())
}

/// 既存のログにヒット論文を追記する（重複排除キーで重複はスキップ）。
///
/// Discover の「さらに読み込む（r）」で追加取得した分を同じログへ反映する。
///
/// # Errors
///
/// DB / JSON エラーの場合。
pub fn append_items(
    conn: &Connection,
    log_id: i64,
    papers: &[RemotePaper],
) -> Result<(), SearchLogError> {
    let mut stmt =
        conn.prepare("SELECT 1 FROM search_set_items WHERE set_id = ?1 AND dedupe_key = ?2")?;
    let max_pos: i64 = conn
        .query_row(
            "SELECT COALESCE(MAX(position), -1) FROM search_set_items WHERE set_id = ?1",
            [log_id],
            |r| r.get(0),
        )
        .unwrap_or(-1);
    let mut position = max_pos + 1;
    for paper in papers {
        let dedupe_key = crate::api::dedupe_key(paper);
        let exists = stmt
            .query_row(params![log_id, &dedupe_key], |_| Ok(true))
            .ok()
            .is_some();
        if exists {
            continue;
        }
        conn.execute(
            "INSERT INTO search_set_items (set_id, position, remote_json, dedupe_key)
             VALUES (?1, ?2, ?3, ?4)",
            params![log_id, position, serde_json::to_string(paper)?, dedupe_key],
        )?;
        position += 1;
    }
    Ok(())
}

/// 検索実行ログを新しい順に一覧する。
///
/// # Errors
///
/// DB エラーの場合。
pub fn list(conn: &Connection) -> Result<Vec<SearchLog>, SearchLogError> {
    let mut stmt = conn.prepare(
        // 検索語が空（CLI・エージェントが作った検索セットは名前だけ）のときは、
        // 名前のまま出す（Library 階層表示・QUERY 画面・phone が同じ値を共有する）。
        "SELECT id, COALESCE(searched_at, created_at), COALESCE(log_date, ''),
                COALESCE(NULLIF(query, ''), name), year,
                limit_txt, source_dbs, note
         FROM search_sets ORDER BY COALESCE(searched_at, created_at) DESC, id DESC",
    )?;
    let rows = stmt
        .query_map([], |row| {
            Ok(SearchLog {
                id: row.get(0)?,
                searched_at: row.get(1)?,
                log_date: row.get(2)?,
                query: row.get(3)?,
                year: row.get::<_, Option<String>>(4)?.unwrap_or_default(),
                limit_txt: row.get::<_, Option<String>>(5)?.unwrap_or_default(),
                source_dbs: row.get::<_, Option<String>>(6)?.unwrap_or_default(),
                reason: row.get(7)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// 1 件のログを取得する。
///
/// # Errors
///
/// DB エラーの場合。
pub fn get(conn: &Connection, log_id: i64) -> Result<Option<SearchLog>, SearchLogError> {
    let log = conn
        .query_row(
            "SELECT id, COALESCE(searched_at, created_at), COALESCE(log_date, ''),
                    COALESCE(NULLIF(query, ''), name), year,
                    limit_txt, source_dbs, note
             FROM search_sets WHERE id = ?1",
            [log_id],
            |row| {
                Ok(SearchLog {
                    id: row.get(0)?,
                    searched_at: row.get(1)?,
                    log_date: row.get(2)?,
                    query: row.get(3)?,
                    year: row.get::<_, Option<String>>(4)?.unwrap_or_default(),
                    limit_txt: row.get::<_, Option<String>>(5)?.unwrap_or_default(),
                    source_dbs: row.get::<_, Option<String>>(6)?.unwrap_or_default(),
                    reason: row.get(7)?,
                })
            },
        )
        .ok();
    Ok(log)
}

/// ログのヒット論文を位置順に取得する。
///
/// # Errors
///
/// DB / JSON エラーの場合。
pub fn items(conn: &Connection, log_id: i64) -> Result<Vec<SearchLogItem>, SearchLogError> {
    let mut stmt = conn.prepare(
        "SELECT position, remote_json, dedupe_key, paper_id
         FROM search_set_items WHERE set_id = ?1 ORDER BY position",
    )?;
    let rows: Vec<(i64, String, String, Option<i64>)> = stmt
        .query_map([log_id], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    let mut items = Vec::with_capacity(rows.len());
    for (position, remote_json, dedupe_key, paper_id) in rows {
        items.push(SearchLogItem {
            position,
            remote: serde_json::from_str(&remote_json)?,
            dedupe_key,
            paper_id,
        });
    }
    Ok(items)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Database;

    fn remote(id: &str, title: &str, doi: Option<&str>) -> RemotePaper {
        RemotePaper {
            id: id.to_string(),
            title: title.to_string(),
            authors: vec!["A".into()],
            abstract_text: String::new(),
            published: chrono::Utc::now(),
            updated: chrono::Utc::now(),
            categories: Vec::new(),
            pdf_url: None,
            doi: doi.map(str::to_string),
            journal_ref: None,
            citation_count: None,
        }
    }

    #[test]
    fn records_and_reads_back_a_search_log() -> Result<(), Box<dyn std::error::Error>> {
        let db = Database::in_memory()?;
        let conn = db.connection();
        let papers = vec![
            remote("2401.00001", "First", None),
            remote("2401.00002", "Second", Some("10.1/x")),
            // 同じ DOI の重複は 1 件にまとまる
            remote("2401.00003", "Second dup", Some("10.1/x")),
        ];
        let log_id = record_search(conn, "discover", "LLM Agent", "2010-2026", "100", "s2,pubmed", &papers)?;
        let logs = list(conn)?;
        assert_eq!(logs.len(), 1);
        assert_eq!(logs[0].query, "LLM Agent");
        assert_eq!(logs[0].source_dbs, "s2,pubmed");
        assert_eq!(logs[0].log_date.len(), 10, "YYYY/MM/DD 形式");

        let items = items(conn, log_id)?;
        assert_eq!(items.len(), 2, "DOI 重複は排除される");
        assert_eq!(items[0].remote.title, "First");

        Ok(())
    }

    #[test]
    fn append_items_skips_already_recorded_papers() -> Result<(), Box<dyn std::error::Error>> {
        let db = Database::in_memory()?;
        let conn = db.connection();
        let first = vec![remote("2401.00001", "First", None)];
        let log_id = record_search(conn, "discover", "q", "", "10", "s2", &first)?;
        // 既存 1 件 + 新規 1 件を追記する。
        let more = vec![
            remote("2401.00001", "First", None),
            remote("2401.00002", "Second", None),
        ];
        append_items(conn, log_id, &more)?;
        let items = items(conn, log_id)?;
        assert_eq!(items.len(), 2);
        Ok(())
    }
}
