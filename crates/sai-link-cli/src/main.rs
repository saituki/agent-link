//! `sai-engine` — ブロック定義ファイルに基づく AI コーディングエージェント実行エンジン。
//!
//! ブロック定義ファイル（Markdown + YAML frontmatter）をパースし、
//! pi エージェントを herdr pane 上で実行するパイプラインを提供します。
//!
//! # サブコマンド
//!
//! - `init` — プロジェクトを初期化する（`PROJECTROOT/.sai/` 作成 + sai-control レジストリ登録）
//! - `block run <定義ファイル> [ユーザー入力]` — ブロックを実行する
//! - `block list` — 実行履歴を表示する
//! - `block show <id>` — 実行詳細を表示する
//!
//! # 使用例
//!
//! ```bash
//! # プロジェクトを初期化（開始時に 1 度のみ）
//! cargo run -- init
//!
//! # プロジェクト名を指定して初期化
//! cargo run -- init --name myproject
//!
//! # ブロックを実行
//! cargo run -- block run my-block.md
//!
//! # ユーザー入力付きで実行
//! cargo run -- block run my-block.md "AI搭載のタスク管理ツール"
//!
//! # 実行履歴を表示
//! cargo run -- block list --limit 10
//!
//! # 詳細表示
//! cargo run -- block show 3
//! ```

mod cli;
mod kanban_run;
mod phone;
mod phone_def;
mod search;
mod usage;

use clap::Parser;
use cli::{
    AgentCmd, BlockCmd, BlockDefCmd, Cli, Cmd, DaemonCmd, McpCmd, PhoneCmd, TaskCmd, UsageCmd,
    WaggleCmd,
};

fn main() {
    let cli = Cli::parse();

    let result = match cli.cmd {
        Cmd::Init { name, update } => cli::handle_init(name, update),
        Cmd::Block { cmd } => match cmd {
            BlockCmd::Run {
                definition_file,
                user_input,
                direction: _direction,
                timeout: _timeout,
                keep_pane: _keep_pane,
                block,
                field,
                round,
                global_round,
                tag,
                think,
                model,
                block_timeout,
                yes,
                input_tokens,
                input_file,
                description,
                kanban: _kanban,
                card,
            } => {
                // 実行方式の統一: 全て `--kanban` 経由で実行する（`--interactive` 廃止）。
                // 定義ファイル直指定の旧経路は kanban 誘導メッセージ付きで終了する。
                if block.as_deref() == Some("user-block") {
                    // user-block: pane/daemon を通さず、直接トークン化・記録する
                    cli::handle_user_block_non_interactive(
                        &input_tokens,
                        input_file.as_ref(),
                        description.as_deref(),
                        round,
                        global_round,
                        tag,
                    )
                } else if block.is_some() || card.is_some() || definition_file.is_none() {
                    // kanban 統一実行（対話選択 or `--block/--field/--yes` 非対話）
                    kanban_run::kanban_block_run(
                        block.as_deref(),
                        &field,
                        round,
                        global_round,
                        tag,
                        think,
                        model,
                        block_timeout,
                        yes,
                        card.as_deref(),
                    )
                } else {
                    let def = definition_file
                        .as_ref()
                        .map(|p| p.display().to_string())
                        .unwrap_or_default();
                    eprintln!(
                        "エラー: 定義ファイル直指定の実行は廃止されました。カードを作成して `--kanban` で実行してください: {}",
                        def
                    );
                    eprintln!("  ヒント: TUI の `Shift+C` でカード作成 → `sai-link block run --kanban --block <名>`");
                    let _ = user_input;
                    std::process::exit(1);
                }
            }
            BlockCmd::List { limit, json } => cli::handle_block_list(limit, json),
            BlockCmd::Show { run_id } => cli::handle_block_show(run_id),
            BlockCmd::Rm { exec_id, yes } => cli::handle_block_rm(exec_id, yes),
            BlockCmd::Preview {
                definition_file,
                user_input,
                direction,
                timeout,
                round,
                global_round,
                tag,
                think,
                model,
                block_timeout,
            } => cli::handle_block_preview(
                &definition_file,
                user_input.as_deref(),
                &direction,
                timeout,
                round,
                global_round,
                tag,
                think,
                model,
                block_timeout,
            ),
        },
        Cmd::BlockDef { cmd } => match cmd {
            BlockDefCmd::List { parent, json } => {
                cli::handle_block_def_list(parent.as_deref(), json)
            }
            BlockDefCmd::Show { name, json } => cli::handle_block_def_show(&name, json),
        },
        Cmd::Task { cmd } => match cmd {
            TaskCmd::List {
                parent_run_id,
                status,
                json,
            } => cli::handle_task_list(parent_run_id, status.as_deref(), json),
            TaskCmd::Show { task_id } => cli::handle_task_show(task_id),
            TaskCmd::Capture { task_id, lines } => cli::handle_task_capture(task_id, lines),
            TaskCmd::Manage {
                action,
                task_id,
                message,
            } => cli::handle_task_manage(&action, task_id, message.as_deref()),
            TaskCmd::Poll { all } => cli::handle_task_poll(all),
        },
        Cmd::Daemon { cmd } => match cmd {
            DaemonCmd::Start => cli::handle_daemon_start(),
            DaemonCmd::Stop { force } => cli::handle_daemon_stop(force),
            DaemonCmd::Status => cli::handle_daemon_status(),
            DaemonCmd::Reload => cli::handle_daemon_reload(),
            DaemonCmd::Recover { apply } => cli::handle_daemon_recover(apply),
            DaemonCmd::Run => cli::handle_daemon_run(),
        },
        Cmd::Search {
            global_round,
            round,
            id,
            block,
            date,
            status,
            r#type,
            word,
            project,
            json,
        } => search::run_search_command(search::SearchArgs {
            global_round,
            round,
            id,
            block,
            date,
            status,
            r#type,
            word,
            project,
            json,
        }),
        Cmd::Usage { cmd } => match cmd {
            UsageCmd::Backfill { dry_run } => {
                usage::run_usage_backfill(usage::BackfillArgs { dry_run })
            }
        },
        Cmd::Waggle { cmd } => match cmd {
            WaggleCmd::Resolve { token } => cli::handle_waggle_resolve(&token),
        },
        Cmd::Mcp { cmd } => match cmd {
            McpCmd::Serve => {
                // stdio で MCP サーバを起動する（Claude Code から呼ばれる）
                sai_link_engine::mcp_server::serve_stdio();
                Ok(())
            }
        },
        Cmd::Agent { cmd } => {
            run_agent_command(cmd);
            Ok(())
        }
        Cmd::Phone { cmd } => match cmd {
            PhoneCmd::Export { run, dry_run } => cli::handle_phone_export(run, dry_run),
            // tree / run / md / waggle / card（実装計画 12）は phone ハンドラが
            // 成功時に JSON 1 個を出力し、失敗時に stderr + 終了コードで終了する
            other => phone::handle(other),
        },
        Cmd::CcStatusline => cli::handle_cc_statusline(),
        Cmd::CcPromptHook => cli::handle_cc_prompt_hook(),
    };

    if let Err(e) = result {
        eprintln!("致命的エラー: {}", e);
        std::process::exit(1);
    }
}

