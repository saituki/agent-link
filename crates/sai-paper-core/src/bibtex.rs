//! BibTeX（.bib）ファイルの読み込みと [`RemotePaper`] 変換。
//!
//! Import 画面用モジュール。`data/bibtex/` 配下の .bib ファイル（直下のみ）を
//! スキャンし、全エントリを Discover 画面と共通の [`RemotePaper`] 一覧へ変換する。
//!
//! 変換方針:
//! - `doi` あり → `id = "DOI:<doi>"`（`papers.doi` の UNIQUE 制約で重複検出）
//! - `url` が arXiv 形式 → `id = "ArXiv:<id>"`（`papers.arxiv_id` 列で重複検出）
//! - どちらも無し → `id = "BibTeX:<citation key>"`（arxiv_id 列は実質プロバイダ ID 兼用）
//! - `year` のみ（月日なし）→ `published = <year>-01-01 00:00 UTC`（S2 クライアントと同一規約）
//! - year 無し → `1970-01-01`（`RemotePaper.published` は非 NULL のための初期値）
//! - S2 出力の BibTeX には abstract が無いことが多いため、abstract は基本空欄

use std::path::{Path, PathBuf};

use biblatex::{Bibliography, DateValue, Entry, PermissiveType};
use chrono::{TimeZone, Utc};
use thiserror::Error;

use crate::models::RemotePaper;

/// year 無しエントリの代替発行日（`RemotePaper.published` は非 NULL のため）。
const EPOCH_FALLBACK_DATE: chrono::NaiveDate = chrono::NaiveDate::from_ymd_opt(1970, 1, 1).unwrap();

/// BibTeX 読み込み時のエラー。
#[derive(Debug, Error)]
pub enum BibtexError {
    /// 1 ファイルの解析に失敗した（他ファイルへの影響は無い）。
    #[error("{file}: {error}")]
    Parse {
        /// 解析に失敗した .bib ファイルのパス。
        file: PathBuf,
        /// `biblatex` クレート由来のエラーメッセージ。
        error: String,
    },
    /// ファイル読み込み・保存に失敗した。
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    /// 保存しようとした BibTeX を読めなかった（ファイルはまだ書かない）。
    #[error("BibTeX を読めませんでした: {error}")]
    ParseSource {
        /// `biblatex` クレート由来のエラーメッセージ。
        error: String,
    },
    /// ファイル名の規則を外れている。
    #[error("ファイル名は英数字・.・-・_ で .bib で終わる 80 文字までにしてください")]
    InvalidName,
    /// 同じ名前の .bib が既に BibTeX フォルダにある（上書きしない）。
    #[error("同じ名前の .bib があります")]
    Exists,
    /// 消す対象の .bib が無い。
    #[error(".bib が見つかりません")]
    NotFound,
    /// 解析できたが論文条目が 1 件も無い。
    #[error("BibTeX の項目がありません")]
    Empty,
}

/// 保存した .bib 1 件の保存先と項目数。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavedBib {
    /// 書き込んだ .bib のパス。
    pub path: PathBuf,
    /// 解析できた論文条目の数。
    pub count: usize,
}

/// BibTeX フォルダの .bib 1 件のまとめ。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BibFileInfo {
    /// ファイル名（フォルダ直下のものだけ）。
    pub name: String,
    /// 解析できた論文条目の数（解析に失敗したら 0）。
    pub count: usize,
    /// 解析の失敗（`<ファイル名>: <原因>`。無ければ `None`）。
    pub error: Option<String>,
}

/// ディレクトリ 1 回分のスキャン結果。
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct BibtexScan {
    /// 全 .bib ファイルから変換した論文一覧（パース失敗ファイルは除外済み）。
    pub papers: Vec<RemotePaper>,
    /// パースに失敗したファイルのメッセージ一覧（`<ファイル名>: <原因>`）。
    pub errors: Vec<String>,
}

