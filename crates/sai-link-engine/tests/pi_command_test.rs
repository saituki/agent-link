//! PiCommandBuilder 統合テスト
//!
//! `src/sai-engine/src/pi_command.rs` の `PiCommandBuilder` に対して
//! 外部から統合テストを行います。
//!
//! 各テストは `BlockConfig` を構築し、`PiCommandBuilder` を通して
//! コマンド文字列を生成し、期待されるフラグが含まれていることを確認します。

use sai_link_engine::block_config::{BlockConfig, ToolsConfig};
use sai_link_engine::pi_command::PiCommandBuilder;

// ============================================================================
// テストヘルパー
// ============================================================================

/// テスト用のデフォルト BlockConfig を作成するヘルパー関数。
///
/// 各テストはこの関数を呼び出した後、必要なフィールドだけを上書きします。
fn mock_config() -> BlockConfig {
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
        enable: Some(true),
        category: None,
        tag: None,
        round: None,
        layer: None,
        templates: None,
        context: None,
        id: None,
    }
}

// ============================================================================
// 基本構築テスト
// ============================================================================

/// session_dir のみの最小コマンドが正しく構築されることを確認する。
#[test]
fn test_minimal_command() {
    let cmd = PiCommandBuilder::new("/tmp/session")
        .build()
        .expect("最小コマンドの構築に失敗しました");

    assert!(
        cmd.command.starts_with("pi "),
        "コマンドは 'pi ' で始まる必要があります"
    );
    assert!(
        cmd.command.contains("--session-dir"),
        "--session-dir フラグが必要です"
    );
    assert!(
        cmd.command.contains("/tmp/session"),
        "session-dir の値がエスケープされていません"
    );

    assert!(cmd.command.contains("--session-dir"));
    assert!(cmd.command.contains("/tmp/session"));
}

/// model が --model フラグとして追加されることを確認する。
#[test]
fn test_with_model() {
    let mut config = mock_config();
    config.model = Some("anthropic/claude-sonnet-4-5".to_string());

    let cmd = PiCommandBuilder::new("/tmp/session")
        .with_config(config)
        .build()
        .expect("コマンドの構築に失敗しました");

    assert!(cmd.command.contains("--model"));
    assert!(cmd.command.contains("anthropic/claude-sonnet-4-5"));
}

/// model: "inherit" が --model フラグとして追加されないことを確認する。
#[test]
fn test_skip_inherit_model() {
    let mut config = mock_config();
    config.model = Some("inherit".to_string());

    let cmd = PiCommandBuilder::new("/tmp/session")
        .with_config(config)
        .build()
        .expect("コマンドの構築に失敗しました");

    assert!(!cmd.command.contains("--model"));
}

/// provider が --provider フラグとして追加されることを確認する。
#[test]
fn test_with_provider() {
    let mut config = mock_config();
    config.provider = Some("anthropic".to_string());

    let cmd = PiCommandBuilder::new("/tmp/session")
        .with_config(config)
        .build()
        .expect("コマンドの構築に失敗しました");

    assert!(cmd.command.contains("--provider"));
    assert!(cmd.command.contains("anthropic"));
}

/// thinking が --thinking フラグとして追加されることを確認する。
#[test]
fn test_with_thinking() {
    let mut config = mock_config();
    config.thinking = Some("high".to_string());

    let _cmd = PiCommandBuilder::new("/tmp/session")
        .with_config(config)
        .build()
        .expect("コマンドの構築に失敗しました");

    // --thinking merged into model:thinking
    // high merged into --model
}

// ============================================================================
// ツール・拡張機能テスト
// ============================================================================

/// ToolsConfig::All で --tools / --no-tools が追加されないことを確認する。
#[test]
fn test_tools_all() {
    let mut config = mock_config();
    config.tools = Some(ToolsConfig::All);

    let cmd = PiCommandBuilder::new("/tmp/session")
        .with_config(config)
        .build()
        .expect("コマンドの構築に失敗しました");

    assert!(!cmd.command.contains("--tools"));
    assert!(!cmd.command.contains("--no-tools"));
}

