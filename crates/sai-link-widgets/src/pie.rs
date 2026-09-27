//! 簡易円グラフ部品（L3: `sai-link-tui` の `ui/cache_view.rs` から移動）。
//!
//! `sai-cache-tui-rs` は `tui-piechart`（ratatui 0.30 依存）を使用するが、
//! 本クレートは ratatui 0.29 のためバッファを共有できない。同じ見た目
//! （Braille 解像度の円 + 凡例）をローカル実装で再現する。
//!
//! `MiniPie` は移動後も `cache_view.rs`（Stats 画面）や History 画面から
//! 参照されるため、このクレート側で pub として公開する。

use ratatui::{
    buffer::Buffer,
    layout::{Margin, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Widget},
};

// ── 円グラフ ─────────────────────────────────────────────────────────────

/// 円グラフの 1 スライス。
pub struct PieSliceData {
    pub label: String,
    pub value: f64,
    pub color: Color,
}

/// 簡易円グラフ（Braille ドット描画 + 右側凡例）。
///
/// `sai-cache-tui-rs` は `tui-piechart`（ratatui 0.30 依存）を使用するが、
/// 本クレートは ratatui 0.29 のためバッファを共有できない。同じ見た目
/// （Braille 解像度の円 + 凡例）をローカル実装で再現する。
pub struct MiniPie {
    pub title: String,
    pub slices: Vec<PieSliceData>,
}

/// Braille 点灯パターン（dx, dy）→ ビット。
fn braille_bit(dx: usize, dy: usize) -> u8 {
    match (dx, dy) {
        (0, 0) => 0x01,
        (0, 1) => 0x02,
        (0, 2) => 0x04,
        (1, 0) => 0x08,
        (1, 1) => 0x10,
        (1, 2) => 0x20,
        (0, 3) => 0x40,
        (1, 3) => 0x80,
        _ => 0,
    }
}

impl Widget for MiniPie {
    fn render(self, area: Rect, buf: &mut Buffer) {
        Block::default()
            .borders(Borders::ALL)
            .title(self.title.clone())
            .render(area, buf);

        let inner = area.inner(Margin {
            vertical: 1,
            horizontal: 1,
        });
        if inner.width == 0 || inner.height == 0 || self.slices.is_empty() {
            return;
        }

        // 凡例の幅（label + % の最長 + マーカー分）
        let total: f64 = self.slices.iter().map(|s| s.value).sum();
        let legend_width = self
            .slices
            .iter()
            .map(|s| s.label.chars().count() + 8)
            .max()
            .unwrap_or(0) as u16
            + 1;
        let pie_w = inner.width.saturating_sub(legend_width);
        let pie_area = Rect {
            x: inner.x,
            y: inner.y,
            width: pie_w,
            height: inner.height,
        };

        // ── 円本体（Braille ドット描画） ──
        if pie_w > 0 && total > 0.0 {
            // ドット座標系（Braille はセル 1 個 = 2x4 ドット。ドット間隔は
            // ほぼ正方形になるため、単位アスペクトの円として描画できる）
            let dots_w = pie_area.width as f64 * 2.0;
            let dots_h = pie_area.height as f64 * 4.0;
            let (cx, cy) = (dots_w / 2.0, dots_h / 2.0);
            let radius = (dots_w.min(dots_h) / 2.0) - 1.0;

            // スライスの累積割合（12 時方向から時計回り）
            let mut bounds: Vec<f64> = Vec::with_capacity(self.slices.len() + 1);
            let mut acc = 0.0;
            for s in &self.slices {
                bounds.push(acc);
                acc += s.value / total;
            }
            bounds.push(acc.max(1.0));

            for cy_cell in 0..pie_area.height {
                for cx_cell in 0..pie_area.width {
                    let mut bits: u8 = 0;
                    let mut cell_color: Option<Color> = None;
                    for dy in 0..4usize {
                        for dx in 0..2usize {
                            let px = cx_cell as f64 * 2.0 + dx as f64 + 0.5;
                            let py = cy_cell as f64 * 4.0 + dy as f64 + 0.5;
                            let ddx = px - cx;
                            let ddy = py - cy;
                            if ddx * ddx + ddy * ddy > radius * radius {
                                continue;
                            }
                            // 角度: 12 時方向 = 0、時計回りが正
                            let theta = (std::f64::consts::FRAC_PI_2 - (-ddy).atan2(ddx))
                                .rem_euclid(std::f64::consts::TAU);
                            let frac = theta / std::f64::consts::TAU;
                            let idx =
                                match bounds.windows(2).position(|w| frac >= w[0] && frac < w[1]) {
                                    Some(i) => i,
                                    None => self.slices.len() - 1,
                                };
                            bits |= braille_bit(dx, dy);
                            cell_color = Some(self.slices[idx].color);
                        }
                    }
                    if bits != 0 {
                        if let Some(cell) =
                            buf.cell_mut((pie_area.x + cx_cell, pie_area.y + cy_cell))
                        {
                            cell.set_char(char::from_u32(0x2800 + bits as u32).unwrap_or('●'));
                            if let Some(c) = cell_color {
                                cell.fg = c;
                            }
                        }
                    }
                }
            }
        }

        // ── 凡例（右側・縦並び） ──
        for (i, s) in self.slices.iter().enumerate() {
            let row = inner.y + i as u16;
            if row >= inner.bottom() {
                break;
            }
            let pct = if total > 0.0 {
                s.value / total * 100.0
            } else {
                0.0
            };
            let line = Line::from(vec![
                Span::styled("■ ", Style::default().fg(s.color)),
                Span::raw(s.label.clone()),
                Span::styled(
                    format!(" {:.1}%", pct),
                    Style::default().fg(Color::DarkGray),
                ),
            ]);
            let x = inner.x + pie_w;
            let legend_area = Rect {
                x,
                y: row,
                width: inner.right().saturating_sub(x),
                height: 1,
            };
            Paragraph::new(line).render(legend_area, buf);
        }
    }
}
