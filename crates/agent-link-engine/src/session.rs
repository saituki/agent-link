//! # JSONL セッションファイル管理モジュール
//!
//! pi エージェントが出力する JSONL 形式のセッションファイルを作成・読み取り・
//! クリーンアップする機能を提供します。
//!
//! このモジュールは `lib/herdrs/session.ts` の TypeScript 実装と
//! 同等の機能を Rust で提供します。
//!
//! ## JSONL フォーマット
//!
//! セッションファイルは JSONL（JSON Lines）形式で、各行が独立した JSON オブジェクトです。
//! 先頭行はセッションヘッダー、以降はメッセージ行が続きます。
//!
//! ```jsonl
//! {"type":"session","version":3,"id":"<uuid>","timestamp":"<ISO 8601>","cwd":"<current_dir>"}
//! {"type":"message","message":{"role":"user","content":[{"type":"text","text":"prompt"}]}}
//! {"type":"message","message":{"role":"assistant","content":[{"type":"text","text":"回答"}]}}
//! ```

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// セッション実ファイル判定用の UUID 正規表現（遅延初期化・コンパイルは初回 1 回のみ）。
fn uuid_pattern_regex() -> &'static regex::Regex {
    static REGEX: OnceLock<regex::Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        regex::Regex::new(
            r"[a-f0-9]{8}-[a-f0-9]{4}-[a-f0-9]{4}-[a-f0-9]{4}-[a-f0-9]{12}(?:\.jsonl)?$",
        )
        .expect("UUID 正規表現は常にコンパイル可能")
    })
}

/// セッション操作に関するエラー。
#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    /// I/O エラー（ファイル作成・読み取り・削除の失敗）
    #[error("I/O エラー: {0}")]
    IoError(#[from] std::io::Error),

    /// JSON パースエラー（セッションファイルの内容が不正な JSONL）
    #[error("JSON パースエラー: {0}")]
    JsonParseError(String),

    /// セッションファイルが大きすぎる（10MB 超過）
    #[error("セッションファイルが大きすぎます: {size} bytes (上限 10MB)")]
    FileTooLarge { size: u64 },

    /// セッションディレクトリの作成に失敗
    #[error("セッションディレクトリ作成エラー: {0}")]
    CreateDirError(String),

    /// セッションファイルの作成に失敗
    #[error("セッションファイル作成エラー: {0}")]
    CreateFileError(String),
}

/// セッションディレクトリ名のプレフィックス
const SESSION_DIR_PREFIX: &str = "pi-sessions-";

/// 最大ファイルサイズ（10MB）
const MAX_FILE_SIZE: u64 = 10 * 1024 * 1024;

/// 0〜4294967295 の乱数を生成する（getrandom 使用・1.7 の修正）。
///
/// SystemTime ベースの簡易乱数は連続呼び出しで衝突しうるため、
/// 暗号学的乱数（getrandom）に置き換えた。
fn random_u32() -> u32 {
    let mut buf = [0u8; 4];
    getrandom::getrandom(&mut buf).expect("OS の乱数生成に失敗");
    u32::from_le_bytes(buf)
}

/// ユニークなセッションディレクトリを作成します。
///
/// `PROJECTROOT/.agent-link/sessions/pi-sessions-<timestamp>-<random>` 形式の
/// ディレクトリを作成し、そのパスを返します。タイムスタンプと乱数を使用して
/// ユニーク性を確保します。
///
/// # 引数
///
/// * `agent_name` — エージェント名（ディレクトリ名に含める。安全な名前に変換される）
///
/// # 戻り値
///
/// 作成されたセッションディレクトリの `PathBuf`
///
/// # エラー
///
/// * 一時ディレクトリの作成に失敗した場合: `SessionError::CreateDirError`
///
/// # 例
///
/// ```
/// use agent_link_engine::session::create_session_dir;
///
/// let dir = create_session_dir("my-agent").expect("セッションディレクトリの作成に失敗");
/// assert!(dir.exists());
/// assert!(dir.to_string_lossy().contains("pi-sessions-"));
/// // クリーンアップ
/// let _ = std::fs::remove_dir_all(&dir);
/// ```
pub fn create_session_dir(agent_name: &str) -> Result<PathBuf, SessionError> {
    let safe_name = sanitize_agent_name(agent_name);
    let timestamp = timestamp_for_dirname();
    let random_suffix: u32 = random_u32();

    let dirname = format!(
        "{}{}-{}-{}",
        SESSION_DIR_PREFIX, timestamp, safe_name, random_suffix
    );
    // プロジェクトルート（デーモン起動パス）の .agent-link/sessions/ 配下にセッションディレクトリを作成
    // worktree 実行時は cwd が `.worktrees/<branch>` に変わるため、
    // cwd 基準ではなくプロジェクトルート基準に固定する（D15・§7.1.7）。
    let base_dir = crate::project::project_root().join(crate::project::PROJECT_DIR).join("sessions");
    fs::create_dir_all(&base_dir).map_err(|e| {
        SessionError::CreateDirError(format!(
            "セッションベースディレクトリ '{}' の作成に失敗: {}",
            base_dir.display(),
            e
        ))
    })?;
    let dirpath = base_dir.join(&dirname);

    fs::create_dir_all(&dirpath).map_err(|e| {
        SessionError::CreateDirError(format!(
            "ディレクトリ '{}' の作成に失敗: {}",
            dirpath.display(),
            e
        ))
    })?;

    Ok(dirpath)
}

/// セッションディレクトリ内に JSONL セッションファイルを作成します。
///
/// `<YYYY-MM-DDTHH-MM>-<safe_agent_name>.jsonl` 形式のファイルを作成し、
/// 先頭行にセッションヘッダーJSONを書き込みます。
///
/// # 引数
///
/// * `session_dir` — セッションディレクトリのパス（`create_session_dir` で作成されたもの）
/// * `agent_name` — エージェント名（ファイル名に使用。安全な名前に変換される）
///
/// # 戻り値
///
/// 作成されたセッションファイルの `PathBuf`
///
/// # エラー
///
/// * ファイルの作成・書き込みに失敗した場合: `SessionError::CreateFileError`
///
/// # 例
///
/// ```
/// use agent_link_engine::session::{create_session_dir, create_session_file};
///
/// let dir = create_session_dir("my-agent").expect("ディレクトリ作成失敗");
/// let file = create_session_file(&dir, "my-agent").expect("セッションファイル作成失敗");
/// assert!(file.exists());
/// assert!(file.extension().unwrap() == "jsonl");
/// // クリーンアップ
/// let _ = std::fs::remove_dir_all(&dir);
/// ```
pub fn create_session_file(session_dir: &Path, agent_name: &str) -> Result<PathBuf, SessionError> {
    let safe_name = sanitize_agent_name(agent_name);
    // 安全な名前を40文字に切り詰め
    let truncated_name: String = safe_name.chars().take(40).collect();
    let timestamp = timestamp_for_filename();
    let filename = format!("{}-{}.jsonl", timestamp, truncated_name);
    let filepath = session_dir.join(&filename);

    // セッションヘッダー行を生成
    let session_id = generate_session_id();
    let iso_timestamp = iso_8601_timestamp();
    let cwd = std::env::current_dir()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| "unknown".to_string());

    let header = serde_json::json!({
        "type": "session",
        "version": 3,
        "id": session_id,
        "timestamp": iso_timestamp,
        "cwd": cwd,
    });

    let header_line = serde_json::to_string(&header)
        .map_err(|e| SessionError::CreateFileError(format!("JSON シリアライズ失敗: {}", e)))?;

    fs::write(&filepath, header_line + "\n").map_err(|e| {
        SessionError::CreateFileError(format!(
            "ファイル '{}' への書き込みに失敗: {}",
            filepath.display(),
            e
        ))
    })?;

    Ok(filepath)
}

/// JSONL セッションファイルから最後のアシスタントメッセージのテキストを抽出します。
///
/// JSONL ファイルを1行ずつ読み取り、以下の条件でテキストを抽出します:
///
/// 1. 行の JSON をパース
/// 2. `type == "message"` かつ `message.role == "assistant"` の行を探す
/// 3. `message.content` 配列内の `type == "text"` 要素の `text` フィールドを連結
/// 4. 最後に見つかったアシスタントテキストを返す
///
/// 10MB を超えるファイルはスキップします。
///
/// # 引数
///
/// * `session_path` — セッションファイルのパス
///
/// # 戻り値
///
/// 抽出されたテキスト（アシスタントメッセージがない場合は空文字列）
///
/// # エラー
///
/// * ファイル読み込みエラー: `SessionError::IoError`
/// * ファイルが10MB超過: `SessionError::FileTooLarge`
pub fn extract_final_output(session_path: &Path) -> Result<String, SessionError> {
    // ファイルサイズチェック
    let metadata = fs::metadata(session_path)?;
    if metadata.len() > MAX_FILE_SIZE {
        return Err(SessionError::FileTooLarge {
            size: metadata.len(),
        });
    }

    let content = fs::read_to_string(session_path)?;
    let mut last_assistant_text: Option<String> = None;

    for line in content.lines() {
        if line.trim().is_empty() {
            continue;
        }

        // JSON としてパース
        let value: serde_json::Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue, // パースできない行はスキップ
        };

        // type == "message" かつ message.role == "assistant" をチェック
        if value.get("type").and_then(|t| t.as_str()) != Some("message") {
            continue;
        }

        let msg = match value.get("message") {
            Some(m) => m,
            None => continue,
        };

        if msg.get("role").and_then(|r| r.as_str()) != Some("assistant") {
            continue;
        }

        // message.content 配列から text 要素を抽出
        let content_array = match msg.get("content").and_then(|c| c.as_array()) {
            Some(arr) => arr,
            None => continue,
        };

        let mut text_parts: Vec<String> = Vec::new();
        for element in content_array {
            if element.get("type").and_then(|t| t.as_str()) == Some("text") {
                if let Some(text) = element.get("text").and_then(|t| t.as_str()) {
                    text_parts.push(text.to_string());
                }
            }
            // type == "thinking" の要素はスキップ
        }

        if !text_parts.is_empty() {
            last_assistant_text = Some(text_parts.concat());
        }
    }

    Ok(last_assistant_text.unwrap_or_default())
}

/// セッションディレクトリ内の全 JSONL ファイルから完全な会話履歴を抽出し、
/// 人間可読な Markdown 形式で返します。
///
/// フォーマット:
/// ```markdown
/// ## USER
/// {text}
///
/// ## ASSISTANT
/// {text}
///
/// ## TOOL: {name}
/// {result}
///
/// ## THINKING
/// {text}
/// ```
pub fn extract_full_transcript(session_dir: &Path) -> Result<String, SessionError> {
    let mut transcript = String::new();
    let mut filenames: Vec<PathBuf> = Vec::new();
    visit_jsonl_files(session_dir, &mut |filepath| {
        filenames.push(filepath.to_path_buf());
    })?;
    filenames.sort();

    for filepath in &filenames {
        if !transcript.is_empty() {
            transcript.push_str("\n---\n\n");
        }
        let metadata = fs::metadata(filepath)?;
        if metadata.len() > MAX_FILE_SIZE {
            continue;
        }
        let content = fs::read_to_string(filepath)?;
        for line in content.lines() {
            if line.trim().is_empty() {
                continue;
            }
            let value: serde_json::Value = match serde_json::from_str(line) {
                Ok(v) => v,
                Err(_) => continue,
            };
            match value.get("type").and_then(|t| t.as_str()) {
                Some("message") => {
                    if let Some(msg) = value.get("message") {
                        let role = msg.get("role").and_then(|r| r.as_str()).unwrap_or("?");
                        let role_header = match role {
                            "user" => "## USER",
                            "assistant" => "## ASSISTANT",
                            _ => &format!("## {}", role.to_uppercase()),
                        };
                        if let Some(content_arr) = msg.get("content").and_then(|c| c.as_array()) {
                            let mut has_content = false;
                            for element in content_arr {
                                match element.get("type").and_then(|t| t.as_str()) {
                                    Some("text") => {
                                        if let Some(text) =
                                            element.get("text").and_then(|t| t.as_str())
                                        {
                                            if !has_content {
                                                transcript
                                                    .push_str(&format!("\n{}\n", role_header));
                                                has_content = true;
                                            }
                                            transcript.push_str(text);
                                            transcript.push('\n');
                                        }
                                    }
                                    Some("tool_use") => {
                                        let tool_name = element
                                            .get("name")
                                            .and_then(|n| n.as_str())
                                            .unwrap_or("?");
                                        let tool_input = element
                                            .get("input")
                                            .map(|i| i.to_string())
                                            .unwrap_or_default();
                                        transcript.push_str(&format!(
                                            "\n## TOOL_CALL: {}\n```json\n{}\n```\n",
                                            tool_name, tool_input
                                        ));
                                    }
                                    Some("thinking") => {
                                        if let Some(text) =
                                            element.get("thinking").and_then(|t| t.as_str())
                                        {
                                            transcript
                                                .push_str(&format!("\n## THINKING\n{}\n", text));
                                        }
                                    }
                                    _ => {}
                                }
                            }
                        }
                    }
                }
                Some("tool_result") | Some("tool_use") => {
                    let content_str = value
                        .get("content")
                        .map(|c| c.to_string())
                        .unwrap_or_default();
                    transcript.push_str(&format!("\n## TOOL_RESULT\n{}\n", content_str));
                }
                _ => {}
            }
        }
    }

    if transcript.is_empty() {
        Ok("(No session transcript available)".to_string())
    } else {
        Ok(transcript)
    }
}

/// セッションディレクトリ内の全 JSONL から user / assistant の text のみを
/// 順序通りに抽出する（message 画面用）。
///
/// - user: message content の text ブロックを連結した本文
/// - assistant: content 配列の `type: text` のみ（thinking / toolCall 除外）
///
/// 空本文のメッセージは除外する。戻り値は `(role, text)` の時系列リストで、
/// role は `user` / `assistant` のいずれか。
pub fn extract_user_assistant_messages(
    session_dir: &Path,
) -> Result<Vec<(String, String)>, SessionError> {
    let mut files: Vec<PathBuf> = Vec::new();
    visit_jsonl_files(session_dir, &mut |p| files.push(p.to_path_buf()))?;
    files.sort();
    let mut out: Vec<(String, String)> = Vec::new();
    for filepath in &files {
        let metadata = fs::metadata(filepath)?;
        if metadata.len() > MAX_FILE_SIZE {
            continue;
        }
        let content = fs::read_to_string(filepath)?;
        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            let Ok(value) = serde_json::from_str::<serde_json::Value>(trimmed) else {
                continue;
            };
            if value.get("type").and_then(|v| v.as_str()) != Some("message") {
                continue;
            }
            let Some(msg) = value.get("message") else {
                continue;
            };
            let role = msg.get("role").and_then(|r| r.as_str()).unwrap_or("");
            match role {
                "user" => {
                    let text = message_text(msg);
                    if !text.trim().is_empty() {
                        out.push(("user".to_string(), text));
                    }
                }
                "assistant" => {
                    let text = extract_text_from_content(msg);
                    if !text.trim().is_empty() {
                        out.push(("assistant".to_string(), text));
                    }
                }
                _ => {}
            }
        }
    }
    Ok(out)
}

/// message の content（配列 text / 文字列）から本文を取り出す。
fn message_text(msg: &serde_json::Value) -> String {
    match msg.get("content") {
        Some(serde_json::Value::Array(items)) => {
            let mut parts: Vec<String> = Vec::new();
            for item in items {
                if item.get("type").and_then(|t| t.as_str()) == Some("text") {
                    if let Some(t) = item.get("text").and_then(|x| x.as_str()) {
                        parts.push(t.to_string());
                    }
                }
            }
            parts.concat()
        }
        Some(serde_json::Value::String(s)) => s.clone(),
        _ => String::new(),
    }
}

/// セッションディレクトリ直下のメインセッションファイル（`.jsonl`）を取得します。
///
/// サブエージェントのセッションはサブディレクトリに分かれるため、
/// メインセッションはディレクトリ直下の `.jsonl` ファイルとして識別可能です。
/// ディレクトリ直下に複数の `.jsonl` がある場合は、最初に見つかったものを返します。
///
/// # 引数
///
/// * `session_dir` — セッションディレクトリのパス
///
/// # 戻り値
///
/// メインセッションファイルの `PathBuf`
///
/// # エラー
///
/// * ディレクトリが存在しないか、`.jsonl` ファイルが見つからない場合:
///   `SessionError::IoError`
pub fn find_main_session_file(session_dir: &Path) -> Result<PathBuf, SessionError> {
    if !session_dir.is_dir() {
        // ディレクトリでない場合、そのパスが直接 jsonl ファイルであれば採用
        if session_dir.extension().and_then(|e| e.to_str()) == Some("jsonl") && session_dir.exists()
        {
            return Ok(session_dir.to_path_buf());
        }
        return Err(SessionError::IoError(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!(
                "セッションディレクトリが存在しません: {}",
                session_dir.display()
            ),
        )));
    }

    // ディレクトリ直下の .jsonl ファイルをすべて収集する
    // （サブディレクトリ＝サブエージェントのセッションは対象外）
    let mut jsonl_files: Vec<PathBuf> = Vec::new();
    for entry in fs::read_dir(session_dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_file() && path.extension().and_then(|e| e.to_str()) == Some("jsonl") {
            jsonl_files.push(path);
        }
    }

    if jsonl_files.is_empty() {
        return Err(SessionError::IoError(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!(
                "セッションディレクトリ内に .jsonl ファイルがありません: {}",
                session_dir.display()
            ),
        )));
    }

    // セッションディレクトリには「メタデータのみの起動ファイル」と「実セッションファイル」が
    // 混在する。実セッションファイルはファイル名に UUID パターン
    // （例: `_01a0236b-ffbc-7ed6-a0d9-cc5745017525.jsonl`）を含むため、
    // これを優先して採用する。UUID パターンを含む方が実セッション（会話履歴）を担う。
    // （fs::read_dir はファイルシステム依存の順序で返すため、最初の1件を採用すると
    //   メタデータファイルを選び、セッション復元に失敗することがある）
    let uuid_pattern = uuid_pattern_regex();

    for f in &jsonl_files {
        if let Some(name) = f.file_name().and_then(|n| n.to_str()) {
            if uuid_pattern.is_match(name) {
                return Ok(f.clone());
            }
        }
    }

    // UUID パターンが見つからない場合はファイルサイズが大きい方を採用（後方互換）
    jsonl_files.sort_by_key(|f| std::fs::metadata(f).ok().map(|m| m.len()).unwrap_or(0));
    Ok(jsonl_files.into_iter().last().unwrap())
}

