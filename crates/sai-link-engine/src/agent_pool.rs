//! # エージェント実行プール — 同時実行制御
//!
//! 同時実行数の上限を管理し、FIFO キューでサブタスク実行を直列化します。
//! `lib/herdrs/agent-pool.ts` の Rust 移植です。
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
//!                          │ pi 完了
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

        // 既に active または queue に存在する場合はスキップ
        if inner.active.contains(&task_id) || inner.queue.contains(&task_id) {
            // 既に登録済み — 現在の状態を返す
            if inner.active.contains(&task_id) {
                return EnqueueResult::Started;
            }
            if let Some(pos) = inner.queue.iter().position(|&id| id == task_id) {
                return EnqueueResult::Queued { position: pos + 1 };
            }
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
        inner.active.remove(&task_id);

        // キューから次のタスクを昇格
        if let Some(next_id) = inner.queue.pop_front() {
            inner.active.insert(next_id);
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
            if inner.active.is_empty() && inner.queue.is_empty() {
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
            if inner.active.is_empty() && inner.queue.is_empty() {
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
}
