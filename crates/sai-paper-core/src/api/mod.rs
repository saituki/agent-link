//! Clients for external scholarly metadata providers.

use crate::models::RemotePaper;
use thiserror::Error;

pub mod arxiv;
/// Client for bioRxiv and medRxiv preprints.
pub mod biorxiv;
/// 検索結果の被引用数を OpenAlex でまとめて補う。
pub mod citations;
/// Client for the Crossref API.
pub mod crossref;
/// Client for OpenAlex scholarly metadata.
pub mod openalex;
/// Client for PubMed via NCBI E-utilities.
pub mod pubmed;
/// Client for Semantic Scholar (S2) Graph API.
pub mod s2;

/// 統一された論文検索クエリパラメータ。
///
/// 各 PaperSource 実装が共通で受け付けるクエリ形式を定義する。
#[derive(Debug, Clone)]
pub struct SearchQuery {
    /// 自然言語検索テキスト（DB 固有の構文なし）。
    pub text: String,
    /// DB 固有の構文をそのまま渡す口（PubMed Boolean など）。
    /// 指定時は `text` を無視する実装もある。
    pub raw_query: Option<String>,
    /// 発行年の下限（inclusive）。
    pub year_from: Option<u16>,
    /// 発行年の上限（inclusive）。
    pub year_to: Option<u16>,
    /// 1 回のリクエストで返す最大件数。
    pub limit: u16,
    /// ページング用のオフセット。
    pub offset: u32,
}

impl SearchQuery {
    /// 新しい検索クエリを作成する。
    ///
    /// デフォルト値: limit=10, offset=0
    #[must_use]
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            raw_query: None,
            year_from: None,
            year_to: None,
            limit: 10,
            offset: 0,
        }
    }

    /// 検索クエリにレート制限情報を設定する。
    #[must_use]
    pub fn with_limit(mut self, limit: u16) -> Self {
        self.limit = limit;
        self
    }

    /// 検索クエリにオフセット情報を設定する。
    #[must_use]
    pub fn with_offset(mut self, offset: u32) -> Self {
        self.offset = offset;
        self
    }

    /// 検索クエリに年度範囲を設定する。
    #[must_use]
    pub fn with_years(mut self, from: Option<u16>, to: Option<u16>) -> Self {
        self.year_from = from;
        self.year_to = to;
        self
    }

    /// 検索クエリに生のクエリ式を設定する。
    #[must_use]
    pub fn with_raw_query(mut self, raw: impl Into<String>) -> Self {
        self.raw_query = Some(raw.into());
        self
    }
}

/// 論文ソースの統一インターフェース。
///
/// 各外部 API（S2、arXiv、PubMed、bioRxiv、OpenAlex）はこのトレイトを実装し、
/// 統一された検索インターフェースを提供する。
#[async_trait::async_trait]
pub trait PaperSource: Send + Sync {
    /// このソースの識別子（"s2", "arxiv", "pubmed", "biorxiv", "openalex"）。
    fn id(&self) -> &'static str;

    /// 検索クエリを実行し、論文リストを返す。
    ///
    /// # Errors
    ///
    /// ネットワークエラーやパースエラーが発生した場合、エラーを返す。
    async fn search(&self, q: &SearchQuery) -> Result<Vec<RemotePaper>, PaperSourceError>;
}

/// 論文ソース実装が返すエラー型。
#[derive(Debug, Error)]
pub enum PaperSourceError {
    /// ネットワークリクエストが失敗した。
    #[error("network error: {0}")]
    Network(String),
    /// レスポンスのパースに失敗した。
    #[error("parse error: {0}")]
    Parse(String),
    /// API が特定のエラー（404、403 等）を返した。
    #[error("API error: {0}")]
    Api(String),
    /// レート制限に達した。
    #[error("rate limited: {0}")]
    RateLimited(String),
}

/// 論文の重複排除キーを計算する。
///
/// DOI → arXiv ID → S2 paperId → 正規化タイトルの順で優先度を付け、
/// 最初に見つかったものをキーとして返す。
///
/// # Returns
///
/// 重複排除用キー（例: "doi:10.1234/example", "s2:...", "title:..."）
#[must_use]
pub fn dedupe_key(paper: &RemotePaper) -> String {
    // DOI で突合
    if let Some(ref doi) = paper.doi {
        return format!("doi:{}", doi.to_lowercase());
    }

    // arXiv ID で突合（externalIds の ArXiv フィールドを見る必要があるが、
    // RemotePaper には直接フィールドがない。外部 ID 抽出は別途実装）
    // 暫定: タイトルベースで以下に続く

    // S2 paperId で突合
    if paper.id.len() == 40 && paper.id.chars().all(|c| c.is_ascii_hexdigit()) {
        return format!("s2:{}", paper.id);
    }

    // 正規化タイトルで突合
    let normalized_title = crate::citation::matcher::normalize_title(&paper.title);
    format!("title:{}", normalized_title)
}

