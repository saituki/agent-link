//! # エージェント実行オーケストレーター
//!
//! ブロック定義ファイルのパースから herdr pane 上でのエージェント実行、
//! プロンプト注入、完了待機、出力抽出、クリーンアップまでの
//! 完全なパイプラインを提供します。
//!
//! ## 2段階の実行モデル
//!
//! TUI からの利用を想定し、パイプラインは「準備」と「実行」の 2 段階に分割されています。
//!
//! ### 段階1: `prepare_pipeline()` — ユーザー確認前の準備
//!
//! ```ignore
//! use agent_link_engine::orchestrator::prepare_pipeline;
//!
//! let prepared = prepare_pipeline(&config)?;
//! // ユーザーに pi コマンドを表示して確認を求める
//! println!("実行コマンド:\n{}", prepared.pi_command);
//! ```
//!
//! ### 段階2: `spawn_pipeline()` — バックグラウンド実行
//!
//! ```ignore
//! use agent_link_engine::orchestrator::spawn_pipeline;
//! use agent_link_engine::herdr::SplitDirection;
//!
//! let handle = spawn_pipeline(prepared, SplitDirection::Right, 300_000, false);
//! // TUI の描画ループ内で進捗をポーリング
//! loop {
//!     let progress = handle.get_progress();
//!     match progress {
//!         agent_link_engine::orchestrator::PipelineProgress::Completed { output, .. } => { break; }
//!         agent_link_engine::orchestrator::PipelineProgress::Failed { .. } => { break; }
//!         _ => { /* 進捗表示を更新 */ }
//!     }
//! }
//! ```
//!
//! ### CLI 互換: `run_pipeline()` — 同期的な一括実行
//!
//! ```ignore
//! use agent_link_engine::orchestrator::run_pipeline;
//!
//! let result = run_pipeline(config)?;
//! println!("出力: {}", result.output);
//! ```

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::block_config::BlockConfig;
use crate::herdr::{HerdrClient, HerdrError, SplitDirection};
use crate::cc_command::CcCommandBuilder;
use crate::placeholder::{
    resolve_placeholder_values, shift_headings, split_fixed_and_variable, PlaceholderValue,
};
use crate::doc_write_notify;
use crate::session::{
    create_session_dir, create_session_file, SessionError,
};
use crate::waggle_integration::WaggleIntegrator;

// ============================================================================
// 型定義
// ============================================================================

/// `timeout` 未指定時の完了待機時間（ミリ秒・30 日 = 実質無期限）。
///
/// timeout はカード frontmatter から廃止済みで、`block.md` に `timeout:` を
/// 書いた場合のみ期限が効く。未指定の実行がファイナライズ待ちのまま
/// `failed` になり、kanban で review レーンへ自動移動するのを防ぐ。
// ponytail: 30 日の擬似無期限。真の無期限が必要なら deadline を Option<Instant> 化する。
pub const NO_TIMEOUT_MS: u64 = 30 * 24 * 60 * 60 * 1000;

/// パイプライン実行設定。
///
/// `run_pipeline()` / `prepare_pipeline()` に渡す設定値です。
#[derive(Debug, Clone)]
pub struct PipelineConfig {
    /// パース済みブロック設定
    pub block_config: BlockConfig,
    /// ブロックプロンプト本文（プレースホルダー未展開）
    pub block_body: String,
    /// 入力フィールド id → 値のマップ（プレースホルダー展開用）
    pub user_inputs: HashMap<String, String>,
    /// 作業ディレクトリ（`None` の場合はプロセス現在ディレクトリを継承）
    pub cwd: Option<PathBuf>,
    /// ペイン分割方向（デフォルト: `SplitDirection::Right`）
    pub direction: SplitDirection,
    /// エージェント完了待機のタイムアウト（ミリ秒、デフォルト: 300,000 = 5分）
    pub timeout_ms: u64,
    /// デバッグ用にペインを維持するかどうか（デフォルト: `false`）
    pub keep_pane: bool,
    /// 実行ID（ペインラベル用。CLI/TUI から設定される）
    pub run_id: Option<i64>,
    /// バッチ実行時に事前作成された herdr タブの ID（U2・R4。None は従来の分割方式）
    pub tab_id: Option<String>,
    /// 動的 pane 配置で決定した分割基準ペインの ID（U2・R4 案X。None は既定の分割方式）
    pub base_pane_id: Option<String>,
    /// 動的 pane 配置で**既に作成済みの pane**（分割は task_manager 側で実施済み）。
    /// Some ならパイプラインは分割をスキップしてこの pane を再利用する（競合防止・案X）。
    pub pane_id: Option<String>,
    /// サブタスクとして実行中かどうか（true の場合、エージェントのプロセスに AGENT_LINK_IS_SUBTASK=1 を付与）
    pub is_subtask: bool,
}

impl Default for PipelineConfig {
    fn default() -> Self {
        let block_config = BlockConfig {
            name: "default".to_string(),
            description: None,
            model: None,
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
        };
        Self {
            block_config,
            block_body: String::new(),
            user_inputs: HashMap::new(),
            cwd: None,
            direction: SplitDirection::Right,
            timeout_ms: NO_TIMEOUT_MS,
            keep_pane: false,
            run_id: None,
            tab_id: None,
            base_pane_id: None,
            pane_id: None,
            is_subtask: false,
        }
    }
}

/// パイプライン実行結果（完了時のみ）。
#[derive(Debug, Clone)]
pub struct PipelineResult {
    /// エージェントの最終出力テキスト（TUI確認用）
    pub output: String,
    /// 終了コード（0 = 成功、1 = エラー、130 = ユーザーキャンセル）
    pub exit_code: i32,
    /// 作成された herdr pane の ID
    pub pane_id: String,
    /// 実行時間（ミリ秒）
    pub elapsed_ms: u64,
    /// 使用したセッションディレクトリ（デバッグ用）
    pub session_dir: PathBuf,
    /// sai-write で発行された waggle トークン一覧
    pub waggle_tokens: Vec<String>,
    /// 出力タイプ: "document" | "code" | "both"
    pub output_type: String,
    /// 入力として使った waggle トークン一覧
    pub input_waggle_tokens: Vec<String>,
    /// pi起動時のHEADコミットハッシュ（再現性用）
    pub git_commit_before: Option<String>,
    /// pi終了時のHEADコミットハッシュ（再現性用）
    pub git_commit_after: Option<String>,
    /// セッション統計情報の JSON
    pub session_stats_json: Option<String>,
    /// 実行時の pi 起動コマンド全文（右SBセッション再開用）
    pub pi_command: String,
    /// 実行ID（サブタスク実行時に DB で作成された execution_history.id）
    pub execution_id: Option<i64>,
}

/// パイプライン実行エラー。
#[derive(Debug, thiserror::Error)]
pub enum OrchestratorError {
    #[error("herdr operation error: {0}")]
    HerdrError(#[from] HerdrError),
    #[error("Command build error: {0}")]
    CommandError(String),
    #[error("Session file error: {0}")]
    SessionError(String),
    #[error("The agent did not finish within {timeout_ms}ms.")]
    Timeout { timeout_ms: u64 },
    #[error("Cancelled.")]
    Cancelled,
    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),
    #[error("System prompt temp file error: {0}")]
    SystemPromptError(String),
    /// 文と確定キーは送ったが、状態が working にならなかった（届いたかどうか分からない）。
    #[error("The agent did not report that it started working within {waited_secs} seconds after the message and the confirm key {submit_key} were sent ({attempts} times). The message may have been delivered; check the screen before sending it again.")]
    PromptUnconfirmed {
        submit_key: String,
        attempts: u32,
        waited_secs: u64,
    },
}

impl From<SessionError> for OrchestratorError {
    fn from(err: SessionError) -> Self {
        OrchestratorError::SessionError(err.to_string())
    }
}

// ============================================================================
// 完了イベント（デーモン連携用）
// ============================================================================

/// デーモンのポーリングループから実行監視スレッドへの完了通知イベント。
///
/// デーモンは `file_written` 通知（sai-write）を処理した際、
/// 通知の `run_id` に対応する実行の監視チャネルへこのイベントを送信します。
/// 実行監視スレッドは sai-write をトークンの収集にだけ使い、完了は `FinalizeRequested`
/// で判定します（メインは `/agent-link-finalize`、サブは `task_report`。P6-2）。
///
/// `FinalizeRequested` はメインではユーザーが `/agent-link-finalize` を入力した際、
/// サブでは `task_report` が `finalize_request` を書いた際にデーモンから送信され、
/// 出力抽出・統計保存を実行して正常完了とします（ケース5）。
///
/// `DaemonShutdown` はデーモン停止時（SIGTERM 受信・P0-2）に全実行監視スレッドへ
/// 送られる切断イベントである。受信側は残留 `SaiWriteDetected` を回収し、
/// 利用可能な出力があれば完了処理へ進み、無ければ失敗確定へ進む。
#[derive(Debug, Clone)]
pub enum CompletionEvent {
    /// sai-write の通知が検出・処理された（run_id 一致）
    SaiWriteDetected {
        /// 対象の実行 ID
        run_id: i64,
        /// トークン化された waggle トークン（成功時）
        waggle_token: Option<String>,
        /// 書き込まれたファイルのパス
        file_path: PathBuf,
    },
    /// 完了要求（メインは `/agent-link-finalize`、サブは `task_report`）
    FinalizeRequested {
        /// 対象の実行 ID
        run_id: i64,
    },
    /// デーモン停止による切断（P0-2）。`running_executions` の全送信先へ送る。
    DaemonShutdown {
        /// 対象の実行 ID
        run_id: i64,
    },
}

/// 実行監視マップ: run_id → 完了通知チャネル（デーモンが保持）。
/// 実行パイプライン（`execute_block_pipeline`）が自身を登録・解除します。
pub type RunningExecutions = Arc<Mutex<HashMap<i64, mpsc::Sender<CompletionEvent>>>>;

// ============================================================================
// 実行時設定（デフォルト値 + 呼び出し時上書き）
// ============================================================================

/// 実行時オーバーライド（§6・P8 で一元化）。
///
/// メイン: CLI 引数 / 対話モード入力。
/// サブ: task_create の inputs / worktree 等。
/// 指定がなければブロック定義のフロントマター値、さらにエンジンのデフォルト値を使用する。
///
/// ブロック定義書（フロントマター）は**デフォルト値**であり、
/// CLI / Web が実行時にこの構造体でオーバーライドする（query3・A5）。
/// 旧 `daemon::BlockRunOverrides` / `interactive::ExecutionSettings` は本構造体に統合された（P8）。
/// `timeout` の単位は**秒**に統一。旧形式の `timeout_ms`（ミリ秒）は alias で受容するが、
/// 古い pending 通知が残る稀なケースのみに影響する移行措置。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ExecutionOverrides {
    /// モデル指定（例: "anthropic/claude-sonnet-4-5"）
    pub model: Option<String>,
    /// thinking budget: off / minimal / low / medium / high / xhigh
    pub thinking: Option<String>,
    /// タイムアウト（**秒**）。None ならフロントマター値または 300 秒
    #[serde(default, alias = "timeout_ms")]
    pub timeout: Option<u32>,
    /// ペイン分割方向
    pub direction: Option<SplitDirection>,
    /// worktree 指定（"isolation" / "none" / リテラルブランチ名）
    pub worktree: Option<String>,
    /// 作業フォルダ（サブタスクの `cwd`。絶対パス。None ならプロジェクトルート）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    /// デバッグ用にペインを維持するか（1.10 の修正: 全経路で伝播）
    #[serde(default)]
    pub keep_pane: bool,
}

/// `resolve_execution_settings` の解決結果。
#[derive(Debug, Clone)]
pub struct ResolvedExecutionSettings {
    /// オーバーライド適用後のブロック設定
    pub config: BlockConfig,
    /// エージェント完了待機のタイムアウト（ミリ秒）
    pub timeout_ms: u64,
    /// ペイン分割方向
    pub direction: SplitDirection,
    /// 適用する worktree ブランチ指定（None = worktree 不使用）
    pub worktree: Option<String>,
    /// デバッグ用にペインを維持するか
    pub keep_pane: bool,
}

/// 実行時設定を解決する共通関数（§6）。
///
/// 優先順位（高い順）:
/// 1. 呼び出し時オーバーライド（`overrides`）
/// 2. ブロック定義のフロントマター（デフォルト値）
/// 3. エンジンのデフォルト（`timeout: 300` 秒など）
pub fn resolve_execution_settings(
    block_config: &BlockConfig,
    overrides: &ExecutionOverrides,
) -> ResolvedExecutionSettings {
    let mut config = block_config.clone();

    // 1. オーバーライドを config に反映
    if let Some(ref model) = overrides.model {
        if !model.is_empty() {
            config.model = Some(model.clone());
        }
    }
    if let Some(ref thinking) = overrides.thinking {
        let t = thinking.trim().to_lowercase();
        if !t.is_empty() {
            if crate::validation::VALID_THINKING_LEVELS.contains(&t.as_str()) {
                // 新 6 値（`none` 含む）は明示指定として block.md の値を上書きする
                config.thinking = Some(t);
            } else {
                // 旧値（off / minimal 等）は拒否して block.md の値に任せる
                eprintln!(
                    "[agent-link] 警告: thinking '{}' は無効です。\
                     'none', 'low', 'medium', 'high', 'xhigh', 'max' のいずれかを指定してください",
                    thinking
                );
            }
        }
    }
    if let Some(timeout) = overrides.timeout {
        config.timeout = Some(timeout);
    }

    // 2. R11: `.agent-link/settings.json` のデフォルト値を補完
    //    ブロック定義に model/thinking が無い場合、settings.json の値を参照する。
    let settings = crate::settings::load_settings();
    if config.model.is_none() || config.model.as_deref() == Some("") {
        if let Some(ref model) = settings.default_model {
            if !model.is_empty() {
                config.model = Some(model.clone());
            }
        }
    }
    if config.thinking.is_none() || config.thinking.as_deref() == Some("") {
        if let Some(ref level) = settings.default_thinking_level {
            let t = level.trim().to_lowercase();
            if t.is_empty() || t == "none" {
                // 未指定・none は block.md の値に任せるため設定しない
            } else if crate::validation::VALID_THINKING_LEVELS.contains(&t.as_str()) {
                config.thinking = Some(t);
            } else {
                // 旧値（off / minimal 等）を含む無効値は拒否する（新 6 値のみ）
                eprintln!(
                    "[agent-link] 警告: settings.json の default_thinking_level '{}' は無効です。\
                     'none', 'low', 'medium', 'high', 'xhigh', 'max' のいずれかを指定してください",
                    level
                );
            }
        }
    }

    // 3. 解決
    // timeout 未指定は実質無期限（block.md に `timeout:` がある時だけ期限が効く）。
    let timeout_ms = config
        .timeout
        .map(|s| (s as u64).saturating_mul(1000))
        .unwrap_or(NO_TIMEOUT_MS);
    let direction = overrides.direction.unwrap_or(SplitDirection::Right);
    let worktree = overrides
        .worktree
        .clone()
        .or_else(|| config.worktree.clone())
        .filter(|w| w != "none" && !w.is_empty());

    ResolvedExecutionSettings {
        config,
        timeout_ms,
        direction,
        worktree,
        keep_pane: overrides.keep_pane,
    }
}

// ============================================================================
// PreparedPipeline — 実行前の準備結果（ユーザー確認用）
// ============================================================================

/// `prepare_pipeline()` の戻り値。
///
/// 起動前の準備（プレースホルダー展開、セッションディレクトリ作成、
/// コマンド構築）の結果を保持します。
/// TUI はこの内容をユーザーに表示して実行確認を求めます。
#[derive(Debug, Clone)]
pub struct PreparedPipeline {
    /// ユーザーに表示する完全な pi 起動コマンド文字列
    pub pi_command: String,
    /// pi コマンドの引数リスト（デバッグ用）
    pub pi_args: Vec<String>,
    /// セッションディレクトリのパス
    pub session_dir: PathBuf,
    /// 最初のメッセージ（`herdr.send_text` で 1 通で送る文）。
    ///
    /// 最初に `# task_text` の節を置き、そのほかの入力値を `# <id>` の節にして並べる
    /// （節の作り方は `split_fixed_and_variable`）。template と block.md の本文（固定部分）は
    /// [`PreparedPipeline::system_prompt`] に入り、`system-prompt.md` 経由で渡す（U6・C10）。
    pub expanded_prompt: String,
    /// 固定部分（system prompt・template と本文の案内文に置き換えたもの）。
    /// run フォルダの `system-prompt.md` に書いて、起動コマンドの
    /// `--append-system-prompt`（pi）/ `--append-system-prompt-file`（cc）で渡す（U6）。
    /// ファイルに書けなかったときは、この内容を最初の文の先頭に付けている。
    pub system_prompt: String,
    /// ブロック名
    pub block_name: String,
    /// 作業ディレクトリ
    pub cwd: Option<PathBuf>,
    /// sai-write ツールを使用するブロックかどうか
    pub uses_doc_write: bool,
    /// 入力として解決された waggle トークン一覧
    pub input_waggle_tokens: Vec<String>,
    /// pi起動時のHEADコミットハッシュ（再現性用）
    pub git_commit_before: Option<String>,
    /// 実行ID（ペインラベル用。`prepare_pipeline()` では設定されず、呼び出し元が設定する）
    pub run_id: Option<i64>,
    /// バッチ実行時に事前作成された herdr タブの ID（U2・R4。None は従来の分割方式）
    pub tab_id: Option<String>,
    /// 動的 pane 配置で決定した分割基準ペインの ID（U2・R4 案X。None は既定の分割方式）
    pub base_pane_id: Option<String>,
    /// 動的 pane 配置で**既に作成済みの pane**（分割は task_manager 側で実施済み）。
    /// Some ならパイプラインは分割をスキップしてこの pane を再利用する（競合防止・案X）。
    pub pane_id: Option<String>,
    /// サブタスクのタスク ID（U3。None はメイン/単体実行で停止通知を行わない）
    pub task_id: Option<i64>,
    /// 親実行 ID（U3。停止通知 JSON の parent_run_id に使用）
    pub parent_run_id: Option<i64>,
    /// プロンプト確定送信キー（cc は Alt+Enter）。
    pub submit_key: &'static str,
    /// エージェント側セッション ID（cc のみ。`--session-id` に渡し DB 保存する）。
    pub agent_session_id: Option<String>,
}

// ============================================================================
// PipelineProgress — 実行中の進捗状態（TUIポーリング用）
// ============================================================================

/// パイプラインの実行状態。
///
/// TUI の描画ループから定期的に `PipelineHandle::get_progress()` で取得し、
/// 進捗表示に使用します。
#[derive(Debug, Clone, PartialEq)]
pub enum PipelineProgress {
    /// 開始前（スレッド起動直後）
    Pending,
    /// herdr pane 分割中
    SplittingPane,
    /// 待機中（シェル初期化 / pi初期化 / アイドル確認 / 作業開始確認 / 完了待機）
    Waiting {
        /// 現在の待機ステップ名
        step: String,
        /// 経過秒数
        elapsed_secs: u64,
    },
    /// pi が作業中（プロンプト送信済み、実行中）
    Running {
        /// 作業開始からの経過秒数
        elapsed_secs: u64,
    },
    /// 出力抽出中
    ExtractingOutput,
    /// クリーンアップ中
    CleaningUp,
    /// 正常完了
    Completed {
        /// エージェントの最終出力テキスト
        output: String,
        /// 合計実行時間（ミリ秒）
        elapsed_ms: u64,
        /// sai-write で発行された waggle トークン一覧
        waggle_tokens: Vec<String>,
        /// セッション統計情報の JSON（トークン数・ツール呼び出し回数など）
        session_stats_json: Option<String>,
        /// pi起動時のHEADコミットハッシュ
        git_commit_before: Option<String>,
        /// pi終了時のHEADコミットハッシュ
        git_commit_after: Option<String>,
        /// 実行に使用した herdr pane ID（自動削除用）
        pane_id: String,
    },
    /// 失敗
    Failed {
        /// エラーメッセージ
        error: String,
    },
    /// ユーザーによるキャンセル
    Cancelled,
}

// ============================================================================
// PipelineHandle — バックグラウンド実行の制御ハンドル
// ============================================================================

/// バックグラウンドで実行中のパイプラインを制御するハンドル。
///
/// `spawn_pipeline()` から即座に返され、TUI はこのハンドルを通じて
/// 進捗ポーリングとキャンセルを行います。
pub struct PipelineHandle {
    /// 現在の進捗状態（Mutex で保護、TUI が定期的に読む）
    progress: Arc<Mutex<PipelineProgress>>,
    /// キャンセルフラグ（TUI が `true` にセットする）
    cancel_flag: Arc<AtomicBool>,
    /// 実行スレッドの JoinHandle（完了待機用）
    thread_handle: Option<JoinHandle<()>>,
}

impl PipelineHandle {
    /// 現在の進捗を取得する（ノンブロッキング）。
    ///
    /// TUI の描画ループから呼び出します。
    /// `Mutex` のロックは即座に取得されるため、UI のフレームレートに影響しません。
    pub fn get_progress(&self) -> PipelineProgress {
        self.progress.lock().unwrap().clone()
    }

    /// 実行をキャンセルする。
    ///
    /// TUI の「停止」ボタン押下時などに呼び出します。
    /// バックグラウンドスレッドは次のチェックポイントで中断され、
    /// クリーンアップを実行してから停止します。
    pub fn cancel(&self) {
        self.cancel_flag.store(true, Ordering::SeqCst);
    }

    /// 完了しているかどうかを返す（ノンブロッキング）。
    ///
    /// `Completed` / `Failed` / `Cancelled` のいずれかであれば `true`。
    pub fn is_finished(&self) -> bool {
        matches!(
            self.get_progress(),
            PipelineProgress::Completed { .. }
                | PipelineProgress::Failed { .. }
                | PipelineProgress::Cancelled
        )
    }

    /// スレッドの完了を待ち、最終的な進捗を返す（ブロッキング）。
    ///
    /// `run_pipeline()` の後方互換実装や、完了後の後処理で使用します。
    pub fn join(mut self) -> PipelineProgress {
        if let Some(handle) = self.thread_handle.take() {
            let _ = handle.join();
        }
        self.get_progress()
    }
}

// ============================================================================
// Public API 1: prepare_pipeline — ユーザー確認前の準備
// ============================================================================

/// パイプラインの準備フェーズを実行します。
///
/// ブロック定義のパース結果から以下の準備を行います：
/// 1. プレースホルダー展開（`{{input.<id>}}` → 実際の値）
/// 2. セッションディレクトリ作成
/// 3. セッションファイル（JSONL）作成
/// 4. 起動コマンド構築（cc）
///
/// **この時点では herdr 操作は一切行いません。**
/// 戻り値の `PreparedPipeline.pi_command` をユーザーに表示し、
/// 確認を得てから `spawn_pipeline()` に渡してください。
///
/// # 引数
///
/// * `config` — パイプライン実行設定
///
/// # 戻り値
///
/// 成功時は `PreparedPipeline`、失敗時は `OrchestratorError`
pub fn prepare_pipeline(config: &PipelineConfig) -> Result<PreparedPipeline, OrchestratorError> {
    // ── 入力値の解決（waggle トークンがある場合はファイル内容を取得） ──
    // DB の親ディレクトリが存在しない場合は作成（プロジェクトルート基準に統一）
    let waggle_db_path = crate::project::db_path();
    if let Some(parent) = waggle_db_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let integrator = WaggleIntegrator::open(&waggle_db_path)
        .map_err(|e| OrchestratorError::CommandError(format!("Failed to initialize the waggle DB: {}", e)))?;

    // HashMap<String, String> → HashMap<String, PlaceholderValue>
    let placeholder_values: HashMap<String, PlaceholderValue> = config
        .user_inputs
        .iter()
        .map(|(k, v)| (k.clone(), PlaceholderValue::from_string(v)))
        .collect();

    let (resolved_values, input_waggle_tokens) =
        resolve_placeholder_values(&placeholder_values, &integrator);

    // ── Step 1: 固定部分（system prompt）と最初の文に分ける ──
    // 固定部分（template と block.md の本文）は run フォルダの `system-prompt.md` に書き、
    // 起動コマンドのフラグで渡す。最初の文には入力値だけが入る。
    let (system_prompt, first_message) =
        split_fixed_and_variable(&config.block_body, &resolved_values);

    // ── pi起動前の Git コミットハッシュを取得（再現性用） ──
    let git_commit_before = get_git_commit_hash();

    // ── Step 2: セッションディレクトリ作成 ──
    // outputs 統一: 実行ブロック関連の全成果物は run フォルダ
    // （`.agent-link/outputs/<owner>/<run_id>/`）に格納する。run_id が確定している場合は
    // `<run_dir>/session/` に作り、status 移動（todo→running→review→R）に
    // 追随させる（run フォルダごと rename されるため DB の session_dir が
    // 古くなっても `resolve_session_dir()` のフォールバックで追従できる）。
    // run フォルダが無い場合（run_id 未確定・テスト等）は旧 `.agent-link/sessions/` に作る。
    let session_dir = match config.run_id.map(crate::run_dir::find_run_dir) {
        // run フォルダ実在（kanban 予約 run は実行開始時に todo→running 済み）
        Some(Some((owner, run))) => {
            let sess = run.join(crate::run_dir::SESSION_SUBDIR);
            std::fs::create_dir_all(&sess)?;
            eprintln!(
                "[agent-link] セッションディレクトリ: {} (owner={})",
                sess.display(),
                owner
            );
            sess
        }
        // run_id はあるが run フォルダが未作成 → running owner に作る
        // （直接 CLI 実行などカードを持たない run も outputs 統一に従う）
        Some(None) => {
            let run = crate::run_dir::ensure_run_dir(
                crate::run_dir::RUNNING_OWNER,
                config.run_id.unwrap_or(0),
            )?;
            let sess = run.join(crate::run_dir::SESSION_SUBDIR);
            std::fs::create_dir_all(&sess)?;
            eprintln!(
                "[agent-link] セッションディレクトリ: {} (owner={})",
                sess.display(),
                crate::run_dir::RUNNING_OWNER
            );
            sess
        }
        // run_id 未確定（テスト等）→ 旧 `.agent-link/sessions/` に作る
        None => create_session_dir(&config.block_config.block_id())?,
    };

    // ── Step 3: セッションファイル作成 ──
    let _session_path = create_session_file(&session_dir, &config.block_config.block_id())?;

    // ── Step 3.5: 固定部分（system prompt）を run フォルダに書く（U6・C10） ──
    // run フォルダは Step 2 で用意済み。`system-prompt.md` に書いて起動コマンドの
    // フラグで渡すので、最初の文には入力値しか出ない。
    // 書けなかったとき（run フォルダが無い・書き込み失敗）は従来どおり 1 通で送る。
    let system_prompt_path = write_system_prompt_file(config.run_id, &system_prompt);
    let expanded_prompt = match (&system_prompt_path, system_prompt.trim().is_empty()) {
        (Some(_), _) => first_message,
        // 固定部分が空なら付け足すものがない
        (None, true) => first_message,
        (None, false) => {
            // run_id 無し（プレビュー・テスト）は最初から run フォルダが無いので警告を出さない。
            if config.run_id.is_some() {
                eprintln!(
                    "[agent-link] 警告: {} に固定部分を置けません。最初の文の先頭に入れます",
                    crate::run_dir::SYSTEM_PROMPT_FILE
                );
            }
            format!("{}\n\n{}", system_prompt.trim_end(), first_message)
        }
    };

    // ── Step 4+5: 起動コマンド構築 ──
    let (mut agent_command, agent_args, agent_session_id) = {
        let cwd = config
            .cwd
            .clone()
            .unwrap_or_else(crate::project::project_root);
        // 道具・フック・statusLine は `.agent-link/cc-settings.json` を `--settings` で渡す
        // （プロジェクトの `.claude/settings.json` は書かない）。cwd に `.mcp.json` が無ければ
        // プロジェクトルートの物を `--mcp-config` で渡す。
        let root = crate::project::project_root();
        let settings = crate::cc_command::ensure_cc_settings(&root).map_err(|e| {
            OrchestratorError::CommandError(format!(
                "Could not write the cc settings {}: {}",
                crate::cc_command::cc_settings_path(&root).display(),
                e
            ))
        })?;
        let session_id = crate::cc_command::generate_uuid_v4();
        let mut builder = CcCommandBuilder::new(&session_id)
            .with_config(config.block_config.clone())
            .with_settings_file(settings);
        if let Some(mcp) = crate::cc_command::resolve_mcp_config(&cwd) {
            builder = builder.with_mcp_config(mcp);
        }
        // 固定部分（template と本文）を system-prompt.md から足す（U6）
        if let Some(path) = &system_prompt_path {
            builder = builder.with_system_prompt_file(path.to_string_lossy());
        }
        let cc_cmd = builder.build()
            .map_err(|e| {
                OrchestratorError::CommandError(format!("Failed to build the cc command: {}", e))
            })?;
        // エージェント側セッション ID（`--session-id` に渡し DB へ保存する）
        (cc_cmd.command, cc_cmd.args, Some(session_id))
    };

    // ── 設定ファイル (env:) の環境変数をコマンド先頭に付与 ──
    // `.agent-link/settings.json` と `~/.agent-link/settings.json` の `env` を、
    // エージェントの実行ファイルより前に並べる。
    // 値は shell のシングルクォートで囲む（herdr::shell_escape_single_quotes）。
    // 予約キー（AGENT_LINK_* の 4 つ）は設定の読み込み側で破棄される。
    // ここは AGENT_LINK_* を付ける「前」に入れる。あとから付く AGENT_LINK_* が
    // この行の上に重なるので、最終的な並びは「AGENT_LINK_* → env → 実行ファイル」になる。
    if let Some(env_pairs) = crate::settings::agent_env_pairs() {
        let rendered: String = env_pairs
            .iter()
            .map(|(k, v)| format!("{}={} ", k, crate::herdr::shell_escape_single_quotes(v)))
            .collect();
        agent_command = format!("{rendered}{agent_command}");
    }

    // ── エージェント用の環境変数をコマンド先頭に付与 ──
    // エージェントプロセスは herdr ペインのシェルで起動されるため、
    // `AGENT_LINK_PROJECT_ROOT`（= プロジェクトルート）と `AGENT_LINK_PARENT_RUN_ID`（= 実行 ID）を
    // シェル変数として先頭に前置する。
    // - AGENT_LINK_PROJECT_ROOT: 実行の中で `.agent-link/` をプロジェクトルート基準で解決するため
    //   （worktree 実行時は cwd が `.worktrees/<branch>` に変わるため必須）
    // - AGENT_LINK_PARENT_RUN_ID: task create / task execute / doc write の run_id に使用
    // - AGENT_LINK_BLOCK_NAME: doc write の通知ブロック名（block_name）に使用
    // - AGENT_LINK_IS_SUBTASK: サブタスク実行中の場合に "1"
    let subtask_env = if config.is_subtask {
        // AGENT_LINK_BIN: 子から agent-link を呼ぶための実行ファイルのパス。
        // shell エスケープして渡す。
        format!(
            "AGENT_LINK_IS_SUBTASK=1 AGENT_LINK_BIN={} ",
            crate::herdr::shell_escape_single_quotes(&crate::cc_command::sai_link_exe())
        )
    } else {
        String::new()
    };
    // デーモン稼働時（AGENT_LINK_PROJECT_ROOT 設定済み）のみ付与。
    // デーモン外（テスト等）では従来どおり run_id 指定時のみ付与する。
    if let Some(project_root) = std::env::var_os("AGENT_LINK_PROJECT_ROOT") {
        let root_str = project_root.to_string_lossy().to_string();
        let escaped_root = crate::herdr::shell_escape_single_quotes(&root_str);
        if let Some(run_id) = config.run_id {
            agent_command = format!(
                "AGENT_LINK_PROJECT_ROOT={} AGENT_LINK_PARENT_RUN_ID={} AGENT_LINK_BLOCK_NAME={} {}{}",
                escaped_root,
                run_id,
                config.block_config.block_id(),
                subtask_env,
                agent_command
            );
        } else {
            agent_command =
                format!("AGENT_LINK_PROJECT_ROOT={} {}{}", escaped_root, subtask_env, agent_command);
        }
    } else if let Some(run_id) = config.run_id {
        agent_command = format!(
            "AGENT_LINK_PARENT_RUN_ID={} AGENT_LINK_BLOCK_NAME={} {}{}",
            run_id,
            config.block_config.block_id(),
            subtask_env,
            agent_command
        );
    }

    // ── CLI を名前だけで呼べるようにする（子の pane・エージェントの pane）──
    // テンプレートとフックは `agent-link …`・`agent-link-web …` を名前だけで呼ぶ。
    // 実行ファイルは同じフォルダにできるので、`sai_link_exe()` のあるフォルダを
    // PATH の先頭に足す。利用者の PATH の設定に頼らない。
    {
        let exe = crate::cc_command::sai_link_exe();
        if let Some(dir) = std::path::Path::new(&exe).parent() {
            if !dir.as_os_str().is_empty() {
                agent_command = format!(
                    "PATH={}:$PATH {}",
                    crate::herdr::shell_escape_single_quotes(&dir.to_string_lossy()),
                    agent_command
                );
            }
        }
    }

    Ok(PreparedPipeline {
        pi_command: agent_command,
        pi_args: agent_args,
        session_dir,
        expanded_prompt,
        system_prompt,
        block_name: config.block_config.block_id().to_string(),
        cwd: config.cwd.clone(),
        uses_doc_write: crate::doc_write_notify::block_uses_doc_write(&config.block_config),
        input_waggle_tokens,
        git_commit_before,
        run_id: config.run_id,
        tab_id: config.tab_id.clone(),
        base_pane_id: config.base_pane_id.clone(),
        pane_id: config.pane_id.clone(),
        task_id: None,       // U3: 後段（run_pipeline_with_watcher）で設定
        parent_run_id: None, // U3: 後段で設定
        submit_key: crate::cc_command::CC_SUBMIT_KEY,
        agent_session_id,
    })
}

