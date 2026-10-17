//! # cc（tmux）の擬似 pane の共通部分
//!
//! cc を擬似 pane として扱う。窓口は [`crate::pane`]（型 `PaneInfo` と `PaneError` もそこにある）。
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
use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::pane::{PaneError, PaneInfo};

/// 実行中の pane_id を控えるファイル名（run セッション dir 直下・orchestrator が書く）。
pub const PANE_ID_FILE: &str = "herdr_pane_id";

/// run セッション dir に控えた pane_id（DB に未記録の実行中用・無ければ None）。
///
/// `agent-link run send` / `run interrupt` が実行中の run の送り先を探すのにも使う
/// （共有契約 C2）。
pub(crate) fn pane_id_from_session_dir(exec: &crate::db::models::ExecutionHistory) -> Option<String> {
    let dir = crate::session::resolve_session_dir(exec.session_dir.as_deref(), exec.id?)?;
    let id = std::fs::read_to_string(dir.join(PANE_ID_FILE)).ok()?;
    let id = id.trim();
    (!id.is_empty()).then(|| id.to_string())
}

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
    /// `working` / `idle` / `blocked`（質問の答え待ち）/ `stopped`（tmux 経路は `starting` = 起動中）
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
pub fn new_pane(cwd: Option<&str>) -> Result<PaneInfo, PaneError> {
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
        updated_at: crate::common::now_iso(),
    };
    write_state(&dir, &state)?;
    Ok(pane_info(&id, cwd, "idle"))
}

/// 起動コマンドを書き、cc を tmux で起こす。
pub fn run(id: &str, command: &str) -> Result<(), PaneError> {
    crate::tmux::run(id, command)
}

/// 入力を送る（tmux の pane だけ。印の無い擬似 pane は動かせない）。
pub fn send_text(id: &str, text: &str) -> Result<(), PaneError> {
    if crate::tmux::is_tmux(id) {
        return crate::tmux::send_text(id, text);
    }
    Err(pane_gone(id))
}

/// キーを送る（tmux の pane だけ）。
pub fn send_keys(id: &str, keys: &str) -> Result<(), PaneError> {
    if crate::tmux::is_tmux(id) {
        return crate::tmux::send_keys(id, keys);
    }
    Err(pane_gone(id))
}

/// 状態を読み、`PaneInfo` を返す。
///
/// `stopped` と、置き場が無い・`state.json` が読めないときは
/// `Err`（pane が消えた扱い）。
pub fn status(id: &str) -> Result<PaneInfo, PaneError> {
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
pub fn read_output(id: &str, lines: u32) -> Result<String, PaneError> {
    if crate::tmux::is_tmux(id) {
        return crate::tmux::read_output(id, lines);
    }
    Ok(String::new())
}

/// tmux の pane を閉じる。印の無い擬似 pane は止めるものが無い。
pub fn close(id: &str) -> Result<(), PaneError> {
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
        .map_err(|e| std::io::Error::other(format!("JSON serialization failed: {}", e)))?;
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
pub(crate) fn pane_gone(id: &str) -> PaneError {
    PaneError::ApiError {
        message: format!("pane not found: {}", id),
    }
}

/// `base`（headless の置き場）の中で、終わった tmux の pane の置き場を消し、消した数を返す。
///
/// 消すのは、名前が `hl-` で始まり、印 `tmux` があり、次のどちらかのもの:
/// - `state.json` の `state` が `stopped`
/// - `live` に名前が無い（tmux の session が無い）
/// 印の無い置き場・読めない物には触らない。消せなかった物は数えない。
pub fn sweep_dead_panes_in(base: &Path, live: &HashSet<String>) -> usize {
    let Ok(entries) = std::fs::read_dir(base) else {
        return 0;
    };
    let mut n = 0;
    for entry in entries.flatten() {
        let dir = entry.path();
        let Some(id) = entry.file_name().to_str().map(String::from) else {
            continue;
        };
        if !is_headless_pane(&id) || !dir.join("tmux").is_file() {
            continue;
        }
        let stopped = read_state(&dir).is_some_and(|s| s.state == "stopped");
        if (stopped || !live.contains(&id)) && std::fs::remove_dir_all(&dir).is_ok() {
            n += 1;
        }
    }
    n
}

/// 今のプロジェクトの置き場を掃除する。`tmux::live_sessions()` が `None` なら何もしないで 0。
pub fn sweep_dead_panes() -> usize {
    match crate::tmux::live_sessions() {
        Some(live) => sweep_dead_panes_in(&crate::project::sai_path(".agent-link/engine/headless"), &live),
        None => 0,
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

    fn mk(base: &Path, id: &str, mark: bool, state: Option<&str>) {
        let d = base.join(id);
        std::fs::create_dir_all(&d).unwrap();
        if mark {
            std::fs::write(d.join("tmux"), b"").unwrap();
        }
        if let Some(st) = state {
            let s = HeadlessState {
                pane_id: id.into(),
                runner: "cc".into(),
                state: st.into(),
                turn: 0,
                partial: String::new(),
                error: String::new(),
                updated_at: String::new(),
            };
            write_state(&d, &s).unwrap();
        }
    }

    #[test]
    fn sweep_removes_only_dead_marked_panes() {
        let t = tempfile::tempdir().unwrap();
        let b = t.path();
        mk(b, "hl-aaaaaaaaaaaa", true, Some("idle"));
        mk(b, "hl-bbbbbbbbbbbb", true, Some("stopped"));
        mk(b, "hl-cccccccccccc", true, Some("working"));
        mk(b, "hl-dddddddddddd", false, Some("stopped"));
        mk(b, "other", true, Some("idle"));
        let live: HashSet<String> = ["hl-bbbbbbbbbbbb", "hl-cccccccccccc"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(sweep_dead_panes_in(b, &live), 2);
        assert!(!b.join("hl-aaaaaaaaaaaa").exists());
        assert!(!b.join("hl-bbbbbbbbbbbb").exists());
        assert!(b.join("hl-cccccccccccc").exists());
        assert!(b.join("hl-dddddddddddd").exists());
        assert!(b.join("other").exists());
    }
}
