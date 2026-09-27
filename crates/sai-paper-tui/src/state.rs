//! Application state machine and navigation commands.

use std::path::PathBuf;

use papr_core::models::{
    AuthorSummary, DashboardStats, LibraryPaper, RemotePaper, ResearchDashboard,
    TagSummary,
};
use papr_core::brief::{HandoffEntry, ReviewEntry};
use papr_core::plugins::PluginInfo;
use papr_core::progress::ReadNext;
use papr_core::project_files::ProjectState;
use papr_core::understanding::UnderstandingEntry;

/// Presentation state for one background transfer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DownloadStatus {
    /// Waiting for the downloader to start.
    Starting,
    /// Receiving bytes from the remote server.
    Running,
    /// Inspecting the completed PDF.
    ExtractingMetadata,
    /// Resolving bibliographic metadata.
    Enriching,
    /// Renaming the completed file.
    Renaming,
    /// Transfer and post-processing completed.
    Completed,
    /// Transfer or post-processing failed.
    Failed(String),
}

/// 手動 PDF 取り込みの対象論文。
///
/// 自動 DL が不可能な電子ジャーナルで、ユーザーがブラウザから
/// 保存した PDF を既存の論文レコードに紐付けるために使う。
#[derive(Debug, Clone)]
pub enum ManualImportPaper {
    /// ローカルライブラリに既に登録されている論文。
    Local {
        /// `papers` テーブルの ID。
        paper_id: i64,
        /// 表示・正規ファイル名生成に使う論文タイトル。
        title: String,
    },
    /// Discover / Dashboard から開いたリモート論文。
    Remote(RemotePaper),
}

/// 手動 PDF 取り込みモーダルの状態。
#[derive(Debug, Default)]
pub struct ManualImportState {
    /// 取り込み対象の論文。
    pub paper: Option<ManualImportPaper>,
    /// 候補 PDF の絶対パス（新しい順）。
    pub candidates: Vec<PathBuf>,
    /// カーソル位置。
    pub selected: usize,
}

/// Download progress shown in the Downloads page and status bar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownloadTask {
    /// Remote paper identifier.
    pub id: String,
    /// Display title.
    pub title: String,
    /// Bytes downloaded so far.
    pub downloaded: u64,
    /// Expected transfer size when known.
    pub total: Option<u64>,
    /// Associated local database paper.
    pub paper_id: Option<i64>,
    /// Current or final PDF path.
    pub pdf_path: Option<String>,
    /// Current presentation state.
    pub status: DownloadStatus,
    /// Metadata retained for retries and completion.
    pub remote_paper: Option<RemotePaper>,
    /// Time of failure, used for UI cleanup.
    pub failed_at: Option<std::time::Instant>,
}

impl DownloadTask {
    /// Display the paper title, falling back to the PDF file name.
    ///
    /// 論文の置き場を整理してから PDF のファイル名はすべて `paper.pdf` になったため、
    /// 題名があるときは題名を先に返す。
    #[must_use]
    pub fn display_name(&self) -> &str {
        let title = self
            .title
            .strip_suffix(".pdf")
            .unwrap_or(&self.title)
            .trim();
        if !title.is_empty() {
            return title;
        }
        self.pdf_path
            .as_deref()
            .and_then(|path| std::path::Path::new(path).file_stem())
            .and_then(|name| name.to_str())
            .unwrap_or("")
    }
}

/// Top-level application pages.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Page {
    /// Research overview.
    Dashboard,
    /// プロジェクトの現況と最新の引き継ぎ（state.md / mind.md）。
    Project,
    /// Remote discovery providers.
    Discover,
    /// BibTeX 手動インポート（`data/bibtex/` の .bib を Discover 風に一覧）。
    Import,
    /// Local catalog.
    Library,
    /// User-defined paper groups.
    Collections,
    /// Followed and cataloged authors.
    Authors,
    /// Markdown research notes.
    Notes,
    /// Remote search sets.
    SearchSets,
    /// Canvas（JSON Canvas 1.0、note_card / report_card の関連表示）。
    Canvas,
}

impl Page {
    /// Whether this page supports local/workspace search.
    #[must_use]
    pub const fn supports_workspace_search(self) -> bool {
        matches!(
            self,
            Self::Library | Self::Collections | Self::Authors | Self::Notes | Self::SearchSets
        )
    }

    /// All pages in sidebar order.
    pub const ALL: [Self; 10] = [
        Self::Dashboard,
        Self::Project,
        Self::Discover,
        Self::Import,
        Self::Library,
        Self::Collections,
        Self::Authors,
        Self::Notes,
        Self::SearchSets,
        Self::Canvas,
    ];

    /// Human-readable navigation label.
    #[must_use]
    pub const fn title(self) -> &'static str {
        match self {
            Self::Dashboard => "Dashboard",
            Self::Project => "Project",
            Self::Discover => "Discover",
            Self::Import => "Import",
            Self::Library => "Library",
            // 仕様 §3.1: Groups（フォルダベースコレクション）の後継は DB 由来タグ一覧。
            Self::Collections => "Tags",
            Self::Authors => "Authors",
            Self::Notes => "Notes",
            Self::SearchSets => "Search Sets",
            Self::Canvas => "Canvas",
        }
    }

    /// Parse a configuration string into a `Page`.
    #[must_use]
    pub fn from_config_str(s: &str) -> Option<Self> {
        let normalized = s.trim().to_lowercase().replace(['-', ' '], "_");
        match normalized.as_str() {
            "dashboard" => Some(Self::Dashboard),
            "project" | "projects" => Some(Self::Project),
            "discover" => Some(Self::Discover),
            "import" => Some(Self::Import),
            "library" => Some(Self::Library),
            "reading_queue" | "readingqueue" => Some(Self::Library),
            "collections" | "groups" | "group" | "tags" | "tag" => Some(Self::Collections),
            "bookmarks" | "bookmark" => Some(Self::Library),
            "authors" | "author" => Some(Self::Authors),
            "notes" | "note" => Some(Self::Notes),
            "canvas" | "canvases" => Some(Self::Canvas),
            "downloads" | "download" => Some(Self::Library),
            "history" => Some(Self::Dashboard),
            "statistics" | "stats" => Some(Self::Dashboard),
            // Settings 画面は廃止（設定は settings.json を直接編集）。過去の設定値は Dashboard に読み替える。
            "settings" => Some(Self::Dashboard),
            _ => None,
        }
    }

    /// Canonical configuration string representation for this page.
    #[must_use]
    pub const fn config_str(self) -> &'static str {
        match self {
            Self::Dashboard => "dashboard",
            Self::Project => "project",
            Self::Discover => "discover",
            Self::Import => "import",
            Self::Library => "library",
            Self::Collections => "collections",
            Self::Authors => "authors",
            Self::Notes => "notes",
            Self::SearchSets => "search_sets",
            Self::Canvas => "canvas",
        }
    }
}

/// Current input focus.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AppMode {
    /// Canvas 本体（開いている Canvas の編集・閲覧）。
    CanvasView,
    /// Canvas の新規作成 / 名前変更の入力。
    CanvasName,
    /// Canvas の削除確認。
    CanvasConfirmDelete,
    /// Normal page navigation.
    #[default]
    Normal,
    /// Fuzzy command lookup.
    CommandPalette,
    /// Shell command palette.
    TerminalCommand,
    /// Shortcut overlay.
    Help,
    /// Text entry for a discovery search.
    Search,
    /// Text entry for filtering the currently displayed discovery results.
    DiscoverFilter,
    /// Full paper metadata view.
    PaperDetail,
    /// Single-line collection input.
    Prompt,
    /// Workspace-local search.
    WorkspaceSearch,
    /// Confirm deletion of a paper or collection.
    ConfirmDelete,
    /// Internal PDF viewer.
    PdfView,
    /// 手動 PDF 取り込みモーダル（~/Downloads の候補から選択）。
    ManualImport,
    /// ブロック選択モーダル（task.md 発行用）。
    BlockPicker,
    /// RAG ハイブリッド検索。
    RagSearch,
    /// 内蔵 Markdown ビューア（読み取り専用）。
    MarkdownView,
    /// 過去の Discover 検索を再表示する QUERY 画面。
    QueryDetail,
    /// 実行中のバックグラウンド処理一覧（`shift+L`）。
    Tasks,
    /// 要対応の一覧（Project ページの `A`、S02）。
    AttentionList,
}

/// RAG 検索（自然言語・ハイブリッド）の状態。
#[derive(Debug, Clone)]
pub struct RagSearchState {
    /// 検索クエリテキスト。
    pub query: String,
    /// クエリ内のカーソル位置。
    pub query_cursor: usize,
    /// 検索結果（Hit レコード）。
    pub results: Vec<papr_core::retrieval::Hit>,
    /// 選択行のインデックス。
    pub selected: usize,
    /// スクロールオフセット。
    pub scroll: usize,
    /// スコープ（papers/notes/documents/all）。
    pub scope: String,
    /// 検索実行中フラグ。
    pub searching: bool,
    /// 「要約付き」で検索するか（Ctrl+e で切り替える。オンのときだけ LLM を呼ぶ）。
    pub evidence: bool,
    /// 各結果の要約（要約付きのときだけ。`results` と同じ並び）。
    pub summaries: Vec<Option<String>>,
    /// 各結果の関連度（要約付きのときだけ。`results` と同じ並び）。
    pub relevances: Vec<Option<u8>>,
    /// 関連度が低くて外した件数（要約付きのときだけ）。
    pub dropped: usize,
    /// 使った入力トークン（要約付きのときだけ）。
    pub prompt_tokens: u64,
    /// 使った出力トークン（要約付きのときだけ）。
    pub completion_tokens: u64,
    /// 要約に失敗した件数（要約付きのときだけ）。
    pub failed: usize,
}

impl Default for RagSearchState {
    fn default() -> Self {
        Self {
            query: String::new(),
            query_cursor: 0,
            results: Vec::new(),
            selected: 0,
            scroll: 0,
            scope: "all".to_string(),
            searching: false,
            evidence: false,
            summaries: Vec::new(),
            relevances: Vec::new(),
            dropped: 0,
            prompt_tokens: 0,
            completion_tokens: 0,
            failed: 0,
        }
    }
}

