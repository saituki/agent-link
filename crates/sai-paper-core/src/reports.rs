//! agent の report（md ファイル）の登録と参照。
//!
//! report の md は sai-link の出力フォルダ
//! （`<root>/.sai/outputs/<owner>/<run_id>/`）に置かれたままとする。
//! sai-paper は「どの report がどの論文・引用箇所・別の report を参照したか」だけを
//! DB に記録する。
//!
//! **report の本文は sai-paper から書き換えない**（要件 R3.3）。
//! このモジュールには md を書き換える関数を置かない。

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use chrono::Utc;
use regex::Regex;
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

/// report 操作のエラー。
#[derive(Debug, Error)]
pub enum ReportError {
    /// SQLite 操作の失敗
    #[error("database error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    /// ファイル読み込みの失敗
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    /// report が見つからない
    #[error("report not found: {0}")]
    NotFound(String),
    /// md ファイルが見つからない
    #[error("markdown file not found: {0}")]
    MarkdownNotFound(String),
}

/// `reports` の 1 行。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Report {
    /// report_id（例: `R-0007`）
    pub id: String,
    /// タイトル（md の H1 または frontmatter から）
    pub title: Option<String>,
    /// md ファイルの絶対パス（登録時の値。移動している場合がある）
    pub md_path: String,
    /// 生成元の agent 実行 ID
    pub run_id: Option<String>,
    /// 実行した block id
    pub block: Option<String>,
    /// 作成時刻（ISO8601）
    pub created_at: String,
    /// md 先頭数行（Canvas カード表示用）
    pub summary: Option<String>,
}

/// `reports` の検査の状態（`report scan` が入れる列）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReportCheck {
    /// 先頭情報（frontmatter）の検査に error が無いこと
    pub header_ok: bool,
    /// 最後に検査した時刻（ISO8601）
    pub checked_at: Option<String>,
    /// 検査で見つけた error の件数
    pub check_errors: i64,
    /// 検査で見つけた warning の件数
    pub check_warnings: i64,
}

/// `report_sources` の 1 行（report が参照した入力元）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReportSource {
    /// `paper` / `chunk` / `report`
    pub kind: String,
    /// `kind` に応じた識別子（`papers.id` / `chunks.id` / `reports.id`）
    pub ref_id: String,
    /// 任意の役割
    pub role: Option<String>,
}

/// 新規 report の登録内容。
#[derive(Debug, Clone, Default)]
pub struct NewReport {
    /// タイトル（None なら md の H1 から推定）
    pub title: Option<String>,
    /// md ファイルの絶対パス
    pub md_path: String,
    /// 生成元の agent 実行 ID
    pub run_id: Option<String>,
    /// 実行した block id
    pub block: Option<String>,
    /// 参照元（論文 / 引用箇所 / 別 report）
    pub sources: Vec<ReportSource>,
    /// summary（None なら md から切り出す）
    pub summary: Option<String>,
}

const REPORT_COLUMNS: &str = "id, title, md_path, run_id, block, created_at, summary";

fn map_report(row: &rusqlite::Row<'_>) -> rusqlite::Result<Report> {
    Ok(Report {
        id: row.get(0)?,
        title: row.get(1)?,
        md_path: row.get(2)?,
        run_id: row.get(3)?,
        block: row.get(4)?,
        created_at: row.get(5)?,
        summary: row.get(6)?,
    })
}

/// md 本文から summary（先頭の意味のある数行）を作る。
///
/// YAML frontmatter・空行・見出し記号を除いた先頭 `max_lines` 行を連結する。
#[must_use]
pub fn summary_from_markdown(markdown: &str, max_lines: usize) -> String {
    let mut body = markdown;
    // 先頭の YAML frontmatter を読み飛ばす
    if let Some(rest) = markdown.strip_prefix("---") {
        if let Some(end) = rest.find("\n---") {
            body = &rest[end + 4..];
        }
    }
    let mut parts: Vec<String> = Vec::new();
    for line in body.lines() {
        let trimmed = line.trim().trim_start_matches('#').trim();
        if trimmed.is_empty() {
            continue;
        }
        parts.push(trimmed.to_string());
        if parts.len() >= max_lines {
            break;
        }
    }
    let joined = parts.join(" / ");
    if joined.chars().count() > 240 {
        let truncated: String = joined.chars().take(240).collect();
        format!("{truncated}…")
    } else {
        joined
    }
}

/// md 本文からタイトル（先頭の H1、無ければ最初の非空行）を取り出す。
#[must_use]
pub fn title_from_markdown(markdown: &str) -> Option<String> {
    let mut body = markdown;
    if let Some(rest) = markdown.strip_prefix("---")
        && let Some(end) = rest.find("\n---")
    {
        body = &rest[end + 4..];
    }
    for line in body.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let text = trimmed.trim_start_matches('#').trim();
        if !text.is_empty() {
            return Some(text.to_string());
        }
    }
    None
}

/// 次に使う report_id（`R-0001` 形式）を返す。
///
/// # Errors
///
/// SQLite エラー。
pub fn next_report_id(conn: &Connection) -> Result<String, ReportError> {
    let max: Option<i64> = conn.query_row(
        "SELECT MAX(CAST(SUBSTR(id, 3) AS INTEGER)) FROM reports WHERE id LIKE 'R-%'",
        [],
        |row| row.get(0),
    )?;
    Ok(format!("R-{:04}", max.unwrap_or(0) + 1))
}

/// report を登録し、採番した report_id を返す。
///
/// md の内容は読み取りのみ（summary / title の切り出しに使う）。
///
/// # Errors
///
/// SQLite / I/O エラー。
pub fn create_report(conn: &Connection, new: &NewReport) -> Result<String, ReportError> {
    let markdown = std::fs::read_to_string(&new.md_path).unwrap_or_default();
    let title = new.title.clone().or_else(|| title_from_markdown(&markdown));
    let summary = new
        .summary
        .clone()
        .unwrap_or_else(|| summary_from_markdown(&markdown, 3));

    let id = next_report_id(conn)?;
    let tx = conn.unchecked_transaction()?;
    tx.execute(
        "INSERT INTO reports (id, title, md_path, run_id, block, created_at, summary)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            id,
            title,
            new.md_path,
            new.run_id,
            new.block,
            Utc::now().to_rfc3339(),
            summary,
        ],
    )?;
    {
        let mut stmt = tx.prepare(
            "INSERT OR IGNORE INTO report_sources (report_id, kind, ref_id, role)
             VALUES (?1, ?2, ?3, ?4)",
        )?;
        for source in &new.sources {
            stmt.execute(params![id, source.kind, source.ref_id, source.role])?;
        }
    }
    tx.commit()?;
    Ok(id)
}

/// report を 1 件取得する。
///
/// # Errors
///
/// SQLite エラー。
pub fn get_report(conn: &Connection, report_id: &str) -> Result<Option<Report>, ReportError> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {REPORT_COLUMNS} FROM reports WHERE id = ?1"
    ))?;
    let report = stmt.query_row(params![report_id], map_report).optional()?;
    Ok(report)
}

