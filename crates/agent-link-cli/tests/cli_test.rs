//! CLI 統合テスト
//!
//! clap の引数パース、ヘルプ表示、エラーハンドリングを
//! `std::process::Command` 経由で検証します。
//!
//! 内部ヘルパー関数（format_status, format_elapsed 等）のテストは
//! `cli.rs` 内の `#[cfg(test)] mod tests` で実施済みです（24件パス確認済み）。

use std::path::PathBuf;
use std::process::Command;

/// `agent-link` のバイナリパスを取得する（Cargo が統合テスト用に提供する env を使用）
fn sai_binary() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_agent-link"))
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
        std::env::temp_dir().join(format!("agent-link-test-{}-{}", std::process::id(), count));
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

/// `--help` が agent-link の説明と新しい命令の木を含み、古いグループを出さないこと
#[test]
fn test_cli_help() {
    let output = Command::new(sai_binary())
        .arg("--help")
        .output()
        .expect("agent-link の実行に失敗しました");
    assert!(output.status.success(), "exit code 0 expected");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("agent-link"),
        "ヘルプに agent-link が含まれること"
    );
    // 新しい木のグループが出る
    for name in ["card", "run", "doc", "task", "def", "project", "session", "schema"] {
        assert!(stdout.contains(name), "ヘルプに {name} が含まれること: {stdout}");
    }
    // 古いグループは無い（隠した別名も消した）
    // （命令名は `  <名前> ` で始まる行に出る。実行ファイル名 `agent-link` には当たらない）
    for name in ["phone", "agent", "block", "mcp"] {
        assert!(
            !stdout.contains(&format!("\n  {name} ")),
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
        .expect("agent-link の実行に失敗しました");
    assert!(
        !output.status.success(),
        "不正なサブコマンドはエラー終了すること"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.starts_with("Invalid arguments: "),
        "日本語の 1 行が表示されること: {stderr:?}"
    );
}


/// `--version` がバージョン情報を表示すること
#[test]
fn test_cli_version() {
    let output = Command::new(sai_binary())
        .arg("--version")
        .output()
        .expect("agent-link --version の実行に失敗しました");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("0.1.0") || stdout.contains("agent-link"));
}

/// `run list --limit 5` が limit オプションを受け付けること
#[test]
fn test_cli_run_list_with_limit() {
    let dir = create_temp_project();

    let output = Command::new(sai_binary())
        .args(["run", "list", "--limit", "5"])
        .env("AGENT_LINK_PROJECT_ROOT", &dir)
        .current_dir(&dir)
        .output()
        .expect("agent-link run list --limit 5 の実行に失敗しました");

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
    let dir = create_temp_project();

    let output = Command::new(sai_binary())
        .args(["run", "list"])
        .env("AGENT_LINK_PROJECT_ROOT", &dir)
        .current_dir(&dir)
        .output()
        .expect("agent-link run list の実行に失敗しました");

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
    let dir = create_temp_project();

    let output = Command::new(sai_binary())
        .args(["run", "list"])
        .env("AGENT_LINK_PROJECT_ROOT", &dir)
        .current_dir(&dir)
        .output()
        .expect("agent-link run list の実行に失敗しました");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("実行履歴はありません"));

    cleanup_temp_dir(&dir);
}

// ============================================================================
// Reflection 記録ファイル削除（R13）
// ============================================================================

use agent_link_engine::db::models::BlockMetadata;
use agent_link_engine::db::SaiStore;

/// 一時ディレクトリに DB と実行レコードを作成し、実行 ID を返す。
fn setup_execution(dir: &std::path::Path, block_name: &str) -> i64 {
    let db_path = dir.join(".agent-link/engine/agent-link.db");
    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent).expect("DB ディレクトリ作成に失敗");
    }
    let store = SaiStore::open(&db_path).expect("テスト用 DB を開けること");

    let block = BlockMetadata::new(
        block_name.to_string(),
        "テスト用ブロック".to_string(),
        "anthropic/claude-sonnet-4-5".to_string(),
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
    // 消せる状態（実行中・待機中は `run rm` が 4 で断る）にしておく
    let id = store
        .create_execution(block_id, None)
        .expect("実行レコードを作成できること");
    store
        .update_execution_status(id, "completed")
        .expect("状態を更新できること");
    id
}

