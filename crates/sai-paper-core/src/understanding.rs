//! トピックごとの「ユーザーの理解の状態」の記録（N04）。
//!
//! - `ok` : 理解している
//! - `vague` : あいまい
//! - `unknown` : まだ確かめていない
//!
//! 書き足していくだけで履歴を残す。画面（Project ページ）と brief は
//! [`latest_per_topic`] が返すトピックごとの最新の 1 件を使う。
//!
//! 保存するのは「理解の確認」の DSR が**ユーザーの確認を取った後**だけである
//! （判定を勝手に保存しないというテーマの方針）。このモジュールはその保存先を
//! 提供するだけで、判定はしない。

use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// `state` に書ける 3 つの値。
pub const STATES: [&str; 3] = ["ok", "vague", "unknown"];

/// 理解の記録 1 件。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UnderstandingEntry {
    /// 主キー。
    pub id: i64,
    /// トピック名。
    pub topic: String,
    /// 理解の状態（`ok` / `vague` / `unknown`）。
    pub state: String,
    /// 判定の根拠（自由文。空でもよい）。
    pub evidence: String,
    /// 判定した実行 ID（`run_id`。無ければ `None`）。
    pub run_id: Option<i64>,
    /// 記録日時（`CURRENT_TIMESTAMP` の形）。
    pub created_at: String,
}

/// 理解の記録操作のエラー。
#[derive(Debug, Error)]
pub enum UnderstandingError {
    /// SQLite 操作に失敗した。
    #[error("database error: {0}")]
    Database(#[from] rusqlite::Error),
    /// トピックが空白だけだった。
    #[error("topic が空です")]
    EmptyTopic,
    /// `state` が 3 つの値のどれでもない。
    #[error("state は ok / vague / unknown のどれかにしてください: {0}")]
    InvalidState(String),
}

/// 理解の状態を表す日本語のラベル（CLI・brief・Project ページで共有する）。
#[must_use]
pub fn state_label(state: &str) -> &'static str {
    match state {
        "ok" => "理解している",
        "vague" => "あいまい",
        "unknown" => "まだ確かめていない",
        _ => "不明",
    }
}

/// 理解の記録を 1 件足し、その ID を返す。
///
/// `topic` は前後の空白を除いて保存する。`state` が [`STATES`] のどれでも
/// 無ければ保存せずに [`UnderstandingError::InvalidState`] を返す。
///
/// # Errors
///
/// DB 操作・入力の検査に失敗したとき。
pub fn add(
    conn: &Connection,
    topic: &str,
    state: &str,
    evidence: &str,
    run_id: Option<i64>,
) -> Result<i64, UnderstandingError> {
    let topic = topic.trim();
    if topic.is_empty() {
        return Err(UnderstandingError::EmptyTopic);
    }
    if !STATES.contains(&state) {
        return Err(UnderstandingError::InvalidState(state.to_string()));
    }
    conn.execute(
        "INSERT INTO understanding (topic, state, evidence, run_id)
         VALUES (?1, ?2, ?3, ?4)",
        params![topic, state, evidence, run_id],
    )?;
    Ok(conn.last_insert_rowid())
}

