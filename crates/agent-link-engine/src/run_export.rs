//! 実行 1 件を `$AGENT_LINK_HOME/web/runs/<project>/<run_id>.json` に書き出す。
//!
//! daemon（起動時と run の状態が変わったとき）と `agent-link run export` が使う。
//! 読み手は Web（`apps/web/src/agent_link_web/runs.py`）。
//!
//! - 書き込みは一時ファイル（`.<run_id>.json.tmp`）→ rename（読み手が書きかけを読まないため）。
//! - 同じ内容（`updated_at` を除く）が既にあれば書き直さない（読み手の無駄な再読込を防ぐ）。
//! - 書き出しの失敗はログに出すだけで、エージェント実行は止めない。

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::db::models::ExecutionHistory;
use crate::db::SaiStore;
use crate::cc_cost::read_cc_cost;
use crate::common::now_iso;
use crate::headless::pane_id_from_session_dir;
use crate::registry::project_name_for;
use crate::transcript::resolve_transcript;
use crate::usage_metrics::{collect_metrics_from_file_cc, CacheUsageTotals};

/// JSON のスキーマ版（00 §3.2。形を変えるときだけ上げる）。
pub const SCHEMA_VERSION: i64 = 1;

// ============================================================================
// JSON ドキュメント（00 §3.2 / §3.4）
// ============================================================================

/// 実行 1 件分の書き出し JSON（00 §3.2）。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RunDoc {
    /// 常に 1
    pub schema_version: i64,
    /// `~/.agent-link/registry.json` の登録名（無ければフォルダ名）
    pub project_name: String,
    /// プロジェクトルートの絶対パス
    pub project_root: String,
    /// `execution_history.id`
    pub run_id: i64,
    /// サブタスクのときの親 run_id
    pub parent_run_id: Option<i64>,
    /// `block_metadata.name`
    pub block_name: String,
    /// task.md frontmatter `description:`（無ければ空）
    pub description: String,
    /// 常に `cc`（Web が記録の読み分けに使う項目。coding agent は Claude Code だけ）
    pub runner: String,
    /// `running` / `completed` / `failed` / `cancelled`
    pub status: String,
    /// 実行時モデル（無いときは stats のモデル名、それも無ければ null）
    pub model: Option<String>,
    /// UTC ISO 8601（末尾 Z）
    pub started_at: Option<String>,
    /// UTC ISO 8601（末尾 Z）
    pub finished_at: Option<String>,
    /// 実行時間（ミリ秒）
    pub duration_ms: Option<i64>,
    /// `sai-<block_name>-<run_id>`（agent-link が付けるペイン名）
    pub herdr_pane_label: String,
    /// 分かれば入れる（実行中は null のことが多い）
    pub herdr_pane_id: Option<String>,
    /// cc の `--session-id` UUID（旧実行は null）
    pub agent_session_id: Option<String>,
    /// 会話記録 jsonl の絶対パス（`transcript` の解決規則で求める）
    pub transcript_path: Option<String>,
    /// task.md の現在の絶対パス
    pub task_md_path: Option<String>,
    /// このファイルを書いた時刻（UTC ISO 8601 Z）
    pub updated_at: String,
    /// 終了後に 00 §3.4 の stats（実行中は null）
    pub stats: Option<RunDocStats>,
}

/// `stats`（00 §3.4）。
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct RunDocStats {
    /// コスト内訳（USD）
    pub cost_usd: RunDocCost,
    /// トークン
    pub tokens: RunDocTokens,
    /// `session_stats_json` の elapsed_ms（無いときは 0）
    pub elapsed_ms: u64,
    /// ターン数
    pub turn_count: u64,
    /// ツール呼び出し回数
    pub tool_call_count: u64,
    /// ツール別呼び出し回数
    pub tool_calls_by_name: HashMap<String, u64>,
    /// 集計に出てきたモデル名（複数なら最頻）
    pub model: Option<String>,
    /// `cc` / `estimate`
    pub cost_source: String,
    /// `cost_source == "estimate"` のとき true
    pub cost_is_estimate: bool,
}

/// `stats.cost_usd`（00 §3.4）。
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct RunDocCost {
    pub input: f64,
    pub output: f64,
    pub cache_read: f64,
    pub cache_write: f64,
    pub total: f64,
}

