//! 引用文献の行と Library の照合、および取り込み用の [`RemotePaper`] 作り。
//!
//! 引用 TUI（緑色の表示）と Web（`sai-moon-paper phone citations`）は同じ決まりで
//! 「この引用は Library にあるか」を判定する。判定はここ 1 か所に置く。

use crate::models::{LibraryMatchEntry, RemotePaper};

use super::db::RefRow;

/// 引用の行が Library にあるかを判定する。
///
/// 決まり（引用 TUI の `in_library` と同じ）:
/// 1. 対象は「PDF が実在する論文」だけ
/// 2. `s2_paper_id` が一致
/// 3. 続くのは DOI が一致（大文字小文字を無視）
/// 4. 題名が一致（前後の空白を取って小文字にして比べる）
///
/// 最初に見つかった 1 件を返す（無ければ `None`）。
#[must_use]
pub fn library_match<'a>(
    entries: &'a [LibraryMatchEntry],
    s2_paper_id: Option<&str>,
    doi: Option<&str>,
    title: Option<&str>,
) -> Option<&'a LibraryMatchEntry> {
    let ref_title = title
        .map(str::trim)
        .filter(|title| !title.is_empty())
        .map(|title| title.to_lowercase());
    let ref_doi = doi.map(str::trim).filter(|doi| !doi.is_empty());
    entries.iter().find(|entry| {
        // 判定の決め手は「ローカル PDF が実際に存在する論文」。
        if !entry
            .pdf_path
            .as_deref()
            .is_some_and(|path| std::path::Path::new(path).is_file())
        {
            return false;
        }
        if let (Some(entry_id), Some(row_id)) = (
            entry.s2_paper_id.as_deref().map(str::trim),
            s2_paper_id.map(str::trim),
        )
            && entry_id == row_id
        {
            return true;
        }
        if let (Some(entry_doi), Some(row_doi)) = (entry.doi.as_deref(), ref_doi)
            && entry_doi.trim().eq_ignore_ascii_case(row_doi)
        {
            return true;
        }
        match (ref_title.as_deref(), entry.title.trim().to_lowercase()) {
            (Some(row_title), entry_title) => entry_title == row_title,
            _ => false,
        }
    })
}

/// 引用の行から取り込み用の [`RemotePaper`] を作る。
///
/// ID は `s2_paper_id` → `DOI:<doi>` の順。無ければ `None`（取り込めない）。
/// 題名は `s2_title` → `title`、年は `year` → `s2_year`（無ければ 2000 年 1 月 1 日）。
/// 著者は `first_author` だけ。ジャーナルは `journal`、DOI は `doi`。
#[must_use]
pub fn ref_remote_paper(row: &RefRow) -> Option<RemotePaper> {
    let id = row
        .s2_paper_id
        .as_deref()
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(str::to_owned)
        .or_else(|| {
            row.doi
                .as_deref()
                .map(str::trim)
                .filter(|doi| !doi.is_empty())
                .map(|doi| format!("DOI:{doi}"))
        })?;
    let title = row
        .s2_title
        .clone()
        .or_else(|| row.title.clone())
        .map(|title| title.trim().to_string())
        .filter(|title| !title.is_empty())?;
    let year = i32::try_from(row.year.or(row.s2_year).unwrap_or(2000)).unwrap_or(2000);
    let published = chrono::NaiveDate::from_ymd_opt(year, 1, 1)
        .unwrap_or_default()
        .and_hms_opt(0, 0, 0)
        .unwrap_or_default()
        .and_utc();
    Some(RemotePaper {
        id,
        title,
        authors: row
            .first_author
            .clone()
            .map(|author| vec![author])
            .unwrap_or_default(),
        abstract_text: String::new(),
        published,
        updated: chrono::Utc::now(),
        categories: Vec::new(),
        pdf_url: None,
        doi: row.doi.clone(),
        journal_ref: row.journal.clone(),
        citation_count: None,
    })
}

