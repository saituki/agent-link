//! # 遠隔の機械で claude を動かすための部品
//!
//! - 遠隔に置く中継スクリプト（[`shim_script`]）と、その ssh の設定（[`ssh_config_text`]）
//! - Mac で決めた命令だけを通す門番（[`gate_plan`]・[`run_gate`]。`agent-link remote-gate`）
//! - Mac の `~/.ssh/authorized_keys` の整え方（[`upsert_authorized_key`]）
//! - Mac から遠隔を操作する ssh の部品（[`ssh_run`]・[`stage_run_files`] など）
//!
//! 通信の形: 遠隔の claude のフックと `agent-link` の命令は、中継が
//! `ssh agent-link-mac <tok> <tok> …` で Mac に送る。各 `<tok>` は `e<hex>`（環境変数 `KEY=VALUE`）か
//! `a<hex>`（引数 1 つ）で、並びは `e` をすべて並べたあとに `a`。sshd は送られた文字列を
//! `SSH_ORIGINAL_COMMAND` に入れて、`authorized_keys` の強制コマンド（門番）を起こす。

use std::io::{Read, Write};
use std::path::Path;
use std::process::{Command, Output, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use crate::common::shell_escape_single_quotes;

/// 遠隔の `$HOME` の下の置き場。
pub const REMOTE_DIR: &str = ".agent-link-remote";
/// 中継の ssh の設定で Mac につける Host の別名。
pub const MAC_ALIAS: &str = "agent-link-mac";

/// 遠隔の操作の失敗（中身は英語 1 行）。
#[derive(Debug, Clone, PartialEq)]
pub struct RemoteError(pub String);

impl std::fmt::Display for RemoteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for RemoteError {}

/// 短い ssh（確認・読み書き）の待ち時間。
const SSH_SHORT: Duration = Duration::from_secs(30);

/// ssh の host 名の書式: 1〜63 文字。先頭は英数字、2 文字目からは英数字・`.`・`_`・`-`。
pub fn is_valid_host(host: &str) -> bool {
    let b = host.as_bytes();
    (1..=63).contains(&b.len())
        && b[0].is_ascii_alphanumeric()
        && b.iter()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'_' | b'-'))
}

/// Mac 側の ssh の実行ファイル。環境変数 `AGENT_LINK_SSH_BIN`（空でなければ）→ `ssh`。
/// （`settings.json` の `config` にも書ける）
pub fn ssh_bin() -> String {
    crate::settings::config_var("AGENT_LINK_SSH_BIN")
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "ssh".to_string())
}

/// `<ssh_bin> -o BatchMode=yes -o ConnectTimeout=10 <host> <script>` を起こす。
///
/// シェルを通さず、引数の配列で起こす。`stdin` が `Some` ならそれを書いて閉じる。
/// `timeout` を超えたら kill して `Err`。終了コードが 0 以外でも `Ok(Output)` を返す（判断は呼び出し側）。
pub fn ssh_run(
    host: &str,
    script: &str,
    stdin: Option<&[u8]>,
    timeout: Duration,
) -> Result<Output, RemoteError> {
    let mut child = Command::new(ssh_bin())
        .args([
            "-o",
            "BatchMode=yes",
            "-o",
            "ConnectTimeout=10",
            // 多重化の親プロセスが出力の口を握り続けないよう、常に単独の接続で行う。
            "-o",
            "ControlMaster=no",
            "-o",
            "ControlPath=none",
            host,
            script,
        ])
        .stdin(if stdin.is_some() { Stdio::piped() } else { Stdio::null() })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| RemoteError(format!("Failed to start {}: {}", ssh_bin(), e)))?;
    // 大きな入出力でも詰まらないよう、書く・読むは別のスレッドで行う。
    if let (Some(data), Some(mut pipe)) = (stdin, child.stdin.take()) {
        let data = data.to_vec();
        std::thread::spawn(move || {
            let _ = pipe.write_all(&data);
        });
    }
    let reader = |pipe: Option<Box<dyn Read + Send>>| {
        let (tx, rx) = mpsc::channel::<Vec<u8>>();
        std::thread::spawn(move || {
            let mut buf = Vec::new();
            if let Some(mut p) = pipe {
                let _ = p.read_to_end(&mut buf);
            }
            let _ = tx.send(buf);
        });
        rx
    };
    let out_rx = reader(child.stdout.take().map(|p| Box::new(p) as Box<dyn Read + Send>));
    let err_rx = reader(child.stderr.take().map(|p| Box::new(p) as Box<dyn Read + Send>));
    let start = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if start.elapsed() > timeout => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(RemoteError(format!(
                    "ssh to {} timed out after {}s",
                    host,
                    timeout.as_secs()
                )));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(100)),
            Err(e) => return Err(RemoteError(format!("Failed to wait for ssh to {}: {}", host, e))),
        }
    };
    // 読み取りは時間切れまで待つ（孫プロセスが口を握っても無期限に待たない）。
    // 時間切れなら空として扱う。
    let left = timeout.saturating_sub(start.elapsed()).max(Duration::from_secs(1));
    Ok(Output {
        status,
        stdout: out_rx.recv_timeout(left).unwrap_or_default(),
        stderr: err_rx.recv_timeout(left).unwrap_or_default(),
    })
}

