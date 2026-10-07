//! 研究日誌（S01）: 記録から「日ごとに何が起きたか」を並べる。
//!
//! LLM は使わない。sai-paper の `papr.db`、sai-link の実行の一覧、
//! 実行フォルダ（`run list --json` の `dir`）の記録を読み、**手元の時刻（ローカル）の 0 時**で
//! 日に分けて 1 日に 1 つの [`JournalDay`] にまとめる。
//!
//! - ファイルには保存しない（開くたびに記録から作る）。
//! - 記録が 1 つも無い日は出さない。
//! - sai-link の DB が無い・読めないときは、sai-paper の記録だけで作る。
//!
//! 記録の時刻の形はまちまちである:
//!
//! - `activity_log.occurred_at` / `reading_history.opened_at` /
//!   `understanding.created_at` は `2026-09-24 00:58:38`（SQLite の
//!   `CURRENT_TIMESTAMP` = UTC）。
//! - `search_sets.searched_at` は `2026-09-23T21:49:41+09:00`（時差つき）。
//! - sai-link の実行の開始時刻などは `2026-09-25 06:01:02`
//!   （`datetime('now')` = UTC）。
//!
//! [`parse_time_in`] は時差つきをその時差で、時差なしを UTC として読む。

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Datelike, FixedOffset, Local, NaiveDate, NaiveDateTime};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use sai_paper_core::activity;
use sai_paper_core::paths::PaperRoots;
use crate::runs;

/// 実行フォルダの中で、日誌の成果物として数えないファイル（引き継ぎ用）。
const SKIPPED_RUN_FILES: [&str; 4] = [
    "task.md",
    "review.md",
    "message.md",
    "finalize-report.md",
];

/// 実行 1 件の行に並べる成果物の最大数（多すぎる実行でも 1 行に収める）。
const MAX_RUN_FILES: usize = 5;

/// 研究日誌の収集に失敗したときのエラー。
#[derive(Debug, Error)]
pub enum JournalError {
    /// SQLite 操作に失敗した。
    #[error("database error: {0}")]
    Database(#[from] rusqlite::Error),
}

/// 1 日分の日誌。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JournalDay {
    /// その日（ローカルの日付）。
    pub date: NaiveDate,
    /// その日の行（束ねた結果。古い種類から並ぶ）。
    pub lines: Vec<String>,
}

/// 日付ごとの出来事を束ねる途中の器。
#[derive(Default)]
struct DayAccum {
    /// 検索クエリ（重複を残す。表示のときに束ねる）。
    searches: Vec<String>,
    /// その日の再検索（rerun）で見つかった新着の数（クエリ → 件数、S03）。
    searches_new: HashMap<String, usize>,
    /// `DL` した論文の番号。
    downloads: Vec<i64>,
    /// 番号が分からない `DL` の件数。
    download_unknown: usize,
    /// 取り込んだ論文の番号。
    imports: Vec<i64>,
    /// 番号が分からない取り込みの件数。
    import_unknown: usize,
    /// 翻訳した論文の番号。
    translates: Vec<i64>,
    /// 番号が分からない翻訳の件数。
    translate_unknown: usize,
    /// radio を作った論文の番号。
    radios: Vec<i64>,
    /// 番号が分からない radio の件数。
    radio_unknown: usize,
    /// 読んだ論文ごとの合計秒数。
    reads: HashMap<i64, i64>,
    /// note を書いた論文の番号。
    notes: Vec<i64>,
    /// 開いたもの `(target, 表示名)`。
    opens: Vec<(String, String)>,
    /// エージェントの実行。
    runs: Vec<RunLine>,
    /// エージェントが読んだ論文の番号。
    agent_reads: Vec<i64>,
    /// 理解の確認 `(topic, state)`。
    understanding: Vec<(String, String)>,
    /// その日の research-tree の出来事（1 日 1 行に束ねる）。
    sci: Option<SciLine>,
}

/// その日の research-tree の出来事。いずれの 0 件は出さない。
#[derive(Default)]
struct SciLine {
    /// DECISIONS（決定・仮決定）の `(Node ID, 項目の題)`（古い順）。
    decisions: Vec<(String, String)>,
    /// 採用した PROPOSAL の数。
    accepted: usize,
    /// SPROUT の数。
    sprouts: usize,
}

/// エージェントの実行 1 件（日の並びを run_id の昇順に決めるため分けて持つ）。
struct RunLine {
    run_id: i64,
    text: String,
}

