//! Kanban ボード用データ構築（TUI 状態側）。
//!
//! カード md の読み書き（frontmatter パース・作成・実行時更新）と
//! レーン分類を行う。分類ロジックの純粋部分は
//! `sai_link_engine::kanban` にあり単体テスト可能。

use std::path::PathBuf;

use sai_link_engine::kanban::KanbanLane;

#[cfg(test)]
use super::AppState;
use crate::ui::kanban::KanbanBoardData;
use crate::ui::kanban::KanbanCardView;

// 移設先（sai-link-widgets）からの再エクスポート。
// `crate::app::kanban::{CardEntry, …}` 参照（data.rs / input.rs / mod.rs）を維持する。
pub use sai_link_widgets::card_entries::{card_path_for_execution, CardEntry};
// `collect_card_entries_from` は `#[cfg(test)]` の `collect_card_entries` からのみ使う。
#[cfg(test)]
pub use sai_link_widgets::card_entries::collect_card_entries_from;







/// 互換用：現在の `AppState` から都度組み立てる。
///
/// 本番の描画・ツリー構築では `AppState::kanban_entries`（`load_data` 済み）を
/// 使い回すこと。ここでは従来通り DB・ファイル走査を行う。
#[cfg(test)]
pub fn collect_card_entries(state: &AppState) -> Vec<CardEntry> {
    let executions = state.store.list_recent_executions(1000).unwrap_or_default();
    let (todo_by_run_id, todo_unlinked) = super::data::scan_todo_cards();
    collect_card_entries_from(
        &state.store,
        &executions,
        &todo_by_run_id,
        &todo_unlinked,
        &state.mb_replica_execs,
    )
}

/// `CardEntry` 一覧をボード描画データへ変換する。
///
/// 選択中のカードは、次のどちらかで照合する。
///
/// 1. カード md のフルパス（同名カードでも誤照合しない）
/// 2. 実行 ID（タスク作成（`task_*`）由来など、カード md のパスが無い実行用。
///    パスが無いとレーン上で黄色にならなかった）
///
/// `scroll` はレーン共通の縦スクロール位置（フォーカス時のホイールで増減）。
///
/// `cost_labels` は `load_data` で 1 回だけ引いた完了実行の費用（exec_id → 表示文字列）。
/// 描画のたびに DB を引かないよう引数で受け取る。
pub fn to_board_data(
    entries: &[CardEntry],
    selected_card_path: Option<&std::path::Path>,
    selected_exec_id: Option<i64>,
    scroll: usize,
    cost_labels: &std::collections::HashMap<i64, String>,
) -> KanbanBoardData {
    let mut lanes: [Vec<KanbanCardView>; 4] = [Vec::new(), Vec::new(), Vec::new(), Vec::new()];
    let mut selected_lane: Option<usize> = None;
    let mut selected_card: Option<PathBuf> = None;
    let mut selected_exec: Option<i64> = None;
    for e in entries {
        let idx = lane_index(e.lane);
        lanes[idx].push(KanbanCardView {
            title: e.title.clone(),
            date: e.date.clone(),
            description: e.description.clone(),
            card_path: e.card_path.clone(),
            exec_id: e.exec_id,
            runner: e.runner.clone(),
            cost: e
                .exec_id
                .and_then(|id| cost_labels.get(&id))
                .cloned(),
        });
        let path_hit = matches!(
            (selected_card_path, e.card_path.as_deref()),
            (Some(sel), Some(p)) if sel == p
        );
        let exec_hit = matches!((selected_exec_id, e.exec_id), (Some(s), Some(p)) if s == p);
        if path_hit || exec_hit {
            selected_lane = Some(idx);
            selected_card = e.card_path.clone();
            selected_exec = e.exec_id;
        }
    }
    KanbanBoardData {
        lanes,
        selected_lane,
        selected_card_path: selected_card,
        selected_exec_id: selected_exec,
        scroll,
    }
}

/// レーン → インデックス（`LANE_NAMES` 順）。
fn lane_index(lane: KanbanLane) -> usize {
    match lane {
        KanbanLane::Todo => 0,
        KanbanLane::Running => 1,
        KanbanLane::Review => 2,
        KanbanLane::Done => 3,
    }
}

