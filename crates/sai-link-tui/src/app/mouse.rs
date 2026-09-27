//! マウス入力処理（app.rs から分割）。

use super::*;

pub fn handle_mouse_event(state: &mut AppState, mouse: MouseEvent) -> bool {
    let col = mouse.column;
    let row = mouse.row;
    let layout = &state.layout;

    // モーダル表示中はマウス操作を無視
    if !matches!(state.modal, Modal::None) {
        return false;
    }

    // CLI モード中はマウス操作を無視（専用キーマップのみ）
    if state.cli_mode.active {
        return false;
    }

    // ── ホイールはフォーカス中の画面へ（スクロールが効くのはフォーカス中の画面だけ）──
    // Workflow 表示中（一時ビューなし）はマウス位置ではなくフォーカス先に振り分ける。
    // フォーカスが workflow のときだけ従来どおりマウス位置ベースの処理へ進む。
    if matches!(
        mouse.kind,
        MouseEventKind::ScrollDown | MouseEventKind::ScrollUp
    ) && state.view_mode == ViewMode::Workflow
        && state.main_panel.md_view.is_none()
        && state.main_panel.project_dash.is_none()
        && state.main_panel.history.is_none()
    {
        let delta = if mouse.kind == MouseEventKind::ScrollDown { 1 } else { -1 };
        match state.focus {
            super::ScreenFocus::Sidebar => {
                state.selection = None; // スクロールで選択範囲は解除（従来どおり）
                super::input::move_sidebar_cursor(state, delta);
                return true;
            }
            super::ScreenFocus::Kanban => {
                state.selection = None;
                super::input::scroll_kanban(state, delta);
                return true;
            }
            super::ScreenFocus::Workflow => {}
        }
    }

    // 検索画面ではマウス操作を無効化（クリック判定の誤動作防止。将来拡張で
    // ホイールスクロール等を対応する）
    if state.view_mode == ViewMode::Search {
        return false;
    }

    // ── テキスト選択（Shift+左ドラッグ / 右ドラッグ / md ビュー中の左ドラッグ）──
    // 選択中はドラッグ・解放を選択操作として処理する。
    // - md ビュー（History / Blocks のメイン画面）では左ドラッグ = 行単位選択（エディター風）
    // - Workflow では左ドラッグ（パン）・クリックは Shift なしのとき従来どおり
    // - Shift+左ドラッグ / 右ドラッグはどこでも選択（md ビュー中は行単位）
    let md_open = state.main_panel.md_view.is_some();
    match mouse.kind {
        MouseEventKind::Down(MouseButton::Left)
            if !mouse.modifiers.contains(KeyModifiers::SHIFT)
                && md_open
                && state.layout.main.contains((col, row).into()) =>
        {
            // md ビュー: 左ドラッグで行単位選択を開始（パンは使わない）
            state.selection = Some(Selection {
                anchor_col: col,
                anchor_row: row,
                end_col: col,
                end_row: row,
                line_mode: true,
            });
            return true;
        }
        MouseEventKind::Down(MouseButton::Left)
            if mouse.modifiers.contains(KeyModifiers::SHIFT) =>
        {
            state.selection = Some(Selection {
                anchor_col: col,
                anchor_row: row,
                end_col: col,
                end_row: row,
                line_mode: md_open,
            });
            return true;
        }
        MouseEventKind::Down(MouseButton::Right) => {
            state.selection = Some(Selection {
                anchor_col: col,
                anchor_row: row,
                end_col: col,
                end_row: row,
                line_mode: md_open,
            });
            return true;
        }
        MouseEventKind::Drag(_) if state.selection.is_some() => {
            if let Some(sel) = &mut state.selection {
                sel.end_col = col;
                sel.end_row = row;
            }
            return true;
        }
        MouseEventKind::Up(_) if state.selection.is_some() => {
            // 移動なし（クリック）ならコピーせず、フォーカスのみ移す
            let no_move = {
                let s = state.selection.as_ref().unwrap();
                s.anchor_col == col && s.anchor_row == row
            };
            if no_move {
                state.selection = None;
                return true;
            }
            if let Some(sel) = &mut state.selection {
                sel.end_col = col;
                sel.end_row = row;
            }
            // 選択範囲のテキストを抽出してコピーし、選択を終了する
            let text = extract_selection_text_for_mode(state);
            let count = text.chars().count();
            let _ = count;
            copy_to_clipboard(&text);
            state.selection = None;
            return true;
        }
        MouseEventKind::ScrollDown | MouseEventKind::ScrollUp => {
            // スクロールで選択範囲がずれるため解除する（スクロール処理は下で継続）
            state.selection = None;
        }
        _ => {
            // 通常クリック等で選択を解除して、従来の処理へ進める
            state.selection = None;
        }
    }

    match mouse.kind {
        MouseEventKind::Down(MouseButton::Left) => {
            // 境界ドラッグ判定（境界線の±2px以内でドラッグ開始）
            // 1. 左SB ↔ メイン（垂直境界・幅変更）
            // 2. Kanban ↔ Workflow（メイン内部の水平境界・高さ変更。Workflow時のみ）
            let left_border = layout.left_sidebar.right();

            if col >= left_border.saturating_sub(2)
                && col <= left_border.saturating_add(1)
                && row >= layout.body.y
                && row < layout.body.bottom()
            {
                state.drag = Some(PanelDrag {
                    border: DragBorder::LeftMain,
                    start_col: col,
                    start_row: row,
                    start_scroll_x: state.main_panel.viewport.scroll_x,
                    start_scroll_y: state.main_panel.viewport.scroll_y,
                    pan_active: false,
                });
                return true;
            }
            // Kanban ↔ Workflow 境界（Workflow 表示時のみ有効。
            // Kanban 側にフォーカスは不要のため、ドラッグのみ受け付ける）
            if state.view_mode == ViewMode::Workflow {
                let h_border = layout.kanban.bottom();
                if row + 1 >= h_border.saturating_sub(1)
                    && row <= h_border
                    && col >= layout.main.x
                    && col < layout.main.right()
                {
                    state.drag = Some(PanelDrag {
                        border: DragBorder::KanbanWorkflow,
                        start_col: col,
                        start_row: row,
                        start_scroll_x: state.main_panel.viewport.scroll_x,
                        start_scroll_y: state.main_panel.viewport.scroll_y,
                        pan_active: false,
                    });
                    return true;
                }
            }
            // 通常クリック
            if layout.left_sidebar.contains((col, row).into()) {
                handle_left_sidebar_click(state, col, row);
            } else if layout.workflow.contains((col, row).into()) {
                // Workflow 以外のモード（Stats / Log / History / Project）では
                // メイン画面のパンは使わない（クリック・パンとも無効）
                if state.view_mode != ViewMode::Workflow {
                    return false;
                }
                // Workflow 領域: パン開始の前に card / round 区切り線ラベルへの
                // ヒットテストを行う（逆順 = 後に描いた card が手前）。
                // ヒットすればそれぞれのドラッグへ、外れたら従来どおり MainPan（パン）。
                let hit_node = state
                    .main_panel
                    .node_screen_rects
                    .iter()
                    .rev()
                    .find(|(_, rect)| rect.contains((col, row).into()))
                    .map(|&(exec_id, _)| exec_id);
                let hit_separator = state
                    .main_panel
                    .separator_screen_rects
                    .iter()
                    .rev()
                    .find(|(_, rect)| rect.contains((col, row).into()))
                    .map(|&(global_round, _)| global_round);

                let border = if let Some(exec_id) = hit_node {
                    let base = state
                        .main_panel
                        .node_offsets
                        .get(&exec_id)
                        .copied()
                        .unwrap_or((0, 0));
                    DragBorder::Node { exec_id, base }
                } else if let Some(global_round) = hit_separator {
                    let base = state
                        .main_panel
                        .separator_offsets
                        .get(&global_round)
                        .copied()
                        .unwrap_or(0);
                    DragBorder::Separator { global_round, base }
                } else {
                    DragBorder::MainPan
                };

                // Kanban 領域は操作不可のためパン対象外。
                state.drag = Some(PanelDrag {
                    border,
                    start_col: col,
                    start_row: row,
                    start_scroll_x: state.main_panel.viewport.scroll_x,
                    start_scroll_y: state.main_panel.viewport.scroll_y,
                    pan_active: false,
                });
            }
        }
        MouseEventKind::Drag(MouseButton::Left) | MouseEventKind::Moved if state.drag.is_some() => {
            if let Some(ref mut d) = state.drag {
                match d.border {
                    DragBorder::LeftMain => {
                        let body_width = layout.body.width.max(1) as i32;
                        let body_left = layout.body.x as i32;
                        let rel_x = col as i32 - body_left;
                        state.left_panel_pct = (rel_x * 100 / body_width).clamp(5, 45) as u16;
                    }
                    DragBorder::KanbanWorkflow => {
                        // Kanban 高さ比率を変更する（15〜70% にクランプ）
                        let main_h = layout.main.height.max(1) as i32;
                        let rel_y = row as i32 - layout.main.y as i32;
                        state.kanban_pct = (rel_y * 100 / main_h).clamp(15, 70) as u16;
                    }
                    DragBorder::MainPan => {
                        // メイン画面: クリックしたまま動かすとビューポートを移動
                        let dx = col as i32 - d.start_col as i32;
                        let dy = row as i32 - d.start_row as i32;
                        // 数px 以上動いたらドラッグ（パン）扱い
                        if !d.pan_active && (dx.abs() > 2 || dy.abs() > 2) {
                            d.pan_active = true;
                        }
                        if d.pan_active {
                            let viewport = &mut state.main_panel.viewport;
                            viewport.manual_scroll = true;
                            viewport.scroll_x = (d.start_scroll_x - dx).max(0);
                            viewport.scroll_y = (d.start_scroll_y - dy).max(0);
                        }
                    }
                    DragBorder::Node { exec_id, base } => {
                        // card のドラッグ移動: 画面上の移動量 ÷ zoom をズレ量に加える
                        let dx_px = col as i32 - d.start_col as i32;
                        let dy_px = row as i32 - d.start_row as i32;
                        if !d.pan_active && (dx_px.abs() > 2 || dy_px.abs() > 2) {
                            d.pan_active = true;
                        }
                        if d.pan_active {
                            let zoom = state.main_panel.zoom;
                            let dx = (dx_px as f64 / zoom).round() as i32;
                            let dy = (dy_px as f64 / zoom).round() as i32;
                            state
                                .main_panel
                                .node_offsets
                                .insert(exec_id, (base.0 + dx, base.1 + dy));
                        }
                    }
                    DragBorder::Separator { global_round, base } => {
                        // round 区切り線は縦方向のみ。前の round に食い込ませない（下限 0）。
                        let dx_px = col as i32 - d.start_col as i32;
                        let dy_px = row as i32 - d.start_row as i32;
                        if !d.pan_active && (dx_px.abs() > 2 || dy_px.abs() > 2) {
                            d.pan_active = true;
                        }
                        if d.pan_active {
                            let zoom = state.main_panel.zoom;
                            let dy = (dy_px as f64 / zoom).round() as i32;
                            state
                                .main_panel
                                .separator_offsets
                                .insert(global_round, (base + dy).max(0));
                        }
                    }
                }
            }
            return true;
        }
        MouseEventKind::Up(MouseButton::Left) => {
            if let Some(d) = state.drag.take() {
                match d.border {
                    // card / 区切り線を実際に動かした場合のみファイルへ保存する
                    DragBorder::Node { .. } | DragBorder::Separator { .. } if d.pan_active => {
                        super::ui_layout::UiLayout {
                            nodes: state.main_panel.node_offsets.clone(),
                            separators: state.main_panel.separator_offsets.clone(),
                        }
                        .save();
                    }
                    // メイン画面 / card: 移動なしの場合はクリックとして扱う
                    DragBorder::MainPan | DragBorder::Node { .. } if !d.pan_active => {
                        handle_main_click(state, col, row);
                    }
                    _ => {}
                }
                return true;
            }
        }
        MouseEventKind::ScrollDown => {
            // Kanban 領域上のホイールは無視（操作不可）。
            // Workflow 領域・他モードのメイン全体でスクロール/ズームを受け付ける。
            if layout.kanban.contains((col, row).into())
                && state.view_mode == ViewMode::Workflow
                && state.main_panel.md_view.is_none()
                && state.main_panel.project_dash.is_none()
                && state.main_panel.history.is_none()
            {
                return false;
            }
            if layout.main.contains((col, row).into()) {
                // Stats ビュー: Per-message breakdown を下スクロール
                if state.view_mode.is_stats() {
                    state.cache_view.per_message_scroll =
                        state.cache_view.per_message_scroll.saturating_add(3);
                } else if state.view_mode == ViewMode::Log {
                    // Log 画面: 下スクロール
                    state.log_state.scroll = state.log_state.scroll.saturating_add(3);
                } else if state.view_mode == ViewMode::History && state.main_panel.history.is_some()
                {
                    // History 画面: Timeline を下スクロール
                    crate::ui::history_view::timeline_scroll(
                        state.main_panel.history.as_mut().unwrap(),
                        3,
                    );
                } else if state.main_panel.md_view.is_some()
                    || state.main_panel.project_dash.is_some()
                {
                    // マークダウン表示 / Project ダッシュボード: 下スクロール
                    md_scroll(state, 1);
                } else if mouse.modifiers.contains(KeyModifiers::CONTROL) {
                    // Ctrl+スクロールダウン = ズームイン（アンカーは Workflow 領域）
                    zoom_main_panel(state, layout.workflow, col, row, 1.25);
                } else {
                    state.main_panel.viewport.scroll_vertical(3);
                }
            } else if layout.left_sidebar.contains((col, row).into())
                && state.is_sidebar_enabled()
                && state.focus == super::ScreenFocus::Sidebar
            {
                // sidebar 上のホイール = カーソル移動（フォーカスが sidebar のときのみ）
                super::input::move_sidebar_cursor(state, 1);
            }
        }
        MouseEventKind::ScrollUp => {
            // Kanban 領域上のホイールは無視（操作不可）。
            if layout.kanban.contains((col, row).into())
                && state.view_mode == ViewMode::Workflow
                && state.main_panel.md_view.is_none()
                && state.main_panel.project_dash.is_none()
                && state.main_panel.history.is_none()
            {
                return false;
            }
            if layout.main.contains((col, row).into()) {
                // Stats ビュー: Per-message breakdown を上スクロール
                if state.view_mode.is_stats() {
                    state.cache_view.per_message_scroll =
                        state.cache_view.per_message_scroll.saturating_sub(3);
                } else if state.view_mode == ViewMode::Log {
                    // Log 画面: 上スクロール
                    state.log_state.scroll = state.log_state.scroll.saturating_sub(3);
                } else if state.view_mode == ViewMode::History && state.main_panel.history.is_some()
                {
                    crate::ui::history_view::timeline_scroll(
                        state.main_panel.history.as_mut().unwrap(),
                        -3,
                    );
                } else if state.main_panel.md_view.is_some()
                    || state.main_panel.project_dash.is_some()
                {
                    // マークダウン表示 / Project ダッシュボード: 上スクロール
                    md_scroll(state, -1);
                } else if mouse.modifiers.contains(KeyModifiers::CONTROL) {
                    // Ctrl+スクロールアップ = ズームアウト（アンカーは Workflow 領域）
                    zoom_main_panel(state, layout.workflow, col, row, 0.8);
                } else {
                    state.main_panel.viewport.scroll_vertical(-3);
                }
            } else if layout.left_sidebar.contains((col, row).into())
                && state.is_sidebar_enabled()
                && state.focus == super::ScreenFocus::Sidebar
            {
                // sidebar 上のホイール = カーソル移動（フォーカスが sidebar のときのみ）
                super::input::move_sidebar_cursor(state, -1);
            }
        }
        _ => return false,
    }

    true
}