/// 一時プロジェクトで CLI を走らせる（`AGENT_LINK_PARENT_RUN_ID` は外す）。
fn run_cli(dir: &std::path::Path, args: &[&str], stdin: Option<&str>) -> std::process::Output {
    use std::io::Write;
    use std::process::Stdio;
    let mut child = Command::new(sai_binary())
        .args(args)
        .env("AGENT_LINK_PROJECT_ROOT", dir)
        .env("AGENT_LINK_HOME", dir.join("home"))
        .env_remove("AGENT_LINK_PARENT_RUN_ID")
        .current_dir(dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("agent-link を起動できること");
    if let Some(text) = stdin {
        child.stdin.take().unwrap().write_all(text.as_bytes()).unwrap();
    }
    child.wait_with_output().unwrap()
}

fn last_stderr_line(out: &std::process::Output) -> String {
    String::from_utf8_lossy(&out.stderr)
        .lines()
        .last()
        .unwrap_or("")
        .to_string()
}

/// `run rm --json` は JSON を返し、無い run は 3、実行中は 4（標準出力は空）。
#[test]
fn test_cli_run_rm_json_and_exit_codes() {
    let dir = create_temp_dir();
    let done = setup_execution(&dir, "rm-json");
    let store = SaiStore::open(&dir.join(".agent-link/engine/agent-link.db")).unwrap();
    let running = store.create_execution(1, None).unwrap();
    store.update_execution_status(running, "running").unwrap();
    drop(store);

    let out = run_cli(&dir, &["run", "rm", &running.to_string(), "--yes", "--json"], None);
    assert_eq!(out.status.code(), Some(4));
    assert!(out.stdout.is_empty());
    assert_eq!(last_stderr_line(&out), "A running run cannot be deleted. Stop it first.");

    let out = run_cli(&dir, &["run", "rm", "999", "--yes", "--json"], None);
    assert_eq!(out.status.code(), Some(3));
    assert!(out.stdout.is_empty());
    assert_eq!(last_stderr_line(&out), "Run 999 not found.");

    let out = run_cli(&dir, &["run", "rm", &done.to_string(), "--yes", "--json"], None);
    assert_eq!(out.status.code(), Some(0));
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["run_id"], done);
    assert_eq!(v["executions"], 1);

    cleanup_temp_dir(&dir);
}

/// `doc write` の直後に、デーモン無しで `doc list --run <id> --all --json` に出る（同じトークンは 1 件）。
#[test]
fn test_cli_doc_write_then_doc_list_without_daemon() {
    let dir = create_temp_dir();
    let run = setup_execution(&dir, "doc-list");
    let run_s = run.to_string();

    let out = run_cli(
        &dir,
        &["doc", "write", "--filename", "a.md", "--description", "A", "--run-id", &run_s, "--json"],
        Some("# a\n"),
    );
    assert_eq!(out.status.code(), Some(0), "{}", String::from_utf8_lossy(&out.stderr));
    let w: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let token = w["token"].as_str().expect("token が返る").to_string();

    let out = run_cli(&dir, &["doc", "list", "--run", &run_s, "--all", "--json"], None);
    assert_eq!(out.status.code(), Some(0));
    let l: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(l["count"], 1);
    assert_eq!(l["items"][0]["token"], token.as_str());

    // run 指定なしの一覧にも出る
    let out = run_cli(&dir, &["doc", "list", "--all", "--json"], None);
    let l: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(l["count"], 1);

    cleanup_temp_dir(&dir);
}

/// `run finalize`: 無い run は 3、実行中でない run は 1（標準出力は空）。
#[test]
fn test_cli_run_finalize_rejects_missing_and_finished() {
    let dir = create_temp_dir();
    let done = setup_execution(&dir, "finalize-done");

    let out = run_cli(&dir, &["run", "finalize", "999", "--json"], None);
    assert_eq!(out.status.code(), Some(3));
    assert!(out.stdout.is_empty());
    assert_eq!(last_stderr_line(&out), "Run 999 not found.");

    let out = run_cli(&dir, &["run", "finalize", &done.to_string(), "--json"], None);
    assert_eq!(out.status.code(), Some(1));
    assert!(out.stdout.is_empty());
    assert_eq!(
        last_stderr_line(&out),
        format!("Run {} is not running (status: completed).", done)
    );

    cleanup_temp_dir(&dir);
}

