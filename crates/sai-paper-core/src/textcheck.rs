//! 文字種と引用の検査、報告書の frontmatter の付与。
//!
//! `sai-sci report prepare` が使う純関数をここに置く（`reports.rs` は本文を書き換えない決まり）。

use regex::Regex;
use std::sync::LazyLock;

/// 日本語の文に混ざってはいけない字が見つかった場所。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForeignChar {
    /// 1 始まりの行番号。
    pub line: usize,
    /// 見つかった字。
    pub ch: char,
    /// `ハングル`・`キリル文字`・`簡体字`。
    pub script: &'static str,
}

/// 日本語では使わない簡体字だけの一覧（誤検出が出たら外す。`据` `的` `与` `没` `体` `后` は入れない）。
const SIMPLIFIED_ONLY: &str = "这们个么吗说时对发现为进过还让给从动实应种样关问题经开务东车见认长门间无头两页觉讲边层书语记变产华业网络优质处计论习结构图线输选择训练测试验统预齐际脑视频类别调";

fn script_of(ch: char) -> Option<&'static str> {
    let o = ch as u32;
    if (0x1100..=0x11ff).contains(&o) || (0x3130..=0x318f).contains(&o) || (0xac00..=0xd7af).contains(&o)
    {
        Some("ハングル")
    } else if (0x0400..=0x04ff).contains(&o) {
        Some("キリル文字")
    } else if SIMPLIFIED_ONLY.contains(ch) {
        Some("簡体字")
    } else {
        None
    }
}

/// 文字種の検査。コード欄（``` の間）とインラインコード（`…`）は見ない。
#[must_use]
pub fn foreign_chars(text: &str) -> Vec<ForeignChar> {
    let mut found = Vec::new();
    let mut in_fence = false;
    for (index, line) in text.lines().enumerate() {
        if line.trim_start().starts_with("```") {
            in_fence = !in_fence;
            continue;
        }
        if in_fence {
            continue;
        }
        let mut in_code = false;
        for ch in line.chars() {
            if ch == '`' {
                in_code = !in_code;
            } else if !in_code && let Some(script) = script_of(ch) {
                found.push(ForeignChar { line: index + 1, ch, script });
            }
        }
    }
    found
}

/// 1 行の文に混ざった字の説明（無ければ `None`）。例: `Some("ハングル「한」が混ざっています")`
#[must_use]
pub fn foreign_message(text: &str) -> Option<String> {
    foreign_chars(text)
        .first()
        .map(|f| format!("{}「{}」が混ざっています", f.script, f.ch))
}

/// 引用 1 つ（`#37.5.2-4 Fig.3` など）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cite {
    /// 論文の番号。
    pub paper: i64,
    /// 節の番号。
    pub section: Option<i64>,
    /// 塊の番号。
    pub passage: Option<i64>,
    /// 塊の範囲の終わり。
    pub passage_end: Option<i64>,
    /// 番号の後ろの補足（`§3.2`・`Fig.3`。無ければ空）。
    pub note: String,
}

static CITE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^#([1-9][0-9]{0,8})((?:\.[1-9][0-9]*){1,2}(?:-[1-9][0-9]*)?)?( .{1,80})?$")
        .expect("cite regex")
});

/// 引用 1 つを読む。形が違えば `None`。
#[must_use]
pub fn parse_cite(text: &str) -> Option<Cite> {
    let caps = CITE_RE.captures(text)?;
    let paper = caps[1].parse().ok()?;
    let (mut section, mut passage, mut passage_end) = (None, None, None);
    if let Some(loc) = caps.get(2) {
        let (head, end) = match loc.as_str().split_once('-') {
            Some((h, e)) => (h, e.parse().ok()),
            None => (loc.as_str(), None),
        };
        let mut nums = head.split('.').filter(|s| !s.is_empty()).map(|s| s.parse::<i64>().ok());
        section = nums.next().flatten();
        passage = nums.next().flatten();
        passage_end = end;
    }
    let note = caps.get(3).map_or_else(String::new, |m| m.as_str().trim().to_string());
    Some(Cite { paper, section, passage, passage_end, note })
}

