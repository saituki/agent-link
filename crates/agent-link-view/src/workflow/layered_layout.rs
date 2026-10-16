//! 日付ベースのブロック配置レイアウト。
//!
//! ブロック接続グラフのノード配置を計算します（`docs/block-position-rule.md` 準拠）。
//!
//! - **行** = 同じ日付（`yymmdd`）の親ブロックのグループ。日付昇順で上から並ぶ
//! - **列** = 同一行内の DATE（作成日時）順。左から右へ並ぶ
//! - **サブブロック（sb）** = フェーズノードの右側に**横並びチェーン**で配置
//!   （`mb → sb1 → sb2 → ...`。同じバッチのサブは同じ段に並べる）
//! - 日付ごとの行グループの上に **区切り線（日付）** を描画するための座標を保持

use std::collections::HashSet;

use petgraph::graph::NodeIndex;

use super::dag::NodeId;

/// 行グループ間の垂直ギャップ（TUI が使う既定値）。
///
/// ユーザー指定: 縦方向の接続線が無いブロック間は狭める（現在値の約 0.5 倍）。
/// 縦エッジ（親→子の接続線）が通る行間も同一ギャップで描画されるため、
/// 全体を 3 → 2 に縮小する（接続線の縦セグメントは edge_render 側で吸収）。
const VERTICAL_GAP: i32 = 2;

/// 同一行内のノード間水平ギャップ（mb-sb / sb-sb の横並び間隔。TUI が使う既定値）。
const HORIZONTAL_GAP: i32 = 4;

/// 区切り線の描画行数（区切り線 1 行 + 空き 2 行。TUI が使う既定値）。
///
/// 区切り線の直下に縦エッジの分岐（水平セグメント）が重ならないよう、
/// 区切り線と次行の間の空きを 1 行分確保する。
const SEPARATOR_LINES: i32 = 3;

/// 札どうしの間隔と区切り線の太さ行数の設定。
///
/// 既定値は TUI（`agent-link-tui`）の画面がそのまま使っている値。
/// Web（`agent-link phone workflow`）は文字マスが 10×20px と大きく、
/// 既定値では札が詰まって見えないので、こちらだけ [`LayoutOptions`] で
/// 広げて使う。間隔の計算は Rust のままなので、どちらの画面でも
/// 「札は agent-link が決めた場所に固定する」約束は変わらない。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LayoutOptions {
    /// 行グループ間の垂直ギャップ（文字マス）
    pub vertical_gap: i32,
    /// 同一行内のノード間水平ギャップ（文字マス）
    pub horizontal_gap: i32,
    /// 区切り線の描画行数（区切り線 1 行 + 空き）
    pub separator_lines: i32,
}

impl Default for LayoutOptions {
    fn default() -> Self {
        Self {
            vertical_gap: VERTICAL_GAP,
            horizontal_gap: HORIZONTAL_GAP,
            separator_lines: SEPARATOR_LINES,
        }
    }
}

// ============================================================================
// NodePosition — 配置されたノードの座標情報
// ============================================================================

/// レイアウト計算後のノード位置情報。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NodePosition {
    /// petgraph のノードインデックス
    pub index: NodeIndex,
    /// 実行 ID
    pub node_id: NodeId,
    /// 左上 X 座標（文字単位）
    pub x: i32,
    /// 左上 Y 座標（文字単位、上から下）
    pub y: i32,
    /// ノード幅（文字単位）
    pub width: u16,
    /// ノード高さ（文字単位）
    pub height: u16,
}

// ============================================================================
// DagLayout — レイアウト結果
// ============================================================================

/// 日付区切り線（`yymmdd────...`）1 本分の情報。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeparatorLine {
    /// Y 座標（コンテンツ座標）
    pub y: i32,
    /// 表示ラベル（日付 `yymmdd`）
    pub label: String,
}

