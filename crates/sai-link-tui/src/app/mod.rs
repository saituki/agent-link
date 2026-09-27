//! アプリケーション状態とイベントループ。
//!
//! Sai-TUI の中核。DB からのデータロード、キー入力処理、
//! 画面描画のオーケストレーションを行います。
//!
//! ブロック実行機能は sai CLI（`sai block run --interactive`）に移行されました。

use std::collections::HashMap;
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

use crossterm::event::{self, Event, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::Rect;
use ratatui::Frame;
use sai_link_engine::db::SaiStore;

use crate::ui::{
    layout::{compute_layout, AppLayout},
    left_sidebar::{LeftSidebar, LeftSidebarState, TreeItem, TreeItemKind},
    log_view::{BlockDetail, LogState, LogView},
    main_panel::{BlockDisplayData, ConnectionDisplayData, MainPanel, MainPanelState},
    modals::{render_modal, Modal},
    ViewMode,
};
use sai_link_widgets::theme::AppTheme;

// ============================================================================
// AppState
// ============================================================================

/// CLI 実行ポップアップ（`x` モード・`block run --kanban` / `block rm` の TUI 再現）。
pub mod card_wizard;
pub mod cli_mode;
mod data;
/// テスト専用: `SAI_PROJECT_ROOT` を触るテストの共有ロック。
///
/// `SAI_PROJECT_ROOT` はプロセスグローバルのため、別 Mutex で直列化しても
/// 環境競合する。全テストはこの1本のロックで束ねる。
#[cfg(test)]
pub(crate) static FILE_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
/// 共有ロックを取得する（`SAI_PROJECT_ROOT` を触るテストは必ず経由する）。
///
/// 1 つ前のテストが panic してロックが毒化していても、そのまま使い続ける。
/// 毒化を `.unwrap()` で握りつぶすと、1 件の失敗が他テスト全部へ
/// 「PoisonError」として伝播して失敗数が不当に増えるため（偶発失敗の原因之一）。
#[cfg(test)]
pub(crate) fn file_test_lock() -> std::sync::MutexGuard<'static, ()> {
    FILE_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}
mod draw;
pub mod external;
pub mod input;
pub mod kanban;
mod mouse;
mod ui_layout;

// サブモジュールの公開関数（run_event_loop から呼ぶ）
use draw::draw_ui;
use input::handle_key_event;
use mouse::handle_mouse_event;
// mouse.rs のズーム関数（input.rs からも呼ぶ）
pub(crate) use mouse::zoom_main_panel;
pub struct AppState {
    /// DB ストア
    pub store: Arc<SaiStore>,

    // ── データキャッシュ ──
    /// ブロック表示データ（メイン画面用、ブロック種別ごとに1つ）
    pub block_displays: Vec<BlockDisplayData>,
    /// 接続表示データ
    pub connection_displays: Vec<ConnectionDisplayData>,
    /// 実行バッチのメイン複製実行ID（`run_batches.mb_execution_id`）の集合（U13・R25）。
    /// 左SB の実行一覧からメイン複製（mb1.x）を除外するために保持する。
    pub mb_replica_execs: std::collections::HashSet<i64>,
    /// sai-write 検出済み（finalized 実行バッチを持つ）実メイン実行 ID の集合。
    /// メイン画面で最終フェーズノード（P_n = sai-write 由来）を合成するかどうかの判定に使用。
    pub finalized_run_parents: std::collections::HashSet<i64>,
    /// 幽霊（orphaned_run）の実行 ID 集合（P1-1）。
    /// `load_data` 時にDB直読みで構築し、デーモン停止中も表示する。空なら表示不変。
    pub orphaned_run_ids: std::collections::HashSet<i64>,
    /// Reflection 記録の記入状況（Notification 色分け用）。
    ///
    /// 起動時とリロード（Shift+R）時にのみ判定し、結果を覚え置きして
    /// 使い回す（日々の描画のたびにファイルを読みに行かない）。
    /// 実行中・待機中の項目は色分け対象外のため値が入らない（None 扱い）。
    pub reflection_status: HashMap<i64, sai_link_engine::reflection_status::ReflectionStatus>,
    /// Kanban ボード表示用のカード一覧キャッシュ（`load_data` 時に構築・使い回し）。
    ///
    /// 描画のたびに DB・ファイル走査すると遅いため、`load_data` で 1 回作り、
    /// `draw_ui`・左ツリー構築では複製せず借用する。データ変更時は
    /// `reload_data`（= `load_data`）で作り直す。
    pub kanban_entries: Vec<crate::app::kanban::CardEntry>,

