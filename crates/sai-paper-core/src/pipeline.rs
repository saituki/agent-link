//! 論文 1 本ぶんの重い処理（図の抽出・PDF 翻訳・DL 後の自動処理。段階 5）。
//!
//! TUI・CLI・phone のどこからでも同じ関数を呼ぶ。どの処理も `jobs` 表に記録し、
//! 同じ論文・同じ種類の処理が実行中なら始めない。

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use tokio::sync::Semaphore;

use crate::Database;

/// 重い処理を同時に走らせる数の上限（`settings.json` の `pipeline.max_parallel`）。
///
/// 最初に呼んだときの値で決まる（途中で設定を変えても、次の起動まで変わらない）。
pub fn slots(max_parallel: usize) -> &'static Semaphore {
    static SLOTS: OnceLock<Semaphore> = OnceLock::new();
    SLOTS.get_or_init(|| Semaphore::new(max_parallel.max(1)))
}

/// 論文の PDF（実ファイル）と題名を DB から引く。
fn paper_pdf(db: &Database, paper_id: i64) -> Result<(String, PathBuf), String> {
    let paper = db
        .library_paper_by_id(paper_id)
        .map_err(|e| format!("DB error {e}"))?
        .ok_or_else(|| "論文が見つかりません".to_string())?;
    let pdf = paper
        .pdf_path
        .filter(|path| !path.trim().is_empty())
        .map(PathBuf::from)
        .ok_or_else(|| "PDFが紐付いていません".to_string())?;
    if !pdf.is_file() {
        return Err(format!("PDFが見つかりません: {}", pdf.display()));
    }
    Ok((paper.title, pdf))
}

/// 図を抽出する（docling）。抽出した図の数を返す。
///
/// # Errors
/// 利用者向けの文（PDF が無い・docling の失敗・実行中など）。
pub async fn extract_figures(db_file: &Path, paper_id: i64) -> Result<usize, String> {
    crate::jobs::run(db_file, "figures", Some(paper_id), || async {
        let db = Database::open(db_file).map_err(|e| format!("DB open error {e}"))?;
        let (_, pdf) = paper_pdf(&db, paper_id)?;
        let key = db
            .citation_key_for_paper(paper_id)
            .map(|raw| crate::citation::normalize_paper_id(&raw))
            .ok_or_else(|| "抽出キーが決まりません（DOI / arXiv ID が無い）".to_string())?;
        let paper_dir = pdf.parent().map(Path::to_path_buf).unwrap_or_default();
        crate::figures::extract_figures(
            db.connection(),
            &key,
            Some(paper_id),
            &pdf,
            &paper_dir,
            db.path_base(),
            false,
        )
        .await
        .map_err(|e| e.to_string())
    })
    .await
}

