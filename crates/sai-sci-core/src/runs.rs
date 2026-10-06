//! sai-link の実行フォルダ（`run list --json` の `dir`）を読む。
//!
//! 実行フォルダの一覧は sai-link の CLI（`run list --json --limit 0`）から取り、
//! ここではその `dir` だけを使う。`run_id` は変わらないので、同じ番号が 2 か所に
//! あるときは新しい方を採用する。
//!
//! ここは「実行とは何か」だけを知る層で、brief（状況まとめ）や出典の自動登録が
//! 共通で使う。ブロック定義の frontmatter は読むが、sai-link の DB と実行フォルダの
//! 並びは直に開かない。

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use crate::link;

/// 1 つの実行フォルダ。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunDir {
    /// sai-link の実行 ID（フォルダ名の数字）。
    pub run_id: i64,
    /// 実行フォルダの親の名前（`todo` / `running` / `review` / `done` / `R0` …）。
    pub owner: String,
    /// 実行フォルダの絶対パス。
    pub path: PathBuf,
}

/// `<root>` の run を新しい順で返す。
///
/// `sai-link run list --json` の `dir` がある物だけを [`RunDir`] にする。
/// 同じ `run_id` が 2 つあれば `run_id` の大きい方を先にする（新しい実行が先）。
///
/// sai-link の一覧を読めないときは空を返し、標準エラーに理由を 1 回だけ出す
/// （今までも `.sai/outputs` が無ければ空を返し、エラーにしなかった）。
#[must_use]
pub fn list_run_dirs(root: &Path) -> Vec<RunDir> {
    match link::runs(root) {
        Ok(runs) => {
            let mut out: Vec<RunDir> = runs
                .into_iter()
                .filter_map(|run| {
                    let path = run.dir?;
                    Some(RunDir {
                        run_id: run.run_id,
                        owner: run.lane,
                        path,
                    })
                })
                .collect();
            out.sort_by(|a, b| b.run_id.cmp(&a.run_id));
            out
        }
        Err(error) => {
            // 1 回だけ出す（status・brief などが何度も呼ぶため）。
            static WARNED: OnceLock<()> = OnceLock::new();
            WARNED.get_or_init(|| {
                eprintln!("sai-link の run の一覧を読めませんでした: {error}");
            });
            Vec::new()
        }
    }
}

/// その実行のブロック ID を返す。
///
/// ① 実行フォルダの `task.md` の frontmatter の `block:`
/// ② 無ければ `sai-link run list --json` の同じ `run_id` の `block`
/// ③ どちらも無ければ `None`
///
/// 子タスクの実行フォルダには `task.md` が無いことがあるため ② が要る。
#[must_use]
pub fn run_block_id(root: &Path, run: &RunDir) -> Option<String> {
    if let Ok(text) = fs::read_to_string(run.path.join("task.md")) {
        let value = read_frontmatter(&text)
            .into_iter()
            .find(|(key, _)| key == "block")
            .map(|(_, value)| value);
        if let Some(value) = value.filter(|value| !value.is_empty()) {
            return Some(value);
        }
    }
    field_from_link(root, run.run_id, "block")
}

/// その実行のタグ（仮説の名前など）を返す。
///
/// ① 実行フォルダの `task.md` の frontmatter の `tag:`
/// ② 無ければ `sai-link run list --json` の同じ `run_id` の `tag`
/// ③ どちらも無い・空なら `None`
///
/// 子タスクの実行フォルダには `task.md` が無いことがあるため ② が要る。
#[must_use]
pub fn run_tag(root: &Path, run: &RunDir) -> Option<String> {
    if let Ok(text) = fs::read_to_string(run.path.join("task.md")) {
        let value = read_frontmatter(&text)
            .into_iter()
            .find(|(key, _)| key == "tag")
            .map(|(_, value)| value);
        if let Some(value) = value.filter(|value| !value.is_empty()) {
            return Some(value);
        }
    }
    field_from_link(root, run.run_id, "tag")
}

/// `run list --json` の同じ `run_id` の項目を返す（空なら `None`）。
fn field_from_link(root: &Path, run_id: i64, field: &str) -> Option<String> {
    let runs = link::runs(root).ok()?;
    let run = runs.into_iter().find(|run| run.run_id == run_id)?;
    let value = match field {
        "block" => run.block,
        "tag" => run.tag,
        _ => String::new(),
    };
    (!value.is_empty()).then_some(value)
}

/// 研究用と見てよい `templates` の値（C-15）。
const RESEARCH_TEMPLATES: [&str; 2] = ["project-brief", "sci-core"];

