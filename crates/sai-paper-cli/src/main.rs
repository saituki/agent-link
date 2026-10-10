//! sai-paper CLI — Rust native (旧 Node core.mjs は廃止)
//!
//! search / get-pdf / extract / graph / graph-show / status の全コマンドが
//! sai-paper-core を直接呼び出し、Node サブプロセスなしで動作する。
//! DB は papr.db（引用テーブル 0016 含む）を共有する。

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use sai_paper_core::citation;
use sai_paper_core::{Config, Database, Paths, RemotePaper, S2Client};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

mod activity;
mod agent_assets;
mod cli_error;
mod content;
mod expand;
mod figures;
mod index;
mod litbase;
mod phone;
mod phone_cite;
mod phone_import;
mod relayout;
mod schema;

#[derive(Parser, Debug)]
#[command(
    name = "sai-paper",
    version,
    about = "sai-paper CLI (Rust native, Node非依存)"
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
        /// JSON 出力（`{"log","items","failed"}`）
        #[arg(long)]
        json: bool,
    },
    /// 外部検索の履歴 1 件とヒット一覧（JSON）
    #[command(name = "search-history")]
    SearchHistory {
        /// 検索履歴 ID（search_history.id）
        log_id: i64,
        /// JSON 出力（いつも JSON。付けても同じ）
        #[arg(long)]
        json: bool,
    },
    /// Library の中を検索し、内部検索の履歴に残す
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
        /// インデックス対象（papers/notes/figures/all）
        #[arg(long)]
        scope: Option<String>,
        /// 論文 ID（繰り返し指定可）
        #[arg(long)]
        paper: Vec<i64>,
        /// 論文の集合で絞る（カンマ区切り。`--paper` と同じ扱い）
        #[arg(long, value_delimiter = ',')]
        papers: Vec<i64>,
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
        /// 範囲の論文ごとに上位 m 件を返す（`--papers` と一緒に使う）
        #[arg(long)]
        each: Option<i64>,
        /// JSON 出力（`{"log","hits"}`）
        #[arg(long)]
        json: bool,
    },
    /// 内部検索の履歴 1 件とヒット一覧（JSON）
    #[command(name = "find-history")]
    FindHistory {
        /// 内部検索の履歴 ID（find_history.id）
        log_id: i64,
        /// JSON 出力（いつも JSON。付けても同じ）
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
    /// 論文の引用の前後を 1 段たどり、Library に無いものを外部検索の履歴に残す（ネットを使う）
    Expand {
        /// 論文 ID（`papers.id`。複数指定可）
        papers: Vec<i64>,
        /// たどる向き（both / refs / cited）
        #[arg(long, default_value = "both")]
        direction: String,
        /// 向きごとに取る最大件数
        #[arg(long, default_value_t = 20)]
        limit: usize,
        /// JSON 出力
        #[arg(long)]
        json: bool,
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

    /// Library の論文の一覧・階層（JSON）
    Library {
        /// サブコマンド
        #[command(subcommand)]
        cmd: LibraryCmd,
    },
    /// 論文 1 件への操作（表示・訳・音声・引用・PDF）
    Paper {
        /// サブコマンド
        #[command(subcommand)]
        cmd: PaperCmd,
    },
    /// BibTeX の取り込み（sai-phone の Import 画面用）
    Import {
        /// サブコマンド
        #[command(subcommand)]
        cmd: phone_import::PhoneImportCmd,
    },
    /// RAG インデックス管理（チャンク化・全文索引）
    Index {
        /// サブコマンド
        #[command(subcommand)]
        cmd: index::IndexCmd,
    },
    /// 引用グラフのキャッシュを作る（`build`）／表示する（`show`）。
    ///
    /// 下位の命令が無いときは古い形（`graph <paper_id> …`）として受ける。
    #[command(args_conflicts_with_subcommands = true)]
    Graph {
        /// サブコマンド（無ければ古い形の引数を使う）
        #[command(subcommand)]
        cmd: Option<GraphCmd>,
        /// 古い形の引数（サブコマンドなしで使う）
        #[command(flatten)]
        args: Box<GraphLegacyArgs>,
    },

    /// Dashboard（TUI と同じ数字カード＋ Radio の出来事を混ぜた RECENT ACTIVITY）
    Dashboard {
        /// JSON 出力（いつも JSON。付けても同じ）
        #[arg(long)]
        json: bool,
    },
    /// 検索に使う DB 一覧（既定の DB と、使える全 DB）
    Providers {
        /// JSON 出力（いつも JSON。付けても同じ）
        #[arg(long)]
        json: bool,
    },

    /// DB status
    Status {
        /// JSON 出力（いつも JSON。付けても同じ）
        #[arg(long)]
        json: bool,
    },
    /// 研究プロジェクトを初期化する（`.sai/paper/` と `papr.db`）
    Init {
        /// 部品を新しい版へ更新する（古い物は退避する）
        #[arg(long)]
        update: bool,
    },
    /// 論文本文 / 引用箇所 / 節の内容を出力する（sai-link の入力展開用）
    Content {
        /// サブコマンド
        #[command(subcommand)]
        cmd: content::ContentCmd,
    },
    /// 「開いた」「エージェントが読んだ」の記録（N01）
    Activity {
        /// サブコマンド
        #[command(subcommand)]
        cmd: activity::ActivityCmd,
    },
    /// 設定（`.sai/paper/settings.json`）を確かめる
    Config {
        /// サブコマンド
        #[command(subcommand)]
        cmd: ConfigCmd,
    },
    /// 命令の一覧を JSON で出す
    Schema {
        /// JSON で出力する
        #[arg(long)]
        json: bool,
    },
    /// 既存データを新しい置き場の規則へ移す（古い別名。隠している）
    #[command(name = "relayout", hide = true)]
    Relayout {
        /// 実際に動かす（既定は予定の表示だけ）
        #[arg(long)]
        apply: bool,
        /// 機械可読出力（JSON）
        #[arg(long)]
        json: bool,
    },
    /// Library の検索（RAG）の精度を測る（古い別名。隠している）
    #[command(hide = true)]
    Eval {
        /// サブコマンド
        #[command(subcommand)]
        cmd: EvalCmd,
    },
}

/// `sai-paper library` のサブコマンド。
#[derive(Subcommand, Debug)]
enum LibraryCmd {
    /// Library の論文一覧（DB 登録日の新しい順、JSON）
    List {
        /// JSON 出力（いつも JSON。付けても同じ）
        #[arg(long)]
        json: bool,
    },
    /// Library のうち PDF の無い論文の一覧（PDF 待ち、新しい順。JSON）
    Pending {
        /// JSON 出力（いつも JSON。付けても同じ）
        #[arg(long)]
        json: bool,
    },
    /// Library の階層表示（検索ログごと → 日付ごとの Import、JSON）
    Tree {
        /// JSON 出力（いつも JSON。付けても同じ）
        #[arg(long)]
        json: bool,
    },
    /// 論文を「移植の束」のフォルダに書き出す（別のプロジェクトへ移すため。JSON）
    Export {
        /// 移植元の papers.id（カンマ区切り。例 5,6,12）
        #[arg(long, value_delimiter = ',')]
        ids: Vec<i64>,
        /// 書き出すフォルダの絶対パス（無いフォルダ）
        #[arg(long)]
        out: PathBuf,
        /// JSON 出力（いつも JSON。付けても同じ）
        #[arg(long)]
        json: bool,
    },
    /// 「移植の束」のフォルダを取り込む（JSON）
    #[command(name = "import-migration")]
    ImportMigration {
        /// 束のフォルダの絶対パス
        #[arg(long)]
        dir: PathBuf,
        /// JSON 出力（いつも JSON。付けても同じ）
        #[arg(long)]
        json: bool,
    },
}

/// `sai-paper paper` のサブコマンド。
#[derive(Subcommand, Debug)]
enum PaperCmd {
    /// 論文 1 件の詳細（JSON）
    Show {
        /// 論文 ID（papers.id）
        paper_id: i64,
        /// JSON 出力（いつも JSON。付けても同じ）
        #[arg(long)]
        json: bool,
    },
    /// ノート（note.md）を標準入力の中身で保存する（JSON）
    Note {
        /// 論文 ID（papers.id）
        paper_id: i64,
        /// JSON 出力（いつも JSON。付けても同じ）
        #[arg(long)]
        json: bool,
    },
    /// 要旨を日本語に訳して保存する（JSON）
    Translate {
        /// 論文 ID（papers.id）
        paper_id: i64,
        /// 訳済みでももう一度訳す
        #[arg(long)]
        force: bool,
        /// JSON 出力（いつも JSON。付けても同じ）
        #[arg(long)]
        json: bool,
    },
    /// PDF を日本語に訳す（数分かかる）
    #[command(name = "translate-pdf")]
    TranslatePdf {
        /// 論文 ID（papers.id）
        paper_id: i64,
        /// JSON 出力（`{"id","pdf_ja_path","status"}`）
        #[arg(long)]
        json: bool,
    },
    /// 音声化する（数分かかる。JSON）
    Radio {
        /// 論文 ID（papers.id）
        paper_id: i64,
        /// JSON 出力（いつも JSON。付けても同じ）
        #[arg(long)]
        json: bool,
    },
    /// 図表（Figure / Table）を抽出して figures/ と DB に保存する（文）
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
    /// 引用一覧（読み取りだけ。抜き出す前の論文にも空の形で返る。JSON）
    Citations {
        /// 論文 ID（papers.id）
        paper_id: i64,
        /// JSON 出力（いつも JSON。付けても同じ）
        #[arg(long)]
        json: bool,
    },
    /// 引用を PDF から抜き出す（docling を使うので数十秒。JSON）
    #[command(name = "citations-extract")]
    CitationsExtract {
        /// 論文 ID（papers.id）
        paper_id: i64,
        /// JSON 出力（いつも JSON。付けても同じ）
        #[arg(long)]
        json: bool,
    },
    /// 検索結果・BibTeX・引用から、PDF 無しで Library に論文を入れる（JSON）
    #[command(group(
        clap::ArgGroup::new("target").required(true).multiple(false).args(["log", "import_id", "cite"]),
    ))]
    Keep {
        /// 検索履歴 ID（`--position` とセットで指定）
        #[arg(long, requires = "position")]
        log: Option<i64>,
        /// 検索履歴の中での位置（0 始まり。`--log` とセットで指定）
        #[arg(long, requires = "log")]
        position: Option<i64>,
        /// BibTeX からの `RemotePaper.id`（例: `ArXiv:1706.03762`）
        #[arg(long = "import-id")]
        import_id: Option<String>,
        /// 引用を持つ論文 ID（`--ref` とセットで指定）
        #[arg(long)]
        cite: Option<i64>,
        /// 引用の番号（`--cite` とセットで指定）
        #[arg(long = "ref", requires = "cite")]
        ref_no: Option<i64>,
        /// JSON 出力（いつも JSON。付けても同じ）
        #[arg(long)]
        json: bool,
    },
    /// 論文の PDF を取得して Library に登録する（JSON）
    #[command(group(
        clap::ArgGroup::new("target").required(true).multiple(false).args(["log", "import_id", "cite"]),
    ))]
    Download {
        /// 検索履歴 ID（`--position` とセットで指定）
        #[arg(long, requires = "position")]
        log: Option<i64>,
        /// 検索履歴の中での位置（0 始まり。`--log` とセットで指定）
        #[arg(long, requires = "log")]
        position: Option<i64>,
        /// BibTeX からの `RemotePaper.id`（例: `ArXiv:1706.03762`）
        #[arg(long = "import-id")]
        import_id: Option<String>,
        /// 引用を持つ論文 ID（`--ref` とセットで指定）
        #[arg(long)]
        cite: Option<i64>,
        /// 引用の番号（`--cite` とセットで指定）
        #[arg(long = "ref", requires = "cite")]
        ref_no: Option<i64>,
        /// JSON 出力（いつも JSON。付けても同じ）
        #[arg(long)]
        json: bool,
    },
    /// Download の後の自動処理（図の抽出 → PDF の翻訳。JSON。標準エラーに sai-progress）
    #[command(name = "after-download")]
    AfterDownload {
        /// 論文 ID（papers.id）
        paper_id: i64,
        /// JSON 出力（いつも JSON。付けても同じ）
        #[arg(long)]
        json: bool,
    },
    /// ダウンロードフォルダの最近の PDF 一覧（手動取り込みの候補。JSON）
    Candidates {
        /// 何分前まで候補に出すか（既定 5 分）
        #[arg(long, default_value_t = 5)]
        minutes: u64,
        /// JSON 出力（いつも JSON。付けても同じ）
        #[arg(long)]
        json: bool,
    },
    /// 手動で保存した PDF（またはアップロード）を Library に入れる（JSON）
    #[command(name = "import-pdf")]
    ImportPdf {
        #[command(flatten)]
        args: Box<crate::phone_cite::ManualImportArgs>,
        /// JSON 出力（いつも JSON。付けても同じ）
        #[arg(long)]
        json: bool,
    },
    /// 前処理の束（sai-prep が作った zip を展開したフォルダ）を取り込む（JSON）
    #[command(name = "import-bundle")]
    ImportBundle {
        /// 展開したフォルダの絶対パス
        #[arg(long)]
        dir: PathBuf,
        /// JSON 出力（`{"paper_id","title","figures","sections","refs","translated"}`）
        #[arg(long)]
        json: bool,
    },
    /// PDF を取得して論文フォルダへ置く（文）
    #[command(name = "get-pdf")]
    GetPdf { ids: Vec<String> },
    /// Docling extract + ref parse + S2 match（saves to papr.db。文）
    Extract { paper_id: String },
}