/// 引用の行の論文のサイト（DOI → S2 の 40 桁の ID → 無ければ `None`）。
#[must_use]
pub fn ref_site_url(row: &RefRow) -> Option<String> {
    if let Some(doi) = row
        .doi
        .as_deref()
        .map(str::trim)
        .filter(|doi| !doi.is_empty())
    {
        return Some(format!("https://doi.org/{doi}"));
    }
    if let Some(s2) = row
        .s2_paper_id
        .as_deref()
        .map(str::trim)
        .filter(|id| super::is_s2_40hex(id))
    {
        return Some(format!("https://semanticscholar.org/paper/{s2}"));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::{library_match, ref_remote_paper, ref_site_url};
    use crate::models::{LibraryMatchEntry, RemotePaper};

    /// 一時フォルダに置いてある PDF の絶対パス（実在する論文だけ判定に使う）。
    fn real_pdf_path() -> (std::path::PathBuf, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!(
            "papr-citation-library-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("paper.pdf");
        std::fs::write(&file, b"%PDF-1.7").unwrap();
        (dir, file)
    }

    /// 照合用の行（`pdf_path` は実在するパス）。
    fn entry(
        id: i64,
        arxiv_id: Option<&str>,
        doi: Option<&str>,
        s2_paper_id: Option<&str>,
        title: &str,
        pdf: Option<&std::path::Path>,
    ) -> LibraryMatchEntry {
        LibraryMatchEntry {
            id,
            arxiv_id: arxiv_id.map(str::to_owned),
            doi: doi.map(str::to_owned),
            s2_paper_id: s2_paper_id.map(str::to_owned),
            title: title.to_owned(),
            pdf_path: pdf.map(|path| path.to_string_lossy().into_owned()),
        }
    }

    /// PDF が実在する論文だけ対象になる（`s2_paper_id` が一致していても PDF が無ければ外す）。
    #[test]
    fn library_match_skips_papers_without_a_real_pdf() {
        let (dir, pdf) = real_pdf_path();
        let entries = vec![entry(1, None, None, Some("abc"), "T", None)];
        assert!(library_match(&entries, Some("abc"), None, None).is_none());

        let entries = vec![entry(1, None, None, Some("abc"), "T", Some(&pdf))];
        assert_eq!(
            library_match(&entries, Some("abc"), None, None).map(|e| e.id),
            Some(1)
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 決まりは S2 ID → DOI（大文字小文字を無視）→ 題名（前後の空白と大文字小文字を無視）。
    #[test]
    fn library_match_follows_s2_then_doi_then_title() {
        let (dir, pdf) = real_pdf_path();
        let entries = vec![
            entry(1, None, Some("10.1/ABC"), None, "First", Some(&pdf)),
            entry(2, None, None, None, "Second", Some(&pdf)),
        ];
        // S2 の ID
        assert_eq!(
            library_match(&entries, Some("s2-id"), None, None)
                .map(|e| e.id),
            None
        );
        // DOI は大文字小文字を無視
        assert_eq!(
            library_match(&entries, None, Some("10.1/abc"), None).map(|e| e.id),
            Some(1)
        );
        // 題名は前後の空白と大文字小文字を無視
        assert_eq!(
            library_match(&entries, None, None, Some("  second ")).map(|e| e.id),
            Some(2)
        );
        // どれにも当たらなければ無い
        assert!(library_match(&entries, None, None, Some("Third")).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 取り込み用の論文は S2 の ID を優先し、無ければ DOI を使う。
    #[test]
    fn ref_remote_paper_prefers_s2_id_then_doi() {
        use crate::citation::db::RefRow;
        use chrono::Datelike as _;
        let row = RefRow {
            ref_no: 1,
            title: Some("Docling Title".to_owned()),
            s2_title: Some("S2 Title".to_owned()),
            first_author: Some("Vaswani".to_owned()),
            year: Some(2017),
            s2_year: Some(2018),
            journal: Some("NeurIPS".to_owned()),
            doi: Some("10.1/x".to_owned()),
            s2_paper_id: Some("0123456789abcdef0123456789abcdef01234567".to_owned()),
            ..RefRow::default()
        };
        let paper: RemotePaper = ref_remote_paper(&row).expect("取り込める");
        assert_eq!(paper.id, "0123456789abcdef0123456789abcdef01234567");
        assert_eq!(paper.title, "S2 Title");
        assert_eq!(paper.authors, vec!["Vaswani".to_owned()]);
        assert_eq!(paper.journal_ref.as_deref(), Some("NeurIPS"));
        assert_eq!(paper.doi.as_deref(), Some("10.1/x"));
        assert_eq!(paper.published.year(), 2017);

        // S2 の ID も出版年も無ければ、docling の題名と年を使う
        let without_s2 = RefRow {
            s2_paper_id: None,
            s2_title: None,
            s2_year: None,
            ..row.clone()
        };
        let paper = ref_remote_paper(&without_s2).expect("DOI があれば取り込める");
        assert_eq!(paper.id, "DOI:10.1/x");
        assert_eq!(paper.title, "Docling Title");
        assert_eq!(paper.published.year(), 2017);

        // 年も無ければ 2000 年の 1 月 1 日。
        let no_year = RefRow {
            year: None,
            ..without_s2
        };
        assert_eq!(
            ref_remote_paper(&no_year).expect("取り込める").published.year(),
            2000
        );
    }

    /// ID も題名も無ければ取り込めない。
    #[test]
    fn ref_remote_paper_needs_an_id_and_a_title() {
        use crate::citation::db::RefRow;
        assert!(ref_remote_paper(&RefRow::default()).is_none());
        let no_title = RefRow {
            s2_paper_id: Some("abc".to_owned()),
            ..RefRow::default()
        };
        assert!(ref_remote_paper(&no_title).is_none());
        let no_id = RefRow {
            title: Some("Title".to_owned()),
            ..RefRow::default()
        };
        assert!(ref_remote_paper(&no_id).is_none());
    }

    /// サイトの並びは DOI → S2 の 40 桁の ID → 無し。
    #[test]
    fn ref_site_url_prefers_doi_then_s2() {
        use crate::citation::db::RefRow;
        let with_doi = RefRow {
            doi: Some("10.1/x".to_owned()),
            s2_paper_id: Some("0123456789abcdef0123456789abcdef01234567".to_owned()),
            ..RefRow::default()
        };
        assert_eq!(
            ref_site_url(&with_doi).as_deref(),
            Some("https://doi.org/10.1/x")
        );
        let s2_only = RefRow {
            s2_paper_id: Some("0123456789abcdef0123456789abcdef01234567".to_owned()),
            ..RefRow::default()
        };
        assert_eq!(
            ref_site_url(&s2_only).as_deref(),
            Some("https://semanticscholar.org/paper/0123456789abcdef0123456789abcdef01234567")
        );
        // 40 桁でない S2 の ID は無し
        let short_s2 = RefRow {
            s2_paper_id: Some("abc".to_owned()),
            ..RefRow::default()
        };
        assert!(ref_site_url(&short_s2).is_none());
    }
}
