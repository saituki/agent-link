//! UI レイアウト計算。
//!
//! 画面を 左SB + メイン に分割し、メインをさらに Kanban(上) + Workflow(下) に
//! 分割します（Header・右SB は廃止）。
//! 左SB 幅・Kanban 高さはドラッグ操作で変更可能です。

use ratatui::layout::{Constraint, Direction, Layout, Rect};

/// 画面領域から各パネルの領域を計算する。
#[derive(Debug, Clone)]
pub struct AppLayout {
    pub left_sidebar: Rect,
    /// メイン全体（Kanban + Workflow の外枠）
    pub main: Rect,
    /// Kanban 領域（メイン上部）
    pub kanban: Rect,
    /// Workflow 領域（メイン下部）
    pub workflow: Rect,
    /// 本文領域（left+mainの親）
    pub body: Rect,
    /// 常設ステータスバー（画面最下端・高さ1）
    pub status_bar: Rect,
}

/// ステータスバーの高さ（行数）。
pub const STATUS_BAR_HEIGHT: u16 = 1;

/// 画面全体を分割して各パネルの領域を計算する。
///
/// * `left_pct` — 本文幅に対する左SBの幅（%）。5〜45% にクランプ
/// * `kanban_pct` — メイン高さに対する Kanban の高さ（%）。15〜70% にクランプ
pub fn compute_layout(terminal_size: Rect, left_pct: u16, kanban_pct: u16) -> AppLayout {
    // 水平分割: 比率で計算
    let left = left_pct.clamp(5, 45);
    let main_pct = 100u16.saturating_sub(left).max(10);

    // 垂直分割: 画面最下端に常設ステータスバー（高さ1）を確保する
    let body_split = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(1),
            Constraint::Length(STATUS_BAR_HEIGHT),
        ])
        .split(terminal_size);
    let body_area = body_split[0];
    let status_bar = body_split[1];

    let h_split = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(left),
            Constraint::Percentage(main_pct),
        ])
        .split(body_area);
    let main = h_split[1];

    // 垂直分割: メインを Kanban(上) + Workflow(下) に分ける
    let kanban_h = kanban_pct.clamp(15, 70);
    let workflow_h = 100u16.saturating_sub(kanban_h).max(10);
    let v_split = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(kanban_h),
            Constraint::Percentage(workflow_h),
        ])
        .split(main);

    AppLayout {
        left_sidebar: h_split[0],
        main,
        kanban: v_split[0],
        workflow: v_split[1],
        body: body_area,
        status_bar,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// レイアウトが左SB + メイン（Kanban + Workflow）の 3 分割になること。
    #[test]
    fn layout_is_sidebar_plus_split_main() {
        let area = Rect::new(0, 0, 100, 30);
        let layout = compute_layout(area, 25, 40);
        assert_eq!(
            layout.body.height + layout.status_bar.height,
            area.height,
            "body + status_bar で画面全体"
        );
        assert_eq!(layout.status_bar.height, 1, "ステータスバーは高さ1");
        assert_eq!(
            layout.status_bar.bottom(),
            area.bottom(),
            "ステータスバーは最下端"
        );
        assert_eq!(layout.left_sidebar.width, 25, "左SB は 25%");
        assert_eq!(layout.main.width, 75, "メインは残り 75%");
        assert_eq!(
            layout.left_sidebar.right(),
            layout.main.x,
            "左SB とメインが隙間なく並ぶ"
        );
        // メイン内部: Kanban(上) + Workflow(下)
        assert_eq!(layout.kanban.x, layout.main.x);
        assert_eq!(layout.workflow.x, layout.main.x);
        assert_eq!(layout.kanban.width, layout.main.width);
        assert_eq!(layout.workflow.width, layout.main.width);
        assert_eq!(layout.kanban.y, layout.main.y);
        assert_eq!(
            layout.kanban.bottom(),
            layout.workflow.y,
            "上下が隙間なく並ぶ"
        );
        assert_eq!(layout.workflow.bottom(), layout.main.bottom());
    }

    /// 左SB 幅は 5〜45% にクランプされること。
    #[test]
    fn layout_clamps_left_pct() {
        let area = Rect::new(0, 0, 100, 30);
        let small = compute_layout(area, 1, 40);
        assert_eq!(small.left_sidebar.width, 5, "下限 5%");
        let large = compute_layout(area, 90, 40);
        assert_eq!(large.left_sidebar.width, 45, "上限 45%");
    }

    /// Kanban 高さは 15〜70% にクランプされること。
    /// （画面最下端の常設ステータスバー 1 行を除いた本文高さが基準）
    #[test]
    fn layout_clamps_kanban_pct() {
        let area = Rect::new(0, 0, 100, 40);
        let small = compute_layout(area, 25, 1);
        // 本文高さ 40 - 1 = 39 の 15% = 6（端数切上げ）
        assert_eq!(small.kanban.height, 6, "下限 15%");
        let large = compute_layout(area, 25, 99);
        // 本文高さ 39 の 70% = 27
        assert_eq!(
            large.kanban.height,
            (39f64 * 0.70).floor() as u16,
            "上限 70%（本文高さ = 端末高さ - ステータスバー）"
        );
    }
}
