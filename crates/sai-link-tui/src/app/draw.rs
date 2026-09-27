//! 画面描画（app.rs から分割）。
//!
//! 画面構成は 左SB + メイン（Kanban上 + Workflow下） の 3 領域
//! （Header・右SB は廃止）。Kanban 専用モードは廃止し、Workflow 表示時は
//! 常に上下 2 分割で表示する。

use super::*;

pub fn draw_ui(frame: &mut Frame, state: &mut AppState) {
    let terminal_size = frame.area();
    let layout = compute_layout(terminal_size, state.left_panel_pct, state.kanban_pct);
    state.layout = layout.clone();

    // 左サイドバー
    frame.render_widget(
        LeftSidebar {
            state: &state.left_sidebar,
            theme: &state.theme,
        },
        layout.left_sidebar,
    );

    // メイン画面
    match state.view_mode {
        // Stats ビュー（キャッシュ統計グラフ）のときは CacheView を描画する
        ViewMode::Stats => {
            frame.render_widget(
                crate::ui::cache_view::CacheView {
                    state: &state.cache_view,
                },
                layout.main,
            );
        }
        // 検索画面（セッション履歴検索。Shift+F で遷移）
        ViewMode::Search => {
            frame.render_widget(
                crate::ui::search::SearchView {
                    state: &mut state.search,
                    theme: &state.theme,
                },
                layout.main,
            );
        }
        // Log 画面（旧右SB の移植。`l` 押下時点の実行詳細で固定）
        ViewMode::Log => {
            frame.render_widget(
                LogView {
                    state: &mut state.log_state,
                    theme: &state.theme,
                },
                layout.main,
            );
        }
        // model-tui（`Shift+M`）: pi の使用可能モデル一覧
        ViewMode::Model => {
            frame.render_widget(
                crate::ui::model_view::ModelListView {
                    state: &state.model_view,
                },
                layout.main,
            );
        }
        // Workflow / History / Project: 上=Kanban・下=Workflow の 2 分割。
        // History / Project（一時ビュー）表示中はメイン全体にそちらを出す。
        _ => {
            let temp_view = state.main_panel.md_view.is_some()
                || state.main_panel.project_dash.is_some()
                || state.main_panel.history.is_some();
            if temp_view {
                frame.render_widget(
                    MainPanel {
                        state: &mut state.main_panel,
                        blocks: &state.block_displays,
                        connections: &state.connection_displays,
                        finalized_runs: &state.finalized_run_parents,
                        orphaned_runs: &state.orphaned_run_ids,
                        theme: &state.theme,
                    },
                    layout.main,
                );
            } else {
                // 上半分: Kanban ボード（読み取り専用。直接操作不可）
                // 一覧は `load_data` 時に構築済み（描画のたびに再計算しない）。
                let sel_item = state
                    .left_sidebar
                    .tree_items
                    .get(state.left_sidebar.selected_index);
                let selected_card = sel_item.and_then(|t| t.card_path.clone());
                // 実行リーフのみ実行 ID で照合（todo カードは負値ダミー ID のため対象外）
                let selected_exec = sel_item
                    .filter(|t| matches!(t.kind, TreeItemKind::Leaf))
                    .map(|t| t.id);
                let board = crate::app::kanban::to_board_data(
                    &state.kanban_entries,
                    selected_card.as_deref(),
                    selected_exec,
                    state.kanban_scroll,
                    &state.main_panel.cost_labels,
                );
                frame.render_widget(
                    crate::ui::kanban::KanbanBoard {
                        data: &board,
                        theme: &state.theme,
                    },
                    layout.kanban,
                );
                // 下半分: Workflow DAG（従来の MainPanel）
                frame.render_widget(
                    MainPanel {
                        state: &mut state.main_panel,
                        blocks: &state.block_displays,
                        connections: &state.connection_displays,
                        finalized_runs: &state.finalized_run_parents,
                        orphaned_runs: &state.orphaned_run_ids,
                        theme: &state.theme,
                    },
                    layout.workflow,
                );
            }
        }
    }

    // 常設ステータスバー（画面最下端・常に表示）
    draw_status_bar(frame.buffer_mut(), layout.status_bar, state);

    // モーダルオーバーレイ（最前面、Help のみ）
    if !matches!(state.modal, Modal::None) {
        render_modal(&state.modal, terminal_size, frame.buffer_mut(), state.theme.main_bg);
    }

    // CLI モードオーバーレイ（`x`・モーダルより前面）
    if state.cli_mode.active {
        crate::app::cli_mode::render_cli_mode(
            &state.cli_mode,
            terminal_size,
            frame.buffer_mut(),
            state.theme.main_bg,
        );
    }

    // パネル枠線（`Tab` のフォーカス中の画面だけ強調色。それ以外は通常色）
    // CLI モード（`x` run-tui）オーバーレイ描画中は枠線を描かない。
    // オーバーレイより後に枠線を描くと、左SB 右辺・メイン左辺の縦線が
    // ダイアログのタイトル（`run-tui (x)`）に上書き混入する（過去バグ）。
    let unfocused_style = ratatui::style::Style::default().fg(state.theme.border);
    let focused_style = ratatui::style::Style::default().fg(state.theme.selected);
    // フォーカス表示はワークフロー画面（3画面が並ぶ構成）のときだけ意味を持つ
    let focus_on = state.view_mode == ViewMode::Workflow;
    let style_for = |f: ScreenFocus| {
        if focus_on && state.focus == f {
            focused_style
        } else {
            unfocused_style
        }
    };

    let left_style = style_for(ScreenFocus::Sidebar);
    let main_style = style_for(ScreenFocus::Workflow);

    if matches!(state.modal, Modal::None) && !state.cli_mode.active {
        draw_panel_border(frame.buffer_mut(), layout.left_sidebar, left_style);
        // Workflow 分割時は Kanban / Workflow に個別枠線を描く。
        // 一時ビュー・他モード時はメイン全体に 1 本。
        let split = state.view_mode == ViewMode::Workflow
            && state.main_panel.md_view.is_none()
            && state.main_panel.project_dash.is_none()
            && state.main_panel.history.is_none();
        if split {
            draw_panel_border(
                frame.buffer_mut(),
                layout.kanban,
                style_for(ScreenFocus::Kanban),
            );
            draw_panel_border(frame.buffer_mut(), layout.workflow, main_style);
        } else {
            draw_panel_border(frame.buffer_mut(), layout.main, main_style);
        }
    }

    // 画面テキストのキャッシュ（ドラッグ選択のコピー用。ハイライト前の内容を保持）
    {
        let area = frame.area();
        let w = area.width as usize;
        let h = area.height as usize;
        let mut cache = vec![vec![' '; w]; h];
        for (i, cell) in frame.buffer_mut().content().iter().enumerate() {
            let x = i % w;
            let y = i / w;
            // 全角文字の右半分（symbol が空）は '\0' でマーク
            cache[y][x] = cell.symbol().chars().next().unwrap_or('\0');
        }
        state.screen_cache = cache;
    }

    // ドラッグ選択ハイライト（最前面に反転表示）
    if let Some(sel) = state.selection {
        // md ビューの行単位選択では、メイン画面の全幅を行単位でハイライトする
        let (x1, x2) = if sel.line_mode {
            (
                state.layout.main.x,
                state.layout.main.right().saturating_sub(1),
            )
        } else {
            (
                sel.anchor_col.min(sel.end_col),
                sel.anchor_col.max(sel.end_col),
            )
        };
        let y1 = sel.anchor_row.min(sel.end_row);
        let y2 = sel.anchor_row.max(sel.end_row);
        for y in y1..=y2 {
            for x in x1..=x2 {
                if let Some(cell) = frame.buffer_mut().cell_mut((x, y)) {
                    cell.modifier.insert(ratatui::style::Modifier::REVERSED);
                }
            }
        }
    }
}

