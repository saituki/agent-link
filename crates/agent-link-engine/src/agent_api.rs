//! # エージェント共通 API（pi / cc 共通の実体）
//!
//! pi/cc から CLI（`agent-link doc write`・`agent-link task create` など）を通じて
//! 呼ばれる機能の唯一の実装です。エージェントは Bash で CLI を呼ぶ
//! （cc は `.claude/settings.json` の `Bash(agent-link *)` の許可、
//! pi は同じ CLI）。旧 MCP サーバと旧 pi-agent-link/（TypeScript 拡張）は廃止し削除済み。
//!
//! すべての関数は `Result<serde_json::Value, String>` を返します。
//! `Err` の文字列はそのままエージェントへ見せる日本語のメッセージです。
//!
//! ツール承認機能（task_create / task_manage のユーザー確認 UI）は廃止済みです。
//! `agent-link task create` は受け取った全タスクを常に承認済みとして作成します。

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use serde_json::{json, Value};

use crate::block_config::BlockConfig;
use crate::db::models::{ExecutionHistory, Task};
use crate::parser::parse_block_file;
use crate::task_manager::TaskManager;

// ============================================================================
// 共通ヘルパー
// ============================================================================

/// `AGENT_LINK_PARENT_RUN_ID` を解決する。
fn env_run_id() -> Option<i64> {
    std::env::var("AGENT_LINK_PARENT_RUN_ID")
        .ok()
        .and_then(|v| v.trim().parse::<i64>().ok())
}

/// 実行 ID を解決する。引数優先、無ければ環境変数 `AGENT_LINK_PARENT_RUN_ID`。
/// 両方無ければエラー。
fn require_run_id(explicit: Option<i64>) -> Result<i64, String> {
    explicit
        .or_else(env_run_id)
        .ok_or_else(|| "AGENT_LINK_PARENT_RUN_ID is not set. This command works only inside a run.".to_string())
}

/// ブロック名を解決する（`AGENT_LINK_BLOCK_NAME`、無ければ `"cc"`）。
fn block_name() -> String {
    std::env::var("AGENT_LINK_BLOCK_NAME").unwrap_or_else(|_| "cc".to_string())
}

/// ミリ秒タイムスタンプ。
fn unix_millis() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

/// 4 バイトの乱数を 8 桁 16 進数文字列にする（通知ファイル名の一意化用）。
fn random_hex8() -> String {
    let mut bytes = [0u8; 4];
    if getrandom::getrandom(&mut bytes).is_err() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let mix = (nanos as u64) ^ ((std::process::id() as u64) << 32);
        bytes.copy_from_slice(&mix.to_le_bytes()[..4]);
    }
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

/// 英数小文字 4 文字の乱数（batch_id 用。移植元は `Math.random().toString(36).substring(2,6)`）。
fn random_base36_4() -> String {
    const CHARS: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    let mut bytes = [0u8; 4];
    if getrandom::getrandom(&mut bytes).is_err() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let mix = (nanos as u64) ^ ((std::process::id() as u64) << 32);
        bytes.copy_from_slice(&mix.to_le_bytes()[..4]);
    }
    bytes
        .iter()
        .map(|b| CHARS[(*b as usize) % CHARS.len()] as char)
        .collect()
}

/// 拡張子から MIME タイプを推測する（pi-agent-link の `guessMimeType` と同等）。
fn guess_mime_type(path: &Path) -> String {
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "md" => "text/markdown",
        "txt" => "text/plain",
        "json" => "application/json",
        "yaml" | "yml" => "text/yaml",
        "html" => "text/html",
        "css" => "text/css",
        "js" => "text/javascript",
        "ts" => "text/typescript",
        "rs" => "text/rust",
        "py" => "text/python",
        "sh" => "text/shell",
        "toml" => "text/toml",
        _ => "application/octet-stream",
    }
    .to_string()
}

/// 通知 JSON を `pending/` に書き込む。
///
/// - 保存先: `crate::common::notify_pending_dir()`
/// - ファイル名: `<timestamp_ms>-<random_hex8>.json`
/// - 内容: `payload` に `type` と `timestamp`（ISO 8601）を必ず含めた JSON
///
/// **全通知の唯一の書き込み口**。通知 JSON の必須フィールド名は変更しないこと
/// （daemon 側の受け口がそのまま前提にしている）。
fn write_notification(kind: &str, mut payload: Value) -> Result<PathBuf, String> {
    let dir = crate::common::notify_pending_dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("Failed to create the notification directory: {}", e))?;

    if let Value::Object(ref mut map) = payload {
        map.insert("type".to_string(), json!(kind));
        map.insert(
            "timestamp".to_string(),
            json!(crate::common::iso_8601_timestamp()),
        );
    }

    let filename = format!("{}-{}.json", unix_millis(), random_hex8());
    let path = dir.join(filename);
    let text = serde_json::to_string_pretty(&payload)
        .map_err(|e| format!("Failed to serialize the notification: {}", e))?;
    std::fs::write(&path, text).map_err(|e| format!("Failed to write the notification: {}", e))?;
    Ok(path)
}

// ============================================================================
// ブロック定義の参照（block_discover / block_inspect / task_create の入力検証で共用）
// ============================================================================

/// ブロック定義一覧（block_discover）の JSON 出力用エントリ。
#[derive(serde::Serialize)]
pub struct BlockDefListJson {
    /// 機械可読 ID（`task_create` の `block_type` に使用）
    pub id: String,
    /// 表示名（画面での表示用。emoji 可）
    pub name: String,
    pub description: Option<String>,
}

/// ブロック定義詳細（block_inspect）の JSON 出力用構造体。
#[derive(serde::Serialize)]
pub struct BlockDefJson {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub subblocks: Vec<String>,
    pub worktree: Option<String>,
    pub timeout: Option<u32>,
    pub templates: Vec<String>,
}

/// `BlockConfig` を詳細 JSON 構造体に変換する。
pub fn block_def_to_json(config: &BlockConfig) -> BlockDefJson {
    BlockDefJson {
        id: config.block_id().to_string(),
        name: config.display_name().to_string(),
        description: config.description.clone(),
        subblocks: config.subblocks.clone().unwrap_or_default(),
        worktree: config.worktree.clone(),
        timeout: config.timeout,
        templates: config.templates.clone().unwrap_or_default(),
    }
}

/// ブロック定義ファイルを各ディレクトリから収集する（ID で重複排除・先勝ち）。
///
/// 探索順: `PROJECTROOT/.agent-link/blocks` → `blocks`（開発用 cwd）→ `lib/herdrs/blocks`。
pub fn collect_block_definitions() -> Vec<(PathBuf, BlockConfig)> {
    let mut out: Vec<(PathBuf, BlockConfig)> = Vec::new();
    let dirs = [
        crate::project::sai_path(".agent-link/blocks"),
        PathBuf::from("blocks"),
        PathBuf::from("lib/herdrs/blocks"),
    ];
    for dir in dirs {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().map(|e| e == "md").unwrap_or(false) {
                if let Ok(parsed) = parse_block_file(&path) {
                    if !out
                        .iter()
                        .any(|(_, c)| c.block_id() == parsed.config.block_id())
                    {
                        out.push((path, parsed.config));
                    }
                }
            }
        }
    }
    out
}

// ============================================================================
// task list --json 出力用エントリ（pi-agent-link の task_list / task_status が使用）
// ============================================================================

/// `task list --json` の出力用エントリ（pi-agent-link の `task_list` / `task_status` が使用）。
#[derive(serde::Serialize)]
pub struct TaskJsonEntry {
    pub server_id: i64,
    pub parent_run_id: i64,
    pub subject: String,
    pub description: String,
    pub status: String,
    pub block_type: Option<String>,
    pub inputs: Option<Value>,
    pub depends_on: Vec<String>,
    pub worktree: String,
    pub cwd: Option<String>,
    /// 子の時間上限（秒。`None` はブロック定義の `timeout:` に従う）
    pub timeout: Option<u32>,
    pub execution_id: Option<i64>,
    pub batch_id: Option<String>,
    pub client_id: Option<String>,
    pub round: i64,
    pub supersedes_batch_id: Option<String>,
    pub approval_comment: Option<String>,
    pub finalized: bool,
    pub deleted_by: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

/// `Task`（DB 行）を agent_api の JSON 表現に変換する（task_list / task_status 共用）。
///
/// `defs` はブロック定義一覧。親が作業用コピーをマージするために使う
/// `worktree_info`（ブランチ名・リポジトリ・作業用コピーの場所）の算出に使う。
fn task_to_json(t: &Task, defs: &[(PathBuf, BlockConfig)]) -> Value {
    json!({
        "server_id": t.id.unwrap_or(0),
        "parent_run_id": t.parent_run_id,
        "subject": t.subject,
        "description": t.description,
        "status": t.status,
        "block_type": t.block_type,
        "inputs": t.inputs_json.as_deref().and_then(|j| serde_json::from_str::<Value>(j).ok()),
        "depends_on": t.depends_on_json.as_deref()
            .and_then(|j| serde_json::from_str::<Vec<String>>(j).ok())
            .unwrap_or_default(),
        "worktree": t.worktree,
        "cwd": t.cwd,
        "host": t.host,
        "timeout": t.timeout,
        "card": t.card_run_id,
        "worktree_info": worktree_info_json(t, defs),
        "execution_id": t.execution_id,
        "batch_id": t.batch_id,
        "client_id": t.client_id,
        "round": t.round,
        "supersedes_batch_id": t.supersedes_batch_id,
        "approval_comment": t.approval_comment,
        "finalized": t.finalized,
        "deleted_by": t.deleted_by,
        "created_at": t.created_at,
        "updated_at": t.updated_at,
    })
}

/// 親が作業用コピーをマージするための情報を求める。
///
/// - `worktree`: 実効の worktree 指定（親指定が優先。未指定ならブロック定義に従う）
/// - `branch`: ブランチ名（`<block_type>-<execution_id>` またはリテラル。未実行の isolation は null）
/// - `repo_root`: 作業用コピーを作る対象リポジトリのルート
/// - `path`: 作業用コピーの場所（`<repo_root>/.worktrees/<branch>`）
fn worktree_info_json(t: &Task, defs: &[(PathBuf, BlockConfig)]) -> Value {
    // 遠隔で動かす子は作業用コピーを作らない
    if t.host.is_some() {
        return Value::Null;
    }
    // 親指定（空 = 未指定）が最優先。未指定ならブロック定義の `worktree:` に従う。
    let effective: String = if !t.worktree.trim().is_empty() {
        t.worktree.trim().to_string()
    } else {
        t.block_type
            .as_deref()
            .and_then(|bt| defs.iter().find(|(_, c)| c.block_id() == bt))
            .and_then(|(_, c)| c.worktree.clone())
            .unwrap_or_else(|| "none".to_string())
    };
    let block_type = t.block_type.as_deref().unwrap_or("");
    let cwd_path = t.cwd.as_deref().map(PathBuf::from);
    // isolation は実行 ID でブランチが決まるため、未実行の間は確定させない。
    let plan = if effective == "isolation" && t.execution_id.is_none() {
        None
    } else {
        crate::orchestrator::worktree_plan(
            cwd_path.as_deref(),
            &effective,
            block_type,
            t.execution_id.unwrap_or(0),
        )
    };
    match plan {
        Some(plan) => json!({
            "worktree": effective,
            "branch": plan.branch,
            "repo_root": plan.repo_root.to_string_lossy(),
            "path": plan.worktree_path.to_string_lossy(),
        }),
        None => json!({
            "worktree": effective,
            "branch": null,
            "repo_root": null,
            "path": null,
        }),
    }
}

/// `run_id` のタスクを取得し、ステータス絞り込み・ソートを行って JSON 配列にする
/// （`task_list` / `task_status` 共用）。
fn tasks_json(run_id: i64, status: Option<&str>, sort: Option<&str>) -> Result<Vec<Value>, String> {
    let store = Arc::new(crate::common::open_db().map_err(|e| format!("Failed to open the database: {}", e))?);
    let manager = TaskManager::new(store);
    let mut tasks = manager
        .list_tasks(run_id)
        .map_err(|e| format!("Failed to list subtasks: {}", e))?;
    // worktree_info（マージ用情報）算出のためブロック定義を一度だけ読む
    let defs = collect_block_definitions();

    if let Some(s) = status {
        if s != "all" {
            tasks.retain(|t| t.status == s);
        }
    }

    match sort.unwrap_or("id") {
        "recent" => tasks.sort_by(|a, b| b.created_at.cmp(&a.created_at)),
        "oldest" => tasks.sort_by(|a, b| a.created_at.cmp(&b.created_at)),
        _ => tasks.sort_by_key(|t| t.id.unwrap_or(0)),
    }

    Ok(tasks.iter().map(|t| task_to_json(t, &defs)).collect())
}

// ============================================================================
// 3.1 write（sai-write）
// ============================================================================

/// 成果物ファイルを自 run フォルダに保存し、`file_written` 通知を書き込む。
///
/// P4: 保存と同時にその場で waggle トークン化し、トークンと参照記法（`ref`）を返す。
/// mint はランダムなので、デーモンが再度トークン化するとトークンが 2 つになる。
/// 通知 payload に `waggle_token` を入れておき、デーモンは再トークン化しない。
pub fn write(
    filename: &str,
    content: &str,
    description: Option<&str>,
    run_id: Option<i64>,
) -> Result<Value, String> {
    write_with_timeout(filename, content, description, run_id, WRITE_CHECK_TIMEOUT)
}

/// `write_check` の命令の打ち切り時間。
const WRITE_CHECK_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// [`write`] の本体。`write_check` の時間の上限を引数で受ける（テストで短くするため）。
fn write_with_timeout(
    filename: &str,
    content: &str,
    description: Option<&str>,
    run_id: Option<i64>,
    check_timeout: std::time::Duration,
) -> Result<Value, String> {
    let trimmed = filename.trim();
    if trimmed.is_empty() {
        return Err("filename is required.".to_string());
    }
    // パス区切りを含む filename は run フォルダ直下の basename のみ使う
    let basename = Path::new(trimmed)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .ok_or("filename is invalid.")?;

    let run_id = require_run_id(run_id)?;

    // settings.json の write_check があれば、保存の前に本文を通す。
    let mut checked: Option<bool> = None;
    let mut warning: Option<String> = None;
    let mut saved_content = content.to_string();
    let check_cmd = crate::settings::load_settings_quiet().0.write_check.unwrap_or_default();
    if !check_cmd.is_empty() {
        let block = block_name();
        let cmd: Vec<String> = check_cmd
            .iter()
            .map(|a| {
                a.replace("{run_id}", &run_id.to_string())
                    .replace("{block}", &block)
                    .replace("{filename}", &basename)
            })
            .collect();
        let envs = [
            ("AGENT_LINK_WRITE_FILENAME", basename.clone()),
            ("AGENT_LINK_WRITE_DESCRIPTION", description.unwrap_or("").to_string()),
            ("AGENT_LINK_PARENT_RUN_ID", run_id.to_string()),
            ("AGENT_LINK_BLOCK_NAME", block),
        ];
        use crate::pre_run::StdinOutcome;
        match crate::pre_run::run_with_stdin(&cmd, &crate::project::project_root(), &envs, content, check_timeout) {
            StdinOutcome::Ok(out) => {
                checked = Some(true);
                if !out.is_empty() {
                    saved_content = out;
                }
            }
            StdinOutcome::Rejected(msg) => {
                return Err(format!("Not saved. The document breaks the rules:\n{msg}"));
            }
            StdinOutcome::Broken(why) => {
                checked = Some(false);
                warning = Some(format!("The checker did not run ({why}). Saved without checking."));
            }
        }
    }
    let content = saved_content.as_str();

    let path = crate::doc_write_notify::resolve_output_path(run_id, &basename)
        .map_err(|e| format!("Failed to resolve the save path: {}", e))?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("Failed to create the save directory: {}", e))?;
    }
    std::fs::write(&path, content).map_err(|e| format!("Failed to write the file: {}", e))?;
    let byte_size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);

    // P4: エージェント側でその場でトークン化する。失敗したらデーモンの非同期処理に任せる。
    let token = tokenize_saved_file(&path, description);
    // `doc list` は DB の output_waggle_tokens を読むので、デーモンの取り込みを待たずその場で載せる。
    // 載せられなくても保存は成功させる（デーモンの取り込みが同じ追記をする）。
    if let Some(tok) = token.as_deref() {
        if let Ok(store) = crate::common::open_db() {
            let _ = store.append_execution_waggle_token(run_id, tok);
        }
    }

    let mut payload = json!({
        "run_id": run_id,
        "file_path": path.to_string_lossy(),
        "block_name": block_name(),
        "description": description,
        "mime_type": guess_mime_type(&path),
        "byte_size": byte_size,
    });
    if let Some(tok) = token.as_deref() {
        // デーモンはこのトークンを見て再トークン化をスキップする
        payload["waggle_token"] = json!(tok);
    }
    write_notification("file_written", payload)?;

    let mut result = match token {
        Some(tok) => json!({
            "status": "ok",
            "path": path.to_string_lossy(),
            "byte_size": byte_size,
            "token": tok,
            "ref": format!("waggle:{}:path:.agent-link/outputs/*/{}/{}", tok, run_id, basename),
            "message": "Saved. When another document refers to this one, write the ref form.",
        }),
        None => json!({
            "status": "ok",
            "path": path.to_string_lossy(),
            "byte_size": byte_size,
            "token": Value::Null,
            "ref": Value::Null,
            "message": "Saved. The daemon turns the document into a waggle token asynchronously.",
        }),
    };
    if let Some(c) = checked {
        result["checked"] = json!(c);
    }
    if let Some(w) = warning {
        result["warning"] = json!(w);
    }
    Ok(result)
}

/// 保存直後のファイルをその場で waggle トークン化し、トークン文字列を返す（P4）。
///
/// DB を開けない・トークン化に失敗した場合は `None` を返し、
/// これまでどおりデーモンの非同期トークン化に任せる。
fn tokenize_saved_file(path: &Path, description: Option<&str>) -> Option<String> {
    let db_path = crate::project::db_path();
    if let Some(parent) = db_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let integrator = crate::waggle_integration::WaggleIntegrator::open(&db_path).ok()?;
    integrator
        .tokenize_file(path, &block_name(), description)
        .ok()
        .map(|t| t.token.as_str().to_string())
}

// ============================================================================
// 3.2 finalize
// ============================================================================

/// 報告のファイル名（`task_report` と `finalize` が同じ場所に書く）。
pub const REPORT_FILE: &str = "finalize-report.md";

/// 報告（`finalize-report.md`）の読み込み先を探す。run フォルダが無い場合は `None`。
///
/// 読むだけなので、フォルダを作らない（未実行の todo カードでも 404 になるようにする）。
pub fn report_path_for_run(run_id: i64) -> Option<std::path::PathBuf> {
    let (_, dir) = crate::run_dir::find_run_dir(run_id)?;
    Some(dir.join(REPORT_FILE))
}

/// 報告の本文を run フォルダの `finalize-report.md` に書く。
///
/// waggle トークン化はしない（通知を書かない）。`task_report` と `finalize` の共用。
fn write_report_file(run_id: i64, report: &str) -> Result<std::path::PathBuf, String> {
    let path = match crate::run_dir::find_run_dir(run_id) {
        Some((_, dir)) => dir.join(REPORT_FILE),
        None => crate::doc_write_notify::resolve_output_path(run_id, REPORT_FILE)
            .map_err(|e| format!("Failed to resolve the save path: {}", e))?,
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("Failed to create the save directory: {}", e))?;
    }
    std::fs::write(&path, report).map_err(|e| format!("Failed to save the report: {}", e))?;
    Ok(path)
}

