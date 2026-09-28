//! モーダルダイアログシステム。
//!
//! ヘルプ表示などのオーバーレイ UI を提供します。
//! フォーム系機能（block run の対話フロー）は sai CLI に移行されました。
//! 実行は `sai block run --interactive` を使用してください。

use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
    widgets::{Block, BorderType, Borders, Widget},
};

use super::draw;

/// 選択中の行のハイライト背景（カードの選択強調と統一）。
const SELECTED_ROW_BG: Color = Color::Rgb(40, 40, 20);

/// ダイアログ枠を ratatui の `Block`（角丸ボーダー）で描く。
///
/// オフスクリーンバッファに描いてから本体バッファへ貼る（ekphos と同じやり方）。
/// `bg` は枠内部の背景色（呼び出し側が事前に画面全体を暗くしている場合はそれと揃える）。
pub(crate) fn draw_dialog_frame(buf: &mut Buffer, rect: Rect, title: &str, border_color: Color, bg: Color) {
    let mut off = Buffer::empty(rect);
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(border_color).bg(bg))
        .title(title.to_string())
        .title_style(Style::default().fg(border_color).bg(bg))
        .style(Style::default().bg(bg))
        .render(rect, &mut off);
    for y in rect.y..rect.y + rect.height {
        for x in rect.x..rect.x + rect.width {
            if let Some(src) = off.cell((x, y)) {
                if let Some(dst) = buf.cell_mut((x, y)) {
                    *dst = src.clone();
                }
            }
        }
    }
}

// ============================================================================
// モーダル状態
// ============================================================================

/// waggle token 選択の 1 項目（`w` キー）。
#[derive(Debug, Clone)]
pub struct WaggleChoice {
    /// waggle トークン文字列
    pub token: String,
    /// 元ファイル名（basename）
    pub file_name: String,
    /// 出力の説明（マニフェストの description）
    pub description: String,
    /// 出力元ブロック名
    pub block_name: String,
    /// 実行状態（running / completed 等・U03。空は従来表示）
    #[allow(dead_code)]
    pub status: String,
}

/// モーダルダイアログの種類（全画面オーバーレイ）。
#[derive(Debug, Clone)]
pub enum Modal {
    /// 非表示
    None,
    /// ヘルプ（全画面）
    Help,
    /// カード新規作成ウィザード（`Shift+C`・段階入力フォーム）
    CardCreate(crate::app::card_wizard::CardWizardState),
    /// waggle token 選択（`w`・md トークンのみ）
    WaggleSelect {
        /// 選択肢一覧
        choices: Vec<WaggleChoice>,
        /// カーソル位置
        cursor: usize,
    },
}

// ============================================================================
// モーダルレンダリング
// ============================================================================

/// 画面中央にモーダルを描画する（Help / CardCreate）。
///
/// `workflow_bg` は Workflow パネル（`MainPanel`）と同じ背景色（`AppTheme::main_bg`）。
/// `WaggleSelect`（`w`）はこの背景色を使い、枠線なしで描画する（他モーダルは従来どおり）。
pub fn render_modal(modal: &Modal, area: Rect, buf: &mut Buffer, workflow_bg: Color) {
    match modal {
        Modal::None => {}
        Modal::Help => {
            // waggle 選択と同じく背景は変更せず、画面中央のダイアログだけを描く。
            render_help(area, buf, workflow_bg);
        }
        Modal::CardCreate(wizard) => {
            crate::app::card_wizard::render_card_wizard(wizard, area, buf, workflow_bg);
        }
        Modal::WaggleSelect { choices, cursor } => {
            render_waggle_select(area, buf, choices, *cursor, workflow_bg);
        }
    }
}

