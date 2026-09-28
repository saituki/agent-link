//! UI コンポーネント。

pub mod cache_view;
pub mod draw;
pub mod history_view;
pub mod kanban;
pub mod layout;
pub mod left_sidebar;
pub mod log_view;
pub mod main_panel;
pub mod markdown;
pub mod modals;
pub mod model_view;
pub mod search;

/// メイン画面の表示モード。
///
/// Header 廃止に伴い、タブ循環（F1）からショートカット切替（トグル）へ移行した。
/// 各モードは同じショートカットを 2 度押すと Workflow に戻る。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewMode {
    /// ワークフロー（ブロック接続グラフ）
    Workflow,
    /// Log（旧右SB のブロック詳細。`l` で遷移）
    Log,
    /// History（セッション出力のマークダウン表示。`h` で遷移）
    History,
    /// Stats（キャッシュ / トークン統計グラフ。`s` で遷移）
    Stats,
    /// Project（プロジェクト概要・編集可。`P` で遷移）
    Project,
    /// Search（セッション履歴検索。`Shift+F` で遷移）
    Search,
    /// message（session の user / assistant message。`Space m` で遷移）
    Message,
    /// md-tui: task.md / message.md のレンダリング表示（`Space t/m` で遷移）
    Md,
    /// model-tui: pi agent で使用可能な model / provider 一覧（`Shift+M` で遷移）
    Model,
}

/// md-tui で開く md 種別（本体は `sai_link_widgets::phone_view`・`sai-link phone md --kind` と同一）。
pub use sai_link_widgets::phone_view::MdKind;

impl ViewMode {
    /// Stats ビュー（キャッシュ統計グラフ）かどうか。
    pub fn is_stats(self) -> bool {
        matches!(self, ViewMode::Stats)
    }
}
