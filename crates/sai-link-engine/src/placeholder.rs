//! # プレースホルダー展開ユーティリティ
//!
//! ブロックプロンプト本文中の `{{input.<id>}}` プレースホルダーを
//! ユーザー入力値で置換する機能を提供します。
//!
//! `lib/herdrs/types.ts` のプレースホルダー処理に相当する機能を
//! Rust で実装します。
//!
//! ## プレースホルダー形式
//!
//! ```text
//! プロジェクト名は {{input.project_name}} です。
//! ```
//!
//! ## 使用例
//!
//! ```
//! use std::collections::HashMap;
//! use sai_link_engine::placeholder::{expand_placeholders, collect_placeholders};
//!
//! let mut values = HashMap::new();
//! values.insert("project_name".to_string(), "MyApp".to_string());
//!
//! let result = expand_placeholders("Hello {{input.project_name}}!", &values);
//! assert_eq!(result, "Hello MyApp!");
//!
//! let ids = collect_placeholders("{{input.a}} and {{input.b}}");
//! assert_eq!(ids, vec!["a", "b"]);
//! ```

use std::collections::HashMap;
use std::sync::OnceLock;

/// プレースホルダー正規表現（遅延初期化・コンパイルは初回 1 回のみ）。
fn placeholder_regex() -> &'static regex::Regex {
    static REGEX: OnceLock<regex::Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        regex::Regex::new(PLACEHOLDER_PATTERN).expect("プレースホルダー正規表現のコンパイルに失敗")
    })
}

/// 文中の waggle 参照（`waggle:<トークン>` または `waggle:<トークン>:path:…`）
/// にマッチする正規表現（P1）。
const WAGGLE_REF_PATTERN: &str = r"waggle:([1-9A-HJ-NP-Za-km-z]{1,23})(?::path:[^\s)]+)?";

/// 値全体が `waggle:<トークン>` 形式か判定する正規表現（P1・`from_string` 用）。
const WAGGLE_TOKEN_PATTERN: &str = r"^waggle:[1-9A-HJ-NP-Za-km-z]{1,23}$";

/// 文中の waggle 参照正規表現（遅延初期化・コンパイルは初回 1 回のみ）。
fn waggle_ref_regex() -> &'static regex::Regex {
    static REGEX: OnceLock<regex::Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        regex::Regex::new(WAGGLE_REF_PATTERN).expect("waggle 参照正規表現のコンパイルに失敗")
    })
}

/// 値全体が `waggle:<トークン>` か判定する正規表現（遅延初期化）。
fn waggle_token_regex() -> &'static regex::Regex {
    static REGEX: OnceLock<regex::Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        regex::Regex::new(WAGGLE_TOKEN_PATTERN).expect("waggle トークン正規表現のコンパイルに失敗")
    })
}

/// プレースホルダー値の種類。
///
/// `expand_placeholders()` に渡す前に、`resolve_placeholder_values()` で
/// waggle トークンを解決してから文字列マップに変換します。
#[derive(Debug, Clone)]
pub enum PlaceholderValue {
    /// 通常の文字列値（そのまま展開される）
    Text(String),
    /// waggle トークン参照（"waggle:AbCdEfGh" 形式）。
    /// `resolve_placeholder_values()` でファイル内容に解決される。
    WaggleToken { token: String },
}

impl PlaceholderValue {
    /// 文字列から PlaceholderValue を作成する。
    ///
    /// 値全体が `^waggle:<base58 1〜23文字>$` に一致するときだけ `WaggleToken`、
    /// それ以外はすべて `Text` を返す（P1）。
    ///
    /// カンマ区切りで複数の waggle トークンが含まれる場合や、
    /// 文中に waggle 参照がある場合は `Text` のままにして、
    /// `resolve_placeholder_values()` で分割・埋め込みされるようにする。
    /// 不正な形式（`waggle:waggle:A` のような二重プレフィックスや空トークン）も
    /// `Text` として扱い、解決時に明示的なエラーメッセージへ変換する。
    pub fn from_string(s: &str) -> Self {
        let trimmed = s.trim();
        if waggle_token_regex().is_match(trimmed) {
            PlaceholderValue::WaggleToken {
                token: trimmed["waggle:".len()..].to_string(),
            }
        } else {
            PlaceholderValue::Text(trimmed.to_string())
        }
    }
}

