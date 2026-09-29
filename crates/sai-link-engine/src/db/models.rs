//! データベースモデル定義
//!
//! `block_metadata` / `block_inputs` / `block_output` / `execution_history` /
//! `git_commits` の 5 テーブルに対応する Rust 構造体を定義します。
//! 各構造体は `serde::Serialize` / `serde::Deserialize` を derive し、
//! `thiserror` を用いたエラー型も提供します。

use serde::{Deserialize, Serialize};
use thiserror::Error;

// ---------------------------------------------------------------------------
// DbError — データベース操作のエラー型
// ---------------------------------------------------------------------------

/// データベース操作におけるエラー型。
///
/// `rusqlite::Error` と `serde_json::Error` をラップし、
/// レコード未存在や重複名などのドメインエラーも表現します。
#[derive(Error, Debug)]
pub enum DbError {
    /// レコードが見つからない（`get_*` 系操作で該当行がない場合）
    #[error("レコードが見つかりません: {0}")]
    NotFound(String),

    /// 重複した名前（`name` UNIQUE 制約違反）
    #[error("重複したブロック名です: '{0}'")]
    DuplicateName(String),

    /// SQLite エラーのラッパー
    #[error("データベースエラー: {0}")]
    Rusqlite(#[from] rusqlite::Error),

    /// JSON シリアライズ/デシリアライズエラーのラッパー
    #[error("JSON エラー: {0}")]
    Json(#[from] serde_json::Error),

    /// 無効な引数
    #[error("無効な引数: {0}")]
    InvalidArgument(String),
}

// ---------------------------------------------------------------------------
// BlockMetadata — ブロックメタデータ
// ---------------------------------------------------------------------------

/// `block_metadata` テーブルの 1 行に対応する構造体。
///
/// ブロック定義ファイル（`.md`）の YAML frontmatter から抽出された
/// エージェント設定を永続化します。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockMetadata {
    /// 主キー（自動採番、挿入前は `None`）
    pub id: Option<i64>,
    /// ブロックの機械可読 ID（`[a-z0-9_-]+` パターン、UNIQUE）
    pub name: String,
    /// ブロックの表示名（sai-tui 表示用。emoji 可。`name` と同じでもよい）
    pub display_name: String,
    /// ブロックの説明
    pub description: String,
    /// 使用する LLM モデル（完全修飾名または "inherit"）
    pub model: String,
    /// LLM プロバイダ（オプション）
    pub provider: Option<String>,
    /// thinking budget レベル
    pub thinking: String,
    /// 使用可能ツール設定（JSON 文字列または "true"/"false"/カンマ区切り）
    pub tools: String,
    /// 使用可能拡張機能設定（JSON 文字列または "true"/"false"/カンマ区切り）
    pub extensions: String,
    /// ロードするスキル（カンマ区切り文字列）
    pub skills: Option<String>,
    /// タイムアウト秒数
    pub timeout: i32,
    /// Git worktree 分離設定
    pub worktree: String,
    /// ブロック有効フラグ
    pub enable: bool,
    /// ブロック定義ファイルのパス
    pub file_path: String,
    /// ファイル内容の SHA-256 ハッシュ（変更検出用）
    pub body_hash: Option<String>,
    /// セクションタグ（例: "brst", "plan", "code", "comp", "rfrs"）
    pub tag: Option<String>,
    /// ラウンド番号（1始まり）
    pub round: Option<i64>,
    /// research-tree の層（`root` / `seed` / `branch`。無ければ図に出さない）
    pub layer: Option<String>,
    /// 作成日時（ISO 8601）
    pub created_at: String,
    /// 更新日時（ISO 8601）
    pub updated_at: String,
}

impl BlockMetadata {
    /// 新しい `BlockMetadata` を作成する。
    ///
    /// `id` は自動採番のため `None` に設定される。
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        name: String,
        description: String,
        model: String,
        provider: Option<String>,
        thinking: String,
        tools: String,
        extensions: String,
        skills: Option<String>,
        timeout: i32,
        worktree: String,
        enable: bool,
        file_path: String,
        body_hash: Option<String>,
    ) -> Self {
        Self {
            id: None,
            display_name: name.clone(),
            name,
            description,
            model,
            provider,
            thinking,
            tools,
            extensions,
            skills,
            timeout,
            worktree,
            enable,
            file_path,
            body_hash,
            tag: None,
            round: None,
            layer: None,
            created_at: String::new(), // DB デフォルト値で設定
            updated_at: String::new(), // DB デフォルト値で設定
        }
    }
}