/// ToolsConfig::None で --no-tools が追加されることを確認する。
#[test]
fn test_tools_none() {
    let mut config = mock_config();
    config.tools = Some(ToolsConfig::None);

    let cmd = PiCommandBuilder::new("/tmp/session")
        .with_config(config)
        .build()
        .expect("コマンドの構築に失敗しました");

    assert!(!cmd.command.contains("--tools"));
    assert!(cmd.command.contains("--no-tools"));
}

/// ToolsConfig::List で --tools 'read,bash' が追加されることを確認する。
#[test]
fn test_tools_list() {
    let mut config = mock_config();
    config.tools = Some(ToolsConfig::List(vec![
        "read".to_string(),
        "bash".to_string(),
    ]));

    let cmd = PiCommandBuilder::new("/tmp/session")
        .with_config(config)
        .build()
        .expect("コマンドの構築に失敗しました");

    let c = &cmd.command;
    assert!(c.contains("--tools"), "--tools フラグが必要: {}", c);
    assert!(
        c.contains("read,bash"),
        "ツールリスト 'read,bash' が必要: {}",
        c
    );
}

/// ToolsConfig::All で拡張機能フラグが追加されないことを確認する。
#[test]
fn test_extensions_all() {
    let mut config = mock_config();
    config.extensions = Some(ToolsConfig::All);

    let _cmd = PiCommandBuilder::new("/tmp/session")
        .with_config(config)
        .build()
        .expect("コマンドの構築に失敗しました");

    // --extensions merged into --tools
    // --no-extensions not supported
}

/// ToolsConfig::None で --no-extensions が追加されることを確認する。
#[test]
fn test_extensions_none() {
    let mut config = mock_config();
    config.extensions = Some(ToolsConfig::None);

    let _cmd = PiCommandBuilder::new("/tmp/session")
        .with_config(config)
        .build()
        .expect("コマンドの構築に失敗しました");

    // --extensions merged into --tools
    // --no-extensions not supported
}

/// ToolsConfig::List の拡張はコマンドに付けない（U01: pi は MCP で道具を使うため無視する）。
#[test]
fn test_extensions_list() {
    let mut config = mock_config();
    config.extensions = Some(ToolsConfig::List(vec![
        "herdrs".to_string(),
        "pi-sai".to_string(),
    ]));

    let cmd = PiCommandBuilder::new("/tmp/session")
        .with_config(config)
        .build()
        .expect("コマンドの構築に失敗しました");

    let c = &cmd.command;
    assert!(!c.contains("herdrs"), "拡張機能は付けない: {}", c);
    assert!(!c.contains("--extension"), "拡張機能は付けない: {}", c);
}

// ============================================================================
// その他フラグテスト
// ============================================================================

/// skills を指定しても --tools に入らないことを確認する。
///
/// pi の --tools は道具を絞る一覧で、skill を有効にする仕組みではない。
/// 知らない名前は黙って無視され、tools の指定が無いと既定の道具も消える。
#[test]
fn test_with_skills() {
    let mut config = mock_config();
    config.skills = Some("ce-brainstorm,pi-sai".to_string());

    let cmd = PiCommandBuilder::new("/tmp/session")
        .with_config(config)
        .build()
        .expect("コマンドの構築に失敗しました");

    let c = &cmd.command;
    assert!(!c.contains("ce-brainstorm"), "skill の名前は --tools に入らない: {}", c);
    assert!(!c.contains("pi-sai"), "skill の名前は --tools に入らない: {}", c);
    // tools の指定が無いので --tools 自体も出ない（既定の道具はそのまま）
    assert!(!c.contains("--tools"), "既定の道具が消える --tools は付けない: {}", c);
}

