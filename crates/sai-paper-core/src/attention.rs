//! 「要対応」の一覧（S02）。
//!
//! ユーザーが「今やること」だけを 1 か所で見られるように、次の 8 種類を集める。
//!
//! 1. `EmptyReview` — 書いていない review（研究用ブロックの実行が終わったのに
//!    `review.md` が空）
//! 2. `Question` — 引き継ぎの「ユーザーへの質問」（最近 5 件の引き継ぎから）
//! 3. `StateUpdate` — 引き継ぎに「現況の更新案」があるのに `state.md` がその後に
//!    直されていない
//! 4. `RecommendCard` — todo に残っている推薦のカード（タグ `推薦`）
//! 5. `ReportIssue` — 出典の要確認（report の問題のうち `error` のもの）
//! 6. `NewSetItems` — 検索セットをもう一度実行して新着があった（S03）
//! 7. `SciDecision` — 判断待ちの項目がある（研究方針の木があるときだけ）
//! 8. `SciTriage` — 仕分けていない提案がある（研究方針の木があるときだけ）
//!
//! LLM は使わない。ファイル（`.sai/outputs/`・`.sai/paper/state.md`）と
//! `papr.db` を読むだけである。
//!
//! 「対応済み」にした記録は `activity_log` の `resolved` に残す。`detail` の JSON は
//! `{"key": "<[`AttentionItem::key`]>", "text": "<その時の本文の先頭 200 字>"}`。
//! 質問と現況の更新案は、答えたかどうかを記録から判断できないので `x` で消す。
//! review・カード・出典は元が片付けば自然に消える（`resolved` でも消せる）。

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::brief;
use crate::database::Database;
use crate::handoff;
use crate::paths::{PaperRoots, state_path};
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
    /// todo に残っている推薦のカード。
    RecommendCard,
    /// report の出典の要確認（`error`）。
    ReportIssue,
    /// 検索セットをもう一度実行して新着があった（S03）。
    NewSetItems,
    /// 研究方針の木の判断待ちの項目（C-08 の `blank = awaiting_decision`）。
    SciDecision,
    /// 研究方針の木の仕分け待ちの提案。
    SciTriage,
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
            Self::NewSetItems => "新着",
            Self::SciDecision => "AWAITING_DECISION",
            Self::SciTriage => "TRIAGE",
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
            Self::NewSetItems => "new_set_items",
            Self::SciDecision => "sci_decision",
            Self::SciTriage => "sci_triage",
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
    Database(#[from] crate::database::DatabaseError),
    /// SQLite の操作に失敗した。
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    /// 引き継ぎの読み出しに失敗した。
    #[error("progress error: {0}")]
    Progress(#[from] progress::ProgressError),
    /// todo カードの読み出しに失敗した。
    #[error("handoff error: {0}")]
    Handoff(#[from] handoff::HandoffError),
    /// report の問題の読み出しに失敗した。
    #[error("report error: {0}")]
    Report(#[from] reports::ReportError),
    /// `resolved` の `detail` の JSON の読み書きに失敗した。
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    /// 研究方針の木の読み出しに失敗した。
    #[error("sci error: {0}")]
    Sci(#[from] crate::sci::SciError),
}

/// 書いていない review の上限（新しい順）。
const EMPTY_REVIEW_LIMIT: usize = 10;

/// 引き継ぎを走査する件数（[`progress::collect`] と同じ）。
const HANDOFF_SCAN: usize = 20;

/// 要対応を集める。並びは 1 → 2 → 3 → 4 → 5 → 6 → 7 → 8（種類の中は新しい順）。
///
/// 研究方針の木があるときは 3（現況の更新案）を集めない。`state.md` は木から書き出すので、
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
    // 研究方針の木があるときだけ 7 と 8 を出し、3 は出さない。
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
    for item in new_set_items(conn)? {
        if !resolved.contains(&item.key) {
            items.push(item);
        }
    }
    if let Some(status) = tree.as_ref() {
        for item in sci_decisions(conn, roots, status)? {
            if !resolved.contains(&item.key) {
                items.push(item);
            }
        }
        for item in sci_triage(conn, status)? {
            if !resolved.contains(&item.key) {
                items.push(item);
            }
        }
    }
    Ok(items)
}

/// 研究方針の木があれば `status` を返す。始めていなければ `None`。
fn sci_tree(
    conn: &Connection,
    roots: &PaperRoots,
) -> Result<Option<crate::sci::compute::Status>, AttentionError> {
    if !crate::sci::store::is_initialized(conn)? {
        return Ok(None);
    }
    let defs = crate::sci::defs::load(&roots.root);
    Ok(Some(crate::sci::compute::status(conn, roots, &defs)?))
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

/// 4. todo に残っている推薦のカード（`run_id` の新しい順）。
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
                title: format!("推薦のカード: {description}（run {}）", card.run_id),
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

/// 6. 検索セットの新着（S03）。
///
/// 最後の `rerun` の記録が `new > 0` で、その後に新着の候補が 1 つも
/// `pending` から動いていないセットを出す。新着の候補は `added_at` が
/// その回の `searched_at` と一致する行（[`crate::search_sets::apply_rerun`] が
/// そうそろえる）。key は `set:<set_id>:<searched_at>`。
fn new_set_items(conn: &Connection) -> Result<Vec<AttentionItem>, AttentionError> {
    // `rerun` の記録を新しい順に読み、set_id ごとに最後の 1 件だけ残す。
    let mut stmt = conn.prepare(
        "SELECT detail, occurred_at FROM activity_log
         WHERE kind = 'discover' AND detail LIKE '%\"rerun\"%'
         ORDER BY occurred_at DESC, rowid DESC",
    )?;
    let rows = stmt
        .query_map([], |row| {
            Ok((row.get::<_, Option<String>>(0)?, row.get::<_, String>(1)?))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;

    // set_id → new の数（新しい記録が先に来るので最初の値が最後の rerun）。
    let mut latest: HashMap<i64, usize> = HashMap::new();
    for (detail, _) in rows {
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
        let new = value
            .get("new")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0) as usize;
        latest.entry(set_id).or_insert(new);
    }

    let mut found: Vec<(String, AttentionItem)> = Vec::new();
    for (set_id, new) in latest {
        if new == 0 {
            continue;
        }
        let set = conn
            .query_row(
                "SELECT name, searched_at FROM search_sets WHERE id = ?1",
                [set_id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?)),
            )
            .optional()?;
        let Some((name, Some(searched_at))) = set else {
            continue;
        };
        // 新着候補が 1 つでも pending から動いていたら出さない。
        let moved: i64 = conn.query_row(
            "SELECT COUNT(*) FROM search_set_items
             WHERE set_id = ?1 AND added_at = ?2 AND decision <> 'pending'",
            rusqlite::params![set_id, &searched_at],
            |row| row.get(0),
        )?;
        if moved > 0 {
            continue;
        }
        found.push((
            searched_at.clone(),
            AttentionItem {
                key: format!("set:{set_id}:{searched_at}"),
                kind: AttentionKind::NewSetItems,
                title: format!("検索セット \"{name}\" に新着 {new} 本"),
                run_id: None,
                open_path: None,
                report_id: None,
            },
        ));
    }
    // 新しい検索ほど先に。
    found.sort_by(|a, b| b.0.cmp(&a.0));
    Ok(found.into_iter().map(|(_, item)| item).collect())
}

/// 7. 判断待ちの項目（`status.nodes` の `blank = awaiting_decision` のもの 1 件ずつ）。
///
/// キーは `sci_decision:<節点>:<その項目と候補の知見の最大の id>`。知見が 1 つ入るたびに
/// キーが変わるので、「対応済み」にした後で新しい知見が入ればもう一度出る。
fn sci_decisions(
    conn: &Connection,
    roots: &PaperRoots,
    status: &crate::sci::compute::Status,
) -> Result<Vec<AttentionItem>, AttentionError> {
    let tree_path = roots.data_dir.join("tree.md");
    let mut items = Vec::new();
    for node in status
        .nodes
        .iter()
        .filter(|node| node.blank.as_deref() == Some("awaiting_decision"))
    {
        let Some(id) = crate::sci::parse_ref(&node.id, 'N').ok() else {
            continue;
        };
        // その項目と子の候補の知見の最大の id（無ければ 0）。
        let findings = node_findings(conn, id)?;
        let latest = findings.iter().copied().max().unwrap_or(0);
        items.push(AttentionItem {
            key: format!("sci_decision:{}:{latest}", node.id),
            kind: AttentionKind::SciDecision,
            title: format!(
                "AWAITING_DECISION: {} {}（OPTION {}）",
                node.id, node.title, node.counts.options
            ),
            run_id: None,
            open_path: Some(tree_path.clone()),
            report_id: None,
        });
    }
    Ok(items)
}

/// 8. 仕分け待ちの提案（1 件にまとめる）。
fn sci_triage(
    conn: &Connection,
    status: &crate::sci::compute::Status,
) -> Result<Vec<AttentionItem>, AttentionError> {
    if status.debts.pending_proposals == 0 {
        return Ok(Vec::new());
    }
    let latest = crate::sci::store::list_proposals(conn, Some("pending"))?
        .iter()
        .map(|proposal| proposal.id)
        .max()
        .unwrap_or(0);
    Ok(vec![AttentionItem {
        key: format!("sci_triage:{latest}"),
        kind: AttentionKind::SciTriage,
        title: format!(
            "TRIAGE 待ちの PROPOSAL が {} 件あります",
            status.debts.pending_proposals
        ),
        run_id: None,
        open_path: None,
        report_id: None,
    }])
}

/// その節点と子の候補の知見の番号（外していないもの）。
fn node_findings(conn: &Connection, node_id: i64) -> Result<Vec<i64>, AttentionError> {
    let mut ids: Vec<i64> = crate::sci::store::list_findings(conn, Some(node_id), false)?
        .iter()
        .map(|finding| finding.id)
        .collect();
    for child in crate::sci::store::children_of(conn, node_id)? {
        ids.extend(
            crate::sci::store::list_findings(conn, Some(child), false)?
                .iter()
                .map(|finding| finding.id),
        );
    }
    Ok(ids)
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
fn strip_frontmatter(text: &str) -> &str {
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
fn review_has_content(body: &str) -> bool {
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

    /// todo のカードを 1 枚作る。
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

    // ---- 4. 推薦のカード ----

    #[test]
    fn recommend_cards_are_collected_newest_first() {
        let root = temp_root("card");
        write_card(&root, 71, "推薦", "候補を 3 本に絞る");
        write_card(&root, 72, "推薦", "評価の分け方を決める");
        // 別タグのカードは拾わない。
        write_card(&root, 73, "次の一手", "別のカード");

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

    // ---- 6. 検索セットの新着（S03） ----

    #[test]
    fn new_set_items_are_collected_and_disappear_after_a_decision() {
        let root = temp_root("new-set-items");
        let db = db();
        let conn = db.connection();
        conn.execute(
            "INSERT INTO search_sets (id, name, source_dbs, query, searched_at)
             VALUES (5, 'eeg-recent', 's2', 'eeg', '2026-09-25T09:00:00+09:00')",
            [],
        )
        .unwrap();
        // 新着候補の `added_at` は `searched_at` とそろっている（apply_rerun と同じ）。
        conn.execute(
            "INSERT INTO search_set_items (set_id, position, remote_json, dedupe_key, decision, added_at)
             VALUES (5, 0, '{}', 'doi:x', 'pending', '2026-09-25T09:00:00+09:00')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO activity_log (kind, detail, occurred_at)
             VALUES ('discover', '{\"set_id\":5,\"rerun\":true,\"new\":1}', '2026-09-25 00:00:00')",
            [],
        )
        .unwrap();

        let items = collect(conn, &roots_of(&root)).unwrap();
        let found: Vec<&AttentionItem> = items
            .iter()
            .filter(|item| item.kind == AttentionKind::NewSetItems)
            .collect();
        assert_eq!(found.len(), 1, "{items:?}");
        assert_eq!(found[0].key, "set:5:2026-09-25T09:00:00+09:00");
        assert_eq!(found[0].title, "検索セット \"eeg-recent\" に新着 1 本");

        // 新着候補を keep にすると消える。
        conn.execute(
            "UPDATE search_set_items SET decision = 'keep' WHERE set_id = 5",
            [],
        )
        .unwrap();
        let after = collect(conn, &roots_of(&root)).unwrap();
        assert!(
            !after
                .iter()
                .any(|item| item.kind == AttentionKind::NewSetItems),
            "{after:?}"
        );
    }

    #[test]
    fn new_set_items_skip_zero_and_use_the_last_rerun() {
        let root = temp_root("new-set-zero");
        let db = db();
        let conn = db.connection();
        conn.execute(
            "INSERT INTO search_sets (id, name, source_dbs, query, searched_at)
             VALUES (7, 'recent', 's2', 'q', '2026-09-25T09:00:00+09:00')",
            [],
        )
        .unwrap();
        // 古い rerun は new > 0、新しい rerun は new = 0。
        conn.execute_batch(
            "INSERT INTO activity_log (kind, detail, occurred_at) VALUES
             ('discover', '{\"set_id\":7,\"rerun\":true,\"new\":4}', '2026-09-25 01:00:00'),
             ('discover', '{\"set_id\":7,\"rerun\":true,\"new\":0}', '2026-09-25 02:00:00');",
        )
        .unwrap();

        let items = collect(conn, &roots_of(&root)).unwrap();
        assert!(
            !items
                .iter()
                .any(|item| item.kind == AttentionKind::NewSetItems),
            "最後の rerun が 0 なら出さない: {items:?}"
        );
    }

    // ---- 対応済み ----

    #[test]
    fn resolved_keys_hide_matching_items() {
        let root = temp_root("resolved");
        write_card(&root, 81, "推薦", "消すカード");

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
        assert_eq!(AttentionKind::NewSetItems.as_str(), "new_set_items");
        assert_eq!(AttentionKind::NewSetItems.label(), "新着");
        assert_eq!(AttentionKind::StateUpdate.label(), "現況");
        assert_eq!(AttentionKind::SciDecision.as_str(), "sci_decision");
        assert_eq!(AttentionKind::SciTriage.as_str(), "sci_triage");
        assert_eq!(AttentionKind::SciDecision.label(), "AWAITING_DECISION");
        assert_eq!(AttentionKind::SciTriage.label(), "TRIAGE");
    }

    // ---- 7 と 8: 研究方針の木 ----

    #[test]
    fn sci_items_appear_and_state_update_is_skipped() {
        use crate::sci::store::{self, NewFinding, NewNode, NewProposal};

        let root = temp_root("sci");
        let roots = roots_of(&root);
        let db = db();
        let conn = db.connection();

        // 引き継ぎの「現況の更新案」（木がある間は出さない）。
        write(
            root.join(".sai/outputs/review/50/task.md"),
            "---\nrun_id: 50\nblock: sci-compare\n---\n\nタスク\n",
        );
        write(
            root.join(".sai/outputs/review/50/handoff.md"),
            "---\nrun_id: 50\n---\n\n## ユーザーへの質問\n- どれにする\n\n## 現況の更新案\n- 目標: 新しくする\n",
        );

        // 木を作る（選ぶ項目に候補を 2 つ、知見を 2 つ入れて判断待ちにする）。
        store::meta_set(conn, "type", "problem").unwrap();
        let item = store::insert_node(
            conn,
            &NewNode {
                kind: "item",
                item_key: Some("target"),
                types: &["choose".to_string()],
                title: "対象",
                body: "",
                state: "open",
                fixed: false,
                ord: 0,
                created_by: "user",
            },
        )
        .unwrap();
        for title in ["てんかん患者", "新生児"] {
            let option = store::insert_node(
                conn,
                &NewNode {
                    kind: "option",
                    item_key: None,
                    types: &[],
                    title,
                    body: "",
                    state: "candidate",
                    fixed: false,
                    ord: 0,
                    created_by: "user",
                },
            )
            .unwrap();
            store::insert_edge(conn, "parent", item, option).unwrap();
            store::insert_finding(
                conn,
                &NewFinding {
                    node_id: option,
                    question_id: None,
                    text: "根拠",
                    cites: &[],
                    polarity: "support",
                    level: None,
                    source: "user",
                    report_id: None,
                    paper_id: None,
                    run_id: None,
                    proposal_id: None,
                    auto: false,
                    created_by: "user",
                },
            )
            .unwrap();
        }
        store::insert_proposal(
            conn,
            &NewProposal {
                run_id: Some(50),
                block: "sci-compare",
                op: "add_finding",
                // 判断待ちの項目そのものではない（`in_progress` になると判断待ちにならない）。
                node_id: None,
                payload: &serde_json::json!({}),
                reason: "比較したい",
                risk: "auto",
                status: "pending",
            },
        )
        .unwrap();

        let items = collect(conn, &roots).unwrap();
        let decision = items
            .iter()
            .find(|item| item.kind == AttentionKind::SciDecision)
            .expect("AWAITING_DECISION が出る");
        assert_eq!(decision.title, "AWAITING_DECISION: N1 対象（OPTION 2）");
        assert_eq!(decision.key, "sci_decision:N1:2");
        assert_eq!(decision.open_path, Some(roots.data_dir.join("tree.md")));
        assert!(decision.run_id.is_none());
        let triage = items
            .iter()
            .find(|item| item.kind == AttentionKind::SciTriage)
            .expect("TRIAGE が出る");
        assert_eq!(triage.title, "TRIAGE 待ちの PROPOSAL が 1 件あります");
        assert_eq!(triage.key, "sci_triage:1");
        assert!(triage.open_path.is_none());
        // 木がある間は「現況の更新案」を出さない。
        assert!(
            !items
                .iter()
                .any(|item| item.kind == AttentionKind::StateUpdate),
            "{items:?}"
        );

        // 対応済みキーにすると消える。
        resolve(&db, decision).unwrap();
        let after = collect(conn, &roots).unwrap();
        assert!(
            !after
                .iter()
                .any(|item| item.kind == AttentionKind::SciDecision),
            "{after:?}"
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
        // 木が無いときは今までと同じ（3 は出て、7 と 8 は出ない）。
        assert!(
            items
                .iter()
                .any(|item| item.kind == AttentionKind::StateUpdate),
            "{items:?}"
        );
        assert!(
            !items.iter().any(|item| matches!(
                item.kind,
                AttentionKind::SciDecision | AttentionKind::SciTriage
            )),
            "{items:?}"
        );
    }
}