/// 標準エラーの最後の空でない行（無ければ終了コードの文）。
pub fn last_line(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr)
        .lines()
        .rev()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| match output.status.code() {
            Some(c) => format!("exit code {}", c),
            None => "killed by a signal".to_string(),
        })
}

/// ssh の結果が 0 でなければ、標準エラーの最後の行を `Err` にする。
fn ok_or_last_line(host: &str, what: &str, output: Output) -> Result<Output, RemoteError> {
    if output.status.success() {
        Ok(output)
    } else {
        Err(RemoteError(format!("{} on {} failed: {}", what, host, last_line(&output))))
    }
}

/// 遠隔の `$HOME`。空なら `Err`。
pub fn remote_home(host: &str) -> Result<String, RemoteError> {
    let out = ok_or_last_line(
        host,
        "Reading $HOME",
        ssh_run(host, "printf %s \"$HOME\"", None, SSH_SHORT)?,
    )?;
    let home = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if home.is_empty() {
        Err(RemoteError(format!("$HOME is empty on {}", host)))
    } else {
        Ok(home)
    }
}

/// `~` と `~/…` を `home` で展開する。ほかはそのまま返す（存在は見ない）。
pub fn expand_remote_path(home: &str, raw: &str) -> String {
    if raw == "~" {
        home.to_string()
    } else if let Some(rest) = raw.strip_prefix("~/") {
        format!("{}/{}", home.trim_end_matches('/'), rest)
    } else {
        raw.to_string()
    }
}

/// 遠隔で `test -d '<path>'` が通るか。
pub fn remote_is_dir(host: &str, path: &str) -> Result<bool, RemoteError> {
    let script = format!("test -d {}", shell_escape_single_quotes(path));
    let out = ssh_run(host, &script, None, SSH_SHORT)?;
    match out.status.code() {
        Some(0) => Ok(true),
        Some(1) => Ok(false),
        _ => Err(RemoteError(format!("Checking a folder on {} failed: {}", host, last_line(&out)))),
    }
}

