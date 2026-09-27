//! 左サイドバー（ブロックツリー）。
//!
//! ブロックを時系列ツリーで表示します。
//! ユーザー起動ブロックをルートとし、サブタスクを入れ子表示します。

use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Style},
    widgets::Widget,
};

use super::draw;
use sai_link_widgets::theme::AppTheme;

/// ツリーアイテムの種別。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TreeItemKind {
    /// グローバルラウンド（R） / ローカルラウンド（r）のフォルダ。
    /// カーソル移動 + Enter（またはクリック）で折りたたみ/展開できる。
    Folder,
    /// 実行（`ID<run_id>/<block_name>` リーフ）。
    /// Enter で右SB と連動する。
    Leaf,
    /// Kanban カード（`.sai/todo/*.md` のリーフ）。
    /// `c` で開く・`d` で Workflow へ移動（カード md 削除）。
    /// 実行リーフと区別するため `card_path` が必ず `Some`。
    Card,
}

/// ツリーアイテム1件。
#[derive(Debug, Clone)]
pub struct TreeItem {
    /// ブロック ID（またはタスク ID）
    pub id: i64,
    /// 表示名
    pub name: String,
    /// インデントレベル（0 = ルート）
    pub depth: usize,
    /// 展開中か
    pub expanded: bool,
    /// 最終ノードか（ツリー描画用）
    pub is_last_child: bool,
    /// アイテム種別（フォルダ / リーフ）
    pub kind: TreeItemKind,
    /// 各祖先が「最後の子」かどうか（ツリー接続線の描画用）
    /// 例: depth=2 のリーフは `[親rフォルダが最後の子か]` を保持する
    pub ancestor_last: Vec<bool>,
    /// Kanban カードのパス（`TreeItemKind::Card` のみ `Some`）。
    ///
    /// 実行リーフは `None`。`c` / `d` はこの値がある項目にのみ有効。
    pub card_path: Option<std::path::PathBuf>,
}

impl TreeItem {
    /// フォルダ（折りたたみ/展開可能）かどうか。
    pub fn is_folder(&self) -> bool {
        matches!(self.kind, TreeItemKind::Folder)
    }
}

/// 左サイドバーの状態。
///
/// フィルタ機能は廃止された（文字キーをショートカットに譲るため）。
#[derive(Debug, Clone, Default)]
pub struct LeftSidebarState {
    /// ツリーアイテム一覧（可視ツリー）
    pub tree_items: Vec<TreeItem>,
    /// 全アイテム（展開状態の保持用）
    pub all_items: Vec<TreeItem>,
    /// 選択中インデックス
    pub selected_index: usize,
    /// スクロールオフセット
    pub scroll_offset: usize,
}

/// 左サイドバーウィジェット。
pub struct LeftSidebar<'a> {
    pub state: &'a LeftSidebarState,
    pub theme: &'a AppTheme,
}

impl Widget for LeftSidebar<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let bg = self.theme.sidebar_bg;
        let fg = self.theme.sidebar_fg;
        let border = self.theme.border;
        let selected_bg = self.theme.info;

        // 背景
        for y in area.top()..area.bottom() {
            for x in area.left()..area.right() {
                if let Some(cell) = buf.cell_mut((x, y)) {
                    cell.set_style(Style::default().bg(bg).fg(fg));
                }
            }
        }

        // 上部ボーダー
        draw_horizontal_line(buf, area, area.y, '─', Style::default().fg(border));

        // ツリーアイテム（フィルタ欄は廃止されたためボーダーの直下から）
        let visible_height = area.height.saturating_sub(1) as usize;
        let start = self
            .state
            .scroll_offset
            .min(self.state.tree_items.len().saturating_sub(1));
        let end = (start + visible_height).min(self.state.tree_items.len());
        let end = end.max(start); // 範囲反転防止

        for (i, item) in self.state.tree_items[start..end].iter().enumerate() {
            let row = area.y + 1 + i as u16;
            if row >= area.bottom() {
                break;
            }

            let is_selected = (start + i) == self.state.selected_index;
            // カーソル選択中 → 黄色、それ以外 → 白色。
            // 旧 Notification 色分け（Reflection/Review 記入状況）は廃止。
            let item_style = if is_selected {
                Style::default().fg(Color::Yellow).bg(selected_bg)
            } else {
                Style::default().fg(Color::White).bg(bg)
            };

            // インデント + ツリー線
            let prefix = build_tree_prefix(item.depth, item.is_last_child, &item.ancestor_last);
            let text = format!("{}{}", prefix, item.name);

            // 幅が狭い場合に隣のパネルへはみ出さないようクリップして描画
            draw::draw_text_clipped(buf, area, area.x + 1, row, &text, item_style);
        }
    }
}

