//! `ext` グループ（拡張の導入・有効化・無効化・更新・CLI の起動）。
//!
//! 処理は `agent_link_engine::ext`。ここは引数を受けて呼び、JSON か人向けの文を出し、
//! 終了コードを決めるだけ（`ARCHITECTURE.md` §5）。

use std::io::Write;
use std::path::Path;

use agent_link_engine::ext::{self, ExtError, RUN_TIMEOUT};
use agent_link_engine::project;
use serde::Serialize;
use serde_json::json;

use crate::cli::ExtCmd;
use crate::cmd_common::{print_json, CmdError};

impl From<ExtError> for CmdError {
    /// 拡張の失敗を CLI の終了コードに読み替える（2・3・4・1）。文はそのまま最後の行に出す。
    fn from(e: ExtError) -> Self {
        match e {
            ExtError::Input(m) => CmdError::input(m),
            ExtError::NotFound(m) => CmdError::missing(m),
            ExtError::Conflict(m) => CmdError::conflict(m),
            ExtError::Other(m) => CmdError::other(m),
        }
    }
}

/// `agent-link ext <sub>` のハンドラ。
pub(crate) fn run(cmd: ExtCmd) -> Result<(), CmdError> {
    let ext_dir = ext::extensions_dir().ok_or_else(|| {
        CmdError::input(
            "Cannot find the extensions folder. Set AGENT_LINK_HOME or AGENT_LINK_EXTENSIONS_DIR.",
        )
    })?;
    let registry = project::sai_home_path("registry.json");
    let exe = current_exe_string();
    match cmd {
        ExtCmd::List { json } => list(&ext_dir, registry.as_deref(), json),
        ExtCmd::Install { dir, link, json } => install(&ext_dir, &dir, link, json),
        ExtCmd::Uninstall { name, json } => {
            ext::uninstall_in(&ext_dir, &name, registry.as_deref())?;
            if json {
                return print_json(&json!({"name": name, "removed": true}));
            }
            println!("Uninstalled {name}.");
            Ok(())
        }
        ExtCmd::Enable { name, json } => {
            let report = ext::enable_in(&ext_dir, &project::project_root(), &name, &exe)?;
            if json {
                return print_serialized(&report);
            }
            println!(
                "Enabled {name} in this project ({} files, init {}).",
                report.assets.len(),
                report.init
            );
            Ok(())
        }
        ExtCmd::Disable { name, json } => {
            let report = ext::disable_in(&ext_dir, &project::project_root(), &name)?;
            if json {
                return print_serialized(&report);
            }
            println!(
                "Disabled {name} in this project ({} removed, {} kept).",
                report.removed.len(),
                report.kept.len()
            );
            Ok(())
        }
        ExtCmd::Update { name, json } => {
            let reports =
                ext::update_in(&ext_dir, &project::project_root(), name.as_deref(), &exe)?;
            if json {
                return print_json(&json!({"updated": reports}));
            }
            if reports.is_empty() {
                println!("No enabled extensions to update.");
            }
            for r in &reports {
                println!(
                    "Updated {} ({} files, update {}).",
                    r.name,
                    r.assets.len(),
                    r.update
                );
            }
            Ok(())
        }
        // `--json` は受けるだけ。出力は拡張の CLI の物をそのまま流す
        ExtCmd::Run {
            name,
            args,
            json: _,
        } => run_cli(&ext_dir, &name, &args, &exe),
    }
}

/// `ext list`。`errors` は人向けのとき標準エラーに出す（終了コードは 0）。
fn list(ext_dir: &Path, registry: Option<&Path>, json: bool) -> Result<(), CmdError> {
    let result = ext::list_in(ext_dir, registry);
    if json {
        return print_json(&result.to_json(ext_dir));
    }
    for (path, error) in &result.errors {
        eprintln!("warning: {}: {}", path.display(), error);
    }
    if result.extensions.is_empty() {
        println!("No extensions in {}.", ext_dir.display());
    }
    for listed in &result.extensions {
        let m = &listed.installed.manifest;
        let state = if listed.enabled_in.is_empty() {
            "not enabled".to_string()
        } else {
            format!("enabled in: {}", listed.enabled_in.join(", "))
        };
        println!("{}  {}  {}", m.name, m.title, state);
    }
    Ok(())
}

/// `ext install`。
fn install(ext_dir: &Path, dir: &Path, link: bool, json: bool) -> Result<(), CmdError> {
    let installed = ext::install_in(ext_dir, dir, link)?;
    let name = &installed.manifest.name;
    if json {
        return print_json(&json!({
            "name": name,
            "path": installed.path.to_string_lossy(),
            "linked": installed.linked,
        }));
    }
    if link {
        println!("Linked {name} to {}.", dir.display());
    } else {
        println!("Installed {name} at {}.", installed.path.display());
    }
    Ok(())
}

/// `ext run`。成功なら子の標準出力・標準エラーをそのまま流す。失敗なら標準出力は空にし、
/// 子の終了コードが 2・3・4 ならそれ、ほかは 1 で終わる。
fn run_cli(ext_dir: &Path, name: &str, args: &[String], exe: &str) -> Result<(), CmdError> {
    let out = ext::run_cli(
        ext_dir,
        &project::project_root(),
        name,
        args,
        exe,
        RUN_TIMEOUT,
    )?;
    if out.code == 0 {
        write_raw(std::io::stdout(), &out.stdout)?;
        write_raw(std::io::stderr(), &out.stderr)?;
        return Ok(());
    }
    if out.stderr.is_empty() {
        eprintln!("Extension command failed with exit code {}.", out.code);
    } else {
        write_raw(std::io::stderr(), &out.stderr)?;
    }
    let code = if matches!(out.code, 2..=4) {
        out.code
    } else {
        1
    };
    std::process::exit(code);
}

/// 報告（`Serialize`）を JSON にして標準出力へ出す。
fn print_serialized<T: Serialize>(value: &T) -> Result<(), CmdError> {
    let value = serde_json::to_value(value).map_err(|e| CmdError::other(e.to_string()))?;
    print_json(&value)
}

fn write_raw(mut out: impl Write, bytes: &[u8]) -> Result<(), CmdError> {
    out.write_all(bytes)
        .and_then(|_| out.flush())
        .map_err(|e| CmdError::other(e.to_string()))
}

/// agent-link 自身の絶対パス（拡張の CLI には `AGENT_LINK_BIN` で渡す）。取れなければ名前だけ。
fn current_exe_string() -> String {
    std::env::current_exe()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| "agent-link".to_string())
}
