//! `sai-moon-paper relayout` — 既存データを新しい置き場の規則へ移す（段階 2d）。
//!
//! 新しい規則（`docs/implementation-plan-2026-09-24-review-fixes.md` 段階 2）:
//!
//! - 論文 1 件分のファイルは `<data_dir>/papers/{id:04}_短いタイトル/` に集める
//! - DB には `.sai/paper/` からの相対パスで書く（移動後に書き直す）
//! - 同じ論文の二重登録（DOI / arXiv / S2 / 正規化タイトルが一致）は
//!   番号の小さい方へまとめ、大きい方を消す
//! - 翻訳 PDF が論文として登録された行は元の論文へまとめる。
//!   元の論文が見つからない場合・元の PDF が無い場合は動かさず「要確認」
//! - DB が指すのに実在しない PDF は動かさず「要確認」
//! - Library に無い抽出記録（`papers_id` が NULL でどの論文にも当たらない行）は
//!   消さず件数だけ表示する
//! - 移したあとに空になったフォルダは消す。data_dir の中で DB が参照しなくなった
//!   古いファイルは `relayout-leftovers/<日時>/` へ移す
//!
//! 既定は dry-run（予定の表示だけ）。`--apply` のときだけ動かし、その前に必ず
//! `papr.db` のバックアップ（`papr.db.bak-relayout-<日時>`）を作る。バックアップが
//! 作れなければ何も変更しない。ファイル操作に失敗したら動かした分を元に戻し、
//! DB は変更しない。何度実行しても同じ結果になる（2 回目は「移すものはありません」）。

use anyhow::{Context, Result};
use chrono::Local;
use rusqlite::{Connection, OptionalExtension, params, params_from_iter};
use sai_paper_core::citation::normalize_paper_id;
use sai_paper_core::citation::docling::sanitize_filename_component;
use sai_paper_core::database::Database;
use sai_paper_core::paths::{self, paper_file_names};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

/// 論文を参照しうる、抽出キーで結び付く表（段階 2c で `papers_id` を持つ）。
const PAPER_KEY_TABLES: &[&str] = &[
    "paper_refs",
    "paper_sentences",
    "paper_extracts",
    "paper_figures",
    "paper_figure_extracts",
    "paper_translations",
];

/// パスを保存している列（表, 列）。移動後に DB を書き直す対象。
const PATH_COLUMNS: &[(&str, &str)] = &[
    ("papers", "pdf_path"),
    ("paper_extracts", "pdf_path"),
    ("paper_extracts", "md_path"),
    ("paper_translations", "pdf_path"),
    ("paper_translations", "pdf_ja_path"),
    ("paper_figures", "image_path"),
    ("paper_figure_extracts", "pdf_path"),
];

/// 走査して古いファイルを探す場所（`data_dir` 直下。設定ファイル等は対象外）。
const LEFTOVER_SCAN_DIRS: &[&str] = &["extracts", "notes", "translations"];

/// DB 参照の優先度（小さい方が先。段階 2d が集める 4 列を先に、残りを後に）。
const PRIO_PDF: usize = 0;
const PRIO_EXTRACT_MD: usize = 1;
const PRIO_TRANSLATION_JA: usize = 2;
const PRIO_FIGURE: usize = 3;
const PRIO_OTHER_COLUMN: usize = 4;
const PRIO_AT_DEST: usize = 5;
const PRIO_LOOSE: usize = 6;

/// 論文 1 行分。
#[derive(Debug, Clone)]
struct PaperRow {
    id: i64,
    title: String,
    doi: Option<String>,
    arxiv_id: Option<String>,
    s2_paper_id: Option<String>,
    pdf_path: Option<String>,
}

/// ファイルの種別（決まった名前に直すかどうかの手がかり）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FileKind {
    /// 論文 本体の PDF（`paper.pdf`）
    MainPdf,
    /// 翻訳 PDF（`paper-ja.pdf` / `paper-dual.pdf`）
    TranslationPdf,
    /// 抽出 Markdown（`extract.md`）
    Extract,
    /// ノート（`note.md`）
    Note,
    /// 図（`figures/<名前>`）
    Image,
    /// そのままの名前で動かすファイル
    Plain,
}

/// 動作 1 件（適用順）。
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "action", rename_all = "snake_case")]
enum Op {
    /// フォルダの名前変更（番号＋短いタイトル）。
    RenameDir { paper: i64, from: PathBuf, to: PathBuf },
    /// ファイルの移動。
    MoveFile { paper: i64, from: PathBuf, to: PathBuf, kind: String },
    /// 中身が同じ重複ファイルの削除（`keep` に中身が残る）。
    DeleteSame { paper: i64, path: PathBuf, keep: PathBuf, defer: bool },
    /// 置き場の無いファイルの退避先（`relayout-leftovers/`）。
    Leftover { paper: Option<i64>, from: PathBuf, to: PathBuf, reason: String },
    /// 移動のあとに空になったフォルダの削除。
    RemoveDir { paper: i64, path: PathBuf },
}

/// 同じ論文のまとめ（番号の小さい方 `to` へ）。
#[derive(Debug, Clone, Serialize)]
struct MergePlan {
    from: i64,
    from_title: String,
    to: i64,
    to_title: String,
    /// 重複の種類。
    reason: String,
    /// まとめ先が PDF を未登録のままなら、移動先のパスをそこに書く。
    adopt_pdf: Option<String>,
    /// 翻訳 PDF の移動先（格納パス）。翻訳の誤登録まとめでのみ使う。
    translation_to: Option<String>,
    /// 翻訳 PDF の元パス（格納パス）。
    translation_from: Option<String>,
}

/// DB のパス書き替え 1 件。
#[derive(Debug, Clone, Serialize)]
struct DbPathUpdate {
    table: String,
    column: String,
    rowid: i64,
    from: String,
    to: String,
}

/// 記録の付け替え 1 件（表示用。実際の書き替えは `db_path_updates` に入る）。
#[derive(Debug, Clone, Serialize)]
struct Repoint {
    paper: i64,
    what: String,
    from: String,
    to: String,
}

/// Library に無い抽出記録の件数（rule 9。件数の表示だけ、消さない）。
#[derive(Debug, Default, Serialize)]
struct Orphans {
    paper_refs: i64,
    paper_sentences: i64,
    paper_extracts: i64,
    paper_figures: i64,
    paper_figure_extracts: i64,
    paper_translations: i64,
}

impl Orphans {
    fn total(&self) -> i64 {
        self.paper_refs
            + self.paper_sentences
            + self.paper_extracts
            + self.paper_figures
            + self.paper_figure_extracts
            + self.paper_translations
    }
}

/// 件数のまとめ。
#[derive(Debug, Default, Serialize)]
struct Counts {
    moves: usize,
    renames: usize,
    deletes: usize,
    leftovers: usize,
    rmdirs: usize,
    repoints: usize,
    merges: usize,
    confirm: usize,
    db_path_updates: usize,
    orphan_records: i64,
}

/// 予定（dry-run）と適用される内容そのもの。
#[derive(Debug, Serialize)]
struct Plan {
    /// バックアップ・退避先の日時（実行ごとに 1 つ）。
    timestamp: String,
    /// ファイル操作（適用順）。
    ops: Vec<Op>,
    /// まとめ。
    merges: Vec<MergePlan>,
    /// 要確認（動かさないもの）。
    confirm: Vec<String>,
    /// DB のパス書き替え。
    db_path_updates: Vec<DbPathUpdate>,
    /// 記録の付け替え（表示用）。
    repoints: Vec<Repoint>,
    /// Library に無い抽出記録。
    orphans: Orphans,
    /// 関わった論文のタイトル（表示用）。
    paper_titles: BTreeMap<i64, String>,
    /// 件数。
    counts: Counts,
}

impl Plan {
    /// 何もしない予定かどうか（要確認・抽出記録の件数表示は含まない）。
    fn is_noop(&self) -> bool {
        self.ops.is_empty() && self.merges.is_empty() && self.db_path_updates.is_empty()
    }
}

/// DB 参照（表の列）1 件。
#[derive(Debug, Clone)]
struct PathEntry {
    prio: usize,
    kind: FileKind,
    /// 参照している論文（Library 外の抽出記録は `None`）。
    paper: Option<i64>,
    stored: String,
}

/// 候補ファイル 1 件。
#[derive(Debug, Clone)]
struct Candidate {
    path: PathBuf,
    prio: usize,
    canonical: String,
}

/// `sai-moon-paper relayout` の本体。
///
/// # 引数
///
/// * `apply` - true のときだけ実際に動かす（false なら予定の表示だけ）
/// * `json` - true のとき機械向け（JSON）出力
pub async fn handle_relayout(apply: bool, json: bool) -> Result<()> {
    let db = crate::open_database()?;
    let data_dir = db
        .data_dir()
        .context("papr.db の場所（data_dir）が解決できません")?
        .to_path_buf();
    let plan = build_plan(&db, &data_dir)?;
    if apply {
        execute_plan(&db, &data_dir, &plan)?;
    }
    print_plan(&plan, apply, json, &data_dir)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// 小さな文字列・パスの規則
// ---------------------------------------------------------------------------

/// 正規化タイトル（小文字化し、英数字以外を除く。段階 2d の重複判定）。
fn norm_title(title: &str) -> String {
    title
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

/// タイトル末尾の翻訳接尾辞（` mono` / ` ja` / ` dual`）。
fn translation_title_suffix(title: &str) -> Option<&'static str> {
    for suffix in ["mono", "ja", "dual"] {
        let full = format!(" {suffix}");
        if title.len() > full.len() && title.ends_with(&full) {
            return Some(suffix);
        }
    }
    None
}

/// 末尾の翻訳接尾辞を除いたタイトル。
fn strip_translation_title(title: &str) -> &str {
    match translation_title_suffix(title) {
        Some(suffix) => &title[..title.len() - (suffix.len() + 1)],
        None => title,
    }
}

/// PDF の名前が翻訳 PDF か（`-mono.pdf` / `-ja.pdf` / `-dual.pdf`）。
fn translation_pdf_kind(file_name: &str) -> Option<&'static str> {
    let lower = file_name.to_ascii_lowercase();
    if lower.ends_with("-mono.pdf") {
        Some("mono")
    } else if lower.ends_with("-ja.pdf") {
        Some("ja")
    } else if lower.ends_with("-dual.pdf") {
        Some("dual")
    } else {
        None
    }
}

/// 保存パス（DB の値）からファイル名を取り出す。
fn stored_file_name(stored: &str) -> Option<String> {
    Path::new(stored)
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
}

/// 翻訳 PDF が論文として登録された行かどうか。
fn is_translation_paper(paper: &PaperRow) -> bool {
    let pdf_is_translation = paper
        .pdf_path
        .as_deref()
        .and_then(stored_file_name)
        .is_some_and(|name| translation_pdf_kind(&name).is_some());
    pdf_is_translation || translation_title_suffix(&paper.title).is_some()
}

/// 決まった名前に直す（段階 2d の集め先の名前）。
fn canonical_name(file_name: &str, kind: FileKind) -> String {
    let lower = file_name.to_ascii_lowercase();
    let translated = if lower.ends_with("-mono.pdf") || lower.ends_with("-ja.pdf") {
        Some(paper_file_names::PDF_JA.to_string())
    } else if lower.ends_with("-dual.pdf") {
        Some(paper_file_names::PDF_DUAL.to_string())
    } else {
        None
    };
    match kind {
        FileKind::MainPdf => translated.unwrap_or_else(|| paper_file_names::PDF.to_string()),
        FileKind::TranslationPdf => translated.unwrap_or_else(|| {
            if lower.ends_with(".pdf") {
                paper_file_names::PDF_JA.to_string()
            } else {
                file_name.to_string()
            }
        }),
        FileKind::Extract => paper_file_names::EXTRACT.to_string(),
        FileKind::Note => paper_file_names::NOTE.to_string(),
        FileKind::Image => format!("{}/{file_name}", paper_file_names::FIGURES_DIR),
        FileKind::Plain => translated.unwrap_or_else(|| file_name.to_string()),
    }
}

