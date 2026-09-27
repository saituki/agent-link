//! グラフビューポート管理。
//!
//! スクロール位置と手動スクロールモードを保持する。

/// スクロール可能なグラフ表示のビューポート状態。
///
/// ズーム・スクロール（パン）の実際の描画・計算は `ui/main_panel.rs` が行うため、
/// この構造体はスクロール位置の状態保持のみを担う。
#[derive(Debug, Clone, Default)]
pub struct GraphViewport {
    /// 水平スクロール位置
    pub scroll_x: i32,
    /// 垂直スクロール位置
    pub scroll_y: i32,
    /// 手動スクロールモード（自動追従を無効化）
    pub manual_scroll: bool,
}

impl GraphViewport {
    /// 水平にスクロールする。
    pub fn scroll_horizontal(&mut self, delta: i32) {
        self.manual_scroll = true;
        self.scroll_x = (self.scroll_x + delta).max(0);
    }

    /// 垂直にスクロールする。
    pub fn scroll_vertical(&mut self, delta: i32) {
        self.manual_scroll = true;
        self.scroll_y = (self.scroll_y + delta).max(0);
    }
}