/// 固定部分（system prompt）を run フォルダの `system-prompt.md` に書く（U6・C10）。
///
/// - 固定部分が空、または run_id が無い場合は `None`（run を作らない）。
/// - run フォルダが無い場合も `None`（固定部分は最初の文に入る）。
/// - 書き込みに失敗したときも `None` にして、警告は [`prepare_pipeline`] が出す。
fn write_system_prompt_file(run_id: Option<i64>, system_prompt: &str) -> Option<PathBuf> {
    if system_prompt.trim().is_empty() {
        return None;
    }
    let (_, run_dir) = crate::run_dir::find_run_dir(run_id?)?;
    let path = run_dir.join(crate::run_dir::SYSTEM_PROMPT_FILE);
    match std::fs::write(&path, system_prompt) {
        Ok(()) => Some(path),
        Err(e) => {
            eprintln!("[agent-link] {} に書けません: {}", path.display(), e);
            None
        }
    }
}

// ============================================================================
// Public API 2: spawn_pipeline — バックグラウンド実行
// ============================================================================

/// パイプラインを**別スレッドでバックグラウンド実行**します。
///
/// この関数は即座に `PipelineHandle` を返します。
/// TUI はこのハンドルを通じて進捗ポーリングとキャンセルを行います。
/// 呼び出し元スレッドはブロックされません。
///
/// # 引数
///
/// * `prepared` — `prepare_pipeline()` の戻り値
/// * `direction` — herdr ペイン分割方向
/// * `timeout_ms` — エージェント完了待機のタイムアウト（ミリ秒）
/// * `keep_pane` — デバッグ用に実行後ペインを維持するか
///
/// # 戻り値
///
/// 即座に `PipelineHandle` を返す（スレッドはバックグラウンドで起動）
pub fn spawn_pipeline(
    prepared: PreparedPipeline,
    direction: SplitDirection,
    timeout_ms: u64,
    keep_pane: bool,
) -> PipelineHandle {
    let progress = Arc::new(Mutex::new(PipelineProgress::Pending));
    let cancel_flag = Arc::new(AtomicBool::new(false));

    let progress_clone = Arc::clone(&progress);
    let cancel_clone = Arc::clone(&cancel_flag);

    let thread_handle = std::thread::spawn(move || {
        execute_pipeline_background(
            prepared,
            direction,
            timeout_ms,
            keep_pane,
            progress_clone,
            cancel_clone,
            None,
        );
    });

    PipelineHandle {
        progress,
        cancel_flag,
        thread_handle: Some(thread_handle),
    }
}

/// パイプラインを**別スレッドでバックグラウンド実行**します（デーモン連携版）。
///
/// `spawn_pipeline` と同じですが、sai-write 完了判定を
/// デーモンの通知処理ループからのイベント（`completion_rx`）で行います。
/// デーモンが `file_written` 通知を処理し、自分の run_id の
/// `CompletionEvent::SaiWriteDetected` を受信した時点で正常完了とみなします（D4）。
pub fn spawn_pipeline_with_watcher(
    prepared: PreparedPipeline,
    direction: SplitDirection,
    timeout_ms: u64,
    keep_pane: bool,
    completion_rx: Option<mpsc::Receiver<CompletionEvent>>,
) -> PipelineHandle {
    let progress = Arc::new(Mutex::new(PipelineProgress::Pending));
    let cancel_flag = Arc::new(AtomicBool::new(false));

    let progress_clone = Arc::clone(&progress);
    let cancel_clone = Arc::clone(&cancel_flag);

    let thread_handle = std::thread::spawn(move || {
        execute_pipeline_background(
            prepared,
            direction,
            timeout_ms,
            keep_pane,
            progress_clone,
            cancel_clone,
            completion_rx,
        );
    });

    PipelineHandle {
        progress,
        cancel_flag,
        thread_handle: Some(thread_handle),
    }
}

// ============================================================================
// Public API 3: run_pipeline — 同期実行（後方互換）
// ============================================================================

/// エージェント実行パイプラインを**同期的に**実行します（CLI 互換）。
///
/// 内部的には `prepare_pipeline()` + スレッド実行 + `join()` の
/// 組み合わせです。TUI からの利用には `spawn_pipeline()` を使用してください。
///
/// # 引数
///
/// * `config` — パイプライン実行設定
///
/// # 戻り値
///
/// 成功時は `PipelineResult`、失敗時は `OrchestratorError`
pub fn run_pipeline(config: PipelineConfig) -> Result<PipelineResult, OrchestratorError> {
    let start = Instant::now();

    // 段階1: 準備
    let prepared = prepare_pipeline(&config)?;
    let input_waggle_tokens = prepared.input_waggle_tokens.clone();
    let git_commit_before = prepared.git_commit_before.clone();
    let session_dir = prepared.session_dir.clone();
    let pi_command = prepared.pi_command.clone();

    // 段階2: バックグラウンド実行 → 完了を同期待ち
    let handle = spawn_pipeline(
        prepared,
        config.direction,
        config.timeout_ms,
        config.keep_pane,
    );

    // スレッドの完了を待つ
    let final_progress = handle.join();

    let elapsed_ms = start.elapsed().as_millis() as u64;
    let git_commit_after = get_git_commit_hash();

    match final_progress {
        PipelineProgress::Completed {
            output,
            waggle_tokens,
            session_stats_json,
            pane_id,
            ..
        } => Ok(PipelineResult {
            output,
            exit_code: 0,
            pane_id,
            elapsed_ms,
            session_dir,
            pi_command,
            waggle_tokens,
            output_type: "document".to_string(),
            input_waggle_tokens,
            git_commit_before,
            git_commit_after,
            session_stats_json,
            execution_id: None,
        }),
        PipelineProgress::Failed { error } => Err(OrchestratorError::SessionError(error)),
        PipelineProgress::Cancelled => Err(OrchestratorError::Cancelled),
        _ => Err(OrchestratorError::SessionError(
            "Unexpected end state.".to_string(),
        )),
    }
}

/// パイプラインを**同期的に**実行します（デーモン連携版）。
///
/// `run_pipeline` と同じですが、sai-write 完了判定をデーモンの
/// 通知処理ループからのイベント（`completion_rx`）で行います。
/// `execute_block_pipeline` から呼び出されます。
pub fn run_pipeline_with_watcher(
    config: PipelineConfig,
    completion_rx: Option<mpsc::Receiver<CompletionEvent>>,
    store: Option<&crate::db::SaiStore>,
    run_id: Option<i64>,
    task_id: Option<i64>,
    parent_run_id: Option<i64>,
) -> Result<PipelineResult, OrchestratorError> {
    let start = Instant::now();

    // 段階1: 準備
    let mut prepared = prepare_pipeline(&config)?;
    // U3: サブブロック停止検知（idle/blocked）通知用にタスク情報を引き継ぐ
    prepared.task_id = task_id;
    prepared.parent_run_id = parent_run_id;
    let input_waggle_tokens = prepared.input_waggle_tokens.clone();
    let git_commit_before = prepared.git_commit_before.clone();
    let session_dir = prepared.session_dir.clone();
    let pi_command = prepared.pi_command.clone();

    // セッション情報（session_dir, pi_command）をDBに保存（成功・失敗にかかわらず右SB表示用）
    if let (Some(s), Some(id)) = (store, run_id) {
        let _ = s.update_execution_session_info(id, &session_dir.to_string_lossy(), &pi_command);
        // cc の場合はエージェント側セッション ID（--session-id の UUID）も保存する
        if let Some(ref sid) = prepared.agent_session_id {
            let _ = s.update_execution_agent_session_id(id, sid);
        }
        // 事前に用意された pane（task_* の動的配置）はこの時点ですでに確定している。
        // 完了時にしか DB へ書かないと、実行中の親から task_* が workspace を解決できず、
        // タブがユーザーの操作中の space に作られてしまう。
        if let Some(ref pid) = prepared.pane_id {
            let _ = s.update_execution_pane(id, pid, 0);
        }
    }

    // 段階2: バックグラウンド実行 → 完了を同期待ち
    let handle = spawn_pipeline_with_watcher(
        prepared,
        config.direction,
        config.timeout_ms,
        config.keep_pane,
        completion_rx,
    );

    // スレッドの完了を待つ
    let final_progress = handle.join();

    let elapsed_ms = start.elapsed().as_millis() as u64;
    let git_commit_after = get_git_commit_hash();

    match final_progress {
        PipelineProgress::Completed {
            output,
            waggle_tokens,
            session_stats_json,
            pane_id,
            ..
        } => Ok(PipelineResult {
            output,
            exit_code: 0,
            pane_id,
            elapsed_ms,
            session_dir,
            pi_command,
            waggle_tokens,
            output_type: "document".to_string(),
            input_waggle_tokens,
            git_commit_before,
            git_commit_after,
            session_stats_json,
            execution_id: None,
        }),
        PipelineProgress::Failed { error } => Err(OrchestratorError::SessionError(error)),
        PipelineProgress::Cancelled => Err(OrchestratorError::Cancelled),
        _ => Err(OrchestratorError::SessionError(
            "Unexpected end state.".to_string(),
        )),
    }
}

/// セッションディレクトリから統計を抽出し JSON 文字列にシリアライズする。
/// 失敗時は `None` を返す（統計抽出はベストエフォート）。
fn extract_and_serialize_stats(session_dir: &std::path::PathBuf) -> Option<String> {
    match crate::session::extract_session_stats(session_dir) {
        Ok(stats) => match serde_json::to_string(&stats) {
            Ok(json) => {
                eprintln!(
                        "[agent-link] セッション統計: turns={}, tools={}, tokens(in={}, out={}, think={})",
                        stats.turn_count,
                        stats.tool_call_count,
                        stats.input_tokens,
                        stats.output_tokens,
                        stats.thinking_tokens
                    );
                Some(json)
            }
            Err(e) => {
                eprintln!("[agent-link] 統計のシリアライズに失敗: {}", e);
                None
            }
        },
        Err(e) => {
            eprintln!("[agent-link] セッション統計の抽出に失敗: {}", e);
            None
        }
    }
}

/// cc 実行の Claude Code セッション JSONL を解決する。
fn claude_session_path(prepared: &PreparedPipeline) -> Option<std::path::PathBuf> {
    let sid = prepared.agent_session_id.as_deref()?;
    let cwd = prepared.cwd.as_deref()?;
    crate::session::claude_session_file(cwd, sid)
}

/// 出力・トランスクリプト・セッション統計を抽出する（`~/.claude/projects/<slug>/<uuid>.jsonl` を集計）。
fn extract_outputs_for_pipeline(prepared: &PreparedPipeline) -> (String, String, Option<String>) {
    match claude_session_path(prepared) {
        Some(path) => {
            let output = crate::session::extract_claude_final_output(&path).unwrap_or_default();
            let transcript = crate::session::extract_claude_full_transcript(&path);
            let stats = crate::session::extract_claude_session_stats(&path)
                .ok()
                .and_then(|s| serde_json::to_string(&s).ok());
            (output, transcript, stats)
        }
        None => (String::new(), String::new(), None),
    }
}

/// Git の HEAD コミットハッシュ（short）を取得する。
fn get_git_commit_hash() -> Option<String> {
    std::process::Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .and_then(|output| {
            if output.status.success() {
                let hash = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if hash.is_empty() {
                    None
                } else {
                    Some(hash)
                }
            } else {
                None
            }
        })
}

/// 入力 waggle トークンからブロック間接続を生成し DB に保存する。
///
/// 各入力トークンについて出力元の実行を検索し、
/// `block_connections` テーブルに接続レコードを挿入します。
/// 既存の接続は重複挿入されません（冪等）。
///
/// # 引数
///
/// * `store` — SaiStore（DB操作）
/// * `target_execution_id` — 入力先（現在の）実行ID
/// * `target_block_id` — 入力先ブロックID
/// * `input_waggle_tokens` — 入力として使った waggle トークン一覧
pub fn insert_block_connections(
    store: &crate::db::SaiStore,
    target_execution_id: i64,
    target_block_id: i64,
    input_waggle_tokens: &[String],
) {
    if input_waggle_tokens.is_empty() {
        return;
    }

    for token in input_waggle_tokens {
        match store.find_source_execution_by_waggle_token(token) {
            Ok(Some(source_exec)) => {
                let source_block_id = source_exec.block_id;
                let source_exec_id = source_exec.id.unwrap_or(0);
                if source_exec_id == 0 {
                    continue;
                }
                let conn_record = crate::db::BlockConnection::new_waggle(
                    source_exec_id,
                    source_block_id,
                    target_execution_id,
                    target_block_id,
                    token.clone(),
                );
                if let Err(e) = store.insert_connection(&conn_record) {
                    eprintln!(
                        "[agent-link] ブロック間接続の挿入に失敗 ({} → {}): {}",
                        source_block_id, target_block_id, e
                    );
                }
            }
            Ok(None) => {
                // トークンの出力元が見つからない — まだトークン化されていない可能性
            }
            Err(e) => {
                eprintln!("[agent-link] トークン '{}' の検索に失敗: {}", token, e);
            }
        }
    }
}

// ============================================================================
// 実行レコード作成（メイン/サブ共通）
// ============================================================================

/// 実行前の準備結果（実行レコード）。
#[derive(Debug, Clone)]
pub struct PreparedExecutionInfo {
    /// 作成された実行 ID（`execution_history.id`）
    pub run_id: i64,
    /// ブロック ID（`block_metadata.id`）
    pub block_id: i64,
}

/// ブロック定義を `block_metadata` に登録（存在すれば更新）する。
///
/// `get_or_create_block` 相当の共通関数。メイン/サブ/デーモンの定期リロードが使用する。
pub fn upsert_block_metadata(
    store: &crate::db::SaiStore,
    config: &BlockConfig,
    file_path: &Path,
) -> Result<i64, OrchestratorError> {
    use crate::db::models::BlockMetadata;

    // P10: 保存経路でも validation を実行し、回復不能なエラーがあるブロックは
    // 登録をスキップして警告を残す（CLI 経路とデーモンの定期リロード経路の整合）。
    // 警告（description/model 欠落など）はデフォルト補完されるため保存は続行する。
    let vresult = crate::validation::validate(config);
    if !vresult.is_valid() {
        let detail: Vec<String> = vresult.errors.iter().map(|e| e.to_string()).collect();
        return Err(OrchestratorError::CommandError(format!(
            "Skipped registering block '{}' because of validation errors: {}",
            config.name,
            detail.join("; ")
        )));
    }

    let file_path_str = file_path.to_string_lossy().to_string();

    let upsert = |existing: Option<BlockMetadata>| -> BlockMetadata {
        let created_at = existing
            .as_ref()
            .map(|e| e.created_at.clone())
            .unwrap_or_default();
        BlockMetadata {
            id: existing.as_ref().and_then(|e| e.id),
            name: config.block_id().to_string(),
            display_name: config.display_name().to_string(),
            description: config.description.clone().unwrap_or_default(),
            model: config
                .model
                .clone()
                .unwrap_or_else(|| "inherit".to_string()),
            thinking: config.thinking.clone().unwrap_or_else(|| "none".to_string()),
            tools: config
                .tools
                .as_ref()
                .map(|t| t.to_string())
                .unwrap_or_else(|| "true".to_string()),
            extensions: config
                .extensions
                .as_ref()
                .map(|e| e.to_string())
                .unwrap_or_else(|| "true".to_string()),
            skills: config.skills.clone(),
            timeout: config.timeout.unwrap_or(300) as i32,
            worktree: config
                .worktree
                .clone()
                .unwrap_or_else(|| "none".to_string()),
            enable: config.enable.unwrap_or(true),
            file_path: file_path_str,
            body_hash: None,
            created_at,
            updated_at: String::new(),
        }
    };

    match store.get_block_by_name(config.block_id()) {
        Ok(existing) => {
            let block_id = existing.id.ok_or_else(|| {
                OrchestratorError::CommandError("The existing block has no ID.".to_string())
            })?;
            let updated = upsert(Some(existing));
            let _ = store.update_block(&updated);
            Ok(block_id)
        }
        Err(_) => {
            let new_block = upsert(None);
            store.insert_block(&new_block).map_err(|e| {
                OrchestratorError::CommandError(format!("Failed to register the block: {}", e))
            })
        }
    }
}

/// 実行レコードを作成する（メイン/サブ共通）。
///
/// 1. ブロック定義を `block_metadata` に登録（存在すれば更新）
/// 2. `execution_history` に実行レコードを作成（`parent_run_id` の有無のみ分岐）
/// 3. サブの場合は親→サブの `task_delegation` 接続を作成
///
/// # 引数
///
/// * `store` — DB ストア
/// * `config` — 実行時設定解決後のブロック設定
/// * `file_path` — ブロック定義ファイルのパス
/// * `user_inputs_json` — ユーザー入力の JSON（未入力なら None）
/// * `parent_run_id` — メイン: None / サブ: 親の run_id
/// * `batch_id` — タスクバッチ ID（P3-A。サブ実行のフェーズ合成に使用。メインは None）
pub fn prepare_execution_record(
    store: &crate::db::SaiStore,
    config: &BlockConfig,
    file_path: &Path,
    user_inputs_json: Option<&str>,
    parent_run_id: Option<i64>,
    batch_id: Option<&str>,
) -> Result<PreparedExecutionInfo, OrchestratorError> {
    prepare_execution_record_full(
        store, config, file_path, user_inputs_json, parent_run_id, batch_id, None,
    )
}

/// 実行レコード作成の完全版（予約 run_id 付き・R302）。
///
/// `reserved_run_id` が `Some` のときは、次の 2 通りを区別する。
///
/// - 親なし（メイン実行・人が起こすカード）: 予約行を確定させて使い回す。
/// - 親あり（`task_create` の `card` 指定・エージェントが起こすカード・U2）:
///   予約行を子として確定させ、`parent_run_id` と `batch_id` を書き込む。
///   run_id は予約のまま変わらず、親から子への線も今の子と同じように作る。
///
/// どちらも `reserved_run_id` が `None` なら従来どおり新規発行する。
pub fn prepare_execution_record_full(
    store: &crate::db::SaiStore,
    config: &BlockConfig,
    file_path: &Path,
    user_inputs_json: Option<&str>,
    parent_run_id: Option<i64>,
    batch_id: Option<&str>,
    reserved_run_id: Option<i64>,
) -> Result<PreparedExecutionInfo, OrchestratorError> {
    let block_id = upsert_block_metadata(store, config, file_path)?;

    // 予約行の確定（メイン実行のみ）
    if parent_run_id.is_none() {
        if let Some(rid) = reserved_run_id {
            store
                .claim_reserved_run(rid, block_id, user_inputs_json)
                .map_err(|e| {
                    OrchestratorError::CommandError(format!(
                        "Failed to claim the reserved run: {}",
                        e
                    ))
                })?;
            // ステータスを running に更新
            store.update_execution_status(rid, "running").map_err(|e| {
                OrchestratorError::CommandError(format!("Failed to update the status: {}", e))
            })?;
            // kanban 実行は呼び出し元が `update_execution_kanban_info` で上書きする
            let _ = store.update_execution_kanban_info(rid, None, "interactive");
            return Ok(PreparedExecutionInfo {
                run_id: rid,
                block_id,
            });
        }
    }

    // 予約行を子として起こす（U2: todo のカードを自分の子にする）。
    // ここでできる子（親から子への線と DB の行）は、今の子（`create_execution_for_subtask`）と同じ形にする。
    if let (Some(pid), Some(rid)) = (parent_run_id, reserved_run_id) {
        store
            .claim_reserved_run_for_subtask(rid, block_id, user_inputs_json, pid, batch_id)
            .map_err(|e| {
                OrchestratorError::CommandError(format!("Failed to claim the reserved run: {}", e))
            })?;

        let parent_block_id = store.get_execution_by_id(pid).map_err(|e| {
            OrchestratorError::CommandError(format!("Failed to get the parent run: {}", e))
        })?;
        let parent_block_id = parent_block_id.block_id;
        let delegation_conn = crate::db::BlockConnection::new_task_delegation(
            pid,
            parent_block_id,
            rid,
            block_id,
        );
        let _ = store.insert_connection(&delegation_conn);

        store.update_execution_status(rid, "running").map_err(|e| {
            OrchestratorError::CommandError(format!("Failed to update the status: {}", e))
        })?;
        // カード経由の実行は呼び出し元が `update_execution_kanban_info` で `kanban` に上書きする
        let _ = store.update_execution_kanban_info(rid, None, "interactive");
        return Ok(PreparedExecutionInfo {
            run_id: rid,
            block_id,
        });
    }

    let run_id = match parent_run_id {
        Some(pid) => {
            // 親実行からブロック ID を取得して委譲接続を作成
            let parent_exec = store.get_execution_by_id(pid).map_err(|e| {
                OrchestratorError::CommandError(format!("Failed to get the parent run: {}", e))
            })?;
            let parent_block_id = parent_exec.block_id;

            let sub_run_id = store
                .create_execution_for_subtask(block_id, user_inputs_json, pid, batch_id)
                .map_err(|e| {
                    OrchestratorError::CommandError(format!(
                        "Failed to create the subtask run record: {}",
                        e
                    ))
                })?;

            // 親→サブの task_delegation 接続を作成（D13）
            let delegation_conn = crate::db::BlockConnection::new_task_delegation(
                pid,
                parent_block_id,
                sub_run_id,
                block_id,
            );
            let _ = store.insert_connection(&delegation_conn);

            sub_run_id
        }
        None => store
            .create_execution(block_id, user_inputs_json)
            .map_err(|e| {
                OrchestratorError::CommandError(format!("Failed to create the run record: {}", e))
            })?,
    };

    // ステータスを running に更新
    store
        .update_execution_status(run_id, "running")
        .map_err(|e| OrchestratorError::CommandError(format!("Failed to update the status: {}", e)))?;

    // 通常実行は `execution_mode='interactive'` を記録する（kanban 実行は
    // CLI 側が `update_execution_kanban_info` で `kanban` を上書きする）。
    let _ = store.update_execution_kanban_info(run_id, None, "interactive");

    Ok(PreparedExecutionInfo { run_id, block_id })
}

// ============================================================================
// worktree 適用（lib/herdrs/runner.ts の Rust 移植・§7.1.6）
// ============================================================================

/// worktree 指定からブランチ名を解決する。
///
/// `lib/herdrs/runner.ts` の `resolveWorktreeBranch()` の移植。
/// - `"isolation"` → `<block_name>-<run_id>`（実行ごとに一意）
/// - `"none"` / 空 → `None`（worktree 不使用）
/// - その他 → リテラルブランチ名（後方互換）
pub fn resolve_worktree_branch(
    worktree_value: &str,
    block_name: &str,
    run_id: i64,
) -> Option<String> {
    match worktree_value {
        "isolation" => Some(format!("{}-{}", block_name, run_id)),
        "none" | "" => None,
        other => Some(other.to_string()),
    }
}