    // ── UI 状態 ──
    /// 表示モード
    pub view_mode: ViewMode,
    /// カラーテーマ
    pub theme: AppTheme,
    /// モーダルダイアログ（None = 非表示）
    pub modal: Modal,
    /// 現在の画面レイアウト（マウスクリック判定用）
    pub layout: AppLayout,
    /// 左SBの幅比率（本文領域に対する%）。右SB は廃止された
    pub left_panel_pct: u16,
    /// Kanban の高さ比率（メイン高さに対する%。残りは Workflow）。
    /// 枠線ドラッグで変更可能（15〜70% にクランプ）
    pub kanban_pct: u16,
    /// ドラッグ状態（パネル境界リサイズ中）
    pub drag: Option<PanelDrag>,

    /// 左サイドバー状態
    pub left_sidebar: LeftSidebarState,
    /// メイン画面状態
    pub main_panel: MainPanelState,

    /// Log 画面状態（旧右SB の内容。`l` 押下時点の実行詳細で固定）
    pub log_state: LogState,
    /// 左SB カーソル位置の実行 ID（ステータスバー右端の表示用）。
    ///
    /// 注意：`l/s/o/h/r/p`・`Space r/t/m`・`x` の対象決定には使わない。
    /// 各 shortcut は押下時点の sidebar カーソル（`sidebar_selected_exec_id`）から
    /// 直接決める（保持値の stale で別ブロックのファイルを開かないため）。
    pub selected_exec_id: Option<i64>,
    /// `Space` リーダーキー待機中か（`Space` → `r`/`t`/`m` で md-tui を開く）
    pub space_pending: bool,

    /// Stats ビュー状態（キャッシュ / トークン統計グラフ。sai-cache-tui-rs 移植）
    pub cache_view: crate::ui::cache_view::CacheViewState,

    /// 検索画面状態（セッション履歴検索。UI 状態のみ保持し、
    /// 検索ロジックは `sai_link_engine::search` に委譲する）
    pub search: crate::ui::search::SearchState,

    /// model-tui 状態（`Shift+M` で遷移。pi の使用可能モデル一覧）。
    pub model_view: crate::ui::model_view::ModelViewState,

    /// 一時メッセージ（エラー・情報表示）。画面下端に数秒間オーバーレイ表示する
    pub message: Option<String>,
    pub message_until: Option<Instant>,
    /// CLI モード状態（`x` で入る `block run` / `block rm` ポップアップ）
    pub cli_mode: cli_mode::CliModeState,
    /// バックグラウンドスレッド（herdr pane 作成等）からのメッセージ受信口
    pub background_rx: Option<mpsc::Receiver<String>>,

    /// 実行中か（false で終了）
    pub running: bool,

    /// ドラッグ選択状態（Shift+左ドラッグ / 右ドラッグ。None = 非選択）
    pub selection: Option<Selection>,
    /// 画面テキストのキャッシュ（選択コピー用。draw_ui で毎フレーム更新）
    pub screen_cache: Vec<Vec<char>>,

    /// 画面フォーカス（`Tab` で sidebar → kanban → workflow を循環）。
    ///
    /// スクロール（ホイール・矢印キー）が効くのはフォーカス中の画面だけ。
    pub focus: ScreenFocus,
    /// Kanban レーンの縦スクロール位置（4レーン同時）。
    pub kanban_scroll: usize,
}

/// フォーカス可能な画面（`Tab` で切り替え）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScreenFocus {
    /// 左サイドバー（ツリー）
    Sidebar,
    /// Kanbanボード（上段）
    Kanban,
    /// Workflow（下段）
    Workflow,
}

impl ScreenFocus {
    /// 次のフォーカス（末尾から先頭へ循環）。
    pub fn next(self) -> Self {
        match self {
            ScreenFocus::Sidebar => ScreenFocus::Kanban,
            ScreenFocus::Kanban => ScreenFocus::Workflow,
            ScreenFocus::Workflow => ScreenFocus::Sidebar,
        }
    }

    /// 1 つ前のフォーカス（`Shift+Tab` 用）。
    pub fn prev(self) -> Self {
        match self {
            ScreenFocus::Sidebar => ScreenFocus::Workflow,
            ScreenFocus::Kanban => ScreenFocus::Sidebar,
            ScreenFocus::Workflow => ScreenFocus::Kanban,
        }
    }

    /// ステータスバー表示用のラベル。
    pub fn label(self) -> &'static str {
        match self {
            ScreenFocus::Sidebar => "sidebar",
            ScreenFocus::Kanban => "kanban",
            ScreenFocus::Workflow => "workflow",
        }
    }
}