// ---------------------------------------------------------------------------
// BlockInput — ブロック入力定義
// ---------------------------------------------------------------------------

/// `block_inputs` テーブルの 1 行に対応する構造体。
///
/// ブロック実行時にユーザーに入力を求めるフィールドの定義を格納します。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockInput {
    /// 主キー（自動採番、挿入前は `None`）
    pub id: Option<i64>,
    /// 所属ブロックの ID（外部キー）
    pub block_id: i64,
    /// フィールド識別子（`[a-z0-9_]+` パターン）
    pub input_id: String,
    /// 入力タイプ（"text", "select", "confirm", "file_token"）
    pub input_type: String,
    /// 表示ラベル
    pub label: String,
    /// ヘルプテキスト
    pub description: Option<String>,
    /// 必須入力フラグ
    pub required: bool,
    /// デフォルト値
    pub default_value: Option<String>,
    /// 選択肢（select タイプ用）JSON 配列文字列
    pub options_json: Option<String>,
    /// 入力元ブロック名（file_token タイプ用）JSON 配列文字列
    pub source_blocks_json: Option<String>,
    /// 入力元カテゴリ（file_token タイプ用）JSON 配列文字列
    pub source_categories_json: Option<String>,
}

impl BlockInput {
    /// 新しい `BlockInput` を作成する。
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        block_id: i64,
        input_id: String,
        input_type: String,
        label: String,
        description: Option<String>,
        required: bool,
        default_value: Option<String>,
        options_json: Option<String>,
        source_blocks_json: Option<String>,
        source_categories_json: Option<String>,
    ) -> Self {
        Self {
            id: None,
            block_id,
            input_id,
            input_type,
            label,
            description,
            required,
            default_value,
            options_json,
            source_blocks_json,
            source_categories_json,
        }
    }
}

// ---------------------------------------------------------------------------
// BlockOutput — ブロック出力定義
// ---------------------------------------------------------------------------

/// `block_output` テーブルの 1 行に対応する構造体。
///
/// ブロック実行後の出力処理設定を格納します。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockOutput {
    /// 主キー（自動採番、挿入前は `None`）
    pub id: Option<i64>,
    /// 所属ブロックの ID（外部キー、UNIQUE）
    pub block_id: i64,
    /// 出力フォーマット（"markdown", "json", "yaml", "code"）
    pub format: String,
    /// waggle トークン化を有効にするか
    pub waggle_tokenize: bool,
    /// 出力の説明
    pub description: Option<String>,
}

impl BlockOutput {
    /// 新しい `BlockOutput` を作成する。
    pub fn new(
        block_id: i64,
        format: String,
        waggle_tokenize: bool,
        description: Option<String>,
    ) -> Self {
        Self {
            id: None,
            block_id,
            format,
            waggle_tokenize,
            description,
        }
    }
}

// ---------------------------------------------------------------------------
// ExecutionHistory — 実行履歴
// ---------------------------------------------------------------------------

/// `runner` カラムのデフォルト（旧 DB 互換・`pi` 扱い）。
fn default_runner() -> String {
    "pi".to_string()
}