/// `finalize_request` 通知を書き込み、実行の明示完了を要求する。
///
/// `report` が空でなければ、その内容を run フォルダの `finalize-report.md` に
/// 先に保存する（Web の run の画面と親の `task_output` が読む場所）。
/// 空なら保存済みの報告に一切触らない。
pub fn finalize(run_id: Option<i64>, report: Option<&str>) -> Result<Value, String> {
    let run_id = require_run_id(run_id)?;
    let store =
        crate::common::open_db().map_err(|e| format!("Failed to open the database: {}", e))?;
    let exec = store
        .get_execution_by_id(run_id)
        .map_err(|_| format!("Run {} not found.", run_id))?;
    if exec.status != "running" {
        return Err(format!("Run {} is not running (status: {}).", run_id, exec.status));
    }
    if let Some(text) = report.map(str::trim).filter(|t| !t.is_empty()) {
        write_report_file(run_id, text)?;
    }
    crate::daemon::write_finalize_request(run_id)
        .map_err(|e| format!("Failed to write finalize_request: {}", e))?;
    Ok(json!({
        "status": "requested",
        "run_id": run_id,
        "message": format!("Requested finalization of run_id={}.", run_id),
    }))
}

// ============================================================================
// 3.3 task_create（承認廃止）
// ============================================================================

/// 正規化済みタスク（内部表現）。
/// U05: `block` / `description` は task.md と同じ項目名でも受け付ける。
/// task.md 作成処理と `task_create` 引数を同じにするための互換。
struct NormalizedTask {
    client_id: String,
    subject: String,
    description: String,
    block_type: Option<String>,
    inputs: Option<Value>,
    depends_on: Vec<String>,
    worktree: String,
    cwd: Option<String>,
    /// 子の時間上限（秒。`None` はブロック定義の `timeout:` に従う）
    timeout: Option<u32>,
    carried_over: bool,
    /// 起こす todo カードの run_id（U2。`Some` なら予約済みの run_id をそのまま子として使う）
    card_run_id: Option<i64>,
    /// 子の model（`None` はカード → ブロック定義 → settings.json の順）
    model: Option<String>,
    /// 子の thinking（`None` はカード → ブロック定義 → settings.json の順）
    thinking: Option<String>,
    /// 遠隔で動かす子の ssh Host 名（`None` は Mac で動かす。`Some` のとき `cwd` は遠隔のフォルダ）
    host: Option<String>,
}

/// todo のカード（`.agent-link/outputs/todo/<run_id>/task.md`）から取り出した値だけを並べたもの。
///
/// `task_create` の `card` 指定を検査し、子タスクの入力（task_text・block・説明）にする。
struct TaskCardSource {
    /// カードの `block:`
    block: String,
    /// カードの `description:`（空のときは `block` の値を使う）
    description: String,
    /// カード本文の `# task_text`
    task_text: String,
}

/// todo のカードを読んで、`task_create` の `card` 指定として使えるか調べる（U2）。
///
/// 検査の順は「所在 → 起動者 → 二重起動」の順で、先に理由の分かるものを出す。
fn load_task_card(card_run_id: i64) -> Result<TaskCardSource, String> {
    // `run_subtask` と同じ場所を読む（todo のカード → `.agent-link/outputs/todo/<run_id>/task.md`）
    let card = crate::orchestrator::read_todo_card(card_run_id)?;
    if !card.frontmatter.is_agent_executer() {
        return Err(format!(
            "Card {} is a human card (executer: human). To start it as a subtask run from an agent, create a card with executer: agent.",
            card_run_id
        ));
    }
    // 同じカードを 2 度の子にはしない（予約行は 1 度しか確定できない）
    if let Ok(store) = crate::common::open_db() {
        if let Ok(tasks) = store.list_unfinished_tasks_by_card(card_run_id) {
            if let Some(existing) = tasks.first().and_then(|t| t.id) {
                return Err(format!(
                    "Card {} was already started as task {}.",
                    card_run_id, existing
                ));
            }
        }
    }
    let block = card.frontmatter.block.trim().to_string();
    if block.is_empty() {
        return Err(format!(
            "Card {} has no block: in its task.md. Add block: <block_id>.",
            card_run_id
        ));
    }
    let task_text = crate::kanban::resolve_card_inputs(&card, &HashMap::new()).map_err(|e| {
        format!("Cannot get task_text of card {}: {}", card_run_id, e)
    })?;
    let task_text = task_text.get("task_text").cloned().unwrap_or_default();
    Ok(TaskCardSource {
        block,
        description: card.frontmatter.description.trim().to_string(),
        task_text,
    })
}

/// サブタスクをバッチ作成する（承認廃止・全タスクを常に承認済みで作成）。
pub fn task_create(
    tasks: &Value,
    round: Option<i64>,
    supersedes_batch_id: Option<&str>,
    run_id: Option<i64>,
) -> Result<Value, String> {
    let round = round.unwrap_or(1);

    let task_array = tasks
        .as_array()
        .filter(|a| !a.is_empty())
        .ok_or_else(|| "tasks must be an array with at least one task.".to_string())?;

    let mut seen_client_ids: HashSet<String> = HashSet::new();
    let mut normalized: Vec<NormalizedTask> = Vec::with_capacity(task_array.len());
    for item in task_array {
        let client_id = item
            .get("client_id")
            .and_then(|v| v.as_str())
            .ok_or("Each task needs client_id (a string).")?
            .to_string();
        if !seen_client_ids.insert(client_id.clone()) {
            return Err(format!(
                "Duplicate client_id '{}'. Give each task a unique client_id.",
                client_id
            ));
        }
        let subject_raw = item
            .get("subject")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        // U2: todo のカードを自分の子として起こす場合の指定（`card`）。
        // 検査は task_create の時点で行う（起動の時点で失敗すると親が理由を見つけにくいため）。
        let card_run_id = match item.get("card") {
            None | Some(Value::Null) => None,
            Some(v) => Some(v.as_i64().filter(|n| *n > 0).ok_or_else(|| {
                format!(
                    "client_id '{}': card must be an integer of 1 or more (the run_id of a todo card).",
                    client_id
                )
            })?),
        };
        let card_source = card_run_id.map(load_task_card).transpose()?;
        // subject は省略可（カード指定のときはカードの説明 → ブロックの順で補う）
        let subject = match (subject_raw, &card_source) {
            (Some(s), _) => s,
            (None, Some(c)) => {
                if c.description.is_empty() {
                    c.block.clone()
                } else {
                    c.description.clone()
                }
            }
            (None, None) => return Err("Each task needs subject (a string).".to_string()),
        };
        // P3: 入力を task_text 1 つに統一する。description / inputs は受け付けない。
        // U05: task.md と同じ項目名 `block` / `description` でも受け付ける（task.md 作成と同一化）。
        // U2: `card` 指定のときは task_text を書かない（カードの本文を使う）。
        let task_text_given = item
            .get("task_text")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        let task_text = match (&card_source, task_text_given) {
            (Some(_), Some(_)) => {
                return Err(
                    "Do not write task_text when card is given. The card body is used."
                        .to_string(),
                )
            }
            (Some(c), None) => c.task_text.clone(),
            (None, Some(t)) => t,
            (None, None) => {
                return Err("Each task needs task_text (a non-empty string).".to_string())
            }
        };
        // 内部表現では description と inputs.task_text に変換して既存の経路をそのまま使う
        let description = item
            .get("description")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| task_text.clone());
        let inputs = Some(json!({ "task_text": task_text }));
        // U2: `card` 指定のときはカードの `block:` を使う（書いていてカードと違うならエラー）。
        let block_given = item
            .get("block_type")
            .or_else(|| item.get("block"))
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        let block_type = match (&card_source, block_given) {
            (Some(c), Some(b)) => {
                if b != c.block {
                    return Err(format!(
                        "client_id '{}': the block of card {} is \"{}\". Make block_type match the block of the card.",
                        client_id,
                        card_run_id.unwrap_or(0),
                        c.block
                    ));
                }
                Some(c.block.clone())
            }
            (Some(c), None) => Some(c.block.clone()),
            (None, b) => b,
        };
        let depends_on = item
            .get("depends_on")
            .and_then(|v| v.as_array())
            .map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect())
            .unwrap_or_default();
        let worktree = item
            .get("worktree")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        // 作業フォルダ（相対パスはプロジェクトルート基準で解決。検証は後段で行う）
        let cwd_raw = item
            .get("cwd")
            .and_then(|v| v.as_str())
            .map(str::to_string);
        // 遠隔で動かす子の ssh Host 名（省略・null・空は Mac）。書式の検査は remote 側（U03）。
        // host があるとき cwd は遠隔のフォルダ。Mac では存在を確かめず、形だけ検査する。
        let host = item
            .get("host")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|h| !h.is_empty())
            .map(str::to_string);
        let cwd_raw = if host.is_some() {
            if !matches!(worktree.trim(), "" | "none") {
                return Err(format!(
                    "client_id '{}': worktree \"{}\" cannot be used with host. Leave it empty or use \"none\".",
                    client_id,
                    worktree.trim()
                ));
            }
            let Some(raw) = cwd_raw else {
                return Err(format!(
                    "client_id '{}': cwd is required when host is set (a folder on the remote machine).",
                    client_id
                ));
            };
            match crate::task_cwd::check_remote_cwd(&raw) {
                Ok(c) => Some(c),
                Err(msg) => return Err(format!("client_id '{}': {}", client_id, msg)),
            }
        } else {
            cwd_raw
        };
        // 子の時間上限（秒。1 以上の整数だけ受け付ける）
        let timeout = match item.get("timeout") {
            None | Some(Value::Null) => None,
            Some(v) => match v
                .as_u64()
                .filter(|n| *n >= 1 && *n <= u32::MAX as u64)
            {
                Some(n) => Some(n as u32),
                None => {
                    return Err(format!(
                        "client_id '{}': timeout must be an integer of 1 or more (seconds).",
                        client_id
                    ))
                }
            },
        };
        // `runner` は廃止した項目。古い入力に残っていても無視する（エラーにしない）。
        // 子の model（省略・空はブロック定義に従う）。cc が黙って捨てる名前は入口で断る。
        // 判定は cc_command::normalize_model と同じにする（inherit は親の既定に従う指定として通す）。
        let model = match item.get("model") {
            None | Some(Value::Null) => None,
            Some(v) => match v.as_str().map(str::trim) {
                Some("") => None,
                Some(m)
                    if m.eq_ignore_ascii_case("inherit")
                        || crate::cc_command::normalize_model(m).is_some() =>
                {
                    Some(m.to_string())
                }
                _ => {
                    return Err(format!(
                        "client_id '{}': model must be opus / sonnet / haiku (or claude-…).",
                        client_id
                    ))
                }
            },
        };
        // 子の thinking（省略・空はブロック定義に従う）。値は新 6 値だけ通す。
        let thinking = match item.get("thinking") {
            None | Some(Value::Null) => None,
            Some(v) => match v.as_str().map(|s| s.trim().to_ascii_lowercase()) {
                Some(t) if t.is_empty() => None,
                Some(t) if crate::validation::VALID_THINKING_LEVELS.contains(&t.as_str()) => {
                    Some(t)
                }
                _ => {
                    return Err(format!(
                        "client_id '{}': thinking must be one of none / low / medium / high / xhigh / max.",
                        client_id
                    ))
                }
            },
        };
        let carried_over = item
            .get("carried_over")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        normalized.push(NormalizedTask {
            client_id,
            subject,
            description,
            block_type,
            inputs,
            depends_on,
            worktree,
            cwd: cwd_raw,
            timeout,
            carried_over,
            card_run_id,
            model,
            thinking,
            host,
        });
    }

    // ── cwd （作業フォルダ）の解決・検証 ──
    // 絶対パス・~/ 始まり・プロジェクトルートからの相対パスを受け付け、絶対パスにする。
    // 存在しない・フォルダでない場合は validation_errors に載せ、タスクを作らない。
    let project_root = crate::project::project_root();
    let mut resolved_cwd: HashMap<String, String> = HashMap::new();
    let mut cwd_errors: Vec<Value> = Vec::new();
    for t in &normalized {
        if let Some(ref raw) = t.cwd {
            // host 付きの cwd は検査済みの遠隔パス。Mac では解決しない
            if t.host.is_some() {
                resolved_cwd.insert(t.client_id.clone(), raw.clone());
                continue;
            }
            match crate::task_cwd::resolve_task_cwd(raw, &project_root) {
                Ok(path) => {
                    resolved_cwd.insert(t.client_id.clone(), path.to_string_lossy().to_string());
                }
                Err(msg) => {
                    cwd_errors.push(json!({
                        "client_id": t.client_id,
                        "subject": t.subject,
                        "block_type": t.block_type,
                        "field": "cwd",
                        "kind": "invalid",
                        "message": format!(
                            "Cannot resolve cwd \"{}\": {}. Give an existing folder as an absolute path, a path starting with ~/, or a path relative to the project root.",
                            raw, msg
                        ),
                    }));
                }
            }
        }
    }

    // ── ブロック定義の存在確認（入力仕様は task_text 1 つに統一されたため存在確認だけ） ──
    let defs = collect_block_definitions();
    let mut validation_errors: Vec<Value> = Vec::new();

    for t in &normalized {
        let Some(ref block_type) = t.block_type else {
            continue;
        };
        if !defs.iter().any(|(_, c)| c.block_id() == block_type) {
            validation_errors.push(json!({
                "client_id": t.client_id,
                "subject": t.subject,
                "block_type": block_type,
                "field": "<block_type>",
                "kind": "missing",
                "message": format!(
                    "Could not get the block definition for block_type \"{}\". Check the ID with `agent-link def list` (give the id, not the display name).",
                    block_type
                ),
            }));
        }
    }

    // cwd の解決エラーも validation_errors に統合する
    validation_errors.extend(cwd_errors);

    if !validation_errors.is_empty() {
        return Ok(json!({
            "status": "failed",
            "error": "Input validation of task_create failed. Fix block_type as the messages above say.",
            "validation_errors": validation_errors,
        }));
    }

    let run_id = require_run_id(run_id)?;
    let batch_id = format!(
        "batch_{}_r{}_{}",
        chrono::Utc::now().format("%Y%m%d"),
        round,
        random_base36_4()
    );

    let tasks_payload: Vec<Value> = normalized
        .iter()
        .map(|t| {
            json!({
                "client_id": t.client_id,
                "subject": t.subject,
                "description": t.description,
                "block_type": t.block_type,
                "inputs": t.inputs,
                "depends_on": t.depends_on,
                "worktree": t.worktree,
                "cwd": resolved_cwd.get(&t.client_id),
                "host": t.host,
                "timeout": t.timeout,
                "model": t.model,
                "thinking": t.thinking,
                "carried_over": t.carried_over,
                "card": t.card_run_id,
                "approval": { "status": "approved", "comment": null },
            })
        })
        .collect();

    let payload = json!({
        "parent_run_id": run_id,
        "batch_id": batch_id,
        "round": round,
        "supersedes_batch_id": supersedes_batch_id,
        "tasks": tasks_payload,
    });
    write_notification("task_created", payload)?;

    // 作られたタスクの task_id を client_id ごとに返す（デーモンが行を作るまで待つ）
    let task_ids = wait_task_ids(&batch_id, &normalized);
    let result_tasks: Vec<Value> = normalized
        .iter()
        .map(|t| {
            json!({
                "client_id": t.client_id,
                "status": "approved",
                "task_id": task_ids.get(&t.client_id).copied(),
            })
        })
        .collect();

    Ok(json!({
        "batch_id": batch_id,
        "round": round,
        "supersedes_batch_id": supersedes_batch_id,
        "status": "approved",
        "tasks": result_tasks,
        "hint": "`agent-link task execute --batch <batch_id>` runs them as a batch. If `task_id` is null, check `server_id` with `agent-link task list`.",
    }))
}

/// `task_create` が書いた通知をデーモンが行にするのを待ち、client_id → task_id を集める。
///
/// デーモンが止まっていれば待たずに空を返す（task_id は null）。動いていても
/// 5 秒で揃わなければ、そのときまでに出来た分を返す。
/// 二重挿入を避けるため、ここでは行を作らない（作るのはデーモンだけ）。
fn wait_task_ids(batch_id: &str, normalized: &[NormalizedTask]) -> HashMap<String, i64> {
    if !crate::daemon::is_running() {
        return HashMap::new();
    }
    let Ok(store) = crate::common::open_db() else {
        return HashMap::new();
    };
    collect_task_ids(&store, batch_id, normalized.len(), std::time::Duration::from_secs(5))
}

/// バッチの行から client_id → task_id を集める。`expected` 件そろうか `wait` を過ぎたら返す。
fn collect_task_ids(
    store: &crate::db::SaiStore,
    batch_id: &str,
    expected: usize,
    wait: std::time::Duration,
) -> HashMap<String, i64> {
    let deadline = std::time::Instant::now() + wait;
    loop {
        let found: HashMap<String, i64> = store
            .list_tasks_by_batch(batch_id)
            .unwrap_or_default()
            .into_iter()
            .filter_map(|t| Some((t.client_id?, t.id?)))
            .collect();
        if found.len() >= expected || std::time::Instant::now() >= deadline {
            return found;
        }
        std::thread::sleep(std::time::Duration::from_millis(250));
    }
}

// ============================================================================
// 3.4 task_list
// ============================================================================

/// 親実行 ID のサブタスク一覧を返す。
pub fn task_list(
    status: Option<&str>,
    sort: Option<&str>,
    run_id: Option<i64>,
) -> Result<Value, String> {
    let run_id = require_run_id(run_id)?;
    let tasks = tasks_json(run_id, status.or(Some("all")), sort)?;
    Ok(json!({ "count": tasks.len(), "tasks": tasks }))
}

// ============================================================================
// 3.5 task_execute
// ============================================================================

/// `.agent-link/settings.json` の `max_concurrent` を読む（無ければ既定 16）。
fn max_concurrent_limit() -> u64 {
    let path = crate::project::sai_path(".agent-link/settings.json");
    let Ok(content) = std::fs::read_to_string(&path) else {
        return 16;
    };
    let Ok(parsed) = serde_json::from_str::<Value>(&content) else {
        return 16;
    };
    parsed
        .get("max_concurrent")
        .and_then(|v| v.as_u64())
        .filter(|&n| n > 0)
        .unwrap_or(16)
}

