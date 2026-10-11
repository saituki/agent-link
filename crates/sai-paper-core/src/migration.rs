//! 論文の移植（別のプロジェクトへ docling や埋め込みをやり直さずに移す）。
//!
//! [`export`] は移植元で「移植の束」のフォルダを作り、[`import`] は移植先で束を取り込む。
//! 束の形は共有契約 C-M1（`manifest.json`・論文の表だけを入れた `papr.db`・
//! `papers/<移植元のフォルダ名>/`）に従う。
//!
//! 取り込みは両方の DB にある列だけを写す（列の積）。論文の id は移植先で振り直し、
//! フォルダの名前と保存するパスも移植先のものに直す。派生の表（節・引用・図・翻訳）は
//! 作り直さず写す（節の要約と引用の照合の結果を失わないため）。

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use rusqlite::types::Value;
use rusqlite::{Connection, OpenFlags, OptionalExtension, params, params_from_iter};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::bundle;
use crate::{Database, figures, paths};

/// 束の `manifest.json` の `format`。
const FORMAT: &str = "sai-paper-migration";
/// 束の `manifest.json` の `version`。
const VERSION: i64 = 1;

/// 束の `papr.db` に写す表と条件（共有契約 C-M1）。
///
/// 条件の `{ids}` は移植する論文の id の並び（数だけなので SQL に直接入れる）。
/// 条件の `src.` は移植元の DB（ATTACH した別名）。
const TABLES: &[(&str, &str)] = &[
    ("papers", "id IN ({ids})"),
    (
        "authors",
        "id IN (SELECT author_id FROM src.paper_authors WHERE paper_id IN ({ids}))",
    ),
    ("paper_authors", "paper_id IN ({ids})"),
    (
        "tags",
        "id IN (SELECT tag_id FROM src.paper_tags WHERE paper_id IN ({ids}))",
    ),
    ("paper_tags", "paper_id IN ({ids})"),
    ("chunks", "paper_id IN ({ids})"),
    (
        "chunk_embeddings",
        "chunk_id IN (SELECT id FROM src.chunks WHERE paper_id IN ({ids}))",
    ),
    ("embedding_models", "1"),
    ("paper_sections", "paper_id IN ({ids})"),
    ("paper_passages", "paper_id IN ({ids})"),
    ("paper_section_builds", "paper_id IN ({ids})"),
    ("paper_extracts", "papers_id IN ({ids})"),
    ("paper_figure_extracts", "papers_id IN ({ids})"),
    ("paper_figures", "papers_id IN ({ids})"),
    ("paper_refs", "papers_id IN ({ids})"),
    ("paper_sentences", "papers_id IN ({ids})"),
    ("paper_translations", "papers_id IN ({ids})"),
];

