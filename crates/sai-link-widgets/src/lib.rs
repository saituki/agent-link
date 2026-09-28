//! # sai-link-widgets — ダッシュボード widget 群（L3）
//!
//! `sai-link-tui` の Project 画面（`ui/project_dashboard.rs`）と
//! テーマ（`ui/theme.rs`）、円グラフ部品（`ui/cache_view.rs` の `MiniPie` /
//! `PieSliceData`）を分離したクレート。sai-control-tui が Project 画面を
//! **再実装せず再利用**するための path dependency 公開用である（D2）。
//!
//! ## 構成
//!
//! - [`theme`] — アプリケーションカラーテーマ（`AppTheme`）
//! - [`pie`] — 簡易円グラフ（`MiniPie` / `PieSliceData`・Braille ドット描画 + 凡例）
//! - [`project_dashboard`] — Project ダッシュボード（`ProjectDashData` + `render_dashboard`
//!   + 共通ビルダ `build_dashboard(store, root, fallback_name)`）
//! - [`cjk`] — CJK 文字幅対応描画ヘルパ（`draw_char_cjk` / `draw_text_cjk` / `draw_text_clipped`）
//! - [`block_history`] — History 画面（Token Usage by Category + Timeline）
//! - [`block_log`] — Log 画面（`[Inputs] [Meta] [Output] [Stats]` 4 セクション詳細）
//! - [`cache_stats`] — Stats 画面（キャッシュ / トークン統計グラフ・sai-cache-tui-rs 移植）
//! - [`pane`] — herdr pane 操作の純関数（`spawn_in_pane` / `build_resume_command` / `reflection_path`）
//! - [`card_entries`] — Kanban カード・レーン一覧の組み立て（TUI / `sai-link phone` 共通）
//! - [`left_tree`] — 左サイドバー ツリーの組み立て（`phone tree` 共通）
//! - [`card_wizard`] — task.md 作成ウィザードの組み立て・検査（`phone card` 共通）
//! - [`phone_view`] — run の中身（md / waggle）の解決（`phone md` / `phone waggle` 共通）
//! - [`workflow`] — 実行グラフ（DAG）の組み立て・配置・材料集め（TUI / `sai-link phone workflow` 共通）
//! - [`user_run`] — 利用者の札（`runner: user`）の始める・終える処理（TUI / `sai-link phone user` 共通）
//!
//! `block_history` / `block_log` / `cache_stats` は sai-link-tui の `ui/` 配下から
//! 移設したもので、描画 + 状態 + **ビルダ純関数**（`build(store, exec_id)` / `load(store, exec_id)`）
//! を提供する。各 TUI（sai-link-tui / sai-control-tui）は「対象プロジェクトの
//! store を開く → ビルダ呼び出し → 状態へ格納」のみを行い、同一 UI を共有する。
//!
//! ## 使用例
//!
//! ```no_run
//! use sai_link_widgets::project_dashboard::{render_dashboard, ProjectDashData};
//! use sai_link_widgets::theme::AppTheme;
//!
//! # fn main() {
//! let mut data = ProjectDashData::default();
//! data.name = "myproject".to_string();
//! // area は Terminal::draw のフレーム領域を渡す
//! // render_dashboard(area, buf, &mut data, &AppTheme::dark());
//! # }
//! ```

pub mod block_history;
pub mod block_log;
pub mod cache_stats;
pub mod card_entries;
pub mod card_wizard;
pub mod cjk;
pub mod fmt;
pub mod left_tree;
pub mod pane;
pub mod phone_view;
pub mod pie;
pub mod project_dashboard;
pub mod theme;
pub mod user_run;
pub mod workflow;
