//! # headless 輸送路（擬似 pane）
//!
//! sai-tree では herdr の pane を使わず、エージェント（cc / pi）を
//! 子プロセスとして動かす。このモジュールは、その子プロセスを
//! **擬似 pane** として扱うための置き場と操作を提供する。
//!
//! ## 置き場
//!
//! `<project_root>/.sai/sai-engine/headless/<id>/`
//!
//! - `command.txt` — 起動コマンド（2 回目以降のターンでは `--resume` / `--continue` 版に書き換わる）
//! - `cwd.txt` — 作業ディレクトリ
//! - `draft.txt` — 入力中のことば（`send_text` が上書き）
//! - `inbox/` — 確定した依頼（`send_keys` が `draft.txt` から rename して積む）
//! - `interrupt` — 中断の印（`send_keys("esc")` が作る）
//! - `stop` — 終了の印（`close` が作る）
//! - `stream.jsonl` — ターンの stdout（1 行ずつ追記）
//! - `stderr.log` — ターンの stderr
//! - `state.json` — 共有契約 C1 の状態（監督スレッドだけが書く）
//!
//! ## 監督スレッド
//!
//! 1 つの擬似 pane に 1 本。200ms ごとに `stop` / `interrupt` / `inbox/` を見て、
//! 依頼があれば `sh -c "<コマンド>"` を子プロセスとして起こし、stdout を
//! `stream.jsonl` へ、stderr を `stderr.log` へ流す。

use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use crate::herdr::{HerdrError, PaneInfo};
use crate::runner::RunnerKind;

/// 擬似 pane の id 接頭辞。
pub const HEADLESS_PREFIX: &str = "hl-";

/// 監督スレッドの巡回間隔（ミリ秒）。
const POLL_MS: u64 = 200;

/// `inbox/` の連番（同一ミリ秒に複数入っても衝突しないようにする）。
static NEXT_SEQ: AtomicU64 = AtomicU64::new(0);

/// 擬似 pane の id かどうか。
pub fn is_headless_pane(id: &str) -> bool {
    id.starts_with(HEADLESS_PREFIX)
}

/// 擬似 pane の置き場。
pub fn pane_dir(id: &str) -> PathBuf {
    crate::project::sai_path(".sai/sai-engine/headless").join(id)
}

/// `state.json` の形（共有契約 C1）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HeadlessState {
    /// 擬似 pane の id（`hl-` + 12 桁の 16 進）
    pub pane_id: String,
    /// 実行主体（`cc` / `pi`）
    pub runner: String,
    /// `working` / `idle` / `stopped`
    pub state: String,
    /// ターン番号
    pub turn: u32,
    /// 生成中の文（無ければ空）
    pub partial: String,
    /// 直前のターンが失敗したときの理由 1 行（無ければ空）
    pub error: String,
    /// 更新時刻（ISO 8601）
    pub updated_at: String,
}

/// 新しい擬似 pane を作る。
pub fn new_pane(cwd: Option<&str>) -> Result<PaneInfo, HerdrError> {
    let hex: String = crate::cc_command::generate_uuid_v4()
        .chars()
        .filter(|c| c.is_ascii_hexdigit())
        .take(12)
        .collect();
    let id = format!("{HEADLESS_PREFIX}{hex}");
    let dir = pane_dir(&id);
    std::fs::create_dir_all(dir.join("inbox"))?;
    if let Some(c) = cwd {
        std::fs::write(dir.join("cwd.txt"), c)?;
    }
    let state = HeadlessState {
        pane_id: id.clone(),
        runner: String::new(),
        state: "idle".to_string(),
        turn: 0,
        partial: String::new(),
        error: String::new(),
        updated_at: crate::phone_export::now_iso(),
    };
    write_state(&dir, &state)?;
    Ok(pane_info(&id, cwd, "idle"))
}

/// 起動コマンドを書き、監督スレッドを起こす。
pub fn run(id: &str, command: &str) -> Result<(), HerdrError> {
    let dir = pane_dir(id);
    std::fs::create_dir_all(dir.join("inbox"))?;
    std::fs::write(dir.join("command.txt"), command)?;
    let id_owned = id.to_string();
    std::thread::spawn(move || supervise(id_owned));
    Ok(())
}

