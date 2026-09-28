//! 外部の引用（Library に無い論文）を公開の API で照合する。
//!
//! `sai-moon-paper report check <id>` / `sai-moon-paper report check --all` を実行したときだけ
//! ネットにつなぐ。LLM は使わず、Crossref・arXiv・PubMed の応答だけを見る。
//!
//! 照合の結果は `citation_checks` に残す。`found` / `not_found` は 30 日の間は
//! 問い合わせ直さない（API の制限に当たらないようにする）。`offline` / `error` は
//! 次に実行したときにまた問う。ネットにつながらないときは「未確認（オフライン）」と
//! して残し、黙って飛ばさない。

use std::sync::{Arc, LazyLock};
use std::time::Duration;

use chrono::Duration as ChronoDuration;
use chrono::{DateTime, Datelike, Utc};
use regex::Regex;
use rusqlite::{Connection, OptionalExtension, params};

use crate::api::arxiv::ArxivClient;
use crate::api::crossref::CrossrefClient;
use crate::api::pubmed::PubmedClient;
use crate::models::RemotePaper;
use crate::reports::{self, CitationTarget, Issue, ReportError};

/// 1 件の問い合わせの制限時間。
const QUERY_TIMEOUT: Duration = Duration::from_secs(15);

/// 照合の結果を残しておく期間。この間は `found` / `not_found` を問い合わせ直さない。
const CACHE_TTL: ChronoDuration = ChronoDuration::days(30);

/// 1 つの識別子を引いた結果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolveOutcome {
    /// 見つかった。
    Found {
        /// 取得した表題。
        title: String,
        /// 取得した出版年。
        year: Option<i32>,
        /// 取得した第一著者の姓。
        first_author: Option<String>,
    },
    /// 見つからない。
    NotFound,
    /// ネットにつながらない（接続の失敗・タイムアウト）。
    Offline,
    /// 応答は来たが解析できないなど、照合できなかった。
    Error(String),
}

/// 識別子を 1 件引く照合器。
///
/// テストで本物の API を呼ばないよう、問い合わせの部分をこの trait に切り出す。
#[async_trait::async_trait]
pub trait Resolver: Send + Sync {
    /// `scheme`（`DOI` / `arXiv` / `PMID`）と `id` を引く。
    async fn resolve(&self, scheme: &str, id: &str) -> ResolveOutcome;
}

/// 本物の公開の API を使う照合器。
pub struct ApiResolver {
    crossref: CrossrefClient,
    arxiv: Option<ArxivClient>,
    pubmed: Option<PubmedClient>,
}

impl ApiResolver {
    fn new() -> Self {
        Self {
            crossref: CrossrefClient::new(),
            arxiv: ArxivClient::new().ok(),
            pubmed: PubmedClient::new(std::env::var("NCBI_API_KEY").ok()).ok(),
        }
    }
}

