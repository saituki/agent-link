//! 検索画面（セッション履歴検索。仕様書 `docs/spec-session-search.md`）。
//!
//! **UI のみ** を担当する。検索ロジック（条件解析・DB フィルタ・jsonl スキャン）
//! は `sai_link_engine::search` に一本化されており、このモジュールは
//! フォームの描画と入力状態の保持のみを行う。
//!
//! ## 画面構成
//!
//! ```text
//! ┌─ Search (Shift+F / Esc で戻る) ─────────────────────────────┐
//! │    Project: sai-link （表示のみ・入力不可）                │
//! │  > Round: 1                                               │
//! │    round: 2                                               │
//! │    ID: 10-20                                              │
//! │    block:                                                 │
//! │    Date: 2026/08/10-2026/08/20                            │
//! │    type: user                                             │
//! │    word: llm, sai-link-tui, 移植                          │
//! │  ─────────────────────────────────────────────────────────  │
//! │  sai-link  | ID5 | user    | 3 | <マッチした content の一部> │
//! │  sai-link  | ID5 | agent    | 4 | ...                       │
//! │  ...                                                        │
//! │  12 件（読み飛ばし 0 件）  Enter=検索  ↑↓=移動            │
//! └───────────────────────────────────────────────────────────┘
//! ```

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Widget},
};

use sai_link_engine::search::{RawSearchForm, SearchHit};

use sai_link_widgets::theme::AppTheme;

// ============================================================================
// 状態
// ============================================================================

/// 編集可能なフォーム項目の数（Round / round / ID / block / Date / status / type / word）。
///
/// `Project` 項目（L2）は自プロジェクト名の**表示のみ**（入力不可）のため
/// 編集対象には含まれない。フォーム描画は Project 行 → 編集可能項目の順。
pub const FORM_ITEMS: usize = 8;

/// フォーム項目のラベル（先頭の `Project` は表示専用・残りは `form` と 1:1 対応）。
pub const FORM_LABELS: [&str; FORM_ITEMS + 1] = [
    "Project", "Round", "round", "ID", "block", "Date", "status", "type", "word",
];

/// 編集可能なフォーム項目の先頭インデックス（Project 行をスキップする）。
const FORM_EDITABLE_START: usize = 1;

/// 検索画面の UI 状態（ロジック非所有。検索実行は `AppState` が engine に委譲）。
#[derive(Debug, Default)]
pub struct SearchState {
    /// 各フォーム項目の生入力文字列（engine と共有する構造体）
    pub form: RawSearchForm,
    /// 自プロジェクト名（Project 行に表示するのみ・入力不可・L2）
    pub project_name: String,
    /// フォーカス中のフォーム項目インデックス（0..FORM_ITEMS）
    pub focus: usize,
    /// テキスト編集中か
    pub editing: bool,
    /// 編集中バッファ（確定時に `form` の該当項目へ反映）
    pub edit_buf: String,
    /// 検索結果（`run_search()` の戻り値。表示専用）
    pub hits: Vec<SearchHit>,
    /// 結果リストのカーソル位置
    pub selected: usize,
    /// 結果リストのスクロール位置
    pub scroll: usize,
    /// 検索条件の書式エラー表示（None = 正常）
    pub error: Option<String>,
    /// 最終検索で読み飛ばした実行数（警告表示用）
    pub skipped: usize,
    /// 1 度でも検索を実行したか（結果 0 件と「未検索」の区別用）
    pub searched: bool,
}

impl SearchState {
    /// フォーカス中のフォーム項目の値を取り出す。
    fn form_value(&self, idx: usize) -> &str {
        match idx {
            0 => &self.form.round_gr,
            1 => &self.form.round,
            2 => &self.form.id,
            3 => &self.form.block,
            4 => &self.form.date,
            5 => &self.form.status,
            6 => &self.form.type_,
            7 => &self.form.word,
            _ => "",
        }
    }

    /// フォーカス中のフォーム項目の値を更新する。
    fn set_form_value(&mut self, idx: usize, v: String) {
        match idx {
            0 => self.form.round_gr = v,
            1 => self.form.round = v,
            2 => self.form.id = v,
            3 => self.form.block = v,
            4 => self.form.date = v,
            5 => self.form.status = v,
            6 => self.form.type_ = v,
            7 => self.form.word = v,
            _ => {}
        }
    }