/// レイアウト計算結果。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DagLayout {
    /// 全ノードの配置情報
    pub nodes: Vec<NodePosition>,
    /// コンテンツ全体の幅
    pub content_width: i32,
    /// コンテンツ全体の高さ
    pub content_height: i32,
    /// 区切り線。日付ごとの行グループの上に描画する
    pub separators: Vec<SeparatorLine>,
}

impl DagLayout {
    /// NodeIndex からノード位置を検索する。
    pub fn find(&self, idx: NodeIndex) -> Option<&NodePosition> {
        self.nodes.iter().find(|n| n.index == idx)
    }
}

// ============================================================================
// LayoutNode / LayoutGraph — レイアウト計算用の入力データ
// ============================================================================

/// レイアウト計算用のノード定義。
#[derive(Debug, Clone)]
pub struct LayoutNode {
    pub index: NodeIndex,
    pub id: NodeId,
    pub width: u16,
    pub height: u16,
    /// 日付（`yymmdd`）。行グループの決定に使用
    pub date: String,
    /// 作成日時（ISO 8601）。同一行内の左→右の順序に使用
    pub created_at: String,
    /// 親実行 ID（サブブロックの場合）。main ブロックの右側に配置する
    pub parent_id: Option<NodeId>,
    /// P3-B: フェーズ親（実メイン実行 ID）。フェーズ合成ノード（小ブロック）の場合に設定
    pub phase_parent: Option<NodeId>,
    /// P3-B: フェーズ番号（0 = 実メイン、>=1 = 合成小ブロック）
    pub phase: Option<u32>,
    /// P3-B: サブが所属するタスクバッチ順序（1 始まり）。バッチサブのみ設定
    pub batch_group: Option<u32>,
}

/// レイアウト計算の入力グラフ。
#[derive(Debug, Clone, Default)]
pub struct LayoutGraph {
    /// 全ノード
    pub nodes: Vec<LayoutNode>,
    /// エッジ（from, to, side）。
    ///
    /// `side == false`（上下辺接地）のエッジは、出力元ノードの中心 X を
    /// 垂直に下るため、レイアウトでその経路を避けてノードを配置する。
    pub edges: Vec<(NodeId, NodeId, bool)>,
}

impl LayoutGraph {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_node(
        &mut self,
        index: NodeIndex,
        id: NodeId,
        width: u16,
        height: u16,
        date: String,
        created_at: String,
        parent_id: Option<NodeId>,
    ) {
        self.nodes.push(LayoutNode {
            index,
            id,
            width,
            height,
            date,
            created_at,
            parent_id,
            phase_parent: None,
            phase: None,
            batch_group: None,
        });
    }

    /// P3-B: フェーズ情報（フェーズ親・フェーズ番号・バッチ順序）を設定する。
    ///
    /// `add_node` 後に呼び出し、ノードのレイアウト上の扱いを決める。
    pub fn set_phase(
        &mut self,
        id: NodeId,
        phase_parent: Option<NodeId>,
        phase: Option<u32>,
        batch_group: Option<u32>,
    ) {
        if let Some(node) = self.nodes.iter_mut().find(|n| n.id == id) {
            node.phase_parent = phase_parent;
            node.phase = phase;
            node.batch_group = batch_group;
        }
    }

    /// エッジを追加する。`side` は `EdgeData.side`（左右辺接地フラグ）と同一。
    pub fn add_edge(&mut self, from: NodeId, to: NodeId, side: bool) {
        self.edges.push((from, to, side));
    }
}

// ============================================================================
// calculate_layout — メインエントリポイント
// ============================================================================

/// レイアウトを計算する（TUI が使う既定の間隔）。
///
/// 行 = (GR, LR) グループ（GR 昇順 → LR 昇順）、列 = DATE 順。
/// サブブロックは main ブロックの右側に縦積みで配置する。
pub fn calculate_layout(graph: &LayoutGraph, _top_down: bool) -> DagLayout {
    calculate_layout_with(graph, &LayoutOptions::default())
}

