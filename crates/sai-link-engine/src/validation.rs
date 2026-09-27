//! バリデーションロジック
//!
//! `BlockConfig` に対して必須フィールドの欠落や値制約違反を
//! 検出するバリデーション機能を提供します。
//!
//! 全てのバリデーションエラーを収集して返すため、ユーザーは
//! 1 回の実行で全ての問題を把握できます。

use crate::block_config::BlockConfig;
use crate::error::ParseError;

/// バリデーション結果を格納する構造体。
#[derive(Debug, Clone)]
pub struct ValidationResult {
    /// バリデーションエラーのリスト（回復不能な問題）
    pub errors: Vec<ParseError>,
    /// 警告メッセージのリスト（注意を促す情報）
    pub warnings: Vec<String>,
}

impl ValidationResult {
    /// エラーが 1 件もない場合に `true` を返す。
    pub fn is_valid(&self) -> bool {
        self.errors.is_empty()
    }

    /// 警告が 1 件以上ある場合に `true` を返す。
    pub fn has_warnings(&self) -> bool {
        !self.warnings.is_empty()
    }
}

/// 有効な thinking レベルの一覧（新 6 値・旧 `off` / `minimal` は不正）。
///
/// 実行時は runner に応じて解釈が変わる（pi → `--thinking`、cc → `--effort`）。
pub const VALID_THINKING_LEVELS: &[&str] = &["none", "low", "medium", "high", "xhigh", "max"];

/// 指定された `BlockConfig` を検証し、全てのエラーと警告を収集して返す。
///
/// 最初のエラーで打ち切らず、可能な限り全フィールドをチェックします。
/// これにより、ユーザーは 1 回の実行でブロック定義の全ての問題を把握できます。
///
/// # 引数
///
/// * `config` - 検証対象のブロック設定
///
/// # 戻り値
///
/// エラーと警告を含む `ValidationResult`。
pub fn validate(config: &BlockConfig) -> ValidationResult {
    let mut errors: Vec<ParseError> = Vec::new();
    let mut warnings: Vec<String> = Vec::new();

    // ---- 必須フィールドチェック ----

    // name: 空文字列でないこと（表示名・emoji 可）
    if config.name.trim().is_empty() {
        errors.push(ParseError::MissingField("name".to_string()));
    }

    // id: 指定されている場合は `[a-z0-9_-]+` パターンに従うこと
    if let Some(ref id) = config.id {
        let trimmed = id.trim();
        if trimmed.is_empty() {
            errors.push(ParseError::MissingField("id".to_string()));
        } else if !is_valid_block_id(trimmed) {
            errors.push(ParseError::InvalidValue {
                field: "id".to_string(),
                value: trimmed.to_string(),
                expected: "[a-z0-9_-]+ パターンに従う機械可読 ID（CLI・コマンドで使用）"
                    .to_string(),
            });
        }
    }

    // description: Some かつ空文字列でないこと（P10: 欠落は警告。保存時にデフォルト補完される）
    match &config.description {
        None => {
            warnings.push(format!(
                "ブロック '{}' に description がありません。空文字として保存されます。",
                config.name
            ));
        }
        Some(desc) if desc.trim().is_empty() => {
            warnings.push(format!(
                "ブロック '{}' の description が空です。",
                config.name
            ));
        }
        Some(_) => {}
    }

    // model: Some かつ空文字列でないこと（"inherit" は有効）
    // P10: 欠落は警告。保存時に "inherit" としてデフォルト補完される
    // （フロントマター = デフォルト値、実行時オーバーライドの哲学・A5）。
    match &config.model {
        None => {
            warnings.push(format!(
                "ブロック '{}' に model がありません。'inherit' として扱われます。",
                config.name
            ));
        }
        Some(m) if m.trim().is_empty() => {
            warnings.push(format!(
                "ブロック '{}' の model が空です。'inherit' として扱われます。",
                config.name
            ));
        }
        Some(_) => {}
    }

    // ---- 値制約チェック ----

    // thinking: 6 種類の有効値のいずれか
    if let Some(ref thinking) = config.thinking {
        let lower = thinking.trim().to_lowercase();
        if !VALID_THINKING_LEVELS.contains(&lower.as_str()) {
            errors.push(ParseError::InvalidThinkingLevel(thinking.clone()));
        }
    }

    // timeout: Some の場合、0 より大きいこと
    if let Some(timeout) = config.timeout {
        if timeout == 0 {
            errors.push(ParseError::InvalidValue {
                field: "timeout".to_string(),
                value: timeout.to_string(),
                expected: "1〜3600 の整数".to_string(),
            });
        }
    }

    // worktree: Some の場合、"none", "isolation", または空でない文字列
    if let Some(ref worktree) = config.worktree {
        let trimmed = worktree.trim();
        if trimmed.is_empty() {
            errors.push(ParseError::InvalidValue {
                field: "worktree".to_string(),
                value: worktree.clone(),
                expected: "\"none\", \"isolation\", または空でないブランチ名".to_string(),
            });
        }
    }

    // enable: Some(false) の場合は警告
    if let Some(false) = config.enable {
        warnings.push(format!(
            "ブロック '{}' は無効化されています（enable: false）",
            config.name
        ));
    }

    // ---- テンプレートチェック ----

    if let Some(ref templates) = config.templates {
        for t in templates {
            let trimmed = t.trim();
            // 空でないこと
            if trimmed.is_empty() {
                errors.push(ParseError::InvalidValue {
                    field: "templates".to_string(),
                    value: t.clone(),
                    expected: "空でないテンプレートファイル名".to_string(),
                });
                continue;
            }
            // パス区切りを含まないプレーンなファイル名であること
            // （`.sai/template/` 直下のファイルのみを参照するため）
            if trimmed.contains('/') || trimmed.contains('\\') || trimmed == "." || trimmed == ".."
            {
                errors.push(ParseError::InvalidValue {
                    field: "templates".to_string(),
                    value: trimmed.to_string(),
                    expected: "`PROJECTROOT/.sai/template/` 直下のファイル名（パスを含まない）"
                        .to_string(),
                });
            }
        }
    }

    ValidationResult { errors, warnings }
}

