//! ブロック設定構造体
//!
//! ブロック定義ファイル（Markdown + YAML frontmatter）の
//! パース結果を格納するデータ構造を定義します。
//!
//! # ブロック定義ファイルの構造
//!
//! ```yaml
//! ---
//! name: my-block
//! description: "ブロックの説明"
//! model: "anthropic/claude-sonnet-4-5"
//! thinking: high
//! tools: read,bash,edit
//! templates: tmp1,tmp2   # .agent-link/template/tmp1.md, tmp2.md を本文の先頭に挿入
//! ---
//! # Markdown 本文
//! ```

use serde::de::{self, Deserializer, Visitor};
use serde::{Deserialize, Serialize};
use std::fmt;

// ---------------------------------------------------------------------------
// ToolsConfig — ツール/拡張機能の設定
// ---------------------------------------------------------------------------

/// ツールまたは拡張機能の設定を表現する列挙型。
///
/// YAML 上では以下の形式を受け付けます:
/// - `true` → 全て有効
/// - `false` → 全て無効
/// - `"tool1,tool2"` → カンマ区切りで指定
#[derive(Debug, Clone, Serialize, PartialEq)]
pub enum ToolsConfig {
    /// 全てのツール/拡張機能を有効にする（YAML: `true`）
    All,
    /// 全てのツール/拡張機能を無効にする（YAML: `false`）
    None,
    /// 指定されたツール/拡張機能のみ有効にする（YAML: `"tool1,tool2"`）
    List(Vec<String>),
}

impl fmt::Display for ToolsConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ToolsConfig::All => write!(f, "true"),
            ToolsConfig::None => write!(f, "false"),
            ToolsConfig::List(items) => write!(f, "{}", items.join(",")),
        }
    }
}

struct ToolsConfigVisitor;

impl<'de> Visitor<'de> for ToolsConfigVisitor {
    type Value = ToolsConfig;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("真偽値（true/false）またはカンマ区切り文字列")
    }

    fn visit_bool<E: de::Error>(self, v: bool) -> Result<ToolsConfig, E> {
        Ok(if v {
            ToolsConfig::All
        } else {
            ToolsConfig::None
        })
    }

    fn visit_str<E: de::Error>(self, v: &str) -> Result<ToolsConfig, E> {
        let items: Vec<String> = v
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        if items.is_empty() {
            Ok(ToolsConfig::None)
        } else {
            Ok(ToolsConfig::List(items))
        }
    }
}

impl<'de> Deserialize<'de> for ToolsConfig {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(ToolsConfigVisitor)
    }
}

// ---------------------------------------------------------------------------
// BlockConfig — ブロック定義全体
// ---------------------------------------------------------------------------

/// ブロック定義全体を表す構造体。
///
/// ブロック定義ファイル（.md）の YAML frontmatter を
/// デシリアライズした結果を格納します。
///
/// # フィールドグループ
///
/// - **エージェント設定**: name, description, model, thinking,
///   tools, extensions, skills, timeout, worktree, enable
/// - **テンプレート**: templates（`.agent-link/template/*.md` の挿入リスト）
///
/// ブロックへの入力は `task_text` の 1 つに統一されている。
/// 本文に `{{input.task_text}}` を書かなくても、最初のメッセージには自動で
/// `# task_text` の節ができる。旧 `inputs:` / `output:` は廃止した。古い定義ファイルに
/// 残っていても未知フィールドとして無視される。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BlockConfig {
    /// ブロックの機械可読 ID（`[a-z0-9_-]+` パターン）。
    ///
    /// CLI・コマンド・`task_create` の `block_type`・DB の UNIQUE キーなど、
    /// プログラムが参照する識別子として `name` の代わりに使用される。
    /// 通常はブロック定義ファイル名（`<id>.md`）と一致させる。
    /// 未指定の場合は `name` がそのまま ID として扱われる（後方互換）。
    #[serde(default)]
    pub id: Option<String>,

    /// ブロックの表示名（sai-tui に表示される文字列）。
    /// emoji や日本語など表示用の文字列を自由に設定できる。
    /// `id` 未指定時はこの値が ID としても使われる（後方互換）。
    pub name: String,

    /// ブロックの説明
    #[serde(default)]
    pub description: Option<String>,

    /// 使用する LLM モデル（完全修飾名または "inherit"）
    #[serde(default)]
    pub model: Option<String>,

    /// thinking budget レベル
    /// 有効値: "none", "low", "medium", "high", "xhigh", "max"
    #[serde(default)]
    pub thinking: Option<String>,

    /// 使用可能ツール設定
    #[serde(default, deserialize_with = "deserialize_option_tools_config")]
    pub tools: Option<ToolsConfig>,

    /// 使用可能拡張機能設定
    #[serde(default, deserialize_with = "deserialize_option_tools_config")]
    pub extensions: Option<ToolsConfig>,

    /// ロードするスキル（カンマ区切り）
    #[serde(default)]
    pub skills: Option<String>,

    /// 呼び出し可能なサブブロック名の一覧（task_create で使用可能な block_type）。
    /// 未指定（None）の場合は全ブロック呼び出し可能（後方互換）。
    /// 指定ありの場合はこの一覧に含まれるブロックのみがサブタスクとして実行できる。
    #[serde(default)]
    pub subblocks: Option<Vec<String>>,

    /// タイムアウト秒数（デフォルト: 300）
    #[serde(default)]
    pub timeout: Option<u32>,

    /// Git worktree 分離設定
    /// 有効値: "none", "isolation", または明示的ブランチ名
    #[serde(default)]
    pub worktree: Option<String>,

    /// ブロックの有効/無効
    #[serde(default)]
    pub enable: Option<bool>,

    /// カテゴリ（herdrs 互換性のため）
    #[serde(default)]
    pub category: Option<String>,

    /// ラウンド番号（1始まり）
    /// フロントマター + 実行時設定の両方から追記可能
    #[serde(default)]
    pub round: Option<u32>,

    /// テンプレートファイル名のリスト。
    ///
    /// `PROJECTROOT/.agent-link/template/<名前>.md` から読み込まれた内容が
    /// リストの順番に、フロントマターとブロック本文（コンテンツ）の間に
    /// コピーペーストされる（`parse_block_file` 時に展開）。
    ///
    /// YAML 上では以下の形式を受け付けます:
    /// - `templates: tmp1,tmp2` → カンマ区切り文字列
    /// - `templates: [tmp1, tmp2]` / 複数行リスト → YAML リスト
    #[serde(default, deserialize_with = "deserialize_option_template_list")]
    pub templates: Option<Vec<String>>,
}

