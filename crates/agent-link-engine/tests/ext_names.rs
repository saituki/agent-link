//! base（agent-link 本体）のソースに、拡張の名前が出ないことを確かめる。
//!
//! 要件: base は拡張の名前を知らない。拡張の名前は拡張のリポジトリにだけ置く。
//! 大文字と小文字を区別せず、禁止語を `contains` で確かめる（新しい依存は足さない）。

use std::fs;
use std::path::{Path, PathBuf};

/// base に出てはいけない語（拡張の名前と、その拡張のデータの名前）。
const FORBIDDEN: &[&str] = &["mlflow", "sai-paper", "sai-sci", "sai-ts", "papr.db"];

/// 検査する場所（`CARGO_MANIFEST_DIR` からの相対）。
const SCANNED: &[&str] = &[
    "src",
    "../agent-link-view/src",
    "../agent-link-cli/src",
    "assets",
    "../../apps/web/src",
    "../../apps/web/hooks",
    "../../scripts",
];

/// フォルダの中のファイルを再帰で集める。
fn collect_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries =
        fs::read_dir(dir).unwrap_or_else(|e| panic!("{} を読めません: {}", dir.display(), e));
    for entry in entries {
        let path = entry.expect("ディレクトリの項目を読めません").path();
        if path.is_dir() {
            collect_files(&path, out);
        } else {
            out.push(path);
        }
    }
}

#[test]
fn base_source_names_no_extension() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    for rel in SCANNED {
        let dir = root.join(rel);
        assert!(dir.is_dir(), "{} がありません", dir.display());
        collect_files(&dir, &mut files);
    }
    files.sort();

    let mut violations = Vec::new();
    for file in &files {
        // テキストとして読めない物（バイナリ・画像）は飛ばす
        let Ok(text) = fs::read_to_string(file) else {
            continue;
        };
        for (i, line) in text.lines().enumerate() {
            let lower = line.to_lowercase();
            for word in FORBIDDEN {
                if lower.contains(word) {
                    violations.push(format!(
                        "{}:{}: {}（{}）",
                        file.display(),
                        i + 1,
                        word,
                        line.trim()
                    ));
                }
            }
        }
    }
    assert!(
        violations.is_empty(),
        "base に拡張の名前が残っています:\n{}",
        violations.join("\n")
    );
}
