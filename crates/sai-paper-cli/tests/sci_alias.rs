//! U15: 古い研究の命令が隠した別名として `sai-sci` に届くことの結合テスト。
//!
//! 偽の `sai-sci`（受けた引数を 1 行ずつファイルに書き、決めた終了コードと標準エラーを返す sh）を
//! `SAI_SCI_BIN` で子に渡す。環境変数は `Command::env` で渡し、試験の中で書き換えない。

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

/// テストで動かす実行ファイル。
fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_sai-moon-paper")
}

/// 一時フォルダ（`Drop` で消す）。
struct Temp {
    dir: PathBuf,
}

impl Temp {
    fn new(tag: &str) -> Self {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "sai-paper-alias-{tag}-{}-{unique}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self { dir }
    }

    fn path(&self) -> &Path {
        &self.dir
    }

    fn project(&self) -> PathBuf {
        self.dir.join("proj")
    }

    fn args_file(&self) -> PathBuf {
        self.dir.join("args.txt")
    }

    /// 偽の `sai-sci` を置いて、その絶対パスを返す。
    fn fake_sci(&self) -> PathBuf {
        let path = self.dir.join("sai-sci");
        let script = r#"#!/bin/sh
if [ -n "$FAKE_SCI_OUT" ]; then
  : > "$FAKE_SCI_OUT"
  for a in "$@"; do printf '%s\n' "$a" >> "$FAKE_SCI_OUT"; done
  printf -- '--stdin--\n' >> "$FAKE_SCI_OUT"
  cat >> "$FAKE_SCI_OUT" 2>/dev/null || true
fi
if [ -n "$FAKE_SCI_STDERR" ]; then printf '%s\n' "$FAKE_SCI_STDERR" >&2; fi
if [ -n "$FAKE_SCI_STDOUT" ]; then printf '%s' "$FAKE_SCI_STDOUT"; fi
exit "${FAKE_SCI_CODE:-0}"
"#;
        std::fs::write(&path, script).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        path
    }
}

impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// 偽の `sai-sci` が受けた引数を読む（標準入力の区切り `--stdin--` まで）。
fn read_args(temp: &Temp) -> Vec<String> {
    let text = std::fs::read_to_string(temp.args_file()).unwrap_or_default();
    let mut args = Vec::new();
    for line in text.lines() {
        if line == "--stdin--" {
            break;
        }
        args.push(line.to_string());
    }
    args
}

/// 偽の `sai-sci` へ渡した標準入力（区切りより後）を読む。
fn read_stdin(temp: &Temp) -> String {
    let text = std::fs::read_to_string(temp.args_file()).unwrap_or_default();
    match text.split_once("--stdin--\n") {
        Some((_, rest)) => rest.to_string(),
        None => String::new(),
    }
}

/// 偽の `sai-sci` を付けて実行する（標準入力は閉じる）。
fn run(temp: &Temp, args: &[&str]) -> (Output, Vec<String>) {
    let project = temp.project();
    std::fs::create_dir_all(project.join(".sai/paper")).unwrap();
    let out = Command::new(bin())
        .args(args)
        .env("SAI_SCI_BIN", temp.fake_sci())
        .env("FAKE_SCI_OUT", temp.args_file())
        .env("SAI_PROJECT_ROOT", &project)
        .current_dir(&project)
        .output()
        .expect("実行できる");
    (out, read_args(temp))
}