/// 論文のフォルダ候補（`<data_dir>/papers/` と `<data_dir>` 直下）。
///
/// 名前が `{id:04}_` で始まる、または `{id:04}` と一致するフォルダ。
fn paper_dirs(data_dir: &Path, paper_id: i64) -> Vec<PathBuf> {
    let prefix = format!("{paper_id:04}_");
    let exact = format!("{paper_id:04}");
    let mut found = Vec::new();
    for base in [data_dir.join("papers"), data_dir.to_path_buf()] {
        let Ok(entries) = fs::read_dir(&base) else {
            continue;
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if (name.starts_with(&prefix) || name == exact) && entry.path().is_dir() {
                found.push(entry.path());
            }
        }
    }
    found.sort();
    found
}

/// 「番号＋短いタイトル」の新しいフォルダ名（作らない）。
fn desired_folder(data_dir: &Path, paper_id: i64, title: &str) -> PathBuf {
    let slug = paths::short_title_slug(title);
    let name = if slug.is_empty() {
        format!("{paper_id:04}")
    } else {
        format!("{paper_id:04}_{slug}")
    };
    data_dir.join("papers").join(name)
}

/// 論文に元の PDF（翻訳 PDF 以外）が実在するか。
fn has_original_pdf(paper: &PaperRow, data_dir: &Path) -> bool {
    if let Some(stored) = paper.pdf_path.as_deref() {
        let path = paths::resolve_stored_path(data_dir, stored);
        if path.is_file()
            && path
                .file_name()
                .map(|n| translation_pdf_kind(&n.to_string_lossy()).is_none())
                .unwrap_or(true)
        {
            return true;
        }
    }
    paper_dirs(data_dir, paper.id)
        .iter()
        .any(|dir| dir.join(paper_file_names::PDF).is_file())
}

/// 中身が同じか（サイズが同じときだけ内容を比較する）。
fn same_content(a: &Path, b: &Path) -> bool {
    let (Ok(ma), Ok(mb)) = (fs::metadata(a), fs::metadata(b)) else {
        return false;
    };
    if !ma.is_file() || !mb.is_file() || ma.len() != mb.len() {
        return false;
    }
    let (Ok(mut fa), Ok(mut fb)) = (fs::File::open(a), fs::File::open(b)) else {
        return false;
    };
    use std::io::Read;
    let mut buf_a = [0_u8; 64 * 1024];
    let mut buf_b = [0_u8; 64 * 1024];
    loop {
        let (na, nb) = match (fa.read(&mut buf_a), fb.read(&mut buf_b)) {
            (Ok(a), Ok(b)) => (a, b),
            _ => return false,
        };
        if na != nb || buf_a[..na] != buf_b[..nb] {
            return false;
        }
        if na == 0 {
            return true;
        }
    }
}

/// パスを退避先へ（`relayout-leftovers/<日時>/<親の名前>/<ファイル名>`）。
///
/// 同名があれば連番を付ける。二度同じ名前にしないよう `taken` で予約もする。
fn leftover_dest(
    data_dir: &Path,
    timestamp: &str,
    from: &Path,
    taken: &mut HashSet<PathBuf>,
) -> PathBuf {
    let base = data_dir.join("relayout-leftovers").join(timestamp);
    let parent_name = from
        .parent()
        .and_then(Path::file_name)
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "_".to_string());
    let file_name = from
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "file".to_string());
    let stem = Path::new(&file_name)
        .file_stem()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| file_name.clone());
    let ext = Path::new(&file_name)
        .extension()
        .map(|n| n.to_string_lossy().into_owned());
    let mut candidate = base.join(&parent_name).join(&file_name);
    let mut counter = 1_usize;
    while taken.contains(&candidate) || candidate.exists() {
        counter += 1;
        let name = match &ext {
            Some(ext) => format!("{stem}-{counter}.{ext}"),
            None => format!("{stem}-{counter}"),
        };
        candidate = base.join(&parent_name).join(name);
    }
    taken.insert(candidate.clone());
    candidate
}

/// リネーム済みの座標へパスを写す（適用時に使う）。
fn map_renames(path: &Path, renames: &[(PathBuf, PathBuf)]) -> PathBuf {
    for (from, to) in renames {
        if let Ok(rest) = path.strip_prefix(from) {
            return to.join(rest);
        }
    }
    path.to_path_buf()
}

/// 表示用に `data_dir` からの相対パス（外ならそのまま）。
fn display_path(data_dir: &Path, path: &Path) -> String {
    match path.strip_prefix(data_dir) {
        Ok(rest) => format!(".sai/paper/{}", rest.display()),
        Err(_) => path.display().to_string(),
    }
}

// ---------------------------------------------------------------------------
// 予定づくり（dry-run と同じ内容）
// ---------------------------------------------------------------------------

/// 論文一覧を読む。
fn load_papers(conn: &Connection) -> Result<Vec<PaperRow>> {
    let mut statement = conn.prepare(
        "SELECT id, title, doi, arxiv_id, s2_paper_id, pdf_path FROM papers ORDER BY id",
    )?;
    let rows = statement
        .query_map([], |row| {
            Ok(PaperRow {
                id: row.get(0)?,
                title: row.get(1)?,
                doi: row.get(2)?,
                arxiv_id: row.get(3)?,
                s2_paper_id: row.get(4)?,
                pdf_path: row.get(5)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// 論文の識別子キー（重複判定用。小文字化して空白を落とす）。
fn identity_keys(paper: &PaperRow) -> Vec<String> {
    let mut keys = Vec::new();
    for raw in [&paper.doi, &paper.arxiv_id, &paper.s2_paper_id]
        .into_iter()
        .flatten()
    {
        let key = raw.trim().to_lowercase();
        if !key.is_empty() {
            keys.push(key);
        }
    }
    keys
}

/// 抽出キー（`papers_id` が NULL の行の照合用に正規化して持つ）。
fn extract_keys(paper: &PaperRow) -> Vec<String> {
    let mut keys = Vec::new();
    for raw in [&paper.s2_paper_id, &paper.arxiv_id, &paper.doi].into_iter().flatten() {
        let key = raw.trim();
        if key.is_empty() {
            continue;
        }
        let normalized = normalize_paper_id(key);
        if !keys.contains(&normalized) {
            keys.push(normalized);
        }
    }
    keys
}

/// DB のパス列を全部読む（`pid` は抽出キーからの引き当て込み）。
fn load_path_entries(conn: &Connection, papers: &[PaperRow]) -> Result<Vec<PathEntry>> {
    let mut key_to_paper: HashMap<String, i64> = HashMap::new();
    for paper in papers {
        for key in extract_keys(paper) {
            key_to_paper.entry(key).or_insert(paper.id);
        }
    }
    let resolve = |papers_id: Option<i64>, key: Option<String>| -> Option<i64> {
        papers_id
            .filter(|id| *id > 0)
            .or_else(|| key.as_deref().and_then(|k| key_to_paper.get(&normalize_paper_id(k)).copied()))
    };

    let mut entries: Vec<(usize, FileKind, Option<i64>, String)> = Vec::new();

    let mut statement =
        conn.prepare("SELECT id, pdf_path FROM papers WHERE pdf_path IS NOT NULL")?;
    let rows = statement.query_map([], |row| {
        Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
    })?;
    for (id, stored) in rows.flatten() {
        entries.push((PRIO_PDF, FileKind::MainPdf, Some(id), stored));
    }
    drop(statement);

    let mut statement = conn.prepare(
        "SELECT papers_id, paper_id, md_path, pdf_path FROM paper_extracts
         WHERE md_path IS NOT NULL OR pdf_path IS NOT NULL",
    )?;
    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, Option<i64>>(0)?,
            row.get::<_, Option<String>>(1)?,
            row.get::<_, Option<String>>(2)?,
            row.get::<_, Option<String>>(3)?,
        ))
    })?;
    for (papers_id, key, md_path, pdf_path) in rows.flatten() {
        let pid = resolve(papers_id, key.clone());
        if let Some(md) = md_path.filter(|s| !s.trim().is_empty()) {
            entries.push((PRIO_EXTRACT_MD, FileKind::Extract, pid, md));
        }
        if let Some(pdf) = pdf_path.filter(|s| !s.trim().is_empty()) {
            entries.push((PRIO_OTHER_COLUMN, FileKind::MainPdf, pid, pdf));
        }
    }
    drop(statement);

    let mut statement = conn.prepare(
        "SELECT papers_id, paper_id, pdf_ja_path, pdf_path FROM paper_translations
         WHERE pdf_ja_path IS NOT NULL OR pdf_path IS NOT NULL",
    )?;
    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, Option<i64>>(0)?,
            row.get::<_, Option<String>>(1)?,
            row.get::<_, Option<String>>(2)?,
            row.get::<_, Option<String>>(3)?,
        ))
    })?;
    for (papers_id, key, ja_path, pdf_path) in rows.flatten() {
        let pid = resolve(papers_id, key.clone());
        if let Some(ja) = ja_path.filter(|s| !s.trim().is_empty()) {
            entries.push((PRIO_TRANSLATION_JA, FileKind::TranslationPdf, pid, ja));
        }
        if let Some(pdf) = pdf_path.filter(|s| !s.trim().is_empty()) {
            entries.push((PRIO_OTHER_COLUMN, FileKind::MainPdf, pid, pdf));
        }
    }
    drop(statement);

    let mut statement =
        conn.prepare("SELECT papers_id, paper_id, image_path FROM paper_figures")?;
    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, Option<i64>>(0)?,
            row.get::<_, Option<String>>(1)?,
            row.get::<_, String>(2)?,
        ))
    })?;
    for (papers_id, key, image) in rows.flatten() {
        entries.push((
            PRIO_FIGURE,
            FileKind::Image,
            resolve(papers_id, key),
            image,
        ));
    }
    drop(statement);

    let mut statement =
        conn.prepare("SELECT papers_id, paper_id, pdf_path FROM paper_figure_extracts
                      WHERE pdf_path IS NOT NULL")?;
    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, Option<i64>>(0)?,
            row.get::<_, Option<String>>(1)?,
            row.get::<_, String>(2)?,
        ))
    })?;
    for (papers_id, key, pdf) in rows.flatten() {
        entries.push((
            PRIO_OTHER_COLUMN,
            FileKind::MainPdf,
            resolve(papers_id, key),
            pdf,
        ));
    }
    drop(statement);

    // 同じパスが複数の列から参照されていたら、優先度の高い方を残す。
    let mut best: HashMap<String, (usize, FileKind, Option<i64>)> = HashMap::new();
    for (prio, kind, paper, stored) in entries {
        match best.get(&stored) {
            Some((existing, _, _)) if *existing <= prio => {}
            _ => {
                best.insert(stored, (prio, kind, paper));
            }
        }
    }
    let mut merged: Vec<PathEntry> = best
        .into_iter()
        .map(|(stored, (prio, kind, paper))| PathEntry {
            prio,
            kind,
            paper,
            stored,
        })
        .collect();
    merged.sort_by(|a, b| a.stored.cmp(&b.stored));
    Ok(merged)
}

/// DB が指すパスの集合（「DB が参照していないファイル」の判定用）。
fn load_all_referenced_paths(entries: &[PathEntry], data_dir: &Path) -> HashSet<PathBuf> {
    entries
        .iter()
        .map(|entry| paths::resolve_stored_path(data_dir, &entry.stored))
        .collect()
}

