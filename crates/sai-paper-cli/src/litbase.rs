//! `sai-paper litbase` — litbase 連携サブコマンド。
//!
//! pi-litbase 拡張が `pi.exec` で呼び出す決定論的 API。すべて `--json` を
//! 受け付け、標準出力に単一の JSON を返す。LLM 推論は行わない。
//! 表を使わない道具（論文の特定・書誌・PDF・本文）だけを残す。

use anyhow::{Context, Result, bail};
use clap::Subcommand;
use rusqlite::OptionalExtension;
use sai_paper_core::litbase;
use sai_paper_core::{Config, Database, Paths, S2Client, S2PaperDetail};
use std::path::{Path, PathBuf};

/// litbase サブコマンド。
#[derive(Subcommand, Debug)]
pub enum LitbaseCmd {
    /// Library / DOI / タイトルから論文を解決する
    Resolve {
        /// 検索語（paper_id / DOI / arXiv ID / タイトル）
        query: String,
    },
    /// Library の論文を最近順に一覧する
    Recent {
        /// 取得件数
        #[arg(long, default_value_t = 20)]
        limit: usize,
    },
    /// S2 からメタデータを取得する（Paper Weight 用）
    Lookup {
        /// DOI
        #[arg(long)]
        doi: Option<String>,
        /// タイトル
        #[arg(long)]
        title: Option<String>,
    },
    /// PDF を確保する（DB → arXiv → S2 OA → Unpaywall）
    EnsurePdf {
        /// 検索語（paper_id / DOI / arXiv ID / タイトル）
        query: String,
    },
    /// 手動取得した PDF を取り込む
    ImportPdf {
        /// 検索語（paper_id / DOI / arXiv ID / タイトル）
        query: String,
        /// PDF パス
        #[arg(long)]
        pdf: PathBuf,
    },
    /// docling で抽出した本文 Markdown を返す
    Text {
        /// 検索語（paper_id / DOI / arXiv ID / タイトル）
        query: String,
    },
    /// 直近のダウンロード PDF 候補を列挙する
    RecentDownloads {
        /// 取得件数
        #[arg(long, default_value_t = 20)]
        limit: usize,
    },
}

/// litbase サブコマンドを実行する。
///
/// # Errors
///
/// 引数不正・DB 操作失敗・ネットワーク失敗時にエラーを返す。
pub async fn handle(cmd: LitbaseCmd, client: &S2Client) -> Result<()> {
    match cmd {
        LitbaseCmd::Resolve { query } => handle_resolve(&query),
        LitbaseCmd::Recent { limit } => handle_recent(limit),
        LitbaseCmd::Lookup { doi, title } => handle_lookup(client, doi, title).await,
        LitbaseCmd::EnsurePdf { query } => handle_ensure_pdf(client, &query).await,
        LitbaseCmd::Text { query } => handle_text(client, &query).await,
        LitbaseCmd::ImportPdf { query, pdf } => handle_import_pdf(&query, &pdf),
        LitbaseCmd::RecentDownloads { limit } => handle_recent_downloads(limit),
    }
}

fn open_db() -> Result<Database> {
    let paths = Paths::discover().context("failed to resolve paths")?;
    Database::open(&paths.database_file).context("failed to open papr.db")
}

fn print_json(value: &serde_json::Value) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

/// DB 保存形（data_dir からの相対）のパスを、外に出す JSON 用の絶対パスへ直す（段階 2b）。
///
/// pi-litbase は絶対パスを前提にしているため、JSON には相対パスを出さない。
fn abs_stored(db: &Database, stored: &str) -> String {
    db.resolve_path(stored).to_string_lossy().into_owned()
}

fn handle_resolve(query: &str) -> Result<()> {
    let db = open_db()?;
    match litbase::resolve_paper(db.connection(), query)? {
        Some(mut paper) => {
            paper.pdf_path = paper.pdf_path.map(|stored| abs_stored(&db, &stored));
            print_json(&serde_json::json!({ "status": "found", "paper": paper }))
        }
        None => print_json(&serde_json::json!({ "status": "not_found", "query": query })),
    }
}