/// `<home>/.agent-link-remote/runs/<run_id>/` を作り、`files`（名前・中身）を書く。
/// 戻り値はそのフォルダの絶対パス。名前は `[A-Za-z0-9._-]+` だけ（違えば `Err`）。
pub fn stage_run_files(
    host: &str,
    home: &str,
    run_id: i64,
    files: &[(&str, &[u8])],
) -> Result<String, RemoteError> {
    for (name, _) in files {
        let ok = !name.is_empty()
            && *name != "."
            && *name != ".."
            && name.bytes().all(|c| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'_' | b'-'));
        if !ok {
            return Err(RemoteError(format!("Invalid file name for the remote run folder: {}", name)));
        }
    }
    let dir = format!("{}/{}/runs/{}", home.trim_end_matches('/'), REMOTE_DIR, run_id);
    let q = shell_escape_single_quotes(&dir);
    for (name, body) in files {
        let target = shell_escape_single_quotes(&format!("{}/{}", dir, name));
        let script = format!("mkdir -p {} && cat > {}", q, target);
        ok_or_last_line(host, "Writing a run file", ssh_run(host, &script, Some(body), SSH_SHORT)?)?;
    }
    if files.is_empty() {
        let script = format!("mkdir -p {}", q);
        ok_or_last_line(host, "Creating the run folder", ssh_run(host, &script, None, SSH_SHORT)?)?;
    }
    Ok(dir)
}

/// 遠隔の `<home>/.claude.json` を読み（無ければ `{}`）、
/// [`crate::tmux::seed_claude_json`] で整えて書き戻す。読めない JSON には触らず `Ok`。
///
/// 場所は遠隔の `$HOME`（`home` は呼び出し側が持つ値で、台本では使わない）。
pub fn seed_remote_claude_json(host: &str, _home: &str, dirs: &[&str]) -> Result<(), RemoteError> {
    let out = ok_or_last_line(
        host,
        "Reading ~/.claude.json",
        ssh_run(host, "cat \"$HOME/.claude.json\" 2>/dev/null || true", None, SSH_SHORT)?,
    )?;
    let text = String::from_utf8_lossy(&out.stdout);
    let value = if text.trim().is_empty() {
        serde_json::json!({})
    } else {
        match serde_json::from_str::<serde_json::Value>(&text) {
            Ok(v) => v,
            Err(_) => return Ok(()),
        }
    };
    let seeded = crate::tmux::seed_claude_json(value, dirs, None);
    let body = serde_json::to_vec_pretty(&seeded)
        .map_err(|e| RemoteError(format!("Failed to serialize ~/.claude.json: {}", e)))?;
    // 一時ファイル名は遠隔のシェルの pid で分ける（同じ名前の書きかけと衝突しない）。
    let script = "t=\"$HOME/.claude.json.agent-link.$$.tmp\"; cat > \"$t\" && mv \"$t\" \"$HOME/.claude.json\"";
    ok_or_last_line(host, "Writing ~/.claude.json", ssh_run(host, script, Some(&body), SSH_SHORT)?)?;
    Ok(())
}

/// 遠隔の `$HOME/.claude/projects/*/<session_id>.jsonl` を `dest` に写す
/// （`dest` の親を作り、一時ファイルに書いて rename）。無ければ `Ok(false)`。
/// `session_id` は `[0-9a-fA-F-]{1,64}` だけ（違えば `Err`）。
pub fn fetch_transcript(host: &str, session_id: &str, dest: &Path) -> Result<bool, RemoteError> {
    let ok = (1..=64).contains(&session_id.len())
        && session_id.bytes().all(|c| c.is_ascii_hexdigit() || c == b'-');
    if !ok {
        return Err(RemoteError(format!("Invalid session id: {}", session_id)));
    }
    let script = format!(
        "for f in \"$HOME\"/.claude/projects/*/{}.jsonl; do [ -f \"$f\" ] && exec cat \"$f\"; done; exit 3",
        session_id
    );
    let out = ssh_run(host, &script, None, Duration::from_secs(120))?;
    match out.status.code() {
        Some(3) => return Ok(false),
        Some(0) => {}
        _ => {
            return Err(RemoteError(format!(
                "Fetching the transcript from {} failed: {}",
                host,
                last_line(&out)
            )))
        }
    }
    let io_err = |e: std::io::Error| RemoteError(format!("Failed to write {}: {}", dest.display(), e));
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).map_err(io_err)?;
    }
    let mut tmp = dest.as_os_str().to_owned();
    tmp.push(".tmp");
    std::fs::write(&tmp, &out.stdout).map_err(io_err)?;
    std::fs::rename(&tmp, dest).map_err(io_err)?;
    Ok(true)
}

