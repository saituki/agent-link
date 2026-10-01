//! キー入力処理。
//!
//! ## キーマップ（`docs/user-demand2.md`・shortcut 修正）
//!
//! プロジェクト全体に関わる shortcut は `Shift + *`（単体キーはどこで押しても
//! 効く）。ブロック履歴 1 件に関する shortcut は単体キーのみ。
//! CLI モード（`x`）中は専用キーマップ（`e`=状態を進める（run→review→done→workflow）・
//! `d`=rm・`x`/`Esc`=終了）に切り替わる（`app/cli_mode.rs`）。
//!
//! | キー | 動作 |
//! |---|---|
//! | `↑` / `↓` / `Enter` | 左SBツリー内のカーソル移動・開閉（Workflow 表示中のみ） |
//! | `s` | Stats へ遷移。再度で Workflow に戻る |
//! | `l` | Log へ遷移（押下時点の選択実行で固定）。再度で Workflow に戻る |
//! | `Space t` | task.md を md-tui に表示 |
//! | `Space m` | message.md を md-tui に表示（旧単体 `m` は廃止） |
//! | `h` | History へ遷移。再度で Workflow に戻る |
//! | `o` | waggle token ファイルを herdr ペインで nvim により開く |
//! | `p` | `pi --session` を herdr ペインで実行 |
//! | Shift+Q | アプリの終了 |
//! | Shift+A | Summary（`.sai/Summary.md`）を herdr ペインで nvim により開く |
//! | Shift+R | リロード（実行状況・色分けを取り込み直す） |
//! | Shift+F | 検索（セッション履歴検索）↔ Workflow |
//! | Shift+B | `.sai/blocks` を herdr ペインで nvim により開く |
//! | Shift+S | `.sai/settings.json` を herdr ペインで nvim により開く |
//! | Shift+H | ヘルプ（全ショートカット一覧）を表示 |
//! | Shift+M | model-tui（pi の使用可能モデル一覧）を表示（Esc で戻る） |
//! | Shift+P | Project へ遷移。再度で Workflow に戻る |
//! | Tab / BackTab | 画面フォーカスの切り替え（sidebar → kanban → workflow を循環） |
//!
//! 実行中メインブロックの完了確定（finalize）は pane 内の `/pi-sai-finalize`
//! または pane を閉じることで行う（TUI 側の Ctrl+D は廃止）。

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind};

use super::{AppState, ViewMode};

// 移設先（sai-link-widgets）から取り込む（カード作成の共通部品・`phone card create` と同一）
#[cfg(test)]
use sai_link_widgets::card_wizard::apply_card_auto_fields;

/// キーイベントを処理する。
pub fn handle_key_event(state: &mut AppState, key: KeyEvent) -> bool {
    if key.kind == KeyEventKind::Release {
        return false;
    }

    // ── モーダル表示中はモーダルにキーを渡す（`x` の CLI モード突入より優先）──
    // 借用競合を避けるため、先にモーダル種別を判定する。
    if matches!(state.modal, super::Modal::Help) {
        state.modal = super::Modal::None;
        return true;
    }
    if matches!(state.modal, super::Modal::CardCreate(_)) {
        return handle_card_create_modal_key(state, key);
    }
    if matches!(state.modal, super::Modal::WaggleSelect { .. }) {
        return handle_waggle_select_modal_key(state, key);
    }

    // ── CLI モード中は専用キーマップ（通常 shortcut を無効化）──
    if state.cli_mode.active {
        return crate::app::cli_mode::handle_cli_mode_key(state, key);
    }

    // ── `x`: CLI モード（`rui-tui`・sidebar 選択を固定して入る）へ入る ──
    if key.code == KeyCode::Char('x') && state.view_mode == ViewMode::Workflow {
        // 選択中の todo カード / 実行リーフを対象として固定する。
        // 対象は選択中アイテムの種別から決める（stale な `selected_exec_id` を
        // 引きずらない。todo カード選択中に `d` で別レーンの実行を削除しないため）。
        let selected = state
            .left_sidebar
            .tree_items
            .get(state.left_sidebar.selected_index)
            .cloned();
        // レーンフォルダ（todo / running / review / done）にカーソルがあるか。
        // ある場合は `e` の対象を「そのレーンの全カード」にする。
        let lane = selected.as_ref().and_then(|item| {
            if matches!(
                item.kind,
                crate::ui::left_sidebar::TreeItemKind::Folder
            ) {
                lane_folder_name(item.id)
            } else {
                None
            }
        });
        let (card_path, card, exec_id) = match selected {
            Some(item) => match item.kind {
                // todo カード: md ファイルが対象。実行 ID は固定しない
                crate::ui::left_sidebar::TreeItemKind::Card => {
                    let p = item.card_path.clone().unwrap_or_default();
                    let c = std::fs::read_to_string(&p)
                        .ok()
                        .and_then(|t| sai_link_engine::kanban::parse_card(&t));
                    (Some(p), c, None)
                }
                // 実行リーフ: item.id が run_id。対応カード md があれば併せて固定
                crate::ui::left_sidebar::TreeItemKind::Leaf => {
                    let c = item
                        .card_path
                        .clone()
                        .and_then(|p| {
                            std::fs::read_to_string(&p)
                                .ok()
                                .and_then(|t| sai_link_engine::kanban::parse_card(&t))
                        });
                    (item.card_path.clone(), c, Some(item.id))
                }
                // フォルダ: 対象外（実行選択も引きずらない）
                crate::ui::left_sidebar::TreeItemKind::Folder => (None, None, None),
            },
            None => (None, None, None),
        };
        state.cli_mode.open_with_selection(card_path, card, exec_id);
        if let Some(lane) = lane {
            state.cli_mode.set_lane_target(lane);
        }
        return true;
    }

    // ── `Tab` / `Shift+Tab`: 画面フォーカスの切り替え（sidebar → kanban → workflow）──
    // スクロール（ホイール・矢印キー）が効くのはフォーカス中の画面だけ。
    // md ビューの見出し一覧（outline）が Tab を使う場面とは重ならないよう、
    // Workflow 表示中（一時ビューなし）に限定する。
    if matches!(key.code, KeyCode::Tab | KeyCode::BackTab)
        && state.view_mode == ViewMode::Workflow
        && state.main_panel.md_view.is_none()
        && state.main_panel.project_dash.is_none()
        && state.main_panel.history.is_none()
    {
        state.focus = if key.code == KeyCode::Tab {
            state.focus.next()
        } else {
            state.focus.prev()
        };
        // Space リーダー待機中なら解除（Tab で別の操作に入ったため）
        state.space_pending = false;
        state.set_message(format!("focus: {}", state.focus.label()));
        return true;
    }

    // ── Stats ビュー（キャッシュ統計グラフ）: ビュー内キーを最優先で処理 ──
    if state.view_mode == ViewMode::Stats && handle_stats_view_key(state, key) {
        return true;
    }

    // ── 検索画面: ビュー内キーを優先処理（処理しないキーは後段へフォールスルー）──
    if state.view_mode == ViewMode::Search && handle_search_view_key(state, key) {
        return true;
    }

    // ── model-tui: 一覧表示中のキーを優先処理 ──
    if state.view_mode == ViewMode::Model && handle_model_view_key(state, key) {
        return true;
    }

    // ── グローバルキー（Shift+* 系は handle_global_shortcuts で処理）──
    match key.code {
        KeyCode::Char('R') => {
            // Shift+R = リロード（実行状況と色分けを作り直す）
            state.reload_data();
            return true;
        }
        KeyCode::Esc => {
            // Space 待機中なら解除のみ
            if state.space_pending {
                state.space_pending = false;
                return true;
            }
            // card / round 区切り線をドラッグ中なら移動を取り消す
            // （ekphos `canvas_cancel_interaction` 相当。開始時点のズレ量へ戻す）
            if let Some(drag) = &state.drag {
                match drag.border {
                    super::DragBorder::Node { exec_id, base } => {
                        state.main_panel.node_offsets.insert(exec_id, base);
                        state.drag = None;
                        return true;
                    }
                    super::DragBorder::Separator { global_round, base } => {
                        state.main_panel.separator_offsets.insert(global_round, base);
                        state.drag = None;
                        return true;
                    }
                    _ => {}
                }
            }
            // ドラッグ選択中なら選択を解除
            if state.selection.is_some() {
                state.selection = None;
                return true;
            }
            // 各ビューから Workflow へ戻る（唯一の戻り手段）
            if !matches!(state.view_mode, ViewMode::Workflow | ViewMode::Search) {
                return_to_workflow(state);
                return true;
            }
        }
        _ => {}
    }

    // ── ショートカット（全パネル共通・仕様書 §3）──
    if handle_global_shortcuts(state, key) {
        return true;
    }

    // ── ビュー固有のキー処理（フォーカスレス: view_mode のみで決定）──
    if state.is_sidebar_enabled() && handle_left_sidebar_key(state, key) {
        return true;
    }
    handle_main_key(state, key)
}