/// 承認済みサブタスクを実行する（非同期。即座に戻る）。
pub fn task_execute(
    batch_id: Option<&str>,
    task_id: Option<i64>,
    run_id: Option<i64>,
) -> Result<Value, String> {
    if batch_id.is_none() && task_id.is_none() {
        return Err("Give either batch_id or task_id.".to_string());
    }
    let run_id = require_run_id(run_id)?;

    if let Some(bid) = batch_id {
        let limit = max_concurrent_limit();
        let approved = tasks_json(run_id, Some("approved"), None)?;
        let count = approved
            .iter()
            .filter(|t| t.get("batch_id").and_then(|v| v.as_str()) == Some(bid))
            .count() as u64;

        if count > limit {
            return Ok(json!({
                "error": "batch_limit_exceeded",
                "batch_id": bid,
                "count": count,
                "limit": limit,
                "required_finalize": true,
                "message": format!(
                    "Batch {} has {} approved tasks, over the batch limit of {}. Split the tasks and create them again, or finalize unneeded tasks and run again.",
                    bid, count, limit
                ),
            }));
        }
        if count == 0 {
            return Ok(json!({
                "batch_id": bid,
                "status": "no_approved_tasks",
                "message": format!(
                    "Batch {} has no approved tasks. Create tasks with task_create and run again.",
                    bid
                ),
            }));
        }

        write_notification(
            "task_execute_request",
            json!({ "parent_run_id": run_id, "batch_id": bid }),
        )?;
    } else if let Some(tid) = task_id {
        write_notification(
            "task_execute_request",
            json!({ "parent_run_id": run_id, "task_id": tid }),
        )?;
    }

    Ok(json!({
        "batch_id": batch_id,
        "status": "queued",
        "run_batch_created": true,
        "message": "Sent to the execution queue (asynchronous). Check progress with `agent-link task show <ID>` or `agent-link task wait <ID>`.",
    }))
}

// ============================================================================
// 3.6 task_output
// ============================================================================

/// `results/` から `-task_<task_id>.json` で終わるファイルを 1 件探す。
///
/// `task_stalled`（アラート）が同時に複数存在しても、親が古いアラートを繰り返し
/// 受け取らないように、完了（`task_stalled` 以外）を優先する。完了が無い場合のみ
/// ファイル名がいちばん大きい（新しい）stalled を返す（U2）。
fn find_result_file(dir: &Path, task_id: i64) -> Option<(PathBuf, Value)> {
    let entries = std::fs::read_dir(dir).ok()?;
    let suffix = format!("-task_{}.json", task_id);
    let mut latest_stalled: Option<(PathBuf, Value)> = None;
    for entry in entries.flatten() {
        let path = entry.path();
        let name = match path.file_name() {
            Some(n) => n.to_string_lossy().to_string(),
            None => continue,
        };
        if !name.ends_with(&suffix) {
            continue;
        }
        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => continue,
        };
        let value: Value = match serde_json::from_str(&content) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let is_stalled = value.get("type").and_then(|v| v.as_str()) == Some("task_stalled");
        if !is_stalled {
            return Some((path, value));
        }
        let newer = match &latest_stalled {
            Some((prev_path, _)) => path.file_name() > prev_path.file_name(),
            None => true,
        };
        if newer {
            latest_stalled = Some((path, value));
        }
    }
    latest_stalled
}

/// タスクの実行 run フォルダにある `finalize-report.md`（task_report の報告本文）を読む。
/// 実行 ID が未確定・run フォルダが無い・ファイルが無い場合は `None`。
fn task_report_text(task_id: i64) -> Option<String> {
    let store = crate::common::open_db().ok()?;
    let task = store.get_task_by_id(task_id).ok()?;
    let exec_id = task.execution_id?;
    let (_, dir) = crate::run_dir::find_run_dir(exec_id)?;
    std::fs::read_to_string(dir.join(REPORT_FILE)).ok()
}

/// 結果通知 JSON を agent_api の戻り値形式に整形する（`task_completed` / `task_stalled` / `task_failed`）。
fn build_task_output_result(task_id: i64, value: &Value) -> Result<Value, String> {
    let notification_type = value.get("type").and_then(|v| v.as_str()).unwrap_or("");

    match notification_type {
        "task_completed" => {
            let parent_run_id = value.get("parent_run_id").and_then(|v| v.as_i64()).unwrap_or(0);
            let waggle_token_strs: Vec<String> = value
                .get("waggle_tokens")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|t| t.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default();

            let db_path = crate::project::db_path();
            if let Some(parent) = db_path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let integrator = crate::waggle_integration::WaggleIntegrator::open(&db_path).ok();

            // P6-3: 成果物の全文ではなく「waggle:<tok>: <description>」の一覧を返す。
            let outputs: Vec<String> = waggle_token_strs
                .iter()
                .map(|token| {
                    let description = integrator
                        .as_ref()
                        .and_then(|i| i.resolve_token(token).ok())
                        .and_then(|v| v.manifest.meta.description.clone())
                        .filter(|d| !d.is_empty());
                    match description {
                        Some(d) => format!("waggle:{}: {}", token, d),
                        None => format!("waggle:{}: (no description)", token),
                    }
                })
                .collect();

            if parent_run_id > 0 && !waggle_token_strs.is_empty() {
                write_notification(
                    "task_output_used",
                    json!({
                        "parent_run_id": parent_run_id,
                        "task_id": task_id,
                        "waggle_tokens": waggle_token_strs,
                    }),
                )?;
            }

            // P6-3: 短い報告（report）と成果物トークンの一覧（outputs）だけを返す。
            // resolved_content / commit_id / elapsed_ms / session_stats は返さない。
            // hint は B7: 子の run は完了しただけでは閉じない（finalized は親が付ける）。
            Ok(json!({
                "task_id": task_id,
                "status": "completed",
                "round": value.get("round").and_then(|v| v.as_i64()).unwrap_or(1),
                "report": task_report_text(task_id),
                "outputs": outputs,
                "hint": "The subtask is done. Read report and outputs, check the deliverables, then close it with agent-link task finalize <ID>. If something is missing, have it fix that with agent-link task send <ID> --message <text>. If you do not close it, the pane and the running folder remain.",
            }))
        }
        "task_stalled" => {
            let reason = value.get("reason").and_then(|v| v.as_str()).unwrap_or("");
            // 質問（reason = "question"）のときは question_id も返し、案内を別にします
            let question_id = if reason == "question" {
                value.get("question_id").and_then(|v| v.as_str())
            } else {
                None
            };
            // long_running / timeout は子がまだ working なので、止めずに画面を見る案内にする
            let hint = if reason == "question" {
                "The subtask asked a question and is waiting for the answer. The question is also shown on the Web home. Read the question with question_id and follow the description of your block to answer. If a Human answers first on the Web, that answer reaches the subtask."
            } else if reason == "long_running" || reason == "timeout" {
                "The subtask is still working. Check the screen with agent-link task capture <ID>. If it is progressing, keep waiting with `agent-link task wait <ID>`. If it is stuck, stop it with `agent-link task interrupt <ID>` and have it investigate the cause with `agent-link task send <ID> --message <text>`. An alert for the same reason keeps arriving while it keeps working."
            } else {
                "The subtask is stopped. Check the screen with agent-link task capture <ID> and send an instruction with agent-link task send <ID> --message <text>. If it is working or blocked, stop it first with agent-link task interrupt <ID>."
            };
            let mut out = json!({
                "task_id": task_id,
                "status": "stalled",
                "reason": value.get("reason").cloned().unwrap_or(Value::Null),
                "message": value.get("message").cloned().unwrap_or(Value::Null),
                "hint": hint,
            });
            // question_id は質問の通知にだけ付ける（他の理由では付けない）
            if let Some(qid) = question_id {
                out["question_id"] = Value::String(qid.to_string());
            }
            Ok(out)
        }
        _ => Ok(json!({
            "task_id": task_id,
            "status": "failed",
            "error": value.get("error").cloned().unwrap_or(Value::Null),
        })),
    }
}

/// `task_output` が 1 回の呼び出しで待つ最大時間（ミリ秒）。
///
/// Claude Code は MCP 呼び出しが 120 秒を超えるとバックグラウンドに回すが、
/// サーバ側の待ちは止まらない。既定値が 120 秒を超えると待ちが重なり詰まるため、
/// 必ず 120 秒より短くする。
pub const TASK_OUTPUT_MAX_WAIT_MS: u64 = 90_000;

/// 待ち時間を上限に切り詰める。`None` は上限と同じ。`0`（即時参照）はそのまま。
pub fn clamp_wait_ms(requested: Option<u64>) -> u64 {
    requested.map_or(TASK_OUTPUT_MAX_WAIT_MS, |t| t.min(TASK_OUTPUT_MAX_WAIT_MS))
}

/// 1 回の待ちは最大何秒か（`TASK_OUTPUT_MAX_WAIT_MS` を秒にしたもの）。文言に使う。
fn max_wait_seconds() -> u64 {
    TASK_OUTPUT_MAX_WAIT_MS / 1000
}

/// 残り（返さなかった子）の `task_id` を `pending` として足す。
fn add_pending(mut result: Value, task_ids: &[i64], done: i64) -> Value {
    let pending: Vec<i64> = task_ids.iter().copied().filter(|id| *id != done).collect();
    if let Some(map) = result.as_object_mut() {
        map.insert("pending".to_string(), json!(pending));
    }
    result
}

/// タスクの実行結果を取得する（未完了なら待機、または即時参照）。
///
/// 単体だけを見る入口。複数を待つ場合は [`task_output_ids`]（`task_ids` 版）を使う。
pub fn task_output(task_id: i64, timeout_ms: Option<u64>) -> Result<Value, String> {
    task_output_ids(&[task_id], timeout_ms, &AtomicBool::new(false), false)
}

/// 複数のタスクの実行結果をまとめて取得する（未完了なら待機、または即時参照）。
///
/// - `task_ids`: 待つ子の `server_id`（先頭から順に調べる）。
/// - `timeout_ms`: 待ち時間。`None` と上限を超えた値は [`TASK_OUTPUT_MAX_WAIT_MS`] に切られる。`0` は待機なし。
/// - `cancel`: 外から立てると待ちが止まる（MCP の `notifications/cancelled` など）。
/// - `with_pending`: `true` のときは「1 件だけ返して残りを `pending` で返す」形になる。
///
/// 見つかった結果ファイルは**消す**（返した子は次の呼び出しで待たないため）。
/// 取り消しで抜けたときは結果ファイルに触れないので、あとから読み直せる。
pub fn task_output_ids(
    task_ids: &[i64],
    timeout_ms: Option<u64>,
    cancel: &AtomicBool,
    with_pending: bool,
) -> Result<Value, String> {
    if task_ids.is_empty() {
        return Err("task_ids needs at least one ID.".to_string());
    }
    let wait_ms = clamp_wait_ms(timeout_ms);
    // 要求された時間が上限を超えていたとき（timeout_ms を省略した時も含む）
    let capped = timeout_ms.map_or(true, |t| t > TASK_OUTPUT_MAX_WAIT_MS);
    let dir = crate::common::notify_results_dir();

    if wait_ms == 0 {
        // 即時参照: 見つかっても結果ファイルは削除しない（移植元 readResult と同じ）
        for &id in task_ids {
            if let Some((_, value)) = find_result_file(&dir, id) {
                let result = build_task_output_result(id, &value)?;
                return Ok(if with_pending { add_pending(result, task_ids, id) } else { result });
            }
        }
        return Ok(in_progress_result(task_ids, with_pending, &format!(
            "The task is not done yet. One wait lasts at most {} seconds. Set timeout_ms, or run `agent-link task wait <ID>` later.",
            max_wait_seconds()
        )));
    }

    let started = std::time::Instant::now();
    // 結果を探す間隔（results/ を読み直すのはこの間隔）
    let scan_interval = std::time::Duration::from_millis(500);
    // 取り消しと時間切れを見る間隔（短いほど取り消しが早く効く）
    let check_interval = std::time::Duration::from_millis(50);
    let mut next_scan = started;
    loop {
        // 取り消しなら結果ファイルに触れずに抜ける（後で読み直せるように）
        if cancel.load(Ordering::Relaxed) {
            return Ok(in_progress_result(task_ids, with_pending, &format!(
                "The wait was cancelled (results are kept). One wait lasts at most {} seconds. Call again.",
                max_wait_seconds()
            )));
        }
        let now = std::time::Instant::now();
        if now >= next_scan {
            for &id in task_ids {
                if let Some((path, value)) = find_result_file(&dir, id) {
                    let result = build_task_output_result(id, &value)?;
                    let _ = std::fs::remove_file(&path);
                    return Ok(if with_pending { add_pending(result, task_ids, id) } else { result });
                }
            }
            next_scan = now + scan_interval;
        }
        if started.elapsed().as_millis() as u64 >= wait_ms {
            break;
        }
        std::thread::sleep(check_interval);
    }

    // 時間切れ
    Ok(timeout_result(task_ids, wait_ms, capped, with_pending))
}

/// 時間切れの戻り値を組み立てる。
///
/// - 単体（`with_pending == false`）で上限に達したときは `in_progress`（もう一回呼べばよい）。
/// - 単体で明示した時間が切れたときは、後方互換のため今まで通り `timeout`。
/// - 複数待ちのときは `task_id` を付けず、`pending`（全件）を返す。
fn timeout_result(task_ids: &[i64], wait_ms: u64, capped: bool, with_pending: bool) -> Value {
    if with_pending {
        return json!({
            "status": "in_progress",
            "pending": task_ids,
            "message": format!(
                "No subtask produced a result within {}ms. One wait lasts at most {} seconds. Call again.",
                wait_ms,
                max_wait_seconds()
            ),
        });
    }
    if capped {
        json!({
            "task_id": task_ids[0],
            "status": "in_progress",
            "message": format!(
                "The task did not finish within {}ms. One wait lasts at most {} seconds. Call again.",
                wait_ms,
                max_wait_seconds()
            ),
        })
    } else {
        json!({
            "task_id": task_ids[0],
            "status": "timeout",
            "error": format!(
                "Timeout: the task did not finish within {}ms. One wait lasts at most {} seconds. Call again.",
                wait_ms,
                max_wait_seconds()
            ),
        })
    }
}

/// 「まだ結果が無い」場合の戻り値を組み立てる。
///
/// 複数待ちのときは `task_id` を付けず、`pending`（全件）と `message` だけ返す。
fn in_progress_result(task_ids: &[i64], with_pending: bool, message: &str) -> Value {
    if with_pending {
        json!({
            "status": "in_progress",
            "pending": task_ids,
            "message": message,
        })
    } else {
        json!({
            "task_id": task_ids[0],
            "status": "in_progress",
            "message": message,
        })
    }
}

// ============================================================================
// 3.6b task_report（サブブロックの完了報告）
// ============================================================================

/// サブブロックの完了報告。`report` を run フォルダの `finalize-report.md` に保存し、
/// 完了（`finalize_request`）を要求する。
///
/// 保存した報告は waggle トークン化しない（通知を書かない）。親は `task_output` の
/// `report` として読む。このツールはサブブロック専用で、メインブロックでは `Err` になる
/// （メインブロックの完了は `agent-link run finalize` を使う）。
pub fn task_report(report: &str, run_id: Option<i64>) -> Result<Value, String> {
    let report = report.trim();
    if report.is_empty() {
        return Err("report (the body of the completion report) is required.".to_string());
    }
    let run_id = require_run_id(run_id)?;

    let store =
        crate::common::open_db().map_err(|e| format!("Failed to open the database: {}", e))?;
    let exec = store
        .get_execution_by_id(run_id)
        .map_err(|e| format!("Failed to get run id = {}: {}", run_id, e))?;
    if exec.parent_run_id.is_none() {
        return Err(
            "task report is for subtasks only. To finish a main block, use agent-link run finalize."
                .to_string(),
        );
    }

    // 報告本文は run フォルダ直下の finalize-report.md に保存する。
    // この保存は waggle トークン化しない（file_written 通知を書かない）。
    write_report_file(run_id, report)?;

    crate::daemon::write_finalize_request(run_id)
        .map_err(|e| format!("Failed to write finalize_request: {}", e))?;

    Ok(json!({
        "status": "reported",
        "run_id": run_id,
        "message": "Sent the completion report.",
    }))
}

// ============================================================================
// 3.6c task_question（サブブロックが親に質問を通知する）
// ============================================================================

/// 質問の文を 1 行にして 500 文字で切る（通知の message に使うため）。
fn one_line_short(text: &str) -> String {
    let flat: String = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    if flat.chars().count() <= 500 {
        flat
    } else {
        flat.chars().take(500).collect::<String>() + "…"
    }
}

/// サブブロックの質問を親へ知らせる（`agent task-question` の実体）。
///
/// 子の run（`parent_run_id` を持つ実行）からのみ呼べる。質問の中身の全文は
/// `apps/web` の DB にあるので、ここでは ID と見出しだけを送る。
pub fn task_question(run_id: Option<i64>, question_id: &str, text: &str) -> Result<Value, String> {
    let run_id = require_run_id(run_id)?;
    let qid = question_id.trim();
    if qid.is_empty() {
        return Err("question_id (the question ID) is required.".to_string());
    }
    let store =
        crate::common::open_db().map_err(|e| format!("Failed to open the database: {}", e))?;
    let exec = store
        .get_execution_by_id(run_id)
        .map_err(|e| format!("Failed to get run id = {}: {}", run_id, e))?;
    let parent_run_id = exec
        .parent_run_id
        .ok_or_else(|| "task-question is for subtask runs only.".to_string())?;

    // この実行がどのタスクの実行かを引き直す（親の task_output はタスク ID で受ける）
    let task_id = store
        .list_tasks_by_parent(parent_run_id)
        .map_err(|e| format!("Failed to list subtasks: {}", e))?
        .into_iter()
        .find(|t| t.execution_id == Some(run_id))
        .and_then(|t| t.id)
        .ok_or_else(|| format!("No task found for run_id={}.", run_id))?;

    let message = format!("Question {}: {}", qid, one_line_short(text));
    crate::task_manager::write_stalled_notification(
        task_id,
        Some(parent_run_id),
        "question",
        &message,
        Some(qid),
    )?;

    Ok(json!({
        "status": "notified",
        "task_id": task_id,
        "parent_run_id": parent_run_id,
        "question_id": qid,
    }))
}

// ============================================================================
// 3.7 task_manage（delete / finalize / send / requeue / interrupt）
// ============================================================================

/// タスクの最新実行の pane を解決する（`task_capture` / `task_manage` 共用）。
///
/// 返り値は `(タスク, 実行レコード, pane_id)`。
/// エラー文は `task_capture` の元の文言をそのまま使っている。
fn task_pane(task_id: i64) -> Result<(Task, ExecutionHistory, String), String> {
    let store = Arc::new(
        crate::common::open_db().map_err(|e| format!("Failed to open the database: {}", e))?,
    );
    let manager = TaskManager::new(Arc::clone(&store));
    let task = manager
        .get_task(task_id)
        .map_err(|e| format!("Failed to get the task: {}", e))?;

    let exec_id = task
        .execution_id
        .ok_or_else(|| format!("Task {} has not been run yet.", task_id))?;
    let exec = store
        .get_execution_by_id(exec_id)
        .map_err(|e| format!("Failed to get the run record: {}", e))?;
    let pane_id = exec
        .herdr_pane_id
        .clone()
        .ok_or_else(|| format!("Run {} has no pane.", exec_id))?;

    Ok((task, exec, pane_id))
}

// ============================================================================
// 3.8 run_send / run_interrupt（実行単位の送信・中断。共有契約 C2）
// ============================================================================

/// 実行の送り先（擬似 pane）を解決する。
///
/// `herdr_pane_id` → run セッション dir の `herdr_pane_id` の順で探す。
/// 実行が無ければ「実行 <id> が見つかりません」（CLI は終了コード 3）、
/// 送り先が無ければ「実行 <id> の送り先が見つかりません」（同じく 3）。
fn run_pane(run_id: i64) -> Result<(ExecutionHistory, String), String> {
    let store = Arc::new(
        crate::common::open_db().map_err(|e| format!("Failed to open the database: {}", e))?,
    );
    let exec = match store.get_execution_by_id(run_id) {
        Ok(e) => e,
        Err(crate::db::models::DbError::NotFound(_)) => {
            return Err(format!("Run {} not found.", run_id));
        }
        Err(e) => return Err(format!("Failed to get the run record: {}", e)),
    };
    let pane_id = exec
        .herdr_pane_id
        .clone()
        .or_else(|| crate::phone_export::pane_id_from_session_dir(&exec))
        .ok_or_else(|| format!("No destination found for run {}.", run_id))?;
    Ok((exec, pane_id))
}

