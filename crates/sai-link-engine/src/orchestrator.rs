//! # エージェント実行オーケストレーター
//!
//! ブロック定義ファイルのパースから herdr pane 上での pi エージェント実行、
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
//! use sai_link_engine::orchestrator::prepare_pipeline;
//!
//! let prepared = prepare_pipeline(&config)?;
//! // ユーザーに pi コマンドを表示して確認を求める
//! println!("実行コマンド:\n{}", prepared.pi_command);
//! ```
//!
//! ### 段階2: `spawn_pipeline()` — バックグラウンド実行
//!
//! ```ignore
//! use sai_link_engine::orchestrator::spawn_pipeline;
//! use sai_link_engine::herdr::SplitDirection;
//!
//! let handle = spawn_pipeline(prepared, SplitDirection::Right, 300_000, false);
//! // TUI の描画ループ内で進捗をポーリング
//! loop {
//!     let progress = handle.get_progress();
//!     match progress {
//!         sai_link_engine::orchestrator::PipelineProgress::Completed { output, .. } => { break; }
//!         sai_link_engine::orchestrator::PipelineProgress::Failed { .. } => { break; }
//!         _ => { /* 進捗表示を更新 */ }
//!     }
//! }
//! ```
//!
//! ### CLI 互換: `run_pipeline()` — 同期的な一括実行
//!
//! ```ignore
//! use sai_link_engine::orchestrator::run_pipeline;
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
use crate::pi_command::PiCommandBuilder;
use crate::placeholder::{expand_placeholders, resolve_placeholder_values, PlaceholderValue};
use crate::runner::RunnerKind;
use crate::sai_write_notify;
use crate::session::{
    create_session_dir, create_session_file, extract_output_from_dir, SessionError,
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
    /// サブタスクとして実行中かどうか（true の場合、pi プロセスに SAI_IS_SUBTASK=1 を付与）
    pub is_subtask: bool,
    /// 実行主体（`pi` / `cc`）。既定 `Pi`。
    pub runner: RunnerKind,
}

impl Default for PipelineConfig {
    fn default() -> Self {
        let block_config = BlockConfig {
            name: "default".to_string(),
            description: None,
            model: None,
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
            context: None,
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
            runner: RunnerKind::Pi,
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
    #[error("herdr 操作エラー: {0}")]
    HerdrError(#[from] HerdrError),
    #[error("pi コマンド構築エラー: {0}")]
    PiCommandError(String),
    #[error("セッションファイルエラー: {0}")]
    SessionError(String),
    #[error("{timeout_ms}ms 以内にエージェントが完了しませんでした")]
    Timeout { timeout_ms: u64 },
    #[error("ユーザーによってキャンセルされました")]
    Cancelled,
    #[error("IO エラー: {0}")]
    IoError(#[from] std::io::Error),
    #[error("システムプロンプト一時ファイルエラー: {0}")]
    SystemPromptError(String),
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
/// で判定します（メインは `/pi-sai-finalize`、サブは `task_report`。P6-2）。
///
/// `FinalizeRequested` はメインではユーザーが `/pi-sai-finalize` を入力した際、
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
    /// 完了要求（メインは `/pi-sai-finalize`、サブは `task_report`）
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
/// CLI / pi-sai が実行時にこの構造体でオーバーライドする（query3・A5）。
/// 旧 `daemon::BlockRunOverrides` / `interactive::ExecutionSettings` は本構造体に統合された（P8）。
/// `timeout` の単位は**秒**に統一。旧形式の `timeout_ms`（ミリ秒）は alias で受容するが、
/// 古い pending 通知が残る稀なケースのみに影響する移行措置。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ExecutionOverrides {
    /// モデル指定（例: "anthropic/claude-sonnet-4-5"）
    pub model: Option<String>,
    /// provider 指定（例: "anthropic"）。None なら model 解決・ブロック定義・settings.json の順
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    /// thinking budget: off / minimal / low / medium / high / xhigh
    pub thinking: Option<String>,
    /// タイムアウト（**秒**）。None ならフロントマター値または 300 秒
    #[serde(default, alias = "timeout_ms")]
    pub timeout: Option<u32>,
    /// ラウンド番号（1始まり）
    pub round: Option<i64>,
    /// グローバルラウンド番号（GR）
    pub global_round: Option<i64>,
    /// タグ
    pub tag: Option<String>,
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
    /// 実行主体（`pi` / `cc`）。未指定は `pi`（後方互換）。
    #[serde(default)]
    pub runner: RunnerKind,
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
    /// 解決後のラウンド番号（DB 保存用）
    pub round: Option<i64>,
    /// 解決後のグローバルラウンド番号（DB 保存用）
    pub global_round: Option<i64>,
    /// 解決後のタグ（DB 保存用）
    pub tag: Option<String>,
    /// デバッグ用にペインを維持するか
    pub keep_pane: bool,
    /// 解決後の実行主体（`pi` / `cc`）。
    pub runner: RunnerKind,
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
            // モデルが明示上書きされた場合、provider が未指定のときのみ
            // モデルから再解決する。task.md / block.md に明示された provider
            // （例: `open-router2`）を上書きしてはならない（過去バグ: model 接頭辞
            // "z-ai" で provider を潰し `Unknown provider "z-ai"` で起動失敗）。
            if model != "inherit"
                && (config.provider.is_none() || config.provider.as_deref() == Some(""))
            {
                if let Some(p) = crate::pi_command::resolve_provider_for_model(model) {
                    config.provider = Some(p);
                }
            }
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
                    "[sai-engine] 警告: thinking '{}' は無効です。\
                     'none', 'low', 'medium', 'high', 'xhigh', 'max' のいずれかを指定してください",
                    thinking
                );
            }
        }
    }
    // provider は model 自動解決の後に適用する（task.md 明示指定を最優先）
    if let Some(ref provider) = overrides.provider {
        if !provider.is_empty() {
            config.provider = Some(provider.clone());
        }
    }
    if let Some(timeout) = overrides.timeout {
        config.timeout = Some(timeout);
    }