    /// 編集を開始する（`edit_buf` に現在値をコピー）。
    pub fn begin_edit(&mut self) {
        self.edit_buf = self.form_value(self.focus).to_string();
        self.editing = true;
    }

    /// 編集を確定してフォームへ反映する。
    pub fn commit_edit(&mut self) {
        if self.editing {
            let v = std::mem::take(&mut self.edit_buf);
            self.set_form_value(self.focus, v);
            self.editing = false;
        }
    }

    /// 編集を破棄する。
    pub fn cancel_edit(&mut self) {
        self.edit_buf.clear();
        self.editing = false;
    }
}

// ============================================================================
// 描画
// ============================================================================

/// 検索画面本体（メイン画面領域へ描画するウィジェット）。
pub struct SearchView<'a> {
    pub state: &'a mut SearchState,
    pub theme: &'a AppTheme,
}

impl Widget for SearchView<'_> {
    fn render(mut self, area: Rect, buf: &mut ratatui::buffer::Buffer) {
        // 小さすぎる領域ではメッセージのみ（レイアウト制約のパニック防止）
        if area.width < 30 || area.height < 8 {
            Paragraph::new("Search: area too small")
                .block(Block::default().borders(Borders::ALL).title("Search"))
                .render(area, buf);
            return;
        }

        let st = &mut self.state;
        let theme = self.theme;

        let block = Block::default()
            .borders(Borders::ALL)
            .title(" Search (Shift+F / Esc で戻る) ")
            .border_style(Style::new().fg(theme.border))
            .style(Style::new().bg(theme.main_bg));
        let inner = block.inner(area);
        block.render(area, buf);

        // フォーム 8 行（Project 表示行 + 編集可能 7 項目）+ 区切り 1 行 + フッタ 1 行を除いた領域を結果に使う
        let form_rows = FORM_ITEMS as u16 + 1; // Project 行 + 編集可能 7 項目
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(form_rows + 1), // フォーム + 区切り
                Constraint::Fill(1),               // 結果リスト
                Constraint::Length(1),             // フッタ
            ])
            .split(inner);

        // ── フォーム ──
        for (display_idx, label) in FORM_LABELS.iter().enumerate() {
            let y = chunks[0].y + display_idx as u16;
            // 先頭行（Project）は自プロジェクト名の表示のみ・入力不可（L2）
            if display_idx == 0 {
                let name = if st.project_name.is_empty() {
                    "（自プロジェクト）".to_string()
                } else {
                    st.project_name.clone()
                };
                let line = Line::from(vec![
                    Span::styled(
                        format!("  {label}: "),
                        Style::new().fg(theme.muted).add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(name, Style::new().fg(theme.info)),
                ]);
                buf.set_line(inner.x, y, &line, inner.width);
                continue;
            }
            let idx = display_idx - FORM_EDITABLE_START; // 編集可能項目のインデックス
            let focused = idx == st.focus;
            // 編集中は edit_buf、それ以外はフォーム値を表示
            let value = if focused && st.editing {
                st.edit_buf.clone()
            } else {
                st.form_value(idx).to_string()
            };
            let mut spans = Vec::with_capacity(4);
            let editing_here = focused && st.editing;
            let prefix = if editing_here { "> " } else { "  " };
            spans.push(Span::styled(
                format!("{prefix}{label}: "),
                if focused {
                    Style::new().fg(theme.info).add_modifier(Modifier::BOLD)
                } else {
                    Style::new().fg(theme.muted)
                },
            ));
            let empty = value.is_empty();
            let value_style = if focused {
                Style::new().fg(theme.fg)
            } else if empty {
                Style::new().fg(theme.muted)
            } else {
                Style::new().fg(theme.fg)
            };
            // 編集中は末尾にカーソル（▌）を表示
            let (v, cursor) = if editing_here {
                (value, "▌")
            } else {
                (value, "")
            };
            spans.push(Span::styled(v, value_style));
            if !cursor.is_empty() {
                spans.push(Span::styled(cursor, Style::new().fg(theme.fg)));
            }
            if empty && !editing_here {
                spans.push(Span::styled("(任意)", Style::new().fg(theme.muted)));
            }
            let line = Line::from(spans);
            buf.set_line(inner.x, y, &line, inner.width);
        }

        // ── 区切り線 ---- ──
        let sep = Line::from(Span::styled(
            "-".repeat(chunks[0].width as usize),
            Style::new().fg(theme.border),
        ));
        buf.set_line(inner.x, chunks[0].y + form_rows, &sep, inner.width);

        // ── エラー表示（結果領域の先頭行） ──
        if let Some(err) = &st.error {
            let line = Line::from(Span::styled(err.clone(), Style::new().fg(theme.error)));
            buf.set_line(inner.x, chunks[1].y, &line, inner.width);
        } else if st.searched && st.hits.is_empty() {
            let line = Line::from(Span::styled("検索結果: 0 件", Style::new().fg(theme.muted)));
            buf.set_line(inner.x, chunks[1].y, &line, inner.width);
        } else if st.hits.is_empty() {
            let line = Line::from(Span::styled(
                "条件を入力して Enter で検索してください",
                Style::new().fg(theme.muted),
            ));
            buf.set_line(inner.x, chunks[1].y, &line, inner.width);
        } else {
            // カーソルを可視範囲に収める（追徳スクロール）
            let visible = chunks[1].height as usize;
            if visible > 0 {
                if st.selected < st.scroll {
                    st.scroll = st.selected;
                } else if st.selected >= st.scroll + visible {
                    st.scroll = st.selected + 1 - visible;
                }
            }
            render_hits(buf, chunks[1], st, theme);
        }

        // ── フッタ ──
        let mut footer = if st.hits.is_empty() {
            String::new()
        } else {
            format!("{} 件", st.hits.len())
        };
        if st.skipped > 0 {
            footer.push_str(&format!("（セッション不在で読み飛ばし {} 件）", st.skipped));
        }
        footer.push_str("  Enter=検索  ↑↓=項目/結果  Esc=戻る");
        let line = Line::from(Span::styled(footer, Style::new().fg(theme.muted)));
        buf.set_line(inner.x, chunks[2].y, &line, inner.width);
    }
}