/// 移植の失敗（CLI の終了コード: 2 = 入力の誤り・3 = 見つからない・1 = その他）。
#[derive(Debug, Error)]
pub enum MigrationError {
    /// 入力の誤り（終了コード 2）。
    #[error("{0}")]
    Input(String),
    /// 見つからない（終了コード 3）。
    #[error("{0}")]
    NotFound(String),
    /// 途中で失敗した（終了コード 1）。
    #[error("{0}")]
    Failed(String),
    /// ファイルの読み書きの失敗。
    #[error("ファイルの読み書きに失敗しました: {0}")]
    Io(#[from] std::io::Error),
    /// データベースの失敗。
    #[error("データベースのエラー: {0}")]
    Database(#[from] rusqlite::Error),
}

impl MigrationError {
    /// CLI の終了コード（2 = 入力の誤り・3 = 見つからない・1 = その他）。
    #[must_use]
    pub fn code(&self) -> i32 {
        match self {
            MigrationError::Input(_) => 2,
            MigrationError::NotFound(_) => 3,
            _ => 1,
        }
    }
}

/// 書き出しの結果。
#[derive(Debug, Clone, Serialize)]
pub struct ExportResult {
    /// 書き出したフォルダ。
    pub dir: String,
    /// 書き出した論文の `papers.id`（移植元）。
    pub papers: Vec<i64>,
    /// 移植元に無かった `papers.id`。
    pub missing: Vec<i64>,
}

/// 取り込みの結果（論文ごと）。
#[derive(Debug, Clone, Default, Serialize)]
pub struct ImportReport {
    /// 新しく作った、または PDF 待ちの論文を補った論文。
    pub imported: Vec<ImportedItem>,
    /// 移植先に PDF が既にあるため飛ばした論文。
    pub skipped: Vec<SkippedItem>,
    /// 失敗した論文（他の論文の取り込みは続ける）。
    pub failed: Vec<FailedItem>,
}

/// 取り込んだ 1 本。`mode` は `new`（新しく作った）か `filled`（PDF 待ちを補った）。
#[derive(Debug, Clone, Serialize)]
pub struct ImportedItem {
    /// 移植元の `papers.id`。
    pub source_id: i64,
    /// 移植先の `papers.id`。
    pub paper_id: i64,
    /// 移植先の題。
    pub title: String,
    /// `new` か `filled`。
    pub mode: &'static str,
}

/// 飛ばした 1 本。
#[derive(Debug, Clone, Serialize)]
pub struct SkippedItem {
    /// 移植元の `papers.id`。
    pub source_id: i64,
    /// 移植先の `papers.id`。
    pub paper_id: i64,
    /// 移植先の題。
    pub title: String,
    /// 飛ばした理由。
    pub reason: &'static str,
}

/// 失敗した 1 本。
#[derive(Debug, Clone, Serialize)]
pub struct FailedItem {
    /// 移植元の `papers.id`。
    pub source_id: i64,
    /// 移植元の題（分からなければ空）。
    pub title: String,
    /// 失敗の理由。
    pub error: String,
}

/// `manifest.json` の形（共有契約 C-M1）。
#[derive(Debug, Deserialize)]
struct Manifest {
    /// 常に `sai-paper-migration`。
    format: String,
    /// 常に 1。
    version: i64,
    /// 束に入った論文の `papers.id`（移植元）。
    papers: Vec<i64>,
}

/// 論文を移植の束のフォルダ `out` に書き出す。
///
/// `db_file` は移植元の `papr.db`、`data_dir` は移植元の `.sai/paper/`。`out` は無い
/// フォルダであること（作るのはこの関数）。失敗したら `out` を消す。
///
/// # Errors
///
/// `ids` が空・`out` が既にある（[`MigrationError::Input`]）、移植元に 1 つも無い
/// （[`MigrationError::NotFound`]）、書き出しの失敗（[`MigrationError::Failed`] など）。
pub fn export(
    db_file: &Path,
    data_dir: &Path,
    ids: &[i64],
    out: &Path,
) -> Result<ExportResult, MigrationError> {
    if ids.is_empty() {
        return Err(MigrationError::Input(
            "論文の id が 1 つもありません".to_string(),
        ));
    }
    if out.exists() {
        return Err(MigrationError::Input(format!(
            "書き出し先が既にあります: {}",
            out.display()
        )));
    }
    if !db_file.is_file() {
        return Err(MigrationError::NotFound(format!(
            "移植元の DB がありません: {}",
            db_file.display()
        )));
    }
    let source = Connection::open_with_flags(db_file, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let mut found = Vec::new();
    let mut missing = Vec::new();
    for &id in ids {
        let title: Option<String> = source
            .query_row("SELECT title FROM papers WHERE id = ?1", [id], |row| {
                row.get(0)
            })
            .optional()?;
        match title {
            Some(title) => found.push((id, title)),
            None => missing.push(id),
        }
    }
    drop(source);
    if found.is_empty() {
        return Err(MigrationError::NotFound(
            "移植元に論文が 1 つもありません".to_string(),
        ));
    }

    fs::create_dir_all(out)?;
    if let Err(error) = write_export(db_file, data_dir, &found, out) {
        let _ = fs::remove_dir_all(out);
        return Err(error);
    }
    Ok(ExportResult {
        dir: out.to_string_lossy().into_owned(),
        papers: found.iter().map(|(id, _)| *id).collect(),
        missing,
    })
}

/// 束の中身（`papr.db`・`papers/`・`manifest.json`）を書く。
fn write_export(
    db_file: &Path,
    data_dir: &Path,
    found: &[(i64, String)],
    out: &Path,
) -> Result<(), MigrationError> {
    let conn = Connection::open(out.join("papr.db"))?;
    conn.execute(
        "ATTACH DATABASE ?1 AS src",
        [db_file.to_string_lossy().into_owned()],
    )?;
    let ids = found
        .iter()
        .map(|(id, _)| id.to_string())
        .collect::<Vec<_>>()
        .join(",");
    for (table, cond) in TABLES {
        if table_columns(&conn, "src", table)?.is_empty() {
            continue;
        }
        let cond = cond.replace("{ids}", &ids);
        conn.execute(
            &format!("CREATE TABLE {table} AS SELECT * FROM src.{table} WHERE {cond}"),
            [],
        )?;
    }
    conn.execute_batch("DETACH DATABASE src")?;
    drop(conn);

    let papers_out = out.join("papers");
    for (id, title) in found {
        let folder = paths::paper_folder(data_dir, *id, title);
        let Some(name) = folder.file_name() else {
            continue;
        };
        if folder.is_dir() {
            copy_dir_all(&folder, &papers_out.join(name))?;
        }
    }

    let manifest = serde_json::json!({
        "format": FORMAT,
        "version": VERSION,
        "papers": found.iter().map(|(id, _)| *id).collect::<Vec<_>>(),
    });
    let text = serde_json::to_string_pretty(&manifest)
        .map_err(|e| MigrationError::Failed(e.to_string()))?;
    fs::write(out.join("manifest.json"), text)?;
    Ok(())
}

/// 移植の束のフォルダ `dir` を、移植先の DB `db` と `data_dir` へ取り込む。
///
/// 論文ごとに 1 つの transaction。失敗した論文は [`ImportReport::failed`] に入れて
/// 次へ進む。`dir` は消さない。
///
/// # Errors
///
/// `manifest.json` の形が違う・`papr.db` が無い（[`MigrationError::Input`]）、
/// 束の DB を開けない（[`MigrationError::Database`]）。
pub fn import(
    db: &mut Database,
    data_dir: &Path,
    dir: &Path,
) -> Result<ImportReport, MigrationError> {
    let papers = read_manifest(dir)?;
    let bundle_db = dir.join("papr.db");
    if !bundle_db.is_file() {
        return Err(MigrationError::Input(format!(
            "束に papr.db がありません: {}",
            dir.display()
        )));
    }
    let conn = db.connection();
    conn.execute(
        "ATTACH DATABASE ?1 AS src",
        [bundle_db.to_string_lossy().into_owned()],
    )?;
    let report = import_all(conn, data_dir, dir, &papers);
    let _ = conn.execute_batch("DETACH DATABASE src");
    report
}

/// `dir/manifest.json` を読み、C-M1 の形か確かめて論文の id を返す。
fn read_manifest(dir: &Path) -> Result<Vec<i64>, MigrationError> {
    let path = dir.join("manifest.json");
    let text = fs::read_to_string(&path).map_err(|e| {
        MigrationError::Input(format!(
            "manifest.json を読めません: {}（{e}）",
            path.display()
        ))
    })?;
    let manifest: Manifest = serde_json::from_str(&text)
        .map_err(|e| MigrationError::Input(format!("manifest.json の形が違います: {e}")))?;
    if manifest.format != FORMAT || manifest.version != VERSION {
        return Err(MigrationError::Input(format!(
            "移植の束ではありません（format: {} / version: {}）",
            manifest.format, manifest.version
        )));
    }
    if manifest.papers.iter().any(|id| *id <= 0) {
        return Err(MigrationError::Input(
            "papers に正の整数でない id があります".to_string(),
        ));
    }
    Ok(manifest.papers)
}

/// 束の論文を 1 本ずつ取り込む。
fn import_all(
    conn: &Connection,
    data_dir: &Path,
    dir: &Path,
    papers: &[i64],
) -> Result<ImportReport, MigrationError> {
    let mut report = ImportReport::default();
    for &source_id in papers {
        match import_one(conn, data_dir, dir, source_id) {
            Ok(Outcome::Imported(item)) => report.imported.push(item),
            Ok(Outcome::Skipped(item)) => report.skipped.push(item),
            Err(error) => report.failed.push(FailedItem {
                source_id,
                title: src_title(conn, source_id)
                    .ok()
                    .flatten()
                    .unwrap_or_default(),
                error: error.to_string(),
            }),
        }
    }
    Ok(report)
}

/// 1 本の取り込みの結果。
enum Outcome {
    /// 新しく作った、または補った。
    Imported(ImportedItem),
    /// 飛ばした。
    Skipped(SkippedItem),
}

/// 1 本を 1 つの transaction で取り込む。失敗したら書いた DB 行と新しく作ったフォルダを戻す。
fn import_one(
    conn: &Connection,
    data_dir: &Path,
    dir: &Path,
    source_id: i64,
) -> Result<Outcome, MigrationError> {
    let tx = conn.unchecked_transaction()?;
    let mut created = None;
    match import_one_in(&tx, data_dir, dir, source_id, &mut created) {
        Ok(outcome) => {
            tx.commit()?;
            Ok(outcome)
        }
        Err(error) => {
            if let Some(folder) = created {
                let _ = fs::remove_dir_all(folder);
            }
            Err(error)
        }
    }
}

/// [`import_one`] の本体。`created` には新しく作ったフォルダを入れる（失敗時に消す）。
fn import_one_in(
    conn: &Connection,
    data_dir: &Path,
    dir: &Path,
    source_id: i64,
    created: &mut Option<PathBuf>,
) -> Result<Outcome, MigrationError> {
    let source_title = src_title(conn, source_id)?
        .ok_or_else(|| MigrationError::Failed("束の DB にこの論文がありません".to_string()))?;

    // (1) 重複を探す。PDF がある論文は飛ばし、PDF 待ちは補い、無ければ新しく作る。
    let keys = src_keys(conn, source_id)?;
    let (paper_id, mode) = match find_duplicate(conn, &keys)? {
        Some((id, title, pdf_path)) => {
            if pdf_path.is_some_and(|p| paths::resolve_stored_path(data_dir, &p).is_file()) {
                return Ok(Outcome::Skipped(SkippedItem {
                    source_id,
                    paper_id: id,
                    title,
                    reason: "既に PDF があります",
                }));
            }
            fill_paper(conn, source_id, id)?;
            clear_derived(conn, id)?;
            (id, "filled")
        }
        None => (insert_paper(conn, source_id)?, "new"),
    };
    let title = title_of(conn, paper_id)?;

    // (3) ファイル。束のフォルダを移植先の論文フォルダへ写す（既にあるファイルは上書きしない）。
    let src_folder = paths::paper_folder(dir, source_id, &source_title);
    let dst_folder = paths::paper_folder(data_dir, paper_id, &title);
    if !dst_folder.exists() {
        *created = Some(dst_folder.clone());
    }
    fs::create_dir_all(&dst_folder)?;
    if src_folder.is_dir() {
        copy_dir_all(&src_folder, &dst_folder)?;
    }
    let dst_pdf = dst_folder.join(paths::paper_file_names::PDF);
    let dst_md = dst_folder.join(paths::paper_file_names::EXTRACT);
    if dst_pdf.is_file() {
        // PDF より新しくする（あとで docling を呼ばないため）。
        if dst_md.is_file() {
            bundle::touch_newer_than(&dst_md, &dst_pdf)
                .map_err(|e| MigrationError::Failed(e.to_string()))?;
        }
        conn.execute(
            "UPDATE papers SET pdf_path = ?1 WHERE id = ?2",
            params![paths::to_stored_path(data_dir, &dst_pdf), paper_id],
        )?;
    }

    // (4)(5) 抽出キーと子の表。
    let key = figures::figure_key_for_paper(conn, paper_id)
        .map_err(|e| MigrationError::Failed(e.to_string()))?;
    let pdf_mtime = if dst_pdf.is_file() {
        Some(bundle::file_mtime_secs(&dst_pdf).map_err(|e| MigrationError::Failed(e.to_string()))?)
    } else {
        None
    };
    let mv = Move {
        conn,
        data_dir,
        source_id,
        paper_id,
        key,
        src_name: src_folder
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
        dst_folder,
        pdf_stored: paths::to_stored_path(data_dir, &dst_pdf),
        md_stored: paths::to_stored_path(data_dir, &dst_md),
        pdf_mtime,
    };
    mv.copy_children()?;

    Ok(Outcome::Imported(ImportedItem {
        source_id,
        paper_id,
        title: title_of(conn, paper_id)?,
        mode,
    }))
}

/// 1 本分の移す先（取り込みの途中で決まる値）。
struct Move<'a> {
    conn: &'a Connection,
    data_dir: &'a Path,
    /// 移植元の `papers.id`。
    source_id: i64,
    /// 移植先の `papers.id`。
    paper_id: i64,
    /// 移植先の抽出キー（`paper_extracts` などの `paper_id`）。
    key: String,
    /// 束の中での論文フォルダの名前（パスの書き換えに使う）。
    src_name: String,
    /// 移植先の論文フォルダ。
    dst_folder: PathBuf,
    /// 移植先の PDF の保存パス（data_dir からの相対形）。
    pdf_stored: String,
    /// 移植先の extract.md の保存パス（data_dir からの相対形）。
    md_stored: String,
    /// 移植先の PDF の更新時刻（PDF が無ければ None。その行は移植元の値のまま）。
    pdf_mtime: Option<String>,
}

impl Move<'_> {
    /// 子の表をすべて写す（作者・タグ・チャンク・埋め込み・節・図・引用・翻訳）。
    fn copy_children(&self) -> Result<(), MigrationError> {
        let t = self.paper_id;
        self.copy_links(
            "paper_authors",
            "authors",
            "author_id",
            &["openalex_id", "orcid", "name"],
        )?;
        self.copy_links("paper_tags", "tags", "tag_id", &["name"])?;
        let chunks = self.copy_chunks()?;
        self.copy_embeddings(&chunks)?;

        self.copy_simple("paper_sections", "paper_id", |cols, row| {
            put(cols, row, "paper_id", Value::Integer(t));
        })?;
        self.copy_simple("paper_passages", "paper_id", |cols, row| {
            put(cols, row, "paper_id", Value::Integer(t));
        })?;
        let extract = self.dst_folder.join(paths::paper_file_names::EXTRACT);
        self.copy_simple("paper_section_builds", "paper_id", |cols, row| {
            put(cols, row, "paper_id", Value::Integer(t));
            put(
                cols,
                row,
                "extract_path",
                Value::Text(extract.to_string_lossy().into_owned()),
            );
        })?;

        self.copy_simple("paper_extracts", "papers_id", |cols, row| {
            put(cols, row, "paper_id", Value::Text(self.key.clone()));
            put(cols, row, "papers_id", Value::Integer(t));
            put(cols, row, "pdf_path", Value::Text(self.pdf_stored.clone()));
            put(cols, row, "md_path", Value::Text(self.md_stored.clone()));
            if let Some(mtime) = &self.pdf_mtime {
                put(cols, row, "pdf_mtime", Value::Text(mtime.clone()));
            }
        })?;
        self.copy_simple("paper_figure_extracts", "papers_id", |cols, row| {
            put(cols, row, "paper_id", Value::Text(self.key.clone()));
            put(cols, row, "papers_id", Value::Integer(t));
            put(cols, row, "pdf_path", Value::Text(self.pdf_stored.clone()));
            if let Some(mtime) = &self.pdf_mtime {
                put(cols, row, "pdf_mtime", Value::Text(mtime.clone()));
            }
        })?;
        self.copy_simple("paper_figures", "papers_id", |cols, row| {
            put(cols, row, "paper_id", Value::Text(self.key.clone()));
            put(cols, row, "papers_id", Value::Integer(t));
            if let Some(Value::Text(path)) = value_at(cols, row, "image_path").cloned() {
                let new = self.to_dst_path(&path, paths::paper_file_names::FIGURES_DIR);
                put(cols, row, "image_path", Value::Text(new));
            }
        })?;
        self.copy_simple("paper_refs", "papers_id", |cols, row| {
            put(cols, row, "paper_id", Value::Text(self.key.clone()));
            put(cols, row, "papers_id", Value::Integer(t));
        })?;
        self.copy_simple("paper_sentences", "papers_id", |cols, row| {
            put(cols, row, "paper_id", Value::Text(self.key.clone()));
            put(cols, row, "papers_id", Value::Integer(t));
        })?;
        self.copy_simple("paper_translations", "papers_id", |cols, row| {
            put(cols, row, "paper_id", Value::Text(self.key.clone()));
            put(cols, row, "papers_id", Value::Integer(t));
            put(cols, row, "pdf_path", Value::Text(self.pdf_stored.clone()));
            if let Some(mtime) = &self.pdf_mtime {
                put(cols, row, "pdf_mtime", Value::Text(mtime.clone()));
            }
            if let Some(Value::Text(path)) = value_at(cols, row, "pdf_ja_path").cloned() {
                let new = self.to_dst_path(&path, "");
                put(cols, row, "pdf_ja_path", Value::Text(new));
            }
        })?;
        Ok(())
    }

    /// 作者・タグのように「論文と名前のある表の結びつき」を写す。
    ///
    /// 移植元の名前の表の行を移植先の行へ寄せる（`keys` の順に探し、無ければ作る）。
    fn copy_links(
        &self,
        link: &str,
        master: &str,
        col: &str,
        keys: &[&str],
    ) -> Result<(), MigrationError> {
        let link_cols = common_columns(self.conn, link)?;
        let master_cols = common_columns(self.conn, master)?;
        if link_cols.is_empty() || master_cols.is_empty() {
            return Ok(());
        }
        let links = select_rows(
            self.conn,
            link,
            &link_cols,
            "paper_id = ?1",
            &[Value::Integer(self.source_id)],
        )?;
        for mut row in links {
            let master_id = int_at(&link_cols, &row, col)?;
            let master_rows = select_rows(
                self.conn,
                master,
                &master_cols,
                "id = ?1",
                &[Value::Integer(master_id)],
            )?;
            let Some(master_row) = master_rows.first() else {
                continue;
            };
            let dst_id = map_named(self.conn, master, &master_cols, master_row, keys)?;
            put(
                &link_cols,
                &mut row,
                "paper_id",
                Value::Integer(self.paper_id),
            );
            put(&link_cols, &mut row, col, Value::Integer(dst_id));
            insert_row(self.conn, link, &link_cols, &row, "INSERT OR IGNORE", &[])?;
        }
        Ok(())
    }

    /// チャンクを写す。移植先で振り直した id の対応（移植元 → 移植先）を返す。
    fn copy_chunks(&self) -> Result<Vec<(i64, i64)>, MigrationError> {
        let cols = common_columns(self.conn, "chunks")?;
        let mut map = Vec::new();
        if cols.is_empty() {
            return Ok(map);
        }
        let rows = select_rows(
            self.conn,
            "chunks",
            &cols,
            "paper_id = ?1",
            &[Value::Integer(self.source_id)],
        )?;
        for mut row in rows {
            let old = int_at(&cols, &row, "id")?;
            put(&cols, &mut row, "paper_id", Value::Integer(self.paper_id));
            let new = insert_row(self.conn, "chunks", &cols, &row, "INSERT", &["id"])?;
            map.push((old, new));
        }
        Ok(map)
    }

    /// 埋め込みを写す。`vec` はそのままのバイト列。モデルは名前で寄せる。
    fn copy_embeddings(&self, chunks: &[(i64, i64)]) -> Result<(), MigrationError> {
        let cols = common_columns(self.conn, "chunk_embeddings")?;
        if cols.is_empty() {
            return Ok(());
        }
        let mut models: HashMap<i64, i64> = HashMap::new();
        for &(old, new) in chunks {
            let rows = select_rows(
                self.conn,
                "chunk_embeddings",
                &cols,
                "chunk_id = ?1",
                &[Value::Integer(old)],
            )?;
            for mut row in rows {
                let model = int_at(&cols, &row, "model_id")?;
                let dst_model = match models.get(&model) {
                    Some(id) => *id,
                    None => {
                        let id = self.map_model(model)?;
                        models.insert(model, id);
                        id
                    }
                };
                put(&cols, &mut row, "chunk_id", Value::Integer(new));
                put(&cols, &mut row, "model_id", Value::Integer(dst_model));
                insert_row(
                    self.conn,
                    "chunk_embeddings",
                    &cols,
                    &row,
                    "INSERT OR REPLACE",
                    &[],
                )?;
            }
        }
        Ok(())
    }

    /// 移植元の埋め込みモデルの id を、移植先の id に寄せる（名前で探し、無ければ作る）。
    fn map_model(&self, source_model: i64) -> Result<i64, MigrationError> {
        let cols = common_columns(self.conn, "embedding_models")?;
        let rows = select_rows(
            self.conn,
            "embedding_models",
            &cols,
            "id = ?1",
            &[Value::Integer(source_model)],
        )?;
        let row = rows
            .first()
            .ok_or_else(|| MigrationError::Failed("束に埋め込みモデルがありません".to_string()))?;
        map_named(self.conn, "embedding_models", &cols, row, &["name"])
    }

    /// 論文に紐づく表を 1 種類写す（`by` は移植元の論文を指す列）。`fix` で差し替える列を決める。
    fn copy_simple(
        &self,
        table: &str,
        by: &str,
        fix: impl Fn(&[String], &mut [Value]),
    ) -> Result<(), MigrationError> {
        let cols = common_columns(self.conn, table)?;
        if cols.is_empty() {
            return Ok(());
        }
        let rows = select_rows(
            self.conn,
            table,
            &cols,
            &format!("{by} = ?1"),
            &[Value::Integer(self.source_id)],
        )?;
        for mut row in rows {
            fix(&cols, &mut row);
            insert_row(self.conn, table, &cols, &row, "INSERT OR REPLACE", &[])?;
        }
        Ok(())
    }

    /// 束の中のパス（`papers/<束の名前>/…`）を移植先のフォルダのパスへ直す（data_dir からの相対形）。
    ///
    /// 束の名前の後ろ（`figures/fig-1.png` など）を移植先のフォルダに付ける。
    /// 名前が見つからなければ、`sub` の下にファイル名だけを付ける。
    fn to_dst_path(&self, value: &str, sub: &str) -> String {
        let prefix = format!("papers/{}/", self.src_name);
        let inner = format!("/{}/", self.src_name);
        let rest = match value.strip_prefix(&prefix) {
            Some(rest) => Some(rest),
            None => value.find(&inner).map(|i| &value[i + inner.len()..]),
        };
        let rel = match rest {
            Some(rest) => PathBuf::from(rest),
            None => match Path::new(value).file_name() {
                Some(name) => Path::new(sub).join(name),
                None => PathBuf::from(sub),
            },
        };
        paths::to_stored_path(self.data_dir, self.dst_folder.join(rel))
    }
}

/// 移植先の `papers` に同じ論文を探すための値（移植元の DOI・arXiv・S2・hash）。
type SourceKeys = (
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
);

/// 移植元の題。無ければ `None`。
fn src_title(conn: &Connection, source_id: i64) -> Result<Option<String>, MigrationError> {
    Ok(conn
        .query_row(
            "SELECT title FROM src.papers WHERE id = ?1",
            [source_id],
            |row| row.get(0),
        )
        .optional()?)
}

/// 移植元の重複判定用の値。
fn src_keys(conn: &Connection, source_id: i64) -> Result<SourceKeys, MigrationError> {
    Ok(conn.query_row(
        "SELECT doi, arxiv_id, s2_paper_id, content_hash FROM src.papers WHERE id = ?1",
        [source_id],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
    )?)
}

/// 移植先で同じ論文を 1 つ探す（DOI・arXiv・S2・hash の順。空は比べない）。
///
/// 戻り値は `(papers.id, 題, pdf_path)`。
fn find_duplicate(
    conn: &Connection,
    keys: &SourceKeys,
) -> Result<Option<(i64, String, Option<String>)>, MigrationError> {
    let (doi, arxiv, s2, hash) = keys;
    Ok(conn
        .query_row(
            "SELECT id, title, pdf_path FROM papers
             WHERE (?1 IS NOT NULL AND ?1 <> '' AND doi = ?1 COLLATE NOCASE)
                OR (?2 IS NOT NULL AND ?2 <> '' AND arxiv_id = ?2 COLLATE NOCASE)
                OR (?3 IS NOT NULL AND ?3 <> '' AND s2_paper_id = ?3)
                OR (?4 IS NOT NULL AND ?4 <> '' AND content_hash = ?4)
             ORDER BY id LIMIT 1",
            params![doi, arxiv, s2, hash],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?)
}

/// 移植元の論文を移植先の `papers` に新しく作る（`id`・`pdf_path` は移植先で決める）。
fn insert_paper(conn: &Connection, source_id: i64) -> Result<i64, MigrationError> {
    let cols = common_columns(conn, "papers")?;
    let rows = select_rows(
        conn,
        "papers",
        &cols,
        "id = ?1",
        &[Value::Integer(source_id)],
    )?;
    let row = rows
        .first()
        .ok_or_else(|| MigrationError::Failed("束の DB にこの論文がありません".to_string()))?;
    Ok(insert_row(
        conn,
        "papers",
        &cols,
        row,
        "INSERT",
        &["id", "pdf_path"],
    )?)
}

/// PDF 待ちの移植先の論文を、移植元の値で補う（移植先の値があれば残す）。
///
/// `content_hash`・`file_size` だけは移植元の値で上書きする。
fn fill_paper(conn: &Connection, source_id: i64, paper_id: i64) -> Result<(), MigrationError> {
    let skip = ["id", "title", "created_at", "pdf_path"];
    let sets: Vec<String> = common_columns(conn, "papers")?
        .into_iter()
        .filter(|c| !skip.contains(&c.as_str()))
        .map(|c| {
            if c == "content_hash" || c == "file_size" {
                format!("{c} = (SELECT s.{c} FROM src.papers AS s WHERE s.id = ?1)")
            } else {
                format!("{c} = COALESCE({c}, (SELECT s.{c} FROM src.papers AS s WHERE s.id = ?1))")
            }
        })
        .collect();
    if sets.is_empty() {
        return Ok(());
    }
    conn.execute(
        &format!("UPDATE main.papers SET {} WHERE id = ?2", sets.join(", ")),
        params![source_id, paper_id],
    )?;
    Ok(())
}

/// 移植先の論文の古い派生の行を消す（PDF 待ちを補うとき、新しい行に置き換えるため）。
fn clear_derived(conn: &Connection, paper_id: i64) -> Result<(), MigrationError> {
    for (table, col) in [
        ("chunks", "paper_id"),
        ("paper_sections", "paper_id"),
        ("paper_passages", "paper_id"),
        ("paper_section_builds", "paper_id"),
        ("paper_extracts", "papers_id"),
        ("paper_figure_extracts", "papers_id"),
        ("paper_figures", "papers_id"),
        ("paper_refs", "papers_id"),
        ("paper_sentences", "papers_id"),
        ("paper_translations", "papers_id"),
    ] {
        conn.execute(
            &format!("DELETE FROM main.{table} WHERE {col} = ?1"),
            [paper_id],
        )?;
    }
    Ok(())
}

/// 移植先の論文の題。
fn title_of(conn: &Connection, paper_id: i64) -> Result<String, MigrationError> {
    Ok(conn.query_row(
        "SELECT title FROM papers WHERE id = ?1",
        [paper_id],
        |row| row.get(0),
    )?)
}

/// 名前のある表（作者・タグ・モデル）で、`keys` の順に同じ行を探す。無ければ作る。
fn map_named(
    conn: &Connection,
    table: &str,
    cols: &[String],
    row: &[Value],
    keys: &[&str],
) -> Result<i64, MigrationError> {
    for key in keys {
        let Some(value) = value_at(cols, row, key) else {
            continue;
        };
        if matches!(value, Value::Null) || matches!(value, Value::Text(t) if t.is_empty()) {
            continue;
        }
        let found: Option<i64> = conn
            .query_row(
                &format!("SELECT id FROM main.{table} WHERE {key} = ?1 LIMIT 1"),
                [value],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(id) = found {
            return Ok(id);
        }
    }
    Ok(insert_row(conn, table, cols, row, "INSERT", &["id"])?)
}

/// 行の型（列の順）。
type Row = Vec<Value>;

/// 移植元（`src`）の表の列のうち、移植先（`main`）にもある列（移植先の順）。
///
/// 移植元に表が無ければ空。
fn common_columns(conn: &Connection, table: &str) -> Result<Vec<String>, MigrationError> {
    let src = table_columns(conn, "src", table)?;
    if src.is_empty() {
        return Ok(Vec::new());
    }
    Ok(table_columns(conn, "main", table)?
        .into_iter()
        .filter(|c| src.contains(c))
        .collect())
}

/// `schema.table` の列名（`PRAGMA table_info` の順）。表が無ければ空。
fn table_columns(
    conn: &Connection,
    schema: &str,
    table: &str,
) -> Result<Vec<String>, MigrationError> {
    let mut stmt = conn.prepare(&format!("PRAGMA {schema}.table_info({table})"))?;
    let names = stmt
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<rusqlite::Result<Vec<String>>>()?;
    Ok(names)
}

/// 移植元（`src`）の表から、`cond`（`?1` に値）に合う行を `cols` の順で読む。
fn select_rows(
    conn: &Connection,
    table: &str,
    cols: &[String],
    cond: &str,
    args: &[Value],
) -> Result<Vec<Row>, MigrationError> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {} FROM src.{table} WHERE {cond}",
        cols.join(", ")
    ))?;
    let rows = stmt
        .query_map(params_from_iter(args.iter()), |row| {
            (0..cols.len())
                .map(|i| row.get::<_, Value>(i))
                .collect::<rusqlite::Result<Row>>()
        })?
        .collect::<rusqlite::Result<Vec<Row>>>()?;
    Ok(rows)
}