impl BlockConfig {
    /// 機械可読なブロック ID を返す。
    ///
    /// `id` が指定されていればそれを、未指定なら `name` をそのまま ID として返す
    /// （後方互換: 従来の `name` のみの定義書でも動作する）。
    ///
    /// CLI・コマンド・`task_create` の `block_type`・DB キーなどは
    /// この値を使用すること（`name` は表示専用）。
    pub fn block_id(&self) -> &str {
        self.id.as_deref().unwrap_or(self.name.as_str())
    }

    /// sai-tui 等に表示する表示名を返す。
    ///
    /// `name` は表示用のため emoji 等を自由に含められる。
    pub fn display_name(&self) -> &str {
        self.name.as_str()
    }


}

/// Option<Vec<String>>（テンプレート名リスト）のカスタムデシリアライザ。
///
/// YAML 上で `null` またはフィールド不在の場合は `None` として扱い、
/// `"tmp1,tmp2"` のようなカンマ区切り文字列と YAML リスト
/// （`[tmp1, tmp2]` / 複数行リスト）の両方を受け付けます。
fn deserialize_option_template_list<'de, D>(
    deserializer: D,
) -> Result<Option<Vec<String>>, D::Error>
where
    D: Deserializer<'de>,
{
    // serde_yaml::Value を中間表現として使用し、手動で分岐する
    let opt = Option::<serde_yaml::Value>::deserialize(deserializer)?;
    match opt {
        None => Ok(None),
        Some(serde_yaml::Value::String(s)) => {
            // カンマ区切り文字列（例: "tmp1,tmp2"）
            let items: Vec<String> = s
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
            if items.is_empty() {
                Ok(None)
            } else {
                Ok(Some(items))
            }
        }
        Some(serde_yaml::Value::Sequence(seq)) => {
            // YAML リスト（例: [tmp1, tmp2]）
            let items: Vec<String> = seq
                .iter()
                .map(|v| {
                    v.as_str()
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                })
                .collect::<Option<Vec<_>>>()
                .ok_or_else(|| {
                    de::Error::custom(
                        "templates の各要素は文字列（テンプレートファイル名）である必要があります",
                    )
                })?;
            if items.is_empty() {
                Ok(None)
            } else {
                Ok(Some(items))
            }
        }
        Some(other) => Err(de::Error::custom(format!(
            "templates の値としてカンマ区切り文字列またはリストが必要ですが、'{:?}' が指定されました",
            other
        ))),
    }
}

/// Option<ToolsConfig> のカスタムデシリアライザ。
///
/// YAML 上で `null` またはフィールド不在の場合は `None` として扱い、
/// `true`/`false`/文字列が指定された場合は対応する `ToolsConfig` に変換します。
fn deserialize_option_tools_config<'de, D>(deserializer: D) -> Result<Option<ToolsConfig>, D::Error>
where
    D: Deserializer<'de>,
{
    // serde_yaml::Value を中間表現として使用し、手動で分岐する
    let opt = Option::<serde_yaml::Value>::deserialize(deserializer)?;
    match opt {
        None => Ok(None),
        Some(serde_yaml::Value::Bool(true)) => Ok(Some(ToolsConfig::All)),
        Some(serde_yaml::Value::Bool(false)) => Ok(Some(ToolsConfig::None)),
        Some(serde_yaml::Value::String(s)) => {
            let items: Vec<String> = s
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
            if items.is_empty() {
                Ok(Some(ToolsConfig::None))
            } else {
                Ok(Some(ToolsConfig::List(items)))
            }
        }
        Some(serde_yaml::Value::Sequence(seq))
            if matches!(seq.first(), Some(serde_yaml::Value::String(_))) =>
        {
            // R12: YAML リスト表記（例: `extensions: ["../pi-sai/extension.ts", "yyy.ts"]` や複数行リスト）
            let items: Vec<String> = seq
                .into_iter()
                .map(|v| {
                    v.as_str()
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                })
                .collect::<Option<Vec<_>>>()
                .ok_or_else(|| {
                    de::Error::custom(
                        "tools/extensions の各要素は文字列（ツール名または拡張機能パス）である必要があります",
                    )
                })?;
            if items.is_empty() {
                Ok(Some(ToolsConfig::None))
            } else {
                Ok(Some(ToolsConfig::List(items)))
            }
        }
        Some(other) => Err(de::Error::custom(format!(
            "tools/extensions の値として真偽値または文字列・リストが必要ですが、'{:?}' が指定されました",
            other
        ))),
    }
}

#[cfg(test)]
mod tests {
    use crate::parser::parse_block_string;

    /// 旧定義に `review_headings` が残っていても読める（使わない）。
    #[test]
    fn legacy_review_headings_is_ignored() {
        let content = "---\nname: test\ndescription: desc\nmodel: m\nreview_headings:\n  - 納得した点\n---\n# Body";
        assert!(parse_block_string(content).is_ok());
    }
}
