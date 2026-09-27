//! Crossref API client for fetching paper metadata by DOI.

use crate::models::RemotePaper;
use chrono::{NaiveDate, TimeZone, Utc};
use reqwest::{Client, Url};
use serde::Deserialize;

const CROSSREF_ENDPOINT: &str = "https://api.crossref.org/works/";

#[derive(Deserialize)]
struct CrossrefResponse {
    message: CrossrefWork,
}

#[derive(Deserialize)]
struct CrossrefWork {
    #[serde(rename = "DOI")]
    doi: Option<String>,
    title: Option<Vec<String>>,
    author: Option<Vec<CrossrefAuthor>>,
    #[serde(rename = "abstract")]
    abstract_text: Option<String>,
    #[serde(rename = "container-title")]
    container_title: Option<Vec<String>>,
    #[serde(rename = "short-container-title")]
    short_container_title: Option<Vec<String>>,
    published: Option<CrossrefDate>,
    created: Option<CrossrefDate>,
    deposited: Option<CrossrefDate>,
}

#[derive(Deserialize)]
struct CrossrefAuthor {
    given: Option<String>,
    family: Option<String>,
}

#[derive(Deserialize)]
struct CrossrefDate {
    #[serde(rename = "date-parts")]
    date_parts: Option<Vec<Vec<u32>>>,
}

/// Crossref の `abstract` に混入する JATS XML を除去し、表示用のプレーンテキストに整形する。
///
/// 実体は [`crate::abstracts::clean_crossref_abstract`]。ここに再掲していた旧実装は
/// `<jats:p>` / `<jats:sec>` のみ除去していたため、`<jats:title>` / `<jats:list>` 等が
/// PAPER 画面にそのまま表示されていた。新規取得時はこの関数経由で整形する。
#[must_use]
pub fn clean_crossref_abstract(raw: &str) -> String {
    crate::abstracts::clean_crossref_abstract(raw)
}

/// Represents an error returned by the Crossref API.
#[derive(Debug, thiserror::Error)]
pub enum CrossrefError {
    /// A network or HTTP request error occurred.
    #[error("HTTP request failed: {0}")]
    Request(#[from] reqwest::Error),
    /// Failed to parse the JSON response from Crossref.
    #[error("Failed to parse response: {0}")]
    Parse(#[from] serde_json::Error),
    /// The requested DOI was not found in Crossref.
    #[error("Work not found")]
    NotFound,
}

/// A client for fetching scholarly metadata from the Crossref API.
#[derive(Clone)]
pub struct CrossrefClient {
    client: Client,
}

impl CrossrefClient {
    /// Creates a new `CrossrefClient` with default configuration.
    #[must_use]
    pub fn new() -> Self {
        Self {
            client: Client::builder()
                .user_agent("papr/0.1.0")
                .build()
                .unwrap_or_default(),
        }
    }

    /// Fetch metadata for a specific DOI.
    ///
    /// # Errors
    /// Returns an error when the request fails or JSON content is malformed.
    pub async fn get_by_doi(&self, doi: &str) -> Result<Option<RemotePaper>, CrossrefError> {
        let url = Url::parse(CROSSREF_ENDPOINT)
            .map_err(|_| CrossrefError::NotFound)?
            .join(doi)
            .map_err(|_| CrossrefError::NotFound)?;
        let response = self.client.get(url).send().await?;
        if response.status() == 404 {
            return Ok(None);
        }
        let response = response.error_for_status()?;

        let body = response.bytes().await?;
        let res: CrossrefResponse = serde_json::from_slice(&body)?;
        let work = res.message;

        let title = work
            .title
            .and_then(|mut t| t.pop())
            .unwrap_or_else(|| "Unknown Title".to_string());

        let authors = work
            .author
            .unwrap_or_default()
            .into_iter()
            .filter_map(|a| match (a.given, a.family) {
                (Some(g), Some(f)) => Some(format!("{g} {f}")),
                (None, Some(f)) => Some(f),
                (Some(g), None) => Some(g),
                _ => None,
            })
            .collect();

        // Abstract from crossref often has JATS XML tags like <jats:p>. We should clean them.
        let raw_abs = work.abstract_text.unwrap_or_default();
        let abstract_text = clean_crossref_abstract(&raw_abs);

        let mut date = Utc::now();
        if let Some(cd) = work.published.or(work.created).or(work.deposited)
            && let Some(parts) = cd.date_parts
            && let Some(p) = parts.first()
        {
            let year = p.first().copied().unwrap_or(0);
            let month = p.get(1).copied().unwrap_or(1);
            let day = p.get(2).copied().unwrap_or(1);
            if let Ok(year) = i32::try_from(year)
                && let Some(nd) = NaiveDate::from_ymd_opt(year, month, day)
                && let Some(ndt) = nd.and_hms_opt(0, 0, 0)
            {
                date = Utc.from_utc_datetime(&ndt);
            }
        }

        Ok(Some(RemotePaper {
            id: work.doi.clone().unwrap_or_else(|| doi.to_string()),
            title,
            authors,
            abstract_text,
            published: date,
            updated: date,
            categories: vec![],
            pdf_url: None,
            doi: work.doi,
            journal_ref: work
                .container_title
                .or(work.short_container_title)
                .and_then(|mut titles| titles.pop()),
            citation_count: None,
        }))
    }
}

impl Default for CrossrefClient {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::clean_crossref_abstract;

    #[test]
    fn jatsタグと分類情報を除去する() {
        let raw = "<jats:title>ABSTRACT</jats:title>                  The widespread adoption.\
                  <jats:p>This article is categorized under:\
                    <jats:list list-type=\"simple\">                      <jats:list-item>                        Algorithmic\
   Development &gt; Text Mining                      </jats:list-item></jats:list></jats:p>";
        assert_eq!(clean_crossref_abstract(raw), "The widespread adoption.");
    }

    #[test]
    fn 段落境界は保持し段落内の改行は畳む() {
        let raw = "<jats:p>一段落目の\n継続行。</jats:p><jats:p>二段落目。</jats:p>";
        assert_eq!(
            clean_crossref_abstract(raw),
            "一段落目の 継続行。\n\n二段落目。"
        );
    }

    #[test]
    fn 実物のwidm要約からタグと分類情報を除去する() {
        let raw = "<jats:title>ABSTRACT</jats:title>\n                  <jats:p>The widespread adoption of the Internet.</jats:p>\n                  <jats:p>\n                    This article is categorized under:\n                    <jats:list list-type=\"simple\">\n                      <jats:list-item>\n                        <jats:p>Algorithmic Development &gt; Text Mining</jats:p>\n                      </jats:list-item>\n                    </jats:list>\n                  </jats:p>";
        assert_eq!(
            clean_crossref_abstract(raw),
            "The widespread adoption of the Internet."
        );
    }
}
