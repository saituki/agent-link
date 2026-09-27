//! Markdown から本文中の引用文を抽出する。
//!
//! 引用を含む**文**を 1 文ずつ切り出し、参考文献の番号に写す（段階 8）。
//! - 番号: `[1]` / `[1, 3]` / `[3–5]` / `[3, 5–7]` / `[9]-[12]`
//! - 著者-年（括弧）: `(Smith et al., 2020; Jones and Lee, 2019a)` / `(A, 2020, B et al., 2021)`
//! - 著者-年（文中）: `Smith et al. (2020)` / `Smith and Jones (2019)` / `Smith (2020, 2021)` /
//!   `Guo et al. [2024b]`
//! 上付きの引用は、手元の docling の出力に現れなかったため扱わない。

use regex::Regex;
use std::sync::LazyLock;

/// 抽出された文: `(本文, 引用 ref_no リスト)`。
pub type ExtractedSentence = (String, Vec<i64>);

/// 文献 1 件（著者-年の突合用）: `(ref_no, 筆頭著者, 年, 年の後ろの字 a/b)`。
pub type AuthorYearRef = (i64, String, Option<i64>, Option<String>);

/// 番号の引用。`[9]-[12]` のように括弧をまたぐ範囲も拾う。
static NUMERIC_CITE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\[(\d{1,3}(?:\s*[,;–—-]\s*\d{1,3})*)\](?:\s*[–—-]\s*\[(\d{1,3})\])?")
        .expect("numeric cite regex")
});

/// 括弧の中身（年を含むもの）。
static PAREN_WITH_YEAR: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\(([^()]{0,400}?(?:19|20)\d{2}[a-z]?)\)").expect("paren year regex")
});

/// 著者名 + 年（括弧の中の 1 件）。`Smith et al., 2020` / `Song & Ermon, 2019` / `OpenAI, 2022`。
/// 年の後ろに `, 2021b` のように年だけが続けば、同じ著者の別の年として拾う。
static NAME_YEARS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?P<name>\p{Lu}[\p{L}'’\-]+)(?:\s+(?:et\s+al\.?|and\s+\p{Lu}[\p{L}'’\-]+|&\s*\p{Lu}[\p{L}'’\-]+))?,?\s+(?P<years>(?:19|20)\d{2}[a-z]?(?:\s*,\s*(?:19|20)\d{2}[a-z]?)*)",
    )
    .expect("name years regex")
});

/// 文中の著者-年: `Smith et al. (2020)` / `Smith and Jones (2019, 2021)` / `Guo et al. [2024b]`。
static INLINE_AUTHOR_YEAR: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?P<name>\p{Lu}[\p{L}'’\-]+)(?:\s+(?:et\s+al\.?|and\s+\p{Lu}[\p{L}'’\-]+|&\s*\p{Lu}[\p{L}'’\-]+))?\s*[\(\[](?P<years>(?:19|20)\d{2}[a-z]?(?:\s*[,;]\s*(?:19|20)\d{2}[a-z]?)*)[\)\]]",
    )
    .expect("inline author year regex")
});

/// 年 1 つ（`2019a` → 2019, 'a'）。
static YEAR: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"((?:19|20)\d{2})([a-z])?").expect("year regex"));

/// 文の区切りで切らない略語（ピリオドの直前の語）。
const ABBREVIATIONS: &[&str] = &[
    "al", "fig", "figs", "e.g", "i.e", "eg", "ie", "vs", "no", "nos", "eq", "eqs", "sec", "secs",
    "ref", "refs", "cf", "etc", "approx", "dr", "mr", "ms", "prof", "vol", "pp", "ch", "tab",
];

