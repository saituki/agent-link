//! # MCP サーバ（`sai-link mcp serve`）
//!
//! pi（pi-mcp-adapter 経由）と Claude Code（`runner=cc`）が共通で使う
//! MCP（Model Context Protocol）サーバです。U01 で pi-sai-link 廃止後は
//! pi/cc ともにこの MCP の 13 ツールを使い、差分は起動コマンドだけです。
//!
//! ## トランスポート
//!
//! stdio 上で **改行区切り JSON-RPC 2.0** をやり取りします
//! （MCP の stdio transport 仕様）。1 行 = 1 メッセージ。
//!
//! ## 公開ツール
//!
//! `sai_write` / `sai_finalize` / `sai_read` / `task_create` / `task_report` /
//! `task_list` / `task_execute` / `task_output` / `task_manage` / `task_status` /
//! `task_capture` / `block_discover` / `block_inspect` の 13 個。実体はすべて
//! `agent_api`（pi の `sai-link agent <サブコマンド>` と共通の唯一実装）を呼ぶだけで、
//! MCP 側にロジックは書かない。
//!
//! 通知プロトコル（pending/・results/）は pi/cc 共通のため、
//! DB への保存・TUI 表示は runner を問わず同じになる。
//!
//! 公開ツール名は `cc_command::MCP_TOOL_NAMES` と 1 個ずつ完全一致させること。

use std::io::{BufRead, Write};

use serde_json::{json, Value};

/// MCP のプロトコルバージョン（initialize でクライアント値が無い場合の既定）。
const DEFAULT_PROTOCOL_VERSION: &str = "2024-11-05";

/// stdio で MCP サーバを実行する（`sai-link mcp serve` から呼ばれる）。
///
/// stdin を 1 行ずつ読み、JSON-RPC を処理して stdout に 1 行で応答する。
/// 通知（id なし）には応答しない。
pub fn serve_stdio() {
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    let mut out = stdout.lock();

    for line in stdin.lock().lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => break,
        };
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let message: Value = match serde_json::from_str(trimmed) {
            Ok(v) => v,
            Err(e) => {
                let err = error_response(Value::Null, -32700, &format!("JSON パース失敗: {}", e));
                write_message(&mut out, &err);
                continue;
            }
        };
        if let Some(response) = handle_message(&message) {
            write_message(&mut out, &response);
        }
    }
}

/// 1 メッセージを処理して応答を返す（通知なら `None`）。
pub fn handle_message(message: &Value) -> Option<Value> {
    let id = message.get("id").cloned();
    let method = message.get("method").and_then(|m| m.as_str()).unwrap_or("");
    let params = message.get("params").cloned().unwrap_or(Value::Null);

    // 通知（id なし）は応答しない
    if id.is_none() {
        return None;
    }
    let id = id.unwrap();

    let result = match method {
        "initialize" => Ok(initialize_result(&params)),
        "ping" => Ok(json!({})),
        "tools/list" => Ok(tools_list_result()),
        "tools/call" => call_tool(&params),
        _ => Err((-32601, format!("未対応のメソッドです: {}", method))),
    };

    Some(match result {
        Ok(value) => json!({ "jsonrpc": "2.0", "id": id, "result": value }),
        Err((code, message)) => error_response(id, code, &message),
    })
}

/// `initialize` の応答を組み立てる。
fn initialize_result(params: &Value) -> Value {
    let protocol_version = params
        .get("protocolVersion")
        .and_then(|v| v.as_str())
        .unwrap_or(DEFAULT_PROTOCOL_VERSION);
    json!({
        "protocolVersion": protocol_version,
        "capabilities": { "tools": {} },
        "serverInfo": { "name": "sai-link", "version": env!("CARGO_PKG_VERSION") }
    })
}

