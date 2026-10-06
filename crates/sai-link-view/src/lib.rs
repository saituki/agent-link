//! # sai-link-view — Web と CLI が使う組み立て処理
//!
//! `sai-link phone …`（Web の中継）が使う「データを組み立てる関数」を集めた
//! クレート。描画は持たない（画面は Web だけにする決定による）。
//!
//! ## 構成
//!
//! - [`block_history`] — History（Token Usage by Category + Timeline）の表示データ組み立て
//! - [`block_log`] — Log（`[Inputs] [Meta] [Output] [Stats]` 相当）の表示データ組み立て
//! - [`cache_stats`] — Stats（キャッシュ / トークン統計）の表示データ組み立て
//! - [`card_entries`] — Kanban カード・レーン一覧の組み立て
//! - [`left_tree`] — 左サイドバー ツリーの組み立て（`phone tree`）
//! - [`card_wizard`] — task.md 作成ウィザードの組み立て・検査（`phone card`）
//! - [`phone_view`] — run の中身（md / waggle）の解決（`phone md` / `phone waggle`）
//! - [`workflow`] — 実行グラフ（DAG）の組み立て・配置・材料集め（`phone workflow`）
//! - [`user_run`] — 利用者の札（`runner: user`）の始める・終える処理（`phone user`）
//! - [`fmt`] — 数値・費用・トークンの表示書式
//!
//! `block_history` / `block_log` / `cache_stats` は、もともと TUI の `ui/` 配下に
//! あったものを移設したもので、**ビルダ純関数**（`build(store, exec_id)` /
//! `load(store, exec_id)`）を提供する。描画部分は TUI の削除に合わせて外した。

pub mod block_history;
pub mod block_log;
pub mod cache_stats;
pub mod card_entries;
pub mod card_wizard;
pub mod fmt;
pub mod left_tree;
pub mod phone_view;
pub mod user_run;
pub mod workflow;

/// テスト用の共有ロック。`SAI_PROJECT_ROOT` はプロセス共通なので、
/// この環境変数を書き換えるテストは全部このロックを取る（並行実行での干渉を避ける）。
#[cfg(test)]
pub(crate) static TEST_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
