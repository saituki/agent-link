//! `sai-sci card --from-json` が読む C-CARD の JSON（Web の `phone sci card` と同じ形）。
//!
//! 中の処理は core（`sai_sci_core::rt::card`）が担う。ここは外から来る値の検査と
//! JSON の形だけを持つ。

use anyhow::{Result, anyhow};
use sai_sci_core::rt::card;
use serde_json::Value;

use crate::sci::parse_stdin_text;

/// C-CARD の JSON を [`card::CardSpec`] にする。
pub(crate) fn card_spec(input: &str) -> Result<card::CardSpec> {
    let value = parse_stdin_text(input)?;
    Ok(card::CardSpec {
        node: text_of(&value, "node"),
        block: text_of(&value, "block"),
        purpose: text_of(&value, "purpose"),
        expect: text_of(&value, "expect"),
        mode: text_of(&value, "mode"),
        papers: numbers_of(&value, "papers")?,
        runner: default_runner(&text_of(&value, "runner")),
        model: text_of(&value, "model"),
        thinking: text_of(&value, "thinking"),
        depth: text_of(&value, "depth"),
        origin_run: number_of(&value, "origin_run")?,
        // Web からは文書を受け取らない（`--input` は CLI の引数だけ）。
        inputs: Vec::new(),
        child: text_of(&value, "child"),
    })
}

/// 文字列の項目を読む（無ければ空文字）。
fn text_of(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string()
}

/// 数が 1 つだけ入っている項目を読む（無ければ `None`）。
fn number_of(value: &Value, key: &str) -> Result<Option<i64>> {
    let Some(item) = value.get(key) else {
        return Ok(None);
    };
    if item.is_null() {
        return Ok(None);
    }
    let number = item.as_i64().ok_or_else(|| anyhow!("{key} は数です"))?;
    Ok(Some(number))
}

/// 数の配列を読む（無ければ空）。
fn numbers_of(value: &Value, key: &str) -> Result<Vec<i64>> {
    let Some(item) = value.get(key) else {
        return Ok(Vec::new());
    };
    if item.is_null() {
        return Ok(Vec::new());
    }
    item.as_array()
        .ok_or_else(|| anyhow!("{key} は数の配列です"))?
        .iter()
        .map(|item| item.as_i64().ok_or_else(|| anyhow!("{key} は数の配列です")))
        .collect()
}

/// 実行者が空なら `cc` にする。
fn default_runner(runner: &str) -> String {
    if runner.is_empty() {
        "cc".to_string()
    } else {
        runner.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn card_spec_from_phone_json() {
        let input = r#"{
            "node": "Q12",
            "block": "sci-ideate",
            "purpose": "「データの量は？」の答え（A）を出す",
            "expect": "A を 2 つ以上",
            "mode": "purpose_first",
            "papers": [37, 40],
            "runner": "",
            "model": "haiku",
            "thinking": "low",
            "depth": "full",
            "origin_run": 120
        }"#;
        let spec = card_spec(input).unwrap();
        assert_eq!(spec.node, "Q12");
        assert_eq!(spec.block, "sci-ideate");
        assert_eq!(spec.expect, "A を 2 つ以上");
        assert_eq!(spec.mode, "purpose_first");
        assert_eq!(spec.papers, vec![37, 40]);
        // 実行者が空なら `cc`（READ_CARD は `user` を書く）。
        assert_eq!(spec.runner, "cc");
        assert_eq!(spec.model, "haiku");
        assert_eq!(spec.thinking, "low");
        assert_eq!(spec.depth, "full");
        assert_eq!(spec.origin_run, Some(120));
        // Web からは文書を受け取らない。
        assert!(spec.inputs.is_empty());
    }

    #[test]
    fn card_spec_from_phone_json_minimal() {
        let spec = card_spec(r#"{"node":"Q12","purpose":"候補を足す"}"#).unwrap();
        assert_eq!(spec.node, "Q12");
        assert!(spec.papers.is_empty());
        assert_eq!(spec.runner, "cc");
        // 空の model・thinking は sai-link の既定に任せる。
        assert!(spec.model.is_empty() && spec.thinking.is_empty());
    }

    #[test]
    fn card_spec_from_phone_json_rejects_bad_shape() {
        // 論文が数でない。
        assert!(card_spec(r#"{"node":"Q12","papers":["37"]}"#).is_err());
        // JSON として読めない。
        assert!(card_spec("{oops").is_err());
    }
}