/// `sai-paper graph` のサブコマンド。
#[derive(Subcommand, Debug)]
enum GraphCmd {
    /// 引用グラフのキャッシュを作る
    Build(GraphBuildArgs),
    /// キャッシュした引用グラフを表示する
    Show {
        /// 論文 ID
        paper_id: String,
    },
}

/// 引用グラフを作る引数（古い形 `graph <paper_id>` と `graph build` で共用）。
#[derive(clap::Args, Debug, Clone)]
struct GraphBuildArgs {
    /// 論文 ID（S2 / arXiv / DOI など）
    paper_id: String,
    /// たどる段数
    #[arg(long, default_value_t = 2)]
    depth: u8,
    /// references / citations / both
    #[arg(long, default_value = "both")]
    direction: String,
    /// 書き出すファイル
    #[arg(long)]
    out: Option<PathBuf>,
    /// Library にある論文だけに絞る
    #[arg(long)]
    library_only: bool,
}

/// 古い形 `graph <paper_id> …` の引数（`args_conflicts_with_subcommands` 用）。
///
/// 下位の命令があるときは使われないため、`paper_id` は `Option` にしてある。
#[derive(clap::Args, Debug)]
struct GraphLegacyArgs {
    /// 論文 ID（S2 / arXiv / DOI など）
    #[arg(value_name = "PAPER_ID")]
    paper_id: Option<String>,
    /// たどる段数
    #[arg(long, default_value_t = 2)]
    depth: u8,
    /// references / citations / both
    #[arg(long, default_value = "both")]
    direction: String,
    /// 書き出すファイル
    #[arg(long)]
    out: Option<PathBuf>,
    /// Library にある論文だけに絞る
    #[arg(long)]
    library_only: bool,
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
    let paths = Paths::discover()?;
    Database::open(&paths.database_file).context("failed to open papr.db")
}