/// 1 行（段落）を文に分ける。表の行（`|` で始まる）は分けない。
///
/// 区切りは `.` / `!` / `?` / `。` の後の空白。略語（`et al.` / `Fig.` / `e.g.` …）と
/// イニシャル（`J.`）、括弧の中のピリオドでは切らない。
fn split_sentences(line: &str) -> Vec<&str> {
    if line.trim_start().starts_with('|') {
        return vec![line];
    }
    let mut out = Vec::new();
    let mut start = 0usize;
    let mut depth = 0i32;
    let chars: Vec<(usize, char)> = line.char_indices().collect();
    for (i, &(idx, c)) in chars.iter().enumerate() {
        match c {
            '(' | '[' => depth += 1,
            ')' | ']' => depth = (depth - 1).max(0),
            _ => {}
        }
        let end = idx + c.len_utf8();
        if c == '。' {
            out.push(&line[start..end]);
            start = end;
            continue;
        }
        if depth > 0 || !matches!(c, '.' | '!' | '?') {
            continue;
        }
        // 次が空白（または行末）のときだけ区切り候補
        let next = chars.get(i + 1).map(|&(_, n)| n);
        if next.is_some_and(|n| !n.is_whitespace()) {
            continue;
        }
        if c == '.' {
            // 直前の語（英字とピリオド）
            let word: String = line[start..idx]
                .chars()
                .rev()
                .take_while(|ch| ch.is_alphabetic() || *ch == '.')
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect();
            let lower = word.to_lowercase();
            let is_initial = word.chars().count() == 1 && word.chars().all(char::is_uppercase);
            if is_initial || ABBREVIATIONS.contains(&lower.trim_end_matches('.')) {
                continue;
            }
        }
        out.push(&line[start..end]);
        start = end;
    }
    if start < line.len() {
        out.push(&line[start..]);
    }
    out.into_iter().map(str::trim).filter(|s| !s.is_empty()).collect()
}

/// 本文を文に分けたもの（References 以降を除き、HTML の `&amp;` を戻す）。
fn body_sentences(markdown: &str) -> Vec<String> {
    let content = remove_references_section(markdown).replace("&amp;", "&");
    split_lines(&content)
        .iter()
        .flat_map(|line| split_sentences(line).into_iter().map(str::to_string).collect::<Vec<_>>())
        .collect()
}

/// 著者-年 1 件を文献の番号に写す（姓と年が合うもの。`2019a` の字も使う）。
fn match_author_year(surname: &str, year: i64, suffix: Option<char>, refs: &[AuthorYearRef]) -> Option<i64> {
    let surname = surname.to_lowercase();
    // 筆頭著者の語のどれかが姓と一致し、年が一致するもの
    let candidates: Vec<&AuthorYearRef> = refs
        .iter()
        .filter(|(_, author, y, _)| {
            *y == Some(year)
                && author
                    .to_lowercase()
                    .split(|ch: char| !ch.is_alphanumeric() && ch != '\'' && ch != '-')
                    .any(|word| word == surname)
        })
        .collect();
    match (candidates.as_slice(), suffix) {
        ([], _) => None,
        ([only], _) => Some(only.0),
        (many, Some(letter)) => {
            let letter = letter.to_string();
            // 文献側に字があればそれで選び、無ければ並び順（a = 1 件目）で選ぶ
            many.iter()
                .find(|(_, _, _, s)| s.as_deref() == Some(letter.as_str()))
                .map(|r| r.0)
                .or_else(|| {
                    let index = (letter.as_bytes()[0] - b'a') as usize;
                    many.get(index).map(|r| r.0)
                })
        }
        (many, None) => Some(many[0].0),
    }
}

/// `2019a, 2020` のような年の並びを 1 つずつにする。
fn years(text: &str) -> Vec<(i64, Option<char>)> {
    YEAR.captures_iter(text)
        .filter_map(|cap| {
            let year = cap[1].parse().ok()?;
            let suffix = cap.get(2).and_then(|m| m.as_str().chars().next());
            Some((year, suffix))
        })
        .collect()
}

/// 本文中の著者-年引用を検出し、参考文献（姓・年・字）に突合して ref_no に写す。
///
/// 括弧の中の並び（`;` / `,` 区切り）と、文中の書き方（`Smith et al. (2020)`）の両方を拾う。
pub fn extract_author_year_sentences(markdown: &str, refs: &[AuthorYearRef]) -> Vec<ExtractedSentence> {
    let mut out = Vec::new();
    for sentence in body_sentences(markdown) {
        let mut cite_refs: Vec<i64> = Vec::new();
        let mut push = |name: &str, years_text: &str| {
            for (year, suffix) in years(years_text) {
                if let Some(no) = match_author_year(name, year, suffix, refs)
                    && !cite_refs.contains(&no)
                {
                    cite_refs.push(no);
                }
            }
        };
        for paren in PAREN_WITH_YEAR.captures_iter(&sentence) {
            for cap in NAME_YEARS.captures_iter(&paren[1]) {
                push(&cap["name"], &cap["years"]);
            }
        }
        for cap in INLINE_AUTHOR_YEAR.captures_iter(&sentence) {
            push(&cap["name"], &cap["years"]);
        }
        cite_refs.sort_unstable();
        if !cite_refs.is_empty() {
            out.push((sentence, cite_refs));
        }
    }
    out
}

