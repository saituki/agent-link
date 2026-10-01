//! `sai-moon-paper attention` — 要対応の一覧と「対応済み」の記録（S02）。
//!
//! - `attention [--json]`: 要対応（書いていない review・質問・現況の更新案・
//!   推薦のカード・出典の要確認）を出す。LLM は使わない。
//! - `attention resolve <key>`: 1 件を「対応済み」にする
//!   （`activity_log` に `kind = 'resolved'` を残す）。

use anyhow::{Context, Result};
use clap::Subcommand;
use sai_paper_core::attention::{self, AttentionItem, AttentionKind};
use sai_paper_core::paths::paper_roots;

use crate::open_database;

/// `sai-moon-paper attention` のサブコマンド。
#[derive(Subcommand, Debug)]
pub enum AttentionCmd {
    /// 要対応の 1 件を「対応済み」にする（key は一覧の `key`）
    Resolve {
        /// 対応済みにする key（例: `question:41:対象は?`）
        key: String,
    },
}

/// `sai-moon-paper attention` を実行する。
pub fn handle(cmd: Option<AttentionCmd>, json: bool) -> Result<()> {
    match cmd {
        Some(AttentionCmd::Resolve { key }) => resolve(&key),
        None => list(json),
    }
}

/// 要対応の一覧を出す。
fn list(json: bool) -> Result<()> {
    let db = open_database()?;
    let roots = paper_roots().context("プロジェクトルート（.sai/paper）が見つかりません")?;
    let items = attention::collect(db.connection(), &roots)?;

    if json {
        println!("{}", serde_json::to_string_pretty(&items)?);
        return Ok(());
    }

    if items.is_empty() {
        println!("要対応はありません");
        return Ok(());
    }
    println!("要対応 {} 件", items.len());
    // 種類の順に並べて出す。
    let order = [
        AttentionKind::EmptyReview,
        AttentionKind::Question,
        AttentionKind::StateUpdate,
        AttentionKind::RecommendCard,
        AttentionKind::ReportIssue,
        AttentionKind::NewSetItems,
        AttentionKind::SciDecision,
        AttentionKind::SciTriage,
    ];
    for kind in order {
        let group: Vec<&AttentionItem> = items.iter().filter(|item| item.kind == kind).collect();
        if group.is_empty() {
            continue;
        }
        println!("\n## {}", kind_heading(kind));
        for item in group {
            println!("  [{}] {}  ({})", kind.label(), item.title, item.key);
        }
    }
    Ok(())
}

/// 種類の見出し。
fn kind_heading(kind: AttentionKind) -> &'static str {
    match kind {
        AttentionKind::EmptyReview => "書いていない review",
        AttentionKind::Question => "ユーザーへの質問",
        AttentionKind::StateUpdate => "現況の更新案",
        AttentionKind::RecommendCard => "推薦のカード",
        AttentionKind::ReportIssue => "出典の要確認",
        AttentionKind::NewSetItems => "検索セットの新着",
        AttentionKind::SciDecision => "AWAITING_DECISION の ITEM",
        AttentionKind::SciTriage => "TRIAGE 待ちの PROPOSAL",
    }
}

/// 1 件を「対応済み」にする。
fn resolve(key: &str) -> Result<()> {
    let db = open_database()?;
    let roots = paper_roots().context("プロジェクトルート（.sai/paper）が見つかりません")?;
    let items = attention::collect(db.connection(), &roots)?;
    let item = items
        .iter()
        .find(|item| item.key == key)
        .with_context(|| format!("要対応の key が見つかりません: {key}"))?;
    attention::resolve(&db, item)?;
    println!("対応済みにしました: {key}");
    Ok(())
}