/// Target to delete.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeletionTarget {
    /// A paper to delete.
    Paper {
        /// ID of the paper in DB.
        id: i64,
        /// Title of the paper.
        title: String,
        /// Path to the PDF file.
        path: Option<std::path::PathBuf>,
    },
    /// A collection to delete.
    Collection {
        /// ID of the collection in DB.
        id: i64,
        /// Name of the collection.
        name: String,
        /// Path to the collection directory.
        path: Option<std::path::PathBuf>,
    },
}

/// Active metadata input prompt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MetadataPrompt {
    /// Target paper identifier.
    pub paper_id: Option<i64>,
    /// Tag 付与モーダルの対象論文（仕様 §3.1）。
    pub tag_paper_id: Option<i64>,
    /// Collection identifier when renaming.
    pub rename_collection_id: Option<i64>,
    /// Paper identifier when renaming a PDF.
    pub rename_paper_id: Option<i64>,
    /// Editable input value.
    pub value: String,
    /// Cursor position in the input value.
    pub cursor: usize,
    /// Selected existing collection.
    pub selected: usize,
    /// Current collection name, if assigning a paper already in a collection.
    pub current_collection: Option<String>,
    /// ブロック ID for handoff（handoff mode の場合）。
    pub handoff_block_id: Option<String>,
}

impl Default for MetadataPrompt {
    fn default() -> Self {
        Self {
            paper_id: None,
            tag_paper_id: None,
            rename_collection_id: None,
            rename_paper_id: None,
            value: String::new(),
            cursor: 0,
            selected: 0,
            current_collection: None,
            handoff_block_id: None,
        }
    }
}

/// Current state of a remote discovery request.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum DiscoveryStatus {
    /// No search has been submitted.
    #[default]
    Idle,
    /// A request is running in the background.
    Loading,
    /// Results are ready for browsing.
    Ready,
    /// The provider returned an error.
    Error(String),
}

/// Discover 検索入力欄のフォーカス対象（R3: word + year + limit + db の 4 段構成）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DiscoverInputFocus {
    /// 1 段目: 検索語。
    #[default]
    Word,
    /// 2 段目左: 発行年区間（例: `2010-2020`）。
    Year,
    /// 2 段目右: 取得件数（空欄時は 100）。
    Limit,
    /// 3 段目: データベース選択（s2, arxiv, pubmed, biorxiv, openalex）。
    Db,
}

impl DiscoverInputFocus {
    /// 次の入力欄へ（word → year → limit → db → word）。
    #[must_use]
    pub fn next(self) -> Self {
        match self {
            Self::Word => Self::Year,
            Self::Year => Self::Limit,
            Self::Limit => Self::Db,
            Self::Db => Self::Word,
        }
    }

    /// 前の入力欄へ（Shift+Tab）。
    #[must_use]
    pub fn prev(self) -> Self {
        match self {
            Self::Word => Self::Db,
            Self::Year => Self::Word,
            Self::Limit => Self::Year,
            Self::Db => Self::Limit,
        }
    }
}

/// Search query, results, and selection state.
#[derive(Debug, Default)]
pub struct DiscoveryState {
    /// Query currently shown in the search field.
    pub query: String,
    /// Cursor position in the query field.
    pub query_cursor: usize,
    /// 現在フォーカス中の入力欄（R3: word / year / limit / db の Tab 移動）。
    pub input_focus: DiscoverInputFocus,
    /// 発行年の区間指定（例: `2010-2020`、単年 `2019`）。空欄なら全期間。
    pub year: String,
    /// Cursor position in the year field.
    pub year_cursor: usize,
    /// 検索取得論文数（空欄時はデフォルト 100）。
    pub limit: String,
    /// Cursor position in the limit field.
    pub limit_cursor: usize,
    /// 検索対象のデータベース（s2, arxiv, pubmed, biorxiv, openalex）。
    /// 複数指定は RFC 3986 エンコード（カンマ区切り）で記録される。
    pub db: String,
    /// Cursor position in the database field.
    pub db_cursor: usize,
    /// Text used to refine the current result cache without starting a search.
    pub filter: String,
    /// Cursor position in the result filter.
    pub filter_cursor: usize,
    /// Papers returned by the most recent request.
    pub results: Vec<RemotePaper>,
    /// Ranked indexes into `results` that are visible after applying `filter`.
    #[doc(hidden)]
    pub filtered_indices: Vec<usize>,
    /// Zero-based page in the globally ranked result cache.
    pub page: usize,
    /// Selected result row.
    pub selected: usize,
    /// Vertical list scroll offset.
    pub scroll: usize,
    /// Remembered selection for each cached page.
    pub page_selections: Vec<usize>,
    /// Remembered scroll offset for each cached page.
    pub page_scrolls: Vec<usize>,
    /// Network request state.
    pub status: DiscoveryStatus,
    /// Monotonically increasing identifier for ignoring superseded searches.
    pub request_id: u64,
    /// Offset of the next remote candidate batch, when more results remain.
    pub next_batch_start: Option<u16>,
    /// Non-blocking progress or partial-failure message shown above the results.
    pub progress_message: Option<String>,
    /// リモート検索のトークン（同じ検索の追加読み込みを同じログへ追記するため）。
    pub search_log_id: Option<i64>,
    /// いま実行中の検索で実際に使った DB 名（`Searching arxiv, ...` の見出しと、
    /// 検索の記録の `source_dbs` に使う）。
    pub active_providers: Vec<String>,
    /// 重複排除キー → 過去の検索でのヒット回数（今回の検索を含む）。
    /// R2: Discover の `hits` 列に使う。検索完了時に集計して入れる。
    pub hit_counts: std::collections::HashMap<String, i64>,
    /// Vertical detail-page scroll offset.
    pub detail_scroll: u16,
}

impl DiscoveryState {
    /// Number of results displayed on one Discover page.
    pub const PAGE_SIZE: usize = 50;

    /// Number of available pages in the globally ranked result cache.
    #[must_use]
    pub fn page_count(&self) -> usize {
        self.visible_result_count().div_ceil(Self::PAGE_SIZE)
    }

    /// Results belonging to the currently visible page.
    #[must_use]
    pub fn current_page_results(&self) -> &[RemotePaper] {
        let start = self
            .page
            .saturating_mul(Self::PAGE_SIZE)
            .min(self.results.len());
        let end = (start + Self::PAGE_SIZE).min(self.results.len());
        &self.results[start..end]
    }

    /// Results on the visible page after applying the local filter.
    #[must_use]
    pub fn visible_page_results(&self) -> Box<dyn Iterator<Item = &RemotePaper> + '_> {
        if self.uses_unfiltered_fallback() {
            Box::new(self.current_page_results().iter())
        } else {
            Box::new(
                self.current_page_indices()
                    .iter()
                    .filter_map(|&index| self.results.get(index)),
            )
        }
    }

    /// Number of filtered results on the visible page.
    #[must_use]
    pub fn visible_page_len(&self) -> usize {
        if self.uses_unfiltered_fallback() {
            self.current_page_results().len()
        } else {
            self.current_page_indices().len()
        }
    }

    /// Total count after applying the local filter.
    #[must_use]
    pub fn filtered_result_count(&self) -> usize {
        self.visible_result_count()
    }

    /// Selected paper on the currently visible page.
    #[must_use]
    pub fn selected_paper(&self) -> Option<&RemotePaper> {
        if self.uses_unfiltered_fallback() {
            self.current_page_results().get(self.selected)
        } else {
            self.current_page_indices()
                .get(self.selected)
                .and_then(|&index| self.results.get(index))
        }
    }

    /// Replace the cached result set after one globally ranked search.
    pub fn set_results(&mut self, results: Vec<RemotePaper>) {
        self.results = results;
        self.rebuild_filter();
        self.page = 0;
        self.selected = 0;
        self.scroll = 0;
        self.page_selections = vec![0; self.page_count()];
        self.page_scrolls = vec![0; self.page_count()];
    }

    /// Start a new search and discard results belonging to the previous query.
    pub fn begin_search(&mut self) -> u64 {
        self.request_id = self.request_id.wrapping_add(1);
        self.search_log_id = None;
        self.active_providers.clear();
        self.hit_counts.clear();
        self.filter.clear();
        self.filter_cursor = 0;
        self.set_results(Vec::new());
        self.status = DiscoveryStatus::Loading;
        self.next_batch_start = None;
        self.progress_message = None;
        self.request_id
    }

    /// Replace an in-progress result snapshot without discarding the paper the user selected.
    pub fn update_results(&mut self, results: Vec<RemotePaper>) {
        let selected_id = self.selected_paper().map(|paper| paper.id.clone());
        self.results = results;
        self.rebuild_filter_with_selected(selected_id.as_deref());
        let page_count = self.page_count();
        self.page_selections.resize(page_count, 0);
        self.page_scrolls.resize(page_count, 0);

        self.page = self.page.min(page_count.saturating_sub(1));
        self.selected = self.selected.min(self.visible_page_len().saturating_sub(1));
    }

    /// Move to the next cached page, preserving the current page view state.
    pub fn next_page(&mut self) -> bool {
        if self.page + 1 >= self.page_count() {
            return false;
        }
        self.store_page_view();
        self.page += 1;
        self.restore_page_view();
        true
    }

    /// Move to the previous cached page, preserving the current page view state.
    pub fn previous_page(&mut self) -> bool {
        if self.page == 0 {
            return false;
        }
        self.store_page_view();
        self.page -= 1;
        self.restore_page_view();
        true
    }

    /// Save the current page's selection and scroll offset after rendering.
    pub fn store_page_view(&mut self) {
        if let Some(selection) = self.page_selections.get_mut(self.page) {
            *selection = self.selected;
        }
        if let Some(scroll) = self.page_scrolls.get_mut(self.page) {
            *scroll = self.scroll;
        }
    }

    fn restore_page_view(&mut self) {
        self.selected = self.page_selections.get(self.page).copied().unwrap_or(0);
        self.scroll = self.page_scrolls.get(self.page).copied().unwrap_or(0);
        self.selected = self.selected.min(self.visible_page_len().saturating_sub(1));
    }

    /// Re-rank the visible result indexes after a local filter edit.
    pub fn rebuild_filter(&mut self) {
        // A changed filter defines a new ranked result set.  Always start at
        // its best match rather than retaining an index from the old set.
        self.rebuild_filter_with_selected(None);
    }

    fn rebuild_filter_with_selected(&mut self, selected_id: Option<&str>) {
        self.filtered_indices = self
            .results
            .iter()
            .enumerate()
            .filter(|(_, paper)| {
                App::matches_query(
                    &self.filter,
                    &paper.title,
                    &paper.author_line(),
                    paper.doi.as_deref(),
                    Some(paper.id.as_str()),
                )
            })
            .map(|(index, _)| index)
            .collect();
        self.page_selections = vec![0; self.page_count()];
        self.page_scrolls = vec![0; self.page_count()];
        self.page = 0;
        self.selected = 0;
        self.scroll = 0;
        if let Some(selected_id) = selected_id
            && let Some(index) = self
                .filtered_indices
                .iter()
                .position(|&index| self.results[index].id == selected_id)
        {
            self.page = index / Self::PAGE_SIZE;
            self.selected = index % Self::PAGE_SIZE;
        }
    }

    fn current_page_indices(&self) -> &[usize] {
        let start = self
            .page
            .saturating_mul(Self::PAGE_SIZE)
            .min(self.filtered_indices.len());
        let end = (start + Self::PAGE_SIZE).min(self.filtered_indices.len());
        &self.filtered_indices[start..end]
    }

    fn visible_result_count(&self) -> usize {
        if self.uses_unfiltered_fallback() {
            self.results.len()
        } else {
            self.filtered_indices.len()
        }
    }

    fn uses_unfiltered_fallback(&self) -> bool {
        self.filter.is_empty() && self.filtered_indices.is_empty() && !self.results.is_empty()
    }
}

