//! # タスク管理モジュール
//!
//! 通知ディレクトリを監視し、`task_created` / `task_execute_request` の
//! 通知ファイルを処理します。サブタスクのライフサイクル全体を管理します。
//!
//! ## 通知プロトコル
//!
//! エージェント側（`agent-link task create` など）が `.agent-link/engine/notifications/pending/` に
//! JSON 通知ファイルを書き込み、デーモンがそれを監視・処理します。
//!
//! ### 通知種別
//!
//! - `task_created` — サブタスク作成要求（バッチ確認）
//! - `task_execute_request` — サブタスク実行要求
//!
//! ### 結果通知 (デーモン → エージェント側)
//!
//! - `task_completed` — `.agent-link/engine/notifications/results/` に書き込み
//! - `task_failed` — 同上

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::agent_pool::{AgentPool, EnqueueResult};
use crate::db::models::Task;
use crate::db::SaiStore;
use crate::pane::{PaneClient, SplitDirection};
use crate::orchestrator::RunningExecutions;

// ============================================================================
// 通知/結果ディレクトリ定数（common に一元化）
// ============================================================================

/// 通知ディレクトリのベースパス（プロジェクトルート基準）
fn notify_base_dir() -> PathBuf {
    crate::common::notify_base_dir()
}
/// 未処理通知の格納先
const PENDING_DIR: &str = crate::common::NOTIFY_PENDING;
/// 処理済み通知の格納先
const PROCESSED_DIR: &str = crate::common::NOTIFY_PROCESSED;
/// 完了結果の格納先（デーモン → エージェント側）
const RESULTS_DIR: &str = crate::common::NOTIFY_RESULTS;

// ============================================================================
// 通知データ型
// ============================================================================

/// `task_created` 通知の JSON 構造。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct TaskCreatedNotification {
    /// 通知タイプ: "task_created"
    #[serde(rename = "type")]
    pub notification_type: String,
    /// 親エージェントの実行 ID
    pub parent_run_id: i64,
    /// バッチ確認用グループ ID
    pub batch_id: String,
    /// 修正ラウンド番号（1 始まり。task_create 再送のたびに増加）
    #[serde(default)]
    pub round: i64,
    /// 置き換え元バッチ ID（修正版バッチ作成時）
    #[serde(default)]
    pub supersedes_batch_id: Option<String>,
    /// 作成するタスクの配列
    pub tasks: Vec<TaskItem>,
}

/// 個別タスクの定義（通知内）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskItem {
    /// エージェント側の一時 ID（結果紐付け用）
    pub client_id: String,
    /// タスク件名
    pub subject: String,
    /// タスク詳細
    pub description: String,
    /// 使用するブロック定義名（省略時は親と同じ）
    #[serde(default)]
    pub block_type: Option<String>,
    /// ブロック入力値
    #[serde(default)]
    pub inputs: Option<serde_json::Value>,
    /// 依存するタスクの client_id 配列
    #[serde(default)]
    pub depends_on: Vec<String>,
    /// worktree 分離設定
    #[serde(default = "default_worktree")]
    pub worktree: String,
    /// サブタスクの作業フォルダ（親が指定。省略時はプロジェクトルート）
    #[serde(default)]
    pub cwd: Option<String>,
    /// 子の時間上限（秒。省略時はブロック定義の `timeout:` に従う）
    #[serde(default)]
    pub timeout: Option<u32>,
    /// 起こす todo カードの run_id（U2。`task_create` の `card` 引数）
    ///
    /// `Some` のときはこの run_id の予約行をそのまま子として使う（新たに採番しない）。
    /// 通知の項目名は `card`（呼び出し側の指定にそろえる）。
    #[serde(rename = "card", default)]
    pub card_run_id: Option<i64>,
    /// 子の model（`task_create` の model。省略時はカード → ブロック定義に従う）
    #[serde(default)]
    pub model: Option<String>,
    /// 子の thinking（`task_create` の thinking。省略時はカード → ブロック定義に従う）
    #[serde(default)]
    pub thinking: Option<String>,
    /// 遠隔で動かす子の ssh Host 名（`task_create` の host。`None` は Mac で動かす）。`cwd` は遠隔のフォルダ
    #[serde(default)]
    pub host: Option<String>,
    /// 前バッチで承認済み・内容変更なし（引き継ぎ）
    #[serde(default)]
    pub carried_over: bool,
    /// ユーザーの個別承認結果（pane 上で決定）
    #[serde(default)]
    pub approval: Option<TaskApproval>,
}

/// 個別タスクの承認結果（pane 上でのユーザー判断）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct TaskApproval {
    /// "approved" | "rejected"
    pub status: String,
    /// 否認理由（ユーザーコメント）
    #[serde(default)]
    pub comment: Option<String>,
}

fn default_worktree() -> String {
    // 空 = 未指定。ブロック定義の `worktree:` に従う（以前の既定 "none" は
    // ブロック定義の isolation を打ち消してしまうため、未指定と none を区別する）。
    String::new()
}

/// タスクの `worktree` 指定を実行時オーバーライド値に変換する。
///
/// - 空（未指定）→ `None`（オーバーライドせず、ブロック定義の `worktree:` に従う）
/// - `"none"` → `Some("none")`（明示的に作業用コピーを作らない。
///   ブロック定義の isolation も打ち消す）
/// - その他 → `Some(そのまま)`（リテラルブランチ名）
pub(crate) fn task_worktree_override(raw: &str) -> Option<String> {
    match raw.trim() {
        "" => None,
        "none" => Some("none".to_string()),
        other => Some(other.to_string()),
    }
}

/// `task_execute_request` 通知の JSON 構造。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct TaskExecuteRequestNotification {
    /// 通知タイプ: "task_execute_request"
    #[serde(rename = "type")]
    pub notification_type: String,
    /// 親エージェントの実行 ID
    pub parent_run_id: i64,
    /// 実行するタスク ID（単体指定・後方互換）
    #[serde(default)]
    pub task_id: Option<i64>,
    /// バッチ単位で実行する場合のバッチ ID
    #[serde(default)]
    pub batch_id: Option<String>,
}

/// `task_manage_request` 通知の JSON 構造（P4）。
///
/// メインブロックがサブブロック（タスク）のライフサイクルを操作する。
/// `action`: "delete" | "finalize" | "send" | "requeue"
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct TaskManageRequestNotification {
    /// 通知タイプ: "task_manage_request"
    #[serde(rename = "type")]
    pub notification_type: String,
    /// 親エージェントの実行 ID
    pub parent_run_id: i64,
    /// 操作種別
    pub action: String,
    /// 対象タスク ID
    pub task_id: i64,
    /// 追加入力指示（action=send 時のみ）
    #[serde(default)]
    pub message: Option<String>,
}

/// `task_output_used` 通知の JSON 構造。
///
/// `task_output` がサブブロックの出力（waggle トークン）を
/// 親ブロックに返却したことを通知する。engine 側で sub→main の
/// `task_output` 接続を作成するために使用する。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct TaskOutputUsedNotification {
    /// 通知タイプ: "task_output_used"
    #[serde(rename = "type")]
    pub notification_type: String,
    /// 親エージェントの実行 ID
    pub parent_run_id: i64,
    /// 出力を受け取ったタスクの ID
    pub task_id: i64,
    /// 受け取った waggle トークン一覧
    #[serde(default)]
    pub waggle_tokens: Vec<String>,
}

/// `task_completed` 結果通知（デーモン → エージェント側）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskCompletedResult {
    /// 通知タイプ: "task_completed"
    #[serde(rename = "type")]
    pub notification_type: String,
    /// タスク ID
    pub task_id: i64,
    /// 親エージェントの実行 ID
    pub parent_run_id: i64,
    /// 完了ステータス
    pub status: String,
    /// エージェントの最終出力テキスト
    pub output: String,
    /// 発行された waggle トークン一覧（最終ラウンドのみ）
    #[serde(default)]
    pub waggle_tokens: Vec<String>,
    /// コミット ID（コード変更ブロックの場合）
    pub commit_id: Option<String>,
    /// 実行時間（ミリ秒）
    pub elapsed_ms: u64,
    /// セッション統計情報
    pub session_stats: Option<serde_json::Value>,
    /// 実行ラウンド番号（task_manage(send) で増加）
    #[serde(default)]
    pub round: i64,
}

/// `task_failed` 結果通知（デーモン → エージェント側）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskFailedResult {
    /// 通知タイプ: "task_failed"
    #[serde(rename = "type")]
    pub notification_type: String,
    /// タスク ID
    pub task_id: i64,
    /// 親エージェントの実行 ID
    pub parent_run_id: i64,
    /// 失敗ステータス
    pub status: String,
    /// エラーメッセージ
    pub error: String,
}

// ============================================================================
// TaskManager
// ============================================================================

/// バッチ実行時にタスクへ割り当てられるタブ内スロット（U2・R4）。
///
/// 発行元メインブロック（parent_run_id）単位で管理されるタブ（U2・R4 案X）。
///
/// 同じメインブロックによって作成されたサブタスクはすべて同じタブに pane を作成する。
/// タブ作成時に生成されたルートペイン（空シェル）は列計算から除外するため保持する。
#[derive(Debug, Clone)]
pub struct TabGroup {
    /// メインブロック専用のタブ ID
    pub tab_id: String,
    /// タブ作成時に生成されたルートペイン ID（未使用時は空シェル・列計算から除外）
    pub root_pane_id: String,
    /// ルートペインを最初のサブエージェントに割り当て済みか。
    ///
    /// `true` ならルートペインも作業 pane として数える（空シェルとして扱わない）。
    pub root_used: bool,
    /// バッチ（実行バッチ run_batch_id）ごとの行アンカー（最初の pane）（U12・R24）。
    /// 縦配置のため、新規バッチは直前バッチのアンカーを Down 分割して行を追加する。
    pub batch_anchors: std::collections::HashMap<i64, String>,
    /// バッチの最後に追加された pane（同一バッチ内を Right 分割して横並びにするための基底）。
    pub batch_last: std::collections::HashMap<i64, String>,
    /// バッチの出現順（行の縦順）。
    pub batch_order: Vec<i64>,
}

/// タブ内の実 pane の列構造から、次の pane 配置（分割基底 + 方向）を決定する純関数（案X）。
///
/// ルール（ユーザー指定タイルレイアウトを実データで検証済み）:
/// - 実 pane が 0 個 → ルートペインを `Right` 分割して最初の列を作る。
/// - 列数 C < cols → 右端列（max x）の最上 pane（min y）を `Right` 分割 → 新しい列を右に追加。
/// - 列数 C == cols → 最小 pane 数の列（同数タイ時は左端）の最上 pane を `Down` 分割 → 既存列の下に追加。
/// - 上限（WIP）に達したら `None`（分割不可・待機）。
///
/// `cols` は WIP から導出（`layout_for_wip`: n<=4 → cols=n、n>4 → cols=ceil(n/2)）。
/// 最悪でも `cols*2 >= WIP` を満たすため、WIP 個までの pane を隙間なく配置できる。
///
/// # 引数
///
/// * `effective` — root を除いた実 pane の `(pane_id, x, y)` 一覧（同 x は同一列）。
/// * `root_pane_id` — タブのルートペイン ID（最初の列作成時に基底として使用）。
///
/// # 戻り値
///
/// `(分割基底paneID, 分割方向)`。上限(WIP)到達時は `None`。
pub fn next_split_target(
    effective: &[(String, i64, i64)],
    root_pane_id: &str,
) -> Option<(String, SplitDirection)> {
    // 実 pane が無い → ルートペインを Right 分割して最初の列を作る
    if effective.is_empty() {
        return Some((root_pane_id.to_string(), SplitDirection::Right));
    }

    // WIP 上限に達している場合は分割不可（待機）
    let wip = crate::settings::wip_limit();
    if effective.len() >= wip {
        return None;
    }

    // 列数は WIP から導出する（n<=4 → cols=n、n>4 → cols=ceil(n/2)）
    let (_cols, _rows) = crate::pane::layout_for_wip(wip, wip);
    let max_cols = if wip <= 4 { wip } else { (wip + 1) / 2 };

    // 列グループ化（x で分類し、各列は y 昇順に並べる）
    let mut cols: Vec<(i64, Vec<(String, i64)>)> = Vec::new();
    for (pid, x, y) in effective {
        match cols.iter_mut().find(|(cx, _)| *cx == *x) {
            Some((_, list)) => list.push((pid.clone(), *y)),
            None => cols.push((*x, vec![(pid.clone(), *y)])),
        }
    }
    for (_, list) in cols.iter_mut() {
        list.sort_by_key(|(_, y)| *y);
    }
    let c = cols.len();

    if c < max_cols {
        // 右端列（max x）の最上 pane（min y）を Right 分割（新しい列を右に追加）
        let right = cols.iter().max_by_key(|(x, _)| *x).unwrap();
        let base = right.1.first().unwrap().0.clone();
        return Some((base, SplitDirection::Right));
    }

    // C == max_cols: 最小 pane 数の列（タイは左端）を Down 分割（既存列の下に追加）
    let min_count = cols.iter().map(|(_, l)| l.len()).min().unwrap();
    let min_col = cols.iter().find(|(_, l)| l.len() == min_count).unwrap();
    let base = min_col.1.first().unwrap().0.clone();
    Some((base, SplitDirection::Down))
}

/// 動的 pane 配置の結果。
///
/// `resolve_pane_placement` の戻り値。
pub enum PlacementResult {
    /// 配置完了（タブID, 作成済みpaneID, 分割方向）
    Placed(String, String, SplitDirection),
    /// WIP 上限到達（待機。エラーでない）
    Waiting,
}

/// バッチ（実行バッチ run_batch_id）単位の縦配置の分割決定（U12・R24 純関数）。
///
/// - 同一バッチ内: 行の右端 pane（`batch_last`）を `Right` 分割して横並び（sb2 等）。
/// - 新規バッチ: 直前バッチのアンカー（`batch_anchors`）を `Down` 分割して縦に展開（mb1.1）。
/// - 最初のバッチ: `root` を `Right` 分割して最初の行を作る（mb1）。
///
/// `batch_anchors`・`batch_last` に残った pane が既に閉じられている場合
/// （確定・削除で pane だけが消えた場合）は、その pane を基点にしない。
/// 代わりに今ある pane から決める `next_split_target` にフォールバックする。
///
/// # 引数
///
/// * `effective` — root を除いた、タブ内に今ある実 pane の `(pane_id, x, y)` 一覧。
/// * `root_exists` — ルートペインがタブ内に今もあるか（未使用 root の生死判定用）。
///
/// # 戻り値
///
/// `(分割基底paneID, 分割方向, 新規バッチか)`。WIP 上限に達していて分割できないときは `None`。
pub fn resolve_batch_split(
    g: &TabGroup,
    rb: i64,
    root_pane_id: &str,
    effective: &[(String, i64, i64)],
    root_exists: bool,
) -> Option<(String, SplitDirection, bool)> {
    // 今の規則どおりに基点と方向を決める
    let (cand, dir, new_batch) = if let Some(last) = g.batch_last.get(&rb) {
        // 同一バッチ内: 行の右端を Right 分割して横並び
        (last.clone(), SplitDirection::Right, false)
    } else {
        // 新規バッチ: 直前バッチのアンカーを Down 分割（縦展開）
        let is_first = g.batch_order.is_empty();
        let base = if is_first {
            root_pane_id.to_string()
        } else {
            g.batch_anchors
                .get(g.batch_order.last().unwrap())
                .cloned()
                .unwrap_or_else(|| root_pane_id.to_string())
        };
        let dir = if is_first {
            SplitDirection::Right
        } else {
            SplitDirection::Down
        };
        (base, dir, true)
    };

    // 基点が今もあるか確かめる。閉じられていたらグリッド配置へ戻す。
    // 未使用のルートペインだけは `effective` から除かれているため別に判定する。
    let alive = effective.iter().any(|(p, _, _)| *p == cand)
        || (cand == root_pane_id && !g.root_used && root_exists);
    if alive {
        Some((cand, dir, new_batch))
    } else {
        // 新しいバッチかどうかは元の判定のまま返す（呼び出し側のアンカー登録のため）
        next_split_target(effective, root_pane_id).map(|(b, d)| (b, d, new_batch))
    }
}

/// タスク管理マネージャー。
///
/// 通知ディレクトリの監視、タスクの CRUD 操作、バッチ確認、
/// 結果通知の書き込みを行います。
pub struct TaskManager {
    /// エージェント実行プール（同時実行制御）
    agent_pool: Arc<AgentPool>,
    /// データベースストア
    store: Arc<SaiStore>,
    /// 通知ディレクトリのベースパス
    notify_base: PathBuf,
    /// 発行元メインブロック（parent_run_id）→ 専用タブ（U2・R4 案X。動的 pane 配置用）
    block_tabs: std::sync::Mutex<std::collections::HashMap<i64, TabGroup>>,
    /// pane 配置（タブ取得・作成 → レイアウト取得 → 配置決定 → 分割 → 状態更新）を
    /// 1 つの原子的な処理にするための専用ロック（U2・R4 案X）。
    ///
    /// `block_tabs` は分割中は保持したくない（`maybe_close_parent_tab`
    /// などと競合するため）ので、配置全体を直列化するのはこのロックの役割。
    pane_placement_lock: std::sync::Mutex<()>,
}

impl TaskManager {
    /// 新しい `TaskManager` を作成する。
    pub fn new(store: Arc<SaiStore>) -> Self {
        let notify_base = notify_base_dir();
        // 必要なディレクトリを事前作成
        let _ = fs::create_dir_all(notify_base.join(PENDING_DIR));
        let _ = fs::create_dir_all(notify_base.join(PROCESSED_DIR));
        let _ = fs::create_dir_all(notify_base.join(RESULTS_DIR));

        Self {
            agent_pool: Arc::new(AgentPool::new()),
            store,
            notify_base,
            block_tabs: std::sync::Mutex::new(std::collections::HashMap::new()),
            pane_placement_lock: std::sync::Mutex::new(()),
        }
    }

