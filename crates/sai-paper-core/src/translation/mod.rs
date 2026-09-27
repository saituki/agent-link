//! 翻訳機能（abstract 日本語訳 + PDF翻訳）。
//!
//! - abstract翻訳: 設定 `translate.abstract.service` で選ぶ。`google`（pdf2zh と同一手法の
//!   スクレイピングを Rust へ移植）か `openailiked`（OpenAI 互換 API）。軽量なため PAPRE 画面の表示時にオンデマンドで実行し、結果を `papers.abstract_ja` にキャッシュする。
//! - PDF翻訳: `pdf2zh` CLI を子プロセスとして呼び出す。重量処理のため手動トリガのみ（`T` キー）。

pub mod google;
pub mod pdf2zh;

use rusqlite::Connection;

use crate::database::DatabaseError;

/// 翻訳エラー。
#[derive(Debug, thiserror::Error)]
pub enum TranslationError {
    /// DB 操作失敗。
    #[error("database error: {0}")]
    Db(#[from] DatabaseError),
    /// rusqlite 直接エラー。
    #[error("database error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    /// ネットワーク/翻訳サービス失敗。
    #[error("translation error: {0}")]
    Service(String),
    /// 入力が空または不正。
    #[error("invalid input: {0}")]
    InvalidInput(String),
}

/// 論文の abstract を日本語へ翻訳し、DB にキャッシュする。
///
/// - `abstract_en` が空なら何もせず `Ok(None)` を返す。
/// - `papers.abstract_ja` が既に存在すればそれを返す（キャッシュヒット）。
/// - キャッシュが無ければ `service` の翻訳を実行し、DB へ保存してから返す。
/// - `force` が true の場合はキャッシュを無視して再翻訳する。
pub async fn ensure_abstract_ja(
    conn: &Connection,
    paper_id: i64,
    abstract_en: &str,
    force: bool,
    service: &crate::config::TranslateService,
) -> Result<Option<String>, TranslationError> {
    let service_name = abstract_service_name(service)?;
    // 翻訳前に JATS・余分な空白を除去する。JATS を翻訳にかけると
    // `<jats:リスト項目>` のような残骸が訳文に残るため、事前に取り除く。
    let cleaned_en = crate::abstracts::sanitize_abstract(abstract_en);
    if cleaned_en.trim().is_empty() {
        return Ok(None);
    }
    let text = cleaned_en.as_str();

    if !force {
        // キャッシュヒット確認
        let cached: Option<Option<String>> = conn
            .query_row(
                "SELECT abstract_ja FROM papers WHERE id = ?1",
                [paper_id],
                |row| row.get(0),
            )
            .ok();
        if let Some(Some(cached_text)) = cached {
            if !cached_text.trim().is_empty() {
                return Ok(Some(cached_text));
            }
        }
    }

    // `jobs` 表に記録し、同じ論文の要旨翻訳が実行中なら始めない（段階 5）。
    let job = match crate::jobs::start(conn, "translate_abstract", Some(paper_id)) {
        Ok(Some(id)) => Some(id),
        Ok(None) => {
            return Err(TranslationError::Service(
                "この論文の要旨はすでに翻訳中です".to_string(),
            ));
        }
        Err(_) => None,
    };
    let result = translate_and_store(conn, paper_id, text, service, service_name).await;
    if let Some(id) = job {
        let error = result.as_ref().err().map(ToString::to_string);
        let _ = crate::jobs::finish(conn, id, error.as_deref());
    }
    result
}

/// 翻訳して DB に保存する（[`ensure_abstract_ja`] の本体）。
async fn translate_and_store(
    conn: &Connection,
    paper_id: i64,
    text: &str,
    service: &crate::config::TranslateService,
    service_name: &str,
) -> Result<Option<String>, TranslationError> {
    let translated = match service_name {
        // 5000 文字ごとの分割は google モジュール内で行う
        "google" => google::translate(text, "en", "ja")
            .await
            .map_err(|e| TranslationError::Service(e.to_string()))?,
        _ => translate_openailiked(text, service).await?,
    };

    if translated.trim().is_empty() || translated == "IRREPARABLE TRANSLATION ERROR" {
        return Err(TranslationError::Service(format!(
            "翻訳結果が空またはエラー: {translated}"
        )));
    }

    // 訳文中の改行を全てなくしてから DB へ保存する。
    let flattened = crate::abstracts::flatten_abstract_ja(&translated);
    if flattened.trim().is_empty() {
        return Err(TranslationError::Service(
            "翻訳結果の整形後に空になりました".to_string(),
        ));
    }
    conn.execute(
        "UPDATE papers SET abstract_ja = ?1, abstract_ja_translated_at = ?2, abstract_ja_service = ?3 WHERE id = ?4",
        rusqlite::params![
            flattened,
            chrono::Utc::now().to_rfc3339(),
            service_name,
            paper_id
        ],
    )?;
    // Dashboard の出来事に「translate」として残す（記録できなくても訳は返す）
    let _ = conn.execute(
        "INSERT INTO activity_log (kind, paper_id, detail) VALUES ('translate', ?1, '要旨')",
        [paper_id],
    );

    Ok(Some(flattened))
}

/// 要旨翻訳に使うサービス名を設定から決める（知らない名前はエラー）。
fn abstract_service_name(
    service: &crate::config::TranslateService,
) -> Result<&'static str, TranslationError> {
    match service.service.as_str() {
        "google" => Ok("google"),
        "openailiked" => Ok("openailiked"),
        other => Err(TranslationError::InvalidInput(format!(
            "translate.abstract.service が不明です: {other}（google / openailiked）"
        ))),
    }
}