/// 結果リストの描画（スクロール + カーソル + 検索語ハイライト）。
fn render_hits(buf: &mut ratatui::buffer::Buffer, area: Rect, st: &SearchState, theme: &AppTheme) {
    let visible = area.height as usize;
    let first = st.scroll.min(st.hits.len().saturating_sub(1));
    for (y, (i, hit)) in (area.y..).zip(st.hits.iter().enumerate().skip(first).take(visible)) {
        if y >= area.y + area.height {
            break;
        }
        let is_selected = i == st.selected;
        let prefix_style = if is_selected {
            Style::new().fg(theme.warning).add_modifier(Modifier::BOLD)
        } else {
            Style::new().fg(theme.muted)
        };
        let mut spans = Vec::with_capacity(8);
        let marker = if is_selected { " > " } else { "   " };
        spans.push(Span::styled(marker, prefix_style));
        // 出自プロジェクト（先頭カラム・L2）
        spans.push(Span::styled(
            format!("{:8}", truncate_chars(&hit.project, 8)),
            Style::new().fg(theme.info),
        ));
        spans.push(Span::styled(" | ", Style::new().fg(theme.border)));
        spans.push(Span::styled(
            format!("ID{}", hit.block_id),
            Style::new().fg(theme.info),
        ));
        spans.push(Span::styled(" | ", Style::new().fg(theme.border)));
        spans.push(Span::styled(
            format!("{:10}", hit.type_.as_str()),
            Style::new().fg(theme.success),
        ));
        spans.push(Span::styled(" | ", Style::new().fg(theme.border)));
        spans.push(Span::styled(
            format!("{:>4}", hit.turn),
            Style::new().fg(theme.warning),
        ));
        spans.push(Span::styled(" | ", Style::new().fg(theme.border)));
        // content 抜粋（検索語をハイライト）
        push_snippet_spans(
            &mut spans,
            &hit.snippet,
            &hit.matched_words,
            theme,
            is_selected,
        );
        buf.set_line(area.x, y, &Line::from(spans), area.width);
    }
}

