//! 読み取り専用の内蔵 Markdown ビューア。
//!
//! `sai-link-tui` の md-tui（`ui/markdown.rs` + `MarkdownView`）を本アプリの
//! `Theme` に合わせて移植したもの。見出しの折りたたみ・アウトライン（見出し一覧）
//! パネルに対応する。編集は行わない（閲覧のみ）。
//!
//! 対応要素:
//! - 見出し（H1〜H6）: レベルに応じた色 + 太字
//! - 段落 / 強調 / 斜体 / 取り消し線
//! - インラインコード / コードブロック（syntect によるシンタックスハイライト）
//! - 箇条書き / 番号付きリスト / タスクリスト
//! - 引用（blockquote）: `│` プレフィックス
//! - テーブル: 罫線付き表示
//! - リンク / 画像: リンク色・プレースホルダ表示
//! - YAML frontmatter: メタデータとして muted 表示
//! - 水平線: セパレータ行

use std::sync::OnceLock;

use pulldown_cmark::{Alignment, CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{
    Block, BorderType, Borders, List, ListItem, ListState, Paragraph, StatefulWidget, Widget, Wrap,
};
use syntect::easy::HighlightLines;
use syntect::highlighting::Theme as SyntectTheme;
use syntect::highlighting::ThemeSet;
use syntect::parsing::SyntaxSet;

use crate::theme::Theme;

/// 本文カーソル行の背景色（sai-link-tui と同じ）。
const MD_CURSOR_BG: Color = Color::Rgb(45, 45, 70);

/// シンタックスハイライト用の定義一式（プロセス全体で 1 回だけ読み込む）。
fn syntax_set() -> &'static SyntaxSet {
    static SYNTAX_SET: OnceLock<SyntaxSet> = OnceLock::new();
    SYNTAX_SET.get_or_init(SyntaxSet::load_defaults_newlines)
}

/// シンタックスハイライトの配色テーマ（プロセス全体で 1 回だけ読み込む）。
fn code_theme() -> &'static SyntectTheme {
    static CODE_THEME: OnceLock<SyntectTheme> = OnceLock::new();
    CODE_THEME.get_or_init(|| {
        let ts = ThemeSet::load_defaults();
        ts.themes
            .get("base16-ocean.dark")
            .or_else(|| ts.themes.values().next())
            .cloned()
            .expect("syntect デフォルトテーマが 1 つも読み込めない")
    })
}

// ============================================================================
// 公開 API
// ============================================================================

/// 見出し 1 件（アウトライン・折りたたみで使う）。
#[derive(Debug, Clone)]
pub struct HeadingEntry {
    /// 見出し行の `Vec<Line>` 内インデックス（折り返し前の論理行番号）。
    pub line_idx: usize,
    /// 見出しレベル（1=H1 .. 6=H6）。
    pub level: u8,
    /// 見出しテキスト（`#` などのマーク抜き）。
    pub title: String,
}

/// レンダリング済み Markdown の閲覧ビュー。
#[derive(Debug)]
pub struct MarkdownView {
    /// スタイル付きの表示行。
    pub lines: Vec<Line<'static>>,
    /// 見出し一覧（折りたたみ・アウトラインで使う）。
    pub headings: Vec<HeadingEntry>,
    /// 折りたたみ中の見出しの `line_idx` 集合。
    pub folded: std::collections::HashSet<usize>,
    /// タイトル（ファイル種別など）。
    pub title: String,
    /// サブタイトル（ファイルパスなど）。
    pub subtitle: String,
    /// スクロール行数（折り返し後の表示行数）。
    pub scroll: usize,
    /// カーソル行（折り返し前の論理行インデックス。折りたたみ反映後）。
    pub cursor: usize,
    /// 本文ビューポートの高さ（描画時に更新。カーソル追従に使う）。
    pub view_height: usize,
    /// アウトラインパネルを表示中か。
    pub outline_visible: bool,
    /// アウトラインパネルのカーソル位置（`headings` のインデックス）。
    pub outline_selected: usize,
    /// フォーカスがアウトラインパネルにあるか（`false` = 本文にフォーカス）。
    pub outline_focused: bool,
}

