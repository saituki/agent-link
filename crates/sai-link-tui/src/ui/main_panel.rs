//! メイン画面（ブロック接続グラフ / マークダウン表示）。
//!
//! Workflow モード: DAG ブロック線図を描画。
//! History モード: ratatui ネイティブ描画（`history_view`）。
//! 一時的なマークダウン表示（`show_block_content` 用）もここで扱う。

use std::collections::HashMap;

use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{
        Block, BorderType, Borders, List, ListItem, ListState, Paragraph, StatefulWidget, Widget,
        Wrap,
    },
};

use crate::graph_layout::{
    calculate_layout, render_edges, render_separators, GraphViewport, NodePosition,
};
// DAG とレイアウトグラフの型は TUI 側では参照しない（組み立ては widgets 側）。
// ただしテスト（`use super::*`）は辺・レイアウトを検証するため公開のまま使う。
#[cfg(test)]
use crate::graph_layout::{DagGraph, EdgeData, EdgeKind, LayoutGraph};
use crate::widgets::block_node::{BlockNodeData, BlockNodeWidget};

use super::draw;
use sai_link_widgets::theme::AppTheme;
use sai_link_widgets::workflow::build_block_graph;

// DAG の組み立て（build_block_graph）と表示データは `sai-link-widgets` にある。
// `sai-link phone workflow`（CLI）と同じ処理を使うため、TUI には実装を残さない。
// テスト（`use super::*`）からも見えるよう再公開する。
pub use sai_link_widgets::workflow::{BlockDisplayData, ConnectionDisplayData};

/// md viewer のカーソル行背景（コードブロック背景より少し明るくして区別する）。
const MD_CURSOR_BG: Color = Color::Rgb(45, 45, 70);

/// 本文領域の指定スクリーン行範囲（折り返し後の行番号）を `color` で塗る。
///
/// `scroll` は Paragraph に渡した行スクロール量。ビューポート外は無視する。
fn fill_rows(
    buf: &mut Buffer,
    body_area: Rect,
    scroll: usize,
    start_row: usize,
    end_row: usize,
    color: Color,
) {
    for row in start_row..end_row {
        if row < scroll {
            continue;
        }
        let y = body_area.y as usize + (row - scroll);
        if y >= body_area.bottom() as usize {
            break;
        }
        for x in body_area.left()..body_area.right() {
            if let Some(cell) = buf.cell_mut((x, y as u16)) {
                cell.set_style(cell.style().bg(color));
            }
        }
    }
}

/// 幽霊バナー（P1-1）。幽霊がある場合のみ1行分の警告を描く。
/// 既存の正常系表示は変えない（追加のみ）。
fn render_orphan_banner(
    orphaned_runs: &std::collections::HashSet<i64>,
    area: Rect,
    buf: &mut Buffer,
) {
    if orphaned_runs.is_empty() {
        return;
    }
    let mut ids: Vec<i64> = orphaned_runs.iter().copied().collect();
    ids.sort_unstable();
    let text = format!(
        "⚠ 取り残し（要確認）: run {}（`sai-link daemon recover` を参照）",
        ids.iter()
            .map(|i| i.to_string())
            .collect::<Vec<_>>()
            .join(", ")
    );
    draw::draw_text_cjk(
        buf,
        area.x + 2,
        area.y,
        &text,
        Style::default().fg(Color::Yellow),
    );
}

// ============================================================================
// MainPanelState
// ============================================================================

/// レンダリング済みマークダウン表示の一時ビュー（`show_block_content` 用）。
///
/// History モード等は `history` を使用する。編集は行わず、スクロールのみ対応。
#[derive(Debug)]
pub struct MarkdownView {
    /// スタイル付きの表示行
    pub lines: Vec<Line<'static>>,
    /// 見出し一覧（折りたたみ・outline パネルで使う）
    pub headings: Vec<crate::ui::markdown::HeadingEntry>,
    /// 折りたたみ中の見出しの `line_idx` 集合
    pub folded: std::collections::HashSet<usize>,
    /// タイトル（ブロック名・セッション名など）
    pub title: String,
    /// サブタイトル（ファイルパス・開始時刻など）
    pub subtitle: String,
    /// スクロール行数（折り返し後の表示行数）
    pub scroll: usize,
    /// カーソル行（折り返し前の表示行インデックス。折りたたみ反映後）
    pub cursor: usize,
    /// 本文ビューポートの高さ（描画時に更新。カーソル追従に使う）
    pub view_height: usize,
    /// outline パネルを表示中か
    pub outline_visible: bool,
    /// outline 側のカーソル位置（`headings` のインデックス）
    pub outline_selected: usize,
    /// フォーカスが outline パネルにあるか（`false` = 本文にフォーカス）
    pub outline_focused: bool,
}

impl MarkdownView {
    /// markdown 本文からビューを構築する（折りたたみ・outline は初期状態＝非表示）。
    pub fn from_markdown(content: &str, theme: &AppTheme, title: String, subtitle: String) -> Self {
        let (lines, headings) = crate::ui::markdown::render_markdown(content, theme);
        Self {
            lines,
            headings,
            folded: std::collections::HashSet::new(),
            title,
            subtitle,
            scroll: 0,
            cursor: 0,
            view_height: 0,
            outline_visible: false,
            outline_selected: 0,
            outline_focused: false,
        }
    }

    /// 現在のカーソル位置に最も近い（`line_idx` がカーソル位置以下で最大の）見出しを返す。
    pub fn heading_at_cursor(&self) -> Option<&crate::ui::markdown::HeadingEntry> {
        let logical = visible_line_indices(self).get(self.cursor).copied();
        let logical = match logical {
            Some(i) => i,
            None => return self.headings.first(),
        };
        self.headings
            .iter()
            .filter(|h| h.line_idx <= logical)
            .max_by_key(|h| h.line_idx)
            .or_else(|| self.headings.first())
    }

    /// 現在のカーソル位置の見出しの折りたたみを切り替える。
    pub fn toggle_fold_at_cursor(&mut self) {
        let idx = match self.heading_at_cursor() {
            Some(h) => h.line_idx,
            None => return,
        };
        if !self.folded.remove(&idx) {
            self.folded.insert(idx);
        }
        // 折りたたみ後もカーソルを表示行の範囲内に保つ
        let count = visible_line_indices(self).len();
        if count > 0 && self.cursor >= count {
            self.cursor = count - 1;
        }
        self.ensure_cursor_visible();
    }

    /// カーソルを `delta` 行移動する（表示行の範囲でクランプ・追従スクロール）。
    pub fn cursor_move(&mut self, delta: i32) {
        let count = visible_line_indices(self).len();
        if count == 0 {
            self.cursor = 0;
            return;
        }
        let max = (count - 1) as i32;
        self.cursor = ((self.cursor as i32 + delta).clamp(0, max)) as usize;
        self.ensure_cursor_visible();
    }

    /// カーソルがビューポートに入るようスクロール位置を調整する。
    pub fn ensure_cursor_visible(&mut self) {
        let h = self.view_height.max(1);
        if self.cursor < self.scroll {
            self.scroll = self.cursor;
        } else if self.cursor >= self.scroll.saturating_add(h) {
            self.scroll = self.cursor + 1 - h;
        }
    }

    /// outline パネルのカーソルを移動する（`headings` の範囲でクランプ）。
    pub fn outline_move(&mut self, delta: i32) {
        if self.headings.is_empty() {
            return;
        }
        let len = self.headings.len() as i32;
        let idx = (self.outline_selected as i32 + delta).clamp(0, len - 1);
        self.outline_selected = idx as usize;
    }

    /// outline で選択中の見出しへ本文をジャンプさせ、折りたたまれていれば展開する。
    pub fn jump_to_outline_selection(&mut self) {
        if let Some(h) = self.headings.get(self.outline_selected).cloned() {
            self.folded.remove(&h.line_idx);
            if let Some(vis) = visible_line_indices(self)
                .iter()
                .position(|&i| i == h.line_idx)
            {
                self.cursor = vis;
            }
            self.ensure_cursor_visible();
        }
        self.outline_focused = false;
    }
}

/// 折りたたみ中の見出し `h` について、「次の同レベル以下見出し」の行番号
/// （無ければ全体の行数）を返す（畳んで隠れる範囲の終端。終端自身は含まない）。
fn fold_end_line(md: &MarkdownView, h: &crate::ui::markdown::HeadingEntry) -> usize {
    md.headings
        .iter()
        .filter(|o| o.line_idx > h.line_idx && o.level <= h.level)
        .map(|o| o.line_idx)
        .min()
        .unwrap_or(md.lines.len())
}

/// 折りたたみ中の見出し `h` の直後に隠れている行数。
fn fold_hidden_line_count(md: &MarkdownView, h: &crate::ui::markdown::HeadingEntry) -> usize {
    fold_end_line(md, h)
        .min(md.lines.len())
        .saturating_sub(h.line_idx + 1)
}

/// 各論理行が折りたたみで隠れているかのフラグ列を返す。
fn hidden_line_flags(md: &MarkdownView) -> Vec<bool> {
    let mut hidden = vec![false; md.lines.len()];
    for h in &md.headings {
        if !md.folded.contains(&h.line_idx) {
            continue;
        }
        let end = fold_end_line(md, h).min(hidden.len());
        for flag in hidden.iter_mut().take(end).skip(h.line_idx + 1) {
            *flag = true;
        }
    }
    hidden
}

/// 折りたたみ反映後も表示される論理行インデックスを昇順で返す。
///
/// `visible_markdown_lines` の並びと一致する（カーソル位置の対応に使う）。
fn visible_line_indices(md: &MarkdownView) -> Vec<usize> {
    let hidden = hidden_line_flags(md);
    (0..md.lines.len()).filter(|&i| !hidden[i]).collect()
}

/// 表示行ごとの折り返し後行数を返す（ratatui の折り返しと完全に一致）。
///
/// カーソル行・コードブロック行のスクリーン行位置を正しく求めるために使う。
/// `Paragraph::line_count`（unstable-rendered-line-info）で実際の折り返し数を数える。
fn wrapped_row_counts(lines: &[Line<'static>], width: u16) -> Vec<usize> {
    if width == 0 {
        return vec![1; lines.len()];
    }
    lines
        .iter()
        .map(|line| {
            Paragraph::new(Text::from(vec![line.clone()]))
                .wrap(Wrap { trim: false })
                .line_count(width)
                .max(1)
        })
        .collect()
}

/// コードブロックの行か（先頭スパンが `code_bg` 付きのガター `│ ` か）。
fn is_code_block_line(line: &Line) -> bool {
    line.spans
        .first()
        .map(|s| s.content == "│ " && s.style.bg.is_some())
        .unwrap_or(false)
}

/// 折りたたみを反映した表示用の行一覧を返す。
///
/// 見出し行には `▼`（展開中）/ `▸`（折りたたみ中）を付ける。折りたたみ中は
/// ekphos に合わせて見出し文の後ろに隠れた行数（`(N行)`）を出す。
fn visible_markdown_lines(md: &MarkdownView) -> Vec<Line<'static>> {
    if md.headings.is_empty() {
        return md.lines.clone();
    }

    let mut lines = md.lines.clone();
    for h in &md.headings {
        let Some(line) = lines.get_mut(h.line_idx) else {
            continue;
        };
        let mut spans = Vec::new();
        if md.folded.contains(&h.line_idx) {
            spans.push(Span::styled("▸ ", Style::default().fg(Color::White)));
            spans.extend(line.spans.iter().cloned());
            let hidden_count = fold_hidden_line_count(md, h);
            spans.push(Span::styled(
                format!("  ({}行)", hidden_count),
                Style::default().fg(Color::White),
            ));
        } else {
            spans.push(Span::styled("▼ ", Style::default().fg(Color::White)));
            spans.extend(line.spans.iter().cloned());
        }
        *line = Line::from(spans);
    }

    if md.folded.is_empty() {
        return lines;
    }

    let hidden = hidden_line_flags(md);
    lines
        .into_iter()
        .enumerate()
        .filter(|(i, _)| !hidden[*i])
        .map(|(_, l)| l)
        .collect()
}

/// md viewer の outline パネル（見出し一覧）を描画する。
fn render_markdown_outline(md: &MarkdownView, area: Rect, buf: &mut Buffer, theme: &AppTheme) {
    let border_color = if md.outline_focused {
        Color::Yellow
    } else {
        theme.muted
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(border_color).bg(theme.main_bg))
        .title(" outline ")
        .title_style(Style::default().fg(border_color).bg(theme.main_bg))
        .style(Style::default().bg(theme.main_bg));
    let inner = block.inner(area);
    block.render(area, buf);

    let items: Vec<ListItem> = md
        .headings
        .iter()
        .map(|h| {
            let indent = "  ".repeat(h.level.saturating_sub(1) as usize);
            ListItem::new(format!("{}{}", indent, h.title))
        })
        .collect();

    let mut state = ListState::default();
    if !md.headings.is_empty() {
        state.select(Some(md.outline_selected.min(md.headings.len() - 1)));
    }

    let list = List::new(items)
        .style(Style::default().fg(theme.fg).bg(theme.main_bg))
        .highlight_style(Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD));

    StatefulWidget::render(list, inner, buf, &mut state);
}

