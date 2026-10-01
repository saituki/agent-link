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

/// 本文があるのに最初の文に節を 1 つも作れなかったときの文（U6・C10）。
const FIRST_MESSAGE_FALLBACK: &str = "system prompt の指示に従って始めてください。";

/// ブロックの本文を「固定部分（system prompt）」と「最初の文」に分ける（U6・C10）。
///
/// 固定部分（`system_prompt`）は template と block.md の本文そのもので、
/// 中の可変な場所だけを「最初の文のどの節にあるか」を示す案内の文に置き換える。
/// - `{{input.<id>}}` → `（最初のメッセージの「# <id>」の節にある）`
/// - `{{context}}`（`context` を渡したときだけ）→
///   `（最初のメッセージの「# context」の節にある）`
///
/// 本文に `{{input.task_text}}`・`{{context}}` の記法が無くても、
/// 最初の文には `# context` → `# task_text` の順で節が自動で作られる。
/// このとき固定部分の先頭に「依頼は○○」の案内文を足す（古い記法の本文には足さない。
/// 置き換え済みの案内と重複させないため）。
///
/// 最初の文（`first_message`）の節の順番は次のとおり。
/// 1. `# context`（`context` を渡して中身が空でないとき）
/// 2. `# task_text`（値が無いとき・空のときは作らない）
/// 3. そのほかの id（本文に現れた `{{input.<id>}}` の順。`task_text` は除く）
///
/// 同じ id が何度出ても節は 1 つだけ。値が無い id は `expand_placeholders` と同じ扱いで
/// `{{input.<id>}}` の表記をそのまま入れる。
/// 節が 1 つも無く固定部分も空なら、最初の文も空（本文が無いので送らない・従来と同じ）。
///
/// # 例
///
/// ```
/// use std::collections::HashMap;
/// use sai_link_engine::placeholder::split_fixed_and_variable;
///
/// let mut values = HashMap::new();
/// values.insert("task_text".to_string(), "ログインを実装せよ".to_string());
/// // 本文に `{{input.task_text}}`・`{{context}}` は書かれていない
/// let body = "# 依頼\n\nあなたは実装者です。";
///
/// let (fixed, first) = split_fixed_and_variable(body, &values, Some("run=1 block=b"));
/// assert!(fixed.contains("依頼は、最初のメッセージの「# task_text」の節にあります。"));
/// assert!(fixed.contains("範囲（研究方針の木から切り出した文脈）は"));
/// assert!(!fixed.contains("ログインを実装せよ"));
/// assert_eq!(
///     first,
///     "# context\n\nrun=1 block=b\n\n# task_text\n\nログインを実装せよ"
/// );
/// ```
pub fn split_fixed_and_variable(
    body: &str,
    values: &HashMap<String, String>,
    context: Option<&str>,
) -> (String, String) {
    // ---- 固定部分：可変な場所を案内の文に置き換える ----
    let mut fixed = placeholder_regex()
        .replace_all(body, |caps: &regex::Captures| {
            format!("（最初のメッセージの「# {}」の節にある）", &caps[1])
        })
        .into_owned();
    if context.is_some() {
        // `context: true` のときだけ本文中の `{{context}}` を案内文にする。
        // context 無しで本文に `{{context}}` が残るときは従来どおり本文のままにする。
        fixed = fixed.replace("{{context}}", "（最初のメッセージの「# context」の節にある）");
    }

    // ---- 最初の文：context → task_text → 本文に現れた残りの id ----
    let task_text = values.get("task_text").map(|v| v.trim()).unwrap_or("");
    let context_body = context.map(|c| c.trim()).unwrap_or("");

    let mut sections: Vec<String> = Vec::new();
    if !context_body.is_empty() {
        // context の出力は `##` から始まる（U02）。見出しを付け直さない。
        sections.push(format!("# context\n\n{}", context_body.trim_end()));
    }
    if !task_text.is_empty() {
        sections.push(format!("# task_text\n\n{}", task_text.trim_end()));
    }
    for id in collect_placeholders(body) {
        // `task_text` は上で作っているので、ここでは扱わない。
        if id == "task_text" {
            continue;
        }
        let value = values
            .get(&id)
            .map(|v| v.trim_end().to_string())
            .unwrap_or_else(|| format!("{{{{input.{}}}}}", id));
        sections.push(format!("# {}\n\n{}", id, value));
    }

    // ---- 案内文を固定部分の先頭に足す ----
    // 固定部分が空白だけのときは足さない（本文が無く案内だけの文になる）。
    if !fixed.trim().is_empty() {
        let mut notes: Vec<&str> = Vec::new();
        // 古い記法の本文には置き換え済みの案内があるため、足さない。
        if !context_body.is_empty() && !body.contains("{{context}}") {
            notes.push(
                "範囲（研究方針の木から切り出した文脈）は、最初のメッセージの「# context」の節にあります。",
            );
        }
        if !task_text.is_empty() && !body.contains("{{input.task_text}}") {
            notes.push("依頼は、最初のメッセージの「# task_text」の節にあります。");
        }
        if !notes.is_empty() {
            // 2 文は 1 行（改行なし）につなぎ、その後に空行 1 行を置いて本文から続ける。
            fixed = format!("{}\n\n{}", notes.join(" "), fixed);
        }
    }

    if sections.is_empty() {
        // 固定部分が空（本文が無い）のときは、最初の文も空のまま（従来と同じ）。
        if fixed.trim().is_empty() {
            return (fixed, String::new());
        }
        return (fixed, FIRST_MESSAGE_FALLBACK.to_string());
    }

    (fixed, sections.join("\n\n"))
}

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