impl DayAccum {
    /// 決まった順（検索 → DL → 取り込み → 翻訳 → radio → 読んだ論文 → note →
    /// 開いた → エージェント → エージェントが読んだ → 理解）で行を作る。
    fn lines(&self) -> Vec<String> {
        let mut lines: Vec<String> = Vec::new();

        let searches = dedup_preserve(&self.searches);
        if !searches.is_empty() {
            let queries = searches
                .iter()
                .map(|query| match self.searches_new.get(query) {
                    // S03: その日の rerun で新着があれば件数を添える。
                    Some(new) => format!("\"{query}\"（新着 {new} 本）"),
                    None => format!("\"{query}\""),
                })
                .collect::<Vec<_>>()
                .join("、");
            lines.push(format!("検索 {} 回: {queries}", searches.len()));
        }

        if let Some(line) = paper_list_line("DL", &self.downloads, self.download_unknown) {
            lines.push(line);
        }
        if let Some(line) = paper_list_line("取り込み", &self.imports, self.import_unknown) {
            lines.push(line);
        }
        if let Some(line) = paper_list_line("翻訳", &self.translates, self.translate_unknown) {
            lines.push(line);
        }
        if let Some(line) = paper_list_line("radio", &self.radios, self.radio_unknown) {
            lines.push(line);
        }

        if !self.reads.is_empty() {
            let mut ids: Vec<i64> = self.reads.keys().copied().collect();
            ids.sort_unstable();
            let parts = ids
                .iter()
                .map(|id| {
                    let seconds = self.reads.get(id).copied().unwrap_or(0);
                    format!("#{id}（{} 分）", minutes(seconds))
                })
                .collect::<Vec<_>>()
                .join("、");
            lines.push(format!("読んだ論文: {parts}"));
        }

        let notes = sorted_unique(&self.notes);
        if !notes.is_empty() {
            lines.push(format!("note を書いた: {}", hash_list(&notes)));
        }

        if let Some(line) = open_line(&self.opens) {
            lines.push(line);
        }

        if !self.runs.is_empty() {
            let mut runs: Vec<&RunLine> = self.runs.iter().collect();
            runs.sort_by_key(|run| run.run_id);
            for run in runs {
                lines.push(run.text.clone());
            }
        }

        let agent_reads = sorted_unique(&self.agent_reads);
        if !agent_reads.is_empty() {
            lines.push(format!(
                "エージェントが読んだ論文: {}",
                hash_list(&agent_reads)
            ));
        }

        if !self.understanding.is_empty() {
            // 同じ日の同じトピックは最後（最新）の 1 件だけ残す。
            let mut order: Vec<&str> = Vec::new();
            let mut latest: HashMap<&str, &str> = HashMap::new();
            for (topic, state) in &self.understanding {
                if !latest.contains_key(topic.as_str()) {
                    order.push(topic.as_str());
                }
                latest.insert(topic.as_str(), state.as_str());
            }
            let parts = order
                .iter()
                .map(|topic| {
                    let state = latest.get(topic).copied().unwrap_or("");
                    format!("{topic} = {}", crate::understanding::state_label(state))
                })
                .collect::<Vec<_>>()
                .join("、");
            lines.push(format!("理解の確認: {parts}"));
        }

        if let Some(sci) = &self.sci
            && (!sci.decisions.is_empty() || sci.accepted > 0 || sci.sprouts > 0)
        {
            lines.push(sci.line());
        }

        lines
    }
}

/// 研究日誌を作る（新しい日が先）。`from` と `to` はどちらも含む。
///
/// 日は **手元の時刻（[`chrono::Local`]）の 0 時**で区切る。
///
/// # Errors
///
/// sai-paper の DB（`papr.db`）の読み出しに失敗したとき。
/// sai-link の DB の失敗は無視する（sai-paper の記録だけで作る）。
pub fn collect(
    conn: &Connection,
    roots: &PaperRoots,
    from: NaiveDate,
    to: NaiveDate,
) -> std::result::Result<Vec<JournalDay>, JournalError> {
    collect_at(conn, roots, from, to, *Local::now().offset())
}

/// [`collect`] の本体。時差を渡せるようにしてテストしやすくしている。
fn collect_at(
    conn: &Connection,
    roots: &PaperRoots,
    from: NaiveDate,
    to: NaiveDate,
    offset: FixedOffset,
) -> std::result::Result<Vec<JournalDay>, JournalError> {
    let mut days: BTreeMap<NaiveDate, DayAccum> = BTreeMap::new();
    let report_ids = report_ids_by_path(conn, roots);

    gather_searches(conn, &mut days, from, to, offset)?;
    gather_reruns(conn, &mut days, from, to, offset)?;
    gather_activity(conn, &mut days, from, to, offset, &report_ids)?;
    gather_reading(conn, &mut days, from, to, offset)?;
    gather_understanding(conn, &mut days, from, to, offset)?;
    // research-tree の表が無い古い DB では research-tree の行は出さない（他はそのまま作る）。
    let _ = gather_sci(conn, &mut days, from, to, offset);
    // sai-link の DB は「読めなくても研究日誌は作る」方針なので、失敗は捨てる。
    gather_runs(roots, &mut days, from, to, offset);

    Ok(days
        .into_iter()
        .rev()
        .map(|(date, accum)| JournalDay {
            date,
            lines: accum.lines(),
        })
        .collect())
}

/// 日誌を Markdown にする（見出しと日の節）。
#[must_use]
pub fn render_markdown(days: &[JournalDay]) -> String {
    let mut out = String::from("# 研究日誌\n");
    if days.is_empty() {
        out.push_str("\n（記録がありません）\n");
        return out;
    }
    for day in days {
        out.push_str(&format!("\n## {}（{}）\n", day.date, weekday_ja(day.date)));
        for line in &day.lines {
            out.push_str(&format!("- {line}\n"));
        }
    }
    out
}

/// 保存された時刻の文字列を、時差 `offset` の日時として読む。
///
/// - 時差つき（RFC3339: `2026-09-23T21:49:41+09:00`）はその時差のまま。
/// - `2026-09-24 00:58:38`（SQLite の `CURRENT_TIMESTAMP` / `datetime('now')`）と
///   `2026-09-24T00:58:38`（時差なし）は UTC とみなす。
/// - 日付だけの文字列は、その日の 00:00 UTC とみなす。
fn parse_time_in(value: &str, offset: FixedOffset) -> Option<DateTime<FixedOffset>> {
    let value = value.trim();
    if let Ok(datetime) = DateTime::parse_from_rfc3339(value) {
        return Some(datetime.with_timezone(&offset));
    }
    if let Ok(naive) = NaiveDateTime::parse_from_str(value, "%Y-%m-%d %H:%M:%S") {
        return Some(naive.and_utc().with_timezone(&offset));
    }
    if let Ok(naive) = NaiveDateTime::parse_from_str(value, "%Y/%m/%d %H:%M:%S") {
        return Some(naive.and_utc().with_timezone(&offset));
    }
    if let Ok(naive) = NaiveDateTime::parse_from_str(value, "%Y-%m-%dT%H:%M:%S") {
        return Some(naive.and_utc().with_timezone(&offset));
    }
    if let Ok(date) = NaiveDate::parse_from_str(value, "%Y-%m-%d") {
        let naive = date.and_hms_opt(0, 0, 0)?;
        return Some(naive.and_utc().with_timezone(&offset));
    }
    None
}

/// 時刻の文字列を、範囲内ならローカルの日付に直す。
fn date_in_range(
    value: &str,
    offset: FixedOffset,
    from: NaiveDate,
    to: NaiveDate,
) -> Option<NaiveDate> {
    let date = parse_time_in(value, offset)?.date_naive();
    (date >= from && date <= to).then_some(date)
}

