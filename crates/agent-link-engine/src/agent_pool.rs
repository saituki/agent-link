//! # エージェント実行プール — 同時実行制御
//!
//! 同時実行数の上限を管理し、FIFO キューでサブタスク実行を直列化します。
//!
//! ## 状態遷移
//!
//! ```text
//!                     ┌──────────┐
//!                     │  QUEUED  │ ← AgentPool.enqueue()
//!                     └────┬─────┘
//!                          │ 空きスロット発生
//!                          ▼
//!                     ┌──────────┐
//!                     │  ACTIVE  │ ← orchestrator::run_subtask() 開始
//!                     └────┬─────┘
//!                          │ cc 完了
//!                     ┌────┴─────┐
//!                     ▼          ▼
//!               ┌──────────┐ ┌──────────┐
//!               │COMPLETED │ │  FAILED  │
//!               └──────────┘ └──────────┘
//!                     │          │
//!                     └────┬─────┘
//!                          │ AgentPool.complete()
//!                          ▼
//!                     次のQUEUEDタスクをACTIVEに
//! ```
//!
//! 答え待ち（質問の答えを待つ子）は `suspend` で ACTIVE から WAITING へ移る。
//! WAITING は枠を持たないので、その分の QUEUED を起動できる。答えが来たら
//! `resume` で ACTIVE に戻す（上限は見ない）。

use std::collections::{HashSet, VecDeque};
use std::sync::{Condvar, Mutex};

// ============================================================================
// AgentPoolConfig
// ============================================================================

/// エージェントプールの設定。
#[derive(Debug, Clone)]
pub struct AgentPoolConfig {
    /// 最大同時実行数（デフォルト: 4）
    pub max_concurrent: usize,
    /// キュー最大長（超過時はエラー。デフォルト: 100）
    pub max_queue_size: usize,
    /// ポーリング間隔（ミリ秒。デフォルト: 250）
    pub poll_interval_ms: u64,
}

impl Default for AgentPoolConfig {
    fn default() -> Self {
        Self {
            max_concurrent: 4,
            max_queue_size: 100,
            poll_interval_ms: 250,
        }
    }
}

// ============================================================================
// AgentPoolResult — enqueue の結果
// ============================================================================

/// `AgentPool::enqueue()` の戻り値。
#[derive(Debug, Clone, PartialEq)]
pub enum EnqueueResult {
    /// 即時実行開始（空きスロットあり）
    Started,
    /// キューに追加（空きスロットなし、順番待ち）
    Queued { position: usize },
    /// キューがあふれた
    QueueFull,
}

// ============================================================================
// PoolStatus — 現在のプール状態
// ============================================================================

/// プールの現在の状態。
#[derive(Debug, Clone)]
pub struct PoolStatus {
    /// 現在実行中のタスク数
    pub active_count: usize,
    /// キューで待機中のタスク数
    pub queued_count: usize,
    /// 実行中のタスクID一覧
    pub active_task_ids: Vec<i64>,
    /// キュー待ちのタスクID一覧
    pub queued_task_ids: Vec<i64>,
    /// 答え待ちで枠を外しているタスク数
    pub waiting_count: usize,
    /// 答え待ちのタスクID一覧
    pub waiting_task_ids: Vec<i64>,
}

// ============================================================================
// AgentPool
// ============================================================================

/// エージェント実行プール。
///
/// 内部で `Mutex` によりスレッドセーフな状態管理を行います。
/// `Condvar` でタスク完了待機をサポートします。
pub struct AgentPool {
    /// 最大同時実行数
    max_concurrent: usize,
    /// キュー最大長
    max_queue_size: usize,
    /// 内部状態（Mutex で保護）
    inner: Mutex<PoolInner>,
    /// 状態変化の通知用
    condvar: Condvar,
}

/// 内部状態（Mutex で保護される）。
struct PoolInner {
    /// 現在実行中のタスクIDのセット
    active: HashSet<i64>,
    /// 答え待ちのタスクID（枠を持たない。`suspend` で active から移る）
    waiting: HashSet<i64>,
    /// 実行待ちタスクIDのキュー（FIFO）
    queue: VecDeque<i64>,
}