/// 左サイドバーのクリック処理。
///
/// フォルダ（R / r）をクリックした場合は折りたたみ/展開をトグルし、
/// リーフ（実行）をクリックした場合は右SB に詳細を読み込む。
fn handle_left_sidebar_click(state: &mut AppState, _col: u16, row: u16) {
    let area = state.layout.left_sidebar;
    // フィルタ欄は廃止されたため、ツリーはボーダーの直下（y+1）から始まる
    let tree_start = area.y + 1;
    if row < tree_start || row >= area.bottom() {
        return;
    }

    let clicked_index = (row - tree_start) as usize + state.left_sidebar.scroll_offset;
    if clicked_index < state.left_sidebar.tree_items.len() {
        state.left_sidebar.selected_index = clicked_index;
        let is_folder = state.left_sidebar.tree_items[clicked_index].is_folder();
        if is_folder {
            state.toggle_left_folder(clicked_index);
        } else {
            // 左SB で選択したら自動追従に戻す（マウスパン・ズーム後の手動スクロールを解除）
            // 実行リーフ・todo カードの区別は種別で行う（実行リーフの `card_path` 有無では判定しない）。
            // 実行リーフは `c` 用に `card_path` を持つため（`data.rs` 参照）、`card_path` の
            // 有無では判定しないこと。キー操作（`select_left_item`）と一致させる。
            state.left_sidebar.selected_index = clicked_index;
            let is_card = matches!(
                state.left_sidebar.tree_items[clicked_index].kind,
                crate::ui::left_sidebar::TreeItemKind::Card
            );
            if !is_card {
                state.selected_exec_id = Some(state.left_sidebar.tree_items[clicked_index].id);
                state.main_panel.selected_block_id = state.selected_exec_id;
            }
            state.main_panel.viewport.manual_scroll = false;
        }
    }
}

