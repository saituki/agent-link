//! History 画面（ratatui ネイティブウィジェット描画）。
//!
//! **Token Usage by Category・円グラフ・Timeline の 3 セクションのみ**を
//! 表示する（セッション出力本文は表示しない）:
//!
//! ```text
//! ┌ Token Usage by Category ──────────────┬┌ Tokens by Type（円グラフ） ┐┐
//! │ ┌ user ┐┌ agent ┐┌ toolCall ┐ ...     ││■ user 25.0%              ││
//! │ │ 100  ││ 200  ││ 50       │        ││■ agent 50.0%             ││
//! │ └──────┘└───────┘└──────────┘        │└ ...                      ┘│
//! ├ Timeline（Time / Type / name / token）─┴─────────────────────────┤
//! │ 08-29 01:00:00 | user | こんにちは | 5                            │
//! └──────────────────────────────────────────────────────────────────┘
//! ```
//!
//! - **近似値のみ使用する**: token はセッション jsonl の文字数換算（近似）であり、
//!   provider の実際の usage（`execution_usage`）は使わない（Project 画面のみ正確値）
//! - 種別: `user` / `agent` / `toolCall` / `toolResult` / `thinking`
//! - 「Token Usage by Category」は **Project 画面の先頭 stat カード
//!   （`BLOCK` / `TOKENS` ... 枠 + 数値）と同型**のボックスで
//!   type ごとのトークン合計値を表示する
//! - Timeline は Project 画面の Recent テーブルと同じ ratatui `Table` +
//!   内部スクロール方式。下段まで余白を使わず、Timeline が画面下まで広がる

use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Paragraph, Row, Table, Widget},
};

use crate::pie::{MiniPie, PieSliceData};
use crate::theme::AppTheme;
use sai_link_engine::session::{HistoryEvent, HistoryTypeTokens};

/// 「Token Usage by Category」セクションの固定高。
///
/// 枠 2 + カード 3 枚積み（各高さ 4・間隔 1 → 14）+ 上下余白 2。
pub(crate) const USAGE_SECTION_H: u16 = 18;

/// Timeline テーブルの既定可視行数（描画時に実測値で上書きされる）。
pub(crate) const DEFAULT_TIMELINE_VISIBLE: usize = 8;

/// History 画面の種別（表示順）。
pub(crate) const HISTORY_TYPES: [&str; 5] = ["user", "agent", "toolCall", "toolResult", "thinking"];

// ============================================================================
// 状態
// ============================================================================

/// History 画面の表示データ（`app/data.rs::load_session_output_to_main()` が構築）。
#[derive(Debug, Default)]
pub struct HistoryViewData {
    /// タイトル（例: `Session #3`）
    pub title: String,
    /// サブタイトル（例: `block 名 | started: ... | status: ...`）
    pub subtitle: String,
    /// 種別ごとの近似トークン合計（文字数換算）
    pub type_tokens: HistoryTypeTokens,
    /// Timeline イベント（時刻順）
    pub events: Vec<HistoryEvent>,
    /// Timeline テーブルの内部スクロール位置（先頭からの行数）
    pub timeline_offset: usize,
    /// Timeline テーブルの実測可視行数（描画時に更新）
    pub(crate) timeline_visible: usize,
}

impl HistoryViewData {
    /// 種別の近似トークン合計を取得する。
    pub fn tokens_of(&self, kind: &str) -> u64 {
        match kind {
            "user" => self.type_tokens.user,
            "agent" => self.type_tokens.agent,
            "toolCall" => self.type_tokens.tool_call,
            "toolResult" => self.type_tokens.tool_result,
            "thinking" => self.type_tokens.thinking,
            _ => 0,
        }
    }

