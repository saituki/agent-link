//! # waggle 統合の統合テスト
//!
//! 外部クレート統合テストとして、`WaggleIntegrator` の公開 API を
//! 実際の SQLite ストアを使用して検証します。
//! （ユニットテストは waggle_integration.rs の mod tests に実装済み）

use std::path::PathBuf;

use sai_link_engine::placeholder::{
    expand_placeholders, resolve_placeholder_values, PlaceholderValue,
};
use sai_link_engine::waggle_integration::WaggleIntegrator;

use std::collections::HashMap;

/// テスト用フィクスチャの絶対パスを取得する。
fn fixture_path(name: &str) -> PathBuf {
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push("tests");
    path.push("fixtures");
    path.push(name);
    path
}

/// インメモリ DB のテスト用インテグレーターを作成する。
fn test_integrator() -> WaggleIntegrator {
    WaggleIntegrator::open_in_memory().expect("インメモリ DB の作成に失敗")
}

// ====================================================================
// 外部 API テスト
// ====================================================================

/// `WaggleIntegrator::open()` でファイルベースのストアが作成できる。
#[test]
fn test_open_file_based_store() {
    let dir = std::env::temp_dir();
    let db_path = dir.join("fn007_test_open.db");

    // テスト後クリーンアップ
    let _ = std::fs::remove_file(&db_path);

    let integrator = WaggleIntegrator::open(&db_path).expect("ファイルベース DB の作成に失敗");

    // トークン化が正常に動作する
    let path = fixture_path("sample_output.md");
    let result = integrator
        .tokenize_file(&path, "file-based-block", None)
        .expect("ファイルベース DB でのトークン化に失敗");
    assert!(!result.token.as_str().is_empty());

    // クリーンアップ
    let _ = std::fs::remove_file(&db_path);
}

/// `WaggleIntegrator::open_in_memory()` でインメモリストアが作成できる。
#[test]
fn test_open_in_memory_store() {
    let integrator = test_integrator();
    let path = fixture_path("sample_output.md");

    let result = integrator
        .tokenize_file(&path, "memory-block", None)
        .expect("インメモリ DB でのトークン化に失敗");
    assert!(!result.token.as_str().is_empty());
}

/// `with_sharer()` で sharer が上書きされる。
#[test]
fn test_with_sharer_override() {
    let custom_sharer =
        waggle_core::Sharer::new("custom-agent").expect("カスタム sharer 名の作成に失敗");
    let integrator = test_integrator().with_sharer(custom_sharer);
    let path = fixture_path("sample_output.md");

    let result = integrator
        .tokenize_file(&path, "sharer-test", None)
        .expect("トークン化に失敗");

    // トークンは正常に発行される
    assert!(!result.token.as_str().is_empty());
}

/// `store()` で内部ストアへの参照が取得できる。
#[test]
fn test_store_accessor() {
    let integrator = test_integrator();
    let store_ref = integrator.store();
    // Arc<SqliteStore> が取得できる
    assert!(std::sync::Arc::ptr_eq(store_ref, store_ref));
}

/// 大量のファイルをトークン化してもエラーが発生しない。
#[test]
fn test_bulk_tokenize() {
    let integrator = test_integrator();
    let path = fixture_path("sample_output.md");

    // 10回連続で同じファイルをトークン化
    // （waggle の nonce による重複排除が正しく動作する確認を含む）
    for i in 0..10 {
        let result = integrator
            .tokenize_file(&path, &format!("bulk-block-{}", i), None)
            .unwrap_or_else(|e| {
                panic!("{} 回目のトークン化に失敗: {}", i, e);
            });
        assert_eq!(result.token.as_str().len(), 8, "トークン長は8文字");
    }
}

/// シーケンシャルアクセステスト: トークン化 → 解決 → 解決 の一連の流れ。
#[test]
fn test_sequential_access_pattern() {
    let integrator = test_integrator();
    let path = fixture_path("sample_output.md");
    let original = std::fs::read_to_string(&path).expect("ファイル読み込みに失敗");

    // トークン化
    let result = integrator
        .tokenize_file(&path, "seq-block", Some("逐次アクセステスト"))
        .expect("トークン化に失敗");

    // トークン解決
    let resolved_url = integrator
        .resolve_target_url(result.token.as_str())
        .expect("URL 解決に失敗");
    assert!(resolved_url.as_str().contains("sample_output.md"));

    // 内容解決
    let resolved_content = integrator
        .resolve_content(result.token.as_str())
        .expect("内容解決に失敗");
    assert_eq!(resolved_content, original);

    // ブロック名検索
    let tokens = integrator
        .find_tokens_by_block("seq-block")
        .expect("ブロック名検索に失敗");
    assert_eq!(tokens.len(), 1);
    assert_eq!(tokens[0], result.token);
}

// ====================================================================
// エッジケーステスト
// ====================================================================

/// 存在しないファイルパスでエラーになる。
#[test]
fn test_tokenize_nonexistent_file() {
    let integrator = test_integrator();
    let path = PathBuf::from("/tmp/fn007_nonexistent_test_file.md");
    let result = integrator.tokenize_file(&path, "error-test", None);
    assert!(result.is_err(), "存在しないファイルでエラーになること");
}