/// 複数 DB で検索し、重複を除いた結果と、実際に使った DB 一覧、失敗した DB を返す。
///
/// 検索そのものは [`sai_paper_core::api::run_search`]（core）にある。
/// DB ごとの検索エラーは標準エラーに出して続ける（1 つでも成功すればその結果を返す）。
/// `sai-paper search` の共通処理。
pub(crate) async fn run_search(
    cfg: &Config,
    query: &str,
    db: &[String],
    limit: u16,
    year_from: Option<u16>,
    year_to: Option<u16>,
    raw_query: Option<&str>,
) -> Result<(Vec<String>, Vec<RemotePaper>, Vec<(String, String)>)> {
    let (dbs_to_use, deduped, failed) =
        sai_paper_core::api::run_search(cfg, query, db, limit, year_from, year_to, raw_query).await;
    // 従来と同じ形（`<db>: <理由>`）で標準エラーに出す。
    for (db_id, reason) in &failed {
        eprintln!("{db_id}: {reason}");
    }
    Ok((dbs_to_use, deduped, failed))
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

/// 研究プロジェクトを初期化する（`.sai/paper/` と `papr.db`）。
///
/// root は `SAI_PROJECT_ROOT`（空でない値）→ cwd。`.sai/paper/` と `papr.db` を作り、
/// `sai-link init` は呼ばない。sai-link が初期化済み（`.sai/settings.json` がある）
/// のときだけ、template を配る。初期化済みでも失敗にしない。
///
/// `update` が `true` のときは、古い template を退避して新しい版に置き換える。
fn handle_init(update: bool) -> Result<()> {
    // root は `SAI_PROJECT_ROOT`（空でない値）→ cwd。上へたどらない。
    let root = match std::env::var("SAI_PROJECT_ROOT") {
        Ok(value) if !value.is_empty() => PathBuf::from(value),
        _ => std::env::current_dir().context("cwd が取得できません")?,
    };
    let sai_dir = root.join(".sai");
    let paper_dir = sai_dir.join("paper");
    // sai-link が初期化済みか（`<root>/.sai/settings.json` があるか）だけを見る。
    let link_initialized = sai_dir.join("settings.json").is_file();

    // `.sai/paper/` の置き場と DB・settings.json を作る（あれば何もしない）。
    // `sai-link init` は呼ばない（sai-paper は単独で使える）。
    for dir in ["papers", "extracts", "bibtex", "plugins"] {
        std::fs::create_dir_all(paper_dir.join(dir))?;
    }
    let paths = paper_paths(&paper_dir);
    Database::open(&paths.database_file).context("papr.db の作成に失敗")?;
    sai_paper_core::Config::load_or_create(&paths).context("settings.json の作成に失敗")?;

    // sai-link の部品（template）は、初期化済みのときだけ配る。
    if link_initialized {
        for line in agent_assets::install_here(&root, update)
            .context("エージェントの部品を配れませんでした")?
        {
            println!("✅ {line}");
        }
    }

    println!(
        "✅ 研究プロジェクトを初期化しました: {}",
        paper_dir.display()
    );
    if !link_initialized {
        println!(
            "   sai-link と使うときは、`sai-link init` のあとで `sai-paper init` をもう一度実行してください"
        );
    }
    Ok(())
}

#[tokio::main]
async fn main() {
    let code = run().await;
    std::process::exit(code);
}

/// CLI の入口。終了コードを返す（CLI 共通の約束）。
async fn run() -> i32 {
    // プロジェクトルートの .env を読み込む（S2 API キーのみ使用。既存の環境変数は上書きしない）
    sai_paper_core::paths::load_project_dotenv();
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(err) => return clap_exit_code(err),
    };
    // 記録の出どころ（SAI_ORIGIN、無ければ cli）。
    sai_paper_core::jobs::set_origin(origin_name());
    let client = match S2Client::new(resolve_api_key()) {
        Ok(client) => client,
        Err(err) => return report_error(&anyhow::Error::from(err)),
    };
    match dispatch(cli.cmd, &client).await {
        Ok(()) => 0,
        Err(err) => report_error(&err),
    }
}

