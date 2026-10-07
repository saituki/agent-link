//! 内部検索（`find`）の履歴。
//!
//! 外部の検索（`search_history`）とは別の表（`find_history` / `find_history_hits`）に残す。
//! Node を付けない検索も含め、人も agent も毎回記録する（断片ごと保存）。

use crate::retrieval::Hit;
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use thiserror::Error;

/// 内部検索の履歴操作のエラー。
#[derive(Debug, Error)]
pub enum FindLogError {
    /// SQLite 操作に失敗した。
    #[error("database error: {0}")]
    Database(#[from] rusqlite::Error),
    /// JSON の変換に失敗した。
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
}

/// 記録に渡す中身。
#[derive(Debug, Clone)]
pub struct FindLogInput<'a> {
    /// 検索語。
    pub query: &'a str,
    /// 検索モード（word / vector / hybrid。無ければ `None`）。
    pub mode: Option<&'a str>,
    /// 上位 K 件（無ければ `None`）。
    pub top_k: Option<i64>,
    /// 検索対象（papers / notes / figures / all。無ければ `None`）。
    pub scope: Option<&'a str>,
    /// 範囲の文字列（`library` / `papers:3,7` / `node:Q12` / `subtree:Q12`）。
    pub range_text: &'a str,
    /// 検索の範囲の論文番号。
    pub papers: &'a [i64],
    /// 結び付いた Node（`rt_nodes.id`）。
    pub node_id: Option<i64>,
    /// 子孫も含めるか。
    pub subtree: bool,
    /// 範囲の論文ごとの件数（`--each`）。
    pub each: Option<i64>,
    /// `phone` / `cli` / `mcp`（経路）。
    pub origin: &'a str,
    /// 実行者（`human` / `agent`）。
    pub executer: &'a str,
    /// sai-link の実行 ID（`SAI_PARENT_RUN_ID`。agent のときだけ入れる）。
    pub run_id: Option<i64>,
}

/// 履歴 1 件の行。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FindLogRow {
    /// 主キー。
    pub id: i64,
    /// 検索実行日時（ISO8601）。
    pub searched_at: String,
    /// 階層表示の日付（`YYYY/MM/DD`）。
    pub log_date: String,
    /// 検索語。
    pub query: String,
    /// 検索モード（指定が無ければ空文字）。
    pub mode: String,
    /// 上位 K 件（指定が無ければ `None`）。
    pub top_k: Option<i64>,
    /// 検索対象（指定が無ければ空文字）。
    pub scope: String,
    /// 範囲の文字列。
    pub range_text: String,
    /// 範囲の論文番号。
    pub papers: Vec<i64>,
    /// 結び付いた Node。
    pub node_id: Option<i64>,
    /// 子孫も含めるか。
    pub subtree: bool,
    /// 範囲の論文ごとの件数。
    pub each: Option<i64>,
    /// `phone` / `cli` / `mcp`（経路）。
    pub origin: String,
    /// 実行者（`human` / `agent`）。
    pub executer: String,
    /// sai-link の実行 ID。
    pub run_id: Option<i64>,
}

/// 階層表示の 1 組（履歴 1 件ごと）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FindGroup {
    /// 履歴 ID。
    pub log_id: i64,
    /// 日付（`YYYY/MM/DD`）。
    pub date: String,
    /// 検索語。
    pub query: String,
    /// 全ヒット数（論文に結び付かないものも数える）。
    pub hit_count: usize,
    /// Library にある論文の番号（ヒット順・重複なし）。
    pub paper_ids: Vec<i64>,
    /// 実行者（`{"kind", "run_id"}`。block 名は入れない）。
    pub actor: FindActor,
}

/// 階層表示の実行者。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FindActor {
    /// `human` / `agent`。
    pub kind: String,
    /// sai-link の実行 ID。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run_id: Option<i64>,
}

impl FindActor {
    /// 履歴の 1 行の `executer` 列から実行者を作る。
    #[must_use]
    pub fn from_executer(executer: &str, run_id: Option<i64>) -> Self {
        if executer == "agent" {
            Self {
                kind: "agent".to_string(),
                run_id,
            }
        } else {
            Self {
                kind: "human".to_string(),
                run_id: None,
            }
        }
    }
}

/// 検索を 1 回記録する。`find_history` に 1 行、ヒットを位置順で `find_history_hits` に入れて log_id を返す。
///
/// ヒット 0 件でも 1 行は残す。1 つのトランザクションで書く。
///
/// # Errors
///
/// DB / JSON エラーの場合。
pub fn record(
    conn: &Connection,
    input: &FindLogInput<'_>,
    hits: &[Hit],
    passages: &HashMap<i64, String>,
) -> Result<i64, FindLogError> {
    let now = chrono::Local::now();
    let searched_at = now.to_rfc3339();
    let log_date = now.format("%Y/%m/%d").to_string();
    // `hits_to_json_with` の 1 要素ずつを丸ごと保存する（履歴を開くだけで同じ結果が出る）。
    let hit_values = crate::retrieval::hits_to_json_with(hits, passages);
    let items: &[serde_json::Value] = match &hit_values {
        serde_json::Value::Array(items) => items,
        _ => &[],
    };
    // `&Connection` では `transaction()` が使えないため、生の SQL で囲む。
    conn.execute_batch("BEGIN IMMEDIATE")?;
    let outcome = record_inner(conn, input, hits, items, &searched_at, &log_date);
    match outcome {
        Ok(log_id) => {
            conn.execute_batch("COMMIT")?;
            Ok(log_id)
        }
        Err(err) => {
            let _ = conn.execute_batch("ROLLBACK");
            Err(err)
        }
    }
}

