//! `task` グループ（新しい木）— サブタスクの操作。
//!
//! 処理の本体は `agent_api` の関数をそのまま呼ぶ。失敗は文で分類して
//! 終了コードにする（`cmd_common::agent_result`）。

use std::sync::atomic::AtomicBool;

use crate::cli::TaskCmd;
use crate::cmd_common::{agent_result, print_json, CmdError};
use agent_link_engine::agent_api;

/// `agent-link task <sub>` のハンドラ。
pub(crate) fn run(cmd: TaskCmd) -> Result<(), CmdError> {
    match cmd {
        TaskCmd::Create {
            round,
            supersedes_batch_id,
            run_id,
            json: _,
        } => {
            let text = read_stdin_text()?;
            let value: serde_json::Value = serde_json::from_str(&text)
                .map_err(|e| CmdError::input(format!("標準入力の JSON の解析に失敗しました: {}", e)))?;
            // 子の配列 `[…]` と `{"tasks":[…]}` の両方を受ける
            let tasks = match value.get("tasks") {
                Some(t) => t.clone(),
                None => value,
            };
            let v = agent_result(agent_api::task_create(
                &tasks,
                round,
                supersedes_batch_id.as_deref(),
                run_id,
            ))?;
            print_json(&v)
        }
        TaskCmd::Execute {
            batch,
            task,
            run_id,
            json: _,
        } => {
            let v = agent_result(agent_api::task_execute(batch.as_deref(), task, run_id))?;
            print_json(&v)
        }
        TaskCmd::Wait {
            task_ids,
            timeout_ms,
            json: _,
        } => {
            if task_ids.is_empty() {
                return Err(CmdError::input(
                    "task wait には 1 件以上のタスク ID を指定してください",
                ));
            }
            if task_ids.len() > 50 {
                return Err(CmdError::input("task wait は 50 件までです"));
            }
            // 2 件以上のときは「1 件返して残りを pending」の形にする
            let with_pending = task_ids.len() > 1;
            let v = agent_result(agent_api::task_output_ids(
                &task_ids,
                timeout_ms,
                &AtomicBool::new(false),
                with_pending,
            ))?;
            print_json(&v)
        }
        TaskCmd::List {
            status,
            sort,
            run_id,
            json: _,
        } => {
            let v = agent_result(agent_api::task_list(status.as_deref(), sort.as_deref(), run_id))?;
            print_json(&v)
        }
        TaskCmd::Show { task_id, json: _ } => {
            let v = agent_result(agent_api::task_status(None, Some(task_id), None))?;
            print_json(&v)
        }
        TaskCmd::Capture {
            task_id,
            lines,
            json: _,
        } => {
            let v = agent_result(agent_api::task_capture(task_id, lines))?;
            print_json(&v)
        }
        TaskCmd::Send {
            task_id,
            message,
            run_id,
            json: _,
        } => {
            let v = agent_result(agent_api::task_manage(
                "send",
                task_id,
                Some(&message),
                run_id,
            ))?;
            print_json(&v)
        }
        TaskCmd::Interrupt {
            task_id,
            run_id,
            json: _,
        } => managed("interrupt", task_id, run_id),
        TaskCmd::Delete {
            task_id,
            run_id,
            json: _,
        } => managed("delete", task_id, run_id),
        TaskCmd::Finalize {
            task_id,
            run_id,
            json: _,
        } => managed("finalize", task_id, run_id),
        TaskCmd::Requeue {
            task_id,
            run_id,
            json: _,
        } => managed("requeue", task_id, run_id),
        TaskCmd::Report { run_id, json: _ } => {
            let text = read_stdin_text()?;
            let v = agent_result(agent_api::task_report(&text, run_id))?;
            print_json(&v)
        }
        TaskCmd::Question {
            qid,
            text,
            run_id,
            json: _,
        } => {
            let v = agent_result(agent_api::task_question(run_id, &qid, &text))?;
            print_json(&v)
        }
    }
}

/// `task interrupt` / `delete` / `finalize` / `requeue` の共通処理。
fn managed(action: &str, task_id: i64, run_id: Option<i64>) -> Result<(), CmdError> {
    let v = agent_result(agent_api::task_manage(action, task_id, None, run_id))?;
    print_json(&v)
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
