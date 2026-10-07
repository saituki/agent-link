//! 図表（Figure / Table）の抽出と保存。
//!
//! 流れ:
//! 1. docling を画像出力モード（`--image-export-mode referenced`）で実行する
//! 2. 出力 JSON の `pictures` から図（キャプション付き）を取り出す
//! 3. 画像を `<paper_dir>/figures/fig-NNN.png` にコピーする
//! 4. 本文から図への言及文を探して `paper_figures` に保存する
//!
//! 図抽出は download / import 直後にバックグラウンドで走り、引用抽出は `s` キーで走る。
//! docling は 1 本につき 1 回にする（段階 5）: 図抽出が先なら `extract.md` もここで作り、
//! あとの `s` と Radio がそれを使う。`s` が先に `extract.md` を作っていれば上書きしない。

use std::path::Path;

use chrono::Utc;
use rusqlite::{Connection, OptionalExtension, params};
use thiserror::Error;

use crate::citation::docling;
use crate::citation::sentences;

/// 図抽出のエラー。
#[derive(Debug, Error)]
pub enum FigureError {
    /// SQLite 操作の失敗
    #[error("database error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    /// ファイル読み書きの失敗
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    /// docling CLI の実行失敗
    #[error("docling error: {0}")]
    Docling(String),
    /// 論文または PDF が見つからない
    #[error("not found: {0}")]
    NotFound(String),
}

/// `paper_figures` の 1 行（図 1 件）。
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct FigureRow {
    /// 抽出キー（S2 40hex / `ArXiv:...` / `DOI:...`）
    pub paper_id: String,
    /// `papers(id)`（NULL 許容）
    pub papers_id: Option<i64>,
    /// 1 起算の図番号
    pub fig_no: i64,
    /// 図ラベル（例: `Figure 1`）
    pub label: Option<String>,
    /// 図のタイトル（docling から取得できない場合は NULL）
    pub title: Option<String>,
    /// ラベルを除いたキャプション本文
    pub caption: Option<String>,
    /// 画像ファイルの絶対パス
    pub image_path: String,
    /// 本文中の言及文（見つからない場合は NULL）
    pub mention: Option<String>,
    /// 掲載ページ
    pub page: Option<i64>,
    /// 記録時刻（ISO8601）
    pub created_at: String,
}

/// `paper_figure_extracts` の 1 行（抽出状態）。
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct FigureExtractRow {
    /// 抽出キー
    pub paper_id: String,
    /// `papers(id)`（NULL 許容）
    pub papers_id: Option<i64>,
    /// parsing / ready / empty / failed
    pub status: String,
    /// 入力 PDF パス
    pub pdf_path: Option<String>,
    /// PDF のエポック秒（stale 判定用）
    pub pdf_mtime: Option<String>,
    /// 抽出した図の数
    pub n_figures: Option<i64>,
    /// 失敗時のエラー
    pub error: Option<String>,
    /// 最終抽出日時（ISO8601）
    pub extracted_at: Option<String>,
}

/// TUI の PAPER 画面が必要とする図の表示状態。
#[derive(Debug, Clone)]
pub enum FigureState {
    /// 未抽出（または抽出中・失敗）。画面は「(図は未抽出)」を出す。
    NotExtracted,
    /// 抽出済みで図がある。
    Ready(Vec<FigureRow>),
    /// 抽出済みだが図が 0 件。FIGURES セクション自体を出さない。
    Empty,
}

const FIGURE_COLUMNS: &str =
    "paper_id, papers_id, fig_no, label, title, caption, image_path, mention, page, created_at";

/// 図一覧を取得する（fig_no 昇順）。
///
/// # Errors
///
/// SQLite エラー。
pub fn get_figures(conn: &Connection, paper_id: &str) -> Result<Vec<FigureRow>, FigureError> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {FIGURE_COLUMNS} FROM paper_figures WHERE paper_id = ?1 ORDER BY fig_no"
    ))?;
    let rows = stmt
        .query_map(params![paper_id], map_figure_row)?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

fn map_figure_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<FigureRow> {
    Ok(FigureRow {
        paper_id: row.get(0)?,
        papers_id: row.get(1)?,
        fig_no: row.get(2)?,
        label: row.get(3)?,
        title: row.get(4)?,
        caption: row.get(5)?,
        image_path: row.get(6)?,
        mention: row.get(7)?,
        page: row.get(8)?,
        created_at: row.get(9)?,
    })
}

