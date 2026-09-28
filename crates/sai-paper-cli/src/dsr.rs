//! `sai-moon-paper dsr` — DSR（研究用ブロック）の report を LLM を使わずに確かめるコマンド。
//!
//! `dsr check` は、report がブロックの `## 出力の形` 節の見出しを持っているかだけを見る。
//! 見出しの一覧はブロックの本文から読む（別のファイルに写すと 2 か所になるため）。

use anyhow::{Context, Result};
use clap::Subcommand;
use sai_paper_core::dsr_check::{self, CheckResult};
use sai_paper_core::paths::paper_roots;
use std::path::{Path, PathBuf};

/// `sai-moon-paper dsr` のサブコマンド。
#[derive(Subcommand, Debug)]
pub enum DsrCmd {
    /// DSR の report が出力の形の見出しを持っているか確かめる（LLM は使わない）
    Check {
        /// 確かめる report の Markdown
        report: PathBuf,
        /// ブロック ID
        #[arg(long)]
        block: String,
        /// JSON で出力する
        #[arg(long)]
        json: bool,
    },
}

/// `sai-moon-paper dsr` を実行する。
///
/// 終了コードは共有契約に従う（0 = 揃っている、1 = 足りない見出しがある、
/// 2 = ブロックが見つからない）。
pub fn handle(cmd: DsrCmd) -> Result<()> {
    match cmd {
        DsrCmd::Check {
            report,
            block,
            json,
        } => handle_check(&report, &block, json),
    }
}

/// `sai-moon-paper dsr check` の本体。足りない見出しがあれば終了コード 1 で終わる。
fn handle_check(report_path: &Path, block: &str, json: bool) -> Result<()> {
    let block_md = match read_block(block) {
        Ok(text) => text,
        Err(message) => {
            eprintln!("{message}");
            std::process::exit(2);
        }
    };

    let report_md = std::fs::read_to_string(report_path)
        .with_context(|| format!("report を読めません: {}", report_path.display()))?;

    let expected = dsr_check::expected_headings(&block_md);
    let result: CheckResult = dsr_check::check(&report_md, &expected);

    if json {
        let value = serde_json::json!({
            "block": block,
            "expected": result.expected,
            "missing": result.missing,
            "has_handoff": result.has_handoff,
        });
        println!("{}", serde_json::to_string(&value)?);
    } else {
        println!("{block} の出力の形: {} 見出し", result.expected);
        if result.missing.is_empty() {
            if result.has_handoff {
                println!("✅ 揃っています（引き継ぎあり）");
            } else {
                println!("✅ 揃っています（引き継ぎなし）");
            }
        } else {
            println!("⚠ 足りない見出し: {}", result.missing.join(", "));
        }
    }

    if result.missing.is_empty() {
        Ok(())
    } else {
        std::process::exit(1);
    }
}

/// ブロック ID から本文を読む。
///
/// `<root>/.sai/blocks/<id>.md` を探し、無ければ `~/.sai-moon/blocks/<id>.md` を探す。
fn read_block(block: &str) -> Result<String, String> {
    let file_name = format!("{block}.md");
    let mut candidates: Vec<PathBuf> = Vec::new();

    if let Some(roots) = paper_roots() {
        candidates.push(roots.root.join(".sai").join("blocks").join(&file_name));
    }
    if let Some(dir) = sai_paper_core::paths::sai_home_path("blocks") {
        candidates.push(dir.join(&file_name));
    }

    for path in &candidates {
        if path.is_file() {
            return std::fs::read_to_string(path)
                .map_err(|error| format!("ブロックを読めません: {} ({error})", path.display()));
        }
    }

    Err(format!("ブロックが見つかりません: {block}"))
}
