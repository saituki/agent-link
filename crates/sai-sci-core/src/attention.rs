//! 「要対応」の一覧（S02）。
//!
//! ユーザーが「今やること」だけを 1 か所で見られるように、次の 5 種類を集める。
//!
//! 1. `EmptyReview` — 書いていない review（研究用ブロックの実行が終わったのに
//!    `review.md` が空）
//! 2. `Question` — 引き継ぎの「ユーザーへの質問」（最近 5 件の引き継ぎから）
//! 3. `StateUpdate` — 引き継ぎに「現況の更新案」があるのに `state.md` がその後に
//!    直されていない
//! 4. `RecommendCard` — todo に残っている推薦の CARD（タグ `推薦`）
//! 5. `ReportIssue` — 出典の要確認（report の問題のうち `error` のもの）
//!
//! LLM は使わない。ファイル（`.sai/outputs/`・`.sai/paper/state.md`）と
//! `papr.db` を読むだけである。
//!
//! 「対応済み」にした記録は `activity_log` の `resolved` に残す。`detail` の JSON は
//! `{"key": "<[`AttentionItem::key`]>", "text": "<その時の本文の先頭 200 字>"}`。
//! 質問と現況の更新案は、答えたかどうかを記録から判断できないので `x` で消す。
//! review・CARD・出典は元が片付けば自然に消える（`resolved` でも消せる）。

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::brief;
use sai_paper_core::database::Database;
use crate::handoff;
use sai_paper_core::paths::{PaperRoots, state_path};
use crate::progress;
use crate::reports;
use crate::runs::{RunDir, is_research_block, list_run_dirs, run_block_id};

/// 要対応の種類。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttentionKind {
    /// 書いていない review。
    EmptyReview,
    /// 引き継ぎの「ユーザーへの質問」。
    Question,
    /// 引き継ぎの「現況の更新案」が `state.md` に反映されていない。
    StateUpdate,
    /// todo に残っている推薦の CARD。
    RecommendCard,
    /// report の出典の要確認（`error`）。
    ReportIssue,
    /// research-tree の判断待ち（★ は目印で計算に使わないので、今は作られない）。
    SciDecision,
}

impl AttentionKind {
    /// 一覧の印（`[review]` など）に使う短い名前。
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::EmptyReview => "review",
            Self::Question => "質問",
            Self::StateUpdate => "現況",
            Self::RecommendCard => "推薦",
            Self::ReportIssue => "出典",
            Self::SciDecision => "AWAITING_DECISION",
        }
    }

    /// `--json` に出す名前（共有契約）。
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::EmptyReview => "empty_review",
            Self::Question => "question",
            Self::StateUpdate => "state_update",
            Self::RecommendCard => "recommend_card",
            Self::ReportIssue => "report_issue",
            Self::SciDecision => "sci_decision",
        }
    }
}

/// 要対応の 1 件。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttentionItem {
    /// 対応済み（`resolved`）と照らすキー。決め方は共有契約で固定する。
    pub key: String,
    /// 種類。
    pub kind: AttentionKind,
    /// 一覧に出す題。
    pub title: String,
    /// 元の実行 ID（無ければ `None`）。
    pub run_id: Option<i64>,
    /// `Enter` で開くファイル（無ければ `None`）。
    pub open_path: Option<PathBuf>,
    /// 出典の要確認のときの report ID。
    pub report_id: Option<String>,
}

