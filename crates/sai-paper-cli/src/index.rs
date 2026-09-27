//! `sai-paper index` — チャンク化と RAG インデックス管理。
//!
//! 索引（分け方・大きさ・埋め込みモデルの組）は `settings.json` の `rag.indexes` に名前付きで並べる。
//! `--index` を省くと `rag.default_index` を使う。索引づくり・埋め込みはこのコマンドを
//! 手で実行したときだけ走る（DL や保存を契機に自動では走らない）。

use anyhow::{Context, Result};
use clap::Subcommand;
use sai_paper_core::chunking::{Scope, build, rebuild, status};
use sai_paper_core::config::RagIndex;
use sai_paper_core::{Database, Paths};

/// インデックス管理サブコマンド。
#[derive(Subcommand, Debug)]
pub enum IndexCmd {
    /// チャンク化を実行（インクリメンタル）
    Build {
        /// 索引の名前（既定: rag.default_index）
        #[arg(long)]
        index: Option<String>,
        /// インデックス対象（papers/notes/all、既定: papers）
        #[arg(long, default_value = "papers")]
        scope: String,
    },
    /// その索引のチャンクだけを削除して再実行（ほかの索引は残す）
    Rebuild {
        /// 索引の名前（既定: rag.default_index）
        #[arg(long)]
        index: Option<String>,
    },
    /// 索引ごとの件数を表示
    Status,
    /// 埋め込みベクトルを生成・保存（その索引の埋め込みモデルで）
    Embed {
        /// 索引の名前（既定: rag.default_index）
        #[arg(long)]
        index: Option<String>,
        /// バッチサイズ（既定: 32）
        #[arg(long, default_value_t = 32)]
        batch: usize,
    },
}

/// DB・設定・選んだ索引を開く。
fn open(index: Option<&str>) -> Result<(Database, sai_paper_core::Config, Paths, RagIndex)> {
    let paths = Paths::discover().context("failed to resolve paths")?;
    let db = Database::open(&paths.database_file).context("failed to open papr.db")?;
    let config = sai_paper_core::Config::load_or_create(&paths)
        .context("failed to load or create configuration")?;
    let index = config.rag.index(index).map_err(anyhow::Error::msg)?;
    Ok((db, config, paths, index))
}

fn handle_build(index: Option<String>, scope_str: &str) -> Result<()> {
    let (db, _, paths, index) = open(index.as_deref())?;
    let scope = Scope::from_str(scope_str).context(format!("invalid scope: {scope_str}"))?;
    let report = build(db.connection(), scope, &index, &paths.data_dir())?;
    println!(
        "{{\"index\": {:?}, \"chunks_created\": {}, \"sources_skipped\": {}}}",
        index.name, report.chunks_created, report.sources_skipped
    );
    Ok(())
}

fn handle_rebuild(index: Option<String>) -> Result<()> {
    let (db, _, paths, index) = open(index.as_deref())?;
    let report = rebuild(db.connection(), &index, &paths.data_dir())?;
    println!(
        "{{\"index\": {:?}, \"chunks_created\": {}, \"sources_skipped\": {}}}",
        index.name, report.chunks_created, report.sources_skipped
    );
    Ok(())
}

/// `index status` の JSON（MCP の `index_status` も同じ形）。
pub fn status_json(db: &Database, config: &sai_paper_core::Config) -> Result<serde_json::Value> {
    let conn = db.connection();
    let status_info = status(conn)?;

    // 索引ごと: 断片の数と、その索引の埋め込みモデルで埋め込んだ数
    let mut indexes = serde_json::Map::new();
    for name in config.rag.index_names() {
        let index = config.rag.index(Some(&name)).map_err(anyhow::Error::msg)?;
        let chunks: i64 =
            conn.query_row("SELECT COUNT(*) FROM chunks WHERE index_name = ?1", [&name], |r| {
                r.get(0)
            })?;
        let embedded: i64 = conn.query_row(
            "SELECT COUNT(*) FROM chunks c
               JOIN chunk_embeddings ce ON ce.chunk_id = c.id
               JOIN embedding_models em ON em.id = ce.model_id
              WHERE c.index_name = ?1 AND em.name = ?2",
            [&name, &index.embed_model],
            |r| r.get(0),
        )?;
        indexes.insert(
            name.clone(),
            serde_json::json!({
                "chunk_strategy": index.chunk_strategy,
                "embed_model": index.embed_model,
                "chunks": chunks,
                "chunks_with_embeddings": embedded,
                "default": name == config.rag.default_index,
            }),
        );
    }

    Ok(serde_json::json!({
        "indexes": indexes,
        "by_source_kind": status_info.by_source_kind,
        "by_strategy": status_info.by_strategy,
        "chunks_with_embeddings": status_info.chunks_with_embeddings,
    }))
}

fn handle_status() -> Result<()> {
    let (db, config, _, _) = open(None)?;
    println!("{}", serde_json::to_string_pretty(&status_json(&db, &config)?)?);
    Ok(())
}

async fn handle_embed(index: Option<String>, batch_size: usize) -> Result<()> {
    let (db, _, _, index) = open(index.as_deref())?;
    let client = sai_paper_core::EmbedClient::for_index(&index);
    let embedded_count =
        sai_paper_core::embeddings::embed_pending(db.connection(), &client, &index.name, batch_size)
            .await
            .context("failed to embed chunks")?;
    println!(
        "{{\"index\": {:?}, \"embedded_count\": {}}}",
        index.name, embedded_count
    );
    Ok(())
}

pub async fn handle_index(cmd: IndexCmd) -> Result<()> {
    match cmd {
        IndexCmd::Build { index, scope } => handle_build(index, &scope),
        IndexCmd::Rebuild { index } => handle_rebuild(index),
        IndexCmd::Status => handle_status(),
        IndexCmd::Embed { index, batch } => handle_embed(index, batch).await,
    }
}
