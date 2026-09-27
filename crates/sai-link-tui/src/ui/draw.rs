//! CJK 文字幅対応の描画ヘルパー。
//!
//! 本体は [`sai_link_widgets::cjk`] に移設済み（sai-control-tui と同一 UI を
//! 共有するため・実装重複の解消）。既存の呼び出し先
//!（`super::draw::draw_text_cjk` 等のパス）を維持するための薄い再エクスポート。

pub use sai_link_widgets::cjk::*;
