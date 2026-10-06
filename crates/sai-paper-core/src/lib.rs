//! Core domain and infrastructure for the `papr` research workspace.

pub mod abstracts;
pub mod activity;
pub mod api;
pub mod attention;
pub mod bibtex;
pub mod brief;
pub mod chunking;
pub mod citation;
pub mod citation_check;
pub mod config;
pub mod database;
pub mod downloads;
pub mod dsr_check;
pub mod embeddings;
pub mod evidence;
pub mod figures;
pub mod find_log;
pub mod handoff;
pub mod jobs;
pub mod journal;
pub mod library;
pub mod library_tree;
pub mod manual_import;
pub mod models;
pub mod paths;
pub mod pipeline;
pub mod podcast_client;
pub mod progress;
pub mod project_files;
pub mod rag_eval;
pub mod reports;
pub mod retrieval;
pub mod rt;
pub mod runs;
pub mod search_log;
pub mod search_sets;
pub mod sections;
pub mod textcheck;
pub mod translation;
pub mod understanding;

pub use abstracts::{clean_crossref_abstract, flatten_abstract_ja, sanitize_abstract};
pub use api::arxiv::{ArxivClient, ArxivRetryPolicy, RankedCandidatePageRequest};
pub use api::biorxiv::BiorxivClient;
pub use api::openalex::OpenAlexClient;
pub use api::pubmed::PubmedClient;
pub use api::s2::{S2Client, S2Error, S2PaperDetail};
pub use api::{
    PaperSource, PaperSourceError, PaperSourceResolver, SearchQuery, dedupe, dedupe_key,
};
pub use attention::{AttentionError, AttentionItem, AttentionKind};
pub use chunking::{BuildReport, IndexStatus, Scope, Strategy};
pub use citation::{ExtractError, ExtractOutput, ensure_extract, is_s2_40hex, normalize_paper_id};
pub use citation_check::{
    CheckOptions, CheckSummary, ResolveOutcome, Resolver, check_report,
};
pub use config::{BriefConfig, Config, EvidenceConfig, Paths, ProgressConfig, ReadNextWeights};
pub use database::Database;
pub use downloads::{DownloadEvent, DownloadManager};
pub use embeddings::{EmbedClient, EmbeddingError, from_blob, register_model, to_blob};
pub use evidence::{
    ChatClient, ChatCompletionsClient, ChatReply, EvidenceError, EvidenceHit, EvidenceResult,
    SearchOutcome, find_maybe_evidence, find_with_evidence,
};
pub use figures::{
    FigureError, FigureExtractRow, FigureRow, FigureState, extract_figures, figure_state_for_paper,
    get_figure_extract, get_figure_extract_for, get_figures, get_figures_for,
};
pub use handoff::{
    BlockInfo, HandoffError, HandoffRequest, HandoffResult, TodoCard, create_handoff_with,
    list_blocks, list_todo_cards,
};
pub use library::{CollectionDirectory, ImportedPdf, LibraryIndexer};
pub use manual_import::{
    ManualImportError, ManualImported, ManualTarget, downloads_dir, import_pdf, recent_pdfs,
};
pub use models::{
    ActivityItem, BookmarkSummary, CollectionSummary, LibraryMatchEntry, LibraryPaper,
    RemotePaper,
};
pub use paths::{
    PaperRoots, canonicalize_path, move_pdf_file, paper_figure_path, paper_figures_dir,
    paper_file_names, paper_file_path, paper_folder, paper_roots,
    sanitize_download_filename_component, translated_pdf_path,
};
pub use reports::{
    IssueCounts, NewReport, Report, ReportError, ReportIssues, ReportSource, create_report,
    get_report, issue_counts, list_issues, list_reports, read_report_markdown, report_sources,
    resolve_md_path,
};
pub use progress::{PaperProgress, Progress, ProgressError, ReadNext, ReportProgress};
pub use retrieval::{
    Hit, HybridSearchOptions, RetrievalError, VectorSearchOptions, WordSearchOptions,
    hybrid_search, neighbors, vector_search, word_search,
};
pub use search_sets::{SearchSet, SearchSetError, SearchSetItem};
