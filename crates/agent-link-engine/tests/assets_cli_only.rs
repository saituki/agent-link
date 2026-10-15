//! 基盤の block・template に MCP の道具名や古い命令名が残っていないことを確かめる（U20）。
//!
//! 正規表現のために新しい依存を足さないため、禁止語を `contains` で確かめる。

use std::fs;
use std::path::{Path, PathBuf};

/// MCP の道具名・古い命令名（禁止語）。
const FORBIDDEN: &[&str] = &[
    "mcp__sai",
    "sai_write",
    "sai_read",
    "sai_finalize",
    "task_create",
    "task_execute",
    "task_output",
    "task_manage",
    "task_report",
    "task_status",
    "task_capture",
    "task_list",
    "block_discover",
    "block_inspect",
    "sai-moon-link",
    "sai-moon-paper",
    "ask_user_phone",
    "mcp serve",
];

/// `assets/` の下の `*.md` をすべて集める。
fn collect_markdown(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries =
        fs::read_dir(dir).unwrap_or_else(|e| panic!("{} を読めません: {}", dir.display(), e));
    for entry in entries {
        let entry = entry.expect("ディレクトリの項目を読めません");
        let path = entry.path();
        if path.is_dir() {
            collect_markdown(&path, out);
        } else if path.extension().and_then(|e| e.to_str()) == Some("md") {
            out.push(path);
        }
    }
}

#[test]
fn assets_have_no_mcp_tool_or_old_command_names() {
    let assets = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets");
    let mut files = Vec::new();
    collect_markdown(&assets, &mut files);
    assert!(!files.is_empty(), "assets に .md がありません");

    let mut violations = Vec::new();
    for file in &files {
        let text = fs::read_to_string(file)
            .unwrap_or_else(|e| panic!("{} を読めません: {}", file.display(), e));
        for (i, line) in text.lines().enumerate() {
            for word in FORBIDDEN {
                if line.contains(word) {
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
        "禁止語が残っています:\n{}",
        violations.join("\n")
    );
}
