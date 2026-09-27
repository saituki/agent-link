//! Kanban ボード描画（読み取り専用）。
//!
//! Card（長方形）を4つのレーンに並べて表示するメイン画面上部。
//! 直接操作は受け付けない（操作は sidebar の `c` / `d` および `Shift+C`）。
//! Card 書式は Workflow と統一（角丸 `╭╮╰╯`。run_id は上辺左）:
//! ```text
//! ╭ ID<run_id> ──────╮
//! │ <block名>        │
//! │ MM/DD HH:MM:SS   │
//! │ <description...> │
//! ╰──────────────────╯
//! ```

use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
    widgets::{Block, Borders, Widget},
};

use crate::widgets::block_node::wrap_description;
use sai_link_widgets::theme::AppTheme;

/// Kanban 上の Card 1件（Workflow の `BlockNodeData` と同内容）。
#[derive(Debug, Clone, Default)]
pub struct KanbanCardView {
    /// 1行目（`ID<run_id>/<block名>`・todo は `todo/<run_id>`）
    pub title: String,
    /// 2行目（日付）
    pub date: String,
    /// 3行目以降（対応カードの frontmatter `description:`）
    pub description: String,
    /// 対応カード md のパス（選択照合用。無ければ `None`）
    pub card_path: Option<std::path::PathBuf>,
    /// 対応する実行 ID（パスが無いカードの選択照合用。todo は `None`）
    pub exec_id: Option<i64>,
    /// runner（pi / cc など。Workflow の Card と表記を揃える。未実行は空）
    pub runner: String,
    /// 完了実行の費用表示（例 `$0.42` / `費用不明`）。未完了・未実行は `None`
    pub cost: Option<String>,
}

/// レーン表示データ（`app/kanban.rs` が構築・このモジュールは描画のみ）。
#[derive(Debug, Clone, Default)]
pub struct KanbanBoardData {
    /// 4レーンの Card 一覧（順序: todo / running / review / done）
    pub lanes: [Vec<KanbanCardView>; 4],
    /// 選択中カードの所属レーン（Header 文字の色分け用。`None` = カード未選択）
    pub selected_lane: Option<usize>,
    /// 選択中カードのパス（個別 Card の色分け用。パス比較のため同名 Card の誤照合なし）
    pub selected_card_path: Option<std::path::PathBuf>,
    /// 選択中カードの実行 ID（パスが無いカードの色分け用）
    pub selected_exec_id: Option<i64>,
    /// レーン共通の縦スクロール位置（フォーカス時のホイール / 矢印キーで増減）
    pub scroll: usize,
}

/// レーン名（表示用）。
pub const LANE_NAMES: [&str; 4] = ["todo", "run", "review", "done"];

/// Kanban カードの高さ（行数。枠2行 + ID/日付/description1行の最小構成）。
const CARD_HEIGHT: u16 = 5;

/// Kanban ボードウィジェット（読み取り専用）。
pub struct KanbanBoard<'a> {
    /// ボードデータ
    pub data: &'a KanbanBoardData,
    /// テーマ
    pub theme: &'a AppTheme,
}

