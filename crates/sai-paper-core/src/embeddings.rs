//! 埋め込みクライアントと、チャンク埋め込みの取得・保存。
//!
//! 外部の埋め込みサーバ（Ollama、OpenAI 互換）から埋め込みベクトルを取得し、
//! SQLite に保存する。複数のモデルをサポートし、モデルメタデータは `embedding_models`
//! テーブルに登録される。

use reqwest::Client;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use thiserror::Error;

/// 埋め込み操作のエラー。
#[derive(Debug, Error)]
pub enum EmbeddingError {
    /// HTTP リクエストが失敗した。
    #[error("HTTP request failed: {0}")]
    Request(#[from] reqwest::Error),
    /// レスポンスのデコードに失敗した。
    #[error("response decode failed: {0}")]
    Decode(String),
    /// SQLite 操作に失敗した。
    #[error("database error: {0}")]
    Database(#[from] rusqlite::Error),
    /// 埋め込み次元が設定と異なる。
    #[error("dimension mismatch: expected {expected}, got {actual}")]
    DimensionMismatch {
        /// 期待される次元数。
        expected: usize,
        /// 実際の次元数。
        actual: usize,
    },
    /// その他のエラー。
    #[error("embedding error: {0}")]
    Other(String),
}

/// 埋め込み操作の結果型。
pub type Result<T> = std::result::Result<T, EmbeddingError>;

/// OpenAI 互換の埋め込みレスポンス。
#[derive(Deserialize)]
struct OpenAIEmbeddingResponse {
    data: Vec<OpenAIEmbedding>,
}

#[derive(Deserialize)]
struct OpenAIEmbedding {
    embedding: Vec<f32>,
}

/// 埋め込みクライアント。
///
/// OpenAI 互換のエンドポイント（Ollama `/v1/embeddings` など）に対して
/// リクエストを送信し、埋め込みベクトルを取得する。
#[derive(Debug, Clone)]
pub struct EmbedClient {
    client: Client,
    endpoint: String,
    model: String,
    dim: usize,
    /// API キーの環境変数名（空ならキーを使わない）。
    api_key_env: String,
    /// 埋め込みに渡す文字の先頭に「タイトル（年）— 章」を付けるか（索引の設定）。
    embed_with_meta: bool,
}

/// 埋め込みリクエストペイロード。
#[derive(Serialize)]
struct EmbeddingRequest {
    model: String,
    input: Vec<String>,
}

impl EmbedClient {
    /// 埋め込みクライアントを、設定から初期化する。
    ///
    /// # Errors
    ///
    /// 設定が存在しない、または設定値が不正な場合。
    pub fn from_config(cfg: &crate::config::Config) -> Result<Self> {
        let index = cfg.rag.index(None).map_err(EmbeddingError::Other)?;
        Ok(Self::for_index(&index))
    }

    /// 索引の埋め込みモデルでクライアントを作る。
    #[must_use]
    pub fn for_index(index: &crate::config::RagIndex) -> Self {
        Self {
            client: Client::new(),
            endpoint: index.embed_endpoint.clone(),
            model: index.embed_model.clone(),
            dim: index.embed_dim as usize,
            api_key_env: index.embed_api_key_env.clone(),
            embed_with_meta: index.embed_with_meta,
        }
    }

    /// 埋め込みに先頭のメタ（タイトル・年・章）を付けるかを返す。
    #[must_use]
    pub fn embed_with_meta(&self) -> bool {
        self.embed_with_meta
    }

    /// 先頭メタを付けるかを変えたクライアントを返す（主にテスト用）。
    #[must_use]
    pub fn with_embed_with_meta(mut self, value: bool) -> Self {
        self.embed_with_meta = value;
        self
    }

    /// モデル名を返す。
    #[must_use]
    pub fn model(&self) -> &str {
        &self.model
    }

    /// 次元数を返す。
    #[must_use]
    pub fn dim(&self) -> usize {
        self.dim
    }

    /// テキストのリストから埋め込みベクトルを取得する。
    ///
    /// 最大 `batch_size` のテキストを一度に送信する。
    /// 入力の順序を保持して出力を返す。
    ///
    /// 429（レート制限）と 5xx エラーは指数バックオフで再送する。
    /// Retry-After ヘッダがあればそれに従う。
    ///
    /// # Errors
    ///
    /// - ネットワークエラー
    /// - レスポンスパースエラー
    /// - 次元不一致
    /// - 認証エラー（401/403）
    pub async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        const MAX_RETRIES: u8 = 5;
        const MAX_RETRY_SECS: u64 = 10;

        if texts.is_empty() {
            return Ok(Vec::new());
        }

        let request = EmbeddingRequest {
            model: self.model.clone(),
            input: texts.to_vec(),
        };

        let mut attempts: u8 = 0;
        loop {
            let mut req = self.client.post(&self.endpoint).json(&request);

            // 環境変数から API キーを取得して認証ヘッダを付ける
            if !self.api_key_env.is_empty() {
                if let Ok(api_key) = std::env::var(&self.api_key_env) {
                    req = req.header("Authorization", format!("Bearer {}", api_key));
                }
            }

            let response = match req.send().await {
                Ok(resp) => resp,
                // ネットワークエラーは一時的な障害として再送する
                Err(e) => {
                    attempts += 1;
                    if attempts > MAX_RETRIES {
                        return Err(EmbeddingError::Request(e));
                    }
                    let wait = 2u64.pow(attempts as u32).min(MAX_RETRY_SECS);
                    tokio::time::sleep(Duration::from_secs(wait)).await;
                    continue;
                }
            };

            let status = response.status();

            // 429（レート制限）と 5xx は指数バックオフで再送
            if status.as_u16() == 429 || (status.as_u16() >= 500 && status.as_u16() < 600) {
                attempts += 1;
                if attempts > MAX_RETRIES {
                    return Err(EmbeddingError::Decode(format!(
                        "HTTP {} (rate limited / server error). 再実行で続きから処理します。",
                        status
                    )));
                }
                let retry_secs = response
                    .headers()
                    .get(reqwest::header::RETRY_AFTER)
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| v.parse::<u64>().ok())
                    .unwrap_or_else(|| 2u64.pow(attempts as u32).min(MAX_RETRY_SECS));
                tokio::time::sleep(Duration::from_secs(retry_secs)).await;
                continue;
            }

            // 認証エラーは環境変数名をメッセージに含める
            if status.as_u16() == 401 || status.as_u16() == 403 {
                return Err(EmbeddingError::Decode(format!(
                    "HTTP {} (認証エラー)。環境変数 {} を設定してください。",
                    status, self.api_key_env
                )));
            }

            // 入力トークン数超過エラー（400）
            if status.as_u16() == 400 {
                let error_text = response.text().await.unwrap_or_default();
                if error_text.contains("exceeding the model maximum") {
                    return Err(EmbeddingError::Decode(format!(
                        "HTTP 400: 入力が長すぎます（モデル制限超過）。チャンク化の再構築が必要かもしれません。詳細: {}",
                        error_text
                    )));
                }
                return Err(EmbeddingError::Decode(format!("HTTP 400: {}", error_text)));
            }

            if !status.is_success() {
                return Err(EmbeddingError::Decode(format!(
                    "HTTP {}: {}",
                    status,
                    response.text().await.unwrap_or_default()
                )));
            }

            let embed_response: OpenAIEmbeddingResponse = response.json().await?;

            // 次元チェック
            if !embed_response.data.is_empty() {
                let actual_dim = embed_response.data[0].embedding.len();
                if actual_dim != self.dim {
                    return Err(EmbeddingError::DimensionMismatch {
                        expected: self.dim,
                        actual: actual_dim,
                    });
                }
            }

            // 入力順を保持して返す
            let embeddings: Vec<Vec<f32>> = embed_response
                .data
                .into_iter()
                .map(|e| e.embedding)
                .collect();
            return Ok(embeddings);
        }
    }
}