/// Markdown から番号の引用を含む文を抽出する。
///
/// References セクション以降を除き、本文を文に分けて、`[N]` / `[N,M]` / `[N-M]` /
/// `[N]-[M]` の引用番号を持つ文だけを返す。
pub fn extract_sentences(markdown: &str) -> Vec<ExtractedSentence> {
    let mut result = Vec::new();
    for sentence in body_sentences(markdown) {
        let mut cite_refs = Vec::new();
        for cap in NUMERIC_CITE.captures_iter(&sentence) {
            let mut nums = parse_ref_numbers(&cap[1]);
            // `[9]-[12]`: 1 つ目の最後の番号から 2 つ目まで
            if let (Some(hi), Some(&lo)) = (cap.get(2), nums.last())
                && let Ok(hi) = hi.as_str().parse::<i64>()
                && hi > lo
                && hi - lo <= 100
            {
                nums.extend(lo + 1..=hi);
            }
            for num in nums {
                if !cite_refs.contains(&num) {
                    cite_refs.push(num);
                }
            }
        }
        cite_refs.sort_unstable();
        if !cite_refs.is_empty() {
            result.push((sentence, cite_refs));
        }
    }
    result
}

/// 本文中から図表ラベル（例: `Figure 1`）を言及している文を 1 つ返す。
///
/// References / Bibliography セクションは対象外。表記ゆれ（`Fig. 1` / `Fig 1` /
/// `Figure1`）を吸収し、数字の直後が数字でないことを確認する（`Figure 10` を
/// `Figure 1` と誤認しないため）。見つからない場合は `None`。
///
/// キャプション行（行頭がラベル + `.` / `:` / `：` / `。`。docling は
/// `Figure 2. キャプション本文` の形で出力する）は図の説明そのものなので
/// 言及として返さない。文中の言及を、行頭から始まる言及より優先する。
#[must_use]
pub fn find_mention(markdown: &str, label: &str) -> Option<String> {
    let aliases = label_aliases(label);
    if aliases.is_empty() {
        return None;
    }
    let content = remove_references_section(markdown);
    // 行頭から始まる言及（例: `Figure 2 shows ...`）は次点として保持する。
    let mut line_head: Option<String> = None;
    for line in split_lines(&content) {
        let lower = line.to_lowercase();
        // 大文字小文字変換でバイト長が変わる文字を含む行は、位置対応が崩れるため
        // 大文字小文字を区別せず探索しない（図ラベルは ASCII なので実害はない）。
        let target = if lower.len() == line.len() {
            lower
        } else {
            line.clone()
        };
        for alias in &aliases {
            let mut from = 0usize;
            while let Some(rel) = target[from..].find(alias.as_str()) {
                let pos = from + rel;
                let end = pos + alias.len();
                // 数字の直後が数字なら別の図番号（例: Figure 10）
                let next_is_digit = line[end..]
                    .chars()
                    .next()
                    .is_some_and(|c| c.is_ascii_digit());
                if !next_is_digit {
                    let next_char = line[end..].chars().next();
                    let at_head = at_line_start(&line, pos);
                    // 行頭ラベル + 文区切りはキャプション行なので言及ではない。
                    let is_caption =
                        at_head && matches!(next_char, Some('.' | ':' | '：' | '。'));
                    if !is_caption {
                        let sentence = sentence_around(&line, pos, end);
                        if at_head {
                            line_head.get_or_insert(sentence);
                        } else {
                            return Some(sentence);
                        }
                    }
                }
                from = end;
                if from >= target.len() {
                    break;
                }
            }
        }
    }
    line_head
}

