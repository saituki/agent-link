//! 要約付きの検索（`find --evidence`）。
//!
//! 検索の上位の結果それぞれに「その問いに関係する部分の要約」と「関連度 0〜10」を
//! LLM で付ける。LLM を余分に呼ぶので、指定したときだけ使う。
//!
//! - 塊ごとに chat API を 1 回呼ぶ（同時に `rag.evidence.max_concurrent` まで）。
//! - JSON が壊れていたら 1 回だけ頼み直す。それでも駄目なら、その塊は要約なしで残す。
//! - 関連度が `rag.evidence.cutoff` 未満のものを外し、関連度の高い順に並べる。
//! - 使ったトークン数を数えて返す（費用を利用者に見せるため）。

use futures_util::stream::{self, StreamExt};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};

use crate::config::Config;
use crate::retrieval::{FindRequest, Hit, RetrievalError, cite_for, find};

/// 要約付きの検索のエラー。
#[derive(Debug, thiserror::Error)]
pub enum EvidenceError {
    /// 検索（`find`）の失敗。
    #[error(transparent)]
    Retrieval(#[from] RetrievalError),
    /// チャット API・設定の失敗。
    #[error("evidence error: {0}")]
    Other(String),
}

/// このモジュールの結果。
pub type Result<T> = std::result::Result<T, EvidenceError>;

/// チャットの応答。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ChatReply {
    /// 応答の本文。
    pub content: String,
    /// 入力トークン数（`usage.prompt_tokens`）。
    pub prompt_tokens: u64,
    /// 出力トークン数（`usage.completion_tokens`）。
    pub completion_tokens: u64,
}

/// チャット API を呼ぶもの。本番は [`ChatCompletionsClient`]、テストでは偽物に差し替える。
#[async_trait::async_trait]
pub trait ChatClient: Send + Sync {
    /// system と user の 2 つのメッセージで 1 回呼ぶ。
    async fn chat(&self, system: &str, user: &str) -> Result<ChatReply>;
}

/// OpenAI 互換の chat/completions を呼ぶクライアント。
///
/// `translate_openailiked` と同じ書き方（`{base_url}/chat/completions` に `model` と
/// `messages` を POST し、`bearer_auth(環境変数のキー)` を付ける）。
#[derive(Debug, Clone)]
pub struct ChatCompletionsClient {
    client: reqwest::Client,
    endpoint: String,
    model: String,
    api_key_env: String,
}

impl ChatCompletionsClient {
    /// 設定 `rag.evidence` からクライアントを作る。
    ///
    /// # Errors
    ///
    /// `rag.evidence.model` が空のとき。
    pub fn from_config(config: &Config) -> Result<Self> {
        let evidence = &config.rag.evidence;
        if evidence.model.trim().is_empty() {
            return Err(EvidenceError::Other(
                "settings.json の rag.evidence.model を設定してください".into(),
            ));
        }
        Ok(Self {
            client: reqwest::Client::new(),
            endpoint: format!(
                "{}/chat/completions",
                evidence.base_url.trim_end_matches('/')
            ),
            model: evidence.model.clone(),
            api_key_env: evidence.api_key_env.clone(),
        })
    }
}

#[async_trait::async_trait]
impl ChatClient for ChatCompletionsClient {
    async fn chat(&self, system: &str, user: &str) -> Result<ChatReply> {
        let key = std::env::var(&self.api_key_env).unwrap_or_default();
        if key.trim().is_empty() {
            return Err(EvidenceError::Other(format!(
                "環境変数 {} に API キーがありません",
                self.api_key_env
            )));
        }
        let body = serde_json::json!({
            "model": self.model,
            "messages": [
                {"role": "system", "content": system},
                {"role": "user", "content": user},
            ],
        });
        let response: serde_json::Value = self
            .client
            .post(&self.endpoint)
            .bearer_auth(key)
            .json(&body)
            .send()
            .await
            .and_then(reqwest::Response::error_for_status)
            .map_err(|e| EvidenceError::Other(e.to_string()))?
            .json()
            .await
            .map_err(|e| EvidenceError::Other(e.to_string()))?;
        let content = response["choices"][0]["message"]["content"]
            .as_str()
            .map(str::to_string)
            .ok_or_else(|| EvidenceError::Other(format!("応答に本文がありません: {response}")))?;
        Ok(ChatReply {
            content,
            prompt_tokens: response["usage"]["prompt_tokens"].as_u64().unwrap_or(0),
            completion_tokens: response["usage"]["completion_tokens"].as_u64().unwrap_or(0),
        })
    }
}