impl Default for ApiResolver {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl Resolver for ApiResolver {
    async fn resolve(&self, scheme: &str, id: &str) -> ResolveOutcome {
        let future = async {
            match scheme {
                "DOI" => match self.crossref.get_by_doi(id).await {
                    Ok(Some(paper)) => found_from_paper(&paper, scheme),
                    Ok(None) => ResolveOutcome::NotFound,
                    Err(error) => crossref_failure(&error),
                },
                "arXiv" => match &self.arxiv {
                    Some(client) => match client.get_by_id(id).await {
                        Ok(Some(paper)) => found_from_paper(&paper, scheme),
                        Ok(None) => ResolveOutcome::NotFound,
                        Err(error) => arxiv_failure(&error),
                    },
                    None => ResolveOutcome::Error("arXiv クライアントを作れません".into()),
                },
                "PMID" => match &self.pubmed {
                    Some(client) => match client.get_by_pmid(id).await {
                        Ok(Some(paper)) => found_from_paper(&paper, scheme),
                        Ok(None) => ResolveOutcome::NotFound,
                        Err(error) => pubmed_failure(&error),
                    },
                    None => ResolveOutcome::Error("PubMed クライアントを作れません".into()),
                },
                other => ResolveOutcome::Error(format!("未対応の scheme: {other}")),
            }
        };
        match tokio::time::timeout(QUERY_TIMEOUT, future).await {
            Ok(outcome) => outcome,
            Err(_) => ResolveOutcome::Offline,
        }
    }
}

/// `RemotePaper` から照合の結果を作る。
fn found_from_paper(paper: &RemotePaper, scheme: &str) -> ResolveOutcome {
    ResolveOutcome::Found {
        title: paper.title.clone(),
        year: Some(paper.published.year()),
        first_author: first_author_surname(&paper.authors, scheme),
    }
}

/// 第一著者の姓を取り出す。
///
/// Crossref・arXiv の著者名は「名 姓」、PubMed は「姓 名」の順である
/// （`crates/sai-paper-core/src/api/pubmed.rs` の `efetch` を参照）。
fn first_author_surname(authors: &[String], scheme: &str) -> Option<String> {
    let first = authors.first()?;
    let surname = if scheme == "PMID" {
        first.split_whitespace().next()
    } else {
        first.split_whitespace().last()
    };
    surname.map(str::to_string)
}

/// ネットにつながらない（接続の失敗・タイムアウト）かを返す。
fn is_offline(error: &reqwest::Error) -> bool {
    error.is_timeout() || error.is_connect()
}

fn crossref_failure(error: &crate::api::crossref::CrossrefError) -> ResolveOutcome {
    match error {
        crate::api::crossref::CrossrefError::NotFound => ResolveOutcome::NotFound,
        crate::api::crossref::CrossrefError::Request(inner) if is_offline(inner) => {
            ResolveOutcome::Offline
        }
        other => ResolveOutcome::Error(other.to_string()),
    }
}

fn arxiv_failure(error: &crate::api::arxiv::ArxivError) -> ResolveOutcome {
    match error {
        crate::api::arxiv::ArxivError::Request(inner) if is_offline(inner) => {
            ResolveOutcome::Offline
        }
        other => ResolveOutcome::Error(other.to_string()),
    }
}

fn pubmed_failure(error: &crate::api::pubmed::PubmedError) -> ResolveOutcome {
    match error {
        crate::api::pubmed::PubmedError::NotFound => ResolveOutcome::NotFound,
        crate::api::pubmed::PubmedError::Request(inner) if is_offline(inner) => {
            ResolveOutcome::Offline
        }
        other => ResolveOutcome::Error(other.to_string()),
    }
}

/// 照合の進め方の指定。
#[derive(Clone)]
pub struct CheckOptions {
    /// 1 件ごとの待ち時間（既定 1 秒）。
    pub interval: Duration,
    /// 問い合わせに使う照合器。
    pub resolver: Arc<dyn Resolver>,
}

impl Default for CheckOptions {
    fn default() -> Self {
        Self {
            interval: Duration::from_secs(1),
            resolver: Arc::new(ApiResolver::new()),
        }
    }
}

/// 1 つの report の照合の結果。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CheckSummary {
    /// 外部の引用（重複を除いた指し先）の数。
    pub total: usize,
    /// 見つかった数。
    pub found: usize,
    /// 見つからなかった数。
    pub not_found: usize,
    /// ネットにつながらず未確認の数。
    pub offline: usize,
}