/// トピックごとの最新の 1 件を返す。
///
/// 「最新」は `created_at` の新しい方、同じ時刻なら `id` の大きい方。
/// 並びは `vague` → `unknown` → `ok`、同じ状態の中は新しい順。
///
/// # Errors
///
/// DB 操作に失敗したとき。
pub fn latest_per_topic(conn: &Connection) -> Result<Vec<UnderstandingEntry>, UnderstandingError> {
    let mut stmt = conn.prepare(
        "SELECT id, topic, state, evidence, run_id, created_at
         FROM (
             SELECT id, topic, state, evidence, run_id, created_at,
                    ROW_NUMBER() OVER (
                        PARTITION BY topic ORDER BY created_at DESC, id DESC
                    ) AS rn
             FROM understanding
         )
         WHERE rn = 1
         ORDER BY CASE state
                      WHEN 'vague' THEN 0
                      WHEN 'unknown' THEN 1
                      WHEN 'ok' THEN 2
                      ELSE 3
                  END,
                  created_at DESC, id DESC",
    )?;
    let rows = stmt
        .query_map([], row_to_entry)?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// 記録を新しい順に返す。`topic` を渡すとそのトピックだけに絞る。
///
/// # Errors
///
/// DB 操作に失敗したとき。
pub fn history(
    conn: &Connection,
    topic: Option<&str>,
) -> Result<Vec<UnderstandingEntry>, UnderstandingError> {
    let mut stmt = conn.prepare(
        "SELECT id, topic, state, evidence, run_id, created_at
         FROM understanding
         WHERE (?1 IS NULL OR topic = ?1)
         ORDER BY created_at DESC, id DESC",
    )?;
    let rows = stmt
        .query_map(params![topic], row_to_entry)?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// 1 行を [`UnderstandingEntry`] にする。
fn row_to_entry(row: &rusqlite::Row<'_>) -> rusqlite::Result<UnderstandingEntry> {
    Ok(UnderstandingEntry {
        id: row.get(0)?,
        topic: row.get(1)?,
        state: row.get(2)?,
        evidence: row.get(3)?,
        run_id: row.get(4)?,
        created_at: row.get(5)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Database;

    /// トピックの最新 1 件と並び（vague → unknown → ok）を確かめる。
    #[test]
    fn latest_per_topic_keeps_newest_and_orders_by_state() -> Result<(), Box<dyn std::error::Error>>
    {
        let db = Database::in_memory()?;
        let conn = db.connection();

        // 同じトピックに 3 件。最後の 1 件が最新（同じ時刻でも id が大きい方）。
        add(conn, "前処理の違い", "vague", "最初", Some(50))?;
        add(conn, "前処理の違い", "ok", "次", Some(51))?;
        let latest_id = add(
            conn,
            "前処理の違い",
            "vague",
            "ノッチフィルタの理由を説明できなかった",
            Some(52),
        )?;
        // 別のトピックに 1 件。
        add(conn, "評価の分け方", "unknown", "", None)?;

        let latest = latest_per_topic(conn)?;
        assert_eq!(latest.len(), 2, "トピックごとに 1 件: {latest:?}");
        // 並びは vague → unknown。
        assert_eq!(latest[0].state, "vague");
        assert_eq!(latest[0].topic, "前処理の違い");
        assert_eq!(latest[0].id, latest_id, "最新の 1 件が選ばれる");
        assert_eq!(latest[0].evidence, "ノッチフィルタの理由を説明できなかった");
        assert_eq!(latest[0].run_id, Some(52));
        assert_eq!(latest[1].state, "unknown");
        assert_eq!(latest[1].topic, "評価の分け方");

        // history は履歴を全部返す（新しい順、topic で絞れる）。
        let all = history(conn, None)?;
        assert_eq!(all.len(), 4);
        let only = history(conn, Some("前処理の違い"))?;
        assert_eq!(only.len(), 3);
        Ok(())
    }

    /// ok だけが 2 つあるとき、vague・unknown の後ろに新しい順で並ぶ。
    #[test]
    fn latest_per_topic_orders_ok_last() -> Result<(), Box<dyn std::error::Error>> {
        let db = Database::in_memory()?;
        let conn = db.connection();
        add(conn, "公開データセットの違い", "ok", "", None)?;
        add(conn, "まだ確かめていない話", "unknown", "", None)?;
        add(conn, "あいまいな話", "vague", "", None)?;

        let states: Vec<String> = latest_per_topic(conn)?
            .into_iter()
            .map(|entry| entry.state)
            .collect();
        assert_eq!(states, vec!["vague", "unknown", "ok"]);
        Ok(())
    }

    /// `state` の値の検査（`add` と CHECK の両方）。
    #[test]
    fn invalid_state_is_rejected() -> Result<(), Box<dyn std::error::Error>> {
        let db = Database::in_memory()?;
        let conn = db.connection();

        // add が断る。
        let error = add(conn, "topic", "good", "", None).unwrap_err();
        assert!(matches!(error, UnderstandingError::InvalidState(value) if value == "good"));
        // 空の topic も断る。
        assert!(matches!(
            add(conn, "   ", "ok", "", None).unwrap_err(),
            UnderstandingError::EmptyTopic
        ));

        // 表の CHECK も断る（add を通さない挿入）。
        let direct = conn.execute(
            "INSERT INTO understanding (topic, state) VALUES ('topic', 'good')",
            [],
        );
        assert!(direct.is_err(), "CHECK が効いていない");
        // 前後の空白を除いた topic が保存される。
        let id = add(conn, "  間隔  ", "ok", "", None)?;
        let topic: String = conn.query_row(
            "SELECT topic FROM understanding WHERE id = ?1",
            [id],
            |row| row.get(0),
        )?;
        assert_eq!(topic, "間隔");
        Ok(())
    }

    /// 空の表では最新も履歴も空を返す。
    #[test]
    fn empty_table_returns_empty() -> Result<(), Box<dyn std::error::Error>> {
        let db = Database::in_memory()?;
        let conn = db.connection();
        assert!(latest_per_topic(conn)?.is_empty());
        assert!(history(conn, None)?.is_empty());
        Ok(())
    }
}