/// report の参照元一覧を取得する。
///
/// # Errors
///
/// SQLite エラー。
pub fn report_sources(
    conn: &Connection,
    report_id: &str,
) -> Result<Vec<ReportSource>, ReportError> {
    let mut stmt = conn.prepare(
        "SELECT kind, ref_id, role FROM report_sources WHERE report_id = ?1 ORDER BY kind, ref_id",
    )?;
    let rows = stmt
        .query_map(params![report_id], |row| {
            Ok(ReportSource {
                kind: row.get(0)?,
                ref_id: row.get(1)?,
                role: row.get(2)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// report を新しい順に一覧する。
///
/// `paper_id` を指定すると、その論文を参照している report のみ返す。
///
/// # Errors
///
/// SQLite エラー。
pub fn list_reports(conn: &Connection, paper_id: Option<i64>) -> Result<Vec<Report>, ReportError> {
    match paper_id {
        Some(pid) => {
            let mut stmt = conn.prepare(&format!(
                "SELECT {} FROM reports r
                 JOIN report_sources s ON s.report_id = r.id
                 WHERE s.kind = 'paper' AND s.ref_id = ?1
                 ORDER BY r.created_at DESC, r.id DESC",
                REPORT_COLUMNS
                    .split(", ")
                    .map(|c| format!("r.{c}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ))?;
            let rows = stmt
                .query_map(params![pid.to_string()], map_report)?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        }
        None => {
            let mut stmt = conn.prepare(&format!(
                "SELECT {REPORT_COLUMNS} FROM reports ORDER BY created_at DESC, id DESC"
            ))?;
            let rows = stmt
                .query_map([], map_report)?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        }
    }
}

/// 論文（`papers.id`）を参照している report の一覧を返す。
///
/// # Errors
///
/// SQLite エラー。
pub fn reports_for_paper(conn: &Connection, papers_id: i64) -> Result<Vec<Report>, ReportError> {
    list_reports(conn, Some(papers_id))
}

/// report の md の実パスを解決する。
///
/// 登録時のパスが無い場合（run が `running` → `done` → `R<n>` と進んで移動した場合）、
/// `run_id` から `<root>/.sai/outputs/*/<run_id>/<ファイル名>` を探す。
#[must_use]
pub fn resolve_md_path(report: &Report) -> Option<PathBuf> {
    let stored = Path::new(&report.md_path);
    if stored.is_file() {
        return Some(stored.to_path_buf());
    }
    let run_id = report.run_id.as_deref()?;
    let filename = stored.file_name()?;
    let root = crate::paths::project_root()?;
    let outputs = root.join(".sai").join("outputs");
    let entries = std::fs::read_dir(&outputs).ok()?;
    for entry in entries.flatten() {
        let candidate = entry.path().join(run_id).join(filename);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

/// report の md 本文を読み出す。
///
/// # Errors
///
/// md が見つからない / 読み込めない場合。
pub fn read_report_markdown(conn: &Connection, report_id: &str) -> Result<String, ReportError> {
    let report =
        get_report(conn, report_id)?.ok_or_else(|| ReportError::NotFound(report_id.to_string()))?;
    let path = resolve_md_path(&report).ok_or_else(|| {
        ReportError::MarkdownNotFound(format!("{} (report_id={})", report.md_path, report.id))
    })?;
    Ok(std::fs::read_to_string(&path)?)
}

/// `papers.id` → `papers.title` を返す（見出し生成用）。
///
/// # Errors
///
/// SQLite エラー。
pub fn paper_title(conn: &Connection, papers_id: i64) -> Result<Option<String>, ReportError> {
    let title: Option<Option<String>> = conn
        .query_row(
            "SELECT title FROM papers WHERE id = ?1",
            [papers_id],
            |row| row.get(0),
        )
        .optional()?;
    Ok(title.flatten())
}

/// report を持つ論文を、report 込みで列挙する（Canvas の right_sidebar 用）。
///
/// 戻り値は `(papers_id, paper_title, report)`。同じ report が複数の論文の下に
/// 現れることがある（要件 R5.5）。
///
/// # Errors
///
/// SQLite エラー。
pub fn paper_report_links(conn: &Connection) -> Result<Vec<(i64, String, Report)>, ReportError> {
    let mut stmt = conn.prepare(
        "SELECT CAST(s.ref_id AS INTEGER), COALESCE(p.title, ''), r.id, r.title, r.md_path,
                r.run_id, r.block, r.created_at, r.summary
         FROM report_sources s
         JOIN reports r ON r.id = s.report_id
         LEFT JOIN papers p ON p.id = CAST(s.ref_id AS INTEGER)
         WHERE s.kind = 'paper' AND s.ref_id GLOB '[0-9]*'
         ORDER BY CAST(s.ref_id AS INTEGER), r.created_at DESC, r.id DESC",
    )?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                Report {
                    id: row.get(2)?,
                    title: row.get(3)?,
                    md_path: row.get(4)?,
                    run_id: row.get(5)?,
                    block: row.get(6)?,
                    created_at: row.get(7)?,
                    summary: row.get(8)?,
                },
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

// 引用の拾い方と、先頭情報（frontmatter）の検査、自動登録（`scan`）。
//
// 引用の書き方は sai-link の `literature-hard-rules` に定めた形に限る。
// 先頭情報の検査は `research-core` に定めた形（`kind` / `block` / `papers`）を見る。
// どちらも md は読み取りのみで、結果は DB と画面に出す（md は書き換えない）。

/// 引用の指し先。
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub enum CitationTarget {
    /// Library の論文（`[#37]` の 37。`papers.id`）
    Paper(i64),
    /// Library に無い論文（`[DOI:…]` / `[arXiv:…]` / `[PMID:…]`）
    External {
        /// `DOI` / `arXiv` / `PMID`
        scheme: String,
        /// 識別子（`10.1000/xyz` など）
        id: String,
    },
    /// 確かめられないもの（`[未確認: …]` の中身）
    Unverified(String),
}

/// md 本文から拾った引用 1 つ。
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Citation {
    /// 引用の指し先
    pub target: CitationTarget,
    /// 場所（`§3.2` / `p.5` / `Fig.3` など。無ければ `None`）
    pub locator: Option<String>,
    /// 1 始まりの行番号
    pub line: usize,
}

/// 検査で見つけた問題 1 つ（`report_issues` の 1 行）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Issue {
    /// `error` / `warning`
    pub level: String,
    /// `header` / `missing_paper` / `unresolved` / `mismatch` / `retracted` / `unsupported_number` / `style`
    pub kind: String,
    /// 人が読む説明
    pub detail: String,
    /// 1 始まりの行番号（分かるときのみ）
    pub line: Option<i64>,
}

/// 引用の括弧を拾う正規表現。
static CITATION_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\[([^\[\]]*)\]").expect("citation bracket regex"));

/// md 本文から引用（共有契約の書き方）をすべて拾う。
///
/// コード欄（``` の間）とインラインのコード（`` ` `` の間）の中は拾わない。
#[must_use]
pub fn parse_citations(markdown: &str) -> Vec<Citation> {
    let masked = mask_code(markdown);
    let mut citations = Vec::new();
    for (index, line) in masked.lines().enumerate() {
        let line_no = index + 1;
        for capture in CITATION_RE.captures_iter(line) {
            if let Some(found) = parse_citation_inner(capture[1].trim(), line_no) {
                citations.extend(found);
            }
        }
    }
    citations
}

/// 括弧の中身を引用に直す。引用の形でなければ `None`。
fn parse_citation_inner(inner: &str, line: usize) -> Option<Vec<Citation>> {
    if inner.is_empty() {
        return None;
    }
    // 未確認: …
    if let Some(rest) = inner.strip_prefix("未確認:") {
        let text = rest.trim();
        if text.is_empty() {
            return None;
        }
        return Some(vec![Citation {
            target: CitationTarget::Unverified(text.to_string()),
            locator: None,
            line,
        }]);
    }
    // Library の論文: [#37] / [#37 §3.2] / [#37, #40]
    if inner.starts_with('#') {
        let mut citations = Vec::new();
        for part in inner.split(',') {
            let part = part.trim();
            if part.is_empty() {
                continue;
            }
            let rest = part.strip_prefix('#')?;
            let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
            if digits.is_empty() {
                return None;
            }
            let number = digits.parse::<i64>().ok()?;
            let locator = rest[digits.len()..].trim();
            citations.push(Citation {
                target: CitationTarget::Paper(number),
                locator: (!locator.is_empty()).then(|| locator.to_string()),
                line,
            });
        }
        return (!citations.is_empty()).then_some(citations);
    }
    // 外部: [DOI:…] / [arXiv:…] / [PMID:…]
    if let Some((scheme, id)) = inner.split_once(':') {
        let scheme = scheme.trim();
        let id = id.trim();
        let canonical = if scheme.eq_ignore_ascii_case("doi") {
            Some("DOI")
        } else if scheme.eq_ignore_ascii_case("arxiv") {
            Some("arXiv")
        } else if scheme.eq_ignore_ascii_case("pmid") {
            Some("PMID")
        } else {
            None
        };
        if let Some(scheme) = canonical
            && !id.is_empty()
        {
            return Some(vec![Citation {
                target: CitationTarget::External {
                    scheme: scheme.to_string(),
                    id: id.to_string(),
                },
                locator: None,
                line,
            }]);
        }
    }
    None
}

/// コード欄とインラインのコードの中身を空白に置き換える（行数は変えない）。
fn mask_code(markdown: &str) -> String {
    let mut out = String::with_capacity(markdown.len());
    let mut in_fence = false;
    let mut first = true;
    for line in markdown.split('\n') {
        if !first {
            out.push('\n');
        }
        first = false;
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            in_fence = !in_fence;
            out.push_str(&" ".repeat(line.chars().count()));
        } else if in_fence {
            out.push_str(&" ".repeat(line.chars().count()));
        } else {
            out.push_str(&mask_inline_code(line));
        }
    }
    out
}

/// 1 行の中のインラインコード（`` ` `` で囲まれた範囲）を空白にする。
fn mask_inline_code(line: &str) -> String {
    let chars: Vec<char> = line.chars().collect();
    let mut out = chars.clone();
    let mut index = 0;
    while index < chars.len() {
        if chars[index] != '`' {
            index += 1;
            continue;
        }
        let mut open = 0;
        while index + open < chars.len() && chars[index + open] == '`' {
            open += 1;
        }
        // 同じ数の `` ` `` が後ろにあれば、その間をコードとみなす。
        let mut cursor = index + open;
        let mut close = None;
        while cursor < chars.len() {
            if chars[cursor] == '`' {
                let mut run = 0;
                while cursor + run < chars.len() && chars[cursor + run] == '`' {
                    run += 1;
                }
                if run == open {
                    close = Some(cursor + run);
                    break;
                }
                cursor += run;
            } else {
                cursor += 1;
            }
        }
        match close {
            Some(end) => {
                for cell in out.iter_mut().take(end).skip(index) {
                    *cell = ' ';
                }
                index = end;
            }
            None => index += open,
        }
    }
    out.into_iter().collect()
}

/// 日付（`2026-09-24`）を数値とみなさないための正規表現。
static DATE_NUMBER_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\d{4}-\d{2}-\d{2}").expect("date regex"));

/// 年（`2023 年`）を数値とみなさないための正規表現。
static YEAR_NUMBER_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\d{4}\s*年").expect("year regex"));

/// 章番号（`§3.2` / `3.2 節`）を数値とみなさないための正規表現。
static SECTION_NUMBER_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"§\s*\d+(?:\.\d+)*|\d+(?:\.\d+)*\s*節").expect("section regex")
});

/// Library の番号（`#37`）を数値とみなさないための正規表現。
static LIBRARY_NUMBER_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"#\d+").expect("library number regex"));

/// 根拠が必要な数字（単位・割合・有意差つき）。
static UNIT_NUMBER_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\d+(?:\.\d+)?\s*(?:%|％|倍|ms|Hz|mg|kg|人|例|件|歳)")
        .expect("unit number regex")
});

/// 根拠が必要な数字（`p <` / `p=` / `n =` など）。
static PN_NUMBER_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"[pnPN]\s*[<>=]\s*\d+(?:\.\d+)?").expect("p/n number regex")
});