/// waggle トークン文字列として妥当か判定する（2.1 の堅牢化）。
///
/// 空でなく、`waggle:` プレフィックスを含まず、空白・制御文字を含まないこと。
fn is_valid_waggle_token(token: &str) -> bool {
    !token.is_empty()
        && !token.contains("waggle:")
        && !token.chars().any(|c| c.is_whitespace() || c.is_control())
}

/// プレースホルダー `{{input.<id>}}` にマッチする正規表現
const PLACEHOLDER_PATTERN: &str = r"\{\{input\.([a-z0-9_]+)\}\}";

/// `{{subject}}` / `{{description}}` をタスク件名・詳細で置換する（P1-4）。
///
/// ブロック定義本文にプレースホルダーが無い場合は何も展開しない
/// （従来の「# タスク: subject」自動挿入は廃止）。
/// `{{input.*}}` の展開は `prepare_pipeline` 内で従来どおり行われるため、
/// ここでは触らない。
pub fn expand_task_placeholders(template: &str, subject: &str, description: &str) -> String {
    template
        .replace("{{subject}}", subject)
        .replace("{{description}}", description)
}

/// テンプレート文字列内の `{{input.<id>}}` プレースホルダーを
/// 指定された値で置換します。
///
/// `values` マップに存在しない `id` のプレースホルダーは
/// 元の表記のまま残されます（エラーにはなりません）。
///
/// # 引数
///
/// * `template` — プレースホルダーを含むテンプレート文字列
/// * `values` — プレースホルダー id → 値 のマッピング
///
/// # 戻り値
///
/// プレースホルダーが展開された文字列
///
/// # 例
///
/// ```
/// use std::collections::HashMap;
/// use sai_link_engine::placeholder::expand_placeholders;
///
/// let mut values = HashMap::new();
/// values.insert("name".to_string(), "World".to_string());
/// values.insert("count".to_string(), "42".to_string());
///
/// // 単一プレースホルダー
/// assert_eq!(
///     expand_placeholders("Hello {{input.name}}!", &values),
///     "Hello World!"
/// );
///
/// // 複数プレースホルダー
/// assert_eq!(
///     expand_placeholders("{{input.name}} x {{input.count}}", &values),
///     "World x 42"
/// );
///
/// // 存在しないプレースホルダーはそのまま
/// assert_eq!(
///     expand_placeholders("{{input.missing}}", &values),
///     "{{input.missing}}"
/// );
/// ```
pub fn expand_placeholders(template: &str, values: &HashMap<String, String>) -> String {
    let re = placeholder_regex();

    re.replace_all(template, |caps: &regex::Captures| {
        let id = &caps[1];
        values
            .get(id)
            .map(|v| v.as_str())
            .unwrap_or(&caps[0]) // 存在しない id は元の表記のまま
            .to_string()
    })
    .to_string()
}

/// テンプレート文字列から全てのプレースホルダー ID を重複なく収集します。
///
/// ブロックの入力フィールドとプレースホルダーの一致を検証するための
/// 事前チェックに使用します。
///
/// # 引数
///
/// * `template` — プレースホルダーを含むテンプレート文字列
///
/// # 戻り値
///
/// プレースホルダー ID の `Vec<String>`（出現順、重複なし）
///
/// # 例
///
/// ```
/// use sai_link_engine::placeholder::collect_placeholders;
///
/// let ids = collect_placeholders("{{input.a}} and {{input.b}} and {{input.a}}");
/// assert_eq!(ids, vec!["a", "b"]);
/// ```
pub fn collect_placeholders(template: &str) -> Vec<String> {
    let re = placeholder_regex();

    let mut ids: Vec<String> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();

    for caps in re.captures_iter(template) {
        let id = caps[1].to_string();
        if seen.insert(id.clone()) {
            ids.push(id);
        }
    }

    ids
}

