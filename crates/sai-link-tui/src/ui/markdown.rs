//! マークダウンレンダリング（pulldown-cmark）。
//!
//! History / Blocks モードで、セッション出力やブロック定義ファイル（.md）を
//! スタイル付きの行（`Line`）へ変換して表示するためのレンダラ。
//!
//! 対応要素:
//! - 見出し（H1〜H6）: レベルに応じた色 + 太字
//! - 段落 / 強調 / 斜体 / 取り消し線
//! - インラインコード / コードブロック
//! - 箇条書き / 番号付きリスト / タスクリスト
//! - 引用（blockquote）: `│` プレフィックス
//! - テーブル: `|` 区切りの簡易表示（ヘッダーは太字）
//! - リンク / 画像: リンク色で表示
//! - YAML frontmatter: メタデータとして muted 表示
//! - 水平線: セパレータ行

use std::sync::OnceLock;

use pulldown_cmark::{Alignment, CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use syntect::easy::HighlightLines;
use syntect::highlighting::{Theme, ThemeSet};
use syntect::parsing::SyntaxSet;

use sai_link_widgets::theme::AppTheme;

/// シンタックスハイライト用の定義一式（プロセス全体で 1 回だけ読み込む）。
fn syntax_set() -> &'static SyntaxSet {
    static SYNTAX_SET: OnceLock<SyntaxSet> = OnceLock::new();
    SYNTAX_SET.get_or_init(SyntaxSet::load_defaults_newlines)
}

/// シンタックスハイライトの配色テーマ（プロセス全体で 1 回だけ読み込む）。
fn code_theme() -> &'static Theme {
    static CODE_THEME: OnceLock<Theme> = OnceLock::new();
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
    /// 見出し行の `Vec<Line>` 内インデックス（折り返し前の論理行番号）
    pub line_idx: usize,
    /// 見出しレベル（1=H1 .. 6=H6）
    pub level: u8,
    /// 見出しテキスト（`#` などのマーク抜き）
    pub title: String,
}

