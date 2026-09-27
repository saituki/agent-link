//! ブロックノードウィジェット。
//!
//! ブロック接続グラフ上の各ブロックを矩形ノードとして描画します。

use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
    widgets::Widget,
};

use sai_link_engine::reflection_status::ReflectionStatus;

use crate::ui::draw;

// ============================================================================
// Notification 色分け（記録の書き忘れを色で知らせる）
// ============================================================================

/// Reflection 記録の記入状況を文字色へ変換する（Notification 4 色ルール）。
///
/// | 記入状況 | 色 |
/// |---|---|
/// | 未記入 | 赤 |
/// | Review のみ | 緑 |
/// | 内容の記録のみ | 青 |
/// | 両方書き終えた | 白 |
pub fn reflection_status_color(status: ReflectionStatus) -> Color {
    match status {
        ReflectionStatus::Empty => Color::Red,
        ReflectionStatus::ReviewOnly => Color::Rgb(110, 220, 130), // 明るい緑
        ReflectionStatus::ContentOnly => Color::Rgb(110, 170, 255), // 明るい青
        ReflectionStatus::Complete => Color::White,
    }
}

// ============================================================================
// BlockNodeWidget
// ============================================================================

/// ブロックノードの描画データ。
///
/// Card 書式（Workflow / Kanban 共通）:
/// ```text
/// ID<run_id>/<block名>
/// MM/DD HH:MM:SS
/// <description複数行・枠内に収まる範囲>
/// ```
#[derive(Debug, Clone)]
pub struct BlockNodeData {
    /// ブロック名
    pub name: String,
    /// タグ
    pub tag: Option<String>,
    /// 実行ID
    pub exec_id: i64,
    /// runner（pi / cc など）
    pub runner: String,
    /// 実行日時（ISO 8601）
    pub created_at: String,
    /// 選択中かどうか
    pub selected: bool,
    /// Card 表示用の説明文（3行目以降。枠内に収まる範囲で複数行表示）。
    ///
    /// 空の場合は 3 行目以降を空白で埋める。
    pub description: String,
    /// 旧 Notification 色分け用の色（廃止済み・互換のため残す。描画では無視）。
    pub status_color: Option<Color>,
    /// 完了実行の費用表示（例 `$0.42` / `費用不明`）。未完了・未設定は `None`（B02）。
    ///
    /// 描画は日付行の右端。日付と重なる幅しか無い場合は描かない。
    pub cost: Option<String>,
}

/// ブロックノード描画ウィジェット。
pub struct BlockNodeWidget<'a> {
    pub data: &'a BlockNodeData,
    pub bg: Color,
}

impl<'a> BlockNodeWidget<'a> {
    pub fn new(data: &'a BlockNodeData, bg: Color) -> Self {
        Self { data, bg }
    }
}

/// カード選択中の枠線色（`sai-link-widgets::theme::AppTheme::selected` と同値）。
const SELECTED_BORDER: Color = Color::Rgb(255, 200, 50);