    /// `phone run --view history` 用の JSON 表現を返す（実装計画 12 §3.2）。
    ///
    /// 描画用の内部スクロール状態（`timeline_offset` / `timeline_visible`）は
    /// 出力しない。項目名は仕様のとおり（`toolCall` / `toolResult` / `time` / `type`）。
    pub fn phone_json(&self) -> PhoneHistoryJson<'_> {
        PhoneHistoryJson {
            title: &self.title,
            subtitle: &self.subtitle,
            type_tokens: PhoneTypeTokens {
                user: self.type_tokens.user,
                agent: self.type_tokens.agent,
                tool_call: self.type_tokens.tool_call,
                tool_result: self.type_tokens.tool_result,
                thinking: self.type_tokens.thinking,
            },
            events: self
                .events
                .iter()
                .map(|e| PhoneEvent {
                    timestamp: &e.timestamp,
                    kind: &e.kind,
                    name: &e.name,
                    tokens: e.tokens,
                })
                .collect(),
        }
    }

    /// 種別ごとの割合（円グラフ用スライス）。
    fn pie_slices(&self) -> Vec<PieSliceData> {
        const COLORS: [ratatui::style::Color; 5] = [
            ratatui::style::Color::Blue,
            ratatui::style::Color::Green,
            ratatui::style::Color::Yellow,
            ratatui::style::Color::Magenta,
            ratatui::style::Color::Cyan,
        ];
        let total: u64 = HISTORY_TYPES.iter().map(|t| self.tokens_of(t)).sum();
        HISTORY_TYPES
            .iter()
            .enumerate()
            .filter(|(_, t)| self.tokens_of(t) > 0)
            .map(|(i, t)| PieSliceData {
                label: t.to_string(),
                value: self.tokens_of(t) as f64,
                color: COLORS[i],
            })
            .chain(
                // 全種別 0 の場合も凡例が消えないよう便宜的な 1 スライスを返す
                (total == 0).then(|| PieSliceData {
                    label: "n/a".to_string(),
                    value: 1.0,
                    color: ratatui::style::Color::DarkGray,
                }),
            )
            .collect()
    }
}

/// Timeline テーブルの内部スクロール位置を変更する（`j/k`・`↑/↓`）。
///
/// 可視行数は描画時に実測され `timeline_visible` に保持される（未描画時は既定値）。
pub fn timeline_scroll(d: &mut HistoryViewData, delta: i32) {
    let visible = d.timeline_visible.max(1);
    let max = d.events.len().saturating_sub(visible);
    d.timeline_offset =
        ((d.timeline_offset as i64 + i64::from(delta)).clamp(0, max as i64)) as usize;
}

/// `phone run --view history` の JSON 形（実装計画 12 §3.2）。
#[derive(serde::Serialize)]
pub struct PhoneHistoryJson<'a> {
    /// タイトル（例: `Session #25`）
    pub title: &'a str,
    /// サブタイトル（`block 名 | started: … | status: …`）
    pub subtitle: &'a str,
    /// 種別ごとの近似トークン合計
    pub type_tokens: PhoneTypeTokens,
    /// Timeline イベント（時刻順）
    pub events: Vec<PhoneEvent<'a>>,
}

/// `type_tokens`（項目名は仕様どおり camelCase の `toolCall` / `toolResult`）。
#[derive(serde::Serialize)]
pub struct PhoneTypeTokens {
    /// user message の text
    pub user: u64,
    /// assistant message の text
    pub agent: u64,
    /// toolCall（名前 + 引数）
    #[serde(rename = "toolCall")]
    pub tool_call: u64,
    /// toolResult の text
    #[serde(rename = "toolResult")]
    pub tool_result: u64,
    /// thinking
    pub thinking: u64,
}

/// Timeline イベント 1 件（`time` / `type` リネーム付き）。
#[derive(serde::Serialize)]
pub struct PhoneEvent<'a> {
    /// ISO 8601 タイムスタンプ
    #[serde(rename = "time")]
    pub timestamp: &'a str,
    /// 種別: `user` / `agent` / `toolCall` / `toolResult` / `thinking`
    #[serde(rename = "type")]
    pub kind: &'a str,
    /// 名前（ツール名・本文冒頭など）
    pub name: &'a str,
    /// 近似トークン数
    pub tokens: u64,
}

