//! # 共通ユーティリティ
//!
//! sai-engine / sai-tui で重複していた処理を一元化するモジュール（P12）。
//!
//! - `extract_frontmatter_name` — ブロック定義の frontmatter から `name` を抽出
//! - `collect_block_files` — ブロック定義ディレクトリの収集（ローカル優先 → グローバル `~/.sai/blocks`）
//! - `open_db` — sai-engine DB をプロジェクトルート基準で開く（P2）
//! - `load_available_models` — `~/.pi/agent/models.json` から `provider/model` 形式のモデル一覧を取得
//! - 日付時刻・表示フォーマットのユーティリティ（chrono 使用）
//!
//! ## ブロック定義の探索パス（A10）
//!
//! ローカル（`PROJECTROOT/.sai/blocks` 等）を優先し、同名ブロックはローカルが
//! グローバル（`~/.sai/blocks`）を上書きする。グローバルのみに存在するブロックも
//! 参照可能にするため、両方をマージして収集する。

use std::collections::HashSet;
use std::path::PathBuf;

use chrono::{SecondsFormat, Utc};

use crate::db::SaiStore;

// ============================================================================
// 終了コードの分類（P18・5.7 の修正）
// ============================================================================

/// 正常終了。
pub const EXIT_OK: i32 = 0;
/// 一般的な利用エラー（引数不足・前提条件違反など）。旧来の終了コード 1 と互換。
pub const EXIT_USAGE: i32 = 1;
/// ブロック定義のパース・バリデーションエラー。
pub const EXIT_PARSE: i32 = 2;
/// ファイル・ブロックが見つからない。
pub const EXIT_NOT_FOUND: i32 = 3;
/// DB 初期化・操作エラー。
pub const EXIT_DB: i32 = 4;

// ============================================================================
// テスト専用の共有ロック（U4）
// ============================================================================

/// テスト専用: 環境変数（SAI_PROJECT_ROOT）と通知ディレクトリを操作・参照する
/// テストの直列化ロック。
///
/// `SAI_PROJECT_ROOT` を設定するテスト（run_dir / agent_api / cc_command / task_cwd）、
/// 通知ディレクトリを操作するテスト（doc_write_notify）、pi コマンド文字列など
/// 環境変数の影響を受ける結果を検証するテスト（orchestrator の `finalize_env_lock`）
/// が共有する。実行順で環境が変わると assert が偶発失敗するため一元化した。
#[cfg(test)]
pub(crate) static SAI_ENV_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

// ============================================================================
// ブロック定義の収集
// ============================================================================

/// ブロック定義ディレクトリの探索候補（優先度順）。
///
/// ローカル優先 → グローバル `~/.sai/blocks`（A10）。
fn block_dir_candidates() -> Vec<PathBuf> {
    let mut candidates = vec![
        crate::project::sai_path(".sai/blocks"),
        PathBuf::from("blocks"),
        PathBuf::from("../blocks"),
        PathBuf::from("../../blocks"),
        PathBuf::from("../engine/blocks"),
        // sai-engine の blocks_ ディレクトリ（後方互換）
        PathBuf::from("crates/engine/blocks_"),
        PathBuf::from("../crates/engine/blocks_"),
    ];
    // グローバル設定 ~/.sai/blocks（ローカルより優先度が低い = 最後に探索）
    if let Some(dir) = crate::project::sai_home_path("blocks") {
        candidates.push(dir);
    }
    candidates
}

/// ブロック定義ディレクトリから `.md` ファイル一覧を収集する。
///
/// 戻り値: `Vec<(表示名, ファイルパス)>`。
/// 同名ブロックは探索順の早いもの（ローカル優先）が採用され、
/// グローバル `~/.sai/blocks` の同名ブロックは上書きされない。
pub fn collect_block_files() -> Vec<(String, String)> {
    let mut result: Vec<(String, String)> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();

    for dir in block_dir_candidates() {
        if dir.is_dir() {
            if let Ok(entries) = std::fs::read_dir(&dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.extension().map(|e| e == "md").unwrap_or(false) {
                        if let Ok(content) = std::fs::read_to_string(&path) {
                            let name = extract_frontmatter_name(&content).unwrap_or_else(|| {
                                path.file_stem()
                                    .map(|s| s.to_string_lossy().to_string())
                                    .unwrap_or_default()
                            });
                            // 同名は先に見つかった（優先度の高い）ものを採用
                            if seen.insert(name.clone()) {
                                result.push((name, path.to_string_lossy().to_string()));
                            }
                        }
                    }
                }
            }
        }
    }
    // P1-1: 表示名（name）昇順で安定ソート（読み込み順に依存しない）
    result.sort_by(|a, b| a.0.cmp(&b.0));
    result
}