/// 1 つの report の外部の引用を照合し、結果を DB に残す。
///
/// 1. report の md を読み、外部の引用（`External`）を集める。
/// 2. `citation_checks` を見て、30 日以内の `found` / `not_found` はそれを使う。
/// 3. 残りを順に問い合わせる（1 件ごとに `opts.interval` 待つ）。
/// 4. `report_issues` の `kind IN ('unresolved', 'mismatch')` を消して入れ直す。
///
/// # Errors
///
/// report や md が見つからないとき、SQLite / I/O の失敗。
pub async fn check_report(
    conn: &Connection,
    report_id: &str,
    opts: &CheckOptions,
) -> Result<CheckSummary, ReportError> {
    let report = reports::get_report(conn, report_id)?
        .ok_or_else(|| ReportError::NotFound(report_id.to_string()))?;
    let path = reports::resolve_md_path(&report).ok_or_else(|| {
        ReportError::MarkdownNotFound(format!("{} (report_id={})", report.md_path, report.id))
    })?;
    let markdown = std::fs::read_to_string(&path)?;
    let citations = reports::parse_citations(&markdown);

    // 外部の引用の指し先（出現順、重複を除く）。
    let mut refs: Vec<(String, String)> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for citation in &citations {
        if let CitationTarget::External { scheme, id } = &citation.target {
            let key = format!("{scheme}:{id}");
            if seen.insert(key) {
                refs.push((scheme.clone(), id.clone()));
            }
        }
    }

    // 1. 30 日以内の結果は使う。
    let now = Utc::now();
    let mut outcomes: std::collections::HashMap<String, ResolveOutcome> =
        std::collections::HashMap::new();
    for (scheme, id) in &refs {
        let key = format!("{scheme}:{id}");
        if let Some(outcome) = cached_outcome(conn, &key, now)? {
            outcomes.insert(key, outcome);
        }
    }

    // 2. 残りを問い合わせる。
    let mut started = false;
    for (scheme, id) in &refs {
        let key = format!("{scheme}:{id}");
        if outcomes.contains_key(&key) {
            continue;
        }
        if started && !opts.interval.is_zero() {
            tokio::time::sleep(opts.interval).await;
        }
        started = true;
        let outcome =
            match tokio::time::timeout(QUERY_TIMEOUT, opts.resolver.resolve(scheme, id)).await {
                Ok(outcome) => outcome,
                Err(_) => ResolveOutcome::Offline,
            };
        write_cache(conn, &key, &outcome, now)?;
        outcomes.insert(key, outcome);
    }

    // 3. 問題を入れ直す。
    conn.execute(
        "DELETE FROM report_issues WHERE report_id = ?1 AND kind IN ('unresolved', 'mismatch')",
        params![report_id],
    )?;
    let mut issues = Vec::new();
    for citation in &citations {
        let CitationTarget::External { scheme, id } = &citation.target else {
            continue;
        };
        let key = format!("{scheme}:{id}");
        let Some(outcome) = outcomes.get(&key) else {
            continue;
        };
        match outcome {
            ResolveOutcome::NotFound => issues.push(Issue {
                level: "error".into(),
                kind: "unresolved".into(),
                detail: format!("{key} が見つかりません"),
                line: Some(citation.line as i64),
            }),
            ResolveOutcome::Offline => issues.push(Issue {
                level: "warning".into(),
                kind: "unresolved".into(),
                detail: format!("{key} は未確認です（オフライン）"),
                line: Some(citation.line as i64),
            }),
            ResolveOutcome::Found {
                year, first_author, ..
            } => {
                let paragraph = paragraph_for_line(&markdown, citation.line).unwrap_or_default();
                issues.extend(mismatch_issues(
                    &key,
                    citation.line,
                    &paragraph,
                    *year,
                    first_author.as_deref(),
                ));
            }
            ResolveOutcome::Error(message) => issues.push(Issue {
                level: "warning".into(),
                kind: "unresolved".into(),
                detail: format!(
                    "{key} は未確認です（照合の失敗: {}）",
                    head_chars(message, 80)
                ),
                line: Some(citation.line as i64),
            }),
        }
    }
    reports::insert_issues(conn, report_id, &issues)?;

    // 4. 件数と時刻を数え直す。
    let (file_name, content_hash): (Option<String>, Option<String>) = conn.query_row(
        "SELECT file_name, content_hash FROM reports WHERE id = ?1",
        params![report_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    reports::write_check(
        conn,
        report_id,
        file_name.as_deref(),
        content_hash.as_deref(),
    )?;

    let mut summary = CheckSummary {
        total: refs.len(),
        ..Default::default()
    };
    for (scheme, id) in &refs {
        let key = format!("{scheme}:{id}");
        match outcomes.get(&key) {
            Some(ResolveOutcome::Found { .. }) => summary.found += 1,
            Some(ResolveOutcome::NotFound) => summary.not_found += 1,
            Some(ResolveOutcome::Offline) => summary.offline += 1,
            Some(ResolveOutcome::Error(_)) | None => {}
        }
    }
    Ok(summary)
}

/// `citation_checks` から 30 日以内の結果を読む（使えるものだけ）。
fn cached_outcome(
    conn: &Connection,
    key: &str,
    now: DateTime<Utc>,
) -> Result<Option<ResolveOutcome>, ReportError> {
    let row = conn
        .query_row(
            "SELECT status, title, year, first_author, checked_at
             FROM citation_checks WHERE ref_id = ?1",
            params![key],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, Option<i32>>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, String>(4)?,
                ))
            },
        )
        .optional()?;
    let Some((status, title, year, first_author, checked_at)) = row else {
        return Ok(None);
    };
    // `offline` / `error` は次にまた問う。
    if status != "found" && status != "not_found" {
        return Ok(None);
    }
    let Ok(checked) = DateTime::parse_from_rfc3339(&checked_at) else {
        return Ok(None);
    };
    if now.signed_duration_since(checked.with_timezone(&Utc)) > CACHE_TTL {
        return Ok(None);
    }
    let outcome = match status.as_str() {
        "found" => ResolveOutcome::Found {
            title: title.unwrap_or_default(),
            year,
            first_author,
        },
        "not_found" => ResolveOutcome::NotFound,
        _ => return Ok(None),
    };
    Ok(Some(outcome))
}

