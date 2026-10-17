//! # 擬似 pane の窓口
//!
//! 擬似 pane（id が `hl-` で始まる）の操作を [`crate::headless`] に回す。
//! 印 `tmux` がある pane は [`crate::tmux`] が動かす。
//!
//! herdr（端末の多重化の道具）は使わない。pane id を入れる環境変数と
//! DB の列の名前は、昔の名前（herdr）のまま残している。

use serde::{Deserialize, Serialize};

/// pane の分割方向（`def preview --direction` の値）。
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

/// pane の状態。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct PaneInfo {
    /// pane の一意識別子
    pub pane_id: String,
    /// pane が所属するワークスペースの ID
    pub workspace_id: String,
    /// pane が所属するタブの ID
    pub tab_id: String,
    /// pane が現在フォーカスされているかどうか
    pub focused: bool,
    /// pane のカレントワーキングディレクトリ（未設定の場合は None）
    pub cwd: Option<String>,
    /// pane で動作しているエージェント名（未設定の場合は None）
    pub agent: Option<String>,
    /// エージェントのステータス（"idle", "working", "done", "blocked" 等）
    pub agent_status: String,
    /// pane に設定されたラベル（未設定の場合は None）
    pub label: Option<String>,
    /// pane 情報のリビジョン番号
    pub revision: i64,
}

/// pane 操作のエラー。
#[derive(Debug, thiserror::Error)]
pub enum PaneError {
    /// JSON のデシリアライズに失敗
    #[error("JSON parse error: {0}")]
    JsonParseError(#[from] serde_json::Error),

    /// I/O エラー（プロセス起動失敗等）
    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),

    /// UTF-8 文字列への変換に失敗
    #[error("UTF-8 conversion error: {0}")]
    Utf8Error(#[from] std::string::FromUtf8Error),

    /// pane の操作が失敗した（メッセージ付き）
    #[error("pane API error: {message}")]
    ApiError {
        /// エラーメッセージ
        message: String,
    },
}

/// 指定 WIP で 1-n pane の均等レイアウト（列数, 行数）を計算する。
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

/// 擬似 pane でない id を受けたときに返すエラー。
fn disabled_error() -> PaneError {
    PaneError::ApiError {
        message: "This pane is not an agent-link pseudo pane.".to_string(),
    }
}

/// ラベル操作用の合成 `PaneInfo`。
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

/// 擬似 pane の窓口。内部状態を持たない（`Send + Sync`）。
#[derive(Default)]
pub struct PaneClient;

impl PaneClient {
    /// 新しい `PaneClient` を作成する。
    pub fn new() -> Self {
        Self
    }

    /// 擬似 pane に起動コマンドを入れる。
    pub fn run_in_pane(&self, pane_id: &str, command: &str) -> Result<(), PaneError> {
        if crate::headless::is_headless_pane(pane_id) {
            return crate::headless::run(pane_id, command);
        }
        Err(disabled_error())
    }

    /// 擬似 pane にテキストを送る（`draft.txt` に上書き）。
    pub fn send_text(&self, pane_id: &str, text: &str) -> Result<(), PaneError> {
        if crate::headless::is_headless_pane(pane_id) {
            return crate::headless::send_text(pane_id, text);
        }
        Err(disabled_error())
    }

    /// 擬似 pane にキーを送る（`esc` は中断、確定キーは `inbox/` へ）。
    pub fn send_keys(&self, pane_id: &str, keys: &str) -> Result<(), PaneError> {
        if crate::headless::is_headless_pane(pane_id) {
            return crate::headless::send_keys(pane_id, keys);
        }
        Err(disabled_error())
    }

    /// 擬似 pane の状態を読む（`state.json`）。
    pub fn get_pane_status(&self, pane_id: &str) -> Result<PaneInfo, PaneError> {
        if crate::headless::is_headless_pane(pane_id) {
            return crate::headless::status(pane_id);
        }
        Err(disabled_error())
    }

    /// 擬似 pane を閉じる（`stop` の印を作る）。
    pub fn close_pane(&self, pane_id: &str) -> Result<(), PaneError> {
        if crate::headless::is_headless_pane(pane_id) {
            return crate::headless::close(pane_id);
        }
        Err(disabled_error())
    }

    /// 擬似 pane の最近の出力を読む（`stream.jsonl` の末尾）。
    pub fn read_pane_output(&self, pane_id: &str, lines: u32) -> Result<String, PaneError> {
        if crate::headless::is_headless_pane(pane_id) {
            return crate::headless::read_output(pane_id, lines);
        }
        Err(disabled_error())
    }

    /// pane にラベルを設定する（合成した `PaneInfo` を返すだけ）。
    pub fn rename_pane(&self, pane_id: &str, label: &str) -> Result<PaneInfo, PaneError> {
        Ok(synthetic_pane(pane_id, Some(label)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_direction_display() {
        assert_eq!(SplitDirection::Right.to_string(), "right");
        assert_eq!(SplitDirection::Down.to_string(), "down");
    }

    #[test]
    fn non_pseudo_pane_operations_are_disabled() {
        let client = PaneClient::new();
        let results = [
            client.run_in_pane("pane_test_001", "echo hi"),
            client.send_text("pane_test_001", "hello"),
            client.send_keys("pane_test_001", "Enter"),
            client.close_pane("pane_test_001"),
        ];
        for r in results {
            match r {
                Err(PaneError::ApiError { message }) => {
                    assert_eq!(message, "This pane is not an agent-link pseudo pane.")
                }
                other => panic!("ApiError を期待: {:?}", other),
            }
        }
        assert!(matches!(
            client.get_pane_status("pane_test_001"),
            Err(PaneError::ApiError { .. })
        ));
        assert!(matches!(
            client.read_pane_output("pane_test_001", 50),
            Err(PaneError::ApiError { .. })
        ));
    }

    #[test]
    fn rename_returns_synthetic_pane() {
        let renamed = PaneClient::new().rename_pane("pane_test_001", "my-label").unwrap();
        assert_eq!(renamed.pane_id, "pane_test_001");
        assert_eq!(renamed.label.as_deref(), Some("my-label"));
    }

    #[test]
    fn layout_for_wip_columns_and_rows() {
        assert_eq!(layout_for_wip(0, 8), (0, 0));
        assert_eq!(layout_for_wip(3, 8), (3, 1));
        assert_eq!(layout_for_wip(5, 8), (3, 2));
        assert_eq!(layout_for_wip(8, 8), (4, 2));
    }
}
