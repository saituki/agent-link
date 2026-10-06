//! `sai-moon-paper understanding` — トピックごとの理解の状態の記録（N04）。
//!
//! - `understanding add`: 「理解の確認」の DSR が、**ユーザーの確認を取った後**に
//!   1 件足す。状態は `ok`（理解している）/ `vague`（あいまい）/ `unknown`
//!   （まだ確かめていない）。
//! - `understanding list`: 記録を一覧する。既定はトピックごとの最新の 1 件、
//!   `--all` で履歴を全部出す。

use anyhow::{Context, Result};
use clap::Subcommand;
use sai_sci_core::understanding::{self, UnderstandingEntry};

use crate::open_database;

/// `sai-moon-paper understanding` のサブコマンド。
#[derive(Subcommand, Debug)]
pub enum UnderstandingCmd {
    /// 理解の状態を 1 件足す（ユーザーの確認の後に呼ぶ）
    Add {
        /// トピック名
        #[arg(long)]
        topic: String,
        /// 理解の状態（ok / vague / unknown）
        #[arg(long)]
        state: String,
        /// 判定の根拠（自由文）
        #[arg(long, default_value = "")]
        evidence: String,
        /// 判定した実行 ID
        #[arg(long)]
        run_id: Option<i64>,
    },
    /// 理解の状態を一覧する（既定はトピックごとの最新）
    List {
        /// 履歴を全部出す
        #[arg(long)]
        all: bool,
        /// JSON で出力する
        #[arg(long)]
        json: bool,
    },
}

/// `sai-moon-paper understanding` を実行する。
pub fn handle(cmd: UnderstandingCmd) -> Result<()> {
    match cmd {
        UnderstandingCmd::Add {
            topic,
            state,
            evidence,
            run_id,
        } => handle_add(&topic, &state, &evidence, run_id),
        UnderstandingCmd::List { all, json } => handle_list(all, json),
    }
}

/// `understanding add` の本体。
fn handle_add(topic: &str, state: &str, evidence: &str, run_id: Option<i64>) -> Result<()> {
    let db = open_database()?;
    let id = understanding::add(db.connection(), topic, state, evidence, run_id)
        .with_context(|| format!("理解の記録を足せません: {topic} / {state}"))?;
    println!(
        "理解を記録しました: {} {}（id {id}）",
        understanding::state_label(state),
        topic.trim()
    );
    Ok(())
}

/// `understanding list` の本体。
fn handle_list(all: bool, json: bool) -> Result<()> {
    let db = open_database()?;
    let conn = db.connection();
    let entries = if all {
        understanding::history(conn, None)?
    } else {
        understanding::latest_per_topic(conn)?
    };

    if json {
        println!("{}", serde_json::to_string_pretty(&entries)?);
        return Ok(());
    }

    for entry in &entries {
        println!("{}", format_line(entry));
    }
    Ok(())
}

/// 1 件を 1 行にする。
///
/// 例: `あいまい  前処理の違い  （run 52）ノッチフィルタの理由を説明できなかった`
fn format_line(entry: &UnderstandingEntry) -> String {
    let mut line = format!(
        "{}  {}",
        understanding::state_label(&entry.state),
        entry.topic
    );
    if let Some(run_id) = entry.run_id {
        line.push_str(&format!("  （run {run_id}）"));
    } else if !entry.evidence.is_empty() {
        // run が無いときは、topic と根拠がくっつかないように空ける。
        line.push_str("  ");
    }
    if !entry.evidence.is_empty() {
        line.push_str(&entry.evidence);
    }
    line
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_shows_state_topic_run_and_evidence() {
        let entry = UnderstandingEntry {
            id: 3,
            topic: "前処理の違い".to_string(),
            state: "vague".to_string(),
            evidence: "ノッチフィルタの理由を説明できなかった".to_string(),
            run_id: Some(52),
            created_at: "2026-09-25 12:00:00".to_string(),
        };
        assert_eq!(
            format_line(&entry),
            "あいまい  前処理の違い  （run 52）ノッチフィルタの理由を説明できなかった"
        );
    }

    #[test]
    fn line_without_run_keeps_evidence_apart() {
        let entry = UnderstandingEntry {
            id: 5,
            topic: "前処理の違い".to_string(),
            state: "vague".to_string(),
            evidence: "説明できなかった".to_string(),
            run_id: None,
            created_at: "2026-09-25 12:00:00".to_string(),
        };
        assert_eq!(
            format_line(&entry),
            "あいまい  前処理の違い  説明できなかった"
        );
    }

    #[test]
    fn line_without_run_or_evidence_is_short() {
        let entry = UnderstandingEntry {
            id: 4,
            topic: "評価の分け方".to_string(),
            state: "unknown".to_string(),
            evidence: String::new(),
            run_id: None,
            created_at: "2026-09-25 12:00:00".to_string(),
        };
        assert_eq!(format_line(&entry), "まだ確かめていない  評価の分け方");
    }
}
