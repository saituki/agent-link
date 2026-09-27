//! PubMed search via NCBI E-utilities API.
//!
//! NCBI E-utilities の esearch.fcgi と efetch.fcgi を使用して PubMed の論文を検索する。
//! API キー（NCBI_API_KEY）は任意。
//! - キーなし: 3 req/s
//! - キーあり: 10 req/s
//!
//! raw_query には PubMed Boolean 式をそのまま渡せる。
//! plain text は title + abstract のマルチフィールド検索にマップされる。

use crate::models::RemotePaper;
use chrono::NaiveDate;
use chrono::{DateTime, Utc};
use quick_xml::de::from_str;
use regex::Regex;
use reqwest::{Client, Url};
use serde::Deserialize;
use std::sync::{Arc, LazyLock};
use std::time::{Duration, Instant};
use thiserror::Error;
use tokio::sync::Mutex;

const ESEARCH_URL: &str = "https://eutils.ncbi.nlm.nih.gov/entrez/eutils/esearch.fcgi";
const EFETCH_URL: &str = "https://eutils.ncbi.nlm.nih.gov/entrez/eutils/efetch.fcgi";

/// API キーなし時のレート制限（3 req/s = 1 リクエスト/333ms）。
const REQUEST_INTERVAL_NO_KEY: Duration = Duration::from_millis(340);
/// API キーあり時のレート制限（10 req/s = 1 リクエスト/100ms）。
const REQUEST_INTERVAL_WITH_KEY: Duration = Duration::from_millis(110);

/// リトライ時の最大待機秒数。
#[allow(dead_code)]
const MAX_RETRY_SECS: u64 = 10;

/// 題名要素（開始タグ・終了タグの組）を取る正規表現。
static ARTICLE_TITLE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?s)(<ArticleTitle(?:\s[^>]*)?>)(.*?)(</ArticleTitle>)"#)
        .expect("ArticleTitle regex")
});

/// 現地語題名要素を取る正規表現。
static VERNACULAR_TITLE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?s)(<VernacularTitle(?:\s[^>]*)?>)(.*?)(</VernacularTitle>)"#)
        .expect("VernacularTitle regex")
});

/// 要旨要素を取る正規表現。属性（`Label="METHODS"` など）は開始タグごと残す。
static ABSTRACT_TEXT_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?s)(<AbstractText(?:\s[^>]*)?>)(.*?)(</AbstractText>)"#)
        .expect("AbstractText regex")
});

/// 任意の開始タグ・終了タグ・空要素タグ。
static ANY_TAG_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"<[^>]+>").expect("any tag regex"));

/// 要素の開始タグと終了タグの間にあるタグをすべて取り除き、中の文字だけを残す。
///
/// 要素名ごとの正規表現で「開始タグ・中身・終了タグ」を取り出し、
/// 真ん中の部分にだけタグ削除をかける（属性は開始タグに残る）。
fn strip_inner_tags(xml: &str, element: &Regex) -> String {
    element
        .replace_all(xml, |caps: &regex::Captures<'_>| {
            let inner = ANY_TAG_RE.replace_all(&caps[2], "");
            format!("{}{}{}", &caps[1], inner, &caps[3])
        })
        .into_owned()
}

/// PubMed XML をシンプル化：題名・要旨の中のタグをすべて取り除き、テキスト内容を保持する。
///
/// MathML（`<mml:math>` など）や `<ext-link>` などのタグが入ると、quick_xml の serde で
/// mixed content（テキスト＋ネストタグ）による "duplicate field $value" エラーになる。
/// 決まったタグの列表で specials ではなく、要素のなかのタグをすべて消す。
fn normalize_pubmed_xml(xml: &str) -> String {
    let result = strip_inner_tags(xml, &ARTICLE_TITLE_RE);
    let result = strip_inner_tags(&result, &VERNACULAR_TITLE_RE);
    strip_inner_tags(&result, &ABSTRACT_TEXT_RE)
}

/// `Sep` のような英語 3 文字や `9` のような数字を月の番号にする。
fn parse_month(raw: &str) -> Option<u32> {
    const MONTHS: [&str; 12] = [
        "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
    ];
    let s = raw.trim();
    if let Ok(month) = s.parse::<u32>() {
        return (1..=12).contains(&month).then_some(month);
    }
    MONTHS
        .iter()
        .position(|name| name.eq_ignore_ascii_case(s))
        .map(|i| i as u32 + 1)
}

/// 年・月・日を `NaiveDate` にする。月と日は無ければ 1 日とする。壊れた値は `None`。
fn build_naive_date(year: Option<u32>, month: Option<u32>, day: Option<u32>) -> Option<NaiveDate> {
    let year = year? as i32;
    let month = month.unwrap_or(1);
    let day = day.unwrap_or(1);
    NaiveDate::from_ymd_opt(year, month, day)
}

