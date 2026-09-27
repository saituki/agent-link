//! Log 画面（旧右サイドバーのメイン画面移植）。
//!
//! 本体は [`sai_link_widgets::block_log`] に移設済み（sai-control-tui の
//! Dashboard から同一ビューを使うため）。既存の型名（`BlockDetail` /
//! `LogState` / `LogView` 等）を維持するための薄い再エクスポート。

pub use sai_link_widgets::block_log::*;