/// Library に無い抽出記録の件数（rule 9）。
fn count_orphan_extracts(conn: &Connection, papers: &[PaperRow]) -> Orphans {
    let mut known: HashSet<String> = HashSet::new();
    for paper in papers {
        for key in extract_keys(paper) {
            known.insert(key);
        }
    }
    let mut orphans = Orphans::default();
    for table in PAPER_KEY_TABLES {
        let select = format!(
            "SELECT paper_id FROM {table} WHERE papers_id IS NULL AND paper_id IS NOT NULL"
        );
        let Ok(mut statement) = conn.prepare(&select) else {
            continue;
        };
        let Ok(rows) = statement.query_map([], |row| row.get::<_, String>(0)) else {
            continue;
        };
        let count = rows
            .flatten()
            .filter(|key| !known.contains(&normalize_paper_id(key)))
            .count() as i64;
        match *table {
            "paper_refs" => orphans.paper_refs = count,
            "paper_sentences" => orphans.paper_sentences = count,
            "paper_extracts" => orphans.paper_extracts = count,
            "paper_figures" => orphans.paper_figures = count,
            "paper_figure_extracts" => orphans.paper_figure_extracts = count,
            "paper_translations" => orphans.paper_translations = count,
            _ => {}
        }
    }
    orphans
}

/// ディレクトリ配下が（`claimed` のファイルを除いて）空になるか。空になるなら
/// 削除するディレクトリを子から順に返す。
fn removal_candidates(dir: &Path, claimed: &HashSet<PathBuf>) -> Option<Vec<PathBuf>> {
    let entries = fs::read_dir(dir).ok()?;
    let mut dirs = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            let mut nested = removal_candidates(&path, claimed)?;
            dirs.append(&mut nested);
        } else if !claimed.contains(&path) {
            return None;
        }
    }
    dirs.push(dir.to_path_buf());
    Some(dirs)
}

