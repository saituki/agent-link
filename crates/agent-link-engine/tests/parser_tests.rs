//! パーサーの統合テスト
//!
//! 実際のフィクスチャファイルを使用したパースの統合テスト。

use agent_link_engine::block_config::ToolsConfig;
use agent_link_engine::error::ParseError;
use agent_link_engine::parse_block_file;

/// フィクスチャディレクトリのパスを取得する。
fn fixtures_dir() -> String {
    let mut path = std::env::current_dir().unwrap();
    // テストはプロジェクトルートまたは tests/ ディレクトリから実行される
    if path.ends_with("agent-link-engine")
    {
        path.push("tests/fixtures");
    } else if path.ends_with("tests") {
        path.push("fixtures");
    } else {
        // プロジェクトルートからの相対パスを試行
        path.push("crates/agent-link-engine/tests/fixtures");
    }
    path.to_string_lossy().to_string()
}

/// フィクスチャファイルのフルパスを生成する。
fn fixture_path(name: &str) -> String {
    format!("{}/{}", fixtures_dir(), name)
}

// ---- 有効なブロック定義のパース ----

#[test]
fn test_parse_valid_minimal() {
    let parsed = parse_block_file(fixture_path("valid-minimal.md")).unwrap();
    assert_eq!(parsed.config.name, "test-block");
    assert_eq!(
        parsed.config.description.as_deref(),
        Some("テスト用の最小ブロック定義")
    );
    assert_eq!(
        parsed.config.model.as_deref(),
        Some("anthropic/claude-sonnet-4-5")
    );
    assert!(parsed
        .body
        .contains("これはテスト用のブロックプロンプト本文です"));
}

#[test]
fn test_parse_valid_full() {
    let parsed = parse_block_file(fixture_path("valid-full.md")).unwrap();
    assert_eq!(parsed.config.name, "sai-brst-grill");
    assert_eq!(parsed.config.thinking.as_deref(), Some("high"));
    assert_eq!(
        parsed.config.tools,
        Some(ToolsConfig::List(vec![
            "read".to_string(),
            "bash".to_string(),
            "edit".to_string(),
            "write".to_string(),
        ]))
    );
    assert_eq!(
        parsed.config.extensions,
        Some(ToolsConfig::List(vec![
            "herdrs".to_string(),
            "pi-sai".to_string(),
        ]))
    );
    assert_eq!(parsed.config.skills.as_deref(), Some("ce-brainstorm"));
    assert_eq!(parsed.config.timeout, Some(600));
    assert_eq!(parsed.config.worktree.as_deref(), Some("isolation"));
    assert_eq!(parsed.config.enable, Some(true));

    // body の検証
    assert!(parsed.body.contains("{{input.project_name}}"));
}

/// 廃止済みの `max_turns` / `makaseru` を含む古い書き方の block.md は、
/// エラーにせず読む（知らない項目は無視される）。`timeout` は今も効くので読める。
#[test]
fn test_parse_legacy_max_turns_and_makaseru_are_ignored() {
    let parsed = parse_block_file(fixture_path("legacy-max-turns-makaseru.md"))
        .expect("古い書き方の block.md でもパースできる");
    assert_eq!(parsed.config.name, "legacy-max-turns");
    assert_eq!(parsed.config.timeout, Some(3600));
}

#[test]
fn test_parse_inherit_model() {
    let parsed = parse_block_file(fixture_path("valid-inherit-model.md")).unwrap();
    assert_eq!(parsed.config.name, "inherit-model-block");
    assert_eq!(parsed.config.model.as_deref(), Some("inherit"));
}

#[test]
fn test_parse_no_inputs() {
    let parsed = parse_block_file(fixture_path("valid-no-inputs.md")).unwrap();
    assert_eq!(parsed.config.name, "no-inputs-block");
}

/// 旧 `inputs:` / `output:` が残った block.md もエラーにならず読み込める。
///
/// P2 で `BlockConfig` から両フィールドを削除したため、未知フィールドとして無視される
/// （`deny_unknown_fields` は付けていない）。
#[test]
fn test_parse_legacy_inputs_output_ignored() {
    let parsed = parse_block_file(fixture_path("legacy-with-inputs.md")).unwrap();
    assert_eq!(parsed.config.name, "legacy-inputs-block");
    assert!(parsed.body.contains("# 旧定義ブロック"));
}

// ---- 無効なブロック定義のパース ----

#[test]
fn test_parse_invalid_missing_name() {
    let result = parse_block_file(fixture_path("invalid-missing-name.md"));
    assert!(result.is_err());
    match result {
        Err(ParseError::MissingField(field)) => assert_eq!(field, "name"),
        _ => panic!(
            "MissingField(name) エラーが期待されますが、{:?} が返されました",
            result
        ),
    }
}

#[test]
fn test_parse_invalid_missing_model() {
    // P10: model 欠落はエラーではなく警告（'inherit' として扱われる・A5 の哲学）
    let result = parse_block_file(fixture_path("invalid-missing-model.md"));
    assert!(result.is_ok());
    if let Ok(parsed) = result {
        assert_eq!(parsed.config.name, "missing-model-block");
    }
}

#[test]
fn test_parse_invalid_thinking() {
    let result = parse_block_file(fixture_path("invalid-thinking.md"));
    assert!(result.is_err());
    assert!(matches!(result, Err(ParseError::InvalidThinkingLevel(_))));
}