/// `task create` の検査失敗は 2・標準出力は空。run の外の `task list` は 2。
#[test]
fn test_cli_task_create_failure_and_outside_run() {
    let dir = create_temp_dir();
    let run = setup_execution(&dir, "task-parent");

    let body = r#"[{"client_id":"a","subject":"s","task_text":"t","block_type":"no-such-block"}]"#;
    let out = run_cli(
        &dir,
        &["task", "create", "--run-id", &run.to_string(), "--json"],
        Some(body),
    );
    assert_eq!(out.status.code(), Some(2));
    assert!(out.stdout.is_empty());
    assert!(last_stderr_line(&out).starts_with("Input validation of task_create failed."));

    let out = run_cli(&dir, &["task", "list", "--json"], None);
    assert_eq!(out.status.code(), Some(2));
    assert!(out.stdout.is_empty());
    assert!(last_stderr_line(&out).contains("AGENT_LINK_PARENT_RUN_ID is not set"));

    cleanup_temp_dir(&dir);
}

/// `init --name`: 形が悪ければ 2（何も作らない）、重複は 1。最後の案内に `.claude/` の commit が無い。
#[test]
fn test_cli_init_name_checks() {
    let dir = create_temp_dir();
    let other = create_temp_dir();

    let out = run_cli(&dir, &["init", "--name", "bad/name"], None);
    assert_eq!(out.status.code(), Some(2));
    assert!(out.stdout.is_empty());
    assert!(last_stderr_line(&out).starts_with("Invalid project name: bad/name."));
    assert!(!dir.join(".agent-link").exists());

    let home = dir.join("home");
    let run_init = |root: &std::path::Path| {
        Command::new(sai_binary())
            .args(["init", "--name", "dup"])
            .env("AGENT_LINK_PROJECT_ROOT", root)
            .env("AGENT_LINK_HOME", &home)
            .current_dir(root)
            .output()
            .unwrap()
    };
    let out = run_init(&dir);
    assert_eq!(out.status.code(), Some(0));
    assert!(!String::from_utf8_lossy(&out.stdout).contains("git commit"));

    let out = run_init(&other);
    assert_eq!(out.status.code(), Some(1));
    assert!(last_stderr_line(&out).starts_with("Failed to register the project: Name dup is already used"));

    cleanup_temp_dir(&dir);
    cleanup_temp_dir(&other);
}

/// R603: `run rm` で実行を削除すると、対応する run フォルダも丸ごと削除されること。
#[test]
fn test_cli_run_rm_deletes_reflection_file() {
    let dir = create_temp_dir();

    let exec_id = setup_execution(&dir, "reflection-rm-test");
    // run フォルダ＋旧 reflection を用意しておく（完了後の状態を想定）
    let run_dir = dir
        .join(".agent-link/outputs/review")
        .join(exec_id.to_string());
    std::fs::create_dir_all(&run_dir).expect("ディレクトリ作成に失敗");
    std::fs::write(run_dir.join("review.md"), "### 追加修正\n").expect("review を用意できること");

    // 実行を削除（--yes で確認スキップ）
    let output = Command::new(sai_binary())
        .args(["run", "rm", "--yes", &exec_id.to_string()])
        // 外部環境の AGENT_LINK_PROJECT_ROOT を無視し、プロジェクトルートを一時ディレクトリに固定
        .env("AGENT_LINK_PROJECT_ROOT", &dir)
        .current_dir(&dir)
        .output()
        .expect("agent-link run rm の実行に失敗しました");
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
        .env("AGENT_LINK_PROJECT_ROOT", &dir)
        .current_dir(&dir)
        .output()
        .expect("agent-link run rm の実行に失敗しました");
    assert!(
        output.status.success(),
        "記録ファイルが存在しなくても削除は失敗しないこと"
    );

    cleanup_temp_dir(&dir);
}

// ============================================================================
// `agent-link agent <...>`（plan-04）
// ============================================================================


