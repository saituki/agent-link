//! 門番（`agent-link remote-gate`）と `remote setup` / `remote check` の試験。
//!
//! 本物の ssh・`~/.ssh`・遠隔の機械には触らない。一時の `HOME` と、
//! `AGENT_LINK_SSH_BIN`（と PATH の先頭）に置いた偽の ssh で行う。

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use agent_link_engine::remote::encode_gate_tokens;

fn bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_agent-link"))
}

fn tok(env: &[(&str, &str)], args: &[&str]) -> String {
    let env: Vec<(String, String)> = env.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
    let args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
    encode_gate_tokens(&env, &args)
}

fn gate(original: &str, extra_env: &[(&str, &Path)]) -> Output {
    let mut cmd = Command::new(bin());
    cmd.arg("remote-gate").env("SSH_ORIGINAL_COMMAND", original);
    for (k, v) in extra_env {
        cmd.env(k, v);
    }
    cmd.output().unwrap()
}

fn last_stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).lines().last().unwrap_or("").to_string()
}

/// `agent-link init` 済みの一時プロジェクト（root, home）。
fn init_project(base: &Path) -> (PathBuf, PathBuf) {
    let root = base.join("proj");
    let home = base.join("al-home");
    std::fs::create_dir_all(&root).unwrap();
    let out = Command::new(bin())
        .args(["init", "--name", "gate-check"])
        .current_dir(&root)
        .env("AGENT_LINK_PROJECT_ROOT", &root)
        .env("AGENT_LINK_HOME", &home)
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    (root, home)
}

#[test]
fn gate_ping() {
    let out = gate(&tok(&[], &["remote-ping"]), &[]);
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(String::from_utf8_lossy(&out.stdout), "agent-link-remote-ok\n");
}

#[test]
fn gate_runs_doc_list_on_this_mac() {
    let base = tempfile::tempdir().unwrap();
    let (root, home) = init_project(base.path());
    let root_s = root.to_string_lossy().to_string();
    let home_s = home.to_string_lossy().to_string();
    let original = tok(
        &[("AGENT_LINK_PROJECT_ROOT", &root_s), ("AGENT_LINK_HOME", &home_s)],
        &["doc", "list", "--json"],
    );
    let out = gate(&original, &[]);
    assert_eq!(out.status.code(), Some(0), "{}", String::from_utf8_lossy(&out.stderr));
    serde_json::from_slice::<serde_json::Value>(&out.stdout).expect("doc list --json の JSON");
}

#[test]
fn gate_refuses_task_create_and_bad_env() {
    let out = gate(&tok(&[], &["task", "create"]), &[]);
    assert_eq!(out.status.code(), Some(2));
    assert!(out.stdout.is_empty());
    assert_eq!(last_stderr(&out), "Remote gate does not allow this command: task create");

    let out = gate(&tok(&[("PATH", "/tmp")], &["remote-ping"]), &[]);
    assert_eq!(out.status.code(), Some(2));
    assert_eq!(last_stderr(&out), "Remote gate refused the environment variable PATH.");

    let out = gate("", &[]);
    assert_eq!(out.status.code(), Some(2));
    assert_eq!(last_stderr(&out), "Remote gate needs hex tokens.");
}

/// 偽の ssh。1 本で、Mac から遠隔（`ssh -o … host script`）と、遠隔の中継から Mac（`ssh -F cfg agent-link-mac toks`）の両方を受ける。
const FAKE_SSH: &str = r#"#!/bin/sh
if [ "$1" = "-F" ]; then
  shift 2; shift
  SSH_ORIGINAL_COMMAND="$*" exec "$FAKE_AGENT_LINK" remote-gate
fi
while [ "$1" = "-o" ]; do shift 2; done
shift
PATH="$FAKE_BIN:/usr/bin:/bin" HOME="$FAKE_REMOTE_HOME" SSH_CONNECTION="10.0.0.1 5000 10.0.0.2 22" exec sh -c "$1"
"#;

struct Fake {
    _base: tempfile::TempDir,
    mac_home: PathBuf,
    remote_home: PathBuf,
    ssh: PathBuf,
}

