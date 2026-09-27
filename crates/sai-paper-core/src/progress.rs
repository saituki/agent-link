//! プロジェクトの進み具合を LLM を使わずにまとめる（N05）。
//!
//! 材料は DB（`papr.db`）とファイルシステム（`.sai/paper/papers/` の note.md、
//! `.sai/outputs/` の引き継ぎ、`state.md`）にある。エージェント（推薦・理解の
//! 確認・DSR）が毎回自分で集めなくてよいよう、[`collect`] が 1 つの
//! [`Progress`] にまとめる。
//!
//! 「読むべき論文」（READ NEXT）は [`read_next`] が点数で並べる。点数は
//! 設定 `progress.read_next`（[`crate::config::ReadNextWeights`]）で変えられる。
//! **PDF が無い論文は外す**（まずダウンロードが要るため）。
//!
//! 速さのために、論文ごとに DB を引き直さない。集計クエリ（`GROUP BY`）を
//! 1 回ずつと、`papers/` を 1 回走査するだけで作る。

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use chrono::NaiveDateTime;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::activity::to_activity_path;
use crate::brief;
use crate::config::ReadNextWeights;
use crate::paths::PaperRoots;
use crate::project_files;
use crate::understanding::{self, UnderstandingEntry};

/// 「使った論文」の数え上げと、質問・未解決の切り出しに使う引き継ぎの件数。
const HANDOFF_SCAN: usize = 20;

/// 「ユーザーへの質問」「未解決」を出す引き継ぎの件数。
const QUESTION_HANDOFFS: usize = 5;

/// 進み具合の収集に失敗したときのエラー。
#[derive(Debug, Error)]
pub enum ProgressError {
    /// SQLite 操作に失敗した。
    #[error("database error: {0}")]
    Database(#[from] rusqlite::Error),
    /// 理解の記録の読み出しに失敗した。
    #[error("understanding error: {0}")]
    Understanding(#[from] understanding::UnderstandingError),
}

/// 論文 1 本の進み具合。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaperProgress {
    /// `papers.id`。
    pub id: i64,
    /// 論文タイトル。
    pub title: String,
    /// PDF があるか（`papers.pdf_path` が空でない）。
    pub has_pdf: bool,
    /// `note.md` の文字数（空白だけなら 0。ファイルが無ければ 0）。
    pub note_chars: usize,
    /// PDF を開いていた合計の秒数（`reading_history.duration_s` の合計）。
    pub reading_seconds: i64,
    /// 最後に PDF を開いた時刻（`reading_history.opened_at` の最大。無ければ `None`）。
    pub last_opened: Option<String>,
    /// エージェントが読んだ回数（`activity_log` の `agent_read` で、その論文のもの）。
    pub agent_reads: usize,
    /// 最近の引き継ぎ（最大 [`HANDOFF_SCAN`] 件）の「使った論文」に出た回数。
    pub handoff_mentions: usize,
    /// 検索セットでの判定（`pending` / `keep` / `drop` / `downloaded`。無ければ `None`）。
    ///
    /// 同じ論文に複数の判定があるときは `keep` > `downloaded` > `pending` > `drop` の
    /// 優先順で 1 つに決める（「keep なのに未読」を拾いやすくするため）。
    pub set_decision: Option<String>,
}

/// report 1 本の進み具合。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReportProgress {
    /// `reports.id`（例: `R-0007`）。
    pub id: String,
    /// md のファイル名（`reports.file_name`。無ければ `md_path` の末尾）。
    pub file: String,
    /// 作成時刻。
    pub created_at: String,
    /// ユーザーが開いた回数（`activity_log` の `open` で、その report のパスを指すもの）。
    pub user_opens: usize,
}