/// `init` は実行の記録の DB を作り、登録簿に載せること（デーモンを起動する前でも READ_CARD を予約できるように）
#[test]
fn test_init_creates_engine_db_and_registers() {
    let dir = create_temp_dir();
    let root = dir.join("proj");
    let home = dir.join("home");
    std::fs::create_dir_all(&root).unwrap();
    let output = Command::new(sai_binary())
        .args(["init", "--name", "init-check"])
        .current_dir(&root)
        .env("AGENT_LINK_PROJECT_ROOT", &root)
        .env("AGENT_LINK_HOME", &home)
        .env_remove("AGENT_LINK_PARENT_RUN_ID")
        .env_remove("AGENT_LINK_BLOCK_NAME")
        .output()
        .expect("agent-link の実行に失敗しました");
    assert!(
        output.status.success(),
        "init が失敗した: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(root.join(".agent-link/engine/agent-link.db").exists(), "DB が作られること");
    let registry = std::fs::read_to_string(home.join("registry.json")).unwrap();
    assert!(registry.contains("init-check"), "登録簿に名前が載ること");
    cleanup_temp_dir(&dir);
}

// ============================================================================
// U06: 新しい命令の木と CLI 共通の約束
// ============================================================================

/// `.agent-link/` を持つ一時プロジェクトを作る（DB は命令を打った時点でできる）。
fn create_temp_project() -> PathBuf {
    let dir = create_temp_dir();
    std::fs::create_dir_all(dir.join(".agent-link")).expect("`.agent-link/` 作成に失敗");
    dir
}

/// 標準入力を与えて実行し、出力を返す。
fn run_with_stdin(args: &[&str], dir: &std::path::Path, input: &[u8]) -> std::process::Output {
    use std::io::Write;
    let mut child = Command::new(sai_binary())
        .args(args)
        .env("AGENT_LINK_PROJECT_ROOT", dir)
        .env_remove("AGENT_LINK_PARENT_RUN_ID")
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
        .env("AGENT_LINK_PROJECT_ROOT", &dir)
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
        .env("AGENT_LINK_PROJECT_ROOT", &dir)
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
        .env("AGENT_LINK_PROJECT_ROOT", &dir)
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
        .env("AGENT_LINK_PROJECT_ROOT", &dir)
        .current_dir(&dir)
        .output()
        .unwrap();
    assert_eq!(many.status.code(), Some(2), "51 件は 2");
    // 2 件だが存在しない: 待たずに「見つからない」（終了コード 3・stdout 空）
    // （pending 付きの JSON は agent_api の task_output_ids の試験で確かめる）
    let two = Command::new(sai_binary())
        .args(["task", "wait", "1", "2", "--timeout-ms", "0"])
        .env("AGENT_LINK_PROJECT_ROOT", &dir)
        .current_dir(&dir)
        .output()
        .unwrap();
    assert_eq!(two.status.code(), Some(3), "task wait 1 2: {}", String::from_utf8_lossy(&two.stderr));
    assert!(two.stdout.is_empty(), "stdout は空");
    assert!(String::from_utf8_lossy(&two.stderr).contains("Task 1 not found."), "{}", String::from_utf8_lossy(&two.stderr));
    cleanup_temp_dir(&dir);
}

/// `run show 999999` は終了コード 3・stdout 空・stderr 1 行。
#[test]
fn test_cli_run_show_not_found() {
    let dir = create_temp_project();
    let out = Command::new(sai_binary())
        .args(["run", "show", "999999"])
        .env("AGENT_LINK_PROJECT_ROOT", &dir)
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

/// `run show abc` は終了コード 2・stderr の最後の行が `Invalid arguments: ` で始まる。
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
        last.starts_with("Invalid arguments: "),
        "最後の行が`Invalid arguments: `で始まる: {stderr:?}"
    );
    // clap の英語の複数行は出さず、日本語の 1 行だけにする
    assert_eq!(stderr.lines().count(), 1, "stderr は 1 行: {stderr:?}");
}