// ============================================================================
// ビルダ純関数（DB → 表示データ変換）
// ============================================================================

/// 実行 1 件分の History 画面データを DB から構築する（ビルダ純関数）。
///
/// sai-link-tui `app/data.rs::load_session_output_to_main()` の DB→データ変換部分を
/// 分離したもの。`sai_control_tui` 等の第 2 クライアントからも同一ビルダを使い、
/// 同一画面を表示できるようにする。
///
/// **近似値のみ使用する**: token はセッション jsonl の文字数換算（近似）であり、
/// provider の実際の usage（`execution_usage`）は使わない。
///
/// - `store`: 対象プロジェクトの `SaiStore`（read-only でも可）
/// - `exec_id`: 対象実行 ID
///
/// 実行レコードが見つからない場合は `None`（呼び出し側でメッセージ表示する）。
/// セッション（`session_dir` 未記録）が無い場合は type_tokens / events が空の
/// データを返す（画面には「時系列イベントはありません」と表示される）。
pub fn build(store: &sai_link_engine::db::SaiStore, exec_id: i64) -> Option<HistoryViewData> {
    let exec = store.get_execution_by_id(exec_id).ok()?;
    let block_name = store
        .get_block_by_id(exec.block_id)
        .map(|b| b.display_name)
        .unwrap_or_else(|_| format!("block#{}", exec.block_id));
    let start_time = exec.started_at.as_deref().unwrap_or("unknown");

    // 近似統計（種別別トークン + Timeline イベント）を jsonl 走査 1 回で取得する。
    // DB の旧パスが残っていても outputs 側へフォールバックする。
    // runner=gl は `~/.sai-gemini/sessions/` の最新セッション（空でない最初の1件）。
    // runner=cc は `~/.claude/projects/` の最新セッション。
    let (type_tokens, events) = if exec.runner == "gl" {
        if let Some(st) = sai_link_engine::session::gemini_session_files(30)
            .into_iter()
            .find_map(|path| {
                sai_link_engine::session::extract_gemini_history_stats(&path)
                    .ok()
                    .filter(|st| !st.events.is_empty())
            })
        {
            (st.type_tokens, st.events)
        } else {
            Default::default()
        }
    } else if exec.runner == "cc" {
        if let Some(st) = sai_link_engine::session::cc_session_files(30)
            .into_iter()
            .find_map(|path| {
                sai_link_engine::session::extract_cc_history_stats(&path)
                    .ok()
                    .filter(|st| !st.events.is_empty())
            })
        {
            (st.type_tokens, st.events)
        } else {
            Default::default()
        }
    } else {
        sai_link_engine::session::resolve_session_dir(exec.session_dir.as_deref(), exec_id)
            .and_then(|dir| sai_link_engine::session::extract_history_stats(&dir).ok())
            .map(|s| (s.type_tokens, s.events))
            .unwrap_or_default()
    };

    Some(HistoryViewData {
        title: format!("Session #{}", exec_id),
        subtitle: format!(
            "{} | started: {} | status: {}",
            block_name, start_time, exec.status
        ),
        type_tokens,
        events,
        timeline_offset: 0,
        timeline_visible: DEFAULT_TIMELINE_VISIBLE,
    })
}

// ============================================================================
// 描画
// ============================================================================

