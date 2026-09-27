//! sai-paper CLI を呼び出して論文本文 / 引用箇所 / report を取得する。
//!
//! `task.md` の `paper_id` / `report_id` / `chunk_id` 入力を agent のプロンプトへ
//! 展開するときに使う。**papr.db を直接読まない**（core を唯一の真実とする）。
//!
//! sai-paper 側の `sai-paper content <kind> <id> --json` をサブプロセスで呼び、
//! `{ "id":..., "title":..., "text":... }` を受け取る。

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// sai-paper から取得した内容。
#[derive(Debug, Clone)]
pub struct SaiPaperContent {
    /// タイトル（論文タイトル / report タイトル / 引用箇所の論文タイトル）。
    pub title: String,
    /// 本文。
    pub text: String,
}

/// `sai-paper content <kind> <id>` を実行して内容を取得する。
///
/// # Errors
///
/// sai-paper が見つからない / 非ゼロ終了 / JSON 解析失敗の場合。
pub fn content(kind: &str, id: &str) -> Result<SaiPaperContent, String> {
    let bin = resolve_sai_paper_bin().ok_or_else(|| {
        "sai-paper バイナリが見つかりません（PATH 上に無い場合は SAI_PAPER_BIN を設定してください）"
            .to_string()
    })?;
    let output = Command::new(&bin)
        .arg("content")
        .arg(kind)
        .arg(id)
        .arg("--json")
        // worktree 実行時でも `.sai/paper/` をプロジェクトルート基準で解決できるよう、
        // 正規化済みのルートを明示的に渡す（既存値があれば尊重する）。
        .env(
            "SAI_PAPER_ROOT",
            std::env::var("SAI_PAPER_ROOT")
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|_| crate::project::project_root()),
        )
        .output()
        .map_err(|e| format!("sai-paper の実行に失敗: {e}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(if stderr.is_empty() {
            format!(
                "sai-paper content {kind} {id} が失敗しました (exit {:?})",
                output.status.code()
            )
        } else {
            stderr
        });
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let value: serde_json::Value = serde_json::from_str(stdout.trim())
        .map_err(|e| format!("sai-paper の JSON 出力を解析できません: {e}"))?;
    Ok(SaiPaperContent {
        title: value
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        text: value
            .get("text")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
    })
}