/// `agent-link run send <run_id>`: 実行中の run に続きの文を送る（共有契約 C2）。
///
/// 擬似 pane に `send_text` → 確定キーを送り、`working` になるのを待つ。
/// 戻り値は `{"run_id", "pane_id", "status": "sent" | "unconfirmed", "message"?}`。
/// `unconfirmed` は文と確定キーを送ったが working にならなかった（届いたか分からない）。
/// 失敗にすると利用者が送り直して二重に届くので、成功（終了コード 0）で返す。
/// 終了している・質問待ちのときは `Err`（CLI は終了コード 4）。
pub fn run_send(run_id: i64, text: &str) -> Result<Value, String> {
    let (_exec, pane_id) = run_pane(run_id)?;
    let herdr = crate::herdr::HerdrClient::new();
    let info = herdr
        .get_pane_status(&pane_id)
        .map_err(|_| format!("Run {} has already ended.", run_id))?;
    if info.agent_status == "blocked" {
        return Err(format!(
            "Run {} is already waiting for a question. Send the answer from the question card.",
            run_id
        ));
    }
    let submit_key = crate::cc_command::CC_SUBMIT_KEY;
    match crate::orchestrator::submit_prompt_verified(&herdr, &pane_id, text, submit_key, None) {
        Ok(()) => Ok(json!({ "run_id": run_id, "pane_id": pane_id, "status": "sent" })),
        Err(e @ crate::orchestrator::OrchestratorError::PromptUnconfirmed { .. }) => Ok(json!({
            "run_id": run_id,
            "pane_id": pane_id,
            "status": "unconfirmed",
            "message": e.to_string(),
        })),
        Err(e) => Err(format!("Failed to send to the pane: {}", e)),
    }
}

/// `agent-link run interrupt <run_id>`: 実行中の run を中断する（共有契約 C2）。
///
/// 擬似 pane に `esc` を送り、`working` でなくなるまで最大 5 秒待つ。
/// 戻り値は `{"run_id", "status": "interrupted" | "still_working", "agent_status"}`。
/// 終了しているときは `Err`（CLI は終了コード 4）。
pub fn run_interrupt(run_id: i64) -> Result<Value, String> {
    let (_exec, pane_id) = run_pane(run_id)?;
    let herdr = crate::herdr::HerdrClient::new();
    let info = herdr
        .get_pane_status(&pane_id)
        .map_err(|_| format!("Run {} has already ended.", run_id))?;
    let mut agent_status = info.agent_status;
    herdr
        .send_keys(&pane_id, "esc")
        .map_err(|e| format!("Failed to send esc: {}", e))?;

    const POLL_MS: u64 = 250;
    const WAIT_MS: u64 = 5_000;
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(WAIT_MS);
    while agent_status == "working" && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(POLL_MS));
        agent_status = herdr
            .get_pane_status(&pane_id)
            .map(|i| i.agent_status)
            .unwrap_or_else(|_| "unknown".to_string());
    }
    let status = if agent_status == "working" {
        "still_working"
    } else {
        "interrupted"
    };
    Ok(json!({
        "run_id": run_id,
        "status": status,
        "agent_status": agent_status,
    }))
}

// ============================================================================
// 3.8b run_wait（自分の子でない run の完了・失敗・停止を待つ）
// ============================================================================

/// 複数の run を待つ。完了・失敗・停止のどれか 1 件を返し、残りを `pending` で返す。
///
/// - 1 回の待ちは最大 [`TASK_OUTPUT_MAX_WAIT_MS`]（`timeout_ms` は上限に切られる。`0` は 1 回だけ調べる）。
/// - `stalled` は pane が idle のまま [`stall_idle_ms`] を超えたときだけ返す（質問は見ない）。
/// - 戻り値の `pending` は、この呼び出しで返さなかった残りの run。
///   返した run は呼び出し側が次から外す（engine は覚えない）。
/// - 存在しない run は `Err`（「見つかりません」。CLI は終了コード 3）。
pub fn run_wait(
    run_ids: &[i64],
    timeout_ms: Option<u64>,
    cancel: &AtomicBool,
) -> Result<Value, String> {
    if run_ids.is_empty() {
        return Err("Give at least one run ID to run wait.".to_string());
    }
    let wait_ms = clamp_wait_ms(timeout_ms);
    let started = std::time::Instant::now();
    // DB と状態ファイルを読む間隔と、取り消しを見る間隔
    let scan_interval = std::time::Duration::from_millis(500);
    let check_interval = std::time::Duration::from_millis(50);
    let mut next_scan = started;
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Ok(run_wait_in_progress(
                run_ids,
                "The wait was cancelled. Call again.",
            ));
        }
        let now = std::time::Instant::now();
        if now >= next_scan {
            if let Some(hit) = run_wait_scan(run_ids)? {
                return Ok(hit);
            }
            next_scan = now + scan_interval;
        }
        if started.elapsed().as_millis() as u64 >= wait_ms {
            break;
        }
        std::thread::sleep(check_interval);
    }
    Ok(run_wait_in_progress(run_ids, &format!(
        "No run produced a result within {}ms. One wait lasts at most {} seconds. Call again.",
        wait_ms,
        max_wait_seconds()
    )))
}

/// 1 回の走査。`run_ids` の順に調べ、最初に止まった（完了・失敗・停止の）run を返す。
fn run_wait_scan(run_ids: &[i64]) -> Result<Option<Value>, String> {
    let store =
        crate::common::open_db().map_err(|e| format!("Failed to open the database: {}", e))?;
    let others = |run_id: i64| -> Vec<i64> {
        run_ids.iter().copied().filter(|&id| id != run_id).collect()
    };
    for &run_id in run_ids {
        let exec = match store.get_execution_by_id(run_id) {
            Ok(e) => e,
            Err(crate::db::models::DbError::NotFound(_)) => {
                return Err(format!("Run {} not found.", run_id));
            }
            Err(e) => return Err(format!("Failed to get the run record: {}", e)),
        };
        let block = store
            .get_block_by_id(exec.block_id)
            .map(|b| b.name)
            .unwrap_or_default();
        match exec.status.as_str() {
            "completed" => {
                return Ok(Some(json!({
                    "status": "completed",
                    "run_id": run_id,
                    "block": block,
                    "pending": others(run_id),
                })));
            }
            "failed" => {
                let error = exec
                    .error_message
                    .filter(|e| !e.is_empty())
                    .unwrap_or_else(|| "(no failure reason recorded)".to_string());
                return Ok(Some(json!({
                    "status": "failed",
                    "run_id": run_id,
                    "block": block,
                    "error": error,
                    "pending": others(run_id),
                })));
            }
            _ => {
                if let Some(secs) = idle_seconds(run_id) {
                    return Ok(Some(json!({
                        "status": "stalled",
                        "run_id": run_id,
                        "block": block,
                        "reason": "idle",
                        "message": format!(
                            "Run {} has been idle for {} seconds or more. It may be waiting for an answer or an instruction. Check the questions with agent-link-web questions.",
                            run_id, secs
                        ),
                        "pending": others(run_id),
                    })));
                }
            }
        }
    }
    Ok(None)
}

/// run の pane が idle のまま止まっていれば、その秒数を返す（止まっていないときは `None`）。
///
/// 止まった時刻は `state.json` の `updated_at`（idle になった時に書かれる）から読む。
/// 1 回の呼び出しをまたいで時間を覚えなくてよいように、ここでは状態ファイルだけを見る。
fn idle_seconds(run_id: i64) -> Option<i64> {
    let (_, pane_id) = run_pane(run_id).ok()?;
    let info = crate::headless::status(&pane_id).ok()?;
    if info.agent_status != "idle" {
        return None;
    }
    let state = crate::headless::read_state(&crate::headless::pane_dir(&pane_id))?;
    let since = chrono::DateTime::parse_from_rfc3339(&state.updated_at).ok()?;
    let idle_ms = chrono::Utc::now()
        .signed_duration_since(since.with_timezone(&chrono::Utc))
        .num_milliseconds();
    (idle_ms >= crate::orchestrator::stall_idle_ms() as i64).then_some(idle_ms / 1000)
}

/// 完了も停止もまだのときの戻り値（`pending` は全件）。
fn run_wait_in_progress(run_ids: &[i64], message: &str) -> Value {
    json!({
        "status": "in_progress",
        "pending": run_ids,
        "message": message,
    })
}

/// `send` の行き先。
#[derive(Debug, PartialEq, Eq)]
enum SendRoute {
    /// 作業中タスクの pane に直接送る。1 回目の実行は `task_report` まで待機が続くため、
    /// 送信後も子は作業を続け、最後に `task_report` で完了する（デーモンを経由しない）。
    DirectToPane,
    /// 従来どおり `task_manage_request` 通知でデーモンに任せる
    /// （completed タスクへのターン送信や、未実行タスクへの新ラウンドなど）。
    Daemon,
}

/// `task_status` と pane の `agent_status` から `send` の行き先を決める。
///
/// `agent_status` は `in_progress` のときだけ使う（取得できなければ `None`）。
/// herdr には触れない純粋関数なので、表の全部を単体テストできる。
fn send_route(
    task_id: i64,
    task_status: &str,
    agent_status: Option<&str>,
) -> Result<SendRoute, String> {
    match task_status {
        "in_progress" => match agent_status {
            Some("idle") | Some("done") => Ok(SendRoute::DirectToPane),
            Some("working") => Err(format!(
                "Task {} is working. To stop it and change the instruction, call agent-link task interrupt <ID> first. To wait for it to finish, use `agent-link task wait <ID>`.",
                task_id
            )),
            Some("blocked") => Err(format!(
                "Task {} is waiting for the user's answer to a question. Wait with `agent-link task wait <ID>` until the user answers it on the Web.",
                task_id
            )),
            other => {
                let label = other.unwrap_or("unknown state");
                Err(format!(
                    "Task {} is stopped at {}. Check the screen with agent-link task capture <ID>, return it to input-waiting with agent-link task interrupt <ID>, and then send.",
                    task_id, label
                ))
            }
        },
        "finalized" | "deleted" => Err(format!(
            "Task {} is {}. You cannot send to a finalized or deleted task. To start over, use requeue or task_create.",
            task_id, task_status
        )),
        // completed / approved / failed などは今までの動作（デーモン経由）。
        _ => Ok(SendRoute::Daemon),
    }
}

/// サブブロック（タスク）を操作する。
///
/// - `delete` … 削除（実行中なら pane を閉じて強制終了 = 人間の ctrl+d と同じ）。デーモン経由。
/// - `finalize` … 完了確定・pane 削除。デーモン経由。
/// - `requeue` … `failed` / `deleted` を `approved` に戻して再実行。デーモン経由。
/// - `send` … 追加指示。`in_progress` で入力待ち（idle）なら pane に直接送る（`DirectToPane`）。
///   それ以外はデーモン経由。
/// - `interrupt` … 作業中（`in_progress`）の pane に `esc` を送って入力を待ちに戻す
///   （人間の esc と同じ）。デーモンを経由しない。
pub fn task_manage(
    action: &str,
    task_id: i64,
    message: Option<&str>,
    run_id: Option<i64>,
) -> Result<Value, String> {
    if !matches!(action, "delete" | "finalize" | "send" | "requeue" | "interrupt") {
        return Err(format!(
            "Invalid action: '{}'. Give one of delete / finalize / send / requeue / interrupt.",
            action
        ));
    }
    if action == "send" && message.map(str::trim).unwrap_or("").is_empty() {
        return Err("action=send needs message (the extra instruction).".to_string());
    }

    // interrupt / send の振り分けにタスクの状態が要るので、いつもタスクを読む。
    let store = Arc::new(
        crate::common::open_db().map_err(|e| format!("Failed to open the database: {}", e))?,
    );
    let task = TaskManager::new(Arc::clone(&store))
        .get_task(task_id)
        .map_err(|e| format!("Failed to get the task: {}", e))?;

    let parent_run_id = match require_run_id(run_id) {
        Ok(id) => id,
        Err(_) => task.parent_run_id,
    };

    match action {
        // ── interrupt: デーモンを経由せず、そのまま pane に esc を送る ──
        "interrupt" => {
            if task.status != "in_progress" {
                return Err(format!(
                    "Task {} is {}. Only a working (in_progress) task can be interrupted.",
                    task_id, task.status
                ));
            }
            let (_task, _exec, pane_id) = task_pane(task_id)?;
            let herdr = crate::herdr::HerdrClient::new();
            herdr
                .send_keys(&pane_id, "esc")
                .map_err(|e| format!("Failed to send esc: {}", e))?;

            // esc を送っただけでは直ちに止まらないことがある。working でなくなるまで待つ。
            const POLL_MS: u64 = 250;
            const WAIT_MS: u64 = 5_000;
            let deadline = std::time::Instant::now() + std::time::Duration::from_millis(WAIT_MS);
            let mut agent_status = herdr
                .get_pane_status(&pane_id)
                .map(|i| i.agent_status)
                .unwrap_or_else(|_| "unknown".to_string());
            while agent_status == "working" && std::time::Instant::now() < deadline {
                std::thread::sleep(std::time::Duration::from_millis(POLL_MS));
                agent_status = herdr
                    .get_pane_status(&pane_id)
                    .map(|i| i.agent_status)
                    .unwrap_or_else(|_| "unknown".to_string());
            }

            if agent_status == "working" {
                // エラーにはしない。親が画面を見て次の手を選べるように状態を返すだけ。
                return Ok(json!({
                    "action": "interrupt",
                    "task_id": task_id,
                    "status": "still_working",
                    "agent_status": agent_status,
                    "message": "Sent esc, but the task is still working. Check with agent-link task capture <ID>, then interrupt again or end it with delete.",
                }));
            }
            Ok(json!({
                "action": "interrupt",
                "task_id": task_id,
                "status": "interrupted",
                "agent_status": agent_status,
                "message": format!(
                    "Sent esc to task {} (agent_status={}). Check the screen with agent-link task capture <ID> and send the next instruction with send.",
                    task_id, agent_status
                ),
            }))
        }

        // ── send: 状態を見て、その場で送るかデーモンに任せるかを決める ──
        "send" => {
            let message = message.unwrap_or_default();
            // agent_status は in_progress のときだけ使う（取得失敗は None 扱い）。
            let pane_agent_status = if task.status == "in_progress" {
                match task_pane(task_id) {
                    Ok((_t, _e, pane_id)) => crate::herdr::HerdrClient::new()
                        .get_pane_status(&pane_id)
                        .map(|i| i.agent_status)
                        .ok(),
                    Err(_) => None,
                }
            } else {
                None
            };

            match send_route(task_id, &task.status, pane_agent_status.as_deref())? {
                SendRoute::DirectToPane => {
                    let (_task, _exec, pane_id) = task_pane(task_id)?;
                    // 説明に追記する（履歴・記録用。task_manager の send と同じ形式）。
                    let new_desc = if task.description.is_empty() {
                        format!("[追加入力指示]\n{}", message)
                    } else {
                        format!("{}\n\n[追加入力指示]\n{}", task.description, message)
                    };
                    store
                        .update_task_description(task_id, &new_desc)
                        .map_err(|e| format!("Failed to update the description: {}", e))?;

                    let herdr = crate::herdr::HerdrClient::new();
                    let submit_key = crate::cc_command::CC_SUBMIT_KEY;
                    // 1 回目の実行は task_report まで待機が続くので、ターン実行は別に起こさない。
                    // ここで送った指示は同じ実行の続きとして処理される。
                    if let Err(e) = crate::orchestrator::submit_prompt_verified(
                        &herdr,
                        &pane_id,
                        message,
                        submit_key,
                        None,
                    ) {
                        // 届いたか分からないときは失敗にしない（送り直すと二重に届く）。
                        return match e {
                            crate::orchestrator::OrchestratorError::PromptUnconfirmed { .. } => {
                                Ok(json!({
                                    "action": "send",
                                    "task_id": task_id,
                                    "status": "unconfirmed",
                                    "message": format!(
                                        "{} Check the screen with `agent-link task capture {}` before sending again.",
                                        e, task_id
                                    ),
                                }))
                            }
                            e => Err(format!("Failed to send to the pane: {}", e)),
                        };
                    }

                    // 注意: ターン送信中（completed のあとに send して in_progress になった状態）に
                    // interrupt すると、子が idle になり `send_turn_to_pane` がそのターンを
                    // 完了とみなしてタスクが completed になる。この場合は次の send が
                    // Daemon 経路へ落ちるので、経路の取り合いは起きない。
                    Ok(json!({
                        "action": "send",
                        "task_id": task_id,
                        "status": "sent",
                        "message": format!(
                            "Sent the extra instruction to the pane of working task {}. Receive its completion with `agent-link task wait <ID>` (the subtask runs `agent-link task report`).",
                            task_id
                        ),
                    }))
                }
                SendRoute::Daemon => {
                    write_notification(
                        "task_manage_request",
                        json!({
                            "parent_run_id": parent_run_id,
                            "action": action,
                            "task_id": task_id,
                            "message": message,
                        }),
                    )?;
                    Ok(json!({
                        "action": action,
                        "task_id": task_id,
                        "status": "requested",
                        "message": format!(
                            "Sent the extra instruction to task {}. The same agent (pane) keeps handling it.",
                            task_id
                        ),
                    }))
                }
            }
        }

        // ── requeue: 条件を先に確かめる ──
        "requeue" => {
            if task.status != "failed" && task.status != "deleted" {
                return Err(format!(
                    "Task {} is {}. Only a failed / deleted task can be requeued.",
                    task_id, task.status
                ));
            }
            write_notification(
                "task_manage_request",
                json!({
                    "parent_run_id": parent_run_id,
                    "action": action,
                    "task_id": task_id,
                    "message": message,
                }),
            )?;
            Ok(json!({
                "action": action,
                "task_id": task_id,
                "status": "requested",
                "message": format!(
                    "Requested an operation on task {} ({}) from agent-link.",
                    task_id, action
                ),
            }))
        }

        // ── delete / finalize: 今までどおり通知を書く ──
        _ => {
            write_notification(
                "task_manage_request",
                json!({
                    "parent_run_id": parent_run_id,
                    "action": action,
                    "task_id": task_id,
                    "message": message,
                }),
            )?;
            Ok(json!({
                "action": action,
                "task_id": task_id,
                "status": "requested",
                "message": format!(
                    "Requested an operation on task {} ({}) from agent-link.",
                    task_id, action
                ),
            }))
        }
    }
}

// ============================================================================
// 3.8 task_status
// ============================================================================

/// サブタスクの状態一覧・単体詳細を返す。
pub fn task_status(
    status: Option<&str>,
    task_id: Option<i64>,
    run_id: Option<i64>,
) -> Result<Value, String> {
    let run_id = require_run_id(run_id)?;
    let mut tasks = tasks_json(run_id, status.or(Some("all")), None)?;
    if let Some(tid) = task_id {
        tasks.retain(|t| t.get("server_id").and_then(|v| v.as_i64()) == Some(tid));
    }
    Ok(json!({
        "count": tasks.len(),
        "tasks": tasks,
        "hint": "To investigate, use agent-link task capture <ID>. To operate, use agent-link task <interrupt|send|delete|finalize|requeue> <ID>.",
    }))
}

