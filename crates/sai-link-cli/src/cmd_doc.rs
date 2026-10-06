//! `doc` グループ（新しい木）— 文書（waggle トークン）の読み書き。
//!
//! `doc write` / `doc read` は `agent_api` の関数を呼び、契約の形に組み直す。
//! `doc search` は `phone search` の出力をそのまま使う。

use crate::cli::{DocCmd, PhoneCmd};
use crate::cmd_common::{agent_result, print_json, print_phone, CmdError};
use sai_link_engine::agent_api;

/// `sai-moon-link doc <sub>` のハンドラ。
pub(crate) fn run(cmd: DocCmd) -> Result<(), CmdError> {
    match cmd {
        DocCmd::Write {
            filename,
            description,
            run_id,
            json,
        } => {
            let content = read_stdin_text()?;
            let v = agent_result(agent_api::write(
                &filename,
                &content,
                description.as_deref(),
                run_id,
            ))?;
            if json {
                print_json(&v)
            } else {
                let path = v.get("path").and_then(|x| x.as_str()).unwrap_or("");
                match v.get("token").and_then(|x| x.as_str()) {
                    Some(token) => {
                        println!("保存しました: {}（トークン {}）", path, token)
                    }
                    None => println!("保存しました: {}", path),
                }
                if let Some(r) = v.get("ref").and_then(|x| x.as_str()) {
                    println!("参照: {}", r);
                }
                Ok(())
            }
        }
        DocCmd::Read { token, json } => {
            let v = agent_result(agent_api::sai_read(&token))?;
            if json {
                let item = serde_json::json!({
                    "token": v.get("token"),
                    "label": v.get("file_name"),
                    "block_name": v.get("block_name"),
                    "description": v.get("description"),
                    "text": v.get("content"),
                });
                print_json(&item)
            } else {
                println!(
                    "{}",
                    v.get("content").and_then(|x| x.as_str()).unwrap_or("")
                );
                Ok(())
            }
        }
        DocCmd::List { run, all, json } => doc_list(run, all, json),
        DocCmd::Search {
            word,
            category,
            since,
            until,
            block,
            from_id,
            to_id,
            limit,
            json: _,
        } => print_phone(PhoneCmd::Search {
            word,
            category,
            since,
            until,
            block,
            from_id,
            to_id,
            limit,
        }),
    }
}

/// `doc list`: waggle 文書の一覧。
///
/// - `--run N` のときはその実行の文書（`phone waggle <id>` と同じ材料）。
/// - `--all` のときは実行中（running / review）も含める。
fn doc_list(run: Option<i64>, all: bool, json: bool) -> Result<(), CmdError> {
    use sai_link_view::phone_view;
    let store =
        crate::cli::open_db().map_err(|e| CmdError::other(format!("DB を開けません: {}", e)))?;
    let choices = match run {
        Some(run_id) => phone_view::waggle_choices(&store, run_id),
        None => phone_view::all_docs(&store, all),
    };
    if json {
        let items: Vec<serde_json::Value> = choices
            .iter()
            .map(|c| {
                serde_json::json!({
                    "token": c.token,
                    "label": c.file_name,
                    "description": c.description,
                    "status": c.status,
                    "ref": phone_view::doc_ref_line(c),
                    "run_id": c.run_id,
                    "block": c.block_name,
                })
            })
            .collect();
        print_json(&serde_json::json!({ "count": items.len(), "items": items }))
    } else {
        for c in &choices {
            println!(
                "waggle:{}: {} (run {} / {} / {})",
                c.token, c.description, c.run_id, c.block_name, c.file_name
            );
        }
        Ok(())
    }
}

/// 標準入力を本文として読む。
fn read_stdin_text() -> Result<String, CmdError> {
    use std::io::Read;
    let mut text = String::new();
    std::io::stdin()
        .read_to_string(&mut text)
        .map_err(|e| CmdError::input(format!("標準入力を読めません: {}", e)))?;
    Ok(text)
}
