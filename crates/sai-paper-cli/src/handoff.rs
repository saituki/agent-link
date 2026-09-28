//! `sai-moon-paper handoff` — task.md 発行コマンド。
//!
//! 今の `sai-moon-paper handoff --block …`（サブコマンドなし）と、
//! `sai-moon-paper handoff create …` / `sai-moon-paper handoff list` の両方を受け付ける。

use anyhow::{Context, Result, bail};
use clap::{Args, Subcommand};
use sai_paper_core::{Database, HandoffRequest, Paths, create_handoff_with, list_todo_cards};

/// カードを作る引数（サブコマンドなしでも `create` でも使う）。
#[derive(Args, Debug, Clone)]
pub struct HandoffCreateArgs {
    /// ブロック ID
    #[arg(long)]
    pub block: Option<String>,
    /// 論文 ID（繰り返し指定可）
    #[arg(long)]
    pub paper: Vec<i64>,
    /// 検索セット名（その中の downloaded 論文すべてを対象）
    #[arg(long)]
    pub set: Option<String>,
    /// 説明文（ファイルパスまたは stdin）
    #[arg(long)]
    pub text: Option<String>,
    /// 説明文（直接指定）
    #[arg(long)]
    pub description: Option<String>,
    /// タグ（任意）
    #[arg(long)]
    pub tag: Option<String>,
    /// 実行エージェント（pi または cc。既定: pi）
    #[arg(long, default_value = "pi")]
    pub runner: String,
    /// 推薦した実行の run_id（任意。カードの `origin_run_id` になる）
    #[arg(long)]
    pub origin_run: Option<i64>,
    /// 推薦の理由（任意。本文の先頭に書く）
    #[arg(long)]
    pub reason: Option<String>,
}

/// `handoff` のサブコマンド。
#[derive(Subcommand, Debug, Clone)]
pub enum HandoffCmd {
    /// カードを作る（サブコマンドなしと同じ）
    Create(HandoffCreateArgs),
    /// todo に残っているカードを一覧する
    List {
        /// タグで絞る（frontmatter の `tag` が一致するものだけ）
        #[arg(long)]
        tag: Option<String>,
        /// JSON 出力（`TodoCard` の配列）
        #[arg(long)]
        json: bool,
    },
}

/// `handoff` コマンドの入口。
pub async fn handle_handoff(cmd: Option<HandoffCmd>, args: HandoffCreateArgs) -> Result<()> {
    match cmd {
        Some(HandoffCmd::List { tag, json }) => handle_list(tag, json),
        Some(HandoffCmd::Create(create_args)) => handle_create(create_args).await,
        None => handle_create(args).await,
    }
}

/// カードを作る。
async fn handle_create(args: HandoffCreateArgs) -> Result<()> {
    let HandoffCreateArgs {
        block,
        paper,
        set,
        text,
        description,
        tag,
        runner,
        origin_run,
        reason,
    } = args;

    let block_id =
        block.context("ブロック ID が必要です。--block を指定してください")?;
    let paths = Paths::discover().context("経路の解決に失敗")?;
    let paper_roots = sai_paper_core::paper_roots().context("プロジェクトルートの解決に失敗")?;
    let db = Database::open(&paths.database_file).context("DB を開けませんでした")?;

    // 論文 ID を決定する
    let mut target_paper_ids = paper;

    // --set が指定されたら、そのセットの downloaded 論文をすべて追加
    if let Some(set) = set {
        let conn = db.connection();
        let mut stmt = conn
            .prepare(
                "SELECT ssi.paper_id FROM search_set_items ssi
                 JOIN search_sets ss ON ss.id = ssi.set_id
                 WHERE ss.name = ?1 AND ssi.decision = 'downloaded' AND ssi.paper_id IS NOT NULL
                 ORDER BY ssi.position",
            )
            .context("DB クエリ失敗")?;
        let ids: Vec<i64> = stmt
            .query_map([&set], |row| row.get(0))
            .context("set_items クエリ失敗")?
            .collect::<std::result::Result<Vec<_>, _>>()
            .context("set_items 取得失敗")?;
        target_paper_ids.extend(ids);
    }

    // 説明文を決定する（論文が無いときは説明文が必須）
    let desc_text = if let Some(d) = description {
        d
    } else if let Some(path) = text {
        if path == "-" {
            use std::io::Read;
            let mut buf = String::new();
            std::io::stdin().read_to_string(&mut buf)?;
            buf
        } else {
            std::fs::read_to_string(&path).context("説明文ファイルの読み込み失敗")?
        }
    } else {
        String::new()
    };

    if target_paper_ids.is_empty() && desc_text.trim().is_empty() {
        bail!("--description か --text で依頼の内容を書いてください");
    }

    // task.md を発行する
    let result = create_handoff_with(
        &db,
        &paper_roots,
        &HandoffRequest {
            block_id: &block_id,
            paper_ids: &target_paper_ids,
            description: &desc_text,
            tag: tag.as_deref(),
            runner: &runner,
            origin_run,
            reason: reason.as_deref(),
        },
    )?;

    // JSON で出力
    println!(
        "{}",
        serde_json::json!({
            "run_id": result.run_id,
            "path": result.path.to_string_lossy()
        })
    );

    Ok(())
}

/// todo のカードを一覧する。
fn handle_list(tag: Option<String>, json: bool) -> Result<()> {
    let paper_roots = sai_paper_core::paper_roots().context("プロジェクトルートの解決に失敗")?;
    let cards = list_todo_cards(&paper_roots, tag.as_deref())?;

    if json {
        println!("{}", serde_json::to_string_pretty(&cards)?);
    } else if cards.is_empty() {
        println!("todo にカードがありません");
    } else {
        for card in &cards {
            println!(
                "run_id={}\tblock={}\ttag={}\torigin_run_id={}\t{}",
                card.run_id,
                card.block,
                card.tag,
                card.origin_run_id
                    .map(|id| id.to_string())
                    .unwrap_or_else(|| "-".to_string()),
                card.description
            );
        }
    }

    Ok(())
}
