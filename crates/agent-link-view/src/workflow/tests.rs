//! Workflow（実行グラフ）の組み立て（`build_block_graph`）と配置（`calculate_layout`）のテスト。
//!
//! 子の段（`task create` 1 回 = 1 段）・委譲の線（必ず親 → 子）・座標を固定値で確かめる。

use super::*;
use crate::workflow::{BlockDisplayData, ConnectionDisplayData, EdgeKind};
use crate::workflow::layered_layout::{DagLayout, NodePosition};

/// テスト用の実ノードを作る（`description` / `model` / `thinking` 以外の全項目を指定）。
fn block(
    exec_id: i64,
    block_id: i64,
    name: &str,
    date: &str,
    parent: Option<i64>,
    batch_id: Option<&str>,
    created_at: &str,
) -> BlockDisplayData {
    BlockDisplayData {
        exec_id,
        block_id,
        name: name.to_string(),
        date: date.to_string(),
        parent_exec_id: parent,
        batch_id: batch_id.map(|s| s.to_string()),
        last_status: "completed".to_string(),
        last_duration_ms: Some(1000),
        created_at: created_at.to_string(),
        description: String::new(),
        model: None,
        thinking: None,
    }
}

fn conn(from: i64, to: i64, ty: &str) -> ConnectionDisplayData {
    ConnectionDisplayData {
        source_execution_id: from,
        target_execution_id: to,
        connection_type: ty.to_string(),
    }
}

/// 親 10（日付 261001）に、バッチ A（子 11・12）とバッチ B（子 13）。
/// バッチ ID の `_r<N>_` は段の順に関係しない（B の方が r の数字は小さい）。
/// 別の親 20 にはバッチなしの子 21 と、その子（孫）22。
fn fixture_blocks() -> Vec<BlockDisplayData> {
    vec![
        block(10, 1, "plan", "261001", None, None, "2026-09-26T10:00:00"),
        block(11, 2, "code", "261001", Some(10), Some("batch_a_r2_x"), "2026-09-26T10:01:00"),
        block(12, 2, "code", "261001", Some(10), Some("batch_a_r2_x"), "2026-09-26T10:02:00"),
        block(13, 3, "reviewer", "261001", Some(10), Some("batch_b_r1_y"), "2026-09-26T10:03:00"),
        block(20, 4, "kantoku", "261002", None, None, "2026-09-26T11:00:00"),
        block(21, 5, "worker", "261002", Some(20), None, "2026-09-26T11:01:00"),
        block(22, 6, "explore", "261002", Some(21), None, "2026-09-26T11:02:00"),
    ]
}

fn pos(layout: &DagLayout, id: u64) -> NodePosition {
    layout.nodes.iter().find(|p| p.node_id == id).copied().expect("ノードが配置されている")
}

/// 親 1・バッチ 2（子 2 人 + 子 1 人）: 1 つ目の子は親と同じ y で親の右、
/// 2 つ目の子は 1 段下で 1 つ目の子と同じ x から始まる。辺は親 → 各子の委譲 3 本だけ。
#[test]
fn children_stack_one_row_per_task_create() {
    let blocks: Vec<_> = fixture_blocks().into_iter().filter(|b| b.exec_id < 20).collect();
    // 子どうしの waggle_token 以外の接続（task_output）は線にしない
    let connections = vec![conn(11, 10, "task_output"), conn(13, 10, "task_output")];
    let (dag, lg) = build_block_graph(&blocks, &connections);
    let layout = calculate_layout(&lg, true);

    let (p, a1, a2, b1) = (pos(&layout, 10), pos(&layout, 11), pos(&layout, 12), pos(&layout, 13));
    assert_eq!(a1.y, p.y, "1 つ目の子は親と同じ y");
    assert_eq!(a2.y, p.y);
    assert!(a1.x > p.x + i32::from(p.width), "親の右");
    assert!(a2.x > a1.x, "同じ段は横一列");
    assert!(b1.y > a1.y, "2 つ目の子は 1 段下");
    assert_eq!(b1.x, a1.x, "1 つ目の子と同じ x から");

    let mut got: Vec<(u64, u64, EdgeKind, bool)> = dag
        .all_edges()
        .into_iter()
        .map(|(f, t, e)| (f.exec_id as u64, t.exec_id as u64, e.kind, e.side))
        .collect();
    got.sort();
    assert_eq!(
        got,
        vec![
            (10, 11, EdgeKind::Delegation, true),
            (10, 12, EdgeKind::Delegation, true),
            (10, 13, EdgeKind::Delegation, true),
        ],
        "辺は親 → 各子の委譲だけ"
    );
}

/// 段は `batch_id` の `_r<N>_` ではなく、組の最小 exec_id の順で決まる。
/// 孫は線を本当の親（子）から引き、配置は一番上の親の段に入る。
#[test]
fn rows_follow_exec_id_and_grandchildren_join_root_rows() {
    let blocks = fixture_blocks();
    let (dag, lg) = build_block_graph(&blocks, &[]);
    let layout = calculate_layout(&lg, true);

    // 親 10: バッチ A（最小 id 11）が段 1、バッチ B（13）が段 2（r の数字は B の方が小さい）
    assert!(pos(&layout, 13).y > pos(&layout, 11).y);
    // 親 20: 子 21 が段 1、孫 22 が段 2
    assert_eq!(pos(&layout, 21).y, pos(&layout, 20).y);
    assert!(pos(&layout, 22).y > pos(&layout, 21).y);

    let has = |f: i64, t: i64| {
        dag.all_edges()
            .iter()
            .any(|(a, b, e)| a.exec_id == f && b.exec_id == t && e.kind == EdgeKind::Delegation)
    };
    assert!(has(21, 22), "孫の線は本当の親（21）から");
    assert!(!has(20, 22));
    assert!(!has(11, 12), "子どうしの鎖は作らない");
}

/// 日付ごとに区切り線が 1 本、ラベルは `yymmdd`。親が増えた分、次の行は下へ下がる。
#[test]
fn separators_per_date() {
    let (_, lg) = build_block_graph(&fixture_blocks(), &[]);
    let layout = calculate_layout(&lg, true);
    let labels: Vec<_> = layout.separators.iter().map(|s| s.label.as_str()).collect();
    assert_eq!(labels, vec!["261001", "261002"]);
    assert!(layout.separators[1].y > layout.separators[0].y);
}