#[test]
fn test_parse_no_frontmatter() {
    let result = parse_block_file(fixture_path("no-frontmatter.md"));
    assert!(result.is_err());
    assert!(matches!(result, Err(ParseError::MissingFrontmatter)));
}

#[test]
fn test_parse_file_not_found() {
    let result = parse_block_file("/tmp/nonexistent-block-definition.md");
    assert!(result.is_err());
    assert!(matches!(result, Err(ParseError::IoError(_))));
}

// ---- ToolsConfig のパース ----

#[test]
fn test_parse_tools_config_bool() {
    // valid-full.md の tools は文字列リスト
    let parsed = parse_block_file(fixture_path("valid-full.md")).unwrap();
    assert_eq!(
        parsed.config.tools,
        Some(ToolsConfig::List(vec![
            "read".to_string(),
            "bash".to_string(),
            "edit".to_string(),
            "write".to_string(),
        ]))
    );
}

// ---- 本文抽出 ----

#[test]
fn test_body_extraction() {
    let parsed = parse_block_file(fixture_path("valid-minimal.md")).unwrap();
    assert_eq!(
        parsed.body.trim(),
        "# テストブロック\n\nこれはテスト用のブロックプロンプト本文です。"
    );
}

#[test]
fn test_body_preserves_markdown() {
    let parsed = parse_block_file(fixture_path("valid-full.md")).unwrap();
    assert!(parsed.body.contains("{{input.project_name}}"));
    assert!(parsed.body.contains("{{input.architecture_style}}"));
    assert!(parsed.body.contains("## プロジェクト名"));
}

// ---- templates 機能の統合テスト ----

/// `parse_block_file`（= `PROJECTROOT/.agent-link/template` の解決込み）の
/// templates 展開を検証する。
///
/// `AGENT_LINK_PROJECT_ROOT` 環境変数を一時ディレクトリに差し替え、
/// その下に `.agent-link/template/*.md` とブロック定義を配置して
/// 実パイプラインと同じ経路（`parse_block_file` → 本文展開）を確認する。
///
/// ※ 環境変数はプロセスローカル。このテストバイナリ内の他テストは
///   fixtures の絶対パスと templates 未使用の定義しか参照しないため、
///   並行実行でも影響はない。
#[test]
fn test_parse_block_file_expands_templates_via_project_root() {
    use std::sync::Mutex;
    use std::sync::OnceLock;

    // 環境変数を共有ロックで直列化する（同一バイナリ内の並行実行対策）
    static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    let _guard = ENV_LOCK.get_or_init(|| Mutex::new(())).lock().unwrap();

    // 一時プロジェクトルートを構築
    let temp_root = tempfile::tempdir().unwrap();
    let tpl_dir = temp_root.path().join(".agent-link/template");
    std::fs::create_dir_all(&tpl_dir).unwrap();
    std::fs::write(tpl_dir.join("tmp1.md"), "# テンプレート1の内容").unwrap();
    std::fs::write(tpl_dir.join("tmp2.md"), "# テンプレート2の内容").unwrap();

    let block_path = temp_root.path().join("test_block.md");
    std::fs::write(
        &block_path,
        "---\nname: test-template\ndescription: desc\nmodel: m\ntemplates: tmp1,tmp2\n---\n# content",
    )
    .unwrap();

    // AGENT_LINK_PROJECT_ROOT を差し替え（既存値を退避して復元）
    let prev = std::env::var_os("AGENT_LINK_PROJECT_ROOT");
    std::env::set_var("AGENT_LINK_PROJECT_ROOT", temp_root.path());

    let result = parse_block_file(&block_path);

    // 環境変数を復元
    match prev {
        Some(v) => std::env::set_var("AGENT_LINK_PROJECT_ROOT", v),
        None => std::env::remove_var("AGENT_LINK_PROJECT_ROOT"),
    }

    let parsed = result.unwrap();
    assert_eq!(
        parsed.config.templates,
        Some(vec!["tmp1".to_string(), "tmp2".to_string()])
    );
    // 実プロジェクトルート解決（.agent-link/template）経由で本文の先頭に展開される
    assert_eq!(
        parsed.body,
        "# テンプレート1の内容\n\n# テンプレート2の内容\n\n# content"
    );
}

/// テンプレートファイルが存在しない場合、`ParseError::TemplateError` になる。
#[test]
fn test_parse_block_file_template_missing_error() {
    use std::sync::Mutex;
    use std::sync::OnceLock;

    static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    let _guard = ENV_LOCK.get_or_init(|| Mutex::new(())).lock().unwrap();

    let temp_root = tempfile::tempdir().unwrap();
    let block_path = temp_root.path().join("test_block.md");
    std::fs::write(
        &block_path,
        "---\nname: test-template\ndescription: desc\nmodel: m\ntemplates: not_exist\n---\n# content",
    )
    .unwrap();

    let prev = std::env::var_os("AGENT_LINK_PROJECT_ROOT");
    std::env::set_var("AGENT_LINK_PROJECT_ROOT", temp_root.path());

    let result = parse_block_file(&block_path);

    match prev {
        Some(v) => std::env::set_var("AGENT_LINK_PROJECT_ROOT", v),
        None => std::env::remove_var("AGENT_LINK_PROJECT_ROOT"),
    }

    match result {
        Err(ParseError::TemplateError { name, .. }) => assert_eq!(name, "not_exist.md"),
        other => panic!("TemplateError が期待されますが、{:?} が返されました", other),
    }
}