/// OpenAI 互換 API（chat/completions）で英語の要旨を日本語にする。
///
/// `prompt_format` が `translategemma` のときは、区切り記号つきの user のメッセージ 1 通に
/// 訳す。入力と出力の合計が 2,048 トークンまでのため、本文を [`TRANSLATEGEMMA_CHUNK_CHARS`]
/// ごとのかたまりに分けて順に送り、つないだ訳を返す。
async fn translate_openailiked(
    text: &str,
    service: &crate::config::TranslateService,
) -> Result<String, TranslationError> {
    let client = reqwest::Client::new();
    let url = format!("{}/chat/completions", service.base_url.trim_end_matches('/'));
    let mut translated = String::new();
    for chunk in chunks_for(service, text) {
        // 要求の組み立て（キーの有無もここで決める）。テストから直接確かめる。
        let request = build_chat_request(&client, &url, service, &chunk)?;
        let response: serde_json::Value = client
            .execute(request)
            .await
            .and_then(reqwest::Response::error_for_status)
            .map_err(|e: reqwest::Error| TranslationError::Service(e.to_string()))?
            .json()
            .await
            .map_err(|e: reqwest::Error| TranslationError::Service(e.to_string()))?;
        let piece = response["choices"][0]["message"]["content"]
            .as_str()
            .ok_or_else(|| TranslationError::Service(format!("応答に訳文がありません: {response}")))?;
        translated.push_str(piece);
    }
    Ok(translated)
}

/// 要旨訳が translateGemma の形かどうか（`translate.prompt_format`）。
#[must_use]
fn is_translategemma(service: &crate::config::TranslateService) -> bool {
    service.prompt_format == "translategemma"
}

/// translateGemma へ 1 通の要求を出すときの、1 かたあたりの文字数の上限。
///
/// 1,500 文字の英文は約 400 トークン。区切り記号（約 70 トークン）と出力 1,024 トークンを
/// 足しても vLLM の `max_model_len` 2,048 に収まる。
const TRANSLATEGEMMA_CHUNK_CHARS: usize = 1_500;
/// translateGemma へ 1 通の要求を出すときの出力の上限トークン。
const TRANSLATEGEMMA_MAX_TOKENS: u32 = 1024;

/// 訳文を 1 通ずつ送る（[`TRANSLATEGEMMA_CHUNK_CHARS`] ごと、それ以外はそのまま）。
fn chunks_for(service: &crate::config::TranslateService, text: &str) -> Vec<String> {
    if is_translategemma(service) {
        split_for_translategemma(text, TRANSLATEGEMMA_CHUNK_CHARS)
    } else {
        vec![text.to_string()]
    }
}

/// translateGemma へ送る 1 通の本文（`<<<source>>>` / `<<<target>>>` / `<<<text>>>` つき）。
///
/// system メッセージは付けない（vLLM が `Conversations must start with a user prompt.` で
/// 弾くため）。
#[must_use]
fn translategemma_content(chunk: &str, lang_in: &str, lang_out: &str) -> String {
    format!("<<<source>>>{lang_in}<<<target>>>{lang_out}<<<text>>>{chunk}")
}

/// 1 通の要求の JSON（テスト用・HTTP は呼ばない）。
fn build_chat_body(service: &crate::config::TranslateService, text: &str) -> serde_json::Value {
    if is_translategemma(service) {
        serde_json::json!({
            "model": service.model,
            "messages": [{"role": "user", "content": translategemma_content(text, "en", "ja")}],
            "max_tokens": TRANSLATEGEMMA_MAX_TOKENS,
            "temperature": 0,
        })
    } else {
        serde_json::json!({
            "model": service.model,
            "messages": [
                {"role": "system", "content": "Translate the user's academic abstract into natural Japanese. Output only the translation."},
                {"role": "user", "content": text},
            ],
        })
    }
}

