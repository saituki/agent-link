//! 前処理の束（sai-prep bundle）の取り込み。
//!
//! 各自の Mac の sai-prep が作った zip を展開したフォルダから、PDF・docling の
//! md/json/図・翻訳 PDF を取り込み、サーバ側では docling も pdf2zh も呼ばずに
//! 図・節・引用の表を作る。zip の展開は Web が行い、ここは展開済みのフォルダを受ける。
//!
//! 束の中身は共有契約 C12（`manifest.json` の形）に従う。`manifest.json` の
//! `target` は「Library にある論文（`paper_id`）」か「BibTeX 1 件（新しく作る）」の
//! どちらか。ファイルの相対パスは `dir` の中に収まっていなければならない。
//!
//! どの段で失敗しても、それまでに写したファイルは残してよい（キャッシュとして
//! 再利用される）。ただし PDF を取り込む前に失敗したら何も書かない。

use std::path::{Component, Path, PathBuf};

use chrono::Utc;
use rusqlite::Connection;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::citation::db::ExtractRow;
use crate::citation::{self, db as citation_db, docling};
use crate::figures::{self, FigureExtractRow};
use crate::manual_import::{self, ManualTarget};
use crate::{bibtex, paths, pipeline, sections, Database};

/// 前処理の束を取り込んだ結果。
#[derive(Debug, Clone, serde::Serialize)]
pub struct BundleResult {
    /// 登録・更新した論文の `papers.id`。
    pub paper_id: i64,
    /// 論文の題名。
    pub title: String,
    /// 保存した図の数。
    pub figures: usize,
    /// 作った節の数。
    pub sections: i64,
    /// 作った引用文献の数。
    pub refs: usize,
    /// 翻訳 PDF を取り込んだか。
    pub translated: bool,
}

