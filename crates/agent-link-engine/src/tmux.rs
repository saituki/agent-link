//! # cc の対話モード（tmux 経路）
//!
//! cc は `claude -p` の子プロセスではなく、tmux の session の中で動く対話モードの `claude` として走る。
//! 擬似 pane（[`crate::headless`] の置き場）に空の印 `tmux` があれば、その pane の操作はすべてここへ回る。
//!
//! - session 名 = pane id（`hl-…`）。環境変数 `HERDR_PANE_ID`・`AGENT_LINK_AGENT_TRANSPORT`・`AGENT_LINK_PROJECT_ROOT` を入れる。
//! - 状態（`working` / `idle`）は cc のフックが `state.json` に書く（[`set_state_from_hook`]）。
//! - 承認は PermissionRequest フックが常に allow を返す（[`crate::cc_command::permission_hook_output`]）。
//! - 起動の前に `~/.claude.json` を整え、初回の画面で止まらないようにする（cc の設定は `--settings` で渡す）。

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::mpsc;
use std::time::Duration;

use serde_json::{json, Value};

use crate::headless::{self, HeadlessState};
use crate::herdr::{HerdrError, PaneInfo};

/// 印のファイル名（置き場の中の空のファイル。共有契約 C-T1）。
const MARK: &str = "tmux";

/// tmux 1 回の命令の上限。
const TMUX_TIMEOUT: Duration = Duration::from_secs(10);

/// 擬似 pane が tmux で動くかどうか（置き場に印があるか）。
pub fn is_tmux(id: &str) -> bool {
    headless::pane_dir(id).join(MARK).is_file()
}

/// cc を tmux の session で起こす（対話モードの `command` を 1 つの引数で渡す）。
pub fn run(id: &str, command: &str) -> Result<(), HerdrError> {
    let dir = headless::pane_dir(id);
    std::fs::create_dir_all(dir.join("inbox"))?;
    std::fs::write(dir.join(MARK), b"")?;
    std::fs::write(dir.join("command.txt"), command)?;
    headless::write_state(
        &dir,
        &HeadlessState {
            pane_id: id.to_string(),
            runner: "cc".to_string(),
            state: "starting".to_string(),
            turn: 0,
            partial: String::new(),
            error: String::new(),
            updated_at: crate::phone_export::now_iso(),
        },
    )?;

    let root = crate::project::project_root();
    let root_s = root.to_string_lossy().to_string();
    let cwd = headless::read_cwd(&dir).unwrap_or_else(|| root_s.clone());
    // cc は実パス（`/var` → `/private/var` など）で trust を引くので、実パスも足す。
    let mut dirs = vec![cwd.clone(), root_s.clone()];
    for d in [&cwd, &root_s] {
        if let Ok(real) = std::fs::canonicalize(d) {
            let real = real.to_string_lossy().to_string();
            if !dirs.contains(&real) {
                dirs.push(real);
            }
        }
    }
    let dirs: Vec<&str> = dirs.iter().map(String::as_str).collect();
    seed_home_claude_json(&dirs)?;

    let env_pane = format!("HERDR_PANE_ID={}", id);
    let env_root = format!("AGENT_LINK_PROJECT_ROOT={}", root_s);
    // フックが同じ Web に届き、cc の中の CLI が同じ置き場を使うよう、tmux サーバの環境に任せず渡す。
    let env_web = format!("AGENT_LINK_WEB_URL={}", crate::cc_command::web_url());
    let env_home = std::env::var("AGENT_LINK_HOME")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .map(|v| format!("AGENT_LINK_HOME={}", v));
    let mut args = vec![
        "new-session",
        "-d",
        "-s",
        id,
        "-x",
        "200",
        "-y",
        "50",
        "-c",
        cwd.as_str(),
        "-e",
        env_pane.as_str(),
        "-e",
        "AGENT_LINK_AGENT_TRANSPORT=tmux",
        "-e",
        env_root.as_str(),
        "-e",
        env_web.as_str(),
    ];
    if let Some(home) = env_home.as_deref() {
        args.extend(["-e", home]);
    }
    args.push(command);
    let out = tmux(&args)?;
    check(&out, id, "start")
}

