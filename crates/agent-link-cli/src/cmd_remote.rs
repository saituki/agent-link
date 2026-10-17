//! `remote setup` / `remote check` — 遠隔の機械で claude を動かす準備と疎通の確認。
//!
//! 部品は `agent_link_engine::remote`。ここは手順（遠隔に置く・鍵を作る・Mac の
//! `~/.ssh/authorized_keys` を整える・確かめる）と出力だけを持つ。

use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::time::Duration;

use agent_link_engine::remote::{self, KeyChange, MAC_ALIAS, REMOTE_DIR};

use crate::cli::RemoteCmd;
use crate::cmd_common::{print_json, CmdError};

pub(crate) fn run(cmd: RemoteCmd) -> Result<(), CmdError> {
    match cmd {
        RemoteCmd::Setup { host, mac, json } => setup(&host, mac, json),
        RemoteCmd::Check { host, json } => check(&host, json),
    }
}

/// 確かめた 1 項目。
struct Check {
    name: &'static str,
    ok: bool,
    detail: String,
}

impl Check {
    fn to_json(&self) -> serde_json::Value {
        serde_json::json!({ "name": self.name, "ok": self.ok, "detail": self.detail })
    }
}

fn validate_host(host: &str) -> Result<(), CmdError> {
    if remote::is_valid_host(host) {
        Ok(())
    } else {
        Err(CmdError::input(format!(
            "host must be an ssh host name (letters, digits, '.', '_', '-'; 1-63 chars): {}",
            host
        )))
    }
}

fn remote_err(e: remote::RemoteError) -> CmdError {
    CmdError::other(e.0)
}

/// 4 項目を順に確かめる。前の項目が落ちたら、後ろは確かめない（落ちた項目が最後に入る）。
fn run_checks(host: &str) -> Vec<Check> {
    let short = Duration::from_secs(30);
    let mut checks: Vec<Check> = Vec::new();
    // 1 項目を確かめて足す。`probe` は Ok(Ok(detail)) = 通った・Ok(Err(detail)) = 落ちた。
    let mut step = |name: &'static str,
                    probe: &dyn Fn() -> Result<Result<String, String>, remote::RemoteError>|
     -> bool {
        let (ok, detail) = match probe() {
            Ok(Ok(d)) => (true, d),
            Ok(Err(d)) => (false, d),
            Err(e) => (false, e.0),
        };
        checks.push(Check { name, ok, detail });
        ok
    };

    let _ = step("mac_to_remote", &|| {
        let out = remote::ssh_run(host, "true", None, short)?;
        Ok(if out.status.success() {
            Ok("ssh works".to_string())
        } else {
            Err(format!(
                "ssh -o BatchMode=yes {} failed. Add IdentityFile for {} to ~/.ssh/config.",
                host, host
            ))
        })
    }) && step("claude", &|| {
        let out = remote::ssh_run(
            host,
            "PATH=\"$HOME/.local/bin:$PATH\" claude --version",
            None,
            short,
        )?;
        Ok(if out.status.success() {
            let text = String::from_utf8_lossy(&out.stdout);
            Ok(text.lines().next().unwrap_or("").trim().to_string())
        } else {
            Err(format!(
                "claude is not installed on {} (looked in PATH and ~/.local/bin).",
                host
            ))
        })
    }) && step("relay", &|| {
        let script = format!(
            "test -x \"$HOME/{d}/bin/agent-link\" && test -f \"$HOME/{d}/ssh_config\"",
            d = REMOTE_DIR
        );
        let out = remote::ssh_run(host, &script, None, short)?;
        Ok(if out.status.success() {
            Ok("relay is installed".to_string())
        } else {
            Err(format!(
                "The relay is missing on {}. Run: agent-link remote setup {}",
                host, host
            ))
        })
    }) && step("remote_to_mac", &|| {
        let script = format!("\"$HOME/{}/bin/agent-link\" remote-ping", REMOTE_DIR);
        let out = remote::ssh_run(host, &script, None, short)?;
        let stdout = String::from_utf8_lossy(&out.stdout);
        Ok(if out.status.success() && stdout.trim() == "agent-link-remote-ok" {
            Ok("agent-link-remote-ok".to_string())
        } else {
            Err(format!(
                "The remote cannot reach this Mac through the gate: {}. Check Remote Login and ~/.ssh/authorized_keys, or pass --mac to setup.",
                remote::last_line(&out).trim_end_matches('.')
            ))
        })
    });
    checks
}

