//! Workflow の DAG を組み立てる材料集め。DB から Workflow（DAG）用の材料を
//! 1 つの関数（[`load_workflow`]）で集める。CLI（`workflow`）がこれを使う。
//!
//! 使うもの:
//! - [`SaiStore`] — DB 読み
//! - [`ViewPrep`] — `card_entries::prepare_view`（実行一覧・todo 辞書・Kanban 札）

use std::collections::{HashMap, HashSet};

use agent_link_engine::db::models::ExecutionHistory;
use agent_link_engine::db::SaiStore;

use super::build::{BlockDisplayData, ConnectionDisplayData};
use crate::card_entries::{description_for_execution_cached, ViewPrep};

/// Workflow の DAG を組み立てる材料。
#[derive(Debug, Clone, Default)]
pub struct WorkflowSource {
    /// `workflow_moved == true` の実行だけ（DAG に載せる物）
    pub blocks: Vec<BlockDisplayData>,
    /// 実行単位の接続（waggle_token / task_delegation / task_output / git_commit）
    pub connections: Vec<ConnectionDisplayData>,
    /// `status=running` の取り残し（幽霊）の実行 ID
    pub orphaned_runs: HashSet<i64>,
    /// 完了した実行の費用の札（例 `'$0.42'` / `費用不明`）
    pub cost_labels: HashMap<i64, String>,
}

/// 実行が Kanban の所有か（Workflow との排他判定）。
///
/// `workflow_moved`（`d` で設定）が真 = Workflow 所有、偽 = Kanban 所有。
/// ファイル（review / カード）は削除しないため、ファイル有無では判定しない。
fn is_kanban_owned(e: &ExecutionHistory) -> bool {
    !e.workflow_moved
}

/// Workflow の DAG の材料を DB から集める。
///
/// - 幽霊（`list_running_executions`）を集め、実行一覧は `prep.executions` を再利用する
/// - 費用の札は完了（`completed` か `finalized`）した実行だけ
/// - `blocks` は `workflow_moved == true` の実行だけ（Kanban との排他）
/// - `include_subtasks` が偽のとき、`parent_run_id` がある実行（`task create` で作った子）は
///   `blocks` に入れない
pub fn load_workflow(
    store: &SaiStore,
    prep: &ViewPrep,
    include_subtasks: bool,
) -> WorkflowSource {
    // 幽霊（`status='running'` の取り残し）の実行 ID 集合。
    let orphaned_runs: HashSet<i64> = store
        .list_running_executions()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|e| e.id)
        .collect();

    let executions: &[ExecutionHistory] = &prep.executions;

    // 完了実行の費用（Card 表示用。例 `$0.42` / `費用不明`）。
    // 費用不明の判定は engine の `usage::total_from_row` に寄せます。
    let usage_map = store.execution_usage_total_map().unwrap_or_default();
    let mut cost_labels: HashMap<i64, String> = HashMap::new();
    for e in executions {
        // 完了した実行（`completed` か finalized）のカードにだけ出す
        if e.status != "completed" && !e.finalized {
            continue;
        }
        let Some(id) = e.id else { continue };
        let cost = usage_map
            .get(&id)
            .map(|row| agent_link_engine::usage::total_from_row(row).cost)
            .unwrap_or(None);
        cost_labels.insert(id, crate::fmt::format_cost(cost));
    }

    // ブロック名などのキャッシュ（ブロック ID ごと）
    let mut block_name_cache: HashMap<i64, String> = HashMap::new();
    let mut blocks: Vec<BlockDisplayData> = Vec::new();
    for e in executions {
        if is_kanban_owned(e) || (!include_subtasks && e.parent_run_id.is_some()) {
            continue;
        }
        let block_id = e.block_id;
        // ブロック名をキャッシュから取得（基本情報は block_metadata）
        let name = match block_name_cache.get(&block_id) {
            Some(cached) => cached.clone(),
            None => {
                let name = store
                    .get_block_by_id(block_id)
                    .map(|b| b.display_name)
                    .unwrap_or_else(|_| format!("(block_id={})", block_id));
                block_name_cache.insert(block_id, name.clone());
                name
            }
        };
        // Card 表示用の description：対応カードの frontmatter から取得する。
        // 対応カードが無い実行は空（todo 辞書を使い回し・再走査なし）。
        let description = description_for_execution_cached(
            store,
            e.kanban_card_path.as_deref(),
            e.id.unwrap_or(0),
            &prep.todo_by_run_id,
        );
        blocks.push(BlockDisplayData {
            exec_id: e.id.unwrap_or(0),
            block_id,
            name,
            date: agent_link_engine::run_dir::root_run_date(store, e),
            parent_exec_id: e.parent_run_id,
            batch_id: e.batch_id.clone(),
            last_status: e.status.clone(),
            last_duration_ms: e.duration_ms,
            created_at: e.created_at.clone(),
            description,
            model: e.model.clone(),
            thinking: e.thinking.clone(),
        });
    }

    // 接続情報（実行単位。waggle_token / task_delegation / task_output を含む）
    let connections: Vec<ConnectionDisplayData> = store
        .list_connections()
        .unwrap_or_default()
        .iter()
        .map(|c| ConnectionDisplayData {
            source_execution_id: c.source_execution_id,
            target_execution_id: c.target_execution_id,
            connection_type: c.connection_type.clone(),
        })
        .collect();

    WorkflowSource {
        blocks,
        connections,
        orphaned_runs,
        cost_labels,
    }
}
