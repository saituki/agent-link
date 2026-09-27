//! PAPER 画面に表示する abstract（英文・日本語訳）の整形。
//!
//! 背景: Crossref が返す abstract には JATS XML（`<jats:title>` / `<jats:p>` /
//! `<jats:list>` 等）と、本文と無関係な分類情報（`This article is categorized under: ...`）が
//! 混入する。従来は `<jats:p>` / `<jats:sec>` のみ除去していたため、残りのタグが
//! PAPER 画面にそのまま表示されていた。また翻訳後の概要訳に改行が残り、
//! 文の途中で折り返されていた。ここで一元的に整形する。

/// 本文と無関係な分類情報の開始マーカー（小文字化して比較する）。
///
/// 英語原文（`This article is categorized under:`）と、既に日本語訳されて
/// DB に保存済みのテキストに残った訳文の両方に対応する。
const CATEGORY_MARKERS: &[&str] = &[
    "this article is categorized under:",
    "この記事は次のように分類されています",
    "この論文は次のように分類されています",
];

/// 単独の見出しタグ（`<jats:title>ABSTRACT</jats:title>` 等）。内容ごと除去する。
///
/// Crossref の abstract 先頭には `<jats:title>ABSTRACT</jats:title>` という見出しが
/// 付くことがあり、内容（`ABSTRACT`）も本文と無関係なため段落ごと落とす。
/// ただし日本語訳文の `<jats:title>要約</jats:title>` は、訳文と一体化している場合に
/// 内容まで落とすと本文を削る恐れがあるため、タグのみ除去して内容は残す。
const TITLE_TAGS_TO_DROP: &[&str] = &["ABSTRACT", "Abstract", "abstract"];

/// `</jats:p>` / `</p>` 以外の段落終了タグ。段落区切り（空行）に変換する。
/// `</jats:p>` / `</p>` は直前の改行を除去してから変換する（後述）。
const PARAGRAPH_END_TAGS: &[&str] = &["</jats:list>", "</jats:sec>", "</jats:title>", "</sec>"];

/// 分類情報マーカー以降を切り捨てる。
fn truncate_category_marker(text: &str) -> String {
    let lowered = text.to_lowercase();
    for marker in CATEGORY_MARKERS {
        if let Some(pos) = lowered.find(marker) {
            // `pos` は `lowered` 上のバイト位置だが、小文字化でバイト長が変わる文字
            // （例: `İ` → `i̇`）を含む場合に原文への適用がずれる。マーカー直前までの
            // テキストを取り出すだけなので、文字境界まで戻して安全に切り詰める。
            let mut end = pos.min(text.len());
            while end > 0 && !text.is_char_boundary(end) {
                end -= 1;
            }
            let mut truncated = text[..end].to_string();
            truncated.truncate(end);
            return truncated;
        }
    }
    text.to_string()
}

/// JATS / HTML タグを除去し、HTML エンティティをデコードする。
///
/// `<jats:p>` で囲まれた段落は段落区切り（`\n\n`）に変換し、それ以外のタグは
/// 単語連結防止のため空白に置換する（開始タグ直後の行頭空白は
/// `collapse_paragraphs` で除去される）。改行・連続空白の畳み込みは行わない。
fn strip_tags_and_entities(text: &str) -> String {
    let mut out = text.to_string();
    // `<jats:title>ABSTRACT</jats:title>` のような見出しは内容ごと除去する。
    for title in TITLE_TAGS_TO_DROP {
        out = out.replace(&format!("<jats:title>{title}</jats:title>"), "\n\n");
    }
    // `<jats:p>...</jats:p>` で囲まれた段落は段落区切りに変換する。
    // `collapse_paragraphs` が空行のみを段落境界とみなすため、閉じタグは
    // 無条件で空行（`\n\n`）に置換する。閉じタグ直前の改行が残ると
    // 三重改行になるが、空行連続は段落分割で吸収される。
    out = out.replace("\r\n", "\n");
    out = out.replace("</jats:p>", "\n\n");
    out = out.replace("</p>", "\n\n");
    for tag in PARAGRAPH_END_TAGS {
        out = out.replace(tag, "\n\n");
    }
    // 残りのタグ（`<jats:...>` / `<...>`）は除去する。タグ前後の単語連結を防ぐため空白に置換する。
    let mut cleaned = String::with_capacity(out.len());
    let mut in_tag = false;
    for ch in out.chars() {
        match ch {
            '<' => in_tag = true,
            '>' if in_tag => {
                in_tag = false;
                cleaned.push(' ');
            }
            _ if in_tag => {}
            _ => cleaned.push(ch),
        }
    }
    out = cleaned;

    // HTML エンティティをデコードする。
    out = out.replace("&gt;", ">");
    out = out.replace("&lt;", "<");
    out = out.replace("&quot;", "\"");
    out = out.replace("&#39;", "'");
    out = out.replace("&apos;", "'");
    out = out.replace("&nbsp;", " ");
    out = out.replace("&amp;", "&");

    // 閉じ忘れ等で残った `<` / `>` の断片を除去する。
    out = out.replace(['<', '>'], " ");
    out
}