/// セッションディレクトリを再帰的に走査し、全 `.jsonl` ファイルから
/// 最後の非空出力を抽出します。
///
/// # 引数
///
/// 実行のセッションディレクトリを解決する（outputs 統一後のフォールバック付き）。
///
/// 1. `outputs/<owner>/<run_id>/session/` を探す（現行正本・owner 移動追従）
/// 2. DB 記録の `session_dir` が存在すればそれを返す（旧 `.agent-link/sessions/` 互換）
/// 3. どちらも無ければ `None`（存在しない記録は切り捨て）
///
/// run フォルダは owner 移動で rename されるため DB 記録が古くなる。
/// 正本を先に見ることで、`m` / `h` / `s` / `p` が移動前の旧 owner パスを
/// 参照して失敗するのを防ぐ。
pub fn resolve_session_dir(recorded: Option<&str>, run_id: i64) -> Option<PathBuf> {
    // outputs 統一後の正本を最優先する。run フォルダは owner 移動
    // （todo→running→review→done→R*）で rename されるため、`find_run_dir()` で
    // 全 owner を走査して現行フォルダを特定する。DB の `session_dir` は
    // 移動前の旧 owner を指したまま古くなるためフォールバック扱いとする。
    if let Some((_, run)) = crate::run_dir::find_run_dir(run_id) {
        let sess = run.join(crate::run_dir::SESSION_SUBDIR);
        if sess.is_dir() {
            return Some(sess);
        }
    }
    // DB 記録は旧 `.agent-link/sessions/...` 互換・未移行分のフォールバック。
    // 存在するパスのみ返す（存在しない記録は切り捨て）。
    if let Some(dir) = recorded.filter(|d| !d.is_empty()) {
        let p = Path::new(dir);
        if p.is_dir() {
            return Some(p.to_path_buf());
        }
    }
    None
}

/// * `session_dir` — セッションディレクトリのパス
///
/// # 戻り値
///
/// 最後に見つかった非空の出力テキスト（見つからない場合は空文字列）
///
/// # エラー
///
/// * ディレクトリの走査に失敗した場合: `SessionError::IoError`
pub fn extract_output_from_dir(session_dir: &Path) -> Result<String, SessionError> {
    let mut last_output: Option<String> = None;

    visit_jsonl_files(session_dir, &mut |filepath| {
        if let Ok(output) = extract_final_output(filepath) {
            if !output.is_empty() {
                last_output = Some(output);
            }
        }
    })?;

    Ok(last_output.unwrap_or_default())
}

/// セッションディレクトリを削除します（ベストエフォート）。
///
/// 削除に失敗してもエラーは無視され、サイレントに無視されます。
///
/// # 引数
///
/// * `session_dir` — 削除するセッションディレクトリのパス
///
/// # 例
///
/// ```
/// use agent_link_engine::session::{create_session_dir, cleanup_session_dir};
///
/// let dir = create_session_dir("test").expect("ディレクトリ作成失敗");
/// assert!(dir.exists());
/// cleanup_session_dir(&dir);
/// assert!(!dir.exists());
/// ```
pub fn cleanup_session_dir(session_dir: &Path) {
    if session_dir.exists() {
        let _ = fs::remove_dir_all(session_dir);
    }
}

// ============================================================================
// セッション統計抽出
// ============================================================================

/// JSONL セッションファイルから抽出された統計情報。
///
/// トークン数は JSONL に明示的なトークンカウントが含まれないため、
/// テキスト長（文字数）を近似値として使用します。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SessionStats {
    /// セッション開始時刻（ISO 8601）
    pub started_at: Option<String>,
    /// セッション終了時刻（ISO 8601）
    pub finished_at: Option<String>,
    /// 経過時間（ミリ秒）
    pub elapsed_ms: Option<u64>,
    /// 入力トークン数（近似: ユーザーメッセージの文字数）
    pub input_tokens: u64,
    /// 出力トークン数（近似: アシスタントのテキストブロック文字数）
    pub output_tokens: u64,
    /// thinking トークン数（近似: thinking ブロックの文字数）
    pub thinking_tokens: u64,
    /// 総トークン数（input + output + thinking）
    pub total_tokens: u64,
    /// ツール呼び出し回数
    pub tool_call_count: u64,
    /// ターン数（user/assistant の往復数）
    pub turn_count: u64,
    /// 使用モデル名（セッションヘッダーから取得、ない場合は None）
    pub model_name: Option<String>,
    /// セッションの合計コスト（USD・usage.cost.total の合計、無い場合は 0）
    #[serde(default)]
    pub cost_usd: f64,
    /// ツール別呼び出し回数
    pub tool_calls_by_name: HashMap<String, u64>,
    /// tool / message / think カテゴリ別のトークン使用量（R1）。
    ///
    /// assistant メッセージに `usage` がある場合は実トークン
    /// （input / output / cache = cacheRead + cacheWrite）を使用し、
    /// `usage` が無い場合は文字数近似（cache は 0）で補完する。
    pub token_usage_by_category: TokenUsageByCategory,
}

/// 特定カテゴリ（tool / message / think）のトークン使用量（R1）。
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct TokenCategoryStats {
    /// 入力トークン数
    pub input: u64,
    /// 出力トークン数
    pub output: u64,
    /// キャッシュトークン数（cacheRead + cacheWrite）
    pub cache: u64,
}

/// tool / message / think カテゴリ別のトークン使用量（R1）。
///
/// カテゴリは assistant メッセージのコンテンツブロック種別から判定する:
/// - `tool`: toolCall / tool_result / toolResult を含む turn
/// - `message`: 通常のテキスト応答
/// - `think`: thinking ブロックを含む turn
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct TokenUsageByCategory {
    /// tool（ツール呼び出し・結果）
    pub tool: TokenCategoryStats,
    /// message（通常メッセージ）
    pub message: TokenCategoryStats,
    /// think（thinking ブロック）
    pub think: TokenCategoryStats,
}

impl SessionStats {
    /// 空の統計情報を作成する。
    pub fn empty() -> Self {
        Self {
            started_at: None,
            finished_at: None,
            elapsed_ms: None,
            input_tokens: 0,
            output_tokens: 0,
            thinking_tokens: 0,
            total_tokens: 0,
            tool_call_count: 0,
            turn_count: 0,
            model_name: None,
            cost_usd: 0.0,
            tool_calls_by_name: HashMap::new(),
            token_usage_by_category: TokenUsageByCategory::default(),
        }
    }
}

/// セッションディレクトリ内の全 `.jsonl` ファイルからセッション統計を抽出します。
///
/// ディレクトリを再帰的に走査し、見つかった全 JSONL ファイルの内容を集計します。
/// 複数の JSONL ファイル（サブエージェントセッションなど）がある場合、
/// それらの統計は合算されます。
///
/// # 引数
///
/// * `session_dir` — セッションディレクトリのパス
///
/// # 戻り値
///
/// 集計された `SessionStats`
///
/// # エラー
///
/// * ファイル読み込みエラー: `SessionError::IoError`
pub fn extract_session_stats(session_dir: &Path) -> Result<SessionStats, SessionError> {
    let mut stats = SessionStats::empty();
    let mut earliest_timestamp: Option<String> = None;
    let mut latest_timestamp: Option<String> = None;

    visit_jsonl_files(session_dir, &mut |filepath| {
        if let Ok(file_stats) = extract_single_file_stats(filepath) {
            // 時刻の統合（最も早い開始時刻と最も遅い終了時刻）
            if let Some(ref ts) = file_stats.started_at {
                if earliest_timestamp.is_none()
                    || ts.as_str() < earliest_timestamp.as_deref().unwrap_or("")
                {
                    earliest_timestamp = Some(ts.clone());
                }
            }
            if let Some(ref ts) = file_stats.finished_at {
                if latest_timestamp.is_none()
                    || ts.as_str() > latest_timestamp.as_deref().unwrap_or("")
                {
                    latest_timestamp = Some(ts.clone());
                }
            }

            // 数値の合算
            stats.input_tokens += file_stats.input_tokens;
            stats.output_tokens += file_stats.output_tokens;
            stats.thinking_tokens += file_stats.thinking_tokens;
            stats.total_tokens += file_stats.total_tokens;
            stats.tool_call_count += file_stats.tool_call_count;
            stats.turn_count += file_stats.turn_count;

            // ツール別呼び出し回数のマージ
            for (name, count) in &file_stats.tool_calls_by_name {
                *stats.tool_calls_by_name.entry(name.clone()).or_insert(0) += count;
            }

            // カテゴリ別トークン使用量の合算（R1）
            stats.token_usage_by_category.tool.input +=
                file_stats.token_usage_by_category.tool.input;
            stats.token_usage_by_category.tool.output +=
                file_stats.token_usage_by_category.tool.output;
            stats.token_usage_by_category.tool.cache +=
                file_stats.token_usage_by_category.tool.cache;
            stats.token_usage_by_category.message.input +=
                file_stats.token_usage_by_category.message.input;
            stats.token_usage_by_category.message.output +=
                file_stats.token_usage_by_category.message.output;
            stats.token_usage_by_category.message.cache +=
                file_stats.token_usage_by_category.message.cache;
            stats.token_usage_by_category.think.input +=
                file_stats.token_usage_by_category.think.input;
            stats.token_usage_by_category.think.output +=
                file_stats.token_usage_by_category.think.output;
            stats.token_usage_by_category.think.cache +=
                file_stats.token_usage_by_category.think.cache;

            // モデル名（最初に見つかったものを採用）
            if stats.model_name.is_none() {
                stats.model_name = file_stats.model_name;
            }
        }
    })?;

    stats.started_at = earliest_timestamp;
    stats.finished_at = latest_timestamp;

    // 経過時間を計算
    stats.elapsed_ms =
        calculate_elapsed_ms(stats.started_at.as_deref(), stats.finished_at.as_deref());

    // total_tokens を再計算（合算後の正確な値を保証）
    stats.total_tokens = stats.input_tokens + stats.output_tokens + stats.thinking_tokens;

    Ok(stats)
}