/// API キー（`api_key_env` が空なら `None` = キー無しで送る）。
///
/// 名前があるのに値が空のときはエラー（設定の書き忘れに気づかせるため）。
fn api_key(service: &crate::config::TranslateService) -> Result<Option<String>, TranslationError> {
    if service.api_key_env.trim().is_empty() {
        return Ok(None);
    }
    let key = std::env::var(&service.api_key_env).unwrap_or_default();
    if key.trim().is_empty() {
        return Err(TranslationError::Service(format!(
            "環境変数 {} に API キーがありません",
            service.api_key_env
        )));
    }
    Ok(Some(key))
}

/// 送る直前の要求を作る（キーの有無もここで決める。まだ送らない）。
fn build_chat_request(
    client: &reqwest::Client,
    url: &str,
    service: &crate::config::TranslateService,
    text: &str,
) -> Result<reqwest::Request, TranslationError> {
    let mut req = client.post(url).json(&build_chat_body(service, text));
    if let Some(key) = api_key(service)? {
        req = req.bearer_auth(key);
    }
    req.build()
        .map_err(|e| TranslationError::Service(format!("要求を作れません: {e}")))
}

/// 本文を 1 通ずつ送れるように分ける。
///
/// 文の終わり（`. ` / `? ` / `! ` / 改行）で区切って積み上げ、`max_chars` を超える前に
/// 1 通として切り出す。1 文が `max_chars` より長いときは空白の位置で切る
/// （空白も無いときは文字で強制的に切る）。戻り値はすべて `max_chars` 以下。
#[must_use]
pub fn split_for_translategemma(text: &str, max_chars: usize) -> Vec<String> {
    let mut chunks: Vec<String> = Vec::new();
    let mut current = String::new();
    for sentence in split_sentences(text) {
        for piece in split_long_sentence(sentence, max_chars) {
            let piece = piece.trim();
            if piece.is_empty() {
                continue;
            }
            if !current.is_empty() && current.chars().count() + 1 + piece.chars().count() > max_chars
            {
                chunks.push(std::mem::take(&mut current));
            }
            if !current.is_empty() {
                current.push(' ');
            }
            current.push_str(piece);
        }
    }
    if !current.trim().is_empty() {
        chunks.push(current);
    }
    chunks
}

/// 文の終わりで切る（文末の空白は持ち越さない）。
fn split_sentences(text: &str) -> Vec<&str> {
    let mut out: Vec<&str> = Vec::new();
    let mut start = 0;
    for (i, ch) in text.char_indices() {
        let end = i + ch.len_utf8();
        let boundary = ch == '\n'
            || ((ch == '.' || ch == '?' || ch == '!')
                && text[end..]
                    .chars()
                    .next()
                    .is_none_or(char::is_whitespace));
        if boundary {
            if !text[start..end].trim().is_empty() {
                out.push(&text[start..end]);
            }
            start = end;
        }
    }
    if !text[start..].trim().is_empty() {
        out.push(&text[start..]);
    }
    out
}

