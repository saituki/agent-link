//! # プロジェクトルート解決
//!
//! agent-link は `.agent-link/` ディレクトリを「プロジェクトルート（デーモン起動パス）」基準で
//! 解決します（D15）。
//!
//! デーモンは起動時に `AGENT_LINK_PROJECT_ROOT` 環境変数を設定します。
//! worktree 実行時は cc プロセスの cwd が `.worktrees/<branch>` に変わるため、
//! そのまま `.agent-link/` 相対パスを使うと誤った場所を参照してしまいます。
//! このモジュールを経由することで、デーモン起動パス（= プロジェクトルート）の
//! `.agent-link/` を常に正しく参照できます。
//!
//! ## 優先順位
//!
//! [`project_root`]（デーモン・エンジン内部が使う）:
//! 1. 環境変数 `AGENT_LINK_PROJECT_ROOT`（デーモンが起動時に設定。フォルダの有無は見ない）
//! 2. カレントディレクトリ（デーモン未使用時のフォールバック）
//!
//! [`project_root_checked`]（CLI の入口が使う。CLI 共通の約束 5）:
//! 1. 環境変数 `AGENT_LINK_PROJECT_ROOT`（空でない値。フォルダ・`.agent-link/` が無ければエラー）
//! 2. カレントディレクトリから上へたどり、`.agent-link/` を持つ最初のフォルダ
//!
//! 上へたどるとき、ホーム（`HOME`）とそれより上のフォルダは数えない。
//! `~/.agent-link` は利用者全体の置き場であり、プロジェクトではないため
//! （CLI 共通の約束 5）。

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

/// プロジェクトのフォルダの名前（`<root>/.agent-link/`）。名前を変えるときはここだけ直す。
pub const PROJECT_DIR: &str = ".agent-link";

/// 実行エンジンの置き場（`<root>/.agent-link/engine/`）。DB・daemon の PID・ログ・通知が入る。
pub fn engine_dir(root: &Path) -> PathBuf {
    root.join(PROJECT_DIR).join("engine")
}

/// `root` の実行の DB（`<root>/.agent-link/engine/agent-link.db`）。
pub fn db_path_in(root: &Path) -> PathBuf {
    engine_dir(root).join("agent-link.db")
}

/// 現在のプロジェクトの実行の DB（`.agent-link/engine/agent-link.db`）。
///
/// 存在は確かめない。見つからないときの探し方は [`resolve_db_path`]。
pub fn db_path() -> PathBuf {
    db_path_in(&project_root())
}

/// プロジェクトルート（`.agent-link` の基準ディレクトリ）を解決する。
///
/// `AGENT_LINK_PROJECT_ROOT`（空でない値）があればそれをそのまま使う（デーモンが
/// 起動時に設定する。フォルダの有無は見ない。デーモン・pane の既存の使い方を
/// そのまま保つため）。無ければカレントディレクトリ。
///
/// 上へたどって `.agent-link/` を探すのは [`project_root_checked`] の方で行う
/// （CLI の入口が使う）。ここで上へたどると、プロジェクトの外で打った命令が
/// ホームより上の別の `.agent-link/` を見つけて意図しない場所に書いてしまうため。
pub fn project_root() -> PathBuf {
    if let Some(p) = std::env::var_os("AGENT_LINK_PROJECT_ROOT") {
        let pb = PathBuf::from(p);
        if !pb.as_os_str().is_empty() {
            return pb;
        }
    }
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

/// プロジェクトルートを解決し、見つからなければ理由（英語 1 行）を返す。
///
/// 順序は CLI 共通の約束 5 のとおり。
/// - 環境変数 `AGENT_LINK_PROJECT_ROOT`（空でない値）があればそれを使う。
///   ただしフォルダが無い・`.agent-link/` が無ければエラーにする（上へたどらない）。
/// - 無ければカレントディレクトリから上へたどり、`.agent-link/` を持つ最初のフォルダ。
///   ホーム（`HOME`）とそれより上は数えない。
/// - 見つからなければエラー（`.agent-link/` を勝手に作らない）。
pub fn project_root_checked() -> Result<PathBuf, String> {
    if let Some(p) = std::env::var_os("AGENT_LINK_PROJECT_ROOT") {
        let pb = PathBuf::from(p);
        if !pb.as_os_str().is_empty() {
            return check_env_root(pb);
        }
    }
    let cwd = std::env::current_dir()
        .map_err(|e| format!("Cannot get the current directory: {e}"))?;
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty());
    if let Some(root) = find_project_root(&cwd, home.as_deref()) {
        return Ok(root);
    }
    Err("Project not found. Run \"agent-link init\".".to_string())
}