/// 収集結果を表示名（name）昇順で安定ソートする（P1-1）。
///
/// `collect_block_files` の返却直前に呼び出され、探索ディレクトリの
/// 読み込み順に依存しない安定した一覧表示を提供する。
/// テスト容易性のため分離している。
pub fn sort_block_files_by_name(mut result: Vec<(String, String)>) -> Vec<(String, String)> {
    result.sort_by(|a, b| a.0.cmp(&b.0));
    result
}

/// Markdown ファイルの frontmatter から `name` フィールドを抽出する。
pub fn extract_frontmatter_name(content: &str) -> Option<String> {
    let mut lines = content.lines();
    if lines.next()?.trim() != "---" {
        return None;
    }
    for line in lines {
        let trimmed = line.trim();
        if trimmed == "---" {
            break;
        }
        if let Some(value) = trimmed
            .strip_prefix("name:")
            .or_else(|| trimmed.strip_prefix("name :"))
        {
            return Some(value.trim().to_string());
        }
    }
    None
}

// ============================================================================
// DB オープン（P2: パス解決の統一）
// ============================================================================

/// sai-engine の DB（`PROJECTROOT/.sai/sai-engine/sai-engine.db`）を開く。
///
/// `project::sai_path` 基準のため、worktree 実行時（cwd が `.worktrees/<branch>`）でも
/// 常にプロジェクトルートの DB を参照する（1.8 / 4.1.3 の修正）。
pub fn open_db() -> Result<SaiStore, Box<dyn std::error::Error>> {
    let db_path = crate::project::sai_path(".sai/sai-engine/sai-engine.db");
    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let store = SaiStore::open(&db_path)?;
    Ok(store)
}

// ============================================================================
// モデル一覧（`~/.pi/agent/models.json` 基準）
// ============================================================================

/// pi agent が使用するモデル設定（`~/.pi/agent/models.json`）のパスを返す。
///
/// 環境変数 `PI_CONFIG_DIR` が設定されている場合は `<dir>/agent/models.json`、
/// 未設定の場合は `~/.pi/agent/models.json` を使用する。
pub fn pi_models_json_path() -> PathBuf {
    if let Some(dir) = std::env::var_os("PI_CONFIG_DIR") {
        let pb = PathBuf::from(dir).join("agent").join("models.json");
        if !pb.as_os_str().is_empty() {
            return pb;
        }
    }
    if let Some(home) = std::env::var_os("HOME") {
        return PathBuf::from(home)
            .join(".pi")
            .join("agent")
            .join("models.json");
    }
    PathBuf::from(".pi/agent/models.json")
}

/// `~/.pi/agent/models.json` から、pi agent で選択可能なモデル一覧を
/// **`provider/model` 形式**で読み込む（`pi --list-models` と同じリスト）。
///
/// JSON 構造:
///
/// ```json
/// { "providers": { "<provider>": { "models": [ { "id": "<model-id>" } ] } } }
/// ```
///
/// ファイルが存在しない・パースに失敗する場合は空リストを返す。
pub fn load_available_models() -> Vec<String> {
    let path = pi_models_json_path();
    let Ok(content) = std::fs::read_to_string(&path) else {
        return Vec::new();
    };
    let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&content) else {
        return Vec::new();
    };

    let mut models = Vec::new();
    if let Some(providers) = parsed.get("providers").and_then(|v| v.as_object()) {
        for (provider_name, provider_config) in providers {
            if let Some(model_list) = provider_config.get("models").and_then(|v| v.as_array()) {
                for model_entry in model_list {
                    if let Some(id) = model_entry.get("id").and_then(|v| v.as_str()) {
                        // `provider/model` 形式（pi の --model と同じ指定形式）
                        let full = format!("{}/{}", provider_name, id);
                        if !models.contains(&full) {
                            models.push(full);
                        }
                    }
                }
            }
        }
    }
    models
}

// ============================================================================
// 通知ディレクトリ定数（P17・5.5 の修正: 一元化）
// ============================================================================

