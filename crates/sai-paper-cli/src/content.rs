//! `sai-paper content` — 論文本文 / 引用箇所 / 節の内容を標準出力へ出す。
//!
//! sai-link は task.md の入力展開（`paper_id` / `chunk_id`）で
//! このコマンドを呼ぶ。**papr.db を直接読ませない**（core を唯一の真実とする）。
//!
//! `--json` では `{ "id": ..., "title": ..., "text": ... }` を返す。

use anyhow::{Context, Result};
use clap::Subcommand;
use rusqlite::Connection;
use sai_paper_core::sections::{PassageRef, SectionsError};
use sai_paper_core::{Paths, citation, figures, sections};
use std::path::Path;

use crate::open_database;

/// `sai-paper content` のサブコマンド。
#[derive(Subcommand, Debug)]
pub enum ContentCmd {
    /// 論文本文（extract.md）の全文
    Paper {
        /// 論文 ID（papers.id）
        paper_id: i64,
        /// JSON で出力する
        #[arg(long)]
        json: bool,
    },
    /// 論文の全チャンク（段落）を字数の上限なしで返す（#論文.章.段落）
    Chunks {
        /// 論文 ID（papers.id）
        paper_id: i64,
        /// JSON で出力する
        #[arg(long)]
        json: bool,
    },
    /// 引用箇所（chunks の 1 行）の本文
    Chunk {
        /// chunk_id（chunks.id）
        chunk_id: i64,
        /// JSON で出力する
        #[arg(long)]
        json: bool,
    },
    /// 論文の節の一覧（ID・見出し・文の数・最初の 2 文）
    Outline {
        /// 論文 ID（papers.id）
        paper_id: i64,
        /// JSON で出力する
        #[arg(long)]
        json: bool,
    },
    /// 節・塊を ID で読む（37.5 = 節、37.5.2 = 塊、37.5.2-4 = 範囲、37 = 全体）
    Read {
        /// 読む ID（複数可）
        #[arg(required = true)]
        refs: Vec<String>,
        /// 返す本文の字数の上限（既定 12000、最大 60000）
        #[arg(long)]
        max_chars: Option<usize>,
        /// JSON で出力する
        #[arg(long)]
        json: bool,
    },
    /// 節と塊の表を作る
    Build {
        /// 論文 ID（複数可）
        paper_ids: Vec<i64>,
        /// extract が ready の論文すべて
        #[arg(long)]
        all: bool,
        /// 本文が同じでも作り直す（要約は消える）
        #[arg(long)]
        force: bool,
        /// 続けて節の要約を作る
        #[arg(long)]
        summarize: bool,
    },
}

/// `read` の字数の上限の既定と最大。
const DEFAULT_MAX_CHARS: usize = 12_000;
const MAX_MAX_CHARS: usize = 60_000;

/// 節の一覧を JSON で返す。CLI の `--json` と MCP の `paper_outline` が共用する。
pub fn outline_json(conn: &Connection, data_dir: &Path, paper_id: i64) -> Result<serde_json::Value> {
    Ok(serde_json::to_value(sections::outline(conn, data_dir, paper_id)?)?)
}

/// 節・塊を読んで JSON で返す。CLI の `--json` と MCP の `paper_read` が共用する。
/// ID は全部を先に検査してから読む。
pub fn read_json(
    conn: &Connection,
    data_dir: &Path,
    refs: &[String],
    max_chars: Option<usize>,
) -> Result<serde_json::Value> {
    let parsed = refs
        .iter()
        .map(|r| sections::parse_ref(r))
        .collect::<Result<Vec<_>, _>>()?;
    let max_chars = max_chars.unwrap_or(DEFAULT_MAX_CHARS).clamp(1, MAX_MAX_CHARS);
    Ok(serde_json::to_value(sections::read(conn, data_dir, &parsed, max_chars)?)?)
}

