//! # herdr CLI 連携モジュール
//!
//! このモジュールは herdr（ターミナルマルチプレクサ）の CLI インターフェースを
//! Rust から操作するための型と関数を提供します。
//!
//! `std::process::Command` を使用して herdr バイナリを呼び出し、
//! JSON レスポンスを `serde_json` でデシリアライズします。
//!
//! このモジュールは `lib/herdrs/pane-manager.ts` の TypeScript 実装と
//! 同等の機能を Rust で提供します。
//!
//! ## 対応コマンド
//!
//! - `herdr pane list` — 全ペイン一覧の取得
//! - `herdr pane split` — ペインの分割
//! - `herdr pane run` — ペイン内でのコマンド実行
//! - `herdr pane send-text` — ペインへのテキスト送信
//! - `herdr pane send-keys` — ペインへのキー送信
//! - `herdr pane get` — ペイン情報の取得
//!
//! - `herdr pane close` — ペインを閉じる
//! ## 使用例
//!
//! ```no_run
//! use sai_link_engine::herdr::{HerdrClient, SplitDirection};
//!
//! let client = HerdrClient::new();
//! if client.is_herdr_available() {
//!     let pane = client.split_pane(SplitDirection::Right, None)
//!         .expect("ペイン分割に失敗しました");
//!     client.run_in_pane(&pane.pane_id, "echo hello")
//!         .expect("コマンド実行に失敗しました");
//!     println!("新しいペイン: {}", pane.pane_id);
//! }
//! ```

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::process::{Command, ExitStatus, Output};

// ============================================================================
// 型定義 (Step 1)
// ============================================================================

/// ペイン分割方向
///
/// `herdr pane split` コマンドの `--direction` オプションに指定する方向。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SplitDirection {
    /// 右方向に分割
    Right,
    /// 下方向に分割
    Down,
}

impl std::fmt::Display for SplitDirection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SplitDirection::Right => write!(f, "right"),
            SplitDirection::Down => write!(f, "down"),
        }
    }
}

/// ペイン情報
///
/// herdr のペインの状態を表す構造体。
/// `herdr pane list` / `herdr pane get` / `herdr pane split` の
/// JSON レスポンスに含まれるペインの属性を保持します。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct PaneInfo {
    /// ペインの一意識別子
    pub pane_id: String,
    /// ペインが所属するワークスペースの ID
    pub workspace_id: String,
    /// ペインが所属するタブの ID
    pub tab_id: String,
    /// ペインが現在フォーカスされているかどうか
    pub focused: bool,
    /// ペインのカレントワーキングディレクトリ（未設定の場合は None）
    pub cwd: Option<String>,
    /// ペインで動作しているエージェント名（未設定の場合は None）
    pub agent: Option<String>,
    /// エージェントのステータス（"idle", "working", "done", "blocked" 等）
    pub agent_status: String,
    /// ペインに設定されたラベル（`herdr pane rename` で設定、未設定の場合は None）
    pub label: Option<String>,
    /// ペイン情報のリビジョン番号
    pub revision: i64,
}

/// `herdr pane list` コマンドの JSON レスポンス全体を表す型。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct PaneListEnvelope {
    /// レスポンスの結果部分
    pub result: PaneListResult,
}

/// `PaneListEnvelope` の内部結果。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct PaneListResult {
    /// 全ペインのリスト
    pub panes: Vec<PaneInfo>,
}

/// `herdr pane split` コマンドの JSON レスポンス全体を表す型。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct PaneSplitEnvelope {
    /// レスポンスの結果部分
    pub result: PaneSplitResult,
}

/// `PaneSplitEnvelope` の内部結果。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct PaneSplitResult {
    /// 新しく作成されたペイン
    pub pane: PaneInfo,
}

/// `herdr pane get` コマンドの JSON レスポンス全体を表す型。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct PaneGetEnvelope {
    /// レスポンスの結果部分
    pub result: PaneGetResult,
}

/// タブ情報
///
/// `herdr tab create` / `herdr tab get` の JSON レスポンスに含まれる
/// タブの属性を保持します。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct TabInfo {
    /// タブの一意識別子
    pub tab_id: String,
    /// タブが所属するワークスペースの ID
    pub workspace_id: String,
    /// タブのラベル（未設定の場合は None）
    pub label: Option<String>,
    /// タブの番号
    pub number: i64,
    /// タブ内のペイン数
    pub pane_count: i64,
    /// タブが現在フォーカスされているかどうか
    #[serde(default)]
    pub focused: bool,
    /// タブ内エージェントのステータス（"unknown", "idle", "working" 等）
    #[serde(default)]
    pub agent_status: String,
}

/// `herdr tab create` コマンドの JSON レスポンス全体を表す型。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct TabCreateEnvelope {
    /// レスポンスの結果部分
    pub result: TabCreateResult,
}

/// `herdr workspace list` コマンドの JSON レスポンス全体を表す型。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct WorkspaceListEnvelope {
    /// レスポンスの結果部分
    pub result: WorkspaceListResult,
}

/// `WorkspaceListEnvelope` の内部結果。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct WorkspaceListResult {
    /// ワークスペース一覧（未設定時は空）
    #[serde(default)]
    pub workspaces: Vec<WorkspaceInfo>,
}