/// 入力中の文を `draft.txt` に上書きする。
pub fn send_text(id: &str, text: &str) -> Result<(), HerdrError> {
    let dir = headless::pane_dir(id);
    if !dir.is_dir() {
        return Err(headless::pane_gone(id));
    }
    std::fs::write(dir.join("draft.txt"), text)?;
    Ok(())
}

/// キーを送る。
///
/// - `esc` / `escape` / `ctrl+c` は `Escape` を送り、Stop フックが呼ばれない中断を補うため `idle` にする。
/// - それ以外は、`draft.txt` があれば bracketed paste で貼ってから確定キー（`Alt+Enter` は `M-Enter`、他は Enter）を送る。
///   無ければ確定キーだけを送る（`submit_prompt_verified` の再送で同じ文が 2 回入らないため）。
pub fn send_keys(id: &str, keys: &str) -> Result<(), HerdrError> {
    let dir = headless::pane_dir(id);
    if !dir.is_dir() {
        return Err(headless::pane_gone(id));
    }
    let key = keys.trim();
    if key.eq_ignore_ascii_case("esc")
        || key.eq_ignore_ascii_case("escape")
        || key.eq_ignore_ascii_case("ctrl+c")
    {
        check(&tmux(&["send-keys", "-t", id, "Escape"])?, id, "interrupt")?;
        set_state(&dir, "idle")?;
        return Ok(());
    }
    let draft = dir.join("draft.txt");
    if draft.is_file() {
        let draft_s = draft.to_string_lossy().to_string();
        check(&tmux(&["load-buffer", "-b", id, draft_s.as_str()])?, id, "paste")?;
        check(&tmux(&["paste-buffer", "-p", "-d", "-b", id, "-t", id])?, id, "paste")?;
        std::fs::remove_file(&draft)?;
        // 貼った文を入力欄が受け取るまで待つ（改行の前に確定させない）
        std::thread::sleep(Duration::from_millis(300));
    }
    // cc 2.1.301 は貼った複数行の文を素の Enter では確定しない。`Alt+Enter` は `M-Enter` で送る。
    let enter = if key.eq_ignore_ascii_case("alt+enter") { "M-Enter" } else { "Enter" };
    check(&tmux(&["send-keys", "-t", id, enter])?, id, "confirm")
}

/// 状態を読み、`PaneInfo` を返す。
///
/// `stopped` と、session が無い・`state.json` が読めないときは `Err`（pane が消えた扱い）。
/// `starting` は `unknown` にする（起動中に文を送らないため）。
pub fn status(id: &str) -> Result<PaneInfo, HerdrError> {
    let dir = headless::pane_dir(id);
    let state = headless::read_state(&dir).ok_or_else(|| headless::pane_gone(id))?;
    if state.state == "stopped" || !has_session(id) {
        return Err(headless::pane_gone(id));
    }
    let agent_status = match state.state.as_str() {
        "working" => "working",
        "idle" => "idle",
        _ => "unknown",
    };
    let cwd = headless::read_cwd(&dir);
    Ok(headless::pane_info(id, cwd.as_deref(), agent_status))
}

/// 画面の末尾 `lines` 行を返す（`capture-pane`）。
pub fn read_output(id: &str, lines: u32) -> Result<String, HerdrError> {
    let start = format!("-{}", lines.max(1));
    let out = tmux(&["capture-pane", "-p", "-t", id, "-S", start.as_str()])?;
    check(&out, id, "read screen")?;
    Ok(String::from_utf8_lossy(&out.stdout).trim_end().to_string())
}

/// session を閉じ、状態を `stopped` にする（session が無くても失敗にしない）。
pub fn close(id: &str) -> Result<(), HerdrError> {
    let dir = headless::pane_dir(id);
    if !dir.is_dir() {
        return Ok(());
    }
    let _ = tmux(&["kill-session", "-t", id]);
    set_state(&dir, "stopped")?;
    Ok(())
}