/// 小数点を含む数（`0.87`）。
static DECIMAL_NUMBER_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\d+\.\d+").expect("decimal regex"));

/// 決まりに合わない引用の書き方（`Smith 2020` / `Smith et al., 2020` / `Smith ら (2020)`）。
static AUTHOR_YEAR_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\(?[A-Z][a-zA-Z-]+( et al\.?| ら)?,? \(?\d{4}\)?").expect("author year regex")
});

/// 括弧の中に引用（共有契約の形）があるかを返す。
fn has_citation(text: &str) -> bool {
    CITATION_RE
        .captures_iter(text)
        .any(|capture| parse_citation_inner(capture[1].trim(), 0).is_some())
}

/// 1 行を文に分ける。区切りは `。` / `. ` / 行の終わり。
fn split_sentences(line: &str) -> Vec<String> {
    let chars: Vec<char> = line.chars().collect();
    let mut sentences = Vec::new();
    let mut current = String::new();
    let mut index = 0;
    while index < chars.len() {
        let ch = chars[index];
        if ch == '。' {
            current.push(ch);
            let sentence = current.trim().to_string();
            if !sentence.is_empty() {
                sentences.push(sentence);
            }
            current.clear();
            index += 1;
        } else if ch == '.' && chars.get(index + 1) == Some(&' ') {
            current.push(ch);
            let sentence = current.trim().to_string();
            if !sentence.is_empty() {
                sentences.push(sentence);
            }
            current.clear();
            index += 2;
        } else {
            current.push(ch);
            index += 1;
        }
    }
    let sentence = current.trim().to_string();
    if !sentence.is_empty() {
        sentences.push(sentence);
    }
    sentences
}

/// 1 つの文に「根拠が必要な数字」があるかを返す。
fn sentence_has_target_number(sentence: &str) -> bool {
    // 日付・年・章番号・Library の番号を先に空白へ置き換える。
    let mut cleaned = DATE_NUMBER_RE.replace_all(sentence, " ").into_owned();
    cleaned = YEAR_NUMBER_RE.replace_all(&cleaned, " ").into_owned();
    cleaned = SECTION_NUMBER_RE.replace_all(&cleaned, " ").into_owned();
    cleaned = LIBRARY_NUMBER_RE.replace_all(&cleaned, " ").into_owned();
    DECIMAL_NUMBER_RE.is_match(&cleaned)
        || UNIT_NUMBER_RE.is_match(&cleaned)
        || PN_NUMBER_RE.is_match(&cleaned)
}

/// 文の先頭 60 字（長いときは `…` を付ける）。
fn head_60(sentence: &str) -> String {
    if sentence.chars().count() > 60 {
        let head: String = sentence.chars().take(60).collect();
        format!("{head}…")
    } else {
        sentence.to_string()
    }
}

/// 引用の無い数字（根拠の無い数字）を拾う。
///
/// 戻り値は `(1 始まりの行番号, その文の先頭 60 字)`。
/// 同じ文に共有契約の引用が 1 つでもあれば対象外。コード欄・インラインコード・
/// frontmatter・見出しの行・`## 引き継ぎ` 節の中も対象外。表の行は、表の直前 3 行か
/// 直後 3 行に引用があれば対象外。
#[must_use]
pub fn unsupported_numbers(markdown: &str) -> Vec<(usize, String)> {
    let masked = mask_code(markdown);
    let lines: Vec<&str> = masked.split('\n').collect();
    let count = lines.len();

    // 除外する行（frontmatter・見出し・`## 引き継ぎ` 節）
    let mut skip = vec![false; count];
    if lines.first().is_some_and(|line| line.trim() == "---") {
        skip[0] = true;
        for (index, line) in lines.iter().enumerate().skip(1) {
            skip[index] = true;
            if line.trim() == "---" {
                break;
            }
        }
    }
    let mut handoff_level: Option<usize> = None;
    for (index, line) in lines.iter().enumerate() {
        let trimmed = line.trim_start();
        if let Some(rest) = trimmed.strip_prefix('#') {
            let level = 1 + rest.chars().take_while(|ch| *ch == '#').count();
            let text = rest.trim_start_matches('#').trim();
            skip[index] = true;
            handoff_level = match handoff_level {
                Some(open) if level <= open => None,
                Some(open) => Some(open),
                None if text.contains("引き継ぎ") => Some(level),
                None => None,
            };
            continue;
        }
        if handoff_level.is_some() {
            skip[index] = true;
        }
    }

    // 表の行（`|` で始まる連続行）は、前後 3 行に引用があればまとめて対象外。
    let citation_lines: HashSet<usize> = parse_citations(markdown)
        .into_iter()
        .map(|citation| citation.line)
        .collect();
    let mut table_covered = vec![false; count];
    let mut index = 0;
    while index < count {
        if skip[index] || !lines[index].trim_start().starts_with('|') {
            index += 1;
            continue;
        }
        let start = index;
        let mut end = index;
        while end + 1 < count && !skip[end + 1] && lines[end + 1].trim_start().starts_with('|') {
            end += 1;
        }
        let near = (start.saturating_sub(3)..start)
            .chain(end + 1..=(end + 3).min(count.saturating_sub(1)))
            .any(|line| citation_lines.contains(&(line + 1)));
        if near {
            for slot in table_covered.iter_mut().take(end + 1).skip(start) {
                *slot = true;
            }
        }
        index = end + 1;
    }

    let mut found = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        if skip[index] || table_covered[index] {
            continue;
        }
        for sentence in split_sentences(line) {
            if has_citation(&sentence) {
                continue;
            }
            if sentence_has_target_number(&sentence) {
                found.push((index + 1, head_60(&sentence)));
            }
        }
    }
    found
}

/// 決まりに合わない引用の書き方（著者 年 の形）の件数を数える。
#[must_use]
fn author_year_count(markdown: &str) -> usize {
    AUTHOR_YEAR_RE.find_iter(&mask_code(markdown)).count()
}