/// 照合の結果を `citation_checks` に書く。
fn write_cache(
    conn: &Connection,
    key: &str,
    outcome: &ResolveOutcome,
    now: DateTime<Utc>,
) -> Result<(), ReportError> {
    let (status, title, year, first_author) = match outcome {
        ResolveOutcome::Found {
            title,
            year,
            first_author,
        } => (
            "found",
            Some(title.as_str()),
            *year,
            first_author.as_deref(),
        ),
        ResolveOutcome::NotFound => ("not_found", None, None, None),
        ResolveOutcome::Offline => ("offline", None, None, None),
        ResolveOutcome::Error(_) => ("error", None, None, None),
    };
    conn.execute(
        "INSERT INTO citation_checks (ref_id, status, title, year, first_author, checked_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT(ref_id) DO UPDATE SET
             status = excluded.status,
             title = excluded.title,
             year = excluded.year,
             first_author = excluded.first_author,
             checked_at = excluded.checked_at",
        params![key, status, title, year, first_author, now.to_rfc3339()],
    )?;
    Ok(())
}

/// 本文に書かれた年らしき 4 桁の数（`19xx` / `20xx`）。
///
/// DOI や arXiv ID の中の 4 桁（`10.2024/…` / `2401.12345`）は年とみなさない。
static YEAR_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\b(?:19|20)\d{2}\b").expect("year regex"));

/// `Smith (2019)` / `Smith et al. (2019)` / `Smith ら (2019)` / `Smith, 2019` の形。
static AUTHOR_YEAR_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"([A-Z][a-zA-Z\-']+(?:(?:\s+(?:and|&|,)\s+)[A-Z][a-zA-Z\-']+)*)(?:\s+et al\.?|\s+ら)?\s*(?:[\(（]\s*(?:19|20)\d{2}\s*[\)）]|,\s*(?:19|20)\d{2})",
    )
    .expect("author year regex")
});

/// `Smith et al.`（年を伴わない形）。
static AUTHOR_ETAL_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"([A-Z][a-zA-Z\-']+)\s+et al\.?").expect("author etal regex"));

