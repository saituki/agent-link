//! `sai-paper` の窓口（Web・TUI）向けの JSON 出力を組み立てる関数群。
//!
//! 以前の `sai-paper phone …` という隠した命令は U19 で消した。今は
//! `sai-paper library`・`sai-paper paper`・`sai-paper dashboard` などの
//! 見える命令が、ここの `Result<serde_json::Value>` を返す関数を呼ぶ。
//!
//! 読み取り用（library / tree / paper / log / providers）は DB に書き込まない（02 の計画）。

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Result, bail};
use chrono::Datelike;
use rusqlite::OptionalExtension;
use sai_paper_core::citation;
use sai_paper_core::citation::db::get_refs_for;
use sai_paper_core::{
    Config, Database, LibraryIndexer, LibraryPaper, Paths, figures, find_history,
    library_tree, paths, podcast_client, search_history, translation,
};
use serde_json::{Value, json};

use crate::open_database;

/// `api::PaperSourceResolver::resolve` が受け付ける DB ID（resolver を参照）。
const AVAILABLE_PROVIDERS: [&str; 6] = ["s2", "pubmed", "biorxiv", "medrxiv", "openalex", "arxiv"];

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
pub(crate) fn library() -> Result<Value> {
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
pub(crate) fn tree() -> Result<Value> {
    let (db, cfg, paths) = open_context()?;
    let papers = library_papers(&db, &cfg, &paths)?;
    let groups = library_tree::build_groups(db.connection(), &papers);
    // 内部検索の履歴（U01。新しい順。`groups` は変えない）。
    let find_groups = find_history::groups(db.connection(), &papers)?;
    Ok(json!({
        "groups": serde_json::to_value(groups)?,
        "find_groups": serde_json::to_value(find_groups)?,
    }))
}

/// `phone paper <id>`: 論文 1 件の詳細（00 §7.2）。
pub(crate) fn paper(paper_id: i64) -> Result<Value> {
    let (db, _cfg, paths) = open_context()?;
    let Some(paper) = db.library_paper_by_id(paper_id)? else {
        return Err(crate::cli_error::CliError::not_found("論文が見つかりません").into());
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
pub(crate) fn log(db: &Database, cfg: &Config, paths: &Paths, log_id: i64) -> Result<Value> {
    let conn = db.connection();
    let Some(log_meta) = search_history::list(conn)?
        .into_iter()
        .find(|entry| entry.id == log_id)
    else {
        return Err(crate::cli_error::CliError::not_found("検索履歴が見つかりません").into());
    };

    let papers = library_papers(db, cfg, paths)?;
    let mut items = Vec::new();
    for item in search_history::items(conn, log_id)? {
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

    let (actor, node) = search_actor(conn, log_id);
    Ok(json!({
        "log": {
            "id": log_meta.id,
            "searched_at": log_meta.searched_at,
            "date": log_meta.log_date,
            "query": log_meta.query,
            "year": log_meta.year,
            "source_dbs": log_meta.source_dbs,
            "executer": log_meta.executer,
            "actor": actor,
            "node": node,
        },
        "items": items,
    }))
}

/// 検索の実行者と、結び付いた research-tree の Node（`log.actor` / `log.node`）。
///
/// 実行者は `search_history` の行の `executer` 列から作る。Node は行の `rt_node`、
/// 無ければ `rt_searches` に残った値。読めないものは空にする（エラーにしない）。
/// - `actor`: `{"kind": "human"}` か `{"kind": "agent", "run_id": 数 | null, "block": null}`
/// - `node`: `{"id": "O5"}`（Node が結び付いていなければ `null`）
fn search_actor(conn: &rusqlite::Connection, log_id: i64) -> (Value, Value) {
    let (executer, set_node, set_run): (String, Option<i64>, Option<i64>) = conn
        .query_row(
            "SELECT executer, rt_node, run_id FROM search_history WHERE id = ?1",
            [log_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap_or(("human".to_string(), None, None));
    let rt: Option<(Option<i64>, Option<i64>)> = conn
        .query_row(
            "SELECT run_id, node_id FROM rt_searches WHERE log_id = ?1 OR set_id = ?1 ORDER BY id LIMIT 1",
            [log_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .ok()
        .flatten();
    let run_id = set_run.or(rt.and_then(|(run, _)| run));
    let node_id = set_node.or(rt.and_then(|(_, node)| node));

    let actor = actor_json(&executer, run_id);
    let node = node_id
        .and_then(|id| sai_paper_core::search_history::rt_label(conn, id))
        .map(|label| json!({ "id": label }));
    (actor, node.unwrap_or(Value::Null))
}

/// `log.actor` の JSON。block の名前は、sai-paper が sai-link のデータを読まないので出さない（null）。
fn actor_json(executer: &str, run_id: Option<i64>) -> Value {
    if executer == "agent" {
        json!({ "kind": "agent", "run_id": run_id, "block": Value::Null })
    } else {
        json!({ "kind": "human" })
    }
}

/// 内部検索の履歴 1 件を、使い回しの JSON（C-2）にする。
///
/// `phone find`（検索直後）と `phone find-log`（過去の履歴）で共用。
pub(crate) fn find_history_json(
    conn: &rusqlite::Connection,
    row: &find_history::FindLogRow,
    rows: &[(i64, Option<i64>, Value)],
) -> Value {
    // 実行者: 行の `executer` 列（`search_actor` と同じ作り）。
    let actor = actor_json(&row.executer, row.run_id);
    // `find_log` に Node がなければ、`sai-sci find` が残した `rt_searches` から読む。
    let node_id = row.node_id.or_else(|| {
        conn.query_row(
            "SELECT node_id FROM rt_searches WHERE tool = 'find' AND log_id = ?1 ORDER BY id DESC LIMIT 1",
            [row.id],
            |r| r.get::<_, i64>(0),
        )
        .optional()
        .ok()
        .flatten()
    });
    let node = node_id
        .and_then(|id| sai_paper_core::search_history::rt_label(conn, id))
        .map(|label| json!({ "id": label }));
    let hits: Vec<Value> = rows
        .iter()
        .map(|(position, paper_id, item)| {
            let map = item.as_object();
            let authors: Vec<Value> = map
                .and_then(|map| map.get("authors"))
                .and_then(|value| value.as_str())
                .and_then(|text| serde_json::from_str::<Vec<String>>(text).ok())
                .unwrap_or_default()
                .into_iter()
                .map(Value::String)
                .collect();
            let get = |key: &str| map.and_then(|map| map.get(key)).cloned().unwrap_or(Value::Null);
            json!({
                "position": position,
                "score": get("score"),
                "chunk_id": get("chunk_id"),
                "source_kind": get("source_kind"),
                "source_ref": get("source_ref"),
                "paper_id": paper_id.map_or(Value::Null, Value::from),
                "title": get("title"),
                "authors": authors,
                "year": get("year"),
                "section": get("section"),
                "text": get("text"),
                "cite": get("cite"),
                "passage": get("passage"),
            })
        })
        .collect();
    json!({
        "log": {
            "id": row.id,
            "date": row.log_date,
            "searched_at": row.searched_at,
            "query": row.query,
            "mode": row.mode,
            "top_k": row.top_k,
            "scope": row.scope,
            "range": row.range_text,
            "papers": row.papers,
            "node": node.unwrap_or(Value::Null),
            "subtree": row.subtree,
            "each": row.each,
            "executer": row.executer,
            "actor": actor,
        },
        "hits": hits,
    })
}

/// `find-history <id>`: 内部検索の履歴 1 件とヒット一覧（U01）。
pub(crate) fn find_history_view(db: &Database, log_id: i64) -> Result<Value> {
    let conn = db.connection();
    let Some(row) = find_history::get(conn, log_id)? else {
        return Err(crate::cli_error::CliError::not_found("検索履歴が見つかりません").into());
    };
    let rows = find_history::hits(conn, log_id)?;
    Ok(find_history_json(conn, &row, &rows))
}

/// `phone providers`: 既定の DB（設定の `discover.providers`）と全 DB の一覧。
pub(crate) fn providers() -> Result<Value> {
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
pub(crate) fn year_label(from: Option<u16>, to: Option<u16>) -> String {
    match (from, to) {
        (Some(from), Some(to)) => format!("{from}-{to}"),
        (Some(from), None) => format!("{from}-"),
        (None, Some(to)) => format!("-{to}"),
        (None, None) => String::new(),
    }
}

/// 検索の応答に `failed` を足す（`log` と `items` はそのまま・08 §3）。
///
/// 失敗した DB と理由を `[{"db": "s2", "reason": "…"}]` の形で入れる。
/// 1 件も失敗していなければ空の配列。`phone log <id>`（過去の検索結果）には
/// 使わない（履歴には理由を残していないため）。
pub(crate) fn with_failed(value: Value, failed: &[(String, String)]) -> Result<Value> {
    let Value::Object(mut map) = value else {
        // 想定より別の形が来たら、そのまま返す（検索の応答は常に object）。
        return Ok(value);
    };
    map.insert(
        "failed".to_owned(),
        Value::Array(
            failed
                .iter()
                .map(|(db_id, reason)| json!({ "db": db_id, "reason": reason }))
                .collect(),
        ),
    );
    Ok(Value::Object(map))
}

/// `phone translate <id>`: 要旨の日本語訳（TUI の `a` / `A` キーと同じ）。
///
/// 訳文は `papers.abstract_ja` に保存され、次の呼び出しでは再利用される。
pub(crate) async fn translate(paper_id: i64, force: bool) -> Result<Value> {
    let (db, _cfg, _paths) = open_context()?;
    let Some(paper) = db.library_paper_by_id(paper_id)? else {
        return Err(crate::cli_error::CliError::not_found("論文が見つかりません").into());
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
pub(crate) async fn radio(paper_id: i64) -> Result<Value> {
    let paths = Paths::discover()?;
    let cfg = Config::load_or_create(&paths)?;
    let radio_id = podcast_client::generate_for_paper(&paths.database_file, paper_id, &cfg.radio)
        .await
        .map_err(|message| anyhow::anyhow!(message))?;
    Ok(json!({ "id": paper_id, "radio_id": radio_id, "status": "ready" }))
}

/// `phone translate-pdf`: PDF を訳し、できた翻訳 PDF の場所を返す。
pub(crate) async fn translate_pdf(paper_id: i64) -> Result<Value> {
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
pub(crate) fn dashboard() -> Result<Value> {
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
    let mut timed: Vec<(chrono::DateTime<chrono::Utc>, Value)> = activity
        .iter()
        .map(|a| {
            (
                a.occurred_at,
                json!({
                    "kind": a.kind,
                    "label": a.label,
                    "occurred_at": a.occurred_at.to_rfc3339(),
                    "paper_id": a.paper_id,
                    "radio_id": a.radio_id,
                }),
            )
        })
        .collect();
    // 研究の run（Action）の行は、sai-sci が持つのでここでは混ぜない（要件定義書 A8）。
    timed.sort_by(|a, b| b.0.cmp(&a.0));
    timed.truncate(DASHBOARD_ACTIVITY_MAX);
    let items: Vec<Value> = timed.into_iter().map(|(_, v)| v).collect();
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
pub(crate) fn save_note(paper_id: i64) -> Result<Value> {
    use std::io::Read as _;
    let (db, _cfg, paths) = open_context()?;
    let Some(paper) = db.library_paper_by_id(paper_id)? else {
        return Err(crate::cli_error::CliError::not_found("論文が見つかりません").into());
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

#[cfg(test)]
mod tests {
    /// 検索の実行者: 窓口の検索は human、agent の検索は rt_searches の run_id と Node を返す。
    #[test]
    fn search_actor_is_human_or_agent_with_node() {
        use super::search_actor;
        use sai_paper_core::search_history::record;

        let db = Database::in_memory().unwrap();
        let conn = db.connection();
        let human = record(conn, "phone", "human", None, "q", "", "10", "s2", &[]).unwrap();
        assert_eq!(search_actor(conn, human).0["kind"], "human");
        assert!(search_actor(conn, human).1.is_null());

        let agent = record(conn, "cli", "agent", None, "q2", "", "10", "s2", &[]).unwrap();
        conn.execute(
            "INSERT INTO rt_searches (run_id, tool, query, set_id) VALUES (7, 'search', 'q2', ?1)",
            [agent],
        )
        .unwrap();
        let (actor, _node) = search_actor(conn, agent);
        assert_eq!(actor["kind"], "agent");
        assert_eq!(actor["run_id"], 7);

        // 履歴に run_id が残っていれば、rt_searches が無くても agent。
        let by_set = record(conn, "cli", "agent", Some(9), "q3", "", "10", "s2", &[]).unwrap();
        let (actor, _) = search_actor(conn, by_set);
        assert_eq!((actor["kind"].as_str(), actor["run_id"].as_i64()), (Some("agent"), Some(9)));
    }

    /// 内部検索の履歴: `find-log` は人と agent を見分け、`find_groups` は新しい順に並ぶ。
    #[test]
    fn find_history_view_distinguishes_human_and_agent() {
        use super::find_history_view;
        use sai_paper_core::find_history::{self, FindLogInput};
        use std::collections::HashMap;

        /// 記録用のヒット 1 件。
        fn hit(chunk_id: i64, paper_id: Option<i64>) -> sai_paper_core::Hit {
            sai_paper_core::Hit {
                score: 0.8,
                chunk_id,
                source_kind: "body".to_string(),
                source_ref: "3".to_string(),
                paper_id,
                title: Some("題".to_string()),
                authors: Some("[\"A. Smith\"]".to_string()),
                year: Some(2024),
                text: "本文".to_string(),
                char_start: None,
                char_end: None,
                md_path: None,
                section: Some("Methods".to_string()),
            }
        }

        let db = Database::in_memory().unwrap();
        let conn = db.connection();
        let empty: HashMap<i64, String> = HashMap::new();
        // 古い履歴（人）→新しい履歴（agent）の順に作る。
        let human = find_history::record(
            conn,
            &FindLogInput {
                query: "人の検索",
                mode: Some("hybrid"),
                top_k: Some(10),
                scope: Some("all"),
                range_text: "library",
                papers: &[3],
                node_id: None,
                subtree: false,
                each: None,
                origin: "phone",
                executer: "human",
                run_id: None,
            },
            &[hit(1, Some(3))],
            &empty,
        )
        .unwrap();
        let agent = find_history::record(
            conn,
            &FindLogInput {
                query: "agent の検索",
                mode: Some("hybrid"),
                top_k: Some(10),
                scope: Some("all"),
                range_text: "library",
                papers: &[],
                node_id: None,
                subtree: false,
                each: None,
                origin: "mcp",
                executer: "agent",
                run_id: Some(5),
            },
            &[hit(2, None)],
            &empty,
        )
        .unwrap();

        let shown = find_history_view(&db, human).unwrap();
        assert_eq!(shown["log"]["query"], "人の検索");
        assert_eq!(shown["log"]["actor"]["kind"], "human");
        assert_eq!(shown["log"]["executer"], "human");
        assert_eq!(shown["hits"][0]["position"], 0);
        assert_eq!(shown["hits"][0]["paper_id"], 3);
        assert_eq!(shown["hits"][0]["authors"][0], "A. Smith");
        assert!(shown["log"]["node"].is_null());

        let shown = find_history_view(&db, agent).unwrap();
        assert_eq!(shown["log"]["actor"]["kind"], "agent");
        assert_eq!(shown["log"]["actor"]["run_id"], 5);
        assert_eq!(shown["log"]["executer"], "agent");
        // block の名前は sai-link のデータを読まないので null。
        assert!(shown["log"]["actor"]["block"].is_null());
        assert!(find_history_view(&db, human + agent + 99).is_err());

        // `rt_searches` に `tool='find'` の行があれば、`find_log` の node が無くても
        // `rt_label` で Node を出す（U13 の `sai-sci find` がこう書く）。
        conn.execute(
            "INSERT INTO rt_nodes (id, kind, text, created_by) VALUES (5, 'Q', '問い', 'user')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO rt_searches (node_id, tool, query, log_id) VALUES (5, 'find', '人の検索', ?1)",
            [human],
        )
        .unwrap();
        let with_node = find_history_view(&db, human).unwrap();
        assert_eq!(with_node["log"]["node"]["id"], "Q5");

        let found = find_history::groups(conn, &[]).unwrap();
        assert_eq!(found.len(), 2);
        // 新しい順（agent が先）。論文に結び付かないヒットは数えるが論文一覧には入れない。
        assert_eq!(found[0].log_id, agent);
        assert_eq!(found[0].hit_count, 1);
        assert!(found[0].paper_ids.is_empty());
        assert_eq!(found[0].actor.kind, "agent");
        assert_eq!(found[1].log_id, human);
        assert_eq!(found[1].actor.kind, "human");
    }

    /// 実行者の決め方: run_id が数なら agent（run_id 7）、`SAI_ORIGIN=phone` なら human。
    #[test]
    fn find_history_executer_follows_origin_and_run_id() {
        use super::find_history_view;
        use sai_paper_core::find_history::{self, FindLogInput};
        use std::collections::HashMap;

        let db = Database::in_memory().unwrap();
        let conn = db.connection();
        let record_with = |origin: &str, run: Option<i64>| {
            let executer = crate::executer_of(origin, run);
            let run_id = if executer == "agent" { run } else { None };
            let id = find_history::record(
                conn,
                &FindLogInput {
                    query: "q",
                    mode: None,
                    top_k: None,
                    scope: None,
                    range_text: "library",
                    papers: &[],
                    node_id: None,
                    subtree: false,
                    each: None,
                    origin,
                    executer,
                    run_id,
                },
                &[],
                &HashMap::new(),
            )
            .unwrap();
            find_history_view(&db, id).unwrap()
        };
        let agent = record_with("cli", Some(7));
        assert_eq!(agent["log"]["executer"], "agent");
        assert_eq!(agent["log"]["actor"]["run_id"], 7);
        let web = record_with("phone", Some(7));
        assert_eq!(web["log"]["executer"], "human");
        assert_eq!(web["log"]["actor"]["kind"], "human");
        let plain = record_with("cli", None);
        assert_eq!(plain["log"]["executer"], "human");
    }

    use super::{with_failed, year_label};
    use serde_json::json;
    use sai_paper_core::Database;

    /// `search` の年のラベル（両方・下限だけ・上限だけ・指定なしの 4 通り）。
    #[test]
    fn year_label_covers_both_ends_and_nothing() {
        assert_eq!(year_label(Some(2010), Some(2020)), "2010-2020");
        assert_eq!(year_label(Some(2010), None), "2010-");
        assert_eq!(year_label(None, Some(2020)), "-2020");
        assert_eq!(year_label(None, None), "");
    }

    /// `phone search` の `failed` は DB と理由の形。0 件なら空の配列（AC1）。
    #[test]
    fn with_failed_adds_db_and_reason() {
        let base = json!({ "log": { "id": 7 }, "items": [] });
        let none = with_failed(base.clone(), &[]).unwrap();
        assert_eq!(none["failed"], json!([]));
        assert_eq!(none["log"]["id"], 7);

        let one = with_failed(
            base,
            &[(
                "s2".to_owned(),
                "search error: invalid S2 response: S2 status 429 (rate limited)".to_owned(),
            )],
        )
        .unwrap();
        assert_eq!(one["failed"][0]["db"], "s2");
        assert!(one["failed"][0]["reason"].as_str().unwrap().contains("429"));
        // 既存の項目はそのまま
        assert!(one["items"].as_array().unwrap().is_empty());
    }
}