/// 埋め込みの先頭に付ける「タイトル（年）— 章」を作る。
/// 無い項目は省く。どれも無ければ `None`。
#[must_use]
pub fn embedding_meta_prefix(
    title: Option<&str>,
    year: Option<i64>,
    section: Option<&str>,
) -> Option<String> {
    let mut head = String::new();
    if let Some(title) = title.filter(|t| !t.trim().is_empty()) {
        head.push_str(title.trim());
    }
    if let Some(year) = year {
        head.push_str(&format!("（{year}）"));
    }
    if let Some(section) = section.filter(|s| !s.trim().is_empty()) {
        if head.is_empty() {
            head.push_str(section.trim());
        } else if head.ends_with('）') {
            // 「タイトル（年）— 章」の形（共有契約）。
            head.push_str("— ");
            head.push_str(section.trim());
        } else {
            head.push_str(" — ");
            head.push_str(section.trim());
        }
    }
    if head.is_empty() { None } else { Some(head) }
}

/// 埋め込みベクトルを計算するもの。本番は `EmbedClient`、テストでは偽物に差し替える。
#[async_trait::async_trait]
pub trait Embedder: Send + Sync {
    /// テキストのリストから埋め込みベクトルを取得する。
    async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>>;
    /// モデル名。
    fn model(&self) -> &str;
    /// エンドポイント。
    fn endpoint(&self) -> &str;
    /// 次元数。
    fn dim(&self) -> usize;
    /// 埋め込みに先頭のメタを付けるか。
    fn embed_with_meta(&self) -> bool;
}