/// sai-paper の活動記録に「開いた」を残す（`sai-paper activity open`）。
///
/// sai-link-tui で成果物の md や review を開いたときに呼ぶ。sai-paper が無い
/// 環境でも画面を止めないため、次のように振る舞う。
///
/// - sai-paper の実行ファイルを解決できない場合は何もしない。
/// - `sai-paper activity open --path <絶対パス> --surface sai-link` を起動するだけで、
///   終了は待たない（標準入出力は捨てる）。
/// - 起動に失敗しても何もしない（ログも出さない）。
pub fn notify_open(path: &Path) {
    let Some(bin) = resolve_sai_paper_bin() else {
        return;
    };
    let abs_path = absolute_path(path);
    // worktree 実行時でも `.sai/paper/` をプロジェクトルート基準で解決できるよう、
    // `SAI_PAPER_ROOT` を明示的に渡す（既存値があれば尊重する）。
    let root = std::env::var("SAI_PAPER_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| crate::project::project_root());
    // 待たない・失敗しても無視する（`spawn` の Result は捨てる）。
    let _ = Command::new(&bin)
        .arg("activity")
        .arg("open")
        .arg("--path")
        .arg(&abs_path)
        .arg("--surface")
        .arg("sai-link")
        .env("SAI_PAPER_ROOT", root)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
}

/// 活動記録に渡す絶対パスを求める。
///
/// 絶対パスはそのまま返す（元ファイルが消えていても記録できるようにする）。
/// 相対パスはプロジェクトルート基準で絶対化する。
fn absolute_path(path: &Path) -> PathBuf {
    if path.is_absolute() {
        return path.to_path_buf();
    }
    crate::project::project_root().join(path)
}

/// `sai-moon-paper` バイナリを解決する。
///
/// 探索順:
/// 1. 環境変数 `SAI_PAPER_BIN`（設定されていればこれを唯一の候補にする）
/// 2. 実行ファイルと同じディレクトリ
/// 3. sai-moon プロジェクトルート配下の `target/{debug,release}/sai-moon-paper`
/// 4. `$PATH` 上の `sai-moon-paper`
#[must_use]
pub fn resolve_sai_paper_bin() -> Option<PathBuf> {
    if let Ok(bin) = std::env::var("SAI_PAPER_BIN") {
        let trimmed = bin.trim();
        // 明示指定がある場合は自動探索をしない。指定先が無いときは None を返し、
        // 別の sai-paper を勝手に使わない（テスト・切り替え時に確実に制御するため）。
        if !trimmed.is_empty() {
            let path = PathBuf::from(trimmed);
            return if path.is_file() { Some(path) } else { None };
        }
    }
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            candidates.push(dir.join("sai-moon-paper"));
            candidates.push(dir.join("sai-moon-paper.exe"));
        }
    }
    // sai-moon のプロジェクトルートと、その兄弟の sai-paper リポジトリを探す。
    let mut roots: Vec<PathBuf> = Vec::new();
    if let Ok(cwd) = std::env::current_dir() {
        roots.push(cwd);
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            roots.push(dir.to_path_buf());
        }
    }
    for base in roots {
        let mut probe = base.clone();
        for _ in 0..6 {
            for root in [
                probe.clone(),
                probe.join("sai-paper"),
                probe.parent().map(|p| p.join("sai-paper")).unwrap_or_default(),
            ] {
                for profile in ["debug", "release"] {
                    candidates.push(root.join("target").join(profile).join("sai-moon-paper"));
                }
            }
            if let Some(parent) = probe.parent() {
                probe = parent.to_path_buf();
            } else {
                break;
            }
        }
    }
    for candidate in candidates {
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    if let Ok(out) = Command::new("which").arg("sai-moon-paper").output() {
        if !out.status.success() {
            return None;
        }
        let path = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if !path.is_empty() {
            let path = PathBuf::from(path);
            if path.is_file() {
                return Some(path);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 実機（sai-paper の DB とバイナリがある環境）での手動確認用。
    ///
    /// `cargo test -p sai-link-engine -- --ignored sai_paper_content_smoke` で実行する。
    #[test]
    #[ignore = "sai-paper のバイナリと DB がある環境でのみ実行する"]
    fn sai_paper_content_smoke() {
        let content = content("paper", "6").expect("sai-paper content paper 6");
        assert!(!content.text.trim().is_empty());
        assert!(!content.title.trim().is_empty());
    }

    /// 環境変数を書き換えるテストを直列化する（プロセスグローバルなため）。
    fn env_lock() -> std::sync::MutexGuard<'static, ()> {
        crate::common::SAI_ENV_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner())
    }

    /// 実行可能なシェルスクリプトを一時ファイルとして作る。
    fn write_script(dir: &Path, body: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join("sai-moon-paper");
        std::fs::write(&path, body).unwrap();
        let mut perms = std::fs::metadata(&path).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&path, perms).unwrap();
        path
    }

    /// ファイルが書かれるまで少し待って読む（別プロセスの書き込み待ち）。
    fn read_until(path: &Path, secs: u64) -> String {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(secs);
        loop {
            if let Ok(text) = std::fs::read_to_string(path) {
                if !text.trim().is_empty() {
                    return text;
                }
            }
            assert!(
                std::time::Instant::now() < deadline,
                "{} が {} 秒以内に書かれませんでした",
                path.display(),
                secs
            );
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }

    #[test]
    fn resolve_bin_prefers_env_override() {
        let _guard = env_lock();
        let dir = std::env::temp_dir();
        let fake = dir.join(format!("sai-paper-fake-{}", std::process::id()));
        std::fs::write(&fake, b"#!/bin/sh\n").unwrap();
        std::env::set_var("SAI_PAPER_BIN", fake.to_string_lossy().to_string());
        assert_eq!(resolve_sai_paper_bin(), Some(fake.clone()));
        std::env::remove_var("SAI_PAPER_BIN");
        let _ = std::fs::remove_file(fake);
    }

    /// 明示指定が無効（存在しないパス）なら自動探索へ落ちず None になること。
    ///
    /// 別の sai-paper（開発機のビルド）を勝手に使わないための担保。
    #[test]
    fn resolve_bin_missing_env_override_is_none() {
        let _guard = env_lock();
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var(
            "SAI_PAPER_BIN",
            dir.path().join("no-such-sai-paper").to_string_lossy().to_string(),
        );
        assert!(resolve_sai_paper_bin().is_none());
        std::env::remove_var("SAI_PAPER_BIN");
    }

    /// `notify_open` が `activity open --path <絶対パス> --surface sai-link` で呼ぶこと。
    #[test]
    fn notify_open_calls_activity_open_with_expected_args() {
        let _guard = env_lock();
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("args.txt");
        let script = write_script(
            dir.path(),
            &format!("#!/bin/sh\nprintf '%s\\n' \"$@\" > '{}'\n", out.display()),
        );
        let md = dir.path().join("report.md");
        std::fs::write(&md, "# report\n").unwrap();

        std::env::set_var("SAI_PAPER_BIN", script.to_string_lossy().to_string());
        notify_open(&md);
        let text = read_until(&out, 5);
        std::env::remove_var("SAI_PAPER_BIN");

        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.first().copied(), Some("activity"), "args: {:?}", lines);
        assert_eq!(lines.get(1).copied(), Some("open"));
        assert_eq!(lines.get(2).copied(), Some("--path"));
        assert_eq!(lines.get(3).copied(), Some(md.to_string_lossy().as_ref()));
        assert_eq!(lines.get(4).copied(), Some("--surface"));
        assert_eq!(lines.get(5).copied(), Some("sai-link"));
    }

    /// sai-paper が遅くても `notify_open` は待たずに戻ること。
    #[test]
    fn notify_open_does_not_wait_for_sai_paper() {
        let _guard = env_lock();
        let dir = tempfile::tempdir().unwrap();
        let script = write_script(dir.path(), "#!/bin/sh\nsleep 10\n");
        std::env::set_var("SAI_PAPER_BIN", script.to_string_lossy().to_string());
        let start = std::time::Instant::now();
        notify_open(&dir.path().join("report.md"));
        let elapsed = start.elapsed();
        std::env::remove_var("SAI_PAPER_BIN");
        assert!(
            elapsed < std::time::Duration::from_secs(5),
            "終了を待たない（10 秒 sleep してもすぐ戻る）: {elapsed:?}"
        );
    }

    /// sai-paper が見つからないときは何も起動せず、すぐ戻ること（パニックしない）。
    #[test]
    fn notify_open_does_nothing_when_bin_missing() {
        let _guard = env_lock();
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var(
            "SAI_PAPER_BIN",
            dir.path().join("no-such-sai-paper").to_string_lossy().to_string(),
        );
        let start = std::time::Instant::now();
        notify_open(&dir.path().join("report.md"));
        let elapsed = start.elapsed();
        std::env::remove_var("SAI_PAPER_BIN");
        assert!(elapsed < std::time::Duration::from_secs(1));
    }
}
