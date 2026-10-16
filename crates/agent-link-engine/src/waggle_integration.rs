//! # waggle 統合モジュール
//!
//! pi エージェントの出力ファイルを waggle トークン化し、後続ブロックから
//! トークン参照可能にするモジュールです。
//!
//! ## 概要
//!
//! 本モジュールは FN-006（エージェント実行オーケストレーター）が出力した
//! pi エージェントの成果物ファイル（Markdown 文書、JSONL セッションログ）を
//! 読み取り、`lib/waggle/crates/waggle-store-sqlite` の `SqliteStore` を
//! 使って waggle トークンを発行します。
//!
//! トークン化された出力は、後続タスクの `task_text` に書いた `waggle:<トークン>`
//! （または `sai_read`）で参照できます。
//!
//! ## 使用例
//!
//! ```no_run
//! use std::path::Path;
//! use agent_link_engine::waggle_integration::WaggleIntegrator;
//!
//! // インメモリ DB で integrator を作成
//! let integrator = WaggleIntegrator::open_in_memory()
//!     .expect("waggle 統合の初期化に失敗");
//!
//! // ファイルをトークン化
//! let result = integrator.tokenize_file(
//!     Path::new("/tmp/output/report.md"),
//!     "analysis-block",
//!     Some("解析結果レポート"),
//! ).expect("トークン化に失敗");
//!
//! println!("トークン: {}, SHA-256: {}", result.token, result.sha256.as_str());
//!
//! // トークンから内容を解決
//! let content = integrator.resolve_content(result.token.as_str())
//!     .expect("内容の解決に失敗");
//! println!("ファイル内容: {}", &content[..100.min(content.len())]);
//! ```

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use sha2::Digest;
use waggle_core::{
    mint, CanonicalUrl, Channel, Entropy, EntropyError, MintOptions, MintSpec, Sharer, TargetMeta,
    Timestamp, Token,
};
use waggle_store::{AppendIntent, AppendStore, Appended, MintNonce, ReadStore, StoreError};
use waggle_store_sqlite::SqliteStore;

// ============================================================================
// エラー型
// ============================================================================

