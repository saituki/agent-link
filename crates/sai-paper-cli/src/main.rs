//! sai-paper CLI — Rust native (旧 Node core.mjs は廃止)
//!
//! search / get-pdf / extract / graph / graph-show / status の全コマンドが
//! sai-paper-core を直接呼び出し、Node サブプロセスなしで動作する。
//! DB は papr.db（引用テーブル 0016 含む）を共有する。

use anyhow::{Context, Result, anyhow, bail};
use chrono::{Duration, Local, NaiveDate};
use clap::{Parser, Subcommand};
use sai_paper_core::citation;
use sai_paper_core::{Config, Database, Paths, RemotePaper, S2Client};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

mod activity;
mod agent_assets;
mod attention;
mod canvas;
mod content;
mod dsr;
mod expand;
mod figures;
mod handoff;
mod index;
mod litbase;
mod mcp;
mod phone;
mod phone_canvas;
mod phone_cite;
mod phone_import;
mod phone_sci;
mod progress;
mod relayout;
mod report;
mod sci;
mod search_sets;
mod understanding;

#[derive(Parser, Debug)]
#[command(
    name = "sai-moon-paper",
    version,
    about = "sai-moon-paper CLI (Rust native, Node非依存)"
)]
struct Cli {
    #[command(subcommand)]
    cmd: Option<Cmd>,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// Multi-database paper search
    Search {
        /// Search query text.
        query: String,
        /// Database to search (repeatable: s2, arxiv, pubmed, biorxiv, openalex).
        #[arg(long)]
        db: Vec<String>,
        /// Maximum number of results.
        #[arg(long, default_value_t = 10)]
        limit: u16,
        /// Start publication year (inclusive).
        #[arg(long)]
        year_from: Option<u16>,
        /// End publication year (inclusive).
        #[arg(long)]
        year_to: Option<u16>,
        /// Database-specific raw query (PubMed Boolean, etc.).
        #[arg(long)]
        raw_query: Option<String>,
        /// Save results as a named search set.
        #[arg(long)]
        save_as: Option<String>,
        /// 検索した節点（`N12`）。記録と検索セットに残る
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
        /// 範囲を広げた理由
        #[arg(long)]
        widen: Option<String>,
        /// Output as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Ensure PDF downloaded (S2 open access / arXiv)
    #[command(name = "get-pdf")]
    GetPdf { ids: Vec<String> },
    /// Docling extract + ref parse + S2 match (saves to papr.db)
    Extract { paper_id: String },
    /// Build S2 citation graph cache (references / citations)
    Graph {
        paper_id: String,
        #[arg(long, default_value_t = 2)]
        depth: u8,
        /// references / citations / both
        #[arg(long, default_value = "both")]
        direction: String,
        #[arg(long)]
        out: Option<PathBuf>,
        /// Restrict traversal to papers in local database
        #[arg(long)]
        library_only: bool,
    },
    /// Show cached citation graph
    #[command(name = "graph-show")]
    GraphShow { paper_id: String },
    /// 論文の引用の前後を 1 段たどり、Library に無いものを検索セットに入れる（ネットを使う）
    Expand {
        /// 論文 ID（`papers.id`。複数指定可）
        papers: Vec<i64>,
        /// たどる向き（both / refs / cited）
        #[arg(long, default_value = "both")]
        direction: String,
        /// 向きごとに取る最大件数
        #[arg(long, default_value_t = 20)]
        limit: usize,
        /// 入れる検索セット名（無ければ作る、あれば足す）
        #[arg(long)]
        save_as: String,
        /// JSON 出力
        #[arg(long)]
        json: bool,
    },
    /// DB status
    Status,
    /// 研究プロジェクトを初期化する（`.sai/` と `.sai/paper/`）
    Init {
        /// プロジェクト名（sai-control 登録用）
        #[arg(long)]
        name: Option<String>,
        /// プロジェクトルート（既定: `SAI_PAPER_ROOT` または cwd）
        #[arg(long)]
        root: Option<PathBuf>,
        /// 既にある研究プロジェクトに、足りないブロック・テンプレート・設定だけを配る（DB と既存のファイルは触らない）
        #[arg(long)]
        assets_only: bool,
        /// DSR が使う分野の観点
        #[arg(long, default_value = "general", value_parser = ["medical", "signal-ml", "general"])]
        domain: String,
    },
    /// エージェントに渡すプロジェクトの状況のまとめを作る
    Brief {
        /// `.sai/template/project-brief.md` に書く（既定は標準出力）
        #[arg(long)]
        write: bool,
    },
    /// litbase 連携（pi-litbase 拡張が呼ぶ決定論的 API）
    Litbase {
        /// サブコマンド
        #[command(subcommand)]
        cmd: litbase::LitbaseCmd,
    },
    /// 検索セット管理
    Set {
        /// サブコマンド
        #[command(subcommand)]
        cmd: search_sets::SearchSetCmd,
    },
    /// 論文をエージェントに引き渡す (task.md 発行)
    #[command(args_conflicts_with_subcommands = true)]
    Handoff {
        /// サブコマンド（無ければ今までどおりカードを作る）
        #[command(subcommand)]
        cmd: Option<handoff::HandoffCmd>,
        /// カードを作る引数（サブコマンドなしで使う）
        #[command(flatten)]
        args: Box<handoff::HandoffCreateArgs>,
    },
    /// RAG インデックス管理（チャンク化・全文索引）
    Index {
        /// サブコマンド
        #[command(subcommand)]
        cmd: index::IndexCmd,
    },
    /// ベクトル/全文/ハイブリッド検索
    Find {
        /// 検索クエリ（自然言語）
        query: String,
        /// 検索モード（word/vector/hybrid）
        #[arg(long)]
        mode: Option<String>,
        /// 上位 K 件
        #[arg(long)]
        top_k: Option<i64>,
        /// 1 本の論文から出す最大の件数（0 で制限なし、既定は config の rag.per_paper）
        #[arg(long)]
        per_paper: Option<i64>,
        /// インデックス対象（papers/notes/all）
        #[arg(long)]
        scope: Option<String>,
        /// 論文 ID（指定時はその論文に絞り込む）
        #[arg(long)]
        paper: Option<i64>,
        /// 索引の名前（既定: rag.default_index。埋め込みモデルも索引で決まる）
        #[arg(long)]
        index: Option<String>,
        /// ハイブリッド検索の vector 側重み（0.0-1.0、既定は config）
        #[arg(long)]
        alpha: Option<f64>,
        /// 融合方法（weighted/rrf、既定は config）
        #[arg(long)]
        fusion: Option<String>,
        /// FTS5 式として渡す（既定: false、自然言語でクォート）
        #[arg(long)]
        fts_expr: bool,
        /// 上位の結果に LLM の要約と関連度を付ける（費用がかかる。必要なときだけ）
        #[arg(long)]
        evidence: bool,
        /// 論文の集合で絞る（カンマ区切り。`--paper` と同じ扱い）
        #[arg(long, value_delimiter = ',')]
        papers: Vec<i64>,
        /// 節点に結び付いた論文に絞る（`N12`）
        #[arg(long)]
        node: Option<String>,
        /// `--node` の子孫の節点も含める
        #[arg(long)]
        subtree: bool,
        /// 範囲の論文ごとに上位 m 件を返す（`--papers` か `--node` と一緒に使う）
        #[arg(long)]
        each: Option<i64>,
        /// 検索の問い（`Q7`）。記録に残り、前の検索を見られる
        #[arg(long)]
        question: Option<String>,
        /// 範囲を広げた理由
        #[arg(long)]
        widen: Option<String>,
        /// JSON 出力
        #[arg(long)]
        json: bool,
    },
    /// 論文の近傍チャンクを検索
    Neighbors {
        /// 論文 ID
        paper_id: i64,
        /// 上位 K 件
        #[arg(long)]
        top_k: Option<i64>,
        /// 索引の名前（既定: rag.default_index）
        #[arg(long)]
        index: Option<String>,
        /// JSON 出力
        #[arg(long)]
        json: bool,
    },
    /// 既存データを新しい置き場の規則へ移す（`papers/{id:04}_短いタイトル/` に集める）。
    /// 既定は dry-run（予定の表示だけ）。`--apply` の前に `papr.db` のバックアップを作る。
    #[command(name = "relayout")]
    Relayout {
        /// 実際に動かす（既定は予定の表示だけ）
        #[arg(long)]
        apply: bool,
        /// 機械可読出力（JSON）
        #[arg(long)]
        json: bool,
    },
    /// PDF を日本語に訳す（TUI の `T` キーと同じ。数分かかる）
    #[command(name = "translate-pdf")]
    TranslatePdf {
        /// 論文 ID（papers.id）
        paper_id: i64,
    },
    /// MCP サーバー（標準入出力）。cc / pi から find・neighbors・index_status・paper・search_sets を使う
    Mcp,
    /// 設定（`.sai/paper/settings.json`）を確かめる
    Config {
        /// サブコマンド
        #[command(subcommand)]
        cmd: ConfigCmd,
    },
    /// agent の report の登録・参照
    Report {
        /// サブコマンド
        #[command(subcommand)]
        cmd: report::ReportCmd,
    },
    /// Canvas を CLI から作る（エージェント用、V03）
    Canvas {
        /// サブコマンド
        #[command(subcommand)]
        cmd: canvas::CanvasCmd,
    },
    /// 論文のメタデータをまとめて確かめる（ネットを使う）
    Enrich {
        /// 撤回された論文を OpenAlex に問い合わせて印を付ける
        #[arg(long)]
        retraction: bool,
        /// 確かめ済みでももう一度問い合わせる
        #[arg(long)]
        force: bool,
    },
    /// 論文本文 / 引用箇所 / report の内容を出力する（sai-link の入力展開用）
    Content {
        /// サブコマンド
        #[command(subcommand)]
        cmd: content::ContentCmd,
    },
    /// DSR（研究用ブロック）の report を確かめる（LLM は使わない）
    Dsr {
        /// サブコマンド
        #[command(subcommand)]
        cmd: dsr::DsrCmd,
    },
    /// 「開いた」「エージェントが読んだ」の記録（N01）
    Activity {
        /// サブコマンド
        #[command(subcommand)]
        cmd: activity::ActivityCmd,
    },
    /// トピックごとの「理解の状態」の記録（N04）
    Understanding {
        /// サブコマンド
        #[command(subcommand)]
        cmd: understanding::UnderstandingCmd,
    },
    /// プロジェクトの進み具合と「読むべき論文」を出す（N05）
    Progress {
        /// JSON で出力する
        #[arg(long)]
        json: bool,
    },
    /// 日ごとの研究日誌を出す（S01。記録を並べるだけで LLM は使わない）
    Journal {
        /// 今日を含む日数（既定 7）
        #[arg(long, default_value_t = 7)]
        days: i64,
        /// この日だけを出す（YYYY-MM-DD）。指定すると `--days` は無視される
        #[arg(long)]
        date: Option<String>,
        /// JSON で出力する
        #[arg(long)]
        json: bool,
    },
    /// 要対応の一覧と「対応済み」の記録（S02。`attention resolve <key>`）
    Attention {
        /// サブコマンド（無ければ一覧を出す）
        #[command(subcommand)]
        cmd: Option<attention::AttentionCmd>,
        /// JSON で出力する
        #[arg(long)]
        json: bool,
    },
    /// 図表（Figure / Table）を抽出して figures/ と DB に保存する
    Figures {
        /// 対象論文 ID（複数指定可）
        paper_ids: Vec<i64>,
        /// ライブラリ内の全論文を対象にする
        #[arg(long)]
        all: bool,
        /// 既に抽出済みでも再実行する
        #[arg(long)]
        force: bool,
    },
    /// sai-phone（iPhone 窓口）向けの JSON 出力
    Phone {
        #[command(subcommand)]
        cmd: phone::PhoneCmd,
    },
    /// 研究方針の木（sai-sci）
    Sci {
        /// サブコマンド
        #[command(subcommand)]
        cmd: sci::SciCmd,
    },
    /// Library の検索（RAG）の精度を測る
    Eval {
        /// サブコマンド
        #[command(subcommand)]
        cmd: EvalCmd,
    },
}

/// `sai-paper eval` のサブコマンド。
#[derive(Subcommand, Debug)]
enum EvalCmd {
    /// 質問集の問いごとに検索し、正解が上位に入った割合と MRR を索引ごとに出す
    Rag {
        /// 索引の名前（複数指定すると索引ごとに 1 行。省略時は `rag.default_index`）
        #[arg(long = "index")]
        indexes: Vec<String>,
        /// 検索モード（word/vector/hybrid）
        #[arg(long, default_value = "hybrid")]
        mode: String,
        /// 上位 K 件
        #[arg(long, default_value_t = 10)]
        top_k: i64,
        /// 質問集のファイル（既定: `<root>/.sai/paper/eval/rag-questions.json`）
        #[arg(long)]
        file: Option<PathBuf>,
        /// JSON 出力
        #[arg(long)]
        json: bool,
        /// 問いごとの最初の正解の順位も出す（圏外は `-`）
        #[arg(long)]
        verbose: bool,
    },
}

/// `sai-paper config` のサブコマンド。
#[derive(Subcommand, Debug)]
enum ConfigCmd {
    /// 今の設定を JSON で表示する（足りない環境変数は警告する）
    Show,
    /// 設定ファイルの場所を表示する
    Path,
}

/// S2 API キーを解決する（設定の `discover.s2_api_key_env` の環境変数。dotenvy 済みの env も参照される）。
fn resolve_api_key() -> Option<String> {
    Paths::discover()
        .ok()
        .and_then(|paths| sai_paper_core::Config::load_or_create(&paths).ok())
        .unwrap_or_default()
        .s2_api_key()
}

pub(crate) fn open_database() -> Result<Database> {
    let paths = Paths::discover().context("failed to resolve paths")?;
    Database::open(&paths.database_file).context("failed to open papr.db")
}

/// `sai-paper journal` — 日ごとの研究日誌を出す（S01）。
///
/// `--date` があればその 1 日、無ければ今日を含む `--days` 日。`--json` なら
/// [`sai_paper_core::journal::JournalDay`] の配列、人向けは
/// [`sai_paper_core::journal::render_markdown`] の結果を出す。
fn handle_journal(days: i64, date: Option<&str>, json: bool) -> Result<()> {
    let db = open_database()?;
    let roots = sai_paper_core::paths::paper_roots()
        .context("プロジェクトルート（.sai/paper）が見つかりません")?;
    let today = Local::now().date_naive();
    let (from, to) = match date {
        Some(value) => {
            let day = NaiveDate::parse_from_str(value, "%Y-%m-%d")
                .with_context(|| format!("--date は YYYY-MM-DD の形で指定してください: {value}"))?;
            (day, day)
        }
        None => {
            let days = days.max(1);
            (today - Duration::days(days - 1), today)
        }
    };
    let journal = sai_paper_core::journal::collect(db.connection(), &roots, from, to)?;

    if json {
        println!("{}", serde_json::to_string_pretty(&journal)?);
    } else {
        print!("{}", sai_paper_core::journal::render_markdown(&journal));
    }
    Ok(())
}

/// 複数 DB で検索し、重複を除いた結果と、実際に使った DB 一覧を返す。
///
/// 検索そのものは [`sai_paper_core::api::run_search`]（core）にある。
/// DB ごとの検索エラーは標準エラーに出して続ける（1 つでも成功すればその結果を返す）。
/// `sai-paper search` と `sai-paper phone search` の共通処理。
pub(crate) async fn run_search(
    cfg: &Config,
    query: &str,
    db: &[String],
    limit: u16,
    year_from: Option<u16>,
    year_to: Option<u16>,
    raw_query: Option<&str>,
) -> Result<(Vec<String>, Vec<RemotePaper>)> {
    let (dbs_to_use, deduped, failed) =
        sai_paper_core::api::run_search(cfg, query, db, limit, year_from, year_to, raw_query).await;
    // 従来と同じ形（`<db>: <理由>`）で標準エラーに出す。
    for (db_id, reason) in &failed {
        eprintln!("{db_id}: {reason}");
    }
    Ok((dbs_to_use, deduped))
}

/// `sai-paper enrich --retraction` の本体。
///
/// DOI のある論文のうち、まだ撤回を確かめていないもの（`--force` なら全部）を
/// 1 秒に 1 件 OpenAlex に問い合わせ、`papers.is_retracted` と
/// `papers.retraction_checked_at` を更新する。ネットにつながらないときは
/// そこで止め、何本まで確かめたかを出す。
async fn enrich_retraction(force: bool) -> Result<()> {
    use sai_paper_core::api::openalex::OpenAlexClient;

    let db = open_database()?;
    let targets = db
        .papers_for_retraction_check(force)
        .context("撤回を確かめる論文の取得に失敗")?;
    if targets.is_empty() {
        println!("確かめる論文はありません（DOI のある論文はすべて確認済みです）");
        return Ok(());
    }
    println!(
        "DOI のある論文 {} 本を確かめます（約 {} 秒）",
        targets.len(),
        targets.len()
    );

    let client = OpenAlexClient::new();
    let mut checked = 0usize;
    let mut retracted: Vec<i64> = Vec::new();
    for (index, (paper_id, doi)) in targets.iter().enumerate() {
        match client.work_status_by_doi(doi).await {
            Ok(status) => {
                let is_retracted = status.as_ref().and_then(|status| status.is_retracted);
                if is_retracted == Some(true) {
                    retracted.push(*paper_id);
                }
                // 見つからない DOI（Ok(None)）も「確かめた」として記録する。
                db.set_retraction(*paper_id, is_retracted)?;
                checked += 1;
            }
            Err(error) => {
                eprintln!("{doi}: {error}");
                println!(
                    "ネットにつながらないか、応答を読めませんでした。{checked} 本まで確かめました"
                );
                break;
            }
        }
        // OpenAlex への配慮で 1 秒あける（最後の 1 本のあとは待たない）。
        if index + 1 < targets.len() {
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        }
    }

    sai_paper_core::reports::refresh_retracted_issues(db.connection())
        .context("撤回を引用した report の更新に失敗")?;
    if retracted.is_empty() {
        println!("撤回: 0 本");
    } else {
        let list = retracted
            .iter()
            .map(|id| format!("#{id}"))
            .collect::<Vec<_>>()
            .join(", ");
        println!("撤回: {} 本（{list}）", retracted.len());
    }
    Ok(())
}

/// エージェントに渡すプロジェクトの状況のまとめ（brief）を作る。
///
/// `--write` で `<root>/.sai/template/project-brief.md` に書き、書いたパスを標準エラーに出す。
/// `papr.db` が無い・開けないときは数字の行だけ「（papr.db を開けません）」にし、
/// 他は出す（brief の失敗でエージェントの起動を止めないため）。設定ファイルが
/// 無いときも既定値で続ける。
fn handle_brief(write: bool) -> Result<()> {
    let paths = Paths::discover()?;
    let roots =
        sai_paper_core::paths::paper_roots().context("研究プロジェクトのルートを解決できません")?;
    let config = Config::load(&paths.config_file).unwrap_or_default();
    let db = if paths.database_file.exists() {
        Database::open(&paths.database_file).ok()
    } else {
        None
    };
    // `--write` のときは、最初に成果物のフォルダを走査して report を登録する。
    // 失敗しても brief は作る（標準エラーに 1 行）。
    if write
        && let Some(db) = db.as_ref()
        && let Err(error) = sai_paper_core::reports::scan(db.connection(), &roots.root)
    {
        eprintln!("report scan: {error}");
    }
    let text = sai_paper_core::brief::build_brief(
        &roots,
        db.as_ref().map(Database::connection),
        &config.brief,
        &config.rag.default_index,
    );
    if write {
        let path = sai_paper_core::brief::write_brief(&roots, &text)?;
        eprintln!("{}", path.display());
    } else {
        print!("{text}");
    }
    Ok(())
}

/// `sai-paper eval rag` の本体。
///
/// 質問集（既定: `<root>/.sai/paper/eval/rag-questions.json`）の問いごとに検索し、
/// 索引ごとの成績（@1・@5・@10・MRR）を出す。
async fn handle_eval_rag(
    indexes: Vec<String>,
    mode: String,
    top_k: i64,
    file: Option<PathBuf>,
    json: bool,
    verbose: bool,
) -> Result<()> {
    let paths = Paths::discover()?;
    let roots =
        sai_paper_core::paths::paper_roots().context("研究プロジェクトのルートを解決できません")?;
    let config =
        Config::load_or_create(&paths).context("failed to load or create configuration")?;

    let questions_path = file.unwrap_or_else(|| {
        roots
            .data_dir
            .join(sai_paper_core::rag_eval::QUESTIONS_RELATIVE_PATH)
    });

    let questions = match sai_paper_core::rag_eval::load_questions(&questions_path) {
        Ok(questions) => questions,
        Err(sai_paper_core::rag_eval::EvalError::NotFound { .. }) => {
            bail!(
                "質問集がありません: {}\n次の形で作ってください（問いは英語で）。ファイルに足すのはユーザーです。\n{}",
                questions_path.display(),
                sai_paper_core::rag_eval::SAMPLE_QUESTIONS_JSON
            );
        }
        Err(error) => bail!("{error}"),
    };

    let index_names = if indexes.is_empty() {
        vec![config.rag.default_index.clone()]
    } else {
        indexes
    };

    let db = open_database()?;
    let mut summaries = Vec::with_capacity(index_names.len());
    let mut all_scores = Vec::with_capacity(index_names.len());
    for index in &index_names {
        let (summary, scores) = sai_paper_core::rag_eval::run(
            db.connection(),
            &config,
            &questions,
            index,
            &mode,
            top_k,
        )
        .await
        .with_context(|| format!("索引 '{index}' の検索に失敗しました"))?;
        summaries.push(summary);
        all_scores.push(scores);
    }

    if json {
        println!("{}", serde_json::to_string_pretty(&summaries)?);
    } else {
        println!(
            "質問集: {}（{} 問）  mode={}  top-k={}",
            questions_path.display(),
            questions.len(),
            mode,
            top_k
        );
        let width = summaries
            .iter()
            .map(|summary| summary.index.chars().count())
            .max()
            .unwrap_or(0)
            .max("索引".chars().count());
        println!(
            "{:<width$}  {:>5}  {:>5}  {:>5}  {:>5}",
            "索引",
            "@1",
            "@5",
            "@10",
            "MRR",
            width = width
        );
        for summary in &summaries {
            println!(
                "{:<width$}  {:>5.2}  {:>5.2}  {:>5.2}  {:>5.2}",
                summary.index,
                summary.hit_at_1,
                summary.hit_at_5,
                summary.hit_at_10,
                summary.mrr,
                width = width
            );
        }
        if verbose {
            println!();
            println!("問いごとの最初の正解の順位（圏外は -）:");
            for (index, scores) in summaries.iter().zip(all_scores.iter()) {
                println!("[{}]", index.index);
                for (question, score) in questions.iter().zip(scores.iter()) {
                    let rank = score
                        .first_hit_rank
                        .map_or_else(|| "-".to_string(), |rank| rank.to_string());
                    println!("  {:>4}  {}", rank, question.q);
                }
            }
        }
    }
    Ok(())
}

/// 研究プロジェクトの [`Paths`] を組み立てる。
fn paper_paths(paper_dir: &std::path::Path) -> Paths {
    Paths {
        config_file: paper_dir.join(sai_paper_core::config::SETTINGS_FILE),
        database_file: paper_dir.join("papr.db"),
        downloads_dir: paper_dir.join("papers"),
        plugins_dir: paper_dir.join("plugins"),
        plugins_config_file: paper_dir.join("plugins.toml"),
    }
}

/// mind.md / state.md のひな形を配り、状況まとめ（`project-brief.md`）を 1 回作る。
///
/// 失敗しても init は成功扱いにし、警告だけ出す（sai-link はプレースホルダの
/// `project-brief.md` でも動くため）。`conn` は `papr.db`（無ければ `None`）。
fn ensure_project_files_and_brief(
    paths: &Paths,
    root: &std::path::Path,
    paper_dir: &std::path::Path,
    conn: Option<&rusqlite::Connection>,
) {
    match sai_paper_core::project_files::ensure(paper_dir) {
        Ok(written) => {
            for path in written {
                println!("✅ {} を作りました", path.display());
            }
        }
        Err(e) => eprintln!("⚠ mind.md / state.md を作れませんでした: {e}"),
    }

    let roots = sai_paper_core::paths::PaperRoots {
        root: root.to_path_buf(),
        data_dir: paper_dir.to_path_buf(),
    };
    let config = Config::load(&paths.config_file).unwrap_or_default();
    let text =
        sai_paper_core::brief::build_brief(&roots, conn, &config.brief, &config.rag.default_index);
    match sai_paper_core::brief::write_brief(&roots, &text) {
        Ok(path) => println!("✅ 状況まとめを作りました: {}", path.display()),
        Err(e) => eprintln!("⚠ 状況まとめを作れませんでした: {e}"),
    }
}

/// 研究プロジェクトを初期化する（`.sai/` と `.sai/paper/`）。
///
/// `assets_only` が `false` のときは 1 度だけ実行する。`.sai/` が無ければ
/// `sai-link init` をサブプロセスで実行し、`.sai/paper/` と `papr.db` を作成する。
/// `assets_only` が `true` のときは、既にあるプロジェクトに足りない部品だけを配り
/// （ブロック・テンプレート・設定）、`papr.db`・`sai-link init`・sai-control 登録は
/// 行わない。legacy `data/` の移行と `.sai/blocks/` の操作は行わない。
///
/// `domain` は DSR が使う分野の観点（`medical` / `signal-ml` / `general`）。
/// `--assets-only` でも選べる。
fn handle_init(
    name: Option<String>,
    root: Option<PathBuf>,
    assets_only: bool,
    domain: &str,
) -> Result<()> {
    let root = root
        .or_else(|| std::env::var("SAI_PAPER_ROOT").ok().map(PathBuf::from))
        .unwrap_or(std::env::current_dir().context("cwd が取得できません")?);
    let sai_dir = root.join(".sai");
    let paper_dir = sai_dir.join("paper");

    // 足りない部品だけを配るモード（既にあるプロジェクトを直さない）。
    if assets_only {
        if !sai_dir.is_dir() {
            bail!(
                "先に `sai-paper init` を実行してください: {} がありません",
                sai_dir.display()
            );
        }
        // エージェントの部品（ブロック・テンプレート・設定・MCP）を配る（段階 7）
        for line in
            agent_assets::install_here(&root).context("エージェントの部品を配れませんでした")?
        {
            println!("✅ {line}");
        }
        // 分野の観点（見本と、選んだ分野の domain.md）を配る
        for line in
            agent_assets::install_domain(&root, domain).context("分野の観点を配れませんでした")?
        {
            println!("✅ {line}");
        }
        let paths = paper_paths(&paper_dir);
        ensure_project_files_and_brief(&paths, &root, &paper_dir, None);
        println!("✅ 足りない部品を配りました: {}", paper_dir.display());
        return Ok(());
    }

    if paper_dir.join("papr.db").exists() {
        bail!(
            "既に初期化されています: {}\n`sai-paper init` はプロジェクト開始時に 1 度のみ使用します\n足りない部品だけ配るには `sai-paper init --assets-only`",
            paper_dir.display()
        );
    }

    // sai-moon-link 側の初期化（未実行なら実行する）
    if !sai_dir.exists() {
        match std::process::Command::new("sai-moon-link")
            .arg("init")
            .current_dir(&root)
            .env("SAI_PROJECT_ROOT", &root)
            .status()
        {
            Ok(status) if status.success() => println!("✅ sai-moon-link init を実行しました"),
            _ => bail!(
                "sai-moon-link init を実行できませんでした。先に実行してください: cd {} && sai-moon-link init",
                root.display()
            ),
        }
    }

    for dir in ["papers", "extracts", "bibtex", "plugins"] {
        std::fs::create_dir_all(paper_dir.join(dir))?;
    }

    // DB 作成（マイグレーション実行）
    let paths = paper_paths(&paper_dir);
    let database = Database::open(&paths.database_file).context("papr.db の作成に失敗")?;
    sai_paper_core::Config::load_or_create(&paths).context("settings.json の作成に失敗")?;

    // sai-control レジストリへ登録（任意）
    let project_name = name
        .filter(|n| !n.is_empty())
        .or_else(|| root.file_name().map(|n| n.to_string_lossy().to_string()))
        .unwrap_or_else(|| "unnamed".to_string());
    match std::process::Command::new("sai-control")
        .args([
            "registry",
            "add",
            "--name",
            &project_name,
            "--root",
            &root.to_string_lossy(),
        ])
        .status()
    {
        Ok(status) if status.success() => {
            println!("✅ sai-control レジストリに登録しました: {project_name}")
        }
        _ => eprintln!(
            "⚠ sai-control registry add を実行できませんでした（任意）。手動登録: sai-control registry add --name {project_name} --root {}",
            root.display()
        ),
    }

    // エージェントの部品（ブロック・テンプレート・pi の拡張・cc の MCP）を配る（段階 7）
    for line in agent_assets::install_here(&root).context("エージェントの部品を配れませんでした")? {
        println!("✅ {line}");
    }
    // 分野の観点（見本と、選んだ分野の domain.md）を配る
    for line in
        agent_assets::install_domain(&root, domain).context("分野の観点を配れませんでした")?
    {
        println!("✅ {line}");
    }

    // mind.md / state.md のひな形と、最初の状況まとめを作る（段階 8）
    ensure_project_files_and_brief(&paths, &root, &paper_dir, Some(database.connection()));

    println!(
        "✅ 研究プロジェクトを初期化しました: {}",
        paper_dir.display()
    );
    println!("   起動: sai-paper-tui  (直接操作)");
    println!("         sai-moon-link-tui   (pi agent 管理)");
    Ok(())
}

/// 検索結果から论文の番号を取り出す（重複なし・昇順。検索の記録に残す）。
fn paper_ids_of_hits(hits: &[sai_paper_core::Hit]) -> Vec<i64> {
    let mut ids: Vec<i64> = hits.iter().filter_map(|hit| hit.paper_id).collect();
    ids.sort_unstable();
    ids.dedup();
    ids
}

/// 墓標の候補（DOI か dedupe_key が墓標と一致するもの）を結果から外す（R-R7）。
///
/// 外した候補は検索セットには `drop` で残すため、ここでは数えるだけ。
/// 返すのは（残す論文, 外した件数）。
fn split_tombstones(
    conn: &rusqlite::Connection,
    papers: &[RemotePaper],
) -> (Vec<RemotePaper>, usize) {
    use sai_paper_core::sci::store;
    let mut kept = Vec::with_capacity(papers.len());
    let mut dropped = 0usize;
    for paper in papers {
        let key = sai_paper_core::api::dedupe_key(paper);
        let marked = store::find_tombstone(conn, None, paper.doi.as_deref(), None, Some(&key))
            .ok()
            .flatten()
            .is_some();
        if marked {
            dropped += 1;
        } else {
            kept.push(paper.clone());
        }
    }
    (kept, dropped)
}

/// `sai-paper search --save-as <名前>` の保存（U06）。
///
/// セットが無ければ作り、あれば同じセットへ足す（`expand --save-as` と同じ動き）。
/// 同じ名前で何度 `--save-as` してもセットは 1 つのままで、重複（`dedupe_key`）は
/// `add_items` が除きます。一時的なセットを作って `set delete` で消す必要がなくなる。
///
/// 返すのは保存先セットの id。
fn save_search_results(
    conn: &rusqlite::Connection,
    name: &str,
    source_dbs: &str,
    query: &str,
    raw_query: Option<&str>,
    papers: Vec<RemotePaper>,
) -> Result<i64> {
    use sai_paper_core::search_sets::{self, SearchSetError};

    // セットは無ければ作り、あればそのまま使う。
    let set_id = match search_sets::get_by_name(conn, name) {
        Ok(set) => set.id,
        Err(SearchSetError::NotFound(_)) => search_sets::create(
            conn,
            search_sets::CreateSearchSetParams {
                name: name.to_string(),
                source_dbs: source_dbs.to_string(),
                query: query.to_string(),
                raw_query: raw_query.map(|q| q.to_string()),
                created_by: "user".to_string(),
                note: None,
            },
        )?,
        Err(error) => return Err(error.into()),
    };

    search_sets::add_items(conn, set_id, papers)?;
    Ok(set_id)
}

#[tokio::main]
async fn main() -> Result<()> {
    // プロジェクトルートの .env を読み込む（S2 API キーのみ使用。既存の環境変数は上書きしない）
    sai_paper_core::paths::load_project_dotenv();
    let cli = Cli::parse();
    let client = S2Client::new(resolve_api_key())?;

    match cli.cmd {
        Some(Cmd::Search {
            query,
            db,
            limit,
            year_from,
            year_to,
            raw_query,
            save_as,
            node,
            question,
            purpose,
            extract,
            widen,
            json,
        }) => {
            let paths = Paths::discover()?;
            let cfg = sai_paper_core::Config::load_or_create(&paths)?;

            // 複数 DB で検索して重複を除く（`phone search` も同じ関数を使う）。
            let (dbs_to_use, deduped) =
                run_search(&cfg, &query, &db, limit, year_from, year_to, raw_query.as_deref())
                    .await?;

            // 問いと節点（`--node` / `--question`）は、記録とセットに書く前に読む。
            let database = open_database().ok();
            let refs = match database.as_ref() {
                Some(db) => {
                    sci::parse_search_refs(db.connection(), node.as_deref(), question.as_deref())?
                }
                None => sci::SearchRefs::default(),
            };

            // 墓標の候補は出力から除く（R-R7）。セットには `drop` で残すので、除前の結果を渡す。
            let (kept, dropped) = match database.as_ref() {
                Some(db) => split_tombstones(db.connection(), &deduped),
                None => (deduped.clone(), 0usize),
            };

            // --save-as オプションで検索セットを保存（同じ名前なら 1 つのセットに足す）
            let set_id = if let Some(set_name) = save_as {
                let Some(database) = database.as_ref() else {
                    bail!("検索セットを保存するには研究プロジェクト（.sai/paper）が必要です");
                };
                let source_dbs_str = dbs_to_use.join(",");

                let set_id = save_search_results(
                    database.connection(),
                    &set_name,
                    &source_dbs_str,
                    &query,
                    raw_query.as_deref(),
                    deduped.clone(),
                )?;

                // 何のための検索かをセットに書く（R-R6）。1 つでもあれば書く。
                let fields = sci::set_fields(&refs, purpose.as_deref(), extract.as_deref());
                if let Some(fields) = fields {
                    sai_paper_core::search_sets::set_sci_fields(
                        database.connection(),
                        set_id,
                        &fields,
                    )
                    .context("検索セットに検索の目的を書けませんでした")?;
                }
                Some(set_id)
            } else {
                None
            };

            // 問いか節点が指定されたときだけ検索の記録を残す。
            if let Some(database) = database.as_ref() {
                let scope = format!("external:{}", dbs_to_use.join(","));
                sci::record_search(
                    database.connection(),
                    &sci::SearchLog {
                        refs: &refs,
                        tool: "search",
                        query: &query,
                        scope: &scope,
                        widen_reason: widen.as_deref().unwrap_or(""),
                        hits: kept.len() as i64,
                        hit_papers: &[],
                        set_id,
                    },
                )?;
            }

            // 出力
            if json {
                println!("{}", serde_json::to_string_pretty(&kept)?);
            } else {
                for paper in kept {
                    println!("{} - {}", paper.id, paper.title);
                    if !paper.authors.is_empty() {
                        println!("  Authors: {}", paper.authors.join(", "));
                    }
                    if !paper.abstract_text.is_empty() {
                        let preview = if paper.abstract_text.len() > 100 {
                            format!("{}...", &paper.abstract_text[..100])
                        } else {
                            paper.abstract_text.clone()
                        };
                        println!("  Abstract: {}", preview);
                    }
                    println!();
                }
            }
            // 除外した件数は最後に 1 行で伝える（`--json` のときは JSON を壊さないよう標準エラー）。
            if dropped > 0 {
                let line = format!("除外済み {dropped} 件（理由は sai-moon-paper sci tombstones）");
                if json {
                    eprintln!("{line}");
                } else {
                    println!("{line}");
                }
            }
        }
        Some(Cmd::GetPdf { ids }) => {
            // 論文フォルダ（番号＋短いタイトル）に置くため、`papers.id` の解決に DB を使う。
            // DB が開けない環境では従来どおり `papers/` 直下に保存される。
            let db = open_database().ok();
            for id in ids {
                let key = citation::normalize_paper_id(&id);
                match litbase::ensure_pdf(&client, &key, db.as_ref()).await {
                    Ok(path) => println!("{key} -> {}", path.display()),
                    Err(e) => eprintln!("{key}: {e:#}"),
                }
            }
        }
        Some(Cmd::Extract { paper_id }) => {
            let db = open_database()?;
            let key = citation::normalize_paper_id(&paper_id);
            let pdf = litbase::ensure_pdf(&client, &key, Some(&db)).await?;
            let data_dir = Paths::discover()?.data_dir();
            println!("extracting {key} from {} ...", pdf.display());
            let out =
                citation::ensure_extract(db.connection(), &client, &key, &pdf, &data_dir).await?;
            println!(
                "{key}: {} refs, {} sentences (style: {})",
                out.refs.len(),
                out.sentences.len(),
                out.style
            );
        }
        Some(Cmd::Graph {
            paper_id,
            depth,
            direction,
            out,
            library_only,
        }) => {
            let db = open_database()?;
            let root = citation::normalize_paper_id(&paper_id);
            let summary = build_graph(&db, &client, &root, depth, &direction, library_only).await?;
            let json = serde_json::to_string_pretty(&summary)?;
            if let Some(out) = out {
                std::fs::write(&out, &json)?;
                println!("graph written to {}", out.display());
            } else {
                println!("{json}");
            }
        }
        Some(Cmd::GraphShow { paper_id }) => {
            let db = open_database()?;
            let root = citation::normalize_paper_id(&paper_id);
            show_graph(&db, &root)?;
        }
        Some(Cmd::Expand {
            papers,
            direction,
            limit,
            save_as,
            json,
        }) => {
            let direction = expand::Direction::parse(&direction)?;
            expand::handle(&client, papers, direction, limit, &save_as, json).await?;
        }
        Some(Cmd::Status) => {
            let db = open_database()?;
            show_status(&db)?;
        }
        Some(Cmd::Init {
            name,
            root,
            assets_only,
            domain,
        }) => {
            handle_init(name, root, assets_only, &domain)?;
        }
        Some(Cmd::Brief { write }) => {
            handle_brief(write)?;
        }
        Some(Cmd::Litbase { cmd }) => {
            litbase::handle(cmd, &client).await?;
        }
        Some(Cmd::Set { cmd }) => {
            search_sets::handle_search_set(cmd).await?;
        }
        Some(Cmd::Handoff { cmd, args }) => {
            handoff::handle_handoff(cmd, *args).await?;
        }
        Some(Cmd::Index { cmd }) => {
            index::handle_index(cmd).await?;
        }
        Some(Cmd::Find {
            query,
            mode,
            top_k,
            per_paper,
            scope,
            paper,
            index,
            alpha,
            fusion,
            fts_expr,
            evidence,
            papers,
            node,
            subtree,
            each,
            question,
            widen,
            json,
        }) => {
            // `--subtree` と `--each` には範囲の指定が要る（MCP の find も同じ検査）。
            sci::check_find_scope(subtree, node.is_some(), each, papers.is_empty())?;
            let db = open_database()?;
            let config = sai_paper_core::Config::load_or_create(&Paths::discover()?)
                .context("failed to load or create configuration")?;

            // 問いと節点（`--node` / `--question`）は、範囲を決める前に読む。
            let refs = sci::parse_search_refs(
                db.connection(),
                node.as_deref(),
                question.as_deref(),
            )?;
            // 論文の集合と、記録に書く範囲の文字列（C-09）。
            let (paper_ids, scope_note) = sai_paper_core::retrieval::resolve_paper_scope(
                db.connection(),
                paper,
                &papers,
                refs.given,
                subtree,
            )
            .map_err(|err| anyhow!("{err}"))?;

            // 前の検索を見る（記録は後で残す。`--json` の無い出力の 1 行目に出す）。
            let previous = if json {
                None
            } else {
                sci::previous_search_line(db.connection(), &refs, &scope_note)
            };

            // CLI・TUI・MCP で同じ core の関数を呼ぶ（段階 7）
            let request = sai_paper_core::retrieval::FindRequest {
                query: query.clone(),
                mode,
                top_k,
                per_paper,
                scope,
                paper_id: None,
                paper_ids,
                each,
                index,
                alpha,
                fusion,
                fts_expr,
            };
            // 要約付きのときだけ LLM のクライアントを作る（`model` が空ならここでエラー）。
            let chat = if evidence {
                Some(sai_paper_core::evidence::ChatCompletionsClient::from_config(&config)?)
            } else {
                None
            };
            let outcome = sai_paper_core::evidence::find_maybe_evidence(
                db.connection(),
                &config,
                request,
                evidence,
                chat.as_ref()
                    .map(|c| c as &dyn sai_paper_core::evidence::ChatClient),
            )
            .await
            .context("search failed")?;
            // 節点か問いの指定があれば検索の記録に残す（R-R3 / R-R4）。
            let (hit_count, hit_papers): (i64, Vec<i64>) = match &outcome {
                sai_paper_core::evidence::SearchOutcome::Plain(hits) => {
                    (hits.len() as i64, paper_ids_of_hits(hits))
                }
                sai_paper_core::evidence::SearchOutcome::Evidence(result) => (
                    result.hits.len() as i64,
                    paper_ids_of_hits(
                        &result
                            .hits
                            .iter()
                            .map(|item| item.hit.clone())
                            .collect::<Vec<_>>(),
                    ),
                ),
            };
            sci::record_search(
                db.connection(),
                &sci::SearchLog {
                    refs: &refs,
                    tool: "find",
                    query: &query,
                    scope: &scope_note,
                    widen_reason: widen.as_deref().unwrap_or(""),
                    hits: hit_count,
                    hit_papers: &hit_papers,
                    set_id: None,
                },
            )?;

            // 前の検索があるときは 1 行目に出す（`--json` の形は変えない）。
            if let Some(previous) = previous {
                println!("{previous}");
            }
            match outcome {
                sai_paper_core::evidence::SearchOutcome::Plain(hits) => {
                    if json {
                        println!(
                            "{}",
                            serde_json::to_string_pretty(
                                &sai_paper_core::retrieval::hits_to_json(&hits)
                            )?
                        );
                    } else {
                        print_search_results(&hits);
                    }
                }
                sai_paper_core::evidence::SearchOutcome::Evidence(result) => {
                    if json {
                        println!("{}", serde_json::to_string_pretty(&result)?);
                    } else {
                        print_evidence_results(&result);
                    }
                }
            }
        }
        Some(Cmd::Neighbors {
            paper_id,
            top_k,
            index,
            json,
        }) => {
            let db = open_database()?;
            let config = sai_paper_core::Config::load_or_create(&Paths::discover()?)
                .context("failed to load or create configuration")?;
            let hits = sai_paper_core::retrieval::find_neighbors(
                db.connection(),
                &config,
                paper_id,
                top_k,
                index.as_deref(),
            )
            .context("neighbors search failed")?;
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&sai_paper_core::retrieval::hits_to_json(&hits))?
                );
            } else {
                for hit in hits {
                    println!("chunk_id={}, score={:.4}", hit.chunk_id, hit.score);
                    println!("  {}", hit.text);
                }
            }
        }
        Some(Cmd::Relayout { apply, json }) => {
            relayout::handle_relayout(apply, json).await?;
        }
        Some(Cmd::TranslatePdf { paper_id }) => {
            let paths = Paths::discover()?;
            let config = sai_paper_core::Config::load_or_create(&paths)?;
            let path = sai_paper_core::pipeline::translate_pdf(
                &paths.database_file,
                paper_id,
                &config.translate.pdf,
            )
            .await
            .map_err(|message| anyhow::anyhow!(message))?;
            println!("翻訳 PDF: {}", path.display());
        }
        Some(Cmd::Mcp) => mcp::serve().await?,
        Some(Cmd::Config { cmd }) => {
            let paths = Paths::discover()?;
            match cmd {
                ConfigCmd::Path => println!("{}", paths.config_file.display()),
                ConfigCmd::Show => {
                    let config = sai_paper_core::Config::load_or_create(&paths)?;
                    println!("{}", serde_json::to_string_pretty(&config)?);
                    for (field, name) in config.missing_env_vars() {
                        eprintln!("⚠ 環境変数 {name} が空です（{field}）。.env か ~/.sai-moon/paper.env に置いてください");
                    }
                }
            }
        }
        Some(Cmd::Figures {
            paper_ids,
            all,
            force,
        }) => {
            figures::handle_figures(paper_ids, all, force).await?;
        }
        Some(Cmd::Report { cmd }) => {
            report::handle_report(cmd).await?;
        }
        Some(Cmd::Canvas { cmd }) => {
            canvas::handle(cmd)?;
        }
        Some(Cmd::Enrich { retraction, force }) => {
            if !retraction {
                bail!("使い方: sai-paper enrich --retraction [--force]");
            }
            enrich_retraction(force).await?;
        }
        Some(Cmd::Content { cmd }) => {
            content::handle_content(cmd)?;
        }
        Some(Cmd::Dsr { cmd }) => {
            dsr::handle(cmd)?;
        }
        Some(Cmd::Activity { cmd }) => {
            activity::handle(cmd)?;
        }
        Some(Cmd::Understanding { cmd }) => {
            understanding::handle(cmd)?;
        }
        Some(Cmd::Progress { json }) => {
            progress::handle(json)?;
        }
        Some(Cmd::Journal { days, date, json }) => {
            handle_journal(days, date.as_deref(), json)?;
        }
        Some(Cmd::Attention { cmd, json }) => {
            attention::handle(cmd, json)?;
        }
        Some(Cmd::Sci { cmd }) => {
            sci::run(cmd)?;
        }
        Some(Cmd::Phone { cmd }) => {
            phone::handle(cmd, &client).await?;
        }
        Some(Cmd::Eval { cmd }) => match cmd {
            EvalCmd::Rag {
                indexes,
                mode,
                top_k,
                file,
                json,
                verbose,
            } => {
                handle_eval_rag(indexes, mode, top_k, file, json, verbose).await?;
            }
        },
        None => {
            println!(
                "sai-paper — sai-paper-core CLI\nusage:\n  sai-paper search \"<query>\" --limit 10\n  sai-paper get-pdf <paperId>\n  sai-paper extract <paperId>\n  sai-paper graph <paperId> --depth 2 --direction both\n  sai-paper graph-show <paperId>\n  sai-paper status"
            );
        }
    }
    Ok(())
}