/// ショートカットキー（単体キー: `s` / `l` / `r` / `h` / `o` / `p`、
/// 全体系 `Shift+*`: `Q` / `F` / `H` / `P` / `B` / `S`）を処理する。
///
/// アクティブパネルに関係なく有効。処理した場合は `true` を返す。
fn handle_global_shortcuts(state: &mut AppState, key: KeyEvent) -> bool {
    // Space 待機中に `r`/`t`/`m` 以外が押されたら待機を解除する（通常処理へ進めない）
    if state.space_pending
        && !matches!(
            key.code,
            KeyCode::Char('r') | KeyCode::Char('t') | KeyCode::Char('m') | KeyCode::Char(' ')
        )
    {
        state.space_pending = false;
        return true;
    }
    match key.code {
        // ── プロジェクト全体に関わる shortcut（Shift+*・どこででも効く）──
        // Shift+Q: アプリの終了
        KeyCode::Char('Q') => {
            state.running = false;
            return false;
        }
        // Shift+F: 検索画面へ遷移（Search 表示中なら Workflow に戻るトグル）
        KeyCode::Char('F') => {
            state.view_mode = ViewMode::Search;
            return true;
        }
        // Shift+H: ヘルプ（全ショートカット一覧）。どの画面からでも開ける
        KeyCode::Char('H') => {
            state.modal = super::Modal::Help;
            return true;
        }
        // Shift+M: model-tui（pi の使用可能モデル一覧）へ遷移。Esc で戻る
        KeyCode::Char('M') => {
            state.open_model_view();
            return true;
        }
        // ── メイン画面遷移系（2 度押しで Workflow に戻るトグル）──
        // `l`: Log（押下時点の sidebar 選択ブロックで固定。表示と実ファイルを一致させる）
        KeyCode::Char('l') => {
            if state.view_mode == ViewMode::Workflow {
                // 押した瞬間に sidebar で選択されていたブロックの詳細を取得し、以後固定する
                state.log_state.detail = sidebar_selected_exec_id(state)
                    .and_then(|id| state.build_block_detail(id));
                state.log_state.scroll = 0;
                state.view_mode = ViewMode::Log;
            }
            true
        }
        // `Space` リーダー: 次の `t`/`m` で md-tui を開く（`Esc` で解除）。
        // Workflow 以外（Md モードの見出し折りたたみ等）では消費せず後段へ流す。
        KeyCode::Char(' ') if state.view_mode == ViewMode::Workflow => {
            state.space_pending = true;
            state.set_message("Space: t=task m=message");
            true
        }
        // md-tui: `Space t` = task.md / `Space m` = message.md（単体 `m` は廃止し Space 経由に統一）
        // いずれも押下時点の sidebar 選択が対象（表示と実ファイルを一致させる）。
        KeyCode::Char('t') if state.space_pending => {
            state.space_pending = false;
            if state.view_mode == ViewMode::Workflow {
                // 実行リーフ → run_id で task.md を開く。todo カード（DB行なし・
                // `sidebar_selected_exec_id` は `None`）→ `card_path` を直接開く（A5）。
                // todo 時点で task.md は必ず存在するため全 status で使用可能。
                if let Some(id) = sidebar_selected_exec_id(state) {
                    state.load_md_view(id, crate::ui::MdKind::Task);
                } else if let Some(path) = sidebar_selected_card_path(state) {
                    state.load_md_file_view(&path, crate::ui::MdKind::Task);
                } else {
                    state.set_message("task: 実行が選択されていません");
                }
            }
            true
        }
        // 単体 `m` は廃止（Space 経由に統一）。Space 待機中のみ message を開く。
        KeyCode::Char('m') => {
            if state.space_pending {
                state.space_pending = false;
                if state.view_mode == ViewMode::Workflow {
                    match sidebar_selected_exec_id(state) {
                        Some(id) => state.load_md_view(id, crate::ui::MdKind::Message),
                        None => state.set_message("message: 実行が選択されていません"),
                    }
                }
            } else if state.view_mode == ViewMode::Workflow {
                state.set_message("message: Space m で開いてください");
            }
            true
        }
        // `h`: History（押下時点の sidebar 選択ブロックで固定。表示と実ファイルを一致させる）
        KeyCode::Char('h') => {
            if state.view_mode == ViewMode::Workflow {
                match sidebar_selected_exec_id(state) {
                    Some(id) => {
                        state.load_session_output_to_main(id);
                        state.view_mode = ViewMode::History;
                    }
                    None => {
                        state.set_message("History: 実行が選択されていません");
                    }
                }
            }
            true
        }
        // `P`: Project（プロジェクト概要。`e` で編集可）
        KeyCode::Char('P') => {
            if state.view_mode == ViewMode::Workflow {
                state.view_mode = ViewMode::Project;
                state.load_project_view();
            }
            true
        }
        // `s`: Stats（押下時点の sidebar 選択ブロックが対象。表示と実ファイルを一致させる）
        KeyCode::Char('s') => {
            if state.view_mode == ViewMode::Workflow {
                state.view_mode = ViewMode::Stats;
                state.load_cache_view(sidebar_selected_exec_id(state));
            }
            true
        }

        // ── pane 作成系（herdr ペインでコマンド実行）──
        // いずれも押下時点の sidebar 選択ブロックが対象（表示と実ファイルを一致させる）。
        // `w`: waggle token 選択（md のみ・Workflow 表示中）。
        // 選択実行の md トークンを一覧表示し、Enter で md-tui 表示する。
        KeyCode::Char('w') => {
            if state.view_mode == ViewMode::Workflow {
                open_waggle_select(state);
            }
            true
        }
        // `p`: セッション再開（押下時点の sidebar 選択ブロックが対象）。
        // runner=gl の実行では sai-gemini-tui（`gl`）を開く。
        KeyCode::Char('p') => {
            let exec_id = sidebar_selected_exec_id(state);
            let is_gl = exec_id
                .and_then(|id| state.store.get_execution_by_id(id).ok())
                .map(|e| e.runner.trim().eq_ignore_ascii_case("gl"))
                .unwrap_or(false);
            if is_gl {
                super::external::open_gl_session(state);
            } else {
                super::external::resume_session(state, exec_id);
            }
            true
        }
        // `c`: sidebar の選択中カード・実行リーフの対応カードを nvim で開く
        // （押下時点の sidebar 選択が対象。表示と実ファイルを一致させる）
        KeyCode::Char('c') => {
            let selected = state
                .left_sidebar
                .tree_items
                .get(state.left_sidebar.selected_index);
            // 実行リーフは `card_path` が未解決でも DB・todo 走査で解決する。
            let card_path = selected.and_then(|t| t.card_path.clone()).or_else(|| {
                let exec_id = sidebar_selected_exec_id(state)?;
                let exec = state.store.get_execution_by_id(exec_id).ok()?;
                super::kanban::card_path_for_execution(exec.kanban_card_path.as_deref(), exec_id)
            });
            super::external::open_card(state, card_path);
            true
        }
        // `d`: 通常画面では何もしない（移動系は `x` モードの `r` / `w` に統一）。
        KeyCode::Char('d') => {
            state.set_message("移動は `x` モードの `r` / `w` を使用してください");
            true
        }
        // `B`: .sai/blocks ディレクトリを nvim で開く
        KeyCode::Char('B') => {
            super::external::open_blocks_dir(state);
            true
        }
        // `S`: .sai/settings.json を nvim で開く
        KeyCode::Char('S') => {
            super::external::open_settings(state);
            true
        }
        // `A`: Summary（.sai/Summary.md）を nvim で開く（どの画面からでも開ける）
        KeyCode::Char('A') => {
            super::external::open_summary(state);
            true
        }
        // `C`: カード新規作成モーダルを開く
        KeyCode::Char('C') => {
            open_card_create_modal(state);
            true
        }
        _ => false,
    }
}

/// Workflow 表示へ戻す（各モードのトグル後半）。
///
/// 左SBカーソル移動を有効に戻し、メイン画面の一時ビューを
/// 破棄する。DB データの再読み込みは位置保持する。
fn return_to_workflow(state: &mut AppState) {
    state.view_mode = ViewMode::Workflow;
    state.main_panel.md_view = None;
    state.main_panel.project_dash = None;
    state.main_panel.history = None;
    state.load_data();
}

// ============================================================================
// 左サイドバー（Workflow 表示中のみカーソル操作可）
// ============================================================================

