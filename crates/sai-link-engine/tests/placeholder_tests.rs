//! # プレースホルダー展開の統合テスト
//!
//! `{{input.<id>}}` 形式のプレースホルダー展開の単体テスト。
//! `sai_link_engine::placeholder` モジュールの公開 API を直接テストします。
//!
//! 本テストファイルは `src/placeholder.rs` 内のユニットテストを
//! 補完するもので、異常系・境界値・特殊文字を含むテストを追加します。

use sai_link_engine::placeholder::{collect_placeholders, expand_placeholders};
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// expand_placeholders のテスト
// ---------------------------------------------------------------------------

/// 単一のプレースホルダーが正しく置換されることを確認。
#[test]
fn test_expand_single_placeholder() {
    let mut inputs = HashMap::new();
    inputs.insert("name".to_string(), "テスト".to_string());

    let result = expand_placeholders("{{input.name}}", &inputs);
    assert_eq!(result, "テスト");
}

/// 複数の異なるプレースホルダーがすべて置換されることを確認。
#[test]
fn test_expand_multiple_placeholders() {
    let mut inputs = HashMap::new();
    inputs.insert("feature".to_string(), "認証機能".to_string());
    inputs.insert("lang".to_string(), "Rust".to_string());

    let result = expand_placeholders("{{input.feature}} を {{input.lang}} で実装", &inputs);
    assert_eq!(result, "認証機能 を Rust で実装");
}

/// 同じプレースホルダーが複数回出現する場合、すべて同じ値で置換されることを確認。
#[test]
fn test_expand_same_placeholder_multiple_times() {
    let mut inputs = HashMap::new();
    inputs.insert("x".to_string(), "42".to_string());

    let result = expand_placeholders("{{input.x}} + {{input.x}} = {{input.x}}", &inputs);
    assert_eq!(result, "42 + 42 = 42");
}

/// 値が提供されていないプレースホルダーは元の表記のまま残ることを確認。
#[test]
fn test_expand_missing_placeholder() {
    let inputs = HashMap::new();
    let result = expand_placeholders("値: {{input.unknown}}", &inputs);
    assert_eq!(result, "値: {{input.unknown}}");
}

/// プレースホルダーがない文字列はそのまま返ることを確認。
#[test]
fn test_expand_no_placeholders() {
    let inputs = HashMap::new();
    let result = expand_placeholders("プレースホルダーなしの文字列", &inputs);
    assert_eq!(result, "プレースホルダーなしの文字列");
}

/// 空文字列がそのまま返ることを確認。
#[test]
fn test_expand_empty_template() {
    let inputs = HashMap::new();
    let result = expand_placeholders("", &inputs);
    assert_eq!(result, "");
}

/// 部分一致（プレフィックス共有）で誤展開されないことを確認。
///
/// `{{input.name_extra}}` は `{{input.name}}` とは別物として扱われる。
#[test]
fn test_expand_partial_match() {
    let mut inputs = HashMap::new();
    inputs.insert("name".to_string(), "基本".to_string());

    let result = expand_placeholders("{{input.name}}と{{input.name_extra}}", &inputs);
    assert_eq!(result, "基本と{{input.name_extra}}");
}

/// 空の値を持つプレースホルダーが正しく空文字列に置換されることを確認。
#[test]
fn test_expand_empty_value() {
    let mut inputs = HashMap::new();
    inputs.insert("empty".to_string(), "".to_string());

    let result = expand_placeholders("値: {{input.empty}}", &inputs);
    assert_eq!(result, "値: ");
}

/// 特殊文字（HTML, シェルメタ文字, 日本語）がエスケープされずに
/// そのまま置換されることを確認。
#[test]
fn test_expand_special_characters() {
    let mut inputs = HashMap::new();
    inputs.insert(
        "code".to_string(),
        "<script>alert('xss')</script>".to_string(),
    );
    inputs.insert("shell".to_string(), "'; rm -rf /; '".to_string());
    inputs.insert("jp".to_string(), "日本語のテスト".to_string());

    let result = expand_placeholders(
        "code={{input.code}} shell={{input.shell}} jp={{input.jp}}",
        &inputs,
    );
    assert_eq!(
        result,
        "code=<script>alert('xss')</script> shell='; rm -rf /; ' jp=日本語のテスト"
    );
}

/// 数値のような値が文字列として正しく置換されることを確認。
#[test]
fn test_expand_numeric_values() {
    let mut inputs = HashMap::new();
    inputs.insert("count".to_string(), "100".to_string());
    inputs.insert("ratio".to_string(), "3.14".to_string());

    let result = expand_placeholders("個数: {{input.count}}, 比率: {{input.ratio}}", &inputs);
    assert_eq!(result, "個数: 100, 比率: 3.14");
}

/// 長い文字列のプレースホルダー展開がパニックしないことを確認。
#[test]
fn test_expand_long_string() {
    let mut inputs = HashMap::new();
    let long_value = "a".repeat(10_000);
    inputs.insert("data".to_string(), long_value.clone());

    let result = expand_placeholders("{{input.data}}", &inputs);
    assert_eq!(result, long_value);
}

// ---------------------------------------------------------------------------
// collect_placeholders のテスト
// ---------------------------------------------------------------------------

/// 全プレースホルダー ID が重複なく収集されることを確認。
#[test]
fn test_collect_placeholders_unique() {
    let mut inputs = HashMap::new();
    inputs.insert("a".to_string(), "1".to_string());
    inputs.insert("b".to_string(), "2".to_string());

    let template = "a={{input.a}}, b={{input.b}}, a={{input.a}}";
    let placeholders = collect_placeholders(template);
    // a は2回出現しても1回だけ収集される
    assert_eq!(placeholders.len(), 2);
    assert!(placeholders.contains(&"a".to_string()));
    assert!(placeholders.contains(&"b".to_string()));
}

/// プレースホルダーがない場合、空の Vec が返ることを確認。
#[test]
fn test_collect_placeholders_empty() {
    let placeholders = collect_placeholders("プレースホルダーなし");
    assert!(placeholders.is_empty());
}

/// プレースホルダーがすべて入力値でカバーされている場合の収集結果を確認。
#[test]
fn test_collect_all_covered() {
    let mut inputs = HashMap::new();
    inputs.insert("name".to_string(), "test".to_string());
    inputs.insert("lang".to_string(), "rust".to_string());

    let template = "{{input.name}} in {{input.lang}}";
    let placeholders = collect_placeholders(template);
    assert_eq!(placeholders.len(), 2);
    assert!(placeholders.contains(&"name".to_string()));
    assert!(placeholders.contains(&"lang".to_string()));
}

/// 収集されたプレースホルダーと inputs のキーが一致することを確認（統合）。
#[test]
fn test_placeholder_key_matches_input() {
    let mut inputs = HashMap::new();
    inputs.insert("project_name".to_string(), "my-project".to_string());
    inputs.insert("architecture".to_string(), "microservices".to_string());

    let template = "プロジェクト: {{input.project_name}}, アーキテクチャ: {{input.architecture}}";
    let placeholders = collect_placeholders(template);

    for id in &placeholders {
        assert!(
            inputs.contains_key(id),
            "プレースホルダー '{}' の入力値が不足しています",
            id
        );
    }
}