/// waggle token 選択モーダルを描画する。
///
/// `↑/↓` で移動・`Enter` で確定・`Esc` で閉じる。
/// 背景= Workflow パネルと同色、枠線なし、文字は白（選択行のみ黄色）。
fn render_waggle_select(
    area: Rect,
    buf: &mut Buffer,
    choices: &[WaggleChoice],
    cursor: usize,
    bg: Color,
) {
    use super::draw::draw_text_clipped;
    let w = 90u16.min(area.width.saturating_sub(4));
    let h = (choices.len() as u16 + 6).clamp(8, area.height.saturating_sub(4));
    let x = area.x + area.width.saturating_sub(w) / 2;
    let y = area.y + area.height.saturating_sub(h) / 2;
    let dlg = Rect::new(x, y, w, h);
    draw_dialog_frame(buf, dlg, " waggle tokens ", Color::Magenta, bg);
    let style_title = Style::default().fg(Color::White);
    draw_text_clipped(
        buf,
        dlg,
        dlg.x + 2,
        dlg.y + 1,
        "Up/Down=move Enter=open Esc=close",
        style_title,
    );
    // 各項目の開始位置を揃える（token / status / file_name / description の固定幅・U03）。
    let token_w = choices.iter().map(|c| c.token.len()).max().unwrap_or(0);
    let status_w = choices
        .iter()
        .map(|c| c.status.len())
        .max()
        .unwrap_or(0)
        .max(4);
    let file_w = choices.iter().map(|c| c.file_name.len()).max().unwrap_or(0);
    for (i, c) in choices.iter().enumerate().take(h as usize - 4) {
        let cur = if i == cursor { "> " } else { "  " };
        let status_label = if c.status.is_empty() {
            " ".repeat(status_w)
        } else {
            format!("[{}]", c.status)
        };
        let line = format!(
            "{}{:<token_w$} {:<status_w$} {:<file_w$} {}",
            cur,
            c.token,
            status_label,
            c.file_name,
            c.description,
            token_w = token_w,
            status_w = status_w + 2,
            file_w = file_w,
        );
        let style = if i == cursor {
            Style::default()
                .fg(Color::Yellow)
                .bg(SELECTED_ROW_BG)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::White)
        };
        draw_text_clipped(buf, dlg, dlg.x + 2, dlg.y + 2 + i as u16, &line, style);
    }
}

/// カード新規作成モーダル（Done レーン実行の複数選択リスト）を描画する。
///
/// `↑/↓` で移動・`Space` で選択切替・`Enter` で確定・`Esc` で閉じる。
/// 選択なし Enter でも確定できる（連結なしの空カードを作る）。
/// 描画のみ（選択状態の保持は `AppState.modal` 側）。
fn render_card_create(
    area: Rect,
    buf: &mut Buffer,
    options: &[i64],
    cursor: usize,
    checked: &[bool],
) {
    use super::draw::draw_text_clipped;
    let w = 60u16.min(area.width.saturating_sub(4));
    let h = (options.len() as u16 + 6).clamp(8, area.height.saturating_sub(4));
    let x = area.x + area.width.saturating_sub(w) / 2;
    let y = area.y + area.height.saturating_sub(h) / 2;
    let dlg = Rect::new(x, y, w, h);
    draw_dialog_frame(buf, dlg, " カード作成 ", Color::White, Color::Black);
    let style_title = ratatui::style::Style::default().fg(Color::Cyan);
    draw_text_clipped(
        buf,
        dlg,
        dlg.x + 2,
        dlg.y + 1,
        "Space=選択/解除 Enter=確定[未選択=空カード] Esc=閉じる",
        style_title,
    );
    if options.is_empty() {
        draw_text_clipped(
            buf,
            dlg,
            dlg.x + 2,
            dlg.y + 2,
            "done レーンなし: Enter で空カードを作る",
            ratatui::style::Style::default().fg(Color::White),
        );
        return;
    }
    for (i, id) in options.iter().enumerate().take(h as usize - 4) {
        let mark = if checked.get(i).copied().unwrap_or(false) {
            "[x]"
        } else {
            "[ ]"
        };
        let cur = if i == cursor { "> " } else { "  " };
        let line = format!("{} {} ID{}", cur, mark, id);
        let style = if i == cursor {
            ratatui::style::Style::default()
                .fg(Color::Yellow)
                .bg(SELECTED_ROW_BG)
                .add_modifier(Modifier::BOLD)
        } else {
            ratatui::style::Style::default().fg(Color::White)
        };
        draw_text_clipped(buf, dlg, dlg.x + 2, dlg.y + 2 + i as u16, &line, style);
    }
}

// ============================================================================
// ヘルプ描画
// ============================================================================