/// プロジェクト全体の進み具合。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Progress {
    /// `state.md` の段階（読めなければ `None`）。
    pub stage: Option<String>,
    /// 論文ごとの進み具合（`papers.id` の昇順）。
    pub papers: Vec<PaperProgress>,
    /// report ごとの進み具合（新しい順）。
    pub reports: Vec<ReportProgress>,
    /// 最近の引き継ぎの「ユーザーへの質問」`(run_id, 本文)`（新しい順）。
    pub open_questions: Vec<(i64, String)>,
    /// 最近の引き継ぎの「未解決」`(run_id, 本文)`（新しい順）。
    pub unresolved: Vec<(i64, String)>,
    /// トピックごとの理解の状態（[`understanding::latest_per_topic`] と同じ並び）。
    pub understanding: Vec<UnderstandingEntry>,
}

/// 「読むべき論文」1 本。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadNext {
    /// `papers.id`。
    pub paper_id: i64,
    /// 論文タイトル。
    pub title: String,
    /// 点数（[`read_next`] の計算結果）。
    pub score: i64,
    /// 当てはまった理由を「・」でつないだ 1 行。
    pub reason: String,
}

/// 進み具合を集める。
///
/// `roots.root` から引き継ぎ（`.sai/outputs/`）、`roots.data_dir` から
/// `state.md` と `papers/` の note を読む。ファイルが無い・読めない部分は
/// 空のまま返す（エラーにはしない）。
///
/// # Errors
///
/// DB の集計に失敗したとき。
pub fn collect(conn: &Connection, roots: &PaperRoots) -> Result<Progress, ProgressError> {
    let stage = project_files::read_state(&roots.data_dir).stage;

    // 引き継ぎ（最大 20 件）から「使った論文」の回数と、質問・未解決を取る。
    let handoffs = brief::recent_handoffs(&roots.root, HANDOFF_SCAN);
    let mut handoff_mentions: HashMap<i64, usize> = HashMap::new();
    for entry in &handoffs {
        for (name, value) in &entry.items {
            if name == "使った論文" {
                for id in paper_numbers(value) {
                    *handoff_mentions.entry(id).or_default() += 1;
                }
            }
        }
    }
    let mut open_questions: Vec<(i64, String)> = Vec::new();
    let mut unresolved: Vec<(i64, String)> = Vec::new();
    for entry in handoffs
        .iter()
        .filter(|entry| entry.readable)
        .take(QUESTION_HANDOFFS)
    {
        for (name, value) in &entry.items {
            let value = value.trim();
            if value.is_empty() || value == "なし" {
                continue;
            }
            match name.as_str() {
                "ユーザーへの質問" => {
                    open_questions.push((entry.run_id, value.to_string()))
                }
                "未解決" => unresolved.push((entry.run_id, value.to_string())),
                _ => {}
            }
        }
    }

    // 読書の記録（論文ごとの合計秒数と最後に開いた時刻）。
    let mut reading: HashMap<i64, (i64, Option<String>)> = HashMap::new();
    {
        let mut stmt = conn.prepare(
            "SELECT paper_id, COALESCE(SUM(duration_s), 0), MAX(opened_at)
             FROM reading_history GROUP BY paper_id",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, Option<String>>(2)?,
            ))
        })?;
        for row in rows {
            let (paper_id, seconds, last) = row?;
            reading.insert(paper_id, (seconds, last));
        }
    }

    // エージェントが読んだ回数（論文に紐づくものだけ）。
    let mut agent_reads: HashMap<i64, usize> = HashMap::new();
    {
        let mut stmt = conn.prepare(
            "SELECT paper_id, COUNT(*) FROM activity_log
             WHERE kind = 'agent_read' AND paper_id IS NOT NULL
             GROUP BY paper_id",
        )?;
        let rows = stmt.query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)))?;
        for row in rows {
            let (paper_id, count) = row?;
            agent_reads.insert(paper_id, usize::try_from(count).unwrap_or(0));
        }
    }

    // 検索セットの判定（論文ごとに 1 つへまとめる）。
    let set_decisions = collect_set_decisions(conn)?;

    // 論文の基本情報と note の文字数。
    let note_chars = note_chars_by_paper(&roots.data_dir);
    let mut papers: Vec<PaperProgress> = Vec::new();
    {
        let mut stmt = conn.prepare("SELECT id, title, pdf_path FROM papers ORDER BY id")?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
            ))
        })?;
        for row in rows {
            let (id, title, pdf_path) = row?;
            let (reading_seconds, last_opened) = reading.get(&id).cloned().unwrap_or((0, None));
            papers.push(PaperProgress {
                id,
                title,
                has_pdf: pdf_path.is_some_and(|path| !path.trim().is_empty()),
                note_chars: note_chars.get(&id).copied().unwrap_or(0),
                reading_seconds,
                last_opened,
                agent_reads: agent_reads.get(&id).copied().unwrap_or(0),
                handoff_mentions: handoff_mentions.get(&id).copied().unwrap_or(0),
                set_decision: set_decisions.get(&id).cloned(),
            });
        }
    }

    let reports = collect_reports(conn, roots)?;

    let understanding = understanding::latest_per_topic(conn)?;

    Ok(Progress {
        stage,
        papers,
        reports,
        open_questions,
        unresolved,
        understanding,
    })
}