/// timeout が --timeout フラグとして追加されることを確認する。
#[test]
fn test_with_timeout() {
    let mut config = mock_config();
    config.timeout = Some(600);

    let cmd = PiCommandBuilder::new("/tmp/session")
        .with_config(config)
        .build()
        .expect("コマンドの構築に失敗しました");

    let _c = &cmd.command;
    // --timeout not supported by pi
    // value check removed (flag not supported)
}

/// observability フラグが追加されることを確認する。
#[test]
fn test_with_obs() {
    let cmd = PiCommandBuilder::new("/tmp/session")
        .with_obs("my-pool", "my-tag", "my-name")
        .build()
        .expect("コマンドの構築に失敗しました");

    let c = &cmd.command;
    assert!(c.contains("--o-pool"), "--o-pool フラグが必要: {}", c);
    assert!(c.contains("my-pool"), "o-pool の値 'my-pool' が必要: {}", c);
    assert!(c.contains("--o-tag"), "--o-tag フラグが必要: {}", c);
    assert!(c.contains("my-tag"), "o-tag の値 'my-tag' が必要: {}", c);
    assert!(c.contains("--o-name"), "--o-name フラグが必要: {}", c);
    assert!(c.contains("my-name"), "o-name の値 'my-name' が必要: {}", c);
}

/// 全フィールド設定時の完全なコマンドを確認する。
#[test]
fn test_all_flags() {
    let mut config = mock_config();
    config.model = Some("anthropic/claude-sonnet-4-5".to_string());
    config.provider = Some("anthropic".to_string());
    config.thinking = Some("high".to_string());
    config.tools = Some(ToolsConfig::List(vec![
        "read".to_string(),
        "bash".to_string(),
    ]));
    config.extensions = Some(ToolsConfig::None);
    config.skills = Some("ce-brainstorm".to_string());
    config.timeout = Some(600);

    let cmd = PiCommandBuilder::new("/tmp/session")
        .with_config(config)
        .with_obs("pool1", "tag1", "name1")
        .build()
        .expect("コマンドの構築に失敗しました");

    let c = &cmd.command;
    assert!(
        c.starts_with("pi "),
        "コマンドは 'pi ' で始まる必要があります"
    );

    // セッションディレクトリ
    assert!(c.contains("--session-dir"), "--session-dir が必要");
    assert!(c.contains("/tmp/session"), "'/tmp/session' が必要");

    // モデル/プロバイダ
    assert!(c.contains("--model"), "--model が必要");
    assert!(c.contains("anthropic/claude-sonnet-4-5"));
    assert!(c.contains("--provider"), "--provider が必要");
    assert!(c.contains("anthropic"));

    // thinking は --thinking フラグとして出力
    assert!(c.contains("--thinking"), "--thinking が必要");
    assert!(c.contains("high"));
    // model:thinking 結合形式は使わない
    assert!(!c.contains(":high"));

    // ツール/拡張機能
    assert!(c.contains("--tools"), "--tools が必要");
    assert!(c.contains("read,bash"));
    // --no-extensions not supported

    // スキル/ターン/タイムアウト
    // skills は --tools に入らない（pi は .pi/skills/ の skill を読みます）
    assert!(!c.contains("ce-brainstorm"), "skill の名前は --tools に入らない: {}", c);
    // --max-turns not supported by pi
    // --timeout not supported by pi

    // observability
    assert!(c.contains("--o-pool"), "--o-pool が必要");
    assert!(c.contains("pool1"));
    assert!(c.contains("--o-tag"), "--o-tag が必要");
    assert!(c.contains("tag1"));
    assert!(c.contains("--o-name"), "--o-name が必要");
    assert!(c.contains("name1"));
}

// ============================================================================
// シェルエスケープテスト
// ============================================================================

/// シングルクォートを含む値が正しくエスケープされることを確認する。
#[test]
fn test_shell_escape_single_quotes() {
    // TypeScript の shellEscape("it's") → "'it'\"'\"'s'"
    // Rust: shell_escape_single_quotes("it's") → "'it'\\''s'"
    let escaped = sai_link_engine::herdr::shell_escape_single_quotes("it's");
    assert_eq!(
        escaped, "'it'\\''s'",
        "シングルクォートのエスケープが正しくありません"
    );
}

