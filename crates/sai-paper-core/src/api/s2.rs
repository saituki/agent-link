#![allow(missing_docs)]
//! S2 (Semantic Scholar) client for Discover — word-based ranking, 1.1s throttle.
//!
//! 引用文献パイプライン用に `get_paper` / `get_paper_references` を提供する。
//! リトライ方針（429 → Retry-After 待機 → 再送）は `search_with_offset` と共通の
//! ループ型で統一している。

use chrono::NaiveDate;
use reqwest::{Client, Url};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};
use thiserror::Error;
use tokio::sync::Mutex;

use crate::models::RemotePaper;

const API_URL: &str = "https://api.semanticscholar.org/graph/v1/paper/search";
const PAPER_API_BASE: &str = "https://api.semanticscholar.org/graph/v1/paper";
const REQUEST_INTERVAL: Duration = Duration::from_millis(1100);

/// 429 / 通信エラーに対する最大再送回数（初回を除く）。
///
/// S2 はキー付きリクエストでも断続的に 429 を返すことがあるため、
/// 単発の失敗で検索全体を諦めないよう複数回再送する。
const MAX_RETRIES: u8 = 5;
/// リトライ時の指数バックオフ上限（秒）。
const MAX_RETRY_SECS: u64 = 10;

/// 論文詳細の取得フィールド（get_paper / references 共通）。
const PAPER_DETAIL_FIELDS: &str =
    "paperId,title,abstract,year,venue,journal,authors,externalIds,citationCount,openAccessPdf,url";

/// 検索（relevance search）の取得フィールド。
/// `publicationDate`（YYYY-MM-DD）は年月表示（R4）、`journal` はジャーナル名列に使用する。
const SEARCH_FIELDS: &str = "paperId,title,abstract,year,publicationDate,venue,journal,authors,externalIds,citationCount,openAccessPdf";

#[derive(Debug, Error)]
pub enum S2Error {
    #[error("invalid S2 URL: {0}")]
    Url(#[from] url::ParseError),
    #[error("S2 request failed: {0}")]
    Request(#[from] reqwest::Error),
    #[error("invalid S2 response: {0}")]
    Decode(String),
    /// 論文が S2 に存在しない（HTTP 404）。
    #[error("S2 paper not found (404): {0}")]
    NotFound(String),
}

#[derive(Debug, Clone)]
pub struct S2Client {
    client: Client,
    api_key: Option<String>,
    next_request_at: Arc<Mutex<Instant>>,
}

#[derive(Deserialize, Debug)]
struct S2SearchResponse {
    #[allow(dead_code)]
    total: usize,
    data: Vec<S2Paper>,
}

#[derive(Deserialize, Debug)]
struct S2Paper {
    #[serde(rename = "paperId")]
    paper_id: String,
    title: Option<String>,
    #[serde(rename = "abstract")]
    abstract_text: Option<String>,
    year: Option<i32>,
    /// 発行日（YYYY-MM-DD）。年月表示（R4）に使用する。
    #[serde(rename = "publicationDate")]
    publication_date: Option<String>,
    venue: Option<String>,
    journal: Option<S2Journal>,
    authors: Option<Vec<S2Author>>,
    #[serde(rename = "externalIds", deserialize_with = "deserialize_external_ids")]
    external_ids: Option<std::collections::HashMap<String, String>>,
    #[serde(rename = "citationCount")]
    citation_count: Option<i32>,
    #[serde(rename = "openAccessPdf")]
    open_access_pdf: Option<S2Pdf>,
}

fn deserialize_external_ids<'de, D>(
    deserializer: D,
) -> Result<Option<std::collections::HashMap<String, String>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let opt: Option<std::collections::HashMap<String, serde_json::Value>> =
        Option::deserialize(deserializer)?;
    Ok(opt.map(|m| {
        m.into_iter()
            .map(|(k, v)| {
                let s = match v {
                    serde_json::Value::String(s) => s,
                    serde_json::Value::Number(n) => n.to_string(),
                    other => other.to_string().trim_matches('"').to_string(),
                };
                (k, s)
            })
            .collect()
    }))
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct S2Author {
    pub name: Option<String>,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct S2Pdf {
    pub url: Option<String>,
}

/// S2 journal information (journal field of the paper detail).
#[derive(Deserialize, Serialize, Debug, Clone, Default)]
pub struct S2Journal {
    pub name: Option<String>,
    pub volume: Option<String>,
    pub pages: Option<String>,
}

/// S2 API から返される論文詳細（get_paper / references 用）。
/// テストで生成できるよう `Default` を導出している。
#[derive(Deserialize, Serialize, Debug, Clone, Default)]
pub struct S2PaperDetail {
    #[serde(rename = "paperId")]
    pub paper_id: Option<String>,
    pub title: Option<String>,
    #[serde(rename = "abstract")]
    pub abstract_text: Option<String>,
    pub year: Option<i32>,
    pub venue: Option<String>,
    pub journal: Option<S2Journal>,
    pub authors: Option<Vec<S2Author>>,
    // references レスポンスの一部エントリは externalIds を欠くことがあるため default を併記
    #[serde(
        rename = "externalIds",
        default,
        deserialize_with = "deserialize_external_ids_lenient"
    )]
    pub external_ids: Option<HashMap<String, String>>,
    #[serde(rename = "citationCount")]
    pub citation_count: Option<i32>,
    #[serde(rename = "openAccessPdf")]
    pub open_access_pdf: Option<S2Pdf>,
    pub url: Option<String>,
}

