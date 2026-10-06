//! `sai-engine` — ブロック定義ファイルに基づく AI コーディングエージェント実行エンジン。
//!
//! ブロック定義ファイル（Markdown + YAML frontmatter）をパースし、
//! pi エージェントを herdr pane 上で実行するパイプラインを提供します。
//!
//! # サブコマンド
//!
//! 新しい木（用途ごと）: `card`・`run`・`doc`・`task`・`def`・`project`・
//! `session`・`workflow`・`schema`。
//! 古い命令（`phone`・`agent`・`block`・`block-def`・トップの `search`・`mcp`）は
//! 隠した別名として同じ動きのまま残す。
//!
//! # 使用例
//!
//! ```bash
//! # プロジェクトを初期化（開始時に 1 度のみ）
//! cargo run -- init
//!
//! # 実行履歴を表示
//! cargo run -- run list --limit 10
//!
//! # 詳細表示
//! cargo run -- run show 3
//! ```

mod cli;
mod cmd_card;
mod cmd_common;
mod cmd_def;
mod cmd_doc;
mod cmd_project;
mod cmd_run;
mod cmd_schema;
mod cmd_task;
mod kanban_run;
mod phone;
mod phone_def;
mod search;
mod usage;

use clap::error::ErrorKind;
use clap::Parser;
use cli::{
    AgentCmd, BlockCmd, BlockDefCmd, Cli, Cmd, DaemonCmd, McpCmd, PhoneCmd, TaskCmd, UsageCmd,
    WaggleCmd,
};

fn main() {
    // clap の使い方の誤りも、最後の行を日本語 1 行にする（CLI 共通の約束 2）
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(e) => handle_clap_error(e),
    };

    // 新しい木はプロジェクトを要る（`init`・`schema` は要らない）。
    // 見つからなければ終了コード 2 で止める（`.sai/` を勝手に作らない）。
    let needs_project = matches!(
        &cli.cmd,
        Cmd::Card { .. }
            | Cmd::Run { .. }
            | Cmd::Doc { .. }
            | Cmd::Task { .. }
            | Cmd::Def { .. }
            | Cmd::Project { .. }
            | Cmd::Session { .. }
            | Cmd::Workflow { .. }
    );
    if needs_project {
        if let Err(msg) = sai_link_engine::project::project_root_checked() {
            eprintln!("{}", msg);
            std::process::exit(cmd_common::EXIT_INPUT);
        }
    }

    let result = match cli.cmd {
        Cmd::Init { name, update } => cli::handle_init(name, update),
        // ── 新しい木 ──
        Cmd::Card { cmd } => run_new(cmd_card::run(cmd)),
        Cmd::Run { cmd } => run_new(cmd_run::run(cmd)),
        Cmd::Doc { cmd } => run_new(cmd_doc::run(cmd)),
        Cmd::Def { cmd } => run_new(cmd_def::run(cmd)),
        Cmd::Project { cmd } => run_new(cmd_project::run(cmd)),
        Cmd::Session { cmd } => run_new(cmd_project::run_session(cmd)),
        Cmd::Workflow { json: _ } => run_new(cmd_run::run_workflow()),
        Cmd::Schema { json } => run_new(cmd_schema::run(json)),
        Cmd::Task { cmd } => match cmd {
            // 旧 `task manage`（隠した別名）は今のまま
            TaskCmd::Manage {
                action,
                task_id,
                message,
            } => cli::handle_task_manage(&action, task_id, message.as_deref()),
            other => run_new(cmd_task::run(other)),
        },
        // ── 古い木（隠した別名）──
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

/// 新しい木のハンドラを走らせ、失敗なら契約の終了コードで終わる。
fn run_new(result: Result<(), cmd_common::CmdError>) -> Result<(), Box<dyn std::error::Error>> {
    if let Err(e) = result {
        cmd_common::exit(e);
    }
    Ok(())
}

/// clap のパース誤りを CLI 共通の約束の形で扱う。
///
/// - `--help`・`--version` などの表示は、今のとおり出して終了コード 0。
/// - それ以外は clap の文を標準エラーに出したあと、最後の行に
///   `引数の誤り: <clap の文の 1 行目から先頭の "error: " を除いた物>` を書いて終了コード 2。
fn handle_clap_error(e: clap::Error) -> ! {
    match e.kind() {
        ErrorKind::DisplayHelp
        | ErrorKind::DisplayVersion
        | ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand => {
            let _ = e.print();
            std::process::exit(0);
        }
        _ => {
            let message = e.to_string();
            // clap の文（「error: …」）を標準エラーへ
            let _ = e.print();
            let first = message
                .lines()
                .find(|l| !l.trim().is_empty())
                .unwrap_or("")
                .trim();
            let first = first.strip_prefix("error: ").unwrap_or(first);
            eprintln!("引数の誤り: {}", first);
            std::process::exit(cmd_common::EXIT_INPUT);
        }
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
