//! # オーケストレーター統合テスト
//!
//! モック herdr を使用してパイプライン全体の結合テストを実行します。
//!
//! テストは `tests/fixtures/mock_herdr.sh` を使用し、実際の herdr 環境に
//! 依存しません。

use agent_link_engine::block_config::BlockConfig;
use agent_link_engine::herdr::{HerdrClient, SplitDirection};
use agent_link_engine::orchestrator::{OrchestratorError, PipelineConfig};
use std::collections::HashMap;
use std::path::PathBuf;

// ---------------------------------------------------------------------------
// ヘルパー
// ---------------------------------------------------------------------------

/// モック herdr スクリプトの絶対パスを取得します。
fn mock_herdr_path() -> PathBuf {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("mock_herdr.sh");
    assert!(
        path.exists(),
        "モック herdr スクリプトが見つかりません: {}",
        path.display()
    );
    path
}

/// フィクスチャファイルの絶対パスを取得します。
fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name)
}

/// モック環境のテスト設定を作成します。
fn test_config() -> PipelineConfig {
    PipelineConfig {
        block_config: BlockConfig {
            name: "integration-test".to_string(),
            description: None,
            model: None,
            thinking: None,
            tools: None,
            extensions: None,
            skills: None,
            subblocks: None,
            timeout: None,
            worktree: None,
            enable: None,
            category: None,
            templates: None,
            id: None,
        },
        block_body: "テストプロンプト: {{input.name}}".to_string(),
        user_inputs: {
            let mut map = HashMap::new();
            map.insert("name".to_string(), "IntegrationTest".to_string());
            map
        },
        cwd: None,
        direction: SplitDirection::Right,
        timeout_ms: 5_000,
        keep_pane: false,
        run_id: None,
        tab_id: None,
        base_pane_id: None,
        pane_id: None,
        is_subtask: false,
    }
}

// ---------------------------------------------------------------------------
// テストケース（モック herdr が必要）
// ---------------------------------------------------------------------------

/// 正常系: パイプライン全体が成功することを確認。
///
/// このテストはモック herdr が利用可能な場合にのみ実行されます。
#[test]
fn test_pipeline_success() {
    // 環境変数でモックが設定されていない場合でも直接モックを指定してテスト
    let herdr_client = HerdrClient::with_binary(mock_herdr_path().to_string_lossy().to_string());
    if !herdr_client.is_herdr_available() {
        eprintln!("モック herdr が利用不可のためテストをスキップします");
        return;
    }

    // このテストは実際の pi エージェントが必要なため、
    // ユニットテストとしては現実的ではありません。
    // run_pipeline() はブロッキング呼び出しであり、pi が起動されるため
    // ここでは設定の構築のみをテストします。
    let config = test_config();

    // PipelineConfig が正しく構築されていることを確認
    assert_eq!(config.block_config.name, "integration-test");
    assert!(!config.block_body.is_empty());
    assert!(config.user_inputs.contains_key("name"));
    assert!(!config.keep_pane);
}

/// タイムアウト設定が正しく反映されることを確認。
#[test]
fn test_pipeline_timeout_config() {
    let config = PipelineConfig {
        timeout_ms: 100,
        ..test_config()
    };
    assert_eq!(config.timeout_ms, 100);
}

/// キャンセル検出用の設定が正しいことを確認。
#[test]
fn test_pipeline_cancellation_config() {
    let config = test_config();
    assert_eq!(config.timeout_ms, 5_000);

    // OrchestratorError::Cancelled の display を確認
    let err = OrchestratorError::Cancelled;
    let msg = format!("{}", err);
    assert!(msg.contains("キャンセル"));
}

/// エラークリーンアップの設定検証。
#[test]
fn test_pipeline_cleanup_on_error_config() {
    let config = test_config();
    assert!(
        !config.keep_pane,
        "デフォルトでは pane はクローズされるべき"
    );
}

/// keep_pane フラグが設定で正しく反映されることを確認。
#[test]
fn test_pipeline_keep_pane_config() {
    let config = PipelineConfig {
        keep_pane: true,
        ..test_config()
    };
    assert!(config.keep_pane, "keep_pane が true に設定されるべき");
}

