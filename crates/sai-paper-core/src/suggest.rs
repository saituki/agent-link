//! Suggest（agent が勧める論文）の保存。
//!
//! 候補は検索履歴の `(history_id, position)` を指す。論文の情報は写さない（Web の「＋ Library に入れる」は
//! 検索履歴の `log_id` と `position` をそのまま使う）。`history_id` に外部キーは付けない（移行 0063）。

use chrono::Local;
use rusqlite::{Connection, OptionalExtension, Row, params};
use thiserror::Error;

/// Suggest 操作のエラー。
#[derive(Debug, Error)]
pub enum SuggestError {
    /// SQLite 操作に失敗した。
    #[error("database error: {0}")]
    Database(#[from] rusqlite::Error),
    /// 対象が見つからない（候補の検索履歴が無い）。
    #[error("not found: {0}")]
    NotFound(String),
}

/// 保存する候補 1 件。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewItem {
    /// 検索履歴 ID（`search_history.id`）。
    pub history_id: i64,
    /// 検索履歴の中の位置（0 始まり）。
    pub position: i64,
    /// 勧める理由 1 行（無ければ `None`）。
    pub note: Option<String>,
}

/// Suggest 1 件の見出し。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Suggest {
    /// 主キー（`paper_suggests.id`）。
    pub id: i64,
    /// 作成日時（ISO8601）。
    pub created_at: String,
    /// 一覧のグループ（`YYYY/MM/DD`）。
    pub date: String,
    /// 実行者（`human` / `agent`）。
    pub executer: String,
    /// sai-link の実行 ID（agent のときだけ）。
    pub run_id: Option<i64>,
    /// 実行した block 名（無ければ `None`）。
    pub block: Option<String>,
    /// 勧めた理由（Markdown）。
    pub reason: String,
    /// 保存した候補の数。
    pub count: i64,
}

const SELECT_SUGGEST: &str =
    "SELECT s.id, s.created_at, s.date, s.executer, s.run_id, s.block, s.reason,
        (SELECT COUNT(*) FROM paper_suggest_items i WHERE i.suggest_id = s.id)
 FROM paper_suggests s";

/// Suggest を 1 件作り、候補を順に保存する。作った Suggest の ID を返す。
///
/// 候補の検索履歴（`history_id`・`position`）が 1 つでも無ければ、何も書かずに `NotFound` を返す。
///
/// # Errors
///
/// 候補が無いときは `NotFound`、DB エラーのときは `Database`。
pub fn add(
    conn: &Connection,
    executer: &str,
    run_id: Option<i64>,
    block: Option<&str>,
    reason: &str,
    items: &[NewItem],
) -> Result<i64, SuggestError> {
    let now = Local::now();
    let tx = conn.unchecked_transaction()?;
    for item in items {
        let found: Option<i64> = tx
            .query_row(
                "SELECT 1 FROM search_history_items WHERE history_id = ?1 AND position = ?2",
                params![item.history_id, item.position],
                |row| row.get(0),
            )
            .optional()?;
        if found.is_none() {
            return Err(SuggestError::NotFound(format!(
                "検索履歴の候補 log={}, position={} が見つかりません",
                item.history_id, item.position
            )));
        }
    }
    tx.execute(
        "INSERT INTO paper_suggests (created_at, date, executer, run_id, block, reason)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            now.to_rfc3339(),
            now.format("%Y/%m/%d").to_string(),
            executer,
            run_id,
            block,
            reason
        ],
    )?;
    let id = tx.last_insert_rowid();
    for (ord, item) in (0_i64..).zip(items) {
        tx.execute(
            "INSERT INTO paper_suggest_items (suggest_id, ord, history_id, position, note)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![id, ord, item.history_id, item.position, item.note],
        )?;
    }
    tx.commit()?;
    Ok(id)
}

