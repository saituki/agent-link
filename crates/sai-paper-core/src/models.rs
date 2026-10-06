//! Shared domain models.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// A paper returned by a remote discovery provider.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemotePaper {
    /// Provider-specific stable identifier.
    pub id: String,
    /// Canonical paper title.
    pub title: String,
    /// Author names in publication order.
    pub authors: Vec<String>,
    /// Full abstract when provided.
    pub abstract_text: String,
    /// Original publication timestamp.
    pub published: DateTime<Utc>,
    /// Most recent revision timestamp.
    pub updated: DateTime<Utc>,
    /// Subject categories assigned by the provider.
    pub categories: Vec<String>,
    /// Direct PDF location.
    pub pdf_url: Option<String>,
    /// DOI when linked by the provider.
    pub doi: Option<String>,
    /// Journal reference when available.
    pub journal_ref: Option<String>,
    /// 被引用数（S2 citationCount）。取得できないプロバイダでは None（表示は '-'）。
    pub citation_count: Option<i32>,
}

impl RemotePaper {
    /// Return a compact comma-separated author line.
    #[must_use]
    pub fn author_line(&self) -> String {
        self.authors.join(", ")
    }
}

/// A paper stored in the local catalog.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Paper {
    /// Stable database identifier.
    pub id: i64,
    /// Canonical title.
    pub title: String,
    /// Optional digital object identifier.
    pub doi: Option<String>,
    /// When the item was added to the library.
    pub created_at: DateTime<Utc>,
}

/// A paper row shown in the local library.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LibraryPaper {
    /// Stable database identifier.
    pub id: i64,
    /// Bibliographic title.
    pub title: String,
    /// Comma-separated authors when known.
    pub authors: String,
    /// Digital object identifier when known.
    pub doi: Option<String>,
    /// arXiv ID.
    pub arxiv_id: Option<String>,
    /// Local PDF path when downloaded or imported.
    pub pdf_path: Option<String>,
    /// PDF size in bytes.
    pub file_size: Option<u64>,
    /// Current reading state.
    pub reading_status: String,
    /// Whether the paper is a favorite.
    pub is_favorite: bool,
    /// Abstract text when available.
    pub abstract_text: Option<String>,
    /// Journal or venue when available.
    pub journal: Option<String>,
    /// Published timestamp as stored string.
    pub published_at: Option<String>,
    /// Updated timestamp as stored string.
    pub updated_at: Option<String>,
    /// 日本語翻訳された abstract（google 等で生成、papers.abstract_ja にキャッシュ）。
    pub abstract_ja: Option<String>,
    /// 日本語訳の生成日時（ISO8601）。
    pub abstract_ja_translated_at: Option<String>,
    /// 被引用数（S2 citationCount）。未取得の場合は None（表示は '-'）。
    pub citation_count: Option<i64>,
    /// 撤回の状態（None = 未確認、Some(false) = 撤回されていない、Some(true) = 撤回）。
    pub is_retracted: Option<bool>,
    /// 撤回を確かめた時刻（ISO8601）。未確認は None。
    pub retraction_checked_at: Option<String>,
}

impl LibraryPaper {
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

/// Named paper collection with its current item count.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CollectionSummary {
    /// Stable collection identifier.
    pub id: i64,
    /// Unique display name.
    pub name: String,
    /// Number of assigned papers.
    pub paper_count: u64,
    /// Backing directory for filesystem collections.
    pub folder_path: Option<String>,
}

/// A bookmarked paper or position.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BookmarkSummary {
    /// Stable bookmark identifier.
    pub id: i64,
    /// Parent paper identifier.
    pub paper_id: i64,
    /// Bibliographic paper title.
    pub paper_title: String,
    /// Comma-separated paper authors when available.
    pub authors: String,
    /// Publication year when known.
    pub year: Option<String>,
    /// Journal or venue when known.
    pub journal: Option<String>,
    /// Digital object identifier when known.
    pub doi: Option<String>,
    /// Local PDF path used to open the bookmarked paper.
    pub pdf_path: String,
    /// Optional bookmarked PDF page.
    pub page: Option<u32>,
    /// Optional user label.
    pub label: Option<String>,
}