// ============================================================================
// 3.9 task_capture
// ============================================================================

/// タスクの pane 出力をキャプチャする。
pub fn task_capture(task_id: i64, lines: Option<u32>) -> Result<Value, String> {
    let (_task, _exec, pane_id) = task_pane(task_id)?;

    let lines = lines.unwrap_or(50);
    let client = crate::herdr::HerdrClient::new();
    let output = client
        .read_pane_output(&pane_id, lines)
        .map_err(|e| format!("Failed to read the pane: {}", e))?;

    let empty = output.is_empty();
    let mut result = json!({
        "task_id": task_id,
        "pane_id": pane_id,
        "lines": lines,
        "output": output,
    });
    if empty {
        result["message"] = json!("(the pane output is empty)");
    }
    Ok(result)
}

// ============================================================================
// 3.10 block_discover
// ============================================================================

/// 呼び出し可能なブロック定義を一覧する。
pub fn block_discover(parent: Option<&str>) -> Result<Value, String> {
    let parent_name: Option<String> = match parent {
        Some(p) if !p.trim().is_empty() => Some(p.to_string()),
        _ => std::env::var("AGENT_LINK_BLOCK_NAME").ok(),
    };

    let mut defs = collect_block_definitions();

    if let Some(ref parent_name) = parent_name {
        let parent_path = crate::orchestrator::resolve_block_path(parent_name);
        if parent_path.exists() {
            if let Ok(parsed) = parse_block_file(&parent_path) {
                match parsed.config.subblocks {
                    // subblocks が定義されていない → 呼び出し可能なサブブロックは「なし」
                    None => defs.clear(),
                    Some(allowed) => {
                        defs.retain(|(_, c)| allowed.iter().any(|b| b == c.block_id()));
                    }
                }
            }
        }
    }

    let blocks: Vec<Value> = defs
        .iter()
        .map(|(_, c)| {
            json!({
                "id": c.block_id(),
                "name": c.display_name(),
                "description": c.description,
            })
        })
        .collect();

    let mut result = json!({
        "count": blocks.len(),
        "blocks": blocks,
        "hint": "Give the id as block_type of agent-link task create. For details, see agent-link def show --kind block --name <id>.",
    });
    if defs.is_empty() {
        result["message"] = json!("No subblock can be called. Check subblocks in the main block definition.");
    }
    Ok(result)
}

// ============================================================================
// 3.11 block_inspect
// ============================================================================

/// ブロック定義の詳細を返す。
pub fn block_inspect(name: &str) -> Result<Value, String> {
    let mut path = crate::orchestrator::resolve_block_path(name);
    if !path.exists() {
        let fallback = collect_block_definitions()
            .into_iter()
            .find(|(_, c)| c.display_name() == name)
            .map(|(p, _)| p);
        if let Some(p) = fallback {
            path = p;
        }
    }
    if !path.exists() {
        return Err(format!(
            "Block definition not found: {} (give the ID or display name).",
            name
        ));
    }

    let parsed = parse_block_file(&path).map_err(|e| format!("Failed to parse the block definition: {}", e))?;
    let entry = block_def_to_json(&parsed.config);
    serde_json::to_value(entry).map_err(|e| format!("Failed to convert to JSON: {}", e))
}

// ============================================================================
// 3.12 sai_read（waggle 文書の一覧と本文）
// ============================================================================

/// waggle トークンの参照記法（`waggle:<tok>` / `waggle:<tok>:path:…`）を
/// トークン文字列だけに正規化する。
fn normalize_waggle_token(input: &str) -> String {
    let s = input.trim();
    let s = s.strip_prefix("waggle:").unwrap_or(s);
    s.split(":path:").next().unwrap_or(s).trim().to_string()
}

/// waggle 文書 1 件のマニフェストから表示用の 1 行を組み立てる。
/// トークンが解決できないときは `None`。
fn waggle_item_line(
    waggle: &crate::waggle_integration::WaggleIntegrator,
    token: &str,
    run_id: i64,
) -> Option<String> {
    let view = waggle.resolve_token(token).ok()?;
    let block_name = waggle.find_block_name_by_token(token);
    // 元ファイル名は `file://…` URL の末尾要素（phone_view と同じ取り出し方）
    let file_name = view
        .manifest
        .target
        .as_str()
        .strip_prefix("file://")
        .and_then(|p| p.rsplit('/').next())
        .unwrap_or("")
        .to_string();
    let description = view.manifest.meta.description.clone().unwrap_or_default();
    Some(format!(
        "waggle:{}: {} (run {} / {} / {})",
        token, description, run_id, block_name, file_name
    ))
}

/// waggle 文書の一覧（本文なし）を返す。`run_id` 指定時はその実行の出力だけ。
///
/// - `None`: 完了済みの実行の全トークン（`list_available_waggle_tokens`）
/// - `Some(id)`: その実行の `output_waggle_tokens`
///
/// 解決できないトークンは一覧から落とす。
/// U03: `include_running=true` の場合、実行中（running / pending・仮 completed）
/// のトークンも含める（w キーで作りたて文書を選ぶ用）。
pub fn sai_read_list(run_id: Option<i64>) -> Result<Value, String> {
    sai_read_list_inner(run_id, false)
}

/// U03: 実行中を含む waggle 文書一覧（`sai_read_list` の拡張版）。
///
/// TUI の w キー・`phone waggle` が使う。`include_running=true` で
/// running / review 中の文書も返す。`run_id` 指定時は従来通りその実行のみ。
pub fn sai_read_list_including_running(run_id: Option<i64>) -> Result<Value, String> {
    sai_read_list_inner(run_id, true)
}

fn sai_read_list_inner(run_id: Option<i64>, include_running: bool) -> Result<Value, String> {
    let store =
        crate::common::open_db().map_err(|e| format!("Failed to open the database: {}", e))?;

    let entries: Vec<(String, i64)> = match run_id {
        Some(id) => {
            let exec = store
                .get_execution_by_id(id)
                .map_err(|e| format!("Failed to get run id = {}: {}", id, e))?;
            let tokens: Vec<String> = exec
                .output_waggle_tokens
                .as_deref()
                .and_then(|j| serde_json::from_str(j).ok())
                .unwrap_or_default();
            tokens.into_iter().map(|t| (t, id)).collect()
        }
        None => {
            // ponytail: 全件は上限なしで取る。件数が増えたら limit 引数を足す
            store
                .list_waggle_tokens(None, -1, include_running)
                .map_err(|e| format!("Failed to list waggle tokens: {}", e))?
                .into_iter()
                .map(|e| (e.token, e.execution_id))
                .collect()
        }
    };

    let db_path = crate::project::db_path();
    let waggle = crate::waggle_integration::WaggleIntegrator::open(&db_path)
        .map_err(|e| format!("Failed to open the waggle DB: {}", e))?;

    let mut items = Vec::new();
    for (token, id) in entries {
        if let Some(line) = waggle_item_line(&waggle, &token, id) {
            items.push(line);
        }
    }

    Ok(json!({
        "count": items.len(),
        "items": items,
        "hint": "Read the body with `agent-link doc read <token>`. To pass it to a subtask, write waggle:<token> in task_text and the body is embedded.",
    }))
}

/// waggle 文書 1 件の本文を返す。
///
/// `token` は `waggle:<tok>` / `waggle:<tok>:path:…` のどちらでもよい。
pub fn sai_read(token: &str) -> Result<Value, String> {
    let token = normalize_waggle_token(token);
    if token.is_empty() {
        return Err("token is required.".to_string());
    }

    let db_path = crate::project::db_path();
    let waggle = crate::waggle_integration::WaggleIntegrator::open(&db_path)
        .map_err(|e| format!("Failed to open the waggle DB: {}", e))?;

    let view = waggle
        .resolve_token(&token)
        .map_err(|e| format!("Failed to resolve waggle token '{}': {}", token, e))?;
    let content = waggle.resolve_content(&token).map_err(|e| {
        format!(
            "Failed to get the body of waggle token '{}': {}",
            token, e
        )
    })?;

    let file_name = view
        .manifest
        .target
        .as_str()
        .strip_prefix("file://")
        .and_then(|p| p.rsplit('/').next())
        .unwrap_or("")
        .to_string();

    Ok(json!({
        "token": token,
        "block_name": waggle.find_block_name_by_token(&token),
        "file_name": file_name,
        "description": view.manifest.meta.description.clone(),
        "content": content,
    }))
}

