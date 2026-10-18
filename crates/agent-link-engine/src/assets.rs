//! `agent-link init` がプロジェクトへ配る基盤のエージェント部品（block と template）。
//!
//! ここに置くのは、どのプロジェクトでも使う block 10 本と、それらが `templates:` で読む
//! template 12 本である。中身は実行ファイルに `include_str!` で埋め込み、`init` が
//! `<root>/.agent-link/blocks/`・`<root>/.agent-link/template/` へ配る。
//!
//! 配り方は共通の取り決めに従う（`init.rs` の [`install_assets`] を参照）。
//!
//! - block は `model:` を `inherit` にする（この PC 専用の値を配らない）。
//! - 更新（`--update`）のときは、利用者がブロックごとに選んだ `model`・`thinking`
//!   を元のファイルの値で残す。消してはいけない。

use std::path::{Path, PathBuf};

use crate::init::{backup_path, InitError, InitReport};

/// `.agent-link/blocks/` に配る基盤の block 10 本（ファイル名, 中身）。
pub const BASE_BLOCKS: &[(&str, &str)] = &[
    ("bugfix.md", include_str!("../assets/blocks/bugfix.md")),
    ("explore.md", include_str!("../assets/blocks/explore.md")),
    ("kantoku.md", include_str!("../assets/blocks/kantoku.md")),
    ("main.md", include_str!("../assets/blocks/main.md")),
    ("plan-kantoku.md", include_str!("../assets/blocks/plan-kantoku.md")),
    ("plan.md", include_str!("../assets/blocks/plan.md")),
    ("review.md", include_str!("../assets/blocks/review.md")),
    ("sub.md", include_str!("../assets/blocks/sub.md")),
    ("web-research.md", include_str!("../assets/blocks/web-research.md")),
    ("worker.md", include_str!("../assets/blocks/worker.md")),
];

/// `.agent-link/template/` に配る基盤の template 12 本（ファイル名, 中身）。
pub const BASE_TEMPLATES: &[(&str, &str)] = &[
    ("link-base.md", include_str!("../assets/template/link-base.md")),
    ("link-user.md", include_str!("../assets/template/link-user.md")),
    ("link-task.md", include_str!("../assets/template/link-task.md")),
    ("link-cli.md", include_str!("../assets/template/link-cli.md")),
    ("link-report.md", include_str!("../assets/template/link-report.md")),
    ("doc-report.md", include_str!("../assets/template/doc-report.md")),
    ("doc-unit.md", include_str!("../assets/template/doc-unit.md")),
    ("doc-guide.md", include_str!("../assets/template/doc-guide.md")),
    ("doc-completion.md", include_str!("../assets/template/doc-completion.md")),
    ("mcp-serena.md", include_str!("../assets/template/mcp-serena.md")),
    ("mcp-chrome-devtools.md", include_str!("../assets/template/mcp-chrome-devtools.md")),
    ("mcp-donsetch.md", include_str!("../assets/template/mcp-donsetch.md")),
];

/// block を更新するとき、元のファイルにあった値で残す frontmatter の項目。
///
/// 利用者がブロックごとに選んだモデル・思考の深さを、更新で消さないため。
const PRESERVED_BLOCK_KEYS: &[&str] = &["model", "thinking"];

/// 1 つの部品を配った結果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AssetOutcome {
    /// 無かったので書いた。
    Created,
    /// `--update` で新しい版に置き換えた（退避先のパス）。
    Updated { backup: PathBuf },
    /// 中身が新しい版と同じなので、何もしなかった。
    Unchanged,
    /// `--update` なしで、既にある物をそのままにした。
    Kept,
}

/// 基盤の block と template を `sai`（`<root>/.agent-link`）へ配り、結果を `report` に足す。
///
/// `update` が false のときは無い物だけ書き、既にある物は中身によらず触らない。
/// true のときは、中身が新しい版と違う既存ファイルを退避して新しい版に置き換える。
/// block は [`PRESERVED_BLOCK_KEYS`] の項目を元の値で残してから比べる（置き換えた後の
/// 中身が元と同じなら「そのまま」）。
pub(crate) fn install_assets(
    sai: &Path,
    update: bool,
    report: &mut InitReport,
) -> Result<(), InitError> {
    for (dir, assets, preserve_frontmatter) in [
        ("blocks", BASE_BLOCKS, true),
        ("template", BASE_TEMPLATES, false),
    ] {
        let dir_path = sai.join(dir);
        for (name, new_content) in assets {
            let path = dir_path.join(name);
            let outcome = install_one(&path, new_content, update, preserve_frontmatter)?;
            report.assets.push((path, outcome));
        }
    }
    Ok(())
}

/// 1 つの部品を配る（上の [`install_assets`] の取り決めを 1 ファイルに適用する）。
pub(crate) fn install_one(
    path: &Path,
    new_content: &str,
    update: bool,
    preserve_frontmatter: bool,
) -> Result<AssetOutcome, InitError> {
    if !path.exists() {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, new_content)?;
        return Ok(AssetOutcome::Created);
    }

    let old = std::fs::read_to_string(path)?;
    let desired = if preserve_frontmatter {
        merge_block_content(new_content, &old)
    } else {
        new_content.to_string()
    };

    // 比べるのは置き換えた後の新しい版。元と同じなら「そのまま」。
    if desired == old {
        return Ok(AssetOutcome::Unchanged);
    }
    if !update {
        return Ok(AssetOutcome::Kept);
    }

    let backup = backup_path(path);
    std::fs::rename(path, &backup)?;
    std::fs::write(path, &desired)?;
    Ok(AssetOutcome::Updated { backup })
}