/// レイアウトを計算する（札どうしの間隔を指定する）。
///
/// 間隔を変えたい画面（Web）は [`LayoutOptions`] の値を作って渡す。
/// 座標を px へ直す処理は呼び出し側に残し、ここでは文字マスのまま返す。
pub fn calculate_layout_with(graph: &LayoutGraph, opts: &LayoutOptions) -> DagLayout {
    if graph.nodes.is_empty() {
        return DagLayout::default();
    }

    // P3-B: フェーズ合成ノード（phase_parent が Some）とそれ以外を分離する。
    // フェーズノードは通常の行グループには配置せず、
    // 実メイン実行を縦に分割した専用の列として扱う。
    let (phase_nodes, rest): (Vec<&LayoutNode>, Vec<&LayoutNode>) =
        graph.nodes.iter().partition(|n| n.phase_parent.is_some());

    // メイン（親なし）とサブ（親あり）に分離。
    // バッチサブ（batch_group あり）も通常サブも親の右側に配置する。
    let (mains, subs): (Vec<&LayoutNode>, Vec<&LayoutNode>) =
        rest.iter().partition(|n| n.parent_id.is_none());

    let mut positions: Vec<NodePosition> = Vec::with_capacity(graph.nodes.len());
    let mut separators: Vec<SeparatorLine> = Vec::new();

    // ── メインを日付グループでソート・グループ化 ──
    // ソートキー: (date, created_at, id)
    let mut mains_sorted: Vec<&LayoutNode> = mains.clone();
    mains_sorted.sort_by(|a, b| {
        (a.date.as_str(), a.created_at.as_str(), a.id)
            .cmp(&(b.date.as_str(), b.created_at.as_str(), b.id))
    });

    let mut groups: Vec<(&str, Vec<&LayoutNode>)> = Vec::new();
    for node in mains_sorted {
        match groups.last_mut() {
            Some((d, list)) if *d == node.date.as_str() => list.push(node),
            _ => groups.push((node.date.as_str(), vec![node])),
        }
    }

    // ── 行の Y 座標と列の X 座標を計算 ──
    let mut y_cursor: i32 = 0;
    // 上の行から下の行へ伸びる縦エッジの経路（入力ノードID, 経路 X）。
    // 出力元の行の配置完了時に追加し、入力先の行の配置時に除去する。
    let mut pending_paths: Vec<(NodeId, i32)> = Vec::new();

    for (date, group) in groups.iter() {
        // 日付ごとの行グループの上に区切り線
        separators.push(SeparatorLine {
            y: y_cursor,
            label: date.to_string(),
        });
        y_cursor += opts.separator_lines;

        let row_y = y_cursor;
        // この行グループに属するノード ID（経路の終端判定に使用）
        let group_ids: HashSet<NodeId> = group.iter().map(|n| n.id).collect();
        // この行で終了するエッジ（入力先ノードがこの行にある）の経路を除去
        pending_paths.retain(|(to_id, _)| !group_ids.contains(to_id));
        // この行を通過する経路 X のみを抽出（ノード配置の回避に使用）
        let path_xs: Vec<i32> = pending_paths.iter().map(|(_, x)| *x).collect();

        let mut x: i32 = 0;
        let mut row_height: i32 = 0;

        // グループ内は DATE 順（mains_sorted のソート順を引き継いでいる）
        for main in group {
            // 上の行から伸びる縦エッジの経路と重ならない位置に配置する
            // （接続線がブロックに遮断されないように）
            x = avoid_paths(x, main.width, &path_xs);

            // main ブロック本体（フェーズ 0）
            positions.push(NodePosition {
                index: main.index,
                node_id: main.id,
                x,
                y: row_y,
                width: main.width,
                height: main.height,
            });
            row_height = row_height.max(i32::from(main.height));

            // この main のフェーズノード（phase_parent == main.id、フェーズ昇順）
            let mut my_phases: Vec<&LayoutNode> = phase_nodes
                .iter()
                .filter(|n| n.phase_parent == Some(main.id))
                .copied()
                .collect();
            my_phases.sort_by_key(|n| n.phase.unwrap_or(0));

            // この main のサブブロック（バッチ順 → DATE 順）
            let mut my_subs: Vec<&LayoutNode> = subs
                .iter()
                .filter(|s| s.parent_id == Some(main.id))
                .copied()
                .collect();
            my_subs.sort_by(|a, b| {
                (a.batch_group.unwrap_or(0), a.created_at.as_str())
                    .cmp(&(b.batch_group.unwrap_or(0), b.created_at.as_str()))
            });

            if my_phases.is_empty() {
                // ── フェーズなし: サブを main の右側に横並びチェーンで配置 ──
                // （mb → sb1 → sb2 → ... と同じ段に並べる）
                if my_subs.is_empty() {
                    x += i32::from(main.width) + opts.horizontal_gap;
                    continue;
                }

                // サブ行: main ブロックの右側に横並び（縦積みではなくチェーン）
                let mut sub_x = x + i32::from(main.width) + opts.horizontal_gap;
                let mut sub_right_max = sub_x;
                for sub in &my_subs {
                    positions.push(NodePosition {
                        index: sub.index,
                        node_id: sub.id,
                        x: sub_x,
                        y: row_y,
                        width: sub.width,
                        height: sub.height,
                    });
                    sub_right_max = sub_x + i32::from(sub.width);
                    // 行の高さにサブの高さも含める（次の行との重なり防止）
                    row_height = row_height.max(i32::from(sub.height));
                    // 次のサブは右隣に横並び
                    sub_x = sub_right_max + opts.horizontal_gap;
                }
                // 次の main はサブ列の右側から始める
                x = sub_right_max + opts.horizontal_gap;
                continue;
            }

            // ── P3-B: フェーズ列モード ──
            // フェーズ行 k: フェーズノード（左・main と同じ X） +
            // バッチ k+1 のサブ群（右・横並びチェーン）。
            // P_0 = main、P_k (k>=1) = 合成小ブロック（最終行はサブなし）。
            let mut sub_right_max = x + i32::from(main.width);
            let mut cursor_y: i32;

            // フェーズ行 0（main + バッチ1 のサブ群: 横並びチェーン）
            let mut row_h = i32::from(main.height);
            let mut sub_x = x + i32::from(main.width) + opts.horizontal_gap;
            for sub in my_subs.iter().filter(|s| s.batch_group == Some(1)) {
                positions.push(NodePosition {
                    index: sub.index,
                    node_id: sub.id,
                    x: sub_x,
                    y: row_y,
                    width: sub.width,
                    height: sub.height,
                });
                sub_right_max = sub_x + i32::from(sub.width);
                row_h = row_h.max(i32::from(sub.height));
                sub_x = sub_right_max + opts.horizontal_gap;
            }
            cursor_y = row_y + row_h + opts.vertical_gap;

            // フェーズ行 k (k>=1): P_k + バッチ k+1 のサブ群（横並びチェーン）
            for (k, phase_node) in my_phases.iter().enumerate() {
                let phase_y = cursor_y;
                positions.push(NodePosition {
                    index: phase_node.index,
                    node_id: phase_node.id,
                    x,
                    y: phase_y,
                    width: phase_node.width,
                    height: phase_node.height,
                });
                let mut phase_row_h = i32::from(phase_node.height);
                let mut sub_x = x + i32::from(phase_node.width) + opts.horizontal_gap;
                // バッチ k+1 のサブ群（P_k の右側。k は 0 始まりなので batch_group = k+2）
                for sub in my_subs
                    .iter()
                    .filter(|s| s.batch_group == Some(k as u32 + 2))
                {
                    positions.push(NodePosition {
                        index: sub.index,
                        node_id: sub.id,
                        x: sub_x,
                        y: phase_y,
                        width: sub.width,
                        height: sub.height,
                    });
                    sub_right_max = sub_x + i32::from(sub.width);
                    phase_row_h = phase_row_h.max(i32::from(sub.height));
                    sub_x = sub_right_max + opts.horizontal_gap;
                }
                cursor_y = phase_y + phase_row_h + opts.vertical_gap;
            }

            // フェーズ列全体の高さを行の高さに反映（次の行との重なり防止）
            row_height = row_height.max(cursor_y - row_y);
            // 次の main はサブ列の右側から始める
            x = sub_right_max + opts.horizontal_gap;
        }

        // この行から出る縦エッジ（上下辺接地）の経路を追加する
        for &(from_id, to_id, side) in &graph.edges {
            if side {
                continue; // 左右辺接地（サブ関係）は下の行を通過しない
            }
            if group_ids.contains(&from_id) {
                if let Some(pos) = positions.iter().find(|p| p.node_id == from_id) {
                    let center_x = pos.x + i32::from(pos.width) / 2;
                    pending_paths.push((to_id, center_x));
                }
            }
        }

        y_cursor = row_y + row_height + opts.vertical_gap;
    }

    let content_width = positions
        .iter()
        .map(|p| p.x + i32::from(p.width))
        .max()
        .unwrap_or(0);
    let content_height = positions
        .iter()
        .map(|p| p.y + i32::from(p.height))
        .max()
        .unwrap_or(0);

    DagLayout {
        nodes: positions,
        content_width,
        content_height,
        separators,
    }
}

