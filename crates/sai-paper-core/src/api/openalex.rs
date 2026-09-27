//! OpenAlex API client for works search and DOI-based journal discovery.
//!
//! 2026 年 2 月以降、OpenAlex は「メールを書けば優遇される（polite pool）」を廃止し、
//! 1 日あたりの使用量の上限（キー無し 1,000 クレジット、キーあり 10 倍）を設けた。
//! そのため `contact_email` は User-Agent に `mailto:` を付けるだけで使用量の上限には効かず、
//! 実際に効かせるのは `Authorization: Bearer <key>` で送る API キーである。

use crate::models::RemotePaper;
use chrono::{NaiveDate, Utc};
use reqwest::{Client, Url};
use serde::Deserialize;
use thiserror::Error;

const OPENALEX_ENDPOINT: &str = "https://api.openalex.org/works";
const OPENALEX_WORKS_SEARCH: &str = "https://api.openalex.org/works";

/// OpenAlex が返すエラー。
#[derive(Debug, Error)]
pub enum OpenAlexError {
    /// URL パース失敗。
    #[error("invalid URL: {0}")]
    Url(#[from] url::ParseError),
    /// ネットワークリクエスト失敗。
    #[error("HTTP request failed: {0}")]
    Request(#[from] reqwest::Error),
    /// JSON パース失敗。
    #[error("failed to parse response: {0}")]
    Parse(#[from] serde_json::Error),
    /// 工作が見つからない（DOI 検索時）。
    #[error("work not found")]
    NotFound,
    /// 1 日の使用量の上限に達した（429）。
    #[error("OpenAlex の 1 日の使用量の上限に達しました（環境変数 OPENALEX_API_KEY を確認してください）")]
    RateLimited,
}

#[derive(Deserialize)]
struct Work {
    /// 工作 ID。
    id: Option<String>,
    /// 工作のタイトル。
    title: Option<String>,
    /// 発行年。
    publication_year: Option<i32>,
    /// 発行日（ISO 8601 形式）。
    publication_date: Option<String>,
    /// アブストラクト（反転インデックス形式）。
    abstract_inverted_index: Option<serde_json::Value>,
    /// 被引用数（OpenAlex の `cited_by_count`）。
    cited_by_count: Option<i32>,
    /// 著者情報。
    authorships: Option<Vec<Authorship>>,
    /// 主要な発行場所（ジャーナル等）。
    primary_location: Option<Location>,
    /// ホストベニュー（発行元）。
    host_venue: Option<Venue>,
    /// DOI。
    doi: Option<String>,
    /// 撤回されているか（OpenAlex の `is_retracted`）。欄が無い応答もあるため `Option`。
    #[serde(default)]
    is_retracted: Option<bool>,
}

impl Work {
    /// 発行場所（ジャーナル等）の名前。空文字は `None` に落とす。
    fn journal_name(&self) -> Option<String> {
        self.primary_location
            .as_ref()
            .and_then(|location| location.source.as_ref())
            .and_then(|source| source.display_name.clone())
            .or_else(|| {
                self.host_venue
                    .as_ref()
                    .and_then(|venue| venue.display_name.clone())
            })
            .filter(|name| !name.trim().is_empty())
    }

    /// 発行日（`publication_date` があればそれ、無ければ `publication_year` の 1 月 1 日）。
    /// どちらも無ければ 2000-01-01。
    fn published(&self) -> chrono::DateTime<Utc> {
        let date = self
            .publication_date
            .as_deref()
            .and_then(|text| NaiveDate::parse_from_str(text, "%Y-%m-%d").ok())
            .or_else(|| {
                self.publication_year
                    .and_then(|year| NaiveDate::from_ymd_opt(year, 1, 1))
            })
            .unwrap_or_else(|| NaiveDate::from_ymd_opt(2000, 1, 1).unwrap());
        date.and_hms_opt(0, 0, 0).unwrap().and_utc()
    }

    /// 素の DOI（`https://doi.org/` や `doi:` を取ったもの）。無ければ `None`。
    fn plain_doi(&self) -> Option<String> {
        self.doi.as_deref().and_then(strip_doi_prefix)
    }

    /// アブストラクトを反転インデックスから復元する。無いときは空文字。
    fn abstract_text(&self) -> String {
        self.abstract_inverted_index
            .as_ref()
            .map(abstract_from_inverted_index)
            .unwrap_or_default()
    }

    /// この工作から取り出した発行場所と撤回の状態。
    fn status(&self) -> WorkStatus {
        WorkStatus {
            journal: self.journal_name(),
            is_retracted: self.is_retracted,
        }
    }
}

/// 工作（論文）に付随する、メタデータの補完で使う情報。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkStatus {
    /// 発行場所（ジャーナル等）の名前。取れなければ `None`。
    pub journal: Option<String>,
    /// 撤回されているか。`None` は OpenAlex の応答に欄が無いことを表す。
    pub is_retracted: Option<bool>,
}

/// 著者情報。
#[derive(Deserialize)]
struct Authorship {
    /// 著者の詳細情報。
    author: Option<Author>,
}

/// 著者。
#[derive(Deserialize)]
struct Author {
    /// 著者の表示名。
    display_name: Option<String>,
}

/// 発行場所。
#[derive(Deserialize)]
struct Location {
    /// ソース（ジャーナル等）。
    source: Option<Venue>,
}

/// ベニュー（ジャーナルや会議など）。
#[derive(Deserialize)]
struct Venue {
    /// ベニューの表示名。
    display_name: Option<String>,
}

/// OpenAlex 工作検索レスポンス。
#[derive(Deserialize)]
struct WorksSearchResponse {
    /// 検索結果の工作リスト。
    results: Vec<Work>,
}

/// DOI から `https://doi.org/` `http://doi.org/` `doi:` を取り除く。空文字なら `None`。
pub(crate) fn strip_doi_prefix(doi: &str) -> Option<String> {
    let mut rest = doi.trim();
    for prefix in [
        "https://doi.org/",
        "http://doi.org/",
        "https://dx.doi.org/",
        "http://dx.doi.org/",
        "doi:",
    ] {
        if let Some(stripped) = rest.strip_prefix(prefix) {
            rest = stripped;
            break;
        }
    }
    let rest = rest.trim();
    if rest.is_empty() {
        None
    } else {
        Some(rest.to_string())
    }
}

/// OpenAlex の `abstract_inverted_index`（`{"単語": [位置, …], …}`）を本文に戻す。
/// 位置の順に空白でつなぐ。形式が違えば空文字。
fn abstract_from_inverted_index(index: &serde_json::Value) -> String {
    let Some(map) = index.as_object() else {
        return String::new();
    };
    let mut slots: Vec<(i64, &str)> = Vec::new();
    for (word, positions) in map {
        let Some(positions) = positions.as_array() else {
            continue;
        };
        for position in positions {
            let Some(position) = position.as_i64() else {
                continue;
            };
            slots.push((position, word));
        }
    }
    slots.sort_unstable();
    slots
        .into_iter()
        .map(|(_, word)| word)
        .collect::<Vec<_>>()
        .join(" ")
}

/// OpenAlex API のクライアント。
///
/// DOI ベースのジャーナル発見と、フルテキスト検索の両方に対応。
#[derive(Clone)]
pub struct OpenAlexClient {
    /// HTTP クライアント。
    client: Client,
    /// Polite pool 利用用のメールアドレス（User-Agent に含める。2026 年 2 月以降は効き目が無い）。
    #[allow(dead_code)]
    contact_email: Option<String>,
    /// API キー。あれば全要求に `Authorization: Bearer` で付ける（URL には出さない）。
    api_key: Option<String>,
}

impl OpenAlexClient {
    /// デフォルト設定でクライアントを構築する。
    #[must_use]
    pub fn new() -> Self {
        Self::with_contact_email(None)
    }

    /// 設定（`[discover]`）からクライアントを構築する。
    ///
    /// 連絡先メールアドレスと OpenAlex の API キー（`discover.openalex_api_key_env`）を渡す。
    #[must_use]
    pub fn from_config(cfg: &crate::Config) -> Self {
        Self::with_parts(cfg.discover.contact_email.clone(), cfg.openalex_api_key())
    }

    /// メールアドレス付きでクライアントを構築する。
    pub fn with_contact_email(contact_email: Option<String>) -> Self {
        Self::with_parts(contact_email, None)
    }

    /// 連絡先メールアドレスと API キーを指定してクライアントを構築する。
    pub fn with_parts(contact_email: Option<String>, api_key: Option<String>) -> Self {
        let user_agent = if let Some(email) = &contact_email {
            format!("sai-paper/{} (mailto:{})", env!("CARGO_PKG_VERSION"), email)
        } else {
            format!("sai-paper/{}", env!("CARGO_PKG_VERSION"))
        };

        Self {
            client: Client::builder()
                .user_agent(&user_agent)
                .build()
                .unwrap_or_default(),
            contact_email,
            api_key: api_key.filter(|key| !key.trim().is_empty()),
        }
    }

    /// 送信するリクエスト。API キーがあるときは `Authorization: Bearer` を付ける。
    pub(crate) fn request(&self, url: Url) -> reqwest::RequestBuilder {
        let request = self.client.get(url);
        match &self.api_key {
            Some(key) => request.bearer_auth(key),
            None => request,
        }
    }

    /// Fetch the canonical journal name for a DOI, if `OpenAlex` has one.
    ///
    /// # Errors
    ///
    /// Returns an error when the request fails or the response is malformed.
    pub async fn journal_by_doi(&self, doi: &str) -> Result<Option<String>, OpenAlexError> {
        Ok(self
            .work_status_by_doi(doi)
            .await?
            .and_then(|status| status.journal))
    }

    /// Fetch the journal and retraction status for a DOI.
    ///
    /// 工作が見つからない（404）ときは `Ok(None)`。ネットワーク・パースの失敗は
    /// `Err`。呼び側は `Ok(None)` と `Err` を区別して扱う。
    ///
    /// # Errors
    ///
    /// Returns an error when the request fails or the response is malformed.
    pub async fn work_status_by_doi(
        &self,
        doi: &str,
    ) -> Result<Option<WorkStatus>, OpenAlexError> {
        let url = format!("{}/{}", OPENALEX_ENDPOINT, format!("https://doi.org/{doi}"));
        let url = Url::parse(&url)?;
        let response = self.request(url).send().await?;
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        if response.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
            return Err(OpenAlexError::RateLimited);
        }
        let response = response.error_for_status()?;
        let work: Work = serde_json::from_slice(&response.bytes().await?)?;
        Ok(Some(work.status()))
    }

    /// OpenAlex で検索を実行して論文リストを返す。
    ///
    /// 検索は `search` パラメータを使用してフルテキスト検索を実行。
    ///
    /// # Errors
    ///
    /// ネットワークエラーまたはパースエラーが発生した場合、エラーを返す。
    pub async fn search(&self, query: &str, limit: u16) -> Result<Vec<RemotePaper>, OpenAlexError> {
        let mut url = Url::parse(OPENALEX_WORKS_SEARCH)?;
        url.query_pairs_mut()
            .append_pair("search", query)
            .append_pair("per-page", &limit.to_string());

        let response = self.request(url).send().await?;
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(Vec::new());
        }
        if response.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
            return Err(OpenAlexError::RateLimited);
        }
        let response = response.error_for_status()?;

