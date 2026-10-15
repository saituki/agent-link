//! # cc（tmux）の擬似 pane の共通部分
//!
//! sai-tree では herdr の pane を使わず、cc を擬似 pane として扱う。
//! cc は tmux の対話モードで動かし、置き場に印 `tmux` がある pane の操作は
//! [`crate::tmux`] へ回す（`run` / `send_text` / `send_keys` / `status` / `read_output` / `close`）。
//! pi の子プロセス（監督スレッド）は U04 で消した。
//!
//! ## 置き場
//!
//! `<project_root>/.agent-link/engine/headless/<id>/`
//!
//! - `cwd.txt` — 作業ディレクトリ
//! - `state.json` — 共有契約 C1 の状態（書くのは tmux の経路）

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::herdr::{HerdrError, PaneInfo};

/// 擬似 pane の id 接頭辞。
pub const HEADLESS_PREFIX: &str = "hl-";

/// 擬似 pane の id かどうか。
pub fn is_headless_pane(id: &str) -> bool {
    id.starts_with(HEADLESS_PREFIX)
}

/// 擬似 pane の置き場。
pub fn pane_dir(id: &str) -> PathBuf {
    crate::project::sai_path(".agent-link/engine/headless").join(id)
}

/// `state.json` の形（共有契約 C1）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HeadlessState {
    /// 擬似 pane の id（`hl-` + 12 桁の 16 進）
    pub pane_id: String,
    /// 実行主体（`cc`。新しい pane は空）
    pub runner: String,
    /// `working` / `idle` / `stopped`（tmux 経路は `starting` = 起動中）
    pub state: String,
    /// ターン番号
    pub turn: u32,
    /// 生成中の文（無ければ空）
    pub partial: String,
    /// 直前のターンが失敗したときの理由 1 行（無ければ空）
    pub error: String,
    /// 更新時刻（ISO 8601）
    pub updated_at: String,
}

/// 新しい擬似 pane を作る。
pub fn new_pane(cwd: Option<&str>) -> Result<PaneInfo, HerdrError> {
    let hex: String = crate::cc_command::generate_uuid_v4()
        .chars()
        .filter(|c| c.is_ascii_hexdigit())
        .take(12)
        .collect();
    let id = format!("{HEADLESS_PREFIX}{hex}");
    let dir = pane_dir(&id);
    std::fs::create_dir_all(&dir)?;
    if let Some(c) = cwd {
        std::fs::write(dir.join("cwd.txt"), c)?;
    }
    let state = HeadlessState {
        pane_id: id.clone(),
        runner: String::new(),
        state: "idle".to_string(),
        turn: 0,
        partial: String::new(),
        error: String::new(),
        updated_at: crate::phone_export::now_iso(),
    };
    write_state(&dir, &state)?;
    Ok(pane_info(&id, cwd, "idle"))
}

/// 起動コマンドを書き、cc を tmux で起こす。
pub fn run(id: &str, command: &str) -> Result<(), HerdrError> {
    crate::tmux::run(id, command)
}

/// 入力を送る（tmux の pane だけ。印の無い擬似 pane は動かせない）。
pub fn send_text(id: &str, text: &str) -> Result<(), HerdrError> {
    if crate::tmux::is_tmux(id) {
        return crate::tmux::send_text(id, text);
    }
    Err(pane_gone(id))
}

/// キーを送る（tmux の pane だけ）。
pub fn send_keys(id: &str, keys: &str) -> Result<(), HerdrError> {
    if crate::tmux::is_tmux(id) {
        return crate::tmux::send_keys(id, keys);
    }
    Err(pane_gone(id))
}

/// 状態を読み、`PaneInfo` を返す。
///
/// `stopped` と、置き場が無い・`state.json` が読めないときは
/// `Err`（pane が消えた扱い）。
pub fn status(id: &str) -> Result<PaneInfo, HerdrError> {
    if crate::tmux::is_tmux(id) {
        return crate::tmux::status(id);
    }
    let dir = pane_dir(id);
    if !dir.is_dir() {
        return Err(pane_gone(id));
    }
    let state = read_state(&dir).ok_or_else(|| pane_gone(id))?;
    if state.state == "stopped" {
        return Err(pane_gone(id));
    }
    let cwd = read_cwd(&dir);
    Ok(pane_info(id, cwd.as_deref(), &state.state))
}

/// tmux の pane の末尾 `lines` 行を返す。印の無い擬似 pane は出力が無い。
pub fn read_output(id: &str, lines: u32) -> Result<String, HerdrError> {
    if crate::tmux::is_tmux(id) {
        return crate::tmux::read_output(id, lines);
    }
    Ok(String::new())
}

/// tmux の pane を閉じる。印の無い擬似 pane は止めるものが無い。
pub fn close(id: &str) -> Result<(), HerdrError> {
    if crate::tmux::is_tmux(id) {
        return crate::tmux::close(id);
    }
    Ok(())
}

// ============================================================================
// 内部: ファイル操作
// ============================================================================

/// `state.json` を読む。
pub(crate) fn read_state(dir: &Path) -> Option<HeadlessState> {
    let text = std::fs::read_to_string(dir.join("state.json")).ok()?;
    serde_json::from_str(&text).ok()
}

/// `cwd.txt` を読む。
pub(crate) fn read_cwd(dir: &Path) -> Option<String> {
    std::fs::read_to_string(dir.join("cwd.txt"))
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// `state.json` を一時ファイル → rename で書く。
pub(crate) fn write_state(dir: &Path, state: &HeadlessState) -> std::io::Result<()> {
    let text = serde_json::to_string_pretty(state)
        .map_err(|e| std::io::Error::other(format!("JSON 直列化失敗: {}", e)))?;
    let tmp = dir.join(".state.json.tmp");
    let dst = dir.join("state.json");
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, &dst)?;
    Ok(())
}

/// `PaneInfo` を組み立てる。
pub(crate) fn pane_info(id: &str, cwd: Option<&str>, status: &str) -> PaneInfo {
    PaneInfo {
        pane_id: id.to_string(),
        workspace_id: "headless".to_string(),
        tab_id: "headless".to_string(),
        focused: false,
        cwd: cwd.map(|s| s.to_string()),
        agent: None,
        agent_status: status.to_string(),
        label: None,
        revision: 0,
    }
}

/// pane が消えたときのエラー。
pub(crate) fn pane_gone(id: &str) -> HerdrError {
    HerdrError::ApiError {
        message: format!("pane が見つかりません: {}", id),
    }
}

// ============================================================================
// テスト
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_headless_pane_prefix() {
        assert!(is_headless_pane("hl-0123456789ab"));
        assert!(!is_headless_pane("pane_test_001"));
    }
}