/// `execution_history` テーブルの 1 行に対応する構造体。
///
/// ブロックの実行記録を格納します。
/// ステートマシン: `pending` → `running` → `completed` | `failed`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionHistory {
    /// 主キー（自動採番、挿入前は `None`）
    pub id: Option<i64>,
    /// 実行されたブロックの ID
    pub block_id: i64,
    /// 実行ステータス（"pending", "running", "completed", "failed"）
    pub status: String,
    /// ユーザー入力値の JSON（`{"input_id": "value", ...}`）
    pub user_inputs_json: Option<String>,
    /// 解決済みプロンプト（プレースホルダー展開後）
    pub resolved_prompt: Option<String>,
    /// 入力として使った waggle トークン一覧 — JSON配列
    pub input_waggle_tokens: Option<String>,
    /// waggle 出力トークン（`waggle_core::Token::as_str()`） — 単一トークン（後方互換）
    pub output_waggle_token: Option<String>,
    /// waggle 出力トークン一覧 — JSON配列（sai-write で複数ファイル出力時）
    pub output_waggle_tokens: Option<String>,
    /// 出力タイプ: "document" | "code" | "both"
    pub output_type: String,
    /// pi起動時のHEADコミットハッシュ（再現性用）
    pub git_commit_before: Option<String>,
    /// pi終了時のHEADコミットハッシュ（再現性用）
    pub git_commit_after: Option<String>,
    /// herdr pane ID
    pub herdr_pane_id: Option<String>,
    /// エージェントプロセス ID
    pub pid: Option<i32>,
    /// 実行開始日時（ISO 8601）
    pub started_at: Option<String>,
    /// 実行終了日時（ISO 8601）
    pub finished_at: Option<String>,
    /// 実行時間（ミリ秒）
    pub duration_ms: Option<i64>,
    /// プロセス終了コード
    pub exit_code: Option<i32>,
    /// エラーメッセージ
    pub error_message: Option<String>,
    /// エージェント出力の全文（FN-012: 実行完了時に保存）
    pub output_content: Option<String>,
    /// 親実行ID（サブタスクの場合。親実行から作成されたことを示す）
    pub parent_run_id: Option<i64>,
    /// タスクバッチID（task_create の batch_id。DAG のフェーズ合成に使用）
    pub batch_id: Option<String>,
    /// ユーザー明示完了（Ctrl+D / pi-sai-finalize）で finalize 済みか（U4）
    pub finalized: bool,
    /// セッション統計情報の JSON（トークン数・ツール呼び出し回数・ターン数など）
    pub session_stats_json: Option<String>,
    /// 実行時のタグ（入力フォームのオーバーライド値）
    pub tag: Option<String>,
    /// 実行時のラウンド番号（入力フォームのオーバーライド値）
    pub round: Option<i64>,
    /// 実行時のグローバルラウンド番号（GR）。DAG の行グループと区切り線に使用
    pub global_round: Option<i64>,
    /// 実行時のモデル（入力フォームのオーバーライド値）
    pub model: Option<String>,
    /// 実行時の provider（入力フォームのオーバーライド値・`p` 再開用）
    pub provider: Option<String>,
    /// 実行時の thinking budget（入力フォームのオーバーライド値）
    pub thinking: Option<String>,
    /// 実行時のタイムアウト秒数（入力フォームのオーバーライド値）
    pub timeout: Option<i32>,
    /// セッションディレクトリのパス
    pub session_dir: Option<String>,
    /// 実行時の pi 起動コマンド全文
    pub pi_command: Option<String>,
    /// 実行に使ったカード md の相対パス（`.sai/todo/xxx.md`）
    pub kanban_card_path: Option<String>,
    /// 実行経路の記録: `interactive` / `kanban`
    pub execution_mode: Option<String>,
    /// Kanban→Workflow 移動済みか（`d` で設定。ファイルは残す）。
    ///
    /// `false` = Kanban 所有（ボード表示）、`true` = Workflow 所有（DAG 表示）。
    /// 旧 DB（カラムなし）は `false` 扱い。
    #[serde(default)]
    pub workflow_moved: bool,
    /// Kanban のレビュー済みフラグ。`completed` のうち true なら done レーン、
    /// false なら review レーン。run-tui の `e` で明示的に更新する。
    /// 旧 DB（カラムなし）は `false` 扱い。
    #[serde(default)]
    pub review_done: bool,
    /// 実行主体（`pi` / `user`）。`user` はユーザータスク（pi 実行なし）。
    ///
    /// 旧 DB（カラムなし）は `pi` 扱い。
    #[serde(default = "default_runner")]
    pub runner: String,
    /// エージェント側セッション ID（Claude Code の `--session-id` の UUID）。
    ///
    /// pi 実行では `None`（pi は `session_dir` の JSONL パスで再開する）。
    /// 旧 DB（カラムなし）は `None` 扱い。
    #[serde(default)]
    pub agent_session_id: Option<String>,
    /// レコード作成日時（ISO 8601）
    pub created_at: String,
}

