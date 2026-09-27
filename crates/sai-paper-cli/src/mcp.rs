//! `sai-paper mcp` — 標準入出力の MCP サーバー（段階 7）。
//!
//! cc（Claude Code）や pi から、RAG の検索と論文・検索記録の読み取りを道具として使う。
//! 中身は CLI の `find` / `neighbors` / `index status` / `phone paper` と同じ関数で、
//! 結果の JSON も CLI の `--json` と同じ形。1 行 1 メッセージの JSON-RPC 2.0。

use std::io::{BufRead, Write};

use anyhow::{Context, Result, anyhow};
use sai_paper_core::{Config, Database, Paths};
use serde_json::{Value, json};

/// 応答する MCP のプロトコル版（相手が別の版を言えばそれに合わせる）。
const PROTOCOL_VERSION: &str = "2025-06-18";

/// 道具の一覧（`tools/list` の応答）。
pub fn tools() -> Value {
    let index = json!({"type": "string", "description": "索引の名前（省略時は settings.json の rag.default_index）"});
    json!([
        {
            "name": "find",
            "description": "論文・ノートの断片を検索する（sai-paper find と同じ。word / vector / hybrid）",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": {"type": "string", "description": "検索語（自然言語）"},
                    "mode": {"type": "string", "enum": ["word", "vector", "hybrid"]},
                    "top_k": {"type": "integer"},
                    "per_paper": {"type": "integer", "description": "1 本の論文から出す最大の件数（既定 2、0 で制限なし）"},
                    "scope": {"type": "string", "enum": ["papers", "notes", "figures", "all"]},
                    "paper_id": {"type": "integer", "description": "この論文に絞る（Library の番号）"},
                    "index": index,
                    "evidence": {"type": "boolean", "description": "true のとき上位の結果に LLM の要約と関連度を付ける（費用がかかる。必要なときだけ）"},
                },
                "required": ["query"],
            },
        },
        {
            "name": "neighbors",
            "description": "論文に内容の近い、ほかの論文の断片を探す（sai-paper neighbors と同じ）",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "paper_id": {"type": "integer"},
                    "top_k": {"type": "integer"},
                    "index": index,
                },
                "required": ["paper_id"],
            },
        },
        {
            "name": "index_status",
            "description": "索引ごとの断片数と埋め込み数（sai-paper index status と同じ）",
            "inputSchema": {"type": "object", "properties": {}},
        },
        {
            "name": "paper",
            "description": "論文 1 件の情報（ファイルの場所・extract.md・メモ・引用文献。sai-paper phone paper と同じ）",
            "inputSchema": {
                "type": "object",
                "properties": {"paper_id": {"type": "integer"}},
                "required": ["paper_id"],
            },
        },
        {
            "name": "search_sets",
            "description": "検索の記録（検索セット）。set_id を省くと一覧、指定するとその中身",
            "inputSchema": {
                "type": "object",
                "properties": {"set_id": {"type": "integer"}},
            },
        },
    ])
}

fn int_arg(args: &Value, name: &str) -> Option<i64> {
    args.get(name).and_then(Value::as_i64)
}

fn str_arg(args: &Value, name: &str) -> Option<String> {
    args.get(name).and_then(Value::as_str).map(str::to_string)
}

fn open() -> Result<(Database, Config)> {
    let paths = Paths::discover()?;
    let db = Database::open(&paths.database_file).context("failed to open papr.db")?;
    let config = Config::load_or_create(&paths)?;
    Ok((db, config))
}

/// 道具を 1 回呼ぶ。戻り値は CLI の `--json` と同じ JSON。
pub async fn call_tool(name: &str, args: &Value) -> Result<Value> {
    match name {
        "find" => {
            let (db, config) = open()?;
            let request = sai_paper_core::retrieval::FindRequest {
                query: str_arg(args, "query").ok_or_else(|| anyhow!("query が必要です"))?,
                mode: str_arg(args, "mode"),
                top_k: int_arg(args, "top_k"),
                per_paper: int_arg(args, "per_paper"),
                scope: str_arg(args, "scope"),
                paper_id: int_arg(args, "paper_id"),
                index: str_arg(args, "index"),
                ..Default::default()
            };
            let evidence = args
                .get("evidence")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            // 要約付きのときだけ LLM のクライアントを作る（`model` が空ならここでエラー）。
            let chat = if evidence {
                Some(sai_paper_core::evidence::ChatCompletionsClient::from_config(&config)?)
            } else {
                None
            };
            let outcome = sai_paper_core::evidence::find_maybe_evidence(
                db.connection(),
                &config,
                request,
                evidence,
                chat.as_ref()
                    .map(|c| c as &dyn sai_paper_core::evidence::ChatClient),
            )
            .await?;
            match outcome {
                sai_paper_core::evidence::SearchOutcome::Plain(hits) => {
                    Ok(sai_paper_core::retrieval::hits_to_json(&hits))
                }
                sai_paper_core::evidence::SearchOutcome::Evidence(result) => {
                    Ok(serde_json::to_value(&result)?)
                }
            }
        }
        "neighbors" => {
            let (db, config) = open()?;
            let paper_id = int_arg(args, "paper_id").ok_or_else(|| anyhow!("paper_id が必要です"))?;
            let hits = sai_paper_core::retrieval::find_neighbors(
                db.connection(),
                &config,
                paper_id,
                int_arg(args, "top_k"),
                str_arg(args, "index").as_deref(),
            )?;
            Ok(sai_paper_core::retrieval::hits_to_json(&hits))
        }
        "index_status" => {
            let (db, config) = open()?;
            crate::index::status_json(&db, &config)
        }
        "paper" => {
            let paper_id =
                int_arg(args, "paper_id").ok_or_else(|| anyhow!("paper_id が必要です"))?;
            let value = crate::phone::paper(paper_id)?;
            // エージェントが論文を読んだ記録（失敗しても本文の出力は止めない）。
            let extract = value
                .get("files")
                .and_then(|files| files.get("extract"))
                .and_then(Value::as_str);
            crate::activity::record_agent_read(
                "mcp",
                "paper",
                Some(paper_id),
                extract.map(std::path::Path::new),
            );
            Ok(value)
        }
        "search_sets" => {
            let (db, _) = open()?;
            let conn = db.connection();
            match int_arg(args, "set_id") {
                Some(set_id) => Ok(json!({
                    "set": sai_paper_core::search_sets::get(conn, set_id)?,
                    "items": sai_paper_core::search_sets::items(conn, set_id)?,
                })),
                None => Ok(serde_json::to_value(sai_paper_core::search_sets::list(conn)?)?),
            }
        }
        other => Err(anyhow!("知らない道具です: {other}")),
    }
}