/// waggle 統合モジュールのエラー型。
///
/// ファイル I/O、waggle ストア操作、ミント処理、ハッシュ計算など、
/// トークン化パイプラインで発生するすべてのエラーを表現します。
#[derive(Debug, thiserror::Error)]
pub enum WaggleIntegrationError {
    /// ファイル読み込みエラー（ファイル不在、パーミッション不足など）。
    #[error("File read error: {0}")]
    FileRead(#[from] std::io::Error),

    /// waggle ストアエラー（DB 接続障害、整合性違反など）。
    #[error("waggle store error: {0}")]
    Store(#[from] StoreError),

    /// waggle ミントエラー（マニフェストサイズ超過、重複キャッチオールなど）。
    #[error("waggle mint error: {0}")]
    Mint(#[from] waggle_core::MintError),

    /// SHA-256 ハッシュ計算エラー。
    #[error("SHA-256 hash error: {0}")]
    Hash(String),

    /// トークンに対応するマニフェストが見つからない。
    #[error("Manifest for token {0} not found.")]
    TokenNotFound(Token),

    /// 無効なターゲット URL（ファイルパス由来の URL 構築失敗など）。
    #[error("Invalid target URL: {0}")]
    InvalidTarget(String),

    /// 空のファイル（空ファイルはトークン化できません）。
    #[error("The file is empty: {path}")]
    EmptyFile {
        /// ファイルパス
        path: PathBuf,
    },

    /// エントロピーソースエラー（乱数生成の失敗）。
    #[error("Entropy source error: {0}")]
    Entropy(String),
}

// ============================================================================
// TokenizedOutput — トークン化結果
// ============================================================================

/// トークン化結果を表す構造体。
///
/// pi エージェントの出力ファイル1つを waggle トークン化した結果を格納します。
/// 発行されたトークン文字列、元のファイルパス、SHA-256 ハッシュ、
/// MIME タイプ、ファイルサイズを持ちます。
#[derive(Debug, Clone)]
pub struct TokenizedOutput {
    /// 発行された waggle トークン（8文字の base58 文字列）。
    pub token: Token,
    /// 元の出力ファイルの絶対パス。
    pub file_path: PathBuf,
    /// ファイル内容の SHA-256 ハッシュ（64桁の16進数小文字）。
    pub sha256: waggle_core::Sha256Hex,
    /// MIME タイプ（例: "text/markdown", "application/jsonl"）。
    pub content_type: String,
    /// ファイルサイズ（バイト）。
    pub byte_size: u64,
}

// ============================================================================
// エントロピーソース
// ============================================================================

/// OS の乱数生成器を使用するエントロピーソース。
///
/// `getrandom` クレートを内部で使用し、暗号学的に安全な乱数を提供します。
/// waggle の `mint()` 関数は `&mut impl Entropy` を要求するため、
/// このソースを渡します。
pub struct OsEntropy;

impl Entropy for OsEntropy {
    fn fill(&mut self, buf: &mut [u8]) -> Result<(), EntropyError> {
        getrandom::getrandom(buf).map_err(|e| EntropyError(e.to_string()))
    }
}

// ============================================================================
// プライベートユーティリティ関数
// ============================================================================

/// ファイルの SHA-256 ハッシュを計算し、`Sha256Hex`・バイト列・ファイルサイズを返す。
///
/// 空ファイルの場合は `EmptyFile` エラーを返します。
/// 戻り値のバイト列は将来 `BlobStore::put()` 用に保持されます。
fn compute_sha256(
    path: &Path,
) -> Result<(waggle_core::Sha256Hex, Vec<u8>, u64), WaggleIntegrationError> {
    let bytes = std::fs::read(path).map_err(WaggleIntegrationError::FileRead)?;
    let byte_size = bytes.len() as u64;

    if bytes.is_empty() {
        return Err(WaggleIntegrationError::EmptyFile {
            path: path.to_path_buf(),
        });
    }

    let hash = sha2::Sha256::digest(&bytes);
    let hex_string = format!("{:x}", hash);

    let sha256 = waggle_core::Sha256Hex::new(&hex_string)
        .map_err(|e| WaggleIntegrationError::Hash(format!("Sha256Hex error: {:?}", e)))?;

    Ok((sha256, bytes, byte_size))
}

/// ファイルパスから MIME タイプを決定する。
///
/// 既知の拡張子に基づいて適切な MIME タイプを返します。
/// 未知の拡張子の場合は `application/octet-stream` を返します。
fn content_type_from_path(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()) {
        Some("md") => "text/markdown",
        Some("jsonl") => "application/jsonl",
        Some("json") => "application/json",
        Some("yaml") | Some("yml") => "application/x-yaml",
        Some("txt") => "text/plain",
        Some("html") | Some("htm") => "text/html",
        Some("csv") => "text/csv",
        Some("png") => "image/png",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("svg") => "image/svg+xml",
        Some("pdf") => "application/pdf",
        _ => "application/octet-stream",
    }
}

/// ファイルの絶対パスから `file://` URL 形式の `CanonicalUrl` を構築する。
///
/// 例: `/home/user/output/report.md` → `file:///home/user/output/report.md`
fn canonical_url_from_path(path: &Path) -> Result<CanonicalUrl, WaggleIntegrationError> {
    let abs = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let url_str = format!("file://{}", abs.display());
    CanonicalUrl::new(&url_str)
        .map_err(|e| WaggleIntegrationError::InvalidTarget(format!("{}({})", e, url_str)))
}

/// `file://` URL からファイルパスを抽出する。
///
/// 以下の形式に対応:
/// - `file:///absolute/path/to/file`
/// - `file://localhost/absolute/path/to/file`
fn extract_file_path_from_url(url: &str) -> Option<PathBuf> {
    let path_str = url
        .strip_prefix("file://localhost")
        .or_else(|| url.strip_prefix("file://"))?;

    // URL デコード（簡易版: スペースのみ対応）
    let decoded = path_str.replace("%20", " ");
    Some(PathBuf::from(decoded))
}

/// 現在時刻を waggle の `Timestamp` として取得する。
fn now_timestamp() -> Timestamp {
    let ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    Timestamp::from_unix_ms(ms)
}

/// ユニークな `MintNonce` を生成する。
///
/// OS の乱数生成器を使用した 8 バイトの乱数から nonce を生成します。
fn generate_nonce() -> Result<MintNonce, WaggleIntegrationError> {
    let mut buf = [0u8; 8];
    getrandom::getrandom(&mut buf).map_err(|e| WaggleIntegrationError::Entropy(e.to_string()))?;
    Ok(MintNonce(u64::from_ne_bytes(buf)))
}

// ============================================================================
// WaggleIntegrator — 中核構造体
// ============================================================================

/// waggle ストアをラップし、トークン化・トークン解決の全操作を提供する中核構造体。
///
/// `SqliteStore` を `Arc` でラップすることでスレッドセーフに共有可能です。
/// FN-006 のオーケストレーターや FN-008 の DB モジュールと
/// 同一ストアインスタンスを共有できます。
///
/// ## 設計意図
///
/// - `store` は `Arc<SqliteStore>` で包み、複数スレッドからの共有を可能にする
/// - `sharer` と `channel` は全トークンで共通のデフォルト値を保持する
/// - エントロピーソースは各 `tokenize_file` 呼び出しごとに新規作成する
///   （OS 乱数生成器はステートレスなため）
///
/// ## 使用例
///
/// ```no_run
/// use std::path::Path;
/// use agent_link_engine::waggle_integration::WaggleIntegrator;
///
/// let integrator = WaggleIntegrator::open_in_memory()
///     .expect("waggle 統合の初期化に失敗");
///
/// // 単一ファイルのトークン化
/// let result = integrator.tokenize_file(
///     Path::new("/tmp/output.md"),
///     "my-block",
///     Some("ブロック出力"),
/// ).expect("トークン化に失敗");
///
/// // トークンから内容を再取得
/// let content = integrator.resolve_content(result.token.as_str())
///     .expect("解決に失敗");
/// ```
pub struct WaggleIntegrator {
    /// スレッドセーフな waggle SQLite ストア。
    store: Arc<SqliteStore>,
    /// 全トークンの発行者名（デフォルト: "agent-link"）。
    sharer: Sharer,
    /// 全トークンのチャンネル（デフォルト: `Channel::subagent_general()`）。
    channel: Channel,
}

impl WaggleIntegrator {
    /// 指定パスに SQLite DB を開く（または新規作成）。
    ///
    /// `sharer` にはデフォルトで `"agent-link"` が設定されます。
    /// 保存されたトークンはプロセス再起動後も利用可能です。
    ///
    /// # 引数
    ///
    /// * `db_path` — SQLite DB ファイルのパス
    ///
    /// # エラー
    ///
    /// DB ファイルの作成またはオープンに失敗した場合は `StoreError` を返します。
    pub fn open(db_path: impl AsRef<Path>) -> Result<Self, StoreError> {
        crate::db::store::ensure_shm_file(db_path.as_ref());
        let store = SqliteStore::open(db_path.as_ref())?;
        Ok(Self {
            store: Arc::new(store),
            sharer: Sharer::new("agent-link").expect("'agent-link' is a valid sharer name"),
            channel: Channel::subagent_general(),
        })
    }

    /// インメモリ DB を開く（テスト用）。
    ///
    /// アプリケーションの終了とともにデータは消滅します。
    /// 主に単体テストや一時的な用途に使用します。
    ///
    /// # エラー
    ///
    /// インメモリ DB の作成に失敗した場合は `StoreError` を返します。
    pub fn open_in_memory() -> Result<Self, StoreError> {
        let store = SqliteStore::open_in_memory()?;
        Ok(Self {
            store: Arc::new(store),
            sharer: Sharer::new("agent-link").expect("'agent-link' is a valid sharer name"),
            channel: Channel::subagent_general(),
        })
    }

    /// 発行者名（sharer）を上書きする。
    ///
    /// # 引数
    ///
    /// * `sharer` — 新しい発行者名
    #[must_use]
    pub fn with_sharer(mut self, sharer: Sharer) -> Self {
        self.sharer = sharer;
        self
    }

    /// 内部ストアへの参照を取得する。
    ///
    /// ストアを直接操作する必要がある場合に使用します（例: FN-008 との連携）。
    #[must_use]
    pub fn store(&self) -> &Arc<SqliteStore> {
        &self.store
    }

    // ====================================================================
    // コアトークン化 API
    // ====================================================================

    /// 単一ファイルを waggle トークン化する。
    ///
    /// 以下の処理を行います:
    ///
    /// 1. `compute_sha256()` でファイルの SHA-256 ハッシュ・内容・サイズを取得
    /// 2. `canonical_url_from_path()` で `CanonicalUrl` を構築
    /// 3. `content_type_from_path()` で MIME タイプを決定
    /// 4. `MintSpec::new()` でベーススペックを作成
    /// 5. `.meta()`, `.label()` でメタデータとラベルを設定
    /// 6. `waggle_core::mint()` でマニフェストを生成
    /// 7. `self.store.append()` でストアに永続化
    /// 8. 発行された `TokenizedOutput` を返す
    ///
    /// # 引数
    ///
    /// * `file_path` — トークン化するファイルのパス
    /// * `block_name` — このファイルを生成したブロック名（ラベルに設定）
    /// * `description` — 出力の説明（省略可）
    ///
    /// # エラー
    ///
    /// 空ファイル、ファイル不在、SHA-256 計算エラー、
    /// waggle ミントエラー、ストア永続化エラーのいずれか。
    pub fn tokenize_file(
        &self,
        file_path: &Path,
        block_name: &str,
        description: Option<&str>,
    ) -> Result<TokenizedOutput, WaggleIntegrationError> {
        // 1. ファイルのハッシュ・内容・サイズを取得
        let (sha256, bytes, byte_size) = compute_sha256(file_path)?;

        // 1.5. Blob 保持（P21・6.3 の修正）:
        // トークン発行時にファイル内容を `.agent-link/engine/waggle-blobs/<sha256>` へコピーし、
        // 元ファイルが worktree 削除・git clean などで消失しても解決できるようにする。
        let blob_dir = crate::project::engine_dir(&crate::project::project_root()).join("waggle-blobs");
        let _ = std::fs::create_dir_all(&blob_dir);
        let blob_path = blob_dir.join(sha256.as_str());
        let _ = std::fs::write(&blob_path, &bytes);

        // 2. CanonicalUrl を構築
        let target = canonical_url_from_path(file_path)?;

        // 3. MIME タイプを決定
        let content_type = content_type_from_path(file_path);

        // 4. MintSpec を構築
        let block_sharer = Sharer::new(block_name).map_err(|e| {
            WaggleIntegrationError::InvalidTarget(format!("sharer '{block_name}': {e}"))
        })?;
        let spec = MintSpec::new(target, block_sharer, self.channel.clone())
            .meta(TargetMeta {
                title: Some(format!("{} output", block_name)),
                description: description.map(|d| d.to_string()),
                ..Default::default()
            })
            .label("block_name", block_name)
            .label("file_path", file_path.to_string_lossy().as_ref())
            .label("sha256", sha256.as_str());

        // 5. エントロピーソースと現在時刻を準備
        let mut entropy = OsEntropy;
        let now = now_timestamp();

        // 6. mint を実行
        let manifest = mint(spec, &MintOptions::default(), &mut entropy, now)?;

        // 7. ユニークな nonce を生成
        let nonce = generate_nonce()?;
        let token = manifest.token;

        // 8. ストアに永続化（pollster で async をブロッキング実行）
        let result = pollster::block_on(self.store.append(AppendIntent::Mint {
            manifest: Box::new(manifest),
            nonce,
        }))?;

        // 9. トークンを抽出して返却
        match result {
            Appended::Minted { .. } => Ok(TokenizedOutput {
                token,
                file_path: file_path.to_path_buf(),
                sha256,
                content_type: content_type.to_string(),
                byte_size,
            }),
            _ => Err(WaggleIntegrationError::Store(StoreError::Backend(
                "append returned an unexpected result (not Minted).".into(),
            ))),
        }
    }

    /// 複数ファイルを一括トークン化するバッチ関数。
    ///
    /// 各ファイルに対して `tokenize_file()` を逐次呼び出します。
    /// いずれかのファイルでエラーが発生した場合、即座にエラーを返します
    /// （部分的な成功は返さない — トランザクションの整合性のため）。
    ///
    /// # 引数
    ///
    /// * `outputs` — `(ファイルパス, 説明)` のタプルのスライス
    /// * `block_name` — ブロック名
    ///
    /// # エラー
    ///
    /// いずれかのファイルでエラーが発生した場合、最初のエラーを返します。
    pub fn tokenize_outputs(
        &self,
        outputs: &[(PathBuf, &str)],
        block_name: &str,
    ) -> Result<Vec<TokenizedOutput>, WaggleIntegrationError> {
        let mut results = Vec::with_capacity(outputs.len());
        for (file_path, description) in outputs {
            let result = self.tokenize_file(file_path, block_name, Some(description))?;
            results.push(result);
        }
        Ok(results)
    }

    // ====================================================================
    // トークン解決 API
    // ====================================================================

    /// トークン文字列からマニフェストビューを取得する。
    ///
    /// 後続ブロックが waggle トークンからメタデータを参照するために使用します。
    ///
    /// # 引数
    ///
    /// * `token_str` — 解決する waggle トークンの文字列（例: `"AbCdEfGh"`）
    ///
    /// # エラー
    ///
    /// トークンがパースできない場合、またはトークンがストアに存在しない場合。
    pub fn resolve_token(
        &self,
        token_str: &str,
    ) -> Result<waggle_store::ManifestView, WaggleIntegrationError> {
        let token = Token::parse(token_str).map_err(|e| {
            WaggleIntegrationError::InvalidTarget(format!("Failed to parse the token: {e}"))
        })?;

        pollster::block_on(self.store.manifest(token))?
            .ok_or(WaggleIntegrationError::TokenNotFound(token))
    }

    /// トークン文字列から元ファイルの `CanonicalUrl` を取得する。
    ///
    /// # 引数
    ///
    /// * `token_str` — 解決する waggle トークンの文字列
    ///
    /// # 戻り値
    ///
    /// 元のファイルパス由来の `file://` URL。
    pub fn resolve_target_url(
        &self,
        token_str: &str,
    ) -> Result<CanonicalUrl, WaggleIntegrationError> {
        let view = self.resolve_token(token_str)?;
        Ok(view.manifest.target.clone())
    }

    /// トークン文字列から元のファイル内容を文字列として取得する。
    ///
    /// まず元のファイルを読み、存在しない場合は **Blob ストア**
    /// （`.agent-link/engine/waggle-blobs/<sha256>`、P21・6.3 で tokenize 時に保持）
    /// から内容を復元する。これにより worktree 削除や git clean で元ファイルが
    /// 消えてもトークン解決が継続する。
    ///
    /// # 引数
    ///
    /// * `token_str` — 解決する waggle トークンの文字列
    ///
    /// # 戻り値
    ///
    /// ファイルの内容を UTF-8 文字列として返します。
    pub fn resolve_content(&self, token_str: &str) -> Result<String, WaggleIntegrationError> {
        let view = self.resolve_token(token_str)?;
        let target_str = view.manifest.target.as_str();
        let file_path = extract_file_path_from_url(target_str).ok_or_else(|| {
            WaggleIntegrationError::InvalidTarget(format!(
                "Cannot extract a path from the file:// URL: {target_str}"
            ))
        })?;

        // 1. 元ファイルから読み取り（通常パス）
        match std::fs::read_to_string(&file_path) {
            Ok(content) => return Ok(content),
            Err(_) => {
                // 2. 元ファイル消失時は Blob ストアから復元を試みる（P21・6.3）
                if let Some(sha256) = view.manifest.labels.get("sha256") {
                    let blob_path =
                        crate::project::engine_dir(&crate::project::project_root()).join("waggle-blobs").join(sha256);
                    if let Ok(content) = std::fs::read_to_string(&blob_path) {
                        return Ok(content);
                    }
                }
                // 3. どちらも無い場合は元ファイルの読み取りエラーを返す
                Err(WaggleIntegrationError::FileRead(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    format!(
                        "Could not get the content from the original file {} or the blob store.",
                        file_path.display()
                    ),
                )))
            }
        }
    }

    /// トークン文字列から出力元ブロック名を取得する。
    ///
    /// マニフェストのラベル `block_name` を参照し、
    /// 見つからない場合は `"unknown"` を返す。
    ///
    /// # 引数
    ///
    /// * `token_str` — waggle トークンの文字列
    ///
    /// # 戻り値
    ///
    /// 出力元ブロック名（見つからない場合は `"unknown"`）
    pub fn find_block_name_by_token(&self, token_str: &str) -> String {
        // resolve_token 経由でマニフェストを取得
        if let Ok(view) = self.resolve_token(token_str) {
            // manifest.labels から block_name を取得
            if let Some(name) = view.manifest.labels.get("block_name") {
                return name.clone();
            }
        }
        // フォールバック: scan_all で検索
        if let Ok(token) = Token::parse(token_str) {
            if let Ok(records) = pollster::block_on(self.store.scan_all()) {
                for record in records {
                    if let waggle_core::LogRecord::Minted { manifest } = record {
                        if manifest.token == token {
                            if let Some(name) = manifest.labels.get("block_name") {
                                return name.clone();
                            }
                        }
                    }
                }
            }
        }
        "unknown".to_string()
    }

    /// 特定のブロック名に関連づけられた全トークンを検索する。
    ///
    /// `scan_all()` ですべてのレコードをスキャンし、ラベル `block_name` が
    /// 一致するマニフェストをフィルタリングします。
    ///
    /// ## パフォーマンス注意
    ///
    /// MVP ではトークン数が少ないため許容可能なパフォーマンスですが、
    /// トークン数が増えた場合は FN-008（DB スキーマ設計）で
    /// `block_name → token` のインデックステーブルを作成することで最適化予定です。
    ///
    /// # 引数
    ///
    /// * `block_name` — 検索するブロック名
    ///
    /// # 戻り値
    ///
    /// 一致するトークンのベクター（見つからない場合は空のベクター）。
    pub fn find_tokens_by_block(
        &self,
        block_name: &str,
    ) -> Result<Vec<Token>, WaggleIntegrationError> {
        let records = pollster::block_on(self.store.scan_all())?;
        let tokens: Vec<Token> = records
            .iter()
            .filter_map(|record| {
                if let waggle_core::LogRecord::Minted { manifest } = record {
                    if manifest.labels.get("block_name").map(String::as_str) == Some(block_name) {
                        return Some(manifest.token);
                    }
                }
                None
            })
            .collect();
        Ok(tokens)
    }
}

// ============================================================================
// テスト用ヘルパー
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    /// テスト用の決定論的エントロピーソース。
    ///
    /// `waggle_core` のテストパターン（`fixed_entropy()`）と同じく、
    /// 固定ステップのカウンタで再現可能な乱数列を生成します。
    #[cfg(test)]
    pub fn counting_entropy() -> impl Entropy {
        let mut n = 0u8;
        move |buf: &mut [u8]| {
            for b in buf.iter_mut() {
                n = n.wrapping_add(13);
                *b = n;
            }
            Ok(())
        }
    }