        let search_response: WorksSearchResponse =
            serde_json::from_slice(&response.bytes().await?)?;

        let papers = search_response
            .results
            .into_iter()
            .map(|work| {
                // `work` を部分的に move するので、先に `&self` が必要な値を取る
                let published = work.published();
                let abstract_text = work.abstract_text();
                let doi = work.plain_doi();
                let citation_count = work.cited_by_count;

                let title = work.title.unwrap_or_default();
                let authors: Vec<String> = work
                    .authorships
                    .as_ref()
                    .map(|auths| {
                        auths
                            .iter()
                            .filter_map(|a| a.author.as_ref())
                            .filter_map(|a| a.display_name.as_ref().cloned())
                            .collect()
                    })
                    .unwrap_or_default();

                let journal_ref = work
                    .primary_location
                    .as_ref()
                    .and_then(|loc| loc.source.as_ref())
                    .and_then(|source| source.display_name.as_ref())
                    .or_else(|| {
                        work.host_venue
                            .as_ref()
                            .and_then(|venue| venue.display_name.as_ref())
                    })
                    .cloned();

                RemotePaper {
                    id: work.id.unwrap_or_else(|| title.clone()),
                    title,
                    authors,
                    abstract_text,
                    published,
                    updated: published,
                    categories: vec!["OpenAlex".to_string()],
                    pdf_url: None,
                    doi,
                    journal_ref,
                    citation_count,
                }
            })
            .collect();

