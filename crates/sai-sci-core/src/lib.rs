//! 研究方針の木・研究の記録・エージェントの報告の検査。
//!
//! 論文とノートの処理は `sai_paper_core` にある。このクレートは sai-paper-core に依存する。

pub mod attention;
pub mod bins;
pub mod brief;
#[cfg(test)]
pub(crate) mod fake_link;
pub mod link;
pub mod citation_check;
pub mod dsr_check;
pub mod handoff;
pub mod journal;
pub mod progress;
pub mod project_files;
pub mod reports;
pub mod rt;
pub mod runs;
pub mod understanding;

pub use attention::{AttentionError, AttentionItem, AttentionKind};
pub use citation_check::{CheckOptions, CheckSummary, ResolveOutcome, Resolver, check_report};
pub use handoff::{
    BlockInfo, HandoffError, HandoffRequest, HandoffResult, TodoCard, create_handoff_with,
    list_blocks, list_todo_cards,
};
pub use progress::{PaperProgress, Progress, ProgressError, ReadNext, ReportProgress};
pub use reports::{
    IssueCounts, NewReport, Report, ReportError, ReportIssues, ReportSource, create_report,
    get_report, issue_counts, list_issues, list_reports, read_report_markdown, report_sources,
    resolve_md_path,
};
