//! 新しい命令の木（`card`・`run`・`doc`・`task`・`def`・`project`・`session`・
//! `workflow`・`schema`）で共通に使う、終了コードと失敗の扱い。
//!
//! CLI 共通の約束（sai-link・sai-moon-paper・sai-sci で同じ）:
//!
//! | コード | 意味 |
//! |---|---|
//! | 0 | 成功 |
//! | 2 | 入力の誤り（引数・値・標準入力。clap の使い方の誤りも 2） |
//! | 3 | 見つからない |
//! | 4 | 状態の衝突（実行中・同じ名前がある・既に…） |
//! | 1 | その他の失敗（DB・ファイル・ネットワーク） |
//!
//! 失敗したときは標準出力に何も書かず、標準エラーの最後の行に日本語 1 行を出す。

use sai_link_engine::common as engine_common;

/// 成功。
#[allow(dead_code)]
pub(crate) const EXIT_OK: i32 = 0;
/// 入力の誤り（引数・値・標準入力。clap の使い方の誤りもここ）。
pub(crate) const EXIT_INPUT: i32 = 2;
/// 見つからない（run・カード・トークン・定義など）。
pub(crate) const EXIT_MISSING: i32 = 3;
/// 状態の衝突（実行中・同じ名前がある・既に…）。
#[allow(dead_code)]
pub(crate) const EXIT_CONFLICT: i32 = 4;
/// その他の失敗（DB・ファイル・ネットワーク・外の道具）。
#[allow(dead_code)]
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
    #[allow(dead_code)]
    pub(crate) fn missing(msg: impl Into<String>) -> Self {
        Self {
            code: EXIT_MISSING,
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

/// 失敗を標準エラーに 1 行出して終了する（標準出力には何も書かない）。
pub(crate) fn exit(e: CmdError) -> ! {
    eprintln!("{}", e.msg);
    std::process::exit(e.code);
}

impl From<crate::phone::PhoneError> for CmdError {
    /// `phone` の終了コードを CLI 共通の約束の値に読み替える。
    ///
    /// usage(1)=入力の誤り(2)・parse(2)=入力の誤り(2)・not_found(3)=見つからない(3)、
    /// internal(4)=その他(1)。
    fn from(e: crate::phone::PhoneError) -> Self {
        let code = if e.code == engine_common::EXIT_PARSE || e.code == engine_common::EXIT_USAGE {
            EXIT_INPUT
        } else if e.code == engine_common::EXIT_NOT_FOUND {
            EXIT_MISSING
        } else {
            EXIT_OTHER
        };
        Self { code, msg: e.msg }
    }
}

/// `agent_api` の `Err(String)` を終了コードに分類する。
///
/// // ponytail: 文で分類。agent_api がエラーの型を持ったらそれに替える
pub(crate) fn classify_agent_error(msg: &str) -> i32 {
    if msg.contains("見つかりません") || msg.contains("存在しません") || msg.contains("解決に失敗") {
        EXIT_MISSING
    } else if msg.contains("実行中") || msg.contains("既に") {
        EXIT_CONFLICT
    } else if msg.contains("必須")
        || msg.contains("パース")
        || msg.contains("不正")
        || msg.contains("JSON")
        || msg.contains("配列")
        || msg.contains("空")
        || msg.contains("指定してください")
    {
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
        .map_err(|e| CmdError::other(format!("JSON 変換に失敗しました: {}", e)))?;
    println!("{}", text);
    Ok(())
}

/// `phone` の実装をそのまま呼び、出力を 1 文字も変えずに出す。
pub(crate) fn print_phone(cmd: crate::cli::PhoneCmd) -> Result<(), CmdError> {
    let text = crate::phone::render(cmd)?;
    println!("{}", text);
    Ok(())
}

/// `Box<dyn Error>` を返す古いハンドラの結果を、契約の終了コードへ読み替える。
pub(crate) fn other_error(e: impl std::fmt::Display) -> CmdError {
    CmdError::other(e.to_string())
}