/// frontmatter を付けた（既にあれば整えた）本文を返す。
///
/// `pairs` は `(キー, 値, 上書きするか)`。既にあるキーは位置を保ち、上書き指定のものだけ値を替える。
/// 無いキーは frontmatter の末尾に足す。`pairs` に無いキー（`kind` など）はそのまま残す。
#[must_use]
pub fn with_frontmatter(markdown: &str, pairs: &[(&str, &str, bool)]) -> String {
    let lines: Vec<&str> = markdown.lines().collect();
    let close = if lines.first().is_some_and(|l| l.trim() == "---") {
        lines.iter().skip(1).position(|l| l.trim() == "---").map(|i| i + 1)
    } else {
        None
    };
    let (head, body): (&[&str], &[&str]) = match close {
        Some(c) => (&lines[1..c], &lines[c + 1..]),
        None => (&[], &lines[..]),
    };

    let mut seen = vec![false; pairs.len()];
    let mut out: Vec<String> = Vec::new();
    let mut skipping = false;
    for line in head {
        let key = (!line.starts_with([' ', '\t', '-'])).then(|| line.split_once(':')).flatten();
        if let Some((key, _)) = key {
            skipping = false;
            if let Some(i) = pairs.iter().position(|p| p.0 == key.trim()) {
                seen[i] = true;
                if pairs[i].2 {
                    out.push(format!("{}: {}", pairs[i].0, pairs[i].1));
                    skipping = true; // 配列の続きの行は捨てる
                    continue;
                }
            }
        } else if skipping {
            continue;
        }
        out.push((*line).to_string());
    }
    for (i, (key, value, _)) in pairs.iter().enumerate() {
        if !seen[i] {
            out.push(format!("{key}: {value}"));
        }
    }

    let mut text = format!("---\n{}\n---\n", out.join("\n"));
    let rest = body.join("\n");
    if close.is_none() {
        text.push('\n');
    }
    text.push_str(&rest);
    if markdown.ends_with('\n') && !rest.is_empty() {
        text.push('\n');
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn foreign_detects_scripts_with_line() {
        let f = foreign_chars("ok\n한국어と这个とпривет\n");
        let scripts: Vec<_> = f.iter().map(|x| (x.line, x.script)).collect();
        assert_eq!(scripts[0], (2, "ハングル"));
        assert!(scripts.contains(&(2, "簡体字")));
        assert!(scripts.contains(&(2, "キリル文字")));
    }

    #[test]
    fn foreign_skips_code_and_japanese() {
        assert!(foreign_chars("```\n한\n```\n`这` と 据える 的 与 没 体 后\n").is_empty());
        assert_eq!(foreign_message("한"), Some("ハングル「한」が混ざっています".into()));
        assert_eq!(foreign_message("日本語"), None);
    }

    #[test]
    fn cite_accepts() {
        assert_eq!(parse_cite("#37").unwrap().paper, 37);
        let c = parse_cite("#37 §3.2").unwrap();
        assert_eq!((c.section, c.note.as_str()), (None, "§3.2"));
        assert_eq!(parse_cite("#37.5").unwrap().section, Some(5));
        let c = parse_cite("#37.5.2").unwrap();
        assert_eq!((c.section, c.passage, c.passage_end), (Some(5), Some(2), None));
        let c = parse_cite("#37.5.2-4").unwrap();
        assert_eq!(c.passage_end, Some(4));
        assert_eq!(parse_cite("#37 Fig.3").unwrap().note, "Fig.3");
        assert_eq!(parse_cite("#37.5 表 2").unwrap().note, "表 2");
    }

    #[test]
    fn cite_rejects() {
        for bad in ["#0", "#37.0", "#37.5.2.1", "37", "#37§3"] {
            assert!(parse_cite(bad).is_none(), "{bad}");
        }
    }

    #[test]
    fn frontmatter_adds_and_keeps() {
        let pairs = [("block", "b", true), ("run", "5", true), ("papers", "1", false)];
        assert_eq!(
            with_frontmatter("# 題\n本文\n", &pairs),
            "---\nblock: b\nrun: 5\npapers: 1\n---\n\n# 題\n本文\n"
        );
        let out = with_frontmatter("---\nkind: 回答\nblock: x\npapers: 9\n---\n本文\n", &pairs);
        assert_eq!(out, "---\nkind: 回答\nblock: b\npapers: 9\nrun: 5\n---\n本文\n");
    }
}
