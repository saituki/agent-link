//! `sai-moon-paper init` が研究プロジェクトに配る論文の部品。
//!
//! - `.sai/template/*.md` を書き出す。既にあるファイルは上書きしない
//!   （利用者が直したものを消さない）。
//! - cc（Claude Code）用に、プロジェクトの `.mcp.json` に `sai-paper` の MCP サーバーを足す。
//!   既にある項目は消さない。
//!
//! 研究の部品（研究方針の木のブロック・共通の決まり・分野の観点・実行先・道具）は
//! `sai-sci` が配る。sai-paper は sai-sci を知らない（要件定義書 A8）。

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde_json::{Value, json};

/// 配るテンプレート（ファイル名, 中身）。
const TEMPLATES: &[(&str, &str)] = &[
    ("literature-hard-rules.md", include_str!("../assets/template/literature-hard-rules.md")),
    ("paper-screening.md", include_str!("../assets/template/paper-screening.md")),
    ("paper-search-strategy.md", include_str!("../assets/template/paper-search-strategy.md")),
    ("rag-deep-search.md", include_str!("../assets/template/rag-deep-search.md")),
    ("rag-simple.md", include_str!("../assets/template/rag-simple.md")),
    ("search-external.md", include_str!("../assets/template/search-external.md")),
    ("search-internal.md", include_str!("../assets/template/search-internal.md")),
];

/// JSON ファイルを読む（無ければ空のオブジェクト）。
fn read_json(path: &Path) -> Result<Value> {
    match std::fs::read_to_string(path) {
        Ok(text) if !text.trim().is_empty() => {
            serde_json::from_str(&text).with_context(|| format!("{} を読めません", path.display()))
        }
        _ => Ok(json!({})),
    }
}

/// JSON ファイルを書く。
fn write_json(path: &Path, value: &Value) -> Result<()> {
    let mut text = serde_json::to_string_pretty(value)?;
    text.push('\n');
    std::fs::write(path, text)?;
    Ok(())
}

/// 配った結果（部品の配り方の「作成・そのまま・更新」）。
#[derive(Debug)]
enum WriteOutcome {
    /// 無かったので書いた。
    Created,
    /// 既に新しい版と同じだった（何もしない）。
    Unchanged,
    /// 古い版を退避して新しい版を書いた（退避先のパス）。
    Updated(PathBuf),
}

/// 更新のときに作る退避先（`<名前>.bak`、既にあれば `<名前>.bak.<YYYYmmdd-HHMMSS>`）。
fn backup_path(path: &Path) -> PathBuf {
    let with_suffix = |suffix: &str| {
        let mut name = path.file_name().map_or_else(std::ffi::OsString::new, |n| n.to_os_string());
        name.push(suffix);
        path.with_file_name(name)
    };
    let plain = with_suffix(".bak");
    if !plain.exists() {
        return plain;
    }
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
    with_suffix(&format!(".bak.{stamp}"))
}

/// 部品を 1 つ配る。`update` のときは古い物を退避して新しい版に置き換える。
fn write_asset(path: &Path, content: &str, update: bool) -> Result<WriteOutcome> {
    if !path.exists() {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, content)?;
        return Ok(WriteOutcome::Created);
    }
    if !update {
        return Ok(WriteOutcome::Unchanged);
    }
    let old = std::fs::read_to_string(path).unwrap_or_default();
    if old == content {
        return Ok(WriteOutcome::Unchanged);
    }
    let backup = backup_path(path);
    std::fs::rename(path, &backup)?;
    std::fs::write(path, content)?;
    Ok(WriteOutcome::Updated(backup))
}

