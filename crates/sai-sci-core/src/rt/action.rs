//! sci の run の一覧（Recent Activity）と、ブロック定義の `sci_type`。

use std::fs;
use std::path::PathBuf;
use std::time::SystemTime;

use chrono::{DateTime, Local, NaiveDateTime, TimeZone, Utc};
use rusqlite::Connection;
use serde::Serialize;

use super::{Result, RtKeys, fmt_id, task_keys};
use crate::runs::{list_run_dirs, read_frontmatter, run_block_id};
use sai_paper_core::paths::PaperRoots;

/// ブロック定義の置き場の候補（`<root>/.sai/blocks/<id>.md` → `<sai_home>/blocks/<id>.md`）。id が `^[a-z0-9-]+$` でなければ空。
#[must_use]
pub fn block_def_paths(roots: &PaperRoots, block: &str) -> Vec<PathBuf> {
    let valid = !block.is_empty()
        && block
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
    if !valid {
        return Vec::new();
    }
    let mut candidates = vec![roots.root.join(".sai").join("blocks").join(format!("{block}.md"))];
    if let Some(dir) = sai_paper_core::paths::sai_home_path("blocks") {
        candidates.push(dir.join(format!("{block}.md")));
    }
    candidates
}

/// 最初に見つかったブロック定義の frontmatter。
#[must_use]
pub fn block_frontmatter(roots: &PaperRoots, block: &str) -> Option<Vec<(String, String)>> {
    block_def_paths(roots, block)
        .into_iter()
        .find_map(|path| fs::read_to_string(path).ok())
        .map(|text| read_frontmatter(&text))
}

/// `sci_type` に書ける値（LabAgent の 3 分類）。
pub const SCI_TYPES: [&str; 3] = ["node", "manage", "support"];

/// ブロック定義の `sci_type`。無い・空なら `None`（知らない値もそのまま返す。検査は block_warnings）。
#[must_use]
pub fn block_sci_type(roots: &PaperRoots, block: &str) -> Option<String> {
    block_frontmatter(roots, block)?
        .into_iter()
        .find(|(k, _)| k == "sci_type")
        .map(|(_, v)| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

/// ブロック定義の書き間違い（知らない `sci_type`）を 1 行ずつ返す。
#[must_use]
pub fn block_warnings(roots: &PaperRoots) -> Vec<String> {
    let mut dirs = vec![roots.root.join(".sai").join("blocks")];
    dirs.extend(sai_paper_core::paths::sai_home_path("blocks"));
    let mut out = Vec::new();
    for dir in dirs {
        let Ok(entries) = fs::read_dir(dir) else { continue };
        let mut files: Vec<PathBuf> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == "md"))
            .collect();
        files.sort();
        for path in files {
            let Ok(text) = fs::read_to_string(&path) else { continue };
            let name = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
            for (key, value) in read_frontmatter(&text) {
                if key == "sci_type" && !SCI_TYPES.contains(&value.trim()) {
                    out.push(format!("{name}: sci_type に知らない値「{}」があります", value.trim()));
                }
            }
        }
    }
    out
}

/// task.md に sci キーがある run。
#[derive(Debug, Clone)]
pub struct SciRun {
    pub run_id: i64,
    pub owner: String,
    pub block: String,
    pub runner: String,
    pub keys: RtKeys,
    /// RFC3339。
    pub occurred_at: String,
    /// task.md の `global_round`。空や数でなければ `None`。
    pub global_round: Option<i64>,
    /// task.md の `round`。空や数でなければ `None`。
    pub round: Option<i64>,
    pub path: PathBuf,
}

fn parse_local(text: &str) -> Option<DateTime<Utc>> {
    let naive = NaiveDateTime::parse_from_str(text.trim(), "%Y/%m/%d %H:%M:%S").ok()?;
    Some(Local.from_local_datetime(&naive).earliest()?.with_timezone(&Utc))
}

/// sci の run（新しい順）。
#[must_use]
pub fn sci_runs(roots: &PaperRoots) -> Vec<SciRun> {
    let mut out = Vec::new();
    for run in list_run_dirs(&roots.root) {
        let task_path = run.path.join("task.md");
        let Ok(text) = fs::read_to_string(&task_path) else { continue };
        let keys = task_keys(&text);
        if !keys.is_sci() {
            continue;
        }
        let pairs = read_frontmatter(&text);
        let get = |name: &str| pairs.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str());
        let when = get("execution_date")
            .and_then(parse_local)
            .or_else(|| get("creation_date").and_then(parse_local))
            .or_else(|| {
                let modified = fs::metadata(&task_path).and_then(|m| m.modified()).unwrap_or(SystemTime::UNIX_EPOCH);
                Some(DateTime::<Utc>::from(modified))
            })
            .unwrap_or_default();
        out.push(SciRun {
            run_id: run.run_id,
            owner: run.owner.clone(),
            block: run_block_id(&roots.root, &run).unwrap_or_default(),
            runner: get("runner").filter(|v| !v.is_empty()).unwrap_or("pi").to_string(),
            keys,
            occurred_at: when.to_rfc3339(),
            global_round: get("global_round").and_then(|v| v.trim().parse::<i64>().ok()),
            round: get("round").and_then(|v| v.trim().parse::<i64>().ok()),
            path: run.path,
        });
    }
    out
}