/// `search_sets` から検索の記録を集める。
fn gather_searches(
    conn: &Connection,
    days: &mut BTreeMap<NaiveDate, DayAccum>,
    from: NaiveDate,
    to: NaiveDate,
    offset: FixedOffset,
) -> std::result::Result<(), JournalError> {
    let mut stmt = conn.prepare("SELECT COALESCE(searched_at, created_at), query FROM search_sets")?;
    let rows = stmt.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;
    for row in rows {
        let (time, query) = row?;
        let query = query.trim();
        if query.is_empty() {
            continue;
        }
        if let Some(date) = date_in_range(&time, offset, from, to) {
            days.entry(date).or_default().searches.push(query.to_string());
        }
    }
    Ok(())
}

/// `activity_log` の rerun（S03）から、その日の検索セットの新着を集める。
///
/// 同じ日に同じクエリの rerun が複数あれば、いちばん大きい新着の数を採る。
fn gather_reruns(
    conn: &Connection,
    days: &mut BTreeMap<NaiveDate, DayAccum>,
    from: NaiveDate,
    to: NaiveDate,
    offset: FixedOffset,
) -> std::result::Result<(), JournalError> {
    // set_id → クエリ（search_sets は rerun で `searched_at` を今に更新するので、
    // ここでは元のクエリだけを引く）。
    let mut queries: HashMap<i64, String> = HashMap::new();
    {
        let mut stmt = conn.prepare("SELECT id, query FROM search_sets")?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })?;
        for row in rows {
            let (id, query) = row?;
            queries.insert(id, query);
        }
    }

    let mut stmt = conn.prepare(
        "SELECT detail, occurred_at FROM activity_log
         WHERE kind = 'discover' AND detail LIKE '%\"rerun\"%'",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, Option<String>>(0)?,
            row.get::<_, String>(1)?,
        ))
    })?;
    for row in rows {
        let (detail, occurred_at) = row?;
        let Some(detail) = detail else {
            continue;
        };
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&detail) else {
            continue;
        };
        if value.get("rerun").and_then(serde_json::Value::as_bool) != Some(true) {
            continue;
        }
        let Some(set_id) = value.get("set_id").and_then(serde_json::Value::as_i64) else {
            continue;
        };
        let Some(query) = queries.get(&set_id).filter(|query| !query.trim().is_empty()) else {
            continue;
        };
        let new = value
            .get("new")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0) as usize;
        if new == 0 {
            continue;
        }
        if let Some(date) = date_in_range(&occurred_at, offset, from, to) {
            let entry = days
                .entry(date)
                .or_default()
                .searches_new
                .entry(query.clone())
                .or_insert(0);
            *entry = (*entry).max(new);
        }
    }
    Ok(())
}

/// `activity_log` から DL・取り込み・翻訳・radio・note・開いた・エージェントが読んだを集める。
fn gather_activity(
    conn: &Connection,
    days: &mut BTreeMap<NaiveDate, DayAccum>,
    from: NaiveDate,
    to: NaiveDate,
    offset: FixedOffset,
    report_ids: &HashMap<String, String>,
) -> std::result::Result<(), JournalError> {
    let mut stmt =
        conn.prepare("SELECT kind, paper_id, detail, occurred_at FROM activity_log")?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, Option<i64>>(1)?,
            row.get::<_, Option<String>>(2)?,
            row.get::<_, String>(3)?,
        ))
    })?;
    for row in rows {
        let (kind, paper_id, detail, occurred_at) = row?;
        let Some(date) = date_in_range(&occurred_at, offset, from, to) else {
            continue;
        };
        let accum = days.entry(date).or_default();
        match kind.as_str() {
            "download" => push_paper(&mut accum.downloads, &mut accum.download_unknown, paper_id),
            "import" => push_paper(&mut accum.imports, &mut accum.import_unknown, paper_id),
            "translate" => {
                push_paper(&mut accum.translates, &mut accum.translate_unknown, paper_id);
            }
            "radio" => push_paper(&mut accum.radios, &mut accum.radio_unknown, paper_id),
            "notes" => {
                if let Some(paper_id) = paper_id {
                    accum.notes.push(paper_id);
                }
            }
            "open" => {
                if let Some(detail) = detail.as_deref()
                    && let Some(entry) = open_entry(detail, report_ids)
                {
                    accum.opens.push(entry);
                }
            }
            "agent_read" => {
                if let Some(paper_id) = paper_id {
                    accum.agent_reads.push(paper_id);
                }
            }
            _ => {}
        }
    }
    Ok(())
}

/// `reading_history` から読んだ論文と読書時間を集める（論文ごとに足す）。
fn gather_reading(
    conn: &Connection,
    days: &mut BTreeMap<NaiveDate, DayAccum>,
    from: NaiveDate,
    to: NaiveDate,
    offset: FixedOffset,
) -> std::result::Result<(), JournalError> {
    let mut stmt = conn.prepare("SELECT paper_id, opened_at, duration_s FROM reading_history")?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, i64>(2)?,
        ))
    })?;
    for row in rows {
        let (paper_id, opened_at, duration_s) = row?;
        if let Some(date) = date_in_range(&opened_at, offset, from, to) {
            *days
                .entry(date)
                .or_default()
                .reads
                .entry(paper_id)
                .or_default() += duration_s;
        }
    }
    Ok(())
}

/// `understanding` から理解の確認を集める。
fn gather_understanding(
    conn: &Connection,
    days: &mut BTreeMap<NaiveDate, DayAccum>,
    from: NaiveDate,
    to: NaiveDate,
    offset: FixedOffset,
) -> std::result::Result<(), JournalError> {
    let mut stmt = conn.prepare(
        "SELECT topic, state, created_at FROM understanding ORDER BY created_at ASC, id ASC",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
        ))
    })?;
    for row in rows {
        let (topic, state, created_at) = row?;
        if let Some(date) = date_in_range(&created_at, offset, from, to) {
            days.entry(date).or_default().understanding.push((topic, state));
        }
    }
    Ok(())
}