impl Widget for KanbanBoard<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.width < 8 || area.height < 5 {
            return;
        }
        // 背景を Workflow と同じ main_bg で塗る（外枠は draw 側の面板枠線）。
        for y in area.top()..area.bottom() {
            for x in area.left()..area.right() {
                if let Some(cell) = buf.cell_mut((x, y)) {
                    cell.set_bg(self.theme.main_bg);
                }
            }
        }
        // Header 行にレーン名を表示する（枠なし・レーン幅中央）。
        // 面板枠線（draw 側・後描画）との重なりを避けるため、レーン枠は Header の下から開始する。
        let header_y = area.y.saturating_add(1);
        let body = Rect::new(
            area.x.saturating_add(1),
            area.y.saturating_add(2),
            area.width.saturating_sub(2),
            area.height.saturating_sub(3),
        );
        if body.height < 4 {
            return;
        }
        // 4レーンを横に均等分割する
        let lane_w = body.width / 4;
        for (i, name) in LANE_NAMES.iter().enumerate() {
            let x = body.x + lane_w * i as u16;
            let w = if i == 3 {
                body.x + body.width - x
            } else {
                lane_w
            };
            let lane_area = Rect::new(x, body.y, w, body.height);
            let highlighted = self.data.selected_lane == Some(i);
            let header_style = if highlighted {
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(self.theme.muted)
            };
            // Header 文字はレーン幅中央に配置する
            let label = format!(" {} ", name);
            let hx = x + w.saturating_sub(label.len() as u16) / 2;
            crate::ui::draw::draw_text_cjk(buf, hx, header_y, &label, header_style);
            // レーン仕切線は通常色で固定（選択時は Header 文字のみ yellow）
            let border_style = Style::default().fg(self.theme.border);
            let block = Block::default()
                .borders(Borders::ALL)
                .border_style(border_style);
            let inner = block.inner(lane_area);
            block.render(lane_area, buf);
            if inner.height == 0 || inner.width == 0 {
                continue;
            }
            let cards = &self.data.lanes[i];
            if cards.is_empty() {
                let style = Style::default().fg(self.theme.muted);
                crate::ui::draw::draw_text_cjk(buf, inner.x, inner.y, "(なし)", style);
                continue;
            }
            // レーンの縦スクロール（レーンごとに枚数が違うため上限はレーン内でクランプ）。
            // 下から切り捨てる挙動はそのまま（枠外には出さない）。
            let scroll = self.data.scroll.min(cards.len().saturating_sub(1));
            let mut y = inner.y;
            for card in cards.iter().skip(scroll) {
                if y + CARD_HEIGHT > inner.bottom() {
                    break;
                }
                // レーン幅いっぱいに Card 矩形を描画する
                let card_area = Rect::new(inner.x, y, inner.width, CARD_HEIGHT);
                // description は Card 内の残り1行に収める
                let desc =
                    wrap_description(&card.description, inner.width.saturating_sub(3) as usize)
                        .into_iter()
                        .next()
                        .unwrap_or_default();
                // `BlockNodeWidget` は `ID<exec_id>/<name>` 形式のため、
                // Kanban 用に title/date/desc を直接描画する軽量版を使う。
                // カーソル選択中カード → 黄色、それ以外 → 白色。
                // パス照合を優先し、パスが無い実行は実行 ID で照合する
                // （タスク作成（`task_*`）由来のカードはパスが無く黄色にならなかった）。
                let card_fg = {
                    let path_hit = matches!(
                        (self.data.selected_card_path.as_deref(), card.card_path.as_deref()),
                        (Some(sel), Some(p)) if sel == p
                    );
                    let exec_hit = matches!(
                        (self.data.selected_exec_id, card.exec_id),
                        (Some(s), Some(p)) if s == p
                    );
                    if path_hit || exec_hit {
                        Color::Yellow
                    } else {
                        Color::White
                    }
                };
                render_kanban_card(
                    card_area,
                    buf,
                    &card.title,
                    &card.runner,
                    &card.date,
                    &desc,
                    card.cost.as_deref(),
                    card_fg,
                    self.theme.main_bg,
                );
                y += CARD_HEIGHT;
            }
        }
    }
}

/// `KanbanCardView.title`（`ID<run_id>/<block名>` 形式）を run_id 部分と表示名部分に分ける。
///
/// workflow の card（`BlockNodeWidget`）と同じく run_id を上辺へ、表示名を本文へ出すため。
/// `/` が無い場合（想定外の形式）はそのまま表示名として扱う。
fn split_kanban_title(title: &str) -> (&str, &str) {
    match title.split_once('/') {
        Some((id, name)) => (id, name),
        None => ("", title),
    }
}