/// Markdown の見出しを `shift` 段下げる（共有契約 C1）。
///
/// 規則:
/// 1. 行頭が `#` の連続 1〜6 個で、その次が空白（半角スペースかタブ）の行を見出しとする。
///    `#` の数に `shift` を足し、6 を超えたら 6 にする。見出しの後ろの文字はそのまま。
/// 2. ```` ``` ```` または `~~~` で始まる行（先頭に空白 3 つまで可）でコードブロックが
///    始まり、同じ記号で開いた個数以上の連続で始まる行で閉じる。コードブロックの中の行
///    （開く行・閉じる行を含む）は触らない。
/// 3. HTML コメント（`<!--` から `-->` まで。複数行にまたがってよい）の中の行は
///    触らない。
/// 4. 行の分け方は `text.split('\n')` と `join("\n")`。末尾の改行・`\r` を保つ。
///
/// # 例
///
/// ```
/// use sai_link_engine::placeholder::shift_headings;
///
/// assert_eq!(shift_headings("# A\n## B", 3), "#### A\n##### B");
/// // 6 段で止まる
/// assert_eq!(shift_headings("###### Z", 3), "###### Z");
/// // コードブロックの中の `#` は下げない
/// assert_eq!(shift_headings("```bash\n# c\n```", 3), "```bash\n# c\n```");
/// ```
pub fn shift_headings(text: &str, shift: usize) -> String {
    /// コードブロックのフェンス（```` ``` ```` / `~~~`）の記号と個数。
    /// 先頭の空白は 3 つまで可。
    fn fence_of(line: &str) -> Option<(char, usize)> {
        let indent = line.chars().take_while(|c| *c == ' ' || *c == '\t').count();
        if indent > 3 {
            return None;
        }
        let rest = line.trim_start_matches([' ', '\t']);
        for ch in ['`', '~'] {
            let n = rest.chars().take_while(|c| *c == ch).count();
            if n >= 3 {
                return Some((ch, n));
            }
        }
        None
    }

    /// フェンスの行が閉じているか（記号は開いた記号と同じ・個数は以上・後ろは空白だけ）。
    fn is_fence_close(line: &str, ch: char, n: usize) -> bool {
        if line.chars().take_while(|c| *c == ' ' || *c == '\t').count() > 3 {
            return false;
        }
        let rest = line.trim_start_matches([' ', '\t']);
        let count = rest.chars().take_while(|c| *c == ch).count();
        count >= n && rest[count..].trim().is_empty()
    }

    /// 1 行の見出しを 1 段下げる。見出しでなければそのまま返す。
    fn shift_line(line: &str, shift: usize) -> String {
        let hashes = line.chars().take_while(|c| *c == '#').count();
        // 1〜6 個の以外や、空白が続かない行は見出しにしない
        if !(1..=6).contains(&hashes) {
            return line.to_string();
        }
        let rest = &line[hashes..];
        if !rest.starts_with(' ') && !rest.starts_with('\t') {
            return line.to_string();
        }
        format!("{}{}", "#".repeat((hashes + shift).min(6)), rest)
    }

    let mut out = String::with_capacity(text.len());
    // 開いているコードブロックのフェンス（`None` はブロックの外）
    let mut fence: Option<(char, usize)> = None;
    let mut in_comment = false;

    for (i, line) in text.split('\n').enumerate() {
        if i > 0 {
            out.push('\n');
        }

        // コードブロックの中は行をそのまま通す（開く行・閉じる行を含む）
        if let Some((ch, n)) = fence {
            out.push_str(line);
            if is_fence_close(line, ch, n) {
                fence = None;
            }
            continue;
        }

        // HTML コメントの中は行をそのまま通す
        if in_comment {
            out.push_str(line);
            if line.contains("-->") {
                in_comment = false;
            }
            continue;
        }

        // コードブロックの開始
        if let Some((ch, n)) = fence_of(line) {
            fence = Some((ch, n));
            out.push_str(line);
            continue;
        }

        // HTML コメントの開始（同一行で閉じるものは見出し判定もしない）
        if line.contains("<!--") {
            if !line.contains("-->") {
                in_comment = true;
            }
            out.push_str(line);
            continue;
        }

        out.push_str(&shift_line(line, shift));
    }

    out
}

