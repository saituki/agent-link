//! 引用文献抽出パイプライン。
//!
//! 処理フロー（`ensure_extract`）:
//! 1. `paper_id` を正規化
//! 2. stale 判定（pdf_path + pdf_mtime + status='ready' が一致すればキャッシュヒット）
//! 3. S2 references 取得（失敗しても抽出は続行。docling のみで構成）
//! 4. docling → Markdown
//! 5. References 分割・パース → raw_refs
//! 6. S2 refs と word検索 → Dice類似度フォールバックでマッチング
//!    （s2_ref は docling_ref のメタデータ補強のみ。リストへの追加はしない）
//! 7. 本文中引用文抽出
//! 8. DB 保存（paper_refs / paper_sentences / paper_extracts）
//!
//! 失敗時は `status='failed'` + エラー記録を DB に残してからエラーを返す。

pub mod db;
pub mod docling;
pub mod library;
pub mod matcher;
pub mod parsers;
pub mod sentences;

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use chrono::Utc;
use rusqlite::Connection;
use thiserror::Error;

use crate::api::s2::{S2Client, S2Error, S2PaperDetail};

use db::{ExtractRow, RefRow};
use docling::DoclingOutput;

/// 抽出結果。
#[derive(Debug)]
pub struct ExtractOutput {
    /// 引用文献リスト（ref_no 昇順）
    pub refs: Vec<RefRow>,
    /// `(本文, 引用 ref_no リスト)`
    pub sentences: Vec<(String, Vec<i64>)>,
    /// ジャーナルスタイル（統合優先のため常に "Generic"）
    pub style: String,
}