/// Kanban 用 Card 矩形を描画する（`BlockNodeWidget` の軽量版）。
///
/// 上辺左に `ID<run_id>`（workflow の card と同じ見た目）、1行目に表示名、
/// 2行目に日付、3行目に desc を描画する。はみ出しはクリップし、枠外には出さない。
fn render_kanban_card(
    area: Rect,
    buf: &mut Buffer,
    title: &str,
    runner: &str,
    date: &str,
    desc: &str,
    cost: Option<&str>,
    fg: Color,
    bg: Color,
) {
    let style = Style::default().fg(fg).bg(bg);
    let border = Style::default().fg(fg).bg(bg);
    if area.width < 3 || area.height < 3 {
        return;
    }
    let (id_part, name_part) = split_kanban_title(title);
    // Workflow の card（`BlockNodeWidget`）と同じく上辺に `ID<run_id>/<runner>` を出す。
    let id_display = if runner.is_empty() {
        id_part.to_string()
    } else {
        format!("{}/{}", id_part, runner)
    };
    let id_part: &str = &id_display;
    // 上辺（workflow の card と同じ角丸 `╭╮`）
    for x in area.x..area.right() {
        let ch = if x == area.x {
            '╭'
        } else if x + 1 == area.right() {
            '╮'
        } else {
            '─'
        };
        if let Some(cell) = buf.cell_mut((x, area.y)) {
            cell.set_char(ch).set_style(border);
        }
    }
    // 上辺左に ID<run_id> を重ねる（残りの `─` は潰さない）
    if !id_part.is_empty() {
        let label = format!(" {} ", id_part);
        let mut cx = area.x + 2;
        let max_x = area.right().saturating_sub(1);
        for ch in label.chars() {
            let w = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(1) as u16;
            if w == 0 || cx + w > max_x {
                break;
            }
            crate::ui::draw::draw_char_cjk(buf, cx, area.y, ch, border);
            cx += w;
        }
    }
    // 下辺（workflow の card と同じ角丸 `╰╯`）
    let by = area.bottom() - 1;
    for x in area.x..area.right() {
        let ch = if x == area.x {
            '╰'
        } else if x + 1 == area.right() {
            '╯'
        } else {
            '─'
        };
        if let Some(cell) = buf.cell_mut((x, by)) {
            cell.set_char(ch).set_style(border);
        }
    }
    // 中段（1行目は run_id を除いた表示名）
    let inner_w = area.width.saturating_sub(2) as usize;
    let lines = [name_part, date, desc];
    for (i, line) in lines.iter().enumerate() {
        let y = area.y + 1 + i as u16;
        if y >= by {
            break;
        }
        if let Some(cell) = buf.cell_mut((area.x, y)) {
            cell.set_char('│').set_style(border);
        }
        if let Some(cell) = buf.cell_mut((area.right() - 1, y)) {
            cell.set_char('│').set_style(border);
        }
        // CJK 対応クリップ描画（bn の内部関数相当をここで再現）
        let text = format!(" {}", line);
        let mut cx = area.x + 1;
        let mut remaining = inner_w;
        for ch in text.chars() {
            let w = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(1);
            if w == 0 || remaining == 0 {
                continue;
            }
            if w as usize > remaining {
                break;
            }
            crate::ui::draw::draw_char_cjk(buf, cx, y, ch, style);
            cx += w as u16;
            remaining -= w;
        }
        for k in 0..remaining {
            if let Some(cell) = buf.cell_mut((cx + k as u16, y)) {
                cell.set_char(' ').set_style(style);
            }
        }
        // 2 行目（日付）の右端に費用を出す（B02）。
        // 日付と衝突する幅しか無ければ描かない（枠外・重なりを避ける）。
        if i == 1 {
            if let Some(cost) = cost {
                let date_w: usize = text
                    .chars()
                    .map(|c| unicode_width::UnicodeWidthChar::width(c).unwrap_or(1))
                    .sum();
                let cost_w: usize = cost
                    .chars()
                    .map(|c| unicode_width::UnicodeWidthChar::width(c).unwrap_or(1))
                    .sum();
                if cost_w > 0 && date_w + cost_w < inner_w {
                    // 右端（右ボーダーの手前）から左へ詰めて描画する
                    let start = area.right() - 1 - cost_w as u16;
                    let mut cx = start;
                    for ch in cost.chars() {
                        let w = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(1);
                        if w == 0 {
                            continue;
                        }
                        crate::ui::draw::draw_char_cjk(buf, cx, y, ch, style);
                        cx += w as u16;
                    }
                }
            }
        }
    }
    // 残り行（CARD_HEIGHT > 5 の場合）は空白で埋める
    for y in area.y + 1 + lines.len() as u16..by {
        if let Some(cell) = buf.cell_mut((area.x, y)) {
            cell.set_char('│').set_style(border);
        }
        if let Some(cell) = buf.cell_mut((area.right() - 1, y)) {
            cell.set_char('│').set_style(border);
        }
        for x in area.x + 1..area.right() - 1 {
            if let Some(cell) = buf.cell_mut((x, y)) {
                cell.set_char(' ').set_style(style);
            }
        }
    }
}