/// 予定を作る。読むだけ（DB・ファイルには何も書かない）。
fn build_plan(db: &Database, data_dir: &Path) -> Result<Plan> {
    let conn = db.connection();
    let timestamp = Local::now().format("%Y%m%d-%H%M%S").to_string();
    let papers = load_papers(conn)?;
    let by_id: HashMap<i64, &PaperRow> = papers.iter().map(|p| (p.id, p)).collect();

    let mut confirm: Vec<String> = Vec::new();
    let mut blocked: BTreeSet<i64> = BTreeSet::new();
    let mut target: HashMap<i64, i64> = HashMap::new();

    // --- 翻訳 PDF の誤登録（rule 7）------------------------------------
    let translation_ids: BTreeSet<i64> = papers
        .iter()
        .filter(|p| is_translation_paper(p))
        .map(|p| p.id)
        .collect();

    // 正規化タイトル（翻訳接尾辞を除く）→ 論文番号
    let mut title_index: HashMap<String, Vec<i64>> = HashMap::new();
    for paper in &papers {
        let key = norm_title(strip_translation_title(&paper.title));
        if !key.is_empty() {
            title_index.entry(key).or_default().push(paper.id);
        }
    }
    // 識別子キー → 論文番号
    let mut identity_index: HashMap<String, Vec<i64>> = HashMap::new();
    for paper in &papers {
        for key in identity_keys(paper) {
            identity_index.entry(key).or_default().push(paper.id);
        }
    }

    let label = |paper: &PaperRow| format!("{}（{}）", paper.id, paper.title);
    let candidates_for = |paper: &PaperRow| -> Vec<i64> {
        let mut cands: BTreeSet<i64> = BTreeSet::new();
        let key = norm_title(strip_translation_title(&paper.title));
        if !key.is_empty()
            && let Some(ids) = title_index.get(&key)
        {
            cands.extend(ids.iter().copied().filter(|id| *id != paper.id));
        }
        for id_key in identity_keys(paper) {
            if let Some(ids) = identity_index.get(&id_key) {
                cands.extend(ids.iter().copied().filter(|id| *id != paper.id));
            }
        }
        // 翻訳でない論文を先に、次に番号が小さい順。
        let mut list: Vec<i64> = cands.into_iter().collect();
        list.sort_by_key(|id| (!translation_ids.contains(id), *id));
        list
    };

    // 元の論文を特定する（元が確定できるまで数回まわし、残りは要確認）。
    let mut reported_pairs: HashSet<(i64, i64)> = HashSet::new();
    let mut pending: Vec<i64> = translation_ids.iter().copied().collect();
    let mut progress = true;
    while progress && !pending.is_empty() {
        progress = false;
        let mut rest = Vec::new();
        for id in pending {
            let Some(paper) = by_id.get(&id) else { continue };
            let cands = candidates_for(paper);
            let Some(orig_id) = cands.first().copied() else {
                blocked.insert(id);
                confirm.push(format!(
                    "論文 {} は翻訳 PDF として登録されていますが、元の論文が見つかりません（動かしません）",
                    label(paper)
                ));
                continue;
            };
            let orig = by_id[&orig_id];
            if !has_original_pdf(orig, data_dir) {
                blocked.insert(id);
                if translation_ids.contains(&orig_id) {
                    // 互いに相手を「元の論文」とみなす組は 1 行にまとめる。
                    let pair = (id.min(orig_id), id.max(orig_id));
                    if reported_pairs.insert(pair) {
                        confirm.push(format!(
                            "論文 {} と {} は同じ論文のようですが、どちらにも元の PDF がありません（動かしません）",
                            pair.0, pair.1
                        ));
                    }
                } else {
                    confirm.push(format!(
                        "論文 {} の元の論文 {}（{}）に元の PDF がありません（動かしません）",
                        label(paper),
                        orig_id,
                        orig.title
                    ));
                }
                continue;
            }
            if translation_ids.contains(&orig_id) && !target.contains_key(&orig_id) {
                if blocked.contains(&orig_id) {
                    blocked.insert(id);
                    confirm.push(format!(
                        "論文 {} の元の論文 {} が動かせないため、まとめられません（動かしません）",
                        label(paper),
                        orig_id
                    ));
                } else {
                    rest.push(id);
                }
                continue;
            }
            target.insert(id, orig_id);
            progress = true;
        }
        pending = rest;
    }
    for id in pending {
        if let Some(paper) = by_id.get(&id) {
            blocked.insert(id);
            confirm.push(format!(
                "論文 {} は翻訳 PDF として登録されていますが、元の論文を確定できません（動かしません）",
                label(paper)
            ));
        }
    }

    // --- 同じ論文の二重登録（rule 6）------------------------------------
    let mut union: HashMap<i64, i64> = HashMap::new();
    let find = |id: i64, union: &mut HashMap<i64, i64>| -> i64 {
        let mut cur = id;
        while let Some(&next) = union.get(&cur) {
            if next == cur {
                break;
            }
            cur = next;
        }
        cur
    };
    let mut key_owner: HashMap<String, i64> = HashMap::new();
    for paper in &papers {
        if translation_ids.contains(&paper.id) {
            continue;
        }
        let mut keys = identity_keys(paper);
        let title_key = norm_title(&paper.title);
        if !title_key.is_empty() {
            keys.push(title_key);
        }
        for key in keys {
            match key_owner.get(&key) {
                None => {
                    key_owner.insert(key, paper.id);
                }
                Some(&other) => {
                    let a = find(paper.id, &mut union);
                    let b = find(other, &mut union);
                    if a != b {
                        let (small, big) = if a < b { (a, b) } else { (b, a) };
                        union.insert(big, small);
                    }
                }
            }
        }
    }

    // まとめ先（翻訳の連鎖も番号の小さい方までたどる）。
    let final_target = |id: i64, target: &HashMap<i64, i64>, union: &HashMap<i64, i64>| -> i64 {
        let mut cur = id;
        for _ in 0..16 {
            let next = match target.get(&cur) {
                Some(&n) => n,
                None => {
                    // 二重登録の根（親が自分自身ならそこ）。
                    let mut node = cur;
                    while let Some(&parent) = union.get(&node)
                        && parent != node
                    {
                        node = parent;
                    }
                    node
                }
            };
            if next == cur {
                return cur;
            }
            cur = next;
        }
        cur
    };

    let mut merges: Vec<MergePlan> = Vec::new();
    for paper in &papers {
        let dest = final_target(paper.id, &target, &union);
        if dest != paper.id && !blocked.contains(&paper.id) {
            let reason = if translation_ids.contains(&paper.id) {
                "翻訳 PDF の誤登録"
            } else {
                "同じ論文の重複登録"
            };
            let dest_title = by_id
                .get(&dest)
                .map(|p| p.title.clone())
                .unwrap_or_default();
            merges.push(MergePlan {
                from: paper.id,
                from_title: paper.title.clone(),
                to: dest,
                to_title: dest_title,
                reason: reason.to_string(),
                adopt_pdf: None,
                translation_to: None,
                translation_from: None,
            });
        }
    }

    // --- 集めるファイル（rule 3〜5・7・8）-------------------------------
    let entries = load_path_entries(conn, &papers)?;
    let referenced: HashSet<PathBuf> = load_all_referenced_paths(&entries, data_dir);

    // DB が指す実在ファイル（パス → 優先度・種別・論文）
    let mut db_files: HashMap<PathBuf, (usize, FileKind, i64)> = HashMap::new();
    let mut missing_pdf: Vec<(i64, String)> = Vec::new();
    // 退避済み（`relayout-leftovers/` の中）を指す行: 集めず、あとで付け替える。
    let leftovers_root = data_dir.join("relayout-leftovers");
    let mut in_leftovers: Vec<(PathBuf, FileKind, i64)> = Vec::new();
    for entry in &entries {
        let path = paths::resolve_stored_path(data_dir, &entry.stored);
        if path.starts_with(&leftovers_root) {
            if let Some(pid) = entry.paper.filter(|id| !blocked.contains(id)) {
                in_leftovers.push((path, entry.kind, final_target(pid, &target, &union)));
            }
            continue;
        }
        if path.is_file() {
            if let Some(pid) = entry.paper
                && !blocked.contains(&pid)
            {
                let dest = final_target(pid, &target, &union);
                if !blocked.contains(&dest) {
                    // 翻訳 PDF が論文として登録された行（rule 7）の本体 PDF は、
                    // 名前が何であってもまとめ先の翻訳 PDF として置く。
                    let kind = if entry.prio == PRIO_PDF
                        && entry.paper.is_some_and(|id| translation_ids.contains(&id))
                    {
                        FileKind::TranslationPdf
                    } else {
                        entry.kind
                    };
                    db_files
                        .entry(path)
                        .and_modify(|current| {
                            if entry.prio < current.0 {
                                *current = (entry.prio, kind, dest);
                            }
                        })
                        .or_insert((entry.prio, kind, dest));
                }
            }
            continue;
        }
        // 実在しない PDF（rule 8）。翻訳 PDF の記録は、あとで論文フォルダの
        // 翻訳 PDF へ付け替えを試みる（付け替え先が無いものだけ要確認）。
        let Some(pid) = entry.paper.filter(|id| !blocked.contains(id)) else {
            continue;
        };
        // まとめて消える論文の PDF は無くても困らない（要確認に出さない）。
        if entry.prio == PRIO_PDF && !merges.iter().any(|merge| merge.from == pid) {
            missing_pdf.push((pid, entry.stored.clone()));
        }
    }
    for (pid, stored) in &missing_pdf {
        let title = by_id.get(pid).map(|p| p.title.as_str()).unwrap_or("");
        confirm.push(format!(
            "論文 {pid}（{title}）が指す PDF が見つかりません: {stored}（動かしません）"
        ));
    }

    // まとめ先ごとに候補を寄せる。DB が指すファイルを先に取る（rule 5）。
    let survivors: BTreeSet<i64> = papers
        .iter()
        .map(|p| p.id)
        .filter(|id| !blocked.contains(id))
        .map(|id| final_target(id, &target, &union))
        .filter(|id| !blocked.contains(id))
        .collect();

    let mut claimed: HashSet<PathBuf> = HashSet::new();
    let mut per_paper: BTreeMap<i64, Vec<Candidate>> = BTreeMap::new();
    for (path, (prio, kind, dest_paper)) in &db_files {
        if !claimed.insert(path.clone()) {
            continue;
        }
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        per_paper.entry(*dest_paper).or_default().push(Candidate {
            path: path.clone(),
            prio: *prio,
            canonical: canonical_name(&name, *kind),
        });
    }

    // フォルダと flat のファイル（`extracts/` `notes/`）。
    let mut source_dirs: BTreeMap<i64, BTreeSet<PathBuf>> = BTreeMap::new();
    for paper in &papers {
        if blocked.contains(&paper.id) {
            continue;
        }
        let dest = final_target(paper.id, &target, &union);
        if blocked.contains(&dest) {
            continue;
        }
        for dir in paper_dirs(data_dir, paper.id) {
            source_dirs.entry(dest).or_default().insert(dir);
        }
        // flat: extracts/{id}.md / extracts/{key}.md / notes/{id}.md
        for (base, suffix, kind) in [
            ("extracts", format!("{}.md", paper.id), FileKind::Extract),
            ("notes", format!("{}.md", paper.id), FileKind::Note),
        ] {
            let path = data_dir.join(base).join(&suffix);
            if path.is_file() && claimed.insert(path.clone()) {
                per_paper.entry(dest).or_default().push(Candidate {
                    path,
                    prio: PRIO_LOOSE,
                    canonical: canonical_name(&suffix, kind),
                });
            }
        }
        for key in extract_keys(paper) {
            for suffix in [
                format!("{}.md", sanitize_filename_component(&key)),
                format!("{}.md", key.replace('/', "_")),
            ] {
                let path = data_dir.join("extracts").join(&suffix);
                if path.is_file() && claimed.insert(path.clone()) {
                    per_paper.entry(dest).or_default().push(Candidate {
                        path,
                        prio: PRIO_LOOSE,
                        canonical: canonical_name(&suffix, FileKind::Extract),
                    });
                }
            }
        }
    }

    // フォルダ内を走査（ホストと移す元）。
    let mut ops: Vec<Op> = Vec::new();
    let mut leftover_taken: HashSet<PathBuf> = HashSet::new();
    let mut host_of: BTreeMap<i64, PathBuf> = BTreeMap::new();
    for &survivor in &survivors {
        let title = by_id
            .get(&survivor)
            .map(|p| p.title.clone())
            .unwrap_or_default();
        let desired = desired_folder(data_dir, survivor, &title);
        // ホストは「まとめ先自身のフォルダ」だけから選ぶ（まとめられた論文の
        // フォルダは移す元。番号が小さい方を残すため順序にも依らない）。
        let papers_root = data_dir.join("papers");
        let own_in_papers_root: Vec<PathBuf> = paper_dirs(data_dir, survivor)
            .into_iter()
            .filter(|dir| dir.parent() == Some(papers_root.as_path()))
            .collect();
        let host_current = if desired.is_dir() {
            desired.clone()
        } else {
            own_in_papers_root
                .first()
                .cloned()
                .unwrap_or(desired.clone())
        };
        host_of.insert(survivor, host_current.clone());

        // 候補: フォルダ内
        let mut dirs_to_scan: Vec<PathBuf> = source_dirs
            .get(&survivor)
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .collect();
        if host_current.is_dir() {
            dirs_to_scan.push(host_current.clone());
        }
        dirs_to_scan.sort();
        for dir in dirs_to_scan {
            let is_host = dir == host_current;
            let Ok(read) = fs::read_dir(&dir) else { continue };
            for entry in read.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    // 図フォルダの中だけ集める（その他のサブフォルダは触らない）。
                    let is_figures = path
                        .file_name()
                        .map(|n| n.to_string_lossy() == paper_file_names::FIGURES_DIR)
                        .unwrap_or(false);
                    if !is_figures {
                        continue;
                    }
                    let Ok(sub) = fs::read_dir(&path) else { continue };
                    for file in sub.flatten() {
                        let file_path = file.path();
                        if !file_path.is_file() || !claimed.insert(file_path.clone()) {
                            continue;
                        }
                        let name = file_path
                            .file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_default();
                        let (prio, kind) = match db_files.get(&file_path) {
                            Some((prio, kind, _)) => (*prio, *kind),
                            None => (
                                if is_host { PRIO_AT_DEST } else { PRIO_LOOSE },
                                FileKind::Image,
                            ),
                        };
                        per_paper.entry(survivor).or_default().push(Candidate {
                            path: file_path,
                            prio,
                            canonical: canonical_name(&name, kind),
                        });
                    }
                    continue;
                }
                if !claimed.insert(path.clone()) {
                    continue;
                }
                let name = path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let (prio, kind) = match db_files.get(&path) {
                    Some((prio, kind, _)) => (*prio, *kind),
                    None => (
                        if is_host { PRIO_AT_DEST } else { PRIO_LOOSE },
                        FileKind::Plain,
                    ),
                };
                per_paper.entry(survivor).or_default().push(Candidate {
                    path,
                    prio,
                    canonical: canonical_name(&name, kind),
                });
            }
        }
    }

    // --- 動作に落とす（まとめ先ごと、番号順）----------------------------
    let mut renames: Vec<(PathBuf, PathBuf)> = Vec::new();
    // (まとめ先, 決まった名前) → 適用後の場所。記録の付け替え先に使う。
    let mut final_dest: HashMap<(i64, String), PathBuf> = HashMap::new();
    // DB のパス（旧の実パス）→ 付け替え先。移動・退避の写像より優先する。
    let mut path_overrides: HashMap<PathBuf, PathBuf> = HashMap::new();
    for &survivor in survivors.iter() {
        let title = by_id
            .get(&survivor)
            .map(|p| p.title.clone())
            .unwrap_or_default();
        let desired = desired_folder(data_dir, survivor, &title);
        let host_current = host_of
            .get(&survivor)
            .cloned()
            .unwrap_or_else(|| desired.clone());
        if host_current != desired && host_current.is_dir() {
            ops.push(Op::RenameDir {
                paper: survivor,
                from: host_current.clone(),
                to: desired.clone(),
            });
            renames.push((host_current.clone(), desired.clone()));
        }
        // ホストは最終的に「番号＋短いタイトル」の名前になる（rename 済みの座標）。
        let host_final = desired.clone();

        // 同じ名前の候補ごとに 1 つだけ残す（rule 5）。
        let mut groups: BTreeMap<String, Vec<Candidate>> = BTreeMap::new();
        for cand in per_paper.remove(&survivor).unwrap_or_default() {
            groups.entry(cand.canonical.clone()).or_default().push(cand);
        }
        for (canonical, mut cands) in groups {
            // 優先度（DB 参照）が同じときは、置き場に近い方を残す
            // （`papers/` 内 > data_dir 内 > 外）。
            let papers_root = data_dir.join("papers");
            let proximity = |path: &Path| -> u8 {
                if path.starts_with(&papers_root) {
                    0
                } else if path.starts_with(data_dir) {
                    1
                } else {
                    2
                }
            };
            cands.sort_by(|a, b| {
                a.prio
                    .cmp(&b.prio)
                    .then_with(|| proximity(&a.path).cmp(&proximity(&b.path)))
                    .then_with(|| a.path.cmp(&b.path))
            });
            let winner = cands[0].clone();
            let dest = host_final.join(&canonical);
            final_dest.insert((survivor, canonical.clone()), dest.clone());
            for loser in cands.iter().skip(1) {
                // 負けた方を指していた DB の行は、残る方（移動先）を指すようにする。
                // （退避先を指したままだと、次の実行で退避を繰り返す）
                path_overrides.insert(loser.path.clone(), dest.clone());
                let loser_current = map_renames(&loser.path, &renames);
                if same_content(&loser_current, &map_renames(&winner.path, &renames)) {
                    ops.push(Op::DeleteSame {
                        paper: survivor,
                        path: loser.path.clone(),
                        keep: dest.clone(),
                        // ホスト内の同じ名前は移動先を空けるため先に消す。
                        defer: !loser_current.starts_with(map_renames(&host_final, &renames)),
                    });
                } else {
                    let to = leftover_dest(data_dir, &timestamp, &loser.path, &mut leftover_taken);
                    ops.push(Op::Leftover {
                        paper: Some(survivor),
                        from: loser.path.clone(),
                        to,
                        reason: "同じ名前で中身が違うファイル".to_string(),
                    });
                }
            }
            let winner_current = map_renames(&winner.path, &renames);
            if winner_current != dest {
                ops.push(Op::MoveFile {
                    paper: survivor,
                    from: winner.path.clone(),
                    to: dest,
                    kind: kind_label(&winner),
                });
            }
        }

        // 空になる移す元フォルダを消す（rule 10）。実行はコミット後。
        let sources: Vec<PathBuf> = source_dirs
            .get(&survivor)
            .map(|set| {
                set.iter()
                    .filter(|dir| dir != &&host_current)
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();
        for dir in sources {
            if let Some(dirs) = removal_candidates(&dir, &claimed) {
                for d in dirs {
                    ops.push(Op::RemoveDir {
                        paper: survivor,
                        path: d,
                    });
                }
            }
        }
    }

    // --- 置き場の無い古いファイル（rule 10。data_dir の中だけ）----------
    let mut scan_targets: Vec<PathBuf> = LEFTOVER_SCAN_DIRS
        .iter()
        .map(|name| data_dir.join(name))
        .collect();
    scan_targets.push(data_dir.join("papers"));
    for base in scan_targets {
        let Ok(read) = fs::read_dir(&base) else { continue };
        for entry in read.flatten() {
            let path = entry.path();
            if !path.is_file() || claimed.contains(&path) || referenced.contains(&path) {
                continue;
            }
            let to = leftover_dest(data_dir, &timestamp, &path, &mut leftover_taken);
            ops.push(Op::Leftover {
                paper: None,
                from: path.clone(),
                to,
                reason: "DB が参照しなくなった古いファイル".to_string(),
            });
            claimed.insert(path);
        }
    }

    // --- 記録の付け替え ---------------------------------------------------
    // 退避済みファイルを指す行: 同じ論文の論文フォルダに同じ種類があればそちらへ。
    for (path, kind, dest_paper) in &in_leftovers {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        if let Some(dest) = final_dest.get(&(*dest_paper, canonical_name(&name, *kind))) {
            path_overrides.insert(path.clone(), dest.clone());
        }
    }
    // 翻訳の記録（行ごと）: 古い場所を指したままなら論文フォルダの翻訳 PDF へ。
    let mut row_overrides: HashMap<(&'static str, &'static str, i64), PathBuf> = HashMap::new();
    let mut repoints: Vec<Repoint> = Vec::new();
    let mut missing_ja: BTreeSet<i64> = BTreeSet::new();
    {
        let mut key_to_paper: HashMap<String, i64> = HashMap::new();
        for paper in &papers {
            for key in extract_keys(paper) {
                key_to_paper.entry(key).or_insert(paper.id);
            }
        }
        let mut statement = conn.prepare(
            "SELECT rowid, papers_id, paper_id, pdf_ja_path, pdf_path FROM paper_translations",
        )?;
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, Option<i64>>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, Option<String>>(4)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        for (rowid, papers_id, key, ja_path, pdf_path) in rows {
            let Some(pid) = papers_id.filter(|id| *id > 0).or_else(|| {
                key.as_deref()
                    .and_then(|k| key_to_paper.get(&normalize_paper_id(k)).copied())
            }) else {
                continue;
            };
            if blocked.contains(&pid) {
                continue;
            }
            let dest_paper = final_target(pid, &target, &union);
            if blocked.contains(&dest_paper) {
                continue;
            }
            let lookup = |name: &str| final_dest.get(&(dest_paper, name.to_string())).cloned();
            if let Some(stored) = ja_path.filter(|s| !s.trim().is_empty()) {
                let current = paths::resolve_stored_path(data_dir, &stored);
                if !current.is_file() && !path_overrides.contains_key(&current) {
                    match lookup(paper_file_names::PDF_JA).or_else(|| lookup(paper_file_names::PDF_DUAL)) {
                        Some(new_path) => {
                            repoints.push(Repoint {
                                paper: dest_paper,
                                what: "翻訳 PDF の記録".to_string(),
                                from: display_path(data_dir, &current),
                                to: display_path(data_dir, &new_path),
                            });
                            row_overrides.insert(("paper_translations", "pdf_ja_path", rowid), new_path);
                        }
                        None => {
                            missing_ja.insert(dest_paper);
                        }
                    }
                }
            }
            if let Some(stored) = pdf_path.filter(|s| !s.trim().is_empty()) {
                let current = paths::resolve_stored_path(data_dir, &stored);
                if !current.is_file()
                    && !path_overrides.contains_key(&current)
                    && let Some(new_path) = lookup(paper_file_names::PDF)
                {
                    row_overrides.insert(("paper_translations", "pdf_path", rowid), new_path);
                }
            }
        }
    }
    if !missing_ja.is_empty() {
        let ids: Vec<String> = missing_ja.iter().map(|id| id.to_string()).collect();
        confirm.push(format!(
            "翻訳 PDF の記録はあるがファイルが見つかりません（{} 件）: 論文 {}（動かしません）",
            missing_ja.len(),
            ids.join(", ")
        ));
    }

    // --- DB の書き替え ---------------------------------------------------
    let db_path_updates =
        plan_db_path_updates(&ops, db, data_dir, &path_overrides, &row_overrides);
    let orphans = count_orphan_extracts(conn, &papers);

    // まとめに必要な移動先を補う（本体 PDF・翻訳 PDF）。
    for merge in merges.iter_mut() {
        let from_row = by_id.get(&merge.from);
        let from_dirs = paper_dirs(data_dir, merge.from);
        let registered_abs: Option<PathBuf> = from_row
            .and_then(|row| row.pdf_path.as_deref())
            .map(|stored| paths::resolve_stored_path(data_dir, stored));
        let is_translation = merge.reason.starts_with("翻訳");

        // 動かした PDF を探す（DB の登録パスが最優先。無ければフォルダ内）。
        let mut matched: Option<(PathBuf, PathBuf)> = None;
        for op in &ops {
            let Op::MoveFile { from, to, .. } = op else {
                continue;
            };
            let by_registration = registered_abs.as_ref() == Some(from);
            let by_folder = from_dirs.iter().any(|dir| from.starts_with(dir));
            let name = from
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            let name_ok = if is_translation {
                translation_pdf_kind(&name).is_some()
            } else {
                name == paper_file_names::PDF
            };
            if by_registration || (by_folder && name_ok) {
                matched = Some((from.clone(), to.clone()));
            }
        }

        match matched {
            // 翻訳 PDF の誤登録（rule 7）: 元の論文の翻訳 PDF として記録する。
            Some((from, to)) if is_translation => {
                merge.translation_from = Some(db.store_path(&from));
                merge.translation_to = Some(db.store_path(&to));
            }
            // 重複（rule 6）: まとめ先が PDF を未登録のときだけ引き継ぐ。
            Some((_, to))
                if to.file_name()
                    .map(|n| n.to_string_lossy() == paper_file_names::PDF)
                    .unwrap_or(false) =>
            {
                let dest_registered = db
                    .connection()
                    .query_row(
                        "SELECT pdf_path FROM papers WHERE id = ?1",
                        [merge.to],
                        |row| row.get::<_, Option<String>>(0),
                    )
                    .ok()
                    .flatten()
                    .is_some();
                if !dest_registered {
                    merge.adopt_pdf = Some(db.store_path(&to));
                }
            }
            _ => {}
        }
    }

    // --- 関わった論文のタイトル（表示用）--------------------------------
    let mut paper_titles: BTreeMap<i64, String> = BTreeMap::new();
    for op in &ops {
        if let Some(paper) = op_paper(op)
            && !paper_titles.contains_key(paper)
            && let Some(row) = by_id.get(paper)
        {
            paper_titles.insert(*paper, row.title.clone());
        }
    }
    for merge in &merges {
        for id in [merge.from, merge.to] {
            if !paper_titles.contains_key(&id)
                && let Some(row) = by_id.get(&id)
            {
                paper_titles.insert(id, row.title.clone());
            }
        }
    }

    for repoint in &repoints {
        if let Some(row) = by_id.get(&repoint.paper) {
            paper_titles.entry(repoint.paper).or_insert_with(|| row.title.clone());
        }
    }

    let mut counts = Counts {
        repoints: repoints.len(),
        merges: merges.len(),
        confirm: confirm.len(),
        db_path_updates: db_path_updates.len(),
        orphan_records: orphans.total(),
        ..Counts::default()
    };
    for op in &ops {
        match op {
            Op::MoveFile { .. } => counts.moves += 1,
            Op::RenameDir { .. } => counts.renames += 1,
            Op::DeleteSame { .. } => counts.deletes += 1,
            Op::Leftover { .. } => counts.leftovers += 1,
            Op::RemoveDir { .. } => counts.rmdirs += 1,
        }
    }

    Ok(Plan {
        timestamp,
        ops,
        merges,
        confirm,
        db_path_updates,
        repoints,
        orphans,
        paper_titles,
        counts,
    })
}

/// 表示用の種別ラベル。
fn kind_label(candidate: &Candidate) -> String {
    let name = candidate
        .path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let lower = name.to_ascii_lowercase();
    if lower.ends_with(".pdf") {
        if translation_pdf_kind(&lower).is_some() || candidate.canonical != paper_file_names::PDF {
            "翻訳PDF".to_string()
        } else {
            "PDF".to_string()
        }
    } else if candidate.canonical == paper_file_names::EXTRACT {
        "抽出".to_string()
    } else if candidate.canonical == paper_file_names::NOTE {
        "ノート".to_string()
    } else if candidate.canonical.starts_with(&format!("{}/", paper_file_names::FIGURES_DIR)) {
        "図".to_string()
    } else {
        "ファイル".to_string()
    }
}

/// 動作から DB のパス書き替えを計算する（移動先は適用時と同じ座標）。
///
/// `path_overrides`（旧の実パス → 新）と `row_overrides`（表・列・行 → 新）は
/// 移動の写像より優先する（記録の付け替え）。
fn plan_db_path_updates(
    ops: &[Op],
    db: &Database,
    data_dir: &Path,
    path_overrides: &HashMap<PathBuf, PathBuf>,
    row_overrides: &HashMap<(&'static str, &'static str, i64), PathBuf>,
) -> Vec<DbPathUpdate> {
    let mut exact: HashMap<PathBuf, PathBuf> = HashMap::new();
    let mut renames: Vec<(PathBuf, PathBuf)> = Vec::new();
    for op in ops {
        match op {
            Op::RenameDir { from, to, .. } => renames.push((from.clone(), to.clone())),
            Op::MoveFile { from, to, .. } | Op::Leftover { from, to, .. } => {
                exact.insert(from.clone(), to.clone());
            }
            Op::DeleteSame { path, keep, .. } => {
                exact.insert(path.clone(), keep.clone());
            }
            Op::RemoveDir { .. } => {}
        }
    }
    let map = |path: &Path| -> PathBuf {
        if let Some(hit) = path_overrides.get(path) {
            return hit.clone();
        }
        if let Some(hit) = exact.get(path) {
            return hit.clone();
        }
        map_renames(path, &renames)
    };

    let conn = db.connection();
    let mut updates = Vec::new();
    for (table, column) in PATH_COLUMNS {
        let select =
            format!("SELECT rowid, {column} FROM {table} WHERE {column} IS NOT NULL");
        let Ok(mut statement) = conn.prepare(&select) else {
            continue;
        };
        let Ok(rows) = statement.query_map([], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        }) else {
            continue;
        };
        for (rowid, stored) in rows.flatten() {
            let current = paths::resolve_stored_path(data_dir, &stored);
            let moved = match row_overrides.get(&(*table, *column, rowid)) {
                Some(hit) => hit.clone(),
                None => map(&current),
            };
            if moved == current {
                continue;
            }
            let new_stored = db.store_path(&moved);
            if new_stored != stored {
                updates.push(DbPathUpdate {
                    table: (*table).to_string(),
                    column: (*column).to_string(),
                    rowid,
                    from: stored,
                    to: new_stored,
                });
            }
        }
    }
    updates.sort_by(|a, b| {
        a.table
            .cmp(&b.table)
            .then_with(|| a.column.cmp(&b.column))
            .then_with(|| a.rowid.cmp(&b.rowid))
    });
    updates
}