/// 要約を付けた 1 件の結果。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvidenceHit {
    /// 元の検索結果。
    pub hit: Hit,
    /// そのまま report に貼れる引用（[`cite_for`] の結果）。
    pub cite: Option<String>,
    /// 問いに関係する部分の要約（要約できなかったときは None）。
    pub summary: Option<String>,
    /// 関連度（0-10、判定できなかったときは None）。
    pub relevance: Option<u8>,
}

/// 要約付きの検索の結果。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvidenceResult {
    /// 関連度の高い順の結果（関連度の無いものは最後）。
    pub hits: Vec<EvidenceHit>,
    /// 関連度が `cutoff` 未満で外した件数。
    pub dropped: usize,
    /// 使った入力トークンの合計。
    pub prompt_tokens: u64,
    /// 使った出力トークンの合計。
    pub completion_tokens: u64,
    /// 要約に失敗した件数（要約なしで残したもの）。
    pub failed: usize,
}

/// 検索の結果（要約なし / 要約付き）。
#[derive(Debug, Clone)]
pub enum SearchOutcome {
    /// 今までどおりの検索結果。
    Plain(Vec<Hit>),
    /// 要約付きの検索結果。
    Evidence(EvidenceResult),
}

/// 設定と `evidence` の指定に従って、要約なし / 要約付きの検索をする。
///
/// `with_evidence` が false のときは `chat` を一切呼ばない。
///
/// # Errors
///
/// `with_evidence` が true で `chat` が None のとき、または [`find_with_evidence`] の失敗。
pub async fn find_maybe_evidence(
    db: &Connection,
    config: &Config,
    req: FindRequest,
    with_evidence: bool,
    chat: Option<&dyn ChatClient>,
) -> Result<SearchOutcome> {
    if !with_evidence {
        return Ok(SearchOutcome::Plain(find(db, config, req).await?));
    }
    let chat = chat.ok_or_else(|| EvidenceError::Other("chat client が必要です".into()))?;
    Ok(SearchOutcome::Evidence(
        find_with_evidence(db, config, req, chat).await?,
    ))
}