/// 記録の出どころ（CLI 共通の約束の 6）。
fn origin_name() -> &'static str {
    match std::env::var("SAI_ORIGIN") {
        Ok(value) if value == "phone" => "phone",
        _ => "cli",
    }
}

/// clap の使い方の誤りの終了コード。`--help`・`--version` は 0。
/// 誤りのときは clap の複数行の文を出さず、日本語の 1 行だけを標準エラーに書く。
fn clap_exit_code(err: clap::Error) -> i32 {
    use clap::error::ErrorKind;
    let kind = err.kind();
    if matches!(kind, ErrorKind::DisplayHelp | ErrorKind::DisplayVersion) {
        let _ = err.print();
        return 0;
    }
    // 命令が足りないときは使い方を出す（従来どおり）
    if kind == ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand {
        let _ = err.print();
    }
    let text = err.to_string();
    // clap の文の最初の段落（空行まで）を 1 行にまとめる
    let first = text
        .trim_start()
        .split("\n\n")
        .next()
        .unwrap_or("")
        .lines()
        .map(str::trim)
        .collect::<Vec<_>>()
        .join(" ");
    let stripped = first.strip_prefix("error: ").unwrap_or(&first);
    eprintln!("引数の誤り: {stripped}");
    2
}

/// エラーを標準エラーの最後の行に 1 行で出し、約束の終了コードを返す。
fn report_error(err: &anyhow::Error) -> i32 {
    let code = exit_code_of(err);
    let message = format!("{err:#}").replace('\n', " ");
    eprintln!("{message}");
    code
}

/// 約束の終了コードを決める（2・3・4。それ以外は 1）。
fn exit_code_of(err: &anyhow::Error) -> i32 {
    for cause in err.chain() {
        if let Some(cli_error) = cause.downcast_ref::<cli_error::CliError>() {
            return cli_error.code;
        }
        if let Some(config_error) = cause.downcast_ref::<sai_paper_core::config::ConfigError>() {
            return match config_error {
                sai_paper_core::config::ConfigError::ProjectNotFound
                | sai_paper_core::config::ConfigError::PaperDirMissing => 2,
                _ => 1,
            };
        }
    }
    1
}