/// cc のフック（`agent-link cc-state-hook` / `cc-prompt-hook`）から状態を書く。
///
/// 環境変数 `HERDR_PANE_ID` の pane が tmux 経路のときだけ書く。それ以外・失敗は何もしない
/// （フックは cc を止めない）。
pub fn set_state_from_hook(state: &str) {
    if !matches!(state, "working" | "idle") {
        return;
    }
    let Ok(id) = std::env::var("HERDR_PANE_ID") else {
        return;
    };
    if !headless::is_headless_pane(&id) || !is_tmux(&id) {
        return;
    }
    let _ = set_state(&headless::pane_dir(&id), state);
}

// ============================================================================
// 純関数（単体テスト対象）
// ============================================================================

/// `~/.claude.json` に、初回の画面を出さないための項目を入れる（他の項目は残す）。
///
/// - `hasCompletedOnboarding` を `true` にする（テーマの選択を出さない）。
/// - `dirs` の各パスの `projects[<パス>].hasTrustDialogAccepted` を `true` にする（フォルダの信頼を出さない）。
/// - `api_key` があれば末尾 20 文字を `customApiKeyResponses.approved` に 1 度だけ足す。
pub fn seed_claude_json(mut value: Value, dirs: &[&str], api_key: Option<&str>) -> Value {
    if !value.is_object() {
        value = json!({});
    }
    value["hasCompletedOnboarding"] = json!(true);
    if !value["projects"].is_object() {
        value["projects"] = json!({});
    }
    for dir in dirs {
        let project = &mut value["projects"][*dir];
        if !project.is_object() {
            *project = json!({});
        }
        project["hasTrustDialogAccepted"] = json!(true);
    }
    if let Some(key) = api_key.filter(|k| !k.trim().is_empty()) {
        let chars: Vec<char> = key.chars().collect();
        let tail: String = chars[chars.len().saturating_sub(20)..].iter().collect();
        if !value["customApiKeyResponses"].is_object() {
            value["customApiKeyResponses"] = json!({});
        }
        let approved = &mut value["customApiKeyResponses"]["approved"];
        if !approved.is_array() {
            *approved = json!([]);
        }
        if let Some(list) = approved.as_array_mut() {
            if !list.iter().any(|v| v.as_str() == Some(tail.as_str())) {
                list.push(json!(tail));
            }
        }
    }
    value
}

// ============================================================================
// 内部
// ============================================================================

/// tmux の命令を実行する（引数を分けて渡す。シェルを通さない）。上限は [`TMUX_TIMEOUT`]。
fn tmux(args: &[&str]) -> Result<Output, HerdrError> {
    let child = Command::new("tmux")
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                api_error("tmux not found. It is required for the cc interactive mode.".to_string())
            } else {
                api_error(format!("Failed to start tmux: {}", e))
            }
        })?;
    // 出力を読みながら待つ（パイプが詰まって止まるのを防ぐ）。
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(child.wait_with_output());
    });
    match rx.recv_timeout(TMUX_TIMEOUT) {
        Ok(Ok(out)) => Ok(out),
        Ok(Err(e)) => Err(e.into()),
        Err(_) => Err(api_error(format!(
            "A tmux command did not finish within {} seconds.",
            TMUX_TIMEOUT.as_secs()
        ))),
    }
}

/// tmux の命令が失敗したら、標準エラーの最後の行を理由にして返す。
fn check(out: &Output, id: &str, what: &str) -> Result<(), HerdrError> {
    if out.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&out.stderr);
    let last = stderr
        .lines()
        .rev()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("")
        .trim();
    Err(api_error(format!("tmux {} failed (pane={}): {}", what, id, last)))
}

