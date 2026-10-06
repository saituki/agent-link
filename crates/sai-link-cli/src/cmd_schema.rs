//! `schema --json` — CLI の命令一覧（Web の「コマンド」画面が読む）。
//!
//! clap の定義（[`Cli::command`]）をたどって出す。隠した命令と隠した引数は出さない。
//! `stdin`・`web`・`json` は clap に無い情報なので、小さな表（下の 2 つの関数）で足す。

use clap::{ArgAction, CommandFactory};

use crate::cli::Cli;
use crate::cmd_common::{print_json, CmdError};

/// `sai-link schema` のハンドラ。
pub(crate) fn run(json: bool) -> Result<(), CmdError> {
    let root = Cli::command();
    let bin = root.get_name().to_string();
    let version = root.get_version().unwrap_or("").to_string();

    let mut commands = Vec::new();
    collect(&root, &[], &mut commands);

    if json {
        print_json(&serde_json::json!({
            "bin": bin,
            "version": version,
            "commands": commands,
        }))
    } else {
        for command in &commands {
            let path = command
                .get("path")
                .and_then(|p| p.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_str())
                        .collect::<Vec<_>>()
                        .join(" ")
                })
                .unwrap_or_default();
            let about = command.get("about").and_then(|x| x.as_str()).unwrap_or("");
            println!("{} — {}", path, about);
        }
        Ok(())
    }
}

/// 下位の命令をたどり、葉（これ以上下が無い命令）だけを集める。
///
/// 隠した命令（`is_hide_set`）は出さない。
fn collect(cmd: &clap::Command, path: &[String], out: &mut Vec<serde_json::Value>) {
    let subs: Vec<&clap::Command> = cmd.get_subcommands().filter(|c| !c.is_hide_set()).collect();
    if subs.is_empty() {
        out.push(command_to_json(cmd, path));
        return;
    }
    for sub in subs {
        let mut next = path.to_vec();
        next.push(sub.get_name().to_string());
        collect(sub, &next, out);
    }
}

/// 葉の命令 1 つを「schema の形」にする。
fn command_to_json(cmd: &clap::Command, path: &[String]) -> serde_json::Value {
    let args: Vec<serde_json::Value> = cmd
        .get_arguments()
        .filter(|a| !a.is_hide_set())
        .filter(|a| {
            let id = a.get_id().as_str();
            // `--help`・`--version` は出さない
            id != "help" && id != "version"
        })
        // `--json` は args に出さず、`json` フィールドで表す
        .filter(|a| a.get_long() != Some("json"))
        .map(arg_to_json)
        .collect();
    let has_json = cmd
        .get_arguments()
        .any(|a| a.get_long() == Some("json"));
    serde_json::json!({
        "path": path,
        "about": about_of(cmd),
        "stdin": stdin_kind(path),
        "json": has_json,
        "web": web_default(path),
        "args": args,
    })
}

/// 説明の 1 行目。
fn about_of(cmd: &clap::Command) -> String {
    cmd.get_about()
        .map(|s| s.to_string().lines().next().unwrap_or("").trim().to_string())
        .unwrap_or_default()
}

/// 引数 1 つを「schema の形」にする。
fn arg_to_json(arg: &clap::Arg) -> serde_json::Value {
    let name = arg
        .get_long()
        .map(str::to_string)
        .unwrap_or_else(|| arg.get_id().as_str().to_string());
    let action = arg.get_action();
    let kind = if arg.is_positional() {
        "positional"
    } else if matches!(
        action,
        ArgAction::SetTrue | ArgAction::SetFalse | ArgAction::Count
    ) {
        "flag"
    } else {
        "option"
    };
    let values: Vec<String> = arg
        .get_possible_values()
        .iter()
        .map(|v| v.get_name().to_string())
        .collect();
    let type_ = if !values.is_empty() {
        "enum".to_string()
    } else {
        value_type(arg)
    };
    let default = arg
        .get_default_values()
        .first()
        .map(|v| v.to_string_lossy().to_string());
    let multiple = matches!(action, ArgAction::Append)
        || arg
            .get_num_args()
            .map(|r| r.max_values() > 1)
            .unwrap_or(false);
    // パスを受ける引数は Web に出さない（iPhone からパスを受け取らない約束）
    let web = type_ != "path";
    serde_json::json!({
        "name": name,
        "kind": kind,
        "type": type_,
        "required": arg.is_required_set(),
        "multiple": multiple,
        "values": values,
        "default": default,
        "help": arg.get_help().map(|h| h.to_string()).unwrap_or_default(),
        "web": web,
    })
}

/// 引数の型（`string`・`int`・`float`・`path`）を値のパーサから判定する。
fn value_type(arg: &clap::Arg) -> String {
    use std::any::TypeId;
    let tid = arg.get_value_parser().type_id();
    if tid == TypeId::of::<i64>()
        || tid == TypeId::of::<u64>()
        || tid == TypeId::of::<u32>()
        || tid == TypeId::of::<i32>()
        || tid == TypeId::of::<usize>()
    {
        "int".to_string()
    } else if tid == TypeId::of::<f64>() || tid == TypeId::of::<f32>() {
        "float".to_string()
    } else if tid == TypeId::of::<std::path::PathBuf>() {
        "path".to_string()
    } else {
        "string".to_string()
    }
}

/// 標準入力の受け取り方（clap に無い情報）。
fn stdin_kind(path: &[String]) -> &'static str {
    match path.join(" ").as_str() {
        "card create" | "task create" => "json",
        "doc write" | "def write" | "task report" | "run md" => "text",
        _ => "none",
    }
}

/// Web の「コマンド」画面に出すか（clap に無い情報）。
///
/// `false` にするのは、エージェントの run の中でしか意味の無い命令と、
/// 下回りの命令（`init`・`daemon recover`・`usage backfill`・`run export`・`schema`）。
fn web_default(path: &[String]) -> bool {
    const AGENT_ONLY: [&str; 6] = [
        "doc write",
        "task create",
        "task execute",
        "task wait",
        "task report",
        "task question",
    ];
    const LOWER_LEVEL: [&str; 5] = [
        "init",
        "daemon recover",
        "usage backfill",
        "run export",
        "schema",
    ];
    let joined = path.join(" ");
    !AGENT_ONLY.contains(&joined.as_str()) && !LOWER_LEVEL.contains(&joined.as_str())
}
