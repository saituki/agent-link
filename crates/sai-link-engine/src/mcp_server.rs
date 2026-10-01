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
//! ## 待つ道具の扱い（task_output）
//!
//! `task_output` だけは待ちになるので、**この 1 つだけ別スレッド**で動かす。
//! 他の道具は短く終わるので読み取りスレッドで順に処理し、同じターン内の呼び出し順
//! （`sai_write` → `sai_finalize`、`task_create` → `task_execute`）を保つ。
//! 待ちのあいだも `task_list` などの別の呼び出しはすぐ返る。
//!
//! - 待ちのあいだに取り消し（`notifications/cancelled`）が来たらフラグを立てて止める。
//!   応答は**書かない**（MCP 仕様どおり）。結果ファイルも消さないので、あとから読み直せる。
//! - stdin が EOF になったら、残りの待ちをすべて止めてから終わる。
//! - 返答は出力のロックを取って 1 行ずつ書くので、順番が入れ替わらない。
//!
//! 公開ツール名は `cc_command::MCP_TOOL_NAMES` と 1 個ずつ完全一致させること。

use std::collections::HashMap;
use std::io::{BufRead, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use serde_json::{json, Value};

/// MCP のプロトコルバージョン（initialize でクライアント値が無い場合の既定）。
const DEFAULT_PROTOCOL_VERSION: &str = "2024-11-05";

/// `task_ids` で受け取れる最大の件数（多すぎると 1 回の待いで全部を見られない）。
const MAX_TASK_IDS: usize = 50;

/// 応答を待つあいだ他の呼び出しを止めないよう、別スレッドで動かす道具。
///
/// 今のところ `task_output` だけ。短い道具は読み取りスレッドで処理する。
fn is_long_running_tool(name: &str) -> bool {
    name == "task_output"
}

/// stdio で MCP サーバを実行する（`sai-link mcp serve` から呼ばれる）。
///
/// stdin を 1 行ずつ読み、JSON-RPC を処理して stdout に 1 行で応答する。
/// 通知（id なし）には応答しない。
pub fn serve_stdio() {
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    serve(stdin.lock(), stdout);
}

/// MCP サーバ本体（読み取りは `reader`、書き込みは `writer`）。
///
/// 待つ道具（`is_long_running_tool`）だけを別スレッドで動かすので、
/// 待たされているあいだも他の呼び出しが返る。すべてのスレッドはここで終わる。
pub fn serve<R: BufRead, W: Write + Send>(reader: R, writer: W) {
    let out = Mutex::new(writer);
    // 待つ呼び出しの id と、取り消し用のフラグ
    let pending: Mutex<HashMap<String, Arc<AtomicBool>>> = Mutex::new(HashMap::new());

    std::thread::scope(|s| {
        for line in reader.lines() {
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
                    let err =
                        error_response(Value::Null, -32700, &format!("JSON パース失敗: {}", e));
                    write_message_locked(&out, &err);
                    continue;
                }
            };

            let method = message.get("method").and_then(|m| m.as_str()).unwrap_or("");
            let params = message.get("params").cloned().unwrap_or(Value::Null);

            // 取り消し: 待っているスレッドのフラグを立てる（応答は書かない）
            if method == "notifications/cancelled" {
                if let Some(request_id) = params.get("requestId") {
                    let key = request_id.to_string();
                    if let Some(flag) = lock(&pending).remove(&key) {
                        flag.store(true, Ordering::Relaxed);
                    }
                }
                continue;
            }

            let tool_name = params.get("name").and_then(|v| v.as_str()).unwrap_or("");
            if method == "tools/call" && is_long_running_tool(tool_name) {
                let id = match message.get("id") {
                    Some(id) => id.clone(),
                    None => continue, // 応答しない呼び出しなので待たせない
                };
                // フラグは読み取りスレッドで登録する（直後の取り消しと取り合いにならないように）
                let key = id.to_string();
                let cancel = Arc::new(AtomicBool::new(false));
                lock(&pending).insert(key.clone(), Arc::clone(&cancel));
                let out = &out;
                let pending = &pending;
                s.spawn(move || {
                    let response = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        call_tool_with(&params, cancel.as_ref())
                    })) {
                        Ok(r) => Some(success_or_error(&id, r)),
                        Err(_) => Some(error_response(
                            id.clone(),
                            -32603,
                            "内部エラー: 道具の処理が異常終了しました",
                        )),
                    };
                    // pending から取り出せたときだけ応答する（取れなければ取り消し済み）
                    let still_wanted = lock(pending).remove(&key).is_some();
                    if still_wanted {
                        if let Some(response) = response {
                            write_message_locked(out, &response);
                        }
                    }
                });
                continue;
            }

            if let Some(response) = handle_message_with(&message, &AtomicBool::new(false)) {
                write_message_locked(&out, &response);
            }
        }

        // EOF: まだ待っている呼び出しをすべて止めてから終わる
        for flag in lock(&pending).values() {
            flag.store(true, Ordering::Relaxed);
        }
    });
}