/// 通知ディレクトリのベースパス（プロジェクトルートからの相対パス）。
pub const NOTIFY_BASE_REL: &str = ".sai/sai-engine/notifications";
/// 未処理通知の格納先ディレクトリ名。
pub const NOTIFY_PENDING: &str = "pending";
/// 処理済み通知の格納先ディレクトリ名。
pub const NOTIFY_PROCESSED: &str = "processed";
/// 完了結果の格納先ディレクトリ名。
pub const NOTIFY_RESULTS: &str = "results";

/// 通知ディレクトリのベースパスをプロジェクトルート基準で返す。
pub fn notify_base_dir() -> PathBuf {
    crate::project::sai_path(NOTIFY_BASE_REL)
}

/// 未処理通知ディレクトリのパスを返す。
pub fn notify_pending_dir() -> PathBuf {
    notify_base_dir().join(NOTIFY_PENDING)
}

/// 処理済み通知ディレクトリのパスを返す。
pub fn notify_processed_dir() -> PathBuf {
    notify_base_dir().join(NOTIFY_PROCESSED)
}

/// 完了結果ディレクトリのパスを返す。
pub fn notify_results_dir() -> PathBuf {
    notify_base_dir().join(NOTIFY_RESULTS)
}

// ============================================================================
// 日付時刻（chrono 使用・P12）
// ============================================================================

/// 現在時刻の ISO 8601 タイムスタンプ（例: `2026-07-30T14:23:05Z`）。
pub fn iso_8601_timestamp() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true)
}

/// ディレクトリ名用タイムスタンプ（例: `20260730T142536`）。
pub fn timestamp_for_dirname() -> String {
    Utc::now().format("%Y%m%dT%H%M%S").to_string()
}

/// ファイル名用タイムスタンプ（例: `2026-07-30T14-23-05`）。
pub fn timestamp_for_filename() -> String {
    Utc::now().format("%Y-%m-%dT%H-%M-%S").to_string()
}

// ============================================================================
// 表示フォーマット
// ============================================================================

/// ISO 8601 文字列を `YYYY-MM-DD HH:MM:SS` 形式に変換する。
pub fn format_datetime(iso: &str) -> String {
    if iso.len() >= 19 {
        let mut result = iso.to_string();
        if let Some(t_pos) = result.find('T') {
            result.replace_range(t_pos..=t_pos, " ");
        }
        if result.len() > 19 {
            result.truncate(19);
        }
        result
    } else {
        iso.to_string()
    }
}

/// ミリ秒を人間可読形式に変換する。
pub fn format_elapsed(ms: Option<i64>) -> String {
    match ms {
        None => "-".to_string(),
        Some(v) if v < 1000 => format!("{}ms", v),
        Some(v) if v < 60_000 => {
            let secs = v as f64 / 1000.0;
            format!("{:.1}s", secs)
        }
        Some(v) => {
            let mins = v / 60_000;
            let secs = (v % 60_000) as f64 / 1000.0;
            format!("{}m {:.0}s", mins, secs)
        }
    }
}

/// 文字列を指定文字数で切り詰め、超過時に `...` を付与する。
pub fn truncate_str(s: &str, max_chars: usize) -> String {
    if s.chars().count() > max_chars {
        let truncated: String = s.chars().take(max_chars).collect();
        format!("{}...", truncated)
    } else {
        s.to_string()
    }
}