/// 落ちた項目があれば、各項目を標準エラーに 1 行ずつ出し、最後の行を `Remote check failed: …` にした失敗を返す。
fn failure_of(checks: &[Check]) -> Option<CmdError> {
    let failed = checks.iter().find(|c| !c.ok)?;
    for c in checks {
        eprintln!("{} {}: {}", if c.ok { "ok  " } else { "FAIL" }, c.name, c.detail);
    }
    Some(CmdError::other(format!(
        "Remote check failed: {}: {}",
        failed.name, failed.detail
    )))
}

fn check(host: &str, json: bool) -> Result<(), CmdError> {
    validate_host(host)?;
    let checks = run_checks(host);
    if let Some(e) = failure_of(&checks) {
        return Err(e);
    }
    if json {
        print_json(&serde_json::json!({
            "host": host,
            "ok": true,
            "checks": checks.iter().map(Check::to_json).collect::<Vec<_>>(),
        }))
    } else {
        for c in &checks {
            println!("ok {}: {}", c.name, c.detail);
        }
        Ok(())
    }
}

fn setup(host: &str, mac: Option<String>, json: bool) -> Result<(), CmdError> {
    validate_host(host)?;
    let short = Duration::from_secs(30);
    let ssh_fail = |what: &str, out: &std::process::Output| {
        CmdError::other(format!("{} on {} failed: {}", what, host, remote::last_line(out)))
    };

    // 1. 遠隔の HOME と、遠隔から見たこの Mac の住所
    let out = remote::ssh_run(host, "printf '%s\\n' \"$HOME\" \"$SSH_CONNECTION\"", None, short)
        .map_err(remote_err)?;
    if !out.status.success() {
        return Err(ssh_fail("Reading $HOME", &out));
    }
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    let mut lines = text.lines();
    let home = lines.next().unwrap_or("").trim().to_string();
    if home.is_empty() {
        return Err(CmdError::other(format!("$HOME is empty on {}", host)));
    }
    let mac_addr = mac
        .map(|m| m.trim().to_string())
        .filter(|m| !m.is_empty())
        .or_else(|| {
            lines
                .next()
                .and_then(|l| l.split_whitespace().next())
                .map(str::to_string)
        })
        .ok_or_else(|| {
            CmdError::other(format!(
                "Cannot tell this Mac's address from {}. Pass --mac <addr>.",
                host
            ))
        })?;
    if !mac_addr
        .bytes()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'_' | b'-' | b':'))
    {
        return Err(CmdError::input(format!("--mac must be a host name or an IP address: {}", mac_addr)));
    }
    let mac_user = ["USER", "LOGNAME"]
        .iter()
        .find_map(|k| std::env::var(k).ok().filter(|v| !v.trim().is_empty()))
        .ok_or_else(|| CmdError::other("Cannot tell the user name of this Mac (USER is not set)."))?;

    // 2. 中継と ssh の設定を遠隔に置く
    let d = format!("\"$HOME/{}\"", REMOTE_DIR);
    let put = |name: &str, mode: &str, body: &str| -> Result<(), CmdError> {
        let script = format!(
            "umask 077 && mkdir -p {d}/bin && chmod 700 {d} && cat > {d}/{name}.tmp && chmod {mode} {d}/{name}.tmp && mv {d}/{name}.tmp {d}/{name}"
        );
        let out = remote::ssh_run(host, &script, Some(body.as_bytes()), short).map_err(remote_err)?;
        if out.status.success() {
            Ok(())
        } else {
            Err(ssh_fail("Writing the relay files", &out))
        }
    };
    put("bin/agent-link", "755", remote::shim_script())?;
    put("ssh_config", "600", &remote::ssh_config_text(&home, &mac_addr, &mac_user))?;

    // 3. 鍵が無ければ作り、公開鍵を読む
    let script = format!(
        "test -f {d}/id_ed25519 || ssh-keygen -q -t ed25519 -N '' -C 'agent-link-remote@{host}' -f {d}/id_ed25519 >&2; cat {d}/id_ed25519.pub"
    );
    let out = remote::ssh_run(host, &script, None, short).map_err(remote_err)?;
    let pubkey = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if !out.status.success() || !pubkey.starts_with("ssh-ed25519 ") {
        return Err(ssh_fail("Creating the relay key", &out));
    }

    // 4. この Mac の authorized_keys に門番の行を足す
    let exe = std::env::current_exe()
        .map_err(|e| CmdError::other(format!("Cannot find this executable: {}", e)))?
        .to_string_lossy()
        .to_string();
    if exe.contains(['"', '\n', '\r']) {
        return Err(CmdError::other(format!("The path of this executable cannot be used in authorized_keys: {}", exe)));
    }
    let change = upsert_mac_authorized_keys(&exe, &pubkey)?;

    // 5. 疎通を確かめる（authorized_keys に足した行は、落ちても残す）
    let checks = run_checks(host);
    if let Some(e) = failure_of(&checks) {
        return Err(e);
    }
    if json {
        print_json(&serde_json::json!({
            "host": host,
            "mac": mac_addr,
            "authorized_keys": change.as_str(),
            "checks": checks.iter().map(Check::to_json).collect::<Vec<_>>(),
            "ok": true,
        }))
    } else {
        println!("relay: installed on {} ({}/{}/bin/agent-link, alias {})", host, home, REMOTE_DIR, MAC_ALIAS);
        println!("mac: {} as {}", mac_addr, mac_user);
        println!("authorized_keys: {}", change.as_str());
        for c in &checks {
            println!("ok {}: {}", c.name, c.detail);
        }
        Ok(())
    }
}