/// 論文ごとの検索セットの判定を 1 つにまとめる。
///
/// 同じ論文に複数の判定があるときは `keep` > `downloaded` > `pending` > `drop` の
/// 優先順で決める。
fn collect_set_decisions(conn: &Connection) -> Result<HashMap<i64, String>, ProgressError> {
    let mut stmt =
        conn.prepare("SELECT paper_id, decision FROM search_set_items WHERE paper_id IS NOT NULL")?;
    let rows = stmt.query_map([], |row| {
        Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
    })?;
    let mut decisions: HashMap<i64, String> = HashMap::new();
    for row in rows {
        let (paper_id, decision) = row?;
        let entry = decisions
            .entry(paper_id)
            .or_insert_with(|| decision.clone());
        if decision_priority(&decision) < decision_priority(entry) {
            *entry = decision;
        }
    }
    Ok(decisions)
}

/// 判定の優先順（小さいほど優先）。
fn decision_priority(decision: &str) -> u8 {
    match decision {
        "keep" => 0,
        "downloaded" => 1,
        "pending" => 2,
        "drop" => 3,
        _ => 4,
    }
}

/// report ごとの進み具合（開かれた回数つき）を新しい順で返す。
fn collect_reports(
    conn: &Connection,
    roots: &PaperRoots,
) -> Result<Vec<ReportProgress>, ProgressError> {
    // report の一覧（file_name と run_id も取る）。
    let mut reports: Vec<(String, String, Option<String>, String)> = Vec::new();
    {
        let mut stmt = conn.prepare(
            "SELECT id, file_name, md_path, run_id, created_at
             FROM reports ORDER BY created_at DESC, id DESC",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, String>(4)?,
            ))
        })?;
        for row in rows {
            let (id, file_name, md_path, run_id, created_at) = row?;
            let file = file_name
                .filter(|name| !name.trim().is_empty())
                .unwrap_or_else(|| {
                    Path::new(&md_path)
                        .file_name()
                        .map(|name| name.to_string_lossy().to_string())
                        .unwrap_or_else(|| md_path.clone())
                });
            reports.push((id, file, run_id, created_at));
        }
    }

    // `(run_id, file_name)` と相対パスから report を引けるようにする。
    let mut by_run_file: HashMap<(String, String), String> = HashMap::new();
    let mut by_path: HashMap<String, String> = HashMap::new();
    for (id, file, run_id, _) in &reports {
        if let Some(run_id) = run_id {
            by_run_file
                .entry((run_id.clone(), file.clone()))
                .or_insert_with(|| id.clone());
        }
    }
    // md_path の相対パスは、md_path を持つ行から作る（file_name が無い行も拾うため）。
    {
        let mut stmt = conn.prepare("SELECT id, md_path FROM reports")?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        for row in rows {
            let (id, md_path) = row?;
            let relative = to_activity_path(&roots.root, &roots.data_dir, Path::new(&md_path));
            by_path.entry(relative).or_insert(id);
        }
    }

    // ユーザーが開いた記録を 1 回だけ走査し、report ごとに数える。
    let mut user_opens: HashMap<String, usize> = HashMap::new();
    {
        let mut stmt = conn.prepare("SELECT detail FROM activity_log WHERE kind = 'open'")?;
        let rows = stmt.query_map([], |row| row.get::<_, Option<String>>(0))?;
        for row in rows {
            let Some(detail) = row? else {
                continue;
            };
            let Some(path) = detail_path(&detail) else {
                continue;
            };
            let report_id = run_file_key(&path)
                .and_then(|key| by_run_file.get(&key).cloned())
                .or_else(|| by_path.get(&path).cloned());
            if let Some(report_id) = report_id {
                *user_opens.entry(report_id).or_default() += 1;
            }
        }
    }

    Ok(reports
        .into_iter()
        .map(|(id, file, _run_id, created_at)| ReportProgress {
            user_opens: user_opens.get(&id).copied().unwrap_or(0),
            id,
            file,
            created_at,
        })
        .collect())
}