/// 段落ごとに内部の改行・連続空白を 1 文字の空白に畳み、空段落を除いて空行で結合する。
///
/// 段落を区切るのは空行（改行の連続）のみ。単独の改行は段落内改行として空白に畳む。
/// 行頭・行末の空白は除去するため、開始タグ由来の行頭空白は残らない。
/// `split_whitespace` は全角空白（U+3000）も区切りとみなすため、
/// 日本語文中の全角空白は半角空白 1 文字に正規化される。
fn collapse_paragraphs(text: &str) -> String {
    // 空行で段落分割 → 各段落内の改行・連続空白を空白 1 文字に畳む。
    let mut paragraphs = Vec::new();
    for para in text.split('\n').collect::<Vec<_>>().chunk_by(|a, b| {
        // 空行か否かでグループ化する（`chunk_by` は連続要素の分割）。
        a.trim().is_empty() == b.trim().is_empty()
    }) {
        if para.iter().all(|line| line.trim().is_empty()) {
            continue;
        }
        let joined = para.join(" ");
        let collapsed = joined.split_whitespace().collect::<Vec<_>>().join(" ");
        if !collapsed.is_empty() {
            paragraphs.push(collapsed);
        }
    }
    paragraphs.join("\n\n")
}

/// Crossref の `abstract` に混入する JATS XML を除去し、表示用のプレーンテキストに整形する。
///
/// 処理内容:
/// 1. 本文と無関係な分類情報（`This article is categorized under:` 以降）を切り捨てる。
/// 2. 段落終了タグを段落区切り（空行）に変換し、残りの全タグ（`<jats:...>` / `<...>`）を除去する。
/// 3. HTML エンティティ（`&gt;` 等）をデコードする。
/// 4. 段落ごとの内部改行・連続空白を 1 文字の空白に畳み、空段落を除いて空行で結合する。
#[must_use]
pub fn clean_crossref_abstract(raw: &str) -> String {
    collapse_paragraphs(&strip_tags_and_entities(&truncate_category_marker(raw)))
}

/// 任意の abstract（英語・日本語問わず）に残った JATS / HTML タグ・余分な空白を除去する。
///
/// 既に DB へ保存済みの汚いテキストや、他プロバイダ由来のテキストを表示直前に
/// 整形するための防御的処理。段落構造（空行）は保持する。
#[must_use]
pub fn sanitize_abstract(raw: &str) -> String {
    collapse_paragraphs(&strip_tags_and_entities(&truncate_category_marker(raw)))
}

/// 空白類（半角・全角・タブ）かどうか。
fn is_space_char(ch: char) -> bool {
    ch == ' ' || ch == '\t' || ch == '　'
}