/// 論文の発行日を求める。
///
/// 1. `Article/ArticleDate`（電子版的発行日）
/// 2. `Article/Journal/JournalIssue/PubDate`（紙の発行日。`Month` は `Sep` でもよい）
/// 3. `PubDate/MedlineDate`（`2021 Jan-Feb` のときの先頭 4 桁を年とする）
///
/// どれも取れなかったときの既定は 2000-01-01。
fn resolve_published(article: &Article) -> DateTime<Utc> {
    let fallback = || {
        NaiveDate::from_ymd_opt(2000, 1, 1)
            .unwrap()
            .and_hms_opt(0, 0, 0)
            .unwrap()
            .and_utc()
    };

    let from_article_date = article.article_date.as_ref().and_then(|date| {
        let year = date.year.as_ref()?.trim().parse::<u32>().ok()?;
        let month = date.month.as_deref().and_then(parse_month);
        let day = date
            .day
            .as_deref()
            .and_then(|d| d.trim().parse::<u32>().ok());
        build_naive_date(Some(year), month, day)
    });
    if let Some(date) = from_article_date {
        return date.and_hms_opt(0, 0, 0).unwrap().and_utc();
    }

    let pub_date = article
        .journal
        .as_ref()
        .and_then(|j| j.journal_issue.as_ref())
        .and_then(|ji| ji.pub_date.as_ref());

    if let Some(pub_date) = pub_date {
        if let Some(date) = pub_date
            .year
            .as_deref()
            .and_then(|y| y.trim().parse::<u32>().ok())
            .and_then(|year| {
                build_naive_date(
                    Some(year),
                    pub_date.month.as_deref().and_then(parse_month),
                    pub_date
                        .day
                        .as_deref()
                        .and_then(|d| d.trim().parse::<u32>().ok()),
                )
            })
        {
            return date.and_hms_opt(0, 0, 0).unwrap().and_utc();
        }

        // MedlineDate（例: "2021 Jan-Feb"）は先頭の 4 桁を年として使う
        if let Some(year) = pub_date
            .medline_date
            .as_deref()
            .and_then(|s| s.get(..4))
            .and_then(|y| y.parse::<u32>().ok())
        {
            if let Some(date) = build_naive_date(Some(year), None, None) {
                return date.and_hms_opt(0, 0, 0).unwrap().and_utc();
            }
        }
    }

    fallback()
}

/// 記事の DOI（素の `10.` 始まり）を探す。
///
/// 1. `Article/ELocationID[@EIdType="doi"]`
/// 2. `PubmedArticle/PubmedData/ArticleIdList/ArticleId[@IdType="doi"]`
///
/// 共有契約 C2: `https://doi.org/` を付けない素の DOI を返す。
fn resolve_doi(article: &Article, pubmed_data: Option<&PubmedData>) -> Option<String> {
    let from_elocation = article.elocation_ids.as_ref().and_then(|ids| {
        ids.iter()
            .find(|id| {
                id.eid_type
                    .as_deref()
                    .is_some_and(|t| t.eq_ignore_ascii_case("doi"))
            })
            .and_then(|id| id.value.clone())
    });
    if let Some(doi) = from_elocation.filter(|d| !d.trim().is_empty()) {
        return Some(doi);
    }

    let from_pubmed_data = pubmed_data
        .and_then(|d| d.article_id_list.as_ref())
        .and_then(|list| {
            list.article_ids
                .iter()
                .find(|id| {
                    id.id_type
                        .as_deref()
                        .is_some_and(|t| t.eq_ignore_ascii_case("doi"))
                })
                .and_then(|id| id.value.clone())
        });
    from_pubmed_data.filter(|d| !d.trim().is_empty())
}

/// テキストコンテンツをネストされたタグを無視して抽出する。
/// <i>, <b>, <sub>, <sup> など、タイトルやアブストラクト内のフォーマットタグを処理する。
fn extract_text_content<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde_json::Value;

    let value = Value::deserialize(deserializer)?;

    if let Some(s) = value.as_str() {
        Ok(Some(s.to_string()))
    } else if value.is_null() {
        Ok(None)
    } else if let Some(obj) = value.as_object() {
        // オブジェクト内のすべての文字列値を再帰的に連結
        let mut parts = Vec::new();
        collect_text_from_value(obj, &mut parts);
        if parts.is_empty() {
            Ok(None)
        } else {
            Ok(Some(parts.join("")))
        }
    } else if let Some(arr) = value.as_array() {
        // 配列の場合、すべての要素から文字列を抽出
        let mut parts = Vec::new();
        for item in arr {
            if let Some(s) = item.as_str() {
                parts.push(s.to_string());
            }
        }
        if parts.is_empty() {
            Ok(None)
        } else {
            Ok(Some(parts.join("")))
        }
    } else {
        Ok(None)
    }
}

/// serde_json の Value から全テキストコンテンツを再帰的に抽出する。
fn collect_text_from_value(
    obj: &serde_json::Map<String, serde_json::Value>,
    parts: &mut Vec<String>,
) {
    for (_, v) in obj.iter() {
        match v {
            serde_json::Value::String(s) => {
                parts.push(s.clone());
            }
            serde_json::Value::Object(nested) => {
                collect_text_from_value(nested, parts);
            }
            serde_json::Value::Array(arr) => {
                for item in arr {
                    if let serde_json::Value::String(s) = item {
                        parts.push(s.clone());
                    } else if let serde_json::Value::Object(nested) = item {
                        collect_text_from_value(nested, parts);
                    }
                }
            }
            _ => {}
        }
    }
}

