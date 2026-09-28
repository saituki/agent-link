//! `sai-moon-paper set` — 検索セット管理サブコマンド。
//!
//! リモート検索結果の保存・注釈付けを行う決定論的 API。

use anyhow::{Context, Result, bail};
use clap::Subcommand;
use sai_paper_core::models::RemotePaper;
use sai_paper_core::search_sets;
use sai_paper_core::{Database, Paths};
use std::io::Read;

/// 検索セット管理サブコマンド。
#[derive(Subcommand, Debug)]
pub enum SearchSetCmd {
    /// セット一覧を表示
    List,
    /// セットの詳細と候補を表示
    Show {
        /// セット名または ID
        name_or_id: String,
        /// decision でフィルタ（pending/keep/drop/downloaded）
        #[arg(long)]
        decision: Option<String>,
    },
    /// 新しいセットを作成
    Create {
        /// セット名
        name: String,
        /// 検索クエリ（`--file` がない場合は必須）
        #[arg(long)]
        query: Option<String>,
        /// 検索に使った DB（カンマ区切り、既定: "manual"）
        #[arg(long, default_value = "manual")]
        source_dbs: String,
        /// 生のクエリ（DB 固有構文）
        #[arg(long)]
        raw_query: Option<String>,
        /// JSON ファイル（`-` は標準入力）
        #[arg(long)]
        file: Option<String>,
        /// 作成者（既定: "user"）
        #[arg(long, default_value = "user")]
        created_by: String,
        /// 何のための検索か（節点・問い・目的・抜き出す項目）
        #[arg(long)]
        node: Option<String>,
        /// 検索の問い（`Q7`）
        #[arg(long)]
        question: Option<String>,
        /// 何のための検索か
        #[arg(long)]
        purpose: Option<String>,
        /// 読み、そこから抜き出す項目
        #[arg(long)]
        extract: Option<String>,
    },
    /// 候補の decision を更新
    Mark {
        /// セット名または ID
        name_or_id: String,
        /// 位置（複数指定可）
        #[arg(long, value_delimiter = ',')]
        position: Vec<i64>,
        /// 判定（keep/drop）
        #[arg(long)]
        decision: String,
        /// 理由
        #[arg(long)]
        reason: Option<String>,
    },
    /// 複数のセットをマージして新しいセットを作成
    Merge {
        /// マージ対象のセット名（複数指定可）
        names: Vec<String>,
        /// 新しいセット名
        #[arg(long)]
        as_name: String,
    },
    /// セットを削除
    Delete {
        /// セット名または ID
        name_or_id: String,
    },
    /// 保存した検索をもう一度実行し、新着だけをセットに足す（S03）
    Rerun {
        /// セット名または ID
        name_or_id: String,
        /// JSON 出力
        #[arg(long)]
        json: bool,
    },
}

pub async fn handle_search_set(cmd: SearchSetCmd) -> Result<()> {
    match cmd {
        SearchSetCmd::List => handle_list(),
        SearchSetCmd::Show {
            name_or_id,
            decision,
        } => handle_show(&name_or_id, decision.as_deref()),
        SearchSetCmd::Create {
            name,
            query,
            source_dbs,
            raw_query,
            file,
            created_by,
            node,
            question,
            purpose,
            extract,
        } => {
            handle_create(CreateArgs {
                name: &name,
                query: query.as_deref(),
                source_dbs: &source_dbs,
                raw_query: raw_query.as_deref(),
                file: file.as_deref(),
                created_by: &created_by,
                node: node.as_deref(),
                question: question.as_deref(),
                purpose: purpose.as_deref(),
                extract: extract.as_deref(),
            })
            .await
        }
        SearchSetCmd::Mark {
            name_or_id,
            position,
            decision,
            reason,
        } => handle_mark(&name_or_id, position, &decision, reason.as_deref()),
        SearchSetCmd::Merge { names, as_name } => handle_merge(names, &as_name),
        SearchSetCmd::Delete { name_or_id } => handle_delete(&name_or_id),
        SearchSetCmd::Rerun { name_or_id, json } => handle_rerun(&name_or_id, json).await,
    }
}

fn open_db() -> Result<Database> {
    let paths = Paths::discover().context("failed to resolve paths")?;
    Database::open(&paths.database_file).context("failed to open papr.db")
}

fn print_json(obj: &serde_json::Value) {
    println!("{}", serde_json::to_string_pretty(obj).unwrap_or_default());
}

fn resolve_set_id(conn: &rusqlite::Connection, name_or_id: &str) -> Result<i64> {
    // ID として解析を試みる
    if let Ok(id) = name_or_id.parse::<i64>() {
        if search_sets::get(conn, id).is_ok() {
            return Ok(id);
        }
    }

    // 名前で検索
    search_sets::get_by_name(conn, name_or_id)
        .map(|s| s.id)
        .context("セットが見つかりません")
}

fn handle_list() -> Result<()> {
    let db = open_db()?;
    let conn = db.connection();

    let sets = search_sets::list(conn)?;

    let mut rows = Vec::new();
    for set in sets {
        let items = search_sets::items(conn, set.id)?;
        let undecided = items.iter().filter(|i| i.decision == "pending").count();

        rows.push(serde_json::json!({
            "id": set.id,
            "name": set.name,
            "items": items.len(),
            "undecided": undecided,
            "created_by": set.created_by,
            "created_at": set.created_at,
        }));
    }

    print_json(&serde_json::json!({ "sets": rows }));
    Ok(())
}

