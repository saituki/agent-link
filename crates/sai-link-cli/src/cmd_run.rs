//! `run` グループ（新しい木）— 実行（run）の参照・操作。
//!
//! 処理の本体は `phone` / `cli` の実装をそのまま呼ぶ（写さない）。
//! `run list --json` だけは sai-sci 向けの新しい形を組み立てる。

use std::collections::HashMap;

use crate::cli::{PhoneCmd, RunCmd};
use crate::cmd_common::{agent_result, other_error, print_json, print_phone, CmdError};

/// `sai-link run <sub>` のハンドラ。
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
        RunCmd::Show { run_id } => crate::cli::handle_block_show(run_id).map_err(other_error),
        RunCmd::Tree { json: _ } => print_phone(PhoneCmd::Tree),
        RunCmd::Log { run_id, json: _ } => print_phone(PhoneCmd::Run {
            run_id,
            view: "log".to_string(),
        }),
        RunCmd::Stats { run_id, json: _ } => print_phone(PhoneCmd::Run {
            run_id,
            view: "stats".to_string(),
        }),
        RunCmd::History { run_id, json: _ } => print_phone(PhoneCmd::Run {
            run_id,
            view: "history".to_string(),
        }),
        RunCmd::Md {
            run_id,
            kind,
            write,
            json: _,
        } => print_phone(PhoneCmd::Md {
            run_id,
            kind,
            write,
        }),
        // run_id を省略すると `SAI_PARENT_RUN_ID`（自分の run）を使う
        RunCmd::Finalize {
            run_id,
            report,
            json: _,
        } => {
            let v = agent_result(sai_link_engine::agent_api::finalize(
                run_id,
                report.as_deref(),
            ))?;
            print_json(&v)
        }
        RunCmd::Advance { run_id, json: _ } => print_phone(PhoneCmd::Advance { run_id }),
        RunCmd::Rm { run_id, yes } => {
            // 確認の入力を待たない。`--yes` が無ければ終了コード 2。
            if !yes {
                return Err(CmdError::input(
                    "実行を消すには --yes を付けてください（確認の入力を待ちません）",
                ));
            }
            crate::cli::handle_block_rm(run_id, true).map_err(other_error)
        }
        RunCmd::Export { run, dry_run } => {
            crate::cli::handle_phone_export(run, dry_run).map_err(other_error)
        }
    }
}

/// `run workflow`（トップの `workflow`）: メイン画面の DAG を JSON で返す。
pub(crate) fn run_workflow() -> Result<(), CmdError> {
    print_phone(PhoneCmd::Workflow)
}

/// `run list --json`: sai-sci が読む形（`{"runs":[…]}`）。
///
/// DB（`execution_history`・`block_metadata`）と run フォルダの場所から組み立てる。
/// run フォルダが無い run は `dir` を `null` にする。
fn run_list_json(limit: i64) -> Result<(), CmdError> {
    let store =
        crate::cli::open_db().map_err(|e| CmdError::other(format!("DB を開けません: {}", e)))?;
    let execs = store
        .list_recent_executions(limit)
        .map_err(|e| CmdError::other(format!("実行履歴の取得に失敗しました: {}", e)))?;
    // N+1 を避ける（ブロック一覧を 1 回で取る）
    let blocks: HashMap<i64, sai_link_engine::db::BlockMetadata> = store
        .list_blocks()
        .map_err(|e| CmdError::other(format!("ブロック一覧の取得に失敗しました: {}", e)))?
        .into_iter()
        .filter_map(|b| b.id.map(|id| (id, b)))
        .collect();

    let mut runs = Vec::with_capacity(execs.len());
    for exec in &execs {
        let run_id = exec.id.unwrap_or(0);
        // lane は run フォルダの親の名前（todo / running / review / done など）
        let (lane, dir) = match sai_link_engine::run_dir::find_run_dir(run_id) {
            Some((owner, path)) => (Some(owner), Some(path.display().to_string())),
            None => (None, None),
        };
        let block = blocks.get(&exec.block_id);
        runs.push(serde_json::json!({
            "run_id": run_id,
            "block": block.map(|b| b.name.clone()).unwrap_or_default(),
            "tag": exec
                .tag
                .clone()
                .or_else(|| block.and_then(|b| b.tag.clone()))
                .unwrap_or_default(),
            "status": exec.status,
            "lane": lane,
            "parent_run_id": exec.parent_run_id,
            "global_round": exec.global_round,
            "round": exec.round,
            "created_at": exec.created_at,
            "started_at": exec.started_at,
            "finished_at": exec.finished_at,
            "description": block.map(|b| b.description.clone()).unwrap_or_default(),
            "dir": dir,
        }));
    }
    print_json(&serde_json::json!({ "runs": runs }))
}