/// 複数の論文ソースから取得した重複を含む論文リストを、
/// DOI → arXiv ID → S2 paperId → 正規化タイトルの順で突合し、一意の論文リストに変換する。
///
/// 突合に成功した場合、最初に見つかった RemotePaper オブジェクト（ソース順）を保持する。
/// タイトルの正規化には `crate::citation::matcher::normalize_title` を使用する。
#[must_use]
pub fn dedupe(papers: Vec<RemotePaper>) -> Vec<RemotePaper> {
    use std::collections::HashSet;

    let mut seen_keys = HashSet::new();
    let mut result = Vec::new();

    for paper in papers {
        let key = dedupe_key(&paper);
        if !seen_keys.insert(key) {
            continue;
        }

        result.push(paper);
    }

    result
}

/// 複数の DB で検索し、重複を除いた候補と、使った DB、失敗した DB を返す。
///
/// `db` が空なら `cfg.discover.providers` を使う。DB ごとの失敗では止めず、
/// 失敗した DB と理由を [`Vec<(String, String)>`]（DB ID, 理由）で返す
/// （1 つでも成功すればその結果を返す）。`sai-moon-paper search` / `phone search` と
/// 検索セットの再実行（S03）の共通処理。
///
/// # Returns
///
/// `(使った DB, 重複を除いた候補, 失敗した DB と理由)`
pub async fn run_search(
    cfg: &crate::Config,
    query: &str,
    db: &[String],
    limit: u16,
    year_from: Option<u16>,
    year_to: Option<u16>,
    raw_query: Option<&str>,
) -> (Vec<String>, Vec<RemotePaper>, Vec<(String, String)>) {
    // 検索クエリを構築
    let mut search_q = SearchQuery::new(query)
        .with_limit(limit)
        .with_years(year_from, year_to);
    if let Some(rq) = raw_query {
        search_q = search_q.with_raw_query(rq);
    }

    // データベースリストを決定（指定なしなら config から）
    let dbs_to_use = if db.is_empty() {
        cfg.discover.providers.clone()
    } else {
        db.to_vec()
    };

    // 各データベースから検索を実行
    let mut all_papers = Vec::new();
    let mut failed: Vec<(String, String)> = Vec::new();
    for db_id in &dbs_to_use {
        match PaperSourceResolver::resolve(db_id, cfg).await {
            Ok(source) => match source.search(&search_q).await {
                Ok(papers) => all_papers.extend(papers),
                Err(error) => failed.push((db_id.clone(), format!("search error: {error}"))),
            },
            Err(error) => failed.push((db_id.clone(), error)),
        }
    }

    let mut papers = dedupe(all_papers);
    // DB が被引用数を返さないとき（arXiv など）は、OpenAlex でまとめて補う
    citations::fill_citation_counts(cfg, &mut papers).await;
    (dbs_to_use, papers, failed)
}

/// 論文ソースを ID から解決する resolver。
///
/// CLI や TUI から `--db pubmed` のような指定を受け、
/// 対応する `PaperSource` 実装へのパスを返す。
pub struct PaperSourceResolver;

impl PaperSourceResolver {
    /// データベース ID から PaperSource を解決する。
    ///
    /// 設定とクライアント初期化を処理し、トレイトオブジェクトを返す。
    ///
    /// # Errors
    ///
    /// 不明な ID またはクライアント初期化エラーの場合、エラーを返す。
    pub async fn resolve(id: &str, cfg: &crate::Config) -> Result<Box<dyn PaperSource>, String> {
        match id {
            "s2" => {
                let api_key = cfg.s2_api_key();
                let client = s2::S2Client::new(api_key).map_err(|e| e.to_string())?;
                Ok(Box::new(client))
            }
            "arxiv" => {
                let client = arxiv::ArxivClient::new().map_err(|e| e.to_string())?;
                Ok(Box::new(client))
            }
            "pubmed" => {
                let api_key = std::env::var("NCBI_API_KEY").ok();
                let client = pubmed::PubmedClient::new(api_key).map_err(|e| e.to_string())?;
                Ok(Box::new(client))
            }
            "biorxiv" => {
                let client = biorxiv::BiorxivClient::new_biorxiv().map_err(|e| e.to_string())?;
                Ok(Box::new(client))
            }
            "medrxiv" => {
                let client = biorxiv::BiorxivClient::new_medrxiv().map_err(|e| e.to_string())?;
                Ok(Box::new(client))
            }
            "openalex" => {
                let client = openalex::OpenAlexClient::from_config(cfg);
                Ok(Box::new(client))
            }
            other => Err(format!("unknown database: {}", other)),
        }
    }
}