/// git リポジトリのルートを解決する（`git rev-parse --show-toplevel`）。
fn git_toplevel() -> Result<PathBuf, OrchestratorError> {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    git_toplevel_at(&cwd)
}

/// 指定ディレクトリを基準に git リポジトリのルートを解決する。
///
/// `cwd` がリポジトリのサブフォルダでも正しくルートを返す
/// （`git -C <dir> rev-parse --show-toplevel`）。
pub fn git_toplevel_at(dir: &Path) -> Result<PathBuf, OrchestratorError> {
    let output = Command::new("git")
        .args(["-C"])
        .arg(dir)
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .map_err(|e| OrchestratorError::CommandError(format!("Failed to run git: {}", e)))?;
    if output.status.success() {
        let s = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !s.is_empty() {
            return Ok(PathBuf::from(s));
        }
    }
    Err(OrchestratorError::CommandError(format!(
        "Cannot resolve a git repository: {}",
        dir.display()
    )))
}

/// 指定ブランチの git worktree を `<repo_root>/.worktrees/<branch>` に作成し、パスを返す。
///
/// 既に worktree が存在する場合は再利用する。
/// 作成した worktree のパスをエージェント起動時の cwd として使用する。
pub fn apply_worktree_at(repo_root: &Path, branch: &str) -> Result<PathBuf, OrchestratorError> {
    let worktrees_dir = repo_root.join(".worktrees");
    std::fs::create_dir_all(&worktrees_dir)?;

    let worktree_path = worktrees_dir.join(branch);
    if worktree_path.exists() {
        // 既存 worktree を再利用
        return Ok(worktree_path);
    }

    // 2. git worktree add（ブランチが無ければ -b で作成）
    let path_str = worktree_path.to_string_lossy().to_string();
    let status = Command::new("git")
        .args(["worktree", "add", "-b", branch])
        .arg(&path_str)
        .current_dir(repo_root)
        .status()
        .map_err(|e| {
            OrchestratorError::CommandError(format!("Failed to run git worktree: {}", e))
        })?;

    if !status.success() {
        // ブランチが既に存在する場合などは -b なしで再試行
        let status2 = Command::new("git")
            .args(["worktree", "add"])
            .arg(&path_str)
            .arg(branch)
            .current_dir(repo_root)
            .status()
            .map_err(|e| {
                OrchestratorError::CommandError(format!("Failed to run git worktree: {}", e))
            })?;
        if !status2.success() {
            return Err(OrchestratorError::CommandError(format!(
                "Failed to create the worktree: {}",
                branch
            )));
        }
    }

    Ok(worktree_path)
}

/// 指定ブランチの git worktree を（プロセスの cwd を含むリポジトリの）
/// `.worktrees/<branch>` に作成し、パスを返す（後方互換ラッパ）。
pub fn apply_worktree(branch: &str) -> Result<PathBuf, OrchestratorError> {
    let repo_root = git_toplevel()?;
    apply_worktree_at(&repo_root, branch)
}

/// 作業用コピー適用後の最終 cwd を求める。
///
/// `requested_cwd` を含むリポジトリの `.worktrees/<branch>` を作り、`requested_cwd`
/// がリポジトリのサブフォルダなら、作業用コピー内の同じサブフォルダを返す。
pub fn apply_worktree_for_cwd(
    requested_cwd: &Path,
    branch: &str,
) -> Result<PathBuf, OrchestratorError> {
    let repo_root = git_toplevel_at(requested_cwd)?;
    let worktree_root = apply_worktree_at(&repo_root, branch)?;
    // シンボリックリンクの差（macOS の `/var` → `/private/var` 等）を吸収してから
    // サブフォルダを求める。canonicalize できなければ字句的に試す。
    let canonical_cwd = requested_cwd
        .canonicalize()
        .unwrap_or_else(|_| requested_cwd.to_path_buf());
    let canonical_repo = std::fs::canonicalize(&repo_root).unwrap_or_else(|_| repo_root.clone());
    let sub = canonical_cwd
        .strip_prefix(&canonical_repo)
        .unwrap_or(Path::new(""));
    Ok(worktree_root.join(sub))
}

/// 作業用コピーの計画（親がマージに使う情報）。
#[derive(Debug, Clone, serde::Serialize)]
pub struct WorktreePlan {
    /// ブランチ名（`<block_type>-<execution_id>` またはリテラル）
    pub branch: String,
    /// 作業用コピーを作る対象リポジトリのルート
    pub repo_root: PathBuf,
    /// 作業用コピーのパス（`<repo_root>/.worktrees/<branch>`）
    pub worktree_path: PathBuf,
}

/// `cwd` / `worktree` 指定から作業用コピーの計画を求める（task_status 等の出力用）。
///
/// worktree を使わない場合（未指定・\"none\"）や git リポジトリでない場合は `None`。
pub fn worktree_plan(
    cwd: Option<&Path>,
    worktree_value: &str,
    block_type: &str,
    run_id: i64,
) -> Option<WorktreePlan> {
    let branch = resolve_worktree_branch(worktree_value, block_type, run_id)?;
    let base = cwd
        .map(PathBuf::from)
        .unwrap_or_else(crate::project::project_root);
    let repo_root = git_toplevel_at(&base).ok()?;
    let worktree_path = repo_root.join(".worktrees").join(&branch);
    Some(WorktreePlan {
        branch,
        repo_root,
        worktree_path,
    })
}

/// 子タスクが実行されたフォルダを返す（`p` の再開を起動時と同じ場所で行うため）。
///
/// worktree の実効値は `worktree_info` と同じく「タスクの指定 > ブロック定義 > none」。
/// 作業用コピーを使う子は、作業用コピー内の `task.cwd` に対応するフォルダ
/// （`apply_worktree_for_cwd` と同じ対応付け）。使わない子は `task.cwd`
/// （無ければプロジェクトルート）。返すフォルダが存在しなければ `None`。
pub fn subtask_run_cwd(
    task: &crate::db::models::Task,
    block_worktree: Option<&str>,
    run_id: i64,
) -> Option<PathBuf> {
    let effective = if !task.worktree.trim().is_empty() {
        task.worktree.trim()
    } else {
        block_worktree.unwrap_or("none")
    };
    let requested = task
        .cwd
        .as_deref()
        .map(PathBuf::from)
        .unwrap_or_else(crate::project::project_root);
    let dir = match worktree_plan(Some(&requested), effective, task.block_type.as_deref().unwrap_or(""), run_id) {
        Some(plan) => {
            let canonical_cwd = requested.canonicalize().unwrap_or_else(|_| requested.clone());
            let canonical_repo =
                std::fs::canonicalize(&plan.repo_root).unwrap_or_else(|_| plan.repo_root.clone());
            let sub = canonical_cwd
                .strip_prefix(&canonical_repo)
                .unwrap_or(Path::new(""));
            plan.worktree_path.join(sub)
        }
        None => requested,
    };
    dir.is_dir().then_some(dir)
}

/// 実行完了後の herdr pane を自動削除する（D5。sai-write 正常完了時のみ）。
///
/// pane_id が空の場合は何もしない。
pub fn close_execution_pane(pane_id: &str) {
    if pane_id.is_empty() {
        return;
    }
    let herdr = HerdrClient::new();
    match herdr.close_pane(pane_id) {
        Ok(()) => {
            eprintln!("[agent-link] pane を自動削除しました: {}", pane_id);
        }
        Err(e) => {
            eprintln!(
                "[agent-link] pane の自動削除に失敗（ベストエフォート）: {}: {}",
                pane_id, e
            );
        }
    }
}

/// メインブロックの Ctrl+D（2回目）によるユーザー明示完了（finalize）を実行する（U4・R8/R9）。
///
/// 1. `finalize_request` 通知を `pending/` に書き込む。
///    デーモンがこれを処理して実行監視スレッドへ `FinalizeRequested` を送信し、
///    `execute_completion_with_doc_write` のケース5（ユーザー要求による正常完了）で
///    `execute_block_pipeline` の Ok パス DB 更新順序（status=completed 等）を再利用する。
/// 2. 実行中 pane をベストエフォートで閉じる（エージェントのプロセス終了。ケース4 でも正常完了になる）。
/// 3. DB を直接 `completed` + `finalized` にする（デーモン非稼働時のフォールバック。
///    デーモン側も同様に completed にするため二重更新は無害）。
///
/// これにより Ctrl+D 終了でも `agent-link block list` が `完了` 表示になり、
/// 右SB の Session 行（session_dir は実行開始時に済み永続化済み）から再開可能になる。
pub fn request_main_block_finalize(store: &crate::db::SaiStore, run_id: i64, pane_id: &str) {
    // 1. finalize_request 通知（デーモン経由で正常完了 DB 更新順序を再利用）
    let _ = crate::daemon::write_finalize_request(run_id);

    // 2. pane を閉じる（ベストエフォート）
    close_execution_pane(pane_id);

    // 3. DB 更新（completed + finalized）
    let _ = store.update_execution_status(run_id, "completed");
    let _ = store.update_execution_finalized(run_id, true);
    eprintln!(
        "[agent-link] Ctrl+D finalize を実行しました: run_id={}, pane={}",
        run_id, pane_id
    );
}

// ============================================================================
// 共通実行パイプライン（メイン/サブ共通・§7.1.3）
// ============================================================================

/// ブロック実行パイプラインの共通関数。
///
/// メイン/サブの実行・完了処理を共通化します。
/// 内部で以下を実行します（§4.2 ケース1 の①〜③に相当）:
/// 1. 実行時設定の解決（`resolve_execution_settings`、§6）
/// 2. 実行レコード作成（`prepare_execution_record`、parent_run_id の有無のみ分岐）
/// 3. worktree 適用（指定時）
/// 4. パイプライン実行（`prepare_pipeline` + `spawn_pipeline_with_watcher` + join）
///    ※ 完了条件は「pi idle/done」ではなく「自分の run_id の sai-write 検出」（D4）
/// 5. sai-write 検出時: 出力抽出・統計抽出 → DB 更新（status=completed）
/// 6. sai-write 検出以外の終了（timeout / pane 強制削除）: DB 更新（status=failed + error_message）
///
/// pane の自動削除（⑤）と、サブブロックの results/ 通知（④）は
/// **呼び出し元**（デーモン / タスクワーカー）が順序を守って実行します。
///
/// # 引数
///
/// * `store` — DB ストア
/// * `parsed` — パース済みブロック定義
/// * `file_path` — ブロック定義ファイルのパス（block_metadata 登録用）
/// * `block_body` — プロンプト本文（プレースホルダーは呼び出し元で展開済み）
/// * `user_inputs` — 入力フィールド値のマップ
/// * `overrides` — 実行時オーバーライド（§6）
/// * `parent_run_id` — メイン: None / サブ: Some
/// * `batch_id` — タスクバッチ ID（P3-A。サブ実行のフェーズ合成に使用。メインは None）
/// * `tab_id` — バッチ実行時に事前作成された herdr タブの ID（U2・R4。None は従来の分割方式）
/// * `watcher_map` — デーモンの実行監視マップ（デーモン稼働時のみ。None なら自己ポーリング）
/// * `card_path` — kanban 実行に使ったカードのプロジェクト相対パス（R121 二段階更新用。None 可）
#[allow(clippy::too_many_arguments)]
pub fn execute_block_pipeline(
    store: &crate::db::SaiStore,
    parsed: &crate::parser::ParsedBlock,
    file_path: &Path,
    block_body: String,
    user_inputs: HashMap<String, String>,
    overrides: ExecutionOverrides,
    parent_run_id: Option<i64>,
    batch_id: Option<String>,
    tab_id: Option<String>,
    base_pane_id: Option<String>,
    pane_id: Option<String>,
    task_id: Option<i64>,
    watcher_map: Option<&RunningExecutions>,
    card_path: Option<String>,
) -> Result<PipelineResult, OrchestratorError> {
    execute_block_pipeline_full(
        store, parsed, file_path, block_body, user_inputs, overrides, parent_run_id,
        batch_id, tab_id, base_pane_id, pane_id, task_id, watcher_map, card_path, None,
    )
}

/// run フォルダに task.md が無ければ、カードと同じ形（`kanban::render_card`）で書く。
///
/// カード経由の実行は task.md を既に持つので何もしない。子タスクの `description` は
/// `task_create` の値（1 行目だけ・frontmatter は 1 行のため）。親子の関係は DB の `parent_run_id` が持つ。
/// 書けなくても実行は止めない。
fn write_run_task_md(
    store: &crate::db::SaiStore,
    run_id: i64,
    resolved: &ResolvedExecutionSettings,
    user_inputs: &HashMap<String, String>,
    task_id: Option<i64>,
) {
    let dir = match crate::run_dir::find_run_dir(run_id) {
        Some((_, d)) => d,
        None => match crate::run_dir::ensure_run_dir(crate::run_dir::RUNNING_OWNER, run_id) {
            Ok(d) => d,
            Err(_) => return,
        },
    };
    let path = dir.join(crate::run_dir::TASK_FILE);
    if path.exists() {
        return;
    }
    let mut fm = crate::kanban::CardFrontmatter::new_card(crate::kanban::card_creation_date());
    fm.execution_date = fm.creation_date.clone();
    fm.run_id = run_id.to_string();
    fm.block = resolved.config.block_id().to_string();
    fm.model = resolved.config.model.clone().unwrap_or_default();
    fm.thinking = resolved.config.thinking.clone().unwrap_or_default();
    let task = task_id.and_then(|t| store.get_task_by_id(t).ok());
    fm.description = task
        .as_ref()
        .map(|t| t.description.lines().next().unwrap_or("").trim().to_string())
        .unwrap_or_default();
    let task_text = user_inputs.get("task_text").map(|s| s.trim_end()).unwrap_or("");
    let card = crate::kanban::KanbanCard {
        frontmatter: fm,
        body: format!("# task_text\n\n{}", task_text),
    };
    let _ = std::fs::write(path, crate::kanban::render_card(&card));
}

/// 共通実行パイプラインの完全版（予約 run_id 付き・R302）。
///
/// `reserved_run_id` が `Some` なら予約行を確定させて使い回す。
/// `None` なら従来どおり新規発行する。
#[allow(clippy::too_many_arguments)]
pub fn execute_block_pipeline_full(
    store: &crate::db::SaiStore,
    parsed: &crate::parser::ParsedBlock,
    file_path: &Path,
    block_body: String,
    user_inputs: HashMap<String, String>,
    overrides: ExecutionOverrides,
    parent_run_id: Option<i64>,
    batch_id: Option<String>,
    tab_id: Option<String>,
    base_pane_id: Option<String>,
    pane_id: Option<String>,
    task_id: Option<i64>,
    watcher_map: Option<&RunningExecutions>,
    card_path: Option<String>,
    reserved_run_id: Option<i64>,
) -> Result<PipelineResult, OrchestratorError> {
    // 1. 実行時設定の解決（§6）
    let resolved = resolve_execution_settings(&parsed.config, &overrides);
    let timeout_ms = resolved.timeout_ms;
    let direction = resolved.direction;
    let worktree = resolved.worktree.clone();

    // 2. 実行レコード作成（予約 run があれば確定させて使い回す・R302）
    let card_path_from_request = card_path;
    let user_inputs_json = if user_inputs.is_empty() {
        None
    } else {
        serde_json::to_string(&user_inputs).ok()
    };
    let prep = prepare_execution_record_full(
        store,
        &resolved.config,
        file_path,
        user_inputs_json.as_deref(),
        parent_run_id,
        batch_id.as_deref(),
        reserved_run_id,
    )?;
    let run_id = prep.run_id;
    let block_id = prep.block_id;

    // U04: 実行レコード作成直後にタスクへ紐付ける。
    // 従来はパイプライン完了後（task_manager::execute_subtask の Ok パス）で
    // `update_task_execution_id` していたため、実行中はタスク未紐付になり、
    // `run_dir::subtask_confirmed` が未紐付→completed 扱いで `review` へ
    // 早期移動→ `session/*.jsonl` が `ENOENT` で止まる窓があった。
    // `task_id` がある場合（サブタスク経路）はここで即紐付けし、確定判定
    // （tasks.finalized / status）を正しく効かせる。
    if let Some(tid) = task_id {
        let _ = store.update_task_execution_id(tid, run_id);
    }

    // 実行レコード作成直後（実行前）にオーバーライド値を保存する。
    let _ = store.update_execution_overrides(
        run_id,
        resolved.config.model.as_deref(),
        resolved.config.thinking.as_deref(),
        resolved.config.timeout.map(|v| v as i32),
    );

    // kanban 実行時は実行開始直後に run フォルダを todo→running へ移動し（R403）、
    // DB の `kanban_card_path` を保存する。旧カード（`run_id: -`）は従来の書戻しを維持。
    if let Some(card_rel) = card_path_from_request.as_deref() {
        let _ = store.update_execution_kanban_info(run_id, Some(card_rel), "kanban");
        // 予約 run 方式：task.md は既に outputs/todo/<run_id>/ にあるため移動のみ
        if reserved_run_id.is_some() {
            let from = crate::run_dir::TODO_OWNER;
            let to = crate::run_dir::RUNNING_OWNER;
            // DB 更新済み→移動の順序（R404）。失敗時は警告に留め巻き戻さない
            // （実行自体は継続し、TUI は DB 状態で表示するため）
            if let Err(e) = crate::run_dir::move_run_dir(run_id, from, to) {
                eprintln!("[agent-link] run フォルダ移動に失敗 ({} → {}): {}", from, to, e);
            }
        } else {
            stamp_card_run_id(card_rel, run_id);
        }
    }

    // カードを使わない実行（task_create の子・直接実行）にも、カードと同じ形の task.md を
    // run フォルダに置く。task.md の置き場と形を全経路で 1 つにし、Web / TUI の task 表示を共通にする。
    write_run_task_md(store, run_id, &resolved, &user_inputs, task_id);

    // 3. worktree 適用（指定時。cwd を作業用コピー内の対応フォルダへ変更）。
    //    作業用コピーを作れない場合（git リポジトリでない等）は指定 cwd で
    //    続行する（1 つのエラーで全タスクが止まる方が問題が大きい）。
    //
    //    親が指定した作業フォルダ（overrides.cwd）を作業用コピーの基準にする。
    //    未指定ならプロジェクトルート。worktree は「指定された cwd を含む
    //    リポジトリ」に作る（親が別プロジェクトの cwd を指定できる）。
    let requested_cwd: PathBuf = overrides
        .cwd
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(crate::project::project_root);
    let cwd = if let Some(ref worktree_val) = worktree {
        match resolve_worktree_branch(worktree_val, &resolved.config.block_id(), run_id) {
            Some(branch) => match apply_worktree_for_cwd(&requested_cwd, &branch) {
                Ok(path) => path,
                Err(e) => {
                    eprintln!(
                        "[agent-link] 作業用コピーを作成できないため指定フォルダで実行します ({}): {}",
                        requested_cwd.display(),
                        e
                    );
                    requested_cwd.clone()
                }
            },
            None => requested_cwd.clone(),
        }
    } else {
        requested_cwd.clone()
    };

    // 4. 完了監視チャネルの登録（デーモン連携時）
    //    ※ この時点で pi は未起動のため、自分の run_id の file_written 通知が
    //       届くことはない（競合なし）。
    let completion_rx = if let Some(map) = watcher_map {
        let (tx, rx) = mpsc::channel::<CompletionEvent>();
        map.lock().unwrap().insert(run_id, tx);
        Some(rx)
    } else {
        None
    };

    // 5. パイプライン実行（sai-write 検出で完了）
    let pipeline_config = PipelineConfig {
        block_config: resolved.config.clone(),
        block_body,
        user_inputs,
        cwd: Some(cwd),
        direction,
        timeout_ms,
        keep_pane: resolved.keep_pane,
        run_id: Some(run_id),
        tab_id,
        base_pane_id,
        pane_id,
        is_subtask: parent_run_id.is_some(),
    };

    let start = Instant::now();
    let result = run_pipeline_with_watcher(
        pipeline_config,
        completion_rx,
        Some(&store),
        Some(run_id),
        task_id,
        parent_run_id,
    );

    // 実行監視マップから解除
    if let Some(map) = watcher_map {
        map.lock().unwrap().remove(&run_id);
    }

    let elapsed_ms = start.elapsed().as_millis() as u64;

    match result {
        Ok(mut pr) => {
            // 6. sai-write 検出（正常完了）→ ③ 実行結果の DB 保存
            pr.execution_id = Some(run_id);
            if pr.elapsed_ms == 0 {
                pr.elapsed_ms = elapsed_ms;
            }

            let _ = store.update_execution_output_content(run_id, &pr.output);
            let _ = store.update_execution_status(run_id, "completed");
            let _ = store.update_execution_output_type(run_id, &pr.output_type);
            if !pr.waggle_tokens.is_empty() {
                let _ = store.merge_execution_waggle_tokens(run_id, &pr.waggle_tokens);
            }
            if !pr.input_waggle_tokens.is_empty() {
                let _ = store.update_execution_input_tokens(run_id, &pr.input_waggle_tokens);
            }
            let _ = store.update_execution_pane(run_id, &pr.pane_id, 0);
            let _ = store.update_execution_git_commits(
                run_id,
                pr.git_commit_before.as_deref(),
                pr.git_commit_after.as_deref(),
            );
            insert_block_connections(store, run_id, block_id, &pr.input_waggle_tokens);
            // kanban 実行ならカードパスを残す（R121: 要求の `card_path` の実パス）
            if let Some(card_rel) = card_path_from_request.as_deref() {
                let _ = store.update_execution_kanban_info(run_id, Some(card_rel), "kanban");
            }
            // R121 二段階更新: `run_id` 確定後にカードへ書き戻す（旧カードのみ）。
            // 予約 run 方式では run_id 確定済みのため書戻し不要。
            // 再パースして frontmatter のみ更新し、本文には触らない。
            // 失敗時は警告に留める（ユーザー編集中の競合を許容）。
            if reserved_run_id.is_none() {
                if let Some(card_rel) = card_path_from_request.as_deref() {
                    stamp_card_run_id(card_rel, run_id);
                }
                // `stamp_card_run_id` 失敗時も DB の `kanban_card_path` があれば
                // カード側の `run_id` を補完する（TUI の todo/running 重複防止用）。
                if let Ok(exec_after) = store.get_execution_by_id(run_id) {
                    if let Some(card_rel) = exec_after.kanban_card_path.as_deref() {
                        stamp_card_run_id(card_rel, run_id);
                    }
                }
            }
            // running→review へ run フォルダを移動する（DB completed 更新後に移動・R404）。
            // 旧カード（新規発行 run）で run フォルダが無い場合は review owner に作成する。
            //
            // サブタスク（parent_run_id あり）はここでは移動しない。sai-write 検出時点は
            // 仮完了で pane は生きており、`session/*.jsonl` へ書き続けるため、
            // 移動すると ENOENT で停止する。移動は親の `task_manage(finalize)` /
            // `delete`（`run_dir::sync_run_owner_by_execution_id`）が行う。
            if parent_run_id.is_none() {
                let to = crate::run_dir::REVIEW_OWNER;
                if let Some((cur, _)) = crate::run_dir::find_run_dir(run_id) {
                    if cur != to {
                        if let Err(e) = crate::run_dir::move_run_dir(run_id, &cur, to) {
                            eprintln!("[agent-link] run フォルダ移動に失敗 ({} → {}): {}", cur, to, e);
                        }
                    }
                } else {
                    let _ = crate::run_dir::ensure_run_dir(to, run_id);
                }
            }
            if let Some(ref stats_json) = pr.session_stats_json {
                let _ = store.update_execution_session_stats(run_id, stats_json);
            }
            // 実行時オーバーライド値を保存（右SB表示用）
            let _ = store.update_execution_overrides(
                run_id,
                resolved.config.model.as_deref(),
                resolved.config.thinking.as_deref(),
                resolved.config.timeout.map(|v| v as i32),
            );
            // L1: 完了時に実行 usage（provider の実際の token / cost）を自動保存する。
            // ベストエフォート（セッションが無い・保存に失敗しても完了は止めない）。
            if let Err(e) = crate::usage::record_execution_usage(store, run_id) {
                eprintln!("[agent-link] 費用の記録に失敗 (run_id={}): {}", run_id, e);
            }

            Ok(pr)
        }
        Err(e) => {
            // 7. sai-write 検出以外の終了（timeout / pane 強制削除 / エラー）→ DB 更新
            let err_msg = format!("{}", e);
            eprintln!("[agent-link] 実行失敗 (run_id={}): {}", run_id, err_msg);
            let _ = store.update_execution_error(run_id, &err_msg, 1);
            // 失敗時もセッション統計を抽出して保存（TUI 右SB 表示用）
            // session_dir は `update_execution_session_info` で事前に保存済みだが、
            // run フォルダの owner 移動で古くなるため現行正本を解決し直す。
            if let Ok(exec) = store.get_execution_by_id(run_id) {
                // cc 実行は `~/.claude` 側の JSONL から統計を抽出する
                // （cwd は worktree を考慮しないプロジェクトルートで近似）
                let claude_json = exec.agent_session_id.as_deref().and_then(|sid| {
                    crate::session::claude_session_file(&crate::project::project_root(), sid)
                });
                if let Some(path) = claude_json {
                    if let Some(stats_json) = crate::session::extract_claude_session_stats(&path)
                        .ok()
                        .and_then(|s| serde_json::to_string(&s).ok())
                    {
                        let _ = store.update_execution_session_stats(run_id, &stats_json);
                    }
                } else {
                    let resolved = crate::session::resolve_session_dir(
                        exec.session_dir.as_deref(),
                        run_id,
                    );
                    if let Some(session_dir) = resolved {
                        if let Some(stats_json) = extract_and_serialize_stats(&session_dir) {
                            let _ = store.update_execution_session_stats(run_id, &stats_json);
                        }
                    }
                    // L1: 失敗実行も usage は保存する（成功時と同一の方針・ベストエフォート）
                    if let Err(e) = crate::usage::record_execution_usage(store, run_id) {
                        eprintln!("[agent-link] 費用の記録に失敗 (run_id={}): {}", run_id, e);
                    }
                }
            }
            Err(e)
        }
    }
}

/// 指定実行 ID に対応する run フォルダを削除する（R603）。
///
/// ブロック履歴の削除（`delete_execution_cascade`）と同時に呼び出し、
/// 対応する run フォルダ（task/review/message/session/成果物）を丸ごと消す。
/// フォルダが存在しない場合は正常に無視して続行する。
pub fn remove_run_dir_files(run_id: i64) -> std::io::Result<()> {
    let _ = crate::run_dir::remove_run_dir(run_id)?;
    // 旧 message も残っていれば消す（移行漏れ掃除）
    let old_message = crate::project::sai_path(MESSAGE_DIR).join(format!("{}.md", run_id));
    if old_message.exists() {
        let _ = std::fs::remove_file(old_message);
    }
    Ok(())
}

// ============================================================================
// message 記録ファイル（`.agent-link/message/<run_id>.md`）
// ============================================================================

/// message 記録ファイルを置くディレクトリ（旧パス・移行期フォールバック用）。
///
/// outputs 統一後の正本は `outputs/<owner>/<run_id>/message.md`。
/// `pi agent` session jsonl から user / assistant message のみを抽出した md を
/// 保存する。既存ファイルは上書きする（再生成で最新化）。
pub const MESSAGE_DIR: &str = ".agent-link/message";

/// 指定実行 ID に対応する message 記録ファイルの絶対パスを返す。
///
/// run フォルダ内の `message.md` を優先し、無ければ旧パスを返す（移行期互換）。
pub fn message_path(run_id: i64) -> PathBuf {
    if let Some((_, dir)) = crate::run_dir::find_run_dir(run_id) {
        return dir.join(crate::run_dir::MESSAGE_FILE);
    }
    crate::project::sai_path(MESSAGE_DIR).join(format!("{}.md", run_id))
}

