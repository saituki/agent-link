//! model-tui: pi agent で使用可能な model / provider 一覧表示。
//!
//! `Shift+M` で遷移し、`Esc` で元の画面（Workflow）へ戻る。pi agent の
//! `Ctrl+L`（model selector）と同じ一覧を、参照専用（Enter 決定なし）で表示する。
//!
//! - 取得元: `~/.pi/agent/models.json`（`provider/model` の一覧）
//! - 文字入力でフィルタ（部分列一致・大文字小文字無視）
//! - `↑` / `↓` / `PageUp` / `PageDown` でカーソル移動
//!
//! ここで選択・決定する機能は持たない。ユーザーは一覧を確認し、`task.md` の
//! frontmatter に手動で `model` / `provider` / `thinking` を記載する。

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::widgets::Widget;

use crate::ui::draw::draw_text_clipped;

/// 一覧の 1 件（provider と model id）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelItem {
    /// provider 名（`[provider]` バッジ）
    pub provider: String,
    /// model id（`provider/` を除いた本体）
    pub model: String,
}

/// model-tui の状態（`AppState` に保持）。
#[derive(Debug, Clone, Default)]
pub struct ModelViewState {
    /// 全モデル（provider 順にソート済み）
    pub items: Vec<ModelItem>,
    /// 入力フィルタ文字列
    pub filter: String,
    /// フィルタ後リストのカーソル位置
    pub cursor: usize,
}

impl ModelViewState {
    /// `~/.pi/agent/models.json` から一覧を読み直す（`Shift+M` 押下時）。
    ///
    /// `provider/model` 形式を provider と model に分割する。provider 名が
    /// 無い（`/` 無し）項目は provider 空欄で表示する。
    pub fn load(&mut self) {
        let mut items: Vec<ModelItem> = sai_link_engine::common::load_available_models()
            .into_iter()
            .map(|full| match full.split_once('/') {
                Some((provider, model)) => ModelItem {
                    provider: provider.to_string(),
                    model: model.to_string(),
                },
                None => ModelItem {
                    provider: String::new(),
                    model: full,
                },
            })
            .collect();
        items.sort_by(|a, b| {
            a.provider
                .cmp(&b.provider)
                .then_with(|| a.model.cmp(&b.model))
        });
        self.items = items;
        self.filter.clear();
        self.cursor = 0;
    }

    /// フィルタ後の項目 index 一覧。
    pub fn filtered_indices(&self) -> Vec<usize> {
        if self.filter.is_empty() {
            return (0..self.items.len()).collect();
        }
        (0..self.items.len())
            .filter(|&i| {
                let item = &self.items[i];
                fuzzy_match(&self.filter, &format!("{}/{}", item.provider, item.model))
            })
            .collect()
    }

    /// カーソルを 1 つ上へ。先頭では末尾へ折り返す（pi と同じ）。
    pub fn cursor_up(&mut self) {
        let n = self.filtered_indices().len();
        if n == 0 {
            self.cursor = 0;
        } else {
            self.cursor = if self.cursor == 0 {
                n - 1
            } else {
                self.cursor - 1
            };
        }
    }

    /// カーソルを 1 つ下へ。末尾では先頭へ折り返す（pi と同じ）。
    pub fn cursor_down(&mut self) {
        let n = self.filtered_indices().len();
        if n == 0 {
            self.cursor = 0;
        } else {
            self.cursor = if self.cursor + 1 >= n { 0 } else { self.cursor + 1 };
        }
    }
}

/// 部分列一致（大文字小文字無視）。pi agent の fuzzyFilter 相当の簡易版。
///
/// `query` の文字が `target` に順番どおり現れれば `true`。
pub fn fuzzy_match(query: &str, target: &str) -> bool {
    let q: Vec<char> = query.to_lowercase().chars().collect();
    if q.is_empty() {
        return true;
    }
    let mut qi = 0usize;
    for tc in target.to_lowercase().chars() {
        if qi < q.len() && q[qi] == tc {
            qi += 1;
        }
    }
    qi == q.len()
}

/// 一覧の表示ウィジェット。
pub struct ModelListView<'a> {
    /// 表示状態
    pub state: &'a ModelViewState,
}