/// ブロック ID が `[a-z0-9_-]+` パターンにマッチするか検証する。
///
/// 機械可読 ID はファイル名（`<id>.md`）や CLI 引数として使われるため、
/// 小文字英数字・ハイフン・アンダースコアのみを許可する。
fn is_valid_block_id(id: &str) -> bool {
    !id.is_empty()
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block_config::{BlockConfig, ToolsConfig};

    /// 最小限の有効な BlockConfig を生成するヘルパー。
    fn valid_config() -> BlockConfig {
        BlockConfig {
            name: "test-block".to_string(),
            description: Some("テスト用ブロック".to_string()),
            model: Some("anthropic/claude-sonnet-4-5".to_string()),
            provider: None,
            thinking: None,
            tools: None,
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
    fn test_validate_valid_block() {
        let result = validate(&valid_config());
        assert!(result.is_valid());
        assert!(!result.has_warnings());
    }

    // ---- 必須フィールド ----

    #[test]
    fn test_validate_missing_name() {
        let mut config = valid_config();
        config.name = String::new();
        let result = validate(&config);
        assert!(!result.is_valid());
        assert!(result
            .errors
            .iter()
            .any(|e| matches!(e, ParseError::MissingField(f) if f == "name")));
    }

    #[test]
    fn test_validate_empty_name() {
        let mut config = valid_config();
        config.name = "   ".to_string();
        let result = validate(&config);
        assert!(!result.is_valid());
    }

    #[test]
    fn test_validate_missing_description() {
        // P10: description 欠落はエラーではなく警告（保存時に空文字補完）
        let mut config = valid_config();
        config.description = None;
        let result = validate(&config);
        assert!(result.is_valid());
        assert!(result.has_warnings());
        assert!(result.warnings.iter().any(|w| w.contains("description")));
    }

    #[test]
    fn test_validate_missing_model() {
        // P10: model 欠落はエラーではなく警告（保存時に 'inherit' 補完）
        let mut config = valid_config();
        config.model = None;
        let result = validate(&config);
        assert!(result.is_valid());
        assert!(result.has_warnings());
        assert!(result.warnings.iter().any(|w| w.contains("model")));
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
    fn test_validate_positive_timeout() {
        let mut config = valid_config();
        config.timeout = Some(300);
        let result = validate(&config);
        assert!(result.is_valid());
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

    // ---- 複数エラーの収集 ----

    #[test]
    fn test_validate_multiple_errors() {
        let mut config = valid_config();
        config.name = String::new();
        config.thinking = Some("invalid".to_string());
        let result = validate(&config);
        // name 欠落 + thinking 不正の 2 エラーが必要
        assert!(
            result.errors.len() >= 2,
            "複数エラーが収集されるべき: {}",
            result.errors.len()
        );
    }

    // ---- 空の worktree ----

    #[test]
    fn test_validate_empty_worktree() {
        let mut config = valid_config();
        config.worktree = Some("".to_string());
        let result = validate(&config);
        assert!(!result.is_valid());
    }

    #[test]
    fn test_validate_valid_worktree_values() {
        for wt in &["none", "isolation", "feature/my-branch"] {
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

    // ---- ToolsConfig ----

    #[test]
    fn test_validate_tools_config() {
        let mut config = valid_config();
        config.tools = Some(ToolsConfig::All);
        config.extensions = Some(ToolsConfig::List(vec!["herdrs".to_string()]));
        let result = validate(&config);
        assert!(result.is_valid());
    }

    // ---- templates ----

    #[test]
    fn test_validate_templates_valid() {
        let mut config = valid_config();
        config.templates = Some(vec!["tmp1".to_string(), "tmp2.md".to_string()]);
        let result = validate(&config);
        assert!(result.is_valid());
    }

    // ---- id（機械可読 ID） ----

    #[test]
    fn test_validate_id_absent_is_ok() {
        // id 未指定（後方互換）: name が ID として使われる
        let config = valid_config();
        assert_eq!(config.id, None);
        assert!(validate(&config).is_valid());
    }

    #[test]
    fn test_validate_id_valid() {
        for id in ["test_merge", "brst-search", "sai-plan-synth", "a1_b-2"] {
            let mut config = valid_config();
            config.id = Some(id.to_string());
            let result = validate(&config);
            assert!(result.is_valid(), "id '{}' が有効として扱われるべき", id);
        }
    }

    #[test]
    fn test_validate_id_invalid() {
        // 機械可読 ID に emoji・空白・大文字・記号は不可
        for bad in [
            "🧪 テスト",
            "Test Block",
            "test block",
            "test/id",
            "test.",
            "日本語",
        ] {
            let mut config = valid_config();
            config.id = Some(bad.to_string());
            let result = validate(&config);
            assert!(!result.is_valid(), "id '{}' はエラーになるべき", bad);
        }
    }

    #[test]
    fn test_validate_id_empty() {
        let mut config = valid_config();
        config.id = Some("   ".to_string());
        let result = validate(&config);
        assert!(!result.is_valid());
    }

    #[test]
    fn test_block_id_resolution() {
        // id 優先・未指定時は name が ID
        let mut config = valid_config();
        config.id = Some("test_merge".to_string());
        config.name = "🧪 テストマージ".to_string();
        assert_eq!(config.block_id(), "test_merge");
        assert_eq!(config.display_name(), "🧪 テストマージ");

        config.id = None;
        config.name = "old-name".to_string();
        assert_eq!(config.block_id(), "old-name");
        assert_eq!(config.display_name(), "old-name");
    }

    #[test]
    fn test_validate_templates_empty_name() {
        let mut config = valid_config();
        config.templates = Some(vec!["   ".to_string()]);
        let result = validate(&config);
        assert!(!result.is_valid());
    }

    #[test]
    fn test_validate_templates_path_traversal() {
        // パス区切り・`..` を含む名前はエラー（`.sai/template/` 直下のみ許可）
        for bad in ["../secret", "sub/name", "a\\b", ".."] {
            let mut config = valid_config();
            config.templates = Some(vec![bad.to_string()]);
            let result = validate(&config);
            assert!(
                !result.is_valid(),
                "テンプレート名 '{}' はエラーになるべき",
                bad
            );
        }
    }
}
