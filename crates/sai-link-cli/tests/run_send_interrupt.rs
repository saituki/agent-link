//! AC1: `sai-link run send` / `sai-link run interrupt` の結合テスト。
//!
//! 一時フォルダのプロジェクト・偽の cc エージェント・擬似 pane（headless）を使う。
//! 監督スレッドはこのテストのプロセスの中で動かし、CLI（別プロセス）が
//! `draft.txt`・`inbox/`・`interrupt` のファイルで依頼を渡す経路を確かめる。
//!
//! - 動いている実行に `run send` すると次のターンが走る（`stream.jsonl` に
//!   `sai_turn` の行が増える）。
//! - `run interrupt` で長く眠る偽のエージェントが止まる。
//! - 無い実行は終了コード 3。空の文は終了コード 2。
//!
//! 一時フォルダをプロジェクトルートと `SAI_HOME` にするので、本物の `.sai` や
//! `~/.sai` は書き換えない。

use sai_link_engine::cc_command::CcCommandBuilder;
use sai_link_engine::db::models::BlockMetadata;
use sai_link_engine::db::SaiStore;
use sai_link_engine::headless::{self, HeadlessState};
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{Duration, Instant};

/// `sai-link` のバイナリパス（Cargo が統合テスト用に提供する env）。
fn sai_binary() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_sai-link"))
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

/// 偽の cc エージェント（stream-json を 4 行出して終わる）。
const FAKE_CC: &str = r##"#!/bin/sh
cat >/dev/null
printf '%s\n' '{"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":"hel"}}}'
printf '%s\n' '{"type":"assistant","message":{}}'
printf '%s\n' '{"type":"result","session_id":"s1","total_cost_usd":0.01}'
"##;

/// 長く眠る偽の cc エージェント（中断されるまで止まらない）。
const SLEEPING_CC: &str = r##"#!/bin/sh
cat >/dev/null
sleep 60
"##;

/// 一時プロジェクトの DB に `running` の実行を 1 件作り、ID を返す。
fn make_execution(project: &Path, runner: &str) -> i64 {
    use std::sync::atomic::{AtomicU32, Ordering};
    static SEQ: AtomicU32 = AtomicU32::new(0);
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    let db_path = project.join(".sai/sai-engine/sai-engine.db");
    std::fs::create_dir_all(db_path.parent().unwrap()).unwrap();
    let store = SaiStore::open(&db_path).unwrap();
    let block = BlockMetadata::new(
        format!("send-test-{}", seq),
        "送信のテスト".to_string(),
        "anthropic/claude-sonnet-4-5".to_string(),
        None,
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
    store.update_execution_runner(run_id, runner).unwrap();
    run_id
}

/// 実行の pane を DB に記録する。
fn record_pane(project: &Path, run_id: i64, pane_id: &str) {
    let store = SaiStore::open(&project.join(".sai/sai-engine/sai-engine.db")).unwrap();
    store.update_execution_pane(run_id, pane_id, 0).unwrap();
}

/// 子プロセスとして `sai-link` を走らせる（プロジェクトと置き場を一時フォルダに固定）。
fn run_cli(project: &Path, home: &Path, args: &[&str]) -> Output {
    Command::new(sai_binary())
        .args(args)
        .env("SAI_PROJECT_ROOT", project)
        .env("SAI_HOME", home)
        .env_remove("SAI_PARENT_RUN_ID")
        .env_remove("SAI_BLOCK_NAME")
        .current_dir(project)
        .output()
        .expect("sai-link の実行に失敗しました")
}

#[test]
fn test_run_send_and_interrupt() {
    let base = tempfile::tempdir().unwrap();
    let project = base.path().join("proj");
    let home = base.path().join("home");
    std::fs::create_dir_all(project.join(".sai")).unwrap();
    std::fs::create_dir_all(&home).unwrap();
    // 監督スレッド（このプロセスの中）と DB が同じ置き場を見るようにする。
    std::env::set_var("SAI_PROJECT_ROOT", &project);
    std::env::set_var("SAI_HOME", &home);

    let fake = project.join("fake_cc.sh");
    write_script(&fake, FAKE_CC);
    std::env::set_var("SAI_CLAUDE_BIN", &fake);

    // ---- 実行 1: 動いている実行に run send すると次のターンが走る ----
    let run1 = make_execution(&project, "cc");
    let command = CcCommandBuilder::new("11111111-2222-4333-8444-555555555555")
        .build()
        .unwrap()
        .command;
    let pane1 = headless::new_pane(Some(&project.to_string_lossy())).unwrap();
    headless::run(&pane1.pane_id, &command).unwrap();
    // 実行中の状態を作るため、最初のターンを自分で始める。
    headless::send_text(&pane1.pane_id, "最初の依頼").unwrap();
    headless::send_keys(&pane1.pane_id, "Alt+Enter").unwrap();
    wait_for_state(&pane1.pane_id, "working", Duration::from_secs(5));
    wait_for_state(&pane1.pane_id, "idle", Duration::from_secs(10));
    record_pane(&project, run1, &pane1.pane_id);

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

    // 2 回目のターンが走り、sai_turn の行が増える。
    wait_for_state(&pane1.pane_id, "working", Duration::from_secs(5));
    wait_for_state(&pane1.pane_id, "idle", Duration::from_secs(10));
    let stream =
        std::fs::read_to_string(headless::pane_dir(&pane1.pane_id).join("stream.jsonl")).unwrap();
    let turns = stream
        .lines()
        .filter(|l| l.contains("\"type\":\"sai_turn\""))
        .count();
    assert_eq!(turns, 2, "sai_turn が 2 行になる: {}", stream);
    assert!(stream.contains("続きの依頼"), "2 回目の本文が入る: {}", stream);

    // ---- 実行 2: run interrupt で長く眠る偽のエージェントが止まる ----
    let sleeper = project.join("sleeping_cc.sh");
    write_script(&sleeper, SLEEPING_CC);
    std::env::set_var("SAI_CLAUDE_BIN", &sleeper);
    let run2 = make_execution(&project, "cc");
    let sleep_cmd = CcCommandBuilder::new("22222222-2222-4333-8444-555555555555")
        .build()
        .unwrap()
        .command;
    let pane2 = headless::new_pane(Some(&project.to_string_lossy())).unwrap();
    headless::run(&pane2.pane_id, &sleep_cmd).unwrap();
    headless::send_text(&pane2.pane_id, "眠る依頼").unwrap();
    headless::send_keys(&pane2.pane_id, "Alt+Enter").unwrap();
    wait_for_state(&pane2.pane_id, "working", Duration::from_secs(5));
    record_pane(&project, run2, &pane2.pane_id);

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

    // ---- 後始末（監督スレッドを止める）----
    let _ = headless::close(&pane1.pane_id);
    let _ = headless::close(&pane2.pane_id);
    std::thread::sleep(Duration::from_millis(500));
}