    /// カスタム `AgentPool` を使用する `TaskManager` を作成する。
    pub fn with_agent_pool(store: Arc<SaiStore>, agent_pool: Arc<AgentPool>) -> Self {
        let notify_base = notify_base_dir();
        let _ = fs::create_dir_all(notify_base.join(PENDING_DIR));
        let _ = fs::create_dir_all(notify_base.join(PROCESSED_DIR));
        let _ = fs::create_dir_all(notify_base.join(RESULTS_DIR));

        Self {
            agent_pool,
            store,
            notify_base,
            block_tabs: std::sync::Mutex::new(std::collections::HashMap::new()),
            pane_placement_lock: std::sync::Mutex::new(()),
        }
    }

    // -----------------------------------------------------------------------
    // task_created — サブタスク作成
    // -----------------------------------------------------------------------

    /// `task_created` 通知を処理する。
    ///
    /// - 承認（`approval.status == "approved"`）タスクのみ DB に保存する。
    ///   旧形式（approval なし）は従来どおり `pending` で保存（CLI approve フロー）。
    /// - 拒否タスクは保存しない（pane 上の承認 UI でメインブロックへ返却済み）。
    /// - `supersedes_batch_id` 指定時は旧バッチの未完了タスクを `deleted` で無効化する。
    ///
    /// # 戻り値
    ///
    /// 作成されたタスクの server_id 一覧
    pub fn process_task_created(
        &self,
        notification: &TaskCreatedNotification,
    ) -> Result<Vec<i64>, TaskManagerError> {
        let mut task_ids = Vec::new();

        // supersedes: 置き換え元バッチの未完了タスクを無効化（メインブロックによる削除）
        if let Some(ref prev_batch) = notification.supersedes_batch_id {
            let prev_tasks = self
                .store
                .list_tasks_by_batch(prev_batch)
                .map_err(|e| TaskManagerError::Db(e.to_string()))?;
            for t in prev_tasks {
                let movable = matches!(
                    t.status.as_str(),
                    "pending" | "approved" | "queued" | "waiting_dependency" | "waiting_pane"
                );
                if movable {
                    if let Some(tid) = t.id {
                        let _ = self.store.mark_task_deleted(tid, "main");
                    }
                }
                // completed / finalized / failed / deleted は履歴として残す
            }
        }

        for item in &notification.tasks {
            // 拒否タスクは保存しない（通知・戻り値のみでメインブロックへ返す）
            let is_approved = match &item.approval {
                None => true, // 旧形式（承認情報なし）は保存する
                Some(a) => a.status == "approved",
            };
            if !is_approved {
                continue;
            }

            let inputs_json = item
                .inputs
                .as_ref()
                .and_then(|v| serde_json::to_string(v).ok());

            let depends_on_json = if item.depends_on.is_empty() {
                None
            } else {
                Some(serde_json::to_string(&item.depends_on).unwrap_or_default())
            };

            let mut task = Task::new(
                notification.parent_run_id,
                item.subject.clone(),
                item.description.clone(),
                item.block_type.clone(),
                inputs_json,
                depends_on_json,
                item.worktree.clone(),
                Some(notification.batch_id.clone()),
                Some(item.client_id.clone()),
            );
            // round / supersedes / コメントを反映
            task.round = notification.round.max(1);
            task.supersedes_batch_id = notification.supersedes_batch_id.clone();
            task.approval_comment = item.approval.as_ref().and_then(|a| a.comment.clone());
            // 作業フォルダ（task_create の cwd。解決・検証済みの絶対パス）
            task.cwd = item.cwd.clone();
            // 子の時間上限（task_create の timeout・秒）
            task.timeout = item.timeout;
            // 起こす todo カード（task_create の card。予約 run_id をそのまま子として使う）
            task.card_run_id = item.card_run_id;
            // 子の model・thinking（task_create の引数）
            task.model = item.model.clone();
            task.thinking = item.thinking.clone();
            // 遠隔で動かす子の ssh Host 名（task_create の host）
            task.host = item.host.clone();
            // 承認情報付き（新形式）なら approved で直接保存（CLI approve 不要）
            if item.approval.is_some() {
                task.status = "approved".to_string();
            }

            let id = self
                .store
                .insert_task(&task)
                .map_err(|e| TaskManagerError::Db(e.to_string()))?;
            task_ids.push(id);
        }

        Ok(task_ids)
    }

    // -----------------------------------------------------------------------
    // バッチ確認（承認/拒否）
    // -----------------------------------------------------------------------

    /// バッチ内の全タスクを `approved` に変更する。
    ///
    /// # 戻り値
    ///
    /// 承認されたタスクの ID 一覧
    pub fn approve_batch(&self, batch_id: &str) -> Result<Vec<i64>, TaskManagerError> {
        self.store
            .update_batch_status(batch_id, "approved")
            .map_err(|e| TaskManagerError::Db(e.to_string()))
    }

    /// バッチ内の全タスクを削除する（拒否時）。
    pub fn reject_batch(&self, batch_id: &str) -> Result<(), TaskManagerError> {
        self.store
            .delete_tasks_by_batch(batch_id)
            .map_err(|e| TaskManagerError::Db(e.to_string()))
    }

    // -----------------------------------------------------------------------
    // task_execute — サブタスク実行
    // -----------------------------------------------------------------------

    /// `task_execute_request` 通知を処理する。
    ///
    /// タスクが `approved` ステータスであることを確認し、
    /// `AgentPool` にエンキューします。
    /// 空きスロットがあれば即時実行を開始します。
    pub fn process_task_execute(&self, task_id: i64) -> Result<ExecuteResult, TaskManagerError> {
        match self.try_promote(task_id)? {
            PromoteResult::Started { task_id } => Ok(ExecuteResult::Started { task_id }),
            PromoteResult::Queued { task_id, position } => {
                Ok(ExecuteResult::Queued { task_id, position })
            }
            PromoteResult::WaitingDependency { task_id } => {
                Ok(ExecuteResult::WaitingDependency { task_id })
            }
            PromoteResult::NotReady => {
                // approved 以外のステータス → 従来どおり InvalidStatus エラー
                let task = self
                    .store
                    .get_task_by_id(task_id)
                    .map_err(|e| TaskManagerError::Db(e.to_string()))?;
                Err(TaskManagerError::InvalidStatus {
                    task_id,
                    current: task.status,
                    expected: "approved".to_string(),
                })
            }
        }
    }

    /// バッチ単位で実行要求を処理する（`task_execute_request` の batch_id 指定時・U2）。
    ///
    /// バッチ内の `approved` タスクを**事前カウント**し、上限超過（`MAX_BATCH_PANES`）を
    /// 即時検出して pane を一切作成せずに失敗させます（R5）。閾値内なら各タスクを
    /// AgentPool へ投入します（実際の pane 分割はワーカーの `execute_subtask` が
    /// 実行時・動的に行う。U2・R4 案X）。
    ///
    /// # 戻り値
    ///
    /// 各タスクの実行結果（n==0 の場合は空リスト）。
    ///
    /// # エラー
    ///
    /// - `TaskManagerError::BatchPaneLimitExceeded` — 承認タスク数が上限超過（呼び出し元は
    ///   `{"error":"pane_limit_exceeded","required_finalize":true}` を tool call result に載せる）
    pub fn process_batch_execute(
        &self,
        batch_id: &str,
    ) -> Result<Vec<ExecuteResult>, TaskManagerError> {
        let tasks = self
            .store
            .list_tasks_by_batch(batch_id)
            .map_err(|e| TaskManagerError::Db(e.to_string()))?;

        // 親実行の特定（バッチ内タスクは同一親想定。無ければ早期 return）
        let parent_run_id = match tasks.iter().find(|t| t.parent_run_id > 0) {
            Some(t) => t.parent_run_id,
            None => return Ok(Vec::new()),
        };

        // R4: 実行対象（approved）タスクの ID を ID 順で収集し、事前カウントする
        let approved_ids: Vec<i64> = tasks
            .iter()
            .filter(|t| t.status == "approved")
            .filter_map(|t| t.id)
            .collect();
        let n = approved_ids.len();

        // n==0: 実行対象なし → 早期 return（pane・実行バッチを一切作成しない）
        if n == 0 {
            return Ok(Vec::new());
        }

        // R5: バッチ内の承認タスク数が max_concurrent（グローバル上限）を超える場合は
        // 明示的な上限超過（toolResult で finalize 要求）。WIP（タブごと）超過は待機で対処する。
        let max_concurrent = crate::settings::max_concurrent_limit();
        if n > max_concurrent {
            return Err(TaskManagerError::BatchPaneLimitExceeded {
                batch_id: batch_id.to_string(),
                n,
                limit: max_concurrent,
            });
        }

        // U2・R4 案X: 実行バッチ（メイン複製 mb1.x）を新規作成する。
        // 同一メインブロックの task_create バッチをまたいだ承認タスクを 1 実行バッチに集約するため、
        // この親の「未所属（run_batch_id 未設定）の approved タスク」も対象に加える。
        let parent_tasks = self
            .store
            .list_tasks_by_parent(parent_run_id)
            .map_err(|e| TaskManagerError::Db(e.to_string()))?;
        let mut run_targets: Vec<i64> = parent_tasks
            .iter()
            .filter(|t| t.status == "approved" && t.run_batch_id.is_none())
            .filter_map(|t| t.id)
            .collect();
        run_targets.sort_unstable();
        run_targets.dedup();
        if run_targets.is_empty() {
            return Ok(Vec::new());
        }

        // 実行バッチ作成（run_batches 行）。
        // P ノード（フェーズノード）は実行レコードとしては作成しない（表示側が合成する）。
        // 以前はここでメイン複製用実行レコード（mb_execution_id）を作成していたが廃止。
        let seq = self
            .store
            .next_run_batch_seq(parent_run_id)
            .map_err(|e| TaskManagerError::Db(e.to_string()))?;
        let run_batch_id = format!("E{}", seq);
        let rb_id = self
            .store
            .create_run_batch(parent_run_id, &run_batch_id, seq, None)
            .map_err(|e| TaskManagerError::Db(e.to_string()))?;

        // 対象タスクに run_batch_id を紐付け、実行投入する
        let mut results = Vec::new();
        for tid in &run_targets {
            let _ = self.store.update_task_run_batch(*tid, rb_id);
        }
        eprintln!(
            "[task_manager] 実行バッチ {} を作成: parent={} seq={} 対象タスク {} 件",
            run_batch_id,
            parent_run_id,
            seq,
            run_targets.len()
        );
        for tid in run_targets {
            results.push(self.process_task_execute(tid)?);
        }
        Ok(results)
    }

    // -----------------------------------------------------------------------
    // 動的 pane 配置（U2・R4 案X）
    // -----------------------------------------------------------------------

    /// 発行元メインブロック（parent_run_id）単位のタブを取得・作成し、
    /// 次の pane 配置（分割基底 + 方向）を現在のタブ内列構造から動的に決定する。
    ///
    /// タブ取得・作成から pane 分割、状態更新までを 1 つの原子的な処理として行い、
    /// **作成済み pane の ID** を返す。
    ///
    /// 複数のワーカースレッドが同時に同じタブへ pane を追加するため、
    /// `pane_placement_lock` を処理全体で保持して直列化する。これにより、
    /// 次の競合を防ぐ:
    /// - ルートペインの二重割り当て（同じ pane に複数タスクが乗る）
    /// - 同じ基底 pane への重複分割（同一 pane ID の重複、anchor の二重登録）
    /// - WIP 上限判定の同時通過
    ///
    /// # 戻り値
    ///
    /// `PlacementResult::Placed(tab_id, pane_id, direction)` または
    /// `PlacementResult::Waiting`（WIP 上限到達。待機・エラーでない）。
    fn resolve_pane_placement(&self, task: &Task) -> Result<PlacementResult, TaskManagerError> {
        // 配置専用ロック（U2・R4 案X）。再試行は `_inner` 内の再帰で行うため、
        // ここで 1 回だけ取得すれば二重ロックにならない。
        let _placement_guard = self.pane_placement_lock.lock().unwrap();
        self.resolve_pane_placement_inner(task, true)
    }

    /// `resolve_pane_placement` の実体（呼び出し元が `pane_placement_lock` を保持済み）。
    ///
    /// sai-tree ではタブ・レイアウト・分割を使わない。擬似 pane を 1 つ作って
    /// 返すだけにし、同時実行の上限は `AgentPool` に任せる。
    fn resolve_pane_placement_inner(
        &self,
        task: &Task,
        _allow_retry: bool,
    ) -> Result<PlacementResult, TaskManagerError> {
        // host 付きの子の cwd は遠隔のパスなので、Mac の pane（cwd.txt・tmux -c）には渡さない
        let pane_cwd = if task.host.is_some() { None } else { task.cwd.as_deref() };
        let pane = crate::headless::new_pane(pane_cwd)
            .map_err(|e| TaskManagerError::Pane(format!("Failed to create the pseudo pane: {}", e)))?;
        Ok(PlacementResult::Placed(
            "headless".to_string(),
            pane.pane_id,
            SplitDirection::Right,
        ))
    }

    /// sai-tree ではタブを使わないので何もしない。
    ///
    /// `block_tabs` は常に空（読み取りだけ残して未使用警告を避ける）。
    fn maybe_close_parent_tab(&self, parent_run_id: i64) {
        let _ = self.block_tabs.lock().unwrap().get(&parent_run_id);
    }

    // -----------------------------------------------------------------------
    // task_manage — サブブロックライフサイクル操作（P4）
    // -----------------------------------------------------------------------

    /// `task_manage_request` 通知を処理する。
    ///
    /// メインブロックがサブブロック（タスク）の全権限を持ち、以下の操作を行う:
    /// - `delete`: タスクを削除（実行中なら AgentPool からキャンセル + pane 強制終了）
    /// - `finalize`: 完了を確定し pane を削除（最終ラウンドのトークン確定）
    /// - `send`: 同一 pane のエージェントに追加指示を送信して継続作業させる（ターン送信）
    /// - `requeue`: failed / deleted タスクを approved に戻して再評価
    ///
    /// # 戻り値
    ///
    /// 処理メッセージ + 実行を開始すべきタスク ID 一覧（send / requeue の新ラウンド）
    /// および同一 pane へのターン送信対象のタスク ID 一覧（send の新実装）。
    pub fn process_task_manage(
        &self,
        action: &str,
        task_id: i64,
        message: Option<&str>,
    ) -> Result<PollEvent, TaskManagerError> {
        let task = self
            .store
            .get_task_by_id(task_id)
            .map_err(|e| TaskManagerError::Db(e.to_string()))?;

        match action {
            "delete" => {
                // 実行中なら AgentPool からキャンセル + pane 強制終了
                if task.status == "in_progress" {
                    self.agent_pool.cancel(task_id);
                    if let Some(exec_id) = task.execution_id {
                        if let Ok(exec) = self.store.get_execution_by_id(exec_id) {
                            if let Some(pane_id) = exec.herdr_pane_id {
                                let _ = crate::orchestrator::close_execution_pane(&pane_id);
                            }
                        }
                    }
                }
                // U2・R4 案X: タスク削除後、親タブが空になればタブ自体を閉じる
                self.maybe_close_parent_tab(task.parent_run_id);
                self.store
                    .mark_task_deleted(task_id, "main")
                    .map_err(|e| TaskManagerError::Db(e.to_string()))?;
                // pane を閉じ・タスクを deleted にしたので、run フォルダを確定後の
                // owner（review）へ移す。sai-write 仮完了のまま running に残さない。
                if let Some(exec_id) = task.execution_id {
                    crate::run_dir::sync_run_owner_by_execution_id(&self.store, exec_id);
                }
                // U2・R4 案X: pane が解放されたため、waiting_pane タスクを再評価して昇格させる
                let mut started = self.promote_waiting_pane_tasks(task.parent_run_id)?;
                Ok(PollEvent {
                    message: format!(
                        "task_manage(delete): task_id={} を削除しました（deleted_by=main）。waiting_pane 再評価={:?}",
                        task_id, started
                    ),
                    started_task_ids: std::mem::take(&mut started),
                    turn_task_ids: Vec::new(),
                })
            }
            "finalize" => {
                // 最新ラウンドの pane を削除し、完了を確定する
                if let Some(exec_id) = task.execution_id {
                    if let Ok(exec) = self.store.get_execution_by_id(exec_id) {
                        if let Some(pane_id) = exec.herdr_pane_id {
                            let _ = crate::orchestrator::close_execution_pane(&pane_id);
                        }
                    }
                }
                // U2・R4 案X: finalize で pane 削除後、親タブが空になればタブ自体を閉じる
                self.maybe_close_parent_tab(task.parent_run_id);
                self.store
                    .update_task_finalized(task_id, true)
                    .map_err(|e| TaskManagerError::Db(e.to_string()))?;
                self.store
                    .update_task_status(task_id, "finalized")
                    .map_err(|e| TaskManagerError::Db(e.to_string()))?;
                // pane を閉じ・タスクを finalized にしたので、run フォルダを確定後の
                // owner（review）へ移す（sai-write 仮完了の間は running に維持していた）。
                if let Some(exec_id) = task.execution_id {
                    crate::run_dir::sync_run_owner_by_execution_id(&self.store, exec_id);
                }
                // 依存待ちタスクを再評価し、finalized になったタスクに依存するタスクを昇格する。
                // （依存は finalized まで待つため、completed の時点では昇格しない。
                //   メインブロックが修正内容をチェック・マージしてから後続を実行させる）
                let mut started = Vec::new();
                match self.promote_waiting_tasks(task_id) {
                    Ok(ids) => started = ids,
                    Err(e) => {
                        eprintln!("[task_manager] finalize 後の依存昇格に失敗: {}", e);
                    }
                }
                // U2・R4 案X: pane が解放されたため、waiting_pane タスクも再評価して昇格させる
                match self.promote_waiting_pane_tasks(task.parent_run_id) {
                    Ok(ids) => started.extend(ids),
                    Err(e) => {
                        eprintln!(
                            "[task_manager] finalize 後の waiting_pane 昇格に失敗: {}",
                            e
                        );
                    }
                }
                Ok(PollEvent {
                    message: format!(
                        "task_manage(finalize): task_id={} の完了を確定しました（pane 削除）",
                        task_id
                    ),
                    started_task_ids: started,
                    turn_task_ids: Vec::new(),
                })
            }
            "send" => {
                // U1 バグ2修正: description への追記は「送信できる」ことが確定したあとに行う。
                // 以前は in_progress の検査より前に追記していたため、拒否された指示でも
                // 説明に書き残り、あとから send するたび説明が膨らんでいた。
                //
                // 追記を後ろに回しても `execute_task_turn` の `extract_latest_turn_message` は
                // 問題ない。process_task_manage はターン送信より前に同期で完了するため。
                let append_description =
                    |store: &crate::db::store::SaiStore| -> Result<(), TaskManagerError> {
                        let new_desc = match (task.description.as_str(), message) {
                            (d, Some(m)) if !d.is_empty() => {
                                format!("{}\n\n[追加入力指示]\n{}", d, m)
                            }
                            (_, Some(m)) => format!("[追加入力指示]\n{}", m),
                            (d, None) => d.to_string(),
                        };
                        store
                            .update_task_description(task_id, &new_desc)
                            .map_err(|e| TaskManagerError::Db(e.to_string()))
                    };

                // P? の修正: send は「同一 pane へのターン送信」で行う。
                // 新規 worktree / pane / ブロックは作成せず、生きているエージェントに
                // 追加指示テキストを送信して継続作業させる（コンテキスト維持）。
                // これにより、サブエージェント自身の実装意図を保ったまま
                // マージコンフリクト解消などの追加工事を指示できる。
                let exec_pane = task
                    .execution_id
                    .and_then(|eid| self.store.get_execution_by_id(eid).ok())
                    .and_then(|e| e.herdr_pane_id)
                    .filter(|p| !p.is_empty());
                // 実行レコードが既にある = 一度起動済み（実行中〜完了〜未 finalize）。
                let ever_executed = task.execution_id.is_some();

                // 実行中のタスクへの割り込み送信は禁止（pane の作業を乱さない）。
                // pane の有無に関係常にここで遮断する（実行中は pane 未記録のことが
                // あり、以前は「pane なし」と判定されて新規実行が開始され、
                // 同じ作業の子が二重に起動していた）。
                if task.status == "in_progress" {
                    return Err(TaskManagerError::InvalidStatus {
                        task_id,
                        current: task.status,
                        expected: "completed".to_string(),
                    });
                }

                if exec_pane.is_some() {
                    append_description(&self.store)?;
                    // 同一 pane へのターン送信（ワーカーの execute_task_turn が処理）
                    Ok(PollEvent {
                        message: format!(
                            "task_manage(send): task_id={} に追加入力指示を送信します（同一 pane でターン実行）",
                            task_id
                        ),
                        started_task_ids: Vec::new(),
                        turn_task_ids: vec![task_id],
                    })
                } else if ever_executed {
                    // 実行済みなのに pane が無い → 再起動すると同じ作業の子が二重になる。
                    // 再実行は requeue の明示操作のみ許可する。
                    Err(TaskManagerError::PaneMissingForTurn { task_id })
                } else {
                    // pane が無い場合（未実行等）は従来どおり新規実行へ（後方互換）
                    append_description(&self.store)?;
                    Ok(PollEvent {
                        message: format!(
                            "task_manage(send): task_id={} に追加指示を反映し新ラウンドを開始します（pane が無いため）",
                            task_id
                        ),
                        started_task_ids: vec![task_id],
                        turn_task_ids: Vec::new(),
                    })
                }
            }
            "requeue" => {
                // failed / deleted タスクを approved に戻して再評価
                if task.status != "failed" && task.status != "deleted" {
                    return Err(TaskManagerError::InvalidStatus {
                        task_id,
                        current: task.status,
                        expected: "failed | deleted".to_string(),
                    });
                }
                self.store
                    .update_task_status(task_id, "approved")
                    .map_err(|e| TaskManagerError::Db(e.to_string()))?;
                let mut started = Vec::new();
                match self.try_promote(task_id)? {
                    PromoteResult::Started { .. } => started.push(task_id),
                    _ => {}
                }
                Ok(PollEvent {
                    message: format!("task_manage(requeue): task_id={} を再実行します", task_id),
                    started_task_ids: started,
                    turn_task_ids: Vec::new(),
                })
            }
            other => Err(TaskManagerError::NotFound(format!(
                "Unknown task_manage action: {}",
                other
            ))),
        }
    }

