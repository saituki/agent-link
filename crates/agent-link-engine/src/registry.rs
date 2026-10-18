//! 登録簿 `$AGENT_LINK_HOME/registry.json` の読み書き。
//!
//! プロジェクトの登録名を引く（書き出し先のフォルダ名になる）。
//! `agent-link init` が足す。読み手は Web（`apps/web/src/agent_link_web/registry.py`）。

use std::path::{Path, PathBuf};

/// パス文字列を比較用に正規化する（末尾スラッシュ除去 + 可能なら canonicalize）。
fn normalize_path_str(p: &Path) -> String {
    let raw = p.to_string_lossy().to_string();
    let trimmed = raw.trim_end_matches('/').to_string();
    match std::fs::canonicalize(&trimmed) {
        Ok(c) => c.to_string_lossy().trim_end_matches('/').to_string(),
        Err(_) => trimmed,
    }
}

/// フォルダ名（プロジェクトルートの最終パス要素）。取れなければ `project`。
fn folder_name(project_root: &Path) -> String {
    project_root
        .file_name()
        .and_then(|s| s.to_str())
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .unwrap_or("project")
        .to_string()
}

/// レジストリ JSON（`~/.agent-link/registry.json`）からプロジェクト名を引く。
///
/// `root` がプロジェクトルートと一致する項目の `name` を返す（パスは正規化して比較）。
/// 見つからなければ `None`。
pub fn project_name_from_registry(registry: &Path, project_root: &Path) -> Option<String> {
    let text = std::fs::read_to_string(registry).ok()?;
    let value: serde_json::Value = serde_json::from_str(&text).ok()?;
    let entries = value.as_array()?;
    let target = normalize_path_str(project_root);
    for e in entries {
        let Some(root) = e.get("root").and_then(|r| r.as_str()) else {
            continue;
        };
        if normalize_path_str(Path::new(root)) != target {
            continue;
        }
        if let Some(name) = e.get("name").and_then(|n| n.as_str()) {
            if !name.trim().is_empty() {
                return Some(name.trim().to_string());
            }
        }
    }
    None
}

/// レジストリ（任意）を使ってプロジェクト名を決める。
///
/// 一致が無ければルートのフォルダ名（例 `agent-link`）。
pub fn project_name_using(registry: Option<&Path>, project_root: &Path) -> String {
    if let Some(reg) = registry {
        if let Some(name) = project_name_from_registry(reg, project_root) {
            return name;
        }
    }
    folder_name(project_root)
}

/// 登録簿の項目を (name, root) で返す（ファイルの順）。読めない・配列でなければ空。
///
/// `name` と `root` が文字列の項目だけを返す。
pub fn list_entries(registry: &Path) -> Vec<(String, PathBuf)> {
    let Some(value) = std::fs::read_to_string(registry)
        .ok()
        .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
    else {
        return Vec::new();
    };
    let Some(entries) = value.as_array() else {
        return Vec::new();
    };
    entries
        .iter()
        .filter_map(|e| {
            let name = e.get("name")?.as_str()?;
            let root = e.get("root")?.as_str()?;
            Some((name.to_string(), PathBuf::from(root)))
        })
        .collect()
}

/// 登録簿に足した結果。
#[derive(Debug, PartialEq, Eq)]
pub enum RegistryAdd {
    /// 新しく足した。
    Added,
    /// 同じルートが既にある（その登録名）。
    AlreadyRegistered(String),
}

