//! # Sai-Engine — AI コーディングエージェントワークフローエンジン
//!
//! Sai-Engine はブロック定義ファイル（Markdown + YAML frontmatter）のパース、
//! 検証、pi エージェント起動コマンドの構築、herdr CLI 連携を提供するライブラリです。
//!
//! ## ブロック定義ファイルのフォーマット
//!
//! ブロック定義ファイルは `.md` 拡張子を持ち、先頭に YAML frontmatter を
//! `---` で区切って配置します。
//!
//! ```markdown
//! ---
//! name: my-block
//! description: "ブロックの説明"
//! model: "anthropic/claude-sonnet-4-5"
//! thinking: high
//! tools: read,bash,edit
//! inputs:
//!   - id: project_name
//!     type: text
//!     label: プロジェクト名
//! output:
//!   format: markdown
//! ---
//! # ブロックプロンプト本文
//! ```
//!
//! ## 使用例 — ブロック定義のパース
//!
//! ```no_run
//! use sai_link_engine::parse_block_file;
//!
//! // ブロック定義ファイルをパース
//! let parsed = parse_block_file("path/to/block.md")
//!     .expect("ブロック定義ファイルのパースに失敗しました");
//!
//! println!("ブロック名: {}", parsed.config.name);
//! println!("モデル: {:?}", parsed.config.model);
//! println!("本文: {}", parsed.body);
//! ```
//!
//! ## 使用例 — pi 起動コマンドの構築
//!
//! ```no_run
//! use sai_link_engine::block_config::BlockConfig;
//! use sai_link_engine::pi_command::PiCommandBuilder;
//!
//! let config = BlockConfig {
//!     id: Some("my-block".to_string()),
//!     name: "🧪 マイブロック".to_string(),
//!     description: None,
//!     model: Some("anthropic/claude-sonnet-4-5".to_string()),
//!     provider: None,
//!     thinking: Some("high".to_string()),
//!     tools: None,
//!     extensions: None,
//!     skills: None,
//!     subblocks: None,
//!     timeout: None,
//!     worktree: None,
//!     enable: None,
//!     category: None,
//!     tag: None,
//!     round: None,
//!     layer: None,
//!     templates: None,
//!     context: None,
//! };
//!
//! let cmd = PiCommandBuilder::new("/tmp/sessions")
//!     .with_config(config)
//!     .build()
//!     .expect("pi 起動コマンドの構築に失敗しました");
//!
//! println!("実行コマンド: {}", cmd.command);
//! ```
//!
//! ## モジュール構成
//!
//! - `block_config` — `BlockConfig` 構造体と関連するデータ型
//! - `parser` — フロントマターパーサー（`parse_block_file`, `parse_block_string`）
//! - `validation` — バリデーションロジック（`validate`, `ValidationResult`）
//! - `error` — エラー型（`ParseError`）
//! - `herdr` — herdr CLI 連携（ペイン操作、シェルエスケープ）
//! - `pi_command` — Pi エージェント起動コマンドビルダー（`PiCommandBuilder`）
//! - `session` — JSONL セッションファイル管理（作成・読み取り・クリーンアップ）
//! - `init` — プロジェクト初期化（`PROJECTROOT/.sai/` の作成）
//! - `placeholder` — `{{input.<id>}}` プレースホルダー展開ユーティリティ
//! - `orchestrator` — エージェント実行オーケストレーター（パイプライン実行）
//! - `waggle_integration` — waggle トークン化統合（出力ファイルのトークン化・解決）

pub mod block_config;
pub mod error;
pub mod herdr;
pub mod parser;
pub mod pi_command;
pub mod validation;

pub mod agent_api;
pub mod agent_pool;
pub mod cc_command;
pub mod common;
pub mod daemon;
pub mod db;
pub mod init;
pub mod kanban;
pub mod mcp_server;
pub mod orchestrator;
pub mod phone_export;
pub mod placeholder;
pub mod pre_run;
pub mod project;
pub mod reflection_status;
pub mod run_dir;
pub mod runner;
pub mod sai_write_notify;
pub mod search;
pub mod session;
pub mod task_cwd;
pub mod task_manager;
pub mod usage;
pub mod usage_metrics;
pub mod waggle_integration;

pub use block_config::*;
pub use error::ParseError;
pub use parser::*;
pub use session::SessionStats;
pub use validation::*;