/// 利用者の最初の発話が自動の節（`# context` か `# task_text`）で始まるかを返す。
///
/// 先頭の空白を無視して ` # context` / `# task_text` で始まり、その直後が改行か
/// 文字列の終わりなら真。`# contextual` のように語が繋がるものは偽。
fn starts_with_auto_section(text: &str) -> bool {
    let head = text.trim_start();
    for prefix in ["# context", "# task_text"] {
        if let Some(rest) = head.strip_prefix(prefix) {
            if rest.is_empty() || rest.starts_with('\n') || rest.starts_with('\r') {
                return true;
            }
        }
    }
    false
}

/// user / assistant メッセージ列から message md 本文を組み立てる（U03）。
///
/// 区切りは `# user (n)` / `# assistant (n)`。最初の利用者の発話が `# context` か
/// `# task_text` で始まるなら、その 2 節がそのまま最上位に出て、区切りは出ない。
/// それ以外は `# user (1)` として出す。区切り以外の本文の見出しは 1 段下げる
/// （最初の発話の内部は下げない）。
///
/// ```md
/// ---
/// block_name: test
/// run_id: 10
/// ---
///
/// # context
/// ## 今回の行動
///
/// # task_text
/// <依頼>
///
/// # assistant (1)
///
/// ## 結論
/// <本文>
///
/// # user (1)
///
/// ## 追加
/// <本文>
///
/// # assistant (2)
///
/// <本文>
/// ```
pub fn render_message_md(block_name: &str, run_id: i64, messages: &[(String, String)]) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "---\nblock_name: {}\nrun_id: {}\n---\n",
        block_name, run_id
    ));
    let mut user_n = 0u32;
    let mut asst_n = 0u32;
    // 最初の利用者の発話を 1 回だけ、自動の節として除外できるかどうかを調べる。
    let mut first_user_seen = false;
    for (role, text) in messages {
        if role == "user" {
            if !first_user_seen {
                first_user_seen = true;
                if starts_with_auto_section(text) {
                    // 中身はすでに正しい深さ（U01 が waggle の中で下げている）。下げない。
                    out.push_str(&format!("\n{}\n", text.trim()));
                    continue;
                }
            }
            user_n += 1;
            out.push_str(&format!(
                "\n# user ({})\n\n{}\n",
                user_n,
                shift_headings(text.trim(), 1)
            ));
        } else {
            asst_n += 1;
            out.push_str(&format!(
                "\n# assistant ({})\n\n{}\n",
                asst_n,
                shift_headings(text.trim(), 1)
            ));
        }
    }
    out
}

/// 実行の session jsonl から message md を生成し `.agent-link/message/<run_id>.md` に保存する。
///
/// `session_dir` が空・存在しない・jsonl が無い場合は空メッセージ列として
/// frontmatter のみの md を保存する。保存したパスを返す。
pub fn write_message_file(store: &crate::db::SaiStore, run_id: i64) -> std::io::Result<PathBuf> {
    let (block_name, session_dir, exec) = match store.get_execution_by_id(run_id) {
        Ok(exec) => {
            let name = store
                .get_block_by_id(exec.block_id)
                .map(|b| b.display_name)
                .unwrap_or_else(|_| format!("block#{}", exec.block_id));
            let dir = crate::session::resolve_session_dir(exec.session_dir.as_deref(), run_id)
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_default();
            (name, dir, Some(exec))
        }
        Err(e) => {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                e.to_string(),
            ))
        }
    };
    // cc は、その run の `agent_session_id` に対応する会話ファイル
    // （`resolve_transcript`）から抽出する。
    // `cc_session_files(...)`（`~/.claude/projects/` 全体で一番新しいもの）を使うと
    // 別の run の会話が出るため使わない。見つからないときは空にする。
    let cc_messages = exec.as_ref().and_then(|exec| {
        crate::phone_export::resolve_transcript(exec, &crate::project::project_root()).and_then(
            |path| {
                crate::session::extract_cc_user_assistant_messages(&path)
                    .ok()
                    .filter(|msgs| !msgs.is_empty())
            },
        )
    });
    let messages = if let Some(msgs) = cc_messages {
        msgs
    } else if session_dir.is_empty() {
        Vec::new()
    } else {
        crate::session::extract_user_assistant_messages(std::path::Path::new(&session_dir))
            .unwrap_or_default()
    };
    let body = render_message_md(&block_name, run_id, &messages);
    let path = message_path(run_id);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&path, body)?;
    Ok(path)
}

/// kanban カードへ `run_id` を書き戻す（R121 二段階更新の二段階目）。
///
/// プロジェクト相対パス（`.agent-link/todo/xxx.md`）を絶対化し、再パースして
/// frontmatter の `run_id` のみ更新する。本文には触らない。
/// ユーザー編集中の競合時は失敗しても警告に留め、呼び出し側で継続する。
fn stamp_card_run_id(card_rel: &str, run_id: i64) {
    let path = crate::project::project_root().join(card_rel);
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) => {
            eprintln!(
                "[agent-link] カード run_id 書戻し失敗（読込）: {}: {}",
                path.display(),
                e
            );
            return;
        }
    };
    let mut card = match crate::kanban::parse_card(&text) {
        Some(c) => c,
        None => {
            eprintln!(
                "[agent-link] カード run_id 書戻し失敗（パース）: {}",
                path.display()
            );
            return;
        }
    };
    card.frontmatter.run_id = run_id.to_string();
    if let Err(e) = std::fs::write(&path, crate::kanban::render_card(&card)) {
        eprintln!(
            "[agent-link] カード run_id 書戻し失敗（書込）: {}: {}",
            path.display(),
            e
        );
    }
}

// ============================================================================
// Public API 6: run_subtask — サブタスク実行
// ============================================================================

/// 子タスクの実行設定（`task_create` の要素から来る値）。
///
/// 各項目の `None` は「次の出所に従う」を表す（model / thinking はカード → ブロック定義）。
#[derive(Debug, Clone, Default)]
pub struct SubtaskSettings {
    /// model（`None` はカードの model、無ければブロック定義に従う）
    pub model: Option<String>,
    /// thinking（`None` はカードの thinking、無ければブロック定義に従う）
    pub thinking: Option<String>,
}

/// サブタスクを実行する簡易パイプライン。
///
/// ブロック定義名と入力からエージェントを起動し、結果を返します。
/// `TaskManager` のワーカー（デーモン）から呼び出され、AgentPool の管理下で実行されます。
/// 内部では共通の `execute_block_pipeline` に委譲します。
///
/// # 引数
///
/// * `store` — DB ストア
/// * `block_type` — 使用するブロック定義名（例: "sai-code"）
/// * `subject` — タスク件名（プロンプトとして注入）
/// * `description` — タスク詳細（プロンプトに追加）
/// * `inputs` — ブロック入力値（JSON）
/// * `parent_run_id` — 親実行 ID
/// * `worktree` — タスクの worktree 指定（"isolation" / "none" / ブランチ名）
/// * `task_cwd` — タスクの作業フォルダ（親が指定した絶対パス。None ならプロジェクトルート）
/// * `task_timeout` — タスクの時間上限（秒。親が `task_create` で指定。None ならブロック定義に従う）
/// * `settings` — 子の実行設定（`task_create` の model / thinking。None はカード → 既定の順）
/// * `batch_id` — タスクバッチ ID（P3-A。DAG のフェーズ合成に使用）
/// * `card_run_id` — 起こす todo カードの run_id（U2。`Some` なら予約済みの run_id と
///   `task.md`（`sci_*` を含む）をそのまま使う。`None` は従来どおり新規に run を作る）
/// * `tab_id` — バッチ実行時に事前作成された herdr タブの ID（U2・R4。None は従来の分割方式）
/// * `direction` — タブ内分割方向（U2・R4。タスクのバッチ内インデックスで交互分割。None はデフォルト）
/// * `watcher_map` — デーモンの実行監視マップ（デーモン稼働時のみ。None なら自己ポーリング）
#[allow(clippy::too_many_arguments)]
pub fn run_subtask(
    store: &crate::db::SaiStore,
    block_type: &str,
    subject: &str,
    description: &str,
    inputs: Option<&serde_json::Value>,
    parent_run_id: i64,
    worktree: Option<String>,
    task_cwd: Option<String>,
    task_timeout: Option<u32>,
    settings: SubtaskSettings,
    batch_id: Option<String>,
    card_run_id: Option<i64>,
    tab_id: Option<String>,
    base_pane_id: Option<String>,
    pane_id: Option<String>,
    direction: Option<SplitDirection>,
    task_id: Option<i64>,
    watcher_map: Option<&RunningExecutions>,
) -> Result<PipelineResult, OrchestratorError> {
    // ブロック定義ファイルのパスを解決
    let block_path = resolve_block_path(block_type);

    // ブロック定義をパース（存在しない場合はエラー）
    if !block_path.exists() {
        return Err(OrchestratorError::CommandError(format!(
            "Block definition file not found: {} -> {}",
            block_type,
            block_path.display()
        )));
    }

    // ブロック定義を読む直前に準備コマンドを走らせる（作り直したテンプレートを
    // そのまま差し込むため）。失敗しても前回のファイルのまま実行を続ける。
    if let crate::pre_run::PreRunOutcome::Failed(msg) = crate::pre_run::run_if_configured() {
        eprintln!(
            "[agent-link] 警告: 実行直前の準備コマンドに失敗しました（前回のファイルのまま続けます）: {msg}"
        );
    }

    let parsed = crate::parser::parse_block_file(&block_path).map_err(|e| {
        OrchestratorError::CommandError(format!("Failed to parse the block definition: {}", e))
    })?;

    // P2: サブブロック呼び出し制限（subblocks）と入力検証
    // 親ブロック定義の `subblocks` に block_type が含まれるか、
    // タスクの inputs がブロック定義の入力仕様に適合するかを検証する。
    validate_subtask_block(store, parent_run_id, block_type)?;

    // プロンプトを構築（P1-4: 自動「# タスク: subject」セクションは廃止。
    // ブロック定義本文中の {{subject}} / {{description}} を展開する）
    let body = crate::placeholder::expand_task_placeholders(&parsed.body, subject, description);

    // ユーザー入力の変換
    let mut user_inputs: HashMap<String, String> = HashMap::new();
    if let Some(inp) = inputs {
        if let Some(obj) = inp.as_object() {
            for (key, value) in obj {
                let val_str = match value {
                    serde_json::Value::String(s) => s.clone(),
                    other => other.to_string(),
                };
                user_inputs.insert(key.clone(), val_str);
            }
        }
    }

    // U2: todo のカードを自分の子として起こす場合の実行設定。
    // カード（`.agent-link/outputs/todo/<run_id>/task.md`）の model / thinking があれば
    // ブロック定義より優先する（`kanban_run::run_card_request` と同じ扱い）。
    // 空欄のときはブロック定義の値に任せる。
    let card = card_run_id
        .map(|rid| read_todo_card(rid))
        .transpose()
        .map_err(|m| OrchestratorError::CommandError(m))?;

    // model / thinking: task_create の値 > カードの task.md の値 > 無し（ブロック定義に任せる）
    let overrides = ExecutionOverrides {
        model: subtask_setting(
            settings.model,
            card.as_ref().map(|c| c.frontmatter.model.as_str()),
        ),
        thinking: subtask_setting(
            settings.thinking,
            card.as_ref().map(|c| c.frontmatter.thinking.as_str()),
        ),
        // timeout はフロントマターの値を尊重する（§7.1.4 の修正点）
        // ただしタスクの timeout（task_create）があればブロック定義より優先する
        timeout: task_timeout,
        // U2・R4: タブ内で交互分割する場合は方向を明示指定（None ならデフォルト）
        direction,
        worktree,
        cwd: task_cwd,
        keep_pane: false,
    };

    // カード指定の子（U2）は、予約済みの run_id と run フォルダの task.md を使い回す。
    // run フォルダの todo → running 移動と `execution_mode = 'kanban'` は
    // `execute_block_pipeline_full` が行う（カード無しの子と同じ経路を通す）。
    let card_rel = card_run_id.map(todo_card_relative_path);
    execute_block_pipeline_full(
        store,
        &parsed,
        &block_path,
        body,
        user_inputs,
        overrides,
        Some(parent_run_id),
        batch_id,
        tab_id,
        base_pane_id,
        pane_id,
        task_id,
        watcher_map,
        card_rel,
        card_run_id,
    )
}

/// 子の model / thinking を決める。`task_create` の値 > カードの値（空は無し）> 無し（ブロック定義に任せる）。
fn subtask_setting(task_value: Option<String>, card_value: Option<&str>) -> Option<String> {
    task_value.or_else(|| card_value.and_then(non_empty_str))
}

/// 前後の空白を除いて空でない値だけ `Some` にする（カードの設定を読むため）。
fn non_empty_str(s: &str) -> Option<String> {
    let t = s.trim();
    if t.is_empty() {
        None
    } else {
        Some(t.to_string())
    }
}

/// todo のカード（`.agent-link/outputs/todo/<run_id>/task.md`）の絶対パスを返す。
pub fn todo_card_path(card_run_id: i64) -> PathBuf {
    crate::project::sai_path(&format!(
        "{}/{}/{}/{}",
        crate::run_dir::OUTPUTS_DIR,
        crate::run_dir::TODO_OWNER,
        card_run_id,
        crate::run_dir::TASK_FILE,
    ))
}

/// todo のカード（`.agent-link/outputs/todo/<run_id>/task.md`）のプロジェクト相対パスを返す。
///
/// `execute_block_pipeline_full` は `card_path` をプロジェクト相対で受け取る（DB の
/// `kanban_card_path` 用）。
fn todo_card_relative_path(card_run_id: i64) -> String {
    todo_card_path(card_run_id)
        .strip_prefix(crate::project::project_root())
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| {
            format!(
                "{}/{}/{}/{}",
                crate::run_dir::OUTPUTS_DIR,
                crate::run_dir::TODO_OWNER,
                card_run_id,
                crate::run_dir::TASK_FILE,
            )
        })
}

/// todo のカードを読み、`run_id` がカードの指定と一致することを確認する。
///
/// カードの `run_id` は todo のフォルダ名と同じでなければならない。違うカードを指されれば止める。
pub fn read_todo_card(card_run_id: i64) -> Result<crate::kanban::KanbanCard, String> {
    let path = todo_card_path(card_run_id);
    let text = std::fs::read_to_string(&path)
        .map_err(|_| format!("Card {} not found in todo.", card_run_id))?;
    let card = crate::kanban::parse_card(&text)
        .ok_or_else(|| format!("Cannot read task.md of card {}.", card_run_id))?;
    if card.frontmatter.run_id.trim() != card_run_id.to_string() {
        return Err(format!(
            "Card {} not found in todo (run_id in task.md: {}).",
            card_run_id,
            card.frontmatter.run_id.trim()
        ));
    }
    Ok(card)
}

// ============================================================================
// サブタスク検証（P2: subblocks 制限・入力検証）
// ============================================================================

/// サブタスクのブロック呼び出し制限を検証する。
///
/// 親実行のブロック定義に `subblocks` が指定されている場合、
/// その一覧に `block_type` が含まれることを確認する。
/// 未指定の場合は全ブロック許可（後方互換）。
fn validate_subtask_block(
    store: &crate::db::SaiStore,
    parent_run_id: i64,
    block_type: &str,
) -> Result<(), OrchestratorError> {
    // 親実行 → 親ブロックを取得
    let parent_exec = store.get_execution_by_id(parent_run_id).map_err(|e| {
        OrchestratorError::CommandError(format!("Failed to get parent run {}: {}", parent_run_id, e))
    })?;
    let parent_block = store.get_block_by_id(parent_exec.block_id).map_err(|e| {
        OrchestratorError::CommandError(format!(
            "Failed to get parent block {}: {}",
            parent_exec.block_id, e
        ))
    })?;

    // 親ブロック定義書をパースし、subblocks を確認
    let parent_path = resolve_block_path(&parent_block.name);
    if !parent_path.exists() {
        // 親定義ファイルが無い場合は制限なし（DB メタデータ由来のみ）
        return Ok(());
    }
    let parsed = crate::parser::parse_block_file(&parent_path).map_err(|e| {
        OrchestratorError::CommandError(format!("Failed to parse the parent block definition: {}", e))
    })?;

    is_subblock_allowed(&parent_block.name, &parsed.config.subblocks, block_type)
}

/// subblocks 制限の判定（純関数。テスト容易性のため分離）。
///
/// `parent_subblocks` が `None`（未指定）ならサブブロック呼び出し**不可**。
/// ブロック定義書に `subblocks:` の一覧が記載された場合のみ、
/// その一覧に含まれるブロックがサブタスクとして実行できる。
fn is_subblock_allowed(
    parent_name: &str,
    parent_subblocks: &Option<Vec<String>>,
    block_type: &str,
) -> Result<(), OrchestratorError> {
    match parent_subblocks {
        // 未指定（None）: subblocks 自体が定義されていない → サブブロックは呼び出せない
        None => Err(OrchestratorError::CommandError(format!(
            "Block '{}' defines no subblocks, so it cannot call subblocks.\n\
             List the callable blocks under subblocks: in the block definition.",
            parent_name
        ))),
        Some(allowed) => {
            if allowed.iter().any(|b| b == block_type) {
                Ok(())
            } else {
                Err(OrchestratorError::CommandError(format!(
                    "Block '{}' is not in the subblocks of parent block '{}', so it cannot be called. Allowed: {}",
                    block_type,
                    parent_name,
                    if allowed.is_empty() {
                        "(none)".to_string()
                    } else {
                        allowed.join(", ")
                    }
                )))
            }
        }
    }
}

/// ブロック定義名からファイルパスを解決する。
///
/// 検索順序:
/// 1. `.agent-link/blocks/<block_type>.md`
/// 2. `blocks/<block_type>.md`
/// 3. `lib/herdrs/blocks/<block_type>.md`
/// 4. `~/.agent-link/blocks/<block_type>.md`（グローバル・全プロジェクトで使用可能）
pub fn resolve_block_path(block_type: &str) -> std::path::PathBuf {
    // まず .agent-link/blocks/ ディレクトリを検索（プロジェクトルート基準）
    let path0 = crate::project::sai_path(".agent-link/blocks").join(format!("{}.md", block_type));
    if path0.exists() {
        return path0;
    }

    // blocks/ ディレクトリを検索
    let path1 = std::path::PathBuf::from("blocks").join(format!("{}.md", block_type));
    if path1.exists() {
        return path1;
    }

    // lib/herdrs/blocks/ を検索
    let path2 = std::path::PathBuf::from("lib/herdrs/blocks").join(format!("{}.md", block_type));
    if path2.exists() {
        return path2;
    }

    // グローバル ~/.agent-link/blocks/ を検索（全プロジェクトで使用可能）
    if let Some(dir) = crate::project::sai_home_path("blocks") {
        let path3 = dir.join(format!("{}.md", block_type));
        if path3.exists() {
            return path3;
        }
    }

    // 見つからなければ .agent-link/blocks/ を返す（エラーは呼び出し元で処理）
    path0
}

// ============================================================================
// 内部: バックグラウンド実行の本体
// ============================================================================

/// 別スレッドで実行される実際のパイプライン処理。
///
/// この関数は `spawn_pipeline()` / `spawn_pipeline_with_watcher()` から
/// `std::thread::spawn` で呼び出されます。
/// キャンセルフラグを定期的にチェックし、中断要求があればクリーンアップして
/// 戻ります。進捗は `progress` に書き込まれ、TUI がポーリングで取得します。
///
/// `completion_rx` が Some の場合（デーモン連携）、sai-write の完了判定を
/// デーモンの通知処理ループからのイベントで行います（§4.2 ケース1）。
fn execute_pipeline_background(
    prepared: PreparedPipeline,
    _direction: SplitDirection,
    timeout_ms: u64,
    keep_pane: bool,
    progress: Arc<Mutex<PipelineProgress>>,
    cancel_flag: Arc<AtomicBool>,
    completion_rx: Option<mpsc::Receiver<CompletionEvent>>,
) {
    let start = Instant::now();
    let herdr = HerdrClient::new();

    // ---- キャンセルチェックマクロ ----
    macro_rules! check_cancel {
        ($label:expr) => {
            if cancel_flag.load(Ordering::SeqCst) {
                eprintln!("[agent-link] {} でキャンセルされました", $label);
                *progress.lock().unwrap() = PipelineProgress::Cancelled;
                return;
            }
        };
    }

    // ---- Step 6: 擬似 pane を用意 ----
    check_cancel!("pane用意前");
    *progress.lock().unwrap() = PipelineProgress::SplittingPane;

    let cwd_str = prepared
        .cwd
        .as_ref()
        .map(|p| p.to_string_lossy().to_string());
    // pane_id 指定あり（task_manager が用意した擬似 pane）は状態を読むだけ。
    // 無ければ擬似 pane を新しく作る（herdr の分割は呼ばない）。
    let pane_info = if let Some(pid) = prepared.pane_id.as_deref() {
        match herdr.get_pane_status(pid) {
            Ok(p) => p,
            Err(e) => {
                let msg = format!("Failed to get the pane to use (pane={}): {}", pid, e);
                *progress.lock().unwrap() = PipelineProgress::Failed { error: msg };
                return;
            }
        }
    } else {
        match crate::headless::new_pane(cwd_str.as_deref()) {
            Ok(p) => p,
            Err(e) => {
                let msg = format!("Failed to create a pseudo pane: {}", e);
                *progress.lock().unwrap() = PipelineProgress::Failed { error: msg };
                return;
            }
        }
    };

    // ---- ペインにラベルを設定（互換のため呼ぶだけ。herdr は使わない） ----
    if let Some(run_id) = prepared.run_id {
        let label = format!("sai-{}-{}", prepared.block_name, run_id);
        let _ = herdr.rename_pane(&pane_info.pane_id, &label);
    }
    // sai-phone 書き出し用に pane_id を run セッション dir へ控える。
    // DB の herdr_pane_id は完了時にしか入らず，ラベルはプロジェクト間で重なるため。
    // DB は task_manager の分岐に使われるので触らない。失敗しても実行は続ける。
    let _ = std::fs::write(
        prepared.session_dir.join(crate::phone_export::PANE_ID_FILE),
        &pane_info.pane_id,
    );

    // ---- Step 8: エージェント起動 ----
    // 起動コマンドの先頭で必ず作業ディレクトリへ `cd` する。
    let launch_command = match cwd_str.as_deref() {
        Some(cwd) => format!(
            "cd {} && {}",
            crate::herdr::shell_escape_single_quotes(cwd),
            prepared.pi_command
        ),
        None => prepared.pi_command.clone(),
    };
    if let Err(e) = herdr.run_in_pane(&pane_info.pane_id, &launch_command) {
        let msg = format!("Failed to start the agent: {}", e);
        *progress.lock().unwrap() = PipelineProgress::Failed { error: msg };
        return;
    }

    // ---- Step 10: アイドル状態確認 (30秒タイムアウト、中断可能) ----
    match wait_for_status_cancelable(
        &herdr,
        &pane_info.pane_id,
        &["idle", "done"],
        30_000,
        "アイドル確認",
        &progress,
        &cancel_flag,
    ) {
        Ok(()) => {}
        Err(OrchestratorError::Timeout { .. }) => {
            // タイムアウトは警告ログ相当だが続行（TUI には表示しない）
        }
        Err(OrchestratorError::Cancelled) => {
            *progress.lock().unwrap() = PipelineProgress::Cancelled;
            return;
        }
        Err(e) => {
            *progress.lock().unwrap() = PipelineProgress::Failed {
                error: e.to_string(),
            };
            return;
        }
    }

    // ---- Step 11: プロンプト送信（送信確認 + 再送つき、F6） ----
    check_cancel!("プロンプト送信前");
    *progress.lock().unwrap() = PipelineProgress::Waiting {
        step: "プロンプト送信".to_string(),
        elapsed_secs: 0,
    };

    match submit_prompt_verified(
        &herdr,
        &pane_info.pane_id,
        &prepared.expanded_prompt,
        prepared.submit_key,
        Some(&cancel_flag),
    ) {
        Ok(()) => {}
        Err(OrchestratorError::Cancelled) => {
            *progress.lock().unwrap() = PipelineProgress::Cancelled;
            return;
        }
        Err(e) => {
            // 最初の文を確定できなかった run の session は残さない（この run の pane だけ）。
            close_execution_pane(&pane_info.pane_id);
            *progress.lock().unwrap() = PipelineProgress::Failed {
                error: e.to_string(),
            };
            return;
        }
    }

    // ---- Step 13-15: 完了待機 → 出力抽出 → クリーンアップ ----
    // 完了判定は「pi idle/done」ではなく「自分の run_id の sai-write 検出」（D4）。
    // `execute_completion_normal` は廃止し、全ブロックを sai-write 方式に統一する（§7.1.3・D22）。
    execute_completion_with_doc_write(
        &herdr,
        &pane_info.pane_id,
        timeout_ms,
        keep_pane,
        &prepared,
        &progress,
        &cancel_flag,
        start,
        completion_rx,
    );
}

// ============================================================================
// 内部: 完了フェーズ（sai-write 検出による完了判定・§4）
// ============================================================================

/// サブブロック停止理由（U3）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StallReason {
    /// 1分間 `idle` が継続（出力なしで停止）
    Idle,
    /// `blocked` 状態（連続2回検出で即時通知）
    Blocked,
    /// `error` 状態（連続2回検出で即時通知）
    Error,
    /// `working` が long_run の間続いた（working が続く限り long_run ごとに繰り返す）。
    /// 値は working が続いている時間。
    LongRunning(Duration),
}

/// サブブロック停止検知の状態機械（U3）。
///
/// 完了待機ループで 250ms ごとに `observe` を呼び、`idle` が閾値（既定60秒）
/// 継続または `blocked` 連続2回で `Some(停止理由)` を返す。以後は `notified` により
/// 再送しない（介入待ち・スパム防止）。
///
/// `working` が続く場合は `long_run`（既定20分）ごとに `LongRunning` を返す。
/// これは `notified` とは独立で、`working` が続く限り繰り返す（U2）。
/// `working` を挟むと `notified` は False に戻り、介入後の再停止を再び通知する。
///
/// `now` を引数で受け取るため、時計を進めた疑似テストが可能（実睡不要）。
#[derive(Debug, Default)]
struct StallDetector {
    /// 最後に `idle` を観測した時刻（None = idle していない）
    last_idle_at: Option<Instant>,
    /// `blocked` の連続観測回数
    blocked_count: u32,
    /// 停止通知を送信済みか（`true` なら以降は通知しない）
    notified: bool,
    /// `working` を観測し始めた時刻（`working` 以外で None に戻る）
    working_since: Option<Instant>,
    /// 最後に LongRunning を通知した時刻
    last_long_alert_at: Option<Instant>,
}

impl StallDetector {
    /// 各ポーリング結果を処理し、停止通知を送信すべきなら `Some(理由)` を返す。
    ///
    /// `long_run` は `working` が何分続いたら `LongRunning` を返すかの間隔。
    /// `working` が続く限りこの間隔ごとに繰り返し返す（`notified` とは独立）。
    fn observe(
        &mut self,
        status: &str,
        now: Instant,
        idle_timeout: Duration,
        long_run: Duration,
    ) -> Option<StallReason> {
        match status {
            "working" => {
                // 介入で子が動き出した可能性があるため、停止通知済みフラグを戻す
                self.blocked_count = 0;
                self.last_idle_at = None;
                self.notified = false;
                if self.working_since.is_none() {
                    self.working_since = Some(now);
                }
                // 基準時刻 = 直前の LongRunning 通知があればそれ、なければ working 開始時刻
                let base = self.last_long_alert_at.or(self.working_since).unwrap_or(now);
                if now.duration_since(base) >= long_run {
                    self.last_long_alert_at = Some(now);
                    return Some(StallReason::LongRunning(
                        now.duration_since(self.working_since.unwrap_or(now)),
                    ));
                }
                None
            }
            "blocked" | "error" | "idle" => {
                self.working_since = None;
                self.last_long_alert_at = None;
                if self.notified {
                    return None;
                }
                if status == "blocked" || status == "error" {
                    self.blocked_count += 1;
                    if self.blocked_count >= 2 {
                        self.notified = true;
                        if status == "error" {
                            Some(StallReason::Error)
                        } else {
                            Some(StallReason::Blocked)
                        }
                    } else {
                        None
                    }
                } else {
                    // idle
                    self.blocked_count = 0;
                    match self.last_idle_at {
                        None => {
                            self.last_idle_at = Some(now);
                            None
                        }
                        Some(start) => {
                            if now.duration_since(start) >= idle_timeout {
                                self.notified = true;
                                Some(StallReason::Idle)
                            } else {
                                None
                            }
                        }
                    }
                }
            }
            _ => {
                // done など → idle タイマーと blocked カウントをリセット
                self.blocked_count = 0;
                self.last_idle_at = None;
                self.working_since = None;
                self.last_long_alert_at = None;
                None
            }
        }
    }
}