fn render_help(area: Rect, buf: &mut Buffer, bg: Color) {
    // 全ショートカット一覧（現在の全操作を網羅）。5 グループ構成:
    //   1. どこででも効くキー / 2. 画面の切り替え / 3. ファイルを開く操作 /
    //   4. サイドバーの操作 / 5. メイン画面での操作
    // 行数が増えたため、画面の大きさに応じて表示範囲を調整する
    // （端末が小さいときは見える範囲だけを表示する）。
    let lines: Vec<(String, Color)> = {
        let mut v = Vec::new();
        let section = |title: &str, items: &[(&str, &str)], v: &mut Vec<(String, Color)>| {
            v.push((format!("■ {}", title), Color::Cyan));
            for (k, d) in items {
                v.push((format!("  {:<18} = {}", k, d), Color::White));
            }
            v.push((String::new(), Color::White));
        };
        section(
            "どこででも効くキー",
            &[
                ("Shift+Q", "アプリの終了"),
                ("Shift+R", "リロード（実行状況を取り込み直す）"),
                ("Shift+H", "ヘルプ（この画面）。何かキーで閉じる"),
                ("Shift+M", "model-tui（pi の使用可能モデル一覧）を表示（Esc で戻る）"),
                ("Shift+F", "検索画面へ遷移（検索画面の操作は下記）"),
                ("Shift+A", "Summary（プロジェクト全体のメモ）を開く"),
                ("Shift+B", ".sai/blocks ディレクトリを nvim で開く"),
                ("Shift+S", ".sai/settings.json を nvim で開く"),
                ("Tab", "画面フォーカス切替（sidebar → kanban → workflow）。スクロールはフォーカス中の画面のみ"),
                ("Shift+C", "カード新規作成（Done レーンのみ・未選択 Enter=連結なし空カード）"),
                ("x", "run-tui（block run / block rm）。↑↓/jk=選択 Enter=確定 Esc=閉じる。レーン選択中は e でレーン全件を1段階移動"),
                ("w", "waggle token を選択して md-tui で開く（md のみ）"),
                ("p", "pi セッションを再開"),
                ("c", "sidebar 選択中のカードを nvim で開く"),
                ("Space t", "task.md を md-tui に表示"),
            ],
            &mut v,
        );
        section(
            "画面の切り替え（Workflow からの片道遷移・戻りは Esc）",
            &[
                ("s", "Stats（キャッシュ統計）へ遷移"),
                ("l", "Log（実行詳細）へ遷移"),
                ("h", "History（セッション履歴）へ遷移"),
                ("Space m", "message（user/assistant のみ抽出・保存・表示）へ遷移"),
                ("Shift+P", "Project（プロジェクト概要）へ遷移"),
                ("Esc", "各画面から Workflow に戻る（唯一の戻り手段）"),
            ],
            &mut v,
        );
        section(
            "Workflow：サイドバー（sidebar 有効時のみ）",
            &[
                ("j / k", "カーソル移動（選択追従）"),
                (
                    "Enter",
                    "フォルダの展開 / 折りたたみ（リーフでは何もしない）",
                ),
                ("ホイール", "カーソル移動と同等（選択追従）"),
                ("クリック", "フォルダは展開/折りたたみ・リーフは選択"),
            ],
            &mut v,
        );
        section(
            "Workflow：メイン（DAG）",
            &[
                ("↑↓←→", "パン"),
                ("+ / -", "ズーム（ビューポート中央基準）"),
                ("ドラッグ", "パン"),
                ("Ctrl+ホイール", "ズーム"),
            ],
            &mut v,
        );
        section(
            "各画面での操作（j/k・矢印・マウス対応）",
            &[
                ("Log", "j/k・↑↓・PgUp/PgDn・ホイールでスクロール"),
                ("History", "j/k・↑↓・PgUp/PgDn・ホイールで Timeline スクロール"),
                ("Stats", "j/k・↑↓・PgUp/PgDn・Home/End・ホイールで Per-message スクロール、1/2/3/4 で表示切替"),
                ("Project", "j/k・↑↓で Recent スクロール、PgUp/PgDn・ホイールで全体スクロール"),
                (
                    "message/task",
                    "j/k・↑↓=カーソル行移動 space/z=カーソル位置の見出しを折りたたみ PgUp/PgDn=ページ移動 Esc=閉じる",
                ),
            ],
            &mut v,
        );
        section(
            "検索画面（Shift+F 遷移後）",
            &[
                ("Tab / BackTab", "フォーム項目間の移動（循環）"),
                ("↑↓ / j/k", "フォーム項目 ⇄ 結果リストの移動"),
                (
                    "Enter",
                    "編集中は確定、フォーム上は編集開始、結果リスト上は検索実行",
                ),
                ("文字 / Backspace", "編集中のみ入力"),
                ("Esc", "編集中はキャンセル、非編集中は Workflow に戻る"),
            ],
            &mut v,
        );
        section(
            "カード新規作成モーダル（Shift+C）",
            &[
                ("↑ / ↓", "移動"),
                ("Space", "選択切替"),
                ("Enter", "確定（未選択=連結なし空カード）"),
                ("Esc", "閉じる"),
            ],
            &mut v,
        );
        v
    };

    // 最長行の表示幅を計算し、ダイアログ幅を動的に決定
    let max_line_width = lines.iter().map(|(text, _)| text_display_width(text)).max().unwrap_or(0);
    let min_dlg_width = 50u16; // 最小ダイアログ幅
    let max_dlg_width = area.width.saturating_sub(4);
    let base_width = (max_line_width as u16 + 4).max(min_dlg_width); // 両側の余白 +4
    let dlg_width = base_width.min(max_dlg_width);

    // ダイアログ内のテキスト描画エリアの幅（両側 +2 の余白を引く）
    let content_width = dlg_width.saturating_sub(4) as usize;

    // 長すぎる行を折り返す
    let wrapped_lines: Vec<(String, Color)> = lines
        .into_iter()
        .flat_map(|(text, color)| {
            if text_display_width(&text) as usize > content_width {
                // 折り返しが必要
                wrap_text_for_help(&text, content_width)
                    .into_iter()
                    .map(|line| (line, color))
                    .collect::<Vec<_>>()
            } else {
                vec![(text, color)]
            }
        })
        .collect();

    let content_height = wrapped_lines.len() as u16 + 4; // 本文 + 枠 + フッタ
    let dlg_height = content_height.min(area.height.saturating_sub(2)).max(6);
    let dx = area.x + (area.width.saturating_sub(dlg_width)) / 2;
    let dy = area.y + (area.height.saturating_sub(dlg_height)) / 2;

    let dlg_rect = Rect::new(dx, dy, dlg_width, dlg_height);
    draw_dialog_frame(buf, dlg_rect, " ヘルプ ", Color::Cyan, bg);

    let x = dx + 2;
    let mut y = dy + 2;
    let body_bottom = dy + dlg_height.saturating_sub(3);

    for (text, color) in &wrapped_lines {
        if y >= body_bottom {
            break; // 端末が小さいときは見える範囲だけを表示する
        }
        if !text.is_empty() {
            draw_text(buf, x, y, *color, text);
        }
        y += 1;
    }

    let footer_y = dy + dlg_height - 2;
    draw_text(buf, dx + 2, footer_y, Color::Gray, "[任意のキーで閉じる]");
}

