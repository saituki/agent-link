//! PDF の確保（`sai-paper paper get-pdf`・`paper extract`・`paper download` が使う）。
//!
//! 取得できた最初の有効な PDF（`%PDF` マジックナンバー検証）を保存して返す。
//! paywall 回避・出版社サイトのスクレイピングは行わない。

use anyhow::{Context, Result, bail};
use sai_paper_core::{Config, Paths, S2Client, S2PaperDetail};
use std::path::PathBuf;

/// PDF を確保する（DB 登録済み → arXiv → S2 OA → Europe PMC → Unpaywall）。
///
/// 取得できた最初の有効な PDF（`%PDF` マジックナンバー検証）を保存して返す。
/// paywall 回避・出版社サイトのスクレイピングは行わない。
///
/// # Errors
///
/// PDF が見つからない、またはすべての候補でダウンロードに失敗した場合にエラーを返す。
pub async fn ensure_pdf(client: &S2Client, key: &str, db: Option<&sai_paper_core::Database>) -> Result<PathBuf> {
    // 1) DB に登録済みのローカル PDF（あわせて papers 行を取得。論文フォルダの決定に使う）
    let meta = db.and_then(|db| {
        sai_paper_core::citation::db::get_paper_meta_by_extract_key(db.connection(), key)
            .ok()
            .flatten()
    });
    if let Some(pdf) = meta.as_ref().and_then(|meta| meta.pdf_path.clone()) {
        // DB 保存形（相対）を実ファイルパスへ直す（段階 2b）。
        let resolved = db
            .map(|db| db.resolve_path(&pdf))
            .unwrap_or_else(|| PathBuf::from(&pdf));
        if resolved.exists() {
            return Ok(resolved);
        }
    }

    // 2) メタデータから合法的な OA 候補 URL を組み立てる
    let detail = client
        .get_paper(key)
        .await
        .with_context(|| format!("S2 paper lookup failed: {key}"))?;
    let candidates = oa_pdf_candidates_from_detail(&detail, key).await?;

    let paths = Paths::discover()?;
    let data_dir = paths.data_dir();
    let title = detail.title.clone().unwrap_or_else(|| key.to_string());
    // 置き場（段階 2a）: `papers.id` が解決できる論文は論文フォルダに置く。
    // DB 未登録の論文だけ従来どおり `papers/` 直下に置く（番号が無いため）。
    let dest = if let Some(meta) = meta.as_ref() {
        let folder_title = if meta.title.trim().is_empty() {
            title.as_str()
        } else {
            meta.title.as_str()
        };
        let folder = sai_paper_core::paths::paper_folder(&data_dir, meta.db_id, folder_title);
        // 書き込む前に論文フォルダを作る。
        std::fs::create_dir_all(&folder)?;
        sai_paper_core::paths::paper_file_path(&folder, "pdf")
    } else {
        std::fs::create_dir_all(&paths.downloads_dir)?;
        paths.downloads_dir.join(format!(
            "{}.pdf",
            sai_paper_core::paths::sanitize_download_filename_component(&title)
        ))
    };
    if dest.exists() {
        return Ok(dest);
    }

    // 3) 候補を順に試し、最初に取得できた有効な PDF を採用する
    let mut errors = Vec::new();
    for (source, url) in candidates {
        match reqwest::get(&url).await {
            Ok(response) => match response.bytes().await {
                Ok(bytes) if bytes.starts_with(b"%PDF") => {
                    std::fs::write(&dest, &bytes)?;
                    return Ok(dest);
                }
                Ok(_) => errors.push(format!("{source}: not a PDF")),
                Err(error) => errors.push(format!("{source}: {error}")),
            },
            Err(error) => errors.push(format!("{source}: {error}")),
        }
    }
    bail!(
        "no downloadable open access PDF for {key}: {}",
        errors.join("; ")
    )
}

/// 論文 1 件の合法的な OA の PDF 候補 URL を（出どころ, URL）の並びで返す。
///
/// 並びは S2 の `openAccessPdf` → arXiv → Europe PMC → Unpaywall。
/// 候補が無いときはエラーにする。
///
/// # Errors
///
/// S2 の論文情報が取れない、または OA の候補が無い場合にエラーを返す。
pub async fn oa_pdf_candidates(client: &S2Client, key: &str) -> Result<Vec<(String, String)>> {
    let detail = client
        .get_paper(key)
        .await
        .with_context(|| format!("S2 paper lookup failed: {key}"))?;
    oa_pdf_candidates_from_detail(&detail, key).await
}

/// すでに取った S2 の論文情報から OA の PDF 候補 URL を作る（`ensure_pdf` と共用）。
///
/// # Errors
///
/// OA の候補が 1 件も無い場合にエラーを返す。
async fn oa_pdf_candidates_from_detail(
    detail: &S2PaperDetail,
    key: &str,
) -> Result<Vec<(String, String)>> {
    let external = detail.external_ids.clone().unwrap_or_default();
    let mut candidates: Vec<(String, String)> = Vec::new();
    if let Some(url) = detail
        .open_access_pdf
        .as_ref()
        .and_then(|p| p.url.clone())
        .filter(|u| !u.trim().is_empty())
    {
        candidates.push(("s2-oa".into(), url));
    }
    if let Some(arxiv) = external.get("ArXiv").filter(|s| !s.trim().is_empty()) {
        candidates.push(("arxiv".into(), format!("https://arxiv.org/pdf/{arxiv}.pdf")));
    }
    if let Some(pmcid) = external
        .get("PubMedCentral")
        .filter(|s| s.starts_with("PMC"))
    {
        candidates.push((
            "europepmc".into(),
            format!("https://europepmc.org/articles/{pmcid}?pdf=render"),
        ));
    }
    if let Some(doi) = external.get("DOI").filter(|s| !s.trim().is_empty()) {
        let email = unpaywall_email();
        if !email.is_empty()
            && let Ok(response) =
                reqwest::get(format!("https://api.unpaywall.org/v2/{doi}?email={email}")).await
            && let Ok(value) = response.json::<serde_json::Value>().await
        {
            let location = &value["best_oa_location"];
            if let Some(url) = location["url_for_pdf"]
                .as_str()
                .filter(|u| !u.is_empty())
                .or_else(|| location["url"].as_str().filter(|u| !u.is_empty()))
            {
                candidates.push(("unpaywall".into(), url.to_string()));
            }
        }
    }
    if candidates.is_empty() {
        bail!("no open access source for {key}");
    }
    Ok(candidates)
}

/// Unpaywall に渡すメールアドレスを設定から返す。
///
/// 設定が無い・空のときは**空文字**を返す（＝Unpaywall を使わない）。
/// 呼び出し側は空文字のとき Unpaywall の API を呼ばない。
fn unpaywall_email() -> String {
    if let Ok(paths) = Paths::discover()
        && let Ok(config) = Config::load_or_create(&paths)
        && !config.discover.unpaywall_email.trim().is_empty()
    {
        return config.discover.unpaywall_email.trim().to_string();
    }
    String::new()
}