/// 入力中のテキストを `draft.txt` に上書きする。
pub fn send_text(id: &str, text: &str) -> Result<(), HerdrError> {
    let dir = pane_dir(id);
    if !dir.is_dir() {
        return Err(pane_gone(id));
    }
    std::fs::write(dir.join("draft.txt"), text)?;
    Ok(())
}

/// キーを送る。
///
/// - `esc` / `Escape` / `ctrl+c` は中断の印 `interrupt` を作る。
/// - それ以外（`Alt+Enter` / `Enter` などの確定キー）は、`draft.txt` があれば
///   `inbox/<unix ミリ秒>-<連番>.txt` へ rename する。無ければ何もしない
///   （`submit_prompt_verified` の再送で同じ文が 2 回入らないため）。
pub fn send_keys(id: &str, keys: &str) -> Result<(), HerdrError> {
    let dir = pane_dir(id);
    if !dir.is_dir() {
        return Err(pane_gone(id));
    }
    let key = keys.trim();
    if key.eq_ignore_ascii_case("esc")
        || key.eq_ignore_ascii_case("escape")
        || key.eq_ignore_ascii_case("ctrl+c")
    {
        std::fs::write(dir.join("interrupt"), b"")?;
        return Ok(());
    }
    let draft = dir.join("draft.txt");
    if draft.is_file() {
        let inbox = dir.join("inbox");
        std::fs::create_dir_all(&inbox)?;
        let millis = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        let seq = NEXT_SEQ.fetch_add(1, Ordering::Relaxed);
        let name = format!("{:013}-{:06}.txt", millis, seq);
        std::fs::rename(&draft, inbox.join(name))?;
    }
    Ok(())
}

/// 状態を読み、`PaneInfo` を返す。
///
/// `stopped` と、置き場が無い・`state.json` が読めないときは
/// `Err`（pane が消えた扱い）。
pub fn status(id: &str) -> Result<PaneInfo, HerdrError> {
    let dir = pane_dir(id);
    if !dir.is_dir() {
        return Err(pane_gone(id));
    }
    let state = read_state(&dir).ok_or_else(|| pane_gone(id))?;
    if state.state == "stopped" {
        return Err(pane_gone(id));
    }
    let cwd = read_cwd(&dir);
    Ok(pane_info(id, cwd.as_deref(), &state.state))
}

/// `stream.jsonl` の末尾 `lines` 行を、1 行 300 文字までに切って返す。
pub fn read_output(id: &str, lines: u32) -> Result<String, HerdrError> {
    let path = pane_dir(id).join("stream.jsonl");
    let content = std::fs::read_to_string(&path).unwrap_or_default();
    let all: Vec<&str> = content.lines().collect();
    let n = lines.max(1) as usize;
    let start = all.len().saturating_sub(n);
    let out: Vec<String> = all[start..]
        .iter()
        .map(|l| l.chars().take(300).collect())
        .collect();
    Ok(out.join("\n"))
}

/// `stop` の印を作る（監督スレッドが動いているターンを止めて `stopped` にする）。
pub fn close(id: &str) -> Result<(), HerdrError> {
    let dir = pane_dir(id);
    if !dir.is_dir() {
        return Ok(());
    }
    std::fs::write(dir.join("stop"), b"")?;
    Ok(())
}

// ============================================================================
// 純関数（単体テスト対象）
// ============================================================================

/// 2 回目以降のターンのコマンドを作る。
///
/// - cc: ` --session-id ` を ` --resume ` に置き換える（同じ session id で続きになる）。
/// - pi: 末尾に ` --continue` を足す（`--session-dir` の中の直近のセッションを続ける）。
pub fn resume_command(cmd: &str, runner: RunnerKind) -> String {
    match runner {
        RunnerKind::Cc => cmd.replacen(" --session-id ", " --resume ", 1),
        RunnerKind::Pi => format!("{} --continue", cmd),
        RunnerKind::Gl => cmd.to_string(),
    }
}