/// 命令を振り分ける。
async fn dispatch(cmd: Option<Cmd>, client: &S2Client) -> Result<()> {
    match cmd {
        Some(Cmd::Search {
            query,
            db,
            limit,
            year_from,
            year_to,
            raw_query,
            json,
        }) => {
            handle_search(SearchArgs {
                query,
                db,
                limit,
                year_from,
                year_to,
                raw_query,
                json,
            })
            .await?;
        }
        Some(Cmd::SearchHistory { log_id, json: _ }) => {
            let (db, cfg, paths) = phone::open_context()?;
            let value = phone::log(&db, &cfg, &paths, log_id)?;
            let text = serde_json::to_string(&value)?;
            sai_paper_core::seen_ids::record(&text);
            println!("{text}");
        }
        Some(Cmd::Graph { cmd, args }) => match cmd {
            Some(GraphCmd::Show { paper_id }) => {
                let db = open_database()?;
                let root = citation::normalize_paper_id(&paper_id);
                show_graph(&db, &root)?;
            }
            Some(GraphCmd::Build(build)) => {
                run_graph_build(client, build).await?;
            }
            None => {
                let Some(paper_id) = args.paper_id.clone() else {
                    return Err(cli_error::CliError::input(
                        "論文 ID を指定してください（`graph build <paper_id>`）",
                    )
                    .into());
                };
                run_graph_build(
                    client,
                    GraphBuildArgs {
                        paper_id,
                        depth: args.depth,
                        direction: args.direction.clone(),
                        out: args.out.clone(),
                        library_only: args.library_only,
                    },
                )
                .await?;
            }
        },
        Some(Cmd::Expand {
            papers,
            direction,
            limit,
            json,
        }) => {
            let direction = expand::Direction::parse(&direction)?;
            expand::handle(&client, papers, direction, limit, json).await?;
        }
        Some(Cmd::Status { json: _ }) => {
            let db = open_database()?;
            show_status(&db)?;
        }
        Some(Cmd::Init { update }) => {
            handle_init(update)?;
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
            papers,
            index,
            alpha,
            fusion,
            fts_expr,
            evidence,
            each,
            json,
        }) => {
            handle_find(FindCliArgs {
                query,
                mode,
                top_k,
                per_paper,
                scope,
                paper,
                papers,
                index,
                alpha,
                fusion,
                fts_expr,
                evidence,
                each,
                json,
            })
            .await?;
        }
        Some(Cmd::FindHistory { log_id, json: _ }) => {
            let (db, _cfg, _paths) = phone::open_context()?;
            let value = phone::find_history_view(&db, log_id)?;
            let text = serde_json::to_string(&value)?;
            sai_paper_core::seen_ids::record(&text);
            println!("{text}");
        }
        Some(Cmd::Library { cmd }) => match cmd {
            LibraryCmd::List { json: _ } => {
                let value = phone::library()?;
                println!("{}", serde_json::to_string(&value)?);
            }
            LibraryCmd::Pending { json: _ } => {
                let value = phone::library_pending()?;
                println!("{}", serde_json::to_string(&value)?);
            }
            LibraryCmd::Tree { json: _ } => {
                let value = phone::tree()?;
                println!("{}", serde_json::to_string(&value)?);
            }
            LibraryCmd::Export { ids, out, json: _ } => {
                let (_db, _cfg, paths) = phone::open_context()?;
                let data_dir = paths.data_dir();
                let value = match sai_paper_core::migration::export(
                    &paths.database_file,
                    &data_dir,
                    &ids,
                    &out,
                ) {
                    Ok(value) => value,
                    Err(error) => {
                        return Err(crate::cli_error::CliError {
                            code: error.code(),
                            message: error.to_string(),
                        }
                        .into());
                    }
                };
                println!("{}", serde_json::to_string(&value)?);
            }
            LibraryCmd::ImportMigration { dir, json: _ } => {
                let (mut db, _cfg, paths) = phone::open_context()?;
                let data_dir = paths.data_dir();
                let value = match sai_paper_core::migration::import(&mut db, &data_dir, &dir) {
                    Ok(value) => value,
                    Err(error) => {
                        return Err(crate::cli_error::CliError {
                            code: error.code(),
                            message: error.to_string(),
                        }
                        .into());
                    }
                };
                println!("{}", serde_json::to_string(&value)?);
            }
        },
        Some(Cmd::Paper { cmd }) => handle_paper(cmd, client).await?,
        Some(Cmd::Import { cmd }) => {
            let value = phone_import::run_import(cmd)?;
            println!("{}", serde_json::to_string(&value)?);
        }
        Some(Cmd::Dashboard { json: _ }) => {
            let value = phone::dashboard()?;
            println!("{}", serde_json::to_string(&value)?);
        }
        Some(Cmd::Providers { json: _ }) => {
            let value = phone::providers()?;
            println!("{}", serde_json::to_string(&value)?);
        }
        Some(Cmd::Schema { json }) => {
            schema::print_schema(json)?;
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
            let text = if json {
                serde_json::to_string_pretty(&sai_paper_core::retrieval::hits_to_json_with(
                    &hits,
                    &sai_paper_core::sections::passages_for_hits(db.connection(), &hits)
                ))?
                + "\n"
            } else {
                hits.iter()
                    .map(|hit| format!("chunk_id={}, score={:.4}\n  {}\n", hit.chunk_id, hit.score, hit.text))
                    .collect()
            };
            sai_paper_core::seen_ids::record(&text);
            print!("{text}");
        }
        Some(Cmd::Relayout { apply, json }) => {
            relayout::handle_relayout(apply, json).await?;
        }
        Some(Cmd::Config { cmd }) => {
            let paths = Paths::discover()?;
            match cmd {
                ConfigCmd::Path => println!("{}", paths.config_file.display()),
                ConfigCmd::Show => {
                    let config = sai_paper_core::Config::load_or_create(&paths)?;
                    println!("{}", serde_json::to_string_pretty(&config)?);
                    for (field, name) in config.missing_env_vars() {
                        eprintln!("⚠ 環境変数 {name} が空です（{field}）。.env か ~/.sai/paper.env に置いてください");
                    }
                }
            }
        }
        Some(Cmd::Enrich { retraction, force }) => {
            if !retraction {
                bail!("使い方: sai-paper enrich --retraction [--force]");
            }
            enrich_retraction(force).await?;
        }
        Some(Cmd::Content { cmd }) => {
            content::handle_content(cmd).await?;
        }
        Some(Cmd::Activity { cmd }) => {
            activity::handle(cmd)?;
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
                "sai-paper — sai-paper-core CLI\nusage:\n  sai-paper search \"<query>\" --limit 10\n  sai-paper paper download --import-id <id>\n  sai-paper graph build <paperId> --depth 2 --direction both\n  sai-paper status"
            );
        }
    }
    Ok(())
}

/// `SAI_PARENT_RUN_ID` の実行 ID（空・数でないときは `None`）。
fn parent_run_id() -> Option<i64> {
    std::env::var("SAI_PARENT_RUN_ID")
        .ok()
        .and_then(|text| text.trim().parse::<i64>().ok())
}

/// 実行者（`human` / `agent`）。Web（`SAI_ORIGIN=phone`）は human、
/// それ以外で `SAI_PARENT_RUN_ID` が数なら agent、他は human。
fn executer_name() -> &'static str {
    executer_of(origin_name(), parent_run_id())
}

/// [`executer_name`] の判定（環境変数を読まない形）。
fn executer_of(origin: &str, run_id: Option<i64>) -> &'static str {
    if origin != "phone" && run_id.is_some() {
        "agent"
    } else {
        "human"
    }
}

/// 履歴に書く実行 ID。human のときは書かない。
fn run_id_for(executer: &str) -> Option<i64> {
    if executer == "agent" { parent_run_id() } else { None }
}