/// 要対応の収集・記録のエラー。
#[derive(Debug, Error)]
pub enum AttentionError {
    /// データベースの操作に失敗した。
    #[error("database error: {0}")]
    Database(#[from] sai_paper_core::database::DatabaseError),
    /// SQLite の操作に失敗した。
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    /// 引き継ぎの読み出しに失敗した。
    #[error("progress error: {0}")]
    Progress(#[from] progress::ProgressError),
    /// todo CARD の読み出しに失敗した。
    #[error("handoff error: {0}")]
    Handoff(#[from] handoff::HandoffError),
    /// report の問題の読み出しに失敗した。
    #[error("report error: {0}")]
    Report(#[from] reports::ReportError),
    /// `resolved` の `detail` の JSON の読み書きに失敗した。
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    /// research-tree の読み出しに失敗した。
    #[error("rt error: {0}")]
    Rt(#[from] crate::rt::RtError),
}

/// 書いていない review の上限（新しい順）。
const EMPTY_REVIEW_LIMIT: usize = 10;

/// 引き継ぎを走査する件数（[`progress::collect`] と同じ）。
const HANDOFF_SCAN: usize = 20;

/// 要対応を集める。並びは 1 → 2 → 3 → 4 → 5 → 6 → 7 → 8（種類の中は新しい順）。
///
/// research-tree があるときは 3（現況の更新案）を集めない。`state.md` は research-tree から書き出すので、
/// 人が直す余地が無いから。代わりに 7 と 8 を足す。
///
/// # Errors
///
/// DB・ファイルの読み出しに失敗したとき。
pub fn collect(
    conn: &Connection,
    roots: &PaperRoots,
) -> Result<Vec<AttentionItem>, AttentionError> {
    let resolved = resolved_keys(conn)?;
    let mut items = Vec::new();
    // research-tree があるときは「現況の更新案」（3）を出さない。
    let tree = sci_tree(conn, roots)?;

    for item in empty_reviews(&roots.root) {
        if !resolved.contains(&item.key) {
            items.push(item);
        }
    }
    for item in questions(conn, roots)? {
        if !resolved.contains(&item.key) {
            items.push(item);
        }
    }
    if tree.is_none()
        && let Some(item) = state_update(&roots.root, &roots.data_dir)
        && !resolved.contains(&item.key)
    {
        items.push(item);
    }
    for item in recommend_cards(roots)? {
        if !resolved.contains(&item.key) {
            items.push(item);
        }
    }
    for item in report_issues(conn)? {
        if !resolved.contains(&item.key) {
            items.push(item);
        }
    }
    Ok(items)
}

/// research-tree があれば `status` を返す。始めていなければ `None`。
fn sci_tree(
    conn: &Connection,
    roots: &PaperRoots,
) -> Result<Option<crate::rt::compute::Status>, AttentionError> {
    if !crate::rt::store::is_initialized(conn)? {
        return Ok(None);
    }
    let defs = crate::rt::defs::load(&roots.root);
    Ok(Some(crate::rt::compute::status(conn, roots, &defs)?))
}

/// 要対応 1 件を「対応済み」にする（`activity_log` に `resolved` を残す）。
///
/// # Errors
///
/// DB の操作に失敗したとき。
pub fn resolve(db: &Database, item: &AttentionItem) -> Result<(), AttentionError> {
    let text: String = item.title.chars().take(200).collect();
    let detail = serde_json::json!({ "key": item.key, "text": text }).to_string();
    db.record_activity("resolved", None, Some(&detail))?;
    Ok(())
}

/// `resolved` の `detail` から、対応済みのキーを集める。
fn resolved_keys(conn: &Connection) -> Result<HashSet<String>, AttentionError> {
    let mut stmt = conn.prepare("SELECT detail FROM activity_log WHERE kind = 'resolved'")?;
    let rows = stmt.query_map([], |row| row.get::<_, Option<String>>(0))?;
    let mut keys = HashSet::new();
    for row in rows {
        let Some(detail) = row? else {
            continue;
        };
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(&detail)
            && let Some(key) = value.get("key").and_then(serde_json::Value::as_str)
            && !key.is_empty()
        {
            keys.insert(key.to_string());
        }
    }
    Ok(keys)
}

/// 1. 書いていない review（新しい順に最大 [`EMPTY_REVIEW_LIMIT`] 件）。
pub fn empty_reviews(root: &Path) -> Vec<AttentionItem> {
    let mut found: Vec<(SystemTime, AttentionItem)> = Vec::new();
    for (run, block_id) in research_runs(root) {
        // まだ終わっていない実行（todo / running）は除く。
        if run.owner == "todo" || run.owner == "running" {
            continue;
        }
        let path = run.path.join("review.md");
        let Ok(text) = fs::read_to_string(&path) else {
            continue;
        };
        let body = strip_frontmatter(&text).trim();
        if review_has_content(body) {
            continue;
        }
        found.push((
            modified_at(&path),
            AttentionItem {
                key: format!("review:{}", run.run_id),
                kind: AttentionKind::EmptyReview,
                title: format!("run {} / {} の review が未記入", run.run_id, block_id),
                run_id: Some(run.run_id),
                open_path: Some(path),
                report_id: None,
            },
        ));
    }
    // 種類の中は新しい順。
    found.sort_by_key(|(modified, item)| (std::cmp::Reverse(*modified), item.run_id));
    found
        .into_iter()
        .take(EMPTY_REVIEW_LIMIT)
        .map(|(_, item)| item)
        .collect()
}

/// 2. 引き継ぎの「ユーザーへの質問」（[`progress::collect`] の `open_questions`）。
fn questions(conn: &Connection, roots: &PaperRoots) -> Result<Vec<AttentionItem>, AttentionError> {
    let progress = progress::collect(conn, roots)?;
    // run_id → 引き継ぎの md（新しい方を採用）。
    let mut files: HashMap<i64, PathBuf> = HashMap::new();
    for entry in brief::recent_handoffs(&roots.root, HANDOFF_SCAN) {
        files.entry(entry.run_id).or_insert(entry.file);
    }
    Ok(progress
        .open_questions
        .into_iter()
        .map(|(run_id, text)| {
            let head: String = text.chars().take(64).collect();
            AttentionItem {
                key: format!("question:{run_id}:{head}"),
                kind: AttentionKind::Question,
                title: format!("run {run_id} の質問: {text}"),
                run_id: Some(run_id),
                open_path: files.get(&run_id).cloned(),
                report_id: None,
            }
        })
        .collect())
}

/// 3. 「現況の更新案」があるのに `state.md` がその後に直されていない（最新の 1 件）。
fn state_update(root: &Path, data_dir: &Path) -> Option<AttentionItem> {
    let state_file = state_path(data_dir);
    let state_modified = modified_at(&state_file);
    brief::recent_handoffs(root, HANDOFF_SCAN)
        .into_iter()
        .find(|entry| entry.state_update().is_some() && entry.modified > state_modified)
        .map(|entry| {
            let update = entry.state_update().unwrap_or_default().to_string();
            AttentionItem {
                key: format!("state:{}", entry.run_id),
                kind: AttentionKind::StateUpdate,
                title: format!("run {} の現況の更新案: {update}", entry.run_id),
                run_id: Some(entry.run_id),
                open_path: Some(state_file),
                report_id: None,
            }
        })
}

/// 4. todo に残っている推薦の CARD（`run_id` の新しい順）。
fn recommend_cards(roots: &PaperRoots) -> Result<Vec<AttentionItem>, AttentionError> {
    let mut cards = handoff::list_todo_cards(roots, Some("推薦"))?;
    cards.sort_by_key(|card| std::cmp::Reverse(card.run_id));
    Ok(cards
        .into_iter()
        .map(|card| {
            let description = if card.description.trim().is_empty() {
                card.block.clone()
            } else {
                card.description.clone()
            };
            AttentionItem {
                key: format!("card:{}", card.run_id),
                kind: AttentionKind::RecommendCard,
                title: format!("推薦の CARD: {description}（run {}）", card.run_id),
                run_id: Some(card.run_id),
                open_path: Some(card.path),
                report_id: None,
            }
        })
        .collect())
}

/// 5. report の出典の要確認（`level = 'error'` を report ごとに 1 件）。
fn report_issues(conn: &Connection) -> Result<Vec<AttentionItem>, AttentionError> {
    let mut items = Vec::new();
    for row in reports::list_issues(conn, None)? {
        let Some(first) = row.issues.iter().find(|issue| issue.level == "error") else {
            continue;
        };
        let count = row
            .issues
            .iter()
            .filter(|issue| issue.level == "error")
            .count();
        let line = first.line.map_or_else(String::new, |line| line.to_string());
        items.push(AttentionItem {
            key: format!("issue:{}:{}:{}", row.report.id, first.kind, line),
            kind: AttentionKind::ReportIssue,
            title: format!("{} に要確認 {} 件", row.report.id, count),
            run_id: row.report.run_id.as_deref().and_then(|id| id.parse().ok()),
            open_path: None,
            report_id: Some(row.report.id.clone()),
        });
    }
    Ok(items)
}

/// 研究用ブロックの実行だけを `(実行, ブロック ID)` の形で返す。
fn research_runs(root: &Path) -> Vec<(RunDir, String)> {
    let mut cache: HashMap<String, bool> = HashMap::new();
    let mut runs = Vec::new();
    for run in list_run_dirs(root) {
        let Some(block_id) = run_block_id(root, &run) else {
            continue;
        };
        let research = *cache
            .entry(block_id.clone())
            .or_insert_with(|| is_research_block(root, &block_id));
        if research {
            runs.push((run, block_id));
        }
    }
    runs
}

/// `review.md` の frontmatter（先頭の `---` から次の `---` まで）を外す。
pub(crate) fn strip_frontmatter(text: &str) -> &str {
    let first_end = text.find('\n').map_or(text.len(), |index| index + 1);
    if text[..first_end].trim() != "---" {
        return text;
    }
    let mut offset = first_end;
    for line in text[first_end..].split_inclusive('\n') {
        offset += line.len();
        if line.trim() == "---" {
            break;
        }
    }
    text[offset..].trim_start_matches('\n')
}

/// `###` の見出しの下に空白以外の文字があるか（`brief` と同じ決まり）。
pub(crate) fn review_has_content(body: &str) -> bool {
    let mut in_section = false;
    for line in body.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("### ") {
            in_section = true;
            continue;
        }
        if trimmed.starts_with("## ") {
            in_section = false;
            continue;
        }
        if in_section && !trimmed.is_empty() {
            return true;
        }
    }
    false
}

/// ファイルの更新時刻（取れなければ UNIX エポック）。
fn modified_at(path: &Path) -> SystemTime {
    fs::metadata(path)
        .and_then(|meta| meta.modified())
        .unwrap_or(SystemTime::UNIX_EPOCH)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// テスト用の一時プロジェクトルート（`.sai/paper` と `.sai/outputs` 付き）。
    fn temp_root(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "sai-paper-attention-{}-{}-{:?}",
            tag,
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join(".sai/paper")).unwrap();
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

    /// ブロック定義を書く。`templates` は frontmatter にそのまま入る。
    fn write_block(root: &Path, id: &str, templates: &str) {
        write(
            root.join(".sai/blocks").join(format!("{id}.md")),
            &format!("---\nname: {id}\ntemplates: {templates}\n---\n\n本文\n"),
        );
    }

    /// 実行フォルダを作る（`task.md` 付き）。
    fn write_run(root: &Path, owner: &str, run_id: i64, block: &str) {
        write(
            root.join(".sai/outputs")
                .join(owner)
                .join(run_id.to_string())
                .join("task.md"),
            &format!("---\nrun_id: {run_id}\nblock: {block}\n---\n\nタスク\n"),
        );
    }

    /// todo の CARD を 1 枚作る。
    fn write_card(root: &Path, run_id: i64, tag: &str, description: &str) {
        write(
            root.join(".sai/outputs/todo")
                .join(run_id.to_string())
                .join("task.md"),
            &format!(
                "---\nrun_id: {run_id}\nblock: paper-screen\ntag: {tag}\ndescription: {description}\norigin_run_id: -\n---\n\nタスク\n"
            ),
        );
    }

    /// 全ての表を持つテスト用のメモリ DB。
    fn db() -> Database {
        Database::in_memory().unwrap()
    }

    // ---- 1. 書いていない review ----

    #[test]
    fn empty_review_is_collected_but_filled_and_unfinished_are_not() {
        let root = temp_root("review");
        write_block(&root, "paper-search", "project-brief");
        write_run(&root, "review", 41, "paper-search");
        write(
            root.join(".sai/outputs/review/41/review.md"),
            "---\n---\n\n### 追加修正\n\n",
        );
        // 記入済みは拾わない。
        write_run(&root, "review", 42, "paper-search");
        write(
            root.join(".sai/outputs/review/42/review.md"),
            "---\n---\n\n### 追加修正\nn\n",
        );
        // todo / running は拾わない。
        write_run(&root, "todo", 43, "paper-search");
        write(
            root.join(".sai/outputs/todo/43/review.md"),
            "---\n---\n\n### 追加修正\n\n",
        );
        write_run(&root, "running", 44, "paper-search");
        write(
            root.join(".sai/outputs/running/44/review.md"),
            "---\n---\n\n### 追加修正\n\n",
        );

        let db = db();
        let items = collect(db.connection(), &roots_of(&root)).unwrap();
        let reviews: Vec<&AttentionItem> = items
            .iter()
            .filter(|item| item.kind == AttentionKind::EmptyReview)
            .collect();

        assert_eq!(reviews.len(), 1, "{items:?}");
        assert_eq!(reviews[0].key, "review:41");
        assert!(
            reviews[0]
                .open_path
                .as_ref()
                .unwrap()
                .ends_with("41/review.md")
        );
    }

    // ---- 2. 質問 ----

    #[test]
    fn questions_are_collected_and_resolved_ones_disappear() {
        let root = temp_root("question");
        write_block(&root, "paper-search", "project-brief");
        write_run(&root, "review", 51, "paper-search");
        write(
            root.join(".sai/outputs/review/51/answer.md"),
            "## 引き継ぎ\n- ユーザーへの質問: 対象は頭皮脳波でよい?\n",
        );

        let db = db();
        let roots = roots_of(&root);
        let items = collect(db.connection(), &roots).unwrap();
        let questions: Vec<&AttentionItem> = items
            .iter()
            .filter(|item| item.kind == AttentionKind::Question)
            .collect();
        assert_eq!(questions.len(), 1, "{items:?}");
        assert_eq!(questions[0].key, "question:51:対象は頭皮脳波でよい?");
        assert!(
            questions[0]
                .open_path
                .as_ref()
                .unwrap()
                .ends_with("51/answer.md")
        );

        // 対応済みにすると消える。
        resolve(&db, questions[0]).unwrap();
        let after = collect(db.connection(), &roots).unwrap();
        assert!(
            !after
                .iter()
                .any(|item| item.kind == AttentionKind::Question),
            "{after:?}"
        );
    }

    // ---- 3. 現況の更新案 ----

    #[test]
    fn state_update_is_collected_only_when_newer_than_state_md() {
        let root = temp_root("state");
        write_block(&root, "paper-search", "project-brief");
        write_run(&root, "review", 61, "paper-search");
        write(
            root.join(".sai/outputs/review/61/answer.md"),
            "## 引き継ぎ\n- 現況の更新案: 段階を絞り込みにする\n",
        );

        let db = db();
        // state.md が無い（未作成）→ 更新案は出る。
        let items = collect(db.connection(), &roots_of(&root)).unwrap();
        let updates: Vec<&AttentionItem> = items
            .iter()
            .filter(|item| item.kind == AttentionKind::StateUpdate)
            .collect();
        assert_eq!(updates.len(), 1, "{items:?}");
        assert_eq!(updates[0].key, "state:61");
        assert!(updates[0].open_path.as_ref().unwrap().ends_with("state.md"));

        // state.md を更新案より後に直すと消える。
        std::thread::sleep(std::time::Duration::from_millis(20));
        write(
            root.join(".sai/paper/state.md"),
            "# 現況\n- 段階: 絞り込み\n",
        );
        let items = collect(db.connection(), &roots_of(&root)).unwrap();
        assert!(
            !items
                .iter()
                .any(|item| item.kind == AttentionKind::StateUpdate),
            "{items:?}"
        );
    }

    // ---- 4. 推薦の CARD ----

    #[test]
    fn recommend_cards_are_collected_newest_first() {
        let root = temp_root("card");
        write_card(&root, 71, "推薦", "候補を 3 本に絞る");
        write_card(&root, 72, "推薦", "評価の分け方を決める");
        // 別タグの CARD は拾わない。
        write_card(&root, 73, "Next Action", "別の CARD");

        let db = db();
        let items = collect(db.connection(), &roots_of(&root)).unwrap();
        let cards: Vec<&AttentionItem> = items
            .iter()
            .filter(|item| item.kind == AttentionKind::RecommendCard)
            .collect();

        assert_eq!(cards.len(), 2, "{items:?}");
        assert_eq!(cards[0].key, "card:72");
        assert_eq!(cards[1].key, "card:71");
        assert!(cards[0].open_path.as_ref().unwrap().ends_with("72/task.md"));
    }

    // ---- 5. 出典の要確認 ----

    #[test]
    fn report_issues_collect_only_errors() {
        let db = db();
        let conn = db.connection();
        conn.execute(
            "INSERT INTO reports (id, title, md_path, run_id, block, created_at, summary)
             VALUES ('R-0007', 'answer', '/tmp/proj/.sai/outputs/review/41/answer.md', '41',
                     'paper-search', '2026-09-24T21:10:00+00:00', '')",
            [],
        )
        .unwrap();
        conn.execute_batch(
            "INSERT INTO report_issues (report_id, level, kind, detail, line) VALUES
             ('R-0007', 'error', 'unresolved', 'a', 12),
             ('R-0007', 'error', 'unresolved', 'b', NULL),
             ('R-0007', 'warning', 'style', 'c', NULL);",
        )
        .unwrap();

        let items = report_issues(conn).unwrap();

        assert_eq!(items.len(), 1, "{items:?}");
        assert_eq!(items[0].key, "issue:R-0007:unresolved:12");
        assert_eq!(items[0].title, "R-0007 に要確認 2 件");
        assert_eq!(items[0].report_id.as_deref(), Some("R-0007"));
        assert_eq!(items[0].run_id, Some(41));
    }

    // ---- 対応済み ----

    #[test]
    fn resolved_keys_hide_matching_items() {
        let root = temp_root("resolved");
        write_card(&root, 81, "推薦", "消す CARD");

        let db = db();
        let roots = roots_of(&root);
        let before = collect(db.connection(), &roots).unwrap();
        assert_eq!(before.len(), 1);
        resolve(&db, &before[0]).unwrap();

        let after = collect(db.connection(), &roots).unwrap();
        assert!(after.is_empty(), "{after:?}");
    }

    #[test]
    fn kind_names_and_labels_are_stable() {
        assert_eq!(AttentionKind::EmptyReview.as_str(), "empty_review");
        assert_eq!(AttentionKind::ReportIssue.as_str(), "report_issue");
        assert_eq!(AttentionKind::StateUpdate.label(), "現況");
        assert_eq!(AttentionKind::SciDecision.as_str(), "sci_decision");
        assert_eq!(AttentionKind::SciDecision.label(), "AWAITING_DECISION");
    }

    // ---- 7 と 8: research-tree ----

    #[test]
    fn sci_items_appear_and_state_update_is_skipped() {
        use crate::rt::store::{self, NewNode};
        use crate::rt::Kind;

        let root = temp_root("sci");
        let roots = roots_of(&root);
        let db = db();
        let conn = db.connection();

        // 引き継ぎの「現況の更新案」（research-tree がある間は出さない）。
        write(
            root.join(".sai/outputs/review/50/task.md"),
            "---\nrun_id: 50\nblock: sci-compare\n---\n\nタスク\n",
        );
        write(
            root.join(".sai/outputs/review/50/handoff.md"),
            "---\nrun_id: 50\n---\n\n## ユーザーへの質問\n- どれにする\n\n## 現況の更新案\n- 目標: 新しくする\n",
        );

        // research-tree を作る（research-tree があると、★ や PENDING の要対応は出ない）。
        store::meta_set(conn, "initialized_at", "2026-10-02 00:00:00").unwrap();
        let add = |kind, parent, text: &str, stance: Option<&str>| {
            store::insert_node(
                conn,
                &NewNode {
                    kind,
                    parent_id: parent,
                    guide_key: None,
                    text,
                    body: "",
                    stance,
                    cites: &[],
                    paper_id: None,
                    run_id: None,
                    report_id: None,
                    mlflow_ref: None,
                    dataset_ref: None,
                    paths: &[],
                    created_by: "user",
                },
            )
            .unwrap()
        };
        let q = add(Kind::A, None, "対象は？", None);
        let a = add(Kind::A, Some(q), "新生児", None);
        add(Kind::F, Some(a), "根拠 1", Some("support"));
        add(Kind::F, Some(a), "根拠 2", Some("support"));

        let items = collect(conn, &roots).unwrap();
        assert!(
            !items.iter().any(|item| matches!(
                item.kind,
                AttentionKind::SciDecision
            )),
            "{items:?}"
        );
        // research-tree がある間は「現況の更新案」を出さない。
        assert!(
            !items
                .iter()
                .any(|item| item.kind == AttentionKind::StateUpdate),
            "{items:?}"
        );

    }

    #[test]
    fn no_tree_keeps_state_update_and_adds_no_sci_items() {
        let root = temp_root("sci-none");
        let roots = roots_of(&root);
        write_block(&root, "paper-search", "project-brief");
        write_run(&root, "review", 60, "paper-search");
        write(
            root.join(".sai/outputs/review/60/answer.md"),
            "## 引き継ぎ\n- 現況の更新案: 段階を絞り込みにする\n",
        );

        let db = db();
        let items = collect(db.connection(), &roots).unwrap();
        // research-tree が無いときは今までと同じ（「現況の更新案」は出て、research-tree の項目は出ない）。
        assert!(
            items
                .iter()
                .any(|item| item.kind == AttentionKind::StateUpdate),
            "{items:?}"
        );
        assert!(
            !items.iter().any(|item| item.kind == AttentionKind::SciDecision),
            "{items:?}"
        );
    }
}
