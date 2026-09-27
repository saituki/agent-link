//! 重い処理の記録（`jobs` 表。段階 5）。
//!
//! TUI・CLI・phone のどこから始めた処理も同じ表に書く。
//! - 同じ論文・同じ種類の処理が `running` なら、新しく始めない（二重起動の防止）。
//! - TASKS 画面（`shift+L`）はこの表を読んで、他の窓口の処理と最近の結果も出す。
//! - プロセスが落ちて `running` のまま残った行は、起動時に `failed`（中断）へ直す。

use std::sync::OnceLock;

use rusqlite::{Connection, OptionalExtension, params};

/// 処理を始めた窓口（`tui` / `cli` / `phone`）。プロセスごとに 1 回だけ決める。
static ORIGIN: OnceLock<&'static str> = OnceLock::new();

/// このプロセスの窓口を決める（最初の 1 回だけ有効）。
pub fn set_origin(origin: &'static str) {
    let _ = ORIGIN.set(origin);
}

fn origin() -> &'static str {
    ORIGIN.get().copied().unwrap_or("cli")
}

/// `running` のまま残った行を「中断」とみなすまでの時間。
pub const STALE_HOURS: i64 = 3;

/// `jobs` の 1 行。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobRow {
    /// 行番号。
    pub id: i64,
    /// 処理の種類。
    pub kind: String,
    /// 対象の論文（Library の番号）。
    pub paper_id: Option<i64>,
    /// `running` / `done` / `failed`。
    pub status: String,
    /// 今のステップ（任意）。
    pub step: Option<String>,
    /// 失敗の理由。
    pub error: Option<String>,
    /// 始めた窓口。
    pub origin: String,
    /// 開始時刻（UTC, ISO 8601）。
    pub started_at: String,
    /// 終了時刻。
    pub finished_at: Option<String>,
}

/// 処理を始める。同じ論文・同じ種類が実行中なら `Ok(None)`（始めない）。
///
/// # Errors
/// DB の書き込みに失敗したとき。
pub fn start(conn: &Connection, kind: &str, paper_id: Option<i64>) -> rusqlite::Result<Option<i64>> {
    // 判定と追加を 1 文で行い、別の窓口と同時に始めても 1 つだけが通るようにする。
    let inserted = conn.execute(
        "INSERT INTO jobs (kind, paper_id, status, origin)
         SELECT ?1, ?2, 'running', ?3
         WHERE NOT EXISTS (
             SELECT 1 FROM jobs WHERE kind = ?1 AND paper_id IS ?2 AND status IN ('queued', 'running')
         )",
        params![kind, paper_id, origin()],
    )?;
    Ok((inserted > 0).then(|| conn.last_insert_rowid()))
}

/// 今のステップを書く（TASKS 画面の表示用）。
///
/// # Errors
/// DB の書き込みに失敗したとき。
pub fn set_step(conn: &Connection, id: i64, step: &str) -> rusqlite::Result<()> {
    conn.execute("UPDATE jobs SET step = ?2 WHERE id = ?1", params![id, step])?;
    Ok(())
}

/// 処理を終える（`error` が `None` なら `done`、あれば `failed`）。
///
/// # Errors
/// DB の書き込みに失敗したとき。
pub fn finish(conn: &Connection, id: i64, error: Option<&str>) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE jobs SET status = ?2, error = ?3,
                finished_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
          WHERE id = ?1",
        params![id, if error.is_some() { "failed" } else { "done" }, error],
    )?;
    Ok(())
}

/// `running` のまま `hours` 時間を過ぎた行を `failed`（中断）にする。直した行数を返す。
///
/// # Errors
/// DB の書き込みに失敗したとき。
pub fn cleanup_stale(conn: &Connection, hours: i64) -> rusqlite::Result<usize> {
    conn.execute(
        "UPDATE jobs SET status = 'failed', error = '中断',
                finished_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
          WHERE status IN ('queued', 'running')
            AND started_at < strftime('%Y-%m-%dT%H:%M:%SZ', 'now', ?1)",
        [format!("-{hours} hours")],
    )
}