/// 要約付きの検索をする。
///
/// 1. `find` で上位を取る（`top_k` は `rag.evidence.top_k`）。
/// 2. 塊ごとに chat API を 1 回呼ぶ（同時に `max_concurrent` まで）。
/// 3. JSON が壊れていたら 1 回だけ頼み直す。それでも駄目なら要約なしで残す。
/// 4. 関連度が `cutoff` 未満のものを外し、関連度の高い順に並べる。
/// 5. 使ったトークン数を数えて返す。
///
/// # Errors
///
/// 検索（`find`）の失敗。
pub async fn find_with_evidence(
    db: &Connection,
    config: &Config,
    req: FindRequest,
    chat: &dyn ChatClient,
) -> Result<EvidenceResult> {
    let evidence = &config.rag.evidence;
    let query = req.query.clone();
    let mut req = req;
    // 要約する件数は設定で決める（`find` 側の `per_paper` の並べ替えはそのまま効く）。
    req.top_k = Some(evidence.top_k);
    let hits = find(db, config, req).await?;

    let system = system_prompt(evidence.summary_chars);
    let concurrency = evidence.max_concurrent.max(1);
    let mut outcomes: Vec<(usize, ChunkOutcome)> =
        stream::iter(hits.iter().enumerate().map(|(index, hit)| {
            let system = system.clone();
            let user = user_prompt(&query, &hit.text);
            async move { (index, summarize(chat, &system, &user).await) }
        }))
        .buffer_unordered(concurrency)
        .collect()
        .await;
    // `buffer_unordered` は順不同なので元の順に戻す。
    outcomes.sort_by_key(|(index, _)| *index);

    let mut evidence_hits: Vec<EvidenceHit> = Vec::with_capacity(outcomes.len());
    let mut dropped = 0usize;
    let mut prompt_tokens = 0u64;
    let mut completion_tokens = 0u64;
    let mut failed = 0usize;
    for (index, outcome) in outcomes {
        prompt_tokens += outcome.prompt_tokens;
        completion_tokens += outcome.completion_tokens;
        if outcome.failed {
            failed += 1;
        }
        if let Some(relevance) = outcome.relevance {
            if relevance < evidence.cutoff {
                dropped += 1;
                continue;
            }
        }
        let hit = hits[index].clone();
        evidence_hits.push(EvidenceHit {
            cite: cite_for(&hit),
            hit,
            summary: outcome.summary,
            relevance: outcome.relevance,
        });
    }
    // 関連度の高い順（`Option` の順序は None が最小なので、降順で None が最後に来る）。
    evidence_hits.sort_by(|a, b| b.relevance.cmp(&a.relevance));

    Ok(EvidenceResult {
        hits: evidence_hits,
        dropped,
        prompt_tokens,
        completion_tokens,
        failed,
    })
}

/// 1 つの塊を要約した結果（内部用）。
struct ChunkOutcome {
    summary: Option<String>,
    relevance: Option<u8>,
    prompt_tokens: u64,
    completion_tokens: u64,
    failed: bool,
}

/// 1 つの塊を要約する。JSON が壊れていたら 1 回だけ頼み直す。
async fn summarize(chat: &dyn ChatClient, system: &str, user: &str) -> ChunkOutcome {
    let mut prompt_tokens = 0u64;
    let mut completion_tokens = 0u64;
    for _attempt in 0..2 {
        let reply = match chat.chat(system, user).await {
            Ok(reply) => reply,
            // 通信・認証の失敗はやり直さない（無駄な呼び出しを増やさない）。
            Err(_) => {
                return ChunkOutcome {
                    summary: None,
                    relevance: None,
                    prompt_tokens,
                    completion_tokens,
                    failed: true,
                };
            }
        };
        prompt_tokens += reply.prompt_tokens;
        completion_tokens += reply.completion_tokens;
        if let Some((summary, relevance)) = parse_reply(&reply.content) {
            return ChunkOutcome {
                summary,
                relevance,
                prompt_tokens,
                completion_tokens,
                failed: false,
            };
        }
    }
    ChunkOutcome {
        summary: None,
        relevance: None,
        prompt_tokens,
        completion_tokens,
        failed: true,
    }
}

/// 塊を要約する system プロンプト。
fn system_prompt(summary_chars: usize) -> String {
    format!(
        "You are given a question and an excerpt from a paper. Summarize only the parts of the excerpt that help answer the question, in Japanese, within {summary_chars} characters. Keep numbers and formulas as they are. If irrelevant, return an empty summary. Rate relevance from 0 to 10. Return JSON: {{\"summary\": \"...\", \"relevance\": 0}}"
    )
}

/// 問いと塊を渡す user プロンプト。
fn user_prompt(query: &str, text: &str) -> String {
    format!("Question: {query}\n\nExcerpt:\n{text}")
}