/// メイン画面の状態。
#[derive(Debug)]
pub struct MainPanelState {
    /// ブロック ID → ノード位置（ビューポート計算後）
    pub node_positions: HashMap<i64, NodePosition>,
    /// 選択中ブロックの ID
    pub selected_block_id: Option<i64>,
    /// グラフビューポート
    pub viewport: GraphViewport,
    /// キャッシュ: レイアウト計算時のブロック数
    pub cached_block_count: usize,
    /// キャッシュ: レイアウト計算時の接続数
    pub cached_conn_count: usize,
    /// キャッシュ: 最後のレイアウト結果（ノード位置）
    pub cached_layout_nodes: Vec<NodePosition>,
    /// キャッシュ: レイアウトのコンテンツサイズ
    pub cached_content_size: (i32, i32),
    /// キャッシュ: レイアウトの区切り線
    pub cached_separators: Vec<crate::graph_layout::SeparatorLine>,

    // ── card / 区切り線の手動配置（ドラッグ） ──
    /// card のドラッグによるズレ量（exec_id → ズーム 1.0 基準の (dx, dy)）。
    /// `.sai/ui-layout.json` に永続化する（起動時に読み込み、drag 終了時に保存）。
    pub node_offsets: HashMap<i64, (i32, i32)>,
    /// round 区切り線のドラッグによるズレ量（global_round → dy）。
    /// その GR 以降の全ノード・区切り線に累積で効く。
    pub separator_offsets: HashMap<i64, i32>,
    /// 毎フレーム描画時に収集する card の当たり判定表（exec_id, スクリーン座標の矩形）。
    /// 描画順（後に描いたものが手前）のまま入る。マウスのヒットテストに使う。
    pub node_screen_rects: Vec<(i64, ratatui::layout::Rect)>,
    /// 毎フレーム描画時に収集する round 区切り線ラベルの当たり判定表
    /// （global_round, スクリーン座標の矩形）。線全体ではなくラベル部分のみ。
    pub separator_screen_rects: Vec<(i64, ratatui::layout::Rect)>,

    // ── マークダウン表示 ──
    /// 一時的なマークダウン表示（`show_block_content` 用・スクロール対応）。
    /// History モード等では `history` を使用する。
    pub md_view: Option<MarkdownView>,
    /// Project ダッシュボード（ratatui ウィジェット描画）。None = 非表示。
    ///
    /// Project モード中は md_view ではなくこちらを表示する。
    /// スクロールは `md_scroll` と同じ枠組み（j/k / PgUp/PgDn / ホイール）。
    pub project_dash: Option<sai_link_widgets::project_dashboard::ProjectDashData>,
    /// History 画面（ratatui ネイティブ描画・近似値）。None = 非表示。
    ///
    /// History モード中は md_view ではなくこちらを表示する。
    pub history: Option<crate::ui::history_view::HistoryViewData>,
    /// ズーム倍率（1.0 = 100%）。Ctrl+スクロール / `+` `-` キーで変更する
    pub zoom: f64,
    /// 完了実行の費用表示（B02。exec_id → `$0.42` / `費用不明`）。
    ///
    /// `load_data` で 1 回だけ `execution_usage` を引き、描画時は DB を触らない。
    /// Workflow の Card（`BlockNodeData.cost`）へ渡す。
    pub cost_labels: HashMap<i64, String>,
}

impl Default for MainPanelState {
    fn default() -> Self {
        Self {
            node_positions: HashMap::new(),
            selected_block_id: None,
            viewport: GraphViewport::default(),
            cached_block_count: 0,
            cached_conn_count: 0,
            cached_layout_nodes: Vec::new(),
            cached_content_size: (0, 0),
            cached_separators: Vec::new(),
            node_offsets: HashMap::new(),
            separator_offsets: HashMap::new(),
            node_screen_rects: Vec::new(),
            separator_screen_rects: Vec::new(),
            md_view: None,
            project_dash: None,
            history: None,
            zoom: 1.0,
            cost_labels: HashMap::new(),
        }
    }
}

// ============================================================================
// MainPanel widget
// ============================================================================

/// メイン画面ウィジェット。
pub struct MainPanel<'a> {
    pub state: &'a mut MainPanelState,
    pub blocks: &'a [BlockDisplayData],
    pub connections: &'a [ConnectionDisplayData],
    /// sai-write 検出済み（finalized 実行バッチを持つ）実メイン実行 ID の集合。
    /// 最終フェーズノード（P_n = sai-write 由来）を合成するかどうかの判定に使用。
    pub finalized_runs: &'a std::collections::HashSet<i64>,
    /// 幽霊（orphaned_run）の実行 ID 集合（P1-1）。
    /// 該当ノード名に「要確認」接尾辞を付ける（色は変えない）。空なら表示不変。
    pub orphaned_runs: &'a std::collections::HashSet<i64>,
    pub theme: &'a AppTheme,
}

impl Widget for MainPanel<'_> {
    fn render(mut self, area: Rect, buf: &mut Buffer) {
        let bg = self.theme.main_bg;

        // 背景
        for y in area.top()..area.bottom() {
            for x in area.left()..area.right() {
                if let Some(cell) = buf.cell_mut((x, y)) {
                    cell.set_style(Style::default().bg(bg));
                }
            }
        }

        // Project ダッシュボード（ratatui ネイティブウィジェット描画）。
        // md_view より優先して表示する。
        if self.state.project_dash.is_some() {
            let theme = self.theme;
            let dash = self.state.project_dash.as_mut().unwrap();
            sai_link_widgets::project_dashboard::render_dashboard(area, buf, dash, theme);
            return;
        }

        // History 画面（ratatui ネイティブ描画・近似値）
        if self.state.history.is_some() {
            let theme = self.theme;
            let hist = self.state.history.as_mut().unwrap();
            crate::ui::history_view::render_history_view(area, buf, hist, theme);
            return;
        }

        // History / Blocks / Reflection 閲覧モード: マークダウン表示（レンダリング専用）
        if self.state.md_view.is_some() {
            self.render_markdown_view(area, buf);
            return;
        }

        if self.blocks.is_empty() && self.orphaned_runs.is_empty() {
            // ブロックなしメッセージ
            let msg = "No block definitions found.";
            draw::draw_text_cjk(
                buf,
                area.x + 2,
                area.y + 1,
                msg,
                Style::default().fg(self.theme.muted),
            );
            return;
        }

        // Workflow モード: DAG グラフ描画
        // 幽霊がある場合は上端に警告バナーを1行描く（P1-1・追加のみ）。
        if !self.orphaned_runs.is_empty() && area.height > 1 {
            let banner = Rect::new(area.x, area.y, area.width, 1);
            render_orphan_banner(self.orphaned_runs, banner, buf);
            let rest = Rect::new(
                area.x,
                area.y + 1,
                area.width,
                area.height.saturating_sub(1),
            );
            self.render_dag(rest, buf);
            return;
        }
        self.render_dag(area, buf);
    }
}