/// `path` の今の中身が、`new_content` を配ったときの中身と同じなら true（無ければ false）。
///
/// [`install_one`] の「そのまま」と同じ比べ方をする（拡張を無効にするとき、利用者が直していない物を選ぶ）。
pub(crate) fn same_as_installed(path: &Path, new_content: &str, preserve_frontmatter: bool) -> bool {
    let Ok(old) = std::fs::read_to_string(path) else {
        return false;
    };
    let desired = if preserve_frontmatter {
        merge_block_content(new_content, &old)
    } else {
        new_content.to_string()
    };
    desired == old
}

/// 新しい block の frontmatter のうち、[`PRESERVED_BLOCK_KEYS`] の項目を
/// 元のファイルにあった値で置き換えた中身を返す。
///
/// 新しい版にその行が無ければ frontmatter の最後に足す。元のファイルにその行が無ければ
/// 新しい版のままにする。frontmatter の区切りや本文（body）は新しい版のものをそのまま使う。
fn merge_block_content(new_content: &str, old_content: &str) -> String {
    let Some(old_lines) = frontmatter_lines(old_content) else {
        return new_content.to_string();
    };
    let Some((start, end)) = frontmatter_middle_range(new_content) else {
        return new_content.to_string();
    };

    let middle = &new_content[start..end];
    let trailing_newline = middle.ends_with('\n');
    let base = middle.trim_end_matches('\n');
    let mut fm_lines: Vec<String> = if base.is_empty() {
        Vec::new()
    } else {
        base.split('\n').map(str::to_string).collect()
    };

    for &key in PRESERVED_BLOCK_KEYS {
        let Some(old_line) = old_lines.iter().find(|l| frontmatter_key(l) == Some(key)) else {
            continue;
        };
        match fm_lines.iter().position(|l| frontmatter_key(l) == Some(key)) {
            Some(i) => fm_lines[i] = (*old_line).to_string(),
            None => fm_lines.push((*old_line).to_string()),
        }
    }

    let mut new_middle = fm_lines.join("\n");
    if trailing_newline && !new_middle.is_empty() {
        new_middle.push('\n');
    }

    let mut out = String::with_capacity(new_content.len());
    out.push_str(&new_content[..start]);
    out.push_str(&new_middle);
    out.push_str(&new_content[end..]);
    out
}

/// frontmatter の行の一覧（区切りの `---` の外側は含めない）。
fn frontmatter_lines(content: &str) -> Option<Vec<&str>> {
    let (start, end) = frontmatter_middle_range(content)?;
    Some(content[start..end].lines().collect())
}

/// frontmatter の中身（開き `---` の次の行頭から、閉じ `---` の行頭まで）の
/// バイト範囲を返す。frontmatter でなければ `None`。
fn frontmatter_middle_range(content: &str) -> Option<(usize, usize)> {
    let mut lines = content.split_inclusive('\n');
    let first = lines.next()?;
    if first.trim_end_matches(['\r', '\n']) != "---" {
        return None;
    }
    let start = first.len();
    let mut offset = start;
    for line in lines {
        if line.trim_end_matches(['\r', '\n']) == "---" {
            return Some((start, offset));
        }
        offset += line.len();
    }
    None
}

/// frontmatter の行から項目名を取り出す（`model: x` → `model`）。
///
/// インデントされた行（`subblocks` の項目など）と空行・コメント行は項目とみなさない。
fn frontmatter_key(line: &str) -> Option<&str> {
    if line.starts_with(' ') || line.starts_with('\t') || line.starts_with('#') {
        return None;
    }
    let (key, _) = line.split_once(':')?;
    let key = key.trim();
    if key.is_empty() { None } else { Some(key) }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 保持する 2 項目だけが元の値になり、他は新しい版のままであること。
    #[test]
    fn test_merge_block_content_keeps_user_frontmatter() {
        let new_content = "---\nid: kantoku\nmodel: inherit\nthinking: low\n---\n\n新しい本文\n";
        let old_content = "---\nid: kantoku\nmodel: sonnet\nthinking: max\n---\n\n古い本文\n";
        let merged = merge_block_content(new_content, old_content);
        assert!(merged.contains("model: sonnet\n"), "{}", merged);
        assert!(merged.contains("thinking: max\n"), "{}", merged);
        assert!(!merged.contains("model: inherit"), "{}", merged);
        assert!(!merged.contains("thinking: low"), "{}", merged);
        assert!(merged.ends_with("\n新しい本文\n"), "{}", merged);
    }

    /// 元のファイルに無い項目は新しい版のままにし、新しい版に無い項目は足すこと。
    #[test]
    fn test_merge_block_content_adds_missing_key() {
        let new_content = "---\nid: main\nmodel: inherit\n---\n\n本文\n";
        let old_content = "---\nid: main\nmodel: opus\nthinking: max\n---\n\n";
        let merged = merge_block_content(new_content, old_content);
        assert!(merged.contains("model: opus\n"), "{}", merged);
        // 新しい版に無い thinking は frontmatter の最後に足す
        assert!(merged.contains("thinking: max\n"), "{}", merged);
        assert!(merged.ends_with("\n本文\n"), "{}", merged);
    }

    /// frontmatter の外側（本文）だけが違っても、保つ項目を戻せば元と一致すること。
    #[test]
    fn test_merge_block_content_unchanged_when_only_preserved() {
        let new_content = "---\nid: sub\nmodel: inherit\nthinking: high\n---\n\n本文\n";
        let old_content = "---\nid: sub\nmodel: haiku\nthinking: high\n---\n\n本文\n";
        let merged = merge_block_content(new_content, old_content);
        assert_eq!(merged, old_content);
    }
}
