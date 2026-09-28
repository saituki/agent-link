//! `papr` executable entry point.

mod state;
pub use state::*;
mod theme;
pub use theme::*;
mod block_picker;
mod citation;
mod editor;
mod figure_view;
mod md_view;
mod pdf_viewer;
mod terminal;
mod terminal_input;
mod tree_view;
mod ui;

use std::{
    collections::{HashMap, HashSet},
    ffi::OsString,
    path::{Path, PathBuf},
    process::{Command as ProcessCommand, Stdio},
    sync::Arc,
    sync::mpsc as std_mpsc,
};

use anyhow::{Context, Result};
use chrono::{Duration, Local, NaiveDate};
use clap::{CommandFactory, Parser, Subcommand};
use clap_complete::{Shell, generate};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use editor::{next_char_boundary, next_word_boundary, prev_char_boundary, prev_word_boundary};
use notify::{Config as NotifyConfig, RecommendedWatcher, RecursiveMode, Watcher};
use papr_core::{
    ArxivClient, CollectionDirectory, Config, DashboardService, Database, DownloadEvent,
    DownloadManager, ImportedPdf, LibraryIndexer, LibraryIngestionService, LibraryWatcher,
    MetadataCandidate, MetadataEnrichmentOutcome, MetadataEnrichmentService, PaperRoots, Paths,
    PluginHost, RemotePaper, S2Client, canonicalize_path, dedupe_remote_papers, move_pdf_file,
    paper_file_names, paper_folder, rank_remote_papers_by_query,
};
use terminal_input::{
    parse_command, sanitize_terminal_output, sort_terminal_candidates, terminal_home_directory,
    terminal_path_candidates,
};

use tokio::{
    sync::{Semaphore, mpsc},
    task::JoinSet,
};

use terminal::TerminalSession;

const DASHBOARD_CANDIDATE_LIMIT: u16 = 30;
const DASHBOARD_DISPLAY_LIMIT: usize = 10;
const DASHBOARD_REPEAT_EXCLUSION_DAYS: i64 = 7;
const MAX_TERMINAL_SCROLLBACK_BYTES: usize = 64 * 1024;
const DASHBOARD_FEED_ALGORITHM_VERSION: &str = "balanced-v3";
const METADATA_ENRICHMENT_CONCURRENCY: usize = 1;

#[derive(Debug, Parser)]
#[command(name = "sai-moon-paper-tui", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Option<CliCommand>,
}

#[derive(Debug, Subcommand)]
enum CliCommand {
    /// Print resolved configuration and data paths.
    Paths,
    /// Scan configured library folders and update the catalog.
    Index,
    /// Generate completion definitions for a supported shell.
    Completions {
        /// Shell syntax to generate.
        shell: Shell,
    },
    /// List discovered plugins and validation diagnostics.
    Plugins,
    /// Invoke an enabled plugin event using the JSON protocol.
    Plugin {
        /// Enabled plugin identifier.
        id: String,
        /// Event or command name.
        event: String,
        /// Execution deadline in seconds.
        #[arg(long, default_value_t = 10)]
        timeout: u64,
    },
}

fn main() -> Result<()> {
    // プロジェクトルートの .env を読み込む（S2 API キーのみ使用。既存の環境変数は上書きしない）
    papr_core::paths::load_project_dotenv();
    let cli = Cli::parse();
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("failed to initialize the Papr async runtime")?
        .block_on(run_app(cli))
}

async fn run_app(cli: Cli) -> Result<()> {
    if let Some(CliCommand::Completions { shell }) = &cli.command {
        generate(
            *shell,
            &mut Cli::command(),
            "sai-moon-paper-tui",
            &mut std::io::stdout(),
        );
        return Ok(());
    }
    let paths = Paths::discover().context("failed to resolve papr directories")?;
    if matches!(&cli.command, Some(CliCommand::Paths)) {
        print_application_paths(&paths);
        return Ok(());
    }

    let config = Config::load_or_create(&paths).context("failed to load configuration")?;
    let plugin_host = PluginHost::discover(&paths.plugins_dir, &config.library.enabled_plugins)
        .context("failed to discover plugins")?;
    if handle_plugin_cli(cli.command.as_ref(), &plugin_host).await? {
        return Ok(());
    }
    let theme = Theme::load(&config.ui.theme).context("failed to load theme")?;
    let database = Database::open(&paths.database_file).context("failed to open database")?;
    // 重い処理の記録（jobs）: この窓口は TUI。落ちて running のまま残った行は「中断」にする。
    papr_core::jobs::set_origin("tui");
    let _ = papr_core::jobs::cleanup_stale(database.connection(), papr_core::jobs::STALE_HOURS);
    let arxiv = ArxivClient::new().context("failed to initialize arXiv client")?;
    let s2 =
        S2Client::new(resolve_s2_api_key(&config)).context("failed to initialize S2 client")?;
    let (today_sender, today_receiver) = mpsc::unbounded_channel();
    let dashboard_startup = prepare_dashboard_feed(&config, &database, &arxiv, &today_sender)?;
    let locations = LibraryLocations::resolve(&config, &paths)?;
    let (mut dashboard, library_papers) = load_initial_dashboard(&database, &paths, &locations)?;
    // RECENT ACTIVITY に Radio の出来事（聴いた・質問した）を混ぜる
    papr_core::podcast_client::merge_radio_activity(
        &mut dashboard.recent_activity,
        &config.radio,
        database.connection(),
    );

    let mut app = initial_app(&config, dashboard);
    app.plugins = plugin_host.plugins();
    app.plugin_diagnostics = plugin_host.diagnostics().len();
    if let Some(papers) = dashboard_startup.cached_papers {
        app.today_papers = papers;
        app.today_status = DiscoveryStatus::Ready;
    } else {
        app.today_status = DiscoveryStatus::Loading;
    }

    app.library.papers = library_papers;
    refresh_organization(&database, &locations.library_roots, &mut app)?;
    let (watch_sender, watch_receiver) = mpsc::unbounded_channel();
    let watcher = start_library_watcher(&locations.library_roots, watch_sender.clone())?;
    let config_filesystem_watcher = ConfigFilesystemWatcher::start(&paths.config_file)
        .context("failed to watch configuration file")?;

    let downloads = DownloadManager::new().context("failed to initialize download manager")?;
    let mut session = TerminalSession::start()?;
    // 図画像のプロトコル判定を先に済ませる（初回描画でのターミナル問い合わせを避ける）。
    figure_view::init();
    let runtime = Runtime {
        metadata_enrichment: MetadataEnrichmentService::new(arxiv.clone()),
        arxiv: arxiv.clone(),
        s2: s2.clone(),
        downloads,
        database,
        database_file: paths.database_file.clone(),
        config_file: paths.config_file.clone(),
        config: config.clone(),
        config_filesystem_watcher,
        config_reload_deadline: None,
        config_reload_attempts: 0,
        plugins_dir: paths.plugins_dir.clone(),
        plugin_host,
        default_downloads_dir: paths.downloads_dir.clone(),
        download_dir: locations.download_dir,
        pdf_viewer: config.ui.pdf_viewer.clone().unwrap_or_else(default_pdf_viewer),
        // 論文フォルダの置き場（`.sai/paper/`）。paths::paper_folder に渡す。
        data_dir: paths.data_dir(),
        library_roots: locations.library_roots,
        collection_roots: locations.collection_roots,
        dashboard_keywords: dashboard_startup.keywords,
        dashboard_keyword_signature: dashboard_startup.keyword_signature,
        dashboard_feed_date: dashboard_startup.feed_date,
        active_dashboard_fetch: dashboard_startup.active_fetch,
        watch_sender,
        watch_receiver,
        watcher,
        active_enrichments: std::collections::HashSet::new(),
    };
    run(
        &mut session,
        &mut app,
        theme,
        runtime,
        today_sender,
        today_receiver,
    )
    .await
}

fn initial_app(config: &Config, dashboard: papr_core::ResearchDashboard) -> App {
    let page = Page::from_config_str(&config.ui.startup_page).unwrap_or(Page::Dashboard);
    let mut app = App {
        page,
        data_dir: papr_core::Paths::discover()
            .map(|paths| paths.data_dir())
            .unwrap_or_default(),
        sidebar_index: Page::ALL
            .iter()
            .position(|&candidate| candidate == page)
            .unwrap_or(0),
        stats: dashboard.counts,
        dashboard,
        ..App::default()
    };
    app.pdf_viewer = config.ui.pdf_viewer.clone().unwrap_or_else(default_pdf_viewer);
    // 起動時から研究方針のページなら、木を最初の 1 回だけ読み直す。
    if app.page == Page::Tree {
        app.tree.stale = true;
    }
    app
}

struct DashboardStartup {
    keywords: Vec<String>,
    keyword_signature: String,
    feed_date: String,
    cached_papers: Option<Vec<RemotePaper>>,
    active_fetch: Option<DashboardFeedKey>,
}

fn prepare_dashboard_feed(
    config: &Config,
    database: &Database,
    arxiv: &ArxivClient,
    sender: &mpsc::UnboundedSender<TodayResponse>,
) -> Result<DashboardStartup> {
    let keywords = config.dashboard_keyword_list();
    let keyword_signature = dashboard_keyword_signature(&keywords);
    let feed_date = local_feed_date();
    let cached_papers = database.dashboard_feed_cache(&feed_date, &keyword_signature)?;
    let active_fetch = if cached_papers.is_none() {
        let key = DashboardFeedKey {
            feed_date: feed_date.clone(),
            keyword_signature: keyword_signature.clone(),
        };
        start_dashboard_fetch(
            arxiv.clone(),
            keywords.clone(),
            dashboard_recent_paper_ids(database, &feed_date)?,
            key.clone(),
            sender.clone(),
        );
        Some(key)
    } else {
        None
    };
    Ok(DashboardStartup {
        keywords,
        keyword_signature,
        feed_date,
        cached_papers,
        active_fetch,
    })
}

fn print_application_paths(paths: &Paths) {
    println!("config: {}", paths.config_file.display());
    println!("database: {}", paths.database_file.display());
    println!("downloads: {}", paths.downloads_dir.display());
    println!("plugins: {}", paths.plugins_dir.display());
}

struct LibraryLocations {
    download_dir: PathBuf,
    collection_roots: Vec<PathBuf>,
    library_roots: Vec<PathBuf>,
}

impl LibraryLocations {
    fn resolve(config: &Config, paths: &Paths) -> Result<Self> {
        let download_dir = config
            .library.download_path
            .clone()
            .unwrap_or_else(|| paths.downloads_dir.clone());
        std::fs::create_dir_all(&download_dir).context("failed to create download directory")?;
        let download_dir = std::fs::canonicalize(&download_dir).unwrap_or(download_dir);
        let collection_roots = config
            .library.import_folders
            .iter()
            .map(|root| std::fs::canonicalize(root).unwrap_or_else(|_| root.clone()))
            .collect::<Vec<_>>();
        // Library 対象フォルダの決定は core に共通化（フォルダ作成はここが担う）。
        let library_roots = papr_core::library_tree::library_roots(config, paths);
        Ok(Self {
            download_dir,
            collection_roots,
            library_roots,
        })
    }
}

fn load_initial_dashboard(
    database: &Database,
    paths: &Paths,
    locations: &LibraryLocations,
) -> Result<(papr_core::ResearchDashboard, Vec<papr_core::LibraryPaper>)> {
    let mut dashboard = database
        .research_dashboard()
        .context("failed to load research dashboard")?;
    let (paper_count, disk_usage) = LibraryIndexer::pdf_storage_stats(&locations.collection_roots);
    let (downloaded, downloads_size) =
        LibraryIndexer::pdf_storage_stats(std::slice::from_ref(&locations.download_dir));
    let papers = database.library_papers_in_roots(&locations.library_roots)?;
    dashboard.counts.papers = paper_count;
    dashboard.counts.downloaded = downloaded;
    dashboard.read = papers
        .iter()
        .filter(|paper| paper.reading_status == "read")
        .count() as u64;
    dashboard.disk_usage = disk_usage;
    dashboard.downloads_size = downloads_size;
    dashboard.database_size = std::fs::metadata(&paths.database_file).map_or(0, |m| m.len());
    Ok((dashboard, papers))
}

async fn handle_plugin_cli(command: Option<&CliCommand>, plugin_host: &PluginHost) -> Result<bool> {
    if matches!(command, Some(CliCommand::Plugins)) {
        for plugin in plugin_host.plugins() {
            println!(
                "{}\t{}\t{}\t{}",
                plugin.id,
                plugin.version,
                if plugin.enabled {
                    "enabled"
                } else {
                    "disabled"
                },
                plugin.name
            );
        }
        for diagnostic in plugin_host.diagnostics() {
            eprintln!(
                "invalid\t{}\t{}",
                diagnostic.path.display(),
                diagnostic.message
            );
        }
        return Ok(true);
    }
    if let Some(CliCommand::Plugin { id, event, timeout }) = command {
        let response = plugin_host
            .invoke(
                id,
                &papr_core::PluginRequest::new(event, serde_json::json!({})),
                std::time::Duration::from_secs(*timeout),
            )
            .await
            .context("plugin invocation failed")?;
        println!("{}", serde_json::to_string_pretty(&response)?);
        return Ok(true);
    }
    Ok(false)
}

/// Project ページから nvim で開くファイルの種類。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProjectFile {
    /// `mind.md`（ユーザーの考え）。
    Mind,
    /// `state.md`（プロジェクトの現況）。
    State,
}

enum UiAction {
    /// `M` / `W`: mind.md / state.md を nvim で開く（無ければひな形を作る）。
    OpenProjectFile(ProjectFile),
    /// `J`: 最近 7 日の研究日誌を内蔵 Markdown ビューアで開く（S01）。
    OpenJournal,
    /// S02: 要対応の一覧の `Enter`: 選んだ 1 件の元を開く。
    OpenAttention(papr_core::attention::AttentionItem),
    /// S02: 要対応の一覧の `x`: 選んだ 1 件を「対応済み」にする。
    ResolveAttention(papr_core::attention::AttentionItem),
    Search(String),
    RetryDiscoverMore,
    /// Import 画面: `data/bibtex/` を再スキャンする（入場時にも自動実行）。
    ScanBibtex,
    OpenPaper(RemotePaper),
    OpenBrowser(String),
    Download(RemotePaper),
    Reindex,
    OpenPdf {
        paper_id: i64,
        path: PathBuf,
        /// 開いた直後に表示するページ（1 始まり。`None` = 先頭）。
        page: Option<usize>,
    },
    /// md の表示画面の本文で `Enter` を押した: 引用（`[#37 §3.2]` など）の根拠を開く。
    OpenCitation {
        paper_id: i64,
        locator: Option<String>,
    },
    OpenNote(PaperTarget),
    Prompt(PaperTarget),
    SubmitPrompt(MetadataPrompt),
    Bookmark(PaperTarget),
    OpenAuthor(i64),
    OpenLibraryPaper(i64),
    RenamePdf(i64),
    /// タグ付与モーダルを開く（R1/R2）。
    OpenTagModal(i64),
    /// タグ一覧で Enter: 選択中タグの論文一覧を開く（R2）。
    OpenTagPaperList(String),
    MarkUnread(i64),
    CopyCitation(PaperTarget),
    /// PDF 抽出（ensure_extract）を実行してから sai-paper-tui を起動する。
    CitationSidecar {
        paper_id: i64,
    },
    /// Discover / Dashboard 論文の引用サイドカー起動（`s` キー）。
    /// PDF 未所持なら自動 DL（失敗時はエラー表示で終了）→ ensure_extract → tui 起動。
    CitationSidecarRemote {
        paper: RemotePaper,
    },
    /// 手動 PDF 取り込みモーダルを開く（`i` キー）。
    OpenManualImport(ManualImportPaper),
    ConfirmManualImport,
    /// Library 論文の abstract を日本語へ翻訳（`a` / `A` キー）。
    TranslateAbstract {
        paper_id: i64,
        force: bool,
    },
    /// Remote 論文の abstract を日本語へ翻訳（Remote PAPER 詳細の `a` / `A` キー）。
    /// `ensure_remote_paper` で DB 行を解決してから Library 論文と同じ翻訳フローへ流用する。
    TranslateAbstractRemote {
        paper: RemotePaper,
        force: bool,
    },
    /// Library 論文の PDF を日本語へ翻訳（`T` キー、pdf2zh）。
    TranslatePdf {
        paper_id: i64,
    },
    /// DL 失敗時の再試行。
    /// PDF ビューアを閉じる。
    ClosePdf,
    /// 論文削除確認。
    ConfirmDeletePaper {
        paper_id: i64,
        title: String,
        path: Option<String>,
    },
    DeletePaper {
        paper_id: i64,
        path: Option<String>,
    },
    /// 検索セットの候補一覧を読み込む。
    LoadSearchSetItems(i64),
    /// 検索セットの候補の判定を更新する。
    MarkSearchSetItem {
        set_id: i64,
        position: i64,
        decision: String,
    },
    /// S03: 検索セットをもう一度実行する（Search Sets 画面の `r`）。
    RerunSearchSet(i64),
    /// handoff モーダルを開く（S キー）。
    OpenHandoff(i64),
    /// RAG 検索を実行する（Enter キー）。
    ExecuteRagSearch,
    /// RAG 検索結果の Markdown ファイルを開く（m キー）。
    OpenRagSearchMarkdown(String),
    /// PAPER 画面の REPORTS 欄: 数字キー `1`〜`5` で report を内蔵ビューアで開く。
    OpenReport(String),
    /// 内蔵ビューアで report を表示中の `C`: 外部の引用を公開 API で照合する。
    CheckReport(String),
    /// 内蔵 Markdown ビューアで note.md / extract.md を開く（`space+n` / `space+m`）。
    OpenMarkdownViewer {
        kind: MdKind,
        paper_id: Option<i64>,
        title: String,
        extract_key: Option<String>,
    },
    /// note.md / extract.md を nvim で開く（単独 `n` / `m`）。
    OpenMarkdownInNvim {
        kind: MdKind,
        paper_id: Option<i64>,
        title: String,
        extract_key: Option<String>,
    },
    /// Library 階層表示の query 行から QUERY 画面を開く。
    OpenQueryDetail(i64),
    /// U23: 研究方針の `tree.md` を内蔵の Markdown ビューアで開く（`o`）。
    OpenTreeMd,
    /// Library 論文のポッドキャスト生成（`r` キー、sai-radio）。
    GeneratePodcast {
        paper_id: i64,
    },
    /// QUERY 画面の Reason を DB に保存する。
    SaveQueryReason,
    /// N08: `N` / `U` で sai-link の todo に「次の一手」「理解の確認」カードを作る（実行はしない）。
    CreateAgentCard(AgentCard),
}

/// N08: `N` / `U` で作る「エージェントへの依頼カード」の種類。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AgentCard {
    /// `N`: 次の一手（ブロック `paper-next`）。
    Next,
    /// `U`: 理解の確認（ブロック `paper-check`）。
    Check,
}

impl AgentCard {
    /// sai-link のブロック ID。
    fn block_id(self) -> &'static str {
        match self {
            Self::Next => "paper-next",
            Self::Check => "paper-check",
        }
    }

    /// タグとトーストに使う短い名前。
    fn label(self) -> &'static str {
        match self {
            Self::Next => "次の一手",
            Self::Check => "理解の確認",
        }
    }

    /// 依頼文（`# task_text` に展開される本文）。
    fn description(self) -> &'static str {
        match self {
            Self::Next => "次の一手を提案してください。",
            Self::Check => "最近の議論について、私の理解を確かめてください。",
        }
    }
}

enum KeyHandling {
    Ignored,
    Handled(Option<Box<UiAction>>),
}

/// 内蔵ビューア / nvim で開く Markdown の種別。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MdKind {
    /// 論文ごとの note.md。
    Note,
    /// docling が出力した extract.md。
    Extract,
}

/// 内蔵ビューア / nvim が開く対象（論文）。
#[derive(Debug, Clone)]
struct MdTarget {
    /// ローカル論文 ID（note.md の解決に使う。未ダウンロードなら None）。
    paper_id: Option<i64>,
    /// 論文タイトル（フォルダ名の解決に使う）。
    title: String,
    /// 抽出キー（extract.md の解決に使う。arXiv / DOI が無ければ None）。
    extract_key: Option<String>,
}

#[derive(Debug)]
enum PaperTarget {
    Local(i64),
    Remote(Box<RemotePaper>),
}

#[derive(Debug)]
struct SearchResponse {
    query: String,
    request_id: u64,
    update: SearchUpdate,
}

#[derive(Debug)]
enum SearchUpdate {
    Partial {
        papers: Vec<RemotePaper>,
        next_start: Option<u16>,
    },
    Retrying {
        attempt: u8,
        max_attempts: u8,
    },
    Complete(Vec<RemotePaper>),
    InitialFailure(String),
    PartialFailure {
        next_start: u16,
    },
}

const DISCOVERY_CANDIDATE_LIMIT: u16 = 250;
const DISCOVERY_FETCH_BATCH_SIZE: u16 = 100;
const DISCOVERY_PAGE_RETRY_ATTEMPTS: u8 = 3;
const DISCOVERY_PAGE_RETRY_BASE_DELAY: std::time::Duration = std::time::Duration::from_millis(500);

#[derive(Debug)]
struct TodayResponse {
    key: DashboardFeedKey,
    result: Result<Vec<RemotePaper>, String>,
}

/// Identifies one daily feed request.  Responses are accepted only when both
/// the local date and the configured feed algorithm/keywords still match.
#[derive(Debug, Clone, PartialEq, Eq)]
struct DashboardFeedKey {
    feed_date: String,
    keyword_signature: String,
}

fn resolve_s2_api_key(config: &Config) -> Option<String> {
    if let Some(key) = config.s2_api_key() {
        return Some(key);
    }
    // Fallback: .env の手動パース（dotenvy が読めない環境向けの保険）。
    // ルート直下と、旧 Web レイアウ相当の親ディレクトリを試す。
    let mut env_candidates: Vec<PathBuf> = Vec::new();
    if let Some(root) = papr_core::paths::project_root() {
        env_candidates.push(root.join(".env"));
        if let Some(par) = root.parent() {
            env_candidates.push(par.join(".env"));
        }
    }
    for path in env_candidates {
        if let Ok(txt) = std::fs::read_to_string(path) {
            for line in txt.lines() {
                let t = line.trim();
                if t.starts_with("SEMANTICSCHOLAR_API_KEY")
                    || t.starts_with("SCHOLAR_API_KEY")
                    || t.starts_with("S2_API_KEY")
                {
                    if let Some(v) = t.split('=').nth(1) {
                        let v = v
                            .trim()
                            .trim_matches('"')
                            .trim_matches('\'')
                            .trim()
                            .to_string();
                        if !v.is_empty() {
                            return Some(v);
                        }
                    }
                }
            }
        }
    }
    None
}

fn discovery_providers(config: &Config) -> Vec<String> {
    let mut providers = config
        .discover
        .providers
        .iter()
        .map(|p| p.trim().to_lowercase())
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>();
    if providers.is_empty() {
        providers = vec!["arxiv".into(), "s2".into()];
    }
    providers
}

/// TUI の Discover 画面の DB 入力欄から providers を決定する。
///
/// 空欄の場合は設定を使用、カンマ区切りで複数指定可能。
/// 無効な DB 名が含まれる場合は、有効な名前のみ使用し、
/// エラーメッセージを返す。
fn get_discovery_providers_from_input(
    db_input: &str,
    config: &Config,
) -> (Vec<String>, Option<String>) {
    let db_input = db_input.trim();

    // 空欄の場合は設定を使用
    if db_input.is_empty() {
        return (discovery_providers(config), None);
    }

    // カンマ区切りで分割して検証
    let valid_names = ["pubmed", "biorxiv", "medrxiv", "s2", "arxiv", "openalex"];
    let mut providers = Vec::new();
    let mut invalid_names = Vec::new();

    for name in db_input.split(',') {
        let name_trimmed = name.trim().to_lowercase();
        if name_trimmed.is_empty() {
            continue;
        }
        if valid_names.contains(&name_trimmed.as_str()) {
            providers.push(name_trimmed);
        } else {
            invalid_names.push(name_trimmed);
        }
    }

    // 有効な provider が 1 つもない場合、設定を使用して警告
    if providers.is_empty() {
        let msg = format!("使えるのは pubmed / biorxiv / medrxiv / s2 / arxiv / openalex です。");
        return (discovery_providers(config), Some(msg));
    }

    // 無効な名前がある場合は警告
    let err_msg = if !invalid_names.is_empty() {
        Some(format!(
            "不明なDB: {}。使えるのは pubmed / biorxiv / medrxiv / s2 / arxiv / openalex です。",
            invalid_names.join(", ")
        ))
    } else {
        None
    };

    (providers, err_msg)
}

struct Runtime {
    arxiv: ArxivClient,
    s2: S2Client,
    metadata_enrichment: MetadataEnrichmentService,
    downloads: DownloadManager,
    database: Database,
    database_file: PathBuf,
    config_file: PathBuf,
    config: Config,
    config_filesystem_watcher: ConfigFilesystemWatcher,
    config_reload_deadline: Option<std::time::Instant>,
    config_reload_attempts: u8,
    plugins_dir: PathBuf,
    plugin_host: PluginHost,
    default_downloads_dir: PathBuf,
    download_dir: PathBuf,
    pdf_viewer: String,
    /// データディレクトリ（`.sai/paper/`）。論文フォルダはここで一元に決める。
    data_dir: PathBuf,
    library_roots: Vec<PathBuf>,
    collection_roots: Vec<PathBuf>,
    dashboard_keywords: Vec<String>,
    dashboard_keyword_signature: String,
    dashboard_feed_date: String,
    active_dashboard_fetch: Option<DashboardFeedKey>,
    watch_sender: mpsc::UnboundedSender<()>,
    watch_receiver: mpsc::UnboundedReceiver<()>,
    watcher: LibraryWatcher,
    active_enrichments: std::collections::HashSet<i64>,
}

/// Watches the configuration's parent directory, rather than just the file,
/// so editor atomic-save strategies (write temporary file, then rename) are
/// observed on every supported platform.
struct ConfigFilesystemWatcher {
    events: std_mpsc::Receiver<notify::Event>,
    _watcher: RecommendedWatcher,
}

impl ConfigFilesystemWatcher {
    fn start(config_file: &Path) -> Result<Self> {
        let (sender, events) = std_mpsc::channel();
        let watch_root = config_file
            .parent()
            .ok_or_else(|| anyhow::anyhow!("configuration file has no parent directory"))?
            .to_path_buf();
        let mut watcher = RecommendedWatcher::new(
            move |event: notify::Result<notify::Event>| {
                if let Ok(event) = event
                    && !matches!(event.kind, notify::EventKind::Access(_))
                {
                    // A non-recursive directory watch is intentional. Inotify
                    // reports an atomic rename as a directory event on some
                    // systems, so filtering for an exact `settings.json` path drops
                    // the very saves we need to handle. The reload path compares
                    // the parsed configuration before rebuilding any subsystem.
                    let _ = sender.send(event);
                }
            },
            NotifyConfig::default(),
        )?;
        watcher.watch(&watch_root, RecursiveMode::NonRecursive)?;
        Ok(Self {
            events,
            _watcher: watcher,
        })
    }

    fn has_changes(&self) -> bool {
        let mut changed = false;
        while self.events.try_recv().is_ok() {
            changed = true;
        }
        changed
    }
}

struct ActionSenders {
    search: mpsc::UnboundedSender<SearchResponse>,
    index: mpsc::UnboundedSender<IndexResponse>,
    download: mpsc::UnboundedSender<DownloadEvent>,
    today: mpsc::UnboundedSender<TodayResponse>,
    app_events: mpsc::UnboundedSender<AppEvent>,
    enrichment: mpsc::UnboundedSender<MetadataEnrichment>,
}

#[derive(Debug)]
struct MetadataEnrichment {
    paper_id: i64,
    /// 補完に使った DOI（撤回の状態を書く先を見つけるため）。
    doi: Option<String>,
    /// OpenAlex で分かった撤回の状態（`enrich_with_status` の戻り値）。
    retraction: Option<bool>,
    outcome: MetadataEnrichmentOutcome,
}

#[derive(Debug)]
enum AppEvent {
    ReadingSessionCompleted {
        session_id: i64,
        duration_s: u64,
    },
    Toast(String),
    RefreshLibrary,
    /// 長時間処理（`s` / `T`）が完了した。実行中表示を消すためのキー。
    OperationFinished(String),
    /// 長時間処理が次のステップへ進んだ（TASKS 画面の進捗表示用）。
    OperationStep {
        /// `RunningOperation::key`。
        key: String,
        /// 新しいステップ番号（0 始まり = 完了済みステップ数）。
        step: usize,
    },
    /// S03: 検索セットの再実行（`r`）が終わった。
    SearchSetRerun {
        /// 実行したセットの ID。
        set_id: i64,
        /// 結果（成功なら [`papr_core::search_sets::RerunResult`]）。
        result: std::result::Result<papr_core::search_sets::RerunResult, String>,
    },
}

#[derive(Debug)]
enum IndexResponse {
    Scan {
        pdfs: Vec<ImportedPdf>,
        directories: Vec<CollectionDirectory>,
    },
}

async fn run(
    session: &mut TerminalSession,
    app: &mut App,
    mut theme: Theme,
    mut runtime: Runtime,
    today_sender: mpsc::UnboundedSender<TodayResponse>,
    mut today_receiver: mpsc::UnboundedReceiver<TodayResponse>,
) -> Result<()> {
    let (sender, mut receiver) = mpsc::unbounded_channel::<SearchResponse>();
    let (index_sender, mut index_receiver) = mpsc::unbounded_channel::<IndexResponse>();
    let (download_sender, mut download_receiver) = mpsc::unbounded_channel::<DownloadEvent>();
    let (app_events_sender, mut app_events_receiver) = mpsc::unbounded_channel::<AppEvent>();
    let (enrichment_sender, mut enrichment_receiver) =
        mpsc::unbounded_channel::<MetadataEnrichment>();
    let senders = ActionSenders {
        search: sender,
        index: index_sender,
        download: download_sender,
        today: today_sender,
        app_events: app_events_sender,
        enrichment: enrichment_sender,
    };
    let mut pending_downloads = HashMap::<String, RemotePaper>::new();
    start_runtime_scan(&runtime, &senders, app);
    let mut last_date_check = std::time::Instant::now();
    let mut last_enrichment_check = std::time::Instant::now();
    let mut last_page: Option<Page> = None;
    let mut last_toast = None;
    let mut force_redraw = true;

    while !app.session_flags.should_quit {
        // state_changed drives non-PDF redraws; force_redraw (for PDF/animation)
        // is consumed and reset at the bottom of the loop after drawing.
        let mut state_changed = force_redraw;

        match process_config_reload(
            session,
            &mut runtime,
            app,
            &mut theme,
            &senders,
            &mut force_redraw,
        )? {
            ConfigReloadOutcome::Unchanged => {}
            ConfigReloadOutcome::Changed => state_changed = true,
            ConfigReloadOutcome::SkipIteration => continue,
        }

        state_changed |= process_dashboard_feed_updates(
            &mut today_receiver,
            &mut runtime,
            &senders,
            app,
            &mut last_date_check,
        )?;
        if last_enrichment_check.elapsed() >= std::time::Duration::from_mins(5) {
            last_enrichment_check = std::time::Instant::now();
            spawn_enrichment_if_needed(&mut runtime, &senders, app)?;
        }
        state_changed |= process_search_updates(&mut receiver, &runtime, app);
        state_changed |=
            process_index_updates(&mut index_receiver, &mut runtime, &senders, app).await?;
        state_changed |= process_enrichment_updates(&mut enrichment_receiver, &mut runtime, app)?;
        state_changed |= process_download_updates(
            &mut download_receiver,
            &mut pending_downloads,
            &mut runtime,
            &senders,
            app,
        )
        .await?;
        state_changed |= process_app_events(&mut app_events_receiver, &runtime, app)?;
        state_changed |= process_page_change(&runtime, app, &mut theme, &mut last_page)?;
        state_changed |= rebuild_library_tree_if_dirty(&runtime, app);
        state_changed |= update_toast_and_download_cleanup(app, &mut last_toast);
        // TASKS 画面は経過時間を秒単位で出すため、開いている間は毎ループ描き直す
        // （ポーリング間隔は 100ms。専用タイマスレッドは不要）。
        state_changed |= app.mode == AppMode::Tasks;
        if app.mode == AppMode::Tasks
            && app
                .jobs_loaded_at
                .is_none_or(|at| at.elapsed() >= std::time::Duration::from_secs(1))
        {
            app.jobs = papr_core::jobs::recent(runtime.database.connection(), 12).unwrap_or_default();
            app.jobs_loaded_at = Some(std::time::Instant::now());
        }

        // ── FIXED EVENT ORDERING ─────────────────────────────────────────────
        // Read all pending events FIRST, THEN draw.  Previously the loop drew
        // state before reading keys, adding one full iteration of input latency.
        // Now: drain pending events → block-wait → draw updated state.

        let preview_active = app.mode == AppMode::PdfView;
        let animating = preview_active && pdf_viewer::is_animating();
        let poll_timeout = terminal_poll_timeout(app, preview_active, animating);
        let got_event = poll_terminal_events(
            app,
            &mut runtime,
            &senders,
            &mut pending_downloads,
            &mut theme,
            poll_timeout,
        )
        .await?;

        if got_event || animating {
            force_redraw = true;
        }
        // Step 3: draw with the fully-updated state (key effects are visible
        // in THIS iteration, not the next one).
        if state_changed || force_redraw {
            force_redraw = false; // consumed here
            draw_application(session, app, &theme, &runtime.database_file)?;
        }

        tokio::task::yield_now().await;
    }
    shutdown_runtime(&mut runtime);
    Ok(())
}

fn shutdown_runtime(_runtime: &mut Runtime) {
    pdf_viewer::cleanup_temp_files();
}

fn draw_application(
    session: &mut TerminalSession,
    app: &mut App,
    theme: &Theme,
    database_file: &Path,
) -> Result<()> {
    session.set_command_cursor_active(app.mode == AppMode::TerminalCommand)?;
    let draw_start = std::time::Instant::now();
    session
        .terminal_mut()
        .draw(|frame| ui::render(frame, app, theme))?;
    if draw_start.elapsed() > std::time::Duration::from_millis(16) {
        log_message(
            database_file,
            &format!("Slow draw: {:?}", draw_start.elapsed()),
        );
    }
    Ok(())
}

enum ConfigReloadOutcome {
    Unchanged,
    Changed,
    SkipIteration,
}

fn process_config_reload(
    session: &mut TerminalSession,
    runtime: &mut Runtime,
    app: &mut App,
    theme: &mut Theme,
    senders: &ActionSenders,
    force_redraw: &mut bool,
) -> Result<ConfigReloadOutcome> {
    if runtime.config_filesystem_watcher.has_changes() {
        runtime.config_reload_deadline =
            Some(std::time::Instant::now() + std::time::Duration::from_millis(150));
        runtime.config_reload_attempts = 0;
    }
    let Some(deadline) = runtime.config_reload_deadline else {
        return Ok(ConfigReloadOutcome::Unchanged);
    };
    if std::time::Instant::now() < deadline {
        return Ok(ConfigReloadOutcome::Unchanged);
    }
    match Config::load(&runtime.config_file) {
        Ok(config) => {
            runtime.config_reload_deadline = None;
            runtime.config_reload_attempts = 0;
            if config == runtime.config {
                return Ok(ConfigReloadOutcome::SkipIteration);
            }
            apply_config_update(runtime, app, &config, theme, senders)?;
            app.toast = Some("Configuration reloaded.".to_owned());
            session.terminal_mut().clear()?;
            *force_redraw = true;
            Ok(ConfigReloadOutcome::Changed)
        }
        Err(_) if runtime.config_reload_attempts < 10 => {
            runtime.config_reload_attempts += 1;
            runtime.config_reload_deadline =
                Some(std::time::Instant::now() + std::time::Duration::from_millis(100));
            Ok(ConfigReloadOutcome::Unchanged)
        }
        Err(error) => {
            runtime.config_reload_deadline = None;
            app.toast = Some(format!("Configuration reload failed: {error}"));
            Ok(ConfigReloadOutcome::Changed)
        }
    }
}

fn process_dashboard_feed_updates(
    receiver: &mut mpsc::UnboundedReceiver<TodayResponse>,
    runtime: &mut Runtime,
    senders: &ActionSenders,
    app: &mut App,
    last_date_check: &mut std::time::Instant,
) -> Result<bool> {
    let mut changed = false;
    while let Ok(TodayResponse { key, result }) = receiver.try_recv() {
        changed = true;
        if key.feed_date != runtime.dashboard_feed_date
            || key.keyword_signature != runtime.dashboard_keyword_signature
        {
            continue;
        }
        runtime.active_dashboard_fetch = None;
        match result {
            Ok(papers) => {
                runtime.database.save_dashboard_feed_cache(
                    &key.feed_date,
                    &runtime.dashboard_keyword_signature,
                    &papers,
                )?;
                app.today_papers = papers;
                app.today_selected = app
                    .today_selected
                    .min(app.today_papers.len().saturating_sub(1));
                app.today_status = DiscoveryStatus::Ready;
            }
            Err(error) => app.today_status = DiscoveryStatus::Error(error),
        }
    }
    if last_date_check.elapsed() >= std::time::Duration::from_secs(1) {
        *last_date_check = std::time::Instant::now();
        let current_date = local_feed_date();
        if current_date != runtime.dashboard_feed_date {
            runtime.dashboard_feed_date = current_date;
            refresh_dashboard_papers(runtime, senders, app)?;
            changed = true;
        }
    }
    Ok(changed)
}

fn process_search_updates(
    receiver: &mut mpsc::UnboundedReceiver<SearchResponse>,
    runtime: &Runtime,
    app: &mut App,
) -> bool {
    let mut changed = false;
    while let Ok(response) = receiver.try_recv() {
        changed = true;
        if response.query != app.discovery.query || response.request_id != app.discovery.request_id
        {
            continue;
        }
        match response.update {
            SearchUpdate::Partial { papers, next_start } => {
                app.discovery.update_results(papers);
                app.discovery.next_batch_start = next_start;
                app.discovery.progress_message =
                    next_start.map(|_| "Loading more results...".to_owned());
                app.discovery.status = DiscoveryStatus::Loading;
                // 最初の途中結果で記録を作る（`Complete` が来なくても候補が残る）。
                // 以降は同じログへ追記するので、`Complete` での呼び出しは重複排除が効く。
                record_search_log(runtime, app);
            }
            SearchUpdate::Retrying {
                attempt,
                max_attempts,
            } => {
                app.discovery.progress_message = Some(format!(
                    "Retrying more results ({attempt}/{max_attempts})..."
                ));
                app.discovery.status = DiscoveryStatus::Loading;
            }
            SearchUpdate::Complete(papers) => {
                app.discovery.update_results(papers);
                app.discovery.next_batch_start = None;
                app.discovery.progress_message = None;
                app.discovery.status = DiscoveryStatus::Ready;
                record_search_log(runtime, app);
            }
            SearchUpdate::InitialFailure(error) => {
                app.discovery.status = DiscoveryStatus::Error(error);
            }
            SearchUpdate::PartialFailure { next_start } => {
                app.discovery.next_batch_start = Some(next_start);
                app.discovery.progress_message =
                    Some("More results could not be loaded. Press r to retry.".to_owned());
                app.discovery.status = DiscoveryStatus::Ready;
                record_search_log(runtime, app);
            }
        }
    }
    changed
}

/// Discover 検索の結果を検索実行ログへ保存する（Library 階層表示・QUERY 画面用）。
///
/// 検索完了時の 1 回目の結果でログを作成し、以降（追加読み込み）は同じログへ追記する。
fn record_search_log(runtime: &Runtime, app: &mut App) {
    let papers = app.discovery.results.clone();
    let conn = runtime.database.connection();
    let query = app.discovery.query.clone();
    let year = app.discovery.year.trim().to_owned();
    let limit = app.discovery.limit.trim().to_owned();
    let source_dbs = app.discovery.db.trim();
    // Database 欄が空なら、実際に使った DB 名を記録する。
    let source_dbs = if source_dbs.is_empty() {
        app.discovery.active_providers.join(",")
    } else {
        source_dbs.to_owned()
    };
    let result = match app.discovery.search_log_id {
        Some(log_id) => papr_core::search_log::append_items(conn, log_id, &papers).map(|()| log_id),
        None => {
            papr_core::search_log::record_search(
                conn, "discover", &query, &year, &limit, &source_dbs, &papers,
            )
        }
    };
    match result {
        Ok(log_id) => {
            app.discovery.search_log_id = Some(log_id);
            app.library_tree_dirty = true;
            // R2: 過去のヒット回数を集計する（今回の検索を含む）。
            let keys: Vec<String> = papers.iter().map(papr_core::dedupe_key).collect();
            if let Ok(counts) = papr_core::search_log::hit_counts_for_keys(conn, &keys) {
                app.discovery.hit_counts = counts;
            }
        }
        Err(error) => {
            app.toast = Some(format!("検索ログの保存に失敗: {error}"));
        }
    }
}

/// Discover のいまの検索の記録に、論文を紐付ける（Library の階層表示用）。
///
/// 記録に候補が無ければ、`app.discovery.results` に同じ論文がある場合だけ足してから紐付ける。
/// Dashboard から取り込んだ論文も、いまの検索の結果にあればその検索で見つかったものなので紐付く。
/// エラーは `app.toast` に出す（TUI の画面を崩す `eprintln!` は使わない）。
fn link_to_discover_log(
    conn: &rusqlite::Connection,
    app: &mut App,
    remote: &RemotePaper,
    paper_id: i64,
) {
    let Some(log_id) = app.discovery.search_log_id else {
        return;
    };
    if let Err(error) = link_to_discover_log_inner(conn, log_id, app, remote, paper_id) {
        app.toast = Some(format!("検索の記録への紐付けに失敗: {error}"));
    }
}

/// `link_to_discover_log` の本体（`app` を可変に持たずエラーを返す）。
fn link_to_discover_log_inner(
    conn: &rusqlite::Connection,
    log_id: i64,
    app: &App,
    remote: &RemotePaper,
    paper_id: i64,
) -> Result<(), String> {
    let key = papr_core::dedupe_key(remote);
    let mut position = search_log_item_position(conn, log_id, &key);
    if position.is_none()
        && app
            .discovery
            .results
            .iter()
            .any(|p| papr_core::dedupe_key(p) == key)
    {
        // 記録の候補に無いので、いまの検索の結果を 1 件足してから紐付ける。
        papr_core::search_log::append_items(conn, log_id, std::slice::from_ref(remote))
            .map_err(|e| e.to_string())?;
        position = search_log_item_position(conn, log_id, &key);
    }
    let Some(position) = position else {
        // いまの検索の結果に無い論文（別の検索の見 Candidates でもない）なので何もしない。
        return Ok(());
    };
    papr_core::search_sets::link_paper(conn, log_id, position, paper_id).map_err(|e| e.to_string())
}

/// 検索の記録の中で `dedupe_key` が一致する候補の位置を返す（無ければ `None`）。
fn search_log_item_position(conn: &rusqlite::Connection, log_id: i64, key: &str) -> Option<i64> {
    let items = papr_core::search_log::items(conn, log_id).ok()?;
    items
        .iter()
        .find(|item| item.dedupe_key == key)
        .map(|item| item.position)
}

async fn process_index_updates(
    receiver: &mut mpsc::UnboundedReceiver<IndexResponse>,
    runtime: &mut Runtime,
    senders: &ActionSenders,
    app: &mut App,
) -> Result<bool> {
    let mut changed = false;
    let has_active_downloads = app.downloads.iter().any(|task| {
        !matches!(
            task.status,
            DownloadStatus::Completed | DownloadStatus::Failed(_)
        )
    });
    while runtime.watch_receiver.try_recv().is_ok() {
        changed = true;
        if !has_active_downloads {
            start_silent_runtime_scan(runtime, senders, app);
        }
    }
    while let Ok(response) = receiver.try_recv() {
        changed = true;
        apply_index_response(response, runtime, senders, app).await?;
    }
    Ok(changed)
}

fn process_enrichment_updates(
    receiver: &mut mpsc::UnboundedReceiver<MetadataEnrichment>,
    runtime: &mut Runtime,
    app: &mut App,
) -> Result<bool> {
    let mut changed = false;
    while let Ok(MetadataEnrichment {
        paper_id,
        doi,
        retraction,
        outcome,
    }) = receiver.try_recv()
    {
        apply_enrichment_outcome(runtime, app, paper_id, doi.as_deref(), retraction, outcome)?;
        runtime.active_enrichments.remove(&paper_id);
        if let Some(task) = app
            .downloads
            .iter_mut()
            .find(|task| task.paper_id == Some(paper_id))
        {
            task.status = DownloadStatus::Completed;
            finalize_download_task(task);
        }
        changed = true;
    }
    if changed {
        refresh_library(runtime, app)?;
        refresh_organization(&runtime.database, &runtime.library_roots, app)?;
        refresh_dashboard(runtime, app)?;
        refresh_downloads(runtime, app);
    }
    if receiver.is_empty() && app.session_flags.enrichment_pending {
        app.session_flags.enrichment_pending = false;
        changed = true;
    }
    Ok(changed)
}

fn apply_enrichment_outcome(
    runtime: &mut Runtime,
    app: &mut App,
    paper_id: i64,
    doi: Option<&str>,
    retraction: Option<bool>,
    outcome: MetadataEnrichmentOutcome,
) -> Result<()> {
    // 補完のついでに分かった撤回の状態を記録する（DOI があるときだけ）。
    if let (Some(doi), Some(status)) = (doi, retraction) {
        runtime.database.set_retraction_by_doi(doi, Some(status))?;
    }
    match outcome {
        MetadataEnrichmentOutcome::Paper(paper) => {
            runtime.database.apply_arxiv_metadata(paper_id, &paper)?;
            for candidate in app
                .today_papers
                .iter_mut()
                .chain(app.discovery.results.iter_mut())
            {
                if candidate.id == paper.id || (paper.doi.is_some() && candidate.doi == paper.doi) {
                    *candidate = merge_enriched_remote_paper(candidate, &paper);
                }
            }
            let _ = runtime.database.save_dashboard_feed_cache(
                &runtime.dashboard_feed_date,
                &runtime.dashboard_keyword_signature,
                &app.today_papers,
            );
        }
        MetadataEnrichmentOutcome::Journal(journal) => {
            runtime
                .database
                .apply_journal_metadata(paper_id, &journal)?;
        }
        MetadataEnrichmentOutcome::Failed(_) => {
            runtime
                .database
                .update_enrichment_status(paper_id, "failed")?;
        }
        MetadataEnrichmentOutcome::Unavailable => {
            runtime
                .database
                .update_enrichment_status(paper_id, "unavailable")?;
        }
    }
    Ok(())
}

async fn process_download_updates(
    receiver: &mut mpsc::UnboundedReceiver<DownloadEvent>,
    pending: &mut HashMap<String, RemotePaper>,
    runtime: &mut Runtime,
    senders: &ActionSenders,
    app: &mut App,
) -> Result<bool> {
    let mut changed = false;
    while let Ok(event) = receiver.try_recv() {
        changed = true;
        apply_download_event(event, pending, runtime, app, senders).await?;
    }
    Ok(changed)
}

fn process_app_events(
    receiver: &mut mpsc::UnboundedReceiver<AppEvent>,
    runtime: &Runtime,
    app: &mut App,
) -> Result<bool> {
    let mut changed = false;
    while let Ok(event) = receiver.try_recv() {
        changed = true;
        match event {
            AppEvent::ReadingSessionCompleted {
                session_id,
                duration_s,
            } => {
                runtime
                    .database
                    .record_reading_duration(session_id, duration_s)?;
                refresh_dashboard(runtime, app)?;
            }
            AppEvent::Toast(message) => app.toast = Some(message),
            AppEvent::SearchSetRerun { set_id, result } => {
                apply_search_set_rerun_result_at(
                    runtime.database.connection(),
                    set_id,
                    result,
                    app,
                );
            }
            AppEvent::OperationFinished(key) => {
                app.running_operations.retain(|op| op.key != key);
            }
            AppEvent::OperationStep { key, step } => {
                if let Some(op) = app.running_operations.iter_mut().find(|op| op.key == key) {
                    op.step = step;
                    op.label = format!("{}: {}", op.step_label(), op.title);
                }
            }
            AppEvent::RefreshLibrary => {
                let _ = refresh_library(runtime, app);
                // 図抽出の完了などでライブラリが更新されたら、PAPER 画面の図も読み直す。
                if app.mode == AppMode::PaperDetail && app.paper_detail_remote.is_none() {
                    reload_paper_figures(&runtime.database, app);
                }
                // Remote PAPER 詳細表示中なら、翻訳完了後の概要訳を DB から取り直す。
                if app.mode == AppMode::PaperDetail
                    && app.page != Page::Library
                    && let Some(paper_id) = app.paper_detail_remote_db_id
                {
                    app.paper_detail_remote_ja = runtime
                        .database
                        .library_paper_by_id(paper_id)
                        .ok()
                        .flatten()
                        .and_then(|paper| paper.abstract_ja)
                        .filter(|ja| !ja.trim().is_empty());
                }
            }
        }
    }
    Ok(changed)
}

fn update_toast_and_download_cleanup(app: &mut App, last_toast: &mut Option<String>) -> bool {
    let mut changed = false;
    if app.toast.is_some() {
        if app.toast != *last_toast {
            app.toast_timestamp = Some(std::time::Instant::now());
            last_toast.clone_from(&app.toast);
            changed = true;
        }
        if app
            .toast_timestamp
            .is_some_and(|timestamp| timestamp.elapsed() >= std::time::Duration::from_secs(7))
        {
            app.toast = None;
            app.toast_timestamp = None;
            *last_toast = None;
            changed = true;
        }
    } else {
        changed |= last_toast.is_some();
        app.toast_timestamp = None;
        *last_toast = None;
    }
    let before = app.downloads.len();
    app.downloads.retain(|task| {
        !matches!(task.status, DownloadStatus::Failed(_))
            || task
                .failed_at
                .is_none_or(|failed_at| failed_at.elapsed() < std::time::Duration::from_mins(2))
    });
    if app.downloads.len() != before {
        changed = true;
    }
    changed
}

fn terminal_poll_timeout(app: &App, preview_active: bool, animating: bool) -> std::time::Duration {
    if !preview_active {
        return std::time::Duration::from_millis(100);
    }
    if animating {
        std::time::Duration::from_millis(8)
    } else if pdf_viewer::is_current_page_cached(app) {
        std::time::Duration::from_millis(250)
    } else {
        std::time::Duration::from_millis(50)
    }
}

async fn poll_terminal_events(
    app: &mut App,
    runtime: &mut Runtime,
    senders: &ActionSenders,
    pending_downloads: &mut HashMap<String, RemotePaper>,
    theme: &mut Theme,
    timeout: std::time::Duration,
) -> Result<bool> {
    let mut received = false;
    while event::poll(std::time::Duration::ZERO)? {
        received = true;
        process_terminal_event(
            event::read()?,
            app,
            runtime,
            senders,
            pending_downloads,
            theme,
        )
        .await?;
    }
    if event::poll(timeout)? {
        received = true;
        process_terminal_event(
            event::read()?,
            app,
            runtime,
            senders,
            pending_downloads,
            theme,
        )
        .await?;
    }
    Ok(received)
}

async fn process_terminal_event(
    event: Event,
    app: &mut App,
    runtime: &mut Runtime,
    senders: &ActionSenders,
    pending_downloads: &mut HashMap<String, RemotePaper>,
    theme: &mut Theme,
) -> Result<()> {
    let action = match event {
        Event::Key(key)
            if key.kind == KeyEventKind::Press
                || (app.mode == AppMode::PdfView && key.kind == KeyEventKind::Repeat) =>
        {
            handle_key(app, key)
        }
        Event::Paste(text) => {
            paste_text_into_active_input(app, &text);
            None
        }
        // マウスはホイールのスクロールだけに使う（カード移動などは無い）。
        Event::Mouse(mouse) => {
            handle_mouse_event(app, mouse);
            None
        }
        _ => None,
    };
    // 研究方針のページに入った直後と、`r` で読み直しの指示を出したあとに木を読み直す。
    if app.tree.stale || app.tree.detail_stale {
        if let Some(roots) = runtime_paper_roots(runtime) {
            tree_view::reload(&mut app.tree, &runtime.database, &roots);
        } else {
            app.tree.stale = false;
            app.tree.detail_stale = false;
            app.tree.error = Some("プロジェクトルートが見つかりません".into());
        }
    }
    if let Some(action) = action {
        apply_ui_action(action, runtime, senders, pending_downloads, app, theme).await?;
    }
    Ok(())
}

/// 発行年入力欄を閉区間に解釈する（SearchQuery.with_years 用）。
/// `2010-2020` → `(2010, 2020)`、単年 `2019` → `(2019, 2019)`、空欄・不正値 → `None`。
fn parse_year_range(raw: &str) -> Option<(i32, i32)> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    let (from, to) = match trimmed.split_once('-') {
        Some((from, to)) => (from.trim(), to.trim()),
        None => (trimmed, trimmed),
    };
    let from = from.parse::<i32>().ok()?;
    let to = to.parse::<i32>().ok()?;
    Some((from.min(to), from.max(to)))
}

/// R3: 取得件数入力欄を解決する。空欄時はデフォルト 100、上限は 1000。
fn parse_discovery_limit(raw: &str) -> u16 {
    raw.trim().parse::<u16>().unwrap_or(100).clamp(1, 1000)
}

/// 次のページを頼むかどうかを判定する。
///
/// - `before` / `after`: 今回のバッチを併合する前の論文数と、併合・重複除去・並べ替えの後の論文数。
/// - `received`: 相手から受け取った件数。
/// - `batch`: 今回頼んだ件数。
///
/// `offset` を見ない DB（arXiv・PubMed・Europe PMC・OpenAlex）は同じ結果を返し続けるため、
/// 新しい論文が 1 件も増えなかった時点で止める。
#[must_use]
fn should_fetch_next(before: usize, after: usize, received: usize, batch: usize) -> bool {
    let added = after > before;
    added && received >= batch && after < DISCOVERY_CANDIDATE_LIMIT as usize
}

async fn fetch_discovery_pages(
    config: Config,
    providers: Vec<String>,
    query: String,
    year: String,
    limit: u16,
    request_id: u64,
    mut papers: Vec<RemotePaper>,
    mut start: u16,
    response_sender: mpsc::UnboundedSender<SearchResponse>,
) {
    use papr_core::api::{PaperSourceResolver, SearchQuery};
    use std::time::Duration;

    // 取得件数（R3）。空欄時はデフォルト 100。
    let candidate_limit = limit.max(1);

    // No provider configured → immediate failure.
    if providers.is_empty() {
        let _ = response_sender.send(SearchResponse {
            query,
            request_id,
            update: SearchUpdate::InitialFailure("no discovery providers configured".into()),
        });
        return;
    }

    // 年度範囲をパースする
    let (year_from, year_to) = match parse_year_range(&year) {
        Some((from, to)) => (Some(from as u16), Some(to as u16)),
        None => (None, None),
    };

    // ---- Unified path for all providers using SearchQuery + dedupe ----
    // Fetches from all specified providers, deduplicates results, and paginates using offset.
    loop {
        // バッチ取得をループで行う（DISCOVERY_FETCH_BATCH_SIZE 単位）
        let batch_size = if start == 0 {
            DISCOVERY_FETCH_BATCH_SIZE
        } else {
            candidate_limit
        };

        // SearchQuery を構築（offset ベース）
        let mut search_q = SearchQuery::new(&query)
            .with_limit(batch_size)
            .with_offset(start as u32);
        if let (Some(from), Some(to)) = (year_from, year_to) {
            search_q = search_q.with_years(Some(from), Some(to));
        }

        // リトライロジック付きで各 provider から検索を実行
        let mut all_batch_papers = Vec::new();
        let mut failed_providers = Vec::new();

        for provider_id in &providers {
            let mut attempt = 0;
            let max_attempts = DISCOVERY_PAGE_RETRY_ATTEMPTS;
            let mut last_error = None;

            loop {
                attempt += 1;
                match PaperSourceResolver::resolve(provider_id, &config).await {
                    Ok(source) => {
                        match source.search(&search_q).await {
                            Ok(batch_papers) => {
                                all_batch_papers.extend(batch_papers);
                                break; // 成功したらループを抜ける
                            }
                            Err(e) => {
                                last_error = Some(e.to_string());
                                if attempt < max_attempts {
                                    // リトライ通知を送信
                                    let _ = response_sender.send(SearchResponse {
                                        query: query.clone(),
                                        request_id,
                                        update: SearchUpdate::Retrying {
                                            attempt: attempt as u8,
                                            max_attempts: max_attempts as u8,
                                        },
                                    });
                                    // バックオフ遅延
                                    let delay = DISCOVERY_PAGE_RETRY_BASE_DELAY.as_millis() as u64
                                        * (2_u64.pow(attempt as u32 - 1));
                                    tokio::time::sleep(Duration::from_millis(delay)).await;
                                }
                            }
                        }
                    }
                    Err(e) => {
                        last_error = Some(e.to_string());
                        if attempt < max_attempts {
                            // リトライ通知を送信
                            let _ = response_sender.send(SearchResponse {
                                query: query.clone(),
                                request_id,
                                update: SearchUpdate::Retrying {
                                    attempt: attempt as u8,
                                    max_attempts: max_attempts as u8,
                                },
                            });
                            // バックオフ遅延
                            let delay = DISCOVERY_PAGE_RETRY_BASE_DELAY.as_millis() as u64
                                * (2_u64.pow(attempt as u32 - 1));
                            tokio::time::sleep(Duration::from_millis(delay)).await;
                        }
                    }
                }

                if attempt >= max_attempts {
                    break; // リトライ上限に到達
                }
            }

            if let Some(error) = last_error {
                failed_providers.push(format!("{}: {}", provider_id, error));
            }
        }

        // ページング判定に使うため、統合前の件数と受信件数を控えておく。
        let before = papers.len();
        let received_count = all_batch_papers.len();
        // 重複除外と関連度でのランキング
        let mut merged = papers.clone();
        merged.extend(all_batch_papers);
        let mut deduped = dedupe_remote_papers(merged);

        // 関連度順に並べ替え
        rank_remote_papers_by_query(&query, &mut deduped);

        // 件数制限を適用
        let has_more = deduped.len() > candidate_limit as usize;
        if has_more {
            let truncated = deduped.into_iter().take(candidate_limit as usize).collect();
            papers = truncated;
        } else {
            papers = deduped;
        }

        // 検索のあと OpenAlex で被引用数をまとめて補う（arXiv などは持たない）。
        // すでに入っている論文は問い合わせないので、何回呼んでもよい。
        papr_core::api::citations::fill_citation_counts(&config, &mut papers).await;

        // エラーメッセージを作成（複数 provider が失敗した場合は結合）
        if !failed_providers.is_empty() {
            let error_msg = failed_providers.join("; ");
            if papers.is_empty() {
                // 初回検索ですべてのプロバイダが失敗
                let _ = response_sender.send(SearchResponse {
                    query,
                    request_id,
                    update: SearchUpdate::InitialFailure(error_msg),
                });
                return;
            } else if start > 0 {
                // 追加読み込みで複数プロバイダ中一部が失敗
                let _ = response_sender.send(SearchResponse {
                    query: query.clone(),
                    request_id,
                    update: SearchUpdate::PartialFailure {
                        next_start: start.saturating_add(candidate_limit),
                    },
                });
                // エラーメッセージはメッセージ欄に表示（以後ページング停止）
                return;
            } else {
                // 初回検索で複数プロバイダ中一部が失敗（結果はあり）
                let _ = response_sender.send(SearchResponse {
                    query: query.clone(),
                    request_id,
                    update: SearchUpdate::Partial {
                        papers: papers.clone(),
                        next_start: Some(start.saturating_add(candidate_limit)),
                    },
                });
                // エラーメッセージを画面下に出すため、続行する
                return;
            }
        }

        // ページング判定：取得件数がバッチサイズに達しているなら、次ページがある可能性がある。
        // ただし「新しい論文が 1 件も増えなかったら」止める（`offset` を見ない DB
        // —arXiv・PubMed・Europe PMC・OpenAlex— が同じ 100 件を返し続けるため）。
        let has_more_pages =
            should_fetch_next(before, papers.len(), received_count, batch_size as usize);

        if has_more_pages {
            let next_start = start.saturating_add(batch_size);
            let _ = response_sender.send(SearchResponse {
                query: query.clone(),
                request_id,
                update: SearchUpdate::Partial {
                    papers: papers.clone(),
                    next_start: Some(next_start),
                },
            });
            start = next_start;
        } else {
            // 取得件数がバッチサイズ未満 = 最後のページに到達
            let _ = response_sender.send(SearchResponse {
                query,
                request_id,
                update: SearchUpdate::Complete(papers),
            });
            return;
        }
    }
}

async fn apply_ui_action(
    action: UiAction,
    runtime: &mut Runtime,
    senders: &ActionSenders,
    pending_downloads: &mut HashMap<String, RemotePaper>,
    app: &mut App,
    theme: &Theme,
) -> Result<()> {
    match action {
        action @ (UiAction::Search(_)
        | UiAction::RetryDiscoverMore
        | UiAction::ScanBibtex
        | UiAction::OpenPaper(_)
        | UiAction::OpenBrowser(_)
        | UiAction::Download(_)
        | UiAction::Reindex
        | UiAction::OpenManualImport(_)
        | UiAction::TranslateAbstractRemote { .. }
        | UiAction::ConfirmManualImport) => {
            apply_discovery_action(action, runtime, senders, pending_downloads, app).await
        }
        action @ (UiAction::OpenPdf { .. }
        | UiAction::OpenNote(_)
        | UiAction::Prompt(_)
        | UiAction::RenamePdf(_)
        | UiAction::OpenTagModal(_)
        | UiAction::OpenTagPaperList(_)
        | UiAction::SubmitPrompt(_)
        | UiAction::Bookmark(_)) => {
            apply_paper_metadata_action(action, runtime, senders, app).await
        }
        action @ (UiAction::ClosePdf
        | UiAction::OpenAuthor(_)
        | UiAction::OpenLibraryPaper(_)
        | UiAction::MarkUnread(_)
        | UiAction::OpenHandoff(_)) => {
            apply_library_navigation_action(action, runtime, senders, app)
        }
        UiAction::CopyCitation(_) => apply_citation_action(action, runtime, senders, app),
        UiAction::OpenProjectFile(file) => apply_open_project_file(file, runtime, app, theme),
        UiAction::OpenJournal => {
            apply_open_journal(runtime, app, theme);
            Ok(())
        }
        UiAction::OpenAttention(item) => apply_open_attention(&item, runtime, app, theme),
        UiAction::ResolveAttention(item) => {
            apply_resolve_attention(&item, runtime, app);
            Ok(())
        }
        action @ (UiAction::ConfirmDeletePaper { .. } | UiAction::DeletePaper { .. }) => {
            apply_deletion_action(action, runtime, app)
        }
        UiAction::CitationSidecar { paper_id } => {
            apply_citation_sidecar(paper_id, runtime, senders, app).await
        }
        UiAction::CitationSidecarRemote { paper } => {
            apply_citation_sidecar_remote(paper, runtime, senders, app).await
        }
        UiAction::TranslateAbstract { paper_id, force } => {
            apply_translate_abstract(paper_id, force, runtime, senders, app).await
        }
        UiAction::TranslatePdf { paper_id } => {
            apply_translate_pdf(paper_id, runtime, senders, app).await
        }
        UiAction::GeneratePodcast { paper_id } => {
            apply_generate_podcast(paper_id, runtime, senders, app).await
        }
        UiAction::LoadSearchSetItems(set_id) => {
            apply_load_search_set_items(set_id, runtime, app).await
        }
        UiAction::MarkSearchSetItem {
            set_id,
            position,
            decision,
        } => apply_mark_search_set_item(set_id, position, decision, runtime, app).await,
        UiAction::RerunSearchSet(set_id) => {
            start_search_set_rerun(set_id, runtime, senders, app);
            Ok(())
        }
        UiAction::ExecuteRagSearch => apply_rag_search(runtime, app).await,
        UiAction::OpenRagSearchMarkdown(path) => {
            open_markdown_file(&path)?;
            record_open_in_tui(runtime, Path::new(&path), None);
            Ok(())
        }
        UiAction::OpenReport(report_id) => {
            open_report_in_viewer(&report_id, runtime, app, theme);
            Ok(())
        }
        UiAction::OpenCitation { paper_id, locator } => {
            apply_open_citation(paper_id, locator.as_deref(), runtime, senders, app, theme).await
        }
        UiAction::CheckReport(report_id) => {
            apply_check_report(&report_id, runtime, app, theme).await
        }
        UiAction::OpenMarkdownViewer {
            kind,
            paper_id,
            title,
            extract_key,
        } => {
            let _ = apply_open_markdown_viewer(
                kind,
                paper_id,
                &title,
                extract_key.as_deref(),
                runtime,
                app,
                theme,
            );
            Ok(())
        }
        UiAction::OpenMarkdownInNvim {
            kind,
            paper_id,
            title,
            extract_key,
        } => {
            apply_open_markdown_in_nvim(
                kind,
                paper_id,
                &title,
                extract_key.as_deref(),
                runtime,
                app,
            );
            Ok(())
        }
        UiAction::OpenQueryDetail(log_id) => {
            apply_open_query_detail(log_id, runtime, app);
            Ok(())
        }
        UiAction::OpenTreeMd => {
            open_md_viewer_path(
                runtime,
                app,
                theme,
                "tree.md",
                &runtime.data_dir.join("tree.md"),
            );
            Ok(())
        }
        UiAction::SaveQueryReason => {
            apply_save_query_reason(runtime, app);
            Ok(())
        }
        UiAction::CreateAgentCard(kind) => {
            apply_create_agent_card(kind, runtime, app);
            Ok(())
        }
    }
}

/// N08: `N` / `U` のカード作成結果（トーストの材料）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AgentCardOutcome {
    /// 新しくカードを作った。
    Created { run_id: i64 },
    /// 未実行の同じカードが既に todo にあった。
    Existing { run_id: i64 },
}

/// N08: sai-link の todo に「次の一手」「理解の確認」カードを 1 枚作る。
///
/// 手順:
/// 1. `todo` に同じブロックの未実行カードがあれば、それを返す（新しく作らない）。
/// 2. ブロック定義が無ければ専用のエラー文字列を返す。
/// 3. [`papr_core::create_handoff_with`] でカードを作る（論文は付けない）。
fn create_agent_card(
    db: &Database,
    roots: &PaperRoots,
    kind: AgentCard,
    runner: &str,
) -> std::result::Result<AgentCardOutcome, String> {
    // 1. 未実行の同じカードを探す（同じカードが並ぶのを防ぐ）。
    let todo = papr_core::list_todo_cards(roots, None).map_err(|e| e.to_string())?;
    if let Some(card) = todo.iter().find(|card| card.block == kind.block_id()) {
        return Ok(AgentCardOutcome::Existing { run_id: card.run_id });
    }

    // 2. ブロック定義を確認する。無ければ配り方を案内する。
    let blocks = papr_core::list_blocks(roots).map_err(|e| e.to_string())?;
    if !blocks.iter().any(|block| block.id == kind.block_id()) {
        return Err(format!(
            "{} ブロックがありません。`sai-moon-paper init --assets-only` で配ってください",
            kind.block_id()
        ));
    }

    // 3. カードを作る。実行は sai-link でユーザーが行う。
    let result = papr_core::create_handoff_with(
        db,
        roots,
        &papr_core::HandoffRequest {
            block_id: kind.block_id(),
            paper_ids: &[],
            description: kind.description(),
            tag: Some(kind.label()),
            runner,
            origin_run: None,
            reason: None,
        },
    )
    .map_err(|e| e.to_string())?;

    Ok(AgentCardOutcome::Created {
        run_id: result.run_id,
    })
}

/// `UiAction::CreateAgentCard` の適用: プロジェクトルートを解決してトーストを出す。
fn apply_create_agent_card(kind: AgentCard, runtime: &Runtime, app: &mut App) {
    let Some(roots) = papr_core::paper_roots() else {
        app.toast = Some("プロジェクトルートが見つかりません".into());
        return;
    };
    apply_create_agent_card_at(kind, &runtime.database, &roots, app);
}

/// [`apply_create_agent_card`] の本体（テストできるようにルートを受け取る）。
fn apply_create_agent_card_at(
    kind: AgentCard,
    db: &Database,
    roots: &PaperRoots,
    app: &mut App,
) {
    match create_agent_card(db, roots, kind, "pi") {
        Ok(AgentCardOutcome::Existing { run_id }) => {
            app.toast = Some(format!(
                "sai-link の todo に未実行の『{}』カードがあります（run {}）",
                kind.label(),
                run_id
            ));
        }
        Ok(AgentCardOutcome::Created { run_id }) => {
            app.toast = Some(format!(
                "sai-link の todo に『{}』カードを作りました（run {}）。sai-link で実行してください",
                kind.label(),
                run_id
            ));
        }
        Err(message) => {
            app.toast = Some(message);
        }
    }
}

/// `s` キー（ライブラリ論文）: 引用抽出（ensure_extract）→ sai-paper-tui 起動。
///
/// 抽出キーは `papers.s2_paper_id` → `arxiv_id` → `doi` の優先順で解決し、
/// `normalize_paper_id` で正規化する。解析はバックグラウンドタスクで実行し、
/// 完了後に sai-paper-tui を起動する（UI はトーストを表示しながら操作可能）。
async fn apply_citation_sidecar(
    paper_id: i64,
    runtime: &mut Runtime,
    senders: &ActionSenders,
    app: &mut App,
) -> Result<()> {
    let Some(raw_key) = runtime.database.citation_key_for_paper(paper_id) else {
        app.toast = Some("citation: arXiv ID or DOI required (S2のみ)".into());
        return Ok(());
    };
    let key = papr_core::citation::normalize_paper_id(&raw_key);
    // PDF パスは DB のライブラリ情報から取得
    let local_pdf = runtime
        .database
        .library_paper_by_id(paper_id)
        .ok()
        .flatten()
        .and_then(|p| p.pdf_path)
        .map(PathBuf::from);
    spawn_citation_flow(key, None, local_pdf, runtime, senders, app);
    Ok(())
}

/// `s` キー（Discover / Dashboard 論文）: PDF 自動 DL → 引用抽出 → tui 起動。
///
/// 仕様（2026-08-28 確定）:
/// - A1: PDF 未所持なら S2 open access / arXiv から自動 DL。
///   DL 失敗時はエラートーストを表示して終了（tui は起動しない）。
/// - A5: DL した PDF は Discover での DL と同じ扱い（attach_download）でライブラリ登録。
/// - A3: docling 解析失敗時は S2 refs のみで tui を起動。
/// - A6: ensure_extract は stale 判定キャッシュ付きのため、2 回目以降は解析なしで即起動。
async fn apply_citation_sidecar_remote(
    paper: RemotePaper,
    runtime: &mut Runtime,
    senders: &ActionSenders,
    app: &mut App,
) -> Result<()> {
    // 抽出キーを解決（paper.id は S2 40hex / arXiv URL / DOI を取り得る）
    let raw_key = if paper.id.starts_with("ArXiv:") || paper.id.starts_with("DOI:") {
        paper.id.clone()
    } else if paper.id.contains('/') {
        format!("DOI:{}", paper.id)
    } else {
        format!("ArXiv:{}", paper.id)
    };
    let key = papr_core::citation::normalize_paper_id(&raw_key);
    // 既存ローカル PDF があれば DL を省略する
    let local_pdf = {
        let conn = runtime.database.connection();
        papr_core::citation::db::get_paper_meta_by_extract_key(conn, &key)
            .ok()
            .flatten()
            .and_then(|meta| meta.pdf_path)
            // DB 保存形（相対）を実ファイルパスへ直す（段階 2b）。
            .map(|stored| runtime.database.resolve_path(&stored))
    };
    spawn_citation_flow(key, Some(paper), local_pdf, runtime, senders, app);
    Ok(())
}

/// 引用サイドカー共通フローをバックグラウンドタスクで起動する。
///
/// UI を凍結させないため、PDF DL / docling 解析（数十秒）は tokio タスクに逃がし、
/// 進捗・結果は `AppEvent::Toast` で通知する（A2: 解析完了まで待ってから tui 起動）。
/// `RunningOperation` を組み立てる（TASKS 画面のステップ表示用）。
///
/// `steps` は処理の全ステップ名、`step` は開始時点で実行中のステップ番号
/// （= 完了済みステップ数）。進捗率はこの 2 つから算出する。
fn running_op(key: &str, title: &str, steps: &[&str], step: usize) -> RunningOperation {
    let steps: Vec<String> = steps.iter().map(|s| (*s).to_string()).collect();
    let mut op = RunningOperation {
        key: key.to_owned(),
        label: String::new(),
        title: title.to_owned(),
        steps,
        step,
        started_at: std::time::Instant::now(),
    };
    op.label = format!("{}: {}", op.step_label(), op.title);
    op
}

fn spawn_citation_flow(
    key: String,
    remote: Option<RemotePaper>,
    local_pdf: Option<PathBuf>,
    runtime: &mut Runtime,
    senders: &ActionSenders,
    app: &mut App,
) {
    // 二重実行防止: 同じ論文の引用抽出が実行中なら無視する。
    let op_key = format!("citation:{key}");
    if app.running_operations.iter().any(|op| op.key == op_key) {
        app.toast = Some("引用抽出はすでに実行中です".into());
        return;
    }
    let title = match remote.as_ref() {
        Some(paper) => paper.title.clone(),
        None => key.clone(),
    };
    // PDF が手元にあれば `PDF取得` は完了済みとして 1 ステップ目から始める。
    let start_step = usize::from(local_pdf.as_ref().is_some_and(|p| p.exists()));
    app.running_operations.push(running_op(
        &op_key,
        &title,
        &["PDF取得", "引用抽出"],
        start_step,
    ));
    app.toast = Some("引用抽出を開始しています（数十秒かかることがあります）…".into());
    let s2 = runtime.s2.clone();
    let db_file = runtime.database_file.clone();
    let data_dir = runtime
        .database_file
        .parent()
        .map(std::path::Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("data"));
    let download_dir = runtime.download_dir.clone();
    let app_events = senders.app_events.clone();
    // Database（rusqlite Connection）は Sync でないため tokio::spawn は使えない。
    // 専用スレッド + current-thread ランタイムで実行し、UI はトースト表示のまま操作可能に保つ。
    std::thread::spawn(move || {
        let rt = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(rt) => rt,
            Err(_) => return,
        };
        rt.block_on(run_citation_sidecar_flow(
            key,
            remote,
            local_pdf,
            s2,
            db_file,
            data_dir,
            download_dir,
            app_events.clone(),
        ));
        // 完了（成功 / 失敗 / 早期 return のいずれでも）を UI に通知し、実行中表示を消す。
        let _ = app_events.send(AppEvent::OperationFinished(op_key));
    });
}

/// 引用サイドカー共通フローの本体（バックグラウンドタスク内で実行）。
async fn run_citation_sidecar_flow(
    key: String,
    remote: Option<RemotePaper>,
    local_pdf: Option<PathBuf>,
    s2: S2Client,
    db_file: PathBuf,
    data_dir: PathBuf,
    download_dir: PathBuf,
    app_events: mpsc::UnboundedSender<AppEvent>,
) {
    // spawn_citation_flow が push した RunningOperation と同じキー。
    let op_key = format!("citation:{key}");
    let notify = |msg: String| {
        let _ = app_events.send(AppEvent::Toast(msg));
    };

    // 1) PDF 確保: ローカル PDF → 無ければ自動 DL（A1: 失敗時は終了）
    let pdf_path = match local_pdf {
        Some(path) if path.exists() => path,
        _ => {
            let Some(paper) = remote.as_ref() else {
                notify("citation: PDF not downloaded yet".into());
                return;
            };
            match download_paper_pdf(&s2, paper, &key, &download_dir).await {
                Ok((path, imported)) => {
                    // A5: Discover DL と同じ扱いでライブラリ登録
                    // （置き場は attach_download 内の paths::paper_folder が決める）。
                    let mut db = match Database::open(&db_file) {
                        Ok(db) => db,
                        Err(e) => {
                            notify(format!("citation: DB open failed: {e}"));
                            return;
                        }
                    };
                    // 登録後は PDF が移動しているため、DB に記録された実パスを使う。
                    let mut resolved = path.clone();
                    match db.attach_download(paper, &imported, &data_dir) {
                        Ok(paper_id) => {
                            if let Ok(Some(saved)) = db.library_paper_by_id(paper_id)
                                && let Some(stored) =
                                    saved.pdf_path.filter(|s| !s.trim().is_empty())
                            {
                                resolved = PathBuf::from(stored);
                            }
                        }
                        Err(e) => notify(format!("citation: library登録に失敗: {e}")),
                    }
                    resolved
                }
                Err(e) => {
                    notify(format!("citation: PDF ダウンロードに失敗 — {e}"));
                    return;
                }
            }
        }
    };

    // 2) 引用抽出（初回のみ解析。2 回目以降は stale キャッシュで即返る）(A6)
    let _ = app_events.send(AppEvent::OperationStep {
        key: op_key,
        step: 1,
    });
    let extract = {
        let db = match Database::open(&db_file) {
            Ok(db) => db,
            Err(e) => {
                notify(format!("citation: DB open failed: {e}"));
                return;
            }
        };
        papr_core::citation::ensure_extract(db.connection(), &s2, &key, &pdf_path, &data_dir).await
    };
    match extract {
        Ok(out) => {
            notify(format!(
                "解析完了: {}件の参考文献、引用文 {}件",
                out.refs.len(),
                out.sentences.len()
            ));
            spawn_papr_sai("citation", &key);
        }
        // A3: docling 解析失敗 → S2 refs のみで tui を起動
        Err(papr_core::ExtractError::Docling(e)) => {
            notify(format!("docling 解析失敗 — S2 の引用リストのみ表示: {e}"));
            spawn_papr_sai("citation", &key);
        }
        // A4: S2 refs 失敗は ensure_extract 内で docling のみ続行するため到達しない想定。
        // DB / IO 等のその他の失敗は起動しない。
        Err(e) => {
            notify(format!("引用抽出に失敗: {e}"));
        }
    }
}

/// S2 / arXiv から論文 PDF をダウンロードする（Discover DL と同じ保存先・検証規約）。
async fn download_paper_pdf(
    s2: &S2Client,
    paper: &RemotePaper,
    key: &str,
    download_dir: &std::path::Path,
) -> anyhow::Result<(PathBuf, ImportedPdf)> {
    // PDF URL 解決: S2 open access → paper.pdf_url → arXiv フォールバック
    let url = match s2.get_paper(key).await {
        Ok(detail) => detail
            .open_access_pdf
            .and_then(|p| p.url)
            .filter(|u| !u.trim().is_empty())
            .or_else(|| paper.pdf_url.clone().filter(|u| !u.trim().is_empty()))
            .or_else(|| {
                detail
                    .external_ids
                    .as_ref()
                    .and_then(|m| m.get("ArXiv").cloned())
                    .filter(|id| !id.trim().is_empty())
                    .map(|id| format!("https://arxiv.org/pdf/{id}.pdf"))
            }),
        // S2 lookup 失敗時も arXiv 由来なら直接 DL を試みる
        Err(_) => paper
            .pdf_url
            .clone()
            .filter(|u| !u.trim().is_empty())
            .or_else(|| {
                if let Some(rest) = key.strip_prefix("ArXiv:") {
                    Some(format!("https://arxiv.org/pdf/{rest}.pdf"))
                } else {
                    None
                }
            }),
    }
    .with_context(|| format!("no open access PDF for {key}"))?;

    // DL（CLI get-pdf と同じく %PDF マジックを検証）
    let bytes = reqwest::get(&url)
        .await
        .with_context(|| format!("download failed: {url}"))?
        .bytes()
        .await
        .with_context(|| format!("download body failed: {url}"))?;
    if !bytes.starts_with(b"%PDF") {
        anyhow::bail!("downloaded file is not a PDF: {url}");
    }

    // 保存先: papr の download_path + タイトル由来ファイル名
    std::fs::create_dir_all(download_dir)?;
    let filename = format!(
        "{}.pdf",
        papr_core::sanitize_download_filename_component(&paper.title)
    );
    let dest = download_dir.join(filename);
    if !dest.exists() {
        std::fs::write(&dest, &bytes)?;
    }
    let imported = LibraryIndexer::inspect(&dest).context("failed to index downloaded PDF")?;
    Ok((dest, imported))
}

/// `a` / `A` キー: abstract を日本語へ翻訳（google スクレイピング）。
async fn apply_translate_abstract(
    paper_id: i64,
    force: bool,
    runtime: &mut Runtime,
    senders: &ActionSenders,
    app: &mut App,
) -> Result<()> {
    let Some(paper) = runtime.database.library_paper_by_id(paper_id)? else {
        app.toast = Some("論文が見つかりません".into());
        return Ok(());
    };
    let Some(abstract_en) = paper.abstract_text.clone().filter(|s| !s.trim().is_empty()) else {
        app.toast = Some("abstractが空のため翻訳できません".into());
        return Ok(());
    };
    if !force {
        if let Some(ja) = paper.abstract_ja.clone().filter(|s| !s.trim().is_empty()) {
            let _ = ja;
            app.toast = Some("すでに翻訳済みです（Aで再翻訳）".into());
            return Ok(());
        }
    }
    spawn_abstract_translation_flow(paper_id, abstract_en, force, runtime, senders, app);
    Ok(())
}

fn spawn_abstract_translation_flow(
    paper_id: i64,
    abstract_en: String,
    force: bool,
    runtime: &mut Runtime,
    senders: &ActionSenders,
    app: &mut App,
) {
    // 二重実行防止 + TASKS 画面への表示（軽い処理だがパイプラインとは別枠で走る）。
    let op_key = format!("abstract:{paper_id}");
    if app.running_operations.iter().any(|op| op.key == op_key) {
        return;
    }
    let title = runtime
        .database
        .library_paper_by_id(paper_id)
        .ok()
        .flatten()
        .map_or_else(|| format!("paper #{paper_id}"), |paper| paper.title);
    app.running_operations
        .push(running_op(&op_key, &title, &["Abstract翻訳"], 0));
    app.toast = Some("abstractを翻訳中…".into());
    let db_file = runtime.database_file.clone();
    let service = runtime.config.translate.abstract_.clone();
    let app_events = senders.app_events.clone();
    std::thread::spawn(move || {
        let rt = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(rt) => rt,
            Err(_) => return,
        };
        rt.block_on(run_abstract_translation_flow(
            paper_id,
            abstract_en,
            force,
            service,
            db_file,
            app_events.clone(),
        ));
        let _ = app_events.send(AppEvent::OperationFinished(op_key));
    });
}

async fn run_abstract_translation_flow(
    paper_id: i64,
    abstract_en: String,
    force: bool,
    service: papr_core::config::TranslateService,
    db_file: PathBuf,
    app_events: mpsc::UnboundedSender<AppEvent>,
) {
    let notify = |msg: String| {
        let _ = app_events.send(AppEvent::Toast(msg));
    };
    let refresh = || {
        let _ = app_events.send(AppEvent::RefreshLibrary);
    };
    let db = match Database::open(&db_file) {
        Ok(db) => db,
        Err(e) => {
            notify(format!("翻訳失敗: DB open error {e}"));
            return;
        }
    };
    match papr_core::translation::ensure_abstract_ja(db.connection(), paper_id, &abstract_en, force, &service)
        .await
    {
        Ok(Some(ja)) => {
            let preview = ja.chars().take(60).collect::<String>();
            notify(format!("翻訳完了: {preview}…"));
            refresh();
        }
        Ok(None) => notify("翻訳スキップ: abstractが空です".into()),
        Err(e) => notify(format!("翻訳失敗: {e}")),
    }
}

/// 修正2: Library に保存した論文の abstract 自動翻訳。
///
/// abstract が未翻訳ならバックグラウンドで日本語訳を開始する。結果は DB
/// （`papers.abstract_ja`）に保存され、以降は Library 詳細で常に表示される。
/// PDF 側の重い処理（図抽出 → PDF 翻訳）は `spawn_paper_pipeline` が担当する。
/// ダウンロード完了・手動 import 完了の直後に呼び出す。
fn auto_translate_after_library_save(
    runtime: &mut Runtime,
    senders: &ActionSenders,
    app: &mut App,
    paper_id: i64,
) {
    // abstract 翻訳: 未翻訳のときのみ（ensure_abstract_ja の DB キャッシュに保存）
    if let Ok(Some(paper)) = runtime.database.library_paper_by_id(paper_id)
        && paper
            .abstract_ja
            .as_deref()
            .is_none_or(|ja| ja.trim().is_empty())
        && let Some(en) = paper
            .abstract_text
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
    {
        spawn_abstract_translation_flow(paper_id, en.to_owned(), false, runtime, senders, app);
    }
}

/// `T` キー: PDF を日本語へ翻訳（pdf2zh）。

async fn apply_translate_pdf(
    paper_id: i64,
    runtime: &mut Runtime,
    senders: &ActionSenders,
    app: &mut App,
) -> Result<()> {
    let Some(paper) = runtime.database.library_paper_by_id(paper_id)? else {
        app.toast = Some("論文が見つかりません".into());
        return Ok(());
    };
    let Some(pdf_path) = paper.pdf_path.clone().filter(|s| !s.trim().is_empty()) else {
        app.toast = Some("PDFが紐付いていません".into());
        return Ok(());
    };
    if !PathBuf::from(&pdf_path).exists() {
        app.toast = Some(format!("PDFが見つかりません: {pdf_path}"));
        return Ok(());
    }
    // 翻訳済みなら再翻訳しない（T キー側で view を開くため通常はここに来ないが安全網）
    if let Some(existing) = papr_core::paths::translated_pdf_path(&pdf_path) {
        app.toast = Some(format!("翻訳済み: {}", existing.display()));
        return Ok(());
    }
    spawn_pdf_translation_flow(paper_id, pdf_path, runtime, senders, app);
    Ok(())
}

fn spawn_pdf_translation_flow(
    paper_id: i64,
    pdf_path: String,
    runtime: &mut Runtime,
    senders: &ActionSenders,
    app: &mut App,
) {
    // 二重実行防止: 同じ論文の PDF 翻訳（単体 / パイプライン経由）が実行中なら無視する。
    let op_key = format!("translate:{paper_id}");
    let pipeline_key = format!("pipeline:{paper_id}");
    if app
        .running_operations
        .iter()
        .any(|op| op.key == op_key || op.key == pipeline_key)
    {
        app.toast = Some("この論文の処理はすでに実行中です（shift+L で確認）".into());
        return;
    }
    let title = runtime
        .database
        .library_paper_by_id(paper_id)
        .ok()
        .flatten()
        .map_or_else(
            || {
                Path::new(&pdf_path)
                    .file_name()
                    .map_or_else(|| pdf_path.clone(), |n| n.to_string_lossy().to_string())
            },
            |paper| paper.title,
        );
    // PDF は手元にあるので `PDF取得` は完了済み扱い。
    app.running_operations
        .push(running_op(&op_key, &title, &["PDF取得", "翻訳"], 1));
    app.toast = Some("PDF翻訳を開始しています（数分かかります）…".into());
    let db_file = runtime.database_file.clone();
    let translate = runtime.config.translate.pdf.clone();
    let max_parallel = runtime.config.pipeline.max_parallel;
    let app_events = senders.app_events.clone();
    std::thread::spawn(move || {
        let rt = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(rt) => rt,
            Err(_) => {
                let _ = app_events.send(AppEvent::OperationFinished(op_key));
                return;
            }
        };
        rt.block_on(async {
            let _permit = papr_core::pipeline::slots(max_parallel).acquire().await;
            run_pdf_translation_flow(paper_id, db_file, translate, app_events.clone()).await;
        });
        let _ = app_events.send(AppEvent::OperationFinished(op_key));
    });
}

/// PDF 翻訳（本体は core の `pipeline::translate_pdf`。TUI・CLI・phone で共通）。
async fn run_pdf_translation_flow(
    paper_id: i64,
    db_file: PathBuf,
    translate: papr_core::config::TranslateService,
    app_events: mpsc::UnboundedSender<AppEvent>,
) {
    let message = match papr_core::pipeline::translate_pdf(&db_file, paper_id, &translate).await {
        Ok(path) => {
            let _ = app_events.send(AppEvent::RefreshLibrary);
            format!("PDF翻訳完了: {}", path.display())
        }
        Err(e) => format!("PDF翻訳失敗: {e}"),
    };
    let _ = app_events.send(AppEvent::Toast(message));
}

/// ポッドキャスト生成を実行する（`r` キー）。
///
/// extract.md が存在することを確認した上で、ポッドキャスト生成フローを起動する。
async fn apply_generate_podcast(
    paper_id: i64,
    runtime: &mut Runtime,
    senders: &ActionSenders,
    app: &mut App,
) -> Result<()> {
    let Some(paper) = runtime.database.library_paper_by_id(paper_id)? else {
        app.toast = Some("論文が見つかりません".into());
        return Ok(());
    };

    // extract.md の存在確認
    let roots = papr_core::paths::paper_roots();
    if let Some(roots) = roots {
        let folder = papr_core::paths::paper_folder(&roots.data_dir, paper_id, &paper.title);
        let extract_path = papr_core::paths::paper_file_path(&folder, "extract");
        if !extract_path.exists() {
            app.toast = Some("先に抽出(sキー)を実行してください".into());
            return Ok(());
        }
    }

    spawn_podcast_generation_flow(paper_id, runtime, senders, app);
    Ok(())
}

/// ポッドキャスト生成をスレッド化して実行する。
fn spawn_podcast_generation_flow(
    paper_id: i64,
    runtime: &mut Runtime,
    senders: &ActionSenders,
    app: &mut App,
) {
    // 二重実行防止: 同じ論文のポッドキャスト生成が実行中なら無視する。
    let op_key = format!("podcast:{paper_id}");
    if app
        .running_operations
        .iter()
        .any(|op| op.key == op_key)
    {
        app.toast = Some("この論文の処理はすでに実行中です（shift+L で確認）".into());
        return;
    }

    let title = runtime
        .database
        .library_paper_by_id(paper_id)
        .ok()
        .flatten()
        .map_or_else(|| format!("論文 {paper_id}"), |paper| paper.title);

    // ポッドキャスト生成の進捗表示（3ステップ）
    app.running_operations
        .push(running_op(&op_key, &title, &["要約", "台本生成", "音声合成"], 0));
    app.toast = Some("ポッドキャスト生成を開始しています（数分かかります）…".into());

    let db_file = runtime.database_file.clone();
    let podcast_config = runtime.config.radio.clone();
    let max_parallel = runtime.config.pipeline.max_parallel;
    let app_events = senders.app_events.clone();

    std::thread::spawn(move || {
        let rt = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(rt) => rt,
            Err(_) => {
                let _ = app_events.send(AppEvent::OperationFinished(op_key));
                return;
            }
        };
        rt.block_on(async {
            let _permit = papr_core::pipeline::slots(max_parallel).acquire().await;
            run_podcast_generation_flow(paper_id, db_file, podcast_config, app_events.clone()).await;
        });
        let _ = app_events.send(AppEvent::OperationFinished(op_key));
    });
}

/// ポッドキャスト生成フローを実行する。
///
/// 処理本体（`paper_podcasts` の更新・`extract.md` 読み込み・sai-radio API 呼び出し）は
/// core の `podcast_client::generate_for_paper` に共通化した（CLI の `phone radio` も同じ）。
/// ここではトースト表示とライブラリ更新だけを担当する。
async fn run_podcast_generation_flow(
    paper_id: i64,
    db_file: PathBuf,
    podcast_config: papr_core::config::RadioConfig,
    app_events: mpsc::UnboundedSender<AppEvent>,
) {
    let notify = |msg: String| {
        let _ = app_events.send(AppEvent::Toast(msg));
    };
    let refresh = || {
        let _ = app_events.send(AppEvent::RefreshLibrary);
    };
    match papr_core::podcast_client::generate_for_paper(&db_file, paper_id, &podcast_config).await
    {
        // Ok の中身は sai-radio に送った音声の名前（<フォルダ名>-<番号>）。
        Ok(_radio_id) => {
            notify("ポッドキャスト生成が完了しました。iPhoneのsai-radioアプリで再生できます".into());
            refresh();
        }
        Err(msg) => notify(format!("ポッドキャスト生成失敗: {msg}")),
    }
}

/// RAG 検索を実行する（Enter キー）。
///
/// `default_mode` に従って word_search / vector_search / hybrid_search を実行し、
/// 結果を `app.rag_search.results` に格納する。ベクトル埋め込みが失敗時は
/// word_search にフォールバック（toast で理由を表示）。
///
/// ponytail: 検索は同期実行（ローカル DB 問合わせが高速のため、スレッド化不要）。
async fn apply_rag_search(runtime: &mut Runtime, app: &mut App) -> Result<()> {
    use papr_core::evidence::{
        ChatClient, ChatCompletionsClient, EvidenceResult, find_with_evidence,
    };
    use papr_core::retrieval::{FindRequest, find};

    let query = app.rag_search.query.trim().to_owned();
    if query.is_empty() {
        return Ok(());
    }
    // 検索は CLI の `find` / MCP と同じ core の関数（普段使う索引 rag.default_index）。
    let request = FindRequest {
        query: query.clone(),
        scope: Some(app.rag_search.scope.clone()),
        ..FindRequest::default()
    };
    let conn = runtime.database.connection();

    // 「要約付き」のときだけ LLM を呼ぶ（model が空ならエラーを出して通常の検索へ）。
    if app.rag_search.evidence {
        match ChatCompletionsClient::from_config(&runtime.config) {
            Ok(client) => {
                let chat: &dyn ChatClient = &client;
                match find_with_evidence(conn, &runtime.config, request.clone(), chat).await {
                    Ok(result) => {
                        let EvidenceResult {
                            hits,
                            dropped,
                            prompt_tokens,
                            completion_tokens,
                            failed,
                        } = result;
                        app.rag_search.summaries = hits.iter().map(|h| h.summary.clone()).collect();
                        app.rag_search.relevances = hits.iter().map(|h| h.relevance).collect();
                        app.rag_search.results = hits.into_iter().map(|h| h.hit).collect();
                        app.rag_search.dropped = dropped;
                        app.rag_search.prompt_tokens = prompt_tokens;
                        app.rag_search.completion_tokens = completion_tokens;
                        app.rag_search.failed = failed;
                        app.rag_search.selected = 0;
                        app.rag_search.scroll = 0;
                        return Ok(());
                    }
                    Err(error) => {
                        app.toast = Some(format!("要約付き検索に失敗しました: {error}"));
                        // そのまま通常の検索にフォールバックする。
                    }
                }
            }
            Err(error) => {
                app.toast = Some(error.to_string());
            }
        }
    }

    let results = match find(conn, &runtime.config, request.clone()).await {
        Ok(hits) => hits,
        // 埋め込みが使えないとき（サーバーに届かない・未作成）は単語検索で出す。
        Err(_) => {
            app.toast = Some("埋め込みサーバに接続できません。単語検索で表示します".into());
            let word = FindRequest {
                mode: Some("word".into()),
                ..request
            };
            find(conn, &runtime.config, word).await.unwrap_or_default()
        }
    };

    app.rag_search.results = results;
    app.rag_search.summaries.clear();
    app.rag_search.relevances.clear();
    app.rag_search.dropped = 0;
    app.rag_search.prompt_tokens = 0;
    app.rag_search.completion_tokens = 0;
    app.rag_search.failed = 0;
    app.rag_search.selected = 0;
    app.rag_search.scroll = 0;
    Ok(())
}

/// Import 画面の `data/bibtex/` スキャンを実行して [`ImportState`] を更新する。
///
/// 同期実行（数十ファイル想定のため spawn 不要）。パースに失敗した .bib は
/// 1 ファイル単位でスキップし、`import.scan_errors` にメッセージを残す。
fn scan_bibtex_import(app: &mut App) {
    let dir = app.data_dir.join("bibtex");
    match papr_core::bibtex::load_bibtex_dir_detailed(&dir) {
        Ok(scan) => {
            app.import.set_results(scan.papers, scan.errors);
            app.import.status = ImportStatus::Ready;
        }
        Err(error) => {
            app.import.results.clear();
            app.import.filtered_indices.clear();
            app.import.scan_errors.clear();
            app.import.selected = 0;
            app.import.scroll = 0;
            app.import.status = ImportStatus::Error(error.to_string());
        }
    }
}

async fn apply_discovery_action(
    action: UiAction,
    runtime: &mut Runtime,
    senders: &ActionSenders,
    pending_downloads: &mut HashMap<String, RemotePaper>,
    app: &mut App,
) -> Result<()> {
    match action {
        UiAction::Search(query) => {
            runtime
                .database
                .record_activity("search", None, Some(&query))?;
            refresh_dashboard(runtime, app)?;
            let request_id = app.discovery.begin_search();
            // R3: year / limit 入力欄の値を検索に反映する（値は画面に残り次回検索も引き継ぐ）。
            let year = app.discovery.year.trim().to_owned();
            let limit = parse_discovery_limit(&app.discovery.limit);
            let config = runtime.config.clone();
            // DB 入力欄を使用してproviders を決定
            let (providers, db_error) =
                get_discovery_providers_from_input(&app.discovery.db, &runtime.config);
            if let Some(msg) = db_error {
                app.discovery.progress_message = Some(msg);
            }
            app.discovery.active_providers = providers.clone();
            let sender = senders.search.clone();
            tokio::spawn(async move {
                fetch_discovery_pages(
                    config,
                    providers,
                    query,
                    year,
                    limit,
                    request_id,
                    Vec::new(),
                    0,
                    sender,
                )
                .await;
            });
        }
        UiAction::ScanBibtex => {
            scan_bibtex_import(app);
        }
        UiAction::RetryDiscoverMore => {
            let Some(start) = app.discovery.next_batch_start else {
                return Ok(());
            };
            app.discovery.status = DiscoveryStatus::Loading;
            app.discovery.progress_message = Some("Loading more results...".to_owned());
            let config = runtime.config.clone();
            // DB 入力欄を使用してproviders を決定
            let (providers, _db_error) =
                get_discovery_providers_from_input(&app.discovery.db, &runtime.config);
            app.discovery.active_providers = providers.clone();
            let sender = senders.search.clone();
            let query = app.discovery.query.clone();
            let year = app.discovery.year.trim().to_owned();
            let limit = parse_discovery_limit(&app.discovery.limit);
            let request_id = app.discovery.request_id;
            let papers = app.discovery.results.clone();
            tokio::spawn(async move {
                fetch_discovery_pages(
                    config, providers, query, year, limit, request_id, papers, start, sender,
                )
                .await;
            });
        }
        UiAction::OpenPaper(paper) => {
            let paper_id = runtime.database.ensure_remote_paper(&paper)?;
            runtime
                .database
                .record_activity("paper_browsed", Some(paper_id), None)?;
            dispatch_plugin_events(runtime, app, &["paper_opened"], paper_id).await?;
            refresh_dashboard(runtime, app)?;
            // PAPER 詳細は「Enter で開いた論文」を保持して表示する。
            // （検索結果のストリーミング更新で選択位置が変わっても影響を受けない）
            app.paper_detail_remote = Some(paper);
            app.mode = AppMode::PaperDetail;
            app.paper_detail_scroll = 0;
            // Remote 詳細でも概要訳を表示できるよう、DB 行 ID と既存の
            // 概要訳（papers.abstract_ja、過去に翻訳済みなら表示）を取得する。
            app.paper_detail_remote_db_id = Some(paper_id);
            app.paper_detail_remote_ja = runtime
                .database
                .library_paper_by_id(paper_id)?
                .and_then(|paper| paper.abstract_ja)
                .filter(|ja| !ja.trim().is_empty());
            refresh_paper_views(runtime, app)?;
            refresh_paper_reports(runtime, app);
        }
        UiAction::OpenBrowser(url) => open_browser(&url, app),
        UiAction::Download(paper) => start_download(
            paper,
            &runtime.download_dir,
            &runtime.downloads,
            &senders.download,
            pending_downloads,
            app,
        ),
        UiAction::Reindex => start_runtime_scan(runtime, senders, app),
        UiAction::OpenManualImport(target) => {
            apply_open_manual_import(target, app);
        }
        UiAction::ConfirmManualImport => {
            apply_confirm_manual_import(runtime, senders, app).await?;
        }
        UiAction::TranslateAbstractRemote { paper, force } => {
            // Remote 詳細の `a` / `A`: DB 行を解決（未登録なら作成）してから
            // Library 論文と同じ翻訳フローへ流用する。
            let paper_id = runtime.database.ensure_remote_paper(&paper)?;
            apply_translate_abstract(paper_id, force, runtime, senders, app).await?;
        }
        _ => unreachable!("discovery action routed to the wrong handler"),
    }
    Ok(())
}

async fn apply_paper_metadata_action(
    action: UiAction,
    runtime: &mut Runtime,
    senders: &ActionSenders,
    app: &mut App,
) -> Result<()> {
    match action {
        UiAction::OpenPdf {
            paper_id,
            path,
            page,
        } => {
            let session_id = runtime.database.record_open(paper_id, true)?;
            open_pdf(
                &runtime.pdf_viewer,
                &path,
                app,
                Some(session_id),
                Some(senders.app_events.clone()),
                page,
            )?;
            dispatch_plugin_events(runtime, app, &["paper_opened"], paper_id).await?;
            refresh_paper_views(runtime, app)?;
        }
        UiAction::OpenNote(target) => {
            let paper_id = resolve_target(target, &mut runtime.database)?;
            runtime
                .database
                .record_activity("note_opened", Some(paper_id), None)?;
            dispatch_plugin_events(runtime, app, &["paper_opened"], paper_id).await?;
            // 仕様 §3.4: ノートは Markdown ファイル + herdr pane で nvim 編集（正式採用）。
            if let Ok(Some(paper)) = runtime.database.library_paper_by_id(paper_id) {
                open_note_in_nvim(runtime, paper_id, &paper.title)?;
            }
        }
        UiAction::Prompt(target) => {
            // Collections は廃止（仕様 §3.1）。タグ付与（t）に移行。
            let _ = resolve_target(target, &mut runtime.database)?;
        }
        UiAction::RenamePdf(id) => open_rename_prompt(app, None, Some(id)),
        UiAction::OpenTagModal(paper_id) => {
            app.modal_return = app.mode;
            app.metadata_prompt = Some(MetadataPrompt {
                paper_id: Some(paper_id),
                tag_paper_id: Some(paper_id),
                rename_collection_id: None,
                rename_paper_id: None,
                value: runtime
                    .database
                    .tags_for_paper(paper_id)
                    .unwrap_or_default()
                    .join(", "),
                cursor: 0,
                selected: 0,
                current_collection: None,
                handoff_block_id: None,
            });
            app.mode = AppMode::Prompt;
        }
        UiAction::OpenTagPaperList(name) => {
            // R2: 選択中タグに紐づく論文一覧へ切り替える。
            app.tag_papers = runtime.database.papers_for_tag_name(&name)?;
            app.active_tag = app.tags.iter().find(|tag| tag.name == name).cloned();
            app.tag_paper_selected = 0;
            app.tag_paper_scroll = 0;
            app.content_focused = true;
        }
        UiAction::SubmitPrompt(prompt) => {
            // Collections は廃止。プロンプトは PDF リネーム用として残す。
            if let Some(block_id) = &prompt.handoff_block_id {
                // handoff: ブロックと説明文が揃ったので task.md を発行する
                if let Some(paper_id) = app.paper_detail_local {
                    match papr_core::paper_roots() {
                        Some(paper_roots) => {
                            match papr_core::create_handoff(
                                &runtime.database,
                                &paper_roots,
                                block_id,
                                &[paper_id],
                                &prompt.value,
                                None,
                                "pi",
                            ) {
                                Ok(result) => {
                                    app.toast = Some(format!(
                                        "task.md を作成しました。sai-link-tui で `x` → `e` を押してください"
                                    ));
                                    eprintln!("Created: {}", result.path.display());
                                }
                                Err(e) => {
                                    app.toast = Some(format!("task.md 発行に失敗: {}", e));
                                }
                            }
                        }
                        None => {
                            app.toast = Some("プロジェクトルートが見つかりません".into());
                        }
                    }
                }
                app.mode = app.modal_return;
            } else if let Some(tag_paper_id) = prompt.tag_paper_id {
                // R1: タグ差分同期（カンマ区切り複数対応）。
                // 追加分は paper_tags に登録、消えた分は削除する。
                let names: Vec<String> = prompt
                    .value
                    .split(',')
                    .map(str::trim)
                    .filter(|name| !name.is_empty())
                    .map(str::to_owned)
                    .collect();
                runtime.database.sync_paper_tags(tag_paper_id, &names)?;
                refresh_organization(&runtime.database, &runtime.library_roots, app)?;
                app.toast = Some("Tags updated.".into());
            } else if let Some(paper_id) = prompt.paper_id {
                rename_paper_pdf(runtime, app, paper_id, &prompt.value)?;
                refresh_after_library_change(runtime, app)?;
                app.toast = Some(format!("Renamed {}", prompt.value));
            }
        }
        UiAction::Bookmark(target) => {
            // Bookmarks は廃止（仕様 §3.5）。旧キー入力は no-op。
            let _ = resolve_target(target, &mut runtime.database)?;
        }
        UiAction::OpenHandoff(_) => {
            // 不到達: OpenHandoff は apply_library_navigation_action で処理される
        }
        _ => unreachable!("paper metadata action routed to the wrong handler"),
    }
    Ok(())
}

fn open_rename_prompt(app: &mut App, collection_id: Option<i64>, paper_id: Option<i64>) {
    app.metadata_prompt = Some(MetadataPrompt {
        paper_id: None,
        tag_paper_id: None,
        rename_collection_id: collection_id,
        rename_paper_id: paper_id,
        value: String::new(),
        cursor: 0,
        selected: 0,
        current_collection: None,
        handoff_block_id: None,
    });
    app.mode = AppMode::Prompt;
}

fn apply_library_navigation_action(
    action: UiAction,
    runtime: &mut Runtime,
    _senders: &ActionSenders,
    app: &mut App,
) -> Result<()> {
    match action {
        UiAction::ClosePdf => close_pdf_viewer(runtime, app)?,
        UiAction::OpenAuthor(id) => {
            open_author(&runtime.database, &runtime.library_roots, app, id)?;
        }
        UiAction::OpenLibraryPaper(paper_id) => {
            runtime
                .database
                .record_activity("paper_browsed", Some(paper_id), None)?;
            // For Library PAPER, ensure the selected index matches the opened paper
            if let Some(idx) = app
                .filtered_library_papers()
                .iter()
                .position(|p| p.id == paper_id)
            {
                app.library.selected = idx;
            }
            // PAPER 詳細は「Enter で開いた論文 ID」を保持して表示する。
            // （再読み込み・フィルタ変更で一覧の選択位置が変わっても影響を受けない）
            app.paper_detail_local = Some(paper_id);
            // ローカル論文の PAPER 画面なので、以前のリモート詳細はクリアする。
            app.paper_detail_remote = None;
            app.paper_detail_remote_db_id = None;
            app.paper_detail_remote_ja = None;
            app.mode = AppMode::PaperDetail;
            app.paper_detail_scroll = 0;
            reload_paper_figures(&runtime.database, app);
            refresh_paper_reports(runtime, app);
        }
        UiAction::MarkUnread(paper_id) => {
            runtime.database.mark_unread(paper_id)?;
            refresh_paper_views(runtime, app)?;
            app.toast = Some("Marked unread".into());
        }
        UiAction::OpenHandoff(paper_id) => {
            // S キー: ブロック選択モーダルを開く
            if let Some(paper_roots) = papr_core::paper_roots() {
                match papr_core::list_blocks(&paper_roots) {
                    Ok(blocks) => {
                        if blocks.is_empty() {
                            app.toast = Some("利用可能なブロックがありません".into());
                        } else {
                            app.block_picker = block_picker::BlockPickerState::new(blocks);
                            app.mode = AppMode::BlockPicker;
                            // 選択した論文 ID を保持（後で使う）
                            // ここは簡易版なので、app.paper_detail_local に保持
                            app.paper_detail_local = Some(paper_id);
                        }
                    }
                    Err(_) => {
                        app.toast = Some("ブロック一覧の読み込みに失敗しました".into());
                    }
                }
            } else {
                app.toast = Some("プロジェクトルートが見つかりません".into());
            }
        }
        _ => unreachable!("library navigation action routed to the wrong handler"),
    }
    Ok(())
}

/// PAPER 画面（Library）の図一覧を DB から読み直す。
///
/// 図が未抽出のときは「(図は未抽出)」を出すフラグを立てる。
fn reload_paper_figures(database: &Database, app: &mut App) {
    app.paper_figures.clear();
    app.paper_figures_not_extracted = false;
    let Some(paper_id) = app.paper_detail_local else {
        return;
    };
    match papr_core::figures::figure_state_for_paper(
        database.connection(),
        paper_id,
        database.path_base(),
    ) {
        Ok(papr_core::figures::FigureState::Ready(rows)) => {
            let keep: Vec<String> = rows.iter().map(|r| r.image_path.clone()).collect();
            figure_view::retain_paths(&keep);
            app.paper_figures = rows;
        }
        Ok(papr_core::figures::FigureState::Empty) => {}
        Ok(papr_core::figures::FigureState::NotExtracted) | Err(_) => {
            app.paper_figures_not_extracted = true;
        }
    }
}

/// PAPER 画面で今開いている論文の `papers.id`。
///
/// リモート詳細は `paper_detail_remote_db_id`、ローカル詳細は `paper_detail_local` を返す。
fn current_paper_db_id(app: &App) -> Option<i64> {
    if app.paper_detail_remote.is_some() {
        app.paper_detail_remote_db_id
    } else {
        app.paper_detail_local
    }
}

/// PAPER 画面の REPORTS 欄（最大 5 本）を DB から読み直す。
///
/// 論文が未確定のときは空にする。件数は `issue_counts` を report ごとに数える。
fn refresh_paper_reports(runtime: &Runtime, app: &mut App) {
    app.paper_reports.clear();
    app.paper_report_total = 0;
    let Some(paper_id) = current_paper_db_id(app) else {
        return;
    };
    let conn = runtime.database.connection();
    let Ok(reports) = papr_core::reports::reports_for_paper(conn, paper_id) else {
        return;
    };
    app.paper_report_total = reports.len();
    for report in reports.into_iter().take(5) {
        let counts = papr_core::reports::issue_counts(conn, &report.id).unwrap_or_default();
        app.paper_reports.push((report, counts));
    }
}

/// 1 論文ぶんの重い後処理（図抽出 → 翻訳）をバックグラウンドで起動する。
///
/// 本体は core の `pipeline::after_download`（1 論文の中は順番に、論文どうしは
/// `pipeline.max_parallel` 本まで並べる。済んだステップは飛ばす）。
/// - DL / import は待たせない（別スレッド）。
/// - 同じ論文が実行中なら `running_operations` で二重起動を防ぐ
///   （他の窓口との二重起動は core が `jobs` 表で防ぐ）。
fn spawn_paper_pipeline(
    runtime: &Runtime,
    senders: &ActionSenders,
    app: &mut App,
    paper_id: i64,
    pdf_path: &Path,
) {
    let op_key = format!("pipeline:{paper_id}");
    if app.running_operations.iter().any(|op| op.key == op_key) || !pdf_path.is_file() {
        return;
    }
    let (do_figures, do_translate) =
        papr_core::pipeline::pending_after_download(&runtime.database, paper_id);
    if !do_figures && !do_translate {
        return;
    }
    let title = runtime
        .database
        .library_paper_by_id(paper_id)
        .ok()
        .flatten()
        .map_or_else(|| format!("paper #{paper_id}"), |p| p.title);

    // `PDF取得` はこの時点で完了しているため、1 ステップ目（図抽出）から始める。
    app.running_operations.push(running_op(
        &op_key,
        &title,
        &["PDF取得", "図抽出", "翻訳"],
        1,
    ));

    let db_file = runtime.database_file.clone();
    let config = runtime.config.clone();
    let app_events = senders.app_events.clone();
    std::thread::spawn(move || {
        // Database（rusqlite Connection）は Sync でないため current_thread ランタイムを使う。
        let rt = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(rt) => rt,
            Err(_) => {
                let _ = app_events.send(AppEvent::OperationFinished(op_key));
                return;
            }
        };
        rt.block_on(papr_core::pipeline::after_download(
            &db_file,
            paper_id,
            &config,
            |message| {
                let _ = app_events.send(AppEvent::Toast(message));
                let _ = app_events.send(AppEvent::RefreshLibrary);
            },
            |step| {
                let _ = app_events.send(AppEvent::OperationStep {
                    key: op_key.clone(),
                    step,
                });
            },
        ));
        let _ = app_events.send(AppEvent::OperationFinished(op_key));
        let _ = app_events.send(AppEvent::RefreshLibrary);
    });
}

/// PAPER 画面のスクロール位置を増減する（内容の末尾を超えない）。
fn scroll_paper_detail(app: &mut App, delta: i64) {
    let max = app
        .paper_detail_content_rows
        .saturating_sub(app.paper_detail_view_rows as usize) as i64;
    let current = i64::from(app.paper_detail_scroll);
    app.paper_detail_scroll = current.saturating_add(delta).clamp(0, max) as u16;
}

/// PAPER 画面のスクロール位置を 1 画面ぶん増減する（PageUp / PageDown）。
///
/// 量は表示行数 - 1（最低 1 行）。重なる 1 行を残すので、1 行ずつ読み進められる。
fn scroll_paper_detail_page(app: &mut App, dir: i64) {
    let step = i64::from(app.paper_detail_view_rows.saturating_sub(1)).max(1);
    scroll_paper_detail(app, dir * step);
}

/// 1 回のホイールで画面を 1 行スクロールする。
///
/// PAPER 画面は本文、PAPER 以外の画面は `j` / `k` と同じ動き
/// （Project ページは `app.project.scroll`、その他は `Command::MoveDown` / `MoveUp`）。
fn scroll_by_wheel(app: &mut App, delta: i64) {
    match app.mode {
        AppMode::PaperDetail => scroll_paper_detail(app, delta),
        AppMode::Normal => {
            if app.page == Page::Project {
                // Project ページは本文をそのまま動かす（j / k と同じ 1 回 1 行）。
                app.scroll_project(delta as i32);
            } else if delta > 0 {
                app.dispatch(Command::MoveDown);
            } else if delta < 0 {
                app.dispatch(Command::MoveUp);
            }
        }
        // 入力欄やダイアログでは動かさない。
        _ => {}
    }
}

fn close_pdf_viewer(runtime: &Runtime, app: &mut App) -> Result<()> {
    if let (Some(session_id), Some(start)) =
        (app.active_pdf_session_id, app.active_pdf_session_start)
    {
        runtime
            .database
            .record_reading_duration(session_id, start.elapsed().as_secs())?;
        refresh_dashboard(runtime, app)?;
    }
    app.active_pdf_session_id = None;
    app.active_pdf_session_start = None;
    // md の表示画面から開いたときはその md へ戻す（V06）。それ以外は今までどおり Normal。
    app.mode = app.pdf_view_return;
    app.pdf_view_return = AppMode::Normal;
    if let Some(path) = app.pdf_viewer_path.take() {
        pdf_viewer::release_document(&path);
    }
    app.pdf_viewer_page = 1;
    app.pdf_viewer_total_pages = 1;
    app.pdf_viewer_scroll_y = 0;
    app.pdf_viewer_page_pixel_h = 0;
    app.pdf_viewer_max_scroll_y = 0;
    Ok(())
}

fn apply_citation_action(
    action: UiAction,
    runtime: &mut Runtime,
    senders: &ActionSenders,
    app: &mut App,
) -> Result<()> {
    match action {
        UiAction::CopyCitation(target) => {
            let metadata = citation_metadata(runtime, target)?;
            if let Some(metadata) = metadata {
                app.toast = Some("Fetching citation...".into());
                tokio::spawn(citation::fetch_and_copy_citation(
                    metadata,
                    senders.app_events.clone(),
                ));
            } else {
                app.toast = Some("Citation metadata not available".into());
            }
        }
        _ => unreachable!("citation action routed to the wrong handler"),
    }
    Ok(())
}

fn citation_metadata(
    runtime: &mut Runtime,
    target: PaperTarget,
) -> Result<Option<papr_core::models::CitationMetadata>> {
    match target {
        PaperTarget::Local(id) => runtime
            .database
            .paper_citation_metadata(id)
            .map_err(Into::into),
        PaperTarget::Remote(paper) => Ok(Some(remote_citation_metadata(*paper))),
    }
}

fn remote_citation_metadata(paper: RemotePaper) -> papr_core::models::CitationMetadata {
    papr_core::models::CitationMetadata {
        title: paper.title,
        authors: paper.authors.join(" and "),
        doi: paper.doi,
        arxiv_id: Some(paper.id),
        year: Some(
            paper
                .published
                .with_timezone(&chrono::Local)
                .format("%Y")
                .to_string(),
        ),
        journal_ref: paper.journal_ref,
    }
}

fn apply_deletion_action(action: UiAction, runtime: &mut Runtime, app: &mut App) -> Result<()> {
    match action {
        UiAction::ConfirmDeletePaper {
            paper_id,
            title,
            path,
        } => {
            app.delete_confirmation = Some(DeletionTarget::Paper {
                id: paper_id,
                title,
                path: path.map(PathBuf::from),
            });
            app.mode = AppMode::ConfirmDelete;
        }
        UiAction::DeletePaper { paper_id, path } => {
            delete_paper(runtime, app, paper_id, path.map(PathBuf::from))?
        }
        _ => unreachable!("deletion action routed to the wrong handler"),
    }
    Ok(())
}

fn delete_paper(
    runtime: &mut Runtime,
    app: &mut App,
    paper_id: i64,
    fallback_path: Option<PathBuf>,
) -> Result<()> {
    let database_path = runtime
        .database
        .library_paper_by_id(paper_id)?
        .and_then(|paper| paper.pdf_path)
        .map(PathBuf::from);
    if let Some(path) = database_path.or(fallback_path).filter(|path| path.exists()) {
        std::fs::remove_file(&path)
            .with_context(|| format!("failed to delete PDF at {}", path.display()))?;
    }
    runtime.database.delete_paper(paper_id)?;
    app.downloads.retain(|task| task.paper_id != Some(paper_id));
    refresh_after_library_change(runtime, app)?;
    app.toast = Some("PDF permanently deleted".into());
    Ok(())
}

fn rename_paper_pdf(runtime: &mut Runtime, app: &mut App, paper_id: i64, name: &str) -> Result<()> {
    if name.contains(['/', '\\']) {
        anyhow::bail!("filename must not contain path separators");
    }
    let new_name = if name.to_lowercase().ends_with(".pdf") {
        name.to_owned()
    } else {
        format!("{name}.pdf")
    };
    let paper = app
        .library
        .papers
        .iter()
        .find(|paper| paper.id == paper_id)
        .context("paper not found")?;
    let source = PathBuf::from(paper.pdf_path.as_ref().context("paper has no local PDF")?);
    let destination = source.with_file_name(new_name);
    if source == destination {
        return Ok(());
    }
    if destination.exists() {
        anyhow::bail!("a file with this name already exists");
    }
    if let Some(task) = app
        .downloads
        .iter_mut()
        .find(|task| task.paper_id == Some(paper_id))
    {
        task.status = DownloadStatus::Renaming;
    }
    move_pdf_file(&source, &destination)?;
    runtime.database.rename_pdf(paper_id, &destination)?;
    if let Some(task) = app
        .downloads
        .iter_mut()
        .find(|task| task.paper_id == Some(paper_id))
    {
        task.pdf_path = Some(destination.to_string_lossy().into_owned());
        task.status = DownloadStatus::Completed;
    }
    refresh_after_library_change(runtime, app)
}

fn refresh_after_library_change(runtime: &mut Runtime, app: &mut App) -> Result<()> {
    refresh_library(runtime, app)?;
    refresh_organization(&runtime.database, &runtime.library_roots, app)?;
    refresh_dashboard(runtime, app)?;
    refresh_downloads(runtime, app);
    Ok(())
}

fn open_author(
    database: &Database,
    library_roots: &[PathBuf],
    app: &mut App,
    author_id: i64,
) -> Result<()> {
    let restore_selection = app.last_opened_author_id == Some(author_id);
    app.active_author = app
        .authors
        .iter()
        .find(|author| author.id == author_id)
        .cloned();
    app.author_papers = database.author_papers(author_id, library_roots)?;
    app.author_paper_selected = if restore_selection {
        app.author_paper_selected
            .min(app.author_papers.len().saturating_sub(1))
    } else {
        0
    };
    app.last_opened_author_id = Some(author_id);
    Ok(())
}

fn add_paper_tag_with_disk(
    runtime: &Runtime,
    _app: &mut App,
    paper_id: i64,
    name: &str,
) -> Result<bool> {
    let name = name.trim();
    if name.is_empty() {
        return Ok(false);
    }
    runtime.database.add_paper_tag(paper_id, name)?;
    Ok(true)
}

async fn dispatch_plugin_events(
    runtime: &Runtime,
    app: &mut App,
    events: &[&str],
    paper_id: i64,
) -> Result<()> {
    let Some(paper) = runtime.database.library_paper_by_id(paper_id)? else {
        return Ok(());
    };

    let paper_json = serde_json::json!({
        "id": paper.id,
        "title": paper.title,
        "authors": paper.authors,
        "doi": paper.doi,
        "arxiv_id": paper.arxiv_id,
        "pdf_path": paper.pdf_path,
        "reading_status": paper.reading_status,
        "is_favorite": paper.is_favorite,
    });

    let enabled_plugins = runtime.plugin_host.plugins();
    let mut organization_dirty = false;
    let mut last_notify = None;

    for plugin in enabled_plugins {
        if !plugin.enabled {
            continue;
        }

        for &event_name in events {
            let request = papr_core::PluginRequest::new(
                event_name,
                serde_json::json!({
                    "paper_id": paper.id,
                    "paper": paper_json.clone(),
                }),
            );

            match runtime
                .plugin_host
                .invoke(&plugin.id, &request, std::time::Duration::from_secs(5))
                .await
            {
                Ok(response) => {
                    for action in response.actions {
                        match action {
                            papr_core::PluginAction::Message { message } => {
                                last_notify = Some(message);
                            }
                            papr_core::PluginAction::AddToCollection { name } => {
                                match add_paper_tag_with_disk(runtime, app, paper.id, &name) {
                                    Ok(changed) => {
                                        if changed {
                                            organization_dirty = true;
                                        }
                                    }
                                    Err(err) => {
                                        eprintln!(
                                            "Failed to add paper to collection '{name}': {err}"
                                        );
                                    }
                                }
                            }
                        }
                    }
                }
                Err(err) => {
                    eprintln!("Plugin '{}' invocation failed: {err}", plugin.id);
                }
            }
        }
    }

    if organization_dirty {
        refresh_organization(&runtime.database, &runtime.library_roots, app)?;
        refresh_library(runtime, app)?;
        if let Some(msg) = last_notify {
            app.toast = Some(msg);
        }
    }

    Ok(())
}

fn resolve_target(target: PaperTarget, database: &mut Database) -> Result<i64> {
    match target {
        PaperTarget::Local(id) => Ok(id),
        PaperTarget::Remote(paper) => database.ensure_remote_paper(&paper).map_err(Into::into),
    }
}

fn refresh_organization(
    database: &Database,
    library_roots: &[PathBuf],
    app: &mut App,
) -> Result<()> {
    // Collections / Bookmarks / Reading Queue は廃止（仕様 §3.1 / §3.5）。
    // タグ一覧を sidebar 用に読み込む。
    app.tags = database.tags()?;
    app.tag_selected = app.tag_selected.min(app.tags.len().saturating_sub(1));
    // R2: タグ → 論文一覧を開いている場合は、タグ更新直後に一覧も最新化する。
    if let Some(tag) = app.active_tag.clone() {
        app.tag_papers = database.papers_for_tag_name(&tag.name)?;
        app.tag_paper_selected = app
            .tag_paper_selected
            .min(app.tag_papers.len().saturating_sub(1));
    }
    app.authors = database.authors(library_roots)?;
    app.author_selected = app.author_selected.min(app.authors.len().saturating_sub(1));
    if let Some(active_id) = app.active_author.as_ref().map(|a| a.id) {
        app.active_author = app.authors.iter().find(|a| a.id == active_id).cloned();
        if app.active_author.is_some() {
            app.author_papers = database.author_papers(active_id, library_roots)?;
            app.author_paper_selected = app
                .author_paper_selected
                .min(app.author_papers.len().saturating_sub(1));
        } else {
            app.author_papers.clear();
            app.author_paper_selected = 0;
        }
    }
    app.notes_papers = database.papers_with_notes(library_roots)?;
    app.notes_selected = app
        .notes_selected
        .min(app.notes_papers.len().saturating_sub(1));
    Ok(())
}

fn refresh_dashboard_papers(
    runtime: &mut Runtime,
    senders: &ActionSenders,
    app: &mut App,
) -> Result<()> {
    if let Some(papers) = runtime.database.dashboard_feed_cache(
        &runtime.dashboard_feed_date,
        &runtime.dashboard_keyword_signature,
    )? {
        app.today_papers = papers;
        app.today_selected = app
            .today_selected
            .min(app.today_papers.len().saturating_sub(1));
        app.today_status = DiscoveryStatus::Ready;
        runtime.active_dashboard_fetch = None;
        return Ok(());
    }
    let key = DashboardFeedKey {
        feed_date: runtime.dashboard_feed_date.clone(),
        keyword_signature: runtime.dashboard_keyword_signature.clone(),
    };
    app.today_status = DiscoveryStatus::Loading;
    if runtime.active_dashboard_fetch.as_ref() == Some(&key) {
        return Ok(());
    }
    start_dashboard_fetch(
        runtime.arxiv.clone(),
        runtime.dashboard_keywords.clone(),
        dashboard_recent_paper_ids(&runtime.database, &runtime.dashboard_feed_date)?,
        key.clone(),
        senders.today.clone(),
    );
    runtime.active_dashboard_fetch = Some(key);
    Ok(())
}

fn start_dashboard_fetch(
    client: ArxivClient,
    keywords: Vec<String>,
    excluded_paper_ids: HashSet<String>,
    key: DashboardFeedKey,
    sender: mpsc::UnboundedSender<TodayResponse>,
) {
    tokio::spawn(async move {
        let result = DashboardService::new(DASHBOARD_CANDIDATE_LIMIT, DASHBOARD_DISPLAY_LIMIT)
            .fetch(client, keywords, excluded_paper_ids, &key.feed_date)
            .await
            .map_err(|error| error.to_string());
        let _ = sender.send(TodayResponse { key, result });
    });
}

fn local_feed_date() -> String {
    Local::now().date_naive().format("%Y-%m-%d").to_string()
}

fn dashboard_keyword_signature(keywords: &[String]) -> String {
    format!("{DASHBOARD_FEED_ALGORITHM_VERSION}:{}", keywords.join(","))
}

fn dashboard_recent_paper_ids(database: &Database, feed_date: &str) -> Result<HashSet<String>> {
    let feed_date = NaiveDate::parse_from_str(feed_date, "%Y-%m-%d")
        .unwrap_or_else(|_| Local::now().date_naive());
    let cutoff = feed_date - Duration::days(DASHBOARD_REPEAT_EXCLUSION_DAYS);
    Ok(database.dashboard_paper_ids_since(&cutoff.to_string())?)
}

// Deterministically order one keyword's eligible papers for a local day.

fn refresh_library(runtime: &Runtime, app: &mut App) -> Result<()> {
    app.library.papers = runtime
        .database
        .library_papers_in_roots(&runtime.library_roots)?;
    if app.library.selected >= app.library.papers.len() {
        app.library.selected = app.library.papers.len().saturating_sub(1);
    }
    app.library_tree_dirty = true;
    Ok(())
}

/// 階層表示の再構築が必要なら実行する（Library ページ・階層モード時のみ）。
fn rebuild_library_tree_if_dirty(runtime: &Runtime, app: &mut App) -> bool {
    if !app.library_tree_dirty
        || app.library_view != LibraryView::Hierarchy
        || app.page != Page::Library
    {
        return false;
    }
    rebuild_library_tree(runtime, app);
    true
}

/// 階層表示の行を再構築する。
///
/// まとめ方（検索ログごと / 日付ごとの Import）は core の `library_tree::build_groups` に
/// 共通化し、ここでは結果を表示行（ラベル・折りたたみ状態）へ変換するだけにする。
///
/// - 折りたたまれている query/import 行の子は出力しない。
fn rebuild_library_tree(runtime: &Runtime, app: &mut App) {
    app.library_tree_dirty = false;
    let papers = app.library.papers.clone();
    let conn = runtime.database.connection();

    let groups = papr_core::library_tree::build_groups(conn, &papers);
    // 論文 ID → app.library.papers の添字（paper_ids から行を引くため）。
    let index_of: HashMap<i64, usize> = papers
        .iter()
        .enumerate()
        .map(|(index, paper)| (paper.id, index))
        .collect();

    let mut rows: Vec<LibraryTreeRow> = Vec::new();
    for group in &groups {
        let (kind, fold_key, label, log_id) = match group.kind {
            papr_core::library_tree::GroupKind::Query => (
                LibraryTreeKind::Query,
                format!("log:{}", group.log_id.unwrap_or_default()),
                format!("{}  {}", group.date, group.query.as_deref().unwrap_or("")),
                group.log_id,
            ),
            papr_core::library_tree::GroupKind::Import => (
                LibraryTreeKind::Import,
                format!("import:{}", group.date),
                format!("{}  Import", group.date),
                None,
            ),
        };
        let folded = app.library_tree_folded.contains(&fold_key);
        rows.push(LibraryTreeRow {
            depth: 0,
            kind,
            label,
            child_count: group.paper_ids.len(),
            fold_key: Some(fold_key),
            log_id,
            paper_id: None,
            year_month: String::new(),
            citation_count: None,
            journal: None,
            title: String::new(),
        });
        if !folded {
            for paper_id in &group.paper_ids {
                if let Some(index) = index_of.get(paper_id) {
                    rows.push(tree_paper_row(&papers[*index]));
                }
            }
        }
    }

    app.library_tree = rows;
    if app.library_tree_selected >= app.library_tree.len() {
        app.library_tree_selected = app.library_tree.len().saturating_sub(1);
    }
}

/// 階層表示の論文行を作る。
fn tree_paper_row(paper: &papr_core::LibraryPaper) -> LibraryTreeRow {
    LibraryTreeRow {
        depth: 1,
        kind: LibraryTreeKind::Paper,
        label: String::new(),
        child_count: 0,
        fold_key: None,
        log_id: None,
        paper_id: Some(paper.id),
        year_month: ui::published_year_month(paper.published_at.as_deref()),
        citation_count: paper.citation_count,
        journal: paper.journal.clone(),
        title: if paper.title.trim().is_empty() {
            paper.display_name().to_owned()
        } else {
            paper.title.clone()
        },
    }
}

fn refresh_dashboard(runtime: &Runtime, app: &mut App) -> Result<()> {
    app.dashboard = runtime.database.research_dashboard()?;
    app.dashboard.counts.papers = LibraryIndexer::count_pdfs(&runtime.collection_roots);
    app.dashboard.counts.downloaded =
        LibraryIndexer::count_pdfs(std::slice::from_ref(&runtime.download_dir));
    app.dashboard.read = runtime
        .database
        .library_papers_in_roots(&runtime.library_roots)?
        .into_iter()
        .filter(|p| p.reading_status == "read")
        .count() as u64;
    app.dashboard.disk_usage = LibraryIndexer::pdf_storage_size(&runtime.collection_roots);
    app.dashboard.downloads_size =
        LibraryIndexer::pdf_storage_size(std::slice::from_ref(&runtime.download_dir));
    app.dashboard.database_size = std::fs::metadata(&runtime.database_file).map_or(0, |m| m.len());
    app.stats = app.dashboard.counts;
    // RECENT ACTIVITY に Radio の出来事（聴いた・質問した）を混ぜる
    papr_core::podcast_client::merge_radio_activity(
        &mut app.dashboard.recent_activity,
        &runtime.config.radio,
        runtime.database.connection(),
    );
    // Project ページ: 現況・最新の引き継ぎ・最近のレビュー。
    // 読めなくても画面は止めず、空のままにする。
    let project_root = runtime
        .data_dir
        .parent()
        .and_then(Path::parent)
        .map(Path::to_path_buf);
    let state_file = papr_core::paths::state_path(&runtime.data_dir);
    // 出典の問題の件数と report の総数（Project ページの行用）。
    let citation_issues = {
        let conn = runtime.database.connection();
        papr_core::total_issue_counts(conn).unwrap_or_default()
    };
    let report_count = {
        let conn = runtime.database.connection();
        papr_core::list_reports(conn, None).map_or(0, |rows| rows.len())
    };
    // 理解の状態（トピックごとの最新）。読めなければ空のまま。
    let understanding = {
        let conn = runtime.database.connection();
        papr_core::understanding::latest_per_topic(conn).unwrap_or_default()
    };
    // 読むべき論文（READ NEXT）。進み具合を集めて点数順に 5 本。読めなければ空。
    let read_next = match project_root.as_deref() {
        Some(root) => {
            let roots = papr_core::PaperRoots {
                root: root.to_path_buf(),
                data_dir: runtime.data_dir.clone(),
            };
            let conn = runtime.database.connection();
            papr_core::progress::collect(conn, &roots).map_or_else(
                |_| Vec::new(),
                |progress| {
                    papr_core::progress::read_next(
                        &progress,
                        &runtime.config.progress.read_next,
                        5,
                    )
                },
            )
        }
        None => Vec::new(),
    };
    let read_next_selected = app
        .project
        .read_next_selected
        .min(read_next.len().saturating_sub(1));
    // 要対応の一覧（S02）。読めなければ空のまま。
    let attention = match project_root.as_deref() {
        Some(root) => {
            let roots = papr_core::PaperRoots {
                root: root.to_path_buf(),
                data_dir: runtime.data_dir.clone(),
            };
            let conn = runtime.database.connection();
            papr_core::attention::collect(conn, &roots).unwrap_or_default()
        }
        None => Vec::new(),
    };
    let attention_selected = app
        .attention_selected
        .min(attention.len().saturating_sub(1));
    let mut project = ProjectView {
        state: papr_core::project_files::read_state(&runtime.data_dir),
        state_exists: state_file.is_file(),
        latest_handoff: None,
        recent_reviews: Vec::new(),
        understanding,
        read_next,
        read_next_selected,
        citation_issues,
        report_count,
        attention,
        scroll: app.project.scroll,
    };
    if let Some(root) = project_root.as_deref() {
        project.latest_handoff = papr_core::brief::recent_handoffs(root, 1).into_iter().next();
        project.recent_reviews = papr_core::brief::recent_reviews(root, 2);
    }
    app.project = project;
    app.attention_selected = attention_selected;
    Ok(())
}

fn refresh_downloads_from_dir(app: &mut App, _download_dir: &Path, _database: &Database) {
    // 完了済みタスクを掃除し、進行中タスク（Downloads ページ廃止後も
    // ステータス管理に使用）のみを保持する。
    let mut transient_downloads = app
        .downloads
        .iter()
        .filter(|task| !matches!(task.status, DownloadStatus::Completed))
        .cloned()
        .collect::<Vec<_>>();

    app.downloads.clear();
    app.downloads.append(&mut transient_downloads);
}

fn refresh_downloads(runtime: &Runtime, app: &mut App) {
    refresh_downloads_from_dir(app, &runtime.download_dir, &runtime.database);
}

fn refresh_paper_views(runtime: &Runtime, app: &mut App) -> Result<()> {
    refresh_library(runtime, app)?;
    refresh_organization(&runtime.database, &runtime.library_roots, app)?;
    refresh_dashboard(runtime, app)?;
    refresh_downloads(runtime, app);
    Ok(())
}

fn default_pdf_viewer() -> String {
    if cfg!(target_os = "macos") {
        "open".into()
    } else if cfg!(target_os = "windows") {
        "cmd /C start msedge \"\"".into()
    } else {
        "xdg-open".into()
    }
}

/// Scroll the PDF viewer by `delta` rows.
fn pdf_scroll(app: &mut App, delta: i64) {
    pdf_viewer::scroll_by_rows(app, delta);
}

fn open_pdf(
    viewer: &str,
    path: &Path,
    app: &mut App,
    session_id: Option<i64>,
    event_sender: Option<mpsc::UnboundedSender<AppEvent>>,
    page: Option<usize>,
) -> Result<()> {
    if !path.exists() {
        app.toast = Some(format!("PDF not found: {}", path.display()));
        return Ok(());
    }

    let absolute_path = canonicalize_path(path).unwrap_or_else(|_| path.to_path_buf());
    let path = &absolute_path;

    if viewer == "internal" {
        // Flush any cached images / protocol state that belong to a different
        // document.  This is the single place that guarantees the cache is
        // always consistent with whatever path is about to be stored in
        // `app.pdf_viewer_path`.
        pdf_viewer::reset_for_new_document(path);
        // md の表示画面から開いたときは、閉じたあと元の md に戻す（V06）。
        if app.mode == AppMode::MarkdownView {
            app.pdf_view_return = AppMode::MarkdownView;
        }
        app.mode = AppMode::PdfView;
        app.pdf_viewer_path = Some(path.clone());
        app.pdf_viewer_scroll_y = 0;
        app.pdf_viewer_page_pixel_h = 0;
        app.pdf_viewer_max_scroll_y = 0;
        app.pdf_viewer_total_pages = pdf_viewer::page_count(path).unwrap_or(1);
        // 引用の `p.5` など、開くページを指定されたときはそのページにする。
        app.pdf_viewer_page = page
            .unwrap_or(1)
            .max(1)
            .min(app.pdf_viewer_total_pages.max(1));
        app.active_pdf_session_id = session_id;
        app.active_pdf_session_start = Some(std::time::Instant::now());
        app.toast = Some(format!(
            "Viewing PDF: {}",
            path.file_name().unwrap_or_default().to_string_lossy()
        ));
        return Ok(());
    }

    let (program, argv) = pdf_viewer_invocation(viewer, path)?;
    let mut command = tokio::process::Command::new(&program);
    command.args(argv);

    command.stdout(Stdio::null());
    command.stderr(Stdio::null());

    match command.spawn() {
        Ok(mut child) => {
            let basename = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            let display_name = if basename.chars().count() > 100 {
                let extension = path
                    .extension()
                    .map(|e| e.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let ext_len = if extension.is_empty() {
                    0
                } else {
                    extension.chars().count() + 1
                };
                let stem = path
                    .file_stem()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let take_len = 100_usize.saturating_sub(ext_len).saturating_sub(1);
                let truncated_stem: String = stem.chars().take(take_len).collect();
                if extension.is_empty() {
                    format!("{truncated_stem}…")
                } else {
                    format!("{truncated_stem}….{extension}")
                }
            } else {
                basename
            };
            // 外のビューアはページを指定できない。指定されたときは何ページ目を見るか伝える。
            let page_hint = match page {
                Some(n) if n > 1 => format!("（{n} ページ目を見てください）"),
                _ => String::new(),
            };
            app.toast = Some(format!("Opened PDF: {display_name}{page_hint}"));
            if let (Some(session_id), Some(sender)) = (session_id, event_sender) {
                let start = std::time::Instant::now();
                tokio::spawn(async move {
                    let _ = child.wait().await;
                    let duration_s = start.elapsed().as_secs();
                    let _ = sender.send(AppEvent::ReadingSessionCompleted {
                        session_id,
                        duration_s,
                    });
                });
            }
        }
        Err(error) => app.toast = Some(format!("Could not open PDF with {program}: {error}")),
    }
    Ok(())
}

/// Construct an external viewer invocation without a shell. This keeps
/// configured programs such as `xdg-open` as ordinary executables and passes
/// the PDF as one OS-native argument, including paths containing spaces.
fn pdf_viewer_invocation(viewer: &str, path: &Path) -> Result<(String, Vec<OsString>)> {
    let mut argv = parse_command(viewer)?;
    if argv.is_empty() {
        argv.push(default_pdf_viewer());
    }
    let has_placeholder = argv.iter().any(|arg| arg.contains("{path}"));
    if has_placeholder {
        let path_text = path.to_string_lossy();
        for arg in &mut argv {
            *arg = arg.replace("{path}", &path_text);
        }
    }

    let program = argv.remove(0);
    #[cfg(target_os = "windows")]
    let (program, argv) = {
        let mut argv = argv;
        if program.eq_ignore_ascii_case("start") {
            argv.insert(0, program);
            if argv.len() == 1 {
                // `start` treats its first quoted argument as a window title.
                argv.push(String::new());
            }
            argv.insert(0, "/C".to_owned());
            ("cmd".to_owned(), argv)
        } else {
            (program, argv)
        }
    };

    let mut command_args = argv.into_iter().map(OsString::from).collect::<Vec<_>>();
    if !has_placeholder {
        command_args.push(path.as_os_str().to_owned());
    }
    Ok((program, command_args))
}

fn open_browser(url: &str, app: &mut App) {
    let mut command = if cfg!(target_os = "macos") {
        let mut command = ProcessCommand::new("open");
        command.arg(url);
        command
    } else if cfg!(target_os = "windows") {
        let mut command = ProcessCommand::new("cmd");
        command.args(["/C", "start", "", url]);
        command
    } else {
        let mut command = ProcessCommand::new("xdg-open");
        command.arg(url);
        command
    };
    command.stdout(Stdio::null()).stderr(Stdio::null());
    app.toast = Some(match command.spawn() {
        Ok(_) => "Opened paper in browser".into(),
        Err(error) => format!("Could not open browser: {error}"),
    });
}

fn start_scan(
    pdf_roots: &[PathBuf],
    collection_roots: &[PathBuf],
    sender: &mpsc::UnboundedSender<IndexResponse>,
    app: &mut App,
    silent: bool,
) {
    if app.library.indexing {
        return;
    }
    app.library.indexing = !silent;
    if !silent {
        app.library.message = Some("Indexing library folders...".into());
    }
    let pdf_roots = pdf_roots.to_vec();
    let collection_roots = collection_roots.to_vec();
    let sender = sender.clone();
    tokio::task::spawn_blocking(move || {
        let _ = sender.send(IndexResponse::Scan {
            pdfs: LibraryIndexer::scan(&pdf_roots),
            directories: LibraryIndexer::collection_directories(&collection_roots),
        });
    });
}

fn log_message(database_file: &Path, message: &str) {
    if let Some(parent) = database_file.parent() {
        let log_file = parent.join("papr.log");
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(log_file)
        {
            use std::io::Write;
            let timestamp = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");
            let _ = writeln!(file, "[{timestamp}] {message}");
        }
    }
}

/// Project enriched metadata onto a paper already shown by the UI.
///
/// Database enrichment deliberately retains the stored abstract when a provider
/// has none. The UI must apply that same merge before it persists the dashboard
/// cache; replacing the object wholesale would otherwise make a valid abstract
/// disappear from the UI even though it remains in `SQLite`.
fn merge_enriched_remote_paper(current: &RemotePaper, enriched: &RemotePaper) -> RemotePaper {
    let mut merged = enriched.clone();
    if merged.abstract_text.trim().is_empty() {
        merged.abstract_text.clone_from(&current.abstract_text);
    }
    // Crossref 由来の abstract は取得時点で整形済みだが、念のため表示前にも整形する。
    merged.abstract_text = papr_core::sanitize_abstract(&merged.abstract_text);
    merged
}

fn start_library_watcher(
    roots: &[PathBuf],
    watch_sender: mpsc::UnboundedSender<()>,
) -> Result<LibraryWatcher> {
    LibraryWatcher::start(roots, move || {
        let _ = watch_sender.send(());
    })
    .context("failed to watch library folders")
}

fn restart_runtime_watcher(runtime: &mut Runtime) -> Result<()> {
    runtime.watcher = start_library_watcher(&runtime.library_roots, runtime.watch_sender.clone())?;
    Ok(())
}

fn spawn_enrichment_if_needed(
    runtime: &mut Runtime,
    senders: &ActionSenders,
    app: &mut App,
) -> Result<()> {
    let all_papers = runtime.database.papers_needing_enrichment_with_doi()?;
    let papers: Vec<_> = all_papers
        .into_iter()
        .filter(|(pid, _, _, _)| !runtime.active_enrichments.contains(pid))
        .collect();
    for task in &mut app.downloads {
        if task.status == DownloadStatus::ExtractingMetadata {
            let needs_enrichment = papers
                .iter()
                .any(|(pid, _, _, _)| Some(*pid) == task.paper_id);
            if !needs_enrichment {
                task.status = DownloadStatus::Completed;
                finalize_download_task(task);
            }
        }
    }
    if !papers.is_empty() {
        for (paper_id, _, _, _) in &papers {
            runtime.active_enrichments.insert(*paper_id);
            if let Some(task) = app
                .downloads
                .iter_mut()
                .find(|t| t.paper_id == Some(*paper_id))
            {
                task.status = DownloadStatus::Enriching;
            }
        }
        let metadata_enrichment = runtime.metadata_enrichment.clone();
        let enrichment_tx = senders.enrichment.clone();
        app.session_flags.enrichment_pending = true;
        let db_file_log = runtime.database_file.clone();
        // arXiv asks API clients to pause between requests; serial enrichment
        // prevents background metadata work from monopolizing the shared client.
        let concurrency = Arc::new(Semaphore::new(METADATA_ENRICHMENT_CONCURRENCY));
        tokio::spawn(async move {
            let mut jobs = JoinSet::new();
            for (paper_id, candidate_arxiv, candidate_doi, pdf_path) in papers {
                let metadata_enrichment = metadata_enrichment.clone();
                let enrichment_tx = enrichment_tx.clone();
                let db_file_log = db_file_log.clone();
                let permit = concurrency.clone();
                jobs.spawn(async move {
                    let permit_guard = match permit.acquire_owned().await {
                        Ok(permit_guard) => permit_guard,
                        Err(error) => {
                            let message = format!("metadata enrichment queue closed: {error}");
                            log_message(&db_file_log, &message);
                            let _ = enrichment_tx.send(MetadataEnrichment {
                                paper_id,
                                doi: None,
                                retraction: None,
                                outcome: MetadataEnrichmentOutcome::Failed(message),
                            });
                            return;
                        }
                    };
                    let doi = candidate_doi.clone();
                    let (outcome, retraction) = metadata_enrichment
                        .enrich_with_status(MetadataCandidate {
                            arxiv_id: candidate_arxiv,
                            doi: candidate_doi,
                            pdf_path: pdf_path.map(PathBuf::from),
                        })
                        .await;
                    if let MetadataEnrichmentOutcome::Failed(error) = &outcome {
                        log_message(&db_file_log, error);
                    }
                    let _ = enrichment_tx.send(MetadataEnrichment {
                        paper_id,
                        doi,
                        retraction,
                        outcome,
                    });
                    drop(permit_guard);
                });
            }
            while let Some(result) = jobs.join_next().await {
                if let Err(error) = result {
                    log_message(
                        &db_file_log,
                        &format!("Metadata enrichment task failed: {error}"),
                    );
                }
            }
        });
    }
    Ok(())
}

fn start_runtime_scan(runtime: &Runtime, senders: &ActionSenders, app: &mut App) {
    start_scan(
        &runtime.library_roots,
        &runtime.collection_roots,
        &senders.index,
        app,
        false,
    );
}

fn start_silent_runtime_scan(runtime: &Runtime, senders: &ActionSenders, app: &mut App) {
    start_scan(
        &runtime.library_roots,
        &runtime.collection_roots,
        &senders.index,
        app,
        true,
    );
}

fn library_index_summary(indexed: usize, imported: usize) -> String {
    format!("Indexed {indexed} PDFs, imported {imported} new")
}

async fn apply_index_response(
    response: IndexResponse,
    runtime: &mut Runtime,
    senders: &ActionSenders,
    app: &mut App,
) -> Result<()> {
    match response {
        IndexResponse::Scan { pdfs, directories } => {
            let result = LibraryIngestionService::new(&runtime.database)
                .ingest_scan(pdfs, &directories)?;
            let imported = result
                .papers
                .iter()
                .filter(|paper| paper.newly_imported)
                .count();
            for paper in result.papers {
                if paper.newly_imported
                    && let Some(paper_id) = paper.paper_id
                {
                    dispatch_plugin_events(runtime, app, &["paper_imported"], paper_id).await?;
                }
            }
            app.library.indexing = false;
            app.library.message = Some(library_index_summary(result.found, imported));

            spawn_enrichment_if_needed(runtime, senders, app)?;
        }
    }
    refresh_library(runtime, app)?;
    refresh_organization(&runtime.database, &runtime.library_roots, app)?;
    refresh_dashboard(runtime, app)?;
    refresh_downloads(runtime, app);
    Ok(())
}

fn start_download(
    paper: RemotePaper,
    directory: &std::path::Path,
    manager: &DownloadManager,
    events: &mpsc::UnboundedSender<DownloadEvent>,
    pending: &mut HashMap<String, RemotePaper>,
    app: &mut App,
) {
    let Some(url) = paper.pdf_url.clone().filter(|u| !u.trim().is_empty()) else {
        app.toast = Some(
            "No PDF URL. Press o to open the site, save the PDF, then press i to import."
                .to_owned(),
        );
        return;
    };
    if app.downloaded_remote_paper(&paper).is_some() {
        app.toast = Some("Paper already available. Skipping download...".to_owned());
        return;
    }
    if pending.contains_key(&paper.id) {
        return;
    }
    let destination = papr_core::downloads::download_destination(directory, &paper);
    let id = paper.id.clone();
    pending.insert(id.clone(), paper.clone());
    app.toast = Some("Downloading paper...".to_owned());
    app.downloads.push(DownloadTask {
        id: id.clone(),
        title: paper.title.clone(),
        downloaded: 0,
        total: None,
        paper_id: None,
        pdf_path: Some(destination.to_string_lossy().into_owned()),
        status: DownloadStatus::Starting,
        remote_paper: Some(paper),
        failed_at: None,
    });
    let manager = manager.clone();
    let events = events.clone();
    tokio::spawn(async move {
        if let Err(error) = manager.download(&id, &url, &destination, &events).await {
            let _ = events.send(DownloadEvent::Failed {
                id,
                error: error.to_string(),
            });
        }
    });
}

async fn apply_download_event(
    event: DownloadEvent,
    pending: &mut HashMap<String, RemotePaper>,
    runtime: &mut Runtime,
    app: &mut App,
    senders: &ActionSenders,
) -> Result<()> {
    let is_completed = matches!(event, DownloadEvent::Completed { .. });
    let id = match &event {
        DownloadEvent::Started { id, .. }
        | DownloadEvent::Progress { id, .. }
        | DownloadEvent::Completed { id, .. }
        | DownloadEvent::Failed { id, .. } => id.clone(),
    };
    let task = app
        .downloads
        .iter_mut()
        .find(|t| t.id == id)
        .context("received download event for unknown task")?;
    let mut downloaded_paper_was_already_indexed = false;
    // Discover の検索の記録への紐付けは `app` 全体を可変に借りたいので、
    // `task`（`app.downloads`）の借用が終わったあとで行う。
    let mut discover_link: Option<(RemotePaper, i64)> = None;
    match event {
        DownloadEvent::Started { total, .. } => {
            task.status = DownloadStatus::Running;
            task.total = total;
        }
        DownloadEvent::Progress { downloaded, .. } => {
            task.status = DownloadStatus::Running;
            task.downloaded = downloaded;
        }
        DownloadEvent::Completed { id, path } => {
            task.status = DownloadStatus::ExtractingMetadata;
            // 一時ファイル（.pdf.part）を .pdf にして Library に登録する処理は
            // core の `register_downloaded_pdf`（`sai-moon-paper phone download` と共通）。
            // 検索セットへの紐付け・通知・要旨の訳・パイプラインは TUI 側のまま。
            let mut downloaded_paper: Option<RemotePaper> = None;
            let mut pdf_moved_to = path.with_extension("");
            let pdf_file_size;
            if let Some(paper) = pending.remove(&id) {
                downloaded_paper = Some(paper.clone());
                let registered = papr_core::downloads::register_downloaded_pdf(
                    &mut runtime.database,
                    &paper,
                    &path,
                    &runtime.data_dir,
                )?;
                downloaded_paper_was_already_indexed = app
                    .library
                    .papers
                    .iter()
                    .any(|library_paper| library_paper.id == registered.paper_id);
                task.paper_id = Some(registered.paper_id);
                pdf_moved_to = registered.path;
                pdf_file_size = registered.file_size;

                let paper_id = registered.paper_id;
                // Link paper to search set if download came from SearchSets page
                if let Some((set_id, position)) = app.search_set_download_context.remove(&id) {
                    let conn = runtime.database.connection();
                    if let Err(e) =
                        papr_core::search_sets::link_paper(conn, set_id, position, paper_id)
                    {
                        app.toast = Some(format!("検索セットへの紐付けに失敗: {e}"));
                    }
                } else {
                    // Discover（または Dashboard）で取り込んだ場合は、いまの Discover の
                    // 検索の記録に紐付ける（Library の階層表示用）。
                    discover_link = Some((paper, paper_id));
                }
            } else {
                // `pending` に無いとき（再 Download など）は登録しない。
                // 一時ファイルを .pdf に変えて大きさを測るだけにする。
                if path.exists() {
                    std::fs::rename(&path, &pdf_moved_to)
                        .context("failed to promote temporary download path")?;
                }
                let pdf = LibraryIndexer::inspect(&pdf_moved_to)
                    .context("failed to index downloaded PDF")?;
                pdf_file_size = pdf.file_size;
            }
            task.pdf_path = Some(pdf_moved_to.to_string_lossy().to_string());
            task.downloaded = pdf_file_size;
            task.total = Some(pdf_file_size);
            // 以降で task（app.downloads の可変借用）を使わないため、必要な値を先に取り出す。
            let saved_paper_id = task.paper_id;
            let saved_pdf_path = task.pdf_path.clone();
            if let Some((remote, paper_id)) = discover_link {
                let conn = runtime.database.connection();
                link_to_discover_log(conn, app, &remote, paper_id);
            }

            if let Some(paper_id) = saved_paper_id {
                dispatch_plugin_events(
                    runtime,
                    app,
                    &["paper_downloaded", "paper_opened"],
                    paper_id,
                )
                .await?;
            }

            // Project the completed download into every workspace before the next
            // render, so remote views immediately expose their local-PDF actions.
            spawn_enrichment_if_needed(runtime, senders, app)?;
            refresh_paper_views(runtime, app)?;
            app.library.message = Some(library_index_summary(
                app.library.papers.len(),
                usize::from(!downloaded_paper_was_already_indexed),
            ));
            refresh_dashboard(runtime, app)?;

            // 修正2: Library 保存時に abstract / PDF の自動翻訳を開始
            // （新規 attach 時のみ。既存論文の再 DL では既に翻訳済みの可能性が高いため触らない）
            //
            // 翻訳は別スレッドで実行され、失敗してもダウンロードの DB 登録・
            // Library 反映は上で完了済み。ここで最後に起動することで、
            // 翻訳の成否が保存処理に影響しないようにしている。
            if let Some(paper_id) = saved_paper_id {
                if downloaded_paper.is_some() {
                    auto_translate_after_library_save(runtime, senders, app, paper_id);
                }
                // 重い処理（図抽出 → PDF 翻訳）は 1 論文 1 本のパイプラインで
                // 順番に走らせる（DL 完了は待たせない）。
                if let Some(pdf) = saved_pdf_path.as_deref() {
                    spawn_paper_pipeline(runtime, senders, app, paper_id, Path::new(pdf));
                }
            }
            // 引用抽出は `s` キー押下時のオンデマンド実行に変更（UiAction::CitationSidecar）。
            // DL 完了時は papers.s2_paper_id への保存（attach_download）のみ行う。
            if downloaded_paper.is_some() {
                app.toast = Some(
                    "Download complete. abstract / PDF の自動翻訳を開始しました（s で引用抽出）"
                        .to_owned(),
                );
            } else if app.toast.is_none() {
                app.toast = Some("Download complete. Press Enter to open the PDF.".to_owned());
            }
        }
        DownloadEvent::Failed { id, error } => {
            pending.remove(&id);
            task.status = DownloadStatus::Failed(error);
            task.failed_at = Some(std::time::Instant::now());
            // 自動取得不可能なジャーナル向けに手動取り込み（o → ブラウザ DL → i）へ誘導する。
            app.toast = Some(
                "Download failed. Press o to open the site, save the PDF, then press i to import."
                    .to_owned(),
            );
        }
    }
    if is_completed {
        app.downloads
            .retain(|t| t.id != id || !matches!(t.status, DownloadStatus::Failed(_)));
    }
    Ok(())
}

// ---- 手動 PDF 取り込み（o: ブラウザで開く → ユーザーが DL → i: 取り込み） ----
// 候補の探し・保存先の決定・登録は core の `manual_import` に置いている
// （引用 TUI と `sai-moon-paper phone` も同じ関数を使う）。

/// 手動取り込みモーダルを開く（候補収集 + 状態設定）。
fn apply_open_manual_import(target: ManualImportPaper, app: &mut App) {
    let Some(dir) = papr_core::manual_import::downloads_dir() else {
        app.toast = Some("Download directory not found.".to_owned());
        return;
    };
    let candidates = papr_core::manual_import::recent_pdfs(
        &dir,
        papr_core::manual_import::TUI_WINDOW,
    );
    if candidates.is_empty() {
        app.toast = Some(format!(
            "No PDFs added within 5 minutes in {}. Save the PDF there, then press i again.",
            dir.display()
        ));
        return;
    }
    app.modal_return = app.mode;
    app.manual_import = ManualImportState {
        paper: Some(target),
        candidates,
        selected: 0,
    };
    app.mode = AppMode::ManualImport;
}

/// 手動取り込みモーダル内のキー処理。
fn handle_manual_import_key(app: &mut App, key: KeyEvent) -> Option<UiAction> {
    let count = app.manual_import.candidates.len();
    match key.code {
        KeyCode::Esc | KeyCode::Char('q') => {
            app.mode = app.modal_return;
        }
        KeyCode::Char('j') | KeyCode::Down => {
            if count > 0 {
                app.manual_import.selected = (app.manual_import.selected + 1).min(count - 1);
            }
        }
        KeyCode::Char('k') | KeyCode::Up => {
            app.manual_import.selected = app.manual_import.selected.saturating_sub(1);
        }
        KeyCode::Enter => return Some(UiAction::ConfirmManualImport),
        _ => {}
    }
    None
}

/// 取り込み確定処理（モーダル内 Enter）。
async fn apply_confirm_manual_import(
    runtime: &mut Runtime,
    senders: &ActionSenders,
    app: &mut App,
) -> Result<()> {
    let Some(target) = app.manual_import.paper.clone() else {
        app.mode = app.modal_return;
        return Ok(());
    };
    let Some(source) = app
        .manual_import
        .candidates
        .get(app.manual_import.selected)
        .cloned()
    else {
        app.toast = Some("No candidate selected.".to_owned());
        return Ok(());
    };
    match import_manual_pdf(runtime, senders, app, &source, &target).await {
        Ok(name) => {
            app.mode = app.modal_return;
            app.manual_import = ManualImportState::default();
            app.toast = Some(format!(
                "Imported {name}. abstract / PDF の自動翻訳を開始しました"
            ));
        }
        Err(error) => {
            // モーダルは開いたまま。別の候補を選び直せるようにする。
            app.toast = Some(format!("Import failed: {error}"));
        }
    }
    Ok(())
}

/// 候補 PDF を検証・移動し、対象論文に紐付けてから TUI だけの後処理を回す。
///
/// 検査・移動・登録・出来事までは core の `manual_import::import_pdf` がやる。
/// ここはプラグインの知らせ・検索履歴への紐付け・自動翻訳・図の抽出だけ。
async fn import_manual_pdf(
    runtime: &mut Runtime,
    senders: &ActionSenders,
    app: &mut App,
    source: &Path,
    target: &ManualImportPaper,
) -> Result<String> {
    // core へ渡す取り込み先（TUI の ManualImportPaper → core の ManualTarget）。
    // Remote の表示名は core が選ぶのと同じ正規名（core の `import_pdf` へ渡す前に見ておく）。
    let mut label = None;
    let core_target = match target {
        ManualImportPaper::Local { paper_id, title } => {
            papr_core::manual_import::ManualTarget::Local {
                paper_id: *paper_id,
                title: title.clone(),
            }
        }
        ManualImportPaper::Remote(paper) => {
            label = Some(
                papr_core::manual_import::unique_pdf_destination(
                    &runtime.download_dir,
                    &paper.title,
                )
                .file_name()
                .map_or_else(|| "PDF".to_owned(), |name| name.to_string_lossy().into_owned()),
            );
            papr_core::manual_import::ManualTarget::Remote(paper.clone())
        }
    };
    let imported = papr_core::manual_import::import_pdf(
        &mut runtime.database,
        &runtime.data_dir,
        &runtime.download_dir,
        source,
        &core_target,
    )
    .map_err(|error| anyhow::anyhow!("{error}"))?;
    let paper_id = imported.paper_id;
    let path = imported.path.clone();

    match target {
        ManualImportPaper::Local { .. } => {
            dispatch_plugin_events(
                runtime,
                app,
                &["paper_downloaded", "paper_opened"],
                paper_id,
            )
            .await?;
            // 修正2: Library 保存時に abstract / PDF の自動翻訳を開始
            auto_translate_after_library_save(runtime, senders, app, paper_id);
            // 図抽出 → PDF 翻訳は 1 本のパイプラインで裏に流す（import を待たせない）。
            spawn_paper_pipeline(runtime, senders, app, paper_id, &path);
        }
        ManualImportPaper::Remote(paper) => {
            dispatch_plugin_events(
                runtime,
                app,
                &["paper_downloaded", "paper_opened"],
                paper_id,
            )
            .await?;
            // Link paper to search set if import came from SearchSets page
            if let Some((set_id, position)) = app.search_set_download_context.remove(&paper.id) {
                let conn = runtime.database.connection();
                if let Err(e) = papr_core::search_sets::link_paper(conn, set_id, position, paper_id)
                {
                    app.toast = Some(format!("検索セットへの紐付けに失敗: {e}"));
                }
            } else {
                // Discover（または Dashboard）で取り込んだ場合は、いまの Discover の
                // 検索の記録に紐付ける（Library の階層表示用）。
                let conn = runtime.database.connection();
                link_to_discover_log(conn, app, paper, paper_id);
            }
            // 修正2: Library 保存時に abstract / PDF の自動翻訳を開始
            auto_translate_after_library_save(runtime, senders, app, paper_id);
            // 図抽出 → PDF 翻訳は 1 本のパイプラインで裏に流す（import を待たせない）。
            spawn_paper_pipeline(runtime, senders, app, paper_id, &path);
        }
    }
    refresh_paper_views(runtime, app)?;
    refresh_dashboard(runtime, app)?;
    // Local は保存先がそのまま残る（ふつうは論文フォルダの paper.pdf）。
    Ok(label.unwrap_or(imported.file_name))
}

fn handle_key(app: &mut App, key: KeyEvent) -> Option<UiAction> {
    // The event loop normally performs this filtering, but keeping it at the
    // state-machine boundary prevents a release event delivered by an
    // enhanced terminal from being interpreted as text after focus changes.
    // Repeats are deliberately consumed only by the PDF viewer below.
    if key.kind == KeyEventKind::Release {
        return None;
    }
    if key.kind == KeyEventKind::Repeat && app.mode != AppMode::PdfView {
        return None;
    }
    if app.mode == AppMode::PdfView {
        return handle_pdf_viewer_key(app, key);
    }
    if app.mode == AppMode::MarkdownView {
        return handle_md_view_key(app, key);
    }
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('t') {
        app.mode = AppMode::TerminalCommand;
        app.terminal_command.clear();
        app.terminal_command_cursor = 0;
        app.terminal_command_output.clear();
        app.terminal_command_directory = terminal_home_directory();
        reset_terminal_completion(app);
        return None;
    }
    // Help is a global command. Resolve it before any view-specific handler
    // (notably PaperDetail) can interpret the key as navigation. Text-entry
    // contexts retain the character as normal input.
    if key.code == KeyCode::Char('?') && !has_active_text_input(app) {
        app.dispatch(Command::ToggleHelp);
        return None;
    }
    // TASKS 画面（実行中のバックグラウンド処理一覧）も help と同じくグローバル。
    if key.code == KeyCode::Char('L') && !has_active_text_input(app) {
        app.dispatch(Command::ToggleTasks);
        return None;
    }
    // M / W も全ページで効く（どこで作業していても考えを書き留められる）。
    if key.code == KeyCode::Char('M') && !has_active_text_input(app) {
        return Some(UiAction::OpenProjectFile(ProjectFile::Mind));
    }
    if key.code == KeyCode::Char('W') && !has_active_text_input(app) {
        return Some(UiAction::OpenProjectFile(ProjectFile::State));
    }
    // S01: `J` も全ページで効く（最近 7 日の研究日誌）。
    if key.code == KeyCode::Char('J') && !has_active_text_input(app) {
        return Some(UiAction::OpenJournal);
    }
    // S02: Project ページの `A` で要対応の一覧を開く（他のページの `A` は変えない）。
    if app.page == Page::Project
        && app.mode == AppMode::Normal
        && key.code == KeyCode::Char('A')
        && !has_active_text_input(app)
    {
        app.mode = AppMode::AttentionList;
        app.attention_selected = 0;
        return None;
    }
    // N08: Dashboard / Project ページの `N` / `U` で sai-link の todo に
    // 「次の一手」「理解の確認」カードを 1 枚作る（実行はしない）。
    // ペインの選択中でもサイドバー側でも同じに効く（M / W と同じ扱い）。
    if matches!(app.page, Page::Dashboard | Page::Project) && !has_active_text_input(app) {
        match key.code {
            KeyCode::Char('N') => return Some(UiAction::CreateAgentCard(AgentCard::Next)),
            KeyCode::Char('U') => return Some(UiAction::CreateAgentCard(AgentCard::Check)),
            _ => {}
        }
    }
    if is_text_paste_shortcut(key) && paste_clipboard_into_active_input(app) {
        return None;
    }
    if let KeyHandling::Handled(action) = handle_active_mode_key(app, key) {
        return action.map(|action| *action);
    }
    if let KeyHandling::Handled(action) = handle_raw_workspace_key(app, key) {
        return action.map(|action| *action);
    }
    let key = normalize_panel_navigation(key);
    if let KeyHandling::Handled(action) = handle_navigation_key(app, key) {
        return action.map(|action| *action);
    }
    handle_focused_workspace_key(app, key)
}

fn handle_active_mode_key(app: &mut App, key: KeyEvent) -> KeyHandling {
    let action = match app.mode {
        AppMode::CommandPalette => {
            handle_command_palette_key(app, key);
            None
        }
        AppMode::TerminalCommand => {
            handle_terminal_palette_key(app, key);
            None
        }
        AppMode::Help => {
            handle_help_key(app, key);
            None
        }
        AppMode::Tasks => {
            if matches!(key.code, KeyCode::Esc | KeyCode::Char('q')) {
                app.dispatch(Command::ToggleTasks);
            }
            None
        }
        AppMode::Prompt => handle_modal_key(app, key),
        AppMode::ConfirmDelete => handle_confirm_delete_key(app, key),
        AppMode::ManualImport => handle_manual_import_key(app, key),
        AppMode::BlockPicker => handle_block_picker_key(app, key),
        AppMode::Search => handle_search_key(app, key),
        AppMode::DiscoverFilter => handle_discover_filter_key(app, key),
        AppMode::WorkspaceSearch => handle_workspace_search_key(app, key),
        AppMode::RagSearch => handle_rag_search_key(app, key),
        AppMode::QueryDetail => handle_query_detail_key(app, key),
        AppMode::AttentionList => {
            return KeyHandling::Handled(handle_attention_list_key(app, key).map(Box::new));
        }
        _ => return KeyHandling::Ignored,
    };
    KeyHandling::Handled(action.map(Box::new))
}

fn handle_attention_list_key(app: &mut App, key: KeyEvent) -> Option<UiAction> {
    let len = app.project.attention.len();
    match key.code {
        KeyCode::Char('j') | KeyCode::Down => {
            app.attention_selected = app
                .attention_selected
                .saturating_add(1)
                .min(len.saturating_sub(1));
            None
        }
        KeyCode::Char('k') | KeyCode::Up => {
            app.attention_selected = app.attention_selected.saturating_sub(1);
            None
        }
        KeyCode::Enter | KeyCode::Right | KeyCode::Char('l') => app
            .project
            .attention
            .get(app.attention_selected)
            .cloned()
            .map(UiAction::OpenAttention),
        KeyCode::Char('x') => app
            .project
            .attention
            .get(app.attention_selected)
            .cloned()
            .map(UiAction::ResolveAttention),
        KeyCode::Esc | KeyCode::Left | KeyCode::Char('h') | KeyCode::Char('q') => {
            app.mode = AppMode::Normal;
            None
        }
        _ => None,
    }
}

fn handle_raw_workspace_key(app: &mut App, key: KeyEvent) -> KeyHandling {
    if app.page == Page::Discover
        && key.modifiers.contains(KeyModifiers::CONTROL)
        && key.code == KeyCode::Right
    {
        app.discovery.next_page();
        return KeyHandling::Handled(None);
    }
    if app.page == Page::Discover
        && key.modifiers.contains(KeyModifiers::CONTROL)
        && key.code == KeyCode::Left
    {
        app.discovery.previous_page();
        return KeyHandling::Handled(None);
    }
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('b') {
        app.dispatch(Command::TogglePalette);
        return KeyHandling::Handled(None);
    }
    if key.code == KeyCode::Char('/') {
        app.page = Page::Discover;
        app.sidebar_index = 1;
        app.content_focused = true;
        app.mode = AppMode::Search;
        // R3: `/` では word 欄から開始する。
        app.discovery.input_focus = DiscoverInputFocus::Word;
        app.discovery.query_cursor = app.discovery.query.len();
        return KeyHandling::Handled(None);
    }
    // RAG ハイブリッド検索（Library / Notes の `f` キー）。
    if key.code == KeyCode::Char('f') && (app.page == Page::Library || app.page == Page::Notes) {
        app.mode = AppMode::RagSearch;
        app.rag_search = state::RagSearchState::default();
        app.rag_search.scope = if app.page == Page::Notes {
            "notes".to_string()
        } else {
            "papers".to_string()
        };
        app.rag_search.query_cursor = 0;
        return KeyHandling::Handled(None);
    }
    if app.page == Page::Discover
        && app.content_focused
        && app.mode == AppMode::Normal
        && !app.discovery.results.is_empty()
        && key.code == KeyCode::Left
    {
        app.content_focused = false;
        return KeyHandling::Handled(None);
    }
    KeyHandling::Ignored
}

fn handle_navigation_key(app: &mut App, key: KeyEvent) -> KeyHandling {
    if key.modifiers.contains(KeyModifiers::CONTROL)
        && let Some(command) = navigation_command(key)
    {
        app.dispatch(command);
        return KeyHandling::Handled(None);
    }
    if app.mode == AppMode::PaperDetail {
        return KeyHandling::Handled(handle_paper_detail_key(app, key).map(Box::new));
    }

    if !app.content_focused {
        if let Some(command) = navigation_command(key) {
            app.dispatch(command);
        }
        return KeyHandling::Handled(None);
    }
    KeyHandling::Ignored
}

fn handle_focused_workspace_key(app: &mut App, key: KeyEvent) -> Option<UiAction> {
    // U23: 研究方針のページは、このページだけのキー（`r` / `o`）だけ先に受け取る。
    if app.page == Page::Tree && app.content_focused
        && let Some(action) = tree_view::handle_tree_key(app, key)
    {
        return Some(action);
    }
    if key.code == KeyCode::Char('r')
        && app.page == Page::Discover
        && !app.discovery.query.trim().is_empty()
    {
        if app.discovery.next_batch_start.is_some()
            && app.discovery.progress_message.as_deref()
                == Some("More results could not be loaded. Press r to retry.")
        {
            return Some(UiAction::RetryDiscoverMore);
        }
        let query = app.discovery.query.trim().to_owned();
        app.discovery.query.clone_from(&query);
        return Some(UiAction::Search(query));
    }
    if key.code == KeyCode::Char('r') && app.page == Page::Library {
        return Some(UiAction::Reindex);
    }
    if key.code == KeyCode::Char('r') && app.page == Page::Import {
        // Import 画面では `r` で data/bibtex/ を再スキャンする。
        return Some(UiAction::ScanBibtex);
    }
    if key.code == KeyCode::Char('i') && app.page == Page::Library {
        // ライブラリ一覧からも手動 PDF 取り込みを開始できる。
        if let Some(p) = app.filtered_library_papers().get(app.library.selected) {
            return Some(UiAction::OpenManualImport(ManualImportPaper::Local {
                paper_id: p.id,
                title: p.title.clone(),
            }));
        }
        return None;
    }
    // sai-paper-tui sidecar: s=citation, m=md (herdr pane — split + run)
    // ライブラリ論文は UiAction::CitationSidecar 経由（オンデマンド抽出→起動）、
    // Discover/Dashboard も UiAction::CitationSidecarRemote 経由（PDF 自動 DL → 抽出 → 起動）。
    // 初回のみ解析、2 回目以降は papr.db キャッシュで即起動。
    if key.code == KeyCode::Char('s') {
        if app.page == Page::Library {
            if let Some(p) = app.filtered_library_papers().get(app.library.selected) {
                return Some(UiAction::CitationSidecar { paper_id: p.id });
            }
            return None;
        }
        let paper: Option<RemotePaper> = match app.page {
            Page::Dashboard => app.today_papers.get(app.today_selected).cloned(),
            Page::Discover => app.discovery.selected_paper().cloned(),
            Page::Import => app.import.selected_paper().cloned(),
            _ => None,
        };
        if let Some(paper) = paper {
            return Some(UiAction::CitationSidecarRemote { paper });
        }
        return None;
    }
    if key.code == KeyCode::Char('m') {
        // 単独 `m`: docling の extract.md を nvim で開く（本来のヘルプ表記どおり）。
        let Some(target) = current_md_target(app) else {
            return None;
        };
        return Some(UiAction::OpenMarkdownInNvim {
            kind: MdKind::Extract,
            paper_id: target.paper_id,
            title: target.title,
            extract_key: target.extract_key,
        });
    }
    if let KeyHandling::Handled(action) = handle_selected_paper_shortcut(app, key) {
        return action.map(|action| *action);
    }
    // R2: Tags 画面（Collections ページ）の Enter / t / Esc 処理。
    if app.page == Page::Collections
        && let Some(action) = handle_collections_key(app, key)
    {
        return Some(action);
    }
    // Project ページの PageUp / PageDown は 10 行スクロール
    // （READ NEXT があっても本文を動かせるようにする）。
    if app.page == Page::Project {
        match key.code {
            KeyCode::PageDown => {
                app.scroll_project(10);
                return None;
            }
            KeyCode::PageUp => {
                app.scroll_project(-10);
                return None;
            }
            _ => {}
        }
    }
    // N05: Project ページの READ NEXT で Enter → 選んだ論文の PAPER 画面。
    if app.page == Page::Project
        && matches!(key.code, KeyCode::Enter | KeyCode::Right | KeyCode::Char('l'))
    {
        return app
            .project
            .read_next
            .get(app.project.read_next_selected)
            .map(|entry| UiAction::OpenLibraryPaper(entry.paper_id));
    }
    if let KeyHandling::Handled(action) = handle_dashboard_key(app, key) {
        return action.map(|action| *action);
    }
    if app.page == Page::Authors {
        let (handled, action) = handle_author_key(app, key);
        if handled {
            return action;
        }
    }
    if app.page == Page::SearchSets {
        let (handled, action) = handle_search_sets_key(app, key);
        if handled {
            return action;
        }
    }
    if let Some(action) = handle_notes_key(app, key) {
        return Some(action);
    }
    if app.page == Page::Discover
        && key.code == KeyCode::Char('>')
        && !app.discovery.results.is_empty()
    {
        app.mode = AppMode::DiscoverFilter;
        app.discovery.filter_cursor = app.discovery.filter.len();
        return None;
    }
    if app.page == Page::Import && key.code == KeyCode::Char('>') && !app.import.results.is_empty()
    {
        // Discover と同じ `>` フィルタ入力を Import でも使う。
        app.mode = AppMode::DiscoverFilter;
        app.import.filter_cursor = app.import.filter.len();
        return None;
    }
    if app.page == Page::Import
        && matches!(
            key.code,
            KeyCode::Enter | KeyCode::Right | KeyCode::Char('l')
        )
    {
        // PAPER 詳細（既存の PaperDetail フロー）へ。abstract は空欄でも既存処理を流用。
        return selected_remote_paper(app).cloned().map(UiAction::OpenPaper);
    }
    if app.page == Page::Discover
        && matches!(
            key.code,
            KeyCode::Enter | KeyCode::Right | KeyCode::Char('l')
        )
    {
        return app
            .discovery
            .selected_paper()
            .cloned()
            .map(UiAction::OpenPaper);
    }

    if let Some(action) = library_action(app, key) {
        return Some(action);
    }

    if let Some(command) = navigation_command(key) {
        if app.page == Page::Discover && command == Command::MoveUp && app.discovery.selected == 0 {
            app.mode = if app.discovery.results.is_empty() {
                AppMode::Search
            } else {
                AppMode::DiscoverFilter
            };
            if app.mode == AppMode::DiscoverFilter {
                app.discovery.filter_cursor = app.discovery.filter.len();
            }
            return None;
        }
        app.dispatch(command);
    }
    None
}

fn handle_selected_paper_shortcut(app: &mut App, key: KeyEvent) -> KeyHandling {
    if key.code == KeyCode::Char('o')
        && let PaperArxivSelection::Selected(arxiv_reference) = selected_paper_arxiv_reference(app)
    {
        return KeyHandling::Handled(
            open_arxiv_page(app, arxiv_reference.as_deref()).map(Box::new),
        );
    }
    if matches!(app.page, Page::Dashboard | Page::Discover | Page::Import) {
        let action = match key.code {
            KeyCode::Char('c') => selected_remote_target(app).map(UiAction::CopyCitation),
            KeyCode::Char('d') => selected_remote_paper(app).cloned().map(UiAction::Download),
            _ => return KeyHandling::Ignored,
        };
        return KeyHandling::Handled(action.map(Box::new));
    }
    let Some(paper_id) = selected_local_paper_id(app) else {
        return KeyHandling::Ignored;
    };
    let action = match key.code {
        KeyCode::Char('u') => UiAction::MarkUnread(paper_id),
        // 'a'（Reading Queue）は廃止（仕様 §3.5）。
        KeyCode::Char('a') => return KeyHandling::Ignored,
        _ => return KeyHandling::Ignored,
    };
    KeyHandling::Handled(Some(Box::new(action)))
}

fn handle_pdf_viewer_key(app: &mut App, key: KeyEvent) -> Option<UiAction> {
    if key.code == KeyCode::Char('?') {
        app.dispatch(Command::ToggleHelp);
        return None;
    }
    let is_scroll_event = matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat);
    match key.code {
        KeyCode::Esc | KeyCode::Char('q') if key.kind == KeyEventKind::Press => {
            Some(UiAction::ClosePdf)
        }
        KeyCode::Up | KeyCode::Char('k') if is_scroll_event => {
            pdf_scroll(app, -1);
            None
        }
        KeyCode::Down | KeyCode::Char('j') if is_scroll_event => {
            pdf_scroll(app, 1);
            None
        }
        KeyCode::PageUp if key.kind == KeyEventKind::Press => {
            pdf_viewer::page_up(app);
            None
        }
        KeyCode::PageDown if key.kind == KeyEventKind::Press => {
            pdf_viewer::page_down(app);
            None
        }
        _ => None,
    }
}

/// カーソル行の最初の引用を、開く操作（`Ok`）かトーストの理由（`Err`）に変える。
/// 引用が無ければ `None`。
///
/// この作業は Library の論文（`#<番号>`）の引用だけを扱う。`[DOI:…]` などの
/// Library に無い引用は開かず、トーストにする。
fn citation_open_at_cursor(md: &md_view::MarkdownView) -> Option<Result<UiAction, String>> {
    use papr_core::reports::CitationTarget;
    let text = md.cursor_line_text()?;
    let citation = papr_core::reports::parse_citations(&text)
        .into_iter()
        .next()?;
    match citation.target {
        CitationTarget::Paper(paper_id) => Some(Ok(UiAction::OpenCitation {
            paper_id,
            locator: citation.locator,
        })),
        CitationTarget::External { .. } | CitationTarget::Unverified(_) => {
            Some(Err("Library にない論文です".into()))
        }
    }
}

/// 内蔵ビューアのキー処理（読み取り専用）。
fn handle_md_view_key(app: &mut App, key: KeyEvent) -> Option<UiAction> {
    if key.code == KeyCode::Esc {
        // 別の md（引用の `§` で開いた extract.md など）から来たときは、元の md に戻す（V06）。
        if let Some(previous) = app.md_view_previous.take() {
            app.md_view = Some(*previous);
            app.md_view_report = app.md_view_report_previous.take();
            app.md_view_return = app.md_view_return_previous;
            return None;
        }
        app.md_view = None;
        app.md_view_report = None;
        app.mode = app.md_view_return;
        return None;
    }
    // report を表示しているときだけ `C` で外部の引用を照合する。
    // note.md / extract.md では何もしない。
    if key.code == KeyCode::Char('C') {
        return app.md_view_report.clone().map(UiAction::CheckReport);
    }
    // 本文にいるときの `Enter` / `n` / `N`（V06）。
    // アウトラインにいるときは下の見出しジャンプに任せる。
    let outline_focused = app
        .md_view
        .as_ref()
        .map(|md| md.outline_focused)
        .unwrap_or(false);
    if !outline_focused {
        match key.code {
            KeyCode::Enter => {
                return match app.md_view.as_ref().and_then(citation_open_at_cursor) {
                    Some(Ok(action)) => Some(action),
                    Some(Err(message)) => {
                        app.toast = Some(message);
                        None
                    }
                    None => {
                        app.toast = Some("引用がありません".into());
                        None
                    }
                };
            }
            KeyCode::Char('n') | KeyCode::Char('N') => {
                let delta = if key.code == KeyCode::Char('n') { 1 } else { -1 };
                let moved = app
                    .md_view
                    .as_mut()
                    .map(|md| md.cursor_move_to_citation(delta))
                    .unwrap_or(false);
                if !moved {
                    app.toast = Some("引用がありません".into());
                }
                return None;
            }
            _ => {}
        }
    }
    let Some(md) = app.md_view.as_mut() else {
        // md が無いときは、覚えている元の md があれば戻す。
        if let Some(previous) = app.md_view_previous.take() {
            app.md_view = Some(*previous);
            app.md_view_report = app.md_view_report_previous.take();
            app.md_view_return = app.md_view_return_previous;
            return None;
        }
        app.md_view_report = None;
        app.mode = app.md_view_return;
        return None;
    };
    let has_headings = !md.headings.is_empty();
    let outline_focused = md.outline_focused;
    match key.code {
        KeyCode::Char('o') if has_headings => {
            md.outline_visible = !md.outline_visible;
            if !md.outline_visible {
                md.outline_focused = false;
            }
        }
        KeyCode::Tab if has_headings => {
            if md.outline_visible {
                md.outline_focused = !md.outline_focused;
            }
        }
        KeyCode::Char('z') | KeyCode::Char(' ') if has_headings && !outline_focused => {
            md.toggle_fold_at_cursor();
        }
        KeyCode::Up | KeyCode::Char('k') if outline_focused => md.outline_move(-1),
        KeyCode::Down | KeyCode::Char('j') if outline_focused => md.outline_move(1),
        KeyCode::Enter if outline_focused => md.jump_to_outline_selection(),
        KeyCode::Up | KeyCode::Char('k') => md.cursor_move(-1),
        KeyCode::Down | KeyCode::Char('j') => md.cursor_move(1),
        KeyCode::PageUp => md.cursor_move(-20),
        KeyCode::PageDown => md.cursor_move(20),
        _ => {}
    }
    None
}

/// QUERY 画面（過去の Discover 検索の再表示）のキー処理。
fn handle_query_detail_key(app: &mut App, key: KeyEvent) -> Option<UiAction> {
    // Reason 編集中は文字入力を受け付ける。
    if app.query_detail.editing_reason {
        match key.code {
            KeyCode::Esc => app.query_detail.editing_reason = false,
            KeyCode::Enter => return Some(UiAction::SaveQueryReason),
            _ => {
                let detail = &mut app.query_detail;
                edit_text(&mut detail.reason, &mut detail.reason_cursor, key);
            }
        }
        return None;
    }
    match key.code {
        KeyCode::Esc | KeyCode::Left | KeyCode::Char('h' | 'q') => {
            app.mode = app.query_detail_return;
        }
        // `e`: Reason を編集する。
        KeyCode::Char('e') => {
            app.query_detail.editing_reason = true;
            app.query_detail.reason_cursor = app.query_detail.reason.len();
        }
        KeyCode::Char('j') | KeyCode::Down => {
            app.query_detail.selected =
                (app.query_detail.selected + 1).min(app.query_detail.items.len().saturating_sub(1));
        }
        KeyCode::Char('k') | KeyCode::Up => {
            app.query_detail.selected = app.query_detail.selected.saturating_sub(1);
        }
        KeyCode::Enter | KeyCode::Right | KeyCode::Char('l') | KeyCode::Char('p') => {
            if let Some(item) = app.query_detail.items.get(app.query_detail.selected) {
                let item = item.clone();
                app.paper_detail_return = AppMode::QueryDetail;
                if let Some(local_id) = item.local_id {
                    return Some(UiAction::OpenLibraryPaper(local_id));
                }
                return Some(UiAction::OpenPaper(item.remote.clone()));
            }
        }
        _ => {}
    }
    None
}

/// ローカル論文の抽出キー（arXiv 優先、次に DOI）。
fn local_extract_key(paper: &papr_core::LibraryPaper) -> Option<String> {
    if let Some(a) = paper
        .arxiv_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        return Some(format!("ArXiv:{a}"));
    }
    if let Some(d) = paper
        .doi
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        return Some(format!("DOI:{d}"));
    }
    None
}

/// リモート論文 ID から抽出キーを組み立てる。
fn remote_extract_key(id: &str) -> String {
    if id.starts_with("ArXiv:") || id.starts_with("DOI:") {
        id.to_string()
    } else if id.contains('/') {
        format!("DOI:{id}")
    } else {
        format!("ArXiv:{id}")
    }
}

/// 現在選択中／詳細表示中の論文から、note.md・extract.md の解決に必要な情報を返す。
fn current_md_target(app: &App) -> Option<MdTarget> {
    if app.page == Page::Library {
        let papers = app.filtered_library_papers();
        let paper = papers.get(app.library.selected)?;
        return Some(MdTarget {
            paper_id: Some(paper.id),
            title: paper.title.clone(),
            extract_key: local_extract_key(paper),
        });
    }
    let paper = if app.mode == AppMode::PaperDetail {
        app.paper_detail_remote
            .as_ref()
            .or_else(|| selected_remote_paper(app))
    } else {
        selected_remote_paper(app)
    }?;
    let local = app.downloaded_remote_paper(paper);
    Some(MdTarget {
        paper_id: local.map(|p| p.id),
        title: local.map_or_else(|| paper.title.clone(), |p| p.title.clone()),
        extract_key: Some(remote_extract_key(&paper.id)),
    })
}

fn handle_command_palette_key(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Esc => app.dispatch(Command::TogglePalette),
        KeyCode::Up => app.palette_selected = app.palette_selected.saturating_sub(1),
        KeyCode::Down => {
            app.palette_selected = (app.palette_selected + 1)
                .min(app.filtered_palette_items().len().saturating_sub(1));
        }
        KeyCode::Enter => {
            let items = app.filtered_palette_items();
            if let Some(&page) = items.get(app.palette_selected) {
                app.dispatch(Command::TogglePalette);
                app.sidebar_index = Page::ALL
                    .iter()
                    .position(|&candidate| candidate == page)
                    .unwrap_or(0);
                app.page = page;
                app.content_focused = true;
                app.mode = if app.active_search_workspaces.contains(&page) {
                    AppMode::WorkspaceSearch
                } else {
                    AppMode::Normal
                };
            }
        }
        _ => {
            let workspace = &mut app.workspace;
            let old_query = workspace.palette_query.clone();
            if edit_text(
                &mut workspace.palette_query,
                &mut workspace.palette_query_cursor,
                key,
            ) && workspace.palette_query != old_query
            {
                workspace.palette_selected = 0;
            }
        }
    }
}

fn handle_terminal_palette_key(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Esc => app.mode = AppMode::Normal,
        KeyCode::Enter if app.terminal_completion_selected.is_some() => {
            apply_selected_terminal_completion(app);
            reset_terminal_completion(app);
        }
        KeyCode::Enter => run_terminal_command(app),
        KeyCode::Tab => {
            complete_terminal_command(app, key.modifiers.contains(KeyModifiers::SHIFT));
        }
        KeyCode::BackTab => complete_terminal_command(app, true),
        _ => {
            reset_terminal_completion(app);
            let workspace = &mut app.workspace;
            let _ = edit_text(
                &mut workspace.terminal_command,
                &mut workspace.terminal_command_cursor,
                key,
            );
        }
    }
}

fn handle_help_key(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Esc | KeyCode::Char('?' | 'q') => app.dispatch(Command::ToggleHelp),
        KeyCode::Up | KeyCode::Char('k') => app.help_scroll = app.help_scroll.saturating_sub(1),
        KeyCode::Down | KeyCode::Char('j') => app.help_scroll = app.help_scroll.saturating_add(1),
        KeyCode::PageUp => app.help_scroll = app.help_scroll.saturating_sub(10),
        KeyCode::PageDown => app.help_scroll = app.help_scroll.saturating_add(10),
        KeyCode::Home => app.help_scroll = 0,
        KeyCode::End => app.help_scroll = usize::MAX,
        _ => {}
    }
}

fn has_active_text_input(app: &App) -> bool {
    matches!(
        app.mode,
        AppMode::CommandPalette
            | AppMode::TerminalCommand
            | AppMode::Prompt
            | AppMode::Search
            | AppMode::DiscoverFilter
            | AppMode::WorkspaceSearch
            | AppMode::QueryDetail
            | AppMode::RagSearch
    )
}

fn run_terminal_command(app: &mut App) {
    let command_text = app.terminal_command.trim().to_owned();
    reset_terminal_completion(app);
    if command_text.is_empty() {
        return;
    }
    if command_text == "clear" {
        app.terminal_command_output.clear();
        app.terminal_command.clear();
        app.terminal_command_cursor = 0;
        return;
    }
    if command_text == "cd" || command_text.starts_with("cd ") {
        let path = command_text[2..].trim();
        app.terminal_command.clear();
        app.terminal_command_cursor = 0;
        change_terminal_directory(app, path);
        return;
    }

    #[cfg(target_os = "windows")]
    let mut command = {
        let mut command = ProcessCommand::new("cmd");
        command.args(["/C", &command_text]);
        command
    };
    #[cfg(not(target_os = "windows"))]
    let mut command = {
        let mut command = ProcessCommand::new("sh");
        command.args(["-c", &command_text]);
        command
    };
    if let Some(directory) = &app.terminal_command_directory {
        command.current_dir(directory);
    }

    let output = match command.output() {
        Ok(output) => {
            let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
            let stderr = String::from_utf8_lossy(&output.stderr);
            if !stderr.is_empty() {
                if !text.is_empty() && !text.ends_with('\n') {
                    text.push('\n');
                }
                text.push_str(&stderr);
            }
            if output.status.success() {
                text
            } else {
                format!(
                    "[exit {}]\n{text}",
                    output
                        .status
                        .code()
                        .map_or_else(|| "signal".to_owned(), |code| code.to_string())
                )
            }
        }
        Err(error) => format!("Could not run command: {error}"),
    };
    append_terminal_output(app, &command_text, &output);
    app.terminal_command.clear();
    app.terminal_command_cursor = 0;
}

fn complete_terminal_command(app: &mut App, backwards: bool) {
    if !app.terminal_completions.is_empty() {
        cycle_terminal_completion(app, backwards);
        return;
    }
    let cursor = app.terminal_command_cursor.min(app.terminal_command.len());
    let before_cursor = &app.terminal_command[..cursor];
    let token_start = before_cursor
        .rfind(char::is_whitespace)
        .map_or(0, |index| index + 1);
    let token = &before_cursor[token_start..];
    let candidates = if before_cursor[..token_start].trim().is_empty() {
        terminal_command_candidates(token)
    } else {
        terminal_path_candidates(token, app.terminal_command_directory.as_deref())
    };
    if candidates.is_empty() {
        return;
    }
    app.terminal_completions = candidates;
    app.terminal_completion_token_start = token_start;
    app.terminal_completion_selected = Some(0);
    app.session_flags.terminal_completion_applied = false;
    if app.terminal_completions.len() == 1 {
        apply_selected_terminal_completion(app);
        // A completed token is now the current input.  The next Tab must
        // inspect that input rather than continuing this one-item session.
        reset_terminal_completion(app);
    }
}

fn cycle_terminal_completion(app: &mut App, backwards: bool) {
    let count = app.terminal_completions.len();
    let Some(selected) = app.terminal_completion_selected else {
        return;
    };
    let next = if !app.session_flags.terminal_completion_applied {
        if backwards {
            count.saturating_sub(1)
        } else {
            selected
        }
    } else if backwards {
        selected.checked_sub(1).unwrap_or(count.saturating_sub(1))
    } else {
        (selected + 1) % count
    };
    app.terminal_completion_selected = Some(next);
    apply_selected_terminal_completion(app);
}

fn apply_selected_terminal_completion(app: &mut App) {
    let Some(selected) = app.terminal_completion_selected else {
        return;
    };
    let Some(candidate) = app.terminal_completions.get(selected).cloned() else {
        return;
    };
    let start = app.terminal_completion_token_start;
    let end = app.terminal_command_cursor.min(app.terminal_command.len());
    if start > end {
        reset_terminal_completion(app);
        return;
    }
    app.terminal_command.replace_range(start..end, &candidate);
    app.terminal_command_cursor = start + candidate.len();
    app.session_flags.terminal_completion_applied = true;
}

fn reset_terminal_completion(app: &mut App) {
    app.terminal_completions.clear();
    app.terminal_completion_selected = None;
    app.terminal_completion_token_start = 0;
    app.session_flags.terminal_completion_applied = false;
}

fn terminal_command_candidates(prefix: &str) -> Vec<String> {
    let normalized_prefix = prefix.to_lowercase();
    let mut candidates = ["cd", "clear"]
        .into_iter()
        .filter(|command| command.to_lowercase().starts_with(&normalized_prefix))
        .map(str::to_owned)
        .collect::<Vec<_>>();
    if let Some(path) = std::env::var_os("PATH") {
        for directory in std::env::split_paths(&path) {
            let Ok(entries) = std::fs::read_dir(directory) else {
                continue;
            };
            candidates.extend(entries.filter_map(Result::ok).filter_map(|entry| {
                let name = entry.file_name().to_string_lossy().into_owned();
                name.to_lowercase()
                    .starts_with(&normalized_prefix)
                    .then_some(name)
            }));
        }
    }
    sort_terminal_candidates(&mut candidates);
    candidates.dedup();
    candidates
}

fn change_terminal_directory(app: &mut App, path: &str) {
    let base = app
        .terminal_command_directory
        .clone()
        .or_else(|| std::env::current_dir().ok());
    let candidate = match base {
        Some(base) if Path::new(path).is_relative() => base.join(path),
        _ => PathBuf::from(path),
    };
    match std::fs::canonicalize(&candidate) {
        Ok(directory) if directory.is_dir() => {
            app.terminal_command_directory = Some(directory.clone());
            append_terminal_output(app, &format!("cd {path}"), &directory.display().to_string());
        }
        Ok(_) => append_terminal_output(app, &format!("cd {path}"), "Not a directory."),
        Err(error) => append_terminal_output(app, &format!("cd {path}"), &format!("cd: {error}")),
    }
}

fn append_terminal_output(app: &mut App, command: &str, output: &str) {
    if !app.terminal_command_output.is_empty() {
        app.terminal_command_output.push('\n');
    }
    app.terminal_command_output.push_str("$ ");
    app.terminal_command_output.push_str(command);
    app.terminal_command_output.push('\n');
    app.terminal_command_output
        .push_str(&sanitize_terminal_output(output));
    if app.terminal_command_output.len() > MAX_TERMINAL_SCROLLBACK_BYTES {
        let start = app.terminal_command_output.len() - MAX_TERMINAL_SCROLLBACK_BYTES;
        let start = next_char_boundary(&app.terminal_command_output, start);
        app.terminal_command_output.drain(..start);
    }
}

fn is_text_paste_shortcut(key: KeyEvent) -> bool {
    // Most Unix terminals encode Ctrl+Shift+V as the same control byte as
    // Ctrl+V. Crossterm correctly reports that as Ctrl+V, but there is no
    // Shift modifier left to inspect unless the terminal supports enhanced
    // keyboard reporting. Accept both encodings.
    key.modifiers.contains(KeyModifiers::CONTROL) && matches!(key.code, KeyCode::Char('v' | 'V'))
}

/// Paste clipboard text into whichever editable control currently owns focus.
/// This is kept ahead of workspace command handling so Ctrl+Shift+V cannot be
/// mistaken for a navigation or single-key action.
fn paste_clipboard_into_active_input(app: &mut App) -> bool {
    if !has_active_text_input(app) {
        return false;
    }
    let Some(text) = read_clipboard_text().filter(|text| !text.is_empty()) else {
        app.toast = Some("Clipboard does not contain text to paste.".into());
        return true;
    };
    paste_text_into_active_input(app, &text)
}

fn paste_text_into_active_input(app: &mut App, text: &str) -> bool {
    if !has_active_text_input(app) {
        return false;
    }
    if text.is_empty() {
        return true;
    }

    let (target, cursor) = match app.mode {
        AppMode::CommandPalette => {
            let workspace = &mut app.workspace;
            (
                &mut workspace.palette_query,
                &mut workspace.palette_query_cursor,
            )
        }
        AppMode::TerminalCommand => {
            let workspace = &mut app.workspace;
            (
                &mut workspace.terminal_command,
                &mut workspace.terminal_command_cursor,
            )
        }
        AppMode::Search => (&mut app.discovery.query, &mut app.discovery.query_cursor),
        AppMode::DiscoverFilter => {
            // Import 画面では DiscoveryState ではなく ImportState のフィルタを編集する。
            if app.page == Page::Import {
                (&mut app.import.filter, &mut app.import.filter_cursor)
            } else {
                (&mut app.discovery.filter, &mut app.discovery.filter_cursor)
            }
        }
        AppMode::WorkspaceSearch => {
            let workspace = &mut app.workspace;
            (
                &mut workspace.workspace_query,
                &mut workspace.workspace_query_cursor,
            )
        }
        AppMode::Prompt => match app.metadata_prompt.as_mut() {
            Some(prompt) => (&mut prompt.value, &mut prompt.cursor),
            None => return false,
        },
        _ => return false,
    };
    target.insert_str(*cursor, text);
    *cursor += text.len();

    if app.mode == AppMode::CommandPalette {
        app.palette_selected = 0;
    }
    if app.mode == AppMode::TerminalCommand {
        reset_terminal_completion(app);
    }
    if app.mode == AppMode::DiscoverFilter {
        if app.page == Page::Import {
            app.import.rebuild_filter();
        } else {
            app.discovery.rebuild_filter();
        }
    }
    true
}

fn read_clipboard_text() -> Option<String> {
    if let Ok(mut clipboard) = arboard::Clipboard::new()
        && let Ok(text) = clipboard.get_text()
    {
        return Some(text);
    }

    for (program, args) in [
        ("wl-paste", &["--no-newline"][..]),
        ("xclip", &["-selection", "clipboard", "-o"][..]),
        ("pbpaste", &[][..]),
    ] {
        if let Ok(output) = ProcessCommand::new(program).args(args).output()
            && output.status.success()
            && let Ok(text) = String::from_utf8(output.stdout)
        {
            return Some(text);
        }
    }
    None
}

fn normalize_panel_navigation(mut key: KeyEvent) -> KeyEvent {
    key.code = match key.code {
        KeyCode::Left => KeyCode::Char('h'),
        KeyCode::Right => KeyCode::Char('l'),
        code => code,
    };
    key
}

fn handle_search_key(app: &mut App, key: KeyEvent) -> Option<UiAction> {
    // R3: word / year / limit の 3 入力欄を Tab / Shift+Tab で行き来する。
    match key.code {
        KeyCode::Esc => app.mode = AppMode::Normal,
        KeyCode::Tab | KeyCode::BackTab => {
            let focus = app.discovery.input_focus;
            app.discovery.input_focus = if key.code == KeyCode::Tab {
                focus.next()
            } else {
                focus.prev()
            };
        }
        KeyCode::Down => {
            // word 欄で下矢印は結果一覧へ。year / limit では次の欄へ移動する。
            if app.discovery.input_focus == DiscoverInputFocus::Word {
                if !app.discovery.results.is_empty() {
                    app.mode = AppMode::Normal;
                    app.discovery.selected = 0;
                }
            } else {
                app.discovery.input_focus = app.discovery.input_focus.next();
            }
        }
        KeyCode::Up => {
            if app.discovery.input_focus != DiscoverInputFocus::Word {
                app.discovery.input_focus = app.discovery.input_focus.prev();
            }
        }
        KeyCode::Right
            if !key.modifiers.contains(KeyModifiers::CONTROL)
                && app.discovery.input_focus == DiscoverInputFocus::Word
                && app.discovery.query_cursor == app.discovery.query.len()
                && !app.discovery.results.is_empty() =>
        {
            app.mode = AppMode::DiscoverFilter;
            app.discovery.filter_cursor = app.discovery.filter.len();
        }
        KeyCode::Left if !key.modifiers.contains(KeyModifiers::CONTROL) => {
            let at_start = match app.discovery.input_focus {
                DiscoverInputFocus::Word => app.discovery.query_cursor == 0,
                DiscoverInputFocus::Year => app.discovery.year_cursor == 0,
                DiscoverInputFocus::Limit => app.discovery.limit_cursor == 0,
                DiscoverInputFocus::Db => app.discovery.db_cursor == 0,
            };
            if at_start {
                if app.discovery.input_focus == DiscoverInputFocus::Word {
                    app.content_focused = false;
                    app.mode = AppMode::Normal;
                } else {
                    // 左端でさらに左 → 前の欄へ移動する
                    app.discovery.input_focus = app.discovery.input_focus.prev();
                }
            } else {
                edit_discovery_focused_field(app, key);
            }
        }
        KeyCode::Enter if !app.discovery.query.trim().is_empty() => {
            // 検索実行（word は必須、year / limit は任意）。
            let query = app.discovery.query.trim().to_owned();
            app.discovery.query.clone_from(&query);
            app.mode = AppMode::Normal;
            return Some(UiAction::Search(query));
        }
        _ => edit_discovery_focused_field(app, key),
    }
    None
}

/// R3: フォーカス中の Discover 入力欄（word / year / limit / db）へキー入力を反映する。
fn edit_discovery_focused_field(app: &mut App, key: KeyEvent) {
    match app.discovery.input_focus {
        DiscoverInputFocus::Word => {
            edit_text(
                &mut app.discovery.query,
                &mut app.discovery.query_cursor,
                key,
            );
        }
        DiscoverInputFocus::Year => {
            edit_text(&mut app.discovery.year, &mut app.discovery.year_cursor, key);
        }
        DiscoverInputFocus::Limit => {
            edit_text(
                &mut app.discovery.limit,
                &mut app.discovery.limit_cursor,
                key,
            );
        }
        DiscoverInputFocus::Db => {
            edit_text(&mut app.discovery.db, &mut app.discovery.db_cursor, key);
        }
    }
}

fn handle_discover_filter_key(app: &mut App, key: KeyEvent) -> Option<UiAction> {
    // Import 画面: Discover と同じ `>` フィルタ体験（検索バーが無い分岐のみ変える）。
    if app.page == Page::Import {
        return handle_import_filter_key(app, key);
    }
    if app.discovery.results.is_empty() {
        app.mode = AppMode::Normal;
        return None;
    }
    if key.code == KeyCode::Char('>') {
        app.mode = AppMode::Normal;
        return None;
    }
    match key.code {
        KeyCode::Esc => app.mode = AppMode::Normal,
        KeyCode::Up => {
            app.mode = AppMode::Search;
            app.discovery.query_cursor = app.discovery.query.len();
        }
        KeyCode::Down | KeyCode::Enter => {
            app.mode = AppMode::Normal;
            app.discovery.selected = app
                .discovery
                .selected
                .min(app.discovery.visible_page_len().saturating_sub(1));
        }
        KeyCode::Left
            if !key.modifiers.contains(KeyModifiers::CONTROL)
                && app.discovery.filter_cursor == 0 =>
        {
            app.mode = AppMode::Search;
            app.discovery.query_cursor = app.discovery.query.len();
        }
        KeyCode::Right
            if !key.modifiers.contains(KeyModifiers::CONTROL)
                && app.discovery.filter_cursor == app.discovery.filter.len() =>
        {
            app.mode = AppMode::Normal;
        }
        _ => {
            if edit_text(
                &mut app.discovery.filter,
                &mut app.discovery.filter_cursor,
                key,
            ) {
                app.discovery.rebuild_filter();
            }
        }
    }
    None
}

/// Import 画面の `>` フィルタ入力処理（[`handle_discover_filter_key`] の Import 版）。
/// 検索バーが無いため `Up` / `Left` 境界は検索モードへ戻らず `Normal` へ戻る。
fn handle_import_filter_key(app: &mut App, key: KeyEvent) -> Option<UiAction> {
    if app.import.results.is_empty() {
        app.mode = AppMode::Normal;
        return None;
    }
    if key.code == KeyCode::Char('>') {
        app.mode = AppMode::Normal;
        return None;
    }
    match key.code {
        KeyCode::Esc => app.mode = AppMode::Normal,
        KeyCode::Down | KeyCode::Enter => {
            app.mode = AppMode::Normal;
            app.import.clamp_selection();
        }
        KeyCode::Left
            if !key.modifiers.contains(KeyModifiers::CONTROL) && app.import.filter_cursor == 0 =>
        {
            app.mode = AppMode::Normal;
        }
        KeyCode::Right
            if !key.modifiers.contains(KeyModifiers::CONTROL)
                && app.import.filter_cursor == app.import.filter.len() =>
        {
            app.mode = AppMode::Normal;
        }
        _ => {
            if edit_text(&mut app.import.filter, &mut app.import.filter_cursor, key) {
                app.import.rebuild_filter();
            }
        }
    }
    None
}

fn handle_workspace_search_key(app: &mut App, key: KeyEvent) -> Option<UiAction> {
    if key.code == KeyCode::Char('>') {
        app.mode = AppMode::Normal;
        app.content_focused = true;
        let page = app.page;
        app.workspace.active_search_workspaces.remove(&page);
        return None;
    }

    match key.code {
        KeyCode::Esc => {
            app.mode = AppMode::Normal;
            let page = app.page;
            app.workspace.active_search_workspaces.remove(&page);
        }
        KeyCode::Down | KeyCode::Enter => {
            app.mode = AppMode::Normal;
            app.content_focused = true;
            let page = app.page;
            app.workspace.active_search_workspaces.remove(&page);
            if key.code == KeyCode::Down {
                match app.page {
                    Page::Library => app.library.selected = 0,
                    Page::Notes => app.notes_selected = 0,
                    _ => {}
                }
            }
        }
        KeyCode::Left
            if !key.modifiers.contains(KeyModifiers::CONTROL)
                && app.workspace_query_cursor == 0 =>
        {
            app.mode = AppMode::Normal;
            app.content_focused = false;
        }
        _ => {
            let workspace = &mut app.workspace;
            edit_text(
                &mut workspace.workspace_query,
                &mut workspace.workspace_query_cursor,
                key,
            );
        }
    }
    None
}

fn handle_confirm_delete_key(app: &mut App, key: KeyEvent) -> Option<UiAction> {
    use DeletionTarget;
    match key.code {
        KeyCode::Char('y' | 'Y') | KeyCode::Enter => {
            let target = app.delete_confirmation.take()?;
            app.mode = AppMode::Normal;
            match target {
                DeletionTarget::Paper { id, path, .. } => Some(UiAction::DeletePaper {
                    paper_id: id,
                    path: path.map(|p| p.to_string_lossy().into_owned()),
                }),
                DeletionTarget::Collection { .. } => None,
            }
        }
        KeyCode::Char('n' | 'N' | 'q') | KeyCode::Esc => {
            app.delete_confirmation = None;
            app.mode = AppMode::Normal;
            None
        }
        _ => None,
    }
}

/// マウスの操作。ホイールのスクロールだけを受け付ける。
///
/// 画面を動かす操作はすべてキーでできる（要件 R5.6）。
fn handle_mouse_event(app: &mut App, mouse: crossterm::event::MouseEvent) {
    use crossterm::event::MouseEventKind;
    // 端末はマウスを取っているので、入力欄（ダイアログ）でなければホイールを
    // スクロールに割り当てる。研究方針のページでは選択が動く。
    match mouse.kind {
        MouseEventKind::ScrollDown => scroll_by_wheel(app, 1),
        MouseEventKind::ScrollUp => scroll_by_wheel(app, -1),
        _ => {}
    }
}

fn handle_block_picker_key(app: &mut App, key: KeyEvent) -> Option<UiAction> {
    match key.code {
        KeyCode::Char('j') | KeyCode::Down => {
            app.block_picker.move_down();
            None
        }
        KeyCode::Char('k') | KeyCode::Up => {
            app.block_picker.move_up();
            None
        }
        KeyCode::Enter => {
            if let Some(block) = app.block_picker.selected_block() {
                app.mode = AppMode::Prompt;
                app.metadata_prompt = Some(MetadataPrompt {
                    paper_id: None,
                    tag_paper_id: None,
                    rename_collection_id: None,
                    rename_paper_id: None,
                    value: String::new(),
                    cursor: 0,
                    selected: 0,
                    current_collection: None,
                    handoff_block_id: Some(block.id.clone()),
                });
                None
            } else {
                None
            }
        }
        KeyCode::Char('q') | KeyCode::Esc => {
            app.mode = AppMode::Normal;
            app.block_picker = block_picker::BlockPickerState::new(Vec::new());
            None
        }
        _ => None,
    }
}

fn handle_notes_key(app: &mut App, key: KeyEvent) -> Option<UiAction> {
    if app.page != Page::Notes {
        return None;
    }
    let paper = *app.filtered_notes_papers().get(app.notes_selected)?;
    match key.code {
        KeyCode::Char('n') => Some(UiAction::OpenNote(PaperTarget::Local(paper.id))),
        // 共通ルール: 論文リストの Enter は PAPER 画面へ遷移する（PDF は直接開かない）。
        KeyCode::Enter | KeyCode::Right | KeyCode::Char('l') => {
            Some(UiAction::OpenLibraryPaper(paper.id))
        }
        KeyCode::Char('t') => Some(UiAction::OpenTagModal(paper.id)),
        KeyCode::Char('B') => Some(UiAction::Bookmark(PaperTarget::Local(paper.id))),
        KeyCode::Char('c') => Some(UiAction::CopyCitation(PaperTarget::Local(paper.id))),
        KeyCode::Char('g') => Some(UiAction::Prompt(PaperTarget::Local(paper.id))),
        KeyCode::Char('R') => Some(UiAction::RenamePdf(paper.id)),
        KeyCode::Char('x') => Some(UiAction::ConfirmDeletePaper {
            paper_id: paper.id,
            title: paper.title.clone(),
            path: paper.pdf_path.clone(),
        }),
        _ => None,
    }
}

fn library_action(app: &mut App, key: KeyEvent) -> Option<UiAction> {
    if app.page != Page::Library {
        return None;
    }
    // `shift+K`: 通常リスト ↔ 階層リストの切り替え。
    if key.code == KeyCode::Char('K') {
        app.library_view = if app.library_view == LibraryView::Hierarchy {
            LibraryView::Flat
        } else {
            LibraryView::Hierarchy
        };
        app.library_tree_dirty = true;
        app.library_tree_selected = 0;
        app.library_tree_scroll = 0;
        return None;
    }
    if app.library_view == LibraryView::Hierarchy {
        return library_hierarchy_action(app, key);
    }
    if matches!(key.code, KeyCode::Enter | KeyCode::Char('l')) {
        // Enter / Right(正規化後は l) / l でカーソル位置の論文を開く。
        let paper = app.detail_library_paper()?;
        return Some(UiAction::OpenLibraryPaper(paper.id));
    }
    if key.code == KeyCode::Char('x') {
        let paper = *app.filtered_library_papers().get(app.library.selected)?;
        return Some(UiAction::ConfirmDeletePaper {
            paper_id: paper.id,
            title: paper.title.clone(),
            path: paper.pdf_path.clone(),
        });
    }
    if key.code == KeyCode::Char('S') {
        // S キーで handoff（論文をエージェントに引き渡す）
        if let Some(paper) = app.filtered_library_papers().get(app.library.selected) {
            let paper_id = paper.id;
            app.modal_return = app.mode;
            // UI will load blocks in the action handler
            return Some(UiAction::OpenHandoff(paper_id));
        }
        return None;
    }
    handle_library_metadata_key(app, key)
}

/// 階層表示（Library の `shift+K`）専用のキー処理。
fn library_hierarchy_action(app: &mut App, key: KeyEvent) -> Option<UiAction> {
    let Some(row) = app.library_tree.get(app.library_tree_selected).cloned() else {
        return None;
    };
    match key.code {
        // `space`: query/import 行の開閉（フォルダツリーの fold/expand）。
        KeyCode::Char(' ') => {
            if let Some(fold_key) = row.fold_key {
                if !app.library_tree_folded.remove(&fold_key) {
                    app.library_tree_folded.insert(fold_key);
                }
                app.library_tree_dirty = true;
            }
            None
        }
        KeyCode::Enter | KeyCode::Right | KeyCode::Char('l') | KeyCode::Char('p') => {
            match row.kind {
                LibraryTreeKind::Query => row.log_id.map(UiAction::OpenQueryDetail),
                LibraryTreeKind::Paper => row.paper_id.map(UiAction::OpenLibraryPaper),
                LibraryTreeKind::Import => None,
            }
        }
        _ => None,
    }
}

fn handle_dashboard_key(app: &mut App, key: KeyEvent) -> KeyHandling {
    // 仕様 §3.3: Dashboard の RA 行で Enter → 該当論文を開く。
    // Library 等の他ページでは Enter / Right / l を横取りしない。
    if app.page != Page::Dashboard {
        return KeyHandling::Ignored;
    }
    if matches!(
        key.code,
        KeyCode::Enter | KeyCode::Right | KeyCode::Char('l')
    ) {
        if let Some(paper_id) = app.selected_activity_paper_id() {
            return KeyHandling::Handled(Some(Box::new(UiAction::OpenLibraryPaper(paper_id))));
        }
    }
    KeyHandling::Ignored
}

fn handle_author_key(app: &mut App, key: KeyEvent) -> (bool, Option<UiAction>) {
    if app.active_author.is_some() {
        match key.code {
            KeyCode::Esc | KeyCode::Char('h') => {
                app.active_author = None;
                app.author_papers.clear();
                return (true, None);
            }
            KeyCode::Enter | KeyCode::Right | KeyCode::Char('l') => {
                let Some(&paper) = app.filtered_author_papers().get(app.author_paper_selected)
                else {
                    return (true, None);
                };
                // 共通ルール: 論文リストの Enter は PAPER 画面へ遷移する。
                return (true, Some(UiAction::OpenLibraryPaper(paper.id)));
            }
            KeyCode::Char('B') => {
                return (
                    true,
                    app.filtered_author_papers()
                        .get(app.author_paper_selected)
                        .map(|&paper| UiAction::Bookmark(PaperTarget::Local(paper.id))),
                );
            }
            KeyCode::Char('c') => {
                return (
                    true,
                    app.filtered_author_papers()
                        .get(app.author_paper_selected)
                        .map(|&paper| UiAction::CopyCitation(PaperTarget::Local(paper.id))),
                );
            }
            KeyCode::Char('R') => {
                return (
                    true,
                    app.filtered_author_papers()
                        .get(app.author_paper_selected)
                        .map(|&paper| UiAction::RenamePdf(paper.id)),
                );
            }
            KeyCode::Char('g') => {
                return (
                    true,
                    app.filtered_author_papers()
                        .get(app.author_paper_selected)
                        .map(|&paper| UiAction::Prompt(PaperTarget::Local(paper.id))),
                );
            }
            KeyCode::Char('n') => {
                return (
                    true,
                    app.filtered_author_papers()
                        .get(app.author_paper_selected)
                        .map(|&paper| UiAction::OpenNote(PaperTarget::Local(paper.id))),
                );
            }
            KeyCode::Char('x') => {
                return (
                    true,
                    app.filtered_author_papers()
                        .get(app.author_paper_selected)
                        .map(|&paper| UiAction::ConfirmDeletePaper {
                            paper_id: paper.id,
                            title: paper.title.clone(),
                            path: paper.pdf_path.clone(),
                        }),
                );
            }
            _ => return (false, None),
        }
    }
    if matches!(
        key.code,
        KeyCode::Enter | KeyCode::Right | KeyCode::Char('l')
    ) {
        let action = app
            .filtered_authors()
            .get(app.author_selected)
            .map(|&author| UiAction::OpenAuthor(author.id));
        return (true, action);
    }
    (false, None)
}

fn handle_search_sets_key(app: &mut App, key: KeyEvent) -> (bool, Option<UiAction>) {
    match key.code {
        KeyCode::Char('j') | KeyCode::Down => {
            if app.search_set_focused_on_items {
                app.search_set_item_selected = app
                    .search_set_item_selected
                    .saturating_add(1)
                    .min(app.search_set_items.len().saturating_sub(1));
            } else {
                app.search_set_selected = app
                    .search_set_selected
                    .saturating_add(1)
                    .min(app.search_sets.len().saturating_sub(1));
                // Load items for newly selected set
                if !app.search_sets.is_empty() {
                    return (
                        true,
                        Some(UiAction::LoadSearchSetItems(
                            app.search_sets[app.search_set_selected].id,
                        )),
                    );
                }
            }
            return (true, None);
        }
        KeyCode::Char('k') | KeyCode::Up => {
            if app.search_set_focused_on_items {
                app.search_set_item_selected = app.search_set_item_selected.saturating_sub(1);
            } else {
                app.search_set_selected = app.search_set_selected.saturating_sub(1);
                // Load items for newly selected set
                if !app.search_sets.is_empty() {
                    return (
                        true,
                        Some(UiAction::LoadSearchSetItems(
                            app.search_sets[app.search_set_selected].id,
                        )),
                    );
                }
            }
            return (true, None);
        }
        KeyCode::Tab | KeyCode::Char('h') | KeyCode::Char('l') => {
            // h/Left から左ペイン（セット一覧）へ移動、または左ペインにいるなら false でナビゲーションに戻す
            if matches!(key.code, KeyCode::Char('h')) && !app.search_set_focused_on_items {
                return (false, None); // 左ペインにいるので、ナビゲーションシステムに任せる
            }
            // l/Right から右ペイン（候補一覧）へ移動、またはすでに右ペインなら何もしない
            if matches!(key.code, KeyCode::Char('l')) && app.search_set_focused_on_items {
                return (true, None); // 右ペインにいるので、何もしない
            }
            // ペインを切り替える
            app.search_set_focused_on_items = !app.search_set_focused_on_items;
            return (true, None);
        }
        KeyCode::Enter => {
            if app.search_set_focused_on_items && !app.search_set_items.is_empty() {
                let item = &app.search_set_items[app.search_set_item_selected];
                return (true, Some(UiAction::OpenPaper(item.remote.clone())));
            }
            return (true, None);
        }
        KeyCode::Char('d') => {
            if app.search_set_focused_on_items && !app.search_set_items.is_empty() {
                let item = &app.search_set_items[app.search_set_item_selected];
                let set_id = app.search_sets[app.search_set_selected].id;
                let position = item.position;
                // Store context for download completion handler
                app.search_set_download_context
                    .insert(item.remote.id.clone(), (set_id, position));
                return (true, Some(UiAction::Download(item.remote.clone())));
            }
            return (true, None);
        }
        KeyCode::Char('i') => {
            if app.search_set_focused_on_items && !app.search_set_items.is_empty() {
                let item = &app.search_set_items[app.search_set_item_selected];
                let set_id = app.search_sets[app.search_set_selected].id;
                let position = item.position;
                // Store context for manual import completion
                app.search_set_download_context
                    .insert(item.remote.id.clone(), (set_id, position));
                app.manual_import.paper = Some(ManualImportPaper::Remote(item.remote.clone()));
                app.mode = AppMode::ManualImport;
                return (true, None);
            }
            return (true, None);
        }
        KeyCode::Char('K') => {
            if app.search_set_focused_on_items && !app.search_set_items.is_empty() {
                let set_id = app.search_sets[app.search_set_selected].id;
                let position = app.search_set_items[app.search_set_item_selected].position;
                return (
                    true,
                    Some(UiAction::MarkSearchSetItem {
                        set_id,
                        position,
                        decision: "keep".to_string(),
                    }),
                );
            }
            return (true, None);
        }
        KeyCode::Char('x') => {
            if app.search_set_focused_on_items && !app.search_set_items.is_empty() {
                let set_id = app.search_sets[app.search_set_selected].id;
                let position = app.search_set_items[app.search_set_item_selected].position;
                return (
                    true,
                    Some(UiAction::MarkSearchSetItem {
                        set_id,
                        position,
                        decision: "drop".to_string(),
                    }),
                );
            }
            return (true, None);
        }
        KeyCode::Char('o') => {
            if app.search_set_focused_on_items && !app.search_set_items.is_empty() {
                let item = &app.search_set_items[app.search_set_item_selected];
                if let Some(url) = &item.remote.pdf_url {
                    return (true, Some(UiAction::OpenBrowser(url.clone())));
                }
            }
            return (true, None);
        }
        // S03: 選んだ検索セットをもう一度実行する（どちらのペインからでも）。
        KeyCode::Char('r') => (
            true,
            app.search_sets
                .get(app.search_set_selected)
                .map(|set| UiAction::RerunSearchSet(set.id)),
        ),
        _ => (false, None),
    }
}

fn navigation_command(key: KeyEvent) -> Option<Command> {
    match (key.code, key.modifiers) {
        (KeyCode::Char('b'), modifiers) if modifiers.contains(KeyModifiers::CONTROL) => {
            Some(Command::TogglePalette)
        }
        (KeyCode::Char('>'), _) => Some(Command::ToggleWorkspaceSearch),
        (KeyCode::Char('j') | KeyCode::Down, _) => Some(Command::MoveDown),
        (KeyCode::Char('k') | KeyCode::Up, _) => Some(Command::MoveUp),
        (KeyCode::Enter | KeyCode::Right | KeyCode::Char('l'), _) => Some(Command::Open),
        (KeyCode::Left | KeyCode::Char('h'), _) => Some(Command::Back),
        (KeyCode::Char('?'), _) => Some(Command::ToggleHelp),
        (KeyCode::Char('q'), _) => Some(Command::Quit),
        _ => None,
    }
}

fn edit_text(text: &mut String, cursor: &mut usize, key: KeyEvent) -> bool {
    let mut changed = false;
    match key.code {
        KeyCode::Left => {
            if key.modifiers.contains(KeyModifiers::CONTROL) {
                *cursor = prev_word_boundary(text, *cursor);
            } else if *cursor > 0 {
                let mut prev = *cursor - 1;
                while prev > 0 && !text.is_char_boundary(prev) {
                    prev -= 1;
                }
                *cursor = prev;
            }
        }
        KeyCode::Right => {
            if key.modifiers.contains(KeyModifiers::CONTROL) {
                *cursor = next_word_boundary(text, *cursor);
            } else if *cursor < text.len() {
                let mut next = *cursor + 1;
                while next < text.len() && !text.is_char_boundary(next) {
                    next += 1;
                }
                *cursor = next;
            }
        }
        KeyCode::Home => {
            let s = &text[..*cursor];
            let line_start = s.rfind('\n').map_or(0, |idx| idx + 1);
            *cursor = line_start;
        }
        KeyCode::End => {
            let line_end = text[*cursor..]
                .find('\n')
                .map_or(text.len(), |idx| *cursor + idx);
            *cursor = line_end;
        }
        KeyCode::Backspace => {
            if *cursor > 0 {
                let prev = if key.modifiers.contains(KeyModifiers::CONTROL) {
                    prev_word_boundary(text, *cursor)
                } else {
                    prev_char_boundary(text, *cursor)
                };
                text.drain(prev..*cursor);
                *cursor = prev;
                changed = true;
            }
        }
        KeyCode::Delete => {
            if *cursor < text.len() {
                let next = if key.modifiers.contains(KeyModifiers::CONTROL) {
                    next_word_boundary(text, *cursor)
                } else {
                    next_char_boundary(text, *cursor)
                };
                text.drain(*cursor..next);
                changed = true;
            }
        }
        KeyCode::Up => move_text_cursor_vertical(text, cursor, false),
        KeyCode::Down => move_text_cursor_vertical(text, cursor, true),
        KeyCode::Char(_) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
            let Some(character) = text_input_char(key) else {
                return false;
            };
            text.insert(*cursor, character);
            *cursor += character.len_utf8();
            changed = true;
        }
        _ => {}
    }
    changed
}

fn move_text_cursor_vertical(text: &str, cursor: &mut usize, down: bool) {
    let current_line_start = text[..*cursor].rfind('\n').map_or(0, |idx| idx + 1);
    let column = *cursor - current_line_start;
    let (line_start, line_len) = if down {
        let Some(current_line_end) = text[*cursor..].find('\n').map(|idx| *cursor + idx) else {
            return;
        };
        let next_line_start = current_line_end + 1;
        let next_line_end = text[next_line_start..]
            .find('\n')
            .map_or(text.len(), |idx| next_line_start + idx);
        (next_line_start, next_line_end - next_line_start)
    } else {
        if current_line_start == 0 {
            return;
        }
        let previous_line = &text[..current_line_start - 1];
        let previous_line_start = previous_line.rfind('\n').map_or(0, |idx| idx + 1);
        (
            previous_line_start,
            current_line_start - 1 - previous_line_start,
        )
    };
    let mut target = line_start + column.min(line_len);
    while target > line_start && !text.is_char_boundary(target) {
        target -= 1;
    }
    *cursor = target;
}

/// Return the layout-resolved character supplied by the terminal.
fn text_input_char(key: KeyEvent) -> Option<char> {
    let KeyCode::Char(character) = key.code else {
        return None;
    };
    Some(character)
}

fn handle_modal_key(app: &mut App, key: KeyEvent) -> Option<UiAction> {
    if app.mode == AppMode::Prompt {
        match key.code {
            KeyCode::Esc => {
                app.metadata_prompt = None;
                app.mode = app.modal_return;
            }
            KeyCode::Enter => {
                // コレクション候補補完は廃止（仕様 §3.1）。
                let prompt = app.metadata_prompt.take();
                app.mode = app.modal_return;
                return prompt.map(UiAction::SubmitPrompt);
            }
            KeyCode::Down => {
                if let Some(prompt) = &mut app.metadata_prompt {
                    let _ = &mut prompt.selected;
                }
            }
            KeyCode::Up => {
                if let Some(prompt) = &mut app.metadata_prompt {
                    prompt.selected = prompt.selected.saturating_sub(1);
                }
            }
            _ => {
                if let Some(prompt) = &mut app.metadata_prompt {
                    edit_text(&mut prompt.value, &mut prompt.cursor, key);
                }
            }
        }
        return None;
    }
    None
}

fn sai_paper_tui_bin() -> Option<PathBuf> {
    // sai-moon-paper-cite バイナリを優先（引用リスト TUI）
    // 探索順: 実行ファイルと同一の target ディレクトリ → ワークスペース target → PATH
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        candidates.push(dir.join("sai-moon-paper-cite"));
        candidates.push(dir.join("sai-moon-paper-cite.exe"));
    }
    let target = papr_core::paths::compile_time_workspace_root().join("target");
    candidates.push(target.join("debug").join("sai-moon-paper-cite"));
    candidates.push(target.join("release").join("sai-moon-paper-cite"));
    candidates.push(target.join("debug").join("sai-moon-paper-cite.exe"));
    for c in candidates {
        let p = PathBuf::from(c);
        if p.is_file() {
            return Some(p);
        }
    }
    // Try PATH
    if let Ok(out) = std::process::Command::new("which")
        .arg("sai-moon-paper-cite")
        .output()
    {
        let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if !s.is_empty() {
            let p = PathBuf::from(s);
            if p.is_file() {
                return Some(p);
            }
        }
    }
    None
}

fn spawn_papr_sai(mode: &str, paper_id: &str) {
    // sai-paper-core の正規化関数で paper id を統一（ArXiv URL / bare ID / 40hex / DOI 等）
    let pid = papr_core::citation::normalize_paper_id(paper_id);
    // Prefer sai-paper-tui (Rust Ratatui) binary — papr identical UI + CITED IN
    if let Some(bin) = sai_paper_tui_bin() {
        let bin_str = bin.display().to_string();
        if let Ok(output) = std::process::Command::new("herdr")
            .args(["pane", "split", "--direction", "right", "--no-focus"])
            .output()
        {
            if let Ok(v) = serde_json::from_slice::<serde_json::Value>(&output.stdout) {
                if let Some(pane_id) = v
                    .get("result")
                    .and_then(|r| r.get("pane"))
                    .and_then(|p| p.get("pane_id"))
                    .and_then(|id| id.as_str())
                {
                    std::thread::sleep(std::time::Duration::from_millis(300));
                    // Quote pid for shell (handles DOI with slashes, etc.)
                    let quoted_pid = format!("'{}'", pid.replace("'", "'\\''"));
                    let cmd = format!("{} {} {}", bin_str, mode, quoted_pid);
                    let _ = std::process::Command::new("herdr")
                        .args(["pane", "run", pane_id, &cmd])
                        .spawn();
                    return;
                }
            }
        }
        let _ = std::process::Command::new(bin).args([mode, &pid]).spawn();
        return;
    }
    // sai-moon-paper-cite バイナリが見つからない場合はビルドを促して終了（Node sidecar は廃止）
    eprintln!("sai-moon-paper-cite binary not found — build with: cargo build -p sai-paper-cite");
}

fn handle_paper_detail_key(app: &mut App, key: KeyEvent) -> Option<UiAction> {
    // Keep the detail view self-contained even if its handler is invoked from
    // a future input path: help must never fall through to detail navigation.
    if key.code == KeyCode::Char('?') {
        app.dispatch(Command::ToggleHelp);
        return None;
    }
    // `space` リーダー: `space+n` = note.md、`space+m` = extract.md を内蔵ビューアで開く。
    if app.space_pending {
        app.space_pending = false;
        let kind = match key.code {
            KeyCode::Char('n') => Some(MdKind::Note),
            KeyCode::Char('m') => Some(MdKind::Extract),
            KeyCode::Esc => return None,
            _ => None,
        };
        if let Some(kind) = kind {
            let target = current_md_target(app)?;
            if kind == MdKind::Note && target.paper_id.is_none() {
                app.toast = Some("note.md: ダウンロード済みの論文ではありません".into());
                return None;
            }
            return Some(UiAction::OpenMarkdownViewer {
                kind,
                paper_id: target.paper_id,
                title: target.title,
                extract_key: target.extract_key,
            });
        }
        // 上記以外のキーはそのまま通常処理へ進む。
    }
    if key.code == KeyCode::Char(' ') {
        app.space_pending = true;
        app.toast = Some("space: n=note.md  m=extract.md（もう一度 space で解除）".into());
        return None;
    }
    // REPORTS 欄: 数字キー `1`〜`5` で内蔵ビューアに report を開く。
    if let KeyCode::Char(digit @ '1'..='5') = key.code {
        let index = digit as usize - '1' as usize;
        if let Some((report, _)) = app.paper_reports.get(index) {
            return Some(UiAction::OpenReport(report.id.clone()));
        }
        return None;
    }
    // Library PAPER: separate handling (local papers)
    // ローカル論文の詳細（`paper_detail_local` あり、またはローカル一覧ページ）のとき。
    if app.is_local_paper_detail() {
        // 詳細表示中は保持 ID に選択を寄せ、一覧の選択位置・フィルタ・
        // 再読み込みの影響を受けず「開いた論文」に操作が作用する。
        let detail_idx = app.paper_detail_local.and_then(|paper_id| {
            app.filtered_library_papers()
                .iter()
                .position(|p| p.id == paper_id)
        });
        if let Some(idx) = detail_idx {
            app.library.selected = idx;
        }
        match key.code {
            KeyCode::Esc | KeyCode::Left | KeyCode::Char('h' | 'q') => app.dispatch(Command::Back),
            KeyCode::Char('j') | KeyCode::Down => {
                scroll_paper_detail(app, 1);
            }
            KeyCode::Char('k') | KeyCode::Up => {
                scroll_paper_detail(app, -1);
            }
            KeyCode::PageDown => {
                scroll_paper_detail_page(app, 1);
            }
            KeyCode::PageUp => {
                scroll_paper_detail_page(app, -1);
            }
            KeyCode::Char('p') | KeyCode::Enter => {
                let (paper_id, path) = selected_library_pdf(app)?;
                return Some(UiAction::OpenPdf {
                    paper_id,
                    path,
                    page: None,
                });
            }
            KeyCode::Char('c') => {
                let paper = *app.filtered_library_papers().get(app.library.selected)?;
                return Some(UiAction::CopyCitation(PaperTarget::Local(paper.id)));
            }
            KeyCode::Char('o') => {
                if let PaperArxivSelection::Selected(arxiv_reference) =
                    selected_paper_arxiv_reference(app)
                {
                    return open_arxiv_page(app, arxiv_reference.as_deref());
                }
                return None;
            }
            KeyCode::Char('s') => {
                // ライブラリ一覧と同じオンデマンド抽出フローへ統一。
                // （旧: spawn_papr_sai 直接呼び出し。未抽出論文で空リストになる問題を修正）
                // ensure_extract の stale キャッシュにより、抽出済み論文は即 tui 起動する。
                if let Some(paper) = app.filtered_library_papers().get(app.library.selected) {
                    return Some(UiAction::CitationSidecar { paper_id: paper.id });
                }
                return None;
            }
            KeyCode::Char('m') => {
                // 単独 `m`: docling の extract.md を nvim で開く。
                if let Some(paper) = app.filtered_library_papers().get(app.library.selected) {
                    return Some(UiAction::OpenMarkdownInNvim {
                        kind: MdKind::Extract,
                        paper_id: Some(paper.id),
                        title: paper.title.clone(),
                        extract_key: local_extract_key(paper),
                    });
                }
                return None;
            }
            KeyCode::Char('i') => {
                // 手動 PDF 取り込み（ブラウザで保存した PDF をこの論文に紐付ける）。
                if let Some(paper) = app.filtered_library_papers().get(app.library.selected) {
                    return Some(UiAction::OpenManualImport(ManualImportPaper::Local {
                        paper_id: paper.id,
                        title: paper.title.clone(),
                    }));
                }
                return None;
            }
            // R1: PAPER 詳細でも `t` でタグ付与モーダルを開ける。
            KeyCode::Char('t') => {
                let paper = *app.filtered_library_papers().get(app.library.selected)?;
                return Some(UiAction::OpenTagModal(paper.id));
            }
            KeyCode::Char('n' | 'g') => {
                let paper = *app.filtered_library_papers().get(app.library.selected)?;
                let target = PaperTarget::Local(paper.id);
                app.modal_return = AppMode::PaperDetail;
                return Some(match key.code {
                    KeyCode::Char('n') => UiAction::OpenNote(target),
                    _ => UiAction::Prompt(target),
                });
            }
            KeyCode::Char('B') => {
                let paper = *app.filtered_library_papers().get(app.library.selected)?;
                return Some(UiAction::Bookmark(PaperTarget::Local(paper.id)));
            }
            // abstract翻訳: a=初回のみ, A=強制再翻訳
            KeyCode::Char('a') => {
                if let Some(paper) = app.filtered_library_papers().get(app.library.selected) {
                    return Some(UiAction::TranslateAbstract {
                        paper_id: paper.id,
                        force: false,
                    });
                }
                return None;
            }
            KeyCode::Char('A') => {
                if let Some(paper) = app.filtered_library_papers().get(app.library.selected) {
                    return Some(UiAction::TranslateAbstract {
                        paper_id: paper.id,
                        force: true,
                    });
                }
                return None;
            }
            // PDF翻訳: T (pdf2zh, 数分) / 翻訳済みなら翻訳 PDF を view で開く
            KeyCode::Char('T') => {
                if let Some(paper) = app.filtered_library_papers().get(app.library.selected) {
                    // 翻訳済み（-dual.pdf / -ja.pdf が実在）なら再翻訳せずに開く
                    if let Some(pdf_path) = paper
                        .pdf_path
                        .as_deref()
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        && let Some(translated) = papr_core::paths::translated_pdf_path(pdf_path)
                    {
                        return Some(UiAction::OpenPdf {
                            paper_id: paper.id,
                            path: translated,
                            page: None,
                        });
                    }
                    return Some(UiAction::TranslatePdf { paper_id: paper.id });
                }
                return None;
            }
            // ポッドキャスト生成: r (sai-radio, 数分)
            KeyCode::Char('r') => {
                if let Some(paper) = app.filtered_library_papers().get(app.library.selected) {
                    return Some(UiAction::GeneratePodcast { paper_id: paper.id });
                }
                return None;
            }
            _ => {}
        }
        return None;
    }
    if matches!(app.page, Page::Dashboard | Page::Discover | Page::Import)
        && matches!(key.code, KeyCode::Char('n' | 't' | 'g' | 'B'))
    {
        return None;
    }

    match key.code {
        KeyCode::Esc | KeyCode::Left | KeyCode::Char('h' | 'q') => app.dispatch(Command::Back),
        KeyCode::Char('j') | KeyCode::Down => {
            scroll_paper_detail(app, 1);
        }
        KeyCode::Char('k') | KeyCode::Up => {
            scroll_paper_detail(app, -1);
        }
        KeyCode::PageDown => {
            scroll_paper_detail_page(app, 1);
        }
        KeyCode::PageUp => {
            scroll_paper_detail_page(app, -1);
        }
        KeyCode::Char('d') => {
            return selected_remote_paper(app).cloned().map(UiAction::Download);
        }
        KeyCode::Char('c') => {
            return selected_remote_target(app).map(UiAction::CopyCitation);
        }
        KeyCode::Char('s') => {
            // Discover/Dashboard 一覧と同じオンデマンド抽出フローへ統一。
            // PDF 未所持なら自動 DL → docling 解析 → tui 起動。
            if let Some(paper) = selected_remote_paper(app) {
                return Some(UiAction::CitationSidecarRemote {
                    paper: paper.clone(),
                });
            }
            return None;
        }
        KeyCode::Char('m') => {
            // 単独 `m`: docling の extract.md を nvim で開く。
            if let Some(target) = current_md_target(app) {
                return Some(UiAction::OpenMarkdownInNvim {
                    kind: MdKind::Extract,
                    paper_id: target.paper_id,
                    title: target.title,
                    extract_key: target.extract_key,
                });
            }
            return None;
        }
        KeyCode::Char('o') => {
            let arxiv_reference = selected_remote_paper(app).map(|paper| paper.id.clone());
            return open_arxiv_page(app, arxiv_reference.as_deref());
        }
        // Remote PAPER 詳細でも `a` / `A` で abstract 翻訳（DB にキャッシュ）。
        KeyCode::Char('a') => {
            return selected_remote_paper(app).cloned().map(|paper| {
                UiAction::TranslateAbstractRemote {
                    paper,
                    force: false,
                }
            });
        }
        KeyCode::Char('A') => {
            return selected_remote_paper(app)
                .cloned()
                .map(|paper| UiAction::TranslateAbstractRemote { paper, force: true });
        }
        KeyCode::Char('i') => {
            // 手動 PDF 取り込み（自動 DL 不可能なジャーナル向け）。
            return selected_remote_paper(app)
                .cloned()
                .map(|p| UiAction::OpenManualImport(ManualImportPaper::Remote(p)));
        }
        KeyCode::Char('p') => {
            let remote = selected_remote_paper(app)?;
            let local = app.downloaded_remote_paper(remote)?;
            let path = local.pdf_path.as_ref()?;
            return Some(UiAction::OpenPdf {
                paper_id: local.id,
                path: PathBuf::from(path),
                page: None,
            });
        }
        KeyCode::Enter => {
            let remote = selected_remote_paper(app)?;
            let local = app.downloaded_remote_paper(remote)?;
            let path = local.pdf_path.as_ref()?;
            return Some(UiAction::OpenPdf {
                paper_id: local.id,
                path: PathBuf::from(path),
                page: None,
            });
        }
        KeyCode::Char('n' | 'g') => {
            app.modal_return = AppMode::PaperDetail;
            let target = selected_remote_target(app)?;
            return Some(match key.code {
                KeyCode::Char('n') => UiAction::OpenNote(target),
                _ => UiAction::Prompt(target),
            });
        }
        KeyCode::Char('B') => return selected_remote_target(app).map(UiAction::Bookmark),
        _ => {}
    }
    None
}

fn handle_library_metadata_key(app: &mut App, key: KeyEvent) -> Option<UiAction> {
    let target = app
        .filtered_library_papers()
        .get(app.library.selected)
        .map(|&paper| PaperTarget::Local(paper.id))?;
    app.modal_return = AppMode::Normal;
    match key.code {
        KeyCode::Char('n') => Some(UiAction::OpenNote(target)),
        // R1: Library 一覧上でも `t` でタグ付与モーダルを開ける。
        KeyCode::Char('t') => {
            if let PaperTarget::Local(id) = target {
                Some(UiAction::OpenTagModal(id))
            } else {
                None
            }
        }
        KeyCode::Char('g') => Some(UiAction::Prompt(target)),
        KeyCode::Char('B') => Some(UiAction::Bookmark(target)),
        KeyCode::Char('c') => Some(UiAction::CopyCitation(target)),
        KeyCode::Char('R') => {
            if let PaperTarget::Local(id) = target {
                Some(UiAction::RenamePdf(id))
            } else {
                None
            }
        }
        _ => None,
    }
}

fn selected_remote_target(app: &App) -> Option<PaperTarget> {
    selected_remote_paper(app)
        .cloned()
        .map(Box::new)
        .map(PaperTarget::Remote)
}

fn selected_remote_paper(app: &App) -> Option<&RemotePaper> {
    // PAPER 詳細で保持している論文を優先（詳細画面の d/c/s/p/Enter 等が
    // 表示中の論文に確実に作用する）。一覧モードでは従来どおり選択中論文。
    if app.mode == AppMode::PaperDetail {
        if let Some(paper) = app.paper_detail_remote.as_ref() {
            return Some(paper);
        }
    }
    match app.page {
        Page::Dashboard => app.today_papers.get(app.today_selected),
        Page::Import => app.import.selected_paper(),
        _ => app.discovery.selected_paper(),
    }
}

fn selected_library_pdf(app: &App) -> Option<(i64, PathBuf)> {
    let paper = app.detail_library_paper()?;
    paper
        .pdf_path
        .as_ref()
        .map(|path| (paper.id, PathBuf::from(path)))
}

enum PaperArxivSelection {
    NoPaper,
    Selected(Option<String>),
}

fn apply_config_update(
    runtime: &mut Runtime,
    app: &mut App,
    config: &Config,
    theme: &mut Theme,
    senders: &ActionSenders,
) -> Result<()> {
    let previous = runtime.config.clone();
    if config.ui.theme != previous.ui.theme {
        let new_theme =
            Theme::load(&config.ui.theme).map_err(|e| anyhow::anyhow!("Theme load failed: {e}"))?;
        *theme = new_theme;
    }

    if config.ui.pdf_viewer != previous.ui.pdf_viewer {
        runtime.pdf_viewer = config.ui.pdf_viewer.clone().unwrap_or_else(default_pdf_viewer);
        app.pdf_viewer.clone_from(&runtime.pdf_viewer);
    }

    let paths_changed = config.library.download_path != previous.library.download_path
        || config.library.import_folders != previous.library.import_folders;
    if paths_changed {
        let download_dir = config
            .library.download_path
            .clone()
            .unwrap_or_else(|| runtime.default_downloads_dir.clone());
        let _ = std::fs::create_dir_all(&download_dir);
        let download_dir = std::fs::canonicalize(&download_dir).unwrap_or(download_dir);
        runtime.download_dir.clone_from(&download_dir);

        let collection_roots: Vec<_> = config
            .library.import_folders
            .iter()
            .map(|root| std::fs::canonicalize(root).unwrap_or_else(|_| root.clone()))
            .collect();
        runtime.collection_roots.clone_from(&collection_roots);
        // 走査対象は core の library_tree に一元化する（論文の置き場
        // `<data_dir>/papers/` を常に含める。段階 2a）。
        runtime.library_roots =
            papr_core::library_tree::library_roots_for(&config, &runtime.default_downloads_dir);
        restart_runtime_watcher(runtime)?;
        refresh_library(runtime, app)?;
        refresh_organization(&runtime.database, &runtime.library_roots, app)?;
        refresh_downloads(runtime, app);
    }

    let old_sig = runtime.dashboard_keyword_signature.clone();
    runtime.dashboard_keywords = config.dashboard_keyword_list();
    runtime.dashboard_keyword_signature = dashboard_keyword_signature(&runtime.dashboard_keywords);
    let keywords_changed = old_sig != runtime.dashboard_keyword_signature;

    if paths_changed {
        refresh_dashboard(runtime, app)?;
    }

    if config.library.enabled_plugins != previous.library.enabled_plugins
        && let Ok(plugin_host) = PluginHost::discover(&runtime.plugins_dir, &config.library.enabled_plugins)
    {
        app.plugins = plugin_host.plugins();
        app.plugin_diagnostics = plugin_host.diagnostics().len();
        runtime.plugin_host = plugin_host;
    }

    if keywords_changed {
        refresh_dashboard_papers(runtime, senders, app)?;
    }

    if config.discover != previous.discover {
        if let Ok(client) = S2Client::new(resolve_s2_api_key(config)) {
            runtime.s2 = client;
        }
    }

    runtime.config = config.clone();
    Ok(())
}

fn finalize_download_task(task: &mut DownloadTask) {
    if let Some(ref pdf_path) = task.pdf_path {
        let final_path = std::path::PathBuf::from(pdf_path);
        let temp_path = final_path.with_extension("pdf.part");
        if temp_path.exists() {
            let _ = std::fs::rename(&temp_path, &final_path);
        }
    }
}

fn open_arxiv_page(app: &mut App, arxiv_reference: Option<&str>) -> Option<UiAction> {
    let Some(url) = arxiv_reference.and_then(arxiv_page_url) else {
        app.toast = Some("No valid arXiv page is available for this paper".into());
        return None;
    };
    Some(UiAction::OpenBrowser(url))
}

fn arxiv_page_url(reference: &str) -> Option<String> {
    let reference = reference.trim();
    if reference.starts_with("https://") || reference.starts_with("http://") {
        return Some(reference.to_owned());
    }
    let id = reference
        .strip_prefix("arXiv:")
        .unwrap_or(reference)
        .trim_end_matches(".pdf");
    let (base, version) = match id.rsplit_once('v') {
        Some((base, version))
            if !version.is_empty()
                && version.chars().all(|character| character.is_ascii_digit()) =>
        {
            (base, Some(version))
        }
        _ => (id, None),
    };
    let modern = base.split_once('.').is_some_and(|(year_month, number)| {
        year_month.len() == 4
            && year_month
                .chars()
                .all(|character| character.is_ascii_digit())
            && matches!(number.len(), 4 | 5)
            && number.chars().all(|character| character.is_ascii_digit())
    });
    let legacy = base.split_once('/').is_some_and(|(category, number)| {
        !category.is_empty()
            && category
                .chars()
                .all(|character| character.is_ascii_alphabetic() || matches!(character, '-' | '.'))
            && number.len() == 7
            && number.chars().all(|character| character.is_ascii_digit())
    });
    (modern || legacy).then(|| match version {
        Some(version) => format!("https://arxiv.org/abs/{base}v{version}"),
        None => format!("https://arxiv.org/abs/{base}"),
    })
}
fn selected_paper_arxiv_reference(app: &App) -> PaperArxivSelection {
    // PAPER 詳細で保持している（リモート）論文を最優先する。
    if app.mode == AppMode::PaperDetail
        && let Some(paper) = app.paper_detail_remote.as_ref()
    {
        return PaperArxivSelection::Selected(Some(paper.id.clone()));
    }
    match app.page {
        Page::Dashboard => app
            .today_papers
            .get(app.today_selected)
            .map_or(PaperArxivSelection::NoPaper, |paper| {
                PaperArxivSelection::Selected(Some(paper.id.clone()))
            }),
        Page::Discover => app
            .discovery
            .selected_paper()
            .map_or(PaperArxivSelection::NoPaper, |paper| {
                PaperArxivSelection::Selected(Some(paper.id.clone()))
            }),
        Page::Import => app
            .import
            .selected_paper()
            .map_or(PaperArxivSelection::NoPaper, |paper| {
                PaperArxivSelection::Selected(Some(paper.id.clone()))
            }),
        Page::Tree => PaperArxivSelection::NoPaper,
        Page::Project => PaperArxivSelection::NoPaper,
        Page::Library | Page::Collections | Page::Authors | Page::Notes => {
            selected_local_paper_id(app).map_or(PaperArxivSelection::NoPaper, |id| {
                PaperArxivSelection::Selected(
                    app.library
                        .papers
                        .iter()
                        .find(|paper| paper.id == id)
                        .and_then(|paper| paper.arxiv_id.clone()),
                )
            })
        }
        Page::SearchSets => PaperArxivSelection::NoPaper,
    }
}
fn process_page_change(
    runtime: &Runtime,
    app: &mut App,
    _theme: &mut Theme,
    last_page: &mut Option<Page>,
) -> Result<bool> {
    if Some(app.page) == *last_page {
        return Ok(false);
    }
    app.workspace_query.clear();
    app.workspace_query_cursor = 0;
    if app.page == Page::Dashboard || app.page == Page::Project {
        refresh_dashboard(runtime, app)?;
    }
    if app.page == Page::Import {
        // Import 画面入場時に data/bibtex/ を自動スキャンする（同期・数十ファイル想定）。
        scan_bibtex_import(app);
    }
    if app.page == Page::SearchSets {
        // S03: 入場時に検索セット一覧と選択中の候補を読み込む。
        refresh_search_sets(runtime, app);
    }
    *last_page = Some(app.page);
    Ok(true)
}

/// Markdown ファイルを herdr pane + nvim で開く（パス指定版）。
///
/// RAG 検索結果から markdown ファイルを直接開くために使用する。
fn open_markdown_file(path: &str) -> Result<()> {
    let cmd = format!("nvim {}", shell_quote(path));
    let spawned = (|| -> Option<()> {
        let output = std::process::Command::new("herdr")
            .args(["pane", "split", "--direction", "right", "--no-focus"])
            .output()
            .ok()?;
        let value: serde_json::Value = serde_json::from_slice(&output.stdout).ok()?;
        let pane_id = value
            .get("result")?
            .get("pane")?
            .get("pane_id")?
            .as_str()?
            .to_string();
        std::thread::sleep(std::time::Duration::from_millis(300));
        std::process::Command::new("herdr")
            .args(["pane", "run", &pane_id, &cmd])
            .spawn()
            .ok()?;
        Some(())
    })();
    if spawned.is_none() {
        // herdr 不在時は直接 spawn（フォールバック）。
        let _ = std::process::Command::new("nvim").arg(path).spawn();
    }
    Ok(())
}

/// note.md / extract.md の実パスを解決する。
fn resolve_md_path(
    kind: MdKind,
    paper_id: Option<i64>,
    title: &str,
    extract_key: Option<&str>,
    runtime: &Runtime,
    app: &App,
) -> Option<PathBuf> {
    match kind {
        MdKind::Note => {
            let id = paper_id?;
            Some(paper_folder(&runtime.data_dir, id, title).join(paper_file_names::NOTE))
        }
        MdKind::Extract => {
            // 引用抽出の記録 → 論文フォルダ → 旧 extracts/ の順に探す（core と共通）。
            // arXiv ID / DOI が無い論文でも papers.id から論文フォルダを見つけられる。
            papr_core::citation::resolve_extract_md(
                runtime.database.connection(),
                paper_id,
                extract_key,
                &app.data_dir,
            )
        }
    }
}

/// プロジェクトルート（`.sai/paper` の 2 つ上）を得る。
fn activity_root(data_dir: &Path) -> PathBuf {
    data_dir
        .parent()
        .and_then(Path::parent)
        .map(Path::to_path_buf)
        .unwrap_or_else(|| data_dir.to_path_buf())
}

/// ユーザーが「開いた」記録を残す（N02）。
///
/// `target` はパスから決め、`surface` は `tui` にする（共有契約）。
/// 記録に失敗しても画面は止めない（戻り値は捨てる）。
fn record_open_in_tui(runtime: &Runtime, path: &Path, paper_id: Option<i64>) {
    let root = activity_root(&runtime.data_dir);
    let _ = record_open_at(&runtime.database, &root, &runtime.data_dir, path, paper_id);
}

/// [`record_open_in_tui`] の本体（テストできるように `Runtime` を取らない）。
fn record_open_at(
    database: &Database,
    root: &Path,
    data_dir: &Path,
    path: &Path,
    paper_id: Option<i64>,
) -> Result<(), papr_core::database::DatabaseError> {
    let target = papr_core::activity::open_target_for_path(root, path);
    let stored = papr_core::activity::to_activity_path(root, data_dir, path);
    let paper_id = paper_id.or_else(|| papr_core::activity::paper_id_for_path(Path::new(&stored)));
    papr_core::activity::record_open(database, target, "tui", paper_id, &stored)?;
    Ok(())
}

/// 別の md を開く前に、今の md の画面を 1 つだけ覚える（`Esc` で戻す。V06）。
fn stash_md_view_before_replacing(app: &mut App) {
    if app.mode == AppMode::MarkdownView && app.md_view_previous.is_none() {
        app.md_view_previous = app.md_view.take().map(Box::new);
        app.md_view_report_previous = app.md_view_report.take();
        app.md_view_return_previous = app.md_view_return;
    }
}

/// 任意の md パスを内蔵 Markdown ビューアで開く（研究方針の `tree.md` や `state.md` 用）。
fn open_md_viewer_path(runtime: &Runtime, app: &mut App, theme: &Theme, label: &str, path: &Path) {
    if !path.is_file() {
        app.toast = Some(format!("{label} が見つかりません: {}", path.display()));
        return;
    }
    match std::fs::read_to_string(path) {
        Ok(content) => {
            stash_md_view_before_replacing(app);
            app.md_view = Some(md_view::MarkdownView::from_markdown(
                &content,
                theme,
                label.to_owned(),
                path.display().to_string(),
            ));
            app.md_view_report = None;
            app.md_view_return = app.mode;
            app.mode = AppMode::MarkdownView;
            record_open_in_tui(runtime, path, None);
        }
        Err(error) => {
            app.toast = Some(format!("{label} の読み込みに失敗: {error}"));
        }
    }
}

/// 内蔵 Markdown ビューアを開く（`space+n` / `space+m`）。
fn apply_open_markdown_viewer(
    kind: MdKind,
    paper_id: Option<i64>,
    title: &str,
    extract_key: Option<&str>,
    runtime: &Runtime,
    app: &mut App,
    theme: &Theme,
) -> bool {
    let label = match kind {
        MdKind::Note => "note.md",
        MdKind::Extract => "extract.md",
    };
    let Some(path) = resolve_md_path(kind, paper_id, title, extract_key, runtime, app) else {
        app.toast = Some(match kind {
            MdKind::Extract => {
                "extract.md はまだ作成されていません（図抽出・引用抽出の実行後に作られます）".into()
            }
            MdKind::Note => format!("{label} の場所を特定できません"),
        });
        return false;
    };
    if !path.is_file() {
        app.toast = Some(format!("{label} が見つかりません: {}", path.display()));
        return false;
    }
    match std::fs::read_to_string(&path) {
        Ok(content) => {
            stash_md_view_before_replacing(app);
            app.md_view = Some(md_view::MarkdownView::from_markdown(
                &content,
                theme,
                label.to_owned(),
                path.display().to_string(),
            ));
            app.md_view_report = None;
            app.md_view_return = app.mode;
            app.mode = AppMode::MarkdownView;
            record_open_in_tui(runtime, &path, paper_id);
            true
        }
        Err(error) => {
            app.toast = Some(format!("{label} の読み込みに失敗: {error}"));
            false
        }
    }
}

/// report の先頭に出す 1 行を作る。
///
/// - 問題あり: `> 出典: 要確認 2 件・注意 1 件（C で外部の引用を確認 / 詳しくは sai-paper report issues R-0007）`
/// - 問題なし: `> 出典: 問題なし（最終確認 09/24）`（`checked_at` の日付。無ければ「未確認」）
///
/// `checked_at` は ISO8601 の文字列。日付を取り出せないときは「未確認」にする。
fn report_header_line(
    counts: papr_core::IssueCounts,
    checked_at: Option<&str>,
    report_id: &str,
) -> String {
    if counts.errors > 0 || counts.warnings > 0 {
        format!(
            "> 出典: 要確認 {} 件・注意 {} 件（C で外部の引用を確認 / 詳しくは sai-paper report issues {report_id}）",
            counts.errors, counts.warnings
        )
    } else {
        let mark = checked_at
            .and_then(|at| chrono::DateTime::parse_from_rfc3339(at).ok())
            .map(|at| at.format("%m/%d").to_string())
            .unwrap_or_else(|| "未確認".to_string());
        format!("> 出典: 問題なし（最終確認 {mark}）")
    }
}

/// report の md を、先頭の 1 行（[`report_header_line`]）を付けて内蔵ビューアで開く。
///
/// md のファイルは書き換えない（表示用の本文だけを作る）。
fn open_report_in_viewer(report_id: &str, runtime: &Runtime, app: &mut App, theme: &Theme) {
    let conn = runtime.database.connection();
    let Some(report) = papr_core::reports::get_report(conn, report_id).ok().flatten() else {
        app.toast = Some(format!("report が見つかりません: {report_id}"));
        return;
    };
    let Some(path) = papr_core::reports::resolve_md_path(&report) else {
        app.toast = Some(format!(
            "report のファイルが見つかりません: {}",
            report.md_path
        ));
        return;
    };
    let body = match std::fs::read_to_string(&path) {
        Ok(body) => body,
        Err(error) => {
            app.toast = Some(format!("report の読み込みに失敗: {error}"));
            return;
        }
    };
    let counts = papr_core::reports::issue_counts(conn, report_id).unwrap_or_default();
    let checked_at = papr_core::reports::report_check(conn, report_id)
        .ok()
        .flatten()
        .and_then(|check| check.checked_at);
    let content = format!(
        "{}\n\n{body}",
        report_header_line(counts, checked_at.as_deref(), report_id)
    );
    stash_md_view_before_replacing(app);
    if app.mode != AppMode::MarkdownView {
        app.md_view_return = app.mode;
    }
    app.md_view = Some(md_view::MarkdownView::from_markdown(
        &content,
        theme,
        format!("report {report_id}"),
        path.display().to_string(),
    ));
    app.md_view_report = Some(report_id.to_owned());
    app.mode = AppMode::MarkdownView;
    record_open_in_tui(runtime, &path, None);
}

/// 引用の `locator`（`§3.2` / `p.5` / `Fig.3` / なし）から決まる飛び先。
#[derive(Debug, Clone, PartialEq, Eq)]
enum CitationLocator {
    /// `[#37]` — PAPER 画面
    Paper,
    /// `[#37 §3.2]` — extract.md の見出し（`§` を除いた番号）
    Heading(String),
    /// `[#37 p.5]` — PDF のページ（1 始まり）
    Page(usize),
    /// `[#37 Fig.3]` — PAPER 画面の FIGURES（図番号）
    Figure(u32),
}

/// 引用の `locator` を飛び先に分ける。形が分からないときは PAPER 画面にする。
fn parse_citation_locator(locator: Option<&str>) -> CitationLocator {
    let Some(locator) = locator.map(str::trim).filter(|s| !s.is_empty()) else {
        return CitationLocator::Paper;
    };
    if let Some(rest) = locator.strip_prefix('§') {
        let number = rest.trim().trim_end_matches('.');
        return if number.is_empty() {
            CitationLocator::Paper
        } else {
            CitationLocator::Heading(number.to_string())
        };
    }
    let lower = locator.to_ascii_lowercase();
    // `p.5` / `p. 5` / `pp.5` の形。
    if let Some(rest) = lower.strip_prefix('p') {
        // `pp.5` の 2 つ目の `p` も読み飛ばす。
        let digits = leading_digits(rest.trim_start_matches('p'));
        if let Ok(page) = digits.parse::<usize>() {
            return CitationLocator::Page(page);
        }
    }
    // `Fig.3` / `fig 3` の形。
    if let Some(rest) = lower.strip_prefix("fig") {
        let digits = leading_digits(rest);
        if let Ok(figure) = digits.parse::<u32>() {
            return CitationLocator::Figure(figure);
        }
    }
    CitationLocator::Paper
}

/// 先頭の `.` と空白を飛ばして、続く数字の並びを返す（無ければ空文字）。
fn leading_digits(rest: &str) -> String {
    rest.trim_start_matches(|c: char| c == '.' || c.is_whitespace())
        .chars()
        .take_while(char::is_ascii_digit)
        .collect()
}

/// md の表示画面の引用（`[#37 §3.2]` など）から根拠を開く（V06）。
///
/// 飛び先の決まり（共有契約）:
/// - `§3.2` → 論文の `extract.md` を md の表示画面で開き、`3.2` の見出しへ飛ぶ
/// - `p.5` → 論文の PDF を 5 ページ目で開く
/// - `Fig.3` → PAPER 画面を開き、FIGURES の欄へ移る（画像が無ければトースト）
/// - 指定なし → PAPER 画面
/// - 論文が Library に無い → トースト
async fn apply_open_citation(
    paper_id: i64,
    locator: Option<&str>,
    runtime: &mut Runtime,
    senders: &ActionSenders,
    app: &mut App,
    theme: &Theme,
) -> Result<()> {
    let Some(paper) = runtime.database.library_paper_by_id(paper_id).ok().flatten() else {
        app.toast = Some(format!("#{paper_id} は Library にありません"));
        return Ok(());
    };
    match parse_citation_locator(locator) {
        CitationLocator::Paper => {
            open_paper_screen_from_citation(paper_id, runtime, senders, app)?;
        }
        CitationLocator::Figure(number) => {
            open_paper_screen_from_citation(paper_id, runtime, senders, app)?;
            if app
                .paper_figures
                .iter()
                .any(|figure| figure.fig_no == i64::from(number))
            {
                app.paper_detail_jump_to_figures = true;
            } else {
                app.toast = Some(format!("図 {number} は未抽出です"));
            }
        }
        CitationLocator::Heading(number) => {
            let extract_key = local_extract_key(&paper);
            // 見出しジャンプは extract.md を実際に開けたときだけ行う。
            let opened = apply_open_markdown_viewer(
                MdKind::Extract,
                Some(paper_id),
                &paper.title,
                extract_key.as_deref(),
                runtime,
                app,
                theme,
            );
            if opened {
                let jumped = app
                    .md_view
                    .as_mut()
                    .map(|md| md.jump_to_heading_number(&number))
                    .unwrap_or(false);
                if !jumped {
                    app.toast = Some(format!("§{number} の見出しが見つかりません"));
                }
            }
        }
        CitationLocator::Page(page) => {
            let Some(path) = paper
                .pdf_path
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(PathBuf::from)
            else {
                app.toast = Some(format!("#{paper_id} の PDF がありません"));
                return Ok(());
            };
            apply_paper_metadata_action(
                UiAction::OpenPdf {
                    paper_id,
                    path,
                    page: Some(page),
                },
                runtime,
                senders,
                app,
            )
            .await?;
        }
    }
    Ok(())
}

/// PAPER 画面を開く。`Esc` で md の表示画面へ戻れるように戻り先を覚える。
fn open_paper_screen_from_citation(
    paper_id: i64,
    runtime: &mut Runtime,
    senders: &ActionSenders,
    app: &mut App,
) -> Result<()> {
    if app.mode == AppMode::MarkdownView {
        app.paper_detail_return = AppMode::MarkdownView;
    }
    apply_library_navigation_action(UiAction::OpenLibraryPaper(paper_id), runtime, senders, app)
}

/// 内蔵ビューアの `C`: report の外部の引用を公開 API で照合する。
///
/// 照合のあとは先頭の 1 行（件数）を作り直すため、同じ report を開き直す。
async fn apply_check_report(
    report_id: &str,
    runtime: &Runtime,
    app: &mut App,
    theme: &Theme,
) -> Result<()> {
    app.toast = Some("外部の引用を確認しています…".into());
    let opts = papr_core::CheckOptions::default();
    let summary = {
        let conn = runtime.database.connection();
        papr_core::check_report(conn, report_id, &opts).await
    };
    match summary {
        Ok(summary) => {
            // `Error` は結果に含まれないので、残りを「照合の失敗」として見せる。
            let failed = summary
                .total
                .saturating_sub(summary.found + summary.not_found + summary.offline);
            let suffix = if failed > 0 {
                format!("・照合の失敗 {failed}")
            } else {
                String::new()
            };
            app.toast = Some(format!(
                "確認済み {}・見つからない {}・未確認 {}{suffix}",
                summary.found, summary.not_found, summary.offline
            ));
            refresh_paper_reports(runtime, app);
            refresh_dashboard(runtime, app)?;
            open_report_in_viewer(report_id, runtime, app, theme);
        }
        Err(error) => {
            app.toast = Some(format!("照合に失敗しました: {error}"));
        }
    }
    Ok(())
}
fn apply_open_markdown_in_nvim(
    kind: MdKind,
    paper_id: Option<i64>,
    title: &str,
    extract_key: Option<&str>,
    runtime: &Runtime,
    app: &mut App,
) {
    let label = match kind {
        MdKind::Note => "note.md",
        MdKind::Extract => "extract.md",
    };
    let Some(path) = resolve_md_path(kind, paper_id, title, extract_key, runtime, app) else {
        app.toast = Some(match kind {
            MdKind::Extract => {
                "extract.md はまだ作成されていません（図抽出・引用抽出の実行後に作られます）".into()
            }
            MdKind::Note => format!("{label} の場所を特定できません"),
        });
        return;
    };
    if !path.is_file() {
        app.toast = Some(format!("{label} が見つかりません: {}", path.display()));
        return;
    }
    if let Err(error) = open_markdown_file(&path.to_string_lossy()) {
        app.toast = Some(format!("{label} を開けません: {error}"));
    } else {
        record_open_in_tui(runtime, &path, paper_id);
    }
}

/// QUERY 画面を開く（検索ログを読み込む）。
fn apply_open_query_detail(log_id: i64, runtime: &Runtime, app: &mut App) {
    let conn = runtime.database.connection();
    let Some(log) = papr_core::search_log::get(conn, log_id).ok().flatten() else {
        app.toast = Some("検索ログが見つかりません".into());
        return;
    };
    let raw_items = papr_core::search_log::items(conn, log_id).unwrap_or_default();
    let papers = app.library.papers.clone();
    let items = raw_items
        .into_iter()
        .map(|item| {
            // 候補に紐付いた論文を先に使う（Library の階層表示と同じ順番）
            let local_id = item
                .paper_id
                .filter(|id| papers.iter().any(|p| p.id == *id))
                .or_else(|| {
                    papr_core::library_tree::match_downloaded_paper(&papers, &item.remote)
                        .map(|index| papers[index].id)
                });
            QueryDetailItem {
                remote: item.remote,
                local_id,
            }
        })
        .collect();
    app.query_detail = QueryDetailState {
        log_id: log.id,
        query: log.query,
        year: log.year,
        limit_txt: log.limit_txt,
        source_dbs: log.source_dbs,
        reason: log.reason.unwrap_or_default(),
        editing_reason: false,
        reason_cursor: 0,
        items,
        selected: 0,
        scroll: 0,
    };
    app.query_detail_return = AppMode::Normal;
    app.mode = AppMode::QueryDetail;
}

/// QUERY 画面の Reason を DB に保存する。
fn apply_save_query_reason(runtime: &Runtime, app: &mut App) {
    let log_id = app.query_detail.log_id;
    let reason = app.query_detail.reason.clone();
    if let Err(error) =
        papr_core::search_log::set_reason(runtime.database.connection(), log_id, &reason)
    {
        app.toast = Some(format!("Reason の保存に失敗: {error}"));
    } else {
        app.toast = Some("Reason を保存しました".into());
    }
    app.query_detail.editing_reason = false;
    app.library_tree_dirty = true;
}

/// `M` / `W`: mind.md / state.md を開く。
///
/// U23: sci（研究方針）を使うプロジェクトの state.md は木からの書き出しなので、
/// nvim ではなく内蔵の Markdown ビューアで読むだけにする（R-M3）。
fn apply_open_project_file(
    file: ProjectFile,
    runtime: &Runtime,
    app: &mut App,
    theme: &Theme,
) -> Result<()> {
    let (path, template, label) = match file {
        ProjectFile::Mind => (
            papr_core::paths::mind_path(&runtime.data_dir),
            papr_core::project_files::MIND_TEMPLATE,
            "mind.md",
        ),
        ProjectFile::State => (
            papr_core::paths::state_path(&runtime.data_dir),
            papr_core::project_files::STATE_TEMPLATE,
            "state.md",
        ),
    };
    if let Err(error) = papr_core::project_files::ensure_one(&path, template) {
        app.toast = Some(format!("{label} を作成できません: {error}"));
        return Ok(());
    }
    if file == ProjectFile::State && state_md_is_sci_backed(&runtime.database) {
        open_md_viewer_path(runtime, app, theme, "state.md", &path);
        app.toast = Some(
            "state.md は研究方針の木から書き出したファイルです。直すときは Web か sai-moon-paper sci を使います"
                .into(),
        );
        return Ok(());
    }
    let cmd = format!("nvim {}", shell_quote(&path.to_string_lossy()));
    open_in_nvim_pane(&path, &cmd);
    record_open_in_tui(runtime, &path, None);
    Ok(())
}

/// state.md が研究方針の木からの書き出しか（=`sci init` をしているか）。
#[must_use]
fn state_md_is_sci_backed(db: &papr_core::Database) -> bool {
    papr_core::sci::store::is_initialized(db.connection()).unwrap_or(false)
}

/// S01: 最近 7 日の研究日誌を内蔵 Markdown ビューアで開く（`J`）。
///
/// 記録から作るだけでファイルには保存しない。`Esc` で元の画面へ戻る。
fn apply_open_journal(runtime: &Runtime, app: &mut App, theme: &Theme) {
    let Some(roots) = papr_core::paths::paper_roots() else {
        app.toast = Some("プロジェクトルートが見つかりません".into());
        return;
    };
    open_journal_at(&runtime.database, &roots, app, theme);
}

/// [`apply_open_journal`] の本体（テストできるように DB とルートを受け取る）。
fn open_journal_at(
    db: &papr_core::Database,
    roots: &papr_core::PaperRoots,
    app: &mut App,
    theme: &Theme,
) {
    let today = chrono::Local::now().date_naive();
    let from = today - chrono::Duration::days(6);
    match papr_core::journal::collect(db.connection(), roots, from, today) {
        Ok(days) => {
            let content = papr_core::journal::render_markdown(&days);
            stash_md_view_before_replacing(app);
            app.md_view = Some(md_view::MarkdownView::from_markdown(
                &content,
                theme,
                "研究日誌".to_owned(),
                "最近 7 日".to_owned(),
            ));
            app.md_view_report = None;
            app.md_view_return = app.mode;
            app.mode = AppMode::MarkdownView;
        }
        Err(error) => {
            app.toast = Some(format!("研究日誌を作れませんでした: {error}"));
        }
    }
}

/// S02: 要対応の 1 件の元を開く（`Enter`）。
///
/// - review / 現況: `M` / `W` と同じく右のペインの nvim で md を開く。
/// - 質問 / 推薦のカード: 内蔵 Markdown ビューアで開く。
/// - 出典: PAPER 画面ではなく report の md を開く。
fn apply_open_attention(
    item: &papr_core::attention::AttentionItem,
    runtime: &Runtime,
    app: &mut App,
    theme: &Theme,
) -> Result<()> {
    match item.kind {
        papr_core::attention::AttentionKind::ReportIssue => {
            if let Some(report_id) = item.report_id.as_deref() {
                open_report_in_viewer(report_id, runtime, app, theme);
            } else {
                app.toast = Some("出典の report が分かりません".into());
            }
        }
        papr_core::attention::AttentionKind::EmptyReview
        | papr_core::attention::AttentionKind::StateUpdate => {
            let Some(path) = item.open_path.as_deref() else {
                app.toast = Some("開くファイルが分かりません".into());
                return Ok(());
            };
            // state.md は無ければひな形を作る（W と同じ）。
            if path.file_name().and_then(|name| name.to_str()) == Some("state.md")
                && let Err(error) = papr_core::project_files::ensure_one(
                    path,
                    papr_core::project_files::STATE_TEMPLATE,
                )
            {
                app.toast = Some(format!("state.md を作成できません: {error}"));
                return Ok(());
            }
            let cmd = format!("nvim {}", shell_quote(&path.to_string_lossy()));
            open_in_nvim_pane(path, &cmd);
            record_open_in_tui(runtime, path, None);
        }
        papr_core::attention::AttentionKind::NewSetItems => {
            // S03: 新着のあった検索セットを Search Sets 画面で開く。
            let Some(set_id) = attention_set_id(&item.key) else {
                app.toast = Some("検索セットが分かりません".into());
                return Ok(());
            };
            app.page = Page::SearchSets;
            app.mode = AppMode::Normal;
            app.content_focused = true;
            refresh_search_sets(runtime, app);
            if let Some(index) = app.search_sets.iter().position(|set| set.id == set_id) {
                app.search_set_selected = index;
                app.search_set_focused_on_items = true;
                app.search_set_items = papr_core::search_sets::items(
                    runtime.database.connection(),
                    set_id,
                )
                .unwrap_or_default();
            } else {
                app.toast = Some("検索セットが見つかりません".into());
            }
        }
        papr_core::attention::AttentionKind::SciDecision
        | papr_core::attention::AttentionKind::SciTriage => {
            open_sci_attention_at(&item, app);
        }
        papr_core::attention::AttentionKind::Question
        | papr_core::attention::AttentionKind::RecommendCard => {
            let Some(path) = item.open_path.as_deref() else {
                app.toast = Some("開くファイルが分かりません".into());
                return Ok(());
            };
            let body = match std::fs::read_to_string(path) {
                Ok(body) => body,
                Err(error) => {
                    app.toast = Some(format!("読み込みに失敗: {error}"));
                    return Ok(());
                }
            };
            stash_md_view_before_replacing(app);
            if app.mode != AppMode::MarkdownView {
                app.md_view_return = app.mode;
            }
            let title = format!("要対応 {}", item.kind.label());
            app.md_view = Some(md_view::MarkdownView::from_markdown(
                &body,
                theme,
                title,
                path.display().to_string(),
            ));
            app.md_view_report = None;
            app.mode = AppMode::MarkdownView;
            record_open_in_tui(runtime, path, None);
        }
    }
    Ok(())
}

/// 要対応の「判断待ち」「仕分けていない提案」を研究方針のページで開く（U23）。
///
/// [`apply_open_attention`] から使う。App だけ受け取るのでテストできる。
fn open_sci_attention_at(item: &papr_core::attention::AttentionItem, app: &mut App) {
    app.page = Page::Tree;
    app.mode = AppMode::Normal;
    app.content_focused = true;
    app.sidebar_index = Page::ALL
        .iter()
        .position(|page| *page == Page::Tree)
        .unwrap_or(app.sidebar_index);
    // SciDecision はキーが `sci_decision:<節点>:<知見の id>`。2 番目の節点を選ぶ。
    if item.kind == papr_core::attention::AttentionKind::SciDecision
        && let Some(node) = item
            .key
            .split(':')
            .nth(1)
            .and_then(|ref_id| papr_core::sci::parse_ref(ref_id, 'N').ok())
    {
        app.tree.pending_select = Some(node);
    }
    app.tree.stale = true;
}

/// S02: 要対応の 1 件を「対応済み」にする（`x`）。
fn apply_resolve_attention(
    item: &papr_core::attention::AttentionItem,
    runtime: &Runtime,
    app: &mut App,
) {
    let Some(roots) = runtime_paper_roots(runtime) else {
        app.toast = Some("プロジェクトルートが見つかりません".into());
        return;
    };
    match resolve_attention_at(&runtime.database, &roots, item, app) {
        Ok(()) => app.toast = Some("対応済みにしました".into()),
        Err(error) => app.toast = Some(format!("対応済みにできません: {error}")),
    }
}

/// [`apply_resolve_attention`] の本体（テストできるように DB とルートを受け取る）。
///
/// `resolved` を記録してから一覧を作り直し、選択行を範囲内に収める。
fn resolve_attention_at(
    db: &papr_core::Database,
    roots: &papr_core::PaperRoots,
    item: &papr_core::attention::AttentionItem,
    app: &mut App,
) -> Result<(), papr_core::attention::AttentionError> {
    papr_core::attention::resolve(db, item)?;
    app.project.attention = papr_core::attention::collect(db.connection(), roots)?;
    app.attention_selected = app
        .attention_selected
        .min(app.project.attention.len().saturating_sub(1));
    Ok(())
}

/// `runtime` からプロジェクトルート（`.sai/paper` の 2 つ上）を組み立てる。
fn runtime_paper_roots(runtime: &Runtime) -> Option<papr_core::PaperRoots> {
    let root = runtime.data_dir.parent().and_then(Path::parent)?;
    Some(papr_core::PaperRoots {
        root: root.to_path_buf(),
        data_dir: runtime.data_dir.clone(),
    })
}

/// ノートを herdr pane + nvim で開く（仕様 §3.4、`s` と同一パターン）。
/// `n` キーでのノート編集は外部エディタ（nvim）が正式採用。
fn open_note_in_nvim(runtime: &Runtime, paper_id: i64, paper_title: &str) -> Result<()> {
    // 書き込む前に論文フォルダを作る。
    let folder = paper_folder(&runtime.data_dir, paper_id, paper_title);
    std::fs::create_dir_all(&folder)?;
    let note_path = folder.join(paper_file_names::NOTE);
    if !note_path.exists() {
        // front matter 自動生成（論文タイトル・著者・リンク）。
        let mut content = String::new();
        if let Ok(Some(paper)) = runtime.database.library_paper_by_id(paper_id) {
            content.push_str(&format!("# {}\n\n", paper.title));
            if !paper.authors.is_empty() {
                content.push_str(&format!("- Authors: {}\n", paper.authors));
            }
            if let Some(arxiv) = &paper.arxiv_id {
                content.push_str(&format!("- arXiv: https://arxiv.org/abs/{arxiv}\n"));
            }
            if let Some(doi) = &paper.doi {
                content.push_str(&format!("- DOI: https://doi.org/{doi}\n"));
            }
            content.push_str("\n---\n\n");
        }
        std::fs::write(&note_path, content)?;
    }
    let cmd = format!("nvim {}", shell_quote(&note_path.to_string_lossy()));
    open_in_nvim_pane(&note_path, &cmd);
    record_open_in_tui(runtime, &note_path, Some(paper_id));
    Ok(())
}

/// `nvim <path>` を herdr pane で開く（herdr 不在時は直接 spawn）。
///
/// `cmd` は herdr pane へ送るコマンド文字列（呼び出し側でパスをクォートする）。
fn open_in_nvim_pane(path: &Path, cmd: &str) {
    let spawned = (|| -> Option<()> {
        let output = std::process::Command::new("herdr")
            .args(["pane", "split", "--direction", "right", "--no-focus"])
            .output()
            .ok()?;
        let value: serde_json::Value = serde_json::from_slice(&output.stdout).ok()?;
        let pane_id = value
            .get("result")?
            .get("pane")?
            .get("pane_id")?
            .as_str()?
            .to_string();
        std::thread::sleep(std::time::Duration::from_millis(300));
        std::process::Command::new("herdr")
            .args(["pane", "run", &pane_id, cmd])
            .spawn()
            .ok()?;
        Some(())
    })();
    if spawned.is_none() {
        // herdr 不在時は直接 spawn（フォールバック）。
        let _ = std::process::Command::new("nvim").arg(path).spawn();
    }
}

/// 最小限のシェルクォート。
fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn selected_local_paper_id(app: &App) -> Option<i64> {
    match app.page {
        Page::Library => app
            .filtered_library_papers()
            .get(app.library.selected)
            .map(|paper| paper.id),
        Page::Notes => app
            .filtered_notes_papers()
            .get(app.notes_selected)
            .map(|paper| paper.id),
        // R2: タグ → 論文一覧上でも t でタグモーダルを開けるようにする。
        Page::Collections if app.active_tag.is_some() => app
            .tag_papers
            .get(app.tag_paper_selected)
            .map(|paper| paper.id),
        _ => None,
    }
}

/// R2: Tags 画面（Collections ページ）のキー処理。
///
/// タグ一覧: `Enter` で選択中タグの論文一覧を開く。
/// 論文一覧: `Esc` / `h` でタグ一覧へ戻り、`Enter` で PDF を開き、
/// `t` で論文ごとのタグモーダルを開く（R1 と同じ動作）。
fn handle_collections_key(app: &mut App, key: KeyEvent) -> Option<UiAction> {
    if app.active_tag.is_some() {
        match key.code {
            KeyCode::Esc | KeyCode::Char('h') => {
                app.active_tag = None;
                app.tag_papers.clear();
                app.tag_paper_selected = 0;
                app.tag_paper_scroll = 0;
                Some(None)
            }
            KeyCode::Enter | KeyCode::Right | KeyCode::Char('l') => {
                let paper = app.tag_papers.get(app.tag_paper_selected)?;
                let path = paper.pdf_path.clone().map(PathBuf::from)?;
                Some(Some(UiAction::OpenPdf {
                    paper_id: paper.id,
                    path,
                    page: None,
                }))
            }
            KeyCode::Char('t') => {
                let paper = app.tag_papers.get(app.tag_paper_selected)?;
                Some(Some(UiAction::OpenTagModal(paper.id)))
            }
            KeyCode::Char('c') => {
                let paper = app.tag_papers.get(app.tag_paper_selected)?;
                Some(Some(UiAction::CopyCitation(PaperTarget::Local(paper.id))))
            }
            KeyCode::Char('x') => {
                let paper = app.tag_papers.get(app.tag_paper_selected)?;
                Some(Some(UiAction::ConfirmDeletePaper {
                    paper_id: paper.id,
                    title: paper.title.clone(),
                    path: paper.pdf_path.clone(),
                }))
            }
            _ => None,
        }
        .flatten()
    } else {
        match key.code {
            KeyCode::Enter | KeyCode::Right | KeyCode::Char('l') => {
                let name = app.tags.get(app.tag_selected)?.name.clone();
                Some(UiAction::OpenTagPaperList(name))
            }
            KeyCode::Char('t') => {
                // R2: 「t ADD TAG TO PAPER」— タグ一覧上で選択中論文へのタグ付与
                let paper_id = selected_local_paper_id(app)?;
                Some(UiAction::OpenTagModal(paper_id))
            }
            _ => None,
        }
    }
}

/// 検索セットの候補一覧を読み込む。
async fn apply_load_search_set_items(
    set_id: i64,
    runtime: &mut Runtime,
    app: &mut App,
) -> Result<()> {
    let conn = runtime.database.connection();
    match papr_core::search_sets::items(conn, set_id) {
        Ok(items) => {
            app.search_set_items = items;
            app.search_set_item_selected = 0;
            app.search_set_item_scroll = 0;
            Ok(())
        }
        Err(e) => {
            app.toast = Some(format!("Failed to load search set items: {}", e));
            Ok(())
        }
    }
}

/// 検索セット候補の decision を更新する。
async fn apply_mark_search_set_item(
    set_id: i64,
    position: i64,
    decision: String,
    runtime: &mut Runtime,
    app: &mut App,
) -> Result<()> {
    let conn = runtime.database.connection();
    match papr_core::search_sets::mark(conn, set_id, position, &decision, None) {
        Ok(_) => {
            app.toast = Some(format!("Marked as {}", decision));
            // Update the local copy
            if let Some(item) = app
                .search_set_items
                .iter_mut()
                .find(|i| i.position == position)
            {
                item.decision = decision;
            }
            Ok(())
        }
        Err(e) => {
            app.toast = Some(format!("Failed to mark item: {}", e));
            Ok(())
        }
    }
}

/// S03: 検索セットの再実行（`r`）を裏で始める。
///
/// `Database`（rusqlite Connection）は `Sync` でないため `tokio::spawn` は使えない。
/// 専用スレッド + current-thread ランタイムで [`papr_core::search_sets::rerun_set`] を
/// 実行し、終わったら [`AppEvent::SearchSetRerun`] で本体へ返す。
fn start_search_set_rerun(
    set_id: i64,
    runtime: &Runtime,
    senders: &ActionSenders,
    app: &mut App,
) {
    let Some(set) = app.search_sets.iter().find(|set| set.id == set_id) else {
        app.toast = Some("検索セットが見つかりません".into());
        return;
    };
    let set_name = set.name.clone();
    app.toast = Some(format!("検索セット \"{set_name}\" をもう一度検索しています…"));

    let config = runtime.config.clone();
    let db_file = runtime.database_file.clone();
    let app_events = senders.app_events.clone();
    std::thread::spawn(move || {
        let send = |result: std::result::Result<
            papr_core::search_sets::RerunResult,
            String,
        >| {
            let _ = app_events.send(AppEvent::SearchSetRerun { set_id, result });
        };
        let db = match papr_core::Database::open(&db_file) {
            Ok(db) => db,
            Err(error) => return send(Err(format!("DB を開けません: {error}"))),
        };
        let rt = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(rt) => rt,
            Err(error) => return send(Err(format!("ランタイムを作れません: {error}"))),
        };
        let result = rt
            .block_on(papr_core::search_sets::rerun_set(&db, &config, &set_name))
            .map_err(|error| error.to_string());
        send(result);
    });
}

/// S03: 検索セットの読み込みをやり直す。
fn refresh_search_sets(runtime: &Runtime, app: &mut App) {
    refresh_search_sets_at(runtime.database.connection(), app);
}

/// [`refresh_search_sets`] の本体（テストできるように接続を受け取る）。
fn refresh_search_sets_at(conn: &rusqlite::Connection, app: &mut App) {
    match papr_core::search_sets::list(conn) {
        Ok(sets) => {
            app.search_sets = sets;
            app.search_set_selected = app
                .search_set_selected
                .min(app.search_sets.len().saturating_sub(1));
            if let Some(set) = app.search_sets.get(app.search_set_selected) {
                app.search_set_items =
                    papr_core::search_sets::items(conn, set.id).unwrap_or_default();
            } else {
                app.search_set_items.clear();
            }
            app.search_set_item_selected = app
                .search_set_item_selected
                .min(app.search_set_items.len().saturating_sub(1));
        }
        Err(error) => app.toast = Some(format!("検索セットの読み込みに失敗: {error}")),
    }
}

/// S03: 再実行の結果を画面に反映する（トーストと一覧の読み直し）。
fn apply_search_set_rerun_result_at(
    conn: &rusqlite::Connection,
    set_id: i64,
    result: std::result::Result<papr_core::search_sets::RerunResult, String>,
    app: &mut App,
) {
    match result {
        Ok(result) => {
            let failed = if result.failed_dbs.is_empty() {
                String::new()
            } else {
                format!("、失敗した DB: {}", result.failed_dbs.join(", "))
            };
            app.toast = Some(format!(
                "新着 {} 本（検索 {} 件{}）",
                result.new_items, result.total_hits, failed
            ));
            refresh_search_sets_at(conn, app);
            if let Some(index) = app.search_sets.iter().position(|set| set.id == set_id) {
                app.search_set_selected = index;
                app.search_set_items =
                    papr_core::search_sets::items(conn, set_id).unwrap_or_default();
            }
        }
        Err(error) => app.toast = Some(format!("もう一度検索できません: {error}")),
    }
}

/// 要対応の `key`（`set:<set_id>:<searched_at>`）から set_id を取り出す。
fn attention_set_id(key: &str) -> Option<i64> {
    let mut parts = key.splitn(3, ':');
    if parts.next() != Some("set") {
        return None;
    }
    parts.next()?.parse().ok()
}

/// RAG ハイブリッド検索のキー入力処理。
fn handle_rag_search_key(app: &mut App, key: KeyEvent) -> Option<UiAction> {
    match key.code {
        KeyCode::Esc => {
            // 検索を中止して元のモードに戻る。
            app.mode = AppMode::Normal;
            app.rag_search = state::RagSearchState::default();
            None
        }
        KeyCode::Char(c) => {
            // Ctrl+j / Ctrl+k で修飾キー付きの場合は先に処理
            if key.modifiers.contains(KeyModifiers::CONTROL) {
                match c {
                    'j' => {
                        // Ctrl-j: Down と同じ
                        if !app.rag_search.results.is_empty() {
                            app.rag_search.selected =
                                (app.rag_search.selected + 1) % app.rag_search.results.len();
                            let visible_height = 10;
                            if app.rag_search.selected >= app.rag_search.scroll + visible_height {
                                app.rag_search.scroll =
                                    app.rag_search.selected.saturating_sub(visible_height - 1);
                            }
                        }
                        return None;
                    }
                    'k' => {
                        // Ctrl-k: Up と同じ
                        if !app.rag_search.results.is_empty() {
                            app.rag_search.selected = if app.rag_search.selected == 0 {
                                app.rag_search.results.len() - 1
                            } else {
                                app.rag_search.selected - 1
                            };
                            if app.rag_search.selected < app.rag_search.scroll {
                                app.rag_search.scroll = app.rag_search.selected;
                            }
                        }
                        return None;
                    }
                    'e' => {
                        // Ctrl-e: 要約付きの検索を切り替える（次の Enter の検索から効く）。
                        // 文字入力と衝突しないよう Ctrl 付きにする。
                        app.rag_search.evidence = !app.rag_search.evidence;
                        // 前の結果は古いモードのものなので消す（表示が混ざらないように）。
                        app.rag_search.results.clear();
                        app.rag_search.summaries.clear();
                        app.rag_search.relevances.clear();
                        app.rag_search.dropped = 0;
                        app.rag_search.prompt_tokens = 0;
                        app.rag_search.completion_tokens = 0;
                        app.rag_search.failed = 0;
                        app.rag_search.selected = 0;
                        app.rag_search.scroll = 0;
                        app.toast = Some(if app.rag_search.evidence {
                            "要約付き: ON（Enter で検索。費用がかかります）".into()
                        } else {
                            "要約付き: OFF".into()
                        });
                        return None;
                    }
                    _ => {}
                }
            }
            // 結果選択中のとき'm'はMarkdownを開く
            if c == 'm'
                && !app.rag_search.results.is_empty()
                && app.rag_search.selected < app.rag_search.results.len()
            {
                let hit = &app.rag_search.results[app.rag_search.selected];
                // 図の結果は、その論文の PAPER 画面（図の一覧）を開く。
                if hit.source_kind == "figure" {
                    if let Some(paper_id) = hit.paper_id {
                        return Some(UiAction::OpenLibraryPaper(paper_id));
                    }
                    app.toast = Some("この図には論文が紐付いていません".into());
                    return None;
                }
                if let Some(md_path) = &hit.md_path {
                    return Some(UiAction::OpenRagSearchMarkdown(md_path.clone()));
                } else {
                    app.toast = Some("Markdownパスが紐付いていません".into());
                    return None;
                }
            }
            // 通常の文字入力（入力欄の編集）
            app.rag_search.query.insert(app.rag_search.query_cursor, c);
            app.rag_search.query_cursor += c.len_utf8();
            None
        }
        KeyCode::Backspace => {
            if app.rag_search.query_cursor > 0 {
                let prev_cursor = app
                    .rag_search
                    .query
                    .char_indices()
                    .rev()
                    .find(|(i, _)| *i < app.rag_search.query_cursor)
                    .map(|(i, _)| i)
                    .unwrap_or(0);
                app.rag_search.query.truncate(prev_cursor);
                app.rag_search.query_cursor = prev_cursor;
            }
            None
        }
        KeyCode::Tab | KeyCode::Down => {
            // 結果リストを選択状態へ、または次の結果へ移動
            if !app.rag_search.results.is_empty() {
                app.rag_search.selected =
                    (app.rag_search.selected + 1) % app.rag_search.results.len();
                // スクロール追従
                let visible_height = 10; // 概算値
                if app.rag_search.selected >= app.rag_search.scroll + visible_height {
                    app.rag_search.scroll =
                        app.rag_search.selected.saturating_sub(visible_height - 1);
                }
            }
            None
        }
        KeyCode::Up => {
            // 前の結果へ移動
            if !app.rag_search.results.is_empty() {
                app.rag_search.selected = if app.rag_search.selected == 0 {
                    app.rag_search.results.len() - 1
                } else {
                    app.rag_search.selected - 1
                };
                // スクロール追従
                if app.rag_search.selected < app.rag_search.scroll {
                    app.rag_search.scroll = app.rag_search.selected;
                }
            }
            None
        }
        KeyCode::Enter => {
            if app.rag_search.results.is_empty() {
                // 結果がない場合は検索を実行
                Some(UiAction::ExecuteRagSearch)
            } else {
                // 結果がある場合は選択中の論文を開く
                let hit = &app.rag_search.results[app.rag_search.selected];
                if let Some(paper_id) = hit.paper_id {
                    Some(UiAction::OpenLibraryPaper(paper_id))
                } else {
                    app.toast = Some("論文に紐づかないヒットです".into());
                    None
                }
            }
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {

    /// `offset` を見ない DB が同じ 100 件を返し続けても、ページ送りは 1 回で終わる。
    #[test]
    fn should_fetch_next_stops_when_no_new_papers() {
        use super::{DISCOVERY_CANDIDATE_LIMIT, should_fetch_next};

        // 初回: 100 件的新規論文 → 次がある
        assert!(should_fetch_next(0, 100, 100, 100));
        // バッチ未満（最後まで来た）→ 止める
        assert!(!should_fetch_next(0, 42, 42, 100));
        // `offset` を無視して同じ 100 件が返る（件数が増えない）→ 止める
        assert!(!should_fetch_next(100, 100, 100, 100));
        // 上限に達したら止める
        let limit = DISCOVERY_CANDIDATE_LIMIT as usize;
        assert!(!should_fetch_next(limit - 50, limit, 100, 100));
    }

    /// Discover で取り込んだ論文を、いまの検索の記録に紐付ける（Library の階層表示用）。
    ///
    /// 記録に候補が無いときは、いまの検索の結果にある場合だけ足してから紐付ける。
    #[test]
    fn link_to_discover_log_links_result_paper_to_current_search()
    -> Result<(), Box<dyn std::error::Error>> {
        use super::{link_to_discover_log_inner, search_log_item_position};
        use papr_core::Database;

        let mut db = Database::in_memory()?;
        let mut app = App::default();

        let remote = remote_paper("discover-link", "Discover Link Target");
        let other = remote_paper("discover-other", "Not In Results");

        // 記録は検索語が空・名前だけのもの（CLI が作る検索セットと同じ形）。
        let log_id = {
            let conn = db.connection();
            papr_core::search_log::record_search(
                conn,
                "discover",
                "",
                "",
                "",
                "arxiv",
                std::slice::from_ref(&remote),
            )?
        };
        app.discovery.search_log_id = Some(log_id);

        // 記録に候補が無い論文は、Discover の結果に無ければ紐付けない。
        app.discovery.results = vec![remote.clone()];
        let other_id = db.ensure_remote_paper(&other)?;
        let remote_id = db.ensure_remote_paper(&remote)?;
        let conn = db.connection();

        link_to_discover_log_inner(conn, log_id, &app, &other, other_id)?;
        assert_eq!(
            search_log_item_position(conn, log_id, &papr_core::dedupe_key(&other)),
            None
        );

        // Discover の結果にある論文は、記録に足してから紐付く。
        link_to_discover_log_inner(conn, log_id, &app, &remote, remote_id)?;
        let items = papr_core::search_log::items(conn, log_id)?;
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].paper_id, Some(remote_id));
        let decision: String = conn.query_row(
            "SELECT decision FROM search_set_items WHERE set_id = ?1 AND position = 0",
            [log_id],
            |row| row.get(0),
        )?;
        assert_eq!(decision, "downloaded", "紐付けのときに decision も入る");
        Ok(())
    }

    #[test]
    fn record_open_dedups_within_window() -> Result<(), Box<dyn std::error::Error>> {
        let root = std::env::temp_dir().join(format!(
            "papr-open-record-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_nanos()
        ));
        let data_dir = root.join(".sai").join("paper");
        // `space n` が開く note.md と同じ場所（`papers/0001_<title>/note.md`）。
        let note = data_dir.join("papers").join("0001_Test").join("note.md");
        fs::create_dir_all(note.parent().expect("note has a parent"))?;
        fs::write(&note, "# note\n")?;
        let mut database = Database::open(&data_dir.join("papr.db"))?;
        // activity_log.paper_id は papers.id を参照するので、先に論文を 1 件作る。
        database.ensure_remote_paper(&remote_paper("open-test", "Test"))?;

        // 内蔵ビューア / nvim / `n` のすべての入口は record_open_at を通る。
        record_open_at(&database, &root, &data_dir, &note, None)?;
        record_open_at(&database, &root, &data_dir, &note, None)?;

        let count: i64 = database.connection().query_row(
            "SELECT COUNT(*) FROM activity_log WHERE kind = 'open'",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(count, 1, "同じ note を 10 分以内に 2 回開いても 1 件になる");

        let (detail, paper_id): (String, Option<i64>) = database.connection().query_row(
            "SELECT detail, paper_id FROM activity_log WHERE kind = 'open'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        let value: serde_json::Value = serde_json::from_str(&detail)?;
        assert_eq!(value["target"], "note");
        assert_eq!(value["surface"], "tui");
        assert_eq!(value["path"], "papers/0001_Test/note.md");
        assert_eq!(paper_id, Some(1));

        fs::remove_dir_all(&root)?;
        Ok(())
    }

    #[test]
    fn shift_k_toggles_library_hierarchy_view() {
        let mut app = App {
            page: Page::Library,
            content_focused: true,
            ..App::default()
        };
        assert_eq!(app.library_view, LibraryView::Flat);
        let _ = handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('K'), KeyModifiers::SHIFT),
        );
        assert_eq!(app.library_view, LibraryView::Hierarchy);
        assert!(app.library_tree_dirty, "切り替え時に再構築フラグが立つ");
        let _ = handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('K'), KeyModifiers::SHIFT),
        );
        assert_eq!(app.library_view, LibraryView::Flat);
    }

    fn query_tree_row() -> LibraryTreeRow {
        LibraryTreeRow {
            depth: 0,
            kind: LibraryTreeKind::Query,
            label: "2026/09/21  LLM Agent".into(),
            child_count: 2,
            fold_key: Some("log:1".into()),
            log_id: Some(1),
            paper_id: None,
            year_month: String::new(),
            citation_count: None,
            journal: None,
            title: String::new(),
        }
    }

    #[test]
    fn hierarchy_query_row_enter_opens_query_detail_and_space_folds() {
        let mut app = App {
            page: Page::Library,
            content_focused: true,
            ..App::default()
        };
        app.library_view = LibraryView::Hierarchy;
        app.library_tree = vec![query_tree_row()];

        // space で query 行を折りたたむ。
        let folded = handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE),
        );
        assert!(folded.is_none());
        assert!(app.library_tree_folded.contains("log:1"));

        // Enter で QUERY 画面を開くアクションを返す。
        let action = handle_key(&mut app, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(matches!(action, Some(UiAction::OpenQueryDetail(1))));
    }

    #[test]
    fn query_detail_enter_opens_downloaded_local_paper() {
        let mut app = App {
            page: Page::Library,
            mode: AppMode::QueryDetail,
            ..App::default()
        };
        app.query_detail.items = vec![QueryDetailItem {
            remote: remote_paper("https://arxiv.org/abs/1", "Downloaded"),
            local_id: Some(42),
        }];
        let action = handle_key(&mut app, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(matches!(action, Some(UiAction::OpenLibraryPaper(42))));
    }

    #[test]
    fn query_detail_enter_opens_remote_paper_when_not_downloaded() {
        let mut app = App {
            page: Page::Library,
            mode: AppMode::QueryDetail,
            ..App::default()
        };
        app.query_detail.items = vec![QueryDetailItem {
            remote: remote_paper("https://arxiv.org/abs/9", "Not downloaded"),
            local_id: None,
        }];
        let action = handle_key(&mut app, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(matches!(action, Some(UiAction::OpenPaper(_))));
    }
    #[test]
    fn discover_enter_opens_the_selected_paper_not_the_head() {
        let mut app = App {
            page: Page::Discover,
            content_focused: true,
            mode: AppMode::Normal,
            discovery: DiscoveryState {
                query: "q".into(),
                status: DiscoveryStatus::Ready,
                results: vec![
                    remote_paper("https://arxiv.org/abs/1", "Head paper"),
                    remote_paper("https://arxiv.org/abs/2", "Middle paper"),
                    remote_paper("https://arxiv.org/abs/3", "Selected paper"),
                ],
                selected: 2,
                ..DiscoveryState::default()
            },
            ..App::default()
        };
        let action = handle_key(&mut app, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        match action {
            Some(UiAction::OpenPaper(p)) => assert_eq!(p.title, "Selected paper"),
            other => panic!("unexpected action: {:?}", other.is_some()),
        }
    }

    #[test]
    fn discover_paper_detail_renders_the_selected_paper() {
        // 選択中 (selected=2) の論文が PAPER 詳細に描かれることを確認する。
        let mut app = App {
            page: Page::Discover,
            content_focused: true,
            mode: AppMode::PaperDetail,
            discovery: DiscoveryState {
                query: "q".into(),
                status: DiscoveryStatus::Ready,
                results: vec![
                    remote_paper("https://arxiv.org/abs/1", "Head paper"),
                    remote_paper("https://arxiv.org/abs/2", "Middle paper"),
                    remote_paper("https://arxiv.org/abs/3", "Selected paper"),
                ],
                selected: 2,
                ..DiscoveryState::default()
            },
            ..App::default()
        };
        let theme = crate::theme::Theme::load("catppuccin-mocha").unwrap();
        let backend = ratatui::backend::TestBackend::new(120, 40);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| crate::ui::render(frame, &mut app, &theme))
            .unwrap();
        let text = terminal.backend().to_string().replace('\n', " ");
        assert!(
            text.contains("Selected paper"),
            "detail must show the selected paper"
        );
        assert!(
            !text.contains("Head paper"),
            "detail must not show the list head"
        );
    }

    #[test]
    fn discover_paper_detail_renders_the_stored_opened_paper() {
        // OpenPaper で保持した論文が、検索結果更新で選択位置が変わっても
        // PAPER 詳細に正しく表示されること（リスト先頭を誤表示しない）。
        let mut app = App {
            page: Page::Discover,
            content_focused: true,
            mode: AppMode::PaperDetail,
            discovery: DiscoveryState {
                query: "q".into(),
                status: DiscoveryStatus::Ready,
                results: vec![
                    remote_paper("https://arxiv.org/abs/1", "Head paper"),
                    remote_paper("https://arxiv.org/abs/3", "Opened paper"),
                ],
                selected: 0,
                ..DiscoveryState::default()
            },
            paper_detail_remote: Some(remote_paper("https://arxiv.org/abs/3", "Opened paper")),
            ..App::default()
        };
        let theme = crate::theme::Theme::load("catppuccin-mocha").unwrap();
        let backend = ratatui::backend::TestBackend::new(120, 40);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| crate::ui::render(frame, &mut app, &theme))
            .unwrap();
        let text = terminal.backend().to_string().replace('\n', " ");
        assert!(
            text.contains("Opened paper"),
            "detail must show the opened paper"
        );
        assert!(
            !text.contains("Head paper"),
            "detail must not show the list head"
        );

        // Back で戻ると保持をクリアする。
        app.dispatch(Command::Back);
        assert!(app.paper_detail_remote.is_none());
    }

    use crate::state::*;
    use std::{ffi::OsString, fs, path::PathBuf, thread, time::Duration};

    use crate::terminal_input::{parse_command, sanitize_terminal_output};
    use chrono::{TimeZone, Utc};
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use papr_core::{Database, LibraryPaper, PaperRoots, RemotePaper};

    use super::{
        AgentCard, AgentCardOutcome, ConfigFilesystemWatcher, PaperTarget, ProjectFile, UiAction,
        apply_create_agent_card_at, apply_search_set_rerun_result_at, attention_set_id,
        complete_terminal_command, create_agent_card, handle_key, handle_manual_import_key,
        handle_mouse_event, handle_paper_detail_key, is_text_paste_shortcut,
        merge_enriched_remote_paper, open_journal_at, open_sci_attention_at,
        paste_text_into_active_input, pdf_viewer_invocation, record_open_at, report_header_line,
        resolve_attention_at, run_terminal_command, state_md_is_sci_backed,
    };

    #[test]
    fn config_watcher_observes_direct_and_atomic_saves() -> Result<(), Box<dyn std::error::Error>> {
        let temp_dir = std::env::temp_dir().join(format!(
            "papr-config-watch-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_nanos()
        ));
        fs::create_dir_all(&temp_dir)?;
        let config_file = temp_dir.join("settings.json");
        fs::write(&config_file, "theme = 'nord'\n")?;
        let watcher = ConfigFilesystemWatcher::start(&config_file)?;

        fs::write(&config_file, "theme = 'dracula'\n")?;
        assert!(wait_for_config_event(&watcher));

        let temporary = temp_dir.join(".settings.json.tmp");
        fs::write(&temporary, "theme = 'light'\n")?;
        fs::rename(&temporary, &config_file)?;
        assert!(wait_for_config_event(&watcher));

        fs::remove_dir_all(temp_dir)?;
        Ok(())
    }

    fn wait_for_config_event(watcher: &ConfigFilesystemWatcher) -> bool {
        for _ in 0..20 {
            if watcher.has_changes() {
                return true;
            }
            thread::sleep(Duration::from_millis(25));
        }
        false
    }

    #[test]
    fn test_word_wise_and_line_editing_navigation() {
        use super::{edit_text, next_word_boundary, prev_word_boundary};

        let text = "hello world  rust_programming  123";
        // Test word boundaries
        assert_eq!(next_word_boundary(text, 0), 6); // start of "world"
        assert_eq!(next_word_boundary(text, 6), 13); // start of "rust_programming"
        assert_eq!(next_word_boundary(text, 13), 31); // start of "123"

        assert_eq!(prev_word_boundary(text, 34), 31); // start of "123"
        assert_eq!(prev_word_boundary(text, 31), 13); // start of "rust_programming"
        assert_eq!(prev_word_boundary(text, 13), 6); // start of "world"

        let mut buffer = "first second\nthird fourth".to_owned();
        let mut cursor = 6;

        // Test Home
        edit_text(
            &mut buffer,
            &mut cursor,
            KeyEvent::new(KeyCode::Home, KeyModifiers::NONE),
        );
        assert_eq!(cursor, 0);

        // Test End
        edit_text(
            &mut buffer,
            &mut cursor,
            KeyEvent::new(KeyCode::End, KeyModifiers::NONE),
        );
        assert_eq!(cursor, 12); // end of "first second"

        // Test Ctrl + Left
        edit_text(
            &mut buffer,
            &mut cursor,
            KeyEvent::new(KeyCode::Left, KeyModifiers::CONTROL),
        );
        assert_eq!(cursor, 6); // start of "second"

        // Test Ctrl + Right
        edit_text(
            &mut buffer,
            &mut cursor,
            KeyEvent::new(KeyCode::Right, KeyModifiers::CONTROL),
        );
        assert_eq!(cursor, 13);
    }

    #[test]
    fn enrichment_projection_keeps_the_displayed_abstract_when_provider_has_none() {
        let mut displayed = remote_paper("https://arxiv.org/abs/2602.00004", "Original title");
        displayed.abstract_text = "Abstract fetched from arXiv.".into();
        let mut provider = displayed.clone();
        provider.title = "Enriched title".into();
        provider.abstract_text = "  ".into();

        let merged = merge_enriched_remote_paper(&displayed, &provider);

        assert_eq!(merged.title, "Enriched title");
        assert_eq!(merged.abstract_text, "Abstract fetched from arXiv.");
    }

    #[test]
    fn control_b_opens_browse_papr() {
        let mut app = App::default();
        let _ = handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('b'), KeyModifiers::CONTROL),
        );
        assert_eq!(app.mode, AppMode::CommandPalette);
    }

    #[test]
    fn palette_navigates_options() {
        let mut app = App {
            mode: AppMode::CommandPalette,
            ..App::default()
        };
        let _ = handle_key(&mut app, KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        let _ = handle_key(&mut app, KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        assert_eq!(app.palette_selected, 2);
    }

    #[test]
    fn palette_filtering_and_typing() {
        let mut app = App {
            mode: AppMode::CommandPalette,
            ..App::default()
        };

        // Type 'l' to filter (should match Library, etc.)
        let _ = handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('l'), KeyModifiers::NONE),
        );
        assert_eq!(app.palette_query, "l");
        let items = app.filtered_palette_items();
        assert!(items.contains(&Page::Library));

        // Down arrow should move selection
        let _ = handle_key(&mut app, KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        // Press Enter to activate
        let action = handle_key(&mut app, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(action.is_none());
        assert_eq!(app.mode, AppMode::Normal);
    }

    #[test]
    fn slash_opens_discovery_search() {
        let mut app = App::default();
        let action = handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('/'), KeyModifiers::NONE),
        );
        assert!(action.is_none());
        assert_eq!(app.mode, AppMode::Search);
        assert_eq!(app.page, Page::Discover);
    }

    #[test]
    fn dashboard_navigation_opens_the_selected_paper() {
        // Dashboard papers UI removed: Enter should not open PAPER
        let first = remote_paper("https://arxiv.org/abs/1", "First paper");
        let second = remote_paper("https://arxiv.org/abs/2", "Selected paper");
        let mut app = App {
            page: Page::Dashboard,
            content_focused: true,
            today_papers: vec![first, second],
            today_selected: 1,
            ..App::default()
        };

        let action = handle_key(&mut app, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(action.is_none());
        assert_eq!(app.mode, AppMode::Normal);
    }

    #[test]
    fn remote_workspace_detail_ignores_bookmark_note_tag_and_group_keys() {
        for page in [Page::Dashboard, Page::Discover] {
            let mut app = App {
                page,
                content_focused: true,
                mode: AppMode::PaperDetail,
                today_papers: vec![remote_paper(
                    "https://arxiv.org/abs/dashboard",
                    "Dashboard paper",
                )],
                discovery: DiscoveryState {
                    results: vec![remote_paper(
                        "https://arxiv.org/abs/discover",
                        "Discover paper",
                    )],
                    ..DiscoveryState::default()
                },
                ..App::default()
            };

            for key in ['B', 'n', 't', 'g'] {
                let action = handle_key(
                    &mut app,
                    KeyEvent::new(KeyCode::Char(key), KeyModifiers::NONE),
                );
                assert!(action.is_none(), "{key} should be ignored in {page:?}");
                assert_eq!(app.mode, AppMode::PaperDetail);
            }
        }
    }

    #[test]
    fn enter_and_right_open_every_navigation_section() {
        for (index, page) in Page::ALL.into_iter().enumerate() {
            for key in [KeyCode::Enter, KeyCode::Right] {
                let mut app = App {
                    sidebar_index: index,
                    ..App::default()
                };
                let action = handle_key(&mut app, KeyEvent::new(key, KeyModifiers::NONE));
                assert!(action.is_none());
                assert_eq!(app.page, page);
                assert!(app.content_focused);
            }
        }
    }

    #[test]
    fn left_returns_to_navigation_without_changing_selection() {
        for (index, page) in Page::ALL.into_iter().enumerate() {
            let mut app = App {
                page,
                sidebar_index: index,
                content_focused: true,
                ..App::default()
            };
            let action = handle_key(&mut app, KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
            assert!(action.is_none());
            assert_eq!(app.page, page);
            assert_eq!(app.sidebar_index, index);
            assert!(!app.content_focused);
        }
    }

    #[test]
    fn test_workspace_search_navigation_and_restore() {
        use std::collections::HashSet;
        for page in [Page::Library, Page::Collections, Page::Authors, Page::Notes] {
            let mut app = App {
                page,
                content_focused: true,
                mode: AppMode::WorkspaceSearch,
                workspace: AppWorkspaceState {
                    workspace_query: "test query".to_string(),
                    workspace_query_cursor: 10,
                    active_search_workspaces: {
                        let mut s = HashSet::new();
                        s.insert(page);
                        s
                    },
                    ..AppWorkspaceState::default()
                },
                ..App::default()
            };

            // Pressing Left Arrow in WorkspaceSearch mode when cursor > 0 should move cursor left
            let action = handle_key(&mut app, KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
            assert!(action.is_none());
            assert!(app.content_focused);
            assert_eq!(app.mode, AppMode::WorkspaceSearch);
            assert_eq!(app.workspace_query_cursor, 9);

            // Pressing Left Arrow when cursor == 0 should transfer focus to navigation pane
            app.workspace_query_cursor = 0;
            let action = handle_key(&mut app, KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
            assert!(action.is_none());
            assert!(!app.content_focused);
            assert_eq!(app.mode, AppMode::Normal);
            assert_eq!(app.workspace_query, "test query");
            assert!(app.active_search_workspaces.contains(&page));

            // Move to another page
            app.sidebar_index = 0; // Dashboard
            app.page = Page::Dashboard;

            // Enter Dashboard - should not restore search mode since it's not active
            app.dispatch(Command::Open);
            assert!(app.content_focused);
            assert_eq!(app.mode, AppMode::Normal);

            // Go back to sidebar, select original page, and enter it
            app.content_focused = false;
            if let Some(index) = Page::ALL.iter().position(|&p| p == page) {
                app.sidebar_index = index;
            }
            app.dispatch(Command::Open);

            // It should enter the workspace and restore search mode and its state!
            assert!(app.content_focused);
            assert_eq!(app.mode, AppMode::WorkspaceSearch);
            assert_eq!(app.workspace_query, "test query");
            assert_eq!(app.workspace_query_cursor, 0);
        }
    }

    #[test]
    fn test_discover_search_navigation() {
        let mut app = App {
            page: Page::Discover,
            content_focused: true,
            mode: AppMode::Search,
            discovery: DiscoveryState {
                query: "test query".to_string(),
                query_cursor: 10,
                ..DiscoveryState::default()
            },
            ..App::default()
        };

        // Pressing Left Arrow when cursor > 0 should move cursor left
        let action = handle_key(&mut app, KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
        assert!(action.is_none());
        assert!(app.content_focused);
        assert_eq!(app.mode, AppMode::Search);
        assert_eq!(app.discovery.query_cursor, 9);

        // Pressing Left Arrow when cursor == 0 should transfer focus to navigation pane
        app.discovery.query_cursor = 0;
        let action = handle_key(&mut app, KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
        assert!(action.is_none());
        assert!(!app.content_focused);
        assert_eq!(app.mode, AppMode::Normal);
    }

    #[test]
    fn discover_results_left_returns_to_navigation_and_preserves_state() {
        for from_filter in [false, true] {
            let mut app = App {
                page: Page::Discover,
                sidebar_index: Page::ALL
                    .iter()
                    .position(|&page| page == Page::Discover)
                    .unwrap_or_default(),
                content_focused: true,
                mode: if from_filter {
                    AppMode::DiscoverFilter
                } else {
                    AppMode::Search
                },
                discovery: DiscoveryState {
                    query: "quantum search".into(),
                    query_cursor: 14,
                    filter: "paper".into(),
                    filter_cursor: 5,
                    ..DiscoveryState::default()
                },
                ..App::default()
            };
            app.discovery.set_results(vec![
                remote_paper("first", "First paper"),
                remote_paper("second", "Second paper"),
            ]);
            app.discovery.filter = "paper".into();
            app.discovery.filter_cursor = app.discovery.filter.len();
            app.discovery.rebuild_filter();

            // Both inputs move into the same results pane.
            assert!(
                handle_key(&mut app, KeyEvent::new(KeyCode::Down, KeyModifiers::NONE)).is_none()
            );
            assert_eq!(app.mode, AppMode::Normal);
            assert_eq!(app.discovery.selected, 0);
            app.discovery.scroll = 1;

            // Left leaves the results pane for navigation without entering either input.
            assert!(
                handle_key(&mut app, KeyEvent::new(KeyCode::Left, KeyModifiers::NONE)).is_none()
            );
            assert!(!app.content_focused);
            assert_eq!(app.mode, AppMode::Normal);
            assert_eq!(app.discovery.query, "quantum search");
            assert_eq!(app.discovery.filter, "paper");
            assert_eq!(app.discovery.selected, 0);
            assert_eq!(app.discovery.scroll, 1);

            // Leaving Discover and returning restores the existing results-pane state.
            app.dispatch(Command::MoveDown);
            app.dispatch(Command::MoveUp);
            app.dispatch(Command::Open);
            assert!(app.content_focused);
            assert_eq!(app.page, Page::Discover);
            assert_eq!(app.mode, AppMode::Normal);
            assert_eq!(app.discovery.query, "quantum search");
            assert_eq!(app.discovery.filter, "paper");
            assert_eq!(app.discovery.selected, 0);
            assert_eq!(app.discovery.scroll, 1);
        }
    }

    #[test]
    fn discover_control_arrows_switch_cached_result_pages() {
        let mut app = App {
            page: Page::Discover,
            content_focused: true,
            ..App::default()
        };
        app.discovery.set_results(
            (0..51)
                .map(|index| {
                    remote_paper(
                        &format!("https://arxiv.org/abs/{index}"),
                        &format!("Paper {index}"),
                    )
                })
                .collect(),
        );

        assert!(
            handle_key(
                &mut app,
                KeyEvent::new(KeyCode::Right, KeyModifiers::CONTROL)
            )
            .is_none()
        );
        assert_eq!(app.discovery.page, 1);
        assert_eq!(app.discovery.current_page_results()[0].title, "Paper 50");
        assert!(
            handle_key(
                &mut app,
                KeyEvent::new(KeyCode::Left, KeyModifiers::CONTROL)
            )
            .is_none()
        );
        assert_eq!(app.discovery.page, 0);
        assert_eq!(app.discovery.current_page_results()[0].title, "Paper 0");
    }

    #[test]
    fn workspace_search_accepts_printable_shift_period_key_events() {
        let mut app = App {
            page: Page::Library,
            content_focused: true,
            ..App::default()
        };
        let key = KeyEvent::new(KeyCode::Char('>'), KeyModifiers::SHIFT);

        let _ = handle_key(&mut app, key);
        assert_eq!(app.mode, AppMode::WorkspaceSearch);

        let _ = handle_key(&mut app, key);
        assert_eq!(app.mode, AppMode::Normal);
    }

    #[test]
    fn paste_text_reaches_discovery_and_workspace_search_fields() {
        let mut discovery = App {
            page: Page::Discover,
            content_focused: true,
            mode: AppMode::Search,
            discovery: DiscoveryState {
                query: "ac".into(),
                query_cursor: 1,
                ..DiscoveryState::default()
            },
            ..App::default()
        };
        assert!(paste_text_into_active_input(&mut discovery, "β"));
        assert_eq!(discovery.discovery.query, "aβc");
        assert_eq!(discovery.discovery.query_cursor, "aβ".len());

        let mut library = App {
            page: Page::Library,
            content_focused: true,
            mode: AppMode::WorkspaceSearch,
            workspace: AppWorkspaceState {
                workspace_query: "paper".into(),
                workspace_query_cursor: 2,
                ..AppWorkspaceState::default()
            },
            ..App::default()
        };
        assert!(paste_text_into_active_input(&mut library, " new"));
        assert_eq!(library.workspace_query, "pa newper");
        assert_eq!(library.workspace_query_cursor, 6);
    }

    #[test]
    fn year_and_limit_inputs_are_parsed() {
        // R3: year / limit 入力欄の解釈
        assert_eq!(super::parse_year_range("2010-2020"), Some((2010, 2020)));
        assert_eq!(super::parse_year_range("2019"), Some((2019, 2019)));
        assert_eq!(super::parse_year_range("2020-2010"), Some((2010, 2020)));
        assert_eq!(super::parse_year_range(""), None);
        assert_eq!(super::parse_year_range("abc"), None);
        assert_eq!(super::parse_discovery_limit(""), 100);
        assert_eq!(super::parse_discovery_limit("250"), 250);
    }

    #[test]
    fn discovery_db_input_parsing() {
        // DB 欄の入力検証：空欄、単一 DB、複数 DB、無効な DB 名
        let config = super::Config::default();

        // 空欄：設定を使用
        let (providers, error) = super::get_discovery_providers_from_input("", &config);
        assert!(providers.len() > 0, "空欄時は config から providers を取得");
        assert!(error.is_none());

        // 単一 DB
        let (providers, error) = super::get_discovery_providers_from_input("pubmed", &config);
        assert_eq!(providers, vec!["pubmed"]);
        assert!(error.is_none());

        // 複数 DB（カンマ区切り）
        let (providers, error) =
            super::get_discovery_providers_from_input("pubmed,openalex", &config);
        assert_eq!(providers, vec!["pubmed", "openalex"]);
        assert!(error.is_none());

        // 大文字を自動小文字化
        let (providers, error) =
            super::get_discovery_providers_from_input("PubMed,OpenAlex", &config);
        assert_eq!(providers, vec!["pubmed", "openalex"]);
        assert!(error.is_none());

        // 無効な DB 名は警告
        let (providers, error) = super::get_discovery_providers_from_input("unknown", &config);
        assert!(
            providers.len() > 0,
            "無効名がある場合は config から providers を取得"
        );
        assert!(error.is_some());

        // 複数指定時に一部無効な場合は警告のみ
        let (providers, error) =
            super::get_discovery_providers_from_input("pubmed,invalid", &config);
        assert_eq!(providers, vec!["pubmed"]);
        assert!(error.is_some());
    }

    #[test]
    fn paste_shortcut_accepts_terminals_that_drop_the_shift_modifier() {
        assert!(is_text_paste_shortcut(KeyEvent::new(
            KeyCode::Char('v'),
            KeyModifiers::CONTROL,
        )));
        assert!(is_text_paste_shortcut(KeyEvent::new(
            KeyCode::Char('V'),
            KeyModifiers::CONTROL | KeyModifiers::SHIFT,
        )));
    }

    #[test]
    fn arrow_and_vim_panel_navigation_preserve_the_same_selection() {
        for (back, open) in [
            (KeyCode::Char('h'), KeyCode::Char('l')),
            (KeyCode::Left, KeyCode::Right),
        ] {
            let mut app = App {
                page: Page::Library,
                // Import 追加により Library の位置は可変。ALL から解決する。
                sidebar_index: Page::ALL
                    .iter()
                    .position(|&p| p == Page::Library)
                    .unwrap_or(2),
                content_focused: true,
                library: LibraryState {
                    selected: 7,
                    ..LibraryState::default()
                },
                ..App::default()
            };
            let _ = handle_key(&mut app, KeyEvent::new(back, KeyModifiers::NONE));
            assert!(!app.content_focused);
            assert_eq!(
                app.sidebar_index,
                Page::ALL
                    .iter()
                    .position(|&p| p == Page::Library)
                    .unwrap_or(2)
            );
            assert_eq!(app.library.selected, 7);

            let _ = handle_key(&mut app, KeyEvent::new(open, KeyModifiers::NONE));
            assert!(app.content_focused);
            assert_eq!(app.page, Page::Library);
            assert_eq!(
                app.sidebar_index,
                Page::ALL
                    .iter()
                    .position(|&p| p == Page::Library)
                    .unwrap_or(2)
            );
            assert_eq!(app.library.selected, 7);
        }
    }

    #[test]
    fn ctrl_t_opens_the_terminal_command_palette() {
        let mut app = App::default();

        let _ = handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('t'), KeyModifiers::CONTROL),
        );

        assert_eq!(app.mode, AppMode::TerminalCommand);
        assert!(app.terminal_command.is_empty());
        assert!(app.terminal_command_output.is_empty());
        assert_eq!(
            app.terminal_command_directory,
            super::terminal_home_directory()
        );
    }

    #[cfg(not(target_os = "windows"))]
    #[test]
    fn successful_terminal_commands_omit_exit_zero() {
        let mut app = App::default();
        app.terminal_command = "true".into();
        app.terminal_command_cursor = app.terminal_command.len();

        run_terminal_command(&mut app);

        assert!(app.terminal_command_output.contains("$ true"));
        assert!(!app.terminal_command_output.contains("[exit 0]"));
    }

    #[test]
    fn terminal_clear_and_control_sequences_stay_inside_the_palette() {
        let mut app = App {
            workspace: AppWorkspaceState {
                terminal_command_output: "previous output".into(),
                terminal_command: "clear".into(),
                ..AppWorkspaceState::default()
            },
            ..App::default()
        };

        run_terminal_command(&mut app);

        assert!(app.terminal_command_output.is_empty());
        assert!(app.terminal_command.is_empty());
        assert_eq!(sanitize_terminal_output("safe\u{1b}[2Jtext"), "safe[2Jtext");
    }

    #[test]
    fn terminal_tab_completes_paths_in_the_session_directory() -> anyhow::Result<()> {
        let root = std::env::temp_dir().join(format!("papr-complete-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root)?;
        std::fs::write(root.join("references.bib"), "")?;
        let mut app = App::default();
        app.terminal_command_directory = Some(root.clone());
        app.terminal_command = "cat ref".into();
        app.terminal_command_cursor = app.terminal_command.len();

        complete_terminal_command(&mut app, false);

        assert_eq!(app.terminal_command, "cat references.bib");
        std::fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn terminal_tab_lists_then_cycles_completion_candidates() -> anyhow::Result<()> {
        let root = std::env::temp_dir().join(format!("papr-cycle-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root)?;
        std::fs::write(root.join("alpha.tex"), "")?;
        std::fs::write(root.join("amber.tex"), "")?;
        let mut app = App::default();
        app.terminal_command_directory = Some(root.clone());
        app.terminal_command = "cat a".into();
        app.terminal_command_cursor = app.terminal_command.len();

        complete_terminal_command(&mut app, false);
        assert_eq!(app.terminal_command, "cat a");
        assert_eq!(app.terminal_completions, vec!["alpha.tex", "amber.tex"]);
        complete_terminal_command(&mut app, false);
        assert_eq!(app.terminal_command, "cat alpha.tex");
        complete_terminal_command(&mut app, false);
        assert_eq!(app.terminal_command, "cat amber.tex");
        complete_terminal_command(&mut app, true);
        assert_eq!(app.terminal_command, "cat alpha.tex");

        std::fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn terminal_completion_is_case_insensitive_and_recomputes_after_accepting() -> anyhow::Result<()>
    {
        let root = std::env::temp_dir().join(format!("papr-case-complete-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("Documents"))?;
        std::fs::create_dir_all(root.join("Downloads"))?;
        std::fs::write(root.join("Documents").join("Alpha.tex"), "")?;
        std::fs::write(root.join("Documents").join("Beta.tex"), "")?;
        let mut app = App::default();
        app.mode = AppMode::TerminalCommand;
        app.terminal_command_directory = Some(root.clone());
        app.terminal_command = "cp doc".into();
        app.terminal_command_cursor = app.terminal_command.len();

        complete_terminal_command(&mut app, false);
        assert_eq!(app.terminal_command, "cp Documents/");
        assert!(app.terminal_completions.is_empty());

        complete_terminal_command(&mut app, false);
        assert_eq!(app.terminal_command, "cp Documents/");
        assert_eq!(
            app.terminal_completions,
            vec!["Documents/Alpha.tex", "Documents/Beta.tex"]
        );
        complete_terminal_command(&mut app, false);
        let _ = handle_key(&mut app, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));

        assert_eq!(app.terminal_command, "cp Documents/Alpha.tex");
        assert!(app.terminal_completions.is_empty());

        std::fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn terminal_enter_accepts_a_completion_before_running() {
        let mut app = App::default();
        app.mode = AppMode::TerminalCommand;
        app.terminal_command = "cle".into();
        app.terminal_command_cursor = app.terminal_command.len();
        app.terminal_completions = vec!["clear".into()];
        app.terminal_completion_selected = Some(0);
        app.terminal_completion_token_start = 0;

        let _ = handle_key(&mut app, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));

        assert_eq!(app.terminal_command, "clear");
        assert!(app.terminal_command_output.is_empty());
        assert!(app.terminal_completions.is_empty());
    }

    #[test]
    fn help_opens_without_navigating_from_dashboard_or_discover() {
        let mut dashboard = App {
            page: Page::Dashboard,
            content_focused: false,
            sidebar_index: 0,
            ..App::default()
        };
        let _ = handle_key(
            &mut dashboard,
            KeyEvent::new(KeyCode::Char('?'), KeyModifiers::NONE),
        );
        assert_eq!(dashboard.mode, AppMode::Help);
        assert_eq!(dashboard.page, Page::Dashboard);
        assert!(!dashboard.content_focused);
        assert_eq!(dashboard.sidebar_index, 0);

        let mut discover = App {
            page: Page::Discover,
            content_focused: true,
            sidebar_index: 1,
            ..App::default()
        };
        discover.discovery.selected = 3;
        let _ = handle_key(
            &mut discover,
            KeyEvent::new(KeyCode::Char('?'), KeyModifiers::NONE),
        );
        assert_eq!(discover.mode, AppMode::Help);
        assert_eq!(discover.page, Page::Discover);
        assert_eq!(discover.discovery.selected, 3);
        assert_eq!(discover.sidebar_index, 1);
    }

    #[test]
    fn help_preserves_dashboard_and_discover_paper_detail_views() {
        for page in [Page::Dashboard, Page::Discover] {
            let mut app = App {
                page,
                content_focused: true,
                mode: AppMode::PaperDetail,
                paper_detail_scroll: 9,
                ..App::default()
            };

            let _ = handle_key(
                &mut app,
                KeyEvent::new(KeyCode::Char('?'), KeyModifiers::NONE),
            );
            assert_eq!(app.mode, AppMode::Help);
            assert_eq!(app.page, page);
            assert_eq!(app.paper_detail_scroll, 9);

            let _ = handle_key(&mut app, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
            assert_eq!(app.mode, AppMode::PaperDetail);
            assert_eq!(app.page, page);
            assert_eq!(app.paper_detail_scroll, 9);
        }
    }

    #[test]
    fn paper_detail_handler_never_routes_help_to_back_navigation() {
        let mut app = App {
            page: Page::Discover,
            content_focused: true,
            mode: AppMode::PaperDetail,
            paper_detail_scroll: 4,
            discovery: DiscoveryState {
                selected: 2,
                ..DiscoveryState::default()
            },
            ..App::default()
        };

        let action = handle_paper_detail_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('?'), KeyModifiers::SHIFT),
        );
        assert!(action.is_none());
        assert_eq!(app.mode, AppMode::Help);
        assert_eq!(app.help_return_mode, AppMode::PaperDetail);
        assert_eq!(app.paper_detail_scroll, 4);
        assert_eq!(app.discovery.selected, 2);
    }

    #[test]
    fn report_header_line_shows_issues_or_checked_date() {
        use papr_core::IssueCounts;

        // 問題あり: 件数と確認方法・詳しい場所を出す
        let line = report_header_line(
            IssueCounts {
                errors: 2,
                warnings: 1,
            },
            None,
            "R-0007",
        );
        assert!(line.starts_with("> 出典:"), "{line}");
        assert!(line.contains("要確認 2 件・注意 1 件"), "{line}");
        assert!(line.contains("C で外部の引用を確認"), "{line}");
        assert!(line.contains("sai-paper report issues R-0007"), "{line}");

        // 問題なし + 最終確認の日付
        let line = report_header_line(
            IssueCounts::default(),
            Some("2026-09-24T21:10:00+00:00"),
            "R-0007",
        );
        assert!(line.contains("出典: 問題なし"), "{line}");
        assert!(line.contains("最終確認 09/24"), "{line}");

        // 問題なし + 未確認
        let line = report_header_line(IssueCounts::default(), None, "R-0007");
        assert!(line.contains("最終確認 未確認"), "{line}");
    }

    #[test]
    fn paper_detail_digits_open_the_reports_and_do_not_collide() {
        let mut app = App {
            page: Page::Library,
            content_focused: true,
            mode: AppMode::PaperDetail,
            paper_detail_local: Some(7),
            ..App::default()
        };
        for (index, id) in ["R-0003", "R-0007", "R-0005"].iter().enumerate() {
            app.paper_reports.push((
                papr_core::Report {
                    id: (*id).to_string(),
                    ..papr_core::Report::default()
                },
                papr_core::IssueCounts::default(),
            ));
            let digit = char::from(b'1' + index as u8);
            let action = handle_paper_detail_key(
                &mut app,
                KeyEvent::new(KeyCode::Char(digit), KeyModifiers::NONE),
            );
            match action {
                Some(UiAction::OpenReport(open)) => assert_eq!(open, *id),
                other => panic!("unexpected action: {:?}", other.is_some()),
            }
        }
        // 範囲外の数字と `0` は何もしない
        app.paper_reports.clear();
        for digit in ['1', '5', '0', '9'] {
            let action = handle_paper_detail_key(
                &mut app,
                KeyEvent::new(KeyCode::Char(digit), KeyModifiers::NONE),
            );
            assert!(action.is_none(), "{digit} が何かを返した");
        }
    }

    #[test]
    fn md_view_c_checks_only_when_a_report_is_open() {
        // report 以外（note.md・extract.md）を表示しているときは何もしない
        let mut app = App {
            md_view_report: None,
            ..App::default()
        };
        assert!(
            super::handle_md_view_key(
                &mut app,
                KeyEvent::new(KeyCode::Char('C'), KeyModifiers::SHIFT),
            )
            .is_none()
        );

        // report を表示しているときだけ照合のアクションを返す
        app.md_view_report = Some("R-0007".to_string());
        match super::handle_md_view_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('C'), KeyModifiers::SHIFT),
        ) {
            Some(UiAction::CheckReport(id)) => assert_eq!(id, "R-0007"),
            other => panic!("unexpected action: {:?}", other.is_some()),
        }
    }

    /// 引用の locator の分け方（`§` / `p.` / `Fig.` / なし）。
    #[test]
    fn citation_locator_destinations() {
        assert_eq!(super::parse_citation_locator(None), super::CitationLocator::Paper);
        assert_eq!(
            super::parse_citation_locator(Some("§3.2")),
            super::CitationLocator::Heading("3.2".into())
        );
        assert_eq!(
            super::parse_citation_locator(Some("§ 3.2.")),
            super::CitationLocator::Heading("3.2".into())
        );
        assert_eq!(
            super::parse_citation_locator(Some("p.5")),
            super::CitationLocator::Page(5)
        );
        assert_eq!(
            super::parse_citation_locator(Some("pp. 12")),
            super::CitationLocator::Page(12)
        );
        assert_eq!(
            super::parse_citation_locator(Some("Fig.3")),
            super::CitationLocator::Figure(3)
        );
        // 形が分からない locator は PAPER 画面。
        assert_eq!(
            super::parse_citation_locator(Some("よく分からない")),
            super::CitationLocator::Paper
        );
    }

    /// md の 4 通りの引用の行から、開く操作と飛び先が決まる（AC3 の代わり）。
    ///
    /// tmux が使えず TUI を対話で操作できないため、本物のビューアの代わりに
    /// カーソルを引用の行へ送り、`citation_open_at_cursor` の結果を確かめる。
    #[test]
    fn citation_lines_decide_destinations() {
        let md = "intro\n\n[#37 §3.2]\n\n[#37 p.5]\n\n[#37 Fig.3]\n\n[#37]\n";
        let theme = crate::theme::Theme::load("catppuccin-mocha").unwrap();
        let mut view =
            crate::md_view::MarkdownView::from_markdown(md, &theme, "t".into(), "s".into());
        let mut seen = Vec::new();
        while view.cursor_move_to_citation(1) {
            match super::citation_open_at_cursor(&view) {
                Some(Ok(UiAction::OpenCitation { paper_id, locator })) => {
                    assert_eq!(paper_id, 37);
                    seen.push(super::parse_citation_locator(locator.as_deref()));
                }
                other => panic!("Library の論文の引用のはず: {}", other.is_some()),
            }
        }
        assert_eq!(
            seen,
            vec![
                super::CitationLocator::Heading("3.2".into()),
                super::CitationLocator::Page(5),
                super::CitationLocator::Figure(3),
                super::CitationLocator::Paper,
            ]
        );
    }

    /// 別の md を開いても `Esc` で元の md に戻る（計画の項目 4）。
    #[test]
    fn md_view_restores_previous_view_on_esc() {
        let theme = crate::theme::Theme::load("catppuccin-mocha").unwrap();
        let mut app = App {
            md_view: Some(crate::md_view::MarkdownView::from_markdown(
                "report body",
                &theme,
                "report R-1".into(),
                "r.md".into(),
            )),
            md_view_report: Some("R-1".into()),
            md_view_return: AppMode::PaperDetail,
            mode: AppMode::MarkdownView,
            ..App::default()
        };

        // 引用の `§` で extract.md を開くのと同じ手順でビューを差し替える。
        super::stash_md_view_before_replacing(&mut app);
        app.md_view = Some(crate::md_view::MarkdownView::from_markdown(
            "## 3.2 Methods",
            &theme,
            "extract.md".into(),
            "e.md".into(),
        ));
        app.md_view_report = None;
        app.md_view_return = app.mode;
        app.mode = AppMode::MarkdownView;

        // 1 回目の Esc で元の report に戻る。
        assert!(
            super::handle_md_view_key(&mut app, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
                .is_none()
        );
        assert_eq!(app.mode, AppMode::MarkdownView);
        assert_eq!(app.md_view_report.as_deref(), Some("R-1"));
        assert_eq!(app.md_view_return, AppMode::PaperDetail);
        assert!(app.md_view.as_ref().unwrap().title.contains("report"));

        // 2 回目の Esc でビューアを閉じる。
        assert!(
            super::handle_md_view_key(&mut app, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
                .is_none()
        );
        assert!(app.md_view.is_none());
        assert_eq!(app.mode, AppMode::PaperDetail);
    }

    /// 外部の引用は開かずトーストになる。
    #[test]
    fn external_citation_becomes_toast() {
        let md = "see [DOI:10.1000/xyz]";
        let theme = crate::theme::Theme::load("catppuccin-mocha").unwrap();
        let view =
            crate::md_view::MarkdownView::from_markdown(md, &theme, "t".into(), "s".into());
        match super::citation_open_at_cursor(&view) {
            Some(Err(message)) => assert!(message.contains("Library にない"), "{message}"),
            _ => panic!("Library に無い引用はトーストのはず"),
        }
    }

    #[test]
    fn discover_filter_refines_results_without_starting_a_search() {
        let mut title_match = remote_paper("title", "Genome assembly");
        title_match.authors = vec!["Ada Author".into()];
        let mut author_match = remote_paper("author", "Other paper");
        author_match.authors = vec!["Genome Researcher".into()];
        let mut abstract_match = remote_paper("abstract", "Third genome paper");
        abstract_match.abstract_text = "A genome-scale analysis.".into();
        let mut app = App {
            page: Page::Discover,
            content_focused: true,
            ..App::default()
        };
        app.discovery
            .set_results(vec![author_match, abstract_match, title_match]);

        let action = handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('>'), KeyModifiers::NONE),
        );
        assert!(action.is_none());
        assert_eq!(app.mode, AppMode::DiscoverFilter);
        for character in "genome".chars() {
            assert!(
                handle_key(
                    &mut app,
                    KeyEvent::new(KeyCode::Char(character), KeyModifiers::NONE)
                )
                .is_none()
            );
        }
        assert_eq!(app.discovery.filtered_result_count(), 3);
        assert_eq!(
            app.discovery
                .selected_paper()
                .map(|paper| paper.id.as_str()),
            Some("author")
        );

        let _ = handle_key(&mut app, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(app.mode, AppMode::Normal);
        assert_eq!(app.discovery.filter, "genome");
        let _ = handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('>'), KeyModifiers::NONE),
        );
        assert_eq!(app.mode, AppMode::DiscoverFilter);
        for _ in 0.."genome".len() {
            let _ = handle_key(
                &mut app,
                KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE),
            );
        }
        assert!(app.discovery.filter.is_empty());
        assert_eq!(app.discovery.filtered_result_count(), 3);
    }

    #[test]
    fn library_enter_opens_selected_paper_even_with_dashboard_activity() {
        // Library で Enter/Right/l を押した際に Dashboard の RA 行（recent_activity）
        // へ横取りされず、カーソル位置の論文が開くこと。
        // 再現: recent_activity が非空だと従来は先頭行の paper_id が優先された。
        fn library_app() -> App {
            let mut app = App {
                page: Page::Library,
                content_focused: true,
                mode: AppMode::Normal,
                library: LibraryState {
                    papers: vec![
                        LibraryPaper {
                            id: 1,
                            title: "Head paper".into(),
                            authors: String::new(),
                            doi: None,
                            arxiv_id: None,
                            pdf_path: Some("/tmp/head.pdf".into()),
                            file_size: None,
                            reading_status: "unread".into(),
                            is_favorite: false,
                            abstract_text: None,
                            journal: None,
                            published_at: None,
                            abstract_ja: None,
                            abstract_ja_translated_at: None,
                            citation_count: None,
                            is_retracted: None,
                            retraction_checked_at: None,
                            updated_at: None,
                        },
                        LibraryPaper {
                            id: 2,
                            title: "Selected paper".into(),
                            authors: String::new(),
                            doi: None,
                            arxiv_id: None,
                            pdf_path: Some("/tmp/selected.pdf".into()),
                            file_size: None,
                            reading_status: "unread".into(),
                            is_favorite: false,
                            abstract_text: None,
                            journal: None,
                            published_at: None,
                            abstract_ja: None,
                            abstract_ja_translated_at: None,
                            citation_count: None,
                            is_retracted: None,
                            retraction_checked_at: None,
                            updated_at: None,
                        },
                    ],
                    selected: 1,
                    ..LibraryState::default()
                },
                ..App::default()
            };
            app.dashboard.recent_activity = vec![papr_core::ActivityItem {
                kind: "paper_browsed".into(),
                paper_id: Some(1),
                label: "Head paper".into(),
                occurred_at: Utc::now(),
                radio_id: None,
            }];
            app.activity_selected = 0;
            app
        }
        for key in [KeyCode::Enter, KeyCode::Right, KeyCode::Char('l')] {
            let mut app = library_app();
            let action = handle_key(&mut app, KeyEvent::new(key, KeyModifiers::NONE));
            match action {
                Some(UiAction::OpenLibraryPaper(id)) => {
                    assert_eq!(id, 2, "Library {key:?} must open the selected paper");
                }
                other => panic!("unexpected action for {key:?}: {:?}", other.is_some()),
            }
        }
    }

    #[test]
    fn library_paper_detail_renders_the_stored_opened_paper() {
        // OpenLibraryPaper で保持した ID の論文が、一覧の選択位置が先頭に
        // 変わっても PAPER 詳細に正しく表示されること。
        let library_paper = |id: i64, title: &str| LibraryPaper {
            id,
            title: title.into(),
            authors: String::new(),
            doi: None,
            arxiv_id: None,
            pdf_path: Some(format!("/tmp/{id}.pdf")),
            file_size: None,
            reading_status: "unread".into(),
            is_favorite: false,
            abstract_text: None,
            journal: None,
            published_at: None,
            abstract_ja: None,
            abstract_ja_translated_at: None,
            citation_count: None,
            is_retracted: None,
            retraction_checked_at: None,
            updated_at: None,
        };
        let mut app = App {
            page: Page::Library,
            content_focused: true,
            mode: AppMode::PaperDetail,
            library: LibraryState {
                papers: vec![
                    library_paper(1, "Head paper"),
                    library_paper(2, "Opened paper"),
                ],
                selected: 0,
                ..LibraryState::default()
            },
            paper_detail_local: Some(2),
            ..App::default()
        };
        let theme = crate::theme::Theme::load("catppuccin-mocha").unwrap();
        let backend = ratatui::backend::TestBackend::new(120, 40);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| crate::ui::render(frame, &mut app, &theme))
            .unwrap();
        let text = terminal.backend().to_string().replace('\n', " ");
        assert!(
            text.contains("Opened paper"),
            "detail must show the opened paper"
        );
        assert!(
            !text.contains("Head paper"),
            "detail must not show the list head"
        );

        // 詳細画面の操作は表示中の論文に作用する（選択位置がずれていても）。
        let action = handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE),
        );
        assert!(
            matches!(action, Some(UiAction::CopyCitation(PaperTarget::Local(2)))),
            "detail action must target the opened paper"
        );

        // Back で戻ると保持をクリアする。
        app.dispatch(Command::Back);
        assert!(app.paper_detail_local.is_none());
    }

    #[test]
    fn library_paper_detail_renders_reports_section() {
        // PAPER 画面の RESOURCES の後ろに REPORTS が出る（AC1〜AC5 の手動確認の代わり）。
        let mut app = App {
            page: Page::Library,
            content_focused: true,
            mode: AppMode::PaperDetail,
            library: LibraryState {
                papers: vec![LibraryPaper {
                    id: 2,
                    title: "Opened paper".into(),
                    authors: String::new(),
                    doi: None,
                    arxiv_id: None,
                    pdf_path: Some("/tmp/2.pdf".into()),
                    file_size: None,
                    reading_status: "unread".into(),
                    is_favorite: false,
                    abstract_text: None,
                    journal: None,
                    published_at: None,
                    abstract_ja: None,
                    abstract_ja_translated_at: None,
                    citation_count: None,
                    is_retracted: None,
                    retraction_checked_at: None,
                    updated_at: None,
                }],
                selected: 0,
                ..LibraryState::default()
            },
            paper_detail_local: Some(2),
            paper_report_total: 2,
            ..App::default()
        };
        app.paper_reports.push((
            papr_core::Report {
                id: "R-0007".into(),
                block: Some("paper-critic".into()),
                created_at: "2026-09-24T21:10:00+00:00".into(),
                ..papr_core::Report::default()
            },
            papr_core::IssueCounts {
                errors: 1,
                warnings: 2,
            },
        ));
        let theme = crate::theme::Theme::load("catppuccin-mocha").unwrap();
        let backend = ratatui::backend::TestBackend::new(120, 120);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| crate::ui::render(frame, &mut app, &theme))
            .unwrap();
        let text = terminal.backend().to_string();
        let compact: String = text.chars().filter(|c| !c.is_whitespace()).collect();
        assert!(compact.contains("REPORTS"), "REPORTS 見出しが無い: {compact}");
        assert!(compact.contains("R-0007"), "report の行が無い: {compact}");
        assert!(
            compact.contains("要確認1・注意2"),
            "件数の表示が無い: {compact}"
        );
    }

    #[test]
    fn manual_import_shortcuts_from_library_list_and_paper_detail() {
        let library_paper = || LibraryPaper {
            id: 1,
            title: "Manual Import Paper".into(),
            authors: String::new(),
            doi: None,
            arxiv_id: None,
            pdf_path: None,
            file_size: None,
            reading_status: "unread".into(),
            is_favorite: false,
            abstract_text: None,
            journal: None,
            published_at: None,
            abstract_ja: None,
            abstract_ja_translated_at: None,
            citation_count: None,
            is_retracted: None,
            retraction_checked_at: None,
            updated_at: None,
        };

        // ライブラリ一覧から `i` で手動取り込みを開始できる。
        let mut list_app = App {
            page: Page::Library,
            content_focused: true,
            ..App::default()
        };
        list_app.library.papers = vec![library_paper()];
        let from_list = handle_key(
            &mut list_app,
            KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE),
        );
        assert!(matches!(
            from_list,
            Some(UiAction::OpenManualImport(ManualImportPaper::Local { .. }))
        ));

        // PAPER 詳細（ローカル論文）からも `i` で開始できる。
        let mut detail_app = App {
            page: Page::Library,
            mode: AppMode::PaperDetail,
            ..App::default()
        };
        detail_app.library.papers = vec![library_paper()];
        let from_detail = handle_key(
            &mut detail_app,
            KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE),
        );
        assert!(matches!(
            from_detail,
            Some(UiAction::OpenManualImport(ManualImportPaper::Local { .. }))
        ));
    }

    #[test]
    fn manual_import_modal_supports_cursor_and_cancel_keys() {
        let mut app = App {
            mode: AppMode::ManualImport,
            modal_return: AppMode::PaperDetail,
            ..App::default()
        };
        app.manual_import = ManualImportState {
            paper: Some(ManualImportPaper::Local {
                paper_id: 1,
                title: "Test".into(),
            }),
            candidates: vec![PathBuf::from("/tmp/a.pdf"), PathBuf::from("/tmp/b.pdf")],
            selected: 0,
        };

        handle_manual_import_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE),
        );
        handle_manual_import_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE),
        );
        assert_eq!(app.manual_import.selected, 1);
        handle_manual_import_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('k'), KeyModifiers::NONE),
        );
        assert_eq!(app.manual_import.selected, 0);

        let confirmed =
            handle_manual_import_key(&mut app, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(matches!(confirmed, Some(UiAction::ConfirmManualImport)));

        handle_manual_import_key(&mut app, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(app.mode, AppMode::PaperDetail);
    }

    #[test]
    fn browser_shortcut_uses_selected_dashboard_and_search_urls() {
        let dashboard_paper = remote_paper("https://arxiv.org/abs/dashboard", "Dashboard");
        let search_paper = remote_paper("https://arxiv.org/abs/search", "Search");
        let mut app = App {
            page: Page::Dashboard,
            content_focused: true,
            today_papers: vec![dashboard_paper],
            ..App::default()
        };
        let dashboard = handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('o'), KeyModifiers::NONE),
        );
        assert!(matches!(
            dashboard,
            Some(UiAction::OpenBrowser(url)) if url.ends_with("/dashboard")
        ));

        app.page = Page::Discover;
        app.discovery.results = vec![search_paper];
        let search = handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('o'), KeyModifiers::NONE),
        );
        assert!(matches!(
            search,
            Some(UiAction::OpenBrowser(url)) if url.ends_with("/search")
        ));
    }

    #[test]
    fn dashboard_and_discover_paper_rows_support_citation_and_download_shortcuts() {
        for page in [Page::Dashboard, Page::Discover] {
            let paper = remote_paper("https://arxiv.org/abs/2607.12345", "Paper");
            let mut app = App {
                page,
                content_focused: true,
                today_papers: (page == Page::Dashboard)
                    .then_some(paper.clone())
                    .into_iter()
                    .collect(),
                discovery: DiscoveryState {
                    results: (page == Page::Discover)
                        .then_some(paper)
                        .into_iter()
                        .collect(),
                    ..DiscoveryState::default()
                },
                ..App::default()
            };
            assert!(matches!(
                handle_key(
                    &mut app,
                    KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE)
                ),
                Some(UiAction::CopyCitation(PaperTarget::Remote(_)))
            ));
            assert!(matches!(
                handle_key(
                    &mut app,
                    KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE)
                ),
                Some(UiAction::Download(_))
            ));
        }
    }

    #[test]
    fn browser_shortcut_opens_local_papers_and_reports_missing_arxiv_metadata() {
        let paper = LibraryPaper {
            id: 42,
            title: "Local paper".into(),
            authors: String::new(),
            doi: None,
            arxiv_id: Some("2607.12345v2".into()),
            pdf_path: Some("/tmp/local.pdf".into()),
            file_size: None,
            reading_status: "unread".into(),
            is_favorite: false,
            abstract_text: None,
            journal: None,
            published_at: None,
            abstract_ja: None,
            abstract_ja_translated_at: None,
            citation_count: None,
            is_retracted: None,
            retraction_checked_at: None,
            updated_at: None,
        };
        let mut app = App {
            page: Page::Library,
            content_focused: true,
            library: LibraryState {
                papers: vec![paper.clone()],
                ..LibraryState::default()
            },
            ..App::default()
        };
        assert!(matches!(
            handle_key(&mut app, KeyEvent::new(KeyCode::Char('o'), KeyModifiers::NONE)),
            Some(UiAction::OpenBrowser(url)) if url == "https://arxiv.org/abs/2607.12345v2"
        ));

        app.library.papers[0].arxiv_id = None;
        assert!(
            handle_key(
                &mut app,
                KeyEvent::new(KeyCode::Char('o'), KeyModifiers::NONE)
            )
            .is_none()
        );
        assert_eq!(
            app.toast.as_deref(),
            Some("No valid arXiv page is available for this paper")
        );
    }

    #[test]
    fn downloaded_remote_paper_opens_from_metadata_without_changing_browser_shortcut()
    -> anyhow::Result<()> {
        let pdf_path = std::env::temp_dir().join(format!(
            "papr-downloaded-remote-paper-{}-{}.pdf",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_err(|error| anyhow::anyhow!(error))?
                .as_nanos()
        ));
        std::fs::write(&pdf_path, b"%PDF-1.4 test")?;

        for page in [Page::Dashboard, Page::Discover] {
            let remote = remote_paper("https://arxiv.org/abs/downloaded", "Downloaded paper");
            let mut app = App {
                page,
                content_focused: true,
                mode: AppMode::PaperDetail,
                today_papers: (page == Page::Dashboard)
                    .then_some(remote.clone())
                    .into_iter()
                    .collect(),
                discovery: DiscoveryState {
                    results: (page == Page::Discover)
                        .then_some(remote)
                        .into_iter()
                        .collect(),
                    ..DiscoveryState::default()
                },
                library: LibraryState {
                    papers: vec![LibraryPaper {
                        id: 42,
                        title: "Downloaded paper".into(),
                        authors: String::new(),
                        doi: None,
                        arxiv_id: Some("https://arxiv.org/abs/downloaded".into()),
                        pdf_path: Some(pdf_path.to_string_lossy().into_owned()),
                        file_size: None,
                        reading_status: "unread".into(),
                        is_favorite: false,
                        abstract_text: None,
                        journal: None,
                        published_at: None,
                        abstract_ja: None,
                        abstract_ja_translated_at: None,
                        citation_count: None,
                        is_retracted: None,
                        retraction_checked_at: None,
                        updated_at: None,
                    }],
                    ..LibraryState::default()
                },
                ..App::default()
            };

            let open = handle_key(&mut app, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
            assert!(matches!(
                open,
                Some(UiAction::OpenPdf {
                    paper_id: 42,
                    path,
                    page: None,
                })
                    if path == pdf_path
            ));

            let browser = handle_key(
                &mut app,
                KeyEvent::new(KeyCode::Char('o'), KeyModifiers::NONE),
            );
            assert!(matches!(
                browser,
                Some(UiAction::OpenBrowser(url)) if url.ends_with("/downloaded")
            ));
        }
        std::fs::remove_file(pdf_path)?;
        Ok(())
    }

    #[test]
    fn dashboard_open_keeps_discover_results_independent() {
        // Dashboard no longer opens PAPER; Command::Open should be no-op for Dashboard
        let dashboard_paper = remote_paper("https://arxiv.org/abs/dashboard", "Dashboard Paper");
        let discover_paper = remote_paper("https://arxiv.org/abs/discover", "Discover Paper");
        let mut app = App {
            page: Page::Dashboard,
            content_focused: true,
            today_papers: vec![dashboard_paper],
            discovery: DiscoveryState {
                query: "search".into(),
                query_cursor: 6,
                results: vec![discover_paper],
                selected: 0,
                scroll: 3,
                status: DiscoveryStatus::Ready,
                detail_scroll: 11,
                ..DiscoveryState::default()
            },
            ..App::default()
        };

        app.dispatch(Command::Open);

        assert_eq!(app.mode, AppMode::Normal);
        assert_eq!(app.discovery.query, "search");
        assert_eq!(app.discovery.query_cursor, 6);
        assert_eq!(app.discovery.scroll, 3);
        assert_eq!(app.discovery.selected, 0);
        assert_eq!(app.discovery.detail_scroll, 11);
        assert_eq!(app.discovery.results.len(), 1);
        assert_eq!(app.discovery.results[0].title, "Discover Paper");
    }

    #[test]
    fn library_enter_opens_selected_pdf() {
        // Library Enter now opens PAPER, not PDF; PDF is opened via p in PAPER view
        let mut app = App {
            page: Page::Library,
            content_focused: true,
            library: LibraryState {
                papers: vec![LibraryPaper {
                    id: 7,
                    title: "Paper".into(),
                    authors: String::new(),
                    doi: None,
                    arxiv_id: None,
                    pdf_path: Some("/tmp/paper.pdf".into()),
                    file_size: None,
                    reading_status: "unread".into(),
                    is_favorite: false,
                    abstract_text: None,
                    journal: None,
                    published_at: None,
                    abstract_ja: None,
                    abstract_ja_translated_at: None,
                    citation_count: None,
                    is_retracted: None,
                    retraction_checked_at: None,
                    updated_at: None,
                }],
                ..LibraryState::default()
            },
            ..App::default()
        };
        let action = handle_key(&mut app, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(matches!(action, Some(UiAction::OpenLibraryPaper(7))));
    }

    #[test]
    fn parses_pdf_viewer_command_with_arguments() -> Result<(), Box<dyn std::error::Error>> {
        assert_eq!(parse_command("xdg-open")?, vec!["xdg-open"]);
        assert_eq!(
            parse_command("'tdf viewer' --flag {path}")?,
            vec!["tdf viewer", "--flag", "{path}"]
        );
        assert_eq!(
            parse_command(r#""C:\\Program Files\\Viewer.exe" --flag"#)?,
            vec![r"C:\Program Files\Viewer.exe", "--flag"]
        );
        Ok(())
    }

    #[test]
    fn external_pdf_viewers_receive_the_pdf_as_one_native_argument()
    -> Result<(), Box<dyn std::error::Error>> {
        let path = std::path::PathBuf::from("/tmp/Papr Papers/first build.pdf");
        let (program, args) = pdf_viewer_invocation("xdg-open", &path)?;
        assert_eq!(program, "xdg-open");
        assert_eq!(args, vec![path.as_os_str().to_owned()]);

        let (program, args) = pdf_viewer_invocation("viewer --open={path}", &path)?;
        assert_eq!(program, "viewer");
        assert_eq!(
            args,
            vec![OsString::from(format!("--open={}", path.display()))]
        );
        Ok(())
    }

    fn remote_paper(id: &str, title: &str) -> RemotePaper {
        let timestamp = Utc
            .with_ymd_and_hms(2026, 1, 1, 0, 0, 0)
            .single()
            .unwrap_or_else(Utc::now);
        RemotePaper {
            id: id.into(),
            title: title.into(),
            authors: vec!["Researcher".into()],
            abstract_text: String::new(),
            published: timestamp,
            updated: timestamp,
            categories: vec!["cs.DL".into()],
            pdf_url: None,
            doi: None,
            journal_ref: None,
            citation_count: None,
        }
    }

    #[test]
    fn rag_search_f_key_opens_search_on_library() {
        let mut app = App {
            page: Page::Library,
            content_focused: true,
            mode: AppMode::Normal,
            ..App::default()
        };

        // Pressing 'f' on Library opens RAG search mode
        let action = handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('f'), KeyModifiers::NONE),
        );
        assert!(action.is_none());
        assert_eq!(app.mode, AppMode::RagSearch);
        assert_eq!(app.rag_search.scope, "papers");
        assert_eq!(app.rag_search.query, "");
    }

    #[test]
    fn rag_search_f_key_opens_search_on_notes() {
        let mut app = App {
            page: Page::Notes,
            content_focused: true,
            mode: AppMode::Normal,
            ..App::default()
        };

        // Pressing 'f' on Notes opens RAG search mode with notes scope
        let action = handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('f'), KeyModifiers::NONE),
        );
        assert!(action.is_none());
        assert_eq!(app.mode, AppMode::RagSearch);
        assert_eq!(app.rag_search.scope, "notes");
    }

    #[test]
    fn rag_search_esc_exits_to_normal_mode() {
        let mut app = App {
            page: Page::Library,
            mode: AppMode::RagSearch,
            rag_search: RagSearchState {
                query: "test".to_string(),
                ..RagSearchState::default()
            },
            ..App::default()
        };

        // Pressing Esc exits RAG search mode
        let action = handle_key(&mut app, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(action.is_none());
        assert_eq!(app.mode, AppMode::Normal);
        assert_eq!(app.rag_search.query, "");
    }

    #[test]
    fn rag_search_text_input_updates_query() {
        let mut app = App {
            page: Page::Library,
            mode: AppMode::RagSearch,
            rag_search: RagSearchState {
                query: "test".to_string(),
                query_cursor: 4,
                ..RagSearchState::default()
            },
            ..App::default()
        };

        // Pressing 'x' adds character to query
        let action = handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE),
        );
        assert!(action.is_none());
        assert_eq!(app.rag_search.query, "testx");
        assert_eq!(app.rag_search.query_cursor, 5);
    }

    /// Ctrl+e で「要約付き」を切り替える。文字入力欄は変わらない（AC4）。
    #[test]
    fn rag_search_ctrl_e_toggles_evidence_without_typing() {
        let mut app = App {
            page: Page::Library,
            mode: AppMode::RagSearch,
            rag_search: RagSearchState {
                query: "test".to_string(),
                query_cursor: 4,
                ..RagSearchState::default()
            },
            ..App::default()
        };
        assert!(!app.rag_search.evidence);
        let action = handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('e'), KeyModifiers::CONTROL),
        );
        assert!(action.is_none());
        assert!(app.rag_search.evidence);
        // 切り替えキーは入力欄に文字を入れない。
        assert_eq!(app.rag_search.query, "test");
        assert_eq!(app.rag_search.query_cursor, 4);
        // もう一度押すと戻る。
        let _ = handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('e'), KeyModifiers::CONTROL),
        );
        assert!(!app.rag_search.evidence);
    }

    #[test]
    fn rag_search_down_moves_selection() {
        use papr_core::retrieval::Hit;

        let mut app = App {
            page: Page::Library,
            mode: AppMode::RagSearch,
            rag_search: RagSearchState {
                query: "test".to_string(),
                results: vec![
                    Hit {
                        score: 0.9,
                        chunk_id: 1,
                        source_kind: "body".into(),
                        source_ref: "paper1".into(),
                        paper_id: Some(1),
                        title: Some("Paper 1".into()),
                        authors: None,
                        year: None,
                        text: "Test content 1".into(),
                        char_start: None,
                        char_end: None,
                        md_path: None,
                        section: None,
                    },
                    Hit {
                        score: 0.8,
                        chunk_id: 2,
                        source_kind: "body".into(),
                        source_ref: "paper2".into(),
                        paper_id: Some(2),
                        title: Some("Paper 2".into()),
                        authors: None,
                        year: None,
                        text: "Test content 2".into(),
                        char_start: None,
                        char_end: None,
                        md_path: None,
                        section: None,
                    },
                ],
                selected: 0,
                ..RagSearchState::default()
            },
            ..App::default()
        };

        // Pressing Down moves selection to next result
        let action = handle_key(&mut app, KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        assert!(action.is_none());
        assert_eq!(app.rag_search.selected, 1);
    }

    #[test]
    fn rag_search_enter_opens_selected_paper() {
        use papr_core::retrieval::Hit;

        let mut app = App {
            page: Page::Library,
            mode: AppMode::RagSearch,
            rag_search: RagSearchState {
                query: "test".to_string(),
                results: vec![Hit {
                    score: 0.9,
                    chunk_id: 1,
                    source_kind: "body".into(),
                    source_ref: "paper1".into(),
                    paper_id: Some(99),
                    title: Some("Paper 1".into()),
                    authors: None,
                    year: None,
                    text: "Test content 1".into(),
                    char_start: None,
                    char_end: None,
                    md_path: None,
                    section: None,
                }],
                selected: 0,
                ..RagSearchState::default()
            },
            ..App::default()
        };

        // Pressing Enter opens the selected paper
        let action = handle_key(&mut app, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(matches!(action, Some(UiAction::OpenLibraryPaper(99))));
    }

    #[test]
    fn rag_search_enter_searches_when_no_results() {
        let mut app = App {
            page: Page::Library,
            mode: AppMode::RagSearch,
            rag_search: RagSearchState {
                query: "search query".to_string(),
                results: vec![],
                ..RagSearchState::default()
            },
            ..App::default()
        };

        // Pressing Enter with no results executes search
        let action = handle_key(&mut app, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(matches!(action, Some(UiAction::ExecuteRagSearch)));
    }

    #[test]
    fn rag_search_m_opens_markdown_when_available() {
        use papr_core::retrieval::Hit;

        let mut app = App {
            page: Page::Library,
            mode: AppMode::RagSearch,
            rag_search: RagSearchState {
                query: "test".to_string(),
                results: vec![Hit {
                    score: 0.9,
                    chunk_id: 1,
                    source_kind: "note".into(),
                    source_ref: "note1".into(),
                    paper_id: None,
                    title: Some("Note 1".into()),
                    authors: None,
                    year: None,
                    text: "Note content".into(),
                    char_start: None,
                    char_end: None,
                    md_path: Some("/path/to/note.md".into()),
                    section: None,
                }],
                selected: 0,
                ..RagSearchState::default()
            },
            ..App::default()
        };

        // Pressing 'm' opens the markdown file
        let action = handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('m'), KeyModifiers::NONE),
        );
        assert!(
            matches!(action, Some(UiAction::OpenRagSearchMarkdown(path)) if path == "/path/to/note.md")
        );
    }

    #[test]
    fn rag_search_m_opens_paper_for_figure_result() {
        use papr_core::retrieval::Hit;

        let mut app = App {
            page: Page::Library,
            mode: AppMode::RagSearch,
            rag_search: RagSearchState {
                query: "test".to_string(),
                results: vec![Hit {
                    score: 0.9,
                    chunk_id: 1,
                    source_kind: "figure".into(),
                    source_ref: "figure_key1_3".into(),
                    paper_id: Some(42),
                    title: Some("Paper A".into()),
                    authors: None,
                    year: None,
                    text: "Figure 3: Overview".into(),
                    char_start: None,
                    char_end: None,
                    md_path: None,
                    section: Some("Figure 3".into()),
                }],
                selected: 0,
                ..RagSearchState::default()
            },
            ..App::default()
        };

        // 図の結果では md_path が無くても、その論文の PAPER 画面を開く。
        let action = handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('m'), KeyModifiers::NONE),
        );
        assert!(matches!(action, Some(UiAction::OpenLibraryPaper(42))));
    }

    #[test]
    fn project_keys_open_mind_and_state_on_every_page() {
        for page in Page::ALL {
            let mut app = App {
                page,
                ..App::default()
            };
            let mind = handle_key(
                &mut app,
                KeyEvent::new(KeyCode::Char('M'), KeyModifiers::NONE),
            );
            assert!(
                matches!(mind, Some(UiAction::OpenProjectFile(ProjectFile::Mind))),
                "M must open mind.md on {page:?}"
            );
            let state = handle_key(
                &mut app,
                KeyEvent::new(KeyCode::Char('W'), KeyModifiers::NONE),
            );
            assert!(
                matches!(state, Some(UiAction::OpenProjectFile(ProjectFile::State))),
                "W must open state.md on {page:?}"
            );
        }
    }

    /// S02: Project ページの `A` で要対応の一覧を開く（他のページでは開かない）。
    #[test]
    fn attention_key_opens_list_only_on_project() {
        let mut project = App {
            page: Page::Project,
            ..App::default()
        };
        let action = handle_key(
            &mut project,
            KeyEvent::new(KeyCode::Char('A'), KeyModifiers::NONE),
        );
        assert!(action.is_none(), "A は画面を切り替えるだけ");
        assert_eq!(project.mode, AppMode::AttentionList);

        for page in Page::ALL.into_iter().filter(|page| *page != Page::Project) {
            let mut app = App {
                page,
                ..App::default()
            };
            let _ = handle_key(
                &mut app,
                KeyEvent::new(KeyCode::Char('A'), KeyModifiers::NONE),
            );
            assert_ne!(
                app.mode,
                AppMode::AttentionList,
                "A must not open attention on {page:?}"
            );
        }
    }

    /// S02: 一覧の j/k/Enter/x/Esc。
    #[test]
    fn attention_list_keys_navigate_open_resolve_and_close() {
        let item = |key: &str, kind| papr_core::attention::AttentionItem {
            key: key.to_string(),
            kind,
            title: format!("{key} の題"),
            run_id: Some(1),
            open_path: Some(std::path::PathBuf::from("/tmp/x.md")),
            report_id: None,
        };
        let mut app = App {
            page: Page::Project,
            mode: AppMode::AttentionList,
            project: ProjectView {
                attention: vec![
                    item("review:1", papr_core::attention::AttentionKind::EmptyReview),
                    item("card:2", papr_core::attention::AttentionKind::RecommendCard),
                ],
                ..ProjectView::default()
            },
            ..App::default()
        };
        // j で 2 行目へ。
        let _ = handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE),
        );
        assert_eq!(app.attention_selected, 1);
        // Enter で 2 行目を開く。
        let action = handle_key(&mut app, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(
            matches!(&action, Some(UiAction::OpenAttention(item)) if item.key == "card:2"),
            "Enter は 2 行目を開く"
        );
        // x で対応済み。
        let action = handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE),
        );
        assert!(
            matches!(&action, Some(UiAction::ResolveAttention(item)) if item.key == "card:2"),
            "x は 2 行目を対応済みにする"
        );
        // k で 1 行目、Esc で閉じる。
        let _ = handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('k'), KeyModifiers::NONE),
        );
        assert_eq!(app.attention_selected, 0);
        let _ = handle_key(&mut app, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(app.mode, AppMode::Normal);
    }

    /// S02: `x` の本体は `resolved` を残して一覧から消す。
    #[test]
    fn resolve_attention_at_records_and_removes() -> Result<(), Box<dyn std::error::Error>> {
        use std::fs;
        let root = std::env::temp_dir().join(format!(
            "sai-paper-tui-attention-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join(".sai/paper"))?;
        fs::create_dir_all(root.join(".sai/outputs/todo/9"))?;
        fs::write(
            root.join(".sai/outputs/todo/9/task.md"),
            "---\nrun_id: 9\nblock: paper-screen\ntag: 推薦\ndescription: 候補を絞る\norigin_run_id: -\n---\n\nタスク\n",
        )?;
        let db = papr_core::Database::in_memory()?;
        let roots = papr_core::PaperRoots {
            root: root.clone(),
            data_dir: root.join(".sai/paper"),
        };
        let mut app = App::default();

        let items = papr_core::attention::collect(db.connection(), &roots)?;
        assert_eq!(items.len(), 1, "{items:?}");
        resolve_attention_at(&db, &roots, &items[0], &mut app)?;
        assert!(app.project.attention.is_empty(), "一覧から消える");
        let resolved: i64 = db.connection().query_row(
            "SELECT COUNT(*) FROM activity_log WHERE kind = 'resolved'",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(resolved, 1);

        fs::remove_dir_all(&root).ok();
        Ok(())
    }

    /// S03: Search Sets 画面の `r` は選んだセットの再実行を頼む。
    #[test]
    fn search_sets_r_key_requests_rerun() {
        let set = papr_core::SearchSet {
            id: 3,
            name: "eeg".into(),
            source_dbs: "s2".into(),
            query: "eeg".into(),
            raw_query: None,
            created_by: "user".into(),
            note: None,
            created_at: "2026-09-25T09:00:00+09:00".into(),
        };
        let mut app = App {
            page: Page::SearchSets,
            content_focused: true,
            search_sets: vec![set],
            ..App::default()
        };
        let action = handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE),
        );
        assert!(matches!(action, Some(UiAction::RerunSearchSet(3))));
    }

    /// S03: 再実行の結果でトーストを出し、一覧と候補を読み直す。
    #[test]
    fn apply_search_set_rerun_result_updates_toast_and_list()
    -> Result<(), Box<dyn std::error::Error>> {
        let db = papr_core::Database::in_memory()?;
        let conn = db.connection();
        conn.execute(
            "INSERT INTO search_sets (id, name, source_dbs, query) VALUES (3, 'eeg', 's2', 'eeg')",
            [],
        )?;

        let mut app = App::default();
        apply_search_set_rerun_result_at(
            conn,
            3,
            Ok(papr_core::search_sets::RerunResult {
                set_id: 3,
                new_items: 5,
                total_hits: 20,
                failed_dbs: vec!["s2".to_string()],
            }),
            &mut app,
        );
        assert_eq!(
            app.toast.as_deref(),
            Some("新着 5 本（検索 20 件、失敗した DB: s2）")
        );
        assert_eq!(app.search_sets.len(), 1, "一覧を読み直す");
        assert_eq!(app.search_set_selected, 0);

        apply_search_set_rerun_result_at(conn, 3, Err("だめ".to_string()), &mut app);
        assert!(
            app.toast
                .as_deref()
                .is_some_and(|toast| toast.contains("もう一度検索できません")),
            "{:?}",
            app.toast
        );
        Ok(())
    }

    /// S03: 要対応の key から set_id を読む。
    #[test]
    fn attention_set_id_reads_key() {
        assert_eq!(
            attention_set_id("set:5:2026-09-25T09:00:00+09:00"),
            Some(5)
        );
        assert_eq!(attention_set_id("review:5"), None);
        assert_eq!(attention_set_id("set:x"), None);
    }

    /// U23: 要対応の「判断待ち」「仕分けていない提案」は研究方針のページで開く。
    ///
    /// 判断待ち（`sci_decision`）のときはキーの 2 番目の節点を選ぶ。
    #[test]
    fn sci_attention_opens_tree_page_and_selects_node() {
        use papr_core::attention::{AttentionItem, AttentionKind};

        let item = AttentionItem {
            key: "sci_decision:N12:31".into(),
            kind: AttentionKind::SciDecision,
            title: "判断待ち".into(),
            run_id: None,
            open_path: None,
            report_id: None,
        };
        let mut app = App::default();
        open_sci_attention_at(&item, &mut app);
        assert_eq!(app.page, Page::Tree);
        assert_eq!(app.mode, AppMode::Normal);
        assert!(app.content_focused);
        assert_eq!(
            app.sidebar_index,
            Page::ALL
                .iter()
                .position(|page| *page == Page::Tree)
                .expect("研究方針のページがある")
        );
        assert_eq!(app.tree.pending_select, Some(12));
        assert!(app.tree.stale);

        // 仕分けていない提案（`sci_triage`）は節点を持たないので選択は動かさない。
        let triage = AttentionItem {
            key: "sci_triage:7".into(),
            kind: AttentionKind::SciTriage,
            ..item
        };
        let mut app = App::default();
        open_sci_attention_at(&triage, &mut app);
        assert_eq!(app.page, Page::Tree);
        assert_eq!(app.tree.pending_select, None);
        assert!(app.tree.stale);
    }

    /// U23: 研究方針のページでは `r` で読み直し、`o` で tree.md を開く。
    #[test]
    fn tree_page_keys_reload_and_open_tree_md() {
        let mut app = App {
            page: Page::Tree,
            content_focused: true,
            ..App::default()
        };
        let reload_key = KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE);
        assert!(handle_key(&mut app, reload_key).is_none());
        assert!(app.tree.stale, "r で読み直しの旗が立つ");

        let open_key = KeyEvent::new(KeyCode::Char('o'), KeyModifiers::NONE);
        assert!(matches!(
            handle_key(&mut app, open_key),
            Some(UiAction::OpenTreeMd)
        ));

        // ページ選択が失われていても、コンテンツにフォーカスがあれば効く。
        let mut app = App {
            page: Page::Dashboard,
            content_focused: false,
            ..App::default()
        };
        assert!(handle_key(&mut app, reload_key).is_none());
        assert!(!app.tree.stale, "フォーカスの外では効かない");
    }

    /// U23: `W` の state.md は、sci を使うプロジェクトでは木からの書き出しなので読むだけ。
    #[test]
    fn state_md_is_sci_backed_only_after_sci_init() -> Result<(), Box<dyn std::error::Error>> {
        let db = papr_core::Database::in_memory()?;
        assert!(!state_md_is_sci_backed(&db), "始めていなければ nvim で開く");

        // 一時フォルダのルートに埋め込みの定義を足して `sci init` 相当を再現する。
        let root = std::env::temp_dir().join(format!("papr-state-md-{}", std::process::id()));
        let defs = papr_core::sci::defs::load(&root);
        papr_core::sci::defs::init_tree(db.connection(), &defs, "problem")?;
        let _ = std::fs::remove_dir_all(&root);
        assert!(state_md_is_sci_backed(&db), "始めていれば読むだけ");
        Ok(())
    }

    /// S01: `J` はどのページでも研究日誌を開く。
    #[test]
    fn journal_key_opens_on_every_page() {
        for page in Page::ALL {
            let mut app = App {
                page,
                ..App::default()
            };
            let action = handle_key(
                &mut app,
                KeyEvent::new(KeyCode::Char('J'), KeyModifiers::NONE),
            );
            assert!(
                matches!(action, Some(UiAction::OpenJournal)),
                "J must open the journal on {page:?}"
            );
        }
    }

    /// 文字の入力中は `J` を文字として扱う（グローバルキーに奪われない）。
    #[test]
    fn journal_key_types_letter_while_searching() {
        let mut app = App {
            page: Page::Library,
            mode: AppMode::WorkspaceSearch,
            ..App::default()
        };
        let action = handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('J'), KeyModifiers::NONE),
        );
        assert!(
            !matches!(action, Some(UiAction::OpenJournal)),
            "J must not open the journal while typing"
        );
        assert_eq!(app.workspace_query, "J");
    }

    /// S01: `J` の経路は `apply_open_journal` → 内蔵 Markdown ビューア。
    /// 記録を作って開き、題が「研究日誌」で本文に日誌の行が入ることを確かめる。
    #[test]
    fn open_journal_at_opens_markdown_view() -> Result<(), Box<dyn std::error::Error>> {
        use std::fs;
        let root = std::env::temp_dir().join(format!(
            "sai-paper-tui-journal-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join(".sai/paper/papers"))?;
        fs::create_dir_all(root.join(".sai/outputs"))?;
        let roots = papr_core::PaperRoots {
            root: root.clone(),
            data_dir: root.join(".sai/paper"),
        };
        let db = papr_core::Database::in_memory()?;
        db.connection().execute(
            "INSERT INTO papers (id, title) VALUES (1, 'x')",
            [],
        )?;
        // ローカルの今日に入るよう、UTC に直した「今」を書く。
        let now_utc = chrono::Local::now()
            .naive_utc()
            .format("%Y-%m-%d %H:%M:%S")
            .to_string();
        db.connection().execute(
            "INSERT INTO activity_log (kind, paper_id, occurred_at)
             VALUES ('download', 1, ?1)",
            [now_utc],
        )?;

        let theme = crate::theme::Theme::load("nord")?;
        let mut app = App::default();
        open_journal_at(&db, &roots, &mut app, &theme);

        assert_eq!(app.mode, AppMode::MarkdownView);
        let view = app.md_view.as_ref().expect("md_view must be set");
        assert_eq!(view.title, "研究日誌");
        assert_eq!(view.subtitle, "最近 7 日");

        fs::remove_dir_all(&root).ok();
        Ok(())
    }

    #[test]
    fn project_keys_type_letters_while_searching() {
        // 検索欄（WorkspaceSearch）への入力中は `W` を文字として扱う。
        let mut app = App {
            page: Page::Library,
            mode: AppMode::WorkspaceSearch,
            ..App::default()
        };
        let action = handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('W'), KeyModifiers::NONE),
        );
        assert!(
            !matches!(action, Some(UiAction::OpenProjectFile(_))),
            "W must not open state.md while typing"
        );
        assert_eq!(app.workspace_query, "W");
    }

    /// N05: READ NEXT があるとき、j / k は選択を動かし、Enter はその論文を開く。
    #[test]
    fn project_read_next_moves_selection_and_opens_paper() {
        let mut app = App {
            page: Page::Project,
            content_focused: true,
            ..App::default()
        };
        app.project.read_next = vec![
            papr_core::progress::ReadNext {
                paper_id: 37,
                title: "First".into(),
                score: 5,
                reason: "引き継ぎで 1 回".into(),
            },
            papr_core::progress::ReadNext {
                paper_id: 12,
                title: "Second".into(),
                score: 3,
                reason: "note なし".into(),
            },
        ];

        handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE),
        );
        assert_eq!(app.project.read_next_selected, 1);
        assert_eq!(
            app.project.scroll, 0,
            "READ NEXT があるときはスクロールしない"
        );
        // 末尾の論文でさらに j を押すと、READ NEXT の下の節へスクロールする。
        handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE),
        );
        assert_eq!(app.project.read_next_selected, 1, "選択は末尾のまま");
        assert_eq!(
            app.project.scroll, 1,
            "READ NEXT の末尾でさらに j を出るとスクロール"
        );
        // 戻ると、READ NEXT の先頭で止まってスクロールは減らない。
        handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('k'), KeyModifiers::NONE),
        );
        assert_eq!(app.project.scroll, 1);
        assert_eq!(app.project.read_next_selected, 0);
        handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('k'), KeyModifiers::NONE),
        );
        assert_eq!(
            app.project.scroll, 0,
            "READ NEXT の先頭で k を押すとスクロールが減る"
        );
        assert_eq!(app.project.read_next_selected, 0);

        let action = handle_key(&mut app, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(matches!(action, Some(UiAction::OpenLibraryPaper(37))));
    }

    /// N05: READ NEXT が空のときは、j / k は今までどおりスクロールし、Enter は何もしない。
    #[test]
    fn project_scrolls_when_read_next_is_empty() {
        let mut app = App {
            page: Page::Project,
            content_focused: true,
            ..App::default()
        };

        handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE),
        );
        assert_eq!(app.project.scroll, 1);
        assert_eq!(app.project.read_next_selected, 0);
        let action = handle_key(&mut app, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(action.is_none(), "READ NEXT が空の Enter は何もしない");
    }

    /// Project ページの PageUp / PageDown は READ NEXT の有無にかかわらず 10 行動かす。
    #[test]
    fn project_page_keys_scroll_ten_lines() {
        let mut app = App {
            page: Page::Project,
            content_focused: true,
            ..App::default()
        };
        app.project.read_next = vec![papr_core::progress::ReadNext {
            paper_id: 37,
            title: "First".into(),
            score: 5,
            reason: "引き継ぎで 1 回".into(),
        }];

        handle_key(
            &mut app,
            KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE),
        );
        assert_eq!(app.project.scroll, 10);
        assert_eq!(app.project.read_next_selected, 0, "選択は動かさない");
        handle_key(
            &mut app,
            KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE),
        );
        assert_eq!(app.project.scroll, 20);
        handle_key(&mut app, KeyEvent::new(KeyCode::PageUp, KeyModifiers::NONE));
        assert_eq!(app.project.scroll, 10);
        // 0 未満にはならない。
        app.project.scroll = 3;
        handle_key(&mut app, KeyEvent::new(KeyCode::PageUp, KeyModifiers::NONE));
        assert_eq!(app.project.scroll, 0);
    }

    /// 端末はマウスを取っているので、どのページでもホイールでスクロールできる。
    #[test]
    fn mouse_wheel_scrolls_paper_detail_and_project() -> Result<(), Box<dyn std::error::Error>> {
        use crossterm::event::{MouseEvent, MouseEventKind};

        let wheel = |kind| MouseEvent {
            kind,
            column: 10,
            row: 10,
            modifiers: crossterm::event::KeyModifiers::NONE,
        };

        // PAPER 画面（Library から開いたローカル詳細）
        let mut app = App {
            page: Page::Library,
            mode: AppMode::PaperDetail,
            ..App::default()
        };
        app.paper_detail_local = Some(1);
        app.paper_detail_content_rows = 100;
        app.paper_detail_view_rows = 20;
        handle_mouse_event(&mut app, wheel(MouseEventKind::ScrollDown));
        assert_eq!(app.paper_detail_scroll, 1, "PAPER 画面のホイール下");
        handle_mouse_event(&mut app, wheel(MouseEventKind::ScrollUp));
        assert_eq!(app.paper_detail_scroll, 0, "PAPER 画面のホイール上");

        // PAPER 画面（Discover から開いたリモート詳細）
        let mut app = App {
            page: Page::Discover,
            mode: AppMode::PaperDetail,
            ..App::default()
        };
        app.paper_detail_content_rows = 100;
        app.paper_detail_view_rows = 20;
        handle_mouse_event(&mut app, wheel(MouseEventKind::ScrollDown));
        assert_eq!(app.paper_detail_scroll, 1);

        // Project ページ
        let mut app = App {
            page: Page::Project,
            content_focused: true,
            ..App::default()
        };
        handle_mouse_event(&mut app, wheel(MouseEventKind::ScrollDown));
        assert_eq!(app.project.scroll, 1, "Project ページのホイール下");
        handle_mouse_event(&mut app, wheel(MouseEventKind::ScrollUp));
        assert_eq!(app.project.scroll, 0, "Project ページのホイール上");

        // それ以外の画面は j / k と同じ（一覧の選択が動く）
        let mut app = App {
            page: Page::Discover,
            content_focused: true,
            ..App::default()
        };
        app.discovery.set_results(
            (0..5)
                .map(|i| remote_paper(&format!("2501.0000{i}"), "A"))
                .collect(),
        );
        handle_mouse_event(&mut app, wheel(MouseEventKind::ScrollDown));
        assert_eq!(app.discovery.selected, 1);
        handle_mouse_event(&mut app, wheel(MouseEventKind::ScrollUp));
        assert_eq!(app.discovery.selected, 0);

        // 研究方針のページ（j / k と同じで選択が動く）
        let mut app = App {
            page: Page::Tree,
            content_focused: true,
            ..App::default()
        };
        app.tree.status = Some(papr_core::sci::compute::Status {
            nodes: (1..=3)
                .map(|id| papr_core::sci::compute::NodeRow {
                    id: format!("N{id}"),
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        });
        handle_mouse_event(&mut app, wheel(MouseEventKind::ScrollDown));
        assert_eq!(app.tree.selected, 1, "研究方針のページのホイール下");
        handle_mouse_event(&mut app, wheel(MouseEventKind::ScrollUp));
        assert_eq!(app.tree.selected, 0, "研究方針のページのホイール上");

        // 入力欄（ダイアログ）では動かさない
        let mut app = App {
            page: Page::Project,
            mode: AppMode::CommandPalette,
            ..App::default()
        };
        handle_mouse_event(&mut app, wheel(MouseEventKind::ScrollDown));
        assert_eq!(app.project.scroll, 0, "ダイアログ中はホイールで動かない");
        Ok(())
    }

    /// N08: `N` / `U` は Dashboard と Project ページだけでカード作成のアクションになる。
    /// ペインの選択中でもサイドバー側でも同じに効く（M / W と同じ扱い）。
    #[test]
    fn agent_card_keys_work_on_dashboard_and_project() {
        for page in [Page::Dashboard, Page::Project] {
            for content_focused in [true, false] {
                let mut app = App {
                    page,
                    content_focused,
                    ..App::default()
                };
                let next = handle_key(
                    &mut app,
                    KeyEvent::new(KeyCode::Char('N'), KeyModifiers::NONE),
                );
                assert!(
                    matches!(next, Some(UiAction::CreateAgentCard(AgentCard::Next))),
                    "N must create a next-step card on {page:?} (focused={content_focused})"
                );
                let check = handle_key(
                    &mut app,
                    KeyEvent::new(KeyCode::Char('U'), KeyModifiers::NONE),
                );
                assert!(
                    matches!(check, Some(UiAction::CreateAgentCard(AgentCard::Check))),
                    "U must create an understanding-check card on {page:?} (focused={content_focused})"
                );
            }
        }
    }

    /// N08: `N` / `U` は他のページでは何もしない（キーの衝突がない）。
    #[test]
    fn agent_card_keys_do_nothing_on_other_pages() {
        for page in Page::ALL {
            if matches!(page, Page::Dashboard | Page::Project) {
                continue;
            }
            let mut app = App {
                page,
                content_focused: true,
                ..App::default()
            };
            let action = handle_key(
                &mut app,
                KeyEvent::new(KeyCode::Char('N'), KeyModifiers::NONE),
            );
            assert!(
                !matches!(action, Some(UiAction::CreateAgentCard(_))),
                "N must not create a card on {page:?}"
            );
        }
    }

    /// N08: 文字入力中は `N` を文字として扱い、カードを作らない。
    #[test]
    fn agent_card_keys_type_letters_while_searching() {
        let mut app = App {
            page: Page::Project,
            content_focused: true,
            mode: AppMode::WorkspaceSearch,
            ..App::default()
        };
        let action = handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('N'), KeyModifiers::NONE),
        );
        assert!(
            !matches!(action, Some(UiAction::CreateAgentCard(_))),
            "N must not create a card while typing"
        );
        assert_eq!(app.workspace_query, "N");
    }

    /// N08 のテスト用に、一時ルートへブロック定義と最小の sai-link DB を置く。
    fn setup_agent_card_project(root: &std::path::Path, block_ids: &[&str]) {
        let blocks = root.join(".sai").join("blocks");
        fs::create_dir_all(&blocks).unwrap();
        for block_id in block_ids {
            fs::write(
                blocks.join(format!("{block_id}.md")),
                format!("---\nname: {block_id}\nid: {block_id}\ndescription: test block\n---\n\n# task_text\n"),
            )
            .unwrap();
        }
        let engine = root.join(".sai").join("sai-engine");
        fs::create_dir_all(&engine).unwrap();
        let conn = rusqlite::Connection::open(engine.join("sai-engine.db")).unwrap();
        conn.execute_batch(
            "CREATE TABLE block_metadata (id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL UNIQUE, display_name TEXT, description TEXT, model TEXT, thinking TEXT, tools TEXT, extensions TEXT, max_turns INTEGER, timeout INTEGER, worktree TEXT, enable INTEGER, file_path TEXT NOT NULL);\n             CREATE TABLE execution_history (id INTEGER PRIMARY KEY AUTOINCREMENT, block_id INTEGER, status TEXT, user_inputs_json TEXT, execution_mode TEXT);",
        )
        .unwrap();
    }

    /// N08: `N` → task.md ができ、もう一度 `N` → 作られず既存の run を返す。
    #[test]
    fn agent_card_creates_once_then_reuses_todo_card() {
        let root = std::env::temp_dir().join(format!(
            "sai-paper-agent-card-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        setup_agent_card_project(&root, &["paper-next", "paper-check"]);
        let data_dir = root.join(".sai").join("paper");
        fs::create_dir_all(&data_dir).unwrap();
        let roots = PaperRoots {
            root: root.clone(),
            data_dir,
        };
        let db = Database::in_memory().unwrap();

        // 1 回目: カードが 1 枚できる。
        let first = create_agent_card(&db, &roots, AgentCard::Next, "pi").unwrap();
        assert_eq!(first, AgentCardOutcome::Created { run_id: 1 });
        let task = root.join(".sai/outputs/todo/1/task.md");
        let content = fs::read_to_string(&task).unwrap();
        assert!(content.contains("block: paper-next"), "content={content}");
        assert!(content.contains("tag: 次の一手"), "content={content}");
        assert!(content.contains("# task_text"), "content={content}");
        assert!(
            content.contains("次の一手を提案してください。"),
            "content={content}"
        );

        // 2 回目: 作られず、既存の run を返す。
        let second = create_agent_card(&db, &roots, AgentCard::Next, "pi").unwrap();
        assert_eq!(second, AgentCardOutcome::Existing { run_id: 1 });
        let todo_count = fs::read_dir(root.join(".sai/outputs/todo")).unwrap().count();
        assert_eq!(todo_count, 1, "同じカードが 2 枚できない");

        // `U` は別ブロックなので、別のカードとして作られる。
        let check = create_agent_card(&db, &roots, AgentCard::Check, "pi").unwrap();
        assert!(matches!(check, AgentCardOutcome::Created { .. }));
        let check_task = root.join(".sai/outputs/todo/2/task.md");
        let check_content = fs::read_to_string(&check_task).unwrap();
        assert!(check_content.contains("block: paper-check"), "content={check_content}");
        assert!(check_content.contains("tag: 理解の確認"), "content={check_content}");

        fs::remove_dir_all(&root).unwrap();
    }

    /// N08: ブロック定義が無ければ作らず、配り方を案内する。
    #[test]
    fn agent_card_reports_missing_block() {
        let root = std::env::temp_dir().join(format!(
            "sai-paper-agent-card-noblock-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        setup_agent_card_project(&root, &[]);
        let roots = PaperRoots {
            root: root.clone(),
            data_dir: root.join(".sai").join("paper"),
        };
        let db = Database::in_memory().unwrap();

        let error = create_agent_card(&db, &roots, AgentCard::Next, "pi").unwrap_err();
        assert!(error.contains("paper-next ブロックがありません"), "error={error}");
        assert!(
            !root.join(".sai/outputs/todo").exists(),
            "ブロックが無いときはカードを作らない"
        );

        fs::remove_dir_all(&root).unwrap();
    }

    /// N08: トーストが「作成」と「既存」で変わる。
    #[test]
    fn agent_card_toast_differs_for_create_and_existing() {
        let root = std::env::temp_dir().join(format!(
            "sai-paper-agent-card-toast-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        setup_agent_card_project(&root, &["paper-next"]);
        let roots = PaperRoots {
            root: root.clone(),
            data_dir: root.join(".sai").join("paper"),
        };
        let db = Database::in_memory().unwrap();

        let mut app = App::default();
        apply_create_agent_card_at(AgentCard::Next, &db, &roots, &mut app);
        let created = app.toast.clone().unwrap_or_default();
        assert!(created.contains("作りました"), "toast={created}");
        assert!(created.contains("run 1"), "toast={created}");

        apply_create_agent_card_at(AgentCard::Next, &db, &roots, &mut app);
        let existing = app.toast.clone().unwrap_or_default();
        assert!(existing.contains("未実行の"), "toast={existing}");
        assert!(existing.contains("run 1"), "toast={existing}");

        fs::remove_dir_all(&root).unwrap();
    }
}