/// `stats.tokens`（00 §3.4）。
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct RunDocTokens {
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    pub total: u64,
}

// ============================================================================
// 書き出し対象の判定
// ============================================================================

/// 書き出すべき実行か（00 §3.2）。
///
/// - `todo` / `pending`（未実行）は書かない
pub fn should_export(status: &str) -> bool {
    !matches!(status, "todo" | "pending")
}

/// 終了状態か（stats を入れる対象）。
pub fn is_finished(status: &str) -> bool {
    matches!(status, "completed" | "failed" | "cancelled")
}

// ============================================================================
// 時刻
// ============================================================================

/// DB の時刻文字列（SQLite `datetime('now')` = `YYYY-MM-DD HH:MM:SS` UTC）を
/// UTC ISO 8601（末尾 `Z`）へ変換する。
///
/// `YYYY-MM-DDTHH:MM:SS` 形式と、既に `Z` で終わる文字列も受け付ける。
/// 変換できない場合は `None`。
pub fn db_time_to_iso(s: &str) -> Option<String> {
    let t = s.trim();
    if t.is_empty() {
        return None;
    }
    if t.ends_with('Z') {
        return Some(t.to_string());
    }
    let b = t.as_bytes();
    if t.len() >= 19
        && b[4] == b'-'
        && b[7] == b'-'
        && (b[10] == b' ' || b[10] == b'T')
        && b[13] == b':'
        && b[16] == b':'
    {
        let mut out = String::with_capacity(20);
        out.push_str(&t[0..10]);
        out.push('T');
        out.push_str(&t[11..19]);
        out.push('Z');
        return Some(out);
    }
    None
}

// ============================================================================
// ドキュメント構築
// ============================================================================

/// 現在の run フォルダの task.md の絶対パス（無ければ None）。
fn task_md_path_for(run_id: i64) -> Option<PathBuf> {
    let (_, dir) = crate::run_dir::find_run_dir(run_id)?;
    let p = dir.join(crate::run_dir::TASK_FILE);
    if p.is_file() {
        Some(p)
    } else {
        None
    }
}

/// 子タスクの実行なら、親が task_create で付けた件名（無ければ `None`）。
fn subtask_subject(store: &SaiStore, parent_run_id: Option<i64>, run_id: i64) -> Option<String> {
    store
        .list_tasks_by_parent(parent_run_id?)
        .ok()?
        .into_iter()
        .find(|t| t.execution_id == Some(run_id))
        .map(|t| t.subject)
        .filter(|s| !s.trim().is_empty())
}

/// task.md frontmatter の `description`（無ければ空）。
fn description_for(task_md: Option<&Path>) -> String {
    let Some(path) = task_md else { return String::new() };
    let Ok(text) = std::fs::read_to_string(path) else {
        return String::new();
    };
    crate::kanban::parse_card(&text)
        .map(|c| c.frontmatter.description)
        .unwrap_or_default()
}

/// 集計結果から最頻出のモデル名を取り出す（空文字は除外）。
fn most_frequent_model(models: impl Iterator<Item = String>) -> Option<String> {
    let mut counts: HashMap<String, usize> = HashMap::new();
    for m in models {
        let m = m.trim().to_string();
        if m.is_empty() {
            continue;
        }
        *counts.entry(m).or_insert(0) += 1;
    }
    counts
        .into_iter()
        .max_by(|a, b| a.1.cmp(&b.1).then_with(|| b.0.cmp(&a.0)))
        .map(|(k, _)| k)
}

/// `execution_usage` の `total` 行からトークン・コストを拾う。
fn tokens_from_usage_row(store: &SaiStore, run_id: i64) -> Option<(RunDocTokens, RunDocCost)> {
    let rows = store.list_execution_usage(run_id).ok()?;
    let total = rows.into_iter().find(|r| r.entry_type == "total")?;
    let tokens = RunDocTokens {
        input: total.input_tokens.max(0) as u64,
        output: total.output_tokens.max(0) as u64,
        cache_read: total.cache_read.max(0) as u64,
        cache_write: total.cache_write.max(0) as u64,
        total: (total.input_tokens + total.output_tokens + total.cache_read + total.cache_write)
            .max(0) as u64,
    };
    let cost = RunDocCost {
        input: 0.0,
        output: 0.0,
        cache_read: 0.0,
        cache_write: 0.0,
        total: total.cost_total,
    };
    Some((tokens, cost))
}