impl ExecutionHistory {
    /// 新しい実行履歴レコードを作成する（status = "pending"）。
    pub fn new(block_id: i64, user_inputs_json: Option<String>) -> Self {
        Self {
            id: None,
            block_id,
            status: "pending".to_string(),
            user_inputs_json,
            resolved_prompt: None,
            input_waggle_tokens: None,
            output_waggle_token: None,
            output_waggle_tokens: None,
            output_type: "document".to_string(),
            git_commit_before: None,
            git_commit_after: None,
            herdr_pane_id: None,
            pid: None,
            started_at: None,
            finished_at: None,
            duration_ms: None,
            exit_code: None,
            error_message: None,
            output_content: None,
            parent_run_id: None,
            batch_id: None,
            finalized: false,
            session_stats_json: None,
            tag: None,
            round: None,
            global_round: None,
            model: None,
            provider: None,
            thinking: None,
            timeout: None,
            session_dir: None,
            pi_command: None,
            kanban_card_path: None,
            execution_mode: None,
            workflow_moved: false,
            review_done: false,
            runner: "pi".to_string(),
            agent_session_id: None,
            created_at: String::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// GitCommit — Git コミット管理
// ---------------------------------------------------------------------------

/// `git_commits` テーブルの 1 行に対応する構造体。
///
/// ブロック実行に関連付けられた Git コミット情報を格納します。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitCommit {
    /// 主キー（自動採番、挿入前は `None`）
    pub id: Option<i64>,
    /// 関連する実行履歴の ID
    pub execution_id: i64,
    /// コミットハッシュ（SHA）
    pub commit_hash: String,
    /// コミットメッセージ
    pub commit_message: Option<String>,
    /// コミット作者名
    pub author_name: Option<String>,
    /// コミット作者メール
    pub author_email: Option<String>,
    /// ブランチ名
    pub branch_name: Option<String>,
    /// コミット日時（ISO 8601）
    pub committed_at: String,
}

impl GitCommit {
    /// 新しい `GitCommit` を作成する。
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        execution_id: i64,
        commit_hash: String,
        commit_message: Option<String>,
        author_name: Option<String>,
        author_email: Option<String>,
        branch_name: Option<String>,
    ) -> Self {
        Self {
            id: None,
            execution_id,
            commit_hash,
            commit_message,
            author_name,
            author_email,
            branch_name,
            committed_at: String::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// BlockConnection — ブロック間接続
// ---------------------------------------------------------------------------

/// `block_connections` テーブルの 1 行に対応する構造体。
///
/// ブロックAの出力（waggle トークン / git commit）を
/// ブロックBが入力として使用した関係を記録します。
/// TUI のブロック線図描画に使用されます。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockConnection {
    /// 主キー（自動採番、挿入前は `None`）
    pub id: Option<i64>,
    /// 出力元の実行 ID
    pub source_execution_id: i64,
    /// 出力元のブロック ID
    pub source_block_id: i64,
    /// 入力先の実行 ID
    pub target_execution_id: i64,
    /// 入力先のブロック ID
    pub target_block_id: i64,
    /// 接続タイプ: "waggle_token" | "git_commit" | "task_delegation" | "task_output" | "kanban_origin" | "review"
    pub connection_type: String,
    /// waggle トークン（connection_type = "waggle_token" の場合）
    pub waggle_token: Option<String>,
    /// git コミットハッシュ（connection_type = "git_commit" の場合）
    pub commit_hash: Option<String>,
}

impl BlockConnection {
    /// 新しい waggle_token 接続を作成する。
    pub fn new_waggle(
        source_execution_id: i64,
        source_block_id: i64,
        target_execution_id: i64,
        target_block_id: i64,
        waggle_token: String,
    ) -> Self {
        Self {
            id: None,
            source_execution_id,
            source_block_id,
            target_execution_id,
            target_block_id,
            connection_type: "waggle_token".to_string(),
            waggle_token: Some(waggle_token),
            commit_hash: None,
        }
    }

    /// 新しい task_delegation 接続を作成する（親→サブの委譲関係）。
    pub fn new_task_delegation(
        source_execution_id: i64,
        source_block_id: i64,
        target_execution_id: i64,
        target_block_id: i64,
    ) -> Self {
        Self {
            id: None,
            source_execution_id,
            source_block_id,
            target_execution_id,
            target_block_id,
            connection_type: "task_delegation".to_string(),
            waggle_token: None,
            commit_hash: None,
        }
    }

    /// 新しい task_output 接続を作成する（サブ→親の出力返却関係）。    ///
    /// `pi-sai` の `task_output` によって親ブロックがサブブロックの出力
    /// （waggle トークン）を受け取った場合に作成される。
    /// `task_delegation`（親→サブ）とは逆向きの関係。
    pub fn new_task_output(
        source_execution_id: i64,
        source_block_id: i64,
        target_execution_id: i64,
        target_block_id: i64,
    ) -> Self {
        Self {
            id: None,
            source_execution_id,
            source_block_id,
            target_execution_id,
            target_block_id,
            connection_type: "task_output".to_string(),
            waggle_token: None,
            commit_hash: None,
        }
    }

}

// ---------------------------------------------------------------------------
// Task — サブタスク管理
// ---------------------------------------------------------------------------

/// `tasks` テーブルの 1 行に対応する構造体。
///
/// 親エージェントが `task_create` で作成するサブタスクを永続化します。
/// ステータス遷移:
/// `pending` → `approved` → `queued` / `waiting_dependency` → `in_progress`
///   → `completed`（仮完了）→ `finalized` | `failed` → `deleted`
/// `deleted_by` で削除者（"user" / "main"）を記録する。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    /// 主キー（自動採番、挿入前は `None`）
    pub id: Option<i64>,
    /// 親実行レコードの ID（どの親エージェントが作成したか）
    pub parent_run_id: i64,
    /// タスク件名
    pub subject: String,
    /// タスク詳細
    pub description: String,
    /// ステータス: "pending" | "approved" | "queued" | "waiting_dependency" |
    /// "in_progress" | "completed" | "finalized" | "failed" | "deleted"
    pub status: String,
    /// 使用するブロック定義名
    pub block_type: Option<String>,
    /// ブロック入力値の JSON（`{"input_id": "value", ...}`）
    pub inputs_json: Option<String>,
    /// 依存タスクID配列の JSON（`["task-1", "task-2"]` — client_id）
    pub depends_on_json: Option<String>,
    /// Git worktree 分離設定: "none" | "isolation" | branch_name
    pub worktree: String,
    /// サブタスクの作業フォルダ（絶対パス。None はプロジェクトルート）
    #[serde(default)]
    pub cwd: Option<String>,
    /// 子の時間上限（秒。`task_create` の timeout 引数）
    ///
    /// 超えてもタスクを failed にはせず、親へ `stalled`（reason=timeout）を同じ間隔で
    /// 繰り返し通知する。`None` はブロック定義の `timeout:` に従う。
    #[serde(default)]
    pub timeout: Option<u32>,
    /// 実行レコードの ID（最新ラウンドの実行を指す。task_send で張り替え）
    pub execution_id: Option<i64>,
    /// バッチ確認用グループ ID
    pub batch_id: Option<String>,
    /// クライアント側の一時 ID（task_create で割り当て。carried_over 照合・依存解決用）
    #[serde(default)]
    pub client_id: Option<String>,
    /// 修正ラウンド番号（1 始まり。task_create 再送のたびに増加）
    #[serde(default)]
    pub round: i64,
    /// 置き換え元バッチ ID（修正版バッチ作成時。このタスクがどの旧バッチを置換したか）
    #[serde(default)]
    pub supersedes_batch_id: Option<String>,
    /// 否認/保留時のユーザーコメント（メインブロックへのフィードバック用）
    #[serde(default)]
    pub approval_comment: Option<String>,
    /// task_finalize 済みか（メインブロックが完了確定したか）
    #[serde(default)]
    pub finalized: bool,
    /// 削除者（"user" | "main"）
    #[serde(default)]
    pub deleted_by: Option<String>,
    /// 削除日時（ISO 8601）
    #[serde(default)]
    pub deleted_at: Option<String>,
    /// 所属実行バッチの ID（task_execute が作成・タスクを紐付ける。task_output 逆引き用）
    #[serde(default)]
    pub run_batch_id: Option<i64>,
    /// 作成日時（ISO 8601）
    pub created_at: String,
    /// 更新日時（ISO 8601）
    pub updated_at: String,
}

impl Task {
    /// 新しい `Task` を作成する。
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        parent_run_id: i64,
        subject: String,
        description: String,
        block_type: Option<String>,
        inputs_json: Option<String>,
        depends_on_json: Option<String>,
        worktree: String,
        batch_id: Option<String>,
        client_id: Option<String>,
    ) -> Self {
        Self {
            id: None,
            parent_run_id,
            subject,
            description,
            status: "pending".to_string(),
            block_type,
            inputs_json,
            depends_on_json,
            worktree,
            cwd: None,
            timeout: None,
            execution_id: None,
            batch_id,
            client_id,
            round: 1,
            supersedes_batch_id: None,
            approval_comment: None,
            finalized: false,
            deleted_by: None,
            deleted_at: None,
            run_batch_id: None,
            created_at: String::new(),
            updated_at: String::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// ConnectionGroup
// ---------------------------------------------------------------------------

/// ブロック間接続の集約結果（線図描画用）。
///
/// `list_connections_grouped()` の戻り値として使用される。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectionGroup {
    /// 出力元ブロックの ID
    pub source_block_id: i64,
    /// 入力先ブロックの ID
    pub target_block_id: i64,
    /// 出力元ブロック名
    pub source_name: String,
    /// 入力先ブロック名
    pub target_name: String,
    /// このブロックペア間の接続数
    pub connection_count: i64,
}

// ---------------------------------------------------------------------------
// WaggleTokenEntry — TUI の waggle トークン選択 UI 用
// ---------------------------------------------------------------------------

/// TUI の waggle トークン選択 UI 用のエントリ。
///
/// `list_available_waggle_tokens()` の戻り値として使用され、
/// 完了済み実行の各出力トークンを個別行として表現します。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WaggleTokenEntry {
    /// waggle トークン文字列
    pub token: String,
    /// 出力元ブロック名
    pub block_name: String,
    /// 実行履歴 ID
    pub execution_id: i64,
    /// ラウンド番号
    pub round: Option<i64>,
    /// 実行作成日時（ISO 8601）
    pub created_at: String,
    /// 出力タイプ（"document" | "code" | "both"）
    pub output_type: String,
    /// 実行状態（running / review / completed 等・文書選択の表示用。旧データは空）
    #[serde(default)]
    pub status: String,
}

// ---------------------------------------------------------------------------
// テスト用ヘルパー
// ---------------------------------------------------------------------------

#[cfg(test)]
pub mod test_helpers {
    use super::*;

    /// テスト用の `BlockMetadata` を作成するファクトリ関数。
    pub fn create_test_block(name: &str) -> BlockMetadata {
        BlockMetadata::new(
            name.to_string(),
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
            format!("blocks/{}.md", name),
            None
        )
    }

    /// テスト用の `BlockInput` を作成するファクトリ関数。
    pub fn create_test_input(block_id: i64, input_id: &str) -> BlockInput {
        BlockInput::new(
            block_id,
            input_id.to_string(),
            "text".to_string(),
            format!("フィールド: {}", input_id),
            None,
            true,
            None,
            None,
            None,
            None,
        )
    }

    /// テスト用の `BlockOutput` を作成するファクトリ関数。
    pub fn create_test_output(block_id: i64) -> BlockOutput {
        BlockOutput::new(
            block_id,
            "markdown".to_string(),
            true,
            Some("テスト用出力".to_string()),
        )
    }

    /// テスト用の `ExecutionHistory` を作成するファクトリ関数。
    pub fn create_test_execution(block_id: i64) -> ExecutionHistory {
        ExecutionHistory::new(block_id, None)
    }

    /// テスト用の `GitCommit` を作成するファクトリ関数。
    pub fn create_test_commit(execution_id: i64, hash: &str) -> GitCommit {
        GitCommit::new(
            execution_id,
            hash.to_string(),
            Some("テストコミット".to_string()),
            Some("テスト太郎".to_string()),
            Some("test@example.com".to_string()),
            Some("feature/test".to_string()),
        )
    }
}

// ============================================================================
// DeleteExecutionSummary — 実行削除の結果サマリ
// ============================================================================

/// 実行レコード削除（カスケード）の結果サマリ。
#[derive(Debug, Clone, Default)]
pub struct DeleteExecutionSummary {
    /// 削除した実行レコード数（サブ実行を含む）
    pub executions: usize,
    /// 削除したブロック間接続数
    pub connections: usize,
    /// 削除したタスク数
    pub tasks: usize,
    /// 削除した git コミット数
    pub git_commits: usize,
}

impl DeleteExecutionSummary {
    /// 別のサマリを加算する（サブ実行の再帰削除結果を集計するため）。
    pub fn merge(&mut self, other: DeleteExecutionSummary) {
        self.executions += other.executions;
        self.connections += other.connections;
        self.tasks += other.tasks;
        self.git_commits += other.git_commits;
    }
}

// ============================================================================
// RunBatch — 実行バッチ（メインブロック複製 mb1.x）
// ============================================================================

/// 実行バッチ（メインブロック複製）のライフサイクルステータス。
///
/// - `open`      = task_execute で作成・受付中（タスク投入可能）
/// - `running`   = 一部タスクが実行中
/// - `complete`  = 全タスク完了（sai-write 前）
/// - `finalized` = sai-write / メイン複製確定（履歴表示のみ）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunBatchStatus {
    Open,
    Running,
    Complete,
    Finalized,
}

impl RunBatchStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            RunBatchStatus::Open => "open",
            RunBatchStatus::Running => "running",
            RunBatchStatus::Complete => "complete",
            RunBatchStatus::Finalized => "finalized",
        }
    }

    pub fn from_str(s: &str) -> RunBatchStatus {
        match s {
            "running" => RunBatchStatus::Running,
            "complete" => RunBatchStatus::Complete,
            "finalized" => RunBatchStatus::Finalized,
            _ => RunBatchStatus::Open,
        }
    }
}

