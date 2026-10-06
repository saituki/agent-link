//! `sai-sci find` / `sai-sci search` — 論文の検索を `sai-moon-paper` の CLI に任せ、
//! 研究方針の Node の範囲の決定と、研究の記録（`rt_searches`・検索セット）だけを行う。
//!
//! 論文の検索そのもの（複数の DB への問い合わせ・セットへの保存）は paper-cli にあり、
//! ライブラリには無い。そこで 2 つの検索の呼び方をそろえ、sai-sci は子として呼ぶ。

use anyhow::{Context, Result, anyhow};
use sai_paper_core::paths::project_root;
use sai_sci_core::bins;
use sai_sci_core::rt;
use serde_json::{Value, json};

use crate::open_database;
use crate::sci;

/// `sai-sci find` の引数（論文の `find` の引数 + `--node` など）。
#[derive(clap::Args, Debug)]
pub struct FindArgs {
    /// 検索クエリ（自然言語）
    pub query: String,
    /// 検索モード（word/vector/hybrid）
    #[arg(long)]
    pub mode: Option<String>,
    /// 上位 K 件
    #[arg(long)]
    pub top_k: Option<i64>,
    /// 1 本の論文から出す最大の件数（0 で制限なし）
    #[arg(long)]
    pub per_paper: Option<i64>,
    /// インデックス対象（papers/notes/figures/all）
    #[arg(long)]
    pub scope: Option<String>,
    /// 論文 ID（繰り返し指定可）
    #[arg(long)]
    pub paper: Vec<i64>,
    /// 論文の集合で絞る（カンマ区切り）
    #[arg(long, value_delimiter = ',')]
    pub papers: Vec<i64>,
    /// 索引の名前
    #[arg(long)]
    pub index: Option<String>,
    /// ハイブリッド検索の vector 側重み（0.0-1.0）
    #[arg(long)]
    pub alpha: Option<f64>,
    /// 融合方法（weighted/rrf）
    #[arg(long)]
    pub fusion: Option<String>,
    /// FTS5 式として渡す
    #[arg(long)]
    pub fts_expr: bool,
    /// 上位の結果に LLM の要約と関連度を付ける
    #[arg(long)]
    pub evidence: bool,
    /// 研究方針の Node の ID（`Q12`・`O5` など）。その Node の下の F の論文に絞る
    #[arg(long)]
    pub node: Option<String>,
    /// `--node` の子孫の Node も含める
    #[arg(long)]
    pub subtree: bool,
    /// 範囲の論文ごとに上位 m 件を返す（`--papers` か `--node` と一緒に使う）
    #[arg(long)]
    pub each: Option<i64>,
    /// 範囲を広げた理由
    #[arg(long)]
    pub widen: Option<String>,
    /// JSON 出力（`{"log","hits"}`）
    #[arg(long)]
    pub json: bool,
}

/// `sai-sci search` の引数（論文の `search` の引数 + `--node` など）。
#[derive(clap::Args, Debug)]
pub struct SearchArgs {
    /// 検索クエリ
    pub query: String,
    /// データベース（繰り返し可: s2, arxiv, pubmed, biorxiv, openalex）
    #[arg(long)]
    pub db: Vec<String>,
    /// 最大件数
    #[arg(long, default_value_t = 10)]
    pub limit: u16,
    /// 出版年の下限
    #[arg(long)]
    pub year_from: Option<u16>,
    /// 出版年の上限
    #[arg(long)]
    pub year_to: Option<u16>,
    /// DB ごとの生のクエリ
    #[arg(long)]
    pub raw_query: Option<String>,
    /// 結果を名前付きの検索セットに保存する
    #[arg(long)]
    pub save_as: Option<String>,
    /// 検索した研究方針の Node の ID（`Q12`・`O5` など）
    #[arg(long)]
    pub node: Option<String>,
    /// 何のための検索か
    #[arg(long)]
    pub purpose: Option<String>,
    /// 読み、そこから抜き出す項目
    #[arg(long)]
    pub extract: Option<String>,
    /// 範囲を広げた理由
    #[arg(long)]
    pub widen: Option<String>,
    /// JSON 出力（`{"log","items","failed"}`）
    #[arg(long)]
    pub json: bool,
}

