//! `sai-moon-paper activity` — 「開いた」「エージェントが読んだ」の記録（N01）。
//!
//! - `activity open`: ユーザーが開いた記録を `activity_log` に足す。呼ぶ側
//!   （Web の画面など）を止めないため、**失敗しても 0 で終わる**
//!   （標準エラーに 1 行だけ出す）。
//! - `activity list`: 記録を新しい順に出す。
//!
//! エージェントが読んだ記録は `sai-moon-paper content` と MCP の `paper` から
//! [`record_agent_read`] で残す。

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use clap::Subcommand;
use sai_paper_core::activity;
use sai_paper_core::paths::paper_roots;
use rusqlite::params;
use serde_json::json;

use crate::open_database;

/// `sai-moon-paper activity` のサブコマンド。
#[derive(Subcommand, Debug)]
pub enum ActivityCmd {
    /// ユーザーが開いた記録を残す（失敗しても 0 で終わる）
    Open {
        /// 開いたファイルのパス
        #[arg(long)]
        path: PathBuf,
        /// 論文 ID（papers.id。省略時はパスから決める）
        #[arg(long)]
        paper: Option<i64>,
        /// 開いた画面（cli / sai-link / phone）
        #[arg(long, default_value = "cli")]
        surface: String,
        /// 記録の種類（省略時はパスから決める: note / extract / mind / state / report / review / canvas）
        #[arg(long)]
        target: Option<String>,
    },
    /// 記録を新しい順に一覧する
    List {
        /// 種類で絞る（open / agent_read / download / discover / notes / import / translate / radio）
        #[arg(long)]
        kind: Option<String>,
        /// 出す件数
        #[arg(long, default_value_t = 50)]
        limit: usize,
        /// JSON で出力する
        #[arg(long)]
        json: bool,
    },
}

/// `sai-moon-paper activity` を実行する。
pub fn handle(cmd: ActivityCmd) -> Result<()> {
    match cmd {
        ActivityCmd::Open {
            path,
            paper,
            surface,
            target,
        } => handle_open(&path, paper, &surface, target.as_deref()),
        ActivityCmd::List { kind, limit, json } => handle_list(kind.as_deref(), limit, json),
    }
}

/// `activity open` の本体。失敗しても 0 で終わる（標準エラーに 1 行だけ出す）。
fn handle_open(
    path: &Path,
    paper: Option<i64>,
    surface: &str,
    target: Option<&str>,
) -> Result<()> {
    if let Err(error) = record_open_inner(path, paper, surface, target) {
        eprintln!("activity open: {error}");
    }
    Ok(())
}

fn record_open_inner(
    path: &Path,
    paper: Option<i64>,
    surface: &str,
    target: Option<&str>,
) -> Result<()> {
    let roots = paper_roots().context("プロジェクトルートが見つかりません")?;
    let target = match target {
        Some(target) => target.to_string(),
        None => activity::open_target_for_path(&roots.root, path).to_string(),
    };
    let stored = activity::to_activity_path(&roots.root, &roots.data_dir, path);
    let paper_id = paper.or_else(|| activity::paper_id_for_path(Path::new(&stored)));
    let db = open_database()?;
    activity::record_open(&db, &target, surface, paper_id, &stored)?;
    Ok(())
}

fn handle_list(kind: Option<&str>, limit: usize, json: bool) -> Result<()> {
    let db = open_database()?;
    let conn = db.connection();
    let mut statement = conn.prepare(
        "SELECT id, kind, paper_id, detail, occurred_at
         FROM activity_log
         WHERE (?1 IS NULL OR kind = ?1)
         ORDER BY occurred_at DESC, id DESC
         LIMIT ?2",
    )?;
    let rows = statement.query_map(params![kind, limit as i64], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, Option<i64>>(2)?,
            row.get::<_, Option<String>>(3)?,
            row.get::<_, String>(4)?,
        ))
    })?;

    let mut items = Vec::new();
    for row in rows {
        let (id, kind, paper_id, detail, occurred_at) = row?;
        items.push(json!({
            "id": id,
            "kind": kind,
            "paper_id": paper_id,
            "detail": detail,
            "occurred_at": occurred_at,
        }));
    }

    if json {
        println!("{}", serde_json::to_string_pretty(&items)?);
    } else {
        for item in &items {
            let paper_id = item["paper_id"]
                .as_i64()
                .map(|id| id.to_string())
                .unwrap_or_else(|| "-".to_string());
            let detail = item["detail"].as_str().unwrap_or("");
            println!(
                "{}\t{}\t{}\t{}",
                item["occurred_at"].as_str().unwrap_or(""),
                item["kind"].as_str().unwrap_or(""),
                paper_id,
                detail
            );
        }
    }
    Ok(())
}

/// エージェントが読んだ記録を残す（[`record_agent_read`] の失敗は無視する）。
///
/// `sai-moon-paper content` と MCP の `paper` から呼ぶ。記録に失敗しても
/// 本文の出力は止めない（標準エラーに 1 行だけ出す）。
pub(crate) fn record_agent_read(
    via: &str,
    target: &str,
    paper_id: Option<i64>,
    path: Option<&Path>,
) {
    if let Err(error) = record_agent_read_inner(via, target, paper_id, path) {
        eprintln!("activity agent_read: {error}");
    }
}

fn record_agent_read_inner(
    via: &str,
    target: &str,
    paper_id: Option<i64>,
    path: Option<&Path>,
) -> Result<()> {
    let roots = paper_roots().context("プロジェクトルートが見つかりません")?;
    let stored = path.map(|path| activity::to_activity_path(&roots.root, &roots.data_dir, path));
    let paper_id = paper_id.or_else(|| {
        stored
            .as_deref()
            .and_then(|stored| activity::paper_id_for_path(Path::new(stored)))
    });
    let db = open_database()?;
    activity::record_agent_read(&db, target, via, paper_id, stored.as_deref())?;
    Ok(())
}