fn fake(with_claude: bool) -> Fake {
    let base = tempfile::tempdir().unwrap();
    let mac_home = base.path().join("mac-home");
    let remote_home = base.path().join("remote-home");
    let fakebin = base.path().join("fakebin");
    for d in [&mac_home, &remote_home, &fakebin] {
        std::fs::create_dir_all(d).unwrap();
    }
    let ssh = fakebin.join("ssh");
    std::fs::write(&ssh, FAKE_SSH).unwrap();
    std::fs::set_permissions(&ssh, std::fs::Permissions::from_mode(0o755)).unwrap();
    if with_claude {
        let dir = remote_home.join(".local/bin");
        std::fs::create_dir_all(&dir).unwrap();
        let claude = dir.join("claude");
        std::fs::write(&claude, "#!/bin/sh\necho '9.9.9 (fake)'\n").unwrap();
        std::fs::set_permissions(&claude, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    Fake { _base: base, mac_home, remote_home, ssh }
}

fn remote_cmd(f: &Fake, args: &[&str]) -> Output {
    let path = format!(
        "{}:{}",
        f.ssh.parent().unwrap().display(),
        std::env::var("PATH").unwrap_or_default()
    );
    Command::new(bin())
        .arg("remote")
        .args(args)
        .env("HOME", &f.mac_home)
        .env("USER", "tester")
        .env("PATH", path)
        .env("AGENT_LINK_SSH_BIN", &f.ssh)
        .env("FAKE_BIN", f.ssh.parent().unwrap())
        .env("FAKE_AGENT_LINK", bin())
        // 開発中の pane の値を、遠隔の環境として中継に拾わせない
        .env_remove("HERDR_PANE_ID")
        .env_remove("AGENT_LINK_PROJECT_ROOT")
        .env_remove("AGENT_LINK_PARENT_RUN_ID")
        .env_remove("AGENT_LINK_BLOCK_NAME")
        .env_remove("AGENT_LINK_IS_SUBTASK")
        .env_remove("AGENT_LINK_WEB_URL")
        .env_remove("AGENT_LINK_HOME")
        .env("FAKE_REMOTE_HOME", &f.remote_home)
        .output()
        .unwrap()
}

#[test]
fn setup_then_check_with_fake_ssh() {
    let f = fake(true);
    let out = remote_cmd(&f, &["setup", "wsl-server", "--json"]);
    assert_eq!(out.status.code(), Some(0), "{}", String::from_utf8_lossy(&out.stderr));
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["host"], "wsl-server");
    assert_eq!(v["mac"], "10.0.0.1");
    assert_eq!(v["authorized_keys"], "added");
    assert_eq!(v["ok"], true);
    assert_eq!(v["checks"].as_array().unwrap().len(), 4);
    assert_eq!(v["checks"][1]["detail"], "9.9.9 (fake)");

    // 遠隔に置いた物
    let d = f.remote_home.join(".agent-link-remote");
    assert_eq!(
        std::fs::metadata(d.join("bin/agent-link")).unwrap().permissions().mode() & 0o777,
        0o755
    );
    assert_eq!(std::fs::metadata(d.join("ssh_config")).unwrap().permissions().mode() & 0o777, 0o600);
    let config = std::fs::read_to_string(d.join("ssh_config")).unwrap();
    assert!(config.contains("HostName 10.0.0.1\n  User tester\n"), "{}", config);
    assert!(d.join("id_ed25519").is_file() && d.join("id_ed25519.pub").is_file());

    // Mac の authorized_keys
    let ak_path = f.mac_home.join(".ssh/authorized_keys");
    let ak = std::fs::read_to_string(&ak_path).unwrap();
    assert!(ak.starts_with(&format!("command=\"{} remote-gate\",restrict ssh-ed25519 ", bin().display())), "{}", ak);
    assert_eq!(std::fs::metadata(&ak_path).unwrap().permissions().mode() & 0o777, 0o600);
    assert_eq!(std::fs::metadata(f.mac_home.join(".ssh")).unwrap().permissions().mode() & 0o777, 0o700);

    // 2 回目は変わらない。check も通る
    let out = remote_cmd(&f, &["setup", "wsl-server", "--json"]);
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["authorized_keys"], "unchanged");
    assert_eq!(std::fs::read_to_string(&ak_path).unwrap(), ak);
    let out = remote_cmd(&f, &["check", "wsl-server", "--json"]);
    assert_eq!(out.status.code(), Some(0));
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["ok"], true);
}

#[test]
fn check_fails_at_first_broken_item() {
    // claude が無い: 2 項目目で止まり、標準出力は空。最後の行は `Remote check failed: …`
    let f = fake(false);
    let out = remote_cmd(&f, &["check", "wsl-server"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(out.stdout.is_empty());
    assert_eq!(
        last_stderr(&out),
        "Remote check failed: claude: claude is not installed on wsl-server (looked in PATH and ~/.local/bin)."
    );
    // 中継が無い（claude はある）
    let f = fake(true);
    let out = remote_cmd(&f, &["check", "wsl-server"]);
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(
        last_stderr(&out),
        "Remote check failed: relay: The relay is missing on wsl-server. Run: agent-link remote setup wsl-server"
    );
}

#[test]
fn setup_rejects_bad_host() {
    let f = fake(true);
    let out = remote_cmd(&f, &["setup", "user@h"]);
    assert_eq!(out.status.code(), Some(2));
    assert_eq!(
        last_stderr(&out),
        "host must be an ssh host name (letters, digits, '.', '_', '-'; 1-63 chars): user@h"
    );
    assert!(!f.mac_home.join(".ssh").exists());
}
