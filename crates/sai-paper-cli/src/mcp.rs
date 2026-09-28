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
                    "papers": {"type": "array", "items": {"type": "integer"}, "description": "論文の集合で絞る（paper_id と足して使う）"},
                    "node": {"type": "string", "description": "節点に結び付いた論文に絞る（N12）"},
                    "subtree": {"type": "boolean", "description": "node の子孫の節点も含める（node と一緒に使う）"},
                    "each": {"type": "integer", "description": "範囲の論文ごとに上位 m 件を返す（papers か node と一緒に使う）"},
                    "question": {"type": "string", "description": "検索の問い（Q7）。記録に残り、前の検索を見られる"},
                    "widen": {"type": "string", "description": "範囲を広げた理由"},
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
        {
            "name": "sci_context",
            "description": "今の実行の範囲（研究方針の木から切り出した入力）を返す",
            "inputSchema": {"type": "object", "properties": {
                "run_id": {"type": "integer", "description": "省略時は環境変数 SAI_PARENT_RUN_ID"},
                "scope": {"type": "string", "enum": ["node", "path", "subtree", "siblings", "summary", "blind"]},
            }},
        },
        {
            "name": "sci_status",
            "description": "研究方針の木の全体（種の文・関門・項目・負債・次の一手）",
            "inputSchema": {"type": "object", "properties": {}},
        },
        {
            "name": "sci_node",
            "description": "節点 1 つの中身（問い・候補・知見・結び付き・決定の記録・行動）",
            "inputSchema": {"type": "object", "properties": {"node": {"type": "string", "description": "N12 の形"}}, "required": ["node"]},
        },
        {
            "name": "sci_propose",
            "description": "研究方針の木への変更を提案する。結び付け・問い・知見はすぐ入り、ほかは利用者が仕分ける",
            "inputSchema": {"type": "object", "properties": {
                "proposals": {"type": "array", "items": {"type": "object"}, "description": "sci-core の決まりの op の JSON"},
                "run_id": {"type": "integer", "description": "省略時は環境変数 SAI_PARENT_RUN_ID"},
            }, "required": ["proposals"]},
        },
    ])
}

fn int_arg(args: &Value, name: &str) -> Option<i64> {
    args.get(name).and_then(Value::as_i64)
}

fn str_arg(args: &Value, name: &str) -> Option<String> {
    args.get(name).and_then(Value::as_str).map(str::to_string)
}

/// 整数の配列を読む（`find` の `papers`）。
fn int_list_arg(args: &Value, name: &str) -> Vec<i64> {
    args.get(name)
        .and_then(Value::as_array)
        .map(|items| items.iter().filter_map(Value::as_i64).collect())
        .unwrap_or_default()
}

fn open() -> Result<(Database, Config)> {
    let paths = Paths::discover()?;
    let db = Database::open(&paths.database_file).context("failed to open papr.db")?;
    let config = Config::load_or_create(&paths)?;
    Ok((db, config))
}

/// 検索結果から论文の番号を取り出す（重複なし・昇順。検索の記録に残す）。
fn paper_ids_of(hits: &[sai_paper_core::Hit]) -> Vec<i64> {
    let mut ids: Vec<i64> = hits.iter().filter_map(|hit| hit.paper_id).collect();
    ids.sort_unstable();
    ids.dedup();
    ids
}