fn handle_left_sidebar_key(state: &mut AppState, key: KeyEvent) -> bool {
    match key.code {
        KeyCode::Char('k') => {
            move_sidebar_cursor(state, -1);
            true
        }
        KeyCode::Char('j') => {
            move_sidebar_cursor(state, 1);
            true
        }
        // Enter: フォルダは折りたたみ/展開。リーフでは何もしない
        KeyCode::Enter => {
            let Some(item) = state
                .left_sidebar
                .tree_items
                .get(state.left_sidebar.selected_index)
                .cloned()
            else {
                return false;
            };
            if item.is_folder() {
                state.toggle_left_folder(state.left_sidebar.selected_index);
            }
            true
        }
        _ => false,
    }
}

/// sidebar のフォルダ ID がレーン（todo / running / review / done）ならその名前を返す。
///
/// R / r フォルダは `None`（`e` の対象にならない）。
fn lane_folder_name(id: i64) -> Option<&'static str> {
    use sai_link_widgets::left_tree as tree;
    match id {
        tree::TODO_FOLDER_ID => Some("todo"),
        tree::RUNNING_FOLDER_ID => Some("running"),
        tree::REVIEW_FOLDER_ID => Some("review"),
        tree::DONE_FOLDER_ID => Some("done"),
        _ => None,
    }
}

/// Kanban レーンの縦スクロール位置を変更する（4レーン同時・上限は最長レーン基準）。
///
/// フォーカスが kanban のときの `↑`/`↓`・ホイールで呼ぶ。
pub(crate) fn scroll_kanban(state: &mut AppState, delta: i32) {
    let counts = crate::app::kanban::lane_counts(&state.kanban_entries);
    let max = counts.iter().max().copied().unwrap_or(0).saturating_sub(1);
    let next = (state.kanban_scroll as i32 + delta).clamp(0, max as i32);
    state.kanban_scroll = next as usize;
}

/// sidebar カーソルを移動し、選択を追従させる（`j/k`・ホイール共通）。
pub(crate) fn move_sidebar_cursor(state: &mut AppState, delta: i32) {
    let len = state.left_sidebar.tree_items.len();
    if len == 0 {
        return;
    }
    let cur = state.left_sidebar.selected_index.min(len - 1) as i32;
    let next = (cur + delta).clamp(0, len as i32 - 1) as usize;
    state.left_sidebar.selected_index = next;
    // 表示域外に出たら追従スクロールする
    let area_h = state.layout.left_sidebar.height.saturating_sub(1) as usize;
    if next < state.left_sidebar.scroll_offset {
        state.left_sidebar.scroll_offset = next;
    } else if area_h > 0 && next >= state.left_sidebar.scroll_offset + area_h {
        state.left_sidebar.scroll_offset = next + 1 - area_h;
    }
    select_left_item(state);
}

/// sidebar で選択中の実行 ID を返す（表示と実ファイルの一致用）。
///
/// 左SBカーソルのアイテム種別から直接決める。`selected_exec_id` の保持値には
/// 頼らない（フォルダ素通り・リロード復元失敗・マウス誤判定による stale を防ぐ）。
/// 実行リーフ（`Leaf`）のみ `Some(実行ID)`、フォルダ・todoカードは `None`。
///
/// **用語**: `run_id`（DB `execution_history.id`）と `exec_id` は同一。
/// todo カードの frontmatter `run_id:` / `outputs/todo/<run_id>/` は予約 run_id であり、
/// 実行開始前は DB `status='todo'` 行のみ（`selected_exec_id` なし）のため
/// `Space t` は `sidebar_selected_card_path` 側で開く（A5）。
pub(crate) fn sidebar_selected_exec_id(state: &AppState) -> Option<i64> {
    let item = state
        .left_sidebar
        .tree_items
        .get(state.left_sidebar.selected_index)?;
    if item.is_folder() {
        return None;
    }
    if matches!(
        item.kind,
        crate::ui::left_sidebar::TreeItemKind::Card
    ) {
        return None;
    }
    Some(item.id)
}

/// sidebar で選択中の todo カード md パスを返す（`Space t` の todo 対応・A5）。
///
/// `TreeItemKind::Card` のみ `Some(パス)`。todo 時点で task.md は必ず存在する。
pub(crate) fn sidebar_selected_card_path(state: &AppState) -> Option<std::path::PathBuf> {
    let item = state
        .left_sidebar
        .tree_items
        .get(state.left_sidebar.selected_index)?;
    if !matches!(
        item.kind,
        crate::ui::left_sidebar::TreeItemKind::Card
    ) {
        return None;
    }
    item.card_path.clone()
}

/// 左SB で選択した実行を確定する。
///
/// - 実行リーフ：`selected_exec_id` を更新（ステータスバー右端の表示用。
///   `l/s/o/h/r/p` 等の対象決定には使わない。各 shortcut は押下時点の sidebar
///   カーソル（`sidebar_selected_exec_id`）から直接決める）
/// - カード（todo）：実行 ID を持たないため `selected_exec_id` は触らない
/// - メイン画面（DAG）の選択を更新し、自動追従に戻す
fn select_left_item(state: &mut AppState) {
    let Some(item) = state
        .left_sidebar
        .tree_items
        .get(state.left_sidebar.selected_index)
        .cloned()
    else {
        return;
    };
    // フォルダは選択対象にならない（折りたたみ/展開のみ）
    if item.is_folder() {
        return;
    }
    // カード（todo）は実行 ID を持たない（負値ダミー ID）ため選択対象外。
    // ハイライト用にパス側だけ寄せ、実行系の選択は残す。
    // 注意：実行リーフ（running/review/done・R/r 配下）も `c` 用に
    // `card_path` を持つため、種別で判定する（`card_path` の有無では判定しない）。
    if matches!(
        item.kind,
        crate::ui::left_sidebar::TreeItemKind::Card
    ) {
        state.main_panel.viewport.manual_scroll = false;
        return;
    }
    state.selected_exec_id = Some(item.id);
    // マウスパン・ズーム後の手動スクロールを解除し、次の描画で
    // 選択ノードがビューポート中央へ移動するようにする
    state.main_panel.viewport.manual_scroll = false;
    state.main_panel.selected_block_id = Some(item.id);
}

// ============================================================================
// メイン画面
// ============================================================================

