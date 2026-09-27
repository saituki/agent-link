//! # sai-write 通知受信モジュール
//!
//! pi-sai 拡張機能の `sai-write` ツールが書き込む通知ファイルを監視し、
//! 書き込まれたドキュメントファイルを waggle トークン化します。
//!
//! ## 通知プロトコル
//!
//! `sai-write` ツール（pi-sai 拡張機能）はファイル書き込み後、
//! 以下の JSON ファイルを `.sai/sai-engine/notifications/pending/` に作成します。
//!
//! ```json
//! {
//!   "type": "file_written",
//!   "file_path": "/absolute/path/to/output.md",
//!   "block_name": "sai-brst-grill",
//!   "description": "ブレインストーミング結果",
//!   "mime_type": "text/markdown",
//!   "byte_size": 1234,
//!   "timestamp": "2026-07-30T01:23:45Z"
//! }
//! ```
//!
//! `sai_write`（P4 以降）はエージェント側で発行済みのトークンを
//! `"waggle_token": "AXANSMpo"` として含めます。このフィールドがある場合、
//! Sai-Engine は再トークン化せず、そのトークンをそのまま使います
//! （mint はランダムなので、再トークン化するとトークンが 2 つになるため）。
//!
//! Sai-Engine はこのディレクトリをポーリングし、新しい通知ファイルを検出すると:
//! 1. 通知 JSON をパース
//! 2. `waggle_token` があればそれを使い、無ければ `WaggleIntegrator::tokenize_file()`
//!    でトークン化
//! 3. 通知ファイルを `processed/` に移動
//! 4. 発行されたトークンを呼び出し元に返す
//!
//! ## 通知の冪等性
//!
//! ファイル名にはタイムスタンプと乱数を含み、同名ファイルの再作成を防止します。
//! 処理済み通知は `processed/` に移動されるため、二重処理は発生しません。

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::waggle_integration::WaggleIntegrator;

// ============================================================================
// 通知データ型
// ============================================================================

/// sai-write ツールが書き込む通知 JSON の構造。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SaiWriteNotification {
    /// 通知タイプ（常に "file_written"）
    #[serde(rename = "type")]
    pub notification_type: String,
    /// このファイルを生成した実行の実行ID（`SAI_PARENT_RUN_ID` から取得）
    /// 完了判定（自分の run_id の sai-write 検出）に使用する。
    /// 後方互換のため未設定（None）を許容する。
    #[serde(default)]
    pub run_id: Option<i64>,
    /// 書き込まれたファイルの絶対パス
    pub file_path: String,
    /// このファイルを生成したブロック名
    pub block_name: String,
    /// 出力の説明
    #[serde(default)]
    pub description: Option<String>,
    /// MIME タイプ
    #[serde(default)]
    pub mime_type: Option<String>,
    /// ファイルサイズ（バイト）
    #[serde(default)]
    pub byte_size: Option<u64>,
    /// 通知タイムスタンプ（ISO 8601）
    #[serde(default)]
    pub timestamp: Option<String>,
    /// エージェント側（`sai_write`）で発行済みの waggle トークン（P4）。
    /// 設定されている場合、デーモンは再トークン化しない（mint はランダムなので
    /// 再トークン化するとトークンが 2 つになる）。未設定なら従来どおり
    /// デーモンがトークン化する。
    #[serde(default)]
    pub waggle_token: Option<String>,
}

// ============================================================================
// 通知処理結果
// ============================================================================

/// 1件の通知処理結果。
#[derive(Debug, Clone)]
pub struct ProcessedNotification {
    /// 処理した通知ファイルのパス
    pub notification_path: PathBuf,
    /// 書き込まれた元ファイルのパス
    pub file_path: PathBuf,
    /// 通知に含まれていた実行ID（`SAI_PARENT_RUN_ID`。未設定なら None）
    /// デーモンの完了判定（監視中の実行との照合）に使用する。
    pub run_id: Option<i64>,
    /// 発行された waggle トークン（トークン化成功時）
    pub waggle_token: Option<String>,
    /// トークン化エラー（失敗時、トークン化以外のエラーでも token は None）
    pub error: Option<String>,
}

// ============================================================================
// 通知ディレクトリ管理（P17・5.5 の修正: 定数は common に一元化）
// ============================================================================

/// 通知ディレクトリのベースパス（プロジェクトルート基準）を取得する。
pub fn notification_base_dir() -> PathBuf {
    crate::common::notify_base_dir()
}