/// 単一の JSONL ファイルからセッション統計を抽出する内部関数。
fn extract_single_file_stats(filepath: &Path) -> Result<SessionStats, SessionError> {
    let metadata = fs::metadata(filepath)?;
    if metadata.len() > MAX_FILE_SIZE {
        return Err(SessionError::FileTooLarge {
            size: metadata.len(),
        });
    }

    let content = fs::read_to_string(filepath)?;
    let mut stats = SessionStats::empty();
    let mut tool_calls_by_name: HashMap<String, u64> = HashMap::new();
    let mut user_message_count: u64 = 0;
    // カテゴリ別トークン使用量（R1）
    let mut breakdown = TokenUsageByCategory::default();

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let value: serde_json::Value = match serde_json::from_str(trimmed) {
            Ok(v) => v,
            Err(_) => continue,
        };

        match value.get("type").and_then(|t| t.as_str()) {
            Some("session") => {
                // セッションヘッダーから開始時刻とモデル名を取得
                if stats.started_at.is_none() {
                    stats.started_at = value
                        .get("timestamp")
                        .and_then(|t| t.as_str())
                        .map(String::from);
                }
                if stats.model_name.is_none() {
                    stats.model_name = value
                        .get("model")
                        .and_then(|m| m.as_str())
                        .map(String::from);
                }
            }
            Some("message") => {
                let msg = match value.get("message") {
                    Some(m) => m,
                    None => continue,
                };

                let role = msg.get("role").and_then(|r| r.as_str()).unwrap_or("");
                let content_array = match msg.get("content").and_then(|c| c.as_array()) {
                    Some(arr) => arr,
                    None => continue,
                };

                // 終了時刻を更新（最後のメッセージのタイムスタンプ）
                if let Some(ts) = value.get("timestamp").and_then(|t| t.as_str()) {
                    stats.finished_at = Some(ts.to_string());
                }

                match role {
                    "user" => {
                        user_message_count += 1;
                        for element in content_array {
                            let block_type =
                                element.get("type").and_then(|t| t.as_str()).unwrap_or("");
                            match block_type {
                                "text" => {
                                    if let Some(text) = element.get("text").and_then(|t| t.as_str())
                                    {
                                        let chars = text.chars().count() as u64;
                                        stats.input_tokens += chars;
                                        // 通常テキスト入力 → message カテゴリ
                                        breakdown.message.input += chars;
                                    }
                                }
                                "tool_result" => {
                                    // ツール結果のテキストも入力としてカウント
                                    if let Some(text) = element.get("text").and_then(|t| t.as_str())
                                    {
                                        let chars = text.chars().count() as u64;
                                        stats.input_tokens += chars;
                                        // ツール結果 → tool カテゴリ
                                        breakdown.tool.input += chars;
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                    "assistant" => {
                        // usage（cacheRead / cacheWrite 含む）が存在するかを判定（R3）
                        let usage = msg.get("usage");
                        let usage_ok = usage.is_some()
                            && usage
                                .and_then(|u| u.get("input"))
                                .and_then(|v| v.as_u64())
                                .is_some()
                            && usage
                                .and_then(|u| u.get("output"))
                                .and_then(|v| v.as_u64())
                                .is_some();

                        // コンテンツブロック種別からカテゴリを判定
                        let mut has_tool_call = false;
                        let mut has_thinking = false;
                        for element in content_array {
                            match element.get("type").and_then(|t| t.as_str()) {
                                Some("toolCall") => has_tool_call = true,
                                Some("thinking") => has_thinking = true,
                                _ => {}
                            }
                        }

                        if usage_ok {
                            let u = usage.unwrap();
                            let in_t = u.get("input").and_then(|v| v.as_u64()).unwrap_or(0);
                            let out_t = u.get("output").and_then(|v| v.as_u64()).unwrap_or(0);
                            let cache_read =
                                u.get("cacheRead").and_then(|v| v.as_u64()).unwrap_or(0);
                            let cache_write =
                                u.get("cacheWrite").and_then(|v| v.as_u64()).unwrap_or(0);
                            // cache = cacheRead + cacheWrite（R3）
                            let cat_cache = cache_read + cache_write;
                            // コスト（usage.cost.total の合計。無い場合は 0）
                            if let Some(cost_total) = u
                                .get("cost")
                                .and_then(|c| c.get("total"))
                                .and_then(|v| v.as_f64())
                            {
                                stats.cost_usd += cost_total;
                            }
                            // 主カテゴリ判定（tool > think > message）
                            let cat: &mut TokenCategoryStats = if has_tool_call {
                                &mut breakdown.tool
                            } else if has_thinking {
                                &mut breakdown.think
                            } else {
                                &mut breakdown.message
                            };
                            cat.input += in_t;
                            cat.output += out_t;
                            cat.cache += cat_cache;
                        }

                        // 既存の文字数近似（トップレベル集計。R2 回帰なし）
                        for element in content_array {
                            let block_type =
                                element.get("type").and_then(|t| t.as_str()).unwrap_or("");
                            match block_type {
                                "text" => {
                                    if let Some(text) = element.get("text").and_then(|t| t.as_str())
                                    {
                                        let chars = text.chars().count() as u64;
                                        stats.output_tokens += chars;
                                        // usage 無し行はカテゴリを文字数近似で補完（cache=0）
                                        if !usage_ok {
                                            breakdown.message.output += chars;
                                        }
                                    }
                                }
                                "thinking" => {
                                    if let Some(text) =
                                        element.get("thinking").and_then(|t| t.as_str())
                                    {
                                        let chars = text.chars().count() as u64;
                                        stats.thinking_tokens += chars;
                                        if !usage_ok {
                                            breakdown.think.output += chars;
                                        }
                                    }
                                }
                                "toolCall" => {
                                    stats.tool_call_count += 1;
                                    if let Some(name) = element.get("name").and_then(|n| n.as_str())
                                    {
                                        *tool_calls_by_name.entry(name.to_string()).or_insert(0) +=
                                            1;
                                    }
                                    if !usage_ok {
                                        // ツール呼び出し分の近似（arguments の文字数）
                                        if let Some(args) = element.get("arguments") {
                                            breakdown.tool.output +=
                                                args.to_string().chars().count() as u64;
                                        }
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                    "toolResult" => {
                        // toolResult のテキストを入力トークンとしてカウント
                        for element in content_array {
                            let block_type =
                                element.get("type").and_then(|t| t.as_str()).unwrap_or("");
                            if block_type == "text" {
                                if let Some(text) = element.get("text").and_then(|t| t.as_str()) {
                                    let chars = text.chars().count() as u64;
                                    stats.input_tokens += chars;
                                    // ツール結果 → tool カテゴリ
                                    breakdown.tool.input += chars;
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }

    stats.turn_count = user_message_count;
    stats.total_tokens = stats.input_tokens + stats.output_tokens + stats.thinking_tokens;
    stats.tool_calls_by_name = tool_calls_by_name;
    stats.token_usage_by_category = breakdown;

    Ok(stats)
}

// ============================================================================
// 実行 usage 抽出（L1: provider usage の合計 + 種別 × ターン別内訳）
// ============================================================================

/// provider が返す 1 件分の usage 量（token 4 種 + コスト）。
#[derive(Debug, Clone, Copy, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct UsageAmount {
    /// 入力トークン
    pub input: u64,
    /// 出力トークン
    pub output: u64,
    /// キャッシュ読み込みトークン
    pub cache_read: u64,
    /// キャッシュ書き込みトークン
    pub cache_write: u64,
    /// コスト（USD・`usage.cost.total`）
    pub cost_total: f64,
}

impl UsageAmount {
    /// 加算する。
    pub fn add(&mut self, other: &UsageAmount) {
        self.input += other.input;
        self.output += other.output;
        self.cache_read += other.cache_read;
        self.cache_write += other.cache_write;
        self.cost_total += other.cost_total;
    }

    /// cacheRead + cacheWrite。
    pub fn cache_total(&self) -> u64 {
        self.cache_read + self.cache_write
    }
}

/// 種別 × ターン別の usage 内訳 1 行（推定値）。
#[derive(Debug, Clone, PartialEq)]
pub struct UsageBreakdownRow {
    /// 種別（`message` / `thinking` / `toolCall` / `toolResult`）
    pub entry_type: String,
    /// ターン番号（1 始まり。assistant 応答の通し番号）
    pub turn_no: u64,
    /// 割り当てられた usage 量
    pub amount: UsageAmount,
}

/// セッション 1 実行分の usage 抽出結果。
#[derive(Debug, Clone, Default)]
pub struct ExecutionUsageScan {
    /// provider usage の実行合計（コスト計算の正）
    pub total: UsageAmount,
    /// 種別 × ターン別の内訳（推定）。各メトリックの合計は [`Self::total`] と一致する
    pub breakdown: Vec<UsageBreakdownRow>,
}

/// セッションディレクトリの jsonl を **1 回** 走査し、
/// 「(A) provider usage の合計」と「(B) 種別 × ターン別トークン内訳」を同時に算出する（L1）。
///
/// ## 集計方針
///
/// - `total`: assistant メッセージが持つ `usage`（input / output / cacheRead /
///   cacheWrite / cost.total）を全ファイル分合算した生の値（**正**）
/// - 内訳: provider は assistant 応答単位でしか usage を報告しないため、
///   1 応答内のエントリサイズ（文字数）比で種別に**推定振り分け**する:
///   - `message` = text ブロック / `thinking` = thinking ブロック /
///     `toolCall` = toolCall の名前 + 引数 / `toolResult` = 直前の toolResult テキスト
///   - すべてのメトリック（token 4 種・コスト）を同じ比率で配分し、
///     端数は大きい側に丸める（largest remainder）。したがって**内訳の合計 == total**
///   - サイズ 0 の種別は行を生成しない。すべて 0 の場合は `message` に全額計上
/// - ターン番号（`turn_no`）は assistant 応答の通し番号（1 始まり。ファイル名順）
/// - パース不能な行はスキップする（10MB 超のファイルもスキップ）
///
/// # エラー
///
/// ディレクトリ走査の I/O エラー時は `SessionError::IoError`。
pub fn extract_execution_usage(session_dir: &Path) -> Result<ExecutionUsageScan, SessionError> {
    let mut files: Vec<PathBuf> = Vec::new();
    visit_jsonl_files(session_dir, &mut |p| files.push(p.to_path_buf()))?;
    files.sort();

    let mut scan = ExecutionUsageScan::default();
    let mut turn_no: u64 = 0;
    // 直前の assistant 応答以降に蓄積した toolResult テキストの文字数
    let mut pending_tool_result_chars: u64 = 0;

    for filepath in &files {
        let metadata = fs::metadata(filepath)?;
        if metadata.len() > MAX_FILE_SIZE {
            continue;
        }
        let content = fs::read_to_string(filepath)?;
        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            let Ok(value) = serde_json::from_str::<serde_json::Value>(trimmed) else {
                continue;
            };
            if value.get("type").and_then(|t| t.as_str()) != Some("message") {
                continue;
            }
            let Some(msg) = value.get("message") else {
                continue;
            };
            let role = msg.get("role").and_then(|r| r.as_str()).unwrap_or("");

            if role == "toolResult" {
                // 次の assistant 応答の input に含まれるため、直近分として蓄積する
                let chars = content_text_chars(msg.get("content"));
                pending_tool_result_chars += chars;
                continue;
            }
            if role != "assistant" {
                continue;
            }

            // usage が無い応答は集計対象外（近似値側で処理される）
            let Some(amount) = parse_assistant_usage(msg) else {
                // usage 無しでも toolResult 蓄積は保持する（次の usage 付き応答へ振る）
                continue;
            };

            turn_no += 1;
            // 種別ごとの文字数（配分の重み）
            let weights: Vec<(&str, u64)> = {
                let arr = msg.get("content").and_then(|c| c.as_array());
                let mut w = Vec::with_capacity(4);
                let mut text_chars = 0u64;
                let mut think_chars = 0u64;
                let mut call_chars = 0u64;
                if let Some(items) = arr {
                    for item in items {
                        match item.get("type").and_then(|t| t.as_str()) {
                            Some("text") => {
                                text_chars += item
                                    .get("text")
                                    .and_then(|t| t.as_str())
                                    .map(|t| t.chars().count() as u64)
                                    .unwrap_or(0);
                            }
                            Some("thinking") => {
                                think_chars += item
                                    .get("thinking")
                                    .and_then(|t| t.as_str())
                                    .map(|t| t.chars().count() as u64)
                                    .unwrap_or(0);
                            }
                            Some("toolCall") => {
                                let name = item.get("name").and_then(|n| n.as_str()).unwrap_or("");
                                let args = item
                                    .get("arguments")
                                    .map(|a| a.to_string())
                                    .unwrap_or_default();
                                call_chars += (name.chars().count() + args.chars().count()) as u64;
                            }
                            _ => {}
                        }
                    }
                }
                if text_chars > 0 {
                    w.push((crate::db::models::USAGE_ENTRY_MESSAGE, text_chars));
                }
                if think_chars > 0 {
                    w.push((crate::db::models::USAGE_ENTRY_THINKING, think_chars));
                }
                if call_chars > 0 {
                    w.push((crate::db::models::USAGE_ENTRY_TOOL_CALL, call_chars));
                }
                if pending_tool_result_chars > 0 {
                    w.push((
                        crate::db::models::USAGE_ENTRY_TOOL_RESULT,
                        pending_tool_result_chars,
                    ));
                }
                if w.is_empty() {
                    // サイズ 0（空応答など）は message に全額計上
                    w.push((crate::db::models::USAGE_ENTRY_MESSAGE, 1));
                }
                w
            };
            pending_tool_result_chars = 0;

            scan.total.add(&amount);
            // token 4 種 + コストを同じ重みで配分する
            let shares = apportion_amount(&amount, &weights);
            for (entry_type, part) in shares {
                scan.breakdown.push(UsageBreakdownRow {
                    entry_type: entry_type.to_string(),
                    turn_no,
                    amount: part,
                });
            }
        }
    }

    Ok(scan)
}

/// assistant メッセージから provider usage を読み取る。
///
/// `usage.input` / `usage.output` が数値でない場合は `None`（usage 無し扱い）。
fn parse_assistant_usage(msg: &serde_json::Value) -> Option<UsageAmount> {
    let usage = msg.get("usage")?;
    let input = usage.get("input").and_then(|v| v.as_u64())?;
    let output = usage.get("output").and_then(|v| v.as_u64())?;
    Some(UsageAmount {
        input,
        output,
        cache_read: usage.get("cacheRead").and_then(|v| v.as_u64()).unwrap_or(0),
        cache_write: usage
            .get("cacheWrite")
            .and_then(|v| v.as_u64())
            .unwrap_or(0),
        cost_total: usage
            .get("cost")
            .and_then(|c| c.get("total"))
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0),
    })
}

/// toolResult / user メッセージの content（配列 or 文字列）から text の文字数合計を返す。
fn content_text_chars(content: Option<&serde_json::Value>) -> u64 {
    match content {
        Some(serde_json::Value::Array(items)) => items
            .iter()
            .filter(|i| i.get("type").and_then(|t| t.as_str()) == Some("text"))
            .filter_map(|i| i.get("text").and_then(|t| t.as_str()))
            .map(|t| t.chars().count() as u64)
            .sum(),
        Some(serde_json::Value::String(s)) => s.chars().count() as u64,
        _ => 0,
    }
}

/// `amount` を重み比で種別へ配分する（largest remainder 方式・合計保存）。
///
/// token は整数、コストは f64 で同じ手順を適用する。配分後の各メトリックの
/// 合計は入力と一致することを保証する。
fn apportion_amount(
    amount: &UsageAmount,
    weights: &[(&'static str, u64)],
) -> Vec<(&'static str, UsageAmount)> {
    let total_w: u64 = weights.iter().map(|(_, w)| *w).sum();
    let entries: Vec<(&'static str, u64)> = weights.iter().map(|(t, w)| (*t, *w)).collect();

    let split_u64 = |total: u64| -> Vec<u64> {
        if total == 0 || total_w == 0 {
            return vec![0; entries.len()];
        }
        let exact: Vec<f64> = entries
            .iter()
            .map(|(_, w)| total as f64 * (*w as f64) / total_w as f64)
            .collect();
        let mut parts: Vec<u64> = exact.iter().map(|v| v.floor() as u64).collect();
        // 端数の大きい順に 1 を配って合計を合わせる
        let assigned: u64 = parts.iter().sum();
        let mut rest = total - assigned;
        let mut order: Vec<usize> = (0..entries.len()).collect();
        order.sort_by(|&a, &b| {
            let fa = exact[a] - exact[a].floor();
            let fb = exact[b] - exact[b].floor();
            fb.partial_cmp(&fa).unwrap_or(std::cmp::Ordering::Equal)
        });
        let mut idx = 0;
        while rest > 0 && !order.is_empty() {
            parts[order[idx % order.len()]] += 1;
            rest -= 1;
            idx += 1;
        }
        parts
    };

    let inputs = split_u64(amount.input);
    let outputs = split_u64(amount.output);
    let cache_reads = split_u64(amount.cache_read);
    let cache_writes = split_u64(amount.cache_write);
    // コスト: f64 のまま比率配分し、残余は最大シェアへ加算（誤差吸収）
    let total_cost = amount.cost_total;
    let mut costs: Vec<f64> = entries
        .iter()
        .map(|(_, w)| {
            if total_w > 0 {
                total_cost * (*w as f64) / total_w as f64
            } else {
                0.0
            }
        })
        .collect();
    if total_cost != 0.0 && !costs.is_empty() {
        let sum: f64 = costs.iter().sum();
        let max_idx = costs
            .iter()
            .enumerate()
            .max_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
            .map(|(i, _)| i)
            .unwrap_or(0);
        costs[max_idx] += total_cost - sum;
    }

    entries
        .iter()
        .enumerate()
        .map(|(i, (t, _))| {
            (
                *t,
                UsageAmount {
                    input: inputs[i],
                    output: outputs[i],
                    cache_read: cache_reads[i],
                    cache_write: cache_writes[i],
                    cost_total: costs[i],
                },
            )
        })
        .collect()
}

/// 時系列イベントレコード（R5）。
///
/// セッション JSONL から抽出した toolCall / ユーザーメッセージ / git 相当イベントを
/// 時刻順のレコードとして表現する。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TimelineEvent {
    /// イベントの ISO 8601 タイムスタンプ（不明な場合は空文字）
    pub timestamp: String,
    /// イベント種別（"user" / "toolCall" / "git"）
    pub kind: String,
    /// 詳細（ユーザーテキスト・ツール名・git コマンド等）
    pub detail: String,
}

/// セッションディレクトリ内の全 JSONL から時系列イベントを抽出し、時刻順に返します（R5）。
///
/// 抽出対象:
/// - ユーザーメッセージ（role == "user" のテキスト）
/// - ツール呼び出し（assistant の toolCall ブロック）
/// - git 相当イベント（toolCall の名前が `git`、または引数に git コマンドを含むもの）
///
/// パース不能な JSONL 行は無視されます。タイムスタンプ順（文字列昇順）にソートして返します。
///
/// # 引数
///
/// * `session_dir` — セッションディレクトリのパス
///
/// # 戻り値
///
/// 時刻順にソートされた `TimelineEvent` の配列
///
/// # エラー
///
/// * ファイル読み込みエラー: `SessionError::IoError`
pub fn extract_timeline_events(session_dir: &Path) -> Result<Vec<TimelineEvent>, SessionError> {
    let mut events: Vec<TimelineEvent> = Vec::new();
    let mut filenames: Vec<PathBuf> = Vec::new();
    visit_jsonl_files(session_dir, &mut |filepath| {
        filenames.push(filepath.to_path_buf());
    })?;
    filenames.sort();

    for filepath in &filenames {
        let metadata = fs::metadata(filepath)?;
        if metadata.len() > MAX_FILE_SIZE {
            continue;
        }
        let content = fs::read_to_string(filepath)?;
        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            // パース不能な行は無視（R5）
            let value: serde_json::Value = match serde_json::from_str(trimmed) {
                Ok(v) => v,
                Err(_) => continue,
            };
            let ts = value
                .get("timestamp")
                .and_then(|t| t.as_str())
                .map(String::from)
                .unwrap_or_default();

            match value.get("type").and_then(|t| t.as_str()) {
                Some("message") => {
                    let msg = match value.get("message") {
                        Some(m) => m,
                        None => continue,
                    };
                    let role = msg.get("role").and_then(|r| r.as_str()).unwrap_or("");
                    match role {
                        "user" => {
                            events.push(TimelineEvent {
                                timestamp: ts,
                                kind: "user".to_string(),
                                detail: extract_text_from_content(&msg),
                            });
                        }
                        "assistant" => {
                            if let Some(arr) = msg.get("content").and_then(|c| c.as_array()) {
                                for element in arr {
                                    if element.get("type").and_then(|t| t.as_str())
                                        == Some("toolCall")
                                    {
                                        let name = element
                                            .get("name")
                                            .and_then(|n| n.as_str())
                                            .unwrap_or("");
                                        let args = element
                                            .get("arguments")
                                            .map(|a| a.to_string())
                                            .unwrap_or_default();
                                        let detail =
                                            format!("{} {}", name, args).trim().to_string();
                                        // git 相当イベントの判定（R5）
                                        let is_git = name == "git" || args.contains("git");
                                        events.push(TimelineEvent {
                                            timestamp: ts.clone(),
                                            kind: if is_git {
                                                "git".to_string()
                                            } else {
                                                "toolCall".to_string()
                                            },
                                            detail,
                                        });
                                    }
                                }
                            }
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
        }
    }

    // 時刻順（ISO 8601 文字列の昇順）にソート
    events.sort_by(|a, b| a.timestamp.cmp(&b.timestamp));
    Ok(events)
}

// ============================================================================
// History 画面用の近似統計（文字数換算・近似値）
// ============================================================================

/// 種別ごとの近似トークン内訳（History 画面専用・文字数換算の近似値）。
///
/// provider の実際の usage ではない。実際の値は `execution_usage` テーブル
/// （[`extract_execution_usage`] の結果）を参照すること。
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct HistoryTypeTokens {
    /// user message の text
    pub user: u64,
    /// assistant message の text
    pub agent: u64,
    /// toolCall（名前 + 引数）
    pub tool_call: u64,
    /// toolResult の text
    pub tool_result: u64,
    /// thinking
    pub thinking: u64,
}

/// History 画面 Timeline の 1 イベント（近似値付き）。
///
/// `toolCall` の行は対になる結果（`toolResult`）を同じ行に持つ（C11）。
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct HistoryEvent {
    /// イベントの ISO 8601 タイムスタンプ（不明な場合は空文字）
    pub timestamp: String,
    /// 種別: `user` / `agent` / `toolCall` / `toolResult` / `thinking`
    pub kind: String,
    /// 名前（toolCall / toolResult = ツール名、user / agent = 本文冒頭、thinking = "thinking"）
    pub name: String,
    /// 近似トークン数（エントリの文字数換算）
    pub tokens: u64,
    /// 呼び出しの番号（cc は `tool_use.id`、pi は `toolCall.id`）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub call_id: Option<String>,
    /// Bash 系の道具の命令（`agent-link doc read` など）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    /// 結果の本文の先頭 200 字
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preview: Option<String>,
    /// 結果が失敗か（`is_error`）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_error: Option<bool>,
    /// 結果の文字数による近似
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result_tokens: Option<u64>,
    /// 失敗の分類（`is_error` が true のときだけ。JSON には出さない）
    #[serde(skip)]
    pub failure: Option<&'static str>,
    /// 呼び出しの語に `--help`・`-h` があるか（U04。JSON には出さない）
    #[serde(skip)]
    pub help: bool,
    /// 入力に `agent-link*` の語があるか（Bash のみ・命令の位置でなくても。U04。JSON には出さない）
    #[serde(skip)]
    pub cli_mention: bool,
}

/// History 画面用のセッション統計（近似値）。
#[derive(Debug, Clone, Default)]
pub struct HistoryStats {
    /// 種別ごとの近似トークン合計
    pub type_tokens: HistoryTypeTokens,
    /// Timeline イベント（時刻順）
    pub events: Vec<HistoryEvent>,
}

/// セッションディレクトリの jsonl を走査し、History 画面用の
/// 近似統計（種別別トークン + Timeline イベント）を抽出する。
///
/// トークン数は文字数換算の**近似値**（History 画面のみで使用する。
/// provider の実際の usage は [`extract_execution_usage`] を使用する）。
///
/// # 種別と集計対象
///
/// | 種別 | 対象 |
/// |---|---|
/// | `user` | user message の text ブロック |
/// | `agent` | assistant message の text ブロック |
/// | `toolCall` | assistant の toolCall（ツール名 + 引数） |
/// | `toolResult` | toolResult の text ブロック（対になる toolCall の行に足す・C11） |
/// | `thinking` | assistant の thinking ブロック |
///
/// # エラー
///
/// ディレクトリ走査の I/O エラー時は `SessionError::IoError`。
pub fn extract_history_stats(session_dir: &Path) -> Result<HistoryStats, SessionError> {
    let mut files: Vec<PathBuf> = Vec::new();
    visit_jsonl_files(session_dir, &mut |p| files.push(p.to_path_buf()))?;
    files.sort();

    let mut stats = HistoryStats::default();
    let t = &mut stats.type_tokens;
    // toolCall の id → events の位置（対になる toolResult は同じ行に足す・C11）
    let mut calls: HashMap<String, usize> = HashMap::new();

    for filepath in &files {
        let metadata = fs::metadata(filepath)?;
        if metadata.len() > MAX_FILE_SIZE {
            continue;
        }
        let content = fs::read_to_string(filepath)?;
        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            let Ok(value) = serde_json::from_str::<serde_json::Value>(trimmed) else {
                continue;
            };
            if value.get("type").and_then(|v| v.as_str()) != Some("message") {
                continue;
            }
            let Some(msg) = value.get("message") else {
                continue;
            };
            let ts = value
                .get("timestamp")
                .and_then(|t| t.as_str())
                .unwrap_or("")
                .to_string();
            let role = msg.get("role").and_then(|r| r.as_str()).unwrap_or("");
            match role {
                "user" => {
                    let chars = content_text_chars(msg.get("content"));
                    if chars == 0 {
                        continue;
                    }
                    t.user += chars;
                    stats.events.push(HistoryEvent {
                        timestamp: ts,
                        kind: "user".to_string(),
                        name: first_line_head(msg.get("content"), 24),
                        tokens: chars,
                        ..Default::default()
                    });
                }
                "assistant" => {
                    let Some(items) = msg.get("content").and_then(|c| c.as_array()) else {
                        continue;
                    };
                    for item in items {
                        match item.get("type").and_then(|k| k.as_str()) {
                            Some("text") => {
                                let Some(text) = item.get("text").and_then(|x| x.as_str()) else {
                                    continue;
                                };
                                let chars = text.chars().count() as u64;
                                if chars == 0 {
                                    continue;
                                }
                                t.agent += chars;
                                stats.events.push(HistoryEvent {
                                    timestamp: ts.clone(),
                                    kind: "agent".to_string(),
                                    name: head_text(text, 24),
                                    tokens: chars,
                                    ..Default::default()
                                });
                            }
                            Some("thinking") => {
                                let chars = item
                                    .get("thinking")
                                    .and_then(|x| x.as_str())
                                    .map(|x| x.chars().count() as u64)
                                    .unwrap_or(0);
                                if chars == 0 {
                                    continue;
                                }
                                t.thinking += chars;
                                stats.events.push(HistoryEvent {
                                    timestamp: ts.clone(),
                                    kind: "thinking".to_string(),
                                    name: "thinking".to_string(),
                                    tokens: chars,
                                    ..Default::default()
                                });
                            }
                            Some("toolCall") => {
                                let name = item.get("name").and_then(|n| n.as_str()).unwrap_or("");
                                let args = item
                                    .get("arguments")
                                    .map(|a| a.to_string())
                                    .unwrap_or_default();
                                let chars = (name.chars().count() + args.chars().count()) as u64;
                                if chars == 0 {
                                    continue;
                                }
                                let call_id = item.get("id").and_then(|x| x.as_str()).unwrap_or("");
                                let input = item
                                    .get("arguments")
                                    .cloned()
                                    .unwrap_or(serde_json::Value::Null);
                                t.tool_call += chars;
                                if !call_id.is_empty() {
                                    calls.insert(call_id.to_string(), stats.events.len());
                                }
                                let cli = cli_info(name, &input);
                                stats.events.push(HistoryEvent {
                                    timestamp: ts.clone(),
                                    kind: "toolCall".to_string(),
                                    name: name.to_string(),
                                    tokens: chars,
                                    call_id: (!call_id.is_empty()).then(|| call_id.to_string()),
                                    command: cli.command,
                                    help: cli.help,
                                    cli_mention: cli.mentions,
                                    ..Default::default()
                                });
                            }
                            _ => {}
                        }
                    }
                }
                "toolResult" => {
                    let body = cc_message_text(msg);
                    let chars = body.chars().count() as u64;
                    let call_id = msg.get("toolCallId").and_then(|x| x.as_str()).unwrap_or("");
                    let is_error = msg.get("isError").and_then(|v| v.as_bool());
                    let matched = calls.get(call_id).copied();
                    if chars == 0 && matched.is_none() {
                        continue;
                    }
                    t.tool_result += chars;
                    match matched {
                        Some(idx) => attach_result(&mut stats.events[idx], &body, is_error),
                        None => {
                            let mut ev = HistoryEvent {
                                timestamp: ts,
                                kind: "toolResult".to_string(),
                                name: msg
                                    .get("toolName")
                                    .and_then(|n| n.as_str())
                                    .unwrap_or("toolResult")
                                    .to_string(),
                                tokens: chars,
                                ..Default::default()
                            };
                            attach_result(&mut ev, &body, is_error);
                            stats.events.push(ev);
                        }
                    }
                }
                _ => {}
            }
        }
    }

    // 時刻順（ISO 8601 文字列の昇順・安定ソート）
    stats.events.sort_by(|a, b| a.timestamp.cmp(&b.timestamp));
    Ok(stats)
}

/// user / toolResult の content から text を連結し、空白を圧縮して冒頭を返す。
fn first_line_head(content: Option<&serde_json::Value>, max_chars: usize) -> String {
    match content {
        Some(serde_json::Value::Array(items)) => {
            for item in items {
                if item.get("type").and_then(|t| t.as_str()) == Some("text") {
                    if let Some(s) = item.get("text").and_then(|t| t.as_str()) {
                        return head_text(s, max_chars);
                    }
                }
            }
            String::new()
        }
        Some(serde_json::Value::String(s)) => head_text(s, max_chars),
        _ => String::new(),
    }
}

/// 改行・タブをスペースに圧縮し、冒頭 `max_chars` 文字を返す。
fn head_text(s: &str, max_chars: usize) -> String {
    let flat: String = s
        .chars()
        .map(|c| {
            if c == '\n' || c == '\r' || c == '\t' {
                ' '
            } else {
                c
            }
        })
        .collect();
    let flat = flat.trim();
    flat.chars().take(max_chars).collect()
}

/// メッセージの `content` 配列から text ブロックの内容を連結して返す内部ヘルパー。
fn extract_text_from_content(msg: &serde_json::Value) -> String {
    let mut parts: Vec<String> = Vec::new();
    if let Some(arr) = msg.get("content").and_then(|c| c.as_array()) {
        for element in arr {
            if element.get("type").and_then(|t| t.as_str()) == Some("text") {
                if let Some(text) = element.get("text").and_then(|t| t.as_str()) {
                    parts.push(text.to_string());
                }
            }
        }
    }
    parts.concat()
}

/// ISO 8601 形式のタイムスタンプ2つから経過ミリ秒を計算する。
///
/// chrono の RFC3339 パーサを使用し、タイムゾーン付き・小数秒にも対応（2.5 の修正）。
/// パースできない場合は `None` を返す（経過時間は欠落するが他への影響はない）。
fn calculate_elapsed_ms(start: Option<&str>, end: Option<&str>) -> Option<u64> {
    let parse_rfc3339 = |s: &str| -> Option<i64> {
        chrono::DateTime::parse_from_rfc3339(s)
            .ok()
            .map(|dt| dt.timestamp())
            .or_else(|| {
                // RFC3339 にタイムゾーンが無い形式（例: `2026-07-30T10:00:00`）も受容する
                chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S")
                    .ok()
                    .map(|dt| dt.and_utc().timestamp())
            })
    };

    let start_secs = parse_rfc3339(start?)?;
    let end_secs = parse_rfc3339(end?)?;

    if end_secs >= start_secs {
        Some(((end_secs - start_secs) as u64).saturating_mul(1000))
    } else {
        None
    }
}

// ============================================================================
// 内部ヘルパー関数
// ============================================================================

/// エージェント名を安全なファイル名用に変換します。
///
/// `[^a-zA-Z0-9_-]` の文字を `_` に置換します。
fn sanitize_agent_name(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// ディレクトリ名用のタイムスタンプ文字列を生成します。
///
/// 形式: `YYYYMMDDTHHMMSS`（例: `20260730T142536`）。
/// 実装は `crate::common`（chrono 使用）に一元化（P12）。
fn timestamp_for_dirname() -> String {
    crate::common::timestamp_for_dirname()
}

/// ファイル名用のタイムスタンプ文字列を生成します。
///
/// 形式: `YYYY-MM-DDTHH-MM`（例: `2026-07-30T14-25`）。
/// 実装は `crate::common`（chrono 使用）に一元化（P12）。
fn timestamp_for_filename() -> String {
    crate::common::timestamp_for_filename()
}

/// ISO 8601 形式のタイムスタンプ文字列を生成します。
///
/// 形式: `2026-07-30T14:25:36Z`。
/// 実装は `crate::common`（chrono 使用）に一元化（P12）。
fn iso_8601_timestamp() -> String {
    crate::common::iso_8601_timestamp()
}

/// セッション ID を生成します（getrandom ベース・1.7 の修正）。
///
/// 16 バイトの暗号学的乱数を 16 進文字列（32 文字）として返します。
fn generate_session_id() -> String {
    let mut buf = [0u8; 16];
    getrandom::getrandom(&mut buf).expect("OS の乱数生成に失敗");
    let mut hex = String::with_capacity(32);
    for b in buf {
        hex.push_str(&format!("{:02x}", b));
    }
    hex
}

/// セッションディレクトリ内の全 `.jsonl` ファイルを再帰的に走査し、
/// 各ファイルに対してコールバックを実行します。
fn visit_jsonl_files(dir: &Path, callback: &mut dyn FnMut(&Path)) -> Result<(), SessionError> {
    if !dir.is_dir() {
        return Ok(());
    }

    // 走査順を安定化する（"最後の非空出力" の判定が FS 依存にならないように）
    let mut entries: Vec<PathBuf> = fs::read_dir(dir)?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .collect();
    entries.sort();

    for path in entries {
        if path.is_dir() {
            // 再帰的に走査
            visit_jsonl_files(&path, callback)?;
        } else if path.extension().and_then(|e| e.to_str()) == Some("jsonl") {
            callback(&path);
        }
    }

    Ok(())
}

// ============================================================================
// Claude Code（runner=cc）セッション
// ============================================================================
//
// Claude Code のセッション JSONL は `~/.claude/projects/<cwd-slug>/<uuid>.jsonl`
// に保存される（pi のように run フォルダ内ではない）。ここでは cc 実行の
// 出力・統計を pi と同じ `SessionStats` 形式へ写像する。

/// Claude Code のプロジェクトディレクトリ（`~/.claude/projects`）。
pub fn claude_projects_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".claude").join("projects"))
}

/// cwd から Claude Code のプロジェクト slug を作る（英数字以外は全部 `-`）。
///
/// 例: `/Users/me/proj` → `-Users-me-proj`
/// `.worktrees` の `.` も `-` になる: `/a/.worktrees/x` → `-a--worktrees-x`
/// （実機の `~/.claude/projects/` で `.` が `-` になることを確認済み。
/// `.` 以外の記号は cc の規則に合わせた（推測））。
pub fn claude_project_slug(cwd: &Path) -> String {
    cwd.to_string_lossy()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

/// Claude Code のセッション JSONL パスを解決する（存在する場合のみ）。
pub fn claude_session_file(cwd: &Path, session_id: &str) -> Option<PathBuf> {
    let dir = claude_projects_dir()?.join(claude_project_slug(cwd));
    let path = dir.join(format!("{}.jsonl", session_id));
    if path.exists() {
        Some(path)
    } else {
        None
    }
}

/// セッション ID だけで `~/.claude/projects/*/<session_id>.jsonl` を探す。
///
/// worktree を消したあとなど cwd が分からないときの予備。セッション ID は UUID で
/// 別の run のファイルを拾うことはない。
pub fn claude_session_file_any_cwd(session_id: &str) -> Option<PathBuf> {
    let file = format!("{}.jsonl", session_id);
    fs::read_dir(claude_projects_dir()?)
        .ok()?
        .flatten()
        .map(|e| e.path().join(&file))
        .find(|p| p.is_file())
}

/// Claude Code の JSONL を 1 行ずつコールバックへ渡す。
fn visit_claude_jsonl(path: &Path, mut callback: impl FnMut(&serde_json::Value)) -> Result<(), SessionError> {
    let content = fs::read_to_string(path)?;
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(line) {
            callback(&value);
        }
    }
    Ok(())
}

/// Claude Code の assistant メッセージ content からテキストを収集する。
fn claude_message_text(msg: &serde_json::Value) -> String {
    let mut out = String::new();
    if let Some(arr) = msg.get("content").and_then(|v| v.as_array()) {
        for block in arr {
            if block.get("type").and_then(|v| v.as_str()) == Some("text") {
                if let Some(t) = block.get("text").and_then(|v| v.as_str()) {
                    out.push_str(t);
                }
            }
        }
    }
    out
}

/// Claude Code の JSONL から `SessionStats` を抽出する。
///
/// provider の `usage`（`input_tokens` / `output_tokens` /
/// `cache_read_input_tokens` / `cache_creation_input_tokens`）を採用する。
pub fn extract_claude_session_stats(path: &Path) -> Result<SessionStats, SessionError> {
    let mut stats = SessionStats::empty();
    let mut earliest: Option<String> = None;
    let mut latest: Option<String> = None;

    visit_claude_jsonl(path, |v| {
        if let Some(ts) = v.get("timestamp").and_then(|x| x.as_str()) {
            if earliest.as_deref().map(|e| ts < e).unwrap_or(true) {
                earliest = Some(ts.to_string());
            }
            if latest.as_deref().map(|l| ts > l).unwrap_or(true) {
                latest = Some(ts.to_string());
            }
        }
        let ty = v.get("type").and_then(|x| x.as_str()).unwrap_or("");
        if ty != "assistant" {
            if ty == "user" {
                stats.turn_count += 1;
            }
            return;
        }
        let Some(msg) = v.get("message") else { return };
        if stats.model_name.is_none() {
            if let Some(model) = msg.get("model").and_then(|x| x.as_str()) {
                stats.model_name = Some(model.to_string());
            }
        }
        let usage = msg.get("usage");
        let input = usage
            .and_then(|u| u.get("input_tokens"))
            .and_then(|x| x.as_u64())
            .unwrap_or(0);
        let output = usage
            .and_then(|u| u.get("output_tokens"))
            .and_then(|x| x.as_u64())
            .unwrap_or(0);
        let cache = usage
            .and_then(|u| u.get("cache_read_input_tokens"))
            .and_then(|x| x.as_u64())
            .unwrap_or(0)
            + usage
                .and_then(|u| u.get("cache_creation_input_tokens"))
                .and_then(|x| x.as_u64())
                .unwrap_or(0);

        stats.turn_count += 1;
        stats.input_tokens += input;
        stats.output_tokens += output;

        // content 種別を判定し、カテゴリ別トークンへ按分する
        let mut has_tool = false;
        let mut has_think = false;
        let mut has_text = false;
        if let Some(arr) = msg.get("content").and_then(|x| x.as_array()) {
            for block in arr {
                match block.get("type").and_then(|x| x.as_str()).unwrap_or("") {
                    "tool_use" => {
                        has_tool = true;
                        stats.tool_call_count += 1;
                        if let Some(name) = block.get("name").and_then(|x| x.as_str()) {
                            *stats.tool_calls_by_name.entry(name.to_string()).or_insert(0) += 1;
                        }
                    }
                    "thinking" => {
                        has_think = true;
                        if let Some(t) = block.get("thinking").and_then(|x| x.as_str()) {
                            stats.thinking_tokens += t.chars().count() as u64;
                        }
                    }
                    "text" => has_text = true,
                    _ => {}
                }
            }
        }
        let cats = [has_tool, has_think, has_text]
            .iter()
            .filter(|b| **b)
            .count() as u64;
        if cats > 0 {
            let di = input / cats;
            let dout = output / cats;
            let dcache = cache / cats;
            if has_tool {
                stats.token_usage_by_category.tool.input += di;
                stats.token_usage_by_category.tool.output += dout;
                stats.token_usage_by_category.tool.cache += dcache;
            }
            if has_think {
                stats.token_usage_by_category.think.input += di;
                stats.token_usage_by_category.think.output += dout;
                stats.token_usage_by_category.think.cache += dcache;
            }
            if has_text {
                stats.token_usage_by_category.message.input += di;
                stats.token_usage_by_category.message.output += dout;
                stats.token_usage_by_category.message.cache += dcache;
            }
        }
    })?;

    stats.started_at = earliest;
    stats.finished_at = latest;
    stats.elapsed_ms = calculate_elapsed_ms(stats.started_at.as_deref(), stats.finished_at.as_deref());
    stats.total_tokens = stats.input_tokens + stats.output_tokens + stats.thinking_tokens;
    Ok(stats)
}

/// Claude Code の JSONL から全文トランスクリプトを組み立てる。
pub fn extract_claude_full_transcript(path: &Path) -> String {
    let mut out = String::new();
    let _ = visit_claude_jsonl(path, |v| {
        match v.get("type").and_then(|x| x.as_str()).unwrap_or("") {
            "user" => {
                if let Some(msg) = v.get("message") {
                    let text = claude_message_text(msg);
                    if !text.trim().is_empty() {
                        out.push_str(&format!("**user**: {}\n\n", text.trim()));
                    }
                }
            }
            "assistant" => {
                if let Some(msg) = v.get("message") {
                    if let Some(arr) = msg.get("content").and_then(|x| x.as_array()) {
                        for block in arr {
                            match block.get("type").and_then(|x| x.as_str()).unwrap_or("") {
                                "text" => {
                                    if let Some(t) = block.get("text").and_then(|x| x.as_str()) {
                                        if !t.trim().is_empty() {
                                            out.push_str(t.trim());
                                            out.push_str("\n\n");
                                        }
                                    }
                                }
                                "tool_use" => {
                                    if let Some(name) = block.get("name").and_then(|x| x.as_str()) {
                                        out.push_str(&format!("[tool: {}]\n\n", name));
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    });
    out.trim().to_string()
}

/// Claude Code の JSONL から最後の assistant テキストを抽出する。
pub fn extract_claude_final_output(path: &Path) -> Result<String, SessionError> {
    let mut last = String::new();
    visit_claude_jsonl(path, |v| {
        if v.get("type").and_then(|x| x.as_str()) == Some("assistant") {
            if let Some(msg) = v.get("message") {
                let text = claude_message_text(msg);
                if !text.trim().is_empty() {
                    last = text.trim().to_string();
                }
            }
        }
    })?;
    Ok(last)
}


// ============================================================================
// sai-gemini（runner=gl）セッション
//
// sai-gemini-tui の会話ログは `~/.agent-link-gemini/sessions/tui-*.jsonl` に保存される
// （1行1JSON・行種別は `session` / `meta` / `message`）。
// `message` 行の `message.role`（user/assistant）と
// `message.content[0].text` を抽出し、pi と同じ `(role, text)` 形式で返す。
// `meta` 行（geminiUrl 等）は無視する。

/// sai-gemini のセッションディレクトリ（`~/.agent-link-gemini/sessions`）。
pub fn gemini_sessions_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".agent-link-gemini").join("sessions"))
}

/// sai-gemini のセッション JSONL 一覧（新しい順・最大 `limit` 件）。
///
/// sai-gemini-tui の `loadAllSessions` と同じ順序（ファイル名の新しい順）。
pub fn gemini_session_files(limit: usize) -> Vec<PathBuf> {
    let Some(dir) = gemini_sessions_dir() else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("jsonl"))
        .collect();
    files.sort();
    files.reverse();
    files.truncate(limit);
    files
}

/// sai-gemini の `message.content`（`[{type: text, text}]`）から本文を取り出す。
fn gemini_message_text(msg: &serde_json::Value) -> String {
    message_text(msg)
}

/// sai-gemini の JSONL 1 ファイルから user / assistant テキストを時系列で抽出する
/// （message 画面用）。`session` / `meta` 行は無視する。
///
/// pi の `extract_user_assistant_messages` と同じ形式の
/// `(role, text)` リストを返す。
pub fn extract_gemini_user_assistant_messages(
    path: &Path,
) -> Result<Vec<(String, String)>, SessionError> {
    let content = fs::read_to_string(path)?;
    let mut out: Vec<(String, String)> = Vec::new();
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        if value.get("type").and_then(|v| v.as_str()) != Some("message") {
            continue;
        }
        let Some(msg) = value.get("message") else {
            continue;
        };
        let role = msg.get("role").and_then(|r| r.as_str()).unwrap_or("");
        let text = gemini_message_text(msg);
        if text.trim().is_empty() {
            continue;
        }
        match role {
            "user" => out.push(("user".to_string(), text)),
            "assistant" => out.push(("assistant".to_string(), text)),
            _ => {}
        }
    }
    Ok(out)
}

/// sai-gemini の JSONL 1 ファイルから History 画面用の近似統計を抽出する。
///
/// pi の `extract_history_stats` と同形式の `HistoryStats` を返す。
/// token は文字数換算の近似である。user → `user`、assistant → `agent` に写像する。
pub fn extract_gemini_history_stats(path: &Path) -> Result<HistoryStats, SessionError> {
    let content = fs::read_to_string(path)?;
    let mut stats = HistoryStats::default();
    let t = &mut stats.type_tokens;
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        if value.get("type").and_then(|v| v.as_str()) != Some("message") {
            continue;
        }
        let Some(msg) = value.get("message") else {
            continue;
        };
        let ts = value
            .get("timestamp")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string();
        let role = msg.get("role").and_then(|r| r.as_str()).unwrap_or("");
        let text = gemini_message_text(msg);
        let chars = text.chars().count() as u64;
        if chars == 0 {
            continue;
        }
        match role {
            "user" => {
                t.user += chars;
                stats.events.push(HistoryEvent {
                    timestamp: ts,
                    kind: "user".to_string(),
                    name: head_text(&text, 24),
                    tokens: chars,
                    ..Default::default()
                });
            }
            "assistant" => {
                t.agent += chars;
                stats.events.push(HistoryEvent {
                    timestamp: ts,
                    kind: "agent".to_string(),
                    name: head_text(&text, 24),
                    tokens: chars,
                    ..Default::default()
                });
            }
            _ => {}
        }
    }
    stats.events.sort_by(|a, b| a.timestamp.cmp(&b.timestamp));
    Ok(stats)
}

// ============================================================================
// Claude Code（cc）セッション抽出
// ============================================================================

/// Claude Code のセッションディレクトリ（`~/.claude/projects`）。
pub fn cc_sessions_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".claude").join("projects"))
}

/// Claude Code のセッション JSONL 一覧（新しい順・最大 `limit` 件）。
/// `~/.claude/projects/<cwd-slug>/` 配下のすべての `.jsonl` を探し、
/// ファイル更新時刻の新しい順に並べる。
pub fn cc_session_files(limit: usize) -> Vec<PathBuf> {
    let Some(dir) = cc_sessions_dir() else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = Vec::new();
    // projects/ 配下のすべてのサブディレクトリ（cwd-slug）を走査
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            // サブディレクトリ内の .jsonl を収集
            if let Ok(sub_entries) = std::fs::read_dir(&path) {
                for sub_entry in sub_entries.flatten() {
                    let sub_path = sub_entry.path();
                    if sub_path.extension().and_then(|e| e.to_str()) == Some("jsonl") {
                        files.push(sub_path);
                    }
                }
            }
        }
    }
    // ファイル更新時刻の新しい順でソート
    files.sort_by_key(|f| {
        std::fs::metadata(f)
            .and_then(|m| m.modified())
            .unwrap_or(std::time::SystemTime::UNIX_EPOCH)
    });
    files.reverse();
    files.truncate(limit);
    files
}

/// Claude Code の JSONL 1 ファイルから History 画面用の近似統計を抽出する。
///
/// gemini / pi と同じ HistoryStats 形式を返す。
/// - user → user（文字列の content も数える）
/// - assistant → agent（block_history.rs の期待値に合わせる）
/// - tool_use → toolCall、対になる tool_result は同じ行に足す（C11）
/// token は usage フィールド（snake_case）の実値を使用する。
pub fn extract_cc_history_stats(path: &Path) -> Result<HistoryStats, SessionError> {
    let content = fs::read_to_string(path)?;
    let mut stats = HistoryStats::default();
    let t = &mut stats.type_tokens;
    // tool_use の id → events の位置（tool_result は同じ行に足す・C11）
    let mut calls: HashMap<String, usize> = HashMap::new();

    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };

        // type が "user" または "assistant" の行のみ処理
        let msg_type = value.get("type").and_then(|v| v.as_str()).unwrap_or("");
        if msg_type != "user" && msg_type != "assistant" {
            continue;
        }

        let Some(msg) = value.get("message") else {
            continue;
        };

        let ts = value
            .get("timestamp")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string();

        let role = msg.get("role").and_then(|r| r.as_str()).unwrap_or("");

        match role {
            "user" => {
                // 依頼文は素の文字列・text ブロックの両方で入るため、cc_message_text で読む
                let text = cc_message_text(msg);
                let chars = text.chars().count() as u64;
                if chars > 0 {
                    t.user += chars;
                    stats.events.push(HistoryEvent {
                        timestamp: ts.clone(),
                        kind: "user".to_string(),
                        name: head_text(&text, 24),
                        tokens: chars,
                        ..Default::default()
                    });
                }
                // tool_result は tool_use_id で直前の tool_use の行に結ぶ
                for item in msg
                    .get("content")
                    .and_then(|c| c.as_array())
                    .into_iter()
                    .flatten()
                {
                    if item.get("type").and_then(|k| k.as_str()) != Some("tool_result") {
                        continue;
                    }
                    let body = cc_message_text(item);
                    let body_chars = body.chars().count() as u64;
                    let call_id = item
                        .get("tool_use_id")
                        .and_then(|x| x.as_str())
                        .unwrap_or("");
                    let is_error = item.get("is_error").and_then(|v| v.as_bool());
                    let matched = calls.get(call_id).copied();
                    if body_chars == 0 && matched.is_none() {
                        continue;
                    }
                    t.tool_result += body_chars;
                    match matched {
                        Some(idx) => attach_result(&mut stats.events[idx], &body, is_error),
                        None => {
                            let mut ev = HistoryEvent {
                                timestamp: ts.clone(),
                                kind: "toolResult".to_string(),
                                name: "toolResult".to_string(),
                                tokens: body_chars,
                                ..Default::default()
                            };
                            attach_result(&mut ev, &body, is_error);
                            stats.events.push(ev);
                        }
                    }
                }
            }
            "assistant" => {
                let Some(content_array) = msg.get("content").and_then(|c| c.as_array()) else {
                    continue;
                };

                // assistant: input + output + cache read/write を集計（cc では usage が完全）
                let usage = |key: &str| {
                    msg.get("usage")
                        .and_then(|u| u.get(key))
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0)
                };
                let usage_output = usage("output_tokens");
                let chars = usage("input_tokens")
                    + usage_output
                    + usage("cache_read_input_tokens")
                    + usage("cache_creation_input_tokens");
                if chars == 0 {
                    continue;
                }
                t.agent += chars;

                // content から種別を判定（thinking / tool_use がある場合は分離）
                let mut has_thinking = false;
                let mut has_tool_use = false;
                for item in content_array {
                    match item.get("type").and_then(|t| t.as_str()) {
                        Some("thinking") => has_thinking = true,
                        Some("tool_use") => has_tool_use = true,
                        _ => {}
                    }
                }

                // thinking がある場合は thinking をイベントに追加
                if has_thinking {
                    t.thinking += usage_output;
                    stats.events.push(HistoryEvent {
                        timestamp: ts.clone(),
                        kind: "thinking".to_string(),
                        name: "thinking".to_string(),
                        tokens: usage_output,
                        ..Default::default()
                    });
                }

                // tool_use は toolCall として 1 行にする（tokens は名前と入力 JSON の文字数）
                if has_tool_use {
                    for item in content_array {
                        if item.get("type").and_then(|t| t.as_str()) != Some("tool_use") {
                            continue;
                        }
                        let tool_name = item
                            .get("name")
                            .and_then(|n| n.as_str())
                            .unwrap_or("unknown")
                            .to_string();
                        let input = item
                            .get("input")
                            .cloned()
                            .unwrap_or(serde_json::Value::Null);
                        let call_id = item.get("id").and_then(|x| x.as_str()).unwrap_or("");
                        let chars =
                            (tool_name.chars().count() + input.to_string().chars().count()) as u64;
                        t.tool_call += chars;
                        if !call_id.is_empty() {
                            calls.insert(call_id.to_string(), stats.events.len());
                        }
                        let cli = cli_info(&tool_name, &input);
                        stats.events.push(HistoryEvent {
                            timestamp: ts.clone(),
                            kind: "toolCall".to_string(),
                            name: tool_name.clone(),
                            tokens: chars,
                            call_id: (!call_id.is_empty()).then(|| call_id.to_string()),
                            command: cli.command,
                            help: cli.help,
                            cli_mention: cli.mentions,
                            ..Default::default()
                        });
                    }
                }

                // 通常のテキスト応答（thinking/tool_use がない場合）
                if !has_thinking && !has_tool_use {
                    let text = extract_text_from_content(msg);
                    stats.events.push(HistoryEvent {
                        timestamp: ts,
                        kind: "agent".to_string(),
                        name: head_text(&text, 24),
                        tokens: chars,
                        ..Default::default()
                    });
                }
            }
            _ => {}
        }
    }
    stats.events.sort_by(|a, b| a.timestamp.cmp(&b.timestamp));
    Ok(stats)
}

/// 1 呼び出しの全文（`agent-link run history <id> --call <CALL_ID>`・C11）。
///
/// `path_or_dir` は cc なら jsonl 1 件、pi ならセッション dir。呼び出しも結果も
/// 見つからなければ `None`（呼び出し側は終了コード 3）。結果は 20000 字で切る。
pub fn history_call(path_or_dir: &Path, runner: &str, call_id: &str) -> Option<HistoryCall> {
    let mut files: Vec<PathBuf> = Vec::new();
    if path_or_dir.is_dir() {
        visit_jsonl_files(path_or_dir, &mut |p| files.push(p.to_path_buf())).ok()?;
    } else {
        files.push(path_or_dir.to_path_buf());
    }

    let mut call: Option<(String, serde_json::Value)> = None;
    let mut result: Option<(String, Option<bool>)> = None;
    for file in &files {
        let Ok(content) = fs::read_to_string(file) else {
            continue;
        };
        for line in content.lines() {
            let Ok(value) = serde_json::from_str::<serde_json::Value>(line.trim()) else {
                continue;
            };
            let Some(msg) = value.get("message") else {
                continue;
            };
            let items = msg.get("content").and_then(|c| c.as_array());
            if runner == "cc" {
                for item in items.into_iter().flatten() {
                    let id = item.get("id").and_then(|x| x.as_str());
                    let used = item.get("tool_use_id").and_then(|x| x.as_str());
                    match item.get("type").and_then(|k| k.as_str()) {
                        Some("tool_use") if id == Some(call_id) => {
                            let name = item.get("name").and_then(|n| n.as_str()).unwrap_or("");
                            let input = item
                                .get("input")
                                .cloned()
                                .unwrap_or(serde_json::Value::Null);
                            call = Some((name.to_string(), input));
                        }
                        Some("tool_result") if used == Some(call_id) => {
                            let is_error = item.get("is_error").and_then(|v| v.as_bool());
                            result = Some((cc_message_text(item), is_error));
                        }
                        _ => {}
                    }
                }
            } else {
                if msg.get("toolCallId").and_then(|x| x.as_str()) == Some(call_id) {
                    let is_error = msg.get("isError").and_then(|v| v.as_bool());
                    result = Some((cc_message_text(msg), is_error));
                }
                for item in items.into_iter().flatten() {
                    if item.get("type").and_then(|k| k.as_str()) == Some("toolCall")
                        && item.get("id").and_then(|x| x.as_str()) == Some(call_id)
                    {
                        let name = item.get("name").and_then(|n| n.as_str()).unwrap_or("");
                        let input = item
                            .get("arguments")
                            .cloned()
                            .unwrap_or(serde_json::Value::Null);
                        call = Some((name.to_string(), input));
                    }
                }
            }
        }
    }

    if call.is_none() && result.is_none() {
        return None;
    }
    let (name, input) = call.unwrap_or((String::new(), serde_json::Value::Null));
    let (body, is_error) = result.unwrap_or((String::new(), None));
    let truncated = body.chars().count() > CALL_RESULT_CHARS;
    Some(HistoryCall {
        call_id: call_id.to_string(),
        command: cli_command(&name, &input),
        name,
        input: input.to_string(),
        result: body.chars().take(CALL_RESULT_CHARS).collect(),
        is_error,
        truncated,
    })
}

/// `history_call` の結果（C11 の JSON 形）。
#[derive(Debug, Clone, serde::Serialize)]
pub struct HistoryCall {
    /// 呼び出しの番号
    pub call_id: String,
    /// 道具の名前（`Bash`・`bash` など）
    pub name: String,
    /// Bash 系の道具の命令（`agent-link doc read` など）
    pub command: Option<String>,
    /// 入力（JSON 文字列）
    pub input: String,
    /// 結果の本文（20000 字まで）
    pub result: String,
    /// 結果が失敗か
    pub is_error: Option<bool>,
    /// 結果が 20000 字を超えて切ったか
    pub truncated: bool,
}

/// History 1 行の結果の本文の先頭（200 字・C11）。
const PREVIEW_CHARS: usize = 200;
/// `history_call` で返す結果の全文の上限（字数・C11）。
const CALL_RESULT_CHARS: usize = 20000;

/// 結果（`toolResult` の本文）を呼び出しの行に足す（preview・is_error・結果の文字数・失敗の分類）。
fn attach_result(ev: &mut HistoryEvent, body: &str, is_error: Option<bool>) {
    ev.preview = Some(body.chars().take(PREVIEW_CHARS).collect());
    ev.is_error = is_error;
    ev.result_tokens = Some(body.chars().count() as u64);
    ev.failure = (is_error == Some(true)).then(|| failure_class(body));
}

/// 失敗の分類（C11）。終了コード 2・3・4 は `input`・`not_found`・`conflict`、
/// それ以外と読めないものは `other`。
pub fn failure_class(result: &str) -> &'static str {
    match exit_code_of(result) {
        Some(2) => "input",
        Some(3) => "not_found",
        Some(4) => "conflict",
        _ => "other",
    }
}

/// 結果の文の行頭の `Exit code <n>`（大文字小文字を区別しない）から終了コードを読む。
pub fn exit_code_of(result: &str) -> Option<i32> {
    result.lines().find_map(|line| {
        if !line.get(..9)?.eq_ignore_ascii_case("exit code") {
            return None;
        }
        line.get(9..)?.split_whitespace().next()?.parse().ok()
    })
}

/// Bash 系の道具の入力から命令名を作る（C11）。
///
/// 道具の名前が `Bash`・`bash` で、入力の `command` の中で命令の位置（先頭・区切りの後・
/// `VAR=値` の後）にある `agent-link`・`agent-link-web`（パスの最後の部分で比べる）から、
/// 続く語のうち `[a-z][a-z0-9-]*` の形の語を 2 つまで足す。区切り（`&&`・`|`・`;`）で止める。
/// 例: `agent-link doc read AXANSMpo --json` → `agent-link doc read`。
pub fn cli_command(tool_name: &str, input: &serde_json::Value) -> Option<String> {
    cli_info(tool_name, input).command
}

/// Bash 系の道具の呼び出しから読んだ CLI の情報（C11・U04）。
#[derive(Debug, Default, Clone, PartialEq)]
pub struct CliInfo {
    /// 命令名（`agent-link doc read` など）。命令の位置に `agent-link*` が無ければ `None`
    pub command: Option<String>,
    /// 命令の語（区切りまで）に `--help`・`-h` があるか
    pub help: bool,
    /// 入力に `agent-link`・`agent-link-web` の語があるか（命令の位置でなくても）
    pub mentions: bool,
}

/// Bash 系の道具の入力から命令名などを読む（C11・U04）。
///
/// 命令の位置（先頭・区切りの後・`VAR=値` と前置きの道具の後）の `agent-link*` から、続く語を
/// 2 つまで足す。heredoc の本文・`#` の注釈・引用符の中の語は命令に数えない。
pub fn cli_info(tool_name: &str, input: &serde_json::Value) -> CliInfo {
    if tool_name != "Bash" && tool_name != "bash" {
        return CliInfo::default();
    }
    let Some(text) = input.get("command").and_then(|v| v.as_str()) else {
        return CliInfo::default();
    };
    let body = strip_heredocs(text);
    let toks = tokens(&body);
    let (command, help) = match find_command(&toks) {
        Some((command, help)) => (Some(command), help),
        None => (None, false),
    };
    CliInfo {
        command,
        help,
        mentions: AGENT_LINK_CLI_NAMES.iter().any(|name| body.contains(name)),
    }
}

const AGENT_LINK_CLI_NAMES: [&str; 2] = ["agent-link", "agent-link-web"];

/// シェルの語（`Word`）か、命令の区切り（`Break`：改行・`;`・`|`・`&&`・括弧など）。
#[derive(Debug, PartialEq)]
enum Tok {
    Word(String),
    Break,
}

/// heredoc（`<<EOF`・`<<-'EOF'`）の本文の行を取り除く。終わりの行は `EOF` の行（前後の空白を除く）。
/// 本文は命令ではないので、`agent-link task create <<'EOF'` の後の語が命令名に入らないようにする。
fn strip_heredocs(text: &str) -> String {
    let mut out = String::new();
    let mut end: Option<String> = None;
    for line in text.lines() {
        if let Some(word) = &end {
            if line.trim() == word {
                end = None;
            }
            continue;
        }
        out.push_str(line);
        out.push('\n');
        end = heredoc_word(line);
    }
    out
}

/// 行の最初の `<<WORD` の WORD を返す（`<<<` の here-string は除く）。
fn heredoc_word(line: &str) -> Option<String> {
    let rest = &line[line.find("<<")? + 2..];
    if rest.starts_with('<') {
        return None;
    }
    let rest = rest.strip_prefix('-').unwrap_or(rest).trim_start();
    let rest = rest.trim_start_matches(|c| c == '\'' || c == '"');
    let word: String = rest
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
        .collect();
    (!word.is_empty()).then_some(word)
}

/// 引用符・注釈（語の先頭の `#` から行末まで）を見て、語と区切りに割る。
/// 引用符の中の空白や区切りは割らない。語は引用符のまま残す（`'x'` は語として `x` と違う）。
fn tokens(text: &str) -> Vec<Tok> {
    fn push_word(word: &mut String, out: &mut Vec<Tok>) {
        if !word.is_empty() {
            out.push(Tok::Word(std::mem::take(word)));
        }
    }
    let mut out = Vec::new();
    let mut word = String::new();
    let mut quote: Option<char> = None;
    let mut comment = false;
    let mut prev = ' ';
    for c in text.chars() {
        if comment {
            if c == '\n' {
                comment = false;
            } else {
                prev = c;
                continue;
            }
        }
        if let Some(q) = quote {
            word.push(c);
            if c == q {
                quote = None;
            }
        } else {
            match c {
                '\'' | '"' => {
                    quote = Some(c);
                    word.push(c);
                }
                '#' if word.is_empty() => comment = true,
                ' ' | '\t' | '\r' => push_word(&mut word, &mut out),
                '\n' | ';' | '|' | '(' | ')' | '`' => {
                    push_word(&mut word, &mut out);
                    out.push(Tok::Break);
                }
                // `2>&1` の `&` は区切りではない
                '&' if prev != '>' && prev != '<' => {
                    push_word(&mut word, &mut out);
                    out.push(Tok::Break);
                }
                _ => word.push(c),
            }
        }
        prev = c;
    }
    push_word(&mut word, &mut out);
    out
}

/// 命令の位置の `agent-link*` を探す。見つかれば命令名と `--help` の有無を返す。
fn find_command(toks: &[Tok]) -> Option<(String, bool)> {
    let mut at_start = true;
    let mut i = 0;
    while i < toks.len() {
        let Tok::Word(w) = &toks[i] else {
            at_start = true;
            i += 1;
            continue;
        };
        if at_start {
            if is_assignment(w) {
                i += 1;
                continue;
            }
            if let Some(next) = after_wrapper(toks, i) {
                i = next;
                continue;
            }
            if is_sai_cli(w) {
                return Some(command_of(&toks[i..]));
            }
        }
        at_start = false;
        i += 1;
    }
    None
}

/// `agent-link*` の語から始まる命令の名前（`agent-link doc read`）と `--help` の有無。
/// 名前は語が `[a-z][a-z0-9-]*` の形のものを 2 つまで足す。区切りで止める。
fn command_of(toks: &[Tok]) -> (String, bool) {
    let words: Vec<&str> = toks
        .iter()
        .take_while(|t| matches!(t, Tok::Word(_)))
        .map(|t| match t {
            Tok::Word(w) => w.as_str(),
            Tok::Break => "",
        })
        .collect();
    let help = words.iter().any(|w| *w == "--help" || *w == "-h");
    let mut parts = vec![basename(words[0]).to_string()];
    for w in &words[1..] {
        if parts.len() == 3 {
            break;
        }
        if is_sub_word(w) {
            parts.push(w.to_string());
        }
    }
    (parts.join(" "), help)
}

/// 前置きの道具（`timeout`・`env`・`exec`・`xargs`・`perl` など）の次の語の位置（U04）。
/// 前置きでなければ `None`。
fn after_wrapper(toks: &[Tok], i: usize) -> Option<usize> {
    let Tok::Word(w) = toks.get(i)? else {
        return None;
    };
    let j = i + 1;
    Some(match basename(w) {
        "exec" | "nohup" | "time" => j,
        // 時間の語（`60`）を 1 つ飛ばす
        "timeout" => skip_options(toks, j, &["-s", "-k"], false) + 1,
        "env" => skip_options(toks, j, &["-u", "-C", "-S"], true),
        "xargs" => skip_options(
            toks,
            j,
            &["-I", "-n", "-P", "-L", "-d", "-s", "-E", "-a"],
            false,
        ),
        // `perl -e '…' agent-link …` の `-e` の script も飛ばす
        "perl" => skip_options(toks, j, &["-e", "-E", "-I", "-M"], false),
        _ => return None,
    })
}

/// オプション（`-x`）を飛ばす。`values` は次の語を値として取るオプション。`assign` が true なら
/// `VAR=値` の代入も飛ばす。
fn skip_options(toks: &[Tok], mut j: usize, values: &[&str], assign: bool) -> usize {
    while let Some(Tok::Word(w)) = toks.get(j) {
        if w.starts_with('-') {
            j += if values.contains(&w.as_str()) { 2 } else { 1 };
        } else if assign && is_assignment(w) {
            j += 1;
        } else {
            break;
        }
    }
    j
}

fn basename(word: &str) -> &str {
    word.rsplit('/').next().unwrap_or(word)
}

fn is_sai_cli(word: &str) -> bool {
    matches!(basename(word), "agent-link" | "agent-link-web")
}

/// `VAR=値` の形の語（環境変数の代入）。
fn is_assignment(word: &str) -> bool {
    match word.split_once('=') {
        Some((key, _)) => {
            !key.is_empty()
                && !key.starts_with(|c: char| c.is_ascii_digit())
                && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        }
        None => false,
    }
}

/// `[a-z][a-z0-9-]*` の形の語か（オプション `-x` や引数 `#37.5` は外れる）。
fn is_sub_word(word: &str) -> bool {
    let mut chars = word.chars();
    matches!(chars.next(), Some('a'..='z'))
        && chars.all(|c| matches!(c, 'a'..='z' | '0'..='9' | '-'))
}

/// cc の message.content（文字列 or `[{type,text}]` ブロック配列）から text のみを
/// 取り出す。pi/gemini は常にブロック配列だが、cc の user メッセージは素の文字列で
/// 入っていることが多いため、両形式を吸収する。
fn cc_message_text(msg: &serde_json::Value) -> String {
    match msg.get("content") {
        Some(serde_json::Value::String(s)) => s.clone(),
        Some(serde_json::Value::Array(items)) => {
            let mut parts: Vec<String> = Vec::new();
            for item in items {
                if item.get("type").and_then(|t| t.as_str()) == Some("text") {
                    if let Some(t) = item.get("text").and_then(|x| x.as_str()) {
                        parts.push(t.to_string());
                    }
                }
            }
            parts.concat()
        }
        _ => String::new(),
    }
}

/// Claude Code（cc）セッション JSONL 1 ファイルから user / assistant のテキストのみを
/// 時系列で抽出する（message 画面用）。pi の `extract_user_assistant_messages` と
/// 同形式の `(role, text)` リストを返す。
pub fn extract_cc_user_assistant_messages(
    path: &Path,
) -> Result<Vec<(String, String)>, SessionError> {
    let content = fs::read_to_string(path)?;
    let mut out: Vec<(String, String)> = Vec::new();
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        let msg_type = value.get("type").and_then(|v| v.as_str()).unwrap_or("");
        if msg_type != "user" && msg_type != "assistant" {
            continue;
        }
        let Some(msg) = value.get("message") else {
            continue;
        };
        let role = msg.get("role").and_then(|r| r.as_str()).unwrap_or("");
        let text = cc_message_text(msg);
        if text.trim().is_empty() {
            continue;
        }
        match role {
            "user" => out.push(("user".to_string(), text)),
            "assistant" => out.push(("assistant".to_string(), text)),
            _ => {}
        }
    }
    Ok(out)
}

// ============================================================================
// テスト
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    /// cc は cwd の `.` も `-` にする（実機の `~/.claude/projects/` の名前）。
    /// worktree（`.worktrees/<branch>`）の会話が見つからなくなっていた。
    #[test]
    fn test_claude_project_slug_replaces_dot() {
        assert_eq!(claude_project_slug(Path::new("/Users/me/proj")), "-Users-me-proj");
        assert_eq!(
            claude_project_slug(Path::new("/Users/alice/work/proj/.worktrees/bugfix-70")),
            "-Users-alice-work-proj--worktrees-bugfix-70"
        );
    }

    /// cwd が分からなくても、セッション ID だけで会話ファイルを探せる。
    #[test]
    fn test_claude_session_file_any_cwd_finds_by_session_id() {
        let _env = crate::common::AGENT_LINK_ENV_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let home = std::env::temp_dir().join(format!("sai-any-cwd-{}", std::process::id()));
        let dir = home.join(".claude/projects/-gone--worktrees-x");
        fs::create_dir_all(&dir).unwrap();
        let sid = "cccccccc-3333-4333-8333-333333333333";
        fs::write(dir.join(format!("{}.jsonl", sid)), "{}\n").unwrap();
        let old = std::env::var_os("HOME");
        std::env::set_var("HOME", &home);
        let found = claude_session_file_any_cwd(sid);
        let missing = claude_session_file_any_cwd("dddddddd-4444-4444-8444-444444444444");
        match old {
            Some(v) => std::env::set_var("HOME", v),
            None => std::env::remove_var("HOME"),
        }
        let _ = fs::remove_dir_all(&home);
        assert_eq!(found, Some(dir.join(format!("{}.jsonl", sid))));
        assert_eq!(missing, None);
    }

    #[test]
    fn test_sanitize_agent_name() {
        // セッション dir はプロジェクトルート配下に作られるため、
        // AGENT_LINK_PROJECT_ROOT を書き換える他モジュールのテストと干渉しないよう直列化する
        // （干渉するとディレクトリが途中で消えて統計が 0 になり、偶発失敗していた）
        let _env = crate::common::AGENT_LINK_ENV_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        assert_eq!(sanitize_agent_name("hello-world"), "hello-world");
        assert_eq!(sanitize_agent_name("Hello World!"), "Hello_World_");
        assert_eq!(sanitize_agent_name("test@agent#123"), "test_agent_123");
        assert_eq!(sanitize_agent_name(""), "");
        assert_eq!(sanitize_agent_name("abc"), "abc");
    }

    #[test]
    fn test_generate_session_id() {
        // セッション dir はプロジェクトルート配下に作られるため、
        // AGENT_LINK_PROJECT_ROOT を書き換える他モジュールのテストと干渉しないよう直列化する
        // （干渉するとディレクトリが途中で消えて統計が 0 になり、偶発失敗していた）
        let _env = crate::common::AGENT_LINK_ENV_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let id1 = generate_session_id();
        let id2 = generate_session_id();
        assert_ne!(id1, id2, "2回の呼び出しで異なるIDが生成されるべき");
        assert!(!id1.is_empty());
        assert!(id1.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn test_random_u32() {
        // セッション dir はプロジェクトルート配下に作られるため、
        // AGENT_LINK_PROJECT_ROOT を書き換える他モジュールのテストと干渉しないよう直列化する
        // （干渉するとディレクトリが途中で消えて統計が 0 になり、偶発失敗していた）
        let _env = crate::common::AGENT_LINK_ENV_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let r1 = random_u32();
        let r2 = random_u32();
        // 乱数なので異なる値になる確率が高い
        assert_ne!(r1, r2);
    }

    // ── セッション統計抽出のテスト ──

    #[test]
    fn test_find_main_session_file() {
        // セッション dir はプロジェクトルート配下に作られるため、
        // AGENT_LINK_PROJECT_ROOT を書き換える他モジュールのテストと干渉しないよう直列化する
        // （干渉するとディレクトリが途中で消えて統計が 0 になり、偶発失敗していた）
        let _env = crate::common::AGENT_LINK_ENV_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = create_session_dir("find-main").expect("ディレクトリ作成失敗");

        // メインセッションファイル（ディレクトリ直下）を作成
        let main_file =
            create_session_file(&dir, "main-agent").expect("メインセッションファイル作成失敗");
        // サブディレクトリ（サブエージェント）に追加のセッションを作成
        let sub_dir = dir.join("sub-agent");
        std::fs::create_dir_all(&sub_dir).expect("サブディレクトリ作成失敗");
        let _sub_file =
            create_session_file(&sub_dir, "sub-agent").expect("サブセッションファイル作成失敗");

        let found = find_main_session_file(&dir).expect("メインセッションファイルを検出できるべき");
        // ディレクトリ直下の .jsonl が選択される（サブディレクトリ内は対象外）
        assert_eq!(found, main_file);

        cleanup_session_dir(&dir);
    }

    #[test]
    fn test_find_main_session_file_missing() {
        // セッション dir はプロジェクトルート配下に作られるため、
        // AGENT_LINK_PROJECT_ROOT を書き換える他モジュールのテストと干渉しないよう直列化する
        // （干渉するとディレクトリが途中で消えて統計が 0 になり、偶発失敗していた）
        let _env = crate::common::AGENT_LINK_ENV_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = create_session_dir("find-main-missing").expect("ディレクトリ作成失敗");
        // .jsonl ファイルを配置しない
        let result = find_main_session_file(&dir);
        assert!(result.is_err(), ".jsonl がない場合はエラーになるべき");

        cleanup_session_dir(&dir);
    }

    #[test]
    fn test_extract_session_stats_basic() {
        // セッション dir はプロジェクトルート配下に作られるため、
        // AGENT_LINK_PROJECT_ROOT を書き換える他モジュールのテストと干渉しないよう直列化する
        // （干渉するとディレクトリが途中で消えて統計が 0 になり、偶発失敗していた）
        let _env = crate::common::AGENT_LINK_ENV_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = create_session_dir("stats-test").expect("ディレクトリ作成失敗");
        let session_path = dir.join("test.jsonl");

        let content = r#"{"type":"session","version":3,"id":"test","timestamp":"2026-07-30T10:00:00Z","cwd":"/tmp"}
{"type":"message","message":{"role":"user","content":[{"type":"text","text":"Hello"}]}}
{"type":"message","message":{"role":"assistant","content":[{"type":"text","text":"Hi there!"}]}}
"#;
        std::fs::write(&session_path, content).expect("テストファイルの書き込みに失敗");

        let stats = extract_session_stats(&dir).expect("統計抽出に成功するべき");

        assert_eq!(stats.turn_count, 1);
        assert!(stats.input_tokens > 0);
        assert!(stats.output_tokens > 0);
        assert!(stats.started_at.is_some());

        cleanup_session_dir(&dir);
    }

    #[test]
    fn test_extract_session_stats_with_thinking() {
        // セッション dir はプロジェクトルート配下に作られるため、
        // AGENT_LINK_PROJECT_ROOT を書き換える他モジュールのテストと干渉しないよう直列化する
        // （干渉するとディレクトリが途中で消えて統計が 0 になり、偶発失敗していた）
        let _env = crate::common::AGENT_LINK_ENV_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = create_session_dir("thinking-stats").expect("ディレクトリ作成失敗");
        let session_path = dir.join("test.jsonl");

        let content = r#"{"type":"session","version":3,"id":"test","timestamp":"2026-07-30T10:00:00Z","cwd":"/tmp"}
{"type":"message","message":{"role":"user","content":[{"type":"text","text":"Hello"}]}}
{"type":"message","message":{"role":"assistant","content":[{"type":"thinking","thinking":"Let me think about this..."},{"type":"text","text":"Here is my answer"}]}}
"#;
        std::fs::write(&session_path, content).expect("テストファイルの書き込みに失敗");

        let stats = extract_session_stats(&dir).expect("統計抽出に成功するべき");

        assert!(
            stats.thinking_tokens > 0,
            "thinking トークンがカウントされるべき"
        );
        assert!(stats.output_tokens > 0, "出力トークンがカウントされるべき");

        cleanup_session_dir(&dir);
    }

    #[test]
    fn test_extract_session_stats_with_tool_calls() {
        // セッション dir はプロジェクトルート配下に作られるため、
        // AGENT_LINK_PROJECT_ROOT を書き換える他モジュールのテストと干渉しないよう直列化する
        // （干渉するとディレクトリが途中で消えて統計が 0 になり、偶発失敗していた）
        let _env = crate::common::AGENT_LINK_ENV_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = create_session_dir("tool-stats").expect("ディレクトリ作成失敗");
        let session_path = dir.join("test.jsonl");

        let content = r#"{"type":"session","version":3,"id":"test","timestamp":"2026-07-30T10:00:00Z","cwd":"/tmp"}
{"type":"message","message":{"role":"user","content":[{"type":"text","text":"Read file"}]}}
{"type":"message","message":{"role":"assistant","content":[{"type":"toolCall","name":"read","id":"call_1","arguments":{"path":"/tmp/test"}},{"type":"toolCall","name":"bash","id":"call_2","arguments":{"command":"ls"}}]}}
"#;
        std::fs::write(&session_path, content).expect("テストファイルの書き込みに失敗");

        let stats = extract_session_stats(&dir).expect("統計抽出に成功するべき");

        assert_eq!(stats.tool_call_count, 2);
        assert_eq!(stats.tool_calls_by_name.get("read"), Some(&1));
        assert_eq!(stats.tool_calls_by_name.get("bash"), Some(&1));

        cleanup_session_dir(&dir);
    }

    #[test]
    fn test_extract_session_stats_multiple_turns() {
        // セッション dir はプロジェクトルート配下に作られるため、
        // AGENT_LINK_PROJECT_ROOT を書き換える他モジュールのテストと干渉しないよう直列化する
        // （干渉するとディレクトリが途中で消えて統計が 0 になり、偶発失敗していた）
        let _env = crate::common::AGENT_LINK_ENV_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = create_session_dir("multi-turn-stats").expect("ディレクトリ作成失敗");
        let session_path = dir.join("test.jsonl");

        let content = r#"{"type":"session","version":3,"id":"test","timestamp":"2026-07-30T10:00:00Z","cwd":"/tmp"}
{"type":"message","message":{"role":"user","content":[{"type":"text","text":"First"}]}}
{"type":"message","message":{"role":"assistant","content":[{"type":"text","text":"Answer 1"}]}}
{"type":"message","message":{"role":"user","content":[{"type":"text","text":"Second"}]}}
{"type":"message","message":{"role":"assistant","content":[{"type":"text","text":"Answer 2"}]}}
{"type":"message","message":{"role":"user","content":[{"type":"text","text":"Third"}]}}
{"type":"message","message":{"role":"assistant","content":[{"type":"text","text":"Answer 3"}]}}
"#;
        std::fs::write(&session_path, content).expect("テストファイルの書き込みに失敗");

        let stats = extract_session_stats(&dir).expect("統計抽出に成功するべき");

        assert_eq!(stats.turn_count, 3, "3往復のターン数がカウントされるべき");

        cleanup_session_dir(&dir);
    }

    #[test]
    fn test_extract_session_stats_elapsed_time() {
        // セッション dir はプロジェクトルート配下に作られるため、
        // AGENT_LINK_PROJECT_ROOT を書き換える他モジュールのテストと干渉しないよう直列化する
        // （干渉するとディレクトリが途中で消えて統計が 0 になり、偶発失敗していた）
        let _env = crate::common::AGENT_LINK_ENV_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = create_session_dir("elapsed-stats").expect("ディレクトリ作成失敗");
        let session_path = dir.join("test.jsonl");

        let content = r#"{"type":"session","version":3,"id":"test","timestamp":"2026-07-30T10:00:00Z","cwd":"/tmp"}
{"type":"message","timestamp":"2026-07-30T10:05:00Z","message":{"role":"user","content":[{"type":"text","text":"Hello"}]}}
{"type":"message","timestamp":"2026-07-30T10:10:00Z","message":{"role":"assistant","content":[{"type":"text","text":"Hi"}]}}
"#;
        std::fs::write(&session_path, content).expect("テストファイルの書き込みに失敗");

        let stats = extract_session_stats(&dir).expect("統計抽出に成功するべき");

        // 10:00:00 → 10:10:00 = 600秒 = 600,000ミリ秒
        assert!(stats.elapsed_ms.is_some());
        assert_eq!(stats.elapsed_ms.unwrap(), 600_000);

        cleanup_session_dir(&dir);
    }

    #[test]
    fn test_extract_session_stats_empty_dir() {
        // セッション dir はプロジェクトルート配下に作られるため、
        // AGENT_LINK_PROJECT_ROOT を書き換える他モジュールのテストと干渉しないよう直列化する
        // （干渉するとディレクトリが途中で消えて統計が 0 になり、偶発失敗していた）
        let _env = crate::common::AGENT_LINK_ENV_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = create_session_dir("empty-stats").expect("ディレクトリ作成失敗");

        let stats = extract_session_stats(&dir).expect("空ディレクトリでもエラーにならないべき");

        assert_eq!(stats.turn_count, 0);
        assert_eq!(stats.input_tokens, 0);
        assert_eq!(stats.tool_call_count, 0);

        cleanup_session_dir(&dir);
    }

    #[test]
    fn test_session_stats_serialization() {
        // セッション dir はプロジェクトルート配下に作られるため、
        // AGENT_LINK_PROJECT_ROOT を書き換える他モジュールのテストと干渉しないよう直列化する
        // （干渉するとディレクトリが途中で消えて統計が 0 になり、偶発失敗していた）
        let _env = crate::common::AGENT_LINK_ENV_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let stats = SessionStats {
            started_at: Some("2026-07-30T10:00:00Z".to_string()),
            finished_at: Some("2026-07-30T10:05:00Z".to_string()),
            elapsed_ms: Some(300_000),
            input_tokens: 100,
            output_tokens: 200,
            thinking_tokens: 50,
            total_tokens: 350,
            tool_call_count: 3,
            turn_count: 2,
            model_name: Some("anthropic/claude-sonnet-4-5".to_string()),
            cost_usd: 0.012,
            tool_calls_by_name: {
                let mut m = std::collections::HashMap::new();
                m.insert("read".to_string(), 2);
                m.insert("write".to_string(), 1);
                m
            },
            token_usage_by_category: TokenUsageByCategory {
                tool: TokenCategoryStats {
                    input: 30,
                    output: 40,
                    cache: 10,
                },
                message: TokenCategoryStats {
                    input: 10,
                    output: 60,
                    cache: 20,
                },
                think: TokenCategoryStats {
                    input: 5,
                    output: 70,
                    cache: 30,
                },
            },
        };

        let json = serde_json::to_string(&stats).expect("シリアライズ成功");
        let deserialized: SessionStats = serde_json::from_str(&json).expect("デシリアライズ成功");

        assert_eq!(deserialized.input_tokens, 100);
        assert_eq!(deserialized.output_tokens, 200);
        assert_eq!(deserialized.tool_call_count, 3);
        assert_eq!(deserialized.turn_count, 2);
        assert_eq!(
            deserialized.model_name.as_deref(),
            Some("anthropic/claude-sonnet-4-5")
        );
        assert_eq!(deserialized.token_usage_by_category.tool.input, 30);
        assert_eq!(deserialized.token_usage_by_category.think.cache, 30);
    }

    // ── R1 / R3: カテゴリ別トークン使用量の集計 ──

    #[test]
    fn test_extract_session_stats_usage_cache_sum() {
        // セッション dir はプロジェクトルート配下に作られるため、
        // AGENT_LINK_PROJECT_ROOT を書き換える他モジュールのテストと干渉しないよう直列化する
        // （干渉するとディレクトリが途中で消えて統計が 0 になり、偶発失敗していた）
        let _env = crate::common::AGENT_LINK_ENV_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = create_session_dir("usage-cache").expect("ディレクトリ作成失敗");
        let session_path = dir.join("test.jsonl");

        // assistant に usage（cacheRead / cacheWrite 含む）を持つ行
        let content = r#"{"type":"session","version":3,"id":"test","timestamp":"2026-07-30T10:00:00Z","cwd":"/tmp"}
{"type":"message","message":{"role":"assistant","content":[{"type":"text","text":"Hello"}],"usage":{"input":100,"output":50,"cacheRead":200,"cacheWrite":30,"totalTokens":380}}}
"#;
        std::fs::write(&session_path, content).expect("テストファイルの書き込みに失敗");

        let stats = extract_session_stats(&dir).expect("統計抽出に成功するべき");

        // cache = cacheRead + cacheWrite = 200 + 30 = 230（R3）
        assert_eq!(stats.token_usage_by_category.message.cache, 230);
        assert_eq!(stats.token_usage_by_category.message.input, 100);
        assert_eq!(stats.token_usage_by_category.message.output, 50);

        cleanup_session_dir(&dir);
    }

    #[test]
    fn test_extract_session_stats_category_breakdown() {
        // セッション dir はプロジェクトルート配下に作られるため、
        // AGENT_LINK_PROJECT_ROOT を書き換える他モジュールのテストと干渉しないよう直列化する
        // （干渉するとディレクトリが途中で消えて統計が 0 になり、偶発失敗していた）
        let _env = crate::common::AGENT_LINK_ENV_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = create_session_dir("cat-breakdown").expect("ディレクトリ作成失敗");
        let session_path = dir.join("test.jsonl");

        // user テキスト → message.input / tool_result → tool.input
        // assistant(usage, toolCall) → tool / (usage, thinking) → think / (usage, text) → message
        // toolResult → tool.input
        let content = r#"{"type":"session","version":3,"id":"test","timestamp":"2026-07-30T10:00:00Z","cwd":"/tmp"}
{"type":"message","message":{"role":"user","content":[{"type":"text","text":"Ask"}]}}
{"type":"message","message":{"role":"assistant","content":[{"type":"toolCall","name":"read","id":"c1","arguments":{"path":"/a"}}],"usage":{"input":10,"output":20,"cacheRead":1,"cacheWrite":2,"totalTokens":33}}}
{"type":"message","message":{"role":"toolResult","toolCallId":"c1","toolName":"read","content":[{"type":"text","text":"data"}],"isError":false}}
{"type":"message","message":{"role":"assistant","content":[{"type":"thinking","thinking":"hmm"}],"usage":{"input":5,"output":7,"cacheRead":0,"cacheWrite":0,"totalTokens":12}}}
{"type":"message","message":{"role":"assistant","content":[{"type":"text","text":"Done"}],"usage":{"input":3,"output":4,"cacheRead":8,"cacheWrite":1,"totalTokens":16}}}
"#;
        std::fs::write(&session_path, content).expect("テストファイルの書き込みに失敗");

        let stats = extract_session_stats(&dir).expect("統計抽出に成功するべき");
        let b = &stats.token_usage_by_category;

        // tool: toolCall の usage（input 10 / output 20 / cache 1+2）＋ toolResult の文字数（4）
        assert_eq!(b.tool.input, 10 + 4);
        assert_eq!(b.tool.output, 20);
        assert_eq!(b.tool.cache, 3);

        // think: thinking の usage
        assert_eq!(b.think.input, 5);
        assert_eq!(b.think.output, 7);
        assert_eq!(b.think.cache, 0);

        // message: user テキスト 3 文字＋ text の usage
        assert_eq!(b.message.input, 3 + 3);
        assert_eq!(b.message.output, 4);
        assert_eq!(b.message.cache, 9);

        cleanup_session_dir(&dir);
    }

    #[test]
    fn test_extract_session_stats_category_merge_multi_file() {
        // セッション dir はプロジェクトルート配下に作られるため、
        // AGENT_LINK_PROJECT_ROOT を書き換える他モジュールのテストと干渉しないよう直列化する
        // （干渉するとディレクトリが途中で消えて統計が 0 になり、偶発失敗していた）
        let _env = crate::common::AGENT_LINK_ENV_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = create_session_dir("cat-merge").expect("ディレクトリ作成失敗");
        let f1 = dir.join("a.jsonl");
        let f2 = dir.join("b.jsonl");

        let c1 = r#"{"type":"session","version":3,"id":"a","timestamp":"2026-07-30T10:00:00Z","cwd":"/tmp"}
{"type":"message","message":{"role":"assistant","content":[{"type":"toolCall","name":"bash","id":"c1","arguments":{"command":"ls"}}],"usage":{"input":10,"output":20,"cacheRead":100,"cacheWrite":50,"totalTokens":180}}}
"#;
        let c2 = r#"{"type":"session","version":3,"id":"b","timestamp":"2026-07-30T11:00:00Z","cwd":"/tmp"}
{"type":"message","message":{"role":"assistant","content":[{"type":"text","text":"hi"}],"usage":{"input":5,"output":6,"cacheRead":7,"cacheWrite":3,"totalTokens":21}}}
"#;
        std::fs::write(&f1, c1).expect("書き込み失敗");
        std::fs::write(&f2, c2).expect("書き込み失敗");

        let stats = extract_session_stats(&dir).expect("統計抽出に成功するべき");
        // 2ファイル跨いで tool と message の cache が合算される
        assert_eq!(stats.token_usage_by_category.tool.cache, 150);
        assert_eq!(stats.token_usage_by_category.message.cache, 10);

        cleanup_session_dir(&dir);
    }

    // ── R2: 既存フィールドの回帰なし ──
    // （既存の基本・ツール・複数ターン・thinking テストがそのまま対象）

    #[test]
    fn test_extract_session_stats_no_usage_fallback() {
        // セッション dir はプロジェクトルート配下に作られるため、
        // AGENT_LINK_PROJECT_ROOT を書き換える他モジュールのテストと干渉しないよう直列化する
        // （干渉するとディレクトリが途中で消えて統計が 0 になり、偶発失敗していた）
        let _env = crate::common::AGENT_LINK_ENV_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // usage が無い行は文字数近似でカテゴリを補完し、cache は 0（R1 の方針）
        let dir = create_session_dir("no-usage").expect("ディレクトリ作成失敗");
        let session_path = dir.join("test.jsonl");

        let content = r#"{"type":"session","version":3,"id":"test","timestamp":"2026-07-30T10:00:00Z","cwd":"/tmp"}
{"type":"message","message":{"role":"user","content":[{"type":"text","text":"Hello"}]}}
{"type":"message","message":{"role":"assistant","content":[{"type":"thinking","thinking":"think"},{"type":"text","text":"answer"}]}}
"#;
        std::fs::write(&session_path, content).expect("テストファイルの書き込みに失敗");

        let stats = extract_session_stats(&dir).expect("統計抽出に成功するべき");
        let b = &stats.token_usage_by_category;

        // message: user "Hello"(5) + assistant "answer"(6)
        assert_eq!(b.message.input, 5);
        assert_eq!(b.message.output, 6);
        // think: "think"(5)
        assert_eq!(b.think.output, 5);
        // cache は 0
        assert_eq!(b.message.cache, 0);
        assert_eq!(b.think.cache, 0);

        cleanup_session_dir(&dir);
    }

    // ── R5: 時系列イベント抽出 ──

    #[test]
    fn test_extract_timeline_events() {
        // セッション dir はプロジェクトルート配下に作られるため、
        // AGENT_LINK_PROJECT_ROOT を書き換える他モジュールのテストと干渉しないよう直列化する
        // （干渉するとディレクトリが途中で消えて統計が 0 になり、偶発失敗していた）
        let _env = crate::common::AGENT_LINK_ENV_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = create_session_dir("timeline").expect("ディレクトリ作成失敗");
        let session_path = dir.join("test.jsonl");

        let content = r#"{"type":"session","version":3,"id":"test","timestamp":"2026-07-30T10:00:00Z","cwd":"/tmp"}
{"type":"message","timestamp":"2026-07-30T10:01:00Z","message":{"role":"user","content":[{"type":"text","text":"Make a change"}]}}
{this is not valid json
{"type":"message","timestamp":"2026-07-30T10:02:00Z","message":{"role":"assistant","content":[{"type":"toolCall","name":"bash","id":"c1","arguments":{"command":"git commit -m init"}}]}}
{"type":"message","timestamp":"2026-07-30T10:03:00Z","message":{"role":"assistant","content":[{"type":"toolCall","name":"read","id":"c2","arguments":{"path":"/a"}}]}}
"#;
        std::fs::write(&session_path, content).expect("テストファイルの書き込みに失敗");

        let events = extract_timeline_events(&dir).expect("時系列抽出に成功するべき");

        // パース不能行を無視し、3 イベントのみ
        assert_eq!(events.len(), 3);
        // 時刻順にソートされている
        assert!(events.windows(2).all(|w| w[0].timestamp <= w[1].timestamp));

        // ユーザーメッセージ
        assert_eq!(events[0].kind, "user");
        assert_eq!(events[0].detail, "Make a change");
        // git 相当イベント（bash で git コマンド）
        assert_eq!(events[1].kind, "git");
        assert!(events[1].detail.contains("git commit"));
        // 通常のツール呼び出し
        assert_eq!(events[2].kind, "toolCall");
        assert!(events[2].detail.contains("read"));

        cleanup_session_dir(&dir);
    }
}

#[cfg(test)]
mod usage_tests {
    use super::*;

    /// usage 付きの assistant 応答を含むテスト用 jsonl を書き出す。
    fn write_usage_jsonl(dir: &std::path::Path) -> PathBuf {
        let path = dir.join("session_01a04b0e-0000-0000-0000-000000000001.jsonl");
        std::fs::write(
            &path,
            concat!(
                r#"{"type":"session","version":3,"id":"abc","timestamp":"2026-08-29T01:00:00.000Z"}"#, "\n",
                // turn 1: text のみ → message に 100% 計上
                r#"{"type":"message","timestamp":"2026-08-29T01:00:01.000Z","message":{"role":"user","content":[{"type":"text","text":"質問です"}]}}"#, "\n",
                r#"{"type":"message","timestamp":"2026-08-29T01:00:02.000Z","message":{"role":"assistant","content":[{"type":"text","text":"回答"}],"usage":{"input":100,"output":10,"cacheRead":0,"cacheWrite":0,"cost":{"total":0.001}}}}"#, "\n",
                // turn 2: toolResult → thinking → toolCall 付き応答
                r#"{"type":"message","timestamp":"2026-08-29T01:00:03.000Z","message":{"role":"toolResult","toolCallId":"t0","toolName":"grep","content":[{"type":"text","text":"結果テキスト"}]}}"#, "\n",
                r#"{"type":"message","timestamp":"2026-08-29T01:00:04.000Z","message":{"role":"assistant","content":[{"type":"thinking","thinking":"思考思考思考"},{"type":"toolCall","id":"t1","name":"grep","arguments":{"q":"llm"}}],"usage":{"input":200,"output":90,"cacheRead":50,"cacheWrite":10,"cost":{"total":0.002}}}}"#, "\n",
            ),
        )
        .unwrap();
        path
    }

    /// provider usage の合計 + 種別 × ターン別内訳が 1 回の走査で取れること。
    #[test]
    fn test_extract_execution_usage_total_and_breakdown() {
        // セッション dir はプロジェクトルート配下に作られるため、
        // AGENT_LINK_PROJECT_ROOT を書き換える他モジュールのテストと干渉しないよう直列化する
        // （干渉するとディレクトリが途中で消えて統計が 0 になり、偶発失敗していた）
        let _env = crate::common::AGENT_LINK_ENV_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        write_usage_jsonl(dir.path());

        let scan = extract_execution_usage(dir.path()).unwrap();

        // total = 生の provider 値の合計（正）
        assert_eq!(scan.total.input, 300);
        assert_eq!(scan.total.output, 100);
        assert_eq!(scan.total.cache_read, 50);
        assert_eq!(scan.total.cache_write, 10);
        assert!((scan.total.cost_total - 0.003).abs() < 1e-9);

        // ターン番号は assistant 応答の通し番号
        let turns: Vec<u64> = scan
            .breakdown
            .iter()
            .map(|b| b.turn_no)
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect();
        assert_eq!(turns, vec![1, 2]);

        // turn 1 は text のみ → message に 100% 計上
        let t1: Vec<_> = scan.breakdown.iter().filter(|b| b.turn_no == 1).collect();
        assert_eq!(t1.len(), 1);
        assert_eq!(t1[0].entry_type, crate::db::models::USAGE_ENTRY_MESSAGE);
        assert_eq!(t1[0].amount.input, 100);

        // turn 2 は thinking / toolCall / toolResult に振り分けられる（推定）
        let t2: Vec<_> = scan.breakdown.iter().filter(|b| b.turn_no == 2).collect();
        let kinds: std::collections::BTreeSet<&str> =
            t2.iter().map(|b| b.entry_type.as_str()).collect();
        assert!(kinds.contains(crate::db::models::USAGE_ENTRY_THINKING));
        assert!(kinds.contains(crate::db::models::USAGE_ENTRY_TOOL_CALL));
        assert!(kinds.contains(crate::db::models::USAGE_ENTRY_TOOL_RESULT));
    }

    /// 内訳の合計 == total（largest remainder による合計保存）。
    #[test]
    fn test_extract_execution_usage_breakdown_sums_match_total() {
        // セッション dir はプロジェクトルート配下に作られるため、
        // AGENT_LINK_PROJECT_ROOT を書き換える他モジュールのテストと干渉しないよう直列化する
        // （干渉するとディレクトリが途中で消えて統計が 0 になり、偶発失敗していた）
        let _env = crate::common::AGENT_LINK_ENV_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        write_usage_jsonl(dir.path());

        let scan = extract_execution_usage(dir.path()).unwrap();
        let sum = scan
            .breakdown
            .iter()
            .fold(UsageAmount::default(), |mut a, b| {
                a.add(&b.amount);
                a
            });
        assert_eq!(sum.input, scan.total.input);
        assert_eq!(sum.output, scan.total.output);
        assert_eq!(sum.cache_read, scan.total.cache_read);
        assert_eq!(sum.cache_write, scan.total.cache_write);
        assert!((sum.cost_total - scan.total.cost_total).abs() < 1e-9);
    }

    /// usage 無しの応答のみの場合は total 0・内訳なし。
    #[test]
    fn test_extract_execution_usage_no_usage() {
        // セッション dir はプロジェクトルート配下に作られるため、
        // AGENT_LINK_PROJECT_ROOT を書き換える他モジュールのテストと干渉しないよう直列化する
        // （干渉するとディレクトリが途中で消えて統計が 0 になり、偶発失敗していた）
        let _env = crate::common::AGENT_LINK_ENV_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("session_01a04b0e-0000-0000-0000-000000000002.jsonl"),
            concat!(
                r#"{"type":"session","version":3,"id":"x"}"#, "\n",
                r#"{"type":"message","message":{"role":"assistant","content":[{"type":"text","text":"usage 無し"}]}}"#, "\n",
            ),
        )
        .unwrap();
        let scan = extract_execution_usage(dir.path()).unwrap();
        assert_eq!(scan.total.input, 0);
        assert!(scan.breakdown.is_empty());
    }

    /// History 画面用の近似統計（種別別トークン + Timeline）。
    #[test]
    fn test_extract_history_stats() {
        // セッション dir はプロジェクトルート配下に作られるため、
        // AGENT_LINK_PROJECT_ROOT を書き換える他モジュールのテストと干渉しないよう直列化する
        // （干渉するとディレクトリが途中で消えて統計が 0 になり、偶発失敗していた）
        let _env = crate::common::AGENT_LINK_ENV_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("session_01a04b0e-0000-0000-0000-000000000003.jsonl"),
            concat!(
                r#"{"type":"session","version":3,"id":"x"}"#, "\n",
                r#"{"type":"message","timestamp":"2026-08-29T01:00:01.000Z","message":{"role":"user","content":[{"type":"text","text":"こんにちは\n質問"}]}}"#, "\n",
                r#"{"type":"message","timestamp":"2026-08-29T01:00:02.000Z","message":{"role":"assistant","content":[{"type":"thinking","thinking":"思考"},{"type":"text","text":"回答"},{"type":"toolCall","id":"t1","name":"grep","arguments":{"q":"llm"}}]}}"#, "\n",
                r#"{"type":"message","timestamp":"2026-08-29T01:00:03.000Z","message":{"role":"toolResult","toolCallId":"t1","toolName":"grep","content":[{"type":"text","text":"結果"}]}}"#, "\n",
                "not json\n",
            ),
        )
        .unwrap();

        let stats = extract_history_stats(dir.path()).unwrap();
        let t = &stats.type_tokens;
        // user: 「こんにちは\n質問」= 8 文字（改行を含む生の文字数・近似値）
        assert_eq!(t.user, 8);
        // agent: 「回答」= 2 文字
        assert_eq!(t.agent, 2);
        // thinking: 「思考」= 2 文字
        assert_eq!(t.thinking, 2);
        // toolCall: "grep" + {"q":"llm"} = 4 + 11 文字
        assert_eq!(t.tool_call, 15);
        // toolResult: 「結果」= 2 文字
        assert_eq!(t.tool_result, 2);

        // Timeline: user → thinking → agent → toolCall（結果は同じ行に入る・C11）
        let kinds: Vec<&str> = stats.events.iter().map(|e| e.kind.as_str()).collect();
        assert_eq!(kinds, vec!["user", "thinking", "agent", "toolCall"]);
        assert_eq!(stats.events[0].name, "こんにちは 質問");
        assert_eq!(stats.events[3].name, "grep");
        assert_eq!(stats.events[3].call_id.as_deref(), Some("t1"));
        assert_eq!(stats.events[3].preview.as_deref(), Some("結果"));
        assert_eq!(stats.events[3].result_tokens, Some(2));
        assert_eq!(stats.events[2].name, "回答");
    }

    /// Claude Code の JSONL から統計・出力・トランスクリプトを抽出する。
    #[test]
    fn test_extract_claude_session() {
        // セッション dir はプロジェクトルート配下に作られるため、
        // AGENT_LINK_PROJECT_ROOT を書き換える他モジュールのテストと干渉しないよう直列化する
        // （干渉するとディレクトリが途中で消えて統計が 0 になり、偶発失敗していた）
        let _env = crate::common::AGENT_LINK_ENV_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("11111111-2222-4333-8444-555555555555.jsonl");
        std::fs::write(
            &path,
            concat!(
                r#"{"type":"user","timestamp":"2026-09-17T01:00:00.000Z","message":{"role":"user","content":[{"type":"text","text":"やあ"}]}}"#, "\n",
                r#"{"type":"assistant","timestamp":"2026-09-17T01:00:01.000Z","message":{"role":"assistant","model":"claude-sonnet-4-5","usage":{"input_tokens":100,"output_tokens":20,"cache_read_input_tokens":30,"cache_creation_input_tokens":10},"content":[{"type":"text","text":"回答A"}]}}"#, "\n",
                r#"{"type":"assistant","timestamp":"2026-09-17T01:00:02.000Z","message":{"role":"assistant","model":"claude-sonnet-4-5","usage":{"input_tokens":50,"output_tokens":10},"content":[{"type":"tool_use","name":"Bash","input":{}}]}}"#, "\n",
                r#"{"type":"assistant","timestamp":"2026-09-17T01:00:03.000Z","message":{"role":"assistant","usage":{"input_tokens":5,"output_tokens":5},"content":[{"type":"text","text":"最終回答"}]}}"#, "\n",
            ),
        )
        .unwrap();

        let stats = extract_claude_session_stats(&path).unwrap();
        assert_eq!(stats.input_tokens, 155);
        assert_eq!(stats.output_tokens, 35);
        assert_eq!(stats.tool_call_count, 1);
        assert_eq!(stats.model_name.as_deref(), Some("claude-sonnet-4-5"));
        assert_eq!(stats.started_at.as_deref(), Some("2026-09-17T01:00:00.000Z"));
        assert_eq!(stats.finished_at.as_deref(), Some("2026-09-17T01:00:03.000Z"));

        assert_eq!(extract_claude_final_output(&path).unwrap(), "最終回答");
        let transcript = extract_claude_full_transcript(&path);
        assert!(transcript.contains("**user**: やあ"));
        assert!(transcript.contains("回答A"));
        assert!(transcript.contains("[tool: Bash]"));
    }

    /// sai-gemini の JSONL から user / assistant を抽出すること。
    #[test]
    fn gemini_messages_and_history_stats() {
        let dir = std::env::temp_dir().join(format!("sai_gemini_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("tui-20260922-000000.jsonl");
        let body = concat!(
            r#"{"id":"tui-20260922-000000","timestamp":"2026-09-22T00:00:00.000000+00:00","title":"テスト会話","type":"session"}"#,
            "\n",
            r#"{"message":{"content":[{"text":"こんにちは","type":"text"}],"role":"user"},"timestamp":"2026-09-22T00:00:10.000000+00:00","type":"message"}"#,
            "\n",
            r#"{"type":"meta","geminiUrl":"https://gemini.google.com/app/abc"}"#,
            "\n",
            r#"{"message":{"content":[{"text":"お返事です","type":"text"}],"role":"assistant"},"timestamp":"2026-09-22T00:00:20.000000+00:00","type":"message"}"#,
            "\n",
        );
        std::fs::write(&path, body).unwrap();
        // message 画面用
        let msgs = extract_gemini_user_assistant_messages(&path).unwrap();
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0], ("user".to_string(), "こんにちは".to_string()));
        assert_eq!(msgs[1], ("assistant".to_string(), "お返事です".to_string()));
        // History 画面用
        let hist = extract_gemini_history_stats(&path).unwrap();
        assert_eq!(hist.type_tokens.user, 5); // 「こんにちは」5文字
        assert_eq!(hist.type_tokens.agent, 5); // 「お返事です」5文字
        assert_eq!(hist.events.len(), 2);
        assert_eq!(hist.events[0].kind, "user");
        assert_eq!(hist.events[1].kind, "agent");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Claude Code（cc）の JSONL から History 統計を抽出すること。
    #[test]
    fn cc_extract_history_stats() {
        let dir = std::env::temp_dir().join(format!("sai_cc_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("test-cc.jsonl");
        // cc の実データ構造: type="user"|"assistant", message.usage は snake_case
        let body = concat!(
            r#"{"type":"user","message":{"role":"user","content":[{"type":"text","text":"こんにちは"}]},"timestamp":"2026-09-22T00:00:00Z"}"#,
            "\n",
            r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"お返事です"}],"usage":{"input_tokens":5,"output_tokens":5,"cache_read_input_tokens":0,"cache_creation_input_tokens":0}},"timestamp":"2026-09-22T00:00:01Z"}"#,
            "\n",
        );
        std::fs::write(&path, body).unwrap();
        // History 画面用統計抽出
        let hist = extract_cc_history_stats(&path).unwrap();
        assert_eq!(hist.type_tokens.user, 5); // user テキスト「こんにちは」5文字
        assert_eq!(hist.type_tokens.agent, 10); // input_tokens(5) + output_tokens(5)
        assert_eq!(hist.events.len(), 2);
        assert_eq!(hist.events[0].kind, "user");
        assert_eq!(hist.events[1].kind, "agent");
        assert_eq!(hist.events[1].tokens, 10);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Claude Code（cc）の JSONL から message 画面用の user/assistant テキストを
    /// 抽出できること。実データでは user の content が素の文字列（配列ではない）で
    /// 入っているため、その形式を明示的に検証する。
    #[test]
    fn cc_extract_user_assistant_messages_handles_string_content() {
        let dir = std::env::temp_dir().join(format!("sai_cc_msg_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("test-cc-message.jsonl");
        let body = concat!(
            r#"{"type":"user","message":{"role":"user","content":"こんにちは"},"timestamp":"2026-09-22T00:00:00Z"}"#,
            "\n",
            r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"thinking","thinking":""},{"type":"text","text":"お返事です"}]},"timestamp":"2026-09-22T00:00:01Z"}"#,
            "\n",
        );
        std::fs::write(&path, body).unwrap();
        let msgs = extract_cc_user_assistant_messages(&path).unwrap();
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0], ("user".to_string(), "こんにちは".to_string()));
        assert_eq!(msgs[1], ("assistant".to_string(), "お返事です".to_string()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// cc の tool_use と tool_result が 1 行にまとまること（C11）。
    #[test]
    fn cc_history_pairs_tool_use_and_result() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cc-pair.jsonl");
        // 結果は 200 字を超える
        let result = format!("Exit code 2\n{}\n引数の誤り", "あ".repeat(250));
        let lines = [
            serde_json::json!({"type": "user", "timestamp": "2026-10-08T00:00:00Z", "message": {"role": "user", "content": "依頼文 #12.3"}}),
            serde_json::json!({"type": "assistant", "timestamp": "2026-10-08T00:00:01Z", "message": {"role": "assistant", "usage": {"input_tokens": 5, "output_tokens": 5}, "content": [{"type": "tool_use", "id": "toolu_1", "name": "Bash", "input": {"command": "cd /x && AGENT_LINK_X=1 /opt/bin/agent-link doc read AXANSMpo # 見た #37.5.2"}}]}}),
            serde_json::json!({"type": "user", "timestamp": "2026-10-08T00:00:02Z", "message": {"role": "user", "content": [{"type": "tool_result", "tool_use_id": "toolu_1", "content": result.clone(), "is_error": true}]}}),
        ];
        let body: String = lines.iter().map(|l| format!("{}\n", l)).collect();
        std::fs::write(&path, body).unwrap();

        let hist = extract_cc_history_stats(&path).unwrap();
        // 文字列の user 行も数える（「依頼文 #12.3」= 9 字）
        assert_eq!(hist.type_tokens.user, 9);
        // 1 行にまとまる（対になる toolResult の行は作らない）
        let kinds: Vec<&str> = hist.events.iter().map(|e| e.kind.as_str()).collect();
        assert_eq!(kinds, vec!["user", "toolCall"]);
        let call = &hist.events[1];
        assert_eq!(call.call_id.as_deref(), Some("toolu_1"));
        assert_eq!(call.command.as_deref(), Some("agent-link doc read"));
        assert_eq!(call.is_error, Some(true));
        assert_eq!(
            call.preview.as_deref().map(|p| p.chars().count()),
            Some(200)
        );
        // 失敗の分類は結果の全文の終了コードで読む
        assert_eq!(call.failure, Some("input"));
        let result_chars = result.chars().count() as u64;
        assert_eq!(call.result_tokens, Some(result_chars));
        assert_eq!(hist.type_tokens.tool_result, result_chars);
        // toolCall の tokens は名前と入力 JSON の文字数（tool_call の合計と同じ）
        assert_eq!(hist.type_tokens.tool_call, call.tokens);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// pi の toolCall と toolResult を call id で結ぶこと（C11）。対の無い結果は toolResult の行。
    #[test]
    fn pi_history_pairs_by_tool_call_id() {
        let dir = tempfile::tempdir().unwrap();
        let body = concat!(
            r#"{"type":"message","timestamp":"2026-10-08T00:00:01.000Z","message":{"role":"assistant","content":[{"type":"toolCall","id":"p1","name":"bash","arguments":{"command":"agent-link doc read AXANSMpo --json"}}]}}"#,
            "\n",
            r#"{"type":"message","timestamp":"2026-10-08T00:00:02.000Z","message":{"role":"toolResult","toolCallId":"p1","toolName":"bash","isError":true,"content":[{"type":"text","text":"Exit code 1\n失敗"}]}}"#,
            "\n",
            r#"{"type":"message","timestamp":"2026-10-08T00:00:03.000Z","message":{"role":"toolResult","toolCallId":"zz","toolName":"read","content":[{"type":"text","text":"対がない"}]}}"#,
            "\n",
        );
        std::fs::write(
            dir.path()
                .join("session_01a04b0e-0000-0000-0000-000000000004.jsonl"),
            body,
        )
        .unwrap();

        let stats = extract_history_stats(dir.path()).unwrap();
        let kinds: Vec<&str> = stats.events.iter().map(|e| e.kind.as_str()).collect();
        assert_eq!(kinds, vec!["toolCall", "toolResult"]);
        let call = &stats.events[0];
        assert_eq!(call.call_id.as_deref(), Some("p1"));
        assert_eq!(call.command.as_deref(), Some("agent-link doc read"));
        assert_eq!(call.is_error, Some(true));
        assert_eq!(call.failure, Some("other"));
        assert_eq!(call.preview.as_deref(), Some("Exit code 1\n失敗"));
        assert_eq!(stats.events[1].name, "read");
        assert_eq!(stats.events[1].is_error, None);
        // tool_result = 「Exit code 1\n失敗」14 字 + 「対がない」4 字
        assert_eq!(stats.type_tokens.tool_result, 14 + 4);
    }

    /// Bash の命令名（`agent-link`・`agent-link-web` の命令の位置）を作ること（C11）。
    #[test]
    fn cli_command_forms() {
        let cmd = |s: &str| cli_command("Bash", &serde_json::json!({ "command": s }));
        assert_eq!(
            cmd("cd /x && AGENT_LINK_X=1 /opt/bin/agent-link doc read AXANSMpo --json").as_deref(),
            Some("agent-link doc read")
        );
        assert_eq!(
            cmd("agent-link task create < task.json").as_deref(),
            Some("agent-link task create")
        );
        assert_eq!(
            cmd("agent-link run history 49 --json && echo agent-link-web").as_deref(),
            Some("agent-link run history")
        );
        assert_eq!(
            cmd("agent-link doc list | head").as_deref(),
            Some("agent-link doc list")
        );
        assert_eq!(cmd("grep -n agent-link src"), None);
        assert_eq!(
            cli_command("Read", &serde_json::json!({ "file_path": "/x" })),
            None
        );
    }

    /// 命令の位置の見落とし（改行・heredoc・前置き・括弧・注釈）と `--help`・入力の `agent-link*`（U04）。
    #[test]
    fn cli_command_unlisted_forms() {
        let cmd = |s: &str| cli_command("Bash", &serde_json::json!({ "command": s }));
        let cases = [
            ("agent-link task create <<'EOF'\n{\"task_text\":\"hypsarrhythmia は\"}\nEOF", "agent-link task create"),
            ("cat <<EOF | agent-link task create\nEOF", "agent-link task create"),
            ("timeout 60 agent-link run list --json", "agent-link run list"),
            ("env -u X AGENT_LINK_A=1 agent-link run show 3", "agent-link run show"),
            // 引用符なしの `x` は命令名に入る（3 語まで）ので、引用符で囲む
            ("(agent-link doc list 'x')", "agent-link doc list"),
            ("echo $(agent-link-web usage)", "agent-link-web usage"),
            ("find . | xargs agent-link doc read", "agent-link doc read"),
            ("exec agent-link run show 3", "agent-link run show"),
            ("perl -e 'alarm 180; exec @ARGV' agent-link doc read", "agent-link doc read"),
            ("agent-link run list # it's a note\nagent-link run show 3", "agent-link run list"),
            ("echo a\nagent-link run show 3", "agent-link run show"),
        ];
        for (input, want) in cases {
            assert_eq!(cmd(input).as_deref(), Some(want), "入力: {input}");
        }
        // heredoc の本文は命令でない（本文だけの agent-link* は命令名にならない）
        assert_eq!(cmd("cat > x.md <<EOF\nagent-link doc read\nEOF"), None);

        let info = |s: &str| cli_info("Bash", &serde_json::json!({ "command": s }));
        assert!(info("agent-link run finalize --help").help);
        assert!(info("agent-link doc read -h").help);
        assert!(!info("agent-link doc read --json").help);
        assert!(info("grep agent-link-web src").mentions);
        assert!(!info("grep sai src").mentions);
        assert!(!info("cat <<EOF\nagent-link doc read\nEOF").mentions);
    }

    /// 失敗の分類と終了コードの読み取り（C11）。
    #[test]
    fn failure_class_reads_exit_code() {
        assert_eq!(exit_code_of("Exit code 3\nx"), Some(3));
        assert_eq!(exit_code_of("exit CODE 2"), Some(2));
        assert_eq!(exit_code_of("x"), None);
        assert_eq!(failure_class("Exit code 3\n見つからない"), "not_found");
        assert_eq!(failure_class("Exit code 2\n引数の誤り"), "input");
        assert_eq!(failure_class("Exit code 1\n何か"), "other");
    }

    /// `history_call` が結果を 20000 字で切ること・無い呼び出しは None（C11）。
    #[test]
    fn history_call_truncates_at_20000() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cc-call.jsonl");
        let lines = [
            serde_json::json!({"type": "assistant", "message": {"role": "assistant", "content": [{"type": "tool_use", "id": "c9", "name": "Bash", "input": {"command": "agent-link run history 49 --json"}}]}}),
            serde_json::json!({"type": "user", "message": {"role": "user", "content": [{"type": "tool_result", "tool_use_id": "c9", "content": "a".repeat(25000), "is_error": false}]}}),
        ];
        let body: String = lines.iter().map(|l| format!("{}\n", l)).collect();
        std::fs::write(&path, body).unwrap();

        let call = history_call(&path, "cc", "c9").unwrap();
        assert_eq!(call.command.as_deref(), Some("agent-link run history"));
        assert_eq!(call.result.chars().count(), 20000);
        assert!(call.truncated);
        assert_eq!(call.is_error, Some(false));
        assert!(history_call(&path, "cc", "nope").is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