/// 4レーンそれぞれのカード枚数（Kanban スクロール上限の計算用）。
pub fn lane_counts(entries: &[CardEntry]) -> [usize; 4] {
    let mut counts = [0usize; 4];
    for e in entries {
        counts[lane_index(e.lane)] += 1;
    }
    counts
}

// ============================================================================
// テスト（Kanban / Workflow の排他）
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use sai_link_engine::db::models::BlockMetadata;
    use std::sync::Arc;

    /// SAI_PROJECT_ROOT はプロセスグローバルのため、ファイル系テストは共有ロック
    /// （`crate::app::file_test_lock()`）で直列化する。

    fn create_block(name: &str) -> BlockMetadata {
        BlockMetadata::new(
            name.to_string(),
            "テスト用".to_string(),
            "m".to_string(),
            None,
            "off".to_string(),
            "true".to_string(),
            "true".to_string(),
            None,
            300,
            "none".to_string(),
            true,
            format!("blocks/{}.md", name),
            None
        )
    }

    /// Card（長方形）は Kanban ボードと Workflow DAG のどちらか一方にのみ存在する。
    /// d 前は Kanban ボードのみ、d 後（`workflow_moved=true`）は Workflow DAG のみ。
    /// ファイル（review / カード）はどちらも残る。
    #[test]
    fn card_exists_in_only_one_of_kanban_or_workflow() {
        let _guard = crate::app::file_test_lock();
        let tmp = std::env::temp_dir().join(format!("sai_kanban_excl_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(tmp.join(".sai")).unwrap();
        std::env::set_var("SAI_PROJECT_ROOT", &tmp);

        let store = Arc::new(sai_link_engine::db::SaiStore::open_in_memory().unwrap());
        let block_id = store.insert_block(&create_block("b")).unwrap();
        let e1 = store.create_execution(block_id, None).unwrap();
        store
            .update_execution_overrides(e1, None, Some(1), Some(1), None, None, None)
            .unwrap();
        store.update_execution_status(e1, "completed").unwrap();
        // 完了時に review テンプレが作られる想定（空テンプレ = review レーン）
        let review = sai_link_engine::orchestrator::review_path(e1);
        sai_link_engine::orchestrator::write_review_template(e1, "b", &[]).unwrap();

        let mut app = AppState::new(Arc::clone(&store));
        app.load_data();
        // d 前：Workflow DAG に存在しない
        assert!(app.block_displays.iter().all(|b| b.exec_id != e1));
        // d 前：Kanban ボードには review レーンで存在する
        let entries = collect_card_entries(&app);
        assert!(entries
            .iter()
            .any(|c| c.exec_id == Some(e1) && c.lane == KanbanLane::Review));

        // d：`workflow_moved=true` で Workflow へ移動（Kanban から消える）。
        // review ファイルは残る。
        store.update_execution_workflow_moved(e1, true).unwrap();
        assert!(review.exists(), "review ファイルは残る");
        app.load_data();
        let entries = collect_card_entries(&app);
        assert!(
            entries.iter().all(|c| c.exec_id != Some(e1)),
            "Kanban から消える"
        );
        assert!(
            app.block_displays.iter().any(|b| b.exec_id == e1),
            "Workflow に現れる"
        );

        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// 実行中は Kanban の running に存在し、Workflow には存在しない。
    #[test]
    fn running_is_kanban_only() {
        let _guard = crate::app::file_test_lock();
        let tmp = std::env::temp_dir().join(format!("sai_kanban_run_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(tmp.join(".sai")).unwrap();
        std::env::set_var("SAI_PROJECT_ROOT", &tmp);

        let store = Arc::new(sai_link_engine::db::SaiStore::open_in_memory().unwrap());
        let block_id = store.insert_block(&create_block("b")).unwrap();
        let e1 = store.create_execution(block_id, None).unwrap();
        store
            .update_execution_overrides(e1, None, Some(1), Some(1), None, None, None)
            .unwrap();
        store.update_execution_status(e1, "running").unwrap();

        let mut app = AppState::new(Arc::clone(&store));
        app.load_data();
        assert!(app.block_displays.iter().all(|b| b.exec_id != e1));
        let entries = collect_card_entries(&app);
        assert!(entries
            .iter()
            .any(|c| c.exec_id == Some(e1) && c.lane == KanbanLane::Running));

        let _ = std::fs::remove_dir_all(&tmp);
    }
}