/// 未処理通知ディレクトリのパスを取得し、存在しなければ作成する。
pub fn ensure_pending_dir() -> Result<PathBuf, std::io::Error> {
    let dir = crate::common::notify_pending_dir();
    fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// 処理済み通知ディレクトリのパスを取得し、存在しなければ作成する。
pub fn ensure_processed_dir() -> Result<PathBuf, std::io::Error> {
    let dir = crate::common::notify_processed_dir();
    fs::create_dir_all(&dir)?;
    Ok(dir)
}

// ============================================================================
// 通知のポーリングと処理
// ============================================================================

/// sai-write の保存先を run フォルダ内に解決する（R503・R504）。
///
/// `filename` のディレクトリ部分・`../` は除去し、自 run フォルダ直下に誘導する。
/// run フォルダが無ければ `running` owner に作成する。
/// 親ディレクトリも作成し、書き込み可能な絶対パスを返す。
pub fn resolve_output_path(run_id: i64, filename: &str) -> std::io::Result<PathBuf> {
    let safe = crate::run_dir::sanitize_filename(filename);
    let dir = crate::run_dir::ensure_run_dir(crate::run_dir::RUNNING_OWNER, run_id)?;
    Ok(dir.join(safe))
}

/// 未処理の通知ファイルを1件読み取り、処理する（なければ None を返す）。
///
/// この関数は 1回のポーリングで **最大1件** の通知を処理します。
/// 複数件ある場合は次回のポーリングで処理されます。
///
/// # 引数
///
/// * `integrator` — waggle トークン化に使用するインテグレーター
///
/// # 戻り値
///
/// * `Some(ProcessedNotification)` — 通知を処理した
/// * `None` — 未処理通知がない
pub fn poll_and_process_one(
    integrator: &WaggleIntegrator,
) -> Result<Option<ProcessedNotification>, std::io::Error> {
    let pending_dir = ensure_pending_dir()?;
    let processed_dir = ensure_processed_dir()?;

    // pending ディレクトリ内の .json ファイルを列挙
    let mut entries: Vec<_> = fs::read_dir(&pending_dir)?
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.path()
                .extension()
                .map(|ext| ext == "json")
                .unwrap_or(false)
        })
        .collect();

    // ファイル名でソート（古い順に処理）
    entries.sort_by_key(|e| e.file_name());

    // 最初の file_written 通知を処理する
    // （task_created / task_execute_request / task_output_used などの
    //   `task_*` 通知は task_manager::poll_one が処理するため、ここでは
    //   スキップして pending/ に残す）
    for entry in entries {
        let notification_path = entry.path();
        if let Ok(content) = fs::read_to_string(&notification_path) {
            if let Ok(value) = serde_json::from_str::<serde_json::Value>(&content) {
                let notif_type = value.get("type").and_then(|v| v.as_str()).unwrap_or("");
                if notif_type != "file_written" {
                    // task_* 通知 → task_manager に任せる（処理しない・移動もしない）
                    continue;
                }
            }
        }
        let result = process_notification_file(&notification_path, integrator, &processed_dir);
        return Ok(Some(result));
    }

    Ok(None)
}

/// 未処理の通知をすべて処理する。
///
/// # 引数
///
/// * `integrator` — waggle トークン化に使用するインテグレーター
///
/// # 戻り値
///
/// 処理された通知のリスト
pub fn poll_and_process_all(
    integrator: &WaggleIntegrator,
) -> Result<Vec<ProcessedNotification>, std::io::Error> {
    let mut results = Vec::new();
    loop {
        match poll_and_process_one(integrator)? {
            Some(result) => results.push(result),
            None => break,
        }
    }
    Ok(results)
}

// ============================================================================
// 内部: 通知ファイルの処理
// ============================================================================