/// 決まりに合わない引用の書き方を 1 件の warning にまとめる。
fn style_issues(markdown: &str) -> Vec<Issue> {
    let count = author_year_count(markdown);
    if count == 0 {
        Vec::new()
    } else {
        vec![Issue {
            level: "warning".into(),
            kind: "style".into(),
            detail: format!(
                "引用の書き方に合わない箇所がありそうです（著者 年 の形）: {count} 件"
            ),
            line: None,
        }]
    }
}

/// Library に無い番号を引用している問題を集める。
fn missing_paper_issues(
    conn: &Connection,
    citations: &[Citation],
) -> Result<Vec<Issue>, ReportError> {
    let mut stmt = conn.prepare("SELECT 1 FROM papers WHERE id = ?1")?;
    let mut issues = Vec::new();
    for citation in citations {
        if let CitationTarget::Paper(number) = &citation.target {
            let exists = stmt.query_row([*number], |_| Ok(())).optional()?.is_some();
            if !exists {
                issues.push(Issue {
                    level: "error".into(),
                    kind: "missing_paper".into(),
                    detail: format!("Library に #{number} がありません"),
                    line: Some(citation.line as i64),
                });
            }
        }
    }
    Ok(issues)
}

/// ネットを使わずに確かめられる問題を、md と DB から集める。
fn collect_issues(
    conn: &Connection,
    markdown: &str,
    block_id: &str,
) -> Result<Vec<Issue>, ReportError> {
    let citations = parse_citations(markdown);
    let mut issues = check_header(markdown, Some(block_id));
    issues.extend(missing_paper_issues(conn, &citations)?);
    for (line, head) in unsupported_numbers(markdown) {
        issues.push(Issue {
            level: "warning".into(),
            kind: "unsupported_number".into(),
            detail: format!("根拠の無い数字: {head}"),
            line: Some(line as i64),
        });
    }
    issues.extend(style_issues(markdown));
    Ok(issues)
}

/// report の先頭情報（frontmatter）を検査する。
///
/// 共有契約の規則に違反があれば返す。frontmatter の読み取りは
/// [`crate::runs::read_frontmatter`] を使う（`papers: [37, 40]` は値として受け取り、
/// ここで数の並びに直す）。
#[must_use]
pub fn check_header(markdown: &str, run_block_id: Option<&str>) -> Vec<Issue> {
    let pairs = crate::runs::read_frontmatter(markdown);
    if pairs.is_empty() {
        return vec![Issue {
            level: "error".into(),
            kind: "header".into(),
            detail: "先頭情報（frontmatter）がありません".into(),
            line: None,
        }];
    }
    let get = |key: &str| {
        pairs
            .iter()
            .find(|(name, _)| name == key)
            .map(|(_, value)| value.as_str())
    };

    let mut issues = Vec::new();

    match get("kind") {
        None | Some("") => issues.push(header_issue("kind が空です")),
        Some(_) => {}
    }

    match get("block") {
        None | Some("") => issues.push(header_issue("block が空です")),
        Some(actual) => {
            if let Some(expected) = run_block_id
                && actual != expected
            {
                issues.push(header_issue(format!(
                    "block が実行のブロック（{expected}）と違います: {actual}"
                )));
            }
        }
    }

    let declared: Option<HashSet<i64>> = match get("papers") {
        None => {
            issues.push(header_issue("papers がありません"));
            None
        }
        Some(value) => {
            let value = value.trim();
            if value.is_empty() {
                Some(HashSet::new())
            } else {
                let mut numbers = HashSet::new();
                let mut valid = true;
                for part in value.split(',') {
                    match part.trim().parse::<i64>() {
                        Ok(number) => {
                            numbers.insert(number);
                        }
                        Err(_) => valid = false,
                    }
                }
                if valid {
                    Some(numbers)
                } else {
                    issues.push(header_issue("papers が数の配列ではありません"));
                    None
                }
            }
        }
    };

    if let Some(declared) = declared {
        // 本文で引用した番号が papers に無いときは warning（同じ番号は 1 回）。
        let mut missing: HashMap<i64, usize> = HashMap::new();
        for citation in parse_citations(markdown) {
            if let CitationTarget::Paper(number) = citation.target
                && !declared.contains(&number)
            {
                missing.entry(number).or_insert(citation.line);
            }
        }
        let mut missing: Vec<(i64, usize)> = missing.into_iter().collect();
        missing.sort_unstable();
        for (number, line) in missing {
            issues.push(Issue {
                level: "warning".into(),
                kind: "header".into(),
                detail: format!("本文で引用した #{number} が papers にありません"),
                line: Some(line as i64),
            });
        }
    }

    issues
}

fn header_issue(detail: impl Into<String>) -> Issue {
    Issue {
        level: "error".into(),
        kind: "header".into(),
        detail: detail.into(),
        line: None,
    }
}

/// `scan` の結果のまとめ。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScanReport {
    /// 新しく登録した report の数
    pub added: usize,
    /// 場所や中身が変わって直した report の数
    pub updated: usize,
    /// 変化の無かった report の数
    pub unchanged: usize,
    /// 研究用でないブロックとして飛ばした実行の数
    pub skipped_dev: usize,
    /// 先頭情報に error がある report の数（DB 全体）
    pub header_errors: usize,
}

/// report scan で登録しない md（sai-link が置くもの）。
fn is_excluded_md(name: &str) -> bool {
    matches!(
        name,
        "task.md" | "review.md" | "message.md" | "finalize-report.md"
    )
}

/// md 本文の sha256（16 進小文字）。
fn sha256_hex(text: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(text.as_bytes());
    format!("{:x}", hasher.finalize())
}

/// 引用から `report_sources` の行を作る（`paper` / `external` のみ、番号は 1 つ）。
fn sources_from_citations(citations: &[Citation]) -> Vec<ReportSource> {
    let mut seen = HashSet::new();
    let mut sources = Vec::new();
    for citation in citations {
        let (kind, ref_id) = match &citation.target {
            CitationTarget::Paper(number) => ("paper", number.to_string()),
            CitationTarget::External { scheme, id } => ("external", format!("{scheme}:{id}")),
            CitationTarget::Unverified(_) => continue,
        };
        if seen.insert((kind.to_string(), ref_id.clone())) {
            sources.push(ReportSource {
                kind: kind.to_string(),
                ref_id,
                role: None,
            });
        }
    }
    sources
}

/// report ごとの問題の件数。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct IssueCounts {
    /// `level='error'` の問題の数
    pub errors: usize,
    /// `level='warning'` の問題の数
    pub warnings: usize,
}

/// report 1 件と、その問題の一覧。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReportIssues {
    /// report
    pub report: Report,
    /// その report の問題
    pub issues: Vec<Issue>,
}

/// 1 件の report の問題の件数を数える。
///
/// # Errors
///
/// SQLite エラー。
pub fn issue_counts(conn: &Connection, report_id: &str) -> Result<IssueCounts, ReportError> {
    let (errors, warnings) = conn.query_row(
        "SELECT
            COALESCE(SUM(CASE WHEN level = 'error' THEN 1 ELSE 0 END), 0),
            COALESCE(SUM(CASE WHEN level = 'warning' THEN 1 ELSE 0 END), 0)
         FROM report_issues WHERE report_id = ?1",
        params![report_id],
        |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
    )?;
    Ok(IssueCounts {
        errors: errors as usize,
        warnings: warnings as usize,
    })
}

/// DB 全体の問題の件数を数える。
///
/// # Errors
///
/// SQLite エラー。
pub fn total_issue_counts(conn: &Connection) -> Result<IssueCounts, ReportError> {
    let (errors, warnings) = conn.query_row(
        "SELECT
            COALESCE(SUM(CASE WHEN level = 'error' THEN 1 ELSE 0 END), 0),
            COALESCE(SUM(CASE WHEN level = 'warning' THEN 1 ELSE 0 END), 0)
         FROM report_issues",
        [],
        |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
    )?;
    Ok(IssueCounts {
        errors: errors as usize,
        warnings: warnings as usize,
    })
}