/// `activity_log.detail` の JSON から `path` を取り出す。
fn detail_path(detail: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(detail).ok()?;
    value
        .get("path")
        .and_then(serde_json::Value::as_str)
        .filter(|path| !path.is_empty())
        .map(str::to_string)
}

/// 記録用の相対パス（`.sai/outputs/<owner>/<run_id>/<file>`）から
/// `(run_id, file_name)` を取り出す。形が違えば `None`。
fn run_file_key(path: &str) -> Option<(String, String)> {
    let parts: Vec<&str> = path.split('/').collect();
    let outputs = parts.iter().position(|part| *part == "outputs")?;
    let run_id = parts.get(outputs + 2)?;
    if run_id.parse::<i64>().is_err() {
        return None;
    }
    let file = parts.last()?;
    if file.is_empty() {
        return None;
    }
    Some(((*run_id).to_string(), (*file).to_string()))
}

/// 「読むべき論文」を点数で並べて上位 `limit` 本を返す。
///
/// 点数 = 引き継ぎに出た回数 × `weights.handoff_mention`
/// ＋（検索セットで `keep` なのに読書時間が 0 なら `weights.kept_but_unread`）
/// ＋（note が無ければ `weights.no_note`）
/// ＋（最後に開いてから `weights.stale_days` 日以上なら、または一度も開いていなければ
/// `weights.stale`）。**PDF が無い論文は外す。**
///
/// 点数が 0 以下になった論文は出さない。並びは点数の降順、同点は `paper_id` の昇順。
#[must_use]
pub fn read_next(progress: &Progress, weights: &ReadNextWeights, limit: usize) -> Vec<ReadNext> {
    let now = chrono::Utc::now().naive_utc();
    let mut scored: Vec<(i64, ReadNext)> = Vec::new();

    for paper in &progress.papers {
        if !paper.has_pdf {
            continue;
        }
        let mut score = 0i64;
        let mut reasons: Vec<String> = Vec::new();

        if paper.handoff_mentions > 0 && weights.handoff_mention != 0 {
            score +=
                i64::try_from(paper.handoff_mentions).unwrap_or(i64::MAX) * weights.handoff_mention;
            reasons.push(format!("引き継ぎで {} 回", paper.handoff_mentions));
        }
        if paper.set_decision.as_deref() == Some("keep")
            && paper.reading_seconds == 0
            && weights.kept_but_unread != 0
        {
            score += weights.kept_but_unread;
            reasons.push("残した候補・未読".to_string());
        }
        if paper.note_chars == 0 && weights.no_note != 0 {
            score += weights.no_note;
            reasons.push("note なし".to_string());
        }
        if is_stale(&paper.last_opened, now, weights.stale_days) && weights.stale != 0 {
            score += weights.stale;
            reasons.push(stale_reason(&paper.last_opened, weights.stale_days));
        }

        if score <= 0 {
            continue;
        }
        scored.push((
            score,
            ReadNext {
                paper_id: paper.id,
                title: paper.title.clone(),
                score,
                reason: reasons.join("・"),
            },
        ));
    }

    scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.paper_id.cmp(&b.1.paper_id)));
    scored
        .into_iter()
        .map(|(_, entry)| entry)
        .take(limit)
        .collect()
}

