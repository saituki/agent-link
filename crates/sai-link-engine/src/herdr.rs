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

use serde::{Deserialize, Serialize};

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
    /// 呼び出し互換のために残すパス（擬似 pane だけを受けるので実際には読まない）。
    #[allow(dead_code)]
    binary_path: String,
}

/// タブ（メインブロック）ごとの pane 上限のデフォルト（U2・R5）。
///
/// タブ 1 枚に最大 pane 数を配置する想定。`.sai/settings.json` の `wip` で上書きされる
/// （`crate::settings::wip_limit()`）。上限超過は待機（waiting_pane）で対処する。
pub const MAX_BATCH_PANES: usize = 8;

/// 1-n pane の均等レイアウト（縦 rows × 横 cols）を計算する（U2・R4 汎用版）。
///
/// WIP（タブごとの pane 上限）から列数・行数を導出する:
/// - `rows = if n <= 4 { 1 } else { 2 }`（最大 2 行。n>4 のときのみ 2 行目を使用）
/// - `cols = if n <= 4 { n } else { ceil(n / 2) }`（列数）
///
/// 例（WIP=8）: n=3 → (3,1)、n=5 → (3,2)、n=8 → (4,2)。
pub fn layout_for_n(n: usize) -> (usize, usize) {
    layout_for_wip(n, crate::settings::wip_limit())
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

/// herdr が使えないときに返すエラー（sai-tree は headless だけ）。
fn disabled_error() -> HerdrError {
    HerdrError::ApiError {
        message: "herdr は使えません（sai-tree は headless だけです）".to_string(),
    }
}

/// ラベル操作用の合成 `PaneInfo`（herdr を呼ばない）。
fn synthetic_pane(pane_id: &str, label: Option<&str>) -> PaneInfo {
    PaneInfo {
        pane_id: pane_id.to_string(),
        workspace_id: "headless".to_string(),
        tab_id: "headless".to_string(),
        focused: false,
        cwd: None,
        agent: None,
        agent_status: "idle".to_string(),
        label: label.map(|s| s.to_string()),
        revision: 0,
    }
}

// ponytail: 名前は herdr のままの擬似 pane の窓口
impl HerdrClient {
    /// 新しい `HerdrClient` を作成します。
    pub fn new() -> Self {
        Self {
            binary_path: "herdr".to_string(),
        }
    }

    /// 任意のバイナリパスを指定して `HerdrClient` を作成します（互換のため残す）。
    pub fn with_binary(path: impl Into<String>) -> Self {
        Self {
            binary_path: path.into(),
        }
    }

    /// herdr CLI が利用可能かどうかを確認します。
    ///
    /// sai-tree は headless だけなので常に `false`。
    pub fn is_herdr_available(&self) -> bool {
        false
    }

    // ====================================================================
    // メイン API（擬似 pane だけを受ける）
    // ====================================================================

    /// 新しい pane を作る（herdr の分割は使わない）。
    ///
    /// 呼び出し側は `crate::headless::new_pane` を使う。ここは互換のため
    /// エラーを返す（§3 で呼ばれなくなる）。
    pub fn split_pane(
        &self,
        _direction: SplitDirection,
        _cwd: Option<&str>,
    ) -> Result<PaneInfo, HerdrError> {
        Err(disabled_error())
    }

    /// 擬似 pane に起動コマンドを入れる。
    pub fn run_in_pane(&self, pane_id: &str, command: &str) -> Result<(), HerdrError> {
        if crate::headless::is_headless_pane(pane_id) {
            return crate::headless::run(pane_id, command);
        }
        Err(disabled_error())
    }

    /// 擬似 pane にテキストを送る（`draft.txt` に上書き）。
    pub fn send_text(&self, pane_id: &str, text: &str) -> Result<(), HerdrError> {
        if crate::headless::is_headless_pane(pane_id) {
            return crate::headless::send_text(pane_id, text);
        }
        Err(disabled_error())
    }

    /// 擬似 pane にキーを送る（`esc` は中断、確定キーは `inbox/` へ）。
    pub fn send_keys(&self, pane_id: &str, keys: &str) -> Result<(), HerdrError> {
        if crate::headless::is_headless_pane(pane_id) {
            return crate::headless::send_keys(pane_id, keys);
        }
        Err(disabled_error())
    }

    /// 擬似 pane の状態を読む（`state.json`）。
    pub fn get_pane_status(&self, pane_id: &str) -> Result<PaneInfo, HerdrError> {
        if crate::headless::is_headless_pane(pane_id) {
            return crate::headless::status(pane_id);
        }
        Err(disabled_error())
    }

    /// 全ペインの一覧を取得する（herdr では使わない）。
    pub fn list_panes(&self) -> Result<Vec<PaneInfo>, HerdrError> {
        Err(disabled_error())
    }

    /// 新しいタブを作成する（herdr では使わない）。
    pub fn create_tab(
        &self,
        _workspace_id: Option<&str>,
        _cwd: Option<&str>,
    ) -> Result<(TabInfo, PaneInfo), HerdrError> {
        Err(disabled_error())
    }

    /// ワークスペース一覧を取得する（herdr では使わない）。
    pub fn workspace_list(&self) -> Result<Vec<WorkspaceInfo>, HerdrError> {
        Err(disabled_error())
    }

    /// 指定ワークスペース内のペイン一覧を取得する（herdr では使わない）。
    pub fn list_panes_in(&self, _workspace_id: &str) -> Result<Vec<PaneInfo>, HerdrError> {
        Err(disabled_error())
    }

    /// プロジェクトで作業しているワークスペースを探す（herdr では使わない）。
    pub fn find_workspace_for_project(&self, _project_root: &std::path::Path) -> Option<String> {
        None
    }

    /// 指定タブ内でペインを分割する（herdr では使わない）。
    pub fn split_pane_in_tab(
        &self,
        _tab_id: &str,
        _direction: SplitDirection,
        _cwd: Option<&str>,
    ) -> Result<PaneInfo, HerdrError> {
        Err(disabled_error())
    }

    /// 指定されたペインを基準に分割する（herdr では使わない）。
    pub fn split_pane_from(
        &self,
        _pane_id: &str,
        _direction: SplitDirection,
        _cwd: Option<&str>,
    ) -> Result<PaneInfo, HerdrError> {
        Err(disabled_error())
    }

    /// セッションスナップショットからレイアウトを取得する（herdr では使わない）。
    pub fn snapshot_layout(&self, _tab_id: &str) -> Result<Vec<LayoutPane>, HerdrError> {
        Err(disabled_error())
    }

    /// 指定されたタブを閉じる（herdr では使わない）。
    pub fn close_tab(&self, _tab_id: &str) -> Result<(), HerdrError> {
        Err(disabled_error())
    }

    /// 擬似 pane を閉じる（`stop` の印を作る）。
    pub fn close_pane(&self, pane_id: &str) -> Result<(), HerdrError> {
        if crate::headless::is_headless_pane(pane_id) {
            return crate::headless::close(pane_id);
        }
        Err(disabled_error())
    }

    /// 擬似 pane の最近の出力を読む（`stream.jsonl` の末尾）。
    pub fn read_pane_output(&self, pane_id: &str, lines: u32) -> Result<String, HerdrError> {
        if crate::headless::is_headless_pane(pane_id) {
            return crate::headless::read_output(pane_id, lines);
        }
        Err(disabled_error())
    }

    /// ペインにラベルを設定する（herdr を呼ばず、合成した `PaneInfo` を返す）。
    pub fn rename_pane(&self, pane_id: &str, label: &str) -> Result<PaneInfo, HerdrError> {
        Ok(synthetic_pane(pane_id, Some(label)))
    }

    /// ペインのラベルをクリアする（herdr を呼ばず、合成した `PaneInfo` を返す）。
    pub fn clear_pane_label(&self, pane_id: &str) -> Result<PaneInfo, HerdrError> {
        Ok(synthetic_pane(pane_id, None))
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
