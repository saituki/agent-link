//! `agent-link` — ブロック定義ファイルに基づく AI コーディングエージェント実行エンジン。
//!
//! ブロック定義ファイル（Markdown + YAML frontmatter）をパースし、
//! cc を擬似 pane（tmux）上で実行するパイプラインを提供します。
//!
//! # サブコマンド
//!
//! 用途ごと: `init`・`card`・`run`・`doc`・`task`・`def`・`project`・
//! `session`・`workflow`・`schema`・`daemon`・`usage`。
//! 内部用（隠したまま）: `waggle resolve`・`cc-prompt-hook`・`cc-state-hook`・`cc-permission-hook`・`cc-statusline`・`remote-gate`・`daemon _run`。
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
mod cmd_ext;
mod cmd_project;
mod cmd_remote;
mod cmd_run;
mod cmd_schema;
mod cmd_task;
mod def_files;
mod kanban_run;
mod search;
mod usage;
mod views;

use clap::error::ErrorKind;
use clap::Parser;
use cli::{Cli, Cmd, DaemonCmd, ExtCmd, UsageCmd, WaggleCmd};

fn main() {
    // clap の使い方の誤りも、最後の行を日本語 1 行にする（CLI 共通の約束 2）
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(e) => handle_clap_error(e),
    };

    // プロジェクトを要る命令は、見つからなければ終了コード 2 で止める（`.agent-link/` を勝手に作らない）。
    // 要らないのは `init`・`schema`・`remote`・`remote-gate`・置き場だけを使う `ext list`/`install`/`uninstall`/`config`。
    // cc のフック（`cc-*`）は、失敗すると cc の操作を止めるので、今のまま検査せずに動かす。
    let needs_project = !matches!(
        &cli.cmd,
        Cmd::Init { .. }
            | Cmd::Schema { .. }
            | Cmd::Remote { .. }
            | Cmd::Ext {
                cmd: ExtCmd::List { .. }
                    | ExtCmd::Install { .. }
                    | ExtCmd::Uninstall { .. }
                    | ExtCmd::Config { .. },
            }
            | Cmd::RemoteGate
            | Cmd::CcPromptHook
            | Cmd::CcStateHook { .. }
            | Cmd::CcPermissionHook
            | Cmd::CcStatusline
    );
    if needs_project {
        if let Err(msg) = agent_link_engine::project::project_root_checked() {
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
        Cmd::Remote { cmd } => run_new(cmd_remote::run(cmd)),
        Cmd::RemoteGate => std::process::exit(agent_link_engine::remote::run_gate()),
        Cmd::Ext { cmd } => run_new(cmd_ext::run(cmd)),
        Cmd::Project { cmd } => run_new(cmd_project::run(cmd)),
        Cmd::Session { cmd } => run_new(cmd_project::run_session(cmd)),
        Cmd::Workflow { json: _, subtasks } => run_new(cmd_run::run_workflow(subtasks)),
        Cmd::Schema { json } => run_new(cmd_schema::run(json)),
        Cmd::Task { cmd } => run_new(cmd_task::run(cmd)),
        Cmd::Daemon { cmd } => match cmd {
            DaemonCmd::Start => run_new(cli::handle_daemon_start()),
            DaemonCmd::Stop { force } => run_new(cli::handle_daemon_stop(force)),
            DaemonCmd::Status => run_new(cli::handle_daemon_status()),
            DaemonCmd::Reload => run_new(cli::handle_daemon_reload()),
            DaemonCmd::Recover { apply } => run_new(cli::handle_daemon_recover(apply)),
            DaemonCmd::Run => run_new(cli::handle_daemon_run()),
        },
        Cmd::Usage { cmd } => match cmd {
            UsageCmd::Backfill { dry_run } => {
                run_new(usage::run_usage_backfill(usage::BackfillArgs { dry_run }))
            }
        },
        Cmd::Waggle { cmd } => match cmd {
            WaggleCmd::Resolve { token } => run_new(cli::handle_waggle_resolve(&token)),
        },
        Cmd::CcPromptHook => cli::handle_cc_prompt_hook(),
        Cmd::CcStateHook { state } => cli::handle_cc_state_hook(&state),
        Cmd::CcPermissionHook => cli::handle_cc_permission_hook(),
        Cmd::CcStatusline => cli::handle_cc_statusline(),
    };

    if let Err(e) = result {
        eprintln!("Fatal error: {}", e);
        std::process::exit(1);
    }
}

/// ハンドラを走らせ、失敗なら契約の終了コードで終わる（`Fatal error:` は付けない）。
fn run_new<E: Into<cmd_common::CmdError>>(
    result: Result<(), E>,
) -> Result<(), Box<dyn std::error::Error>> {
    if let Err(e) = result {
        cmd_common::exit(e.into());
    }
    Ok(())
}

/// clap のパース誤りを CLI 共通の約束の形で扱う。
///
/// - `--help`・`--version` などの表示は、今のとおり出して終了コード 0。
/// - それ以外は clap の複数行の文を出さず、標準エラーに
///   `Invalid arguments: <clap の文の 1 行目から先頭の "error: " を除いた物>` の 1 行だけを書いて終了コード 2。
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
            // clap の文の最初の段落（空行まで）を 1 行にまとめる
            let first = message
                .trim_start()
                .split("\n\n")
                .next()
                .unwrap_or("")
                .lines()
                .map(str::trim)
                .collect::<Vec<_>>()
                .join(" ");
            let first = first.strip_prefix("error: ").unwrap_or(&first);
            eprintln!("Invalid arguments: {}", first);
            std::process::exit(cmd_common::EXIT_INPUT);
        }
    }
}