/// S2 は `externalIds` をまれに配列 (`[]`) で返すことがある。
/// パース失敗で全体を落とさないよう緩く扱う。
fn deserialize_external_ids_lenient<'de, D>(
    deserializer: D,
) -> Result<Option<HashMap<String, String>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value: Option<serde_json::Value> = Option::deserialize(deserializer)?;
    let Some(value) = value else {
        return Ok(None);
    };
    let Ok(map) = serde_json::from_value::<HashMap<String, serde_json::Value>>(value) else {
        return Ok(None);
    };
    Ok(Some(
        map.into_iter()
            .map(|(k, v)| {
                let s = match v {
                    serde_json::Value::String(s) => s,
                    serde_json::Value::Number(n) => n.to_string(),
                    other => other.to_string().trim_matches('"').to_string(),
                };
                (k, s)
            })
            .collect(),
    ))
}

/// `/paper/{id}/references` / `/paper/{id}/citations` の1エントリ。
/// 通常は `citedPaper` / `citingPaper` に入るが、フラットな形でも受けられるよう
/// `flatten` で併記する。
#[derive(Deserialize, Debug)]
struct S2RefEntry {
    #[serde(rename = "citedPaper")]
    cited_paper: Option<S2PaperDetail>,
    #[serde(rename = "citingPaper")]
    citing_paper: Option<S2PaperDetail>,
    #[serde(flatten)]
    inline: S2PaperDetail,
}

impl S2RefEntry {
    fn into_detail(self) -> S2PaperDetail {
        // 実体が入っている方（paperId / title のどちらかがある方）を優先する
        let usable = |p: &S2PaperDetail| p.paper_id.is_some() || p.title.is_some();
        if self.cited_paper.as_ref().is_some_and(|p| usable(p)) {
            return self.cited_paper.unwrap();
        }
        if self.citing_paper.as_ref().is_some_and(|p| usable(p)) {
            return self.citing_paper.unwrap();
        }
        self.inline
    }
}

/// `/paper/{id}/references` の1ページ。
#[derive(Deserialize, Debug)]
struct S2RefPage {
    next: Option<usize>,
    data: Option<Vec<S2RefEntry>>,
}