    /// テスト用フィクスチャの絶対パスを取得する。
    fn test_fixture_path(name: &str) -> PathBuf {
        let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        path.push("tests");
        path.push("fixtures");
        path.push(name);
        path
    }

    /// インメモリ DB のテスト用インテグレーターを作成する。
    fn test_integrator() -> WaggleIntegrator {
        WaggleIntegrator::open_in_memory().expect("インメモリ DB の作成に失敗")
    }

    // ====================================================================
    // トークン化基本テスト
    // ====================================================================

    #[test]
    fn test_compute_sha256_valid_file() {
        let path = test_fixture_path("sample_output.md");
        let (sha256, bytes, size) = compute_sha256(&path).expect("SHA-256 計算に失敗");
        assert_eq!(sha256.as_str().len(), 64, "SHA-256 は64桁の16進数");
        assert!(
            sha256
                .as_str()
                .chars()
                .all(|c| c.is_ascii_hexdigit() || c.is_ascii_lowercase()),
            "SHA-256 は小文字16進数のみ"
        );
        assert!(!bytes.is_empty(), "バイト列が空ではない");
        assert!(size > 0, "ファイルサイズが正の値");
    }

    #[test]
    fn test_compute_sha256_empty_file_error() {
        // 空のテンポラリファイルを作成
        let dir = std::env::temp_dir();
        let path = dir.join("fn007_test_empty.tmp");
        std::fs::write(&path, b"").expect("空ファイルの作成に失敗");

        let result = compute_sha256(&path);
        assert!(result.is_err(), "空ファイルでエラーになること");
        match result {
            Err(WaggleIntegrationError::EmptyFile { path: p }) => {
                assert_eq!(p, path);
            }
            _ => panic!("EmptyFile エラーが返されること"),
        }

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_compute_sha256_file_not_found() {
        let result = compute_sha256(&PathBuf::from("/tmp/nonexistent_file_fn007.md"));
        assert!(result.is_err());
        match result {
            Err(WaggleIntegrationError::FileRead(_)) => {} // OK
            _ => panic!("FileRead エラーが返されること"),
        }
    }

    #[test]
    fn test_content_type_md() {
        assert_eq!(
            content_type_from_path(Path::new("output.md")),
            "text/markdown"
        );
    }

    #[test]
    fn test_content_type_jsonl() {
        assert_eq!(
            content_type_from_path(Path::new("session.jsonl")),
            "application/jsonl"
        );
    }

    #[test]
    fn test_content_type_json() {
        assert_eq!(
            content_type_from_path(Path::new("data.json")),
            "application/json"
        );
    }

    #[test]
    fn test_content_type_unknown() {
        assert_eq!(
            content_type_from_path(Path::new("output.bin")),
            "application/octet-stream"
        );
    }

    #[test]
    fn test_canonical_url_from_path() {
        let path = test_fixture_path("sample_output.md");
        let url = canonical_url_from_path(&path).expect("CanonicalUrl の構築に失敗");
        let url_str = url.as_str();
        assert!(
            url_str.starts_with("file://"),
            "file:// URL で始まること: {}",
            url_str
        );
        assert!(url_str.contains("sample_output.md"), "ファイル名を含むこと");
    }

    #[test]
    fn test_extract_file_path_from_url() {
        let url = "file:///home/user/output/report.md";
        let path = extract_file_path_from_url(url).expect("パスの抽出に失敗");
        assert_eq!(path, PathBuf::from("/home/user/output/report.md"));
    }

    #[test]
    fn test_extract_file_path_from_url_with_localhost() {
        let url = "file://localhost/home/user/output/report.md";
        let path = extract_file_path_from_url(url).expect("パスの抽出に失敗");
        assert_eq!(path, PathBuf::from("/home/user/output/report.md"));
    }

    #[test]
    fn test_extract_file_path_from_url_with_encoding() {
        let url = "file:///home/user/my%20output/report.md";
        let path = extract_file_path_from_url(url).expect("パスの抽出に失敗");
        assert_eq!(path, PathBuf::from("/home/user/my output/report.md"));
    }

    #[test]
    fn test_extract_file_path_from_url_non_file() {
        let url = "https://example.com/file.md";
        let result = extract_file_path_from_url(url);
        assert!(result.is_none(), "非 file:// URL は None を返す");
    }

    // ====================================================================
    // トークン化統合テスト（インメモリ DB）
    // ====================================================================

    #[test]
    fn test_tokenize_markdown_file() {
        let integrator = test_integrator();
        let path = test_fixture_path("sample_output.md");
        let result = integrator
            .tokenize_file(&path, "test-block", Some("テスト出力"))
            .expect("トークン化に失敗");

        // トークンが空でないこと
        assert!(!result.token.as_str().is_empty(), "トークンが空でない");
        assert_eq!(result.token.as_str().len(), 8, "トークン長は8文字");

        // SHA-256 が64桁の16進数であること
        assert_eq!(result.sha256.as_str().len(), 64);

        // MIME タイプ
        assert_eq!(result.content_type, "text/markdown");

        // ファイルパス
        assert_eq!(result.file_path, path);

        // ファイルサイズ
        assert!(result.byte_size > 0);
    }

    #[test]
    fn test_tokenize_same_content_same_hash() {
        let integrator = test_integrator();
        let path = test_fixture_path("sample_output.md");

        let result1 = integrator
            .tokenize_file(&path, "test-block", None)
            .expect("1回目のトークン化");
        let result2 = integrator
            .tokenize_file(&path, "test-block", None)
            .expect("2回目のトークン化");

        // SHA-256 ハッシュは同一（同じファイル内容）
        assert_eq!(
            result1.sha256.as_str(),
            result2.sha256.as_str(),
            "同じ内容のファイルは同じ SHA-256"
        );

        // トークン文字列は異なる（nonce が異なるため）
        assert_ne!(
            result1.token.as_str(),
            result2.token.as_str(),
            "同じ内容でもトークン文字列は異なる"
        );
    }

    #[test]
    fn test_tokenize_empty_file_error() {
        let integrator = test_integrator();
        let dir = std::env::temp_dir();
        let path = dir.join("fn007_test_empty_tokenize.tmp");
        std::fs::write(&path, b"").expect("空ファイルの作成に失敗");

        let result = integrator.tokenize_file(&path, "test-block", None);
        assert!(result.is_err(), "空ファイルでエラーになること");

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_tokenize_file_not_found() {
        let integrator = test_integrator();
        let path = PathBuf::from("/tmp/nonexistent_fn007_tokenize.md");
        let result = integrator.tokenize_file(&path, "test-block", None);
        assert!(result.is_err(), "存在しないファイルでエラーになること");
    }

    #[test]
    fn test_tokenize_outputs_multiple() {
        let integrator = test_integrator();
        let path_md = test_fixture_path("sample_output.md");

        // JSONL フィクスチャが存在すれば使用、なければ sample_output.md を兼用
        let path_jsonl = test_fixture_path("sample_output.md"); // このテストでは同じファイルを使用

        let outputs = vec![(path_md.clone(), "Markdown出力"), (path_jsonl, "JSONL出力")];

        let results = integrator
            .tokenize_outputs(&outputs, "multi-block")
            .expect("バッチトークン化に失敗");

        assert_eq!(results.len(), 2, "2つのファイルがトークン化される");
        assert_eq!(results[0].content_type, "text/markdown");
        assert!(!results[0].token.as_str().is_empty());
        assert!(!results[1].token.as_str().is_empty());
    }

    // ====================================================================
    // トークン解決テスト
    // ====================================================================

    #[test]
    fn test_resolve_token_roundtrip() {
        let integrator = test_integrator();
        let path = test_fixture_path("sample_output.md");
        let original_content = std::fs::read_to_string(&path).expect("元ファイルの読み込みに失敗");

        let result = integrator
            .tokenize_file(&path, "roundtrip-block", None)
            .expect("トークン化に失敗");

        // トークンから内容を解決
        let resolved = integrator
            .resolve_content(result.token.as_str())
            .expect("内容の解決に失敗");

        assert_eq!(
            resolved, original_content,
            "トークン化→解決で元の内容が一致"
        );
    }

    #[test]
    fn test_resolve_unknown_token_error() {
        let integrator = test_integrator();

        // 存在しないトークン文字列（正しい base58 形式だがストアに存在しない）
        let result = integrator.resolve_content("12345678");
        assert!(result.is_err(), "存在しないトークンでエラーになること");
        match result {
            Err(WaggleIntegrationError::TokenNotFound(_)) => {} // ストアに存在しない
            _ => panic!("TokenNotFound エラーになること"),
        }
    }

    #[test]
    fn test_resolve_target_url() {
        let integrator = test_integrator();
        let path = test_fixture_path("sample_output.md");

        let result = integrator
            .tokenize_file(&path, "url-block", None)
            .expect("トークン化に失敗");

        let resolved_url = integrator
            .resolve_target_url(result.token.as_str())
            .expect("URL の解決に失敗");

        let url_str = resolved_url.as_str();
        assert!(url_str.starts_with("file://"), "file:// URL であること");
        assert!(url_str.contains("sample_output.md"), "ファイル名を含むこと");
    }

    // ====================================================================
    // ラベル・検索テスト
    // ====================================================================

    #[test]
    fn test_tokenize_sets_block_name_label() {
        let integrator = test_integrator();
        let path = test_fixture_path("sample_output.md");

        let result = integrator
            .tokenize_file(&path, "label-test-block", None)
            .expect("トークン化に失敗");

        let view = integrator
            .resolve_token(result.token.as_str())
            .expect("トークン解決に失敗");

        // ラベルを確認
        let block_name_label = view.manifest.labels.get("block_name").map(String::as_str);
        assert_eq!(block_name_label, Some("label-test-block"));

        let sha256_label = view.manifest.labels.get("sha256").map(String::as_str);
        assert_eq!(sha256_label, Some(result.sha256.as_str()));
    }

    #[test]
    fn test_find_tokens_by_block() {
        let integrator = test_integrator();
        let path = test_fixture_path("sample_output.md");

        // 同じブロック名で2回トークン化
        let result1 = integrator
            .tokenize_file(&path, "search-block", Some("1回目"))
            .expect("1回目のトークン化");
        let result2 = integrator
            .tokenize_file(&path, "search-block", Some("2回目"))
            .expect("2回目のトークン化");

        // 別のブロック名でもトークン化
        let _result3 = integrator
            .tokenize_file(&path, "other-block", None)
            .expect("3回目のトークン化");

        // "search-block" のトークンを検索
        let tokens = integrator
            .find_tokens_by_block("search-block")
            .expect("ブロック名による検索に失敗");

        assert_eq!(tokens.len(), 2, "search-block は2つのトークンを持つ");
        assert!(tokens.contains(&result1.token), "1回目のトークンが含まれる");
        assert!(tokens.contains(&result2.token), "2回目のトークンが含まれる");
    }

    // ====================================================================
    // インメモリ DB テスト
    // ====================================================================

    #[test]
    fn test_resolve_invalid_token_format() {
        let integrator = test_integrator();

        // base58 に含まれない文字 '0' を含むトークン
        let result = integrator.resolve_content("abc0defg");
        assert!(result.is_err(), "無効なトークンでエラーになること");
        match &result {
            Err(WaggleIntegrationError::InvalidTarget(msg)) => {
                assert!(msg.contains("parse"), "パースエラーメッセージを含む");
            }
            _ => panic!("InvalidTarget エラーになること: {:?}", result),
        }

        // 24文字超のトークン
        let result = integrator.resolve_content("aaaaaaaaaaaaaaaaaaaaaaaaa"); // 25 chars
        assert!(result.is_err(), "長すぎるトークンでエラーになること");
    }

    #[test]
    fn test_in_memory_store_persistence() {
        let integrator = test_integrator();
        let path = test_fixture_path("sample_output.md");

        let result = integrator
            .tokenize_file(&path, "persistence-block", None)
            .expect("トークン化に失敗");

        // トークン化直後に読み取り可能（read-your-writes）
        let view = integrator
            .resolve_token(result.token.as_str())
            .expect("即時読み取りに失敗");
        assert_eq!(view.manifest.token, result.token);
    }

    #[test]
    fn test_in_memory_store_isolation() {
        let integrator1 = test_integrator();
        let integrator2 = test_integrator();
        let path = test_fixture_path("sample_output.md");

        let result1 = integrator1
            .tokenize_file(&path, "isolation-block", None)
            .expect("integrator1 のトークン化");

        // integrator2 には integrator1 のトークンが存在しない
        let view = pollster::block_on(integrator2.store().manifest(result1.token))
            .expect("manifest 呼び出しに成功");
        assert!(
            view.is_none(),
            "異なるインスタンス間でデータが分離されている"
        );
    }
}