        Ok(papers)
    }
}

impl Default for OpenAlexClient {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(json: &str) -> Work {
        serde_json::from_str(json).expect("Work の JSON")
    }

    #[test]
    fn work_reads_year_citations_abstract_and_plain_doi() {
        let work = parse(
            r#"{
                "id": "W1",
                "title": "A Paper",
                "publication_date": "2024-05-12",
                "cited_by_count": 42,
                "abstract_inverted_index": {"Deep": [0], "learning": [1, 3], "is": [2]},
                "doi": "https://doi.org/10.1/abc"
            }"#,
        );

        assert_eq!(work.published().format("%Y-%m-%d").to_string(), "2024-05-12");
        assert_eq!(work.cited_by_count, Some(42));
        assert_eq!(work.abstract_text(), "Deep learning is learning");
        assert_eq!(work.plain_doi().as_deref(), Some("10.1/abc"));
    }

    #[test]
    fn published_falls_back_to_year_then_to_2000() {
        let with_year = parse(r#"{"publication_year": 2019}"#);
        assert_eq!(with_year.published().format("%Y-%m-%d").to_string(), "2019-01-01");

        // `publication_date` が読めない形のときは `publication_year` にフォールバック
        let broken = parse(r#"{"publication_date": "2024/05/12", "publication_year": 2019}"#);
        assert_eq!(broken.published().format("%Y-%m-%d").to_string(), "2019-01-01");

        let empty = parse(r#"{"id": "W2"}"#);
        assert_eq!(empty.published().format("%Y-%m-%d").to_string(), "2000-01-01");
        assert_eq!(empty.abstract_text(), "");
        assert_eq!(empty.plain_doi(), None);
    }

    #[test]
    fn strip_doi_prefix_removes_resolver_forms() {
        assert_eq!(strip_doi_prefix("https://doi.org/10.1/abc").as_deref(), Some("10.1/abc"));
        assert_eq!(strip_doi_prefix("http://doi.org/10.1/abc").as_deref(), Some("10.1/abc"));
        assert_eq!(strip_doi_prefix("doi:10.1/abc").as_deref(), Some("10.1/abc"));
        assert_eq!(strip_doi_prefix("10.1/abc").as_deref(), Some("10.1/abc"));
        assert_eq!(strip_doi_prefix("  "), None);
    }

    #[test]
    fn work_status_reads_retraction_flag() {
        // `is_retracted: true`
        let retracted = parse(
            r#"{"id":"W1","is_retracted":true,"primary_location":{"source":{"display_name":"J"}}}"#,
        );
        let status = retracted.status();
        assert_eq!(status.is_retracted, Some(true));
        assert_eq!(status.journal.as_deref(), Some("J"));

        // `is_retracted: false`
        let ok = parse(r#"{"id":"W2","is_retracted":false}"#);
        assert_eq!(ok.status().is_retracted, Some(false));
        assert_eq!(ok.status().journal, None);

        // 欄が無い（未確認）
        let absent = parse(r#"{"id":"W3"}"#);
        assert_eq!(absent.status().is_retracted, None);

        // host_venue へのフォールバック
        let venue = parse(r#"{"id":"W4","host_venue":{"display_name":"V"}}"#);
        assert_eq!(venue.status().journal.as_deref(), Some("V"));

        // 名前が空白だけなら None
        let empty = parse(r#"{"id":"W5","primary_location":{"source":{"display_name":"  "}}}"#);
        assert_eq!(empty.status().journal, None);
    }
}