/// 1つの通知ファイルを読み取り、トークン化し、処理済みに移動する。
fn process_notification_file(
    notification_path: &Path,
    integrator: &WaggleIntegrator,
    processed_dir: &Path,
) -> ProcessedNotification {
    // 1. 通知 JSON を読み取り
    let content = match fs::read_to_string(notification_path) {
        Ok(c) => c,
        Err(e) => {
            let result = ProcessedNotification {
                notification_path: notification_path.to_path_buf(),
                file_path: PathBuf::new(),
                run_id: None,
                waggle_token: None,
                error: Some(format!("通知ファイルの読み取りに失敗: {}", e)),
            };
            move_to_processed(notification_path, processed_dir);
            return result;
        }
    };

    // 2. JSON をパース
    let notification: SaiWriteNotification = match serde_json::from_str(&content) {
        Ok(n) => n,
        Err(e) => {
            let result = ProcessedNotification {
                notification_path: notification_path.to_path_buf(),
                file_path: PathBuf::new(),
                run_id: None,
                waggle_token: None,
                error: Some(format!("通知 JSON のパースに失敗: {}", e)),
            };
            move_to_processed(notification_path, processed_dir);
            return result;
        }
    };

    let file_path = PathBuf::from(&notification.file_path);
    let run_id = notification.run_id;

    // 3. ファイルの存在確認
    if !file_path.exists() {
        let result = ProcessedNotification {
            notification_path: notification_path.to_path_buf(),
            file_path,
            run_id,
            waggle_token: None,
            error: Some(format!(
                "通知されたファイルが存在しません: {}",
                notification.file_path
            )),
        };
        move_to_processed(notification_path, processed_dir);
        return result;
    }

    // 3.5 エージェント側で発行済みのトークンがあれば再トークン化しない（P4）
    if let Some(token) = notification.waggle_token.clone() {
        let result = ProcessedNotification {
            notification_path: notification_path.to_path_buf(),
            file_path,
            run_id,
            waggle_token: Some(token),
            error: None,
        };
        move_to_processed(notification_path, processed_dir);
        return result;
    }

    // 4. waggle トークン化
    match integrator.tokenize_file(
        &file_path,
        &notification.block_name,
        notification.description.as_deref(),
    ) {
        Ok(tokenized) => {
            let result = ProcessedNotification {
                notification_path: notification_path.to_path_buf(),
                file_path,
                run_id,
                waggle_token: Some(tokenized.token.as_str().to_string()),
                error: None,
            };
            move_to_processed(notification_path, processed_dir);
            result
        }
        Err(e) => {
            let result = ProcessedNotification {
                notification_path: notification_path.to_path_buf(),
                file_path,
                run_id,
                waggle_token: None,
                error: Some(format!("waggle トークン化に失敗: {}", e)),
            };
            move_to_processed(notification_path, processed_dir);
            result
        }
    }
}

/// 通知ファイルを processed ディレクトリに移動する。
///
/// 移動に失敗した場合は削除を試みる。
fn move_to_processed(source: &Path, processed_dir: &Path) {
    let filename = source.file_name().unwrap_or_default();
    let dest = processed_dir.join(filename);

    // 同名ファイルが既に存在する場合はリネーム
    let dest = if dest.exists() {
        let timestamp = timestamp_for_filename();
        processed_dir.join(format!("{}-{}", timestamp, filename.to_string_lossy()))
    } else {
        dest
    };

    if fs::rename(source, &dest).is_err() {
        // 移動に失敗した場合は削除
        let _ = fs::remove_file(source);
    }
}

/// ファイル名用のタイムスタンプを生成する。
fn timestamp_for_filename() -> String {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let secs = duration.as_secs();
    format!("{}", secs)
}

// ============================================================================
// sai-write ツール使用の検出
// ============================================================================

use crate::block_config::{BlockConfig, ToolsConfig};

/// ブロック定義が `sai-write` ツールを使用するか判定する。
///
/// `tools` フィールドに `sai-write` が含まれている場合に `true` を返します。
///
/// # ブロック定義ファイルでの指定例
///
/// ```yaml
/// tools: sai-write,bash,read
/// ```
pub fn block_uses_sai_write(config: &BlockConfig) -> bool {
    match &config.tools {
        Some(ToolsConfig::List(tools)) => tools.iter().any(|t| t == "sai-write"),
        Some(ToolsConfig::All) => {
            // tools: true の場合、sai-write が有効かどうかは
            // extensions の指定に依存する（pi-sai 拡張が必須）
            // デフォルトでは false
            false
        }
        _ => false,
    }
}

