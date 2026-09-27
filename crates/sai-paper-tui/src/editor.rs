//! TUI テキスト編集用の文字境界・単語境界ユーティリティ。
//!
//! 設定モーダル・ノートエディタ・ターミナルコマンド入力など、
//! カーソルを文字単位 / 単語単位で移動させる箇所で共用する。
//!
//! 備考: かつては config editor 用の折返し幾何関数群
//! （`config_editor_wrap_rows` / `cursor_visual_position` /
//! `expand_tabs_for_editor_view` 等）もここにあったが、
//! config editor 機能廃止に伴い削除した。

/// `cursor` の直前の文字境界位置を返す。
#[must_use]
pub fn prev_char_boundary(text: &str, cursor: usize) -> usize {
    if cursor == 0 {
        return 0;
    }
    let mut prev = cursor - 1;
    while prev > 0 && !text.is_char_boundary(prev) {
        prev -= 1;
    }
    prev
}

/// `cursor` の直後の文字境界位置を返す。
#[must_use]
pub fn next_char_boundary(text: &str, cursor: usize) -> usize {
    if cursor >= text.len() {
        return text.len();
    }
    let mut next = cursor + 1;
    while next < text.len() && !text.is_char_boundary(next) {
        next += 1;
    }
    next.min(text.len())
}

/// `cursor` の直前の単語境界位置を返す。
#[must_use]
pub fn prev_word_boundary(text: &str, cursor: usize) -> usize {
    if cursor == 0 {
        return 0;
    }
    let mut pos = cursor.min(text.len());

    // まず左側の空白 / 改行をスキップする
    while pos > 0 {
        let prev = prev_char_boundary(text, pos);
        let ch = text[prev..pos].chars().next().unwrap_or('\n');
        if !ch.is_whitespace() {
            break;
        }
        pos = prev;
    }

    if pos == 0 {
        return 0;
    }

    // 現在位置の文字種（単語構成文字かどうか）を判定する
    let prev = prev_char_boundary(text, pos);
    let first_ch = text[prev..pos].chars().next().unwrap_or('\n');
    let is_word_char = first_ch.is_alphanumeric() || first_ch == '_';

    // 同じ文字種の間をスキップする
    while pos > 0 {
        let prev = prev_char_boundary(text, pos);
        let ch = text[prev..pos].chars().next().unwrap_or('\n');
        if ch.is_whitespace() {
            break;
        }
        let ch_is_word = ch.is_alphanumeric() || ch == '_';
        if ch_is_word != is_word_char {
            break;
        }
        pos = prev;
    }

    pos
}

/// `cursor` の直後の単語境界位置を返す。
#[must_use]
pub fn next_word_boundary(text: &str, cursor: usize) -> usize {
    let mut pos = cursor.min(text.len());
    if pos >= text.len() {
        return text.len();
    }

    // カーソル位置の文字種を判定する
    let first_ch = text[pos..].chars().next().unwrap_or('\n');

    if first_ch.is_whitespace() {
        // 空白 / 改行をスキップする
        while pos < text.len() {
            let next = next_char_boundary(text, pos);
            let ch = text[pos..next].chars().next().unwrap_or('\n');
            if !ch.is_whitespace() {
                break;
            }
            pos = next;
        }
    } else {
        let is_word_char = first_ch.is_alphanumeric() || first_ch == '_';
        // 同じ文字種の間をスキップする
        while pos < text.len() {
            let next = next_char_boundary(text, pos);
            let ch = text[pos..next].chars().next().unwrap_or('\n');
            if ch.is_whitespace() {
                break;
            }
            let ch_is_word = ch.is_alphanumeric() || ch == '_';
            if ch_is_word != is_word_char {
                break;
            }
            pos = next;
        }
        // 末尾の空白をスキップする
        while pos < text.len() {
            let next = next_char_boundary(text, pos);
            let ch = text[pos..next].chars().next().unwrap_or('\n');
            if !ch.is_whitespace() {
                break;
            }
            pos = next;
        }
    }

    pos
}