/// 新しい順に `limit` 件（実行中を先に）。
///
/// # Errors
/// DB の読み込みに失敗したとき。
pub fn recent(conn: &Connection, limit: usize) -> rusqlite::Result<Vec<JobRow>> {
    let mut statement = conn.prepare(
        "SELECT id, kind, paper_id, status, step, error, origin, started_at, finished_at
           FROM jobs
          ORDER BY (status IN ('queued', 'running')) DESC, id DESC
          LIMIT ?1",
    )?;
    let rows = statement.query_map([limit as i64], |row| {
        Ok(JobRow {
            id: row.get(0)?,
            kind: row.get(1)?,
            paper_id: row.get(2)?,
            status: row.get(3)?,
            step: row.get(4)?,
            error: row.get(5)?,
            origin: row.get(6)?,
            started_at: row.get(7)?,
            finished_at: row.get(8)?,
        })
    })?;
    rows.collect()
}

/// 同じ論文・同じ種類の処理が実行中か。
///
/// # Errors
/// DB の読み込みに失敗したとき。
pub fn is_running(conn: &Connection, kind: &str, paper_id: Option<i64>) -> rusqlite::Result<bool> {
    conn.query_row(
        "SELECT 1 FROM jobs WHERE kind = ?1 AND paper_id IS ?2 AND status IN ('queued', 'running')",
        params![kind, paper_id],
        |_| Ok(()),
    )
    .optional()
    .map(|found| found.is_some())
}

/// 処理 1 つを記録付きで走らせる（開始 → 本体 → 終了）。
///
/// 実行中なら本体を呼ばずに「実行中です」を返す。DB が開けないときは記録なしで走らせる。
///
/// # Errors
/// 本体の失敗（利用者向けの文）か、二重起動。
pub async fn run<T, F, Fut>(db_file: &std::path::Path, kind: &str, paper_id: Option<i64>, body: F) -> Result<T, String>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<T, String>>,
{
    let id = match crate::Database::open(db_file) {
        Ok(db) => match start(db.connection(), kind, paper_id) {
            Ok(Some(id)) => Some(id),
            Ok(None) => return Err("この論文の同じ処理はすでに実行中です（shift+L で確認）".to_string()),
            Err(_) => None,
        },
        Err(_) => None,
    };
    let result = body().await;
    if let Some(id) = id
        && let Ok(db) = crate::Database::open(db_file)
    {
        let _ = finish(db.connection(), id, result.as_ref().err().map(String::as_str));
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db() -> crate::Database {
        let db = crate::Database::in_memory().unwrap();
        db.connection()
            .execute("INSERT INTO papers (id, title) VALUES (1, 'A')", [])
            .unwrap();
        db
    }

    /// 同じ論文・同じ種類は 1 つしか始まらない。終われば次を始められる。
    #[test]
    fn second_start_is_refused_while_running() {
        let db = db();
        let conn = db.connection();
        let first = start(conn, "translate_pdf", Some(1)).unwrap().expect("1 つ目は始まる");
        assert_eq!(start(conn, "translate_pdf", Some(1)).unwrap(), None);
        assert!(start(conn, "radio", Some(1)).unwrap().is_some(), "種類が違えば始まる");
        assert!(is_running(conn, "translate_pdf", Some(1)).unwrap());
        finish(conn, first, None).unwrap();
        assert!(!is_running(conn, "translate_pdf", Some(1)).unwrap());
        assert!(start(conn, "translate_pdf", Some(1)).unwrap().is_some());
    }

    /// 古い running は「中断」になり、新しい running は残る。
    #[test]
    fn stale_running_rows_become_failed() {
        let db = db();
        let conn = db.connection();
        let old = start(conn, "figures", Some(1)).unwrap().unwrap();
        conn.execute(
            "UPDATE jobs SET started_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now', '-4 hours') WHERE id = ?1",
            [old],
        )
        .unwrap();
        let fresh = start(conn, "radio", Some(1)).unwrap().unwrap();
        assert_eq!(cleanup_stale(conn, STALE_HOURS).unwrap(), 1);
        let rows = recent(conn, 10).unwrap();
        let row = |id| rows.iter().find(|r| r.id == id).unwrap().clone();
        assert_eq!(row(old).status, "failed");
        assert_eq!(row(old).error.as_deref(), Some("中断"));
        assert_eq!(row(fresh).status, "running");
    }
}