/// 生成中の文（`partial`）を stdout の 1 行で更新する。
///
/// - cc: `type == "stream_event"` かつ `event.delta.type == "text_delta"` なら
///   `event.delta.text` を足す。`type == "assistant"` または `type == "result"` なら空にする。
/// - pi: `type == "message_update"` で `assistantMessageEvent.type == "text_delta"` なら
///   `assistantMessageEvent.delta` を足す。`type == "message_end"` または
///   `type == "agent_end"` なら空にする。
pub fn update_partial(partial: &mut String, line: &serde_json::Value, runner: RunnerKind) {
    let ty = line.get("type").and_then(|v| v.as_str()).unwrap_or("");
    match runner {
        RunnerKind::Cc => match ty {
            "stream_event" => {
                if let Some(delta) = line.pointer("/event/delta") {
                    if delta.get("type").and_then(|v| v.as_str()) == Some("text_delta") {
                        if let Some(text) = delta.get("text").and_then(|v| v.as_str()) {
                            partial.push_str(text);
                        }
                    }
                }
            }
            "assistant" | "result" => partial.clear(),
            _ => {}
        },
        RunnerKind::Pi => match ty {
            "message_update" => {
                if let Some(ev) = line.get("assistantMessageEvent") {
                    if ev.get("type").and_then(|v| v.as_str()) == Some("text_delta") {
                        if let Some(delta) = ev.get("delta").and_then(|v| v.as_str()) {
                            partial.push_str(delta);
                        }
                    }
                }
            }
            "message_end" | "agent_end" => partial.clear(),
            _ => {}
        },
        RunnerKind::Gl => {}
    }
}

/// cc の `result` 行から `(session_id, total_cost_usd)` を取り出す。
pub fn cc_cost_from_result(line: &serde_json::Value) -> Option<(String, f64)> {
    if line.get("type").and_then(|v| v.as_str()) != Some("result") {
        return None;
    }
    let session_id = line.get("session_id").and_then(|v| v.as_str())?;
    let total = line
        .get("total_cost_usd")
        .and_then(|v| v.as_f64())
        .filter(|v| v.is_finite() && *v >= 0.0)?;
    Some((session_id.to_string(), total))
}

/// cc の `result` 行を指定フォルダへ書く（テスト用）。書いたら `true`。
pub fn handle_cc_result_in(line: &serde_json::Value, dir: &Path) -> bool {
    match cc_cost_from_result(line) {
        Some((session_id, total)) => {
            crate::phone_export::write_cc_cost(dir, &session_id, total).is_ok()
        }
        None => false,
    }
}

// ============================================================================
// 内部: 監督スレッド
// ============================================================================

/// 動いているターン 1 本。
struct RunningTurn {
    child: Child,
    stdout_rx: Receiver<String>,
    last_stderr: std::sync::Arc<std::sync::Mutex<String>>,
}