/// waggle 文書の本文を、出力トークンのマーカー付きブロックに変換する（P1-3）。
///
/// 複数トークンを連結するときの書式を 1 箇所に集約する。
fn wrap_token_content(token: &str, block_name: &str, content: &str) -> String {
    format!(
        "<!-- waggle:token:{} block:{} -->\n\n\
         ## 出力トークン: {} ({})\n\n\
         {}\n\n\
         <!-- /waggle:token:{} -->",
        token, block_name, token, block_name, content, token
    )
}

/// 解決失敗時のメッセージを作る（U08）。
///
/// - `TokenNotFound`（このプロジェクトの記録に無いトークン）は、**どのプロジェクトで探したか**が
///   分かる文にする。別プロジェクトのトークンを `waggle:` で書いた場合の混乱を防ぐ。
/// - それ以外（ファイルが読めない等）は従来の文のまま。
fn resolve_failure_message(
    token: &str,
    e: &crate::waggle_integration::WaggleIntegrationError,
) -> String {
    use crate::waggle_integration::WaggleIntegrationError;
    match e {
        WaggleIntegrationError::TokenNotFound(_) => format!(
            "[waggle トークン '{}' はこのプロジェクト（{}）の記録にありません。\
             別のプロジェクトの文書なら、本文を貼るか、そのプロジェクトで実行してください]",
            token,
            project_dir_display(),
        ),
        _ => format!("[waggle トークン '{}' の解決に失敗: {}]", token, e),
    }
}

/// エラーメッセージに出すプロジェクトフォルダ（人間向けの絶対パス）。
///
/// `resolve_db_path()` は `<root>/.sai/sai-engine/sai-engine.db` なので
/// 3 つ親がプロジェクトルート。辿れない場合は `project_root()` に落とす。
fn project_dir_display() -> String {
    // 相対パスの候補（`../.sai/…` など）でも絶対パスで出すため canonicalize する。
    // DB が無ければ canonicalize が失敗するので project_root()（cwd 基準）に落とす
    std::fs::canonicalize(crate::project::resolve_db_path())
        .ok()
        .and_then(|db| db.parent()?.parent()?.parent().map(|p| p.to_path_buf()))
        .unwrap_or_else(crate::project::project_root)
        .display()
        .to_string()
}

/// 文中の waggle 参照（`waggle:<tok>` / `waggle:<tok>:path:…`）を
/// 該当文書の本文に置き換える（P1-5）。
///
/// 置き換える本文の中の `waggle:` は再帰展開しない。
/// 解決を試みたトークンは `waggle_tokens` に push する（失敗した場合も含む）。
fn expand_inline_waggle(
    text: &str,
    integrator: &crate::waggle_integration::WaggleIntegrator,
    waggle_tokens: &mut Vec<String>,
) -> String {
    waggle_ref_regex()
        .replace_all(text, |caps: &regex::Captures| {
            let token = &caps[1];
            waggle_tokens.push(token.to_string());
            match integrator.resolve_content(token) {
                Ok(content) => {
                    let block_name = integrator.find_block_name_by_token(token);
                    format!(
                        "\n\n{}\n\n",
                        wrap_token_content(token, &block_name, &content)
                    )
                }
                Err(e) => resolve_failure_message(token, &e)
            }
        })
        .to_string()
}