/// メイン画面のクリック処理。
///
/// `node_screen_rects` を逆順（後に描いた card が手前）でヒットテストし、
/// 当たった card を選択する（クリックで選択・ドラッグで移動が揃う）。
/// 何も当たらなければ何もしない（選択は変えない）。
fn handle_main_click(state: &mut AppState, col: u16, row: u16) {
    let hit = state
        .main_panel
        .node_screen_rects
        .iter()
        .rev()
        .find(|(_, rect)| rect.contains((col, row).into()))
        .map(|&(exec_id, _)| exec_id);
    if let Some(exec_id) = hit {
        state.main_panel.selected_block_id = Some(exec_id);
        state.selected_exec_id = Some(exec_id);
    }
}

/// メイン画面のズーム倍率を変更する。
///
/// マウスカーソル直下のコンテンツ座標が同じ位置に留まるよう、
/// スクロール位置も倍率比で補正する（ズームのアンカー）。
/// 編集モード（エディタ表示中）では何もしない。
pub(crate) fn zoom_main_panel(
    state: &mut AppState,
    main_area: Rect,
    col: u16,
    row: u16,
    factor: f64,
) {
    if state.main_panel.md_view.is_some() {
        return;
    }
    let new_zoom = (state.main_panel.zoom * factor).clamp(0.2, 5.0);
    if (new_zoom - state.main_panel.zoom).abs() < f64::EPSILON {
        return;
    }

    let rel_x = i64::from(col.saturating_sub(main_area.x));
    let rel_y = i64::from(row.saturating_sub(main_area.y));
    let sx = i64::from(state.main_panel.viewport.scroll_x);
    let sy = i64::from(state.main_panel.viewport.scroll_y);
    let ratio = new_zoom / state.main_panel.zoom;

    // カーソル直下のコンテンツ座標 (rel + scroll) を倍率比で変換し、
    // 再びカーソル位置に対応するようスクロールを補正する
    state.main_panel.viewport.scroll_x = (((rel_x + sx) as f64 * ratio - rel_x as f64)
        .round()
        .max(0.0)) as i32;
    state.main_panel.viewport.scroll_y = (((rel_y + sy) as f64 * ratio - rel_y as f64)
        .round()
        .max(0.0)) as i32;
    state.main_panel.viewport.manual_scroll = true;
    state.main_panel.zoom = new_zoom;
}

