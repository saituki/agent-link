//! # 実行主体（runner）の抽象化
//!
//! ブロックを実行するエージェント CLI を表す [`RunnerKind`] を提供します。
//!
//! - `pi`   — pi コーディングエージェント（既存・既定）
//! - `cc`   — Claude Code（`cc` コマンド。ユーザー環境で `cc` を shim する前提）
//! - `gl`   — sai-gemini-tui（`gl` コマンド。`~/.local/bin/gl` ラッパー経由で起動）
//!
//! runner は kanban カード frontmatter の `runner:` で指定され、
//! `ExecutionOverrides` → DB `execution_history.runner` → 起動コマンド構築 →
//! プロンプト送信キー、まで一貫して伝播します。
//!
//! 後方互換: 未指定・未知の値は `Pi` として扱います。

use serde::{Deserialize, Serialize};

/// ブロックを実行するエージェント CLI の種別。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum RunnerKind {
    /// pi コーディングエージェント（既定）。
    #[default]
    Pi,
    /// Claude Code（`cc` コマンド）。
    Cc,
    /// sai-gemini-tui（`gl` コマンド）。
    ///
    /// 起動時のタスク書き込みなし（空セッションを開くだけ）。
    /// `~/.sai-gemini/sessions/*.jsonl` を解析して表示する。
    Gl,
}

impl RunnerKind {
    /// カード frontmatter / DB の文字列から解決する。
    ///
    /// - `"cc"` / `"claude"` / `"claude-code"` → [`RunnerKind::Cc`]
    /// - `"gl"` / `"gemini"` → [`RunnerKind::Gl`]
    /// - `"pi"` / 空 / 未知 → [`RunnerKind::Pi`]（後方互換）
    pub fn parse(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "cc" | "claude" | "claude-code" | "claudecode" => RunnerKind::Cc,
            "gl" | "gemini" | "sai-gemini" => RunnerKind::Gl,
            _ => RunnerKind::Pi,
        }
    }

    /// DB に保存する正規文字列を返す（`"pi"` / `"cc"` / `"gl"`）。
    pub fn as_str(self) -> &'static str {
        match self {
            RunnerKind::Pi => "pi",
            RunnerKind::Cc => "cc",
            RunnerKind::Gl => "gl",
        }
    }

    /// 起動する CLI のコマンド名。
    ///
    /// `Cc` は `claude`（`SAI_CLAUDE_BIN` があればそれを使う。
    /// Linux の `cc` は C コンパイラなので shim を置かない）。
    pub fn bin(self) -> &'static str {
        match self {
            RunnerKind::Pi => "pi",
            RunnerKind::Cc => "claude",
            RunnerKind::Gl => "gl",
        }
    }

    /// プロンプト本文を確定送信するキー。
    ///
    /// pi・Claude Code はマルチライン入力を `Alt+Enter` で確定する
    /// （`~/.claude/keybindings.json` で `enter` = 改行 / `alt+enter` = 送信 に
    /// 割り当てられているユーザー環境を前提とする）。
    /// gl は起動時のタスク書き込みなしのため送信自体を行わない。
    pub fn submit_key(self) -> &'static str {
        match self {
            RunnerKind::Pi | RunnerKind::Cc => "Alt+Enter",
            RunnerKind::Gl => "Enter",
        }
    }

    /// Claude Code かどうか。
    pub fn is_claude(self) -> bool {
        matches!(self, RunnerKind::Cc)
    }

    /// タスク書き込みなしで起動するかどうか（gl のみ真）。
    ///
    /// gl は空セッションを開くだけで、プロンプト送信・タスク登録を行わない。
    pub fn is_no_task_write(self) -> bool {
        matches!(self, RunnerKind::Gl)
    }
}

impl std::fmt::Display for RunnerKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

// ============================================================================
// テスト
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_known_values() {
        assert_eq!(RunnerKind::parse("pi"), RunnerKind::Pi);
        assert_eq!(RunnerKind::parse("PI"), RunnerKind::Pi);
        assert_eq!(RunnerKind::parse("cc"), RunnerKind::Cc);
        assert_eq!(RunnerKind::parse(" claude "), RunnerKind::Cc);
        assert_eq!(RunnerKind::parse("claude-code"), RunnerKind::Cc);
    }

    #[test]
    fn parse_unknown_defaults_to_pi() {
        assert_eq!(RunnerKind::parse(""), RunnerKind::Pi);
        assert_eq!(RunnerKind::parse("-"), RunnerKind::Pi);
        assert_eq!(RunnerKind::parse("gl"), RunnerKind::Gl);
        assert_eq!(RunnerKind::parse("GL"), RunnerKind::Gl);
        assert_eq!(RunnerKind::parse("gemini"), RunnerKind::Gl);
        assert_eq!(RunnerKind::parse("sai-gemini"), RunnerKind::Gl);
    }

    #[test]
    fn submit_key_differs() {
        assert_eq!(RunnerKind::Pi.submit_key(), "Alt+Enter");
        assert_eq!(RunnerKind::Cc.submit_key(), "Alt+Enter");
        assert_eq!(RunnerKind::Gl.submit_key(), "Enter");
    }

    #[test]
    fn serde_roundtrip_and_default() {
        assert_eq!(serde_json::to_string(&RunnerKind::Cc).unwrap(), "\"cc\"");
        assert_eq!(serde_json::to_string(&RunnerKind::Pi).unwrap(), "\"pi\"");
        assert_eq!(serde_json::to_string(&RunnerKind::Gl).unwrap(), "\"gl\"");
        // フィールド欠落は Pi（後方互換）
        let missing: Result<RunnerKind, _> = serde_json::from_str("null");
        assert!(missing.is_err() || missing.unwrap() == RunnerKind::Pi);
    }

    #[test]
    fn display_matches_as_str() {
        assert_eq!(RunnerKind::Pi.to_string(), "pi");
        assert_eq!(RunnerKind::Cc.to_string(), "cc");
        assert_eq!(RunnerKind::Gl.to_string(), "gl");
    }

    #[test]
    fn gl_is_no_task_write_only() {
        assert!(RunnerKind::Gl.is_no_task_write());
        assert!(!RunnerKind::Pi.is_no_task_write());
        assert!(!RunnerKind::Cc.is_no_task_write());
    }
}