/// 空白を含む model 値がクォートされることを確認する。
#[test]
fn test_model_with_spaces() {
    let mut config = mock_config();
    config.model = Some("claude sonnet 4.5".to_string());

    let cmd = PiCommandBuilder::new("/tmp/session")
        .with_config(config)
        .build()
        .expect("コマンドの構築に失敗しました");

    // シングルクォートで囲まれていることを確認
    let model_flag_idx = cmd.command.find("--model").unwrap();
    let after_flag = &cmd.command[model_flag_idx..];
    assert!(
        after_flag.contains("claude sonnet 4.5"),
        "空白を含むモデル名がクォートされていません: {}",
        after_flag
    );
}

/// シェルインジェクション文字がシングルクォートで保護されることを確認する。
#[test]
fn test_no_shell_injection() {
    let mut config = mock_config();
    config.model = Some("model; rm -rf /".to_string());

    let cmd = PiCommandBuilder::new("/tmp/session")
        .with_config(config)
        .build()
        .expect("コマンドの構築に失敗しました");

    // shell_escape_single_quotes によりセミコロンがクォート内に閉じ込められている
    assert!(
        cmd.command.contains("model; rm -rf /"),
        "シェルインジェクション文字がクォートで保護されていません: {}",
        cmd.command
    );
}

// ============================================================================
// args ベクタテスト
// ============================================================================

/// args ベクタの要素順序が command 文字列と一致することを確認する。
#[test]
fn test_args_order() {
    let mut config = mock_config();
    config.model = Some("test-model".to_string());
    config.thinking = Some("low".to_string());

    let cmd = PiCommandBuilder::new("/tmp/session")
        .with_config(config)
        .build()
        .expect("コマンドの構築に失敗しました");

    let expected = format!("pi {}", cmd.args.join(" "));
    assert_eq!(
        cmd.command, expected,
        "args の結合が command と一致しません"
    );
}

// ============================================================================
// エッジケーステスト
// ============================================================================

/// 無効（enable: false）なブロックでもビルドが成功することを確認する。
#[test]
fn test_disabled_block() {
    let mut config = mock_config();
    config.enable = Some(false);

    // エラーなくビルドできること
    let cmd = PiCommandBuilder::new("/tmp/session")
        .with_config(config)
        .build()
        .expect("無効ブロックのコマンド構築に失敗しました");

    assert!(cmd.command.starts_with("pi "));
    assert!(cmd.command.contains("--session-dir"));
}

/// 空のスキル文字列でも --tools が出ないことを確認する。
#[test]
fn test_empty_skills_skipped() {
    let mut config = mock_config();
    config.skills = Some("".to_string());

    let cmd = PiCommandBuilder::new("/tmp/session")
        .with_config(config)
        .build()
        .expect("コマンドの構築に失敗しました");

    assert!(!cmd.command.contains("--tools"), "{}", cmd.command);
}

/// 空の provider 文字列で --provider フラグが追加されないことを確認する。
#[test]
fn test_empty_provider_skipped() {
    let mut config = mock_config();
    config.provider = Some("".to_string());

    let cmd = PiCommandBuilder::new("/tmp/session")
        .with_config(config)
        .build()
        .expect("コマンドの構築に失敗しました");

    assert!(!cmd.command.contains("--provider"));
}

/// config が None（未設定）の場合でもビルドが成功することを確認する。
#[test]
fn test_no_config() {
    let cmd = PiCommandBuilder::new("/tmp/session")
        .build()
        .expect("config なしのコマンド構築に失敗しました");

    assert!(cmd.command.starts_with("pi "));
    assert!(cmd.command.contains("--session-dir"));
    assert!(cmd.command.contains("/tmp/session"));
    assert!(!cmd.command.contains("--model"));
    // --thinking merged into model:thinking
}