/// その日の research-tree の出来事（1 日 1 行にする）。
///
/// 決めるのは 3 つ。
///
/// - DECISIONS（`rt_log` の `action` が `star`・`set_kind`・`remove`）
/// - 採用した PROPOSAL（`rt_proposals` の `status = accepted` で、その日 `decided_at` があるもの）
/// - SPROUT（`rt_nodes` の生きた `kind = 'S'` で、`created_at` がその日のもの）
///
/// 表が無い（新しい research-tree を始める前の DB）ときは何もしない。
fn gather_sci(
    conn: &Connection,
    days: &mut BTreeMap<NaiveDate, DayAccum>,
    from: NaiveDate,
    to: NaiveDate,
    offset: FixedOffset,
) -> rusqlite::Result<()> {
    if !rt_tables_exist(conn)? {
        return Ok(());
    }

    // Node 番号 → (ID, 文)（DECISIONS の行に出す）。
    let mut titles: HashMap<i64, (String, String)> = HashMap::new();
    {
        let mut stmt = conn.prepare("SELECT id, kind, text FROM rt_nodes")?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?;
        for row in rows {
            let (id, kind, text) = row?;
            titles.insert(id, (format!("{kind}{id}"), text));
        }
    }

    // 決定。
    {
        let mut stmt = conn.prepare(
            "SELECT node_id, created_at FROM rt_log
             WHERE action IN ('star', 'set_kind', 'remove')
             ORDER BY created_at ASC, id ASC",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })?;
        for row in rows {
            let (node_id, created_at) = row?;
            if let Some(date) = date_in_range(&created_at, offset, from, to) {
                let (id, title) = titles
                    .get(&node_id)
                    .cloned()
                    .unwrap_or_else(|| (format!("N{node_id}"), String::new()));
                days.entry(date)
                    .or_default()
                    .sci
                    .get_or_insert_with(SciLine::default)
                    .decisions
                    .push((id, title));
            }
        }
    }

    // 採用した PROPOSAL。
    {
        let mut stmt = conn.prepare(
            "SELECT decided_at FROM rt_proposals
             WHERE status = 'accepted' AND decided_at IS NOT NULL",
        )?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
        for row in rows {
            let decided_at = row?;
            if let Some(date) = date_in_range(&decided_at, offset, from, to) {
                days.entry(date)
                    .or_default()
                    .sci
                    .get_or_insert_with(SciLine::default)
                    .accepted += 1;
            }
        }
    }

    // SPROUT。
    {
        let mut stmt = conn.prepare("SELECT created_at FROM rt_nodes WHERE kind = 'S' AND removed_at IS NULL")?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
        for row in rows {
            let created_at = row?;
            if let Some(date) = date_in_range(&created_at, offset, from, to) {
                days.entry(date)
                    .or_default()
                    .sci
                    .get_or_insert_with(SciLine::default)
                    .sprouts += 1;
            }
        }
    }
    Ok(())
}

/// research-tree の表があるか（無い DB でも日誌は作り続けるため）。
fn rt_tables_exist(conn: &Connection) -> rusqlite::Result<bool> {
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master
         WHERE type = 'table' AND name IN ('rt_log', 'rt_proposals', 'rt_nodes')",
        [],
        |row| row.get(0),
    )?;
    Ok(count == 3)
}

impl SciLine {
    /// 1 日 1 行の文にする（0 件のものは書かない）。
    fn line(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        if !self.decisions.is_empty() {
            // 決定の名前は 3 つまで。4 つ以上は「など」を付ける。
            let names: Vec<String> = self
                .decisions
                .iter()
                .take(3)
                .map(|(id, title)| {
                    if title.trim().is_empty() {
                        id.clone()
                    } else {
                        format!("{id} {}", title.trim())
                    }
                })
                .collect();
            let names = names.join("・");
            let names = if self.decisions.len() > 3 {
                format!("{names}など")
            } else {
                names
            };
            parts.push(format!("決定 {} 件（{names}）", self.decisions.len()));
        }
        if self.accepted > 0 {
            parts.push(format!("採用 {} 件", self.accepted));
        }
        if self.sprouts > 0 {
            parts.push(format!("SPROUT {} 件", self.sprouts));
        }
        format!("research-tree: {}", parts.join("・"))
    }
}

/// sai-link の `run list --json` からエージェントの実行を集める。
///
/// 子の実行は親の行に `（子 N 件）` と添えるだけにする（親だけを並べる）。
fn gather_runs(
    roots: &PaperRoots,
    days: &mut BTreeMap<NaiveDate, DayAccum>,
    from: NaiveDate,
    to: NaiveDate,
    offset: FixedOffset,
) {
    let Ok(runs) = crate::link::runs(&roots.root) else {
        return;
    };

    let mut children: HashMap<i64, usize> = HashMap::new();
    for run in &runs {
        if let Some(parent) = run.parent_run_id {
            *children.entry(parent).or_default() += 1;
        }
    }

    let run_dirs: HashMap<i64, PathBuf> = runs
        .iter()
        .filter_map(|run| run.dir.clone().map(|dir| (run.run_id, dir)))
        .collect();

    for run in &runs {
        if run.parent_run_id.is_some() {
            continue;
        }
        // 時刻は `started_at`、空なら `created_at`。
        let time = if run.started_at.is_empty() {
            run.created_at.as_str()
        } else {
            run.started_at.as_str()
        };
        let Some(date) = date_in_range(time, offset, from, to) else {
            continue;
        };
        let run_id = run.run_id;
        let block = run.block.as_str();
        let dev = !runs::is_research_block(&roots.root, block);
        let files = run_dirs
            .get(&run_id)
            .map(|path| run_md_files(path))
            .unwrap_or_default();
        let mut text = String::new();
        if dev {
            text.push_str("（開発）");
        }
        text.push_str(&format!(
            "実行 run {run_id} {block}（{}）",
            status_label(&run.status)
        ));
        if let Some(count) = children.get(&run_id).copied().filter(|count| *count > 0) {
            text.push_str(&format!("（子 {count} 件）"));
        }
        if !files.is_empty() {
            let shown = files
                .iter()
                .take(MAX_RUN_FILES)
                .cloned()
                .collect::<Vec<_>>()
                .join("、");
            if files.len() > MAX_RUN_FILES {
                text.push_str(&format!(
                    ": {shown}…（他 {} 件）",
                    files.len() - MAX_RUN_FILES
                ));
            } else {
                text.push_str(&format!(": {shown}"));
            }
        }
        days.entry(date).or_default().runs.push(RunLine {
            run_id,
            text,
        });
    }
}