/// 実行バッチ（メインブロック複製 mb1.x）を表すレコード。
///
/// `task_execute` が作成し、1 つのメインブロック（parent_run_id）のタスクを
/// 実行バッチとしてまとめる。`seq` はメイン複製の連番（mb1.1 = seq2, mb1.2 = seq3...、
/// seq1 は実メイン mb1 自体に対応）。
/// `mb_execution_id` は接続線の endpoint として使うメイン複製用実行レコード ID。
#[derive(Debug, Clone)]
pub struct RunBatch {
    /// 主キー
    pub id: Option<i64>,
    /// 実メインの実行 ID
    pub parent_run_id: i64,
    /// 実行バッチ ID（E1, E2...）
    pub batch_id: String,
    /// 実行バッチ連番（mb1.x の x に対応。seq1=実メイン）
    pub seq: i64,
    /// ライフサイクルステータス
    pub status: RunBatchStatus,
    /// メイン複製用実行レコード ID（接続 endpoint。None の場合あり）
    pub mb_execution_id: Option<i64>,
    /// 作成日時
    pub created_at: String,
    /// 確定日時
    pub finished_at: Option<String>,
}

impl RunBatch {
    /// 新しい実行バッチを作成する。
    pub fn new(parent_run_id: i64, batch_id: String, seq: i64) -> Self {
        Self {
            id: None,
            parent_run_id,
            batch_id,
            seq,
            status: RunBatchStatus::Open,
            mb_execution_id: None,
            created_at: String::new(),
            finished_at: None,
        }
    }
}