/// 図を一括置換する（既存行を削除してから挿入）。
///
/// `papers_id` が分かっているときは、同じ論文で別の抽出キーに残っている古い行も
/// 消す（抽出キーが変わった論文で行が二重にならないように。段階 2c）。
/// 挿入する行の `papers_id` もこの引数の値で揃える。
///
/// # Errors
///
/// SQLite エラー。
pub fn replace_figures(
    conn: &Connection,
    paper_id: &str,
    papers_id: Option<i64>,
    rows: &[FigureRow],
) -> Result<(), FigureError> {
    let tx = conn.unchecked_transaction()?;
    tx.execute(
        "DELETE FROM paper_figures WHERE paper_id = ?1",
        params![paper_id],
    )?;
    if let Some(papers_id) = papers_id {
        tx.execute(
            "DELETE FROM paper_figures WHERE papers_id = ?1 AND paper_id <> ?2",
            params![papers_id, paper_id],
        )?;
    }
    {
        let mut stmt = tx.prepare(&format!(
            "INSERT INTO paper_figures ({FIGURE_COLUMNS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)"
        ))?;
        for r in rows {
            stmt.execute(params![
                r.paper_id,
                papers_id,
                r.fig_no,
                r.label,
                r.title,
                r.caption,
                r.image_path,
                r.mention,
                r.page,
                r.created_at,
            ])?;
        }
    }
    tx.commit()?;
    Ok(())
}

/// 図一覧を、まず `papers_id` で、無ければ抽出キーで取得する（段階 2c）。
///
/// 探す順序は引用と同じ 3 段階（`papers_id` + 現在のキー → `papers_id` → キー）。
///
/// # Errors
///
/// SQLite エラー。
pub fn get_figures_for(
    conn: &Connection,
    papers_id: Option<i64>,
    key: Option<&str>,
) -> Result<Vec<FigureRow>, FigureError> {
    if let Some(papers_id) = papers_id {
        if let Some(key) = key {
            let mut stmt = conn.prepare(&format!(
                "SELECT {FIGURE_COLUMNS} FROM paper_figures
                 WHERE papers_id = ?1 AND paper_id = ?2 ORDER BY fig_no"
            ))?;
            let rows = stmt
                .query_map(params![papers_id, key], map_figure_row)?
                .collect::<Result<Vec<_>, _>>()?;
            if !rows.is_empty() {
                return Ok(rows);
            }
        }
        let mut stmt = conn.prepare(&format!(
            "SELECT {FIGURE_COLUMNS} FROM paper_figures WHERE papers_id = ?1 ORDER BY fig_no"
        ))?;
        let rows = stmt
            .query_map(params![papers_id], map_figure_row)?
            .collect::<Result<Vec<_>, _>>()?;
        if !rows.is_empty() {
            return Ok(rows);
        }
    }
    match key {
        Some(key) => get_figures(conn, key),
        None => Ok(Vec::new()),
    }
}

/// 抽出状態を取得する。
///
/// # Errors
///
/// SQLite エラー。
pub fn get_figure_extract(
    conn: &Connection,
    paper_id: &str,
) -> Result<Option<FigureExtractRow>, FigureError> {
    let mut stmt = conn.prepare(
        "SELECT paper_id, papers_id, status, pdf_path, pdf_mtime, n_figures, error, extracted_at
         FROM paper_figure_extracts WHERE paper_id = ?1",
    )?;
    let row = stmt
        .query_row(params![paper_id], map_figure_extract_row)
        .optional()?;
    Ok(row)
}

