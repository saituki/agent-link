//! `sai-paper phone` — sai-phone（iPhone 窓口）向けの JSON 出力。
//!
//! 全サブコマンドが `Result<serde_json::Value>` を返す内部関数にまとまっており、
//! `handle` が 1 か所で出力と失敗処理を担う。
//!
//! - `Ok(v)` → JSON 1 行を標準出力へ
//! - `Err(e)` → `{"error": "<メッセージ>"}` を標準出力へ出し、終了コード 1
//!
//! 読み取り用サブコマンド（library / tree / paper / log / providers）は DB に
//! 書き込まない（02 の計画）。

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use chrono::{Datelike, Local};
use clap::Subcommand;
use rusqlite::OptionalExtension;
use sai_paper_core::citation;
use sai_paper_core::citation::db::get_refs_for;
use sai_paper_core::paths::PaperRoots;
use sai_paper_core::{
    Config, Database, LibraryIndexer, LibraryPaper, Paths, S2Client, figures, library_tree, paths,
    podcast_client, search_log, translation,
};
use serde_json::{Value, json};

use crate::open_database;

/// `api::PaperSourceResolver::resolve` が受け付ける DB ID（resolver を参照）。
const AVAILABLE_PROVIDERS: [&str; 6] = ["s2", "pubmed", "biorxiv", "medrxiv", "openalex", "arxiv"];

/// `sai-paper phone` のサブコマンド。
#[derive(Subcommand, Debug)]
pub enum PhoneCmd {
    /// Library の論文一覧（DB 登録日の新しい順）
    Library,
    /// Library の階層表示（検索ログごと → 日付ごとの Import）
    Tree,
    /// 論文 1 件の詳細（ファイル・メモ・引用文献・音声化状態つき）
    Paper {
        /// 論文 ID（papers.id）
        paper_id: i64,
    },
    /// 検索履歴 1 件とヒット一覧
    Log {
        /// 検索ログ ID（search_logs.id）
        log_id: i64,
    },
    /// 検索に使う DB 一覧（既定の DB と、使える全 DB）
    Providers,
    /// 論文を検索し、TUI の Discover と同じ検索履歴に記録する
    Search {
        /// 検索ワード
        query: String,
        /// 検索する DB（繰り返し指定可。無ければ config の既定）
        #[arg(long)]
        db: Vec<String>,
        /// 開始年（指定年の下限）
        #[arg(long)]
        year_from: Option<u16>,
        /// 終了年（指定年の上限）
        #[arg(long)]
        year_to: Option<u16>,
        /// 取得件数（既定 20、1〜100 に丸める）
        #[arg(long, default_value_t = 20)]
        limit: u16,
    },
    /// 要旨を日本語に訳して保存する（TUI の `a` / `A` キーと同じ）
    Translate {
        /// 論文 ID（papers.id）
        paper_id: i64,
        /// 訳済みでももう一度訳す
        #[arg(long)]
        force: bool,
    },
    /// 音声化する（TUI の `r` キーと同じ。数分かかり、終わるまで戻らない）
    Radio {
        /// 論文 ID（papers.id）
        paper_id: i64,
    },
    /// PDF を日本語に訳す（TUI の `T` キーと同じ。数分かかり、終わるまで戻らない）
    #[command(name = "translate-pdf")]
    TranslatePdf {
        /// 論文 ID（papers.id）
        paper_id: i64,
    },
    /// Dashboard（TUI と同じ数字カード＋ Radio の出来事を混ぜた RECENT ACTIVITY）
    Dashboard,
    /// ノート（note.md）を標準入力の中身で保存する（無ければ TUI の `n` と同じ場所に作る）
    Note {
        /// 論文 ID（papers.id）
        paper_id: i64,
    },
    /// 現況（state.md）・考え（mind.md）・状況のまとめ・要対応をまとめて出す（S05）
    Project,
    /// mind.md の「## メモ」に標準入力の 1 行を足す（S05。書き込みはここだけ）
    #[command(name = "mind-append")]
    MindAppend,
    /// Canvas（sai-phone の Canvas 画面用。`--` の後ろが Canvas 名）
    Canvas {
        #[command(subcommand)]
        cmd: crate::phone_canvas::PhoneCanvasCmd,
    },
    /// BibTeX の取り込み（sai-phone の Import 画面用）
    Import {
        #[command(subcommand)]
        cmd: crate::phone_import::PhoneImportCmd,
    },
    /// 論文の PDF を取得して Library に登録する（sai-phone の Download 画面用）
    #[command(group(
        clap::ArgGroup::new("target").required(true).multiple(false).args(["log", "import_id"]),
    ))]
    Download {
        /// 検索履歴 ID（`--position` とセットで指定）
        #[arg(long, requires = "position")]
        log: Option<i64>,
        /// 検索履歴の中での位置（0 始まり。`--log` とセットで指定）
        #[arg(long, requires = "log")]
        position: Option<i64>,
        /// BibTeX からの `RemotePaper.id`（例: `ArXiv:1706.03762`）
        #[arg(long = "import-id")]
        import_id: Option<String>,
    },
    /// Download の後の自動処理（図の抽出 → PDF の翻訳。`phone download` のあとで呼ぶ）
    AfterDownload {
        /// 論文 ID（papers.id）
        paper_id: i64,
    },
    /// 要対応の 1 件を「対応済み」にする（`sai-paper attention resolve` と同じ。JSON で返す）
    AttentionResolve {
        /// 対応済みにする key（例: `question:41:対象は?`）
        key: String,
    },
}