/// report とその問題を一覧する。
///
/// `report_id` を指定すると、その report だけを返す（存在しなければ空）。
/// 指定しないときは、新しい report から順にすべての report を返す
/// （問題の無い report も含む。表示するかどうかは呼び出し側が決める）。
///
/// # Errors
///
/// SQLite エラー。
pub fn list_issues(
    conn: &Connection,
    report_id: Option<&str>,
) -> Result<Vec<ReportIssues>, ReportError> {
    let reports = match report_id {
        Some(id) => match get_report(conn, id)? {
            Some(report) => vec![report],
            None => Vec::new(),
        },
        None => list_reports(conn, None)?,
    };
    let mut stmt = conn.prepare(
        "SELECT level, kind, detail, line FROM report_issues
         WHERE report_id = ?1 ORDER BY level ASC, (line IS NULL) ASC, line ASC, kind ASC",
    )?;
    let mut result = Vec::with_capacity(reports.len());
    for report in reports {
        let issues = stmt
            .query_map(params![report.id], |row| {
                Ok(Issue {
                    level: row.get(0)?,
                    kind: row.get(1)?,
                    detail: row.get(2)?,
                    line: row.get(3)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        result.push(ReportIssues { report, issues });
    }
    Ok(result)
}

/// 検査の結果（件数と時刻）を `reports` に書き込む。
///
/// 件数は `report_issues` からその都度数え直す（他の作業が入れた問題も含める）。
/// `file_name` / `content_hash` は `scan` が入れる列で、`scan` 以外（`report check`）が
/// 数え直すときは既存の値をそのまま渡す（`None` なら NULL のままにする）。
pub(crate) fn write_check(
    conn: &Connection,
    report_id: &str,
    file_name: Option<&str>,
    content_hash: Option<&str>,
) -> Result<(), ReportError> {
    let counts = issue_counts(conn, report_id)?;
    // `header_ok` は先頭情報（frontmatter）の検査に error が無いこと。
    let header_errors: i64 = conn.query_row(
        "SELECT COUNT(*) FROM report_issues
         WHERE report_id = ?1 AND kind = 'header' AND level = 'error'",
        params![report_id],
        |row| row.get(0),
    )?;
    conn.execute(
        "UPDATE reports SET file_name = ?1, content_hash = ?2, header_ok = ?3, checked_at = ?4,
                check_errors = ?5, check_warnings = ?6 WHERE id = ?7",
        params![
            file_name,
            content_hash,
            header_errors == 0,
            Utc::now().to_rfc3339(),
            counts.errors as i64,
            counts.warnings as i64,
            report_id
        ],
    )?;
    Ok(())
}

/// 検査の問題を `report_issues` に入れる。
pub(crate) fn insert_issues(
    conn: &Connection,
    report_id: &str,
    issues: &[Issue],
) -> Result<(), ReportError> {
    let mut stmt = conn.prepare(
        "INSERT INTO report_issues (report_id, level, kind, detail, line)
         VALUES (?1, ?2, ?3, ?4, ?5)",
    )?;
    for issue in issues {
        stmt.execute(params![
            report_id,
            issue.level,
            issue.kind,
            issue.detail,
            issue.line
        ])?;
    }
    Ok(())
}

/// `papers` に `is_retracted` の列があるか。
///
/// 古い最小スキーマで組んだテスト（`citation_check.rs` の `setup` など）には
/// この列が無い。そのときは撤回の検査を何もしない（migration 0041 が当たって
/// いれば必ずある）。
fn has_papers_retracted_column(conn: &Connection) -> Result<bool, ReportError> {
    let mut stmt = conn.prepare("PRAGMA table_info(papers)")?;
    let mut rows = stmt.query([])?;
    while let Some(row) = rows.next()? {
        let name: String = row.get(1)?;
        if name == "is_retracted" {
            return Ok(true);
        }
    }
    Ok(false)
}

/// 撤回された論文を引用した report に `retracted`（error）を入れ直す。
///
/// `report_sources(kind='paper')` の論文が `is_retracted = 1` なら、その report に
/// `level='error', kind='retracted', detail='撤回された論文 #12 を引用しています'`
/// を入れる。同じ組は重ねず、撤回でなくなった組は消す。件数は `write_check` で
/// 数え直す。入れた問題の数を返す。
///
/// # Errors
///
/// SQLite エラー。
pub fn refresh_retracted_issues(conn: &Connection) -> Result<usize, ReportError> {
    if !has_papers_retracted_column(conn)? {
        return Ok(0);
    }

    // いま `retracted` の問題を持っている report（消えた組を数え直すために使う）。
    let mut stmt = conn.prepare("SELECT DISTINCT report_id FROM report_issues WHERE kind = 'retracted'")?;
    let before: HashSet<String> = stmt
        .query_map([], |row| row.get(0))?
        .collect::<Result<_, _>>()?;
    drop(stmt);

    // 入れるべき `(report_id, paper_id)`（重複は `report_sources` の主キーで消えている）。
    let mut stmt = conn.prepare(
        "SELECT DISTINCT rs.report_id, CAST(rs.ref_id AS INTEGER)
         FROM report_sources rs
         JOIN papers p ON p.id = CAST(rs.ref_id AS INTEGER)
         WHERE rs.kind = 'paper' AND p.is_retracted = 1
         ORDER BY rs.report_id, 2",
    )?;
    let desired: Vec<(String, i64)> = stmt
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<Result<_, _>>()?;
    drop(stmt);

    conn.execute("DELETE FROM report_issues WHERE kind = 'retracted'", [])?;
    let mut stmt = conn.prepare(
        "INSERT INTO report_issues (report_id, level, kind, detail, line)
         VALUES (?1, 'error', 'retracted', ?2, NULL)",
    )?;
    for (report_id, paper_id) in &desired {
        stmt.execute(params![
            report_id,
            format!("撤回された論文 #{paper_id} を引用しています")
        ])?;
    }
    drop(stmt);

    // 前から持っていた report と、今入れた report の件数を数え直す。
    let mut affected: HashSet<String> = before;
    for (report_id, _) in &desired {
        affected.insert(report_id.clone());
    }
    for report_id in &affected {
        recount_check(conn, report_id)?;
    }
    Ok(desired.len())
}

/// `report_issues` の件数を数え直す（`file_name` / `content_hash` は保つ）。
fn recount_check(conn: &Connection, report_id: &str) -> Result<(), ReportError> {
    let (file_name, content_hash): (Option<String>, Option<String>) = conn.query_row(
        "SELECT file_name, content_hash FROM reports WHERE id = ?1",
        params![report_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    write_check(
        conn,
        report_id,
        file_name.as_deref(),
        content_hash.as_deref(),
    )
}

/// 出典（`paper` / `external`）を入れ直す。
fn insert_sources(
    conn: &Connection,
    report_id: &str,
    sources: &[ReportSource],
) -> Result<(), ReportError> {
    let mut stmt = conn.prepare(
        "INSERT OR IGNORE INTO report_sources (report_id, kind, ref_id, role)
         VALUES (?1, ?2, ?3, ?4)",
    )?;
    for source in sources {
        stmt.execute(params![
            report_id,
            source.kind,
            source.ref_id,
            source.role
        ])?;
    }
    Ok(())
}

/// `(run_id, file_name)` の report の `(id, content_hash, md_path)` を返す。
fn find_existing(
    conn: &Connection,
    run_id: &str,
    file_name: &str,
) -> Result<Option<(String, Option<String>, String)>, ReportError> {
    let mut stmt = conn.prepare(
        "SELECT id, content_hash, md_path FROM reports WHERE run_id = ?1 AND file_name = ?2",
    )?;
    let row = stmt
        .query_row(params![run_id, file_name], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })
        .optional()?;
    Ok(row)
}

/// 成果物のフォルダを読んで report を自動登録する。
///
/// 1 回の scan は 1 つのトランザクションにまとめない（report ごとに確定する。
/// 途中で失敗しても前の分は残る）。
///
/// # Errors
///
/// SQLite / I/O エラー。
pub fn scan(conn: &Connection, root: &Path) -> Result<ScanReport, ReportError> {
    let mut summary = ScanReport::default();
    let mut research_cache: HashMap<String, bool> = HashMap::new();

    for run in crate::runs::list_run_dirs(root) {
        let Some(block_id) = crate::runs::run_block_id(root, &run) else {
            summary.skipped_dev += 1;
            continue;
        };
        let research = *research_cache
            .entry(block_id.clone())
            .or_insert_with(|| crate::runs::is_research_block(root, &block_id));
        if !research {
            summary.skipped_dev += 1;
            continue;
        }

        let run_id = run.run_id.to_string();
        let Ok(entries) = std::fs::read_dir(&run.path) else {
            continue;
        };
        let mut files: Vec<PathBuf> = entries
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| path.is_file())
            .collect();
        files.sort();

        for path in files {
            let Some(file_name) = path
                .file_name()
                .map(|name| name.to_string_lossy().to_string())
            else {
                continue;
            };
            if !file_name.to_ascii_lowercase().ends_with(".md") || is_excluded_md(&file_name) {
                continue;
            }
            let Ok(markdown) = std::fs::read_to_string(&path) else {
                continue;
            };
            let content_hash = sha256_hex(&markdown);
            let md_path = path.to_string_lossy().to_string();

            match find_existing(conn, &run_id, &file_name)? {
                None => {
                    let citations = parse_citations(&markdown);
                    let issues = collect_issues(conn, &markdown, &block_id)?;
                    let new = NewReport {
                        title: None,
                        md_path: md_path.clone(),
                        run_id: Some(run_id.clone()),
                        block: Some(block_id.clone()),
                        sources: sources_from_citations(&citations),
                        summary: None,
                    };
                    let report_id = match create_report(conn, &new) {
                        Ok(id) => id,
                        Err(error) => {
                            // 同時に登録が走った場合は、既にあれば二重に数えない。
                            if find_existing(conn, &run_id, &file_name)?.is_some() {
                                summary.unchanged += 1;
                                continue;
                            }
                            return Err(error);
                        }
                    };
                    insert_issues(conn, &report_id, &issues)?;
                    if let Err(error) =
                        write_check(conn, &report_id, Some(&file_name), Some(&content_hash))
                    {
                        // file_name の UNIQUE 違反（同時登録）。既にあればこの行を消す。
                        if find_existing(conn, &run_id, &file_name)?.is_some() {
                            conn.execute("DELETE FROM reports WHERE id = ?1", params![report_id])?;
                            summary.unchanged += 1;
                            continue;
                        }
                        return Err(error);
                    }
                    summary.added += 1;
                }
                Some((report_id, old_hash, old_path)) => {
                    let changed = old_hash.as_deref() != Some(content_hash.as_str())
                        || old_path != md_path;
                    if changed {
                        conn.execute(
                            "DELETE FROM report_sources
                             WHERE report_id = ?1 AND kind IN ('paper', 'external')",
                            params![report_id],
                        )?;
                        // この作業が入れる問題だけを消して入れ直す。
                        // C04 の `unresolved` / `mismatch`、C06 の `retracted` は残す。
                        conn.execute(
                            "DELETE FROM report_issues WHERE report_id = ?1
                             AND kind IN ('header', 'missing_paper', 'unsupported_number', 'style')",
                            params![report_id],
                        )?;
                        let citations = parse_citations(&markdown);
                        insert_sources(conn, &report_id, &sources_from_citations(&citations))?;
                        let issues = collect_issues(conn, &markdown, &block_id)?;
                        insert_issues(conn, &report_id, &issues)?;
                        write_check(conn, &report_id, Some(&file_name), Some(&content_hash))?;
                        conn.execute(
                            "UPDATE reports SET md_path = ?1, block = ?2 WHERE id = ?3",
                            params![md_path, block_id, report_id],
                        )?;
                        summary.updated += 1;
                    } else {
                        // 中身は同じでも、他の作業が入れた問題を拾って件数を数え直す。
                        write_check(conn, &report_id, Some(&file_name), Some(&content_hash))?;
                        summary.unchanged += 1;
                    }
                }
            }
        }
    }

    summary.header_errors = conn.query_row(
        "SELECT COUNT(*) FROM reports WHERE header_ok = 0",
        [],
        |row| row.get::<_, i64>(0),
    )? as usize;
    // 撤回された論文を引用した report の印を入れ直す（migration 0041 の列があるときだけ）。
    refresh_retracted_issues(conn)?;
    Ok(summary)
}

/// report の検査結果（`report_issues`）を返す。
///
/// # Errors
///
/// SQLite エラー。
pub fn report_issues(conn: &Connection, report_id: &str) -> Result<Vec<Issue>, ReportError> {
    let mut stmt = conn.prepare(
        "SELECT level, kind, detail, line FROM report_issues
         WHERE report_id = ?1 ORDER BY level DESC, kind, line",
    )?;
    let rows = stmt
        .query_map(params![report_id], |row| {
            Ok(Issue {
                level: row.get(0)?,
                kind: row.get(1)?,
                detail: row.get(2)?,
                line: row.get(3)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// report の検査の状態（`report scan` が入れる列）を返す。
///
/// # Errors
///
/// SQLite エラー。
pub fn report_check(
    conn: &Connection,
    report_id: &str,
) -> Result<Option<ReportCheck>, ReportError> {
    let row = conn
        .query_row(
            "SELECT header_ok, checked_at, check_errors, check_warnings\n             FROM reports WHERE id = ?1",
            params![report_id],
            |row| {
                Ok(ReportCheck {
                    header_ok: row.get(0)?,
                    checked_at: row.get(1)?,
                    check_errors: row.get(2)?,
                    check_warnings: row.get(3)?,
                })
            },
        )
        .optional()?;
    Ok(row)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE papers (id INTEGER PRIMARY KEY, title TEXT,
                 is_retracted INTEGER, retraction_checked_at TEXT);",
        )
        .unwrap();
        conn.execute_batch(include_str!("../migrations/0028_reports.sql"))
            .unwrap();
        conn.execute_batch(include_str!("../migrations/0039_report_scan.sql"))
            .unwrap();
        conn
    }

    fn write_md(dir: &Path, name: &str, body: &str) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, body).unwrap();
        path
    }

    #[test]
    fn summary_skips_frontmatter_and_blank_lines() {
        let md = "---\nreport_id: R-0001\n---\n# Title\n\nFirst line.\nSecond line.\nThird line.\nFourth.";
        assert_eq!(
            summary_from_markdown(md, 3),
            "Title / First line. / Second line."
        );
        assert_eq!(title_from_markdown(md), Some("Title".to_string()));
    }

    #[test]
    fn create_and_read_report_with_sources() -> Result<(), Box<dyn std::error::Error>> {
        let conn = setup();
        conn.execute("INSERT INTO papers (id, title) VALUES (1, 'Paper A')", [])
            .unwrap();
        let dir = std::env::temp_dir().join(format!(
            "sai_report_test_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir)?;
        let md = write_md(&dir, "report.md", "# My Report\n\nBody text.");

        let id = create_report(
            &conn,
            &NewReport {
                md_path: md.to_string_lossy().to_string(),
                run_id: Some("42".into()),
                block: Some("konsaru".into()),
                sources: vec![
                    ReportSource {
                        kind: "paper".into(),
                        ref_id: "1".into(),
                        role: None,
                    },
                    ReportSource {
                        kind: "chunk".into(),
                        ref_id: "88421".into(),
                        role: None,
                    },
                ],
                ..Default::default()
            },
        )?;
        assert_eq!(id, "R-0001");
        // 2 件目は連番
        let id2 = create_report(
            &conn,
            &NewReport {
                md_path: md.to_string_lossy().to_string(),
                ..Default::default()
            },
        )?;
        assert_eq!(id2, "R-0002");

        let report = get_report(&conn, "R-0001")?.expect("exists");
        assert_eq!(report.title.as_deref(), Some("My Report"));
        assert_eq!(report.run_id.as_deref(), Some("42"));
        assert_eq!(report.summary.as_deref(), Some("My Report / Body text."));

        let sources = report_sources(&conn, "R-0001")?;
        assert_eq!(sources.len(), 2);
        assert_eq!(reports_for_paper(&conn, 1)?.len(), 1);
        assert_eq!(list_reports(&conn, None)?.len(), 2);
        assert_eq!(
            read_report_markdown(&conn, "R-0001")?,
            "# My Report\n\nBody text."
        );

        let links = paper_report_links(&conn)?;
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].0, 1);
        assert_eq!(links[0].2.id, "R-0001");
        Ok(())
    }

    #[test]
    fn resolve_md_path_falls_back_to_run_folder() -> Result<(), Box<dyn std::error::Error>> {
        let report = Report {
            id: "R-0001".into(),
            md_path: "/nonexistent/old/location/report.md".into(),
            run_id: Some("0042".into()),
            ..Default::default()
        };
        // 実在しないので None（project_root が無い環境でも panic しない）
        assert!(resolve_md_path(&report).is_none());
        Ok(())
    }

    #[test]
    fn parse_citations_reads_contract_forms() {
        let md = "# report\n\n場所付き [#37 §3.2] と複数 [#37, #40]。外部 [DOI:10.1/x] と [arXiv:2401.12345] と [PMID:12345678]。未確認 [未確認: 著者 年 タイトル]。番号 [#370]。\n\n```\nコード [#1]\n```\n\nインライン `[#2]` は拾わない。\n";
        let citations = parse_citations(md);
        let papers: Vec<(i64, Option<String>)> = citations
            .iter()
            .filter_map(|citation| match &citation.target {
                CitationTarget::Paper(number) => Some((*number, citation.locator.clone())),
                _ => None,
            })
            .collect();
        assert_eq!(
            papers,
            vec![
                (37, Some("§3.2".to_string())),
                (37, None),
                (40, None),
                (370, None),
            ]
        );
        assert!(citations.iter().any(|c| c.target
            == CitationTarget::External {
                scheme: "DOI".into(),
                id: "10.1/x".into()
            }));
        assert!(citations.iter().any(|c| c.target
            == CitationTarget::External {
                scheme: "arXiv".into(),
                id: "2401.12345".into()
            }));
        assert!(citations.iter().any(|c| c.target
            == CitationTarget::External {
                scheme: "PMID".into(),
                id: "12345678".into()
            }));
        assert!(
            citations
                .iter()
                .any(|c| c.target == CitationTarget::Unverified("著者 年 タイトル".into()))
        );
        // コード欄とインラインコードの中は拾わない
        assert!(
            !citations
                .iter()
                .any(|c| matches!(c.target, CitationTarget::Paper(1 | 2)))
        );
    }

    #[test]
    fn check_header_reports_violations() {
        // 正しい
        let ok = "---\nkind: 回答\nblock: paper-critic\npapers: [37, 40]\n---\n\n本文 [#37] [#40]。\n";
        assert!(check_header(ok, Some("paper-critic")).is_empty());

        // frontmatter なし
        let issues = check_header("本文だけ\n", Some("paper-critic"));
        assert_eq!(issues.len(), 1);
        assert!(issues[0].detail.contains("先頭情報"));

        // kind が空
        let issues = check_header(
            "---\nkind:\nblock: paper-critic\npapers: []\n---\n",
            Some("paper-critic"),
        );
        assert!(issues.iter().any(|issue| issue.detail == "kind が空です"));

        // block 違い
        let issues = check_header(
            "---\nkind: 回答\nblock: paper-map\npapers: []\n---\n",
            Some("paper-critic"),
        );
        assert!(issues.iter().any(|issue| issue
            .detail
            .contains("block が実行のブロック（paper-critic）と違います: paper-map")));

        // papers が数でない
        let issues = check_header(
            "---\nkind: 回答\nblock: paper-critic\npapers: [37, abc]\n---\n",
            Some("paper-critic"),
        );
        assert!(
            issues
                .iter()
                .any(|issue| issue.detail == "papers が数の配列ではありません")
        );

        // 本文と papers の食い違い（warning）
        let issues = check_header(
            "---\nkind: 回答\nblock: paper-critic\npapers: [37]\n---\n\n本文 [#40]。\n",
            Some("paper-critic"),
        );
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].level, "warning");
        assert!(issues[0].detail.contains("#40"));
    }

    fn temp_root(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "sai-paper-reports-{}-{}-{:?}",
            tag,
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_file(path: PathBuf, text: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    /// 研究用ブロック `paper-search` と実行フォルダ `running/<run_id>` を持つ根を作る。
    fn research_root(tag: &str, run_id: &str) -> PathBuf {
        let root = temp_root(tag);
        write_file(
            root.join(".sai/blocks/paper-search.md"),
            "---\nname: paper-search\ntemplates: project-brief,research-core\n---\n",
        );
        write_file(
            root.join(format!(".sai/outputs/running/{run_id}/task.md")),
            &format!("---\nrun_id: {run_id}\nblock: paper-search\n---\n"),
        );
        root
    }

    #[test]
    fn unsupported_numbers_picks_only_unsupported() {
        let md = r"---
kind: 回答
block: paper-critic
papers: [37]
---

根拠の無い 85% の文。
根拠のある 85% の文 [#37]。
日付は 2026-09-24 と 2023 年、章は §3.2 と 3.2 節、番号は #37。
0.87 は根拠なし。
```
コード 0.99
```

[#37]

| 精度 | 値 |
|---|---|
| A | 0.87 |

## 引き継ぎ
引き継ぎの 0.5。

## 結果

| B | 0.93 |

| C | 0.95 |

[#37]
";
        let found = unsupported_numbers(md);
        let lines: Vec<usize> = found.iter().map(|(line, _)| *line).collect();
        assert_eq!(lines, vec![7, 10, 26], "拾う行: {found:?}");
        assert_eq!(found[0].1, "根拠の無い 85% の文。");
        assert_eq!(found[1].1, "0.87 は根拠なし。");
        assert_eq!(found[2].1, "| B | 0.93 |");
        // 引用のある文、日付・章番号・Library の番号、コード欄、引き継ぎ節は拾わない
        assert!(!found.iter().any(|(_, head)| head.contains("根拠のある")));
        assert!(!found.iter().any(|(_, head)| head.contains("日付は")));
        assert!(!found.iter().any(|(_, head)| head.contains("コード")));
        assert!(!found.iter().any(|(_, head)| head.contains("引き継ぎ")));
        // 表の前後に引用があれば、その表の数字は拾わない
        assert!(!found.iter().any(|(_, head)| head.contains("| A |")));
        assert!(!found.iter().any(|(_, head)| head.contains("| C |")));
    }

    #[test]
    fn author_year_style_is_counted() {
        let md = "Smith 2020 の結果。また (Smith et al., 2020) と Tanaka ら (2021)。\n";
        let issues = style_issues(md);
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].level, "warning");
        assert_eq!(issues[0].kind, "style");
        assert!(issues[0].detail.ends_with("3 件"), "{}", issues[0].detail);
        // 決まった形（`[#37]`）だけなら style は出ない
        assert!(style_issues("Smith の結果 [#37]。\n").is_empty());
    }

    #[test]
    fn refresh_retracted_issues_marks_and_clears() -> Result<(), Box<dyn std::error::Error>> {
        let conn = setup();
        conn.execute(
            "INSERT INTO papers (id, title, is_retracted) VALUES (1, 'A', 1)",
            [],
        )?;
        conn.execute(
            "INSERT INTO papers (id, title, is_retracted) VALUES (2, 'B', 0)",
            [],
        )?;
        conn.execute(
            "INSERT INTO papers (id, title, is_retracted) VALUES (3, 'C', NULL)",
            [],
        )?;
        // R-0001 は #1（撤回）と #2（撤回されていない）を引用する。
        let report_id = create_report(
            &conn,
            &NewReport {
                md_path: "/tmp/retracted-test.md".into(),
                sources: vec![
                    ReportSource {
                        kind: "paper".into(),
                        ref_id: "1".into(),
                        role: None,
                    },
                    ReportSource {
                        kind: "paper".into(),
                        ref_id: "2".into(),
                        role: None,
                    },
                ],
                ..Default::default()
            },
        )?;

        // 撤回された論文を引用した report に error が 1 つ付く。
        assert_eq!(refresh_retracted_issues(&conn)?, 1);
        let issues = report_issues(&conn, &report_id)?;
        let retracted: Vec<&Issue> = issues
            .iter()
            .filter(|issue| issue.kind == "retracted")
            .collect();
        assert_eq!(retracted.len(), 1);
        assert_eq!(retracted[0].level, "error");
        assert_eq!(retracted[0].detail, "撤回された論文 #1 を引用しています");
        let check = report_check(&conn, &report_id)?.expect("検査結果");
        assert_eq!(check.check_errors, 1);

        // 2 回呼んでも重ならない。
        assert_eq!(refresh_retracted_issues(&conn)?, 1);
        assert_eq!(
            report_issues(&conn, &report_id)?
                .iter()
                .filter(|issue| issue.kind == "retracted")
                .count(),
            1
        );
        let check = report_check(&conn, &report_id)?.expect("検査結果");
        assert_eq!(check.check_errors, 1);

        // 撤回でなくなると消える。
        conn.execute("UPDATE papers SET is_retracted = 0 WHERE id = 1", [])?;
        assert_eq!(refresh_retracted_issues(&conn)?, 0);
        assert!(
            !report_issues(&conn, &report_id)?
                .iter()
                .any(|issue| issue.kind == "retracted")
        );
        let check = report_check(&conn, &report_id)?.expect("検査結果");
        assert_eq!(check.check_errors, 0);
        Ok(())
    }

    #[test]
    fn scan_flags_missing_paper() -> Result<(), Box<dyn std::error::Error>> {
        let root = research_root("missing-paper", "10");
        write_file(
            root.join(".sai/outputs/running/10/answer.md"),
            "---\nkind: 回答\nblock: paper-search\npapers: [1, 3]\n---\n\n本文 [#1] と [#3] と [#3]。\n",
        );
        let conn = setup();
        conn.execute("INSERT INTO papers (id, title) VALUES (1, 'A')", [])?;
        conn.execute("INSERT INTO papers (id, title) VALUES (2, 'B')", [])?;
        let summary = scan(&conn, &root)?;
        assert_eq!(summary.added, 1);
        let report_id: String = conn.query_row(
            "SELECT id FROM reports WHERE run_id = '10' AND file_name = 'answer.md'",
            [],
            |row| row.get(0),
        )?;
        let issues = report_issues(&conn, &report_id)?;
        let missing: Vec<&Issue> = issues
            .iter()
            .filter(|issue| issue.kind == "missing_paper")
            .collect();
        assert!(!missing.is_empty());
        assert!(missing.iter().all(|issue| issue.detail.contains("#3")));
        assert!(!missing.iter().any(|issue| issue.detail.contains("#1")));
        assert!(missing.iter().all(|issue| issue.level == "error"));
        let check = report_check(&conn, &report_id)?.expect("検査結果");
        assert!(check.check_errors >= 1);
        // 先頭情報は正しいので header_ok は保つ
        assert!(check.header_ok);
        Ok(())
    }

    #[test]
    fn rescan_keeps_other_issue_kinds() -> Result<(), Box<dyn std::error::Error>> {
        let root = research_root("rescan-keep", "11");
        let md_path = root.join(".sai/outputs/running/11/answer.md");
        write_file(
            md_path.clone(),
            "---\nkind: 回答\nblock: paper-search\npapers: [3]\n---\n\n本文 [#3]。\n",
        );
        let conn = setup();
        conn.execute("INSERT INTO papers (id, title) VALUES (1, 'A')", [])?;
        let summary = scan(&conn, &root)?;
        assert_eq!(summary.added, 1);
        let report_id: String = conn.query_row(
            "SELECT id FROM reports WHERE run_id = '11' AND file_name = 'answer.md'",
            [],
            |row| row.get(0),
        )?;
        assert!(
            report_issues(&conn, &report_id)?
                .iter()
                .any(|issue| issue.kind == "missing_paper")
        );

        // 別の作業（C04）が入れる問題を模す。
        // C06 の `retracted` は scan の最後の `refresh_retracted_issues` が入れ直すので、
        // ここでは手で入れない（`refresh_retracted_issues_marks_and_clears` で確かめる）。
        for (level, kind) in [("error", "unresolved"), ("warning", "mismatch")] {
            conn.execute(
                "INSERT INTO report_issues (report_id, level, kind, detail, line)
                 VALUES (?1, ?2, ?3, ?4, NULL)",
                params![report_id, level, kind, "他の作業の問題"],
            )?;
        }

        // 中身が同じでも、他の作業が入れた問題を拾って件数を数え直す
        let same = scan(&conn, &root)?;
        assert_eq!(same.unchanged, 1);
        let check = report_check(&conn, &report_id)?.expect("検査結果");
        assert_eq!(check.check_errors, 2, "missing_paper と unresolved");
        assert_eq!(check.check_warnings, 1, "mismatch");

        // 中身を変えて読み直させる（[#3] → [#1]）
        write_file(
            md_path.clone(),
            "---\nkind: 回答\nblock: paper-search\npapers: [1]\n---\n\n本文 [#1]。\n",
        );
        let re = scan(&conn, &root)?;
        assert_eq!(re.updated, 1);
        let issues = report_issues(&conn, &report_id)?;
        for kind in ["unresolved", "mismatch"] {
            assert!(issues.iter().any(|issue| issue.kind == kind), "{kind} は残る");
        }
        assert!(
            !issues.iter().any(|issue| issue.kind == "missing_paper"),
            "読み直しで消える"
        );
        let check = report_check(&conn, &report_id)?.expect("検査結果");
        assert_eq!(check.check_errors, 1, "unresolved の error だけ");
        assert_eq!(check.check_warnings, 1, "mismatch");

        // 共有契約の API
        let counts = issue_counts(&conn, &report_id)?;
        assert_eq!(counts.errors, 1);
        assert_eq!(counts.warnings, 1);
        let listed = list_issues(&conn, Some(&report_id))?;
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].issues.len(), 2);
        let total = total_issue_counts(&conn)?;
        assert_eq!(total.errors, 1);
        assert_eq!(total.warnings, 1);
        assert_eq!(list_issues(&conn, None)?.len(), 1);
        Ok(())
    }

    #[test]
    fn scan_registers_updates_and_skips() -> Result<(), Box<dyn std::error::Error>> {
        let root = temp_root("scan");
        // 研究用ブロックと開発用ブロックの定義
        write_file(
            root.join(".sai/blocks/paper-search.md"),
            "---\nname: paper-search\ntemplates: project-brief,research-core\n---\n",
        );
        write_file(
            root.join(".sai/blocks/main.md"),
            "---\nname: main\ntemplates: sai-link,sai-docs\n---\n",
        );

        // 研究用の実行（task.md で block が分かる）
        write_file(
            root.join(".sai/outputs/running/5/task.md"),
            "---\nrun_id: 5\nblock: paper-search\n---\n",
        );
        write_file(
            root.join(".sai/outputs/running/5/paper-map-a.md"),
            "---\nkind: 分野の地図\nblock: paper-search\npapers: [37, 40]\n---\n\n本文 [#37] と [#40]。\n",
        );
        // 先頭情報の無い md も登録する
        write_file(root.join(".sai/outputs/running/5/note.md"), "# メモ\n\n本文。\n");
        // task.md / message.md は登録しない
        write_file(root.join(".sai/outputs/running/5/message.md"), "連絡\n");

        // 開発用の実行
        write_file(
            root.join(".sai/outputs/todo/6/task.md"),
            "---\nrun_id: 6\nblock: main\n---\n",
        );
        write_file(root.join(".sai/outputs/todo/6/dev.md"), "# 開発\n");

        let conn = setup();
        let first = scan(&conn, &root)?;
        assert_eq!(first.added, 2, "研究用の 2 件だけ登録");
        assert_eq!(first.skipped_dev, 1);
        assert_eq!(first.header_errors, 1);

        // 二重にならない
        let second = scan(&conn, &root)?;
        assert_eq!(second.added, 0);
        assert_eq!(second.unchanged, 2);

        let reports = list_reports(&conn, None)?;
        assert_eq!(reports.len(), 2);
        let map_id: String = conn.query_row(
            "SELECT id FROM reports WHERE run_id = '5' AND file_name = 'paper-map-a.md'",
            [],
            |row| row.get(0),
        )?;
        let note_id: String = conn.query_row(
            "SELECT id FROM reports WHERE run_id = '5' AND file_name = 'note.md'",
            [],
            |row| row.get(0),
        )?;
        let map_check = report_check(&conn, &map_id)?.expect("paper-map-a の検査結果");
        assert!(map_check.header_ok, "先頭情報は正しい");
        assert_eq!(report_sources(&conn, &map_id)?.len(), 2);
        let note_check = report_check(&conn, &note_id)?.expect("note の検査結果");
        assert!(!note_check.header_ok, "先頭情報の無い md は header_ok = 0");
        assert!(
            report_issues(&conn, &note_id)?
                .iter()
                .any(|issue| issue.kind == "header")
        );

        // フォルダが running → R3 に移っても同じ report
        std::fs::create_dir_all(root.join(".sai/outputs/R3"))?;
        std::fs::rename(
            root.join(".sai/outputs/running/5"),
            root.join(".sai/outputs/R3/5"),
        )?;
        let third = scan(&conn, &root)?;
        assert_eq!(third.added, 0);
        assert_eq!(third.updated, 2);
        let moved = get_report(&conn, &map_id)?.expect("同じ report");
        assert!(moved.md_path.ends_with("R3/5/paper-map-a.md"));

        // 中身が変わると出典を読み直す
        write_file(
            root.join(".sai/outputs/R3/5/paper-map-a.md"),
            "---\nkind: 分野の地図\nblock: paper-search\npapers: [37]\n---\n\n本文 [#37]。\n",
        );
        let fourth = scan(&conn, &root)?;
        assert_eq!(fourth.updated, 1);
        assert_eq!(report_sources(&conn, &map_id)?.len(), 1);
        Ok(())
    }
}
