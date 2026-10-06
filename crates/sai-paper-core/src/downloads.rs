//! Streaming PDF download manager.
use std::path::{Path, PathBuf};

use futures_util::StreamExt;
use reqwest::Client;
use thiserror::Error;
use tokio::{fs, io::AsyncWriteExt, sync::mpsc};

use crate::models::RemotePaper;

/// Progress events emitted by a PDF download.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DownloadEvent {
    /// Transfer began and may have a known content length.
    Started {
        /// Provider paper identifier.
        id: String,
        /// Expected response bytes.
        total: Option<u64>,
    },
    /// More response bytes were persisted.
    Progress {
        /// Provider paper identifier.
        id: String,
        /// Bytes persisted so far.
        downloaded: u64,
        /// Expected response bytes.
        total: Option<u64>,
    },
    /// The temporary file was atomically promoted.
    Completed {
        /// Provider paper identifier.
        id: String,
        /// Final PDF path.
        path: PathBuf,
    },
    /// The transfer could not complete.
    Failed {
        /// Provider paper identifier.
        id: String,
        /// Human-readable failure.
        error: String,
    },
}

/// PDF transfer errors.
#[derive(Debug, Error)]
pub enum DownloadError {
    /// Network transfer failed.
    #[error("download request failed: {0}")]
    Request(#[from] reqwest::Error),
    /// Destination file operation failed.
    #[error("download file operation failed: {0}")]
    Io(#[from] std::io::Error),
}

/// Reusable asynchronous PDF downloader.
#[derive(Debug, Clone)]
pub struct DownloadManager {
    client: Client,
}

impl DownloadManager {
    /// Create a downloader with a bounded transfer timeout.
    ///
    /// # Errors
    ///
    /// Returns an error when the HTTP client cannot be configured.
    pub fn new() -> Result<Self, DownloadError> {
        Ok(Self {
            client: Client::builder()
                .user_agent(concat!("papr/", env!("CARGO_PKG_VERSION")))
                .timeout(std::time::Duration::from_mins(5))
                .build()?,
        })
    }

    /// Stream a URL to a partial file and atomically finalize it.
    ///
    /// # Errors
    ///
    /// Returns an error for HTTP failures or destination file errors.
    pub async fn download(
        &self,
        id: &str,
        url: &str,
        destination: &Path,
        events: &mpsc::UnboundedSender<DownloadEvent>,
    ) -> Result<(), DownloadError> {
        let temp_destination = destination.with_extension("pdf.part");
        if let Some(parent) = temp_destination.parent() {
            fs::create_dir_all(parent).await?;
        }
        let response = self.client.get(url).send().await?.error_for_status()?;
        let total = response.content_length();
        let _ = events.send(DownloadEvent::Started {
            id: id.to_owned(),
            total,
        });
        let mut file = fs::File::create(&temp_destination).await?;
        let mut downloaded = 0_u64;
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            file.write_all(&chunk).await?;
            downloaded = downloaded.saturating_add(chunk.len() as u64);
            let _ = events.send(DownloadEvent::Progress {
                id: id.to_owned(),
                downloaded,
                total,
            });
        }
        file.flush().await?;
        drop(file);
        let _ = events.send(DownloadEvent::Completed {
            id: id.to_owned(),
            path: temp_destination,
        });
        Ok(())
    }
}

/// Download した PDF の置き場（`download_dir` 直下にタイトル由来のファイル名で置く）。
///
/// ファイル名の付け方は TUI の Download と共通（`sanitize_download_filename_component`。
/// タイトルが空なら id の末尾を使う）。
#[must_use]
pub fn download_destination(dir: &Path, paper: &RemotePaper) -> PathBuf {
    let sanitized_title = crate::paths::sanitize_download_filename_component(&paper.title);
    let filename = if sanitized_title.is_empty() {
        paper
            .id
            .rsplit('/')
            .next()
            .unwrap_or("paper")
            .chars()
            .map(|c| if c == '/' { '_' } else { c })
            .collect::<String>()
    } else {
        sanitized_title
    };
    dir.join(format!("{filename}.pdf"))
}

/// Download 完了時の Library 登録の結果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegisteredPdf {
    /// 登録された論文の DB ID（`papers.id`）。
    pub paper_id: i64,
    /// `attach_download` が論文フォルダへ動かした後の PDF の実パス
    /// （DB に記録されたパスを採用する）。
    pub path: PathBuf,
    /// PDF の大きさ（バイト）。
    pub file_size: u64,
}