/// History 画面本体（メイン画面領域へ描画する）。
pub fn render_history_view(
    area: Rect,
    buf: &mut Buffer,
    d: &mut HistoryViewData,
    theme: &AppTheme,
) {
    // 小さすぎる領域ではメッセージのみ（レイアウト制約のパニック防止）
    if area.width < 40 || area.height < 8 {
        Paragraph::new(format!("History: {} / (area too small)", d.title)).render(area, buf);
        return;
    }

    // ── タイトル行 + セパレータ（既存の md_view と同一の体裁） ──
    draw_header(area, buf, &d.title, &d.subtitle, theme);

    let body = Rect::new(
        area.x,
        area.y + 2,
        area.width,
        area.height.saturating_sub(2),
    );
    let guide_h = 1u16;
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(USAGE_SECTION_H), // Token Usage by Category + 円グラフ
            Constraint::Fill(1),                 // Timeline（画面下まで）
            Constraint::Length(guide_h),         // キーガイド
        ])
        .split(body);

    render_usage_section(chunks[0], buf, d, theme);
    render_timeline_table(chunks[1], buf, d, theme);

    // ── キーガイド ──
    let guide = "↑↓/jk: Timeline | Esc close";
    draw_header_line(
        Rect::new(body.x, chunks[2].y, body.width, 1),
        buf,
        guide,
        Style::default().fg(theme.muted),
    );
}

/// タイトル行 + セパレータを描く（`main_panel::render_markdown_view` と同一の体裁）。
fn draw_header(area: Rect, buf: &mut Buffer, title: &str, subtitle: &str, theme: &AppTheme) {
    draw_header_line(
        Rect::new(area.x + 1, area.y, area.width.saturating_sub(2), 1),
        buf,
        &format!(" {}: {}", title, subtitle),
        Style::default().fg(theme.info),
    );
    for x in area.x..area.x + area.width {
        buf.cell_mut((x, area.y + 1))
            .map(|c| c.set_char('─').set_style(Style::default().fg(theme.muted)));
    }
}

/// 1 行テキストを描く（クリップは set_line に任せる）。
fn draw_header_line(area: Rect, buf: &mut Buffer, text: &str, style: Style) {
    buf.set_line(
        area.x,
        area.y,
        &Line::from(Span::styled(text.to_string(), style)),
        area.width,
    );
}

/// 「Token Usage by Category」セクション: type ごとの合計値ボックス + 円グラフ。
///
/// ボックスは **Project 画面の先頭 stat カード（`render_cards`）と同一構造**:
/// 枠付きボックスに「種別（muted）+ トークン合計値（色付き太字）」を 2 行で表示する。
fn render_usage_section(area: Rect, buf: &mut Buffer, d: &HistoryViewData, theme: &AppTheme) {
    // セクション枠（タイトル付き）
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.border))
        .title(Span::styled(
            " Token Usage by Category ",
            Style::default().fg(theme.info).add_modifier(Modifier::BOLD),
        ));
    let inner = block.inner(area);
    block.render(area, buf);

    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(inner);

    // 左: 3 列 × 3 行のカード配置（Project の stat カードと同型・高さ 4）。
    // 1 列目: user / agent / input を縦に 3 枚配置する。
    let user = d.tokens_of("user");
    let agent = d.tokens_of("agent");
    let tool_call = d.tokens_of("toolCall");
    let tool_result = d.tokens_of("toolResult");
    let thinking = d.tokens_of("thinking");
    let input = user + tool_result;
    let output = agent + tool_call + thinking;
    let total = input + output;
    // 見た目順（行優先）: user toolResult thinking / agent toolCall output / input total (空)
    // 1 列目に user / agent / input が縦に 3 枚並ぶ。
    let ordered: [[(&str, u64, usize); 3]; 3] = [
        [
            ("user", user, 0),
            ("toolResult", tool_result, 3),
            ("thinking", thinking, 4),
        ],
        [
            ("agent", agent, 1),
            ("toolCall", tool_call, 2),
            ("output", output, 1),
        ],
        [("input", input, 0), ("total", total, 5), ("", 0, 0)],
    ];
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(14),
            Constraint::Fill(1),
        ])
        .split(cols[0]);
    let row_areas = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(4),
            Constraint::Length(4),
            Constraint::Length(4),
        ])
        .spacing(1)
        .split(rows[1]);
    for (row_area, row) in row_areas.iter().zip(ordered) {
        let boxes = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Ratio(1, 3); 3])
            .spacing(1)
            .split(*row_area);
        for (kind_area, (kind, value, ci)) in boxes.iter().zip(row) {
            if kind.is_empty() {
                continue;
            }
            let color = if kind == "total" {
                ratatui::style::Color::White
            } else {
                pie_color(ci)
            };
            // Project 画面 `render_cards` と同一構造（ラベル + 値の 2 行）
            let card = Paragraph::new(vec![
                Line::from(Span::styled(kind, Style::default().fg(theme.muted))),
                Line::from(Span::styled(
                    crate::fmt::format_num(value),
                    Style::default().fg(color).add_modifier(Modifier::BOLD),
                )),
            ])
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(theme.border)),
            )
            .alignment(ratatui::layout::Alignment::Center);
            card.render(*kind_area, buf);
        }
    }

    // 右: type ごとの token 割合の円グラフ
    let pie = MiniPie {
        title: " Tokens by Type ".to_string(),
        slices: d.pie_slices(),
    };
    pie.render(cols[1], buf);
}