/// `.agent-link/` の無いフォルダで `run list` は終了コード 2。
#[test]
fn test_cli_run_list_without_project() {
    let dir = create_temp_dir();
    let home = create_temp_dir();
    let out = Command::new(sai_binary())
        .args(["run", "list"])
        .env_remove("AGENT_LINK_PROJECT_ROOT")
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

/// ホームの `.agent-link/` をプロジェクトと見なさない。
#[test]
fn test_cli_run_list_ignores_home_sai() {
    let base = create_temp_dir().canonicalize().unwrap();
    let home = base.join("home");
    let sub = home.join("sub");
    std::fs::create_dir_all(home.join(".agent-link")).unwrap();
    std::fs::create_dir_all(&sub).unwrap();
    let out = Command::new(sai_binary())
        .args(["run", "list"])
        .env_remove("AGENT_LINK_PROJECT_ROOT")
        .env("HOME", &home)
        .current_dir(&sub)
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(2),
        "ホームの .agent-link は数えない: {}",
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
    assert_eq!(value["bin"], "agent-link");
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

/// `card create` の検査を通す最小の block を、一時プロジェクトに書く。
fn write_card_block(dir: &std::path::Path) {
    let blocks = dir.join(".agent-link/blocks");
    std::fs::create_dir_all(&blocks).expect("blocks 作成に失敗");
    std::fs::write(blocks.join("t.md"), "---\nname: t\nid: t\n---\n本文\n")
        .expect("block 書き込みに失敗");
}

/// `card create` の `meta` が task.md の frontmatter に書かれる。
#[test]
fn test_cli_card_create_writes_meta() {
    let dir = create_temp_project();
    write_card_block(&dir);
    let out = run_with_stdin(
        &["card", "create"],
        &dir,
        br#"{"block":"t","task_text":"hello","meta":{"note":"n1"}}"#,
    );
    assert!(
        out.status.success(),
        "card create: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&out.stdout).expect("JSON");
    let path = value["path"].as_str().expect("path があること");
    let text = std::fs::read_to_string(path).expect("task.md を読めること");
    assert!(text.contains("note: n1"), "meta が frontmatter に書かれる: {text}");
    cleanup_temp_dir(&dir);
}

/// `card create` の `meta` は標準のキーを断る（終了コード 2）。
#[test]
fn test_cli_card_create_rejects_standard_meta() {
    let dir = create_temp_project();
    write_card_block(&dir);
    let out = run_with_stdin(
        &["card", "create"],
        &dir,
        br#"{"block":"t","task_text":"x","meta":{"run_id":"7"}}"#,
    );
    assert_eq!(out.status.code(), Some(2), "標準のキーは 2");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("is reserved"), "理由を出す: {stderr:?}");
    cleanup_temp_dir(&dir);
}

/// `card create` は block 無し・知らない model（2）と、無い doc_tokens（3）を断る。標準出力は空。
#[test]
fn test_cli_card_create_rejects_bad_input() {
    let dir = create_temp_project();
    write_card_block(&dir);
    let cases: [(&[u8], i32, &str); 3] = [
        (br#"{"task_text":"x"}"#, 2, "block is required."),
        (br#"{"block":"t","model":"gpt","task_text":"x"}"#, 2, "Invalid model: gpt."),
        (
            br#"{"block":"t","task_text":"x","doc_tokens":["ZZZZZZZZ"]}"#,
            3,
            "Document not found: ZZZZZZZZ.",
        ),
    ];
    for (input, code, msg) in cases {
        let out = run_with_stdin(&["card", "create", "--json"], &dir, input);
        assert_eq!(out.status.code(), Some(code), "{}", String::from_utf8_lossy(input));
        assert!(out.stdout.is_empty(), "失敗時の標準出力は空");
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(stderr.trim_end().lines().last().unwrap().starts_with(msg), "{stderr:?}");
    }
    cleanup_temp_dir(&dir);
}

/// `.agent-link/` の無い `AGENT_LINK_PROJECT_ROOT` は 2 で断り、フォルダを作らない。
#[test]
fn test_cli_project_root_env_without_agent_link_is_rejected() {
    let dir = create_temp_dir();
    let out = Command::new(sai_binary())
        .args(["run", "list", "--json"])
        .env("AGENT_LINK_PROJECT_ROOT", &dir)
        .current_dir(&dir)
        .output()
        .expect("実行に失敗");
    assert_eq!(out.status.code(), Some(2));
    assert!(out.stdout.is_empty());
    assert!(String::from_utf8_lossy(&out.stderr).contains("has no .agent-link/"));
    assert!(!dir.join(".agent-link").exists(), "フォルダを作らない");
    cleanup_temp_dir(&dir);
}
