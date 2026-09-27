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
            "cwd が空です。絶対パス・~/ 始まり・プロジェクトルートからの相対パスを指定してください。"
                .to_string(),
        );
    }

    let expanded = expand_tilde(trimmed, home)
        .ok_or_else(|| format!("cwd の `~` を解決できません（HOME 未設定）: {}", trimmed))?;

    let absolute = if expanded.is_absolute() {
        expanded
    } else {
        project_root.join(expanded)
    };

    let normalized = normalize_path(&absolute);
    if !normalized.exists() {
        return Err(format!("cwd が存在しません: {}", normalized.display()));
    }
    if !normalized.is_dir() {
        return Err(format!(
            "cwd がフォルダではありません: {}",
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
            "sai-link-task-cwd-{}-{}-{}",
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
        assert!(err.contains("存在しません"), "実際: {}", err);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn resolve_file_is_error() {
        let root = temp_dir("file");
        let file = root.join("a.txt");
        fs::write(&file, "x").unwrap();
        let err = resolve_task_cwd(file.to_str().unwrap(), &root).unwrap_err();
        assert!(err.contains("フォルダではありません"), "実際: {}", err);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn resolve_empty_is_error() {
        let err = resolve_task_cwd("   ", Path::new("/")).unwrap_err();
        assert!(err.contains("空"), "実際: {}", err);
    }

    #[test]
    fn normalize_collapses_components() {
        assert_eq!(
            normalize_path(Path::new("/a/b/./c/../d")),
            PathBuf::from("/a/b/d")
        );
    }
}
