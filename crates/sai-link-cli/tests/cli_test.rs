//! CLI 統合テスト
//!
//! clap の引数パース、ヘルプ表示、エラーハンドリングを
//! `std::process::Command` 経由で検証します。
//!
//! 内部ヘルパー関数（format_status, format_elapsed 等）のテストは
//! `cli.rs` 内の `#[cfg(test)] mod tests` で実施済みです（24件パス確認済み）。

use std::path::PathBuf;
use std::process::Command;

/// `sai` のバイナリパスを取得する（Cargo が統合テスト用に提供する env を使用）
fn sai_binary() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_sai-moon-link"))
}

/// テストフィクスチャファイルの絶対パスを取得する
fn fixture_path(name: &str) -> PathBuf {
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push("tests");
    path.push("fixtures");
    path.push(name);
    path
}

use std::sync::atomic::{AtomicU32, Ordering};

/// テンポラリテストディレクトリを作成する（既存の DB の影響を受けない）
fn create_temp_dir() -> PathBuf {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let count = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir =
        std::env::temp_dir().join(format!("sai-engine-test-{}-{}", std::process::id(), count));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("テンポラリディレクトリ作成失敗");
    dir
}

/// テンポラリテストディレクトリをクリーンアップする
fn cleanup_temp_dir(dir: &PathBuf) {
    let _ = std::fs::remove_dir_all(dir);
}

// ============================================================================
// CLI 引数パーステスト
// ============================================================================

/// `--help` が Sai-Engine の説明を含むこと
#[test]
fn test_cli_help() {
    let output = Command::new(sai_binary())
        .arg("--help")
        .output()
        .expect("sai の実行に失敗しました");
    assert!(output.status.success(), "exit code 0 expected");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Sai-Engine"),
        "ヘルプに Sai-Engine が含まれること"
    );
    assert!(
        stdout.contains("block"),
        "ヘルプに block サブコマンドが含まれること"
    );
}

/// `block --help` が 3 つのサブコマンドを表示すること
#[test]
fn test_cli_block_help() {
    let output = Command::new(sai_binary())
        .args(["block", "--help"])
        .output()
        .expect("sai block --help の実行に失敗しました");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("run"),
        "block サブコマンドに run が含まれること"
    );
    assert!(
        stdout.contains("list"),
        "block サブコマンドに list が含まれること"
    );
    assert!(
        stdout.contains("show"),
        "block サブコマンドに show が含まれること"
    );
}

/// `block run --help` が正しい引数情報を表示すること
#[test]
fn test_cli_block_run_help() {
    let output = Command::new(sai_binary())
        .args(["block", "run", "--help"])
        .output()
        .expect("sai block run --help の実行に失敗しました");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("定義ファイル"),
        "run ヘルプに 定義ファイル 引数が含まれること"
    );
    assert!(
        stdout.contains("ユーザー入力"),
        "run ヘルプに ユーザー入力 引数が含まれること"
    );
    assert!(
        stdout.contains("--direction"),
        "run ヘルプに --direction オプションが含まれること"
    );
    assert!(
        stdout.contains("--timeout"),
        "run ヘルプに --timeout オプションが含まれること"
    );
    assert!(
        stdout.contains("--keep-pane"),
        "run ヘルプに --keep-pane オプションが含まれること"
    );
}

/// 存在しないサブコマンドでエラー終了すること
#[test]
fn test_cli_invalid_subcommand() {
    let output = Command::new(sai_binary())
        .arg("invalid-subcommand")
        .output()
        .expect("sai の実行に失敗しました");
    assert!(
        !output.status.success(),
        "不正なサブコマンドはエラー終了すること"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("error"), "エラーメッセージが表示されること");
}

/// `help block` サブコマンドでブロックサブコマンド一覧を表示すること
#[test]
fn test_cli_block_help_subcommand() {
    let output = Command::new(sai_binary())
        .args(["help", "block"])
        .output()
        .expect("sai help block の実行に失敗しました");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("run"));
    assert!(stdout.contains("list"));
    assert!(stdout.contains("show"));
}

/// 存在しないファイルパスを指定した場合にエラーメッセージが表示されること
///
/// kanban 統一後は定義ファイル直指定が廃止され、誘導メッセージで終了する。
#[test]
fn test_cli_block_run_nonexistent_file() {
    let output = Command::new(sai_binary())
        .args(["block", "run", "/nonexistent/path/to/block.md"])
        .output()
        .expect("sai の実行に失敗しました");
    assert!(
        !output.status.success(),
        "存在しないファイルはエラー終了すること"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("廃止") || stderr.contains("見つかりません"),
        "エラーメッセージが日本語で表示されること"
    );
}

/// 無効なブロック定義ファイルを指定した場合にパースエラーが表示されること
///
/// kanban 統一後は定義ファイル直指定が廃止され、誘導メッセージで終了する。
#[test]
fn test_cli_block_run_invalid_block() {
    let output = Command::new(sai_binary())
        .args([
            "block",
            "run",
            fixture_path("invalid-missing-name.md").to_str().unwrap(),
        ])
        .output()
        .expect("sai の実行に失敗しました");
    assert!(
        !output.status.success(),
        "無効なブロック定義はエラー終了すること"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("廃止")
            || stderr.contains("パース")
            || stderr.contains("エラー")
            || stderr.contains("name"),
        "エラーメッセージが表示されること: {}",
        stderr
    );
}