/// グラフキャッシュを構築する（S2 のみ・references / citations / both）。
async fn build_graph(
    db: &Database,
    client: &S2Client,
    root: &str,
    depth: u8,
    direction: &str,
    library_only: bool,
) -> Result<serde_json::Value> {
    let fetch_refs = matches!(direction, "both" | "refs" | "references" | "out");
    let fetch_cits = matches!(direction, "both" | "citations" | "in");
    anyhow::ensure!(
        fetch_refs || fetch_cits,
        "invalid --direction: {direction} (references / citations / both)"
    );

    // ノード: paperId -> (depth, title, year, venue, citationCount)
    let mut nodes: HashMap<
        String,
        (
            i64,
            Option<String>,
            Option<i64>,
            Option<String>,
            Option<i64>,
        ),
    > = HashMap::new();
    let mut edges: HashSet<(String, String, String)> = HashSet::new();

    let detail = client.get_paper(root).await.context("root lookup")?;
    nodes.insert(
        root.to_string(),
        (
            0,
            detail.title.clone(),
            detail.year.map(i64::from),
            detail.venue.clone(),
            detail.citation_count.map(i64::from),
        ),
    );

    let mut frontier = vec![root.to_string()];
    for d in 1..=i64::from(depth) {
        let mut next = Vec::new();
        for id in &frontier {
            if fetch_refs {
                for r in client.get_paper_references(id).await.unwrap_or_default() {
                    let Some(pid) = r.paper_id.clone() else {
                        continue;
                    };
                    // library_only の場合、papers テーブルに存在するかチェック
                    if library_only {
                        let exists: bool = db
                            .connection()
                            .query_row(
                                "SELECT 1 FROM papers WHERE s2_paper_id = ?1 LIMIT 1",
                                [&pid],
                                |_| Ok(true),
                            )
                            .unwrap_or(false);
                        if !exists {
                            continue;
                        }
                    }
                    if edges.insert((id.clone(), pid.clone(), "ref".into())) {
                        nodes.entry(pid.clone()).or_insert((
                            d,
                            r.title.clone(),
                            r.year.map(i64::from),
                            r.venue.clone(),
                            r.citation_count.map(i64::from),
                        ));
                        next.push(pid);
                    }
                }
            }
            if fetch_cits {
                for c in client.get_paper_citations(id).await.unwrap_or_default() {
                    let Some(pid) = c.paper_id.clone() else {
                        continue;
                    };
                    // library_only の場合、papers テーブルに存在するかチェック
                    if library_only {
                        let exists: bool = db
                            .connection()
                            .query_row(
                                "SELECT 1 FROM papers WHERE s2_paper_id = ?1 LIMIT 1",
                                [&pid],
                                |_| Ok(true),
                            )
                            .unwrap_or(false);
                        if !exists {
                            continue;
                        }
                    }
                    if edges.insert((pid.clone(), id.clone(), "cited_by".into())) {
                        nodes.entry(pid.clone()).or_insert((
                            d,
                            c.title.clone(),
                            c.year.map(i64::from),
                            c.venue.clone(),
                            c.citation_count.map(i64::from),
                        ));
                        next.push(pid);
                    }
                }
            }
        }
        frontier = next;
        if frontier.is_empty() {
            break;
        }
    }

    // s2_graph テーブルに保存（0015 マイグレーション）
    let conn = db.connection();
    let tx = conn.unchecked_transaction()?;
    tx.execute("DELETE FROM s2_graph_nodes WHERE root_id = ?1", [root])?;
    tx.execute("DELETE FROM s2_graph_edges WHERE root_id = ?1", [root])?;
    {
        let mut stmt = tx.prepare(
            "INSERT INTO s2_graph_nodes (root_id, paper_id, depth, direction, title, year, venue, citation_count)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        )?;
        for (pid, (d, title, year, venue, cc)) in &nodes {
            stmt.execute((root, pid, d, direction, title, year, venue, cc))?;
        }
    }
    {
        let mut stmt = tx.prepare(
            "INSERT INTO s2_graph_edges (root_id, src, dst, kind) VALUES (?1, ?2, ?3, ?4)",
        )?;
        for (src, dst, kind) in &edges {
            stmt.execute((root, src, dst, kind))?;
        }
    }
    tx.commit()?;

    Ok(serde_json::json!({
        "root": root,
        "depth": depth,
        "direction": direction,
        "nodes": nodes.len(),
        "edges": edges.len(),
    }))
}

/// 検索結果をターミナル向けに整形して出力する。
fn print_search_results(hits: &[sai_paper_core::Hit]) {
    for (idx, hit) in hits.iter().enumerate() {
        print_search_result(hit, idx + 1);
    }
}

/// 検索結果 1 件を表示する（`print_search_results` と `print_evidence_results` で共用）。
fn print_search_result(hit: &sai_paper_core::Hit, index: usize) {
    println!("\n--- Result {} ---", index);
    println!("Score: {:.4}", hit.score);
    println!("Source: {}", hit.source_kind);
    if let Some(cite) = sai_paper_core::retrieval::cite_for(hit) {
        println!("Cite: {cite}");
    }
    if let Some(section) = &hit.section {
        println!("Section: {section}");
    }
    if let Some(title) = &hit.title {
        println!("Title: {}", title);
    }
    // テキストスニペット（最初の 100 文字、改行で切る）
    let snippet = hit
        .text
        .lines()
        .next()
        .unwrap_or("")
        .chars()
        .take(100)
        .collect::<String>();
    println!("Text: {}", snippet);
    if hit.text.lines().count() > 1 || hit.text.len() > 100 {
        println!("  ...");
    }
}

/// 要約付きの検索の結果を表示する。各結果に関連度と要約を足し、最後にトークン数を出す。
fn print_evidence_results(result: &sai_paper_core::evidence::EvidenceResult) {
    for (idx, item) in result.hits.iter().enumerate() {
        print_search_result(&item.hit, idx + 1);
        if let Some(relevance) = item.relevance {
            println!("関連度 {relevance}/10");
        } else {
            println!("関連度 不明");
        }
        if let Some(summary) = &item.summary {
            println!("要約: {summary}");
        }
    }
    let total = result.hits.len() + result.dropped;
    println!(
        "\n要約付き: {total} 件中 {} 件を外しました。トークン 入力 {}・出力 {}（失敗 {}）",
        result.dropped,
        with_commas(result.prompt_tokens),
        with_commas(result.completion_tokens),
        result.failed,
    );
}

/// 3 桁ごとに区切った数（例: `12340` -> `12,340`）。
fn with_commas(value: u64) -> String {
    let digits = value.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// グラフキャッシュを表示する。
fn show_graph(db: &Database, root: &str) -> Result<()> {
    let conn = db.connection();
    let mut nodes: Vec<serde_json::Value> = Vec::new();
    {
        let mut stmt = conn.prepare(
            "SELECT paper_id, depth, title, year, venue, citation_count
             FROM s2_graph_nodes WHERE root_id = ?1 ORDER BY depth, paper_id",
        )?;
        let rows = stmt.query_map([root], |row| {
            Ok(serde_json::json!({
                "paper_id": row.get::<_, String>(0)?,
                "depth": row.get::<_, i64>(1)?,
                "title": row.get::<_, Option<String>>(2)?,
                "year": row.get::<_, Option<i64>>(3)?,
                "venue": row.get::<_, Option<String>>(4)?,
                "citation_count": row.get::<_, Option<i64>>(5)?,
            }))
        })?;
        for r in rows {
            nodes.push(r?);
        }
    }
    let mut edges: Vec<serde_json::Value> = Vec::new();
    {
        let mut stmt =
            conn.prepare("SELECT src, dst, kind FROM s2_graph_edges WHERE root_id = ?1")?;
        let rows = stmt.query_map([root], |row| {
            Ok(serde_json::json!({
                "src": row.get::<_, String>(0)?,
                "dst": row.get::<_, String>(1)?,
                "kind": row.get::<_, String>(2)?,
            }))
        })?;
        for r in rows {
            edges.push(r?);
        }
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "root": root,
            "nodes": nodes,
            "edges": edges,
        }))?
    );
    Ok(())
}