/// 論文の全チャンクを JSON で返す（字数の上限なし）。論文・本文が無ければ終了コード 3。
pub fn chunks_json(conn: &Connection, data_dir: &Path, paper_id: i64) -> Result<serde_json::Value> {
    let title: Option<String> = conn
        .query_row("SELECT title FROM papers WHERE id = ?1", [paper_id], |row| row.get(0))
        .ok()
        .flatten();
    let title = title.ok_or_else(|| {
        crate::cli_error::CliError::not_found(format!("論文が見つかりません: id={paper_id}"))
    })?;
    let result = sections::read(conn, data_dir, &[PassageRef::Paper { paper_id }], usize::MAX)
        .map_err(|e| match e {
            SectionsError::NoExtract(_) | SectionsError::NotFound(_) => anyhow::Error::new(
                crate::cli_error::CliError::not_found(format!(
                    "論文 {paper_id} の本文がまだ取り出されていません"
                )),
            ),
            other => other.into(),
        })?;
    let chunks: Vec<serde_json::Value> = result
        .passages
        .into_iter()
        .map(|p| {
            serde_json::json!({
                "id": p.id,
                "section": p.section,
                "heading": p.heading,
                "text": p.text,
                "n_sentences": p.n_sentences,
            })
        })
        .collect();
    Ok(serde_json::json!({ "paper_id": paper_id, "title": title, "chunks": chunks }))
}

/// 人が読む outline の表。`level` が 2 以上の節は 2 字ずつ字下げし、lead は 40 字で切る。
fn outline_text(value: &serde_json::Value) -> String {
    let mut out = format!(
        "#{} {}（節 {}）\n",
        value["paper_id"],
        value["title"].as_str().unwrap_or(""),
        value["sections"].as_array().map_or(0, Vec::len)
    );
    for sec in value["sections"].as_array().into_iter().flatten() {
        let level = sec["level"].as_i64().unwrap_or(1).max(1) as usize;
        let lead: String = sec["lead"].as_str().unwrap_or("").chars().take(40).collect();
        let heading = format!("{}{}", "  ".repeat(level - 1), sec["heading"].as_str().unwrap_or(""));
        out.push_str(&format!(
            "{}   {heading}  {} 文  {lead}\n",
            sec["id"].as_str().unwrap_or(""),
            sec["n_sentences"]
        ));
    }
    out
}

/// 人が読む read の出力。塊ごとに `### #37.5.1（見出し）` と本文。
fn read_text(value: &serde_json::Value) -> String {
    let mut out = String::new();
    for p in value["passages"].as_array().into_iter().flatten() {
        out.push_str(&format!(
            "### {}（{}）\n{}\n\n",
            p["id"].as_str().unwrap_or(""),
            p["heading"].as_str().unwrap_or(""),
            p["text"].as_str().unwrap_or("")
        ));
    }
    if let Some(next) = value["next"].as_str() {
        out.push_str(&format!("（字数の上限で止めました。続きは {next} から）\n"));
    }
    out
}

/// 論文の `data_dir` と DB の接続を開く。
fn open_for_read() -> Result<(sai_paper_core::Database, std::path::PathBuf)> {
    Ok((open_database()?, Paths::discover()?.data_dir().to_path_buf()))
}

/// ID の先頭の論文番号（`#37.5` → 37）。読んだ記録に使う。
pub fn paper_id_of_ref(r: &str) -> Option<i64> {
    r.trim().trim_start_matches('#').split(['.', '-']).next()?.parse().ok()
}