/// `--version` がバージョン情報を表示すること
#[test]
fn test_cli_version() {
    let output = Command::new(sai_binary())
        .arg("--version")
        .output()
        .expect("sai --version の実行に失敗しました");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("0.1.0") || stdout.contains("sai-link"));
}

/// `block list --limit 5` が limit オプションを受け付けること
#[test]
fn test_cli_block_list_with_limit() {
    let dir = create_temp_dir();

    let output = Command::new(sai_binary())
        .args(["block", "list", "--limit", "5"])
        .env("SAI_PROJECT_ROOT", &dir)
        .current_dir(&dir)
        .output()
        .expect("sai block list --limit 5 の実行に失敗しました");

    // 空の DB でも limit オプション付きで正常動作すること
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("実行履歴はありません"));

    cleanup_temp_dir(&dir);
}

// ============================================================================
// block list / block show のテスト（空の DB で動作確認）
// ============================================================================

/// `block list` が空の DB で「履歴なし」メッセージを表示すること
#[test]
fn test_cli_block_list_empty() {
    let dir = create_temp_dir();

    let output = Command::new(sai_binary())
        .args(["block", "list"])
        .env("SAI_PROJECT_ROOT", &dir)
        .current_dir(&dir)
        .output()
        .expect("sai block list の実行に失敗しました");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("実行履歴はありません"),
        "空の DB では「履歴なし」メッセージが表示されること"
    );

    cleanup_temp_dir(&dir);
}

/// `block show` が存在しない ID でエラーを表示すること
#[test]
fn test_cli_block_show_not_found() {
    let dir = create_temp_dir();

    let output = Command::new(sai_binary())
        .args(["block", "show", "99999"])
        .current_dir(&dir)
        .output()
        .expect("sai block show の実行に失敗しました");

    assert!(
        !output.status.success(),
        "存在しない ID はエラー終了すること"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("見つかりません"),
        "エラーメッセージが表示されること"
    );

    cleanup_temp_dir(&dir);
}

/// `block list` がデフォルトで実行履歴なしメッセージを表示すること
#[test]
fn test_cli_block_list_default() {
    let dir = create_temp_dir();

    let output = Command::new(sai_binary())
        .args(["block", "list"])
        .env("SAI_PROJECT_ROOT", &dir)
        .current_dir(&dir)
        .output()
        .expect("sai block list の実行に失敗しました");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("実行履歴はありません"));

    cleanup_temp_dir(&dir);
}

// ============================================================================
// Reflection 記録ファイル削除（R13）
// ============================================================================

use sai_link_engine::db::models::BlockMetadata;
use sai_link_engine::db::SaiStore;

/// 一時ディレクトリに DB と実行レコードを作成し、実行 ID を返す。
fn setup_execution(dir: &std::path::Path, block_name: &str) -> i64 {
    let db_path = dir.join(".sai/sai-engine/sai-engine.db");
    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent).expect("DB ディレクトリ作成に失敗");
    }
    let store = SaiStore::open(&db_path).expect("テスト用 DB を開けること");

    let block = BlockMetadata::new(
        block_name.to_string(),
        "テスト用ブロック".to_string(),
        "anthropic/claude-sonnet-4-5".to_string(),
        None,
        "off".to_string(),
        "true".to_string(),
        "true".to_string(),
        None,
        300,
        "none".to_string(),
        true,
        "/tmp/test-block.md".to_string(),
        None,
    );
    let block_id = store
        .insert_block(&block)
        .expect("ブロックを挿入できること");
    store
        .create_execution(block_id, None)
        .expect("実行レコードを作成できること")
}

/// R603: `block rm` で実行を削除すると、対応する run フォルダも丸ごと削除されること。
#[test]
fn test_cli_block_rm_deletes_reflection_file() {
    let dir = create_temp_dir();

    let exec_id = setup_execution(&dir, "reflection-rm-test");
    // run フォルダ＋旧 reflection を用意しておく（完了後の状態を想定）
    let run_dir = dir
        .join(".sai/outputs/review")
        .join(exec_id.to_string());
    std::fs::create_dir_all(&run_dir).expect("ディレクトリ作成に失敗");
    std::fs::write(run_dir.join("review.md"), "### 追加修正\n").expect("review を用意できること");

    // 実行を削除（--yes で確認スキップ）
    let output = Command::new(sai_binary())
        .args(["block", "rm", "--yes", &exec_id.to_string()])
        // 外部環境の SAI_PROJECT_ROOT を無視し、プロジェクトルートを一時ディレクトリに固定
        .env("SAI_PROJECT_ROOT", &dir)
        .current_dir(&dir)
        .output()
        .expect("sai block rm の実行に失敗しました");
    assert!(output.status.success(), "block rm が成功すること");

    assert!(
        !run_dir.exists(),
        "対応する run フォルダが丸ごと削除されること"
    );

    cleanup_temp_dir(&dir);
}