    // 2. R11: `.sai/settings.json` のデフォルト値を補完
    //    ブロック定義に model/provider/thinking が無い場合、settings.json の値を参照する。
    let settings = crate::pi_command::load_settings();
    if config.model.is_none() || config.model.as_deref() == Some("") {
        if let Some(ref model) = settings.default_model {
            if !model.is_empty() {
                config.model = Some(model.clone());
            }
        }
    }
    if config.provider.is_none() || config.provider.as_deref() == Some("") {
        if let Some(ref provider) = settings.default_provider {
            if !provider.is_empty() {
                config.provider = Some(provider.clone());
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
                    "[sai-engine] 警告: settings.json の default_thinking_level '{}' は無効です。\
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
    let round = overrides.round.or(config.round.map(|v| v as i64));
    let global_round = overrides.global_round;
    let tag = overrides.tag.clone().or_else(|| config.tag.clone());

    ResolvedExecutionSettings {
        config,
        timeout_ms,
        direction,
        worktree,
        round,
        global_round,
        tag,
        keep_pane: overrides.keep_pane,
        runner: overrides.runner,
    }
}

// ============================================================================
// PreparedPipeline — 実行前の準備結果（ユーザー確認用）
// ============================================================================

/// `prepare_pipeline()` の戻り値。
///
/// pi 起動前の準備（プレースホルダー展開、セッションディレクトリ作成、
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
    /// プレースホルダー展開済みのプロンプト本文
    pub expanded_prompt: String,
    /// ブロック名
    pub block_name: String,
    /// 作業ディレクトリ
    pub cwd: Option<PathBuf>,
    /// sai-write ツールを使用するブロックかどうか
    pub uses_sai_write: bool,
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
    /// 実行主体（`pi` / `cc`）。
    pub runner: RunnerKind,
    /// プロンプト確定送信キー（pi・cc は Alt+Enter / gl は Enter）。
    pub submit_key: &'static str,
    /// エージェント側セッション ID（cc のみ。`--session-id` に渡し DB 保存する）。
    pub agent_session_id: Option<String>,
    /// エージェント終了後に同じシェルで実行するコマンド（メインブロックの自動完了・D1）。
    ///
    /// `pi_command` には混ぜない。DB に保存する起動コマンドと再開（p）の組み立てを
    /// 変えないため。DB にも保存しない。
    pub exit_command: Option<String>,
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
/// 4. pi 起動コマンド構築（`PiCommandBuilder` 使用）
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
    let waggle_db_path = crate::project::sai_path(".sai/sai-engine/sai-engine.db");
    if let Some(parent) = waggle_db_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let integrator = WaggleIntegrator::open(&waggle_db_path)
        .map_err(|e| OrchestratorError::PiCommandError(format!("waggle DB 初期化失敗: {}", e)))?;

    // HashMap<String, String> → HashMap<String, PlaceholderValue>
    let placeholder_values: HashMap<String, PlaceholderValue> = config
        .user_inputs
        .iter()
        .map(|(k, v)| (k.clone(), PlaceholderValue::from_string(v)))
        .collect();

    let (resolved_values, input_waggle_tokens) =
        resolve_placeholder_values(&placeholder_values, &integrator);

    // ── Step 1: プレースホルダー展開 ──
    let expanded_prompt = expand_placeholders(&config.block_body, &resolved_values);

    // ── Step 1.5: 範囲（context）の差し込み（`context: true` のブロックだけ） ──
    // ファイルには書かず、メモリ上で本文に入れる（並列の実行で混ざらないように）。
    // プレースホルダーの展開の後に入れるので、範囲の中の文字が展開されることはない。
    let expanded_prompt = if config.block_config.context == Some(true) {
        let text = crate::pre_run::context_text(config.run_id, config.block_config.block_id());
        insert_context(&expanded_prompt, &text)
    } else {
        expanded_prompt
    };

    // ── pi起動前の Git コミットハッシュを取得（再現性用） ──
    let git_commit_before = get_git_commit_hash();

    // ── Step 2: セッションディレクトリ作成 ──
    // outputs 統一: 実行ブロック関連の全成果物は run フォルダ
    // （`.sai/outputs/<owner>/<run_id>/`）に格納する。run_id が確定している場合は
    // `<run_dir>/session/` に作り、status 移動（todo→running→review→R）に
    // 追随させる（run フォルダごと rename されるため DB の session_dir が
    // 古くなっても `resolve_session_dir()` のフォールバックで追従できる）。
    // run フォルダが無い場合（run_id 未確定・テスト等）は旧 `.sai/sessions/` に作る。
    let session_dir = match config.run_id.map(crate::run_dir::find_run_dir) {
        // run フォルダ実在（kanban 予約 run は実行開始時に todo→running 済み）
        Some(Some((owner, run))) => {
            let sess = run.join(crate::run_dir::SESSION_SUBDIR);
            std::fs::create_dir_all(&sess)?;
            eprintln!(
                "[sai-engine] セッションディレクトリ: {} (owner={})",
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
                "[sai-engine] セッションディレクトリ: {} (owner={})",
                sess.display(),
                crate::run_dir::RUNNING_OWNER
            );
            sess
        }
        // run_id 未確定（テスト等）→ 旧 `.sai/sessions/` に作る
        None => create_session_dir(&config.block_config.block_id())?,
    };

    // ── Step 3: セッションファイル作成 ──
    let _session_path = create_session_file(&session_dir, &config.block_config.block_id())?;

    // ── Step 4+5: 起動コマンド構築（runner 分岐） ──
    let session_dir_str = session_dir.to_string_lossy().to_string();

    // エージェント側セッション ID（cc のみ。`--session-id` に渡し DB へ保存する）
    let mut agent_session_id: Option<String> = None;

    let (mut agent_command, agent_args) = match config.runner {
        RunnerKind::Pi => {
            let mut builder = PiCommandBuilder::new(&session_dir_str);
            builder = builder.with_config(config.block_config.clone());
            // 作業用コピーに `.mcp.json` が無い（未 commit）ときは、プロジェクトルートの
            // 設定を渡す。渡さないと pi は sai の MCP を 1 つも読めない（cc の --mcp-config と同じ）。
            let cwd = config
                .cwd
                .clone()
                .unwrap_or_else(crate::project::project_root);
            if let Some(mcp) = crate::cc_command::resolve_sai_mcp_config(&cwd) {
                builder = builder.with_mcp_config(mcp.to_string_lossy());
            }
            let pi_cmd = builder.build().map_err(|e| {
                OrchestratorError::PiCommandError(format!("pi コマンド構築失敗: {}", e))
            })?;
            (pi_cmd.command, pi_cmd.args)
        }
        RunnerKind::Cc => {
            let cwd = config
                .cwd
                .clone()
                .unwrap_or_else(crate::project::project_root);
            // 道具・MCP・statusLine は `sai-link init` が作る .claude/settings.json と
            // .mcp.json で制御する。無ければ sai の MCP が使えないので起動前に止める。
            // cwd がプロジェクトルートの外（別プロジェクトの cwd 等）でも、
            // sai-link 側の設定があればそれを `--mcp-config` / `--settings` で渡す。
            let config_dir = crate::cc_command::check_project_files(&cwd)
                .map_err(OrchestratorError::PiCommandError)?;
            let session_id = crate::cc_command::generate_uuid_v4();
            agent_session_id = Some(session_id.clone());
            let mut builder =
                CcCommandBuilder::new(&session_id).with_config(config.block_config.clone());
            // cwd 自身に設定が無い（プロジェクトルートから渡す）ときだけ明示指定する。
            if config_dir != cwd {
                builder = builder.with_config_dir(config_dir);
            }
            let cc_cmd = builder.build()
                .map_err(|e| {
                    OrchestratorError::PiCommandError(format!("cc コマンド構築失敗: {}", e))
                })?;
            (cc_cmd.command, cc_cmd.args)
        }
        RunnerKind::Gl => {
            // gl（sai-gemini-tui）は起動時のタスク書き込みなし。
            // 空セッションを開くだけのため、プロンプト送信は行わない。
            // `gl` ラッパー（`~/.local/bin/gl`）をそのまま起動する。
            ("gl".to_string(), Vec::new())
        }
    };

    // ── エージェント用の環境変数をコマンド先頭に付与 ──
    // エージェントプロセスは herdr ペインのシェルで起動されるため、
    // `SAI_PROJECT_ROOT`（= プロジェクトルート）と `SAI_PARENT_RUN_ID`（= 実行 ID）を
    // シェル変数として先頭に前置する。
    // - SAI_PROJECT_ROOT: pi-sai / MCP サーバが `.sai/` をプロジェクトルート基準で解決するため
    //   （worktree 実行時は cwd が `.worktrees/<branch>` に変わるため必須）
    // - SAI_PARENT_RUN_ID: task_create / task_execute / sai_write の run_id に使用
    // - SAI_BLOCK_NAME: sai-write の通知ブロック名（block_name）に使用
    // - SAI_IS_SUBTASK: サブタスク実行中の場合に "1"
    let subtask_env = if config.is_subtask {
        "SAI_IS_SUBTASK=1 "
    } else {
        ""
    };
    // デーモン稼働時（SAI_PROJECT_ROOT 設定済み）のみ付与。
    // デーモン外（テスト等）では従来どおり run_id 指定時のみ付与する。
    if let Some(project_root) = std::env::var_os("SAI_PROJECT_ROOT") {
        let root_str = project_root.to_string_lossy().to_string();
        let escaped_root = crate::herdr::shell_escape_single_quotes(&root_str);
        if let Some(run_id) = config.run_id {
            agent_command = format!(
                "SAI_PROJECT_ROOT={} SAI_PARENT_RUN_ID={} SAI_BLOCK_NAME={} {}{}",
                escaped_root,
                run_id,
                config.block_config.block_id(),
                subtask_env,
                agent_command
            );
        } else {
            agent_command =
                format!("SAI_PROJECT_ROOT={} {}{}", escaped_root, subtask_env, agent_command);
        }
    } else if let Some(run_id) = config.run_id {
        agent_command = format!(
            "SAI_PARENT_RUN_ID={} SAI_BLOCK_NAME={} {}{}",
            run_id,
            config.block_config.block_id(),
            subtask_env,
            agent_command
        );
    }

    // ── エージェント終了後の自動完了コマンド（U01・D1）──
    // pi / cc は Ctrl+D で終わるだけなので、pane のシェルが続けて
    // `sai-link agent finalize --run-id N` を実行するようにする。
    // - 対象はメインブロックの pi / cc だけ（サブブロックは task_report で完了する）
    // - 直前の `SAI_PROJECT_ROOT=…` はエージェントのプロセスにしか効かないため、
    //   このコマンドには自前で `SAI_PROJECT_ROOT` を付ける
    let exit_command = if !config.is_subtask
        && matches!(config.runner, RunnerKind::Pi | RunnerKind::Cc)
    {
        config.run_id.map(|run_id| {
            let root = std::env::var("SAI_PROJECT_ROOT")
                .ok()
                .filter(|v| !v.trim().is_empty())
                .unwrap_or_else(|| {
                    crate::project::project_root().to_string_lossy().to_string()
                });
            format!(
                "SAI_PROJECT_ROOT={} {} agent finalize --run-id {}",
                crate::herdr::shell_escape_single_quotes(&root),
                crate::herdr::shell_escape_single_quotes(&crate::cc_command::sai_link_exe()),
                run_id
            )
        })
    } else {
        None
    };

    Ok(PreparedPipeline {
        pi_command: agent_command,
        pi_args: agent_args,
        session_dir,
        expanded_prompt,
        block_name: config.block_config.block_id().to_string(),
        cwd: config.cwd.clone(),
        uses_sai_write: crate::sai_write_notify::block_uses_sai_write(&config.block_config),
        input_waggle_tokens,
        git_commit_before,
        run_id: config.run_id,
        tab_id: config.tab_id.clone(),
        base_pane_id: config.base_pane_id.clone(),
        pane_id: config.pane_id.clone(),
        task_id: None,       // U3: 後段（run_pipeline_with_watcher）で設定
        parent_run_id: None, // U3: 後段で設定
        runner: config.runner,
        submit_key: config.runner.submit_key(),
        agent_session_id,
        exit_command,
    })
}

/// 範囲（context）の文を本文に入れる（`{{context}}` の差し込み）。
///
/// `{{context}}` があれば最初の 1 つだけを `text` に置き換えます。
/// 無い場合は（利用者がブロックを直して `{{context}}` を消しても）範囲が失われないよう、
/// `text` と空行を本文の先頭に置きます。
pub fn insert_context(body: &str, text: &str) -> String {
    match body.find("{{context}}") {
        Some(pos) => {
            let mut out = String::with_capacity(body.len() + text.len());
            out.push_str(&body[..pos]);
            out.push_str(text);
            out.push_str(&body[pos + "{{context}}".len()..]);
            out
        }
        None => format!("{text}\n\n{body}"),
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
            "予期しない終了状態です".to_string(),
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
            "予期しない終了状態です".to_string(),
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
                        "[sai-engine] セッション統計: turns={}, tools={}, tokens(in={}, out={}, think={})",
                        stats.turn_count,
                        stats.tool_call_count,
                        stats.input_tokens,
                        stats.output_tokens,
                        stats.thinking_tokens
                    );
                Some(json)
            }
            Err(e) => {
                eprintln!("[sai-engine] 統計のシリアライズに失敗: {}", e);
                None
            }
        },
        Err(e) => {
            eprintln!("[sai-engine] セッション統計の抽出に失敗: {}", e);
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

/// runner に応じて出力・トランスクリプト・セッション統計を抽出する。
///
/// - pi: run セッション dir 内の pi JSONL を集計
/// - cc: `~/.claude/projects/<slug>/<uuid>.jsonl` を集計
/// - gl: sai-link 側の出力・統計は空（会話は `session::gemini_*` で読む）
fn extract_outputs_for_pipeline(prepared: &PreparedPipeline) -> (String, String, Option<String>) {
    match prepared.runner {
        RunnerKind::Cc => match claude_session_path(prepared) {
            Some(path) => {
                let output =
                    crate::session::extract_claude_final_output(&path).unwrap_or_default();
                let transcript = crate::session::extract_claude_full_transcript(&path);
                let stats = crate::session::extract_claude_session_stats(&path)
                    .ok()
                    .and_then(|s| serde_json::to_string(&s).ok());
                (output, transcript, stats)
            }
            None => (String::new(), String::new(), None),
        },
        RunnerKind::Pi => {
            let output = extract_output_from_dir(&prepared.session_dir).unwrap_or_default();
            let transcript =
                crate::session::extract_full_transcript(&prepared.session_dir).unwrap_or_default();
            let stats = extract_and_serialize_stats(&prepared.session_dir);
            (output, transcript, stats)
        }
        RunnerKind::Gl => {
            // gl は sai-gemini 側で会話するため、sai-link 側の出力・統計は空。
            // 会話内容は `session::gemini_*` で `~/.sai-gemini/sessions/` から読む。
            (String::new(), String::new(), None)
        }
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
                        "[sai-engine] ブロック間接続の挿入に失敗 ({} → {}): {}",
                        source_block_id, target_block_id, e
                    );
                }
            }
            Ok(None) => {
                // トークンの出力元が見つからない — まだトークン化されていない可能性
            }
            Err(e) => {
                eprintln!("[sai-engine] トークン '{}' の検索に失敗: {}", token, e);
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
        return Err(OrchestratorError::PiCommandError(format!(
            "ブロック '{}' はバリデーションエラーのため登録をスキップします: {}",
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
            provider: config.provider.clone(),
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
            tag: config.tag.clone(),
            round: config.round.map(|v| v as i64),
            created_at,
            updated_at: String::new(),
        }
    };

    match store.get_block_by_name(config.block_id()) {
        Ok(existing) => {
            let block_id = existing.id.ok_or_else(|| {
                OrchestratorError::PiCommandError("既存ブロックに ID がありません".to_string())
            })?;
            let updated = upsert(Some(existing));
            let _ = store.update_block(&updated);
            Ok(block_id)
        }
        Err(_) => {
            let new_block = upsert(None);
            store.insert_block(&new_block).map_err(|e| {
                OrchestratorError::PiCommandError(format!("ブロック登録に失敗: {}", e))
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
/// `reserved_run_id` が `Some` かつ親なし（メイン実行）なら予約行を確定させる。
/// サブタスク・予約なしは従来どおり新規発行する。
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
                    OrchestratorError::PiCommandError(format!(
                        "予約 run の確定に失敗: {}",
                        e
                    ))
                })?;
            // ステータスを running に更新
            store.update_execution_status(rid, "running").map_err(|e| {
                OrchestratorError::PiCommandError(format!("ステータス更新に失敗: {}", e))
            })?;
            // kanban 実行は呼び出し元が `update_execution_kanban_info` で上書きする
            let _ = store.update_execution_kanban_info(rid, None, None, "interactive");
            return Ok(PreparedExecutionInfo {
                run_id: rid,
                block_id,
            });
        }
    }

    let run_id = match parent_run_id {
        Some(pid) => {
            // 親実行からブロック ID を取得して委譲接続を作成
            let parent_exec = store.get_execution_by_id(pid).map_err(|e| {
                OrchestratorError::PiCommandError(format!("親実行の取得に失敗: {}", e))
            })?;
            let parent_block_id = parent_exec.block_id;

            let sub_run_id = store
                .create_execution_for_subtask(block_id, user_inputs_json, pid, batch_id)
                .map_err(|e| {
                    OrchestratorError::PiCommandError(format!(
                        "サブタスク実行レコード作成に失敗: {}",
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
                OrchestratorError::PiCommandError(format!("実行レコード作成に失敗: {}", e))
            })?,
    };

    // ステータスを running に更新
    store
        .update_execution_status(run_id, "running")
        .map_err(|e| OrchestratorError::PiCommandError(format!("ステータス更新に失敗: {}", e)))?;

    // 通常実行は `execution_mode='interactive'` を記録する（kanban 実行は
    // CLI 側が `update_execution_kanban_info` で `kanban` を上書きする）。
    let _ = store.update_execution_kanban_info(run_id, None, None, "interactive");

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
        .map_err(|e| OrchestratorError::PiCommandError(format!("git の実行に失敗: {}", e)))?;
    if output.status.success() {
        let s = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !s.is_empty() {
            return Ok(PathBuf::from(s));
        }
    }
    Err(OrchestratorError::PiCommandError(format!(
        "git リポジトリを解決できません: {}",
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
            OrchestratorError::PiCommandError(format!("git worktree の実行に失敗: {}", e))
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
                OrchestratorError::PiCommandError(format!("git worktree の実行に失敗: {}", e))
            })?;
        if !status2.success() {
            return Err(OrchestratorError::PiCommandError(format!(
                "worktree の作成に失敗: {}",
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
            eprintln!("[sai-engine] pane を自動削除しました: {}", pane_id);
        }
        Err(e) => {
            eprintln!(
                "[sai-engine] pane の自動削除に失敗（ベストエフォート）: {}: {}",
                pane_id, e
            );
        }
    }
}

/// メインブロックの Ctrl+D（2回目）によるユーザー明示完了（finalize）を実行する（U4・R8/R9）。
///
/// 1. `finalize_request` 通知を `pending/` に書き込む。
///    デーモンがこれを処理して実行監視スレッドへ `FinalizeRequested` を送信し、
///    `execute_completion_with_sai_write` のケース5（ユーザー要求による正常完了）で
///    `execute_block_pipeline` の Ok パス DB 更新順序（status=completed 等）を再利用する。
/// 2. 実行中 pane をベストエフォートで閉じる（pi プロセス終了。ケース4 でも正常完了になる）。
/// 3. DB を直接 `completed` + `finalized` にする（デーモン非稼働時のフォールバック。
///    デーモン側も同様に completed にするため二重更新は無害）。
///
/// これにより Ctrl+D 終了でも `sai block list` が `完了` 表示になり、
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
        "[sai-engine] Ctrl+D finalize を実行しました: run_id={}, pane={}",
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
/// `task_create` の値（1 行目だけ・frontmatter は 1 行のため）、`origin_run_id` は親の run_id。
/// 書けなくても実行は止めない。
fn write_run_task_md(
    store: &crate::db::SaiStore,
    run_id: i64,
    resolved: &ResolvedExecutionSettings,
    user_inputs: &HashMap<String, String>,
    parent_run_id: Option<i64>,
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
    let origin = parent_run_id.map_or("-".to_string(), |p| p.to_string());
    let mut fm = crate::kanban::CardFrontmatter::new_card(crate::kanban::card_creation_date(), origin);
    fm.execution_date = fm.creation_date.clone();
    fm.run_id = run_id.to_string();
    fm.runner = resolved.runner.as_str().to_string();
    fm.block = resolved.config.block_id().to_string();
    fm.model = resolved.config.model.clone().unwrap_or_default();
    fm.provider = resolved.config.provider.clone().unwrap_or_default();
    fm.thinking = resolved.config.thinking.clone().unwrap_or_default();
    fm.round = resolved.round.map(|v| v.to_string()).unwrap_or_default();
    fm.global_round = resolved.global_round.map(|v| v.to_string()).unwrap_or_default();
    fm.tag = resolved.tag.clone().unwrap_or_default();
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

    // P1-2: サブタスクが親から round/global_round/tag を引き継げるよう、
    // 実行レコード作成直後（実行前）にオーバーライド値を保存する。
    // （旧来は完了時にしか保存しておらず、親実行中に起動するサブが None を読んでいた）
    let _ = store.update_execution_overrides(
        run_id,
        resolved.tag.as_deref(),
        resolved.round,
        resolved.global_round,
        resolved.config.model.as_deref(),
        resolved.config.thinking.as_deref(),
        resolved.config.timeout.map(|v| v as i32),
    );
    // provider も保存する（`p` 再開コマンドの `--provider` 出力用・P2）
    let _ = store.update_execution_provider(run_id, resolved.config.provider.as_deref());
    // 実行主体（pi / cc）を保存する（カード frontmatter の `runner:` 由来）
    let _ = store.update_execution_runner(run_id, resolved.runner.as_str());

    // kanban 実行時は実行開始直後に run フォルダを todo→running へ移動し（R403）、
    // DB の `kanban_card_path` を保存する。旧カード（`run_id: -`）は従来の書戻しを維持。
    if let Some(card_rel) = card_path_from_request.as_deref() {
        let origin_csv = user_inputs.get("origin_run_id").map(|s| s.as_str());
        let _ = store.update_execution_kanban_info(run_id, origin_csv, Some(card_rel), "kanban");
        // 予約 run 方式：task.md は既に outputs/todo/<run_id>/ にあるため移動のみ
        if reserved_run_id.is_some() {
            let from = crate::run_dir::TODO_OWNER;
            let to = crate::run_dir::RUNNING_OWNER;
            // DB 更新済み→移動の順序（R404）。失敗時は警告に留め巻き戻さない
            // （実行自体は継続し、TUI は DB 状態で表示するため）
            if let Err(e) = crate::run_dir::move_run_dir(run_id, from, to) {
                eprintln!("[sai-engine] run フォルダ移動に失敗 ({} → {}): {}", from, to, e);
            }
        } else {
            stamp_card_run_id(card_rel, run_id);
        }
    }

    // カードを使わない実行（task_create の子・直接実行）にも、カードと同じ形の task.md を
    // run フォルダに置く。task.md の置き場と形を全経路で 1 つにし、Web / TUI の task 表示を共通にする。
    write_run_task_md(store, run_id, &resolved, &user_inputs, parent_run_id, task_id);

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
                        "[sai-engine] 作業用コピーを作成できないため指定フォルダで実行します ({}): {}",
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
        runner: resolved.runner,
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
                let _ = store.update_execution_waggle_tokens(run_id, &pr.waggle_tokens);
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
            // R51/R31: kanban 実行時は `origin_run_id` 基準で `kanban_origin` 接続を挿入し、
            // kanban 情報を DB に保存する（R30: 作成日時と来歴識別子を既存DBに保存）。
            // `origin_run_id` は実行要求の user_inputs に載って user_inputs_json に残る。
            // R121: カードパスは実パスを保存する（要求の `card_path` を使用）。
            if let Ok(exec) = store.get_execution_by_id(run_id) {
                if let Some(ref json) = exec.user_inputs_json {
                    if let Ok(map) =
                        serde_json::from_str::<std::collections::HashMap<String, String>>(json)
                    {
                        if let Some(origin_csv) = map.get("origin_run_id") {
                            let ids = parse_origin_run_ids(origin_csv);
                            insert_kanban_origin_connections(store, run_id, block_id, &ids);
                            let _ = store.update_execution_kanban_info(
                                run_id,
                                Some(origin_csv),
                                card_path_from_request.as_deref(),
                                "kanban",
                            );
                        } else if let Some(card_rel) = card_path_from_request.as_deref() {
                            // origin 無しでもカード由来ならカードパスを残す
                            let _ = store.update_execution_kanban_info(
                                run_id,
                                None,
                                Some(card_rel),
                                "kanban",
                            );
                        }
                    }
                }
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
                            eprintln!("[sai-engine] run フォルダ移動に失敗 ({} → {}): {}", cur, to, e);
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
                resolved.tag.as_deref(),
                resolved.round,
                resolved.global_round,
                resolved.config.model.as_deref(),
                resolved.config.thinking.as_deref(),
                resolved.config.timeout.map(|v| v as i32),
            );
            let _ = store.update_execution_provider(run_id, resolved.config.provider.as_deref());
            // 既存 reflection の新規作成は停止し、新 review テンプレートのみ生成する。
            // 読み取り・削除時の掃除（reflection 判定・削除）は残す。
            if let Err(e) = write_review_template(
                run_id,
                &resolved.config.name,
                &resolved.config.review_headings_or_default(),
            ) {
                eprintln!("[sai-engine] Review 記録ファイルの生成に失敗: {}", e);
            }
            // L1: 完了時に実行 usage（provider の実際の token / cost）を自動保存する。
            // ベストエフォート（セッションが無い・保存に失敗しても完了は止めない）。
            if let Err(e) = crate::usage::record_execution_usage(store, run_id) {
                eprintln!("[sai-engine] 費用の記録に失敗 (run_id={}): {}", run_id, e);
            }

            Ok(pr)
        }
        Err(e) => {
            // 7. sai-write 検出以外の終了（timeout / pane 強制削除 / エラー）→ DB 更新
            let err_msg = format!("{}", e);
            eprintln!("[sai-engine] 実行失敗 (run_id={}): {}", run_id, err_msg);
            let _ = store.update_execution_error(run_id, &err_msg, 1);
            // 失敗時もセッション統計を抽出して保存（TUI 右SB 表示用）
            // session_dir は `update_execution_session_info` で事前に保存済みだが、
            // run フォルダの owner 移動で古くなるため現行正本を解決し直す。
            if let Ok(exec) = store.get_execution_by_id(run_id) {
                // cc 実行は `~/.claude` 側の JSONL から統計を抽出する
                // （cwd は worktree を考慮しないプロジェクトルートで近似）
                let claude_json = if exec.runner == "cc" {
                    exec.agent_session_id.as_deref().and_then(|sid| {
                        crate::session::claude_session_file(&crate::project::project_root(), sid)
                    })
                } else {
                    None
                };
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
                        eprintln!("[sai-engine] 費用の記録に失敗 (run_id={}): {}", run_id, e);
                    }
                }
            }
            Err(e)
        }
    }
}

// ============================================================================
// Reflection 記録ファイル（R8 / R10 / R11 / R13）
// ============================================================================

/// Reflection 記録ファイルを置くディレクトリ（プロジェクトルート基準）。
/// ブロック 1 実行につき `<run_id>.md` の 1 ファイルを対応付ける（R10）。
pub const REFLECTION_DIR: &str = ".sai/reflection";

/// 指定実行 ID に対応する Reflection 記録ファイルの絶対パスを返す。
///
/// `.sai` 配下のパス解決は既存の `project_root` / `sai_path` に従う（D15）。
pub fn reflection_path(run_id: i64) -> PathBuf {
    crate::project::sai_path(REFLECTION_DIR).join(format!("{}.md", run_id))
}

/// ブロック実行完了後に Reflection 記録ファイルを初期テンプレートで生成する（R8 / R11）。
///
/// 将来の開発者向けの記録として、ブロック名・実行日時・実行 ID のフロントマターと
/// `## Run Reason` / `## Run Evaluation` / `## Run Next` の 3 セクションを初期状態で書き出す。
///
/// **冪等**: 既存ファイルが存在する場合は上書きせずそのまま返す。
///
/// # 引数
///
/// * `run_id` — 実行 ID（`.sai/reflection/<run_id>.md` のファイル名に使用）
/// * `block_name` — ブロック名（フロントマターの `block` に使用）
///
/// # 戻り値
///
/// 生成（または既存）の Reflection 記録ファイルのパス。
pub fn write_reflection_template(run_id: i64, block_name: &str) -> std::io::Result<PathBuf> {
    let path = reflection_path(run_id);
    if path.exists() {
        // R11: 既存ファイルは上書きしない
        return Ok(path);
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    // 実行日時（フロントマターの `date` に使用）
    let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
    // 統一書式（R14）: `r` で開く側のテンプレートと同じ書式に統一し、
    // 末尾に確認メモ用の `## Review` セクションを含める（Review 項目は空で用意する）
    let body = format!(
        "---\nblock: {}\ndate: {}\nrun_id: {}\n---\n\n## Run Reason\n\n\n## Run Evaluation\n\n\n## Run Next\n\n\n## Review\n\n",
        block_name, now, run_id
    );
    std::fs::write(&path, body)?;
    Ok(path)
}

/// 指定実行 ID に対応する run フォルダを削除する（R603）。
///
/// ブロック履歴の削除（`delete_execution_cascade`）と同時に呼び出し、
/// 対応する run フォルダ（task/review/message/session/成果物）を丸ごと消す。
/// 旧 reflection ファイルもあれば併せて消す（移行期のみ）。
/// フォルダが存在しない場合は正常に無視して続行する。
pub fn remove_run_dir_files(run_id: i64) -> std::io::Result<()> {
    let _ = crate::run_dir::remove_run_dir(run_id)?;
    let legacy = reflection_path(run_id);
    if legacy.exists() {
        std::fs::remove_file(legacy)?;
    }
    // 旧 review/message も残っていれば消す（移行漏れ掃除）
    let old_review = crate::project::sai_path(REVIEW_DIR).join(format!("{}.md", run_id));
    if old_review.exists() {
        let _ = std::fs::remove_file(old_review);
    }
    let old_message = crate::project::sai_path(MESSAGE_DIR).join(format!("{}.md", run_id));
    if old_message.exists() {
        let _ = std::fs::remove_file(old_message);
    }
    Ok(())
}

/// 指定実行 ID に対応する Reflection 記録ファイルを削除する（R13・旧式）。
///
/// outputs 統一後は [`remove_run_dir_files`] を使うこと。旧コード互換のため残す。
///
/// ブロック履歴の削除（`delete_execution_cascade`）と同時に呼び出し、
/// 対応する記録ファイルも削除する。
///
/// ファイルが存在しない場合はエラーにせず正常に無視して続行する（ファイル不在でも削除が失敗しない）。
pub fn remove_reflection_file(run_id: i64) -> std::io::Result<()> {
    let path = reflection_path(run_id);
    if !path.exists() {
        return Ok(());
    }
    std::fs::remove_file(path)
}

// ============================================================================
// Review 記録ファイル（新形式・`.sai/review/<run_id>.md`）
// ============================================================================

/// Review 記録ファイルを置くディレクトリ（旧パス・移行期フォールバック用）。
///
/// outputs 統一後の正本は `outputs/<owner>/<run_id>/review.md`。
/// 旧 `.sai/review/` は読み取り互換のみ。新規ファイルはここに作らない。
pub const REVIEW_DIR: &str = ".sai/review";

/// 指定実行 ID に対応する Review 記録ファイルの絶対パスを返す。
///
/// run フォルダ内の `review.md` を優先し、無ければ旧パスを返す（移行期互換）。
pub fn review_path(run_id: i64) -> PathBuf {
    if let Some(p) = run_review_path(run_id) {
        // run フォルダがあっても review.md 未作成の場合はその予定地を返す
        return p;
    }
    let legacy = crate::project::sai_path(REVIEW_DIR).join(format!("{}.md", run_id));
    if legacy.exists() {
        return legacy;
    }
    // 新規作成予定地：既存 owner があればそこ、無ければ review owner
    crate::run_dir::ensure_run_dir(crate::run_dir::REVIEW_OWNER, run_id)
        .map(|d| d.join(crate::run_dir::REVIEW_FILE))
        .unwrap_or(legacy)
}

/// run フォルダ内の `review.md` パスを返す（run フォルダが無ければ `None`）。
pub fn run_review_path(run_id: i64) -> Option<PathBuf> {
    crate::run_dir::find_run_dir(run_id).map(|(_, dir)| dir.join(crate::run_dir::REVIEW_FILE))
}

// ============================================================================
// message 記録ファイル（`.sai/message/<run_id>.md`）
// ============================================================================

/// message 記録ファイルを置くディレクトリ（旧パス・移行期フォールバック用）。
///
/// outputs 統一後の正本は `outputs/<owner>/<run_id>/message.md`。
/// `pi agent` session jsonl から user / assistant message のみを抽出した md を
/// 保存する。既存ファイルは上書きする（再生成で最新化）。
pub const MESSAGE_DIR: &str = ".sai/message";

/// 指定実行 ID に対応する message 記録ファイルの絶対パスを返す。
///
/// run フォルダ内の `message.md` を優先し、無ければ旧パスを返す（移行期互換）。
pub fn message_path(run_id: i64) -> PathBuf {
    if let Some((_, dir)) = crate::run_dir::find_run_dir(run_id) {
        return dir.join(crate::run_dir::MESSAGE_FILE);
    }
    crate::project::sai_path(MESSAGE_DIR).join(format!("{}.md", run_id))
}

/// user / assistant メッセージ列から message md 本文を組み立てる。
///
/// ```md
/// ---
/// block_name: test
/// run_id: 10
/// ---
///
/// ## user (1)
///
/// <本文>
///
/// ## assistant (1)
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
    for (role, text) in messages {
        if role == "user" {
            user_n += 1;
            out.push_str(&format!("\n## user ({})\n\n{}\n", user_n, text.trim()));
        } else {
            asst_n += 1;
            out.push_str(&format!("\n## assistant ({})\n\n{}\n", asst_n, text.trim()));
        }
    }
    out
}

/// 実行の session jsonl から message md を生成し `.sai/message/<run_id>.md` に保存する。
///
/// `session_dir` が空・存在しない・jsonl が無い場合は空メッセージ列として
/// frontmatter のみの md を保存する。保存したパスを返す。
pub fn write_message_file(store: &crate::db::SaiStore, run_id: i64) -> std::io::Result<PathBuf> {
    let (block_name, runner, session_dir, exec) = match store.get_execution_by_id(run_id) {
        Ok(exec) => {
            let name = store
                .get_block_by_id(exec.block_id)
                .map(|b| b.display_name)
                .unwrap_or_else(|_| format!("block#{}", exec.block_id));
            let dir = crate::session::resolve_session_dir(exec.session_dir.as_deref(), run_id)
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_default();
            (name, exec.runner.clone(), dir, Some(exec))
        }
        Err(e) => {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                e.to_string(),
            ))
        }
    };
    // runner=gl は sai-gemini の最新セッション JSONL（空でない最初の1件）から抽出する。
    let gemini_messages = if runner == "gl" {
        crate::session::gemini_session_files(30)
            .into_iter()
            .find_map(|path| {
                crate::session::extract_gemini_user_assistant_messages(&path)
                    .ok()
                    .filter(|msgs| !msgs.is_empty())
            })
    } else {
        None
    };
    // runner=cc は、その run の `agent_session_id` に対応する会話ファイル
    // （`resolve_transcript`）から抽出する。
    // `cc_session_files(...)`（`~/.claude/projects/` 全体で一番新しいもの）を使うと
    // 別の run の会話が出るため使わない。見つからないときは空にする。
    let cc_messages = match (runner == "cc", exec.as_ref()) {
        (true, Some(exec)) => crate::phone_export::resolve_transcript(
            exec,
            &crate::project::project_root(),
        )
        .and_then(|path| {
            crate::session::extract_cc_user_assistant_messages(&path)
                .ok()
                .filter(|msgs| !msgs.is_empty())
        }),
        _ => None,
    };
    let messages = if let Some(msgs) = gemini_messages {
        msgs
    } else if let Some(msgs) = cc_messages {
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

/// review.md のひな形本文を組み立てる。
///
/// 書式は frontmatter（`block` / `date` / `run_id`）＋ `### <見出し>` の並び。
/// 各見出しの後ろに空行 2 つ、最後の見出しの後ろは空行 1 つを置く。
/// `headings` が空の場合は既定（追加修正 / 追加実装）を使う。
///
/// 例（`block_name = "plan"`, `run_id = 1`, `headings = ["納得した点", "疑問に思った点", "次に知りたいこと"]`）:
/// ```text
/// ---
/// block: plan
/// date: 2026-09-25 10:00:00
/// run_id: 1
/// ---
///
/// ### 納得した点
///
///
/// ### 疑問に思った点
///
///
/// ### 次に知りたいこと
///
/// ```
pub fn review_template_text(block_name: &str, run_id: i64, headings: &[String]) -> String {
    let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
    let headings: Vec<String> = if headings.is_empty() {
        crate::block_config::default_review_headings()
    } else {
        headings.to_vec()
    };
    let sections = headings
        .iter()
        .map(|h| format!("### {}", h))
        .collect::<Vec<_>>()
        .join("\n\n\n");
    format!(
        "---\nblock: {}\ndate: {}\nrun_id: {}\n---\n\n{}\n\n",
        block_name, now, run_id, sections
    )
}

/// ブロック実行完了後に Review 記録ファイルを空テンプレートで生成する。
///
/// 見出しはブロック定義の `review_headings` から受け取る（無ければ既定の
/// `追加修正` / `追加実装`）。書式は [`review_template_text`] に集約している。
/// フロントマターは現状踏襲（`block` / `date` / `run_id`）。
///
/// **冪等**: 既存ファイルが存在する場合は上書きせずそのまま返す。
pub fn write_review_template(
    run_id: i64,
    block_name: &str,
    headings: &[String],
) -> std::io::Result<PathBuf> {
    let path = review_path(run_id);
    if path.exists() {
        return Ok(path);
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let body = review_template_text(block_name, run_id, headings);
    std::fs::write(&path, body)?;
    Ok(path)
}

/// kanban カードへ `run_id` を書き戻す（R121 二段階更新の二段階目）。
///
/// プロジェクト相対パス（`.sai/todo/xxx.md`）を絶対化し、再パースして
/// frontmatter の `run_id` のみ更新する。本文には触らない。
/// ユーザー編集中の競合時は失敗しても警告に留め、呼び出し側で継続する。
fn stamp_card_run_id(card_rel: &str, run_id: i64) {
    let path = crate::project::project_root().join(card_rel);
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) => {
            eprintln!(
                "[sai-engine] カード run_id 書戻し失敗（読込）: {}: {}",
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
                "[sai-engine] カード run_id 書戻し失敗（パース）: {}",
                path.display()
            );
            return;
        }
    };
    card.frontmatter.run_id = run_id.to_string();
    if let Err(e) = std::fs::write(&path, crate::kanban::render_card(&card)) {
        eprintln!(
            "[sai-engine] カード run_id 書戻し失敗（書込）: {}: {}",
            path.display(),
            e
        );
    }
}

/// kanban 実行時に `origin_run_id` の各 ID → 新 run_id へ
/// `kanban_origin` 接続を挿入する（Workflow の線を `origin_run_id` 基準にする）。
///
/// 既存の接続は重複挿入されない（`connection_exists` で冪等）。
pub fn insert_kanban_origin_connections(
    store: &crate::db::SaiStore,
    target_execution_id: i64,
    target_block_id: i64,
    origin_run_ids: &[i64],
) {
    for &source_exec_id in origin_run_ids {
        let Ok(source_exec) = store.get_execution_by_id(source_exec_id) else {
            continue;
        };
        let source_block_id = source_exec.block_id;
        let Ok(exists) =
            store.connection_exists(source_exec_id, target_execution_id, "kanban_origin")
        else {
            continue;
        };
        if exists {
            continue;
        }
        let conn = crate::db::BlockConnection::new_kanban_origin(
            source_exec_id,
            source_block_id,
            target_execution_id,
            target_block_id,
        );
        if let Err(e) = store.insert_connection(&conn) {
            eprintln!(
                "[sai-engine] kanban_origin 接続の挿入に失敗 ({} → {}): {}",
                source_exec_id, target_execution_id, e
            );
        }
    }
}

/// `origin_run_id` のカンマ区切り文字列を ID 配列に変換する。
///
/// 空文字・空白・数値でない要素は無視する。
pub fn parse_origin_run_ids(s: &str) -> Vec<i64> {
    s.split(',')
        .filter_map(|t| t.trim().parse::<i64>().ok())
        .collect()
}

// ============================================================================
// Public API 6: run_subtask — サブタスク実行
// ============================================================================

/// サブタスクを実行する簡易パイプライン。
///
/// ブロック定義名と入力からエージェントを起動し、結果を返します。
/// `TaskManager` のワーカー（デーモン）から呼び出され、AgentPool の管理下で実行されます。
/// 内部では共通の `execute_block_pipeline` に委譲します。
///
/// # 引数
///
/// * `store` — DB ストア
/// * `block_type` — 使用するブロック定義名（例: "sai-code-research"）
/// * `subject` — タスク件名（プロンプトとして注入）
/// * `description` — タスク詳細（プロンプトに追加）
/// * `inputs` — ブロック入力値（JSON）
/// * `parent_run_id` — 親実行 ID
/// * `worktree` — タスクの worktree 指定（"isolation" / "none" / ブランチ名）
/// * `task_cwd` — タスクの作業フォルダ（親が指定した絶対パス。None ならプロジェクトルート）
/// * `task_timeout` — タスクの時間上限（秒。親が `task_create` で指定。None ならブロック定義に従う）
/// * `batch_id` — タスクバッチ ID（P3-A。DAG のフェーズ合成に使用）
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
    batch_id: Option<String>,
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
        return Err(OrchestratorError::PiCommandError(format!(
            "ブロック定義ファイルが見つかりません: {} → {}",
            block_type,
            block_path.display()
        )));
    }

    // ブロック定義を読む直前に準備コマンドを走らせる（作り直したテンプレートを
    // そのまま差し込むため）。失敗しても前回のファイルのまま実行を続ける。
    if let crate::pre_run::PreRunOutcome::Failed(msg) = crate::pre_run::run_if_configured() {
        eprintln!(
            "[sai-engine] 警告: 実行直前の準備コマンドに失敗しました（前回のファイルのまま続けます）: {msg}"
        );
    }

    let parsed = crate::parser::parse_block_file(&block_path).map_err(|e| {
        OrchestratorError::PiCommandError(format!("ブロック定義のパースに失敗: {}", e))
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

    // 親実行から round / global_round / tag を継承（§6.2・既存実装を維持）
    let parent_exec = store.get_execution_by_id(parent_run_id).ok();
    let overrides = ExecutionOverrides {
        model: None,
        provider: None,
        thinking: None,
        // timeout はフロントマターの値を尊重する（§7.1.4 の修正点）
        // ただしタスクの timeout（task_create）があればブロック定義より優先する
        timeout: task_timeout,
        round: parent_exec.as_ref().and_then(|e| e.round),
        global_round: parent_exec.as_ref().and_then(|e| e.global_round),
        tag: parent_exec.as_ref().and_then(|e| e.tag.clone()),
        // U2・R4: タブ内で交互分割する場合は方向を明示指定（None ならデフォルト）
        direction,
        worktree,
        cwd: task_cwd,
        keep_pane: false,
        // サブタスクは pi のみ（cc からの task_* は本タスク外）
        runner: RunnerKind::Pi,
    };

    execute_block_pipeline(
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
        None, // サブタスク: カード無し
    )
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
        OrchestratorError::PiCommandError(format!("親実行 {} の取得に失敗: {}", parent_run_id, e))
    })?;
    let parent_block = store.get_block_by_id(parent_exec.block_id).map_err(|e| {
        OrchestratorError::PiCommandError(format!(
            "親ブロック {} の取得に失敗: {}",
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
        OrchestratorError::PiCommandError(format!("親ブロック定義のパースに失敗: {}", e))
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
        None => Err(OrchestratorError::PiCommandError(format!(
            "ブロック '{}' は subblocks が定義されていないため、サブブロックを呼び出せません。\n\
             ブロック定義書の subblocks: に呼び出し可能ブロックを列挙してください。",
            parent_name
        ))),
        Some(allowed) => {
            if allowed.iter().any(|b| b == block_type) {
                Ok(())
            } else {
                Err(OrchestratorError::PiCommandError(format!(
                    "ブロック '{}' は親ブロック '{}' の subblocks に含まれていないため呼び出せません。許可: {}",
                    block_type,
                    parent_name,
                    if allowed.is_empty() {
                        "(なし)".to_string()
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
/// 1. `.sai/blocks/<block_type>.md`
/// 2. `blocks/<block_type>.md`
/// 3. `lib/herdrs/blocks/<block_type>.md`
/// 4. `~/.sai-moon/blocks/<block_type>.md`（グローバル・全プロジェクトで使用可能）
pub fn resolve_block_path(block_type: &str) -> std::path::PathBuf {
    // まず .sai/blocks/ ディレクトリを検索（プロジェクトルート基準）
    let path0 = crate::project::sai_path(".sai/blocks").join(format!("{}.md", block_type));
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

    // グローバル ~/.sai-moon/blocks/ を検索（全プロジェクトで使用可能）
    if let Some(dir) = crate::project::sai_home_path("blocks") {
        let path3 = dir.join(format!("{}.md", block_type));
        if path3.exists() {
            return path3;
        }
    }

    // 見つからなければ .sai/blocks/ を返す（エラーは呼び出し元で処理）
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
    direction: SplitDirection,
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
                eprintln!("[sai-engine] {} でキャンセルされました", $label);
                *progress.lock().unwrap() = PipelineProgress::Cancelled;
                return;
            }
        };
    }

    // ---- Step 6: herdr pane 分割 ----
    check_cancel!("pane分割前");
    *progress.lock().unwrap() = PipelineProgress::SplittingPane;

    let cwd_str = prepared
        .cwd
        .as_ref()
        .map(|p| p.to_string_lossy().to_string());
    // U2・R4・案X: 動的 pane 配置は**分割済み pane を再利用**（pane_id Some）で行う。
    // task_manager がタブ内ロック下で分割を作成済みのため、ここでは分割せず取得する。
    // base_pane_id（分割対象を明示）はサブではない代替経路、tab_id / 未指定は従来方式。
    let pane_info = if let Some(pid) = prepared.pane_id.as_deref() {
        match herdr.get_pane_status(pid) {
            Ok(p) => p,
            Err(e) => {
                let msg = format!("利用予定の pane の取得に失敗 (pane={}): {}", pid, e);
                *progress.lock().unwrap() = PipelineProgress::Failed { error: msg };
                return;
            }
        }
    } else if let Some(base_pane_id) = prepared.base_pane_id.as_deref() {
        match herdr.split_pane_from(base_pane_id, direction, cwd_str.as_deref()) {
            Ok(p) => p,
            Err(e) => {
                let msg = format!("herdr 指定 pane 分割に失敗 (base={}): {}", base_pane_id, e);
                *progress.lock().unwrap() = PipelineProgress::Failed { error: msg };
                return;
            }
        }
    } else {
        match prepared.tab_id.as_deref() {
            Some(tab_id) => match herdr.split_pane_in_tab(tab_id, direction, cwd_str.as_deref()) {
                Ok(p) => p,
                Err(e) => {
                    let msg = format!("herdr タブ内 pane 分割に失敗 (tab={}): {}", tab_id, e);
                    *progress.lock().unwrap() = PipelineProgress::Failed { error: msg };
                    return;
                }
            },
            None => match herdr.split_pane(direction, cwd_str.as_deref()) {
                Ok(p) => p,
                Err(e) => {
                    let msg = format!("herdr pane 分割に失敗: {}", e);
                    *progress.lock().unwrap() = PipelineProgress::Failed { error: msg };
                    return;
                }
            },
        }
    };

    // ---- ペインにラベルを設定（ユーザーが herdr サイドバーで識別可能にする） ----
    if let Some(run_id) = prepared.run_id {
        let label = format!("sai-{}-{}", prepared.block_name, run_id);
        if let Err(e) = herdr.rename_pane(&pane_info.pane_id, &label) {
            eprintln!("[sai-engine] ペインラベル設定に失敗: {}", e);
        } else {
            eprintln!("[sai-engine] ペインラベルを設定: {}", label);
        }
    }
    // sai-phone 書き出し用に pane_id を run セッション dir へ控える。
    // DB の herdr_pane_id は完了時にしか入らず，ラベルはプロジェクト間で重なるため。
    // DB は task_manager の分岐に使われるので触らない。失敗しても実行は続ける。
    let _ = std::fs::write(
        prepared.session_dir.join(crate::phone_export::PANE_ID_FILE),
        &pane_info.pane_id,
    );

    // ---- Step 7: シェル初期化待機 (3秒、中断可能) ----
    interruptible_sleep(3, "シェル初期化", &progress, &cancel_flag);
    check_cancel!("シェル初期化後");

    // ---- Step 8: pi 起動 ----
    // 起動コマンドの先頭で必ず作業ディレクトリへ `cd` する。
    // pane を先行作成している場合（task_* の動的配置）、分割元の cwd を引き継いでいるため、
    // そのまま起動するとプロジェクトと無関係な場所で子エージェントが始まる
    // （本体フォルダで作業してしまう・別の AGENTS.md を読む）ことになる。
    let launch_command = match cwd_str.as_deref() {
        Some(cwd) => format!(
            "cd {} && {}",
            crate::herdr::shell_escape_single_quotes(cwd),
            prepared.pi_command
        ),
        None => prepared.pi_command.clone(),
    };
    // エージェント終了後に同じシェルで完了処理を出す（U01・D1）。
    // `;` で繋ぐので、エージェントがどう終わっても実行される。
    let launch_command = match prepared.exit_command.as_deref() {
        Some(exit_command) => format!("{} ; {}", launch_command, exit_command),
        None => launch_command,
    };
    if let Err(e) = herdr.run_in_pane(&pane_info.pane_id, &launch_command) {
        let msg = format!("pi 起動に失敗: {}", e);
        *progress.lock().unwrap() = PipelineProgress::Failed { error: msg };
        return;
    }

    // ---- Step 9: pi 初期化待機 (5秒、中断可能) ----
    interruptible_sleep(5, "pi初期化", &progress, &cancel_flag);
    check_cancel!("pi初期化後");

    // ---- Step 10: pi アイドル状態確認 (30秒タイムアウト、中断可能) ----
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
            *progress.lock().unwrap() = PipelineProgress::Failed {
                error: e.to_string(),
            };
            return;
        }
    }

    // ---- Step 13-15: 完了待機 → 出力抽出 → クリーンアップ ----
    // 完了判定は「pi idle/done」ではなく「自分の run_id の sai-write 検出」（D4）。
    // `execute_completion_normal` は廃止し、全ブロックを sai-write 方式に統一する（§7.1.3・D22）。
    execute_completion_with_sai_write(
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

/// 長時間アラートの間隔（既定 20 分。`SAI_LONG_RUN_ALERT_MS` で上書き可能）。
fn long_run_alert_ms() -> u64 {
    std::env::var("SAI_LONG_RUN_ALERT_MS")
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .filter(|v| *v > 0)
        .unwrap_or(1_200_000)
}

/// `long_running` 通知の本文。
fn long_running_message(elapsed: Duration, pane_id: &str) -> String {
    format!(
        "サブブロックが {} 分以上 working のままです（pane={}）。task_capture で画面を確かめてください。進んでいれば task_output で待ち続けてかまいません。1 つの命令が終わらずに止まっているなら task_manage interrupt で止め、task_manage send で原因を調べさせてください。",
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
        "サブブロックが timeout（{} 秒）を超えても完了していません（経過 {} 分、pane={}）。task_capture で画面を確かめてください。時間がかかる理由がもっともなら、そのまま task_output で待ってください（同じ間隔でまた知らせます）。止めるなら task_manage interrupt、終了させるなら task_manage delete を使ってください。",
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
fn execute_completion_with_sai_write(
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
    let waggle_db_path = crate::project::sai_path(".sai/sai-engine/sai-engine.db");
    if let Some(parent) = waggle_db_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let integrator = match WaggleIntegrator::open(&waggle_db_path) {
        Ok(i) => i,
        Err(e) => {
            *progress.lock().unwrap() = PipelineProgress::Failed {
                error: format!("waggle DB の初期化に失敗: {}", e),
            };

            return;
        }
    };

    // sai-write 通知ディレクトリを事前作成
    let _ = sai_write_notify::ensure_pending_dir();
    let _ = sai_write_notify::ensure_processed_dir();

    let run_start = Instant::now();
    // timeout 超過時、子ブロックは失敗にせず通知を繰り返し送るので deadline を延ばす
    let mut deadline = Instant::now() + Duration::from_millis(timeout_ms);
    let mut waggle_tokens: Vec<String> = Vec::new();
    // pane 消滅判定の連続エラーカウント（一時的な herdr エラーへの耐性）
    let mut pane_error_count: u32 = 0;

    // U3: サブブロック停止検知の閾値（既定 60 秒。`SAI_IDLE_TIMEOUT_MS` で上書き可能）
    let idle_timeout_ms = std::env::var("SAI_IDLE_TIMEOUT_MS")
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .filter(|v| *v > 0)
        .unwrap_or(60_000);
    // U2: 長時間アラートの間隔（既定 20 分。`SAI_LONG_RUN_ALERT_MS` で上書き可能）
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
                        "[sai-engine] サブブロック timeout 超過 (task_id={}, reason=timeout): {}",
                        tid, msg
                    );
                    let _ = crate::task_manager::write_stalled_notification(
                        tid,
                        Some(pid),
                        "timeout",
                        &msg,
                    );
                    deadline = Instant::now() + Duration::from_millis(timeout_ms);
                } else {
                    *progress.lock().unwrap() = PipelineProgress::Failed {
                        error: format!("タイムアウト: {}ms 以内にファイナライズ（/pi-sai-finalize または task_manage(finalize)）による完了を検出できませんでした。以降はユーザーが手動で対応してください。", timeout_ms),
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
                        eprintln!("[sai-engine] sai-write 検出（デーモン）: token={}", token);
                    }
                    // P6-2: サブブロックも task_report（FinalizeRequested）で完了する。
                    // sai-write はトークンの収集だけを行い、ここでは完了しない。
                    // - メインブロック（親実行なし）: ユーザーが任意のタイミングで
                    //   `/pi-sai-finalize` を実行して最終化するまで pi エージェントは継続する。
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
                                            "[sai-engine] sai-write 検出: run_batches を finalized に更新 (parent={})",
                                            run_id
                                        ),
                                        Ok(false) => {}
                                        Err(e) => eprintln!(
                                            "[sai-engine] run_batches finalized 更新失敗 (parent={}): {}",
                                            run_id, e
                                        ),
                                    }
                                }
                                Err(e) => eprintln!(
                                    "[sai-engine] DB オープン失敗のため run_batches を finalized にできません: {}",
                                    e
                                ),
                            }
                        }
                    }
                }
                Ok(CompletionEvent::FinalizeRequested { run_id }) => {
                    eprintln!(
                        "[sai-engine] ファイナライズ要求検出 (run_id={}): メインは /pi-sai-finalize、サブは task_report による完了要求です",
                        run_id
                    );
                    // P6-2: メイン（/pi-sai-finalize）もサブ（task_report）もこの分岐で完了する。
                    // チャネルに残っている SaiWriteDetected イベントをすべて消費する
                    loop {
                        match rx.try_recv() {
                            Ok(CompletionEvent::SaiWriteDetected { waggle_token, .. }) => {
                                if let Some(ref token) = waggle_token {
                                    waggle_tokens.push(token.clone());
                                    eprintln!("[sai-engine] ファイナライズ時: sai-write トークンを収集: token={}", token);
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
                        "[sai-engine] デーモン停止を検出 (run_id={}): 残留出力を回収して確定します",
                        run_id
                    );
                    loop {
                        match rx.try_recv() {
                            Ok(CompletionEvent::SaiWriteDetected { waggle_token, .. }) => {
                                if let Some(ref token) = waggle_token {
                                    waggle_tokens.push(token.clone());
                                    eprintln!("[sai-engine] 停止時: sai-write トークンを収集: token={}", token);
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
                            error: "デーモン停止により中断しました（出力なし）".to_string(),
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
                                                "サブブロックが {} 秒以上 idle のまま停止しています（pane={}）。task_capture で画面を確かめ、task_manage send で指示を送って再開させてください（blocked / error なら先に task_manage interrupt）。",
                                                idle_timeout_ms / 1000,
                                                pane_id
                                            ),
                                        ),
                                        StallReason::Blocked => (
                                            "blocked",
                                            format!(
                                                "サブブロックが blocked 状態で停止しています（pane={}）。task_capture で画面を確かめ、task_manage send で指示を送って再開させてください（blocked / error なら先に task_manage interrupt）。",
                                                pane_id
                                            ),
                                        ),
                                        StallReason::Error => (
                                            "error",
                                            format!(
                                                "サブブロックが error 状態で停止しています（pane={}）。task_capture で画面を確かめ、task_manage send で指示を送って再開させてください（blocked / error なら先に task_manage interrupt）。",
                                                pane_id
                                            ),
                                        ),
                                        StallReason::LongRunning(elapsed) => (
                                            "long_running",
                                            long_running_message(elapsed, pane_id),
                                        ),
                                    };
                                    eprintln!(
                                        "[sai-engine] サブブロック停止検知 (task_id={}, reason={}): {}",
                                        tid, why, msg
                                    );
                                    let _ = crate::task_manager::write_stalled_notification(
                                        tid,
                                        stalled_parent_run_id,
                                        why,
                                        &msg,
                                    );
                                }
                            }
                        }
                        Err(_) => {
                            pane_error_count += 1;
                            if pane_error_count >= 3 {
                                eprintln!(
                                    "[sai-engine] ペインが削除されました (pane={}): 利用可能な出力を抽出して完了します",
                                    pane_id
                                );
                                break; // 抽出処理へ（ケース4: 正常完了扱い）
                            }
                        }
                    }
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    *progress.lock().unwrap() = PipelineProgress::Failed {
                        error: "デーモンとの接続が切断されました（sai-write 完了判定不能）"
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
                match sai_write_notify::poll_and_process_all(&integrator) {
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
                    error: format!("{}ms 以内にエージェントが完了しませんでした", timeout_ms),
                };

                return;
            }

            // 進捗を更新
            let running_secs = run_start.elapsed().as_secs();
            *progress.lock().unwrap() = PipelineProgress::Running {
                elapsed_secs: running_secs,
            };

            // --- 1. pi の状態をチェック ---
            let pi_done = match herdr.get_pane_status(pane_id) {
                Ok(info) => {
                    if info.agent_status == "blocked" {
                        // blocked 検出 → 残りの通知を処理してからキャンセル
                        match sai_write_notify::poll_and_process_all(&integrator) {
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
            match sai_write_notify::poll_and_process_one(&integrator) {
                Ok(Some(result)) => {
                    if let Some(ref token) = result.waggle_token {
                        waggle_tokens.push(token.clone());
                        eprintln!("[sai-engine] sai-write 通知を処理: token={}", token);
                    }
                    if let Some(ref err) = result.error {
                        eprintln!("[sai-engine] sai-write 通知処理エラー: {}", err);
                    }
                }
                Err(e) => {
                    eprintln!("[sai-engine] 通知ポーリングエラー: {}", e);
                }
                _ => {}
            }

            // --- pi が完了したら残りの通知を一括処理して終了 ---
            if pi_done {
                match sai_write_notify::poll_and_process_all(&integrator) {
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
    // runner に応じて出力・トランスクリプト・統計を抽出する（pi / cc）
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
/// * `submit_key` — 入力確定キー（`RunnerKind::submit_key()`。pi・cc は `Alt+Enter`、gl は `Enter`）
/// * `task_id` — 長時間アラートを `results/` に書き込むためのタスク ID（`None` なら書かない）
/// * `parent_run_id` — 通知に添える親実行 ID
pub fn send_turn_to_pane(
    herdr: &HerdrClient,
    pane_id: &str,
    message: &str,
    timeout_ms: u64,
    session_dir: &Path,
    submit_key: &str,
    task_id: Option<i64>,
    parent_run_id: Option<i64>,
) -> Result<TurnResult, OrchestratorError> {
    let start = Instant::now();

    // 1. pane の生存確認（cwd は worktree の場合があるためコミット取得に利用）
    let pane_cwd = match herdr.get_pane_status(pane_id) {
        Ok(info) => info.cwd.clone(),
        Err(e) => {
            return Err(OrchestratorError::PiCommandError(format!(
                "ターン送信先の pane が利用できません: {} ({})",
                pane_id, e
            )))
        }
    };

    // 2. プロンプト送信（送信確認 + 再送つき、F6。キャンセルフラグはこの経路では未使用）
    submit_prompt_verified(herdr, pane_id, message, submit_key, None)?;

    // 5. 完了待機（idle/done で完了、blocked でキャンセル、タイムアウトで失敗）
    let waggle_db_path = crate::project::sai_path(".sai/sai-engine/sai-engine.db");
    if let Some(parent) = waggle_db_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let integrator = WaggleIntegrator::open(&waggle_db_path).map_err(|e| {
        OrchestratorError::PiCommandError(format!("waggle DB の初期化に失敗: {}", e))
    })?;
    let _ = sai_write_notify::ensure_pending_dir();
    let _ = sai_write_notify::ensure_processed_dir();

    // 子ブロックは timeout を超えても失敗にせず、stalled(timeout) を繰り返し送る
    let mut deadline = Instant::now() + Duration::from_millis(timeout_ms);
    let mut waggle_tokens: Vec<String> = Vec::new();
    let mut pane_error_count: u32 = 0;
    // U2: ターン送信中も長時間アラートを親へ知らせる（間隔は `SAI_LONG_RUN_ALERT_MS`）
    let long_run_ms = long_run_alert_ms();
    let mut stall = StallDetector::default();

    loop {
        // タイムアウトチェック
        if Instant::now() >= deadline {
            // 子ブロックは failed にせず、stalled(timeout) を送って deadline を延ばす
            if let Some(tid) = task_id {
                let msg = timeout_alert_message(timeout_ms, start.elapsed(), pane_id);
                eprintln!(
                    "[sai-engine] サブブロック timeout 超過 (task_id={}, reason=timeout): {}",
                    tid, msg
                );
                let _ = crate::task_manager::write_stalled_notification(
                    tid,
                    parent_run_id,
                    "timeout",
                    &msg,
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
                            "[sai-engine] サブブロック長時間アラート (task_id={}, reason=long_running): {}",
                            tid, msg
                        );
                        let _ = crate::task_manager::write_stalled_notification(
                            tid,
                            parent_run_id,
                            "long_running",
                            &msg,
                        );
                    }
                }
            }
            Err(_) => {
                pane_error_count += 1;
                if pane_error_count >= 3 {
                    return Err(OrchestratorError::PiCommandError(
                        "ターン実行中に pane が消滅しました".to_string(),
                    ));
                }
            }
        }

        // sai-write 通知の収集（ターン中にファイル保存があればトークン化）
        if let Ok(Some(result)) = sai_write_notify::poll_and_process_one(&integrator) {
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
/// * `submit_key` — 確定キー（`RunnerKind::submit_key()`。pi・cc とも `Alt+Enter`。
///   `~/.claude/keybindings.json` の設定に従う）
/// * `cancel_flag` — キャンセルフラグ（無い場合は `None`）
///
/// # 戻り値
///
/// * `Ok(())` — `agent_status` が `working` になった
/// * `Err(OrchestratorError::Cancelled)` — `blocked` になった、またはキャンセルされた
/// * `Err(OrchestratorError::PiCommandError)` — 送信自体に失敗、または 3 回試しても working になりません
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
        .map_err(|e| OrchestratorError::PiCommandError(format!("テキスト送信に失敗: {}", e)))?;

    for attempt in 1..=MAX_ATTEMPTS {
        if cancelled(cancel_flag) {
            return Err(OrchestratorError::Cancelled);
        }

        // 2. 入力欄が整うのを待ってから確定キーを送る
        std::thread::sleep(Duration::from_millis(300));
        herdr.send_keys(pane_id, submit_key).map_err(|e| {
            OrchestratorError::PiCommandError(format!("確定キー送信に失敗: {}", e))
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
            "[sai-engine] プロンプトが送信されていない可能性があります（{} 回目）。確定キー {} を再送します",
            attempt, submit_key
        );
    }

    Err(OrchestratorError::PiCommandError(format!(
        "プロンプトを送信できませんでした（確定キー {} を {} 回送っても working になりません）",
        submit_key, MAX_ATTEMPTS
    )))
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
    use crate::block_config::default_review_headings;
    use std::collections::HashMap;

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
                context: None,
                id: None,
            },
            block_body: String::new(),
            user_inputs: HashMap::new(),
            cwd: None,
            direction: SplitDirection::Right,
            timeout_ms: 5_000,
            keep_pane: false,
            run_id: None,
            tab_id: None,
            base_pane_id: None,
            pane_id: None,
            is_subtask: false,
            runner: RunnerKind::Pi,
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
        // 環境変数（SAI_PROJECT_ROOT）を書き換えるテストと直列化する（U4）
        let _env_lock = finalize_env_lock();
        let config = minimal_config();
        let prepared = prepare_pipeline(&config).expect("prepare_pipeline に失敗");

        assert_eq!(prepared.block_name, "test-block");
        assert!(prepared.pi_command.starts_with("pi "));
        assert!(prepared.pi_command.contains("--session-dir"));
        assert!(prepared
            .session_dir
            .to_string_lossy()
            .contains("pi-sessions-"));
        assert!(prepared.expanded_prompt.is_empty()); // block_body が空なので

        // クリーンアップ（テストではセッションディレクトリを削除）
        crate::session::cleanup_session_dir(&prepared.session_dir);
    }

    /// run_id が設定されている場合、pi-sai 用の環境変数
    /// （SAI_PARENT_RUN_ID / SAI_BLOCK_NAME）がコマンド先頭に付与される。
    #[test]
    fn test_prepare_pipeline_adds_pi_sai_env_prefix() {
        // 環境変数の影響を受けないよう SAI_PROJECT_ROOT を書き換えるテストと直列化する（U4）
        let _env_lock = finalize_env_lock();
        // 親プロセスから SAI_PROJECT_ROOT が引き継がれていても、未設定時の挙動を検証する
        let _project_root_guard = EnvVarGuard::remove("SAI_PROJECT_ROOT");
        let mut config = minimal_config();
        config.run_id = Some(1234);
        config.block_config.name = "sai-test-main".to_string();
        let prepared = prepare_pipeline(&config).expect("prepare_pipeline に失敗");

        assert!(
            prepared
                .pi_command
                .starts_with("SAI_PARENT_RUN_ID=1234 SAI_BLOCK_NAME=sai-test-main pi "),
            "環境変数プレフィックスが付与されるべき: {}",
            prepared.pi_command
        );
        assert!(prepared.pi_command.contains("--session-dir"));

        // クリーンアップ
        crate::session::cleanup_session_dir(&prepared.session_dir);
    }

    /// メインブロックの pi / cc には、エージェント終了後に実行する
    /// `sai-link agent finalize --run-id N` が `exit_command` に入る（U01・D1）。
    /// `pi_command`（DB に保存する起動コマンド）には混ぜない。
    #[test]
    fn test_prepare_pipeline_sets_exit_command_for_main_block() {
        let _env_lock = finalize_env_lock();
        let _project_root_guard = EnvVarGuard::remove("SAI_PROJECT_ROOT");
        let mut config = minimal_config();
        config.run_id = Some(1234);
        config.block_config.name = "sai-test-main".to_string();
        let prepared = prepare_pipeline(&config).expect("prepare_pipeline に失敗");

        let exit = prepared
            .exit_command
            .clone()
            .expect("メインブロックの pi には exit_command が付くべき");
        assert!(
            exit.contains("agent finalize --run-id 1234"),
            "exit_command に agent finalize が入るべき: {}",
            exit
        );
        assert!(
            !prepared.pi_command.contains("agent finalize"),
            "pi_command に agent finalize を混ぜない: {}",
            prepared.pi_command
        );
        assert!(
            exit.starts_with("SAI_PROJECT_ROOT="),
            "exit_command にも SAI_PROJECT_ROOT が付くべき: {}",
            exit
        );

        crate::session::cleanup_session_dir(&prepared.session_dir);
    }

    /// サブブロックと gl には exit_command を付けない（U01）。
    /// サブブロックは今までどおり `task_report` で完了する。
    #[test]
    fn test_prepare_pipeline_no_exit_command_for_subtask_and_gl() {
        let _env_lock = finalize_env_lock();
        let _project_root_guard = EnvVarGuard::remove("SAI_PROJECT_ROOT");

        let mut subtask = minimal_config();
        subtask.run_id = Some(1234);
        subtask.is_subtask = true;
        let prepared = prepare_pipeline(&subtask).expect("prepare_pipeline に失敗");
        assert!(
            prepared.exit_command.is_none(),
            "サブブロックには付けない: {:?}",
            prepared.exit_command
        );
        crate::session::cleanup_session_dir(&prepared.session_dir);

        let mut gl = minimal_config();
        gl.run_id = Some(1234);
        gl.runner = RunnerKind::Gl;
        let prepared = prepare_pipeline(&gl).expect("prepare_pipeline に失敗");
        assert!(
            prepared.exit_command.is_none(),
            "gl には付けない: {:?}",
            prepared.exit_command
        );
        crate::session::cleanup_session_dir(&prepared.session_dir);
    }

    /// `runner=cc` かつ cwd がプロジェクトルートの外のとき、sai-link 側の設定
    /// （`.mcp.json` / `.claude/settings.json`）が `--mcp-config` /
    /// `--settings` で渡り、設定チェックで止まらないこと。
    #[test]
    fn test_prepare_pipeline_cc_passes_project_config_for_external_cwd() {
        let _env_lock = finalize_env_lock();
        let root = finalize_test_temp_dir("cc-external-root");
        std::fs::create_dir_all(root.join(".claude")).unwrap();
        std::fs::write(root.join(".claude/settings.json"), "{}").unwrap();
        std::fs::write(
            root.join(".mcp.json"),
            crate::cc_command::mcp_config_json().to_string(),
        )
        .unwrap();
        let _env_guard = EnvVarGuard::set("SAI_PROJECT_ROOT", root.as_os_str());

        // 設定を持たない別フォルダ（プロジェクト外相当）を作業フォルダにする
        let external = finalize_test_temp_dir("cc-external-cwd");
        let mut config = minimal_config();
        config.runner = RunnerKind::Cc;
        config.cwd = Some(external);
        config.run_id = Some(4321);

        let prepared = prepare_pipeline(&config).expect("prepare_pipeline に失敗");
        let mcp = root.join(".mcp.json");
        assert!(
            prepared
                .pi_command
                .contains(&format!("--mcp-config '{}'", mcp.display())),
            "--mcp-config が sai-link 側のパスで渡るべき: {}",
            prepared.pi_command
        );
        let settings = root.join(".claude/settings.json");
        assert!(
            prepared
                .pi_command
                .contains(&format!("--settings '{}'", settings.display())),
            "--settings が sai-link 側のパスで渡るべき: {}",
            prepared.pi_command
        );

        crate::session::cleanup_session_dir(&prepared.session_dir);
    }

    #[test]
    fn test_prepare_pipeline_with_body() {
        let mut config = minimal_config();
        config.block_body = "# テスト\nプロジェクト: {{input.name}}".to_string();

        let prepared = prepare_pipeline(&config).expect("prepare_pipeline に失敗");

        // 本文がプレースホルダー展開されている
        assert!(prepared.expanded_prompt.contains("# テスト"));
        assert!(prepared.expanded_prompt.contains("{{input.name}}")); // 値未指定なので残る

        crate::session::cleanup_session_dir(&prepared.session_dir);
    }

    #[test]
    fn test_prepare_pipeline_with_user_inputs() {
        let mut config = minimal_config();
        config.block_body = "プロジェクト: {{input.name}}".to_string();
        let mut inputs = HashMap::new();
        inputs.insert("name".to_string(), "MyApp".to_string());
        config.user_inputs = inputs;

        let prepared = prepare_pipeline(&config).expect("prepare_pipeline に失敗");

        assert!(prepared.expanded_prompt.contains("MyApp"));
        assert!(!prepared.expanded_prompt.contains("{{input.name}}"));

        crate::session::cleanup_session_dir(&prepared.session_dir);
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
        assert!(msg.contains("以内に"));

        let err_cancelled = OrchestratorError::Cancelled;
        let msg = format!("{}", err_cancelled);
        assert!(msg.contains("キャンセル"));

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
        assert!(msg.contains("セッションファイル"));
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
            provider: None,
            thinking: None,
            tools: None,
            extensions: None,
            skills: None,
            subblocks: None,
                timeout: None,
            worktree: worktree.map(str::to_string),
            enable: None,
            category: None,
            tag: None,
            round: None,
            templates: None,
            review_headings: None,
            context: None,
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
            provider: None,
            thinking: None,
            tools: None,
            extensions: None,
            skills: None,
            subblocks: None,
                timeout: Some(600),
            worktree: None,
            enable: None,
            category: None,
            tag: None,
            round: None,
            templates: None,
            review_headings: None,
            context: None,
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
            provider: None,
            thinking: Some("low".to_string()),
            tools: None,
            extensions: None,
            skills: None,
            subblocks: None,
            timeout: Some(300),
            worktree: Some("none".to_string()),
            enable: None,
            category: None,
            tag: Some("brst".to_string()),
            round: Some(2),
            templates: None,
            review_headings: None,
            context: None,
            id: None,
        };
        let overrides = ExecutionOverrides {
            model: Some("anthropic/claude-sonnet-4-5".to_string()),
            provider: None,
            thinking: Some("high".to_string()),
            timeout: Some(900),
            round: Some(3),
            global_round: Some(1),
            tag: Some("plan".to_string()),
            direction: Some(SplitDirection::Down),
            worktree: Some("isolation".to_string()),
            cwd: None,
            keep_pane: false,
            runner: RunnerKind::Pi,
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
        assert_eq!(resolved.round, Some(3));
        assert_eq!(resolved.global_round, Some(1));
        assert_eq!(resolved.tag.as_deref(), Some("plan"));
    }

    /// モデルを明示上書きしても、block.md / task.md に明示された provider は保持される。
    ///
    /// 過去バグ: model オーバーライド時に接頭辞（`z-ai/glm-5.3-flash` → `z-ai`）を
    /// provider と断定して明示 provider を上書きし、pi が
    /// `Unknown provider "z-ai"` で起動失敗していた。モデル ID 自体が `/` を
    /// 含む OpenRouter 系 ID では接頭辞解決は不正確なため、明示 provider を最優先とする。
    #[test]
    fn test_resolve_execution_settings_model_override_keeps_provider() {
        // models.json 解決が SAI_PROJECT_ROOT に依存するため、環境変数を触るテストと直列化する（U4）
        let _env_lock = finalize_env_lock();
        // ブロック定義: model=z-ai/glm-5.3-flash + provider=open-router2（実ブロック sai.md と同形）
        let config = BlockConfig {
            name: "test".to_string(),
            description: None,
            model: Some("z-ai/glm-5.3-flash".to_string()),
            provider: Some("open-router2".to_string()),
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
            context: None,
            id: None,
        };

        // task.md 経由で model が上書きされるケース（provider はオーバーライドなし）
        let overrides = ExecutionOverrides {
            model: Some("z-ai/glm-5.3-flash".to_string()),
            ..ExecutionOverrides::default()
        };
        let resolved = resolve_execution_settings(&config, &overrides);

        assert_eq!(
            resolved.config.model.as_deref(),
            Some("z-ai/glm-5.3-flash")
        );
        // 明示 provider は model からの再解決で上書きされない
        // （pi へは `--provider open-router2 --model z-ai/glm-5.3-flash` として渡る）
        assert_eq!(
            resolved.config.provider.as_deref(),
            Some("open-router2"),
            "明示 provider は model オーバーライド時も保持されるべき"
        );
    }

    /// task.md の `provider: zzz` オーバーライドは最優先で `--provider zzz` に
    /// 反映される（model 接頭辞を provider と誤解釈しない・P2）。
    #[test]
    fn test_resolve_execution_settings_task_provider_wins_over_model_prefix() {
        // models.json 解決が SAI_PROJECT_ROOT に依存するため、環境変数を触るテストと直列化する（U4）
        let _env_lock = finalize_env_lock();
        let config = BlockConfig {
            name: "test".to_string(),
            description: None,
            model: None,
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
            context: None,
            id: None,
        };
        // task.md: `model: z-ai/glm-5.3-flash` + `provider: open-router2`
        let overrides = ExecutionOverrides {
            model: Some("z-ai/glm-5.3-flash".to_string()),
            provider: Some("open-router2".to_string()),
            ..ExecutionOverrides::default()
        };
        let resolved = resolve_execution_settings(&config, &overrides);

        assert_eq!(resolved.config.model.as_deref(), Some("z-ai/glm-5.3-flash"));
        assert_eq!(
            resolved.config.provider.as_deref(),
            Some("open-router2"),
            "task.md の provider は最優先で反映されるべき"
        );

        // PiCommandBuilder で `--provider open-router2` と `--model z-ai/glm-5.3-flash`
        // が別フラグで出力されること（接頭辞が provider に使われない）
        let cmd = crate::pi_command::PiCommandBuilder::new("/tmp/session")
            .with_config(resolved.config)
            .build()
            .unwrap();
        let c = &cmd.command;
        assert!(
            c.contains("--provider open-router2") && c.contains("--model z-ai/glm-5.3-flash"),
            "task.md の provider / model が別フラグで反映される: {c}"
        );
        assert!(
            !c.contains("--provider z-ai"),
            "model 接頭辞が provider に使われてはならない: {c}"
        );
    }

    #[test]
    fn test_resolve_execution_settings_engine_default_timeout() {
        // フロントマター・オーバーライドとも未指定 → 実質無期限（NO_TIMEOUT_MS）
        let config = BlockConfig {
            name: "test".to_string(),
            description: None,
            model: None,
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
            context: None,
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
        // 空リスト → すべて拒否（"(なし)" 表示）
        let allowed = Some(Vec::new());
        let err = is_subblock_allowed("parent", &allowed, "x").expect_err("空リストは全拒否");
        assert!(format!("{}", err).contains("(なし)"));
    }

    // ---------------------------------------------------------------------
    // P1-2 / P3-A: サブ実行レコードへの round / global_round / batch_id 保存
    // ---------------------------------------------------------------------

    /// P1-2 / P3-A の回帰テスト。
    ///
    /// メイン実行開始時に round / global_round が保存され（P1-2 の早期保存）、
    /// サブ実行レコード作成時に batch_id が保存される（P3-A）ことを検証する。
    /// 旧コード（修正前）では、サブ実行の round / global_round / batch_id が
    /// すべて NULL になるため、このテストが失敗する。
    #[test]
    fn test_subtask_record_inherits_round_global_round_and_batch_id() {
        use crate::db::models::test_helpers::create_test_block;
        use crate::db::SaiStore;

        let store = SaiStore::open_in_memory().unwrap();
        // ブロック定義を登録（ファイルパスは存在しなくても upsert は動作する）
        let _block_id = store.insert_block(&create_test_block("test-sub")).unwrap();

        // 親（メイン）実行を作成し、実行開始時に round / global_round / tag を保存
        // （P1-2: execute_block_pipeline が実行レコード作成直後に保存するのと同じ操作）
        let parent_block_id = store
            .insert_block(&create_test_block("test-parent"))
            .unwrap();
        let parent_run_id = store.create_execution(parent_block_id, None).unwrap();
        store
            .update_execution_overrides(
                parent_run_id,
                Some("test"),
                // tag
                Some(1),
                // round
                Some(5),
                // global_round
                None,
                None,
                None
            )
            .unwrap();

        // サブ実行レコード作成（P3-A: batch_id を渡す。run_subtask →
        // execute_block_pipeline → prepare_execution_record の流れを再現）
        let sub_config = BlockConfig {
            name: "test-sub".to_string(),
            description: None,
            model: Some("inherit".to_string()),
            provider: None,
            thinking: None,
            tools: None,
            extensions: None,
            skills: None,
            subblocks: None,
                timeout: Some(300),
            worktree: None,
            enable: None,
            category: None,
            tag: None,
            round: None,
            templates: None,
            review_headings: None,
            context: None,
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

        // P1-2: 実行レコード作成直後（実行前）に、親から継承したオーバーライド値を保存
        // （run_subtask は parent_exec.round / global_round を overrides に入れて
        //  execute_block_pipeline に渡すため、ここでは親の値をそのまま保存する）
        let parent_exec = store.get_execution_by_id(parent_run_id).unwrap();
        store
            .update_execution_overrides(
                prep.run_id,
                parent_exec.tag.as_deref(),
                parent_exec.round,
                parent_exec.global_round,
                None,
                None,
                None
            )
            .unwrap();

        // 検証: サブ実行に round / global_round / batch_id が保存されている
        let sub_exec = store.get_execution_by_id(prep.run_id).unwrap();
        assert_eq!(
            sub_exec.round,
            Some(1),
            "サブ実行の round が親から引き継がれる（P1-2）"
        );
        assert_eq!(
            sub_exec.global_round,
            Some(5),
            "サブ実行の global_round が親から引き継がれる（P1-2）"
        );
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

    // ---- U4: Ctrl+D finalize（request_main_block_finalize） ----

    /// SAI_PROJECT_ROOT を書き換えるテスト用のロック（環境変数はプロセス共有のため
    /// 並列実行される他テストと直列化する）。ロック保持中に別テストがパニックしても
    /// 後続テストが Poison で落ちないよう、取得時は毒化を回復する。
    /// FINALIZE_ENV_LOCK を毒化回復付きで取得する。
    fn finalize_env_lock() -> std::sync::MutexGuard<'static, ()> {
        // 環境変数を設定する他モジュールのテスト（run_dir 等）と排他する
        crate::common::SAI_ENV_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner())
    }

    /// ユニークな一時ディレクトリを作成する（テスト用プロジェクトルート）。
    fn finalize_test_temp_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "sai-engine-finalize-{}-{}",
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
        // SAI_ENV_TEST_LOCK は通知ディレクトリ・環境変数の両方を保護する。
        let _guard = finalize_env_lock();
        let temp_root = finalize_test_temp_dir("request-finalize");
        let _env_guard = EnvVarGuard::set("SAI_PROJECT_ROOT", temp_root.as_os_str());

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

        // 2. DB が completed + finalized=true になっている（sai block list で「完了」表示）
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
        let _root_env = EnvVarGuard::set("SAI_PROJECT_ROOT", temp_root.as_os_str());
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
            store.update_execution_runner(run_id, "cc").unwrap();
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
        let _root_env = EnvVarGuard::set("SAI_PROJECT_ROOT", temp_root.as_os_str());
        let _home_env = EnvVarGuard::set("HOME", temp_root.as_os_str());

        let store = crate::db::SaiStore::open_in_memory().unwrap();
        let block = crate::db::models::test_helpers::create_test_block("cc-msg-empty");
        let block_id = store.insert_block(&block).unwrap();
        let run_id = store.create_execution(block_id, None).unwrap();
        store.update_execution_runner(run_id, "cc").unwrap();
        // agent_session_id を入れないので解決できない
        let md = write_message_file(&store, run_id).expect("message.md を書ける");
        let text = std::fs::read_to_string(&md).unwrap();
        assert!(
            !text.contains("## user") && !text.contains("## assistant"),
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

    // ---- review のひな形（M05: 見出しをブロックごとに選ぶ） ----

    /// 既定の見出しで作ったひな形の書式が、従来の書式と一致すること。
    /// （`date:` 行のみ実行時刻で変わるため、それ以降を比較する）
    #[test]
    fn review_template_text_default_matches_current_format() {
        let text = review_template_text("plan-ice", 10, &default_review_headings());
        let mut lines = text.lines();
        assert_eq!(lines.next(), Some("---"));
        assert_eq!(lines.next(), Some("block: plan-ice"));
        assert!(
            lines.next().unwrap_or_default().starts_with("date: "),
            "date 行が必要"
        );
        assert_eq!(lines.next(), Some("run_id: 10"));
        let (_, rest) = text
            .split_once("\nrun_id: 10\n")
            .expect("run_id 行が見つかる");
        assert_eq!(rest, "---\n\n### 追加修正\n\n\n### 追加実装\n\n");
    }

    /// 研究向けの 3 見出しで見本どおりのひな形になること。
    #[test]
    fn review_template_text_research_headings() {
        let headings = vec![
            "納得した点".to_string(),
            "疑問に思った点".to_string(),
            "次に知りたいこと".to_string(),
        ];
        let text = review_template_text("🔍 論文検索", 5, &headings);
        let (header, rest) = text
            .split_once("\n---\n\n")
            .expect("frontmatter が見つかる");
        assert!(
            header.starts_with("---\nblock: 🔍 論文検索\ndate: "),
            "header: {header}"
        );
        assert!(header.ends_with("\nrun_id: 5"), "header: {header}");
        assert_eq!(
            rest,
            "### 納得した点\n\n\n### 疑問に思った点\n\n\n### 次に知りたいこと\n\n"
        );
    }

    /// 見出しが空なら既定の 2 見出しに戻す。
    #[test]
    fn review_template_text_empty_headings_use_default() {
        let text = review_template_text("b", 1, &[]);
        assert!(
            text.ends_with("### 追加修正\n\n\n### 追加実装\n\n"),
            "{text}"
        );
    }

    // ── 範囲（context）の差し込み（U16） ──

    /// `{{context}}` があれば最初の 1 つだけを置き換え、2 つ目以降は残す。
    #[test]
    fn insert_context_replaces_first_or_prepends() {
        // 置換
        assert_eq!(
            insert_context("前\n{{context}}\n後", "範囲"),
            "前\n範囲\n後"
        );
        // 最初の 1 つだけ（2 つ目以降はそのまま残る）
        assert_eq!(
            insert_context("{{context}} と {{context}}", "範囲"),
            "範囲 と {{context}}"
        );
        // 無いときは先頭に範囲と空行を置く
        assert_eq!(insert_context("# 本文", "範囲"), "範囲\n\n# 本文");
    }

    /// AC1: `context: true` のブロックは `prepare_pipeline` で本文の
    /// `{{context}}` が命令の標準出力に置き換わる。
    /// `context` の無いブロックの本文は今と 1 字も変わらない。
    #[test]
    fn test_prepare_pipeline_inserts_context_when_enabled() {
        let _env_lock = finalize_env_lock();
        // テスト用のプロジェクトルート（settings.json と .sai/sai-engine を持つ）
        let root = finalize_test_temp_dir("context");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join(".sai")).unwrap();
        std::fs::write(
            root.join(".sai/settings.json"),
            r#"{"context": ["/bin/echo", "run={run_id}", "block={block}"]}"#,
        )
        .unwrap();
        let _root_guard = EnvVarGuard::set("SAI_PROJECT_ROOT", root.as_os_str());
        let _home_guard = EnvVarGuard::set("SAI_SAI_HOME", root.as_os_str());

        // context: true → 本文の {{context}} が標準出力に置き換わる
        let mut config = minimal_config();
        config.run_id = Some(99);
        config.block_config.context = Some(true);
        config.block_body = "前\n{{context}}\n後".to_string();
        let prepared = prepare_pipeline(&config).expect("prepare_pipeline に失敗");
        assert_eq!(prepared.expanded_prompt, "前\nrun=99 block=test-block\n後");
        crate::session::cleanup_session_dir(&prepared.session_dir);

        // context なし → 本文は今と変わらない（settings.json に context が有っても使わない）
        let mut config = minimal_config();
        config.run_id = Some(99);
        config.block_body = "前\n{{context}}\n後".to_string();
        let prepared = prepare_pipeline(&config).expect("prepare_pipeline に失敗");
        assert_eq!(prepared.expanded_prompt, "前\n{{context}}\n後");
        crate::session::cleanup_session_dir(&prepared.session_dir);

        let _ = std::fs::remove_dir_all(&root);
    }
}