/// 1 メッセージを処理して応答を返す（通知なら `None`）。
pub fn handle_message(message: &Value) -> Option<Value> {
    handle_message_with(message, &AtomicBool::new(false))
}

/// `handle_message` の本体。`cancel` は待つ道具（`task_output`）の待ちを止めるフラグ。
fn handle_message_with(message: &Value, cancel: &AtomicBool) -> Option<Value> {
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
        "tools/call" => call_tool_with(&params, cancel),
        _ => Err((-32601, format!("未対応のメソッドです: {}", method))),
    };

    Some(success_or_error(&id, result))
}

/// 処理結果を MCP の応答（`result` か `error`）にまとめる。
fn success_or_error(id: &Value, result: Result<Value, (i32, String)>) -> Value {
    match result {
        Ok(value) => json!({ "jsonrpc": "2.0", "id": id, "result": value }),
        Err((code, message)) => error_response(id.clone(), code, &message),
    }
}

/// ロックが汚染されていても（別スレッドでパニックが起きた）拿到了まま使い続ける。
fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
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
            "description": "実行を完了する。report（要点）を付けると run の「報告」として残り、Web の run の画面で読める。子の run は task_report を使う。",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "run_id": { "type": "integer", "description": "完了する実行 ID（省略時は SAI_PARENT_RUN_ID）" },
                    "report": { "type": "string", "description": "要点（結論・保存した文書のトークン・残った問い）。省略可" }
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
            "description": "新しいサブタスクを作成する。承認は不要で、指定したタスクはそのまま作成される。block_type（task.md の block と同じ）には block_discover で確認した id を指定する。task_text は子ブロックの {{input.task_text}} に入るテキストで、文中の waggle:<トークン> は実行時に文書の本文へ置き換わる。description（task.md の説明・省略時は task_text と同じ）を付けられる。cwd で作業フォルダ（別プロジェクト等）、worktree で作業用コピーの有無を指定できる。card（整数）を渡すと、todo のカード（executer: professor）を自分の子として起こす（予約済みの run_id と task.md をそのまま使う。task_text・block_type は書かない）。実行すると子の run フォルダに task.md（description と task_text）が保存され、TUI / Web で見られる。",
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
                                "task_text": { "type": "string", "description": "子ブロックの最初のメッセージの「# task_text」の節に入るテキスト。文中の waggle:<トークン> は実行時に文書の本文へ置き換わる" },
                                "card": { "type": "integer", "description": "todo のカードの run_id。executer: professor のカードを、自分の子として起こす。予約済みの run_id と task.md（sci_* を含む）をそのまま使う。task_text は書かない" },
                                "block_type": { "type": "string", "description": "使用するブロック定義の id（表示名ではなく id。task.md の block と同じ）" },
                                "block": { "type": "string", "description": "block_type と同じ（task.md 互換の別名）" },
                                "description": { "type": "string", "description": "タスクの説明（task.md の description。省略時は task_text と同じ）" },
                                "depends_on": { "type": "array", "items": { "type": "string" }, "description": "依存するタスクの client_id（同一バッチ内）" },
                                "cwd": { "type": "string", "description": "サブタスクの作業フォルダ。絶対パス・~/ 始まり・プロジェクトルートからの相対パス。存在するフォルダであること。省略時はプロジェクトルート" },
                                "timeout": { "type": "integer", "description": "子の時間上限（秒）。ブロック定義の timeout を上書きする。超えてもタスクは失敗にせず、親の task_output に stalled(reason=timeout) を返す。完了しないあいだは同じ間隔で繰り返し知らせる。省略時はブロック定義に従う" },
                                "worktree": { "type": "string", "enum": ["isolation", "none"], "description": "作業用コピー（git worktree）の指定。未指定ならブロック定義の worktree に従う。isolation なら cwd を含むリポジトリの .worktrees/<block>-<run_id> を作る。none なら作らない（ブロック定義の isolation も打ち消す）" },
                                "carried_over": { "type": "boolean", "description": "前バッチからの引き継ぎタスク" }
                            },
                            "required": ["client_id", "subject"]
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
            "description": "タスクの結果を取得する。1 回の待ちは最大 90 秒で、超えると status=in_progress で返る（もう一度呼ぶ）。task_ids を渡すと複数の子を 1 回で待ち、最初に完了・stalled（質問を含む）になった子を 1 件返し、残りを pending で返す。返った子は次の呼び出しの task_ids から外すこと。",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "task_id": { "type": "integer", "description": "タスクの server_id（task_ids とどちらか一方）" },
                    "task_ids": { "type": "array", "items": { "type": "integer" }, "description": "複数の子を待つ server_id の配列（task_id とどちらか一方）。最大 50 件" },
                    "timeout_ms": { "type": "integer", "description": "最大待機時間（ミリ秒）。上限 90000（超える値は 90000 に切り詰める）。既定 90000。0 で待機なし" }
                }
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
// 取り消しなしで呼ぶ口（待ちがない側で使う）。
#[allow(dead_code)]
fn call_tool(params: &Value) -> Result<Value, (i32, String)> {
    call_tool_with(params, &AtomicBool::new(false))
}