/// 最後に開いてから `stale_days` 日以上たっているか（一度も開いていなければ `true`）。
fn is_stale(last_opened: &Option<String>, now: NaiveDateTime, stale_days: i64) -> bool {
    match last_opened.as_deref().and_then(parse_timestamp) {
        Some(opened) => (now - opened).num_days() >= stale_days,
        None => true,
    }
}

/// 古いときの理由の 1 文。
fn stale_reason(last_opened: &Option<String>, stale_days: i64) -> String {
    if last_opened.is_none() {
        "まだ開いていない".to_string()
    } else {
        format!("{stale_days} 日以上開いていない")
    }
}

/// 保存されている時刻の文字列を読む（`CURRENT_TIMESTAMP` と ISO8601 の両方）。
fn parse_timestamp(value: &str) -> Option<NaiveDateTime> {
    NaiveDateTime::parse_from_str(value, "%Y-%m-%d %H:%M:%S")
        .ok()
        .or_else(|| NaiveDateTime::parse_from_str(value, "%Y-%m-%dT%H:%M:%S").ok())
        .or_else(|| {
            chrono::DateTime::parse_from_rfc3339(value)
                .ok()
                .map(|datetime| datetime.naive_utc())
        })
}

/// 文中の `#<番号>` を全部読む。
///
/// 番号は `#` のあとに続く数字を**最後まで**取る。`#370` は 370 であり、
/// 37 ではない。`#37,` や `#37 ` は 37。
fn paper_numbers(text: &str) -> Vec<i64> {
    let bytes = text.as_bytes();
    let mut numbers = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] != b'#' {
            index += 1;
            continue;
        }
        let start = index + 1;
        let mut end = start;
        while end < bytes.len() && bytes[end].is_ascii_digit() {
            end += 1;
        }
        if end > start {
            if let Ok(number) = text[start..end].parse::<i64>() {
                numbers.push(number);
            }
            index = end;
        } else {
            // 数字が無いときは `#` の次から続ける（`##37` を取りこぼさない）。
            index = start;
        }
    }
    numbers
}

/// `papers/` の論文フォルダを 1 回だけ走査し、`papers.id` → note.md の文字数を作る。
///
/// フォルダ名は `{id:04}` か `{id:04}_...`。同じ番号が複数あれば、名前の
/// 辞書順で最初のものを [`crate::paths::paper_folder`] と同じ規則で選ぶ。
fn note_chars_by_paper(data_dir: &Path) -> HashMap<i64, usize> {
    let mut folders: HashMap<i64, (String, PathBuf)> = HashMap::new();
    let papers_root = data_dir.join("papers");
    if let Ok(entries) = fs::read_dir(&papers_root) {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().to_string();
            let Some(id) = paper_id_from_folder(&name) else {
                continue;
            };
            match folders.get(&id) {
                Some((known, _)) if *known <= name => {}
                _ => {
                    folders.insert(id, (name, path));
                }
            }
        }
    }

    let mut note_chars: HashMap<i64, usize> = HashMap::new();
    for (id, (_, folder)) in folders {
        if let Ok(text) = fs::read_to_string(folder.join(crate::paths::paper_file_names::NOTE)) {
            note_chars.insert(id, text.trim().chars().count());
        }
    }
    note_chars
}