/// ワークスペース（herdr の「space」）1 件。
///
/// 不要なフィールドは serde のデフォルトで無視される。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct WorkspaceInfo {
    /// ワークスペース ID
    pub workspace_id: String,
    /// ラベル（プロジェクト名等。未設定は空文字）
    #[serde(default)]
    pub label: String,
    /// フォーカス中か
    #[serde(default)]
    pub focused: bool,
}

/// `TabCreateEnvelope` の内部結果。
///
/// 新規タブ作成時は最初の（ルート）ペインも同時に生成されます。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct TabCreateResult {
    /// タブ作成時に生成される最初のペイン
    pub root_pane: PaneInfo,
    /// 作成されたタブ
    pub tab: TabInfo,
}

/// `PaneGetEnvelope` の内部結果。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct PaneGetResult {
    /// 取得したペイン
    pub pane: PaneInfo,
}

// ============================================================================
// セッションスナップショット関連の型（U2・R4 動的 pane 配置用）
// ============================================================================

/// `herdr api snapshot` の JSON レスポンス全体を表す型。
///
/// 不要なフィールド（agents / workspaces / protocol 等）は `serde(default)` で
/// 無視し、レイアウト計算に必要な `layouts` のみをデシリアライズする。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct SessionSnapshotEnvelope {
    /// レスポンスの結果部分
    pub result: SessionSnapshotResult,
}

/// `SessionSnapshotEnvelope` の内部結果。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct SessionSnapshotResult {
    /// スナップショット本体
    pub snapshot: SessionSnapshot,
}

/// セッション全体のスナップショット。
///
/// タブごとのレイアウト（pane の矩形座標）を保持する。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct SessionSnapshot {
    /// タブごとのレイアウト（pane の配置情報）
    #[serde(default)]
    pub layouts: Vec<TabLayout>,
}

/// 1 タブ内のレイアウト（pane 配置）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct TabLayout {
    /// タブの ID
    pub tab_id: String,
    /// タブが所属するワークスペースの ID
    #[serde(default)]
    pub workspace_id: String,
    /// タブ内のペイン（矩形座標付き）
    #[serde(default)]
    pub panes: Vec<LayoutPane>,
}

/// 矩形座標付きのペイン情報（スナップショット内の配置）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct LayoutPane {
    /// ペインの一意識別子
    pub pane_id: String,
    /// ペインの矩形座標
    pub rect: Rect,
}

/// ペインの矩形座標。
///
/// `x` / `y` はタイルの左上座標、`width` / `height` はサイズ。
/// 同タブ内で同じ `x` を持つペインは同一列に属する。
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct Rect {
    /// 左上 X 座標
    pub x: i64,
    /// 左上 Y 座標
    pub y: i64,
    /// 幅
    pub width: i64,
    /// 高さ
    pub height: i64,
}

/// herdr CLI がエラー時に返す JSON レスポンスを表す型。
///
/// 例: `{"error": {"message": "pane not found"}}`
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct HerdrErrorEnvelope {
    /// エラー情報（正常時は None）
    pub error: Option<HerdrErrorBody>,
}

/// エラーレスポンスの内部メッセージ。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct HerdrErrorBody {
    /// エラーメッセージ（エラーによっては含まれない場合あり）
    pub message: Option<String>,
}

// ============================================================================
// エラー型定義 (Step 4)
// ============================================================================

/// herdr CLI 操作に関するエラー
///
/// herdr バイナリの呼び出しから JSON パースまで、
/// herdr 連携で発生しうる全てのエラーを表現します。
#[derive(Debug, thiserror::Error)]
pub enum HerdrError {
    /// herdr バイナリが見つからない
    ///
    /// `PATH` 上に herdr が存在しない、または実行権限がない場合。
    #[error("herdr が見つかりません。herdr がインストールされているか確認してください。")]
    HerdrNotFound,

    /// herdr CLI が非ゼロの終了コードを返した
    ///
    /// exit_code は `Option<i32>` として保持され、プロセスがシグナルで
    /// 終了した場合は `None` となります。エラーメッセージには含まれませんが、
    /// プログラムからは `exit_code` フィールドで参照可能です。
    #[error("herdr CLI エラー: {stderr}")]
    HerdrCliError {
        /// 終了コード（プロセスがシグナルで終了した場合は None）
        exit_code: Option<i32>,
        /// 標準エラー出力の内容
        stderr: String,
    },

    /// フォーカスされたペインが見つからない
    ///
    /// `split_pane()` 実行時に `focused: true` のペインが
    /// 1つも存在しない場合。
    #[error("フォーカスされたペインが見つかりません。")]
    NoFocusedPane,

    /// herdr コマンドの標準出力が空
    #[error("herdr の出力が空です (コマンド: {command})")]
    EmptyOutput {
        /// 実行したコマンド文字列
        command: String,
    },