/// tmux の pane で走らせる命令。
/// `<ssh_bin> -t -o ServerAliveInterval=15 -o ServerAliveCountMax=4 '<host>' '<remote_script>'`
pub fn launch_command(host: &str, remote_script: &str) -> String {
    format!(
        "{} -t -o ServerAliveInterval=15 -o ServerAliveCountMax=4 {} {}",
        shell_escape_single_quotes(&ssh_bin()),
        shell_escape_single_quotes(host),
        shell_escape_single_quotes(remote_script),
    )
}

// ============================================================================
// 中継（遠隔に置く物）
// ============================================================================

/// 遠隔の `~/.agent-link-remote/bin/agent-link` の本文。
///
/// 環境変数（許す物だけ）と引数を 16 進にして `ssh agent-link-mac <tok> …` で Mac の門番へ送る。
/// `$t` は 16 進と `e`・`a` だけなので、引用符なしで分けてよい（`set -f` でパスの展開も止める）。
pub fn shim_script() -> &'static str {
    r#"#!/bin/sh
# agent-link の中継（agent-link remote setup が置いた）。命令を Mac の agent-link へ ssh で送る。
d="$HOME/.agent-link-remote"
hex() { printf '%s' "$1" | od -An -v -tx1 | tr -d ' \n'; }
set -f
t=""
for k in HERDR_PANE_ID AGENT_LINK_PROJECT_ROOT AGENT_LINK_PARENT_RUN_ID AGENT_LINK_BLOCK_NAME AGENT_LINK_IS_SUBTASK AGENT_LINK_WEB_URL AGENT_LINK_HOME; do
  eval "v=\${$k-}"
  [ -n "$v" ] && t="$t e$(hex "$k=$v")"
done
for a in "$@"; do t="$t a$(hex "$a")"; done
exec ssh -F "$d/ssh_config" agent-link-mac $t
"#
}

/// 中継の ssh の設定。`home` は遠隔の `$HOME`、`mac_addr` は遠隔から見た Mac の住所。
pub fn ssh_config_text(home: &str, mac_addr: &str, mac_user: &str) -> String {
    format!(
        "Host {alias}\n  HostName {addr}\n  User {user}\n  IdentityFile {d}/id_ed25519\n  IdentitiesOnly yes\n  BatchMode yes\n  ConnectTimeout 10\n  StrictHostKeyChecking accept-new\n  UserKnownHostsFile {d}/known_hosts\n  ControlMaster auto\n  ControlPath {d}/cm-%C\n  ControlPersist 10m\n  ServerAliveInterval 15\n  LogLevel ERROR\n",
        alias = MAC_ALIAS,
        addr = mac_addr,
        user = mac_user,
        d = format!("{}/{}", home.trim_end_matches('/'), REMOTE_DIR),
    )
}

// ============================================================================
// authorized_keys
// ============================================================================

/// [`upsert_authorized_key`] の結果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyChange {
    Added,
    Updated,
    Unchanged,
}

impl KeyChange {
    pub fn as_str(self) -> &'static str {
        match self {
            KeyChange::Added => "added",
            KeyChange::Updated => "updated",
            KeyChange::Unchanged => "unchanged",
        }
    }
}

