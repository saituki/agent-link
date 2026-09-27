//! `sai-paper content` — 論文本文 / 引用箇所 / report の内容を標準出力へ出す。
//!
//! sai-link は task.md の入力展開（`paper_id` / `chunk_id` / `report_id`）で
//! このコマンドを呼ぶ。**papr.db を直接読ませない**（core を唯一の真実とする）。
//!
//! `--json` では `{ "id": ..., "title": ..., "text": ... }` を返す。

use anyhow::{Context, Result};
use clap::Subcommand;
use sai_paper_core::{Paths, citation, figures, reports};

use crate::open_database;

/// `sai-paper content` のサブコマンド。
#[derive(Subcommand, Debug)]
pub enum ContentCmd {
    /// 論文本文（extract.md）の全文
    Paper {
        /// 論文 ID（papers.id）
        paper_id: i64,
        /// JSON で出力する
        #[arg(long)]
        json: bool,
    },
    /// 引用箇所（chunks の 1 行）の本文
    Chunk {
        /// chunk_id（chunks.id）
        chunk_id: i64,
        /// JSON で出力する
        #[arg(long)]
        json: bool,
    },
    /// report の md 全文
    Report {
        /// report_id（例: R-0001）
        report_id: String,
        /// JSON で出力する
        #[arg(long)]
        json: bool,
    },
}

/// `sai-paper content` を実行する。
pub fn handle_content(cmd: ContentCmd) -> Result<()> {
    match cmd {
        ContentCmd::Paper { paper_id, json } => print_paper(paper_id, json),
        ContentCmd::Chunk { chunk_id, json } => print_chunk(chunk_id, json),
        ContentCmd::Report { report_id, json } => print_report(&report_id, json),
    }
}

/// 論文本文（extract.md）を出力する。
fn print_paper(paper_id: i64, json: bool) -> Result<()> {
    let db = open_database()?;
    let conn = db.connection();
    let title: Option<String> = conn
        .query_row(
            "SELECT title FROM papers WHERE id = ?1",
            [paper_id],
            |row| row.get(0),
        )
        .ok()
        .flatten();
    let title = title.with_context(|| format!("論文が見つかりません: id={paper_id}"))?;

    // 抽出キーは無くてもよい（S2 ハッシュのみの論文でも papers.id から解決できる）。
    let key = figures::extract_key_for_paper(conn, paper_id)?;
    let data_dir = Paths::discover()?.data_dir().to_path_buf();
    let md_path = citation::resolve_extract_md(conn, Some(paper_id), key.as_deref(), &data_dir)
        .with_context(|| format!("論文 {paper_id} の extract.md が見つかりません（抽出未実行？）"))?;
    let text = std::fs::read_to_string(&md_path)
        .with_context(|| format!("extract.md の読み込みに失敗: {}", md_path.display()))?;

    if json {
        println!(
            "{}",
            serde_json::json!({
                "id": paper_id,
                "title": title,
                "text": text,
                "md_path": md_path.to_string_lossy(),
            })
        );
    } else {
        print!("{text}");
    }
    // エージェントが論文を読んだ記録（失敗しても本文の出力は止めない）。
    crate::activity::record_agent_read("cli", "paper", Some(paper_id), Some(&md_path));
    Ok(())
}

/// 引用箇所（chunks の 1 行）を出力する。
fn print_chunk(chunk_id: i64, json: bool) -> Result<()> {
    let db = open_database()?;
    let conn = db.connection();
    let row: Option<(String, Option<i64>, String)> = conn
        .query_row(
            "SELECT text, paper_id, source_kind FROM chunks WHERE id = ?1",
            [chunk_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .ok();
    let (text, paper_id, source_kind) =
        row.with_context(|| format!("chunk が見つかりません: id={chunk_id}"))?;
    let title = paper_id
        .and_then(|id| {
            conn.query_row("SELECT title FROM papers WHERE id = ?1", [id], |row| {
                row.get::<_, Option<String>>(0)
            })
            .ok()
            .flatten()
        })
        .unwrap_or_default();

    if json {
        println!(
            "{}",
            serde_json::json!({
                "id": chunk_id,
                "title": title,
                "paper_id": paper_id,
                "text": text,
            })
        );
    } else {
        print!("{text}");
    }
    // エージェントが読んだ記録。断片の出どころで種類を変える（失敗は無視）。
    let target = if source_kind.starts_with("note") {
        "note"
    } else if source_kind == "abstract" {
        "paper"
    } else {
        "extract"
    };
    crate::activity::record_agent_read("cli", target, paper_id, None);
    Ok(())
}

/// report の md 全文を出力する。
fn print_report(report_id: &str, json: bool) -> Result<()> {
    let db = open_database()?;
    let conn = db.connection();
    let report = reports::get_report(conn, report_id)
        .context("report の取得に失敗")?
        .with_context(|| format!("report が見つかりません: {report_id}"))?;
    let text = reports::read_report_markdown(conn, report_id)
        .with_context(|| format!("report 本文の読み出しに失敗: {report_id}"))?;
    if json {
        println!(
            "{}",
            serde_json::json!({
                "id": report.id,
                "title": report.title,
                "text": text,
            })
        );
    } else {
        print!("{text}");
    }
    // エージェントが report を読んだ記録（失敗しても本文の出力は止めない）。
    let report_path = reports::resolve_md_path(&report);
    crate::activity::record_agent_read("cli", "report", None, report_path.as_deref());
    Ok(())
}