/// 円グラフ・ボックスの種別色（`HISTORY_TYPES` の並び順に対応）。
fn pie_color(i: usize) -> ratatui::style::Color {
    const COLORS: [ratatui::style::Color; 5] = [
        ratatui::style::Color::Blue,
        ratatui::style::Color::Green,
        ratatui::style::Color::Yellow,
        ratatui::style::Color::Magenta,
        ratatui::style::Color::Cyan,
    ];
    COLORS[i % COLORS.len()]
}

/// Timeline テーブル（Time / Type / name / token）。内部スクロール方式は
/// Project 画面の Recent テーブルと同一。画面下まで広がる。
fn render_timeline_table(area: Rect, buf: &mut Buffer, d: &mut HistoryViewData, theme: &AppTheme) {
    let visible = area.height.saturating_sub(3) as usize; // 枠 2 + ヘッダ 1
    let visible = visible.max(1);
    d.timeline_visible = visible; // timeline_scroll のクランプ用に実測値を保持
    let max_offset = d.events.len().saturating_sub(visible);
    d.timeline_offset = d.timeline_offset.min(max_offset);

    let start = d.timeline_offset;
    let end = (start + visible).min(d.events.len());
    let header = Row::new(["Time", "Type", "name", "token"])
        .style(Style::default().fg(theme.info).add_modifier(Modifier::BOLD));
    let rows: Vec<Row> = d.events[start..end]
        .iter()
        .map(|e| {
            let color = kind_color(&e.kind);
            Row::new(vec![
                Cell::from(format_time(&e.timestamp)),
                Cell::from(Span::styled(e.kind.clone(), Style::default().fg(color))),
                Cell::from(e.name.clone()),
                Cell::from(crate::fmt::format_num(e.tokens)),
            ])
        })
        .collect();
    let widths = [
        Constraint::Percentage(17),
        Constraint::Percentage(12),
        Constraint::Percentage(59),
        Constraint::Percentage(12),
    ];
    let title = if d.events.is_empty() {
        " Timeline （時系列イベントはありません） ".to_string()
    } else {
        format!(" Timeline {}-{}/{} ", start + 1, end, d.events.len())
    };
    let table = Table::new(rows, widths).header(header).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme.border))
            .title(Span::styled(
                title,
                Style::default().fg(theme.info).add_modifier(Modifier::BOLD),
            )),
    );
    table.render(area, buf);
}

/// 種別の表示色。
fn kind_color(kind: &str) -> ratatui::style::Color {
    match kind {
        "user" => ratatui::style::Color::Blue,
        "agent" => ratatui::style::Color::Green,
        "toolCall" => ratatui::style::Color::Yellow,
        "toolResult" => ratatui::style::Color::Magenta,
        "thinking" => ratatui::style::Color::Cyan,
        _ => ratatui::style::Color::DarkGray,
    }
}