/// `authorized_keys` の中身に `command="<exe> remote-gate",restrict <pubkey_line>` の行を入れる。
///
/// 鍵の本体（公開鍵の 2 語目）を含む行が無ければ末尾に足す。あって同じなら何もしない。
/// あって違えば（`exe` のパスが変わったなど）その行を入れ替える。ほかの行はそのまま残す。
pub fn upsert_authorized_key(current: &str, exe: &str, pubkey_line: &str) -> (String, KeyChange) {
    let pubkey_line = pubkey_line.trim();
    let body = pubkey_line.split_whitespace().nth(1).unwrap_or(pubkey_line);
    let new_line = format!("command=\"{} remote-gate\",restrict {}", exe, pubkey_line);
    let mut lines: Vec<&str> = current.lines().collect();
    let found = lines
        .iter()
        .position(|l| !l.trim_start().starts_with('#') && l.split_whitespace().any(|w| w == body));
    let change = match found {
        Some(i) if lines[i] == new_line => return (current.to_string(), KeyChange::Unchanged),
        Some(i) => {
            lines[i] = &new_line;
            KeyChange::Updated
        }
        None => {
            lines.push(&new_line);
            KeyChange::Added
        }
    };
    (lines.join("\n") + "\n", change)
}

// ============================================================================
// 門番
// ============================================================================

/// 門番が決めた実行の中身。
#[derive(Debug, Clone, PartialEq)]
pub enum GatePlan {
    /// Mac の `agent-link` をこの引数で実行する。
    Exec { env: Vec<(String, String)>, args: Vec<String> },
    /// `apps/web/hooks/<name>.py` を実行する。
    WebHook { name: String, env: Vec<(String, String)> },
    /// `agent-link-remote-ok` を出して 0。
    Ping,
}

/// 環境変数と引数を `e<hex>`・`a<hex>` の列（空白区切り）にする。
pub fn encode_gate_tokens(env: &[(String, String)], args: &[String]) -> String {
    let hex = |s: &str| s.bytes().map(|b| format!("{:02x}", b)).collect::<String>();
    let envs = env.iter().map(|(k, v)| format!("e{}", hex(&format!("{}={}", k, v))));
    let args = args.iter().map(|a| format!("a{}", hex(a)));
    envs.chain(args).collect::<Vec<_>>().join(" ")
}

/// 16 進（大文字小文字どちらも）を UTF-8 の文字列に戻す。
fn unhex(s: &str) -> Option<String> {
    if s.len() % 2 != 0 || !s.is_ascii() {
        return None;
    }
    let bytes = (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok())
        .collect::<Option<Vec<u8>>>()?;
    String::from_utf8(bytes).ok()
}

fn refuse_env(key: &str) -> (i32, String) {
    (2, format!("Remote gate refused the environment variable {}.", key))
}

/// 通す環境変数の値の規則。
fn env_value_ok(key: &str, value: &str) -> bool {
    let all = |max: usize, f: fn(char) -> bool| {
        (1..=max).contains(&value.chars().count()) && value.chars().all(f)
    };
    match key {
        "HERDR_PANE_ID" => value
            .strip_prefix("hl-")
            .is_some_and(|h| (1..=32).contains(&h.len()) && h.bytes().all(|c| matches!(c, b'0'..=b'9' | b'a'..=b'f'))),
        "AGENT_LINK_PROJECT_ROOT" => {
            Path::new(value).is_absolute() && Path::new(value).join(".agent-link").is_dir()
        }
        "AGENT_LINK_PARENT_RUN_ID" => all(18, |c| c.is_ascii_digit()),
        "AGENT_LINK_BLOCK_NAME" => all(64, |c| c.is_ascii_alphanumeric() || c == '_' || c == '-'),
        "AGENT_LINK_IS_SUBTASK" => value == "1",
        "AGENT_LINK_WEB_URL" => value
            .strip_prefix("http://127.0.0.1:")
            .is_some_and(|p| (1..=5).contains(&p.len()) && p.bytes().all(|c| c.is_ascii_digit())),
        "AGENT_LINK_HOME" => Path::new(value).is_absolute() && Path::new(value).is_dir(),
        _ => false,
    }
}