fn handle_main_key(state: &mut AppState, key: KeyEvent) -> bool {
    // Log 表示中: スクロール
    if state.view_mode == ViewMode::Log {
        return match key.code {
            KeyCode::Up | KeyCode::Char('k') => {
                log_scroll(state, -1);
                true
            }
            KeyCode::Down | KeyCode::Char('j') => {
                log_scroll(state, 1);
                true
            }
            KeyCode::PageUp => {
                log_scroll(state, -20);
                true
            }
            KeyCode::PageDown => {
                log_scroll(state, 20);
                true
            }
            KeyCode::Esc => {
                return_to_workflow(state);
                true
            }
            _ => false,
        };
    }

    // History / Project モード: マークダウン表示・ダッシュボードのスクロール・クローズ
    if state.main_panel.md_view.is_some()
        || state.main_panel.project_dash.is_some()
        || state.main_panel.history.is_some()
    {
        // History 画面（ratatui ネイティブ描画）では `j/k`・`↑/↓`・PgUp/PgDn を
        // Timeline の内部スクロールに割当てる（セッション出力は表示しない）
        if state.view_mode == ViewMode::History && state.main_panel.history.is_some() {
            return match key.code {
                KeyCode::Up | KeyCode::Char('k') => {
                    crate::ui::history_view::timeline_scroll(
                        state.main_panel.history.as_mut().unwrap(),
                        -1,
                    );
                    true
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    crate::ui::history_view::timeline_scroll(
                        state.main_panel.history.as_mut().unwrap(),
                        1,
                    );
                    true
                }
                KeyCode::PageUp => {
                    crate::ui::history_view::timeline_scroll(
                        state.main_panel.history.as_mut().unwrap(),
                        -5,
                    );
                    true
                }
                KeyCode::PageDown => {
                    crate::ui::history_view::timeline_scroll(
                        state.main_panel.history.as_mut().unwrap(),
                        5,
                    );
                    true
                }
                KeyCode::Esc => {
                    return_to_workflow(state);
                    true
                }
                _ => false,
            };
        }
        // Project ダッシュボードでは `j/k`・`↑/↓` を Recent テーブルの
        // 内部スクロールに割当てる（ページ全体は PgUp/PgDn・ホイール）
        if state.view_mode == ViewMode::Project
            && matches!(
                key.code,
                KeyCode::Up | KeyCode::Down | KeyCode::Char('k') | KeyCode::Char('j')
            )
            && state
                .main_panel
                .project_dash
                .as_ref()
                .map(|d| !d.recent.is_empty())
                .unwrap_or(false)
        {
            let delta = matches!(key.code, KeyCode::Down | KeyCode::Char('j')) as i32 * 2 - 1;
            sai_link_widgets::project_dashboard::recent_scroll(
                state.main_panel.project_dash.as_mut().unwrap(),
                delta,
            );
            return true;
        }
        // md viewer: 見出し折りたたみ（z・Space）・outline パネル（o・Tab・↑↓/jk・Enter）
        if state.view_mode == ViewMode::Md && state.main_panel.md_view.is_some() {
            let md = state.main_panel.md_view.as_ref().unwrap();
            let has_headings = !md.headings.is_empty();
            let outline_focused = md.outline_focused;
            return match key.code {
                KeyCode::Char('o') if has_headings => {
                    let md = state.main_panel.md_view.as_mut().unwrap();
                    md.outline_visible = !md.outline_visible;
                    if !md.outline_visible {
                        md.outline_focused = false;
                    }
                    true
                }
                KeyCode::Tab if has_headings => {
                    let md = state.main_panel.md_view.as_mut().unwrap();
                    if md.outline_visible {
                        md.outline_focused = !md.outline_focused;
                    }
                    true
                }
                KeyCode::Char('z') | KeyCode::Char(' ') if has_headings && !outline_focused => {
                    state
                        .main_panel
                        .md_view
                        .as_mut()
                        .unwrap()
                        .toggle_fold_at_cursor();
                    true
                }
                KeyCode::Up | KeyCode::Char('k') if outline_focused => {
                    state.main_panel.md_view.as_mut().unwrap().outline_move(-1);
                    true
                }
                KeyCode::Down | KeyCode::Char('j') if outline_focused => {
                    state.main_panel.md_view.as_mut().unwrap().outline_move(1);
                    true
                }
                KeyCode::Enter if outline_focused => {
                    state
                        .main_panel
                        .md_view
                        .as_mut()
                        .unwrap()
                        .jump_to_outline_selection();
                    true
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    super::md_scroll(state, -1);
                    true
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    super::md_scroll(state, 1);
                    true
                }
                KeyCode::PageUp => {
                    super::md_scroll(state, -20);
                    true
                }
                KeyCode::PageDown => {
                    super::md_scroll(state, 20);
                    true
                }
                KeyCode::Esc => {
                    return_to_workflow(state);
                    true
                }
                _ => false,
            };
        }
        return match key.code {
            KeyCode::Up | KeyCode::Char('k') => {
                super::md_scroll(state, -1);
                true
            }
            KeyCode::Down | KeyCode::Char('j') => {
                super::md_scroll(state, 1);
                true
            }
            KeyCode::PageUp => {
                super::md_scroll(state, -20);
                true
            }
            KeyCode::PageDown => {
                super::md_scroll(state, 20);
                true
            }
            KeyCode::Esc => {
                return_to_workflow(state);
                true
            }
            _ => false,
        };
    }

    // Workflow モード: フォーカス中の画面へスクロールを振り分ける
    // （スクロールが効くのはフォーカス中の画面のみ）
    match key.code {
        KeyCode::Up | KeyCode::Down => {
            let delta = if key.code == KeyCode::Up { -1 } else { 1 };
            match state.focus {
                // `j/k` は常に sidebar。`↑↓` はフォーカス中画面をスクロールする
                crate::app::ScreenFocus::Sidebar => {
                    move_sidebar_cursor(state, delta);
                    true
                }
                // kanban の card に選択カーソルは無い（レーンの見かけ位置だけ動かす）
                crate::app::ScreenFocus::Kanban => {
                    scroll_kanban(state, delta);
                    true
                }
                crate::app::ScreenFocus::Workflow => {
                    state.main_panel.viewport.scroll_vertical(delta * 3);
                    true
                }
            }
        }
        KeyCode::Left | KeyCode::Right => {
            // 横スクロールは Workflow のみ（sidebar / kanban には対象が無い）
            if state.focus == crate::app::ScreenFocus::Workflow {
                let dx = if key.code == KeyCode::Left { -3 } else { 3 };
                state.main_panel.viewport.scroll_horizontal(dx);
                true
            } else {
                false
            }
        }
        // `+` / `-` キーでもズーム（アンカーは Workflow 領域の中央）。
        // description はノード矩形の残り行数で再計算されるため自動追従する。
        KeyCode::Char('+') | KeyCode::Char('=') => {
            let a = state.layout.workflow;
            super::zoom_main_panel(state, a, a.x + a.width / 2, a.y + a.height / 2, 1.25);
            true
        }
        KeyCode::Char('-') => {
            let a = state.layout.workflow;
            super::zoom_main_panel(state, a, a.x + a.width / 2, a.y + a.height / 2, 0.8);
            true
        }
        _ => false,
    }
}

/// Log 画面のスクロール位置を変更する（クランプは描画時に行われる）。
fn log_scroll(state: &mut AppState, delta: i64) {
    state.log_state.scroll = (state.log_state.scroll as i64 + delta).max(0) as usize;
}

// ============================================================================
// 検索画面（セッション履歴検索。Shift+F で遷移）
// ============================================================================

/// 検索画面中のキー処理。
///
/// | キー | 動作 |
/// |---|---|
/// | `↑` / `↓` / `k` / `j` | 編集中でなければフォーム項目 → 結果リストの移動 |
/// | `Tab` / `Shift+Tab` | フォーム項目間の移動（循環） |
/// | `Enter` | 編集中なら確定、編集中でなければフォーム上で編集開始 / 結果リスト上で検索実行 |
/// | 文字入力 / Backspace | 編集中のみ（`edit_buf` を更新） |
/// | `Esc` | 編集中ならキャンセル、編集中でなければ Workflow へ戻る |
///
/// 未処理のキーは `false` を返し、
/// 既存のグローバル処理へ委ねる（終了・リロード動作は現行維持）。
fn handle_search_view_key(state: &mut AppState, key: KeyEvent) -> bool {
    use crate::ui::search::FORM_ITEMS;
    let search = &mut state.search;

    // ── 編集中: 全キーを吸収する ──
    if search.editing {
        match key.code {
            KeyCode::Enter => {
                search.commit_edit();
                return true;
            }
            KeyCode::Esc => {
                search.cancel_edit();
                return true;
            }
            KeyCode::Backspace => {
                search.edit_buf.pop();
                return true;
            }
            KeyCode::Char(c) => {
                search.edit_buf.push(c);
                return true;
            }
            _ => return true, // 編集中はその他のキーも無視して吸収
        }
    }

    // ── 非編集中 ──
    // focus: 0..FORM_ITEMS = フォーム項目、FORM_ITEMS = 結果リスト
    match key.code {
        // フォーム項目移動（Tab で循環。結果リストからは先頭項目へ戻る）
        KeyCode::Tab => {
            search.focus = (search.focus + 1) % (FORM_ITEMS + 1);
            true
        }
        KeyCode::BackTab => {
            search.focus = (search.focus + FORM_ITEMS) % (FORM_ITEMS + 1);
            true
        }
        // ↑/↓: フォーム項目 ⇄ 結果リストの移動。結果リスト上ではカーソル移動
        KeyCode::Up | KeyCode::Char('k') => {
            if search.focus == FORM_ITEMS {
                if search.selected > 0 {
                    search.selected -= 1;
                } else {
                    search.focus = FORM_ITEMS - 1; // フォームへ戻る
                }
            } else if search.focus > 0 {
                search.focus -= 1;
            }
            true
        }
        KeyCode::Down | KeyCode::Char('j') => {
            if search.focus == FORM_ITEMS {
                if !search.hits.is_empty() {
                    search.selected = (search.selected + 1).min(search.hits.len() - 1);
                }
            } else if search.focus < FORM_ITEMS - 1 {
                search.focus += 1;
            } else {
                // フォーム末尾から結果リスト（検索実行位置）へ降りる
                search.focus = FORM_ITEMS;
            }
            true
        }
        // 検索実行 / 編集開始
        KeyCode::Enter => {
            if search.focus == FORM_ITEMS {
                state.run_session_search();
            } else {
                search.begin_edit();
            }
            true
        }
        KeyCode::Esc => {
            // Workflow へ戻る（Shift+F トグルと同等）
            return_to_workflow(state);
            true
        }
        _ => false, // 未処理はグローバルキー等へフォールスルー
    }
}

// ============================================================================
// model-tui（`Shift+M`・pi の使用可能モデル一覧）
// ============================================================================