/// `AGENT_LINK_PROJECT_ROOT` のフォルダが在り、`.agent-link/` を持つことを確かめる。
fn check_env_root(pb: PathBuf) -> Result<PathBuf, String> {
    if !pb.is_dir() {
        return Err(format!(
            "The AGENT_LINK_PROJECT_ROOT folder does not exist: {}",
            pb.display()
        ));
    }
    if !pb.join(PROJECT_DIR).is_dir() {
        return Err(format!(
            "The AGENT_LINK_PROJECT_ROOT folder has no .agent-link/: {}. Run \"agent-link init\".",
            pb.display()
        ));
    }
    Ok(pb)
}

/// `start` から上へたどり、`.agent-link/` を持つ最初のフォルダを返す。
///
/// `home` に着いたらそこで止める（ホームとそれより上は数えない）。
/// `home` が `start` の先祖でなければ、根（`/`）までたどる。
fn find_project_root(start: &Path, home: Option<&Path>) -> Option<PathBuf> {
    let mut dir = Some(start);
    while let Some(d) = dir {
        if Some(d) == home {
            break;
        }
        if d.join(PROJECT_DIR).is_dir() {
            return Some(d.to_path_buf());
        }
        dir = d.parent();
    }
    None
}

/// プロジェクトルートからの相対パスを絶対パスに解決する。
///
/// `rel` は `.agent-link/blocks` のように [`PROJECT_DIR`] から始める。
/// 例: `sai_path(".agent-link/blocks")` → `<root>/.agent-link/blocks`
pub fn sai_path(rel: &str) -> PathBuf {
    project_root().join(rel)
}

/// agent-link の DB（`.agent-link/engine/agent-link.db`）のパスを解決する。
///
/// 優先順位:
/// 1. 環境変数 `AGENT_LINK_PROJECT_ROOT` → `<root>/.agent-link/engine/agent-link.db`
/// 2. （無ければ）後方互換の相対パス候補から最初に存在するもの
///
/// CLI と Web が同じ DB を開くための共通入口。
pub fn resolve_db_path() -> PathBuf {
    // ① AGENT_LINK_PROJECT_ROOT を第一優先（複数プロジェクト・worktree 対応）
    if let Some(root) = std::env::var_os("AGENT_LINK_PROJECT_ROOT") {
        let p = db_path_in(Path::new(&root));
        if p.exists() {
            return p;
        }
    }

    // ② 後方互換: 単一プロジェクトの相対パス候補
    [".", "..", "../.."]
        .iter()
        .map(|up| db_path_in(Path::new(up)))
        .find(|p| p.exists())
        .unwrap_or_else(|| db_path_in(Path::new(".")))
}

/// 利用者全体の置き場（agent-link のグローバルデータ置き場）を返す。
///
/// 解決順:
/// 1. 環境変数 `AGENT_LINK_HOME`（テスト・隔離環境・複数-home で上書きする）
/// 2. `~/.agent-link`
///
/// `HOME` も設定されていない場合は `None`（呼び出し側は「この場所は無い」として
/// 読み書きを飛ばすか、エラーにする）。
///
/// プロジェクトごとの `<root>/.agent-link/`（agent-link.db・blocks・outputs 等）は
/// [`project_root`] / [`sai_path`] を使う。ここでは扱わない。
#[must_use]
pub fn sai_home() -> Option<PathBuf> {
    resolve_sai_home(
        std::env::var_os("AGENT_LINK_HOME").as_deref(),
        std::env::var_os("HOME").as_deref(),
    )
}