/// 実行フォルダの md（`task.md` などの引き継ぎ用を除く）を名前順で返す。
fn run_md_files(path: &Path) -> Vec<String> {
    let Ok(entries) = fs::read_dir(path) else {
        return Vec::new();
    };
    let mut files: Vec<String> = entries
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("md") {
                return None;
            }
            let name = path.file_name()?.to_string_lossy().to_string();
            if SKIPPED_RUN_FILES.contains(&name.as_str()) {
                return None;
            }
            Some(name)
        })
        .collect();
    files.sort();
    files
}

/// `reports` の絶対パスを、`activity_log` に書かれる相対パスへ直した対応表。
fn report_ids_by_path(conn: &Connection, roots: &PaperRoots) -> HashMap<String, String> {
    let mut map = HashMap::new();
    let Ok(mut stmt) = conn.prepare("SELECT id, md_path FROM reports") else {
        return map;
    };
    let Ok(rows) = stmt.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    }) else {
        return map;
    };
    for row in rows.flatten() {
        let (id, md_path) = row;
        let relative = activity::to_activity_path(&roots.root, &roots.data_dir, Path::new(&md_path));
        map.insert(relative, id);
    }
    map
}

/// `open` の `detail` から `(target, 表示名)` を作る。
fn open_entry(detail: &str, report_ids: &HashMap<String, String>) -> Option<(String, String)> {
    let value: serde_json::Value = serde_json::from_str(detail).ok()?;
    let target = value
        .get("target")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("report")
        .to_string();
    let path = value
        .get("path")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");
    let path_obj = Path::new(path);
    let file_name = path_obj
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_default();
    let label = match target.as_str() {
        "report" => report_ids
            .get(path)
            .cloned()
            .unwrap_or_else(|| file_name.clone()),
        "note" | "extract" => activity::paper_id_for_path(path_obj)
            .map(|id| format!("#{id}"))
            .unwrap_or_else(|| file_name.clone()),
        "canvas" => path_obj
            .file_stem()
            .map(|stem| stem.to_string_lossy().to_string())
            .unwrap_or_else(|| file_name.clone()),
        _ => file_name.clone(),
    };
    if label.is_empty() {
        return None;
    }
    Some((target, label))
}

/// 「開いた」の行を作る（mind / state はファイル名だけ、ほかは `target 表示名`）。
fn open_line(opens: &[(String, String)]) -> Option<String> {
    if opens.is_empty() {
        return None;
    }
    let mut by_target: HashMap<&str, Vec<String>> = HashMap::new();
    for (target, label) in opens {
        by_target.entry(target.as_str()).or_default().push(label.clone());
    }
    let mut targets: Vec<&str> = by_target.keys().copied().collect();
    targets.sort_by_key(|target| (target_rank(target), *target));
    let segments = targets
        .into_iter()
        .map(|target| {
            let labels = dedup_preserve(by_target.get(target).map(Vec::as_slice).unwrap_or(&[]));
            if matches!(target, "mind" | "state") {
                labels.join("、")
            } else {
                format!("{target} {}", labels.join("、"))
            }
        })
        .collect::<Vec<_>>()
        .join(" / ");
    Some(format!("開いた: {segments}"))
}

/// 「開いた」の並びの優先順（report を先、mind / state を後ろに）。
fn target_rank(target: &str) -> u8 {
    match target {
        "report" => 0,
        "note" => 1,
        "extract" => 2,
        "canvas" => 3,
        "review" => 4,
        "mind" => 5,
        "state" => 6,
        _ => 7,
    }
}

/// `DL 5 本: #40 #41` の形の行を作る（番号が無いものは件数だけ数える）。
fn paper_list_line(label: &str, ids: &[i64], unknown: usize) -> Option<String> {
    let ids = sorted_unique(ids);
    let total = ids.len() + unknown;
    if total == 0 {
        return None;
    }
    let mut line = format!("{label} {total} 本");
    if !ids.is_empty() {
        line.push_str(&format!(": {}", hash_list(&ids)));
    }
    Some(line)
}

/// 数字の一覧を `#40 #41` の形にする。
fn hash_list(ids: &[i64]) -> String {
    ids.iter()
        .map(|id| format!("#{id}"))
        .collect::<Vec<_>>()
        .join(" ")
}

/// `paper_id` があれば番号へ、無ければ「番号なし」の件数を増やす。
fn push_paper(ids: &mut Vec<i64>, unknown: &mut usize, paper_id: Option<i64>) {
    match paper_id {
        Some(id) => ids.push(id),
        None => *unknown += 1,
    }
}

/// 並べて重複を除く。
fn sorted_unique(ids: &[i64]) -> Vec<i64> {
    let mut ids = ids.to_vec();
    ids.sort_unstable();
    ids.dedup();
    ids
}

/// 順番を保ったまま重複を除く。
fn dedup_preserve(values: &[String]) -> Vec<String> {
    let mut seen: Vec<&str> = Vec::new();
    let mut unique: Vec<String> = Vec::new();
    for value in values {
        if seen.contains(&value.as_str()) {
            continue;
        }
        seen.push(value.as_str());
        unique.push(value.clone());
    }
    unique
}

/// 秒を分に直す（四捨五入）。
fn minutes(seconds: i64) -> i64 {
    if seconds <= 0 {
        return 0;
    }
    (seconds + 30) / 60
}