/// Suggest を新しい順（`created_at` の降順、同じなら ID の降順）に一覧する。
///
/// # Errors
///
/// DB エラーの場合。
pub fn list(conn: &Connection) -> Result<Vec<Suggest>, SuggestError> {
    let mut stmt = conn.prepare(&format!(
        "{SELECT_SUGGEST} ORDER BY s.created_at DESC, s.id DESC"
    ))?;
    let rows = stmt
        .query_map([], row_to_suggest)?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Suggest 1 件と、勧めた候補（`ord` の順）を取得する。無ければ `None`。
///
/// # Errors
///
/// DB エラーの場合。
pub fn get(conn: &Connection, id: i64) -> Result<Option<(Suggest, Vec<NewItem>)>, SuggestError> {
    let Some(suggest) = conn
        .query_row(
            &format!("{SELECT_SUGGEST} WHERE s.id = ?1"),
            [id],
            row_to_suggest,
        )
        .optional()?
    else {
        return Ok(None);
    };
    let mut stmt = conn.prepare(
        "SELECT history_id, position, note FROM paper_suggest_items WHERE suggest_id = ?1 ORDER BY ord",
    )?;
    let items = stmt
        .query_map([id], |row| {
            Ok(NewItem {
                history_id: row.get(0)?,
                position: row.get(1)?,
                note: row.get(2)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Some((suggest, items)))
}

fn row_to_suggest(row: &Row) -> rusqlite::Result<Suggest> {
    Ok(Suggest {
        id: row.get(0)?,
        created_at: row.get(1)?,
        date: row.get(2)?,
        executer: row.get(3)?,
        run_id: row.get(4)?,
        block: row.get(5)?,
        reason: row.get(6)?,
        count: row.get(7)?,
    })
}

#[cfg(test)]
mod tests {
    use super::{NewItem, SuggestError, add, get, list};
    use crate::database::Database;
    use crate::search_history::record;
    use rusqlite::{Connection, params};

    /// 検索履歴を 1 件作り、候補を `n` 件入れる。ログ ID を返す。
    fn log_with_items(conn: &Connection, n: i64) -> i64 {
        let log = record(conn, "phone", "human", None, "q", "", "10", "s2", &[]).unwrap();
        for position in 0..n {
            conn.execute(
                "INSERT INTO search_history_items (history_id, position, remote_json, dedupe_key)
                 VALUES (?1, ?2, '{}', ?3)",
                params![log, position, format!("k{position}")],
            )
            .unwrap();
        }
        log
    }

    /// add → get で、候補の順と理由（note）が戻る。実行者と block も残る。
    #[test]
    fn add_then_get_keeps_order_and_notes() {
        let db = Database::in_memory().unwrap();
        let conn = db.connection();
        let log = log_with_items(conn, 3);
        let items = vec![
            NewItem {
                history_id: log,
                position: 2,
                note: Some("手法が近い".into()),
            },
            NewItem {
                history_id: log,
                position: 0,
                note: None,
            },
        ];
        let id = add(
            conn,
            "agent",
            Some(7),
            Some("lab-search"),
            "探した理由",
            &items,
        )
        .unwrap();

        let (suggest, got) = get(conn, id).unwrap().expect("保存した Suggest が読める");
        assert_eq!(suggest.count, 2);
        assert_eq!(suggest.executer, "agent");
        assert_eq!(suggest.run_id, Some(7));
        assert_eq!(suggest.block.as_deref(), Some("lab-search"));
        assert_eq!(suggest.reason, "探した理由");
        assert_eq!(got, items);
        assert!(get(conn, id + 1).unwrap().is_none());
    }

    /// 無い候補があれば NotFound。Suggest も候補も 1 行も増えない。
    #[test]
    fn missing_candidate_is_not_found_and_writes_nothing() {
        let db = Database::in_memory().unwrap();
        let conn = db.connection();
        let log = log_with_items(conn, 1);
        let items = vec![
            NewItem {
                history_id: log,
                position: 0,
                note: None,
            },
            NewItem {
                history_id: log,
                position: 9,
                note: None,
            },
        ];
        let err = add(conn, "human", None, None, "r", &items).unwrap_err();
        assert!(matches!(err, SuggestError::NotFound(_)), "{err:?}");
        let count = |sql: &str| -> i64 { conn.query_row(sql, [], |row| row.get(0)).unwrap() };
        assert_eq!(count("SELECT COUNT(*) FROM paper_suggests"), 0);
        assert_eq!(count("SELECT COUNT(*) FROM paper_suggest_items"), 0);
    }

    /// list は新しい Suggest を先頭にする。
    #[test]
    fn list_is_newest_first() {
        let db = Database::in_memory().unwrap();
        let conn = db.connection();
        let log = log_with_items(conn, 1);
        let items = vec![NewItem {
            history_id: log,
            position: 0,
            note: None,
        }];
        let first = add(conn, "human", None, None, "a", &items).unwrap();
        let second = add(conn, "human", None, None, "b", &items).unwrap();
        let ids: Vec<i64> = list(conn).unwrap().iter().map(|s| s.id).collect();
        assert_eq!(ids, vec![second, first]);
    }
}