/// JSON-RPC の要求 1 つに答える（通知なら `None`）。
pub async fn handle_message(message: &Value) -> Option<Value> {
    let id = message.get("id")?.clone();
    let method = message.get("method").and_then(Value::as_str).unwrap_or("");
    let params = message.get("params").cloned().unwrap_or(Value::Null);
    let result = match method {
        "initialize" => Ok(json!({
            "protocolVersion": params
                .get("protocolVersion")
                .and_then(Value::as_str)
                .unwrap_or(PROTOCOL_VERSION),
            "capabilities": {"tools": {}},
            "serverInfo": {"name": "sai-paper", "version": env!("CARGO_PKG_VERSION")},
        })),
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({"tools": tools()})),
        "tools/call" => {
            let name = params.get("name").and_then(Value::as_str).unwrap_or("");
            let args = params.get("arguments").cloned().unwrap_or_else(|| json!({}));
            // 道具の失敗は MCP の決まりどおり isError の結果として返す（接続は切らない）。
            Ok(match call_tool(name, &args).await {
                Ok(value) => json!({
                    "content": [{"type": "text", "text": serde_json::to_string_pretty(&value).unwrap_or_default()}],
                }),
                Err(error) => json!({
                    "content": [{"type": "text", "text": format!("{error:#}")}],
                    "isError": true,
                }),
            })
        }
        _ => Err(json!({"code": -32601, "message": format!("method not found: {method}")})),
    };
    Some(match result {
        Ok(result) => json!({"jsonrpc": "2.0", "id": id, "result": result}),
        Err(error) => json!({"jsonrpc": "2.0", "id": id, "error": error}),
    })
}

/// 標準入力から 1 行ずつ読み、標準出力に 1 行ずつ答える。
pub async fn serve() -> Result<()> {
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let reply = match serde_json::from_str::<Value>(&line) {
            Ok(message) => handle_message(&message).await,
            Err(e) => Some(json!({
                "jsonrpc": "2.0", "id": null,
                "error": {"code": -32700, "message": format!("parse error: {e}")},
            })),
        };
        if let Some(reply) = reply {
            writeln!(stdout, "{reply}")?;
            stdout.flush()?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// tools/list に 5 つの道具が並び、どれも入力の形を持つ。
    #[tokio::test]
    async fn tools_list_has_all_tools() {
        let reply = handle_message(&json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}))
            .await
            .unwrap();
        let names: Vec<&str> = reply["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|tool| {
                assert!(tool["inputSchema"].is_object());
                tool["name"].as_str().unwrap()
            })
            .collect();
        assert_eq!(names, ["find", "neighbors", "index_status", "paper", "search_sets"]);
    }

    /// find の入力に `per_paper`（1 本の論文から出す最大件数）がある。
    #[test]
    fn find_tool_exposes_per_paper() {
        let listed = tools();
        let find = listed
            .as_array()
            .unwrap()
            .iter()
            .find(|tool| tool["name"] == "find")
            .unwrap();
        assert!(find["inputSchema"]["properties"]["per_paper"].is_object());
    }

    /// initialize は相手の版に合わせ、通知には答えない。
    #[tokio::test]
    async fn initialize_and_notifications() {
        let reply = handle_message(&json!({
            "jsonrpc": "2.0", "id": 0, "method": "initialize",
            "params": {"protocolVersion": "2025-03-26"},
        }))
        .await
        .unwrap();
        assert_eq!(reply["result"]["protocolVersion"], "2025-03-26");
        assert_eq!(reply["result"]["serverInfo"]["name"], "sai-paper");
        assert!(
            handle_message(&json!({"jsonrpc": "2.0", "method": "notifications/initialized"}))
                .await
                .is_none()
        );
    }

    /// find の引数が足りないときは、接続を切らずに isError の結果で返す。
    #[tokio::test]
    async fn find_without_query_is_tool_error() {
        let reply = handle_message(&json!({
            "jsonrpc": "2.0", "id": 2, "method": "tools/call",
            "params": {"name": "find", "arguments": {}},
        }))
        .await
        .unwrap();
        assert_eq!(reply["result"]["isError"], true);
        assert!(reply["result"]["content"][0]["text"].as_str().unwrap().contains("query"));
    }
}