/// 動作がどの論文のものか（表示用）。
fn op_paper(op: &Op) -> Option<&i64> {
    match op {
        Op::RenameDir { paper, .. }
        | Op::MoveFile { paper, .. }
        | Op::DeleteSame { paper, .. }
        | Op::RemoveDir { paper, .. } => Some(paper),
        Op::Leftover { paper, .. } => paper.as_ref(),
    }
}

// ---------------------------------------------------------------------------
// 適用
// ---------------------------------------------------------------------------

/// 適用中の巻き戻し操作。
enum Undo {
    /// ファイルを元の場所へ戻す（座標は実行時点）。
    Move { from: PathBuf, to: PathBuf },
    /// フォルダの名前を戻す。
    Rename { current: PathBuf, original: PathBuf },
    /// 作ったフォルダ（巻き戻しで消す。中身が空になったら）。
    CreatedDir { path: PathBuf },
}

/// 1 つの動作を実行する。
fn run_op(
    op: &Op,
    renames: &mut Vec<(PathBuf, PathBuf)>,
    undo: &mut Vec<Undo>,
    deferred: &mut Vec<Op>,
) -> Result<()> {
    match op {
        Op::RenameDir { from, to, .. } => {
            fs::rename(from, to)
                .with_context(|| format!("フォルダの名前変更に失敗: {}", from.display()))?;
            renames.push((from.clone(), to.clone()));
            undo.push(Undo::Rename {
                current: to.clone(),
                original: from.clone(),
            });
        }
        Op::MoveFile { from, to, .. } | Op::Leftover { from, to, .. } => {
            let src = map_renames(from, renames);
            if src == *to {
                return Ok(());
            }
            // 作る予定だったフォルダは、巻き戻しで消せるように記録しておく。
            let mut created: Vec<PathBuf> = Vec::new();
            if let Some(parent) = to.parent() {
                let mut probe = parent.to_path_buf();
                while !probe.as_path().exists() {
                    created.push(probe.clone());
                    if !probe.pop() {
                        break;
                    }
                }
                fs::create_dir_all(parent)
                    .with_context(|| format!("移動先フォルダを作れません: {}", parent.display()))?;
            }
            paths::move_pdf_file(&src, to).with_context(|| {
                format!("ファイルの移動に失敗: {} → {}", src.display(), to.display())
            })?;
            // 外側から内側の順に積む（巻き戻しは逆順 = 内側から消す）。
            for dir in created.into_iter().rev() {
                undo.push(Undo::CreatedDir { path: dir });
            }
            undo.push(Undo::Move {
                from: src,
                to: to.clone(),
            });
        }
        Op::DeleteSame { defer, .. } => {
            if *defer {
                deferred.push(op.clone());
                return Ok(());
            }
            run_delete(op, renames)?;
        }
        Op::RemoveDir { .. } => {
            // コミット後にまとめて処理する。
        }
    }
    Ok(())
}

/// 同一内容ファイルの削除（中身は残す側に存在する）。
fn run_delete(op: &Op, renames: &[(PathBuf, PathBuf)]) -> Result<()> {
    if let Op::DeleteSame { path, .. } = op {
        let current = map_renames(path, renames);
        if current.is_file() {
            fs::remove_file(&current)
                .with_context(|| format!("重複ファイルを消せません: {}", current.display()))?;
        }
    }
    Ok(())
}

/// 実行した分を元へ戻す（動かしたファイル・フォルダ名）。
fn rollback(undo: Vec<Undo>) {
    for entry in undo.into_iter().rev() {
        match entry {
            Undo::Move { from, to } => {
                if to.is_file() {
                    if let Some(parent) = from.parent() {
                        let _ = fs::create_dir_all(parent);
                    }
                    let _ = paths::move_pdf_file(&to, &from);
                }
            }
            Undo::Rename { current, original } => {
                if current.is_dir() && !original.exists() {
                    let _ = fs::rename(&current, &original);
                }
            }
            Undo::CreatedDir { path } => {
                // 中身が残っていれば消えない（エラーは無視）。
                let _ = fs::remove_dir(&path);
            }
        }
    }
}