/// `pos` より前が markdown の装飾記号と空白だけか（= ラベルが行頭にある）。
fn at_line_start(line: &str, pos: usize) -> bool {
    line[..pos]
        .chars()
        .all(|c| c.is_whitespace() || matches!(c, '#' | '*' | '_' | '>' | '-' | '+' | '|' | '`'))
}

/// ラベルから表記ゆれを生成する（`Figure 1` → `figure 1` / `fig. 1` / `fig 1` / `figure1`）。
fn label_aliases(label: &str) -> Vec<String> {
    let label = label.trim().to_lowercase();
    let mut parts = label.split_whitespace();
    let Some(word) = parts.next() else {
        return Vec::new();
    };
    let num = parts.next().unwrap_or_default();
    if num.is_empty() {
        return vec![label];
    }
    let mut aliases = vec![format!("{word} {num}")];
    if word.starts_with("fig") {
        for alt in ["fig", "fig.", "figure"] {
            for sep in [" ", ""] {
                aliases.push(format!("{alt}{sep}{num}"));
            }
        }
    } else if word.starts_with("tab") {
        for alt in ["table", "tab", "tab."] {
            for sep in [" ", ""] {
                aliases.push(format!("{alt}{sep}{num}"));
            }
        }
    }
    aliases.sort_by_key(|a| std::cmp::Reverse(a.len()));
    aliases.dedup();
    aliases
}

/// `pos..end` が属する文を抜き出す（句点・感嘆符・疑問符・`.` + 空白を境界とする）。
///
/// 境界は `pos..end` の内側では判定しない（`Fig. 1` のピリオドで切れないようにする）。
fn sentence_around(line: &str, pos: usize, end: usize) -> String {
    // 直前の境界
    let mut start = 0usize;
    let chars: Vec<(usize, char)> = line.char_indices().collect();
    for i in 0..chars.len() {
        let (idx, c) = chars[i];
        if idx >= pos {
            break;
        }
        let is_boundary = matches!(c, '。' | '！' | '？')
            || (c == '.'
                && line[idx + c.len_utf8()..]
                    .chars()
                    .next()
                    .is_none_or(|n| n.is_whitespace()));
        if is_boundary {
            start = idx + c.len_utf8();
        }
    }
    // 直後の境界
    let mut stop = line.len();
    for (idx, c) in chars.iter().copied() {
        if idx < end {
            continue;
        }
        if matches!(c, '。' | '！' | '？') {
            stop = idx + c.len_utf8();
            break;
        }
        if c == '.'
            && line[idx + c.len_utf8()..]
                .chars()
                .next()
                .is_none_or(|n| n.is_whitespace())
        {
            stop = idx + c.len_utf8();
            break;
        }
    }
    line[start..stop].trim().to_string()
}

/// References セクション以降を除去する（`parsers::extract_references_section` と同一の見出し判定）。
fn remove_references_section(markdown: &str) -> String {
    match super::parsers::extract_references_section(markdown) {
        Some(_) => {
            // 見出し行の位置で打ち切り
            static HEADING: LazyLock<Regex> = LazyLock::new(|| {
                Regex::new(
                    r"(?im)^[ \t>]*(?:#{1,6}[ \t]*)?(?:\*{0,2})?(?:references|bibliography)(?:\**)?(?:[ \t]*:)?[ \t\*]*$",
                )
                .expect("heading regex")
            });
            let heading = &*HEADING;
            let cut = markdown
                .lines()
                .position(|line| heading.is_match(line))
                .unwrap_or(usize::MAX);
            markdown.lines().take(cut).collect::<Vec<_>>().join("\n")
        }
        None => markdown.to_string(),
    }
}

/// 本文を行単位に分割する（空行・短すぎる行は除外）。
fn split_lines(text: &str) -> Vec<String> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && l.chars().count() > 10)
        .map(str::to_string)
        .collect()
}

