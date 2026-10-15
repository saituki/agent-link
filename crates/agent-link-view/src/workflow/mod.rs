//! # Workflow — 実行グラフ（DAG）の組み立て・配置・材料集め
//!
//! 旧 TUI の `ui/main_panel.rs` と `graph_layout/` にあった
//! 「Workflow（実行ブロック線図）」のロジックを移設したもの。
//!
//! - [`dag`] — DAG データ構造（[`DagGraph`] / [`EdgeData`] / [`EdgeKind`] / `NodeId`）
//! - [`layered_layout`] — Sugiyama 風の階層レイアウト計算（[`calculate_layout`] と
//!   間隔を変える画面の [`calculate_layout_with`]）
//! - [`build`] — 実行データから DAG を組み立てる（[`build_block_graph`] と表示データ型）
//! - [`load`] — DB から DAG の材料を集める（[`load_workflow`]）
//!
//! `agent-link phone workflow`（CLI）が座標を付けて Web へ返す。
//!
//! ## 使用例
//!
//! ```no_run
//! use agent_link_view::workflow::{build_block_graph, calculate_layout, load_workflow};
//! use agent_link_view::card_entries::prepare_view;
//! # fn f(store: &agent_link_engine::db::SaiStore) {
//! let prep = prepare_view(store);
//! let src = load_workflow(store, &prep);
//! let (dag, layout_graph) = build_block_graph(&src.blocks, &src.connections, &src.finalized_runs);
//! let layout = calculate_layout(&layout_graph, true);
//! # let _ = (dag, layout);
//! # }
//! ```

pub mod build;
pub mod dag;
pub mod layered_layout;
pub mod load;
#[cfg(test)]
mod tests;

pub use build::{build_block_graph, BlockDisplayData, ConnectionDisplayData};
pub use dag::{DagGraph, EdgeData, EdgeKind, NodeId};
pub use layered_layout::{
    calculate_layout, calculate_layout_with, DagLayout, LayoutGraph, LayoutNode, LayoutOptions,
    NodePosition, SeparatorLine,
};
pub use load::{load_workflow, WorkflowSource};
