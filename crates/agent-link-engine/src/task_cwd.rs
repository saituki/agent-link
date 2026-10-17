//! # タスク作業フォルダ（cwd）の解決
//!
//! `task_create` で親エージェントが指定できる `cwd`（サブタスクの作業フォルダ）を
//! 絶対パスに解決し、存在確認を行う。
//!
//! 受け付ける形式:
//! - 絶対パス（例: `/home/user/workdir/other-project`）
//! - `~/` 始まり（例: `~/workdir/Sai/sai-phone`）
//! - プロジェクトルートからの相対パス（例: `../sai-phone`）
//!
//! 存在しない・フォルダでない場合はエラーメッセージを返す。呼び出し元
//! （`agent_api::task_create`）はこれを `validation_errors` に変換し、タスクを作らない。

use std::path::{Component, Path, PathBuf};

/// `cwd` 指定文字列を絶対パスに解決し、存在確認する。
///
/// `project_root` は相対パスの基準ディレクトリ。`~/` は `HOME` 環境変数で展開する。
pub fn resolve_task_cwd(raw: &str, project_root: &Path) -> Result<PathBuf, String> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    resolve_task_cwd_with_home(raw, project_root, home.as_deref())
}

/// `resolve_task_cwd` の本体（`HOME` を明示的に受け取る版。テスト用）。
pub fn resolve_task_cwd_with_home(
    raw: &str,
    project_root: &Path,
    home: Option<&Path>,
) -> Result<PathBuf, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(
            "cwd is empty. Give an absolute path, a path starting with ~/, or a path relative to the project root."
                .to_string(),
        );
    }

    let expanded = expand_tilde(trimmed, home)
        .ok_or_else(|| format!("Cannot resolve `~` in cwd because HOME is not set: {}", trimmed))?;

    let absolute = if expanded.is_absolute() {
        expanded
    } else {
        project_root.join(expanded)
    };

    let normalized = normalize_path(&absolute);
    if !normalized.exists() {
        return Err(format!("cwd does not exist: {}", normalized.display()));
    }
    if !normalized.is_dir() {
        return Err(format!(
            "cwd is not a folder: {}",
            normalized.display()
        ));
    }
    Ok(normalized)
}

/// `~` / `~/` を `home` で展開する。`~` を使わない場合はそのまま返す。
fn expand_tilde(s: &str, home: Option<&Path>) -> Option<PathBuf> {
    if s == "~" {
        return home.map(PathBuf::from);
    }
    if let Some(rest) = s.strip_prefix("~/") {
        return home.map(|h| h.join(rest));
    }
    Some(PathBuf::from(s))
}

