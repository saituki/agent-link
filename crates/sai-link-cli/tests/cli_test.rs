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
    PathBuf::from(env!("CARGO_BIN_EXE_sai-link"))
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

/// `--help` が Sai-Engine の説明と新しい命令の木を含み、古いグループを出さないこと
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
    // 新しい木のグループが出る
    for name in ["card", "run", "doc", "task", "def", "project", "session", "schema"] {
        assert!(stdout.contains(name), "ヘルプに {name} が含まれること: {stdout}");
    }
    // 古いグループは無い（隠した別名も消した）
    for name in ["phone", "agent", "block", "mcp"] {
        assert!(
            !stdout.contains(name),
            "ヘルプに古いグループ {name} が出ないこと: {stdout}"
        );
    }
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
    assert!(
        stderr.starts_with("引数の誤り: "),
        "日本語の 1 行が表示されること: {stderr:?}"
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

/// `run list --limit 5` が limit オプションを受け付けること
#[test]
fn test_cli_run_list_with_limit() {
    let dir = create_temp_dir();

    let output = Command::new(sai_binary())
        .args(["run", "list", "--limit", "5"])
        .env("SAI_PROJECT_ROOT", &dir)
        .current_dir(&dir)
        .output()
        .expect("sai run list --limit 5 の実行に失敗しました");

    // 空の DB でも limit オプション付きで正常動作すること
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("実行履歴はありません"));

    cleanup_temp_dir(&dir);
}

// ============================================================================
// run list / run show のテスト（空の DB で動作確認）
// ============================================================================

/// `run list` が空の DB で「履歴なし」メッセージを表示すること
#[test]
fn test_cli_run_list_empty() {
    let dir = create_temp_dir();

    let output = Command::new(sai_binary())
        .args(["run", "list"])
        .env("SAI_PROJECT_ROOT", &dir)
        .current_dir(&dir)
        .output()
        .expect("sai run list の実行に失敗しました");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("実行履歴はありません"),
        "空の DB では「履歴なし」メッセージが表示されること"
    );

    cleanup_temp_dir(&dir);
}


