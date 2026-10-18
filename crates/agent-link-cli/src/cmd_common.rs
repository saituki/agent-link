//! 新しい命令の木（`card`・`run`・`doc`・`task`・`def`・`project`・`session`・
//! `workflow`・`schema`）で共通に使う、終了コードと失敗の扱い。
//!
//! CLI 共通の約束（agent-link で守る）:
//!
//! | コード | 意味 |
//! |---|---|
//! | 0 | 成功 |
//! | 2 | 入力の誤り（引数・値・標準入力。clap の使い方の誤りも 2） |
//! | 3 | 見つからない |
//! | 4 | 状態の衝突（実行中・同じ名前がある・既に…） |
//! | 1 | その他の失敗（DB・ファイル・ネットワーク） |
//!
//! 失敗したときは標準出力に何も書かず、標準エラーの最後の行に英語 1 行を出す。

use agent_link_engine::common as engine_common;

/// 入力の誤り（引数・値・標準入力。clap の使い方の誤りもここ）。
pub(crate) const EXIT_INPUT: i32 = 2;
/// 見つからない（run・カード・トークン・定義など）。
pub(crate) const EXIT_MISSING: i32 = 3;
/// 状態の衝突（実行中・同じ名前がある・既に…）。
pub(crate) const EXIT_CONFLICT: i32 = 4;
/// その他の失敗（DB・ファイル・ネットワーク・外の道具）。
pub(crate) const EXIT_OTHER: i32 = 1;

/// 新しい木の失敗（終了コード + 日本語 1 行）。
#[derive(Debug)]
pub(crate) struct CmdError {
    pub(crate) code: i32,
    pub(crate) msg: String,
}

impl CmdError {
    /// 入力の誤り（終了コード 2）。
    pub(crate) fn input(msg: impl Into<String>) -> Self {
        Self {
            code: EXIT_INPUT,
            msg: msg.into(),
        }
    }

    /// 見つからない（終了コード 3）。
    pub(crate) fn missing(msg: impl Into<String>) -> Self {
        Self {
            code: EXIT_MISSING,
            msg: msg.into(),
        }
    }

    /// 状態の衝突（終了コード 4）。
    pub(crate) fn conflict(msg: impl Into<String>) -> Self {
        Self {
            code: EXIT_CONFLICT,
            msg: msg.into(),
        }
    }

    /// その他の失敗（終了コード 1）。
    pub(crate) fn other(msg: impl Into<String>) -> Self {
        Self {
            code: EXIT_OTHER,
            msg: msg.into(),
        }
    }
}

/// `Box<dyn Error>` の失敗は、終了コード 1 の失敗として扱う。
impl From<Box<dyn std::error::Error>> for CmdError {
    fn from(e: Box<dyn std::error::Error>) -> Self {
        Self::other(e.to_string())
    }
}

/// 失敗を標準エラーに 1 行出して終了する（標準出力には何も書かない）。
pub(crate) fn exit(e: CmdError) -> ! {
    eprintln!("{}", e.msg);
    std::process::exit(e.code);
}

impl From<crate::views::ViewError> for CmdError {
    /// `views` の終了コードを CLI 共通の約束の値に読み替える。
    ///
    /// usage(1)=入力の誤り(2)・parse(2)=入力の誤り(2)・not_found(3)=見つからない(3)、
    /// conflict(5)=状態の衝突(4)・internal(4)=その他(1)。
    fn from(e: crate::views::ViewError) -> Self {
        let code = if e.code == engine_common::EXIT_PARSE || e.code == engine_common::EXIT_USAGE {
            EXIT_INPUT
        } else if e.code == engine_common::EXIT_NOT_FOUND {
            EXIT_MISSING
        } else if e.code == crate::views::EXIT_VIEW_CONFLICT {
            EXIT_CONFLICT
        } else {
            EXIT_OTHER
        };
        Self { code, msg: e.msg }
    }
}

/// `agent_api` の `Err(String)` を終了コードに分類する（C-ERR）。
///
/// 見つからない → 状態の衝突 → 入力の誤りの順に、語を含むかで決める（大文字小文字は区別しない）。
/// どれも含まなければ 1。
///
/// // ponytail: 文で分類。agent_api がエラーの型を持ったらそれに替える
pub(crate) fn classify_agent_error(msg: &str) -> i32 {
    let lower = msg.to_lowercase();
    let has = |words: &[&str]| words.iter().any(|w| lower.contains(w));
    if has(&["not found", "does not exist", "failed to resolve"]) {
        EXIT_MISSING
    } else if has(&["already", "is running", "is not running", "in progress"]) {
        EXIT_CONFLICT
    } else if has(&[
        "invalid", "must", "required", "is not set", "needs", "parse", "array", "empty", "specify",
        "at least", "give either",
    ]) {
        EXIT_INPUT
    } else {
        EXIT_OTHER
    }
}

/// `agent_api` の結果を、契約の終了コードを持つ [`CmdError`] に読み替える。
pub(crate) fn agent_result(
    r: Result<serde_json::Value, String>,
) -> Result<serde_json::Value, CmdError> {
    r.map_err(|msg| CmdError {
        code: classify_agent_error(&msg),
        msg,
    })
}

/// JSON を 1 つ pretty で標準出力に出す。
pub(crate) fn print_json(value: &serde_json::Value) -> Result<(), CmdError> {
    let text = serde_json::to_string_pretty(value)
        .map_err(|e| CmdError::other(format!("Failed to serialize JSON: {}", e)))?;
    println!("{}", text);
    Ok(())
}

/// `views` の実装をそのまま呼び、出力を 1 文字も変えずに出す。
pub(crate) fn print_view(cmd: crate::cli::ViewCmd) -> Result<(), CmdError> {
    let text = crate::views::render(cmd)?;
    println!("{}", text);
    Ok(())
}

/// `Box<dyn Error>` を返す古いハンドラの結果を、契約の終了コードへ読み替える。
pub(crate) fn other_error(e: impl std::fmt::Display) -> CmdError {
    CmdError::other(e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_agent_error_maps_contract_words() {
        assert_eq!(classify_agent_error("Run 3 is not running"), EXIT_CONFLICT);
        assert_eq!(classify_agent_error("Give either batch_id or task_id."), EXIT_INPUT);
        assert_eq!(classify_agent_error("Task 9 does not exist."), EXIT_MISSING);
        assert_eq!(classify_agent_error("Task is ALREADY done"), EXIT_CONFLICT);
        // 日本語の語は見ない（英語の文だけを契約として扱う）
        assert_eq!(classify_agent_error("空"), EXIT_OTHER);
        assert_eq!(classify_agent_error("見つかりません"), EXIT_OTHER);
    }
}