fn handle_show(name_or_id: &str, decision_filter: Option<&str>) -> Result<()> {
    let db = open_db()?;
    let conn = db.connection();

    let set_id = resolve_set_id(conn, name_or_id)?;
    let set = search_sets::get(conn, set_id)?;
    let items = search_sets::items(conn, set_id)?;

    let filtered_items: Vec<_> = items
        .iter()
        .filter(|item| decision_filter.is_none() || decision_filter == Some(item.decision.as_str()))
        .map(|item| {
            serde_json::json!({
                "position": item.position,
                "title": item.remote.title,
                "authors": item.remote.authors,
                "doi": item.remote.doi,
                "decision": item.decision,
                "reason": item.reason,
            })
        })
        .collect();

    print_json(&serde_json::json!({
        "set": {
            "id": set.id,
            "name": set.name,
            "query": set.query,
            "source_dbs": set.source_dbs,
            "created_by": set.created_by,
            "created_at": set.created_at,
        },
        "items": filtered_items,
    }));
    Ok(())
}

/// `set create` の引数（[`SearchSetCmd::Create`] の 1 対 1）。
struct CreateArgs<'a> {
    name: &'a str,
    query: Option<&'a str>,
    source_dbs: &'a str,
    raw_query: Option<&'a str>,
    file: Option<&'a str>,
    created_by: &'a str,
    node: Option<&'a str>,
    question: Option<&'a str>,
    purpose: Option<&'a str>,
    extract: Option<&'a str>,
}

async fn handle_create(args: CreateArgs<'_>) -> Result<()> {
    let CreateArgs {
        name,
        query,
        source_dbs,
        raw_query,
        file,
        created_by,
        node,
        question,
        purpose,
        extract,
    } = args;
    // query または file は必須
    if query.is_none() && file.is_none() {
        bail!("--query または --file が必要です");
    }

    let db = open_db()?;
    let conn = db.connection();

    let query_text = query.unwrap_or("").to_string();

    let set_id = search_sets::create(
        conn,
        search_sets::CreateSearchSetParams {
            name: name.to_string(),
            source_dbs: source_dbs.to_string(),
            query: query_text,
            raw_query: raw_query.map(|s| s.to_string()),
            created_by: created_by.to_string(),
            note: None,
        },
    )?;

    // file から RemotePaper を読み込む
    if let Some(file_path) = file {
        let json_str = if file_path == "-" {
            let mut buf = String::new();
            std::io::stdin().read_to_string(&mut buf)?;
            buf
        } else {
            std::fs::read_to_string(file_path)?
        };

        let papers: Vec<RemotePaper> = serde_json::from_str(&json_str)?;
        search_sets::add_items(conn, set_id, papers)?;
    }

    // 何のための検索かをセットに書く（R-R6）。1 つも無ければ何もしない。
    let refs = crate::sci::parse_search_refs(conn, node, question)?;
    if let Some(fields) = crate::sci::set_fields(&refs, purpose, extract) {
        search_sets::set_sci_fields(conn, set_id, &fields)?;
    }

    print_json(&serde_json::json!({ "id": set_id, "name": name }));
    Ok(())
}

fn handle_mark(
    name_or_id: &str,
    positions: Vec<i64>,
    decision: &str,
    reason: Option<&str>,
) -> Result<()> {
    let db = open_db()?;
    let conn = db.connection();

    let set_id = resolve_set_id(conn, name_or_id)?;

    search_sets::mark_many(conn, set_id, positions, decision, reason)?;

    print_json(&serde_json::json!({ "marked": true }));
    Ok(())
}

fn handle_merge(names: Vec<String>, as_name: &str) -> Result<()> {
    let db = open_db()?;
    let conn = db.connection();

    let mut set_ids = Vec::new();
    for name in names {
        set_ids.push(resolve_set_id(conn, &name)?);
    }

    let merged_id = search_sets::merge(conn, set_ids, as_name.to_string(), "user")?;

    print_json(&serde_json::json!({ "id": merged_id, "name": as_name }));
    Ok(())
}

/// 保存した検索をもう一度実行し、新着だけをセットに足す（S03）。
async fn handle_rerun(name_or_id: &str, json: bool) -> Result<()> {
    let paths = Paths::discover().context("failed to resolve paths")?;
    let cfg = sai_paper_core::Config::load_or_create(&paths)?;
    let db = open_db()?;

    let result = search_sets::rerun_set(&db, &cfg, name_or_id).await?;

    if json {
        print_json(&serde_json::to_value(&result)?);
    } else {
        let failed = if result.failed_dbs.is_empty() {
            String::new()
        } else {
            format!("、失敗した DB: {}", result.failed_dbs.join(", "))
        };
        println!(
            "新着 {} 本（検索 {} 件{}）",
            result.new_items, result.total_hits, failed
        );
    }
    Ok(())
}

fn handle_delete(name_or_id: &str) -> Result<()> {
    let db = open_db()?;
    let conn = db.connection();

    let set_id = resolve_set_id(conn, name_or_id)?;
    search_sets::delete(conn, set_id)?;

    print_json(&serde_json::json!({ "deleted": true }));
    Ok(())
}