/// `sai-paper phone` を実行し、JSON 1 行を標準出力へ出す。
///
/// 失敗はどのサブコマンドでも同じ形（`{"error": …}` + 終了コード 1）で返す。
pub async fn handle(cmd: PhoneCmd, client: &S2Client) -> Result<()> {
    // 重い処理の記録（jobs）に、この窓口を phone として残す。
    sai_paper_core::jobs::set_origin("phone");
    let outcome = match cmd {
        PhoneCmd::Library => library(),
        PhoneCmd::Tree => tree(),
        PhoneCmd::Paper { paper_id } => paper(paper_id),
        PhoneCmd::Log { log_id } => {
            let (db, cfg, paths) = open_context()?;
            log(&db, &cfg, &paths, log_id)
        }
        PhoneCmd::Providers => providers(),
        PhoneCmd::Search {
            query,
            db,
            year_from,
            year_to,
            limit,
        } => search(&query, &db, year_from, year_to, limit).await,
        PhoneCmd::Translate { paper_id, force } => translate(paper_id, force).await,
        PhoneCmd::Radio { paper_id } => radio(paper_id).await,
        PhoneCmd::TranslatePdf { paper_id } => translate_pdf(paper_id).await,
        PhoneCmd::Dashboard => dashboard(),
        PhoneCmd::Note { paper_id } => save_note(paper_id),
        PhoneCmd::Project => project(),
        PhoneCmd::MindAppend => mind_append(),
        PhoneCmd::Canvas { cmd } => crate::phone_canvas::run(cmd),
        PhoneCmd::Import { cmd } => crate::phone_import::run_import(cmd),
        PhoneCmd::Download {
            log,
            position,
            import_id,
        } => crate::phone_import::download(client, log, position, import_id.as_deref()).await,
        PhoneCmd::AfterDownload { paper_id } => crate::phone_import::after_download(paper_id).await,
        PhoneCmd::AttentionResolve { key } => crate::phone_import::attention_resolve(&key),
    };
    match outcome {
        Ok(value) => {
            println!("{}", serde_json::to_string(&value)?);
            Ok(())
        }
        Err(err) => {
            println!("{}", json!({ "error": format!("{err:#}") }));
            std::process::exit(1);
        }
    }
}

/// 読み取り系サブコマンド共通の準備（DB・config・パス）。
pub(crate) fn open_context() -> Result<(Database, Config, Paths)> {
    let db = open_database()?;
    let paths = Paths::discover()?;
    let cfg = Config::load_or_create(&paths)?;
    Ok((db, cfg, paths))
}

