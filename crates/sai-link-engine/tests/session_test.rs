//! # セッション管理モジュールの単体テスト
//!
//! JSONL セッションファイルの作成・読み取り・クリーンアップのテスト。
//! 実際のファイルシステムを使用するため、一時ディレクトリ上でテストを実行します。

use sai_link_engine::session::{
    cleanup_session_dir, create_session_dir, create_session_file, extract_final_output,
    extract_output_from_dir,
};
use std::path::Path;

// ---------------------------------------------------------------------------
// ヘルパー
// ---------------------------------------------------------------------------

/// テスト用のセッションディレクトリを作成し、クリーンアップを保証する。
struct TestSessionDir {
    path: std::path::PathBuf,
}

impl TestSessionDir {
    fn new(agent_name: &str) -> Self {
        let path = create_session_dir(agent_name).expect("セッションディレクトリの作成に失敗");
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TestSessionDir {
    fn drop(&mut self) {
        cleanup_session_dir(&self.path);
    }
}

// ---------------------------------------------------------------------------
// テストケース
// ---------------------------------------------------------------------------

/// `create_session_dir()` が正しい形式のディレクトリを作成することを確認。
#[test]
fn test_create_session_dir() {
    let test_dir = TestSessionDir::new("test-agent");
    let path_str = test_dir.path().to_string_lossy();

    // .sai/sessions/pi-sessions- を含むことを確認
    assert!(
        path_str.contains(".sai/sessions/pi-sessions-"),
        "パスが '.sai/sessions/pi-sessions-' を含むべき: {}",
        path_str
    );

    // ディレクトリが存在することを確認
    assert!(
        test_dir.path().exists(),
        "ディレクトリが存在するべき: {}",
        path_str
    );
    assert!(
        test_dir.path().is_dir(),
        "ディレクトリであるべき: {}",
        path_str
    );
}

/// `create_session_file()` が正しい JSONL ヘッダーを含むファイルを作成することを確認。
#[test]
fn test_create_session_file() {
    let test_dir = TestSessionDir::new("test-agent");
    let file_path =
        create_session_file(test_dir.path(), "test-agent").expect("セッションファイル作成に失敗");

    // ファイルが存在することを確認
    assert!(
        file_path.exists(),
        "セッションファイルが存在するべき: {}",
        file_path.display()
    );

    // 拡張子が .jsonl であることを確認
    assert_eq!(
        file_path.extension().unwrap(),
        "jsonl",
        "拡張子が .jsonl であるべき"
    );

    // ファイル名にエージェント名が含まれていることを確認
    let filename = file_path.file_name().unwrap().to_string_lossy();
    assert!(
        filename.contains("test-agent"),
        "ファイル名にエージェント名が含まれるべき: {}",
        filename
    );

    // 1行目が正しいJSONセッションヘッダーであることを確認
    let content = std::fs::read_to_string(&file_path).expect("ファイル読み込みに失敗");
    let first_line = content
        .lines()
        .next()
        .expect("ファイルが空であるべきではない");

    let header: serde_json::Value =
        serde_json::from_str(first_line).expect("1行目が有効なJSONであるべき");

    assert_eq!(header["type"], "session");
    assert_eq!(header["version"], 3);
    assert!(header["id"].is_string());
    assert!(header["timestamp"].is_string());
    assert!(header["cwd"].is_string());
}

/// `create_session_file()` が特殊文字を含むエージェント名でも正しく動作することを確認。
#[test]
fn test_create_session_file_special_chars() {
    let test_dir = TestSessionDir::new("special@agent#123");
    let file_path = create_session_file(test_dir.path(), "special@agent#123")
        .expect("特殊文字を含むエージェント名でも成功するべき");

    let filename = file_path.file_name().unwrap().to_string_lossy();
    // 特殊文字が _ に置換され、長さが制限されていることを確認
    assert!(
        filename.contains("special_agent_123"),
        "特殊文字がアンダースコアに置換されるべき: {}",
        filename
    );
}

/// `extract_final_output()` が単一のアシスタントメッセージを正しく抽出することを確認。
#[test]
fn test_extract_final_output_single_message() {
    let test_dir = TestSessionDir::new("single-msg-test");
    let session_path = test_dir.path().join("test.jsonl");

    let content = r#"{"type":"session","version":3,"id":"test","timestamp":"2026-01-01T00:00:00Z","cwd":"/tmp"}
{"type":"message","message":{"role":"user","content":[{"type":"text","text":"hello"}]}}
{"type":"message","message":{"role":"assistant","content":[{"type":"text","text":"Hello! How can I help you?"}]}}
"#;
    std::fs::write(&session_path, content).expect("テストファイルの書き込みに失敗");

    let output = extract_final_output(&session_path).expect("出力抽出に成功するべき");
    assert_eq!(output, "Hello! How can I help you?");
}

/// `extract_final_output()` が複数メッセージから最後のアシスタントテキストのみを抽出することを確認。
#[test]
fn test_extract_final_output_multiple_messages() {
    let test_dir = TestSessionDir::new("multi-msg-test");
    let session_path = test_dir.path().join("test.jsonl");

    let content = r#"{"type":"session","version":3,"id":"test","timestamp":"2026-01-01T00:00:00Z","cwd":"/tmp"}
{"type":"message","message":{"role":"user","content":[{"type":"text","text":"first"}]}}
{"type":"message","message":{"role":"assistant","content":[{"type":"text","text":"First response"}]}}
{"type":"message","message":{"role":"user","content":[{"type":"text","text":"second"}]}}
{"type":"message","message":{"role":"assistant","content":[{"type":"text","text":"Second response"}]}}
"#;
    std::fs::write(&session_path, content).expect("テストファイルの書き込みに失敗");

    let output = extract_final_output(&session_path).expect("出力抽出に成功するべき");
    assert_eq!(output, "Second response");
}

/// `extract_final_output()` がアシスタントメッセージがない場合に空文字列を返すことを確認。
#[test]
fn test_extract_final_output_no_assistant() {
    let test_dir = TestSessionDir::new("no-assistant-test");
    let session_path = test_dir.path().join("test.jsonl");

    let content = r#"{"type":"session","version":3,"id":"test","timestamp":"2026-01-01T00:00:00Z","cwd":"/tmp"}
{"type":"message","message":{"role":"user","content":[{"type":"text","text":"hello"}]}}
"#;
    std::fs::write(&session_path, content).expect("テストファイルの書き込みに失敗");

    let output = extract_final_output(&session_path).expect("出力抽出に成功するべき");
    assert_eq!(
        output, "",
        "アシスタントメッセージがない場合は空文字列を返すべき"
    );
}

/// `extract_final_output()` が thinking ブロックをスキップし、text ブロックのみ抽出することを確認。
#[test]
fn test_extract_final_output_skips_thinking() {
    let test_dir = TestSessionDir::new("thinking-test");
    let session_path = test_dir.path().join("test.jsonl");

    let content = r#"{"type":"session","version":3,"id":"test","timestamp":"2026-01-01T00:00:00Z","cwd":"/tmp"}
{"type":"message","message":{"role":"assistant","content":[{"type":"thinking","text":"thinking content"},{"type":"text","text":"Final answer"}]}}
"#;
    std::fs::write(&session_path, content).expect("テストファイルの書き込みに失敗");

    let output = extract_final_output(&session_path).expect("出力抽出に成功するべき");
    assert_eq!(
        output, "Final answer",
        "thinking ブロックをスキップして text のみ抽出するべき"
    );
}

/// `extract_final_output()` が thinking のみで text がない場合に空文字列を返すことを確認。
#[test]
fn test_extract_final_output_only_thinking() {
    let test_dir = TestSessionDir::new("only-thinking-test");
    let session_path = test_dir.path().join("test.jsonl");

    let content = r#"{"type":"session","version":3,"id":"test","timestamp":"2026-01-01T00:00:00Z","cwd":"/tmp"}
{"type":"message","message":{"role":"assistant","content":[{"type":"thinking","text":"just thinking"}]}}
"#;
    std::fs::write(&session_path, content).expect("テストファイルの書き込みに失敗");

    let output = extract_final_output(&session_path).expect("出力抽出に成功するべき");
    assert_eq!(output, "", "text ブロックがない場合も空文字列を返すべき");
}

/// `extract_final_output()` が不正な JSON 行をスキップすることを確認。
#[test]
fn test_extract_final_output_handles_invalid_json() {
    let test_dir = TestSessionDir::new("invalid-json-test");
    let session_path = test_dir.path().join("test.jsonl");

    let content = r#"{"type":"session","version":3,"id":"test","timestamp":"2026-01-01T00:00:00Z","cwd":"/tmp"}
{"type":"message","message":{"role":"assistant","content":[{"type":"text","text":"valid"}]}}
invalid json line
{"type":"message","message":{"role":"assistant","content":[{"type":"text","text":"after invalid"}]}}
"#;
    std::fs::write(&session_path, content).expect("テストファイルの書き込みに失敗");

    let output = extract_final_output(&session_path).expect("出力抽出に成功するべき");
    // 不正なJSON行はスキップされ、最後の有効なアシスタントメッセージが抽出される
    assert_eq!(output, "after invalid");
}

/// `extract_output_from_dir()` がディレクトリ内の全 `.jsonl` ファイルから出力を抽出することを確認。
#[test]
fn test_extract_output_from_dir() {
    let test_dir = TestSessionDir::new("dir-scan-test");

    // 複数のセッションファイルを作成
    let file1 = test_dir.path().join("session1.jsonl");
    let content1 = r#"{"type":"session","version":3,"id":"s1","timestamp":"2026-01-01T00:00:00Z","cwd":"/tmp"}
{"type":"message","message":{"role":"assistant","content":[{"type":"text","text":"Output from session 1"}]}}
"#;
    std::fs::write(&file1, content1).expect("ファイル1の書き込みに失敗");

    let file2 = test_dir.path().join("session2.jsonl");
    let content2 = r#"{"type":"session","version":3,"id":"s2","timestamp":"2026-01-01T00:00:01Z","cwd":"/tmp"}
{"type":"message","message":{"role":"assistant","content":[{"type":"text","text":"Output from session 2"}]}}
"#;
    std::fs::write(&file2, content2).expect("ファイル2の書き込みに失敗");

    let output =
        extract_output_from_dir(test_dir.path()).expect("ディレクトリからの出力抽出に成功するべき");
    // 最後の非空出力（session2）が返される
    assert_eq!(output, "Output from session 2");
}

/// `extract_output_from_dir()` が空のディレクトリで空文字列を返すことを確認。
#[test]
fn test_extract_output_from_dir_empty() {
    let test_dir = TestSessionDir::new("empty-dir-test");
    let output =
        extract_output_from_dir(test_dir.path()).expect("空ディレクトリでもエラーにならないべき");
    assert_eq!(output, "", "空ディレクトリの場合は空文字列を返すべき");
}

/// `cleanup_session_dir()` でセッションディレクトリが削除されることを確認。
#[test]
fn test_cleanup_session_dir() {
    let dir = create_session_dir("cleanup-test").expect("作成に成功するべき");
    assert!(dir.exists(), "ディレクトリが存在することを確認");

    cleanup_session_dir(&dir);
    assert!(
        !dir.exists(),
        "クリーンアップ後にディレクトリが存在しないべき"
    );
}

/// `cleanup_session_dir()` が存在しないディレクトリでパニックしないことを確認。
#[test]
fn test_cleanup_session_dir_nonexistent() {
    let nonexistent = Path::new("/tmp/nonexistent_pi_sessions_dir_xyz123");
    // 存在しないディレクトリのクリーンアップはパニックしない
    cleanup_session_dir(nonexistent);
    // 到達したら成功
}

/// `extract_final_output()` が10MBを超えるファイルをスキップすることを確認。
#[test]
fn test_extract_final_output_large_file() {
    let test_dir = TestSessionDir::new("large-file-test");
    let session_path = test_dir.path().join("large.jsonl");

    // 10MBを超えるファイルを作成（実際のヘッダー行 + パディング）
    let header = r#"{"type":"session","version":3,"id":"test","timestamp":"2026-01-01T00:00:00Z","cwd":"/tmp"}"#;
    let padding = "x".repeat(11 * 1024 * 1024); // 11MB のパディング
    let content = format!("{}\n{}", header, padding);
    std::fs::write(&session_path, content).expect("テストファイルの書き込みに失敗");

    let result = extract_final_output(&session_path);
    assert!(result.is_err(), "10MBを超えるファイルはエラーになるべき");
}

/// 実際のサンプルセッションファイルから出力が正しく抽出されることを確認。
#[test]
fn test_extract_from_sample_file() {
    let manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let sample_path = manifest_dir
        .join("tests")
        .join("fixtures")
        .join("sample-session.jsonl");

    assert!(sample_path.exists(), "サンプルファイルが存在するべき");

    let output = extract_final_output(&sample_path).expect("出力抽出に成功するべき");
    // 最後のアシスタントメッセージのテキストが抽出される
    assert!(
        output.contains("テスト分析結果"),
        "サンプルファイルから正しい出力が抽出されるべき: {}",
        output
    );
    assert!(
        output.contains("テストカバレッジ"),
        "出力に期待される内容が含まれるべき"
    );
}