/// `data/bibtex/` をスキャンし、全 .bib の全エントリを 1 つのリストへ変換する。
///
/// パースに失敗したファイルはスキップされ、エラーメッセージのみが
/// [`BibtexScan::errors`] に記録される（他ファイルは継続）。
/// ディレクトリが存在しない場合は空の結果を返す（エラーにはしない）。
///
/// # Errors
///
/// ディレクトリ走査自体の IO エラーを返す。
pub fn load_bibtex_dir_detailed(dir: &Path) -> Result<BibtexScan, BibtexError> {
    let mut scan = BibtexScan::default();
    if !dir.is_dir() {
        return Ok(scan);
    }

    // ファイル名順に読む（再スキャンで同じ順序を保つため）
    let mut bib_paths: Vec<PathBuf> = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if is_bib_path(&path) {
            bib_paths.push(path);
        }
    }
    bib_paths.sort();

    for path in bib_paths {
        match parse_bibtex_file(&path) {
            Ok(mut papers) => scan.papers.append(&mut papers),
            Err(err) => {
                let name = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .map_or_else(|| path.display().to_string(), str::to_owned);
                scan.errors.push(format!("{name}: {err}"));
            }
        }
    }

    scan.papers = dedupe_papers(scan.papers);
    Ok(scan)
}

/// 1 つの .bib ファイルを解析し、全エントリを [`RemotePaper`] へ変換する。
///
/// # Errors
///
/// ファイル読み込み失敗は [`BibtexError::Io`]、解析失敗は
/// [`BibtexError::Parse`] として返す（呼び出し側は 1 ファイル単位で
/// スキップ判断できる）。
pub fn parse_bibtex_file(path: &Path) -> Result<Vec<RemotePaper>, BibtexError> {
    let source = std::fs::read_to_string(path)?;
    papers_from_source(&source).map_err(|error| BibtexError::Parse {
        file: path.to_path_buf(),
        error,
    })
}

/// BibTeX の文字列を解析して [`RemotePaper`] 一覧へ変換する（`biblatex` のエラーは文字列で返す）。
fn papers_from_source(source: &str) -> Result<Vec<RemotePaper>, String> {
    let bibliography = Bibliography::parse(source).map_err(|error| error.to_string())?;
    Ok(bibliography
        .iter()
        .filter_map(entry_to_remote_paper)
        .collect())
}

/// 保存できる .bib のファイル名かどうか（英数字・`.`・`-`・`_` のみで 1〜80 文字 + `.bib`）。
///
/// `..` を含む名前や、先頭が `.` の名前は（フォルダの外に出る・隠しファイルになるので）必ず弾く。
/// `.bib` の大文字小文字は問わない。
#[must_use]
pub fn is_valid_bib_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    let Some(stem) = lower.strip_suffix(".bib") else {
        return false;
    };
    if stem.is_empty()
        || stem.len() > BIB_NAME_MAX_STEM_LEN
        || stem.contains("..")
        || stem.starts_with('.')
    {
        return false;
    }
    name.chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

/// `.bib` の名前のうち拡張子を除いた部分の最大長。
const BIB_NAME_MAX_STEM_LEN: usize = 80;

/// BibTeX フォルダの .bib を 1 件保存する。
///
/// 保存の前に本文を解析し、**読めない・項目が 0 件・同名が既にあるときは保存しない**
/// （iPhone から誤って既存の .bib を消さないため上書きしない）。フォルダが無ければ作る。
///
/// # Errors
///
/// 名前の規則違反は [`BibtexError::InvalidName`]、解析失敗は
/// [`BibtexError::ParseSource`]、0 件は [`BibtexError::Empty`]、同名は
/// [`BibtexError::Exists`]、ファイル書き込みは [`BibtexError::Io`]。
pub fn save_bib_file(dir: &Path, file_name: &str, content: &str) -> Result<SavedBib, BibtexError> {
    if !is_valid_bib_name(file_name) {
        return Err(BibtexError::InvalidName);
    }
    // 保存前の検査（壊れた .bib や空の .bib を残さない）
    let count = papers_from_source(content)
        .map_err(|error| BibtexError::ParseSource { error })?
        .len();
    if count == 0 {
        return Err(BibtexError::Empty);
    }
    std::fs::create_dir_all(dir)?;
    let path = dir.join(file_name);
    if path.exists() {
        return Err(BibtexError::Exists);
    }
    // 書きかけが中途半端に残らないよう、一時ファイルへ書いてから入れ替える。
    let temp_path = dir.join(format!("{file_name}.tmp"));
    std::fs::write(&temp_path, content)?;
    std::fs::rename(&temp_path, &path)?;
    Ok(SavedBib { path, count })
}

