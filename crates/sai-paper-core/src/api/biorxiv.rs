//! bioRxiv / medRxiv preprint search via the Europe PMC REST API.
//!
//! bioRxiv / medRxiv 自身の API はキーワード検索を持たない（2026-09-26 時点では
//! `https://api.biorxiv.org/details/...` が HTTP 200 で本文 0 バイトを返す）。
//! そこで Europe PMC の REST 検索を使う。Europe PMC には bioRxiv / medRxiv の
//! プレプリントが `SRC:PPR` として入り、`PUBLISHER:"bioRxiv"` で出版社を絞れる。
//!
//! API: `https://www.ebi.ac.uk/europepmc/webservices/rest/search`
//! - `query`: `(<検索語>) AND SRC:PPR AND PUBLISHER:"bioRxiv"`（+ 年の範囲）
//! - `format=json`, `resultType=core`（要旨・DOI・被引用数を取るため）
//! - `pageSize`: 1 リクエストの最大件数（上限 1000）
//!
//! 次のページは応答の `nextCursorMark` を `cursorMark=` に渡して取る（今回は未使用）。
//! ページ送りの止め方は U03 が扱う。

use crate::models::RemotePaper;
use chrono::{DateTime, NaiveDate, Utc};
use regex::Regex;
use reqwest::{Client, Url};
use serde::Deserialize;
use std::sync::LazyLock;
use std::time::Duration as StdDuration;
use thiserror::Error;

const EUROPEPMC_SEARCH_URL: &str = "https://www.ebi.ac.uk/europepmc/webservices/rest/search";
/// Europe PMC の `pageSize` の上限。
const PAGE_SIZE_MAX: u16 = 1000;

/// 要旨の HTML タグ（`<h4>`, `<i>`, `<p>` など）を取り除く。
static HTML_TAG_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"<[^>]+>").expect("html tag regex"));

/// 著者文字列の区切り（`, `）。`Graham JP, Wheeler EE, …` のような形。
static AUTHOR_SEPARATOR: &str = ", ";

/// Europe PMC 検索の単位。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreprintServer {
    /// bioRxiv。
    Biorxiv,
    /// medRxiv。
    Medrxiv,
}

impl PreprintServer {
    /// DB ID（`PaperSource::id`・`RemotePaper.id`・PDF URL に使う）。
    pub fn id(self) -> &'static str {
        match self {
            Self::Biorxiv => "biorxiv",
            Self::Medrxiv => "medrxiv",
        }
    }

    /// Europe PMC の `PUBLISHER:` に渡す値（表示名にも使う）。
    pub fn publisher(self) -> &'static str {
        match self {
            Self::Biorxiv => "bioRxiv",
            Self::Medrxiv => "medRxiv",
        }
    }
}