/// 道具を 1 回呼ぶ。戻り値は CLI の `--json` と同じ JSON。
pub async fn call_tool(name: &str, args: &Value) -> Result<Value> {
    match name {
        "find" => {
            let (db, config) = open()?;
            let conn = db.connection();
            let query = str_arg(args, "query").ok_or_else(|| anyhow!("query が必要です"))?;
            // 節点と問い（記録に残す）。節点だけなら検索の範囲も狭める。
            let refs = crate::sci::parse_search_refs(
                conn,
                str_arg(args, "node").as_deref(),
                str_arg(args, "question").as_deref(),
            )?;
            let papers = int_list_arg(args, "papers");
            let subtree = args.get("subtree").and_then(Value::as_bool).unwrap_or(false);
            // `--subtree` と `--each` には範囲の指定が要る（CLI の find と同じ検査）。
            crate::sci::check_find_scope(subtree, refs.given.is_some(), int_arg(args, "each"), papers.is_empty())?;
            let (paper_ids, scope_note) = sai_paper_core::retrieval::resolve_paper_scope(
                conn,
                int_arg(args, "paper_id"),
                &papers,
                refs.given,
                subtree,
            )
            .map_err(|err| anyhow!("{err}"))?;
            let request = sai_paper_core::retrieval::FindRequest {
                query: query.clone(),
                mode: str_arg(args, "mode"),
                top_k: int_arg(args, "top_k"),
                per_paper: int_arg(args, "per_paper"),
                scope: str_arg(args, "scope"),
                paper_id: None,
                paper_ids,
                each: int_arg(args, "each"),
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
                conn,
                &config,
                request,
                evidence,
                chat.as_ref()
                    .map(|c| c as &dyn sai_paper_core::evidence::ChatClient),
            )
            .await?;
            // 節点か問いがあれば検索の記録に残す（CLI の find と同じ処理）。
            let (count, hit_papers) = match &outcome {
                sai_paper_core::evidence::SearchOutcome::Plain(hits) => {
                    (hits.len() as i64, paper_ids_of(hits))
                }
                sai_paper_core::evidence::SearchOutcome::Evidence(result) => {
                    let hits: Vec<_> = result.hits.iter().map(|item| item.hit.clone()).collect();
                    (hits.len() as i64, paper_ids_of(&hits))
                }
            };
            crate::sci::record_search(
                conn,
                &crate::sci::SearchLog {
                    refs: &refs,
                    tool: "find",
                    query: &query,
                    scope: &scope_note,
                    widen_reason: str_arg(args, "widen").as_deref().unwrap_or(""),
                    hits: count,
                    hit_papers: &hit_papers,
                    set_id: None,
                },
            )?;
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
        "sci_context" => {
            let env = crate::sci::open_env()?;
            let run = int_arg(args, "run_id").or_else(crate::sci::parent_run_id);
            let Some(run_id) = run else {
                return Err(anyhow!("run_id が必要です"));
            };
            let text = sai_paper_core::sci::context::build(
                env.conn(),
                &env.roots,
                &env.defs,
                env.config.sci.context_max_chars,
                run_id,
                str_arg(args, "scope").as_deref(),
            )
            .context("入力の範囲を組み立てられませんでした")?;
            Ok(json!({ "text": text }))
        }
        "sci_status" => {
            let env = crate::sci::open_env()?;
            let status = sai_paper_core::sci::compute::status(env.conn(), &env.roots, &env.defs)
                .context("現況を集められませんでした")?;
            Ok(serde_json::to_value(status)?)
        }
        "sci_node" => {
            let env = crate::sci::open_env()?;
            let node = str_arg(args, "node").ok_or_else(|| anyhow!("node が必要です"))?;
            let id = crate::sci::parse_id(&node, 'N')?;
            let detail =
                sai_paper_core::sci::compute::node_detail(env.conn(), &env.roots, &env.defs, id)
                    .context("節点の詳細を集められませんでした")?;
            Ok(serde_json::to_value(detail)?)
        }
        "sci_propose" => {
            let proposals = args
                .get("proposals")
                .and_then(Value::as_array)
                .ok_or_else(|| anyhow!("proposals が必要です"))?;
            let env = crate::sci::open_env()?;
            let actor = crate::sci::actor_of(
                int_arg(args, "run_id"),
                std::env::var("SAI_BLOCK_NAME").ok(),
            );
            let results =
                sai_paper_core::sci::ops::propose(env.conn(), &env.roots, &env.defs, &actor, &json!({
                    "proposals": proposals.clone(),
                }))
                .map_err(|err| anyhow!("{err}"))?;
            env.export()?;
            Ok(json!({ "results": results }))
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

    /// tools/list に 9 つの道具が並び、どれも入力の形を持つ。
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
        assert_eq!(
            names,
            [
                "find",
                "neighbors",
                "index_status",
                "paper",
                "search_sets",
                "sci_context",
                "sci_status",
                "sci_node",
                "sci_propose"
            ]
        );
    }

    /// find の入力に範囲を絞る引数がある（papers / node / subtree / each / question）。
    #[test]
    fn find_tool_exposes_scope_arguments() {
        let listed = tools();
        let find = listed
            .as_array()
            .unwrap()
            .iter()
            .find(|tool| tool["name"] == "find")
            .unwrap();
        for name in ["papers", "node", "subtree", "each", "question"] {
            assert!(
                find["inputSchema"]["properties"][name].is_object(),
                "find に {name} が無い"
            );
        }
    }

    /// sci_propose に proposals が無いときは isError で返す（接続は切らない）。
    #[tokio::test]
    async fn sci_propose_requires_proposals() {
        let reply = handle_message(&json!({
            "jsonrpc": "2.0", "id": 3, "method": "tools/call",
            "params": {"name": "sci_propose", "arguments": {}},
        }))
        .await
        .unwrap();
        assert_eq!(reply["result"]["isError"], true);
        assert!(reply["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("proposals"));
    }

    /// sci_node に node が無いときは isError で返す。
    #[tokio::test]
    async fn sci_node_requires_node() {
        let reply = handle_message(&json!({
            "jsonrpc": "2.0", "id": 4, "method": "tools/call",
            "params": {"name": "sci_node", "arguments": {}},
        }))
        .await
        .unwrap();
        assert_eq!(reply["result"]["isError"], true);
        assert!(reply["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("node"));
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