impl AgentPool {
    /// 新しい `AgentPool` をデフォルト設定で作成する。
    pub fn new() -> Self {
        Self::with_config(AgentPoolConfig::default())
    }

    /// 設定付きで新しい `AgentPool` を作成する。
    pub fn with_config(config: AgentPoolConfig) -> Self {
        Self {
            max_concurrent: config.max_concurrent,
            max_queue_size: config.max_queue_size,
            inner: Mutex::new(PoolInner {
                active: HashSet::new(),
                waiting: HashSet::new(),
                queue: VecDeque::new(),
            }),
            condvar: Condvar::new(),
        }
    }

    /// 最大同時実行数で新しい `AgentPool` を作成する。
    pub fn with_max_concurrent(max_concurrent: usize) -> Self {
        Self::with_config(AgentPoolConfig {
            max_concurrent,
            ..Default::default()
        })
    }

    /// タスクを実行キューに追加する。
    ///
    /// 空きスロットがあれば `Started` を返す。
    /// 空きがなければ `Queued` を返す（キュー待ち）。
    /// キューがあふれた場合は `QueueFull` を返す。
    pub fn enqueue(&self, task_id: i64) -> EnqueueResult {
        let mut inner = self.inner.lock().unwrap();

        // 既に active または waiting（答え待ち）なら実行中。queue にあれば順番を返す
        if inner.active.contains(&task_id) || inner.waiting.contains(&task_id) {
            return EnqueueResult::Started;
        }
        if let Some(pos) = inner.queue.iter().position(|&id| id == task_id) {
            return EnqueueResult::Queued { position: pos + 1 };
        }

        // 空きスロットがあれば即時実行
        if inner.active.len() < self.max_concurrent {
            inner.active.insert(task_id);
            return EnqueueResult::Started;
        }

        // キューに空きがあれば追加
        if inner.queue.len() < self.max_queue_size {
            inner.queue.push_back(task_id);
            let position = inner.queue.len();
            return EnqueueResult::Queued { position };
        }

        EnqueueResult::QueueFull
    }

    /// タスクの実行完了を通知する。
    ///
    /// active から削除し、キュー待ちの先頭タスクがあれば active に昇格させる。
    /// 昇格したタスクの ID を返す（なければ `None`）。
    pub fn complete(&self, task_id: i64) -> Option<i64> {
        let mut inner = self.inner.lock().unwrap();
        // 答え待ちの子は枠を持っていないので、空いた枠はない（昇格もしない）
        if inner.waiting.remove(&task_id) {
            self.condvar.notify_all();
            return None;
        }
        inner.active.remove(&task_id);

        // キューから次のタスクを昇格
        let promoted = if let Some(next_id) = inner.queue.pop_front() {
            inner.active.insert(next_id);
            Some(next_id)
        } else {
            None
        };

        // 状態変化を通知
        self.condvar.notify_all();

        promoted
    }

    /// 実行中のタスクをキャンセルする。
    ///
    /// active から削除し、キューが空くのを待っている後続タスクを処理可能にする。
    pub fn cancel(&self, task_id: i64) {
        let mut inner = self.inner.lock().unwrap();
        // 答え待ちの子は枠を持っていないので、昇格もしない（complete と同じ）
        if inner.waiting.remove(&task_id) {
            self.condvar.notify_all();
            return;
        }
        inner.active.remove(&task_id);

        // キューから次のタスクを昇格
        if let Some(next_id) = inner.queue.pop_front() {
            inner.active.insert(next_id);
        }

        self.condvar.notify_all();
    }

    /// 答え待ちの子を同時実行の枠から外す（active → waiting）。
    ///
    /// 枠が空くので、キューの先頭（起動してよい子）を active に入れて返す。
    /// `task_id` が active に無ければ何もせず `None` を返す。
    pub fn suspend(&self, task_id: i64) -> Option<i64> {
        let mut inner = self.inner.lock().unwrap();
        if !inner.active.remove(&task_id) {
            return None;
        }
        inner.waiting.insert(task_id);

        let promoted = inner.queue.pop_front();
        if let Some(next_id) = promoted {
            inner.active.insert(next_id);
        }

        self.condvar.notify_all();
        promoted
    }