// ---------------------------------------------------------------------------
// 実行 usage（L1: provider が返す実際の token / cost）
// ---------------------------------------------------------------------------

/// `execution_usage` テーブルの 1 行。
///
/// - `entry_type = "total"` の行が 1 本: provider usage の実行合計（コスト計算の正）。
///   `turn_no` は `None`
/// - それ以外の行: 種別 × ターンの内訳（分析用）。provider は assistant 応答単位でしか
///   usage を報告しないため、種別への振り分けはエントリサイズ比による**推定**である
///   （詳細は `crate::session::extract_execution_usage` の doc を参照）
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ExecutionUsageRow {
    /// 主キー（読み取り時のみ設定）
    pub id: Option<i64>,
    /// 実行 ID（`execution_history.id`）
    pub execution_id: i64,
    /// 種別: `total` / `message` / `thinking` / `toolCall` / `toolResult`
    pub entry_type: String,
    /// 種別行のターン番号（1 始まり）。`total` 行は `None`
    pub turn_no: Option<i64>,
    /// 入力トークン
    pub input_tokens: i64,
    /// 出力トークン
    pub output_tokens: i64,
    /// キャッシュ読み込みトークン
    pub cache_read: i64,
    /// キャッシュ書き込みトークン
    pub cache_write: i64,
    /// コスト（USD）
    pub cost_total: f64,
}