/// 偽の `sai-sci` を付けて、標準入力を渡して実行する。
fn run_stdin(temp: &Temp, args: &[&str], input: &str) -> (Output, Vec<String>, String) {
    let project = temp.project();
    std::fs::create_dir_all(project.join(".sai/paper")).unwrap();
    let mut child = Command::new(bin())
        .args(args)
        .env("SAI_SCI_BIN", temp.fake_sci())
        .env("FAKE_SCI_OUT", temp.args_file())
        .env("SAI_PROJECT_ROOT", &project)
        .current_dir(&project)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("起動できる");
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(input.as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    let args = read_args(temp);
    let stdin = read_stdin(temp);
    (out, args, stdin)
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// `sci …` はそのまま `sai-sci` に届く（先頭の `sci` を取る）。
#[test]
fn sci_subcommands_pass_through() {
    let temp = Temp::new("sci");
    let (out, args) = run(&temp, &["sci", "status", "--json"]);
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(args, ["status", "--json"]);

    let (_, args) = run(&temp, &["sci", "node", "Q12", "--json"]);
    assert_eq!(args, ["node", "Q12", "--json"]);
}

/// `brief`・`journal`・`progress`・`understanding`・`dsr`・`report` は同じ名前に届く。
#[test]
fn research_commands_pass_through() {
    let temp = Temp::new("research");
    let (_, args) = run(&temp, &["brief", "--write"]);
    assert_eq!(args, ["brief", "--write"]);
    let (_, args) = run(&temp, &["journal", "--days", "3", "--json"]);
    assert_eq!(args, ["journal", "--days", "3", "--json"]);
    let (_, args) = run(&temp, &["progress", "--json"]);
    assert_eq!(args, ["progress", "--json"]);
    let (_, args) = run(&temp, &["understanding", "add", "--topic", "T", "--state", "S"]);
    assert_eq!(args, ["understanding", "add", "--topic", "T", "--state", "S"]);
    let (_, args) = run(&temp, &["dsr", "check", "r.md", "--block", "b"]);
    assert_eq!(args, ["dsr", "check", "r.md", "--block", "b"]);
    let (_, args) = run(&temp, &["report", "list"]);
    assert_eq!(args, ["report", "list"]);
}

/// `attention` は下位の命令が無ければ `attention list`、`resolve` はそのまま。
#[test]
fn attention_maps_to_list_or_resolve() {
    let temp = Temp::new("attention");
    let (_, args) = run(&temp, &["attention"]);
    assert_eq!(args, ["attention", "list"]);
    let (_, args) = run(&temp, &["attention", "--json"]);
    assert_eq!(args, ["attention", "list", "--json"]);
    let (_, args) = run(&temp, &["attention", "resolve", "key:1"]);
    assert_eq!(args, ["attention", "resolve", "key:1"]);
}

/// `content report <id>` は `report content <id>` に届く。
#[test]
fn content_report_maps_to_report_content() {
    let temp = Temp::new("content");
    let (_, args) = run(&temp, &["content", "report", "R-0001", "--json"]);
    assert_eq!(args, ["report", "content", "R-0001", "--json"]);
}

/// `find` は `--node` があるときだけそのまま `sai-sci` に届く。
#[test]
fn find_with_node_passes_through() {
    let temp = Temp::new("find");
    let (_, args) = run(&temp, &["find", "EEG", "--node", "O5", "--subtree", "--each", "2", "--json"]);
    assert_eq!(
        args,
        ["find", "EEG", "--node", "O5", "--subtree", "--each", "2", "--json"]
    );
}

/// `search` は研究の引数があるときだけそのまま `sai-sci` に届く。
#[test]
fn search_with_research_args_passes_through() {
    let temp = Temp::new("search");
    let (_, args) = run(
        &temp,
        &["search", "EEG", "--node", "O5", "--purpose", "確かめる", "--json"],
    );
    assert_eq!(
        args,
        ["search", "EEG", "--node", "O5", "--purpose", "確かめる", "--limit", "10", "--json"]
    );
}

/// `init --sci --type K`・`init --type K` は `sai-sci init --type K` になる。
#[test]
fn init_sci_alias_runs_sai_sci_init() {
    let temp = Temp::new("init-sci");
    let (_, args) = run(&temp, &["init", "--sci", "--type", "problem"]);
    assert_eq!(args, ["init", "--type", "problem"]);
}

/// `sci init --json` は子を呼び、`tree` だけを出す。
#[test]
fn sci_init_json_prints_only_tree() {
    let temp = Temp::new("sci-init");
    let project = temp.project();
    std::fs::create_dir_all(project.join(".sai/paper")).unwrap();
    let out = Command::new(bin())
        .args(["sci", "init", "--type", "problem", "--json"])
        .env("SAI_SCI_BIN", temp.fake_sci())
        .env("FAKE_SCI_OUT", temp.args_file())
        .env(
            "FAKE_SCI_STDOUT",
            r#"{"messages":["a"],"tree":{"type":"problem","created":2}}"#,
        )
        .env("SAI_PROJECT_ROOT", &project)
        .current_dir(&project)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0));
    let tree: serde_json::Value = serde_json::from_str(stdout(&out).trim()).unwrap();
    assert_eq!(tree["type"], "problem");
    assert_eq!(tree["created"], 2);
    assert_eq!(read_args(&temp), ["init", "--type", "problem", "--json"]);
}