#[async_trait::async_trait]
impl Embedder for EmbedClient {
    async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        EmbedClient::embed(self, texts).await
    }

    fn model(&self) -> &str {
        &self.model
    }

    fn endpoint(&self) -> &str {
        &self.endpoint
    }

    fn dim(&self) -> usize {
        self.dim
    }

    fn embed_with_meta(&self) -> bool {
        self.embed_with_meta
    }
}

/// f32 ベクトルを little-endian バイナリに変換する。
#[must_use]
pub fn to_blob(vec: &[f32]) -> Vec<u8> {
    vec.iter().flat_map(|f| f.to_le_bytes()).collect()
}

/// little-endian バイナリを f32 ベクトルに変換する。
pub fn from_blob(blob: &[u8]) -> Result<Vec<f32>> {
    if blob.len() % 4 != 0 {
        return Err(EmbeddingError::Other(
            "blob size must be a multiple of 4 bytes".to_string(),
        ));
    }

    let mut result = Vec::with_capacity(blob.len() / 4);
    for chunk in blob.chunks_exact(4) {
        let bytes = [chunk[0], chunk[1], chunk[2], chunk[3]];
        result.push(f32::from_le_bytes(bytes));
    }

    Ok(result)
}

/// モデルを DB に登録し、model_id を返す。
/// 既に登録済みならそのメタデータを返す。
pub fn register_model(conn: &Connection, client: &dyn Embedder) -> Result<i64> {
    let now = chrono::Utc::now().to_rfc3339();

    // 既存レコードをチェック
    let model_id: Option<i64> = conn
        .query_row(
            "SELECT id FROM embedding_models WHERE name = ?1",
            [client.model()],
            |row| row.get(0),
        )
        .ok();

    if let Some(id) = model_id {
        return Ok(id);
    }

    // 新規登録
    conn.execute(
        "INSERT INTO embedding_models (name, endpoint, dim, created_at) VALUES (?1, ?2, ?3, ?4)",
        rusqlite::params![client.model(), client.endpoint(), client.dim() as i64, now],
    )?;

    Ok(conn.last_insert_rowid())
}