/// 常設ステータスバーを描画する（画面最下端・常に表示）。
///
/// 左側に一時メッセージ（あれば。なければキーガイド）、右側に選択中実行 ID を表示する。
/// `Space` 待機中はガイドを強調表示する。
fn draw_status_bar(
    buf: &mut ratatui::buffer::Buffer,
    area: ratatui::layout::Rect,
    state: &AppState,
) {
    if area.height == 0 || area.width == 0 {
        return;
    }
    let y = area.y;
    let style = ratatui::style::Style::default()
        .fg(ratatui::style::Color::White)
        .bg(ratatui::style::Color::DarkGray);
    // 背景を塗る
    for x in area.x..area.right() {
        if let Some(cell) = buf.cell_mut((x, y)) {
            cell.set_style(style);
        }
    }
    // 左側: Space 待機中 > 一時メッセージ > 既定ガイド
    let left = if state.space_pending {
        "Space: r=review t=task m=message (Esc=解除)".to_string()
    } else if let Some(msg) = state.message.as_deref() {
        msg.to_string()
    } else {
        "Space r/t/m: md表示 | x: CLI | Shift+H: ヘルプ".to_string()
    };
    // 右側: フォーカス中の画面 + 選択中実行 ID
    let focus_part = if state.view_mode == ViewMode::Workflow {
        format!("[{}] ", state.focus.label())
    } else {
        String::new()
    };
    let right = match state.selected_exec_id {
        Some(id) => format!("{}ID{} ", focus_part, id),
        None => {
            if focus_part.is_empty() {
                String::new()
            } else {
                format!("{}", focus_part.trim_end())
            }
        }
    };
    crate::ui::draw::draw_text_cjk(buf, area.x + 1, y, &left, style);
    if !right.is_empty() {
        let rx = area.right().saturating_sub(right.len() as u16 + 1);
        crate::ui::draw::draw_text_cjk(buf, rx.max(area.x + 1), y, &right, style);
    }
}