/// 抽出状態を upsert する。
///
/// `papers_id` が入っている行は、同じ論文で別の抽出キーに残っている古い行を
/// 消してから書く（段階 2c）。
///
/// # Errors
///
/// SQLite エラー。
pub fn upsert_figure_extract(conn: &Connection, row: &FigureExtractRow) -> Result<(), FigureError> {
    let tx = conn.unchecked_transaction()?;
    if let Some(papers_id) = row.papers_id {
        tx.execute(
            "DELETE FROM paper_figure_extracts WHERE papers_id = ?1 AND paper_id <> ?2",
            params![papers_id, row.paper_id],
        )?;
    }
    tx.execute(
        "INSERT INTO paper_figure_extracts
           (paper_id, papers_id, status, pdf_path, pdf_mtime, n_figures, error, extracted_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
         ON CONFLICT(paper_id) DO UPDATE SET
           papers_id = COALESCE(excluded.papers_id, paper_figure_extracts.papers_id),
           status = excluded.status,
           pdf_path = excluded.pdf_path, pdf_mtime = excluded.pdf_mtime,
           n_figures = excluded.n_figures, error = excluded.error,
           extracted_at = excluded.extracted_at",
        params![
            row.paper_id,
            row.papers_id,
            row.status,
            row.pdf_path,
            row.pdf_mtime,
            row.n_figures,
            row.error,
            row.extracted_at,
        ],
    )?;
    tx.commit()?;
    Ok(())
}

/// 抽出状態を、まず `papers_id` で、無ければ抽出キーで取得する（段階 2c）。
///
/// 引用と同じ 3 段階（`papers_id` + 現在のキー → `papers_id` → キー）。
///
/// # Errors
///
/// SQLite エラー。
pub fn get_figure_extract_for(
    conn: &Connection,
    papers_id: Option<i64>,
    key: Option<&str>,
) -> Result<Option<FigureExtractRow>, FigureError> {
    const SELECT: &str =
        "SELECT paper_id, papers_id, status, pdf_path, pdf_mtime, n_figures, error, extracted_at
             FROM paper_figure_extracts";
    if let Some(papers_id) = papers_id {
        if let Some(key) = key {
            let mut stmt =
                conn.prepare(&format!("{SELECT} WHERE papers_id = ?1 AND paper_id = ?2"))?;
            let row = stmt
                .query_row(params![papers_id, key], map_figure_extract_row)
                .optional()?;
            if row.is_some() {
                return Ok(row);
            }
        }
        let mut stmt = conn.prepare(&format!("{SELECT} WHERE papers_id = ?1"))?;
        let row = stmt
            .query_row(params![papers_id], map_figure_extract_row)
            .optional()?;
        if row.is_some() {
            return Ok(row);
        }
    }
    match key {
        Some(key) => get_figure_extract(conn, key),
        None => Ok(None),
    }
}

/// `paper_figure_extracts` の行をマップする（取得関数の共通部）。
fn map_figure_extract_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<FigureExtractRow> {
    Ok(FigureExtractRow {
        paper_id: row.get(0)?,
        papers_id: row.get(1)?,
        status: row.get(2)?,
        pdf_path: row.get(3)?,
        pdf_mtime: row.get(4)?,
        n_figures: row.get(5)?,
        error: row.get(6)?,
        extracted_at: row.get(7)?,
    })
}

/// `papers.id` から抽出キーを解決する（`s2_paper_id` → `arxiv_id` → `doi` の優先順）。
///
/// # Errors
///
/// SQLite エラー。
pub fn extract_key_for_paper(
    conn: &Connection,
    papers_id: i64,
) -> Result<Option<String>, FigureError> {
    let key: Option<Option<String>> = conn
        .query_row(
            "SELECT s2_paper_id, arxiv_id, doi FROM papers WHERE id = ?1",
            [papers_id],
            |row| {
                let s2: Option<String> = row.get(0)?;
                let arxiv: Option<String> = row.get(1)?;
                let doi: Option<String> = row.get(2)?;
                Ok(s2.or(arxiv).or(doi))
            },
        )
        .optional()?;
    Ok(key
        .flatten()
        .map(|k| crate::citation::normalize_paper_id(&k))
        .filter(|k| !k.trim().is_empty()))
}

/// 図の抽出に使うキー。`extract_key_for_paper` が無ければ `paper:<papers.id>`。
/// 論文の識別子（s2・arXiv・DOI）が無くても図を抽出できるようにする。
///
/// extract.md を探すキーには使わない（`paper:<id>` を混ぜると探索が壊れる）。
///
/// # Errors
///
/// SQLite エラー。
pub fn figure_key_for_paper(conn: &Connection, papers_id: i64) -> Result<String, FigureError> {
    Ok(extract_key_for_paper(conn, papers_id)?.unwrap_or_else(|| format!("paper:{papers_id}")))
}

/// PAPER 画面が必要とする図の表示状態を返す。
///
/// 「未抽出」と「抽出済みだが 0 件」を区別する。
///
/// 記録はまず `papers_id` で、無ければ抽出キーで探す（抽出キーが変わっても
/// 見えるように。段階 2c）。
///
/// `data_dir`（`.sai/paper/`）は `paper_figures.image_path` の相対パスを
/// 実ファイルパスへ直すために使う（段階 2b）。
///
/// # Errors
///
/// SQLite エラー。
pub fn figure_state_for_paper(
    conn: &Connection,
    papers_id: i64,
    data_dir: &Path,
) -> Result<FigureState, FigureError> {
    let key = extract_key_for_paper(conn, papers_id)?;
    let Some(extract) = get_figure_extract_for(conn, Some(papers_id), key.as_deref())? else {
        return Ok(FigureState::NotExtracted);
    };
    match extract.status.as_str() {
        "ready" => {
            // 見つかった行のキー（抽出キーが変わっても記録側の実キー）で図も読む。
            let figures = get_figures_for(conn, Some(papers_id), Some(&extract.paper_id))?;
            // 画像ファイルが消えている行は表示しない（手動削除や移動に追従）。
            // image_path は DB では data_dir からの相対なので、実パスへ直してから判定する。
            let alive: Vec<FigureRow> = figures
                .into_iter()
                .map(|mut figure| {
                    figure.image_path =
                        crate::paths::resolve_stored_path(data_dir, &figure.image_path)
                            .to_string_lossy()
                            .into_owned();
                    figure
                })
                .filter(|f| Path::new(&f.image_path).is_file())
                .collect();
            if alive.is_empty() {
                Ok(FigureState::Empty)
            } else {
                Ok(FigureState::Ready(alive))
            }
        }
        "empty" => Ok(FigureState::Empty),
        // parsing / failed / 未知は「未抽出」として扱う（実行中はトーストで表示される）。
        _ => Ok(FigureState::NotExtracted),
    }
}

/// PDF のエポック秒（stale 判定用）を返す。
fn pdf_mtime_secs(pdf_path: &Path) -> Result<String, FigureError> {
    Ok(std::fs::metadata(pdf_path)?
        .modified()?
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| FigureError::Io(std::io::Error::other("invalid PDF mtime")))?
        .as_secs()
        .to_string())
}