/// 研究プロジェクト（`root`）に部品を配る。何をしたかの一覧を返す（表示用）。
///
/// `sai_paper_bin` は `.mcp.json` に書く `sai-paper` の実行ファイル。
/// `update` が `true` のときは、古いテンプレートを退避して新しい版に置き換える。
///
/// # Errors
/// ファイルの読み書きに失敗したとき。
pub fn install(root: &Path, sai_paper_bin: &Path, update: bool) -> Result<Vec<String>> {
    let mut done = Vec::new();
    let sai = root.join(".sai");

    let mut created = 0usize;
    let mut updated: Vec<PathBuf> = Vec::new();
    for (name, content) in TEMPLATES {
        match write_asset(&sai.join("template").join(name), content, update)? {
            WriteOutcome::Created => created += 1,
            WriteOutcome::Unchanged => {}
            WriteOutcome::Updated(backup) => updated.push(backup),
        }
    }
    done.push(format!("テンプレートを {created} 件置きました（既にあるものはそのまま）"));
    for backup in &updated {
        done.push(format!("更新しました（古い物: {}）", backup.display()));
    }

    // cc 用: .mcp.json に sai-paper の MCP サーバーを足す（既にあれば触らない）
    let mcp_path = root.join(".mcp.json");
    let mut mcp = read_json(&mcp_path)?;
    let servers = mcp
        .as_object_mut()
        .context(".mcp.json がオブジェクトではありません")?
        .entry("mcpServers")
        .or_insert_with(|| json!({}));
    if let Some(servers) = servers.as_object_mut()
        && !servers.contains_key("sai-paper")
    {
        servers.insert(
            "sai-paper".to_string(),
            json!({
                "command": sai_paper_bin.to_string_lossy(),
                "args": ["mcp"],
                "env": {"SAI_PROJECT_ROOT": root.to_string_lossy()},
            }),
        );
        write_json(&mcp_path, &mcp)?;
        done.push(".mcp.json に sai-paper の MCP サーバーを足しました".to_string());
    }
    Ok(done)
}

/// この PC の拡張を探して配る（`sai-moon-paper init` から呼ぶ）。
///
/// # Errors
/// ファイルの読み書きに失敗したとき。
pub fn install_here(root: &Path, update: bool) -> Result<Vec<String>> {
    let bin = std::env::current_exe()
        .and_then(std::fs::canonicalize)
        .unwrap_or_else(|_| PathBuf::from("sai-moon-paper"));
    install(root, &bin, update)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 部品を配ると、テンプレートと `.mcp.json` ができる。
    /// 既にあるファイルは残り、2 回目も重複しない。研究の部品（block・pre_run・
    /// 分野の観点・`project-brief.md`）は作らない。
    #[test]
    fn install_writes_templates_and_mcp_without_research_parts() {
        let root = std::env::temp_dir().join(format!("sai-paper-assets-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join(".sai/template")).unwrap();
        std::fs::write(root.join(".sai/template/rag-simple.md"), "利用者が直した").unwrap();
        std::fs::write(
            root.join(".mcp.json"),
            r#"{"mcpServers": {"other": {"command": "x"}}}"#,
        )
        .unwrap();
        for _ in 0..2 {
            install(&root, Path::new("/bin/sai-paper"), false).unwrap();
        }

        assert_eq!(
            std::fs::read_to_string(root.join(".sai/template/rag-simple.md")).unwrap(),
            "利用者が直した"
        );
        for (name, content) in TEMPLATES {
            let path = root.join(".sai/template").join(name);
            assert!(path.is_file(), "{name}");
            assert!(!content.contains("/Users/"), "{name} に絶対パスが無い");
        }
        // 研究の部品は作らない。
        assert!(!root.join(".sai/blocks").exists(), "block を作っている");
        assert!(!root.join(".sai/template/domain.md").exists(), "domain.md を作っている");
        assert!(
            !root.join(".sai/template/project-brief.md").exists(),
            "project-brief.md を作っている"
        );
        assert!(!root.join(".sai/paper/mind.md").exists(), "mind.md を作っている");
        // `.sai/settings.json` には何も書かない（pre_run を作らない）。
        assert!(!root.join(".sai/settings.json").exists(), "settings.json を作っている");

        let mcp = read_json(&root.join(".mcp.json")).unwrap();
        assert_eq!(mcp["mcpServers"]["other"]["command"], "x");
        assert_eq!(mcp["mcpServers"]["sai-paper"]["args"][0], "mcp");
        std::fs::remove_dir_all(&root).ok();
    }

    /// `install --update` はテンプレートを入れ替えて `.bak` を作る。
    #[test]
    fn install_update_replaces_template_and_makes_a_backup() {
        let root = std::env::temp_dir().join(format!("sai-paper-update-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join(".sai/template")).unwrap();
        std::fs::write(root.join(".sai/template/rag-simple.md"), "利用者が直した\n").unwrap();

        install(&root, Path::new("/bin/sai-paper"), true).unwrap();

        let shipped = include_str!("../assets/template/rag-simple.md");
        assert_eq!(
            std::fs::read_to_string(root.join(".sai/template/rag-simple.md")).unwrap(),
            shipped
        );
        assert_eq!(
            std::fs::read_to_string(root.join(".sai/template/rag-simple.md.bak")).unwrap(),
            "利用者が直した\n"
        );
        std::fs::remove_dir_all(&root).ok();
    }
}