/// `tools/list` の応答を組み立てる。
fn tools_list_result() -> Value {
    json!({ "tools": [
        {
            "name": "sai_write",
            "description": "成果物ファイルを自 run フォルダ (.sai/outputs/<owner>/<run_id>/) に保存し、waggle トークン化をデーモンに通知する。ブロックの成果物は必ずこのツールで保存すること。",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "filename": { "type": "string", "description": "保存ファイル名（run フォルダ直下。ディレクトリ部分は無視される）" },
                    "content": { "type": "string", "description": "ファイル内容" },
                    "description": { "type": "string", "description": "成果物の説明（waggle トークンのラベル）" }
                },
                "required": ["filename", "content"]
            }
        },
        {
            "name": "sai_finalize",
            "description": "メインブロックの実行を明示的に完了する。出力を確定し、実行を completed にする。",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "run_id": { "type": "integer", "description": "完了する実行 ID（省略時は SAI_PARENT_RUN_ID）" }
                }
            }
        },
        {
            "name": "sai_read",
            "description": "waggle 文書を探して読む。引数なし=完了済み run の全文書の一覧、run_id=その実行の文書の一覧、token=本文。まず一覧の description で内容を判断し、必要な文書だけ本文を読む。",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "run_id": { "type": "integer", "description": "一覧を絞る実行 ID" },
                    "token": { "type": "string", "description": "本文を読む waggle トークン（waggle: は付けても付けなくてもよい）" }
                }
            }
        },
        {
            "name": "task_create",
            "description": "新しいサブタスクを作成する。承認は不要で、指定したタスクはそのまま作成される。block_type（task.md の block と同じ）には block_discover で確認した id を指定する。task_text は子ブロックの {{input.task_text}} に入るテキストで、文中の waggle:<トークン> は実行時に文書の本文へ置き換わる。description（task.md の説明・省略時は task_text と同じ）を付けられる。cwd で作業フォルダ（別プロジェクト等）、worktree で作業用コピーの有無を指定できる。作成時に親 run へ task-<client_id>.md の写しを保存し TUI/phone で確認できる。",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "tasks": {
                        "type": "array",
                        "description": "作成するタスクの配列",
                        "items": {
                            "type": "object",
                            "properties": {
                                "client_id": { "type": "string", "description": "クライアント側の一時 ID（結果の紐付け用・バッチ内で一意）" },
                                "subject": { "type": "string", "description": "タスク件名" },
                                "task_text": { "type": "string", "description": "子ブロックの {{input.task_text}} に入るテキスト。文中の waggle:<トークン> は実行時に文書の本文へ置き換わる" },
                                "block_type": { "type": "string", "description": "使用するブロック定義の id（表示名ではなく id。task.md の block と同じ）" },
                                "block": { "type": "string", "description": "block_type と同じ（task.md 互換の別名）" },
                                "description": { "type": "string", "description": "タスクの説明（task.md の description。省略時は task_text と同じ）" },
                                "depends_on": { "type": "array", "items": { "type": "string" }, "description": "依存するタスクの client_id（同一バッチ内）" },
                                "cwd": { "type": "string", "description": "サブタスクの作業フォルダ。絶対パス・~/ 始まり・プロジェクトルートからの相対パス。存在するフォルダであること。省略時はプロジェクトルート" },
                                "timeout": { "type": "integer", "description": "子の時間上限（秒）。ブロック定義の timeout を上書きする。超えてもタスクは失敗にせず、親の task_output に stalled(reason=timeout) を返す。完了しないあいだは同じ間隔で繰り返し知らせる。省略時はブロック定義に従う" },
                                "worktree": { "type": "string", "enum": ["isolation", "none"], "description": "作業用コピー（git worktree）の指定。未指定ならブロック定義の worktree に従う。isolation なら cwd を含むリポジトリの .worktrees/<block>-<run_id> を作る。none なら作らない（ブロック定義の isolation も打ち消す）" },
                                "carried_over": { "type": "boolean", "description": "前バッチからの引き継ぎタスク" }
                            },
                            "required": ["client_id", "subject", "task_text"]
                        }
                    },
                    "round": { "type": "integer", "description": "修正ラウンド番号（1 始まり）。作り直し時は前回+1" },
                    "supersedes_batch_id": { "type": "string", "description": "置き換え元バッチ ID（作り直し時に前回の batch_id）" }
                },
                "required": ["tasks"]
            }
        },
        {
            "name": "task_report",
            "description": "サブブロック（worker）が最後に 1 回呼ぶ完了報告。結果・要約・コミット・完了条件・問題点・kantoku への依頼を 10 行以内で書く。呼ぶとタスクは完了扱いになる",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "report": { "type": "string", "description": "完了報告の本文（10 行以内）" }
                },
                "required": ["report"]
            }
        },
        {
            "name": "task_list",
            "description": "作成済みサブタスクの一覧を取得する。ステータスで絞り込める。各タスクには親がマージに使う cwd / worktree_info（ブランチ名・リポジトリ・作業用コピーの場所）が含まれる。",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "status": { "type": "string", "enum": ["pending","approved","queued","waiting_dependency","waiting_pane","in_progress","completed","finalized","failed","deleted","all"], "description": "フィルタするステータス。既定 all" },
                    "sort": { "type": "string", "enum": ["id","recent","oldest"], "description": "ソート順。既定 id" }
                }
            }
        },
        {
            "name": "task_execute",
            "description": "作成済みタスクを実行キューへ送る。batch_id でバッチ一括、task_id で単体。非同期で別 pane 実行され、即座に制御が戻る。結果は task_output で取得する。",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "batch_id": { "type": "string", "description": "実行するバッチ ID（task_create の戻り値）" },
                    "task_id": { "type": "integer", "description": "実行するタスクの server_id（単体・再実行用）" }
                }
            }
        },
        {
            "name": "task_output",
            "description": "タスクの結果を取得する。未完了の場合は timeout_ms まで待機する。完了時は短い報告文（report）と、worker が保存した文書の waggle トークン一覧（outputs）だけを返す。",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "task_id": { "type": "integer", "description": "タスクの server_id" },
                    "timeout_ms": { "type": "integer", "description": "最大待機時間（ミリ秒）。既定 300000。0 で待機なし" }
                },
                "required": ["task_id"]
            }
        },
        {
            "name": "task_manage",
            "description": "サブタスクを操作する。interrupt（esc を送って作業中の子を止め、入力待ちにする。人間の esc と同じ）/ send（追加指示。入力待ちの子にはその場で届く。作業中（working）の子には送れないので先に interrupt）/ delete（削除。実行中なら pane を閉じて強制終了。人間の ctrl+d と同じ）/ finalize（完了確定・pane 削除）/ requeue（failed / deleted を再実行）。断られたときはエラーで理由を返す。",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "action": { "type": "string", "enum": ["delete","finalize","send","requeue","interrupt"], "description": "操作種別" },
                    "task_id": { "type": "integer", "description": "対象タスクの server_id" },
                    "message": { "type": "string", "description": "action=send の追加指示テキスト" }
                },
                "required": ["action", "task_id"]
            }
        },
        {
            "name": "task_status",
            "description": "サブタスクの状態を詳細に取得する。status / execution_id / finalized / deleted_by / round / cwd / worktree_info（マージ用のブランチ名・リポジトリ・作業用コピーの場所）を確認し、失敗原因の調査や finalize / send / delete の判断に使う。",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "status": { "type": "string", "enum": ["pending","approved","queued","waiting_dependency","waiting_pane","in_progress","completed","finalized","failed","deleted","all"], "description": "フィルタするステータス。既定 all" },
                    "task_id": { "type": "integer", "description": "特定タスクのみ取得する場合の server_id" }
                }
            }
        },
        {
            "name": "task_capture",
            "description": "サブタスクの pane 出力を取得する。サブブロックが何を出力して、どこで詰まったかを確認する。",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "task_id": { "type": "integer", "description": "対象タスクの server_id" },
                    "lines": { "type": "integer", "description": "読み取る行数。既定 50" }
                },
                "required": ["task_id"]
            }
        },
        {
            "name": "block_discover",
            "description": "呼び出し可能なサブブロックの一覧を取得する。自ブロックの subblocks 制限が適用される。task_create の前に必ず確認すること。",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "parent": { "type": "string", "description": "親ブロック名（省略時は SAI_BLOCK_NAME）" }
                }
            }
        },
        {
            "name": "block_inspect",
            "description": "ブロック定義の詳細（入力フィールドの型・必須・選択肢・入力元ブロック、出力形式、subblocks）を取得する。task_create の inputs を正しく組み立てるために使う。",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "ブロック定義の id（block_discover で確認した id）" }
                },
                "required": ["name"]
            }
        }
    ]})
}