/// `search` の引数（clap の [`Cmd::Search`] の 1 対 1）。
struct SearchArgs {
    query: String,
    db: Vec<String>,
    limit: u16,
    year_from: Option<u16>,
    year_to: Option<u16>,
    raw_query: Option<String>,
    json: bool,
}

/// `search` — 複数 DB で検索し、履歴に残す（CLI 共通の約束）。
async fn handle_search(args: SearchArgs) -> Result<()> {
    let SearchArgs {
        query,
        db,
        limit,
        year_from,
        year_to,
        raw_query,
        json,
    } = args;
    let query = query.trim();
    if query.is_empty() {
        return Err(cli_error::CliError::input("検索語が空です").into());
    }
    let limit = limit.clamp(1, 100);

    let (database, cfg, paths) = phone::open_context()?;
    let (dbs_to_use, deduped, failed) =
        run_search(&cfg, query, &db, limit, year_from, year_to, raw_query.as_deref()).await?;

    let conn = database.connection();
    let source_dbs_str = dbs_to_use.join(",");
    let year = phone::year_label(year_from, year_to);
    let origin = origin_name();
    // 検索の履歴は毎回必ず残す（Library の階層に出る）。1 回ごとに別の履歴にする。
    let executer = executer_name();
    let log_id = sai_paper_core::search_history::record(
        conn,
        origin,
        executer,
        run_id_for(executer),
        query,
        &year,
        &limit.to_string(),
        &source_dbs_str,
        &deduped,
    )?;
    // Dashboard の出来事に「discover」として残すのは、窓口から呼ばれたときだけ。
    if origin == "phone" {
        database.record_activity("discover", None, Some(query))?;
    }

    if json {
        let value = phone::with_failed(phone::log(&database, &cfg, &paths, log_id)?, &failed)?;
        println!("{}", serde_json::to_string(&value)?);
    } else {
        for paper in &deduped {
            print_remote_paper(paper);
        }
    }
    Ok(())
}

/// 検索結果 1 件を人向けに出す（要旨は文字単位で先頭 100 字）。
fn print_remote_paper(paper: &RemotePaper) {
    println!("{} - {}", paper.id, paper.title);
    if !paper.authors.is_empty() {
        println!("  Authors: {}", paper.authors.join(", "));
    }
    if !paper.abstract_text.is_empty() {
        let preview: String = if paper.abstract_text.chars().count() > 100 {
            let head: String = paper.abstract_text.chars().take(100).collect();
            format!("{head}...")
        } else {
            paper.abstract_text.clone()
        };
        println!("  Abstract: {preview}");
    }
    println!();
}

/// `find` の引数（clap の [`Cmd::Find`] の 1 対 1）。
struct FindCliArgs {
    query: String,
    mode: Option<String>,
    top_k: Option<i64>,
    per_paper: Option<i64>,
    scope: Option<String>,
    paper: Vec<i64>,
    papers: Vec<i64>,
    index: Option<String>,
    alpha: Option<f64>,
    fusion: Option<String>,
    fts_expr: bool,
    evidence: bool,
    each: Option<i64>,
    json: bool,
}

/// `find` — Library の中を検索し、内部検索の履歴に残す。
///
/// `--json` は `{"log","hits"}`（`--evidence` のときは `"evidence"` を足す）。
async fn handle_find(args: FindCliArgs) -> Result<()> {
    let FindCliArgs {
        query,
        mode,
        top_k,
        per_paper,
        scope,
        paper,
        papers,
        index,
        alpha,
        fusion,
        fts_expr,
        evidence,
        each,
        json,
    } = args;
    let query = query.trim();
    if query.is_empty() {
        return Err(cli_error::CliError::input("検索語が空です").into());
    }
    if let Some(mode) = &mode
        && !matches!(mode.as_str(), "word" | "vector" | "hybrid")
    {
        return Err(cli_error::CliError::input(
            "検索モードが正しくありません（word / vector / hybrid）",
        )
        .into());
    }
    if let Some(scope) = &scope
        && !matches!(scope.as_str(), "papers" | "notes" | "figures" | "all")
    {
        return Err(cli_error::CliError::input(
            "検索対象が正しくありません（papers / notes / figures / all）",
        )
        .into());
    }
    // `--paper`（繰り返し）と `--papers`（コンマ区切り）を 1 つの列にまとめる。
    let mut all_papers = paper;
    all_papers.extend(papers);
    // `--each` には範囲の指定が要る（Node の指定は別名が先に取る）。
    if each.is_some() && all_papers.is_empty() {
        return Err(cli_error::CliError::input("--each は --papers と一緒に使います").into());
    }

    let (db, config, _paths) = phone::open_context()?;
    let conn = db.connection();
    let (paper_ids, scope_note) =
        sai_paper_core::retrieval::resolve_paper_scope(conn, None, &all_papers)?;

    // 内部検索の履歴に残す範囲のコピー（request が値を動かすため先に写す）。
    let log_mode = mode.clone();
    let log_scope = scope.clone();
    let log_papers: Vec<i64> = paper_ids.clone().unwrap_or_default();
    let request = sai_paper_core::retrieval::FindRequest {
        query: query.to_string(),
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
        conn,
        &config,
        request,
        evidence,
        chat.as_ref()
            .map(|c| c as &dyn sai_paper_core::evidence::ChatClient),
    )
    .await
    .context("search failed")?;
    let plain_hits: Vec<sai_paper_core::Hit> = match &outcome {
        sai_paper_core::evidence::SearchOutcome::Plain(hits) => hits.clone(),
        sai_paper_core::evidence::SearchOutcome::Evidence(result) => {
            result.hits.iter().map(|item| item.hit.clone()).collect()
        }
    };
    let passages = sai_paper_core::sections::passages_for_hits(conn, &plain_hits);
    // 履歴に残す（記録に失敗したら終了コード 1）。
    let executer = executer_name();
    let log_id = sai_paper_core::find_history::record(
        conn,
        &sai_paper_core::find_history::FindLogInput {
            query,
            mode: log_mode.as_deref(),
            top_k,
            scope: log_scope.as_deref(),
            range_text: &scope_note,
            papers: &log_papers,
            node_id: None,
            subtree: false,
            each,
            origin: origin_name(),
            executer,
            run_id: run_id_for(executer),
        },
        &plain_hits,
        &passages,
    )?;

    if json {
        let mut value = phone::find_history_view(&db, log_id)?;
        if let sai_paper_core::evidence::SearchOutcome::Evidence(result) = &outcome
            && let serde_json::Value::Object(map) = &mut value
        {
            map.insert("evidence".to_string(), serde_json::to_value(result)?);
        }
        let text = serde_json::to_string(&value)?;
        sai_paper_core::seen_ids::record(&text);
        println!("{text}");
    } else {
        let text = match outcome {
            sai_paper_core::evidence::SearchOutcome::Plain(hits) => search_results_text(&hits),
            sai_paper_core::evidence::SearchOutcome::Evidence(result) => {
                evidence_results_text(&result)
            }
        };
        sai_paper_core::seen_ids::record(&text);
        print!("{text}");
    }
    Ok(())
}