    // -----------------------------------------------------------------------
    // 実行可否判定（P3: 依存関係・実行上限）
    // -----------------------------------------------------------------------

    /// `approved` タスクの依存関係を確認する。
    ///
    /// `depends_on`（client_id 配列）の全タスクが **`finalized`（完了確定）** なら true。
    /// 依存タスクが `completed`（仮完了）の時点では false（待機継続）。
    ///
    /// ## 設計判断（レビュー前の誤実装の引き継ぎ防止）
    ///
    /// 依存タスクは「仮完了（completed）」ではなく「完了確定（finalized）」まで待つ。
    /// メインブロックが `task_manage(finalize)` で修正内容をチェック・マージしてから
    /// 後続タスクを実行させることで、間違った実装を後続タスクが引き継ぐのを防ぐ。
    fn dependencies_completed(&self, task: &Task) -> Result<bool, TaskManagerError> {
        let deps = crate::db::store::parse_depends_on(task.depends_on_json.as_deref());
        if deps.is_empty() {
            return Ok(true);
        }
        for dep in deps {
            let dep_task = self
                .store
                .get_task_by_client_id(task.parent_run_id, &dep)
                .map_err(|e| TaskManagerError::Db(e.to_string()))?;
            let done = dep_task
                .as_ref()
                .map(|t| t.status == "finalized")
                .unwrap_or(false);
            if !done {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// タスクの実行可否を判定し、実行可能なら AgentPool へ投入する。
    ///
    /// 判定順序:
    /// 1. ステータスが `approved` である
    /// 2. 依存（`depends_on`）がすべて完了している
    /// 3. 実行上限（AgentPool 空き）がある → `in_progress` / `queued`
    ///
    /// 依存未完了なら `waiting_dependency` で待機させる。
    /// 完了時昇格（`complete_task` / `fail_task`）からも呼ばれる。
    pub fn try_promote(&self, task_id: i64) -> Result<PromoteResult, TaskManagerError> {
        let task = self
            .store
            .get_task_by_id(task_id)
            .map_err(|e| TaskManagerError::Db(e.to_string()))?;

        // 1. 承認状態チェック（approved / waiting_dependency / queued / waiting_pane のみ再評価可能）
        if !matches!(
            task.status.as_str(),
            "approved" | "waiting_dependency" | "queued" | "waiting_pane"
        ) {
            return Ok(PromoteResult::NotReady);
        }

        // 2. 依存チェック
        if !self.dependencies_completed(&task)? {
            self.store
                .update_task_status(task_id, "waiting_dependency")
                .map_err(|e| TaskManagerError::Db(e.to_string()))?;
            return Ok(PromoteResult::WaitingDependency { task_id });
        }

        // 3. 実行上限（AgentPool）判定
        match self.agent_pool.enqueue(task_id) {
            EnqueueResult::Started => {
                self.store
                    .update_task_status(task_id, "in_progress")
                    .map_err(|e| TaskManagerError::Db(e.to_string()))?;
                Ok(PromoteResult::Started { task_id })
            }
            EnqueueResult::Queued { position } => {
                self.store
                    .update_task_status(task_id, "queued")
                    .map_err(|e| TaskManagerError::Db(e.to_string()))?;
                Ok(PromoteResult::Queued { task_id, position })
            }
            EnqueueResult::QueueFull => Err(TaskManagerError::QueueFull),
        }
    }

    /// 依存待ちタスクを再評価し、実行可能になったものを AgentPool へ投入する。
    ///
    /// `complete_task` / `fail_task` から呼ばれる。
    /// 戻り値: 実行を開始すべき（Started）タスク ID 一覧。
    /// ※ Queued は AgentPool 内で待機し、スロット解放時に昇格されるため含めない。
    fn promote_waiting_tasks(&self, completed_task_id: i64) -> Result<Vec<i64>, TaskManagerError> {
        let waiting = self
            .store
            .get_tasks_waiting_on(completed_task_id)
            .map_err(|e| TaskManagerError::Db(e.to_string()))?;

        let mut started = Vec::new();
        for w in waiting {
            if let Some(wid) = w.id {
                match self.try_promote(wid)? {
                    PromoteResult::Started { .. } => started.push(wid),
                    _ => {}
                }
            }
        }
        Ok(started)
    }

    /// pane が解放された（finalize / delete）際に、同一メインブロックの
    /// `waiting_pane` タスクを再評価して実行昇格させる（U2・R4 案X）。
    ///
    /// `try_promote` は WIP 空き・依存・AgentPool を再判定するため、
    /// WIP が空いた分のタスクが自動で実行される。戻り値は昇格（Started）したタスク ID。
    fn promote_waiting_pane_tasks(&self, parent_run_id: i64) -> Result<Vec<i64>, TaskManagerError> {
        let tasks = self
            .store
            .list_tasks_by_parent(parent_run_id)
            .map_err(|e| TaskManagerError::Db(e.to_string()))?;
        let mut started = Vec::new();
        for t in tasks {
            if t.status == "waiting_pane" {
                if let Some(tid) = t.id {
                    match self.try_promote(tid)? {
                        PromoteResult::Started { .. } => started.push(tid),
                        _ => {}
                    }
                }
            }
        }
        Ok(started)
    }

    // -----------------------------------------------------------------------
    // task_output — サブ出力の返却（sub→main 接続作成）
    // -----------------------------------------------------------------------

    /// `task_output_used` 通知を処理する。
    ///
    /// 親ブロックが `task_output` でサブブロックの出力
    /// （waggle トークン）を受け取った場合に、サブ実行 → 親実行の
    /// `task_output` 接続を `block_connections` に作成する。
    ///
    /// 既に同じ組み合わせの接続が存在する場合は何もしない（冪等）。
    pub fn process_task_output_used(
        &self,
        notification: &TaskOutputUsedNotification,
    ) -> Result<String, TaskManagerError> {
        let task_id = notification.task_id;
        let parent_run_id = notification.parent_run_id;

        // タスクからサブ実行 ID を取得
        let task = self
            .store
            .get_task_by_id(task_id)
            .map_err(|e| TaskManagerError::Db(e.to_string()))?;
        let sub_exec_id = match task.execution_id {
            Some(eid) => eid,
            None => {
                return Ok(format!(
                    "task_output_used をスキップ: task_id={} はまだ実行されていません",
                    task_id
                ))
            }
        };

        if parent_run_id <= 0 {
            return Ok(format!(
                "task_output_used をスキップ: task_id={} の parent_run_id が不明 ({})",
                task_id, parent_run_id
            ));
        }

        // サブ実行・親実行のブロック ID を取得
        let sub_block_id = match self.store.get_execution_by_id(sub_exec_id) {
            Ok(e) => e.block_id,
            Err(e) => {
                return Err(TaskManagerError::Db(format!(
                    "サブ実行 {} の取得に失敗: {}",
                    sub_exec_id, e
                )))
            }
        };
        let parent_block_id = match self.store.get_execution_by_id(parent_run_id) {
            Ok(e) => e.block_id,
            Err(e) => {
                return Err(TaskManagerError::Db(format!(
                    "親実行 {} の取得に失敗: {}",
                    parent_run_id, e
                )))
            }
        };

        // task_output の接続先は常に実メイン（parent_run_id）。
        // フェーズノード（P_k）への割当は表示側でタスクのバッチ所属から合成するため、
        // engine 側はメイン複製（mb_execution_id）を参照しない。
        let target_exec_id = parent_run_id;

        // 冪等チェック（同じ sub→main の task_output 接続が既にあればスキップ）
        let exists = self
            .store
            .connection_exists(sub_exec_id, target_exec_id, "task_output")
            .unwrap_or(false);
        if exists {
            return Ok(format!(
                "task_output_used を処理: task_id={} の接続は既に存在します（sub={} → main={}）",
                task_id, sub_exec_id, target_exec_id
            ));
        }

        let conn_record = crate::db::BlockConnection::new_task_output(
            sub_exec_id,
            sub_block_id,
            target_exec_id,
            parent_block_id,
        );
        self.store
            .insert_connection(&conn_record)
            .map_err(|e| TaskManagerError::Db(format!("Failed to insert the task_output connection: {}", e)))?;

        Ok(format!(
            "task_output_used を処理: task_id={} の出力を親ブロックが受領 → 接続作成 (sub={} → main={})",
            task_id, sub_exec_id, target_exec_id
        ))
    }

    // -----------------------------------------------------------------------
    // タスク完了 → 結果通知
    // -----------------------------------------------------------------------

    /// サブタスクを実際に実行する（デーモンのワーカースレッドから呼ばれる）。
    ///
    /// `orchestrator::run_subtask`（内部で `execute_block_pipeline`）を同期的に呼び出し、
    /// 成功時は `complete_task()`、失敗時は `fail_task()` を呼びます。
    ///
    /// # 引数
    ///
    /// * `task_id` — 実行するタスク ID
    /// * `watcher_map` — デーモンの実行監視マップ（デーモン稼働時のみ。None なら自己ポーリング）
    ///
    /// # 戻り値
    ///
    /// 完了処理（`complete_task` / `fail_task`）で昇格したタスク ID 一覧
    /// （AgentPool 昇格 + 依存解決で昇格）。デーモンのワーカーが実行タスクキューに再投入する。
    pub fn execute_subtask(
        &self,
        task_id: i64,
        watcher_map: Option<&RunningExecutions>,
    ) -> Vec<i64> {
        // ステータスを in_progress に更新（Started は既に更新済み、昇格タスクは未更新）
        let _ = self.store.update_task_status(task_id, "in_progress");

        // タスク情報を取得
        let task = match self.store.get_task_by_id(task_id) {
            Ok(t) => t,
            Err(e) => {
                eprintln!("[task_manager] タスク {} の取得に失敗: {}", task_id, e);
                return self
                    .fail_task(task_id, &format!("Failed to get the task: {}", e))
                    .unwrap_or_default();
            }
        };

        // 6.8 の修正: block_type 未指定の場合は明確なエラーにしてから終了する。
        // （"default" に補完すると存在しないブロック定義ファイルとして汎用エラーになる）
        let block_type = match task.block_type.as_deref() {
            Some(bt) if !bt.is_empty() => bt.to_string(),
            _ => {
                let msg = format!(
                    "Task {} has no block_type. Give block_type in task_create (subject: {}).",
                    task_id, task.subject
                );
                eprintln!("[task_manager] {}", msg);
                return self.fail_task(task_id, &msg).unwrap_or_default();
            }
        };
        let inputs: Option<serde_json::Value> = task
            .inputs_json
            .as_deref()
            .and_then(|j| serde_json::from_str(j).ok());
        // タスクの worktree 指定（親の指定を最優先。未指定ならブロック定義に従う）。
        // 変換ルールは `task_worktree_override` を参照。
        let worktree = task_worktree_override(&task.worktree);
        // 作業フォルダ（親が指定した絶対パス。未指定ならプロジェクトルート）
        let task_cwd = task.cwd.clone();

        eprintln!(
            "[task_manager] サブタスク実行開始: task_id={}, block_type={}, subject={}",
            task_id, block_type, task.subject
        );

        // U2・R4 案X: 発行元メインブロック単位のタブで、実行時に現在の列構造から
        // 動的に pane 配置（分割）を決定する。タブロック下で分割まで行い作成済み pane を返す。
        // WIP 上限到達は「待機」（waiting_pane）とし、エラーにしない。
        let (tab_id, pane_id, _direction) = match self.resolve_pane_placement(&task) {
            Ok(PlacementResult::Placed(tab_id, pane_id, direction)) => (tab_id, pane_id, direction),
            Ok(PlacementResult::Waiting) => {
                // WIP 満杯: 待機状態へ移行し、pane 解放時に再評価されるようにする。
                eprintln!(
                    "[task_manager] WIP 上限到達: task_id={} を waiting_pane に移行（pane 解放待ち）",
                    task_id
                );
                let _ = self.store.update_task_status(task_id, "waiting_pane");
                let _ = self.agent_pool.complete(task_id);
                return Vec::new();
            }
            Err(e) => {
                let msg = format!("Failed to place a pane (task_id={}): {}", task_id, e);
                eprintln!("[task_manager] {}", msg);
                return self.fail_task(task_id, &msg).unwrap_or_default();
            }
        };

        match crate::orchestrator::run_subtask(
            &self.store,
            &block_type,
            &task.subject,
            &task.description,
            inputs.as_ref(),
            task.parent_run_id,
            worktree,
            task_cwd,              // 親が指定した作業フォルダ（絶対パス）
            task.timeout,          // 親が指定した子の時間上限（秒）
            crate::orchestrator::SubtaskSettings {
                model: task.model.clone(),       // 親が指定した子の model（None はカード → ブロック定義）
                thinking: task.thinking.clone(), // 親が指定した子の thinking（None はカード → ブロック定義）
                host: task.host.clone(),         // 遠隔で動かす子の ssh Host 名（None は Mac）
            },
            task.batch_id.clone(), // P3-A: タスクバッチID を実行レコードに伝播
            task.card_run_id,      // U2: todo のカードを自分の子として起こす場合の予約 run_id
            Some(tab_id),          // U2・R4: 動的配置で使用するタブ
            None,          // base_pane_id: 分割は resolve_pane_placement で実施済みのため不使用
            Some(pane_id), // U2・R4 案X: 分割済み pane を再利用（パイプライン内では分割しない）
            None,          // direction: 分割済みのため不要
            Some(task_id), // U3: 停止検知通知（idle/blocked）の紐付けに使用
            watcher_map,
        ) {
            Ok(pipeline_result) => {
                // 実行ID をタスクに紐づけ
                if let Some(exec_id) = pipeline_result.execution_id {
                    let _ = self.store.update_task_execution_id(task_id, exec_id);
                }

                let task_result = TaskResult {
                    output: pipeline_result.output.clone(),
                    waggle_tokens: pipeline_result.waggle_tokens.clone(),
                    commit_id: pipeline_result
                        .git_commit_after
                        .clone()
                        .or(pipeline_result.git_commit_before.clone()),
                    elapsed_ms: pipeline_result.elapsed_ms,
                    session_stats: pipeline_result
                        .session_stats_json
                        .as_ref()
                        .and_then(|j| serde_json::from_str(j).ok()),
                    execution_id: pipeline_result.execution_id,
                };

                eprintln!(
                    "[task_manager] サブタスク完了: task_id={}, elapsed_ms={}, waggle_tokens={:?}",
                    task_id, pipeline_result.elapsed_ms, pipeline_result.waggle_tokens
                );

                // ④ 結果通知（results/ への task_completed + AgentPool 解放 + 依存昇格）
                match self.complete_task(task_id, &task_result) {
                    Ok(next) => {
                        // P4: サブタスクは pane を維持（仮完了）。
                        // メインブロックが task_manage(finalize) で確定するまで残す
                        // （task_capture での調査・task_manage(send) での追加ラウンドに使用）。
                        eprintln!(
                            "[task_manager] サブタスク完了（pane 維持・finalize 待ち）: task_id={}, pane_id={}",
                            task_id, pipeline_result.pane_id
                        );
                        next
                    }
                    Err(e) => {
                        eprintln!("[task_manager] タスク {} の完了通知に失敗: {}", task_id, e);
                        Vec::new()
                    }
                }
            }
            Err(e) => {
                eprintln!(
                    "[task_manager] サブタスク失敗: task_id={}, error={}",
                    task_id, e
                );
                match self.fail_task(task_id, &e.to_string()) {
                    Ok(next) => next,
                    Err(e2) => {
                        eprintln!("[task_manager] タスク {} の失敗通知に失敗: {}", task_id, e2);
                        Vec::new()
                    }
                }
            }
        }
    }

    /// 同一 pane へのターン送信を実行する（task_manage(send) の新実装）。
    ///
    /// 新しい worktree / pane / ブロックは起動せず、既存のエージェント（pane）に
    /// 追加指示テキストを送信し、そのターンの完了を待つ。
    /// 同一エージェントのコンテキスト（自身の実装意図・作業履歴）が保たれたまま
    /// マージコンフリクト解消などの追加工事を行える。
    /// 完了後は実行レコードとタスクを更新し、`task_completed` 通知を発行する
    /// （メインブロックの task_output が結果を受信できるようにする）。
    ///
    /// # 引数
    ///
    /// * `task_id` — ターン送信するタスク ID
    /// * `_watcher_map` — デーモンの実行監視マップ（ターンでは pane 状態を直接監視するため不使用）
    ///
    /// # 戻り値
    ///
    /// 完了処理（complete_task / fail_task）で昇格したタスク ID 一覧。
    pub fn execute_task_turn(
        &self,
        task_id: i64,
        _watcher_map: Option<&RunningExecutions>,
    ) -> Vec<i64> {
        // 1. タスク取得
        let task = match self.store.get_task_by_id(task_id) {
            Ok(t) => t,
            Err(e) => {
                eprintln!(
                    "[task_manager] ターン送信: タスク {} の取得に失敗: {}",
                    task_id, e
                );
                return Vec::new();
            }
        };

        // 2. 実行レコードと pane の取得
        let Some(exec_id) = task.execution_id else {
            eprintln!(
                "[task_manager] ターン送信: タスク {} に実行レコードがありません",
                task_id
            );
            return Vec::new();
        };
        let exec = match self.store.get_execution_by_id(exec_id) {
            Ok(e) => e,
            Err(e) => {
                eprintln!(
                    "[task_manager] ターン送信: 実行 {} の取得に失敗: {}",
                    exec_id, e
                );
                return Vec::new();
            }
        };
        let Some(pane_id) = exec.herdr_pane_id.clone() else {
            eprintln!(
                "[task_manager] ターン送信: 実行 {} に pane がありません",
                exec_id
            );
            return Vec::new();
        };

        // 3. 追加指示メッセージの抽出（description の最後の [追加入力指示] 以降）
        let message = match Self::extract_latest_turn_message(&task.description) {
            Some(m) => m,
            None => {
                eprintln!(
                    "[task_manager] ターン送信: タスク {} の追加指示が抽出できません（description に [追加入力指示] がありません）",
                    task_id
                );
                return Vec::new();
            }
        };

        // 4. タイムアウト・セッション情報の解決（owner 移動追従のため現行正本を解決）
        // timeout 未指定は実質無期限（block.md に `timeout:` がある時だけ期限が効く）。
        let timeout_ms = exec
            .timeout
            .map(|s| s as u64 * 1000)
            .unwrap_or(crate::orchestrator::NO_TIMEOUT_MS);
        let session_dir = exec
            .id
            .and_then(|eid| crate::session::resolve_session_dir(exec.session_dir.as_deref(), eid))
            .or_else(|| exec.session_dir.clone().map(PathBuf::from));

        // 5. 状態を in_progress に（ターン実行中であることを示す）
        let _ = self.store.update_task_status(task_id, "in_progress");
        eprintln!(
            "[task_manager] ターン送信開始: task_id={}, pane_id={}",
            task_id, pane_id
        );

        // 6. 同一 pane へテキスト送信 + 完了待機
        let pane_client = PaneClient::new();
        let submit_key = crate::cc_command::CC_SUBMIT_KEY;
        let turn_result = crate::orchestrator::send_turn_to_pane(
            &pane_client,
            &pane_id,
            &message,
            timeout_ms,
            session_dir.as_deref().unwrap_or_else(|| Path::new(".")),
            submit_key,
            Some(task_id),
            Some(task.parent_run_id),
            Some(exec_id),
        );

        match turn_result {
            Ok(turn) => {
                // 実行レコードの更新（出力・ステータス・トークン・コミット）
                let _ = self
                    .store
                    .update_execution_output_content(exec_id, &turn.output);
                let _ = self.store.update_execution_status(exec_id, "completed");
                if !turn.waggle_tokens.is_empty() {
                    let _ = self
                        .store
                        .merge_execution_waggle_tokens(exec_id, &turn.waggle_tokens);
                }
                let _ = self.store.update_execution_git_commits(
                    exec_id,
                    None,
                    turn.git_commit_after.as_deref(),
                );
                eprintln!(
                    "[task_manager] ターン完了: task_id={}, elapsed_ms={}, waggle_tokens={:?}",
                    task_id, turn.elapsed_ms, turn.waggle_tokens
                );

                // タスク完了処理（task_completed 通知 → メインの task_output が受信）
                let task_result = TaskResult {
                    output: turn.output,
                    waggle_tokens: turn.waggle_tokens,
                    commit_id: turn.git_commit_after,
                    elapsed_ms: turn.elapsed_ms,
                    session_stats: None,
                    execution_id: Some(exec_id),
                };
                match self.complete_task(task_id, &task_result) {
                    Ok(next) => next,
                    Err(e) => {
                        eprintln!("[task_manager] ターン完了通知に失敗: {}", e);
                        Vec::new()
                    }
                }
            }
            Err(e) => {
                eprintln!(
                    "[task_manager] ターン送信失敗: task_id={}, error={}",
                    task_id, e
                );
                match self.fail_task(task_id, &format!("Failed to send the turn: {}", e)) {
                    Ok(next) => next,
                    Err(e2) => {
                        eprintln!("[task_manager] タスク {} の失敗通知に失敗: {}", task_id, e2);
                        Vec::new()
                    }
                }
            }
        }
    }

    /// description から最新の追加入力指示（最後の `[追加入力指示]` 以降）を抽出する。
    ///
    /// task_manage(send) は description に `[追加入力指示]` を追記するため、
    /// 同一 pane へのターン送信ではその末尾部分をメッセージとして送信する。
    fn extract_latest_turn_message(description: &str) -> Option<String> {
        const MARKER: &str = "[追加入力指示]";
        let parts: Vec<&str> = description.split(MARKER).collect();
        if parts.len() >= 2 {
            let last = parts.last().unwrap_or(&"").trim().to_string();
            if !last.is_empty() {
                return Some(last);
            }
        }
        None
    }

    /// タスクの正常完了を通知する（agent-link 内部から呼ばれる）。
    ///
    /// 1. DB のタスクステータスを `completed` に更新
    /// 2. AgentPool のスロットを解放（実行上限待ちタスクを昇格）
    /// 3. 依存待ち（`waiting_dependency`）タスクを再評価し、実行可能なら昇格
    /// 4. `.agent-link/engine/notifications/results/` に完了通知 JSON を書き込む
    ///
    /// # 戻り値
    ///
    /// 実行を開始すべきタスク ID 一覧（AgentPool 昇格 + 依存解決で昇格）。
    /// デーモンのワーカーがこれらを実行タスクキューへ再投入する。
    pub fn complete_task(
        &self,
        task_id: i64,
        result: &TaskResult,
    ) -> Result<Vec<i64>, TaskManagerError> {
        // タスク取得
        let task = self
            .store
            .get_task_by_id(task_id)
            .map_err(|e| TaskManagerError::Db(e.to_string()))?;

        // P4: deleted タスクは最終確定済みのため上書きしない（実行中削除との競合防止）
        let deleted = task.status == "deleted";
        if !deleted {
            self.store
                .update_task_status(task_id, "completed")
                .map_err(|e| TaskManagerError::Db(e.to_string()))?;
        }

        let mut next: Vec<i64> = Vec::new();

        // AgentPool のスロット解放（実行上限待ちタスクの昇格）
        if let Some(p) = self.agent_pool.complete(task_id) {
            next.push(p);
        }

        // 依存待ちタスクの再評価・昇格
        next.extend(self.promote_waiting_tasks(task_id)?);

        if !deleted {
            // 完了結果通知を書き込む（最終ラウンドの waggle トークン・round 付き）
            let completed = TaskCompletedResult {
                notification_type: "task_completed".to_string(),
                task_id,
                parent_run_id: task.parent_run_id,
                status: "completed".to_string(),
                output: result.output.clone(),
                waggle_tokens: result.waggle_tokens.clone(),
                commit_id: result.commit_id.clone(),
                elapsed_ms: result.elapsed_ms,
                session_stats: result.session_stats.clone(),
                round: task.round,
            };

            self.write_result_notification(task_id, &completed)?;
        }

        Ok(next)
    }

    /// タスクの失敗を通知する（agent-link 内部から呼ばれる）。
    ///
    /// # 戻り値
    ///
    /// 実行を開始すべきタスク ID 一覧（AgentPool 昇格 + 依存解決で昇格）。
    pub fn fail_task(&self, task_id: i64, error: &str) -> Result<Vec<i64>, TaskManagerError> {
        // タスク取得
        let task = self.store.get_task_by_id(task_id).map_err(|e| {
            // タスクが見つからない場合もエージェントプールのクリーンアップは行う
            TaskManagerError::Db(e.to_string())
        })?;

        // P4: deleted タスクは最終確定済みのため上書きしない（実行中削除との競合防止）
        let deleted = task.status == "deleted";
        if !deleted {
            self.store
                .update_task_status(task_id, "failed")
                .map_err(|e| TaskManagerError::Db(e.to_string()))?;
        }

        let mut next: Vec<i64> = Vec::new();

        // AgentPool のスロット解放（実行上限待ちタスクの昇格）
        if let Some(p) = self.agent_pool.complete(task_id) {
            next.push(p);
        }

        // 依存待ちタスクの再評価・昇格
        next.extend(self.promote_waiting_tasks(task_id)?);

        if !deleted {
            // 失敗結果通知を書き込む
            let failed = TaskFailedResult {
                notification_type: "task_failed".to_string(),
                task_id,
                parent_run_id: task.parent_run_id,
                status: "failed".to_string(),
                error: error.to_string(),
            };

            self.write_result_notification(task_id, &failed)?;
        }

        Ok(next)
    }

    // -----------------------------------------------------------------------
    // タスク一覧取得
    // -----------------------------------------------------------------------

    /// 親実行IDでサブタスク一覧を取得する。
    pub fn list_tasks(&self, parent_run_id: i64) -> Result<Vec<Task>, TaskManagerError> {
        self.store
            .list_tasks_by_parent(parent_run_id)
            .map_err(|e| TaskManagerError::Db(e.to_string()))
    }

    /// タスクIDで1件取得する。
    pub fn get_task(&self, task_id: i64) -> Result<Task, TaskManagerError> {
        self.store
            .get_task_by_id(task_id)
            .map_err(|e| TaskManagerError::Db(e.to_string()))
    }

    // -----------------------------------------------------------------------
    // 通知ファイルのポーリング（ディレクトリ監視）
    // -----------------------------------------------------------------------

    /// `pending/` ディレクトリ内の未処理通知を1件処理する。
    ///
    /// 通知の種類に応じて適切なハンドラを呼び出します。
    /// 処理後、通知ファイルは `processed/` に移動されます。
    /// ただし `file_written` や `block_run_request` など task 系以外の通知は
    /// スキップし、`pending/` に残します（それぞれ `doc_write_notify` / デーモンが処理する）。
    ///
    /// **Started タスクの同期実行は行いません**（§7.1.4）。
    /// デーモンが `started_task_ids` を受け取り、AgentPool ワーカーで非同期実行します。
    ///
    /// # 戻り値
    ///
    /// 処理結果（メッセージ + 実行開始可能なタスク ID 一覧）。未処理通知がなければ `None`。
    pub fn poll_one_event(&self) -> Result<Option<PollEvent>, TaskManagerError> {
        let pending_dir = self.notify_base.join(PENDING_DIR);
        let processed_dir = self.notify_base.join(PROCESSED_DIR);

        // pending ディレクトリ内の .json ファイルを列挙
        let mut entries: Vec<_> = fs::read_dir(&pending_dir)
            .map_err(|e| TaskManagerError::Io(e.to_string()))?
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.path()
                    .extension()
                    .map(|ext| ext == "json")
                    .unwrap_or(false)
            })
            .collect();

        // ファイル名でソート（古い順に処理）
        entries.sort_by_key(|e| e.file_name());

        // P7: 非 task 通知（file_written / block_run_request 等）はスキップして
        // 後続の task 通知を探す（1.13 の修正）。
        // それらは doc_write_notify / デーモンの別ルートが処理するため、
        // pending/ に残したまま先頭で諦めずに次の通知へ進む。
        for entry in entries {
            let notification_path = entry.path();
            let content = match fs::read_to_string(&notification_path) {
                Ok(c) => c,
                Err(_) => continue,
            };

            // 通知タイプを判別
            let type_value: serde_json::Value = match serde_json::from_str(&content) {
                Ok(v) => v,
                Err(_) => continue, // パース不能な通知はスキップ
            };

            let notif_type = type_value
                .get("type")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");

            // task 系以外の通知（file_written / block_run_request 等）は処理しない。
            // 移動せず pending/ に残し、doc_write_notify / デーモンに任せる。
            let is_task_notification = matches!(
                notif_type,
                "task_created"
                    | "task_execute_request"
                    | "task_output_used"
                    | "task_manage_request"
            );
            if !is_task_notification {
                continue;
            }

            let mut started_task_ids: Vec<i64> = Vec::new();
            let mut turn_task_ids: Vec<i64> = Vec::new();

            // 通知タイプに応じたハンドラを呼び出す（エラーは捕捉して後で返す）
            // ※ エラー時も通知は processed/ に移動する（再処理ループを防ぐ）
            let handler_result: Result<String, TaskManagerError> = match notif_type {
                "task_created" => match serde_json::from_str::<TaskCreatedNotification>(&content) {
                    Ok(notif) => match self.process_task_created(&notif) {
                        Ok(ids) => Ok(format!(
                            "task_created を処理: batch_id={}, {}件のタスクを作成 (IDs: {:?})",
                            notif.batch_id,
                            ids.len(),
                            ids
                        )),
                        Err(e) => Err(e),
                    },
                    Err(e) => Err(TaskManagerError::Json(e.to_string())),
                },
                "task_execute_request" => {
                    match serde_json::from_str::<TaskExecuteRequestNotification>(&content) {
                        Ok(notif) => {
                            // バッチ単位 or 単体指定を処理
                            let exec_results: Result<Vec<ExecuteResult>, TaskManagerError> =
                                if let Some(ref batch_id) = notif.batch_id {
                                    self.process_batch_execute(batch_id)
                                } else if let Some(task_id) = notif.task_id {
                                    self.process_task_execute(task_id).map(|r| vec![r])
                                } else {
                                    Err(TaskManagerError::NotFound(
                                        "task_execute_request has neither task_id nor batch_id.".to_string(),
                                    ))
                                };

                            match exec_results {
                                Ok(results) => {
                                    let summary: Vec<String> =
                                        results.iter().map(|r| format!("{:?}", r)).collect();
                                    let result_msg = format!(
                                        "task_execute_request を処理: {}",
                                        summary.join(", ")
                                    );

                                    // Started の場合は実行タスクキューに投入（デーモンのワーカーが非同期実行）
                                    for r in &results {
                                        if matches!(r, ExecuteResult::Started { .. }) {
                                            if let ExecuteResult::Started { task_id } = r {
                                                started_task_ids.push(*task_id);
                                            }
                                        }
                                    }

                                    Ok(result_msg)
                                }
                                Err(e) => Err(e),
                            }
                        }
                        Err(e) => Err(TaskManagerError::Json(e.to_string())),
                    }
                }
                "task_output_used" => {
                    match serde_json::from_str::<TaskOutputUsedNotification>(&content) {
                        Ok(notif) => self.process_task_output_used(&notif),
                        Err(e) => Err(TaskManagerError::Json(e.to_string())),
                    }
                }
                "task_manage_request" => {
                    match serde_json::from_str::<TaskManageRequestNotification>(&content) {
                        Ok(notif) => {
                            // Started（send / requeue の新ラウンド）は実行タスクキューに投入。
                            // 同一 pane へのターン送信（send の新実装）は turn_task_ids で返す。
                            //
                            // U1 バグ1修正: ここでは `?` で早期 return しない。
                            // この分岐の役目は「通知を 1 件処理して processed/ へ移す」ことであり、
                            // handler が Err を返しても processed/ への移動までは到達しなければならない。
                            // 以前の `?` により、エラー時は pending/ に通知が残ったまま
                            // 同じ通知が繰り返し再処理され、後続の通知を詰まらせていた。
                            self.process_task_manage(
                                &notif.action,
                                notif.task_id,
                                notif.message.as_deref(),
                            )
                            .map(|event| {
                                started_task_ids.extend(event.started_task_ids);
                                turn_task_ids.extend(event.turn_task_ids);
                                event.message
                            })
                        }
                        Err(e) => Err(TaskManagerError::Json(e.to_string())),
                    }
                }
                _ => Ok(format!("不明な通知タイプ '{}' をスキップ", notif_type)),
            };

            // 処理済みに移動（エラー時も移動して再処理を防ぐ）
            let filename = notification_path.file_name().unwrap_or_default();
            let dest = processed_dir.join(filename);
            let _ = fs::rename(&notification_path, &dest);

            match handler_result {
                Ok(result_msg) => {
                    return Ok(Some(PollEvent {
                        message: result_msg,
                        started_task_ids,
                        turn_task_ids,
                    }))
                }
                Err(e) => return Err(e),
            }
        }

        Ok(None)
    }

    /// `pending/` ディレクトリ内の未処理通知を1件処理する（後方互換・メッセージのみ）。
    pub fn poll_one(&self) -> Result<Option<String>, TaskManagerError> {
        Ok(self.poll_one_event()?.map(|e| e.message))
    }

    /// `pending/` ディレクトリ内の全未処理通知を処理する。
    pub fn poll_all(&self) -> Result<Vec<String>, TaskManagerError> {
        let mut results = Vec::new();
        loop {
            match self.poll_one_event()? {
                Some(event) => results.push(event.message),
                None => break,
            }
        }
        Ok(results)
    }

    // -----------------------------------------------------------------------
    // 結果通知の書き込み（デーモン → エージェント側）
    // -----------------------------------------------------------------------

    /// 結果通知 JSON を `results/` ディレクトリに書き込む。
    fn write_result_notification<T: Serialize>(
        &self,
        task_id: i64,
        result: &T,
    ) -> Result<(), TaskManagerError> {
        let results_dir = self.notify_base.join(RESULTS_DIR);
        fs::create_dir_all(&results_dir).map_err(|e| TaskManagerError::Io(e.to_string()))?;

        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();

        let filename = format!("{}-task_{}.json", timestamp, task_id);
        let path = results_dir.join(filename);

        let json = serde_json::to_string_pretty(result)
            .map_err(|e| TaskManagerError::Json(e.to_string()))?;

        fs::write(&path, json).map_err(|e| TaskManagerError::Io(e.to_string()))?;

        Ok(())
    }

    /// AgentPool への参照を取得する。
    pub fn agent_pool(&self) -> &Arc<AgentPool> {
        &self.agent_pool
    }
}

// ============================================================================
// 停止検知通知（U3: サブブロック停止/error 検知）
// ============================================================================

/// `task_stalled` 通知（U3）の JSON 構造。
///
/// サブブロックが 1分 idle または blocked で停止した際に orchestrator から
/// `results/` へ書き込み、メインブロックの `task_output` がポーリング取得できるようにする。
/// メインブロックはこの通知を受け取り `task_manage(send)` で同一 pane へ介入する（R6/R7）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct TaskStalledResult {
    /// 通知タイプ: "task_stalled"
    #[serde(rename = "type")]
    pub notification_type: String,
    /// 停止したサブタスクのタスク ID
    pub task_id: i64,
    /// 親実行 ID（出力返却関係の参照用）
    pub parent_run_id: Option<i64>,
    /// ステータス: "stalled"
    pub status: String,
    /// 停止理由: "idle" | "blocked" | "error" | "long_running" | "timeout" | "question"
    pub reason: String,
    /// 停止内容の説明（介入のヒントを含む）
    pub message: String,
    /// 子がした質問の ID（`reason` が "question" のときだけ入る）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub question_id: Option<String>,
}

