//! sai-sci の block・template・README・TUTORIAL・道具・実行先に、MCP の道具名や古い命令名が
//! 残っていないことを確かめる（U21）。
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
    "sci_propose",
    "sci_context",
    "sci_status",
    "sci_node",
    "sai-moon-link",
    "sai-moon-paper",
    "ask_user_phone",
    "phone_api",
    "--executer professor",
    "SAI_SAI_HOME",
    ".sai-moon",
];

/// `dir` の下のファイルを集める（`ext` の拡張子だけ。`None` は全部）。
/// `__pycache__` などの生成物は読まない。
fn collect(dir: &Path, ext: Option<&str>, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if path.is_dir() {
            if name == "__pycache__" {
                continue;
            }
            collect(&path, ext, out);
        } else if name.ends_with(".pyc") {
            continue;
        } else if ext.is_none() || path.extension().and_then(|e| e.to_str()) == ext {
            out.push(path);
        }
    }
}

fn check(files: &[PathBuf], root: &Path, violations: &mut Vec<String>) {
    for file in files {
        let Ok(text) = fs::read_to_string(file) else {
            continue;
        };
        for (i, line) in text.lines().enumerate() {
            for word in FORBIDDEN {
                if line.contains(word) {
                    violations.push(format!(
                        "{}:{}: {}（{}）",
                        file.strip_prefix(root).unwrap_or(file).display(),
                        i + 1,
                        word,
                        line.trim()
                    ));
                }
            }
        }
    }
}

#[test]
fn sai_sci_assets_have_no_mcp_tool_or_old_command_names() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../sai-sci");
    let mut files = Vec::new();
    collect(&root.join("blocks"), Some("md"), &mut files);
    collect(&root.join("template"), Some("md"), &mut files);
    for name in ["README.md", "TUTORIAL.md"] {
        let path = root.join(name);
        assert!(path.is_file(), "{name} がありません");
        files.push(path);
    }
    // 道具は `tools/*.py`（写しの `eda/`・`biosignal/` は対象外）。
    let entries = fs::read_dir(root.join("tools")).expect("sai-sci/tools を読めます");
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("py") {
            files.push(path);
        }
    }
    collect(&root.join("exec"), None, &mut files);
    assert!(!files.is_empty(), "sai-sci に読むファイルがありません");

    let mut violations = Vec::new();
    check(&files, &root, &mut violations);
    assert!(
        violations.is_empty(),
        "禁止語が残っています:\n{}",
        violations.join("\n")
    );
}