/// 日本語の概要訳（`abstract_ja`）を 1 段落の連続文に平坦化する。
///
/// 翻訳結果に含まれる改行を全てなくす。タグ除去後の改行とその前後の空白を除去し、
/// 英単語同士の結合時にだけ半角空白を 1 つ残す（例: `improved\ninformation` →
/// `improved information`、日本語文中の `されました\n。この` → `されました。この`）。
/// 残った 2 文字以上の連続空白は 1 文字に畳む。タグ由来の単独空白
/// （`<jats:title>要約</jats:title>\n本文` → `要約 本文`）も 1 文字に畳まれる。
#[must_use]
pub fn flatten_abstract_ja(raw: &str) -> String {
    let text = strip_tags_and_entities(&truncate_category_marker(raw));
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < chars.len() {
        let ch = chars[i];
        if ch == '\n' || ch == '\r' {
            // 連続する改行・空白をまとめて飛ばす。
            let mut j = i;
            while j < chars.len()
                && (chars[j] == '\n' || chars[j] == '\r' || is_space_char(chars[j]))
            {
                j += 1;
            }
            // 英単語同士の結合時にだけ空白を残す。日本語文中は詰める。
            let prev = out.chars().last();
            let next = chars.get(j).copied();
            let need_space = matches!(prev, Some(p) if p.is_ascii_alphanumeric())
                && matches!(next, Some(n) if n.is_ascii_alphanumeric());
            if need_space {
                out.push(' ');
            }
            i = j;
            continue;
        }
        out.push(ch);
        i += 1;
    }
    // 残った連続空白（半角・全角・タブ）を 1 文字の半角空白に畳む。
    let mut collapsed = String::with_capacity(out.len());
    let mut in_spaces = false;
    for ch in out.chars() {
        if is_space_char(ch) {
            if !in_spaces {
                collapsed.push(' ');
            }
            in_spaces = true;
        } else {
            collapsed.push(ch);
            in_spaces = false;
        }
    }
    collapsed.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::{clean_crossref_abstract, flatten_abstract_ja, sanitize_abstract};

    #[test]
    fn jatsタグと分類情報を除去する() {
        let raw = "<jats:title>ABSTRACT</jats:title>                  The widespread adoption.\
                  <jats:p>This article is categorized under:\n                    <jats:list list-type=\"simple\">                      <jats:list-item>                        Algorithmic\n   Development &gt; Text Mining                      </jats:list-item></jats:list></jats:p>";
        assert_eq!(clean_crossref_abstract(raw), "The widespread adoption.");
    }

    #[test]
    fn 段落境界は保持し段落内の改行は畳む() {
        // `</jats:p>` は段落境界（空行）。段落内の単独改行は空白に畳む。
        let raw = "<jats:p>一段落目の\n継続行。</jats:p><jats:p>二段落目。</jats:p>";
        assert_eq!(
            clean_crossref_abstract(raw),
            "一段落目の 継続行。\n\n二段落目。"
        );
        // 閉じタグ直前の改行が残っても段落は分かれたまま（空行連続は吸収）。
        let raw_trailing_newline =
            "<jats:p>一段落目の\n継続行。\n</jats:p><jats:p>二段落目。</jats:p>";
        assert_eq!(
            clean_crossref_abstract(raw_trailing_newline),
            "一段落目の 継続行。\n\n二段落目。"
        );
    }

    #[test]
    fn 日本語訳の改行を全てなくす() {
        let raw = "インターネットの普及により、医療文書のデジタル保存、共有、管理が可能になり、医療に変革がもたらされました\n。この変化により、情報へのアクセスが改善され、患者ケアが強化されました。";
        assert_eq!(
            flatten_abstract_ja(raw),
            "インターネットの普及により、医療文書のデジタル保存、共有、管理が可能になり、医療に変革がもたらされました。この変化により、情報へのアクセスが改善され、患者ケアが強化されました。"
        );
    }

    #[test]
    fn 英語訳の単語境界では空白を残す() {
        let raw = "This shift has improved\ninformation access, and opened\nnew avenues.";
        assert_eq!(
            flatten_abstract_ja(raw),
            "This shift has improved information access, and opened new avenues."
        );
    }

    #[test]
    fn 訳文中のjats残骸と分類訳文を除去する() {
        // `<jats:title>要約</jats:title>本文` のようにタグと本文が連続している場合、
        // タグ除去では空白を挟まないため `要約本文です。` になる。
        // 実物の翻訳テキストにはタグ後に空白・改行が残ることが多く、その場合は
        // `要約 本文です。` のように空白が残る。いずれも改行・タグ残骸は無い。
        let raw = "<jats:title>要約</jats:title>本文です。                                      この記事は次のように分類されています。                    <jats:list list-type=\"simple\">\n   <jats:リスト項目>                        アルゴリズム開発 &gt;テキストマイニング";
        let flattened = flatten_abstract_ja(raw);
        assert_eq!(flattened, "要約本文です。");
        assert!(!flattened.contains('\n'));
        assert!(!flattened.contains("jats"));
        let raw_spaced = "<jats:title>要約</jats:title>\n本文です。この記事は次のように分類されています。<jats:list>分類</jats:list>";
        // `</jats:title>\n本文` は `\n</...>` ではなく `>\n文` の形なので、
        // タグ除去後の改行は日本語文中として詰められ `要約本文です。` になる。
        assert_eq!(flatten_abstract_ja(raw_spaced), "要約本文です。");
    }

    #[test]
    fn 連続空白を畳む() {
        assert_eq!(
            sanitize_abstract("The   widespread    adoption."),
            "The widespread adoption."
        );
    }
}