/// 先頭の frontmatter（最初の行が `---` で、後ろに `---` だけの行がある）を外す。
///
/// 閉じる `---` が無いときは何もしない（そのまま返す）。
/// 外したあとの先頭の空行も落とす。
///
/// # 例
///
/// ```
/// use sai_link_engine::placeholder::strip_frontmatter;
///
/// assert_eq!(strip_frontmatter("---\ntitle: A\n---\n# 題\n"), "# 題\n");
/// // 閉じる `---` が無ければそのまま
/// assert_eq!(strip_frontmatter("---\ntitle: A\n"), "---\ntitle: A\n");
/// ```
pub fn strip_frontmatter(text: &str) -> &str {
    let Some(first_line) = text.strip_prefix("---") else {
        return text;
    };
    // 1 行目が `---` だけであること（`---x` などは frontmatter にしない）
    let Some(rest) = first_line
        .strip_prefix('\n')
        .or_else(|| first_line.strip_prefix("\r\n"))
    else {
        return text;
    };

    // 閉じる `---` を探す
    let mut offset = 0usize;
    for line in rest.split_inclusive('\n') {
        offset += line.len();
        if line.trim_end_matches('\n').trim_end_matches('\r') == "---" {
            return rest[offset..].trim_start_matches(['\n', '\r']);
        }
    }
    // 閉じる `---` が無いので何もしない
    text
}