    /// 答えが来て動き出した子を枠に戻す（waiting → active）。
    ///
    /// 上限は見ない。戻すときに待たせると、答えを受けた子がまた止まるため。
    /// 一時的に `max_concurrent` を超えることは許す。
    pub fn resume(&self, task_id: i64) {
        let mut inner = self.inner.lock().unwrap();
        if inner.waiting.remove(&task_id) {
            inner.active.insert(task_id);
        }
        self.condvar.notify_all();
    }

    /// 全タスクの完了を待つ（最大 `timeout_ms` ミリ秒）。
    ///
    /// タイムアウトした場合は `false` を返す。
    pub fn wait_all(&self, timeout_ms: u64) -> bool {
        let mut inner = self.inner.lock().unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(timeout_ms);

        loop {
            if inner.active.is_empty() && inner.waiting.is_empty() && inner.queue.is_empty() {
                return true;
            }

            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            if remaining.is_zero() {
                return false;
            }

            // Condvar で待機（最大 remaining まで）
            let result = self
                .condvar
                .wait_timeout(inner, remaining)
                .unwrap_or_else(|e| e.into_inner());
            inner = result.0;

            // タイムアウトチェック
            if inner.active.is_empty() && inner.waiting.is_empty() && inner.queue.is_empty() {
                return true;
            }
            if std::time::Instant::now() >= deadline {
                return false;
            }
        }
    }

    /// 現在のプール状態を取得する。
    pub fn status(&self) -> PoolStatus {
        let inner = self.inner.lock().unwrap();
        PoolStatus {
            active_count: inner.active.len(),
            queued_count: inner.queue.len(),
            active_task_ids: inner.active.iter().copied().collect(),
            queued_task_ids: inner.queue.iter().copied().collect(),
            waiting_count: inner.waiting.len(),
            waiting_task_ids: inner.waiting.iter().copied().collect(),
        }
    }
}

impl Default for AgentPool {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// テスト
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_enqueue_started_when_slots_available() {
        let pool = AgentPool::with_max_concurrent(2);
        let result = pool.enqueue(1);
        assert_eq!(result, EnqueueResult::Started);
        let status = pool.status();
        assert_eq!(status.active_count, 1);
        assert!(status.active_task_ids.contains(&1));
    }

    #[test]
    fn test_enqueue_queued_when_no_slots() {
        let pool = AgentPool::with_max_concurrent(1);
        // 1つ目: 即時実行
        assert_eq!(pool.enqueue(1), EnqueueResult::Started);
        // 2つ目: キュー待ち
        assert_eq!(pool.enqueue(2), EnqueueResult::Queued { position: 1 });
        let status = pool.status();
        assert_eq!(status.active_count, 1);
        assert_eq!(status.queued_count, 1);
    }

    #[test]
    fn test_enqueue_queue_full() {
        let pool = AgentPool::with_config(AgentPoolConfig {
            max_concurrent: 1,
            max_queue_size: 2,
            poll_interval_ms: 250,
        });
        assert_eq!(pool.enqueue(1), EnqueueResult::Started);
        assert_eq!(pool.enqueue(2), EnqueueResult::Queued { position: 1 });
        assert_eq!(pool.enqueue(3), EnqueueResult::Queued { position: 2 });
        assert_eq!(pool.enqueue(4), EnqueueResult::QueueFull);
    }

    #[test]
    fn test_complete_promotes_next() {
        let pool = AgentPool::with_max_concurrent(1);
        pool.enqueue(1); // Started
        pool.enqueue(2); // Queued

        let promoted = pool.complete(1);
        assert_eq!(promoted, Some(2));

        let status = pool.status();
        assert_eq!(status.active_count, 1);
        assert!(status.active_task_ids.contains(&2));
        assert_eq!(status.queued_count, 0);
    }

    #[test]
    fn test_complete_no_queue() {
        let pool = AgentPool::with_max_concurrent(2);
        pool.enqueue(1);
        let promoted = pool.complete(1);
        assert_eq!(promoted, None);
        assert_eq!(pool.status().active_count, 0);
    }

    #[test]
    fn test_cancel_promotes_next() {
        let pool = AgentPool::with_max_concurrent(1);
        pool.enqueue(1);
        pool.enqueue(2);
        pool.cancel(1);

        let status = pool.status();
        assert_eq!(status.active_count, 1);
        assert!(status.active_task_ids.contains(&2));
        assert_eq!(status.queued_count, 0);
    }