/// BibTeX フォルダの .bib を 1 件消す。
///
/// # Errors
///
/// 名前の規則違反は [`BibtexError::InvalidName`]、対象が無ければ
/// [`BibtexError::NotFound`]。
pub fn remove_bib_file(dir: &Path, file_name: &str) -> Result<(), BibtexError> {
    if !is_valid_bib_name(file_name) {
        return Err(BibtexError::InvalidName);
    }
    let path = dir.join(file_name);
    if !path.is_file() {
        return Err(BibtexError::NotFound);
    }
    std::fs::remove_file(&path)?;
    Ok(())
}

/// BibTeX フォルダの .bib ごとの件数と解析の失敗をファイル名順で返す。
///
/// フォルダが無いときは空のリストを返す（エラーにはしない）。
///
/// # Errors
///
/// ディレクトリ走査自体の IO エラーを返す。
pub fn list_bib_files(dir: &Path) -> Result<Vec<BibFileInfo>, BibtexError> {
    if !dir.is_dir() {
        return Ok(Vec::new());
    }
    let mut bib_paths: Vec<PathBuf> = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        if is_bib_path(&path) {
            bib_paths.push(path);
        }
    }
    bib_paths.sort();

    Ok(bib_paths
        .iter()
        .map(|path| {
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .map_or_else(|| path.display().to_string(), str::to_owned);
            match parse_bibtex_file(path) {
                Ok(papers) => BibFileInfo {
                    name,
                    count: papers.len(),
                    error: None,
                },
                Err(err) => {
                    let error = format!("{name}: {err}");
                    BibFileInfo {
                        name,
                        count: 0,
                        error: Some(error),
                    }
                },
            }
        })
        .collect())
}

/// 直下の `.bib` ファイルかどうか（大文字小文字を問わない）。
fn is_bib_path(path: &Path) -> bool {
    path.is_file()
        && path
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("bib"))
}

/// 同一ディレクトリ内の重複エントリを除去する（`id` が同一のものは先頭のみ残す）。
fn dedupe_papers(papers: Vec<RemotePaper>) -> Vec<RemotePaper> {
    let mut seen = std::collections::HashSet::new();
    papers
        .into_iter()
        .filter(|paper| seen.insert(paper.id.to_ascii_lowercase()))
        .collect()
}

/// url から arXiv ID を抽出する（`https://arxiv.org/abs/1706.03762` → `1706.03762`）。
fn arxiv_id_from_url(url: &str) -> Option<String> {
    let trimmed = url.trim();
    let marker = if let Some(pos) = trimmed.find("/abs/") {
        &trimmed[pos + 5..]
    } else if let Some(pos) = trimmed.find("/pdf/") {
        let after = &trimmed[pos + 5..];
        after.strip_suffix(".pdf").unwrap_or(after)
    } else {
        return None;
    };
    let id = marker
        .split(['?', '#'])
        .next()?
        .trim_end_matches('/')
        .trim();
    if id.is_empty() {
        return None;
    }
    // バージョン表記（v2 など）は除いた ID に正規化する
    if let Some(pos) = id.rfind('v')
        && id[pos + 1..].chars().all(|c| c.is_ascii_digit())
        && pos + 1 < id.len()
    {
        return Some(id[..pos].to_owned());
    }
    Some(id.to_owned())
}

