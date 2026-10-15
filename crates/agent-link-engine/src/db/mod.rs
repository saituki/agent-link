//! # agent-link データベースモジュール
//!
//! waggle の `SqliteStore` と同一の SQLite DB ファイル上に
//! agent-link 専用テーブル（`block_metadata` / `block_inputs` /
//! `block_output` / `execution_history` / `git_commits`）を作成し、
//! CRUD 操作を提供します。
//!
//! ## モジュール構成
//!
//! - `models` — テーブルに対応する Rust 構造体とエラー型
//! - `schema` — テーブル定義 DDL と初期化関数
//! - `store` — CRUD 操作の実装（`SaiStore`）
//!
//! ## 使用例
//!
//! ```no_run
//! use agent_link_engine::db::SaiStore;
//!
//! let store = SaiStore::open_in_memory().unwrap();
//! let blocks = store.list_blocks().unwrap();
//! println!("ブロック数: {}", blocks.len());
//! ```

pub mod models;
pub mod schema;
pub mod store;

pub use models::{
    BlockConnection, BlockInput, BlockMetadata, BlockOutput, ConnectionGroup, DbError,
    ExecutionHistory, GitCommit, RunBatch, RunBatchStatus, Task, WaggleTokenEntry,
};
pub use schema::init_sai_tables;
pub use store::SaiStore;