/// PDF を日本語に翻訳する（pdf2zh）。できた翻訳 PDF（論文フォルダの `paper-ja.pdf`）を返す。
///
/// `paper_translations` に開始・完了・失敗を記録する。
///
/// # Errors
/// 利用者向けの文（PDF が無い・翻訳の失敗・実行中など）。
pub async fn translate_pdf(
    db_file: &Path,
    paper_id: i64,
    translate: &crate::config::TranslateService,
) -> Result<PathBuf, String> {
    crate::jobs::run(db_file, "translate_pdf", Some(paper_id), || async {
        let db = Database::open(db_file).map_err(|e| format!("DB open error {e}"))?;
        let (title, pdf) = paper_pdf(&db, paper_id)?;
        let key = crate::citation::normalize_paper_id(
            &db.citation_key_for_paper(paper_id)
                .unwrap_or_else(|| pdf.to_string_lossy().into_owned()),
        );
        let mtime = std::fs::metadata(&pdf)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs().to_string())
            .unwrap_or_default();
        // パスは data_dir からの相対で書く（段階 2b）。同じ論文（papers_id）で
        // 古い抽出キーに残っている行は消す（段階 2c）。
        let stored_pdf = db.store_path(&pdf);
        let record = |sql: &str, extra: &[&dyn rusqlite::ToSql]| {
            let _ = db.connection().execute(
                "DELETE FROM paper_translations WHERE papers_id = ?1 AND paper_id <> ?2",
                rusqlite::params![paper_id, key],
            );
            let mut values: Vec<&dyn rusqlite::ToSql> =
                vec![&key, &stored_pdf, &mtime, &translate.service, &paper_id];
            values.extend_from_slice(extra);
            let _ = db.connection().execute(sql, values.as_slice());
        };
        record(
            "INSERT INTO paper_translations (paper_id, status, pdf_path, pdf_mtime, service, lang_in, lang_out, papers_id)
             VALUES (?1, 'translating', ?2, ?3, ?4, 'en', 'ja', ?5)
             ON CONFLICT(paper_id) DO UPDATE SET status='translating', pdf_path=?2, pdf_mtime=?3, service=?4, error=NULL,
                papers_id=COALESCE(excluded.papers_id, paper_translations.papers_id)",
            &[],
        );

        match crate::translation::pdf2zh::run_pdf2zh(&pdf, translate, "en", "ja").await {
            Ok(ja_path) => {
                // 翻訳 PDF は論文フォルダの paper-ja.pdf に置く。
                let folder = db
                    .data_dir()
                    .map(|dir| crate::paths::paper_folder(dir, paper_id, &title));
                let final_ja = match folder {
                    Some(folder) => {
                        let _ = std::fs::create_dir_all(&folder);
                        let target = folder.join(crate::paths::paper_file_names::PDF_JA);
                        if crate::paths::move_pdf_file(&ja_path, &target).is_ok() {
                            target
                        } else {
                            ja_path
                        }
                    }
                    None => ja_path,
                };
                let stored_ja = db.store_path(&final_ja);
                let now = chrono::Utc::now().to_rfc3339();
                record(
                    "INSERT INTO paper_translations (paper_id, status, pdf_path, pdf_mtime, service, papers_id, pdf_ja_path, translated_at)
                     VALUES (?1, 'ready', ?2, ?3, ?4, ?5, ?6, ?7)
                     ON CONFLICT(paper_id) DO UPDATE SET status='ready', pdf_path=?2, pdf_mtime=?3, service=?4, pdf_ja_path=?6,
                        translated_at=?7, error=NULL, papers_id=COALESCE(excluded.papers_id, paper_translations.papers_id)",
                    &[&stored_ja, &now],
                );
                // Dashboard の出来事に「translate」として残す
                let _ = db.record_activity("translate", Some(paper_id), Some("PDF"));
                Ok(final_ja)
            }
            Err(e) => {
                let message = e.to_string();
                record(
                    "INSERT INTO paper_translations (paper_id, status, pdf_path, pdf_mtime, service, papers_id, error)
                     VALUES (?1, 'failed', ?2, ?3, ?4, ?5, ?6)
                     ON CONFLICT(paper_id) DO UPDATE SET status='failed', pdf_path=?2, pdf_mtime=?3, service=?4, error=?6,
                        papers_id=COALESCE(excluded.papers_id, paper_translations.papers_id)",
                    &[&message],
                );
                Err(message)
            }
        }
    })
    .await
}

/// DL 後の自動処理で、まだ済んでいないもの（図の抽出, PDF 翻訳）。
///
/// 図は抽出済み（ready / empty）なら、翻訳は翻訳 PDF があれば飛ばす。
#[must_use]
pub fn pending_after_download(db: &Database, paper_id: i64) -> (bool, bool) {
    let Ok((_, pdf)) = paper_pdf(db, paper_id) else {
        return (false, false);
    };
    let key = db
        .citation_key_for_paper(paper_id)
        .map(|raw| crate::citation::normalize_paper_id(&raw));
    let figures_done = crate::figures::get_figure_extract_for(db.connection(), Some(paper_id), key.as_deref())
        .ok()
        .flatten()
        .is_some_and(|extract| matches!(extract.status.as_str(), "ready" | "empty"));
    let translated = crate::paths::translated_pdf_path(&pdf.to_string_lossy()).is_some();
    (!figures_done && key.is_some(), !translated)
}

/// DL 後の自動処理（図の抽出 → PDF 翻訳）。1 論文の中は順番に、論文どうしは
/// `pipeline.max_parallel` 本まで並べて走らせる。
///
/// `notify` は結果の文（TUI のトースト等）、`on_step` はステップ番号
/// （1: 図の抽出, 2: 翻訳）を受け取る。
pub async fn after_download(
    db_file: &Path,
    paper_id: i64,
    config: &crate::Config,
    notify: impl Fn(String),
    on_step: impl Fn(usize),
) {
    let (do_figures, do_translate) = match Database::open(db_file) {
        Ok(db) => pending_after_download(&db, paper_id),
        Err(_) => return,
    };
    if !do_figures && !do_translate {
        return;
    }
    // ponytail: 同時実行数だけを絞る単純な上限。待っている間も表示は「図抽出」のまま。
    let _permit = slots(config.pipeline.max_parallel).acquire().await;
    on_step(1);
    if do_figures {
        notify(match extract_figures(db_file, paper_id).await {
            Ok(0) => "図は見つかりませんでした".to_string(),
            Ok(count) => format!("図を {count} 件抽出しました"),
            Err(e) => format!("図抽出失敗: {e}"),
        });
    }
    on_step(2);
    if do_translate {
        notify(match translate_pdf(db_file, paper_id, &config.translate.pdf).await {
            Ok(path) => format!("PDF翻訳完了: {}", path.display()),
            Err(e) => format!("PDF翻訳失敗: {e}"),
        });
    }
}