/// Library の論文一覧（`phone library` / `tree` / `log` で共通、`phone download` も使う）。
///
/// TUI の Library 表示と同じ対象フォルダで絞り込む。並びは DB の
/// `created_at` の新しい順（同刻は ID の新しい順）。
pub(crate) fn library_papers(
    db: &Database,
    cfg: &Config,
    paths: &Paths,
) -> Result<Vec<LibraryPaper>> {
    let roots = library_tree::library_roots(cfg, paths);
    let mut papers = db.library_papers_in_roots(&roots)?;
    let created = created_at_map(db)?;
    papers.sort_by(|a, b| {
        let key_a = (created.get(&a.id).map(String::as_str).unwrap_or(""), a.id);
        let key_b = (created.get(&b.id).map(String::as_str).unwrap_or(""), b.id);
        key_b.cmp(&key_a)
    });
    Ok(papers)
}

/// 論文 ID → DB 登録時刻（`papers.created_at` を 1 回だけ読む）。
fn created_at_map(db: &Database) -> Result<HashMap<i64, String>> {
    let mut stmt = db.connection().prepare("SELECT id, created_at FROM papers")?;
    let rows = stmt.query_map([], |row| {
        Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
    })?;
    Ok(rows.collect::<Result<HashMap<_, _>, _>>()?)
}

/// 論文 ID → 音声化ステータス（`paper_podcasts.status` を 1 回だけ読む）。
fn podcast_status_map(db: &Database) -> Result<HashMap<i64, String>> {
    let mut stmt = db
        .connection()
        .prepare("SELECT paper_id, status FROM paper_podcasts")?;
    let rows = stmt.query_map([], |row| {
        Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
    })?;
    Ok(rows.collect::<Result<HashMap<_, _>, _>>()?)
}

/// 実在するファイルの絶対パス（無ければ `None`）。
fn abs_if_file(path: &Path) -> Option<String> {
    if !path.is_file() {
        return None;
    }
    let absolute = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
    Some(absolute.to_string_lossy().into_owned())
}

/// 論文の抽出テキスト（extract.md）の場所（実在するものだけ）。
///
/// TUI / `content paper` と同じ解決順（引用抽出の記録 → 論文フォルダ →
/// 旧 `extracts/`）を優先し、無ければ論文フォルダ（`paths::paper_folder`）も試す
/// （extract.md が 2 系統に分かれている実データに対応するため）。
fn extract_file(db: &Database, paper: &LibraryPaper, data_dir: &Path) -> Option<PathBuf> {
    citation::resolve_extract_md(db.connection(), Some(paper.id), None, data_dir).or_else(|| {
        let roots = paths::paper_roots()?;
        let dir = paths::paper_folder(&roots.data_dir, paper.id, &paper.title);
        let file = paths::paper_file_path(&dir, "extract");
        file.is_file().then_some(file)
    })
}

/// 論文のノート（note.md）の場所（実在するものだけ）。
///
/// 論文フォルダ（`paths::paper_folder`。段階 2a で置き場はここだけ）→
/// PDF のあるフォルダ（DB の絶対パス。段階 2b で整理）の順に探す。
fn note_file(paths: &Paths, paper: &LibraryPaper) -> Option<PathBuf> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    dirs.push(paths::paper_folder(
        &paths.data_dir(),
        paper.id,
        &paper.title,
    ));
    if let Some(parent) = paper
        .pdf_path
        .as_deref()
        .map(Path::new)
        .and_then(Path::parent)
    {
        dirs.push(parent.to_path_buf());
    }
    dirs.into_iter()
        .map(|dir| paths::paper_file_path(&dir, "note"))
        .find(|file| file.is_file())
}

/// `published_at` 文字列から年の 4 桁文字列を作る（00 §7.2）。
///
/// 先頭 4 文字が数字で読めない、または 1900 未満（未設定の既定値）なら `None`（JSON `null`）。
fn published_year(published_at: Option<&str>) -> Option<String> {
    let raw = published_at?.trim();
    let year: u16 = raw.get(..4)?.parse().ok()?;
    if year < 1900 { None } else { Some(year.to_string()) }
}

/// リモート論文の出版年（`RemotePaper.published`）。1900 未満（未設定の既定値）なら `None`。
pub(crate) fn remote_year(published: &chrono::DateTime<chrono::Utc>) -> Option<String> {
    let year = published.year();
    if (1900..=9999).contains(&year) {
        Some(year.to_string())
    } else {
        None
    }
}