/// 1 つの擬似 pane を監督する（`run` からスレッドで起こされる）。
fn supervise(id: String) {
    let dir = pane_dir(&id);
    let base_command = match std::fs::read_to_string(dir.join("command.txt")) {
        Ok(s) => s.trim_end_matches('\n').to_string(),
        Err(_) => return,
    };
    let runner = infer_runner(&base_command);
    let cwd = read_cwd(&dir);
    let mut state = read_state(&dir).unwrap_or_else(|| HeadlessState {
        pane_id: id.clone(),
        runner: String::new(),
        state: "idle".to_string(),
        turn: 0,
        partial: String::new(),
        error: String::new(),
        updated_at: crate::phone_export::now_iso(),
    });
    state.runner = runner.as_str().to_string();

    let mut turn = state.turn;
    let mut partial = String::new();
    let mut running: Option<RunningTurn> = None;
    let mut last_key = String::new();

    loop {
        std::thread::sleep(Duration::from_millis(POLL_MS));

        // 1. stop: 動いているターンを止め、stopped にして終わる。
        if dir.join("stop").exists() {
            if let Some(mut t) = running.take() {
                kill_process_group(&mut t.child);
                drain_stdout(&dir, &mut t.stdout_rx, runner, &mut partial);
            }
            state.state = "stopped".to_string();
            state.partial = String::new();
            state.turn = turn;
            state.error = String::new();
            write_state_if_changed(&dir, &state, &mut last_key);
            return;
        }

        // 2. interrupt: 印を消し、動いているターンのプロセスを止める。
        if dir.join("interrupt").exists() {
            let _ = std::fs::remove_file(dir.join("interrupt"));
            if let Some(mut t) = running.take() {
                kill_process_group(&mut t.child);
                drain_stdout(&dir, &mut t.stdout_rx, runner, &mut partial);
            }
            partial.clear();
            state.state = "idle".to_string();
        }

        // 3. ターンが動いていなければ、inbox の一番古い依頼でターンを始める。
        if running.is_none() {
            if let Some(message) = take_oldest_inbox(&dir) {
                turn += 1;
                let command = if turn > 1 {
                    resume_command(&base_command, runner)
                } else {
                    base_command.clone()
                };
                let _ = std::fs::write(dir.join("command.txt"), &command);
                let _ = append_json_line(
                    &dir,
                    &serde_json::json!({
                        "type": "sai_turn",
                        "turn": turn,
                        "ts": crate::phone_export::now_iso(),
                        "text": message,
                    }),
                );
                match start_turn(&dir, &command, &message, cwd.as_deref(), &id) {
                    Ok(t) => {
                        running = Some(t);
                        state.state = "working".to_string();
                        state.error = String::new();
                        partial.clear();
                    }
                    Err(e) => {
                        state.state = "idle".to_string();
                        state.error = format!("ターンの起動に失敗: {}", e);
                    }
                }
            }
        }

        // 4. 出力を読み、プロセスが終わったらターンを閉じる。
        if let Some(t) = running.as_mut() {
            drain_stdout(&dir, &mut t.stdout_rx, runner, &mut partial);
            match t.child.try_wait() {
                Ok(Some(status)) => {
                    drain_stdout(&dir, &mut t.stdout_rx, runner, &mut partial);
                    let last_err = t
                        .last_stderr
                        .lock()
                        .map(|g| g.clone())
                        .unwrap_or_default();
                    if !status.success() && !last_err.is_empty() {
                        state.error = last_err;
                    } else {
                        state.error = String::new();
                    }
                    state.state = "idle".to_string();
                    partial.clear();
                    running = None;
                }
                Ok(None) => {}
                Err(_) => {
                    state.state = "idle".to_string();
                    partial.clear();
                    running = None;
                }
            }
        }

        state.turn = turn;
        state.partial = partial.clone();
        write_state_if_changed(&dir, &state, &mut last_key);
    }
}

/// 起動コマンドから実行主体を推定する。
fn infer_runner(command: &str) -> RunnerKind {
    if command.contains("--session-dir") {
        RunnerKind::Pi
    } else if command.contains("--session-id")
        || command.contains("--output-format stream-json")
    {
        RunnerKind::Cc
    } else {
        RunnerKind::Pi
    }
}