/// biblatex のエントリ 1 件を [`RemotePaper`] へ変換する。
///
/// タイトルが取得できない（必須フィールドが欠落した）エントリは変換しない。
fn entry_to_remote_paper(entry: &Entry) -> Option<RemotePaper> {
    let title = field_string(entry, "title")
        .or_else(|| field_string(entry, "booktitle"))?
        .trim()
        .to_owned();
    let title = if title.is_empty() { return None } else { title };

    // 著者（"and" 区切りは biblatex が Person 列へ分割済み）
    let authors: Vec<String> = entry
        .author()
        .unwrap_or_default()
        .iter()
        .map(person_display_name)
        .filter(|name| !name.is_empty())
        .collect();

    let doi = field_string(entry, "doi").map(|s| s.trim().to_owned());
    let doi = doi.filter(|s| !s.is_empty());
    let url = field_string(entry, "url").map(|s| s.trim().to_owned());
    let arxiv_id = url.as_deref().and_then(arxiv_id_from_url);

    // `id` は `ensure_remote_paper` の arxiv_id 列相当として upsert 判断に使われる
    // プロバイダ ID なので、DOI / arXiv / citation key の優先度で生成する
    let id = if let Some(doi) = &doi {
        format!("DOI:{doi}")
    } else if let Some(arxiv_id) = &arxiv_id {
        format!("ArXiv:{arxiv_id}")
    } else {
        format!("BibTeX:{}", entry.key)
    };

    // journal / booktitle を表示用の journal 参照へ
    let journal_ref = field_string(entry, "journal")
        .or_else(|| field_string(entry, "journaltitle"))
        .or_else(|| field_string(entry, "booktitle"))
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty());

    let published = published_date(entry);
    let updated = published;
    // S2 エクスポートには abstract が無いことが多い（あれば使用）
    let abstract_text = field_string(entry, "abstract").unwrap_or_default();
    // keywords をカテゴリとして流用（無ければ空）
    let categories: Vec<String> = field_string(entry, "keywords")
        .map(|s| {
            s.split(',')
                .map(str::trim)
                .filter(|k| !k.is_empty())
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();

    Some(RemotePaper {
        id,
        title,
        authors,
        abstract_text,
        published,
        updated,
        categories,
        pdf_url: None,
        doi,
        journal_ref,
        citation_count: None,
    })
}

/// 文字列フィールドを安全に取り出す（欠落・型不正時は `None`）。
fn field_string(entry: &Entry, field: &str) -> Option<String> {
    entry.get_as::<String>(field).ok().filter(|s| !s.is_empty())
}

/// `Person` を "Given Name Family Name" 形式の表示名へ変換する。
fn person_display_name(person: &biblatex::Person) -> String {
    let mut parts: Vec<String> = Vec::new();
    for value in [
        person.given_name.trim(),
        person.prefix.trim(),
        person.name.trim(),
    ] {
        if !value.is_empty() {
            parts.push(value.to_owned());
        }
    }
    let mut name = parts.join(" ");
    let suffix = person.suffix.trim();
    if !suffix.is_empty() {
        name.push(' ');
        name.push_str(suffix);
    }
    name
}

/// `date` / `year` フィールドから発行日時を組み立てる。
///
/// - `date = {2017-06-12}` → その日時の 0 時 UTC
/// - `year = {2017}` のみ → `2017-01-01 00:00 UTC`（S2 クライアントと同一規約）
/// - どちらも無し → `1970-01-01`（初期値。`RemotePaper.published` は非 NULL）
fn published_date(entry: &Entry) -> chrono::DateTime<Utc> {
    let to_utc = |year: i32, month: Option<u8>, day: Option<u8>| {
        Utc.with_ymd_and_hms(
            year,
            i64::from(month.unwrap_or(1).max(1)) as u32,
            i64::from(day.unwrap_or(1).max(1)) as u32,
            0,
            0,
            0,
        )
        .single()
        .unwrap_or_else(|| {
            Utc.from_utc_datetime(&EPOCH_FALLBACK_DATE.and_hms_opt(0, 0, 0).unwrap())
        })
    };

    if let Ok(PermissiveType::Typed(date)) = entry.date() {
        let datetime = match date.value {
            DateValue::At(datetime) => Some(datetime),
            DateValue::After(datetime)
            | DateValue::Before(datetime)
            | DateValue::Between(datetime, _) => Some(datetime),
        };
        if let Some(datetime) = datetime {
            // biblatex の Datetime は month / day が 0 始まりのため 1 始まりに補正する
            return to_utc(
                datetime.year,
                datetime.month.map(|m| m.saturating_add(1)),
                datetime.day.map(|d| d.saturating_add(1)),
            );
        }
    }

    // date フィールドが Chunks（解析できなかった日付文字列）のケース
    if let Ok(PermissiveType::Chunks(chunks)) = entry.date()
        && let Some(year) = first_year_in(
            &chunks
                .iter()
                .map(|c| c.v.get().to_string())
                .collect::<String>(),
        )
    {
        return to_utc(year, None, None);
    }

    // date フィールド自体が無い / 解析不能 → year フィールドから抽出
    if let Some(year) = field_string(entry, "year").and_then(|s| first_year_in(&s)) {
        return to_utc(year, None, None);
    }

    Utc.from_utc_datetime(&EPOCH_FALLBACK_DATE.and_hms_opt(0, 0, 0).unwrap())
}

/// 文字列から最初の 4 桁（または 1〜2 桁以上）の年らしい数値を抽出する。
fn first_year_in(text: &str) -> Option<i32> {
    let mut digits = String::new();
    for c in text.chars() {
        if c.is_ascii_digit() {
            digits.push(c);
        } else if !digits.is_empty() {
            break;
        }
    }
    if digits.is_empty() {
        return None;
    }
    digits
        .parse::<i32>()
        .ok()
        .filter(|y| (1000..=9999).contains(y))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ソース文字列を直接解析するテストヘルパー。
    fn parse_bibtex_source(source: &str) -> Vec<RemotePaper> {
        Bibliography::parse(source)
            .expect("パース成功")
            .iter()
            .filter_map(entry_to_remote_paper)
            .collect()
    }

    #[test]
    fn converts_s2_style_bibtex() {
        let source = r#"
@article{Vaswani2017AttentionIA,
  title   = {Attention is all you Need},
  author  = {Ashish Vaswani and Noam Shazeer},
  journal = {ArXiv},
  year    = {2017},
  volume  = {abs/1706.03762},
  url     = {https://arxiv.org/abs/1706.03762}
}
"#;
        let papers = parse_bibtex_source(source);
        assert_eq!(papers.len(), 1);
        let paper = &papers[0];
        assert_eq!(paper.id, "ArXiv:1706.03762");
        assert_eq!(paper.title, "Attention is all you Need");
        assert_eq!(paper.authors, vec!["Ashish Vaswani", "Noam Shazeer"]);
        assert_eq!(paper.published.format("%Y-%m-%d").to_string(), "2017-01-01");
        assert_eq!(paper.journal_ref.as_deref(), Some("ArXiv"));
        assert_eq!(paper.doi, None);
        assert_eq!(paper.abstract_text, "");
    }

    #[test]
    fn converts_doi_style_bibtex() {
        let source = r#"
@article{Lee2024SomePaper,
  title   = {Some Paper},
  author  = {Alice Lee and Bob Smith},
  journal = {Nature Communications},
  year    = {2024},
  volume  = {15},
  pages   = {1234},
  doi     = {10.1038/s41586-024-00000-x}
}
"#;
        let papers = parse_bibtex_source(source);
        assert_eq!(papers.len(), 1);
        let paper = &papers[0];
        assert_eq!(paper.id, "DOI:10.1038/s41586-024-00000-x");
        assert_eq!(paper.doi.as_deref(), Some("10.1038/s41586-024-00000-x"));
        assert_eq!(paper.published.format("%Y-%m-%d").to_string(), "2024-01-01");
    }

    #[test]
    fn falls_back_to_citation_key_id() {
        let source = r#"
@inproceedings{MyKey2020,
  title = {A Paper Without DOI or ArXiv},
  author = {Carol Dane},
  year  = {2020}
}
"#;
        let papers = parse_bibtex_source(source);
        assert_eq!(papers.len(), 1);
        assert_eq!(papers[0].id, "BibTeX:MyKey2020");
    }

    #[test]
    fn missing_year_uses_epoch_fallback() {
        let source = r#"
@misc{NoYear,
  title = {Undated Paper}
}
"#;
        let papers = parse_bibtex_source(source);
        assert_eq!(papers.len(), 1);
        assert_eq!(
            papers[0].published.format("%Y-%m-%d").to_string(),
            "1970-01-01"
        );
    }

    #[test]
    fn full_date_field_is_respected() {
        let source = r#"
@article{WithDate,
  title = {Dated Paper},
  date  = {2021-03-05}
}
"#;
        let papers = parse_bibtex_source(source);
        assert_eq!(
            papers[0].published.format("%Y-%m-%d").to_string(),
            "2021-03-05"
        );
    }

    #[test]
    fn entries_without_title_are_skipped() {
        let source = r#"
@misc{NoTitle,
  author = {Nobody},
  year   = {2020}
}
"#;
        let papers = parse_bibtex_source(source);
        assert!(papers.is_empty());
    }

    #[test]
    fn tex_braces_are_flattened() {
        let source = r#"
@article{NestedTitle,
  title = {{Nested {Inner} Title}},
  year  = {2019}
}
"#;
        let papers = parse_bibtex_source(source);
        assert_eq!(papers[0].title, "Nested Inner Title");
    }

    #[test]
    fn scan_skips_broken_files_and_continues() {
        let dir =
            std::env::temp_dir().join(format!("sai-paper-bibtex-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("good.bib"),
            "@article{A, title = {Good}, year = {2020}}",
        )
        .unwrap();
        std::fs::write(dir.join("broken.bib"), "@article{Broken, title = {").unwrap();
        std::fs::write(dir.join("ignored.txt"), "not a bib").unwrap();

        let scan = load_bibtex_dir_detailed(&dir).unwrap();
        assert_eq!(scan.papers.len(), 1);
        assert_eq!(scan.papers[0].title, "Good");
        assert_eq!(scan.errors.len(), 1);
        assert!(scan.errors[0].contains("broken.bib"));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn scan_returns_empty_for_missing_dir() {
        let scan = load_bibtex_dir_detailed(Path::new("/nonexistent-dir-for-sai-paper")).unwrap();
        assert!(scan.papers.is_empty());
        assert!(scan.errors.is_empty());
    }

    #[test]
    fn arxiv_url_detection() {
        assert_eq!(
            arxiv_id_from_url("https://arxiv.org/abs/1706.03762"),
            Some("1706.03762".to_owned())
        );
        assert_eq!(
            arxiv_id_from_url("https://arxiv.org/pdf/1706.03762v2.pdf"),
            Some("1706.03762".to_owned())
        );
        assert_eq!(arxiv_id_from_url("https://example.com/paper"), None);
    }

    #[test]
    fn duplicate_ids_are_deduped() {
        let papers = vec![remote("BibTeX:a"), remote("BibTeX:a"), remote("BibTeX:b")];
        let deduped = dedupe_papers(papers);
        assert_eq!(deduped.len(), 2);
    }

    /// テスト用の空フォルダ（テストごとに作り分ける）。
    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "sai-paper-bibtex-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    const SAMPLE_BIB: &str = r#"
@article{Vaswani2017AttentionIA,
  title  = {Attention is all you Need},
  author = {Ashish Vaswani},
  year   = {2017},
  url    = {https://arxiv.org/abs/1706.03762}
}
"#;

    /// 保存できる名前か（英数字・.・-・_ + .bib。`..` やパス記号は弾く）。
    #[test]
    fn bib_name_rule() {
        assert!(is_valid_bib_name("refs.bib"));
        assert!(is_valid_bib_name("a-b_c.2024.bib"));
        assert!(is_valid_bib_name("REFS.BIB"));
        assert!(!is_valid_bib_name("refs.txt"));
        assert!(!is_valid_bib_name("../x.bib"));
        assert!(!is_valid_bib_name("a/b.bib"));
        assert!(!is_valid_bib_name("..bib"));
        assert!(!is_valid_bib_name(".bib"));
        assert!(!is_valid_bib_name("x..y.bib"));
        assert!(!is_valid_bib_name(&format!("{}.bib", "a".repeat(81))));
        assert!(is_valid_bib_name(&format!("{}.bib", "a".repeat(80))));
    }

    /// 正しい .bib は保存でき、項目数が返る。
    #[test]
    fn save_bib_file_writes_and_counts() {
        let dir = temp_dir("save-ok");
        let saved = save_bib_file(&dir, "refs.bib", SAMPLE_BIB).unwrap();
        assert_eq!(saved.count, 1);
        assert_eq!(saved.path, dir.join("refs.bib"));
        assert_eq!(load_bibtex_dir_detailed(&dir).unwrap().papers.len(), 1);
        // 一時ファイル（.bib.tmp）は残さない。
        assert!(!dir.join("refs.bib.tmp").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// フォルダが無ければ作って保存する（TUI の Import と同じ置き場）。
    #[test]
    fn save_bib_file_creates_dir() {
        let dir = temp_dir("save-mkdir").join("bibtex");
        let saved = save_bib_file(&dir, "refs.bib", SAMPLE_BIB).unwrap();
        assert!(saved.path.is_file());
        let _ = std::fs::remove_dir_all(dir.parent().unwrap());
    }

    /// 0 件の .bib は保存しない。
    #[test]
    fn save_bib_file_rejects_empty() {
        let dir = temp_dir("save-empty");
        let err = save_bib_file(&dir, "empty.bib", "@misc{NoTitle, author = {Nobody}}")
            .unwrap_err();
        assert!(matches!(err, BibtexError::Empty));
        assert!(!dir.join("empty.bib").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 壊れた .bib は保存しない。
    #[test]
    fn save_bib_file_rejects_broken() {
        let dir = temp_dir("save-broken");
        let err = save_bib_file(&dir, "broken.bib", "@article{Broken, title = {").unwrap_err();
        assert!(matches!(err, BibtexError::ParseSource { .. }));
        assert!(!dir.join("broken.bib").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 同名の .bib は上書きしない。
    #[test]
    fn save_bib_file_rejects_duplicate_name() {
        let dir = temp_dir("save-dup");
        save_bib_file(&dir, "refs.bib", SAMPLE_BIB).unwrap();
        let err = save_bib_file(&dir, "refs.bib", SAMPLE_BIB).unwrap_err();
        assert!(matches!(err, BibtexError::Exists));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `../x.bib` などの名前は保存も削除も許さない。
    #[test]
    fn save_and_remove_reject_bad_names() {
        let dir = temp_dir("bad-name");
        assert!(matches!(
            save_bib_file(&dir, "../x.bib", SAMPLE_BIB).unwrap_err(),
            BibtexError::InvalidName
        ));
        assert!(matches!(
            remove_bib_file(&dir, "..bib").unwrap_err(),
            BibtexError::InvalidName
        ));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 削除できたときだけ成功し、無ければ NotFound。
    #[test]
    fn remove_bib_file_removes_and_reports_missing() {
        let dir = temp_dir("remove");
        save_bib_file(&dir, "refs.bib", SAMPLE_BIB).unwrap();
        remove_bib_file(&dir, "refs.bib").unwrap();
        assert!(!dir.join("refs.bib").exists());
        assert!(matches!(
            remove_bib_file(&dir, "refs.bib").unwrap_err(),
            BibtexError::NotFound
        ));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 一覧はファイル名順で、解析に失敗したファイルは error に文言を入れる。
    #[test]
    fn list_bib_files_reports_each_file() {
        let dir = temp_dir("list");
        save_bib_file(&dir, "b.bib", SAMPLE_BIB).unwrap();
        std::fs::write(dir.join("a-broken.bib"), "@article{Broken, title = {").unwrap();
        std::fs::write(dir.join("c.txt"), "not a bib").unwrap();

        let files = list_bib_files(&dir).unwrap();
        assert_eq!(files.len(), 2);
        assert_eq!(files[0].name, "a-broken.bib");
        assert_eq!(files[0].count, 0);
        assert!(files[0].error.as_deref().unwrap().contains("a-broken.bib"));
        assert_eq!(files[1].name, "b.bib");
        assert_eq!(files[1].count, 1);
        assert!(files[1].error.is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// フォルダが無いときの一覧は空（エラーにしない）。
    #[test]
    fn list_bib_files_returns_empty_for_missing_dir() {
        let files = list_bib_files(Path::new("/nonexistent-bibtex-dir-u2")).unwrap();
        assert!(files.is_empty());
    }

    fn remote(id: &str) -> RemotePaper {
        RemotePaper {
            id: id.to_owned(),
            title: id.to_owned(),
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
}
