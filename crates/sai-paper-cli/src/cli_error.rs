//! CLI 共通の約束の終了コードを持つエラー（`crates/sai-paper-cli/src/cli_error.rs`）。
//!
//! 約束の終了コード（2・3・4）を `anyhow` のエラーとして運ぶ。`main` の終わりで
//! `downcast_ref::<CliError>()` を見て終了コードを決める。これ以外のエラーは 1 になる。

use std::fmt;

/// 約束の終了コード（2・3・4）を持つエラー。これ以外のエラーは 1 になる。
#[derive(Debug)]
pub struct CliError {
    /// 終了コード（2 = 入力の誤り・3 = 見つからない・4 = 状態の衝突）。
    pub code: i32,
    /// 標準エラーの最後の行に出す理由。
    pub message: String,
}

impl CliError {
    /// 入力の誤り（終了コード 2）。引数・値・標準入力の JSON の誤り。
    pub fn input(m: impl Into<String>) -> Self {
        Self { code: 2, message: m.into() }
    }

    /// 見つからない（終了コード 3）。run・カード・トークン・論文・定義など。
    pub fn not_found(m: impl Into<String>) -> Self {
        Self { code: 3, message: m.into() }
    }

    /// 状態の衝突（終了コード 4）。実行中・同じ名前がある・既に初期化済みなど。
    #[allow(dead_code)] // 終了コード 4 の約束として残す。
    pub fn conflict(m: impl Into<String>) -> Self {
        Self { code: 4, message: m.into() }
    }
}

impl fmt::Display for CliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for CliError {}