/// 1 ターン分のプロセスを起こし、stdout / stderr の読み取りスレッドを付ける。
fn start_turn(
    dir: &Path,
    command: &str,
    message: &str,
    cwd: Option<&str>,
    id: &str,
) -> std::io::Result<RunningTurn> {
    let mut cmd = Command::new("sh");
    cmd.arg("-c").arg(command);
    if let Some(c) = cwd {
        cmd.current_dir(c);
    }
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    cmd.env("HERDR_PANE_ID", id);
    cmd.env("SAI_AGENT_TRANSPORT", "headless");
    // プロセスグループを分け、中断時にグループごと止められるようにする。
    cmd.process_group(0);
    let mut child = cmd.spawn()?;

    // 依頼文を stdin に書いて閉じる。
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(message.as_bytes());
    }

    let stdout = child.stdout.take().expect("stdout パイプ");
    let stderr = child.stderr.take().expect("stderr パイプ");

    let (tx, rx) = mpsc::channel::<String>();
    std::thread::spawn(move || {
        let reader = BufReader::new(stdout);
        for line in reader.lines() {
            match line {
                Ok(l) => {
                    if tx.send(l).is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });

    let last_stderr = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
    let last_clone = std::sync::Arc::clone(&last_stderr);
    let stderr_log = dir.join("stderr.log");
    std::thread::spawn(move || {
        let reader = BufReader::new(stderr);
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&stderr_log)
            .ok();
        for line in reader.lines() {
            match line {
                Ok(l) => {
                    if let Some(f) = file.as_mut() {
                        let _ = writeln!(f, "{}", l);
                    }
                    let trimmed = l.trim();
                    if !trimmed.is_empty() {
                        if let Ok(mut g) = last_clone.lock() {
                            *g = trimmed.to_string();
                        }
                    }
                }
                Err(_) => break,
            }
        }
    });

    Ok(RunningTurn {
        child,
        stdout_rx: rx,
        last_stderr,
    })
}

/// stdout の溜まった行を `stream.jsonl` に追記し、partial と cc の費用を更新する。
fn drain_stdout(dir: &Path, rx: &mut Receiver<String>, runner: RunnerKind, partial: &mut String) {
    while let Ok(line) = rx.try_recv() {
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(dir.join("stream.jsonl"))
        {
            let _ = writeln!(f, "{}", line);
        }
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(&line) {
            update_partial(partial, &value, runner);
            if matches!(runner, RunnerKind::Cc) {
                if let Some((session_id, total)) = cc_cost_from_result(&value) {
                    if let Some(cost_dir) = crate::phone_export::cc_cost_dir() {
                        let _ = crate::phone_export::write_cc_cost(&cost_dir, &session_id, total);
                    }
                }
            }
        }
    }
}

/// `inbox/` の一番古いファイルを読み、消して中身を返す。
fn take_oldest_inbox(dir: &Path) -> Option<String> {
    let inbox = dir.join("inbox");
    let mut entries: Vec<PathBuf> = std::fs::read_dir(&inbox)
        .ok()?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().map(|e| e == "txt").unwrap_or(false))
        .collect();
    entries.sort();
    let path = entries.into_iter().next()?;
    let text = std::fs::read_to_string(&path).ok()?;
    let _ = std::fs::remove_file(&path);
    Some(text)
}

/// プロセスグループへ SIGTERM を送り、2 秒たっても終わらなければ SIGKILL。
fn kill_process_group(child: &mut Child) {
    let pid = child.id() as i32;
    if pid > 0 {
        // 負の pid はプロセスグループ全体を指す。
        unsafe {
            libc::kill(-pid, libc::SIGTERM);
        }
    }
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        match child.try_wait() {
            Ok(Some(_)) => return,
            Ok(None) => {
                if Instant::now() >= deadline {
                    break;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(_) => break,
        }
    }
    if pid > 0 {
        unsafe {
            libc::kill(-pid, libc::SIGKILL);
        }
    }
    let _ = child.wait();
}

// ============================================================================
// 内部: ファイル操作
// ============================================================================

/// `state.json` を読む。
fn read_state(dir: &Path) -> Option<HeadlessState> {
    let text = std::fs::read_to_string(dir.join("state.json")).ok()?;
    serde_json::from_str(&text).ok()
}

/// `cwd.txt` を読む。
fn read_cwd(dir: &Path) -> Option<String> {
    std::fs::read_to_string(dir.join("cwd.txt"))
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// `state.json` を一時ファイル → rename で書く。
fn write_state(dir: &Path, state: &HeadlessState) -> std::io::Result<()> {
    let text = serde_json::to_string_pretty(state)
        .map_err(|e| std::io::Error::other(format!("JSON 直列化失敗: {}", e)))?;
    let tmp = dir.join(".state.json.tmp");
    let dst = dir.join("state.json");
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, &dst)?;
    Ok(())
}

/// 中身が変化したときだけ `state.json` を書く。
fn write_state_if_changed(dir: &Path, state: &HeadlessState, last_key: &mut String) {
    let key = format!(
        "{}|{}|{}|{}|{}",
        state.runner, state.state, state.turn, state.partial, state.error
    );
    if *last_key == key {
        return;
    }
    let mut to_write = state.clone();
    to_write.updated_at = crate::phone_export::now_iso();
    if write_state(dir, &to_write).is_ok() {
        *last_key = key;
    }
}

/// JSON 1 行を `stream.jsonl` に追記する。
fn append_json_line(dir: &Path, value: &serde_json::Value) -> std::io::Result<()> {
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join("stream.jsonl"))?;
    writeln!(f, "{}", value)
}