/// `tools/call` を処理する。
///
/// 各分岐は `agent_api` を 1 回呼ぶだけ（ロジックは MCP 側に書かない）。
/// `sai_write` / `sai_finalize` のみ既存の応答文言（人が読む整形テキスト）を
/// 維持するため `tool_sai_write` / `tool_sai_finalize` 経由で呼ぶ。
fn call_tool(params: &Value) -> Result<Value, (i32, String)> {
    let name = params
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or((-32602, "tools/call に name がありません".to_string()))?;
    let arguments = params
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| json!({}));

    if name == "sai_write" {
        return Ok(text_content_result(tool_sai_write(&arguments)));
    }
    if name == "sai_finalize" {
        return Ok(text_content_result(tool_sai_finalize(&arguments)));
    }

    // 引数取り出しの補助
    let s = |k: &str| arguments.get(k).and_then(|v| v.as_str()).map(str::to_string);
    let i = |k: &str| arguments.get(k).and_then(|v| v.as_i64());
    let u = |k: &str| arguments.get(k).and_then(|v| v.as_u64());

    let result: Result<Value, String> = match name {
        "task_create" => match arguments.get("tasks") {
            Some(tasks) => crate::agent_api::task_create(
                tasks,
                i("round"),
                s("supersedes_batch_id").as_deref(),
                i("run_id"),
            ),
            None => Err("tasks は必須です".to_string()),
        },
        "task_list" => crate::agent_api::task_list(
            s("status").as_deref(),
            s("sort").as_deref(),
            i("run_id"),
        ),
        "task_execute" => crate::agent_api::task_execute(
            s("batch_id").as_deref(),
            i("task_id"),
            i("run_id"),
        ),
        "task_output" => match i("task_id") {
            Some(id) => crate::agent_api::task_output(id, u("timeout_ms")),
            None => Err("task_id は必須です（整数）".to_string()),
        },
        "task_report" => match s("report") {
            Some(r) => crate::agent_api::task_report(&r, i("run_id")),
            None => Err("report は必須です".to_string()),
        },
        "task_manage" => match (s("action"), i("task_id")) {
            (Some(a), Some(id)) => {
                crate::agent_api::task_manage(&a, id, s("message").as_deref(), i("run_id"))
            }
            _ => Err("action と task_id は必須です".to_string()),
        },
        "task_status" => crate::agent_api::task_status(
            s("status").as_deref(),
            i("task_id"),
            i("run_id"),
        ),
        "task_capture" => match i("task_id") {
            Some(id) => crate::agent_api::task_capture(id, u("lines").map(|v| v as u32)),
            None => Err("task_id は必須です（整数）".to_string()),
        },
        "sai_read" => match s("token") {
            Some(t) => crate::agent_api::sai_read(&t),
            None => crate::agent_api::sai_read_list(i("run_id")),
        },
        "block_discover" => crate::agent_api::block_discover(s("parent").as_deref()),
        "block_inspect" => match s("name") {
            Some(n) => crate::agent_api::block_inspect(&n),
            None => Err("name は必須です".to_string()),
        },
        other => Err(format!("未知のツールです: {}", other)),
    };

    Ok(json_content_result(result))
}

