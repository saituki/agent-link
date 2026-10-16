//! AC1: `agent-link run send` / `agent-link run interrupt` の結合テスト。
//!
//! 一時フォルダのプロジェクト・偽の cc（tmux の pane の中で動く sh）を使う。
//! CLI（別プロセス）が tmux の pane に入力と中断を送る経路を確かめる。
//!
//! - 動いている実行に `run send` すると次のターンが走る（画面に `got: <本文>` が出る）。
//! - `run interrupt` で中断でき、pane が `idle` に戻る。
//! - 無い実行は終了コード 3。空の文は終了コード 2。
//! - 状態のフックを呼ばない偽の cc には `status: "unconfirmed"`（終了コード 0）で返る。
//!
//! tmux が使えない環境では skip と表示して成功で返す。一時フォルダをプロジェクトルートと
//! `AGENT_LINK_HOME` にするので、本物の `.agent-link` や `~/.agent-link` は書き換えない。

use agent_link_engine::db::models::BlockMetadata;
use agent_link_engine::db::SaiStore;
use agent_link_engine::headless::{self, HeadlessState};
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{Duration, Instant};

/// `agent-link` のバイナリパス（Cargo が統合テスト用に提供する env）。
fn sai_binary() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_agent-link"))
}

/// 終わるときに tmux の session を消す（失敗した場合も残さないため）。
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

/// 実行権限つきで sh スクリプトを書く。
fn write_script(path: &Path, body: &str) {
    let mut f = std::fs::File::create(path).unwrap();
    f.write_all(body.as_bytes()).unwrap();
    f.flush().unwrap();
    let mut perms = std::fs::metadata(path).unwrap().permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(path, perms).unwrap();
}

/// `state.json` を読む（無ければ panic）。
fn read_state(pane: &str) -> HeadlessState {
    let path = headless::pane_dir(pane).join("state.json");
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("state.json を読めません ({}): {}", path.display(), e));
    serde_json::from_str(&text).expect("state.json のパース")
}