/// `entry_type` 定数: 実行合計行。
pub const USAGE_ENTRY_TOTAL: &str = "total";
/// `entry_type` 定数: 通常メッセージ（assistant の text）。
pub const USAGE_ENTRY_MESSAGE: &str = "message";
/// `entry_type` 定数: thinking。
pub const USAGE_ENTRY_THINKING: &str = "thinking";
/// `entry_type` 定数: toolCall。
pub const USAGE_ENTRY_TOOL_CALL: &str = "toolCall";
/// `entry_type` 定数: toolResult。
pub const USAGE_ENTRY_TOOL_RESULT: &str = "toolResult";

impl ExecutionUsageRow {
    /// `total` 行（実行合計）を作成する。
    pub fn total(execution_id: i64) -> Self {
        Self {
            execution_id,
            entry_type: USAGE_ENTRY_TOTAL.to_string(),
            turn_no: None,
            ..Default::default()
        }
    }

    /// 種別行（内訳）を作成する。
    pub fn breakdown(execution_id: i64, entry_type: &str, turn_no: i64) -> Self {
        Self {
            execution_id,
            entry_type: entry_type.to_string(),
            turn_no: Some(turn_no),
            ..Default::default()
        }
    }

    /// キャッシュ合計（cache_read + cache_write）。
    pub fn cache_total(&self) -> i64 {
        self.cache_read + self.cache_write
    }
}