    /// JSON のデシリアライズに失敗
    #[error("JSON パース失敗: {0}")]
    JsonParseError(#[from] serde_json::Error),

    /// I/O エラー（プロセス起動失敗等）
    #[error("IO エラー: {0}")]
    IoError(#[from] std::io::Error),

    /// UTF-8 文字列への変換に失敗
    #[error("UTF-8 変換エラー: {0}")]
    Utf8Error(#[from] std::string::FromUtf8Error),

    /// herdr API がエラーレスポンスを返した
    ///
    /// コマンドは成功（exit 0）したが、JSON レスポンスに
    /// `error` フィールドが含まれていた場合。
    #[error("herdr API エラー: {message}")]
    ApiError {
        /// エラーメッセージ
        message: String,
    },

    /// herdr CLI がタイムアウトした
    ///
    /// 指定時間内に herdr コマンドが完了しなかった場合。
    /// ペインが削除された直後などに herdr CLI がハングすることがあるため、
    /// デーモンの実行パイプラインを保護する（§4.2 ケース3 の耐障害性）。
    #[error(
        "herdr CLI タイムアウト: {timeout_ms}ms 以内に完了しませんでした (コマンド: {command})"
    )]
    HerdrTimeout {
        /// タイムアウト（ミリ秒）
        timeout_ms: u64,
        /// 実行したコマンド文字列
        command: String,
    },
}

// ============================================================================
// HerdrClient 構造体とメソッド (Step 2 + Step 3)
// ============================================================================

/// herdr CLI 操作用クライアント
///
/// `std::process::Command` を使用して herdr バイナリを呼び出し、
/// ペイン操作のためのメソッドを提供します。
///
/// # バイナリパスのカスタマイズ
///
/// デフォルトでは `PATH` 上の `herdr` バイナリを使用します。
/// テスト時など、異なるパスのバイナリを使用する場合は
/// [`HerdrClient::with_binary`] を使用してください。
///
/// # スレッド安全性
///
/// `HerdrClient` は内部状態を持たないため `Send + Sync` です。
///
/// # 例
///
/// ```no_run
/// use sai_link_engine::herdr::{HerdrClient, SplitDirection};
///
/// let client = HerdrClient::new();
/// if client.is_herdr_available() {
///     let panes = client.list_panes().expect("ペイン一覧の取得に失敗");
///     println!("現在のペイン数: {}", panes.len());
/// }
/// ```
pub struct HerdrClient {
    /// herdr バイナリのパス（デフォルト: "herdr"）
    binary_path: String,
}

/// herdr CLI 呼び出しの最大待機時間（ミリ秒）。
///
/// ペイン削除直後などに herdr CLI がハングするケースがあるため、
/// デーモンの実行パイプラインを保護するために設ける（§4.2 ケース3）。
/// 環境変数 `SAI_HERDR_TIMEOUT_MS` で上書きできる（2.10 の修正）。
fn herdr_timeout_ms() -> u64 {
    std::env::var("SAI_HERDR_TIMEOUT_MS")
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .filter(|v| *v > 0)
        .unwrap_or(30_000)
}

/// タブ（メインブロック）ごとの pane 上限のデフォルト（U2・R5）。
///
/// タブ 1 枚に最大 pane 数を配置する想定。`.sai/settings.json` の `wip` で上書きされる
/// （`crate::pi_command::wip_limit()`）。上限超過は待機（waiting_pane）で対処する。
pub const MAX_BATCH_PANES: usize = 8;

/// 1-n pane の均等レイアウト（縦 rows × 横 cols）を計算する（U2・R4 汎用版）。
///
/// WIP（タブごとの pane 上限）から列数・行数を導出する:
/// - `rows = if n <= 4 { 1 } else { 2 }`（最大 2 行。n>4 のときのみ 2 行目を使用）
/// - `cols = if n <= 4 { n } else { ceil(n / 2) }`（列数）
///
/// 例（WIP=8）: n=3 → (3,1)、n=5 → (3,2)、n=8 → (4,2)。
pub fn layout_for_n(n: usize) -> (usize, usize) {
    layout_for_wip(n, crate::pi_command::wip_limit())
}

/// 指定 WIP で 1-n pane の均等レイアウト（縦 rows × 横 cols）を計算する（汎用・テスト用）。
///
/// 計算式: `rows = if n <= 4 { 1 } else { 2 }`、`cols = if n <= 4 { n } else { ceil(n/2) }`。
pub fn layout_for_wip(n: usize, wip: usize) -> (usize, usize) {
    if n == 0 || wip == 0 {
        return (0, 0);
    }
    let rows = if n <= 4 { 1 } else { 2 };
    let cols = if n <= 4 { n } else { (n + 1) / 2 };
    (cols, rows)
}

impl HerdrClient {
    /// 新しい `HerdrClient` を作成します。
    ///
    /// デフォルトのバイナリパスは `"herdr"` で、`PATH` 環境変数から検索されます。
    pub fn new() -> Self {
        Self {
            binary_path: "herdr".to_string(),
        }
    }

    /// 任意のバイナリパスを指定して `HerdrClient` を作成します。
    ///
    /// テストでモックスクリプトを使用する場合などに利用します。
    ///
    /// # 引数
    ///
    /// * `path` — herdr 互換バイナリのパス（例: `"tests/fixtures/mock_herdr.sh"`）
    ///
    /// # 例
    ///
    /// ```
    /// use sai_link_engine::herdr::HerdrClient;
    /// let client = HerdrClient::with_binary("/path/to/mock_herdr.sh");
    /// ```
    pub fn with_binary(path: impl Into<String>) -> Self {
        Self {
            binary_path: path.into(),
        }
    }

