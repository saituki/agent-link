//! sai-phone 向け実行情報の書き出し（実装計画 01 §2.2 / 00 §3）。
//!
//! sai-link デーモン（および CLI `sai-link phone export`）が、実行 1 件につき
//! JSON ファイル 1 個を `~/.sai-moon/phone/runs/<project_name>/<run_id>.json` に書く。
//! sai-phone はこのフォルダを読むだけで、sai-phone への HTTP 送信はしない。
//!
//! - JSON の形（schema_version = 1）は **00-overview.md §3.2 が正**。
//! - 書き込みは一時ファイル（`.<run_id>.json.tmp`）→ rename（読み手が書きかけを読まないため）。
//! - 同じ内容（`updated_at` を除く）が既にあれば書き直さない
//!   （sai-phone の無駄な再読込を防ぐ）。
//! - 書き出しの失敗はログに出すだけでエージェント実行は止めない（おまけ機能のため）。

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::db::models::ExecutionHistory;
use crate::db::SaiStore;
use crate::usage_metrics::{collect_metrics_from_file, collect_metrics_from_file_cc, CacheUsageTotals};

/// JSON のスキーマ版（00 §3.2。形を変えるときだけ上げる）。
pub const SCHEMA_VERSION: i64 = 1;

// ============================================================================
// JSON ドキュメント（00 §3.2 / §3.4）
// ============================================================================

/// 実行 1 件分の書き出し JSON（00 §3.2）。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PhoneRunDoc {
    /// 常に 1
    pub schema_version: i64,
    /// `~/.sai-moon/registry.json` の登録名（無ければフォルダ名）
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
    /// `cc` / `pi` / `gl`（`user` は書き出さない）
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
    /// `sai-<block_name>-<run_id>`（sai-link が付けるペイン名）
    pub herdr_pane_label: String,
    /// 分かれば入れる（実行中は null のことが多い）
    pub herdr_pane_id: Option<String>,
    /// cc の `--session-id` UUID（pi は null）
    pub agent_session_id: Option<String>,
    /// 会話記録 jsonl の絶対パス（00 §3.3 の方法で解決）
    pub transcript_path: Option<String>,
    /// task.md の現在の絶対パス
    pub task_md_path: Option<String>,
    /// このファイルを書いた時刻（UTC ISO 8601 Z）
    pub updated_at: String,
    /// 終了後に 00 §3.4 の stats（実行中は null）
    pub stats: Option<PhoneRunStats>,
}

/// `stats`（00 §3.4）。
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct PhoneRunStats {
    /// コスト内訳（USD）
    pub cost_usd: PhoneCost,
    /// トークン
    pub tokens: PhoneTokens,
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
    /// `pi` / `cc` / `estimate`
    pub cost_source: String,
    /// `cost_source == "estimate"` のとき true
    pub cost_is_estimate: bool,
}

/// `stats.cost_usd`（00 §3.4）。
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct PhoneCost {
    pub input: f64,
    pub output: f64,
    pub cache_read: f64,
    pub cache_write: f64,
    pub total: f64,
}