/// `.` / `..` を字句的に畳む（ファイルシステムには触れない）。
///
/// `canonicalize` はシンボリックリンクを解決してしまう（macOS の `/tmp` →
/// `/private/tmp` 等）ため、表示・保存用には字句的な正規化を使う。
pub fn normalize_path(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for comp in p.components() {
        match comp {
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() && !p.is_absolute() {
                    out.push("..");
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    if out.as_os_str().is_empty() {
        PathBuf::from(".")
    } else {
        out
    }
}

/// `host` 付きの子の `cwd`（遠隔の機械のフォルダ）を検査する。存在は見ない。
///
/// `/` か `~/` で始まり、`'`・改行・NUL・`..` の要素を含まなければ、trim した値を返す。
/// 展開は起動のときに遠隔の `$HOME` で行う。
pub fn check_remote_cwd(raw: &str) -> Result<String, String> {
    let t = raw.trim();
    let ok = (t.starts_with('/') || t.starts_with("~/"))
        && !t.contains(['\'', '\n', '\r', '\0'])
        && !t.split('/').any(|c| c == "..");
    if ok {
        Ok(t.to_string())
    } else {
        Err(format!(
            "cwd with host must be an absolute path or start with ~/ on the remote machine: {}",
            raw
        ))
    }
}

// ============================================================================
// テスト
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// 一時ディレクトリを作る（依存クレートを増やさない最小実装）。
    fn temp_dir(tag: &str) -> PathBuf {
        let base = std::env::temp_dir().join(format!(
            "agent-link-task-cwd-{}-{}-{}",
            std::process::id(),
            tag,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        fs::create_dir_all(&base).unwrap();
        base
    }

    #[test]
    fn resolve_absolute_path() {
        let dir = temp_dir("abs");
        let resolved = resolve_task_cwd(dir.to_str().unwrap(), Path::new("/")).unwrap();
        assert_eq!(resolved, dir);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn resolve_relative_to_project_root() {
        let root = temp_dir("relroot");
        let sub = root.join("sub/dir");
        fs::create_dir_all(&sub).unwrap();
        let resolved = resolve_task_cwd("sub/dir", &root).unwrap();
        assert_eq!(resolved, sub);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn resolve_relative_with_dotdot() {
        let root = temp_dir("dotdot");
        let sibling = root.parent().unwrap().join(format!(
            "{}-sibling",
            root.file_name().unwrap().to_string_lossy()
        ));
        fs::create_dir_all(&sibling).unwrap();
        let resolved = resolve_task_cwd(
            &format!("../{}", sibling.file_name().unwrap().to_string_lossy()),
            &root,
        )
        .unwrap();
        assert_eq!(resolved, sibling);
        let _ = fs::remove_dir_all(&root);
        let _ = fs::remove_dir_all(&sibling);
    }

    #[test]
    fn resolve_tilde_start() {
        // HOME 環境変数を触らず、明示的な home で検証する（並列テストとの干渉回避）。
        let home = temp_dir("home");
        let target = home.join("workdir/project");
        fs::create_dir_all(&target).unwrap();
        let resolved =
            resolve_task_cwd_with_home("~/workdir/project", Path::new("/"), Some(&home)).unwrap();
        assert_eq!(resolved, target);

        // 裸の `~`（HOME 未設定ならエラー）
        assert_eq!(
            resolve_task_cwd_with_home("~", Path::new("/"), Some(&home)).unwrap(),
            home
        );
        assert!(resolve_task_cwd_with_home("~/x", Path::new("/"), None).is_err());
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn resolve_missing_path_is_error() {
        let root = temp_dir("missing");
        let err = resolve_task_cwd("no-such-folder-xyz", &root).unwrap_err();
        assert!(err.contains("does not exist"), "実際: {}", err);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn resolve_file_is_error() {
        let root = temp_dir("file");
        let file = root.join("a.txt");
        fs::write(&file, "x").unwrap();
        let err = resolve_task_cwd(file.to_str().unwrap(), &root).unwrap_err();
        assert!(err.contains("is not a folder"), "実際: {}", err);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn resolve_empty_is_error() {
        let err = resolve_task_cwd("   ", Path::new("/")).unwrap_err();
        assert!(err.contains("empty"), "実際: {}", err);
    }

    #[test]
    fn normalize_collapses_components() {
        assert_eq!(
            normalize_path(Path::new("/a/b/./c/../d")),
            PathBuf::from("/a/b/d")
        );
    }

    #[test]
    fn remote_cwd_accepts_absolute_and_tilde() {
        assert_eq!(check_remote_cwd(" ~/work/sai-tree ").unwrap(), "~/work/sai-tree");
        assert_eq!(check_remote_cwd("/home/u/a..b").unwrap(), "/home/u/a..b");
    }

    #[test]
    fn remote_cwd_rejects_bad_shapes() {
        for bad in ["", "relative/path", "~", "~user/x", "/a/../b", "~/..", "/a'b", "/a\nb", "/a\rb", "/a\0b"] {
            let err = check_remote_cwd(bad).unwrap_err();
            assert!(err.starts_with("cwd with host must be an absolute path"), "{:?}: {}", bad, err);
        }
    }
}