/// 状態が `want` になるまで最大 `timeout` 待つ。
fn wait_for_state(pane: &str, want: &str, timeout: Duration) -> HeadlessState {
    let deadline = Instant::now() + timeout;
    loop {
        let state = read_state(pane);
        if state.state == want {
            return state;
        }
        if Instant::now() >= deadline {
            panic!(
                "状態が {} になりません（今は {}、partial={:?}、error={:?}）",
                want, state.state, state.partial, state.error
            );
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// 画面に `want` が出るまで最大 `timeout` 待つ。
fn wait_for_screen(pane: &str, want: &str, timeout: Duration) {
    let deadline = Instant::now() + timeout;
    loop {
        let screen = headless::read_output(pane, 50).unwrap_or_default();
        if screen.contains(want) {
            return;
        }
        if Instant::now() >= deadline {
            panic!("画面に {:?} が出ません。今の画面:\n{}", want, screen);
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// 偽の cc（標準入力の 1 行ごとに `got: <行>` を出す）。
///
/// 本物の cc と同じく、ターンの始めと終わりに状態のフック（`cc-state-hook`）を呼ぶ。
/// `run send` は確定キーのあと `working` になるのを 250ms ごとに見に行くので、1 秒は `working` に留まる。`眠る` を含む行は
/// 30 秒 `working` のままにして、`run interrupt` で中断できるかを確かめる。
fn fake_cc() -> String {
    let bin = sai_binary();
    format!(
        r##"#!/bin/sh
echo ready
while IFS= read -r line; do
  '{bin}' cc-state-hook working </dev/null >/dev/null 2>&1
  # 確定キー M-Enter の ESC を画面に出すと、tmux が後ろの出力をエスケープ列として飲み込むので外す
  line=$(printf '%s' "$line" | tr -d '\033')
  echo "got: $line"
  case "$line" in *眠る*) sleep 30 ;; *) sleep 1 ;; esac
  '{bin}' cc-state-hook idle </dev/null >/dev/null 2>&1
done
"##,
        bin = bin.display()
    )
}

/// 状態のフックを呼ばない偽の cc（文は受け取るが `state.json` は idle のまま）。
fn fake_cc_no_hook() -> String {
    r##"#!/bin/sh
echo ready
while IFS= read -r line; do
  line=$(printf '%s' "$line" | tr -d '\033')
  echo "got: $line"
done
"##
    .to_string()
}

/// 一時プロジェクトの DB に `running` の実行を 1 件作り、ID を返す。
fn make_execution(project: &Path) -> i64 {
    use std::sync::atomic::{AtomicU32, Ordering};
    static SEQ: AtomicU32 = AtomicU32::new(0);
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    let db_path = project.join(".agent-link/engine/agent-link.db");
    std::fs::create_dir_all(db_path.parent().unwrap()).unwrap();
    let store = SaiStore::open(&db_path).unwrap();
    let block = BlockMetadata::new(
        format!("send-test-{}", seq),
        "送信のテスト".to_string(),
        "anthropic/claude-sonnet-4-5".to_string(),
        "off".to_string(),
        "true".to_string(),
        "true".to_string(),
        None,
        300,
        "none".to_string(),
        true,
        format!("/tmp/send-test-{}.md", seq),
        None,
    );
    let block_id = store.insert_block(&block).unwrap();
    let run_id = store.create_execution(block_id, None).unwrap();
    store.update_execution_status(run_id, "running").unwrap();
    run_id
}

/// 実行の pane を DB に記録する。
fn record_pane(project: &Path, run_id: i64, pane_id: &str) {
    let store = SaiStore::open(&project.join(".agent-link/engine/agent-link.db")).unwrap();
    store.update_execution_pane(run_id, pane_id, 0).unwrap();
}

/// 子プロセスとして `agent-link` を走らせる（プロジェクトと置き場を一時フォルダに固定）。
fn run_cli(project: &Path, home: &Path, args: &[&str]) -> Output {
    Command::new(sai_binary())
        .args(args)
        .env("AGENT_LINK_PROJECT_ROOT", project)
        .env("AGENT_LINK_HOME", home)
        .env_remove("AGENT_LINK_PARENT_RUN_ID")
        .env_remove("AGENT_LINK_BLOCK_NAME")
        .current_dir(project)
        .output()
        .expect("agent-link の実行に失敗しました")
}

#[test]
fn test_run_send_and_interrupt() {
    if !tmux_usable() {
        eprintln!("skip: tmux が使えないため run send / run interrupt を確かめません（tmux -V が失敗）");
        return;
    }
    let base = tempfile::tempdir().unwrap();
    let project = base.path().join("proj");
    let home = base.path().join("home");
    std::fs::create_dir_all(project.join(".agent-link")).unwrap();
    std::fs::create_dir_all(&home).unwrap();
    // 監督（このプロセスの中）と DB が同じ置き場を見るようにする。
    std::env::set_var("AGENT_LINK_PROJECT_ROOT", &project);
    std::env::set_var("AGENT_LINK_HOME", &home);

    let fake = project.join("fake_cc.sh");
    write_script(&fake, &fake_cc());
    let command = format!("'{}'", fake.display());

    // ---- 実行 1: 動いている実行に run send すると次のターンが走る ----
    let run1 = make_execution(&project);
    let pane1 = headless::new_pane(Some(&project.to_string_lossy())).unwrap();
    let _kill1 = KillSession(pane1.pane_id.clone());
    headless::run(&pane1.pane_id, &command).unwrap();
    record_pane(&project, run1, &pane1.pane_id);
    // 実行中の状態を作るため、最初のターンを自分で始める。
    headless::send_text(&pane1.pane_id, "最初の依頼").unwrap();
    headless::send_keys(&pane1.pane_id, "Alt+Enter").unwrap();
    wait_for_screen(&pane1.pane_id, "got: 最初の依頼", Duration::from_secs(10));

    let out = run_cli(
        &project,
        &home,
        &["run", "send", &run1.to_string(), "--text", "続きの依頼", "--json"],
    );
    assert!(
        out.status.success(),
        "run send が失敗した: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).expect("run send の JSON");
    assert_eq!(v["status"], "sent");
    assert_eq!(v["run_id"], run1);
    assert_eq!(v["pane_id"], pane1.pane_id);

    // 2 回目のターンが走り、画面に出る。
    wait_for_screen(&pane1.pane_id, "got: 続きの依頼", Duration::from_secs(10));

    // ---- 実行 2: run interrupt で中断でき、pane が idle に戻る ----
    let run2 = make_execution(&project);
    let pane2 = headless::new_pane(Some(&project.to_string_lossy())).unwrap();
    let _kill2 = KillSession(pane2.pane_id.clone());
    headless::run(&pane2.pane_id, &command).unwrap();
    record_pane(&project, run2, &pane2.pane_id);
    headless::send_text(&pane2.pane_id, "眠る依頼").unwrap();
    headless::send_keys(&pane2.pane_id, "Alt+Enter").unwrap();
    wait_for_screen(&pane2.pane_id, "got: 眠る依頼", Duration::from_secs(10));

    let out = run_cli(
        &project,
        &home,
        &["run", "interrupt", &run2.to_string(), "--json"],
    );
    assert!(
        out.status.success(),
        "run interrupt が失敗した: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).expect("run interrupt の JSON");
    assert_eq!(v["status"], "interrupted", "interrupt の結果: {}", v);
    assert_eq!(v["run_id"], run2);
    wait_for_state(&pane2.pane_id, "idle", Duration::from_secs(8));

    // ---- 無い実行は終了コード 3（stdout は空・stderr は日本語 1 行）----
    let out = run_cli(
        &project,
        &home,
        &["run", "send", "999999", "--text", "x", "--json"],
    );
    assert_eq!(
        out.status.code(),
        Some(3),
        "無い実行は 3: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(out.stdout.is_empty(), "stdout は空");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(
        stderr.lines().filter(|l| !l.trim().is_empty()).count(),
        1,
        "stderr は 1 行: {stderr:?}"
    );

    let out = run_cli(
        &project,
        &home,
        &["run", "interrupt", "999999", "--json"],
    );
    assert_eq!(out.status.code(), Some(3), "無い実行の interrupt は 3");

    // ---- 空の文は終了コード 2 ----
    let out = run_cli(
        &project,
        &home,
        &["run", "send", &run1.to_string(), "--text", "   ", "--json"],
    );
    assert_eq!(
        out.status.code(),
        Some(2),
        "空の文は 2: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(out.stdout.is_empty(), "stdout は空");

    // ---- 実行 3: フックが状態を知らせないときは unconfirmed（終了コード 0）----
    let fake_nh = project.join("fake_cc_no_hook.sh");
    write_script(&fake_nh, &fake_cc_no_hook());
    let run3 = make_execution(&project);
    let pane3 = headless::new_pane(Some(&project.to_string_lossy())).unwrap();
    let _kill3 = KillSession(pane3.pane_id.clone());
    headless::run(&pane3.pane_id, &format!("'{}'", fake_nh.display())).unwrap();
    record_pane(&project, run3, &pane3.pane_id);
    let out = run_cli(
        &project,
        &home,
        &["run", "send", &run3.to_string(), "--text", "未確認の依頼", "--json"],
    );
    assert!(
        out.status.success(),
        "unconfirmed は成功で返す: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).expect("run send の JSON");
    assert_eq!(v["status"], "unconfirmed", "結果: {}", v);
    assert_eq!(v["run_id"], run3);
    assert_eq!(v["pane_id"], pane3.pane_id);
    assert!(
        v["message"].as_str().is_some_and(|m| m.contains("before sending it again")),
        "message: {}",
        v
    );
    // 文は pane に届いている。
    wait_for_screen(&pane3.pane_id, "got: 未確認の依頼", Duration::from_secs(10));

    // ---- 後始末（tmux の session を止める）----
    let _ = headless::close(&pane3.pane_id);
    let _ = headless::close(&pane1.pane_id);
    let _ = headless::close(&pane2.pane_id);
}