/// `SSH_ORIGINAL_COMMAND`（C3 の tok 列）から、門番がすることを決める純関数。
///
/// 断るときは `Err((終了コード 2, 英語 1 行))`。
pub fn gate_plan(original_command: &str) -> Result<GatePlan, (i32, String)> {
    let needs = || (2, "Remote gate needs hex tokens.".to_string());
    let mut env: Vec<(String, String)> = Vec::new();
    let mut args: Vec<String> = Vec::new();
    let mut tokens = 0;
    for tok in original_command.split_whitespace() {
        tokens += 1;
        if !tok.is_ascii() {
            return Err(needs());
        }
        let (kind, hex) = tok.split_at(1);
        let text = unhex(hex).ok_or_else(needs)?;
        match kind {
            "e" if args.is_empty() => {
                let (k, v) = text.split_once('=').unwrap_or((text.as_str(), ""));
                if !env_value_ok(k, v) {
                    return Err(refuse_env(k));
                }
                env.push((k.to_string(), v.to_string()));
            }
            "a" => args.push(text),
            _ => return Err(needs()),
        }
    }
    if tokens == 0 {
        return Err(needs());
    }
    let a: Vec<&str> = args.iter().map(String::as_str).collect();
    let allowed = match a.as_slice() {
        ["remote-ping"] => return Ok(GatePlan::Ping),
        ["web-hook", name @ ("cc_register" | "cc_question")] => {
            return Ok(GatePlan::WebHook { name: name.to_string(), env })
        }
        ["cc-state-hook", "working" | "idle" | "blocked" | "tool"]
        | ["cc-prompt-hook"]
        | ["cc-permission-hook"]
        | ["cc-statusline"] => true,
        ["doc", "write" | "read" | "list", ..] | ["task", "report", ..] => true,
        _ => false,
    };
    if allowed {
        Ok(GatePlan::Exec { env, args })
    } else {
        let head = a.iter().take(2).copied().collect::<Vec<_>>().join(" ");
        Err((2, format!("Remote gate does not allow this command: {}", head)))
    }
}