/// bioRxiv / medRxiv 検索が返すエラー。
#[derive(Debug, Error)]
pub enum BiorxivError {
    /// URL パース失敗。
    #[error("invalid URL: {0}")]
    Url(#[from] url::ParseError),
    /// ネットワークリクエスト失敗。
    #[error("network error: {0}")]
    Request(#[from] reqwest::Error),
    /// JSON パース失敗。
    #[error("JSON parse error: {0}")]
    ParseJson(#[from] serde_json::Error),
    /// API レスポンスエラー。
    #[error("API error: {0}")]
    ApiError(String),
}

/// Europe PMC の `search` 応答。
#[derive(Debug, Deserialize)]
struct EuropePmcSearchResponse {
    /// 結果リスト。
    #[serde(rename = "resultList")]
    result_list: ResultList,
}

/// Europe PMC 応答の `resultList`。
#[derive(Debug, Deserialize)]
struct ResultList {
    /// 1 件の検索結果。
    #[serde(rename = "result", default)]
    results: Vec<EuropePmcResult>,
}

/// 1 件の Europe PMC 検索結果（`resultType=core`）。
#[derive(Debug, Deserialize, Clone)]
struct EuropePmcResult {
    /// Europe PMC 内部の ID（`PPR1325040` など）。
    #[serde(default)]
    id: Option<String>,
    /// 素の DOI（`10.64898/2026.09.18.751510`）。
    #[serde(default)]
    doi: Option<String>,
    /// 題名。
    #[serde(default)]
    title: Option<String>,
    /// 著者一覧（`, ` 区切り）。
    #[serde(rename = "authorString", default)]
    author_string: Option<String>,
    /// 要旨（HTML タグを含むことがある）。
    #[serde(rename = "abstractText", default)]
    abstract_text: Option<String>,
    /// 被引用数。
    #[serde(
        rename = "citedByCount",
        default,
        deserialize_with = "number_or_string_i32"
    )]
    cited_by_count: Option<i32>,
    /// 最初の発行日（`2026-09-22`）。
    #[serde(rename = "firstPublicationDate", default)]
    first_publication_date: Option<String>,
    /// 発行年。
    #[serde(rename = "pubYear", default)]
    pub_year: Option<String>,
}
/// 被引用数が JSON の数でも文字列でも来るときの読み取り。
fn number_or_string_i32<'de, D>(deserializer: D) -> Result<Option<i32>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum NumberOrString {
        Number(i32),
        Text(String),
    }

    Ok(match Option::<NumberOrString>::deserialize(deserializer)? {
        Some(NumberOrString::Number(n)) => Some(n),
        Some(NumberOrString::Text(s)) => s.trim().parse::<i32>().ok(),
        None => None,
    })
}

/// 設定済み、再利用可能な bioRxiv / medRxiv クライアント（実体は Europe PMC 検索）。
#[derive(Debug, Clone)]
pub struct BiorxivClient {
    client: Client,
    /// 対象とするプレプリントのサーバ。
    server: PreprintServer,
}

impl BiorxivClient {
    /// 新しい bioRxiv クライアントを構築する。
    ///
    /// # Errors
    ///
    /// HTTP クライアントの初期化に失敗した場合。
    pub fn new_biorxiv() -> Result<Self, BiorxivError> {
        Self::new(PreprintServer::Biorxiv)
    }

    /// 新しい medRxiv クライアントを構築する。
    ///
    /// # Errors
    ///
    /// HTTP クライアントの初期化に失敗した場合。
    pub fn new_medrxiv() -> Result<Self, BiorxivError> {
        Self::new(PreprintServer::Medrxiv)
    }

    /// 指定したプレプリントサーバのクライアントを構築する。
    ///
    /// # Errors
    ///
    /// HTTP クライアントの初期化に失敗した場合。
    pub fn new(server: PreprintServer) -> Result<Self, BiorxivError> {
        Ok(Self {
            client: Client::builder()
                .user_agent(concat!("sai-paper/", env!("CARGO_PKG_VERSION")))
                .timeout(StdDuration::from_secs(30))
                .build()?,
            server,
        })
    }

    /// 対象とするプレプリントサーバ。
    pub fn server(&self) -> PreprintServer {
        self.server
    }

    /// Europe PMC の検索式を作る。
    ///
    /// `(<検索語>) AND SRC:PPR AND PUBLISHER:"<出版社>"` に、年の範囲があれば
    /// `AND FIRST_PDATE:[<年-01-01> TO <年-12-31>]` を足す。
    pub fn build_query(&self, terms: &str, year_from: Option<u16>, year_to: Option<u16>) -> String {
        let mut query = format!(
            "({terms}) AND SRC:PPR AND PUBLISHER:\"{}\"",
            self.server.publisher()
        );
        if let Some(from) = year_from {
            query.push_str(&format!(" AND FIRST_PDATE:[{from}-01-01 TO "));
            match year_to {
                Some(to) => query.push_str(&format!("{to}-12-31]")),
                None => query.push_str("3000-12-31]"),
            }
        } else if let Some(to) = year_to {
            query.push_str(&format!(" AND FIRST_PDATE:[1000-01-01 TO {to}-12-31]"));
        }
        query
    }