/// `find` を実行する。
pub fn find(args: &FindArgs) -> Result<()> {
    let query = args.query.trim();
    if query.is_empty() {
        return Err(crate::cli_error::CliError::input("検索語が空です").into());
    }
    if let Some(mode) = &args.mode
        && !matches!(mode.as_str(), "word" | "vector" | "hybrid")
    {
        return Err(crate::cli_error::CliError::input(
            "検索モードが正しくありません（word / vector / hybrid）",
        )
        .into());
    }
    if let Some(scope) = &args.scope
        && !matches!(scope.as_str(), "papers" | "notes" | "figures" | "all")
    {
        return Err(crate::cli_error::CliError::input(
            "検索対象が正しくありません（papers / notes / figures / all）",
        )
        .into());
    }
    let mut all_papers = args.paper.clone();
    all_papers.extend(args.papers.clone());
    sci::check_find_scope(args.subtree, args.node.is_some(), args.each, all_papers.is_empty())?;

    let db = open_database()?;
    let conn = db.connection();
    let refs = sci::parse_search_refs(conn, args.node.as_deref())?;
    let (paper_ids, scope_note) =
        rt::scope::resolve_scope(conn, None, &all_papers, refs.node, args.subtree)
            .map_err(|err| anyhow!("{err}"))?;
    // 人向けの出力のときだけ、記録を残す前に前回の行を作る。
    let previous = if args.json {
        None
    } else {
        sci::previous_search_line(conn, &refs, &scope_note)
    };
    let node_name = args.node.as_deref().map(|text| text.trim().to_string());
    let ids = paper_ids.unwrap_or_default();

    // Node の下に論文が無いときは、論文の CLI を呼ばずに 0 件で記録する。
    if ids.is_empty() {
        sci::record_search(
            conn,
            &sci::SearchLog {
                refs: &refs,
                tool: "find",
                query,
                scope: &scope_note,
                widen_reason: args.widen.as_deref().unwrap_or(""),
                hits: 0,
                hit_papers: &[],
                set_id: None,
                log_id: None,
            },
        )?;
        if args.json {
            println!("{}", json!({ "log": null, "hits": [] }));
        } else {
            eprintln!(
                "{} の下に論文がありません",
                node_name.as_deref().unwrap_or(&scope_note)
            );
        }
        return Ok(());
    }

    let root = project_root().context("プロジェクトルートが見つかりません")?;
    let mut child_args = vec!["find".to_string(), query.to_string()];
    child_args.push("--papers".to_string());
    child_args.push(join_ids(&ids));
    if let Some(mode) = &args.mode {
        child_args.push("--mode".to_string());
        child_args.push(mode.clone());
    }
    push_i64(&mut child_args, "--top-k", args.top_k);
    push_i64(&mut child_args, "--per-paper", args.per_paper);
    push_str(&mut child_args, "--scope", args.scope.as_deref());
    push_str(&mut child_args, "--index", args.index.as_deref());
    push_f64(&mut child_args, "--alpha", args.alpha);
    push_str(&mut child_args, "--fusion", args.fusion.as_deref());
    if args.fts_expr {
        child_args.push("--fts-expr".to_string());
    }
    if args.evidence {
        child_args.push("--evidence".to_string());
    }
    push_i64(&mut child_args, "--each", args.each);
    push_str(&mut child_args, "--widen", args.widen.as_deref());
    child_args.push("--json".to_string());

    let out = crate::run_child(
        &bins::paper_bin(),
        bins::PAPER_BIN_NAME,
        "find",
        &child_args,
        &root,
    )?;
    let mut value: Value =
        serde_json::from_str(&out).map_err(|err| anyhow!("find の JSON を読めません: {err}"))?;
    let log_id = value
        .get("log")
        .and_then(|log| log.get("id"))
        .and_then(Value::as_i64)
        .context("find の JSON に log.id がありません")?;
    let hits = value
        .get("hits")
        .and_then(Value::as_array)
        .context("find の JSON に hits がありません")?
        .clone();
    let hit_papers = hit_paper_ids(&hits);

    sci::record_search(
        conn,
        &sci::SearchLog {
            refs: &refs,
            tool: "find",
            query,
            scope: &scope_note,
            widen_reason: args.widen.as_deref().unwrap_or(""),
            hits: hits.len() as i64,
            hit_papers: &hit_papers,
            set_id: None,
            log_id: Some(log_id),
        },
    )?;

    if let Some(previous) = previous {
        println!("{previous}");
    }
    if args.json {
        if let Some(name) = &node_name
            && let Some(log) = value.get_mut("log").and_then(Value::as_object_mut)
        {
            log.insert("node".to_string(), json!({ "id": name }));
        }
        println!("{}", serde_json::to_string(&value)?);
    } else {
        for hit in &hits {
            println!("{}", hit_line(hit));
        }
    }
    Ok(())
}

