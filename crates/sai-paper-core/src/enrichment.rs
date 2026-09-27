//! Provider-neutral paper metadata enrichment workflow.

use std::path::{Path, PathBuf};

use crate::{
    RemotePaper,
    api::{arxiv::ArxivClient, crossref::CrossrefClient, openalex::OpenAlexClient},
};

/// Input hints available for one metadata enrichment request.
#[derive(Debug, Clone)]
pub struct MetadataCandidate {
    /// Candidate arXiv identifier.
    pub arxiv_id: Option<String>,
    /// Candidate DOI.
    pub doi: Option<String>,
    /// Local PDF used to discover missing identifiers.
    pub pdf_path: Option<PathBuf>,
}

/// Provider result produced without any frontend-specific presentation policy.
#[derive(Debug, Clone)]
pub enum MetadataEnrichmentOutcome {
    /// Complete remote paper metadata.
    Paper(RemotePaper),
    /// Journal metadata only.
    Journal(String),
    /// No provider could resolve the candidate.
    Unavailable,
    /// Provider lookup failed.
    Failed(String),
}

/// Applies the shared Crossref/OpenAlex/arXiv fallback policy.
#[derive(Clone)]
pub struct MetadataEnrichmentService {
    arxiv: ArxivClient,
    crossref: CrossrefClient,
    openalex: OpenAlexClient,
}

impl MetadataEnrichmentService {
    /// Construct the service around the shared rate-limited arXiv client.
    #[must_use]
    pub fn new(arxiv: ArxivClient) -> Self {
        Self {
            arxiv,
            crossref: CrossrefClient::new(),
            openalex: OpenAlexClient::new(),
        }
    }

    /// Enrich one candidate using `PDF`, `Crossref`, `OpenAlex`, and arXiv fallbacks.
    ///
    /// 撤回の状態も必要な呼び側は [`Self::enrich_with_status`] を使う。
    pub async fn enrich(&self, candidate: MetadataCandidate) -> MetadataEnrichmentOutcome {
        self.enrich_with_status(candidate).await.0
    }

    /// `enrich` と同じだが、OpenAlex で分かった撤回の状態も返す。
    ///
    /// 戻り値の `Option<bool>` は `None` = DOI が無い・OpenAlex に問い合わせていない・
    /// 応答に欄が無いのいずれか、`Some(true)` = 撤回、`Some(false)` = 撤回されていない。
    /// **DB には触らない**（書くのは呼び側の役）。OpenAlex への問い合わせは 1 回だけ。
    pub async fn enrich_with_status(
        &self,
        mut candidate: MetadataCandidate,
    ) -> (MetadataEnrichmentOutcome, Option<bool>) {
        if let Some(path) = candidate.pdf_path.clone() {
            enrich_identifiers_from_pdf(&path, &mut candidate).await;
        }

        let mut retraction = None;
        if let Some(doi) = candidate.doi.as_deref() {
            // 補完のついでに OpenAlex で撤回の状態も見る（DOI があるときだけ）。
            // ネットにつながらないときは `Err` になり、状態は分からないままにする。
            let status = self.openalex.work_status_by_doi(doi).await.ok().flatten();
            retraction = status.as_ref().and_then(|status| status.is_retracted);
            match self.crossref.get_by_doi(doi).await {
                Ok(Some(mut paper)) => {
                    if paper.journal_ref.is_none()
                        && let Some(journal) = status.as_ref().and_then(|s| s.journal.clone())
                    {
                        paper.journal_ref = Some(journal);
                    }
                    return (MetadataEnrichmentOutcome::Paper(paper), retraction);
                }
                Ok(None) => {
                    if let Some(journal) = status.and_then(|s| s.journal) {
                        return (MetadataEnrichmentOutcome::Journal(journal), retraction);
                    }
                }
                Err(error) => {
                    if let Some(journal) = status.and_then(|s| s.journal) {
                        return (MetadataEnrichmentOutcome::Journal(journal), retraction);
                    }
                    return (
                        MetadataEnrichmentOutcome::Failed(format!(
                            "Crossref enrichment failed for {doi}: {error}"
                        )),
                        retraction,
                    );
                }
            }
        }

        let Some(arxiv_id) = candidate.arxiv_id else {
            return (MetadataEnrichmentOutcome::Unavailable, retraction);
        };
        let outcome = match self.arxiv.get_by_id(&arxiv_id).await {
            Ok(Some(paper)) => MetadataEnrichmentOutcome::Paper(paper),
            Ok(None) => MetadataEnrichmentOutcome::Unavailable,
            Err(error) => MetadataEnrichmentOutcome::Failed(format!(
                "arXiv enrichment failed for {arxiv_id}: {error}"
            )),
        };
        (outcome, retraction)
    }
}

async fn enrich_identifiers_from_pdf(path: &Path, candidate: &mut MetadataCandidate) {
    let Ok(output) = tokio::process::Command::new("pdftotext")
        .args(["-l", "2", &path.to_string_lossy(), "-"])
        .output()
        .await
    else {
        return;
    };
    if !output.status.success() {
        return;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let lower_text = text.to_lowercase();
    if candidate.arxiv_id.is_none()
        && let Some(index) = lower_text.find("arxiv:")
    {
        let value = &text[index + 6..];
        let end = value
            .find(|character: char| !character.is_ascii_digit() && character != '.')
            .unwrap_or(value.len());
        let id = &value[..end];
        if id.len() >= 7 {
            candidate.arxiv_id = Some(id.to_owned());
        }
    }
    if candidate.doi.is_none()
        && let Some(index) = lower_text.find("10.")
    {
        let value = &text[index..];
        let end = value
            .find(|character: char| character.is_whitespace() || character == '\n')
            .unwrap_or(value.len());
        let id = value[..end].trim_end_matches(['.', ',', ';', ')']);
        if id.len() >= 5 {
            candidate.doi = Some(id.to_owned());
        }
    }
}
