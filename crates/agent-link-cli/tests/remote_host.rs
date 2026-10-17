//! 遠隔の子（`task create` の `host`）の通しの試験。偽の ssh を使う。
//!
//! 役は 4 つ。すべて一時フォルダで、本物の `~/.ssh`・`~/.claude`・`~/.agent-link`・遠隔の機械には触らない。
//!
//! - Mac のプロジェクト（`agent-link init` 済み）と Mac の `HOME`・`AGENT_LINK_HOME`
//! - 偽の遠隔の `HOME`（`remote_home`）
//! - 偽の ssh（`fakebin/ssh`）。Mac から遠隔（`ssh [-t] -o … host script`）と、
//!   遠隔の中継から Mac（`ssh -F cfg agent-link-mac <tok>…`）の両方を受ける。
//!   Mac 側は `AGENT_LINK_SSH_BIN`、遠隔の中継側は遠隔の PATH の先頭で差し替える。
//! - 偽の遠隔の claude（`remote_home/.local/bin/claude`）
//!
//! 1〜4 は ssh-keygen だけで動く。5（起動から完了まで）は tmux が要る。使えなければ skip と表示して成功で返す。
//! 偽の ssh と偽の claude は、パスを本文に直に書く（tmux の pane には試験の環境変数が届かないため）。

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

use agent_link_engine::db::models::BlockMetadata;
use agent_link_engine::db::SaiStore;

fn bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_agent-link"))
}

fn write_exec(path: &Path, body: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, body).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

fn last_stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).lines().last().unwrap_or("").to_string()
}

fn stderr_of(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).to_string()
}

fn tmux_usable() -> bool {
    Command::new("tmux").arg("-V").output().map(|o| o.status.success()).unwrap_or(false)
}

fn ssh_keygen_usable() -> bool {
    Command::new("ssh-keygen").arg("-?").output().is_ok()
}

struct World {
    _base: tempfile::TempDir,
    mac_home: PathBuf,
    al_home: PathBuf,
    project: PathBuf,
    remote_home: PathBuf,
    fakebin: PathBuf,
}

impl World {
    /// 一時フォルダに 4 つの役を作る。`agent-link init` まで済ませる。
    fn new() -> World {
        let base = tempfile::tempdir().unwrap();
        // macOS の /var → /private/var の違いを避ける
        let b = std::fs::canonicalize(base.path()).unwrap();
        let w = World {
            mac_home: b.join("mac-home"),
            al_home: b.join("al-home"),
            project: b.join("proj"),
            remote_home: b.join("remote-home"),
            fakebin: b.join("fakebin"),
            _base: base,
        };
        for d in [&w.mac_home, &w.al_home, &w.project, &w.remote_home, &w.fakebin] {
            std::fs::create_dir_all(d).unwrap();
        }
        write_exec(&w.ssh(), &w.fake_ssh_script());
        write_exec(
            &w.remote_home.join(".local/bin/claude"),
            "#!/bin/sh\necho '9.9.9 (Claude Code)'\n",
        );
        let out = w.mac(&["init", "--name", "remote-host"]);
        assert!(out.status.success(), "init: {}", stderr_of(&out));
        w
    }

    fn ssh(&self) -> PathBuf {
        self.fakebin.join("ssh")
    }