/// 文字列 1 件を MCP の `content` テキストとして組み立てる（`sai_write` / `sai_finalize` 用）。
fn text_content_result(text: Result<String, String>) -> Value {
    match text {
        Ok(text) => json!({
            "content": [ { "type": "text", "text": text } ],
            "isError": false
        }),
        Err(e) => json!({
            "content": [ { "type": "text", "text": format!("エラー: {}", e) } ],
            "isError": true
        }),
    }
}

/// JSON 値を整形して MCP の `content` テキストとして組み立てる（`task_*` / `block_*` 用）。
fn json_content_result(result: Result<Value, String>) -> Value {
    match result {
        Ok(value) => json!({
            "content": [ { "type": "text", "text": serde_json::to_string_pretty(&value).unwrap_or_default() } ],
            "isError": false
        }),
        Err(e) => json!({
            "content": [ { "type": "text", "text": format!("エラー: {}", e) } ],
            "isError": true
        }),
    }
}

/// `sai_write` ツール本体。実体は `agent_api::write`（pi / cc 共通）。
fn tool_sai_write(arguments: &Value) -> Result<String, String> {
    let filename = arguments
        .get("filename")
        .and_then(|v| v.as_str())
        .ok_or("filename は必須です")?;
    let content = arguments
        .get("content")
        .and_then(|v| v.as_str())
        .ok_or("content は必須です")?;
    let description = arguments.get("description").and_then(|v| v.as_str());

    let result = crate::agent_api::write(filename, content, description, None)?;
    let path = result.get("path").and_then(|v| v.as_str()).unwrap_or_default();
    let byte_size = result.get("byte_size").and_then(|v| v.as_u64()).unwrap_or(0);
    let token = result.get("token").and_then(|v| v.as_str());
    let reference = result.get("ref").and_then(|v| v.as_str());
    match (token, reference) {
        (Some(token), Some(reference)) => Ok(format!(
            "保存しました: {} ({} bytes)。waggle トークン: {} / 他の文書から参照するときは {} の形で書いてください。",
            path, byte_size, token, reference
        )),
        _ => Ok(format!(
            "保存しました: {} ({} bytes)。waggle トークン化はデーモンが非同期で行います。",
            path, byte_size
        )),
    }
}

