//! エラー型定義
//!
//! ブロック定義ファイルのパース・バリデーションにおけるエラー型を提供します。
//! `thiserror` クレートを使用して `std::error::Error` を自動導出しています。

use thiserror::Error;

/// ブロック定義ファイルのパース・バリデーションエラー
///
/// ファイル読み込み、YAML パース、必須フィールドの欠落、
/// 値制約違反など、ブロック定義ファイルの処理における
/// 全てのエラーを表現します。
#[derive(Error, Debug, Clone)]
pub enum ParseError {
    /// ファイル読み込みエラー
    ///
    /// 指定されたパスのファイルが存在しない、読み取り権限がない、
    /// または I/O エラーが発生した場合。
    #[error("ファイル読み込みエラー: {0}")]
    IoError(String),

    /// YAML パースエラー
    ///
    /// フロントマターの YAML 書式が不正で、`serde_yaml` による
    /// デシリアライズに失敗した場合。
    #[error("YAML パースエラー: {0}")]
    YamlError(String),

    /// フロントマターが見つからない
    ///
    /// ファイルの先頭が `---` で始まっていない場合。
    /// ブロック定義ファイルは必ず YAML frontmatter を持つ必要があります。
    #[error("フロントマターが見つかりません: ファイルは `---` で始まる必要があります")]
    MissingFrontmatter,

    /// 閉じ `---` がない
    ///
    /// 開始 `---` 以降で閉じ `---` が見つからない場合。
    #[error("フロントマターの閉じ `---` が見つかりません")]
    UnclosedFrontmatter,

    /// 必須フィールド欠落
    ///
    /// `name`、`description`、`model` などの必須フィールドが
    /// 設定されていないか、空文字列の場合。
    #[error("必須フィールド '{0}' が設定されていません")]
    MissingField(String),

    /// 値制約違反
    ///
    /// フィールドの値が許可された値の範囲や列挙値に含まれていない場合。
    #[error("フィールド '{field}' の値 '{value}' が無効です: {expected}")]
    InvalidValue {
        /// フィールド名
        field: String,
        /// 実際の値
        value: String,
        /// 期待される値の説明
        expected: String,
    },

    /// 不正な thinking レベル
    ///
    /// `thinking` フィールドに `"none"`, `"low"`, `"medium"`,
    /// `"high"`, `"xhigh"`, `"max"` 以外の値が指定された場合
    /// （旧値 `off` / `minimal` もここに含まれる）。
    #[error("thinking の値 '{0}' は無効です。'none', 'low', 'medium', 'high', 'xhigh', 'max' のいずれかを指定してください")]
    InvalidThinkingLevel(String),

    /// 警告（バリデーション警告用）
    ///
    /// ブロックの実行自体は可能だが、注意を促すべき状態
    /// （例: `enable: false`）。
    #[error("警告: {0}")]
    Warning(String),

    /// テンプレートファイルの読み込みエラー
    ///
    /// `templates` に指定された `.agent-link/template/*.md` が存在しない、
    /// 読み取り権限がない、または I/O エラーが発生した場合。
    #[error("テンプレート '{name}' の読み込みに失敗: {message}")]
    TemplateError {
        /// テンプレートファイル名（例: "tmp1.md"）
        name: String,
        /// 失敗の詳細（I/O エラーメッセージとパス）
        message: String,
    },
}

impl From<std::io::Error> for ParseError {
    fn from(err: std::io::Error) -> Self {
        ParseError::IoError(err.to_string())
    }
}

impl From<serde_yaml::Error> for ParseError {
    fn from(err: serde_yaml::Error) -> Self {
        ParseError::YamlError(err.to_string())
    }
}