/// 研究用のブロックかどうかを返す。
///
/// ブロック定義（`<root>/.sai/blocks/<id>.md`、無ければ `~/.sai-moon/blocks/<id>.md`）の
/// frontmatter の `templates:` に `project-brief`（論文の作業）か `sci-core`（研究方針の作業）を
/// 含めば `true`。定義が無ければ `false`。
///
/// sci のブロックは `project-brief` を入れない（要件 R-D3）ので、`sci-core` も研究用と見る。
/// 同じブロックについて何度も呼ぶときは、呼び出し側で結果を `HashMap` に覚えてよい
/// （ファイルを毎回読むため）。
#[must_use]
pub fn is_research_block(root: &Path, block_id: &str) -> bool {
    let mut candidates = vec![
        root.join(".sai")
            .join("blocks")
            .join(format!("{block_id}.md")),
    ];
    if let Some(dir) = sai_paper_core::paths::sai_home_path("blocks") {
        candidates.push(dir.join(format!("{block_id}.md")));
    }

    for path in candidates {
        let Ok(text) = fs::read_to_string(&path) else {
            continue;
        };
        return read_frontmatter(&text)
            .into_iter()
            .filter(|(key, _)| key == "templates")
            .any(|(_, value)| {
                value.split(',').any(|part| {
                    RESEARCH_TEMPLATES.contains(&part.trim())
                })
            });
    }
    false
}

/// md の先頭の frontmatter（最初の `---` から次の `---` まで）を `key: value` の組にする。
///
/// - 先頭の行が `---` でなければ空を返す。
/// - 値は前後の空白・引用符を落とす。`[a, b]` の形も中身を取る。
/// - YAML の配列（`- a` の行）は、直前のキーの値に `,` でつないで 1 つの値にする
///   （`templates:` だけ扱えればよい）。
/// - 入れ子の写像は平坦に読む（`key:` の行は独立した組になる）。この用途では
///   `name` / `block` / `templates` しか見ないため十分。
#[must_use]
pub fn read_frontmatter(text: &str) -> Vec<(String, String)> {
    let mut pairs: Vec<(String, String)> = Vec::new();
    let mut lines = text.lines();
    match lines.next() {
        Some(first) if first.trim() == "---" => {}
        _ => return pairs,
    }

    let mut array_key: Option<usize> = None;
    for line in lines {
        let trimmed = line.trim();
        if trimmed == "---" {
            break;
        }
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if let Some(item) = trimmed.strip_prefix("- ") {
            if let Some(index) = array_key {
                let value = normalize_value(item);
                if !value.is_empty() {
                    if pairs[index].1.is_empty() {
                        pairs[index].1 = value;
                    } else {
                        pairs[index].1.push(',');
                        pairs[index].1.push_str(&value);
                    }
                }
            }
            continue;
        }
        let Some((key, value)) = trimmed.split_once(':') else {
            continue;
        };
        array_key = Some(pairs.len());
        pairs.push((key.trim().to_string(), normalize_value(value)));
    }
    pairs
}