/// `sai_finalize` ツール本体。実体は `agent_api::finalize`（pi / cc 共通）。
fn tool_sai_finalize(arguments: &Value) -> Result<String, String> {
    let run_id = arguments.get("run_id").and_then(|v| v.as_i64());
    let result = crate::agent_api::finalize(run_id)?;
    let run_id = result.get("run_id").and_then(|v| v.as_i64()).unwrap_or(0);
    Ok(format!(
        "run_id={} の完了処理を要求しました。出力を確定します。",
        run_id
    ))
}

/// JSON-RPC エラー応答を組み立てる。
fn error_response(id: Value, code: i32, message: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": { "code": code, "message": message }
    })
}

/// 1 メッセージを 1 行で書き出す。
fn write_message<W: Write>(out: &mut W, message: &Value) {
    let _ = serde_json::to_writer(&mut *out, message);
    let _ = out.write_all(b"\n");
    let _ = out.flush();
}

// ============================================================================
// テスト
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initialize_returns_capabilities() {
        let msg = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": { "protocolVersion": "2025-06-18" }
        });
        let res = handle_message(&msg).unwrap();
        assert_eq!(res["result"]["protocolVersion"], "2025-06-18");
        assert!(res["result"]["capabilities"]["tools"].is_object());
        assert_eq!(res["result"]["serverInfo"]["name"], "sai-link");
    }

    #[test]
    fn tools_list_has_all_tools() {
        let msg = json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" });
        let res = handle_message(&msg).unwrap();
        let tools = res["result"]["tools"].as_array().unwrap();
        let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
        for expected in [
            "sai_write", "sai_finalize", "sai_read", "task_create", "task_report",
            "task_list", "task_execute", "task_output", "task_manage", "task_status",
            "task_capture", "block_discover", "block_inspect",
        ] {
            assert!(names.contains(&expected), "{} が tools/list にありません", expected);
        }
        assert_eq!(tools.len(), 13);
        // すべてのツールが inputSchema を持つこと
        for t in tools {
            assert!(t["inputSchema"]["type"] == "object");
        }
    }

    #[test]
    fn notification_has_no_response() {
        let msg = json!({ "jsonrpc": "2.0", "method": "notifications/initialized" });
        assert!(handle_message(&msg).is_none());
    }

    #[test]
    fn unknown_method_returns_error() {
        let msg = json!({ "jsonrpc": "2.0", "id": 3, "method": "no/such" });
        let res = handle_message(&msg).unwrap();
        assert_eq!(res["error"]["code"], -32601);
    }

    #[test]
    fn unknown_tool_name_returns_is_error() {
        let msg = json!({
            "jsonrpc": "2.0", "id": 5, "method": "tools/call",
            "params": { "name": "no_such_tool", "arguments": {} }
        });
        let res = handle_message(&msg).unwrap();
        assert_eq!(res["result"]["isError"], true);
    }

    #[test]
    fn sai_finalize_requires_run_id() {
        std::env::remove_var("SAI_PARENT_RUN_ID");
        let msg = json!({
            "jsonrpc": "2.0", "id": 4, "method": "tools/call",
            "params": { "name": "sai_finalize", "arguments": {} }
        });
        let res = handle_message(&msg).unwrap();
        assert_eq!(res["result"]["isError"], true);
    }
}