// ============================================================================
// テスト
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_frontmatter_name() {
        let content = "---\nname: my-block\ndescription: test\n---\n# body";
        assert_eq!(
            extract_frontmatter_name(content),
            Some("my-block".to_string())
        );

        let no_frontmatter = "# body only";
        assert_eq!(extract_frontmatter_name(no_frontmatter), None);
    }

    #[test]
    fn test_timestamp_formats() {
        let iso = iso_8601_timestamp();
        assert!(iso.starts_with("20"), "ISO8601 は 20xx で始まる: {}", iso);
        assert_eq!(iso.len(), 20, "例: 2026-07-30T14:23:05Z (20文字)");

        let dirname = timestamp_for_dirname();
        assert_eq!(dirname.len(), 15, "例: 20260730T142536");

        let fname = timestamp_for_filename();
        assert_eq!(fname.len(), 19, "例: 2026-07-30T14-23-05");
    }

    #[test]
    fn test_format_datetime() {
        assert_eq!(
            format_datetime("2026-07-30T14:23:05"),
            "2026-07-30 14:23:05"
        );
        assert_eq!(
            format_datetime("2026-07-30T14:23:05+09:00"),
            "2026-07-30 14:23:05"
        );
        assert_eq!(format_datetime(""), "");
    }

    #[test]
    fn test_format_elapsed() {
        assert_eq!(format_elapsed(None), "-");
        assert_eq!(format_elapsed(Some(500)), "500ms");
        assert_eq!(format_elapsed(Some(45200)), "45.2s");
        assert_eq!(format_elapsed(Some(90000)), "1m 30s");
    }

    #[test]
    fn test_truncate_str() {
        assert_eq!(truncate_str("hello", 10), "hello");
        assert_eq!(
            truncate_str("hello world this is long", 10),
            "hello worl..."
        );
    }

    /// P1-1: 収集結果が表示名（name）昇順になることを確認する。
    ///
    /// `collect_block_files` は探索パス（cwd）に依存するため、
    /// ソートロジックを共有するヘルパー関数 `sort_block_files_by_name` を
    /// 直接検証する（実装は `collect_block_files` の返却直前で使用）。
    #[test]
    fn test_block_files_sorted_ascending() {
        let files = vec![
            ("zebra".to_string(), "/tmp/z.md".to_string()),
            ("alpha".to_string(), "/tmp/a.md".to_string()),
            ("beta".to_string(), "/tmp/b.md".to_string()),
            ("alpha".to_string(), "/tmp/a2.md".to_string()), // 同名は最初の位置を維持（安定ソート）
        ];
        let sorted = sort_block_files_by_name(files);
        assert_eq!(sorted[0].0, "alpha");
        assert_eq!(sorted[1].0, "alpha");
        assert_eq!(sorted[2].0, "beta");
        assert_eq!(sorted[3].0, "zebra");
    }

    // -------------------------------------------------------------------
    // `~/.pi/agent/models.json` からのモデル一覧取得
    // -------------------------------------------------------------------

    /// PI_CONFIG_DIR 環境変数を操作するテストの直列化ロック（環境干渉防止）。
    static PI_CONFIG_DIR_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// テスト用の models.json を書き込み、`load_available_models()` が
    /// `provider/model` 形式の一覧を返すことを検証する。
    #[test]
    fn test_load_available_models_provider_slash_id() {
        let _guard = PI_CONFIG_DIR_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = std::env::temp_dir().join(format!(
            "sai_models_test_{}_{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        let agent_dir = tmp.join("agent");
        std::fs::create_dir_all(&agent_dir).unwrap();
        std::fs::write(
            agent_dir.join("models.json"),
            r#"{
                "providers": {
                    "prov-a": { "models": [ { "id": "model-1" }, { "id": "model-2" } ] },
                    "prov-b": { "models": [ { "id": "model-3" } ] }
                }
            }"#,
        )
        .unwrap();

        // 既存の環境変数を退避してテスト用パスへ向ける
        let saved = std::env::var("PI_CONFIG_DIR").ok();
        std::env::set_var("PI_CONFIG_DIR", &tmp);

        let models = super::load_available_models();

        // 復元（テスト失敗時でも環境を汚さない）
        match saved {
            Some(v) => std::env::set_var("PI_CONFIG_DIR", v),
            None => std::env::remove_var("PI_CONFIG_DIR"),
        }
        let _ = std::fs::remove_dir_all(&tmp);

        assert_eq!(models.len(), 3);
        assert!(models.contains(&"prov-a/model-1".to_string()));
        assert!(models.contains(&"prov-a/model-2".to_string()));
        assert!(models.contains(&"prov-b/model-3".to_string()));
    }

    /// models.json が存在しない場合は空リストを返す（旧実装のデフォルト候補は廃止）。
    #[test]
    fn test_load_available_models_missing_file_returns_empty() {
        let _guard = PI_CONFIG_DIR_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = std::env::temp_dir().join(format!(
            "sai_models_none_{}_{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        std::fs::create_dir_all(&tmp).unwrap();

        let saved = std::env::var("PI_CONFIG_DIR").ok();
        std::env::set_var("PI_CONFIG_DIR", &tmp);
        let models = super::load_available_models();
        match saved {
            Some(v) => std::env::set_var("PI_CONFIG_DIR", v),
            None => std::env::remove_var("PI_CONFIG_DIR"),
        }
        let _ = std::fs::remove_dir_all(&tmp);

        assert!(models.is_empty());
    }
}
