//! AC3: 擬似 pane のターン輸送の結合テスト（pi の子プロセス経路）。
//!
//! 偽の pi エージェント（sh スクリプト）を使い、
//! `headless::run` → `send_text` → `send_keys("Alt+Enter")` でターンが走ること、
//! 2 回目の送信で `command.txt` に `--continue` が付くこと、`send_keys("esc")` で
//! 長く眠るエージェントが止まること、`close` で `stopped` になることを確かめる。
//! cc は tmux 経路（`tests/tmux_turns.rs`）で確かめる。
//!
//! 一時フォルダをプロジェクトルートと `SAI_HOME` にするので、本物の `.sai` や
//! `~/.sai` は書き換えない。

use sai_link_engine::headless::{self, HeadlessState};
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::time::{Duration, Instant};

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

/// 偽の pi エージェント（pi の JSON 行を出して終わる）。
const FAKE_PI: &str = r##"#!/bin/sh
cat >/dev/null
printf '%s\n' '{"type":"message_update","assistantMessageEvent":{"type":"text_delta","delta":"hel"}}'
printf '%s\n' '{"type":"message_update","assistantMessageEvent":{"type":"text_delta","delta":"lo"}}'
printf '%s\n' '{"type":"message_end"}'
printf '%s\n' '{"type":"agent_end"}'
"##;

/// 長く眠る偽の pi エージェント（中断されるまで止まらない）。
const SLEEPING_PI: &str = r##"#!/bin/sh
cat >/dev/null
sleep 60
"##;

#[test]
fn test_headless_turn_transport() {
    let project = tempfile::tempdir().unwrap();
    let sai_home = tempfile::tempdir().unwrap();
    // 本物のプロジェクト・置き場を触らないように隔離する。
    std::env::set_var("SAI_PROJECT_ROOT", project.path());
    std::env::set_var("SAI_HOME", sai_home.path());
    let sessions = project.path().join("pi-sessions");
    let session_arg = format!("--session-dir '{}' --mode json", sessions.display());

    let fake = project.path().join("fake_pi.sh");
    write_script(&fake, FAKE_PI);
    let command = format!("'{}' {}", fake.display(), session_arg);

    // ---- 1 回目のターン ----
    let pane = headless::new_pane(Some(&project.path().to_string_lossy())).unwrap();
    headless::run(&pane.pane_id, &command).unwrap();
    headless::send_text(&pane.pane_id, "1 回目の依頼").unwrap();
    headless::send_keys(&pane.pane_id, "Alt+Enter").unwrap();

    // working になったあと idle に戻る。
    wait_for_state(&pane.pane_id, "working", Duration::from_secs(5));
    let state = wait_for_state(&pane.pane_id, "idle", Duration::from_secs(10));
    assert_eq!(state.runner, "pi", "runner が pi になる");
    assert_eq!(state.turn, 1, "ターン番号が 1");
    assert_eq!(state.error, "", "エラーなし");

    // stream.jsonl に sai_turn とエージェントの行が入る。
    let stream =
        std::fs::read_to_string(headless::pane_dir(&pane.pane_id).join("stream.jsonl")).unwrap();
    assert!(stream.contains("\"type\":\"sai_turn\""), "{}", stream);
    assert!(stream.contains("1 回目の依頼"), "{}", stream);
    assert!(stream.contains("\"text_delta\""), "{}", stream);
    assert!(stream.contains("\"type\":\"agent_end\""), "{}", stream);

    // ---- 2 回目のターン: command.txt が --continue になる ----
    headless::send_text(&pane.pane_id, "2 回目の依頼").unwrap();
    headless::send_keys(&pane.pane_id, "Alt+Enter").unwrap();
    wait_for_state(&pane.pane_id, "working", Duration::from_secs(5));
    let state = wait_for_state(&pane.pane_id, "idle", Duration::from_secs(10));
    assert_eq!(state.turn, 2, "ターン番号が 2");
    let cmd2 =
        std::fs::read_to_string(headless::pane_dir(&pane.pane_id).join("command.txt")).unwrap();
    assert!(cmd2.ends_with("--continue"), "2 回目は --continue: {}", cmd2);

    // ---- 中断: 長く眠る偽のエージェントを esc で止める ----
    let sleeper = project.path().join("sleeping_pi.sh");
    write_script(&sleeper, SLEEPING_PI);
    let sleep_cmd = format!("'{}' {}", sleeper.display(), session_arg);
    let pane2 = headless::new_pane(Some(&project.path().to_string_lossy())).unwrap();
    headless::run(&pane2.pane_id, &sleep_cmd).unwrap();
    headless::send_text(&pane2.pane_id, "眠る依頼").unwrap();
    headless::send_keys(&pane2.pane_id, "Alt+Enter").unwrap();
    wait_for_state(&pane2.pane_id, "working", Duration::from_secs(5));

    headless::send_keys(&pane2.pane_id, "esc").unwrap();
    let state = wait_for_state(&pane2.pane_id, "idle", Duration::from_secs(8));
    assert_eq!(state.state, "idle", "esc でターンが終わる");

    // ---- close で stopped になる ----
    headless::close(&pane2.pane_id).unwrap();
    wait_for_state(&pane2.pane_id, "stopped", Duration::from_secs(5));

    // 後始末（監督スレッドを止める）。
    let _ = headless::close(&pane.pane_id);
    let _ = std::fs::remove_dir_all(project.path().join(".sai/sai-engine/headless"));
}