// ============================================================================
// テキスト選択とクリップボード
// ============================================================================

/// モードに応じた選択テキスト抽出。
///
/// - md ビュー（line_mode）: コンテンツ行を**行単位**で抽出（エディター風・罫線なし）
/// - それ以外: 画面セルの矩形から抽出
fn extract_selection_text_for_mode(state: &AppState) -> String {
    let line_mode = state
        .selection
        .map(|s| s.line_mode && state.main_panel.md_view.is_some())
        .unwrap_or(false);
    if line_mode {
        extract_md_selection_text(state)
    } else {
        extract_selection_text(state)
    }
}

/// md ビュー（History / Blocks のメイン画面）の選択テキストを行単位で抽出する。
///
/// 画面セルではなく `MarkdownView.lines`（レンダリング済みテキスト行）から取得するため、
/// 罫線（`│` 等）やスタイルが混入せず、エディターと同じ「行全体」がコピーされる。
fn extract_md_selection_text(state: &AppState) -> String {
    let (Some(sel), Some(md)) = (&state.selection, state.main_panel.md_view.as_ref()) else {
        return String::new();
    };
    let y1 = sel.anchor_row.min(sel.end_row) as usize;
    let y2 = sel.anchor_row.max(sel.end_row) as usize;
    // 本文領域はメイン画面の y+2（タイトル・セパレータ分）から。
    // 表示行 = コンテンツ行 - md.scroll の対応でコンテンツ行を逆算する。
    let body_top = state.layout.main.y as usize + 2;
    let mut out = String::new();
    let mut first = true;
    for y in y1..=y2 {
        let body_row = y as i64 - body_top as i64;
        if body_row < 0 {
            continue; // タイトル/セパレータより上はスキップ
        }
        let content_row = body_row + md.scroll as i64;
        // コンテンツ行の範囲外（本文下部のガイド行等）は空行として扱う
        let line = match md.lines.get(content_row as usize) {
            Some(l) => line_to_string(l),
            None => String::new(),
        };
        if !first {
            out.push('\n');
        }
        out.push_str(&line);
        first = false;
    }
    out
}