/// ドラッグによるテキスト選択状態。
///
/// 座標は画面（スクリーン）セル座標。`draw_ui` が `screen_cache` から
/// 矩形内の文字を抜き出してコピーする。
#[derive(Debug, Clone, Copy)]
pub struct Selection {
    /// 選択開始位置（アンカー）
    pub anchor_col: u16,
    pub anchor_row: u16,
    /// 現在の選択端（ドラッグ中に更新）
    pub end_col: u16,
    pub end_row: u16,
    /// md ビュー（History / Blocks のメイン画面）での行単位選択か。
    /// true ならハイライトとコピーが「行単位」（エディター風）になる。
    pub line_mode: bool,
}

/// ターミナル内でバックグラウンド実行されていたコマンドの型は削除済み
/// （ボトムステータスバーの統合ターミナル廃止に伴い削除）。

impl AppState {
    /// 新しいアプリケーション状態を作成する。
    pub fn new(store: Arc<SaiStore>) -> Self {
        let theme = AppTheme::dark();

        // card / round 区切り線のドラッグによるズレ量を起動時に 1 回読み込む
        // （`.sai/ui-layout.json`。無ければ空のまま = 初期位置）。
        let saved_layout = ui_layout::UiLayout::load();
        let main_panel = MainPanelState {
            node_offsets: saved_layout.nodes,
            separator_offsets: saved_layout.separators,
            ..MainPanelState::default()
        };

        Self {
            store,
            block_displays: Vec::new(),
            connection_displays: Vec::new(),
            mb_replica_execs: std::collections::HashSet::new(),
            finalized_run_parents: std::collections::HashSet::new(),
            orphaned_run_ids: std::collections::HashSet::new(),
            reflection_status: HashMap::new(),
            kanban_entries: Vec::new(),
            view_mode: ViewMode::Workflow,
            theme,
            modal: Modal::None,
            layout: AppLayout {
                left_sidebar: ratatui::layout::Rect::new(0, 0, 20, 20),
                main: ratatui::layout::Rect::new(20, 0, 60, 20),
                kanban: ratatui::layout::Rect::new(20, 0, 60, 8),
                workflow: ratatui::layout::Rect::new(20, 8, 60, 12),
                body: ratatui::layout::Rect::new(0, 0, 80, 19),
                status_bar: ratatui::layout::Rect::new(0, 19, 80, 1),
            },
            left_panel_pct: 25,
            kanban_pct: 40,
            drag: None,
            left_sidebar: LeftSidebarState::default(),
            main_panel,
            log_state: LogState::default(),
            selected_exec_id: None,
            space_pending: false,
            message: None,
            message_until: None,
            cli_mode: cli_mode::CliModeState::default(),
            background_rx: None,
            cache_view: crate::ui::cache_view::CacheViewState::default(),
            search: crate::ui::search::SearchState::default(),
            model_view: crate::ui::model_view::ModelViewState::default(),
            running: true,
            selection: None,
            screen_cache: Vec::new(),
            // 既定のフォーカスは Workflow（従来の操作感を維持）
            focus: ScreenFocus::Workflow,
            kanban_scroll: 0,
        }
    }

    /// sidebar が操作可能か（`Workflow` 表示中のみ真）。
    pub fn is_sidebar_enabled(&self) -> bool {
        self.view_mode == ViewMode::Workflow
    }

    /// model-tui を開く（`Shift+M`）。一覧を再読込して `ViewMode::Model` へ遷移する。
    pub fn open_model_view(&mut self) {
        self.model_view.load();
        self.view_mode = ViewMode::Model;
    }

    /// 一時メッセージを表示する（3 秒）。
    pub fn set_message(&mut self, msg: impl Into<String>) {
        self.message = Some(msg.into());
        self.message_until = Some(Instant::now() + Duration::from_secs(3));
    }

    /// フォーム値で検索を実行し `SearchState` を更新する。
    ///
    /// 検索ロジックは engine（`parse_query` / `run_search_verbose`）に委譲し、
    /// 書式エラーは `SearchState.error` へ文字列化して保持する。
    pub(crate) fn run_session_search(&mut self) {
        let query = match sai_link_engine::search::parse_query(&self.search.form) {
            Ok(q) => q,
            Err(e) => {
                self.search.error = Some(e.to_string());
                self.search.searched = false;
                return;
            }
        };
        match sai_link_engine::search::run_search_verbose(&self.store, &query) {
            Ok((hits, skipped)) => {
                let mut hits = hits;
                // 出自プロジェクト名（L2）: 自プロジェクト検索のため表示名を補完する
                for h in &mut hits {
                    if h.project.is_empty() {
                        h.project = self.search.project_name.clone();
                    }
                }
                self.search.hits = hits;
                self.search.skipped = skipped;
                self.search.selected = 0;
                self.search.scroll = 0;
                self.search.error = None;
                self.search.searched = true;
            }
            Err(e) => {
                self.search.error = Some(e.to_string());
                self.search.searched = false;
            }
        }
    }

