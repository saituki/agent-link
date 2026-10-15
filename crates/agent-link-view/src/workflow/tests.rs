//! Workflow（実行グラフ）の**移設リグレッションテスト**。
//!
//! 旧 TUI の `ui/main_panel.rs` から `agent-link-view::workflow` へ
//! 移した `build_block_graph` と `calculate_layout` が、**移す前とまったく同じ
//! 結果**を返すことを固定値で確かめる。
//!
//! 固定値: 移設前（`main` の `main_panel.rs` + `graph_layout/`）で
//! 同じ入力で流して得た値をそのまま書いてある。処理を変えるときは
//! 出力が変わることを確認したうえで、この値も更新すること。

use std::collections::HashSet;

use super::*;
use crate::workflow::{BlockDisplayData, ConnectionDisplayData, EdgeKind};

/// テスト用の実ノードを作る（`description` / `reflection_status` 以外の全項目を指定）。
fn block(
    exec_id: i64,
    block_id: i64,
    name: &str,
    tag: &str,
    gr: i64,
    lr: i64,
    parent: Option<i64>,
    batch_id: Option<&str>,
    created_at: &str,
) -> BlockDisplayData {
    BlockDisplayData {
        exec_id,
        block_id,
        name: name.to_string(),
        tag: Some(tag.to_string()),
        global_round: Some(gr),
        round: Some(lr),
        parent_exec_id: parent,
        batch_id: batch_id.map(|s| s.to_string()),
        phase: None,
        runner: "pi".to_string(),
        last_status: "completed".to_string(),
        last_duration_ms: Some(1000),
        created_at: created_at.to_string(),
        description: String::new(),
        reflection_status: None,
    }
}

fn conn(from: i64, to: i64, ty: &str) -> ConnectionDisplayData {
    ConnectionDisplayData {
        source_execution_id: from,
        target_execution_id: to,
        connection_type: ty.to_string(),
    }
}

/// 固定入力（フェーズ合成・複数 round・main 同士の waggle を含む）。
fn fixture_blocks() -> Vec<BlockDisplayData> {
    vec![
        // round 1 の実メイン
        block(10, 1, "plan", "plan", 1, 1, None, None, "2026-09-26T10:00:00"),
        // 10 のバッチ 1 サブ（2 件）
        block(
            11,
            2,
            "code",
            "code",
            1,
            1,
            Some(10),
            Some("batch_20260926_r1_aaa"),
            "2026-09-26T10:01:00",
        ),
        block(
            12,
            2,
            "code",
            "code",
            1,
            1,
            Some(10),
            Some("batch_20260926_r1_aaa"),
            "2026-09-26T10:02:00",
        ),
        // 10 のバッチ 2 サブ
        block(
            13,
            3,
            "reviewer",
            "rfrs",
            1,
            1,
            Some(10),
            Some("batch_20260926_r2_bbb"),
            "2026-09-26T10:03:00",
        ),
        // round 2 の別メイン（main 同士は waggle_token でつながる）
        block(20, 4, "kantoku", "plan", 2, 2, None, None, "2026-09-26T11:00:00"),
        // 20 のサブ（batch_id なし = レガシー。フェーズ合成はしない）
        block(
            21,
            5,
            "legacy-sub",
            "code",
            2,
            2,
            Some(20),
            None,
            "2026-09-26T11:01:00",
        ),
    ]
}

fn fixture_connections() -> Vec<ConnectionDisplayData> {
    vec![
        conn(11, 10, "task_output"),
        conn(12, 10, "task_output"),
        conn(13, 10, "task_output"),
        conn(21, 20, "task_output"),
        conn(10, 20, "waggle_token"),
    ]
}

/// 移設前の実装が返した値を固定したもの（`main` の `main_panel.rs:934-` と
/// `graph_layout/layered_layout.rs` で得た結果を 2026-09-27 に記録）。
const EXPECTED_NODES: &[(u64, i32, i32, u16, u16)] = &[
    // (node_id, x, y, width, height)
    (10, 0, 3, 27, 6),
    (11, 31, 3, 27, 6),
    (12, 62, 3, 27, 6),
    // フェーズ合成: 10 * 10_000 + 1 = 100001
    (100001, 0, 11, 27, 6),
    (13, 31, 11, 27, 6),
    // 10 * 10_000 + 2 = 100002（finalized のときだけ作られる）
    (100002, 0, 19, 27, 6),
    (20, 14, 32, 27, 6),
    (21, 45, 32, 27, 6),
];