/// `search` を実行する。
pub fn search(args: &SearchArgs) -> Result<()> {
    let query = args.query.trim();
    if query.is_empty() {
        return Err(crate::cli_error::CliError::input("検索語が空です").into());
    }
    let db = open_database()?;
    let conn = db.connection();
    let refs = sci::parse_search_refs(conn, args.node.as_deref())?;
    let node_name = args.node.as_deref().map(|text| text.trim().to_string());

    let root = project_root().context("プロジェクトルートが見つかりません")?;
    let mut child_args = vec!["search".to_string(), query.to_string()];
    for database in &args.db {
        child_args.push("--db".to_string());
        child_args.push(database.clone());
    }
    child_args.push("--limit".to_string());
    child_args.push(args.limit.to_string());
    push_u16(&mut child_args, "--year-from", args.year_from);
    push_u16(&mut child_args, "--year-to", args.year_to);
    push_str(&mut child_args, "--raw-query", args.raw_query.as_deref());
    push_str(&mut child_args, "--save-as", args.save_as.as_deref());
    child_args.push("--json".to_string());

    let out = crate::run_child(
        &bins::paper_bin(),
        bins::PAPER_BIN_NAME,
        "search",
        &child_args,
        &root,
    )?;
    let mut value: Value =
        serde_json::from_str(&out).map_err(|err| anyhow!("search の JSON を読めません: {err}"))?;
    let log_id = value
        .get("log")
        .and_then(|log| log.get("id"))
        .and_then(Value::as_i64)
        .context("search の JSON に log.id がありません")?;
    let source_dbs = value
        .get("log")
        .and_then(|log| log.get("source_dbs"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let items = value
        .get("items")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    // 何のための検索かを、履歴の行と、保存したセットの両方に書く。
    if let Some(fields) = sci::set_fields(&refs, args.purpose.as_deref(), args.extract.as_deref()) {
        sai_paper_core::search_sets::set_sci_fields(conn, log_id, &fields)
            .context("検索に目的を書けませんでした")?;
        if let Some(name) = &args.save_as
            && let Ok(set) = sai_paper_core::search_sets::get_by_name(conn, name)
        {
            sai_paper_core::search_sets::set_sci_fields(conn, set.id, &fields)
                .context("検索セットに目的を書けませんでした")?;
        }
    }
    let set_id = args
        .save_as
        .as_deref()
        .and_then(|name| sai_paper_core::search_sets::get_by_name(conn, name).ok())
        .map(|set| set.id);

    if refs.node.is_some() {
        let scope = format!("external:{source_dbs}");
        sci::record_search(
            conn,
            &sci::SearchLog {
                refs: &refs,
                tool: "search",
                query,
                scope: &scope,
                widen_reason: args.widen.as_deref().unwrap_or(""),
                hits: items.len() as i64,
                hit_papers: &[],
                set_id,
                log_id: Some(log_id),
            },
        )?;
    }

    if args.json {
        if let Some(name) = &node_name
            && let Some(log) = value.get_mut("log").and_then(Value::as_object_mut)
        {
            log.insert("node".to_string(), json!({ "id": name }));
        }
        println!("{}", serde_json::to_string(&value)?);
    } else {
        for item in &items {
            let title = item.get("title").and_then(Value::as_str).unwrap_or("");
            match item.get("year").and_then(Value::as_str) {
                Some(year) if !year.is_empty() => println!("- {title}（{year}）"),
                _ => println!("- {title}"),
            }
        }
    }
    Ok(())
}

/// 検索結果 1 件を人向けの 1 行にする（無い項目は飛ばす）。
fn hit_line(hit: &Value) -> String {
    let mut line = String::from("- ");
    if let Some(cite) = hit.get("cite").and_then(Value::as_str) {
        line.push_str(cite);
        line.push(' ');
    }
    if let Some(title) = hit.get("title").and_then(Value::as_str) {
        line.push_str(title);
    }
    let text = hit.get("text").and_then(Value::as_str).unwrap_or("");
    if !text.is_empty() {
        let head: String = text.chars().take(120).collect();
        line.push_str(": ");
        line.push_str(&head);
    }
    line
}

/// 論文の番号の集まりをカンマでつなぐ（`--papers` の引数）。
fn join_ids(ids: &[i64]) -> String {
    ids.iter().map(i64::to_string).collect::<Vec<_>>().join(",")
}

/// 検索結果から論文の番号を取り出す（重複なし・昇順）。
fn hit_paper_ids(hits: &[Value]) -> Vec<i64> {
    let mut ids: Vec<i64> = hits
        .iter()
        .filter_map(|hit| hit.get("paper_id").and_then(Value::as_i64))
        .collect();
    ids.sort_unstable();
    ids.dedup();
    ids
}

/// 任意の文字列の引数を足す。
fn push_str(args: &mut Vec<String>, flag: &str, value: Option<&str>) {
    if let Some(value) = value {
        args.push(flag.to_string());
        args.push(value.to_string());
    }
}

/// 任意の整数の引数を足す。
fn push_i64(args: &mut Vec<String>, flag: &str, value: Option<i64>) {
    if let Some(value) = value {
        args.push(flag.to_string());
        args.push(value.to_string());
    }
}

/// 任意の小数の引数を足す。
fn push_f64(args: &mut Vec<String>, flag: &str, value: Option<f64>) {
    if let Some(value) = value {
        args.push(flag.to_string());
        args.push(value.to_string());
    }
}

/// 任意の年の引数を足す。
fn push_u16(args: &mut Vec<String>, flag: &str, value: Option<u16>) {
    if let Some(value) = value {
        args.push(flag.to_string());
        args.push(value.to_string());
    }
}