/// 引用番号文字列をパースする（例: `"1,2,3"` → `[1,2,3]`, `"1-3"` → `[1,2,3]`）。
///
/// 区切りはカンマ / 空白 / セミコロン、範囲は `-` / `–` / `—`。
pub fn parse_ref_numbers(raw: &str) -> Vec<i64> {
    // `3 – 5` のように範囲の記号の前後に空白があっても 1 つの範囲として読む
    static SPACED_DASH: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"\s*([\u2013\u2014-])\s*").expect("spaced dash regex"));
    let raw = SPACED_DASH.replace_all(raw, "$1");
    let mut nums: Vec<i64> = Vec::new();
    for token in raw.split([',', ';', ' ']).filter(|s| !s.is_empty()) {
        // 範囲: 1-3 / 1–3 / 1—3
        if let Some(sep_pos) = token.find(['-', '\u{2013}', '\u{2014}']) {
            let lo_str = &token[..sep_pos];
            let sep_len = token[sep_pos..].chars().next().map_or(1, char::len_utf8);
            let hi_str = &token[sep_pos + sep_len..];
            if let (Ok(lo), Ok(hi)) = (lo_str.trim().parse::<i64>(), hi_str.trim().parse::<i64>()) {
                if lo >= 1 && hi >= lo && hi - lo <= 100 {
                    nums.extend(lo..=hi);
                }
            }
        } else if let Ok(n) = token.trim().parse::<i64>() {
            if (1..10_000).contains(&n) {
                nums.push(n);
            }
        }
    }
    nums.sort_unstable();
    nums.dedup();
    nums
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_ref_numbers_single() {
        assert_eq!(parse_ref_numbers("5"), vec![5]);
    }

    #[test]
    fn parse_ref_numbers_multiple() {
        assert_eq!(parse_ref_numbers("1,2,3"), vec![1, 2, 3]);
    }

    #[test]
    fn parse_ref_numbers_range() {
        assert_eq!(parse_ref_numbers("1-3"), vec![1, 2, 3]);
        assert_eq!(parse_ref_numbers("1\u{2013}3"), vec![1, 2, 3]);
    }

    #[test]
    fn parse_ref_numbers_mixed() {
        assert_eq!(parse_ref_numbers("1, 3-5, 7"), vec![1, 3, 4, 5, 7]);
    }

    #[test]
    fn extract_sentences_finds_citations() {
        let md = "This is a sentence [1].\nThis has multiple [2, 3].\nThis has none.\nReferences\nFoo [4].";
        let result = extract_sentences(md);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].1, vec![1]);
        assert_eq!(result[1].1, vec![2, 3]);
    }

    #[test]
    fn extract_sentences_handles_heading_style_references() {
        let md = "Body cites [7] here.\n\n## References\n\n[7] Foo, 2020.";
        let result = extract_sentences(md);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].1, vec![7]);
    }

    #[test]
    fn find_mention_locates_figure_reference() {
        let md = "We introduce the pipeline. As shown in Figure 1, the model improves a lot. Figure 10 is elsewhere.\n\n## References\n\nFigure 1 appears in refs only.";
        assert_eq!(
            find_mention(md, "Figure 1"),
            Some("As shown in Figure 1, the model improves a lot.".to_string())
        );
        // Fig. 表記も拾う
        let md2 = "The result (see Fig. 2.1 for details) is stable.";
        assert!(find_mention(md2, "Figure 2").is_some());
        // キャプション行（"Figure 5: ..."）は言及として拾わない
        let md3 = "Intro text.\nFigure 5: Sample results.\nAs discussed in Figure 5, gains hold.";
        assert_eq!(
            find_mention(md3, "Figure 5"),
            Some("As discussed in Figure 5, gains hold.".to_string())
        );
        // References 内だけの言及は拾わない
        assert_eq!(
            find_mention("Body only.\n\n## References\n\nFigure 3: X", "Figure 3"),
            None
        );
    }

    #[test]
    fn find_mention_skips_docling_caption_lines() {
        // docling のキャプション行（`Figure 2. ...`）は言及として返さない
        assert_eq!(
            find_mention("Figure 2. A caption of the figure.", "Figure 2"),
            None
        );
        // ラベルだけの行も同様
        assert_eq!(find_mention("Figure 1. Overview of it.", "Figure 1"), None);
        // markdown 装飾付きのキャプション行も同様
        assert_eq!(
            find_mention("## **Figure 4.** Results summary.", "Figure 4"),
            None
        );
        // 文中の言及は返す
        assert_eq!(
            find_mention(
                "Figure 2. A caption of the figure.\nAs shown in Figure 2, we improve things.",
                "Figure 2"
            ),
            Some("As shown in Figure 2, we improve things.".to_string())
        );
        // キャプション行より後に文中の言及があれば、そちらを優先する
        assert_eq!(
            find_mention(
                "Figure 3 shows the pipeline overview.\nWe evaluate it in Figure 3, with gains.",
                "Figure 3"
            ),
            Some("We evaluate it in Figure 3, with gains.".to_string())
        );
        // 文中の言及が無ければ行頭の言及を返す
        assert_eq!(
            find_mention("Figure 3 shows the pipeline overview.", "Figure 3"),
            Some("Figure 3 shows the pipeline overview.".to_string())
        );
    }

    #[test]
    fn remove_references_section_works() {
        let md = "Body text.\n\n## References\n\n[1] Foo, 2020.";
        let removed = remove_references_section(md);
        assert!(removed.contains("Body text."));
        assert!(!removed.contains("Foo"));
    }

    #[test]
    fn extract_author_year_sentences_maps_to_ref_no() {
        // 著者-年形式の本文引用 (Author et al., Year) を (姓, 年) で ref_no に写像する
        let md = "Recent work shows that blackboard architectures help (Nii, 1986).\nMany studies followed (Brown et al., 2020).\nNo citation here.\n\n## References\n\nNii, H. P. (1986). Title. Journal\nBrown, T. (2020). Title. Other Journal";
        // (ref_no, 姓, 年)
        let refs = vec![
            (1_i64, "Nii".to_string(), Some(1986_i64), None),
            (2_i64, "Brown".to_string(), Some(2020_i64), None),
            (3_i64, "Wang".to_string(), Some(2021_i64), None),
        ];
        let result = extract_author_year_sentences(md, &refs);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].1, vec![1]);
        assert_eq!(result[1].1, vec![2]);
    }

    /// 略語・イニシャル・括弧の中のピリオドでは文を切らない。
    #[test]
    fn split_sentences_keeps_abbreviations() {
        let line = "As in Fig. 2 and e.g. prior work by J. Smith et al. (see Sec. 3.1). This is next.";
        assert_eq!(
            split_sentences(line),
            vec!["As in Fig. 2 and e.g. prior work by J. Smith et al. (see Sec. 3.1).", "This is next."]
        );
    }

    /// 引用 1 つにつき、その文だけを返す（段落全体ではない）。
    #[test]
    fn numeric_citations_are_one_sentence_each() {
        let md = "First claim holds [1]. Unrelated sentence here. Ranges work [3 – 5] and [9]-[11].";
        let result = extract_sentences(md);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0], ("First claim holds [1].".to_string(), vec![1]));
        assert_eq!(result[1].1, vec![3, 4, 5, 9, 10, 11]);
    }

    fn refs() -> Vec<AuthorYearRef> {
        vec![
            (1, "Smith, J.".into(), Some(2020), None),
            (2, "Jones".into(), Some(2019), Some("a".into())),
            (3, "Jones".into(), Some(2019), Some("b".into())),
            (4, "Tianyu Guo".into(), Some(2024), None),
            (5, "Tianyu Guo".into(), Some(2024), None),
            (6, "Smith, J.".into(), Some(2021), None),
        ]
    }

    /// 括弧の中の並び（`;` と `,`）、`2019a / 2019b` の区別。
    #[test]
    fn author_year_lists_and_suffixes() {
        let md = "Prior work (Smith et al., 2020; Jones and Lee, 2019b) differs. Also (Jones & Lee, 2019a, Smith, 2021).";
        let result = extract_author_year_sentences(md, &refs());
        assert_eq!(result[0].1, vec![1, 3]);
        assert_eq!(result[1].1, vec![2, 6]);
    }

    /// 文中の書き方: `Smith et al. (2020)` / `Smith (2020, 2021)` / `Guo et al. [2024b]`。
    #[test]
    fn inline_author_year_forms() {
        let md = "Smith et al. (2020) showed it. Smith (2020, 2021) repeated it. Guo et al. [2024b] surveyed it.";
        let result = extract_author_year_sentences(md, &refs());
        assert_eq!(result[0].1, vec![1]);
        assert_eq!(result[1].1, vec![1, 6]);
        // 文献に字が無いときは並び順（b = 2 件目）で選ぶ
        assert_eq!(result[2].1, vec![5]);
    }
}