/// 長時間アラートの間隔（既定 20 分。`AGENT_LINK_LONG_RUN_ALERT_MS` で上書き可能）。
fn long_run_alert_ms() -> u64 {
    std::env::var("AGENT_LINK_LONG_RUN_ALERT_MS")
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .filter(|v| *v > 0)
        .unwrap_or(1_200_000)
}

/// idle の停止とみなす時間（ミリ秒）。既定 60 秒。`AGENT_LINK_IDLE_TIMEOUT_MS` で上書き可能。
///
/// `agent-link run wait` も同じ値で止まりを判定するため、ここを正本にする。
pub(crate) fn stall_idle_ms() -> u64 {
    std::env::var("AGENT_LINK_IDLE_TIMEOUT_MS")
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .filter(|v| *v > 0)
        .unwrap_or(60_000)
}

/// 画面の行に cc の認証切れの表示があれば、その行を返す。
///
/// 401 で止まった cc は Stop フックを呼ばず `working` のまま残るので、状態では分からない。
// ponytail: 画面の文字で判定する。agent がこの文言をファイルの内容として表示すると誤検出する。
//           版で表示が変わったら正規表現を足す
pub(crate) fn auth_error_line(screen: &str) -> Option<String> {
    let re = regex::Regex::new(r"API Error: 401|Invalid bearer token|Please run /login").ok()?;
    screen
        .lines()
        .find(|line| re.is_match(line))
        .map(|line| line.trim().to_string())
}

/// pane が消えたときの判定。finalize の要求より前に消えたなら失敗の理由、後なら `None`（完了の流れへ）。
pub(crate) fn pane_gone_error(finalize_requested: bool) -> Option<&'static str> {
    if finalize_requested {
        None
    } else {
        Some("The agent exited before finalize (the pane is gone).")
    }
}

/// `long_running` 通知の本文。
fn long_running_message(elapsed: Duration, pane_id: &str) -> String {
    format!(
        "サブブロックが {} 分以上 working のままです（pane={}）。agent-link task capture <ID> で画面を確かめてください。進んでいれば `agent-link task wait <ID>` で待ち続けてかまいません。1 つの命令が終わらずに止まっているなら agent-link task interrupt <ID> で止め、agent-link task send <ID> --message <文> で原因を調べさせてください。",
        elapsed.as_secs() / 60,
        pane_id
    )
}

/// timeout 超過通知の本文（子ブロック用）。
///
/// 子ブロックは timeout を超えても failed にせず、この通知を同じ間隔で
/// 繰り返し親へ送る（続けるか止めるかは親が決める）。
fn timeout_alert_message(timeout_ms: u64, elapsed: Duration, pane_id: &str) -> String {
    format!(
        "サブブロックが timeout（{} 秒）を超えても完了していません（経過 {} 分、pane={}）。agent-link task capture <ID> で画面を確かめてください。時間がかかる理由がもっともなら、そのまま `agent-link task wait <ID>` で待ってください（同じ間隔でまた知らせます）。止めるなら agent-link task interrupt <ID>、終了させるなら agent-link task delete <ID> を使ってください。",
        timeout_ms / 1000,
        elapsed.as_secs() / 60,
        pane_id
    )
}

/// SaiWriteDetected 受信時に `run_batches` を finalized にしてよいか。
///
/// `run_batches` はメインブロック（親実行なし）の TUI フェーズ表示専用のため、
/// サブブロック（親実行あり）では触らない（P6-2）。
fn should_finalize_run_batch(parent_run_id: Option<i64>) -> bool {
    parent_run_id.is_none()
}

/// sai-write モード: sai-write 検出（またはデーモンからの完了イベント）で完了する。
///
/// `completion_rx` が Some の場合（デーモン連携）:
/// - 完了判定 = 自分の run_id の sai-write 検出イベント（D4・§4.2 ケース1）
/// - waggle トークン化（①）はデーモンのポーリングループが実施済み
/// - timeout 超過 → ケース2（ユーザー通知）
/// - pane 消滅 → ケース3（強制終了）
///
/// `completion_rx` が None の場合（デーモン非使用）:
/// 従来どおり通知ディレクトリを自己ポーリングし、pi idle/done でも完了する
/// （後方互換・テスト用）。
/// セッションファイルとペインは保持する（`pi resume` での復元用）。
///
/// `completion_rx` が Some の場合（デーモン使用）:
/// SaiWriteDetected イベントを待つ。ユーザーが pane を削除した場合（§4.2 ケース4）、
/// その時点で利用可能な出力を抽出して正常完了とみなす。これにより、
/// ユーザーが ctrl+d で pi を終了 → pane 削除 した場合でも、
/// "completed" ステータスになる。
fn execute_completion_with_doc_write(
    herdr: &HerdrClient,
    pane_id: &str,
    timeout_ms: u64,
    _keep_pane: bool,
    prepared: &PreparedPipeline,
    progress: &Arc<Mutex<PipelineProgress>>,
    cancel_flag: &Arc<AtomicBool>,
    start: Instant,
    completion_rx: Option<mpsc::Receiver<CompletionEvent>>,
) {
    // waggle インテグレーターを初期化
    let waggle_db_path = crate::project::db_path();
    if let Some(parent) = waggle_db_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let integrator = match WaggleIntegrator::open(&waggle_db_path) {
        Ok(i) => i,
        Err(e) => {
            *progress.lock().unwrap() = PipelineProgress::Failed {
                error: format!("Failed to initialize the waggle DB: {}", e),
            };

            return;
        }
    };

    // sai-write 通知ディレクトリを事前作成
    let _ = doc_write_notify::ensure_pending_dir();
    let _ = doc_write_notify::ensure_processed_dir();

    let run_start = Instant::now();
    // timeout 超過時、子ブロックは失敗にせず通知を繰り返し送るので deadline を延ばす
    let mut deadline = Instant::now() + Duration::from_millis(timeout_ms);
    let mut waggle_tokens: Vec<String> = Vec::new();
    // pane 消滅判定の連続エラーカウント（一時的な herdr エラーへの耐性）
    let mut pane_error_count: u32 = 0;
    // 認証切れの画面確認を次にいつ行うか（30 秒ごと）
    let mut next_screen_check = Instant::now();
    // 前回 401 の文言が出ていた画面の末尾。次の確認でも同じなら止まっているとみなす
    let mut last_auth_screen: Option<String> = None;

    // U3: サブブロック停止検知の閾値（既定 60 秒。`AGENT_LINK_IDLE_TIMEOUT_MS` で上書き可能）
    let idle_timeout_ms = stall_idle_ms();
    // U2: 長時間アラートの間隔（既定 20 分。`AGENT_LINK_LONG_RUN_ALERT_MS` で上書き可能）
    let long_run_ms = long_run_alert_ms();
    let mut stall = StallDetector::default();
    // U3: 停止通知（idle/blocked）を `results/` に書き込み、メインの task_output が受信できるようにする
    let stalled_task_id = prepared.task_id;
    let stalled_parent_run_id = prepared.parent_run_id;

    // ── 完了待機ループ ──
    if let Some(rx) = completion_rx {
        // ── デーモン連携モード: 完了判定はデーモンの通知処理からのイベント（§4.2） ──
        // waggle トークン化（①）はデーモンのポーリングループが実施済みのため、
        // ここでは完了イベントの受信のみを待つ。
        loop {
            // キャンセルチェック
            if cancel_flag.load(Ordering::SeqCst) {
                *progress.lock().unwrap() = PipelineProgress::Cancelled;

                return;
            }

            // タイムアウトチェック（§4.2 ケース2）
            if Instant::now() >= deadline {
                // 子ブロック（親実行あり）は failed にせず、stalled(timeout) を
                // 送って deadline を延ばす。続けるか止めるかは親が決める。
                if let (Some(tid), Some(pid)) = (stalled_task_id, stalled_parent_run_id) {
                    let msg = timeout_alert_message(timeout_ms, run_start.elapsed(), pane_id);
                    eprintln!(
                        "[agent-link] サブブロック timeout 超過 (task_id={}, reason=timeout): {}",
                        tid, msg
                    );
                    let _ = crate::task_manager::write_stalled_notification(
                        tid,
                        Some(pid),
                        "timeout",
                        &msg,
                            None,
                    );
                    deadline = Instant::now() + Duration::from_millis(timeout_ms);
                } else {
                    *progress.lock().unwrap() = PipelineProgress::Failed {
                        error: format!("Timed out: no completion by finalize (agent-link run finalize) was detected within {}ms. Handle the rest manually.", timeout_ms),
                    };

                    return;
                }
            }

            // 進捗を更新
            let running_secs = run_start.elapsed().as_secs();
            *progress.lock().unwrap() = PipelineProgress::Running {
                elapsed_secs: running_secs,
            };

            // 完了イベント待ち（250ms タイムアウトでポーリング）
            match rx.recv_timeout(Duration::from_millis(250)) {
                Ok(CompletionEvent::SaiWriteDetected { waggle_token, .. }) => {
                    if let Some(ref token) = waggle_token {
                        waggle_tokens.push(token.clone());
                        eprintln!("[agent-link] sai-write 検出（デーモン）: token={}", token);
                    }
                    // P6-2: サブブロックも task_report（FinalizeRequested）で完了する。
                    // sai-write はトークンの収集だけを行い、ここでは完了しない。
                    // - メインブロック（親実行なし）: ユーザーが任意のタイミングで
                    //   `/agent-link-finalize` を実行して最終化するまでエージェントは継続する。
                    // - サブブロック（親実行あり）: task_report が finalize_request を書くまで継続する。
                    //   pane は維持され、メインブロックが `task_manage(finalize)` で削除・確定する。
                    //
                    // メインブロックのみ、sai-write 検出で最新の open 実行バッチを finalized にする。
                    // TUI はこのフラグで最終フェーズノード（P_n = sai-write 由来）を合成する。
                    // サブブロックでは親の実行バッチを触らない（run_batches はメイン専用）。
                    if should_finalize_run_batch(prepared.parent_run_id) {
                        if let Some(run_id) = prepared.run_id {
                            match crate::common::open_db() {
                                Ok(store) => {
                                    match store.finalize_latest_open_run_batch(run_id) {
                                        Ok(true) => eprintln!(
                                            "[agent-link] sai-write 検出: run_batches を finalized に更新 (parent={})",
                                            run_id
                                        ),
                                        Ok(false) => {}
                                        Err(e) => eprintln!(
                                            "[agent-link] run_batches finalized 更新失敗 (parent={}): {}",
                                            run_id, e
                                        ),
                                    }
                                }
                                Err(e) => eprintln!(
                                    "[agent-link] DB オープン失敗のため run_batches を finalized にできません: {}",
                                    e
                                ),
                            }
                        }
                    }
                }
                Ok(CompletionEvent::FinalizeRequested { run_id }) => {
                    eprintln!(
                        "[agent-link] ファイナライズ要求検出 (run_id={}): メインは /agent-link-finalize、サブは `agent-link task report` による完了要求です",
                        run_id
                    );
                    // P6-2: メイン（/agent-link-finalize）もサブ（task_report）もこの分岐で完了する。
                    // チャネルに残っている SaiWriteDetected イベントをすべて消費する
                    loop {
                        match rx.try_recv() {
                            Ok(CompletionEvent::SaiWriteDetected { waggle_token, .. }) => {
                                if let Some(ref token) = waggle_token {
                                    waggle_tokens.push(token.clone());
                                    eprintln!("[agent-link] ファイナライズ時: sai-write トークンを収集: token={}", token);
                                }
                            }
                            Ok(CompletionEvent::FinalizeRequested { .. }) => {
                                // 2回目以降のファイナライズ要求は無視
                            }
                            Ok(CompletionEvent::DaemonShutdown { .. }) => {
                                // 停止中に届いた切断は無視（抜けて完了処理へ進む）
                            }
                            Err(mpsc::TryRecvError::Empty) => break,
                            Err(mpsc::TryRecvError::Disconnected) => break,
                        }
                    }
                    break; // 出力抽出処理へ（ケース5: ユーザー要求による正常完了）
                }
                Ok(CompletionEvent::DaemonShutdown { run_id }) => {
                    // P0-2: デーモン停止による切断。残留 SaiWriteDetected を回収し、
                    // 利用可能な出力があれば完了処理へ進み、無ければ失敗確定へ進む。
                    eprintln!(
                        "[agent-link] デーモン停止を検出 (run_id={}): 残留出力を回収して確定します",
                        run_id
                    );
                    loop {
                        match rx.try_recv() {
                            Ok(CompletionEvent::SaiWriteDetected { waggle_token, .. }) => {
                                if let Some(ref token) = waggle_token {
                                    waggle_tokens.push(token.clone());
                                    eprintln!("[agent-link] 停止時: sai-write トークンを収集: token={}", token);
                                }
                            }
                            Ok(CompletionEvent::FinalizeRequested { .. }) => {}
                            Ok(CompletionEvent::DaemonShutdown { .. }) => {}
                            Err(mpsc::TryRecvError::Empty) => break,
                            Err(mpsc::TryRecvError::Disconnected) => break,
                        }
                    }
                    if waggle_tokens.is_empty() {
                        *progress.lock().unwrap() = PipelineProgress::Failed {
                            error: "Interrupted because the daemon stopped (no output).".to_string(),
                        };
                        return;
                    }
                    break; // 抽出処理へ（出力あり＝完了扱い）
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    // pane 消滅チェック（§4.2 ケース3・4）
                    // 閉じた pane への herdr 操作は exit コードやエラー種別が不定
                    // （ApiError / HerdrCliError / HerdrTimeout 等）のため、
                    // 連続エラーで pane 消滅と判定する（一時エラーはグレースで回避）。
                    // ユーザーが ctrl+d で pi を終了 → pane を削除した場合（ケース4）、
                    // その時点で利用可能な出力を抽出して正常完了とする。
                    match herdr.get_pane_status(pane_id) {
                        Ok(info) => {
                            pane_error_count = 0;
                            // 認証切れ（cc が 401 で止まった）の検出。画面は 30 秒に 1 回だけ読む。
                            if Instant::now() >= next_screen_check {
                                next_screen_check = Instant::now() + Duration::from_secs(30);
                                let screen = herdr.read_pane_output(pane_id, 40).unwrap_or_default();
                                let line = auth_error_line(&screen);
                                // agent が 401 の文言をファイルの内容として表示しているだけなら、
                                // 画面は動く。同じ画面が 2 回続いたときだけ止まっているとみなす。
                                let stuck = line.is_some()
                                    && last_auth_screen.as_deref() == Some(screen.as_str());
                                last_auth_screen = line.as_ref().map(|_| screen);
                                if let (true, Some(line)) = (stuck, line) {
                                    eprintln!(
                                        "[agent-link] 認証切れを検出 (pane={}): {}",
                                        pane_id, line
                                    );
                                    *progress.lock().unwrap() = PipelineProgress::Failed {
                                        error: format!(
                                            "Claude authentication failed (401). Check the Claude login or token: {}",
                                            line
                                        ),
                                    };
                                    return;
                                }
                            }
                            // U3: サブブロック停止検知（idle 60秒 or blocked 連続2回）。
                            //     正常完了（sai-write 検出）は completion_rx を先にチェックして
                            //     優先するため、この分岐は受信タイムアウト時のみ到達する。
                            if let Some(reason) = stall.observe(
                                &info.agent_status,
                                Instant::now(),
                                Duration::from_millis(idle_timeout_ms),
                                Duration::from_millis(long_run_ms),
                            ) {
                                if let Some(tid) = stalled_task_id {
                                    let (why, msg) = match reason {
                                        StallReason::Idle => (
                                            "idle",
                                            format!(
                                                "サブブロックが {} 秒以上 idle のまま停止しています（pane={}）。agent-link task capture <ID> で画面を確かめ、agent-link task send <ID> --message <文> で指示を送って再開させてください（blocked / error なら先に agent-link task interrupt <ID>）。",
                                                idle_timeout_ms / 1000,
                                                pane_id
                                            ),
                                        ),
                                        StallReason::Blocked => (
                                            "blocked",
                                            format!(
                                                "サブブロックが blocked 状態で停止しています（pane={}）。agent-link task capture <ID> で画面を確かめ、agent-link task send <ID> --message <文> で指示を送って再開させてください（blocked / error なら先に agent-link task interrupt <ID>）。",
                                                pane_id
                                            ),
                                        ),
                                        StallReason::Error => (
                                            "error",
                                            format!(
                                                "サブブロックが error 状態で停止しています（pane={}）。agent-link task capture <ID> で画面を確かめ、agent-link task send <ID> --message <文> で指示を送って再開させてください（blocked / error なら先に agent-link task interrupt <ID>）。",
                                                pane_id
                                            ),
                                        ),
                                        StallReason::LongRunning(elapsed) => (
                                            "long_running",
                                            long_running_message(elapsed, pane_id),
                                        ),
                                    };
                                    eprintln!(
                                        "[agent-link] サブブロック停止検知 (task_id={}, reason={}): {}",
                                        tid, why, msg
                                    );
                                    let _ = crate::task_manager::write_stalled_notification(
                                        tid,
                                        stalled_parent_run_id,
                                        why,
                                        &msg,
                                        None,
                                    );
                                }
                            }
                        }
                        Err(_) => {
                            pane_error_count += 1;
                            if pane_error_count >= 3 {
                                // finalize の要求より前に pane が消えたら失敗にする（0 ターンの run を「完了」にしない）。
                                // run_id が無いときは判定できないので、従来どおり完了の流れに入る。
                                let finalized = prepared
                                    .run_id
                                    .map_or(true, crate::daemon::finalize_request_seen);
                                if let Some(error) = pane_gone_error(finalized) {
                                    eprintln!(
                                        "[agent-link] ペインが finalize の前に削除されました (pane={}): 失敗にします",
                                        pane_id
                                    );
                                    *progress.lock().unwrap() = PipelineProgress::Failed {
                                        error: error.to_string(),
                                    };
                                    return;
                                }
                                eprintln!(
                                    "[agent-link] ペインが削除されました (pane={}): 利用可能な出力を抽出して完了します",
                                    pane_id
                                );
                                break; // 抽出処理へ（ケース4: 正常完了扱い）
                            }
                        }
                    }
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    *progress.lock().unwrap() = PipelineProgress::Failed {
                        error: "The connection to the daemon was lost (cannot tell whether the run completed)."
                            .to_string(),
                    };

                    return;
                }
            }
        }
    } else {
        // ── スタンドアロンモード: 通知ディレクトリを自己ポーリング（後方互換・テスト用） ──
        loop {
            // キャンセルチェック
            if cancel_flag.load(Ordering::SeqCst) {
                *progress.lock().unwrap() = PipelineProgress::Cancelled;

                return;
            }

            // タイムアウトチェック
            if Instant::now() >= deadline {
                // タイムアウト前に残りの通知を処理
                match doc_write_notify::poll_and_process_all(&integrator) {
                    Ok(results) => {
                        for r in &results {
                            if let Some(ref token) = r.waggle_token {
                                waggle_tokens.push(token.clone());
                            }
                        }
                    }
                    Err(_) => {}
                }
                *progress.lock().unwrap() = PipelineProgress::Failed {
                    error: format!("The agent did not finish within {}ms.", timeout_ms),
                };

                return;
            }

            // 進捗を更新
            let running_secs = run_start.elapsed().as_secs();
            *progress.lock().unwrap() = PipelineProgress::Running {
                elapsed_secs: running_secs,
            };

            // --- 1. エージェントの状態をチェック ---
            let pi_done = match herdr.get_pane_status(pane_id) {
                Ok(info) => {
                    if info.agent_status == "blocked" {
                        // blocked 検出 → 残りの通知を処理してからキャンセル
                        match doc_write_notify::poll_and_process_all(&integrator) {
                            Ok(results) => {
                                for r in &results {
                                    if let Some(ref token) = r.waggle_token {
                                        waggle_tokens.push(token.clone());
                                    }
                                }
                            }
                            Err(_) => {}
                        }
                        *progress.lock().unwrap() = PipelineProgress::Cancelled;

                        return;
                    }
                    info.agent_status == "idle" || info.agent_status == "done"
                }
                Err(_) => false, // エラー時は続行
            };

            // --- 2. sai-write 通知を1件処理 ---
            match doc_write_notify::poll_and_process_one(&integrator, None) {
                Ok(Some(result)) => {
                    if let Some(ref token) = result.waggle_token {
                        waggle_tokens.push(token.clone());
                        eprintln!("[agent-link] sai-write 通知を処理: token={}", token);
                    }
                    if let Some(ref err) = result.error {
                        eprintln!("[agent-link] sai-write 通知処理エラー: {}", err);
                    }
                }
                Err(e) => {
                    eprintln!("[agent-link] 通知ポーリングエラー: {}", e);
                }
                _ => {}
            }

            // --- エージェントが完了したら残りの通知を一括処理して終了 ---
            if pi_done {
                match doc_write_notify::poll_and_process_all(&integrator) {
                    Ok(results) => {
                        for r in &results {
                            if let Some(ref token) = r.waggle_token {
                                waggle_tokens.push(token.clone());
                            }
                        }
                    }
                    Err(_) => {}
                }
                break;
            }

            std::thread::sleep(Duration::from_millis(250));
        }
    }

    // ── 完了: 出力抽出 + 統計抽出 + トークン情報付きで完了通知 ──
    if cancel_flag.load(Ordering::SeqCst) {
        *progress.lock().unwrap() = PipelineProgress::Cancelled;

        return;
    }
    *progress.lock().unwrap() = PipelineProgress::ExtractingOutput;
    // 出力・トランスクリプト・統計を抽出する
    let (_output, full_transcript, session_stats_json) = extract_outputs_for_pipeline(&prepared);

    // 一時ファイルのみクリーンアップ（セッション・ペインは維持）

    let elapsed_ms = start.elapsed().as_millis() as u64;
    let git_commit_after = get_git_commit_hash();

    // waggle トークン情報を出力に付加
    let output_with_tokens = if waggle_tokens.is_empty() {
        full_transcript
    } else {
        format!(
            "{}\n\n<!-- sai-write-tokens: {} -->",
            full_transcript,
            serde_json::to_string(&waggle_tokens).unwrap_or_default()
        )
    };

    *progress.lock().unwrap() = PipelineProgress::Completed {
        output: output_with_tokens,
        elapsed_ms,
        waggle_tokens,
        session_stats_json,
        git_commit_before: prepared.git_commit_before.clone(),
        git_commit_after,
        pane_id: pane_id.to_string(),
    };
}

// ============================================================================
// Public API 7: send_turn_to_pane — 同一 pane へのターン送信
// ============================================================================

/// ターン送信の結果。
#[derive(Debug, Clone, Default)]
pub struct TurnResult {
    /// ターン後のフルトランスクリプト（全履歴 + sai-write トークン付加）
    pub output: String,
    /// 収集した waggle トークン一覧
    pub waggle_tokens: Vec<String>,
    /// ターン終了時の HEAD コミット（短縮ハッシュ）
    pub git_commit_after: Option<String>,
    /// 所要時間（ミリ秒）
    pub elapsed_ms: u64,
}

/// 既存の pane（同一エージェント）にテキストを送信し、そのターンの完了を待つ。
///
/// `task_manage(send)` の新実装（従来の「新規ブロック起動」から変更）。
/// 新しい worktree / pane は作成せず、生きている pane のエージェントに
/// 追加指示を送って継続作業させる（同一エージェントのコンテキストを維持）。
/// これにより、サブエージェント自身の実装意図を保ったまま、
/// マージコンフリクトの解消などの追加工事を指示できる。
///
/// 完了判定 = pi のエージェントステータスが idle/done に戻ること。
/// sai-write 通知があれば waggle トークンとして収集する（任意）。
///
/// # 引数
///
/// * `herdr` — herdr クライアント
/// * `pane_id` — 送信先の既存 pane
/// * `message` — 送信する追加指示テキスト
/// * `timeout_ms` — 完了待機のタイムアウト（ミリ秒）
/// * `session_dir` — セッションファイルのディレクトリ（出力抽出用）
/// * `submit_key` — 入力確定キー（`cc_command::CC_SUBMIT_KEY`。`Alt+Enter`）
/// * `task_id` — 長時間アラートを `results/` に書き込むためのタスク ID（`None` なら書かない）
/// * `parent_run_id` — 通知に添える親実行 ID
/// * `run_id` — このターンの実行の ID。`Some` のときは、この run の `file_written` だけを集める
///   （ほかの run の文書を取り込まないため）。`None` は絞らない
pub fn send_turn_to_pane(
    herdr: &HerdrClient,
    pane_id: &str,
    message: &str,
    timeout_ms: u64,
    session_dir: &Path,
    submit_key: &str,
    task_id: Option<i64>,
    parent_run_id: Option<i64>,
    run_id: Option<i64>,
) -> Result<TurnResult, OrchestratorError> {
    let start = Instant::now();

    // 1. pane の生存確認（cwd は worktree の場合があるためコミット取得に利用）
    let pane_cwd = match herdr.get_pane_status(pane_id) {
        Ok(info) => info.cwd.clone(),
        Err(e) => {
            return Err(OrchestratorError::CommandError(format!(
                "The pane for sending the turn is not available: {} ({})",
                pane_id, e
            )))
        }
    };

    // 2. プロンプト送信（送信確認 + 再送つき、F6。キャンセルフラグはこの経路では未使用）
    submit_prompt_verified(herdr, pane_id, message, submit_key, None)?;

    // 5. 完了待機（idle/done で完了、blocked でキャンセル、タイムアウトで失敗）
    let waggle_db_path = crate::project::db_path();
    if let Some(parent) = waggle_db_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let integrator = WaggleIntegrator::open(&waggle_db_path).map_err(|e| {
        OrchestratorError::CommandError(format!("Failed to initialize the waggle DB: {}", e))
    })?;
    let _ = doc_write_notify::ensure_pending_dir();
    let _ = doc_write_notify::ensure_processed_dir();

    // 子ブロックは timeout を超えても失敗にせず、stalled(timeout) を繰り返し送る
    let mut deadline = Instant::now() + Duration::from_millis(timeout_ms);
    let mut waggle_tokens: Vec<String> = Vec::new();
    let mut pane_error_count: u32 = 0;
    // U2: ターン送信中も長時間アラートを親へ知らせる（間隔は `AGENT_LINK_LONG_RUN_ALERT_MS`）
    let long_run_ms = long_run_alert_ms();
    let mut stall = StallDetector::default();

    loop {
        // タイムアウトチェック
        if Instant::now() >= deadline {
            // 子ブロックは failed にせず、stalled(timeout) を送って deadline を延ばす
            if let Some(tid) = task_id {
                let msg = timeout_alert_message(timeout_ms, start.elapsed(), pane_id);
                eprintln!(
                    "[agent-link] サブブロック timeout 超過 (task_id={}, reason=timeout): {}",
                    tid, msg
                );
                let _ = crate::task_manager::write_stalled_notification(
                    tid,
                    parent_run_id,
                    "timeout",
                    &msg,
                    None,
                );
                deadline = Instant::now() + Duration::from_millis(timeout_ms);
            } else {
                return Err(OrchestratorError::Timeout { timeout_ms });
            }
        }

        // pane 状態の確認
        match herdr.get_pane_status(pane_id) {
            Ok(info) => {
                pane_error_count = 0;
                if info.agent_status == "blocked" {
                    return Err(OrchestratorError::Cancelled);
                }
                if info.agent_status == "idle" || info.agent_status == "done" {
                    break;
                }
                // blocked / idle / done 以外（working など）は長時間アラートの監視対象。
                // idle 判定は使わないため `idle_timeout` には `Duration::MAX` を渡す。
                if let Some(StallReason::LongRunning(elapsed)) = stall.observe(
                    &info.agent_status,
                    Instant::now(),
                    Duration::MAX,
                    Duration::from_millis(long_run_ms),
                ) {
                    let msg = long_running_message(elapsed, pane_id);
                    if let Some(tid) = task_id {
                        eprintln!(
                            "[agent-link] サブブロック長時間アラート (task_id={}, reason=long_running): {}",
                            tid, msg
                        );
                        let _ = crate::task_manager::write_stalled_notification(
                            tid,
                            parent_run_id,
                            "long_running",
                            &msg,
                            None,
                        );
                    }
                }
            }
            Err(_) => {
                pane_error_count += 1;
                if pane_error_count >= 3 {
                    return Err(OrchestratorError::CommandError(
                        "The pane disappeared during the turn.".to_string(),
                    ));
                }
            }
        }

        // sai-write 通知の収集（ターン中にファイル保存があればトークン化）
        if let Ok(Some(result)) = doc_write_notify::poll_and_process_one(&integrator, run_id) {
            if let Some(ref token) = result.waggle_token {
                waggle_tokens.push(token.clone());
            }
        }

        std::thread::sleep(Duration::from_millis(250));
    }

    // 6. 出力抽出（フルトランスクリプト + トークン付加）
    let full_transcript = crate::session::extract_full_transcript(session_dir).unwrap_or_default();
    let output = if waggle_tokens.is_empty() {
        full_transcript
    } else {
        format!(
            "{}\n\n<!-- sai-write-tokens: {} -->",
            full_transcript,
            serde_json::to_string(&waggle_tokens).unwrap_or_default()
        )
    };

    // 7. コミット取得（pane の cwd = worktree の場合はそちらを優先）
    let git_commit_after = match pane_cwd {
        Some(dir) if Path::new(&dir).exists() => git_commit_hash_in(Path::new(&dir)),
        _ => get_git_commit_hash(),
    };

    Ok(TurnResult {
        output,
        waggle_tokens,
        git_commit_after,
        elapsed_ms: start.elapsed().as_millis() as u64,
    })
}