/// Import 画面のスキャン状態。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum ImportStatus {
    /// まだスキャンしていない（Import 画面未入場）。
    #[default]
    Idle,
    /// `data/bibtex/` をスキャン中（同期スキャンのため一瞬のみ表示）。
    Loading,
    /// スキャン完了（結果の有無は `results` で判断）。
    Ready,
    /// スキャン自体の失敗（ディレクトリ走査の IO エラーなど）。
    Error(String),
}

/// BibTeX インポート画面の状態（[`DiscoveryState`] から検索系を除いた形）。
#[derive(Debug, Default)]
pub struct ImportState {
    /// `data/bibtex/` の全 .bib エントリ（スキャン結果）。
    pub results: Vec<RemotePaper>,
    /// `>` フィルタ（DiscoverFilter と同じ入力体験）。
    pub filter: String,
    /// フィルタ入力のカーソル位置。
    pub filter_cursor: usize,
    /// `filter` 適用後に表示される `results` のインデックス。
    #[doc(hidden)]
    pub filtered_indices: Vec<usize>,
    /// 選択中の行。
    pub selected: usize,
    /// 垂直スクロール位置。
    pub scroll: usize,
    /// スキャン状態。
    pub status: ImportStatus,
    /// パースに失敗した .bib ファイルのメッセージ一覧。
    pub scan_errors: Vec<String>,
}

impl ImportState {
    /// フィルタが空でも `filtered_indices` が空でも結果をそのまま見せる
    /// （Discover と同じ fallback 規約）。
    fn uses_unfiltered_fallback(&self) -> bool {
        self.filter.is_empty() && self.filtered_indices.is_empty() && !self.results.is_empty()
    }

    /// 表示中の行数（フィルタ適用後）。
    #[must_use]
    pub fn visible_len(&self) -> usize {
        if self.uses_unfiltered_fallback() {
            self.results.len()
        } else {
            self.filtered_indices.len()
        }
    }

    /// 選択中の論文。
    #[must_use]
    pub fn selected_paper(&self) -> Option<&RemotePaper> {
        if self.uses_unfiltered_fallback() {
            self.results.get(self.selected)
        } else {
            self.filtered_indices
                .get(self.selected)
                .and_then(|&index| self.results.get(index))
        }
    }

    /// スキャン結果を差し替えて選択状態をリセットする（ステータスは Ready）。
    pub fn set_results(&mut self, papers: Vec<RemotePaper>, errors: Vec<String>) {
        self.results = papers;
        self.scan_errors = errors;
        self.rebuild_filter();
        self.selected = 0;
        self.scroll = 0;
        self.status = ImportStatus::Ready;
    }

    /// フィルタ変更後に表示インデックスを再構築する
    /// （[`App::matches_query`] で title / author / DOI / ID を判定）。
    pub fn rebuild_filter(&mut self) {
        self.filtered_indices = self
            .results
            .iter()
            .enumerate()
            .filter(|(_, paper)| {
                App::matches_query(
                    &self.filter,
                    &paper.title,
                    &paper.author_line(),
                    paper.doi.as_deref(),
                    Some(paper.id.as_str()),
                )
            })
            .map(|(index, _)| index)
            .collect();
        self.selected = 0;
        self.scroll = 0;
    }

    /// 選択を 1 つ上へ（先頭でクランプ）。
    pub fn move_up(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    /// 選択を 1 つ下へ（末尾でクランプ）。
    pub fn move_down(&mut self) {
        self.selected = (self.selected + 1).min(self.visible_len().saturating_sub(1));
    }

    /// 選択を行数に合わせてクランプする（再スキャン後など）。
    pub fn clamp_selection(&mut self) {
        self.selected = self.selected.min(self.visible_len().saturating_sub(1));
    }
}

/// Local catalog rows and selection.
#[derive(Debug, Default)]
pub struct LibraryState {
    /// Papers currently loaded from `SQLite`.
    pub papers: Vec<LibraryPaper>,
    /// Selected library row.
    pub selected: usize,
    /// Vertical list scroll offset.
    pub scroll: usize,
    /// Whether a background filesystem scan is active.
    pub indexing: bool,
    /// Last indexing summary or failure.
    pub message: Option<String>,
}

/// Library の表示モード。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LibraryView {
    /// 通常の 1 行 1 論文リスト。
    #[default]
    Flat,
    /// 検索履歴とダウンロード済み論文の階層表示。
    Hierarchy,
}

/// 階層表示の行の種別。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LibraryTreeKind {
    /// 検索履歴の query 行。
    Query,
    /// 検索を経由しない取り込みの Import 行。
    Import,
    /// 論文行。
    Paper,
}

/// 階層表示の 1 行（表示中の行のみ保持する）。
#[derive(Debug, Clone)]
pub struct LibraryTreeRow {
    /// 0 = query/import 行、1 = paper 行。
    pub depth: u8,
    /// 行の種別。
    pub kind: LibraryTreeKind,
    /// query/import 行: 見出しラベル。
    pub label: String,
    /// query/import 行: 子の件数。
    pub child_count: usize,
    /// query/import 行: 折りたたみキー（`log:<id>` / `import:<date>`）。
    pub fold_key: Option<String>,
    /// query 行: ログ ID（QUERY 画面遷移用）。
    pub log_id: Option<i64>,
    /// paper 行: ローカル論文 ID。
    pub paper_id: Option<i64>,
    /// paper 行: 出版年月（`YYYY/MM`）。
    pub year_month: String,
    /// paper 行: 被引用数。
    pub citation_count: Option<i64>,
    /// paper 行: ジャーナル。
    pub journal: Option<String>,
    /// paper 行: タイトル。
    pub title: String,
}

/// QUERY 画面（過去の Discover 検索の再表示）の状態。
#[derive(Debug, Default)]
pub struct QueryDetailState {
    /// 表示中の検索ログ ID。
    pub log_id: i64,
    /// 検索ワード。
    pub query: String,
    /// 発行年区間。
    pub year: String,
    /// 取得件数。
    pub limit_txt: String,
    /// 検索対象 DB。
    pub source_dbs: String,
    /// 検索理由（編集可）。
    pub reason: String,
    /// Reason を編集中か。
    pub editing_reason: bool,
    /// Reason 入力のカーソル位置。
    pub reason_cursor: usize,
    /// ヒットした論文（全件）。
    pub items: Vec<QueryDetailItem>,
    /// 選択中の行。
    pub selected: usize,
    /// スクロール位置。
    pub scroll: usize,
}

/// QUERY 画面の論文 1 件。
#[derive(Debug, Clone)]
pub struct QueryDetailItem {
    /// リモート論文メタデータ。
    pub remote: RemotePaper,
    /// ダウンロード済みのローカル論文 ID（未ダウンロードなら None）。
    pub local_id: Option<i64>,
}

/// 実行中の長時間処理（論文パイプライン / `s` / `T` / abstract 翻訳）。
#[derive(Debug, Clone)]
pub struct RunningOperation {
    /// 重複判定用キー（`pipeline:<paper_id>` / `citation:<id>` / `translate:<paper_id>`）。
    pub key: String,
    /// ステータスバーに出す 1 行ラベル。
    pub label: String,
    /// TASKS 画面に出す名前（論文タイトル等）。
    pub title: String,
    /// この処理のステップ名（例: `PDF取得` / `図抽出` / `翻訳`）。
    pub steps: Vec<String>,
    /// 現在のステップ番号（0 始まり）。= 完了済みステップ数。
    pub step: usize,
    /// 開始時刻（経過時間表示用）。
    pub started_at: std::time::Instant,
}

impl RunningOperation {
    /// 進捗率（%）。完了済みステップ数 / 全ステップ数。
    ///
    /// docling も pdf2zh も実進捗を報告しないため、ステップ単位の近似のみを出す。
    #[must_use]
    pub fn percent(&self) -> u16 {
        let total = self.steps.len();
        if total == 0 {
            return 0;
        }
        let done = self.step.min(total);
        u16::try_from(done * 100 / total).unwrap_or(100)
    }

    /// 実行中のステップ名。全ステップ完了後は最後のステップ名を返す。
    #[must_use]
    pub fn step_label(&self) -> &str {
        self.steps
            .get(self.step)
            .or_else(|| self.steps.last())
            .map_or("処理中", String::as_str)
    }

    /// 開始からの経過時間を `M:SS` で返す。
    #[must_use]
    pub fn elapsed_text(&self) -> String {
        let secs = self.started_at.elapsed().as_secs();
        format!("{}:{:02}", secs / 60, secs % 60)
    }

    /// テキストの進捗バー（`width` 文字）。
    #[must_use]
    pub fn progress_bar(&self, width: usize) -> String {
        let filled = usize::from(self.percent()) * width / 100;
        format!(
            "[{}{}]",
            "#".repeat(filled),
            "-".repeat(width.saturating_sub(filled))
        )
    }
}

