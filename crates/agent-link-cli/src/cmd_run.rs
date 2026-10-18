//! `run` グループ（新しい木）— 実行（run）の参照・操作。
//!
//! 処理の本体は `views` / `cli` の実装をそのまま呼ぶ（写さない）。
//! `run list --json` だけは新しい形（`{"runs":[…]}`）を組み立てる。

use std::collections::HashMap;

use crate::cli::{ViewCmd, RunCmd};
use crate::cmd_common::{agent_result, other_error, print_json, print_view, CmdError};

/// `agent-link run <sub>` のハンドラ。
pub(crate) fn run(cmd: RunCmd) -> Result<(), CmdError> {
    match cmd {
        RunCmd::List { limit, json } => {
            // `--limit 0` は全部（SQLite は LIMIT -1 で無制限）
            let limit = if limit <= 0 { -1 } else { limit };
            if json {
                run_list_json(limit)
            } else {
                crate::cli::handle_block_list(limit, false).map_err(other_error)
            }
        }
        RunCmd::Show { run_id, json } => {
            crate::cli::handle_block_show(run_id, json).map_err(other_error)
        }
        RunCmd::Tree { json: _ } => print_view(ViewCmd::Tree),
        RunCmd::Log { run_id, json: _ } => print_view(ViewCmd::Run {
            run_id,
            view: "log".to_string(),
        }),
        RunCmd::Stats { run_id, json: _ } => print_view(ViewCmd::Run {
            run_id,
            view: "stats".to_string(),
        }),
        RunCmd::History {
            run_id,
            call: None,
            json: _,
        } => print_view(ViewCmd::Run {
            run_id,
            view: "history".to_string(),
        }),
        RunCmd::History {
            run_id,
            call: Some(call_id),
            json,
        } => history_call(run_id, &call_id, json),
        RunCmd::Md {
            run_id,
            kind,
            write,
            json: _,
        } => print_view(ViewCmd::Md {
            run_id,
            kind,
            write,
        }),
        // run_id を省略すると `AGENT_LINK_PARENT_RUN_ID`（自分の run）を使う
        RunCmd::Finalize {
            run_id,
            report,
            json: _,
        } => {
            let v = agent_result(agent_link_engine::agent_api::finalize(
                run_id,
                report.as_deref(),
            ))?;
            print_json(&v)
        }
        RunCmd::Advance { run_id, json: _ } => print_view(ViewCmd::Advance { run_id }),
        // 実行中の run に続きの文を送る（--text が無ければ標準入力の全文）
        RunCmd::Send {
            run_id,
            text,
            json: _,
        } => {
            let text = match text {
                Some(t) => t,
                None => {
                    use std::io::Read;
                    let mut buf = String::new();
                    std::io::stdin().read_to_string(&mut buf).map_err(|e| {
                        CmdError::other(format!("Failed to read standard input: {}.", e))
                    })?;
                    buf
                }
            };
            if text.trim().is_empty() {
                return Err(CmdError::input("The text to send is empty."));
            }
            let v = agent_result(agent_link_engine::agent_api::run_send(run_id, &text))?;
            print_json(&v)
        }
        // 実行中の run を中断する（人間の esc と同じ）
        RunCmd::Interrupt { run_id, json: _ } => {
            let v = agent_result(agent_link_engine::agent_api::run_interrupt(run_id))?;
            print_json(&v)
        }
        // 子でない run も含めて待つ（1〜50 件。task wait と同じ上限）
        RunCmd::Wait {
            run_ids,
            timeout_ms,
            json: _,
        } => {
            if run_ids.is_empty() {
                return Err(CmdError::input(
                    "run wait needs at least one run ID.",
                ));
            }
            if run_ids.len() > 50 {
                return Err(CmdError::input("run wait accepts at most 50 run IDs."));
            }
            let cancel = std::sync::atomic::AtomicBool::new(false);
            let v = agent_result(agent_link_engine::agent_api::run_wait(
                &run_ids,
                timeout_ms,
                &cancel,
            ))?;
            print_json(&v)
        }
        RunCmd::Rm { run_id, yes, json: _ } => {
            // 確認の入力を待たない。`--yes` が無ければ終了コード 2。
            if !yes {
                return Err(CmdError::input(
                    "Pass --yes to delete a run. The command does not wait for confirmation.",
                ));
            }
            print_view(ViewCmd::Rm { run_id })
        }
        RunCmd::Export { run, dry_run } => crate::cli::handle_run_export(run, dry_run),
    }
}

/// `run workflow`（トップの `workflow`）: メイン画面の DAG を JSON で返す。
pub(crate) fn run_workflow(subtasks: bool) -> Result<(), CmdError> {
    print_view(ViewCmd::Workflow { subtasks })
}

/// `run list --json`: `{"runs":[…]}` の形。
///
/// DB（`execution_history`・`block_metadata`）と run フォルダの場所から組み立てる。
/// run フォルダが無い run は `dir` を `null` にする。
fn run_list_json(limit: i64) -> Result<(), CmdError> {
    let store =
        crate::cli::open_db().map_err(|e| CmdError::other(format!("Cannot open the DB: {}.", e)))?;
    let execs = store
        .list_recent_executions(limit)
        .map_err(|e| CmdError::other(format!("Failed to load the run history: {}.", e)))?;
    // N+1 を避ける（ブロック一覧を 1 回で取る）
    let blocks: HashMap<i64, agent_link_engine::db::BlockMetadata> = store
        .list_blocks()
        .map_err(|e| CmdError::other(format!("Failed to load the block list: {}.", e)))?
        .into_iter()
        .filter_map(|b| b.id.map(|id| (id, b)))
        .collect();

    let mut runs = Vec::with_capacity(execs.len());
    for exec in &execs {
        let run_id = exec.id.unwrap_or(0);
        // lane は run フォルダの親の名前（todo / running / review / 日付 yymmdd）
        let (lane, dir) = match agent_link_engine::run_dir::find_run_dir(run_id) {
            Some((owner, path)) => (Some(owner), Some(path.display().to_string())),
            None => (None, None),
        };
        let block = blocks.get(&exec.block_id);
        runs.push(serde_json::json!({
            "run_id": run_id,
            "block": block.map(|b| b.name.clone()).unwrap_or_default(),
            "status": exec.status,
            "lane": lane,
            "parent_run_id": exec.parent_run_id,
            "created_at": exec.created_at,
            "started_at": exec.started_at,
            "finished_at": exec.finished_at,
            "description": block.map(|b| b.description.clone()).unwrap_or_default(),
            "dir": dir,
        }));
    }
    print_json(&serde_json::json!({ "runs": runs }))
}

/// `agent-link run history <id> --call <CALL_ID>`: その呼び出しの全文（C11）。
/// `--json` が無ければ、命令（無ければ道具の名前）と結果の本文を出す。
fn history_call(run_id: i64, call_id: &str, json: bool) -> Result<(), CmdError> {
    let v = crate::views::history_call(run_id, call_id)?;
    if json {
        return print_json(&v);
    }
    let head = v["command"].as_str().or(v["name"].as_str()).unwrap_or("");
    println!("{}", head);
    println!("{}", v["result"].as_str().unwrap_or(""));
    Ok(())
}
