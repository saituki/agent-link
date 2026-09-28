//! DAG (有向非巡回グラフ) データ構造。
//!
//! ブロック間接続を表現するための汎用 DAG コンテナ。
//! `petgraph::DiGraph` をラップし、ノード ID → NodeIndex の高速マッピングを提供します。

use std::collections::HashMap;

use petgraph::{
    graph::{DiGraph, NodeIndex},
    visit::EdgeRef,
    Direction,
};
use ratatui::style::Style;

/// ノード ID 型。sai-engine のブロック ID を u64 として扱う。
pub type NodeId = u64;

/// エッジの接続種別。バス行の割り当て（接続種別ごとに段差させて重なりを防ぐ）に使用する。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub enum EdgeKind {
    /// 不明・その他
    #[default]
    Other,
    /// フェーズ縦接続鎖（mb1 → P_1 → P_2 → ...）
    PhaseChain,
    /// task_delegation（main→sub 委譲、左右辺接地）
    Delegation,
    /// task_output（sub→main 返却）
    TaskOutput,
    /// waggle_token（文書の受け渡し）
    WaggleToken,
}

/// エッジデータ。スタイルとラベルを保持する。
#[derive(Debug, Clone, Default)]
pub struct EdgeData {
    pub style: Option<Style>,
    /// 左右辺（ブロック-サブブロック）接続かどうか。
    ///
    /// `true`: 長方形の左右辺の中点で接地（task_delegation / task_output、
    /// および端点にサブブロックを含む接続）
    /// `false`: 長方形の上下辺の中点で接地（main 同士の waggle_token 等）
    pub side: bool,
    /// 接続種別（色の由来となる接続の種類）。
    pub kind: EdgeKind,
}

/// 汎用 DAG グラフコンテナ。
///
/// `petgraph::DiGraph` を内部で使用し、ノード ID → NodeIndex の
/// 高速マッピングを提供します。
pub struct DagGraph<N> {
    pub graph: DiGraph<N, EdgeData>,
    pub id_to_index: HashMap<NodeId, NodeIndex>,
    pub root: Option<NodeIndex>,
}

impl<N> DagGraph<N> {
    pub fn new() -> Self {
        Self {
            graph: DiGraph::new(),
            id_to_index: HashMap::new(),
            root: None,
        }
    }

    /// ノードを追加し、NodeIndex を返す。
    pub fn add_node(&mut self, id: NodeId, data: N) -> NodeIndex {
        let idx = self.graph.add_node(data);
        self.id_to_index.insert(id, idx);
        idx
    }

    /// 有向エッジを追加する。
    pub fn add_edge(&mut self, from: NodeIndex, to: NodeIndex, data: EdgeData) {
        self.graph.add_edge(from, to, data);
    }

    /// NodeIndex からノードデータを取得する。
    pub fn get(&self, idx: NodeIndex) -> Option<&N> {
        self.graph.node_weight(idx)
    }

    /// ノード ID から NodeIndex を取得する。
    pub fn index_of(&self, id: NodeId) -> Option<NodeIndex> {
        self.id_to_index.get(&id).copied()
    }

    /// 指定ノードからの出力エッジを走査する。
    pub fn edges_from(&self, idx: NodeIndex) -> impl Iterator<Item = (NodeIndex, &EdgeData)> + '_ {
        self.graph
            .edges_directed(idx, Direction::Outgoing)
            .map(|e| (e.target(), e.weight()))
    }

    /// 全エッジを「(元ノード, 先ノード, 辺データ)」の一覧で返す。
    ///
    /// `sai-link phone workflow`（CLI）が petgraph を直接触らずに済むよう、
    /// 端点を **ノードデータ**で返す。順序は `add_edge` 順（安定）。
    pub fn all_edges(&self) -> Vec<(&N, &N, &EdgeData)> {
        self.graph
            .edge_references()
            .map(|e| {
                (
                    self.graph.node_weight(e.source()).expect("端点は必ず存在する"),
                    self.graph.node_weight(e.target()).expect("端点は必ず存在する"),
                    e.weight(),
                )
            })
            .collect()
    }
}

impl<N> Default for DagGraph<N> {
    fn default() -> Self {
        Self::new()
    }
}