/// `PaneInfo` を組み立てる。
fn pane_info(id: &str, cwd: Option<&str>, status: &str) -> PaneInfo {
    PaneInfo {
        pane_id: id.to_string(),
        workspace_id: "headless".to_string(),
        tab_id: "headless".to_string(),
        focused: false,
        cwd: cwd.map(|s| s.to_string()),
        agent: None,
        agent_status: status.to_string(),
        label: None,
        revision: 0,
    }
}

/// pane が消えたときのエラー。
fn pane_gone(id: &str) -> HerdrError {
    HerdrError::ApiError {
        message: format!("pane が見つかりません: {}", id),
    }
}

// ============================================================================
// テスト
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resume_command_cc_replaces_session_id() {
        let cmd = "claude -p --session-id '11111111-2222-4333-8444-555555555555' --model 'sonnet'";
        assert_eq!(
            resume_command(cmd, RunnerKind::Cc),
            "claude -p --resume '11111111-2222-4333-8444-555555555555' --model 'sonnet'"
        );
    }

    #[test]
    fn resume_command_pi_appends_continue() {
        let cmd = "pi --session-dir '/tmp/sess' --mode json";
        assert_eq!(
            resume_command(cmd, RunnerKind::Pi),
            "pi --session-dir '/tmp/sess' --mode json --continue"
        );
    }

    #[test]
    fn update_partial_cc_text_delta_and_reset() {
        let mut partial = String::new();
        let delta = serde_json::json!({
            "type": "stream_event",
            "event": {"type": "content_block_delta", "delta": {"type": "text_delta", "text": "hel"}}
        });
        update_partial(&mut partial, &delta, RunnerKind::Cc);
        update_partial(&mut partial, &delta, RunnerKind::Cc);
        assert_eq!(partial, "helhel");
        let assistant = serde_json::json!({"type": "assistant"});
        update_partial(&mut partial, &assistant, RunnerKind::Cc);
        assert!(partial.is_empty());
        // result でも空になる
        partial.push_str("x");
        let result = serde_json::json!({"type": "result", "total_cost_usd": 0.01});
        update_partial(&mut partial, &result, RunnerKind::Cc);
        assert!(partial.is_empty());
    }

    #[test]
    fn update_partial_pi_text_delta_and_reset() {
        let mut partial = String::new();
        let delta = serde_json::json!({
            "type": "message_update",
            "assistantMessageEvent": {"type": "text_delta", "delta": "wor"}
        });
        update_partial(&mut partial, &delta, RunnerKind::Pi);
        assert_eq!(partial, "wor");
        let end = serde_json::json!({"type": "message_end"});
        update_partial(&mut partial, &end, RunnerKind::Pi);
        assert!(partial.is_empty());
        partial.push_str("y");
        let agent_end = serde_json::json!({"type": "agent_end"});
        update_partial(&mut partial, &agent_end, RunnerKind::Pi);
        assert!(partial.is_empty());
    }

    #[test]
    fn cc_cost_from_result_reads_fields() {
        let line = serde_json::json!({
            "type": "result",
            "session_id": "s1",
            "total_cost_usd": 0.0123
        });
        assert_eq!(
            cc_cost_from_result(&line),
            Some(("s1".to_string(), 0.0123))
        );
        let assistant = serde_json::json!({"type": "assistant"});
        assert_eq!(cc_cost_from_result(&assistant), None);
    }

    #[test]
    fn handle_cc_result_in_writes_cost_file() {
        let dir = tempfile::tempdir().unwrap();
        let line = serde_json::json!({
            "type": "result",
            "session_id": "s1",
            "total_cost_usd": 0.01
        });
        assert!(handle_cc_result_in(&line, dir.path()));
        let text = std::fs::read_to_string(dir.path().join("s1.json")).unwrap();
        let value: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(value["session_id"], "s1");
        assert_eq!(value["total_cost_usd"], 0.01);
        assert!(value["updated_at"].is_string());
    }

    #[test]
    fn is_headless_pane_prefix() {
        assert!(is_headless_pane("hl-0123456789ab"));
        assert!(!is_headless_pane("pane_test_001"));
    }

    #[test]
    fn infer_runner_from_command() {
        assert_eq!(
            infer_runner("pi --session-dir '/tmp/s'"),
            RunnerKind::Pi
        );
        assert_eq!(
            infer_runner("claude -p --output-format stream-json --session-id 'x'"),
            RunnerKind::Cc
        );
    }
}