/// `papr.db` のバックアップを作る（`VACUUM INTO`）。作れなければ None。
fn backup_database(db: &Database, data_dir: &Path, timestamp: &str) -> Option<PathBuf> {
    let base = data_dir.join(format!("papr.db.bak-relayout-{timestamp}"));
    let mut target = base.clone();
    let mut counter = 1_usize;
    while target.exists() {
        counter += 1;
        target = data_dir.join(format!("papr.db.bak-relayout-{timestamp}-{counter}"));
    }
    let sql = "VACUUM INTO ?1";
    match db
        .connection()
        .execute(sql, params![target.to_string_lossy().to_string()])
    {
        Ok(_) => Some(target),
        Err(_) => None,
    }
}

/// 予定を適用する。失敗したら動かしたファイルを戻し、DB は変更しない。
fn execute_plan(db: &Database, data_dir: &Path, plan: &Plan) -> Result<()> {
    // 1) バックアップ。作れなければ何もしない。
    let backup = backup_database(db, data_dir, &plan.timestamp).with_context(|| {
        format!(
            "papr.db のバックアップを作れなかったため、何も変更しませんでした（{}/papr.db.bak-relayout-{}）",
            data_dir.display(),
            plan.timestamp
        )
    })?;

    // 2) ファイル操作（失敗したら巻き戻す）。
    let mut undo: Vec<Undo> = Vec::new();
    let mut renames: Vec<(PathBuf, PathBuf)> = Vec::new();
    let mut deferred: Vec<Op> = Vec::new();
    for op in &plan.ops {
        if let Err(error) = run_op(op, &mut renames, &mut undo, &mut deferred) {
            rollback(undo);
            return Err(error.context(
                "ファイル操作に失敗したため、動かした分を元に戻し、DB は変更していません",
            ));
        }
    }

    // 3) DB の更新（1 つのトランザクション）。
    if let Err(error) = apply_db(db, plan) {
        rollback(undo);
        return Err(error.context(
            "DB の更新に失敗したため、動かしたファイルを元に戻しました（DB は変更なし）",
        ));
    }

    // 4) コミット後の片付け（重複の削除・空フォルダ）。失敗しても結果は保たれる。
    for op in deferred {
        if let Err(error) = run_delete(&op, &renames) {
            eprintln!("警告: {error:#}");
        }
    }
    for op in &plan.ops {
        if let Op::RemoveDir { path, .. } = op && path.is_dir() {
            match fs::remove_dir(path) {
                Err(error) if error.kind() != std::io::ErrorKind::DirectoryNotEmpty => {
                    eprintln!(
                        "警告: 空フォルダを削除できません: {}: {error}",
                        path.display()
                    );
                }
                _ => {}
            }
        }
    }
    let _ = backup;
    Ok(())
}

/// DB の更新を 1 つのトランザクションで行う。
fn apply_db(db: &Database, plan: &Plan) -> Result<()> {
    let transaction = db.connection().unchecked_transaction()?;
    for update in &plan.db_path_updates {
        let sql = format!(
            "UPDATE {} SET {} = ?1 WHERE rowid = ?2",
            update.table, update.column
        );
        transaction.execute(&sql, params![update.to, update.rowid])?;
    }
    for merge in &plan.merges {
        merge_papers(&transaction, db, merge)?;
    }
    transaction.commit()?;
    Ok(())
}

/// 消す側（大きい番号）の論文行。
#[derive(Debug)]
struct MergeSourceRow {
    doi: Option<String>,
    arxiv_id: Option<String>,
    s2_paper_id: Option<String>,
    file_size: Option<i64>,
    content_hash: Option<String>,
    indexed_at: Option<String>,
}