/// Commands available to keybindings and Browse Papr.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    /// Move selection up.
    MoveUp,
    /// Move selection down.
    MoveDown,
    /// Activate selected sidebar page.
    Open,
    /// Return focus to the navigation pane.
    Back,
    /// Show command lookup.
    TogglePalette,
    /// Show shortcut help.
    ToggleHelp,
    /// 実行中のバックグラウンド処理一覧を開閉する。
    ToggleTasks,
    /// Toggle workspace-local search focus.
    ToggleWorkspaceSearch,
    /// Terminate the application.
    Quit,
}

/// Infrequent session-level signals owned by the application event loop.
#[derive(Debug, Default)]
pub struct AppSessionFlags {
    /// Whether the event loop should stop.
    pub should_quit: bool,
    /// Whether the selected terminal completion has been inserted into input.
    pub terminal_completion_applied: bool,
    /// Whether background arXiv metadata enrichment is in progress.
    pub enrichment_pending: bool,
}

/// Secondary workspace state exposed through [`App`]'s dereference implementation.
#[derive(Debug)]
pub struct AppWorkspaceState {
    pub(crate) palette_selected: usize,
    pub(crate) palette_scroll: usize,
    pub(crate) palette_query: String,
    pub(crate) palette_query_cursor: usize,
    pub(crate) terminal_command: String,
    pub(crate) terminal_command_cursor: usize,
    pub(crate) terminal_command_output: String,
    pub(crate) terminal_command_directory: Option<std::path::PathBuf>,
    pub(crate) terminal_completions: Vec<String>,
    pub(crate) terminal_completion_selected: Option<usize>,
    pub(crate) terminal_completion_token_start: usize,
    pub(crate) workspace_query: String,
    pub(crate) workspace_query_cursor: usize,
    pub(crate) active_search_workspaces: std::collections::HashSet<Page>,
    pub(crate) pdf_viewer_path: Option<std::path::PathBuf>,
    pub(crate) pdf_viewer_page: usize,
    /// 内蔵 PDF ビューアを閉じたあとに戻るモード（md の表示画面から開いたとき用）。
    pub(crate) pdf_view_return: AppMode,
    pub(crate) pdf_viewer_scroll_y: u32,
    pub(crate) pdf_viewer_total_pages: usize,
    pub(crate) pdf_viewer_page_pixel_h: u32,
    pub(crate) pdf_viewer_max_scroll_y: u32,
    pub(crate) active_pdf_session_id: Option<i64>,
    pub(crate) active_pdf_session_start: Option<std::time::Instant>,
    pub(crate) pdf_viewer: String,
}

impl Default for AppWorkspaceState {
    fn default() -> Self {
        Self {
            palette_selected: 0,
            palette_scroll: 0,
            palette_query: String::new(),
            palette_query_cursor: 0,
            terminal_command: String::new(),
            terminal_command_cursor: 0,
            terminal_command_output: String::new(),
            terminal_command_directory: None,
            terminal_completions: Vec::new(),
            terminal_completion_selected: None,
            terminal_completion_token_start: 0,
            workspace_query: String::new(),
            workspace_query_cursor: 0,
            active_search_workspaces: std::collections::HashSet::new(),
            pdf_viewer_path: None,
            pdf_viewer_page: 1,
            pdf_view_return: AppMode::Normal,
            pdf_viewer_scroll_y: 0,
            pdf_viewer_total_pages: 0,
            pdf_viewer_page_pixel_h: 0,
            pdf_viewer_max_scroll_y: 0,
            active_pdf_session_id: None,
            active_pdf_session_start: None,
            pdf_viewer: "internal".to_owned(),
        }
    }
}
/// Project ページに出すもの（[`crate::refresh_dashboard`] のときに作り直す）。
#[derive(Debug, Clone, Default)]
pub struct ProjectView {
    /// state.md から読み取った現況。
    pub state: ProjectState,
    /// state.md が存在するか（未記入の表示に使う）。
    pub state_exists: bool,
    /// 最新の引き継ぎ（最大 1 件）。
    pub latest_handoff: Option<HandoffEntry>,
    /// 最近のレビュー（最大 2 件）。
    pub recent_reviews: Vec<ReviewEntry>,
    /// 理解の状態（トピックごとの最新の一覧）。
    pub understanding: Vec<UnderstandingEntry>,
    /// 読むべき論文（READ NEXT。`refresh_dashboard` で作り直す）。
    pub read_next: Vec<ReadNext>,
    /// READ NEXT で選んでいる行の番号。
    pub read_next_selected: usize,
    /// DB 全体の出典の問題の件数（要確認=error / 注意=warning）。
    pub citation_issues: papr_core::IssueCounts,
    /// 登録済みの report の総数（0 なら出典の行を出さない）。
    pub report_count: usize,
    /// 要対応の一覧（S02。`refresh_dashboard` で作り直す）。
    pub attention: Vec<papr_core::attention::AttentionItem>,
    /// ページ全体の縦スクロール位置。
    pub scroll: u16,
}

/// Complete UI state independent of terminal rendering.
#[derive(Debug)]
pub struct App {
    /// Active content page.
    pub page: Page,
    /// data ディレクトリ（docling Markdown 出力先 `extracts/` の親）。
    pub data_dir: std::path::PathBuf,
    /// Selected sidebar row.
    pub sidebar_index: usize,
    /// Vertical list scroll offset for the sidebar.
    pub sidebar_scroll: usize,
    /// Whether keyboard input targets section content instead of the navigation pane.
    pub content_focused: bool,
    /// Current modal input mode.
    pub mode: AppMode,
    /// First visible row in the complete keyboard reference.
    pub help_scroll: usize,
    /// Mode to restore after dismissing the keyboard reference.
    pub help_return_mode: AppMode,
    /// 要対応の一覧（S02）で選んでいる行の番号。
    pub attention_selected: usize,
    /// Session-level signals consumed by the event loop.
    pub session_flags: AppSessionFlags,
    /// Summary metrics loaded from persistence.
    pub stats: DashboardStats,
    /// Complete local dashboard snapshot.
    pub dashboard: ResearchDashboard,
    /// Project ページの現況・引き継ぎ（refresh_dashboard で作り直す）。
    pub project: ProjectView,
    /// Latest papers loaded for Today in Research.
    pub today_papers: Vec<RemotePaper>,
    /// Selected dashboard paper row.
    pub today_selected: usize,
    /// Vertical list scroll offset for the dashboard feed.
    pub today_scroll: usize,
    /// Loading state for the latest-paper feed.
    pub today_status: DiscoveryStatus,
    /// Secondary state for search, palettes, citations, editors, and PDF viewing.
    pub(crate) workspace: AppWorkspaceState,
    /// Remote paper discovery state.
    pub discovery: DiscoveryState,
    /// BibTeX import state（Import 画面）。
    pub import: ImportState,
    /// Scroll offset within the currently open paper detail view.
    pub paper_detail_scroll: u16,
    /// PAPER 詳細（PaperDetail モード）で表示中のリモート論文。
    /// Discover / Dashboard / Import で Enter した際に [`crate::UiAction::OpenPaper`]
    /// で保存され、検索結果のストリーミング更新で一覧の選択位置が変化しても
    /// 詳細画面は常に「開いた論文」を表示し続ける（リスト先頭を誤表示しないための
    /// 修正）。`Command::Back` で詳細から戻る際にクリアする。
    pub paper_detail_remote: Option<RemotePaper>,
    /// PAPER 詳細（PaperDetail モード）で表示中のローカル論文 ID（Library）。
    /// Library で Enter した際に [`crate::UiAction::OpenLibraryPaper`]
    /// で保存され、一覧の再読み込み（reindex 等）で選択位置が変化しても
    /// 詳細画面は常に「開いた論文」を表示し続ける。`Command::Back` でクリアする。
    pub paper_detail_local: Option<i64>,
    /// Remote PAPER 詳細に対応する DB 行 ID（`papers` テーブル）。
    /// `UiAction::OpenPaper` の `ensure_remote_paper` で解決され、概要訳
    /// （`abstract_ja`）の再取得や翻訳トリガーに使う。`Command::Back` でクリアする。
    pub paper_detail_remote_db_id: Option<i64>,
    /// Remote PAPER 詳細で表示する概要（日本語訳）。DB から取得し、翻訳完了の
    /// `AppEvent::RefreshLibrary` 受信時に取り直す。
    pub paper_detail_remote_ja: Option<String>,
    /// Canvas 画面の状態。
    pub canvas: crate::canvas_view::CanvasState,
    /// 内蔵 Markdown ビューアを閉じたあとに戻るモード。
    pub md_view_return: AppMode,
    /// PAPER 画面（Library）の FIGURES セクションに出す抽出済みの図。
    /// 未抽出・失敗・抽出中のときは空。
    pub paper_figures: Vec<papr_core::figures::FigureRow>,
    /// 図がまだ抽出されていないか（`(図は未抽出)` の 1 行を出すかどうか）。
    /// 図がある場合・抽出済みで 0 件の場合は false。
    pub paper_figures_not_extracted: bool,
    /// PAPER 画面の内容全体の行数（テキスト + FIGURES）。描画時に更新する。
    pub paper_detail_content_rows: usize,
    /// PAPER 画面の本文表示領域の行数。描画時に更新する（スクロール上限用）。
    pub paper_detail_view_rows: u16,
    /// PAPER 画面を開いた直後に FIGURES の欄へ移すか（引用の `Fig.` から開いたとき）。
    /// 描画時に 1 度だけ使って false に戻す。
    pub paper_detail_jump_to_figures: bool,
    /// Local paper catalog state.
    pub library: LibraryState,
    /// Library の表示モード（通常 / 階層）。
    pub library_view: LibraryView,
    /// 階層表示の表示中行。
    pub library_tree: Vec<LibraryTreeRow>,
    /// 階層表示の選択行。
    pub library_tree_selected: usize,
    /// 階層表示のスクロール位置。
    pub library_tree_scroll: usize,
    /// 階層表示で折りたたまれている query/import 行のキー。
    pub library_tree_folded: std::collections::HashSet<String>,
    /// 階層表示の再構築が必要か。
    pub library_tree_dirty: bool,
    /// QUERY 画面の状態。
    pub query_detail: QueryDetailState,
    /// QUERY 画面から戻る先のモード。
    pub query_detail_return: AppMode,
    /// PAPER 画面から戻る先のモード（QUERY 画面経由なら QueryDetail）。
    pub paper_detail_return: AppMode,
    /// 実行中の長時間処理（`s` / `T`）の一覧。完了まで表示し続ける。
    pub running_operations: Vec<RunningOperation>,
    /// `jobs` 表の最近の行（TASKS 画面用。CLI / phone の処理と終わった処理も出す）。
    pub jobs: Vec<papr_core::jobs::JobRow>,
    /// `jobs` を最後に読んだ時刻（TASKS 画面を開いている間、1 秒ごとに読み直す）。
    pub jobs_loaded_at: Option<std::time::Instant>,
    /// Background PDF transfers.
    pub downloads: Vec<DownloadTask>,
    /// 読み取り専用の内蔵 Markdown ビューアの状態（開いているときのみ `Some`）。
    pub md_view: Option<crate::md_view::MarkdownView>,
    /// md の表示画面から別の md（引用の `§` で開く extract.md など）を開いたとき、
    /// 元の画面を 1 つだけ覚える。`Esc` で元の md に戻すために使う（V06）。
    pub md_view_previous: Option<Box<crate::md_view::MarkdownView>>,
    /// [`App::md_view_previous`] と一対の、元の画面の report_id。
    pub md_view_report_previous: Option<String>,
    /// [`App::md_view_previous`] と一対の、元の画面を閉じたあとに戻るモード。
    pub md_view_return_previous: AppMode,
    /// 内蔵ビューアが report を表示しているときの report_id（それ以外は `None`）。
    /// `C` キー（外部の照合）を report のときだけ受け付けるために使う。
    pub md_view_report: Option<String>,
    /// 開いている論文の REPORTS 欄に出す report（最大 5 本）と、その問題の件数。
    pub paper_reports: Vec<(papr_core::Report, papr_core::IssueCounts)>,
    /// 開いている論文を引用した report の総数（「…ほか N 本」の表示用）。
    pub paper_report_total: usize,
    /// `space` リーダー待機中か（`space` → `n`/`m` の 2 キー操作）。
    pub space_pending: bool,
    /// Active collection prompt.
    pub metadata_prompt: Option<MetadataPrompt>,
    /// Tag summaries（仕様 §3.1: Groups の後継）。
    pub tags: Vec<TagSummary>,
    /// Selected tag row.
    pub tag_selected: usize,
    /// Vertical list scroll offset for the tags list.
    pub tag_scroll: usize,
    /// タグ一覧で Enter した際に開いたタグ（R2: タグ → 論文一覧）。
    /// `None` ならタグ一覧を表示する。
    pub active_tag: Option<TagSummary>,
    /// 選択中タグに紐づく論文一覧（R2）。
    pub tag_papers: Vec<LibraryPaper>,
    /// タグの論文一覧での選択行。
    pub tag_paper_selected: usize,
    /// タグの論文一覧のスクロール位置。
    pub tag_paper_scroll: usize,
    /// Selected RECENT ACTIVITY row on the Dashboard.
    pub activity_selected: usize,