/// `0030` / `0030_Title` のフォルダ名から `papers.id` を読む。
///
/// `00301_Title` のような余分な桁は拾わない（[`crate::paths::paper_folder`] と同じ規則）。
fn paper_id_from_folder(name: &str) -> Option<i64> {
    let prefix: String = name.chars().take(4).collect();
    if prefix.len() != 4 || !prefix.chars().all(|ch| ch.is_ascii_digit()) {
        return None;
    }
    let rest = &name[prefix.len()..];
    if !rest.is_empty() && !rest.starts_with('_') {
        return None;
    }
    prefix.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Database;

    /// テスト用の一時プロジェクトルート（`.sai/paper` と `.sai/outputs` 付き）。
    fn temp_root(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "sai-paper-progress-{tag}-{}-{:?}",
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

    fn write(path: PathBuf, text: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    fn add_paper(conn: &Connection, id: i64, title: &str, pdf: Option<&str>) {
        conn.execute(
            "INSERT INTO papers (id, title, pdf_path) VALUES (?1, ?2, ?3)",
            rusqlite::params![id, title, pdf],
        )
        .unwrap();
    }

    fn write_note(root: &Path, id: i64, title: &str, text: &str) {
        let folder = root
            .join(".sai/paper/papers")
            .join(format!("{id:04}_{title}"));
        write(folder.join("note.md"), text);
    }

    fn write_run(root: &Path, owner: &str, run_id: i64, block: &str) {
        let dir = root
            .join(".sai/outputs")
            .join(owner)
            .join(run_id.to_string());
        write(
            dir.join("task.md"),
            &format!("---\nrun_id: {run_id}\nblock: {block}\n---\n\nタスク\n"),
        );
    }

    fn write_block(root: &Path, id: &str) {
        write(
            root.join(".sai/blocks").join(format!("{id}.md")),
            &format!("---\nname: {id}\ntemplates: project-brief\n---\n\n本文\n"),
        );
    }

    // ---- AC2: #37 の数え方 ----

    /// `#370` を 37 と数えない。`#37,` と `#37 ` は 37。
    #[test]
    fn paper_numbers_reads_full_digit_run() {
        assert_eq!(paper_numbers("#370"), vec![370]);
        assert_eq!(paper_numbers("#37, #40"), vec![37, 40]);
        assert_eq!(paper_numbers("#37 と#12"), vec![37, 12]);
        assert_eq!(paper_numbers("なし"), Vec::<i64>::new());
        // 最大連続の数字を取るので、`#370` の中に 37 は現れない。
        assert!(!paper_numbers("#370").contains(&37));
    }

    /// 引き継ぎの「使った論文」が論文ごとの回数になる。
    #[test]
    fn collect_counts_handoff_mentions() -> Result<(), Box<dyn std::error::Error>> {
        let root = temp_root("mentions");
        write_block(&root, "paper-read");
        write_run(&root, "running", 41, "paper-read");
        write(
            root.join(".sai/outputs/running/41/answer.md"),
            "## 引き継ぎ\n- 使った論文: #370, #37, #37\n- 未解決: 前処理の手順\n- ユーザーへの質問: 対象は?\n",
        );
        let roots = roots_of(&root);
        let db = Database::in_memory()?;
        let conn = db.connection();

        let progress = collect(conn, &roots)?;
        // 論文は 1 本も登録していないので papers は空。質問・未解決だけが入る。
        assert!(progress.papers.is_empty());
        assert_eq!(progress.unresolved, vec![(41, "前処理の手順".to_string())]);
        assert_eq!(progress.open_questions, vec![(41, "対象は?".to_string())]);

        fs::remove_dir_all(&root).ok();
        Ok(())
    }

    /// 論文を登録した状態で、引き継ぎの回数が [`PaperProgress`] に入る。
    #[test]
    fn handoff_mentions_reach_registered_papers() -> Result<(), Box<dyn std::error::Error>> {
        let root = temp_root("mentions-paper");
        write_block(&root, "paper-read");
        write_run(&root, "running", 41, "paper-read");
        write(
            root.join(".sai/outputs/running/41/answer.md"),
            "## 引き継ぎ\n- 使った論文: #370, #37, #37\n",
        );
        let roots = roots_of(&root);
        let db = Database::in_memory()?;
        let conn = db.connection();
        add_paper(conn, 37, "Thirty seven", Some("papers/0037_x/paper.pdf"));
        add_paper(conn, 370, "Three seventy", Some("papers/0370_x/paper.pdf"));

        let progress = collect(conn, &roots)?;
        let by_id: HashMap<i64, usize> = progress
            .papers
            .iter()
            .map(|paper| (paper.id, paper.handoff_mentions))
            .collect();
        assert_eq!(by_id.get(&37), Some(&2));
        assert_eq!(by_id.get(&370), Some(&1));

        fs::remove_dir_all(&root).ok();
        Ok(())
    }

    // ---- AC1: 点数と並び ----

    fn weights() -> ReadNextWeights {
        ReadNextWeights::default()
    }

    /// 点数・順位・理由が決まる。PDF の無い論文は外れる。
    #[test]
    fn read_next_scores_and_excludes_without_pdf() {
        let progress = Progress {
            stage: None,
            papers: vec![
                PaperProgress {
                    id: 1,
                    title: "handoff many times".into(),
                    has_pdf: true,
                    note_chars: 0,
                    reading_seconds: 0,
                    last_opened: None,
                    agent_reads: 0,
                    handoff_mentions: 3,
                    set_decision: None,
                },
                PaperProgress {
                    id: 2,
                    title: "kept but unread".into(),
                    has_pdf: true,
                    note_chars: 5,
                    // 一度開いているので stale しない（30 日以内にした）。
                    reading_seconds: 0,
                    last_opened: Some(
                        chrono::Utc::now()
                            .naive_utc()
                            .format("%Y-%m-%d %H:%M:%S")
                            .to_string(),
                    ),
                    agent_reads: 0,
                    handoff_mentions: 0,
                    set_decision: Some("keep".into()),
                },
                PaperProgress {
                    id: 3,
                    title: "no pdf".into(),
                    has_pdf: false,
                    note_chars: 0,
                    reading_seconds: 0,
                    last_opened: None,
                    agent_reads: 0,
                    handoff_mentions: 5,
                    set_decision: None,
                },
                PaperProgress {
                    id: 4,
                    title: "note only".into(),
                    has_pdf: true,
                    note_chars: 0,
                    reading_seconds: 0,
                    last_opened: None,
                    agent_reads: 0,
                    handoff_mentions: 0,
                    set_decision: None,
                },
            ],
            reports: Vec::new(),
            open_questions: Vec::new(),
            unresolved: Vec::new(),
            understanding: Vec::new(),
        };

        let ranked = read_next(&progress, &weights(), 5);

        // 論文 1: 3*3(handoff) + 1(no_note) + 1(stale) = 11
        // 論文 2: 2(kept) = 2
        // 論文 4: 1(no_note) + 1(stale) = 2（同点は id 昇順なので 2 が先）
        // 論文 3 は PDF が無いので出ない。
        let ids: Vec<i64> = ranked.iter().map(|entry| entry.paper_id).collect();
        assert_eq!(ids, vec![1, 2, 4]);
        assert_eq!(ranked[0].score, 11);
        assert_eq!(
            ranked[0].reason,
            "引き継ぎで 3 回・note なし・まだ開いていない"
        );
        assert_eq!(ranked[1].score, 2);
        assert_eq!(ranked[1].reason, "残した候補・未読");
        assert_eq!(ranked[2].score, 2);
        assert_eq!(ranked[2].reason, "note なし・まだ開いていない");
    }

    /// 重みを変えると順位が変わる。
    #[test]
    fn read_next_ranking_changes_with_weights() {
        let mut handoff = weights();
        handoff.handoff_mention = 1;
        handoff.no_note = 5;
        handoff.stale = 0;
        let progress = Progress {
            stage: None,
            papers: vec![
                PaperProgress {
                    id: 1,
                    title: "handoff".into(),
                    has_pdf: true,
                    note_chars: 10,
                    reading_seconds: 0,
                    last_opened: None,
                    agent_reads: 0,
                    handoff_mentions: 3,
                    set_decision: None,
                },
                PaperProgress {
                    id: 2,
                    title: "note".into(),
                    has_pdf: true,
                    note_chars: 0,
                    reading_seconds: 0,
                    last_opened: None,
                    agent_reads: 0,
                    handoff_mentions: 0,
                    set_decision: None,
                },
            ],
            reports: Vec::new(),
            open_questions: Vec::new(),
            unresolved: Vec::new(),
            understanding: Vec::new(),
        };
        // handoff_mention=1 なので論文 1 は 3、note の重み 5 で論文 2 は 5 → 2 が先。
        let ranked = read_next(&progress, &handoff, 5);
        assert_eq!(ranked[0].paper_id, 2);
        assert_eq!(ranked[0].score, 5);
        assert_eq!(ranked[1].paper_id, 1);

        // 既定の重みなら handoff の方が強い。
        let ranked_default = read_next(&progress, &weights(), 5);
        assert_eq!(ranked_default[0].paper_id, 1);
    }

    /// 点数 0 は出さない。
    #[test]
    fn read_next_drops_zero_score() {
        let progress = Progress {
            stage: None,
            papers: vec![PaperProgress {
                id: 1,
                title: "nothing".into(),
                has_pdf: true,
                note_chars: 10,
                reading_seconds: 30,
                last_opened: Some(
                    chrono::Utc::now()
                        .naive_utc()
                        .format("%Y-%m-%d %H:%M:%S")
                        .to_string(),
                ),
                agent_reads: 1,
                handoff_mentions: 0,
                set_decision: Some("drop".into()),
            }],
            reports: Vec::new(),
            open_questions: Vec::new(),
            unresolved: Vec::new(),
            understanding: Vec::new(),
        };
        assert!(read_next(&progress, &weights(), 5).is_empty());
    }

    // ---- collect: note と report ----

    /// note の文字数と、PDF の有無が入る。
    #[test]
    fn collect_reads_note_and_pdf() -> Result<(), Box<dyn std::error::Error>> {
        let root = temp_root("note");
        let db = Database::in_memory()?;
        let conn = db.connection();
        add_paper(conn, 1, "with note", Some("papers/0001_x/paper.pdf"));
        add_paper(conn, 2, "empty note", Some("papers/0002_x/paper.pdf"));
        add_paper(conn, 3, "no pdf", None);
        write_note(&root, 1, "x", "  メモです  \n");
        write_note(&root, 2, "y", "   \n");

        let progress = collect(conn, &roots_of(&root))?;
        let by_id: HashMap<i64, &PaperProgress> =
            progress.papers.iter().map(|p| (p.id, p)).collect();
        assert_eq!(by_id[&1].note_chars, "メモです".chars().count());
        assert_eq!(by_id[&2].note_chars, 0, "空白だけの note は 0");
        assert!(by_id[&1].has_pdf);
        assert!(!by_id[&3].has_pdf);

        fs::remove_dir_all(&root).ok();
        Ok(())
    }

    /// report が開かれた回数をパスから数える。
    #[test]
    fn collect_counts_report_opens() -> Result<(), Box<dyn std::error::Error>> {
        let root = temp_root("report");
        let db = Database::in_memory()?;
        let conn = db.connection();
        // report の md を outputs に置く（相対パスは .sai/outputs/done/41/answer.md）。
        write(
            root.join(".sai/outputs/done/41/answer.md"),
            "# report\n本文\n",
        );
        conn.execute(
            "INSERT INTO reports (id, file_name, md_path, run_id, created_at)
             VALUES ('R-0001', 'answer.md', ?1, '41', '2026-09-25 12:00:00')",
            [root
                .join(".sai/outputs/done/41/answer.md")
                .to_string_lossy()
                .to_string()],
        )?;
        // 開いた記録を 2 件（相対パス）＋別の report を 1 件。
        for _ in 0..2 {
            conn.execute(
                "INSERT INTO activity_log (kind, detail) VALUES ('open', ?1)",
                [r#"{"target":"report","path":".sai/outputs/done/41/answer.md","surface":"tui"}"#],
            )?;
        }
        conn.execute(
            "INSERT INTO activity_log (kind, detail) VALUES ('open', ?1)",
            [r#"{"target":"report","path":".sai/outputs/done/99/other.md","surface":"tui"}"#],
        )?;

        let progress = collect(conn, &roots_of(&root))?;
        assert_eq!(progress.reports.len(), 1);
        assert_eq!(progress.reports[0].id, "R-0001");
        assert_eq!(progress.reports[0].file, "answer.md");
        assert_eq!(progress.reports[0].user_opens, 2);

        fs::remove_dir_all(&root).ok();
        Ok(())
    }
}