/// `run list` がデフォルトで実行履歴なしメッセージを表示すること
#[test]
fn test_cli_run_list_default() {
    let dir = create_temp_dir();

    let output = Command::new(sai_binary())
        .args(["run", "list"])
        .env("SAI_PROJECT_ROOT", &dir)
        .current_dir(&dir)
        .output()
        .expect("sai run list の実行に失敗しました");

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

/// R603: `run rm` で実行を削除すると、対応する run フォルダも丸ごと削除されること。
#[test]
fn test_cli_run_rm_deletes_reflection_file() {
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
        .args(["run", "rm", "--yes", &exec_id.to_string()])
        // 外部環境の SAI_PROJECT_ROOT を無視し、プロジェクトルートを一時ディレクトリに固定
        .env("SAI_PROJECT_ROOT", &dir)
        .current_dir(&dir)
        .output()
        .expect("sai run rm の実行に失敗しました");
    assert!(output.status.success(), "run rm が成功すること");

    assert!(
        !run_dir.exists(),
        "対応する run フォルダが丸ごと削除されること"
    );

    cleanup_temp_dir(&dir);
}

/// R13/AE1: 記録ファイルが存在しない状態でも `run rm` が失敗しないこと。
#[test]
fn test_cli_run_rm_missing_reflection_no_error() {
    let dir = create_temp_dir();

    let exec_id = setup_execution(&dir, "reflection-rm-missing");

    // Reflection ファイルを用意しないまま削除を実行（--yes で確認スキップ）
    let output = Command::new(sai_binary())
        .args(["run", "rm", "--yes", &exec_id.to_string()])
        .env("SAI_PROJECT_ROOT", &dir)
        .current_dir(&dir)
        .output()
        .expect("sai run rm の実行に失敗しました");
    assert!(
        output.status.success(),
        "記録ファイルが存在しなくても削除は失敗しないこと"
    );

    cleanup_temp_dir(&dir);
}

// ============================================================================
// `sai-link agent <...>`（plan-04）
// ============================================================================


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
        .env("SAI_HOME", &home)
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

// ============================================================================
// U06: 新しい命令の木と CLI 共通の約束
// ============================================================================

/// `.sai/` を持つ一時プロジェクトを作る（DB は命令を打った時点でできる）。
fn create_temp_project() -> PathBuf {
    let dir = create_temp_dir();
    std::fs::create_dir_all(dir.join(".sai")).expect("`.sai/` 作成に失敗");
    dir
}

/// 標準入力を与えて実行し、出力を返す。
fn run_with_stdin(args: &[&str], dir: &std::path::Path, input: &[u8]) -> std::process::Output {
    use std::io::Write;
    let mut child = Command::new(sai_binary())
        .args(args)
        .env("SAI_PROJECT_ROOT", dir)
        .env_remove("SAI_PARENT_RUN_ID")
        .current_dir(dir)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("コマンド起動に失敗");
    child
        .stdin
        .take()
        .expect("stdin を取れること")
        .write_all(input)
        .expect("標準入力の書き込みに失敗");
    child.wait_with_output().expect("実行に失敗")
}


/// `run tree` が JSON を返すこと。
#[test]
fn test_cli_run_tree_returns_json() {
    let dir = create_temp_project();
    let out = Command::new(sai_binary())
        .args(["run", "tree"])
        .env("SAI_PROJECT_ROOT", &dir)
        .current_dir(&dir)
        .output()
        .unwrap();
    assert!(out.status.success(), "run tree: {}", String::from_utf8_lossy(&out.stderr));
    let _: serde_json::Value = serde_json::from_slice(&out.stdout).expect("JSON であること");
    cleanup_temp_dir(&dir);
}

/// `card options` が JSON を返すこと。
#[test]
fn test_cli_card_options_returns_json() {
    let dir = create_temp_project();
    let out = Command::new(sai_binary())
        .args(["card", "options"])
        .env("SAI_PROJECT_ROOT", &dir)
        .current_dir(&dir)
        .output()
        .unwrap();
    assert!(out.status.success(), "card options: {}", String::from_utf8_lossy(&out.stderr));
    let _: serde_json::Value = serde_json::from_slice(&out.stdout).expect("JSON であること");
    cleanup_temp_dir(&dir);
}

/// `task wait` の件数の検査（0 件・51 件は終了コード 2、2 件は pending 付き JSON）。
#[test]
fn test_cli_task_wait_counts() {
    let dir = create_temp_project();
    // 0 件は clap が弾く
    let zero = Command::new(sai_binary())
        .args(["task", "wait"])
        .env("SAI_PROJECT_ROOT", &dir)
        .current_dir(&dir)
        .output()
        .unwrap();
    assert_eq!(zero.status.code(), Some(2), "0 件は 2");
    // 51 件は 2
    let mut args: Vec<String> = vec!["task".into(), "wait".into()];
    for i in 1..=51 {
        args.push(i.to_string());
    }
    let many = Command::new(sai_binary())
        .args(&args)
        .env("SAI_PROJECT_ROOT", &dir)
        .current_dir(&dir)
        .output()
        .unwrap();
    assert_eq!(many.status.code(), Some(2), "51 件は 2");
    // 2 件・待たない: pending 付きの JSON
    let two = Command::new(sai_binary())
        .args(["task", "wait", "1", "2", "--timeout-ms", "0"])
        .env("SAI_PROJECT_ROOT", &dir)
        .current_dir(&dir)
        .output()
        .unwrap();
    assert!(two.status.success(), "task wait 1 2: {}", String::from_utf8_lossy(&two.stderr));
    let value: serde_json::Value = serde_json::from_slice(&two.stdout).expect("JSON であること");
    assert!(value.get("pending").is_some(), "pending が付く: {value:?}");
    cleanup_temp_dir(&dir);
}

/// `run show 999999` は終了コード 3・stdout 空・stderr 1 行。
#[test]
fn test_cli_run_show_not_found() {
    let dir = create_temp_project();
    let out = Command::new(sai_binary())
        .args(["run", "show", "999999"])
        .env("SAI_PROJECT_ROOT", &dir)
        .current_dir(&dir)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(3), "見つからない: {}", String::from_utf8_lossy(&out.stderr));
    assert!(out.stdout.is_empty(), "stdout は空");
    let stderr = String::from_utf8_lossy(&out.stderr);
    let lines: Vec<&str> = stderr.lines().filter(|l| !l.trim().is_empty()).collect();
    assert_eq!(lines.len(), 1, "stderr は 1 行: {stderr:?}");
    cleanup_temp_dir(&dir);
}