    /// herdr CLI が利用可能かどうかを確認します。
    ///
    /// `herdr --version` を実行し、プロセスが成功すれば `true` を返します。
    /// herdr がインストールされていない場合や実行権限がない場合は `false` を返します。
    ///
    /// # 戻り値
    ///
    /// herdr が利用可能な場合は `true`、それ以外は `false`
    pub fn is_herdr_available(&self) -> bool {
        Command::new(&self.binary_path)
            .arg("--version")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    // ====================================================================
    // 内部ヘルパー
    // ====================================================================

    /// herdr コマンドを実行し、プロセスの出力を返す内部ヘルパー。
    ///
    /// I/O エラー（バイナリが見つからない等）が発生した場合は
    /// `HerdrError::IoError` を返します。
    /// コマンド自体の exit code のチェックは呼び出し元が行います。
    ///
    /// **タイムアウト付き**（`HERDR_COMMAND_TIMEOUT_MS`）:
    /// ペイン削除直後などに herdr CLI がハングするケースがあるため、
    /// 指定時間内に完了しない場合はプロセスを kill して `HerdrError::HerdrTimeout` を返す。
    fn run_command(&self, args: &[&str]) -> Result<Output, HerdrError> {
        let command_str = format!("{} {}", self.binary_path, args.join(" "));

        let mut child = Command::new(&self.binary_path)
            .args(args)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(HerdrError::IoError)?;

        let timeout_ms = herdr_timeout_ms();
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(timeout_ms);

        loop {
            match child.try_wait()? {
                Some(status) => {
                    // プロセス終了後、パイプから出力を読み取る
                    let mut stdout = Vec::new();
                    let mut stderr = Vec::new();
                    if let Some(mut so) = child.stdout.take() {
                        let _ = std::io::Read::read_to_end(&mut so, &mut stdout);
                    }
                    if let Some(mut se) = child.stderr.take() {
                        let _ = std::io::Read::read_to_end(&mut se, &mut stderr);
                    }
                    return Ok(Output {
                        status,
                        stdout,
                        stderr,
                    });
                }
                None => {
                    // タイムアウトチェック
                    if std::time::Instant::now() >= deadline {
                        let _ = child.kill();
                        let _ = child.wait();
                        return Err(HerdrError::HerdrTimeout {
                            timeout_ms,
                            command: command_str,
                        });
                    }
                    std::thread::sleep(std::time::Duration::from_millis(50));
                }
            }
        }
    }

    /// herdr コマンドを実行し、成功時は標準出力の文字列を返す内部ヘルパー。
    ///
    /// コマンドが非ゼロの終了コードを返した場合:
    /// - exit code 127 → `HerdrError::HerdrNotFound`
    /// - その他 → `HerdrError::HerdrCliError`
    fn run_and_get_stdout(&self, args: &[&str]) -> Result<String, HerdrError> {
        let output = self.run_command(args)?;
        if !output.status.success() {
            return Err(make_cli_error(output.status, &output.stderr));
        }
        let stdout = String::from_utf8(output.stdout)?;
        Ok(stdout.trim().to_string())
    }

    /// herdr コマンドを実行し、JSON レスポンスをデシリアライズする内部ヘルパー。
    ///
    /// 1. `run_and_get_stdout` でコマンドを実行
    /// 2. 空出力の場合は `HerdrError::EmptyOutput`
    /// 3. まず `HerdrErrorEnvelope` としてパースし、`error` フィールドがあれば `ApiError`
    /// 4. 正常系として型 `T` にデシリアライズ
    fn herdr_json<T: DeserializeOwned>(&self, args: &[&str]) -> Result<T, HerdrError> {
        let command_str = format!("{} {}", self.binary_path, args.join(" "));
        let raw = self.run_and_get_stdout(args)?;

        if raw.is_empty() {
            return Err(HerdrError::EmptyOutput {
                command: command_str,
            });
        }

        // エラーレスポンス（{"error": {"message": "..."}}）を確認
        if let Ok(error_envelope) = serde_json::from_str::<HerdrErrorEnvelope>(&raw) {
            if let Some(error_body) = error_envelope.error {
                let message = error_body
                    .message
                    .unwrap_or_else(|| "不明なエラー".to_string());
                return Err(HerdrError::ApiError { message });
            }
        }

        // 正常系としてデシリアライズ
        let result: T = serde_json::from_str(&raw)?;
        Ok(result)
    }

    /// JSON パース不要のコマンド（run, send-text, send-keys）用の簡易実行ヘルパー。
    ///
    /// コマンドの成否のみを確認し、成功時は `Ok(())` を返します。
    fn run_simple_command(&self, args: &[&str]) -> Result<(), HerdrError> {
        let output = self.run_command(args)?;
        if !output.status.success() {
            return Err(make_cli_error(output.status, &output.stderr));
        }
        Ok(())
    }

    /// フォーカスされたペインを取得する内部ヘルパー。
    ///
    /// `list_panes()` の結果から `focused: true` のペインを検索します。
    /// フォーカスされたペインが存在しない場合は `HerdrError::NoFocusedPane` を返します。
    fn get_focused_pane(&self) -> Result<PaneInfo, HerdrError> {
        let panes = self.list_panes()?;
        panes
            .into_iter()
            .find(|p| p.focused)
            .ok_or(HerdrError::NoFocusedPane)
    }

    // ====================================================================
    // メイン API (Step 3)
    // ====================================================================

    /// 現在フォーカスされているペインを指定した方向に分割します。
    ///
    /// 新しいペインにはフォーカスを移動しません（`--no-focus` オプション）。
    ///
    /// # 引数
    ///
    /// * `direction` — 分割方向（`SplitDirection::Right` または `SplitDirection::Down`）
    /// * `cwd` — 新しいペインの作業ディレクトリ（`None` の場合はカレントディレクトリを継承）
    ///
    /// # 戻り値
    ///
    /// 新しく作成されたペインの情報（`PaneInfo`）
    ///
    /// # エラー
    ///
    /// - `HerdrError::NoFocusedPane` — フォーカスされたペインが見つからない場合
    /// - `HerdrError::HerdrNotFound` — herdr バイナリが見つからない場合
    /// - `HerdrError::HerdrCliError` — herdr CLI がエラーを返した場合
    /// - `HerdrError::ApiError` — herdr API がエラーレスポンスを返した場合
    pub fn split_pane(
        &self,
        direction: SplitDirection,
        cwd: Option<&str>,
    ) -> Result<PaneInfo, HerdrError> {
        let source = self.get_focused_pane()?.pane_id;

        let direction_str = direction.to_string();
        let mut args: Vec<&str> = vec![
            "pane",
            "split",
            &source,
            "--direction",
            &direction_str,
            "--no-focus",
        ];

        if let Some(dir) = cwd {
            args.push("--cwd");
            args.push(dir);
        }

        let envelope: PaneSplitEnvelope = self.herdr_json(&args)?;
        Ok(envelope.result.pane)
    }

    /// 指定されたペインでコマンドを実行します。
    ///
    /// コマンド文字列はそのまま herdr に渡されます。
    /// `std::process::Command` はシェルを経由しないため、
    /// シェルエスケープは不要です。
    pub fn run_in_pane(&self, pane_id: &str, command: &str) -> Result<(), HerdrError> {
        self.run_simple_command(&["pane", "run", pane_id, command])
    }

    /// 指定されたペインにテキストを送信します。
    ///
    /// テキストはそのまま herdr に渡されます。
    pub fn send_text(&self, pane_id: &str, text: &str) -> Result<(), HerdrError> {
        self.run_simple_command(&["pane", "send-text", pane_id, text])
    }

    /// 指定されたペインにキーイベントを送信します。
    ///
    /// keys 引数はシェルエスケープされません。
    /// `Enter`, `Ctrl+c`, `Alt+Enter` 等のキー名を直接渡します。
    ///
    /// # 引数
    ///
    /// * `pane_id` — 対象ペインの ID
    /// * `keys` — 送信するキー（例: `"Enter"`, `"Ctrl+c"`, `"Alt+Enter"`）
    ///
    /// # エラー
    ///
    /// - `HerdrError::HerdrNotFound` — herdr バイナリが見つからない場合
    /// - `HerdrError::HerdrCliError` — herdr CLI がエラーを返した場合
    pub fn send_keys(&self, pane_id: &str, keys: &str) -> Result<(), HerdrError> {
        self.run_simple_command(&["pane", "send-keys", pane_id, keys])
    }

    /// 指定されたペインの最新情報を取得します。
    ///
    /// ペインの状態が変化したかどうかを確認するために使用します。
    ///
    /// # 引数
    ///
    /// * `pane_id` — 対象ペインの ID
    ///
    /// # 戻り値
    ///
    /// ペインの最新状態を表す `PaneInfo`
    ///
    /// # エラー
    ///
    /// - `HerdrError::HerdrNotFound` — herdr バイナリが見つからない場合
    /// - `HerdrError::HerdrCliError` — herdr CLI がエラーを返した場合
    /// - `HerdrError::ApiError` — 指定されたペインが存在しない場合等
    pub fn get_pane_status(&self, pane_id: &str) -> Result<PaneInfo, HerdrError> {
        let envelope: PaneGetEnvelope = self.herdr_json(&["pane", "get", pane_id])?;
        Ok(envelope.result.pane)
    }

    /// 全ペインの一覧を取得します。
    ///
    /// # 戻り値
    ///
    /// 現在のワークスペース内の全ペインの `Vec<PaneInfo>`
    ///
    /// # エラー
    ///
    /// - `HerdrError::HerdrNotFound` — herdr バイナリが見つからない場合
    /// - `HerdrError::HerdrCliError` — herdr CLI がエラーを返した場合
    pub fn list_panes(&self) -> Result<Vec<PaneInfo>, HerdrError> {
        let envelope: PaneListEnvelope = self.herdr_json(&["pane", "list"])?;
        Ok(envelope.result.panes)
    }

    /// 新しいタブを作成します（U2・R4）。
    ///
    /// `herdr tab create --no-focus` を実行します。
    /// タブ作成と同時に最初の（ルート）ペインが生成されるため、
    /// バッチ実行の pane 総数を事前カウントしてからタブ 1 枚を用意し、
    /// そのタブ内で `split_pane_in_tab` により均等分割します。
    ///
    /// # 引数
    ///
    /// * `workspace_id` — タブを作成する workspace の ID（U11・R23）。
    ///   `Some` の場合は `--workspace <id>` で明示指定します。メインブロックが作業していた
    ///   プロジェクト（space）へタブを作成するために使用します。`None` の場合は
    ///   herdr の現在 workspace を使用する従来動作にフォールバックします。
    ///
    /// # 戻り値
    ///
    /// タブ情報と、タブ作成と同時に生成される最初の（ルート）ペインの情報。
    /// ルートペインは空のシェルで、sai サブタスクの dynamic pane 配置では
    /// 列計算から除外するために保持する（U2・R4 案X）。
    ///
    /// # エラー
    ///
    /// - `HerdrError::HerdrNotFound` — herdr バイナリが見つからない場合
    /// - `HerdrError::HerdrCliError` — herdr CLI がエラーを返した場合
    pub fn create_tab(
        &self,
        workspace_id: Option<&str>,
        cwd: Option<&str>,
    ) -> Result<(TabInfo, PaneInfo), HerdrError> {
        let mut args: Vec<&str> = vec!["tab", "create", "--no-focus"];
        // U11・R23: メインブロックの workspace を明示指定（別 space にタブが作られないようにする）
        if let Some(ws) = workspace_id {
            args.push("--workspace");
            args.push(ws);
        }
        // タブ作成時に自動生成されるルートペインの作業ディレクトリを明示する。
        // 指定が無いとプロジェクトと無関係な場所が cwd になり、
        // そこで起動した子エージェントが別の AGENTS.md を読んでしまう。
        if let Some(dir) = cwd {
            args.push("--cwd");
            args.push(dir);
        }
        let envelope: TabCreateEnvelope = self.herdr_json(&args)?;
        let root_pane = envelope.result.root_pane;
        Ok((envelope.result.tab, root_pane))
    }

    /// ワークスペース（space）一覧を取得します。
    pub fn workspace_list(&self) -> Result<Vec<WorkspaceInfo>, HerdrError> {
        let envelope: WorkspaceListEnvelope = self.herdr_json(&["workspace", "list"])?;
        Ok(envelope.result.workspaces)
    }

    /// 指定ワークスペース内のペイン一覧を取得します。
    pub fn list_panes_in(&self, workspace_id: &str) -> Result<Vec<PaneInfo>, HerdrError> {
        let envelope: PaneListEnvelope =
            self.herdr_json(&["pane", "list", "--workspace", workspace_id])?;
        Ok(envelope.result.panes)
    }

    /// プロジェクトフォルダで作業しているペインを持つワークスペースを探す。
    ///
    /// 親実行の pane が特定できなかった場合の最終フォールバック用。
    /// プロジェクトルート配下（`.worktrees/<branch>` を含む）で開いているペインを
    /// 持つ space を候補とし、
    ///
    /// 1. 候補が 1 つだけ → それを返す
    /// 2. 複数 → ラベルがプロジェクトフォルダ名に一致するものを返す
    /// 3. それ以外 → `None`（呼び出し元は従来どおり「現在の space」へフォールバック）
    pub fn find_workspace_for_project(&self, project_root: &std::path::Path) -> Option<String> {
        let root = project_root.to_string_lossy().to_string();
        let project_name = project_root
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        let workspaces = self.workspace_list().ok()?;
        let mut matched: Vec<WorkspaceInfo> = Vec::new();
        for ws in workspaces {
            let panes = match self.list_panes_in(&ws.workspace_id) {
                Ok(p) => p,
                Err(_) => continue,
            };
            let hit = panes.iter().any(|p| match p.cwd.as_deref() {
                Some(cwd) => cwd == root || cwd.starts_with(&format!("{}/", root)),
                None => false,
            });
            if hit {
                matched.push(ws);
            }
        }
        match matched.len() {
            1 => Some(matched.remove(0).workspace_id),
            n if n > 1 => matched
                .into_iter()
                .find(|w| !project_name.is_empty() && w.label == project_name)
                .map(|w| w.workspace_id),
            _ => None,
        }
    }

    /// 指定タブ内でペインを分割します（U2・R4）。
    ///
    /// `split_pane()` の `get_focused_pane()` 依存（逐次縮小の原因）を排除し、
    /// タブに属する既存ペインを分割基準として `--no-focus` で新規ペインを作成します。
    /// 分割先ペインの `tab_id` は元ペインと同じタブに属します。
    ///
    /// # 引数
    ///
    /// * `tab_id` — 分割対象が属するタブの ID
    /// * `direction` — 分割方向（`SplitDirection::Right` または `SplitDirection::Down`）
    /// * `cwd` — 新しいペインの作業ディレクトリ（`None` の場合はカレントディレクトリを継承）
    ///
    /// # 戻り値
    ///
    /// 新しく作成されたペインの情報（`PaneInfo`）
    ///
    /// # エラー
    ///
    /// - `HerdrError::ApiError` — 指定タブ内にペインが見つからない場合
    /// - `HerdrError::HerdrNotFound` — herdr バイナリが見つからない場合
    pub fn split_pane_in_tab(
        &self,
        tab_id: &str,
        direction: SplitDirection,
        cwd: Option<&str>,
    ) -> Result<PaneInfo, HerdrError> {
        // タブ内で分割基準となるペインを選択する。
        // タブに属する最初のペインを基準とし、フォーカス依存を排除する。
        let panes = self.list_panes()?;
        let source = panes
            .into_iter()
            .find(|p| p.tab_id == tab_id)
            .map(|p| p.pane_id)
            .ok_or_else(|| HerdrError::ApiError {
                message: format!("タブ {} 内に分割基準となるペインが見つかりません", tab_id),
            })?;

        let direction_str = direction.to_string();
        let mut args: Vec<&str> = vec![
            "pane",
            "split",
            &source,
            "--direction",
            &direction_str,
            "--no-focus",
        ];

        if let Some(dir) = cwd {
            args.push("--cwd");
            args.push(dir);
        }

        let envelope: PaneSplitEnvelope = self.herdr_json(&args)?;
        Ok(envelope.result.pane)
    }

    /// 指定されたペインを基準に分割します（U2・R4 動的配置用）。
    ///
    /// `split_pane_in_tab` と異なり、フォーカスや「タブ内最初のペイン」に依存せず、
    /// 渡された `pane_id` を分割基準として明示します。
    /// レイアウト計算（列構造）で決定した基底ペインを指定して
    /// Right / Down 分割を行う際に使用します。
    ///
    /// # 引数
    ///
    /// * `pane_id` — 分割基準とするペインの ID
    /// * `direction` — 分割方向（`SplitDirection::Right` / `SplitDirection::Down`）
    /// * `cwd` — 新しいペインの作業ディレクトリ（`None` の場合はカレントディレクトリを継承）
    ///
    /// # 戻り値
    ///
    /// 新しく作成されたペインの情報（`PaneInfo`）
    pub fn split_pane_from(
        &self,
        pane_id: &str,
        direction: SplitDirection,
        cwd: Option<&str>,
    ) -> Result<PaneInfo, HerdrError> {
        let direction_str = direction.to_string();
        let mut args: Vec<&str> = vec![
            "pane",
            "split",
            pane_id,
            "--direction",
            &direction_str,
            "--no-focus",
        ];

        if let Some(dir) = cwd {
            args.push("--cwd");
            args.push(dir);
        }

        let envelope: PaneSplitEnvelope = self.herdr_json(&args)?;
        Ok(envelope.result.pane)
    }

    /// セッションスナップショットから、指定タブのペイン配置を取得します（U2・R4）。
    ///
    /// `herdr api snapshot` を実行し、`tab_id` に一致するタブの
    /// ペイン矩形座標（`LayoutPane`）一覧を返します。
    /// これにより「現在開かれている pane の列構造」を把握し、
    /// 次の pane の配置（基底 + 分割方向）を動的に決定できます。
    ///
    /// # 引数
    ///
    /// * `tab_id` — 対象タブの ID
    ///
    /// # 戻り値
    ///
    /// 指定タブ内のペイン配置一覧。タブが存在しない場合や
    /// レイアウト取得に失敗した場合は `HerdrError` を返します。
    pub fn snapshot_layout(&self, tab_id: &str) -> Result<Vec<LayoutPane>, HerdrError> {
        let envelope: SessionSnapshotEnvelope = self.herdr_json(&["api", "snapshot"])?;
        let layout = envelope
            .result
            .snapshot
            .layouts
            .into_iter()
            .find(|l| l.tab_id == tab_id)
            .ok_or_else(|| HerdrError::ApiError {
                message: format!(
                    "スナップショットにタブ {} のレイアウトが見つかりません",
                    tab_id
                ),
            })?;
        Ok(layout.panes)
    }

    /// 指定されたタブを閉じます（ベストエフォート・U2・R4）。
    ///
    /// `herdr tab close <tab_id>` を実行します。
    /// タブ内の全ペインが閉じられ、最終的にタブごと破棄されます。
    /// タブが既に存在しない場合など、エラーは無視され `Ok(())` を返します。
    ///
    /// # 引数
    ///
    /// * `tab_id` — 閉じるタブの ID
    ///
    /// # 戻り値
    ///
    /// 常に `Ok(())`（エラーは無視される）
    pub fn close_tab(&self, tab_id: &str) -> Result<(), HerdrError> {
        let result = self.run_simple_command(&["tab", "close", tab_id]);
        match result {
            Ok(()) => Ok(()),
            Err(_) => Ok(()), // エラーを無視
        }
    }

    /// 指定されたペインを閉じます（ベストエフォート）。
    ///
    /// `herdr pane close <pane_id>` を実行します。
    /// ペインが既に存在しない場合など、エラーは無視され `Ok(())` を返します。
    ///
    /// # 引数
    ///
    /// * `pane_id` — 閉じるペインの ID
    ///
    /// # 戻り値
    ///
    /// 常に `Ok(())`（エラーは無視される）
    ///
    /// # 例
    ///
    /// ```no_run
    /// use sai_link_engine::herdr::HerdrClient;
    ///
    /// let client = HerdrClient::new();
    /// // エラーは無視される
    /// let _ = client.close_pane("pane_test_001");
    /// ```
    pub fn close_pane(&self, pane_id: &str) -> Result<(), HerdrError> {
        // TypeScript 版と同様、ベストエフォートでエラーは無視
        let result = self.run_simple_command(&["pane", "close", pane_id]);
        match result {
            Ok(()) => Ok(()),
            Err(_) => Ok(()), // エラーを無視
        }
    }

    /// ペインの最近の出力を読み取る（タスク調査用・P4）。
    ///
    /// `herdr pane read <pane_id> --source recent-unwrapped --lines N --format text` を
    /// 実行し、ペインの最近の出力テキストを返す。
    /// サブブロックの失敗原因調査（task_capture）に使用する。
    ///
    /// # 引数
    ///
    /// * `pane_id` — 読み取るペインの ID
    /// * `lines` — 読み取る行数（デフォルト 50）
    ///
    /// # 戻り値
    ///
    /// ペインの出力テキスト（trim 済み）
    pub fn read_pane_output(&self, pane_id: &str, lines: u32) -> Result<String, HerdrError> {
        let lines_str = lines.to_string();
        let output = self.run_command(&[
            "pane",
            "read",
            pane_id,
            "--source",
            "recent-unwrapped",
            "--lines",
            &lines_str,
            "--format",
            "text",
        ])?;
        if !output.status.success() {
            return Err(make_cli_error(output.status, &output.stderr));
        }
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }

    /// 指定されたペインにラベルを設定します。
    ///
    /// `herdr pane rename <pane_id> <label>` を実行します。
    /// ラベルは herdr のサイドバーに表示され、ユーザーがペインを
    /// 視覚的に識別するために使用されます。
    ///
    /// # 引数
    ///
    /// * `pane_id` — 対象ペインの ID
    /// * `label` — 設定するラベル（例: `"sai-my-block-42"`）
    ///
    /// # 戻り値
    ///
    /// 更新されたペイン情報（`label` フィールドを含む）
    ///
    /// # エラー
    ///
    /// - `HerdrError::HerdrNotFound` — herdr バイナリが見つからない場合
    /// - `HerdrError::HerdrCliError` — herdr CLI がエラーを返した場合
    ///
    /// # 例
    ///
    /// ```no_run
    /// use sai_link_engine::herdr::HerdrClient;
    ///
    /// let client = HerdrClient::new();
    /// let info = client.rename_pane("wA:pC", "sai-test-block-3")
    ///     .expect("ラベル設定に失敗");
    /// assert_eq!(info.label.as_deref(), Some("sai-test-block-3"));
    /// ```
    pub fn rename_pane(&self, pane_id: &str, label: &str) -> Result<PaneInfo, HerdrError> {
        let envelope: PaneGetEnvelope = self.herdr_json(&["pane", "rename", pane_id, label])?;
        Ok(envelope.result.pane)
    }

    /// 指定されたペインのラベルをクリアします。
    ///
    /// `herdr pane rename <pane_id> --clear` を実行します。
    ///
    /// # 引数
    ///
    /// * `pane_id` — 対象ペインの ID
    ///
    /// # 戻り値
    ///
    /// 更新されたペイン情報（`label` が `None` になっている）
    ///
    /// # エラー
    ///
    /// - `HerdrError::HerdrNotFound` — herdr バイナリが見つからない場合
    /// - `HerdrError::HerdrCliError` — herdr CLI がエラーを返した場合
    pub fn clear_pane_label(&self, pane_id: &str) -> Result<PaneInfo, HerdrError> {
        let envelope: PaneGetEnvelope = self.herdr_json(&["pane", "rename", pane_id, "--clear"])?;
        Ok(envelope.result.pane)
    }
}

impl Default for HerdrClient {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// スタンドアロン関数 (Step 2)
// ============================================================================

/// herdr CLI が利用可能かどうかを確認します。
///
/// 簡易チェック用のスタンドアロン関数版。
/// `HerdrClient::new().is_herdr_available()` と同義です。
///
/// # 戻り値
///
/// herdr が利用可能な場合は `true`、それ以外は `false`
///
/// # 例
///
/// ```
/// use sai_link_engine::herdr::is_herdr_available;
/// let available = is_herdr_available();
/// ```
pub fn is_herdr_available() -> bool {
    HerdrClient::new().is_herdr_available()
}

/// シェル用にシングルクォートをエスケープします。
///
/// `lib/herdrs/types.ts` の `shellEscape()` 関数および
/// `lib/herdrs/pane-manager.ts` のエスケープ処理と等価です。
///
/// 変換ルール:
/// - シングルクォート `'` を `'\''`（クォートを閉じる + エスケープ + クォートを開く）に置換
/// - 全体をシングルクォート `'...'` で囲む
///
/// # 引数
///
/// * `value` — エスケープする文字列
///
/// # 戻り値
///
/// シェルセーフにエスケープされた文字列（シングルクォートで囲まれる）
///
/// # 例
///
/// ```
/// use sai_link_engine::herdr::shell_escape_single_quotes;
///
/// assert_eq!(shell_escape_single_quotes("hello"), "'hello'");
/// assert_eq!(shell_escape_single_quotes("it's"), "'it'\\''s'");
/// ```
pub fn shell_escape_single_quotes(value: &str) -> String {
    let escaped = value.replace('\'', "'\\''");
    format!("'{}'", escaped)
}

// ============================================================================
// 内部ユーティリティ
// ============================================================================

/// プロセスの終了ステータスと stderr から `HerdrError::HerdrCliError` または
/// `HerdrError::HerdrNotFound` を生成します。
///
/// exit code 127（command not found）の場合は `HerdrNotFound` を返します。
fn make_cli_error(status: ExitStatus, stderr: &[u8]) -> HerdrError {
    let stderr_str = String::from_utf8_lossy(stderr).to_string();
    match status.code() {
        Some(127) => HerdrError::HerdrNotFound,
        Some(code) => HerdrError::HerdrCliError {
            exit_code: Some(code),
            stderr: stderr_str,
        },
        None => HerdrError::HerdrCliError {
            exit_code: None,
            stderr: stderr_str,
        },
    }
}
