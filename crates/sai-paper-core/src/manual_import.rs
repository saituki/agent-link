//! 手動 PDF 取り込みの共通処理（候補の探し・保存先の決定・登録）。
//!
//! ブラウザで保存した PDF を利用者が手動で Library に入れる流れは、論文 TUI と
//! 引用 TUI と Web（`sai-paper phone`）の 3 つの窓口から使う。ここに 1 つだけ置き、
//! 窓口ごとの後処理（プラグイン通知・翻訳・図の抽出）は呼び出し側に残す。

use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use thiserror::Error;

use crate::database::{Database, DatabaseError};
use crate::library::LibraryError;
use crate::models::RemotePaper;
use crate::paths::{self, move_pdf_file, paper_file_path, paper_folder};
use crate::LibraryIndexer;

/// TUI が見せる候補の窓（5 分）。
pub const TUI_WINDOW: Duration = Duration::from_secs(5 * 60);

/// 取り込み先の出し先。
#[derive(Debug, Clone)]
pub enum ManualTarget {
    /// 既に Library にある論文へ PDF を付ける。
    Local {
        /// 論文 ID（`papers.id`）。
        paper_id: i64,
        /// 論文の題名（論文フォルダの名前に使う）。
        title: String,
    },
    /// 検索結果・BibTeX・引用の論文を、新しく Library に入れる。
    Remote(RemotePaper),
}

/// 取り込みの結果。
#[derive(Debug, Clone)]
pub struct ManualImported {
    /// 登録後の論文 ID（`papers.id`）。
    pub paper_id: i64,
    /// 登録後の PDF の実パス（DB に残ったもの）。
    pub path: PathBuf,
    /// 登録後の PDF のファイル名（表示用）。
    pub file_name: String,
}