/// model-tui 表示中のキー処理。
///
/// | キー | 動作 |
/// |---|---|
/// | 文字 | フィルタ文字列に追加（大文字小文字無視の部分列一致） |
/// | `Backspace` | フィルタ末尾を削除 |
/// | `↑` / `↓` | カーソル移動（端で折り返し） |
/// | `PageUp` / `PageDown` | カーソルを 10 件移動 |
/// | `Esc` | Workflow へ戻る |
///
/// `Enter` による決定は行わない（参照専用）。未処理の `Q`（終了）は `false` を
/// 返してグローバルショートカットへ委ねる。
fn handle_model_view_key(state: &mut AppState, key: KeyEvent) -> bool {
    use crossterm::event::KeyModifiers;
    match key.code {
        KeyCode::Esc => {
            return_to_workflow(state);
            true
        }
        KeyCode::Up => {
            state.model_view.cursor_up();
            true
        }
        KeyCode::Down => {
            state.model_view.cursor_down();
            true
        }
        KeyCode::PageUp => {
            for _ in 0..10 {
                state.model_view.cursor_up();
            }
            true
        }
        KeyCode::PageDown => {
            for _ in 0..10 {
                state.model_view.cursor_down();
            }
            true
        }
        KeyCode::Backspace => {
            state.model_view.filter.pop();
            state.model_view.cursor = 0;
            true
        }
        KeyCode::Char(c)
            if !key.modifiers.contains(KeyModifiers::CONTROL)
                && !key.modifiers.contains(KeyModifiers::ALT) =>
        {
            // 終了キー `Shift+Q` だけはグローバルへ流す。
            if c == 'Q' {
                return false;
            }
            state.model_view.filter.push(c);
            state.model_view.cursor = 0;
            true
        }
        _ => true,
    }
}

// ============================================================================
// Stats ビュー（キャッシュ統計グラフ・sai-cache-tui-rs 移植）
// ============================================================================

/// Stats ビュー中のキー処理。
///
/// | キー | 動作 |
/// |---|---|
/// | `1` / `2` / `3` / `4` | チャート表示モード切替 |
/// | `s` / `Esc` | Workflow へ戻る（トグル） |
/// | `↑` / `↓` / `k` / `j` | Per-message breakdown のスクロール（1 行） |
/// | `PgUp` / `PgDn` | Per-message breakdown のスクロール（10 行） |
/// | `Home` / `End` | Per-message breakdown の先頭 / 末尾へ |
///
/// 未処理のキーは `false` を返し、グローバルキー（Shift+* 等）へ委ねる。
fn handle_stats_view_key(state: &mut AppState, key: KeyEvent) -> bool {
    use crate::ui::cache_view::CacheChartMode;
    match key.code {
        KeyCode::Char('1') => {
            state.cache_view.chart_mode = CacheChartMode::PerTurn;
            true
        }
        KeyCode::Char('2') => {
            state.cache_view.chart_mode = CacheChartMode::CumulativePercent;
            true
        }
        KeyCode::Char('3') => {
            state.cache_view.chart_mode = CacheChartMode::CumulativeTotal;
            true
        }
        KeyCode::Char('4') => {
            state.cache_view.chart_mode = CacheChartMode::StatsTable;
            true
        }
        KeyCode::Esc => {
            // Stats から Workflow へ戻る（`s` 再押しでは戻らない）
            return_to_workflow(state);
            true
        }
        KeyCode::Up | KeyCode::Char('k') => {
            state.cache_view.per_message_scroll =
                state.cache_view.per_message_scroll.saturating_sub(1);
            true
        }
        KeyCode::Down | KeyCode::Char('j') => {
            state.cache_view.per_message_scroll =
                state.cache_view.per_message_scroll.saturating_add(1);
            true
        }
        KeyCode::PageUp => {
            state.cache_view.per_message_scroll =
                state.cache_view.per_message_scroll.saturating_sub(10);
            true
        }
        KeyCode::PageDown => {
            state.cache_view.per_message_scroll =
                state.cache_view.per_message_scroll.saturating_add(10);
            true
        }
        KeyCode::Home => {
            state.cache_view.per_message_scroll = 0;
            true
        }
        KeyCode::End => {
            // 描画時に表示行数へクランプされるため最大値を指定する
            state.cache_view.per_message_scroll = usize::MAX;
            true
        }
        _ => false,
    }
}

// ============================================================================
// カード新規作成モーダル（`Shift+C`）
// ============================================================================

/// `Shift+C`: カード新規作成モーダルを開く。
///
/// **Done レーン**の実行（DB `review_done=true`・未移動。`kanban_entries` の
/// `lane == Done`）のみを選択可能にする。Workflow 移動済み（`workflow_moved=true`）
/// の実行は対象外。複数選択 → 結合 → `outputs/todo/<run_id>/task.md` に作成 →
/// nvim で開く。
/// Done が0件のとき・何も選ばず Enter のときは、連結なしの空カード
/// （frontmatter のみ・本文空・`origin_run_id` なし）を作る。
/// 1から todo を書きたいときに使う。
pub(crate) fn open_card_create_modal(state: &mut AppState) {
    super::card_wizard::open_card_wizard(state);
}

/// カード新規作成モーダル内のキー処理。
///
/// `↑/↓` で移動・`Space` で選択切替・`Enter` で確定・`Esc` で閉じる。
/// 確定時は選択した done 実行の review 本文を結合してカード作成する。
/// 何も選ばず Enter のときは連結なしの空カードを作る。
/// 処理した場合は `true` を返す（借用競合を避けるため modal を取り出して扱う）。
fn handle_card_create_modal_key(state: &mut AppState, key: KeyEvent) -> bool {
    super::card_wizard::handle_card_wizard_key(state, key)
}

/// `w`: 選択実行の md waggle トークンを開く（0/1/複数で分岐）。
///
/// - 0 件: メッセージのみ
/// - 1 件: 直接 md-tui 表示
/// - 2 件以上: 選択モーダル
fn open_waggle_select(state: &mut AppState) {
    let Some(exec_id) = sidebar_selected_exec_id(state) else {
        state.set_message("waggle: 実行が選択されていません");
        return;
    };
    let choices = state.waggle_choices_for(exec_id);
    match choices.len() {
        0 => state.set_message(format!("waggle: ID{} に md トークンがありません", exec_id)),
        1 => {
            let token = choices[0].token.clone();
            state.load_waggle_token_view(exec_id, &token);
        }
        _ => {
            state.modal = super::Modal::WaggleSelect { choices, cursor: 0 };
        }
    }
}

/// waggle token 選択モーダル内のキー処理。
///
/// `↑/↓` で移動・`Enter` で md-tui 表示・`Esc` で閉じる。
fn handle_waggle_select_modal_key(state: &mut AppState, key: KeyEvent) -> bool {
    let super::Modal::WaggleSelect {
        choices,
        mut cursor,
    } = std::mem::replace(&mut state.modal, super::Modal::None)
    else {
        return true;
    };
    match key.code {
        KeyCode::Up | KeyCode::Char('k') => {
            if cursor > 0 {
                cursor = cursor.saturating_sub(1);
            }
            state.modal = super::Modal::WaggleSelect { choices, cursor };
        }
        KeyCode::Down | KeyCode::Char('j') => {
            if cursor + 1 < choices.len() {
                cursor += 1;
            }
            state.modal = super::Modal::WaggleSelect { choices, cursor };
        }
        KeyCode::Enter => {
            if let (Some(exec_id), Some(choice)) =
                (sidebar_selected_exec_id(state), choices.get(cursor))
            {
                let token = choice.token.clone();
                state.load_waggle_token_view(exec_id, &token);
            }
        }
        KeyCode::Esc => {}
        _ => {
            state.modal = super::Modal::WaggleSelect { choices, cursor };
        }
    }
    true
}

// ============================================================================
// テスト（検索画面）
// ============================================================================

#[cfg(test)]
mod search_tests {
    use super::*;
    use crossterm::event::KeyModifiers;
    use sai_link_engine::db::SaiStore;
    use std::sync::Arc;

    /// テスト用の AppState（in-memory DB）を生成する。
    fn test_state() -> AppState {
        let store = Arc::new(SaiStore::open_in_memory().unwrap());
        AppState::new(store)
    }

    /// 通常の Char キーイベントを生成する。
    fn key(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
    }

    fn ctrl(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
    }

    fn tab_key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    /// `Tab` で sidebar → kanban → workflow と循環し、`BackTab`(Shift+Tab) で逆順になる。
    #[test]
    fn tab_cycles_screen_focus() {
        let mut state = test_state();
        state.view_mode = ViewMode::Workflow;
        // 既定のフォーカスは Workflow（従来の操作感を維持）
        assert_eq!(state.focus, crate::app::ScreenFocus::Workflow);

        assert!(handle_key_event(&mut state, tab_key(KeyCode::Tab)));
        assert_eq!(state.focus, crate::app::ScreenFocus::Sidebar);
        assert!(handle_key_event(&mut state, tab_key(KeyCode::Tab)));
        assert_eq!(state.focus, crate::app::ScreenFocus::Kanban);
        assert!(handle_key_event(&mut state, tab_key(KeyCode::Tab)));
        assert_eq!(state.focus, crate::app::ScreenFocus::Workflow);
        assert!(handle_key_event(&mut state, tab_key(KeyCode::Tab)));
        assert_eq!(
            state.focus,
            crate::app::ScreenFocus::Sidebar,
            "末尾から先頭へ循環する"
        );

        assert!(handle_key_event(&mut state, tab_key(KeyCode::BackTab)));
        assert_eq!(
            state.focus,
            crate::app::ScreenFocus::Workflow,
            "BackTab は逆順で循環する"
        );
    }