/// 指定ディレクトリへ `task_stalled` 通知を書き込む内部ヘルパー（テスト容易性のため分離）。
fn write_stalled_notification_in(
    results_dir: &Path,
    task_id: i64,
    parent_run_id: Option<i64>,
    reason: &str,
    message: &str,
    question_id: Option<&str>,
) -> Result<(), String> {
    fs::create_dir_all(results_dir).map_err(|e| e.to_string())?;

    // 古い stalled 通知を消す（U2: アラートを繰り返すと親が古い通知を受け取るのを防ぐ）。
    // completed / failed のファイルは消さない。
    let suffix = format!("-task_{}.json", task_id);
    if let Ok(entries) = fs::read_dir(results_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            let name = path.file_name().unwrap_or_default().to_string_lossy().to_string();
            if !name.ends_with(&suffix) {
                continue;
            }
            let is_stalled = fs::read_to_string(&path)
                .ok()
                .and_then(|c| serde_json::from_str::<serde_json::Value>(&c).ok())
                .and_then(|v| {
                    v.get("type")
                        .and_then(|t| t.as_str())
                        .map(|t| t == "task_stalled")
                })
                .unwrap_or(false);
            if is_stalled {
                let _ = fs::remove_file(&path);
            }
        }
    }

    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();

    // `task_output` が `-task_<task_id>.json` で照合するため、命名規則を揃える。
    let filename = format!("{}-task_{}.json", timestamp, task_id);
    let path = results_dir.join(filename);

    let stalled = TaskStalledResult {
        notification_type: "task_stalled".to_string(),
        task_id,
        parent_run_id,
        status: "stalled".to_string(),
        reason: reason.to_string(),
        message: message.to_string(),
        question_id: question_id.map(str::to_string),
    };

    let json = serde_json::to_string_pretty(&stalled).map_err(|e| e.to_string())?;
    fs::write(&path, json).map_err(|e| e.to_string())?;
    Ok(())
}