/// `phone library`: Library の論文一覧（00 §7.2）。
///
/// `created_at` は `papers` から、`podcast` は `paper_podcasts` から 1 回ずつ読む。
fn library() -> Result<Value> {
    let (db, cfg, paths) = open_context()?;
    let papers = library_papers(&db, &cfg, &paths)?;
    let created = created_at_map(&db)?;
    let podcasts = podcast_status_map(&db)?;
    let data_dir = paths.data_dir();

    let mut items = Vec::with_capacity(papers.len());
    for paper in &papers {
        items.push(json!({
            "id": paper.id,
            "title": paper.title,
            "authors": paper.authors,
            "year": published_year(paper.published_at.as_deref()),
            "journal": paper.journal,
            "reading_status": paper.reading_status,
            "is_favorite": paper.is_favorite,
            "created_at": created.get(&paper.id),
            "has_pdf": library_tree::paper_pdf_exists(paper),
            "has_extract": extract_file(&db, paper, &data_dir).is_some(),
            "podcast": podcasts.get(&paper.id),
        }));
    }
    Ok(json!({ "papers": items }))
}

/// `phone tree`: 階層グループ一覧（TUI の階層表示と同じまとめ方）。
fn tree() -> Result<Value> {
    let (db, cfg, paths) = open_context()?;
    let papers = library_papers(&db, &cfg, &paths)?;
    let groups = library_tree::build_groups(db.connection(), &papers);
    Ok(json!({ "groups": serde_json::to_value(groups)? }))
}