/// 終了済み実行の `stats` を作る（00 §3.4）。
fn build_stats(
    store: &SaiStore,
    exec: &ExecutionHistory,
    transcript: Option<&Path>,
) -> RunDocStats {
    // ── トークン・コスト内訳: transcript の集計 ──
    let mut totals: Option<CacheUsageTotals> = None;
    let mut top_model: Option<String> = None;
    if let Some(tp) = transcript {
        if let Ok(metrics) = collect_metrics_from_file_cc(tp) {
            top_model = most_frequent_model(metrics.all_messages.iter().map(|m| m.model.clone()));
            totals = Some(metrics.tree_totals);
        }
    }

    // transcript が無い・解析失敗 → execution_usage の total 行、どちらも無ければ 0
    let (tokens, mut cost) = match &totals {
        Some(t) => (
            RunDocTokens {
                input: t.input,
                output: t.output,
                cache_read: t.cache_read,
                cache_write: t.cache_write,
                total: t.total_tokens,
            },
            RunDocCost {
                input: t.cost.input,
                output: t.cost.output,
                cache_read: t.cost.cache_read,
                cache_write: t.cost.cache_write,
                total: t.cost.total,
            },
        ),
        None => {
            let run_id = exec.id.unwrap_or(0);
            tokens_from_usage_row(store, run_id).unwrap_or_default()
        }
    };

    // ── コストの出どころ（00 §3.4） ──
    let (cost_source, cost_is_estimate) =
        match exec.agent_session_id.as_deref().and_then(read_cc_cost) {
            // cc 自身が数えたセッション金額（/usage の額）を最優先
            Some(cc_total) => {
                cost.total = cc_total;
                ("cc", false)
            }
            // 連携前の過去実行など → 概算
            None => ("estimate", true),
        };

    // ── elapsed / turn / tool は session_stats_json（SessionStats）から ──
    let stats_opt = exec
        .session_stats_json
        .as_deref()
        .and_then(|j| serde_json::from_str::<crate::session::SessionStats>(j).ok());
    let (elapsed_ms, turn_count, tool_call_count, tool_calls_by_name, db_model) = match stats_opt {
        Some(s) => (
            s.elapsed_ms.unwrap_or(0),
            s.turn_count,
            s.tool_call_count,
            s.tool_calls_by_name,
            s.model_name,
        ),
        None => (0, 0, 0, HashMap::new(), None),
    };

    let model = top_model.or(db_model);

    RunDocStats {
        cost_usd: cost,
        tokens,
        elapsed_ms,
        turn_count,
        tool_call_count,
        tool_calls_by_name,
        model,
        cost_source: cost_source.to_string(),
        cost_is_estimate,
    }
}

/// DB の 1 行から 00 §3.2 の全項目を埋めた JSON ドキュメントを作る。
pub fn build_run_doc(
    store: &SaiStore,
    exec: &ExecutionHistory,
    project_root: &Path,
    project_name: &str,
) -> RunDoc {
    let run_id = exec.id.unwrap_or(0);
    let block_name = store
        .get_block_by_id(exec.block_id)
        .map(|b| b.name)
        .unwrap_or_default();
    let transcript = resolve_transcript(exec, project_root);
    let task_md = task_md_path_for(run_id);
    let description = description_for(task_md.as_deref());
    // 子タスク（task.md の無い sub の実行）は task_create の件名を説明にする
    // （一覧で何の実行か分かるように）
    let description = if description.trim().is_empty() {
        subtask_subject(store, exec.parent_run_id, run_id).unwrap_or_default()
    } else {
        description
    };

    // stats は終了後だけ入れる（実行中は null）
    let stats = if is_finished(&exec.status) {
        Some(build_stats(store, exec, transcript.as_deref()))
    } else {
        None
    };

    // run レベルの model: execution_history.model → stats のモデル名 → null
    let stats_model = stats.as_ref().and_then(|s| s.model.clone());
    let model = exec
        .model
        .as_deref()
        .map(|m| m.trim().to_string())
        .filter(|m| !m.is_empty())
        .or(stats_model);

    let herdr_pane_label = format!("sai-{}-{}", block_name, run_id);

    RunDoc {
        schema_version: SCHEMA_VERSION,
        project_name: project_name.to_string(),
        project_root: project_root.to_string_lossy().to_string(),
        run_id,
        parent_run_id: exec.parent_run_id,
        block_name,
        description,
        runner: "cc".to_string(),
        status: exec.status.clone(),
        model,
        started_at: exec.started_at.as_deref().and_then(db_time_to_iso),
        finished_at: exec.finished_at.as_deref().and_then(db_time_to_iso),
        duration_ms: exec.duration_ms,
        herdr_pane_label,
        herdr_pane_id: exec.herdr_pane_id.clone().or_else(|| pane_id_from_session_dir(exec)),
        agent_session_id: exec.agent_session_id.clone(),
        transcript_path: transcript.map(|p| p.to_string_lossy().to_string()),
        task_md_path: task_md.map(|p| p.to_string_lossy().to_string()),
        updated_at: now_iso(),
        stats,
    }
}