/// プロジェクトの `results/` へ `task_stalled` 通知を書き込む（orchestrator から呼ばれる）。
pub fn write_stalled_notification(
    task_id: i64,
    parent_run_id: Option<i64>,
    reason: &str,
    message: &str,
    question_id: Option<&str>,
) -> Result<(), String> {
    let results_dir = notify_base_dir().join(RESULTS_DIR);
    write_stalled_notification_in(
        &results_dir,
        task_id,
        parent_run_id,
        reason,
        message,
        question_id,
    )
}

// ============================================================================
// 結果型
// ============================================================================

/// `process_task_execute` の戻り値。
#[derive(Debug, Clone)]
pub enum ExecuteResult {
    /// 即時実行開始
    Started { task_id: i64 },
    /// 実行上限待ち（AgentPool キュー）
    Queued { task_id: i64, position: usize },
    /// 依存タスク完了待ち
    WaitingDependency { task_id: i64 },
}

/// `try_promote` の戻り値（実行可否判定結果）。
#[derive(Debug, Clone)]
pub enum PromoteResult {
    /// 即時実行開始（AgentPool active に投入済み）
    Started { task_id: i64 },
    /// 実行上限待ち（AgentPool キューに投入済み）
    Queued { task_id: i64, position: usize },
    /// 依存タスク未完了（`waiting_dependency` で待機）
    WaitingDependency { task_id: i64 },
    /// 承認状態などにより実行不可（ステータス未変更）
    NotReady,
}

/// `poll_one_event` の戻り値。
#[derive(Debug, Clone)]
pub struct PollEvent {
    /// 処理結果の説明メッセージ
    pub message: String,
    /// 実行開始可能（Started）になったタスク ID 一覧
    /// （デーモンが実行タスクキューに投入して非同期実行する）
    pub started_task_ids: Vec<i64>,
    /// 同一 pane へのターン送信対象のタスク ID 一覧
    /// （task_manage(send) の新実装。デーモンが execute_task_turn で処理する）
    pub turn_task_ids: Vec<i64>,
}

/// タスク完了時に渡す結果情報。
#[derive(Debug, Clone, Default)]
pub struct TaskResult {
    /// エージェントの最終出力
    pub output: String,
    /// 発行された waggle トークン一覧
    pub waggle_tokens: Vec<String>,
    /// コミット ID（コード変更ブロックの場合）
    pub commit_id: Option<String>,
    /// 実行時間（ミリ秒）
    pub elapsed_ms: u64,
    /// セッション統計情報（JSON）
    pub session_stats: Option<serde_json::Value>,
    /// 実行ID（execution_history.id）
    pub execution_id: Option<i64>,
}

// ============================================================================
// エラー型
// ============================================================================

/// タスク管理操作のエラー型。
#[derive(Debug, thiserror::Error)]
pub enum TaskManagerError {
    #[error("Database error: {0}")]
    Db(String),

    #[error("I/O error: {0}")]
    Io(String),

    #[error("JSON error: {0}")]
    Json(String),

    #[error("Task {task_id} has the wrong status: current={current}, expected={expected}.")]
    InvalidStatus {
        task_id: i64,
        current: String,
        expected: String,
    },

    #[error("The execution queue is full (over the maximum length).")]
    QueueFull,

    #[error("Batch {batch_id} has {n} approved tasks, over the pane limit of {limit}. No pane is created and the batch fails at once (return required_finalize in the tool call result).")]
    BatchPaneLimitExceeded {
        /// 実行対象バッチ ID
        batch_id: String,
        /// 承認タスク数
        n: usize,
        /// pane 上限（MAX_BATCH_PANES = 8）
        limit: usize,
    },

    #[error("pane operation error: {0}")]
    Pane(String),

    #[error("The tab of main block {parent_run_id} reached the pane limit of {limit}, so no new pane can be created. Finalize unneeded panes and run again.")]
    PaneLimitReached {
        /// 発行元メインブロックの実行 ID（タブの識別子）
        parent_run_id: i64,
        /// pane 上限（MAX_BATCH_PANES = 8）
        limit: usize,
    },

    #[error("Task not found: {0}")]
    NotFound(String),

    #[error(
        "Task {task_id} was already run, but its working pane is not found. No new run starts, so the same subtask does not run twice. Run it again with task_manage(action=requeue)."
    )]
    PaneMissingForTurn {
        /// 対象タスク ID
        task_id: i64,
    },
}