/// `phone paper <id>`: 論文 1 件の詳細（00 §7.2）。
pub(crate) fn paper(paper_id: i64) -> Result<Value> {
    let (db, _cfg, paths) = open_context()?;
    let Some(paper) = db.library_paper_by_id(paper_id)? else {
        bail!("論文が見つかりません");
    };

    // s2_paper_id は LibraryPaper に無いため DB から直接読む。
    let s2_paper_id: Option<String> = db
        .connection()
        .query_row(
            "SELECT s2_paper_id FROM papers WHERE id = ?1",
            [paper_id],
            |row| row.get(0),
        )
        .optional()?;

    // files: 実在するファイルだけ絶対パス（無ければ null）。
    let pdf_path = paper
        .pdf_path
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let pdf = pdf_path.and_then(|p| abs_if_file(Path::new(p)));
    let pdf_ja = pdf_path
        .and_then(paths::translated_pdf_path)
        .and_then(|p| abs_if_file(&p));
    let data_dir = paths.data_dir();
    let extract = extract_file(&db, &paper, &data_dir).and_then(|p| abs_if_file(&p));
    let note_path = note_file(&paths, &paper);
    let note = note_path.as_deref().and_then(abs_if_file);

    // note: note.md の本文（trim して空なら null）。
    let note_text = note_path
        .as_deref()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty());

    // refs: 引用文献抽出済みのときだけ（無ければ []）。
    //    記録はまず papers_id で、無ければ抽出キーで探す（段階 2c）。
    //    JSON の形は変えない。
    let refs = {
        // paper_refs のキーは正規化済み（`ArXiv:…` / `DOI:…` / 40hex）なので揃える。
        let key = db
            .citation_key_for_paper(paper_id)
            .map(|raw_key| citation::normalize_paper_id(&raw_key));
        // local_paper_id 判定用の表を 1 回だけ読む。
        let mut stmt = db
            .connection()
            .prepare("SELECT id, s2_paper_id, doi FROM papers")?;
        let local_rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, Option<String>>(2)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        drop(stmt);

        get_refs_for(db.connection(), Some(paper_id), key.as_deref())?
            .into_iter()
            .map(|row| {
                let local_paper_id =
                    local_paper_id(&local_rows, row.s2_paper_id.as_deref(), row.doi.as_deref());
                json!({
                    "ref_no": row.ref_no,
                    "title": row.s2_title.or(row.title),
                    "authors": row.first_author,
                    "year": row.s2_year.or(row.year),
                    "journal": row.s2_venue.or(row.journal),
                    "doi": row.doi,
                    "s2_paper_id": row.s2_paper_id,
                    "local_paper_id": local_paper_id,
                })
            })
            .collect::<Vec<_>>()
    };

    // podcast: paper_podcasts の行（無ければ null）。
    // radio_id は音声の名前（<フォルダ名>-<番号>）。再生画面への飛び先に使う。
    let podcast = db
        .connection()
        .query_row(
            "SELECT status, generated_at, error FROM paper_podcasts WHERE paper_id = ?1",
            [paper_id],
            |row| {
                Ok(json!({
                    "status": row.get::<_, String>(0)?,
                    "generated_at": row.get::<_, Option<String>>(1)?,
                    "error": row.get::<_, Option<String>>(2)?,
                    "radio_id": podcast_client::radio_paper_id(paper_id),
                }))
            },
        )
        .optional()?;

    // figures: PAPER 画面の FIGURES と同じ中身（画像が消えた図は出さない）。
    // state は ready（図あり）/ empty（抽出済みで 0 件）/ not_extracted（未抽出・抽出中・失敗）。
    // image_path は data_dir からの相対なので、実パスへ直してから返す（段階 2b）。
    let (figure_state, figure_rows) =
        match figures::figure_state_for_paper(db.connection(), paper_id, &data_dir)? {
            figures::FigureState::Ready(rows) => ("ready", rows),
            figures::FigureState::Empty => ("empty", Vec::new()),
            figures::FigureState::NotExtracted => ("not_extracted", Vec::new()),
        };
    let figure_items: Vec<Value> = figure_rows
        .iter()
        .map(|f| {
            json!({
                "fig_no": f.fig_no,
                "label": f.label,
                "title": f.title,
                "caption": f.caption,
                "mention": f.mention,
                "page": f.page,
                "image": f.image_path,
            })
        })
        .collect();

    Ok(json!({
        "figures": { "state": figure_state, "items": figure_items },
        "paper": {
            "id": paper.id,
            "title": paper.title,
            "authors": paper.authors,
            "year": published_year(paper.published_at.as_deref()),
            "journal": paper.journal,
            "doi": paper.doi,
            "arxiv_id": paper.arxiv_id,
            "s2_paper_id": s2_paper_id,
            "citation_count": paper.citation_count,
            "reading_status": paper.reading_status,
            "abstract": paper.abstract_text,
            "abstract_ja": paper.abstract_ja,
        },
        "files": {
            "pdf": pdf,
            "pdf_ja": pdf_ja,
            "extract": extract,
            "note": note,
        },
        "note": note_text,
        "refs": refs,
        "podcast": podcast,
    }))
}

/// 引用文献に対応する Library 論文 ID（S2 ID → DOI（大文字小文字無視）の順に探す）。
fn local_paper_id(
    rows: &[(i64, Option<String>, Option<String>)],
    s2_paper_id: Option<&str>,
    doi: Option<&str>,
) -> Option<i64> {
    if let Some(s2) = s2_paper_id.map(str::trim).filter(|s| !s.is_empty())
        && let Some(hit) = rows.iter().find(|(_, s2_of_row, _)| {
            s2_of_row.as_deref().is_some_and(|s| s.eq_ignore_ascii_case(s2))
        })
    {
        return Some(hit.0);
    }
    let doi = doi?.trim();
    if doi.is_empty() {
        return None;
    }
    rows.iter()
        .find(|(_, _, doi_of_row)| {
            doi_of_row
                .as_deref()
                .is_some_and(|d| d.eq_ignore_ascii_case(doi))
        })
        .map(|(id, _, _)| *id)
}