/// スタイル付き `Line` から表示テキスト（プレーン文字列）を取り出す。
fn line_to_string(line: &ratatui::text::Line) -> String {
    let mut s = String::new();
    for span in &line.spans {
        s.push_str(span.content.as_ref());
    }
    s
}

/// 選択範囲（矩形）内の画面テキストを抽出する。
///
/// `draw_ui` が毎フレーム更新する `state.screen_cache`（セル文字列）から
/// アンカー〜端の矩形を切り出す。全角文字の右半分（`'\0'`）はスキップし、
/// 行末の空白は整形する。
fn extract_selection_text(state: &AppState) -> String {
    let Some(sel) = &state.selection else {
        return String::new();
    };
    let x1 = sel.anchor_col.min(sel.end_col) as usize;
    let x2 = sel.anchor_col.max(sel.end_col) as usize;
    let y1 = sel.anchor_row.min(sel.end_row) as usize;
    let y2 = sel.anchor_row.max(sel.end_row) as usize;

    let mut out = String::new();
    for y in y1..=y2 {
        let mut row = String::new();
        if let Some(r) = state.screen_cache.get(y) {
            let x_end = x2.min(r.len().saturating_sub(1));
            for x in x1..=x_end {
                let ch = r[x];
                if ch != '\0' {
                    row.push(ch);
                }
            }
        }
        // 行末の空白は削って整形（選択範囲の右端が半端な場合に備える）
        let row = row.trim_end();
        out.push_str(row);
        if y < y2 {
            out.push('\n');
        }
    }
    out
}