impl Widget for BlockNodeWidget<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        // カーソル選択中 → 黄色+太字、それ以外 → 白色。
        // 旧 Notification 色分け（Reflection/Review 記入状況）は廃止。
        let text_style = if self.data.selected {
            Style::default()
                .fg(Color::Yellow)
                .bg(self.bg)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::White).bg(self.bg)
        };

        let border_style = Style::default().bg(self.bg);
        let border_fg = if self.data.selected {
            SELECTED_BORDER
        } else {
            match self.data.tag.as_deref() {
                Some("brst") => Color::Cyan,
                Some("plan") => Color::Blue,
                Some("code") => Color::Green,
                Some("comp") => Color::Magenta,
                Some("rfrs") => Color::Red,
                _ => Color::Gray,
            }
        };

        let width = area.width as usize;
        let h = area.height as usize;

        // 上部ボーダー: ╭──────╮
        draw_horizontal_border(buf, area, area.y, '╭', '─', '╮', border_style.fg(border_fg));
        // 上辺の左に `ID<run_id>` を重ねる（ekphos 方式）。色は枠線色、選択中は太字。
        // ラベル自身の表示幅だけを描画幅にし、残りの `─` を空白で潰さないようにする。
        let id_style = if self.data.selected {
            border_style.fg(border_fg).add_modifier(Modifier::BOLD)
        } else {
            border_style.fg(border_fg)
        };
        let id_label = if self.data.runner.is_empty() {
            format!(" ID{} ", self.data.exec_id)
        } else {
            format!(" ID{}/{} ", self.data.exec_id, self.data.runner)
        };
        let id_label_width: usize = id_label
            .chars()
            .map(|c| unicode_width::UnicodeWidthChar::width(c).unwrap_or(1))
            .sum();
        draw_text_inner(
            buf,
            area.x + 2,
            area.y,
            &id_label,
            id_label_width.min(width.saturating_sub(4)),
            id_style,
        );

        // 中段: 左右の縦線 + テキスト内容
        // 1行目: ブロック名のみ（`ID<run_id>` は上辺へ移動済み） / 2行目: `MM/DD HH:MM:SS`
        // 3行目以降: `description` を枠内に収まる範囲で複数行表示する。
        // `+`/`-` でノード矩形が変わっても残り行数で再計算されるため追従する。
        let title = format!(" {}", self.data.name);
        let date_str = format!(" {}", format_iso_date_seconds(&self.data.created_at));
        let desc_lines = wrap_description(&self.data.description, width.saturating_sub(2));
        for row in 1..h.saturating_sub(1) {
            // 左縦線
            set_cell(
                buf,
                area.x,
                area.y + row as u16,
                '│',
                border_style.fg(border_fg),
            );

            let content_row = row;
            if content_row == 1 {
                draw_text_inner(
                    buf,
                    area.x + 1,
                    area.y + row as u16,
                    &title,
                    width.saturating_sub(2),
                    text_style,
                );
            } else if content_row == 2 && h > 3 {
                draw_text_inner(
                    buf,
                    area.x + 1,
                    area.y + row as u16,
                    &date_str,
                    width.saturating_sub(2),
                    text_style,
                );
                // 日付行の右端に費用を出す（B02）。幅が足りなければ省く。
                if let Some(cost) = self.data.cost.as_deref() {
                    let inner_w = width.saturating_sub(2);
                    let cost_w = text_width(cost);
                    if cost_w > 0 && text_width(&date_str) + cost_w < inner_w {
                        let mut cx = area.x + 1 + (inner_w - cost_w) as u16;
                        for ch in cost.chars() {
                            let w = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(1);
                            if w == 0 {
                                continue;
                            }
                            draw::draw_char_cjk(buf, cx, area.y + row as u16, ch, text_style);
                            cx += w as u16;
                        }
                    }
                }
            } else if content_row >= 3 && h > 4 {
                // description の該当行（なければ空白で埋める）
                let line = desc_lines
                    .get(content_row - 3)
                    .map(|s| format!(" {}", s))
                    .unwrap_or_default();
                draw_text_inner(
                    buf,
                    area.x + 1,
                    area.y + row as u16,
                    &line,
                    width.saturating_sub(2),
                    text_style,
                );
            } else {
                // 空行 → スペースで塗る（背景色統一）
                fill_row(
                    buf,
                    area.x + 1,
                    area.y + row as u16,
                    width.saturating_sub(2),
                    text_style,
                );
            }

            // 右縦線
            set_cell(
                buf,
                area.x + area.width.saturating_sub(1),
                area.y + row as u16,
                '│',
                border_style.fg(border_fg),
            );
        }

        // 下部ボーダー: ╰──────╯（最下行）
        if h >= 2 {
            draw_horizontal_border(
                buf,
                area,
                area.y + area.height.saturating_sub(1),
                '╰',
                '─',
                '╯',
                border_style.fg(border_fg),
            );
        }
    }
}

// ============================================================================
// ヘルパー
// ============================================================================

/// 水平ボーダー行を指定されたy座標に描画する。
fn draw_horizontal_border(
    buf: &mut Buffer,
    area: Rect,
    y: u16,
    left: char,
    mid: char,
    right: char,
    style: Style,
) {
    set_cell(buf, area.x, y, left, style);
    for x in area.x + 1..area.x + area.width.saturating_sub(1) {
        set_cell(buf, x, y, mid, style);
    }
    set_cell(buf, area.x + area.width.saturating_sub(1), y, right, style);
}

/// セルに安全に文字を設定する。
fn set_cell(buf: &mut Buffer, x: u16, y: u16, ch: char, style: Style) {
    if let Some(cell) = buf.cell_mut((x, y)) {
        cell.set_char(ch).set_style(style);
    }
}

/// 文字列の表示幅（CJK 全角 = 2 セル）。
fn text_width(s: &str) -> usize {
    s.chars()
        .map(|c| unicode_width::UnicodeWidthChar::width(c).unwrap_or(1))
        .sum()
}

