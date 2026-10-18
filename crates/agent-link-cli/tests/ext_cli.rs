//! `agent-link ext` の統合テスト（U02）。
//!
//! 一時フォルダに置き場（`AGENT_LINK_HOME/extensions`）・登録簿・プロジェクトを作り、
//! ビルドした `agent-link` を動かす。見本の拡張は engine の `tests/fixtures/ext/hello`。

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU32, Ordering};

use serde_json::{json, Value};

fn sai_binary() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_agent-link"))
}

/// 見本の拡張（`hello`）のフォルダ。
fn hello_fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../agent-link-engine/tests/fixtures/ext/hello")
}

/// 一時フォルダの組（テストごとに別）。`root` は `agent-link init` 済みのプロジェクト。
struct Env {
    base: PathBuf,
    home: PathBuf,
    root: PathBuf,
}

impl Env {
    fn new(tag: &str) -> Self {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let base =
            std::env::temp_dir().join(format!("agent-link-ext-{tag}-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let env = Self {
            home: base.join("home"),
            root: base.join("proj"),
            base,
        };
        std::fs::create_dir_all(&env.root).expect("プロジェクトのフォルダ作成に失敗");
        let out = env.run(&["init", "--name", "proj"]);
        assert!(out.status.success(), "init が失敗: {}", stderr(&out));
        env
    }

    fn ext_dir(&self) -> PathBuf {
        self.home.join("extensions")
    }

    /// プロジェクト（`root`）の中で `agent-link <args>` を動かす。
    fn run(&self, args: &[&str]) -> Output {
        self.command(args)
            .output()
            .expect("agent-link の実行に失敗しました")
    }

    fn command(&self, args: &[&str]) -> Command {
        let mut cmd = Command::new(sai_binary());
        cmd.args(args)
            .current_dir(&self.root)
            .env("AGENT_LINK_HOME", &self.home)
            .env("AGENT_LINK_PROJECT_ROOT", &self.root)
            .env_remove("AGENT_LINK_EXTENSIONS_DIR")
            .env_remove("AGENT_LINK_PARENT_RUN_ID")
            .env_remove("AGENT_LINK_BLOCK_NAME");
        cmd
    }

    /// 標準入力に `input` を流して動かす（子が読み切らずに終わっても詰まらない）。
    fn run_stdin(&self, args: &[&str], input: &[u8]) -> Output {
        use std::io::Write;
        use std::process::Stdio;
        let mut child = self
            .command(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("agent-link の起動に失敗しました");
        let mut stdin = child.stdin.take().unwrap();
        let input = input.to_vec();
        let writer = std::thread::spawn(move || {
            let _ = stdin.write_all(&input);
        });
        let out = child.wait_with_output().expect("待機に失敗しました");
        let _ = writer.join();
        out
    }

    /// `hello`（`hooks.question` を宣言した見本）を置き場に入れて有効にする。
    fn enable_hooked_hello(&self) {
        let dir = self.base.join("hooked/hello");
        std::fs::create_dir_all(dir.join("bin")).unwrap();
        std::fs::write(
            dir.join("extension.json"),
            r#"{"name": "hello", "title": "Hello", "cli": {"bin": "bin/hello"}, "hooks": {"question": ["q", "{run_id}"]}}"#,
        )
        .unwrap();
        let bin = dir.join("bin/hello");
        std::fs::write(
            &bin,
            "#!/bin/sh\ncase \"$1\" in\n  q) cat > \"$AGENT_LINK_PROJECT_ROOT/q.json\"; [ \"$2\" = 9 ] && exit 5; exit 0 ;;\n  echo) cat ;;\nesac\n",
        )
        .unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        json(&self.run(&["ext", "install", &path_str(&dir), "--json"]));
        json(&self.run(&["ext", "enable", "hello", "--json"]));
    }

    /// プロジェクトの外（`base`）で動かす。`AGENT_LINK_PROJECT_ROOT` も外す。
    fn run_outside(&self, args: &[&str]) -> Output {
        let mut cmd = self.command(args);
        cmd.current_dir(&self.base)
            .env_remove("AGENT_LINK_PROJECT_ROOT");
        cmd.output().expect("agent-link の実行に失敗しました")
    }
}

impl Drop for Env {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.base);
    }
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

/// 成功した命令の標準出力を JSON として読む。
fn json(out: &Output) -> Value {
    assert!(out.status.success(), "失敗: {}", stderr(out));
    serde_json::from_slice(&out.stdout)
        .unwrap_or_else(|e| panic!("標準出力が JSON でない（{e}）: {}", stdout(out)))
}

/// 失敗のとき: 終了コードが `code`、標準出力は空、標準エラーは 1 行以上。
fn assert_failure(out: &Output, code: i32) {
    assert_eq!(
        out.status.code(),
        Some(code),
        "終了コード: stderr={}",
        stderr(out)
    );
    assert!(
        out.stdout.is_empty(),
        "失敗のとき標準出力は空: {}",
        stdout(out)
    );
    assert!(!stderr(out).trim().is_empty(), "標準エラーに理由が出る");
}

fn path_str(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

#[test]
fn install_enable_run_disable_uninstall() {
    let env = Env::new("flow");
    let hello = path_str(&hello_fixture());

    // 複製で入れる
    let v = json(&env.run(&["ext", "install", &hello, "--json"]));
    assert_eq!(v["name"], "hello");
    assert_eq!(v["linked"], false);
    assert!(env.ext_dir().join("hello/extension.json").is_file());
    assert!(env.ext_dir().join("hello/bin/hello").is_file());

    // list --json の形（C3 の項目がすべてある）
    let v = json(&env.run(&["ext", "list", "--json"]));
    assert_eq!(v["dir"], path_str(&env.ext_dir()));
    assert_eq!(v["errors"], json!([]));
    let ext = &v["extensions"][0];
    for key in [
        "name",
        "title",
        "version",
        "description",
        "path",
        "linked",
        "requires",
        "cli",
        "init",
        "update",
        "blocks",
        "templates",
        "ui",
        "service",
        "hooks",
        "enabled_in",
    ] {
        assert!(ext.get(key).is_some(), "list --json に {key} が無い: {ext}");
    }
    assert_eq!(ext["name"], "hello");
    assert_eq!(ext["cli"]["bin"], "bin/hello");
    assert_eq!(ext["blocks"], json!(["hello-agent.md"]));
    assert_eq!(ext["templates"], json!(["hello-rules.md"]));
    assert_eq!(ext["enabled_in"], json!([]));

    // 有効にする: block・template を写し、init を起こす
    let v = json(&env.run(&["ext", "enable", "hello", "--json"]));
    assert_eq!(v["name"], "hello");
    assert_eq!(v["enabled"], true);
    assert_eq!(v["init"], "ran");
    assert!(
        env.root.join(".agent-link/ext/hello").is_dir(),
        "init が作るデータ"
    );
    assert!(env.root.join(".agent-link/blocks/hello-agent.md").is_file());
    assert!(env
        .root
        .join(".agent-link/template/hello-rules.md")
        .is_file());

    // 拡張の CLI: 標準出力をそのまま流す
    let out = env.run(&["ext", "run", "hello", "--", "greet", "x"]);
    assert!(out.status.success(), "run: {}", stderr(&out));
    assert_eq!(json(&out), json!({"hello": "x"}));

    // 失敗: 終了コードは子のまま（3）、標準出力は空、標準エラーは子の物
    let out = env.run(&["ext", "run", "hello", "--", "fail"]);
    assert_failure(&out, 3);
    assert!(stderr(&out).contains("Something went wrong."));

    // 無効にする: 写した物を消す
    let v = json(&env.run(&["ext", "disable", "hello", "--json"]));
    assert_eq!(v["enabled"], false);
    assert!(!env.root.join(".agent-link/blocks/hello-agent.md").exists());
    assert!(!env
        .root
        .join(".agent-link/template/hello-rules.md")
        .exists());

    // 消す
    let v = json(&env.run(&["ext", "uninstall", "hello", "--json"]));
    assert_eq!(v, json!({"name": "hello", "removed": true}));
    assert!(!env.ext_dir().join("hello").exists());
}

#[test]
fn error_exit_codes_and_empty_stdout() {
    let env = Env::new("errors");
    let hello = path_str(&hello_fixture());

    // 入っていない拡張を有効にする: 3
    assert_failure(&env.run(&["ext", "enable", "nosuch"]), 3);

    // 壊れた宣言の install: 2（知らない項目 bogus）
    let broken = env.base.join("broken");
    std::fs::create_dir_all(&broken).unwrap();
    std::fs::write(
        broken.join("extension.json"),
        r#"{"name": "broken", "title": "Broken", "bogus": 1}"#,
    )
    .unwrap();
    assert_failure(&env.run(&["ext", "install", &path_str(&broken)]), 2);
    assert!(!env.ext_dir().join("broken").exists(), "壊れた物は置かない");

    // 有効な拡張の uninstall: 4
    json(&env.run(&["ext", "install", &hello, "--json"]));
    json(&env.run(&["ext", "enable", "hello", "--json"]));
    assert_failure(&env.run(&["ext", "uninstall", "hello"]), 4);
    assert!(env.ext_dir().join("hello").is_dir(), "消されない");

    // 有効でない拡張の run: 4
    json(&env.run(&["ext", "disable", "hello", "--json"]));
    assert_failure(&env.run(&["ext", "run", "hello", "--", "greet", "x"]), 4);

    // プロジェクトの外での enable: 2（needs_project）。list は外でも 0
    let out = env.run_outside(&["ext", "enable", "hello"]);
    assert_failure(&out, 2);
    let out = env.run_outside(&["ext", "list", "--json"]);
    assert_eq!(json(&out)["extensions"][0]["name"], "hello");
}

#[test]
fn hook_question_passes_stdin_to_the_owner() {
    let env = Env::new("hook-question");
    env.enable_hooked_hello();
    let input = r#"{"run_id": 7, "block": "hello-agent", "qid": "q1", "questions": [{"question": "Go?"}], "answers": {"Go?": "yes"}, "answered_by": "user"}"#;

    // 持ち主あり: 子が標準入力の中身をそのまま受け取る
    let v = json(&env.run_stdin(&["ext", "hook", "question"], input.as_bytes()));
    assert_eq!(v, json!({"ext": "hello", "ran": true, "code": 0}));
    assert_eq!(std::fs::read_to_string(env.root.join("q.json")).unwrap(), input);

    // 子が 0 以外で終わっても、この命令は 0 で終わる
    let failing = input.replace("\"run_id\": 7", "\"run_id\": 9");
    let v = json(&env.run_stdin(&["ext", "hook", "question", "--json"], failing.as_bytes()));
    assert_eq!(v, json!({"ext": "hello", "ran": true, "code": 5}));

    // 持ち主なし（block が当てはまらない）
    let other = input.replace("hello-agent", "plan");
    let v = json(&env.run_stdin(&["ext", "hook", "question"], other.as_bytes()));
    assert_eq!(v, json!({"ext": null, "ran": false, "code": null}));
}

#[test]
fn hook_question_rejects_bad_input() {
    let env = Env::new("hook-bad");
    env.enable_hooked_hello();
    for input in ["not json", r#"{"block": "hello"}"#, r#"{"run_id": "7", "block": "hello"}"#, r#"{"run_id": 7}"#] {
        assert_failure(&env.run_stdin(&["ext", "hook", "question"], input.as_bytes()), 2);
    }
    let out = env.run_stdin(&["ext", "hook", "nosuch"], b"{}");
    assert_failure(&out, 2);
    assert!(stderr(&out).contains("Unknown hook point: nosuch. Use question."));
}

#[test]
fn run_stdin_passes_body_and_checks_limits() {
    let env = Env::new("run-stdin");
    env.enable_hooked_hello();

    // 改行を含む本文がそのまま届く
    let out = env.run_stdin(&["ext", "run", "hello", "--stdin", "--", "echo"], b"a\nb\n");
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(stdout(&out), "a\nb\n");

    // 上限ちょうどは通り、1 バイト超えると 2
    let limit = vec![b'x'; 1_000_000];
    let out = env.run_stdin(&["ext", "run", "hello", "--stdin", "--", "echo"], &limit);
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(out.stdout.len(), 1_000_000);
    let over = vec![b'x'; 1_000_001];
    let out = env.run_stdin(&["ext", "run", "hello", "--stdin", "--", "echo"], &over);
    assert_failure(&out, 2);
    assert!(stderr(&out).contains("Input is larger than 1000000 bytes."));

    // UTF-8 でなければ 2
    let out = env.run_stdin(&["ext", "run", "hello", "--stdin", "--", "echo"], &[0xff, 0xfe]);
    assert_failure(&out, 2);
    assert!(stderr(&out).contains("Input must be UTF-8 text."));

    // --stdin が無ければ子の標準入力は閉じている（読んでも空）
    let out = env.run_stdin(&["ext", "run", "hello", "--", "echo"], b"ignored");
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(out.stdout.is_empty());
}