/// markdown 文字列をスタイル付きの行へ変換する。見出し一覧も併せて返す。
pub fn render_markdown(md: &str, theme: &AppTheme) -> (Vec<Line<'static>>, Vec<HeadingEntry>) {
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

// ============================================================================
// レンダリング状態
// ============================================================================

/// レンダリング中の状態。
struct Ctx<'a> {
    theme: &'a AppTheme,
    /// 出力行
    lines: Vec<Line<'static>>,
    /// 現在の行に積むスパン
    cur: Vec<Span<'static>>,
    /// インラインスタイルのスタック（強調 / リンク / 引用など）
    styles: Vec<Style>,
    /// 現在のインラインスタイル（`styles` の合成結果）
    style: Style,
    /// コードブロック内か（Text は行単位で出力）
    in_code_block: bool,
    /// メタデータ（frontmatter）ブロック内か
    in_metadata: bool,
    /// ネストしたリストの番号カウンタ（0 = 箇条書き）
    list_counters: Vec<u64>,
    /// 引用の深さ
    quote_depth: usize,
    /// テーブル中か
    in_table: bool,
    /// テーブルのヘッダー行か
    in_table_head: bool,
    /// 現在のテーブル行のセル一覧
    table_row: Vec<Vec<Span<'static>>>,
    /// 現在のセルのスパン
    table_cell: Vec<Span<'static>>,
    /// セル内か
    in_cell: bool,
    /// テーブル全体の行バッファ（ヘッダーか, セルごとの Span 列）。
    /// `TagEnd::Table` でまとめて列幅を決めて罫線付きで整形する。
    table_rows: Vec<(bool, Vec<Vec<Span<'static>>>)>,
    /// テーブルの列寄せ（GFM の `:---`/`---:`/`:---:`）
    table_alignments: Vec<Alignment>,
    /// image 内か（true の間、alt テキストの Text イベントは無視する。
    /// 代わりに `[image: <path>]` プレースホルダを Tag::Image 開始時に出す）
    in_image: bool,
    /// link 内のリンク先 URL（TagEnd::Link で `(url)` として追記した後にクリアする）
    current_link_url: Option<String>,
    /// 見出し内か（`Some` なら見出しテキストを `heading_text_buf` に積む）
    in_heading: bool,
    /// 見出しテキストの蓄積バッファ
    heading_text_buf: String,
    /// 見出し一覧（アウトライン・折りたたみ用）
    headings: Vec<HeadingEntry>,
    /// コードブロックの言語別シンタックスハイライタ（対応言語のフェンス内のみ `Some`）
    highlighter: Option<HighlightLines<'static>>,
}

impl<'a> Ctx<'a> {
    fn new(theme: &'a AppTheme) -> Self {
        Self {
            theme,
            lines: Vec::new(),
            cur: Vec::new(),
            styles: Vec::new(),
            style: Style::default().fg(theme.fg),
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

    // -------------------------------------------------------------------
    // イベント処理
    // -------------------------------------------------------------------

    fn handle(&mut self, event: Event<'_>) {
        match event {
            Event::Start(tag) => self.start(tag),
            Event::End(tag_end) => self.end(tag_end),
            Event::Text(text) => self.text(&text),
            Event::Code(text) => {
                // インラインコード: ekphos と同じくバッククォートを外し、
                // マゼンタ色＋ code_bg 背景で表示する。
                let style = Style::default()
                    .fg(self.theme.inline_code)
                    .bg(self.theme.code_bg);
                self.push_span(&text, style);
            }
            Event::SoftBreak | Event::HardBreak => self.flush_line(),
            Event::Rule => self.rule(),
            Event::TaskListMarker(checked) => {
                let mark = if checked { "[x] " } else { "[ ] " };
                self.push_span(mark, Style::default().fg(self.theme.warning));
            }
            Event::Html(_) => {} // HTML は表示しない
            Event::InlineHtml(text) => {
                // インライン HTML はテキストとして残す（タグが崩れない程度）
                self.push_span(&text, self.style);
            }
            Event::InlineMath(text) | Event::DisplayMath(text) => {
                let style = Style::default().fg(self.theme.warning);
                self.push_span(&format!("${}$", text), style);
            }
            Event::FootnoteReference(_) => {} // 注釈参照は表示しない
        }
    }

    /// 開始タグ。
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
                // 引用内のテキストは muted で表示
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
                // 冒頭に言語ラベル（ekphos 風のガター │ 付き）。言語未指定なら出さない。
                if let Some(l) = lang {
                    self.lines.push(Line::from(vec![
                        code_gutter_span(self.theme),
                        Span::styled(
                            l.to_string(),
                            Style::default().fg(self.theme.info).bg(self.theme.code_bg),
                        ),
                    ]));
                }
            }
            Tag::List(Some(start)) => {
                // ネスト開始前に親行を確定する（子マーカーが親行に連結されるのを防ぐ）
                if !self.cur.is_empty() {
                    self.flush_line();
                }
                self.list_counters.push(start);
            }
            Tag::List(None) => {
                if !self.cur.is_empty() {
                    self.flush_line();
                }
                self.list_counters.push(0);
            }
            Tag::Item => {
                let marker = self.item_marker();
                self.push_span(&marker, Style::default().fg(self.theme.success));
            }
            Tag::Emphasis => {
                self.push_style(Style::default().add_modifier(Modifier::ITALIC));
            }
            Tag::Strong => {
                self.push_style(
                    Style::default()
                        .fg(self.theme.strong)
                        .add_modifier(Modifier::BOLD)
                );
            }
            Tag::Strikethrough => {
                self.push_style(Style::default().add_modifier(Modifier::CROSSED_OUT));
            }
            Tag::Link { dest_url, .. } => {
                // リンクテキストはリンク色 + 下線。`(url)` は TagEnd::Link で追記する。
                self.push_style(
                    Style::default()
                        .fg(self.theme.link)
                        .add_modifier(Modifier::UNDERLINED),
                );
                self.current_link_url = Some(dest_url.to_string());
            }
            Tag::Image { dest_url, .. } => {
                // 実画像表示は ratatui-image（ratatui 0.30 化）が前提のため今回は対象外。
                // alt テキストは無視し、`[image: <path>]` プレースホルダをミュートカラーで表示する。
                self.in_image = true;
                self.push_span(
                    &format!("[image: {}]", dest_url),
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
                // frontmatter の開始ラベル
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

    /// 終了タグ。
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
                    self.push_span(
                        &format!(" ({})", url),
                        Style::default().fg(self.theme.muted),
                    );
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
                // ヘッダー行は TableRow を経由せず TableHead 内に直接セルが来るため、
                // in_table_head が true のうちに行を確定する（バッファへ積むだけ。
                // 罫線・幅・太字化は TagEnd::Table でまとめて行う）
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

    /// テキストイベント。
    fn text(&mut self, text: &str) {
        if self.in_image {
            // alt テキストは `[image: <path>]` プレースホルダに置き換え済みのため無視する
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

    // -------------------------------------------------------------------
    // 出力ヘルパー
    // -------------------------------------------------------------------

    /// 現在の行にスパンを追加する（セル内ならセルに追加）。
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

    /// 現在の行を確定して出力する（引用プレフィックス付き）。
    fn flush_line(&mut self) {
        if self.cur.is_empty() && self.quote_depth == 0 {
            return;
        }
        let mut spans = Vec::new();
        // 引用プレフィックス（深さ分）
        for _ in 0..self.quote_depth {
            spans.push(Span::styled("│ ", Style::default().fg(self.theme.muted)));
        }
        spans.append(&mut self.cur);
        self.lines.push(Line::from(spans));
        self.cur = Vec::new();
    }

    /// 段落・ブロックの切れ目に空行を追加する（連続する空行は抑制）。
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

    /// 水平線（セパレータ行）。
    fn rule(&mut self) {
        self.flush_line();
        self.lines.push(Line::from(vec![Span::styled(
            "──────────────────",
            Style::default().fg(self.theme.muted),
        )]));
    }

    /// コードブロック内の行を出力する。左に ekphos 風のガター `│ ` を付ける。
    ///
    /// コードブロック内の Text イベントは行全体（末尾に `\n` を含む）を運ぶため、
    /// `\n` で分割して 1 行ずつ出力する。末尾の改行による空要素は除き、
    /// コード内の意図的な空行は保持する。
    fn push_code_line(&mut self, text: &str) {
        let mut parts: Vec<&str> = text.split('\n').collect();
        // 末尾の改行による空要素を除去（コードブロック内の意図的な空行は残す）
        if parts.last() == Some(&"") {
            parts.pop();
        }
        if let Some(mut hl) = self.highlighter.take() {
            for line in &parts {
                let mut spans = vec![code_gutter_span(self.theme)];
                spans.extend(highlight_code_line(&mut hl, line, self.theme.code_bg));
                self.lines.push(Line::from(spans));
            }
            self.highlighter = Some(hl);
            return;
        }
        let style = Style::default().fg(self.theme.muted).bg(self.theme.code_bg);
        for line in parts {
            self.lines.push(Line::from(vec![
                code_gutter_span(self.theme),
                Span::styled(line.to_string(), style),
            ]));
        }
    }

    /// メタデータ（frontmatter）内の行を出力する。
    fn push_metadata_line(&mut self, text: &str) {
        let style = Style::default().fg(self.theme.muted);
        for line in text.split('\n') {
            self.lines
                .push(Line::from(vec![Span::styled(format!("  {}", line), style)]));
        }
    }

    /// テーブルの 1 行を確定し、行バッファへ積む（罫線・幅の確定は `render_table` で行う）。
    fn finish_table_row(&mut self, is_head: bool) {
        // 途中でセルが閉じていない場合の保険
        if self.in_cell {
            self.in_cell = false;
            let cell = std::mem::take(&mut self.table_cell);
            self.table_row.push(cell);
        }
        let row = std::mem::take(&mut self.table_row);
        self.table_rows.push((is_head, row));
    }

    /// テーブル全体（`table_rows` に貯めた全行）を罫線付きで整形して出力する。
    ///
    /// 列幅は `cap_column_widths`（ekphos 移植）で画面幅に収める。
    /// セル内の `<br>` 系タグはセル内改行として複数行セルに分ける。
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

        // 各セルを <br> で複数行（セル内改行）に分割する。
        let rows: Vec<(bool, Vec<Vec<Vec<Span<'static>>>>)> = rows
            .into_iter()
            .map(|(is_head, cells)| {
                (
                    is_head,
                    cells.into_iter().map(split_cell_spans_by_br).collect(),
                )
            })
            .collect();

        // 列ごとの自然な幅（内容の最大表示幅）。
        let mut natural = vec![1usize; ncols];
        for (_, cells) in &rows {
            for (i, cell_lines) in cells.iter().enumerate() {
                let w = cell_lines.iter().map(|l| spans_width(l)).max().unwrap_or(0);
                natural[i] = natural[i].max(w);
            }
        }

        // 画面幅に収める。ponytail: 実ビューポート幅は render_markdown に渡っていないため
        // 固定の目安幅を使う。可変にするならビューポート幅を引き回す。
        const TABLE_MAX_WIDTH: usize = 100;
        let overhead = 2 + ncols.saturating_sub(1) + ncols * 2; // 外枠│2 + 列間│ + 各セル左右パディング
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

    // -------------------------------------------------------------------
    // スタイル管理
    // -------------------------------------------------------------------

    /// インラインスタイルをスタックに積む。
    fn push_style(&mut self, style: Style) {
        self.style = self.style.patch(style);
        self.styles.push(style);
    }

    /// インラインスタイルをスタックから取り出す。
    fn pop_style(&mut self) {
        self.styles.pop();
        // スタックの内容を再合成して現在スタイルを復元する
        let mut s = Style::default().fg(self.theme.fg);
        for st in &self.styles {
            s = s.patch(*st);
        }
        self.style = s;
    }

    /// 見出しレベルに応じたスタイル。
    ///
    /// 階層が深いほど暗い同系色にする（`#` が最も明るく、`######` が最も暗い）。
    fn heading_style(&self, level: HeadingLevel) -> Style {
        Style::default()
            .fg(heading_color(self.theme.heading, heading_level_to_u8(level)))
            .add_modifier(Modifier::BOLD)
    }

    /// リスト項目のマーカー文字列を生成する。
    ///
    /// 番号付きリストは `N. ` を返してカウンタを進める。
    /// 箇条書きはネストの深さに応じて記号を変える。
    /// ネストの深さに応じたインデント（2 スペース/段）を前置する。
    fn item_marker(&mut self) -> String {
        let depth = self.list_counters.len().saturating_sub(1);
        let indent = "  ".repeat(depth);
        if let Some(counter) = self.list_counters.last_mut() {
            if *counter > 0 {
                let n = *counter;
                *counter += 1;
                return format!("{}{}. ", indent, n);
            }
        }
        let bullet = match depth % 3 {
            0 => "• ",
            1 => "◦ ",
            _ => "▪ ",
        };
        format!("{}{}", indent, bullet)
    }
}

/// コードブロックの左ガター（`│ `。ekphos 風）。行背景と揃えるため `code_bg` を敷く。
fn code_gutter_span(theme: &AppTheme) -> Span<'static> {
    Span::styled("│ ", Style::default().fg(theme.muted).bg(theme.code_bg))
}

/// コードブロックの 1 行を syntect でトークンごとに色分けする。
///
/// 対応言語が見つからない場合は呼び出し側が単色フォールバックを使う
/// （`highlighter` が `None` のときはこの関数自体が呼ばれない）。
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
        Err(_) => vec![Span::styled(
            line.to_string(),
            Style::default().bg(code_bg),
        )],
    }
}

/// 見出しレベル別の明るさ（1=最も明るい .. 6=最も暗い）。
///
/// テーマの見出し色を基準に、階層が深くなるほど暗くした同系色を使う。
const HEADING_BRIGHTNESS: [f32; 6] = [1.15, 1.0, 0.8, 0.62, 0.48, 0.38];

/// 見出しレベル（1..6）に対応する色を基準色から算出する。
///
/// 基準色が RGB でない場合はそのまま返す（テーマ側の色を優先）。
fn heading_color(base: Color, level: u8) -> Color {
    let f = HEADING_BRIGHTNESS[usize::from(level.clamp(1, 6)) - 1];
    match base {
        Color::Rgb(r, g, b) => Color::Rgb(
            scale_channel(r, f),
            scale_channel(g, f),
            scale_channel(b, f),
        ),
        other => other,
    }
}

/// 1 チャンネルを明るさ係数でスケールする（0..=255 にクランプ）。
fn scale_channel(v: u8, f: f32) -> u8 {
    (f32::from(v) * f).round().clamp(0.0, 255.0) as u8
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
// テーブル整形（ekphos `ui/content/tables.rs` 移植）
// ============================================================================

/// テーブル列の最小幅（この幅未満へは縮めない。ただし自然幅がこれ未満ならそのまま）。
const TABLE_COLUMN_MIN_WIDTH: usize = 8;

/// 各列の「自然な幅」（内容の最大幅）を、画面幅の予算 `available` に収まるまで
/// 縮める。最も広い列から 1 文字ずつ縮めるため、狭い列はできる限り元の幅を保つ。
/// 各列は自然幅が `TABLE_COLUMN_MIN_WIDTH` 未満でない限り、その幅を下限とする。
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
            None => return widths, // 全列がすでに下限。これ以上縮められない
        }
    }
}

/// セル内で素通しされた `<br>` / `<br/>` / `<br />`（インライン HTML）を境界に、
/// Span 列を複数行（セル内改行）へ分割する。タグを含まなければ 1 行のまま返す。
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
            lines.last_mut().expect("最低 1 行は必ず存在する").push(span);
        }
    }
    lines
}