fn handle_recent(limit: usize) -> Result<()> {
    let db = open_db()?;
    let papers = litbase::recent_papers(db.connection(), limit)?
        .into_iter()
        .map(|mut paper| {
            paper.pdf_path = paper.pdf_path.map(|stored| abs_stored(&db, &stored));
            paper
        })
        .collect::<Vec<_>>();
    print_json(&serde_json::json!({ "papers": papers }))
}

async fn handle_lookup(
    client: &S2Client,
    doi: Option<String>,
    title: Option<String>,
) -> Result<()> {
    if let Some(doi) = doi {
        let key = format!("DOI:{}", doi.trim());
        let detail = client
            .get_paper(&key)
            .await
            .with_context(|| format!("S2 lookup failed: {key}"))?;
        return print_json(&serde_json::to_value(detail)?);
    }
    if let Some(title) = title {
        let results = client.search(&title, 1).await?;
        return print_json(&serde_json::json!({ "results": results }));
    }
    bail!("--doi または --title が必要です");
}

/// 論文の抽出キーと DOI URL を解決する。
fn resolve_key(db: &Database, query: &str) -> (String, Option<String>) {
    if let Ok(Some(paper)) = litbase::resolve_paper(db.connection(), query) {
        let doi_url = paper
            .doi
            .as_ref()
            .filter(|d| !d.trim().is_empty())
            .map(|d| format!("https://doi.org/{d}"));
        if let Some(s2) = paper.s2_paper_id.filter(|s| !s.trim().is_empty()) {
            return (s2, doi_url);
        }
        if let Some(arxiv) = paper.arxiv_id.filter(|s| !s.trim().is_empty()) {
            return (format!("ArXiv:{arxiv}"), doi_url);
        }
        if let Some(doi) = paper.doi.filter(|s| !s.trim().is_empty()) {
            return (format!("DOI:{doi}"), doi_url);
        }
    }
    let key = sai_paper_core::citation::normalize_paper_id(query);
    let doi_url = key
        .strip_prefix("DOI:")
        .map(|doi| format!("https://doi.org/{doi}"));
    (key, doi_url)
}

async fn handle_ensure_pdf(client: &S2Client, query: &str) -> Result<()> {
    let db = open_db()?;
    let (key, doi_url) = resolve_key(&db, query);
    match ensure_pdf(client, &key, Some(&db)).await {
        Ok(path) => print_json(&serde_json::json!({
            "status": "ready",
            "paper_key": key,
            "pdf_path": path,
        })),
        Err(error) => print_json(&serde_json::json!({
            "status": "manual_required",
            "paper_key": key,
            "doi_url": doi_url,
            "reason": error.to_string(),
        })),
    }
}

async fn handle_text(client: &S2Client, query: &str) -> Result<()> {
    let db = open_db()?;
    let (key, _) = resolve_key(&db, query);
    let pdf = ensure_pdf(client, &key, Some(&db)).await?;
    let data_dir = Paths::discover()?.data_dir();
    let output =
        sai_paper_core::citation::ensure_extract(db.connection(), client, &key, &pdf, &data_dir)
            .await?;
    // 記録はまず papers_id で、無ければ抽出キーで探す（段階 2c）。
    let mut md_path: Option<String> = None;
    if let Some(id) = Database::papers_id_for_extract_key(db.connection(), &key) {
        md_path = db
            .connection()
            .query_row(
                "SELECT md_path FROM paper_extracts WHERE papers_id = ?1",
                [id],
                |row| row.get(0),
            )
            .optional()?;
    }
    if md_path.is_none() {
        md_path = db
            .connection()
            .query_row(
                "SELECT md_path FROM paper_extracts WHERE paper_id = ?1",
                [&key],
                |row| row.get(0),
            )
            .optional()?;
    }
    // md_path は DB では data_dir からの相対（段階 2b）なので実パスへ直して読む。
    let markdown = match md_path
        .map(|stored| sai_paper_core::paths::resolve_stored_path(&data_dir, &stored))
        .filter(|path| path.is_file())
    {
        Some(path) => std::fs::read_to_string(path)?,
        None => String::new(),
    };
    print_json(&serde_json::json!({
        "status": "ready",
        "paper_key": key,
        "refs": output.refs.len(),
        "sentences": output.sentences.len(),
        "markdown": markdown,
    }))
}

