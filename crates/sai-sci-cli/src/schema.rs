//! `sai-sci schema --json` — 命令の一覧を JSON で出す。
//!
//! 形は CLI 共通の約束の「schema の形」と同じ。`sai-link` 側（U06）と同じ形にするが、
//! クレートをまたいで共有はしない。
//!
//! clap の定義（[`crate::Cli`]）をたどり、下位の命令を持たない命令（葉）だけを並べる。
//! 隠した命令（`hide = true`）と `--help`・`--version`・`--json` と隠した引数は出さない。

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
        .map(|arg| arg_json(arg))
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
fn arg_json(arg: &Arg) -> serde_json::Value {
    let kind = if arg.is_positional() {
        "positional"
    } else if matches!(arg.get_action(), ArgAction::SetTrue | ArgAction::SetFalse) {
        "flag"
    } else {
        "option"
    };
    let value_type = value_type(arg);
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
/// パスは clap の `PathBuf` から見分ける。`dsr check` の位置引数（report）と
/// `report add --path` は `PathBuf` なので、ここで `path` になる。
fn value_type(arg: &Arg) -> &'static str {
    use std::any::TypeId;
    // 選べる値があるものは enum。
    if !arg.get_possible_values().is_empty() {
        return "enum";
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

/// 標準入力から読むもの（`none`・`json`・`text`・`bytes`）。
fn stdin_kind(path: &[String]) -> &'static str {
    let words: Vec<&str> = path.iter().map(String::as_str).collect();
    match words.as_slice() {
        ["type-save"] | ["report", "prepare"] => "text",
        // `card` は `--from-json` のときだけ読む（about にそう書いてある）。
        ["op"] | ["apply"] | ["card"] => "json",
        _ => "none",
    }
}

/// Web の「コマンド」画面に出すか。
///
/// `false` にするのは、エージェントの run の中でしか意味の無い命令（`context`・`op`・
/// `report prepare`・`op`・`approval record`）と、下回りの命令（`init`・`schema`）。
fn web_allowed(path: &[String]) -> bool {
    let words: Vec<&str> = path.iter().map(String::as_str).collect();
    !matches!(
        words.as_slice(),
        ["init"] | ["schema"] | ["report", "prepare"] | ["context"] | ["op"] | ["approval", "record"]
    )
}

#[cfg(test)]
mod tests {
    use super::schema_json;

    /// `schema --json` の形が読め、`bin` が sai-sci、`init`・`schema` の web が false。
    #[test]
    fn schema_has_expected_shape() {
        let value = schema_json();
        assert_eq!(value["bin"], "sai-sci");
        let commands = value["commands"].as_array().expect("commands は配列");
        assert!(!commands.is_empty(), "命令が 1 つも無い");

        let find = |path: &[&str]| {
            commands.iter().find(|cmd| {
                cmd["path"].as_array().is_some_and(|words| {
                    words.iter().map(|w| w.as_str().unwrap_or("")).collect::<Vec<_>>() == path
                })
            })
        };

        let init = find(&["init"]).expect("init がある");
        assert_eq!(init["web"], false);
        let schema = find(&["schema"]).expect("schema がある");
        assert_eq!(schema["web"], false);
        // 提案の命令は消えた（op と approval record に置き換わった）。
        for gone in [&["propose"][..], &["proposals"][..], &["triage"][..]] {
            assert!(find(gone).is_none(), "{gone:?} は消えているはず");
        }
        // エージェントの run の中だけで使う命令。
        for path in [&["context"][..], &["op"][..], &["report", "prepare"][..], &["approval", "record"][..]] {
            let cmd = find(path).unwrap_or_else(|| panic!("{path:?} がある"));
            assert_eq!(cmd["web"], false, "{path:?}");
        }

        // 標準入力の形。
        assert_eq!(find(&["type-save"]).unwrap()["stdin"], "text");
        assert_eq!(find(&["op"]).unwrap()["stdin"], "json");
        assert_eq!(find(&["card"]).unwrap()["stdin"], "json");
        assert_eq!(find(&["report", "prepare"]).unwrap()["stdin"], "text");
        assert_eq!(find(&["status"]).unwrap()["stdin"], "none");

        // パスを受ける引数は type が path で web が false。
        let check = find(&["dsr", "check"]).expect("dsr check がある");
        let report = check["args"]
            .as_array()
            .unwrap()
            .iter()
            .find(|arg| arg["name"] == "report")
            .expect("report の位置引数がある");
        assert_eq!(report["type"], "path");
        assert_eq!(report["web"], false);

        let add = find(&["report", "add"]).expect("report add がある");
        let path = add["args"]
            .as_array()
            .unwrap()
            .iter()
            .find(|arg| arg["name"] == "path")
            .expect("--path がある");
        assert_eq!(path["type"], "path");
        assert_eq!(path["web"], false);
    }
}