/// Span 列の表示幅（全角文字対応）。
fn spans_width(spans: &[Span]) -> usize {
    spans
        .iter()
        .map(|s| unicode_width::UnicodeWidthStr::width(s.content.as_ref()))
        .sum()
}

/// テーブルの罫線行（`┌───┬───┐` 等）を 1 行組み立てる。
fn border_line(widths: &[usize], left: char, mid: char, right: char, style: Style) -> Line<'static> {
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

// ============================================================================
// テスト
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::Color;

    fn render(md: &str) -> Vec<Line<'static>> {
        render_markdown(md, &AppTheme::dark()).0
    }

    fn text_of(line: &Line) -> String {
        line.spans.iter().map(|s| s.content.as_ref()).collect()
    }

    /// 見出しは太字 + 色付き、レベルが深いほど暗い同系色になる。
    #[test]
    fn heading_is_bold_and_colored() {
        let lines = render("# Title\n\n## Sub\n\n### Deep");
        // 見出し行のみ抽出（段落間の空行を除く）
        let heads: Vec<&Line> = lines.iter().filter(|l| !l.spans.is_empty()).collect();
        assert_eq!(heads.len(), 3);
        assert_eq!(text_of(heads[0]), "Title");
        assert!(heads[0].spans[0]
            .style
            .add_modifier
            .contains(Modifier::BOLD));
        let c1 = heads[0].spans[0].style.fg;
        let c2 = heads[1].spans[0].style.fg;
        let c3 = heads[2].spans[0].style.fg;
        assert_eq!(c1, Some(Color::Rgb(115, 253, 253))); // H1 = 最も明るいシアン
        assert_eq!(c2, Some(Color::Rgb(100, 220, 220))); // H2 = 基準色
        assert_eq!(c3, Some(Color::Rgb(80, 176, 176))); // H3 = さらに暗い
        // 階層が深いほど暗くなる（同系色のまま明るさだけ下がる）
        let lum = |c: Option<Color>| match c {
            Some(Color::Rgb(r, g, b)) => u32::from(r) + u32::from(g) + u32::from(b),
            _ => 0,
        };
        assert!(lum(c1) > lum(c2), "H1 は H2 より明るい");
        assert!(lum(c2) > lum(c3), "H2 は H3 より明るい");
    }

    /// 強調・斜体・取り消し線が Modifier として反映される。
    #[test]
    fn inline_modifiers() {
        let lines = render("**bold** and *italic* and ~~strike~~");
        let joined: String = lines[0].spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(joined, "bold and italic and strike");
        assert!(lines[0].spans[0]
            .style
            .add_modifier
            .contains(Modifier::BOLD));
        assert!(lines[0].spans[2]
            .style
            .add_modifier
            .contains(Modifier::ITALIC));
        assert!(lines[0].spans[4]
            .style
            .add_modifier
            .contains(Modifier::CROSSED_OUT));
    }

    /// インラインコードは ekphos と同じくバッククォートを外し、マゼンタ色＋ code_bg 背景で表示される。
    #[test]
    fn inline_code() {
        let lines = render("run `cargo build`");
        let joined: String = lines[0].spans.iter().map(|s| s.content.as_ref()).collect();
        // バッククォートは表示しない（ekphos と同じ）
        assert_eq!(joined, "run cargo build");
        let code = &lines[0].spans[1];
        assert_eq!(code.content.as_ref(), "cargo build");
        assert_eq!(code.style.fg, Some(Color::Rgb(220, 120, 220))); // inline_code = マゼンタ
        assert_eq!(code.style.bg, Some(Color::Rgb(28, 28, 38))); // code_bg
    }

    /// コードブロックは muted + 背景色付きで、左に `│ ` ガター付きで 1 行ずつ出力される。
    /// 言語指定があれば冒頭にラベル行が出る。
    #[test]
    fn code_block() {
        let md = "```rust\nfn main() {}\nlet x = 1;\n```\n\nafter";
        let lines = render(md);
        // 冒頭に言語ラベル行
        assert!(
            lines.iter().any(|l| text_of(l) == "│ rust"),
            "言語ラベル行が無い"
        );
        let idx = lines
            .iter()
            .position(|l| text_of(l).contains("fn main"))
            .unwrap();
        let code_line = &lines[idx];
        assert_eq!(text_of(code_line), "│ fn main() {}");
        assert_eq!(code_line.spans[0].style.bg, Some(Color::Rgb(28, 28, 38))); // ガターも code_bg
        assert!(lines.iter().any(|l| text_of(l) == "│ let x = 1;"));
        // コードブロック後に本文が続く（ガター無し）
        assert!(lines.iter().any(|l| text_of(l) == "after"));
    }

    /// 箇条書きと番号付きリストのマーカー。
    #[test]
    fn list_markers() {
        let md = "- apple\n- banana\n\n1. first\n2. second";
        let lines = render(md);
        assert!(lines.iter().any(|l| text_of(l) == "• apple"));
        assert!(lines.iter().any(|l| text_of(l) == "• banana"));
        assert!(lines.iter().any(|l| text_of(l) == "1. first"));
        assert!(lines.iter().any(|l| text_of(l) == "2. second"));
    }

    /// タスクリストのチェックボックス。
    #[test]
    fn task_list() {
        let md = "- [x] done\n- [ ] todo";
        let lines = render(md);
        assert!(lines.iter().any(|l| text_of(l) == "• [x] done"));
        assert!(lines.iter().any(|l| text_of(l) == "• [ ] todo"));
    }

    /// 引用は muted + `│` プレフィックス。
    #[test]
    fn block_quote() {
        let md = "> quote line\n>\n> second";
        let lines = render(md);
        let quote = lines
            .iter()
            .find(|l| text_of(l).contains("quote line"))
            .unwrap();
        let joined: String = quote.spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(
            joined.starts_with("│ "),
            "引用プレフィックス付き: {:?}",
            joined
        );
        assert_eq!(quote.spans[0].style.fg, Some(Color::Rgb(100, 100, 120))); // muted
    }

    /// テーブルは罫線（┌┬┐ ├┼┤ └┴┘）で囲んで表示され、ヘッダーは太字。
    #[test]
    fn table() {
        let md = "| a | b |\n|---| --- |\n| 1 | 2 |";
        let lines = render(md);
        let texts: Vec<String> = lines.iter().map(text_of).collect();

        // 上辺・ヘッダー区切り・下辺の罫線が揃う
        assert!(
            texts
                .iter()
                .any(|t| t.starts_with('┌') && t.contains('┬') && t.ends_with('┐')),
            "上辺の罫線が無い: {:?}",
            texts
        );
        assert!(
            texts
                .iter()
                .any(|t| t.starts_with('├') && t.contains('┼') && t.ends_with('┤')),
            "ヘッダー区切りの罫線が無い: {:?}",
            texts
        );
        assert!(
            texts
                .iter()
                .any(|t| t.starts_with('└') && t.contains('┴') && t.ends_with('┘')),
            "下辺の罫線が無い: {:?}",
            texts
        );

        // ヘッダー行・データ行は `│` で囲まれる
        let head = lines.iter().find(|l| text_of(l).contains('a')).unwrap();
        let joined = text_of(head);
        assert!(
            joined.starts_with('│') && joined.ends_with('│'),
            "ヘッダー行が │ で囲まれていない: {:?}",
            joined
        );
        assert!(joined.contains('a') && joined.contains('b'));
        // ヘッダーセルは太字
        let a_span = head
            .spans
            .iter()
            .find(|s| s.content.as_ref() == "a")
            .unwrap();
        assert!(a_span.style.add_modifier.contains(Modifier::BOLD));
        assert!(texts.iter().any(|t| t.contains('1') && t.contains('2')));
    }

    /// cap_column_widths: 画面幅を超える場合は最も広い列から最小幅まで縮めること。
    #[test]
    fn cap_column_widths_shrinks_widest_column_first() {
        // col1(5)・col2(3) は自然幅がすでに最小幅の床（8）未満なのでそれ以上縮まない。
        // col0(20) だけが床（8）まで縮む。
        let widths = cap_column_widths(&[20, 5, 3], 15);
        assert_eq!(widths[1], 5, "自然幅内に収まる列はそのまま");
        assert_eq!(widths[2], 3, "自然幅内に収まる列はそのまま");
        assert!(widths[0] < 20, "最も広い列が縮む");
        assert_eq!(
            widths[0], TABLE_COLUMN_MIN_WIDTH,
            "他列がすでに床なので、最も広い列は最小幅まで縮む"
        );

        // 全列とも予算内に収まる場合は縮めない。
        let unchanged = cap_column_widths(&[20, 5, 3], 100);
        assert_eq!(unchanged, vec![20, 5, 3]);
    }

    /// CJK（全角）文字を含むセルの表示幅が正しく計算され、罫線の幅に反映されること。
    #[test]
    fn table_column_width_accounts_for_cjk_width() {
        let md = "| a | 名前 |\n|---|---|\n| 1 | 山田太郎 |";
        let lines = render(md);
        let border = lines
            .iter()
            .find(|l| text_of(l).starts_with('┌'))
            .map(text_of)
            .unwrap();
        // "山田太郎" は全角 4 文字（表示幅 8）。パディング込みで ─ が 10 本のはず。
        let seg = border
            .split('┬')
            .nth(1)
            .unwrap()
            .trim_end_matches('┐');
        assert_eq!(
            seg.chars().count(),
            10,
            "CJK 列の罫線幅が表示幅を反映していない: {:?}",
            border
        );
    }

    /// YAML frontmatter はメタデータとして muted 表示される。
    #[test]
    fn yaml_frontmatter() {
        let md = "---\nname: my-block\ntag: test\n---\n\n# Body";
        let lines = render(md);
        assert!(lines.iter().any(|l| text_of(l).contains("metadata")));
        assert!(lines.iter().any(|l| text_of(l).contains("name: my-block")));
        // 本文も表示される
        assert!(lines.iter().any(|l| text_of(l) == "Body"));
    }

    /// 水平線がセパレータ行になる。
    #[test]
    fn horizontal_rule() {
        let lines = render("before\n\n---\n\nafter");
        assert!(lines.iter().any(|l| l
            .spans
            .first()
            .map(|s| s.content.as_ref().starts_with('─'))
            .unwrap_or(false)));
    }

    /// リンクは `text` がリンク色 + 下線、`(url)` がミュートカラーで併記される。
    #[test]
    fn link() {
        let lines = render("see [docs](https://example.com)");
        let joined: String = lines[0].spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(joined, "see docs (https://example.com)");
        let text_span = lines[0]
            .spans
            .iter()
            .find(|s| s.content.as_ref() == "docs")
            .unwrap();
        assert_eq!(text_span.style.fg, Some(Color::Rgb(80, 160, 255))); // link（青）
        assert!(text_span.style.add_modifier.contains(Modifier::UNDERLINED));
        let url_span = lines[0]
            .spans
            .iter()
            .find(|s| s.content.as_ref() == " (https://example.com)")
            .unwrap();
        assert_eq!(url_span.style.fg, Some(Color::Rgb(100, 100, 120))); // muted
    }

    /// image は alt テキストではなく `[image: <path>]` をミュートカラーで表示する
    /// （実画像表示は ratatui-image 前提のため対象外）。
    #[test]
    fn image_shows_path_placeholder() {
        let lines = render("![a screenshot](./assets/shot.png)");
        let joined = text_of(&lines[0]);
        assert_eq!(joined, "[image: ./assets/shot.png]");
        assert_eq!(
            lines[0].spans[0].style.fg,
            Some(Color::Rgb(100, 100, 120)) // muted
        );
    }

    /// 空文字列・空行のみでもクラッシュしない。
    #[test]
    fn empty_input() {
        assert!(render("").is_empty());
        assert!(render("\n\n\n").is_empty());
    }

    /// 見出し一覧（HeadingEntry）が line_idx/level/title を正しく返す。
    #[test]
    fn headings_are_extracted() {
        let (lines, headings) = render_markdown("# Title\n\nbody\n\n## Sub", &AppTheme::dark());
        assert_eq!(headings.len(), 2);
        assert_eq!(headings[0].level, 1);
        assert_eq!(headings[0].title, "Title");
        assert_eq!(text_of(&lines[headings[0].line_idx]), "Title");
        assert_eq!(headings[1].level, 2);
        assert_eq!(headings[1].title, "Sub");
        assert_eq!(text_of(&lines[headings[1].line_idx]), "Sub");
    }

    /// 言語指定ありのフェンスコードブロックは単色でなく複数の Span に分かれる。
    #[test]
    fn fenced_code_block_gets_syntax_highlighted() {
        let md = "```rust\nfn main() { let x = 1; }\n```";
        let lines = render(md);
        let code_line = lines
            .iter()
            .find(|l| text_of(l).contains("fn main"))
            .expect("コード行が見つからない");
        assert!(
            code_line.spans.len() > 1,
            "言語指定ありのコードは複数 Span に分かれるべき: {:?}",
            code_line.spans
        );

        // 言語指定なしは単色フォールバック（ガター + 内容の 2 Span）。ラベル行も出ない。
        let md_plain = "```\nplain text line\n```";
        let lines_plain = render(md_plain);
        assert!(
            lines_plain.iter().all(|l| text_of(l) != "rust"),
            "言語未指定ならラベル行を出さない"
        );
        let plain_line = lines_plain
            .iter()
            .find(|l| text_of(l).contains("plain text line"))
            .expect("プレーンコード行が見つからない");
        assert_eq!(plain_line.spans.len(), 2, "ガター + 内容の 2 Span のはず");
        assert_eq!(text_of(plain_line), "│ plain text line");
    }
}