fn handle_import_pdf(query: &str, pdf: &Path) -> Result<()> {
    if !pdf.exists() {
        bail!("PDF が見つかりません: {}", pdf.display());
    }
    let bytes = std::fs::read(pdf)?;
    if !bytes.starts_with(b"%PDF") {
        bail!("PDF マジックナンバーがありません: {}", pdf.display());
    }
    let db = open_db()?;
    let paper = litbase::resolve_paper(db.connection(), query)?
        .with_context(|| format!("論文を解決できません: {query}"))?;
    let paths = Paths::discover()?;
    let data_dir = paths.data_dir();
    // 置き場（段階 2a）: 論文フォルダ。既に `paper.pdf` があるときだけ従来どおり
    // 衝突しない名前（`papers/` 直下）にして、既存の PDF を上書きしない。
    let folder = sai_paper_core::paths::paper_folder(&data_dir, paper.id, &paper.title);
    let folder_pdf = sai_paper_core::paths::paper_file_path(&folder, "pdf");
    let destination = if folder_pdf.exists() {
        let stem = sai_paper_core::paths::sanitize_download_filename_component(&paper.title);
        let stem = if stem.is_empty() {
            paper.id.to_string()
        } else {
            stem
        };
        std::fs::create_dir_all(&paths.downloads_dir)?;
        let mut destination = paths.downloads_dir.join(format!("{stem}.pdf"));
        let mut suffix = 2;
        while destination.exists() {
            destination = paths.downloads_dir.join(format!("{stem} ({suffix}).pdf"));
            suffix += 1;
        }
        destination
    } else {
        // 書き込む前に論文フォルダを作る。
        std::fs::create_dir_all(&folder)?;
        folder_pdf
    };
    std::fs::copy(pdf, &destination)?;
    // DB には data_dir からの相対で書く（段階 2b）。JSON には絶対パスのまま出す。
    db.connection().execute(
        "UPDATE papers SET pdf_path = ?1 WHERE id = ?2",
        rusqlite::params![db.store_path(&destination), paper.id],
    )?;
    print_json(&serde_json::json!({
        "status": "imported",
        "paper_id": paper.id,
        "pdf_path": destination,
    }))
}

fn handle_recent_downloads(limit: usize) -> Result<()> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let Some(home) = home else {
        bail!("HOME が解決できません");
    };
    let mut dirs = vec![home.join("Downloads"), home.join("ダウンロード")];
    if let Ok(xdg) = std::env::var("XDG_DOWNLOAD_DIR") {
        let raw = xdg.replace("$HOME", &home.to_string_lossy());
        dirs.insert(0, PathBuf::from(raw));
    }
    let dir = dirs
        .into_iter()
        .find(|d| d.is_dir())
        .context("ダウンロードディレクトリが見つかりません")?;
    let mut candidates: Vec<serde_json::Value> = Vec::new();
    for entry in std::fs::read_dir(&dir)? {
        let entry = entry?;
        let path = entry.path();
        if path
            .extension()
            .and_then(|e| e.to_str())
            .map(str::to_ascii_lowercase)
            != Some("pdf".to_string())
        {
            continue;
        }
        let metadata = entry.metadata()?;
        let modified = metadata
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0);
        candidates.push(serde_json::json!({
            "path": path,
            "modified_unix": modified,
        }));
    }
    candidates.sort_by(|a, b| {
        b["modified_unix"]
            .as_u64()
            .cmp(&a["modified_unix"].as_u64())
    });
    candidates.truncate(limit);
    print_json(&serde_json::json!({ "dir": dir, "candidates": candidates }))
}

/// PDF を確保する（DB 登録済み → arXiv → S2 OA → Europe PMC → Unpaywall）。
///
/// 取得できた最初の有効な PDF（`%PDF` マジックナンバー検証）を保存して返す。
/// paywall 回避・出版社サイトのスクレイピングは行わない。
///
/// # Errors
///
/// PDF が見つからない、またはすべての候補でダウンロードに失敗した場合にエラーを返す。
pub async fn ensure_pdf(client: &S2Client, key: &str, db: Option<&Database>) -> Result<PathBuf> {
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