impl S2Client {
    pub fn new(api_key: Option<String>) -> Result<Self, S2Error> {
        Ok(Self {
            client: Client::builder()
                .user_agent(concat!("papr/", env!("CARGO_PKG_VERSION")))
                // limit=100 の応答は数百 KB になりやすい。S2 側の遅延（2〜6 秒以上）
                // を見込んで十分な余裕を持たせる。
                .timeout(Duration::from_secs(30))
                .build()?,
            api_key,
            next_request_at: Arc::new(Mutex::new(Instant::now())),
        })
    }

    async fn throttle(&self) {
        let mut next = self.next_request_at.lock().await;
        let wait = next.saturating_duration_since(Instant::now());
        if !wait.is_zero() {
            tokio::time::sleep(wait).await;
        }
        *next = Instant::now() + REQUEST_INTERVAL;
    }

    /// GET リクエストを実行し、本文テキストを返す共通ヘルパ。
    ///
    /// throttle → リクエスト → 429 / 通信エラー（タイムアウト・接続断）は
    /// Retry-After を尊重した指数バックオフで再送（最大 `MAX_RETRIES` 回）。
    /// `search_with_offset` と同一のループ型リトライ。
    async fn get_text(&self, url: &Url) -> Result<String, S2Error> {
        let mut attempts: u8 = 0;
        loop {
            self.throttle().await;
            let mut req = self.client.get(url.clone());
            if let Some(k) = &self.api_key {
                req = req.header("x-api-key", k);
            }
            let resp = match req.send().await {
                Ok(resp) => resp,
                // タイムアウト / 接続断は一時的な障害として再送する
                Err(e) => {
                    attempts += 1;
                    if attempts > MAX_RETRIES {
                        return Err(S2Error::Request(e));
                    }
                    let wait = 2u64.pow(attempts as u32).min(MAX_RETRY_SECS);
                    tokio::time::sleep(Duration::from_secs(wait)).await;
                    continue;
                }
            };
            let status = resp.status();
            if status.as_u16() == 429 {
                attempts += 1;
                if attempts > MAX_RETRIES {
                    return Err(S2Error::Decode(
                        "S2 status 429 (rate limited) — set SEMANTICSCHOLAR_API_KEY"
                            .to_string(),
                    ));
                }
                let retry_secs = resp
                    .headers()
                    .get(reqwest::header::RETRY_AFTER)
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| v.parse::<u64>().ok())
                    .unwrap_or_else(|| 2u64.pow(attempts as u32).min(MAX_RETRY_SECS));
                tokio::time::sleep(Duration::from_secs(retry_secs)).await;
                continue;
            }
            if status.as_u16() == 404 {
                return Err(S2Error::NotFound(url.path().to_string()));
            }
            if status.as_u16() == 403 {
                return Err(S2Error::Decode(
                    "S2 status 403 Forbidden — API キーが無効です（環境変数 SEMANTICSCHOLAR_API_KEY を確認してください）"
                        .to_string(),
                ));
            }
            if !status.is_success() {
                return Err(S2Error::Decode(format!("S2 status {}", status)));
            }
            let text = resp
                .text()
                .await
                .map_err(|e| S2Error::Decode(e.to_string()))?;
            return Ok(text);
        }
    }

    /// URL パスセグメント用の最小エンコーダ。
    ///
    /// `DOI:10.xxx/yyy` の `/` はパスとして意味を持つためエンコード必須。
    /// `:` は S2 API がそのまま受け付けるため保持する。
    fn encode_path_segment(value: &str) -> String {
        let mut out = String::with_capacity(value.len());
        for b in value.bytes() {
            if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~' | b':') {
                out.push(b as char);
            } else {
                out.push_str(&format!("%{b:02X}"));
            }
        }
        out
    }

    /// 単一論文の詳細を取得する。
    ///
    /// `paper_id` は S2 40hex / `ArXiv:1706.03762` / `DOI:10.xxx/yyy` 等を
    /// そのまま受け付ける（S2 API が paperId キーとして解釈する）。
    pub async fn get_paper(&self, paper_id: &str) -> Result<S2PaperDetail, S2Error> {
        let url = Url::parse(&format!(
            "{PAPER_API_BASE}/{}?fields={PAPER_DETAIL_FIELDS}",
            Self::encode_path_segment(paper_id),
        ))?;
        let text = self.get_text(&url).await?;
        serde_json::from_str(&text).map_err(|e| S2Error::Decode(e.to_string()))
    }

    /// 論文の引用文献リストを取得する（専用エンドポイント + ページング、最大1000件/ページ）。
    pub async fn get_paper_references(
        &self,
        paper_id: &str,
    ) -> Result<Vec<S2PaperDetail>, S2Error> {
        self.get_paper_links(paper_id, "references").await
    }

    /// 論文の被引用リストを取得する（専用エンドポイント + ページング）。
    pub async fn get_paper_citations(&self, paper_id: &str) -> Result<Vec<S2PaperDetail>, S2Error> {
        self.get_paper_links(paper_id, "citations").await
    }

    /// `/paper/{id}/references` / `/paper/{id}/citations` の共通実装。
    async fn get_paper_links(
        &self,
        paper_id: &str,
        kind: &str,
    ) -> Result<Vec<S2PaperDetail>, S2Error> {
        let ref_fields = PAPER_DETAIL_FIELDS
            .split(',')
            .map(|f| f.trim())
            .collect::<Vec<_>>()
            .join(",");
        let mut all: Vec<S2PaperDetail> = Vec::new();
        let mut offset: usize = 0;
        loop {
            let mut url = Url::parse(&format!(
                "{PAPER_API_BASE}/{}/{kind}",
                Self::encode_path_segment(paper_id),
            ))?;
            url.query_pairs_mut()
                .append_pair("fields", &ref_fields)
                .append_pair("offset", &offset.to_string())
                .append_pair("limit", "1000");
            let text = self.get_text(&url).await?;
            let page: S2RefPage =
                serde_json::from_str(&text).map_err(|e| S2Error::Decode(e.to_string()))?;
            let chunk = page
                .data
                .unwrap_or_default()
                .into_iter()
                .map(S2RefEntry::into_detail)
                .collect::<Vec<_>>();
            let got = chunk.len();
            all.extend(chunk);
            match page.next {
                Some(next_offset) if got > 0 && next_offset > offset => {
                    offset = next_offset;
                }
                _ => break,
            }
        }
        Ok(all)
    }

    pub async fn search(&self, query: &str, limit: u16) -> Result<Vec<RemotePaper>, S2Error> {
        self.search_with_offset(query, limit, 0, None).await
    }

    /// relevance search（`GET /graph/v1/paper/search`）。
    ///
    /// `year` には単年（`"2019"`）または閉区間（`"2010-2020"`）を指定できる
    /// （S2 Graph API の `year` パラメータ仕様）。`None` で全期間。
    /// 1 リクエストの `limit` 上限は 100。超過分は呼び出し側が offset ページングで集める。
    pub async fn search_with_offset(
        &self,
        query: &str,
        limit: u16,
        offset: u16,
        year: Option<&str>,
    ) -> Result<Vec<RemotePaper>, S2Error> {
        // S2 はキー付きでも断続的に 429 / 通信エラーを返す。指数バックオフで再送し、
        // Retry-After ヘッダがあれば尊重する（`get_text` と同一のループ型リトライ）。
        let mut attempts: u8 = 0;
        loop {
            self.throttle().await;
            let mut url = Url::parse(API_URL)?;
            url.query_pairs_mut()
                .append_pair("query", query)
                .append_pair("limit", &limit.to_string())
                .append_pair("offset", &offset.to_string());
            if let Some(year) = year {
                url.query_pairs_mut().append_pair("year", year);
            }
            url.query_pairs_mut().append_pair("fields", SEARCH_FIELDS);
            let mut req = self.client.get(url);
            if let Some(k) = &self.api_key {
                req = req.header("x-api-key", k);
            }
            let resp = match req.send().await {
                Ok(resp) => resp,
                // タイムアウト / 接続断は一時的な障害として再送する
                Err(e) => {
                    attempts += 1;
                    if attempts > MAX_RETRIES {
                        return Err(S2Error::Request(e));
                    }
                    let wait = 2u64.pow(attempts as u32).min(MAX_RETRY_SECS);
                    tokio::time::sleep(Duration::from_secs(wait)).await;
                    continue;
                }
            };
            if resp.status().as_u16() == 429 {
                attempts += 1;
                if attempts > MAX_RETRIES {
                    let root = crate::paths::project_root()
                        .map(|p| p.display().to_string())
                        .unwrap_or_else(|| ".".to_string());
                    return Err(S2Error::Decode(format!(
                        "S2 status 429 (rate limited) — set SEMANTICSCHOLAR_API_KEY in {root}/.env"
                    )));
                }
                let retry_secs = resp
                    .headers()
                    .get(reqwest::header::RETRY_AFTER)
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| v.parse::<u64>().ok())
                    .unwrap_or_else(|| 2u64.pow(attempts as u32).min(MAX_RETRY_SECS));
                tokio::time::sleep(Duration::from_secs(retry_secs)).await;
                continue;
            }
            if resp.status().as_u16() == 403 {
                return Err(S2Error::Decode(
                    "S2 status 403 Forbidden — API キーが無効です（環境変数 SEMANTICSCHOLAR_API_KEY を確認してください）"
                        .to_string(),
                ));
            }
            if !resp.status().is_success() {
                return Err(S2Error::Decode(format!("S2 status {}", resp.status())));
            }
            let text = resp
                .text()
                .await
                .map_err(|e| S2Error::Decode(e.to_string()))?;
            let body: S2SearchResponse =
                serde_json::from_str(&text).map_err(|e| S2Error::Decode(e.to_string()))?;
            return Ok(body
                .data
                .into_iter()
                .map(|p| {
                    // publicationDate（YYYY-MM-DD）を優先し、無ければ year の 1/1 にフォールバック
                    let date = p
                        .publication_date
                        .as_deref()
                        .and_then(|d| NaiveDate::parse_from_str(d, "%Y-%m-%d").ok())
                        .map(|d| d.and_hms_opt(0, 0, 0).unwrap().and_utc())
                        .unwrap_or_else(|| {
                            let year = p.year.unwrap_or(2000);
                            NaiveDate::from_ymd_opt(year, 1, 1)
                                .unwrap()
                                .and_hms_opt(0, 0, 0)
                                .unwrap()
                                .and_utc()
                        });
                    // ジャーナル名列（R4）: journal.name を優先し、無ければ venue
                    let journal_ref = p
                        .journal
                        .and_then(|j| j.name)
                        .filter(|name| !name.trim().is_empty())
                        .or(p.venue);
                    RemotePaper {
                        id: p.paper_id,
                        title: p.title.unwrap_or_default(),
                        authors: p
                            .authors
                            .unwrap_or_default()
                            .into_iter()
                            .filter_map(|a| a.name)
                            .collect(),
                        abstract_text: p.abstract_text.unwrap_or_default(),
                        published: date,
                        updated: date,
                        categories: vec![],
                        pdf_url: p
                            .open_access_pdf
                            .and_then(|x| x.url.filter(|u| !u.trim().is_empty()))
                            .or_else(|| {
                                p.external_ids.as_ref().and_then(|m| {
                                    m.get("ArXiv").and_then(|id| {
                                        let trimmed = id.trim();
                                        if trimmed.is_empty() {
                                            None
                                        } else {
                                            Some(format!("https://arxiv.org/pdf/{}.pdf", trimmed))
                                        }
                                    })
                                })
                            }),
                        doi: p.external_ids.as_ref().and_then(|m| m.get("DOI").cloned()),
                        journal_ref,
                        citation_count: p.citation_count,
                    }
                })
                .collect());
        }
    }
}