/// テキストを指定位置に描画（幅制限あり、枠内用）。CJK 対応。
fn draw_text_inner(buf: &mut Buffer, x: u16, y: u16, text: &str, max_width: usize, style: Style) {
    let mut cx = x;
    let mut remaining = max_width;
    for ch in text.chars() {
        let width = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(1);
        if width == 0 || remaining == 0 {
            continue;
        }
        if width as usize > remaining {
            break;
        }
        draw::draw_char_cjk(buf, cx, y, ch, style);
        cx += width as u16;
        remaining -= width;
    }
    // 残りをデフォルト背景で塗る
    for i in 0..remaining {
        set_cell(buf, cx + i as u16, y, ' ', style);
    }
}

/// 1行を指定スタイルのスペースで塗りつぶす。
fn fill_row(buf: &mut Buffer, x: u16, y: u16, width: usize, style: Style) {    for i in 0..width {
        set_cell(buf, x + i as u16, y, ' ', style);
    }
}

/// ISO 8601 日時文字列を "MM/DD HH:MM:SS" 形式に変換する（年は省略）。
/// 例: "2026-07-31T12:03:45" → "07/31 12:03:45"
fn format_iso_date_seconds(iso: &str) -> String {
    if iso.len() >= 19 {
        // 月/日 (5..10) と 時:分:秒 (11..19)
        format!("{} {}", &iso[5..10].replace('-', "/"), &iso[11..19])
    } else if iso.len() >= 16 {
        format!("{} {}", &iso[5..10].replace('-', "/"), &iso[11..16])
    } else {
        iso.to_string()
    }
}

/// `YYYY/MM/DD HH:MM:SS` を `MM/DD HH:MM` に変換する（sidebar todo 子の表示用）。
/// 形式外はそのまま返す。
pub fn format_creation_date_todo(date: &str) -> String {
    let t = date.trim();
    if t.len() >= 16 && t.as_bytes().get(4) == Some(&b'/') {
        // `YYYY/MM/DD HH:MM:SS` → `MM/DD HH:MM`
        format!("{} {}", &t[5..10].replace('-', "/"), &t[11..16])
    } else {
        t.to_string()
    }
}

/// description を表示幅に合わせて折り返す（CJK 幅対応）。
///
/// 改行で段落分割し、各段落を表示幅で折り返す。空行は空行のまま残す。
/// 呼び出し側が残り行数で切り詰める（枠外に出さない）。
pub fn wrap_description(desc: &str, max_width: usize) -> Vec<String> {
    use unicode_width::UnicodeWidthChar;
    if max_width == 0 {
        return Vec::new();
    }
    let mut out: Vec<String> = Vec::new();
    if desc.trim().is_empty() {
        return out;
    }
    for para in desc.split('\n') {
        if para.trim().is_empty() {
            out.push(String::new());
            continue;
        }
        let mut cur = String::new();
        let mut cur_w = 0usize;
        for ch in para.chars() {
            let w = UnicodeWidthChar::width(ch).unwrap_or(1).max(1);
            if cur_w + w > max_width {
                out.push(cur);
                cur = String::new();
                cur_w = 0;
                if w > max_width {
                    continue;
                }
            }
            cur.push(ch);
            cur_w += w;
        }
        out.push(cur);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::buffer::Buffer;

    fn node(cost: Option<&str>) -> BlockNodeData {
        BlockNodeData {
            name: "b".to_string(),
            tag: None,
            exec_id: 1,
            runner: "pi".to_string(),
            created_at: "2026-09-26T12:00:00".to_string(),
            selected: false,
            description: String::new(),
            status_color: None,
            cost: cost.map(|s| s.to_string()),
        }
    }

    fn render(data: &BlockNodeData, w: u16, h: u16) -> String {
        let area = Rect::new(0, 0, w, h);
        let mut buf = Buffer::empty(area);
        BlockNodeWidget::new(data, Color::Black).render(area, &mut buf);
        buf.content.iter().map(|c| c.symbol()).collect()
    }

    /// 完了実行の Card は日付行の右端に費用を描く。
    #[test]
    fn renders_cost_on_date_row() {
        let text = render(&node(Some("$0.42")), 30, 6);
        assert!(text.contains("$0.42"), "費用が描画される: {}", text);
    }

    /// 幅が足りなければ費用は描かない（日付を潰さない）。
    #[test]
    fn omits_cost_when_too_narrow() {
        let text = render(&node(Some("$0.42")), 14, 6);
        assert!(!text.contains("$0.42"), "狭い Card では省略: {}", text);
    }
}