/// LLM の応答から `{"summary": "...", "relevance": 0}` を取り出す。
///
/// 前後に説明やコードフェンスがあっても、最初の `{` から最後の `}` までを JSON とみなす。
/// `summary` と `relevance` の両方が読めなかったときは None（壊れた応答として扱う）。
fn parse_reply(content: &str) -> Option<(Option<String>, Option<u8>)> {
    let start = content.find('{')?;
    let end = content.rfind('}')?;
    if end < start {
        return None;
    }
    let value: serde_json::Value = serde_json::from_str(&content[start..=end]).ok()?;
    let summary = value
        .get("summary")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    // 10 を超える値は 10 に丸める。
    let relevance = value
        .get("relevance")
        .and_then(serde_json::Value::as_u64)
        .map(|r| r.min(10) as u8);
    if summary.is_none() && relevance.is_none() {
        return None;
    }
    Some((summary, relevance))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    /// `user` に含まれる語で応答を選ぶ偽のチャット。呼ばれた回数を数える。
    ///
    /// 語に空文字を書くと、どんな `user` にも当たるので、単純な順番待ちの列として使える。
    struct KeyedChat {
        replies: Mutex<Vec<(String, Result<ChatReply>)>>,
        calls: Arc<Mutex<Vec<(String, String)>>>,
    }

    impl KeyedChat {
        fn new(pairs: Vec<(&str, Result<ChatReply>)>) -> Self {
            Self {
                replies: Mutex::new(
                    pairs
                        .into_iter()
                        .map(|(needle, reply)| (needle.to_string(), reply))
                        .collect(),
                ),
                calls: Arc::new(Mutex::new(Vec::new())),
            }
        }

        fn calls(&self) -> Arc<Mutex<Vec<(String, String)>>> {
            Arc::clone(&self.calls)
        }
    }

    #[async_trait::async_trait]
    impl ChatClient for KeyedChat {
        async fn chat(&self, system: &str, user: &str) -> Result<ChatReply> {
            self.calls
                .lock()
                .unwrap()
                .push((system.to_string(), user.to_string()));
            let mut replies = self.replies.lock().unwrap();
            let position = replies
                .iter()
                .position(|(needle, _)| user.contains(needle.as_str()));
            match position {
                Some(index) => replies.remove(index).1,
                None => Err(EvidenceError::Other("応答がありません".into())),
            }
        }
    }

    fn reply(content: &str, prompt: u64, completion: u64) -> Result<ChatReply> {
        Ok(ChatReply {
            content: content.to_string(),
            prompt_tokens: prompt,
            completion_tokens: completion,
        })
    }

    // ---- parse_reply ----

    /// JSON の前後に説明やコードフェンスがあっても読める。
    #[test]
    fn parse_reply_accepts_code_fence_and_extra_text() {
        let parsed =
            parse_reply("Here you go:\n```json\n{\"summary\": \"要約\", \"relevance\": 8}\n```")
                .unwrap();
        assert_eq!(parsed.0.as_deref(), Some("要約"));
        assert_eq!(parsed.1, Some(8));
    }

    /// 空の要約は None にし、関連度だけ残す。
    #[test]
    fn parse_reply_treats_empty_summary_as_none() {
        let parsed = parse_reply("{\"summary\": \"  \", \"relevance\": 3}").unwrap();
        assert_eq!(parsed.0, None);
        assert_eq!(parsed.1, Some(3));
    }

    /// 11 以上は 10 に丸める。
    #[test]
    fn parse_reply_clamps_relevance() {
        let parsed = parse_reply("{\"summary\": \"x\", \"relevance\": 99}").unwrap();
        assert_eq!(parsed.1, Some(10));
    }

    /// JSON でない応答は壊れた扱い。
    #[test]
    fn parse_reply_rejects_non_json() {
        assert!(parse_reply("I cannot answer.").is_none());
        assert!(parse_reply("{}").is_none());
    }

    // ---- summarize ----

    /// 1 回目が壊れていても、2 回目で読めれば要約を付ける（呼び出しは 2 回）。
    #[tokio::test]
    async fn summarize_retries_once_on_broken_json() {
        let fake = KeyedChat::new(vec![
            ("", reply("not json", 10, 1)),
            (
                "",
                reply("{\"summary\": \"要約\", \"relevance\": 7}", 20, 2),
            ),
        ]);
        let calls = fake.calls();
        let outcome = summarize(&fake, "sys", "user").await;
        assert_eq!(calls.lock().unwrap().len(), 2);
        assert_eq!(outcome.summary.as_deref(), Some("要約"));
        assert_eq!(outcome.relevance, Some(7));
        assert!(!outcome.failed);
        // 2 回分のトークンを合計する。
        assert_eq!(outcome.prompt_tokens, 30);
        assert_eq!(outcome.completion_tokens, 3);
    }

    /// 2 回とも壊れていたら、要約なしで残し、失敗として数える（呼び出しは 2 回）。
    #[tokio::test]
    async fn summarize_keeps_chunk_without_summary_when_json_stays_broken() {
        let fake = KeyedChat::new(vec![
            ("", reply("bad", 1, 1)),
            ("", reply("still bad", 1, 1)),
        ]);
        let calls = fake.calls();
        let outcome = summarize(&fake, "sys", "user").await;
        assert_eq!(calls.lock().unwrap().len(), 2);
        assert!(outcome.summary.is_none());
        assert!(outcome.relevance.is_none());
        assert!(outcome.failed);
    }

    /// 通信の失敗はやり直さず、失敗として数える（呼び出しは 1 回）。
    #[tokio::test]
    async fn summarize_does_not_retry_chat_errors() {
        let fake = KeyedChat::new(vec![("", Err(EvidenceError::Other("network".into())))]);
        let calls = fake.calls();
        let outcome = summarize(&fake, "sys", "user").await;
        assert_eq!(calls.lock().unwrap().len(), 1);
        assert!(outcome.failed);
    }

    // ---- find_with_evidence（in-memory の DB を使う） ----

    /// word モードの検索ができる最小のスキーマを作る。
    fn test_conn() -> Connection {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "PRAGMA foreign_keys = ON;
             CREATE TABLE papers (
               id INTEGER PRIMARY KEY,
               title TEXT,
               published_at TEXT
             );
             CREATE TABLE chunks (
               id INTEGER PRIMARY KEY,
               source_kind TEXT NOT NULL,
               source_ref TEXT NOT NULL,
               paper_id INTEGER REFERENCES papers(id),
               ord INTEGER NOT NULL,
               text TEXT NOT NULL,
               char_start INTEGER,
               char_end INTEGER,
               strategy TEXT NOT NULL,
               created_at TEXT NOT NULL,
               index_name TEXT NOT NULL DEFAULT 'default',
               section TEXT
             );
             CREATE VIRTUAL TABLE chunk_fts USING fts5(
               text,
               content='chunks',
               content_rowid='id',
               tokenize='unicode61 remove_diacritics 2'
             );
             CREATE TRIGGER chunks_fts_insert AFTER INSERT ON chunks BEGIN
               INSERT INTO chunk_fts(rowid, text) VALUES (new.id, new.text);
             END;
             CREATE TABLE paper_extracts (
               paper_id TEXT PRIMARY KEY,
               md_path TEXT
             );",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO papers (id, title, published_at) VALUES (1, 'P1', '2024-01-01T00:00:00+00:00')",
            [],
        )
        .unwrap();
        conn
    }

    fn seed(conn: &Connection, id: i64, text: &str) {
        let now = chrono::Utc::now().to_rfc3339();
        conn.execute(
            "INSERT INTO chunks (id, source_kind, source_ref, paper_id, ord, text, strategy, created_at, index_name)
             VALUES (?1, 'body', ?2, 1, ?1, ?3, 'sentence', ?4, 'default')",
            rusqlite::params![id, format!("chunk{id}"), text, now],
        )
        .unwrap();
    }

    /// word モードで検索するテスト用の設定。
    fn word_config(cutoff: u8) -> Config {
        let mut config = Config::default();
        config.rag.default_mode = "word".into();
        // 1 本の論文に偏らないよう上限は外す。
        config.rag.per_paper = 0;
        config.rag.evidence.cutoff = cutoff;
        config.rag.evidence.top_k = 10;
        config.rag.evidence.max_concurrent = 4;
        config
    }

    /// 関連度が cutoff 未満のものを外し、高い順に並べ、トークンを合計する。
    #[tokio::test]
    async fn find_with_evidence_drops_low_relevance_and_orders() {
        let conn = test_conn();
        seed(&conn, 1, "alpha marker one");
        seed(&conn, 2, "alpha marker two");
        seed(&conn, 3, "alpha marker three");
        let config = word_config(3);
        // 3 件のうち関連度 2 は外れ、8 と 5 が残る（高い順）。
        let fake = KeyedChat::new(vec![
            (
                "marker one",
                reply("{\"summary\": \"s1\", \"relevance\": 5}", 100, 10),
            ),
            (
                "marker two",
                reply("{\"summary\": \"s2\", \"relevance\": 2}", 200, 20),
            ),
            (
                "marker three",
                reply("{\"summary\": \"s3\", \"relevance\": 8}", 300, 30),
            ),
        ]);
        let request = FindRequest {
            query: "alpha".into(),
            ..FindRequest::default()
        };
        let result = find_with_evidence(&conn, &config, request, &fake)
            .await
            .unwrap();
        assert_eq!(result.hits.len(), 2);
        assert_eq!(result.dropped, 1);
        assert_eq!(result.hits[0].relevance, Some(8));
        assert_eq!(result.hits[0].summary.as_deref(), Some("s3"));
        assert_eq!(result.hits[1].relevance, Some(5));
        // トークンは全件（外した分も）の合計。
        assert_eq!(result.prompt_tokens, 600);
        assert_eq!(result.completion_tokens, 60);
        assert_eq!(result.failed, 0);
        // cite は `cite_for` の結果。
        assert_eq!(result.hits[0].cite.as_deref(), Some("[#1]"));
    }

    /// 関連度が無い結果は最後に置く（外さない）。
    #[tokio::test]
    async fn find_with_evidence_puts_unknown_relevance_last() {
        let conn = test_conn();
        seed(&conn, 1, "beta marker one");
        seed(&conn, 2, "beta marker two");
        let config = word_config(1);
        let fake = KeyedChat::new(vec![
            (
                "marker one",
                reply("{\"summary\": \"s1\", \"relevance\": 6}", 1, 1),
            ),
            // 壊れた応答を 2 回返し、要約なし（relevance None）で残す。
            ("marker two", reply("bad", 1, 1)),
            ("marker two", reply("bad", 1, 1)),
        ]);
        let request = FindRequest {
            query: "beta".into(),
            ..FindRequest::default()
        };
        let result = find_with_evidence(&conn, &config, request, &fake)
            .await
            .unwrap();
        assert_eq!(result.hits.len(), 2);
        assert_eq!(result.dropped, 0);
        assert_eq!(result.hits[0].relevance, Some(6));
        assert_eq!(result.hits[1].relevance, None);
        assert_eq!(result.hits[1].summary, None);
        assert_eq!(result.failed, 1);
    }

    /// `evidence` を付けない検索は偽のチャットを呼ばない（AC3）。
    #[tokio::test]
    async fn plain_search_does_not_call_chat() {
        let conn = test_conn();
        seed(&conn, 1, "gamma topic");
        let config = word_config(3);
        let fake = KeyedChat::new(vec![("", reply("{}", 1, 1))]);
        let calls = fake.calls();
        let request = FindRequest {
            query: "gamma".into(),
            ..FindRequest::default()
        };
        let outcome = find_maybe_evidence(&conn, &config, request, false, Some(&fake))
            .await
            .unwrap();
        assert!(matches!(outcome, SearchOutcome::Plain(_)));
        assert!(calls.lock().unwrap().is_empty());
    }

    /// `model` が空のときはクライアントを作れない（AC2）。
    #[test]
    fn client_requires_model() {
        let config = Config::default();
        let error = ChatCompletionsClient::from_config(&config).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("settings.json の rag.evidence.model を設定してください")
        );
    }
}