    /// Vertical scroll offset for RECENT ACTIVITY.
    pub activity_scroll: usize,
    /// Active delete confirmation state.
    pub delete_confirmation: Option<DeletionTarget>,
    /// Authors.
    pub authors: Vec<AuthorSummary>,
    /// Selected author row.
    pub author_selected: usize,
    /// Vertical list scroll offset for the authors list.
    pub author_scroll: usize,
    /// Author currently opened for paper browsing.
    pub active_author: Option<AuthorSummary>,
    /// Papers assigned to the active author.
    pub author_papers: Vec<LibraryPaper>,
    /// Selected paper within the active author.
    pub author_paper_selected: usize,
    /// Vertical list scroll offset for papers within an author.
    pub author_paper_scroll: usize,
    /// Most recently opened author, used to restore its paper cursor.
    pub last_opened_author_id: Option<i64>,
    /// Short user-facing operation result.
    pub toast: Option<String>,
    /// Timestamp when toast was last set.
    pub toast_timestamp: Option<std::time::Instant>,
    /// Mode restored when an editor or prompt closes.
    pub modal_return: AppMode,
    /// Discovered plugin summaries.
    pub plugins: Vec<PluginInfo>,
    /// Number of invalid plugin bundles found at startup.
    pub plugin_diagnostics: usize,
    /// Papers with associated notes.
    pub notes_papers: Vec<LibraryPaper>,
    /// Selected paper with notes.
    pub notes_selected: usize,
    /// Vertical scroll offset for notes papers.
    pub notes_scroll: usize,
    /// 手動 PDF 取り込みモーダルの状態。
    pub manual_import: ManualImportState,
    /// 検索セット一覧。
    pub search_sets: Vec<papr_core::SearchSet>,
    /// 検索セット一覧での選択行。
    pub search_set_selected: usize,
    /// 検索セット一覧のスクロール位置。
    pub search_set_scroll: usize,
    /// 選択中の検索セット内の候補一覧。
    pub search_set_items: Vec<papr_core::SearchSetItem>,
    /// 候補一覧での選択行。
    pub search_set_item_selected: usize,
    /// 候補一覧のスクロール位置。
    pub search_set_item_scroll: usize,
    /// SearchSets ページでどのペインにフォーカスがあるか（true: 右・候補、false: 左・セット一覧）。
    pub search_set_focused_on_items: bool,
    /// 検索セットからのダウンロードの文脈を追跡（paper_id -> (set_id, position)）。
    pub search_set_download_context: std::collections::HashMap<String, (i64, i64)>,
    /// Available startup page choices.
    pub startup_page_options: Vec<String>,
    /// ブロック選択モーダルの状態。
    pub block_picker: crate::block_picker::BlockPickerState,
    /// RAG ハイブリッド検索の状態。
    pub rag_search: RagSearchState,
}

impl std::ops::Deref for App {
    type Target = AppWorkspaceState;

    fn deref(&self) -> &Self::Target {
        &self.workspace
    }
}

impl std::ops::DerefMut for App {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.workspace
    }
}

impl Default for App {
    fn default() -> Self {
        Self {
            page: Page::Dashboard,
            data_dir: std::path::PathBuf::new(),
            sidebar_index: 0,
            sidebar_scroll: 0,
            content_focused: false,
            mode: AppMode::Normal,
            help_scroll: 0,
            help_return_mode: AppMode::Normal,
            attention_selected: 0,
            session_flags: AppSessionFlags::default(),
            stats: DashboardStats::default(),
            dashboard: ResearchDashboard::default(),
            project: ProjectView::default(),
            today_papers: Vec::new(),
            today_selected: 0,
            today_scroll: 0,
            today_status: DiscoveryStatus::Idle,
            workspace: AppWorkspaceState::default(),
            discovery: DiscoveryState::default(),
            import: ImportState::default(),
            paper_detail_scroll: 0,
            paper_detail_remote: None,
            paper_detail_local: None,
            paper_detail_remote_db_id: None,
            paper_detail_remote_ja: None,
            canvas: crate::canvas_view::CanvasState::default(),
            md_view_return: AppMode::Normal,
            paper_figures: Vec::new(),
            paper_figures_not_extracted: false,
            paper_detail_content_rows: 0,
            paper_detail_view_rows: 0,
            paper_detail_jump_to_figures: false,
            library: LibraryState::default(),
            library_view: LibraryView::default(),
            library_tree: Vec::new(),
            library_tree_selected: 0,
            library_tree_scroll: 0,
            library_tree_folded: std::collections::HashSet::new(),
            library_tree_dirty: true,
            query_detail: QueryDetailState::default(),
            query_detail_return: AppMode::Normal,
            paper_detail_return: AppMode::Normal,
            running_operations: Vec::new(),
            jobs: Vec::new(),
            jobs_loaded_at: None,
            downloads: Vec::new(),
            md_view: None,
            md_view_previous: None,
            md_view_report_previous: None,
            md_view_return_previous: AppMode::Normal,
            md_view_report: None,
            paper_reports: Vec::new(),
            paper_report_total: 0,
            space_pending: false,
            metadata_prompt: None,
            manual_import: ManualImportState::default(),
            tags: Vec::new(),
            tag_selected: 0,
            tag_scroll: 0,
            active_tag: None,
            tag_papers: Vec::new(),
            tag_paper_selected: 0,
            tag_paper_scroll: 0,
            activity_selected: 0,
            activity_scroll: 0,
            delete_confirmation: None,
            authors: Vec::new(),
            author_selected: 0,
            author_scroll: 0,
            active_author: None,
            author_papers: Vec::new(),
            author_paper_selected: 0,
            author_paper_scroll: 0,
            last_opened_author_id: None,
            toast: None,
            toast_timestamp: None,
            modal_return: AppMode::Normal,
            plugins: Vec::new(),
            plugin_diagnostics: 0,
            notes_papers: Vec::new(),
            notes_selected: 0,
            notes_scroll: 0,
            search_sets: Vec::new(),
            search_set_selected: 0,
            search_set_scroll: 0,
            search_set_items: Vec::new(),
            search_set_item_selected: 0,
            search_set_item_scroll: 0,
            search_set_focused_on_items: false,
            search_set_download_context: std::collections::HashMap::new(),
            startup_page_options: vec!["dashboard".into(), "discover".into(), "library".into()],
            block_picker: crate::block_picker::BlockPickerState::new(Vec::new()),
            rag_search: RagSearchState::default(),
        }
    }
}

impl App {
    /// PAPER 画面がローカル論文の詳細を表示すべきか。
    ///
    /// リモート詳細（`paper_detail_remote`）が優先。ローカル論文の一覧ページ
    /// （Library / Tags / Authors / Notes）で開いた場合はローカル詳細を表示する。
    #[must_use]
    pub fn is_local_paper_detail(&self) -> bool {
        self.paper_detail_remote.is_none()
            && (self.paper_detail_local.is_some()
                || matches!(
                    self.page,
                    Page::Library | Page::Notes | Page::Collections | Page::Authors
                ))
    }

    /// Set the active page and synchronize `sidebar_index`.
    pub fn set_page(&mut self, page: Page) {
        self.page = page;
        if let Some(index) = Page::ALL.iter().position(|&p| p == page) {
            self.sidebar_index = index;
        }
    }

