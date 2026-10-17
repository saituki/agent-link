//! 実行 1 件の会話の記録（jsonl）の場所を解決する。
//!
//! 書き出し・検索・履歴・費用の集計が、同じ解決を使う。

use std::path::{Path, PathBuf};

use crate::db::models::ExecutionHistory;

/// 会話記録 jsonl の絶対パスを解決する。
///
/// - `agent_session_id` があれば `session::claude_session_file(cwd, agent_session_id)`。
///   cwd はプロジェクトルート（worktree 実行なら `.worktrees/<branch>` も探索）。
///   それでも無ければセッション ID だけで探す（`claude_session_file_any_cwd`）。
///   **`cc_session_files(...)`（全体で一番新しい cc セッション）は使わない**
///   （別の実行のファイルを拾うことがあるため）。
/// - `agent_session_id` が無い古い記録は `session_dir` の主セッションを使う。
pub fn resolve_transcript(exec: &ExecutionHistory, project_root: &Path) -> Option<PathBuf> {
    let Some(sid) = exec.agent_session_id.as_deref() else {
        let run_id = exec.id.unwrap_or(0);
        let dir = crate::session::resolve_session_dir(exec.session_dir.as_deref(), run_id)?;
        return crate::session::find_main_session_file(&dir).ok();
    };
    if let Some(p) = crate::session::claude_session_file(project_root, sid) {
        return Some(p);
    }
    // worktree 実行時は cwd が `.worktrees/<branch>` になるため追加探索
    let wt = project_root.join(".worktrees");
    if let Ok(entries) = std::fs::read_dir(&wt) {
        for dir in entries.flatten().map(|e| e.path()).filter(|p| p.is_dir()) {
            if let Some(p) = crate::session::claude_session_file(&dir, sid) {
                return Some(p);
            }
        }
    }
    // worktree を消したあとなど cwd が分からないときは、セッション ID で探す
    crate::session::claude_session_file_any_cwd(sid)
}