/// 前処理の束の取り込みの失敗。
#[derive(Debug, Error)]
pub enum BundleError {
    /// ファイルの読み書きの失敗。
    #[error("ファイルの読み書きに失敗しました: {0}")]
    Io(#[from] std::io::Error),
    /// `manifest.json` の形が違う（終了コード 2 の対象）。
    #[error("manifest.json の形が違います: {0}")]
    Manifest(String),
    /// 対象の論文が見つからない（終了コード 3 の対象）。
    #[error("{0}")]
    NotFound(String),
    /// 対象の論文に既に PDF がある（終了コード 4 の対象）。
    #[error("{0}")]
    PdfExists(String),
    /// PDF の取り込みの失敗。
    #[error("PDF を取り込めませんでした: {0}")]
    Import(#[from] manual_import::ManualImportError),
    /// 図の保存の失敗。
    #[error("図を保存できませんでした: {0}")]
    Figures(#[from] figures::FigureError),
    /// 引用の表の保存の失敗。
    #[error("引用の表を保存できませんでした: {0}")]
    Citation(#[from] citation::CitationDbError),
    /// 節の表の保存の失敗。
    #[error("節の表を保存できませんでした: {0}")]
    Sections(#[from] sections::SectionsError),
    /// 翻訳の記録の失敗。
    #[error("翻訳を記録できませんでした: {0}")]
    Translation(String),
    /// データベースの失敗。
    #[error("データベースのエラー: {0}")]
    Database(#[from] crate::database::DatabaseError),
}

impl BundleError {
    /// CLI の終了コード（2 = 入力の誤り・3 = 見つからない・4 = 状態の衝突・1 = その他）。
    #[must_use]
    pub fn code(&self) -> i32 {
        match self {
            BundleError::Manifest(_) => 2,
            BundleError::NotFound(_) => 3,
            BundleError::PdfExists(_) => 4,
            _ => 1,
        }
    }
}

/// `manifest.json` の形（共有契約 C12）。
#[derive(Debug, Deserialize)]
struct Manifest {
    /// 常に `sai-prep-bundle`。
    format: String,
    /// 常に 1。
    version: u32,
    /// 対象（`paper_id` か `bibtex` のどちらか）。
    target: BundleTarget,
    /// PDF の相対パス。
    pdf: String,
    /// PDF の SHA-256（64 桁の 16 進）。
    pdf_sha256: String,
    /// docling の出力の相対パス。
    docling: DoclingPaths,
    /// 翻訳（無ければ null）。
    translation: Option<TranslationPaths>,
}

/// `manifest.json` の `target`。
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum BundleTarget {
    /// Library にある論文。
    PaperId {
        /// `papers.id`。
        paper_id: i64,
    },
    /// BibTeX 1 件の文字列。
    Bibtex {
        /// BibTeX の本文。
        bibtex: String,
    },
}

/// `manifest.json` の `docling`。
#[derive(Debug, Deserialize)]
struct DoclingPaths {
    /// docling の Markdown の相対パス。
    md: String,
    /// docling の JSON の相対パス。
    json: String,
    /// docling の出力フォルダ（画像 URI の基準）の相対パス。
    dir: String,
}

/// `manifest.json` の `translation`。
#[derive(Debug, Deserialize)]
struct TranslationPaths {
    /// 翻訳 PDF（本文のみ）の相対パス。
    mono: String,
    /// 対訳 PDF の相対パス。
    dual: String,
    /// 翻訳に使ったサービス名。
    service: String,
}

/// 前処理の束（展開済みフォルダ）を取り込む。
///
/// 手順は共有契約 C12 に従う（PDF の取り込み → md の配置 → 図 → 引用 → 節 → 翻訳）。
/// サーバ側では docling も pdf2zh も呼ばない。
///
/// `data_dir` は `.sai/paper/`（`db` を開いたフォルダ）。`db_file` は
/// `data_dir/papr.db` として導く（[`pipeline::register_translation`] が使う）。
///
/// # Errors
///
/// `manifest.json` の誤り（[`BundleError::Manifest`]）・対象なし
/// （[`BundleError::NotFound`]）・PDF が既にある（[`BundleError::PdfExists`]）など。
pub fn import_bundle(
    db: &mut Database,
    data_dir: &Path,
    dir: &Path,
) -> Result<BundleResult, BundleError> {
    // 1. manifest.json を読み、C12 の形か確かめる。
    let manifest = read_manifest(dir)?;
    let pdf_src = resolve_relative(dir, &manifest.pdf)?;
    require_file(&pdf_src, "PDF")?;
    let md_src = resolve_relative(dir, &manifest.docling.md)?;
    require_file(&md_src, "docling の Markdown")?;
    let json_src = resolve_relative(dir, &manifest.docling.json)?;
    require_file(&json_src, "docling の JSON")?;
    let docling_dir = resolve_relative(dir, &manifest.docling.dir)?;
    if !docling_dir.is_dir() {
        return Err(BundleError::Manifest(format!(
            "docling のフォルダがありません: {}",
            manifest.docling.dir
        )));
    }
    let actual_sha = sha256_file(&pdf_src)?;
    if !actual_sha.eq_ignore_ascii_case(manifest.pdf_sha256.trim()) {
        return Err(BundleError::Manifest(
            "PDF の SHA-256 が manifest.json と一致しません".to_string(),
        ));
    }
    // 翻訳があれば、そのファイルも先に確かめる（PDF を取り込む前に失敗させる）。
    let translation = match &manifest.translation {
        Some(translation) => {
            let mono = resolve_relative(dir, &translation.mono)?;
            let dual = resolve_relative(dir, &translation.dual)?;
            require_file(&mono, "翻訳 PDF（mono）")?;
            require_file(&dual, "翻訳 PDF（dual）")?;
            if translation.service.trim().is_empty() {
                return Err(BundleError::Manifest(
                    "translation.service が空です".to_string(),
                ));
            }
            Some((mono, dual, translation.service.clone()))
        }
        None => None,
    };

    // 2. 対象を決める。
    let (target, title) = resolve_target(db, data_dir, &manifest.target)?;

    // 3. PDF を取り込む。取り込み先の一時の置き場は使い捨てる。
    let tmp_dir = temp_bundle_dir();
    std::fs::create_dir_all(&tmp_dir)?;
    let imported = manual_import::import_pdf(db, data_dir, &tmp_dir, &pdf_src, &target);
    let _ = std::fs::remove_dir_all(&tmp_dir);
    let imported = imported?;
    let paper_id = imported.paper_id;
    let paper_dir = imported
        .path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| paths::paper_folder(data_dir, paper_id, &title));
    std::fs::create_dir_all(&paper_dir)?;

    // サーバ側で計算し直した PDF の情報（Mac の値は使わない）。
    let pdf_stored = paths::to_stored_path(data_dir, &imported.path);
    let pdf_mtime = file_mtime_secs(&imported.path)?;

    // 4. docling の md から画像を除いて、論文フォルダの extract.md に書く（base64 の画像を残さない）。
    let markdown = crate::chunking::strip_images(&std::fs::read_to_string(&md_src)?);
    let extract_dst = paper_dir.join(paths::paper_file_names::EXTRACT);
    std::fs::write(&extract_dst, &markdown)?;
    // PDF より新しくする（あとで docling を呼ばないため）。
    touch_newer_than(&extract_dst, &imported.path)?;
    let md_stored = paths::to_stored_path(data_dir, &extract_dst);

    // 5. 図を保存する。
    let key = figures::figure_key_for_paper(db.connection(), paper_id)?;
    let docling_figures = docling::read_docling_figures(&json_src, &docling_dir)
        .map_err(|e| BundleError::Figures(figures::FigureError::Docling(e.to_string())))?;
    let figure_count = figures::store_figures(
        db.connection(),
        &key,
        Some(paper_id),
        &markdown,
        &docling_figures,
        &paper_dir,
        data_dir,
    )?;
    figures::upsert_figure_extract(
        db.connection(),
        &FigureExtractRow {
            paper_id: key.clone(),
            papers_id: Some(paper_id),
            status: if figure_count == 0 { "empty" } else { "ready" }.to_string(),
            pdf_path: Some(pdf_stored.clone()),
            pdf_mtime: Some(pdf_mtime.clone()),
            n_figures: Some(figure_count as i64),
            error: None,
            extracted_at: Some(Utc::now().to_rfc3339()),
        },
    )?;

    // 6. 引用の表を作る（docling も S2 も呼ばない。md から Rust だけで組み立てる）。
    let refs = build_citations(
        db.connection(),
        paper_id,
        &key,
        &markdown,
        &pdf_stored,
        &pdf_mtime,
        &md_stored,
    )?;

    // 7. 節の表を作る（extract.md から。docling 不要）。
    sections::ensure(db.connection(), data_dir, paper_id)?;
    let section_count = sections::count_sections(db.connection(), paper_id)?;

    // 8. 翻訳 PDF を写して記録する。
    let translated = match &translation {
        Some((mono, dual, service)) => {
            std::fs::copy(mono, paper_dir.join(paths::paper_file_names::PDF_JA))?;
            std::fs::copy(dual, paper_dir.join(paths::paper_file_names::PDF_DUAL))?;
            let db_file = data_dir.join("papr.db");
            pipeline::register_translation(
                &db_file,
                paper_id,
                &paper_dir.join(paths::paper_file_names::PDF_JA),
                service,
            )
            .map_err(BundleError::Translation)?;
            true
        }
        None => false,
    };

    let title = db
        .library_paper_by_id(paper_id)
        .ok()
        .flatten()
        .map(|paper| paper.title)
        .unwrap_or(title);
    Ok(BundleResult {
        paper_id,
        title,
        figures: figure_count,
        sections: section_count,
        refs,
        translated,
    })
}

/// `dir/manifest.json` を読んで C12 の形か確かめる。
fn read_manifest(dir: &Path) -> Result<Manifest, BundleError> {
    let path = dir.join("manifest.json");
    let text = std::fs::read_to_string(&path).map_err(|e| {
        BundleError::Manifest(format!("manifest.json を読めません: {}（{e}）", path.display()))
    })?;
    let manifest: Manifest = serde_json::from_str(&text)
        .map_err(|e| BundleError::Manifest(format!("manifest.json を解釈できません: {e}")))?;
    if manifest.format != "sai-prep-bundle" {
        return Err(BundleError::Manifest(format!(
            "format が sai-prep-bundle ではありません: {}",
            manifest.format
        )));
    }
    if manifest.version != 1 {
        return Err(BundleError::Manifest(format!(
            "version が 1 ではありません: {}",
            manifest.version
        )));
    }
    Ok(manifest)
}

/// `target` から取り込み先（`ManualTarget`）と題名を決める。
fn resolve_target(
    db: &Database,
    data_dir: &Path,
    target: &BundleTarget,
) -> Result<(ManualTarget, String), BundleError> {
    match target {
        BundleTarget::PaperId { paper_id } => {
            let Some(paper) = db.library_paper_by_id(*paper_id)? else {
                return Err(BundleError::NotFound("論文が見つかりません".to_string()));
            };
            // 既に PDF がある論文には付けない（上書きしない）。
            let existing = paths::paper_folder(data_dir, *paper_id, &paper.title)
                .join(paths::paper_file_names::PDF);
            if existing.exists() {
                return Err(BundleError::PdfExists(
                    "この論文にはもう PDF があります".to_string(),
                ));
            }
            Ok((
                ManualTarget::Local {
                    paper_id: *paper_id,
                    title: paper.title.clone(),
                },
                paper.title,
            ))
        }
        BundleTarget::Bibtex { bibtex: source } => {
            let remote = bibtex::parse_bibtex_str(source)
                .map_err(|e| BundleError::Manifest(format!("BibTeX を読めません: {e}")))?
                .into_iter()
                .next()
                .ok_or_else(|| BundleError::NotFound("BibTeX に論文がありません".to_string()))?;
            let title = remote.title.clone();
            Ok((ManualTarget::Remote(remote), title))
        }
    }
}

/// md から引用文献と引用文の表を作り、`paper_extracts` を ready にする。
///
/// `ensure_extract` と同じ公開部品（[`citation::refs_from_markdown`]・
/// [`citation::cited_sentences`]）で組み立てる。S2 の参考文献は呼ばない
/// （サーバでは重い処理を動かさない）。作った引用文献の数を返す。
fn build_citations(
    conn: &Connection,
    paper_id: i64,
    key: &str,
    markdown: &str,
    pdf_stored: &str,
    pdf_mtime: &str,
    md_stored: &str,
) -> Result<usize, BundleError> {
    let doi = doi_for_journal(conn, paper_id, key);
    let journal = citation::parsers::detect_journal(doi.as_deref());
    let (ref_rows, numbered) = citation::refs_from_markdown(markdown, journal, key);
    let sentences = citation::cited_sentences(markdown, &ref_rows, numbered);
    citation_db::replace_refs(conn, key, Some(paper_id), &ref_rows)?;
    citation_db::replace_sentences(conn, key, Some(paper_id), &sentences)?;
    citation_db::upsert_extract(
        conn,
        &ExtractRow {
            paper_id: key.to_string(),
            status: "ready".to_string(),
            style: Some("Generic".to_string()),
            pdf_path: Some(pdf_stored.to_string()),
            pdf_mtime: Some(pdf_mtime.to_string()),
            n_refs: Some(ref_rows.len() as i64),
            n_sentences: Some(sentences.len() as i64),
            md_path: Some(md_stored.to_string()),
            error: None,
            extracted_at: Some(Utc::now().to_rfc3339()),
            papers_id: Some(paper_id),
        },
    )?;
    Ok(ref_rows.len())
}

/// ジャーナル判定に使う DOI を決める（キーが DOI 形式ならそれを、無ければ DB から）。
fn doi_for_journal(conn: &Connection, paper_id: i64, key: &str) -> Option<String> {
    if let Some(rest) = key.strip_prefix("DOI:") {
        return Some(rest.to_string());
    }
    conn.query_row("SELECT doi FROM papers WHERE id = ?1", [paper_id], |row| {
        row.get::<_, Option<String>>(0)
    })
    .ok()
    .flatten()
}

/// 相対パスを `dir` の中に収まる形で解決する。
///
/// 絶対パス・`..`・空文字は [`BundleError::Manifest`] にする。
fn resolve_relative(dir: &Path, rel: &str) -> Result<PathBuf, BundleError> {
    let rel = rel.trim();
    if rel.is_empty() {
        return Err(BundleError::Manifest("空のパスがあります".to_string()));
    }
    let path = Path::new(rel);
    if path.is_absolute() {
        return Err(BundleError::Manifest(format!(
            "絶対パスは使えません: {rel}"
        )));
    }
    let mut out = dir.to_path_buf();
    for component in path.components() {
        match component {
            Component::Normal(part) => out.push(part),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(BundleError::Manifest(format!(
                    "`..` は使えません: {rel}"
                )));
            }
        }
    }
    Ok(out)
}

/// ファイルの実在を確かめる。
fn require_file(path: &Path, label: &str) -> Result<(), BundleError> {
    if path.is_file() {
        Ok(())
    } else {
        Err(BundleError::Manifest(format!(
            "{label} がありません: {}",
            path.display()
        )))
    }
}

/// ファイルの SHA-256（小文字の 16 進）を返す。
pub fn sha256_file(path: &Path) -> Result<String, BundleError> {
    let bytes = std::fs::read(path)?;
    Ok(format!("{:x}", Sha256::digest(&bytes)))
}

/// ファイルの更新時刻（エポック秒）を返す。
pub(crate) fn file_mtime_secs(path: &Path) -> Result<String, BundleError> {
    Ok(std::fs::metadata(path)?
        .modified()?
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| BundleError::Manifest("PDF の更新時刻が読めません".to_string()))?
        .as_secs()
        .to_string())
}

/// `target` を `reference` より新しくする（`reusable_markdown` の条件を満たす）。
pub(crate) fn touch_newer_than(target: &Path, reference: &Path) -> Result<(), BundleError> {
    let now = std::time::SystemTime::now();
    let reference_mtime = std::fs::metadata(reference).and_then(|m| m.modified()).ok();
    let modified = match reference_mtime {
        Some(reference_mtime) if now <= reference_mtime => {
            reference_mtime + std::time::Duration::from_secs(1)
        }
        _ => now,
    };
    let file = std::fs::OpenOptions::new().write(true).open(target)?;
    file.set_times(std::fs::FileTimes::new().set_modified(modified))?;
    Ok(())
}

/// `extract.md` から画像を除いた結果の 1 行（`sai-paper paper strip-images`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct StripRow {
    /// 論文 ID（papers.id）。
    pub id: i64,
    /// 除く前の大きさ（バイト）。
    pub before: u64,
    /// 除いた後の大きさ（バイト）。
    pub after: u64,
}

/// 既にある論文の `extract.md` から画像を除く。`extract.md` が無い論文は飛ばす。
/// 中身が変わったものだけ書き直し、更新時刻は元のまま残す（PDF より新しいことを保つ）。
/// 節の表は作り直さず、sha だけ付け替える（要約を消さないため）。
///
/// # Errors
///
/// DB・ファイルの失敗。
pub fn strip_extract_images(
    conn: &Connection,
    data_dir: &Path,
    ids: &[i64],
) -> Result<Vec<StripRow>, BundleError> {
    let mut rows = Vec::new();
    for &id in ids {
        let Some(path) = citation::resolve_extract_md(conn, Some(id), None, data_dir) else {
            continue;
        };
        let text = std::fs::read_to_string(&path)?;
        let stripped = crate::chunking::strip_images(&text);
        if stripped != text {
            let old_sha = sha256_file(&path)?;
            let mtime = std::fs::metadata(&path)?.modified()?;
            let tmp = path.with_extension("md.tmp");
            std::fs::write(&tmp, &stripped)?;
            std::fs::rename(&tmp, &path)?;
            std::fs::OpenOptions::new()
                .write(true)
                .open(&path)?
                .set_times(std::fs::FileTimes::new().set_modified(mtime))?;
            sections::rebind_extract_sha(conn, id, &old_sha, &sha256_file(&path)?)?;
        }
        rows.push(StripRow {
            id,
            before: text.len() as u64,
            after: stripped.len() as u64,
        });
    }
    Ok(rows)
}

/// 取り込みで使う一時の置き場（プロセスと時刻で衝突しない名前）。
fn temp_bundle_dir() -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    std::env::temp_dir().join(format!("sai-bundle-{}-{nanos}", std::process::id()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// テストごとの一時フォルダ（`Drop` で消す）。
    struct Temp(PathBuf);

    impl Temp {
        fn new(tag: &str) -> Self {
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or_default();
            let dir = std::env::temp_dir().join(format!(
                "sai-bundle-unit-{tag}-{}-{nanos}",
                std::process::id()
            ));
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }
    }

    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// 文中と行全体の data URI 画像を含む docling の md。
    fn md_with_base64() -> String {
        let big = "QUJD".repeat(2000);
        format!(
            "## Strip paper\n\nSee the figure ![f](data:image/png;base64,{big}) here.\n\n\
             ![Image](data:image/png;base64,{big})\n\n## 1 Intro\n\nBody text of the intro.\n"
        )
    }

    /// 束の取り込みで `extract.md` に `data:image` が残らない（画像の参照は本文の周りだけ）。
    #[test]
    fn import_bundle_keeps_no_data_uri_in_extract() {
        let temp = Temp::new("import");
        let data_dir = temp.0.join("data");
        std::fs::create_dir_all(&data_dir).unwrap();
        let mut db = Database::open(&data_dir.join("papr.db")).unwrap();
        db.connection()
            .execute("INSERT INTO papers (title, doi) VALUES ('Strip paper', '10.1/strip')", [])
            .unwrap();
        let paper_id = db.connection().last_insert_rowid();

        let bundle = temp.0.join("bundle");
        std::fs::create_dir_all(bundle.join("docling")).unwrap();
        let pdf = b"%PDF-1.4 strip\n%%EOF\n".to_vec();
        std::fs::write(bundle.join("paper.pdf"), &pdf).unwrap();
        std::fs::write(bundle.join("docling/paper.md"), md_with_base64()).unwrap();
        std::fs::write(bundle.join("docling/paper.json"), r#"{"texts": [], "pictures": []}"#).unwrap();
        let manifest = serde_json::json!({
            "format": "sai-prep-bundle",
            "version": 1,
            "target": {"paper_id": paper_id},
            "pdf": "paper.pdf",
            "pdf_sha256": format!("{:x}", Sha256::digest(&pdf)),
            "docling": {"md": "docling/paper.md", "json": "docling/paper.json", "dir": "docling"},
            "translation": null,
            "tools": {"sai_prep": "0.1.0"},
            "created_at": "2026-10-07T12:00:00+09:00",
        });
        std::fs::write(bundle.join("manifest.json"), manifest.to_string()).unwrap();

        import_bundle(&mut db, &data_dir, &bundle).expect("取り込める");
        let extract = paths::paper_folder(&data_dir, paper_id, "Strip paper")
            .join(paths::paper_file_names::EXTRACT);
        let text = std::fs::read_to_string(&extract).unwrap();
        assert!(!text.contains("data:image"), "{}", text.len());
        assert!(text.contains("See the figure"));
        assert!(text.contains("Body text of the intro."));
    }

    /// 既にある論文の画像を除く。2 回目は変化 0。要約（節の表）は消えない。
    #[test]
    fn strip_extract_images_shrinks_once_and_keeps_summaries() {
        let temp = Temp::new("strip");
        let db = Database::in_memory_with_data_dir(&temp.0).unwrap();
        let conn = db.connection();
        let id = db.insert_paper("Old paper", None).unwrap();
        let md_path = temp.0.join("extract.md");
        std::fs::write(&md_path, md_with_base64()).unwrap();
        conn.execute(
            "INSERT INTO paper_extracts (paper_id, status, md_path, papers_id) VALUES ('k', 'ready', ?1, ?2)",
            rusqlite::params![md_path.to_string_lossy(), id],
        )
        .unwrap();

        sections::ensure(conn, &temp.0, id).unwrap();
        conn.execute("UPDATE paper_sections SET summary = 'keep' WHERE paper_id = ?1 AND sec_no = 1", [id])
            .unwrap();

        let first = strip_extract_images(conn, &temp.0, &[id, 9999]).unwrap();
        assert_eq!(first.len(), 1, "extract.md の無い論文は飛ばす");
        assert!(first[0].after < first[0].before);
        let text = std::fs::read_to_string(&md_path).unwrap();
        assert!(!text.contains("data:image"));
        assert!(text.contains("Body text of the intro."));

        // 表の sha が付け替わっているので、読んでも作り直さず要約も残る。
        assert_eq!(sections::ensure(conn, &temp.0, id).unwrap(), sections::BuildState::Fresh);
        let summary: Option<String> = conn
            .query_row(
                "SELECT summary FROM paper_sections WHERE paper_id = ?1 AND sec_no = 1",
                [id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(summary.as_deref(), Some("keep"));

        let second = strip_extract_images(conn, &temp.0, &[id]).unwrap();
        assert_eq!(second[0].before, second[0].after, "2 回目は変化 0");
    }
}
