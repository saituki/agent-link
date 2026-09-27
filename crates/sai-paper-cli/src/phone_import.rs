//! `sai-paper phone import` / `phone download` / `phone after-download` /
//! `phone attention-resolve` — sai-phone（Web）の Import・Download 画面用（U2）。
//!
//! sai-phone は sai-paper の DB を直接開かず、このコマンドの JSON だけを読む。
//! 共通ルール（`crates/sai-paper-cli/src/phone.rs` と同じ）:
//!
//! - 成功: 終了コード 0、標準出力に JSON 1 行
//! - 失敗: 終了コード 1、標準出力に `{"error": "<日本語の文>"}`
//!
//! BibTeX の置き場は TUI の Import 画面と同じ `<data_dir>/bibtex/`（読み込みは
//! `sai_paper_core::bibtex::load_bibtex_dir_detailed`）なので、Web で足した .bib は
//! TUI にもそのまま出る。PDF の登録は TUI と同じ
//! `sai_paper_core::downloads::register_downloaded_pdf` を使う（Download が 2 か所に
//! 増えないように）。検索セットへの紐付け・プラグイン通知・図抽出のパイプラインなど
//! 画面固有の処理は行わない（`phone after-download` が別コマンドで担当する）。

use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result, bail};
use clap::Subcommand;
use sai_paper_core::bibtex::{self, list_bib_files, load_bibtex_dir_detailed};
use sai_paper_core::downloads::{DownloadEvent, DownloadManager, download_destination};
use sai_paper_core::{
    Config, Database, Paths, S2Client, attention, downloads, library_tree, paths, pipeline,
    search_log, translation,
};
use serde_json::{Value, json};
use tokio::sync::mpsc;

use crate::open_database;
use crate::phone::{library_papers, open_context, remote_year};

/// `phone import add` で受け付ける .bib の最大バイト（1MB）。
const MAX_BIB_BYTES: usize = 1024 * 1024;

/// `sai-paper phone import` のサブコマンド。
#[derive(Subcommand, Debug)]
pub enum PhoneImportCmd {
    /// BibTeX フォルダの .bib 一覧と、取り込める論文の一覧を出す
    List,
    /// 標準入力の BibTeX（1MB まで）を .bib として保存する
    Add {
        /// 保存するファイル名（例: `refs.bib`）
        #[arg(long)]
        name: String,
    },
    /// .bib を 1 件消す
    Remove {
        /// 消すファイル名（例: `refs.bib`）
        #[arg(long)]
        name: String,
    },
}

/// `sai-paper phone import` を実行して JSON 1 行を返す。
pub fn run_import(cmd: PhoneImportCmd) -> Result<Value> {
    match cmd {
        PhoneImportCmd::List => import_list(),
        PhoneImportCmd::Add { name } => import_add(&name),
        PhoneImportCmd::Remove { name } => import_remove(&name),
    }
}

/// BibTeX フォルダ（`<data_dir>/bibtex`。TUI の Import 画面と同じ置き場）。
fn bibtex_dir(paths: &Paths) -> PathBuf {
    paths.data_dir().join("bibtex")
}

/// `phone import list`: .bib ごとの件数と、取り込める論文の一覧。
///
/// フォルダが無いときは空の `files` / `entries` / `errors` で成功する。
fn import_list() -> Result<Value> {
    let (db, cfg, paths) = open_context()?;
    let dir = bibtex_dir(&paths);
    let files = list_bib_files(&dir)?;
    let scan = load_bibtex_dir_detailed(&dir)?;
    let library = library_papers(&db, &cfg, &paths)?;

    let file_items: Vec<Value> = files
        .iter()
        .map(|file| json!({ "name": file.name, "count": file.count, "error": file.error }))
        .collect();
    // 並びは load_bibtex_dir_detailed のまま（ファイル名順・重複はまとめ済み）。
    let entries: Vec<Value> = scan
        .papers
        .iter()
        .map(|remote| {
            let local_paper_id = library_tree::match_downloaded_paper(&library, remote)
                .map(|index| library[index].id);
            json!({
                "id": remote.id,
                "title": remote.title,
                "authors": remote.authors,
                "year": remote_year(&remote.published),
                "journal": remote.journal_ref,
                "doi": remote.doi,
                "abstract": remote.abstract_text,
                "citation_count": remote.citation_count,
                "local_paper_id": local_paper_id,
            })
        })
        .collect();

    Ok(json!({
        "dir": dir.to_string_lossy(),
        "files": file_items,
        "entries": entries,
        "errors": scan.errors,
    }))
}