/// exec の別名は、子の終了コード（2・3・4）をそのまま返す。
#[test]
fn exec_alias_keeps_exit_code() {
    let temp = Temp::new("exit");
    for code in [2, 3, 4] {
        let project = temp.project();
        std::fs::create_dir_all(project.join(".sai/paper")).unwrap();
        let out = Command::new(bin())
            .args(["sci", "status"])
            .env("SAI_SCI_BIN", temp.fake_sci())
            .env("FAKE_SCI_OUT", temp.args_file())
            .env("FAKE_SCI_CODE", code.to_string())
            .env("SAI_PROJECT_ROOT", &project)
            .current_dir(&project)
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(code));
    }
}

/// `report prepare` は `sai-sci` が無いとき終了コード 2（sai-link の write_check のため）。
#[test]
fn report_prepare_missing_sai_sci_is_exit_2() {
    let temp = Temp::new("prepare-missing");
    let project = temp.project();
    std::fs::create_dir_all(project.join(".sai/paper")).unwrap();
    let out = Command::new(bin())
        .args(["report", "prepare", "--run", "1", "--block", "b", "--filename", "f.md"])
        .env("SAI_SCI_BIN", temp.path().join("does-not-exist"))
        .env("SAI_PROJECT_ROOT", &project)
        .current_dir(&project)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("sai-sci が見つかりません"));
}

/// exec の別名は、`sai-sci` が無いとき終了コード 1 と日本語の文。
#[test]
fn exec_alias_missing_sai_sci_is_exit_1() {
    let temp = Temp::new("missing");
    let project = temp.project();
    std::fs::create_dir_all(project.join(".sai/paper")).unwrap();
    let out = Command::new(bin())
        .args(["sci", "status"])
        .env("SAI_SCI_BIN", temp.path().join("does-not-exist"))
        .env("SAI_PROJECT_ROOT", &project)
        .current_dir(&project)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("sai-sci が見つかりません"));
}