/// ディレクトリパスを指定した場合のエラー。
#[test]
fn test_tokenize_directory_path() {
    let integrator = test_integrator();
    let dir = std::env::temp_dir();
    let result = integrator.tokenize_file(&dir, "dir-test", None);
    // ディレクトリはファイル読み込みでエラーになる
    assert!(result.is_err());
}

/// 空のトークン文字列での解決。
#[test]
fn test_resolve_empty_token() {
    let integrator = test_integrator();
    let result = integrator.resolve_content("");
    assert!(result.is_err(), "空のトークンでエラーになること");
}

// ====================================================================
// TokenizedOutput 構造体テスト
// ====================================================================

/// `TokenizedOutput` の全フィールドが正しく設定される。
#[test]
fn test_tokenized_output_fields() {
    let integrator = test_integrator();
    let path = fixture_path("sample_output.md");
    let _abs_path = path.canonicalize().unwrap_or_else(|_| path.clone());

    let result = integrator
        .tokenize_file(&path, "field-test", Some("フィールド確認"))
        .expect("トークン化に失敗");

    // token: 8文字の base58
    assert_eq!(result.token.as_str().len(), 8);

    // file_path: 正規化された絶対パス
    assert_eq!(result.file_path, path);

    // sha256: 64桁の小文字16進数
    assert_eq!(result.sha256.as_str().len(), 64);
    assert!(result
        .sha256
        .as_str()
        .chars()
        .all(|c| c.is_ascii_hexdigit()));

    // content_type: text/markdown
    assert_eq!(result.content_type, "text/markdown");

    // byte_size: 正の値
    assert!(result.byte_size > 0);
}

// ====================================================================
// 複数 waggle トークン入力の解決テスト（file_token 複数選択相当）
// ====================================================================

/// 複数 wt がカンマ区切り（"waggle:A,waggle:B"）で入力された場合、
/// 各トークンが文書本文に解決され、マーカー付きで連結されることを検証する。
/// （interactive.rs の file_token 複数選択 → prepare_pipeline の経路相当）
#[test]
fn test_resolve_multiple_waggle_tokens_input() {
    let integrator = test_integrator();

    // 異なる内容の 2 ファイルをトークン化
    let path_a = fixture_path("sample_output.md");
    let path_b = fixture_path("valid-minimal.md");
    let content_a = std::fs::read_to_string(&path_a).unwrap();
    let content_b = std::fs::read_to_string(&path_b).unwrap();

    let result_a = integrator
        .tokenize_file(&path_a, "multi-block-a", None)
        .expect("トークン化 A に失敗");
    let result_b = integrator
        .tokenize_file(&path_b, "multi-block-b", None)
        .expect("トークン化 B に失敗");

    // interactive.rs の複数選択と同じ形式でカンマ結合
    let joined = format!(
        "waggle:{},waggle:{}",
        result_a.token.as_str(),
        result_b.token.as_str()
    );

    let mut values: HashMap<String, PlaceholderValue> = HashMap::new();
    values.insert(
        "source_docs".to_string(),
        PlaceholderValue::from_string(&joined),
    );

    let (resolved, waggle_tokens) = resolve_placeholder_values(&values, &integrator);

    // 両トークンとも入力トークンとして記録される
    assert_eq!(waggle_tokens.len(), 2);
    assert!(waggle_tokens.contains(&result_a.token.as_str().to_string()));
    assert!(waggle_tokens.contains(&result_b.token.as_str().to_string()));

    // 解決値に両方の本文が含まれる（wt 文字列そのものは残らない）
    let resolved_text = &resolved["source_docs"];
    assert!(
        resolved_text.contains(&content_a),
        "トークン A の本文が含まれるべき"
    );
    assert!(
        resolved_text.contains(&content_b),
        "トークン B の本文が含まれるべき"
    );
    assert!(
        resolved_text.contains("waggle:token:"),
        "複数トークン時はマーカー付き連結"
    );

    // ブロック本文の {{input.*}} 展開まで通す（prepare_pipeline の Step 1 相当）
    let expanded = expand_placeholders(
        "以下の資料を踏まえて作業してください:\n\n{{input.source_docs}}",
        &resolved,
    );
    assert!(expanded.contains(&content_a));
    assert!(expanded.contains(&content_b));
    assert!(!expanded.contains("{{input."));
}

/// 単一 wt の場合はマーカー無しで本文のみが展開されることを検証する。
#[test]
fn test_resolve_single_waggle_token_input() {
    let integrator = test_integrator();
    let path = fixture_path("sample_output.md");
    let content = std::fs::read_to_string(&path).unwrap();

    let result = integrator
        .tokenize_file(&path, "single-block", None)
        .expect("トークン化に失敗");

    let mut values: HashMap<String, PlaceholderValue> = HashMap::new();
    values.insert(
        "doc".to_string(),
        PlaceholderValue::from_string(&format!("waggle:{}", result.token.as_str())),
    );

    let (resolved, waggle_tokens) = resolve_placeholder_values(&values, &integrator);

    assert_eq!(waggle_tokens, vec![result.token.as_str().to_string()]);
    // 単一トークン: 本文のみ（生の wt 文字列は含まれない）
    assert_eq!(resolved["doc"], content);
}