/// PubMed API が返すエラー。
#[derive(Debug, Error)]
pub enum PubmedError {
    /// URL パース失敗。
    #[error("invalid URL: {0}")]
    Url(#[from] url::ParseError),
    /// ネットワークリクエスト失敗（URL は外してある。`api_key` が URL に入るため）。
    #[error("network error: {0}")]
    Request(reqwest::Error),
    /// XML パース失敗。
    #[error("XML parse error: {0}")]
    ParseXml(#[from] quick_xml::de::DeError),
    /// API エラーレスポンス。
    #[error("API error: {0}")]
    ApiError(String),
    /// 論文が見つからない。
    ///
    /// `search` / `search_raw` は 0 件で `Ok(vec![])` を返すので、これらは使わない。
    #[error("no results found")]
    NotFound,
}

impl From<reqwest::Error> for PubmedError {
    /// NCBI の API キーは URL の `api_key` に入るので、エラー文に URL を残さない。
    fn from(error: reqwest::Error) -> Self {
        Self::Request(error.without_url())
    }
}

/// PubMed esearch API レスポンス。
#[derive(Debug, Deserialize)]
pub struct EsearchResponse {
    /// 検索結果の総件数。
    #[serde(rename = "Count")]
    pub count: String,
    /// 取得した論文 ID のリスト。
    #[serde(rename = "IdList")]
    pub id_list: Option<IdList>,
}

/// esearch API が返す ID リスト。
#[derive(Debug, Deserialize)]
pub struct IdList {
    /// PubMed ID の配列。
    #[serde(rename = "Id", default)]
    pub ids: Vec<String>,
}

/// PubMed efetch API レスポンス（PubmedArticleSet）。
#[derive(Debug, Deserialize)]
pub struct PubmedArticleSet {
    /// 検索結果の論文配列。
    #[serde(rename = "PubmedArticle", default)]
    pub articles: Vec<PubmedArticle>,
}

/// 1 つの PubMed 論文エントリ。
#[derive(Debug, Deserialize)]
pub struct PubmedArticle {
    /// MedLine 引用メタデータ。
    #[serde(rename = "MedlineCitation")]
    pub citation: Option<MedlineCitation>,
    /// 論文の詳細情報。
    #[serde(rename = "Article")]
    pub article: Option<Article>,
    /// PubMed 独自のデータ（ArticleIdList に DOI が入る）。
    #[serde(rename = "PubmedData", default)]
    pub pubmed_data: Option<PubmedData>,
}

/// `PubmedArticle/PubmedData`。
#[derive(Debug, Deserialize, Clone)]
pub struct PubmedData {
    /// 論文 ID のリスト。
    #[serde(rename = "ArticleIdList", default)]
    pub article_id_list: Option<ArticleIdList>,
}

/// `PubmedData/ArticleIdList`。
#[derive(Debug, Deserialize, Clone)]
pub struct ArticleIdList {
    /// 論文 ID の配列（`doi`, `pmc`, `pubmed` など）。
    #[serde(rename = "ArticleId", default)]
    pub article_ids: Vec<ArticleId>,
}

/// 1 つの `ArticleId`（属性 `IdType` を持つ）。
#[derive(Debug, Deserialize, Clone)]
pub struct ArticleId {
    /// ID の種類（"doi", "pmc", "pubmed" など）。
    #[serde(rename = "@IdType", default)]
    pub id_type: Option<String>,
    /// ID の値。
    #[serde(rename = "$value", default)]
    pub value: Option<String>,
}

/// PubMed MedLine 引用情報。
#[derive(Debug, Deserialize, Clone)]
pub struct MedlineCitation {
    /// PMID（PubMed ID）。
    #[serde(rename = "PMID")]
    pub pmid: Option<String>,
    /// 論文の詳細情報。
    #[serde(rename = "Article")]
    pub article: Option<Article>,
}

/// PubMed 論文の詳細情報。
#[derive(Debug, Deserialize, Clone)]
pub struct Article {
    /// 論文のタイトル。<i>, <b> などのネストされたタグを含む場合がある。
    #[serde(rename = "ArticleTitle", default)]
    #[serde(deserialize_with = "extract_text_content")]
    pub title: Option<String>,
    /// 発行されたジャーナル。
    #[serde(rename = "Journal", default)]
    pub journal: Option<Journal>,
    /// 著者のリスト。
    #[serde(rename = "AuthorList", default)]
    pub authors: Option<AuthorList>,
    /// 論文のアブストラクト。
    #[serde(rename = "Abstract", default)]
    pub abstract_text: Option<Abstract>,
    /// 出版物タイプ。
    #[serde(rename = "PublicationTypeList", default)]
    pub publication_types: Option<PublicationTypeList>,
    /// 記事の日付。
    #[serde(rename = "ArticleDate", default)]
    pub article_date: Option<ArticleDate>,
    /// 言語（オプション）。
    #[serde(rename = "Language", default)]
    pub language: Option<String>,
    /// 電子版の識別子（`pii`, `doi` 等）。`doi` を探すのに使う。
    #[serde(rename = "ELocationID", default)]
    pub elocation_ids: Option<Vec<ELocationId>>,
}

/// 1 つの `ELocationID`（属性 `EIdType` を持つ）。
#[derive(Debug, Deserialize, Clone)]
pub struct ELocationId {
    /// ID の種類（"pii", "doi" など）。
    #[serde(rename = "@EIdType", default)]
    pub eid_type: Option<String>,
    /// ID の値。
    #[serde(rename = "$value", default)]
    pub value: Option<String>,
}

/// ジャーナル情報。
#[derive(Debug, Deserialize, Clone)]
pub struct Journal {
    /// ジャーナルのタイトル。
    #[serde(rename = "Title")]
    pub title: Option<String>,
    /// 号の情報（発行日の第 2 の候補）。
    #[serde(rename = "JournalIssue", default)]
    pub journal_issue: Option<JournalIssue>,
}

/// ジャーナルの号。
#[derive(Debug, Deserialize, Clone)]
pub struct JournalIssue {
    /// 号の発行日。
    #[serde(rename = "PubDate", default)]
    pub pub_date: Option<PubDate>,
}

/// ジャーナル号の発行日。
#[derive(Debug, Deserialize, Clone)]
pub struct PubDate {
    /// 発行年。
    #[serde(rename = "Year", default)]
    pub year: Option<String>,
    /// 発行月。`Sep` のような英語 3 文字もある。
    #[serde(rename = "Month", default)]
    pub month: Option<String>,
    /// 発行日。
    #[serde(rename = "Day", default)]
    pub day: Option<String>,
    /// `2021 Jan-Feb` のような範囲表記（`Year` が無いときの候補）。
    #[serde(rename = "MedlineDate", default)]
    pub medline_date: Option<String>,
}

/// 著者リスト。
#[derive(Debug, Deserialize, Clone)]
pub struct AuthorList {
    /// 著者の配列。
    #[serde(rename = "Author", default)]
    pub authors: Vec<Author>,
}

/// 1 人の著者。
#[derive(Debug, Deserialize, Clone)]
pub struct Author {
    /// 著者の名字。
    #[serde(rename = "LastName")]
    pub last_name: Option<String>,
    /// 著者の名前。
    #[serde(rename = "ForeName")]
    pub fore_name: Option<String>,
}

/// アブストラクトテキスト要素。複数の `AbstractText` 要素を Vec で保持する。
///
/// quick_xml でネストされたタグを処理するため、全フィールドを見て
/// 手動でテキストを再構成する。
#[derive(Debug, Deserialize, Clone)]
#[serde(from = "AbstractTextRaw")]
pub struct AbstractText {
    /// アブストラクトのテキスト内容。<i>, <b> などのネストされたタグを含む場合がある。
    pub text: Option<String>,
    /// Label 属性（オプション）。"BACKGROUND", "METHODS" など。
    pub label: Option<String>,
}

/// quick_xml デシリアライズの中間構造。
///
/// `normalize_pubmed_xml` が要旨の中のタグをすべて取り除くので、
/// `$value` と `@Label` だけを持つ。
#[derive(Debug, Deserialize, Clone)]
struct AbstractTextRaw {
    /// メインのテキストコンテンツ。
    #[serde(rename = "$value", default)]
    main_text: Option<String>,
    /// Label 属性。
    #[serde(rename = "@Label", default)]
    label: Option<String>,
}

impl From<AbstractTextRaw> for AbstractText {
    fn from(raw: AbstractTextRaw) -> Self {
        AbstractText {
            text: raw.main_text,
            label: raw.label,
        }
    }
}

/// 複数の AbstractText 要素を持つ Abstract 構造。
#[derive(Debug, Deserialize, Clone)]
pub struct Abstract {
    /// 複数の AbstractText 要素の配列。
    #[serde(rename = "AbstractText", default)]
    pub texts: Vec<AbstractText>,
    /// 著作権情報（Abstract の子要素）。
    #[serde(rename = "CopyrightInformation", default)]
    pub copyright_information: Option<String>,
}

impl Abstract {
    /// 複数の AbstractText 要素を 1 つの文字列に連結する。
    /// ラベルがあれば "LABEL: テキスト" の形式、なければテキストのみを連結する。
    pub fn combined_text(&self) -> String {
        self.texts
            .iter()
            .filter_map(|at| {
                at.text.as_ref().map(|text| {
                    if let Some(label) = &at.label {
                        format!("{}: {}", label, text)
                    } else {
                        text.clone()
                    }
                })
            })
            .collect::<Vec<_>>()
            .join(" ")
    }
}

/// 記事の発行日。
#[derive(Debug, Deserialize, Clone)]
pub struct ArticleDate {
    /// 発行年。
    #[serde(rename = "Year")]
    pub year: Option<String>,
    /// 発行月。
    #[serde(rename = "Month")]
    pub month: Option<String>,
    /// 発行日。
    #[serde(rename = "Day")]
    pub day: Option<String>,
}

/// 出版物タイプのリスト。
#[derive(Debug, Deserialize, Clone)]
pub struct PublicationTypeList {
    /// 出版物タイプの配列。
    #[serde(rename = "PublicationType", default)]
    pub types: Vec<String>,
}

/// 設定済み、再利用可能な PubMed API クライアント。
#[derive(Debug, Clone)]
pub struct PubmedClient {
    client: Client,
    api_key: Option<String>,
    next_request_at: Arc<Mutex<Instant>>,
}

impl PubmedClient {
    /// 新しい PubMed クライアントを構築する。
    ///
    /// API キーは環境変数 `NCBI_API_KEY` から読み込まれる。
    pub fn new(api_key: Option<String>) -> Result<Self, PubmedError> {
        Ok(Self {
            client: Client::builder()
                .user_agent(concat!("sai-paper/", env!("CARGO_PKG_VERSION")))
                .timeout(Duration::from_secs(30))
                .build()?,
            api_key,
            next_request_at: Arc::new(Mutex::new(Instant::now())),
        })
    }

    /// リクエスト前にレート制限を適用する。
    async fn throttle(&self) {
        let interval = if self.api_key.is_some() {
            REQUEST_INTERVAL_WITH_KEY
        } else {
            REQUEST_INTERVAL_NO_KEY
        };
        let mut next = self.next_request_at.lock().await;
        let wait = next.saturating_duration_since(Instant::now());
        if !wait.is_zero() {
            tokio::time::sleep(wait).await;
        }
        *next = Instant::now() + interval;
    }

    /// SearchQuery テキストから PubMed 検索式を構築する。
    /// 簡易版: title と abstract のマルチフィールド検索に変換。
    fn build_query(text: &str) -> String {
        format!("({} [Title] OR {} [Abstract])", text, text)
    }

    /// esearch で論文 ID を検索する。
    async fn esearch(&self, query: &str, retmax: u16) -> Result<Vec<String>, PubmedError> {
        self.throttle().await;

        let mut url = Url::parse(ESEARCH_URL)?;
        url.query_pairs_mut()
            .append_pair("db", "pubmed")
            .append_pair("term", query)
            .append_pair("retmax", &retmax.to_string())
            .append_pair("tool", "sai-paper")
            .append_pair("email", "sai@example.com");
        if let Some(key) = &self.api_key {
            url.query_pairs_mut().append_pair("api_key", key);
        }

        let resp = self.client.get(url).send().await?;
        let text = resp.text().await?;

        let esr: EsearchResponse = from_str(&text)
            .map_err(|e| PubmedError::ApiError(format!("esearch parse failed: {}", e)))?;

        Ok(esr.id_list.map(|ids| ids.ids).unwrap_or_default())
    }

    /// efetch で論文の詳細を取得する。
    /// 1 件の解析失敗は警告として無視し、解析できた論文のみを返す。
    async fn efetch(&self, pmids: &[String]) -> Result<Vec<RemotePaper>, PubmedError> {
        if pmids.is_empty() {
            return Ok(Vec::new());
        }

        self.throttle().await;

        let mut url = Url::parse(EFETCH_URL)?;
        let pmid_str = pmids.join(",");
        url.query_pairs_mut()
            .append_pair("db", "pubmed")
            .append_pair("id", &pmid_str)
            .append_pair("rettype", "xml")
            .append_pair("tool", "sai-paper")
            .append_pair("email", "sai@example.com");
        if let Some(key) = &self.api_key {
            url.query_pairs_mut().append_pair("api_key", key);
        }

        let resp = self.client.get(url).send().await?;
        let text = resp.text().await?;

        // XMLをシンプルなテキスト抽出で前処理：mixed content（テキスト＋ネストタグ）を処理
        let normalized_text = normalize_pubmed_xml(&text);

        let article_set: PubmedArticleSet = from_str(&normalized_text)
            .map_err(|e| PubmedError::ApiError(format!("efetch parse failed: {}", e)))?;

        let papers = article_set
            .articles
            .into_iter()
            .filter_map(|article| {
                let pmid = article
                    .citation
                    .as_ref()
                    .and_then(|c| c.pmid.as_ref().cloned());

                let article_node = article.article.or_else(|| {
                    article
                        .citation
                        .as_ref()
                        .and_then(|c| c.article.as_ref().cloned())
                })?;

                let doi = resolve_doi(&article_node, article.pubmed_data.as_ref());

                let title = article_node.title.clone().unwrap_or_default();
                let authors: Vec<String> = article_node
                    .authors
                    .as_ref()
                    .map(|al| {
                        al.authors
                            .iter()
                            .filter_map(|a| {
                                let last = a.last_name.as_ref().map(|s| s.as_str()).unwrap_or("");
                                let first = a.fore_name.as_ref().map(|s| s.as_str()).unwrap_or("");
                                if last.is_empty() {
                                    None
                                } else if first.is_empty() {
                                    Some(last.to_string())
                                } else {
                                    Some(format!("{} {}", last, first))
                                }
                            })
                            .collect()
                    })
                    .unwrap_or_default();

                let abstract_text = article_node
                    .abstract_text
                    .as_ref()
                    .map(|a| a.combined_text())
                    .unwrap_or_default();

                let published = resolve_published(&article_node);

                let journal_ref = article_node
                    .journal
                    .as_ref()
                    .and_then(|j| j.title.as_ref().cloned());

                Some(RemotePaper {
                    id: pmid.unwrap_or_else(|| title.clone()),
                    title,
                    authors,
                    abstract_text,
                    published,
                    updated: published,
                    categories: vec!["PubMed".to_string()],
                    pdf_url: None,
                    doi,
                    journal_ref,
                    citation_count: None,
                })
            })
            .collect();

        Ok(papers)
    }

    /// PMID を 1 件だけ引く。
    ///
    /// 見つからないときは `Ok(None)` を返す。
    ///
    /// # Errors
    ///
    /// ネットワーク・解析の失敗。
    pub async fn get_by_pmid(&self, pmid: &str) -> Result<Option<RemotePaper>, PubmedError> {
        let mut papers = self.efetch(&[pmid.to_string()]).await?;
        Ok(papers.pop())
    }

    /// 統合検索インターフェース。
    ///
    /// 0 件はエラーではなく `Ok(vec![])` にする。
    pub async fn search(&self, text: &str, limit: u16) -> Result<Vec<RemotePaper>, PubmedError> {
        let query = Self::build_query(text);
        let ids = self.esearch(&query, limit).await?;

        if ids.is_empty() {
            return Ok(Vec::new());
        }

        self.efetch(&ids).await
    }

    /// raw_query 対応検索（Boolean 式をそのまま使用）。
    ///
    /// 0 件はエラーではなく `Ok(vec![])` にする。
    pub async fn search_raw(
        &self,
        raw_query: &str,
        limit: u16,
    ) -> Result<Vec<RemotePaper>, PubmedError> {
        let ids = self.esearch(raw_query, limit).await?;

        if ids.is_empty() {
            return Ok(Vec::new());
        }

        self.efetch(&ids).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pubmed_article_parsing() {
        // 簡略版の PubMed XML を使った単体テスト（Abstract 要素は削除）
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE PubmedArticleSet PUBLIC "-//NLM//DTD PubMedArticle, 1st January 2019//EN"
"https://dtd.nlm.nih.gov/ncbi/pubmed/out/pubmed_190101.dtd">
<PubmedArticleSet>
  <PubmedArticle>
    <MedlineCitation>
      <PMID Version="1">12345678</PMID>
      <Article>
        <ArticleTitle>Sample Paper Title</ArticleTitle>
        <Journal>
          <Title>Sample Journal</Title>
        </Journal>
        <AuthorList>
          <Author>
            <LastName>Doe</LastName>
            <ForeName>John</ForeName>
          </Author>
        </AuthorList>
        <ArticleDate>
          <Year>2020</Year>
          <Month>1</Month>
          <Day>15</Day>
        </ArticleDate>
      </Article>
    </MedlineCitation>
  </PubmedArticle>
</PubmedArticleSet>"#;

        let result: PubmedArticleSet = from_str(xml).expect("parsing failed");
        assert_eq!(result.articles.len(), 1);

        let article = &result.articles[0];
        assert_eq!(
            article
                .citation
                .as_ref()
                .unwrap()
                .pmid
                .as_ref()
                .map(|s| s.as_str()),
            Some("12345678")
        );

        // PMID をテスト
        assert!(article.citation.is_some());
    }

    #[test]
    fn test_pubmed_multiple_abstract_text() {
        // 実際の API レスポンス形式（複数の AbstractText と CopyrightInformation を含む）
        // テスト_pubmed_real_api_response で詳細なテストを実施しているため、ここは簡略版
        let xml = r#"<?xml version="1.0"?>
<PubmedArticleSet>
<PubmedArticle><MedlineCitation><PMID Version="1">38000000</PMID><Article><ArticleTitle>Test</ArticleTitle><Journal><Title>Test Journal</Title></Journal><Abstract><AbstractText>Abstract content here.</AbstractText></Abstract></Article></MedlineCitation></PubmedArticle>
</PubmedArticleSet>"#;

        let result: PubmedArticleSet = from_str(xml).expect("parsing failed");
        assert_eq!(result.articles.len(), 1);

        let article = &result.articles[0];
        // 記事が正しく解析されたことを確認
        assert!(article.citation.is_some());
    }

    #[test]
    fn test_pubmed_real_api_response() {
        // 実際の API レスポンス形式（CopyrightInformation, ELocationID 等を含む）
        let xml = r#"<?xml version="1.0" ?>
<!DOCTYPE PubmedArticleSet PUBLIC "-//NLM//DTD PubMedArticle, 1st January 2025//EN" "https://dtd.nlm.nih.gov/ncbi/pubmed/out/pubmed_250101.dtd">
<PubmedArticleSet>
<PubmedArticle><MedlineCitation Status="Publisher" Owner="NLM"><PMID Version="1">42762415</PMID><DateRevised><Year>2026</Year><Month>09</Month><Day>19</Day></DateRevised><Article PubModel="Print-Electronic"><Journal><ISSN IssnType="Electronic">1573-4919</ISSN><Title>Molecular and cellular biochemistry</Title></Journal><ArticleTitle>NRF2 signaling drives chemoresistance in NSCLC and glioblastoma.</ArticleTitle><ELocationID EIdType="doi" ValidYN="Y">10.1007/s11010-026-05747-z</ELocationID><Abstract><AbstractText>Although great advances have been made in cancer treatment, lung cancer and glioblastoma patients continue to present a dismal prognosis, mainly due to drug resistance. Evidence suggests that NRF2 and glutathione (GSH) play a fundamental role in chemotherapy resistance. Notably, GSH depletion by buthionine sulfoximine (BSO) has been demonstrated to sensitize human tumor cells to a wide variety of chemotherapeutic agents. Furthermore, BSO in combination with TMZ or cisplatin enhanced chemotherapy-induced cytotoxicity and engaged both apoptotic and ferroptosis-related responses in NSCLC cells. These findings provide preclinical evidence supporting further investigation of NRF2/GSH targeting as a strategy to overcome chemotherapy resistance.</AbstractText></Abstract><CopyrightInformation>© 2026. The Author(s).</CopyrightInformation><AuthorList CompleteYN="Y"><Author ValidYN="Y"><LastName>de Souza</LastName><ForeName>Izadora</ForeName></Author></AuthorList><Language>eng</Language><PublicationTypeList><PublicationType UI="D016428">Journal Article</PublicationType></PublicationTypeList><ArticleDate DateType="Electronic"><Year>2026</Year><Month>09</Month><Day>19</Day></ArticleDate></Article></MedlineCitation></PubmedArticle>
</PubmedArticleSet>"#;

        let result: PubmedArticleSet = from_str(xml).expect("parsing failed");
        assert_eq!(result.articles.len(), 1);

        let article = &result.articles[0];
        let title = article
            .citation
            .as_ref()
            .and_then(|c| c.article.as_ref())
            .and_then(|a| a.title.as_ref())
            .map(|s| s.as_str());

        assert_eq!(
            title,
            Some("NRF2 signaling drives chemoresistance in NSCLC and glioblastoma.")
        );
    }

    #[test]
    fn test_pubmed_multiple_abstract_text_with_labels() {
        // 複数の AbstractText 要素（ラベル付き）を持つ Abstract のテスト。
        // これは以前のバグの直接的な原因となったケース。
        let xml = r#"<?xml version="1.0" ?>
<PubmedArticleSet>
<PubmedArticle>
  <MedlineCitation>
    <PMID Version="1">99999999</PMID>
    <Article>
      <ArticleTitle>Multi-section abstract test</ArticleTitle>
      <Journal>
        <Title>Test Journal</Title>
      </Journal>
      <Abstract>
        <AbstractText Label="BACKGROUND">This study investigates cancer mechanisms.</AbstractText>
        <AbstractText Label="METHODS">We used multiple analytical techniques.</AbstractText>
        <AbstractText Label="RESULTS">Results showed significant findings.</AbstractText>
        <AbstractText Label="CONCLUSIONS">Further research is needed.</AbstractText>
      </Abstract>
    </Article>
  </MedlineCitation>
</PubmedArticle>
</PubmedArticleSet>"#;

        let result: PubmedArticleSet = from_str(xml).expect("parsing failed");
        assert_eq!(result.articles.len(), 1);

        let article = &result.articles[0];
        let abstract_option = article
            .citation
            .as_ref()
            .and_then(|c| c.article.as_ref())
            .and_then(|a| a.abstract_text.as_ref());

        assert!(abstract_option.is_some());
        let abstract_text = abstract_option.unwrap();

        // 複数の AbstractText 要素が Vec に保存されていることを確認
        assert_eq!(abstract_text.texts.len(), 4);
        // combined_text メソッドがラベル付きで連結することを確認
        let combined = abstract_text.combined_text();
        assert!(combined.contains("BACKGROUND: This study investigates cancer mechanisms."));
        assert!(combined.contains("METHODS: We used multiple analytical techniques."));
        assert!(combined.contains("RESULTS: Results showed significant findings."));
        assert!(combined.contains("CONCLUSIONS: Further research is needed."));
    }

    /// AC1: 題名と要旨に MathML（`<mml:math>` など）が入る XML でも解析でき、
    /// タグが消えて中の文字が残る。
    #[test]
    fn pubmed_mathml_in_title_and_abstract() {
        let xml = r#"<?xml version="1.0" ?>
<PubmedArticleSet>
<PubmedArticle><MedlineCitation><PMID Version="1">40000001</PMID><Article><ArticleTitle>Ca<sup>2+</sup> dynamics in <mml:math><mml:mrow><mml:mi>x</mml:mi></mml:mrow></mml:math> space</ArticleTitle><Journal><Title>J Math</Title></Journal><Abstract><AbstractText Label="RESULTS">Ca<sup>2+</sup> rises by <mml:math><mml:mi>x</mml:mi></mml:math> %</AbstractText></Abstract></Article></MedlineCitation></PubmedArticle>
</PubmedArticleSet>"#;

        // 前処理後の XML: 要素の中のタグがすべて消えている
        let normalized = normalize_pubmed_xml(xml);
        assert!(!normalized.contains("mml:math"));
        assert!(!normalized.contains("<sup>"));
        assert!(normalized.contains("<AbstractText Label=\"RESULTS\">"));

        let result: PubmedArticleSet = from_str(&normalized).expect("parsing failed");
        let article = result.articles[0]
            .citation
            .as_ref()
            .and_then(|c| c.article.as_ref())
            .expect("article");

        // タグは消えても中の文字は残る
        assert_eq!(article.title.as_deref(), Some("Ca2+ dynamics in x space"));

        let combined = article
            .abstract_text
            .as_ref()
            .expect("abstract")
            .combined_text();
        assert_eq!(combined, "RESULTS: Ca2+ rises by x %");
    }

    /// AC2: `ArticleDate` が無く `JournalIssue/PubDate` だけがあるとき、
    /// 英語 3 文字の `Month` を読めて 2021-09-01 になる。
    #[test]
    fn pubmed_published_from_journal_issue_pub_date() {
        let xml = r#"<Article><Journal><Title>J Chem</Title><JournalIssue><PubDate><Year>2021</Year><Month>Sep</Month></PubDate></JournalIssue></Journal></Article>"#;
        let article: Article = from_str(xml).expect("parsing failed");

        assert_eq!(
            resolve_published(&article).date_naive(),
            NaiveDate::from_ymd_opt(2021, 9, 1).unwrap()
        );

        // `ArticleDate` があればそちらを優先する
        let with_article_date = r#"<Article><ArticleDate><Year>2022</Year><Month>3</Month><Day>4</Day></ArticleDate><Journal><JournalIssue><PubDate><Year>2021</Year><Month>Sep</Month></PubDate></JournalIssue></Journal></Article>"#;
        let article: Article = from_str(with_article_date).expect("parsing failed");
        assert_eq!(
            resolve_published(&article).date_naive(),
            NaiveDate::from_ymd_opt(2022, 3, 4).unwrap()
        );

        // どれも無ければ 2000-01-01
        let bare: Article = from_str(r#"<Article><Journal><Title>J</Title></Journal></Article>"#)
            .expect("parsing failed");
        assert_eq!(
            resolve_published(&bare).date_naive(),
            NaiveDate::from_ymd_opt(2000, 1, 1).unwrap()
        );
    }

    /// AC2: `ELocationID EIdType="doi"` から素の DOI が入る。
    #[test]
    fn pubmed_doi_from_elocation_id() {
        let xml = r#"<PubmedArticle><MedlineCitation><PMID Version="1">1</PMID><Article><ArticleTitle>T</ArticleTitle><ELocationID EIdType="pii">S0000000000000000</ELocationID><ELocationID EIdType="doi">10.1007/s11010-026-05747-z</ELocationID></Article></MedlineCitation><PubmedData><ArticleIdList><ArticleId IdType="pubmed">1</ArticleId><ArticleId IdType="doi">9.9999/other</ArticleId></ArticleIdList></PubmedData></PubmedArticle>"#;
        let parsed: PubmedArticle = from_str(xml).expect("parsing failed");
        let article = parsed
            .citation
            .as_ref()
            .and_then(|c| c.article.as_ref())
            .expect("article");

        // ELocationID の doi を優先する
        assert_eq!(
            resolve_doi(article, parsed.pubmed_data.as_ref()).as_deref(),
            Some("10.1007/s11010-026-05747-z")
        );

        // ELocationID に doi が無ければ PubmedData の ArticleId を見る
        let xml2 = r#"<PubmedArticle><MedlineCitation><PMID Version="1">1</PMID><Article><ArticleTitle>T</ArticleTitle></Article></MedlineCitation><PubmedData><ArticleIdList><ArticleId IdType="pubmed">1</ArticleId><ArticleId IdType="doi">10.64898/2026.09.18.751510</ArticleId></ArticleIdList></PubmedData></PubmedArticle>"#;
        let parsed: PubmedArticle = from_str(xml2).expect("parsing failed");
        let article = parsed
            .citation
            .as_ref()
            .and_then(|c| c.article.as_ref())
            .expect("article");
        assert_eq!(
            resolve_doi(article, parsed.pubmed_data.as_ref()).as_deref(),
            Some("10.64898/2026.09.18.751510")
        );
    }
}
