//! 「開いた」「エージェントが読んだ」の記録（N01）。
//!
//! `activity_log` に残す 2 種類 `open` / `agent_read` の、パスの決め方と
//! 記録の作り方をまとめる。詳細な形（`detail` の JSON）は共有契約:
//!
//! - `open`: `{"target": "note|extract|mind|state|report|review|canvas", "path": "<相対パス>", "surface": "tui|sai-link|phone"}`
//! - `agent_read`: `{"target": "paper|note|extract|report", "via": "mcp|cli", "path": "<相対パス（分かれば）>"}`
//!
//! 同じ対象・同じ種類の記録は [`DEDUP_WINDOW_MINUTES`] 分以内なら 1 件にまとめる
//! （[`Database::record_activity_dedup`]）。

use std::path::{Component, Path, PathBuf};

use serde_json::json;

use crate::database::{Database, DatabaseError};
use crate::paths;

/// 同じ対象・同じ種類の記録を 1 件にまとめる時間（分）。
pub const DEDUP_WINDOW_MINUTES: i64 = 10;

/// パスから `open` の `target` を決める（共有契約）。
///
/// ファイル名で決まるもの（`note.md` / `extract.md` / `mind.md` / `state.md` /
/// `review.md`）を先に見て、次に拡張子 `.canvas`、最後に `.sai/outputs/` の下なら
/// `report` とする。どれにも当たらないときは `report` にする。
#[must_use]
pub fn open_target_for_path(root: &Path, path: &Path) -> &'static str {
    let _ = root; // 将来の拡張のため受け取る（いまはパスの形だけで決まる）
    let name = path.file_name().and_then(|name| name.to_str()).unwrap_or("");
    match name {
        "note.md" => "note",
        "extract.md" => "extract",
        "mind.md" => "mind",
        "state.md" => "state",
        "review.md" => "review",
        _ if path.extension().and_then(|ext| ext.to_str()) == Some("canvas") => "canvas",
        _ if is_under_outputs(path) => "report",
        _ => "report",
    }
}

/// `path` が `.sai/outputs/` の下にあるか（成分の並びで見る）。
fn is_under_outputs(path: &Path) -> bool {
    let names: Vec<&str> = path
        .components()
        .filter_map(|component| match component {
            Component::Normal(name) => name.to_str(),
            _ => None,
        })
        .collect();
    names.windows(2).any(|pair| pair == [".sai", "outputs"])
}

/// 記録用のパスへ変換する（共有契約）。
///
/// - `.sai/paper/` の下 → `data_dir` からの相対（例 `papers/0030_Foo/extract.md`）
/// - プロジェクトルートの下 → ルートからの相対（例 `.sai/outputs/R9/41/answer.md`）
/// - どちらでもない → 絶対パスのまま
///
/// 相対パスの入力は、まず `root` からの相対として実在を試し、無ければ
/// 現在の作業ディレクトリからの相対として解決する。
#[must_use]
pub fn to_activity_path(root: &Path, data_dir: &Path, path: &Path) -> String {
    let absolute = absolutize(root, path);

    // 1) `.sai/paper/` の下は data_dir からの相対（`to_stored_path` が同じ規則）。
    let stored = paths::to_stored_path(data_dir, &absolute);
    if Path::new(&stored).is_relative() {
        return stored;
    }

    // 2) プロジェクトルートの下はルートからの相対。
    if let Ok(relative) = absolute.strip_prefix(root)
        && !relative.as_os_str().is_empty()
    {
        return components_to_string(relative);
    }
    // 符号リンク差（macOS の `/tmp` ↔ `/private/tmp` など）を吸収してから比べる。
    if let (Ok(canonical_root), Ok(canonical_absolute)) = (
        paths::canonicalize_path(root),
        paths::canonicalize_path(&absolute),
    ) && let Ok(relative) = canonical_absolute.strip_prefix(&canonical_root)
        && !relative.as_os_str().is_empty()
    {
        return components_to_string(relative);
    }

    // 3) どちらでもない → 絶対パス（`to_stored_path` は入力の絶対パスを返す）。
    stored
}

/// 相対パスの入力を絶対パスへそろえる。
fn absolutize(root: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        return path.to_path_buf();
    }
    let from_root = root.join(path);
    if from_root.exists() {
        return from_root;
    }
    std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf())
}

/// 相対パスを区切り `/` の文字列にする（DB に書く形を OS に依らせない）。
fn components_to_string(relative: &Path) -> String {
    let mut stored = String::new();
    for component in relative.components() {
        if !component.as_os_str().is_empty() {
            if !stored.is_empty() {
                stored.push('/');
            }
            stored.push_str(&component.as_os_str().to_string_lossy());
        }
    }
    stored
}

/// パスが論文フォルダ（`papers/<番号4桁>_...`）の下なら `papers.id` を返す。
#[must_use]
pub fn paper_id_for_path(path: &Path) -> Option<i64> {
    let names: Vec<&str> = path
        .components()
        .filter_map(|component| match component {
            Component::Normal(name) => name.to_str(),
            _ => None,
        })
        .collect();
    names.windows(2).find_map(|pair| {
        if pair[0] == "papers" {
            parse_paper_folder(pair[1])
        } else {
            None
        }
    })
}

/// `0030_Foo` のような論文フォルダ名から番号を読む。
fn parse_paper_folder(name: &str) -> Option<i64> {
    let (number, _) = name.split_once('_')?;
    if number.len() == 4 && number.chars().all(|ch| ch.is_ascii_digit()) {
        number.parse().ok()
    } else {
        None
    }
}