    fn fake_ssh_script(&self) -> String {
        format!(
            r#"#!/bin/sh
# 遠隔の中継から Mac へ: 門番を起こす（Mac の HOME で）
if [ "$1" = "-F" ]; then
  shift 3
  SSH_ORIGINAL_COMMAND="$*" HOME='{mac_home}' exec '{bin}' remote-gate
fi
# Mac から遠隔へ: 遠隔の HOME・PATH で script を走らせる
while [ "$1" = "-o" ] || [ "$1" = "-t" ]; do
  case "$1" in -o) shift 2 ;; *) shift ;; esac
done
shift
PATH='{fakebin}:/usr/bin:/bin' HOME='{remote_home}' SSH_CONNECTION="10.0.0.1 5000 10.0.0.2 22" exec sh -c "$1"
"#,
            mac_home = self.mac_home.display(),
            bin = bin().display(),
            fakebin = self.fakebin.display(),
            remote_home = self.remote_home.display(),
        )
    }

    /// Mac 側の `agent-link`。開発中の pane の値は外す。
    fn mac_cmd(&self, args: &[&str]) -> Command {
        let mut c = Command::new(bin());
        c.args(args)
            .current_dir(&self.project)
            .env("HOME", &self.mac_home)
            .env("USER", "tester")
            .env("PATH", format!("{}:{}", self.fakebin.display(), std::env::var("PATH").unwrap_or_default()))
            .env("AGENT_LINK_SSH_BIN", self.ssh())
            .env("AGENT_LINK_PROJECT_ROOT", &self.project)
            .env("AGENT_LINK_HOME", &self.al_home)
            .env_remove("HERDR_PANE_ID")
            .env_remove("AGENT_LINK_PARENT_RUN_ID")
            .env_remove("AGENT_LINK_BLOCK_NAME")
            .env_remove("AGENT_LINK_IS_SUBTASK");
        c
    }

    fn mac(&self, args: &[&str]) -> Output {
        self.mac_cmd(args).output().unwrap()
    }

    /// `remote setup wsl-server` を通す（claude の偽は `new()` が置く）。
    fn setup(&self) {
        let out = self.mac(&["remote", "setup", "wsl-server", "--json"]);
        assert_eq!(out.status.code(), Some(0), "setup: {}", stderr_of(&out));
    }

    /// 遠隔の `sh -c script`（中継が PATH の先頭）。`env` は遠隔の環境に足す。
    fn remote_sh(&self, env: &[(&str, &str)], script: &str) -> Output {
        let mut c = Command::new("sh");
        c.arg("-c")
            .arg(script)
            .env_clear()
            .env("HOME", &self.remote_home)
            .env(
                "PATH",
                format!("{}/.agent-link-remote/bin:{}:/usr/bin:/bin", self.remote_home.display(), self.fakebin.display()),
            );
        for (k, v) in env {
            c.env(k, v);
        }
        c.output().unwrap()
    }

    /// DB に `running` の run を 1 件作る（`run_send_interrupt.rs` と同じ作り方）。
    fn make_run(&self) -> i64 {
        let db = self.project.join(".agent-link/engine/agent-link.db");
        std::fs::create_dir_all(db.parent().unwrap()).unwrap();
        let store = SaiStore::open(&db).unwrap();
        let block = BlockMetadata::new(
            "remote-host-parent".to_string(),
            "遠隔の試験の親".to_string(),
            "anthropic/claude-sonnet-4-5".to_string(),
            "off".to_string(),
            "true".to_string(),
            "true".to_string(),
            None,
            300,
            "none".to_string(),
            true,
            "/tmp/remote-host-parent.md".to_string(),
            None,
        );
        let block_id = store.insert_block(&block).unwrap();
        let run_id = store.create_execution(block_id, None).unwrap();
        store.update_execution_status(run_id, "running").unwrap();
        run_id
    }
}

// ----------------------------------------------------------------------------
// 1. setup
// ----------------------------------------------------------------------------