/// Recent Activity に出す Action の行。
#[derive(Debug, Clone, Serialize)]
pub struct ActionItem {
    pub kind: String,
    pub label: String,
    pub occurred_at: String,
    pub paper_id: Option<i64>,
    pub radio_id: Option<String>,
    pub run_id: i64,
    pub node: Option<String>,
    pub block: String,
    pub owner: String,
}

/// `todo` を除く sci の run すべての行。
pub fn activity(_conn: &Connection, roots: &PaperRoots) -> Result<Vec<ActionItem>> {
    Ok(sci_runs(roots)
        .into_iter()
        .filter(|r| r.owner != "todo")
        .map(|r| {
            let node = r.keys.node.map(|(k, i)| fmt_id(k, i));
            let label = match &node {
                Some(n) => format!("{}（{n}）: {}", r.block, r.keys.purpose),
                None => format!("{}: {}", r.block, r.keys.purpose),
            };
            ActionItem {
                kind: "action".into(),
                label,
                occurred_at: r.occurred_at,
                paper_id: None,
                radio_id: None,
                run_id: r.run_id,
                node,
                block: r.block,
                owner: r.owner,
            }
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sai_paper_core::database::Database;

    struct Env {
        dir: PathBuf,
        roots: PaperRoots,
        db: Database,
    }

    fn env(tag: &str) -> Env {
        let dir = std::env::temp_dir().join(format!("sai-rt-action-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let data_dir = dir.join(".sai/paper");
        fs::create_dir_all(&data_dir).unwrap();
        fs::create_dir_all(dir.join(".sai/blocks")).unwrap();
        let db = Database::open(&data_dir.join("papr.db")).unwrap();
        Env { roots: PaperRoots { root: dir.clone(), data_dir }, dir, db }
    }

    fn block(e: &Env, id: &str, fm: &str) {
        fs::write(e.dir.join(format!(".sai/blocks/{id}.md")), format!("---\nname: {id}\n{fm}---\n本文\n")).unwrap();
    }

    fn run(e: &Env, owner: &str, id: i64, block: &str, extra: &str) -> PathBuf {
        let p = e.dir.join(format!(".sai/outputs/{owner}/{id}"));
        fs::create_dir_all(&p).unwrap();
        fs::write(
            p.join("task.md"),
            format!("---\nrunner: pi\nblock: {block}\nexecution_date: 2026/10/02 09:16:54\nsci_node: Q1\nsci_purpose: 目的\n{extra}---\n本文\n"),
        )
        .unwrap();
        p
    }

    #[test]
    fn sci_type_read_and_warned() {
        let e = env("type");
        block(&e, "x", "sci_type: foo\n");
        block(&e, "ok", "sci_type: node\n");
        assert_eq!(block_sci_type(&e.roots, "ok").as_deref(), Some("node"));
        assert_eq!(block_sci_type(&e.roots, "missing"), None);
        let w = block_warnings(&e.roots);
        assert_eq!(w, vec!["x: sci_type に知らない値「foo」があります".to_string()]);
    }

    #[test]
    fn activity_lists_sci_runs_without_todo() {
        let e = env("act");
        block(&e, "b", "sci_type: manage\n");
        run(&e, "done", 10, "b", "");
        run(&e, "running", 11, "b", "");
        run(&e, "todo", 12, "b", "");
        crate::link::forget(&e.dir);
        let a = activity(e.db.connection(), &e.roots).unwrap();
        assert!(a.iter().any(|a| a.run_id == 10 && a.label == "b（Q1）: 目的"));
        assert!(a.iter().any(|a| a.run_id == 11));
        assert!(a.iter().all(|a| a.run_id != 12));
    }

    #[test]
    fn block_id_must_be_lowercase_before_it_becomes_a_file_name() {
        // `^[a-z0-9-]+$` に通る ID だけを読む。外へ出る値をそのままファイル名にしない。
        let e = env("bid");
        for good in ["sci-t", "a", "0", "a-b-9"] {
            assert!(!block_def_paths(&e.roots, good).is_empty(), "{good}");
        }
        for bad in ["", "..", "../sci-t", "a/b", "Sci-T", "sci_t", "sci.t", "a b"] {
            assert!(block_def_paths(&e.roots, bad).is_empty(), "{bad}");
        }
        let _ = fs::remove_dir_all(&e.dir);
    }
}