    /// Project ページの本文を `delta` 行スクロールする（ホイール用）。
    ///
    /// 上限は描画（`render_project`）がこれまでどおり丸めるので、ここでは
    /// 0 で止まることだけを守る。
    pub fn scroll_project(&mut self, delta: i32) {
        let current = i32::from(self.project.scroll);
        self.project.scroll = current.saturating_add(delta).max(0) as u16;
    }

    /// Returns the local PDF record for a remote paper when it has been downloaded.
    #[must_use]
    pub fn downloaded_remote_paper(&self, remote: &RemotePaper) -> Option<&LibraryPaper> {
        self.library.papers.iter().find(|local| {
            // The library list is an in-memory projection that can briefly lag
            // behind a filesystem delete (for example after a rename followed
            // immediately by deletion).  A stored path alone must not suppress
            // a new download: only an existing local PDF does that.
            if !local
                .pdf_path
                .as_deref()
                .is_some_and(|path| std::path::Path::new(path).is_file())
            {
                return false;
            }
            if local.arxiv_id.as_deref() == Some(remote.id.as_str()) {
                return true;
            }
            if let (Some(local_doi), Some(remote_doi)) = (&local.doi, &remote.doi)
                && local_doi.eq_ignore_ascii_case(remote_doi)
            {
                return true;
            }
            local.title.eq_ignore_ascii_case(&remote.title)
        })
    }

    /// Check if a query matches the title, authors, DOI, or arXiv ID case-insensitively.
    #[must_use]
    pub fn matches_query(
        query: &str,
        title: &str,
        authors: &str,
        doi: Option<&str>,
        arxiv_id: Option<&str>,
    ) -> bool {
        if query.is_empty() {
            return true;
        }
        let q = query.trim().to_ascii_lowercase();

        // 1. Check if the query matches an arXiv ID
        if let Some(q_arxiv) = parse_arxiv_id(&q)
            && let Some(p_arxiv_raw) = arxiv_id
            && let Some(p_arxiv) = parse_arxiv_id(&clean_arxiv_id(p_arxiv_raw))
            && strip_arxiv_version(&q_arxiv) == strip_arxiv_version(&p_arxiv)
        {
            return true;
        }

        // 2. Check if the query matches DOI
        if let Some(p_doi) = doi
            && p_doi.trim().to_ascii_lowercase() == q
        {
            return true;
        }

        // 3. Fall back to title and authors matching
        title.to_lowercase().contains(&q) || authors.to_lowercase().contains(&q)
    }

    /// Get the library papers filtered by the workspace search query.
    #[must_use]
    pub fn filtered_library_papers(&self) -> Vec<&LibraryPaper> {
        self.library
            .papers
            .iter()
            .filter(|p| {
                Self::matches_query(
                    &self.workspace_query,
                    &p.title,
                    &p.authors,
                    p.doi.as_deref(),
                    p.arxiv_id.as_deref(),
                )
            })
            .collect()
    }

    /// PAPER 詳細（Library）で操作対象とするローカル論文を返す。
    ///
    /// 詳細表示中（[`AppMode::PaperDetail`]）は Enter で開いた論文
    /// （[`App::paper_detail_local`]）を優先し、一覧の選択位置・フィルタ・
    /// 再読み込みの影響を受けない。一覧モードでは従来どおり選択行を返す。
    #[must_use]
    pub fn detail_library_paper(&self) -> Option<&LibraryPaper> {
        if self.mode == AppMode::PaperDetail
            && let Some(paper_id) = self.paper_detail_local
            && let Some(paper) = self.library.papers.iter().find(|p| p.id == paper_id)
        {
            return Some(paper);
        }
        let selected = self.library.selected;
        self.filtered_library_papers().get(selected).copied()
    }

    /// RECENT ACTIVITY の行数（Dashboard の j/k 範囲用）。
    #[must_use]
    pub fn activity_len(&self) -> usize {
        self.dashboard.recent_activity.len()
    }

    /// 選択中の RA 行の paper_id（enter で論文を開く用、仕様 §3.3）。
    #[must_use]
    pub fn selected_activity_paper_id(&self) -> Option<i64> {
        self.dashboard
            .recent_activity
            .get(self.activity_selected)
            .and_then(|item| item.paper_id)
    }

    /// Get the authors filtered by the workspace search query.
    #[must_use]
    pub fn filtered_authors(&self) -> Vec<&AuthorSummary> {
        self.authors
            .iter()
            .filter(|a| Self::matches_query(&self.workspace_query, &a.name, "", None, None))
            .collect()
    }

    /// Get the active author's papers filtered by the workspace search query.
    #[must_use]
    pub fn filtered_author_papers(&self) -> Vec<&LibraryPaper> {
        self.author_papers
            .iter()
            .filter(|p| {
                Self::matches_query(
                    &self.workspace_query,
                    &p.title,
                    &p.authors,
                    p.doi.as_deref(),
                    p.arxiv_id.as_deref(),
                )
            })
            .collect()
    }

    /// Get the notes papers filtered by the workspace search query.
    #[must_use]
    pub fn filtered_notes_papers(&self) -> Vec<&LibraryPaper> {
        self.notes_papers
            .iter()
            .filter(|p| {
                Self::matches_query(
                    &self.workspace_query,
                    &p.title,
                    &p.authors,
                    p.doi.as_deref(),
                    p.arxiv_id.as_deref(),
                )
            })
            .collect()
    }
    /// Get Browse Papr destinations filtered by the query.
    #[must_use]
    pub fn filtered_palette_items(&self) -> Vec<Page> {
        let q = self.palette_query.to_lowercase();
        Page::ALL
            .iter()
            .copied()
            .filter(|page| page.title().to_lowercase().contains(&q))
            .collect()
    }

    /// Apply a semantic command to the state machine.
    /// Apply a semantic command to the state machine.
    pub fn dispatch(&mut self, command: Command) {
        match command {
            Command::MoveUp => self.move_selection_up(),
            Command::MoveDown => self.move_selection_down(),
            Command::Open => {
                if !self.content_focused {
                    self.page = Page::ALL[self.sidebar_index];
                    self.content_focused = true;
                    if self.page == Page::Canvas {
                        // Canvas 一覧はタグから作るため、DB を持つ呼び出し側で読み直す。
                        self.canvas.names_stale = true;
                    }
                    if self.active_search_workspaces.contains(&self.page) {
                        self.mode = AppMode::WorkspaceSearch;
                    } else {
                        self.mode = AppMode::Normal;
                    }
                } else if self.page == Page::Discover && !self.discovery.results.is_empty() {
                    self.mode = AppMode::PaperDetail;
                    self.paper_detail_scroll = 0;
                } else if self.page == Page::Import && !self.import.results.is_empty() {
                    self.mode = AppMode::PaperDetail;
                    self.paper_detail_scroll = 0;
                }
            }
            Command::Back => {
                if self.mode == AppMode::PaperDetail {
                    self.mode = self.paper_detail_return;
                    self.paper_detail_return = AppMode::Normal;
                    self.space_pending = false;
                    // PAPER 詳細で保持していた論文をクリア（次回は選択中の論文を表示）。
                    self.paper_detail_remote = None;
                    self.paper_detail_local = None;
                    self.paper_detail_remote_db_id = None;
                    self.paper_detail_remote_ja = None;
                } else {
                    self.content_focused = false;
                }
            }
            Command::TogglePalette => {
                self.mode = if self.mode == AppMode::CommandPalette {
                    AppMode::Normal
                } else {
                    AppMode::CommandPalette
                };
                self.palette_selected = 0;
                self.palette_scroll = 0;
                self.palette_query.clear();
                self.palette_query_cursor = 0;
            }
            Command::ToggleHelp => {
                if self.mode == AppMode::Help {
                    self.mode = self.help_return_mode;
                } else {
                    self.help_return_mode = self.mode;
                    self.mode = AppMode::Help;
                    self.help_scroll = 0;
                }
            }
            Command::ToggleTasks => {
                // help と同じ開閉規約（戻り先を覚えて Esc で戻す）。
                if self.mode == AppMode::Tasks {
                    self.mode = self.help_return_mode;
                } else {
                    self.help_return_mode = self.mode;
                    self.mode = AppMode::Tasks;
                }
            }
            Command::ToggleWorkspaceSearch => {
                if self.page.supports_workspace_search() {
                    if self.mode == AppMode::WorkspaceSearch {
                        self.mode = AppMode::Normal;
                        self.content_focused = true;
                        let page = self.page;
                        self.workspace.active_search_workspaces.remove(&page);
                    } else {
                        self.mode = AppMode::WorkspaceSearch;
                        self.content_focused = true;
                        let page = self.page;
                        self.workspace.active_search_workspaces.insert(page);
                    }
                }
            }
            Command::Quit => self.session_flags.should_quit = true,
        }
    }

    fn move_selection_up(&mut self) {
        if self.content_focused {
            self.move_workspace_selection_up();
        } else {
            self.sidebar_index = self.sidebar_index.saturating_sub(1);
            self.page = Page::ALL[self.sidebar_index];
        }
        if self.mode == AppMode::WorkspaceSearch {
            let page = self.page;
            self.workspace.active_search_workspaces.insert(page);
        }
    }

    fn move_workspace_selection_up(&mut self) {
        if self.page == Page::Library && self.library_view == LibraryView::Hierarchy {
            self.library_tree_selected = self.library_tree_selected.saturating_sub(1);
            return;
        }
        let selection = match self.page {
            Page::Library => Some(&mut self.library.selected),
            Page::Notes => Some(&mut self.notes_selected),
            // R2: Tags 画面のカーソル移動。タグ → 論文一覧中は論文側を移動する。
            Page::Collections if self.active_tag.is_some() => {
                self.tag_paper_selected = self.tag_paper_selected.saturating_sub(1);
                return;
            }
            Page::Collections => Some(&mut self.tag_selected),
            _ => None,
        };
        if let Some(selection) = selection {
            if *selection == 0 {
                self.mode = AppMode::WorkspaceSearch;
            } else {
                *selection -= 1;
            }
            return;
        }
        match self.page {
            // 仕様 §3.3: Dashboard の j/k は RECENT ACTIVITY を移動する。
            Page::Dashboard => {
                self.activity_selected = self.activity_selected.saturating_sub(1);
            }
            Page::Discover => {
                self.discovery.selected = self.discovery.selected.saturating_sub(1);
                self.discovery.store_page_view();
            }
            Page::Import => {
                self.import.move_up();
            }
            Page::Project => {
                if self.project.read_next.is_empty() || self.project.read_next_selected == 0 {
                    self.project.scroll = self.project.scroll.saturating_sub(1);
                } else {
                    self.project.read_next_selected =
                        self.project.read_next_selected.saturating_sub(1);
                }
            }
            Page::Authors => {
                let selection = if self.active_author.is_some() {
                    &mut self.author_paper_selected
                } else {
                    &mut self.author_selected
                };
                if *selection == 0 {
                    self.mode = AppMode::WorkspaceSearch;
                } else {
                    *selection -= 1;
                }
            }
            _ => {}
        }
    }

