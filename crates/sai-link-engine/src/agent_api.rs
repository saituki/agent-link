//! # エージェント共通 API（pi / cc 共通の実体）
//!
//! pi/cc 共通 MCP サーバ（`mcp_server.rs`・`sai-link mcp serve` の `sai`）から
//! 呼ばれる機能の唯一の実装です。pi は `pi-mcp-adapter` が `.mcp.json` を読み
//! `mcp__sai__*` として呼び、cc は `.mcp.json` + `.claude/settings.json` で呼ぶ。
//! 旧 pi-sai-link/（TypeScript 拡張・`sai-link agent` 薄皮）は廃止し削除済み。
//!
//! すべての関数は `Result<serde_json::Value, String>` を返します。
//! `Err` の文字列はそのままエージェントへ見せる日本語のメッセージです。
//!
//! ツール承認機能（task_create / task_manage のユーザー確認 UI）は廃止済みです。
//! `task_create` は受け取った全タスクを常に承認済みとして作成します。

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::{json, Value};

use crate::block_config::BlockConfig;
use crate::db::models::{ExecutionHistory, Task};
use crate::parser::parse_block_file;
use crate::task_manager::TaskManager;

// ============================================================================
// 共通ヘルパー
// ============================================================================

/// `SAI_PARENT_RUN_ID` を解決する。
fn env_run_id() -> Option<i64> {
    std::env::var("SAI_PARENT_RUN_ID")
        .ok()
        .and_then(|v| v.trim().parse::<i64>().ok())
}

/// 実行 ID を解決する。引数優先、無ければ環境変数 `SAI_PARENT_RUN_ID`。
/// 両方無ければエラー。
fn require_run_id(explicit: Option<i64>) -> Result<i64, String> {
    explicit
        .or_else(env_run_id)
        .ok_or_else(|| "SAI_PARENT_RUN_ID が設定されていません（run 実行中ではありません）".to_string())
}

/// ブロック名を解決する（`SAI_BLOCK_NAME`、無ければ `"cc"`）。
fn block_name() -> String {
    std::env::var("SAI_BLOCK_NAME").unwrap_or_else(|_| "cc".to_string())
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

/// 拡張子から MIME タイプを推測する（pi-sai の `guessMimeType` と同等）。
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
    std::fs::create_dir_all(&dir).map_err(|e| format!("通知ディレクトリ作成に失敗: {}", e))?;

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
        .map_err(|e| format!("通知のシリアライズに失敗: {}", e))?;
    std::fs::write(&path, text).map_err(|e| format!("通知の書き込みに失敗: {}", e))?;
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
    /// 表示名（sai-link-tui 等の表示用。emoji 可）
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
/// 探索順: `PROJECTROOT/.sai/blocks` → `blocks`（開発用 cwd）→ `lib/herdrs/blocks`。
pub fn collect_block_definitions() -> Vec<(PathBuf, BlockConfig)> {
    let mut out: Vec<(PathBuf, BlockConfig)> = Vec::new();
    let dirs = [
        crate::project::sai_path(".sai/blocks"),
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
// task list --json 出力用エントリ（pi-sai の task_list / task_status が使用）
// ============================================================================

/// `task list --json` の出力用エントリ（pi-sai の `task_list` / `task_status` が使用）。
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
    let store = Arc::new(crate::common::open_db().map_err(|e| format!("DB オープンに失敗しました: {}", e))?);
    let manager = TaskManager::new(store);
    let mut tasks = manager
        .list_tasks(run_id)
        .map_err(|e| format!("タスク一覧取得エラー: {}", e))?;
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
    let trimmed = filename.trim();
    if trimmed.is_empty() {
        return Err("filename は必須です".to_string());
    }
    // パス区切りを含む filename は run フォルダ直下の basename のみ使う
    let basename = Path::new(trimmed)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .ok_or("filename が不正です")?;

    let run_id = require_run_id(run_id)?;

    let path = crate::sai_write_notify::resolve_output_path(run_id, &basename)
        .map_err(|e| format!("保存先の解決に失敗しました: {}", e))?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("保存先作成に失敗: {}", e))?;
    }
    std::fs::write(&path, content).map_err(|e| format!("ファイル書き込みに失敗: {}", e))?;
    let byte_size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);

    // P4: エージェント側でその場でトークン化する。失敗したらデーモンの非同期処理に任せる。
    let token = tokenize_saved_file(&path, description);

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

    Ok(match token {
        Some(tok) => json!({
            "status": "ok",
            "path": path.to_string_lossy(),
            "byte_size": byte_size,
            "token": tok,
            "ref": format!("waggle:{}:path:.sai/outputs/*/{}/{}", tok, run_id, basename),
            "message": "保存しました。他の文書から参照するときは ref の形で書いてください。",
        }),
        None => json!({
            "status": "ok",
            "path": path.to_string_lossy(),
            "byte_size": byte_size,
            "token": Value::Null,
            "ref": Value::Null,
            "message": "保存しました。waggle トークン化はデーモンが非同期で行います。",
        }),
    })
}