/// `build` の本体。1 論文 1 行を出す。
async fn build(paper_ids: Vec<i64>, all: bool, force: bool, summarize: bool) -> Result<()> {
    let (db, data_dir) = open_for_read()?;
    let conn = db.connection();
    let mut ids = paper_ids;
    let mut ok = 0usize;
    let mut failed = 0usize;
    if all {
        let mut stmt = conn.prepare(
            "SELECT DISTINCT papers_id FROM paper_extracts WHERE status = 'ready' AND papers_id IS NOT NULL ORDER BY papers_id",
        )?;
        ids = stmt.query_map([], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
    }
    if ids.is_empty() {
        anyhow::bail!("論文の番号か --all を指定してください");
    }
    let config = sai_paper_core::Config::load_or_create(&Paths::discover()?)?;
    let client = if summarize {
        Some(sai_paper_core::evidence::ChatCompletionsClient::from_config(&config)?)
    } else {
        None
    };
    for id in ids {
        let state = if force {
            sections::rebuild(conn, &data_dir, id)
        } else {
            sections::ensure(conn, &data_dir, id)
        };
        let state = match state {
            Ok(s) => s,
            Err(e) => {
                eprintln!("#{id}: {e}");
                failed += 1;
                continue;
            }
        };
        let n_sections = sections::count_sections(conn, id)?;
        let n_passages: i64 = conn.query_row(
            "SELECT COUNT(*) FROM paper_passages WHERE paper_id = ?1",
            [id],
            |r| r.get(0),
        )?;
        let label = if state == sections::BuildState::Rebuilt { "作り直した" } else { "変わらず" };
        println!("#{id} 節 {n_sections} 個・塊 {n_passages} 個（{label}）");
        ok += 1;
        if let Some(client) = &client {
            let model = &config.rag.evidence.model;
            let n = sections::summarize_file(
                &Paths::discover()?.database_file,
                id,
                client,
                model,
                config.rag.evidence.max_concurrent,
            )
            .await?;
            println!("要約 {n} 節（model: {model}）");
        }
    }
    // 全部失敗したときだけ終了コード 1 にする（1 件ずつの失敗は続ける）。
    if ok == 0 && failed > 0 {
        anyhow::bail!("すべて失敗しました（{failed} 件）");
    }
    Ok(())
}

/// `sai-paper content` を実行する。
pub async fn handle_content(cmd: ContentCmd) -> Result<()> {
    match cmd {
        ContentCmd::Outline { paper_id, json } => {
            let (db, data_dir) = open_for_read()?;
            let value = outline_json(db.connection(), &data_dir, paper_id)?;
            let text = if json { value.to_string() + "\n" } else { outline_text(&value) };
            sai_paper_core::seen_ids::record(&text);
            print!("{text}");
            crate::activity::record_agent_read("cli", "outline", Some(paper_id), None);
            Ok(())
        }
        ContentCmd::Read { refs, max_chars, json } => {
            let (db, data_dir) = open_for_read()?;
            let value = read_json(db.connection(), &data_dir, &refs, max_chars)?;
            let text = if json { value.to_string() + "\n" } else { read_text(&value) };
            sai_paper_core::seen_ids::record(&text);
            print!("{text}");
            for id in refs.iter().filter_map(|r| paper_id_of_ref(r)).collect::<std::collections::BTreeSet<_>>() {
                crate::activity::record_agent_read("cli", "read", Some(id), None);
            }
            Ok(())
        }
        ContentCmd::Chunks { paper_id, json } => {
            // Web の閲覧は「エージェントが読んだ記録」に入れない。
            let (db, data_dir) = open_for_read()?;
            let value = chunks_json(db.connection(), &data_dir, paper_id)?;
            let text = if json {
                value.to_string() + "\n"
            } else {
                read_text(&serde_json::json!({ "passages": value["chunks"] }))
            };
            sai_paper_core::seen_ids::record(&text);
            print!("{text}");
            Ok(())
        }
        ContentCmd::Build { paper_ids, all, force, summarize } => build(paper_ids, all, force, summarize).await,
        ContentCmd::Paper { paper_id, json } => print_paper(paper_id, json),
        ContentCmd::Chunk { chunk_id, json } => print_chunk(chunk_id, json),
    }
}

/// 論文本文（extract.md）を出力する。
fn print_paper(paper_id: i64, json: bool) -> Result<()> {
    let db = open_database()?;
    let conn = db.connection();
    let title: Option<String> = conn
        .query_row(
            "SELECT title FROM papers WHERE id = ?1",
            [paper_id],
            |row| row.get(0),
        )
        .ok()
        .flatten();
    let title = title.ok_or_else(|| {
        crate::cli_error::CliError::not_found(format!("論文が見つかりません: id={paper_id}"))
    })?;

    // 抽出キーは無くてもよい（S2 ハッシュのみの論文でも papers.id から解決できる）。
    let key = figures::extract_key_for_paper(conn, paper_id)?;
    let data_dir = Paths::discover()?.data_dir().to_path_buf();
    let md_path = citation::resolve_extract_md(conn, Some(paper_id), key.as_deref(), &data_dir)
        .ok_or_else(|| {
            crate::cli_error::CliError::not_found(format!(
                "論文 {paper_id} の extract.md が見つかりません（抽出未実行？）"
            ))
        })?;
    let text = citation::docling::strip_figure_images(
        &std::fs::read_to_string(&md_path)
            .with_context(|| format!("extract.md の読み込みに失敗: {}", md_path.display()))?,
    );

    if json {
        println!(
            "{}",
            serde_json::json!({
                "id": paper_id,
                "title": title,
                "text": text,
                "md_path": md_path.to_string_lossy(),
            })
        );
    } else {
        print!("{text}");
    }
    // エージェントが論文を読んだ記録（失敗しても本文の出力は止めない）。
    crate::activity::record_agent_read("cli", "paper", Some(paper_id), Some(&md_path));
    Ok(())
}

/// 引用箇所（chunks の 1 行）を出力する。
fn print_chunk(chunk_id: i64, json: bool) -> Result<()> {
    let db = open_database()?;
    let conn = db.connection();
    let row: Option<(String, Option<i64>, String)> = conn
        .query_row(
            "SELECT text, paper_id, source_kind FROM chunks WHERE id = ?1",
            [chunk_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .ok();
    let (text, paper_id, source_kind) =
        row.with_context(|| format!("chunk が見つかりません: id={chunk_id}"))?;
    let title = paper_id
        .and_then(|id| {
            conn.query_row("SELECT title FROM papers WHERE id = ?1", [id], |row| {
                row.get::<_, Option<String>>(0)
            })
            .ok()
            .flatten()
        })
        .unwrap_or_default();

    if json {
        println!(
            "{}",
            serde_json::json!({
                "id": chunk_id,
                "title": title,
                "paper_id": paper_id,
                "text": text,
            })
        );
    } else {
        print!("{text}");
    }
    // エージェントが読んだ記録。断片の出どころで種類を変える（失敗は無視）。
    let target = if source_kind.starts_with("note") {
        "note"
    } else if source_kind == "abstract" {
        "paper"
    } else {
        "extract"
    };
    crate::activity::record_agent_read("cli", target, paper_id, None);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sai_paper_core::Database;

    /// 一時フォルダの papr.db に論文 1 件と extract.md（ready）を置く。
    fn setup(name: &str) -> (Database, std::path::PathBuf, i64) {
        let dir = std::env::temp_dir().join(format!("content-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let db = Database::in_memory_with_data_dir(&dir).unwrap();
        let id = db.insert_paper("Test paper", None).unwrap();
        let body = "Convolutional networks have been used widely in vision tasks. We apply them to text and report the results here. ";
        let md = format!(
            "## Test paper\n\nA short front matter line with several words in it for the test.\n\n\
             ## Abstract\n\n{0}\n\n## 1 Introduction\n\n{0}\n\n{0}\n\n### 1.1 Detail\n\n{0}\n",
            body.repeat(3)
        );
        let md_path = dir.join("extract.md");
        std::fs::write(&md_path, md).unwrap();
        db.connection()
            .execute(
                "INSERT INTO paper_extracts (paper_id, status, md_path, papers_id) VALUES ('k', 'ready', ?1, ?2)",
                rusqlite::params![md_path.to_string_lossy(), id],
            )
            .unwrap();
        (db, dir, id)
    }

    /// outline の JSON が共有契約の項目を持ち、人が読む表に節が並ぶ。
    #[test]
    fn outline_json_has_contract_fields() {
        let (db, dir, id) = setup("outline");
        let v = outline_json(db.connection(), &dir, id).unwrap();
        assert_eq!(v["paper_id"], id);
        assert!(v["extract_sha256"].as_str().unwrap().len() == 12);
        let secs = v["sections"].as_array().unwrap();
        assert!(secs.len() >= 3);
        for key in ["id", "sec_no", "heading", "level", "kind", "n_passages", "n_sentences", "lead"] {
            assert!(secs[0].get(key).is_some(), "{key} が無い");
        }
        assert!(outline_text(&v).contains(&format!("#{id}.")));
    }

    /// outline の JSON を台帳に記録すると、節の番号 `#<論文>.<節>` が run の台帳に並ぶ。
    #[test]
    fn outline_json_is_recorded_in_ledger() {
        let (db, dir, id) = setup("ledger");
        let v = outline_json(db.connection(), &dir, id).unwrap();
        sai_paper_core::seen_ids::record_for(&dir, 7, &v.to_string());
        let ledger = std::fs::read_to_string(sai_paper_core::seen_ids::ledger_path(&dir, 7)).unwrap();
        let sec = v["sections"][0]["sec_no"].as_i64().unwrap();
        assert!(ledger.lines().any(|l| l == format!("#{id}.{sec}")), "{ledger}");
    }

    /// 2 つの範囲を順に返し、上限を超えると truncated と next が入る。誤った ID は読む前に失敗する。
    #[test]
    fn read_json_orders_truncates_and_rejects_bad_ref() {
        let (db, dir, id) = setup("read");
        let conn = db.connection();
        let outline = outline_json(conn, &dir, id).unwrap();
        let sec = outline["sections"][1]["sec_no"].as_i64().unwrap();
        let full = read_json(conn, &dir, &[format!("{id}.{sec}")], None).unwrap();
        assert_eq!(full["truncated"], false);
        assert!(!full["passages"].as_array().unwrap().is_empty());
        let whole = read_json(conn, &dir, &[id.to_string()], Some(50)).unwrap();
        assert_eq!(whole["truncated"], true);
        assert!(whole["next"].is_string());
        assert!(read_text(&whole).contains("### #"));
        assert!(read_json(conn, &dir, &["abc".to_string()], None).is_err());
    }

    /// chunks は上限なしで全段落を返し、画像行は本文に入らない。論文・本文が無ければ終了コード 3。
    #[test]
    fn chunks_json_returns_all_and_not_found() {
        let (db, dir, id) = setup("chunks");
        let conn = db.connection();
        std::fs::write(
            dir.join("extract.md"),
            "## Test paper\n\nFront matter line with several words in it.\n\n![Image](/tmp/x.png)\n\n## 1 Intro\n\nBody text of the intro section goes here.\n",
        )
        .unwrap();
        let v = chunks_json(conn, &dir, id).unwrap();
        assert_eq!(v["paper_id"], id);
        assert_eq!(v["title"], "Test paper");
        let chunks = v["chunks"].as_array().unwrap();
        assert!(!chunks.is_empty());
        for key in ["id", "section", "heading", "text", "n_sentences"] {
            assert!(chunks[0].get(key).is_some(), "{key} が無い");
        }
        assert!(chunks.iter().all(|c| !c["text"].as_str().unwrap().contains("![Image]")));
        assert!(chunks[0]["id"].as_str().unwrap().starts_with(&format!("#{id}.")));
        let code = |e: anyhow::Error| e.downcast_ref::<crate::cli_error::CliError>().map(|c| c.code);
        assert_eq!(code(chunks_json(conn, &dir, 9999).unwrap_err()), Some(3));
        let none = db.insert_paper("No body", None).unwrap();
        assert_eq!(code(chunks_json(conn, &dir, none).unwrap_err()), Some(3));
    }
}
