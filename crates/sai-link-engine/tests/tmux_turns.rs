//! AC5: cc の tmux 経路の結合テスト。
//!
//! 偽の cc（標準入力の 1 行を読むたびに `got: <行>` を出す sh スクリプト）を
//! `tmux::run` で tmux の session に起こし、`send_text` + `send_keys("Alt+Enter")` の後に
//! 画面（`read_output`）に `got: <本文>` が出ること、`close` で session が消えることを確かめる。
//!
//! tmux が使えない環境（`tmux -V` が失敗する）では skip と表示して成功で返す。
//! 一時フォルダをプロジェクトルート・`SAI_HOME`・`HOME` にするので、本物の
//! `~/.claude.json` や `.sai` は書き換えない。

use sai_link_engine::headless;
use sai_link_engine::tmux;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::process::Command;
use std::time::{Duration, Instant};

/// 終わるときに session を消す（失敗した場合も残さないため）。
struct KillSession(String);

impl Drop for KillSession {
    fn drop(&mut self) {
        let _ = Command::new("tmux").args(["kill-session", "-t", &self.0]).output();
    }
}

/// `tmux -V` が成功するか（Mac で dyld が落ちる環境では false になる）。
fn tmux_usable() -> bool {
    Command::new("tmux")
        .arg("-V")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// 画面に `want` が出るまで最大 `timeout` 待つ。最後に見た画面を返す。
fn wait_for_screen(pane: &str, want: &str, timeout: Duration) -> String {
    let deadline = Instant::now() + timeout;
    loop {
        let screen = headless::read_output(pane, 50).unwrap_or_default();
        if screen.contains(want) {
            return screen;
        }
        if Instant::now() >= deadline {
            panic!("画面に {:?} が出ません。今の画面:\n{}", want, screen);
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

#[test]
fn test_cc_tmux_turn() {
    if !tmux_usable() {
        eprintln!("skip: tmux が使えないため cc の tmux 経路を確かめません（tmux -V が失敗）");
        return;
    }

    let project = tempfile::tempdir().unwrap();
    let sai_home = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    std::env::set_var("SAI_PROJECT_ROOT", project.path());
    std::env::set_var("SAI_HOME", sai_home.path());
    std::env::set_var("HOME", home.path());

    // 標準入力の 1 行ごとに `got: <行>` を出す偽の cc。
    let fake = project.path().join("fake_cc.sh");
    let mut f = std::fs::File::create(&fake).unwrap();
    f.write_all(b"#!/bin/sh\necho ready\nwhile IFS= read -r line; do echo \"got: $line\"; done\n")
        .unwrap();
    f.flush().unwrap();
    let mut perms = std::fs::metadata(&fake).unwrap().permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&fake, perms).unwrap();

    let pane = headless::new_pane(Some(&project.path().to_string_lossy())).unwrap();
    let id = pane.pane_id.clone();
    let _kill = KillSession(id.clone());

    tmux::run(&id, &format!("'{}'", fake.display())).unwrap();

    // 起動の前に整えたファイル（設定と初回の画面を出さない項目）。
    assert!(project.path().join(".claude/settings.json").is_file());
    let claude_json: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(home.path().join(".claude.json")).unwrap())
            .unwrap();
    assert_eq!(claude_json["hasCompletedOnboarding"], true);

    // run 直後は起動中（starting）。フックが無いので idle にはならない。
    let info = headless::status(&id).unwrap();
    assert_eq!(info.agent_status, "unknown", "起動中は unknown");

    wait_for_screen(&id, "ready", Duration::from_secs(10));
    headless::send_text(&id, "テスト文").unwrap();
    headless::send_keys(&id, "Alt+Enter").unwrap();
    wait_for_screen(&id, "got: テスト文", Duration::from_secs(10));

    // 複数行の文も 1 回の確定で届く（bracketed paste で改行を送信しない）。
    headless::send_text(&id, "一行目\n二行目").unwrap();
    headless::send_keys(&id, "Alt+Enter").unwrap();
    wait_for_screen(&id, "got: 二行目", Duration::from_secs(10));

    // close で session が消え、状態は stopped になる。
    headless::close(&id).unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let alive = Command::new("tmux")
            .args(["has-session", "-t", &id])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        if !alive {
            break;
        }
        assert!(Instant::now() < deadline, "close の後も session が残っています");
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(headless::status(&id).is_err(), "stopped の pane は見えない");
}