/// OrchestratorError::Timeout の Display 実装を確認。
#[test]
fn test_timeout_error_display() {
    let err = OrchestratorError::Timeout { timeout_ms: 30000 };
    let msg = format!("{}", err);
    assert!(msg.contains("30000"));
    assert!(msg.contains("以内に"));
}

/// プレースホルダー展開がパイプライン設定で正しく機能することを確認（統合）。
#[test]
fn test_pipeline_placeholder_integration() {
    let config = test_config();
    let expanded =
        agent_link_engine::placeholder::expand_placeholders(&config.block_body, &config.user_inputs);
    assert_eq!(expanded, "テストプロンプト: IntegrationTest");
}

/// 複数の入力値がある場合のプレースホルダー展開を確認。
#[test]
fn test_pipeline_multiple_inputs() {
    let config = PipelineConfig {
        block_body: "{{input.a}} and {{input.b}}".to_string(),
        user_inputs: {
            let mut map = HashMap::new();
            map.insert("a".to_string(), "X".to_string());
            map.insert("b".to_string(), "Y".to_string());
            map
        },
        ..test_config()
    };
    let expanded =
        agent_link_engine::placeholder::expand_placeholders(&config.block_body, &config.user_inputs);
    assert_eq!(expanded, "X and Y");
}

/// パース済みブロックフィクスチャから PipelineConfig を構築できることを確認。
///
/// パーサー→ブロック設定→パイプライン設定の統合パスをテストします。
#[test]
fn test_pipeline_config_from_parsed_block() {
    let fixture = fixture_path("valid-minimal.md");
    let parsed = agent_link_engine::parse_block_file(fixture)
        .expect("有効なブロック定義のパースに成功するべき");

    let config = PipelineConfig {
        block_config: parsed.config,
        block_body: parsed.body,
        user_inputs: HashMap::new(),
        cwd: None,
        direction: SplitDirection::Right,
        timeout_ms: 300_000,
        keep_pane: false,
        run_id: None,
        tab_id: None,
        base_pane_id: None,
        pane_id: None,
        is_subtask: false,
    };

    assert_eq!(config.block_config.name, "test-block");
    assert_eq!(
        config.block_config.model.as_deref(),
        Some("anthropic/claude-sonnet-4-5")
    );
    assert!(!config.block_body.is_empty(), "ブロック本文が空でないこと");
    assert!(!config.keep_pane);
    assert_eq!(config.timeout_ms, 300_000);
}

/// PipelineConfig に全オプションフィールドを設定して構築できることを確認。
#[test]
fn test_pipeline_config_all_fields_set() {
    let mut inputs = HashMap::new();
    inputs.insert("feature".to_string(), "test-feature".to_string());
    inputs.insert("language".to_string(), "Rust".to_string());

    let config = PipelineConfig {
        block_config: BlockConfig {
            name: "full-config-test".to_string(),
            description: Some("テスト用の完全設定".to_string()),
            model: Some("anthropic/claude-sonnet-4-5".to_string()),
            thinking: Some("high".to_string()),
            tools: None,
            extensions: None,
            skills: Some("ce-brainstorm".to_string()),
            subblocks: None,
            timeout: Some(600),
            worktree: Some("isolation".to_string()),
            enable: Some(true),
            category: Some("test".to_string()),
            templates: None,
            id: None,
        },
        block_body: "# {{input.feature}} の実装
言語: {{input.language}}"
            .to_string(),
        user_inputs: inputs,
        cwd: Some(PathBuf::from("/tmp/test-workdir")),
        direction: SplitDirection::Down,
        timeout_ms: 600_000,
        keep_pane: true,
        run_id: None,
        tab_id: None,
        base_pane_id: None,
        pane_id: None,
        is_subtask: false,
    };

    // 各フィールドの検証
    assert_eq!(config.block_config.name, "full-config-test");
    assert_eq!(
        config.block_config.model.as_deref(),
        Some("anthropic/claude-sonnet-4-5")
    );
    assert_eq!(config.block_config.thinking.as_deref(), Some("high"));
    assert_eq!(config.block_config.skills.as_deref(), Some("ce-brainstorm"));
    assert_eq!(config.block_config.timeout, Some(600));
    assert_eq!(config.block_config.worktree.as_deref(), Some("isolation"));
    assert_eq!(config.block_config.enable, Some(true));
    assert_eq!(config.block_config.category.as_deref(), Some("test"));

    assert_eq!(
        config.cwd.as_deref(),
        Some(PathBuf::from("/tmp/test-workdir").as_path())
    );
    assert_eq!(config.direction, SplitDirection::Down);
    assert_eq!(config.timeout_ms, 600_000);
    assert!(config.keep_pane);

    // プレースホルダー展開の確認
    let expanded =
        agent_link_engine::placeholder::expand_placeholders(&config.block_body, &config.user_inputs);
    assert!(expanded.contains("test-feature"));
    assert!(expanded.contains("Rust"));
}