impl BookmarkSummary {
    /// Display the paper title, falling back to the PDF file name.
    #[must_use]
    pub fn display_name(&self) -> &str {
        let title = self
            .paper_title
            .strip_suffix(".pdf")
            .unwrap_or(&self.paper_title)
            .trim();
        if !title.is_empty() {
            return title;
        }
        std::path::Path::new(&self.pdf_path)
            .file_stem()
            .and_then(|name| name.to_str())
            .unwrap_or("")
    }
}

/// One human-readable research activity event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActivityItem {
    /// Event type stored in the activity log.
    pub kind: String,
    /// Related paper ID (for `enter` navigation from the RA list).
    pub paper_id: Option<i64>,
    /// Related paper title or event detail.
    pub label: String,
    /// UTC event timestamp.
    pub occurred_at: DateTime<Utc>,
    /// sai-radio の音声の名前（Radio の再生・質問の行だけ。再生画面への飛び先に使う）。
    pub radio_id: Option<String>,
}

/// Web（`phone_cite`・`phone_import`）での Library 照合に使う最小限の論文情報（R5）。
///
/// `pdf_path` は照合時に実ファイル存在を確認し、ローカル PDF が実在する
/// 論文のみ「Library 存在（緑色表示）」と判定する。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LibraryMatchEntry {
    /// Library 論文 ID（`papers.id`）。
    pub id: i64,
    /// arXiv ID.
    pub arxiv_id: Option<String>,
    /// DOI.
    pub doi: Option<String>,
    /// S2 paper ID（40hex）.
    pub s2_paper_id: Option<String>,
    /// 論文タイトル。
    pub title: String,
    /// ローカル PDF パス。
    pub pdf_path: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::{BookmarkSummary, LibraryPaper};

    fn library_paper(title: &str, pdf_path: Option<&str>) -> LibraryPaper {
        LibraryPaper {
            id: 1,
            title: title.into(),
            authors: String::new(),
            doi: None,
            arxiv_id: None,
            pdf_path: pdf_path.map(Into::into),
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
        }
    }

    fn bookmark(paper_title: &str, pdf_path: &str) -> BookmarkSummary {
        BookmarkSummary {
            id: 1,
            paper_id: 2,
            paper_title: paper_title.into(),
            authors: String::new(),
            year: None,
            journal: None,
            doi: None,
            pdf_path: pdf_path.into(),
            page: None,
            label: None,
        }
    }

    /// 論文の置き場を整理して PDF が `paper.pdf` になったあとでも題名を出す。
    #[test]
    fn display_name_prefers_title_over_pdf_file_stem() {
        let paper = library_paper("SemMSA: A Paper", Some("/tmp/0005_x/paper.pdf"));
        assert_eq!(paper.display_name(), "SemMSA: A Paper");
        let mark = bookmark("SemMSA: A Paper", "/tmp/0005_x/paper.pdf");
        assert_eq!(mark.display_name(), "SemMSA: A Paper");
    }

    /// 題名が空のときだけ PDF のファイル名（拡張子なし）を使う。
    #[test]
    fn display_name_falls_back_to_pdf_file_stem() {
        let paper = library_paper("", Some("/tmp/0005_x/paper.pdf"));
        assert_eq!(paper.display_name(), "paper");
        let paper = library_paper("   ", Some("/tmp/0005_x/paper.pdf"));
        assert_eq!(paper.display_name(), "paper");
        let mark = bookmark("", "/tmp/0005_x/paper.pdf");
        assert_eq!(mark.display_name(), "paper");
    }

    /// 題名に `.pdf` が付いていても拡張子は落とす。
    #[test]
    fn display_name_strips_pdf_suffix_from_title() {
        let paper = library_paper("SemMSA.pdf", None);
        assert_eq!(paper.display_name(), "SemMSA");
        let mark = bookmark("SemMSA.pdf", "");
        assert_eq!(mark.display_name(), "SemMSA");
    }
}