/// 移植先（`main`）の表へ 1 行入れて、新しい rowid を返す。`skip` の列は入れない。
fn insert_row(
    conn: &Connection,
    table: &str,
    cols: &[String],
    row: &[Value],
    verb: &str,
    skip: &[&str],
) -> Result<i64, rusqlite::Error> {
    let picked: Vec<(&str, &Value)> = cols
        .iter()
        .zip(row)
        .filter(|(c, _)| !skip.contains(&c.as_str()))
        .map(|(c, v)| (c.as_str(), v))
        .collect();
    let names = picked
        .iter()
        .map(|(c, _)| *c)
        .collect::<Vec<_>>()
        .join(", ");
    let marks = (1..=picked.len())
        .map(|i| format!("?{i}"))
        .collect::<Vec<_>>()
        .join(", ");
    let values: Vec<&Value> = picked.iter().map(|(_, v)| *v).collect();
    conn.execute(
        &format!("{verb} INTO main.{table} ({names}) VALUES ({marks})"),
        params_from_iter(values),
    )?;
    Ok(conn.last_insert_rowid())
}

/// 行の列 `name` を `value` にする（列が無ければ何もしない）。
fn put(cols: &[String], row: &mut [Value], name: &str, value: Value) {
    if let Some(i) = cols.iter().position(|c| c == name) {
        row[i] = value;
    }
}