// ============================================================================
// テスト
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    /// `task_output` の待ちのテストに使う、誰の run にも無いタスク番号。
    /// （タスクを作ると `AGENT_LINK_PROJECT_ROOT` を持つロックを長く抱え、
    /// 環境の取り合いで他のテストが落ちるため）
    const NO_SUCH_TASK_A: i64 = 999_998;
    const NO_SUCH_TASK_B: i64 = 999_999;

    // 環境変数を設定する他モジュールのテスト（orchestrator / cc_command / task_cwd）と
    // 排他する。モジュール固有のロックに分けると AGENT_LINK_PROJECT_ROOT が競合する。
    struct EnvGuard {
        _lock: std::sync::MutexGuard<'static, ()>,
        project_root: PathBuf,
    }

    impl EnvGuard {
        fn new() -> Self {
            let lock = crate::common::AGENT_LINK_ENV_TEST_LOCK
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            let dir = std::env::temp_dir().join(format!(
                "agent-link-agent-api-test-{}-{}",
                std::process::id(),
                unix_millis()
            ));
            std::fs::create_dir_all(&dir).unwrap();
            std::env::set_var("AGENT_LINK_PROJECT_ROOT", &dir);
            Self {
                _lock: lock,
                project_root: dir,
            }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            std::env::remove_var("AGENT_LINK_PROJECT_ROOT");
            std::env::remove_var("AGENT_LINK_PARENT_RUN_ID");
            let _ = std::fs::remove_dir_all(&self.project_root);
        }
    }

    /// run_wait の試験用に、同じブロックで run を n 件作る。
    fn make_runs(n: usize) -> Vec<i64> {
        let store = crate::common::open_db().unwrap();
        let block = crate::db::models::test_helpers::create_test_block("run-wait-test-block");
        let block_id = store.insert_block(&block).unwrap();
        (0..n).map(|_| store.create_execution(block_id, None).unwrap()).collect()
    }

    /// run に擬似 pane を付け、状態ファイルを書く。`updated_ago_secs` 秒前に更新された状態にする。
    fn attach_pane_state(run_id: i64, state: &str, updated_ago_secs: i64) {
        let store = crate::common::open_db().unwrap();
        let pane_id = format!("hl-{:012x}", run_id);
        store.update_execution_pane(run_id, &pane_id, 0).unwrap();
        let dir = crate::headless::pane_dir(&pane_id);
        std::fs::create_dir_all(&dir).unwrap();
        let updated_at =
            (chrono::Utc::now() - chrono::Duration::seconds(updated_ago_secs)).to_rfc3339();
        crate::headless::write_state(
            &dir,
            &crate::headless::HeadlessState {
                pane_id: pane_id.clone(),
                runner: "cc".to_string(),
                state: state.to_string(),
                turn: 1,
                partial: String::new(),
                error: String::new(),
                updated_at,
            },
        )
        .unwrap();
    }

    /// run_wait: 完了した run を返し、残りは pending に入る（試験は timeout 0 で 1 回だけ調べる）。
    #[test]
    fn run_wait_returns_completed_and_leaves_rest_pending() {
        let _guard = EnvGuard::new();
        let runs = make_runs(2);
        crate::common::open_db()
            .unwrap()
            .update_execution_status(runs[0], "completed")
            .unwrap();
        attach_pane_state(runs[1], "working", 0);

        let v = run_wait(&runs, Some(0), &AtomicBool::new(false)).unwrap();
        assert_eq!(v["status"], "completed");
        assert_eq!(v["run_id"], runs[0]);
        assert_eq!(v["pending"], json!([runs[1]]));
    }

    /// run_wait: 失敗した run は error 付きで返る。
    #[test]
    fn run_wait_returns_failed_with_error() {
        let _guard = EnvGuard::new();
        let runs = make_runs(1);
        crate::common::open_db()
            .unwrap()
            .update_execution_error(runs[0], "Claude の認証に失敗しました（401）", 1)
            .unwrap();

        let v = run_wait(&runs, Some(0), &AtomicBool::new(false)).unwrap();
        assert_eq!(v["status"], "failed");
        assert_eq!(v["run_id"], runs[0]);
        assert!(v["error"].as_str().unwrap().contains("401"));
        assert_eq!(v["pending"], json!([]));
    }

    /// run_wait: idle が 60 秒を超えて続いた run は stalled（reason idle）。
    /// 更新から 60 秒たっていなければ stalled にしない。
    #[test]
    fn run_wait_stalls_only_after_idle_threshold() {
        let _guard = EnvGuard::new();
        let runs = make_runs(2);
        attach_pane_state(runs[0], "idle", 3600);
        attach_pane_state(runs[1], "idle", 0);

        let v = run_wait(&runs[..1], Some(0), &AtomicBool::new(false)).unwrap();
        assert_eq!(v["status"], "stalled");
        assert_eq!(v["reason"], "idle");
        assert_eq!(v["run_id"], runs[0]);

        let v = run_wait(&runs[1..], Some(0), &AtomicBool::new(false)).unwrap();
        assert_eq!(v["status"], "in_progress", "idle になってから 60 秒たっていない");
    }

    /// run_wait: 時間内に止まらなければ in_progress で、pending は全件。
    #[test]
    fn run_wait_times_out_as_in_progress() {
        let _guard = EnvGuard::new();
        let runs = make_runs(2);
        attach_pane_state(runs[0], "working", 0);
        attach_pane_state(runs[1], "working", 0);

        let v = run_wait(&runs, Some(0), &AtomicBool::new(false)).unwrap();
        assert_eq!(v["status"], "in_progress");
        assert_eq!(v["pending"], json!(runs));
    }

    /// run_wait: 無い run_id は「見つかりません」で断る（CLI は終了コード 3）。
    #[test]
    fn run_wait_missing_run_is_error() {
        let _guard = EnvGuard::new();
        let err = run_wait(&[NO_SUCH_TASK_A], Some(0), &AtomicBool::new(false)).unwrap_err();
        assert!(err.contains("not found"), "{err}");
    }

    #[test]
    fn write_notification_includes_type_and_timestamp() {
        let guard = EnvGuard::new();
        let path = write_notification("unit_test", json!({ "foo": "bar" })).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        let value: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(value["type"], "unit_test");
        assert!(value["timestamp"].as_str().unwrap().len() > 0);
        assert_eq!(value["foo"], "bar");
        drop(guard);
    }

    /// P4: doc write がトークンと ref を返し、通知に waggle_token が入る。
    /// デーモン相当の処理（poll_and_process_one）でも同じトークンが返り、
    /// 再トークン化されない（トークンが増えない）ことを確認する。
    // ── write_check ──

    /// write_check を設定した一時プロジェクトで `write` を呼ぶ補助。script は sh の本文。
    fn write_with_check(
        guard: &EnvGuard,
        script: &str,
        timeout_ms: u64,
        content: &str,
    ) -> Result<Value, String> {
        let root = &guard.project_root;
        std::fs::create_dir_all(root.join("empty-home")).unwrap();
        std::env::set_var("AGENT_LINK_HOME", root.join("empty-home"));
        std::env::set_var("AGENT_LINK_PARENT_RUN_ID", "42");
        std::env::set_var("AGENT_LINK_BLOCK_NAME", "blk");
        let sh = root.join("check.sh");
        std::fs::write(&sh, script).unwrap();
        let cmd = json!([ "sh", sh.to_string_lossy(), "{run_id}", "{block}", "{filename}" ]);
        std::fs::create_dir_all(root.join(".agent-link")).unwrap();
        std::fs::write(
            root.join(".agent-link/settings.json"),
            json!({ "write_check": cmd }).to_string(),
        )
        .unwrap();
        let r = write_with_timeout(
            "a/doc.md",
            content,
            Some("説明"),
            None,
            std::time::Duration::from_millis(timeout_ms),
        );
        std::env::remove_var("AGENT_LINK_HOME");
        std::env::remove_var("AGENT_LINK_BLOCK_NAME");
        r
    }

    #[test]
    fn write_check_rejected_does_not_save() {
        let g = EnvGuard::new();
        let err = write_with_check(&g, "echo 違反です >&2; exit 1", 5000, "x").unwrap_err();
        assert!(err.contains("違反です"), "{err}");
        assert!(!g.project_root.join(".agent-link/outputs").exists()
            || !walk_has(&g.project_root.join(".agent-link/outputs"), "doc.md"));
    }

    fn walk_has(dir: &Path, name: &str) -> bool {
        std::fs::read_dir(dir).map(|rd| rd.flatten().any(|e| {
            e.file_name() == name || (e.path().is_dir() && walk_has(&e.path(), name))
        })).unwrap_or(false)
    }

    #[test]
    fn write_check_stdout_replaces_content() {
        let g = EnvGuard::new();
        let r = write_with_check(&g, "sed 's/^/X/'", 5000, "abc\n").unwrap();
        assert_eq!(r["checked"], true);
        let saved = std::fs::read_to_string(r["path"].as_str().unwrap()).unwrap();
        assert_eq!(saved, "Xabc\n");
    }

    #[test]
    fn write_check_broken_saves_original_with_warning() {
        let g = EnvGuard::new();
        for (script, ms) in [("exit 3", 5000), ("sleep 5", 300)] {
            let r = write_with_check(&g, script, ms, "orig").unwrap();
            assert_eq!(r["checked"], false);
            assert!(r["warning"].as_str().unwrap().contains("Saved without checking"));
            assert_eq!(std::fs::read_to_string(r["path"].as_str().unwrap()).unwrap(), "orig");
        }
    }

    #[test]
    fn write_check_missing_binary_saves_original_with_warning() {
        let g = EnvGuard::new();
        let root = &g.project_root;
        std::fs::create_dir_all(root.join("empty-home")).unwrap();
        std::env::set_var("AGENT_LINK_HOME", root.join("empty-home"));
        std::env::set_var("AGENT_LINK_PARENT_RUN_ID", "42");
        std::fs::create_dir_all(root.join(".agent-link")).unwrap();
        std::fs::write(
            root.join(".agent-link/settings.json"),
            r#"{"write_check":["/no/such/bin"]}"#,
        )
        .unwrap();
        let r = write("doc.md", "orig", None, None).unwrap();
        std::env::remove_var("AGENT_LINK_HOME");
        assert_eq!(r["checked"], false);
        assert!(r["warning"].is_string());
    }

    #[test]
    fn write_check_expands_placeholders_and_env() {
        let g = EnvGuard::new();
        let out = g.project_root.join("seen.txt");
        let script = format!(
            "echo \"$1 $2 $3 $AGENT_LINK_WRITE_FILENAME $AGENT_LINK_WRITE_DESCRIPTION\" > {}; cat >/dev/null",
            out.display()
        );
        write_with_check(&g, &script, 5000, "x").unwrap();
        assert_eq!(
            std::fs::read_to_string(out).unwrap().trim(),
            "42 blk doc.md doc.md 説明"
        );
    }

    #[test]
    fn write_without_write_check_has_no_checked_key() {
        let g = EnvGuard::new();
        std::fs::create_dir_all(g.project_root.join("empty-home")).unwrap();
        std::env::set_var("AGENT_LINK_HOME", g.project_root.join("empty-home"));
        std::env::set_var("AGENT_LINK_PARENT_RUN_ID", "42");
        let r = write("doc.md", "x", None, None).unwrap();
        std::env::remove_var("AGENT_LINK_HOME");
        assert!(r.get("checked").is_none() && r.get("warning").is_none());
    }

    #[test]
    fn write_returns_token_and_ref_and_daemon_keeps_the_same_token() {
        let guard = EnvGuard::new();
        std::env::set_var("AGENT_LINK_PARENT_RUN_ID", "42");

        let result = write("report.md", "# 報告\n", Some("報告書"), None).unwrap();
        let token = result["token"].as_str().expect("token が返る").to_string();
        assert!(!token.is_empty());
        assert_eq!(
            result["ref"].as_str().unwrap(),
            format!("waggle:{}:path:.agent-link/outputs/*/42/report.md", token)
        );

        // 通知 payload に waggle_token が入っていること
        let pending = crate::common::notify_pending_dir();
        let entries: Vec<_> = std::fs::read_dir(&pending).unwrap().flatten().collect();
        assert_eq!(entries.len(), 1);
        let value: Value =
            serde_json::from_str(&std::fs::read_to_string(entries[0].path()).unwrap()).unwrap();
        assert_eq!(value["waggle_token"].as_str().unwrap(), token);

        // デーモン相当の処理：同じトークンが返り、再トークン化されない
        let db_path = crate::project::db_path();
        let integrator = crate::waggle_integration::WaggleIntegrator::open(&db_path).unwrap();
        let processed = crate::doc_write_notify::poll_and_process_one(&integrator, None)
            .unwrap()
            .expect("通知が処理される");
        assert_eq!(processed.waggle_token.as_deref(), Some(token.as_str()));
        assert!(processed.error.is_none());
        // write が mint したトークンが実際に DB に存在する（エージェント側でトークン化済み）
        assert!(
            integrator.resolve_token(&token).is_ok(),
            "write が mint したトークンが DB に存在する"
        );

        drop(guard);
    }

    /// AC6: stalled と completed が両方あるときは completed を返す。
    /// 完了が無い場合のみ、ファイル名がいちばん大きい（新しい）stalled を返す。
    #[test]
    fn find_result_file_prefers_completed_over_stalled() {
        let dir = std::env::temp_dir().join(format!(
            "agent-link-find-result-file-{}-{}",
            std::process::id(),
            unix_millis()
        ));
        std::fs::create_dir_all(&dir).unwrap();

        // 古い stalled と新しい stalled
        std::fs::write(
            dir.join("1000-task_7.json"),
            json!({ "type": "task_stalled", "task_id": 7, "reason": "idle" }).to_string(),
        )
        .unwrap();
        std::fs::write(
            dir.join("3000-task_7.json"),
            json!({ "type": "task_stalled", "task_id": 7, "reason": "long_running" }).to_string(),
        )
        .unwrap();

        // completed なし → 新しい stalled が返る
        let (_, value) = find_result_file(&dir, 7).expect("stalled が取れる");
        assert_eq!(value["type"], "task_stalled");
        assert_eq!(value["reason"], "long_running");

        // completed を足す → completed が優先される
        std::fs::write(
            dir.join("4000-task_7.json"),
            json!({ "type": "task_completed", "task_id": 7 }).to_string(),
        )
        .unwrap();
        let (_, value) = find_result_file(&dir, 7).expect("completed が取れる");
        assert_eq!(value["type"], "task_completed");

        // 別タスクのファイルは拾わない
        assert!(find_result_file(&dir, 8).is_none());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn task_create_marks_all_tasks_approved() {
        let guard = EnvGuard::new();
        std::env::set_var("AGENT_LINK_PARENT_RUN_ID", "42");

        let tasks = json!([
            { "client_id": "t1", "subject": "件名1", "task_text": "説明1" },
            { "client_id": "t2", "subject": "件名2", "task_text": "説明2" },
        ]);
        let result = task_create(&tasks, None, None, None).unwrap();
        assert_eq!(result["status"], "approved");
        for t in result["tasks"].as_array().unwrap() {
            assert_eq!(t["status"], "approved");
        }

        // pending/ の通知にも approval.status == approved が付いていること
        let pending = crate::common::notify_pending_dir();
        let entries: Vec<_> = std::fs::read_dir(&pending).unwrap().flatten().collect();
        assert_eq!(entries.len(), 1);
        let text = std::fs::read_to_string(entries[0].path()).unwrap();
        let value: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(value["type"], "task_created");
        for t in value["tasks"].as_array().unwrap() {
            assert_eq!(t["approval"]["status"], "approved");
            assert_eq!(t["approval"]["comment"], Value::Null);
        }

        drop(guard);
    }

    #[test]
    fn task_create_batch_id_matches_pattern() {
        let guard = EnvGuard::new();
        std::env::set_var("AGENT_LINK_PARENT_RUN_ID", "42");

        let tasks = json!([
            { "client_id": "t1", "subject": "件名", "task_text": "説明" },
        ]);
        let result = task_create(&tasks, Some(3), None, None).unwrap();
        let batch_id = result["batch_id"].as_str().unwrap();

        // batch_<8桁数字>_r<round>_<4文字> にマッチすること
        let parts: Vec<&str> = batch_id.splitn(4, '_').collect();
        assert_eq!(parts.len(), 4, "batch_id: {}", batch_id);
        assert_eq!(parts[0], "batch");
        assert_eq!(parts[1].len(), 8);
        assert!(parts[1].chars().all(|c| c.is_ascii_digit()));
        assert_eq!(parts[2], "r3");
        assert_eq!(parts[3].len(), 4);

        drop(guard);
    }

    /// AC4: task_text が無いと Err になる。description / inputs は入力として受け付けない。
    #[test]
    fn task_create_requires_task_text() {
        let guard = EnvGuard::new();
        std::env::set_var("AGENT_LINK_PARENT_RUN_ID", "42");

        // description だけでは作れない
        let tasks = json!([
            { "client_id": "t1", "subject": "件名", "description": "説明" },
        ]);
        let err = task_create(&tasks, None, None, None).unwrap_err();
        assert!(err.contains("task_text"), "err={}", err);

        // 空文字の task_text も不可
        let tasks = json!([
            { "client_id": "t1", "subject": "件名", "task_text": "   " },
        ]);
        let err = task_create(&tasks, None, None, None).unwrap_err();
        assert!(err.contains("task_text"), "err={}", err);

        // 通知を書いていないこと
        let pending = crate::common::notify_pending_dir();
        let count = std::fs::read_dir(&pending)
            .map(|d| d.flatten().count())
            .unwrap_or(0);
        assert_eq!(count, 0);

        drop(guard);
    }

    /// AC4: task_text だけが採用され、description / inputs は無視される。
    #[test]
    /// U05: `description` は task.md と同じ項目として受け付ける（省略時は task_text）。
    /// `inputs` は受け付けない（task_text に一本化）。`block` は `block_type` の別名。
    #[test]
    fn task_create_ignores_description_and_inputs() {
        let guard = EnvGuard::new();
        std::env::set_var("AGENT_LINK_PARENT_RUN_ID", "42");

        let tasks = json!([
            { "client_id": "t1", "subject": "件名", "task_text": "本文",
              "description": "説明文", "inputs": { "foo": "bar" },
              "block": "no-such-block-alias-check" },
        ]);
        // block 別名の検証は別テストで行うため、ここでは block を外して description のみ確認
        let tasks = json!([
            { "client_id": "t1", "subject": "件名", "task_text": "本文",
              "description": "説明文", "inputs": { "foo": "bar" } },
        ]);
        let result = task_create(&tasks, None, None, None).unwrap();
        assert_eq!(result["status"], "approved");

        let pending = crate::common::notify_pending_dir();
        let entries: Vec<_> = std::fs::read_dir(&pending).unwrap().flatten().collect();
        assert_eq!(entries.len(), 1);
        let value: Value =
            serde_json::from_str(&std::fs::read_to_string(entries[0].path()).unwrap()).unwrap();
        let t = &value["tasks"][0];
        // U05: description は task.md と同じ項目として採用される
        assert_eq!(t["description"], "説明文");
        // inputs は task_text だけ（inputs 引数は受け付けない）
        assert_eq!(t["inputs"], json!({ "task_text": "本文" }));

        drop(guard);
    }

    /// U05: `block` は `block_type` の別名として受け付ける。
    #[test]
    fn task_create_accepts_block_alias() {
        let guard = EnvGuard::new();
        std::env::set_var("AGENT_LINK_PARENT_RUN_ID", "42");
        // 存在しない block 名で別名経路の検証が通ること（検証エラーになる＝別名が読まれた）
        let tasks = json!([
            { "client_id": "t1", "subject": "件名", "task_text": "本文", "block": "no-such-block" },
        ]);
        let result = task_create(&tasks, None, None, None).unwrap();
        assert_eq!(result["status"], "failed");
        let errors = result["validation_errors"].as_array().unwrap();
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0]["block_type"], "no-such-block");
        drop(guard);
    }

    #[test]
    fn task_create_rejects_unknown_block_type_without_writing_notification() {
        let guard = EnvGuard::new();
        std::env::set_var("AGENT_LINK_PARENT_RUN_ID", "42");

        let tasks = json!([
            { "client_id": "t1", "subject": "件名", "task_text": "本文", "block_type": "no-such-block" },
        ]);
        let result = task_create(&tasks, None, None, None).unwrap();
        assert_eq!(result["status"], "failed");
        let errors = result["validation_errors"].as_array().unwrap();
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0]["kind"], "missing");
        assert_eq!(errors[0]["field"], "<block_type>");

        // 通知は書かれていないこと
        let pending = crate::common::notify_pending_dir();
        let count = std::fs::read_dir(&pending).map(|d| d.flatten().count()).unwrap_or(0);
        assert_eq!(count, 0);

        drop(guard);
    }

    // -----------------------------------------------------------------------
    // U2: `card`（todo のカードを自分の子として起こす）
    // -----------------------------------------------------------------------

    /// テスト用のブロック定義を置く（`task_create` の block_type 検証を通すため）。
    fn write_test_block(root: &Path, block_id: &str) {
        let dir = root.join(".agent-link/blocks");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join(format!("{}.md", block_id)),
            format!(
                "---\nname: {id}\ndescription: テスト用ブロック\nmodel: anthropic/claude-sonnet-4-5\n---\n# テスト\n\n本文\n",
                id = block_id
            ),
        )
        .unwrap();
    }

    /// todo のカード（`.agent-link/outputs/todo/<run_id>/task.md`）を作る（テスト用）。
    fn write_todo_card(
        root: &Path,
        run_id: i64,
        block: &str,
        executer: &str,
        description: &str,
        task_text: &str,
    ) {
        let dir = root.join(".agent-link/outputs/todo").join(run_id.to_string());
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("task.md"),
            format!(
                "---\ncreation_date: 2026/09/30 10:00:00\nexecution_date: -\nrun_id: {rid}\n\
                 description: {desc}\nblock: {block}\nthinking: \nmodel: \n\
                 executer: {executer}\n\
                 note: n1\n---\n\n# task_text\n\n{text}\n",
                rid = run_id,
                desc = description,
                block = block,
                executer = executer,
                text = task_text
            ),
        )
        .unwrap();
    }

    /// テスト用の親実行を作る（`tasks.parent_run_id` は `execution_history` を指すため）。
    ///
    /// 新しい DB では最初の実行の id が 1 になる。返り値は run_id。
    fn create_test_parent_run(store: &crate::db::SaiStore, block_id: &str) -> i64 {
        let block = crate::db::models::test_helpers::create_test_block(block_id);
        let id = store.insert_block(&block).unwrap();
        store.create_execution(id, None).unwrap()
    }

    /// `task_created` 通知 1 枚を読んで `TaskCreatedNotification` にする（デーモン側の入口）。
    fn read_task_created_notification() -> crate::task_manager::TaskCreatedNotification {
        let pending = crate::common::notify_pending_dir();
        let entries: Vec<_> = std::fs::read_dir(&pending).unwrap().flatten().collect();
        let mut texts = Vec::new();
        for e in entries {
            let text = std::fs::read_to_string(e.path()).unwrap();
            let v: Value = serde_json::from_str(&text).unwrap();
            if v["type"] == "task_created" {
                texts.push(text);
            }
        }
        assert_eq!(texts.len(), 1, "task_created 通知が 1 枚だけ書かれる");
        serde_json::from_str(&texts[0]).unwrap()
    }

    /// U2 AC1: `card` を渡すと、カードの本文・ブロック・説明からタスクが作られる。
    /// 通知に `card` が載り、デーモンが作ると `tasks.card_run_id` に入る。
    #[test]
    fn task_create_with_card_uses_card_body_block_and_description() {
        let guard = EnvGuard::new();
        let store = crate::common::open_db().unwrap();
        let parent_run_id = create_test_parent_run(&store, "card-parent");
        std::env::set_var("AGENT_LINK_PARENT_RUN_ID", &parent_run_id.to_string());
        write_test_block(&guard.project_root, "card-block");
        write_todo_card(
            &guard.project_root,
            120,
            "card-block",
            "professor",
            "N12 の読解",
            "N12 を読解して報告",
        );

        // subject を書かないと、カードの description（無ければ block）が入る
        let tasks = json!([{ "client_id": "R1", "card": 120, "timeout": 3600 }]);
        let result = task_create(&tasks, None, None, None).unwrap();
        assert_eq!(result["status"], "approved");

        let notif = read_task_created_notification();
        assert_eq!(notif.parent_run_id, parent_run_id);
        assert_eq!(notif.tasks.len(), 1);
        let t = &notif.tasks[0];
        assert_eq!(t.subject, "N12 の読解");
        assert_eq!(t.block_type.as_deref(), Some("card-block"));
        assert_eq!(t.timeout, Some(3600));
        assert_eq!(t.card_run_id, Some(120));
        assert_eq!(
            t.inputs,
            Some(json!({ "task_text": "N12 を読解して報告" })),
            "task_text はカード本文から取る"
        );

        // デーモンが作っても card_run_id が保存される
        let manager = crate::task_manager::TaskManager::new(std::sync::Arc::new(store));
        let ids = manager.process_task_created(&notif).unwrap();
        assert_eq!(ids.len(), 1);
        let created = manager.get_task(ids[0]).unwrap();
        assert_eq!(created.card_run_id, Some(120));
        assert_eq!(created.subject, "N12 の読解");
        // task_list / task_status の JSON に "card" が出る
        assert_eq!(task_to_json(&created, &[])["card"], json!(120));

        drop(guard);
    }

    /// 廃止した `runner` を書いても、値が何でもエラーにせず無視して作れる。出力の JSON に `runner` は無い。
    #[test]
    fn task_create_ignores_runner() {
        let guard = EnvGuard::new();
        let store = Arc::new(crate::common::open_db().unwrap());
        let parent_run_id = create_test_parent_run(&store, "runner-parent");
        std::env::set_var("AGENT_LINK_PARENT_RUN_ID", &parent_run_id.to_string());
        write_test_block(&guard.project_root, "runner-block");

        let items = json!([
            { "client_id": "R1", "subject": "件名", "task_text": "本文",
              "block_type": "runner-block", "runner": "pi" },
            { "client_id": "R2", "subject": "件名", "task_text": "本文",
              "block_type": "runner-block", "runner": "gl" },
            { "client_id": "R3", "subject": "件名", "task_text": "本文",
              "block_type": "runner-block", "runner": "cc" },
        ]);
        let result = task_create(&items, None, None, None).unwrap();
        assert_eq!(result["status"], "approved");
        let notif = read_task_created_notification();
        assert_eq!(notif.tasks.len(), 3);

        let manager = crate::task_manager::TaskManager::new(store.clone());
        let ids = manager.process_task_created(&notif).unwrap();
        let created = manager.get_task(ids[0]).unwrap();
        assert!(task_to_json(&created, &[]).get("runner").is_none());

        drop(guard);
    }

    /// `model` / `thinking` は通知と行に載る。cc が黙って捨てる値と thinking の外れ値は task_create で断る。
    #[test]
    fn task_create_model_and_thinking_pass_and_bad_values_rejected() {
        let guard = EnvGuard::new();
        let store = Arc::new(crate::common::open_db().unwrap());
        let parent_run_id = create_test_parent_run(&store, "model-parent");
        std::env::set_var("AGENT_LINK_PARENT_RUN_ID", &parent_run_id.to_string());
        write_test_block(&guard.project_root, "model-block");
        let item = |extra: Value| {
            let mut v = json!({ "client_id": "M1", "subject": "件名", "task_text": "本文",
                "block_type": "model-block" });
            v.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
            json!([v])
        };

        let err = task_create(&item(json!({ "thinking": "off" })), None, None, None).unwrap_err();
        assert!(err.contains("thinking must be one of none / low"), "err={}", err);
        let err = task_create(&item(json!({ "model": "z-ai/glm-5.3-flash" })), None, None, None)
            .unwrap_err();
        assert!(err.contains("model must be opus / sonnet / haiku"), "err={}", err);

        let result = task_create(
            &item(json!({ "model": "haiku", "thinking": "LOW" })),
            None,
            None,
            None,
        )
        .unwrap();
        assert_eq!(result["status"], "approved");
        let notif = read_task_created_notification();
        assert_eq!(notif.tasks[0].model.as_deref(), Some("haiku"));
        assert_eq!(notif.tasks[0].thinking.as_deref(), Some("low"));

        let manager = crate::task_manager::TaskManager::new(store.clone());
        let ids = manager.process_task_created(&notif).unwrap();
        let created = manager.get_task(ids[0]).unwrap();
        assert_eq!(created.model.as_deref(), Some("haiku"));
        assert_eq!(created.thinking.as_deref(), Some("low"));

        drop(guard);
    }

    /// `task_create --json` の各 task に task_id が入る。デーモンが止まっていれば null（待たない）。
    #[test]
    fn task_create_returns_null_task_id_when_daemon_stopped() {
        let guard = EnvGuard::new();
        let store = crate::common::open_db().unwrap();
        let parent_run_id = create_test_parent_run(&store, "tid-parent");
        std::env::set_var("AGENT_LINK_PARENT_RUN_ID", &parent_run_id.to_string());
        write_test_block(&guard.project_root, "tid-block");

        let tasks = json!([{ "client_id": "T1", "subject": "件名", "task_text": "本文",
            "block_type": "tid-block" }]);
        let result = task_create(&tasks, None, None, None).unwrap();
        assert_eq!(result["tasks"][0]["client_id"], "T1");
        assert!(result["tasks"][0]["task_id"].is_null());
        drop(guard);
    }

    /// 行が出来ていれば、その task_id を client_id ごとに集める。揃わなければ出来た分だけ返す。
    #[test]
    fn collect_task_ids_maps_created_rows_and_stops_on_timeout() {
        let guard = EnvGuard::new();
        let store = Arc::new(crate::common::open_db().unwrap());
        let parent_run_id = create_test_parent_run(&store, "collect-parent");
        std::env::set_var("AGENT_LINK_PARENT_RUN_ID", &parent_run_id.to_string());
        write_test_block(&guard.project_root, "collect-block");
        let tasks = json!([{ "client_id": "C1", "subject": "件名", "task_text": "本文",
            "block_type": "collect-block" }]);
        task_create(&tasks, None, None, None).unwrap();
        let notif = read_task_created_notification();
        let manager = crate::task_manager::TaskManager::new(store.clone());
        let ids = manager.process_task_created(&notif).unwrap();

        let map = collect_task_ids(&store, &notif.batch_id, 1, std::time::Duration::from_millis(100));
        assert_eq!(map.get("C1"), Some(&ids[0]));
        // 2 件を待っても 1 件しか無いので、待ち時間の後に出来た分だけ返す
        let partial = collect_task_ids(&store, &notif.batch_id, 2, std::time::Duration::from_millis(100));
        assert_eq!(partial.len(), 1);
        drop(guard);
    }

    /// U2: todo にカードが無い（run_id が違うものは無い）是エラー。
    #[test]
    fn task_create_with_card_rejects_missing_card() {
        let guard = EnvGuard::new();
        std::env::set_var("AGENT_LINK_PARENT_RUN_ID", "42");
        write_test_block(&guard.project_root, "card-block");
        // 121 のカードがあるだけで、120 は無い
        write_todo_card(&guard.project_root, 121, "card-block", "professor", "説明", "本文");

        let tasks = json!([{ "client_id": "R1", "subject": "件名", "card": 120 }]);
        let err = task_create(&tasks, None, None, None).unwrap_err();
        assert!(err.contains("Card 120 not found in todo"), "err={}", err);

        // 通知は書かれていないこと
        let pending = crate::common::notify_pending_dir();
        let count = std::fs::read_dir(&pending)
            .map(|d| d.flatten().count())
            .unwrap_or(0);
        assert_eq!(count, 0);

        drop(guard);
    }

    /// U2: `executer: human` のカードはエージェントが起こせない。
    #[test]
    fn task_create_with_card_rejects_human_executer() {
        let guard = EnvGuard::new();
        std::env::set_var("AGENT_LINK_PARENT_RUN_ID", "42");
        write_test_block(&guard.project_root, "card-block");
        write_todo_card(&guard.project_root, 120, "card-block", "human", "説明", "本文");

        let tasks = json!([{ "client_id": "R1", "subject": "件名", "card": 120 }]);
        let err = task_create(&tasks, None, None, None).unwrap_err();
        assert!(err.contains("is a human card"), "err={}", err);
        assert!(err.contains("executer: agent"), "err={}", err);

        drop(guard);
    }

    /// U12: `executer: agent` のカードも、旧カードの値と同じに起こせる。
    #[test]
    fn task_create_with_card_accepts_agent_executer() {
        let guard = EnvGuard::new();
        let store = crate::common::open_db().unwrap();
        let parent_run_id = create_test_parent_run(&store, "card-agent-parent");
        std::env::set_var("AGENT_LINK_PARENT_RUN_ID", &parent_run_id.to_string());
        write_test_block(&guard.project_root, "card-block");
        write_todo_card(&guard.project_root, 122, "card-block", "agent", "agent の説明", "本文");

        let tasks = json!([{ "client_id": "R1", "card": 122 }]);
        let result = task_create(&tasks, None, None, None).unwrap();
        assert_eq!(result["status"], "approved");
        let notif = read_task_created_notification();
        assert_eq!(notif.tasks[0].card_run_id, Some(122));
        assert_eq!(notif.tasks[0].subject, "agent の説明");

        drop(guard);
    }

    /// U2: `card` と `task_text` の併用はできない（依頼の正は 1 つ）。
    #[test]
    fn task_create_with_card_rejects_task_text() {
        let guard = EnvGuard::new();
        std::env::set_var("AGENT_LINK_PARENT_RUN_ID", "42");
        write_test_block(&guard.project_root, "card-block");
        write_todo_card(&guard.project_root, 120, "card-block", "professor", "説明", "本文");

        let tasks = json!([{ "client_id": "R1", "subject": "件名", "card": 120, "task_text": "書かない" }]);
        let err = task_create(&tasks, None, None, None).unwrap_err();
        assert!(
            err.contains("Do not write task_text when card is given"),
            "err={}",
            err
        );

        drop(guard);
    }

    /// U2: カードの `block:` と違う block_type を書くとエラー。一致すれば通る。
    #[test]
    fn task_create_with_card_rejects_different_block_type() {
        let guard = EnvGuard::new();
        std::env::set_var("AGENT_LINK_PARENT_RUN_ID", "42");
        write_test_block(&guard.project_root, "card-block");
        write_test_block(&guard.project_root, "other-block");
        write_todo_card(&guard.project_root, 120, "card-block", "professor", "説明", "本文");

        let tasks = json!([
            { "client_id": "R1", "subject": "件名", "card": 120, "block_type": "other-block" },
        ]);
        let err = task_create(&tasks, None, None, None).unwrap_err();
        assert!(
            err.contains("Make block_type match the block of the card"),
            "err={}",
            err
        );

        // 一致していれば通る
        let tasks = json!([
            { "client_id": "R2", "subject": "件名", "card": 120, "block_type": "card-block" },
        ]);
        assert_eq!(task_create(&tasks, None, None, None).unwrap()["status"], "approved");

        drop(guard);
    }

    /// U2: 同じカードを 2 度起こせない（既に未完了タスクがあればエラー）。
    /// 前のタスクが確定（finalized）すれば再び起こせる。
    #[test]
    fn task_create_with_card_rejects_duplicate_start() {
        let guard = EnvGuard::new();
        let store = crate::common::open_db().unwrap();
        let parent_run_id = create_test_parent_run(&store, "card-parent-dup");
        std::env::set_var("AGENT_LINK_PARENT_RUN_ID", &parent_run_id.to_string());
        write_test_block(&guard.project_root, "card-block");
        write_todo_card(&guard.project_root, 120, "card-block", "professor", "説明", "本文");

        let tasks = json!([{ "client_id": "R1", "card": 120 }]);
        assert_eq!(task_create(&tasks, None, None, None).unwrap()["status"], "approved");

        // デーモンがタスクを作る
        let notif = read_task_created_notification();
        let first_task_id = {
            let manager = crate::task_manager::TaskManager::new(std::sync::Arc::new(store));
            let ids = manager.process_task_created(&notif).unwrap();
            assert_eq!(ids.len(), 1);
            ids[0]
        };

        // もう一度同じカードで起こそうとするとエラー
        let tasks = json!([{ "client_id": "R2", "card": 120 }]);
        let err = task_create(&tasks, None, None, None).unwrap_err();
        assert!(
            err.contains(&format!("Card 120 was already started as task {}", first_task_id)),
            "err={}",
            err
        );

        // 前のタスクが確定（finalized）すれば再び起こせる
        let store = crate::common::open_db().unwrap();
        store.update_task_status(first_task_id, "finalized").unwrap();
        assert_eq!(
            task_create(&tasks, None, None, None).unwrap()["status"],
            "approved"
        );

        drop(guard);
    }

    /// cwd 指定が絶対パスに解決され、通知 payload と結果に載ること。
    #[test]
    fn task_create_resolves_cwd_to_absolute_path() {
        let guard = EnvGuard::new();
        std::env::set_var("AGENT_LINK_PARENT_RUN_ID", "42");
        let sub = guard.project_root.join("sub/dir");
        std::fs::create_dir_all(&sub).unwrap();

        let tasks = json!([
            { "client_id": "t1", "subject": "件名", "task_text": "説明", "cwd": "sub/dir" },
            { "client_id": "t2", "subject": "件名2", "task_text": "説明2" },
        ]);
        let result = task_create(&tasks, None, None, None).unwrap();
        assert_eq!(result["status"], "approved");

        // 通知 payload の cwd が絶対パスになっていること
        let pending = crate::common::notify_pending_dir();
        let entries: Vec<_> = std::fs::read_dir(&pending).unwrap().flatten().collect();
        assert_eq!(entries.len(), 1);
        let value: Value =
            serde_json::from_str(&std::fs::read_to_string(entries[0].path()).unwrap()).unwrap();
        let tasks_json = value["tasks"].as_array().unwrap();
        let t1 = tasks_json.iter().find(|t| t["client_id"] == "t1").unwrap();
        assert_eq!(t1["cwd"].as_str().unwrap(), sub.to_string_lossy());
        let t2 = tasks_json.iter().find(|t| t["client_id"] == "t2").unwrap();
        assert_eq!(t2["cwd"], Value::Null, "未指定の cwd は null");

        drop(guard);
    }

    /// 存在しない cwd は validation_errors で返し、通知を書かないこと。
    #[test]
    fn task_create_rejects_missing_cwd_without_notification() {
        let guard = EnvGuard::new();
        std::env::set_var("AGENT_LINK_PARENT_RUN_ID", "42");

        let tasks = json!([
            { "client_id": "t1", "subject": "件名", "task_text": "説明", "cwd": "no-such-folder-xyz" },
        ]);
        let result = task_create(&tasks, None, None, None).unwrap();
        assert_eq!(result["status"], "failed");
        let errors = result["validation_errors"].as_array().unwrap();
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0]["field"], "cwd");
        assert_eq!(errors[0]["client_id"], "t1");

        let pending = crate::common::notify_pending_dir();
        let count = std::fs::read_dir(&pending)
            .map(|d| d.flatten().count())
            .unwrap_or(0);
        assert_eq!(count, 0, "検証失敗時は通知を書かない");

        drop(guard);
    }

    /// cwd にファイル（フォルダでない）を指定しても validation_errors になること。
    #[test]
    fn task_create_rejects_file_cwd() {
        let guard = EnvGuard::new();
        std::env::set_var("AGENT_LINK_PARENT_RUN_ID", "42");
        let file = guard.project_root.join("a.txt");
        std::fs::write(&file, "x").unwrap();

        let tasks = json!([
            { "client_id": "t1", "subject": "件名", "task_text": "説明", "cwd": file.to_string_lossy() },
        ]);
        let result = task_create(&tasks, None, None, None).unwrap();
        assert_eq!(result["status"], "failed");
        assert!(result["validation_errors"][0]["message"]
            .as_str()
            .unwrap()
            .contains("is not a folder"));

        drop(guard);
    }

    /// host 付きの子は、Mac に無い遠隔の cwd（`~/` 始まり）を受け付け、通知 payload に host・cwd をそのまま載せること。
    #[test]
    fn task_create_accepts_host_with_remote_cwd() {
        let guard = EnvGuard::new();
        std::env::set_var("AGENT_LINK_PARENT_RUN_ID", "42");

        let tasks = json!([
            { "client_id": "W1", "subject": "W1", "task_text": "調べる",
              "host": " wsl-server ", "cwd": "~/work/sai-tree" },
            { "client_id": "W2", "subject": "W2", "task_text": "調べる",
              "host": "wsl-server", "cwd": "/home/u/proj", "worktree": "none" },
            { "client_id": "L1", "subject": "L1", "task_text": "Mac で動く", "host": "  " },
        ]);
        let result = task_create(&tasks, None, None, None).unwrap();
        assert_eq!(result["status"], "approved");

        let pending = crate::common::notify_pending_dir();
        let entries: Vec<_> = std::fs::read_dir(&pending).unwrap().flatten().collect();
        assert_eq!(entries.len(), 1);
        let value: Value =
            serde_json::from_str(&std::fs::read_to_string(entries[0].path()).unwrap()).unwrap();
        let tasks_json = value["tasks"].as_array().unwrap();
        let find = |id: &str| tasks_json.iter().find(|t| t["client_id"] == id).unwrap();
        assert_eq!(find("W1")["host"], "wsl-server");
        assert_eq!(find("W1")["cwd"], "~/work/sai-tree");
        assert_eq!(find("W2")["cwd"], "/home/u/proj");
        assert_eq!(find("L1")["host"], Value::Null, "空の host は Mac");
        assert_eq!(find("L1")["cwd"], Value::Null);

        drop(guard);
    }

    /// host 付きで cwd 無し・形の誤り・worktree 指定の組み合わせは、それぞれの文で断ること。
    #[test]
    fn task_create_rejects_bad_host_combinations() {
        let guard = EnvGuard::new();
        std::env::set_var("AGENT_LINK_PARENT_RUN_ID", "42");
        let create = |extra: Value| {
            let mut item = json!({ "client_id": "W1", "subject": "W1", "task_text": "x", "host": "wsl-server" });
            for (k, v) in extra.as_object().unwrap() {
                item[k] = v.clone();
            }
            task_create(&json!([item]), None, None, None).unwrap_err()
        };

        assert_eq!(
            create(json!({})),
            "client_id 'W1': cwd is required when host is set (a folder on the remote machine)."
        );
        for bad in ["relative/path", "/a/../b", "~", "/a'b"] {
            assert_eq!(
                create(json!({ "cwd": bad })),
                format!("client_id 'W1': cwd with host must be an absolute path or start with ~/ on the remote machine: {}", bad)
            );
        }
        assert_eq!(
            create(json!({ "cwd": "~/x", "worktree": "isolation" })),
            "client_id 'W1': worktree \"isolation\" cannot be used with host. Leave it empty or use \"none\"."
        );

        let pending = crate::common::notify_pending_dir();
        let count = std::fs::read_dir(&pending).map(|d| d.flatten().count()).unwrap_or(0);
        assert_eq!(count, 0, "断った入力は通知を書かない");

        drop(guard);
    }

    /// `task_to_json` に host が出て、host があると worktree_info は null になること。
    #[test]
    fn task_to_json_shows_host_and_nulls_worktree_info() {
        let mut t = Task::new(
            1,
            "件名".to_string(),
            "説明".to_string(),
            None,
            None,
            None,
            "".to_string(),
            None,
            Some("W1".to_string()),
        );
        let local = task_to_json(&t, &[]);
        assert_eq!(local["host"], Value::Null);
        assert!(local["worktree_info"].is_object());

        t.host = Some("wsl-server".to_string());
        t.cwd = Some("~/work/sai-tree".to_string());
        let remote = task_to_json(&t, &[]);
        assert_eq!(remote["host"], "wsl-server");
        assert_eq!(remote["cwd"], "~/work/sai-tree");
        assert_eq!(remote["worktree_info"], Value::Null);
    }

    #[test]
    fn task_manage_rejects_invalid_action() {        let err = task_manage("invalid", 1, None, Some(1)).unwrap_err();
        assert!(err.contains("Invalid action"));
    }

    /// U3 AC1: `task_create` が `timeout`（秒）を受け付け、通知 payload に載せること。
    #[test]
    fn task_create_accepts_timeout() {
        let guard = EnvGuard::new();
        std::env::set_var("AGENT_LINK_PARENT_RUN_ID", "42");

        let tasks = json!([
            { "client_id": "t1", "subject": "件名", "task_text": "説明", "timeout": 600 },
            { "client_id": "t2", "subject": "件名2", "task_text": "説明2" },
        ]);
        let result = task_create(&tasks, None, None, None).unwrap();
        assert_eq!(result["status"], "approved");

        let pending = crate::common::notify_pending_dir();
        let entries: Vec<_> = std::fs::read_dir(&pending).unwrap().flatten().collect();
        assert_eq!(entries.len(), 1);
        let value: Value =
            serde_json::from_str(&std::fs::read_to_string(entries[0].path()).unwrap()).unwrap();
        let tasks_json = value["tasks"].as_array().unwrap();
        let t1 = tasks_json.iter().find(|t| t["client_id"] == "t1").unwrap();
        assert_eq!(t1["timeout"].as_u64(), Some(600));
        let t2 = tasks_json.iter().find(|t| t["client_id"] == "t2").unwrap();
        assert_eq!(t2["timeout"], Value::Null, "未指定の timeout は null");

        drop(guard);
    }

    /// U3 AC2: 1 未満・非整数の `timeout` は Err になり、通知を書かないこと。
    #[test]
    fn task_create_rejects_invalid_timeout() {
        let guard = EnvGuard::new();
        std::env::set_var("AGENT_LINK_PARENT_RUN_ID", "42");

        for bad in [json!(0), json!(-5), json!("abc"), json!(1.5)] {
            let tasks = json!([
                { "client_id": "t1", "subject": "件名", "task_text": "説明", "timeout": bad },
            ]);
            let err = task_create(&tasks, None, None, None).unwrap_err();
            assert!(
                err.contains("timeout must be an integer of 1 or more (seconds)"),
                "timeout={} の エラーメッセージ: {}",
                bad,
                err
            );

            let pending = crate::common::notify_pending_dir();
            let count = std::fs::read_dir(&pending)
                .map(|d| d.flatten().count())
                .unwrap_or(0);
            assert_eq!(count, 0, "不正な timeout のときは通知を書かない");
        }

        drop(guard);
    }

    #[test]
    fn task_manage_send_requires_message() {
        let err = task_manage("send", 1, None, Some(1)).unwrap_err();
        assert!(err.contains("message"));
    }

    /// U1 AC3: send_route の振り分け表を全行確かめる（herdr には触れない）。
    #[test]
    fn send_route_table() {
        // in_progress + idle / done → pane に直接送る
        assert_eq!(
            send_route(12, "in_progress", Some("idle")),
            Ok(SendRoute::DirectToPane)
        );
        assert_eq!(
            send_route(12, "in_progress", Some("done")),
            Ok(SendRoute::DirectToPane)
        );

        // in_progress + working → エラー（自動で esc はしない）
        let err = send_route(12, "in_progress", Some("working")).unwrap_err();
        assert!(err.contains("is working"), "{}", err);
        assert!(err.contains("agent-link task interrupt <ID>"), "{}", err);

        // in_progress + blocked → 質問の答え待ち（interrupt は勧めない）
        let err = send_route(12, "in_progress", Some("blocked")).unwrap_err();
        assert!(err.contains("waiting for the user's answer"), "{}", err);
        assert!(err.contains("agent-link task wait <ID>"), "{}", err);

        // in_progress + error / 未知の値 / None → エラー
        for st in [Some("error"), Some("weird"), None] {
            let err = send_route(12, "in_progress", st).unwrap_err();
            assert!(err.contains("is stopped at"), "{}", err);
            assert!(err.contains("agent-link task interrupt <ID>"), "{}", err);
        }
        // None のときは「unknown state」と出る
        assert!(
            send_route(12, "in_progress", None)
                .unwrap_err()
                .contains("unknown state"),
            "agent_status が取れないときは unknown state と表示する"
        );

        // finalized / deleted → エラー
        for st in ["finalized", "deleted"] {
            let err = send_route(12, st, None).unwrap_err();
            assert!(err.contains(st), "{}", err);
            assert!(err.contains("requeue"), "{}", err);
        }

        // completed / approved / failed などは従来どおりデーモン経由
        for st in ["completed", "approved", "failed", "pending", "queued"] {
            assert_eq!(
                send_route(12, st, None),
                Ok(SendRoute::Daemon),
                "{} は Daemon 経由",
                st
            );
        }
    }

    /// U1 AC4: interrupt は in_progress 以外のタスクで Err を返す（herdr を呼ばない）。
    #[test]
    fn task_manage_interrupt_requires_in_progress() {
        let guard = EnvGuard::new();

        let store = crate::common::open_db().unwrap();
        let block = crate::db::models::test_helpers::create_test_block("agent-api-interrupt-block");
        let block_id = store.insert_block(&block).unwrap();
        let parent_run_id = store.create_execution(block_id, None).unwrap();
        let exec_id = store
            .create_execution_for_subtask(block_id, None, parent_run_id, None)
            .unwrap();
        let task = crate::db::models::Task::new(
            parent_run_id,
            "件名".to_string(),
            "説明".to_string(),
            Some("test-block".to_string()),
            None,
            None,
            String::new(),
            None,
            Some("t-int".to_string()),
        );
        let task_id = store.insert_task(&task).unwrap();
        store.update_task_execution_id(task_id, exec_id).unwrap();
        // 初期状態（pending）のまま = in_progress ではない
        let err = task_manage("interrupt", task_id, None, Some(parent_run_id)).unwrap_err();
        assert!(
            err.contains("Only a working (in_progress) task can be interrupted"),
            "{}",
            err
        );
        assert!(err.contains("pending"), "{}", err);

        // completed でも同じエラー
        store.update_task_status(task_id, "completed").unwrap();
        let err2 = task_manage("interrupt", task_id, None, Some(parent_run_id)).unwrap_err();
        assert!(err2.contains("completed"), "{}", err2);

        drop(guard);
    }

    #[test]
    fn require_run_id_prefers_explicit_then_env_then_errors() {
        let _guard = EnvGuard::new();

        std::env::remove_var("AGENT_LINK_PARENT_RUN_ID");
        assert_eq!(require_run_id(Some(7)).unwrap(), 7);

        std::env::set_var("AGENT_LINK_PARENT_RUN_ID", "9");
        assert_eq!(require_run_id(None).unwrap(), 9);

        std::env::remove_var("AGENT_LINK_PARENT_RUN_ID");
        assert!(require_run_id(None).is_err());
    }

    // ------------------------------------------------------------------
    // P6: task_report / task_output の簡素化
    // ------------------------------------------------------------------

    /// テスト用に DB へブロックと実行を作り、その run_id を返す。
    /// `is_subtask` が true なら、メイン実行を親としてサブブロック実行を作る。
    fn create_test_execution(is_subtask: bool) -> i64 {
        let store = crate::common::open_db().unwrap();
        let block = crate::db::models::test_helpers::create_test_block("agent-api-test-block");
        let block_id = store.insert_block(&block).unwrap();
        if is_subtask {
            // parent_run_id は execution_history への FK なので、実在する親を作る
            let parent = store.create_execution(block_id, None).unwrap();
            store
                .create_execution_for_subtask(block_id, None, parent, None)
                .unwrap()
        } else {
            store.create_execution(block_id, None).unwrap()
        }
    }

    /// AC7: task_report は finalize-report.md を保存し、finalize_request を書く。
    /// （waggle トークン化はしない = file_written 通知を書かない）
    #[test]
    fn task_report_saves_report_and_writes_finalize_request() {
        let guard = EnvGuard::new();
        let run_id = create_test_execution(true);

        let result = task_report("結果: DONE\nコミット: abc1234", Some(run_id)).unwrap();
        assert_eq!(result["status"], "reported");
        assert_eq!(result["run_id"], run_id);

        // run フォルダの finalize-report.md に保存される
        let (_, dir) = crate::run_dir::find_run_dir(run_id).expect("run フォルダが作られる");
        let saved = std::fs::read_to_string(dir.join("finalize-report.md")).unwrap();
        assert!(saved.contains("DONE"));

        // pending/ の通知を確認する（finalize_request あり・file_written なし）
        let pending = crate::common::notify_pending_dir();
        let values: Vec<Value> = std::fs::read_dir(&pending)
            .unwrap()
            .flatten()
            .filter_map(|e| std::fs::read_to_string(e.path()).ok())
            .filter_map(|t| serde_json::from_str::<Value>(&t).ok())
            .collect();
        assert!(
            values
                .iter()
                .any(|v| v["type"] == "finalize_request" && v["run_id"] == run_id),
            "finalize_request が書かれる"
        );
        assert!(
            !values.iter().any(|v| v["type"] == "file_written"),
            "task_report の保存は waggle トークン化しない"
        );

        drop(guard);
    }

    /// AC7: メインブロック（parent_run_id なし）では task_report は Err。
    #[test]
    fn task_report_rejects_main_block() {
        let guard = EnvGuard::new();
        let run_id = create_test_execution(false);
        let err = task_report("報告", Some(run_id)).unwrap_err();
        assert!(err.contains("for subtasks only"), "err={}", err);
        drop(guard);
    }

    #[test]
    fn task_report_rejects_empty_report() {
        let guard = EnvGuard::new();
        let run_id = create_test_execution(true);
        let err = task_report("   ", Some(run_id)).unwrap_err();
        assert!(err.contains("report"), "err={}", err);
        drop(guard);
    }

    /// AC7: task_output の completed は task_id / status / round / report / outputs だけを返す。
    #[test]
    fn task_output_completed_returns_report_and_outputs_only() {
        let guard = EnvGuard::new();

        // サブタスク実行と、実行 ID 紐付きのタスクを作る
        let store = crate::common::open_db().unwrap();
        let block = crate::db::models::test_helpers::create_test_block("agent-api-output-block");
        let block_id = store.insert_block(&block).unwrap();
        // tasks.parent_run_id は execution_history への FK なので実在する親を作る
        let parent_run_id = store.create_execution(block_id, None).unwrap();
        let exec_id = store
            .create_execution_for_subtask(block_id, None, parent_run_id, None)
            .unwrap();
        let task = crate::db::models::Task::new(
            parent_run_id,
            "件名".to_string(),
            "説明".to_string(),
            Some("test-block".to_string()),
            None,
            None,
            String::new(),
            None,
            Some("t1".to_string()),
        );
        let task_id = store.insert_task(&task).unwrap();
        store.update_task_execution_id(task_id, exec_id).unwrap();

        // run フォルダに task_report の報告を置く
        let dir = crate::run_dir::ensure_run_dir(crate::run_dir::RUNNING_OWNER, exec_id).unwrap();
        std::fs::write(dir.join("finalize-report.md"), "結果: DONE").unwrap();

        // results/ に task_completed 通知を置く
        let results = crate::common::notify_results_dir();
        std::fs::create_dir_all(&results).unwrap();
        std::fs::write(
            results.join(format!("1-task_{}.json", task_id)),
            serde_json::to_string(&json!({
                "type": "task_completed",
                "task_id": task_id,
                "parent_run_id": parent_run_id,
                "waggle_tokens": ["FAKE1"],
                "round": 2,
                "commit_id": "abc1234",
                "elapsed_ms": 1234,
                "session_stats": { "turns": 3 },
            }))
            .unwrap(),
        )
        .unwrap();

        let result = task_output(task_id, Some(0)).unwrap();
        assert_eq!(result["task_id"], task_id);
        assert_eq!(result["status"], "completed");
        assert_eq!(result["round"], 2);
        assert_eq!(result["report"], "結果: DONE");
        assert_eq!(result["outputs"][0], "waggle:FAKE1: (no description)");
        // 削除した項目が返らないこと
        assert!(result.get("resolved_content").is_none());
        assert!(result.get("waggle_tokens").is_none());
        assert!(result.get("commit_id").is_none());
        assert!(result.get("elapsed_ms").is_none());
        assert!(result.get("session_stats").is_none());
        // B7: 完了には閉じる案内（hint）が付く（status / report / outputs の形は変えない）
        let hint = result["hint"].as_str().expect("hint が返る");
        assert!(hint.contains("agent-link task finalize <ID>"), "{}", hint);

        drop(guard);
    }

    /// テスト用に親実行と、指定した件数のタスクを作る。
    /// 戻り値はタスクの id（作った順）。
    fn create_tasks(tag: &str, count: usize) -> (Vec<i64>, i64) {
        let store = crate::common::open_db().unwrap();
        let block = crate::db::models::test_helpers::create_test_block(tag);
        let block_id = store.insert_block(&block).unwrap();
        // tasks.parent_run_id は execution_history への FK なので実在する親を作る
        let parent_run_id = store.create_execution(block_id, None).unwrap();
        let mut ids = Vec::new();
        for n in 1..=count {
            let exec_id = store
                .create_execution_for_subtask(block_id, None, parent_run_id, None)
                .unwrap();
            let task = crate::db::models::Task::new(
                parent_run_id,
                format!("件名{}", n),
                "説明".to_string(),
                Some("test-block".to_string()),
                None,
                None,
                String::new(),
                None,
                Some(format!("t{}", n)),
            );
            let task_id = store.insert_task(&task).unwrap();
            store.update_task_execution_id(task_id, exec_id).unwrap();
            ids.push(task_id);
        }
        (ids, parent_run_id)
    }

    /// `results/` に `task_completed` の通知を 1 件置いて、パスを返す。
    fn write_task_completed_result(task_id: i64, parent_run_id: i64) -> PathBuf {
        let results = crate::common::notify_results_dir();
        std::fs::create_dir_all(&results).unwrap();
        let path = results.join(format!("{}-task_{}.json", unix_millis(), task_id));
        std::fs::write(
            &path,
            serde_json::to_string(&json!({
                "type": "task_completed",
                "task_id": task_id,
                "parent_run_id": parent_run_id,
                "waggle_tokens": [],
                "round": 1,
            }))
            .unwrap(),
        )
        .unwrap();
        path
    }

    /// AC2: 待ちは 90 秒で頭打ち。120 秒（MCP 呼び出しをバックグラウンドに回す限界）より短い。
    #[test]
    fn clamp_wait_ms_caps_at_90_seconds() {
        assert!(TASK_OUTPUT_MAX_WAIT_MS < 120_000);
        assert_eq!(clamp_wait_ms(None), 90_000);
        assert_eq!(clamp_wait_ms(Some(10_000_000)), 90_000);
        assert_eq!(clamp_wait_ms(Some(0)), 0);
        assert_eq!(clamp_wait_ms(Some(1234)), 1234);
    }

    /// AC3: task_ids の後ろの子が終わっていたら、その子を 1 件だけ返し残りは pending になる。
    /// 返した子の結果ファイルは消える（次に待つ必要がない）。
    #[test]
    fn task_output_ids_returns_first_finished_and_the_rest_pending() {
        let guard = EnvGuard::new();
        let (ids, parent_run_id) = create_tasks("agent-api-multi-block", 2);
        let (a, b) = (ids[0], ids[1]);
        let path = write_task_completed_result(b, parent_run_id);

        let cancel = AtomicBool::new(false);
        let out = task_output_ids(&[a, b], Some(3000), &cancel, true).unwrap();
        assert_eq!(out["task_id"], b);
        assert_eq!(out["status"], "completed");
        assert_eq!(out["pending"], json!([a]));
        assert!(!path.exists(), "返した子の結果ファイルは消える");

        drop(guard);
    }

    /// 複数待ちで誰も終わらないと、全部が pending で返る。
    ///
    /// 待つ対象は作らない（作ると `AGENT_LINK_PROJECT_ROOT` を持つロックを長く抱え、
    /// 環境の取り合いで他のテストが落ちるため）。誰も居ないことは大きなタスク番号で確認する。
    #[test]
    fn task_output_ids_timeout_lists_all_pending() {
        let cancel = AtomicBool::new(false);
        let ids = [NO_SUCH_TASK_A, NO_SUCH_TASK_B];

        let started = std::time::Instant::now();
        let out = task_output_ids(&ids, Some(200), &cancel, true).unwrap();
        assert_eq!(out["status"], "in_progress");
        assert_eq!(out["pending"], json!(ids));
        assert!(out.get("task_id").is_none(), "複数待ちでは task_id を付けない");
        let message = out["message"].as_str().unwrap();
        assert!(message.contains("at most 90 seconds"), "{}", message);
        assert!(started.elapsed() < std::time::Duration::from_secs(3));
    }

    /// AC4: 待ちのあいだに取り消しが来たら待たずに `in_progress` で返る。
    /// 誰かの結果があっても良いので、待つ対象は作らない（ロックを短く保つため）。
    #[test]
    fn task_output_ids_cancel_stops_waiting_midway() {
        let cancel = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&cancel);
        let handle = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(100));
            flag.store(true, Ordering::Relaxed);
        });

        let started = std::time::Instant::now();
        let out =
            task_output_ids(&[NO_SUCH_TASK_A, NO_SUCH_TASK_B], Some(60_000), cancel.as_ref(), true)
                .unwrap();
        handle.join().unwrap();

        assert_eq!(out["status"], "in_progress");
        let message = out["message"].as_str().unwrap();
        assert!(message.contains("The wait was cancelled"), "{}", message);
        assert!(message.contains("at most 90 seconds"), "{}", message);
        assert!(
            started.elapsed() < std::time::Duration::from_secs(3),
            "取り消しで早く返る: {:?}",
            started.elapsed()
        );
    }

    /// AC4: 取り消しで抜けたときは結果ファイルを消さない（あとから読み直せる）。
    /// 取り消しは結果を読むより先に確かめるので、完了待ちでもファイルは残る。
    #[test]
    fn task_output_ids_cancelled_keeps_result_file() {
        let guard = EnvGuard::new();
        // 待つ対象の 1 件目に結果を先に置いておく
        let path = write_task_completed_result(NO_SUCH_TASK_B, 0);

        let cancel = AtomicBool::new(true); // 最初から取り消し済み
        let out = task_output_ids(&[NO_SUCH_TASK_B], Some(60_000), &cancel, true).unwrap();
        assert_eq!(out["status"], "in_progress");
        assert!(path.exists(), "取り消したときは結果ファイルを残す");

        drop(guard);
    }

    /// 後方互換: 明示した待ち時間が切れたときは今まで通り timeout。
    #[test]
    fn task_output_timeout_status_stays_timeout() {
        let cancel = AtomicBool::new(false);
        let out = task_output_ids(&[NO_SUCH_TASK_A], Some(200), &cancel, false).unwrap();
        assert_eq!(out["task_id"], NO_SUCH_TASK_A);
        assert_eq!(out["status"], "timeout");
        let error = out["error"].as_str().unwrap();
        assert!(error.contains("at most 90 seconds"), "{}", error);
    }

    /// 上限で切った待ち（90 秒以上を要求されたとき、または timeout_ms を省いたとき）は
    /// in_progress を返す。90 秒は待たず、文の組み立てだけを確かめる。
    #[test]
    fn timeout_result_reports_in_progress_when_wait_was_capped() {
        let single = timeout_result(&[7], TASK_OUTPUT_MAX_WAIT_MS, true, false);
        assert_eq!(single["task_id"], 7);
        assert_eq!(single["status"], "in_progress");
        assert!(single.get("error").is_none(), "in_progress では error にしない");
        assert!(single["message"].as_str().unwrap().contains("at most 90 seconds"));

        let multiple = timeout_result(&[7, 8], TASK_OUTPUT_MAX_WAIT_MS, true, true);
        assert_eq!(multiple["status"], "in_progress");
        assert_eq!(multiple["pending"], json!([7, 8]));
        assert!(multiple.get("task_id").is_none());
    }

    /// 1 件も指定されなければエラー（待つ対象が居ない）。
    #[test]
    fn task_output_ids_rejects_empty_list() {
        let err = task_output_ids(&[], Some(0), &AtomicBool::new(false), true).unwrap_err();
        assert!(err.contains("at least one ID"), "err={}", err);
    }
    /// 付けないと、すでに保存済みの報告は書き換えられない。
    #[test]
    fn finalize_saves_report_only_when_given() {
        let guard = EnvGuard::new();
        let run_id = create_test_execution(false);
        crate::common::open_db()
            .unwrap()
            .update_execution_status(run_id, "running")
            .unwrap();

        // report 付きで完了 → finalize-report.md ができる
        let result = finalize(Some(run_id), Some("結果: DONE\n Waggle: ABC123")).unwrap();
        assert_eq!(result["status"], "requested");
        let path = report_path_for_run(run_id).expect("run フォルダがある");
        let saved = std::fs::read_to_string(&path).unwrap();
        assert!(saved.contains("DONE"));
        assert!(saved.contains("Waggle: ABC123"));

        // report 無しで完了 → 保存済みの報告を消さない・書き換えもしない
        finalize(Some(run_id), None).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), saved);
        // 空白だけの report も「無し」と同じ
        finalize(Some(run_id), Some("   ")).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), saved);

        drop(guard);
    }

    /// 無い run は「not found」（3）、実行中でない run は「not running」（1）で弾く。
    /// 弾いたときは report も書かない。
    #[test]
    fn finalize_rejects_missing_and_not_running_run() {
        let guard = EnvGuard::new();
        let err = finalize(Some(999_999), Some("x")).unwrap_err();
        assert_eq!(err, "Run 999999 not found.");

        let run_id = create_test_execution(false);
        let store = crate::common::open_db().unwrap();
        store.update_execution_status(run_id, "completed").unwrap();
        let err = finalize(Some(run_id), Some("x")).unwrap_err();
        assert_eq!(err, format!("Run {} is not running (status: completed).", run_id));
        assert!(report_path_for_run(run_id).map_or(true, |p| !p.exists()));

        drop(guard);
    }

    /// AC4: 子の run の中から task_question を呼ぶと、親の task_output が
    /// reason = "question" と question_id 付きの stalled を返す。
    #[test]
    fn task_question_notifies_parent_with_question_id() {
        let guard = EnvGuard::new();

        // 親実行 → 子の実行 → その実行に紐づくタスク
        let store = crate::common::open_db().unwrap();
        let block = crate::db::models::test_helpers::create_test_block("agent-api-question-block");
        let block_id = store.insert_block(&block).unwrap();
        let parent_run_id = store.create_execution(block_id, None).unwrap();
        let exec_id = store
            .create_execution_for_subtask(block_id, None, parent_run_id, None)
            .unwrap();
        let task = crate::db::models::Task::new(
            parent_run_id,
            "件名".to_string(),
            "説明".to_string(),
            Some("test-block".to_string()),
            None,
            None,
            String::new(),
            None,
            Some("q1".to_string()),
        );
        let task_id = store.insert_task(&task).unwrap();
        store.update_task_execution_id(task_id, exec_id).unwrap();

        let result = task_question(Some(exec_id), "q1", "資料はどれですか\n  1) A  2) B").unwrap();
        assert_eq!(result["status"], "notified");
        assert_eq!(result["task_id"], task_id);
        assert_eq!(result["question_id"], "q1");

        // 親の task_output は question の stalled を返す
        let out = task_output(task_id, Some(0)).unwrap();
        assert_eq!(out["status"], "stalled");
        assert_eq!(out["reason"], "question");
        assert_eq!(out["question_id"], "q1");
        // 質問の文は 1 行に畳まれる
        let msg = out["message"].as_str().unwrap();
        assert!(msg.contains("Question q1:"), "{}", msg);
        assert!(!msg.contains('\n'), "{}", msg);
        // 質問用の案内（agent-link task capture <ID> で止めない）
        let hint = out["hint"].as_str().unwrap();
        assert!(hint.contains("question"), "{}", hint);

        drop(guard);
    }

    /// AC4: 親のない run では task-question は Err。
    #[test]
    fn task_question_rejects_main_block() {
        let guard = EnvGuard::new();
        let run_id = create_test_execution(false);
        let err = task_question(Some(run_id), "q1", "質問").unwrap_err();
        assert!(err.contains("task-question is for subtask runs only"), "err={}", err);
        drop(guard);
    }

    /// AC4: 質問 ID が空なら Err（通知を書かない）。
    #[test]
    fn task_question_rejects_empty_question_id() {
        let guard = EnvGuard::new();
        let run_id = create_test_execution(true);
        let err = task_question(Some(run_id), "  ", "質問").unwrap_err();
        assert!(err.contains("question_id"), "err={}", err);
        drop(guard);
    }
}