impl MarkdownView {
    /// Markdown 本文からビューを構築する（折りたたみ・アウトラインは初期状態＝非表示）。
    #[must_use]
    pub fn from_markdown(content: &str, theme: &Theme, title: String, subtitle: String) -> Self {
        let (lines, headings) = render_markdown(content, theme);
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
    fn heading_at_cursor(&self) -> Option<&HeadingEntry> {
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

    /// アウトラインパネルのカーソルを移動する（`headings` の範囲でクランプ）。
    pub fn outline_move(&mut self, delta: i32) {
        if self.headings.is_empty() {
            return;
        }
        let len = self.headings.len() as i32;
        let idx = (self.outline_selected as i32 + delta).clamp(0, len - 1);
        self.outline_selected = idx as usize;
    }

    /// アウトラインで選択中の見出しへ本文をジャンプさせ、折りたたまれていれば展開する。
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

    /// 見出しの番号（`3.2` など）に当たる最初の見出しへカーソルを移す。
    ///
    /// `3.2` は `3.2` / `3.2.` / `3.2 Methods` に当たり、`3.20 Results` には当たらない。
    /// 折りたたまれていれば展開する。見つからないときは動かさず `false` を返す。
    pub fn jump_to_heading_number(&mut self, number: &str) -> bool {
        let Some(index) = find_heading_by_number(&self.headings, number) else {
            return false;
        };
        let line_idx = self.headings[index].line_idx;
        self.folded.remove(&line_idx);
        if let Some(vis) = visible_line_indices(self)
            .iter()
            .position(|&i| i == line_idx)
        {
            self.cursor = vis;
        }
        self.ensure_cursor_visible();
        true
    }

    /// カーソル位置の表示行の文字を返す（引用の拾い出しに使う）。
    pub fn cursor_line_text(&self) -> Option<String> {
        let logical = visible_line_indices(self).get(self.cursor).copied()?;
        Some(line_text(self.lines.get(logical)?))
    }

    /// 引用（`[#37]` など）のある次（`delta > 0`）／前（`delta < 0`）の表示行へ
    /// カーソルを動かす。見つからないときは動かさず `false` を返す。
    pub fn cursor_move_to_citation(&mut self, delta: i32) -> bool {
        if delta == 0 {
            return false;
        }
        let visible = visible_line_indices(self);
        if visible.is_empty() {
            return false;
        }
        let step = i64::from(delta);
        let mut i = self.cursor as i64 + step;
        while i >= 0 && (i as usize) < visible.len() {
            if line_has_citation(&self.lines, visible[i as usize]) {
                self.cursor = i as usize;
                self.ensure_cursor_visible();
                return true;
            }
            i += step;
        }
        false
    }
}

/// `Line` の文字を連結して返す。
fn line_text(line: &Line) -> String {
    line.spans.iter().map(|s| s.content.as_ref()).collect()
}

/// 論理行に引用（`[#37]` / `[DOI:…]` など）があるか。
fn line_has_citation(lines: &[Line<'static>], index: usize) -> bool {
    let Some(line) = lines.get(index) else {
        return false;
    };
    !papr_core::reports::parse_citations(&line_text(line)).is_empty()
}

/// 見出しの一覧から、番号に当たる最初の見出しの index を返す。
fn find_heading_by_number(headings: &[HeadingEntry], number: &str) -> Option<usize> {
    let number = number.trim().trim_end_matches('.');
    if number.is_empty() {
        return None;
    }
    headings
        .iter()
        .position(|h| heading_number_matches(&h.title, number))
}

/// 見出しの文字が番号で始まるか。
///
/// `3.2` は `3.2` / `3.2.` / `3.2 Methods` に当たり、`3.20 Results` や `3.2.1 …` には
/// 当たらない（番号の並びがそのまま一致することを求める）。
fn heading_number_matches(title: &str, number: &str) -> bool {
    // `§3.2` のように記号が付く見出しも許す。
    let title = title.trim_start_matches(|c: char| c == '§' || c.is_whitespace());
    let mut end = 0usize;
    for (i, c) in title.char_indices() {
        if c.is_ascii_digit() || c == '.' {
            end = i + c.len_utf8();
        } else {
            break;
        }
    }
    if end == 0 {
        return false;
    }
    let prefix = title[..end].trim_end_matches('.');
    if prefix != number {
        return false;
    }
    // 番号の並びの直後は、終端か空白でなければ別の番号（`3.2x`）とみなす。
    matches!(title[end..].chars().next(), None | Some(' ') | Some('\t'))
}

/// 折りたたみ中の見出し `h` について、「次の同レベル以下見出し」の行番号
/// （無ければ全体の行数）を返す（畳んで隠れる範囲の終端。終端自身は含まない）。
fn fold_end_line(md: &MarkdownView, h: &HeadingEntry) -> usize {
    md.headings
        .iter()
        .filter(|o| o.line_idx > h.line_idx && o.level <= h.level)
        .map(|o| o.line_idx)
        .min()
        .unwrap_or(md.lines.len())
}

/// 折りたたみ中の見出し `h` の直後に隠れている行数。
fn fold_hidden_line_count(md: &MarkdownView, h: &HeadingEntry) -> usize {
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
fn visible_line_indices(md: &MarkdownView) -> Vec<usize> {
    let hidden = hidden_line_flags(md);
    (0..md.lines.len()).filter(|&i| !hidden[i]).collect()
}

/// 表示行ごとの折り返し後行数を返す（ratatui の折り返しと一致）。
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

/// コードブロックの行か（先頭スパンが `surface` 背景付きのガター `│ ` か）。
fn is_code_block_line(line: &Line) -> bool {
    line.spans
        .first()
        .map(|s| s.content == "│ " && s.style.bg.is_some())
        .unwrap_or(false)
}

/// 折りたたみを反映した表示用の行一覧を返す。見出し行には `▼`/`▸` を付ける。
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

/// 本文領域の指定スクリーン行範囲（折り返し後の行番号）を `color` で塗る。
fn fill_rows(
    buf: &mut ratatui::buffer::Buffer,
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

/// md ビューアのアウトラインパネル（見出し一覧）を描画する。
fn render_markdown_outline(
    md: &MarkdownView,
    area: Rect,
    buf: &mut ratatui::buffer::Buffer,
    theme: &Theme,
) {
    let border_color = if md.outline_focused {
        Color::Yellow
    } else {
        theme.muted
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(border_color).bg(theme.background))
        .title(" outline ")
        .title_style(Style::default().fg(border_color).bg(theme.background))
        .style(Style::default().bg(theme.background));
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
        .style(Style::default().fg(theme.text).bg(theme.background))
        .highlight_style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        );

    StatefulWidget::render(list, inner, buf, &mut state);
}

/// 内蔵 Markdown ビューアを全画面に描画する。
pub fn render_markdown_view(
    frame: &mut Frame<'_>,
    area: Rect,
    md: &mut MarkdownView,
    theme: &Theme,
) {
    let info = theme.accent;
    let muted = theme.muted;
    let bg = theme.background;

    // タイトル行
    let title = format!(" {}: {}", md.title, md.subtitle);
    let title_area = Rect::new(area.x + 1, area.y, area.width.saturating_sub(2), 1);
    frame.render_widget(
        Paragraph::new(title).style(Style::default().fg(info).bg(bg)),
        title_area,
    );

    // セパレータ
    let sep_area = Rect::new(area.x, area.y + 1, area.width, 1);
    frame.render_widget(
        Paragraph::new("─".repeat(area.width as usize)).style(Style::default().fg(muted).bg(bg)),
        sep_area,
    );

    // 本文（タイトル2行とガイド1行分を除く）
    let full_body_area = Rect::new(
        area.x + 1,
        area.y + 2,
        area.width.saturating_sub(2),
        area.height.saturating_sub(3),
    );

    let show_outline = md.outline_visible && !md.headings.is_empty();
    let (body_area, outline_area) = if show_outline {
        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Min(20), Constraint::Length(24)])
            .split(full_body_area);
        (cols[0], Some(cols[1]))
    } else {
        (full_body_area, None)
    };

    md.view_height = body_area.height as usize;
    let display_lines = visible_markdown_lines(md);
    let cursor = md.cursor;
    let scroll = md.scroll;
    let headings_empty = md.headings.is_empty();

    let heights = wrapped_row_counts(&display_lines, body_area.width);
    let mut row_offsets: Vec<usize> = Vec::with_capacity(heights.len() + 1);
    let mut acc = 0usize;
    row_offsets.push(0);
    for h in &heights {
        acc += *h;
        row_offsets.push(acc);
    }
    let code_line_idx: Vec<usize> = display_lines
        .iter()
        .enumerate()
        .filter(|(_, l)| is_code_block_line(l))
        .map(|(i, _)| i)
        .collect();

    let scroll_y = u16::try_from(scroll).unwrap_or(u16::MAX);
    let paragraph = Paragraph::new(Text::from(display_lines))
        .style(Style::default().bg(bg))
        .wrap(Wrap { trim: false })
        .scroll((scroll_y, 0));
    frame.render_widget(paragraph, body_area);

    // コードブロック行・カーソル行の背景をそろえる。
    let code_bg = theme.surface;
    let buf = frame.buffer_mut();
    for i in code_line_idx {
        let start = row_offsets.get(i).copied().unwrap_or(0);
        let end = row_offsets.get(i + 1).copied().unwrap_or(start + 1);
        fill_rows(buf, body_area, scroll, start, end, code_bg);
    }
    if cursor < row_offsets.len().saturating_sub(1) {
        let start = row_offsets[cursor];
        let end = row_offsets[cursor + 1];
        fill_rows(buf, body_area, scroll, start, end, MD_CURSOR_BG);
    }

    if let Some(outline_area) = outline_area {
        render_markdown_outline(md, outline_area, frame.buffer_mut(), theme);
    }

    // キー操作ガイド（下部）
    let guide = if headings_empty {
        "↑↓/jk/PgUp/PgDn move | Enter citation | n N citation | Esc close"
    } else {
        "↑↓/jk move | space/z fold | PgUp/PgDn page | o outline | Tab focus | Enter citation | n N citation | Esc close"
    };
    let guide_area = Rect::new(
        area.x + 1,
        area.bottom().saturating_sub(1),
        area.width.saturating_sub(2),
        1,
    );
    frame.render_widget(
        Paragraph::new(guide).style(Style::default().fg(muted).bg(bg)),
        guide_area,
    );
}

// ============================================================================
// レンダリング本体
// ============================================================================

/// Markdown 文字列をスタイル付きの行へ変換する。見出し一覧も併せて返す。
pub fn render_markdown(md: &str, theme: &Theme) -> (Vec<Line<'static>>, Vec<HeadingEntry>) {
    let mut opts = Options::empty();
    opts.insert(Options::ENABLE_TABLES);
    opts.insert(Options::ENABLE_FOOTNOTES);
    opts.insert(Options::ENABLE_STRIKETHROUGH);
    opts.insert(Options::ENABLE_TASKLISTS);
    opts.insert(Options::ENABLE_YAML_STYLE_METADATA_BLOCKS);

    let parser = Parser::new_ext(md, opts);
    let mut ctx = Ctx::new(theme);
    for event in parser {
        ctx.handle(event);
    }
    ctx.finish()
}

/// レンダリング中の状態。
struct Ctx<'a> {
    theme: &'a Theme,
    lines: Vec<Line<'static>>,
    cur: Vec<Span<'static>>,
    styles: Vec<Style>,
    style: Style,
    in_code_block: bool,
    in_metadata: bool,
    list_counters: Vec<u64>,
    quote_depth: usize,
    in_table: bool,
    in_table_head: bool,
    table_row: Vec<Vec<Span<'static>>>,
    table_cell: Vec<Span<'static>>,
    in_cell: bool,
    table_rows: Vec<(bool, Vec<Vec<Span<'static>>>)>,
    table_alignments: Vec<Alignment>,
    in_image: bool,
    current_link_url: Option<String>,
    in_heading: bool,
    heading_text_buf: String,
    headings: Vec<HeadingEntry>,
    highlighter: Option<HighlightLines<'static>>,
}

impl<'a> Ctx<'a> {
    fn new(theme: &'a Theme) -> Self {
        Self {
            theme,
            lines: Vec::new(),
            cur: Vec::new(),
            styles: Vec::new(),
            style: Style::default().fg(theme.text),
            in_code_block: false,
            in_metadata: false,
            list_counters: Vec::new(),
            quote_depth: 0,
            in_table: false,
            in_table_head: false,
            table_row: Vec::new(),
            table_cell: Vec::new(),
            in_cell: false,
            table_rows: Vec::new(),
            table_alignments: Vec::new(),
            in_image: false,
            current_link_url: None,
            in_heading: false,
            heading_text_buf: String::new(),
            headings: Vec::new(),
            highlighter: None,
        }
    }

    fn finish(mut self) -> (Vec<Line<'static>>, Vec<HeadingEntry>) {
        self.flush_line();
        (self.lines, self.headings)
    }

    fn handle(&mut self, event: Event<'_>) {
        match event {
            Event::Start(tag) => self.start(tag),
            Event::End(tag_end) => self.end(tag_end),
            Event::Text(text) => self.text(&text),
            Event::Code(text) => {
                let style = Style::default()
                    .fg(self.theme.success)
                    .bg(self.theme.surface);
                self.push_span(&text, style);
            }
            Event::SoftBreak | Event::HardBreak => self.flush_line(),
            Event::Rule => self.rule(),
            Event::TaskListMarker(checked) => {
                let mark = if checked { "[x] " } else { "[ ] " };
                self.push_span(mark, Style::default().fg(self.theme.warning));
            }
            Event::Html(_) => {}
            Event::InlineHtml(text) => {
                self.push_span(&text, self.style);
            }
            Event::InlineMath(text) | Event::DisplayMath(text) => {
                let style = Style::default().fg(self.theme.warning);
                self.push_span(&format!("${text}$"), style);
            }
            Event::FootnoteReference(_) => {}
        }
    }

    fn start(&mut self, tag: Tag<'_>) {
        match tag {
            Tag::Paragraph => {}
            Tag::Heading { level, .. } => {
                self.in_heading = true;
                self.heading_text_buf.clear();
                self.push_style(self.heading_style(level));
            }
            Tag::BlockQuote(_) => {
                self.quote_depth += 1;
                self.push_style(Style::default().fg(self.theme.muted));
            }
            Tag::CodeBlock(kind) => {
                self.in_code_block = true;
                let lang = match &kind {
                    CodeBlockKind::Fenced(l) if !l.trim().is_empty() => Some(l.as_ref()),
                    _ => None,
                };
                self.highlighter = lang
                    .and_then(|l| syntax_set().find_syntax_by_token(l))
                    .map(|syntax| HighlightLines::new(syntax, code_theme()));
                if let Some(l) = lang {
                    self.lines.push(Line::from(vec![
                        code_gutter_span(self.theme),
                        Span::styled(
                            l.to_string(),
                            Style::default()
                                .fg(self.theme.accent)
                                .bg(self.theme.surface),
                        ),
                    ]));
                }
            }
            Tag::List(Some(start)) => self.list_counters.push(start),
            Tag::List(None) => self.list_counters.push(0),
            Tag::Item => {
                let marker = self.item_marker();
                self.push_span(&marker, Style::default().fg(self.theme.success));
            }
            Tag::Emphasis => {
                self.push_style(Style::default().add_modifier(Modifier::ITALIC));
            }
            Tag::Strong => {
                self.push_style(Style::default().add_modifier(Modifier::BOLD));
            }
            Tag::Strikethrough => {
                self.push_style(Style::default().add_modifier(Modifier::CROSSED_OUT));
            }
            Tag::Link { dest_url, .. } => {
                self.push_style(
                    Style::default()
                        .fg(self.theme.secondary)
                        .add_modifier(Modifier::UNDERLINED),
                );
                self.current_link_url = Some(dest_url.to_string());
            }
            Tag::Image { dest_url, .. } => {
                self.in_image = true;
                self.push_span(
                    &format!("[image: {dest_url}]"),
                    Style::default().fg(self.theme.muted),
                );
            }
            Tag::Table(alignments) => {
                self.in_table = true;
                self.table_rows.clear();
                self.table_alignments = alignments;
            }
            Tag::TableHead => {
                self.in_table_head = true;
            }
            Tag::TableRow => {
                self.table_row.clear();
            }
            Tag::TableCell => {
                self.in_cell = true;
                self.table_cell.clear();
            }
            Tag::MetadataBlock(_) => {
                self.in_metadata = true;
                self.lines.push(Line::from(vec![Span::styled(
                    "── metadata ──",
                    Style::default().fg(self.theme.muted),
                )]));
            }
            Tag::FootnoteDefinition(_) => {}
            Tag::HtmlBlock => {}
            _ => {}
        }
    }

    fn end(&mut self, tag_end: TagEnd) {
        match tag_end {
            TagEnd::Paragraph => {
                self.flush_line();
                self.blank_line();
            }
            TagEnd::Heading(level) => {
                self.flush_line();
                self.pop_style();
                if self.in_heading {
                    self.headings.push(HeadingEntry {
                        line_idx: self.lines.len().saturating_sub(1),
                        level: heading_level_to_u8(level),
                        title: std::mem::take(&mut self.heading_text_buf),
                    });
                }
                self.in_heading = false;
                self.blank_line();
            }
            TagEnd::BlockQuote(_) => {
                self.quote_depth = self.quote_depth.saturating_sub(1);
                self.pop_style();
            }
            TagEnd::CodeBlock => {
                self.in_code_block = false;
                self.highlighter = None;
                self.blank_line();
            }
            TagEnd::List(_) => {
                self.list_counters.pop();
                self.blank_line();
            }
            TagEnd::Item => self.flush_line(),
            TagEnd::Emphasis | TagEnd::Strong | TagEnd::Strikethrough => self.pop_style(),
            TagEnd::Link => {
                self.pop_style();
                if let Some(url) = self.current_link_url.take() {
                    self.push_span(&format!(" ({url})"), Style::default().fg(self.theme.muted));
                }
            }
            TagEnd::Image => {
                self.in_image = false;
            }
            TagEnd::Table => {
                self.in_table = false;
                self.render_table();
                self.blank_line();
            }
            TagEnd::TableHead => {
                self.finish_table_row(true);
                self.in_table_head = false;
            }
            TagEnd::TableRow => {
                self.finish_table_row(false);
            }
            TagEnd::TableCell => {
                self.in_cell = false;
                let cell = std::mem::take(&mut self.table_cell);
                self.table_row.push(cell);
            }
            TagEnd::MetadataBlock(_) => {
                self.in_metadata = false;
                self.blank_line();
            }
            TagEnd::FootnoteDefinition => {}
            _ => {}
        }
    }

    fn text(&mut self, text: &str) {
        if self.in_image {
            return;
        }
        if self.in_code_block {
            self.push_code_line(text);
            return;
        }
        if self.in_metadata {
            self.push_metadata_line(text);
            return;
        }
        if self.in_heading {
            self.heading_text_buf.push_str(text);
        }
        self.push_span(text, self.style);
    }

    fn push_span(&mut self, text: &str, style: Style) {
        if text.is_empty() {
            return;
        }
        let span = Span::styled(text.to_string(), style);
        if self.in_cell {
            self.table_cell.push(span);
        } else {
            self.cur.push(span);
        }
    }

    fn flush_line(&mut self) {
        if self.cur.is_empty() && self.quote_depth == 0 {
            return;
        }
        let mut spans = Vec::new();
        for _ in 0..self.quote_depth {
            spans.push(Span::styled("│ ", Style::default().fg(self.theme.muted)));
        }
        spans.append(&mut self.cur);
        self.lines.push(Line::from(spans));
        self.cur = Vec::new();
    }

    fn blank_line(&mut self) {
        if let Some(last) = self.lines.last() {
            if last.spans.is_empty() {
                return;
            }
        } else {
            return;
        }
        self.lines.push(Line::default());
    }

    fn rule(&mut self) {
        self.flush_line();
        self.lines.push(Line::from(vec![Span::styled(
            "──────────────────",
            Style::default().fg(self.theme.muted),
        )]));
    }

    fn push_code_line(&mut self, text: &str) {
        let mut parts: Vec<&str> = text.split('\n').collect();
        if parts.last() == Some(&"") {
            parts.pop();
        }
        if let Some(mut hl) = self.highlighter.take() {
            for line in &parts {
                let mut spans = vec![code_gutter_span(self.theme)];
                spans.extend(highlight_code_line(&mut hl, line, self.theme.surface));
                self.lines.push(Line::from(spans));
            }
            self.highlighter = Some(hl);
            return;
        }
        let style = Style::default().fg(self.theme.muted).bg(self.theme.surface);
        for line in parts {
            self.lines.push(Line::from(vec![
                code_gutter_span(self.theme),
                Span::styled(line.to_string(), style),
            ]));
        }
    }

    fn push_metadata_line(&mut self, text: &str) {
        let style = Style::default().fg(self.theme.muted);
        for line in text.split('\n') {
            self.lines
                .push(Line::from(vec![Span::styled(format!("  {line}"), style)]));
        }
    }

    fn finish_table_row(&mut self, is_head: bool) {
        if self.in_cell {
            self.in_cell = false;
            let cell = std::mem::take(&mut self.table_cell);
            self.table_row.push(cell);
        }
        let row = std::mem::take(&mut self.table_row);
        self.table_rows.push((is_head, row));
    }

    fn render_table(&mut self) {
        let rows = std::mem::take(&mut self.table_rows);
        let alignments = std::mem::take(&mut self.table_alignments);
        if rows.is_empty() {
            return;
        }
        let ncols = rows.iter().map(|(_, cells)| cells.len()).max().unwrap_or(0);
        if ncols == 0 {
            return;
        }

        let rows: Vec<(bool, Vec<Vec<Vec<Span<'static>>>>)> = rows
            .into_iter()
            .map(|(is_head, cells)| {
                (
                    is_head,
                    cells.into_iter().map(split_cell_spans_by_br).collect(),
                )
            })
            .collect();

        let mut natural = vec![1usize; ncols];
        for (_, cells) in &rows {
            for (i, cell_lines) in cells.iter().enumerate() {
                let w = cell_lines.iter().map(|l| spans_width(l)).max().unwrap_or(0);
                natural[i] = natural[i].max(w);
            }
        }

        const TABLE_MAX_WIDTH: usize = 100;
        let overhead = 2 + ncols.saturating_sub(1) + ncols * 2;
        let budget = TABLE_MAX_WIDTH.saturating_sub(overhead);
        let widths = cap_column_widths(&natural, budget);

        let border_style = Style::default().fg(self.theme.muted);
        self.lines
            .push(border_line(&widths, '┌', '┬', '┐', border_style));
        for (is_head, cells) in &rows {
            let row_height = cells.iter().map(|c| c.len()).max().unwrap_or(1).max(1);
            for line_idx in 0..row_height {
                let mut spans = vec![Span::styled("│", border_style)];
                for (col, cell_lines) in cells.iter().enumerate() {
                    let w = widths.get(col).copied().unwrap_or(0);
                    let align = alignments.get(col).copied().unwrap_or(Alignment::None);
                    let content = cell_lines.get(line_idx).cloned().unwrap_or_default();
                    let pad = w.saturating_sub(spans_width(&content));
                    let (lp, rp) = match align {
                        Alignment::Right => (pad, 0),
                        Alignment::Center => (pad / 2, pad - pad / 2),
                        Alignment::Left | Alignment::None => (0, pad),
                    };
                    spans.push(Span::raw(" ".repeat(lp + 1)));
                    for mut s in content {
                        if *is_head {
                            s.style = s.style.add_modifier(Modifier::BOLD);
                        }
                        spans.push(s);
                    }
                    spans.push(Span::raw(" ".repeat(rp + 1)));
                    spans.push(Span::styled("│", border_style));
                }
                self.lines.push(Line::from(spans));
            }
            if *is_head {
                self.lines
                    .push(border_line(&widths, '├', '┼', '┤', border_style));
            }
        }
        self.lines
            .push(border_line(&widths, '└', '┴', '┘', border_style));
    }

    fn push_style(&mut self, style: Style) {
        self.style = self.style.patch(style);
        self.styles.push(style);
    }

    fn pop_style(&mut self) {
        self.styles.pop();
        let mut s = Style::default().fg(self.theme.text);
        for st in &self.styles {
            s = s.patch(*st);
        }
        self.style = s;
    }

    fn heading_style(&self, level: HeadingLevel) -> Style {
        let color = match level {
            HeadingLevel::H1 | HeadingLevel::H2 => self.theme.success,
            HeadingLevel::H3 | HeadingLevel::H4 => self.theme.accent,
            _ => self.theme.text,
        };
        Style::default().fg(color).add_modifier(Modifier::BOLD)
    }

    fn item_marker(&mut self) -> String {
        if let Some(counter) = self.list_counters.last_mut() {
            if *counter > 0 {
                let n = *counter;
                *counter += 1;
                return format!("{n}. ");
            }
        }
        match self.list_counters.len().saturating_sub(1) % 3 {
            0 => "• ".to_string(),
            1 => "◦ ".to_string(),
            _ => "▪ ".to_string(),
        }
    }
}

/// コードブロックの左ガター（`│ `）。
fn code_gutter_span(theme: &Theme) -> Span<'static> {
    Span::styled("│ ", Style::default().fg(theme.muted).bg(theme.surface))
}

/// コードブロックの 1 行を syntect でトークンごとに色分けする。
fn highlight_code_line(hl: &mut HighlightLines, line: &str, code_bg: Color) -> Vec<Span<'static>> {
    let with_nl = format!("{line}\n");
    match hl.highlight_line(&with_nl, syntax_set()) {
        Ok(ranges) => ranges
            .into_iter()
            .filter_map(|(style, piece)| {
                let piece = piece.trim_end_matches('\n');
                if piece.is_empty() {
                    return None;
                }
                let c = style.foreground;
                Some(Span::styled(
                    piece.to_string(),
                    Style::default().fg(Color::Rgb(c.r, c.g, c.b)).bg(code_bg),
                ))
            })
            .collect(),
        Err(_) => vec![Span::styled(line.to_string(), Style::default().bg(code_bg))],
    }
}

/// 見出しレベルを 1(H1)..6(H6) の数値に変換する。
fn heading_level_to_u8(level: HeadingLevel) -> u8 {
    match level {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}

// ============================================================================
// テーブル整形
// ============================================================================

/// テーブル列の最小幅。
const TABLE_COLUMN_MIN_WIDTH: usize = 8;

/// 各列の自然な幅を、予算 `available` に収まるまで縮める。
fn cap_column_widths(natural: &[usize], available: usize) -> Vec<usize> {
    let mut widths: Vec<usize> = natural.to_vec();
    if widths.is_empty() {
        return widths;
    }
    loop {
        let total: usize = widths.iter().sum();
        if total <= available {
            return widths;
        }
        let mut target: Option<usize> = None;
        let mut max_w: usize = 0;
        for (i, &w) in widths.iter().enumerate() {
            let floor = TABLE_COLUMN_MIN_WIDTH.min(natural[i]);
            if w > floor && w > max_w {
                max_w = w;
                target = Some(i);
            }
        }
        match target {
            Some(i) => widths[i] -= 1,
            None => return widths,
        }
    }
}

/// セル内の `<br>` 系タグを境界に、Span 列を複数行へ分割する。
fn split_cell_spans_by_br(spans: Vec<Span<'static>>) -> Vec<Vec<Span<'static>>> {
    let mut lines: Vec<Vec<Span<'static>>> = vec![Vec::new()];
    for span in spans {
        let is_br = matches!(
            span.content.as_ref().to_ascii_lowercase().as_str(),
            "<br>" | "<br/>" | "<br />"
        );
        if is_br {
            lines.push(Vec::new());
        } else {
            lines
                .last_mut()
                .expect("最低 1 行は必ず存在する")
                .push(span);
        }
    }
    lines
}

/// Span 列の表示幅（全角文字対応）。
fn spans_width(spans: &[Span]) -> usize {
    spans.iter().map(Span::width).sum()
}

/// テーブルの罫線行（`┌───┬───┐` 等）を 1 行組み立てる。
fn border_line(
    widths: &[usize],
    left: char,
    mid: char,
    right: char,
    style: Style,
) -> Line<'static> {
    let mut s = String::new();
    s.push(left);
    for (i, w) in widths.iter().enumerate() {
        if i > 0 {
            s.push(mid);
        }
        for _ in 0..w + 2 {
            s.push('─');
        }
    }
    s.push(right);
    Line::from(vec![Span::styled(s, style)])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn theme() -> Theme {
        Theme::load("catppuccin-mocha").expect("builtin theme")
    }

    fn text_of(line: &Line) -> String {
        line.spans.iter().map(|s| s.content.as_ref()).collect()
    }

    /// 見出しが抽出され、レベルと本文が正しく取れる。
    #[test]
    fn headings_are_extracted() {
        let (lines, headings) = render_markdown("# Title\n\nbody\n\n## Sub", &theme());
        assert_eq!(headings.len(), 2);
        assert_eq!(headings[0].level, 1);
        assert_eq!(headings[0].title, "Title");
        assert_eq!(text_of(&lines[headings[0].line_idx]), "Title");
        assert_eq!(headings[1].level, 2);
        assert_eq!(headings[1].title, "Sub");
    }

    /// 見出しを折りたたむと、その配下の行が非表示になる。
    #[test]
    fn folding_hides_child_lines() {
        let md = "# A\n\n## B\n\ntext-b\n\n# C\n\ntext-c";
        let mut view = MarkdownView::from_markdown(md, &theme(), "t".into(), "s".into());
        let total = view.lines.len();
        assert!(total > 3);
        // 先頭見出し A を折りたたむ（B とその配下が隠れる）。
        view.toggle_fold_at_cursor();
        let visible = visible_line_indices(&view).len();
        assert!(visible < total, "折りたたみで行数が減るはず");
        assert!(visible >= 2, "C と本文は残るはず");
    }

    /// 空文字列でもクラッシュしない。
    #[test]
    fn empty_input() {
        let (lines, headings) = render_markdown("", &theme());
        assert!(lines.is_empty());
        assert!(headings.is_empty());
    }

    /// 見出しの番号の当て方（AC1）。
    #[test]
    fn heading_number_matching_rules() {
        assert!(heading_number_matches("3.2 Methods", "3.2"));
        assert!(heading_number_matches("3.2. Methods", "3.2"));
        assert!(heading_number_matches("3.2", "3.2"));
        assert!(heading_number_matches("§3.2 手法", "3.2"));
        assert!(!heading_number_matches("3.20 Results", "3.2"));
        assert!(!heading_number_matches("3.2.1 Details", "3.2"));
        assert!(!heading_number_matches("Methods", "3.2"));
        assert!(!heading_number_matches("3.2x", "3.2"));
    }

    /// 見出しの番号へのジャンプ（AC1）。
    #[test]
    fn jump_to_heading_number_moves_cursor() {
        let md = "# 1 Intro\n\n## 3.2 Methods\n\ntext\n\n## 3.20 Results\n";
        let mut view = MarkdownView::from_markdown(md, &theme(), "t".into(), "s".into());
        assert!(view.jump_to_heading_number("3.2"), "3.2 が当たるはず");
        assert_eq!(
            view.cursor_line_text().as_deref(),
            Some("3.2 Methods"),
            "カーソルが 3.2 の見出しに乗るはず"
        );
        assert!(
            !view.jump_to_heading_number("9.9"),
            "無い番号では動かないはず"
        );
    }

    /// 引用の行の移動（AC2）。
    #[test]
    fn citation_cursor_moves_between_citations() {
        let md = "first line\n\nsee [#37 §3.2] here\n\nmiddle\n\nand [#40]\n\nlast";
        let mut view = MarkdownView::from_markdown(md, &theme(), "t".into(), "s".into());
        assert!(view.cursor_move_to_citation(1));
        assert!(view.cursor_line_text().unwrap().contains("[#37"));
        assert!(view.cursor_move_to_citation(1));
        assert!(view.cursor_line_text().unwrap().contains("[#40]"));
        assert!(!view.cursor_move_to_citation(1), "後ろに引用は無い");
        assert!(view.cursor_move_to_citation(-1));
        assert!(view.cursor_line_text().unwrap().contains("[#37"));
        assert!(!view.cursor_move_to_citation(-1), "前に引用は無い");
    }

    /// 引用の無い md では動かない（AC2）。
    #[test]
    fn citation_cursor_is_still_without_citations() {
        let md = "just text\n\nno brackets here";
        let mut view = MarkdownView::from_markdown(md, &theme(), "t".into(), "s".into());
        assert!(!view.cursor_move_to_citation(1));
        assert!(!view.cursor_move_to_citation(-1));
        assert_eq!(view.cursor, 0);
    }

    /// 外部の引用（DOI）や見出しの引用だけの md でも、行の移動は引用を拾う。
    #[test]
    fn citation_cursor_picks_external_citations() {
        let md = "intro\n\na [DOI:10.1000/xyz] b\n\nc [未確認: 著者 年] d";
        let mut view = MarkdownView::from_markdown(md, &theme(), "t".into(), "s".into());
        assert!(view.cursor_move_to_citation(1));
        assert!(view.cursor_line_text().unwrap().contains("DOI:10.1000/xyz"));
        assert!(view.cursor_move_to_citation(1));
        assert!(view.cursor_line_text().unwrap().contains("未確認"));
    }
}