/// `phone import add --name <file.bib>`: 標準入力の BibTeX を保存する。
///
/// 壊れた BibTeX・0 件・同名は保存せず、その旨の日本語エラーで落とす
/// （既存の .bib を上書きしない）。
fn import_add(name: &str) -> Result<Value> {
    let paths = Paths::discover()?;
    let content = read_stdin_limited()?;
    let saved = bibtex::save_bib_file(&bibtex_dir(&paths), name, &content)
        .map_err(|error| anyhow::anyhow!("{error}"))?;
    Ok(json!({
        "name": name,
        "count": saved.count,
        "path": saved.path.to_string_lossy(),
    }))
}

/// `phone import remove --name <file.bib>`: .bib を 1 件消す。
fn import_remove(name: &str) -> Result<Value> {
    let paths = Paths::discover()?;
    bibtex::remove_bib_file(&bibtex_dir(&paths), name)
        .map_err(|error| anyhow::anyhow!("{error}"))?;
    Ok(json!({ "name": name, "removed": true }))
}

/// 標準入力を [`MAX_BIB_BYTES`] まで読んで文字にする（超えたら日本語エラー）。
fn read_stdin_limited() -> Result<String> {
    let mut buffer = Vec::new();
    std::io::stdin()
        .lock()
        .take(u64::try_from(MAX_BIB_BYTES).unwrap_or(u64::MAX) + 1)
        .read_to_end(&mut buffer)?;
    if buffer.len() > MAX_BIB_BYTES {
        bail!("1MB までにしてください");
    }
    String::from_utf8(buffer).context("BibTeX を UTF-8 として読めませんでした")
}

/// `phone download`: 検索履歴の 1 件、または BibTeX の 1 件の PDF を取りにいき、
/// Library に登録する。
///
/// `--log <log_id> --position <n>` と `--import-id <RemotePaper.id>` のどちらかを
/// 1 つだけ指定する。既に Library にあるときは取りに行かず `status: "exists"`。
pub async fn download(
    client: &S2Client,
    log_id: Option<i64>,
    position: Option<i64>,
    import_id: Option<&str>,
) -> Result<Value> {
    let (mut db, cfg, paths) = open_context()?;
    let data_dir = paths.data_dir();
    let paper = find_target_paper(&db, &paths, log_id, position, import_id)?;

    // 既に Library に PDF があれば取りに行かない。
    let library = library_papers(&db, &cfg, &paths)?;
    if let Some(index) = library_tree::match_downloaded_paper(&library, &paper) {
        let local = &library[index];
        return Ok(json!({
            "paper_id": local.id,
            "title": local.title,
            "status": "exists",
            "pdf_path": local.pdf_path,
            "source": null,
            "abstract_ja": false,
        }));
    }

    // 取りに行く URL の候補（pdf_url → OA の候補）。
    let candidates = pdf_candidates(client, &paper).await?;
    if candidates.is_empty() {
        bail!("PDF の場所が見つかりません。Mac の sai-paper-tui で PDF を取り込んでください");
    }

    // 置き場は TUI と同じ（設定の library.download_path、無ければ .sai/paper/papers）。
    let download_dir = cfg
        .library
        .download_path
        .clone()
        .unwrap_or_else(|| paths.downloads_dir.clone());
    std::fs::create_dir_all(&download_dir)?;
    let destination = download_destination(&download_dir, &paper);

    let manager = DownloadManager::new().context("downloader を初期化できませんでした")?;
    let mut errors: Vec<String> = Vec::new();
    let mut got: Option<(String, PathBuf)> = None;
    for (source, url) in candidates {
        match download_once(&manager, &paper.id, &url, &destination).await {
            Ok(path) => {
                if looks_like_pdf(&path) {
                    got = Some((source, path));
                    break;
                }
                let _ = std::fs::remove_file(&path);
                errors.push(format!("{source}: PDF ではない"));
            }
            Err(error) => errors.push(format!("{source}: {error}")),
        }
    }
    let Some((source, downloaded)) = got else {
        bail!("PDF を取れませんでした: {}", errors.join("; "));
    };

    // .pdf.part → .pdf にして Library に登録する（TUI と同じ処理）。
    let registered = downloads::register_downloaded_pdf(&mut db, &paper, &downloaded, &data_dir)
        .map_err(|error| anyhow::anyhow!("{error}"))?;

    // 要旨の日本語訳（Download の成否には影響させない。失敗しても downloaded のまま）。
    let abstract_ja = translate_abstract_if_needed(&db, &cfg, registered.paper_id, &paper).await;

    Ok(json!({
        "paper_id": registered.paper_id,
        "title": paper.title,
        "status": "downloaded",
        "pdf_path": registered.path.to_string_lossy(),
        "source": source,
        "abstract_ja": abstract_ja,
    }))
}