/// R13/AE1: 記録ファイルが存在しない状態でも `block rm` が失敗しないこと。
#[test]
fn test_cli_block_rm_missing_reflection_no_error() {
    let dir = create_temp_dir();

    let exec_id = setup_execution(&dir, "reflection-rm-missing");

    // Reflection ファイルを用意しないまま削除を実行（--yes で確認スキップ）
    let output = Command::new(sai_binary())
        .args(["block", "rm", "--yes", &exec_id.to_string()])
        .env("SAI_PROJECT_ROOT", &dir)
        .current_dir(&dir)
        .output()
        .expect("sai block rm の実行に失敗しました");
    assert!(
        output.status.success(),
        "記録ファイルが存在しなくても削除は失敗しないこと"
    );

    cleanup_temp_dir(&dir);
}

// ============================================================================
// `sai-link agent <...>`（plan-04）
// ============================================================================

/// `agent block-discover` が JSON 1 行を返すこと（空のブロック定義でも成功すること）。
#[test]
fn test_cli_agent_block_discover_returns_json() {
    let dir = create_temp_dir();

    let output = Command::new(sai_binary())
        .args(["agent", "block-discover"])
        .env("SAI_PROJECT_ROOT", &dir)
        .env_remove("SAI_BLOCK_NAME")
        .current_dir(&dir)
        .output()
        .expect("sai-link agent block-discover の実行に失敗しました");

    assert!(output.status.success(), "block-discover は成功すること");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let lines: Vec<&str> = stdout.lines().filter(|l| !l.trim().is_empty()).collect();
    assert_eq!(lines.len(), 1, "出力は JSON 1 行であること: {stdout:?}");
    let value: serde_json::Value = serde_json::from_str(lines[0]).expect("JSON として解釈できること");
    assert!(value.get("count").is_some(), "count フィールドを含むこと");
    assert!(value.get("blocks").is_some(), "blocks フィールドを含むこと");

    cleanup_temp_dir(&dir);
}

/// `agent task-manage` に不正な action を渡すと `{"error":"..."}` を stdout に出し、
/// 終了コード 1 になること。
#[test]
fn test_cli_agent_task_manage_invalid_action_errors() {
    let dir = create_temp_dir();

    let output = Command::new(sai_binary())
        .args(["agent", "task-manage", "bogus", "1"])
        .env("SAI_PROJECT_ROOT", &dir)
        .current_dir(&dir)
        .output()
        .expect("sai-link agent task-manage の実行に失敗しました");

    assert!(
        !output.status.success(),
        "不正な action は終了コード 1 になること"
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let value: serde_json::Value =
        serde_json::from_str(stdout.trim()).expect("stdout は JSON であること");
    assert!(
        value["error"]
            .as_str()
            .unwrap_or_default()
            .contains("無効な action"),
        "エラーメッセージが表示されること: {stdout:?}"
    );

    cleanup_temp_dir(&dir);
}

/// `agent task-list` を `SAI_PARENT_RUN_ID` 未設定で実行すると、
/// run_id 未設定を示すエラーが返ること。
#[test]
fn test_cli_agent_task_list_without_run_id_errors() {
    let dir = create_temp_dir();

    let output = Command::new(sai_binary())
        .args(["agent", "task-list"])
        .env("SAI_PROJECT_ROOT", &dir)
        .env_remove("SAI_PARENT_RUN_ID")
        .current_dir(&dir)
        .output()
        .expect("sai-link agent task-list の実行に失敗しました");

    assert!(!output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let value: serde_json::Value =
        serde_json::from_str(stdout.trim()).expect("stdout は JSON であること");
    assert_eq!(
        value["error"],
        "SAI_PARENT_RUN_ID が設定されていません（run 実行中ではありません）"
    );

    cleanup_temp_dir(&dir);
}

/// `init` は実行の記録の DB を作り、登録簿に載せること（デーモンを起動する前でも読む札を予約できるように）
#[test]
fn test_init_creates_engine_db_and_registers() {
    let dir = create_temp_dir();
    let root = dir.join("proj");
    let home = dir.join("home");
    std::fs::create_dir_all(&root).unwrap();
    let output = Command::new(sai_binary())
        .args(["init", "--name", "init-check"])
        .current_dir(&root)
        .env("SAI_PROJECT_ROOT", &root)
        .env("SAI_SAI_HOME", &home)
        .env_remove("SAI_PARENT_RUN_ID")
        .env_remove("SAI_BLOCK_NAME")
        .output()
        .expect("sai の実行に失敗しました");
    assert!(
        output.status.success(),
        "init が失敗した: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(root.join(".sai/sai-engine/sai-engine.db").exists(), "DB が作られること");
    let registry = std::fs::read_to_string(home.join("registry.json")).unwrap();
    assert!(registry.contains("init-check"), "登録簿に名前が載ること");
    cleanup_temp_dir(&dir);
}