// ============================================================================
// ファイル書き出し
// ============================================================================

/// `<AGENT_LINK_HOME>/web/runs` のルート（置き場を解決できなければ None）。
pub fn export_root() -> Option<PathBuf> {
    crate::project::sai_home().map(|h| h.join("web").join("runs"))
}

/// 書き出し先フォルダ `<AGENT_LINK_HOME>/web/runs/<project_name>`（置き場を解決できなければ None）。
pub fn export_dir(project_name: &str) -> Option<PathBuf> {
    export_root().map(|r| r.join(project_name))
}

/// 1 実行分の JSON を指定フォルダに書く（一時ファイル → rename）。
///
/// 戻り値: `true` = 書いた、`false` = 同じ内容が既にあり省略した。
pub fn write_run_json_in(dir: &Path, doc: &RunDoc) -> std::io::Result<bool> {
    std::fs::create_dir_all(dir)?;
    let final_path = dir.join(format!("{}.json", doc.run_id));
    let tmp_path = dir.join(format!(".{}.json.tmp", doc.run_id));

    let text = serde_json::to_string_pretty(doc)
        .map_err(|e| std::io::Error::other(format!("JSON serialization failed: {}", e)))?;

    // 同じ内容（updated_at を除く）が既にあれば書かない（読み手の再読込防止）
    if let Ok(existing) = std::fs::read_to_string(&final_path) {
        if unchanged(&existing, &text) {
            return Ok(false);
        }
    }

    std::fs::write(&tmp_path, &text)?;
    std::fs::rename(&tmp_path, &final_path)?;
    Ok(true)
}

/// 2 つの JSON 文字列が `updated_at` を除いて同じか。
fn unchanged(existing: &str, new: &str) -> bool {
    let (Ok(mut a), Ok(mut b)) = (
        serde_json::from_str::<serde_json::Value>(existing),
        serde_json::from_str::<serde_json::Value>(new),
    ) else {
        return false;
    };
    if let Some(obj) = a.as_object_mut() {
        obj.remove("updated_at");
    }
    if let Some(obj) = b.as_object_mut() {
        obj.remove("updated_at");
    }
    a == b
}

/// 1 実行分を書き出す（対象外の実行は `None`）。
pub fn write_run_json(doc: &RunDoc) -> std::io::Result<Option<bool>> {
    let Some(dir) = export_dir(&doc.project_name) else {
        return Ok(None);
    };
    write_run_json_in(&dir, doc).map(Some)
}

// ============================================================================
// 一括書き出し
// ============================================================================

/// 差分検知用のシグネチャ。
///
/// デーモンは前回のシグネチャと比較して「変わった実行だけ」書き直す。
/// `session_stats_json` も含めるのは、終了直後に統計が遅れて入ることがあるため。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportSig {
    pub status: String,
    pub finished_at: Option<String>,
    pub session_stats_json: Option<String>,
}

impl ExportSig {
    /// 実行レコードからシグネチャを作る。
    pub fn of(exec: &ExecutionHistory) -> Self {
        Self {
            status: exec.status.clone(),
            finished_at: exec.finished_at.clone(),
            session_stats_json: exec.session_stats_json.clone(),
        }
    }
}

/// プロジェクトルート 1 つ分の書き出し器（レジストリ読み込みを 1 回に集約）。
pub struct Exporter {
    project_root: PathBuf,
    project_name: String,
    dir: PathBuf,
}