/// `phone log <log_id>`: 検索履歴とヒット一覧（00 §7.2）。
///
/// `phone search`（03）も記録直後のログをこの関数で出力する。
fn log(db: &Database, cfg: &Config, paths: &Paths, log_id: i64) -> Result<Value> {
    let conn = db.connection();
    let Some(log_meta) = search_log::list(conn)?
        .into_iter()
        .find(|entry| entry.id == log_id)
    else {
        bail!("検索履歴が見つかりません");
    };

    let papers = library_papers(db, cfg, paths)?;
    let mut items = Vec::new();
    for item in search_log::items(conn, log_id)? {
        let remote = item.remote;
        let local_paper_id = library_tree::match_downloaded_paper(&papers, &remote)
            .map(|index| papers[index].id);
        items.push(json!({
            "position": item.position,
            "id": remote.id,
            "title": remote.title,
            "authors": remote.authors,
            "year": remote_year(&remote.published),
            "journal": remote.journal_ref,
            "doi": remote.doi,
            "abstract": remote.abstract_text,
            "citation_count": remote.citation_count,
            "local_paper_id": local_paper_id,
        }));
    }

    Ok(json!({
        "log": {
            "id": log_meta.id,
            "searched_at": log_meta.searched_at,
            "date": log_meta.log_date,
            "query": log_meta.query,
            "year": log_meta.year,
            "source_dbs": log_meta.source_dbs,
        },
        "items": items,
    }))
}

/// `phone providers`: 既定の DB（設定の `discover.providers`）と全 DB の一覧。
fn providers() -> Result<Value> {
    let paths = Paths::discover()?;
    let cfg = Config::load_or_create(&paths)?;
    Ok(json!({
        "default": cfg.discover.providers,
        "available": AVAILABLE_PROVIDERS,
    }))
}

/// 検索年のラベル（`search_logs.year` に保存する文字列）。
///
/// 両方指定 `2010-2020` / 下限だけ `2010-` / 上限だけ `-2020` / 指定なしは空文字。
fn year_label(from: Option<u16>, to: Option<u16>) -> String {
    match (from, to) {
        (Some(from), Some(to)) => format!("{from}-{to}"),
        (Some(from), None) => format!("{from}-"),
        (None, Some(to)) => format!("-{to}"),
        (None, None) => String::new(),
    }
}

/// `phone search <query>`: 検索して検索履歴に記録し、そのログを返す（00 §7.2）。
///
/// 出力は `phone log <新しい log_id>` と同じ形。結果 0 件でも履歴は記録する（TUI と同じ）。
async fn search(
    query: &str,
    db: &[String],
    year_from: Option<u16>,
    year_to: Option<u16>,
    limit: u16,
) -> Result<Value> {
    let query = query.trim();
    if query.is_empty() {
        bail!("検索語が空です");
    }
    let limit = limit.clamp(1, 100);

    let (database, cfg, paths) = open_context()?;
    let (dbs, papers) =
        crate::run_search(&cfg, query, db, limit, year_from, year_to, None).await?;
    let year = year_label(year_from, year_to);
    let log_id = search_log::record_search(
        database.connection(),
        "phone",
        query,
        &year,
        &limit.to_string(),
        &dbs.join(","),
        &papers,
    )?;
    // Dashboard の出来事に「discover」として残す（TUI の検索と同じ）
    database.record_activity("discover", None, Some(query))?;
    log(&database, &cfg, &paths, log_id)
}

/// `phone translate <id>`: 要旨の日本語訳（TUI の `a` / `A` キーと同じ）。
///
/// 訳文は `papers.abstract_ja` に保存され、次の呼び出しでは再利用される。
async fn translate(paper_id: i64, force: bool) -> Result<Value> {
    let (db, _cfg, _paths) = open_context()?;
    let Some(paper) = db.library_paper_by_id(paper_id)? else {
        bail!("論文が見つかりません");
    };
    let abstract_en = paper.abstract_text.as_deref().unwrap_or("").trim();
    if abstract_en.is_empty() {
        bail!("要旨がありません");
    }
    // 訳済みなら --force が無ければそれを返す（翻訳しない）。
    if !force
        && paper
            .abstract_ja
            .as_deref()
            .is_some_and(|ja| !ja.trim().is_empty())
    {
        return Ok(json!({ "id": paper_id, "abstract_ja": paper.abstract_ja }));
    }
    let service = Config::load_or_create(&Paths::discover()?)?.translate.abstract_;
    match translation::ensure_abstract_ja(db.connection(), paper_id, abstract_en, force, &service)
        .await
    {
        Ok(Some(ja)) => Ok(json!({ "id": paper_id, "abstract_ja": ja })),
        Ok(None) => bail!("要旨がありません"),
        Err(err) => bail!("翻訳できませんでした: {err}"),
    }
}

