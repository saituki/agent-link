//! Google 翻訳（非公式スクレイピング）の Rust 移植。
//!
//! `docs/pdf2zh_google_translate.md` の Python 実装を `reqwest` で再現する。
//! エンドポイント: `https://translate.google.com/m`
//! UA偽装必須、 `t0` / `result-container` を正規表現で抜く。

use regex::Regex;
use std::sync::OnceLock;

/// Google 翻訳エンドポイント。
const ENDPOINT: &str = "https://translate.google.com/m";

/// IE6 偽装 UA（UA無しだと 403/bot判定）。
const USER_AGENT: &str = "Mozilla/4.0 (compatible;MSIE 6.0;Windows NT 5.1;SV1;.NET CLR 1.1.4322;.NET CLR 2.0.50727;.NET CLR 3.0.04506.30)";

/// 制御文字を除去する（`unicodedata.category(ch)[0] != \"C\"` に相当）。
fn remove_control_characters(s: &str) -> String {
    s.chars().filter(|ch| !ch.is_control()).collect()
}

/// HTMLエンティティをデコードする（最小実装）。
fn html_unescape(s: &str) -> String {
    // 順序重要: &amp; は最後に
    let mut out = s.to_string();
    out = out.replace("&lt;", "<");
    out = out.replace("&gt;", ">");
    out = out.replace("&quot;", "\"");
    out = out.replace("&#39;", "'");
    out = out.replace("&apos;", "'");
    // 数値参照 &#123; / &#xAB;
    // 簡易的に regex で置換
    let re_dec = Regex::new(r"&#(\d+);").unwrap();
    out = re_dec
        .replace_all(&out, |caps: &regex::Captures| {
            let code: u32 = caps[1].parse().unwrap_or(0);
            char::from_u32(code)
                .map(|c| c.to_string())
                .unwrap_or_default()
        })
        .to_string();
    let re_hex = Regex::new(r"&#x([0-9a-fA-F]+);").unwrap();
    out = re_hex
        .replace_all(&out, |caps: &regex::Captures| {
            let code = u32::from_str_radix(&caps[1], 16).unwrap_or(0);
            char::from_u32(code)
                .map(|c| c.to_string())
                .unwrap_or_default()
        })
        .to_string();
    out = out.replace("&amp;", "&");
    out
}

fn result_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r#"(?s)class="(?:t0|result-container)">(.*?)<"#).unwrap())
}

/// 単一テキスト（5000文字以内）を翻訳する内部実装。
async fn translate_chunk(
    text: &str,
    lang_in: &str,
    lang_out: &str,
) -> Result<String, anyhow::Error> {
    if text.trim().is_empty() {
        return Ok(text.to_string());
    }

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()?;

    let resp = client
        .get(ENDPOINT)
        .header("User-Agent", USER_AGENT)
        .query(&[("sl", lang_in), ("tl", lang_out), ("q", text)])
        .send()
        .await?;

    if resp.status().as_u16() == 400 {
        return Ok("IRREPARABLE TRANSLATION ERROR".to_string());
    }

    let status = resp.status();
    let body = resp.text().await?;

    if !status.is_success() {
        anyhow::bail!(
            "Google翻訳 HTTP {}: {}",
            status,
            &body[..body.len().min(500)]
        );
    }

    let caps = result_regex().captures(&body);
    let Some(caps) = caps else {
        anyhow::bail!(
            "翻訳結果のパースに失敗しました（HTML構造が変更された可能性）: {}",
            &body[..body.len().min(500)]
        );
    };
    let raw = caps.get(1).map(|m| m.as_str()).unwrap_or("");
    let decoded = html_unescape(raw);
    Ok(remove_control_characters(&decoded))
}

/// テキストを日本語へ翻訳する（5000文字ごとに分割して結合）。
///
/// `lang_in` / `lang_out` は `en` / `ja` 等の言語コード。
pub async fn translate(text: &str, lang_in: &str, lang_out: &str) -> Result<String, anyhow::Error> {
    if text.trim().is_empty() {
        return Ok(text.to_string());
    }

    // 5000文字制限を考慮して分割
    const CHUNK_SIZE: usize = 5000;
    if text.len() <= CHUNK_SIZE {
        return translate_chunk(text, lang_in, lang_out).await;
    }

    // 文字境界を考慮して分割
    let mut out = String::new();
    let mut start = 0;
    while start < text.len() {
        let mut end = (start + CHUNK_SIZE).min(text.len());
        // UTF-8境界まで戻す
        while end < text.len() && !text.is_char_boundary(end) {
            end -= 1;
        }
        let chunk = &text[start..end];
        let translated = translate_chunk(chunk, lang_in, lang_out).await?;
        if translated == "IRREPARABLE TRANSLATION ERROR" {
            return Ok(translated);
        }
        out.push_str(&translated);
        start = end;
        // レート制限緩和（分割時は少し待つ）
        if start < text.len() {
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn html_unescape_basic() {
        assert_eq!(html_unescape("a &lt;b&gt; &amp; c"), "a <b> & c");
        assert_eq!(html_unescape("&#39;test&#39;"), "'test'");
        assert_eq!(html_unescape("&#x27;hi&#x27;"), "'hi'");
    }

    #[test]
    fn remove_control_characters_basic() {
        assert_eq!(remove_control_characters("a\x00b\x1Fc"), "abc");
    }
}
