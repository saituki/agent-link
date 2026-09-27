//! # プロジェクトルート解決
//!
//! sai-engine は `.sai/` ディレクトリを「プロジェクトルート（デーモン起動パス）」基準で
//! 解決します（D15）。
//!
//! デーモンは起動時に `SAI_PROJECT_ROOT` 環境変数を設定します。
//! worktree 実行時は pi プロセスの cwd が `.worktrees/<branch>` に変わるため、
//! そのまま `.sai/` 相対パスを使うと誤った場所を参照してしまいます。
//! このモジュールを経由することで、デーモン起動パス（= プロジェクトルート）の
//! `.sai/` を常に正しく参照できます。
//!
//! ## 優先順位
//!
//! 1. 環境変数 `SAI_PROJECT_ROOT`（デーモンが起動時に設定）
//! 2. カレントディレクトリ（デーモン未使用時のフォールバック。テスト互換）

use std::path::PathBuf;

/// プロジェクトルート（`.sai` の基準ディレクトリ）を解決する。
pub fn project_root() -> PathBuf {
    if let Some(p) = std::env::var_os("SAI_PROJECT_ROOT") {
        let pb = PathBuf::from(p);
        if !pb.as_os_str().is_empty() {
            return pb;
        }
    }
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

/// プロジェクトルートからの相対パスを絶対パスに解決する。
///
/// 例: `sai_path(".sai/sai-engine/sai-engine.db")` → `<root>/.sai/sai-engine/sai-engine.db`
pub fn sai_path(rel: &str) -> PathBuf {
    project_root().join(rel)
}

/// sai-engine の DB（`.sai/sai-engine/sai-engine.db`）のパスを解決する。
///
/// 優先順位:
/// 1. 環境変数 `SAI_PROJECT_ROOT` → `<root>/.sai/sai-engine/sai-engine.db`
/// 2. （無ければ）後方互換の相対パス候補から最初に存在するもの
///
/// 旧 sai-link-tui `paths::resolve_db_path` と同じ解像順（TUI / CLI / widgets で
/// 同じ DB を開くための共通入口）。
pub fn resolve_db_path() -> PathBuf {
    // ① SAI_PROJECT_ROOT を第一優先（複数プロジェクト・worktree 対応）
    if let Some(root) = std::env::var_os("SAI_PROJECT_ROOT") {
        let p = PathBuf::from(root).join(".sai/sai-engine/sai-engine.db");
        if p.exists() {
            return p;
        }
    }

    // ② 後方互換: 単一プロジェクトの相対パス候補
    const REL: [&str; 3] = [
        ".sai/sai-engine/sai-engine.db",
        "../.sai/sai-engine/sai-engine.db",
        "../../.sai/sai-engine/sai-engine.db",
    ];
    REL.iter()
        .map(PathBuf::from)
        .find(|p| p.exists())
        .unwrap_or_else(|| PathBuf::from(".sai/sai-engine/sai-engine.db"))
}
