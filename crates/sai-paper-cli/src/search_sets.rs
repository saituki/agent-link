//! `sai-moon-paper set` — 検索セット管理サブコマンド。
//!
//! リモート検索結果の保存・注釈付けを行う決定論的 API。

use anyhow::{Context, Result};
use clap::Subcommand;
use sai_paper_core::models::RemotePaper;
use sai_paper_core::search_sets;
use sai_paper_core::{Database, Paths};
use std::io::Read;

/// 検索セット管理サブコマンド。
#[derive(Subcommand, Debug)]
pub enum SearchSetCmd {
    /// セット一覧を表示
    List {
        /// JSON 出力（いつも JSON。付けても同じ）
        #[arg(long)]
        json: bool,
    },
    /// セットの詳細と候補を表示
    Show {
        /// セット名または ID
        name_or_id: String,
        /// decision でフィルタ（pending/keep/drop/downloaded）
        #[arg(long)]
        decision: Option<String>,
        /// JSON 出力（いつも JSON。付けても同じ）
        #[arg(long)]
        json: bool,
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
        /// 結ぶ研究方針の Node の ID（`Q12`・`O5` など）
        #[arg(long)]
        node: Option<String>,
        /// 何のための検索か
        #[arg(long)]
        purpose: Option<String>,
        /// 読み、そこから抜き出す項目
        #[arg(long)]
        extract: Option<String>,
        /// JSON 出力（いつも JSON。付けても同じ）
        #[arg(long)]
        json: bool,
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
        /// JSON 出力（いつも JSON。付けても同じ）
        #[arg(long)]
        json: bool,
    },
    /// `pending` の候補をすべて drop にする
    #[command(name = "drop-pending")]
    DropPending {
        /// セット名または ID
        name_or_id: String,
        /// 理由（既定は「まとめて drop」）
        #[arg(long)]
        reason: Option<String>,
        /// JSON 出力（いつも JSON。付けても同じ）
        #[arg(long)]
        json: bool,
    },
    /// 複数のセットをマージして新しいセットを作成
    Merge {
        /// マージ対象のセット名（複数指定可）
        names: Vec<String>,
        /// 新しいセット名
        #[arg(long)]
        as_name: String,
        /// JSON 出力（いつも JSON。付けても同じ）
        #[arg(long)]
        json: bool,
    },
    /// セットを削除
    Delete {
        /// セット名または ID
        name_or_id: String,
        /// JSON 出力（いつも JSON。付けても同じ）
        #[arg(long)]
        json: bool,
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
        SearchSetCmd::List { json: _ } => handle_list(),
        SearchSetCmd::Show {
            name_or_id,
            decision,
            json: _,
        } => handle_show(&name_or_id, decision.as_deref()),
        SearchSetCmd::Create {
            name,
            query,
            source_dbs,
            raw_query,
            file,
            created_by,
            node,
            purpose,
            extract,
            json: _,
        } => {
            handle_create(CreateArgs {
                name: &name,
                query: query.as_deref(),
                source_dbs: &source_dbs,
                raw_query: raw_query.as_deref(),
                file: file.as_deref(),
                created_by: &created_by,
                node: node.as_deref(),
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
            json: _,
        } => handle_mark(&name_or_id, position, &decision, reason.as_deref()),
        SearchSetCmd::DropPending {
            name_or_id,
            reason,
            json: _,
        } => handle_drop_pending(&name_or_id, reason.as_deref()),
        SearchSetCmd::Merge {
            names,
            as_name,
            json: _,
        } => handle_merge(names, &as_name),
        SearchSetCmd::Delete {
            name_or_id,
            json: _,
        } => handle_delete(&name_or_id),
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

pub(crate) fn resolve_set_id(conn: &rusqlite::Connection, name_or_id: &str) -> Result<i64> {
    // ID として解析を試みる
    if let Ok(id) = name_or_id.parse::<i64>() {
        if search_sets::get(conn, id).is_ok() {
            return Ok(id);
        }
    }

    // 名前で検索
    search_sets::get_by_name(conn, name_or_id)
        .map(|s| s.id)
        .map_err(|_| crate::cli_error::CliError::not_found("セットが見つかりません").into())
}

fn handle_list() -> Result<()> {
    let db = open_db()?;
    let conn = db.connection();

    // 一覧は `phone set list` と同じ形（`{"sets":[…]}`）で出す。
    let value = crate::phone::set_list(conn)?;
    print_json(&value);
    Ok(())
}

fn handle_show(name_or_id: &str, decision_filter: Option<&str>) -> Result<()> {
    let db = open_db()?;
    let conn = db.connection();

    let set_id = resolve_set_id(conn, name_or_id)?;
    let mut value = crate::phone::set_show(conn, set_id)?;
    // `--decision` があれば、候補をその判定の物だけに絞る。
    if let Some(decision) = decision_filter
        && let Some(items) = value.get_mut("items").and_then(|v| v.as_array_mut())
    {
        items.retain(|item| item.get("decision").and_then(|d| d.as_str()) == Some(decision));
    }
    print_json(&value);
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
        purpose,
        extract,
    } = args;
    // query または file は必須
    if query.is_none() && file.is_none() {
        return Err(crate::cli_error::CliError::input("--query または --file が必要です").into());
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
    )
    .map_err(|error| {
        // 同じ名前があるときは状態の衝突（終了コード 4）。
        if matches!(&error, search_sets::SearchSetError::Invalid(message) if message.contains("既に存在"))
        {
            anyhow::Error::from(crate::cli_error::CliError::conflict(format!("{error}")))
        } else {
            anyhow::Error::from(error)
        }
    })?;

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

    // 何のための検索かをセットに書く。1 つも無ければ何もしない。
    let refs = crate::sci::parse_search_refs(conn, node)?;
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

    // `phone set mark` と同じ形（`{"set_id","changed"}`）で出す。
    let value = crate::phone::mark_positions(conn, set_id, &positions, decision, reason)?;
    print_json(&value);
    Ok(())
}

/// `set drop-pending` — `pending` の候補をすべて drop にする（`phone` の形）。
fn handle_drop_pending(name_or_id: &str, reason: Option<&str>) -> Result<()> {
    let db = open_db()?;
    let conn = db.connection();

    let set_id = resolve_set_id(conn, name_or_id)?;
    let value = crate::phone::set_drop_pending(conn, set_id, reason)?;
    print_json(&value);
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