/// `PlaceholderValue` のマップを解決し、通常の文字列マップに変換する。
///
/// `WaggleToken` 値は `integrator.resolve_content()` でファイル内容に解決され、
/// `Text` 値はそのまま文字列に変換されます。
///
/// また、Text 値が `"waggle:A,waggle:B"` 形式のカンマ区切り複数トークンを
/// 含む場合、各トークンを個別に解決し、マークダウン連結フォーマットで結合します。
///
/// # 引数
///
/// * `values` — プレースホルダー id → PlaceholderValue のマッピング
/// * `integrator` — waggle トークン解決に使用するインテグレーター
///
/// # 戻り値
///
/// 解決済みの `HashMap<String, String>` と、解決された waggle トークンのリスト
pub fn resolve_placeholder_values(
    values: &HashMap<String, PlaceholderValue>,
    integrator: &crate::waggle_integration::WaggleIntegrator,
) -> (HashMap<String, String>, Vec<String>) {
    let mut resolved: HashMap<String, String> = HashMap::new();
    let mut waggle_tokens: Vec<String> = Vec::new();

    for (id, value) in values {
        match value {
            PlaceholderValue::Text(text) => {
                // カンマ区切りの全要素が、trim 後に `waggle:` で始まり空白も `:path:` も
                // 含まない場合だけ「トークン列」として扱う（旧カード形式との互換）。
                // `waggle:<tok>:path:…` 単独は文中の参照として展開する。
                let parts: Vec<&str> = text.split(',').map(|s| s.trim()).collect();
                let is_token_list = parts.iter().all(|p| {
                    p.starts_with("waggle:")
                        && !p.contains(":path:")
                        && !p.chars().any(char::is_whitespace)
                });

                if !is_token_list {
                    // トークン列ではない: 文中の waggle 参照を 1 段だけ本文に展開する（P1）
                    resolved.insert(
                        id.clone(),
                        expand_inline_waggle(text, integrator, &mut waggle_tokens),
                    );
                } else if parts.len() > 1 {
                    // 複数トークン: 連結フォーマット
                    let mut contents = Vec::new();
                    for token_str in &parts {
                        if let Some(token) = token_str.strip_prefix("waggle:") {
                            if !is_valid_waggle_token(token) {
                                contents.push(format!(
                                    "[トークン '{}' の形式が不正です: waggle: を二重に含むか、空トークンです]",
                                    token_str
                                ));
                                continue;
                            }
                            waggle_tokens.push(token.to_string());
                            match integrator.resolve_content(token) {
                                Ok(content) => {
                                    let block_name = integrator.find_block_name_by_token(token);
                                    contents.push(wrap_token_content(token, &block_name, &content));
                                }
                                Err(e) => {
                                    contents.push(resolve_failure_message(token, &e));
                                }
                            }
                        }
                    }
                    resolved.insert(id.clone(), contents.join("\n\n---\n\n"));
                } else {
                    // 単一トークン: 既存の WaggleToken と同様に本文そのものを展開
                    if let Some(token) = parts[0].strip_prefix("waggle:") {
                        if !is_valid_waggle_token(token) {
                            resolved.insert(id.clone(), format!(
                                "[トークン '{}' の形式が不正です: waggle: を二重に含むか、空トークンです]",
                                parts[0]
                            ));
                        } else {
                            waggle_tokens.push(token.to_string());
                            match integrator.resolve_content(token) {
                                Ok(content) => {
                                    resolved.insert(id.clone(), content);
                                }
                                Err(e) => {
                                    let fallback = resolve_failure_message(token, &e);
                                    resolved.insert(id.clone(), fallback);
                                }
                            }
                        }
                    }
                }
            }
            PlaceholderValue::WaggleToken { token } => {
                if !is_valid_waggle_token(token) {
                    resolved.insert(
                        id.clone(),
                        format!(
                        "[トークン '{}' の形式が不正です: waggle: を二重に含むか、空トークンです]",
                        token
                    ),
                    );
                    continue;
                }
                waggle_tokens.push(token.clone());
                match integrator.resolve_content(token) {
                    Ok(content) => {
                        resolved.insert(id.clone(), content);
                    }
                    Err(e) => {
                        // 解決失敗時はエラーメッセージをプレースホルダー値として展開
                        let fallback = resolve_failure_message(token, &e);
                        resolved.insert(id.clone(), fallback);
                    }
                }
            }
        }
    }

    (resolved, waggle_tokens)
}