// ============================================================================
// PaperSource トレイト実装
// ============================================================================

/// S2 クライアントの PaperSource 実装。
#[async_trait::async_trait]
impl PaperSource for s2::S2Client {
    fn id(&self) -> &'static str {
        "s2"
    }

    async fn search(&self, q: &SearchQuery) -> Result<Vec<RemotePaper>, PaperSourceError> {
        let query = q.raw_query.as_ref().unwrap_or(&q.text).clone();

        // S2 の year パラメータは "YYYY" または "YYYY-YYYY" 形式を期待
        let year_param = match (q.year_from, q.year_to) {
            (Some(from), Some(to)) => Some(format!("{}-{}", from, to)),
            (Some(from), None) => Some(from.to_string()),
            (None, Some(to)) => Some(format!("0-{}", to)),
            (None, None) => None,
        };

        self.search_with_offset(&query, q.limit, q.offset as u16, year_param.as_deref())
            .await
            .map_err(|e| PaperSourceError::Network(e.to_string()))
    }
}

/// arXiv クライアントの PaperSource 実装。
#[async_trait::async_trait]
impl PaperSource for arxiv::ArxivClient {
    fn id(&self) -> &'static str {
        "arxiv"
    }

    async fn search(&self, q: &SearchQuery) -> Result<Vec<RemotePaper>, PaperSourceError> {
        let query = q.raw_query.as_ref().unwrap_or(&q.text).clone();

        self.search(&query, q.limit)
            .await
            .map_err(|e| PaperSourceError::Network(e.to_string()))
    }
}

/// OpenAlex クライアントの PaperSource 実装。
#[async_trait::async_trait]
impl PaperSource for openalex::OpenAlexClient {
    fn id(&self) -> &'static str {
        "openalex"
    }

    async fn search(&self, q: &SearchQuery) -> Result<Vec<RemotePaper>, PaperSourceError> {
        let query = q.raw_query.as_ref().unwrap_or(&q.text).clone();

        self.search(&query, q.limit)
            .await
            .map_err(|e| PaperSourceError::Network(e.to_string()))
    }
}

/// PubMed クライアントの PaperSource 実装。
#[async_trait::async_trait]
impl PaperSource for pubmed::PubmedClient {
    fn id(&self) -> &'static str {
        "pubmed"
    }

    async fn search(&self, q: &SearchQuery) -> Result<Vec<RemotePaper>, PaperSourceError> {
        if let Some(raw_query) = &q.raw_query {
            // raw_query が指定されている場合、それをそのまま使用
            self.search_raw(raw_query, q.limit)
                .await
                .map_err(|e| PaperSourceError::Network(e.to_string()))
        } else {
            // plain text 検索
            self.search(&q.text, q.limit)
                .await
                .map_err(|e| PaperSourceError::Network(e.to_string()))
        }
    }
}

/// bioRxiv / medRxiv クライアントの PaperSource 実装。
///
/// 検索そのものは Europe PMC の REST 検索（`BiorxivClient::search`）が行う。
#[async_trait::async_trait]
impl PaperSource for biorxiv::BiorxivClient {
    fn id(&self) -> &'static str {
        match self.server() {
            biorxiv::PreprintServer::Biorxiv => "biorxiv",
            biorxiv::PreprintServer::Medrxiv => "medrxiv",
        }
    }

    async fn search(&self, q: &SearchQuery) -> Result<Vec<RemotePaper>, PaperSourceError> {
        let terms = q.raw_query.clone().unwrap_or_else(|| q.text.clone());

        self.search(&terms, q.limit, q.year_from, q.year_to)
            .await
            .map_err(|e| PaperSourceError::Network(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// AC5: `medrxiv` が解決でき、`id()` が `medrxiv` を返す。
    #[tokio::test]
    async fn resolver_resolves_medrxiv() {
        let cfg = crate::Config::default();

        let source = PaperSourceResolver::resolve("medrxiv", &cfg)
            .await
            .expect("medrxiv must resolve");
        assert_eq!(source.id(), "medrxiv");

        let biorxiv = PaperSourceResolver::resolve("biorxiv", &cfg)
            .await
            .expect("biorxiv must resolve");
        assert_eq!(biorxiv.id(), "biorxiv");

        let unknown = PaperSourceResolver::resolve("medrxiv2", &cfg).await;
        assert!(unknown.is_err());
    }
}