// ============================================================================
// マウスイベントハンドラ
// ============================================================================

/// パネルに四隅付きの枠線を描画する。背景色は変更しない。
fn draw_panel_border(
    buf: &mut ratatui::buffer::Buffer,
    rect: ratatui::layout::Rect,
    style: ratatui::style::Style,
) {
    use ratatui::style::Color;
    // 0 サイズの面板には描画しない（小さいターミナルでのオーバーフロー回避）
    if rect.height == 0 || rect.width == 0 {
        return;
    }
    let fg = style.fg.unwrap_or(Color::Reset);
    // 上辺
    for x in rect.x..rect.x + rect.width {
        if let Some(cell) = buf.cell_mut((x, rect.y)) {
            cell.set_char('─');
            cell.fg = fg;
        }
    }
    // 下辺
    for x in rect.x..rect.x + rect.width {
        if let Some(cell) = buf.cell_mut((x, rect.y + rect.height - 1)) {
            cell.set_char('─');
            cell.fg = fg;
        }
    }
    // 左辺
    for y in rect.y..rect.y + rect.height {
        if let Some(cell) = buf.cell_mut((rect.x, y)) {
            cell.set_char('│');
            cell.fg = fg;
        }
    }
    // 右辺
    for y in rect.y..rect.y + rect.height {
        if let Some(cell) = buf.cell_mut((rect.x + rect.width - 1, y)) {
            cell.set_char('│');
            cell.fg = fg;
        }
    }
    // 四隅
    if let Some(cell) = buf.cell_mut((rect.x, rect.y)) {
        cell.set_char('┌');
        cell.fg = fg;
    }
    if let Some(cell) = buf.cell_mut((rect.x + rect.width - 1, rect.y)) {
        cell.set_char('┐');
        cell.fg = fg;
    }
    if let Some(cell) = buf.cell_mut((rect.x, rect.y + rect.height - 1)) {
        cell.set_char('└');
        cell.fg = fg;
    }
    if let Some(cell) = buf.cell_mut((rect.x + rect.width - 1, rect.y + rect.height - 1)) {
        cell.set_char('┘');
        cell.fg = fg;
    }
}