// ============================================================================
// テスト
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn test_results_dir() -> PathBuf {
        // テスト用の一時ディレクトリ。
        // 並行実行されるテストどうしで名前が重ならないよう、单調増加カウンタを併用する
        // （SystemTime の分解能だけでは同じ値になりRaceしてファイル数アサーションが壊れる）。
        static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "sai-stalled-test-{}-{}",
            std::process::id(),
            seq
        ));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    // ---- 停止検知通知（U3） ----

    /// タスクの worktree 指定 → 実行時オーバーライド値の変換（3 通り）。
    ///
    /// - 未指定（空）は None にしてブロック定義に従わせる
    /// - "none" は明示的な無効化として Some("none")
    /// - その他はリテラルブランチ名
    #[test]
    fn test_task_worktree_override_three_states() {
        assert_eq!(task_worktree_override(""), None);
        assert_eq!(task_worktree_override("   "), None);
        assert_eq!(task_worktree_override("none"), Some("none".to_string()));
        assert_eq!(
            task_worktree_override("isolation"),
            Some("isolation".to_string())
        );
        assert_eq!(
            task_worktree_override("feature/x"),
            Some("feature/x".to_string())
        );
    }

    /// `task_stalled` 通知が `results/` に `-task_<id>.json` の命名で書き込まれること。
    #[test]
    fn test_write_stalled_notification_creates_result_file() {
        let dir = test_results_dir();
        let results_dir = dir.join(RESULTS_DIR);

        write_stalled_notification_in(&results_dir, 42, Some(7), "idle", "サブブロックが停止", None)
            .unwrap();

        let entries = fs::read_dir(&results_dir).unwrap();
        let files: Vec<_> = entries.filter_map(|e| e.ok()).collect();
        assert_eq!(files.len(), 1, "通知ファイルが1件生成される");
        let name = files[0].file_name().to_string_lossy().to_string();
        assert!(
            name.ends_with("-task_42.json"),
            "task_id で照合できる命名になる: {}",
            name
        );

        let content = fs::read_to_string(files[0].path()).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&content).unwrap();
        assert_eq!(parsed["type"], "task_stalled");
        assert_eq!(parsed["task_id"], 42);
        assert_eq!(parsed["parent_run_id"], 7);
        assert_eq!(parsed["status"], "stalled");
        assert_eq!(parsed["reason"], "idle");

        let _ = fs::remove_dir_all(&dir);
    }

    /// 質問の通知は question_id を持ち、他の理由では question_id が入らないこと。
    #[test]
    fn test_write_stalled_notification_carries_question_id() {
        let dir = test_results_dir();
        let results_dir = dir.join(RESULTS_DIR);

        // 質問以外には question_id を入れない
        write_stalled_notification_in(&results_dir, 1, Some(7), "idle", "停止", None).unwrap();
        let files: Vec<_> = fs::read_dir(&results_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .collect();
        let content = fs::read_to_string(files[0].path()).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&content).unwrap();
        assert!(
            parsed.get("question_id").is_none(),
            "質問以外の通知には question_id を入れない: {}",
            content
        );

        // 質問は question_id 付きで入る
        let dir2 = test_results_dir();
        let results2 = dir2.join(RESULTS_DIR);
        write_stalled_notification_in(&results2, 2, Some(7), "question", "質問 q1: どれ", Some("q1"))
            .unwrap();
        let file = fs::read_dir(&results2)
            .unwrap()
            .filter_map(|e| e.ok())
            .next()
            .unwrap();
        let parsed: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(file.path()).unwrap()).unwrap();
        assert_eq!(parsed["reason"], "question");
        assert_eq!(parsed["question_id"], "q1");

        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&dir2);
    }

    /// 複数タスクで同時に書いてもそれぞれ別ファイルになること。
    #[test]
    fn test_write_stalled_notification_supports_multiple_tasks() {
        let dir = test_results_dir();
        let results_dir = dir.join(RESULTS_DIR);

        write_stalled_notification_in(&results_dir, 1, None, "blocked", "a", None).unwrap();
        write_stalled_notification_in(&results_dir, 2, None, "idle", "b", None).unwrap();

        let files: Vec<_> = fs::read_dir(&results_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .collect();
        assert_eq!(files.len(), 2);

        let _ = fs::remove_dir_all(&dir);
    }

    /// AC5: 同じ task_id で 2 回書くと stalled のファイルは最新 1 件だけ残る。
    /// completed / failed のファイルは消えない。
    #[test]
    fn test_write_stalled_notification_keeps_latest_only() {
        let dir = test_results_dir();
        let results_dir = dir.join(RESULTS_DIR);

        // 完了通知が先にある状態を作る（削除されてはいけない）
        fs::create_dir_all(&results_dir).unwrap();
        fs::write(
            results_dir.join("1000-task_42.json"),
            serde_json::json!({ "type": "task_completed", "task_id": 42 }).to_string(),
        )
        .unwrap();

        write_stalled_notification_in(&results_dir, 42, Some(7), "idle", "1 回目", None).unwrap();
        std::thread::sleep(Duration::from_millis(5));
        write_stalled_notification_in(&results_dir, 42, Some(7), "long_running", "2 回目", None).unwrap();

        let files: Vec<_> = fs::read_dir(&results_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .collect();
        // completed 1 件 + stalled 1 件 = 2 件
        assert_eq!(files.len(), 2, "古い stalled は消え、completed は残る");

        let mut stalled_reasons = Vec::new();
        let mut completed_count = 0;
        for f in &files {
            let content = fs::read_to_string(f.path()).unwrap();
            let v: serde_json::Value = serde_json::from_str(&content).unwrap();
            match v["type"].as_str().unwrap_or("") {
                "task_stalled" => stalled_reasons.push(v["reason"].as_str().unwrap_or("").to_string()),
                "task_completed" => completed_count += 1,
                other => panic!("想定外の type: {}", other),
            }
        }
        assert_eq!(completed_count, 1, "completed は消えない");
        assert_eq!(
            stalled_reasons,
            vec!["long_running".to_string()],
            "最新の stalled だけ残る"
        );

        let _ = fs::remove_dir_all(&dir);
    }

    /// U1 AC1: `task_manage_request` の処理がエラーになっても、通知が pending/ に
    /// 残らないこと（processed/ へ移す）。
    ///
    /// 以前は `process_task_manage(...)?` の `?` で早期 return していたため、
    /// エラー時に processed/ への移動に到達せず、同じ通知が再処理され続けて
    /// 後続の通知を詰まらせていた。
    #[test]
    fn test_poll_task_manage_error_moves_to_processed() {
        let (mut manager, store) = create_test_manager();

        // 共有の .agent-link/ を触らないよう、通知ディレクトリだけ一時ディレクトリへ差し替える
        // （notify_base は非公開フィールドだが、同じモジュールの test からは代入できる）
        let notify_base = test_results_dir();
        for sub in [PENDING_DIR, PROCESSED_DIR, RESULTS_DIR] {
            let _ = fs::create_dir_all(notify_base.join(sub));
        }
        manager.notify_base = notify_base.clone();

        // in_progress のタスク（send は拒否される）
        let id = create_approved_task(&manager, &store, "batch_mngerr1", "s-1");
        let exec_id = create_dummy_execution(&store, 410);
        store.update_execution_pane(exec_id, "pane-mng-err", 0).unwrap();
        store.update_task_execution_id(id, exec_id).unwrap();
        store.update_task_status(id, "in_progress").unwrap();

        let pending = notify_base.join(PENDING_DIR);
        let processed = notify_base.join(PROCESSED_DIR);
        fs::write(
            pending.join("0001-task_manage.json"),
            serde_json::json!({
                "type": "task_manage_request",
                "parent_run_id": 42,
                "action": "send",
                "task_id": id,
                "message": "指示",
            })
            .to_string(),
        )
        .unwrap();

        // 処理は Err を返す
        let result = manager.poll_one_event();
        assert!(result.is_err(), "in_progress への send はエラーになる");

        // pending/ は空、processed/ に移っている
        let pending_files: Vec<_> = fs::read_dir(&pending).unwrap().flatten().collect();
        assert!(
            pending_files.is_empty(),
            "エラーでも pending/ は空になること"
        );
        let processed_files: Vec<_> = fs::read_dir(&processed).unwrap().flatten().collect();
        assert_eq!(processed_files.len(), 1, "エラーでも processed/ へ移動すること");

        // description は変わっていない
        let task = manager.get_task(id).unwrap();
        assert_eq!(task.description, "説明", "エラー時に説明は変わらないこと");

        let _ = fs::remove_dir_all(&notify_base);
    }

    fn create_test_manager() -> (TaskManager, Arc<SaiStore>) {
        // テスト用の一時ディレクトリ
        let _ = fs::create_dir_all(".agent-link/engine/notifications/pending");
        let _ = fs::create_dir_all(".agent-link/engine/notifications/processed");
        let _ = fs::create_dir_all(".agent-link/engine/notifications/results");

        let store = Arc::new(SaiStore::open_in_memory().unwrap());
        let manager = TaskManager::new(Arc::clone(&store));
        (manager, store)
    }

    /// host 付きの子の pane には、遠隔の cwd を書かない（`cwd.txt` を作らない）。
    /// host なしの子は今まで通り cwd を書く。
    #[test]
    fn pane_placement_does_not_write_remote_cwd() {
        let _lock = crate::common::AGENT_LINK_ENV_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let root = std::env::temp_dir().join(format!("agent-link-tm-pane-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        std::env::set_var("AGENT_LINK_PROJECT_ROOT", &root);

        let (manager, _store) = create_test_manager();
        let mut task = Task::new(
            1, "件名".into(), "説明".into(), None, None, None, "".into(), None, Some("W1".into()),
        );
        task.cwd = Some("~/work/sai-tree".into());
        let place = |m: &TaskManager, t: &Task| match m.resolve_pane_placement(t).unwrap() {
            PlacementResult::Placed(_, pane_id, _) => crate::headless::pane_dir(&pane_id),
            PlacementResult::Waiting => panic!("waiting"),
        };

        let local_dir = place(&manager, &task);
        assert_eq!(fs::read_to_string(local_dir.join("cwd.txt")).unwrap(), "~/work/sai-tree");

        task.host = Some("wsl-server".into());
        let remote_dir = place(&manager, &task);
        assert!(!remote_dir.join("cwd.txt").exists(), "遠隔のパスを cwd.txt に書かない");

        std::env::remove_var("AGENT_LINK_PROJECT_ROOT");
        let _ = fs::remove_dir_all(&root);
    }

    /// 外部キー制約を満たすためのダミー実行レコードを作成するヘルパー。
    fn create_dummy_execution(store: &SaiStore, run_id: i64) -> i64 {
        let block_id = store
            .get_block_by_name("test-block-for-tasks")
            .ok()
            .map(|b| b.id.unwrap())
            .unwrap_or_else(|| {
                let block = crate::db::models::BlockMetadata::new(
                    "test-block-for-tasks".to_string(),
                    "タスクテスト用ブロック".to_string(),
                    "inherit".to_string(),
                    "off".to_string(),
                    "true".to_string(),
                    "true".to_string(),
                    None,
                    300,
                    "none".to_string(),
                    true,
                    "blocks/test-block-for-tasks.md".to_string(),
                    None
                );
                store.insert_block(&block).unwrap()
            });

        // 既存の実行があれば再利用
        let existing = store
            .list_executions_by_block(block_id)
            .ok()
            .unwrap_or_default()
            .into_iter()
            .find(|e| e.id == Some(run_id));
        if let Some(exec) = existing {
            return exec.id.unwrap();
        }

        store.create_execution(block_id, None).unwrap()
    }

    #[test]
    fn test_process_task_created() {
        let (manager, store) = create_test_manager();
        let parent_run_id = create_dummy_execution(&store, 42);

        let notif = TaskCreatedNotification {
            notification_type: "task_created".to_string(),
            parent_run_id,
            batch_id: "batch_test_001".to_string(),
            round: 1,
            supersedes_batch_id: None,
            tasks: vec![TaskItem {
                client_id: "task-1".to_string(),
                subject: "テストタスク1".to_string(),
                description: "テスト説明1".to_string(),
                block_type: Some("sai-code".to_string()),
                inputs: None,
                depends_on: vec![],
                worktree: "none".to_string(),
                cwd: None,
                timeout: None,
                model: None,
                thinking: None,
                host: None,
                carried_over: false,
                card_run_id: None,
                approval: None,
            }],
        };

        let ids = manager.process_task_created(&notif).unwrap();
        assert_eq!(ids.len(), 1);
        assert!(ids[0] > 0);

        // DB から確認
        let task = manager.get_task(ids[0]).unwrap();
        assert_eq!(task.subject, "テストタスク1");
        assert_eq!(task.status, "pending");
        assert_eq!(task.batch_id.as_deref(), Some("batch_test_001"));
        assert_eq!(task.parent_run_id, parent_run_id);
    }

    /// U3 AC5: `TaskItem.timeout` が Task に入り、DB まで届くこと。
    #[test]
    fn test_process_task_created_carries_timeout() {
        let (manager, store) = create_test_manager();
        let parent_run_id = create_dummy_execution(&store, 43);

        let notif = TaskCreatedNotification {
            notification_type: "task_created".to_string(),
            parent_run_id,
            batch_id: "batch_timeout_001".to_string(),
            round: 1,
            supersedes_batch_id: None,
            tasks: vec![TaskItem {
                client_id: "task-timeout".to_string(),
                subject: "時間のかかるタスク".to_string(),
                description: "テスト説明".to_string(),
                block_type: Some("sai-code".to_string()),
                inputs: None,
                depends_on: vec![],
                worktree: "none".to_string(),
                cwd: None,
                timeout: Some(600),
                model: None,
                thinking: None,
                host: None,
                carried_over: false,
                card_run_id: None,
                approval: None,
            }],
        };

        let ids = manager.process_task_created(&notif).unwrap();
        let task = manager.get_task(ids[0]).unwrap();
        assert_eq!(task.timeout, Some(600));
        // DB 経由でも同じ値が返る
        assert_eq!(store.get_task_by_id(ids[0]).unwrap().timeout, Some(600));
    }

    #[test]
    fn test_approve_and_reject_batch() {
        let (manager, store) = create_test_manager();
        let parent_run_id = create_dummy_execution(&store, 1);

        // タスク作成
        let notif = TaskCreatedNotification {
            notification_type: "task_created".to_string(),
            parent_run_id,
            batch_id: "batch_appr_test".to_string(),
            round: 1,
            supersedes_batch_id: None,
            tasks: vec![
                TaskItem {
                    client_id: "a-1".to_string(),
                    subject: "タスクA".to_string(),
                    description: "説明A".to_string(),
                    block_type: None,
                    inputs: None,
                    depends_on: vec![],
                    worktree: "none".to_string(),
                    cwd: None,
                    timeout: None,
                    model: None,
                    thinking: None,
                    host: None,
                    carried_over: false,
                    card_run_id: None,
                    approval: None,
                },
                TaskItem {
                    client_id: "a-2".to_string(),
                    subject: "タスクB".to_string(),
                    description: "説明B".to_string(),
                    block_type: None,
                    inputs: None,
                    depends_on: vec![],
                    worktree: "none".to_string(),
                    cwd: None,
                    timeout: None,
                    model: None,
                    thinking: None,
                    host: None,
                    carried_over: false,
                    card_run_id: None,
                    approval: None,
                },
            ],
        };
        let ids = manager.process_task_created(&notif).unwrap();
        assert_eq!(ids.len(), 2);

        // 承認
        let approved = manager.approve_batch("batch_appr_test").unwrap();
        assert_eq!(approved.len(), 2);

        // 承認後のステータス確認
        for id in &ids {
            let task = manager.get_task(*id).unwrap();
            assert_eq!(task.status, "approved");
        }

        // reject テスト用の別バッチ
        let notif2 = TaskCreatedNotification {
            notification_type: "task_created".to_string(),
            parent_run_id,
            batch_id: "batch_reject_test".to_string(),
            round: 1,
            supersedes_batch_id: None,
            tasks: vec![TaskItem {
                client_id: "r-1".to_string(),
                subject: "拒否タスク".to_string(),
                description: "拒否される".to_string(),
                block_type: None,
                inputs: None,
                depends_on: vec![],
                worktree: "none".to_string(),
                cwd: None,
                timeout: None,
                model: None,
                thinking: None,
                host: None,
                carried_over: false,
                card_run_id: None,
                approval: None,
            }],
        };
        manager.process_task_created(&notif2).unwrap();
        manager.reject_batch("batch_reject_test").unwrap();

        // 拒否後はタスクが存在しない
        let tasks = manager.list_tasks(parent_run_id).unwrap();
        // 承認したタスクのみ残っている
        assert_eq!(tasks.len(), 2);
    }

    #[test]
    fn test_process_task_execute_and_complete() {
        let (manager, store) = create_test_manager();
        let parent_run_id = create_dummy_execution(&store, 10);

        // タスク作成 → 承認
        let notif = TaskCreatedNotification {
            notification_type: "task_created".to_string(),
            parent_run_id,
            batch_id: "batch_exec_test".to_string(),
            round: 1,
            supersedes_batch_id: None,
            tasks: vec![TaskItem {
                client_id: "exec-1".to_string(),
                subject: "実行タスク".to_string(),
                description: "実行説明".to_string(),
                block_type: Some("sai-code-work".to_string()),
                inputs: None,
                depends_on: vec![],
                worktree: "isolation".to_string(),
                cwd: None,
                timeout: None,
                model: None,
                thinking: None,
                host: None,
                carried_over: false,
                card_run_id: None,
                approval: None,
            }],
        };
        let ids = manager.process_task_created(&notif).unwrap();
        manager.approve_batch("batch_exec_test").unwrap();

        // 実行
        let execute_result = manager.process_task_execute(ids[0]).unwrap();
        assert!(matches!(execute_result, ExecuteResult::Started { .. }));

        let task = manager.get_task(ids[0]).unwrap();
        assert_eq!(task.status, "in_progress");

        // 完了
        let task_result = TaskResult {
            output: "エージェントの出力内容".to_string(),
            waggle_tokens: vec!["AbCdEfGh".to_string()],
            commit_id: Some("abc123".to_string()),
            elapsed_ms: 45000,
            session_stats: Some(serde_json::json!({
                "turn_count": 5,
                "tool_call_count": 12,
            })),
            execution_id: Some(100),
        };
        manager.complete_task(ids[0], &task_result).unwrap();

        let task = manager.get_task(ids[0]).unwrap();
        assert_eq!(task.status, "completed");
    }

    #[test]
    fn test_execute_with_invalid_status() {
        let (manager, store) = create_test_manager();
        let parent_run_id = create_dummy_execution(&store, 11);

        let notif = TaskCreatedNotification {
            notification_type: "task_created".to_string(),
            parent_run_id,
            batch_id: "batch_inv".to_string(),
            round: 1,
            supersedes_batch_id: None,
            tasks: vec![TaskItem {
                client_id: "inv-1".to_string(),
                subject: "未承認タスク".to_string(),
                description: "承認前に実行しようとする".to_string(),
                block_type: None,
                inputs: None,
                depends_on: vec![],
                worktree: "none".to_string(),
                cwd: None,
                timeout: None,
                model: None,
                thinking: None,
                host: None,
                carried_over: false,
                card_run_id: None,
                approval: None,
            }],
        };
        let ids = manager.process_task_created(&notif).unwrap();
        // 承認せずに実行 → エラー
        let result = manager.process_task_execute(ids[0]);
        assert!(result.is_err());
        match result.unwrap_err() {
            TaskManagerError::InvalidStatus { .. } => {}
            _ => panic!("InvalidStatus エラーを期待"),
        }
    }

    #[test]
    fn test_list_tasks_by_parent() {
        let (manager, store) = create_test_manager();
        let parent_run_id = create_dummy_execution(&store, 99);

        let notif = TaskCreatedNotification {
            notification_type: "task_created".to_string(),
            parent_run_id,
            batch_id: "batch_list_test".to_string(),
            round: 1,
            supersedes_batch_id: None,
            tasks: vec![
                TaskItem {
                    client_id: "l-1".to_string(),
                    subject: "一覧タスク1".to_string(),
                    description: "説明1".to_string(),
                    block_type: None,
                    inputs: None,
                    depends_on: vec![],
                    worktree: "none".to_string(),
                    cwd: None,
                    timeout: None,
                    model: None,
                    thinking: None,
                    host: None,
                    carried_over: false,
                    card_run_id: None,
                    approval: None,
                },
                TaskItem {
                    client_id: "l-2".to_string(),
                    subject: "一覧タスク2".to_string(),
                    description: "説明2".to_string(),
                    block_type: None,
                    inputs: None,
                    depends_on: vec!["l-1".to_string()],
                    worktree: "none".to_string(),
                    cwd: None,
                    timeout: None,
                    model: None,
                    thinking: None,
                    host: None,
                    carried_over: false,
                    card_run_id: None,
                    approval: None,
                },
            ],
        };
        manager.process_task_created(&notif).unwrap();

        let tasks = manager.list_tasks(parent_run_id).unwrap();
        assert_eq!(tasks.len(), 2);
        assert_eq!(tasks[0].subject, "一覧タスク1");
        assert_eq!(tasks[1].subject, "一覧タスク2");
    }

    #[test]
    fn test_fail_task() {
        let (manager, store) = create_test_manager();
        let parent_run_id = create_dummy_execution(&store, 12);

        let notif = TaskCreatedNotification {
            notification_type: "task_created".to_string(),
            parent_run_id,
            batch_id: "batch_fail_test".to_string(),
            round: 1,
            supersedes_batch_id: None,
            tasks: vec![TaskItem {
                client_id: "f-1".to_string(),
                subject: "失敗タスク".to_string(),
                description: "失敗する".to_string(),
                block_type: None,
                inputs: None,
                depends_on: vec![],
                worktree: "none".to_string(),
                cwd: None,
                timeout: None,
                model: None,
                thinking: None,
                host: None,
                carried_over: false,
                card_run_id: None,
                approval: None,
            }],
        };
        let ids = manager.process_task_created(&notif).unwrap();
        manager.approve_batch("batch_fail_test").unwrap();
        manager.process_task_execute(ids[0]).unwrap();

        manager.fail_task(ids[0], "タイムアウトにより失敗").unwrap();

        let task = manager.get_task(ids[0]).unwrap();
        assert_eq!(task.status, "failed");
    }

    #[test]
    fn test_process_task_output_used_creates_sub_to_main_connection() {
        let (manager, store) = create_test_manager();
        let parent_run_id = create_dummy_execution(&store, 21);
        let parent_block_id = store.get_execution_by_id(parent_run_id).unwrap().block_id;

        // サブタスク作成・承認・実行（サブ実行レコード作成）
        let notif = TaskCreatedNotification {
            notification_type: "task_created".to_string(),
            parent_run_id,
            batch_id: "batch_output_test".to_string(),
            round: 1,
            supersedes_batch_id: None,
            tasks: vec![TaskItem {
                client_id: "o-1".to_string(),
                subject: "出力テスト".to_string(),
                description: "出力を親に返す".to_string(),
                block_type: None,
                inputs: None,
                depends_on: vec![],
                worktree: "none".to_string(),
                cwd: None,
                timeout: None,
                model: None,
                thinking: None,
                host: None,
                carried_over: false,
                card_run_id: None,
                approval: None,
            }],
        };
        let ids = manager.process_task_created(&notif).unwrap();
        manager.approve_batch("batch_output_test").unwrap();
        manager.process_task_execute(ids[0]).unwrap();

        // サブ実行レコードを作成しタスクに紐付け（run_subtask 相当）
        let sub_block_id = store
            .get_block_by_name("test-block-for-tasks")
            .unwrap()
            .id
            .unwrap();
        let sub_run_id = store
            .create_execution_for_subtask(sub_block_id, None, parent_run_id, None)
            .unwrap();
        store.update_task_execution_id(ids[0], sub_run_id).unwrap();

        // task_output_used 通知を処理 → sub→main 接続が作成される
        let notif_output = TaskOutputUsedNotification {
            notification_type: "task_output_used".to_string(),
            parent_run_id,
            task_id: ids[0],
            waggle_tokens: vec!["AbCdEfGh".to_string()],
        };
        let msg = manager.process_task_output_used(&notif_output).unwrap();
        assert!(
            msg.contains("接続作成"),
            "接続作成メッセージを期待: {}",
            msg
        );

        let conns = store.list_connections().unwrap();
        assert_eq!(conns.len(), 1, "task_output 接続が1件作成される");
        let conn = &conns[0];
        assert_eq!(conn.connection_type, "task_output");
        assert_eq!(conn.source_execution_id, sub_run_id, "source はサブ実行");
        assert_eq!(conn.target_execution_id, parent_run_id, "target は親実行");
        assert_eq!(conn.source_block_id, sub_block_id);
        assert_eq!(conn.target_block_id, parent_block_id);

        // 冪等: 同じ通知を再処理しても重複しない
        manager.process_task_output_used(&notif_output).unwrap();
        let conns = store.list_connections().unwrap();
        assert_eq!(conns.len(), 1, "重複接続は作成されない");
    }

    #[test]
    fn test_process_task_output_used_skips_unexecuted_task() {
        let (manager, store) = create_test_manager();
        let parent_run_id = create_dummy_execution(&store, 22);

        let notif = TaskCreatedNotification {
            notification_type: "task_created".to_string(),
            parent_run_id,
            batch_id: "batch_output_skip".to_string(),
            round: 1,
            supersedes_batch_id: None,
            tasks: vec![TaskItem {
                client_id: "o-2".to_string(),
                subject: "未実行".to_string(),
                description: "実行されていない".to_string(),
                block_type: None,
                inputs: None,
                depends_on: vec![],
                worktree: "none".to_string(),
                cwd: None,
                timeout: None,
                model: None,
                thinking: None,
                host: None,
                carried_over: false,
                card_run_id: None,
                approval: None,
            }],
        };
        let ids = manager.process_task_created(&notif).unwrap();

        // 未実行（execution_id 未設定）のタスク → スキップ
        let notif_output = TaskOutputUsedNotification {
            notification_type: "task_output_used".to_string(),
            parent_run_id,
            task_id: ids[0],
            waggle_tokens: vec!["AbCdEfGh".to_string()],
        };
        let msg = manager.process_task_output_used(&notif_output).unwrap();
        assert!(
            msg.contains("スキップ"),
            "スキップメッセージを期待: {}",
            msg
        );
        assert!(store.list_connections().unwrap().is_empty());
    }

    // ---------------------------------------------------------------------
    // Phase 3: 承認反映・実行可否判定・依存解決のテスト
    // ---------------------------------------------------------------------

    /// 承認情報付き task_created: approved で保存・rejected は保存しないこと。
    #[test]
    fn test_process_task_created_with_approval() {
        let (manager, store) = create_test_manager();
        let parent_run_id = create_dummy_execution(&store, 201);

        let notif = TaskCreatedNotification {
            notification_type: "task_created".to_string(),
            parent_run_id,
            batch_id: "batch_appr_p3".to_string(),
            round: 1,
            supersedes_batch_id: None,
            tasks: vec![
                TaskItem {
                    client_id: "ok-1".to_string(),
                    subject: "承認タスク".to_string(),
                    description: "承認された".to_string(),
                    block_type: None,
                    inputs: None,
                    depends_on: vec![],
                    worktree: "none".to_string(),
                    cwd: None,
                    timeout: None,
                    model: None,
                    thinking: None,
                    host: None,
                    carried_over: false,
                    card_run_id: None,
                    approval: Some(TaskApproval {
                        status: "approved".to_string(),
                        comment: Some("OK 進めて".to_string()),
                    }),
                },
                TaskItem {
                    client_id: "ng-1".to_string(),
                    subject: "拒否タスク".to_string(),
                    description: "拒否された".to_string(),
                    block_type: None,
                    inputs: None,
                    depends_on: vec![],
                    worktree: "none".to_string(),
                    cwd: None,
                    timeout: None,
                    model: None,
                    thinking: None,
                    host: None,
                    carried_over: false,
                    card_run_id: None,
                    approval: Some(TaskApproval {
                        status: "rejected".to_string(),
                        comment: Some("分割して".to_string()),
                    }),
                },
            ],
        };

        let ids = manager.process_task_created(&notif).unwrap();
        assert_eq!(ids.len(), 1, "承認タスクのみ保存される");

        let task = manager.get_task(ids[0]).unwrap();
        assert_eq!(task.status, "approved", "承認済みタスクは approved で保存");
        assert_eq!(task.approval_comment.as_deref(), Some("OK 進めて"));
        assert_eq!(task.client_id.as_deref(), Some("ok-1"));
    }

    /// supersedes_batch_id で旧バッチの未完了タスクが deleted になること。
    #[test]
    fn test_process_task_created_supersedes() {
        let (manager, store) = create_test_manager();
        let parent_run_id = create_dummy_execution(&store, 202);

        // 旧バッチ（approval なし → pending 保存）
        let old_notif = TaskCreatedNotification {
            notification_type: "task_created".to_string(),
            parent_run_id,
            batch_id: "batch_v1".to_string(),
            round: 1,
            supersedes_batch_id: None,
            tasks: vec![TaskItem {
                client_id: "a-1".to_string(),
                subject: "旧タスク".to_string(),
                description: "旧".to_string(),
                block_type: None,
                inputs: None,
                depends_on: vec![],
                worktree: "none".to_string(),
                cwd: None,
                timeout: None,
                model: None,
                thinking: None,
                host: None,
                carried_over: false,
                card_run_id: None,
                approval: None,
            }],
        };
        let old_ids = manager.process_task_created(&old_notif).unwrap();

        // 新バッチ（supersedes = batch_v1、承認済み）
        let new_notif = TaskCreatedNotification {
            notification_type: "task_created".to_string(),
            parent_run_id,
            batch_id: "batch_v2".to_string(),
            round: 2,
            supersedes_batch_id: Some("batch_v1".to_string()),
            tasks: vec![TaskItem {
                client_id: "a-1".to_string(),
                subject: "新タスク".to_string(),
                description: "新".to_string(),
                block_type: None,
                inputs: None,
                depends_on: vec![],
                worktree: "none".to_string(),
                cwd: None,
                timeout: None,
                model: None,
                thinking: None,
                host: None,
                carried_over: true,
                card_run_id: None,
                approval: Some(TaskApproval {
                    status: "approved".to_string(),
                    comment: None,
                }),
            }],
        };
        let new_ids = manager.process_task_created(&new_notif).unwrap();

        // 旧タスクは deleted 化
        let old_task = manager.get_task(old_ids[0]).unwrap();
        assert_eq!(old_task.status, "deleted");
        assert_eq!(old_task.deleted_by.as_deref(), Some("main"));

        // 新タスクは approved・round=2・supersedes 記録
        let new_task = manager.get_task(new_ids[0]).unwrap();
        assert_eq!(new_task.status, "approved");
        assert_eq!(new_task.round, 2);
        assert_eq!(new_task.supersedes_batch_id.as_deref(), Some("batch_v1"));
    }

    /// 依存未完了タスクは waiting_dependency、依存完了で in_progress になること。
    #[test]
    fn test_try_promote_dependency() {
        let (manager, store) = create_test_manager();
        let parent_run_id = create_dummy_execution(&store, 203);

        // 承認付きで 2 タスク作成（b-2 は b-1 に依存）
        let notif = TaskCreatedNotification {
            notification_type: "task_created".to_string(),
            parent_run_id,
            batch_id: "batch_dep".to_string(),
            round: 1,
            supersedes_batch_id: None,
            tasks: vec![
                TaskItem {
                    client_id: "b-1".to_string(),
                    subject: "依存元".to_string(),
                    description: "".to_string(),
                    block_type: None,
                    inputs: None,
                    depends_on: vec![],
                    worktree: "none".to_string(),
                    cwd: None,
                    timeout: None,
                    model: None,
                    thinking: None,
                    host: None,
                    carried_over: false,
                    card_run_id: None,
                    approval: Some(TaskApproval {
                        status: "approved".to_string(),
                        comment: None,
                    }),
                },
                TaskItem {
                    client_id: "b-2".to_string(),
                    subject: "依存先".to_string(),
                    description: "".to_string(),
                    block_type: None,
                    inputs: None,
                    depends_on: vec!["b-1".to_string()],
                    worktree: "none".to_string(),
                    cwd: None,
                    timeout: None,
                    model: None,
                    thinking: None,
                    host: None,
                    carried_over: false,
                    card_run_id: None,
                    approval: Some(TaskApproval {
                        status: "approved".to_string(),
                        comment: None,
                    }),
                },
            ],
        };
        let ids = manager.process_task_created(&notif).unwrap();
        let dep_id = ids[0];
        let waiting_id = ids[1];

        // 依存未完了 → waiting_dependency
        let r = manager.try_promote(waiting_id).unwrap();
        assert!(matches!(r, PromoteResult::WaitingDependency { .. }));
        assert_eq!(
            manager.get_task(waiting_id).unwrap().status,
            "waiting_dependency"
        );

        // 依存元を完了（仮完了: completed）→ この時点では昇格しない（finalized 待ち）
        let dep_done = TaskResult {
            output: "done".to_string(),
            waggle_tokens: vec![],
            commit_id: None,
            elapsed_ms: 1,
            session_stats: None,
            execution_id: None,
        };
        let next = manager.complete_task(dep_id, &dep_done).unwrap();
        assert!(
            !next.contains(&waiting_id),
            "completed（仮完了）では依存先は昇格しない（finalized 待ち）: {:?}",
            next
        );
        assert_eq!(
            manager.get_task(waiting_id).unwrap().status,
            "waiting_dependency",
            "依存元が completed でも依存先は waiting_dependency のまま"
        );

        // 依存元を finalize（完了確定）→ 依存先が昇格（Started）
        let event = manager
            .process_task_manage("finalize", dep_id, None)
            .unwrap();
        assert!(
            event.started_task_ids.contains(&waiting_id),
            "finalized で依存先が昇格する: {:?}",
            event.started_task_ids
        );
        assert_eq!(manager.get_task(waiting_id).unwrap().status, "in_progress");
        assert_eq!(manager.get_task(waiting_id).unwrap().status, "in_progress");
    }

    /// 実行上限（AgentPool max_concurrent=4）超過時は queued になること。
    #[test]
    fn test_try_promote_queue_full_queues() {
        let (manager, store) = create_test_manager();
        let parent_run_id = create_dummy_execution(&store, 204);

        // 5 タスクを承認済みで作成
        let tasks: Vec<TaskItem> = (0..5)
            .map(|i| TaskItem {
                client_id: format!("q-{}", i),
                subject: format!("タスク{}", i),
                description: "".to_string(),
                block_type: None,
                inputs: None,
                depends_on: vec![],
                worktree: "none".to_string(),
                cwd: None,
                timeout: None,
                model: None,
                thinking: None,
                host: None,
                carried_over: false,
                card_run_id: None,
                approval: Some(TaskApproval {
                    status: "approved".to_string(),
                    comment: None,
                }),
            })
            .collect();
        let notif = TaskCreatedNotification {
            notification_type: "task_created".to_string(),
            parent_run_id,
            batch_id: "batch_q".to_string(),
            round: 1,
            supersedes_batch_id: None,
            tasks,
        };
        let ids = manager.process_task_created(&notif).unwrap();
        assert_eq!(ids.len(), 5);

        // 4 個は Started、5 個目は Queued
        let mut started = 0;
        let mut queued = 0;
        for id in &ids {
            match manager.try_promote(*id).unwrap() {
                PromoteResult::Started { .. } => started += 1,
                PromoteResult::Queued { .. } => queued += 1,
                other => panic!("想定外: {:?}", other),
            }
        }
        assert_eq!(started, 4);
        assert_eq!(queued, 1);
        assert_eq!(manager.get_task(ids[4]).unwrap().status, "queued");
    }

    /// バッチ単位の task_execute が承認済みタスクのみ投入すること。
    #[test]
    fn test_process_batch_execute() {
        let store = Arc::new(SaiStore::open_in_memory().unwrap());
        let manager = TaskManager::with_agent_pool(Arc::clone(&store), Arc::new(AgentPool::new()));
        let parent_run_id = create_dummy_execution(&store, 205);

        let notif = TaskCreatedNotification {
            notification_type: "task_created".to_string(),
            parent_run_id,
            batch_id: "batch_be".to_string(),
            round: 1,
            supersedes_batch_id: None,
            tasks: vec![
                TaskItem {
                    client_id: "be-1".to_string(),
                    subject: "承認".to_string(),
                    description: "".to_string(),
                    block_type: None,
                    inputs: None,
                    depends_on: vec![],
                    worktree: "none".to_string(),
                    cwd: None,
                    timeout: None,
                    model: None,
                    thinking: None,
                    host: None,
                    carried_over: false,
                    card_run_id: None,
                    approval: Some(TaskApproval {
                        status: "approved".to_string(),
                        comment: None,
                    }),
                },
                TaskItem {
                    client_id: "be-2".to_string(),
                    subject: "未承認".to_string(),
                    description: "".to_string(),
                    block_type: None,
                    inputs: None,
                    depends_on: vec![],
                    worktree: "none".to_string(),
                    cwd: None,
                    timeout: None,
                    model: None,
                    thinking: None,
                    host: None,
                    carried_over: false,
                    card_run_id: None,
                    approval: None, // 旧形式 → pending
                },
            ],
        };
        let ids = manager.process_task_created(&notif).unwrap();
        assert_eq!(ids.len(), 2);

        // バッチ実行: 承認済み（be-1）のみ Started
        let results = manager.process_batch_execute("batch_be").unwrap();
        assert_eq!(results.len(), 1);
        assert!(matches!(results[0], ExecuteResult::Started { .. }));

        // be-2 は pending のまま
        assert_eq!(manager.get_task(ids[1]).unwrap().status, "pending");
    }

    // ---------------------------------------------------------------------
    // Phase 4: task_manage（delete / finalize / send / requeue）のテスト
    // ---------------------------------------------------------------------

    /// 承認済みタスクを1件作るヘルパー。
    fn create_approved_task(
        manager: &TaskManager,
        store: &SaiStore,
        batch_id: &str,
        client_id: &str,
    ) -> i64 {
        let parent_run_id = create_dummy_execution(store, 300);
        let notif = TaskCreatedNotification {
            notification_type: "task_created".to_string(),
            parent_run_id,
            batch_id: batch_id.to_string(),
            round: 1,
            supersedes_batch_id: None,
            tasks: vec![TaskItem {
                client_id: client_id.to_string(),
                subject: "タスク".to_string(),
                description: "説明".to_string(),
                block_type: None,
                inputs: None,
                depends_on: vec![],
                worktree: "none".to_string(),
                cwd: None,
                timeout: None,
                model: None,
                thinking: None,
                host: None,
                carried_over: false,
                card_run_id: None,
                approval: Some(TaskApproval {
                    status: "approved".to_string(),
                    comment: None,
                }),
            }],
        };
        let ids = manager.process_task_created(&notif).unwrap();
        ids[0]
    }

    #[test]
    fn test_task_manage_delete() {
        let (manager, store) = create_test_manager();
        let id = create_approved_task(&manager, &store, "batch_del", "d-1");

        let event = manager.process_task_manage("delete", id, None).unwrap();
        assert!(event.started_task_ids.is_empty());

        let task = manager.get_task(id).unwrap();
        assert_eq!(task.status, "deleted");
        assert_eq!(task.deleted_by.as_deref(), Some("main"));
        assert!(task.deleted_at.is_some());
    }

    #[test]
    fn test_task_manage_finalize() {
        let (manager, store) = create_test_manager();
        let id = create_approved_task(&manager, &store, "batch_fin", "f-1");

        let event = manager.process_task_manage("finalize", id, None).unwrap();
        assert!(event.started_task_ids.is_empty());

        let task = manager.get_task(id).unwrap();
        assert_eq!(task.status, "finalized");
        assert!(task.finalized);
    }

    #[test]
    fn test_task_manage_send_appends_instruction() {
        let (manager, store) = create_test_manager();
        let id = create_approved_task(&manager, &store, "batch_snd", "s-1");

        let event = manager
            .process_task_manage("send", id, Some("アウトプットが不足。追記して"))
            .unwrap();

        // 新ラウンドはワーカーが実行（started_task_ids に含まれる）
        assert!(event.started_task_ids.contains(&id));

        // description に追加指示が追記されている
        let task = manager.get_task(id).unwrap();
        assert!(task.description.contains("追加入力指示"));
        assert!(task.description.contains("アウトプットが不足。追記して"));
        assert!(task.description.starts_with("説明"));
    }

    #[test]
    fn test_task_manage_requeue_failed() {
        let (manager, store) = create_test_manager();
        let id = create_approved_task(&manager, &store, "batch_rq", "r-1");

        // 失敗させる
        manager.fail_task(id, "失敗").unwrap();
        assert_eq!(manager.get_task(id).unwrap().status, "failed");

        // requeue → approved → Started（in_progress）
        let event = manager.process_task_manage("requeue", id, None).unwrap();
        assert!(event.started_task_ids.contains(&id));
        assert_eq!(manager.get_task(id).unwrap().status, "in_progress");
    }

    /// 実行中削除後、後続の complete_task が deleted を上書きしないこと（競合防止）。
    #[test]
    fn test_complete_task_does_not_override_deleted() {
        let (manager, store) = create_test_manager();
        let id = create_approved_task(&manager, &store, "batch_race", "race-1");

        // 実行開始 → 削除（実行中削除のシナリオ）
        manager.try_promote(id).unwrap();
        assert_eq!(manager.get_task(id).unwrap().status, "in_progress");
        manager.process_task_manage("delete", id, None).unwrap();
        assert_eq!(manager.get_task(id).unwrap().status, "deleted");

        // 後から実行スレッドが完了処理を行っても deleted のまま
        let result = TaskResult {
            output: "遅延完了".to_string(),
            waggle_tokens: vec![],
            commit_id: None,
            elapsed_ms: 1,
            session_stats: None,
            execution_id: None,
        };
        manager.complete_task(id, &result).unwrap();
        assert_eq!(manager.get_task(id).unwrap().status, "deleted");
    }

    /// 不明なアクションはエラーになること。
    #[test]
    fn test_task_manage_unknown_action() {
        let (manager, store) = create_test_manager();
        let id = create_approved_task(&manager, &store, "batch_unk", "u-1");
        let err = manager.process_task_manage("hoge", id, None).unwrap_err();
        assert!(matches!(err, TaskManagerError::NotFound(_)));
    }

    /// extract_latest_turn_message: 最後の [追加入力指示] 以降を抽出する。
    #[test]
    fn test_extract_latest_turn_message() {
        // 追加入力指示なし → None
        assert!(TaskManager::extract_latest_turn_message("タスク説明のみ").is_none());

        // 1回の追加入力指示 → そのメッセージを返す
        let desc1 = "タスク説明\n\n[追加入力指示]\nfeature_a.md の競合を解消せよ";
        assert_eq!(
            TaskManager::extract_latest_turn_message(desc1).as_deref(),
            Some("feature_a.md の競合を解消せよ")
        );

        // 複数回の追加入力指示 → 最後の指示を返す
        let desc2 =
            format!("タスク説明\n\n[追加入力指示]\n1回目の指示\n\n[追加入力指示]\n2回目の指示");
        assert_eq!(
            TaskManager::extract_latest_turn_message(&desc2).as_deref(),
            Some("2回目の指示")
        );

        // 空メッセージ → None
        assert!(TaskManager::extract_latest_turn_message("説明\n\n[追加入力指示]\n   ").is_none());
    }

    /// task_manage(send): pane がある場合はターン送信、ない場合は新規実行（後方互換）。
    #[test]
    fn test_task_manage_send_turn_mode() {
        let (manager, store) = create_test_manager();

        // ── ケース1: 実行レコード + pane がある → ターン送信（turn_task_ids）──
        let id = create_approved_task(&manager, &store, "batch_send1", "s-1");
        let exec_id = create_dummy_execution(&store, 400);
        store
            .update_execution_pane(exec_id, "pane-test-1", 0)
            .unwrap();
        store.update_task_execution_id(id, exec_id).unwrap();
        store.update_task_status(id, "completed").unwrap();

        let event = manager
            .process_task_manage("send", id, Some("競合を解消せよ"))
            .unwrap();
        assert!(event.started_task_ids.is_empty(), "新規実行はしない");
        assert_eq!(event.turn_task_ids, vec![id], "同一 pane へのターン送信");

        // description に [追加入力指示] が追記されている
        let task = manager.get_task(id).unwrap();
        assert!(task.description.contains("[追加入力指示]"));
        assert!(task.description.contains("競合を解消せよ"));

        // ── ケース2: pane が無い → 従来どおり新規実行（started_task_ids）──
        let id2 = create_approved_task(&manager, &store, "batch_send2", "s-2");
        let event2 = manager
            .process_task_manage("send", id2, Some("指示"))
            .unwrap();
        assert_eq!(event2.started_task_ids, vec![id2]);
        assert!(event2.turn_task_ids.is_empty());

        // ── ケース3: 実行中（in_progress）+ pane あり → エラー ──
        let id3 = create_approved_task(&manager, &store, "batch_send3", "s-3");
        let exec3 = create_dummy_execution(&store, 401);
        store
            .update_execution_pane(exec3, "pane-test-3", 0)
            .unwrap();
        store.update_task_execution_id(id3, exec3).unwrap();
        store.update_task_status(id3, "in_progress").unwrap();
        let err = manager
            .process_task_manage("send", id3, Some("指示"))
            .unwrap_err();
        assert!(matches!(
            err,
            TaskManagerError::InvalidStatus {
                task_id: _,
                current,
                expected
            } if current == "in_progress" && expected == "completed"
        ));
        // U1 バグ2修正: エラーになった send は description を書き換えない。
        let task3_after = manager.get_task(id3).unwrap();
        assert_eq!(
            task3_after.description, "説明",
            "in_progress への send はエラーで、説明は変わらないこと"
        );

        // ── ケース4: 実行中（in_progress）+ pane 未記録 → エラー（新規実行しない）──
        // 実行中は実行レコードが未紐付けのことがあり、以前は「pane なし」と判定されて
        // 同じ作業の子が二重に起動していた。
        let id4 = create_approved_task(&manager, &store, "batch_send4", "s-4");
        store.update_task_status(id4, "in_progress").unwrap();
        let err4 = manager
            .process_task_manage("send", id4, Some("指示"))
            .unwrap_err();
        assert!(
            matches!(err4, TaskManagerError::InvalidStatus { .. }),
            "実行中は新規実行しない: {:?}",
            err4
        );

        // ── ケース5: 実行済みだが pane が無い → エラー（再実行は requeue のみ）──
        let id5 = create_approved_task(&manager, &store, "batch_send5", "s-5");
        let exec5 = create_dummy_execution(&store, 403);
        store.update_task_execution_id(id5, exec5).unwrap();
        store.update_task_status(id5, "completed").unwrap();
        let err5 = manager
            .process_task_manage("send", id5, Some("指示"))
            .unwrap_err();
        assert!(
            matches!(
                err5,
                TaskManagerError::PaneMissingForTurn { task_id } if task_id == id5
            ),
            "実行済みで pane が無いときは新規実行しない: {:?}",
            err5
        );
    }

    // ---------------------------------------------------------------------
    // U2・R4/R5: バッチ実行の pane 事前カウント・タブ作成・上限制御
    // ---------------------------------------------------------------------

    /// max_concurrent を確実に超えるタスク数を返す（オーバーリミットテスト用）。
    fn super_max_concurrent() -> usize {
        crate::settings::max_concurrent_limit() + 1
    }

    /// 承認タスクを n 件バッチに作成するヘルパー。
    fn create_approved_batch_tasks(
        manager: &TaskManager,
        store: &SaiStore,
        batch_id: &str,
        n: usize,
    ) -> Vec<i64> {
        let parent_run_id = create_dummy_execution(store, 5100);
        let tasks: Vec<TaskItem> = (0..n)
            .map(|i| TaskItem {
                client_id: format!("u2-{}", i),
                subject: format!("U2タスク{}", i),
                description: "".to_string(),
                block_type: None,
                inputs: None,
                depends_on: vec![],
                worktree: "none".to_string(),
                cwd: None,
                timeout: None,
                model: None,
                thinking: None,
                host: None,
                carried_over: false,
                card_run_id: None,
                approval: Some(TaskApproval {
                    status: "approved".to_string(),
                    comment: None,
                }),
            })
            .collect();
        let notif = TaskCreatedNotification {
            notification_type: "task_created".to_string(),
            parent_run_id,
            batch_id: batch_id.to_string(),
            round: 1,
            supersedes_batch_id: None,
            tasks,
        };
        manager.process_task_created(&notif).unwrap()
    }

    /// n==0（承認タスクなし）のバッチ実行が pane を作成せず早期 return すること（R4）。
    #[test]
    fn test_process_batch_execute_no_approved() {
        let (manager, store) = create_test_manager();
        let parent_run_id = create_dummy_execution(&store, 5001);
        // 承認なし（旧形式・pending 保存）のタスクを 1 件だけ作る
        let notif = TaskCreatedNotification {
            notification_type: "task_created".to_string(),
            parent_run_id,
            batch_id: "batch_u2_none".to_string(),
            round: 1,
            supersedes_batch_id: None,
            tasks: vec![TaskItem {
                client_id: "n-1".to_string(),
                subject: "未承認".to_string(),
                description: "".to_string(),
                block_type: None,
                inputs: None,
                depends_on: vec![],
                worktree: "none".to_string(),
                cwd: None,
                timeout: None,
                model: None,
                thinking: None,
                host: None,
                carried_over: false,
                card_run_id: None,
                approval: None,
            }],
        };
        manager.process_task_created(&notif).unwrap();

        let results = manager.process_batch_execute("batch_u2_none").unwrap();
        assert!(results.is_empty(), "承認タスクが無ければ空リストを返すべき");
        assert!(
            manager.block_tabs.lock().unwrap().is_empty(),
            "タブ作成もタブ登録も行われないべき"
        );
    }

    /// n が max_concurrent を超えるバッチ実行が即時失敗（BatchPaneLimitExceeded）し、
    /// タスクを promote せず pane も作成しないこと（R5・U2/R4 案X）。
    #[test]
    fn test_process_batch_execute_over_limit() {
        let (manager, store) = create_test_manager();
        let t = super_max_concurrent();
        let ids = create_approved_batch_tasks(&manager, &store, "batch_u2_over", t);
        assert_eq!(ids.len(), t);

        let err = manager.process_batch_execute("batch_u2_over").unwrap_err();
        match &err {
            TaskManagerError::BatchPaneLimitExceeded { batch_id, n, limit } => {
                assert_eq!(batch_id, "batch_u2_over");
                assert_eq!(*n, t);
                assert_eq!(
                    *limit,
                    std::cmp::max(crate::settings::max_concurrent_limit(), 8)
                );
            }
            other => panic!("BatchPaneLimitExceeded を期待: {:?}", other),
        }
        assert!(
            format!("{}", err).contains("required_finalize"),
            "エラーメッセージに finalize 要求のヒントが含まれるべき"
        );

        // タスクは approved のまま（promote されず pane 未作成）
        for id in &ids {
            assert_eq!(manager.get_task(*id).unwrap().status, "approved");
        }
        assert!(manager.block_tabs.lock().unwrap().is_empty());
    }

    /// `process_batch_execute` はタブを事前作成しないこと（U2・R4 案X）。
    /// pane 分割はワーカーの `execute_subtask` が実行時・動的に行うため、
    /// バッチ処理時点ではタブを作らない。
    #[test]
    fn test_process_batch_execute_does_not_create_tab() {
        let (manager, store) = create_test_manager();
        let ids = create_approved_batch_tasks(&manager, &store, "batch_u2_dyn", 3);
        assert_eq!(ids.len(), 3);

        let results = manager.process_batch_execute("batch_u2_dyn").unwrap();
        assert_eq!(results.len(), 3);
        for r in &results {
            assert!(
                matches!(r, ExecuteResult::Started { .. }),
                "承認済み 3 件はすべて即時開始されるべき: {:?}",
                r
            );
        }
        // タブは事前作成されない（動的 pane 配置は execute_subtask で行う）
        assert!(
            manager.block_tabs.lock().unwrap().is_empty(),
            "process_batch_execute 時点ではタブを登録しない（案X）"
        );
    }

    /// `next_split_target` 純関数がタイルレイアウトの配置ルールを満たすことを検証する（案X）。
    ///
    /// - 実 pane 0 個 → ルートを Right（最初の列）
    /// - 列数 1..3 → 右端列の最上 pane を Right（新しい列を右に追加）
    /// - 列数 4   → 最小 pane 数の列（タイ左端）の最上 pane を Down
    /// - 上限 8   → None
    #[test]
    fn test_next_split_target_layout_rules() {
        use crate::pane::SplitDirection as D;
        let root = "root".to_string();

        // 0 pane: ルートを Right（最初の列）
        assert_eq!(
            next_split_target(&[], &root),
            Some((root.clone(), D::Right))
        );

        // 1 pane（1列）: 右端列の最上 pane を Right
        let p1 = vec![("p1".to_string(), 0, 0)];
        assert_eq!(
            next_split_target(&p1, &root),
            Some(("p1".to_string(), D::Right))
        );

        // 2 pane（例: [p1 | p2] 2列）: 右端列 p2 を Right
        let p2 = vec![("p1".to_string(), 0, 0), ("p2".to_string(), 100, 0)];
        assert_eq!(
            next_split_target(&p2, &root),
            Some(("p2".to_string(), D::Right))
        );

        // 3 pane（3列）: 右端列 p3 を Right → 4列目を追加
        let p3 = vec![
            ("p1".to_string(), 0, 0),
            ("p2".to_string(), 100, 0),
            ("p3".to_string(), 200, 0),
        ];
        assert_eq!(
            next_split_target(&p3, &root),
            Some(("p3".to_string(), D::Right))
        );

        // 4 pane（4列各1）: 最小列（タイ左端 p1）を Down
        let p4 = vec![
            ("p1".to_string(), 0, 0),
            ("p2".to_string(), 100, 0),
            ("p3".to_string(), 200, 0),
            ("p4".to_string(), 300, 0),
        ];
        assert_eq!(
            next_split_target(&p4, &root),
            Some(("p1".to_string(), D::Down))
        );

        // 5 pane（列1が2個 p1/p1a）: 最小列=残り1個の列（タイ左端 p2）を Down
        let p5 = vec![
            ("p1".to_string(), 0, 0),
            ("p1a".to_string(), 0, 50),
            ("p2".to_string(), 100, 0),
            ("p3".to_string(), 200, 0),
            ("p4".to_string(), 300, 0),
        ];
        assert_eq!(
            next_split_target(&p5, &root),
            Some(("p2".to_string(), D::Down))
        );

        // 6 pane（列1,2が2個）: 最小列=残り1個の列（タイ左端 p3）を Down
        let p6 = vec![
            ("p1".to_string(), 0, 0),
            ("p1a".to_string(), 0, 50),
            ("p2".to_string(), 100, 0),
            ("p2a".to_string(), 100, 50),
            ("p3".to_string(), 200, 0),
            ("p4".to_string(), 300, 0),
        ];
        assert_eq!(
            next_split_target(&p6, &root),
            Some(("p3".to_string(), D::Down))
        );

        // 上限 8（4列×2）: None（分割不可）
        let p8 = vec![
            ("p1".to_string(), 0, 0),
            ("p1a".to_string(), 0, 50),
            ("p2".to_string(), 100, 0),
            ("p2a".to_string(), 100, 50),
            ("p3".to_string(), 200, 0),
            ("p3a".to_string(), 200, 50),
            ("p4".to_string(), 300, 0),
            ("p4a".to_string(), 300, 50),
        ];
        assert_eq!(next_split_target(&p8, &root), None);
    }

    /// R24（U12）: バッチ単位の縦配置（`resolve_batch_split`）を検証する。
    ///
    /// 期待する配置（AE5）:
    /// ```text
    /// mb1 - sb1 - sb2   （バッチ1: root の右に横並び）
    /// mb1.1 - sb3       （バッチ2: mb1 の下に縦展開 → 右に横並び）
    /// ```
    #[test]
    fn test_resolve_batch_split_vertical_layout() {
        use crate::pane::SplitDirection as D;
        let mut g = TabGroup {
            tab_id: "t1".to_string(),
            root_pane_id: "root".to_string(),
            root_used: false,
            batch_anchors: std::collections::HashMap::new(),
            batch_last: std::collections::HashMap::new(),
            batch_order: Vec::new(),
        };
        // タブ内に今ある実 pane（テストの流れに合わせて増やす）
        let mut effective: Vec<(String, i64, i64)> = Vec::new();

        // バッチ1 最初のタスク: root を Right 分割（mb1 位置）
        let (base, dir, new) =
            resolve_batch_split(&g, 1, "root", &effective, true).expect("配置できるはず");
        assert_eq!((base.as_str(), dir), ("root", D::Right));
        assert!(new, "最初のバッチは新規バッチ");
        // 状態更新（resolve_pane_placement と同じ処理を再現）
        g.batch_order.push(1);
        g.batch_anchors.insert(1, "mb1".to_string());
        g.batch_last.insert(1, "mb1".to_string());
        effective.push(("mb1".to_string(), 0, 0));

        // バッチ1 の 2 つ目: mb1 の右（sb1）
        let (base, dir, new) =
            resolve_batch_split(&g, 1, "root", &effective, true).expect("配置できるはず");
        assert_eq!((base.as_str(), dir), ("mb1", D::Right));
        assert!(!new);
        g.batch_last.insert(1, "sb1".to_string());
        effective.push(("sb1".to_string(), 100, 0));

        // バッチ1 の 3 つ目: sb1 の右（sb2）
        let (base, dir, new) =
            resolve_batch_split(&g, 1, "root", &effective, true).expect("配置できるはず");
        assert_eq!((base.as_str(), dir), ("sb1", D::Right));
        assert!(!new);
        g.batch_last.insert(1, "sb2".to_string());
        effective.push(("sb2".to_string(), 200, 0));

        // バッチ2 最初: 直前アンカー mb1 の下（mb1.1）
        let (base, dir, new) =
            resolve_batch_split(&g, 2, "root", &effective, true).expect("配置できるはず");
        assert_eq!(
            (base.as_str(), dir),
            ("mb1", D::Down),
            "新規バッチは直前バッチのアンカーを Down 分割して縦展開"
        );
        assert!(new, "バッチ2 は新規バッチ");
        g.batch_order.push(2);
        g.batch_anchors.insert(2, "mb1.1".to_string());
        g.batch_last.insert(2, "mb1.1".to_string());
        effective.push(("mb1.1".to_string(), 0, 100));

        // バッチ2 の 2 つ目: mb1.1 の右（sb3）
        let (base, dir, new) =
            resolve_batch_split(&g, 2, "root", &effective, true).expect("配置できるはず");
        assert_eq!((base.as_str(), dir), ("mb1.1", D::Right));
        assert!(!new);
    }

    /// U04: 直前バッチのアンカー（`batch_anchors`）が閉じられていたら、
    /// 今ある pane から決める `next_split_target` にフォールバックする。
    #[test]
    fn test_resolve_batch_split_falls_back_when_anchor_closed() {
        use crate::pane::SplitDirection as D;
        let mut g = TabGroup {
            tab_id: "t1".to_string(),
            root_pane_id: "root".to_string(),
            root_used: true,
            batch_anchors: std::collections::HashMap::new(),
            batch_last: std::collections::HashMap::new(),
            batch_order: Vec::new(),
        };
        g.batch_order.push(1);
        g.batch_anchors.insert(1, "mb1".to_string());
        g.batch_last.insert(1, "sb1".to_string());

        // mb1 は閉じられ、sb1 だけが今ある
        let effective = vec![("sb1".to_string(), 0, 0)];
        let (base, dir, new) = resolve_batch_split(&g, 2, "root", &effective, true)
            .expect("今ある pane から配置できるはず");
        assert_eq!(base, "sb1", "閉じた mb1 ではなく今ある pane を使う");
        assert_eq!(dir, D::Right);
        assert!(new, "新しいバッチの判定は元のまま");
    }

    /// U04: 同一バッチの `batch_last` が閉じられていたら、
    /// 今ある pane から決める `next_split_target` にフォールバックする。
    #[test]
    fn test_resolve_batch_split_falls_back_when_last_closed() {
        use crate::pane::SplitDirection as D;
        let mut g = TabGroup {
            tab_id: "t1".to_string(),
            root_pane_id: "root".to_string(),
            root_used: true,
            batch_anchors: std::collections::HashMap::new(),
            batch_last: std::collections::HashMap::new(),
            batch_order: Vec::new(),
        };
        g.batch_order.push(1);
        g.batch_anchors.insert(1, "mb1".to_string());
        g.batch_last.insert(1, "mb1".to_string());

        // mb1 は閉じられ、sb1・sb2 が今ある
        let effective = vec![("sb1".to_string(), 0, 0), ("sb2".to_string(), 100, 0)];
        let (base, dir, new) = resolve_batch_split(&g, 1, "root", &effective, true)
            .expect("今ある pane から配置できるはず");
        assert_eq!(base, "sb2", "閉じた mb1 ではなく今ある pane を使う");
        assert_eq!(dir, D::Right);
        assert!(!new, "同一バッチの判定は元のまま");
    }
}