    #[test]
    fn test_wait_all_empty() {
        let pool = AgentPool::new();
        assert!(pool.wait_all(10));
    }

    #[test]
    fn test_wait_all_with_active() {
        let pool = AgentPool::with_max_concurrent(1);
        pool.enqueue(1);
        // アクティブなタスクがある状態では即座に false
        assert!(!pool.wait_all(10));
        pool.complete(1);
        assert!(pool.wait_all(10));
    }

    #[test]
    fn test_duplicate_enqueue() {
        let pool = AgentPool::with_max_concurrent(2);
        assert_eq!(pool.enqueue(1), EnqueueResult::Started);
        // 同じタスクIDの再エンキューは Started を返す（既に active）
        assert_eq!(pool.enqueue(1), EnqueueResult::Started);
        assert_eq!(pool.status().active_count, 1);
    }

    #[test]
    fn test_status_empty() {
        let pool = AgentPool::new();
        let status = pool.status();
        assert_eq!(status.active_count, 0);
        assert_eq!(status.queued_count, 0);
    }

    #[test]
    fn test_fifo_order() {
        let pool = AgentPool::with_max_concurrent(1);
        pool.enqueue(10);
        pool.enqueue(20);
        pool.enqueue(30);

        assert_eq!(pool.complete(10), Some(20));
        assert_eq!(pool.complete(20), Some(30));
        assert_eq!(pool.complete(30), None);
    }

    #[test]
    fn test_suspend_frees_slot_for_queued_and_resume_restores() {
        // AC1: 上限 2 で A・B が active、C が queue
        let pool = AgentPool::with_max_concurrent(2);
        pool.enqueue(1); // A
        pool.enqueue(2); // B
        assert_eq!(pool.enqueue(3), EnqueueResult::Queued { position: 1 }); // C

        // A が答え待ち → 枠が空いて C が起動する
        assert_eq!(pool.suspend(1), Some(3));
        let status = pool.status();
        assert_eq!(status.active_count, 2);
        assert!(status.active_task_ids.contains(&2) && status.active_task_ids.contains(&3));
        assert_eq!(status.waiting_task_ids, vec![1]);
        assert_eq!(status.queued_count, 0);

        // A に答えが来て動き出す → 上限を見ずに active へ戻る
        pool.resume(1);
        assert_eq!(pool.status().active_count, 3);
        assert_eq!(pool.status().waiting_count, 0);

        // A が終わる → active から消える（昇格なし）
        assert_eq!(pool.complete(1), None);
        assert_eq!(pool.status().active_count, 2);
    }

    #[test]
    fn test_complete_while_waiting_does_not_promote() {
        // 答え待ちの子が完了しても枠は空いていないので、queue を昇格させない
        let pool = AgentPool::with_max_concurrent(1);
        pool.enqueue(1);
        assert_eq!(pool.suspend(1), None); // queue が空
        pool.enqueue(2); // 枠が空いているので即 Started
        pool.enqueue(3); // queue へ
        assert_eq!(pool.complete(1), None);
        let status = pool.status();
        assert_eq!(status.active_task_ids, vec![2]);
        assert_eq!(status.queued_task_ids, vec![3]);
        assert_eq!(status.waiting_count, 0);
    }

    #[test]
    fn test_suspend_unknown_task_is_noop() {
        let pool = AgentPool::with_max_concurrent(1);
        pool.enqueue(1);
        pool.enqueue(2); // queue
        assert_eq!(pool.suspend(2), None, "queue の子は suspend できない");
        assert_eq!(pool.status().active_count, 1);
        assert_eq!(pool.status().queued_count, 1);
    }

    #[test]
    fn test_waiting_task_counts_as_running() {
        let pool = AgentPool::with_max_concurrent(1);
        pool.enqueue(1);
        pool.suspend(1);
        assert_eq!(
            pool.enqueue(1),
            EnqueueResult::Started,
            "答え待ちも登録済み扱い"
        );
        assert!(!pool.wait_all(10), "答え待ちがいる間は全体が終わっていない");
        pool.cancel(1);
        assert!(pool.wait_all(10));
    }
}