/// [`record`] の本体（トランザクションの中）。
fn record_inner(
    conn: &Connection,
    input: &FindLogInput<'_>,
    hits: &[Hit],
    items: &[serde_json::Value],
    searched_at: &str,
    log_date: &str,
) -> Result<i64, FindLogError> {
    conn.execute(
        "INSERT INTO find_history
            (searched_at, log_date, query, mode, top_k, scope, range_text, papers_json,
             node_id, subtree, each_m, origin, run_id, executer)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
        params![
            searched_at,
            log_date,
            input.query,
            input.mode.unwrap_or(""),
            input.top_k,
            input.scope.unwrap_or(""),
            input.range_text,
            serde_json::to_string(input.papers)?,
            input.node_id,
            i64::from(input.subtree),
            input.each,
            input.origin,
            input.run_id,
            input.executer,
        ],
    )?;
    let log_id = conn.last_insert_rowid();
    for (position, (hit, item)) in hits.iter().zip(items.iter()).enumerate() {
        conn.execute(
            "INSERT INTO find_history_hits (log_id, position, paper_id, hit_json)
             VALUES (?1, ?2, ?3, ?4)",
            params![
                log_id,
                position as i64,
                hit.paper_id,
                serde_json::to_string(item)?,
            ],
        )?;
    }
    Ok(log_id)
}

/// 履歴を新しい順に一覧する。
///
/// # Errors
///
/// DB / JSON エラーの場合。
pub fn list(conn: &Connection) -> Result<Vec<FindLogRow>, FindLogError> {
    let mut stmt = conn.prepare(
        "SELECT id, searched_at, log_date, query, mode, top_k, scope, range_text, papers_json,
                node_id, subtree, each_m, origin, run_id, executer
         FROM find_history ORDER BY searched_at DESC, id DESC",
    )?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, Option<i64>>(5)?,
                row.get::<_, String>(6)?,
                row.get::<_, String>(7)?,
                row.get::<_, String>(8)?,
                row.get::<_, Option<i64>>(9)?,
                row.get::<_, i64>(10)?,
                row.get::<_, Option<i64>>(11)?,
                row.get::<_, String>(12)?,
                row.get::<_, Option<i64>>(13)?,
                row.get::<_, String>(14)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    rows.into_iter()
        .map(
            |(
                id,
                searched_at,
                log_date,
                query,
                mode,
                top_k,
                scope,
                range_text,
                papers_json,
                node_id,
                subtree,
                each,
                origin,
                run_id,
                executer,
            )| {
                Ok(FindLogRow {
                    id,
                    searched_at,
                    log_date,
                    query,
                    mode,
                    top_k,
                    scope,
                    range_text,
                    papers: serde_json::from_str(&papers_json)?,
                    node_id,
                    subtree: subtree != 0,
                    each,
                    origin,
                    executer,
                    run_id,
                })
            },
        )
        .collect()
}

/// 履歴 1 件を読む（無ければ `None`）。
///
/// # Errors
///
/// DB / JSON エラーの場合。
pub fn get(conn: &Connection, log_id: i64) -> Result<Option<FindLogRow>, FindLogError> {
    Ok(list(conn)?.into_iter().find(|row| row.id == log_id))
}

