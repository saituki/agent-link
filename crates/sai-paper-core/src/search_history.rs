//! 検索の記録（Library の階層表示・QUERY 画面・phone が読む窓口）。
//!
//! 外部検索の履歴は `search_history` / `search_history_items` に 1 回の検索ごとに 1 行残す。
//! ここでの「ログ ID」は `search_history.id`。実行者は `executer`（`human` / `agent`）。
//!
//! ダウンロード状態は `papers` テーブルと突合して判定する。

use crate::models::RemotePaper;
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// 検索履歴操作のエラー。
#[derive(Debug, Error)]
pub enum SearchHistoryError {
    /// SQLite 操作に失敗した。
    #[error("database error: {0}")]
    Database(#[from] rusqlite::Error),
    /// JSON の変換に失敗した。
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    /// 対象が見つからない。
    #[error("not found: {0}")]
    NotFound(String),
}

/// 検索履歴のメタデータ。
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
    /// 経路（`phone` / `cli` / `discover` / `mcp`）。
    pub origin: String,
    /// 実行者（`human` / `agent`）。
    pub executer: String,
    /// sai-link の実行 ID（agent のときだけ）。
    pub run_id: Option<i64>,
}

/// 検索履歴のヒット論文 1 件。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SearchLogItem {
    /// ログ内での位置（0 始まり）。
    pub position: i64,
    /// リモート API が返したメタデータ。
    pub remote: RemotePaper,
    /// 重複排除キー。
    pub dedupe_key: String,
    /// この候補が Library に取り込まれた論文 ID（未取り込みなら `None`）。
    /// `search_history_items.paper_id` を読む。階層表示はこれを先に使う。
    #[serde(default)]
    pub paper_id: Option<i64>,
}

/// 検索履歴を 1 件作成し、ヒット論文を保存する。作成したログ ID を返す。
///
/// `origin` は経路（`phone` / `cli` / `discover` / `mcp`）、`executer` は実行者
/// （`human` / `agent`）。`run_id` は agent のときだけ渡す。
///
/// # Errors
///
/// DB / JSON エラーの場合。
#[allow(clippy::too_many_arguments)]
pub fn record(
    conn: &Connection,
    origin: &str,
    executer: &str,
    run_id: Option<i64>,
    query: &str,
    year: &str,
    limit_txt: &str,
    source_dbs: &str,
    papers: &[RemotePaper],
) -> Result<i64, SearchHistoryError> {
    let now = chrono::Local::now();
    let searched_at = now.to_rfc3339();
    let log_date = now.format("%Y/%m/%d").to_string();
    // 名前列は表に残っているだけの内部の値。重ならないよう、経路と日時（ナノ秒まで）から作る。
    conn.execute(
        "INSERT INTO search_history (name, source_dbs, query, created_by, created_at,
                                     year, limit_txt, origin, searched_at, log_date,
                                     executer, run_id)
         VALUES (?1, ?2, ?3, 'user', ?4, ?5, ?6, ?7, ?4, ?8, ?9, ?10)",
        params![
            format!("{origin}-{searched_at}"),
            source_dbs,
            query,
            searched_at,
            year,
            limit_txt,
            origin,
            log_date,
            executer,
            run_id
        ],
    )?;
    let log_id = conn.last_insert_rowid();
    append_items(conn, log_id, papers)?;
    Ok(log_id)
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
) -> Result<(), SearchHistoryError> {
    let mut stmt =
        conn.prepare("SELECT 1 FROM search_history_items WHERE history_id = ?1 AND dedupe_key = ?2")?;
    let max_pos: i64 = conn
        .query_row(
            "SELECT COALESCE(MAX(position), -1) FROM search_history_items WHERE history_id = ?1",
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
            "INSERT INTO search_history_items (history_id, position, remote_json, dedupe_key)
             VALUES (?1, ?2, ?3, ?4)",
            params![log_id, position, serde_json::to_string(paper)?, dedupe_key],
        )?;
        position += 1;
    }
    Ok(())
}