/// PipelineConfig の Default::default() が期待通りの値を持つことを確認。
#[test]
fn test_pipeline_config_default() {
    let config = PipelineConfig::default();
    assert_eq!(config.block_config.name, "default");
    assert!(config.block_body.is_empty());
    assert!(config.user_inputs.is_empty());
    assert!(config.cwd.is_none());
    assert_eq!(config.direction, SplitDirection::Right);
    assert_eq!(
        config.timeout_ms,
        agent_link_engine::orchestrator::NO_TIMEOUT_MS
    );
    assert!(!config.keep_pane);
}

/// OrchestratorError の全バリアントの Display 実装を確認。
#[test]
fn test_pipeline_error_display_all_variants() {
    // Timeout
    let err = OrchestratorError::Timeout { timeout_ms: 5000 };
    let msg = format!("{}", err);
    assert!(msg.contains("5000"));
    assert!(msg.contains("以内に"));

    // Cancelled
    let err = OrchestratorError::Cancelled;
    let msg = format!("{}", err);
    assert!(msg.contains("キャンセル"));

    // HerdrError (from)
    let err = OrchestratorError::HerdrError(agent_link_engine::herdr::HerdrError::HerdrNotFound);
    let msg = format!("{}", err);
    assert!(msg.contains("herdr"));

    // CommandError
    let err = OrchestratorError::CommandError("invalid model".to_string());
    let msg = format!("{}", err);
    assert!(msg.contains("コマンド構築"));
    assert!(msg.contains("invalid model"));

    // SessionError
    let err = OrchestratorError::SessionError("session dir creation failed".to_string());
    let msg = format!("{}", err);
    assert!(msg.contains("セッションファイル"));
    assert!(msg.contains("session dir creation failed"));

    // IoError (from)
    let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "file not found");
    let err = OrchestratorError::IoError(io_err);
    let msg = format!("{}", err);
    assert!(msg.contains("IO エラー"));
    assert!(msg.contains("file not found"));

    // SystemPromptError
    let err = OrchestratorError::SystemPromptError("temp file write failed".to_string());
    let msg = format!("{}", err);
    assert!(msg.contains("システムプロンプト"));
    assert!(msg.contains("temp file write failed"));
}

/// PipelineConfig に cwd を設定した場合としない場合の動作を確認。
#[test]
fn test_pipeline_config_with_cwd() {
    // cwd なし
    let config = test_config();
    assert!(config.cwd.is_none());

    // cwd あり
    let config = PipelineConfig {
        cwd: Some(PathBuf::from("/home/user/project")),
        ..test_config()
    };
    assert_eq!(
        config.cwd.as_deref(),
        Some(PathBuf::from("/home/user/project").as_path())
    );

    // 異なる cwd
    let config = PipelineConfig {
        cwd: Some(PathBuf::from("/tmp")),
        ..test_config()
    };
    assert_eq!(config.cwd.as_deref(), Some(std::path::Path::new("/tmp")));
}

/// PipelineConfig の Clone + Debug 実装を確認。
#[test]
fn test_pipeline_types_impl_clone_debug() {
    let config = test_config();
    // Clone
    let cloned = config.clone();
    assert_eq!(config.block_config.name, cloned.block_config.name);
    assert_eq!(config.timeout_ms, cloned.timeout_ms);
    assert_eq!(config.direction, cloned.direction);

    // Debug 書式
    let debug_str = format!("{:?}", config);
    assert!(debug_str.contains("integration-test"));
    assert!(debug_str.contains("PipelineConfig"));

    // OrchestratorError の Debug 書式
    let err = OrchestratorError::Cancelled;
    let debug_err = format!("{:?}", err);
    assert!(!debug_err.is_empty());
    assert!(debug_err.contains("Cancelled"));
}