/// レジストリ JSON（`~/.agent-link/registry.json`）にプロジェクトを足す（`agent-link init` 用）。
///
/// 同じルートが既にあれば何もしない。同じ名前で別のルートがあればエラーにする
/// （apps/web は名前でプロジェクトを引くため）。ファイルが無ければ作る。
///
/// # Errors
/// 読み書きに失敗したとき、ファイルが配列でないとき、名前が他のルートで使われているとき。
pub fn register_project(registry: &Path, name: &str, project_root: &Path) -> Result<RegistryAdd, String> {
    let mut entries: Vec<serde_json::Value> = match std::fs::read_to_string(registry) {
        Ok(text) if !text.trim().is_empty() => serde_json::from_str::<serde_json::Value>(&text)
            .ok()
            .and_then(|v| v.as_array().cloned())
            .ok_or_else(|| format!("{} is not a JSON array.", registry.display()))?,
        Ok(_) => Vec::new(),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(e) => return Err(format!("Cannot read {}: {e}", registry.display())),
    };
    let target = normalize_path_str(project_root);
    for e in &entries {
        let root = e.get("root").and_then(|r| r.as_str()).unwrap_or_default();
        let entry_name = e.get("name").and_then(|n| n.as_str()).unwrap_or_default();
        if normalize_path_str(Path::new(root)) == target {
            return Ok(RegistryAdd::AlreadyRegistered(entry_name.to_string()));
        }
        if entry_name == name {
            return Err(format!(
                "Name {name} is already used by another folder ({root}). Choose another name with --name."
            ));
        }
    }
    let now = chrono::Utc::now().to_rfc3339();
    entries.push(serde_json::json!({
        "name": name,
        "root": project_root.to_string_lossy(),
        "kind": "link",
        "color": null,
        "created_at": now,
        "last_opened": now,
    }));
    if let Some(parent) = registry.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("Cannot create {}: {e}", parent.display()))?;
    }
    let text = serde_json::to_string_pretty(&entries).map_err(|e| e.to_string())?;
    std::fs::write(registry, text + "\n").map_err(|e| format!("Cannot write {}: {e}", registry.display()))?;
    Ok(RegistryAdd::Added)
}

/// 現在のプロジェクトのプロジェクト名（`~/.agent-link/registry.json` → フォルダ名）。
pub fn project_name_for(project_root: &Path) -> String {
    let registry = crate::project::sai_home().map(|h| h.join("registry.json"));
    project_name_using(registry.as_deref(), project_root)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// レジストリに一致が無いときフォルダ名になる（00 §3.2 project_name）。
    #[test]
    fn project_name_falls_back_to_folder_name() {
        let tmp = std::env::temp_dir().join(format!(
            "registry_test_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let root = tmp.join("my-project");
        std::fs::create_dir_all(&root).unwrap();
        let registry = tmp.join("registry.json");
        // 別プロジェクトのみ登録（一致なし）
        std::fs::write(
            &registry,
            r#"[{"name":"other","root":"/nowhere/other"}]"#,
        )
        .unwrap();
        assert_eq!(project_name_using(Some(&registry), &root), "my-project");

        // 登録があれば name が使われる（末尾スラッシュ差は吸収）
        let root_str = root.to_string_lossy().to_string();
        std::fs::write(
            &registry,
            format!(
                r#"[{{"name":"registered-name","root":"{}/"}}]"#,
                root_str.replace('\\', "\\\\")
            ),
        )
        .unwrap();
        assert_eq!(project_name_using(Some(&registry), &root), "registered-name");

        // レジストリが無ければフォルダ名
        assert_eq!(project_name_using(None, &root), "my-project");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// 登録簿への追加: 無ければ作り、同じルートは増やさず、同じ名前の別ルートは断る。
    #[test]
    fn register_project_adds_once_and_rejects_name_clash() {
        let tmp = std::env::temp_dir().join(format!(
            "registry_test_add_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let root = tmp.join("practice");
        std::fs::create_dir_all(&root).unwrap();
        let registry = tmp.join("home/registry.json");

        assert_eq!(register_project(&registry, "practice", &root), Ok(RegistryAdd::Added));
        assert_eq!(project_name_using(Some(&registry), &root), "practice");
        let text = std::fs::read_to_string(&registry).unwrap();
        assert!(text.contains("\"kind\": \"link\""), "{text}");

        assert_eq!(
            register_project(&registry, "practice", &root),
            Ok(RegistryAdd::AlreadyRegistered("practice".to_string()))
        );
        let other = tmp.join("other");
        std::fs::create_dir_all(&other).unwrap();
        assert!(register_project(&registry, "practice", &other).is_err());
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