/// `phone sci status` は形 A で子の JSON をそのまま出す。
#[test]
fn phone_sci_shape_a_prints_child_json() {
    let temp = Temp::new("phone-sci");
    let project = temp.project();
    std::fs::create_dir_all(project.join(".sai/paper")).unwrap();
    let out = Command::new(bin())
        .args(["phone", "sci", "status"])
        .env("SAI_SCI_BIN", temp.fake_sci())
        .env("FAKE_SCI_OUT", temp.args_file())
        .env("FAKE_SCI_STDOUT", r#"{"progress":1}"#)
        .env("SAI_PROJECT_ROOT", &project)
        .current_dir(&project)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(stdout(&out).trim(), r#"{"progress":1}"#);
    assert_eq!(read_args(&temp), ["status", "--json"]);
}

/// 形 A の失敗は `{"error": …}` と終了コード 1 になる（標準エラーの最後の行を使う）。
#[test]
fn phone_sci_shape_a_failure_is_json_error() {
    let temp = Temp::new("phone-sci-fail");
    let project = temp.project();
    std::fs::create_dir_all(project.join(".sai/paper")).unwrap();
    let out = Command::new(bin())
        .args(["phone", "sci", "node", "Q999"])
        .env("SAI_SCI_BIN", temp.fake_sci())
        .env("FAKE_SCI_OUT", temp.args_file())
        .env("FAKE_SCI_CODE", "3")
        .env("FAKE_SCI_STDERR", "Q999 はありません")
        .env("SAI_PROJECT_ROOT", &project)
        .current_dir(&project)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    let value: serde_json::Value = serde_json::from_str(stdout(&out).trim()).unwrap();
    assert_eq!(value["error"], "Q999 はありません");
    assert_eq!(read_args(&temp), ["node", "Q999", "--json"]);
}

/// `phone sci apply` は標準入力を子へ渡す。
#[test]
fn phone_sci_apply_forwards_stdin() {
    let temp = Temp::new("phone-sci-apply");
    let project = temp.project();
    std::fs::create_dir_all(project.join(".sai/paper")).unwrap();
    let (out, args, stdin) = run_stdin(&temp, &["phone", "sci", "apply"], r#"{"op":"add_o"}"#);
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(args, ["apply", "--json"]);
    assert_eq!(stdin.trim(), r#"{"op":"add_o"}"#);
}

/// `phone sci triage P1 --action accept --note=N` は `--accept` と標準入力の `--edit` になる。
#[test]
fn phone_sci_triage_accept_maps_args() {
    let temp = Temp::new("phone-sci-triage");
    let project = temp.project();
    std::fs::create_dir_all(project.join(".sai/paper")).unwrap();
    let (out, args, _) = run_stdin(
        &temp,
        &["phone", "sci", "triage", "P1", "--action", "accept", "--note=N"],
        r#"{"text":"別の文"}"#,
    );
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(args[0], "triage");
    assert_eq!(args[1], "P1");
    assert!(args.contains(&"--accept".to_string()));
    assert_eq!(args.last().unwrap(), "--note=N");
    assert!(args.iter().any(|a| a.starts_with("--edit=")), "edit が無い: {args:?}");
}

/// `phone sci triage … --action 知らない` は形 A の失敗になる。
#[test]
fn phone_sci_triage_unknown_action_fails() {
    let temp = Temp::new("phone-sci-triage-bad");
    let project = temp.project();
    std::fs::create_dir_all(project.join(".sai/paper")).unwrap();
    let out = Command::new(bin())
        .args(["phone", "sci", "triage", "P1", "--action", "nope"])
        .env("SAI_SCI_BIN", temp.fake_sci())
        .env("FAKE_SCI_OUT", temp.args_file())
        .env("SAI_PROJECT_ROOT", &project)
        .current_dir(&project)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    let value: serde_json::Value = serde_json::from_str(stdout(&out).trim()).unwrap();
    assert!(value["error"].as_str().unwrap().contains("知らない仕分けです"));
}

/// `phone project`・`phone mind-append`・`phone attention-resolve` も形 A で `sai-sci` を呼ぶ。
#[test]
fn phone_research_commands_use_sai_sci() {
    let temp = Temp::new("phone-project");
    let (out, args) = run(&temp, &["phone", "project"]);
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(args, ["project", "show", "--json"]);

    let temp = Temp::new("phone-memo");
    let (out, args, stdin) = run_stdin(&temp, &["phone", "mind-append"], "メモの 1 行\n");
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(args, ["project", "memo", "メモの 1 行", "--json"]);
    assert!(stdin.is_empty());

    let temp = Temp::new("phone-attention");
    let (out, args) = run(&temp, &["phone", "attention-resolve", "key:1"]);
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(args, ["attention", "resolve", "key:1", "--json"]);
}

/// `phone find --node` は形 A で `sai-sci find` を呼ぶ。
#[test]
fn phone_find_with_node_uses_sai_sci() {
    let temp = Temp::new("phone-find");
    let (out, args) = run(&temp, &["phone", "find", "EEG", "--node", "O5", "--subtree"]);
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(args, ["find", "EEG", "--node", "O5", "--subtree", "--json"]);
}

/// MCP の `sci_status` と `find`（`node` あり）が `sai-sci` を呼ぶ。
#[test]
fn mcp_tools_call_sai_sci() {
    let temp = Temp::new("mcp");
    let project = temp.project();
    std::fs::create_dir_all(project.join(".sai/paper")).unwrap();
    let mut child = Command::new(bin())
        .args(["mcp"])
        .env("SAI_SCI_BIN", temp.fake_sci())
        .env("FAKE_SCI_OUT", temp.args_file())
        .env("FAKE_SCI_STDOUT", r#"{"progress":2}"#)
        .env("SAI_PROJECT_ROOT", &project)
        .current_dir(&project)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    // initialize（通知でないので応答が返る）
    writeln!(
        stdin,
        r#"{{"jsonrpc":"2.0","id":1,"method":"initialize","params":{{"protocolVersion":"2025-06-18"}}}}"#
    )
    .unwrap();
    // sci_status
    writeln!(
        stdin,
        r#"{{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{{"name":"sci_status","arguments":{{}}}}}}"#
    )
    .unwrap();
    // sci_context
    writeln!(
        stdin,
        r#"{{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{{"name":"sci_context","arguments":{{"run_id":7}}}}}}"#
    )
    .unwrap();
    // sci_node
    writeln!(
        stdin,
        r#"{{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{{"name":"sci_node","arguments":{{"node":"Q1"}}}}}}"#
    )
    .unwrap();
    // find（node あり）
    writeln!(
        stdin,
        r#"{{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{{"name":"find","arguments":{{"query":"EEG","node":"O5"}}}}}}"#
    )
    .unwrap();
    // sci_propose（最後なので引数と標準入力を確かめる）
    writeln!(
        stdin,
        r#"{{"jsonrpc":"2.0","id":6,"method":"tools/call","params":{{"name":"sci_propose","arguments":{{"proposals":[{{"op":"add_o"}}],"run_id":7}}}}}}"#
    )
    .unwrap();
    drop(stdin);
    let out = child.wait_with_output().unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    let replies: Vec<serde_json::Value> = text
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(replies.len(), 6, "応答が 6 つでない: {text}");
    for reply in &replies[1..] {
        assert_ne!(reply["result"]["isError"], true, "道具が失敗している: {reply}");
    }
    assert!(replies[1]["result"]["content"][0]["text"]
        .as_str()
        .unwrap()
        .contains("progress"));
    assert!(replies[2]["result"]["content"][0]["text"]
        .as_str()
        .unwrap()
        .contains("text"));
    // 最後の sci_propose が受けた引数と標準入力
    let args = read_args(&temp);
    assert_eq!(args, ["propose", "--run", "7"]);
    assert!(read_stdin(&temp).contains("add_o"));
}

/// `phone sci` のほかの行の引数の並び（types・proposals・read・card・init）。
#[test]
fn phone_sci_other_mappings() {
    let temp = Temp::new("phone-sci-others");
    let (_, args) = run(&temp, &["phone", "sci", "types"]);
    assert_eq!(args, ["types", "--json"]);
    let (_, args) = run(&temp, &["phone", "sci", "proposals"]);
    assert_eq!(args, ["proposals", "--json"]);
    let (_, args) = run(&temp, &["phone", "sci", "read", "7"]);
    assert_eq!(args, ["read", "7", "--json"]);

    let (_, args, stdin) = run_stdin(&temp, &["phone", "sci", "card"], r#"{"node":"Q1"}"#);
    assert_eq!(args, ["card", "--from-json", "--json"]);
    assert!(stdin.contains("Q1"));

    // `phone sci init` は子を呼び、`tree` だけを出す。
    let project = temp.project();
    std::fs::create_dir_all(project.join(".sai/paper")).unwrap();
    let out = Command::new(bin())
        .args(["phone", "sci", "init", "--type", "problem"])
        .env("SAI_SCI_BIN", temp.fake_sci())
        .env("FAKE_SCI_OUT", temp.args_file())
        .env("FAKE_SCI_STDOUT", r#"{"messages":[],"tree":{"type":"problem"}}"#)
        .env("SAI_PROJECT_ROOT", &project)
        .current_dir(&project)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0));
    let tree: serde_json::Value = serde_json::from_str(stdout(&out).trim()).unwrap();
    assert_eq!(tree["type"], "problem");
    assert_eq!(read_args(&temp), ["init", "--type", "problem", "--json"]);
}

/// `--help` と `schema --json` に古い研究の命令が出ない。
#[test]
fn help_and_schema_hide_research_commands() {
    let out = Command::new(bin()).arg("--help").output().unwrap();
    let help = String::from_utf8_lossy(&out.stdout);
    for hidden in ["brief", "journal", "progress", "understanding", "attention", "sci", "dsr"] {
        assert!(
            !help.lines().any(|line| {
                let line = line.trim_start();
                line == hidden || line.starts_with(&format!("{hidden} "))
            }),
            "help に {hidden} が出ている"
        );
    }

    let out = Command::new(bin()).args(["schema", "--json"]).output().unwrap();
    let value: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let paths: Vec<Vec<String>> = value["commands"]
        .as_array()
        .unwrap()
        .iter()
        .map(|cmd| {
            cmd["path"]
                .as_array()
                .unwrap()
                .iter()
                .map(|w| w.as_str().unwrap().to_string())
                .collect()
        })
        .collect();
    for hidden in ["brief", "journal", "progress", "understanding", "attention", "sci", "dsr", "report"] {
        assert!(
            !paths.iter().any(|path| path == &vec![hidden.to_string()]),
            "schema に {hidden} が出ている"
        );
    }
}