/// ISO 8601 タイムスタンプを `MM-DD HH:MM:SS` 形式に整形する。
///
/// パースはせず固定桁で切り出す（不正値はそのまま短縮表示）。
fn format_time(ts: &str) -> String {
    let t = ts.trim();
    if t.len() < 19 {
        return t.to_string();
    }
    // "2026-08-29T01:00:02.000Z" → "08-29 01:00:02"
    let mut out = String::with_capacity(17);
    out.push_str(&t[5..10]);
    out.push(' ');
    out.push_str(&t[11..19]);
    out
}

// ============================================================================
// テスト
// ============================================================================

#[cfg(test)]
mod tests {
    use crate::theme::AppTheme;
    use ratatui::{backend::TestBackend, Terminal};

    use super::*;

    /// バッファ全文を空白なしで連結した文字列を返す。
    fn flat_content(term: &Terminal<TestBackend>) -> String {
        term.backend()
            .buffer()
            .content
            .iter()
            .map(|c| c.symbol())
            .collect::<String>()
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect()
    }

    /// テスト用データ。
    fn sample_data() -> HistoryViewData {
        HistoryViewData {
            title: "Session #1".into(),
            subtitle: "blk | started: 2026-08-29 01:00:00 | status: completed".into(),
            type_tokens: HistoryTypeTokens {
                user: 100,
                agent: 200,
                tool_call: 50,
                tool_result: 30,
                thinking: 20,
            },
            events: (0..20)
                .map(|i| HistoryEvent {
                    timestamp: format!("2026-08-29T01:00:{:02}.000Z", i),
                    kind: ["user", "agent", "toolCall", "toolResult", "thinking"][i % 5]
                        .to_string(),
                    name: format!("entry{}", i),
                    tokens: 10 * (i as u64 + 1),
                })
                .collect(),
            timeline_offset: 0,
            timeline_visible: DEFAULT_TIMELINE_VISIBLE,
        }
    }

    /// 全セクション（ボックス・円グラフ・Timeline）が描画され、
    /// セッション出力は表示されないこと。
    #[test]
    fn history_view_renders_sections() {
        let backend = TestBackend::new(100, 40);
        let mut term = Terminal::new(backend).unwrap();
        let mut d = sample_data();
        term.draw(|f| {
            render_history_view(f.area(), f.buffer_mut(), &mut d, &AppTheme::dark());
        })
        .unwrap();
        let content = flat_content(&term);
        assert!(
            content.contains("Session#1"),
            "タイトルが表示される: {content}"
        );
        assert!(
            content.contains("TokenUsagebyCategory"),
            "種別セクションが表示される: {content}"
        );
        for kind in HISTORY_TYPES {
            assert!(
                content.contains(kind),
                "種別 {kind} が表示される: {content}"
            );
        }
        // 種別カードに合計値（Project の stat カード同型・ラベル + 数値）が表示される
        assert!(
            content.contains("100"),
            "user の合計値が表示される: {content}"
        );
        assert!(
            content.contains("200"),
            "agent の合計値が表示される: {content}"
        );
        assert!(
            content.contains("50"),
            "toolCall の合計値が表示される: {content}"
        );
        assert!(
            content.contains("TokensbyType"),
            "円グラフが表示される: {content}"
        );
        assert!(
            content.contains("Timeline1-16/20"),
            "Timeline の表示範囲が出る"
        );
        assert!(content.contains("entry0"), "Timeline の行が見える");
        // A画面カード: input = user + toolResult = 130、output = 270、total = 400
        assert!(
            content.contains("input"),
            "input カードが表示される: {content}"
        );
        assert!(
            content.contains("130"),
            "input 合計値が表示される: {content}"
        );
        assert!(
            content.contains("output"),
            "output カードが表示される: {content}"
        );
        assert!(
            content.contains("270"),
            "output 合計値が表示される: {content}"
        );
        assert!(
            content.contains("total"),
            "total カードが表示される: {content}"
        );
        assert!(
            content.contains("400"),
            "total 合計値が表示される: {content}"
        );
    }

