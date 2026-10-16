//! バリデーションの統合テスト
//!
//! 実際のフィクスチャファイルを使用したバリデーションの統合テスト。

use agent_link_engine::block_config::{BlockConfig, ToolsConfig};
use agent_link_engine::error::ParseError;
use agent_link_engine::validation::validate;

/// 最小限の有効な BlockConfig を生成するヘルパー。
fn valid_config() -> BlockConfig {
    BlockConfig {
        name: "test-block".to_string(),
        description: Some("テスト用ブロック".to_string()),
        model: Some("anthropic/claude-sonnet-4-5".to_string()),
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
    }
}

// ---- 有効なケース ----

#[test]
fn test_validate_valid_block() {
    let result = validate(&valid_config());
    assert!(result.is_valid());
    assert!(!result.has_warnings());
}

#[test]
fn test_validate_full_config() {
    let mut config = valid_config();
    config.thinking = Some("high".to_string());
    config.tools = Some(ToolsConfig::All);
    config.extensions = Some(ToolsConfig::List(vec!["herdrs".to_string()]));
    config.timeout = Some(300);
    config.worktree = Some("isolation".to_string());
    let result = validate(&config);
    assert!(result.is_valid());
}

// ---- 必須フィールド ----

#[test]
fn test_validate_missing_name() {
    let mut config = valid_config();
    config.name = String::new();
    let result = validate(&config);
    assert!(!result.is_valid());
}

#[test]
fn test_validate_empty_description() {
    // P10: description 空は警告（保存時に空文字補完）
    let mut config = valid_config();
    config.description = Some(String::new());
    let result = validate(&config);
    assert!(result.is_valid());
    assert!(result.has_warnings());
}

#[test]
fn test_validate_none_description() {
    // P10: description 欠落は警告（保存時に空文字補完）
    let mut config = valid_config();
    config.description = None;
    let result = validate(&config);
    assert!(result.is_valid());
    assert!(result.has_warnings());
}

#[test]
fn test_validate_missing_model() {
    // P10: model 欠落は警告（保存時に 'inherit' 補完）
    let mut config = valid_config();
    config.model = None;
    let result = validate(&config);
    assert!(result.is_valid());
    assert!(result.has_warnings());
}

#[test]
fn test_validate_empty_model() {
    // P10: model 空は警告（保存時に 'inherit' 補完）
    let mut config = valid_config();
    config.model = Some(String::new());
    let result = validate(&config);
    assert!(result.is_valid());
    assert!(result.has_warnings());
}

#[test]
fn test_validate_inherit_model() {
    let mut config = valid_config();
    config.model = Some("inherit".to_string());
    let result = validate(&config);
    assert!(result.is_valid());
}

// ---- thinking チェック ----

#[test]
fn test_validate_invalid_thinking() {
    let mut config = valid_config();
    config.thinking = Some("super".to_string());
    let result = validate(&config);
    assert!(!result.is_valid());
    assert!(result
        .errors
        .iter()
        .any(|e| matches!(e, ParseError::InvalidThinkingLevel(_))));
}

#[test]
fn test_validate_valid_thinking_values() {
    for level in &["none", "low", "medium", "high", "xhigh", "max"] {
        let mut config = valid_config();
        config.thinking = Some(level.to_string());
        let result = validate(&config);
        assert!(
            result.is_valid(),
            "thinking '{}' が有効として扱われるべき",
            level
        );
    }
}

/// 旧 thinking 値（off / minimal）は新 6 値のみの方針により不正扱いになること。
#[test]
fn test_validate_legacy_thinking_values_rejected() {
    for level in &["off", "minimal"] {
        let mut config = valid_config();
        config.thinking = Some(level.to_string());
        let result = validate(&config);
        assert!(
            !result.is_valid(),
            "旧 thinking '{}' は不正扱いになるべき",
            level
        );
    }
}

#[test]
fn test_validate_thinking_case_insensitive() {
    let mut config = valid_config();
    config.thinking = Some("HIGH".to_string());
    let result = validate(&config);
    assert!(result.is_valid());
}

// ---- timeout ----

#[test]
fn test_validate_zero_timeout() {
    let mut config = valid_config();
    config.timeout = Some(0);
    let result = validate(&config);
    assert!(!result.is_valid());
}

#[test]
fn test_validate_negative_timeout() {
    // timeout は u32 なので負の値は YAML パース時にエラーになる
    // ここでは 0 のケースのみテスト
    let mut config = valid_config();
    config.timeout = Some(0);
    let result = validate(&config);
    assert!(!result.is_valid());
}

// ---- enable: false の警告 ----

#[test]
fn test_validate_disabled_block_warning() {
    let mut config = valid_config();
    config.enable = Some(false);
    let result = validate(&config);
    assert!(result.is_valid());
    assert!(result.has_warnings());
    assert!(result.warnings.iter().any(|w| w.contains("無効化")));
}

#[test]
fn test_validate_enabled_block_no_warning() {
    let mut config = valid_config();
    config.enable = Some(true);
    let result = validate(&config);
    assert!(result.is_valid());
    assert!(!result.has_warnings());
}

#[test]
fn test_validate_none_enable_no_warning() {
    let config = valid_config();
    let result = validate(&config);
    assert!(result.is_valid());
    assert!(!result.has_warnings());
}

// ---- 複数エラーの収集 ----

#[test]
fn test_validate_multiple_errors() {
    let mut config = valid_config();
    config.name = String::new();
    config.thinking = Some("invalid".to_string());
    let result = validate(&config);
    // name 欠落 + thinking 不正の 2 エラーが必要（P10: description/model は警告に緩和）
    assert!(
        result.errors.len() >= 2,
        "複数エラーが収集されるべき（2つ以上）: {}",
        result.errors.len()
    );
}

#[test]
fn test_validate_empty_worktree() {
    let mut config = valid_config();
    config.worktree = Some("".to_string());
    let result = validate(&config);
    assert!(!result.is_valid());
}

#[test]
fn test_validate_valid_worktree_values() {
    for wt in &["none", "isolation", "feature/my-branch", "my-branch"] {
        let mut config = valid_config();
        config.worktree = Some(wt.to_string());
        let result = validate(&config);
        assert!(
            result.is_valid(),
            "worktree '{}' が有効として扱われるべき",
            wt
        );
    }
}