/// 抽出エラー。
#[derive(Debug, Error)]
pub enum ExtractError {
    /// 引用テーブル操作の失敗
    #[error("database error: {0}")]
    Db(#[from] CitationDbError),
    /// docling CLI の実行失敗（未インストール / 異常終了）
    #[error("docling error: {0}")]
    Docling(String),
    /// ファイル読み書きの失敗
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    /// S2 API の失敗（レート制限・404 等）
    #[error("S2 API error: {0}")]
    S2(#[from] S2Error),
    /// 論文が見つからない
    #[error("paper not found: {0}")]
    NotFound(String),
}

/// 引用テーブル操作のエラー（CRUD 層）。
#[derive(Debug, Error)]
pub enum CitationDbError {
    /// SQLite 操作の失敗
    #[error("database error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    /// JSON エンコード / デコードの失敗
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
}

/// 抽出キーが S2 40hex かどうかを判定する。
#[must_use]
pub fn is_s2_40hex(id: &str) -> bool {
    id.len() == 40 && id.chars().all(|c| c.is_ascii_hexdigit())
}

/// paper_id を正規化する。
///
/// | 入力 | 出力 |
/// |---|---|
/// | `ff312b60...` (40hex) | 40hex（小文字化） |
/// | `ArXiv:ff312b60...` (ArXiv: + 40hex) | 40hex（prefix 剥離） |
/// | arXiv URL (`https://arxiv.org/abs/2401.12345v2`) | `ArXiv:2401.12345` |
/// | `ArXiv:1706.03762` / bare arXiv ID | `ArXiv:1706.03762` |
/// | `DOI:10.1038/...` / bare DOI | `DOI:10.1038/...` |
/// | その他 | そのまま |
#[must_use]
pub fn normalize_paper_id(raw: &str) -> String {
    let s = raw.trim();

    // arXiv URL → ArXiv:<id>
    if let Some(pos) = s.find("arxiv.org/abs/") {
        let tail = &s[pos + "arxiv.org/abs/".len()..];
        let id = tail
            .split('v')
            .next()
            .unwrap_or(tail)
            .split('?')
            .next()
            .unwrap_or(tail)
            .trim_matches('/')
            .trim();
        return format!("ArXiv:{id}");
    }

    // DOI URL / bare DOI → DOI:<...>
    if let Some(pos) = s.find("doi.org/") {
        let doi = s[pos + "doi.org/".len()..].trim_matches('/').trim();
        return format!("DOI:{doi}");
    }
    if s.starts_with("10.") && s.contains('/') {
        return format!("DOI:{s}");
    }
    if let Some(rest) = s.strip_prefix("DOI:") {
        return format!("DOI:{}", rest.trim());
    }

    // ArXiv: prefix
    if let Some(rest) = s
        .strip_prefix("ArXiv:")
        .or_else(|| s.strip_prefix("arXiv:"))
    {
        let rest = rest.trim();
        if is_s2_40hex(rest) {
            return rest.to_lowercase(); // ArXiv: + 40hex → prefix 剥離
        }
        return format!("ArXiv:{rest}");
    }

    // bare arXiv ID（例: 1706.03762）
    if s.chars().next().is_some_and(|c| c.is_ascii_digit()) && s.contains('.') && s.len() < 32 {
        return format!("ArXiv:{s}");
    }

    // 40hex はそのまま（小文字化）
    if is_s2_40hex(s) {
        return s.to_lowercase();
    }

    s.to_string()
}

/// 指定論文の引用文献抽出を実行する（メインエントリ）。
///
/// # 引数
/// - `conn`: papr.db コネクション（呼び出し側が所有するものを渡す）
/// - `s2_client`: S2 API クライアント
/// - `paper_id`: 生の ID（内部で `normalize_paper_id` により正規化）
/// - `pdf_path`: PDF ファイルパス
/// - `data_dir`: data/ ディレクトリ（`extracts/` の親）
pub async fn ensure_extract(
    conn: &Connection,
    s2_client: &S2Client,
    paper_id: &str,
    pdf_path: &Path,
    data_dir: &Path,
) -> Result<ExtractOutput, ExtractError> {
    ensure_extract_with(
        conn,
        paper_id,
        pdf_path,
        data_dir,
        |id| async move { s2_client.get_paper_references(&id).await },
        docling::run_docling,
    )
    .await
}

/// `ensure_extract` の本体。S2 取得と docling 実行を注入可能にしたもの
/// （テストでモックする）。
async fn ensure_extract_with<G, Gfut, F, Fut>(
    conn: &Connection,
    paper_id: &str,
    pdf_path: &Path,
    data_dir: &Path,
    fetch_s2_refs: G,
    run_docling_fn: F,
) -> Result<ExtractOutput, ExtractError>
where
    G: FnOnce(String) -> Gfut,
    Gfut: std::future::Future<Output = Result<Vec<S2PaperDetail>, S2Error>>,
    F: FnOnce(PathBuf, PathBuf, PathBuf) -> Fut,
    Fut: std::future::Future<Output = Result<DoclingOutput, ExtractError>>,
{
    let normalized_id = normalize_paper_id(paper_id);
    // Library 登録済みなら対応する Library の番号を先に決めておく（段階 2c）。
    // 書き込む行の `papers_id` に使い、読むときの引き当てにも使う。
    let papers_id = crate::database::Database::papers_id_for_extract_key(conn, &normalized_id);

    // PDF の最終更新時刻（stale 判定用）
    let pdf_mtime = std::fs::metadata(pdf_path)?
        .modified()?
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| ExtractError::Io(std::io::Error::other("invalid PDF mtime")))?
        .as_secs()
        .to_string();
    // DB に書く/比べるパスは data_dir からの相対（段階 2b）。
    let pdf_stored = crate::paths::to_stored_path(data_dir, pdf_path);
    let extracts_dir = data_dir.join("extracts");

    // 1. stale 判定（キャッシュヒットなら DB の結果を返す）
    //    比較も DB 保存形（相対パス）で行う。記録はまず papers_id で、無ければ
    //    抽出キーで探す（抽出キーが変わっても過去の記録が見えるように。段階 2c）。
    {
        if let Some(existing) = db::get_extract_for(conn, papers_id, Some(&normalized_id))?
            && existing.status == "ready"
            && existing.pdf_path.as_deref() == Some(pdf_stored.as_str())
            && existing.pdf_mtime.as_deref() == Some(pdf_mtime.as_str())
        {
            let refs = db::get_refs_for(conn, papers_id, Some(&normalized_id))?;
            let sentences = db::get_sentences_for(conn, papers_id, Some(&normalized_id))?
                .into_iter()
                .map(|s| {
                    let cites: Vec<i64> = serde_json::from_str(&s.cite_refs).unwrap_or_default();
                    (s.text, cites)
                })
                .collect();
            return Ok(ExtractOutput {
                refs,
                sentences,
                style: existing.style.unwrap_or_else(|| "Generic".to_string()),
            });
        }
    }

    // 2. S2 references 取得（失敗しても docling のみで続行）
    let s2_refs: Vec<S2PaperDetail> = match fetch_s2_refs(normalized_id.clone()).await {
        Ok(refs) => refs,
        Err(e) => {
            // 404 / レート制限等は警告扱い。空リストで続行する。
            eprintln!("sai-paper-core: S2 references unavailable for {normalized_id}: {e}");
            Vec::new()
        }
    };

    // 3. status=parsing を記録
    // Library 登録済み（papers_id が分かる）なら論文フォルダ（paths::paper_folder）に書く。
    // 無ければ data_dir/extracts/ に書く（登録前・プロジェクト外の抽出）。
    let registered: Option<(i64, String)> = match papers_id {
        Some(pid) => {
            let title: String = conn
                .query_row(
                    "SELECT COALESCE(title, '') FROM papers WHERE id = ?1",
                    [pid],
                    |row| row.get(0),
                )
                .unwrap_or_default();
            Some((pid, title))
        }
        None => db::get_paper_meta_by_extract_key(conn, &normalized_id)
            .ok()
            .flatten()
            .map(|meta| (meta.db_id, meta.title)),
    };
    let md_path = if let Some((db_id, title)) = registered
        && let Some(roots) = crate::paths::paper_roots()
    {
        let p_dir = crate::paths::paper_folder(&roots.data_dir, db_id, &title);
        // 書き込む前に論文フォルダを作る。
        let _ = std::fs::create_dir_all(&p_dir);
        p_dir.join(crate::paths::paper_file_names::EXTRACT)
    } else {
        let _ = std::fs::create_dir_all(&extracts_dir);
        extracts_dir.join(format!(
            "{}.md",
            docling::sanitize_filename_component(&normalized_id)
        ))
    };
    // md_path も DB には data_dir からの相対で書く（段階 2b）。
    let md_stored = crate::paths::to_stored_path(data_dir, &md_path);
    db::upsert_extract(
        conn,
        &ExtractRow {
            paper_id: normalized_id.clone(),
            status: "parsing".to_string(),
            pdf_path: Some(pdf_stored.clone()),
            pdf_mtime: Some(pdf_mtime.clone()),
            md_path: Some(md_stored.clone()),
            papers_id,
            ..Default::default()
        },
    )?;

    // `jobs` 表に記録し、同じ論文の引用抽出が実行中なら始めない（段階 5）。
    // Library に無い論文（papers_id 無し）は記録しない。
    let job = match papers_id.map(|pid| crate::jobs::start(conn, "extract", Some(pid))) {
        Some(Ok(None)) => {
            return Err(ExtractError::Io(std::io::Error::other(
                "この論文の引用抽出はすでに実行中です",
            )));
        }
        Some(Ok(Some(id))) => Some(id),
        _ => None,
    };

    // 失敗時は必ず failed を記録してから返す
    let result = run_pipeline(
        conn,
        s2_refs,
        &normalized_id,
        papers_id,
        pdf_path,
        &extracts_dir,
        &md_path,
        &pdf_mtime,
        &pdf_stored,
        &md_stored,
        run_docling_fn,
    )
    .await;

    if let Some(id) = job {
        let error = result.as_ref().err().map(ToString::to_string);
        let _ = crate::jobs::finish(conn, id, error.as_deref());
    }
    if let Err(e) = &result {
        let _ = db::upsert_extract(
            conn,
            &ExtractRow {
                paper_id: normalized_id.clone(),
                status: "failed".to_string(),
                pdf_path: Some(pdf_stored),
                pdf_mtime: Some(pdf_mtime),
                error: Some(e.to_string()),
                extracted_at: Some(Utc::now().to_rfc3339()),
                papers_id,
                ..Default::default()
            },
        );
    }
    result
}

/// paper_id（正規化済み）から DOI を解決する（ジャーナル判定用）。
///
/// - paper_id 自体が DOI 形式（`DOI:10.3389/...` / `10.3389/...`）ならそれを返す
/// - それ以外（S2 40hex 等）は `papers` テーブルから DOI を引く
/// - 取得できない場合は `None`（→ `detect_journal` は `Generic` にフォールバック）
fn resolve_paper_doi(conn: &Connection, paper_id: &str) -> Option<String> {
    let trimmed = paper_id.trim();
    let bare = trimmed.strip_prefix("DOI:").unwrap_or(trimmed);
    if bare.starts_with("10.") && bare.contains('/') {
        return Some(bare.to_string());
    }
    db::get_paper_meta_by_extract_key(conn, paper_id)
        .ok()
        .flatten()
        .and_then(|m| m.doi)
}

/// 使い回せる extract.md の中身（PDF より新しく、空でないもの）。
fn reusable_markdown(md_path: &Path, pdf_path: &Path) -> Option<String> {
    let modified = |path: &Path| std::fs::metadata(path).and_then(|m| m.modified()).ok();
    if modified(md_path)? < modified(pdf_path)? {
        return None;
    }
    std::fs::read_to_string(md_path)
        .ok()
        .filter(|text| !text.trim().is_empty())
}

/// References 欄を読んで参考文献の行を作る。戻り値の `bool` は番号付きスタイルか。
///
/// ref_no は「本文中の引用番号 [n]」と同じ番号空間に揃える（CITED IN 照合の前提）。
/// エントリ先頭に番号（`[12]` / `12.`）がある場合はそれを採用し、
/// 番号なしスタイル（著者-年形式等）のみ行順で採番する。
/// 同一番号が重複した場合は最初の 1 件のみ採用する（paper_refs の主キー
/// (paper_id, ref_no) 違反による抽出全体の失敗を防ぐ防御）。
#[must_use]
pub fn refs_from_markdown(
    markdown: &str,
    journal: parsers::Journal,
    paper_id: &str,
) -> (Vec<RefRow>, bool) {
    let raw_refs = parsers::parse_references(markdown, journal);
    let numbered = raw_refs.iter().all(|r| r.number.is_some());
    let mut ref_rows: Vec<RefRow> = raw_refs
        .iter()
        .enumerate()
        .map(|(i, r)| RefRow {
            paper_id: paper_id.to_string(),
            ref_no: if numbered {
                r.number.unwrap_or(i as i64 + 1)
            } else {
                i as i64 + 1
            },
            first_author: r.first_author.clone(),
            year: r.year,
            year_suffix: r.year_suffix.clone(),
            journal: r.journal.clone(),
            title: r.title.clone(),
            doi: r.doi.clone(),
            raw_text: r.raw_text.clone(),
            ..Default::default()
        })
        .collect();
    if numbered {
        let mut seen: HashSet<i64> = HashSet::new();
        ref_rows.retain(|r| seen.insert(r.ref_no));
    }
    (ref_rows, numbered)
}

/// 文献の行の先頭から、筆頭著者の名前を取り出す（姓の突合用）。
///
/// `Talor Abramovich, Meet Udeshi, …`（名 姓）でも `Achiam, J., …`（姓, 名）でも、
/// 最初の `,` / ` and ` / `. ` までを返す（どちらの形でも姓が含まれる）。
fn first_author_name(raw: Option<&str>) -> Option<String> {
    let raw = raw?.trim_start_matches(['-', '*', ' ']);
    // `Khai T. Tran, …` のイニシャルで切らないよう、`. ` は他の区切りが無いときだけ使う
    let end = [", ", " and ", " & "]
        .iter()
        .filter_map(|sep| raw.find(sep))
        .min()
        .or_else(|| raw.find(". "))
        .unwrap_or(raw.len());
    let name = raw[..end].trim();
    (!name.is_empty()).then(|| name.to_string())
}

/// 本文から引用を含む文を取り出し、参考文献の番号に写す。
///
/// 番号付きスタイルは `[n]`、著者-年スタイルは `(Author, Year)` を
/// ref（姓, 年）に写像して `n_sentences` / CITED IN を埋める。
#[must_use]
pub fn cited_sentences(
    markdown: &str,
    ref_rows: &[RefRow],
    numbered: bool,
) -> Vec<sentences::ExtractedSentence> {
    let mut raw_sentences = sentences::extract_sentences(markdown);
    if !numbered {
        // 著者-年形式: (Author, Year) を (ref_no, 姓, 年) に突合
        let ref_map: Vec<sentences::AuthorYearRef> = ref_rows
            .iter()
            .map(|r| {
                (
                    r.ref_no,
                    first_author_name(Some(&r.raw_text)).unwrap_or_else(|| r.first_author.clone().unwrap_or_default()),
                    r.year,
                    r.year_suffix.clone(),
                )
            })
            .collect();
        raw_sentences.extend(sentences::extract_author_year_sentences(markdown, &ref_map));
    }
    raw_sentences
}

/// docling 実行以降のパイプライン本体。
#[allow(clippy::too_many_arguments)]
async fn run_pipeline<F, Fut>(
    conn: &Connection,
    s2_refs: Vec<crate::api::s2::S2PaperDetail>,
    normalized_id: &str,
    papers_id: Option<i64>,
    pdf_path: &Path,
    extracts_dir: &Path,
    md_path: &Path,
    pdf_mtime: &str,
    pdf_stored: &str,
    md_stored: &str,
    run_docling_fn: F,
) -> Result<ExtractOutput, ExtractError>
where
    F: FnOnce(PathBuf, PathBuf, PathBuf) -> Fut,
    Fut: std::future::Future<Output = Result<DoclingOutput, ExtractError>>,
{
    // 4. docling → Markdown。DL 後の図の抽出が先に extract.md を作っていれば、
    //    docling を呼ばずにそれを使う（docling は 1 本につき 1 回。段階 5）。
    let docling_result = match reusable_markdown(md_path, pdf_path) {
        Some(markdown) => DoclingOutput {
            markdown,
            md_path: md_path.to_path_buf(),
            style: "Generic".to_string(),
            figures: Vec::new(),
        },
        None => {
            run_docling_fn(
                pdf_path.to_path_buf(),
                extracts_dir.to_path_buf(),
                md_path.to_path_buf(),
            )
            .await?
        }
    };

    // 5. References 分割・パース
    // ジャーナル判別（DOI prefix）をしてから汎用/ジャーナル固有パーサを選択する。
    // 旧 Python `detect_journal` 相当。DOI は paper_id 自身（DOI: 形式）か DB から解決。
    let doi_for_journal = resolve_paper_doi(conn, normalized_id);
    let journal = parsers::detect_journal(doi_for_journal.as_deref());
    let (mut ref_rows, numbered) = refs_from_markdown(&docling_result.markdown, journal, normalized_id);

    // S2 refs は docling_ref のメタデータ補強（s2_* 列の追記）のみに使う。
    // docling にない S2 refs をリストに追加しない（S2 返却には論文以外の
    // ゴミが多く含まれるため。docling_ref のみでリストを構成する）。
    matcher::match_refs(&mut ref_rows, &s2_refs, Default::default());

    // 7. 本文中引用文抽出
    let raw_sentences = cited_sentences(&docling_result.markdown, &ref_rows, numbered);

    // 8. DB 保存（同じ Library の論文で古い抽出キーに残っている行は消す。段階 2c）
    db::replace_refs(conn, normalized_id, papers_id, &ref_rows)?;
    db::replace_sentences(conn, normalized_id, papers_id, &raw_sentences)?;
    db::upsert_extract(
        conn,
        &ExtractRow {
            paper_id: normalized_id.to_string(),
            status: "ready".to_string(),
            style: Some(docling_result.style.clone()),
            pdf_path: Some(pdf_stored.to_string()),
            pdf_mtime: Some(pdf_mtime.to_string()),
            n_refs: Some(ref_rows.len() as i64),
            n_sentences: Some(raw_sentences.len() as i64),
            md_path: Some(md_stored.to_string()),
            error: None,
            extracted_at: Some(Utc::now().to_rfc3339()),
            papers_id,
        },
    )?;

    Ok(ExtractOutput {
        refs: ref_rows,
        sentences: raw_sentences,
        style: docling_result.style,
    })
}

/// 論文の extract.md の実パスを解決する（引用抽出 → 論文フォルダ → 旧 `extracts/` の順）。
///
/// `key` が無い論文（S2 ハッシュのみで arXiv ID も DOI も持たない等）でも、
/// `papers_id` があれば論文フォルダ（[`crate::paths::paper_folder`]）の
/// extract.md を見つける。実在するファイルだけを返す。
#[must_use]
pub fn resolve_extract_md(
    conn: &Connection,
    papers_id: Option<i64>,
    key: Option<&str>,
    data_dir: &Path,
) -> Option<PathBuf> {
    // キー未指定なら papers 行から組み立てる（s2_paper_id → arxiv_id → doi）。
    let key = key.map(normalize_paper_id).or_else(|| {
        papers_id.and_then(|id| crate::figures::extract_key_for_paper(conn, id).ok().flatten())
    });

    // 1) paper_extracts.md_path（引用抽出済みならこれが正）。
    //    探す順序は、① papers_id かつ現在のキー → ② papers_id → ③ 抽出キー。
    //    同じ論文に複数の抽出キーで記録が残っているとき、現在のキーの行を優先し、
    //    それしか無ければ papers_id で拾う（キーが変わっても記録が見えるように。段階 2c）。
    const READY_MD_BY_PAPERS_AND_KEY: &str = "SELECT md_path FROM paper_extracts
         WHERE papers_id = ?1 AND paper_id = ?2 AND status = 'ready'";
    const READY_MD_BY_PAPERS: &str =
        "SELECT md_path FROM paper_extracts WHERE papers_id = ?1 AND status = 'ready'";
    const READY_MD_BY_KEY: &str =
        "SELECT md_path FROM paper_extracts WHERE paper_id = ?1 AND status = 'ready'";
    let ready_md = |sql: &str, params: &[&dyn rusqlite::ToSql]| -> Option<String> {
        conn.prepare(sql)
            .ok()?
            .query_row(params, |row| row.get::<_, Option<String>>(0))
            .ok()
            .flatten()
            .filter(|s| !s.trim().is_empty())
    };
    let md_path = (|| -> Option<String> {
        let id = papers_id?;
        if let Some(k) = key.as_deref() {
            let params: &[&dyn rusqlite::ToSql] = &[&id, &k];
            if let Some(md) = ready_md(READY_MD_BY_PAPERS_AND_KEY, params) {
                return Some(md);
            }
        }
        let params: &[&dyn rusqlite::ToSql] = &[&id];
        ready_md(READY_MD_BY_PAPERS, params)
    })()
    .or_else(|| {
        let k = key.as_deref()?;
        let params: &[&dyn rusqlite::ToSql] = &[&k];
        ready_md(READY_MD_BY_KEY, params)
    });
    if let Some(md) = md_path {
        // DB から読んだ相対パスを実ファイルパスへ直す（段階 2b）。
        let path = crate::paths::resolve_stored_path(data_dir, &md);
        if path.is_file() {
            return Some(path);
        }
    }

    // 2) 論文フォルダ配下の extract.md（置き場は paths::paper_folder で決める）
    if let Some(id) = papers_id {
        let title: String = conn
            .query_row(
                "SELECT COALESCE(title, '') FROM papers WHERE id = ?1",
                [id],
                |row| row.get(0),
            )
            .unwrap_or_default();
        let folder = crate::paths::paper_folder(data_dir, id, &title);
        let candidate = folder.join(crate::paths::paper_file_names::EXTRACT);
        if candidate.is_file() {
            return Some(candidate);
        }
        // 論文フォルダの外に PDF がある場合の保険（data_dir の外は絶対パスのまま）。
        let pdf_path: Option<String> = conn
            .query_row("SELECT pdf_path FROM papers WHERE id = ?1", [id], |row| {
                row.get(0)
            })
            .ok()
            .flatten();
        if let Some(pdf) = pdf_path {
            let pdf = crate::paths::resolve_stored_path(data_dir, &pdf);
            if let Some(parent) = pdf.parent() {
                let candidate = parent.join(crate::paths::paper_file_names::EXTRACT);
                if candidate.is_file() {
                    return Some(candidate);
                }
            }
        }
    }

    // 3) 旧レイアウト: <data_dir>/extracts/<sanitized key>.md
    let key = key?;
    let legacy = data_dir
        .join("extracts")
        .join(format!("{}.md", docling::sanitize_filename_component(&key)));
    legacy.is_file().then_some(legacy)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::s2::S2Error;
    use crate::api::s2::S2PaperDetail;

    const SAMPLE_MD: &str = "Body text cites [1] here with detail.\n\n## References\n\n[1] Smith, J. Attention Is All You Need. NeurIPS (2017).\n";

    fn memory_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        // 0016 は papers テーブルへの ALTER を含むため、最小限の papers を先に作成
        conn.execute_batch(
            "CREATE TABLE papers (id INTEGER PRIMARY KEY, title TEXT, doi TEXT, arxiv_id TEXT);",
        )
        .unwrap();
        // papers_id の列（段階 2c）を持つため、引用・翻訳・図のマイグレーションを
        // 0016 の順に適用する。
        for sql in [
            include_str!("../../migrations/0016_citation_tables.sql"),
            include_str!("../../migrations/0017_translation.sql"),
            include_str!("../../migrations/0025_paper_extracts_papers_id.sql"),
            include_str!("../../migrations/0027_paper_figures.sql"),
            include_str!("../../migrations/0033_papers_id_unify.sql"),
        ] {
            conn.execute_batch(sql).unwrap();
        }
        conn
    }

