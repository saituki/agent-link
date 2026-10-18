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

    // 壊れた宣言の install: 2（知らない項目 hooks）
    let broken = env.base.join("broken");
    std::fs::create_dir_all(&broken).unwrap();
    std::fs::write(
        broken.join("extension.json"),
        r#"{"name": "broken", "title": "Broken", "hooks": {}}"#,
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