/// `phone radio <id>`: 音声化（TUI の `r` キーと同じ流れ。数分かかる）。
///
/// `paper_podcasts` を generating → ready / failed に更新するのは core の共通処理。
/// `radio_id` は sai-radio に登録された音声の名前（<フォルダ名>-<番号>）。
async fn radio(paper_id: i64) -> Result<Value> {
    let paths = Paths::discover()?;
    let cfg = Config::load_or_create(&paths)?;
    let radio_id = podcast_client::generate_for_paper(&paths.database_file, paper_id, &cfg.radio)
        .await
        .map_err(|message| anyhow::anyhow!(message))?;
    Ok(json!({ "id": paper_id, "radio_id": radio_id, "status": "ready" }))
}

/// `phone translate-pdf`: PDF を訳し、できた翻訳 PDF の場所を返す。
async fn translate_pdf(paper_id: i64) -> Result<Value> {
    let paths = Paths::discover()?;
    let cfg = Config::load_or_create(&paths)?;
    let path = sai_paper_core::pipeline::translate_pdf(&paths.database_file, paper_id, &cfg.translate.pdf)
        .await
        .map_err(|message| anyhow::anyhow!(message))?;
    Ok(json!({ "id": paper_id, "pdf_ja_path": path, "status": "ready" }))
}

/// Dashboard の出来事の最大件数（iPhone に送る量を抑える）。
const DASHBOARD_ACTIVITY_MAX: usize = 300;

/// `phone dashboard`: TUI の Dashboard と同じ数字カードと RECENT ACTIVITY。
///
/// - 数字は TUI と同じ数え方（論文は Library フォルダの PDF 数、ノートは論文フォルダの note.md の数）。
/// - 出来事には sai-radio の「聴いた・質問した」を混ぜる（止まっていれば sai-paper の分だけ）。
fn dashboard() -> Result<Value> {
    let (db, cfg, paths) = open_context()?;
    let collection_roots: Vec<PathBuf> = cfg
        .library.import_folders
        .iter()
        .map(|root| std::fs::canonicalize(root).unwrap_or_else(|_| root.clone()))
        .collect();
    let tags: i64 = db
        .connection()
        .query_row("SELECT COUNT(*) FROM tags", [], |row| row.get(0))?;
    let notes = sai_paper_core::paths::note_count(&paths.data_dir());
    let mut activity = db.all_activity()?;
    podcast_client::merge_radio_activity(&mut activity, &cfg.radio, db.connection());
    activity.truncate(DASHBOARD_ACTIVITY_MAX);
    let items: Vec<Value> = activity
        .iter()
        .map(|a| {
            json!({
                "kind": a.kind,
                "label": a.label,
                "occurred_at": a.occurred_at.to_rfc3339(),
                "paper_id": a.paper_id,
                "radio_id": a.radio_id,
            })
        })
        .collect();
    Ok(json!({
        "cards": {
            "papers": LibraryIndexer::count_pdfs(&collection_roots),
            "tags": tags,
            "notes": notes,
            "pdf_bytes": LibraryIndexer::pdf_storage_size(&collection_roots),
            "db_bytes": std::fs::metadata(&paths.database_file).map_or(0, |m| m.len()),
        },
        "activity": items,
    }))
}

/// `phone note <id>`: 標準入力の中身でノート（note.md）を保存する。
///
/// 既にあるノートはその場所で上書きする。無ければ論文フォルダ
/// （`paths::paper_folder`）に作る。保存したら出来事に「notes」を残す。
fn save_note(paper_id: i64) -> Result<Value> {
    use std::io::Read as _;
    let (db, _cfg, paths) = open_context()?;
    let Some(paper) = db.library_paper_by_id(paper_id)? else {
        bail!("論文が見つかりません");
    };
    let mut text = String::new();
    std::io::stdin().read_to_string(&mut text)?;
    let file = match note_file(&paths, &paper) {
        Some(file) => file,
        None => {
            // 書き込む前に論文フォルダを作る。
            let dir = paths::paper_folder(&paths.data_dir(), paper.id, &paper.title);
            std::fs::create_dir_all(&dir)?;
            paths::paper_file_path(&dir, "note")
        }
    };
    std::fs::write(&file, &text)?;
    db.record_activity("notes", Some(paper_id), None)?;
    let note = text.trim();
    Ok(json!({ "id": paper_id, "note": (!note.is_empty()).then_some(note) }))
}