/// session が生きているか。
fn has_session(id: &str) -> bool {
    tmux(&["has-session", "-t", id])
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// 状態を書き換える。`stopped` の pane は `stopped` 以外に戻さない（終わった pane を生き返らせない）。
fn set_state(dir: &Path, value: &str) -> std::io::Result<()> {
    let Some(mut state) = headless::read_state(dir) else {
        return Ok(());
    };
    if state.state == "stopped" && value != "stopped" {
        return Ok(());
    }
    state.state = value.to_string();
    if value != "working" {
        state.partial.clear();
    }
    state.updated_at = crate::phone_export::now_iso();
    headless::write_state(dir, &state)
}

/// `$HOME/.claude.json` を [`seed_claude_json`] で整える。読めない JSON は触らない。
fn seed_home_claude_json(dirs: &[&str]) -> std::io::Result<()> {
    let Some(home) = std::env::var_os("HOME") else {
        return Ok(());
    };
    let path = PathBuf::from(home).join(".claude.json");
    let current = match std::fs::read_to_string(&path) {
        Ok(text) => match serde_json::from_str::<Value>(&text) {
            Ok(v) => v,
            Err(_) => return Ok(()),
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => json!({}),
        Err(e) => return Err(e),
    };
    let key = std::env::var("ANTHROPIC_API_KEY").ok();
    let seeded = seed_claude_json(current, dirs, key.as_deref());
    let text = serde_json::to_string_pretty(&seeded)
        .map_err(|e| std::io::Error::other(format!("JSON serialization failed: {}", e)))?;
    let tmp = path.with_file_name(".claude.json.agent-link.tmp");
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, &path)
}

fn api_error(message: String) -> HerdrError {
    HerdrError::ApiError { message }
}

// ============================================================================
// テスト
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seed_claude_json_sets_items_and_keeps_others() {
        let current = json!({"numStartups": 3, "projects": {"/x": {"allowedTools": ["Bash"]}}});
        let v = seed_claude_json(
            current,
            &["/w", "/root"],
            Some("sk-abcdefghijklmnopqrstuvwxyz"),
        );
        assert_eq!(v["hasCompletedOnboarding"], true);
        assert_eq!(v["projects"]["/w"]["hasTrustDialogAccepted"], true);
        assert_eq!(v["projects"]["/root"]["hasTrustDialogAccepted"], true);
        assert_eq!(v["projects"]["/x"]["allowedTools"], json!(["Bash"]));
        assert_eq!(v["numStartups"], 3);
        // 元のパスと実パスの両方に trust が入る
        let v = seed_claude_json(v, &["/var/x", "/private/var/x"], None);
        assert_eq!(v["projects"]["/var/x"]["hasTrustDialogAccepted"], true);
        assert_eq!(v["projects"]["/private/var/x"]["hasTrustDialogAccepted"], true);
        // 末尾 20 文字だけ（"ghijklmnopqrstuvwxyz" は 20 文字）
        assert_eq!(
            v["customApiKeyResponses"]["approved"],
            json!(["ghijklmnopqrstuvwxyz"])
        );
    }

    #[test]
    fn seed_claude_json_adds_api_key_tail_once() {
        let key = Some("sk-abcdefghijklmnopqrstuvwxyz");
        let v = seed_claude_json(json!({}), &[], key);
        let v = seed_claude_json(v, &[], key);
        assert_eq!(
            v["customApiKeyResponses"]["approved"].as_array().unwrap().len(),
            1
        );
    }

    #[test]
    fn set_state_does_not_revive_stopped_pane() {
        let dir = tempfile::tempdir().unwrap();
        let state = |s: &str| HeadlessState {
            pane_id: "hl-000000000000".to_string(),
            runner: "cc".to_string(),
            state: s.to_string(),
            turn: 0,
            partial: String::new(),
            error: String::new(),
            updated_at: String::new(),
        };
        headless::write_state(dir.path(), &state("starting")).unwrap();
        set_state(dir.path(), "idle").unwrap();
        assert_eq!(headless::read_state(dir.path()).unwrap().state, "idle");
        set_state(dir.path(), "stopped").unwrap();
        set_state(dir.path(), "working").unwrap();
        assert_eq!(headless::read_state(dir.path()).unwrap().state, "stopped");
    }
}