/// `paper …` の入口。
async fn handle_paper(cmd: PaperCmd, client: &S2Client) -> Result<()> {
    match cmd {
        PaperCmd::Show { paper_id, json: _ } => {
            let value = phone::paper(paper_id)?;
            println!("{}", serde_json::to_string(&value)?);
        }
        PaperCmd::Note { paper_id, json: _ } => {
            let value = phone::save_note(paper_id)?;
            println!("{}", serde_json::to_string(&value)?);
        }
        PaperCmd::Translate { paper_id, force, json: _ } => {
            let value = phone::translate(paper_id, force).await?;
            println!("{}", serde_json::to_string(&value)?);
        }
        PaperCmd::TranslatePdf { paper_id, json } => {
            let value = phone::translate_pdf(paper_id).await?;
            if json {
                println!("{}", serde_json::to_string(&value)?);
            } else {
                let path = value
                    .get("pdf_ja_path")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("");
                println!("翻訳 PDF: {path}");
            }
        }
        PaperCmd::Radio { paper_id, json: _ } => {
            let value = phone::radio(paper_id).await?;
            println!("{}", serde_json::to_string(&value)?);
        }
        PaperCmd::Figures {
            paper_ids,
            all,
            force,
        } => {
            figures::handle_figures(paper_ids, all, force).await?;
        }
        PaperCmd::Citations { paper_id, json: _ } => {
            let value = phone_cite::citations(paper_id)?;
            println!("{}", serde_json::to_string(&value)?);
        }
        PaperCmd::CitationsExtract { paper_id, json: _ } => {
            let value = phone_cite::citations_extract(paper_id, client).await?;
            println!("{}", serde_json::to_string(&value)?);
        }
        PaperCmd::Download {
            log,
            position,
            import_id,
            cite,
            ref_no,
            json: _,
        } => {
            let _ = (log, position, import_id, cite, ref_no);
            return Err(crate::cli_error::CliError::conflict(
                "サーバでは PDF を落としません。sai-prep から送ってください",
            )
            .into());
        }
        PaperCmd::AfterDownload { paper_id, json: _ } => {
            let value = phone_import::after_download(paper_id).await?;
            println!("{}", serde_json::to_string(&value)?);
        }
        PaperCmd::Candidates { minutes, json: _ } => {
            let value = phone_cite::manual_candidates(minutes)?;
            println!("{}", serde_json::to_string(&value)?);
        }
        PaperCmd::ImportPdf { args, json: _ } => {
            let value = phone_cite::manual_import_command(&args)?;
            println!("{}", serde_json::to_string(&value)?);
        }
        PaperCmd::ImportBundle { dir, json: _ } => {
            let (mut db, _cfg, paths) = phone::open_context()?;
            let data_dir = paths.data_dir();
            let value = match sai_paper_core::bundle::import_bundle(&mut db, &data_dir, &dir) {
                Ok(value) => value,
                Err(error) => {
                    return Err(crate::cli_error::CliError {
                        code: error.code(),
                        message: error.to_string(),
                    }
                    .into());
                }
            };
            println!("{}", serde_json::to_string(&value)?);
        }
        PaperCmd::Keep {
            log,
            position,
            import_id,
            cite,
            ref_no,
            json: _,
        } => {
            let value = phone_import::keep(
                log,
                position,
                import_id.as_deref(),
                cite,
                ref_no,
            )?;
            println!("{}", serde_json::to_string(&value)?);
        }
        PaperCmd::GetPdf { ids } => {
            let _ = ids;
            return Err(crate::cli_error::CliError::conflict(
                "サーバでは PDF を落としません。sai-prep から送ってください",
            )
            .into());
        }
        PaperCmd::Extract { paper_id } => {
            let _ = paper_id;
            return Err(crate::cli_error::CliError::conflict(
                "サーバでは PDF を落としません。sai-prep から送ってください",
            )
            .into());
        }
    }
    Ok(())
}

/// `paper get-pdf` — 1 件ずつの失敗は標準エラーに出して続け、全部失敗したときだけ 1。
async fn handle_get_pdf(client: &S2Client, ids: Vec<String>) -> Result<()> {
    let db = open_database().ok();
    let mut ok = 0usize;
    let mut failed = 0usize;
    for id in ids {
        let key = citation::normalize_paper_id(&id);
        match litbase::ensure_pdf(client, &key, db.as_ref()).await {
            Ok(path) => {
                println!("{key} -> {}", path.display());
                ok += 1;
            }
            Err(e) => {
                eprintln!("{key}: {e:#}");
                failed += 1;
            }
        }
    }
    if ok == 0 && failed > 0 {
        bail!("すべて失敗しました（{failed} 件）");
    }
    Ok(())
}