/// 行の列 `name` の値。
fn value_at<'a>(cols: &[String], row: &'a [Value], name: &str) -> Option<&'a Value> {
    cols.iter().position(|c| c == name).map(|i| &row[i])
}

/// 行の列 `name` の整数。整数でなければ失敗にする。
fn int_at(cols: &[String], row: &[Value], name: &str) -> Result<i64, MigrationError> {
    match value_at(cols, row, name) {
        Some(Value::Integer(i)) => Ok(*i),
        _ => Err(MigrationError::Failed(format!(
            "整数の列 {name} がありません"
        ))),
    }
}

/// フォルダを再帰で写す。既にあるファイルは上書きしない。シンボリックリンクは写さない。
fn copy_dir_all(src: &Path, dst: &Path) -> std::io::Result<()> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let to = dst.join(entry.file_name());
        if file_type.is_dir() {
            copy_dir_all(&entry.path(), &to)?;
        } else if file_type.is_file() && !to.exists() {
            fs::copy(entry.path(), &to)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn to_dst_path_rewrites_bundle_folder_name() {
        let conn = Connection::open_in_memory().unwrap();
        let mv = Move {
            conn: &conn,
            data_dir: Path::new("/d"),
            source_id: 1,
            paper_id: 2,
            key: String::new(),
            src_name: "0001_a".to_string(),
            dst_folder: PathBuf::from("/d/papers/0002_a"),
            pdf_stored: String::new(),
            md_stored: String::new(),
            pdf_mtime: None,
        };
        assert_eq!(
            mv.to_dst_path("papers/0001_a/figures/f.png", "figures"),
            "papers/0002_a/figures/f.png"
        );
        assert_eq!(
            mv.to_dst_path("/old/0001_a/paper-ja.pdf", ""),
            "papers/0002_a/paper-ja.pdf"
        );
        assert_eq!(
            mv.to_dst_path("/elsewhere/f.png", "figures"),
            "papers/0002_a/figures/f.png"
        );
    }
}
