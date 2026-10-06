//! `sai-paper schema --json` — 命令の一覧を JSON で出す。
//!
//! 形は CLI 共通の約束の「schema の形」と同じ。`sai-link` 側（U06）と同じ形にするが、
//! 2 つのクレートは互いに依存しない約束なので、クレートをまたいで共有はしない。
//!
//! clap の定義（[`crate::Cli`]）をたどり、下位の命令を持たない命令（葉）だけを並べる。
//! 隠した命令（`hide = true`）と `--help`・`--version`・`--json`・隠した引数は出さない。

use anyhow::Result;
use clap::{Arg, ArgAction, Command, CommandFactory};

/// `schema` を実行する。
pub fn print_schema(_json: bool) -> Result<()> {
    let value = schema_json();
    println!("{}", serde_json::to_string_pretty(&value)?);
    Ok(())
}

/// 命令の一覧の JSON を組み立てる。
pub fn schema_json() -> serde_json::Value {
    let cmd = crate::Cli::command();
    let mut commands = Vec::new();
    collect_leaves(&cmd, &mut Vec::new(), &mut commands);
    serde_json::json!({
        "bin": cmd.get_name(),
        "version": cmd.get_version().unwrap_or(""),
        "commands": commands,
    })
}

/// 葉の命令を集める（下位の命令を持たない命令だけ）。
fn collect_leaves(cmd: &Command, path: &mut Vec<String>, out: &mut Vec<serde_json::Value>) {
    for sub in cmd.get_subcommands() {
        if sub.is_hide_set() {
            continue;
        }
        path.push(sub.get_name().to_string());
        if sub.has_subcommands() {
            collect_leaves(sub, path, out);
        } else {
            out.push(leaf_json(sub, path));
        }
        path.pop();
    }
}

/// 葉の命令 1 つ分の JSON。
fn leaf_json(cmd: &Command, path: &[String]) -> serde_json::Value {
    let about = first_line(cmd.get_about().map(std::string::ToString::to_string));
    let args: Vec<serde_json::Value> = cmd
        .get_arguments()
        .filter(|arg| is_visible_arg(arg))
        .map(|arg| arg_json(arg, path))
        .collect();
    serde_json::json!({
        "path": path,
        "about": about,
        "stdin": stdin_kind(path),
        "json": has_json_flag(cmd),
        "web": web_allowed(path),
        "args": args,
    })
}

/// 説明の 1 行目だけを取り出す。
fn first_line(text: Option<String>) -> String {
    text.unwrap_or_default()
        .lines()
        .next()
        .unwrap_or("")
        .to_string()
}

/// `--help`・`--version`・`--json` と隠した引数は出さない。
fn is_visible_arg(arg: &Arg) -> bool {
    if arg.is_hide_set() {
        return false;
    }
    !matches!(arg.get_id().as_str(), "help" | "version" | "json")
}

/// `--json` を受け付けるか。
fn has_json_flag(cmd: &Command) -> bool {
    cmd.get_arguments().any(|arg| arg.get_id() == "json")
}

/// 引数 1 つ分の JSON。
fn arg_json(arg: &Arg, path: &[String]) -> serde_json::Value {
    let kind = if arg.is_positional() {
        "positional"
    } else if matches!(arg.get_action(), ArgAction::SetTrue | ArgAction::SetFalse) {
        "flag"
    } else {
        "option"
    };
    let value_type = value_type(arg, path);
    let values: Vec<String> = arg
        .get_possible_values()
        .iter()
        .map(|value| value.get_name().to_string())
        .collect();
    let default = arg
        .get_default_values()
        .first()
        .map(|value| value.to_string_lossy().into_owned());
    serde_json::json!({
        "name": arg.get_id().as_str(),
        "kind": kind,
        "type": value_type,
        "required": arg.is_required_set(),
        "multiple": matches!(arg.get_action(), ArgAction::Append),
        "values": values,
        "default": default,
        "help": first_line(arg.get_help().map(std::string::ToString::to_string)),
        "web": value_type != "path",
    })
}