// ============================================================================
// テスト
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block_config::{BlockConfig, ToolsConfig};

    /// 通知ディレクトリ（`.sai/sai-engine/notifications`）を共有するテストを
    /// 直列化するためのロック。Rust のテストは並列実行されるため、
    /// ディレクトリの作成/削除が競合しないようにする。
    // NOTIFY_DIR_LOCK は common::SAI_ENV_TEST_LOCK に一元化（U4）

    // ---- block_uses_sai_write ----

    fn config_with_tools(tools: Option<ToolsConfig>) -> BlockConfig {
        BlockConfig {
            name: "test".to_string(),
            description: None,
            model: None,
            provider: None,
            thinking: None,
            tools,
            extensions: None,
            skills: None,
            subblocks: None,
            timeout: None,
            worktree: None,
            enable: None,
            category: None,
            tag: None,
            round: None,
            templates: None,
            review_headings: None,
            id: None,
        }
    }

    #[test]
    fn test_block_uses_sai_write_true() {
        let config = config_with_tools(Some(ToolsConfig::List(vec![
            "sai-write".to_string(),
            "bash".to_string(),
            "read".to_string(),
        ])));
        assert!(block_uses_sai_write(&config));
    }

    #[test]
    fn test_block_uses_sai_write_false() {
        let config = config_with_tools(Some(ToolsConfig::List(vec![
            "read".to_string(),
            "bash".to_string(),
            "edit".to_string(),
        ])));
        assert!(!block_uses_sai_write(&config));
    }

    #[test]
    fn test_block_uses_sai_write_none() {
        let config = config_with_tools(None);
        assert!(!block_uses_sai_write(&config));
    }

    #[test]
    fn test_block_uses_sai_write_all() {
        let config = config_with_tools(Some(ToolsConfig::All));
        assert!(!block_uses_sai_write(&config));
    }

    // ---- notification JSON のパース ----

    #[test]
    fn test_parse_notification_json() {
        let json = r#"{
            "type": "file_written",
            "file_path": "/home/user/output/report.md",
            "block_name": "sai-brst-grill",
            "description": "ブレインストーミング結果",
            "mime_type": "text/markdown",
            "byte_size": 1234,
            "timestamp": "2026-07-30T01:23:45Z"
        }"#;

        let notification: SaiWriteNotification =
            serde_json::from_str(json).expect("JSONパースに失敗");

        assert_eq!(notification.notification_type, "file_written");
        assert_eq!(notification.file_path, "/home/user/output/report.md");
        assert_eq!(notification.block_name, "sai-brst-grill");
        assert_eq!(
            notification.description.as_deref(),
            Some("ブレインストーミング結果")
        );
        assert_eq!(notification.mime_type.as_deref(), Some("text/markdown"));
        assert_eq!(notification.byte_size, Some(1234));
    }

    #[test]
    fn test_parse_notification_minimal() {
        let json = r#"{
            "type": "file_written",
            "file_path": "/tmp/out.md",
            "block_name": "test-block"
        }"#;

        let notification: SaiWriteNotification =
            serde_json::from_str(json).expect("JSONパースに失敗");

        assert_eq!(notification.notification_type, "file_written");
        assert_eq!(notification.file_path, "/tmp/out.md");
        assert_eq!(notification.block_name, "test-block");
        assert!(notification.description.is_none());
        // run_id は後方互換のため None（未指定）を許容
        assert!(notification.run_id.is_none());
    }

    /// run_id 付き通知のパース（D11・Phase 1）
    #[test]
    fn test_parse_notification_with_run_id() {
        let json = r#"{
            "type": "file_written",
            "run_id": 42,
            "file_path": "/tmp/out.md",
            "block_name": "test-block"
        }"#;

        let notification: SaiWriteNotification =
            serde_json::from_str(json).expect("JSONパースに失敗");

        assert_eq!(notification.notification_type, "file_written");
        assert_eq!(notification.run_id, Some(42));
    }

    // ---- 通知ディレクトリの作成 ----

    #[test]
    fn test_ensure_dirs_creates() {
        let _guard = crate::common::SAI_ENV_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let pending = ensure_pending_dir().expect("pendingディレクトリ作成");
        let processed = ensure_processed_dir().expect("processedディレクトリ作成");

        assert!(pending.exists());
        assert!(processed.exists());
        assert!(pending.ends_with("pending"));
        assert!(processed.ends_with("processed"));

        // クリーンアップ
        let _ = fs::remove_dir_all(notification_base_dir());
    }

    // ---- poll_and_process_one 空ディレクトリ ----

    #[test]
    fn test_poll_empty_directory() {
        let _guard = crate::common::SAI_ENV_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // 空の通知ディレクトリで poll してもエラーにならない
        let _ = ensure_pending_dir();
        let _ = ensure_processed_dir();

        // インメモリのインテグレーターでテスト
        let integrator = WaggleIntegrator::open_in_memory().expect("インメモリDBの作成に失敗");

        let result = poll_and_process_one(&integrator).expect("pollに失敗");
        assert!(result.is_none(), "空ディレクトリではNoneが返る");

        // クリーンアップ
        let _ = fs::remove_dir_all(notification_base_dir());
    }

    // ---- poll_and_process_all ----

    #[test]
    fn test_poll_and_process_all_empty() {
        let _guard = crate::common::SAI_ENV_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _ = ensure_pending_dir();
        let _ = ensure_processed_dir();

        let integrator = WaggleIntegrator::open_in_memory().expect("インメモリDBの作成に失敗");

        let results = poll_and_process_all(&integrator).expect("poll_allに失敗");
        assert!(results.is_empty());

        let _ = fs::remove_dir_all(notification_base_dir());
    }

    /// task_created 通知は処理せず pending/ に残し、
    /// file_written 通知のみを処理する。
    #[test]
    fn test_poll_skips_task_notifications() {
        let _guard = crate::common::SAI_ENV_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _ = ensure_pending_dir();
        let _ = ensure_processed_dir();
        let pending = crate::common::notify_pending_dir();

        let integrator = WaggleIntegrator::open_in_memory().expect("インメモリDBの作成に失敗");

        // 1. task_created 通知（file_written ではない）を書き込む
        let task_notif = pending.join("1000-task_created.json");
        fs::write(
            &task_notif,
            r#"{"type":"task_created","parent_run_id":14,"batch_id":"b1","tasks":[]}"#,
        )
        .unwrap();

        // 2. 実在ファイルを指す file_written 通知を書き込む
        let target_file = std::env::temp_dir().join("sai_write_test_target.md");
        fs::write(&target_file, "# テスト").unwrap();
        let file_notif = pending.join("2000-file_written.json");
        fs::write(
            &file_notif,
            format!(
                r#"{{"type":"file_written","file_path":"{}","block_name":"test","mime_type":"text/markdown","byte_size":10}}"#,
                target_file.to_string_lossy()
            ),
        )
        .unwrap();

        // 3. poll_and_process_one → file_written のみ処理される
        let result = poll_and_process_one(&integrator)
            .expect("pollに失敗")
            .expect("file_written 通知が処理されるべき");
        assert!(result.waggle_token.is_some(), "waggle トークンが発行される");

        // 4. task_created 通知は pending/ に残っている
        assert!(
            task_notif.exists(),
            "task_created 通知は task_manager 用に pending/ に残すべき"
        );
        // file_written 通知は processed/ に移動済み
        assert!(
            !file_notif.exists(),
            "file_written 通知は処理済みに移動する"
        );

        // クリーンアップ
        let _ = fs::remove_dir_all(notification_base_dir());
        let _ = fs::remove_file(&target_file);
    }

    /// P4: 通知に waggle_token があればデーモンは再トークン化しない。
    #[test]
    fn test_notification_with_token_skips_tokenize() {
        let _guard = crate::common::SAI_ENV_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _ = ensure_pending_dir();
        let _ = ensure_processed_dir();
        let pending = crate::common::notify_pending_dir();

        let integrator = WaggleIntegrator::open_in_memory().expect("インメモリDBの作成に失敗");

        let target_file = std::env::temp_dir().join("sai_write_skip_tokenize_test.md");
        fs::write(&target_file, "# テスト").unwrap();
        let notif = pending.join("3000-file_written.json");
        fs::write(
            &notif,
            format!(
                r#"{{"type":"file_written","run_id":7,"file_path":"{}","block_name":"test","waggle_token":"AXANSMpo"}}"#,
                target_file.to_string_lossy()
            ),
        )
        .unwrap();

        let result = poll_and_process_one(&integrator)
            .expect("pollに失敗")
            .expect("file_written 通知が処理されるべき");
        assert_eq!(result.waggle_token.as_deref(), Some("AXANSMpo"));
        assert_eq!(result.run_id, Some(7));
        assert!(result.error.is_none());
        // 再トークン化していない → インメモリ DB にこのトークンは存在しない
        assert!(
            integrator.resolve_token("AXANSMpo").is_err(),
            "通知のトークンをそのまま使い、再 mint しない"
        );
        assert!(!notif.exists(), "処理済みに移動する");

        // クリーンアップ
        let _ = fs::remove_dir_all(notification_base_dir());
        let _ = fs::remove_file(&target_file);
    }
}
