//! DSR の report が「出力の形」の見出しを持っているかを、LLM を使わずに確かめる。
//!
//! 期待する見出しは、ブロックの本文（`.sai/blocks/<id>.md`）の `## 出力の形` 節から読む。
//! 見出しの一覧を別のファイルに写すと 2 か所になり、片方が古くなるためである。

/// report の見出しが期待どおりかを確かめた結果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckResult {
    /// 期待する見出しの数。
    pub expected: usize,
    /// report に無かった見出し（期待する見出しの並び順）。
    pub missing: Vec<String>,
    /// report に `## 引き継ぎ` があるか。
    pub has_handoff: bool,
}

/// ブロックの本文から、`## 出力の形` 節の見出しを順に取り出す。
///
/// - `## 出力の形` の次の行から、コード欄の外にある次の `## ` 行の手前までを対象にする。
/// - コード欄（```` ``` ```` で囲まれた範囲）の中の `## ` / `### ` 行も見出しとして拾う。
/// - 見出しの文字の後ろの `（…）`（全角括弧）は切り落とす。
/// - 表の行や箇条書きなど、`## ` / `### ` で始まらない行は見出しにしない。
#[must_use]
pub fn expected_headings(block_md: &str) -> Vec<String> {
    let mut headings = Vec::new();
    let mut in_section = false;
    let mut in_fence = false;

    for line in block_md.lines() {
        let line = line.trim_end();

        // コード欄の開始・終了（` ``` ` で始まる行で切り替える）
        if line.trim_start().starts_with("```") {
            in_fence = !in_fence;
            continue;
        }

        if !in_section {
            if !in_fence && line.starts_with("## 出力の形") {
                in_section = true;
            }
            continue;
        }

        // コード欄の外にある次の `## ` で節が終わる
        if !in_fence && line.starts_with("## ") {
            break;
        }

        if let Some(text) = heading_text(line) {
            headings.push(text);
        }
    }

    headings
}

/// report の見出しを集め、期待する見出しのうち無いものを返す。
///
/// 一致の判定は「前後の空白を除いて、report の見出しが期待する見出しで始まる」。
/// たとえば期待が `方法の弱点` なら、report の `方法の弱点（3 点）` は一致とみなす。
#[must_use]
pub fn check(report_md: &str, expected: &[String]) -> CheckResult {
    let report_headings: Vec<String> = report_md.lines().filter_map(heading_text).collect();

    let missing = expected
        .iter()
        .filter(|want| {
            !report_headings
                .iter()
                .any(|got| got.starts_with(want.as_str()))
        })
        .cloned()
        .collect();

    let has_handoff = report_headings.iter().any(|h| h.starts_with("引き継ぎ"));

    CheckResult {
        expected: expected.len(),
        missing,
        has_handoff,
    }
}

/// `## ` / `### ` で始まる行から見出しの文字を取り出す。見出しでなければ `None`。
///
/// 見出しの後ろの `（…）` は切り落とす。切り落とした結果が空なら見出しにしない。
fn heading_text(line: &str) -> Option<String> {
    let line = line.trim();
    let rest = line
        .strip_prefix("### ")
        .or_else(|| line.strip_prefix("## "))?;
    let rest = rest.trim();
    let text = match rest.find('（') {
        Some(index) => &rest[..index],
        None => rest,
    }
    .trim();

    if text.is_empty() {
        None
    } else {
        Some(text.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// D05 で作った批判役のブロック（期待する見出しは 6 つ）。
    const CRITIC: &str = include_str!("../../sai-paper-cli/assets/blocks/paper-critic.md");

    #[test]
    fn コード欄の中の見出しを拾う() {
        let block = "\
## 出力の形
report は次の見出しで書きます。

```markdown
## ああ
### いい（補足）
## うう
```
";
        assert_eq!(expected_headings(block), vec!["ああ", "いい", "うう"]);
    }

    #[test]
    fn そのままの行の見出しを拾う() {
        let block = "\
## 出力の形
report は次の見出しで書きます。

```markdown
## そのまま見出し
- ## 箇条書きの見出しは拾わない
```
";
        // 箇条書き（`- ## …`）は行頭が `## ` でないので拾わない
        assert_eq!(expected_headings(block), vec!["そのまま見出し"]);
    }

    #[test]
    fn 表や箇条書きや括弧だけの行は拾わない() {
        let block = "\
## 出力の形
```markdown
## 本物
| サブトピック | 研究の量 |
|---|---|
- 目的:
（研究の量・証拠の強さ）
## あと
```
";
        assert_eq!(expected_headings(block), vec!["本物", "あと"]);
    }

    #[test]
    fn 見出しの後ろの全角括弧を切り落とす() {
        let block = "\
## 出力の形
```markdown
## 手薄な領域（なぜ手薄と言えるか）
## トピック × 観点の表
### 検索用（英語の検索語と組み合わせ）
```
";
        assert_eq!(
            expected_headings(block),
            vec!["手薄な領域", "トピック × 観点の表", "検索用"]
        );
    }

    #[test]
    fn 出力の形の後ろの節を拾わない() {
        let block = "\
## 出力の形
```markdown
## ああ
```
## メモ
## ええ
";
        assert_eq!(expected_headings(block), vec!["ああ"]);
    }

    #[test]
    fn 批判役のブロックの見出しは6つ() {
        let headings = expected_headings(CRITIC);
        assert_eq!(
            headings,
            vec![
                "主張ごとの証拠の強さ",
                "方法の弱点",
                "結果の信頼性",
                "食い違う報告とその理由",
                "著者に聞きたいこと",
                "引き継ぎ",
            ]
        );
    }

    #[test]
    fn 揃っているときは足りない見出しが無い() {
        let report = "\
## 主張ごとの証拠の強さ
## 方法の弱点
## 結果の信頼性
## 食い違う報告とその理由
## 著者に聞きたいこと
## 引き継ぎ
";
        let expected = expected_headings(CRITIC);
        let result = check(report, &expected);
        assert_eq!(result.expected, 6);
        assert!(result.missing.is_empty());
        assert!(result.has_handoff);
    }

    #[test]
    fn 足りない見出しを返す() {
        let report = "\
## 主張ごとの証拠の強さ
## 方法の弱点
## 引き継ぎ
";
        let expected = expected_headings(CRITIC);
        let result = check(report, &expected);
        assert_eq!(
            result.missing,
            vec![
                "結果の信頼性",
                "食い違う報告とその理由",
                "著者に聞きたいこと"
            ]
        );
    }

    #[test]
    fn 見出しの後ろに補足があっても一致とみなす() {
        let report = "\
## 方法の弱点（3 点）
## 引き継ぎ（research-core のひな形のまま）
";
        let expected = vec!["方法の弱点".to_string(), "引き継ぎ".to_string()];
        let result = check(report, &expected);
        assert!(result.missing.is_empty());
        assert!(result.has_handoff);
    }

    #[test]
    fn 引き継ぎが無ければhas_handoffはfalse() {
        let report = "## 方法の弱点\n";
        let expected = vec!["方法の弱点".to_string()];
        let result = check(report, &expected);
        assert!(!result.has_handoff);
    }
}