/// 検索履歴を新しい順に一覧する。
///
/// # Errors
///
/// DB エラーの場合。
pub fn list(conn: &Connection) -> Result<Vec<SearchLog>, SearchHistoryError> {
    let mut stmt = conn.prepare(
        // 検索語が空（CLI・エージェントが作った検索セットは名前だけ）のときは、
        // 名前のまま出す（Library 階層表示・QUERY 画面・phone が同じ値を共有する）。
        "SELECT id, COALESCE(searched_at, created_at), COALESCE(log_date, ''),
                COALESCE(NULLIF(query, ''), name), year,
                limit_txt, source_dbs, note, origin, executer, run_id
         FROM search_history ORDER BY COALESCE(searched_at, created_at) DESC, id DESC",
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
                origin: row.get(8)?,
                executer: row.get(9)?,
                run_id: row.get(10)?,
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
pub fn get(conn: &Connection, log_id: i64) -> Result<Option<SearchLog>, SearchHistoryError> {
    let log = conn
        .query_row(
            "SELECT id, COALESCE(searched_at, created_at), COALESCE(log_date, ''),
                    COALESCE(NULLIF(query, ''), name), year,
                    limit_txt, source_dbs, note, origin, executer, run_id
             FROM search_history WHERE id = ?1",
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
                    origin: row.get(8)?,
                    executer: row.get(9)?,
                    run_id: row.get(10)?,
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
pub fn items(conn: &Connection, log_id: i64) -> Result<Vec<SearchLogItem>, SearchHistoryError> {
    let mut stmt = conn.prepare(
        "SELECT position, remote_json, dedupe_key, paper_id
         FROM search_history_items WHERE history_id = ?1 ORDER BY position",
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

/// 検索履歴に「何のための検索か」を書く欄（R-R6）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SciSetFields {
    /// 結ぶ Node の番号。
    pub node: Option<i64>,
    /// 結ぶ問いの番号。
    pub question: Option<i64>,
    /// 何を確かめるための検索か。
    pub purpose: Option<String>,
    /// 検索結果から抜き出したい項目。
    pub extract: Option<String>,
}

/// 検索履歴に、何のための検索かを書く（Node・目的・抜き出す項目）。`node` は `rt_nodes.id`。
/// `question` は読まない（古い `sci_question` 列は NULL にする）。
///
/// # Errors
///
/// 履歴が無い、または DB エラーの場合。
pub fn set_sci_fields(conn: &Connection, log_id: i64, fields: &SciSetFields) -> Result<(), SearchHistoryError> {
    let updated = conn.execute(
        "UPDATE search_history
            SET rt_node = ?1, sci_question = NULL, purpose = ?3, extract = ?4
          WHERE id = ?5",
        params![
            fields.node,
            Option::<i64>::None,
            fields.purpose,
            fields.extract,
            log_id,
        ],
    )?;
    if updated == 0 {
        return Err(SearchHistoryError::NotFound(format!(
            "履歴 ID {} が見つかりません",
            log_id
        )));
    }
    Ok(())
}

/// research-tree の Node の番号から、外に出す形の文字（例 `O5`）を作る。
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

/// 取り込んだ論文を履歴の候補に結ぶ（`paper_id` だけを書く）。
///
/// # Errors
///
/// 候補が見つからない、または DB エラーの場合。
pub fn link_paper(
    conn: &Connection,
    log_id: i64,
    position: i64,
    paper_id: i64,
) -> Result<(), SearchHistoryError> {
    let rows = conn.execute(
        "UPDATE search_history_items SET paper_id = ?1
         WHERE history_id = ?2 AND position = ?3",
        params![paper_id, log_id, position],
    )?;
    if rows == 0 {
        return Err(SearchHistoryError::NotFound(format!(
            "候補 history_id={log_id}, position={position} が見つかりません"
        )));
    }
    Ok(())
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
    fn records_and_reads_back_a_search_history() -> Result<(), Box<dyn std::error::Error>> {
        let db = Database::in_memory()?;
        let conn = db.connection();
        let papers = vec![
            remote("2401.00001", "First", None),
            remote("2401.00002", "Second", Some("10.1/x")),
            // 同じ DOI の重複は 1 件にまとまる
            remote("2401.00003", "Second dup", Some("10.1/x")),
        ];
        let log_id = record(conn, "discover", "human", None, "LLM Agent", "2010-2026", "100", "s2,pubmed", &papers)?;
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
        let log_id = record(conn, "discover", "human", None, "q", "", "10", "s2", &first)?;
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
    #[test]
    fn link_paper_sets_paper_id_and_sci_fields_write_columns() -> Result<(), Box<dyn std::error::Error>> {
        let db = Database::in_memory()?;
        let conn = db.connection();
        conn.execute("INSERT INTO papers (title) VALUES ('P')", [])?;
        let paper_id = conn.last_insert_rowid();
        let log_id = record(
            conn,
            "cli",
            "agent",
            Some(7),
            "q",
            "",
            "10",
            "s2",
            &[remote("2401.00001", "First", None)],
        )?;
        link_paper(conn, log_id, 0, paper_id)?;
        assert_eq!(items(conn, log_id)?[0].paper_id, Some(paper_id));
        assert!(link_paper(conn, log_id, 5, paper_id).is_err());
        let log = get(conn, log_id)?.expect("row");
        assert_eq!((log.executer.as_str(), log.run_id, log.origin.as_str()), ("agent", Some(7), "cli"));

        set_sci_fields(
            conn,
            log_id,
            &SciSetFields {
                node: Some(7),
                question: Some(3),
                purpose: Some("手法の候補を探す".to_string()),
                extract: Some("Dataset と Loss".to_string()),
            },
        )?;
        let row: (Option<i64>, Option<i64>, Option<String>, Option<String>) = conn.query_row(
            "SELECT rt_node, sci_question, purpose, extract FROM search_history WHERE id = ?1",
            [log_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )?;
        assert_eq!(row.0, Some(7));
        assert_eq!(row.1, None);
        assert_eq!(row.2.as_deref(), Some("手法の候補を探す"));
        assert!(set_sci_fields(conn, 9999, &SciSetFields::default()).is_err());
        Ok(())
    }

    #[test]
    fn parse_year_reads_range_single_and_empty() {
        assert_eq!(parse_year("2025-2026"), (Some(2025), Some(2026)));
        assert_eq!(parse_year("2025"), (Some(2025), Some(2025)));
        assert_eq!(parse_year(""), (None, None));
    }
}