/// ノードを配置する X 位置を、上の行から伸びる縦エッジの経路を避けるように調整する。
///
/// 縦エッジは出力元ノードの中心 X を垂直に下るため、その経路 X がノードの
/// 範囲 [x, x+width) に含まれると接続線がブロックに遮断される。
/// 経路を含まない最小の X まで右へシフトする。
fn avoid_paths(mut x: i32, width: u16, paths: &[i32]) -> i32 {
    if paths.is_empty() {
        return x;
    }
    let w = i32::from(width);
    let mut sorted: Vec<i32> = paths.to_vec();
    sorted.sort_unstable();
    sorted.dedup();
    // 経路がノード範囲に含まれなくなるまで右へシフトする（有限回で収束）
    let mut changed = true;
    while changed {
        changed = false;
        for &p in &sorted {
            if p >= x && p < x + w {
                x = p + 1;
                changed = true;
            }
        }
    }
    x
}

// ============================================================================
// テスト
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn add(
        graph: &mut LayoutGraph,
        id: u64,
        date: &str,
        created_at: &str,
        parent: Option<NodeId>,
    ) -> NodeIndex {
        let idx = petgraph::graph::NodeIndex::new(graph.nodes.len());
        graph.add_node(idx, id, 16, 6, date.to_string(), created_at.to_string(), parent);
        idx
    }

    fn find_pos(layout: &DagLayout, id: u64) -> NodePosition {
        layout
            .nodes
            .iter()
            .find(|p| p.node_id == id)
            .copied()
            .expect("ノードが配置されていない")
    }

    /// 既定の設定は今の TUI と同じ間隔（縦 2・横 4・区切り 3）であること。
    /// ここが変わると TUI の画面がずれるため、値を固定しておく。
    #[test]
    fn test_layout_options_default_is_legacy() {
        let d = LayoutOptions::default();
        assert_eq!(d.vertical_gap, 2, "縦の既定値は 2 のまま");
        assert_eq!(d.horizontal_gap, 4, "横の既定値は 4 のまま");
        assert_eq!(d.separator_lines, 3, "区切りの既定値は 3 のまま");
    }

    /// 既定の値では calculate_layout と calculate_layout_with が同じ座標になること
    /// （TUI の画面が動いてないことの確認）。
    #[test]
    fn test_layout_with_default_matches_calculate_layout() {
        let mut g = LayoutGraph::new();
        add(&mut g, 1, "261001", "2026-08-01T10:00:00", None);
        add(&mut g, 2, "261001", "2026-08-01T10:01:00", None);
        add(&mut g, 3, "261002", "2026-08-01T10:02:00", None);
        g.add_edge(1, 3, false);

        let a = calculate_layout(&g, true);
        let b = calculate_layout_with(&g, &LayoutOptions::default());
        assert_eq!(a, b, "既定値では同じ座標になる");
    }

    /// 広くした値では行と列の間隔も区切りの空きも広がる（座標が散る）こと。
    /// Web 側はこれで札どうしに余裕ができる。TUI の既定値は変わらない。
    #[test]
    fn test_layout_with_wider_options_spreads_gaps() {
        let mut g = LayoutGraph::new();
        add(&mut g, 1, "261001", "2026-08-01T10:00:00", None);
        add(&mut g, 2, "261001", "2026-08-01T10:01:00", None);
        add(&mut g, 3, "261002", "2026-08-01T10:02:00", None);

        let narrow = calculate_layout(&g, true);
        let wide = calculate_layout_with(
            &g,
            &LayoutOptions { vertical_gap: 4, horizontal_gap: 8, separator_lines: 4 },
        );

        // 同じ行の 2 つ目の札は右へ離れる
        let n1 = find_pos(&narrow, 1);
        let w1 = find_pos(&wide, 1);
        let n2 = find_pos(&narrow, 2);
        let w2 = find_pos(&wide, 2);
        assert_eq!(n2.y - n1.y, w2.y - w1.y, "同じ行の中の 2 つの高さは変わらない");
        assert_eq!(n2.x - n1.x, 20, "既定では 16（幅）+ 4（間隔）");
        assert_eq!(w2.x - w1.x, 24, "広い方は 16（幅）+ 8（間隔）");

        // 区切りの空きが増えて次の行が下がる
        let n3 = find_pos(&narrow, 3);
        let w3 = find_pos(&wide, 3);
        assert_eq!(n3.x, w3.x, "行の始まりは左のままである");
        assert!(w3.y - n3.y > 0, "縦の間隔が広がる: {} -> {}", n3.y, w3.y);
        // 全世界の大きさも広がる
        assert!(wide.content_width > narrow.content_width);
        assert!(wide.content_height > narrow.content_height);
    }

    #[test]
    fn test_rows_grouped_by_date_and_columns_by_created_at() {
        let mut g = LayoutGraph::new();
        add(&mut g, 1, "261001", "2026-08-01T10:00:00", None);
        add(&mut g, 2, "261002", "2026-08-01T10:01:00", None); // 同じ日付 (a)
        add(&mut g, 3, "261002", "2026-08-01T10:02:00", None); // 同じ日付 (b)
        add(&mut g, 4, "261004", "2026-08-01T10:03:00", None);
        add(&mut g, 5, "261003", "2026-08-01T10:04:00", None);

        let layout = calculate_layout(&g, true);

        let p1 = find_pos(&layout, 1);
        let p2 = find_pos(&layout, 2);
        let p3 = find_pos(&layout, 3);
        let p5 = find_pos(&layout, 5);
        let p4 = find_pos(&layout, 4);

        // 同じ日付の 2 つは同じ行
        assert_eq!(p2.y, p3.y, "同じ日付は同じ行になるべき");
        // 行の並び: 日付昇順
        assert!(p1.y < p2.y, "日付昇順で上から並ぶべき");
        assert!(p2.y < p5.y, "日付昇順で上から並ぶべき");
        assert!(p5.y < p4.y, "日付昇順で上から並ぶべき");
        // 同じ行内は created_at 順（左 → 右）
        assert!(p2.x < p3.x, "created_at 順に左から並ぶべき");
    }

    /// サブブロックは main ブロックの右側に**横並びチェーン**で配置されること。
    #[test]
    fn test_sub_blocks_placed_right_of_main() {
        let mut g = LayoutGraph::new();
        add(&mut g, 10, "261001", "2026-08-01T10:00:00", None); // main
        add(
            &mut g,
            11,
            "261001",
            "2026-08-01T10:00:10",
            Some(10),
        ); // sub1
        add(
            &mut g,
            12,
            "261001",
            "2026-08-01T10:00:20",
            Some(10),
        ); // sub2

        let layout = calculate_layout(&g, true);

        let main = find_pos(&layout, 10);
        let sub1 = find_pos(&layout, 11);
        let sub2 = find_pos(&layout, 12);

        // sub は main の右側
        assert!(
            sub1.x >= main.x + i32::from(main.width),
            "sub は main の右側に配置"
        );
        // 横並びチェーン（mb → sb1 → sb2 と同じ段）
        assert!(sub1.y == main.y, "最初の sub は main と同じ行の高さ");
        assert!(sub2.y == sub1.y, "sub は同じ段に横並び（縦積みではない）");
        assert!(sub2.x > sub1.x, "sub は右に横並びチェーンで並ぶ");
        // main と sub は同じ行（GR/LR 継承）
        assert_eq!(main.y, sub1.y);
    }

    /// 区切り線は日付ごとに 1 本、ラベルは日付になること。
    #[test]
    fn test_separators_per_date() {
        let mut g = LayoutGraph::new();
        add(&mut g, 1, "261001", "2026-08-01T10:00:00", None);
        add(&mut g, 2, "261001", "2026-08-01T10:01:00", None);
        add(&mut g, 3, "261002", "2026-08-01T10:02:00", None);
        add(&mut g, 4, "261003", "2026-08-01T10:03:00", None);

        let layout = calculate_layout(&g, true);

        assert_eq!(
            layout
                .separators
                .iter()
                .map(|s| s.label.clone())
                .collect::<Vec<_>>(),
            vec![
                "261001".to_string(),
                "261002".to_string(),
                "261003".to_string()
            ],
            "日付ごとに区切り線（最初の日付含む）"
        );
    }

    /// 行は date 昇順（created_at が後でも、日付が早ければ上）。
    #[test]
    fn test_rows_sorted_by_date_before_created_at() {
        let mut g = LayoutGraph::new();
        add(&mut g, 1, "261002", "2026-08-01T10:00:00", None);
        add(&mut g, 2, "261001", "2026-08-01T10:01:00", None);

        let layout = calculate_layout(&g, true);

        let p1 = find_pos(&layout, 1);
        let p2 = find_pos(&layout, 2);
        assert!(p2.y < p1.y, "date が早い方が上の行");
    }

    /// 上の行から伸びる縦エッジの経路（出力元ノードの中心 X）を
    /// 途中の行のノードが避けて配置されること。
    #[test]
    fn test_nodes_avoid_vertical_edge_paths() {
        let mut g = LayoutGraph::new();
        // 行 0: ID1 (x=0), ID2 (x=18)。行 1: ID3, ID4。行 2: ID5。
        // ID2→ID5 の縦エッジ経路（ID2 の中心 X）を、途中の行 1 のノードが避ける。
        add(&mut g, 1, "261001", "2026-08-01T10:00:00", None);
        add(&mut g, 2, "261001", "2026-08-01T10:01:00", None);
        add(&mut g, 3, "261002", "2026-08-01T10:02:00", None);
        add(&mut g, 4, "261002", "2026-08-01T10:03:00", None);
        add(&mut g, 5, "261003", "2026-08-01T10:04:00", None);
        g.add_edge(2, 5, false);

        let layout = calculate_layout(&g, true);

        let p2 = find_pos(&layout, 2);
        let path_x = p2.x + i32::from(p2.width) / 2;
        // 途中の行（行 1）のノードが経路を避けること
        for id in [3u64, 4] {
            let p = find_pos(&layout, id);
            assert!(
                path_x < p.x || path_x >= p.x + i32::from(p.width),
                "途中の行のノード ID{} は縦エッジの経路 x={} を避けるべき（x={}）",
                id,
                path_x,
                p.x
            );
        }
    }

    /// 経路が無い場合はノードは詰めて配置されること（経路回避が過剰に
    /// ノードを右へシフトしない）。
    #[test]
    fn test_nodes_packed_when_no_paths() {
        let mut g = LayoutGraph::new();
        add(&mut g, 1, "261001", "2026-08-01T10:00:00", None);
        add(&mut g, 2, "261001", "2026-08-01T10:01:00", None);
        add(&mut g, 3, "261002", "2026-08-01T10:02:00", None);
        add(&mut g, 4, "261002", "2026-08-01T10:03:00", None);
        // エッジなし（経路回避しない）

        let layout = calculate_layout(&g, true);

        let p1 = find_pos(&layout, 1);
        let p2 = find_pos(&layout, 2);
        let p3 = find_pos(&layout, 3);
        let p4 = find_pos(&layout, 4);
        // 同一行内は隙間 HORIZONTAL_GAP 文字で詰めて配置
        assert_eq!(p1.x, 0);
        assert_eq!(p2.x, p1.x + i32::from(p1.width) + HORIZONTAL_GAP);
        assert_eq!(p3.x, 0, "経路がなければ次の行も左詰め");
        assert_eq!(p4.x, p3.x + i32::from(p3.width) + HORIZONTAL_GAP);
    }

    /// P3-B: フェーズ列モードでは、フェーズノードがメインの下に縦に並び、
    /// 各フェーズの右側にそのバッチのサブ群が配置されること。
    #[test]
    fn test_phase_columns_stack_vertically() {
        let mut g = LayoutGraph::new();
        // メイン（フェーズ0）
        let _m_idx = add(&mut g, 10, "261001", "2026-08-01T10:00:00", None);
        // 合成フェーズノード P_1 / P_2（phase_parent = 10）
        let _p1_idx = add(
            &mut g,
            100_001,
            "261001",
            "2026-08-01T10:00:00",
            Some(10),
        );
        let _p2_idx = add(
            &mut g,
            100_002,
            "261001",
            "2026-08-01T10:00:00",
            Some(10),
        );
        // バッチサブ: バッチ1（11, 12）・バッチ2（13）
        let _s11 = add(
            &mut g,
            11,
            "261001",
            "2026-08-01T10:01:00",
            Some(10),
        );
        let _s12 = add(
            &mut g,
            12,
            "261001",
            "2026-08-01T10:02:00",
            Some(10),
        );
        let _s13 = add(
            &mut g,
            13,
            "261001",
            "2026-08-01T10:03:00",
            Some(10),
        );

        g.set_phase(100_001, Some(10), Some(1), None);
        g.set_phase(100_002, Some(10), Some(2), None);
        g.set_phase(11, None, None, Some(1));
        g.set_phase(12, None, None, Some(1));
        g.set_phase(13, None, None, Some(2));

        let layout = calculate_layout(&g, true);

        let m = find_pos(&layout, 10);
        let p1 = find_pos(&layout, 100_001);
        let p2 = find_pos(&layout, 100_002);
        let s11 = find_pos(&layout, 11);
        let s12 = find_pos(&layout, 12);
        let s13 = find_pos(&layout, 13);

        // フェーズノードはメインと同じ X で縦に並ぶ
        assert_eq!(p1.x, m.x, "フェーズ1 はメインと同じ X");
        assert_eq!(p2.x, m.x, "フェーズ2 もメインと同じ X");
        assert!(p1.y > m.y, "フェーズ1 はメインの下");
        assert!(p2.y > p1.y, "フェーズ2 はフェーズ1 の下");

        // バッチ1 のサブはフェーズ0（メイン）の右側に横並びチェーン、バッチ2 はフェーズ1 の右側
        assert!(s11.x > m.x + i32::from(m.width), "バッチ1 はメインの右側");
        assert!(s12.x > s11.x, "同一バッチのサブは右に横並びチェーン");
        assert_eq!(s11.y, s12.y, "同一バッチのサブは同じ段（縦積みではない）");
        assert!(
            s13.x > p1.x + i32::from(p1.width),
            "バッチ2 はフェーズ1 の右側"
        );
        assert!(s13.y >= p1.y, "バッチ2 はフェーズ1 と同じ行");

        // フェーズノードはメインと重ならない（縦に積まれている）
        assert!(
            p1.y >= m.y + i32::from(m.height),
            "フェーズ1 はメインの下端より下"
        );
        assert!(
            p2.y >= p1.y + i32::from(p1.height),
            "フェーズ2 はフェーズ1 の下端より下"
        );
    }
}