    /// 検索式を Europe PMC の URL にする。
    fn build_url(
        &self,
        terms: &str,
        limit: u16,
        year_from: Option<u16>,
        year_to: Option<u16>,
    ) -> Result<Url, BiorxivError> {
        let mut url = Url::parse(EUROPEPMC_SEARCH_URL)?;
        url.query_pairs_mut()
            .append_pair("query", &self.build_query(terms, year_from, year_to))
            .append_pair("format", "json")
            .append_pair("resultType", "core")
            .append_pair("pageSize", &limit.clamp(1, PAGE_SIZE_MAX).to_string());
        Ok(url)
    }

    /// 検索して `RemotePaper` のリストを返す。
    ///
    /// `limit` は返す件数の上限（Europe PMC の `pageSize`）。
    /// ページ送り（`offset` / `cursorMark`）はこの関数では扱わない。
    ///
    /// # Errors
    ///
    /// ネットワーク・URL・JSON のいずれかが失敗した場合。
    pub async fn search(
        &self,
        terms: &str,
        limit: u16,
        year_from: Option<u16>,
        year_to: Option<u16>,
    ) -> Result<Vec<RemotePaper>, BiorxivError> {
        let url = self.build_url(terms, limit, year_from, year_to)?;

        let resp = self.client.get(url).send().await?.error_for_status()?;
        let text = resp.text().await?;

        Self::parse_response(&text, self.server)
    }

    /// Europe PMC の応答 JSON を `RemotePaper` のリストに直す。
    ///
    /// # Errors
    ///
    /// 本文が空の場合、または JSON の読み取りに失敗した場合。
    pub fn parse_response(
        text: &str,
        server: PreprintServer,
    ) -> Result<Vec<RemotePaper>, BiorxivError> {
        if text.trim().is_empty() {
            return Err(BiorxivError::ApiError(
                "Europe PMC が空の応答を返しました".to_string(),
            ));
        }

        let response: EuropePmcSearchResponse = serde_json::from_str(text)
            .map_err(|e| BiorxivError::ApiError(format!("Europe PMC parse failed: {}", e)))?;

        Ok(response
            .result_list
            .results
            .into_iter()
            .filter_map(|result| convert(result, server))
            .collect())
    }
}

/// 1 件の Europe PMC 結果を `RemotePaper` にする。
///
/// 共有契約 C2: `doi` は素の DOI（`https://doi.org/` を付けない）。
fn convert(result: EuropePmcResult, server: PreprintServer) -> Option<RemotePaper> {
    // 題名にも HTML タグ（`<i>` など）が混ざることがあるので取り除いてから使う
    let title = result
        .title
        .as_deref()
        .map(strip_html)
        .filter(|t| !t.is_empty())?;
    let doi = result
        .doi
        .as_deref()
        .map(str::trim)
        .filter(|d| !d.is_empty())
        .map(ToString::to_string);

    // DOI が無いときは Europe PMC の内部 ID を使う
    let id = match &doi {
        Some(doi) => format!("{}/{}", server.id(), doi),
        None => format!("{}/{}", server.id(), result.id?),
    };

    let authors: Vec<String> = result
        .author_string
        .as_deref()
        .map(|s| {
            s.split(AUTHOR_SEPARATOR)
                .map(|name| name.trim().trim_end_matches('.').to_string())
                .filter(|name| !name.is_empty())
                .collect()
        })
        .unwrap_or_default();

    let abstract_text = result
        .abstract_text
        .as_deref()
        .map(strip_html)
        .unwrap_or_default();

    let published = resolve_published(
        result.first_publication_date.as_deref(),
        result.pub_year.as_deref(),
    );

    Some(RemotePaper {
        id,
        title,
        authors,
        abstract_text,
        published,
        updated: published,
        categories: vec![format!("{} preprint", server.publisher())],
        pdf_url: doi
            .as_deref()
            .map(|doi| format!("https://www.{}.org/content/{doi}.full.pdf", server.id())),
        doi,
        journal_ref: Some(format!("{} (preprint)", server.id().to_uppercase())),
        citation_count: result.cited_by_count,
    })
}