// ============================================================================
// 描画ヘルパー
// ============================================================================

/// テキストを描画する（CJK 対応、背景色なし＝透過）。
fn draw_text(buf: &mut Buffer, x: u16, y: u16, color: Color, text: &str) {
    draw::draw_text_cjk(buf, x, y, text, Style::default().fg(color));
}

/// CJK文字を考慮した表示幅を計算する（全角文字は幅2、半角文字は幅1）。
fn text_display_width(text: &str) -> usize {
    use unicode_width::UnicodeWidthChar;
    text.chars()
        .map(|ch| ch.width().unwrap_or(1).max(1))
        .sum()
}

/// テキストを指定幅で折り返す（CJK対応）。
/// 各行は content_width 以下の表示幅に収まるよう分割される。
fn wrap_text_for_help(text: &str, content_width: usize) -> Vec<String> {
    use unicode_width::UnicodeWidthChar;
    if content_width == 0 {
        return vec![text.to_string()];
    }
    let mut result = Vec::new();
    let mut current_line = String::new();
    let mut current_width = 0usize;

    for ch in text.chars() {
        let ch_width = ch.width().unwrap_or(1).max(1);
        if current_width + ch_width > content_width && !current_line.is_empty() {
            // 現在の行を確定し、新しい行を開始
            result.push(current_line);
            current_line = String::new();
            current_width = 0;
        }
        current_line.push(ch);
        current_width += ch_width;
    }
    if !current_line.is_empty() {
        result.push(current_line);
    }
    result
}

/// 画面全体を完全な背景色で塗りつぶす（メイン画面を隠す）。
fn fill_rect(buf: &mut Buffer, area: Rect, color: Color, _alpha: f32) {
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            if let Some(cell) = buf.cell_mut((x, y)) {
                cell.set_char(' ').set_style(Style::default().bg(color));
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

    /// waggle token 選択モーダルの描画がパニックしないこと。
    #[test]
    fn render_waggle_select_does_not_panic() {
        let area = Rect::new(0, 0, 80, 24);
        let mut buf = Buffer::empty(area);
        let modal = Modal::WaggleSelect {
            choices: vec![
                WaggleChoice {
                    token: "AbCdEfGh".to_string(),
                    file_name: "memo.md".to_string(),
                    description: "メモ".to_string(),
                    block_name: "user".to_string(),
                    status: "running".to_string(),
                },
                WaggleChoice {
                    token: "IjKlMnOp".to_string(),
                    file_name: "report.md".to_string(),
                    description: String::new(),
                    block_name: "sai".to_string(),
                    status: "completed".to_string(),
                },
            ],
            cursor: 1,
        };
        render_modal(&modal, area, &mut buf, Color::Rgb(18, 18, 24));

        // 空の選択肢でもパニックしない
        let empty = Modal::WaggleSelect {
            choices: Vec::new(),
            cursor: 0,
        };
        render_modal(&empty, area, &mut buf, Color::Rgb(18, 18, 24));
    }
}