// ============================================================================
// テスト
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_expand_single_placeholder() {
        let mut values = HashMap::new();
        values.insert("name".to_string(), "World".to_string());

        assert_eq!(
            expand_placeholders("Hello {{input.name}}!", &values),
            "Hello World!"
        );
    }

    #[test]
    fn test_expand_multiple_placeholders() {
        let mut values = HashMap::new();
        values.insert("first".to_string(), "Alice".to_string());
        values.insert("last".to_string(), "Smith".to_string());

        assert_eq!(
            expand_placeholders("{{input.first}} {{input.last}}", &values),
            "Alice Smith"
        );
    }

    #[test]
    fn test_expand_missing_placeholder() {
        let values = HashMap::new();
        // 値が提供されていないプレースホルダーは元の表記のまま
        assert_eq!(
            expand_placeholders("Hello {{input.name}}!", &values),
            "Hello {{input.name}}!"
        );
    }

    #[test]
    fn test_expand_no_placeholders() {
        let values = HashMap::new();
        assert_eq!(expand_placeholders("Hello World!", &values), "Hello World!");
    }

    #[test]
    fn test_expand_empty_template() {
        let values = HashMap::new();
        assert_eq!(expand_placeholders("", &values), "");
    }

    #[test]
    fn test_expand_partial_placeholder() {
        let mut values = HashMap::new();
        values.insert("name".to_string(), "Test".to_string());

        // `{{input.` だけではマッチしない
        assert_eq!(
            expand_placeholders("{{input.name}} and {{input.", &values),
            "Test and {{input."
        );
    }

    #[test]
    fn test_collect_placeholders() {
        let ids = collect_placeholders("{{input.a}} and {{input.b}} and {{input.a}}");
        assert_eq!(ids, vec!["a", "b"]);
    }

    #[test]
    fn test_collect_placeholders_no_placeholders() {
        let ids: Vec<String> = collect_placeholders("Hello World!");
        assert!(ids.is_empty());
    }

    #[test]
    fn test_collect_placeholders_empty() {
        let ids: Vec<String> = collect_placeholders("");
        assert!(ids.is_empty());
    }

    #[test]
    fn test_expand_with_hyphen_and_underscore() {
        let mut values = HashMap::new();
        values.insert("project_name".to_string(), "MyApp".to_string());

        assert_eq!(
            expand_placeholders("Project: {{input.project_name}}", &values),
            "Project: MyApp"
        );
    }

    // ---- P1-4: expand_task_placeholders ----

    #[test]
    fn test_expand_task_placeholders_basic() {
        let result = expand_task_placeholders(
            "件名: {{subject}}\n詳細: {{description}}",
            "ログイン機能",
            "認証周りを実装",
        );
        assert_eq!(result, "件名: ログイン機能\n詳細: 認証周りを実装");
    }

    #[test]
    fn test_expand_task_placeholders_without_placeholders() {
        // プレースホルダーが無い本文はそのまま（自動挿入なし）
        let result = expand_task_placeholders("本文のみ", "件名", "詳細");
        assert_eq!(result, "本文のみ");
    }

    #[test]
    fn test_expand_task_placeholders_empty_subject() {
        let result = expand_task_placeholders("S={{subject}}", "", "d");
        assert_eq!(result, "S=");
    }

    // ---- P1: PlaceholderValue::from_string の境界 ----

    /// 値全体が `waggle:<トークン>` のときだけ `WaggleToken` になる。
    /// 前後の空白は trim される。
    #[test]
    fn test_from_string_whole_value_is_waggle_token() {
        match PlaceholderValue::from_string("waggle:AXANSMpo") {
            PlaceholderValue::WaggleToken { token } => assert_eq!(token, "AXANSMpo"),
            other => panic!("WaggleToken になるべき: {other:?}"),
        }
        match PlaceholderValue::from_string("  waggle:AXANSMpo\n") {
            PlaceholderValue::WaggleToken { token } => assert_eq!(token, "AXANSMpo"),
            other => panic!("trim して WaggleToken になるべき: {other:?}"),
        }
    }

    /// 文中に waggle 参照がある値は `Text` のまま（`resolve_placeholder_values` で
    /// インライン展開する）。
    #[test]
    fn test_from_string_inline_reference_is_text() {
        assert!(matches!(
            PlaceholderValue::from_string("下記を実装せよ。waggle:AXANSMpo"),
            PlaceholderValue::Text(_)
        ));
        assert!(matches!(
            PlaceholderValue::from_string("waggle:AXANSMpo と比較せよ"),
            PlaceholderValue::Text(_)
        ));
    }

    /// `waggle:<tok>:path:…` は全体一致ではないので `Text`
    /// （参照として本文に置き換える対象）。
    #[test]
    fn test_from_string_path_reference_is_text() {
        let value = "waggle:AXANSMpo:path:.sai/outputs/plan/1/a.md";
        assert!(matches!(
            PlaceholderValue::from_string(value),
            PlaceholderValue::Text(_)
        ));
    }

    /// 不正な形式（二重プレフィックス・24 文字以上・base58 以外）は `Text` にし、
    /// 解決時にエラーメッセージへ変換できるようにする。
    #[test]
    fn test_from_string_invalid_is_text() {
        for value in [
            "waggle:waggle:AXANSMpo",             // 二重プレフィックス
            "waggle:",                            // 空トークン
            "waggle:123456789012345678901234",    // 24 文字（上限超え）
            "waggle:AX0NSMpo",                    // base58 に無い '0'
            "waggle:AXANSMpo:path:with space.md", // 空白を含む path
        ] {
            assert!(
                matches!(
                    PlaceholderValue::from_string(value),
                    PlaceholderValue::Text(_)
                ),
                "Text になるべき: {value}"
            );
        }
    }

    /// 見つからないトークンの文に「どのプロジェクトの記録で探したか」が入る（U08・AC3）。
    #[test]
    fn test_missing_token_message_names_project() {
        let integrator = crate::waggle_integration::WaggleIntegrator::open_in_memory().unwrap();
        // 空のストアなので TokenNotFound になる
        let e = integrator
            .resolve_content("AbCdEfGh")
            .expect_err("存在しないトークンは解決に失敗するはず");
        let msg = resolve_failure_message("AbCdEfGh", &e);
        assert!(msg.contains("AbCdEfGh"), "{}", msg);
        assert!(msg.contains("はこのプロジェクト（"), "{}", msg);
        assert!(msg.contains("）の記録にありません"), "{}", msg);
        assert!(
            msg.contains("別のプロジェクトの文書なら、本文を貼るか、そのプロジェクトで実行してください"),
            "{}",
            msg
        );
        // プロジェクトのパスは解決できる AbsolutePath で出る
        let dir = project_dir_display();
        assert!(!dir.is_empty() && std::path::Path::new(&dir).is_absolute(), "{}", dir);
    }

    /// 見つからない以外の理由（ファイルが読めない等）は従来の文のまま。
    #[test]
    fn test_other_error_message_keeps_old_text() {
        use crate::waggle_integration::WaggleIntegrationError;
        let msg = resolve_failure_message(
            "AbCdEfGh",
            &WaggleIntegrationError::Hash("壊れたハッシュ".to_string()),
        );
        assert!(msg.contains("の解決に失敗"), "{}", msg);
        assert!(msg.contains("壊れたハッシュ"), "{}", msg);
    }
}