/// `paper extract` — Docling 抽出 + ref parse + S2 match。
async fn handle_extract(client: &S2Client, paper_id: &str) -> Result<()> {
    let db = open_database()?;
    let key = citation::normalize_paper_id(paper_id);
    let pdf = litbase::ensure_pdf(client, &key, Some(&db)).await?;
    let data_dir = Paths::discover()?.data_dir();
    println!("extracting {key} from {} ...", pdf.display());
    let out = citation::ensure_extract(db.connection(), client, &key, &pdf, &data_dir).await?;
    println!(
        "{key}: {} refs, {} sentences (style: {})",
        out.refs.len(),
        out.sentences.len(),
        out.style
    );
    Ok(())
}

/// `graph build` / 古い形 `graph <paper_id>` の本体。
async fn run_graph_build(client: &S2Client, args: GraphBuildArgs) -> Result<()> {
    let db = open_database()?;
    let root = citation::normalize_paper_id(&args.paper_id);
    let summary = build_graph(&db, client, &root, args.depth, &args.direction, args.library_only).await?;
    let json = serde_json::to_string_pretty(&summary)?;
    if let Some(out) = args.out {
        std::fs::write(&out, &json)?;
        println!("graph written to {}", out.display());
    } else {
        println!("{json}");
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

/// 検索結果をターミナル向けに整形した文字列（各行の改行を含む）。
fn search_results_text(hits: &[sai_paper_core::Hit]) -> String {
    hits.iter()
        .enumerate()
        .map(|(idx, hit)| search_result_text(hit, idx + 1))
        .collect()
}

/// 検索結果 1 件の表示（`search_results_text` と `evidence_results_text` で共用）。
fn search_result_text(hit: &sai_paper_core::Hit, index: usize) -> String {
    let mut out = format!(
        "\n--- Result {} ---\nScore: {:.4}\nSource: {}\n",
        index, hit.score, hit.source_kind
    );
    if let Some(cite) = sai_paper_core::retrieval::cite_for(hit) {
        out.push_str(&format!("Cite: {cite}\n"));
    }
    if let Some(section) = &hit.section {
        out.push_str(&format!("Section: {section}\n"));
    }
    if let Some(title) = &hit.title {
        out.push_str(&format!("Title: {}\n", title));
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
    out.push_str(&format!("Text: {}\n", snippet));
    if hit.text.lines().count() > 1 || hit.text.len() > 100 {
        out.push_str("  ...\n");
    }
    out
}

/// 要約付きの検索の結果の表示。各結果に関連度と要約を足し、最後にトークン数を出す。
fn evidence_results_text(result: &sai_paper_core::evidence::EvidenceResult) -> String {
    let mut out = String::new();
    for (idx, item) in result.hits.iter().enumerate() {
        out.push_str(&search_result_text(&item.hit, idx + 1));
        if let Some(relevance) = item.relevance {
            out.push_str(&format!("関連度 {relevance}/10\n"));
        } else {
            out.push_str("関連度 不明\n");
        }
        if let Some(summary) = &item.summary {
            out.push_str(&format!("要約: {summary}\n"));
        }
    }
    let total = result.hits.len() + result.dropped;
    out.push_str(&format!(
        "\n要約付き: {total} 件中 {} 件を外しました。トークン 入力 {}・出力 {}（失敗 {}）\n",
        result.dropped,
        with_commas(result.prompt_tokens),
        with_commas(result.completion_tokens),
        result.failed,
    ));
    out
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

    /// U19 AC2: 消した隠した命令・隠した引数は clap の誤りになる（終了コード 2）。
    #[test]
    fn removed_commands_and_args_are_rejected() {
        use clap::Parser as _;
        for args in [
            vec!["sai-paper", "phone", "dashboard"],
            vec!["sai-paper", "sci", "status"],
            vec!["sai-paper", "mcp"],
            vec!["sai-paper", "brief"],
            vec!["sai-paper", "journal"],
            vec!["sai-paper", "understanding", "ls"],
            vec!["sai-paper", "attention"],
            vec!["sai-paper", "dsr", "check"],
            vec!["sai-paper", "report", "list"],
            vec!["sai-paper", "get-pdf", "1"],
            vec!["sai-paper", "extract", "1"],
            vec!["sai-paper", "translate-pdf", "1"],
            vec!["sai-paper", "figures", "1"],
            vec!["sai-paper", "graph-show", "1"],
            vec!["sai-paper", "init", "--sci", "--type", "problem"],
            vec!["sai-paper", "init", "--name", "x", "--assets-only"],
            vec!["sai-paper", "search", "EEG", "--node", "O5"],
            vec!["sai-paper", "find", "EEG", "--node", "O5"],
            vec!["sai-paper", "find", "EEG", "--subtree"],
            vec!["sai-paper", "set", "list"],
            vec!["sai-paper", "search", "EEG", "--save-as", "x"],
            vec!["sai-paper", "expand", "1", "--save-as", "x"],
            vec!["sai-paper", "search-log", "1"],
            vec!["sai-paper", "find-log", "1"],
        ] {
            assert!(Cli::try_parse_from(&args).is_err(), "通ってしまう: {args:?}");
        }
    }

    /// U19: 残す隠した命令（`relayout`・`eval rag`）は今までどおり解釈できる。
    #[test]
    fn kept_hidden_commands_still_parse() {
        use clap::Parser as _;
        for args in [
            vec!["sai-paper", "relayout", "--json"],
            vec!["sai-paper", "eval", "rag"],
        ] {
            assert!(Cli::try_parse_from(&args).is_ok(), "解釈できない: {args:?}");
        }
    }

    /// U19: `init` は `--update` だけを受ける。
    #[test]
    fn init_accepts_only_update() {
        use clap::Parser as _;
        let cli = Cli::try_parse_from(["sai-paper", "init", "--update"]).expect("init --update は通る");
        assert!(matches!(cli.cmd, Some(Cmd::Init { update: true })));
    }
}