/// 1 文が長すぎるときだけ、`max_chars` 以下の切り口へ割る。
fn split_long_sentence(sentence: &str, max_chars: usize) -> Vec<String> {
    if max_chars == 0 || sentence.chars().count() <= max_chars {
        return vec![sentence.to_string()];
    }
    let mut out: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut current_len = 0usize;
    for word in sentence.split_inclusive(char::is_whitespace) {
        let word_len = word.chars().count();
        if current_len > 0 && current_len + word_len > max_chars {
            out.push(std::mem::take(&mut current));
            current_len = 0;
        }
        // 空白の無い 1 語が長すぎるときは文字で強制的に切る
        if word_len > max_chars {
            for ch in word.chars() {
                if current_len + 1 > max_chars {
                    out.push(std::mem::take(&mut current));
                    current_len = 0;
                }
                current.push(ch);
                current_len += 1;
            }
            continue;
        }
        current.push_str(word);
        current_len += word_len;
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{
        abstract_service_name, build_chat_body, build_chat_request, chunks_for,
        split_for_translategemma,
    };
    use crate::config::TranslateService;
    use reqwest::header::AUTHORIZATION;

    /// 設定に従って要旨の翻訳サービスが選ばれる（HTTP は呼ばない）。
    #[test]
    fn abstract_service_follows_settings() {
        let pick = |name: &str| {
            abstract_service_name(&TranslateService {
                service: name.into(),
                ..TranslateService::default()
            })
        };
        assert_eq!(pick("google").unwrap(), "google");
        assert_eq!(pick("openailiked").unwrap(), "openailiked");
        assert!(pick("deepl").is_err());
    }

    /// 既定は dgx の vLLM で動く translateGemma（キー無し・区切り記号つきの形）。
    #[test]
    fn default_service_is_translategemma() {
        let service = TranslateService::default();
        assert_eq!(service.base_url, "http://192.168.100.26:8000/v1");
        assert_eq!(service.model, "translategemma-4b");
        assert_eq!(service.api_key_env, "");
        assert_eq!(service.prompt_format, "translategemma");
    }

    /// 短い要旨は 1 つのかたまりになる。
    #[test]
    fn short_abstract_stays_in_one_chunk() {
        let chunks = split_for_translategemma("We propose a new method. It works well.", 1_500);
        assert_eq!(chunks, ["We propose a new method. It works well."]);
    }

    /// 長い要旨は文の終わりで分かれ、どのかたまりも上限以下になる。
    #[test]
    fn long_abstract_splits_at_sentence_end() {
        let sentence = "The loss is defined as {v1} where {v2} denotes the learning rate. ";
        // 1,500 文字を超えるように同じ文を並べる
        let long = sentence.repeat(30);
        let chunks = split_for_translategemma(&long, 1_500);
        assert!(chunks.len() > 1, "1 つのかたまりに収まってはいけない");
        for chunk in &chunks {
            assert!(chunk.chars().count() <= 1_500, "{chunk}");
            assert!(chunk.ends_with('.'), "文の終わりで分かれるべき: {chunk}");
        }
        // 元の文はどのかたまりにも欠けていない（かたまり間で空白は落ちる）
        let joined: String = chunks.join(" ");
        assert_eq!(
            joined.split_whitespace().collect::<Vec<_>>(),
            long.split_whitespace().collect::<Vec<_>>()
        );
    }

    /// 1 文が長すぎるときは空白の位置で切る（切り口はすべて上限以下）。
    #[test]
    fn oversized_single_sentence_splits_at_whitespace() {
        let long = "a".repeat(80);
        let text = format!("{long} {long} {long}.");
        let chunks = split_for_translategemma(&text, 100);
        assert!(chunks.len() > 1);
        for chunk in &chunks {
            assert!(chunk.chars().count() <= 100, "{chunk}");
        }
        assert_eq!(
            chunks.join(" ").split_whitespace().collect::<Vec<_>>(),
            text.split_whitespace().collect::<Vec<_>>()
        );
    }

    /// translateGemma の要求は system が無く、区切り記号つきの user のメッセージ 1 つ。
    #[test]
    fn translategemma_body_has_no_system_message() {
        let body = build_chat_body(&TranslateService::default(), "Hello.");
        let messages = body["messages"].as_array().unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0]["role"], "user");
        assert_eq!(
            messages[0]["content"],
            "<<<source>>>en<<<target>>>ja<<<text>>>Hello."
        );
        assert_eq!(body["model"], "translategemma-4b");
        assert_eq!(body["max_tokens"], 1_024);
        assert_eq!(body["temperature"], 0);
    }

    /// `prompt_format` が空のときは今までどおりの指示文つきチャット。
    #[test]
    fn plain_chat_body_keeps_system_message() {
        let service = TranslateService {
            prompt_format: String::new(),
            ..TranslateService::default()
        };
        let body = build_chat_body(&service, "Hello.");
        let messages = body["messages"].as_array().unwrap();
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0]["role"], "system");
        assert_eq!(messages[1]["content"], "Hello.");
        // 分けない（1 通で全部送る）
        assert_eq!(chunks_for(&service, "Hello."), ["Hello."]);
    }

    /// `api_key_env` が空なら、送る前の要求に認証ヘッダが付かない（vLLM はキーを見ない）。
    #[test]
    fn empty_api_key_env_sends_without_authorization() {
        let service = TranslateService::default();
        assert_eq!(service.api_key_env, "");
        let request = build_chat_request(
            &reqwest::Client::new(),
            "http://192.168.100.26:8000/v1/chat/completions",
            &service,
            "Hello.",
        )
        .unwrap();
        assert!(request.headers().get(AUTHORIZATION).is_none());
    }

    /// `api_key_env` に名前があるのに値が空のときはエラー（設定の書き忘れに気づかせる）。
    #[test]
    fn named_but_empty_api_key_env_is_an_error() {
        let service = TranslateService {
            api_key_env: "SAI_PAPER_TEST_KEY_MISSING".into(),
            ..TranslateService::default()
        };
        let err = build_chat_request(
            &reqwest::Client::new(),
            "http://127.0.0.1:1/chat/completions",
            &service,
            "Hello.",
        )
        .unwrap_err();
        assert!(
            err.to_string()
                .contains("SAI_PAPER_TEST_KEY_MISSING"),
            "{err}"
        );
    }
}

