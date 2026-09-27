//! sai-link の実行フォルダ（`<root>/.sai/outputs/<owner>/<run_id>/`）を読む。
//!
//! 実行フォルダは実行が進むと `todo/` → `running/` → `review/` → `done/` と
//! フォルダごと移る。`run_id` は変わらないので、同じ番号が 2 か所にあるときは
//! 更新時刻の新しい方を採用する。
//!
//! ここは「実行とは何か」だけを知る層で、brief（状況まとめ）や出典の自動登録が
//! 共通で使う。ブロック定義の frontmatter と sai-link の DB の読み取りだけを行い、
//! どちらも書き換えない。

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

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

/// `<root>/.sai/outputs/*/*/` のうち、フォルダ名が数字のものを新しい順で返す。
///
/// 同じ `run_id` が 2 か所にあれば、フォルダの更新時刻が新しい方を採用する。
/// `.sai/outputs/` が無い・読めないときは空を返す（エラーにはしない）。
#[must_use]
pub fn list_run_dirs(root: &Path) -> Vec<RunDir> {
    let outputs = root.join(".sai").join("outputs");
    let mut by_id: HashMap<i64, (SystemTime, RunDir)> = HashMap::new();

    let Ok(owners) = fs::read_dir(&outputs) else {
        return Vec::new();
    };
    for owner in owners.flatten() {
        let owner_name = owner.file_name().to_string_lossy().to_string();
        let Ok(runs) = fs::read_dir(owner.path()) else {
            continue;
        };
        for run in runs.flatten() {
            let name = run.file_name().to_string_lossy().to_string();
            let Ok(run_id) = name.parse::<i64>() else {
                continue;
            };
            let path = run.path();
            if !path.is_dir() {
                continue;
            }
            let modified = modified_at(&path);
            let entry = RunDir {
                run_id,
                owner: owner_name.clone(),
                path,
            };
            match by_id.get(&run_id) {
                Some((known, _)) if *known >= modified => {}
                _ => {
                    by_id.insert(run_id, (modified, entry));
                }
            }
        }
    }

    let mut runs: Vec<RunDir> = by_id.into_values().map(|(_, run)| run).collect();
    // 新しい実行が先。同じフォルダ内で決定的にするため run_id の降順。
    runs.sort_by(|a, b| b.run_id.cmp(&a.run_id));
    runs
}

/// その実行のブロック ID を返す。
///
/// ① 実行フォルダの `task.md` の frontmatter の `block:`
/// ② 無ければ sai-link の DB（`<root>/.sai/sai-engine/sai-engine.db`）を
///    **読み取り専用** で開き `execution_history` から引く
/// ③ どちらも無ければ `None`
///
/// 子タスクの実行フォルダには `task.md` が無いことがあるため ② が要る。
/// DB が無い・開けない・表が無いときも `None`（エラーにしない）。
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
    block_id_from_db(root, run.run_id)
}

/// その実行のタグ（仮説の名前など）を返す。
///
/// ① 実行フォルダの `task.md` の frontmatter の `tag:`
/// ② 無ければ sai-link の DB（`<root>/.sai/sai-engine/sai-engine.db`）を
///    **読み取り専用** で開き `execution_history.tag` から引く
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
    tag_from_db(root, run.run_id)
}