/// `--log` / `--import-id` から対象の [`sai_paper_core::models::RemotePaper`] を探す。
fn find_target_paper(
    db: &Database,
    paths: &Paths,
    log_id: Option<i64>,
    position: Option<i64>,
    import_id: Option<&str>,
) -> Result<sai_paper_core::models::RemotePaper> {
    match (log_id, import_id) {
        (Some(log_id), None) => {
            let position =
                position.context("phone download には --log と --position の両方が必要です")?;
            search_log::items(db.connection(), log_id)?
                .into_iter()
                .find(|item| item.position == position)
                .map(|item| item.remote)
                .context("論文が見つかりません")
        }
        (None, Some(import_id)) => load_bibtex_dir_detailed(&bibtex_dir(paths))?
            .papers
            .into_iter()
            .find(|paper| paper.id == import_id)
            .context("論文が見つかりません"),
        _ => bail!(
            "phone download には --log と --position、または --import-id のどちらかを指定してください"
        ),
    }
}

/// 取りに行く PDF の URL 候補（`(出どころ, URL)`）。
///
/// 1. `RemotePaper.pdf_url`（TUI の Discover のヒット）
/// 2. DOI か arXiv の ID が分かるなら、`get-pdf` と同じ OA の候補
///    （S2 の `openAccessPdf` → arXiv → Europe PMC → Unpaywall）
///
/// OA の問い合わせが失敗しても 1. が残っていれば続ける。
async fn pdf_candidates(
    client: &S2Client,
    paper: &sai_paper_core::models::RemotePaper,
) -> Result<Vec<(String, String)>> {
    let mut candidates: Vec<(String, String)> = pdf_url_candidate(paper).into_iter().collect();
    // OA の問い合わせに使う key（DOI か arXiv。どちらの kind も無ければ OA の候補は調べない）。
    if let Some(key) = oa_key(paper)
        && let Ok(extra) = crate::litbase::oa_pdf_candidates(client, &key).await
    {
        for (source, url) in extra {
            if !candidates.iter().any(|(_, existing)| existing == &url) {
                candidates.push((source, url));
            }
        }
    }
    Ok(candidates)
}

/// `RemotePaper.pdf_url` の候補（空文字は候補にしない）。
fn pdf_url_candidate(paper: &sai_paper_core::models::RemotePaper) -> Option<(String, String)> {
    paper
        .pdf_url
        .as_deref()
        .map(str::trim)
        .filter(|url| !url.is_empty())
        .map(|url| ("pdf_url".to_owned(), url.to_owned()))
}

