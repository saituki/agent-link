//! Stats ビュー（キャッシュ / トークン統計グラフ）。
//!
//! 本体は [`sai_link_widgets::cache_stats`] に移設済み（sai-control-tui の
//! Dashboard から同一ビューを使うため・sai-cache-tui-rs 移植版の全面移設）。
//! 既存の型名・関数名を維持するための薄い再エクスポート。

pub use sai_link_widgets::cache_stats::*;