/// `call_tool` の本体。`cancel` は `task_output` の待ちを止めるフラグ。
///
/// 外から来る値（`task_id` / `task_ids`）はここで検査する。
fn call_tool_with(params: &Value, cancel: &AtomicBool) -> Result<Value, (i32, String)> {
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
        "task_output" => run_task_output(&arguments, cancel),
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

/// `task_output` の引数を検査して待つ。
///
/// `task_id`（単体）と `task_ids`（複数待ち）はどちらか一方だけ。検査は
/// ここ（外から来る値の検査は窓口で行うという約束）で行い、
/// 通った値だけを `agent_api` に渡す。
fn run_task_output(arguments: &Value, cancel: &AtomicBool) -> Result<Value, String> {
    let one = arguments.get("task_id").and_then(|v| v.as_i64());
    let many = arguments.get("task_ids");
    let timeout_ms = arguments.get("timeout_ms").and_then(|v| v.as_u64());
    match (one, many) {
        (Some(_), Some(_)) => Err("task_id と task_ids は同時に指定できません".to_string()),
        (None, None) => Err("task_id または task_ids は必須です".to_string()),
        (Some(id), None) => crate::agent_api::task_output_ids(&[id], timeout_ms, cancel, false),
        (None, Some(Value::Array(items))) => {
            if items.is_empty() {
                return Err("task_ids は 1 件以上の整数の配列にしてください".to_string());
            }
            if items.len() > MAX_TASK_IDS {
                return Err(format!("task_ids は {} 件までです", MAX_TASK_IDS));
            }
            let mut ids = Vec::with_capacity(items.len());
            for item in items {
                ids.push(item.as_i64().ok_or("task_ids は 1 件以上の整数の配列にしてください")?);
            }
            crate::agent_api::task_output_ids(&ids, timeout_ms, cancel, true)
        }
        (None, Some(_)) => Err("task_ids は 1 件以上の整数の配列にしてください".to_string()),
    }
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
    let report = arguments.get("report").and_then(|v| v.as_str());
    let result = crate::agent_api::finalize(run_id, report)?;
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

/// 出力のロックを取って 1 メッセージを書き出す（別スレッドからも使う）。
fn write_message_locked<W: Write>(out: &Mutex<W>, message: &Value) {
    let mut guard = lock(out);
    write_message(&mut *guard, message);
}

// ============================================================================
// テスト
// ============================================================================

#[cfg(test)]
mod tests {
    // ここで動かすテストは環境変数（SAI_PROJECT_ROOT など）を書き換えない。
    // 書き換えると、並列に走る orchestrator のテスト（ロックを取らない
    // `prepare_pipeline` 系）とぶつかって落ちるので、DB を触らない呼び出しで確かめる。
    use super::*;
    use std::io::Read;
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    /// テスト用の読み取り側。チャネルから 1 行ずつ受け取る（閉じると EOF）。
    struct ChannelReader {
        rx: mpsc::Receiver<String>,
        buf: Vec<u8>,
    }

    impl ChannelReader {
        fn new(rx: mpsc::Receiver<String>) -> Self {
            Self {
                rx,
                buf: Vec::new(),
            }
        }

        /// 次の 1 行（改行付き）を buf に入れる。無ければ `false`。
        fn fill_next_line(&mut self) -> bool {
            match self.rx.recv() {
                Ok(line) => {
                    let mut bytes = format!("{}\n", line).into_bytes();
                    self.buf.append(&mut bytes);
                    true
                }
                Err(_) => false,
            }
        }
    }

    impl Read for ChannelReader {
        fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
            if self.buf.is_empty() && !self.fill_next_line() {
                return Ok(0);
            }
            let n = self.buf.len().min(out.len());
            out[..n].copy_from_slice(&self.buf[..n]);
            self.buf.drain(..n);
            Ok(n)
        }
    }

    // `lines()` を使えるようにするため BufRead も実装する（1 行ずつ返す）。
    impl BufRead for ChannelReader {
        fn fill_buf(&mut self) -> std::io::Result<&[u8]> {
            if self.buf.is_empty() {
                self.fill_next_line();
            }
            Ok(&self.buf)
        }

        fn consume(&mut self, amt: usize) {
            let amt = amt.min(self.buf.len());
            self.buf.drain(..amt);
        }
    }

    /// テスト用の書き込み側。書いた内容を共有バッファに残す。
    struct SharedWriter {
        buf: Arc<Mutex<Vec<u8>>>,
    }

    impl Write for SharedWriter {
        fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
            lock(&self.buf).extend_from_slice(data);
            Ok(data.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    /// テスト用に動かしたサーバ。チャネルで入力を流し、出力を共有バッファで受け取る。
    struct ServeTest {
        tx: Option<mpsc::Sender<String>>,
        done: mpsc::Receiver<()>,
        out: Arc<Mutex<Vec<u8>>>,
        handle: Option<std::thread::JoinHandle<()>>,
    }

    impl ServeTest {
        fn start() -> Self {
            let (tx, rx) = mpsc::channel::<String>();
            let (done_tx, done_rx) = mpsc::channel::<()>();
            let out = Arc::new(Mutex::new(Vec::new()));
            let writer_out = Arc::clone(&out);
            let handle = std::thread::spawn(move || {
                serve(ChannelReader::new(rx), SharedWriter { buf: writer_out });
                let _ = done_tx.send(());
            });
            Self {
                tx: Some(tx),
                done: done_rx,
                out,
                handle: Some(handle),
            }
        }

        /// 1 メッセージを 1 行として流す。
        fn send(&self, message: Value) {
            self.tx
                .as_ref()
                .expect("送信口は閉じられていない")
                .send(message.to_string())
                .unwrap();
        }

        /// 今までの出力を文字列にする。
        fn output(&self) -> String {
            String::from_utf8(lock(&self.out).clone()).expect("出力は UTF-8")
        }

        /// 指定した id の応答があれば返す（1 行ずつ JSON として読めるものだけ）。
        fn response_of(&self, id: i64) -> Option<Value> {
            self.output()
                .lines()
                .filter_map(|l| serde_json::from_str::<Value>(l).ok())
                .find(|v| v["id"] == id)
        }

        /// 指定した id の応答が書かれるまで待つ。時間切れなら `None`。
        fn wait_for_id(&self, id: i64, limit: Duration) -> Option<Value> {
            let deadline = Instant::now() + limit;
            loop {
                if let Some(value) = self.response_of(id) {
                    return Some(value);
                }
                if Instant::now() >= deadline {
                    return None;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
        }

        /// 入力を閉じて（EOF）サーバが終わるのを待つ。時間内に終われば `true`。
        fn finish(&mut self, limit: Duration) -> bool {
            self.tx.take();
            let finished = self.done.recv_timeout(limit).is_ok();
            if let Some(handle) = self.handle.take() {
                let _ = handle.join();
            }
            finished
        }
    }

    /// 待つ `task_output` を 1 行にする（id と timeout_ms を指定）。
    fn call_task_output(id: i64, task_id: i64, timeout_ms: i64) -> Value {
        json!({
            "jsonrpc": "2.0", "id": id, "method": "tools/call",
            "params": { "name": "task_output",
                        "arguments": { "task_id": task_id, "timeout_ms": timeout_ms } }
        })
    }

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

    /// `sai_finalize` は任意の `report` を受け取る（報告は run の「報告」として残る）。
    #[test]
    fn sai_finalize_schema_has_optional_report() {
        let list = tools_list_result();
        let tool = list["tools"]
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["name"] == "sai_finalize")
            .expect("sai_finalize がある");
        let props = &tool["inputSchema"]["properties"];
        assert!(props["report"].is_object(), "report が無い: {}", tool);
        assert_eq!(props["report"]["type"], "string");
        assert!(
            tool["inputSchema"].get("required").is_none(),
            "report は任意: {}",
            tool["inputSchema"]
        );
    }

    /// AC1: 待たれている `task_output` があっても、短い呼び出しは 3 秒以内に返る。
    /// 取り消しのあとに応答が書かれないことも確かめる（AC4）。
    ///
    /// 短い呼び出しは DB を触らないもの（`ping` と、見慣れない道具名）を使う。
    /// 返るのは読み取りスレッドの処理なので、どの道具でも同じ道理。
    #[test]
    fn waiting_task_output_does_not_block_other_calls() {
        let mut server = ServeTest::start();
        // 存在しないタスクを 60 秒待つ（上限の 90 秒には届かない）
        server.send(call_task_output(1, 999_999, 60_000));
        server.send(json!({ "jsonrpc": "2.0", "id": 2, "method": "ping" }));
        server.send(json!({
            "jsonrpc": "2.0", "id": 3, "method": "tools/call",
            "params": { "name": "no_such_tool", "arguments": {} }
        }));

        assert!(
            server.wait_for_id(2, Duration::from_secs(3)).is_some(),
            "ping が 3 秒以内に返る"
        );
        assert!(
            server.wait_for_id(3, Duration::from_secs(3)).is_some(),
            "tools/call が 3 秒以内に返る"
        );
        assert!(
            server.response_of(1).is_none(),
            "task_output はまだ待たしている（応答が無い）"
        );

        // 取り消しを送る → 待ちが終わる。MCP 仕様どおり応答は書かれない。
        server.send(json!({
            "jsonrpc": "2.0", "method": "notifications/cancelled",
            "params": { "requestId": 1 }
        }));
        assert!(server.finish(Duration::from_secs(3)), "serve が 3 秒以内に終わる");
        assert!(
            server.response_of(1).is_none(),
            "取り消された task_output には応答しない: {}",
            server.output()
        );
    }

    /// 待っている `task_output` があっても、stdin が閉じた（EOF）ら 3 秒以内に終わる。
    #[test]
    fn serve_returns_on_eof_while_task_output_is_waiting() {
        let mut server = ServeTest::start();
        server.send(call_task_output(1, 999_999, 60_000));
        // 別スレッドの待ちが始まるのを少し待つ
        std::thread::sleep(Duration::from_millis(200));
        assert!(server.finish(Duration::from_secs(3)), "EOF で 3 秒以内に終わる");
    }

    /// AC5: 短い道具と待つ道具が同時に走っても、書き込んだ行は 1 行ずつに壊れない。
    /// 短い道具の応答には id が重複せず全部そろう。
    #[test]
    fn output_lines_stay_one_json_per_line() {
        let mut server = ServeTest::start();
        for id in 1..=50 {
            server.send(json!({
                "jsonrpc": "2.0", "id": id, "method": "tools/call",
                "params": { "name": "no_such_tool", "arguments": {} }
            }));
        }
        // 待たない task_output（timeout_ms = 0）を混ぜる
        server.send(call_task_output(1000, 999_999, 0));
        assert!(server.finish(Duration::from_secs(10)), "全部終わる");

        let output = server.output();
        let lines: Vec<&str> = output.lines().filter(|l| !l.is_empty()).collect();
        assert_eq!(lines.len(), 51, "51 件の応答がある");
        let mut ids = Vec::new();
        for line in lines {
            let value: Value =
                serde_json::from_str(line).unwrap_or_else(|e| panic!("1 行ずつ JSON: {}", e));
            ids.push(value["id"].as_i64().expect("id がある"));
        }
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), 51, "id が重複しない");
    }

    /// AC5: 短い道具（読み取りスレッドで処理される）の応答は送った順に並ぶ。
    /// 道具は `task_manage`（引数が足りずエラーになるだけのもの）を使う。
    #[test]
    fn short_tool_responses_keep_send_order() {
        let mut server = ServeTest::start();
        for id in 1..=2 {
            server.send(json!({
                "jsonrpc": "2.0", "id": id, "method": "tools/call",
                "params": { "name": "task_manage", "arguments": {} }
            }));
        }
        assert!(server.finish(Duration::from_secs(10)));

        let output = server.output();
        let lines: Vec<&str> = output.lines().filter(|l| !l.is_empty()).collect();
        assert_eq!(lines.len(), 2);
        let first: Value = serde_json::from_str(lines[0]).unwrap();
        let second: Value = serde_json::from_str(lines[1]).unwrap();
        assert_eq!(first["id"], 1, "先に送ったものが先に返る");
        assert_eq!(second["id"], 2);
        // 引数が足りないのでエラーになる（= 実際の処理はしていない）
        assert_eq!(first["result"]["isError"], true);
    }

    /// `task_output` の引数の検査。task_id / task_ids の両方、どちらも無し、
    /// 空配列、文字列を含む配列、51 件はすべてエラーになる。
    #[test]
    fn task_output_rejects_bad_arguments() {
        let cases = vec![
            json!({ "task_id": 1, "task_ids": [1, 2] }),
            json!({}),
            json!({ "task_ids": [] }),
            json!({ "task_ids": ["1"] }),
            json!({ "task_ids": (1..=51).collect::<Vec<i64>>() }),
        ];
        for arguments in cases {
            let msg = json!({
                "jsonrpc": "2.0", "id": 9, "method": "tools/call",
                "params": { "name": "task_output", "arguments": arguments }
            });
            let res = handle_message(&msg).unwrap();
            assert_eq!(
                res["result"]["isError"], true,
                "エラーになるはず: arguments={}",
                arguments
            );
        }
    }

    /// tools/list の task_output に task_ids があり、required に task_id が無いこと。
    #[test]
    fn task_output_schema_accepts_task_ids() {
        let list = tools_list_result();
        let tool = list["tools"]
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["name"] == "task_output")
            .expect("task_output がある");
        let props = &tool["inputSchema"]["properties"];
        assert_eq!(props["task_ids"]["type"], "array", "task_ids がある: {}", tool);
        assert_eq!(props["task_ids"]["items"]["type"], "integer");
        assert_eq!(props["timeout_ms"]["type"], "integer");
        assert!(
            tool["inputSchema"].get("required").is_none(),
            "task_id は必須ではない: {}",
            tool["inputSchema"]
        );
    }
}