/// OA の候補を S2 に問い合わせる時の論文 key。
///
/// - DOI があれば `DOI:<doi>`（S2 が受け付ける形）
/// - `ArXiv:` / `DOI:` で始まる id ならその id のまま（S2 が paperId キーとして解釈する）
/// - どれも無ければ `None`（OA の候補は調べない）
fn oa_key(paper: &sai_paper_core::models::RemotePaper) -> Option<String> {
    if let Some(doi) = paper
        .doi
        .as_deref()
        .map(str::trim)
        .filter(|doi| !doi.is_empty())
    {
        return Some(format!("DOI:{doi}"));
    }
    ["ArXiv:", "DOI:"]
        .into_iter()
        .find(|prefix| paper.id.starts_with(prefix))
        .and_then(|_| Some(paper.id.trim().to_owned()))
        .filter(|id| id.len() > "ArXiv:".len())
}

/// 1 つの候補を [`DownloadManager`] で取りにいき、一時ファイル（`.pdf.part`）の場所を返す。
///
/// 完了イベント（`DownloadEvent::Completed`）で受け取った一時ファイルのパスを返す。
async fn download_once(
    manager: &DownloadManager,
    id: &str,
    url: &str,
    destination: &Path,
) -> Result<PathBuf> {
    let (events, mut receiver) = mpsc::unbounded_channel();
    let mut error: Option<String> = None;
    if let Err(report) = manager.download(id, url, destination, &events).await {
        error = Some(report.to_string());
    }
    drop(events);
    while let Some(event) = receiver.recv().await {
        match event {
            DownloadEvent::Completed { path, .. } => return Ok(path),
            DownloadEvent::Failed { error: message, .. } => error = Some(message),
            DownloadEvent::Started { .. } | DownloadEvent::Progress { .. } => {}
        }
    }
    Err(anyhow::anyhow!(
        error.unwrap_or_else(|| "転送しませんでした".to_owned())
    ))
}

/// 受け取った物が `%PDF-` で始まるか（TUI の手動取り込みと同じ水準）。
fn looks_like_pdf(path: &Path) -> bool {
    let mut magic = [0_u8; 5];
    let Ok(mut file) = std::fs::File::open(path) else {
        return false;
    };
    std::io::Read::read_exact(&mut file, &mut magic).is_ok() && &magic == b"%PDF-"
}

/// 要旨があり、まだ訳していなければ日本語訳して保存する（訳できたら `true`）。
///
/// 失敗しても Download の結果には影響しない（呼び出し側は戻り値を捨てるだけ）。
async fn translate_abstract_if_needed(
    db: &Database,
    cfg: &Config,
    paper_id: i64,
    paper: &sai_paper_core::models::RemotePaper,
) -> bool {
    let abstract_en = paper.abstract_text.trim();
    if abstract_en.is_empty() {
        return false;
    }
    let already = db
        .library_paper_by_id(paper_id)
        .ok()
        .flatten()
        .and_then(|saved| saved.abstract_ja)
        .is_some_and(|ja| !ja.trim().is_empty());
    if already {
        return false;
    }
    matches!(
        translation::ensure_abstract_ja(
            db.connection(),
            paper_id,
            abstract_en,
            false,
            &cfg.translate.abstract_
        )
        .await,
        Ok(Some(_))
    )
}

/// `phone after-download <paper_id>`: Download の後の自動処理（図の抽出 → PDF 翻訳）。
///
/// TUI の `spawn_paper_pipeline` と同じ `pipeline::after_download` を使い、
/// 途中の文を `messages` に集める。PDF が紐付いていない論文は失敗。
pub async fn after_download(paper_id: i64) -> Result<Value> {
    let paths = Paths::discover()?;
    let cfg = Config::load_or_create(&paths)?;
    {
        let db = open_database()?;
        let Some(paper) = db.library_paper_by_id(paper_id)? else {
            bail!("論文が見つかりません");
        };
        if paper
            .pdf_path
            .as_deref()
            .is_none_or(|pdf| pdf.trim().is_empty())
        {
            bail!("PDFが紐付いていません");
        }
    }

    let collected: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&collected);
    pipeline::after_download(
        &paths.database_file,
        paper_id,
        &cfg,
        move |message| {
            if let Ok(mut messages) = sink.lock() {
                messages.push(message);
            }
        },
        |_| {},
    )
    .await;

    let messages = collected
        .lock()
        .map(|messages| messages.clone())
        .unwrap_or_default();
    Ok(json!({ "paper_id": paper_id, "messages": messages }))
}