/// papr.db の概要を表示する。
fn show_status(db: &Database) -> Result<()> {
    let conn = db.connection();
    let papers: i64 = conn.query_row("SELECT COUNT(*) FROM papers", [], |r| r.get(0))?;
    let with_pdf: i64 = conn.query_row(
        "SELECT COUNT(*) FROM papers WHERE pdf_path IS NOT NULL",
        [],
        |r| r.get(0),
    )?;
    let with_s2: i64 = conn.query_row(
        "SELECT COUNT(*) FROM papers WHERE s2_paper_id IS NOT NULL",
        [],
        |r| r.get(0),
    )?;
    let mut extract_rows: Vec<serde_json::Value> = Vec::new();
    {
        let mut stmt = conn.prepare(
            "SELECT status, COUNT(*) FROM paper_extracts GROUP BY status ORDER BY status",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(serde_json::json!({
                "status": row.get::<_, String>(0)?,
                "count": row.get::<_, i64>(1)?,
            }))
        })?;
        for r in rows {
            extract_rows.push(r?);
        }
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "db": "papr.db",
            "papers": papers,
            "papers_with_pdf": with_pdf,
            "papers_with_s2_id": with_s2,
            "extracts": extract_rows,
        }))?
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sai_paper_core::search_sets;

    /// テスト用の検索候補を作る（id と DOI で dedupe_key が決まる）。
    fn remote_paper(id: &str, title: &str, doi: Option<&str>) -> RemotePaper {
        use chrono::Utc;

        RemotePaper {
            id: id.to_string(),
            title: title.to_string(),
            authors: vec!["Tester".to_string()],
            abstract_text: format!("abstract of {title}"),
            published: Utc::now(),
            updated: Utc::now(),
            categories: vec![],
            pdf_url: None,
            doi: doi.map(|d| d.to_string()),
            journal_ref: None,
            citation_count: None,
        }
    }

    /// U06 AC3: 同じ名前で 2 回 `--save-as` しても、セットは 1 つで候補は重複なく足される。
    #[test]
    fn ac3_same_save_as_name_reuses_one_set_without_duplicates() {
        let db = Database::in_memory().expect("メモリ DB を作れる");
        let conn = db.connection();

        let first = vec![
            remote_paper("arxiv:1", "Paper one", Some("10.1/one")),
            remote_paper("arxiv:2", "Paper two", None),
        ];

        let set_id = save_search_results(conn, "u06-check", "arxiv", "EEG LLM", None, first)
            .expect("1 回目は保存できる");

        // 2 回目は 1 つ重複（Paper one）+ 1 つ新しい（Paper three）。
        let second = vec![
            remote_paper("arxiv:1", "Paper one", Some("10.1/one")),
            remote_paper("arxiv:3", "Paper three", None),
        ];
        let set_id_again =
            save_search_results(conn, "u06-check", "arxiv", "sEEG LLM", None, second)
                .expect("2 回目はエラーにならない");

        assert_eq!(set_id, set_id_again, "同じセットに入る");

        let sets: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM search_sets WHERE name = 'u06-check'",
                [],
                |r| r.get(0),
            )
            .expect("セットを数えられる");
        assert_eq!(sets, 1, "セットは 1 つだけ");

        let items = search_sets::items(conn, set_id).expect("候補を読める");
        assert_eq!(items.len(), 3, "重複した候補は足されない");
        let titles: Vec<&str> = items.iter().map(|i| i.remote.title.as_str()).collect();
        assert_eq!(
            titles,
            vec!["Paper one", "Paper two", "Paper three"],
            "1 回目の候補のまま 2 回目の新しい候補が足される"
        );
    }

    /// 別の名前なら別のセットになる（名前でまとめられることの反対の確認）。
    #[test]
    fn ac3_different_save_as_names_make_different_sets() {
        let db = Database::in_memory().expect("メモリ DB を作れる");
        let conn = db.connection();

        let papers = vec![remote_paper("arxiv:1", "Paper one", None)];
        let a = save_search_results(conn, "set-a", "arxiv", "q", None, papers.clone())
            .expect("set-a を作れる");
        let b = save_search_results(conn, "set-b", "arxiv", "q", None, papers)
            .expect("set-b を作れる");

        assert_ne!(a, b, "名前が違えば別々");
        assert_eq!(search_sets::items(conn, a).expect("読む").len(), 1);
        assert_eq!(search_sets::items(conn, b).expect("読む").len(), 1);
    }
}