    /// Timeline の内部スクロール: オフセット変更で表示行が切り替わること。
    #[test]
    fn history_timeline_scrolls_internally() {
        let mut d = sample_data();
        // 描画前は可視行数が既定値（8）のため +5 で 5 になる
        timeline_scroll(&mut d, 5);
        assert_eq!(d.timeline_offset, 5);
        timeline_scroll(&mut d, -5);
        assert_eq!(d.timeline_offset, 0);

        // 可視行数を超えるイベント数ではスクロールする
        d.events = (0..60)
            .map(|i| HistoryEvent {
                timestamp: format!("2026-08-29T01:{:02}:{:02}.000Z", i / 60, i % 60),
                kind: "user".into(),
                name: format!("entry{}", i),
                tokens: 1,
            })
            .collect();
        // 描画して可視行数を実測（25 行）した後はスクロールできる
        let backend = TestBackend::new(100, 40);
        let mut term = Terminal::new(backend).unwrap();
        term.draw(|f| {
            render_history_view(f.area(), f.buffer_mut(), &mut d, &AppTheme::dark());
        })
        .unwrap();
        timeline_scroll(&mut d, 5);
        assert_eq!(d.timeline_offset, 5);
        timeline_scroll(&mut d, 100);
        assert_eq!(d.timeline_offset, 44, "max offset = 60 - 可視 16 行");
        term.draw(|f| {
            render_history_view(f.area(), f.buffer_mut(), &mut d, &AppTheme::dark());
        })
        .unwrap();
        let content = flat_content(&term);
        assert!(
            content.contains("Timeline45-60/60"),
            "表示範囲が正しい: {content}"
        );
        assert!(content.contains("entry59"), "末尾の行が見える");
    }

    /// Timeline は画面下まで広がる（セクション固定高より多くの行が見える）。
    #[test]
    fn history_timeline_fills_bottom() {
        let backend = TestBackend::new(100, 40);
        let mut term = Terminal::new(backend).unwrap();
        let mut d = sample_data();
        term.draw(|f| {
            render_history_view(f.area(), f.buffer_mut(), &mut d, &AppTheme::dark());
        })
        .unwrap();
        // 40 行画面: usage 18 + ガイド 1 + タイトル 2 を除いた Timeline は高さ 19
        // → 可視行数 = 19 - 3（枠 + ヘッダ）= 16 行（実測値が保持される）
        assert_eq!(d.timeline_visible, 16);
        // 16 件が可視（Timeline は画面下まで広がる）
        let content = flat_content(&term);
        assert!(content.contains("Timeline1-16/20"), "表示範囲: {content}");
        assert!(
            content.contains("entry15"),
            "16 行目まで表示される: {content}"
        );
    }

    /// イベント無しでも panic しないこと。
    #[test]
    fn history_view_renders_empty() {
        let backend = TestBackend::new(100, 40);
        let mut term = Terminal::new(backend).unwrap();
        let mut d = HistoryViewData::default();
        d.title = "Session #0".into();
        term.draw(|f| {
            render_history_view(f.area(), f.buffer_mut(), &mut d, &AppTheme::dark());
        })
        .unwrap();
        let content = flat_content(&term);
        assert!(content.contains("時系列イベントはありません"));
    }

    /// 極小領域でも panic しないこと。
    #[test]
    fn history_view_renders_tiny_area() {
        let backend = TestBackend::new(20, 5);
        let mut term = Terminal::new(backend).unwrap();
        let mut d = sample_data();
        term.draw(|f| {
            render_history_view(f.area(), f.buffer_mut(), &mut d, &AppTheme::dark());
        })
        .unwrap();
        let content = flat_content(&term);
        assert!(
            content.contains("History:"),
            "フォールバック表示: {content}"
        );
    }

    /// 時刻整形。
    #[test]
    fn test_format_time() {
        assert_eq!(format_time("2026-08-29T01:00:02.000Z"), "08-29 01:00:02");
        assert_eq!(format_time("bad"), "bad");
        assert_eq!(format_time(""), "");
    }
}