    fn temp_pdf() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "sai_paper_extract_test_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let pdf = dir.join("fake.pdf");
        std::fs::write(&pdf, b"%PDF-fake").unwrap();
        pdf
    }

    /// arXiv ID も DOI も無い論文でも、論文フォルダの extract.md を見つけられること。
    #[test]
    fn resolve_extract_md_finds_paper_folder_copy() {
        let conn = memory_db();
        conn.execute_batch("ALTER TABLE papers ADD COLUMN pdf_path TEXT;")
            .unwrap();
        let pdf = temp_pdf();
        let dir = pdf.parent().unwrap().to_path_buf();
        let expected = dir.join(crate::paths::paper_file_names::EXTRACT);
        std::fs::write(&expected, "# body").unwrap();
        conn.execute(
            "INSERT INTO papers (id, title, pdf_path) VALUES (1, 'A', ?1)",
            [pdf.to_string_lossy().to_string()],
        )
        .unwrap();

        let found = resolve_extract_md(&conn, Some(1), None, Path::new("/nonexistent"));
        assert_eq!(found, Some(expected));
        std::fs::remove_dir_all(&dir).ok();
    }

    /// S2 refs を返さない fetch クロージャ（密閉テスト用）
    fn no_refs() -> impl FnOnce(String) -> std::future::Ready<Result<Vec<S2PaperDetail>, S2Error>> {
        |_: String| std::future::ready(Ok(Vec::new()))
    }

    fn mock_runner(
        markdown: &'static str,
    ) -> impl FnOnce(PathBuf, PathBuf, PathBuf) -> std::future::Ready<Result<DoclingOutput, ExtractError>>
    {
        move |_, _, md_path| {
            std::fs::write(&md_path, markdown).unwrap();
            std::future::ready(Ok(DoclingOutput {
                markdown: markdown.to_string(),
                md_path,
                style: "Generic".to_string(),
                figures: Vec::new(),
            }))
        }
    }

    #[tokio::test]
    async fn ensure_extract_parses_and_stores() {
        let conn = memory_db();
        let pdf = temp_pdf();
        let data_dir = pdf.parent().unwrap().to_path_buf();

        let out = ensure_extract_with(
            &conn,
            "ff312b6027d012403760c288fd6531145006cb9f",
            &pdf,
            &data_dir,
            no_refs(),
            mock_runner(SAMPLE_MD),
        )
        .await
        .unwrap();
        // docling refs 1件
        assert_eq!(out.refs.len(), 1);
        assert_eq!(out.refs[0].ref_no, 1);
        assert_eq!(out.refs[0].year, Some(2017));
        // 本文引用文 1件
        assert_eq!(out.sentences.len(), 1);

        let extract = db::get_extract(&conn, "ff312b6027d012403760c288fd6531145006cb9f")
            .unwrap()
            .unwrap();
        assert_eq!(extract.status, "ready");
        assert_eq!(extract.n_refs, Some(1));
    }

    #[tokio::test]
    async fn ensure_extract_cache_hit_on_second_run() {
        let conn = memory_db();
        let pdf = temp_pdf();
        let data_dir = pdf.parent().unwrap().to_path_buf();
        let id = "ff312b6027d012403760c288fd6531145006cb9f";

        ensure_extract_with(
            &conn,
            id,
            &pdf,
            &data_dir,
            no_refs(),
            mock_runner(SAMPLE_MD),
        )
        .await
        .unwrap();
        // 同じ PDF で再実行 → キャッシュヒット（DB から読み直し、docling / S2 を呼ばない）
        let out = ensure_extract_with(
            &conn,
            id,
            &pdf,
            &data_dir,
            no_refs(),
            mock_runner("SHOULD NOT BE USED"),
        )
        .await
        .unwrap();
        assert_eq!(out.refs.len(), 1);
    }

    /// 先に extract.md があれば docling を呼ばずにそれを解析する（段階 5）。
    #[tokio::test]
    async fn ensure_extract_reuses_existing_markdown_without_docling() {
        let conn = memory_db();
        let pdf = temp_pdf();
        let data_dir = pdf.parent().unwrap().to_path_buf();
        let id = "ff312b6027d012403760c288fd6531145006cb9f";
        let md = data_dir
            .join("extracts")
            .join(format!("{}.md", docling::sanitize_filename_component(id)));
        std::fs::create_dir_all(md.parent().unwrap()).unwrap();
        std::fs::write(&md, SAMPLE_MD).unwrap();

        let calls = std::cell::Cell::new(0);
        let out = ensure_extract_with(&conn, id, &pdf, &data_dir, no_refs(), |_, _, _| {
            calls.set(calls.get() + 1);
            std::future::ready(Err(ExtractError::Docling("呼ばれないはず".to_string())))
        })
        .await
        .unwrap();
        assert_eq!(calls.get(), 0, "docling は呼ばれない");
        assert_eq!(out.refs.len(), 1);
        assert_eq!(out.sentences.len(), 1);
    }

    #[tokio::test]
    async fn ensure_extract_records_failure() {
        let conn = memory_db();
        let pdf = temp_pdf();
        let data_dir = pdf.parent().unwrap().to_path_buf();
        let id = "ff312b6027d012403760c288fd6531145006cb9f";

        let result = ensure_extract_with(&conn, id, &pdf, &data_dir, no_refs(), |_, _, _| {
            std::future::ready(Err(ExtractError::Docling("boom".to_string())))
        })
        .await;
        assert!(result.is_err());
        let extract = db::get_extract(&conn, id).unwrap().unwrap();
        assert_eq!(extract.status, "failed");
        assert!(
            extract
                .error
                .as_deref()
                .unwrap_or_default()
                .contains("boom")
        );
    }

    #[tokio::test]
    async fn ensure_extract_enriches_without_appending_s2_refs() {
        let conn = memory_db();
        let pdf = temp_pdf();
        let data_dir = pdf.parent().unwrap().to_path_buf();
        let id = "ff312b6027d012403760c288fd6531145006cb9f";

        // S2 refs 2件（1件は docling refs とマッチ、1件は docling に対応なし）
        let s2_refs = vec![
            S2PaperDetail {
                paper_id: Some("s2-a".into()),
                title: Some("Attention Is All You Need".into()),
                ..Default::default()
            },
            S2PaperDetail {
                paper_id: Some("s2-b".into()),
                title: Some("Completely Unrelated Work".into()),
                ..Default::default()
            },
        ];
        // s2_refs を注入するため run_pipeline を直接呼ぶ
        let extracts_dir = data_dir.join("extracts");
        std::fs::create_dir_all(&extracts_dir).unwrap();
        let md_path = extracts_dir.join("test.md");
        std::fs::write(&md_path, SAMPLE_MD).unwrap();
        let out = run_pipeline(
            &conn,
            s2_refs,
            id,
            None,
            &pdf,
            &extracts_dir,
            &md_path,
            "0",
            pdf.to_str().unwrap(),
            &crate::paths::to_stored_path(&data_dir, &md_path),
            move |_, _, save| {
                std::future::ready({
                    std::fs::write(&save, SAMPLE_MD).unwrap();
                    Ok(DoclingOutput {
                        markdown: SAMPLE_MD.to_string(),
                        md_path: save,
                        style: "Generic".to_string(),
                        figures: Vec::new(),
                    })
                })
            },
        )
        .await
        .unwrap();
        // docling refs のみでリスト構成。S2 にしかない refs は追加しない
        assert_eq!(out.refs.len(), 1);
        // マッチした s2_ref でメタデータが補強される
        let matched = &out.refs[0];
        assert_eq!(matched.ref_no, 1);
        assert_eq!(matched.s2_paper_id.as_deref(), Some("s2-a"));
    }

    #[tokio::test]
    async fn ensure_extract_dedupes_duplicate_entry_numbers() {
        let conn = memory_db();
        let pdf = temp_pdf();
        let data_dir = pdf.parent().unwrap().to_path_buf();
        let id = "ff312b6027d012403760c288fd6531145006cb9f";

        // 同一番号 [1] が重複する異常データでも PK 違反で失敗しない
        const DUP_MD: &str =
            "Body [1].\n\n## References\n\n[1] A. Foo. (2020).\n[1] B. Bar. (2021).\n";
        let extracts_dir = data_dir.join("extracts");
        std::fs::create_dir_all(&extracts_dir).unwrap();
        let md_path = extracts_dir.join("test_dup.md");
        let out = run_pipeline(
            &conn,
            Vec::new(),
            id,
            None,
            &pdf,
            &extracts_dir,
            &md_path,
            "0",
            pdf.to_str().unwrap(),
            &crate::paths::to_stored_path(&data_dir, &md_path),
            move |_, _, save| {
                std::future::ready({
                    std::fs::write(&save, DUP_MD).unwrap();
                    Ok(DoclingOutput {
                        markdown: DUP_MD.to_string(),
                        md_path: save,
                        style: "Generic".to_string(),
                        figures: Vec::new(),
                    })
                })
            },
        )
        .await
        .unwrap();
        // 最初の 1 件のみ採用
        assert_eq!(out.refs.len(), 1);
        assert_eq!(out.refs[0].ref_no, 1);
    }

    #[test]
    fn normalize_paper_id_cases() {
        let hex = "ff312b6027d012403760c288fd6531145006cb9f";
        assert_eq!(normalize_paper_id(hex), hex);
        assert_eq!(normalize_paper_id(&format!("ArXiv:{hex}")), hex);
        assert_eq!(normalize_paper_id("ArXiv:1706.03762"), "ArXiv:1706.03762");
        assert_eq!(normalize_paper_id("1706.03762"), "ArXiv:1706.03762");
        assert_eq!(
            normalize_paper_id("https://arxiv.org/abs/2401.12345v2"),
            "ArXiv:2401.12345"
        );
        assert_eq!(
            normalize_paper_id("DOI:10.1038/nature12373"),
            "DOI:10.1038/nature12373"
        );
        assert_eq!(
            normalize_paper_id("10.1038/nature12373"),
            "DOI:10.1038/nature12373"
        );
        assert_eq!(
            normalize_paper_id("https://doi.org/10.1038/nature12373"),
            "DOI:10.1038/nature12373"
        );
        assert!(is_s2_40hex(hex));
        assert!(!is_s2_40hex("ArXiv:1706.03762"));
    }

    #[tokio::test]
    async fn test_extract_md_saved_to_paper_folder() {
        let conn = memory_db();
        let pdf = temp_pdf();
        let data_dir = pdf.parent().unwrap().to_path_buf();
        let id = "ff312b6027d012403760c288fd6531145006cb9f";
        let _out = ensure_extract_with(
            &conn,
            id,
            &pdf,
            &data_dir,
            no_refs(),
            mock_runner(SAMPLE_MD),
        )
        .await
        .unwrap();
        let extract = db::get_extract(&conn, id).unwrap().unwrap();
        assert!(extract.md_path.is_some());
        // md_path は DB 上では data_dir からの相対（段階 2b）なので、実パスへ直して存在確認する。
        assert!(
            extract
                .md_path
                .as_ref()
                .map(|p| crate::paths::resolve_stored_path(&data_dir, p))
                .is_some_and(|p| p.exists())
        );
    }

    /// 抽出キーが変わっても（S2 の ID が後から見つかっても）記録が見えること（段階 2c）。
    #[tokio::test]
    async fn records_stay_visible_after_extract_key_changes() {
        let conn = memory_db();
        // Library 登録済み・arXiv ID のみ（S2 ID はまだ無い）の論文
        conn.execute(
            "INSERT INTO papers (id, title, arxiv_id) VALUES (1, 'Key change', '2401.00001')",
            [],
        )
        .unwrap();
        let pdf = temp_pdf();
        let data_dir = pdf.parent().unwrap().to_path_buf();

        ensure_extract_with(
            &conn,
            "ArXiv:2401.00001",
            &pdf,
            &data_dir,
            no_refs(),
            mock_runner(SAMPLE_MD),
        )
        .await
        .unwrap();

        // 書いた直後は arXiv キーで読める。行には papers_id が入っている。
        let papers_id = crate::Database::papers_id_for_extract_key(&conn, "ArXiv:2401.00001");
        assert_eq!(papers_id, Some(1));
        assert_eq!(
            db::get_refs_for(&conn, papers_id, Some("ArXiv:2401.00001"))
                .unwrap()
                .len(),
            1
        );
        let stored_papers_id: Option<i64> = conn
            .query_row("SELECT papers_id FROM paper_refs", [], |row| row.get(0))
            .unwrap();
        assert_eq!(stored_papers_id, Some(1));

        // その後 S2 の ID が見つかる（抽出キーが変わる）
        let s2 = "ff312b6027d012403760c288fd6531145006cb9f";
        conn.execute("UPDATE papers SET s2_paper_id = ?1 WHERE id = 1", [s2])
            .unwrap();
        assert_eq!(
            crate::Database::papers_id_for_extract_key(&conn, s2),
            Some(1)
        );

        // キーだけでは見つからず、papers_id を渡すと見える（papers_id 優先の効き目）。
        assert!(
            db::get_refs_for(&conn, None, Some(s2)).unwrap().is_empty(),
            "新しいキーの行はまだ無い（このテストの前提）"
        );
        assert_eq!(
            db::get_refs_for(&conn, Some(1), Some(s2)).unwrap().len(),
            1,
            "キーが変わって引用文献が見えなくなった"
        );
        assert_eq!(
            db::get_sentences_for(&conn, Some(1), Some(s2))
                .unwrap()
                .len(),
            1,
            "キーが変わって引用文が見えなくなった"
        );
        assert!(
            db::get_extract_for(&conn, Some(1), Some(s2))
                .unwrap()
                .is_some(),
            "キーが変わって抽出状態が見えなくなった"
        );
        // 抽出 Markdown の場所も papers_id 経由で見える。
        let md = resolve_extract_md(&conn, Some(1), Some(s2), &data_dir);
        assert!(md.is_some_and(|p| p.is_file()), "extract.md が見つからない");
    }

    /// キーが変わったあとに再抽出しても、同じ論文の行が二重にならないこと（段階 2c）。
    #[tokio::test]
    async fn reextract_after_key_change_does_not_duplicate_rows() {
        let conn = memory_db();
        conn.execute(
            "INSERT INTO papers (id, title, arxiv_id) VALUES (1, 'Reextract', '2401.00002')",
            [],
        )
        .unwrap();
        let pdf = temp_pdf();
        let data_dir = pdf.parent().unwrap().to_path_buf();
        ensure_extract_with(
            &conn,
            "ArXiv:2401.00002",
            &pdf,
            &data_dir,
            no_refs(),
            mock_runner(SAMPLE_MD),
        )
        .await
        .unwrap();

        // S2 の ID が見つかる（抽出キーが変わる）
        let s2 = "ff312b6027d012403760c288fd6531145006cb9f";
        conn.execute("UPDATE papers SET s2_paper_id = ?1 WHERE id = 1", [s2])
            .unwrap();

        // 別の PDF で再抽出（stale 判定が外れる）
        let pdf2 = temp_pdf();
        ensure_extract_with(
            &conn,
            s2,
            &pdf2,
            &data_dir,
            no_refs(),
            mock_runner(SAMPLE_MD),
        )
        .await
        .unwrap();

        let count = |sql: &str| -> i64 { conn.query_row(sql, [], |row| row.get(0)).unwrap() };
        assert_eq!(
            count("SELECT COUNT(*) FROM paper_refs"),
            1,
            "引用文献が二重になった"
        );
        assert_eq!(
            count("SELECT COUNT(*) FROM paper_sentences"),
            1,
            "引用文が二重になった"
        );
        assert_eq!(
            count("SELECT COUNT(*) FROM paper_extracts"),
            1,
            "抽出状態が二重になった"
        );
        // 残っているのは新しいキーの行で、papers_id も入っている。
        let (row_key, row_papers_id): (String, Option<i64>) = conn
            .query_row(
                "SELECT paper_id, papers_id FROM paper_extracts",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(row_key, s2);
        assert_eq!(row_papers_id, Some(1));
    }
}