/// Download 完了時の登録のエラー。
#[derive(Debug, Error)]
pub enum RegisterError {
    /// 一時ファイル（`.pdf.part`）を `.pdf` へ改名できなかった。
    #[error("一時ファイルを PDF へ改名できません: {0}")]
    Rename(#[from] std::io::Error),
    /// PDF を読み込めない（ハッシュ計算・メタデータ取得の失敗）。
    #[error("PDF を読めません: {0}")]
    Inspect(#[from] crate::library::LibraryError),
    /// DB の書き込みに失敗した。
    #[error("DB error: {0}")]
    Database(#[from] crate::database::DatabaseError),
}

/// Download し終えた PDF（`.pdf.part`）を `.pdf` にし、Library に登録する。
///
/// TUI の Download 完了時と `sai-paper paper download` が共通で使う処理。
/// 検索セットへの紐付け・プラグイン通知・要旨の訳・図抽出のパイプラインなど
/// 画面固有の処理は呼び出し側（TUI / CLI）が行う。
///
/// # Errors
///
/// 改名が [`RegisterError::Rename`]、PDF の読み込みが [`RegisterError::Inspect`]、
/// DB の書き込みが [`RegisterError::Database`] になる。
pub fn register_downloaded_pdf(
    db: &mut crate::Database,
    paper: &RemotePaper,
    downloaded: &Path,
    data_dir: &Path,
) -> Result<RegisteredPdf, RegisterError> {
    // `.pdf.part` → `.pdf`
    let final_path = downloaded.with_extension("");
    if downloaded.exists() {
        std::fs::rename(downloaded, &final_path)?;
    }
    let pdf = crate::LibraryIndexer::inspect(&final_path)?;
    // 論文フォルダへの移動先は attach_download 内の paths::paper_folder が決める。
    let paper_id = db.attach_download(paper, &pdf, data_dir)?;
    db.record_activity("downloaded", Some(paper_id), None)?;
    // attach_download が PDF を論文フォルダ（絶対パス）へ移動済み。
    // 以後の翻訳などの入力に使うのは DB に記録された実パス。
    let path = db
        .library_paper_by_id(paper_id)?
        .and_then(|saved| saved.pdf_path)
        .map_or(final_path, PathBuf::from);
    Ok(RegisteredPdf {
        paper_id,
        path,
        file_size: pdf.file_size,
    })
}

#[cfg(test)]
mod tests {
    use super::{download_destination, register_downloaded_pdf};
    use crate::models::RemotePaper;
    use std::path::Path;

    fn paper(id: &str, title: &str) -> RemotePaper {
        let now = chrono::Utc::now();
        RemotePaper {
            id: id.to_owned(),
            title: title.to_owned(),
            authors: Vec::new(),
            abstract_text: String::new(),
            published: now,
            updated: now,
            categories: Vec::new(),
            pdf_url: None,
            doi: None,
            journal_ref: None,
            citation_count: None,
        }
    }

    /// 置き場のファイル名はタイトル由来（スラッシュは `_` になる）。
    #[test]
    fn destination_uses_sanitized_title() {
        let dest = download_destination(Path::new("/x/papers"), &paper("DOI:10.1/a", "A/B paper"));
        assert_eq!(dest, Path::new("/x/papers/A_B paper.pdf"));
    }

    /// タイトルが空なら id の末尾を使う。
    #[test]
    fn destination_falls_back_to_id_tail() {
        let dest = download_destination(Path::new("/x/papers"), &paper("DOI:10.1/a", "  "));
        assert_eq!(dest, Path::new("/x/papers/a.pdf"));
    }

    /// `.pdf.part` を `.pdf` に変えて Library に登録し、実パスを返す。
    #[test]
    fn register_moves_temp_file_and_registers() {
        let dir = std::env::temp_dir().join(format!(
            "sai-paper-register-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let data_dir = dir.join(".sai/paper");
        std::fs::create_dir_all(dir.join("downloads")).unwrap();
        let part = dir.join("downloads").join("paper.pdf.part");
        std::fs::write(&part, b"%PDF-1.7\n%stub").unwrap();

        let mut db = crate::Database::open(&data_dir.join("papr.db")).unwrap();
        let paper = paper("DOI:10.1/register-test", "Register Test");
        let registered =
            register_downloaded_pdf(&mut db, &paper, &part, &data_dir).unwrap();

        assert!(!part.exists());
        assert!(registered.path.is_file());
        assert!(registered.path.ends_with("paper.pdf"));
        assert!(registered.file_size > 0);
        assert!(db.library_paper_by_id(registered.paper_id).unwrap().is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }
}