/// `$HOME/.ssh/authorized_keys` を整える。`~/.ssh` は 0700・新しいファイルは 0600。
/// 書くときは一時ファイルから rename する（権限は元のまま）。
fn upsert_mac_authorized_keys(exe: &str, pubkey: &str) -> Result<KeyChange, CmdError> {
    let err = |what: &str, e: std::io::Error| CmdError::other(format!("{}: {}", what, e));
    let home = std::env::var_os("HOME")
        .filter(|h| !h.is_empty())
        .ok_or_else(|| CmdError::other("HOME is not set."))?;
    let ssh_dir = PathBuf::from(home).join(".ssh");
    if !ssh_dir.exists() {
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&ssh_dir)
            .map_err(|e| err("Cannot create ~/.ssh", e))?;
    }
    let path = ssh_dir.join("authorized_keys");
    // シンボリックリンクなら、リンク先を書き換える
    let target = std::fs::canonicalize(&path).unwrap_or(path);
    let (current, mode) = match std::fs::read_to_string(&target) {
        Ok(text) => {
            let mode = std::fs::metadata(&target)
                .map(|m| m.permissions().mode() & 0o777)
                .unwrap_or(0o600);
            (text, mode)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (String::new(), 0o600),
        Err(e) => return Err(err("Cannot read ~/.ssh/authorized_keys", e)),
    };
    let (next, change) = remote::upsert_authorized_key(&current, exe, pubkey);
    if change != KeyChange::Unchanged {
        write_replacing(&target, &next, mode).map_err(|e| err("Cannot write ~/.ssh/authorized_keys", e))?;
    }
    Ok(change)
}

/// 一時ファイルに書いて rename する。
fn write_replacing(target: &Path, text: &str, mode: u32) -> std::io::Result<()> {
    use std::io::Write;
    let mut tmp = target.as_os_str().to_owned();
    tmp.push(".agent-link.tmp");
    let tmp = PathBuf::from(tmp);
    let _ = std::fs::remove_file(&tmp);
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(mode)
        .open(&tmp)?;
    f.write_all(text.as_bytes())?;
    f.sync_all()?;
    drop(f);
    std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(mode))?;
    std::fs::rename(&tmp, target)
}
