//! # グラフレイアウトモジュール
//!
//! Workflow（実行ブロック線図）のデータ構造・配置計算は
//! `sai-link-widgets` の [`sai_link_widgets::workflow`] にある（TUI と
//! `sai-link phone workflow` が同じ関数を使う）。ここは **TUI 固有**の部分だけ。
//!
//! - `edge_render` — 罫線文字によるエッジ描画（端末専用。Web は SVG で自前に描く）
//! - `viewport_state` — スクロール・ビューポート管理

mod edge_render;
mod viewport_state;

pub use edge_render::{render_edges, render_separators};
pub use sai_link_widgets::workflow::{
    calculate_layout, DagLayout, NodePosition, SeparatorLine,
};
// DAG とレイアウトグラフの型は TUI の実装（`edge_render` / `main_panel`）からは
// 直接参照しない。移設先（`sai_link_widgets::workflow`）から使う。
// 下列は `main_panel` のテスト（`use super::*`）が使うための再公開。
#[cfg(test)]
pub use sai_link_widgets::workflow::{DagGraph, EdgeData, EdgeKind, LayoutGraph};
pub use viewport_state::GraphViewport;