/// `detail` の JSON から、重複判定に使うキーを取り出す（共有契約）。
///
/// `path` があればそれ、無ければ `target` と `paper_id` の組。
/// JSON として読めないときは、その文字列をそのままキーにする。
#[must_use]
pub fn target_key(detail_json: &str, paper_id: Option<i64>) -> String {
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(detail_json) {
        if let Some(path) = value
            .get("path")
            .and_then(serde_json::Value::as_str)
            .filter(|path| !path.is_empty())
        {
            return path.to_string();
        }
        if let Some(target) = value.get("target").and_then(serde_json::Value::as_str) {
            return match paper_id {
                Some(id) => format!("{target}#{id}"),
                None => target.to_string(),
            };
        }
    }
    detail_json.to_string()
}

/// ユーザーが開いた記録を残す。
///
/// `stored_path` は [`to_activity_path`] で作った記録用のパス。
///
/// # Errors
///
/// データベースの操作に失敗したときに返す。
pub fn record_open(
    db: &Database,
    target: &str,
    surface: &str,
    paper_id: Option<i64>,
    stored_path: &str,
) -> Result<bool, DatabaseError> {
    let detail = json!({
        "target": target,
        "path": stored_path,
        "surface": surface,
    })
    .to_string();
    let key = target_key(&detail, paper_id);
    db.record_activity_dedup("open", paper_id, &detail, &key, DEDUP_WINDOW_MINUTES)
}

/// エージェントが読んだ記録を残す。
///
/// `stored_path` は分かるときだけ渡す（[`to_activity_path`] で作った記録用のパス）。
///
/// # Errors
///
/// データベースの操作に失敗したときに返す。
pub fn record_agent_read(
    db: &Database,
    target: &str,
    via: &str,
    paper_id: Option<i64>,
    stored_path: Option<&str>,
) -> Result<bool, DatabaseError> {
    let mut detail = json!({
        "target": target,
        "via": via,
    });
    if let Some(path) = stored_path {
        detail["path"] = json!(path);
    }
    let detail = detail.to_string();
    let key = target_key(&detail, paper_id);
    db.record_activity_dedup("agent_read", paper_id, &detail, &key, DEDUP_WINDOW_MINUTES)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use chrono::Utc;

    use super::{open_target_for_path, paper_id_for_path, to_activity_path};

    fn temp_root(tag: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "papr-activity-{tag}-{}-{}",
            std::process::id(),
            Utc::now().timestamp_micros()
        ))
    }

    #[test]
    fn open_target_is_decided_by_file_name_and_outputs() {
        let root = std::path::Path::new("/tmp/project");
        assert_eq!(
            open_target_for_path(root, std::path::Path::new("a/b/note.md")),
            "note"
        );
        assert_eq!(
            open_target_for_path(root, std::path::Path::new("a/b/extract.md")),
            "extract"
        );
        assert_eq!(
            open_target_for_path(root, std::path::Path::new("a/mind.md")),
            "mind"
        );
        assert_eq!(
            open_target_for_path(root, std::path::Path::new("a/state.md")),
            "state"
        );
        assert_eq!(
            open_target_for_path(root, std::path::Path::new("a/review.md")),
            "review"
        );
        assert_eq!(
            open_target_for_path(root, std::path::Path::new("a/board.canvas")),
            "canvas"
        );
        assert_eq!(
            open_target_for_path(
                root,
                std::path::Path::new("/tmp/project/.sai/outputs/R9/41/answer.md")
            ),
            "report"
        );
        // 出力フォルダの外の、名前で決まらない md も report として扱う。
        assert_eq!(
            open_target_for_path(root, std::path::Path::new("misc/other.md")),
            "report"
        );
    }

    #[test]
    fn activity_path_is_relative_under_data_dir_root_or_absolute() {
        let root = temp_root("path");
        let data_dir = root.join(".sai").join("paper");
        fs::create_dir_all(data_dir.join("papers").join("0001_Foo")).unwrap();
        fs::create_dir_all(root.join(".sai").join("outputs").join("R9").join("41")).unwrap();

        // `.sai/paper/` の下 → data_dir からの相対。
        assert_eq!(
            to_activity_path(
                &root,
                &data_dir,
                &data_dir.join("papers").join("0001_Foo").join("note.md")
            ),
            "papers/0001_Foo/note.md"
        );
        // ルートの下（data_dir の外） → ルートからの相対。
        assert_eq!(
            to_activity_path(
                &root,
                &data_dir,
                &root.join(".sai").join("outputs").join("R9").join("41").join("answer.md")
            ),
            ".sai/outputs/R9/41/answer.md"
        );
        // ルートの外 → 絶対パスのまま。
        let outside = temp_root("outside").join("other.md");
        assert_eq!(
            to_activity_path(&root, &data_dir, &outside),
            outside.to_string_lossy()
        );

        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn paper_id_is_read_from_paper_folder() {
        assert_eq!(
            paper_id_for_path(std::path::Path::new("papers/0030_Foo/extract.md")),
            Some(30)
        );
        assert_eq!(
            paper_id_for_path(std::path::Path::new(
                "/tmp/project/.sai/paper/papers/0007_Bar/paper.pdf"
            )),
            Some(7)
        );
        assert_eq!(
            paper_id_for_path(std::path::Path::new("papers/not_numbered/note.md")),
            None
        );
        assert_eq!(
            paper_id_for_path(std::path::Path::new(".sai/paper/state.md")),
            None
        );
    }
}