/// テキストをクリップボードへコピーする。
///
/// 1. **OSC 52**（`ESC ] 52 ; c ; <base64> ESC \\`）を端末エミュレータへ送る。
///    Windows Terminal / iTerm2 / kitty / WezTerm / foot 等で動作。
/// 2. 外部クリップボードツール（`pbcopy` / `xclip` / `wl-copy`）へも
///    ベストエフォートで送る（どれか 1 つでも起動できれば採用）。
fn copy_to_clipboard(text: &str) {
    use std::io::Write;

    // OSC 52: クリップボードへセット
    let b64 = base64_encode(text.as_bytes());
    let seq = format!("\x1b]52;c;{}\x1b\\", b64);
    let mut out = std::io::stdout();
    let _ = out.write_all(seq.as_bytes());
    let _ = out.flush();

    // 外部ツールへのフォールバック（spawn できた最初のものだけ使う）
    for (tool, args) in [
        ("pbcopy", &[][..]),
        ("xclip", &["-selection", "clipboard"][..]),
        ("wl-copy", &[][..]),
    ] {
        if let Ok(mut child) = std::process::Command::new(tool)
            .args(args)
            .stdin(std::process::Stdio::piped())
            .spawn()
        {
            if let Some(mut stdin) = child.stdin.take() {
                let _ = stdin.write_all(text.as_bytes());
                let _ = stdin.flush();
            }
            let _ = child.wait();
            break;
        }
    }
}

/// 簡易 Base64 エンコーダ（OSC 52 用。外部クレート不要）。
fn base64_encode(data: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0];
        let b1 = *chunk.get(1).unwrap_or(&0);
        let b2 = *chunk.get(2).unwrap_or(&0);
        let n = ((b0 as u32) << 16) | ((b1 as u32) << 8) | (b2 as u32);
        out.push(TABLE[(n >> 18) as usize & 63] as char);
        out.push(TABLE[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            TABLE[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            TABLE[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

// ============================================================================
// イベントループ
// ============================================================================

#[cfg(test)]
mod tests {
    // 現時点ではマウス処理の単体テストなし（手動確認手順は実装計画書 U7 参照）
}