/// frontmatter の 1 つの値から、外側の空白・引用符・角括弧を落とす。
fn normalize_value(raw: &str) -> String {
    let raw = raw.trim();
    let raw = raw
        .strip_prefix('[')
        .and_then(|rest| rest.strip_suffix(']'))
        .unwrap_or(raw);
    raw.trim()
        .trim_matches('"')
        .trim_matches('\'')
        .trim()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// テスト用の一時プロジェクトルート。
    fn temp_root(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "sai-paper-runs-{}-{}-{:?}",
            tag,
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write(path: PathBuf, text: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    #[test]
    fn read_frontmatter_scalar_and_array() {
        let text = "---\nname: main\nid: main\ntemplates:\n  - sai-link\n  - project-brief\ndescription: 実装\n---\n\n# 本文\ntemplates: 偽物\n";
        let pairs = read_frontmatter(text);

        assert_eq!(
            pairs,
            vec![
                ("name".to_string(), "main".to_string()),
                ("id".to_string(), "main".to_string()),
                (
                    "templates".to_string(),
                    "sai-link,project-brief".to_string()
                ),
                ("description".to_string(), "実装".to_string()),
            ]
        );
    }

    #[test]
    fn read_frontmatter_comma_string_and_inline_array() {
        let text = "---\ntemplates: sai-link, sai-docs\nother: [a, b]\n---\n";
        let pairs = read_frontmatter(text);

        assert_eq!(
            pairs[0],
            ("templates".to_string(), "sai-link, sai-docs".to_string())
        );
        assert_eq!(pairs[1], ("other".to_string(), "a, b".to_string()));
    }

    #[test]
    fn read_frontmatter_without_heading_is_empty() {
        assert!(read_frontmatter("# 本文\n---\nblock: main\n---\n").is_empty());
        assert!(read_frontmatter("---\nblock: main\n").len() == 1); // 閉じが無くても読む
    }

    #[test]
    fn list_run_dirs_uses_link_runs_and_sorts() {
        let root = temp_root("run-dirs");
        let dir8 = root.join(".sai/outputs/review/8");
        let dir7 = root.join(".sai/outputs/running/7");
        fs::create_dir_all(&dir8).unwrap();
        fs::create_dir_all(&dir7).unwrap();
        crate::fake_link::write_runs(
            &root,
            &[
                crate::fake_link::run_value(7, "running", &dir7, "main", "", "completed"),
                crate::fake_link::run_value(8, "review", &dir8, "", "", "completed"),
            ],
        );

        let runs = list_run_dirs(&root);

        assert_eq!(runs.len(), 2);
        assert_eq!(runs[0].run_id, 8);
        assert_eq!(runs[1].run_id, 7);
        assert_eq!(runs[1].owner, "running");
        assert!(runs[1].path.ends_with("running/7"));
    }

    #[test]
    fn run_block_id_prefers_task_md() {
        let root = temp_root("block-task");
        let dir = root.join(".sai/outputs/review/11");
        write(
            dir.join("task.md"),
            "---\nrun_id: 11\nblock: paper-search\n---\n",
        );

        let run = RunDir {
            run_id: 11,
            owner: "review".to_string(),
            path: dir,
        };
        assert_eq!(run_block_id(&root, &run).as_deref(), Some("paper-search"));
    }

    #[test]
    fn run_block_id_falls_back_to_link() {
        let root = temp_root("block-link");
        let dir = root.join(".sai/outputs/R9/12");
        fs::create_dir_all(&dir).unwrap();
        crate::fake_link::write_runs(
            &root,
            &[crate::fake_link::run_value(
                12,
                "R9",
                &dir,
                "paper-library-qa",
                "",
                "completed",
            )],
        );

        let run = RunDir {
            run_id: 12,
            owner: "R9".to_string(),
            path: dir,
        };
        assert_eq!(
            run_block_id(&root, &run).as_deref(),
            Some("paper-library-qa")
        );
    }

    #[test]
    fn run_block_id_without_link_is_none() {
        let root = temp_root("block-none");
        let dir = root.join(".sai/outputs/R9/13");
        fs::create_dir_all(&dir).unwrap();

        let run = RunDir {
            run_id: 13,
            owner: "R9".to_string(),
            path: dir,
        };
        assert_eq!(run_block_id(&root, &run), None);
    }

    #[test]
    fn run_tag_prefers_task_md() {
        let root = temp_root("tag-task");
        let dir = root.join(".sai/outputs/review/21");
        write(
            dir.join("task.md"),
            "---\nrun_id: 21\nblock: paper-search\ntag: 仮説B\n---\n",
        );

        let run = RunDir {
            run_id: 21,
            owner: "review".to_string(),
            path: dir,
        };
        assert_eq!(run_tag(&root, &run).as_deref(), Some("仮説B"));
    }

    #[test]
    fn run_tag_falls_back_to_link() {
        let root = temp_root("tag-link");
        let dir = root.join(".sai/outputs/R9/22");
        fs::create_dir_all(&dir).unwrap();
        crate::fake_link::write_runs(
            &root,
            &[crate::fake_link::run_value(
                22,
                "R9",
                &dir,
                "paper-library-qa",
                "仮説C",
                "completed",
            )],
        );

        let run = RunDir {
            run_id: 22,
            owner: "R9".to_string(),
            path: dir,
        };
        assert_eq!(run_tag(&root, &run).as_deref(), Some("仮説C"));
    }

    #[test]
    fn run_tag_empty_or_missing_is_none() {
        let root = temp_root("tag-none");
        // task.md の tag: が空
        let dir = root.join(".sai/outputs/review/23");
        write(
            dir.join("task.md"),
            "---\nrun_id: 23\nblock: paper-search\ntag:\n---\n",
        );
        let run = RunDir {
            run_id: 23,
            owner: "review".to_string(),
            path: dir,
        };
        assert_eq!(run_tag(&root, &run), None);

        // task.md も sai-link の一覧も無い
        let dir = root.join(".sai/outputs/R9/24");
        fs::create_dir_all(&dir).unwrap();
        let run = RunDir {
            run_id: 24,
            owner: "R9".to_string(),
            path: dir,
        };
        assert_eq!(run_tag(&root, &run), None);
    }

    #[test]
    fn is_research_block_reads_templates() {
        let root = temp_root("research");
        write(
            root.join(".sai/blocks/paper-search.md"),
            "---\nname: paper-search\ntemplates:\n  - project-brief\n  - research-core\n---\n\n本文\n",
        );
        write(
            root.join(".sai/blocks/paper-screen.md"),
            "---\nname: paper-screen\ntemplates: project-brief,research-core\n---\n\n本文\n",
        );
        write(
            root.join(".sai/blocks/main.md"),
            "---\nname: main\ntemplates: sai-link,sai-docs\n---\n\n本文\n",
        );

        assert!(is_research_block(&root, "paper-search"));
        assert!(is_research_block(&root, "paper-screen"));
        assert!(!is_research_block(&root, "main"));
        assert!(!is_research_block(&root, "missing"));
    }

    #[test]
    fn is_research_block_accepts_sci_core() {
        let root = temp_root("research-sci");
        // sci のブロックは project-brief を入れない（要件 R-D3）。
        write(
            root.join(".sai/blocks/sci-compare.md"),
            "---\nname: sci-compare\ntemplates:\n  - sci-core\n  - literature-hard-rules\n---\n\n本文\n",
        );
        write(
            root.join(".sai/blocks/sci-search.md"),
            "---\nname: sci-search\ntemplates: sci-core,sai-docs\n---\n\n本文\n",
        );

        assert!(is_research_block(&root, "sci-compare"));
        assert!(is_research_block(&root, "sci-search"));
    }
}