    fn move_selection_down(&mut self) {
        if !self.content_focused {
            self.sidebar_index = (self.sidebar_index + 1).min(Page::ALL.len().saturating_sub(1));
            self.page = Page::ALL[self.sidebar_index];
            return;
        }
        self.move_workspace_selection_down();
    }

    /// コンテンツ領域の選択を 1 行下へ移動する（末尾でクランプ、副作用なし）。
    fn move_workspace_selection_down(&mut self) {
        if self.page == Page::Library && self.library_view == LibraryView::Hierarchy {
            self.library_tree_selected =
                (self.library_tree_selected + 1).min(self.library_tree.len().saturating_sub(1));
            return;
        }
        match self.page {
            Page::Dashboard => {
                self.activity_selected =
                    (self.activity_selected + 1).min(self.activity_len().saturating_sub(1));
            }
            Page::Discover => {
                self.discovery.selected = (self.discovery.selected + 1)
                    .min(self.discovery.visible_page_len().saturating_sub(1));
                self.discovery.store_page_view();
            }
            Page::Import => {
                self.import.move_down();
            }
            Page::Project => {
                if self.project.read_next.is_empty()
                    || self.project.read_next_selected + 1 >= self.project.read_next.len()
                {
                    self.project.scroll = self.project.scroll.saturating_add(1);
                } else {
                    self.project.read_next_selected = (self.project.read_next_selected + 1)
                        .min(self.project.read_next.len().saturating_sub(1));
                }
            }
            Page::Library => {
                self.library.selected = (self.library.selected + 1)
                    .min(self.filtered_library_papers().len().saturating_sub(1));
            }
            Page::Authors if self.active_author.is_some() => {
                self.author_paper_selected = (self.author_paper_selected + 1)
                    .min(self.filtered_author_papers().len().saturating_sub(1));
            }
            Page::Authors => {
                self.author_selected =
                    (self.author_selected + 1).min(self.filtered_authors().len().saturating_sub(1));
            }
            Page::Notes => {
                self.notes_selected = (self.notes_selected + 1)
                    .min(self.filtered_notes_papers().len().saturating_sub(1));
            }
            // R2: Tags 画面のカーソル移動（末尾でクランプ）。
            // タグ → 論文一覧中は論文側を移動する。
            Page::Collections if self.active_tag.is_some() => {
                self.tag_paper_selected =
                    (self.tag_paper_selected + 1).min(self.tag_papers.len().saturating_sub(1));
            }
            Page::Collections => {
                self.tag_selected = (self.tag_selected + 1).min(self.tags.len().saturating_sub(1));
            }
            _ => {}
        }
    }
}

fn strip_arxiv_version(id: &str) -> &str {
    if let Some(pos) = id.rfind('v')
        && id[pos + 1..].chars().all(|c| c.is_ascii_digit())
        && pos + 1 < id.len()
    {
        return &id[..pos];
    }
    id
}

fn clean_arxiv_id(s: &str) -> String {
    let s = s.trim().to_ascii_lowercase();
    let clean = if let Some(idx) = s.find("/abs/") {
        &s[idx + 5..]
    } else if let Some(idx) = s.find("/pdf/") {
        let after = &s[idx + 5..];
        after.strip_suffix(".pdf").unwrap_or(after)
    } else {
        &s
    };
    clean.trim().to_string()
}