    /// md ビュー表示中（見出し一覧が Tab を使う場面）では切り替えない。
    #[test]
    fn tab_does_not_steal_md_outline_key() {
        let mut state = test_state();
        state.view_mode = ViewMode::Md;
        state.main_panel.md_view = Some(crate::ui::main_panel::MarkdownView::from_markdown(
            "# 見出し\n\n本文\n",
            &sai_link_widgets::theme::AppTheme::dark(),
            "task.md".to_string(),
            String::new(),
        ));
        let before = state.focus;
        assert!(handle_key_event(&mut state, tab_key(KeyCode::Tab)));
        assert_eq!(state.focus, before, "md ビュー中はフォーカスを奪わない");
    }

    /// Kanban スクロールは最長レーンまで（1 枚は常に見える）。
    #[test]
    fn kanban_scroll_is_clamped_by_lane_length() {
        use sai_link_engine::kanban::KanbanLane;
        use crate::app::kanban::CardEntry;
        let mut state = test_state();
        // todo 3 枚
        state.kanban_entries = (0..3)
            .map(|i| CardEntry {
                title: format!("todo/{}", i),
                date: String::new(),
                description: String::new(),
                lane: KanbanLane::Todo,
                card_path: None,
                exec_id: None,
                runner: String::new(),
                executer: String::new(),
            })
            .collect();
        scroll_kanban(&mut state, 1);
        scroll_kanban(&mut state, 1);
        scroll_kanban(&mut state, 1);
        scroll_kanban(&mut state, 1);
        assert_eq!(state.kanban_scroll, 2, "上限は枚数 - 1（下端まで）");
        scroll_kanban(&mut state, -5);
        assert_eq!(state.kanban_scroll, 0, "下限は 0");
        // カードが 0 枚ならスクロールしない
        state.kanban_entries.clear();
        scroll_kanban(&mut state, 3);
        assert_eq!(state.kanban_scroll, 0);
    }

    /// `j/k` は常に sidebar カーソル（フォーカスが workflow でも変化しない）。
    #[test]
    fn jk_always_moves_sidebar_cursor() {
        let mut state = test_state();
        state.view_mode = ViewMode::Workflow;
        state.focus = crate::app::ScreenFocus::Workflow;
        // レーンフォルダ + カード相当の最低限のツリー
        state.left_sidebar.tree_items = vec![
            crate::ui::left_sidebar::TreeItem {
                id: 1,
                name: "todo".into(),
                depth: 0,
                expanded: false,
                is_last_child: false,
                kind: crate::ui::left_sidebar::TreeItemKind::Folder,
                ancestor_last: Vec::new(),
                card_path: None,
            },
            crate::ui::left_sidebar::TreeItem {
                id: 2,
                name: "ID1/b".into(),
                depth: 0,
                expanded: false,
                is_last_child: true,
                kind: crate::ui::left_sidebar::TreeItemKind::Leaf,
                ancestor_last: Vec::new(),
                card_path: None,
            },
        ];
        state.left_sidebar.selected_index = 0;
        assert!(handle_key_event(&mut state, key('j')));
        assert_eq!(state.left_sidebar.selected_index, 1, "`j` で sidebar が動く");
        assert!(handle_key_event(&mut state, key('k')));
        assert_eq!(state.left_sidebar.selected_index, 0);
    }

    #[test]
    fn shift_f_toggles_search_view() {
        let mut state = test_state();
        // Shift+F で Search 遷移
        assert!(handle_key_event(&mut state, key('F')));
        assert_eq!(state.view_mode, ViewMode::Search);
        assert!(!state.is_sidebar_enabled());
        // Shift+F 再押しでは Search のまま（戻りは Esc）
        assert!(handle_key_event(&mut state, key('F')));
        assert_eq!(state.view_mode, ViewMode::Search);
        assert!(handle_key_event(&mut state, KeyCode::Esc.into()));
        assert_eq!(state.view_mode, ViewMode::Workflow);
        assert!(state.is_sidebar_enabled());
    }

    #[test]
    fn search_form_navigation_and_edit() {
        let mut state = test_state();
        state.view_mode = ViewMode::Search;
        state.search.focus = 0;

        // Down で次の項目へ
        assert!(handle_key_event(&mut state, key('j')));
        assert_eq!(state.search.focus, 1);

        // Enter で編集開始 → 文字入力 → 確定で form へ反映
        assert!(handle_key_event(&mut state, KeyCode::Enter.into()));
        assert!(state.search.editing);
        assert!(handle_key_event(&mut state, key('l')));
        assert!(handle_key_event(&mut state, key('l')));
        assert!(handle_key_event(&mut state, key('m')));
        assert!(handle_key_event(&mut state, KeyCode::Enter.into()));
        assert!(!state.search.editing);
        assert_eq!(state.search.form.round, "llm");
    }

    #[test]
    fn search_editing_absorbs_q() {
        let mut state = test_state();
        state.view_mode = ViewMode::Search;
        state.search.begin_edit();
        // 編集中は `q` が入力として扱われ、終了しない
        assert!(handle_key_event(&mut state, key('q')));
        assert!(state.running);
        assert_eq!(state.search.edit_buf, "q");
    }

    #[test]
    fn search_falls_through_to_global_keys() {
        let mut state = test_state();
        state.view_mode = ViewMode::Search;
        // 非編集中の Shift+Q は既存のグローバル処理（終了）に到達する
        handle_key_event(&mut state, key('Q'));
        assert!(!state.running);

        // 単体 `q` は廃止済み（終了しない）
        let mut state2 = test_state();
        state2.view_mode = ViewMode::Search;
        handle_key_event(&mut state2, key('q'));
        assert!(state2.running);
    }

    #[test]
    fn search_enter_runs_search() {
        let mut state = test_state();
        state.view_mode = ViewMode::Search;
        state.search.form.word = "llm".into();
        state.search.form.round_gr = "1".into();
        // フォーカスを結果リスト（検索実行位置）へ
        state.search.focus = crate::ui::search::FORM_ITEMS;
        assert!(handle_key_event(&mut state, KeyCode::Enter.into()));
        // in-memory DB は空なので 0 件だが「検索済み」になる
        assert!(state.search.searched);
        assert!(state.search.hits.is_empty());
        assert!(state.search.error.is_none());
    }

    #[test]
    fn search_invalid_query_sets_error() {
        let mut state = test_state();
        state.view_mode = ViewMode::Search;
        state.search.form.round_gr = "x".into(); // 不正な数値範囲
        state.search.focus = crate::ui::search::FORM_ITEMS;
        assert!(handle_key_event(&mut state, KeyCode::Enter.into()));
        assert!(!state.search.searched);
        assert!(state.search.error.is_some());
    }

    #[test]
    fn search_esc_returns_to_workflow() {
        let mut state = test_state();
        state.view_mode = ViewMode::Search;
        state.search.focus = crate::ui::search::FORM_ITEMS; // 結果リスト上（非編集中）
        assert!(handle_key_event(&mut state, KeyCode::Esc.into()));
        assert_eq!(state.view_mode, ViewMode::Workflow);
    }

    // -------------------------------------------------------------------
    // Shift+* グローバルキー（リロード・ヘルプ・終了・旧キー廃止）
    // -------------------------------------------------------------------

    #[test]
    fn shift_r_reloads_data() {
        let mut state = test_state();
        // Shift+R（大文字 R）でリロードする
        assert!(handle_key_event(&mut state, KeyCode::Char('R').into()));
        // reload_data は異常なく完走する（in-memory DB は空）
        assert!(state.running);
    }

    #[test]
    fn shift_h_opens_help_and_any_key_closes() {
        let mut state = test_state();
        // Shift+H でヘルプが開く
        assert!(handle_key_event(&mut state, KeyCode::Char('H').into()));
        assert!(matches!(state.modal, super::super::Modal::Help));
        // 何かキーを押すと閉じる
        assert!(handle_key_event(&mut state, KeyCode::Char('x').into()));
        assert!(matches!(state.modal, super::super::Modal::None));
    }

    #[test]
    fn shift_q_quits_app() {
        let mut state = test_state();
        // Shift+Q でアプリを終了する
        handle_key_event(&mut state, KeyCode::Char('Q').into());
        assert!(!state.running);
    }