/// 指定論文の図を抽出する（メインエントリ）。
///
/// # 引数
/// - `conn`: papr.db コネクション
/// - `key`: 抽出キー（正規化済み）
/// - `papers_id`: `papers.id`（`paper_figures.papers_id` に記録する）
/// - `pdf_path`: 入力 PDF
/// - `paper_dir`: 論文フォルダ（`figures/` と `extract.md` の親）
/// - `data_dir`: `.sai/paper/`（DB へのパス保存に使う。段階 2b）
/// - `force`: 既に抽出済みでも再実行する
///
/// # 戻り値
/// 抽出した図の数。
///
/// # Errors
///
/// docling 実行失敗・I/O 失敗・SQLite エラー。失敗時は `status='failed'` を記録する。
pub async fn extract_figures(
    conn: &Connection,
    key: &str,
    papers_id: Option<i64>,
    pdf_path: &Path,
    paper_dir: &Path,
    data_dir: &Path,
    force: bool,
) -> Result<usize, FigureError> {
    if !pdf_path.is_file() {
        return Err(FigureError::NotFound(format!(
            "PDF not found: {}",
            pdf_path.display()
        )));
    }
    let pdf_mtime = pdf_mtime_secs(pdf_path)?;
    // DB に書く/比べるパスは data_dir からの相対（段階 2b）。
    let pdf_stored = crate::paths::to_stored_path(data_dir, pdf_path);

    // キャッシュヒット（同じ PDF で既に ready / empty）なら再実行しない。
    // 記録はまず papers_id で、無ければ抽出キーで探す（段階 2c）。
    if !force
        && let Some(existing) = get_figure_extract_for(conn, papers_id, Some(key))?
        && matches!(existing.status.as_str(), "ready" | "empty")
        && existing.pdf_path.as_deref() == Some(pdf_stored.as_str())
        && existing.pdf_mtime.as_deref() == Some(pdf_mtime.as_str())
    {
        return Ok(existing.n_figures.unwrap_or(0).max(0) as usize);
    }

    std::fs::create_dir_all(paper_dir)?;
    let extract_md_path = paper_dir.join(crate::paths::paper_file_names::EXTRACT);
    let tmp_dir = paper_dir.join(".docling-figures-tmp");
    let _ = std::fs::remove_dir_all(&tmp_dir);
    std::fs::create_dir_all(&tmp_dir)?;

    upsert_figure_extract(
        conn,
        &FigureExtractRow {
            paper_id: key.to_string(),
            papers_id,
            status: "parsing".to_string(),
            pdf_path: Some(pdf_stored.clone()),
            pdf_mtime: Some(pdf_mtime.clone()),
            extracted_at: Some(Utc::now().to_rfc3339()),
            ..Default::default()
        },
    )?;

    let result = run_figure_pipeline(
        conn,
        key,
        papers_id,
        pdf_path,
        paper_dir,
        data_dir,
        &tmp_dir,
        &extract_md_path,
    )
    .await;

    // 一時ディレクトリは成功・失敗どちらでも後始末する。
    let _ = std::fs::remove_dir_all(&tmp_dir);

    match result {
        Ok(count) => {
            let status = if count == 0 { "empty" } else { "ready" };
            upsert_figure_extract(
                conn,
                &FigureExtractRow {
                    paper_id: key.to_string(),
                    papers_id,
                    status: status.to_string(),
                    pdf_path: Some(pdf_stored),
                    pdf_mtime: Some(pdf_mtime),
                    n_figures: Some(count as i64),
                    error: None,
                    extracted_at: Some(Utc::now().to_rfc3339()),
                },
            )?;
            Ok(count)
        }
        Err(e) => {
            upsert_figure_extract(
                conn,
                &FigureExtractRow {
                    paper_id: key.to_string(),
                    papers_id,
                    status: "failed".to_string(),
                    pdf_path: Some(pdf_stored),
                    pdf_mtime: Some(pdf_mtime),
                    n_figures: Some(0),
                    error: Some(e.to_string()),
                    extracted_at: Some(Utc::now().to_rfc3339()),
                },
            )?;
            Err(e)
        }
    }
}

