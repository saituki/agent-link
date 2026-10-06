//! `sai-moon-paper mcp` — 標準入出力の MCP サーバー（段階 7）。
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

/// `sci_propose` の入力の型（`sai-sci-core` の `propose_schema()` の写し）。
///
/// sai-paper は sai-sci に依存しないため、形をここに写してある。
/// この道具はガイド 4 で MCP ごと消す。
const SCI_PROPOSE_SCHEMA: &str = include_str!("../assets/sci_propose_schema.json");

/// `sci_propose` の入力の型を JSON として読む（読めなければ空の形）。
fn propose_schema() -> Value {
    serde_json::from_str(SCI_PROPOSE_SCHEMA).unwrap_or_else(|_| json!({"type": "object"}))
}

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
                    "node": {"type": "string", "description": "研究方針の Node（Q12・O5 など）。記録に残り、前の検索を見られる。find ではその Node の下の F の論文に絞る"},
                    "subtree": {"type": "boolean", "description": "node の子孫の NODE も含める（node と一緒に使う）"},
                    "each": {"type": "integer", "description": "SCOPE の論文ごとに上位 m 件を返す（papers か node と一緒に使う）"},
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
            "description": "索引ごとの断片数と埋め込み数（sai-moon-paper index status と同じ）",
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
            "name": "paper_outline",
            "description": "論文の節の一覧（ID・見出し・文の数・最初の 2 文・要約）。論文を読むときは、まずこれを見て読む節を選ぶ。全文は読まない",
            "inputSchema": {
                "type": "object",
                "properties": {"paper_id": {"type": "integer", "description": "Library の番号（#37 なら 37）"}},
                "required": ["paper_id"],
            },
        },
        {
            "name": "paper_read",
            "description": "論文の節・塊を ID で読む（37.5 = 節、37.5.2 = 塊、37.5.2-4 = 塊の範囲、37 = 全体）。出典は返った塊の ID で [#37.5.2] と書く",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "refs": {"type": "array", "items": {"type": "string"}, "description": "読む ID（1〜20 個）"},
                    "max_chars": {"type": "integer", "description": "返す本文の字数の上限（既定 12000、最大 60000）。超えたら next に続きの ID が入る"},
                },
                "required": ["refs"],
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
            "description": "今の実行の文脈（研究方針の木から範囲を切り出したもの・出せる PROPOSAL の op の説明）を返す（古い別名。ガイド 4 で MCP ごと消す）",
            "inputSchema": {"type": "object", "properties": {
                "run_id": {"type": "integer", "description": "省略時は環境変数 SAI_PARENT_RUN_ID"},
                "scope": {"type": "string", "enum": ["node", "path", "subtree", "siblings", "summary", "blind"]},
            }},
        },
        {
            "name": "sci_status",
            "description": "研究方針の木の全体（Q・O・F・E、Objectivity・Logicality、進み具合、SEED、Next Action、DEBT）。brief を true にすると進み具合・Next Action・DEBT の数・注意だけになる（古い別名。ガイド 4 で MCP ごと消す）",
            "inputSchema": {"type": "object", "properties": {
                "brief": {"type": "boolean", "description": "true のとき進み具合・Next Action・DEBT の数・注意だけ返す（短い文脈で現在地を読む）"},
            }},
        },
        {
            "name": "sci_node",
            "description": "Node 1 つの中身（親までの道・子・ガイド・Logicality の記録・決定の記録・Action の履歴・仕分け待ち・Next Action・検索の記録）（古い別名。ガイド 4 で MCP ごと消す）",
            "inputSchema": {"type": "object", "properties": {"node": {"type": "string", "description": "Q12・O5・F3・E2 の形"}}, "required": ["node"]},
        },
        {
            "name": "sci_propose",
            "description": "研究方針の木への変更を PROPOSAL として出す。O・F・E・Logicality はすぐ入り、Q を足す・写す・✕ は利用者が仕分けてから入る。★ などは出せない。1 件でも違反があれば何も入らない（古い別名。ガイド 4 で MCP ごと消す）",
            "inputSchema": propose_schema(),
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

/// 道具を 1 回呼ぶ。戻り値は CLI の `--json` と同じ JSON。
pub async fn call_tool(name: &str, args: &Value) -> Result<Value> {
    match name {
        "find" => {
            // 引数の検査は DB を開く前に行う（DB が開けないときも同じエラーの文になる）。
            let query = str_arg(args, "query").ok_or_else(|| anyhow!("query が必要です"))?;
            // 研究方針の Node があれば sai-sci が範囲を決めて記録する（古い別名。ガイド 4 で消す）。
            if let Some(node) = str_arg(args, "node") {
                let mut child = vec!["find".to_string(), query, "--node".to_string(), node];
                if args.get("subtree").and_then(Value::as_bool).unwrap_or(false) {
                    child.push("--subtree".to_string());
                }
                if let Some(each) = int_arg(args, "each") {
                    child.push("--each".to_string());
                    child.push(each.to_string());
                }
                if let Some(widen) = str_arg(args, "widen") {
                    child.push("--widen".to_string());
                    child.push(widen);
                }
                if let Some(mode) = str_arg(args, "mode") {
                    child.push("--mode".to_string());
                    child.push(mode);
                }
                if let Some(top_k) = int_arg(args, "top_k") {
                    child.push("--top-k".to_string());
                    child.push(top_k.to_string());
                }
                if let Some(per_paper) = int_arg(args, "per_paper") {
                    child.push("--per-paper".to_string());
                    child.push(per_paper.to_string());
                }
                if let Some(scope) = str_arg(args, "scope") {
                    child.push("--scope".to_string());
                    child.push(scope);
                }
                if let Some(index) = str_arg(args, "index") {
                    child.push("--index".to_string());
                    child.push(index);
                }
                if let Some(paper_id) = int_arg(args, "paper_id") {
                    child.push("--paper".to_string());
                    child.push(paper_id.to_string());
                }
                let papers = int_list_arg(args, "papers");
                if !papers.is_empty() {
                    child.push("--papers".to_string());
                    child.push(papers.iter().map(i64::to_string).collect::<Vec<_>>().join(","));
                }
                if args.get("evidence").and_then(Value::as_bool).unwrap_or(false) {
                    child.push("--evidence".to_string());
                }
                child.push("--json".to_string());
                let value = crate::sci_bridge::capture_json(&child, None)
                    .map_err(|message| anyhow!("{message}"))?;
                // 道具の形は今までどおりヒットの配列（`{"log","hits"}` の `hits`）。
                return Ok(value.get("hits").cloned().unwrap_or(value));
            }
            let (db, config) = open()?;
            let conn = db.connection();
            let papers = int_list_arg(args, "papers");
            let each = int_arg(args, "each");
            // `--each` には範囲の指定が要る（CLI の find と同じ検査）。
            if each.is_some() && papers.is_empty() {
                return Err(anyhow!("each は papers と一緒に使ってください"));
            }
            let (paper_ids, scope_note) = sai_paper_core::retrieval::resolve_paper_scope(
                conn,
                int_arg(args, "paper_id"),
                &papers,
            )?;
            // 内部検索の履歴に残す範囲のコピー（U01。`request` が動くため先に写す）。
            let log_mode = str_arg(args, "mode");
            let log_scope = str_arg(args, "scope");
            let log_papers: Vec<i64> = paper_ids.clone().unwrap_or_default();
            let log_each = int_arg(args, "each");
            let log_widen = str_arg(args, "widen").unwrap_or_default();
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
            // 内部検索の履歴（U01。Node の有無に関わらず毎回。失敗しても検索は続ける）。
            {
                let plain_hits: Vec<sai_paper_core::Hit> = match &outcome {
                    sai_paper_core::evidence::SearchOutcome::Plain(hits) => hits.clone(),
                    sai_paper_core::evidence::SearchOutcome::Evidence(result) => {
                        result.hits.iter().map(|item| item.hit.clone()).collect()
                    }
                };
                let passages = sai_paper_core::sections::passages_for_hits(conn, &plain_hits);
                if let Err(err) = sai_paper_core::find_log::record(
                    conn,
                    &sai_paper_core::find_log::FindLogInput {
                        query: &query,
                        mode: log_mode.as_deref(),
                        top_k: int_arg(args, "top_k"),
                        scope: log_scope.as_deref(),
                        range_text: &scope_note,
                        papers: &log_papers,
                        node_id: None,
                        subtree: false,
                        each: log_each,
                        widen_reason: &log_widen,
                        origin: "mcp",
                        run_id: crate::sci_bridge::parent_run_id(),
                    },
                    &plain_hits,
                    &passages,
                ) {
                    eprintln!("検索履歴を残せませんでした: {err:#}");
                }
            }
            match outcome {
                sai_paper_core::evidence::SearchOutcome::Plain(hits) => {
                    Ok(sai_paper_core::retrieval::hits_to_json_with(
                        &hits,
                        &sai_paper_core::sections::passages_for_hits(conn, &hits),
                    ))
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
            Ok(sai_paper_core::retrieval::hits_to_json_with(
                &hits,
                &sai_paper_core::sections::passages_for_hits(db.connection(), &hits),
            ))
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
        "paper_outline" => {
            let paper_id =
                int_arg(args, "paper_id").ok_or_else(|| anyhow!("paper_id が必要です"))?;
            let (db, _) = open()?;
            let data_dir = Paths::discover()?.data_dir().to_path_buf();
            let value = crate::content::outline_json(db.connection(), &data_dir, paper_id)?;
            crate::activity::record_agent_read("mcp", "outline", Some(paper_id), None);
            Ok(value)
        }
        "paper_read" => {
            let refs: Vec<String> = args
                .get("refs")
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
                .unwrap_or_default();
            if refs.is_empty() || refs.len() > 20 {
                return Err(anyhow!("refs は 1〜20 個にしてください"));
            }
            let (db, _) = open()?;
            let data_dir = Paths::discover()?.data_dir().to_path_buf();
            let max_chars = int_arg(args, "max_chars").map(|n| n.max(1) as usize);
            let value = crate::content::read_json(db.connection(), &data_dir, &refs, max_chars)?;
            for r in &refs {
                if let Some(id) = crate::content::paper_id_of_ref(r) {
                    crate::activity::record_agent_read("mcp", "read", Some(id), None);
                }
            }
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
            // `sai-sci context` を子として呼ぶ（古い別名。ガイド 4 で MCP ごと消す）。
            let run = int_arg(args, "run_id").or_else(crate::sci_bridge::parent_run_id);
            let Some(run_id) = run else {
                return Err(anyhow!("run_id が必要です"));
            };
            let mut child = vec!["context".to_string(), "--run".to_string(), run_id.to_string()];
            if let Some(scope) = str_arg(args, "scope") {
                child.push("--scope".to_string());
                child.push(scope);
            }
            let text = crate::sci_bridge::capture(&child, None)
                .map_err(|message| anyhow!("{message}"))?;
            Ok(json!({ "text": text }))
        }
        "sci_status" => {
            let mut child = vec!["status".to_string()];
            if args.get("brief").and_then(Value::as_bool).unwrap_or(false) {
                child.push("--brief".to_string());
            }
            child.push("--json".to_string());
            crate::sci_bridge::capture_json(&child, None).map_err(|message| anyhow!("{message}"))
        }
        "sci_node" => {
            let node = str_arg(args, "node").ok_or_else(|| anyhow!("node が必要です"))?;
            let child = vec!["node".to_string(), node, "--json".to_string()];
            crate::sci_bridge::capture_json(&child, None).map_err(|message| anyhow!("{message}"))
        }
        "sci_propose" => {
            let proposals = args
                .get("proposals")
                .and_then(Value::as_array)
                .ok_or_else(|| anyhow!("proposals が必要です"))?;
            let mut child = vec!["propose".to_string()];
            if let Some(run_id) = int_arg(args, "run_id") {
                child.push("--run".to_string());
                child.push(run_id.to_string());
            }
            let input = json!({ "proposals": proposals.clone() }).to_string();
            crate::sci_bridge::capture_json(&child, Some(&input))
                .map_err(|message| anyhow!("{message}"))
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

    /// tools/list に 11 個の道具が並び、どれも入力の形を持つ。
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
                "paper_outline",
                "paper_read",
                "search_sets",
                "sci_context",
                "sci_status",
                "sci_node",
                "sci_propose"
            ]
        );
    }

    /// find の入力に範囲を絞る引数がある（papers / node / subtree / each）。
    #[test]
    fn find_tool_exposes_scope_arguments() {
        let listed = tools();
        let find = listed
            .as_array()
            .unwrap()
            .iter()
            .find(|tool| tool["name"] == "find")
            .unwrap();
        for name in ["papers", "node", "subtree", "each"] {
            assert!(
                find["inputSchema"]["properties"][name].is_object(),
                "find に {name} が無い"
            );
        }
    }

    /// sci_propose の items は Auto・Human の op だけ（star などの利用者専用は無い）。
    #[test]
    fn sci_propose_schema_has_no_user_only_ops() {
        let listed = tools();
        let tool = listed.as_array().unwrap().iter().find(|t| t["name"] == "sci_propose").unwrap();
        let variants = tool["inputSchema"]["properties"]["proposals"]["items"]["oneOf"].as_array().unwrap();
        let ops: Vec<&str> = variants.iter().map(|v| v["properties"]["op"]["const"].as_str().unwrap()).collect();
        for op in ["add_q", "add_o", "add_f", "add_e", "record_logic", "copy", "remove"] {
            assert!(ops.contains(&op), "{op} が無い");
        }
        for op in ["star", "unstar", "set_kind", "edit", "set_target"] {
            assert!(!ops.contains(&op), "{op} が出ている");
        }
        let find = listed.as_array().unwrap().iter().find(|t| t["name"] == "find").unwrap();
        assert!(find["inputSchema"]["properties"]["question"].is_null());
        let node = listed.as_array().unwrap().iter().find(|t| t["name"] == "sci_node").unwrap();
        assert!(node["inputSchema"]["properties"]["node"]["description"].as_str().unwrap().contains("Q12"));
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

    /// paper_read に refs が 0 個のとき、paper_outline に paper_id が無いときは isError で返す。
    #[tokio::test]
    async fn paper_tools_validate_arguments() {
        for (name, args, word) in [
            ("paper_read", json!({"refs": []}), "refs"),
            ("paper_outline", json!({}), "paper_id"),
        ] {
            let reply = handle_message(&json!({
                "jsonrpc": "2.0", "id": 5, "method": "tools/call",
                "params": {"name": name, "arguments": args},
            }))
            .await
            .unwrap();
            assert_eq!(reply["result"]["isError"], true, "{name}");
            assert!(reply["result"]["content"][0]["text"].as_str().unwrap().contains(word));
        }
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