    /// 旧グローバルキー（単体 `q` / Ctrl+Q / Ctrl+F / Ctrl+R / `?`）は廃止済み。
    /// いずれも終了・遷移・ヘルプ表示をしない。
    #[test]
    fn legacy_keys_are_disabled() {
        let mut state = test_state();
        handle_key_event(&mut state, key('q'));
        assert!(state.running, "単体 q では終了しない");

        let mut state2 = test_state();
        handle_key_event(&mut state2, ctrl('q'));
        assert!(state2.running, "Ctrl+Q では終了しない");

        let mut state3 = test_state();
        handle_key_event(&mut state3, ctrl('f'));
        assert_eq!(
            state3.view_mode,
            ViewMode::Workflow,
            "Ctrl+F では遷移しない"
        );

        let mut state4 = test_state();
        handle_key_event(&mut state4, ctrl('r'));
        assert!(state4.running, "Ctrl+R は廃止済み（未定義キー扱い）");

        let mut state5 = test_state();
        handle_key_event(&mut state5, KeyCode::Char('?').into());
        assert!(
            matches!(state5.modal, super::super::Modal::None),
            "`?` ではヘルプを開かない"
        );
    }
}

// ============================================================================
// History ビュー（ratatui ネイティブ描画）のキー操作テスト
// ============================================================================

#[cfg(test)]
mod history_tests {
    use super::*;
    use sai_link_engine::db::{models::BlockMetadata, SaiStore};
    use sai_link_engine::session::HistoryEvent;
    use std::sync::Arc;

    /// History ビュー表示中の AppState を生成する。
    fn history_state() -> AppState {
        let store = Arc::new(SaiStore::open_in_memory().unwrap());
        let block = BlockMetadata::new(
            "hist-block".to_string(),
            String::new(),
            "m".to_string(),
            None,
            "off".to_string(),
            "true".to_string(),
            "true".to_string(),
            None,
            300,
            "none".to_string(),
            true,
            "blocks/test.md".to_string(),
            None
        );
        let block_id = store.insert_block(&block).unwrap();
        let e1 = store.create_execution(block_id, None).unwrap();
        let mut app = AppState::new(store);
        app.view_mode = ViewMode::History;
        app.load_session_output_to_main(e1);
        app
    }

    /// `j` / `k`（↑/↓）で Timeline の内部スクロールが動くこと。
    #[test]
    fn history_jk_scrolls_timeline() {
        let mut state = history_state();
        state.main_panel.history.as_mut().unwrap().events = (0..20)
            .map(|i| HistoryEvent {
                timestamp: String::new(),
                kind: "user".into(),
                name: format!("e{i}"),
                tokens: 1,
            })
            .collect();

        assert!(handle_key_event(&mut state, KeyCode::Char('j').into()));
        assert_eq!(
            state.main_panel.history.as_ref().unwrap().timeline_offset,
            1
        );
        assert!(handle_key_event(&mut state, KeyCode::Char('k').into()));
        assert_eq!(
            state.main_panel.history.as_ref().unwrap().timeline_offset,
            0
        );
    }

    /// PgUp/PgDn で Timeline がまとめてスクロールされること・Esc で Workflow へ戻ること。
    #[test]
    fn history_pgdn_pgup_and_esc() {
        let mut state = history_state();
        state.main_panel.history.as_mut().unwrap().events = (0..20)
            .map(|i| HistoryEvent {
                timestamp: String::new(),
                kind: "user".into(),
                name: format!("e{i}"),
                tokens: 1,
            })
            .collect();

        assert!(handle_key_event(&mut state, KeyCode::PageDown.into()));
        assert_eq!(
            state.main_panel.history.as_ref().unwrap().timeline_offset,
            5
        );
        assert!(handle_key_event(&mut state, KeyCode::PageUp.into()));
        assert_eq!(
            state.main_panel.history.as_ref().unwrap().timeline_offset,
            0
        );

        assert!(handle_key_event(&mut state, KeyCode::Esc.into()));
        assert_eq!(state.view_mode, ViewMode::Workflow);
        assert!(
            state.main_panel.history.is_none(),
            "Esc で history は破棄される"
        );
    }
}

/// フォーカスレス体系の新規テスト。
#[cfg(test)]
mod focusless_tests {
    use super::*;
    use crossterm::event::KeyModifiers;
    use sai_link_engine::db::SaiStore;
    use std::sync::Arc;

    fn key(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
    }

    /// `j/k` で sidebar カーソルが移動し、他画面では sidebar に影響しないこと。
    #[test]
    fn jk_moves_sidebar_only_in_workflow() {
        let mut state = AppState::new(Arc::new(SaiStore::open_in_memory().unwrap()));
        state.left_sidebar.tree_items = (0..3)
            .map(|i| crate::ui::left_sidebar::TreeItem {
                id: i,
                name: format!("n{i}"),
                depth: 0,
                expanded: false,
                is_last_child: false,
                kind: crate::ui::left_sidebar::TreeItemKind::Leaf,
                ancestor_last: Vec::new(),
                card_path: None,
            })
            .collect();
        state.left_sidebar.selected_index = 0;
        state.layout.left_sidebar.height = 20;
        assert!(handle_key_event(&mut state, key('j')));
        assert_eq!(state.left_sidebar.selected_index, 1);
        assert!(handle_key_event(&mut state, key('k')));
        assert_eq!(state.left_sidebar.selected_index, 0);
        // 他画面では sidebar に影響しない
        state.view_mode = ViewMode::Log;
        state.log_state.detail = None;
        assert!(handle_key_event(&mut state, key('j')));
        assert_eq!(state.left_sidebar.selected_index, 0);
    }

    /// `s/l/h` 再押しでは戻らず、`Esc` でのみ Workflow に戻ること。
    #[test]
    fn view_keys_are_one_way_and_esc_returns() {
        let mut state = AppState::new(Arc::new(SaiStore::open_in_memory().unwrap()));
        for (c, mode) in [('s', ViewMode::Stats), ('l', ViewMode::Log)] {
            assert!(handle_key_event(&mut state, key(c)));
            assert_eq!(state.view_mode, mode);
            assert!(handle_key_event(&mut state, key(c)));
            assert_eq!(state.view_mode, mode, "{c} 再押しでは戻らない");
            assert!(handle_key_event(
                &mut state,
                KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)
            ));
            assert_eq!(state.view_mode, ViewMode::Workflow);
        }
    }

    /// `Enter` がフォルダをトグルし、リーフで選択を変えないこと。
    #[test]
    fn enter_toggles_folder_only() {
        let mut state = AppState::new(Arc::new(SaiStore::open_in_memory().unwrap()));
        state.left_sidebar.tree_items = vec![crate::ui::left_sidebar::TreeItem {
            id: 1,
            name: "R1".into(),
            depth: 0,
            expanded: false,
            is_last_child: false,
            kind: crate::ui::left_sidebar::TreeItemKind::Folder,
            ancestor_last: Vec::new(),
            card_path: None,
        }];
        // フォルダは全アイテム再構築が必要なため、all_items 側にも用意する
        state.left_sidebar.all_items = state.left_sidebar.tree_items.clone();
        state.view_mode = ViewMode::Workflow;
        assert!(handle_key_event(
            &mut state,
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)
        ));
    }
}

/// message 画面のテスト。
#[cfg(test)]
mod message_tests {
    use super::*;
    use crossterm::event::KeyModifiers;
    use sai_link_engine::db::SaiStore;
    use std::sync::Arc;

    fn key(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
    }

