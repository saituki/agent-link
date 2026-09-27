//! sai-link-tui のプロジェクト・DB パス解決。
//!
//! 複数プロジェクト・worktree の並列運用を考慮し、環境変数 `SAI_PROJECT_ROOT`
//! （デーモン＝sai-engine が設定）を第一優先で解決する。
//! `SAI_PROJECT_ROOT` が無い場合は後方互換の相対パス候補から選ぶ。

use std::path::PathBuf;

/// sai-engine の DB（`.sai/sai-engine/sai-engine.db`）のパスを解決する。
///
/// 解決本体は `sai_link_engine::project::resolve_db_path` へ移設（CLI / widgets と
/// 同一の DB を開くための共通入口。実装計画 12 §3.1）。
pub fn resolve_db_path() -> PathBuf {
    sai_link_engine::project::resolve_db_path()
}

/// TUI 専用の card 配置ファイル（`.sai/ui-layout.json`）のパスを解決する。
///
/// card のドラッグ移動・round 区切り線のドラッグで生じたズレ量を保存する。
/// DB スキーマとは無関係の UI 装飾情報のため、`sai-link-engine` の DB には置かない。
/// 存在確認はしない（未作成でも呼び出し側が「読み込み失敗は無視」で扱うため）。
pub fn resolve_ui_layout_path() -> PathBuf {
    if let Some(root) = std::env::var_os("SAI_PROJECT_ROOT") {
        return PathBuf::from(root).join(".sai/ui-layout.json");
    }
    PathBuf::from(".sai/ui-layout.json")
}
