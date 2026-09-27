//! ブロック選択モーダル。
//!
//! task.md 発行時に実行ブロックを選択する UI。
//! `.sai/blocks/*.md` と `~/.sai-moon/blocks/*.md` から
//! frontmatter の id / name / description を読む。

use papr_core::BlockInfo;

/// ブロック選択モーダルの状態。
#[derive(Debug, Clone, Default)]
pub struct BlockPickerState {
    /// 利用可能なブロック一覧。
    pub blocks: Vec<BlockInfo>,
    /// 現在選択されているブロックのインデックス。
    pub selected: usize,
    /// スクロール位置。
    pub scroll: usize,
}

impl BlockPickerState {
    /// 新規初期化。
    #[must_use]
    pub fn new(blocks: Vec<BlockInfo>) -> Self {
        Self {
            blocks,
            selected: 0,
            scroll: 0,
        }
    }

    /// 現在選択されているブロックを取得する。
    pub fn selected_block(&self) -> Option<&BlockInfo> {
        self.blocks.get(self.selected)
    }

    /// 上へ移動。
    pub fn move_up(&mut self) {
        if self.selected > 0 {
            self.selected -= 1;
        }
        self.update_scroll();
    }

    /// 下へ移動。
    pub fn move_down(&mut self) {
        if self.selected < self.blocks.len().saturating_sub(1) {
            self.selected += 1;
        }
        self.update_scroll();
    }

    /// スクロール位置を選択位置に合わせて更新。
    fn update_scroll(&mut self) {
        const VISIBLE_ROWS: usize = 10;
        if self.selected < self.scroll {
            self.scroll = self.selected;
        } else if self.selected >= self.scroll + VISIBLE_ROWS {
            self.scroll = self.selected.saturating_sub(VISIBLE_ROWS - 1);
        }
    }
}