/// 著者名の並びを分ける（`and` / `&` / `,`）。
static AUTHOR_SEP_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\s*(?:,|&|\band\b)\s*").expect("author separator regex"));

/// 先頭 `max` 字まで（文字単位。長いときは切る）。
fn head_chars(text: &str, max: usize) -> String {
    text.chars().take(max).collect()
}

/// 段落ごとの `(1 始まりの開始行, 終了行, 本文)`。
fn paragraphs(markdown: &str) -> Vec<(usize, usize, String)> {
    let lines: Vec<&str> = markdown.split('\n').collect();
    let mut result = Vec::new();
    let mut start: Option<usize> = None;
    for (index, line) in lines.iter().enumerate() {
        if line.trim().is_empty() {
            if let Some(begin) = start.take() {
                result.push((begin + 1, index, lines[begin..index].join("\n")));
            }
        } else if start.is_none() {
            start = Some(index);
        }
    }
    if let Some(begin) = start {
        result.push((begin + 1, lines.len(), lines[begin..].join("\n")));
    }
    result
}

/// 指定した行を含む段落の本文を返す。
fn paragraph_for_line(markdown: &str, line: usize) -> Option<String> {
    paragraphs(markdown)
        .into_iter()
        .find(|(start, end, _)| *start <= line && line <= *end)
        .map(|(_, _, text)| text)
}

/// 本文の中の年らしき 4 桁の数。
fn years_in(text: &str) -> Vec<i32> {
    let bytes = text.as_bytes();
    let mut years = Vec::new();
    for found in YEAR_RE.find_iter(text) {
        let start = found.start();
        let end = found.end();
        // `.` や数字に挟まれた 4 桁は年とみなさない（DOI・arXiv ID など）。
        if start > 0 && (bytes[start - 1] == b'.' || bytes[start - 1].is_ascii_digit()) {
            continue;
        }
        if end < bytes.len() && (bytes[end] == b'.' || bytes[end].is_ascii_digit()) {
            continue;
        }
        if let Ok(year) = found.as_str().parse::<i32>() {
            years.push(year);
        }
    }
    years
}

/// 本文の中の著者の姓らしき語。
fn author_names_in(text: &str) -> Vec<String> {
    let mut names = Vec::new();
    for capture in AUTHOR_YEAR_RE.captures_iter(text) {
        for name in AUTHOR_SEP_RE.split(&capture[1]) {
            let name = name.trim();
            if !name.is_empty() {
                names.push(name.to_string());
            }
        }
    }
    for capture in AUTHOR_ETAL_RE.captures_iter(text) {
        names.push(capture[1].to_string());
    }
    names
}