// ============================================================================
// テスト
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn card(title: &str) -> KanbanCardView {
        KanbanCardView {
            title: title.to_string(),
            date: "08/01 10:00:00".to_string(),
            description: "説明文".to_string(),
            card_path: None,
            exec_id: None,
            runner: String::new(),
            cost: None,
        }
    }

    #[test]
    fn renders_four_lanes_without_panic() {
        let theme = AppTheme::dark();
        let data = KanbanBoardData {
            lanes: [
                vec![card("todo/a.md")],
                vec![],
                vec![card("ID11/review")],
                vec![card("ID9/done")],
            ],
            selected_lane: Some(0),
            selected_card_path: None,
            selected_exec_id: None,
            scroll: 0,
        };
        let area = Rect::new(0, 0, 80, 20);
        let mut buf = Buffer::empty(area);
        KanbanBoard {
            data: &data,
            theme: &theme,
        }
        .render(area, &mut buf);
        // 先頭セルの走査ではなく、4レーン名がどこかに描画されていること
        let mut symbols = String::new();
        for y in 0..area.height {
            for x in 0..area.width {
                symbols.push_str(buf[(x, y)].symbol());
            }
        }
        for name in LANE_NAMES {
            assert!(symbols.contains(name), "レーン {} が描画される", name);
        }
    }

    /// Card の書式（上辺の ID・1行目名前・日付・description）が描画されること。
    /// run_id は workflow の card と同じく上辺へ移動し、1行目は表示名のみになる。
    #[test]
    fn renders_card_three_lines() {
        let theme = AppTheme::dark();
        let data = KanbanBoardData {
            lanes: [vec![card("ID1/b")], vec![], vec![], vec![]],
            selected_lane: None,
            selected_card_path: None,
            selected_exec_id: None,
            scroll: 0,
        };
        let area = Rect::new(0, 0, 80, 20);
        let mut buf = Buffer::empty(area);
        KanbanBoard {
            data: &data,
            theme: &theme,
        }
        .render(area, &mut buf);
        // 全角文字の右半分セルは空 symbol のため、各セルの先頭 char を集める
        let mut symbols = String::new();
        for y in 0..area.height {
            for x in 0..area.width {
                let s = buf[(x, y)].symbol();
                if s.is_empty() {
                    continue;
                }
                symbols.push_str(s);
            }
        }
        assert!(symbols.contains("ID1"), "run_id が上辺に描画される");
        assert!(symbols.contains('b'), "表示名が本文に描画される");
        assert!(!symbols.contains("ID1/b"), "run_id と表示名は分離されるべき");
        assert!(
            symbols.contains("説") && symbols.contains("明") && symbols.contains("文"),
            "description が描画される"
        );
    }

    /// 完了実行のカードは日付行の右端に費用を出すこと（B02）。
    #[test]
    fn renders_card_cost_on_date_row() {
        let theme = AppTheme::dark();
        let mut c = card("ID1/b");
        c.cost = Some("$0.42".to_string());
        let data = KanbanBoardData {
            lanes: [vec![c], vec![], vec![], vec![]],
            selected_lane: None,
            selected_card_path: None,
            selected_exec_id: None,
            scroll: 0,
        };
        let area = Rect::new(0, 0, 160, 20);
        let mut buf = Buffer::empty(area);
        KanbanBoard {
            data: &data,
            theme: &theme,
        }
        .render(area, &mut buf);
        let mut symbols = String::new();
        for y in 0..area.height {
            for x in 0..area.width {
                let s = buf[(x, y)].symbol();
                if s.is_empty() {
                    continue;
                }
                symbols.push_str(s);
            }
        }
        assert!(symbols.contains("$0.42"), "費用が描画される: {}", symbols);
    }
}