/// 指定ディレクトリ（worktree 等）の HEAD コミットハッシュ（short）を取得する。
fn git_commit_hash_in(dir: &Path) -> Option<String> {
    std::process::Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .current_dir(dir)
        .output()
        .ok()
        .and_then(|output| {
            if output.status.success() {
                let hash = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if hash.is_empty() {
                    None
                } else {
                    Some(hash)
                }
            } else {
                None
            }
        })
}

// ============================================================================
// 内部ヘルパー: 中断可能スリープ
// ============================================================================

/// キャンセル可能なスリープ（秒単位）。
///
/// 1秒ごとにキャンセルフラグをチェックし、中断要求があれば即座に戻ります。
/// 待機中は `progress` に `Waiting` 状態を書き込みます。
fn interruptible_sleep(
    total_secs: u64,
    step_name: &str,
    progress: &Arc<Mutex<PipelineProgress>>,
    cancel_flag: &Arc<AtomicBool>,
) {
    let step = step_name.to_string();
    for i in 0..total_secs {
        if cancel_flag.load(Ordering::SeqCst) {
            return;
        }
        *progress.lock().unwrap() = PipelineProgress::Waiting {
            step: step.clone(),
            elapsed_secs: i,
        };
        std::thread::sleep(Duration::from_secs(1));
    }
}

// ============================================================================
// 内部ヘルパー: プロンプト送信（送信確認 + 再送、F6）
// ============================================================================

/// プロンプトを送信し、実際に送信されたことを確認する（F6）。
///
/// 起動直後のエージェント TUI は確定キーを取りこぼすことがある
/// （テキストは入力欄に残り、エージェントは idle のまま）。
/// そのため送信後に `agent_status` を監視し、`working` にならなければ
/// 確定キーだけを再送する。
///
/// # 引数
///
/// * `herdr` — herdr クライアント
/// * `pane_id` — 送信先 pane
/// * `text` — 送信するプロンプト本文
/// * `submit_key` — 確定キー（`cc_command::CC_SUBMIT_KEY`。`Alt+Enter`。
///   `~/.claude/keybindings.json` の設定に従う）
/// * `cancel_flag` — キャンセルフラグ（無い場合は `None`）
///
/// # 戻り値
///
/// * `Ok(())` — `agent_status` が `working` になった
/// * `Err(OrchestratorError::Cancelled)` — `blocked` になった、またはキャンセルされた
/// * `Err(OrchestratorError::CommandError)` — 送信自体に失敗
/// * `Err(OrchestratorError::PromptUnconfirmed)` — 文と確定キーは送ったが、3 回試しても
///   working にならなかった（届いたかどうか分からない）
///
/// # 可視性
///
/// `agent_api::task_manage`（作業中タスクへの直接 `send`）からも使うため `pub(crate)`。
/// シグネチャは変わらない（共有契約）。
pub(crate) fn submit_prompt_verified(
    herdr: &HerdrClient,
    pane_id: &str,
    text: &str,
    submit_key: &str,
    cancel_flag: Option<&Arc<AtomicBool>>,
) -> Result<(), OrchestratorError> {
    /// 確定キーの再送を含む試行回数
    const MAX_ATTEMPTS: u32 = 3;
    /// 1 回の試行で working を待つ時間
    const ATTEMPT_WAIT_MS: u64 = 5_000;
    /// 状態ポーリング間隔
    const POLL_MS: u64 = 250;

    let cancelled = |flag: Option<&Arc<AtomicBool>>| {
        flag.map(|f| f.load(Ordering::SeqCst)).unwrap_or(false)
    };

    // 1. テキストを入力欄へ送る（1 回だけ。再送は確定キーのみ）
    herdr
        .send_text(pane_id, text)
        .map_err(|e| OrchestratorError::CommandError(format!("Failed to send text: {}", e)))?;

    for attempt in 1..=MAX_ATTEMPTS {
        if cancelled(cancel_flag) {
            return Err(OrchestratorError::Cancelled);
        }

        // 2. 入力欄が整うのを待ってから確定キーを送る
        std::thread::sleep(Duration::from_millis(300));
        herdr.send_keys(pane_id, submit_key).map_err(|e| {
            OrchestratorError::CommandError(format!("Failed to send the confirm key: {}", e))
        })?;

        // 3. working になるか確認する
        let deadline = Instant::now() + Duration::from_millis(ATTEMPT_WAIT_MS);
        while Instant::now() < deadline {
            if cancelled(cancel_flag) {
                return Err(OrchestratorError::Cancelled);
            }
            match herdr.get_pane_status(pane_id) {
                Ok(info) if info.agent_status == "working" => return Ok(()),
                Ok(info) if info.agent_status == "blocked" => {
                    return Err(OrchestratorError::Cancelled)
                }
                _ => {}
            }
            std::thread::sleep(Duration::from_millis(POLL_MS));
        }

        eprintln!(
            "[agent-link] プロンプトが送信されていない可能性があります（{} 回目）。確定キー {} を再送します",
            attempt, submit_key
        );
    }

    Err(OrchestratorError::PromptUnconfirmed {
        submit_key: submit_key.to_string(),
        attempts: MAX_ATTEMPTS,
        waited_secs: (MAX_ATTEMPTS as u64) * ATTEMPT_WAIT_MS / 1000,
    })
}

// ============================================================================
// 内部ヘルパー: キャンセル可能ポーリング
// ============================================================================

/// キャンセル可能なステータスポーリング。
///
/// `wait_for_status` のキャンセル対応版。
/// 250ms 間隔で herdr のペイン状態をポーリングし、
/// 各イテレーションでキャンセルフラグをチェックします。
fn wait_for_status_cancelable(
    herdr: &HerdrClient,
    pane_id: &str,
    target_statuses: &[&str],
    timeout_ms: u64,
    step_name: &str,
    progress: &Arc<Mutex<PipelineProgress>>,
    cancel_flag: &Arc<AtomicBool>,
) -> Result<(), OrchestratorError> {
    let deadline = Instant::now() + Duration::from_millis(timeout_ms);
    let step = step_name.to_string();
    let _ = &step; // 将来の拡張用に変数を保持

    loop {
        // キャンセルチェック
        if cancel_flag.load(Ordering::SeqCst) {
            return Err(OrchestratorError::Cancelled);
        }

        if Instant::now() >= deadline {
            return Err(OrchestratorError::Timeout { timeout_ms });
        }

        // 進捗を更新
        let elapsed = (timeout_ms as u64).saturating_sub(
            deadline
                .saturating_duration_since(Instant::now())
                .as_millis() as u64,
        );
        *progress.lock().unwrap() = PipelineProgress::Waiting {
            step: step.clone(),
            elapsed_secs: elapsed / 1000,
        };

        match herdr.get_pane_status(pane_id) {
            Ok(info) => {
                if target_statuses.contains(&info.agent_status.as_str()) {
                    return Ok(());
                }
            }
            Err(HerdrError::HerdrNotFound) => {
                return Err(HerdrError::HerdrNotFound.into());
            }
            Err(_) => {
                // その他のエラーはリトライ
            }
        }

        // 250ms 待機（中断可能）
        if cancel_flag.load(Ordering::SeqCst) {
            return Err(OrchestratorError::Cancelled);
        }
        std::thread::sleep(Duration::from_millis(250));
    }
}

// ============================================================================
// 旧ヘルパー関数（テスト互換のため残す）
// ============================================================================

/// 指定されたステータスになるまでポーリングします（キャンセル非対応、テスト用）。
#[cfg(test)]
fn wait_for_status(
    herdr: &HerdrClient,
    pane_id: &str,
    target_statuses: &[&str],
    timeout_ms: u64,
) -> Result<(), OrchestratorError> {
    let deadline = Instant::now() + Duration::from_millis(timeout_ms);

    loop {
        if Instant::now() >= deadline {
            return Err(OrchestratorError::Timeout { timeout_ms });
        }

        match herdr.get_pane_status(pane_id) {
            Ok(info) => {
                if target_statuses.contains(&info.agent_status.as_str()) {
                    return Ok(());
                }
            }
            Err(HerdrError::HerdrNotFound) => {
                return Err(HerdrError::HerdrNotFound.into());
            }
            Err(_) => {}
        }

        std::thread::sleep(Duration::from_millis(250));
    }
}

/// 指定されたステータスになるまでポーリングします（blocked検出付き、キャンセル非対応、テスト用）。
#[cfg(test)]
fn wait_for_status_with_cancellation(
    herdr: &HerdrClient,
    pane_id: &str,
    target_statuses: &[&str],
    timeout_ms: u64,
) -> Result<(), OrchestratorError> {
    let deadline = Instant::now() + Duration::from_millis(timeout_ms);

    loop {
        if Instant::now() >= deadline {
            return Err(OrchestratorError::Timeout { timeout_ms });
        }

        match herdr.get_pane_status(pane_id) {
            Ok(info) => {
                if info.agent_status == "blocked" {
                    return Err(OrchestratorError::Cancelled);
                }
                if target_statuses.contains(&info.agent_status.as_str()) {
                    return Ok(());
                }
            }
            Err(HerdrError::HerdrNotFound) => {
                return Err(HerdrError::HerdrNotFound.into());
            }
            Err(_) => {}
        }

        std::thread::sleep(Duration::from_millis(250));
    }
}