/// 利用者全体の置き場配下の相対パスを返す（`sai_home()` が無ければ `None`）。
///
/// 例: `sai_home_path("web/runs")` → `~/.agent-link/web/runs`
pub fn sai_home_path(rel: &str) -> Option<PathBuf> {
    sai_home().map(|h| h.join(rel))
}

/// [`sai_home`] の本体（環境変数の値を受け取る版）。
///
/// 環境変数を書き換えないのでテストが並列実行しても壊れない。
fn resolve_sai_home(sai_home_env: Option<&OsStr>, home_env: Option<&OsStr>) -> Option<PathBuf> {
    if let Some(p) = sai_home_env.filter(|p| !p.is_empty()) {
        return Some(PathBuf::from(p));
    }
    home_env
        .filter(|h| !h.is_empty())
        .map(|h| PathBuf::from(h).join(PROJECT_DIR))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `AGENT_LINK_HOME` があればそれ、無ければ `~/.agent-link` を返す。
    #[test]
    fn sai_home_prefers_env_then_sai_dir() {
        // ① AGENT_LINK_HOME があればそれが優先される
        assert_eq!(
            resolve_sai_home(
                Some(OsStr::new("/tmp/sai-home-test")),
                Some(OsStr::new("/home/u"))
            ),
            Some(PathBuf::from("/tmp/sai-home-test")),
            "AGENT_LINK_HOME があればそれを使う"
        );
        // ② 無ければ ~/.agent-link
        assert_eq!(
            resolve_sai_home(None, Some(OsStr::new("/home/u"))),
            Some(PathBuf::from("/home/u/.agent-link"))
        );
        // ③ 空文字の AGENT_LINK_HOME は無視して ~/.agent-link に落ちる
        assert_eq!(
            resolve_sai_home(Some(OsStr::new("")), Some(OsStr::new("/home/u"))),
            Some(PathBuf::from("/home/u/.agent-link"))
        );
        // ④ HOME も無ければ None
        assert_eq!(resolve_sai_home(None, None), None);
    }

    /// 上へたどって `.agent-link/` を持つ最初のフォルダを見つける。
    #[test]
    fn find_project_root_walks_up() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("proj");
        let sub = root.join("a/b");
        std::fs::create_dir_all(&sub).unwrap();
        // root に `.agent-link/` を作る（sub には作らないので root が見つかる）
        std::fs::create_dir_all(root.join(".agent-link")).unwrap();
        let found = find_project_root(&sub, None);
        assert_eq!(found.as_deref(), Some(root.as_path()));
    }

    /// ホームに着いたら止める（ホームの `.agent-link/` は数えない）。
    #[test]
    fn find_project_root_stops_at_home() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        let sub = home.join("work");
        std::fs::create_dir_all(home.join(".agent-link")).unwrap();
        std::fs::create_dir_all(&sub).unwrap();
        // sub にもホームにも `.agent-link/` は無い（ホームにはあるが数えない）
        assert_eq!(find_project_root(&sub, Some(&home)), None);
        // ホームそのものでも止まる
        assert_eq!(find_project_root(&home, Some(&home)), None);
    }

    /// ホームの外なら根までたどる。
    #[test]
    fn find_project_root_outside_home() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("p");
        let sub = root.join("a");
        std::fs::create_dir_all(&sub).unwrap();
        let home = tmp.path().join("elsewhere");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::create_dir_all(root.join(".agent-link")).unwrap();
        assert_eq!(find_project_root(&sub, Some(&home)), Some(root));
    }

    /// `AGENT_LINK_PROJECT_ROOT` のフォルダに `.agent-link/` が無ければ断る（フォルダも作らない）。
    #[test]
    fn env_root_without_agent_link_dir_is_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().to_path_buf();
        let err = check_env_root(root.clone()).unwrap_err();
        assert!(err.contains("has no .agent-link/"), "{err}");
        assert!(!root.join(".agent-link").exists());
        assert!(check_env_root(root.join("none")).unwrap_err().contains("does not exist"));
        std::fs::create_dir_all(root.join(".agent-link")).unwrap();
        assert_eq!(check_env_root(root.clone()), Ok(root));
    }
}