/// `stats.tokens`（00 §3.4）。
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct PhoneTokens {
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
/// - runner = `user`（ユーザータスク）は書かない
pub fn should_export(status: &str, runner: &str) -> bool {
    if matches!(status, "todo" | "pending") {
        return false;
    }
    !runner.trim().eq_ignore_ascii_case("user")
}

/// 終了状態か（stats を入れる対象）。
pub fn is_finished(status: &str) -> bool {
    matches!(status, "completed" | "failed" | "cancelled")
}

// ============================================================================
// パス・時刻・プロジェクト名
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

/// パス文字列を比較用に正規化する（末尾スラッシュ除去 + 可能なら canonicalize）。
fn normalize_path_str(p: &Path) -> String {
    let raw = p.to_string_lossy().to_string();
    let trimmed = raw.trim_end_matches('/').to_string();
    match std::fs::canonicalize(&trimmed) {
        Ok(c) => c.to_string_lossy().trim_end_matches('/').to_string(),
        Err(_) => trimmed,
    }
}

/// フォルダ名（プロジェクトルートの最終パス要素）。取れなければ `project`。
fn folder_name(project_root: &Path) -> String {
    project_root
        .file_name()
        .and_then(|s| s.to_str())
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .unwrap_or("project")
        .to_string()
}

/// レジストリ JSON（`~/.sai-moon/registry.json`）からプロジェクト名を引く。
///
/// `root` がプロジェクトルートと一致する項目の `name` を返す（パスは正規化して比較）。
/// 見つからなければ `None`。
pub fn project_name_from_registry(registry: &Path, project_root: &Path) -> Option<String> {
    let text = std::fs::read_to_string(registry).ok()?;
    let value: serde_json::Value = serde_json::from_str(&text).ok()?;
    let entries = value.as_array()?;
    let target = normalize_path_str(project_root);
    for e in entries {
        let Some(root) = e.get("root").and_then(|r| r.as_str()) else {
            continue;
        };
        if normalize_path_str(Path::new(root)) != target {
            continue;
        }
        if let Some(name) = e.get("name").and_then(|n| n.as_str()) {
            if !name.trim().is_empty() {
                return Some(name.trim().to_string());
            }
        }
    }
    None
}

/// レジストリ（任意）を使ってプロジェクト名を決める。
///
/// 一致が無ければルートのフォルダ名（例 `sai-phone`）。
pub fn project_name_using(registry: Option<&Path>, project_root: &Path) -> String {
    if let Some(reg) = registry {
        if let Some(name) = project_name_from_registry(reg, project_root) {
            return name;
        }
    }
    folder_name(project_root)
}

/// 現在のプロジェクトのプロジェクト名（`~/.sai-moon/registry.json` → フォルダ名）。
pub fn project_name_for(project_root: &Path) -> String {
    let registry = crate::project::sai_home().map(|h| h.join("registry.json"));
    project_name_using(registry.as_deref(), project_root)
}

// ============================================================================
// transcript_path の解決（00 §3.3 を厳守）
// ============================================================================

/// 会話記録 jsonl の絶対パスを解決する。
///
/// - **cc**: `session::claude_session_file(cwd, agent_session_id)`。
///   cwd はプロジェクトルート（worktree 実行なら `.worktrees/<branch>` も探索）。
///   **`cc_session_files(...)`（全体で一番新しい cc セッション）は使わない**
///   （別の実行のファイルを拾うことがあるため）。
/// - **pi**: `session::resolve_session_dir` → `session::find_main_session_file`。
/// - **gl**: `None`（今回の対象外）。
pub fn resolve_transcript(exec: &ExecutionHistory, project_root: &Path) -> Option<PathBuf> {
    match exec.runner.as_str() {
        "cc" => {
            let sid = exec.agent_session_id.as_deref()?;
            if let Some(p) = crate::session::claude_session_file(project_root, sid) {
                return Some(p);
            }
            // worktree 実行時は cwd が `.worktrees/<branch>` になるため追加探索
            let wt = project_root.join(".worktrees");
            if let Ok(entries) = std::fs::read_dir(&wt) {
                for dir in entries.flatten().map(|e| e.path()).filter(|p| p.is_dir()) {
                    if let Some(p) = crate::session::claude_session_file(&dir, sid) {
                        return Some(p);
                    }
                }
            }
            None
        }
        "gl" => None,
        _ => {
            let run_id = exec.id.unwrap_or(0);
            let dir = crate::session::resolve_session_dir(exec.session_dir.as_deref(), run_id)?;
            crate::session::find_main_session_file(&dir).ok()
        }
    }
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

/// cc 自身のセッション金額（`~/.sai-moon/cc-cost/<session_id>.json`・00 §3.8）を読む。
///
/// 無ければ `None`（呼び出し側は概算にフォールバックする）。
pub fn read_cc_cost(session_id: &str) -> Option<f64> {
    if !is_safe_id(session_id) {
        return None;
    }
    let home = crate::project::sai_home()?;
    let path = home.join("cc-cost").join(format!("{}.json", session_id));
    let text = std::fs::read_to_string(path).ok()?;
    let value: serde_json::Value = serde_json::from_str(&text).ok()?;
    value
        .get("total_cost_usd")
        .and_then(|v| v.as_f64())
        .filter(|v| v.is_finite() && *v >= 0.0)
}

/// セッション ID として安全な文字だけか（パストラバーセル防止）。
fn is_safe_id(id: &str) -> bool {
    let b = id.as_bytes();
    (8..=64).contains(&b.len())
        && b.iter()
            .all(|c| c.is_ascii_alphanumeric() || *c == b'-' || *c == b'_')
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
fn tokens_from_usage_row(store: &SaiStore, run_id: i64) -> Option<(PhoneTokens, PhoneCost)> {
    let rows = store.list_execution_usage(run_id).ok()?;
    let total = rows.into_iter().find(|r| r.entry_type == "total")?;
    let tokens = PhoneTokens {
        input: total.input_tokens.max(0) as u64,
        output: total.output_tokens.max(0) as u64,
        cache_read: total.cache_read.max(0) as u64,
        cache_write: total.cache_write.max(0) as u64,
        total: (total.input_tokens + total.output_tokens + total.cache_read + total.cache_write)
            .max(0) as u64,
    };
    let cost = PhoneCost {
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
) -> PhoneRunStats {
    // ── トークン・コスト内訳: transcript の集計（TUI Stats と同一計算） ──
    let mut totals: Option<CacheUsageTotals> = None;
    let mut top_model: Option<String> = None;
    if let Some(tp) = transcript {
        let collected = if exec.runner == "cc" {
            collect_metrics_from_file_cc(tp)
        } else {
            collect_metrics_from_file(tp)
        };
        if let Ok(metrics) = collected {
            top_model = most_frequent_model(metrics.all_messages.iter().map(|m| m.model.clone()));
            totals = Some(metrics.tree_totals);
        }
    }

    // transcript が無い・解析失敗 → execution_usage の total 行、どちらも無ければ 0
    let (tokens, mut cost) = match &totals {
        Some(t) => (
            PhoneTokens {
                input: t.input,
                output: t.output,
                cache_read: t.cache_read,
                cache_write: t.cache_write,
                total: t.total_tokens,
            },
            PhoneCost {
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
    let (cost_source, cost_is_estimate) = match exec.runner.as_str() {
        "cc" => match exec.agent_session_id.as_deref().and_then(read_cc_cost) {
            // cc 自身が数えたセッション金額（/usage の額）を最優先
            Some(cc_total) => {
                cost.total = cc_total;
                ("cc", false)
            }
            // 連携前（§2.5 導入前）の過去実行など → TUI Stats と同じ概算
            None => ("estimate", true),
        },
        "pi" => ("pi", false),
        _ => ("estimate", true),
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

    PhoneRunStats {
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
) -> PhoneRunDoc {
    let run_id = exec.id.unwrap_or(0);
    let block_name = store
        .get_block_by_id(exec.block_id)
        .map(|b| b.name)
        .unwrap_or_default();
    let transcript = resolve_transcript(exec, project_root);
    let task_md = task_md_path_for(run_id);
    let description = description_for(task_md.as_deref());
    // 子タスク（task.md の無い sub の実行）は task_create の件名を説明にする
    // （sai-phone の「直近の実行」や一覧で何の実行か分かるように）
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

    PhoneRunDoc {
        schema_version: SCHEMA_VERSION,
        project_name: project_name.to_string(),
        project_root: project_root.to_string_lossy().to_string(),
        run_id,
        parent_run_id: exec.parent_run_id,
        block_name,
        description,
        runner: exec.runner.clone(),
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

/// 実行中の pane_id を控えるファイル名（run セッション dir 直下・orchestrator が書く）。
pub const PANE_ID_FILE: &str = "herdr_pane_id";

/// run セッション dir に控えた pane_id（DB に未記録の実行中用・無ければ None）。
fn pane_id_from_session_dir(exec: &ExecutionHistory) -> Option<String> {
    let dir = crate::session::resolve_session_dir(exec.session_dir.as_deref(), exec.id?)?;
    let id = std::fs::read_to_string(dir.join(PANE_ID_FILE)).ok()?;
    let id = id.trim();
    (!id.is_empty()).then(|| id.to_string())
}

/// 現在時刻（UTC ISO 8601・末尾 Z）。
pub fn now_iso() -> String {
    chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string()
}

/// 現在の UNIX 秒（小数）。
///
/// `~/.claude/statusline-command.sh` の `jq '{rate_limits, saved_at: now}'` と同じ形にするため。
fn unix_now_f64() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

/// JSON を一時ファイル → rename で書く（読み手が書きかけを読まないようにするため）。
///
/// `statusline-command.sh` と同じ形。親ディレクトリが無ければ作る。
/// `tmp` と `dst` は同じディレクトリに置くこと（rename が原子的なのは同一 FS のため）。
fn write_json_atomic(tmp: &Path, dst: &Path, body: &serde_json::Value) -> std::io::Result<()> {
    if let Some(parent) = dst.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let text = serde_json::to_string_pretty(body)
        .map_err(|e| std::io::Error::other(format!("JSON 直列化失敗: {}", e)))?;
    std::fs::write(tmp, text)?;
    std::fs::rename(tmp, dst)?;
    Ok(())
}

// ============================================================================
// ファイル書き出し
// ============================================================================

/// `~/.sai-moon/phone/runs` のルート（置き場を解決できなければ None）。
pub fn export_root() -> Option<PathBuf> {
    crate::project::sai_home().map(|h| h.join("phone").join("runs"))
}

/// 書き出し先フォルダ `~/.sai-moon/phone/runs/<project_name>`（置き場を解決できなければ None）。
pub fn export_dir(project_name: &str) -> Option<PathBuf> {
    export_root().map(|r| r.join(project_name))
}

/// 1 実行分の JSON を指定フォルダに書く（一時ファイル → rename）。
///
/// 戻り値: `true` = 書いた、`false` = 同じ内容が既にあり省略した。
pub fn write_run_json_in(dir: &Path, doc: &PhoneRunDoc) -> std::io::Result<bool> {
    std::fs::create_dir_all(dir)?;
    let final_path = dir.join(format!("{}.json", doc.run_id));
    let tmp_path = dir.join(format!(".{}.json.tmp", doc.run_id));

    let text = serde_json::to_string_pretty(doc)
        .map_err(|e| std::io::Error::other(format!("JSON 直列化失敗: {}", e)))?;

    // 同じ内容（updated_at を除く）が既にあれば書かない（sai-phone の再読込防止）
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
pub fn write_run_json(doc: &PhoneRunDoc) -> std::io::Result<Option<bool>> {
    let Some(dir) = export_dir(&doc.project_name) else {
        return Ok(None);
    };
    write_run_json_in(&dir, doc).map(Some)
}

// ============================================================================
// 一括書き出し
// ============================================================================

/// 差分検知用のシグネチャ（実装計画 01 §2.3）。
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
            std::io::Error::other("HOME が未設定のため phone 書き出し先を決められません")
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
    pub fn build_doc(&self, store: &SaiStore, exec: &ExecutionHistory) -> PhoneRunDoc {
        build_run_doc(store, exec, &self.project_root, &self.project_name)
    }

    /// 1 実行分を書き出す。
    ///
    /// 戻り値: `Ok(Some(書いたか))` = 書き出し対象、
    /// `Ok(None)` = 対象外（`todo` / `pending` / runner=`user`）。
    pub fn export(
        &self,
        store: &SaiStore,
        exec: &ExecutionHistory,
    ) -> std::io::Result<Option<bool>> {
        if !should_export(&exec.status, &exec.runner) {
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
            "{} 件書く / {} 件省略 / {} 件失敗",
            self.written, self.skipped, self.failed
        )
    }
}

/// 書き出し対象の実行をドキュメント列にする（`--dry-run` 用・ファイルは書かない）。
pub fn build_docs(
    store: &SaiStore,
    execs: &[ExecutionHistory],
    project_root: &Path,
) -> Vec<PhoneRunDoc> {
    let project_name = project_name_for(project_root);
    execs
        .iter()
        .filter(|e| should_export(&e.status, &e.runner))
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
            eprintln!("[sai-engine] phone 書き出し: {}", e);
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
                // 失敗はログのみ（sai-phone 連携はおまけ機能・実行は止めない）
                summary.failed += 1;
                eprintln!(
                    "[sai-engine] phone 書き出し失敗: run_id={}: {}",
                    exec.id.unwrap_or(0),
                    e
                );
            }
        }
    }
    summary
}

/// プロジェクトの全実行を書き出す（デーモン起動時・`sai-link phone export`）。
pub fn export_all(store: &SaiStore) -> ExportSummary {
    let project_root = crate::project::project_root();
    let execs = match store.list_all_executions() {
        Ok(e) => e,
        Err(e) => {
            eprintln!("[sai-engine] phone 書き出し: 実行一覧の取得に失敗: {}", e);
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
        .map_err(|e| std::io::Error::other(format!("実行が見つからない: {}", e)))?;
    let project_root = crate::project::project_root();
    let exporter = Exporter::new(&project_root)?;
    exporter.export(store, &exec)
}

// ============================================================================
// cc statusLine（00 §3.8 / 01 §2.5）
// ============================================================================

/// `~/.sai-moon/cc-cost` の出力先（置き場を解決できなければ None）。
pub fn cc_cost_dir() -> Option<PathBuf> {
    crate::project::sai_home().map(|h| h.join("cc-cost"))
}

/// `~/.sai-moon/phone` の出力先（置き場を解決できなければ None）。
///
/// sai-phone の Usage タブ（`usage.py` の `cc_limits`）がここから
/// `cc-rate-limits.json` を読む。
pub fn cc_rate_limits_dir() -> Option<PathBuf> {
    crate::project::sai_home().map(|h| h.join("phone"))
}

/// `~/.sai-moon/phone/cc-rate-limits.json` の出力先（置き場を解決できなければ None）。
pub fn cc_rate_limits_path() -> Option<PathBuf> {
    cc_rate_limits_dir().map(|d| d.join("cc-rate-limits.json"))
}

/// cc の statusLine 標準入力 1 件を処理する（`sai-link cc-statusline` の本体）。
///
/// - 標準入力 JSON から `session_id` と `cost.total_cost_usd` を取り出し、
///   `~/.sai-moon/cc-cost/<session_id>.json` に `{session_id, total_cost_usd, updated_at}`
///   を書く（一時ファイル → rename）。
/// - 戻り値は cc の画面下に出す 1 行（例 `sai #58 $1.27`）。
/// - 読めない・JSON が壊れている・書けない場合は **空行**（cc の画面を壊さない）。
pub fn handle_cc_statusline(input: &str) -> String {
    let Some(dir) = cc_cost_dir() else {
        return String::new();
    };
    handle_cc_statusline_in(input, &dir, cc_rate_limits_path().as_deref())
}

/// [`handle_cc_statusline`] の出力先指定版（テスト用）。
///
/// `rate_limits_path` は `cc-rate-limits.json` の出力先（`None` なら書かない）。
pub fn handle_cc_statusline_in(
    input: &str,
    dir: &Path,
    rate_limits_path: Option<&Path>,
) -> String {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(input) else {
        return String::new();
    };
    let Some(session_id) = value.get("session_id").and_then(|v| v.as_str()) else {
        return String::new();
    };
    if !is_safe_id(session_id) {
        return String::new();
    }
    // 5 時間枠（rate_limits）を sai-phone へ保存する（U04）。
    // sai-link は cc の statusLine を差し替えるため、既定の statusline-command.sh が
    // 書いていた `~/.sai-moon/phone/cc-rate-limits.json` が更新されなくなる。
    // cost の有無に関係なく、rate_limits があれば書く。失敗しても画面表示は変えない。
    if let Some(rate_limits) = value.get("rate_limits") {
        if let Some(path) = rate_limits_path {
            let body = serde_json::json!({
                "rate_limits": rate_limits,
                "saved_at": unix_now_f64(),
            });
            let tmp = path.with_file_name(".cc-rate-limits.json.tmp");
            if let Err(e) = write_json_atomic(&tmp, path, &body) {
                eprintln!("[sai-engine] cc-rate-limits の書込みに失敗: {}", e);
            }
        }
    }
    // コストが取れない場合は書かない（概算フォールバックを妨げないため）
    let Some(total) = value
        .get("cost")
        .and_then(|c| c.get("total_cost_usd"))
        .and_then(|v| v.as_f64())
        .filter(|v| v.is_finite() && *v >= 0.0)
    else {
        return String::new();
    };

    // 一時ファイル → rename（読み手が書きかけを読まないため）
    let tmp = dir.join(format!(".{}.json.tmp", session_id));
    let dst = dir.join(format!("{}.json", session_id));
    let body = serde_json::json!({
        "session_id": session_id,
        "total_cost_usd": total,
        "updated_at": now_iso(),
    });
    if let Err(e) = write_json_atomic(&tmp, &dst, &body) {
        eprintln!("[sai-engine] cc-cost の書込みに失敗: {}", e);
        return String::new();
    }

    // 画面表示: SAI_PARENT_RUN_ID（sai-link が cc に渡す実行 ID）があれば run_id 付き
    let amount = format!("${:.2}", total);
    match std::env::var("SAI_PARENT_RUN_ID") {
        Ok(id) if !id.trim().is_empty() => format!("sai #{} {}", id.trim(), amount),
        _ => format!("sai {}", amount),
    }
}

// ============================================================================
// テスト（01 §4）
// ============================================================================

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

    /// レジストリに一致が無いときフォルダ名になる（00 §3.2 project_name）。
    #[test]
    fn project_name_falls_back_to_folder_name() {
        let tmp = std::env::temp_dir().join(format!(
            "sai_phone_reg_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let root = tmp.join("my-project");
        std::fs::create_dir_all(&root).unwrap();
        let registry = tmp.join("registry.json");
        // 別プロジェクトのみ登録（一致なし）
        std::fs::write(
            &registry,
            r#"[{"name":"other","root":"/nowhere/other"}]"#,
        )
        .unwrap();
        assert_eq!(project_name_using(Some(&registry), &root), "my-project");

        // 登録があれば name が使われる（末尾スラッシュ差は吸収）
        let root_str = root.to_string_lossy().to_string();
        std::fs::write(
            &registry,
            format!(
                r#"[{{"name":"registered-name","root":"{}/"}}]"#,
                root_str.replace('\\', "\\\\")
            ),
        )
        .unwrap();
        assert_eq!(project_name_using(Some(&registry), &root), "registered-name");

        // レジストリが無ければフォルダ名
        assert_eq!(project_name_using(None, &root), "my-project");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// `todo` / `pending` と runner=`user` は書き出されない（00 §3.2）。
    #[test]
    fn todo_and_user_runs_are_not_exported() {
        assert!(!should_export("todo", "pi"));
        assert!(!should_export("pending", "cc"));
        assert!(!should_export("running", "user"));
        assert!(!should_export("completed", "user"));
        // それ以外は書き出す
        assert!(should_export("running", "pi"));
        assert!(should_export("completed", "pi"));
        assert!(should_export("failed", "cc"));
        assert!(should_export("cancelled", "gl"));
    }

    /// 一時ファイル → rename で最終ファイルができ、`.tmp` が残らないこと。
    /// また、同じ内容の再書き出しは省略されること。
    #[test]
    fn write_run_json_uses_tmp_rename_and_skips_unchanged() {
        let dir = std::env::temp_dir().join(format!(
            "sai_phone_w_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();

        let doc = PhoneRunDoc {
            schema_version: SCHEMA_VERSION,
            project_name: "demo".to_string(),
            project_root: "/tmp/demo".to_string(),
            run_id: 42,
            parent_run_id: None,
            block_name: "b".to_string(),
            description: "説明".to_string(),
            runner: "pi".to_string(),
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
        let loaded: PhoneRunDoc =
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

    /// statusLine 標準入力 JSON から金額を取り出しファイルを書けること（00 §3.8）。
    #[test]
    fn cc_statusline_writes_cost_file() {
        let dir = std::env::temp_dir().join(format!(
            "sai_cc_cost_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let session_id = "11111111-2222-4333-8444-555555555555";
        let input = format!(
            r#"{{"session_id":"{}","transcript_path":"/tmp/x.jsonl","cost":{{"total_cost_usd":1.274}}}}"#,
            session_id
        );
        let line = handle_cc_statusline_in(&input, &dir, None);
        assert!(line.contains("1.27"), "画面表示に金額が入る: {}", line);
        assert!(!dir.join(format!(".{}.json.tmp", session_id)).exists());

        let text = std::fs::read_to_string(dir.join(format!("{}.json", session_id))).unwrap();
        let v: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(v["session_id"], session_id);
        assert!((v["total_cost_usd"].as_f64().unwrap() - 1.274).abs() < 1e-9);
        assert!(v["updated_at"].as_str().unwrap().ends_with('Z'));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// statusLine 入力が壊れている・コストが無いときは空行を返しファイルを書かない。
    #[test]
    fn cc_statusline_swallows_bad_input() {
        let dir = std::env::temp_dir().join(format!(
            "sai_cc_cost_bad_{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        // 壊れた JSON
        assert_eq!(handle_cc_statusline_in("not json", &dir, None), "");
        // session_id なし
        assert_eq!(
            handle_cc_statusline_in(r#"{"cost":{"total_cost_usd":1.0}}"#, &dir, None),
            ""
        );
        // cost なし（概算フォールバックを妨げないため書かない）
        let sid = "11111111-2222-4333-8444-555555555555";
        assert_eq!(
            handle_cc_statusline_in(&format!(r#"{{"session_id":"{}"}}"#, sid), &dir, None),
            ""
        );
        assert!(!dir.join(format!("{}.json", sid)).exists());
        // パストラバーセッション ID
        assert_eq!(
            handle_cc_statusline_in(
                r#"{"session_id":"../../etc/passwd","cost":{"total_cost_usd":1.0}}"#,
                &dir,
                None
            ),
            ""
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// statusLine 入力に `rate_limits` があれば `~/.sai-moon/phone/cc-rate-limits.json`
    /// が書かれること（U04）。cost が無くても書く。
    #[test]
    fn cc_statusline_writes_rate_limits_without_cost() {
        let dir = std::env::temp_dir().join(format!(
            "sai_cc_rl_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let rl_path = dir.join("phone").join("cc-rate-limits.json");
        let sid = "11111111-2222-4333-8444-555555555555";

        // cost なし・rate_limits あり → rate-limits だけ書く（画面表示は空行）
        let line = handle_cc_statusline_in(
            &format!(
                r#"{{"session_id":"{}","rate_limits":{{"five_hour":{{"used_percent":12.5}}}}}}"#,
                sid
            ),
            &dir,
            Some(&rl_path),
        );
        assert_eq!(line, "", "cost が無いので画面表示は空行: {}", line);
        let v: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&rl_path).unwrap()).unwrap();
        assert_eq!(v["rate_limits"]["five_hour"]["used_percent"], 12.5);
        assert!(
            v["saved_at"].as_f64().is_some(),
            "saved_at は UNIX 秒の小数: {}",
            v["saved_at"]
        );
        // 一時ファイルは残さない
        assert!(!rl_path.with_file_name(".cc-rate-limits.json.tmp").exists());
        // cost ファイルは書かれていない
        assert!(!dir.join(format!("{}.json", sid)).exists());

        // rate_limits 無し → rate-limits ファイルは書かない（古い値を残す）
        let before = std::fs::read_to_string(&rl_path).unwrap();
        handle_cc_statusline_in(
            &format!(r#"{{"session_id":"{}","cost":{{"total_cost_usd":1.0}}}}"#, sid),
            &dir,
            Some(&rl_path),
        );
        assert_eq!(std::fs::read_to_string(&rl_path).unwrap(), before);

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 実行 1 行分のドキュメントが DB の値から正しく構築されること。
    #[test]
    fn build_run_doc_from_db_row() {
        let _env = crate::common::SAI_ENV_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // 既存の SAI_PROJECT_ROOT を退避して終了時に復元する（並行テストへの影響防止）
        let prev_root = std::env::var_os("SAI_PROJECT_ROOT");
        let tmp = std::env::temp_dir().join(format!("sai_phone_doc_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(tmp.join(".sai")).unwrap();
        std::env::set_var("SAI_PROJECT_ROOT", &tmp);

        let store = SaiStore::open_in_memory().unwrap();
        let block = BlockMetadata::new(
            "b".to_string(),
            "テストブロック".to_string(),
            "m".to_string(),
            None,
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
        assert!(!should_export(&exec.status, &exec.runner));

        // running へ遷移（started_at が入る）→ 書き出し対象
        store.update_execution_status(run_id, "running").unwrap();
        let exec = store.get_execution_by_id(run_id).unwrap();
        assert!(should_export(&exec.status, &exec.runner));

        let doc = build_run_doc(&store, &exec, &tmp, "demo-project");
        assert_eq!(doc.schema_version, 1);
        assert_eq!(doc.run_id, run_id);
        assert_eq!(doc.block_name, "b");
        assert_eq!(doc.runner, "pi");
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
        // transcript が無い → トークン・コストは 0、pi runner は cost_source=pi
        assert_eq!(stats.tokens.total, 0);
        assert_eq!(stats.cost_source, "pi");
        assert!(!stats.cost_is_estimate);

        let _ = std::fs::remove_dir_all(&tmp);
        match prev_root {
            Some(v) => std::env::set_var("SAI_PROJECT_ROOT", v),
            None => std::env::remove_var("SAI_PROJECT_ROOT"),
        }
    }
}