/// sai-link の実行の状態を日本語にする。
fn status_label(status: &str) -> String {
    match status {
        "completed" => "完了".to_string(),
        "running" => "実行中".to_string(),
        "failed" | "error" => "失敗".to_string(),
        "pending" => "待機".to_string(),
        "todo" => "未実行".to_string(),
        "cancelled" | "canceled" => "中止".to_string(),
        other => other.to_string(),
    }
}

/// 日付の曜日を日本語 1 文字で返す。
fn weekday_ja(date: NaiveDate) -> &'static str {
    match date.weekday() {
        chrono::Weekday::Mon => "月",
        chrono::Weekday::Tue => "火",
        chrono::Weekday::Wed => "水",
        chrono::Weekday::Thu => "木",
        chrono::Weekday::Fri => "金",
        chrono::Weekday::Sat => "土",
        chrono::Weekday::Sun => "日",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sai_paper_core::Database;

    /// テスト用の時差（日本標準時）。
    fn jst() -> FixedOffset {
        FixedOffset::east_opt(9 * 3600).unwrap()
    }

    fn date(year: i32, month: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(year, month, day).unwrap()
    }

    /// テスト用の一時プロジェクトルート（`.sai/paper` と `.sai/outputs` 付き）。
    fn temp_root(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "sai-paper-journal-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join(".sai/paper/papers")).unwrap();
        fs::create_dir_all(dir.join(".sai/outputs")).unwrap();
        dir
    }

    fn roots_of(root: &Path) -> PaperRoots {
        PaperRoots {
            root: root.to_path_buf(),
            data_dir: root.join(".sai/paper"),
        }
    }

    fn add_paper(conn: &Connection, id: i64, title: &str) {
        conn.execute(
            "INSERT INTO papers (id, title) VALUES (?1, ?2)",
            rusqlite::params![id, title],
        )
        .unwrap();
    }

    // ---- AC1: 日の分け方 ----

    /// UTC の記録が日本時間では翌日に入る。時差つきの記録はその時差で分ける。
    #[test]
    fn utc_record_falls_on_next_local_day_in_jst() {
        // 2026-09-24 16:00 UTC = 2026-09-25 01:00 JST。
        let time = parse_time_in("2026-09-24 16:00:00", jst()).unwrap();
        assert_eq!(time.date_naive(), date(2026, 9, 25));

        // 2026-09-24 14:59 UTC = 2026-09-24 23:59 JST（まだ 24 日）。
        let before = parse_time_in("2026-09-24 14:59:00", jst()).unwrap();
        assert_eq!(before.date_naive(), date(2026, 9, 24));

        // 時差つき（+09:00）はそのまま 9 月 23 日。
        let offset_time =
            parse_time_in("2026-09-23T21:49:41.474053+09:00", jst()).unwrap();
        assert_eq!(offset_time.date_naive(), date(2026, 9, 23));

        // 時差なしの ISO も UTC とみなす。
        let iso = parse_time_in("2026-09-24T16:00:00", jst()).unwrap();
        assert_eq!(iso.date_naive(), date(2026, 9, 25));

        // 読めない文字列は None。
        assert!(parse_time_in("昨日", jst()).is_none());
    }

    // ---- AC2: 束ね方 ----

    /// 同じ日の DL 5 件が 1 行に束ねられる。
    #[test]
    fn downloads_on_same_day_become_one_line() -> Result<(), Box<dyn std::error::Error>> {
        let root = temp_root("downloads");
        let db = Database::in_memory()?;
        let conn = db.connection();
        for id in 40..=44 {
            add_paper(conn, id, "x");
            conn.execute(
                "INSERT INTO activity_log (kind, paper_id, occurred_at)
                 VALUES ('download', ?1, '2026-09-25 03:00:00')",
                [id],
            )?;
        }

        let days = collect_at(
            conn,
            &roots_of(&root),
            date(2026, 9, 24),
            date(2026, 9, 26),
            jst(),
        )?;
        assert_eq!(days.len(), 1);
        assert_eq!(days[0].date, date(2026, 9, 25));
        assert_eq!(days[0].lines, vec!["DL 5 本: #40 #41 #42 #43 #44"]);

        fs::remove_dir_all(&root).ok();
        Ok(())
    }

    /// 読書時間が論文ごとに足される。
    #[test]
    fn reading_seconds_are_summed_per_paper() -> Result<(), Box<dyn std::error::Error>> {
        let root = temp_root("reading");
        let db = Database::in_memory()?;
        let conn = db.connection();
        add_paper(conn, 37, "x");
        add_paper(conn, 12, "y");
        // 37 は 600 秒 + 900 秒 = 1500 秒 = 25 分。12 は 180 秒 = 3 分。
        for (id, seconds) in [(37, 600), (37, 900), (12, 180)] {
            conn.execute(
                "INSERT INTO reading_history (paper_id, opened_at, duration_s)
                 VALUES (?1, '2026-09-25 03:00:00', ?2)",
                rusqlite::params![id, seconds],
            )?;
        }

        let days = collect_at(
            conn,
            &roots_of(&root),
            date(2026, 9, 25),
            date(2026, 9, 25),
            jst(),
        )?;
        assert_eq!(days[0].lines, vec!["読んだ論文: #12（3 分）、#37（25 分）"]);

        fs::remove_dir_all(&root).ok();
        Ok(())
    }

    /// 記録の無い日は出ない。
    #[test]
    fn days_without_records_are_skipped() -> Result<(), Box<dyn std::error::Error>> {
        let root = temp_root("empty");
        let db = Database::in_memory()?;
        let conn = db.connection();
        add_paper(conn, 1, "x");
        conn.execute(
            "INSERT INTO activity_log (kind, paper_id, occurred_at)
             VALUES ('download', 1, '2026-09-25 03:00:00')",
            [],
        )?;

        let days = collect_at(
            conn,
            &roots_of(&root),
            date(2026, 9, 20),
            date(2026, 9, 26),
            jst(),
        )?;
        assert_eq!(days.len(), 1);
        assert_eq!(days[0].date, date(2026, 9, 25));

        fs::remove_dir_all(&root).ok();
        Ok(())
    }

    /// 範囲の外は出ない。
    #[test]
    fn records_outside_range_are_excluded() -> Result<(), Box<dyn std::error::Error>> {
        let root = temp_root("range");
        let db = Database::in_memory()?;
        let conn = db.connection();
        add_paper(conn, 1, "x");
        // 範囲の外（9/23）と中（9/25）に 1 件ずつ。
        for occurred in ["2026-09-23 03:00:00", "2026-09-25 03:00:00"] {
            conn.execute(
                "INSERT INTO activity_log (kind, paper_id, occurred_at)
                 VALUES ('download', 1, ?1)",
                [occurred],
            )?;
        }

        let days = collect_at(
            conn,
            &roots_of(&root),
            date(2026, 9, 24),
            date(2026, 9, 26),
            jst(),
        )?;
        assert_eq!(days.len(), 1);
        assert_eq!(days[0].date, date(2026, 9, 25));

        fs::remove_dir_all(&root).ok();
        Ok(())
    }

    // ---- AC3: sai-link の DB が無いとき ----

    /// sai-link の DB が無くても、sai-paper の記録だけで作れる。
    #[test]
    fn works_without_sai_link_db() -> Result<(), Box<dyn std::error::Error>> {
        let root = temp_root("no-engine");
        // `.sai/sai-engine/` は作らない。
        let db = Database::in_memory()?;
        let conn = db.connection();

        let days = collect_at(
            conn,
            &roots_of(&root),
            date(2026, 9, 25),
            date(2026, 9, 25),
            jst(),
        )?;
        assert!(days.is_empty());

        fs::remove_dir_all(&root).ok();
        Ok(())
    }

    // ---- そのほかの材料 ----

    /// 検索・note・開いた・エージェントが読んだ・理解が 1 日の行になる。
    #[test]
    fn other_sources_become_lines() -> Result<(), Box<dyn std::error::Error>> {
        let root = temp_root("misc");
        let db = Database::in_memory()?;
        let conn = db.connection();
        add_paper(conn, 37, "x");
        add_paper(conn, 40, "y");
        conn.execute(
            "INSERT INTO search_sets (name, source_dbs, query, searched_at)
             VALUES ('s1', 's2', 'EEG preprocessing', '2026-09-25T09:00:00+09:00')",
            [],
        )?;
        for (kind, paper_id) in [("notes", Some(37)), ("agent_read", Some(40))] {
            conn.execute(
                "INSERT INTO activity_log (kind, paper_id, occurred_at)
                 VALUES (?1, ?2, '2026-09-25 04:00:00')",
                rusqlite::params![kind, paper_id],
            )?;
        }
        conn.execute(
            "INSERT INTO activity_log (kind, detail, occurred_at)
             VALUES ('open', '{\"target\":\"state\",\"path\":\".sai/paper/state.md\"}', '2026-09-25 04:00:00')",
            [],
        )?;
        conn.execute(
            "INSERT INTO understanding (topic, state, created_at)
             VALUES ('前処理の違い', 'vague', '2026-09-25 04:00:00')",
            [],
        )?;

        let days = collect_at(
            conn,
            &roots_of(&root),
            date(2026, 9, 25),
            date(2026, 9, 25),
            jst(),
        )?;
        let lines = &days[0].lines;
        assert!(lines.iter().any(|line| line == "検索 1 回: \"EEG preprocessing\""), "{lines:?}");
        assert!(lines.iter().any(|line| line == "note を書いた: #37"), "{lines:?}");
        assert!(lines.iter().any(|line| line == "開いた: state.md"), "{lines:?}");
        assert!(
            lines.iter().any(|line| line == "エージェントが読んだ論文: #40"),
            "{lines:?}"
        );
        assert!(
            lines.iter().any(|line| line == "理解の確認: 前処理の違い = あいまい"),
            "{lines:?}"
        );

        fs::remove_dir_all(&root).ok();
        Ok(())
    }

    /// S03: その日に rerun の記録があれば、検索の行に新着の数を添える。
    #[test]
    fn rerun_new_count_is_added_to_search_line() -> Result<(), Box<dyn std::error::Error>> {
        let root = temp_root("rerun");
        let db = Database::in_memory()?;
        let conn = db.connection();
        conn.execute(
            "INSERT INTO search_sets (id, name, source_dbs, query, searched_at)
             VALUES (3, 'eeg', 's2', 'EEG preprocessing', '2026-09-25T09:00:00+09:00')",
            [],
        )?;
        conn.execute(
            "INSERT INTO activity_log (kind, detail, occurred_at)
             VALUES ('discover', '{\"set_id\":3,\"rerun\":true,\"new\":5}', '2026-09-25 01:00:00')",
            [],
        )?;

        let days = collect_at(
            conn,
            &roots_of(&root),
            date(2026, 9, 25),
            date(2026, 9, 25),
            jst(),
        )?;
        let lines = &days[0].lines;
        assert!(
            lines
                .iter()
                .any(|line| line == "検索 1 回: \"EEG preprocessing\"（新着 5 本）"),
            "{lines:?}"
        );

        fs::remove_dir_all(&root).ok();
        Ok(())
    }

    /// sai-link の `run list --json` から実行を取り、開発用ブロックには印を付ける。
    #[test]
    fn runs_come_from_sai_link() -> Result<(), Box<dyn std::error::Error>> {
        let root = temp_root("runs");
        // 研究用ブロック（`project-brief` テンプレート付き）。
        fs::create_dir_all(root.join(".sai/blocks")).unwrap();
        fs::write(
            root.join(".sai/blocks/paper-library-qa.md"),
            "---\nname: paper-library-qa\ntemplates: project-brief\n---\n\n本文\n",
        )
        .unwrap();
        fs::create_dir_all(root.join(".sai/outputs/running/41")).unwrap();
        fs::write(root.join(".sai/outputs/running/41/answer.md"), "答\n").unwrap();
        crate::fake_link::write_runs(
            &root,
            &[
                serde_json::json!({
                    "run_id": 41, "block": "paper-library-qa", "tag": "",
                    "status": "completed", "lane": "running",
                    "parent_run_id": null, "global_round": "", "round": "",
                    "created_at": "2026-09-25 03:00:00",
                    "started_at": "2026-09-25 03:00:00",
                    "finished_at": "", "description": "",
                    "dir": root.join(".sai/outputs/running/41").display().to_string(),
                }),
                serde_json::json!({
                    "run_id": 42, "block": "main", "tag": "",
                    "status": "completed", "lane": "done",
                    "parent_run_id": null, "global_round": "", "round": "",
                    "created_at": "2026-09-25 05:00:00",
                    "started_at": "2026-09-25 05:00:00",
                    "finished_at": "", "description": "", "dir": null,
                }),
            ],
        );

        let db = Database::in_memory()?;
        let conn = db.connection();
        let days = collect_at(
            conn,
            &roots_of(&root),
            date(2026, 9, 25),
            date(2026, 9, 25),
            jst(),
        )?;
        let lines = &days[0].lines;
        assert!(
            lines
                .iter()
                .any(|line| line == "実行 run 41 paper-library-qa（完了）: answer.md"),
            "{lines:?}"
        );
        assert!(
            lines.iter().any(|line| line == "（開発）実行 run 42 main（完了）"),
            "{lines:?}"
        );

        fs::remove_dir_all(&root).ok();
        Ok(())
    }

    /// `render_markdown` が見出しと日を出す。
    #[test]
    fn render_markdown_has_heading_and_days() {
        let markdown = render_markdown(&[JournalDay {
            date: date(2026, 9, 25),
            lines: vec!["DL 1 本: #1".to_string()],
        }]);
        assert!(markdown.starts_with("# 研究日誌\n"));
        assert!(markdown.contains("## 2026-09-25（金）"), "{markdown}");
        assert!(markdown.contains("- DL 1 本: #1"), "{markdown}");

        let empty = render_markdown(&[]);
        assert!(empty.contains("（記録がありません）"), "{empty}");
    }

    // ---- research-tree の出来事 ----

    // ---- research-tree の出来事 ----

    #[test]
    fn journal_has_sci_line() {
        let database = Database::in_memory().unwrap();
        let conn = database.connection();
        let root = temp_root("sci");
        let roots = roots_of(&root);
        use crate::rt::store::{self, NewNode};
        use crate::rt::Kind;

        // Node を 2 つ作る（Q1・O2）。
        let mut ids = Vec::new();
        for (kind, text) in [(Kind::Q, "小目的"), (Kind::O, "手法")] {
            ids.push(
                store::insert_node(
                    conn,
                    &NewNode {
                        kind,
                        parent_id: ids.first().copied(),
                        guide_key: None,
                        text,
                        body: "",
                        answer_kind: (kind == Kind::O).then_some("answer"),
                        cites: &[],
                        paper_id: None,
                        run_id: None,
                        report_id: None,
                        mlflow_ref: None,
                        proposal_id: None,
                        paths: &[],
                        created_by: "user",
                    },
                )
                .unwrap(),
            );
        }

        // DECISIONS 2 件（追記だけなので時刻は今のままにする）。ほかの action は数えない。
        for node in &ids {
            store::insert_log(conn, *node, "star", "", "user", None).unwrap();
        }
        store::insert_log(conn, ids[0], "edit", "", "user", None).unwrap();
        // 採用した PROPOSAL 2 件と SPROUT 1 個。
        for _ in 0..2 {
            store::insert_proposal(
                conn,
                Some(120),
                Some("sci-compare"),
                "add_finding",
                None,
                &serde_json::json!({}),
                "auto",
                "pending",
            )
            .unwrap();
        }
        conn.execute(
            "UPDATE rt_proposals SET status = 'accepted', decided_by = 'user', decided_at = ?1",
            rusqlite::params![crate::rt::now_text()],
        )
        .unwrap();
        store::insert_node(
            conn,
            &crate::rt::store::NewNode {
                kind: crate::rt::Kind::S,
                parent_id: None,
                guide_key: None,
                text: "最初の SPROUT",
                body: "「頭皮脳波で小目的を行う」",
                answer_kind: None,
                cites: &[],
                paper_id: None,
                run_id: None,
                report_id: None,
                mlflow_ref: None,
                proposal_id: None,
                paths: &[],
                created_by: "user",
            },
        )
        .unwrap();

        // 記録は UTC なので JST では日付が変わることがある。2 日ぶん見て 1 行だけ出す。
        let today = Local::now().date_naive();
        let days = collect_at(
            conn,
            &roots,
            today - chrono::Days::new(1),
            today + chrono::Days::new(1),
            jst(),
        )
        .unwrap();
        let (day, line) = days
            .iter()
            .find_map(|day| {
                day.lines
                    .iter()
                    .find(|line| line.starts_with("research-tree: "))
                    .map(|line| (day.date, line))
            })
            .expect("research-tree の行が出る");
        assert!(
            day >= today - chrono::Days::new(1) && day <= today + chrono::Days::new(1),
            "{day}"
        );
        assert_eq!(
            line,
            "research-tree: 決定 2 件（Q1 小目的・O2 手法）・採用 2 件・SPROUT 1 件"
        );

        // 決定が 4 つ以上なら「など」を付ける。0 件のものは書かない。
        let many = SciLine {
            decisions: vec![
                ("N1".to_string(), "小目的".to_string()),
                ("N2".to_string(), "手法".to_string()),
                ("N3".to_string(), "差分".to_string()),
                ("N4".to_string(), "対象".to_string()),
            ],
            accepted: 0,
            sprouts: 0,
        };
        assert_eq!(many.line(), "research-tree: 決定 4 件（N1 小目的・N2 手法・N3 差分など）");
        assert_eq!(SciLine::default().line(), "research-tree: ");

        let _ = fs::remove_dir_all(&root);
    }
}