impl Exporter {
    /// 書き出し器を作る（レジストリからプロジェクト名を解決・HOME 未設定はエラー）。
    pub fn new(project_root: &Path) -> std::io::Result<Self> {
        let project_name = project_name_for(project_root);
        let dir = export_dir(&project_name).ok_or_else(|| {
            std::io::Error::other("HOME is not set, so the export destination cannot be decided.")
        })?;
        Ok(Self {
            project_root: project_root.to_path_buf(),
            project_name,
            dir,
        })
    }

    /// 解決したプロジェクト名。
    pub fn project_name(&self) -> &str {
        &self.project_name
    }

    /// 書き出し先フォルダ。
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// 実行レコード 1 行から JSON ドキュメントを作る。
    pub fn build_doc(&self, store: &SaiStore, exec: &ExecutionHistory) -> RunDoc {
        build_run_doc(store, exec, &self.project_root, &self.project_name)
    }

    /// 1 実行分を書き出す。
    ///
    /// 戻り値: `Ok(Some(書いたか))` = 書き出し対象、
    /// `Ok(None)` = 対象外（`todo` / `pending`）。
    pub fn export(
        &self,
        store: &SaiStore,
        exec: &ExecutionHistory,
    ) -> std::io::Result<Option<bool>> {
        if !should_export(&exec.status) {
            return Ok(None);
        }
        let doc = self.build_doc(store, exec);
        write_run_json_in(&self.dir, &doc).map(Some)
    }
}

/// 書き出し結果の集計。
#[derive(Debug, Clone, Default)]
pub struct ExportSummary {
    /// 書いた件数
    pub written: usize,
    /// 同じ内容のため省略した件数
    pub skipped: usize,
    /// 失敗した件数（ログのみ・実行は止めない）
    pub failed: usize,
    /// 書いた run_id 一覧
    pub written_run_ids: Vec<i64>,
}

impl ExportSummary {
    /// 要約 1 行（ログ用）。
    pub fn brief(&self) -> String {
        format!(
            "{} written / {} skipped / {} failed",
            self.written, self.skipped, self.failed
        )
    }
}

/// 書き出し対象の実行をドキュメント列にする（`--dry-run` 用・ファイルは書かない）。
pub fn build_docs(
    store: &SaiStore,
    execs: &[ExecutionHistory],
    project_root: &Path,
) -> Vec<RunDoc> {
    let project_name = project_name_for(project_root);
    execs
        .iter()
        .filter(|e| should_export(&e.status))
        .map(|e| build_run_doc(store, e, project_root, &project_name))
        .collect()
}

/// 実行列を書き出す（書き出し対象のみ）。
pub fn export_execs(
    store: &SaiStore,
    execs: &[ExecutionHistory],
    project_root: &Path,
) -> ExportSummary {
    let mut summary = ExportSummary::default();
    let exporter = match Exporter::new(project_root) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("[agent-link] 書き出し: {}", e);
            summary.failed = execs.len();
            return summary;
        }
    };
    for exec in execs {
        match exporter.export(store, exec) {
            Ok(Some(true)) => {
                summary.written += 1;
                summary.written_run_ids.push(exec.id.unwrap_or(0));
            }
            Ok(Some(false)) => summary.skipped += 1,
            Ok(None) => {}
            Err(e) => {
                // 失敗はログのみ（書き出しの失敗で実行は止めない）
                summary.failed += 1;
                eprintln!(
                    "[agent-link] 書き出し失敗: run_id={}: {}",
                    exec.id.unwrap_or(0),
                    e
                );
            }
        }
    }
    summary
}

/// プロジェクトの全実行を書き出す（デーモン起動時）。
pub fn export_all(store: &SaiStore) -> ExportSummary {
    let project_root = crate::project::project_root();
    let execs = match store.list_all_executions() {
        Ok(e) => e,
        Err(e) => {
            eprintln!("[agent-link] 書き出し: 実行一覧の取得に失敗: {}", e);
            return ExportSummary::default();
        }
    };
    export_execs(store, &execs, &project_root)
}