#[test]
fn setup_puts_one_forced_command_line_and_is_idempotent() {
    if !ssh_keygen_usable() {
        eprintln!("skip: ssh-keygen が使えないため remote setup を確かめません");
        return;
    }
    let w = World::new();
    let out = w.mac(&["remote", "setup", "wsl-server", "--json"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr_of(&out));
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["authorized_keys"], "added");
    assert_eq!(v["ok"], true);
    // remote_to_mac が ok（中継 → 偽の ssh → 門番 → remote-ping の往復）
    let checks = v["checks"].as_array().unwrap();
    let rtm = checks.iter().find(|c| c["name"] == "remote_to_mac").expect("remote_to_mac");
    assert_eq!(rtm["ok"], true, "{v}");

    let ak_path = w.mac_home.join(".ssh/authorized_keys");
    let ak = std::fs::read_to_string(&ak_path).unwrap();
    let lines: Vec<&str> = ak.lines().collect();
    assert_eq!(lines.len(), 1, "{ak}");
    let want = format!("command=\"{} remote-gate\",restrict ssh-ed25519 ", bin().display());
    assert!(lines[0].starts_with(&want), "{ak}");

    // もう一度: 変わらない
    let out = w.mac(&["remote", "setup", "wsl-server", "--json"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr_of(&out));
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["authorized_keys"], "unchanged");
    assert_eq!(std::fs::read_to_string(&ak_path).unwrap(), ak);
}

// ----------------------------------------------------------------------------
// 2. 中継の往復
// ----------------------------------------------------------------------------

#[test]
fn relay_writes_a_doc_into_the_mac_run() {
    if !ssh_keygen_usable() {
        eprintln!("skip: ssh-keygen が使えないため中継を確かめません");
        return;
    }
    let w = World::new();
    w.setup();
    let run = w.make_run();
    let run_s = run.to_string();
    let root = w.project.to_string_lossy().to_string();
    let al_home = w.al_home.to_string_lossy().to_string();

    let out = w.remote_sh(
        &[
            ("AGENT_LINK_PROJECT_ROOT", &root),
            ("AGENT_LINK_PARENT_RUN_ID", &run_s),
            ("AGENT_LINK_HOME", &al_home),
        ],
        "agent-link doc write --filename r.md --description '作業報告 R: 遠隔の試験' --json <<'EOF'\n# 遠隔の試験\n本文\nEOF",
    );
    assert_eq!(out.status.code(), Some(0), "{}", stderr_of(&out));
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).expect("doc write の JSON");
    let token = v["token"].as_str().expect("token が返る").to_string();

    // Mac の run に文書ができている
    let out = w.mac(&["doc", "list", "--run", &run_s, "--all", "--json"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr_of(&out));
    let l: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(l["count"], 1, "{l}");
    assert_eq!(l["items"][0]["token"], token.as_str());
    let out = w.mac(&["doc", "read", &token]);
    assert!(String::from_utf8_lossy(&out.stdout).contains("# 遠隔の試験"), "{}", stderr_of(&out));
}

// ----------------------------------------------------------------------------
// 3. 門番が断る
// ----------------------------------------------------------------------------

#[test]
fn gate_refuses_other_commands_through_the_relay() {
    if !ssh_keygen_usable() {
        eprintln!("skip: ssh-keygen が使えないため中継を確かめません");
        return;
    }
    let w = World::new();
    w.setup();
    let out = w.remote_sh(&[], "agent-link task create </dev/null");
    assert_eq!(out.status.code(), Some(2), "{}", stderr_of(&out));
    assert!(out.stdout.is_empty());
    assert_eq!(last_stderr(&out), "Remote gate does not allow this command: task create");
}

// ----------------------------------------------------------------------------
// 4. 状態のフック
// ----------------------------------------------------------------------------

#[test]
fn state_hook_through_the_relay_updates_the_mac_pane() {
    if !ssh_keygen_usable() {
        eprintln!("skip: ssh-keygen が使えないため中継を確かめません");
        return;
    }
    let w = World::new();
    w.setup();
    // Mac の pane のフォルダ（tmux の印あり・idle）
    let pane = "hl-0a1b2c3d4e5f";
    let dir = w.project.join(".agent-link/engine/headless").join(pane);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("tmux"), b"").unwrap();
    std::fs::write(
        dir.join("state.json"),
        serde_json::json!({
            "pane_id": pane, "runner": "", "state": "idle", "turn": 0,
            "partial": "", "error": "", "updated_at": "2026-10-17T00:00:00Z"
        })
        .to_string(),
    )
    .unwrap();

    let root = w.project.to_string_lossy().to_string();
    let out = w.remote_sh(
        &[("HERDR_PANE_ID", pane), ("AGENT_LINK_PROJECT_ROOT", &root)],
        "agent-link cc-state-hook working </dev/null",
    );
    assert_eq!(out.status.code(), Some(0), "{}", stderr_of(&out));
    let state: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("state.json")).unwrap()).unwrap();
    assert_eq!(state["state"], "working", "{state}");
}