/// `phone project`: 現況（state.md）・考え（mind.md）・状況のまとめ（brief）・要対応を出す（S05）。
fn project() -> Result<Value> {
    let (db, cfg, _paths) = open_context()?;
    let roots = paths::paper_roots().context("プロジェクトルート（.sai/paper）が見つかりません")?;
    project_json(&roots, &db, &cfg)
}

/// `phone project` の本体（テストからは場所を直接渡す）。
///
/// JSON のキーは `state` / `mind` / `brief` / `attention` の 4 つで、
/// `brief --write` と違い `project-brief.md` は書き換えない。
fn project_json(roots: &PaperRoots, db: &Database, cfg: &Config) -> Result<Value> {
    let state = read_text(&paths::state_path(&roots.data_dir));
    let mind = read_text(&paths::mind_path(&roots.data_dir));
    let brief = sai_paper_core::brief::build_brief(
        roots,
        Some(db.connection()),
        &cfg.brief,
        &cfg.rag.default_index,
    );
    let attention = sai_paper_core::attention::collect(db.connection(), roots)?;
    Ok(json!({
        "state": state,
        "mind": mind,
        "brief": brief,
        "attention": attention,
    }))
}

/// `phone mind-append`: 標準入力の 1 行を mind.md の「## メモ」の節に足す（S05）。
///
/// state.md はスマホからは読むだけ（書き換えない）。
fn mind_append() -> Result<Value> {
    let roots = paths::paper_roots().context("プロジェクトルート（.sai/paper）が見つかりません")?;
    let mut input = String::new();
    std::io::stdin().read_line(&mut input)?;
    let text = input.trim_end_matches(['\n', '\r']);
    let stamp = Local::now().format("%m/%d %H:%M").to_string();
    let line = sai_paper_core::project_files::append_memo(&roots.data_dir, text, &stamp)?;
    Ok(json!({ "ok": true, "line": line }))
}

/// ファイルの中身（無い・読めないときは空の文字）。
fn read_text(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::{project_json, year_label};
    use sai_paper_core::paths::PaperRoots;
    use sai_paper_core::{Config, Database};

    /// `search` の年のラベル（両方・下限だけ・上限だけ・指定なしの 4 通り）。
    #[test]
    fn year_label_covers_both_ends_and_nothing() {
        assert_eq!(year_label(Some(2010), Some(2020)), "2010-2020");
        assert_eq!(year_label(Some(2010), None), "2010-");
        assert_eq!(year_label(None, Some(2020)), "-2020");
        assert_eq!(year_label(None, None), "");
    }

    /// `phone project` は 4 つのキーを返し、`project-brief.md` を書き換えない（AC2）。
    #[test]
    fn project_json_has_keys_and_does_not_write_brief() {
        let dir = std::env::temp_dir().join(format!(
            "sai-paper-phone-project-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let data_dir = dir.join(".sai/paper");
        std::fs::create_dir_all(&data_dir).unwrap();
        std::fs::write(data_dir.join("state.md"), "# 現況\n\n- 目標: テスト\n").unwrap();
        std::fs::write(data_dir.join("mind.md"), "# mind\n\n## メモ\n").unwrap();

        let db = Database::open(&data_dir.join("papr.db")).unwrap();
        let roots = PaperRoots {
            root: dir.clone(),
            data_dir: data_dir.clone(),
        };
        let value = project_json(&roots, &db, &Config::default()).unwrap();

        for key in ["state", "mind", "brief", "attention"] {
            assert!(value.get(key).is_some(), "missing key: {key}");
        }
        assert!(value["attention"].is_array());
        assert!(value["state"].as_str().unwrap().contains("テスト"));
        // `brief --write` と違い `project-brief.md` は作らない。
        assert!(!dir.join(".sai/template/project-brief.md").exists());

        let _ = std::fs::remove_dir_all(&dir);
    }
}