/// 要旨の HTML タグを取り除く。
fn strip_html(text: &str) -> String {
    HTML_TAG_RE.replace_all(text, "").trim().to_string()
}

/// 発行日を作る。`firstPublicationDate` → `pubYear` の 1 月 1 日 → 既定 2000-01-01。
fn resolve_published(
    first_publication_date: Option<&str>,
    pub_year: Option<&str>,
) -> DateTime<Utc> {
    if let Some(date) = first_date(first_publication_date) {
        return date;
    }
    if let Some(date) = pub_year
        .and_then(|year| year.trim().parse::<i32>().ok())
        .and_then(|year| NaiveDate::from_ymd_opt(year, 1, 1))
    {
        return date.and_hms_opt(0, 0, 0).unwrap().and_utc();
    }
    NaiveDate::from_ymd_opt(2000, 1, 1)
        .unwrap()
        .and_hms_opt(0, 0, 0)
        .unwrap()
        .and_utc()
}

/// `2026-09-22` のような日付文字列を `DateTime<Utc>` にする。
fn first_date(raw: Option<&str>) -> Option<DateTime<Utc>> {
    let raw = raw?.trim();
    let date = NaiveDate::parse_from_str(raw, "%Y-%m-%d")
        .or_else(|_| NaiveDate::parse_from_str(raw, "%Y/%m/%d"))
        .ok()?;
    Some(date.and_hms_opt(0, 0, 0)?.and_utc())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 計画書の「Europe PMC で確かめたこと」（2026-09-26、curl）に基づく応答。
    const SAMPLE: &str = r#"{
  "hitCount": 10659,
  "request": {
    "queryString": "(CRISPR) AND SRC:PPR AND PUBLISHER:\"bioRxiv\"",
    "resultType": "core"
  },
  "resultList": {
    "result": [
      {
        "id": "PPR1325040",
        "source": "PPR",
        "pmid": "41234567",
        "doi": "10.64898/2026.09.18.751510",
        "title": "Programmable <i>RNA</i> sensing in mammalian cells.",
        "authorString": "Graham JP, Wheeler EE, Naylor LH, Doe A",
        "abstractText": "<h4>Abstract</h4><p>We show that <i>CRISPR</i> sensors detect RNA.</p>",
        "citedByCount": 12,
        "firstPublicationDate": "2026-09-22",
        "pubYear": "2026",
        "bookOrReportDetails": {
          "publisher": "bioRxiv"
        }
      }
    ]
  }
}"#;

    /// AC4: Europe PMC の応答が表のとおり `RemotePaper` になる。
    #[test]
    fn europe_pmc_response_maps_to_remote_paper() {
        let papers =
            BiorxivClient::parse_response(SAMPLE, PreprintServer::Biorxiv).expect("parsing failed");
        assert_eq!(papers.len(), 1);
        let paper = &papers[0];

        assert_eq!(paper.id, "biorxiv/10.64898/2026.09.18.751510");
        assert_eq!(paper.title, "Programmable RNA sensing in mammalian cells.");
        assert_eq!(
            paper.authors,
            vec!["Graham JP", "Wheeler EE", "Naylor LH", "Doe A"]
        );
        assert_eq!(
            paper.abstract_text,
            "AbstractWe show that CRISPR sensors detect RNA."
        );
        assert_eq!(paper.doi.as_deref(), Some("10.64898/2026.09.18.751510"));
        assert_eq!(paper.citation_count, Some(12));
        assert_eq!(
            paper.published,
            NaiveDate::from_ymd_opt(2026, 9, 22)
                .unwrap()
                .and_hms_opt(0, 0, 0)
                .unwrap()
                .and_utc()
        );
        assert_eq!(paper.updated, paper.published);
        assert_eq!(
            paper.pdf_url.as_deref(),
            Some("https://www.biorxiv.org/content/10.64898/2026.09.18.751510.full.pdf")
        );
        assert_eq!(paper.categories, vec!["bioRxiv preprint"]);
        assert_eq!(paper.journal_ref.as_deref(), Some("BIORXIV (preprint)"));
    }

    /// medRxiv は PDF URL・分類・誌名が medRxiv 側になる。
    #[test]
    fn europe_pmc_medrxiv_response() {
        let json = r#"{
  "hitCount": 4,
  "resultList": {
    "result": [
      {
        "id": "PPR9999999",
        "source": "PPR",
        "doi": "10.1101/2026.09.20.123456",
        "title": "A medRxiv preprint.",
        "authorString": "Doe J.",
        "abstractText": "Text.",
        "citedByCount": "3",
        "firstPublicationDate": "2026-09-20",
        "pubYear": "2026"
      }
    ]
  }
}"#;
        let papers =
            BiorxivClient::parse_response(json, PreprintServer::Medrxiv).expect("parsing failed");
        let paper = &papers[0];
        assert_eq!(paper.id, "medrxiv/10.1101/2026.09.20.123456");
        assert_eq!(paper.authors, vec!["Doe J"]);
        assert_eq!(paper.citation_count, Some(3));
        assert_eq!(
            paper.pdf_url.as_deref(),
            Some("https://www.medrxiv.org/content/10.1101/2026.09.20.123456.full.pdf")
        );
        assert_eq!(paper.categories, vec!["medRxiv preprint"]);
        assert_eq!(paper.journal_ref.as_deref(), Some("MEDRXIV (preprint)"));
    }

    /// DOI が無ければ Europe PMC の内部 ID を使い、PDF URL は作らない。
    #[test]
    fn europe_pmc_result_without_doi() {
        let json = r#"{"resultList":{"result":[
          {"id":"PPR1234567","source":"PPR","title":"No DOI here","authorString":"Doe J.","abstractText":"Body.","citedByCount":1,"pubYear":"2024"}
        ]}}"#;
        let papers =
            BiorxivClient::parse_response(json, PreprintServer::Biorxiv).expect("parsing failed");
        let paper = &papers[0];
        assert_eq!(paper.id, "biorxiv/PPR1234567");
        assert_eq!(paper.doi, None);
        assert_eq!(paper.pdf_url, None);
        // firstPublicationDate が無ければ pubYear の 1 月 1 日
        assert_eq!(
            paper.published,
            NaiveDate::from_ymd_opt(2024, 1, 1)
                .unwrap()
                .and_hms_opt(0, 0, 0)
                .unwrap()
                .and_utc()
        );
    }

    /// 空の本文（bioRxiv API のように 0 バイト）はエラーにする。
    #[test]
    fn europe_pmc_empty_body_is_error() {
        let err = BiorxivClient::parse_response("   ", PreprintServer::Biorxiv)
            .expect_err("empty body must fail");
        assert!(err.to_string().contains("空の応答"));
    }

    /// 検索式は `SRC:PPR` と `PUBLISHER:"bioRxiv"` を含み、年を足すと範囲が入る。
    #[test]
    fn europe_pmc_query_building() {
        let client = BiorxivClient::new_biorxiv().expect("client");
        assert_eq!(
            client.build_query("CRISPR", None, None),
            "(CRISPR) AND SRC:PPR AND PUBLISHER:\"bioRxiv\""
        );
        assert_eq!(
            client.build_query("Ca2+ signaling", Some(2020), Some(2024)),
            "(Ca2+ signaling) AND SRC:PPR AND PUBLISHER:\"bioRxiv\" AND FIRST_PDATE:[2020-01-01 TO 2024-12-31]"
        );

        let medrxiv = BiorxivClient::new_medrxiv().expect("client");
        assert_eq!(
            medrxiv.build_query("crispr", Some(2023), None),
            "(crispr) AND SRC:PPR AND PUBLISHER:\"medRxiv\" AND FIRST_PDATE:[2023-01-01 TO 3000-12-31]"
        );
        assert_eq!(medrxiv.server(), PreprintServer::Medrxiv);
        assert_eq!(medrxiv.server().id(), "medrxiv");
    }
}