// ----------------------------------------------------------------------------
// 5. 起動から完了まで
// ----------------------------------------------------------------------------

/// 終わるときに daemon と tmux の session を止める（失敗しても残さない）。
struct Cleanup {
    bin: PathBuf,
    project: PathBuf,
    al_home: PathBuf,
    mac_home: PathBuf,
}

impl Drop for Cleanup {
    fn drop(&mut self) {
        let _ = Command::new(&self.bin)
            .args(["daemon", "stop", "--force"])
            .current_dir(&self.project)
            .env("HOME", &self.mac_home)
            .env("AGENT_LINK_PROJECT_ROOT", &self.project)
            .env("AGENT_LINK_HOME", &self.al_home)
            .output();
        // この試験の pane（hl-…）だけを止める
        let dir = self.project.join(".agent-link/engine/headless");
        if let Ok(rd) = std::fs::read_dir(dir) {
            for e in rd.flatten() {
                let name = e.file_name().to_string_lossy().to_string();
                if name.starts_with("hl-") {
                    let _ = Command::new("tmux").args(["kill-session", "-t", &name]).output();
                }
            }
        }
    }
}

/// 偽の遠隔の claude。`--settings` を確かめ、フック・文書・報告・会話記録を、本物と同じ順で出す。
const FAKE_REMOTE_CLAUDE: &str = r##"#!/bin/sh
exec 2>"$HOME/claude.err"
sid=""; settings=""
while [ $# -gt 0 ]; do
  case "$1" in --session-id) sid="$2"; shift ;; --settings) settings="$2"; shift ;; esac
  shift
done
if [ ! -f "$settings" ]; then echo "no --settings file: $settings" >&2; sleep 30; exit 3; fi
agent-link cc-state-hook idle </dev/null
echo ready
IFS= read -r line
printf '%s' '{"prompt":"x"}' | agent-link cc-prompt-hook
# 親は 250ms ごとに working を見に来る。1 秒以上は working に留める
sleep 2
mkdir -p "$HOME/.claude/projects/-x"
printf '%s\n' '{"type":"user","message":{"role":"user","content":"x"}}' > "$HOME/.claude/projects/-x/$sid.jsonl"
agent-link doc write --filename remote-out.md --description "作業報告 R: 遠隔の成果" <<'EOT'
# 遠隔の成果
EOT
agent-link task report <<'EOT'
結果: DONE
要約: 遠隔の偽 claude が動いた
EOT
agent-link cc-state-hook idle </dev/null
sleep 60
"##;