#[test]
fn layout_matches_pre_move_values() {
    let blocks = fixture_blocks();
    let connections = fixture_connections();
    // 10 を finalized とみなす（実メインの最終フェーズ P_2 が合成される条件）
    let finalized: HashSet<i64> = [10].into_iter().collect();

    let (dag, layout_graph) = build_block_graph(&blocks, &connections, &finalized);
    let layout = calculate_layout(&layout_graph, true);

    // ノード位置（node_id 順に並べる。順序は layout.nodes の順）
    let got: Vec<(u64, i32, i32, u16, u16)> = layout
        .nodes
        .iter()
        .map(|p| (p.node_id, p.x, p.y, p.width, p.height))
        .collect();
    assert_eq!(got, EXPECTED_NODES, "DAG の座標が移設前から変わった");

    // コンテンツサイズ
    assert_eq!(layout.content_width, 89);
    assert_eq!(layout.content_height, 38);
}

/// フェーズ合成ノード（`メイン実行ID * 10_000 + k`）が DAG の形のまま出ること。
#[test]
fn phase_synthetic_nodes_have_expected_ids_and_phase() {
    let blocks = fixture_blocks();
    let connections = fixture_connections();
    let finalized: HashSet<i64> = [10].into_iter().collect();
    let (dag, _) = build_block_graph(&blocks, &connections, &finalized);

    for (node_id, phase, parent) in [
        (10u64, None, None),
        (100001, Some(1u32), Some(10)),
        (100002, Some(2), Some(10)),
    ] {
        let b = dag
            .get(dag.index_of(node_id).expect("ノードが存在する"))
            .unwrap_or_else(|| panic!("ノード {node_id} が無い"));
        assert_eq!(b.exec_id as u64, node_id);
        assert_eq!(b.phase, phase, "node_id={node_id} の phase");
        assert_eq!(b.parent_exec_id, parent, "node_id={node_id} の親");
    }

    // finalized でないメインは最終フェーズを作らない（バッチ数 - 1 まで）
    let (dag2, _) = build_block_graph(&blocks, &connections, &HashSet::new());
    assert!(dag2.index_of(100001).is_some(), "P_1 は finalized に関係なく合成する");
    assert!(
        dag2.index_of(100002).is_none(),
        "P_2 は sai-write 検出済み（finalized）のときだけ合成する"
    );
}

/// 辺の種類（`EdgeKind`）と接地方法（`side`）が移設前と同じこと。
#[test]
fn edge_kinds_and_sides_match_pre_move_values() {
    let blocks = fixture_blocks();
    let connections = fixture_connections();
    let finalized: HashSet<i64> = [10].into_iter().collect();
    let (dag, _) = build_block_graph(&blocks, &connections, &finalized);

    // (from_id, to_id, kind, side)
    let mut got: Vec<(u64, u64, EdgeKind, bool)> = dag
        .all_edges()
        .into_iter()
        .map(|(from, to, e)| (from.exec_id as u64, to.exec_id as u64, e.kind, e.side))
        .collect();
    got.sort();

    let expected: Vec<(u64, u64, EdgeKind, bool)> = vec![
        // 委譲: 実メイン → バッチ 1 の先頭サブ、バッチ内は横チェーン（左右辺接地）
        (10, 11, EdgeKind::Delegation, true),
        // main 同士の waggle_token（上下辺接地）
        (10, 20, EdgeKind::WaggleToken, false),
        // フェーズ縦鎖（上下辺接地）
        (10, 100001, EdgeKind::PhaseChain, false),
        (11, 12, EdgeKind::Delegation, true),
        // task_output: バッチサブ → 該当フェーズ（上下辺接地）
        (11, 100001, EdgeKind::TaskOutput, false),
        (12, 100001, EdgeKind::TaskOutput, false),
        (13, 100002, EdgeKind::TaskOutput, false),
        // レガシーサブ（batch_id なし）→ 実メイン（左右辺接地）
        (20, 21, EdgeKind::Delegation, true),
        (21, 20, EdgeKind::TaskOutput, true),
        // バッチ 2 は P_1 から委譲（左右辺接地）
        (100001, 13, EdgeKind::Delegation, true),
        (100001, 100002, EdgeKind::PhaseChain, false),
    ];
    assert_eq!(got, expected, "辺の種類・接地方法が移設前から変わった");
}

/// round 区切り線（`SeparatorLine`）の label と座標。
#[test]
fn separators_match_pre_move_values() {
    let blocks = fixture_blocks();
    let connections = fixture_connections();
    let finalized: HashSet<i64> = [10].into_iter().collect();
    let (_, layout_graph) = build_block_graph(&blocks, &connections, &finalized);
    let layout = calculate_layout(&layout_graph, true);

    let got: Vec<(i32, String, i64)> = layout
        .separators
        .iter()
        .map(|s| (s.y, s.label.clone(), s.global_round))
        .collect();
    assert_eq!(
        got,
        vec![(0, "round1".to_string(), 1), (29, "round2".to_string(), 2)],
        "区切り線が移設前から変わった"
    );
}