    /// 期限切れメッセージを消し、バックグラウンド処理の結果をメッセージへ反映する。
    ///
    /// イベントループの各反復で呼ぶ。
    pub fn tick_messages(&mut self) {
        if let Some(rx) = &self.background_rx {
            if let Ok(msg) = rx.try_recv() {
                self.set_message(msg);
            }
        }
        if let Some(until) = self.message_until {
            if Instant::now() > until {
                self.message = None;
                self.message_until = None;
            }
        }
    }
}

pub struct PanelDrag {
    pub border: DragBorder,
    /// ドラッグ開始時のカーソル位置（メイン画面パン用）
    pub start_col: u16,
    pub start_row: u16,
    /// ドラッグ開始時のスクロール位置（メイン画面パン用）
    pub start_scroll_x: i32,
    pub start_scroll_y: i32,
    /// メイン画面パン: 移動量が閾値を超えてドラッグ中かどうか
    pub pan_active: bool,
}

/// ドラッグ可能な操作。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DragBorder {
    /// 左SB ↔ メインの境界（幅変更）
    LeftMain,
    /// Kanban ↔ Workflow の境界（高さ変更）
    KanbanWorkflow,
    /// メイン画面のドラッグによるビューポート移動
    MainPan,
    /// card のドラッグ移動。`base` はドラッグ開始時点のズレ量（Esc キャンセルで戻す先）
    Node { exec_id: i64, base: (i32, i32) },
    /// round 区切り線ラベルのドラッグ（縦方向のみ）。`base` は開始時点のズレ量
    Separator { global_round: i64, base: i32 },
}

// ============================================================================
// イベントハンドラ
// ============================================================================

/// イベントループを実行する。画面再描画が必要なら描画する。
pub fn run_event_loop<B: ratatui::backend::Backend>(
    terminal: &mut ratatui::Terminal<B>,
    state: &mut AppState,
) -> std::io::Result<()> {
    // 初回フレームを描画してからループに入る（イベントが無い間は描画しないため）
    terminal.draw(|frame| draw_ui(frame, state))?;

    while state.running {
        // 溜まっているイベントを一括処理（非ブロッキング）。
        // 連続するホイール/キーリピートを取りこぼさず、1 反復で全部さばいて
        // 最後に 1 回だけ再描画する（スクロール追従の遅れを解消）。
        let mut handled = false;
        while event::poll(std::time::Duration::ZERO)? {
            match event::read()? {
                Event::Key(key) => {
                    if handle_key_event(state, key) {
                        handled = true;
                    }
                }
                Event::Mouse(mouse) => {
                    if handle_mouse_event(state, mouse) {
                        handled = true;
                    }
                }
                Event::Resize(_, _) => handled = true, // リサイズは即再描画
                _ => {}
            }
            if !state.running {
                break;
            }
        }

        // 一時メッセージの寿命切替・バックグラウンド処理結果の反映。
        // 変化があった場合は再描画する。
        let msg_before = state.message.clone();
        state.tick_messages();
        if state.message != msg_before {
            handled = true;
        }

        // 描画（イベントがあったときのみ。アイドル時は描画ゼロ）
        if handled {
            terminal.draw(|frame| draw_ui(frame, state))?;
        }

        // 次イベントを短時間待機（16ms ≒ 60fps 相当。イベント到着時は即起床）
        if state.running {
            let _ = event::poll(std::time::Duration::from_millis(16))?;
        }
    }

    Ok(())
}

// ============================================================================
// ユーティリティ
// ============================================================================

/// マークダウン表示のスクロール位置を変更する。
///
/// History / Blocks モードで、キー入力（input.rs）とマウスホイール（mouse.rs）
/// の両方から呼ばれる。スクロールは表示行数を超えない範囲にクランプされる。
pub(crate) fn md_scroll(state: &mut AppState, delta: i32) {
    // md viewer: カーソル行を移動し、ビューポート内に追従スクロールする
    // （`j/k`・矢印・ホイール・PgUp/PgDn）。
    if let Some(md) = &mut state.main_panel.md_view {
        md.cursor_move(delta);
    }
    // Project ダッシュボードのスクロールも同期する（ratatui ウィジェット描画）
    if let Some(dash) = &mut state.main_panel.project_dash {
        let max = dash.max_scroll as i64;
        dash.scroll = ((dash.scroll as i64 + i64::from(delta)).clamp(0, max)) as usize;
    }
}