/// 1 件だけ書き出す。
///
/// 戻り値: `Ok(Some(書いたか))` = 書き出し対象、`Ok(None)` = 対象外（todo / user 等）。
pub fn export_one(store: &SaiStore, run_id: i64) -> std::io::Result<Option<bool>> {
    let exec = store
        .get_execution_by_id(run_id)
        .map_err(|e| std::io::Error::other(format!("Run not found: {}", e)))?;
    let project_root = crate::project::project_root();
    let exporter = Exporter::new(&project_root)?;
    exporter.export(store, &exec)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::models::BlockMetadata;

    /// DB 時刻（`YYYY-MM-DD HH:MM:SS`）→ UTC ISO 8601（Z 付き）に変換される。
    #[test]
    fn db_time_converts_to_iso_z() {
        assert_eq!(
            db_time_to_iso("2026-09-23 01:02:03").as_deref(),
            Some("2026-09-23T01:02:03Z")
        );
        // T 区切りも許容
        assert_eq!(
            db_time_to_iso("2026-09-23T01:02:03").as_deref(),
            Some("2026-09-23T01:02:03Z")
        );
        // 既に Z 付き
        assert_eq!(
            db_time_to_iso("2026-09-23T01:02:03Z").as_deref(),
            Some("2026-09-23T01:02:03Z")
        );
        // 空・不正は None
        assert_eq!(db_time_to_iso(""), None);
        assert_eq!(db_time_to_iso("not a date"), None);
    }


    /// `todo` / `pending` は書き出されない（00 §3.2）。
    #[test]
    fn todo_and_pending_runs_are_not_exported() {
        assert!(!should_export("todo"));
        assert!(!should_export("pending"));
        // それ以外は書き出す
        assert!(should_export("running"));
        assert!(should_export("completed"));
        assert!(should_export("failed"));
        assert!(should_export("cancelled"));
    }

    /// 一時ファイル → rename で最終ファイルができ、`.tmp` が残らないこと。
    /// また、同じ内容の再書き出しは省略されること。
    #[test]
    fn write_run_json_uses_tmp_rename_and_skips_unchanged() {
        let dir = std::env::temp_dir().join(format!(
            "run_export_test_w_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();

        let doc = RunDoc {
            schema_version: SCHEMA_VERSION,
            project_name: "demo".to_string(),
            project_root: "/tmp/demo".to_string(),
            run_id: 42,
            parent_run_id: None,
            block_name: "b".to_string(),
            description: "説明".to_string(),
            runner: "cc".to_string(),
            status: "completed".to_string(),
            model: None,
            started_at: Some("2026-09-23T01:02:03Z".to_string()),
            finished_at: Some("2026-09-23T01:10:00Z".to_string()),
            duration_ms: Some(477000),
            herdr_pane_label: "sai-b-42".to_string(),
            herdr_pane_id: None,
            agent_session_id: None,
            transcript_path: None,
            task_md_path: None,
            updated_at: now_iso(),
            stats: None,
        };

        // 1 回目: 書かれる
        assert!(write_run_json_in(&dir, &doc).unwrap());
        let final_path = dir.join("42.json");
        assert!(final_path.is_file(), "最終ファイルができる");
        assert!(
            !dir.join(".42.json.tmp").exists(),
            ".tmp が残っていないこと"
        );
        let loaded: RunDoc =
            serde_json::from_str(&std::fs::read_to_string(&final_path).unwrap()).unwrap();
        assert_eq!(loaded.schema_version, 1);
        assert_eq!(loaded.run_id, 42);
        assert_eq!(loaded.herdr_pane_label, "sai-b-42");

        // 2 回目: 同じ内容（updated_at は無視）→ 省略される
        let mut doc2 = doc.clone();
        doc2.updated_at = now_iso();
        assert!(!write_run_json_in(&dir, &doc2).unwrap());

        // status が変われば書き直される
        let mut doc3 = doc2;
        doc3.status = "running".to_string();
        assert!(write_run_json_in(&dir, &doc3).unwrap());

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 実行 1 行分のドキュメントが DB の値から正しく構築されること。
    #[test]
    fn build_run_doc_from_db_row() {
        let _env = crate::common::AGENT_LINK_ENV_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // 既存の AGENT_LINK_PROJECT_ROOT を退避して終了時に復元する（並行テストへの影響防止）
        let prev_root = std::env::var_os("AGENT_LINK_PROJECT_ROOT");
        let tmp = std::env::temp_dir().join(format!("run_export_test_doc_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(tmp.join(".agent-link")).unwrap();
        std::env::set_var("AGENT_LINK_PROJECT_ROOT", &tmp);

        let store = SaiStore::open_in_memory().unwrap();
        let block = BlockMetadata::new(
            "b".to_string(),
            "テストブロック".to_string(),
            "m".to_string(),
            "off".to_string(),
            "true".to_string(),
            "true".to_string(),
            None,
            300,
            "none".to_string(),
            true,
            "blocks/b.md".to_string(),
            None
        );
        let block_id = store.insert_block(&block).unwrap();
        let run_id = store.create_execution(block_id, None).unwrap();
        // todo → 書き出し対象外
        let exec = store.get_execution_by_id(run_id).unwrap();
        assert!(!should_export(&exec.status));

        // running へ遷移（started_at が入る）→ 書き出し対象
        store.update_execution_status(run_id, "running").unwrap();
        let exec = store.get_execution_by_id(run_id).unwrap();
        assert!(should_export(&exec.status));

        let doc = build_run_doc(&store, &exec, &tmp, "demo-project");
        assert_eq!(doc.schema_version, 1);
        assert_eq!(doc.run_id, run_id);
        assert_eq!(doc.block_name, "b");
        assert_eq!(doc.runner, "cc");
        assert_eq!(doc.status, "running");
        assert_eq!(doc.herdr_pane_label, format!("sai-b-{}", run_id));
        assert_eq!(doc.project_name, "demo-project");
        // running 中は stats が null
        assert!(doc.stats.is_none());
        // 時刻は Z 付き ISO
        assert!(
            doc.started_at
                .as_deref()
                .unwrap()
                .ends_with('Z'),
            "started_at は ISO Z: {:?}",
            doc.started_at
        );

        // 終了後は stats が入る
        let stats_json = serde_json::to_string(&crate::session::SessionStats {
            started_at: None,
            finished_at: None,
            elapsed_ms: Some(1000),
            input_tokens: 0,
            output_tokens: 0,
            thinking_tokens: 0,
            total_tokens: 0,
            tool_call_count: 3,
            turn_count: 2,
            model_name: Some("claude-sonnet-5".to_string()),
            cost_usd: 0.0,
            tool_calls_by_name: HashMap::from([("Bash".to_string(), 3u64)]),
            token_usage_by_category: crate::session::TokenUsageByCategory::default(),
        })
        .unwrap();
        store
            .update_execution_session_stats(run_id, &stats_json)
            .unwrap();
        store.update_execution_status(run_id, "completed").unwrap();
        let exec = store.get_execution_by_id(run_id).unwrap();
        let doc = build_run_doc(&store, &exec, &tmp, "demo-project");
        assert_eq!(doc.status, "completed");
        assert!(doc.finished_at.is_some());
        let stats = doc.stats.expect("終了後は stats が入る");
        assert_eq!(stats.elapsed_ms, 1000);
        assert_eq!(stats.turn_count, 2);
        assert_eq!(stats.tool_call_count, 3);
        assert_eq!(stats.tool_calls_by_name.get("Bash"), Some(&3));
        // transcript が無い → トークン・コストは 0、cc のセッション金額も無いので cost_source=estimate
        assert_eq!(stats.tokens.total, 0);
        assert_eq!(stats.cost_source, "estimate");
        assert!(stats.cost_is_estimate);

        let _ = std::fs::remove_dir_all(&tmp);
        match prev_root {
            Some(v) => std::env::set_var("AGENT_LINK_PROJECT_ROOT", v),
            None => std::env::remove_var("AGENT_LINK_PROJECT_ROOT"),
        }
    }

    /// 書き出しは `<AGENT_LINK_HOME>/web/` に出る。
    #[test]
    fn export_paths_use_web_directory_under_sai_home() {
        let _env = crate::common::AGENT_LINK_ENV_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let prev = std::env::var_os("AGENT_LINK_HOME");
        let home = std::path::PathBuf::from("/tmp/sai-home-web-test");
        std::env::set_var("AGENT_LINK_HOME", &home);
        assert_eq!(export_root(), Some(home.join("web").join("runs")));
        assert_eq!(export_dir("demo"), Some(home.join("web/runs/demo")));
        match prev {
            Some(v) => std::env::set_var("AGENT_LINK_HOME", v),
            None => std::env::remove_var("AGENT_LINK_HOME"),
        }
    }
}