/// snippet を検索語ハイライト付きの Span 列に展開する。
fn push_snippet_spans(
    spans: &mut Vec<Span<'static>>,
    snippet: &str,
    words: &[String],
    theme: &AppTheme,
    selected: bool,
) {
    let base_style = if selected {
        Style::new().fg(theme.fg)
    } else {
        Style::new().fg(theme.fg).add_modifier(Modifier::DIM)
    };
    let hi_style = Style::new()
        .fg(Color::Black)
        .bg(theme.warning)
        .add_modifier(Modifier::BOLD);
    if words.is_empty() || snippet.is_empty() {
        spans.push(Span::styled(snippet.to_string(), base_style));
        return;
    }
    // 大文字小文字を無視してハイライト（engine が小文字化した語を渡す）
    let lower = snippet.to_lowercase();
    let chars: Vec<char> = snippet.chars().collect();
    let lower_chars: Vec<char> = lower.chars().collect();
    let mut mark = vec![false; chars.len()];
    for w in words {
        if w.is_empty() {
            continue;
        }
        let wc: Vec<char> = w.chars().collect();
        if wc.is_empty() || wc.len() > lower_chars.len() {
            continue;
        }
        let mut i = 0;
        while i + wc.len() <= lower_chars.len() {
            if lower_chars[i..i + wc.len()] == wc[..] {
                for m in mark.iter_mut().take(i + wc.len()).skip(i) {
                    *m = true;
                }
                i += wc.len();
            } else {
                i += 1;
            }
        }
    }
    let mut run_start = 0usize;
    let mut cur = mark[0];
    for i in 1..=chars.len() {
        let m = mark.get(i).copied().unwrap_or(!cur);
        if m != cur || i == chars.len() {
            let s: String = chars[run_start..i].iter().collect();
            spans.push(Span::styled(s, if cur { hi_style } else { base_style }));
            run_start = i;
            cur = m;
        }
    }
}

/// 文字列を先頭 `max` 文字に切り詰める（カラム幅調整用）。
fn truncate_chars(s: &str, max: usize) -> String {
    s.chars().take(max).collect()
}

// ============================================================================
// テスト
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::buffer::Buffer;
    use sai_link_engine::search::{SearchHit, SearchHitType};

    /// フォーム・結果リスト・ハイライトを含む描画がパニックしないこと。
    #[test]
    fn search_view_renders_without_panic() {
        let mut st = SearchState::default();
        st.form = RawSearchForm {
            round_gr: "1".into(),
            word: "llm".into(),
            ..Default::default()
        };
        st.hits = vec![
            SearchHit {
                project: "proj".into(),
                block_id: 5,
                exec_id: 1,
                type_: SearchHitType::User,
                turn: 4,
                snippet: "llm を含む user 発言の抜粋です".into(),
                matched_words: vec!["llm".into()],
                session_path: std::path::PathBuf::from("/tmp/a.jsonl"),
            },
            SearchHit {
                project: "proj".into(),
                block_id: 5,
                exec_id: 1,
                type_: SearchHitType::Agent,
                turn: 7,
                snippet: "回答テキスト".into(),
                matched_words: vec![],
                session_path: std::path::PathBuf::from("/tmp/a.jsonl"),
            },
        ];
        st.selected = 0;
        st.searched = true;

        let theme = AppTheme::dark();
        let area = Rect::new(0, 0, 80, 24);
        let mut buf = Buffer::empty(area);
        SearchView {
            state: &mut st,
            theme: &theme,
        }
        .render(area, &mut buf);

        // タイトルとフォームラベル・結果行が描画されていること
        let text: String = buf.content().iter().map(|c| c.symbol()).collect();
        assert!(text.contains("Project:"));
        assert!(text.contains("Round:"));
        assert!(text.contains("word:"));
        assert!(text.contains("ID5"));
        assert!(text.contains("user"));
    }

    /// 極小領域でもパニックしないこと。
    #[test]
    fn search_view_renders_tiny_area() {
        let mut st = SearchState::default();
        let theme = AppTheme::dark();
        let area = Rect::new(0, 0, 20, 5);
        let mut buf = Buffer::empty(area);
        SearchView {
            state: &mut st,
            theme: &theme,
        }
        .render(area, &mut buf);
    }
}