    /// session jsonl（user 2件・assistant text 2件・thinking 除外）から
    /// message md が生成・保存・表示されること。
    #[test]
    fn m_extracts_and_shows_messages() {
        let _guard = crate::app::file_test_lock();
        let tmp = std::env::temp_dir().join(format!("sai_msg_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        std::env::set_var("SAI_PROJECT_ROOT", &tmp);

        let store = SaiStore::open_in_memory().unwrap();
        let block_id = store
            .insert_block(&sai_link_engine::db::models::BlockMetadata::new(
                "test".to_string(),
                String::new(),
                "m".to_string(),
                None,
                "off".to_string(),
                "true".to_string(),
                "true".to_string(),
                None,
                300,
                "none".to_string(),
                true,
                "blocks/test.md".to_string(),
                None
            ))
            .unwrap();
        let e1 = store.create_execution(block_id, None).unwrap();

        // session jsonl を用意（user文字列形式・assistant配列形式・thinking）
        let sdir = tmp.join("sess");
        std::fs::create_dir_all(&sdir).unwrap();
        let lines = [
            r#"{"type":"message","message":{"role":"user","content":"hello"}}"#,
            r#"{"type":"message","message":{"role":"assistant","content":[{"type":"text","text":"world"},{"type":"thinking","thinking":"hidden"}]}}"#,
            r#"{"type":"message","message":{"role":"user","content":[{"type":"text","text":"second"}]}}"#,
            r#"{"type":"message","message":{"role":"assistant","content":[{"type":"text","text":"reply2"}]}}"#,
        ];
        std::fs::write(sdir.join("main.jsonl"), lines.join("\n") + "\n").unwrap();
        store
            .update_execution_session_info(e1, &sdir.to_string_lossy(), "pi")
            .unwrap();

        let mut state = AppState::new(Arc::new(store));
        state.load_data();
        // 初期は全閉鎖のため実行リーフ選択の前に全展開する
        loop {
            let idx = state
                .left_sidebar
                .tree_items
                .iter()
                .position(|t| t.is_folder() && !t.expanded);
            let Some(i) = idx else { break };
            state.toggle_left_folder(i);
        }
        state.selected_exec_id = Some(e1);
        // `Space m` は sidebar カーソル基準のため、実行リーフを選択状態にする
        if let Some(idx) = state
            .left_sidebar
            .tree_items
            .iter()
            .position(|t| t.id == e1)
        {
            state.left_sidebar.selected_index = idx;
        }

        assert!(handle_key_event(&mut state, key(' ')));
        assert!(handle_key_event(&mut state, key('m')));
        assert_eq!(state.view_mode, ViewMode::Md);
        assert!(state.main_panel.md_view.is_some());

        // 保存ファイルの内容を検証
        let path = sai_link_engine::orchestrator::message_path(e1);
        let body = std::fs::read_to_string(&path).unwrap();
        assert!(body.contains("block_name: test"));
        assert!(body.contains("run_id:"));
        assert!(body.contains("# user (1)"));
        assert!(body.contains("hello"));
        assert!(body.contains("# assistant (1)"));
        assert!(body.contains("world"));
        assert!(body.contains("# user (2)"));
        assert!(body.contains("# assistant (2)"));
        assert!(!body.contains("hidden"), "thinking は除外される");

        // `Space m` 再押しでは戻らない。Esc で Workflow に戻る
        assert!(handle_key_event(&mut state, key(' ')));
        assert!(handle_key_event(&mut state, key('m')));
        assert_eq!(state.view_mode, ViewMode::Md);
        assert!(handle_key_event(
            &mut state,
            KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)
        ));
        assert_eq!(state.view_mode, ViewMode::Workflow);

        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// 未選択で `Space m` を押すと遷移せずメッセージのみであること。
    #[test]
    fn m_without_selection_shows_message() {
        let mut state = AppState::new(Arc::new(SaiStore::open_in_memory().unwrap()));
        state.load_data();
        assert!(handle_key_event(&mut state, key(' ')));
        assert!(handle_key_event(&mut state, key('m')));
        assert_eq!(state.view_mode, ViewMode::Workflow);
        assert!(state
            .message
            .as_deref()
            .unwrap_or("")
            .contains("選択されていません"));
    }

    /// 単体 `m` は md-tui に遷移せずガイダンスを表示すること。
    #[test]
    fn bare_m_shows_guidance() {
        let mut state = AppState::new(Arc::new(SaiStore::open_in_memory().unwrap()));
        state.load_data();
        assert!(handle_key_event(&mut state, key('m')));
        assert_eq!(state.view_mode, ViewMode::Workflow);
        assert!(state
            .message
            .as_deref()
            .unwrap_or("")
            .contains("Space m"));
    }

    /// `Space t` で task.md を md-tui に表示すること。
    #[test]
    fn space_t_shows_task() {
        use std::sync::Arc as StdArc;
        let _guard = crate::app::file_test_lock();
        let tmp = std::env::temp_dir().join(format!("sai_mdtask_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        std::env::set_var("SAI_PROJECT_ROOT", &tmp);
        let store = SaiStore::open_in_memory().unwrap();
        let block_id = store
            .insert_block(&sai_link_engine::db::models::BlockMetadata::new(
                "test".to_string(),
                String::new(),
                "m".to_string(),
                None,
                "off".to_string(),
                "true".to_string(),
                "true".to_string(),
                None,
                300,
                "none".to_string(),
                true,
                "blocks/test.md".to_string(),
                None
            ))
            .unwrap();
        let e1 = store.create_execution(block_id, None).unwrap();
        // run フォルダに task.md を用意
        let dir = sai_link_engine::run_dir::ensure_run_dir(
            sai_link_engine::run_dir::REVIEW_OWNER,
            e1,
        )
        .unwrap();
        std::fs::write(dir.join(sai_link_engine::run_dir::TASK_FILE), "# hello\n").unwrap();
        let mut state = AppState::new(StdArc::new(store));
        state.load_data();
        // 初期は全閉鎖のため実行リーフ選択の前に全展開する
        loop {
            let idx = state
                .left_sidebar
                .tree_items
                .iter()
                .position(|t| t.is_folder() && !t.expanded);
            let Some(i) = idx else { break };
            state.toggle_left_folder(i);
        }
        state.selected_exec_id = Some(e1);
        // sidebar ツリー上で実行リーフを選択状態にする（`Space t` はカーソル基準）
        if let Some(idx) = state
            .left_sidebar
            .tree_items
            .iter()
            .position(|t| t.id == e1)
        {
            state.left_sidebar.selected_index = idx;
        }
        assert!(handle_key_event(&mut state, key(' ')));
        assert!(handle_key_event(&mut state, key('t')));
        assert_eq!(state.view_mode, ViewMode::Md);
        assert!(state.main_panel.md_view.is_some());
        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// `Shift+M` で model-tui へ遷移し、`Esc` で Workflow に戻ること。
    #[test]
    fn shift_m_toggles_model_view() {
        let mut state = AppState::new(Arc::new(SaiStore::open_in_memory().unwrap()));
        assert!(handle_key_event(&mut state, key('M')));
        assert_eq!(state.view_mode, ViewMode::Model);
        // フィルタ入力は一覧に反映され、ビューは維持される
        assert!(handle_key_event(&mut state, key('d')));
        assert_eq!(state.model_view.filter, "d");
        assert_eq!(state.view_mode, ViewMode::Model);
        // Esc で Workflow に戻る
        assert!(handle_key_event(&mut state, KeyCode::Esc.into()));
        assert_eq!(state.view_mode, ViewMode::Workflow);
    }

    /// カード作成の自動項目はラウンドを埋めない（review の引き継ぎは廃止）。
    #[test]
    fn card_auto_fields_leave_rounds_empty() {
        let store = SaiStore::open_in_memory().unwrap();
        let block_id = store
            .insert_block(&sai_link_engine::db::models::BlockMetadata::new(
                "b".to_string(),
                String::new(),
                "m".to_string(),
                None,
                "off".to_string(),
                "true".to_string(),
                "true".to_string(),
                None,
                300,
                "none".to_string(),
                true,
                "blocks/b.md".to_string(),
                None
            ))
            .unwrap();
        let e1 = store.create_execution(block_id, None).unwrap();
        store
            .update_execution_overrides(e1, None, Some(2), Some(5), None, None, None)
            .unwrap();
        let _ = e1;
        let mut fm = sai_link_engine::kanban::CardFrontmatter::new_card(
            "2026/09/17 10:00:00".to_string(),
        );
        apply_card_auto_fields(&mut fm);
        assert_eq!(fm.global_round, "");
        assert_eq!(fm.round, "");
    }

    /// Md モードでの `Space` は見出し折りたたみに使われ、Workflow のリーダーキー
    /// （`Space t/m`）に飲み込まれて無視されないこと。
    #[test]
    fn space_toggles_fold_in_md_mode() {
        let store = sai_link_engine::db::SaiStore::open_in_memory().unwrap();
        let mut app = AppState::new(std::sync::Arc::new(store));
        let (lines, headings) = crate::ui::markdown::render_markdown(
            "# A\nbody\n",
            &sai_link_widgets::theme::AppTheme::dark(),
        );
        app.main_panel.md_view = Some(crate::ui::main_panel::MarkdownView {
            lines,
            headings,
            folded: std::collections::HashSet::new(),
            title: "t".into(),
            subtitle: String::new(),
            scroll: 0,
            cursor: 0,
            view_height: 0,
            outline_visible: false,
            outline_selected: 0,
            outline_focused: false,
        });
        app.view_mode = ViewMode::Md;

        let handled = handle_key_event(&mut app, key(' '));
        assert!(handled, "space はハンドルされるべき");
        assert!(
            app.main_panel
                .md_view
                .as_ref()
                .unwrap()
                .folded
                .contains(&0),
            "space で見出しが折りたためること（Workflow のリーダーキーに飲み込まれない）"
        );
        // Workflow のリーダーキー待機状態には入らない
        assert!(!app.space_pending, "Md モードでは space_pending は立たない");
    }
}