/// 段落の年・著者と取得した値を照らして、食い違いを `mismatch` にする。
///
/// 段落に年も著者も無ければ照らさない。
fn mismatch_issues(
    key: &str,
    line: usize,
    paragraph: &str,
    year: Option<i32>,
    first_author: Option<&str>,
) -> Vec<Issue> {
    let mut issues = Vec::new();
    let years = years_in(paragraph);
    if let Some(fetched) = year
        && !years.is_empty()
        && !years.iter().any(|value| (value - fetched).abs() <= 1)
    {
        issues.push(Issue {
            level: "warning".into(),
            kind: "mismatch".into(),
            detail: format!(
                "{key} の年が本文（{}）と違います（取得: {fetched}）",
                years[0]
            ),
            line: Some(line as i64),
        });
    }
    if let Some(fetched) = first_author {
        let names = author_names_in(paragraph);
        if !names.is_empty() && !names.iter().any(|name| name.eq_ignore_ascii_case(fetched)) {
            issues.push(Issue {
                level: "warning".into(),
                kind: "mismatch".into(),
                detail: format!(
                    "{key} の著者が本文（{}）と違います（取得: {fetched}）",
                    names[0]
                ),
                line: Some(line as i64),
            });
        }
    }
    issues
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::sync::Mutex;

    /// 本物の API を呼ばない照合器。呼ばれた回数も数える。
    struct FakeResolver {
        outcomes: HashMap<String, ResolveOutcome>,
        calls: Mutex<Vec<String>>,
    }

    impl FakeResolver {
        fn new(outcomes: HashMap<String, ResolveOutcome>) -> Self {
            Self {
                outcomes,
                calls: Mutex::new(Vec::new()),
            }
        }

        fn calls(&self) -> Vec<String> {
            self.calls.lock().unwrap().clone()
        }
    }

    #[async_trait::async_trait]
    impl Resolver for FakeResolver {
        async fn resolve(&self, scheme: &str, id: &str) -> ResolveOutcome {
            let key = format!("{scheme}:{id}");
            self.calls.lock().unwrap().push(key.clone());
            self.outcomes
                .get(&key)
                .cloned()
                .unwrap_or(ResolveOutcome::NotFound)
        }
    }

    fn setup() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE papers (id INTEGER PRIMARY KEY, title TEXT);")
            .unwrap();
        conn.execute_batch(include_str!("../migrations/0028_reports.sql"))
            .unwrap();
        conn.execute_batch(include_str!("../migrations/0039_report_scan.sql"))
            .unwrap();
        conn.execute_batch(include_str!("../migrations/0040_citation_checks.sql"))
            .unwrap();
        conn
    }

    fn temp_root(tag: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "sai_citation_check_{tag}_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    fn write_file(path: PathBuf, text: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    /// md を置いて登録する（`report add` 相当）。
    fn register(conn: &Connection, path: &std::path::Path, body: &str) -> String {
        write_file(path.to_path_buf(), body);
        reports::create_report(
            conn,
            &reports::NewReport {
                md_path: path.to_string_lossy().to_string(),
                ..Default::default()
            },
        )
        .unwrap()
    }

    fn options(outcomes: Vec<(&str, ResolveOutcome)>) -> (CheckOptions, Arc<FakeResolver>) {
        let map: HashMap<String, ResolveOutcome> = outcomes
            .into_iter()
            .map(|(key, outcome)| (key.to_string(), outcome))
            .collect();
        let resolver = Arc::new(FakeResolver::new(map));
        (
            CheckOptions {
                interval: Duration::ZERO,
                resolver: resolver.clone(),
            },
            resolver,
        )
    }

    #[tokio::test]
    async fn not_found_is_error_offline_is_warning_found_is_clean() {
        let conn = setup();
        let dir = temp_root("outcomes");
        let path = dir.join("report.md");
        let report_id = register(
            &conn,
            &path,
            "# report\n\nA [DOI:10.1/missing]。\n\nB [DOI:10.2/offline]。\n\nC [DOI:10.3/found]。\n",
        );
        let (opts, _resolver) = options(vec![
            ("DOI:10.1/missing", ResolveOutcome::NotFound),
            ("DOI:10.2/offline", ResolveOutcome::Offline),
            (
                "DOI:10.3/found",
                ResolveOutcome::Found {
                    title: "Found".into(),
                    year: Some(2020),
                    first_author: Some("Smith".into()),
                },
            ),
        ]);
        let summary = check_report(&conn, &report_id, &opts).await.unwrap();
        assert_eq!(summary.total, 3);
        assert_eq!(summary.found, 1);
        assert_eq!(summary.not_found, 1);
        assert_eq!(summary.offline, 1);

        let issues = reports::report_issues(&conn, &report_id).unwrap();
        assert!(issues.iter().any(|i| i.level == "error"
            && i.kind == "unresolved"
            && i.detail.contains("10.1/missing")));
        assert!(issues.iter().any(|i| i.level == "warning"
            && i.kind == "unresolved"
            && i.detail.contains("オフライン")));
        assert!(!issues.iter().any(|i| i.detail.contains("10.3/found")));

        let check = reports::report_check(&conn, &report_id).unwrap().unwrap();
        assert_eq!(check.check_errors, 1);
        assert_eq!(check.check_warnings, 1);
    }

    #[tokio::test]
    async fn error_is_warning_unresolved() {
        let conn = setup();
        let dir = temp_root("error");
        let path = dir.join("report.md");
        let report_id = register(&conn, &path, "# report\n\nA [DOI:10.1/error]。\n");
        let (opts, _resolver) = options(vec![(
            "DOI:10.1/error",
            ResolveOutcome::Error("HTTP 500".into()),
        )]);
        let summary = check_report(&conn, &report_id, &opts).await.unwrap();
        assert_eq!(summary.total, 1);
        assert_eq!(summary.found + summary.not_found + summary.offline, 0);

        let issues = reports::report_issues(&conn, &report_id).unwrap();
        let unresolved: Vec<&Issue> = issues.iter().filter(|i| i.kind == "unresolved").collect();
        assert_eq!(unresolved.len(), 1, "照合の失敗を黙って飛ばさない");
        assert_eq!(unresolved[0].level, "warning");
        assert!(unresolved[0].detail.contains("照合の失敗"));
        assert!(unresolved[0].detail.contains("HTTP 500"));

        let check = reports::report_check(&conn, &report_id).unwrap().unwrap();
        assert_eq!(check.check_errors, 0, "error にはしない");
        assert_eq!(check.check_warnings, 1);
    }

    #[test]
    fn head_chars_truncates_by_char() {
        assert_eq!(head_chars("short", 80), "short");
        assert_eq!(head_chars(&"あ".repeat(200), 80).chars().count(), 80);
    }

    #[tokio::test]
    async fn cache_skips_fresh_found_but_rechecks_offline() {
        let conn = setup();
        let dir = temp_root("cache");
        let path = dir.join("report.md");
        let report_id = register(
            &conn,
            &path,
            "# report\n\nA [DOI:10.1/found]。\n\nB [DOI:10.2/offline]。\n",
        );
        let (first, first_resolver) = options(vec![
            (
                "DOI:10.1/found",
                ResolveOutcome::Found {
                    title: "Found".into(),
                    year: Some(2020),
                    first_author: Some("Smith".into()),
                },
            ),
            ("DOI:10.2/offline", ResolveOutcome::Offline),
        ]);
        check_report(&conn, &report_id, &first).await.unwrap();
        assert_eq!(first_resolver.calls().len(), 2);

        // 2 回目: `found` は問い合わせず、`offline` は問い合わせ直す。
        let (second, second_resolver) = options(vec![
            (
                "DOI:10.1/found",
                ResolveOutcome::Found {
                    title: "Found".into(),
                    year: Some(2020),
                    first_author: Some("Smith".into()),
                },
            ),
            ("DOI:10.2/offline", ResolveOutcome::Offline),
        ]);
        check_report(&conn, &report_id, &second).await.unwrap();
        assert_eq!(
            second_resolver.calls(),
            vec!["DOI:10.2/offline".to_string()]
        );

        // 30 日を過ぎた `found` は問い合わせ直す。
        conn.execute(
            "UPDATE citation_checks SET checked_at = ?1 WHERE ref_id = 'DOI:10.1/found'",
            params![(Utc::now() - ChronoDuration::days(40)).to_rfc3339()],
        )
        .unwrap();
        let (third, third_resolver) = options(vec![
            (
                "DOI:10.1/found",
                ResolveOutcome::Found {
                    title: "Found".into(),
                    year: Some(2020),
                    first_author: Some("Smith".into()),
                },
            ),
            ("DOI:10.2/offline", ResolveOutcome::Offline),
        ]);
        check_report(&conn, &report_id, &third).await.unwrap();
        assert!(
            third_resolver
                .calls()
                .contains(&"DOI:10.1/found".to_string())
        );
    }

    #[tokio::test]
    async fn year_and_author_mismatch_are_flagged() {
        let conn = setup();
        let dir = temp_root("mismatch");
        let path = dir.join("report.md");
        let report_id = register(
            &conn,
            &path,
            "# report\n\nSmith (2019) の研究 [DOI:10.1/year]。\n\nJones (2021) の研究 [DOI:10.2/author]。\n\nこの研究は重要である [DOI:10.3/clean]。\n",
        );
        let (opts, _resolver) = options(vec![
            (
                "DOI:10.1/year",
                ResolveOutcome::Found {
                    title: "Year".into(),
                    year: Some(2021),
                    first_author: Some("Smith".into()),
                },
            ),
            (
                "DOI:10.2/author",
                ResolveOutcome::Found {
                    title: "Author".into(),
                    year: Some(2021),
                    first_author: Some("Smith".into()),
                },
            ),
            (
                "DOI:10.3/clean",
                ResolveOutcome::Found {
                    title: "Clean".into(),
                    year: Some(2020),
                    first_author: Some("Smith".into()),
                },
            ),
        ]);
        check_report(&conn, &report_id, &opts).await.unwrap();
        let issues = reports::report_issues(&conn, &report_id).unwrap();
        let mismatches: Vec<&Issue> = issues.iter().filter(|i| i.kind == "mismatch").collect();
        assert_eq!(mismatches.len(), 2);
        assert!(
            mismatches
                .iter()
                .any(|i| i.detail.contains("年が本文（2019）") && i.detail.contains("取得: 2021"))
        );
        assert!(
            mismatches
                .iter()
                .any(|i| i.detail.contains("著者が本文（Jones）")
                    && i.detail.contains("取得: Smith"))
        );
        // 年も著者も無い段落は照らさない。
        assert!(!issues.iter().any(|i| i.detail.contains("10.3/clean")));
    }

    #[test]
    fn years_ignore_doi_and_arxiv_digits() {
        assert_eq!(years_in("Smith (2019) の研究"), vec![2019]);
        assert_eq!(years_in("[DOI:10.2024/xyz]"), Vec::<i32>::new());
        assert_eq!(years_in("[arXiv:2401.12345]"), Vec::<i32>::new());
        assert_eq!(years_in("2023 年の研究"), vec![2023]);
    }

    #[tokio::test]
    async fn rescan_keeps_unresolved_and_mismatch() {
        // scan は研究用ブロックの実行フォルダを読む。C03 と同じ形の根を作る。
        let root = temp_root("rescan");
        write_file(
            root.join(".sai/blocks/paper-search.md"),
            "---\nname: paper-search\ntemplates: project-brief,research-core\n---\n",
        );
        write_file(
            root.join(".sai/outputs/running/11/task.md"),
            "---\nrun_id: 11\nblock: paper-search\n---\n",
        );
        write_file(
            root.join(".sai/outputs/running/11/answer.md"),
            "---\nkind: 回答\nblock: paper-search\npapers: []\n---\n\nSmith (2019) [DOI:10.1/x]。\n",
        );
        let conn = setup();
        let summary = reports::scan(&conn, &root).unwrap();
        assert_eq!(summary.added, 1);
        let report_id: String = conn
            .query_row(
                "SELECT id FROM reports WHERE run_id = '11' AND file_name = 'answer.md'",
                [],
                |row| row.get(0),
            )
            .unwrap();

        let (opts, _resolver) = options(vec![(
            "DOI:10.1/x",
            ResolveOutcome::Found {
                title: "X".into(),
                year: Some(2021),
                first_author: Some("Smith".into()),
            },
        )]);
        check_report(&conn, &report_id, &opts).await.unwrap();
        assert!(
            reports::report_issues(&conn, &report_id)
                .unwrap()
                .iter()
                .any(|i| i.kind == "mismatch")
        );

        // 中身は同じでも scan の読み直しで `mismatch` は消えない。
        let again = reports::scan(&conn, &root).unwrap();
        assert_eq!(again.unchanged, 1);
        assert!(
            reports::report_issues(&conn, &report_id)
                .unwrap()
                .iter()
                .any(|i| i.kind == "mismatch"),
            "scan で読み直しても mismatch は残る"
        );
    }
}