/// 履歴のヒットを位置順に読む（`position`・`paper_id`・ヒットの JSON）。
///
/// # Errors
///
/// DB / JSON エラーの場合。
pub fn hits(
    conn: &Connection,
    log_id: i64,
) -> Result<Vec<(i64, Option<i64>, serde_json::Value)>, FindLogError> {
    let mut stmt = conn.prepare(
        "SELECT position, paper_id, hit_json FROM find_history_hits
         WHERE log_id = ?1 ORDER BY position",
    )?;
    let rows = stmt
        .query_map([log_id], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, Option<i64>>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    rows.into_iter()
        .map(|(position, paper_id, hit_json)| {
            Ok((position, paper_id, serde_json::from_str(&hit_json)?))
        })
        .collect()
}

/// Library の階層に出す組（履歴 1 件につき 1 組。新しい順）。
///
/// `paper_ids` は、ヒットの `paper_id` のうち `papers`（Library の論文一覧）にあるものを、
/// ヒットの順（スコアの高い順）で重複なしに並べたもの。論文に結び付かないヒットは
/// `hit_count` に数えるが `paper_ids` には入れない。
///
/// # Errors
///
/// DB / JSON エラーの場合。
pub fn groups(
    conn: &Connection,
    papers: &[crate::LibraryPaper],
) -> Result<Vec<FindGroup>, FindLogError> {
    let logs = list(conn)?;
    let known: std::collections::HashSet<i64> = papers.iter().map(|paper| paper.id).collect();
    let mut out = Vec::with_capacity(logs.len());
    for log in &logs {
        let rows = hits(conn, log.id)?;
        let mut paper_ids: Vec<i64> = Vec::new();
        for (_, paper_id, _) in &rows {
            if let Some(id) = paper_id
                && known.contains(id)
                && !paper_ids.contains(id)
            {
                paper_ids.push(*id);
            }
        }
        out.push(FindGroup {
            log_id: log.id,
            date: log.log_date.clone(),
            query: log.query.clone(),
            hit_count: rows.len(),
            paper_ids,
            actor: FindActor::from_executer(&log.executer, log.run_id),
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 記録用のヒット 1 件（断片の JSON 化に必要な項目だけ埋める）。
    fn hit(chunk_id: i64, paper_id: Option<i64>) -> Hit {
        Hit {
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

    /// 記録用の入力（テストごとの違いは呼び出し側で変える）。
    fn input<'a>(
        query: &'a str,
        origin: &'a str,
        executer: &'a str,
        run_id: Option<i64>,
        papers: &'a [i64],
    ) -> FindLogInput<'a> {
        FindLogInput {
            query,
            mode: Some("hybrid"),
            top_k: Some(10),
            scope: Some("all"),
            range_text: "library",
            papers,
            node_id: None,
            subtree: false,
            each: None,
            origin,
            executer,
            run_id,
        }
    }

    #[test]
    fn records_and_reads_back_a_find_log() -> Result<(), Box<dyn std::error::Error>> {
        let db = crate::Database::in_memory()?;
        let conn = db.connection();
        let papers = [3, 7];
        let log_id = record(
            conn,
            &input("epilepsy EEG", "cli", "human", None, &papers),
            &[hit(1, Some(3)), hit(2, Some(7))],
            &HashMap::new(),
        )?;
        let logs = list(conn)?;
        assert_eq!(logs.len(), 1);
        assert_eq!(logs[0].id, log_id);
        assert_eq!(logs[0].query, "epilepsy EEG");
        assert_eq!(logs[0].papers, vec![3, 7]);
        assert_eq!(logs[0].log_date.len(), 10, "YYYY/MM/DD 形式");

        let loaded = get(conn, log_id)?.expect("log exists");
        assert_eq!(loaded.mode, "hybrid");
        assert_eq!(loaded.origin, "cli");

        let rows = hits(conn, log_id)?;
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].0, 0);
        assert_eq!(rows[0].1, Some(3));
        assert!(rows[0].2.get("cite").is_some());

        // ヒット 0 件でも 1 行は残る。
        let empty = record(conn, &input("空", "cli", "human", None, &[]), &[], &HashMap::new())?;
        assert_eq!(hits(conn, empty)?.len(), 0);
        assert_eq!(list(conn)?.len(), 2);
        Ok(())
    }

    #[test]
    fn actor_follows_executer_column() -> Result<(), Box<dyn std::error::Error>> {
        let db = crate::Database::in_memory()?;
        let conn = db.connection();
        let agent = record(conn, &input("a", "cli", "agent", Some(5), &[]), &[], &HashMap::new())?;
        let human = record(conn, &input("h", "cli", "human", None, &[]), &[], &HashMap::new())?;
        assert_eq!(get(conn, agent)?.expect("row").executer, "agent");
        assert_eq!(get(conn, human)?.expect("row").executer, "human");
        let actor = FindActor::from_executer("agent", Some(5));
        assert_eq!((actor.kind.as_str(), actor.run_id), ("agent", Some(5)));
        assert_eq!(FindActor::from_executer("human", Some(5)).run_id, None);
        Ok(())
    }

    #[test]
    fn groups_keep_hit_order_and_skip_unknown_papers() -> Result<(), Box<dyn std::error::Error>> {
        let db = crate::Database::in_memory()?;
        let conn = db.connection();
        record(
            conn,
            &input("q", "phone", "human", None, &[]),
            // Library に無い 9、論文に結び付かないもの、重複した 3
            &[hit(1, Some(9)), hit(2, None), hit(3, Some(3)), hit(4, Some(3))],
            &HashMap::new(),
        )?;
        let papers = vec![crate::LibraryPaper {
            id: 3,
            title: "題".to_string(),
            authors: String::new(),
            doi: None,
            arxiv_id: None,
            pdf_path: None,
            file_size: None,
            reading_status: "unread".to_string(),
            is_favorite: false,
            abstract_text: None,
            journal: None,
            published_at: None,
            updated_at: None,
            abstract_ja: None,
            abstract_ja_translated_at: None,
            citation_count: None,
            is_retracted: None,
            retraction_checked_at: None,
        }];
        let ids: Vec<i64> = papers.iter().map(|paper| paper.id).collect();
        assert!(ids.contains(&3));
        let found = groups(conn, &papers)?;
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].hit_count, 4);
        assert_eq!(found[0].paper_ids, vec![3]);
        Ok(())
    }
}