impl<'a> MainPanel<'a> {
    /// マークダウン表示を描画する（History / Blocks モード）。
    fn render_markdown_view(&mut self, area: Rect, buf: &mut Buffer) {
        let info = self.theme.info;
        let muted = self.theme.muted;
        let bg = self.theme.main_bg;

        // タイトル行
        let title = {
            let md = self.state.md_view.as_ref().unwrap();
            format!(" {}: {}", md.title, md.subtitle)
        };
        draw::draw_text_cjk(
            buf,
            area.x + 1,
            area.y,
            &title,
            Style::default().fg(info).bg(bg),
        );

        // セパレータ
        for x in area.x..area.x + area.width {
            buf.cell_mut((x, area.y + 1))
                .map(|c| c.set_char('─').set_style(Style::default().fg(muted)));
        }

        // 本文（Paragraph + 行スクロール。タイトル2行とガイド1行分を除く）
        let full_body_area = Rect::new(
            area.x + 1,
            area.y + 2,
            area.width.saturating_sub(2),
            area.height.saturating_sub(3),
        );

        let md = self.state.md_view.as_ref().unwrap();
        let show_outline = md.outline_visible && !md.headings.is_empty();

        // outline 表示中は md viewer の描画領域だけをローカルに左右分割する
        // （AppLayout/compute_layout は変更しない）。
        let (body_area, outline_area) = if show_outline {
            let cols = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Min(20), Constraint::Length(24)])
                .split(full_body_area);
            (cols[0], Some(cols[1]))
        } else {
            (full_body_area, None)
        };

        // 折りたたみ反映後の表示行・カーソル・スクロール位置を取得し、
        // ビューポート高さを更新する（カーソル追従に使う）。
        let (display_lines, cursor, scroll, headings_empty) = {
            let md = self.state.md_view.as_mut().unwrap();
            md.view_height = body_area.height as usize;
            (
                visible_markdown_lines(md),
                md.cursor,
                md.scroll,
                md.headings.is_empty(),
            )
        };

        // 折り返し後のスクリーン行位置（カーソル行・コードブロック行の背景用）。
        let heights = wrapped_row_counts(&display_lines, body_area.width);
        let mut row_offsets: Vec<usize> = Vec::with_capacity(heights.len() + 1);
        let mut acc = 0usize;
        row_offsets.push(0);
        for h in &heights {
            acc += *h;
            row_offsets.push(acc);
        }
        // コードブロック行（先頭スパンが code_bg 付きガターの行）
        let code_line_idx: Vec<usize> = display_lines
            .iter()
            .enumerate()
            .filter(|(_, l)| is_code_block_line(l))
            .map(|(i, _)| i)
            .collect();

        // scroll は u16 に収まる範囲でクランプ（超過分は Paragraph 側で調整される）
        let scroll_y = scroll.min(u16::MAX as usize) as u16;
        // 長い行は右 SB と重ならないよう、body 幅で折り返す（ratatui は unicode-width 基準）。
        let paragraph = Paragraph::new(Text::from(display_lines))
            .style(Style::default().bg(bg))
            .wrap(Wrap { trim: false })
            .scroll((scroll_y, 0));
        Widget::render(&paragraph, body_area, buf);

        // コードブロック行は行全体の背景色を揃える（ekphos と同じ）。
        // 折り返しで複数行になる場合は折り返し後の全行を塗る。
        let code_bg = self.theme.code_bg;
        for i in code_line_idx {
            let start = row_offsets.get(i).copied().unwrap_or(0);
            let end = row_offsets.get(i + 1).copied().unwrap_or(start + 1);
            fill_rows(buf, body_area, scroll, start, end, code_bg);
        }

        // カーソル行は行全体をハイライトする（ekphos の `▶` 相当）。
        if cursor < row_offsets.len().saturating_sub(1) {
            let start = row_offsets[cursor];
            let end = row_offsets[cursor + 1];
            fill_rows(buf, body_area, scroll, start, end, MD_CURSOR_BG);
        }

        if let Some(outline_area) = outline_area {
            let md = self.state.md_view.as_ref().unwrap();
            render_markdown_outline(md, outline_area, buf, self.theme);
        }

        // キー操作ガイド（下部）
        let guide = if headings_empty {
            "↑↓/jk/PgUp/PgDn move | Esc close"
        } else {
            "↑↓/jk move | space/z fold | PgUp/PgDn page | o outline | Tab focus | Esc close"
        };
        draw::draw_text_cjk(
            buf,
            area.x + 1,
            area.bottom().saturating_sub(1),
            guide,
            Style::default().fg(muted).bg(bg),
        );
    }

    /// Workflow モード: DAG グラフを描画する。
    fn render_dag(&mut self, area: Rect, buf: &mut Buffer) {
        let bg = self.theme.main_bg;

        // グラフを構築してレイアウト
        // 幽霊ノードは色を変えず、表示名に「要確認」接尾辞のみ付ける（P1-1）。
        let orphaned = self.orphaned_runs;
        let blocks: Vec<BlockDisplayData> = self
            .blocks
            .iter()
            .map(|b| {
                if orphaned.contains(&b.exec_id) && b.phase.is_none() {
                    let mut marked = b.clone();
                    marked.name = format!("{}（要確認）", b.name);
                    marked
                } else {
                    b.clone()
                }
            })
            .collect();
        let (dag, layout_graph) =
            build_block_graph(&blocks, self.connections, self.finalized_runs);

        // データが変わった場合のみレイアウト再計算
        let block_count = self.blocks.len();
        let conn_count = self.connections.len();
        let layout_changed = block_count != self.state.cached_block_count
            || conn_count != self.state.cached_conn_count;

        let layout = if layout_changed {
            let l = calculate_layout(&layout_graph, true);
            self.state.cached_layout_nodes = l.nodes.clone();
            self.state.cached_content_size = (l.content_width, l.content_height);
            self.state.cached_separators = l.separators.clone();
            self.state.cached_block_count = block_count;
            self.state.cached_conn_count = conn_count;
            l
        } else {
            // キャッシュから復元（DagLayout を再構築）
            crate::graph_layout::DagLayout {
                nodes: self.state.cached_layout_nodes.clone(),
                content_width: self.state.cached_content_size.0,
                content_height: self.state.cached_content_size.1,
                separators: self.state.cached_separators.clone(),
            }
        };

        // card ドラッグ・round 区切り線ドラッグのズレ量を適用してからズームでスケールする。
        // キャッシュ（cached_layout_nodes 等）はズレ量を含まない素のレイアウトのまま保つ
        // （ズレ量を変えるたびに calculate_layout を再実行しないため）。
        let layout = apply_layout_offsets(
            &layout,
            &self.state.node_offsets,
            &self.state.separator_offsets,
        );
        // キャッシュは基本（ズーム 1.0）のまま、描画用にズーム倍率でスケールする
        let layout = scale_layout(&layout, self.state.zoom);

        // パネルの境界線（┌│┘）はパネル矩形の最外周に描画されるため、
        // コンテンツ（DAG・ノード）は枠の 1 セル内側に限定して描画する。
        // こうしないとノードの左端や上端が境界線に重なって見切れる。
        let viewport_area = Rect::new(
            area.x + 1,
            area.y + 1,
            area.width.saturating_sub(2),
            area.height.saturating_sub(2),
        );
        let content_left = viewport_area.x as i32;
        let content_top = viewport_area.y as i32;
        let content_right = viewport_area.right() as i32;
        let content_bottom = viewport_area.bottom() as i32;

        // ビューポートを更新
        let selected_idx = self
            .state
            .selected_block_id
            .and_then(|id| dag.index_of(id as u64));

        let (scroll_x, scroll_y) = if self.state.viewport.manual_scroll {
            (self.state.viewport.scroll_x, self.state.viewport.scroll_y)
        } else {
            let target = selected_idx.or(dag.root).and_then(|idx| layout.find(idx));

            if let Some(t) = target {
                center_on(
                    t,
                    layout.content_width,
                    layout.content_height,
                    viewport_area,
                )
            } else {
                (0, 0)
            }
        };

        self.state.viewport.scroll_x = scroll_x;
        self.state.viewport.scroll_y = scroll_y;

        // 区切り線（roundN）を先に描画。ラベル部分の当たり判定（drag 用）を収集する。
        self.state.separator_screen_rects = render_separators(
            &layout.separators,
            layout.content_width,
            scroll_x,
            scroll_y,
            viewport_area,
            buf,
        );

        // エッジを描画（card の辺には触れない。選択中ブロックの接続線はハイライトする）。
        render_edges(
            &dag,
            &layout,
            scroll_x,
            scroll_y,
            true,
            self.state.selected_block_id.map(|id| id as u64),
            viewport_area,
            buf,
        );

        // ノード位置を保存しつつ描画。card の当たり判定表（drag 用）は毎フレーム作り直す。
        let mut positions = HashMap::new();
        self.state.node_screen_rects.clear();
        for pos in &layout.nodes {
            positions.insert(pos.node_id as i64, *pos);

            // ノード描画（P3-B2: 合成フェーズノードは dag から取得）
            if let Some(block) = dag.get(pos.index) {
                // 選択判定: 実メイン（mb1-1）に加え、フェーズノード（mb1-2 / mb1-3 等）も
                // 同じ実メインの選択として色を変える（左SB でメインブロック選択時）。
                let is_selected = is_node_selected(self.state.selected_block_id, block);
                let node_data = BlockNodeData {
                    name: block.name.clone(),
                    tag: block.tag.clone(),
                    exec_id: block.exec_id,
                    runner: block.runner.clone(),
                    created_at: block.created_at.clone(),
                    description: block.description.clone(),
                    selected: is_selected,
                    status_color: None,
                    cost: self.state.cost_labels.get(&block.exec_id).cloned(),
                };

                // スクリーン座標（クランプなし、パネル外も許容）
                // コンテンツ基準（枠の内側）からオフセットする
                let sx = pos.x + content_left - scroll_x;
                let sy = pos.y + content_top - scroll_y;

                // 可視判定: ノードの矩形がコンテンツ領域と交差するか
                let node_left = sx;
                let node_top = sy;
                let node_right = sx + pos.width as i32;
                let node_bottom = sy + pos.height as i32;

                if node_right > content_left
                    && node_left < content_right
                    && node_bottom > content_top
                    && node_top < content_bottom
                {
                    // ヒットテスト用の当たり判定（画面クリップ範囲へ寄せる。
                    // 一部が枠外に出ていても、見えている範囲はクリックで拾えるようにする）
                    let rx = sx.max(i32::from(viewport_area.x));
                    let ry = sy.max(i32::from(viewport_area.y));
                    let r_right = node_right.min(i32::from(viewport_area.right()));
                    let r_bottom = node_bottom.min(i32::from(viewport_area.bottom()));
                    if r_right > rx && r_bottom > ry {
                        self.state.node_screen_rects.push((
                            pos.node_id as i64,
                            Rect::new(
                                rx as u16,
                                ry as u16,
                                (r_right - rx) as u16,
                                (r_bottom - ry) as u16,
                            ),
                        ));
                    }

                    // オフスクリーンバッファにノードを描画（原点(0,0)基準）
                    let off_rect = Rect::new(0, 0, pos.width, pos.height);
                    let mut node_buf = Buffer::empty(off_rect);
                    BlockNodeWidget::new(&node_data, bg).render(off_rect, &mut node_buf);

                    // スクリーン座標でメインバッファにコピー（コンテンツ領域内のみ）
                    for ny in 0..pos.height {
                        for nx in 0..pos.width {
                            let dx = sx + nx as i32;
                            let dy = sy + ny as i32;
                            if dx >= content_left
                                && dx < content_right
                                && dy >= content_top
                                && dy < content_bottom
                            {
                                if let Some(src_cell) = node_buf.cell((nx, ny)).cloned() {
                                    if let Some(cell) = buf.cell_mut((dx as u16, dy as u16)) {
                                        // ノード本体で常に上書きする
                                        // （エッジ線がノード内部に見えるのを防ぐ）
                                        *cell = src_cell;
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        self.state.node_positions = positions;
    }
}


/// ノードの選択判定。
///
/// 実メイン（mb1-1）に加え、フェーズノード（mb1-2 / mb1-3 等の合成小ブロック）も
/// 同じ実メイン（フェーズ親）の選択として扱い、色を変える。
fn is_node_selected(selected: Option<i64>, block: &BlockDisplayData) -> bool {
    selected == Some(block.exec_id) || (block.phase.is_some() && selected == block.parent_exec_id)
}

fn center_on(target: &NodePosition, cw: i32, ch: i32, vp: Rect) -> (i32, i32) {
    let cx = target.x + i32::from(target.width) / 2;
    let cy = target.y + i32::from(target.height) / 2;
    let mx = (cw - i32::from(vp.width)).max(0);
    let my = (ch - i32::from(vp.height)).max(0);
    (
        (cx - i32::from(vp.width) / 2).clamp(0, mx),
        (cy - i32::from(vp.height) / 2).clamp(0, my),
    )
}

/// card のドラッグ・round 区切り線のドラッグによるズレ量をレイアウトへ適用する。
///
/// - `node_offsets`: ノード（exec_id）ごとの個別ズレ量
/// - `separator_offsets`: round（global_round）ごとの縦方向ズレ量。その round
///   以降の全ノード・全区切り線に**累積**で効く（round の昇順に積み上げていく。
///   round3 を下へ引くと round3 以降がまとめて下へ移動し、余白が広がる）
///
/// `calculate_layout` の結果（キャッシュされた基本レイアウト）は変えず、
/// 描画直前にこの関数でズレ量を足す。適用後は `content_width` /
/// `content_height` を再計算し、負方向へはみ出す場合は全体を平行移動して
/// 0 以上に正規化する。
fn apply_layout_offsets(
    layout: &crate::graph_layout::DagLayout,
    node_offsets: &HashMap<i64, (i32, i32)>,
    separator_offsets: &HashMap<i64, i32>,
) -> crate::graph_layout::DagLayout {
    if node_offsets.is_empty() && separator_offsets.is_empty() {
        return layout.clone();
    }

    // 区切り線ごとの累積 dy（y 昇順 = GR 昇順で積み上げる）。
    let mut seps = layout.separators.clone();
    seps.sort_by_key(|s| s.y);
    let mut cum = 0i32;
    let cum_by_sep_y: Vec<(i32, i32)> = seps
        .iter()
        .map(|s| {
            cum += separator_offsets.get(&s.global_round).copied().unwrap_or(0);
            (s.y, cum)
        })
        .collect();

    // 指定 y（オフセット適用前の元座標）が属する区切り線グループの累積 dy。
    // 該当する区切り線が無ければ（レガシーレイアウト等）0。
    let separator_dy_for = |y: i32| -> i32 {
        cum_by_sep_y
            .iter()
            .rev()
            .find(|(sep_y, _)| *sep_y <= y)
            .map(|(_, dy)| *dy)
            .unwrap_or(0)
    };

    let nodes: Vec<NodePosition> = layout
        .nodes
        .iter()
        .map(|p| {
            let (ndx, ndy) = node_offsets
                .get(&(p.node_id as i64))
                .copied()
                .unwrap_or((0, 0));
            NodePosition {
                x: p.x + ndx,
                y: p.y + ndy + separator_dy_for(p.y),
                ..*p
            }
        })
        .collect();
    let separators: Vec<crate::graph_layout::SeparatorLine> = seps
        .into_iter()
        .zip(cum_by_sep_y.iter())
        .map(|(s, &(_, dy))| crate::graph_layout::SeparatorLine {
            y: s.y + dy,
            ..s
        })
        .collect();

    // 負方向にはみ出す場合は全体を平行移動して 0 以上に正規化する。
    let min_x = nodes.iter().map(|p| p.x).chain(std::iter::once(0)).min().unwrap();
    let min_y = nodes
        .iter()
        .map(|p| p.y)
        .chain(separators.iter().map(|s| s.y))
        .chain(std::iter::once(0))
        .min()
        .unwrap();
    let (shift_x, shift_y) = (-min_x, -min_y);

    let nodes: Vec<NodePosition> = nodes
        .into_iter()
        .map(|p| NodePosition {
            x: p.x + shift_x,
            y: p.y + shift_y,
            ..p
        })
        .collect();
    let separators: Vec<crate::graph_layout::SeparatorLine> = separators
        .into_iter()
        .map(|s| crate::graph_layout::SeparatorLine {
            y: s.y + shift_y,
            ..s
        })
        .collect();

    let content_width = nodes
        .iter()
        .map(|p| p.x + i32::from(p.width))
        .max()
        .unwrap_or(0);
    let content_height = nodes
        .iter()
        .map(|p| p.y + i32::from(p.height))
        .max()
        .unwrap_or(0);

    crate::graph_layout::DagLayout {
        nodes,
        content_width,
        content_height,
        separators,
    }
}

/// レイアウト全体をズーム倍率でスケールする。
///
/// ノード位置・サイズ・区切り線・コンテンツサイズを倍率で変換する。
/// キャッシュ（基本レイアウト）はズームに依存せず、毎フレームここで変換するため
/// 拡大縮小を繰り返しても座標の丸め誤差が累積しない。
fn scale_layout(
    layout: &crate::graph_layout::DagLayout,
    zoom: f64,
) -> crate::graph_layout::DagLayout {
    if (zoom - 1.0).abs() < f64::EPSILON {
        return layout.clone();
    }
    let scale = |v: i32| ((v as f64) * zoom).round() as i32;
    crate::graph_layout::DagLayout {
        nodes: layout
            .nodes
            .iter()
            .map(|p| NodePosition {
                index: p.index,
                node_id: p.node_id,
                x: scale(p.x),
                y: scale(p.y),
                width: (((p.width as f64) * zoom).round() as u16).max(1),
                height: (((p.height as f64) * zoom).round() as u16).max(1),
            })
            .collect(),
        content_width: scale(layout.content_width),
        content_height: scale(layout.content_height),
        separators: layout
            .separators
            .iter()
            .map(|s| crate::graph_layout::SeparatorLine {
                y: scale(s.y),
                label: s.label.clone(),
                global_round: s.global_round,
            })
            .collect(),
    }
}
// ============================================================================
// DAG 描画の回帰テスト（docs/block-position-rule.md 準拠）
// ============================================================================

#[cfg(test)]
mod dag_rendering_tests {
    use super::*;
    use crate::widgets::block_node::{BlockNodeData, BlockNodeWidget};
    use ratatui::buffer::Buffer as TuiBuffer;
    use sai_link_widgets::theme::AppTheme;

    /// テスト用ブロック: 7/8 は同ブロックの実行、9 は別ブロック、11/12 は 9 のサブ。
    fn sample_blocks() -> Vec<BlockDisplayData> {
        vec![
            BlockDisplayData {
                exec_id: 7,
                block_id: 1,
                name: "brst-grill".into(),
                tag: Some("brst".into()),
                global_round: Some(1),
                round: Some(1),
                parent_exec_id: None,
                batch_id: None,
                phase: None,
                runner: "pi".into(),
                last_status: "completed".into(),
                reflection_status: None,
                last_duration_ms: Some(1000),
                created_at: "2026-08-01T10:00:00".into(),
                description: String::new(),
            },
            BlockDisplayData {
                exec_id: 8,
                block_id: 1,
                name: "brst-grill".into(),
                tag: Some("brst".into()),
                global_round: Some(1),
                round: Some(1),
                parent_exec_id: None,
                batch_id: None,
                phase: None,
                runner: "pi".into(),
                last_status: "completed".into(),
                reflection_status: None,
                last_duration_ms: Some(1000),
                created_at: "2026-08-01T10:01:00".into(),
                description: String::new(),
            },
            BlockDisplayData {
                exec_id: 9,
                block_id: 2,
                name: "plan-ice".into(),
                tag: Some("plan".into()),
                global_round: Some(1),
                round: Some(2),
                parent_exec_id: None,
                batch_id: None,
                phase: None,
                runner: "pi".into(),
                last_status: "completed".into(),
                reflection_status: None,
                last_duration_ms: Some(1000),
                created_at: "2026-08-01T10:02:00".into(),
                description: String::new(),
            },
            BlockDisplayData {
                exec_id: 10,
                block_id: 1,
                name: "brst-grill".into(),
                tag: Some("brst".into()),
                global_round: Some(2),
                round: Some(1),
                parent_exec_id: None,
                batch_id: None,
                phase: None,
                runner: "pi".into(),
                last_status: "completed".into(),
                reflection_status: None,
                last_duration_ms: Some(1000),
                created_at: "2026-08-01T10:03:00".into(),
                description: String::new(),
            },
            // サブブロック（exec 9 の子）
            BlockDisplayData {
                exec_id: 11,
                block_id: 3,
                name: "sub-task".into(),
                tag: Some("rfrs".into()),
                global_round: Some(1),
                round: Some(2),
                parent_exec_id: Some(9),
                batch_id: None,
                phase: None,
                runner: "pi".into(),
                last_status: "completed".into(),
                reflection_status: None,
                last_duration_ms: Some(500),
                created_at: "2026-08-01T10:04:00".into(),
                description: String::new(),
            },
            BlockDisplayData {
                exec_id: 12,
                block_id: 3,
                name: "sub-task".into(),
                tag: Some("rfrs".into()),
                global_round: Some(1),
                round: Some(2),
                parent_exec_id: Some(9),
                batch_id: None,
                phase: None,
                runner: "pi".into(),
                last_status: "completed".into(),
                reflection_status: None,
                last_duration_ms: Some(500),
                created_at: "2026-08-01T10:05:00".into(),
                description: String::new(),
            },
        ]
    }

    fn sample_connections() -> Vec<ConnectionDisplayData> {
        vec![
            ConnectionDisplayData {
                source_execution_id: 7,
                target_execution_id: 9,
                connection_type: "waggle_token".into(),
            },
            ConnectionDisplayData {
                source_execution_id: 9,
                target_execution_id: 11,
                connection_type: "task_delegation".into(),
            },
            ConnectionDisplayData {
                source_execution_id: 9,
                target_execution_id: 12,
                connection_type: "task_delegation".into(),
            },
            // task_output（sub→main）: 相互接続が1本の線で表現される
            ConnectionDisplayData {
                source_execution_id: 11,
                target_execution_id: 9,
                connection_type: "task_output".into(),
            },
            ConnectionDisplayData {
                source_execution_id: 12,
                target_execution_id: 9,
                connection_type: "task_output".into(),
            },
        ]
    }

    /// P3-B2: フェーズ合成のテストデータ。
    /// メイン exec 30 に batch_id 付きサブを持つ（バッチ batch_1_r1: 31, 32 / batch_1_r2: 33）。
    fn phase_blocks() -> Vec<BlockDisplayData> {
        vec![
            BlockDisplayData {
                exec_id: 30,
                block_id: 1,
                name: "main-block".into(),
                tag: Some("plan".into()),
                global_round: Some(1),
                round: Some(1),
                parent_exec_id: None,
                batch_id: None,
                phase: None,
                runner: "pi".into(),
                last_status: "completed".into(),
                reflection_status: None,
                last_duration_ms: Some(1000),
                created_at: "2026-08-01T10:00:00".into(),
                description: String::new(),
            },
            BlockDisplayData {
                exec_id: 31,
                block_id: 3,
                name: "sub-task".into(),
                tag: Some("code".into()),
                global_round: Some(1),
                round: Some(1),
                parent_exec_id: Some(30),
                batch_id: Some("batch_1_r1".into()),
                phase: None,
                runner: "pi".into(),
                last_status: "completed".into(),
                reflection_status: None,
                last_duration_ms: Some(500),
                created_at: "2026-08-01T10:01:00".into(),
                description: String::new(),
            },
            BlockDisplayData {
                exec_id: 32,
                block_id: 3,
                name: "sub-task".into(),
                tag: Some("code".into()),
                global_round: Some(1),
                round: Some(1),
                parent_exec_id: Some(30),
                batch_id: Some("batch_1_r1".into()),
                phase: None,
                runner: "pi".into(),
                last_status: "completed".into(),
                reflection_status: None,
                last_duration_ms: Some(500),
                created_at: "2026-08-01T10:02:00".into(),
                description: String::new(),
            },
            BlockDisplayData {
                exec_id: 33,
                block_id: 3,
                name: "sub-task".into(),
                tag: Some("code".into()),
                global_round: Some(1),
                round: Some(1),
                parent_exec_id: Some(30),
                batch_id: Some("batch_1_r2".into()),
                phase: None,
                runner: "pi".into(),
                last_status: "completed".into(),
                reflection_status: None,
                last_duration_ms: Some(500),
                created_at: "2026-08-01T10:03:00".into(),
                description: String::new(),
            },
        ]
    }

    fn phase_connections() -> Vec<ConnectionDisplayData> {
        vec![
            ConnectionDisplayData {
                source_execution_id: 30,
                target_execution_id: 31,
                connection_type: "task_delegation".into(),
            },
            ConnectionDisplayData {
                source_execution_id: 30,
                target_execution_id: 32,
                connection_type: "task_delegation".into(),
            },
            ConnectionDisplayData {
                source_execution_id: 30,
                target_execution_id: 33,
                connection_type: "task_delegation".into(),
            },
            // task_output（sub→main）: フェーズノードへの読み替え対象
            ConnectionDisplayData {
                source_execution_id: 31,
                target_execution_id: 30,
                connection_type: "task_output".into(),
            },
            ConnectionDisplayData {
                source_execution_id: 32,
                target_execution_id: 30,
                connection_type: "task_output".into(),
            },
            ConnectionDisplayData {
                source_execution_id: 33,
                target_execution_id: 30,
                connection_type: "task_output".into(),
            },
        ]
    }

    /// P3-B2 改: batch_id を持つサブがあるメインは、フェーズノード（合成小ブロック）が
    /// 生成されること。ただし最終フェーズ（P_n = sai-write 由来）は finalized のときのみ。
    #[test]
    fn phases_are_synthesized_for_batched_subs() {
        let blocks = phase_blocks();
        let connections = phase_connections();
        // sai-write 未検出（finalized なし）: バッチ数 2 → P_1 のみ（P_2 = sai-write 由来は未合成）
        let (dag, layout_graph) =
            build_block_graph(&blocks, &connections, &std::collections::HashSet::new());
        let layout = calculate_layout(&layout_graph, true);

        use petgraph::visit::EdgeRef;
        let has_edge = |from: u64, to: u64| -> bool {
            let from_idx = dag.index_of(from).unwrap();
            dag.graph
                .edges_directed(from_idx, petgraph::Direction::Outgoing)
                .any(|e| e.target() == dag.index_of(to).unwrap())
        };

        // 合成ノードの ID: 30 * 10_000 + k
        let p1 = 30u64 * 10_000 + 1; // フェーズ 1（バッチ2 への委譲・バッチ1 の受領）
        let p2 = 30u64 * 10_000 + 2; // フェーズ 2（sai-write 由来・finalized 時のみ）
        assert!(
            dag.index_of(p1).is_some(),
            "フェーズ1 の合成ノードが生成されるべき"
        );
        assert!(
            !dag.index_of(p2).is_some(),
            "sai-write 未検出では最終フェーズは生成されないべき"
        );

        // 委譲: フェーズ0(=30) → バッチ1 の先頭（31）、バッチ1 内はチェーン（31 → 32）、
        // フェーズ1 → バッチ2（33）。メインから 2 番目以降へは直接接続しない（貫通防止）。
        assert!(has_edge(30, 31), "フェーズ0 → バッチ1 の先頭サブへ委譲");
        assert!(has_edge(31, 32), "バッチ1 内はチェーン接続（sb1 → sb2）");
        assert!(
            !has_edge(30, 32),
            "フェーズ0 から 2 番目以降へは直接接続しない"
        );
        assert!(has_edge(p1, 33), "フェーズ1 → バッチ2 の委譲");

        // task_output: バッチ1 → フェーズ1。バッチ2 は受領先が未合成のため接続線なし
        assert!(
            has_edge(31, p1) && has_edge(32, p1),
            "バッチ1 → フェーズ1 の task_output"
        );
        assert!(
            !has_edge(33, p2),
            "最終バッチは sai-write 未検出では接続線を引かない"
        );

        // 縦接続鎖: mb1 → P_1
        assert!(has_edge(30, p1), "メインからフェーズ1への縦接続鎖");

        // レイアウト: フェーズノードはメインの下に縦に並ぶ
        let m = layout.nodes.iter().find(|p| p.node_id == 30).unwrap();
        let f1 = layout.nodes.iter().find(|p| p.node_id == p1).unwrap();
        assert!(f1.y > m.y, "フェーズ1 はメインの下");

        // バッチ1 のサブはフェーズ0 の右側、バッチ2 のサブはフェーズ1 の右側
        let s31 = layout.nodes.iter().find(|p| p.node_id == 31).unwrap();
        let s33 = layout.nodes.iter().find(|p| p.node_id == 33).unwrap();
        assert!(s31.x > m.x + i32::from(m.width), "バッチ1 はメインの右側");
        assert!(
            s33.x > f1.x + i32::from(f1.width),
            "バッチ2 はフェーズ1 の右側"
        );
        assert!(s31.y >= m.y, "バッチ1 はフェーズ0 と同じ行");
        assert!(s33.y >= f1.y, "バッチ2 はフェーズ1 と同じ行");

        // フェーズ表示名: メイン実行 ID / batch_id
        let f1_block = dag.get(f1.index).unwrap();
        assert!(
            f1_block.name.starts_with("id30/"),
            "フェーズ名は id30/... 形式: {}",
            f1_block.name
        );
    }

    /// sai-write 検出済み（finalized 実行バッチあり）の場合、最終フェーズノード
    /// （P_n）が合成され、最終バッチの task_output 接続がそこへ向くこと。
    #[test]
    fn final_phase_is_synthesized_when_finalized() {
        let blocks = phase_blocks();
        let connections = phase_connections();
        let mut finalized = std::collections::HashSet::new();
        finalized.insert(30i64);
        let (dag, layout_graph) = build_block_graph(&blocks, &connections, &finalized);
        calculate_layout(&layout_graph, true);

        use petgraph::visit::EdgeRef;
        let has_edge = |from: u64, to: u64| -> bool {
            let from_idx = dag.index_of(from).unwrap();
            dag.graph
                .edges_directed(from_idx, petgraph::Direction::Outgoing)
                .any(|e| e.target() == dag.index_of(to).unwrap())
        };

        let p1 = 30u64 * 10_000 + 1;
        let p2 = 30u64 * 10_000 + 2;
        assert!(
            dag.index_of(p1).is_some() && dag.index_of(p2).is_some(),
            "finalized 時は P_1・P_2（最終出力）とも合成されるべき"
        );

        // 最終バッチ（33）の出力は最終フェーズ P_2 へ向く
        assert!(
            has_edge(33, p2),
            "バッチ2 → フェーズ2（sai-write 由来）の task_output"
        );
        // 縦接続鎖: mb1 → P_1 → P_2
        assert!(
            has_edge(30, p1) && has_edge(p1, p2),
            "縦接続鎖 mb1 → P_1 → P_2"
        );
    }

    /// task_create 未使用（batch_id なしサブのみ）の場合はフェーズノードを作らない
    /// （従来どおり横並びチェーン）。
    #[test]
    fn no_phase_without_batched_tasks() {
        let blocks = sample_blocks(); // サブ（11, 12）は batch_id なし
        let connections = sample_connections();
        let mut finalized = std::collections::HashSet::new();
        finalized.insert(9i64); // sai-write 済みでも task_create がなければ P は作らない
        let (dag, _layout_graph) = build_block_graph(&blocks, &connections, &finalized);
        assert!(
            dag.index_of(9u64 * 10_000 + 1).is_none(),
            "task_create 未使用なら sai-write があってもフェーズノードは作らない"
        );
    }

    /// ブロック・接続を描画し、罫線文字列を返す（ドット込み）。
    fn render_to_string(
        blocks: &[BlockDisplayData],
        connections: &[ConnectionDisplayData],
    ) -> String {
        let (dag, layout_graph) =
            build_block_graph(blocks, connections, &std::collections::HashSet::new());
        let layout = calculate_layout(&layout_graph, true);

        let area = Rect::new(0, 0, 90, 30);
        let mut buf = TuiBuffer::empty(area);

        render_separators(
            &layout.separators,
            layout.content_width,
            0,
            0,
            area,
            &mut buf,
        );
        render_edges(&dag, &layout, 0, 0, true, None, area, &mut buf);

        let theme = AppTheme::dark();
        for pos in &layout.nodes {
            if let Some(block) = blocks.iter().find(|b| b.exec_id == pos.node_id as i64) {
                let node_data = BlockNodeData {
                    name: block.name.clone(),
                    tag: block.tag.clone(),
                    exec_id: block.exec_id,
                    runner: block.runner.clone(),
                    created_at: block.created_at.clone(),
                    description: block.description.clone(),
                    selected: false,
                    status_color: None,
                    cost: None,
                };
                let off_rect = Rect::new(0, 0, pos.width, pos.height);
                let mut node_buf = TuiBuffer::empty(off_rect);
                BlockNodeWidget::new(&node_data, theme.main_bg).render(off_rect, &mut node_buf);
                for ny in 0..pos.height {
                    for nx in 0..pos.width {
                        if let Some(src) = node_buf.cell((nx, ny)).cloned() {
                            let dx = (pos.x + nx as i32) as u16;
                            let dy = (pos.y + ny as i32) as u16;
                            if dx < area.width && dy < area.height {
                                if let Some(cell) = buf.cell_mut((dx, dy)) {
                                    *cell = src;
                                }
                            }
                        }
                    }
                }
            }
        }

        let mut out = String::new();
        for y in 0..area.height {
            for x in 0..area.width {
                out.push(buf[(x, y)].symbol().chars().next().unwrap_or(' '));
            }
            out.push('\n');
        }
        out
    }

    /// エッジは実行単位で張られる: トークン関係のない exec8 から exec9 への
    /// エッジは存在しない。
    #[test]
    fn edges_are_execution_level() {
        let blocks = sample_blocks();
        let connections = sample_connections();
        let (dag, _) = build_block_graph(&blocks, &connections, &std::collections::HashSet::new());

        use petgraph::visit::EdgeRef;
        let has_edge = |from: u64, to: u64| -> bool {
            let from_idx = dag.index_of(from).unwrap();
            dag.graph
                .edges_directed(from_idx, petgraph::Direction::Outgoing)
                .any(|e| e.target() == dag.index_of(to).unwrap())
        };

        // waggle_token は 7→9 のみ（8 は 9 とトークン関係がない）
        assert!(has_edge(7, 9), "waggle_token: 7→9 は接続されるべき");
        assert!(!has_edge(8, 9), "トークン関係のない 8→9 は接続されないべき");
        assert!(!has_edge(10, 9), "GR の異なる 10→9 は接続されないべき");
        // 委譲はチェーン接続（9 → 11 → 12）: バッチ 2 番目以降は前のサブから繋がり、
        // メインから直接は貫通しない（長方形内部に接続線が入らないようにする）
        assert!(
            has_edge(9, 11) && has_edge(11, 12),
            "main→sub 委譲はチェーンで接続"
        );
        assert!(
            !has_edge(9, 12),
            "メインから 2 番目以降のサブへは直接接続しない（貫通防止）"
        );
        // task_output は sub→main（上下辺接地）
        assert!(
            has_edge(11, 9) && has_edge(12, 9),
            "task_output は sub→main"
        );
    }

    /// フェーズ合成の描画を /tmp にダンプする（確認用）。
    #[test]
    fn dump_phase_visual() {
        let blocks = phase_blocks();
        let connections = phase_connections();
        let mut finalized = std::collections::HashSet::new();
        finalized.insert(30i64);
        let (dag, layout_graph) = build_block_graph(&blocks, &connections, &finalized);
        let layout = calculate_layout(&layout_graph, true);

        let area = Rect::new(0, 0, 100, 60);
        let mut buf = TuiBuffer::empty(area);

        render_separators(
            &layout.separators,
            layout.content_width,
            0,
            0,
            area,
            &mut buf,
        );
        render_edges(&dag, &layout, 0, 0, true, None, area, &mut buf);

        let theme = AppTheme::dark();
        for pos in &layout.nodes {
            if let Some(block) = dag.get(pos.index) {
                let node_data = BlockNodeData {
                    name: block.name.clone(),
                    tag: block.tag.clone(),
                    exec_id: block.exec_id,
                    runner: block.runner.clone(),
                    created_at: block.created_at.clone(),
                    description: block.description.clone(),
                    selected: false,
                    status_color: None,
                    cost: None,
                };
                let off_rect = Rect::new(0, 0, pos.width, pos.height);
                let mut node_buf = TuiBuffer::empty(off_rect);
                BlockNodeWidget::new(&node_data, theme.main_bg).render(off_rect, &mut node_buf);
                for ny in 0..pos.height {
                    for nx in 0..pos.width {
                        if let Some(src) = node_buf.cell((nx, ny)).cloned() {
                            let dx = (pos.x + nx as i32) as u16;
                            let dy = (pos.y + ny as i32) as u16;
                            if dx < area.width && dy < area.height {
                                if let Some(cell) = buf.cell_mut((dx, dy)) {
                                    *cell = src;
                                }
                            }
                        }
                    }
                }
            }
        }
        let mut out = String::new();
        for y in 0..area.height {
            for x in 0..area.width {
                out.push(buf[(x, y)].symbol().chars().next().unwrap_or(' '));
            }
            out.push('\n');
        }
        std::fs::write("/tmp/dag_phase.txt", out.as_str()).unwrap();
        println!("{}", out);
    }

    /// フェーズノード（mb1-2 / mb1-3 等の合成小ブロック）も実メインの選択として
    /// 色が変わること（左SB でメインブロックを選択したとき、mb1-2,3 もハイライト）。
    #[test]
    fn phase_nodes_highlighted_when_main_selected() {
        // is_node_selected のロジック検証
        let main_block = BlockDisplayData {
            exec_id: 30,
            block_id: 1,
            name: "main-block".into(),
            tag: None,
            global_round: Some(1),
            round: Some(1),
            parent_exec_id: None,
            batch_id: None,
            phase: None,
            runner: "pi".into(),
            last_status: "completed".into(),
            reflection_status: None,
            last_duration_ms: None,
            created_at: String::new(),
            description: String::new(),
        };
        // 実メイン選択時: 実メイン自身は選択扱い
        assert!(is_node_selected(Some(30), &main_block));
        // フェーズノード（合成小ブロック・phase あり・フェーズ親 = 30）
        let phase1 = BlockDisplayData {
            exec_id: 30 * 10_000 + 1,
            block_id: 1,
            name: "main-block".into(),
            tag: None,
            global_round: Some(1),
            round: Some(1),
            parent_exec_id: Some(30),
            batch_id: None,
            phase: Some(1),
            runner: "pi".into(),
            last_status: "completed".into(),
            reflection_status: None,
            last_duration_ms: None,
            created_at: String::new(),
            description: String::new(),
        };
        // 実メイン（30）選択時: フェーズノードも選択扱い
        assert!(
            is_node_selected(Some(30), &phase1),
            "フェーズノードも選択扱い（mb1-2,3 のハイライト）"
        );
        // 別のメイン選択時: フェーズノードは非選択
        assert!(!is_node_selected(Some(99), &phase1));
        // 実サブ（phase なし）は選択扱いにならない
        let sub = BlockDisplayData {
            exec_id: 31,
            block_id: 2,
            name: "sub-task".into(),
            tag: None,
            global_round: Some(1),
            round: Some(1),
            parent_exec_id: Some(30),
            batch_id: None,
            phase: None,
            runner: "pi".into(),
            last_status: "completed".into(),
            reflection_status: None,
            last_duration_ms: None,
            created_at: String::new(),
            description: String::new(),
        };
        assert!(
            !is_node_selected(Some(30), &sub),
            "実サブはメイン選択で色が変わらない"
        );

        // 描画の回帰: フェーズデータを描画し、フェーズノードが選択色で描画される
        let (dag, layout_graph) = build_block_graph(
            &phase_blocks(),
            &phase_connections(),
            &std::collections::HashSet::new(),
        );
        let layout = calculate_layout(&layout_graph, true);
        let theme = AppTheme::dark();

        // フェーズノード 300001 を選択色で描画
        let p1 = 30u64 * 10_000 + 1;
        let pos = layout.nodes.iter().find(|p| p.node_id == p1).unwrap();
        let block = dag.get(pos.index).unwrap();
        let node_data = BlockNodeData {
            name: block.name.clone(),
            tag: block.tag.clone(),
            exec_id: block.exec_id,
            runner: block.runner.clone(),
            created_at: block.created_at.clone(),
            description: block.description.clone(),
            selected: is_node_selected(Some(30), block),
            status_color: None,
            cost: None,
        };
        assert!(node_data.selected, "フェーズノードは選択扱いで描画される");
        let off_rect = Rect::new(0, 0, pos.width, pos.height);
        let mut node_buf = TuiBuffer::empty(off_rect);
        BlockNodeWidget::new(&node_data, theme.main_bg).render(off_rect, &mut node_buf);
        // テキスト行の色が選択色（Yellow）であること
        let text_cell = &node_buf[(1, 1)];
        assert_eq!(
            text_cell.fg,
            Color::Yellow,
            "選択中のフェーズノードの文字色は Yellow になるべき"
        );
    }

    /// 相互接続（main→sub 委譲 + sub→main 返却）は 1 本の線に統合され、
    /// 両端が T字（受け取る側）でノードの辺に隙間なく接続する。
    /// 接続点のドット `·` は廃止され、受け取る側の色（委譲=cyan / 返却=green）で
    /// 入出力を判定する。
    #[test]
    fn mutual_delegation_and_output_is_single_line_with_t_ends() {
        let s = render_to_string(&sample_blocks(), &sample_connections());
        // ドット `·` は廃止
        assert!(
            !s.contains('·'),
            "接続点のドット `·` は廃止されるべき\n{}",
            s
        );
        // 相互接続は 1 本の線になり、両端が T字（├ / ┤）でボーダーに接続する
        assert!(
            s.contains('├') && s.contains('┤'),
            "相互接続は `├──┤` の1本線になるべき\n{}",
            s
        );
    }

    /// 区切り線は `roundN────...`（ラベルが左端）で表示される。
    #[test]
    fn separator_label_is_on_left() {
        let s = render_to_string(&sample_blocks(), &sample_connections());
        let first_line = s.lines().next().unwrap_or("");
        assert!(
            first_line.starts_with("round1"),
            "区切り線は round1 から始まるべき: {:?}",
            first_line
        );
        assert!(
            first_line.trim_end().ends_with("─"),
            "区切り線は ─ で終わるべき: {:?}",
            first_line
        );
    }

    #[test]
    fn dump_sample_visual() {
        let s = render_to_string(&sample_blocks(), &sample_connections());
        std::fs::write("/tmp/dag_sample.txt", s.as_str()).unwrap();
        let (dag2, lg2) = build_block_graph(
            &sample_blocks(),
            &sample_connections(),
            &std::collections::HashSet::new(),
        );
        let lay2 = calculate_layout(&lg2, true);
        for p2 in &lay2.nodes {
            let bname = sample_blocks()
                .iter()
                .find(|b| b.exec_id == p2.node_id as i64)
                .map(|b| b.name.clone())
                .unwrap_or_default();
            println!(
                "NODE id={} x={} y={} w={} h={} name={}",
                p2.node_id, p2.x, p2.y, p2.width, p2.height, bname
            );
        }
        for p2 in &lay2.nodes {
            for (to_idx, e) in dag2.edges_from(p2.index) {
                let tp = lay2.find(to_idx).unwrap();
                println!(
                    "EDGE {} -> {} side={} style={:?}",
                    p2.node_id,
                    tp.node_id,
                    e.side,
                    e.style.map(|s| s.fg)
                );
            }
        }
        let area2 = Rect::new(0, 0, 90, 30);
        let mut buf2 = TuiBuffer::empty(area2);
        render_edges(&dag2, &lay2, 0, 0, true, None, area2, &mut buf2);
        println!("{}", s);
    }

    /// 選択中ブロックに接続するエッジ（入出力）はハイライト色（Yellow）で描画され、
    /// 選択していない場合は通常色（マゼンタ等）のままであること。
    #[test]
    fn selected_block_edges_are_highlighted() {
        use ratatui::style::Color as TuiColor;

        let blocks = sample_blocks();
        let connections = sample_connections();
        let (dag, layout_graph) =
            build_block_graph(&blocks, &connections, &std::collections::HashSet::new());
        let layout = calculate_layout(&layout_graph, true);
        let area = Rect::new(0, 0, 90, 30);

        // 選択なし: 通常色
        let mut buf_plain = TuiBuffer::empty(area);
        render_separators(
            &layout.separators,
            layout.content_width,
            0,
            0,
            area,
            &mut buf_plain,
        );
        render_edges(&dag, &layout, 0, 0, true, None, area, &mut buf_plain);

        // 選択あり（exec 9）: 9 に接続する全エッジが黄色
        let mut buf_sel = TuiBuffer::empty(area);
        render_separators(
            &layout.separators,
            layout.content_width,
            0,
            0,
            area,
            &mut buf_sel,
        );
        render_edges(&dag, &layout, 0, 0, true, Some(9), area, &mut buf_sel);

        let node9 = layout.nodes.iter().find(|p| p.node_id == 9).unwrap();
        // 委譲エッジ: 9 の右辺中点 (x+width-1, y+height/2) からサブへ伸びる線
        let side_y = node9.y + i32::from(node9.height) / 2;
        let from_x = node9.x + i32::from(node9.width) - 1;
        let line_x = from_x + 1; // ボーダーの右隣の線セル
        assert_ne!(
            buf_plain[(line_x as u16, side_y as u16)].fg,
            TuiColor::Yellow,
            "選択なしではハイライトされない"
        );
        assert_eq!(
            buf_sel[(line_x as u16, side_y as u16)].fg,
            TuiColor::Yellow,
            "選択ブロックの接続線はハイライト色になるべき"
        );

        // 縦エッジ 7→9（選択ブロックへの入力線）も黄色
        let node7 = layout.nodes.iter().find(|p| p.node_id == 7).unwrap();
        let path_x = node7.x + i32::from(node7.width) / 2;
        let path_y = node7.y + i32::from(node7.height); // 下端の直下
        assert_eq!(
            buf_sel[(path_x as u16, path_y as u16)].fg,
            TuiColor::Yellow,
            "選択ブロックに入力する縦エッジもハイライト色になるべき"
        );
    }

    /// ズーム倍率でレイアウト（ノード位置・サイズ・区切り線）がスケールされ、
    /// zoom = 1.0 の場合は元のレイアウトと一致すること。
    #[test]
    fn scale_layout_scales_positions_and_sizes() {
        let (_, layout_graph) = build_block_graph(
            &sample_blocks(),
            &sample_connections(),
            &std::collections::HashSet::new(),
        );
        let base = calculate_layout(&layout_graph, true);

        // zoom = 1.0: 不変
        let same = scale_layout(&base, 1.0);
        assert_eq!(same, base, "zoom=1.0 ではレイアウトが変わらないべき");

        // zoom = 2.0: 全座標・サイズが 2 倍になる
        let doubled = scale_layout(&base, 2.0);
        for (p, q) in base.nodes.iter().zip(doubled.nodes.iter()) {
            assert_eq!(q.x, p.x * 2, "ノード {} の x が 2 倍になるべき", p.node_id);
            assert_eq!(q.y, p.y * 2, "ノード {} の y が 2 倍になるべき", p.node_id);
            assert_eq!(
                q.width,
                p.width * 2,
                "ノード {} の width が 2 倍になるべき",
                p.node_id
            );
            assert_eq!(
                q.height,
                p.height * 2,
                "ノード {} の height が 2 倍になるべき",
                p.node_id
            );
        }
        assert_eq!(doubled.content_width, base.content_width * 2);
        assert_eq!(doubled.content_height, base.content_height * 2);

        // zoom = 0.5: 半分（四捨五入。幅・高さは最低 1 セル）
        let halved = scale_layout(&base, 0.5);
        for (p, q) in base.nodes.iter().zip(halved.nodes.iter()) {
            assert_eq!(q.x, ((p.x as f64) * 0.5).round() as i32);
            assert_eq!(q.y, ((p.y as f64) * 0.5).round() as i32);
            assert!(
                q.width >= 1 && q.height >= 1,
                "ズームアウトしても 0 にならない"
            );
        }
    }

    /// apply_layout_offsets: ノードの個別ズレ量・round のズレ量（その round 以降へ
    /// 累積）・content_width/height の再計算・負方向へのはみ出し正規化を検証する。
    #[test]
    fn apply_layout_offsets_moves_nodes_and_separators() {
        let (_, layout_graph) = build_block_graph(
            &sample_blocks(),
            &sample_connections(),
            &std::collections::HashSet::new(),
        );
        let base = calculate_layout(&layout_graph, true);
        let node7_base = *base.nodes.iter().find(|p| p.node_id == 7).unwrap();
        let node10_base = *base.nodes.iter().find(|p| p.node_id == 10).unwrap();

        // offset なし: 不変
        let same = apply_layout_offsets(&base, &HashMap::new(), &HashMap::new());
        assert_eq!(same, base, "offset が無ければレイアウトは不変であるべき");

        // ノード個別のズレ量: 指定したノードだけが動く
        let mut node_offsets = HashMap::new();
        node_offsets.insert(7i64, (3, 2));
        let moved = apply_layout_offsets(&base, &node_offsets, &HashMap::new());
        let node7_moved = moved.nodes.iter().find(|p| p.node_id == 7).unwrap();
        assert_eq!(node7_moved.x, node7_base.x + 3);
        assert_eq!(node7_moved.y, node7_base.y + 2);
        for p in moved.nodes.iter().filter(|p| p.node_id != 7) {
            let base_p = base.nodes.iter().find(|b| b.node_id == p.node_id).unwrap();
            assert_eq!(p.x, base_p.x, "ノード {} は動かないべき", p.node_id);
            assert_eq!(p.y, base_p.y, "ノード {} は動かないべき", p.node_id);
        }
        assert_eq!(
            moved.content_width,
            moved
                .nodes
                .iter()
                .map(|p| p.x + i32::from(p.width))
                .max()
                .unwrap(),
            "content_width が再計算されるべき"
        );
        assert_eq!(
            moved.content_height,
            moved
                .nodes
                .iter()
                .map(|p| p.y + i32::from(p.height))
                .max()
                .unwrap(),
            "content_height が再計算されるべき"
        );

        // round2 のズレ量: round2（ID10）だけが動き、round1（ID7 等）は動かない
        let mut sep_offsets = HashMap::new();
        sep_offsets.insert(2i64, 5);
        let sep_moved = apply_layout_offsets(&base, &HashMap::new(), &sep_offsets);
        let node10_moved = sep_moved.nodes.iter().find(|p| p.node_id == 10).unwrap();
        assert_eq!(
            node10_moved.y,
            node10_base.y + 5,
            "round2 の card は round2 のズレ量で動く"
        );
        for id in [7i64, 8, 9, 11, 12] {
            let p = sep_moved
                .nodes
                .iter()
                .find(|p| p.node_id as i64 == id)
                .unwrap();
            let base_p = base.nodes.iter().find(|b| b.node_id as i64 == id).unwrap();
            assert_eq!(
                p.y, base_p.y,
                "round1 の card {} は round2 のズレ量の影響を受けないべき",
                id
            );
        }
        let sep2 = sep_moved
            .separators
            .iter()
            .find(|s| s.global_round == 2)
            .unwrap();
        let base_sep2 = base.separators.iter().find(|s| s.global_round == 2).unwrap();
        assert_eq!(sep2.y, base_sep2.y + 5, "round2 の区切り線自体も動く");
        let sep1 = sep_moved
            .separators
            .iter()
            .find(|s| s.global_round == 1)
            .unwrap();
        let base_sep1 = base.separators.iter().find(|s| s.global_round == 1).unwrap();
        assert_eq!(sep1.y, base_sep1.y, "round1 の区切り線は動かないべき");

        // 負方向にはみ出す場合は全体を平行移動して 0 以上に正規化する
        let mut neg_offsets = HashMap::new();
        neg_offsets.insert(7i64, (-100, -100));
        let normalized = apply_layout_offsets(&base, &neg_offsets, &HashMap::new());
        assert!(
            normalized.nodes.iter().all(|p| p.x >= 0 && p.y >= 0),
            "負方向にはみ出したら全体を平行移動して 0 以上に正規化する"
        );
    }

    /// waggle_token の縦方向エッジは、入力先ブロックの上辺中点に `┴`、
    /// 出力元の下辺中点に `┬` が付き、ノードの辺と接続線が隙間なくつながる。
    /// （T字の軸は接続線の方向を向く: 上辺=線が上から来る→`┴`、下辺=線が下へ出る→`┬`）
    #[test]
    fn vertical_edge_connects_target_top_with_t() {
        let s = render_to_string(&sample_blocks(), &sample_connections());
        // ekphos 方式: card の辺（╭ 行・╰ 行）には接続線を一切重ねない
        // （旧 GroundDot による ┴/┬ の上書きは廃止）
        assert!(
            s.lines()
                .filter(|l| l.starts_with('╭'))
                .all(|l| !l.contains('┴')),
            "card 上辺に ┴ を重ねてはいけない（枠は無傷のまま）\n{}",
            s
        );
        assert!(
            s.lines()
                .filter(|l| l.starts_with('╰'))
                .all(|l| !l.contains('┬')),
            "card 下辺に ┬ を重ねてはいけない（枠は無傷のまま）\n{}",
            s
        );
        // 入力先の直前（card の外側 1 行）には常に矢印が立つ
        assert!(s.contains('▼'), "card の直上に矢印 ▼ が立つべき\n{}", s);
        // ドット `·` は廃止されている
        assert!(!s.contains('·'), "ドット `·` は廃止されるべき\n{}", s);
    }
}
// 実 DB 描画の回帰テスト（docs/tui-fix.md の 4 問題の検証）
#[cfg(test)]
mod render_real_db {
    use super::*;
    use crate::widgets::block_node::{BlockNodeData, BlockNodeWidget};
    use ratatui::buffer::Buffer as TuiBuffer;
    use sai_link_widgets::theme::AppTheme;

    /// 実 DB の実行履歴を再現したテストデータ。
    /// GR1: ID7/ID8 (LR1), ID9 (LR2) / GR2: ID10/ID11 (LR1), ID12 (LR2) / GR3: ID13 (LR1)
    fn real_db_blocks() -> Vec<BlockDisplayData> {
        vec![
            BlockDisplayData {
                exec_id: 7,
                block_id: 1,
                name: "test_write".into(),
                tag: Some("test".into()),
                global_round: Some(1),
                round: Some(1),
                parent_exec_id: None,
                batch_id: None,
                phase: None,
                runner: "pi".into(),
                last_status: "completed".into(),
                reflection_status: None,
                last_duration_ms: Some(1000),
                created_at: "2026-08-02T02:08:09".into(),
                description: String::new(),
            },
            BlockDisplayData {
                exec_id: 8,
                block_id: 1,
                name: "test_write".into(),
                tag: Some("test".into()),
                global_round: Some(1),
                round: Some(1),
                parent_exec_id: None,
                batch_id: None,
                phase: None,
                runner: "pi".into(),
                last_status: "completed".into(),
                reflection_status: None,
                last_duration_ms: Some(1000),
                created_at: "2026-08-02T02:08:39".into(),
                description: String::new(),
            },
            BlockDisplayData {
                exec_id: 9,
                block_id: 2,
                name: "test_merge".into(),
                tag: Some("test".into()),
                global_round: Some(1),
                round: Some(2),
                parent_exec_id: None,
                batch_id: None,
                phase: None,
                runner: "pi".into(),
                last_status: "completed".into(),
                reflection_status: None,
                last_duration_ms: Some(1000),
                created_at: "2026-08-02T02:09:46".into(),
                description: String::new(),
            },
            BlockDisplayData {
                exec_id: 10,
                block_id: 1,
                name: "test_write".into(),
                tag: Some("test".into()),
                global_round: Some(2),
                round: Some(1),
                parent_exec_id: None,
                batch_id: None,
                phase: None,
                runner: "pi".into(),
                last_status: "completed".into(),
                reflection_status: None,
                last_duration_ms: Some(1000),
                created_at: "2026-08-02T02:20:20".into(),
                description: String::new(),
            },
            BlockDisplayData {
                exec_id: 11,
                block_id: 1,
                name: "test_write".into(),
                tag: Some("test".into()),
                global_round: Some(2),
                round: Some(1),
                parent_exec_id: None,
                batch_id: None,
                phase: None,
                runner: "pi".into(),
                last_status: "completed".into(),
                reflection_status: None,
                last_duration_ms: Some(1000),
                created_at: "2026-08-02T03:06:34".into(),
                description: String::new(),
            },
            BlockDisplayData {
                exec_id: 12,
                block_id: 2,
                name: "test_merge".into(),
                tag: Some("test".into()),
                global_round: Some(2),
                round: Some(2),
                parent_exec_id: None,
                batch_id: None,
                phase: None,
                runner: "pi".into(),
                last_status: "completed".into(),
                reflection_status: None,
                last_duration_ms: Some(1000),
                created_at: "2026-08-02T03:07:43".into(),
                description: String::new(),
            },
            BlockDisplayData {
                exec_id: 13,
                block_id: 2,
                name: "test_merge".into(),
                tag: Some("test".into()),
                global_round: Some(3),
                round: Some(1),
                parent_exec_id: None,
                batch_id: None,
                phase: None,
                runner: "pi".into(),
                last_status: "completed".into(),
                reflection_status: None,
                last_duration_ms: Some(1000),
                created_at: "2026-08-02T03:09:27".into(),
                description: String::new(),
            },
        ]
    }

    /// 実 DB の block_connections を再現したテストデータ。
    fn real_db_connections() -> Vec<ConnectionDisplayData> {
        vec![
            ConnectionDisplayData {
                source_execution_id: 7,
                target_execution_id: 9,
                connection_type: "waggle_token".into(),
            },
            ConnectionDisplayData {
                source_execution_id: 11,
                target_execution_id: 12,
                connection_type: "waggle_token".into(),
            },
            ConnectionDisplayData {
                source_execution_id: 10,
                target_execution_id: 12,
                connection_type: "waggle_token".into(),
            },
            ConnectionDisplayData {
                source_execution_id: 11,
                target_execution_id: 13,
                connection_type: "waggle_token".into(),
            },
            ConnectionDisplayData {
                source_execution_id: 10,
                target_execution_id: 13,
                connection_type: "waggle_token".into(),
            },
            ConnectionDisplayData {
                source_execution_id: 8,
                target_execution_id: 13,
                connection_type: "waggle_token".into(),
            },
        ]
    }

    /// 縦エッジ（上下辺接地）の経路が途中のノードを横切らないこと。
    ///
    /// 旧描画の問題: 10→13 の線が ID12 を横切り「ID12→ID13」に見えた（問題2）、
    /// 8→13 の線が ID11 に重なった（問題3）。レイアウトの経路回避で解消する。
    #[test]
    fn vertical_edges_do_not_cross_nodes() {
        let blocks = real_db_blocks();
        let connections = real_db_connections();
        let (_dag, layout_graph) =
            build_block_graph(&blocks, &connections, &std::collections::HashSet::new());
        let layout = calculate_layout(&layout_graph, true);

        for &(from_id, to_id, side) in &layout_graph.edges {
            if side {
                continue; // 左右辺接地（サブ関係）は下の行を通過しない
            }
            let from_pos = layout
                .nodes
                .iter()
                .find(|p| p.node_id == from_id)
                .expect("出力元ノードが配置されていない");
            let to_pos = layout
                .nodes
                .iter()
                .find(|p| p.node_id == to_id)
                .expect("入力先ノードが配置されていない");
            let path_x = from_pos.x + i32::from(from_pos.width) / 2;
            let from_y = from_pos.y + i32::from(from_pos.height);
            let to_y = to_pos.y;

            for node in &layout.nodes {
                if node.node_id == from_id || node.node_id == to_id {
                    continue;
                }
                // 途中にあるノード（from の下端と to の上端の間）のみチェック
                let node_top = node.y;
                let node_bottom = node.y + i32::from(node.height);
                if node_top < to_y && node_bottom > from_y {
                    assert!(
                        path_x < node.x || path_x >= node.x + i32::from(node.width),
                        "エッジ {}→{} の経路 x={} がノード {} (x={}..{}) を横切る",
                        from_id,
                        to_id,
                        path_x,
                        node.node_id,
                        node.x,
                        node.x + i32::from(node.width)
                    );
                }
            }
        }
    }

    /// 区切り線（roundN）と縦エッジの**バス行（入力先の直前）**が重ならないこと。
    ///
    /// 旧描画の問題: round3 の青線と接続線の横線が重なった（問題4）。
    /// SEPARATOR_LINES の余白拡大とターゲット側バス化で解消する。
    #[test]
    fn separators_do_not_overlap_target_buses() {
        let blocks = real_db_blocks();
        let connections = real_db_connections();
        let (_dag, layout_graph) =
            build_block_graph(&blocks, &connections, &std::collections::HashSet::new());
        let layout = calculate_layout(&layout_graph, true);

        // 縦エッジのバス行（入力先の上辺の 2 行上。直上 1 行は矢印専用の行）
        let mut bus_rows: Vec<i32> = Vec::new();
        for &(_, to_id, side) in &layout_graph.edges {
            if side {
                continue;
            }
            let to_pos = layout
                .nodes
                .iter()
                .find(|p| p.node_id == to_id)
                .expect("入力先ノードが配置されていない");
            bus_rows.push(to_pos.y - 2);
        }

        // バス行が区切り線の行と重ならないこと
        for sep in &layout.separators {
            assert!(
                !bus_rows.contains(&sep.y),
                "区切り線 {} (y={}) と縦エッジのバス行が重なる",
                sep.label,
                sep.y
            );
        }
    }

    fn render(
        blocks: &[BlockDisplayData],
        connections: &[ConnectionDisplayData],
        w: u16,
        h: u16,
    ) -> String {
        let (dag, layout_graph) =
            build_block_graph(blocks, connections, &std::collections::HashSet::new());
        let layout = calculate_layout(&layout_graph, true);
        let area = Rect::new(0, 0, w, h);
        let mut buf = TuiBuffer::empty(area);
        render_separators(
            &layout.separators,
            layout.content_width,
            0,
            0,
            area,
            &mut buf,
        );
        render_edges(&dag, &layout, 0, 0, true, None, area, &mut buf);
        let theme = AppTheme::dark();
        for pos in &layout.nodes {
            if let Some(block) = blocks.iter().find(|b| b.exec_id == pos.node_id as i64) {
                let node_data = BlockNodeData {
                    name: block.name.clone(),
                    tag: block.tag.clone(),
                    exec_id: block.exec_id,
                    runner: block.runner.clone(),
                    created_at: block.created_at.clone(),
                    description: block.description.clone(),
                    selected: false,
                    status_color: None,
                    cost: None,
                };
                let off_rect = Rect::new(0, 0, pos.width, pos.height);
                let mut node_buf = TuiBuffer::empty(off_rect);
                BlockNodeWidget::new(&node_data, theme.main_bg).render(off_rect, &mut node_buf);
                for ny in 0..pos.height {
                    for nx in 0..pos.width {
                        if let Some(src) = node_buf.cell((nx, ny)).cloned() {
                            let dx = (pos.x + nx as i32) as u16;
                            let dy = (pos.y + ny as i32) as u16;
                            if dx < area.width && dy < area.height {
                                if let Some(cell) = buf.cell_mut((dx, dy)) {
                                    *cell = src;
                                }
                            }
                        }
                    }
                }
            }
        }
        let mut out = String::new();
        for y in 0..area.height {
            for x in 0..area.width {
                out.push(buf[(x, y)].symbol().chars().next().unwrap_or(' '));
            }
            out.push('\n');
        }
        out
    }

    #[test]
    fn real_db_visual() {
        let blocks = real_db_blocks();
        let connections = real_db_connections();
        let (dag, layout_graph) =
            build_block_graph(&blocks, &connections, &std::collections::HashSet::new());
        let layout = calculate_layout(&layout_graph, true);
        for p in &layout.nodes {
            let b = blocks
                .iter()
                .find(|b| b.exec_id == p.node_id as i64)
                .unwrap();
            println!(
                "NODE id={} x={} y={} w={} h={} (GR={:?} LR={:?})",
                p.node_id, p.x, p.y, p.width, p.height, b.global_round, b.round
            );
        }
        for sep in &layout.separators {
            println!("SEP y={} label={}", sep.y, sep.label);
        }
        for p in &layout.nodes {
            for (to_idx, e) in dag.edges_from(p.index) {
                let tp = layout.find(to_idx).unwrap();
                println!(
                    "EDGE {} -> {} side={} style={:?}",
                    p.node_id,
                    tp.node_id,
                    e.side,
                    e.style.map(|s| s.fg)
                );
            }
        }
        let s = render(&blocks, &connections, 100, 60);
        println!("===== REAL DB =====\n{}\n===== END =====", s);
        std::fs::write("/tmp/dag_real.txt", s).unwrap();
    }

    /// 接続種別ごとのバス行段差（2026-09-01 検証実行のシナリオ）:
    /// ID6（メイン）の複製ブロック ID6.1（P_1）とサブ ID9 が同じ行にあり、
    /// ID7/ID8 からの task_output（緑）と waggle（マゼンタ）が同じバス行に
    /// 重なって区別できなかった問題の回帰テスト。
    /// - task_output（ID7,8 → ID6.1）のバス: 緑、一段上のバス行
    /// - waggle（ID7,8 → ID9）のバス: マゼンタ、入力先直前のバス行
    #[test]
    fn task_output_and_waggle_buses_are_staggered_by_kind() {
        use ratatui::buffer::Buffer as TuiBuffer;

        let mk = |exec_id: i64,
                  block_id: i64,
                  name: &str,
                  parent: Option<i64>,
                  batch: Option<&str>,
                  at: &str| BlockDisplayData {
            exec_id,
            block_id,
            name: name.into(),
            tag: None,
            global_round: Some(2),
            round: Some(1),
            parent_exec_id: parent,
            batch_id: batch.map(|s| s.to_string()),
            phase: None,
            runner: "pi".into(),
            last_status: "completed".into(),
            reflection_status: None,
            last_duration_ms: Some(1000),
            created_at: at.into(),
            description: String::new(),
        };
        let blocks = vec![
            mk(6, 3, "main", None, None, "2026-09-01T01:42:08"),
            mk(
                7,
                6,
                "sub-a",
                Some(6),
                Some("batch_20260901_r1_r5ca"),
                "2026-09-01T01:42:31",
            ),
            mk(
                8,
                6,
                "sub-b",
                Some(6),
                Some("batch_20260901_r1_r5ca"),
                "2026-09-01T01:42:31",
            ),
            mk(
                9,
                7,
                "waggle",
                Some(6),
                Some("batch_20260901_r1_irn1"),
                "2026-09-01T01:43:17",
            ),
        ];
        let conn = |s: i64, t: i64, ty: &str| ConnectionDisplayData {
            source_execution_id: s,
            target_execution_id: t,
            connection_type: ty.into(),
        };
        let connections = vec![
            conn(6, 7, "task_delegation"),
            conn(6, 8, "task_delegation"),
            conn(7, 6, "task_output"),
            conn(8, 6, "task_output"),
            conn(6, 9, "task_delegation"),
            conn(7, 9, "waggle_token"),
            conn(8, 9, "waggle_token"),
            conn(9, 6, "task_output"),
        ];
        // sai-write 検出済（P_1 / P_2 を合成）
        let finalized: std::collections::HashSet<i64> = [6i64].into_iter().collect();

        let (dag, layout_graph) = build_block_graph(&blocks, &connections, &finalized);
        let layout = calculate_layout(&layout_graph, true);
        let area = Rect::new(0, 0, 120, 80);
        let mut buf = TuiBuffer::empty(area);
        render_edges(&dag, &layout, 0, 0, true, None, area, &mut buf);

        let fg_at = |x: i32, y: i32| buf[(x as u16, y as u16)].fg;
        let sym_at = |x: i32, y: i32| {
            buf[(x as u16, y as u16)]
                .symbol()
                .chars()
                .next()
                .unwrap_or(' ')
        };
        let pos = |id: i64| {
            layout
                .nodes
                .iter()
                .find(|p| p.node_id == id as u64)
                .map(|p| (p.x, p.y, p.width, p.height))
                .unwrap_or_else(|| panic!("node {} not laid out", id))
        };

        let (_m6x, m6y, m6w, m6h) = pos(6);
        let (s7x, _, s7w, _) = pos(7);
        let (s8x, _, s8w, _) = pos(8);
        let (p1x, p1y, p1w, _) = pos(60_001);
        let (id9x, id9y, id9w, _) = pos(9);
        let _ = (m6y, m6w, m6h, s7w, s8w, id9w);

        // ID9 は P_1 と同じ行に配置される（バッチ 2 → フェーズ 1 行）
        assert_eq!(id9y, p1y, "ID9 は P_1 と同じ行");

        let m6c = _m6x + i32::from(m6w) / 2;
        let s7c = s7x + i32::from(s7w) / 2;
        let s8c = s8x + i32::from(s8w) / 2;
        let p1c = p1x + i32::from(p1w) / 2;
        let _id9c = id9x + i32::from(id9w) / 2;

        // バス行: P_1 の上 2 行（= ID9 の上 2 行。直上 1 行は常に矢印用）と上 3 行
        let bus_row_near = p1y - 2;
        let bus_row_far = p1y - 3;

        // ID7/ID8 → ID6.1（task_output・緑）のバスは 1 行上（遠い方）
        // P_1 中点と ID7 中点の間に緑の横線がある
        let mid = (p1c + s7c) / 2;
        assert_eq!(sym_at(mid, bus_row_far), '─', "遠いバス行に横線がある");
        assert_eq!(
            fg_at(mid, bus_row_far),
            Color::Green,
            "task_output バスは緑"
        );

        // ID7/ID8 → ID9（waggle・マゼンタ）のバスは入力先直前の行
        // ID7 中点と ID8 中点の間にマゼンタの横線がある
        let mid2 = (s7c + s8c) / 2;
        assert_eq!(sym_at(mid2, bus_row_near), '─', "近いバス行に横線がある");
        assert_eq!(
            fg_at(mid2, bus_row_near),
            Color::Magenta,
            "waggle バスはマゼンタ"
        );

        // 近いバス行（マゼンタ）と遠いバス行（緑）は別行（重なり解消）
        assert_ne!(bus_row_near, bus_row_far);

        // P_1 中点の列はフェーズ鎖（シアン）の直進縦線が通る
        // （ID6 中点 == P_1 中点。ターゲット直前のバス行でシアンの縦線）
        assert_eq!(
            fg_at(m6c, bus_row_near),
            Color::Cyan,
            "フェーズ鎖の縦線はシアン"
        );

        // デバッグ出力
        let mut out = String::new();
        for y in 0..area.height {
            for x in 0..area.width {
                let c = buf[(x, y)].clone();
                out.push(c.symbol().chars().next().unwrap_or(' '));
            }
            out.push('\n');
        }
        std::fs::write("/tmp/dag_stagger.txt", out).unwrap();
    }

    /// 複数のブロックが 1 つのブロックに接続するとき（ID10, ID11 → ID12 /
    /// ID8, ID10, ID11 → ID13）、入力先の直前の**バス行**で合流し、
    /// 入力先の中点（`┬`）で折れ曲がって上辺中点（`┴`）へ接続する。
    ///
    /// - 合流点は T 字（`├` `┤`）、入力元の縦線がバスより先へ伸びる場合の
    ///   合流は `├`/`┤`、バス行を**通過するだけ**の縦線
    ///   （ID8→ID13 が ID12 のバスを横切る）は `┼` で交差する
    /// - 区切り線（roundN）との交差は許容
    #[test]
    fn converging_edges_merge_on_target_side_bus() {
        let blocks = real_db_blocks();
        let connections = real_db_connections();
        let s = render(&blocks, &connections, 100, 60);

        // ドット `·` は廃止
        assert!(!s.contains('·'), "ドット `·` は廃止されるべき\n{}", s);

        // ID12 のバス行: ID10 の合流 `├`・ID12 の中点 `┬`・ID8 の交差 `┼`・ID11 の合流 `┤`
        let bus12 = s
            .lines()
            .find(|l| l.contains('├') && l.contains('┤'))
            .expect("ID12 のバス行が見つからない");
        assert!(
            bus12.contains('┬'),
            "バス行に ID12 の中点 ┬ が無い: {bus12}"
        );
        assert!(bus12.contains('┼'), "バス行に ID8 の交差 ┼ が無い: {bus12}");

        // ID13 のバス行: ID8 の合流 `┴`・ID11 の合流 `╯`
        let bus13 = s
            .lines()
            .find(|l| l.contains('┴') && l.contains('╯'))
            .expect("ID13 のバス行が見つからない");
        assert!(
            bus13.contains('├'),
            "ID13 のバス行に ID10 の合流 ├ が無い: {bus13}"
        );

        // ekphos 方式: card の辺（╭ 行・╰ 行）には接続線を一切重ねない
        assert!(
            s.lines()
                .filter(|l| l.starts_with('╭'))
                .all(|l| !l.contains('┴')),
            "card 上辺に ┴ を重ねてはいけない（枠は無傷のまま）\n{}",
            s
        );
        assert!(
            s.lines()
                .filter(|l| l.starts_with('╰'))
                .all(|l| !l.contains('┬')),
            "card 下辺に ┬ を重ねてはいけない（枠は無傷のまま）\n{}",
            s
        );
        assert!(s.contains('▼'), "card の直上に矢印 ▼ が立つべき\n{}", s);

        // ID8 の線は ID7→ID9 の幹線に合流しない
        // （7/8 の下辺の直下の行は縦線のみで、`┤`/`╯` を含まない）
        let below78 = s.lines().nth(9).unwrap_or("");
        assert!(
            !below78.contains('┤') && !below78.contains('╯') && below78.contains('│'),
            "ID8 の線が ID7→ID9 の幹線に合流している（行: {below78:?}）\n{}",
            s
        );
    }
}

// マークダウン表示（History/Blocks モード）の描画テスト
#[cfg(test)]
mod markdown_view_tests {
    use super::*;
    use ratatui::buffer::Buffer as TuiBuffer;
    use sai_link_widgets::theme::AppTheme;

    /// MarkdownView を設定した MainPanel を描画してバッファを返す。
    fn render_md(
        lines: Vec<Line<'static>>,
        scroll: usize,
        title: &str,
        w: u16,
        h: u16,
    ) -> (TuiBuffer, MainPanelState) {
        let mut state = MainPanelState::default();
        state.md_view = Some(MarkdownView {
            lines,
            headings: Vec::new(),
            folded: std::collections::HashSet::new(),
            title: title.to_string(),
            subtitle: "/path/to/file.md".into(),
            scroll,
            cursor: 0,
            view_height: 0,
            outline_visible: false,
            outline_selected: 0,
            outline_focused: false,
        });
        let theme = AppTheme::dark();
        let area = Rect::new(0, 0, w, h);
        let mut buf = TuiBuffer::empty(area);
        MainPanel {
            state: &mut state,
            blocks: &[],
            connections: &[],
            finalized_runs: &std::collections::HashSet::new(),
            orphaned_runs: &std::collections::HashSet::new(),
            theme: &theme,
        }
        .render(area, &mut buf);
        (buf, state)
    }

    /// 1行分のシンボルを文字列化する。
    fn row(buf: &TuiBuffer, y: u16, width: u16) -> String {
        (0..width)
            .map(|x| buf[(x, y)].symbol().chars().next().unwrap_or(' '))
            .collect()
    }

    /// タイトル行・セパレータ・本文・キーガイドが描画されること。
    #[test]
    fn markdown_view_draws_title_body_and_guide() {
        let lines = vec![
            Line::from("# Heading"),
            Line::default(),
            Line::from("body text"),
        ];
        let (buf, _) = render_md(lines, 0, "Session #1 (test)", 50, 10);

        // タイトル行: " Session #1 (test): /path/to/file.md "
        let title = row(&buf, 0, 50);
        assert!(title.contains("Session #1 (test)"), "タイトル: {:?}", title);
        assert!(
            title.contains("/path/to/file.md"),
            "サブタイトル: {:?}",
            title
        );

        // セパレータ行（2行目）は ─
        let sep = row(&buf, 1, 50);
        assert!(sep.contains('─'), "セパレータ: {:?}", sep);

        // 本文（3行目から）: 見出しが表示される
        let body = row(&buf, 2, 50);
        assert!(body.contains("# Heading"), "本文1行目: {:?}", body);

        // キーガイド（最終行）
        let guide = row(&buf, 9, 50);
        assert!(guide.contains("move"), "キーガイド: {:?}", guide);
        assert!(guide.contains("Esc close"), "キーガイド: {:?}", guide);
    }

    /// スクロール位置に応じて本文がずれること（キーガイドは固定）。
    #[test]
    fn markdown_view_scrolls_content() {
        let lines: Vec<Line<'static>> =
            (0..20).map(|i| Line::from(format!("line {}", i))).collect();
        let (buf0, _) = render_md(lines.clone(), 0, "t", 40, 12);
        let (buf5, _) = render_md(lines, 5, "t", 40, 12);

        // スクロールなし: 先頭行が見える
        let top0 = row(&buf0, 2, 40);
        assert!(top0.contains("line 0"), "先頭: {:?}", top0);

        // スクロール 5: 6 行目が見える
        let top5 = row(&buf5, 2, 40);
        assert!(top5.contains("line 5"), "スクロール後: {:?}", top5);

        // キーガイドはスクロールの影響を受けない
        let guide0 = row(&buf0, 11, 40);
        let guide5 = row(&buf5, 11, 40);
        assert!(guide0.contains("move") && guide5.contains("move"));
    }

    /// 折りたたみ中の見出し行には `▸` と、隠れている行数 `(N行)` が付くこと。
    /// 展開中の見出しには付かない。
    #[test]
    fn folded_heading_shows_hidden_line_count() {
        fn text_of(line: &Line) -> String {
            line.spans.iter().map(|s| s.content.as_ref()).collect()
        }

        let theme = AppTheme::dark();
        let md_text = "# A\nbody a1\nbody a2\n\n# B\nbody b1\n";
        let (lines, headings) = crate::ui::markdown::render_markdown(md_text, &theme);
        let heading_a = headings.iter().find(|h| h.title == "A").unwrap().line_idx;
        let heading_b = headings.iter().find(|h| h.title == "B").unwrap().line_idx;

        let mut folded = std::collections::HashSet::new();
        folded.insert(heading_a);
        let md = MarkdownView {
            lines,
            headings,
            folded,
            title: "t".into(),
            subtitle: String::new(),
            scroll: 0,
            cursor: 0,
            view_height: 0,
            outline_visible: false,
            outline_selected: 0,
            outline_focused: false,
        };

        let visible = visible_markdown_lines(&md);
        let folded_line = visible
            .iter()
            .find(|l| text_of(l).contains('A'))
            .expect("折りたたみ中の見出し行が見つからない");
        let text = text_of(folded_line);
        assert!(text.starts_with('▸'), "折りたたみグリフ ▸ が無い: {:?}", text);
        let hidden_count = heading_b - (heading_a + 1);
        assert!(
            text.contains(&format!("({}行)", hidden_count)),
            "隠れた行数が表示されていない（期待 {}行）: {:?}",
            hidden_count,
            text
        );
        // 隠れている本文行（body a1 / a2）は visible に含まれない
        assert!(!visible.iter().any(|l| text_of(l).contains("body a1")));
        // B 以降は表示される（展開中の見出しには行数が付かない）
        let heading_b_line = visible
            .iter()
            .find(|l| text_of(l).contains('B'))
            .expect("見出し B が見つからない");
        let b_text = text_of(heading_b_line);
        assert!(b_text.starts_with('▼'), "展開中は ▼: {:?}", b_text);
        assert!(!b_text.contains('行'), "展開中は行数を出さない: {:?}", b_text);
    }
}