/// 手動取り込みの失敗。
#[derive(Debug, Error)]
pub enum ManualImportError {
    /// ファイルを読めなかった（開けない・先頭が読めない）。
    #[error("ファイルを読めませんでした: {0}")]
    Io(#[from] std::io::Error),
    /// 先頭が `%PDF-` でなかった。
    #[error("PDF のファイルではありません: {0}")]
    NotPdf(PathBuf),
    /// 論文フォルダを作れなかった。
    #[error("論文フォルダを作れませんでした: {0}")]
    Folder(PathBuf, #[source] std::io::Error),
    /// 保存先へ移せなかった。
    #[error("PDF を移せませんでした: {0}")]
    Move(PathBuf, #[source] std::io::Error),
    /// 移した PDF の情報（大きさ・内容ハッシュ）を読めなかった。
    #[error("PDF の情報を読めませんでした: {0}")]
    Inspect(#[source] LibraryError),
    /// 論文として登録できなかった。
    #[error("論文として登録できませんでした: {0}")]
    Database(#[from] DatabaseError),
}

/// ユーザーのダウンロードディレクトリ（ブラウザの保存先）を解決する。
///
/// 優先順位は `XDG_DOWNLOAD_DIR` → `~/.config/user-dirs.dirs` → `~/Downloads` →
/// `~/ダウンロード`（日本語ロケール）。実在する最初の候補を返す。
#[must_use]
pub fn downloads_dir() -> Option<PathBuf> {
    let home = home_directory()?;
    resolve_download_dir(&home)
}

/// ホームディレクトリ（`HOME` → `USERPROFILE`）。
fn home_directory() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

/// home 配下で実際に使うダウンロードディレクトリを優先順に解決する（純粋関数）。
fn resolve_download_dir(home: &Path) -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();

    // 1. XDG_DOWNLOAD_DIR 環境変数（xdg-user-dirs がログイン時にエクスポートする）
    if let Some(raw) = std::env::var_os("XDG_DOWNLOAD_DIR") {
        candidates.push(expand_home(&raw.to_string_lossy(), home));
    }

    // 2. ~/.config/user-dirs.dirs に書かれた XDG_DOWNLOAD_DIR
    //    このファイルはロケール名（例: 日本語なら ダウンロード）で記録されるため
    //    読む必要がある
    if let Some(from_file) = user_dirs_download_dir(home) {
        candidates.push(from_file);
    }

    // 3. 一般的なフォールバック（英語 / 日本語ロケール）
    candidates.push(home.join("Downloads"));
    candidates.push(home.join("ダウンロード"));

    // 実在する最初の候補を返す
    candidates.into_iter().find(|dir| dir.is_dir())
}

/// "$HOME/..." の形式を home に展開する（それ以外は home に結合する）。
fn expand_home(raw: &str, home: &Path) -> PathBuf {
    raw.strip_prefix("$HOME/")
        .map_or_else(|| home.join(raw), |rest| home.join(rest))
}

/// ~/.config/user-dirs.dirs から XDG_DOWNLOAD_DIR の値を読み取る。
fn user_dirs_download_dir(home: &Path) -> Option<PathBuf> {
    let contents = std::fs::read_to_string(home.join(".config").join("user-dirs.dirs")).ok()?;
    for line in contents.lines() {
        let line = line.trim();
        let Some(rest) = line.strip_prefix("XDG_DOWNLOAD_DIR=") else {
            continue;
        };
        // クォートを外して $HOME を展開する
        let value = rest.trim().trim_matches('"');
        return Some(expand_home(value, home));
    }
    None
}

/// 指定ディレクトリ直下の PDF のうち、更新時刻が window 以内のものを新しい順に返す。
#[must_use]
pub fn recent_pdfs(dir: &Path, window: Duration) -> Vec<PathBuf> {
    let cutoff = SystemTime::now()
        .checked_sub(window)
        .unwrap_or(SystemTime::UNIX_EPOCH);
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut items: Vec<(SystemTime, PathBuf)> = entries
        .flatten()
        .filter(|entry| {
            entry
                .path()
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("pdf"))
        })
        .filter_map(|entry| {
            let modified = entry.metadata().ok()?.modified().ok()?;
            (modified >= cutoff).then_some((modified, entry.path()))
        })
        .collect();
    items.sort_by_key(|(modified, _)| std::cmp::Reverse(*modified));
    items.into_iter().map(|(_, path)| path).collect()
}

/// 自動 DL と同じ正規名で、衝突しない保存先を決める。
///
/// `<題名>.pdf` を試し、既にあれば `<題名> (2).pdf` と番号を足していく。
#[must_use]
pub fn unique_pdf_destination(dir: &Path, title: &str) -> PathBuf {
    let base = paths::sanitize_download_filename_component(title);
    let base = if base.is_empty() {
        "paper".to_owned()
    } else {
        base
    };
    let mut candidate = dir.join(format!("{base}.pdf"));
    let mut suffix = 2;
    while candidate.exists() {
        candidate = dir.join(format!("{base} ({suffix}).pdf"));
        suffix += 1;
    }
    candidate
}

/// 先頭 5 バイトが `%PDF-` かどうか。
///
/// # Errors
///
/// ファイルを開けないとき・先頭 5 バイトを読めないときはその理由を返す。
pub fn is_pdf_file(path: &Path) -> std::io::Result<bool> {
    let mut magic = [0_u8; 5];
    let mut file = std::fs::File::open(path)?;
    file.read_exact(&mut magic)?;
    Ok(&magic == b"%PDF-")
}

/// 候補 PDF を検査・移動し、対象論文に紐付けて Library に登録する。
///
/// 行うのは次の 6 つまで（TUI だけの後処理・プラグイン通知・翻訳・図の抽出は行わない）。
///
/// 1. `%PDF-` の検査
/// 2. 保存先の決定（`Local` は論文フォルダの `paper.pdf`、既にあれば衝突しない名前）
/// 3. 保存先への移動
/// 4. [`LibraryIndexer::inspect`]（大きさ・内容ハッシュ）
/// 5. 登録（`Local` は `attach_manual_pdf`、`Remote` は `attach_download`）
/// 6. 出来事に `downloaded` を残す
///
/// # Errors
///
/// PDF で無い・移動できない・登録できないときは [`ManualImportError`] を返す。
pub fn import_pdf(
    db: &mut Database,
    data_dir: &Path,
    download_dir: &Path,
    source: &Path,
    target: &ManualTarget,
) -> Result<ManualImported, ManualImportError> {
    // 1. %PDF マジック検証（自動 DL と同一の水準）
    if !is_pdf_file(source)? {
        return Err(ManualImportError::NotPdf(source.to_path_buf()));
    }

    // 2. 保存先を決める。Local は論文フォルダの `paper.pdf`、既にあれば
    //    既存の PDF を上書きしないよう衝突しない名前に逃がす。
    let title = match target {
        ManualTarget::Local { title, .. } => title.clone(),
        ManualTarget::Remote(paper) => paper.title.clone(),
    };
    let destination = match target {
        ManualTarget::Local { paper_id, .. } => {
            let folder = paper_folder(data_dir, *paper_id, &title);
            let candidate = paper_file_path(&folder, "pdf");
            if candidate.exists() {
                unique_pdf_destination(download_dir, &title)
            } else {
                // 書き込む前に論文フォルダを作る。
                std::fs::create_dir_all(&folder)
                    .map_err(|error| ManualImportError::Folder(folder.clone(), error))?;
                candidate
            }
        }
        ManualTarget::Remote(_) => unique_pdf_destination(download_dir, &title),
    };

    // 3. 移動
    move_pdf_file(source, &destination)
        .map_err(|error| ManualImportError::Move(destination.clone(), error))?;

    // 4. 中身の情報を読む
    let pdf = LibraryIndexer::inspect(&destination).map_err(ManualImportError::Inspect)?;

    // 5. 登録
    let paper_id = match target {
        ManualTarget::Local { paper_id, .. } => {
            db.attach_manual_pdf(*paper_id, &pdf)?;
            *paper_id
        }
        ManualTarget::Remote(paper) => db.attach_download(paper, &pdf, data_dir)?,
    };

    // 6. 出来事に残す（TUI と同じ「downloaded」）
    db.record_activity("downloaded", Some(paper_id), None)?;

    // 登録後の実パスは DB から取る（attach_download が論文フォルダへ移すことがある）。
    let path = db
        .library_paper_by_id(paper_id)
        .ok()
        .flatten()
        .and_then(|paper| paper.pdf_path)
        .filter(|pdf| !pdf.trim().is_empty())
        .map(PathBuf::from)
        .unwrap_or(destination);
    let file_name = path
        .file_name()
        .map_or_else(|| path.display().to_string(), |name| {
            name.to_string_lossy().into_owned()
        });
    Ok(ManualImported {
        paper_id,
        path,
        file_name,
    })
}

#[cfg(test)]
mod tests {
    use super::{
        ManualTarget, TUI_WINDOW, import_pdf, is_pdf_file, recent_pdfs, resolve_download_dir,
        unique_pdf_destination, user_dirs_download_dir,
    };
    use crate::models::RemotePaper;
    use crate::{Database, LibraryPaper};
    use std::path::PathBuf;

    /// テストごとに別の名前の一時フォルダを作る。
    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "papr-manual-import-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("一時フォルダを作る");
        dir
    }

    /// テスト用の [`RemotePaper`]（取り込みの登録に必要な最低だけ）。
    fn remote(id: &str, title: &str) -> RemotePaper {
        let now = chrono::Utc::now();
        RemotePaper {
            id: id.to_owned(),
            title: title.to_owned(),
            authors: vec!["Vaswani".to_owned()],
            abstract_text: String::new(),
            published: now,
            updated: now,
            categories: Vec::new(),
            pdf_url: None,
            doi: Some("10.1000/xyz".to_owned()),
            journal_ref: Some("NeurIPS".to_owned()),
            citation_count: None,
        }
    }

    /// 窓の中の PDF だけ、新しい順に返す（PDF 以外と窓の外は除く）。
    #[test]
    fn recent_pdfs_keeps_only_fresh_pdfs() {
        let dir = temp_dir("recent");
        let fresh = dir.join("fresh.pdf");
        let stale = dir.join("stale.pdf");
        let ignored = dir.join("notes.txt");
        std::fs::write(&fresh, b"pdf").expect("新しい PDF");
        std::fs::write(&stale, b"pdf").expect("古い PDF");
        std::fs::write(&ignored, b"txt").expect("PDF 以外");
        // 10 分前の時刻に戻す（窓の外）。
        let old = std::time::SystemTime::now() - std::time::Duration::from_secs(600);
        std::fs::OpenOptions::new()
            .write(true)
            .open(&stale)
            .and_then(|file| file.set_times(std::fs::FileTimes::new().set_modified(old)))
            .expect("更新時刻を戻す");

        assert_eq!(recent_pdfs(&dir, TUI_WINDOW), vec![fresh]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 保存先は衝突したら番号を足す（`paper (2).pdf`）。題名が空なら `paper`。
    #[test]
    fn unique_pdf_destination_appends_conflict_suffix() {
        let dir = temp_dir("unique");
        let title = "My Paper: A Study";
        let first = unique_pdf_destination(&dir, title);
        assert_eq!(
            first.file_name().and_then(|n| n.to_str()),
            Some("My Paper: A Study.pdf")
        );
        std::fs::write(&first, b"x").expect("既存の PDF");
        let second = unique_pdf_destination(&dir, title);
        assert_eq!(
            second.file_name().and_then(|n| n.to_str()),
            Some("My Paper: A Study (2).pdf")
        );
        std::fs::write(&second, b"x").expect("既存の PDF");
        assert_eq!(
            unique_pdf_destination(&dir, title)
                .file_name()
                .and_then(|n| n.to_str()),
            Some("My Paper: A Study (3).pdf")
        );
        assert_eq!(
            unique_pdf_destination(&dir, "  ")
                .file_name()
                .and_then(|n| n.to_str()),
            Some("paper.pdf")
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 先頭 5 バイトが `%PDF-` のときだけ true。
    #[test]
    fn is_pdf_file_checks_the_magic_bytes() {
        let dir = temp_dir("magic");
        let good = dir.join("good.pdf");
        let bad = dir.join("bad.pdf");
        let short = dir.join("short.pdf");
        std::fs::write(&good, b"%PDF-1.7 rest").unwrap();
        std::fs::write(&bad, b"<html>nope</html>").unwrap();
        std::fs::write(&short, b"%PD").unwrap();
        assert!(is_pdf_file(&good).unwrap());
        assert!(!is_pdf_file(&bad).unwrap());
        // 5 バイトに満たないファイルは読めない（元の TUI も同じ）。
        assert!(is_pdf_file(&short).is_err());
        assert!(is_pdf_file(&dir.join("missing.pdf")).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 日本語ロケール（user-dirs.dirs に `ダウンロード`）を先に使う。
    #[test]
    fn download_dir_prefers_the_localized_folder() {
        let dir = temp_dir("download-dir");
        let home = dir.join("home");
        std::fs::create_dir_all(home.join(".config")).unwrap();
        std::fs::create_dir_all(home.join("ダウンロード")).unwrap();
        std::fs::write(
            home.join(".config").join("user-dirs.dirs"),
            "XDG_DOWNLOAD_DIR=\"$HOME/ダウンロード\"\n",
        )
        .unwrap();
        assert_eq!(
            user_dirs_download_dir(&home),
            Some(home.join("ダウンロード"))
        );
        assert_eq!(
            resolve_download_dir(&home),
            Some(home.join("ダウンロード"))
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// user-dirs.dirs が無ければ `~/Downloads` に落ちる。
    #[test]
    fn download_dir_falls_back_to_english_folder() {
        let dir = temp_dir("download-fallback");
        let home = dir.join("home");
        std::fs::create_dir_all(home.join("Downloads")).unwrap();
        assert_eq!(resolve_download_dir(&home), Some(home.join("Downloads")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Remote の取り込み（元のファイルは動き、Library に入り、出来事が残る）。
    #[test]
    fn import_pdf_registers_a_remote_paper() {
        let dir = temp_dir("remote");
        let data_dir = dir.join("data");
        std::fs::create_dir_all(&data_dir).unwrap();
        let downloads = dir.join("downloads");
        std::fs::create_dir_all(&downloads).unwrap();
        let source = downloads.join("incoming.pdf");
        std::fs::write(&source, b"%PDF-1.7 body").unwrap();

        let mut db = Database::open(&data_dir.join("papr.db")).unwrap();
        let imported = import_pdf(
            &mut db,
            &data_dir,
            &downloads,
            &source,
            &ManualTarget::Remote(remote("ArXiv:1706.03762", "Attention Is All You Need")),
        )
        .expect("取り込める");
        assert!(!source.exists(), "元のファイルは移動される");
        assert!(imported.path.is_file(), "登録後の PDF が残る");
        // 登録は attach_download が行うので、論文フォルダの `paper.pdf` に入る。
        assert_eq!(imported.file_name, "paper.pdf");
        assert!(imported.path.starts_with(&data_dir));

        let saved: LibraryPaper = db
            .library_paper_by_id(imported.paper_id)
            .unwrap()
            .expect("論文が入っている");
        assert_eq!(saved.title, "Attention Is All You Need");
        // 出来事に download として残る（"downloaded" は "download" に正規化される）。
        let kinds: Vec<String> = db
            .all_activity()
            .unwrap()
            .into_iter()
            .map(|item| item.kind)
            .collect();
        assert!(kinds.iter().any(|kind| kind == "download"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Local の取り込み（論文フォルダの `paper.pdf` になる）。
    ///
    /// 既に `paper.pdf` がある論文へ 2 枚目を付けたときは、既存を上書きせず
    /// ダウンロードフォルダの衝突しない名前になる。
    #[test]
    fn import_pdf_attaches_to_a_local_paper() {
        let dir = temp_dir("local");
        let data_dir = dir.join("data");
        std::fs::create_dir_all(&data_dir).unwrap();
        let downloads = dir.join("downloads");
        std::fs::create_dir_all(&downloads).unwrap();

        // PDF を持っていない論文を 1 件だけ作る。
        let mut db = Database::open(&data_dir.join("papr.db")).unwrap();
        db.connection()
            .execute(
                "INSERT INTO papers (title, doi) VALUES ('Local Paper', '10.1/local')",
                [],
            )
            .expect("論文を 1 件入れる");
        let paper_id = db.connection().last_insert_rowid();

        let source = downloads.join("incoming.pdf");
        std::fs::write(&source, b"%PDF-1.7 body").unwrap();
        let imported = import_pdf(
            &mut db,
            &data_dir,
            &downloads,
            &source,
            &ManualTarget::Local {
                paper_id,
                title: "Local Paper".to_owned(),
            },
        )
        .expect("取り込める");
        assert_eq!(imported.paper_id, paper_id);
        assert_eq!(imported.file_name, "paper.pdf");
        assert!(imported.path.ends_with("paper.pdf"));

        // 2 枚目は既存の上書きを避けてダウンロードフォルダの衝突しない名前。
        let second = downloads.join("incoming2.pdf");
        std::fs::write(&second, b"%PDF-1.7 another").unwrap();
        let imported = import_pdf(
            &mut db,
            &data_dir,
            &downloads,
            &second,
            &ManualTarget::Local {
                paper_id,
                title: "Local Paper".to_owned(),
            },
        )
        .expect("2 枚目も取り込める");
        assert_eq!(imported.file_name, "Local Paper.pdf");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// PDF で無いファイルは動かさず失敗する。
    #[test]
    fn import_pdf_rejects_a_non_pdf_file() {
        let dir = temp_dir("not-pdf");
        let data_dir = dir.join("data");
        std::fs::create_dir_all(&data_dir).unwrap();
        let downloads = dir.join("downloads");
        std::fs::create_dir_all(&downloads).unwrap();
        let source = downloads.join("incoming.pdf");
        std::fs::write(&source, "<html>PDF ではない</html>").unwrap();

        let mut db = Database::open(&data_dir.join("papr.db")).unwrap();
        let error = import_pdf(
            &mut db,
            &data_dir,
            &downloads,
            &source,
            &ManualTarget::Remote(remote("ArXiv:1", "T")),
        )
        .expect_err("PDF で無いと失敗する");
        assert!(matches!(error, super::ManualImportError::NotPdf(_)));
        assert!(source.exists(), "元のファイルは動かさない");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