/// 保存直後のファイルをその場で waggle トークン化し、トークン文字列を返す（P4）。
///
/// DB を開けない・トークン化に失敗した場合は `None` を返し、
/// これまでどおりデーモンの非同期トークン化に任せる。
fn tokenize_saved_file(path: &Path, description: Option<&str>) -> Option<String> {
    let db_path = crate::project::sai_path(".sai/sai-engine/sai-engine.db");
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

/// `finalize_request` 通知を書き込み、実行の明示完了を要求する。
pub fn finalize(run_id: Option<i64>) -> Result<Value, String> {
    let run_id = require_run_id(run_id)?;
    crate::daemon::write_finalize_request(run_id)
        .map_err(|e| format!("finalize_request の書き込みに失敗しました: {}", e))?;
    Ok(json!({
        "status": "requested",
        "run_id": run_id,
        "message": format!("run_id={} の完了処理を要求しました。", run_id),
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
}

/// todo のカード（`.sai/outputs/todo/<run_id>/task.md`）から取り出した値だけを並べたもの。
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
    // `run_subtask` と同じ場所を読む（todo のカード → `.sai/outputs/todo/<run_id>/task.md`）
    let card = crate::orchestrator::read_todo_card(card_run_id)?;
    if !card.frontmatter.is_professor_executer() {
        return Err(format!(
            "カード {} は人が起こすカードです（executer: {}）。professor が起こすには executer: professor のカードを作ってください",
            card_run_id,
            card.frontmatter.executer()
        ));
    }
    // 同じカードを 2 度の子にはしない（予約行は 1 度しか確定できない）
    if let Ok(store) = crate::common::open_db() {
        if let Ok(tasks) = store.list_unfinished_tasks_by_card(card_run_id) {
            if let Some(existing) = tasks.first().and_then(|t| t.id) {
                return Err(format!(
                    "カード {} は既にタスク {} で起こしています",
                    card_run_id, existing
                ));
            }
        }
    }
    let block = card.frontmatter.block.trim().to_string();
    if block.is_empty() {
        return Err(format!(
            "カード {} の task.md に block: がありません。block: <block_id> を記載してください",
            card_run_id
        ));
    }
    let task_text = crate::kanban::resolve_card_inputs(&card, &HashMap::new()).map_err(|e| {
        format!("カード {} の task_text が取れません: {}", card_run_id, e)
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
        .ok_or_else(|| "tasks は1つ以上のタスクを含む配列である必要があります".to_string())?;

    let mut seen_client_ids: HashSet<String> = HashSet::new();
    let mut normalized: Vec<NormalizedTask> = Vec::with_capacity(task_array.len());
    for item in task_array {
        let client_id = item
            .get("client_id")
            .and_then(|v| v.as_str())
            .ok_or("各タスクに client_id（文字列）が必要です")?
            .to_string();
        if !seen_client_ids.insert(client_id.clone()) {
            return Err(format!(
                "client_id '{}' が重複しています。各タスクに一意の client_id を割り当ててください。",
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
                    "client_id '{}' の card は 1 以上の整数（todo のカードの run_id）で指定してください",
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
            (None, None) => return Err("各タスクに subject（文字列）が必要です".to_string()),
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
                    "card を指定したときは task_text を書かないでください（カードの本文を使います）"
                        .to_string(),
                )
            }
            (Some(c), None) => c.task_text.clone(),
            (None, Some(t)) => t,
            (None, None) => {
                return Err("各タスクに task_text（空でない文字列）が必要です".to_string())
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
                        "client_id '{}': カード {} の block は「{}」です。block_type はカードの block と一致させてください",
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
                        "client_id '{}' の timeout は 1 以上の整数（秒）で指定してください",
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
                            "cwd「{}」を解決できません: {}。絶対パス・~/ 始まり・プロジェクトルートからの相対パスで、存在するフォルダを指定してください。",
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
                    "指定された block_type「{}」のブロック定義を取得できませんでした。block_discover で正しい id を確認してください（表示名ではなく id を指定）。",
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
            "error": "task_create の入力検証に失敗しました。下記の修正指示に従って block_type を修正してください。",
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
                "timeout": t.timeout,
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

    let result_tasks: Vec<Value> = normalized
        .iter()
        .map(|t| json!({ "client_id": t.client_id, "status": "approved" }))
        .collect();

    Ok(json!({
        "batch_id": batch_id,
        "round": round,
        "supersedes_batch_id": supersedes_batch_id,
        "status": "approved",
        "tasks": result_tasks,
        "hint": "task_execute に batch_id を渡すと一括実行されます。server_id は task_list で確認してください。",
    }))
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

/// `.sai/settings.json` の `max_concurrent` を読む（無ければ既定 16）。
fn max_concurrent_limit() -> u64 {
    let path = crate::project::sai_path(".sai/settings.json");
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
        return Err("batch_id または task_id のどちらかを指定してください。".to_string());
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
                    "バッチ {} の承認タスク数 {} が一括投入上限 {} を超えています。タスクを分割して再作成するか、不要なタスクを finalize してから再実行してください。",
                    bid, count, limit
                ),
            }));
        }
        if count == 0 {
            return Ok(json!({
                "batch_id": bid,
                "status": "no_approved_tasks",
                "message": format!(
                    "バッチ {} に承認済みタスクがありません。task_create でタスクを作成してから再実行してください。",
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
        "message": "実行キューへ送信しました（非同期）。進捗は task_status / task_output で確認してください。",
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
    std::fs::read_to_string(dir.join("finalize-report.md")).ok()
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

            let db_path = crate::project::sai_path(".sai/sai-engine/sai-engine.db");
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
                        None => format!("waggle:{}: (description なし)", token),
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
            Ok(json!({
                "task_id": task_id,
                "status": "completed",
                "round": value.get("round").and_then(|v| v.as_i64()).unwrap_or(1),
                "report": task_report_text(task_id),
                "outputs": outputs,
            }))
        }
        "task_stalled" => {
            let reason = value.get("reason").and_then(|v| v.as_str()).unwrap_or("");
            // long_running / timeout は子がまだ working なので、止めずに画面を見る案内にする
            let hint = if reason == "long_running" || reason == "timeout" {
                "子はまだ working です。task_capture で画面を確かめ、進んでいれば task_output で待ち続けてください。止まっているなら task_manage interrupt で止め、send で原因を調べさせてください。同じ理由のアラートは、working が続く限り繰り返し届きます。"
            } else {
                "サブブロックが止まっています。task_capture で画面を確かめ、task_manage send で指示を送ってください。作業中（working）や blocked なら先に task_manage interrupt で止めてください。"
            };
            Ok(json!({
                "task_id": task_id,
                "status": "stalled",
                "reason": value.get("reason").cloned().unwrap_or(Value::Null),
                "message": value.get("message").cloned().unwrap_or(Value::Null),
                "hint": hint,
            }))
        }
        _ => Ok(json!({
            "task_id": task_id,
            "status": "failed",
            "error": value.get("error").cloned().unwrap_or(Value::Null),
        })),
    }
}

/// タスクの実行結果を取得する（未完了なら待機、または即時参照）。
pub fn task_output(task_id: i64, timeout_ms: Option<u64>) -> Result<Value, String> {
    let timeout_ms = timeout_ms.unwrap_or(300_000);
    let dir = crate::common::notify_results_dir();

    if timeout_ms == 0 {
        // 即時参照: 見つかっても結果ファイルは削除しない（移植元 readResult と同じ）
        return match find_result_file(&dir, task_id) {
            Some((_, value)) => build_task_output_result(task_id, &value),
            None => Ok(json!({
                "task_id": task_id,
                "status": "in_progress",
                "message": "タスクはまだ完了していません。task_output を再度呼び出すか、timeout_ms を指定してください。",
            })),
        };
    }

    let started = std::time::Instant::now();
    let poll_interval = std::time::Duration::from_millis(500);
    let found = loop {
        if let Some(found) = find_result_file(&dir, task_id) {
            break Some(found);
        }
        if started.elapsed().as_millis() as u64 >= timeout_ms {
            break None;
        }
        std::thread::sleep(poll_interval);
    };

    match found {
        Some((path, value)) => {
            let result = build_task_output_result(task_id, &value)?;
            let _ = std::fs::remove_file(&path);
            Ok(result)
        }
        None => Ok(json!({
            "task_id": task_id,
            "status": "timeout",
            "error": format!(
                "タイムアウト: {}ms 以内にタスクが完了しませんでした。task_output を再度呼び出して結果を確認してください。",
                timeout_ms
            ),
        })),
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
/// （メインブロックの完了は `finalize` / `sai_finalize` を使う）。
pub fn task_report(report: &str, run_id: Option<i64>) -> Result<Value, String> {
    let report = report.trim();
    if report.is_empty() {
        return Err("report（完了報告の本文）は必須です".to_string());
    }
    let run_id = require_run_id(run_id)?;

    let store =
        crate::common::open_db().map_err(|e| format!("DB オープンに失敗しました: {}", e))?;
    let exec = store
        .get_execution_by_id(run_id)
        .map_err(|e| format!("実行 id = {} の取得に失敗しました: {}", run_id, e))?;
    if exec.parent_run_id.is_none() {
        return Err(
            "task_report はサブブロック専用です。メインブロックの完了は sai_finalize を使ってください"
                .to_string(),
        );
    }

    // 報告本文は run フォルダ直下の finalize-report.md に保存する。
    // この保存は waggle トークン化しない（file_written 通知を書かない）。
    let path = match crate::run_dir::find_run_dir(run_id) {
        Some((_, dir)) => dir.join("finalize-report.md"),
        None => crate::sai_write_notify::resolve_output_path(run_id, "finalize-report.md")
            .map_err(|e| format!("保存先の解決に失敗しました: {}", e))?,
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("保存先作成に失敗: {}", e))?;
    }
    std::fs::write(&path, report).map_err(|e| format!("報告の保存に失敗: {}", e))?;

    crate::daemon::write_finalize_request(run_id)
        .map_err(|e| format!("finalize_request の書き込みに失敗しました: {}", e))?;

    Ok(json!({
        "status": "reported",
        "run_id": run_id,
        "message": "完了報告を送りました。",
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
        crate::common::open_db().map_err(|e| format!("DB オープンに失敗しました: {}", e))?,
    );
    let manager = TaskManager::new(Arc::clone(&store));
    let task = manager
        .get_task(task_id)
        .map_err(|e| format!("タスク取得エラー: {}", e))?;

    let exec_id = task
        .execution_id
        .ok_or_else(|| format!("タスク {} はまだ実行されていません", task_id))?;
    let exec = store
        .get_execution_by_id(exec_id)
        .map_err(|e| format!("実行レコード取得エラー: {}", e))?;
    let pane_id = exec
        .herdr_pane_id
        .clone()
        .ok_or_else(|| format!("実行 {} に pane がありません", exec_id))?;

    Ok((task, exec, pane_id))
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
                "タスク {} は作業中（working）です。止めて指示を変えるなら先に task_manage interrupt を呼んでください。終わるのを待つなら task_output を使ってください。",
                task_id
            )),
            other => {
                let label = other.unwrap_or("状態不明");
                Err(format!(
                    "タスク {} は {} で止まっています。task_capture で画面を確かめ、task_manage interrupt で入力待ちに戻してから send してください。",
                    task_id, label
                ))
            }
        },
        "finalized" | "deleted" => Err(format!(
            "タスク {} は {} です。確定・削除済みのタスクには send できません。やり直すなら requeue か task_create を使ってください。",
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
            "無効な action: '{}'。delete / finalize / send / requeue / interrupt のいずれかを指定してください。",
            action
        ));
    }
    if action == "send" && message.map(str::trim).unwrap_or("").is_empty() {
        return Err("action=send には message（追加入力指示）が必須です。".to_string());
    }

    // interrupt / send の振り分けにタスクの状態が要るので、いつもタスクを読む。
    let store = Arc::new(
        crate::common::open_db().map_err(|e| format!("DB オープンに失敗しました: {}", e))?,
    );
    let task = TaskManager::new(Arc::clone(&store))
        .get_task(task_id)
        .map_err(|e| format!("タスク取得エラー: {}", e))?;

    let parent_run_id = match require_run_id(run_id) {
        Ok(id) => id,
        Err(_) => task.parent_run_id,
    };

    match action {
        // ── interrupt: デーモンを経由せず、そのまま pane に esc を送る ──
        "interrupt" => {
            if task.status != "in_progress" {
                return Err(format!(
                    "タスク {} は {} です。interrupt できるのは作業中（in_progress）のタスクだけです。",
                    task_id, task.status
                ));
            }
            let (_task, _exec, pane_id) = task_pane(task_id)?;
            let herdr = crate::herdr::HerdrClient::new();
            herdr
                .send_keys(&pane_id, "esc")
                .map_err(|e| format!("esc の送信に失敗: {}", e))?;

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
                    "message": "esc を送りましたが、まだ working です。task_capture で確かめ、もう一度 interrupt するか、delete で終了させてください。",
                }));
            }
            Ok(json!({
                "action": "interrupt",
                "task_id": task_id,
                "status": "interrupted",
                "agent_status": agent_status,
                "message": format!(
                    "タスク {} に esc を送りました（agent_status={}）。task_capture で画面を確かめ、send で次の指示を送ってください。",
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
                    let (_task, exec, pane_id) = task_pane(task_id)?;
                    // 説明に追記する（履歴・記録用。task_manager の send と同じ形式）。
                    let new_desc = if task.description.is_empty() {
                        format!("[追加入力指示]\n{}", message)
                    } else {
                        format!("{}\n\n[追加入力指示]\n{}", task.description, message)
                    };
                    store
                        .update_task_description(task_id, &new_desc)
                        .map_err(|e| format!("説明の更新に失敗: {}", e))?;

                    let herdr = crate::herdr::HerdrClient::new();
                    let submit_key = crate::runner::RunnerKind::parse(&exec.runner).submit_key();
                    // 1 回目の実行は task_report まで待機が続くので、ターン実行は別に起こさない。
                    // ここで送った指示は同じ実行の続きとして処理される。
                    crate::orchestrator::submit_prompt_verified(
                        &herdr,
                        &pane_id,
                        message,
                        submit_key,
                        None,
                    )
                    .map_err(|e| format!("pane への送信に失敗: {}", e))?;

                    // 注意: ターン送信中（completed のあとに send して in_progress になった状態）に
                    // interrupt すると、子が idle になり `send_turn_to_pane` がそのターンを
                    // 完了とみなしてタスクが completed になる。この場合は次の send が
                    // Daemon 経路へ落ちるので、経路の取り合いは起きない。
                    Ok(json!({
                        "action": "send",
                        "task_id": task_id,
                        "status": "sent",
                        "message": format!(
                            "作業中のタスク {} の pane に追加指示を送りました。完了は task_report（task_output）で受け取ってください。",
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
                            "タスク {} に追加指示を送信しました。同一エージェント（pane）が継続して対応します。",
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
                    "タスク {} は {} です。requeue できるのは failed / deleted のタスクだけです。",
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
                    "タスク {} の操作（{}）を sai-engine に要求しました。",
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
                    "タスク {} の操作（{}）を sai-engine に要求しました。",
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
        "hint": "調査は task_capture <ID>、操作は task_manage <interrupt|send|delete|finalize|requeue> <ID> を使用してください。",
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
        .map_err(|e| format!("pane 読み取りに失敗: {}", e))?;

    let empty = output.is_empty();
    let mut result = json!({
        "task_id": task_id,
        "pane_id": pane_id,
        "lines": lines,
        "output": output,
    });
    if empty {
        result["message"] = json!("(pane の出力は空です)");
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
        _ => std::env::var("SAI_BLOCK_NAME").ok(),
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
        "hint": "task_create の block_type には id を指定してください。詳細は block_inspect <id>。",
    });
    if defs.is_empty() {
        result["message"] = json!("呼び出し可能なサブブロックがありません。メインブロック定義の subblocks を確認してください。");
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
            "ブロック定義が見つかりません: {}（ID または表示名で指定）",
            name
        ));
    }

    let parsed = parse_block_file(&path).map_err(|e| format!("ブロック定義のパースに失敗: {}", e))?;
    let entry = block_def_to_json(&parsed.config);
    serde_json::to_value(entry).map_err(|e| format!("JSON 変換に失敗: {}", e))
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
        crate::common::open_db().map_err(|e| format!("DB オープンに失敗しました: {}", e))?;

    let entries: Vec<(String, i64)> = match run_id {
        Some(id) => {
            let exec = store
                .get_execution_by_id(id)
                .map_err(|e| format!("実行 id = {} の取得に失敗しました: {}", id, e))?;
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
                .list_waggle_tokens(None, None, -1, include_running)
                .map_err(|e| format!("waggle トークン一覧の取得に失敗しました: {}", e))?
                .into_iter()
                .map(|e| (e.token, e.execution_id))
                .collect()
        }
    };

    let db_path = crate::project::sai_path(".sai/sai-engine/sai-engine.db");
    let waggle = crate::waggle_integration::WaggleIntegrator::open(&db_path)
        .map_err(|e| format!("waggle DB のオープンに失敗しました: {}", e))?;

    let mut items = Vec::new();
    for (token, id) in entries {
        if let Some(line) = waggle_item_line(&waggle, &token, id) {
            items.push(line);
        }
    }

    Ok(json!({
        "count": items.len(),
        "items": items,
        "hint": "本文は sai_read { token }。子タスクへは task_text に waggle:<token> と書けば本文が埋め込まれる",
    }))
}

/// waggle 文書 1 件の本文を返す。
///
/// `token` は `waggle:<tok>` / `waggle:<tok>:path:…` のどちらでもよい。
pub fn sai_read(token: &str) -> Result<Value, String> {
    let token = normalize_waggle_token(token);
    if token.is_empty() {
        return Err("token は必須です".to_string());
    }

    let db_path = crate::project::sai_path(".sai/sai-engine/sai-engine.db");
    let waggle = crate::waggle_integration::WaggleIntegrator::open(&db_path)
        .map_err(|e| format!("waggle DB のオープンに失敗しました: {}", e))?;

    let view = waggle
        .resolve_token(&token)
        .map_err(|e| format!("waggle トークン '{}' の解決に失敗しました: {}", token, e))?;
    let content = waggle.resolve_content(&token).map_err(|e| {
        format!(
            "waggle トークン '{}' の本文取得に失敗しました: {}",
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

    // 環境変数を設定する他モジュールのテスト（orchestrator / cc_command / task_cwd）と
    // 排他する。モジュール固有のロックに分けると SAI_PROJECT_ROOT が競合する。
    struct EnvGuard {
        _lock: std::sync::MutexGuard<'static, ()>,
        project_root: PathBuf,
    }

    impl EnvGuard {
        fn new() -> Self {
            let lock = crate::common::SAI_ENV_TEST_LOCK
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            let dir = std::env::temp_dir().join(format!(
                "sai-link-agent-api-test-{}-{}",
                std::process::id(),
                unix_millis()
            ));
            std::fs::create_dir_all(&dir).unwrap();
            std::env::set_var("SAI_PROJECT_ROOT", &dir);
            Self {
                _lock: lock,
                project_root: dir,
            }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            std::env::remove_var("SAI_PROJECT_ROOT");
            std::env::remove_var("SAI_PARENT_RUN_ID");
            let _ = std::fs::remove_dir_all(&self.project_root);
        }
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

    /// P4: sai_write がトークンと ref を返し、通知に waggle_token が入る。
    /// デーモン相当の処理（poll_and_process_one）でも同じトークンが返り、
    /// 再トークン化されない（トークンが増えない）ことを確認する。
    #[test]
    fn write_returns_token_and_ref_and_daemon_keeps_the_same_token() {
        let guard = EnvGuard::new();
        std::env::set_var("SAI_PARENT_RUN_ID", "42");

        let result = write("report.md", "# 報告\n", Some("報告書"), None).unwrap();
        let token = result["token"].as_str().expect("token が返る").to_string();
        assert!(!token.is_empty());
        assert_eq!(
            result["ref"].as_str().unwrap(),
            format!("waggle:{}:path:.sai/outputs/*/42/report.md", token)
        );

        // 通知 payload に waggle_token が入っていること
        let pending = crate::common::notify_pending_dir();
        let entries: Vec<_> = std::fs::read_dir(&pending).unwrap().flatten().collect();
        assert_eq!(entries.len(), 1);
        let value: Value =
            serde_json::from_str(&std::fs::read_to_string(entries[0].path()).unwrap()).unwrap();
        assert_eq!(value["waggle_token"].as_str().unwrap(), token);

        // デーモン相当の処理：同じトークンが返り、再トークン化されない
        let db_path = crate::project::sai_path(".sai/sai-engine/sai-engine.db");
        let integrator = crate::waggle_integration::WaggleIntegrator::open(&db_path).unwrap();
        let processed = crate::sai_write_notify::poll_and_process_one(&integrator)
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
            "sai-link-find-result-file-{}-{}",
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
        std::env::set_var("SAI_PARENT_RUN_ID", "42");

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
        std::env::set_var("SAI_PARENT_RUN_ID", "42");

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
        std::env::set_var("SAI_PARENT_RUN_ID", "42");

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
        std::env::set_var("SAI_PARENT_RUN_ID", "42");

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
        std::env::set_var("SAI_PARENT_RUN_ID", "42");
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
        std::env::set_var("SAI_PARENT_RUN_ID", "42");

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
        let dir = root.join(".sai/blocks");
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

    /// todo のカード（`.sai/outputs/todo/<run_id>/task.md`）を作る（テスト用）。
    fn write_todo_card(
        root: &Path,
        run_id: i64,
        block: &str,
        executer: &str,
        description: &str,
        task_text: &str,
    ) {
        let dir = root.join(".sai/outputs/todo").join(run_id.to_string());
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("task.md"),
            format!(
                "---\ncreation_date: 2026/09/30 10:00:00\nexecution_date: -\nrun_id: {rid}\n\
                 description: {desc}\nrunner: pi\nblock: {block}\nthinking: \nmodel: \n\
                 provider: \nround: 1\nglobal_round: 1\ntag: \nexecuter: {executer}\n\
                 sci_node: N12\n---\n\n# task_text\n\n{text}\n",
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
        std::env::set_var("SAI_PARENT_RUN_ID", &parent_run_id.to_string());
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

    /// U2: todo にカードが無い（run_id が違うものは無い）是エラー。
    #[test]
    fn task_create_with_card_rejects_missing_card() {
        let guard = EnvGuard::new();
        std::env::set_var("SAI_PARENT_RUN_ID", "42");
        write_test_block(&guard.project_root, "card-block");
        // 121 のカードがあるだけで、120 は無い
        write_todo_card(&guard.project_root, 121, "card-block", "professor", "説明", "本文");

        let tasks = json!([{ "client_id": "R1", "subject": "件名", "card": 120 }]);
        let err = task_create(&tasks, None, None, None).unwrap_err();
        assert!(err.contains("todo にカード 120 がありません"), "err={}", err);

        // 通知は書かれていないこと
        let pending = crate::common::notify_pending_dir();
        let count = std::fs::read_dir(&pending)
            .map(|d| d.flatten().count())
            .unwrap_or(0);
        assert_eq!(count, 0);

        drop(guard);
    }

    /// U2: `executer: human` のカードは professor が起こせない。
    #[test]
    fn task_create_with_card_rejects_human_executer() {
        let guard = EnvGuard::new();
        std::env::set_var("SAI_PARENT_RUN_ID", "42");
        write_test_block(&guard.project_root, "card-block");
        write_todo_card(&guard.project_root, 120, "card-block", "human", "説明", "本文");

        let tasks = json!([{ "client_id": "R1", "subject": "件名", "card": 120 }]);
        let err = task_create(&tasks, None, None, None).unwrap_err();
        assert!(err.contains("人が起こすカードです"), "err={}", err);
        assert!(err.contains("executer: professor"), "err={}", err);

        drop(guard);
    }

    /// U2: `card` と `task_text` の併用はできない（依頼の正は 1 つ）。
    #[test]
    fn task_create_with_card_rejects_task_text() {
        let guard = EnvGuard::new();
        std::env::set_var("SAI_PARENT_RUN_ID", "42");
        write_test_block(&guard.project_root, "card-block");
        write_todo_card(&guard.project_root, 120, "card-block", "professor", "説明", "本文");

        let tasks = json!([{ "client_id": "R1", "subject": "件名", "card": 120, "task_text": "書かない" }]);
        let err = task_create(&tasks, None, None, None).unwrap_err();
        assert!(
            err.contains("card を指定したときは task_text を書かないでください"),
            "err={}",
            err
        );

        drop(guard);
    }

    /// U2: カードの `block:` と違う block_type を書くとエラー。一致すれば通る。
    #[test]
    fn task_create_with_card_rejects_different_block_type() {
        let guard = EnvGuard::new();
        std::env::set_var("SAI_PARENT_RUN_ID", "42");
        write_test_block(&guard.project_root, "card-block");
        write_test_block(&guard.project_root, "other-block");
        write_todo_card(&guard.project_root, 120, "card-block", "professor", "説明", "本文");

        let tasks = json!([
            { "client_id": "R1", "subject": "件名", "card": 120, "block_type": "other-block" },
        ]);
        let err = task_create(&tasks, None, None, None).unwrap_err();
        assert!(
            err.contains("block_type はカードの block と一致させてください"),
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
        std::env::set_var("SAI_PARENT_RUN_ID", &parent_run_id.to_string());
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
            err.contains(&format!("カード 120 は既にタスク {} で起こしています", first_task_id)),
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
        std::env::set_var("SAI_PARENT_RUN_ID", "42");
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
        std::env::set_var("SAI_PARENT_RUN_ID", "42");

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
        std::env::set_var("SAI_PARENT_RUN_ID", "42");
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
            .contains("フォルダではありません"));

        drop(guard);
    }

    #[test]
    fn task_manage_rejects_invalid_action() {        let err = task_manage("invalid", 1, None, Some(1)).unwrap_err();
        assert!(err.contains("無効な action"));
    }

    /// U3 AC1: `task_create` が `timeout`（秒）を受け付け、通知 payload に載せること。
    #[test]
    fn task_create_accepts_timeout() {
        let guard = EnvGuard::new();
        std::env::set_var("SAI_PARENT_RUN_ID", "42");

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
        std::env::set_var("SAI_PARENT_RUN_ID", "42");

        for bad in [json!(0), json!(-5), json!("abc"), json!(1.5)] {
            let tasks = json!([
                { "client_id": "t1", "subject": "件名", "task_text": "説明", "timeout": bad },
            ]);
            let err = task_create(&tasks, None, None, None).unwrap_err();
            assert!(
                err.contains("timeout は 1 以上の整数（秒）"),
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
        assert!(err.contains("作業中（working）"), "{}", err);
        assert!(err.contains("task_manage interrupt"), "{}", err);

        // in_progress + blocked / error / 未知の値 / None → エラー
        for st in [Some("blocked"), Some("error"), Some("weird"), None] {
            let err = send_route(12, "in_progress", st).unwrap_err();
            assert!(err.contains("で止まっています"), "{}", err);
            assert!(err.contains("task_manage interrupt"), "{}", err);
        }
        // None のときは「状態不明」と出る
        assert!(
            send_route(12, "in_progress", None)
                .unwrap_err()
                .contains("状態不明"),
            "agent_status が取れないときは 状態不明 と表示する"
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
            err.contains("interrupt できるのは作業中（in_progress）のタスクだけ"),
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

        std::env::remove_var("SAI_PARENT_RUN_ID");
        assert_eq!(require_run_id(Some(7)).unwrap(), 7);

        std::env::set_var("SAI_PARENT_RUN_ID", "9");
        assert_eq!(require_run_id(None).unwrap(), 9);

        std::env::remove_var("SAI_PARENT_RUN_ID");
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
        assert!(err.contains("サブブロック専用"), "err={}", err);
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
        assert_eq!(result["outputs"][0], "waggle:FAKE1: (description なし)");
        // 削除した項目が返らないこと
        assert!(result.get("resolved_content").is_none());
        assert!(result.get("waggle_tokens").is_none());
        assert!(result.get("commit_id").is_none());
        assert!(result.get("elapsed_ms").is_none());
        assert!(result.get("session_stats").is_none());

        drop(guard);
    }
}