/// 未埋め込みチャンクをバッチで埋め込み、`chunk_embeddings` に保存する。
///
/// 指定されたクライアントのモデルに対して、まだ埋め込みされていないチャンクを
/// 取得し、バッチで埋め込みを生成してデータベースに保存する。
/// 既にそのモデルで埋め込まれたチャンクはスキップされる。
///
/// HTTP 400（トークン超過）が出た場合、該当テキストを 60% に切り詰めて再試行する（最大 3 回）。
/// 3 回とも失敗したチャンクのみをスキップする。
///
/// # Arguments
///
/// * `conn` - SQLite コネクション
/// * `client` - 埋め込みクライアント
/// * `index_name` - 埋め込む索引の名前
/// * `batch_size` - バッチサイズ（1バッチで何個のチャンクを一度に埋め込むか）
///
/// # Returns
///
/// 埋め込まれたチャンク数。
///
/// # Errors
///
/// - SQLite 操作が失敗した場合
/// - 埋め込みサーバへのリクエストが失敗した場合
/// - テキスト取得やベクトル保存に失敗した場合
pub async fn embed_pending(
    conn: &Connection,
    client: &dyn Embedder,
    index_name: &str,
    batch_size: usize,
) -> Result<i64> {
    // 登録済みモデルを確認
    let mut stmt = conn.prepare("SELECT id, name FROM embedding_models")?;
    let registered: Vec<(i64, String)> = stmt
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<std::result::Result<Vec<_>, _>>()?;

    // モデル ID を取得（なければ登録）
    let model_id = if let Some((id, _)) = registered.iter().find(|(_, name)| name == client.model())
    {
        *id
    } else {
        register_model(conn, client)?
    };

    // 埋め込みを必要とするチャンク（このモデルでまだ埋め込まれていない）を取得
    // 注: chunks テーブルの主キー列は id であり、chunk_id ではない
    // 対象はこの索引（index_name）の断片だけ（段階 7）。
    let mut stmt = conn.prepare(
        "SELECT id FROM chunks
         WHERE index_name = ?2 AND NOT EXISTS (
            SELECT 1 FROM chunk_embeddings ce
            WHERE ce.chunk_id = chunks.id AND ce.model_id = ?1
         )",
    )?;

    let chunk_ids: Vec<i64> = stmt
        .query_map(rusqlite::params![model_id, index_name], |row| row.get(0))?
        .collect::<std::result::Result<Vec<_>, _>>()?;

    if chunk_ids.is_empty() {
        return Ok(0);
    }

    // バッチで埋め込みを生成
    let mut embedded_count = 0i64;
    let mut skipped_count = 0i64;
    let embed_with_meta = client.embed_with_meta();

    for batch in chunk_ids.chunks(batch_size) {
        // テキストを取得。`embed_with_meta` のときだけ先頭にメタを付ける（chunks.text は変えない）。
        let texts: Vec<String> = batch
            .iter()
            .map(|chunk_id| {
                conn.query_row(
                    "SELECT c.text, c.section, p.title,
                            CAST(SUBSTR(p.published_at, 1, 4) AS INTEGER)
                       FROM chunks c
                       LEFT JOIN papers p ON p.id = c.paper_id
                      WHERE c.id = ?1",
                    [chunk_id],
                    |row| {
                        let text: String = row.get(0)?;
                        if !embed_with_meta {
                            return Ok(text);
                        }
                        let section: Option<String> = row.get(1)?;
                        let title: Option<String> = row.get(2)?;
                        let year: Option<i64> = row.get(3)?;
                        Ok(match embedding_meta_prefix(title.as_deref(), year, section.as_deref()) {
                            Some(prefix) => format!("{}\n{}", prefix, text),
                            None => text,
                        })
                    },
                )
            })
            .collect::<std::result::Result<Vec<_>, _>>()?;

        // 埋め込みを取得
        match client.embed(&texts).await {
            Ok(embeddings) => {
                // DB に保存
                for (chunk_id, embedding) in batch.iter().zip(embeddings.iter()) {
                    let blob = to_blob(embedding);
                    conn.execute(
                        "INSERT INTO chunk_embeddings (chunk_id, model_id, vec) VALUES (?1, ?2, ?3)",
                        rusqlite::params![chunk_id, model_id, blob],
                    )?;
                    embedded_count += 1;
                }
            }
            Err(e) => {
                // トークン数超過エラーの場合は、バッチ内の各チャンクを個別に再試行
                if let EmbeddingError::Decode(msg) = &e {
                    if msg.contains("exceeding the model maximum") {
                        // バッチ内の各チャンクを個別に処理
                        for (idx, chunk_id) in batch.iter().enumerate() {
                            let original_text = texts[idx].clone();
                            let mut truncated_text = original_text.clone();
                            let mut retries = 0;
                            let max_retries = 3;

                            // ponytail: 先頭部分だけのベクトルになる。上限に収まるチャンク分割が効いていれば発生しないはず。
                            loop {
                                match client.embed(&[truncated_text.clone()]).await {
                                    Ok(embeddings) => {
                                        if let Some(embedding) = embeddings.first() {
                                            let blob = to_blob(embedding);
                                            conn.execute(
                                                "INSERT INTO chunk_embeddings (chunk_id, model_id, vec) VALUES (?1, ?2, ?3)",
                                                rusqlite::params![chunk_id, model_id, blob],
                                            )?;
                                            embedded_count += 1;
                                        }
                                        break;
                                    }
                                    Err(ref retry_err) => {
                                        retries += 1;
                                        if retries >= max_retries {
                                            // 3 回失敗したのでスキップ
                                            eprintln!(
                                                "警告：チャンク {} は {} 回の再試行後、埋め込みができませんでした。スキップします。",
                                                chunk_id, max_retries
                                            );
                                            skipped_count += 1;
                                            break;
                                        }
                                        // テキストを 60% に切り詰める
                                        let chars: Vec<char> = truncated_text.chars().collect();
                                        let new_len = ((chars.len() as f64) * 0.6).ceil() as usize;
                                        if new_len == 0 {
                                            eprintln!(
                                                "警告：チャンク {} は短すぎて処理できません。スキップします。詳細: {}",
                                                chunk_id, retry_err
                                            );
                                            skipped_count += 1;
                                            break;
                                        }
                                        truncated_text = chars[..new_len].iter().collect();
                                    }
                                }
                            }
                        }
                        continue;
                    }
                }

                // その他のエラーの場合、既に保存済みのチャンク数を含めてエラーを返す
                if embedded_count > 0 {
                    eprintln!(
                        "埋め込みエラー：{} 件まで埋め込み済み。再実行で続きから処理します。詳細: {}",
                        embedded_count, e
                    );
                }
                return Err(e);
            }
        }
    }

    if skipped_count > 0 {
        eprintln!(
            "埋め込み完了：{} 件埋め込み、{} 件スキップ",
            embedded_count, skipped_count
        );
    }

    Ok(embedded_count)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_to_blob_round_trip() {
        let original = vec![1.0, 2.5, -3.14, 0.0];
        let blob = to_blob(&original);
        let recovered = from_blob(&blob).unwrap();

        assert_eq!(original.len(), recovered.len());
        for (a, b) in original.iter().zip(recovered.iter()) {
            assert!((a - b).abs() < 1e-6);
        }
    }

    #[test]
    fn test_from_blob_invalid_length() {
        let invalid = vec![0u8; 3]; // not a multiple of 4
        assert!(from_blob(&invalid).is_err());
    }

    #[test]
    fn test_to_blob_little_endian() {
        let vec = vec![1.0_f32];
        let blob = to_blob(&vec);
        let bytes = 1.0_f32.to_le_bytes();
        assert_eq!(&blob, &bytes.to_vec());
    }

    #[test]
    fn test_embed_dimension_validation() {
        // テストはスタブサーバが必要なため、ここでは型チェックのみ
        let client = EmbedClient {
            client: Client::new(),
            endpoint: "http://localhost:11434/v1/embeddings".to_string(),
            model: "test-model".to_string(),
            dim: 768,
            api_key_env: String::new(),
            embed_with_meta: false,
        };
        assert_eq!(client.dim(), 768);
        assert_eq!(client.model(), "test-model");
    }

    #[test]
    fn test_cosine_similarity() {
        // 単位ベクトル (1, 0) と (0, 1) の類似度は 0
        let a = vec![1.0, 0.0];
        let b = vec![0.0, 1.0];
        let sim = cosine_similarity(&a, &b);
        assert!((sim - 0.0).abs() < 1e-6);

        // 同じベクトルの類似度は 1
        let c = vec![3.0, 4.0];
        let d = vec![3.0, 4.0];
        let sim = cosine_similarity(&c, &d);
        assert!((sim - 1.0).abs() < 1e-6);

        // 反対方向の類似度は -1
        let e = vec![1.0, 0.0];
        let f = vec![-1.0, 0.0];
        let sim = cosine_similarity(&e, &f);
        assert!((sim - (-1.0)).abs() < 1e-6);
    }

    #[tokio::test]
    async fn test_dimension_mismatch_rejection() {
        // stub サーバを起動して、異なる次元のレスポンスを返す
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("failed to bind");
        let addr = listener.local_addr().expect("failed to get addr");

        // スレッドで stub サーバを実行
        let _server_handle = std::thread::spawn(move || {
            for stream in listener.incoming() {
                let mut stream = stream.expect("failed to accept");
                let mut buf = vec![0u8; 4096];
                let _n = std::io::Read::read(&mut stream, &mut buf).expect("read failed");

                // 100 次元のレスポンスを返す（期待: 768）
                // 実際の100要素配列
                let embedding_data = vec![0.1f32; 100];
                let json = serde_json::json!({
                    "data": [{
                        "embedding": embedding_data
                    }]
                });

                let response_text = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                    json.to_string().len(),
                    json.to_string()
                );
                std::io::Write::write_all(&mut stream, response_text.as_bytes())
                    .expect("write failed");
            }
        });

        let client = EmbedClient {
            client: Client::new(),
            endpoint: format!("http://{}/v1/embeddings", addr),
            model: "test".to_string(),
            dim: 768, // 期待: 768
            api_key_env: String::new(),
            embed_with_meta: false,
        };

        let result = client.embed(&["test".to_string()]).await;
        assert!(result.is_err());
        if let Err(EmbeddingError::DimensionMismatch { expected, actual }) = result {
            assert_eq!(expected, 768);
            assert_eq!(actual, 100);
        } else {
            panic!("expected DimensionMismatch error");
        }
    }

    #[tokio::test]
    async fn test_batching_order_preservation() {
        // stub サーバを起動して、入力順を保持してレスポンスを返す
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("failed to bind");
        let addr = listener.local_addr().expect("failed to get addr");

        let _server_handle = std::thread::spawn(move || {
            for stream in listener.incoming() {
                let mut stream = stream.expect("failed to accept");
                let mut buf = vec![0u8; 4096];
                let _n = std::io::Read::read(&mut stream, &mut buf).expect("read failed");

                // 3 つの異なるベクトルを返す（入力は [0, 1, 2] の 3 個のテキスト）
                let embeddings = vec![
                    vec![1.0, 0.0, 0.0],
                    vec![0.0, 1.0, 0.0],
                    vec![0.0, 0.0, 1.0],
                ];

                let json = serde_json::json!({
                    "data": embeddings.iter().map(|e| {
                        serde_json::json!({"embedding": e})
                    }).collect::<Vec<_>>()
                });

                let response_text = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                    json.to_string().len(),
                    json.to_string()
                );
                std::io::Write::write_all(&mut stream, response_text.as_bytes())
                    .expect("write failed");
                break;
            }
        });

        let client = EmbedClient {
            client: Client::new(),
            endpoint: format!("http://{}/v1/embeddings", addr),
            model: "test".to_string(),
            dim: 3,
            api_key_env: String::new(),
            embed_with_meta: false,
        };

        let inputs = vec![
            "text0".to_string(),
            "text1".to_string(),
            "text2".to_string(),
        ];
        let result = client.embed(&inputs).await;

        assert!(result.is_ok());
        let embeddings = result.unwrap();
        assert_eq!(embeddings.len(), 3);
        // 順序を確認
        assert_eq!(embeddings[0], vec![1.0, 0.0, 0.0]);
        assert_eq!(embeddings[1], vec![0.0, 1.0, 0.0]);
        assert_eq!(embeddings[2], vec![0.0, 0.0, 1.0]);
    }

    /// 埋め込みの先頭に付ける「タイトル（年）— 章」の形（共有契約）。
    #[test]
    fn test_embedding_meta_prefix_format() {
        assert_eq!(
            embedding_meta_prefix(Some("Title"), Some(2024), Some("3 Methods")).as_deref(),
            Some("Title（2024）— 3 Methods")
        );
        assert_eq!(
            embedding_meta_prefix(Some("Title"), None, Some("3 Methods")).as_deref(),
            Some("Title — 3 Methods")
        );
        assert_eq!(
            embedding_meta_prefix(None, Some(2024), None).as_deref(),
            Some("（2024）")
        );
        assert_eq!(
            embedding_meta_prefix(Some("Title"), Some(2024), None).as_deref(),
            Some("Title（2024）")
        );
        assert_eq!(embedding_meta_prefix(None, None, None), None);
    }

    /// 偽の埋め込み。渡された文字を記録する。
    struct FakeEmbedder {
        model: String,
        with_meta: bool,
        seen: std::sync::Mutex<Vec<String>>,
    }

    #[async_trait::async_trait]
    impl Embedder for FakeEmbedder {
        async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
            let mut seen = self.seen.lock().expect("seen");
            seen.extend(texts.iter().cloned());
            Ok(vec![vec![0.0f32; 4]; texts.len()])
        }

        fn model(&self) -> &str {
            &self.model
        }

        fn endpoint(&self) -> &str {
            "http://fake"
        }

        fn dim(&self) -> usize {
            4
        }

        fn embed_with_meta(&self) -> bool {
            self.with_meta
        }
    }

    fn embed_test_db() -> Connection {
        let conn = Connection::open_in_memory().expect("in-memory db");
        conn.execute_batch(
            "CREATE TABLE papers (id INTEGER PRIMARY KEY, title TEXT, published_at TEXT);
             CREATE TABLE chunks (
               id INTEGER PRIMARY KEY,
               source_kind TEXT NOT NULL,
               source_ref TEXT NOT NULL,
               paper_id INTEGER,
               ord INTEGER NOT NULL,
               text TEXT NOT NULL,
               strategy TEXT NOT NULL,
               created_at TEXT NOT NULL,
               index_name TEXT NOT NULL DEFAULT 'default',
               section TEXT
             );
             CREATE TABLE embedding_models (
               id INTEGER PRIMARY KEY,
               name TEXT NOT NULL UNIQUE,
               endpoint TEXT NOT NULL,
               dim INTEGER NOT NULL,
               created_at TEXT NOT NULL
             );
             CREATE TABLE chunk_embeddings (
               chunk_id INTEGER NOT NULL REFERENCES chunks(id),
               model_id INTEGER NOT NULL REFERENCES embedding_models(id),
               vec BLOB NOT NULL,
               PRIMARY KEY (chunk_id, model_id)
             );",
        )
        .expect("schema");
        conn
    }

    /// AC2: `embed_with_meta` が true の索引では、埋め込みに渡す文字の先頭に
    /// 「タイトル（年）— 章」が付く。`chunks.text` は変わらない。false なら付かない。
    #[tokio::test]
    async fn test_embed_pending_with_meta() {
        let conn = embed_test_db();
        conn.execute(
            "INSERT INTO papers (id, title, published_at) VALUES (1, 'Attention Is All You Need', '2017-06-12T00:00:00+00:00')",
            [],
        )
        .expect("paper");
        for (id, index_name) in [(1i64, "section-meta"), (2, "plain")] {
            conn.execute(
                "INSERT INTO chunks (id, source_kind, source_ref, paper_id, ord, text, strategy, created_at, index_name, section)
                 VALUES (?1, 'body', 'extract_1', 1, 0, 'Body text.', 'section', '2026-01-01T00:00:00Z', ?2, '3 Methods')",
                rusqlite::params![id, index_name],
            )
            .expect("chunk");
        }

        // 先頭を付ける索引。
        let with_meta = FakeEmbedder {
            model: "fake".to_string(),
            with_meta: true,
            seen: std::sync::Mutex::new(Vec::new()),
        };
        let count = embed_pending(&conn, &with_meta, "section-meta", 32)
            .await
            .expect("embed_pending");
        assert_eq!(count, 1);
        let seen = with_meta.seen.lock().expect("seen").clone();
        assert_eq!(
            seen[0],
            "Attention Is All You Need（2017）— 3 Methods\nBody text."
        );

        // 保存された本文は変わらない。
        let stored: String = conn
            .query_row("SELECT text FROM chunks WHERE id = 1", [], |row| row.get(0))
            .expect("stored text");
        assert_eq!(stored, "Body text.");

        // 先頭を付けない索引。
        let plain = FakeEmbedder {
            model: "fake".to_string(),
            with_meta: false,
            seen: std::sync::Mutex::new(Vec::new()),
        };
        let count = embed_pending(&conn, &plain, "plain", 32)
            .await
            .expect("embed_pending");
        assert_eq!(count, 1);
        let seen = plain.seen.lock().expect("seen").clone();
        assert_eq!(seen[0], "Body text.");
    }

    #[test]
    fn test_embed_pending_query_prepares() {
        // SQL クエリが正しくスキーマに対して prepare できることを確認
        // chunks.chunk_id ではなく chunks.id であることをチェック
        use rusqlite::params;

        let conn = Connection::open_in_memory().expect("failed to open in-memory db");

        // スキーマを初期化
        conn.execute(
            "CREATE TABLE chunks (
                id INTEGER PRIMARY KEY,
                source_kind TEXT NOT NULL,
                source_ref TEXT NOT NULL,
                ord INTEGER NOT NULL,
                text TEXT NOT NULL,
                strategy TEXT NOT NULL,
                created_at TEXT NOT NULL
            )",
            [],
        )
        .expect("failed to create chunks table");

        conn.execute(
            "CREATE TABLE embedding_models (
                id INTEGER PRIMARY KEY,
                name TEXT NOT NULL UNIQUE,
                endpoint TEXT NOT NULL,
                dim INTEGER NOT NULL,
                created_at TEXT NOT NULL
            )",
            [],
        )
        .expect("failed to create embedding_models table");

        conn.execute(
            "CREATE TABLE chunk_embeddings (
                chunk_id INTEGER NOT NULL REFERENCES chunks(id),
                model_id INTEGER NOT NULL REFERENCES embedding_models(id),
                vec BLOB NOT NULL,
                PRIMARY KEY (chunk_id, model_id)
            )",
            [],
        )
        .expect("failed to create chunk_embeddings table");

        // テストデータを挿入
        conn.execute(
            "INSERT INTO chunks (source_kind, source_ref, ord, text, strategy, created_at)
             VALUES (?, ?, ?, ?, ?, ?)",
            params![
                "abstract",
                "test_ref",
                1,
                "sample text",
                "paragraph",
                "2026-09-20T00:00:00Z"
            ],
        )
        .expect("failed to insert chunk");

        conn.execute(
            "INSERT INTO embedding_models (name, endpoint, dim, created_at)
             VALUES (?, ?, ?, ?)",
            params!["test-model", "http://test", 768, "2026-09-20T00:00:00Z"],
        )
        .expect("failed to insert model");

        let model_id = 1i64;

        // このクエリが prepare できることを確認（chunks.id を使用）
        let mut stmt = conn
            .prepare(
                "SELECT id FROM chunks
             WHERE NOT EXISTS (
                SELECT 1 FROM chunk_embeddings ce
                WHERE ce.chunk_id = chunks.id AND ce.model_id = ?1
             )",
            )
            .expect("failed to prepare query");

        let result: Vec<i64> = stmt
            .query_map([model_id], |row| row.get(0))
            .expect("failed to query_map")
            .collect::<std::result::Result<Vec<_>, _>>()
            .expect("failed to collect");

        // 埋め込みが存在しないので、チャンク 1 が返るはず
        assert_eq!(result, vec![1]);
    }
}

/// コサイン類似度を計算する。
///
/// 正規化されたベクトル間での内積を返す。
#[allow(dead_code)]
fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }

    let dot_product: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();

    if norm_a == 0.0 || norm_b == 0.0 {
        0.0
    } else {
        dot_product / (norm_a * norm_b)
    }
}
