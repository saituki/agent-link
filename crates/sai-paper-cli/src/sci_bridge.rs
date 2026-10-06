//! 古い研究の命令を `sai-sci` に渡す橋（ガイド 4 の U19 で消す一時のもの）。
//!
//! sai-paper は sai-sci に依存しない（要件定義書 A8）。古い命令名（`sci …`・`brief`・
//! `phone sci …`・MCP の `sci_*`）は、動いている daemon・Web・エージェント・各プロジェクトの
//! 設定が切り替えるまで、`sai-sci` の実行ファイルを呼ぶ隠した別名として残す。
//!
//! 呼び方は 2 つ:
//!
//! - [`exec`]: そのまま置き換える（標準入力・出力・エラー・環境・終了コードを引き継ぐ）。
//! - [`run_json`] / [`run_init_tree`]: 子として呼び、出力を受け取って形を整える（`phone …`）。

use std::io::Write as _;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use serde_json::Value;

/// `sai-sci` が見つからないときの文。
pub(crate) const MISSING: &str = "sai-sci が見つかりません（SAI_SCI_BIN か PATH を確かめてください）";

/// 子が失敗したときの、標準エラーの無いときの文。
const FAILED: &str = "sai-sci が失敗しました";

/// `sai-sci` の実行ファイルを探す。
///
/// 環境変数 `SAI_SCI_BIN`（空でない値）→ 自分の実行ファイルと同じフォルダの `sai-sci` →
/// 名前だけ `sai-sci`（PATH）の順。
pub(crate) fn bin() -> PathBuf {
    if let Ok(value) = std::env::var("SAI_SCI_BIN") {
        let value = value.trim();
        if !value.is_empty() {
            return PathBuf::from(value);
        }
    }
    if let Ok(exe) = std::env::current_exe().and_then(std::fs::canonicalize)
        && let Some(dir) = exe.parent()
    {
        let candidate = dir.join("sai-sci");
        if candidate.is_file() {
            return candidate;
        }
    }
    PathBuf::from("sai-sci")
}

/// `sai-link` の実行 ID（`SAI_PARENT_RUN_ID`。空・数でないときは `None`）。
pub(crate) fn parent_run_id() -> Option<i64> {
    std::env::var("SAI_PARENT_RUN_ID")
        .ok()
        .and_then(|text| text.trim().parse::<i64>().ok())
}

/// `sai-sci <args>` に置き換える（`exec`）。
///
/// 標準入力・出力・エラー・環境・終了コードはそのまま引き継がれる。起動できなければ
/// 標準エラーに [`MISSING`] を出し、`missing_code` で終わる（`report prepare` だけ 2。
/// sai-link の `write_check` が 1 を「断った」と読んで保存を止めてしまうため）。
pub(crate) fn exec(args: &[String], missing_code: i32) -> ! {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt as _;
        let error = Command::new(bin()).args(args).exec();
        let _ = error;
        eprintln!("{MISSING}");
        std::process::exit(missing_code);
    }
    #[cfg(not(unix))]
    {
        match Command::new(bin()).args(args).status() {
            Ok(status) => std::process::exit(status.code().unwrap_or(1)),
            Err(_) => {
                eprintln!("{MISSING}");
                std::process::exit(missing_code);
            }
        }
    }
}

/// 標準エラーの、空でない最後の行。
fn last_stderr_line(stderr: &[u8]) -> Option<String> {
    String::from_utf8_lossy(stderr)
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .next_back()
        .map(str::to_string)
}

/// 子として `sai-sci` を呼び、標準出力を返す。
///
/// 失敗（起動できない・終了コードが 0 でない）は、日本語 1 行の `Err`。
/// `stdin` が `Some` のときだけ子の標準入力へ渡す（渡さないときは閉じる）。
pub(crate) fn capture(args: &[String], stdin: Option<&str>) -> Result<String, String> {
    let stdin_kind = if stdin.is_some() { Stdio::piped() } else { Stdio::null() };
    let mut child = Command::new(bin())
        .args(args)
        .stdin(stdin_kind)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|_| MISSING.to_string())?;
    if let Some(text) = stdin
        && let Some(mut pipe) = child.stdin.take()
    {
        let _ = pipe.write_all(text.as_bytes());
    }
    let output = child.wait_with_output().map_err(|_| FAILED.to_string())?;
    if output.status.success() {
        return Ok(String::from_utf8_lossy(&output.stdout).into_owned());
    }
    Err(last_stderr_line(&output.stderr).unwrap_or_else(|| FAILED.to_string()))
}

/// 子として `sai-sci` を呼び、標準出力を JSON として読む。
pub(crate) fn capture_json(args: &[String], stdin: Option<&str>) -> Result<Value, String> {
    let text = capture(args, stdin)?;
    serde_json::from_str(&text).map_err(|_| "sai-sci の JSON を読めません".to_string())
}

/// 子の標準出力をそのまま出す（形 A）。失敗は `{"error": …}` を出して終了コード 1。
pub(crate) fn run_json(args: &[String], stdin: Option<&str>) -> ! {
    match capture(args, stdin) {
        Ok(stdout) => exit_with(0, &stdout),
        Err(message) => exit_with(1, &format!("{}\n", serde_json::json!({ "error": message }))),
    }
}

/// 子として呼び、JSON の `tree` だけを出し直す（`sci init --json` と `phone sci init`）。
pub(crate) fn run_init_tree(args: &[String]) -> ! {
    match capture_json(args, None) {
        Ok(value) => {
            let tree = value.get("tree").cloned().unwrap_or(Value::Null);
            exit_with(0, &format!("{tree}\n"));
        }
        Err(message) => exit_with(1, &format!("{}\n", serde_json::json!({ "error": message }))),
    }
}

/// 標準出力へ書いてから、終了コードを付けて終わる（`process::exit` は溜めを流さないため）。
fn exit_with(code: i32, text: &str) -> ! {
    let mut out = std::io::stdout();
    let _ = out.write_all(text.as_bytes());
    let _ = out.flush();
    std::process::exit(code);
}
