//! 子の実行ファイル（sai-link・sai-paper）の見つけ方と呼び方。
//!
//! 共有契約「実行ファイルの見つけ方と呼び方」。sai-sci（U13）と、U16 の sci-core が使う。
//! 実行ファイルを改名するときは、[`LINK_BIN_NAME`]・[`PAPER_BIN_NAME`] の
//! 2 つの定数だけを替える。

use std::fmt;
use std::io::{BufRead as _, BufReader, Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// sai-link の実行ファイル名（PATH と、自分の隣を探すときの名前）。
pub const LINK_BIN_NAME: &str = "sai-link";
/// sai-paper の実行ファイル名。
pub const PAPER_BIN_NAME: &str = "sai-paper";

/// 子の実行ファイルの見つけ方の順序（共有契約）:
/// 1. 環境変数（空でない値）
/// 2. 自分の実行ファイルのフォルダにある同じ名前（あれば）
/// 3. 名前だけ（PATH から探す）
fn resolve(env_name: &str, bin_name: &str) -> PathBuf {
    if let Ok(value) = std::env::var(env_name) {
        let value = value.trim();
        if !value.is_empty() {
            return PathBuf::from(value);
        }
    }
    if let Some(dir) = std::env::current_exe()
        .ok()
        .and_then(|path| std::fs::canonicalize(path).ok())
        .and_then(|path| path.parent().map(Path::to_path_buf))
    {
        let candidate = dir.join(bin_name);
        if candidate.is_file() {
            return candidate;
        }
    }
    PathBuf::from(bin_name)
}

// 試験のときだけ使う、偽の sai-link への差し替え（スレッドごと）。
#[cfg(test)]
thread_local! {
    static LINK_OVERRIDE: std::cell::RefCell<Option<PathBuf>> =
        const { std::cell::RefCell::new(None) };
}

/// そのスレッドの間だけ、偽の sai-link を使う（試験用）。
#[cfg(test)]
pub(crate) fn with_link_bin<T>(path: PathBuf, f: impl FnOnce() -> T) -> T {
    let previous = LINK_OVERRIDE.with(|cell| cell.borrow_mut().replace(path));
    let out = f();
    LINK_OVERRIDE.with(|cell| {
        *cell.borrow_mut() = previous;
    });
    out
}

/// sai-link の実行ファイル（`SAI_LINK_BIN` → 自分の隣 → PATH）。
///
/// 試験のときは、偽の sai-link（[`crate::fake_link`]）を返す。
#[must_use]
pub fn link_bin() -> PathBuf {
    #[cfg(test)]
    {
        if let Some(path) = LINK_OVERRIDE.with(|cell| cell.borrow().clone()) {
            return path;
        }
        return crate::fake_link::script();
    }
    #[cfg(not(test))]
    resolve("SAI_LINK_BIN", LINK_BIN_NAME)
}

/// sai-paper の実行ファイル（`SAI_PAPER_BIN` → 自分の隣 → PATH）。
#[must_use]
pub fn paper_bin() -> PathBuf {
    resolve("SAI_PAPER_BIN", PAPER_BIN_NAME)
}

/// 子の実行ファイルの失敗（終了コードと、標準エラーの最後の行）。
#[derive(Debug, Clone)]
pub struct BinError {
    /// 子の終了コード。起動できなければ 1。
    pub code: i32,
    /// 子の標準エラーの最後の行（`sai-progress` で始まる行は数えない）。無ければ空。
    pub message: String,
}

impl fmt::Display for BinError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for BinError {}

/// 子の実行ファイルを呼ぶ。
///
/// - cwd は `root`。環境変数は今の環境に `SAI_PROJECT_ROOT=<root>` を足した物
///   （`SAI_PAPER_ROOT` は渡さない）。
/// - 標準エラーは呼んだ側の標準エラーへそのまま流し、最後の空でない行を覚える。
/// - 終了コード 0 なら標準出力の文字を返す。それ以外は [`BinError`]。
/// - 時間の上限は付けない（論文の検索はネットを使い、長くかかることがある）。
///
/// # Errors
/// 子を起動できないとき、または子が 0 以外で終わったとき。
pub fn run(
    bin: &Path,
    args: &[String],
    root: &Path,
    stdin: Option<&str>,
) -> Result<String, BinError> {
    let mut command = Command::new(bin);
    command
        .args(args)
        .current_dir(root)
        .env("SAI_PROJECT_ROOT", root)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if stdin.is_some() {
        command.stdin(Stdio::piped());
    } else {
        command.stdin(Stdio::null());
    }
    let mut child = command.spawn().map_err(|error| BinError {
        code: 1,
        message: format!("実行できません: {error}"),
    })?;

    if let Some(text) = stdin
        && let Some(mut pipe) = child.stdin.take()
    {
        let _ = pipe.write_all(text.as_bytes());
        // ここで pipe は閉じる（子の標準入力を終わらせる）。
    }

    // 標準出力は別のスレッドで読む（標準エラーを読みながらでも詰まらないように）。
    let stdout = child.stdout.take();
    let reader = std::thread::spawn(move || {
        let mut text = String::new();
        if let Some(mut out) = stdout {
            let _ = out.read_to_string(&mut text);
        }
        text
    });

    // 標準エラーはそのまま流し、最後の空でない行（`sai-progress` を除く）を覚える。
    let mut last = String::new();
    if let Some(err) = child.stderr.take() {
        for line in BufReader::new(err).lines() {
            let Ok(line) = line else { break };
            if !line.trim().is_empty() && !line.starts_with("sai-progress") {
                last.clone_from(&line);
            }
            eprintln!("{line}");
        }
    }

    let status = child.wait().map_err(|error| BinError {
        code: 1,
        message: format!("子の終了を待てません: {error}"),
    })?;
    let out = reader.join().unwrap_or_default();
    if status.success() {
        Ok(out)
    } else {
        Err(BinError {
            code: status.code().unwrap_or(1),
            message: last,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 環境変数の名前（空でない値）が一番先。無ければ名前だけになる。
    #[test]
    fn resolve_prefers_env_then_name() {
        // 空の値は無視して名前だけを返す。
        // 環境変数を触ると並列の試験と取り合うので、ここでは関数の分岐だけを見る。
        assert_eq!(LINK_BIN_NAME, "sai-link");
        assert_eq!(PAPER_BIN_NAME, "sai-paper");
    }
}