/// 門番の本体（`agent-link remote-gate`）。終了コードを返す。
pub fn run_gate() -> i32 {
    let original = std::env::var("SSH_ORIGINAL_COMMAND").unwrap_or_default();
    let (env, program, args) = match gate_plan(&original) {
        Err((code, msg)) => {
            eprintln!("{}", msg);
            return code;
        }
        Ok(GatePlan::Ping) => {
            println!("agent-link-remote-ok");
            return 0;
        }
        Ok(GatePlan::Exec { env, args }) => match std::env::current_exe() {
            Ok(exe) => (env, exe, args),
            Err(e) => {
                eprintln!("Remote gate cannot find its own executable: {}", e);
                return 1;
            }
        },
        Ok(GatePlan::WebHook { name, env }) => {
            // フォルダかファイルが無ければ何もしない（cc を止めない）
            let script = crate::cc_command::web_hooks_dir()
                .map(|d| d.join(format!("{}.py", name)))
                .filter(|p| p.is_file());
            match script {
                Some(p) => (env, p, Vec::new()),
                None => return 0,
            }
        }
    };
    let mut cmd = Command::new(&program);
    cmd.args(&args);
    for (k, _) in std::env::vars_os() {
        let k = k.to_string_lossy();
        if k.starts_with("AGENT_LINK_") || k == "HERDR_PANE_ID" {
            cmd.env_remove(&*k);
        }
    }
    cmd.envs(env);
    // `cc_question.py` が `shutil.which("agent-link")` で探すので、自分のフォルダを PATH の先頭に足す
    if let Some(dir) = std::env::current_exe().ok().and_then(|e| e.parent().map(Path::to_path_buf)) {
        let mut paths = vec![dir];
        paths.extend(std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()));
        if let Ok(joined) = std::env::join_paths(paths) {
            cmd.env("PATH", joined);
        }
    }
    match cmd.status() {
        Ok(status) => status.code().unwrap_or(1),
        Err(e) => {
            eprintln!("Remote gate failed to start {}: {}", program.display(), e);
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tok(env: &[(&str, &str)], args: &[&str]) -> String {
        let env: Vec<(String, String)> =
            env.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        let args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        encode_gate_tokens(&env, &args)
    }

    fn project() -> tempfile::TempDir {
        let t = tempfile::tempdir().unwrap();
        std::fs::create_dir(t.path().join(".agent-link")).unwrap();
        t
    }

    #[test]
    fn host_format() {
        assert!(is_valid_host("wsl-server"));
        assert!(is_valid_host("a.b_c-1"));
        for bad in ["", "-oProxyCommand=x", "a b", "user@h", ".x", &"a".repeat(64)] {
            assert!(!is_valid_host(bad), "{:?}", bad);
        }
        assert!(is_valid_host(&"a".repeat(63)));
    }

    #[test]
    fn tokens_round_trip() {
        let p = project();
        let root = p.path().to_string_lossy().to_string();
        let args = ["doc", "write", "--description", "", "日本語 と\n改行"];
        let s = tok(&[("AGENT_LINK_PROJECT_ROOT", &root), ("AGENT_LINK_PARENT_RUN_ID", "42")], &args);
        assert!(!s.contains('\n'));
        match gate_plan(&s).unwrap() {
            GatePlan::Exec { env, args: got } => {
                assert_eq!(got, args.iter().map(|s| s.to_string()).collect::<Vec<_>>());
                assert_eq!(env[0], ("AGENT_LINK_PROJECT_ROOT".to_string(), root));
                assert_eq!(env[1], ("AGENT_LINK_PARENT_RUN_ID".to_string(), "42".to_string()));
            }
            other => panic!("{:?}", other),
        }
    }

    #[test]
    fn gate_allows_commands() {
        assert_eq!(gate_plan(&tok(&[], &["remote-ping"])), Ok(GatePlan::Ping));
        for args in [
            vec!["cc-state-hook", "idle"],
            vec!["cc-state-hook", "tool"],
            vec!["cc-prompt-hook"],
            vec!["cc-permission-hook"],
            vec!["cc-statusline"],
            vec!["doc", "read", "AXANSMpo"],
            vec!["doc", "list", "--json"],
            vec!["task", "report", "--json"],
        ] {
            assert!(matches!(gate_plan(&tok(&[], &args)), Ok(GatePlan::Exec { .. })), "{:?}", args);
        }
        assert_eq!(
            gate_plan(&tok(&[("AGENT_LINK_WEB_URL", "http://127.0.0.1:8400")], &["web-hook", "cc_question"])),
            Ok(GatePlan::WebHook {
                name: "cc_question".to_string(),
                env: vec![("AGENT_LINK_WEB_URL".to_string(), "http://127.0.0.1:8400".to_string())],
            })
        );
    }

    #[test]
    fn gate_refuses() {
        let refused = |s: String| gate_plan(&s).unwrap_err();
        assert_eq!(
            refused(tok(&[], &["task", "create"])),
            (2, "Remote gate does not allow this command: task create".to_string())
        );
        for args in [
            vec!["web-hook", "other"],
            vec!["cc-state-hook", "x"],
            vec!["daemon", "stop"],
            vec!["remote-gate"],
        ] {
            assert_eq!(refused(tok(&[], &args)).0, 2, "{:?}", args);
        }
        assert_eq!(
            refused(tok(&[("PATH", "/tmp")], &["remote-ping"])),
            (2, "Remote gate refused the environment variable PATH.".to_string())
        );
        assert_eq!(refused(tok(&[("HERDR_PANE_ID", "hl-XYZ")], &["remote-ping"])).0, 2);
        let plain = tempfile::tempdir().unwrap();
        let root = plain.path().to_string_lossy().to_string();
        assert_eq!(
            refused(tok(&[("AGENT_LINK_PROJECT_ROOT", &root)], &["remote-ping"])),
            (2, "Remote gate refused the environment variable AGENT_LINK_PROJECT_ROOT.".to_string())
        );
        for (k, v) in [
            ("AGENT_LINK_PARENT_RUN_ID", "1x"),
            ("AGENT_LINK_PARENT_RUN_ID", "1234567890123456789"),
            ("AGENT_LINK_BLOCK_NAME", "a b"),
            ("AGENT_LINK_IS_SUBTASK", "0"),
            ("AGENT_LINK_WEB_URL", "http://evil.example:80"),
            ("AGENT_LINK_HOME", "relative"),
        ] {
            assert_eq!(refused(tok(&[(k, v)], &["remote-ping"])).0, 2, "{}={}", k, v);
        }
        assert!(gate_plan(&tok(&[("HERDR_PANE_ID", "hl-0a1f")], &["remote-ping"])).is_ok());
        // 壊れた tok・空・順序違い
        for bad in ["", "   ", "azz", "a6", "x61", "a61 e41", "a ff"] {
            assert_eq!(gate_plan(bad).unwrap_err(), (2, "Remote gate needs hex tokens.".to_string()), "{:?}", bad);
        }
    }

    #[test]
    fn empty_argument_is_bare_a() {
        assert_eq!(tok(&[], &["doc", ""]), "a646f63 a");
        assert!(matches!(
            gate_plan(&tok(&[], &["doc", "list", ""])),
            Ok(GatePlan::Exec { .. })
        ));
    }

    #[test]
    fn authorized_keys_upsert() {
        let key = "ssh-ed25519 AAAAC3Nz agent-link-remote@wsl-server";
        let line = format!("command=\"/opt/agent-link remote-gate\",restrict {}", key);
        let (s, c) = upsert_authorized_key("ssh-rsa OTHER me@mac", "/opt/agent-link", key);
        assert_eq!(c, KeyChange::Added);
        assert_eq!(s, format!("ssh-rsa OTHER me@mac\n{}\n", line));
        let (s2, c2) = upsert_authorized_key(&s, "/opt/agent-link", key);
        assert_eq!((s2.as_str(), c2), (s.as_str(), KeyChange::Unchanged));
        let (s3, c3) = upsert_authorized_key(&s, "/new/agent-link", key);
        assert_eq!(c3, KeyChange::Updated);
        assert_eq!(
            s3,
            format!("ssh-rsa OTHER me@mac\ncommand=\"/new/agent-link remote-gate\",restrict {}\n", key)
        );
        // 空のファイル
        assert_eq!(upsert_authorized_key("", "/x", key).1, KeyChange::Added);
    }

    #[test]
    fn remote_path_and_launch_quoting() {
        assert_eq!(expand_remote_path("/home/u", "~"), "/home/u");
        assert_eq!(expand_remote_path("/home/u/", "~/work/a"), "/home/u/work/a");
        assert_eq!(expand_remote_path("/home/u", "/srv/x"), "/srv/x");
        assert_eq!(expand_remote_path("/home/u", "~x"), "~x");
        std::env::remove_var("AGENT_LINK_SSH_BIN");
        assert_eq!(
            launch_command("wsl-server", "cd 'a b' && claude"),
            "'ssh' -t -o ServerAliveInterval=15 -o ServerAliveCountMax=4 'wsl-server' 'cd '\\''a b'\\'' && claude'"
        );
    }

    #[test]
    fn shim_and_config_text() {
        assert!(shim_script().starts_with("#!/bin/sh\n"));
        assert!(shim_script().contains("exec ssh -F \"$d/ssh_config\" agent-link-mac $t"));
        let c = ssh_config_text("/home/u", "100.1.2.3", "me");
        assert!(c.starts_with("Host agent-link-mac\n  HostName 100.1.2.3\n  User me\n"));
        assert!(c.contains("IdentityFile /home/u/.agent-link-remote/id_ed25519\n"));
        assert!(c.contains("ControlPath /home/u/.agent-link-remote/cm-%C\n"));
    }
}