/// `phone attention-resolve <key>`: 要対応の 1 件を「対応済み」にする（JSON で返す）。
///
/// 処理は `sai-paper attention resolve` と同じ（core の
/// [`attention::collect`] → [`attention::resolve`]）。違うのは表示が JSON なだけ。
pub fn attention_resolve(key: &str) -> Result<Value> {
    let db = open_database()?;
    let roots = paths::paper_roots().context("プロジェクトルート（.sai/paper）が見つかりません")?;
    let items = attention::collect(db.connection(), &roots)?;
    let item = items
        .iter()
        .find(|item| item.key == key)
        .with_context(|| format!("要対応の key が見つかりません: {key}"))?;
    attention::resolve(&db, item)?;
    Ok(json!({ "ok": true, "key": key }))
}

#[cfg(test)]
mod tests {
    use super::{MAX_BIB_BYTES, looks_like_pdf, oa_key, pdf_url_candidate};
    use sai_paper_core::models::RemotePaper;

    fn paper(id: &str, pdf_url: Option<&str>, doi: Option<&str>) -> RemotePaper {
        let now = chrono::Utc::now();
        RemotePaper {
            id: id.to_owned(),
            title: "Title".to_owned(),
            authors: Vec::new(),
            abstract_text: String::new(),
            published: now,
            updated: now,
            categories: Vec::new(),
            pdf_url: pdf_url.map(str::to_owned),
            doi: doi.map(str::to_owned),
            journal_ref: None,
            citation_count: None,
        }
    }

    /// 1MB の上限（`phone import add` の受け口）。
    #[test]
    fn bib_limit_is_one_mib() {
        assert_eq!(MAX_BIB_BYTES, 1024 * 1024);
    }

    /// `%PDF-` で始まるファイルだけ差し上げる（TUI の手動取り込みと同じ水準）。
    #[test]
    fn pdf_magic_check() {
        let dir = std::env::temp_dir().join(format!(
            "sai-paper-phone-import-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let good = dir.join("good.pdf");
        let bad = dir.join("bad.pdf");
        let short = dir.join("short.pdf");
        std::fs::write(&good, b"%PDF-1.7 rest").unwrap();
        std::fs::write(&bad, b"<html>nope</html>").unwrap();
        std::fs::write(&short, b"%PD").unwrap();
        assert!(looks_like_pdf(&good));
        assert!(!looks_like_pdf(&bad));
        assert!(!looks_like_pdf(&short));
        assert!(!looks_like_pdf(&dir.join("missing.pdf")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `pdf_url` があれば先頭の候補になる（空文字は候補にしない）。
    #[test]
    fn pdf_url_is_the_first_candidate() {
        let candidate = pdf_url_candidate(&paper("ArXiv:1", Some("https://e/x.pdf"), None))
            .expect("候補がある");
        assert_eq!(
            candidate,
            ("pdf_url".to_owned(), "https://e/x.pdf".to_owned())
        );
        assert!(pdf_url_candidate(&paper("ArXiv:1", Some("  "), None)).is_none());
    }

    /// OA の問い合わせの key（DOI は `DOI:` 付き、`ArXiv:` の id はそのまま）。
    #[test]
    fn oa_key_uses_doi_or_arxiv_id() {
        assert_eq!(
            oa_key(&paper("DOI:10.1/x", None, Some("10.1/x"))).as_deref(),
            Some("DOI:10.1/x")
        );
        assert_eq!(
            oa_key(&paper("ArXiv:1706.03762", None, None)).as_deref(),
            Some("ArXiv:1706.03762")
        );
        assert!(oa_key(&paper("BibTeX:Key2020", None, None)).is_none());
    }
}
