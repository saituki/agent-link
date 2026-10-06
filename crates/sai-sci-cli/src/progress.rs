//! `sai-sci progress` — プロジェクトの進み具合と「読むべき論文」（N05）。
//!
//! LLM を使わず、DB とファイル（`state.md`・引き継ぎ・note）から
//! [`sai_sci_core::progress::collect`] でまとめる。人向けの要約と、
//! 推薦・DSR が読む `--json`（`Progress` と `read_next` 5 本）を出す。

use anyhow::{Context, Result};
use sai_sci_core::progress as progress_core;
use sai_sci_core::progress::{Progress, ReadNext};
use sai_sci_core::understanding;
use sai_paper_core::{Config, Paths};

use crate::open_database;

/// 「読むべき論文」として出す本数。
const READ_NEXT_LIMIT: usize = 5;

/// 「未解決」「ユーザーへの質問」として出す最大件数。
const LINES_LIMIT: usize = 5;

/// `sai-sci progress` を実行する。
pub fn handle(json: bool) -> Result<()> {
    let db = open_database()?;
    let roots = sai_paper_core::paths::paper_roots()
        .context("プロジェクトルート（.sai/paper）が見つかりません")?;
    let progress = progress_core::collect(db.connection(), &roots)?;
    let read_next = progress_core::read_next(&progress, &read_weights(), READ_NEXT_LIMIT);

    if json {
        let output = serde_json::json!({ "progress": progress, "read_next": read_next });
        println!("{}", serde_json::to_string_pretty(&output)?);
        return Ok(());
    }

    print_human(&progress, &read_next);
    Ok(())
}

/// 設定 `progress.read_next` の重みを読む（読めなければ既定）。
///
/// `progress` は読むだけのコマンドなので、設定ファイルを作らない
/// [`Config::load`] を使う。
fn read_weights() -> sai_paper_core::ReadNextWeights {
    Paths::discover()
        .ok()
        .and_then(|paths| Config::load(&paths.config_file).ok())
        .map(|config| config.progress.read_next)
        .unwrap_or_default()
}

/// 人向けの要約を出す。
fn print_human(progress: &Progress, read_next: &[ReadNext]) {
    match progress.stage.as_deref() {
        Some(stage) => println!("段階: {stage}"),
        None => println!("段階: 未記入（state.md の「- 段階:」）"),
    }

    println!("READ NEXT ─ 読むべき論文（上位 {READ_NEXT_LIMIT} 本）");
    if read_next.is_empty() {
        println!("  （読むべき論文はありません）");
    }
    for entry in read_next {
        println!(
            "  #{} {}  {}",
            entry.paper_id,
            truncate(&entry.title, 60),
            entry.reason
        );
    }

    print_pairs("未解決", &progress.unresolved);
    print_pairs("ユーザーへの質問", &progress.open_questions);

    println!("理解の状態");
    if progress.understanding.is_empty() {
        println!("  （まだ記録がありません）");
    }
    for entry in &progress.understanding {
        println!(
            "  {}  {}",
            understanding::state_label(&entry.state),
            entry.topic
        );
    }

    let unopened = progress
        .reports
        .iter()
        .filter(|report| report.user_opens == 0)
        .count();
    println!(
        "開かれていない report: {unopened} 件（全 {} 件）",
        progress.reports.len()
    );
}

/// `(run_id, 本文)` の一覧を区別して最大 [`LINES_LIMIT`] 件出す。
fn print_pairs(label: &str, pairs: &[(i64, String)]) {
    println!("{label}（{} 件）", pairs.len());
    if pairs.is_empty() {
        println!("  （なし）");
        return;
    }
    for (run_id, text) in pairs.iter().take(LINES_LIMIT) {
        println!("  run {run_id}: {}", single_line(text));
    }
    if pairs.len() > LINES_LIMIT {
        println!("  …（残り {} 件）", pairs.len() - LINES_LIMIT);
    }
}

/// 引き継ぎの本文を 1 行にする（改行を空白に、長すぎれば切る）。
fn single_line(text: &str) -> String {
    truncate(&text.split_whitespace().collect::<Vec<_>>().join(" "), 100)
}

/// 表示幅（文字数）で切って末尾に `…` を付ける。
fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let head: String = text.chars().take(max.saturating_sub(1)).collect();
    format!("{head}…")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_keeps_short_and_cuts_long() {
        assert_eq!(truncate("短い", 10), "短い");
        let long = "あ".repeat(20);
        let cut = truncate(&long, 5);
        assert_eq!(cut.chars().count(), 5);
        assert!(cut.ends_with('…'));
    }

    #[test]
    fn single_line_joins_newlines() {
        assert_eq!(single_line("前処理の\n手順"), "前処理の 手順");
    }
}