/// `sai-link agent <...>` を実行し、結果 JSON を stdout に 1 行で出力する。
///
/// 成功時は `agent_api` の戻り値をそのまま、失敗時は `{"error": "..."}` を出力し、
/// 終了コード 1 で終了する（pi 拡張は stdout の JSON だけを読む）。
fn run_agent_command(cmd: AgentCmd) {
    use sai_link_engine::agent_api;

    let result: Result<serde_json::Value, String> = match cmd {
        AgentCmd::Write {
            filename,
            content_file,
            description,
            run_id,
        } => read_content(&content_file)
            .and_then(|content| agent_api::write(&filename, &content, description.as_deref(), run_id)),

        AgentCmd::Finalize { run_id, report } => {
            agent_api::finalize(run_id, report.as_deref())
        }

        AgentCmd::TaskQuestion {
            qid,
            text,
            run_id,
        } => agent_api::task_question(run_id, &qid, &text),

        AgentCmd::TaskCreate {
            tasks_json,
            round,
            supersedes_batch_id,
            run_id,
        } => read_content(&tasks_json)
            .and_then(|text| {
                serde_json::from_str::<serde_json::Value>(&text)
                    .map_err(|e| format!("tasks_json の JSON パースに失敗しました: {}", e))
            })
            .and_then(|tasks| {
                agent_api::task_create(&tasks, round, supersedes_batch_id.as_deref(), run_id)
            }),

        AgentCmd::TaskList { status, sort, run_id } => {
            agent_api::task_list(status.as_deref(), sort.as_deref(), run_id)
        }

        AgentCmd::TaskExecute { batch_id, task_id, run_id } => {
            agent_api::task_execute(batch_id.as_deref(), task_id, run_id)
        }

        AgentCmd::TaskOutput { task_id, timeout_ms } => agent_api::task_output(task_id, timeout_ms),

        AgentCmd::TaskReport {
            report_file,
            run_id,
        } => read_content(&report_file)
            .and_then(|report| agent_api::task_report(&report, run_id)),

        AgentCmd::TaskManage { action, task_id, message, run_id } => {
            agent_api::task_manage(&action, task_id, message.as_deref(), run_id)
        }

        AgentCmd::TaskStatus { status, task_id, run_id } => {
            agent_api::task_status(status.as_deref(), task_id, run_id)
        }

        AgentCmd::TaskCapture { task_id, lines } => agent_api::task_capture(task_id, lines),

        AgentCmd::BlockDiscover { parent } => agent_api::block_discover(parent.as_deref()),

        AgentCmd::BlockInspect { name } => agent_api::block_inspect(&name),

        AgentCmd::SaiRead { target, run_id, all } => {
            if target == "list" {
                // U03: --all で実行中も含める（TUI w キー用）。既定は完了済みのみ。
                if all {
                    agent_api::sai_read_list_including_running(run_id)
                } else {
                    agent_api::sai_read_list(run_id)
                }
            } else {
                agent_api::sai_read(&target)
            }
        }
    };

    match result {
        Ok(value) => {
            println!("{}", serde_json::to_string(&value).unwrap_or_default());
        }
        Err(message) => {
            let body = serde_json::json!({ "error": message });
            println!("{}", serde_json::to_string(&body).unwrap_or_default());
            eprintln!("{}", message);
            std::process::exit(1);
        }
    }
}

/// ファイルまたは標準入力（`-`）から本文を読む。
///
/// 長いプロンプトや JSON をコマンドライン引数に載せると
/// シェルの引数長制限・エスケープ事故が起きるため、ファイル / 標準入力で受ける。
fn read_content(spec: &str) -> Result<String, String> {
    use std::io::Read;

    if spec == "-" {
        let mut buf = String::new();
        std::io::stdin()
            .read_to_string(&mut buf)
            .map_err(|e| format!("標準入力の読み取りに失敗しました: {}", e))?;
        return Ok(buf);
    }
    std::fs::read_to_string(spec)
        .map_err(|e| format!("ファイル '{}' の読み取りに失敗しました: {}", spec, e))
}