/// docling 実行から保存までの本体。
async fn run_figure_pipeline(
    conn: &Connection,
    key: &str,
    papers_id: Option<i64>,
    pdf_path: &Path,
    paper_dir: &Path,
    data_dir: &Path,
    tmp_dir: &Path,
    extract_md_path: &Path,
) -> Result<usize, FigureError> {
    // 先に `s`（引用抽出）が extract.md を作っていれば上書きしない（画像だけ取る）。
    // 無ければここで作り、あとの `s` と Radio が使う（docling は 1 本につき 1 回。段階 5）。
    let md_target = if extract_md_path.is_file() {
        tmp_dir.join("extract.md")
    } else {
        extract_md_path.to_path_buf()
    };
    let output = docling::run_docling_with_images(
        pdf_path.to_path_buf(),
        tmp_dir.to_path_buf(),
        md_target,
    )
    .await
    .map_err(|e| FigureError::Docling(e.to_string()))?;

    let figures = output.figures;
    if figures.is_empty() {
        replace_figures(conn, key, papers_id, &[])?;
        return Ok(0);
    }

    let figures_dir = crate::paths::paper_figures_dir(paper_dir);
    std::fs::create_dir_all(&figures_dir)?;
    // 再抽出時に古い画像を残さないよう、figures 配下の既存 PNG を消してから書く。
    if let Ok(entries) = std::fs::read_dir(&figures_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|ext| ext == "png") {
                let _ = std::fs::remove_file(path);
            }
        }
    }

    let now = Utc::now().to_rfc3339();
    let mut rows = Vec::with_capacity(figures.len());
    for (index, figure) in figures.iter().enumerate() {
        let fig_no = index as i64 + 1;
        let dest = crate::paths::paper_figure_path(paper_dir, fig_no);
        std::fs::copy(&figure.image_path, &dest)?;
        let mention = figure
            .label
            .as_deref()
            .and_then(|label| sentences::find_mention(&output.markdown, label));
        rows.push(FigureRow {
            paper_id: key.to_string(),
            papers_id,
            fig_no,
            label: figure.label.clone(),
            title: None,
            caption: figure.caption.clone(),
            image_path: dest.to_string_lossy().to_string(),
            mention,
            page: figure.page,
            created_at: now.clone(),
        });
    }

    // 画像の場所も DB には data_dir からの相対で書く（段階 2b）。
    for row in &mut rows {
        row.image_path = crate::paths::to_stored_path(data_dir, &row.image_path);
    }
    replace_figures(conn, key, papers_id, &rows)?;
    Ok(rows.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE papers (id INTEGER PRIMARY KEY, title TEXT, doi TEXT, arxiv_id TEXT, s2_paper_id TEXT);",
        )
        .unwrap();
        conn.execute_batch(include_str!("../migrations/0027_paper_figures.sql"))
            .unwrap();
        conn
    }

    #[test]
    fn figure_state_distinguishes_missing_and_empty() {
        let conn = setup();
        conn.execute(
            "INSERT INTO papers (id, title, arxiv_id) VALUES (1, 'A', '2401.00001')",
            [],
        )
        .unwrap();
        // 状態行が無い → 未抽出
        assert!(matches!(
            figure_state_for_paper(&conn, 1, std::path::Path::new("")).unwrap(),
            FigureState::NotExtracted
        ));

        // ready だが 0 件 → Empty
        upsert_figure_extract(
            &conn,
            &FigureExtractRow {
                paper_id: "ArXiv:2401.00001".to_string(),
                papers_id: Some(1),
                status: "ready".to_string(),
                n_figures: Some(0),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(matches!(
            figure_state_for_paper(&conn, 1, std::path::Path::new("")).unwrap(),
            FigureState::Empty
        ));

        // failed は未抽出扱い
        upsert_figure_extract(
            &conn,
            &FigureExtractRow {
                paper_id: "ArXiv:2401.00001".to_string(),
                papers_id: Some(1),
                status: "failed".to_string(),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(matches!(
            figure_state_for_paper(&conn, 1, std::path::Path::new("")).unwrap(),
            FigureState::NotExtracted
        ));
    }

    #[test]
    fn extract_key_prefers_s2_then_arxiv_then_doi() {
        let conn = setup();
        conn.execute(
            "INSERT INTO papers (id, title, arxiv_id, doi) VALUES (1, 'A', '2401.00001', '10.1/x')",
            [],
        )
        .unwrap();
        assert_eq!(
            extract_key_for_paper(&conn, 1).unwrap().as_deref(),
            Some("ArXiv:2401.00001")
        );
        conn.execute(
            "UPDATE papers SET s2_paper_id = 'ff312b6027d012403760c288fd6531145006cb9f' WHERE id = 1",
            [],
        )
        .unwrap();
        assert_eq!(
            extract_key_for_paper(&conn, 1).unwrap().as_deref(),
            Some("ff312b6027d012403760c288fd6531145006cb9f")
        );
    }

    #[test]
    fn figure_key_falls_back_to_papers_id() {
        let conn = setup();
        conn.execute("INSERT INTO papers (id, title) VALUES (7, 'No ids')", []).unwrap();
        assert_eq!(extract_key_for_paper(&conn, 7).unwrap(), None);
        assert_eq!(figure_key_for_paper(&conn, 7).unwrap(), "paper:7");
        conn.execute("UPDATE papers SET arxiv_id = '2401.00001' WHERE id = 7", []).unwrap();
        assert_eq!(figure_key_for_paper(&conn, 7).unwrap(), "ArXiv:2401.00001");
    }

    #[test]
    fn figures_roundtrip() {
        let conn = setup();
        conn.execute(
            "INSERT INTO papers (id, title, arxiv_id) VALUES (1, 'A', '2401.00001')",
            [],
        )
        .unwrap();
        let rows = vec![
            FigureRow {
                paper_id: "ArXiv:2401.00001".into(),
                papers_id: Some(1),
                fig_no: 1,
                label: Some("Figure 1".into()),
                caption: Some("Workflow.".into()),
                image_path: "/tmp/fig-001.png".into(),
                mention: Some("As shown in Figure 1, ...".into()),
                page: Some(3),
                created_at: "2026-01-01T00:00:00Z".into(),
                ..Default::default()
            },
            FigureRow {
                paper_id: "ArXiv:2401.00001".into(),
                fig_no: 2,
                ..Default::default()
            },
        ];
        replace_figures(&conn, "ArXiv:2401.00001", Some(1), &rows).unwrap();
        let loaded = get_figures(&conn, "ArXiv:2401.00001").unwrap();
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[0].page, Some(3));
        assert_eq!(loaded[0].label.as_deref(), Some("Figure 1"));
        // 書いた行には papers_id が入っている（段階 2c）。
        assert_eq!(loaded[0].papers_id, Some(1));
        assert_eq!(loaded[1].papers_id, Some(1));
        // 再実行で重複しない
        replace_figures(&conn, "ArXiv:2401.00001", Some(1), &rows).unwrap();
        assert_eq!(get_figures(&conn, "ArXiv:2401.00001").unwrap().len(), 2);
    }
}
