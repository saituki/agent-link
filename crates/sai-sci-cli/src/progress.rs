//! `sai-sci progress` — プロジェクトの進み具合と「読むべき論文」（N05）。
//!
//! LLM を使わず、DB とファイル（`state.md`・引き継ぎ・note）から
//! [`sai_sci_core::progress::collect`] でまとめる。人向けの要約と、
//! 推薦・DSR が読む `--json`（`Progress` と `read_next` 5 本）を出す。

use std::fmt::Write as _;

use anyhow::{Context, Result};
use sai_sci_core::progress as progress_core;
use sai_sci_core::progress::{Progress, ReadNext};
use sai_sci_core::understanding;
use sai_paper_core::{Config, Paths, seen_ids};

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
        let text = format!("{}\n", serde_json::to_string_pretty(&output)?);
        seen_ids::record(&text);
        print!("{text}");
        return Ok(());
    }

    let mut text = String::new();
    render_human(&mut text, &progress, &read_next)?;
    seen_ids::record(&text);
    print!("{text}");
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

/// 人向けの要約を `out` に書く（1 行ずつ改行で終える）。
fn render_human(out: &mut String, progress: &Progress, read_next: &[ReadNext]) -> std::fmt::Result {
    match progress.stage.as_deref() {
        Some(stage) => writeln!(out, "段階: {stage}")?,
        None => writeln!(out, "段階: 未記入（state.md の「- 段階:」）")?,
    }

    writeln!(out, "READ NEXT ─ 読むべき論文（上位 {READ_NEXT_LIMIT} 本）")?;
    if read_next.is_empty() {
        writeln!(out, "  （読むべき論文はありません）")?;
    }
    for entry in read_next {
        writeln!(
            out,
            "  #{} {}  {}",
            entry.paper_id,
            truncate(&entry.title, 60),
            entry.reason
        )?;
    }

    render_pairs(out, "未解決", &progress.unresolved)?;
    render_pairs(out, "ユーザーへの質問", &progress.open_questions)?;

    writeln!(out, "理解の状態")?;
    if progress.understanding.is_empty() {
        writeln!(out, "  （まだ記録がありません）")?;
    }
    for entry in &progress.understanding {
        writeln!(
            out,
            "  {}  {}",
            understanding::state_label(&entry.state),
            entry.topic
        )?;
    }

    let unopened = progress
        .reports
        .iter()
        .filter(|report| report.user_opens == 0)
        .count();
    writeln!(
        out,
        "開かれていない report: {unopened} 件（全 {} 件）",
        progress.reports.len()
    )
}

/// `(run_id, 本文)` の一覧を区別して最大 [`LINES_LIMIT`] 件書く。
fn render_pairs(out: &mut String, label: &str, pairs: &[(i64, String)]) -> std::fmt::Result {
    writeln!(out, "{label}（{} 件）", pairs.len())?;
    if pairs.is_empty() {
        return writeln!(out, "  （なし）");
    }
    for (run_id, text) in pairs.iter().take(LINES_LIMIT) {
        writeln!(out, "  run {run_id}: {}", single_line(text))?;
    }
    if pairs.len() > LINES_LIMIT {
        writeln!(out, "  …（残り {} 件）", pairs.len() - LINES_LIMIT)?;
    }
    Ok(())
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