/// waggle 文書の本文を、出力トークンのマーカー付きブロックに変換する（P1-3）。
///
/// 中身は [`strip_frontmatter`] で frontmatter を外してから [`shift_headings`] で
/// 3 段下げる。包みの見出しも `###` なので、中身の `#` は `# task_text` の節より
/// 深くならない。
///
/// 複数トークンを連結するときの書式を 1 箇所に集約する。
fn wrap_token_content(token: &str, block_name: &str, content: &str) -> String {
    let body = shift_headings(strip_frontmatter(content), 3);
    format!(
        "<!-- waggle:token:{} block:{} -->\n\n\
         ### 出力トークン: {} ({})\n\n\
         {}\n\n\
         <!-- /waggle:token:{} -->",
        token, block_name, token, block_name, body, token
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

    // ---- U6: split_fixed_and_variable ----

    /// `{{input.task_text}}` と `{{context}}` を含む本文（古い記法）で、
    /// 固定部分と最初の文が分かれる。案内文は重複して足さない。
    #[test]
    fn test_split_fixed_and_variable_with_input_and_context() {
        let mut values = HashMap::new();
        values.insert("task_text".to_string(), "ログインを実装せよ".to_string());
        let body = "# 依頼\n{{input.task_text}}\n\n# 範囲\n{{context}}";

        let (fixed, first) =
            split_fixed_and_variable(body, &values, Some("run=1 block=test-block"));

        // 固定部分：値は出ず、案内の文に置き換わる（`# context` に変わった）
        assert!(fixed.contains("# 依頼"));
        assert!(
            fixed.contains("（最初のメッセージの「# task_text」の節にある）"),
            "{fixed}"
        );
        assert!(
            fixed.contains("（最初のメッセージの「# context」の節にある）"),
            "{fixed}"
        );
        assert!(!fixed.contains("ログインを実装せよ"), "{fixed}");
        assert!(!fixed.contains("run=1"), "{fixed}");
        // 古い記法なので先頭の案内文は足さない
        assert!(!fixed.contains("節にあります。"), "{fixed}");

        // 最初の文：context → task_text の順に節が並ぶ
        assert_eq!(
            first,
            "# context\n\nrun=1 block=test-block\n\n# task_text\n\nログインを実装せよ"
        );
    }

    /// 同じ id が何度出ても節は 1 つだけ、出現順。
    #[test]
    fn test_split_fixed_and_variable_dedupes_in_order() {
        let mut values = HashMap::new();
        values.insert("a".to_string(), "A値".to_string());
        values.insert("b".to_string(), "B値".to_string());
        let body = "{{input.b}} と {{input.a}} と {{input.b}}";

        let (fixed, first) = split_fixed_and_variable(body, &values, None);
        assert_eq!(
            fixed,
            "（最初のメッセージの「# b」の節にある） と \
             （最初のメッセージの「# a」の節にある） と \
             （最初のメッセージの「# b」の節にある）",
            "固定部分は本文の案内文だけ（可変な値は出ない）"
        );
        assert_eq!(first, "# b\n\nB値\n\n# a\n\nA値");
    }

    /// context を渡さないとき `{{context}}` は固定部分に残る（本文をそのまま使う）。
    #[test]
    fn test_split_fixed_and_variable_without_context() {
        let mut values = HashMap::new();
        values.insert("task_text".to_string(), "依頼の本文".to_string());
        let body = "{{input.task_text}}\n\n{{context}}";

        let (fixed, first) = split_fixed_and_variable(body, &values, None);
        assert!(fixed.contains("{{context}}"), "{fixed}");
        assert!(fixed.contains("（最初のメッセージの「# task_text」の節にある）"));
        assert_eq!(first, "# task_text\n\n依頼の本文");
    }

    // ---- U01: 本文に記法が無くても自動で節と案内文ができる ----

    /// 本文に `{{input.task_text}}`・`{{context}}` が無くても、
    /// `# context` → `# task_text` の順に節ができる（AC1）。
    #[test]
    fn test_split_fixed_and_variable_auto_sections_without_old_notation() {
        let mut values = HashMap::new();
        values.insert("task_text".to_string(), "ログインを実装せよ".to_string());
        let body = "# 依頼\n\nあなたは実装者です。";

        let (fixed, first) = split_fixed_and_variable(body, &values, Some("run=1 block=b"));

        assert_eq!(first, "# context\n\nrun=1 block=b\n\n# task_text\n\nログインを実装せよ");
        // 固定部分の先頭に案内が 2 文（この順に 1 行で）付く
        let notes = "範囲（研究方針の木から切り出した文脈）は、最初のメッセージの「# context」の節にあります。 \
                     依頼は、最初のメッセージの「# task_text」の節にあります。";
        assert_eq!(fixed, format!("{notes}\n\n{body}"));
    }

    /// 古い記法（本文に `{{input.task_text}}` と `{{context}}`）でも節は 1 つずつで、
    /// 案内文は重複しない（AC2）。
    #[test]
    fn test_split_fixed_and_variable_old_notation_has_no_guide() {
        let mut values = HashMap::new();
        values.insert("task_text".to_string(), "依頼".to_string());
        let body = "{{input.task_text}} と {{context}}";

        let (fixed, first) = split_fixed_and_variable(body, &values, Some("範囲"));

        assert_eq!(first, "# context\n\n範囲\n\n# task_text\n\n依頼");
        assert_eq!(fixed.matches("節にある").count(), 2, "{fixed}");
        assert!(!fixed.contains("依頼は、最初のメッセージ"), "{fixed}");
    }

    /// `context: true` でなければ `# context` の節も案内の context の文も出ない（AC3）。
    #[test]
    fn test_split_fixed_and_variable_no_context_gives_no_context_section() {
        let mut values = HashMap::new();
        values.insert("task_text".to_string(), "依頼".to_string());
        let body = "# 本文";

        let (fixed, first) = split_fixed_and_variable(body, &values, None);

        assert_eq!(first, "# task_text\n\n依頼");
        assert!(!first.contains("# context"), "{first}");
        assert!(!fixed.contains("研究方針の木"), "{fixed}");
        // task_text の案内だけは付く
        assert!(fixed.contains("依頼は、最初のメッセージ"), "{fixed}");
    }

    /// `values` に `task_text` が無い／空白だけのときは `# task_text` の節も案内も出ない。
    #[test]
    fn test_split_fixed_and_variable_without_task_text_value() {
        let body = "# 本文";
        let (fixed, first) = split_fixed_and_variable(body, &HashMap::new(), None);
        assert_eq!(first, FIRST_MESSAGE_FALLBACK);
        assert_eq!(fixed, body);

        let mut blank = HashMap::new();
        blank.insert("task_text".to_string(), "  \n".to_string());
        let (fixed, first) = split_fixed_and_variable(body, &blank, None);
        assert_eq!(first, FIRST_MESSAGE_FALLBACK);
        assert_eq!(fixed, body);
    }

    /// context の中身が trim 後に空なら `# context` の節が出ない。
    #[test]
    fn test_split_fixed_and_variable_blank_context_body() {
        let mut values = HashMap::new();
        values.insert("task_text".to_string(), "依頼".to_string());
        let (fixed, first) = split_fixed_and_variable("# 本文", &values, Some("\n  \n"));

        assert_eq!(first, "# task_text\n\n依頼");
        assert!(!first.contains("# context"), "{first}");
        assert!(!fixed.contains("研究方針の木"), "{fixed}");
    }

    /// 本文が空白だけのときは案内文を足さない（案内だけの文にならない）。
    #[test]
    fn test_split_fixed_and_variable_blank_body_has_no_guide() {
        let mut values = HashMap::new();
        values.insert("task_text".to_string(), "依頼".to_string());
        let (fixed, first) = split_fixed_and_variable("  \n", &values, Some("範囲"));

        assert_eq!(fixed, "  \n");
        assert_eq!(first, "# context\n\n範囲\n\n# task_text\n\n依頼");
    }

    // ---- U01: shift_headings / strip_frontmatter / wrap_token_content ----

    /// 見出しを `shift` 段下げる。6 段で止まる。
    #[test]
    fn test_shift_headings_basic() {
        assert_eq!(shift_headings("# A", 1), "## A");
        assert_eq!(shift_headings("# A\n## B", 3), "#### A\n##### B");
        assert_eq!(shift_headings("###### Z", 3), "###### Z");
        assert_eq!(shift_headings("####### 七つ", 1), "####### 七つ");
        // 見出しの後ろの文字（タブも）はそのまま
        assert_eq!(shift_headings("## 題\t 余り", 2), "#### 題\t 余り");
        // 見出しでなければそのまま
        assert_eq!(shift_headings("本文 # は 見出し", 2), "本文 # は 見出し");
        // `shift` が 0 でも元の形
        assert_eq!(shift_headings("# A", 0), "# A");
    }

    /// 末尾の改行を保つ。
    #[test]
    fn test_shift_headings_keeps_trailing_newline() {
        assert_eq!(shift_headings("# A\n", 3), "#### A\n");
        assert_eq!(
            shift_headings("# A\n## B\n```bash\n# c\n```\n", 3),
            "#### A\n##### B\n```bash\n# c\n```\n"
        );
    }

    /// コードブロックの中の `#` は下げない（```` ``` ```` と `~~~` の両方）。
    #[test]
    fn test_shift_headings_skips_code_fence() {
        assert_eq!(
            shift_headings("```bash\n# c\n## d\n```", 3),
            "```bash\n# c\n## d\n```"
        );
        // 言語名付き・中の例
        assert_eq!(
            shift_headings("```md\n# 題\n\n## 節\n```", 2),
            "```md\n# 題\n\n## 節\n```"
        );
        // 波記号のフェンス
        assert_eq!(
            shift_headings("~~~\n# c\n~~~\n# A", 3),
            "~~~\n# c\n~~~\n#### A"
        );
        // 短い ～ の行（3 未満）はフェンスにしない
        assert_eq!(shift_headings("~~\n# A", 1), "~~\n## A");
        // 閉じは開いた記号と同じ（別の記号では閉じない）
        assert_eq!(
            shift_headings("```\n# c\n~~~\n# d\n```", 3),
            "```\n# c\n~~~\n# d\n```"
        );
        // 4 個の ``` で開いたなら 3 個の行では閉じない
        assert_eq!(
            shift_headings("````\n# c\n```\n# d\n````", 3),
            "````\n# c\n```\n# d\n````"
        );
        // 先頭 3 つの空白は許される（4 つ以上はフェンスではない）
        assert_eq!(
            shift_headings("   ```\n# c\n   ```\n# A", 3),
            "   ```\n# c\n   ```\n#### A"
        );
    }

    /// HTML コメントの中を下げない（複数行にまたがるもの・同一行で閉じるもの）。
    #[test]
    fn test_shift_headings_skips_html_comment() {
        assert_eq!(
            shift_headings("<!--\n# A\n-->\n# B", 1),
            "<!--\n# A\n-->\n## B"
        );
        assert_eq!(shift_headings("<!-- # A -->\n# B", 1), "<!-- # A -->\n## B");
        // コメントの後にテキストが続く行も下げない
        assert_eq!(
            shift_headings("<!-- note -->\n# A\n本文", 1),
            "<!-- note -->\n## A\n本文"
        );
    }

    /// `#見出し`（空白なし）は下げない。
    #[test]
    fn test_shift_headings_ignores_hash_without_space() {
        assert_eq!(shift_headings("#見出し", 3), "#見出し");
        assert_eq!(shift_headings(" # 見出し", 3), " # 見出し");
        assert_eq!(shift_headings("##見出し", 1), "##見出し");
    }

    /// 先頭の frontmatter を外す。閉じる `---` が無ければそのまま。
    #[test]
    fn test_strip_frontmatter() {
        assert_eq!(
            strip_frontmatter("---\ntitle: A\n---\n# 題\n"),
            "# 題\n"
        );
        // 外したあとの先頭の空行も落とす
        assert_eq!(
            strip_frontmatter("---\ntitle: A\n---\n\n# 題\n"),
            "# 題\n"
        );
        // 閉じる `---` が無い
        assert_eq!(
            strip_frontmatter("---\ntitle: A\n# 題\n"),
            "---\ntitle: A\n# 題\n"
        );
        // frontmatter が無い
        assert_eq!(strip_frontmatter("# 題\n"), "# 題\n");
        assert_eq!(strip_frontmatter(""), "");
        // `---` だけじゃない最初の行は frontmatter にしない
        assert_eq!(strip_frontmatter("---x\n---\n# 題\n"), "---x\n---\n# 題\n");
        // CRLF でも外す
        assert_eq!(strip_frontmatter("---\r\ntitle: A\r\n---\r\n# 題\r\n"), "# 題\r\n");
    }

    /// 包みの見出しは `###`、中身は frontmatter を外して 3 段下げられる（AC4）。
    #[test]
    fn test_wrap_token_content_new_shape() {
        let content = "---\ntitle: A\n---\n\n# 題\n\n```bash\n# c\n```";
        let out = wrap_token_content("AbCdEfGh", "plan-block", content);

        assert!(
            out.starts_with("<!-- waggle:token:AbCdEfGh block:plan-block -->\n\n### 出力トークン: AbCdEfGh (plan-block)\n\n"),
            "{out}"
        );
        assert!(out.ends_with("\n\n<!-- /waggle:token:AbCdEfGh -->"), "{out}");
        // 中身は frontmatter が無く 3 段下げられている
        assert!(out.contains("\n#### 題\n"), "{out}");
        assert!(!out.contains("title: A"), "{out}");
        // コードブロックの中の `#` は変わらない
        assert!(out.contains("```bash\n# c\n```"), "{out}");
        assert!(!out.contains("\n## 出力トークン"), "{out}");
    }

    /// 値が無い id は `expand_placeholders` と同じ扱いで `{{input.<id>}}` のまま入る。
    #[test]
    fn test_split_fixed_and_variable_missing_value_keeps_placeholder() {
        let values = HashMap::new();
        let (fixed, first) = split_fixed_and_variable("{{input.name}}", &values, None);
        assert!(fixed.contains("（最初のメッセージの「# name」の節にある）"));
        assert_eq!(first, "# name\n\n{{input.name}}");
    }

    /// 節が 1 つも無い本文（固定部分は実体あり）→ 最初の文は案内の文。
    #[test]
    fn test_split_fixed_and_variable_no_sections() {
        let values = HashMap::new();
        let (fixed, first) = split_fixed_and_variable("# 本文だけ", &values, None);
        assert_eq!(fixed, "# 本文だけ");
        assert_eq!(first, FIRST_MESSAGE_FALLBACK);
    }

    /// 本文が空なら両方が空（従来どおり何も送らない）。
    #[test]
    fn test_split_fixed_and_variable_empty_body() {
        let values = HashMap::new();
        let (fixed, first) = split_fixed_and_variable("", &values, None);
        assert!(fixed.is_empty());
        assert!(first.is_empty());
    }

    /// context が空文字なら `# context` の節は作らない（範囲が無いので）。
    #[test]
    fn test_split_fixed_and_variable_blank_context() {
        let values = HashMap::new();
        let (fixed, first) = split_fixed_and_variable("{{context}}", &values, Some("  \n"));
        assert_eq!(fixed, "（最初のメッセージの「# context」の節にある）");
        assert_eq!(first, FIRST_MESSAGE_FALLBACK);
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