// ============================================================================
// テスト
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block_config::BlockConfig;
    use std::collections::HashMap;

    /// 子の model / thinking は task_create の値 > カード > 無し の順。空のカード値は無しと同じ。
    #[test]
    fn subtask_setting_prefers_task_then_card_then_none() {
        assert_eq!(
            subtask_setting(Some("haiku".into()), Some("opus")),
            Some("haiku".into())
        );
        assert_eq!(subtask_setting(None, Some(" opus ")), Some("opus".into()));
        assert_eq!(subtask_setting(None, Some("   ")), None);
        assert_eq!(subtask_setting(None, None), None);
    }

    // ---- StallDetector（U3: 停止検知） ----

    /// long_run に十分大きい値（working では LongRunning が出ないようにする）
    const NO_LONG: Duration = Duration::from_secs(3600);

    #[test]
    fn test_stall_detector_idle_detects_after_timeout() {
        let mut det = StallDetector::default();
        let t0 = Instant::now();
        let timeout = Duration::from_millis(100);
        // 最初の idle ではタイマー開始のみ（通知なし）
        assert_eq!(
            det.observe("idle", t0, timeout, NO_LONG),
            None,
            "1回目の idle ではまだ通知しない"
        );
        // 閾値前の idle は継続中扱い
        assert_eq!(
            det.observe("idle", t0 + Duration::from_millis(50), timeout, NO_LONG),
            None
        );
        // 閾値超えで Idle 通知
        assert_eq!(
            det.observe("idle", t0 + Duration::from_millis(150), timeout, NO_LONG),
            Some(StallReason::Idle)
        );
    }

    #[test]
    fn test_stall_detector_idle_reset_on_working() {
        let mut det = StallDetector::default();
        let t0 = Instant::now();
        let timeout = Duration::from_millis(100);
        det.observe("idle", t0, timeout, NO_LONG);
        // working が挟まるとタイマーがリセットされる
        det.observe("working", t0 + Duration::from_millis(10), timeout, NO_LONG);
        det.observe("idle", t0 + Duration::from_millis(20), timeout, NO_LONG);
        // リセット後、閾値分の idle が続けて初めて検知する
        assert_eq!(
            det.observe("idle", t0 + Duration::from_millis(200), timeout, NO_LONG),
            Some(StallReason::Idle)
        );
    }

    #[test]
    fn test_stall_detector_blocked_twice() {
        let mut det = StallDetector::default();
        let t0 = Instant::now();
        let timeout = Duration::from_millis(100);
        // 1回目の blocked ではまだ通知しない
        assert_eq!(det.observe("blocked", t0, timeout, NO_LONG), None);
        // 連続2回目で即時通知
        assert_eq!(
            det.observe("blocked", t0 + Duration::from_millis(1), timeout, NO_LONG),
            Some(StallReason::Blocked)
        );
    }

    #[test]
    fn test_stall_detector_error_twice() {
        let mut det = StallDetector::default();
        let t0 = Instant::now();
        let timeout = Duration::from_millis(100);
        // error も blocked 同様、連続2回で即時通知する
        assert_eq!(det.observe("error", t0, timeout, NO_LONG), None);
        assert_eq!(
            det.observe("error", t0 + Duration::from_millis(1), timeout, NO_LONG),
            Some(StallReason::Error)
        );
    }

    #[test]
    fn test_stall_detector_no_resend_after_notified() {
        let mut det = StallDetector::default();
        let t0 = Instant::now();
        let timeout = Duration::from_millis(100);
        det.observe("idle", t0, timeout, NO_LONG);
        assert_eq!(
            det.observe("idle", t0 + Duration::from_millis(150), timeout, NO_LONG),
            Some(StallReason::Idle)
        );
        // 通知済みなら再送しない（working を挟まなければ介入待ち・スパム防止）
        assert_eq!(
            det.observe("idle", t0 + Duration::from_millis(300), timeout, NO_LONG),
            None
        );
        assert_eq!(
            det.observe("blocked", t0 + Duration::from_millis(400), timeout, NO_LONG),
            None
        );
    }

    #[test]
    fn test_stall_detector_done_resets_blocked() {
        let mut det = StallDetector::default();
        let t0 = Instant::now();
        let timeout = Duration::from_millis(100);
        det.observe("blocked", t0, timeout, NO_LONG);
        // done が挟まると blocked カウントがリセットされる
        det.observe("done", t0 + Duration::from_millis(1), timeout, NO_LONG);
        det.observe("blocked", t0 + Duration::from_millis(2), timeout, NO_LONG);
        assert_eq!(
            det.observe("blocked", t0 + Duration::from_millis(3), timeout, NO_LONG),
            Some(StallReason::Blocked)
        );
    }

    /// AC1: working を観測し続けると long_run ごとに LongRunning が繰り返す。
    #[test]
    fn test_stall_detector_long_running_repeats() {
        let mut det = StallDetector::default();
        let t0 = Instant::now();
        let long_run = Duration::from_millis(100);
        // 閾値前は None
        assert_eq!(
            det.observe("working", t0, Duration::from_millis(100), long_run),
            None
        );
        assert_eq!(
            det.observe(
                "working",
                t0 + Duration::from_millis(50),
                Duration::from_millis(100),
                long_run
            ),
            None
        );
        // 100ms で 1 回目（working 継続 100ms）
        assert_eq!(
            det.observe(
                "working",
                t0 + Duration::from_millis(100),
                Duration::from_millis(100),
                long_run
            ),
            Some(StallReason::LongRunning(Duration::from_millis(100)))
        );
        // 100ms〜200ms は None
        assert_eq!(
            det.observe(
                "working",
                t0 + Duration::from_millis(150),
                Duration::from_millis(100),
                long_run
            ),
            None
        );
        // 200ms で 2 回目（working 継続 200ms）
        assert_eq!(
            det.observe(
                "working",
                t0 + Duration::from_millis(200),
                Duration::from_millis(100),
                long_run
            ),
            Some(StallReason::LongRunning(Duration::from_millis(200)))
        );
    }

    /// AC2: idle を挟むと working の経過時間がリセットされる。
    #[test]
    fn test_stall_detector_long_running_reset_on_idle() {
        let mut det = StallDetector::default();
        let t0 = Instant::now();
        let long_run = Duration::from_millis(100);
        // working 90ms（閾値未満）
        assert_eq!(
            det.observe("working", t0, Duration::from_millis(100), long_run),
            None
        );
        assert_eq!(
            det.observe(
                "working",
                t0 + Duration::from_millis(90),
                Duration::from_millis(100),
                long_run
            ),
            None
        );
        // idle でリセット
        det.observe(
            "idle",
            t0 + Duration::from_millis(95),
            Duration::from_millis(100),
            long_run,
        );
        // working に戻して 90ms なら出ない
        assert_eq!(
            det.observe(
                "working",
                t0 + Duration::from_millis(100),
                Duration::from_millis(100),
                long_run
            ),
            None
        );
        assert_eq!(
            det.observe(
                "working",
                t0 + Duration::from_millis(190),
                Duration::from_millis(100),
                long_run
            ),
            None
        );
    }

    /// AC3: idle で通知したあと working → idle で再び Idle が出る（介入後の再検知）。
    #[test]
    fn test_stall_detector_renotify_after_working() {
        let mut det = StallDetector::default();
        let t0 = Instant::now();
        let timeout = Duration::from_millis(100);
        det.observe("idle", t0, timeout, NO_LONG);
        assert_eq!(
            det.observe("idle", t0 + Duration::from_millis(150), timeout, NO_LONG),
            Some(StallReason::Idle)
        );
        // working を挟むと notified が戻る
        det.observe("working", t0 + Duration::from_millis(200), timeout, NO_LONG);
        // 改めて idle タイマーが始まり、閾値超えで再び通知
        det.observe("idle", t0 + Duration::from_millis(210), timeout, NO_LONG);
        assert_eq!(
            det.observe("idle", t0 + Duration::from_millis(400), timeout, NO_LONG),
            Some(StallReason::Idle)
        );
    }

    /// 最小構成の PipelineConfig を作成するヘルパー
    fn minimal_config() -> PipelineConfig {
        PipelineConfig {
            block_config: BlockConfig {
                name: "test-block".to_string(),
                description: Some("テスト用".to_string()),
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
            },
            block_body: String::new(),
            user_inputs: HashMap::new(),
            cwd: Some(finalize_test_temp_dir("cc-project")),
            direction: SplitDirection::Right,
            timeout_ms: 5_000,
            keep_pane: false,
            run_id: None,
            tab_id: None,
            base_pane_id: None,
            pane_id: None,
            is_subtask: false,
        }
    }

    // ---- PipelineConfig ----

    #[test]
    fn test_pipeline_config_default() {
        let config = PipelineConfig::default();
        assert_eq!(config.block_config.name, "default");
        assert_eq!(config.direction, SplitDirection::Right);
        assert_eq!(config.timeout_ms, NO_TIMEOUT_MS);
        assert!(!config.keep_pane);
    }

    #[test]
    fn test_pipeline_config_minimal() {
        let config = minimal_config();
        assert_eq!(config.block_config.name, "test-block");
        assert_eq!(config.timeout_ms, 5_000);
    }

    // ---- prepare_pipeline ----

    #[test]
    fn test_prepare_pipeline_basic() {
        // 環境変数（AGENT_LINK_PROJECT_ROOT）を書き換えるテストと直列化する（U4）
        let _env_lock = finalize_env_lock();
        let config = minimal_config();
        let prepared = prepare_pipeline(&config).expect("prepare_pipeline に失敗");

        assert_eq!(prepared.block_name, "test-block");
        assert!(
            prepared.pi_command.starts_with("PATH="),
            "PATH を先頭に付ける: {}",
            prepared.pi_command
        );
        assert!(
            prepared.pi_command.contains(" claude --session-id"),
            "claude を起動する: {}",
            prepared.pi_command
        );
        assert!(prepared.pi_command.contains("--session-id"));
        assert!(prepared
            .session_dir
            .to_string_lossy()
            .contains("pi-sessions-"));
        assert!(prepared.expanded_prompt.is_empty()); // block_body が空なので
        assert!(prepared.system_prompt.is_empty()); // 固定部分も空

        // クリーンアップ（テストではセッションディレクトリを削除）
        crate::session::cleanup_session_dir(&prepared.session_dir);
    }

    /// run_id が設定されている場合、エージェント用の環境変数
    /// （AGENT_LINK_PARENT_RUN_ID / AGENT_LINK_BLOCK_NAME）がコマンド先頭に付与される。
    #[test]
    fn test_prepare_pipeline_adds_sai_env_prefix() {
        // 環境変数の影響を受けないよう AGENT_LINK_PROJECT_ROOT を書き換えるテストと直列化する（U4）
        let _env_lock = finalize_env_lock();
        // 親プロセスから AGENT_LINK_PROJECT_ROOT が引き継がれていても、未設定時の挙動を検証する
        let _project_root_guard = EnvVarGuard::remove("AGENT_LINK_PROJECT_ROOT");
        let mut config = minimal_config();
        config.run_id = Some(1234);
        config.block_config.name = "sai-test-main".to_string();
        let prepared = prepare_pipeline(&config).expect("prepare_pipeline に失敗");

        assert!(
            prepared
                .pi_command
                .contains("AGENT_LINK_PARENT_RUN_ID=1234 AGENT_LINK_BLOCK_NAME=sai-test-main claude "),
            "環境変数プレフィックスが付与されるべき: {}",
            prepared.pi_command
        );
        assert!(
            prepared.pi_command.starts_with("PATH="),
            "PATH を先頭に付ける: {}",
            prepared.pi_command
        );
        assert!(prepared.pi_command.contains("--session-id"));

        // クリーンアップ
        crate::session::cleanup_session_dir(&prepared.session_dir);
    }

    /// `env` を設定したテスト用の環境（プロジェクトルートと `~/.agent-link` 差し替え）を作る。
    ///
    /// 返り値の一本目・二本目はスコープを抜けたときに元へ戻すガード。
    /// 三本目・四本目は後片付け用のディレクトリ。
    #[allow(clippy::type_complexity)]
    fn finalize_settings_env_dirs(
        name: &str,
        project_settings: Option<&str>,
        home_settings: Option<&str>,
    ) -> (
        EnvVarGuard,
        EnvVarGuard,
        std::path::PathBuf,
        std::path::PathBuf,
    ) {
        let root = finalize_test_temp_dir(name);
        let home = std::env::temp_dir().join(format!(
            "{}-home-{}",
            root.file_name().unwrap().to_string_lossy(),
            std::process::id()
        ));
        std::fs::create_dir_all(root.join(".agent-link")).unwrap();
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(&home).unwrap();
        if let Some(body) = project_settings {
            std::fs::write(root.join(".agent-link/settings.json"), body).unwrap();
        }
        if let Some(body) = home_settings {
            std::fs::write(home.join("settings.json"), body).unwrap();
        }
        let root_guard = EnvVarGuard::set("AGENT_LINK_PROJECT_ROOT", root.as_os_str());
        let home_guard = EnvVarGuard::set("AGENT_LINK_HOME", home.as_os_str());
        (root_guard, home_guard, root, home)
    }

    /// 利用者全体（`~/.agent-link/settings.json`）の `env` が agent の起動コマンドへ付くこと。
    /// 接続先などを 1 か所に書けば 全 agent に配られる。
    #[test]
    fn test_prepare_pipeline_adds_settings_env_prefix() {
        let _env_lock = finalize_env_lock();
        let (_root_guard, _home_guard, root, home) = finalize_settings_env_dirs(
            "settings-env-prefix",
            None,
            Some(r#"{"env": {"EXAMPLE_URL": "http://127.0.0.1:5001"}}"#),
        );
        let mut config = minimal_config();
        config.run_id = Some(1234);
        config.block_config.name = "sai-test-main".to_string();

        let prepared = prepare_pipeline(&config).expect("prepare_pipeline に失敗");

        assert!(
            prepared.pi_command.contains(
                "AGENT_LINK_BLOCK_NAME=sai-test-main EXAMPLE_URL='http://127.0.0.1:5001' claude "
            ),
            "設定の env が AGENT_LINK_* の後ろに付くべき: {}",
            prepared.pi_command
        );
        let _ = std::fs::remove_dir_all(root);
        let _ = std::fs::remove_dir_all(home);
        crate::session::cleanup_session_dir(&prepared.session_dir);
    }

    /// 設定の `env` は agent-link 自身が付ける `AGENT_LINK_*` より後ろに並ぶこと。
    /// （前だと利用者が AGENT_LINK_PROJECT_ROOT を上書きできてしまうため）
    #[test]
    fn test_prepare_pipeline_env_comes_after_sai_vars() {
        let _env_lock = finalize_env_lock();
        let (_root_guard, _home_guard, root, home) = finalize_settings_env_dirs(
            "settings-env-order",
            None,
            Some(r#"{"env": {"EXAMPLE_URL": "http://127.0.0.1:5001"}}"#),
        );
        let mut config = minimal_config();
        config.run_id = Some(1234);
        config.block_config.name = "sai-test-main".to_string();

        let prepared = prepare_pipeline(&config).expect("prepare_pipeline に失敗");
        let cmd = &prepared.pi_command;

        let pos_root = cmd
            .find("AGENT_LINK_PROJECT_ROOT=")
            .unwrap_or_else(|| panic!("AGENT_LINK_PROJECT_ROOT が付くべき: {}", cmd));
        let pos_parent = cmd
            .find("AGENT_LINK_PARENT_RUN_ID=")
            .unwrap_or_else(|| panic!("AGENT_LINK_PARENT_RUN_ID が付くべき: {}", cmd));
        let pos_block = cmd
            .find("AGENT_LINK_BLOCK_NAME=")
            .unwrap_or_else(|| panic!("AGENT_LINK_BLOCK_NAME が付くべき: {}", cmd));
        let pos_env = cmd
            .find("EXAMPLE_URL=")
            .unwrap_or_else(|| panic!("設定の env が付くべき: {}", cmd));
        let pos_pi = cmd.find(" claude ").unwrap_or_else(|| panic!("claude が付くべき: {}", cmd));

        assert!(pos_root < pos_parent && pos_parent < pos_block);
        assert!(
            pos_block < pos_env,
            "env は AGENT_LINK_* より後ろに並ぶべき: {}",
            cmd
        );
        assert!(pos_env < pos_pi, "env はエージェントより前に並ぶべき: {}", cmd);

        let _ = std::fs::remove_dir_all(root);
        let _ = std::fs::remove_dir_all(home);
        crate::session::cleanup_session_dir(&prepared.session_dir);
    }

    /// 設定の `env` の値は shell のシングルクォートで 1 つだけ包まれること。
    /// 空白やシングルクォートを含む値でもコマンドが壊れない。
    #[test]
    fn test_prepare_pipeline_env_value_is_shell_quoted() {
        let _env_lock = finalize_env_lock();
        let (_root_guard, _home_guard, root, home) = finalize_settings_env_dirs(
            "settings-env-quote",
            None,
            Some(r#"{"env": {"AGENT_LINK_TEST_QUOTE": "a b'c"}}"#),
        );
        let mut config = minimal_config();
        config.block_config.name = "sai-test-main".to_string();

        let prepared = prepare_pipeline(&config).expect("prepare_pipeline に失敗");
        let escaped = crate::herdr::shell_escape_single_quotes("a b'c");
        assert!(
            prepared.pi_command.contains(&format!(
                "AGENT_LINK_TEST_QUOTE={} claude ",
                crate::herdr::shell_escape_single_quotes("a b'c")
            )),
            "値がシングルクォートで囲まれるべき: {}",
            prepared.pi_command
        );
        assert_eq!(
            prepared
                .pi_command
                .matches(&escaped)
                .count(),
            1,
            "エスケープ済みの値が 1 回だけ現れるべき: {}",
            prepared.pi_command
        );

        let _ = std::fs::remove_dir_all(root);
        let _ = std::fs::remove_dir_all(home);
        crate::session::cleanup_session_dir(&prepared.session_dir);
    }

    /// サブブロックの起動コマンドには AGENT_LINK_IS_SUBTASK=1 と AGENT_LINK_BIN（実行ファイルのパス）が付く。
    /// 子が AGENT_LINK_BIN で agent-link の場所を知るため。
    #[test]
    fn test_prepare_pipeline_subtask_gets_sai_link_bin() {
        let _env_lock = finalize_env_lock();
        let _project_root_guard = EnvVarGuard::remove("AGENT_LINK_PROJECT_ROOT");

        let mut subtask = minimal_config();
        subtask.run_id = Some(1234);
        subtask.is_subtask = true;
        let prepared = prepare_pipeline(&subtask).expect("prepare_pipeline に失敗");
        let cmd = &prepared.pi_command;
        assert!(cmd.contains("AGENT_LINK_IS_SUBTASK=1 "), "{}", cmd);
        assert!(
            cmd.contains("AGENT_LINK_BIN="),
            "AGENT_LINK_BIN が付くこと: {}",
            cmd
        );
        assert!(
            cmd.contains(&crate::cc_command::sai_link_exe()),
            "AGENT_LINK_BIN は sai_link_exe() のパス: {}",
            cmd
        );
        // 子の pane は CLI を名前だけで呼べるよう PATH の先頭に実行ファイルのフォルダが付く
        let exe_dir = std::path::Path::new(&crate::cc_command::sai_link_exe())
            .parent()
            .expect("実行ファイルの親フォルダ")
            .to_path_buf();
        assert!(
            cmd.contains(&format!(
                "PATH='{}':$PATH ",
                exe_dir.display()
            )),
            "PATH の先頭に実行ファイルのフォルダが付く: {}",
            cmd
        );
        crate::session::cleanup_session_dir(&prepared.session_dir);

        // メイン実行には付かない
        let mut main = minimal_config();
        main.run_id = Some(1234);
        let prepared = prepare_pipeline(&main).expect("prepare_pipeline に失敗");
        assert!(
            !prepared.pi_command.contains("AGENT_LINK_BIN="),
            "{}",
            prepared.pi_command
        );
        crate::session::cleanup_session_dir(&prepared.session_dir);
    }

    /// cwd がプロジェクトルートの外のとき、agent-link 側の設定
    /// （`.mcp.json` / `.claude/settings.json`）が `--mcp-config` /
    /// `--settings` で渡り、設定チェックで止まらないこと。
    #[test]
    fn test_prepare_pipeline_cc_passes_project_config_for_external_cwd() {
        let _env_lock = finalize_env_lock();
        let root = finalize_test_temp_dir("cc-external-root");
        std::fs::write(
            root.join(".mcp.json"),
            crate::cc_command::mcp_config_json().to_string(),
        )
        .unwrap();
        let _env_guard = EnvVarGuard::set("AGENT_LINK_PROJECT_ROOT", root.as_os_str());

        // 設定を持たない別フォルダ（プロジェクト外相当）を作業フォルダにする
        let external = finalize_test_temp_dir("cc-external-cwd");
        let mut config = minimal_config();
        config.cwd = Some(external.clone());
        config.run_id = Some(4321);

        let prepared = prepare_pipeline(&config).expect("prepare_pipeline に失敗");
        let mcp = root.join(".mcp.json");
        assert!(
            prepared
                .pi_command
                .contains(&format!("--mcp-config '{}'", mcp.display())),
            "--mcp-config が agent-link 側のパスで渡るべき: {}",
            prepared.pi_command
        );
        let settings = root.join(".agent-link/cc-settings.json");
        assert!(settings.is_file(), "cc の設定はルートの .agent-link に書く");
        assert!(!external.join(".claude/settings.json").exists());
        assert!(!root.join(".claude/settings.json").exists());
        assert!(
            prepared
                .pi_command
                .contains(&format!("--settings '{}'", settings.display())),
            "--settings が agent-link 側のパスで渡るべき: {}",
            prepared.pi_command
        );

        crate::session::cleanup_session_dir(&prepared.session_dir);
    }

    #[test]
    fn test_prepare_pipeline_with_body() {
        let _env_lock = finalize_env_lock();
        let mut config = minimal_config();
        config.block_body = "# テスト\nプロジェクト: {{input.name}}".to_string();

        let prepared = prepare_pipeline(&config).expect("prepare_pipeline に失敗");

        // 固定部分（system prompt）: 値は案内の文になる
        assert!(prepared.system_prompt.contains("# テスト"));
        assert!(prepared
            .system_prompt
            .contains("（最初のメッセージの「# name」の節にある）"));
        // 値が無いので最初の文には `{{input.name}}` がそのまま入る
        // （run フォルダが無いので固定部分は最初の文の先頭に入る）
        assert!(prepared.expanded_prompt.contains("{{input.name}}"));

        crate::session::cleanup_session_dir(&prepared.session_dir);
    }

    #[test]
    fn test_prepare_pipeline_with_user_inputs() {
        let _env_lock = finalize_env_lock();
        let mut config = minimal_config();
        config.block_body = "プロジェクト: {{input.name}}".to_string();
        let mut inputs = HashMap::new();
        inputs.insert("name".to_string(), "MyApp".to_string());
        config.user_inputs = inputs;

        let prepared = prepare_pipeline(&config).expect("prepare_pipeline に失敗");

        // 固定部分には案内文だけ入り、値は最初の文の節に入る（U6）
        assert!(prepared
            .system_prompt
            .contains("（最初のメッセージの「# name」の節にある）"));
        assert!(!prepared.system_prompt.contains("MyApp"));
        assert!(prepared.expanded_prompt.contains("MyApp"));
        assert!(!prepared.expanded_prompt.contains("{{input.name}}"));

        crate::session::cleanup_session_dir(&prepared.session_dir);
    }

    /// U6: run フォルダがあるときは固定部分を `system-prompt.md` に書き、
    /// 起動コマンドは `--append-system-prompt <絶対パス>` になる。
    /// 最初の文には入力値の節だけが入る。
    #[test]
    fn test_prepare_pipeline_writes_system_prompt_file() {
        let _env_lock = finalize_env_lock();
        let root = finalize_test_temp_dir("system-prompt");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join(".agent-link")).unwrap();
        let _root_guard = EnvVarGuard::set("AGENT_LINK_PROJECT_ROOT", root.as_os_str());
        let _home_guard = EnvVarGuard::set("AGENT_LINK_HOME", root.as_os_str());

        // run フォルダを用意する（todo で予約したように）
        let run_dir = crate::run_dir::ensure_run_dir(crate::run_dir::TODO_OWNER, 321).unwrap();

        let mut config = minimal_config();
        config.run_id = Some(321);
        config.block_body = "# 依頼\n{{input.task_text}}".to_string();
        let mut inputs = HashMap::new();
        inputs.insert("task_text".to_string(), "登録を実装せよ".to_string());
        config.user_inputs = inputs;

        let prepared = prepare_pipeline(&config).expect("prepare_pipeline に失敗");

        // 固定部分は run フォルダ直下の system-prompt.md
        let sp = run_dir.join(crate::run_dir::SYSTEM_PROMPT_FILE);
        assert!(sp.is_file(), "{} が作られる", sp.display());
        let written = std::fs::read_to_string(&sp).unwrap();
        assert_eq!(written, prepared.system_prompt);
        assert!(written.contains("（最初のメッセージの「# task_text」の節にある）"));
        assert!(!written.contains("登録を実装せよ"));

        // 起動コマンドは絶対パスを渡す（run フォルダの system-prompt.md）
        assert!(
            std::path::Path::new(&sp).is_absolute(),
            "system-prompt.md は絶対パスで渡す: {}",
            sp.display()
        );
        assert!(
            prepared
                .pi_command
                .contains(&format!("--append-system-prompt-file '{}'", sp.display())),
            "固定部分のファイルが渡る: {}",
            prepared.pi_command
        );

        // 最初の文は入力値の節だけ
        assert_eq!(prepared.expanded_prompt, "# task_text\n\n登録を実装せよ");

        let _ = std::fs::remove_dir_all(&root);
    }

    // ---- PipelineProgress ----

    #[test]
    fn test_pipeline_progress_clone() {
        let p = PipelineProgress::Pending;
        assert_eq!(p.clone(), PipelineProgress::Pending);

        let w = PipelineProgress::Waiting {
            step: "テスト".to_string(),
            elapsed_secs: 3,
        };
        assert_eq!(
            w,
            PipelineProgress::Waiting {
                step: "テスト".to_string(),
                elapsed_secs: 3,
            }
        );

        let r = PipelineProgress::Running { elapsed_secs: 42 };
        assert_eq!(r, PipelineProgress::Running { elapsed_secs: 42 });

        let c = PipelineProgress::Completed {
            output: "done".to_string(),
            elapsed_ms: 1000,
            waggle_tokens: vec!["AbCdEfGh".to_string()],
            session_stats_json: None,
            git_commit_before: None,
            git_commit_after: None,
            pane_id: "pane_test_001".to_string(),
        };
        match c {
            PipelineProgress::Completed {
                output,
                elapsed_ms,
                waggle_tokens,
                ..
            } => {
                assert_eq!(output, "done");
                assert_eq!(elapsed_ms, 1000);
                assert_eq!(waggle_tokens.len(), 1);
            }
            _ => panic!("Completed のはず"),
        }
    }

    // ---- PipelineHandle ----

    #[test]
    fn test_pipeline_handle_cancel() {
        // ダミーの handle を作成してキャンセルが動作することを確認
        let progress = Arc::new(Mutex::new(PipelineProgress::Pending));
        let cancel_flag = Arc::new(AtomicBool::new(false));

        let handle = PipelineHandle {
            progress: Arc::clone(&progress),
            cancel_flag: Arc::clone(&cancel_flag),
            thread_handle: None,
        };

        assert!(!handle.is_finished());
        assert_eq!(handle.get_progress(), PipelineProgress::Pending);

        handle.cancel();
        assert!(cancel_flag.load(Ordering::SeqCst));
    }

    #[test]
    fn test_pipeline_handle_is_finished() {
        let progress = Arc::new(Mutex::new(PipelineProgress::Completed {
            output: "ok".to_string(),
            elapsed_ms: 100,
            waggle_tokens: Vec::new(),
            session_stats_json: None,
            git_commit_before: None,
            git_commit_after: None,
            pane_id: "pane_test_001".to_string(),
        }));
        let cancel_flag = Arc::new(AtomicBool::new(false));

        let handle = PipelineHandle {
            progress,
            cancel_flag,
            thread_handle: None,
        };

        assert!(handle.is_finished());

        // Failed も finished
        let progress2 = Arc::new(Mutex::new(PipelineProgress::Failed {
            error: "err".to_string(),
        }));
        let handle2 = PipelineHandle {
            progress: progress2,
            cancel_flag: Arc::new(AtomicBool::new(false)),
            thread_handle: None,
        };
        assert!(handle2.is_finished());

        // Cancelled も finished
        let progress3 = Arc::new(Mutex::new(PipelineProgress::Cancelled));
        let handle3 = PipelineHandle {
            progress: progress3,
            cancel_flag: Arc::new(AtomicBool::new(false)),
            thread_handle: None,
        };
        assert!(handle3.is_finished());

        // Running は finished ではない
        let progress4 = Arc::new(Mutex::new(PipelineProgress::Running { elapsed_secs: 5 }));
        let handle4 = PipelineHandle {
            progress: progress4,
            cancel_flag: Arc::new(AtomicBool::new(false)),
            thread_handle: None,
        };
        assert!(!handle4.is_finished());
    }

    #[test]
    fn test_pipeline_handle_join() {
        let progress = Arc::new(Mutex::new(PipelineProgress::Completed {
            output: "joined".to_string(),
            elapsed_ms: 50,
            waggle_tokens: Vec::new(),
            session_stats_json: None,
            git_commit_before: None,
            git_commit_after: None,
            pane_id: "pane_test_001".to_string(),
        }));
        let handle = PipelineHandle {
            progress,
            cancel_flag: Arc::new(AtomicBool::new(false)),
            thread_handle: None, // スレッドなしでも join 可能
        };

        let final_progress = handle.join();
        assert!(matches!(final_progress, PipelineProgress::Completed { .. }));
    }

    // ---- wait_for_status (旧関数、テスト用) ----

    #[test]
    fn test_wait_for_status_timeout() {
        let herdr = HerdrClient::with_binary("/nonexistent/herdr_binary");
        let result = wait_for_status(&herdr, "test-pane", &["idle"], 100);
        match result {
            Err(OrchestratorError::HerdrError(HerdrError::HerdrNotFound)) => {}
            Err(OrchestratorError::HerdrError(HerdrError::IoError(_))) => {}
            _ => {}
        }
    }

    #[test]
    fn test_wait_for_status_with_cancellation_timeout() {
        let herdr = HerdrClient::with_binary("/nonexistent/herdr_binary");
        let result = wait_for_status_with_cancellation(&herdr, "test-pane", &["idle"], 100);
        match result {
            Err(OrchestratorError::HerdrError(HerdrError::HerdrNotFound)) => {}
            Err(OrchestratorError::HerdrError(HerdrError::IoError(_))) => {}
            _ => {}
        }
    }

    // ---- OrchestratorError ----

    #[test]
    fn test_orchestrator_error_display() {
        let err_timeout = OrchestratorError::Timeout { timeout_ms: 5000 };
        let msg = format!("{}", err_timeout);
        assert!(msg.contains("5000ms"));
        assert!(msg.contains("within"));

        let err_cancelled = OrchestratorError::Cancelled;
        let msg = format!("{}", err_cancelled);
        assert!(msg.contains("Cancelled"));

        let err_herdr = OrchestratorError::HerdrError(HerdrError::HerdrNotFound);
        let msg = format!("{}", err_herdr);
        assert!(msg.contains("herdr"));
    }

    #[test]
    fn test_session_error_conversion() {
        let session_err = SessionError::CreateDirError("test error".to_string());
        let orch_err: OrchestratorError = session_err.into();
        let msg = format!("{}", orch_err);
        assert!(msg.contains("test error"));
        assert!(msg.contains("Session file"));
    }

    #[test]
    fn test_orchestrator_error_from_io() {
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "file not found");
        let orch_err: OrchestratorError = io_err.into();
        let msg = format!("{}", orch_err);
        assert!(msg.contains("file not found"));
    }

    // ---- interruptible_sleep のキャンセル動作 ----

    #[test]
    fn test_interruptible_sleep_cancelled() {
        let progress = Arc::new(Mutex::new(PipelineProgress::Pending));
        let cancel_flag = Arc::new(AtomicBool::new(true)); // 最初からキャンセル済み

        let start = Instant::now();
        interruptible_sleep(10, "テスト", &progress, &cancel_flag);
        let elapsed = start.elapsed();

        // 即座に戻ること（1秒も待たない）
        assert!(elapsed < Duration::from_secs(1));
        // progress は更新されない（最初のイテレーションでキャンセルされて return するため）
    }

    #[test]
    fn test_interruptible_sleep_updates_progress() {
        let progress = Arc::new(Mutex::new(PipelineProgress::Pending));
        let cancel_flag = Arc::new(AtomicBool::new(false));

        // 1秒だけスリープ
        interruptible_sleep(1, "テスト待機", &progress, &cancel_flag);

        // progress が Waiting に更新されていること
        let p = progress.lock().unwrap().clone();
        assert!(matches!(
            p,
            PipelineProgress::Waiting { ref step, .. } if step == "テスト待機"
        ));
    }

    // ---- キャンセルフラグの同時実行テスト ----

    #[test]
    fn test_cancel_flag_shared_between_threads() {
        let progress = Arc::new(Mutex::new(PipelineProgress::Pending));
        let cancel_flag = Arc::new(AtomicBool::new(false));

        let cf = Arc::clone(&cancel_flag);

        // 別スレッドでスリープ開始
        let handle = std::thread::spawn(move || {
            interruptible_sleep(60, "長期待機", &progress, &cf);
        });

        // 100ms 後にキャンセル
        std::thread::sleep(Duration::from_millis(100));
        cancel_flag.store(true, Ordering::SeqCst);

        // スレッドが1秒以内に終了すること
        let start = Instant::now();
        handle.join().unwrap();
        assert!(start.elapsed() < Duration::from_secs(2));
    }

    // ---- resolve_worktree_branch（§7.1.6・Phase 7）----

    #[test]
    fn test_resolve_worktree_branch_isolation() {
        // isolation → <block_name>-<run_id>
        let branch = resolve_worktree_branch("isolation", "sai-test-main", 42);
        assert_eq!(branch.as_deref(), Some("sai-test-main-42"));
    }

    #[test]
    fn test_resolve_worktree_branch_none() {
        assert_eq!(resolve_worktree_branch("none", "sai-test-main", 1), None);
        assert_eq!(resolve_worktree_branch("", "sai-test-main", 1), None);
    }

    #[test]
    fn test_resolve_worktree_branch_literal() {
        // リテラルブランチ名（後方互換）
        let branch = resolve_worktree_branch("feature/sai-test", "sai-test-main", 1);
        assert_eq!(branch.as_deref(), Some("feature/sai-test"));
    }

    /// テスト用の最小 BlockConfig。
    fn worktree_test_block(worktree: Option<&str>) -> BlockConfig {
        BlockConfig {
            name: "wt-test".to_string(),
            description: None,
            model: None,
            thinking: None,
            tools: None,
            extensions: None,
            skills: None,
            subblocks: None,
                timeout: None,
            worktree: worktree.map(str::to_string),
            enable: None,
            category: None,
            templates: None,
            id: None,
        }
    }

    /// worktree の 3 通り（未指定 / none / isolation）× ブロック設定の優先順位。
    ///
    /// - 未指定（None）→ ブロック定義に従う
    /// - `"none"` → ブロック定義の isolation も打ち消して無効化
    /// - `"isolation"` → そのまま（ブロック定義と同値）
    #[test]
    fn test_worktree_priority_three_states_vs_block_config() {
        // ブロック定義が isolation を要求している場合
        let config = worktree_test_block(Some("isolation"));

        // ① 未指定 → ブロック定義に従う（isolation）
        let o = ExecutionOverrides {
            worktree: None,
            ..ExecutionOverrides::default()
        };
        assert_eq!(
            resolve_execution_settings(&config, &o).worktree.as_deref(),
            Some("isolation"),
            "未指定はブロック定義に従う"
        );

        // ② none → ブロック定義の isolation を打ち消す
        let o = ExecutionOverrides {
            worktree: Some("none".to_string()),
            ..ExecutionOverrides::default()
        };
        assert_eq!(
            resolve_execution_settings(&config, &o).worktree,
            None,
            "none は明示的に作業用コピーを作らない"
        );

        // ③ isolation → そのまま
        let o = ExecutionOverrides {
            worktree: Some("isolation".to_string()),
            ..ExecutionOverrides::default()
        };
        assert_eq!(
            resolve_execution_settings(&config, &o).worktree.as_deref(),
            Some("isolation")
        );

        // ブロック定義が未指定なら、未指定は None のまま
        let config_none = worktree_test_block(None);
        let o = ExecutionOverrides {
            worktree: None,
            ..ExecutionOverrides::default()
        };
        assert_eq!(resolve_execution_settings(&config_none, &o).worktree, None);
    }

    /// 一時 git リポジトリを作る（worktree テスト用）。
    fn init_temp_git_repo() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let git = |args: &[&str]| {
            let status = std::process::Command::new("git")
                .args(args)
                .current_dir(dir.path())
                .env("GIT_AUTHOR_NAME", "sai-test")
                .env("GIT_AUTHOR_EMAIL", "sai-test@example.com")
                .env("GIT_COMMITTER_NAME", "sai-test")
                .env("GIT_COMMITTER_EMAIL", "sai-test@example.com")
                .status()
                .unwrap();
            assert!(status.success(), "git {:?} が失敗しました", args);
        };
        git(&["init", "-q"]);
        std::fs::write(dir.path().join("README.md"), "init\n").unwrap();
        // サブフォルダも commit 済みにしておく（`git worktree add` は HEAD を
        // checkout するため、未 commit のフォルダは作業用コピーに現れない）。
        let sub = dir.path().join("crates/app");
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::write(sub.join(".keep"), "\n").unwrap();
        git(&["add", "-A"]);
        git(&["commit", "-q", "-m", "init"]);
        dir
    }

    /// 別の一時 git リポジトリを cwd にしたとき、そのリポジトリの
    /// `.worktrees/<branch>` に作業用コピーができ、サブフォルダ指定なら
    /// 作業用コピー内の同じサブフォルダが返ること。
    #[test]
    fn test_apply_worktree_for_cwd_uses_cwd_repository_and_subfolder() {
        let repo = init_temp_git_repo();
        let sub = repo.path().join("crates/app");
        std::fs::create_dir_all(&sub).unwrap();
        let branch = "test-block-99";
        let wt = apply_worktree_for_cwd(&sub, branch).unwrap();

        // リポジトリ直下の .worktrees/<branch> に作られる
        // （git はシンボリックリンクを解決した実パスを返すため canonicalize して比較）
        let repo_canon = repo.path().canonicalize().unwrap();
        let expected_root = repo_canon.join(".worktrees").join(branch);
        assert!(
            expected_root.is_dir(),
            ".worktrees/{} が作られる: {}",
            branch,
            expected_root.display()
        );
        // サブフォルダ指定は作業用コピー内の同じサブフォルダへ移動する
        assert_eq!(wt, expected_root.join("crates/app"));
        assert!(wt.is_dir(), "作業用コピー内のサブフォルダが存在する");
        assert!(
            wt.join(".keep").is_file(),
            "作業用コピーにリポジトリの内容が入っている"
        );

        // ブランチが切られている
        let out = std::process::Command::new("git")
            .args(["-C"])
            .arg(&wt)
            .args(["rev-parse", "--abbrev-ref", "HEAD"])
            .output()
            .unwrap();
        assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), branch);
    }

    /// リポジトリのルートそのものを cwd にした場合も `.worktrees/<branch>` を返す。
    #[test]
    fn test_apply_worktree_for_cwd_from_repo_root() {
        let repo = init_temp_git_repo();
        let wt = apply_worktree_for_cwd(repo.path(), "root-case-1").unwrap();
        let repo_canon = repo.path().canonicalize().unwrap();
        assert_eq!(wt, repo_canon.join(".worktrees/root-case-1"));
        assert!(wt.is_dir());
    }

    /// worktree_plan が cwd のリポジトリを基準にブランチ・リポジトリ・場所を返す。
    #[test]
    fn test_worktree_plan_uses_cwd_repository() {
        let repo = init_temp_git_repo();
        let plan = worktree_plan(Some(repo.path()), "isolation", "test-block", 7)
            .expect("git リポジトリなら計画が返る");
        let repo_canon = repo.path().canonicalize().unwrap();
        assert_eq!(plan.branch, "test-block-7");
        assert_eq!(plan.repo_root, repo_canon);
        assert_eq!(
            plan.worktree_path,
            repo_canon.join(".worktrees/test-block-7")
        );
        // none は計画なし
        assert!(worktree_plan(Some(repo.path()), "none", "test-block", 7).is_none());
    }

    /// `p` 再開用: 子タスクの実行フォルダが起動時と同じ場所になること。
    #[test]
    fn test_subtask_run_cwd() {
        let repo = init_temp_git_repo();
        let repo_canon = repo.path().canonicalize().unwrap();
        let sub = repo.path().join("crates/app");
        let mk = |worktree: &str, cwd: &Path| {
            let mut t = crate::db::models::Task::new(
                1,
                "s".into(),
                "d".into(),
                Some("worker".into()),
                None,
                None,
                worktree.into(),
                None,
                None,
            );
            t.cwd = Some(cwd.to_string_lossy().to_string());
            t
        };

        // 作業用コピーがまだ無い → None（呼び出し元はプロジェクトルートで再開する）
        assert_eq!(subtask_run_cwd(&mk("", &sub), Some("isolation"), 5), None);

        // ブロック定義が isolation → 作業用コピー内の同じサブフォルダ
        apply_worktree_for_cwd(&sub, "worker-5").unwrap();
        assert_eq!(
            subtask_run_cwd(&mk("", &sub), Some("isolation"), 5),
            Some(repo_canon.join(".worktrees/worker-5/crates/app"))
        );

        // 親が none を指定 → ブロック定義が isolation でも task.cwd
        assert_eq!(
            subtask_run_cwd(&mk("none", &sub), Some("isolation"), 5),
            Some(sub.clone())
        );

        // 指定もブロック定義も無い → task.cwd
        assert_eq!(subtask_run_cwd(&mk("", repo.path()), None, 5), Some(repo.path().to_path_buf()));
    }

    // ---- resolve_execution_settings（§6）----

    #[test]
    fn test_resolve_execution_settings_defaults() {
        // オーバーライドなし → フロントマター値 / エンジンデフォルト
        let config = BlockConfig {
            name: "test".to_string(),
            description: None,
            model: Some("inherit".to_string()),
            thinking: None,
            tools: None,
            extensions: None,
            skills: None,
            subblocks: None,
                timeout: Some(600),
            worktree: None,
            enable: None,
            category: None,
            templates: None,
            id: None,
        };
        let resolved = resolve_execution_settings(&config, &ExecutionOverrides::default());
        assert_eq!(resolved.timeout_ms, 600_000); // フロントマター値
        assert_eq!(resolved.direction, SplitDirection::Right);
        assert!(resolved.worktree.is_none());
    }

    #[test]
    fn test_resolve_execution_settings_overrides() {
        let config = BlockConfig {
            name: "test".to_string(),
            description: None,
            model: Some("inherit".to_string()),
            thinking: Some("low".to_string()),
            tools: None,
            extensions: None,
            skills: None,
            subblocks: None,
            timeout: Some(300),
            worktree: Some("none".to_string()),
            enable: None,
            category: None,
            templates: None,
            id: None,
        };
        let overrides = ExecutionOverrides {
            model: Some("anthropic/claude-sonnet-4-5".to_string()),
            thinking: Some("high".to_string()),
            timeout: Some(900),
            direction: Some(SplitDirection::Down),
            worktree: Some("isolation".to_string()),
            cwd: None,
            keep_pane: false,
        };
        let resolved = resolve_execution_settings(&config, &overrides);
        // オーバーライドが優先される
        assert_eq!(resolved.timeout_ms, 900_000);
        assert_eq!(resolved.direction, SplitDirection::Down);
        assert_eq!(resolved.worktree.as_deref(), Some("isolation"));
        assert_eq!(
            resolved.config.model.as_deref(),
            Some("anthropic/claude-sonnet-4-5")
        );
        assert_eq!(resolved.config.thinking.as_deref(), Some("high"));
    }

    #[test]
    fn test_resolve_execution_settings_engine_default_timeout() {
        // フロントマター・オーバーライドとも未指定 → 実質無期限（NO_TIMEOUT_MS）
        let config = BlockConfig {
            name: "test".to_string(),
            description: None,
            model: None,
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
        };
        let resolved = resolve_execution_settings(&config, &ExecutionOverrides::default());
        assert_eq!(resolved.timeout_ms, NO_TIMEOUT_MS);
    }

    // ---------------------------------------------------------------------
    // サブタスク検証（P2: subblocks 制限・入力検証）
    // ---------------------------------------------------------------------

    #[test]
    fn test_is_subblock_allowed_none() {
        // subblocks 未指定 → サブブロックは呼び出せない
        let err = is_subblock_allowed("parent", &None, "any-block")
            .expect_err("subblocks 未指定の親はサブブロック呼び出し不可のはず");
        let msg = format!("{}", err);
        assert!(msg.contains("parent"));
        assert!(msg.contains("subblocks"));
    }

    #[test]
    fn test_is_subblock_allowed_in_list() {
        let allowed = Some(vec!["sai-ui-impl".to_string(), "sai-code-work".to_string()]);
        assert!(is_subblock_allowed("parent", &allowed, "sai-ui-impl").is_ok());
        assert!(is_subblock_allowed("parent", &allowed, "sai-code-work").is_ok());
    }

    #[test]
    fn test_is_subblock_allowed_denied() {
        let allowed = Some(vec!["sai-ui-impl".to_string()]);
        let err = is_subblock_allowed("parent", &allowed, "sai-security-audit")
            .expect_err("subblocks に含まれないブロックは拒否されるべき");
        let msg = format!("{}", err);
        assert!(msg.contains("sai-security-audit"));
        assert!(msg.contains("parent"));
        assert!(msg.contains("sai-ui-impl"));
    }

    #[test]
    fn test_is_subblock_allowed_empty_list() {
        // 空リスト → すべて拒否（"(none)" 表示）
        let allowed = Some(Vec::new());
        let err = is_subblock_allowed("parent", &allowed, "x").expect_err("空リストは全拒否");
        assert!(format!("{}", err).contains("(none)"));
    }

    // ---------------------------------------------------------------------
    // P3-A: サブ実行レコードへの batch_id 保存
    // ---------------------------------------------------------------------

    /// P3-A の回帰テスト: サブ実行レコード作成時に batch_id と親 run が保存される。
    #[test]
    fn test_subtask_record_saves_batch_id() {
        use crate::db::models::test_helpers::create_test_block;
        use crate::db::SaiStore;

        let store = SaiStore::open_in_memory().unwrap();
        // ブロック定義を登録（ファイルパスは存在しなくても upsert は動作する）
        let _block_id = store.insert_block(&create_test_block("test-sub")).unwrap();
        let parent_block_id = store
            .insert_block(&create_test_block("test-parent"))
            .unwrap();
        let parent_run_id = store.create_execution(parent_block_id, None).unwrap();

        // サブ実行レコード作成（P3-A: batch_id を渡す。run_subtask →
        // execute_block_pipeline → prepare_execution_record の流れを再現）
        let sub_config = BlockConfig {
            name: "test-sub".to_string(),
            description: None,
            model: Some("inherit".to_string()),
            thinking: None,
            tools: None,
            extensions: None,
            skills: None,
            subblocks: None,
            timeout: Some(300),
            worktree: None,
            enable: None,
            category: None,
            templates: None,
            id: None,
        };
        let prep = prepare_execution_record(
            &store,
            &sub_config,
            std::path::Path::new("blocks/test-sub.md"),
            None,
            Some(parent_run_id),
            Some("batch_test_r1"),
        )
        .expect("サブ実行レコードの作成に失敗");

        let sub_exec = store.get_execution_by_id(prep.run_id).unwrap();
        assert_eq!(sub_exec.parent_run_id, Some(parent_run_id));
        assert_eq!(
            sub_exec.batch_id.as_deref(),
            Some("batch_test_r1"),
            "サブ実行の batch_id が保存される（P3-A）"
        );
    }

    /// P1-4 の回帰テスト: サブタスクのプロンプト本文に `# タスク:` が
    /// 自動挿入されないこと（ブロック定義本文の `{{subject}}` / `{{description}}`
    /// のみが展開される）。
    #[test]
    fn test_subtask_body_uses_placeholders_without_task_header() {
        // プレースホルダーがある本文: 該当位置のみ展開される
        let body = crate::placeholder::expand_task_placeholders(
            "件名: {{subject}}\n詳細: {{description}}",
            "t1 結果の要約",
            "要約して保存する",
        );
        assert_eq!(body, "件名: t1 結果の要約\n詳細: 要約して保存する");
        assert!(
            !body.starts_with("# タスク:"),
            "自動「# タスク:」セクションは挿入されない（P1-4）"
        );

        // プレースホルダーがない本文: そのまま（自動挿入なし）
        let body2 = crate::placeholder::expand_task_placeholders(
            "# サブタスク: 入力文書の要約\n\n## 手順\n要約してください。",
            "t1 結果の要約",
            "要約して保存する",
        );
        assert_eq!(
            body2,
            "# サブタスク: 入力文書の要約\n\n## 手順\n要約してください。"
        );
        assert!(
            !body2.contains("# タスク: t1"),
            "本文に subject が先頭挿入されない（P1-4）"
        );
    }

    // ---------------------------------------------------------------------
    // U2: todo のカードを自分の子として起こす（予約 run を子にする）
    // ---------------------------------------------------------------------

    /// テスト用の子ブロック設定を作る（実行時の設定解決にだけ使う最小のもの）。
    fn card_child_config(block_id: &str) -> BlockConfig {
        BlockConfig {
            id: Some(block_id.to_string()),
            name: block_id.to_string(),
            description: None,
            model: None,
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
        }
    }

    /// U2 AC2: 予約 run を子として起こすと、run_id が予約のまま、親（`parent_run_id`）・
    /// `batch_id`・親から子への線が今の子と同じ形で作られる。
    #[test]
    fn test_prepare_execution_record_full_uses_reserved_run_as_child() {
        use crate::db::SaiStore;

        let store = SaiStore::open_in_memory().unwrap();
        let parent_block_id = store
            .insert_block(&crate::db::models::test_helpers::create_test_block(
                "reserved-child-parent",
            ))
            .unwrap();
        let parent_run_id = store.create_execution(parent_block_id, None).unwrap();
        let child_config = card_child_config("reserved-child");
        let child_file = std::path::Path::new("blocks/reserved-child.md");
        let reserved = store.create_reserved_run(None).unwrap();

        let prep = prepare_execution_record_full(
            &store,
            &child_config,
            child_file,
            Some(r#"{"task_text":"カード本文"}"#),
            Some(parent_run_id),
            Some("batch_20260930_r1_abcd"),
            Some(reserved),
        )
        .expect("予約 run を子として起こせる");

        // run_id は予約のまま（新たに採番しない）
        assert_eq!(prep.run_id, reserved);

        let child = store.get_execution_by_id(reserved).unwrap();
        assert_eq!(child.status, "running");
        assert_eq!(child.parent_run_id, Some(parent_run_id));
        assert_eq!(child.batch_id.as_deref(), Some("batch_20260930_r1_abcd"));

        // 親から子への task_delegation 線がある（今の子と同じ作り）
        let conns = store.list_connections().unwrap();
        let delegation = conns
            .iter()
            .find(|c| c.target_execution_id == reserved)
            .expect("親から子への線が作られている");
        assert_eq!(delegation.source_execution_id, parent_run_id);
        assert_eq!(delegation.target_execution_id, reserved);
        assert_eq!(
            delegation.connection_type, "task_delegation",
            "線は task_delegation と同じ"
        );

        // もう一度起こそうとすると `todo` でないので失敗する（予約行を使い切っている）
        let err = prepare_execution_record_full(
            &store,
            &child_config,
            child_file,
            None,
            Some(parent_run_id),
            None,
            Some(reserved),
        )
        .expect_err("予約行は 1 度しか確定できない");
        assert!(
            format!("{}", err).contains("Failed to claim the reserved run"),
            "実際のエラー: {}",
            err
        );
    }

    /// カード無しの子（`reserved_run_id = None`）の挙動が変わっていないこと（AC4）。
    ///
    /// 新しく採番され、`parent_run_id` と `batch_id` が入り、親から子への線も作られる。
    #[test]
    fn test_prepare_execution_record_full_child_without_card_still_creates_new_run() {
        use crate::db::SaiStore;

        let store = SaiStore::open_in_memory().unwrap();
        let parent_block_id = store
            .insert_block(&crate::db::models::test_helpers::create_test_block(
                "plain-child-parent",
            ))
            .unwrap();
        let parent_run_id = store.create_execution(parent_block_id, None).unwrap();
        let child_config = card_child_config("plain-child");

        let prep = prepare_execution_record_full(
            &store,
            &child_config,
            std::path::Path::new("blocks/plain-child.md"),
            None,
            Some(parent_run_id),
            Some("batch_plain_r1"),
            None,
        )
        .expect("カード無しの子は従来どおり作れる");

        assert_ne!(prep.run_id, parent_run_id);
        let child = store.get_execution_by_id(prep.run_id).unwrap();
        assert_eq!(child.status, "running");
        assert_eq!(child.parent_run_id, Some(parent_run_id));
        assert_eq!(child.batch_id.as_deref(), Some("batch_plain_r1"));
        let conns = store.list_connections().unwrap();
        assert!(
            conns
                .iter()
                .any(|c| c.source_execution_id == parent_run_id
                    && c.target_execution_id == prep.run_id),
            "カード無しの子にも親から子への線がある"
        );
    }

    /// U2 AC3: カード指定の子は run フォルダが `running/<run_id>/` に移り、
    /// task.md の `sci_*` と `executer: professor` がそのまま残る。
    #[test]
    fn test_reserved_child_run_dir_moves_todo_to_running_keeping_sci_frontmatter() {
        use crate::db::SaiStore;

        let _guard = finalize_env_lock();
        let temp_root = finalize_test_temp_dir("reserved-child-dir");
        let _env_guard = EnvVarGuard::set("AGENT_LINK_PROJECT_ROOT", temp_root.as_os_str());

        let store = SaiStore::open_in_memory().unwrap();
        let parent_block_id = store
            .insert_block(&crate::db::models::test_helpers::create_test_block(
                "reserved-dir-parent",
            ))
            .unwrap();
        let parent_run_id = store.create_execution(parent_block_id, None).unwrap();
        let child_config = card_child_config("reserved-dir-child");
        let reserved = store.create_reserved_run(None).unwrap();

        // 予約 run のカードを作る（todo/<run_id>/task.md・メタの項目つき）
        let card_path = todo_card_path(reserved);
        std::fs::create_dir_all(card_path.parent().unwrap()).unwrap();
        let card_text = format!(
            "---\ncreation_date: 2026/09/30 10:00:00\nexecution_date: -\nrun_id: {}\n\
             description: N12 の読解\nrunner: pi\nblock: reserved-dir-child\nthinking: \n\
             model: \nprovider: \ntag: \n\
             executer: professor\nnote: n1\n---\n\n# task_text\n\nN12 を読解する\n",
            reserved
        );
        std::fs::write(&card_path, &card_text).unwrap();

        // run フォルダの移動: todo → running（execute_block_pipeline_full と同じ手順）
        crate::run_dir::ensure_run_dir(crate::run_dir::TODO_OWNER, reserved).unwrap();
        let prep = prepare_execution_record_full(
            &store,
            &child_config,
            std::path::Path::new("blocks/reserved-dir-child.md"),
            Some(r#"{"task_text":"N12 を読解する"}"#),
            Some(parent_run_id),
            Some("batch_reserved_r1"),
            Some(reserved),
        )
        .expect("予約 run を子として起こせる");
        assert_eq!(prep.run_id, reserved);
        crate::run_dir::move_run_dir(reserved, crate::run_dir::TODO_OWNER, crate::run_dir::RUNNING_OWNER)
            .expect("run フォルダを running へ移せる");

        // 移動後は running/<run_id>/task.md
        let (owner, dir) = crate::run_dir::find_run_dir(reserved).expect("run フォルダが見つかる");
        assert_eq!(owner, crate::run_dir::RUNNING_OWNER);
        assert_eq!(
            dir,
            crate::run_dir::ensure_run_dir(crate::run_dir::RUNNING_OWNER, reserved).unwrap()
        );

        // カード由来の task.md を書き直す処理は task.md があれば何もしない
        let resolved = resolve_execution_settings(&child_config, &ExecutionOverrides::default());
        let mut inputs: HashMap<String, String> = HashMap::new();
        inputs.insert("task_text".to_string(), "別の本文".to_string());
        write_run_task_md(&store, reserved, &resolved, &inputs, None);

        let moved_text = std::fs::read_to_string(dir.join(crate::run_dir::TASK_FILE)).unwrap();
        assert!(
            moved_text.contains("executer: professor"),
            "executer: professor が残る:\n{}",
            moved_text
        );
        assert!(
            moved_text.contains("note: n1"),
            "メタの項目（note）が残る:\n{}",
            moved_text
        );
        assert!(
            moved_text.contains("N12 を読解する"),
            "カード本文の task_text が残る:\n{}",
            moved_text
        );
        assert!(
            !moved_text.contains("別の本文"),
            "既存の task.md を別の本文で上書きしない:\n{}",
            moved_text
        );

        // 予約行は running になり、子の行として扱われる
        let child = store.get_execution_by_id(reserved).unwrap();
        assert_eq!(child.status, "running");
        assert_eq!(child.parent_run_id, Some(parent_run_id));

        std::fs::remove_dir_all(&temp_root).ok();
    }

    // ---- U4: Ctrl+D finalize（request_main_block_finalize） ----

    /// AGENT_LINK_PROJECT_ROOT を書き換えるテスト用のロック（環境変数はプロセス共有のため
    /// 並列実行される他テストと直列化する）。ロック保持中に別テストがパニックしても
    /// 後続テストが Poison で落ちないよう、取得時は毒化を回復する。
    /// FINALIZE_ENV_LOCK を毒化回復付きで取得する。
    fn finalize_env_lock() -> std::sync::MutexGuard<'static, ()> {
        // 環境変数を設定する他モジュールのテスト（run_dir 等）と排他する
        crate::common::AGENT_LINK_ENV_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner())
    }

    /// ユニークな一時ディレクトリを作成する（テスト用プロジェクトルート）。
    fn finalize_test_temp_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "agent-link-finalize-{}-{}",
            name,
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("一時ディレクトリ作成");
        dir
    }

    /// 環境変数を一時的に設定し、スコープ終了時（パニック時含む）に元へ戻すガード。
    struct EnvVarGuard {
        key: String,
        prev: Option<std::ffi::OsString>,
    }

    impl EnvVarGuard {
        fn set(key: &str, value: &std::ffi::OsStr) -> Self {
            let prev = std::env::var_os(key);
            std::env::set_var(key, value);
            EnvVarGuard {
                key: key.to_string(),
                prev,
            }
        }

        /// 環境変数を一時的に削除する（テスト内で未設定を保証する）。
        fn remove(key: &str) -> Self {
            let prev = std::env::var_os(key);
            std::env::remove_var(key);
            EnvVarGuard {
                key: key.to_string(),
                prev,
            }
        }
    }

    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            match &self.prev {
                Some(v) => std::env::set_var(&self.key, v),
                None => std::env::remove_var(&self.key),
            }
        }
    }

    /// request_main_block_finalize が
    /// 1. finalize_request 通知を pending/ に書き込む
    /// 2. DB を completed + finalized=true にする
    /// ことを検証する（U4・R8/R9）。pane 削除は herdr 不在のためベストエフォートで
    /// 失敗するが、全体は成功（最終状態は completed）になる。
    #[test]
    fn test_request_main_block_finalize() {
        // 環境変数・通知ディレクトリを触るテストと直列化する（並列干渉防止）。
        // AGENT_LINK_ENV_TEST_LOCK は通知ディレクトリ・環境変数の両方を保護する。
        let _guard = finalize_env_lock();
        let temp_root = finalize_test_temp_dir("request-finalize");
        let _env_guard = EnvVarGuard::set("AGENT_LINK_PROJECT_ROOT", temp_root.as_os_str());

        // 実行中メインブロックを用意
        let store = crate::db::SaiStore::open_in_memory().unwrap();
        let block = crate::db::models::test_helpers::create_test_block("finalize-main");
        let block_id = store.insert_block(&block).unwrap();
        let run_id = store.create_execution(block_id, None).unwrap();
        store.update_execution_status(run_id, "running").unwrap();

        // Ctrl+D 2回目相当の finalize を実行（herdr 不在のため pane 削除は失敗で無視）
        request_main_block_finalize(&store, run_id, "pane-finalize-test");

        // 1. finalize_request 通知が pending/ に書き込まれている
        let pending = crate::common::notify_pending_dir();
        let finalize_files: Vec<_> = std::fs::read_dir(&pending)
            .expect("pending ディレクトリが存在する")
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().contains("finalize_request"))
            .collect();
        assert!(
            !finalize_files.is_empty(),
            "finalize_request 通知が書き込まれるべき"
        );
        let content =
            std::fs::read_to_string(finalize_files[0].path()).expect("通知ファイルを読める");
        let json: serde_json::Value = serde_json::from_str(&content).unwrap();
        assert_eq!(json["type"], "finalize_request");
        assert_eq!(json["run_id"], run_id);

        // 2. DB が completed + finalized=true になっている（agent-link block list で「完了」表示）
        let exec = store.get_execution_by_id(run_id).unwrap();
        assert_eq!(exec.status, "completed", "Ctrl+D finalize は正常完了扱い");
        assert!(exec.finalized, "finalize フラグが true になる");

        let _ = std::fs::remove_dir_all(&temp_root);
    }

    /// cc の message.md は、その run 自身のセッション（`agent_session_id`）の
    /// 会話だけを出す（U03）。`~/.claude/projects/` 全体で一番新しいセッションを
    /// 使うと、別 run の会話が出る。
    #[test]
    fn test_write_message_file_cc_uses_own_session_transcript() {
        let _guard = finalize_env_lock();
        let temp_root = finalize_test_temp_dir("cc-message");
        // project_root()（resolve_transcript 用）と claude_projects_dir()（HOME 用）の
        // 両方を一時フォルダに向ける。
        let _root_env = EnvVarGuard::set("AGENT_LINK_PROJECT_ROOT", temp_root.as_os_str());
        let _home_env = EnvVarGuard::set("HOME", temp_root.as_os_str());

        // 2 つの cc セッションファイルを作る（agent_session_id が違う）。
        let cc_dir = temp_root
            .join(".claude")
            .join("projects")
            .join(crate::session::claude_project_slug(&temp_root));
        std::fs::create_dir_all(&cc_dir).unwrap();
        let sid_a = "aaaaaaaa-1111-4111-8111-111111111111";
        let sid_b = "bbbbbbbb-2222-4222-8222-222222222222";
        for (sid, marker) in [(sid_a, "会話A"), (sid_b, "会話B")] {
            let line = serde_json::json!({
                "type": "user",
                "message": { "role": "user", "content": [{"type": "text", "text": marker}] }
            });
            std::fs::write(
                cc_dir.join(format!("{}.jsonl", sid)),
                format!("{}\n", line),
            )
            .unwrap();
        }

        let store = crate::db::SaiStore::open_in_memory().unwrap();
        let block = crate::db::models::test_helpers::create_test_block("cc-msg-main");
        let block_id = store.insert_block(&block).unwrap();
        let run_a = store.create_execution(block_id, None).unwrap();
        let run_b = store.create_execution(block_id, None).unwrap();
        for (run_id, sid) in [(run_a, sid_a), (run_b, sid_b)] {
            store
                .update_execution_agent_session_id(run_id, sid)
                .unwrap();
        }

        let md_a = write_message_file(&store, run_a).expect("run A の message.md を書ける");
        let md_b = write_message_file(&store, run_b).expect("run B の message.md を書ける");
        let text_a = std::fs::read_to_string(&md_a).unwrap();
        let text_b = std::fs::read_to_string(&md_b).unwrap();

        assert!(text_a.contains("会話A"), "run A は自分の会話: {}", text_a);
        assert!(
            !text_a.contains("会話B"),
            "run A に別 run の会話が混ざらない: {}",
            text_a
        );
        assert!(text_b.contains("会話B"), "run B は自分の会話: {}", text_b);
        assert!(
            !text_b.contains("会話A"),
            "run B に別 run の会話が混ざらない: {}",
            text_b
        );

        let _ = std::fs::remove_dir_all(&temp_root);
    }

    /// cc の実行でセッションが見つからないときは空のメッセージ列になる（U03）。
    /// 「最新セッション」に戻さない。
    #[test]
    fn test_write_message_file_cc_without_transcript_is_empty() {
        let _guard = finalize_env_lock();
        let temp_root = finalize_test_temp_dir("cc-message-empty");
        let _root_env = EnvVarGuard::set("AGENT_LINK_PROJECT_ROOT", temp_root.as_os_str());
        let _home_env = EnvVarGuard::set("HOME", temp_root.as_os_str());

        let store = crate::db::SaiStore::open_in_memory().unwrap();
        let block = crate::db::models::test_helpers::create_test_block("cc-msg-empty");
        let block_id = store.insert_block(&block).unwrap();
        let run_id = store.create_execution(block_id, None).unwrap();
        // agent_session_id を入れないので解決できない
        let md = write_message_file(&store, run_id).expect("message.md を書ける");
        let text = std::fs::read_to_string(&md).unwrap();
        assert!(
            !text.contains("# user") && !text.contains("# assistant"),
            "セッションが無いときは会話が出ない: {}",
            text
        );

        let _ = std::fs::remove_dir_all(&temp_root);
    }

    /// finalize 後の実行が session_dir を持ち、その直下 .jsonl が
    /// find_main_session_file で解決できること（U4・R9: 右SB Session 再開）。
    #[test]
    fn test_finalized_execution_session_is_resolvable() {
        let store = crate::db::SaiStore::open_in_memory().unwrap();
        let block = crate::db::models::test_helpers::create_test_block("session-main");
        let block_id = store.insert_block(&block).unwrap();
        let run_id = store.create_execution(block_id, None).unwrap();

        // セッションディレクトリを用意（実行開始時に永続化される想定）
        let session_dir = finalize_test_temp_dir("session-dir");
        let session_file = session_dir.join("main.jsonl");
        std::fs::write(&session_file, "{\"type\":\"user\"}\n").unwrap();

        // session_dir を保存（右SB 表示用・U4 のセッション永続化）
        store
            .update_execution_session_info(run_id, &session_dir.to_string_lossy(), "pi --test")
            .unwrap();
        store.update_execution_status(run_id, "completed").unwrap();
        store.update_execution_finalized(run_id, true).unwrap();

        // 取得した実行から session_dir 直下の .jsonl が解決できる
        let exec = store.get_execution_by_id(run_id).unwrap();
        let dir = exec
            .session_dir
            .clone()
            .expect("session_dir が永続化されている");
        let resolved = crate::session::find_main_session_file(std::path::Path::new(&dir))
            .expect("session_dir 直下の .jsonl が解決できる");
        assert_eq!(resolved, session_file);

        let _ = std::fs::remove_dir_all(&session_dir);
    }

    /// P6-2: run_batches を finalized にするのはメインブロック（親実行なし）だけ。
    /// サブブロックでは親の実行バッチを触らない。
    #[test]
    fn test_should_finalize_run_batch_only_for_main_block() {
        assert!(
            should_finalize_run_batch(None),
            "メインブロックは run_batches を finalized にしてよい"
        );
        assert!(
            !should_finalize_run_batch(Some(42)),
            "サブブロックは run_batches を触らない"
        );
    }

    // ── `{{context}}` は特別扱いしない ──

    /// 本文の `{{context}}` は置き換えず、そのまま固定部分に残る。
    /// 最初の文には入力値の節だけがつく。
    #[test]
    fn test_prepare_pipeline_leaves_context_notation_alone() {
        let _env_lock = finalize_env_lock();
        let root = finalize_test_temp_dir("context");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join(".agent-link")).unwrap();
        let _root_guard = EnvVarGuard::set("AGENT_LINK_PROJECT_ROOT", root.as_os_str());
        let _home_guard = EnvVarGuard::set("AGENT_LINK_HOME", root.as_os_str());

        let mut config = minimal_config();
        config.run_id = Some(99);
        config.block_body = "前\n{{context}}\n後".to_string();
        let prepared = prepare_pipeline(&config).expect("prepare_pipeline に失敗");
        assert_eq!(prepared.system_prompt, "前\n{{context}}\n後");
        // 固定部分が有るのに節が無いので、最初の文は案内だけ
        assert_eq!(
            prepared.expanded_prompt,
            "system prompt の指示に従って始めてください。"
        );
        crate::session::cleanup_session_dir(&prepared.session_dir);

        let _ = std::fs::remove_dir_all(&root);
    }

    // ---- U03: render_message_md の見出しの構造 ----

    /// 発話列を (role, text) の vec にする補助。
    fn render_msgs(pairs: &[(&str, &str)]) -> String {
        let messages: Vec<(String, String)> = pairs
            .iter()
            .map(|(role, text)| (role.to_string(), text.to_string()))
            .collect();
        render_message_md("plan", 129, &messages)
    }

    /// 最初の発話が `# context` で始まるなら、区切りなしでそのまま出る。
    #[test]
    fn test_render_message_md_first_user_context_has_no_divider() {
        let text = render_msgs(&[
            ("user", "# context\n## 今回の行動\n## 資料\n\n# task_text\n依頼"),
            ("assistant", "# 結論\n本文"),
            ("user", "# 追加\n続けて"),
            ("assistant", "了解"),
        ]);
        assert!(!text.contains("# user (0)"), "{}", text);
        assert!(!text.contains("# user (2)"), "{}", text);
        assert!(text.contains("# context\n## 今回の行動\n## 資料"), "{}", text);
        assert!(text.contains("# task_text\n依頼"), "{}", text);
        // 最初の発話の内部は下げない
        assert!(text.contains("\n# context\n"), "{}", text);
        assert!(text.contains("\n# task_text\n"), "{}", text);
        // 次の利用者の発話は 1 から
        assert!(text.contains("# user (1)\n\n## 追加\n続けて"), "{}", text);
        assert!(text.contains("# assistant (1)\n\n## 結論\n本文"), "{}", text);
        assert!(text.contains("# assistant (2)\n\n了解"), "{}", text);
        assert!(!text.contains("## user ("), "{}", text);
        assert!(!text.contains("## assistant ("), "{}", text);
    }

    /// context が無く最初の発話が `# task_text` で始まる場合も同じ。
    #[test]
    fn test_render_message_md_first_user_task_text_has_no_divider() {
        let text = render_msgs(&[
            ("user", "# task_text\n依頼"),
            ("assistant", "はい"),
        ]);
        assert!(text.contains("\n# task_text\n依頼\n"), "{}", text);
        assert!(!text.contains("# user ("), "{}", text);
        assert!(text.contains("# assistant (1)"), "{}", text);
    }

    /// 最初の発話が自動の節で始まらないときは `# user (1)` として残る。
    #[test]
    fn test_render_message_md_first_user_plain_keeps_divider() {
        let text = render_msgs(&[
            ("user", "hello"),
            ("assistant", "world"),
            ("user", "again"),
            ("assistant", "ok"),
        ]);
        assert!(text.contains("# user (1)\n\nhello"), "{}", text);
        assert!(text.contains("# assistant (1)\n\nworld"), "{}", text);
        assert!(text.contains("# user (2)\n\nagain"), "{}", text);
        assert!(text.contains("# assistant (2)\n\nok"), "{}", text);
    }

    /// assistant の本文の見出しは 1 段下がる。コードブロックの中の `#` は下がらない。
    #[test]
    fn test_render_message_md_shifts_assistant_headings() {
        let text = render_msgs(&[("assistant", "# 結論\n本文\n\n```bash\n# コメント\n```\n\n## 補足")]);
        assert!(text.contains("# assistant (1)\n\n## 結論\n"), "{}", text);
        assert!(text.contains("\n```bash\n# コメント\n```"), "{}", text);
        assert!(text.contains("### 補足"), "{}", text);
    }

    /// `# contextual` で始まる最初の発話は、自動の節として除外しない。
    #[test]
    fn test_render_message_md_contextual_prefix_is_not_auto_section() {
        let text = render_msgs(&[("user", "# contextual\n本文")]);
        assert!(text.contains("# user (1)\n\n## contextual\n本文"), "{}", text);
    }

    /// 空の発話列なら frontmatter だけ。
    #[test]
    fn test_render_message_md_empty_is_frontmatter_only() {
        let text = render_message_md("plan", 1, &[]);
        assert_eq!(text, "---\nblock_name: plan\nrun_id: 1\n---\n");
    }

    /// 401 で止まった cc の画面は、その行を理由として返す。ふつうの画面は何も返さない。
    #[test]
    fn auth_error_line_finds_401_screen() {
        let screen = "> 続けて\n\nFailed to authenticate. API Error: 401 Invalid bearer token\n";
        assert_eq!(
            auth_error_line(screen).as_deref(),
            Some("Failed to authenticate. API Error: 401 Invalid bearer token")
        );
        assert!(auth_error_line("Please run /login · API Error: 401 Invalid bearer token").is_some());
        assert!(auth_error_line("作業中です\n10 ファイルを変更").is_none());
    }

    /// pane が消えたとき、finalize の要求より前なら失敗、後なら完了の流れ（`None`）。
    #[test]
    fn pane_gone_error_depends_on_finalize_request() {
        assert_eq!(
            pane_gone_error(false),
            Some("The agent exited before finalize (the pane is gone).")
        );
        assert_eq!(pane_gone_error(true), None);
    }
}