/// `run show abc` は終了コード 2・stderr の最後の行が `引数の誤り: ` で始まる。
#[test]
fn test_cli_run_show_parse_error() {
    let out = Command::new(sai_binary())
        .args(["run", "show", "abc"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2), "引数の誤りは 2");
    assert!(out.stdout.is_empty(), "stdout は空");
    let stderr = String::from_utf8_lossy(&out.stderr);
    let last = stderr
        .lines()
        .filter(|l| !l.trim().is_empty())
        .last()
        .unwrap_or("");
    assert!(
        last.starts_with("引数の誤り: "),
        "最後の行が「引数の誤り: 」で始まる: {stderr:?}"
    );
    // clap の英語の複数行は出さず、日本語の 1 行だけにする
    assert_eq!(stderr.lines().count(), 1, "stderr は 1 行: {stderr:?}");
}

/// `.sai/` の無いフォルダで `run list` は終了コード 2。
#[test]
fn test_cli_run_list_without_project() {
    let dir = create_temp_dir();
    let home = create_temp_dir();
    let out = Command::new(sai_binary())
        .args(["run", "list"])
        .env_remove("SAI_PROJECT_ROOT")
        .env("HOME", &home)
        .current_dir(&dir)
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(2),
        "プロジェクトが無ければ 2: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(out.stdout.is_empty(), "stdout は空");
    cleanup_temp_dir(&dir);
    cleanup_temp_dir(&home);
}

/// ホームの `.sai/` をプロジェクトと見なさない。
#[test]
fn test_cli_run_list_ignores_home_sai() {
    let base = create_temp_dir().canonicalize().unwrap();
    let home = base.join("home");
    let sub = home.join("sub");
    std::fs::create_dir_all(home.join(".sai")).unwrap();
    std::fs::create_dir_all(&sub).unwrap();
    let out = Command::new(sai_binary())
        .args(["run", "list"])
        .env_remove("SAI_PROJECT_ROOT")
        .env("HOME", &home)
        .current_dir(&sub)
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(2),
        "ホームの .sai は数えない: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    cleanup_temp_dir(&base);
}

/// `schema --json` が「schema の形」で出る。
#[test]
fn test_cli_schema_json() {
    let out = Command::new(sai_binary())
        .args(["schema", "--json"])
        .output()
        .unwrap();
    assert!(out.status.success(), "schema: {}", String::from_utf8_lossy(&out.stderr));
    let value: serde_json::Value = serde_json::from_slice(&out.stdout).expect("JSON");
    assert_eq!(value["bin"], "sai-link");
    let commands = value["commands"].as_array().expect("commands 配列");
    let card_create = commands
        .iter()
        .find(|c| c["path"] == serde_json::json!(["card", "create"]))
        .expect("card create があること");
    assert_eq!(card_create["stdin"], "json");
    assert_eq!(card_create["json"], true);
    let doc_write = commands
        .iter()
        .find(|c| c["path"] == serde_json::json!(["doc", "write"]))
        .expect("doc write があること");
    assert_eq!(doc_write["web"], false, "doc write は Web に出さない");
    // 隠した命令は出ない
    for hidden in ["phone", "agent", "block", "block-def", "mcp", "search"] {
        assert!(
            commands.iter().all(|c| {
                c["path"].as_array().and_then(|p| p.first()).and_then(|x| x.as_str())
                    != Some(hidden)
            }),
            "隠した命令 {hidden} が出ないこと"
        );
    }
}

/// `card create` の `meta` が task.md の frontmatter に書かれる。
#[test]
fn test_cli_card_create_writes_meta() {
    let dir = create_temp_project();
    let out = run_with_stdin(
        &["card", "create"],
        &dir,
        br#"{"task_text":"hello","meta":{"sci_node":"Q12"}}"#,
    );
    assert!(
        out.status.success(),
        "card create: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&out.stdout).expect("JSON");
    let path = value["path"].as_str().expect("path があること");
    let text = std::fs::read_to_string(path).expect("task.md を読めること");
    assert!(text.contains("sci_node: Q12"), "meta が frontmatter に書かれる: {text}");
    cleanup_temp_dir(&dir);
}

/// `card create` の `meta` は標準のキーを断る（終了コード 2）。
#[test]
fn test_cli_card_create_rejects_standard_meta() {
    let dir = create_temp_project();
    let out = run_with_stdin(
        &["card", "create"],
        &dir,
        br#"{"task_text":"x","meta":{"run_id":"7"}}"#,
    );
    assert_eq!(out.status.code(), Some(2), "標準のキーは 2");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("標準のキー"), "理由を出す: {stderr:?}");
    cleanup_temp_dir(&dir);
}