/// 論文の重複をまとめる（`from` を消し、`to` へ全部移す）。
///
/// 論文を参照する表は `PRAGMA foreign_key_list` と migration を見て拾う:
/// `paper_authors` / `paper_tags` / `reading_history` / `activity_log` /
/// `chunks` / `search_set_items` / 6 表の `papers_id` /
/// `paper_podcasts`（外部キー未宣言） / `report_sources`（kind='paper' の文字参照）。
/// `collections`・`collection_papers`・`bookmarks`・`reading_queue` は 0018 で廃止済み。
fn merge_papers(transaction: &Connection, db: &Database, merge: &MergePlan) -> Result<()> {
    let (small, big) = (merge.to, merge.from);

    let big_row: Option<MergeSourceRow> = transaction
        .query_row(
            "SELECT doi, arxiv_id, s2_paper_id, file_size, content_hash, indexed_at
             FROM papers WHERE id = ?1",
            [big],
            |row| {
                Ok(MergeSourceRow {
                    doi: row.get(0)?,
                    arxiv_id: row.get(1)?,
                    s2_paper_id: row.get(2)?,
                    file_size: row.get(3)?,
                    content_hash: row.get(4)?,
                    indexed_at: row.get(5)?,
                })
            },
        )
        .optional()?;
    let Some(big_row) = big_row else {
        return Ok(());
    };
    let MergeSourceRow {
        doi: big_doi,
        arxiv_id: big_arxiv,
        s2_paper_id: big_s2,
        file_size: big_size,
        content_hash: big_hash,
        indexed_at: big_indexed,
    } = big_row;

    // 抽出キーの行（papers_id が NULL のまま残っていた分も含む）。
    let big_keys: Vec<String> = [&big_s2, &big_arxiv, &big_doi]
        .into_iter()
        .flatten()
        .filter(|value| !value.trim().is_empty())
        .map(|value| normalize_paper_id(value))
        .collect();
    for table in PAPER_KEY_TABLES {
        transaction.execute(
            &format!("UPDATE {table} SET papers_id = ?1 WHERE papers_id = ?2"),
            params![small, big],
        )?;
        if !big_keys.is_empty() {
            let placeholders = vec!["?"; big_keys.len()].join(", ");
            let sql = format!(
                "UPDATE {table} SET papers_id = ?1 WHERE papers_id IS NULL AND paper_id IN ({placeholders})"
            );
            let mut values: Vec<&dyn rusqlite::ToSql> = vec![&small];
            let key_params: Vec<&dyn rusqlite::ToSql> =
                big_keys.iter().map(|k| k as &dyn rusqlite::ToSql).collect();
            values.extend(key_params);
            transaction.execute(&sql, params_from_iter(values))?;
        }
    }

    // 著者・タグ
    transaction.execute(
        "INSERT OR IGNORE INTO paper_authors (paper_id, author_id, position)
         SELECT ?1, author_id, position FROM paper_authors WHERE paper_id = ?2",
        params![small, big],
    )?;
    transaction.execute(
        "INSERT OR IGNORE INTO paper_tags (paper_id, tag_id)
         SELECT ?1, tag_id FROM paper_tags WHERE paper_id = ?2",
        params![small, big],
    )?;

    // 読書の履歴・活動記録
    transaction.execute(
        "INSERT INTO reading_history (paper_id, opened_at, page, duration_s)
         SELECT ?1, opened_at, page, duration_s FROM reading_history WHERE paper_id = ?2",
        params![small, big],
    )?;
    transaction.execute(
        "INSERT INTO activity_log (kind, paper_id, detail, occurred_at)
         SELECT kind, ?1, detail, occurred_at FROM activity_log WHERE paper_id = ?2",
        params![small, big],
    )?;

    // 索引（チャンク）と検索セット
    transaction.execute(
        "UPDATE chunks SET paper_id = ?1 WHERE paper_id = ?2",
        params![small, big],
    )?;
    transaction.execute(
        "UPDATE search_set_items SET paper_id = ?1 WHERE paper_id = ?2",
        params![small, big],
    )?;

    // 音声化の状態（外部キー未宣言）
    let has_small_podcast: bool = transaction
        .query_row(
            "SELECT 1 FROM paper_podcasts WHERE paper_id = ?1",
            [small],
            |row| row.get::<_, i64>(0),
        )
        .optional()?
        .is_some();
    if has_small_podcast {
        transaction.execute(
            "DELETE FROM paper_podcasts WHERE paper_id = ?1",
            [big],
        )?;
    } else {
        transaction.execute(
            "UPDATE paper_podcasts SET paper_id = ?1 WHERE paper_id = ?2",
            params![small, big],
        )?;
    }

    // レポート側の参照（kind='paper' は papers.id の文字参照）
    transaction.execute(
        "UPDATE OR IGNORE report_sources SET ref_id = ?1 WHERE kind = 'paper' AND ref_id = ?2",
        params![small.to_string(), big.to_string()],
    )?;
    transaction.execute(
        "DELETE FROM report_sources WHERE kind = 'paper' AND ref_id = ?1",
        [big.to_string()],
    )?;

    // 論文そのものの属性（読書状態・お気に入り・評価・要約）
    transaction.execute(
        "UPDATE papers SET
            reading_status = CASE WHEN reading_status = 'unread'
                THEN (SELECT reading_status FROM papers WHERE id = ?2)
                ELSE reading_status END,
            is_favorite = MAX(is_favorite, (SELECT is_favorite FROM papers WHERE id = ?2)),
            rating = COALESCE(rating, (SELECT rating FROM papers WHERE id = ?2)),
            abstract = CASE WHEN abstract IS NULL OR trim(abstract) = ''
                THEN (SELECT abstract FROM papers WHERE id = ?2) ELSE abstract END,
            abstract_ja = CASE WHEN abstract_ja IS NULL OR trim(abstract_ja) = ''
                THEN (SELECT abstract_ja FROM papers WHERE id = ?2) ELSE abstract_ja END
          WHERE id = ?1",
        params![small, big],
    )?;

    // 大きい方を消す（残す方は番号の小さい方）
    transaction.execute("DELETE FROM papers WHERE id = ?1", [big])?;

    // 識別子と PDF の引き継ぎ（重複制約に引っかからないときだけ）
    for (column, value) in [("doi", big_doi), ("arxiv_id", big_arxiv), ("s2_paper_id", big_s2)] {
        let Some(value) = value.filter(|value| !value.trim().is_empty()) else {
            continue;
        };
        let sql = format!(
            "UPDATE papers SET {column} = ?1
             WHERE id = ?2 AND ({column} IS NULL OR trim({column}) = '')
               AND NOT EXISTS (SELECT 1 FROM papers WHERE {column} = ?1)"
        );
        transaction.execute(&sql, params![value, small])?;
    }
    if let Some(stored) = &merge.adopt_pdf {
        transaction.execute(
            "UPDATE papers SET pdf_path = ?1, file_size = ?2, content_hash = ?3, indexed_at = ?4
             WHERE id = ?5 AND pdf_path IS NULL",
            params![stored, big_size, big_hash, big_indexed, small],
        )?;
    }

    // 翻訳 PDF の記録をまとめ先へ（rule 7）
    if let Some(to_stored) = &merge.translation_to {
        let from_stored = merge.translation_from.clone();
        let rows: Vec<(i64, Option<String>)> = {
            let mut statement = transaction.prepare(
                "SELECT rowid, pdf_ja_path FROM paper_translations WHERE papers_id = ?1",
            )?;
            statement
                .query_map([small], |row| Ok((row.get(0)?, row.get(1)?)))?
                .collect::<Result<Vec<_>, _>>()?
        };
        let had_rows = !rows.is_empty();
        for (rowid, current) in rows {
            let replace = match current.as_deref() {
                None => true,
                Some(value) => {
                    value == from_stored.as_deref().unwrap_or("")
                        || !db.resolve_path(value).is_file()
                }
            };
            if replace {
                transaction.execute(
                    "UPDATE paper_translations SET pdf_ja_path = ?1 WHERE rowid = ?2",
                    params![to_stored, rowid],
                )?;
            }
        }
        if !had_rows
            && let Some(key) = db.citation_key_for_paper(small)
        {
            transaction.execute(
                "INSERT OR IGNORE INTO paper_translations (paper_id, status, pdf_ja_path, papers_id)
                 VALUES (?1, 'ready', ?2, ?3)",
                params![normalize_paper_id(&key), to_stored, small],
            )?;
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// 出力
// ---------------------------------------------------------------------------

/// 予定を見せる（`--json` で機械向け）。
fn print_plan(plan: &Plan, apply: bool, json: bool, data_dir: &Path) -> Result<()> {
    if json {
        println!("{}", serde_json::to_string_pretty(plan)?);
        return Ok(());
    }

    if apply {
        println!("sai-paper relayout（--apply で適用しました）");
    } else {
        println!("sai-paper relayout（dry-run: 予定の表示だけ。--apply で実行）");
    }
    println!("日時: {}", plan.timestamp);
    println!();

    // 論文ごと
    let mut by_paper: BTreeMap<i64, Vec<&Op>> = BTreeMap::new();
    let mut no_paper: Vec<&Op> = Vec::new();
    for op in &plan.ops {
        match op_paper(op) {
            Some(paper) => {
                by_paper.entry(*paper).or_default().push(op);
            }
            None => no_paper.push(op),
        }
    }
    if !by_paper.is_empty() {
        println!("■ 論文ごとの予定");
        for (paper, ops) in &by_paper {
            let title = plan
                .paper_titles
                .get(paper)
                .map(|t| t.as_str())
                .unwrap_or("");
            println!("【論文 {paper}】{title}");
            for op in ops {
                match op {
                    Op::RenameDir { from, to, .. } => println!(
                        "  - フォルダ名を変更: {} → {}",
                        display_path(data_dir, from),
                        display_path(data_dir, to)
                    ),
                    Op::MoveFile { from, to, kind, .. } => println!(
                        "  - {kind} を移動: {} → {}",
                        display_path(data_dir, from),
                        display_path(data_dir, to)
                    ),
                    Op::DeleteSame { path, .. } => println!(
                        "  - 同じ中身の重複を削除: {}",
                        display_path(data_dir, path)
                    ),
                    Op::Leftover { from, to, reason, .. } => println!(
                        "  - {reason}: {} → {}",
                        display_path(data_dir, from),
                        display_path(data_dir, to)
                    ),
                    Op::RemoveDir { path, .. } => println!(
                        "  - 空フォルダを削除: {}",
                        display_path(data_dir, path)
                    ),
                }
            }
        }
        println!();
    }
    if !no_paper.is_empty() {
        println!(
            "■ 置き場の無い古いファイル → {}/{}/",
            display_path(data_dir, &data_dir.join("relayout-leftovers")),
            plan.timestamp
        );
        for op in &no_paper {
            if let Op::Leftover { from, reason, .. } = op {
                println!("  - {reason}: {}", display_path(data_dir, from));
            }
        }
        println!();
    }

    if !plan.merges.is_empty() {
        println!("■ まとめ（同じ論文の二重登録・翻訳 PDF の誤登録）");
        for merge in &plan.merges {
            println!(
                "  - 論文 {}（{}） → 論文 {}（{}）［{}］",
                merge.from, merge.from_title, merge.to, merge.to_title, merge.reason
            );
        }
        println!();
    }

    if !plan.confirm.is_empty() {
        println!("■ 要確認（動きません）");
        for line in &plan.confirm {
            println!("  - {line}");
        }
        println!();
    }

    if plan.counts.orphan_records > 0 {
        println!("■ Library に無い抽出記録（件数だけ表示。動きません）");
        for (table, count) in [
            ("paper_refs", plan.orphans.paper_refs),
            ("paper_sentences", plan.orphans.paper_sentences),
            ("paper_extracts", plan.orphans.paper_extracts),
            ("paper_figures", plan.orphans.paper_figures),
            ("paper_figure_extracts", plan.orphans.paper_figure_extracts),
            ("paper_translations", plan.orphans.paper_translations),
        ] {
            if count > 0 {
                println!("  - {table}: {count} 件");
            }
        }
        println!();
    }

    if !plan.repoints.is_empty() {
        println!("■ 記録の付け替え");
        for r in &plan.repoints {
            println!("  - 論文 {}: {}を付け替え: {} → {}", r.paper, r.what, r.from, r.to);
        }
        println!();
    }

    if !plan.db_path_updates.is_empty() {
        println!("■ DB のパス書き替え: {} 件", plan.db_path_updates.len());
        println!();
    }

    if plan.is_noop() {
        println!("移すものはありません。");
    } else {
        println!("■ 合計");
        println!("  - ファイル移動: {} 件", plan.counts.moves);
        println!("  - フォルダ名変更: {} 件", plan.counts.renames);
        println!("  - 同じ中身の重複削除: {} 件", plan.counts.deletes);
        println!("  - leftovers へ退避: {} 件", plan.counts.leftovers);
        println!("  - 空フォルダ削除: {} 件", plan.counts.rmdirs);
        println!("  - まとめ: {} 件", plan.counts.merges);
        println!("  - 要確認: {} 件", plan.counts.confirm);
        println!("  - 記録の付け替え: {} 件", plan.counts.repoints);
        println!("  - DB のパス書き替え: {} 件", plan.counts.db_path_updates);
        println!(
            "  - Library に無い抽出記録: {} 件（動きません）",
            plan.counts.orphan_records
        );
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// テスト（一時ディレクトリに `.sai/paper/` を作って試す）
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// 一時プロジェクト。破棄すると中ごと消える。
    struct Proj {
        root: PathBuf,
        data_dir: PathBuf,
        db: Database,
    }

    impl Proj {
        fn new(tag: &str) -> Self {
            let root = std::env::temp_dir().join(format!(
                "sai-relayout-{tag}-{}-{:?}",
                std::process::id(),
                std::thread::current().id()
            ));
            let _ = fs::remove_dir_all(&root);
            let data_dir = root.join(".sai").join("paper");
            fs::create_dir_all(data_dir.join("papers")).unwrap();
            let db = Database::open(&data_dir.join("papr.db")).expect("テスト用 DB を開く");
            Self {
                root,
                data_dir,
                db,
            }
        }

        /// 論文を 1 行作る（番号・タイトル・登録 PDF）。
        fn add_paper(&self, id: i64, title: &str, pdf_path: Option<&str>) {
            self.db
                .connection()
                .execute(
                    "INSERT INTO papers (id, title, pdf_path) VALUES (?1, ?2, ?3)",
                    params![id, title, pdf_path],
                )
                .expect("論文の追加");
        }

        /// `data_dir` 相対のファイルを作る（フォルダごと作る）。
        fn write(&self, relative: &str, content: &str) -> PathBuf {
            let path = self.data_dir.join(relative);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, content).unwrap();
            path
        }

        fn plan(&self) -> Plan {
            build_plan(&self.db, &self.data_dir).expect("予定の作成")
        }

        fn apply(&self, plan: &Plan) -> Result<()> {
            execute_plan(&self.db, &self.data_dir, plan)
        }

        fn pdf_path_of(&self, id: i64) -> Option<String> {
            self.db
                .connection()
                .query_row(
                    "SELECT pdf_path FROM papers WHERE id = ?1",
                    [id],
                    |row| row.get::<_, Option<String>>(0),
                )
                .ok()
                .flatten()
        }

        /// 配下のファイルを全部挙げる（leftovers の確認用）。
        fn files_below(&self, base: &Path) -> Vec<PathBuf> {
            let mut found = Vec::new();
            let Ok(entries) = fs::read_dir(base) else {
                return found;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    found.extend(self.files_below(&path));
                } else {
                    found.push(path);
                }
            }
            found
        }
    }

    impl Drop for Proj {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    /// 誤った場所（`data_dir` 直下）の extract.md が正しい場所へ移る。
    #[test]
    fn misplaced_extract_moves_into_paper_folder() {
        let proj = Proj::new("extract");
        proj.add_paper(1, "Alpha Study", None);
        proj.write("0001_Alpha-Study-Old/extract.md", "# 抽出結果");

        let plan = proj.plan();
        let dest = proj.data_dir.join("papers/0001_Alpha-Study/extract.md");
        assert!(
            plan.ops.iter().any(|op| matches!(
                op,
                Op::MoveFile { to, .. } if *to == dest
            )),
            "移動予定に入っていること: {:?}",
            plan.ops
        );

        proj.apply(&plan).unwrap();
        assert_eq!(fs::read_to_string(&dest).unwrap(), "# 抽出結果");
        assert!(
            !proj.data_dir.join("0001_Alpha-Study-Old").exists(),
            "空になった誤った場所のフォルダは消える"
        );
    }

    /// フォルダ名が短いタイトルに変わり、DB のパスも追従する。
    #[test]
    fn folder_renamed_to_short_title_and_db_follows() {
        let proj = Proj::new("rename");
        let old_stored = "papers/0002_Long Title For Testing Rename Behavior/paper.pdf";
        proj.write(old_stored, "%PDF-1.4");
        proj.add_paper(
            2,
            "Long Title For Testing Rename Behavior",
            Some(old_stored),
        );

        let plan = proj.plan();
        let new_dir = proj
            .data_dir
            .join("papers/0002_Long-Title-For-Testing-Rename-Behavior");
        assert!(
            plan.ops.iter().any(|op| matches!(
                op,
                Op::RenameDir { to, .. } if *to == new_dir
            )),
            "フォルダ名変更の予定があること: {:?}",
            plan.ops
        );
        assert!(
            plan.db_path_updates
                .iter()
                .any(|u| u.table == "papers"
                    && u.column == "pdf_path"
                    && u.to == "papers/0002_Long-Title-For-Testing-Rename-Behavior/paper.pdf"),
            "DB の書き替え予定があること: {:?}",
            plan.db_path_updates
        );

        proj.apply(&plan).unwrap();
        assert!(new_dir.join("paper.pdf").is_file());
        assert!(!proj.data_dir.join("papers/0002_Long Title For Testing Rename Behavior").exists());
        assert_eq!(
            proj.pdf_path_of(2).unwrap(),
            "papers/0002_Long-Title-For-Testing-Rename-Behavior/paper.pdf"
        );
    }

    /// 中身が違う同名ファイルは、DB が指す方を残して leftovers へ移す。
    #[test]
    fn different_same_name_file_goes_to_leftovers() {
        let proj = Proj::new("dupe");
        proj.add_paper(3, "Same", Some("papers/0003_Same/paper.pdf"));
        proj.write("papers/0003_Same/paper.pdf", "AAA");
        proj.write("0003_Same/paper.pdf", "BBB");

        let plan = proj.plan();
        assert!(
            plan.ops.iter().any(|op| matches!(op, Op::Leftover { .. })),
            "leftovers への退避予定があること: {:?}",
            plan.ops
        );

        proj.apply(&plan).unwrap();
        assert_eq!(
            fs::read_to_string(proj.data_dir.join("papers/0003_Same/paper.pdf")).unwrap(),
            "AAA",
            "DB が指す方は残る"
        );
        assert_eq!(proj.pdf_path_of(3).unwrap(), "papers/0003_Same/paper.pdf");

        let leftovers = proj.files_below(&proj.data_dir.join("relayout-leftovers"));
        assert_eq!(leftovers.len(), 1, "leftovers: {leftovers:?}");
        assert_eq!(fs::read_to_string(&leftovers[0]).unwrap(), "BBB");
        assert!(!proj.data_dir.join("0003_Same").exists());
    }

    /// 重複した論文をまとめると、ノート・タグ・索引が残る方へ移り、消える方は残らない。
    #[test]
    fn duplicate_papers_merge_notes_tags_and_index() {
        let proj = Proj::new("merge");
        proj.add_paper(4, "Duplicate Paper One", None);
        proj.add_paper(5, "duplicate paper  one", None);
        let conn = proj.db.connection();
        conn.execute("INSERT INTO tags (name) VALUES ('t4')", [])
            .unwrap();
        let tag4 = conn.last_insert_rowid();
        conn.execute("INSERT INTO tags (name) VALUES ('t5')", [])
            .unwrap();
        let tag5 = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO paper_tags (paper_id, tag_id) VALUES (4, ?1)",
            [tag4],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO paper_tags (paper_id, tag_id) VALUES (5, ?1)",
            [tag5],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO chunks (source_kind, source_ref, paper_id, ord, text,
                                 char_start, char_end, strategy, created_at)
             VALUES ('body', 'ref-5', 5, 0, '索引された本文', 0, 7, 'paragraph', CURRENT_TIMESTAMP)",
            [],
        )
        .unwrap();

        let plan = proj.plan();
        assert_eq!(plan.merges.len(), 1, "まとめが 1 件: {:?}", plan.merges);
        assert_eq!(plan.merges[0].from, 5, "番号が大きい方を消す");
        assert_eq!(plan.merges[0].to, 4);

        proj.apply(&plan).unwrap();

        let conn = proj.db.connection();
        let left: i64 = conn
            .query_row("SELECT COUNT(*) FROM papers WHERE id = 5", [], |r| r.get(0))
            .unwrap();
        assert_eq!(left, 0, "消える論文は残らない");

        let tags_moved: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM paper_tags WHERE paper_id = 4",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(tags_moved, 2, "タグは両方とも残る方へ移る");
        let tags_left: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM paper_tags WHERE paper_id = 5",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(tags_left, 0);

        let chunks_moved: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM chunks WHERE paper_id = 4",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(chunks_moved, 1, "索引が残る方へ移る");
        let chunks_left: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM chunks WHERE paper_id = 5",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(chunks_left, 0, "消える論文に索引は残らない");
    }

    /// 翻訳 PDF が論文として登録された行は、元の論文へまとめて翻訳 PDF になる。
    #[test]
    fn translation_pdf_row_merges_into_original() {
        let proj = Proj::new("translation");
        proj.add_paper(6, "Real Paper", Some("papers/0006_Real-Paper/paper.pdf"));
        proj.write("papers/0006_Real-Paper/paper.pdf", "%PDF orig");

        // data_dir の外に置かれた翻訳 PDF（-mono.pdf が論文として登録されている）
        let mono = proj.root.join("papers").join("Real Paper-mono.pdf");
        fs::create_dir_all(mono.parent().unwrap()).unwrap();
        fs::write(&mono, "%PDF ja").unwrap();
        proj.add_paper(7, "Real Paper mono", Some(&mono.to_string_lossy()));

        proj.db
            .connection()
            .execute(
                "INSERT INTO paper_translations (paper_id, status, papers_id)
                 VALUES ('s2-key', 'ready', 6)",
                [],
            )
            .unwrap();

        let plan = proj.plan();
        assert_eq!(plan.merges.len(), 1, "{:?}", plan.merges);
        assert_eq!(plan.merges[0].from, 7);
        assert_eq!(plan.merges[0].to, 6);
        assert_eq!(plan.merges[0].reason, "翻訳 PDF の誤登録");
        assert!(plan.merges[0].translation_to.is_some(), "{:?}", plan.merges[0]);

        proj.apply(&plan).unwrap();

        assert!(!mono.exists(), "元の場所からは動く");
        let ja = proj.data_dir.join("papers/0006_Real-Paper/paper-ja.pdf");
        assert_eq!(fs::read_to_string(&ja).unwrap(), "%PDF ja");

        let conn = proj.db.connection();
        let left: i64 = conn
            .query_row("SELECT COUNT(*) FROM papers WHERE id = 7", [], |r| r.get(0))
            .unwrap();
        assert_eq!(left, 0, "誤登録の行は消える");
        let recorded: Option<String> = conn
            .query_row(
                "SELECT pdf_ja_path FROM paper_translations WHERE papers_id = 6",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            recorded.as_deref(),
            Some("papers/0006_Real-Paper/paper-ja.pdf"),
            "元の論文の翻訳 PDF として記録される"
        );
        assert_eq!(
            proj.pdf_path_of(6).unwrap(),
            "papers/0006_Real-Paper/paper.pdf",
            "元の PDF は変わらない"
        );
    }

    /// 元の PDF が無い論文と、実在しない PDF は動かさず「要確認」に出す。
    #[test]
    fn unresolvable_and_missing_pdfs_stay_as_confirm() {
        let proj = Proj::new("confirm");
        // 翻訳 PDF のみで元の論文が見つからない
        proj.add_paper(
            8,
            "Ghost Translation",
            Some("papers/0008_Ghost Translation/paper-ja.pdf"),
        );
        proj.write("papers/0008_Ghost Translation/paper-ja.pdf", "%PDF ja");
        // 登録された PDF が実在しない
        proj.add_paper(
            9,
            "Missing Pdf Paper",
            Some("papers/0009_Missing-Pdf-Paper/paper.pdf"),
        );

        let plan = proj.plan();
        assert_eq!(plan.confirm.len(), 2, "{:?}", plan.confirm);
        assert!(
            plan.confirm.iter().any(|line| line.contains("Ghost Translation")),
            "{:?}",
            plan.confirm
        );
        assert!(
            plan.confirm.iter().any(|line| line.contains("見つかりません")),
            "{:?}",
            plan.confirm
        );
        assert!(plan.ops.is_empty(), "何も動かさない: {:?}", plan.ops);
        assert!(plan.merges.is_empty());
        assert!(plan.is_noop());

        proj.apply(&plan).unwrap();
        assert!(
            proj.data_dir
                .join("papers/0008_Ghost Translation/paper-ja.pdf")
                .is_file(),
            "要確認のファイルはそのまま"
        );
        assert!(proj.pdf_path_of(9).is_some(), "実在しない PDF の行も残す");
    }

    /// 2 回目の `--apply` は何も起こさない（同じ結果になる）。
    #[test]
    fn second_apply_does_nothing() {
        let proj = Proj::new("idempotent");
        proj.add_paper(1, "Alpha Study", Some("0001_Alpha-Study-Old/paper.pdf"));
        proj.write("0001_Alpha-Study-Old/paper.pdf", "%PDF");
        proj.write("0001_Alpha-Study-Old/extract.md", "md");
        proj.add_paper(2, "Beta Research", None);
        proj.write("notes/2.md", "note");

        let plan = proj.plan();
        assert!(!plan.is_noop(), "最初は何か起きる");
        proj.apply(&plan).unwrap();

        let again = proj.plan();
        assert!(
            again.is_noop(),
            "2 回目は何も起きない: ops={:?} merges={:?} db={:?}",
            again.ops,
            again.merges,
            again.db_path_updates
        );
        assert_eq!(fs::read_to_string(proj.data_dir.join("papers/0001_Alpha-Study/paper.pdf")).unwrap(), "%PDF");
        assert_eq!(
            fs::read_to_string(proj.data_dir.join("papers/0002_Beta-Research/note.md")).unwrap(),
            "note"
        );
    }

    /// 同じ論文の抽出記録が 2 行あり、中身の違う extract.md を指していても、
    /// 2 回目の実行では何も動かない（負けた方の記録も残る方のファイルを指す）。
    #[test]
    fn conflicting_extract_rows_settle_after_one_apply() {
        let proj = Proj::new("extract-rows");
        proj.add_paper(6, "Gamma Work", None);
        proj.write("papers/0006_Gamma-Work-Old/extract.md", "AAA");
        proj.write("extracts/DOI_10.1_x/extract.md", "BBB");
        for (key, md) in [
            ("ArXiv:2401.00001", "papers/0006_Gamma-Work-Old/extract.md"),
            ("DOI:10.1/x", "extracts/DOI_10.1_x/extract.md"),
        ] {
            proj.db
                .connection()
                .execute(
                    "INSERT INTO paper_extracts (paper_id, status, md_path, papers_id)
                     VALUES (?1, 'ready', ?2, 6)",
                    params![key, md],
                )
                .unwrap();
        }

        let plan = proj.plan();
        proj.apply(&plan).unwrap();
        let again = proj.plan();
        assert!(
            again.is_noop(),
            "2 回目は何も起きない: ops={:?} db={:?}",
            again.ops,
            again.db_path_updates
        );
        let paths: Vec<String> = proj
            .db
            .connection()
            .prepare("SELECT md_path FROM paper_extracts ORDER BY paper_id")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert_eq!(
            paths,
            vec!["papers/0006_Gamma-Work/extract.md"; 2],
            "どちらの記録も残る方のファイルを指す"
        );
    }

    /// 翻訳の記録が消えたファイルを指すときは、論文フォルダの翻訳 PDF へ付け替える
    /// （paper-ja.pdf を優先、無ければ paper-dual.pdf、どちらも無ければ要確認）。
    #[test]
    fn stale_translation_records_repoint_to_folder_files() {
        let proj = Proj::new("translation-repoint");
        proj.add_paper(1, "Ja Both", None);
        proj.write("papers/0001_Ja-Both/paper-ja.pdf", "ja");
        proj.write("papers/0001_Ja-Both/paper-dual.pdf", "dual");
        proj.add_paper(2, "Dual Only", None);
        proj.write("papers/0002_Dual-Only/paper-dual.pdf", "dual");
        proj.add_paper(3, "Nothing", None);
        for id in 1..=3 {
            proj.db
                .connection()
                .execute(
                    "INSERT INTO paper_translations (paper_id, status, pdf_ja_path, papers_id)
                     VALUES (?1, 'ready', ?2, ?3)",
                    params![format!("key{id}"), format!("translations/gone{id}.pdf"), id],
                )
                .unwrap();
        }

        let plan = proj.plan();
        assert_eq!(plan.repoints.len(), 2, "付け替え: {:?}", plan.repoints);
        assert!(
            plan.confirm.iter().any(|line| line.contains('3')),
            "翻訳 PDF の無い論文 3 は要確認: {:?}",
            plan.confirm
        );
        proj.apply(&plan).unwrap();

        let ja_of = |id: i64| -> Option<String> {
            proj.db
                .connection()
                .query_row(
                    "SELECT pdf_ja_path FROM paper_translations WHERE papers_id = ?1",
                    [id],
                    |row| row.get(0),
                )
                .unwrap()
        };
        assert_eq!(ja_of(1).unwrap(), "papers/0001_Ja-Both/paper-ja.pdf");
        assert_eq!(ja_of(2).unwrap(), "papers/0002_Dual-Only/paper-dual.pdf");
        assert_eq!(ja_of(3).unwrap(), "translations/gone3.pdf", "無いものは触らない");
        assert!(proj.plan().repoints.is_empty(), "2 回目は付け替え無し");
    }

    /// 途中で失敗したら、動かしたファイルも DB も元のまま戻る。
    #[test]
    fn failure_restores_files_and_keeps_db() {
        let proj = Proj::new("rollback");
        // 10 番: 先に処理される（成功する）
        proj.add_paper(10, "First Move", Some("0010_First-Move/paper.pdf"));
        proj.write("0010_First-Move/paper.pdf", "%PDF first");
        // 11 番: 移動先に同名のディレクトリを置き、移動を失敗させる
        proj.add_paper(11, "Blocked Move", None);
        proj.write("0011_Blocked-Move/paper.pdf", "%PDF blocked");
        fs::create_dir_all(proj.data_dir.join("papers/0011_Blocked-Move/paper.pdf")).unwrap();

        let plan = proj.plan();
        assert!(
            plan.ops.iter().filter(|op| matches!(op, Op::MoveFile { .. })).count() == 2,
            "2 つとも動かす予定: {:?}",
            plan.ops
        );
        let error = proj.apply(&plan).expect_err("途中で失敗すること");
        let message = format!("{error:#}");
        assert!(message.contains("元に戻し") || message.contains("失敗"), "{message}");

        assert_eq!(
            fs::read_to_string(proj.data_dir.join("0010_First-Move/paper.pdf")).unwrap(),
            "%PDF first",
            "先に動かしたファイルは元の場所に戻る"
        );
        assert!(
            !proj.data_dir.join("papers/0010_First-Move/paper.pdf").exists(),
            "移動先には残らない"
        );
        assert_eq!(
            proj.pdf_path_of(10).unwrap(),
            "0010_First-Move/paper.pdf",
            "DB は変更されない"
        );
        assert_eq!(
            fs::read_to_string(proj.data_dir.join("0011_Blocked-Move/paper.pdf")).unwrap(),
            "%PDF blocked"
        );
    }
}