fn parse_arxiv_id(s: &str) -> Option<String> {
    let s = s.trim().to_ascii_lowercase();
    let clean = clean_arxiv_id(&s);
    let (base, version) = if let Some(v_idx) = clean.rfind('v') {
        let (b, v) = clean.split_at(v_idx);
        let v_suffix = &v[1..];
        if !v_suffix.is_empty() && v_suffix.chars().all(|c| c.is_ascii_digit()) {
            (b, Some(v.to_string()))
        } else {
            (clean.as_str(), None)
        }
    } else {
        (clean.as_str(), None)
    };

    // Modern format: "YYMM.NNNN" or "YYMM.NNNNN"
    if base.len() >= 9 && base.len() <= 10 {
        let parts: Vec<&str> = base.split('.').collect();
        if parts.len() == 2 {
            let yymm = parts[0];
            let nnnn = parts[1];
            if yymm.len() == 4
                && yymm.chars().all(|c| c.is_ascii_digit())
                && (nnnn.len() == 4 || nnnn.len() == 5)
                && nnnn.chars().all(|c| c.is_ascii_digit())
            {
                let mut normalized = base.to_string();
                if let Some(v) = version {
                    normalized.push_str(&v);
                }
                return Some(normalized);
            }
        }
    }

    // Legacy format: "archive/YYMMNNN" or "subject-class/YYMMNNN"
    if let Some(slash_idx) = base.find('/') {
        let (cat, num) = base.split_at(slash_idx);
        let num = &num[1..];
        if num.len() == 7
            && num.chars().all(|c| c.is_ascii_digit())
            && !cat.is_empty()
            && cat
                .chars()
                .all(|c| c.is_ascii_alphabetic() || c == '-' || c == '.')
        {
            let mut normalized = base.to_string();
            if let Some(v) = version {
                normalized.push_str(&v);
            }
            return Some(normalized);
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use chrono::Utc;

    use papr_core::{LibraryPaper, RemotePaper};

    use super::{
        App, AppMode, Command, DiscoveryState, DownloadStatus, DownloadTask, ImportState,
        ImportStatus, Page, RunningOperation,
    };

    /// 題名が空のときだけ PDF のファイル名（拡張子なし）を使う。
    #[test]
    fn download_task_display_name_prefers_title() {
        let task = DownloadTask {
            id: "arXiv:2401.00001".into(),
            title: "SemMSA: A Paper".into(),
            downloaded: 1,
            total: None,
            paper_id: None,
            pdf_path: Some("/tmp/0005_x/paper.pdf".into()),
            status: DownloadStatus::Starting,
            remote_paper: None,
            failed_at: None,
        };
        assert_eq!(task.display_name(), "SemMSA: A Paper");
        let mut nameless = task;
        nameless.title = String::new();
        assert_eq!(nameless.display_name(), "paper");
    }

    fn remote_paper(index: usize) -> RemotePaper {
        RemotePaper {
            id: format!("https://arxiv.org/abs/2501.{index:05}"),
            title: format!("Paper {index}"),
            authors: Vec::new(),
            abstract_text: String::new(),
            published: Utc::now(),
            updated: Utc::now(),
            categories: Vec::new(),
            pdf_url: None,
            doi: None,
            journal_ref: None,
            citation_count: None,
        }
    }

    #[test]
    fn navigation_is_bounded_and_opens_selection() {
        let mut app = App::default();
        app.dispatch(Command::MoveUp);
        assert_eq!(app.sidebar_index, 0);
        app.dispatch(Command::MoveDown);
        app.dispatch(Command::Open);
        assert_eq!(app.page, Page::Project);
    }

    #[test]
    fn discovery_pages_cache_results_and_restore_each_page_view() {
        let mut discovery = DiscoveryState::default();
        discovery.set_results((0..101).map(remote_paper).collect());

        assert_eq!(discovery.page_count(), 3);
        assert_eq!(discovery.current_page_results().len(), 50);
        assert_eq!(discovery.current_page_results()[0].title, "Paper 0");

        discovery.selected = 17;
        discovery.scroll = 9;
        assert!(discovery.next_page());
        assert_eq!(discovery.page, 1);
        assert_eq!(discovery.selected, 0);
        assert_eq!(discovery.scroll, 0);
        assert_eq!(discovery.current_page_results()[0].title, "Paper 50");

        discovery.selected = 4;
        discovery.scroll = 2;
        assert!(discovery.previous_page());
        assert_eq!(
            (discovery.page, discovery.selected, discovery.scroll),
            (0, 17, 9)
        );
        assert!(discovery.next_page());
        assert_eq!(
            (discovery.page, discovery.selected, discovery.scroll),
            (1, 4, 2)
        );
        assert_eq!(discovery.results.len(), 101);
    }

    #[test]
    fn incremental_discovery_updates_keep_the_selected_paper() {
        let mut discovery = DiscoveryState::default();
        discovery.set_results((0..100).map(remote_paper).collect());
        discovery.selected = 23;

        let mut updated = (100..150).map(remote_paper).collect::<Vec<_>>();
        updated.push(remote_paper(23));
        discovery.update_results(updated);

        assert_eq!(
            discovery.selected_paper().map(|paper| paper.title.as_str()),
            Some("Paper 23")
        );
        assert_eq!(discovery.begin_search(), 1);
        assert!(discovery.results.is_empty());
    }

    #[test]
    fn test_arxiv_id_search_matching() {
        // Modern ID match (with and without version)
        assert!(App::matches_query(
            "1402.4146v2",
            "Title",
            "Authors",
            None,
            Some("http://arxiv.org/abs/1402.4146v2")
        ));
        assert!(App::matches_query(
            "1402.4146",
            "Title",
            "Authors",
            None,
            Some("http://arxiv.org/abs/1402.4146v2")
        ));
        assert!(App::matches_query(
            "1402.4146v2",
            "Title",
            "Authors",
            None,
            Some("1402.4146")
        ));
        assert!(App::matches_query(
            "1402.4146",
            "Title",
            "Authors",
            None,
            Some("1402.4146")
        ));
        assert!(App::matches_query(
            "1402.4146v1",
            "Title",
            "Authors",
            None,
            Some("1402.4146v3")
        ));

        // Legacy ID match (with and without version)
        assert!(App::matches_query(
            "hep-th/0309012v1",
            "Title",
            "Authors",
            None,
            Some("https://arxiv.org/abs/hep-th/0309012")
        ));
        assert!(App::matches_query(
            "hep-th/0309012",
            "Title",
            "Authors",
            None,
            Some("https://arxiv.org/abs/hep-th/0309012v3")
        ));
        assert!(App::matches_query(
            "math.GT/0309012",
            "Title",
            "Authors",
            None,
            Some("math.gt/0309012")
        ));

        // Case insensitivity
        assert!(App::matches_query(
            "HeP-Th/0309012",
            "Title",
            "Authors",
            None,
            Some("hep-th/0309012")
        ));

        // Non-matching
        assert!(!App::matches_query(
            "1402.4146",
            "Title",
            "Authors",
            None,
            Some("1502.4146")
        ));
        assert!(!App::matches_query(
            "hep-th/0309012",
            "Title",
            "Authors",
            None,
            Some("hep-th/0309013")
        ));
    }

    #[test]
    fn test_discover_filter_matches_query() {
        // Exact title match
        assert!(App::matches_query("rust", "Rust", "Authors", None, None));
        // Substring title match
        assert!(App::matches_query(
            "rust",
            "The Rust compiler",
            "Authors",
            None,
            None
        ));
        // Author match
        assert!(App::matches_query(
            "smith",
            "Title",
            "John Smith, Jane Doe",
            None,
            None
        ));
        // Case insensitivity
        assert!(App::matches_query(
            "RuSt",
            "rust compiler",
            "Authors",
            None,
            None
        ));
        // Non-matching
        assert!(!App::matches_query(
            "rust",
            "C++ compiler",
            "John Smith",
            None,
            None
        ));
    }

    #[test]
    fn missing_local_pdf_does_not_mark_remote_paper_as_downloaded() {
        let remote = remote_paper(42);
        let mut app = App::default();
        app.library.papers.push(LibraryPaper {
            id: 1,
            title: remote.title.clone(),
            authors: String::new(),
            doi: None,
            arxiv_id: Some(remote.id.clone()),
            pdf_path: Some("/definitely/missing/papr-paper.pdf".into()),
            file_size: None,
            reading_status: "unread".into(),
            is_favorite: false,
            abstract_text: None,
            journal: None,
            published_at: None,
            abstract_ja: None,
            abstract_ja_translated_at: None,
            updated_at: None,
            citation_count: None,
            is_retracted: None,
            retraction_checked_at: None,
        });

        assert!(app.downloaded_remote_paper(&remote).is_none());
    }

    #[test]
    fn test_page_from_config_str_and_set_page() {
        assert_eq!(Page::from_config_str("dashboard"), Some(Page::Dashboard));
        assert_eq!(Page::from_config_str("discover"), Some(Page::Discover));
        assert_eq!(Page::from_config_str("library"), Some(Page::Library));
        // 仕様 §2: 旧ページ名は現行ページへフォールバック
        assert_eq!(Page::from_config_str("reading_queue"), Some(Page::Library));
        assert_eq!(Page::from_config_str("projects"), Some(Page::Project));
        assert_eq!(
            Page::from_config_str("collections"),
            Some(Page::Collections)
        );
        assert_eq!(Page::from_config_str("groups"), Some(Page::Collections));
        assert_eq!(Page::from_config_str("tags"), Some(Page::Collections));
        assert_eq!(Page::from_config_str("bookmarks"), Some(Page::Library));
        assert_eq!(Page::from_config_str("authors"), Some(Page::Authors));
        assert_eq!(Page::from_config_str("notes"), Some(Page::Notes));
        assert_eq!(Page::from_config_str("downloads"), Some(Page::Library));
        assert_eq!(Page::from_config_str("history"), Some(Page::Dashboard));
        assert_eq!(Page::from_config_str("statistics"), Some(Page::Dashboard));
        // Settings 画面は廃止したため、"settings" は Dashboard に読み替える。
        assert_eq!(Page::from_config_str("settings"), Some(Page::Dashboard));
        assert_eq!(Page::from_config_str("unknown_page"), None);

        let mut app = App::default();
        assert_eq!(app.page, Page::Dashboard);
        assert_eq!(app.sidebar_index, 0);

        app.set_page(Page::Library);
        assert_eq!(app.page, Page::Library);
        assert_eq!(Page::ALL.get(app.sidebar_index), Some(&Page::Library));

        app.set_page(Page::Discover);
        assert_eq!(app.page, Page::Discover);
        assert_eq!(Page::ALL.get(app.sidebar_index), Some(&Page::Discover));
    }

    #[test]
    fn library_selection_clamps_at_both_ends_via_keyboard() {
        let mut app = App::default();
        for index in 0..5 {
            app.library.papers.push(LibraryPaper {
                id: index + 1,
                title: format!("Paper {index}"),
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
                updated_at: None,
                abstract_ja: None,
                abstract_ja_translated_at: None,
                citation_count: None,
                is_retracted: None,
                retraction_checked_at: None,
            });
        }
        app.page = Page::Library;
        app.content_focused = true;

        // 末尾でクランプ（WorkspaceSearch 等の副作用なし）
        for _ in 0..99 {
            app.move_workspace_selection_down();
        }
        assert_eq!(app.library.selected, 4);
        assert_eq!(app.mode, AppMode::Normal);

        // 上端でクランプはキーボードでは WorkspaceSearch 切替副作用付きのため、
        // 端に達した状態からさらに Down もクランプされることのみ確認する
        for _ in 0..5 {
            app.move_workspace_selection_down();
        }
        assert_eq!(app.library.selected, 4);
    }

    #[test]
    fn import_page_is_in_sidebar_and_resolves_config_str() {
        // Page::ALL の sidebar 順序を確認する（Settings は廃止済み）。
        assert_eq!(
            Page::ALL,
            [
                Page::Dashboard,
                Page::Project,
                Page::Discover,
                Page::Import,
                Page::Library,
                Page::Collections,
                Page::Authors,
                Page::Notes,
                Page::SearchSets,
                Page::Canvas,
            ]
        );
        assert_eq!(Page::from_config_str("project"), Some(Page::Project));
        assert_eq!(Page::from_config_str("projects"), Some(Page::Project));
        assert_eq!(Page::Project.title(), "Project");
        assert_eq!(Page::Project.config_str(), "project");
        // Project はワークスペース検索バー（/）非対応
        assert!(!Page::Project.supports_workspace_search());
        assert_eq!(Page::from_config_str("import"), Some(Page::Import));
        assert_eq!(Page::Import.title(), "Import");
        assert_eq!(Page::Import.config_str(), "import");
        // Import はワークスペース検索バー（/）非対応
        assert!(!Page::Import.supports_workspace_search());
    }

    #[test]
    fn import_state_filter_and_selection() {
        let mut import = ImportState::default();
        import.set_results(
            vec![
                remote_paper(1),
                RemotePaper {
                    id: "BibTeX:distinct".into(),
                    title: "Quantum Gravity".into(),
                    authors: vec!["Grace Hopper".into()],
                    abstract_text: String::new(),
                    published: Utc::now(),
                    updated: Utc::now(),
                    categories: Vec::new(),
                    pdf_url: None,
                    doi: None,
                    journal_ref: None,
                    citation_count: None,
                },
                remote_paper(3),
            ],
            vec![],
        );
        assert_eq!(import.status, ImportStatus::Ready);
        assert_eq!(import.visible_len(), 3);
        assert_eq!(
            import.selected_paper().map(|p| p.title.as_str()),
            Some("Paper 1")
        );

        // フィルタで絞り込む（title / author を対象）
        import.filter = "quantum".into();
        import.rebuild_filter();
        assert_eq!(import.visible_len(), 1);
        assert_eq!(
            import.selected_paper().map(|p| p.title.as_str()),
            Some("Quantum Gravity")
        );

        // 選択の移動とクランプ
        import.filter.clear();
        import.rebuild_filter();
        import.move_down();
        import.move_down();
        assert_eq!(import.selected, 2);
        import.move_down(); // 末尾でクランプ
        assert_eq!(import.selected, 2);
        import.move_up();
        assert_eq!(import.selected, 1);
        import.clamp_selection();
        assert_eq!(import.selected, 1);
    }

    #[test]
    fn import_state_keeps_scan_errors() {
        let mut import = ImportState::default();
        import.set_results(
            vec![remote_paper(1)],
            vec!["broken.bib: parse error".into()],
        );
        assert_eq!(import.scan_errors.len(), 1);
        assert!(import.scan_errors[0].starts_with("broken.bib"));
    }

    fn op(steps: &[&str], step: usize) -> RunningOperation {
        RunningOperation {
            key: "pipeline:1".into(),
            label: String::new(),
            title: "Attention Is All You Need".into(),
            steps: steps.iter().map(|s| (*s).to_string()).collect(),
            step,
            started_at: std::time::Instant::now(),
        }
    }

    #[test]
    fn pipeline_progress_counts_completed_steps() {
        let steps = ["PDF取得", "図抽出", "翻訳"];
        // PDF 取得済みで開始 → 1/3 完了。
        assert_eq!(op(&steps, 1).percent(), 33);
        assert_eq!(op(&steps, 1).step_label(), "図抽出");
        // 図抽出まで完了 → 2/3。
        assert_eq!(op(&steps, 2).percent(), 66);
        assert_eq!(op(&steps, 2).step_label(), "翻訳");
        // 全ステップ完了（片付け直前）→ 100%、ラベルは最後のステップのまま。
        assert_eq!(op(&steps, 3).percent(), 100);
        assert_eq!(op(&steps, 3).step_label(), "翻訳");
    }

    #[test]
    fn progress_bar_width_tracks_percentage() {
        assert_eq!(op(&["PDF取得", "図抽出"], 0).progress_bar(10), "[----------]");
        assert_eq!(op(&["PDF取得", "図抽出"], 1).progress_bar(10), "[#####-----]");
        assert_eq!(op(&["PDF取得", "図抽出"], 2).progress_bar(10), "[##########]");
    }

    #[test]
    fn empty_step_list_never_divides_by_zero() {
        let empty = op(&[], 0);
        assert_eq!(empty.percent(), 0);
        assert_eq!(empty.step_label(), "処理中");
    }

    #[test]
    fn tasks_screen_toggles_like_help_and_returns_to_previous_mode() {
        let mut app = App {
            mode: AppMode::PaperDetail,
            ..App::default()
        };
        app.dispatch(Command::ToggleTasks);
        assert_eq!(app.mode, AppMode::Tasks);
        app.dispatch(Command::ToggleTasks);
        assert_eq!(app.mode, AppMode::PaperDetail);
    }

}
