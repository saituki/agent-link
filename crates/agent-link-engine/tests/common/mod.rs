//! # テスト共通ユーティリティ
//!
//! agent-link の全テストファイルから使用される共通ヘルパー関数と
//! 定数を提供します。

use std::path::PathBuf;

/// フィクスチャディレクトリの絶対パスを取得する。
///
/// `CARGO_MANIFEST_DIR` を基準に `tests/fixtures/` を解決します。
///
/// # 戻り値
///
/// フィクスチャディレクトリの絶対パス
pub fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
}

/// フィクスチャファイルの絶対パスを返す。
///
/// # 引数
///
/// * `name` - フィクスチャファイル名（例: `"valid-minimal.md"`）
///
/// # 戻り値
///
/// フィクスチャファイルの絶対パス
///
/// # パニック
///
/// フィクスチャファイルが存在しない場合にパニックします。
pub fn fixture_path(name: &str) -> PathBuf {
    let path = fixtures_dir().join(name);
    assert!(
        path.exists(),
        "フィクスチャファイルが見つかりません: {}",
        path.display()
    );
    path
}

/// フィクスチャファイルの内容を文字列として読み込む。
///
/// # 引数
///
/// * `name` - フィクスチャファイル名
///
/// # 戻り値
///
/// ファイルの内容
pub fn read_fixture(name: &str) -> String {
    std::fs::read_to_string(fixture_path(name))
        .unwrap_or_else(|e| panic!("フィクスチャ '{}' の読み込みに失敗: {}", name, e))
}

/// 一時ディレクトリを作成し、スコープ終了時に自動削除する。
///
/// # 戻り値
///
/// 自動クリーンアップ付きの `TempDir`
pub fn setup_temp_dir() -> tempfile::TempDir {
    tempfile::tempdir().expect("一時ディレクトリの作成に失敗しました")
}

/// エラー結果に特定の部分文字列が含まれることをアサートする。
///
/// # 引数
///
/// * `result` - `Result<T, E>` の参照
/// * `expected_substr` - エラー表示に含まれているべき部分文字列
///
/// # パニック
///
/// - `result` が `Ok` の場合
/// - エラー文字列に `expected_substr` が含まれない場合
pub fn assert_err_contains<T, E>(result: &Result<T, E>, expected_substr: &str)
where
    E: std::fmt::Display,
{
    match result {
        Ok(_) => panic!("エラーが期待されましたが Ok が返されました"),
        Err(e) => {
            let err_str = format!("{}", e);
            assert!(
                err_str.contains(expected_substr),
                "エラー文字列に '{0}' が含まれていません: {1}",
                expected_substr,
                err_str
            );
        }
    }
}