#[test]
fn remote_child_runs_from_launch_to_completion() {
    if !ssh_keygen_usable() || !tmux_usable() {
        eprintln!("skip: ssh-keygen か tmux が使えないため、遠隔の子の起動から完了までを確かめません");
        return;
    }
    let w = World::new();
    w.setup();
    write_exec(&w.remote_home.join(".local/bin/claude"), FAKE_REMOTE_CLAUDE);
    std::fs::create_dir_all(w.remote_home.join("work")).unwrap();
    let empty_hooks = w.al_home.join("no-hooks");
    std::fs::create_dir_all(&empty_hooks).unwrap();

    let _cleanup = Cleanup {
        bin: bin(),
        project: w.project.clone(),
        al_home: w.al_home.clone(),
        mac_home: w.mac_home.clone(),
    };
    let envs = |c: &mut Command| {
        // 本物の Web に届かせない
        c.env("AGENT_LINK_WEB_URL", "http://127.0.0.1:1")
            .env("AGENT_LINK_WEB_HOOKS_DIR", &empty_hooks);
    };

    let mut d = w.mac_cmd(&["daemon", "start"]);
    envs(&mut d);
    let out = d.output().unwrap();
    assert!(out.status.success(), "daemon start: {}", stderr_of(&out));

    let run = w.make_run();
    let run_s = run.to_string();
    let body = r#"[{"client_id":"T1","subject":"T1: 遠隔の試し","block_type":"sub","host":"fakehost","cwd":"~/work","task_text":"何もせず task report せよ"}]"#;
    let mut c = w.mac_cmd(&["task", "create", "--run-id", &run_s, "--json"]);
    envs(&mut c);
    let mut child = c.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
    std::io::Write::write_all(child.stdin.as_mut().unwrap(), body.as_bytes()).unwrap();
    let out = child.wait_with_output().unwrap();
    assert_eq!(out.status.code(), Some(0), "task create: {}", stderr_of(&out));
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let batch = v["batch_id"].as_str().expect("batch_id").to_string();
    let task_id = v["tasks"][0]["task_id"].as_i64().expect("task_id").to_string();

    let mut c = w.mac_cmd(&["task", "execute", "--batch", &batch, "--run-id", &run_s, "--json"]);
    envs(&mut c);
    let out = c.output().unwrap();
    assert_eq!(out.status.code(), Some(0), "task execute: {}", stderr_of(&out));

    // 完了まで待つ（1 回の待ちは最大 90 秒。全体で 150 秒まで）
    let deadline = Instant::now() + Duration::from_secs(150);
    let done = loop {
        let mut c = w.mac_cmd(&["task", "wait", &task_id, "--timeout-ms", "20000", "--json"]);
        envs(&mut c);
        let out = c.output().unwrap();
        assert_eq!(out.status.code(), Some(0), "task wait: {}", stderr_of(&out));
        let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
        match v["status"].as_str() {
            Some("completed") => break v,
            Some("failed") => panic!("子が失敗: {v}\n{}", daemon_log(&w)),
            _ => {}
        }
        if Instant::now() > deadline {
            let cap = w.mac(&["task", "capture", &task_id, "--json"]);
            panic!(
                "完了しない: {v}\ncapture: {}\n{}",
                String::from_utf8_lossy(&cap.stdout),
                daemon_log(&w)
            );
        }
    };
    assert!(
        done["report"].as_str().unwrap_or("").contains("遠隔の偽 claude が動いた"),
        "report: {done}"
    );
    let outputs = done["outputs"].to_string();
    assert!(outputs.contains("遠隔の成果"), "outputs: {done}");

    // host 付きの子の task の JSON
    let out = w.mac_cmd(&["task", "show", &task_id, "--json"]).env("AGENT_LINK_PARENT_RUN_ID", &run_s).output().unwrap();
    let t: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(t["tasks"][0]["host"], "fakehost", "{t}");
    assert!(t["tasks"][0]["worktree_info"].is_null(), "{t}");

    // 会話記録が Mac の HOME に写る（pane を閉じる処理のあと。少し待つ）
    let projects = w.mac_home.join(".claude/projects");
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if find_jsonl(&projects) {
            break;
        }
        if Instant::now() > deadline {
            panic!("会話記録が {} に写らない\n{}", projects.display(), daemon_log(&w));
        }
        std::thread::sleep(Duration::from_millis(300));
    }
}

fn find_jsonl(dir: &Path) -> bool {
    let Ok(rd) = std::fs::read_dir(dir) else { return false };
    rd.flatten().any(|e| {
        let p = e.path();
        if p.is_dir() {
            find_jsonl(&p)
        } else {
            p.extension().is_some_and(|x| x == "jsonl")
        }
    })
}

fn daemon_log(w: &World) -> String {
    let p = w.project.join(".agent-link/engine/daemon.log");
    format!("daemon.log:\n{}\nclaude.err:\n{}", std::fs::read_to_string(p).unwrap_or_default(), std::fs::read_to_string(w.remote_home.join("claude.err")).unwrap_or_default())
}
