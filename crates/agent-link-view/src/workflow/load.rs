//! Workflow の DAG を組み立てる材料集め（agent-link-tui の `app/data.rs` から移設）。
//!
//! TUI の `AppState::load_data` のうち Workflow（DAG）用の部分を 1 つの関数にまとめたもの。
//! TUI と CLI（`agent-link phone workflow`）は**同じ手順・同じ条件**で材料を集める。
//!
//! 使うもの:
//! - [`SaiStore`] — DB 読み
//! - [`ViewPrep`] — `card_entries::prepare_view`（実行一覧・todo 辞書・Kanban 札）

use std::collections::{HashMap, HashSet};

use agent_link_engine::db::models::ExecutionHistory;
use agent_link_engine::db::SaiStore;
use agent_link_engine::reflection_status::ReflectionStatus;

use super::build::{BlockDisplayData, ConnectionDisplayData};
use crate::card_entries::{description_for_execution_cached, ViewPrep};

/// Workflow の DAG を組み立てる材料（TUI の `load_data` と同じ手順で集める）。
#[derive(Debug, Clone, Default)]
pub struct WorkflowSource {
    /// `workflow_moved == true` の実行だけ（DAG に載せる物）
    pub blocks: Vec<BlockDisplayData>,
    /// 実行単位の接続（waggle_token / task_delegation / task_output / git_commit）
    pub connections: Vec<ConnectionDisplayData>,
    /// sai-write 検出済み（finalized の run_batch を持つ）実メインの実行 ID
    pub finalized_runs: HashSet<i64>,
    /// `status=running` の取り残し（幽霊）の実行 ID
    pub orphaned_runs: HashSet<i64>,
    /// 完了した実行の費用の札（例 `'$0.42'` / `費用不明`）
    pub cost_labels: HashMap<i64, String>,
    /// 全実行（DAG 載否を問わず）の review の記入状況
    pub reflection: HashMap<i64, ReflectionStatus>,
}

/// 実行が Kanban の所有か（Workflow との排他判定）。
///
/// `workflow_moved`（`d` で設定）が真 = Workflow 所有、偽 = Kanban 所有。
/// ファイル（review / カード）は削除しないため、ファイル有無では判定しない。
fn is_kanban_owned(e: &ExecutionHistory) -> bool {
    !e.workflow_moved
}

/// review（`reflection`）の記入状況（`.agent-link/reflection/<run_id>.md`）を判定する。
///
/// 完了・失敗など**終端ステータスの実行のみ**判定する（実行中・待機中は
/// 色分け対象外のため `None` 扱い = 辞書に載せない）。
fn judge_reflection_statuses(
    executions: &[ExecutionHistory],
) -> HashMap<i64, ReflectionStatus> {
    let mut out: HashMap<i64, ReflectionStatus> = HashMap::new();
    for e in executions {
        let Some(id) = e.id else { continue };
        if e.status != "completed" && e.status != "failed" {
            continue;
        }
        let path = agent_link_engine::orchestrator::reflection_path(id);
        out.insert(id, ReflectionStatus::judge_file(&path));
    }
    out
}

/// Workflow の DAG の材料を DB から集める（TUI の `load_data` と同じ手順・同じ条件）。
///
/// - mb 複製の実行 ID と finalized 親（`list_all_run_batch` の `Finalized`）、
///   幽霊（`list_running_executions`）を集め、実行一覧は `prep.executions` を再利用する
/// - 費用の札は完了（`completed` か `finalized`）した実行だけ
/// - `blocks` は `workflow_moved == true` の実行だけ（Kanban との排他）
/// - `reflection` は全実行を対象にする（DAG 載否にかかわらず sidebar でも色分けする）
pub fn load_workflow(store: &SaiStore, prep: &ViewPrep) -> WorkflowSource {
    // sai-write 検出済み（finalized 実行バッチを持つ）実メインの集合。
    // 最終フェーズノード（P_n = sai-write 由来）を合成する判定に使用。
    let finalized_runs: HashSet<i64> = store
        .list_all_run_batches()
        .unwrap_or_default()
        .into_iter()
        .filter(|rb| rb.status == agent_link_engine::db::models::RunBatchStatus::Finalized)
        .map(|rb| rb.parent_run_id)
        .collect();

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

    // Notification 判定は全実行対象（DAG 載否にかかわらず覚え置きする）。
    let reflection = judge_reflection_statuses(executions);

    // ブロック名などのキャッシュ（ブロック ID ごと）
    let mut block_name_cache: HashMap<i64, String> = HashMap::new();
    let mut blocks: Vec<BlockDisplayData> = Vec::new();
    for e in executions {
        if is_kanban_owned(e) {
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
            // 実実行はフェーズなし（合成ノードのみ設定）
            phase: None,
            last_status: e.status.clone(),
            last_duration_ms: e.duration_ms,
            created_at: e.created_at.clone(),
            description,
            // Notification 色分け用の記入状況（実行中・待機中は対象外で None）
            reflection_status: reflection.get(&e.id.unwrap_or(0)).copied(),
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
        finalized_runs,
        orphaned_runs,
        cost_labels,
        reflection,
    }
}