// ============================================================================
// ヘルパー
// ============================================================================

/// ツリー接続線のプレフィックスを構築する。
///
/// ファイルエクスプローラ風の階層表示を行う。
/// - depth 0（R フォルダ）: 接続線なし
/// - 祖先レベルごとに「│ 」（継続）または「  」（終端）を並べる
/// - 自分自身は「├─」または「└─」で分岐を表現する
fn build_tree_prefix(depth: usize, is_last: bool, ancestor_last: &[bool]) -> String {
    if depth == 0 {
        return String::new();
    }

    let mut prefix = String::new();
    // 上位の階層の接続線（各祖先が最後の子なら「  」、続くなら「│ 」）
    for &last in ancestor_last.iter().take(depth - 1) {
        if last {
            prefix.push_str("  ");
        } else {
            prefix.push_str("│ ");
        }
    }

    // 最終ノードの分岐
    if is_last {
        prefix.push_str("└─");
    } else {
        prefix.push_str("├─");
    }

    prefix
}

fn draw_horizontal_line(buf: &mut Buffer, area: Rect, y: u16, ch: char, style: Style) {
    for x in area.left()..area.right() {
        if let Some(cell) = buf.cell_mut((x, y)) {
            cell.set_char(ch).set_style(style);
        }
    }
}

// ============================================================================
// テスト
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::buffer::Buffer as TuiBuffer;

    /// 幅が狭い左SBでは、ツリーのテキストが隣のパネルへはみ出さないこと。
    #[test]
    fn sidebar_text_is_clipped_to_area() {
        let theme = AppTheme::dark();
        let mut state = LeftSidebarState::default();
        state.tree_items.push(TreeItem {
            id: 1,
            name: "very-long-block-name-that-overflows".into(),
            depth: 0,
            expanded: false,
            is_last_child: false,
            kind: TreeItemKind::Leaf,
            ancestor_last: Vec::new(),
            card_path: None,
        });

        // 左SB: 幅 12。全体バッファ: 幅 40（右側にメイン画面がある想定）
        let area = Rect::new(0, 0, 12, 10);
        let mut buf = TuiBuffer::empty(Rect::new(0, 0, 40, 10));
        LeftSidebar {
            state: &state,
            theme: &theme,
        }
        .render(area, &mut buf);

        // 左SBの外（x >= 12）に文字が描画されていないこと
        for x in area.right()..40 {
            for y in 0..10 {
                assert_eq!(
                    buf[(x, y)].symbol(),
                    " ",
                    "x={} y={} に左SBのテキストがはみ出している",
                    x,
                    y
                );
            }
        }
    }

    /// ツリーの接続線プレフィックスが階層に応じて正しく構築されること。
    #[test]
    fn tree_prefix_reflects_hierarchy() {
        // depth 0: 接続線なし
        assert_eq!(build_tree_prefix(0, true, &[]), "");
        // depth 1: 自分の分岐のみ（├─ / └─）
        assert_eq!(build_tree_prefix(1, false, &[]), "├─");
        assert_eq!(build_tree_prefix(1, true, &[]), "└─");
        // depth 2: 祖先（親 r フォルダ）が最後の子でないなら「│ 」+ 分岐
        assert_eq!(build_tree_prefix(2, false, &[false]), "│ ├─");
        assert_eq!(build_tree_prefix(2, true, &[false]), "│ └─");
        // depth 2: 祖先が最後の子なら「  」+ 分岐
        assert_eq!(build_tree_prefix(2, true, &[true]), "  └─");
    }
}
