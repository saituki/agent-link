//! P1: 文中の waggle 参照を展開する機能の統合テスト。
//!
//! `task_text` のような 1 つのテキストの途中に `waggle:<トークン>` や
//! `waggle:<トークン>:path:…` が現れたとき、その部分が文書の本文へ
//! 置き換わること（再帰展開しないこと）を検証する。

use std::collections::HashMap;
use std::path::PathBuf;

use agent_link_engine::placeholder::{
    resolve_placeholder_values, shift_headings, strip_frontmatter, PlaceholderValue,
};
use agent_link_engine::waggle_integration::WaggleIntegrator;

/// テスト用フィクスチャの絶対パスを取得する。
fn fixture_path(name: &str) -> PathBuf {
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push("tests");
    path.push("fixtures");
    path.push(name);
    path
}

/// マーカー付きのブロックに埋め込まれる本文（frontmatter を外して 3 段下げたもの）。
fn embedded_body(content: &str) -> String {
    shift_headings(strip_frontmatter(content), 3).trim_end().to_string()
}

/// インメモリ DB のテスト用インテグレーターを作成する。
fn test_integrator() -> WaggleIntegrator {
    WaggleIntegrator::open_in_memory().expect("インメモリ DB の作成に失敗")
}

/// `sample_output.md` をトークン化し、(トークン文字列, 本文) を返す。
fn tokenize_sample(integrator: &WaggleIntegrator) -> (String, String) {
    let path = fixture_path("sample_output.md");
    let content = std::fs::read_to_string(&path).expect("フィクスチャの読み込みに失敗");
    let result = integrator
        .tokenize_file(&path, "plan-block", Some("実装計画書"))
        .expect("トークン化に失敗");
    (result.token.as_str().to_string(), content)
}

/// `task_text` 1 件を解決し、(解決後の文字列, 記録されたトークン一覧) を返す。
fn resolve_one(text: &str, integrator: &WaggleIntegrator) -> (String, Vec<String>) {
    let mut values: HashMap<String, PlaceholderValue> = HashMap::new();
    values.insert("task_text".to_string(), PlaceholderValue::from_string(text));
    let (resolved, tokens) = resolve_placeholder_values(&values, integrator);
    (resolved["task_text"].clone(), tokens)
}

/// 文中の単独参照 `下記を実装せよ。waggle:<tok>` が本文に展開される。
#[test]
fn test_inline_waggle_embedded_in_sentence() {
    let integrator = test_integrator();
    let (token, content) = tokenize_sample(&integrator);

    let (resolved, tokens) =
        resolve_one(&format!("下記を実装せよ。waggle:{}", token), &integrator);

    assert!(
        resolved.contains(&embedded_body(&content)),
        "本文が埋め込まれるべき: {resolved}"
    );
    // 包みの見出しは `###`、中身は 3 段下げられている
    assert!(resolved.contains("### 出力トークン: "), "{resolved}");
    assert!(resolved.contains("#### テストブロック出力"), "{resolved}");
    assert!(!resolved.contains("\n# テストブロック出力"), "{resolved}");
    assert_eq!(tokens, vec![token.clone()]);
    // 生の参照記法は残らない（マーカーの `waggle:token:` は本文側）
    assert!(
        !resolved.contains(&format!("waggle:{}", token)),
        "生の参照記法が残ってはいけない"
    );
}

/// `waggle:<tok> と比較せよ` のように参照の後に文が続いても、
/// エラー文字列にならず本文が埋め込まれる（回帰テスト）。
#[test]
fn test_inline_waggle_followed_by_text() {
    let integrator = test_integrator();
    let (token, content) = tokenize_sample(&integrator);

    let (resolved, tokens) =
        resolve_one(&format!("waggle:{} と比較せよ", token), &integrator);

    assert!(!resolved.contains("形式が不正"), "回帰: {}", resolved);
    assert!(!resolved.contains("解決に失敗"), "回帰: {}", resolved);
    assert!(resolved.contains(&embedded_body(&content)));
    assert_eq!(tokens, vec![token]);
}

/// `waggle:<tok>:path:…` は `:path:…` も含めて本文に置き換わる。
#[test]
fn test_inline_waggle_with_path_reference() {
    let integrator = test_integrator();
    let (token, content) = tokenize_sample(&integrator);

    let (resolved, tokens) = resolve_one(
        &format!("waggle:{}:path:.agent-link/outputs/*/1/a.md を読め", token),
        &integrator,
    );

    assert!(!resolved.contains(":path:"), ":path: も消えるべき: {resolved}");
    assert!(resolved.contains(&embedded_body(&content)));
    assert_eq!(tokens, vec![token]);
}

/// 値が `waggle:<tok>:path:…` だけでも、トークン列ではなく参照として本文に置き換わる。
#[test]
fn test_standalone_path_reference() {
    let integrator = test_integrator();
    let (token, content) = tokenize_sample(&integrator);

    let (resolved, tokens) = resolve_one(
        &format!("waggle:{}:path:.agent-link/outputs/*/1/a.md", token),
        &integrator,
    );

    assert!(!resolved.contains(":path:"), ":path: も消えるべき: {resolved}");
    assert!(resolved.contains(&embedded_body(&content)));
    assert_eq!(tokens, vec![token]);
}

/// 存在しないトークンは失敗メッセージに置き換わる（U08: どのプロジェクトで
/// 探したかが分かる文になる）。
#[test]
fn test_inline_waggle_unresolvable_token() {
    let integrator = test_integrator();

    let (resolved, tokens) = resolve_one("waggle:11111111 を読め", &integrator);

    assert!(resolved.contains("の記録にありません"), "{}", resolved);
    assert!(resolved.contains("11111111"));
    // 解決を試みたトークンは記録される
    assert_eq!(tokens, vec!["11111111".to_string()]);
}

/// waggle 参照を含まないテキストはそのまま残る。
#[test]
fn test_plain_text_without_waggle_is_unchanged() {
    let integrator = test_integrator();

    let (resolved, tokens) = resolve_one("ただのテキスト", &integrator);

    assert_eq!(resolved, "ただのテキスト");
    assert!(tokens.is_empty());
}

/// 埋め込んだ文書の中の `waggle:` は再帰展開しない。
#[test]
fn test_inline_waggle_is_not_recursive() {
    let integrator = test_integrator();

    // 外側の文書の本文に、別トークン参照を書いておく
    let inner_path = std::env::temp_dir().join("sai_link_inline_waggle_inner.md");
    std::fs::write(
        &inner_path,
        "内側の本文\nwaggle:11111111 は展開されない\n",
    )
    .expect("一時ファイルの書き込みに失敗");

    let outer = integrator
        .tokenize_file(&inner_path, "outer-block", None)
        .expect("トークン化に失敗");

    let (resolved, tokens) =
        resolve_one(&format!("参照: waggle:{}", outer.token.as_str()), &integrator);

    assert!(resolved.contains("内側の本文"));
    assert!(
        resolved.contains("waggle:11111111"),
        "埋め込んだ文書の中の waggle: は再帰展開されないべき: {resolved}"
    );
    assert_eq!(tokens, vec![outer.token.as_str().to_string()]);

    let _ = std::fs::remove_file(&inner_path);
}