impl Widget for ModelListView<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.width == 0 || area.height == 0 {
            return;
        }
        let indices = self.state.filtered_indices();
        let total = self.state.items.len();
        let matched = indices.len();

        // タイトル行
        draw_text_clipped(
            buf,
            area,
            area.x,
            area.y,
            &format!(
                " model-tui — {} / {} models   (Esc: back) ",
                matched, total
            ),
            Style::default().fg(Color::Cyan),
        );
        // フィルタ行
        draw_text_clipped(
            buf,
            area,
            area.x,
            area.y + 1,
            &format!("filter: {}", self.state.filter),
            Style::default().fg(Color::Yellow),
        );
        if area.height < 3 {
            return;
        }

        let list_top = area.y + 2;
        let list_h = area.height.saturating_sub(3) as usize;
        if list_h == 0 {
            return;
        }
        if matched == 0 {
            draw_text_clipped(
                buf,
                area,
                area.x,
                list_top,
                "  No matching models",
                Style::default().fg(Color::Gray),
            );
            return;
        }
        // カーソルが見えるよう中央寄せでウィンドウを決める（pi と同じ挙動）。
        let cursor = self.state.cursor.min(matched - 1);
        let start = if matched <= list_h {
            0
        } else {
            cursor
                .saturating_sub(list_h / 2)
                .min(matched - list_h)
        };
        for (row, &idx) in indices.iter().skip(start).take(list_h).enumerate() {
            let item = &self.state.items[idx];
            let selected = start + row == cursor;
            let marker = if selected { "→ " } else { "  " };
            let line = format!("{}{} [{}]", marker, item.model, item.provider);
            let style = if selected {
                Style::default().fg(Color::Black).bg(Color::Cyan)
            } else {
                Style::default().fg(Color::White)
            };
            let y = list_top + row as u16;
            // 背景ハイライトをウィンドウ右端まで伸ばす
            if selected {
                for x in area.x..area.x + area.width {
                    if let Some(c) = buf.cell_mut((x, y)) {
                        c.set_style(Style::default().bg(Color::Cyan));
                    }
                }
            }
            draw_text_clipped(buf, area, area.x, y, &line, style);
        }
        // スクロール位置（末尾表示）
        if matched > list_h {
            let info = format!("  ({}/{})", cursor + 1, matched);
            let y = area.y + area.height - 1;
            // info は ASCII のみのため文字数 = 表示幅
            let w = info.chars().count() as u16;
            let x = area.x + area.width.saturating_sub(w);
            draw_text_clipped(buf, area, x, y, &info, Style::default().fg(Color::Gray));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fuzzy_match_subsequence() {
        assert!(fuzzy_match("", "anything"));
        assert!(fuzzy_match("dsf", "deepseek/deepseek-v4-flash"));
        assert!(fuzzy_match("DEEP", "deepseek/deepseek-v4-flash"));
        assert!(!fuzzy_match("zzz", "deepseek/deepseek-v4-flash"));
        // 順序が違うと一致しない
        assert!(!fuzzy_match("fsd", "deepseek"));
    }

    #[test]
    fn cursor_wraps() {
        let mut s = ModelViewState {
            items: vec![
                ModelItem { provider: "a".into(), model: "m1".into() },
                ModelItem { provider: "a".into(), model: "m2".into() },
            ],
            filter: String::new(),
            cursor: 0,
        };
        s.cursor_up();
        assert_eq!(s.cursor, 1);
        s.cursor_down();
        assert_eq!(s.cursor, 0);
        s.cursor_down();
        assert_eq!(s.cursor, 1);
    }

    #[test]
    fn render_does_not_panic() {
        use ratatui::buffer::Buffer;
        use ratatui::layout::Rect;
        use ratatui::widgets::Widget;
        let state = ModelViewState {
            items: vec![
                ModelItem { provider: "openrouter".into(), model: "deepseek-v4".into() },
                ModelItem { provider: "local".into(), model: "qwen".into() },
            ],
            filter: "deep".into(),
            cursor: 0,
        };
        let area = Rect::new(0, 0, 60, 12);
        let mut buf = Buffer::empty(area);
        ModelListView { state: &state }.render(area, &mut buf);
        // 空領域でもパニックしない
        let empty = ModelViewState::default();
        let small = Rect::new(0, 0, 4, 2);
        let mut buf2 = Buffer::empty(small);
        ModelListView { state: &empty }.render(small, &mut buf2);
    }

    #[test]
    fn filter_indices_respect_query() {
        let s = ModelViewState {
            items: vec![
                ModelItem { provider: "openrouter".into(), model: "deepseek-v4".into() },
                ModelItem { provider: "anthropic".into(), model: "claude".into() },
            ],
            filter: "deep".into(),
            cursor: 0,
        };
        assert_eq!(s.filtered_indices(), vec![0]);
    }
}
