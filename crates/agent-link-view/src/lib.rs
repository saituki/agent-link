//! # agent-link-view — CLI が使う表示用の組み立て
//!
//! CLI（`agent-link-cli`）が使う表示用の組み立てを集めたクレート。DB から読んで、
//! 命令が出す JSON の材料を組み立てる。描画は持たない。クレートを分けておくのは、
//! `engine → view` の依存をコンパイラが禁じるため。
//!
//! ## 構成
//!
//! - [`block_history`] — History（Token Usage by Category + Timeline）の表示データ組み立て
//! - [`block_log`] — Log（`[Inputs] [Meta] [Output] [Stats]` 相当）の表示データ組み立て
//! - [`cache_stats`] — Stats（キャッシュ / トークン統計）の表示データ組み立て
//! - [`card_entries`] — Kanban カード・レーン一覧の組み立て
//! - [`left_tree`] — 左サイドバー ツリーの組み立て（`run tree`）
//! - [`card_wizard`] — task.md 作成ウィザードの組み立て・検査（`card …`）
//! - [`run_docs`] — run の中身（md / waggle）の解決（`run md` / `doc list --run`）
//! - [`workflow`] — 実行グラフ（DAG）の組み立て・配置・材料集め（`workflow`）
//! - [`fmt`] — 数値・費用・トークンの表示書式
//!
//! `block_history` / `block_log` / `cache_stats` は、**ビルダ純関数**（`build(store, exec_id)` /
//! `load(store, exec_id)`）を提供する（`run history` / `run log` / `run stats`）。

pub mod block_history;
pub mod block_log;
pub mod cache_stats;
pub mod card_entries;
pub mod card_wizard;
pub mod fmt;
pub mod left_tree;
pub mod run_docs;
pub mod workflow;

/// テスト用の共有ロック。`AGENT_LINK_PROJECT_ROOT` はプロセス共通なので、
/// この環境変数を書き換えるテストは全部このロックを取る（並行実行での干渉を避ける）。
#[cfg(test)]
pub(crate) static TEST_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
