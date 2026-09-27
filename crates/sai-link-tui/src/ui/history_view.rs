//! History 画面（ratatui ネイティブウィジェット描画）。
//!
//! 本体は [`sai_link_widgets::block_history`] に移設済み（sai-control-tui の
//! Dashboard から同一ビューを使うため）。既存の型名・関数名を維持するための
//! 薄い再エクスポート。

pub use sai_link_widgets::block_history::*;
