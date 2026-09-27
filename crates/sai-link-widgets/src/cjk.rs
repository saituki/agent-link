//! CJK 文字幅対応の描画ヘルパー。
//!
//! 全角文字（日本語など）を正しく表示するためのユーティリティ。
//! ratatui の `buf.set_string()` が内部的に `unicode-width` を使うため、
//! 単一スタイルのテキスト描画は `buf.set_string()` に任せる。
//! スタイル混在が必要な場合のみ本モジュールの関数を使用する。

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use unicode_width::UnicodeWidthChar;

/// 1 文字を描画し、実際に消費したセル幅（全角=2, 半角=1）を返す。
/// 全角文字の場合は右隣セルに同じスタイル（背景色）を適用する。
///
/// # 戻り値
/// 消費した水平セル数（1 または 2）。`width==0` の結合文字の場合は 0。
pub fn draw_char_cjk(buf: &mut Buffer, x: u16, y: u16, ch: char, style: Style) -> u16 {
    let width = ch.width().unwrap_or(1) as u16;
    if width == 0 {
        return 0; // 結合文字はスキップ
    }
    buf.cell_mut((x, y))
        .map(|c| c.set_char(ch).set_style(style));
    if width == 2 {
        // 全角文字の右隣セルに背景色のみ適用
        buf.cell_mut((x + 1, y)).map(|c| c.set_style(style));
    }
    width
}

/// CJK 対応のテキスト描画。`buf.set_string()` のラッパーだが、
/// 明示的に全角対応であることを示すために用意。
/// 単一スタイルのテキストはこちらを使用する。
pub fn draw_text_cjk(buf: &mut Buffer, x: u16, y: u16, text: &str, style: Style) {
    buf.set_string(x, y, text, style);
}

/// 指定矩形内に収まるようクリップしながらテキストを描画する。
///
/// サイドバーの幅が狭いとき等に、テキストが隣のパネルへはみ出すのを防ぐ。
/// 全角文字が右端で途中までしか入らない場合は、その文字を描画しない。
pub fn draw_text_clipped(buf: &mut Buffer, area: Rect, x: u16, y: u16, text: &str, style: Style) {
    // 行が領域の上下の外なら何もしない
    if y < area.top() || y >= area.bottom() || x >= area.right() {
        return;
    }
    let mut cx = x;
    for ch in text.chars() {
        let width = ch.width().unwrap_or(1) as u16;
        if width == 0 {
            continue;
        }
        // 右端を超える文字は以降すべて描画しない
        if cx + width > area.right() {
            break;
        }
        draw_char_cjk(buf, cx, y, ch, style);
        cx += width;
    }
}