/// 引数の型（`string`・`int`・`float`・`enum`・`path`）。
///
/// パスは clap の `PathBuf` から、数は `i64`・`u16` などの型から見分ける。
/// `--file` のように文字で受けるがパスとして扱う物だけ、命令の並びで上書きする。
fn value_type(arg: &Arg, path: &[String]) -> &'static str {
    use std::any::TypeId;
    // 選べる値があるものは enum。
    if !arg.get_possible_values().is_empty() {
        return "enum";
    }
    let name = arg.get_id().as_str();
    // 文字で受けるが実体はパスの引数（iPhone にパスを出さないため）。
    let is_paper_import = path.len() == 2 && path[0] == "paper" && path[1] == "import-pdf";
    let is_set_create = path.len() == 2 && path[0] == "set" && path[1] == "create";
    if name == "file" && (is_paper_import || is_set_create) {
        return "path";
    }
    let tid = arg.get_value_parser().type_id();
    if tid == TypeId::of::<std::path::PathBuf>() {
        return "path";
    }
    if tid == TypeId::of::<f64>() {
        return "float";
    }
    let is_int = [
        TypeId::of::<i8>(),
        TypeId::of::<i16>(),
        TypeId::of::<i32>(),
        TypeId::of::<i64>(),
        TypeId::of::<isize>(),
        TypeId::of::<u8>(),
        TypeId::of::<u16>(),
        TypeId::of::<u32>(),
        TypeId::of::<u64>(),
        TypeId::of::<usize>(),
    ]
    .iter()
    .any(|int_type| arg.get_value_parser().type_id() == *int_type);
    if is_int {
        return "int";
    }
    "string"
}

/// 標準入力から読むもの（`none`・`text`・`bytes`）。
fn stdin_kind(path: &[String]) -> &'static str {
    let words: Vec<&str> = path.iter().map(String::as_str).collect();
    match words.as_slice() {
        ["paper", "note"] | ["import", "add"] => "text",
        ["paper", "import-pdf"] => "bytes",
        _ => "none",
    }
}

/// Web の「コマンド」画面に出すか。
fn web_allowed(path: &[String]) -> bool {
    let words: Vec<&str> = path.iter().map(String::as_str).collect();
    !matches!(
        words.as_slice(),
        ["init"] | ["schema"] | ["activity", "open"]
    )
}

#[cfg(test)]
mod tests {
    use super::schema_json;

    /// `schema --json` の形が読め、`paper note` の stdin が text、`init` の web が false。
    #[test]
    fn schema_has_expected_shape() {
        let value = schema_json();
        assert_eq!(value["bin"], "sai-paper");
        let commands = value["commands"].as_array().expect("commands は配列");
        assert!(!commands.is_empty(), "命令が 1 つも無い");

        let find = |path: &[&str]| {
            commands.iter().find(|cmd| {
                cmd["path"].as_array().is_some_and(|words| {
                    words.iter().map(|w| w.as_str().unwrap_or("")).collect::<Vec<_>>() == path
                })
            })
        };

        let note = find(&["paper", "note"]).expect("paper note がある");
        assert_eq!(note["stdin"], "text");
        let init = find(&["init"]).expect("init がある");
        assert_eq!(init["web"], false);
        let schema = find(&["schema"]).expect("schema がある");
        assert_eq!(schema["web"], false);

        // 隠した命令は出ない。
        for hidden in ["phone", "get-pdf", "extract", "translate-pdf", "figures", "graph-show"] {
            assert!(find(&[hidden]).is_none(), "隠した {hidden} が出ている");
        }

        // パスを受ける引数は type が path で web が false。
        let import_pdf = find(&["paper", "import-pdf"]).expect("paper import-pdf がある");
        let file = import_pdf["args"]
            .as_array()
            .unwrap()
            .iter()
            .find(|arg| arg["name"] == "file")
            .expect("--file がある");
        assert_eq!(file["type"], "path");
        assert_eq!(file["web"], false);
    }
}