/// sai-link の DB から実行のブロック ID を引く（[`run_block_id`] の ②）。
fn block_id_from_db(root: &Path, run_id: i64) -> Option<String> {
    let db_path = root.join(".sai").join("sai-engine").join("sai-engine.db");
    if !db_path.exists() {
        return None;
    }
    let conn =
        rusqlite::Connection::open_with_flags(&db_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .ok()?;
    conn.query_row(
        "SELECT b.name FROM execution_history e \
         JOIN block_metadata b ON b.id = e.block_id WHERE e.id = ?1",
        [run_id],
        |row| row.get::<_, String>(0),
    )
    .ok()
}

/// sai-link の DB から実行のタグを引く（[`run_tag`] の ②）。
///
/// `tag` の列が無い・値が NULL・空のときは `None`。
fn tag_from_db(root: &Path, run_id: i64) -> Option<String> {
    let db_path = root.join(".sai").join("sai-engine").join("sai-engine.db");
    if !db_path.exists() {
        return None;
    }
    let conn =
        rusqlite::Connection::open_with_flags(&db_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .ok()?;
    conn.query_row("SELECT tag FROM execution_history WHERE id = ?1", [run_id], |row| {
        row.get::<_, Option<String>>(0)
    })
    .ok()
    .flatten()
    .filter(|value| !value.is_empty())
}

/// 研究用のブロックかどうかを返す。
///
/// ブロック定義（`<root>/.sai/blocks/<id>.md`、無ければ `~/.sai/blocks/<id>.md`）の
/// frontmatter の `templates:` に `project-brief` を含めば `true`。定義が無ければ `false`。
///
/// 同じブロックについて何度も呼ぶときは、呼び出し側で結果を `HashMap` に覚えてよい
/// （ファイルを毎回読むため）。
#[must_use]
pub fn is_research_block(root: &Path, block_id: &str) -> bool {
    let mut candidates = vec![
        root.join(".sai")
            .join("blocks")
            .join(format!("{block_id}.md")),
    ];
    if let Some(home) = std::env::var_os("HOME") {
        candidates.push(
            PathBuf::from(home)
                .join(".sai")
                .join("blocks")
                .join(format!("{block_id}.md")),
        );
    }

    for path in candidates {
        let Ok(text) = fs::read_to_string(&path) else {
            continue;
        };
        return read_frontmatter(&text)
            .into_iter()
            .filter(|(key, _)| key == "templates")
            .any(|(_, value)| value.split(',').any(|part| part.trim() == "project-brief"));
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

/// フォルダの更新時刻（取れなければ UNIX エポック）。
fn modified_at(path: &Path) -> SystemTime {
    fs::metadata(path)
        .and_then(|meta| meta.modified())
        .unwrap_or(SystemTime::UNIX_EPOCH)
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
    fn list_run_dirs_keeps_newest_of_same_run() {
        let root = temp_root("run-dirs");
        write(
            root.join(".sai/outputs/todo/7/task.md"),
            "---\nblock: main\n---\n",
        );
        std::thread::sleep(std::time::Duration::from_millis(20));
        write(root.join(".sai/outputs/running/7/answer.md"), "本体\n");
        write(root.join(".sai/outputs/review/8/task.md"), "---\n---\n");
        write(root.join(".sai/outputs/not-a-run/x/task.md"), "---\n---\n");

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
        // DB には別の名前を入れておき、task.md が優先されることを見る
        make_link_db(&root, &[(11, "main")]);

        let run = RunDir {
            run_id: 11,
            owner: "review".to_string(),
            path: dir,
        };
        assert_eq!(run_block_id(&root, &run).as_deref(), Some("paper-search"));
    }

    #[test]
    fn run_block_id_falls_back_to_db() {
        let root = temp_root("block-db");
        let dir = root.join(".sai/outputs/R9/12");
        fs::create_dir_all(&dir).unwrap();
        make_link_db(&root, &[(12, "paper-library-qa")]);

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
    fn run_block_id_without_db_is_none() {
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
        // DB には別のタグを入れておき、task.md が優先されることを見る
        make_link_db(&root, &[(21, "paper-search")]);
        set_link_db_tag(&root, 21, "仮説Z");

        let run = RunDir {
            run_id: 21,
            owner: "review".to_string(),
            path: dir,
        };
        assert_eq!(run_tag(&root, &run).as_deref(), Some("仮説B"));
    }

    #[test]
    fn run_tag_falls_back_to_db() {
        let root = temp_root("tag-db");
        let dir = root.join(".sai/outputs/R9/22");
        fs::create_dir_all(&dir).unwrap();
        make_link_db(&root, &[(22, "paper-library-qa")]);
        set_link_db_tag(&root, 22, "仮説C");

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

        // task.md も DB も無い
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

    /// sai-link の DB の最小の表（`execution_history` と `block_metadata`）を作る。
    fn make_link_db(root: &Path, runs: &[(i64, &str)]) {
        let path = root.join(".sai/sai-engine/sai-engine.db");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute_batch(
            "CREATE TABLE block_metadata(id INTEGER PRIMARY KEY, name TEXT NOT NULL);
             CREATE TABLE execution_history(id INTEGER PRIMARY KEY, block_id INTEGER NOT NULL, tag TEXT);",
        )
        .unwrap();
        for (run_id, name) in runs {
            conn.execute(
                "INSERT INTO block_metadata (id, name) VALUES (?1, ?2)",
                rusqlite::params![run_id, name],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO execution_history (id, block_id) VALUES (?1, ?1)",
                rusqlite::params![run_id],
            )
            .unwrap();
        }
    }

    /// テスト用の DB の実行にタグを入れる（[`make_link_db`] の後で呼ぶ）。
    fn set_link_db_tag(root: &Path, run_id: i64, tag: &str) {
        let conn = rusqlite::Connection::open(root.join(".sai/sai-engine/sai-engine.db")).unwrap();
        conn.execute(
            "UPDATE execution_history SET tag = ?2 WHERE id = ?1",
            rusqlite::params![run_id, tag],
        )
        .unwrap();
    }
}
