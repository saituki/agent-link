//! 研究の型とガイドの読み込み、型から Q の木を作る処理（C-DEF）。
//!
//! 元の版はリポジトリの `sai-sci/types/` と `sai-sci/guides/` で、ここに埋め込む。
//! 読む順は 埋め込み → 利用者全体 `<sai_home>/sci/` → プロジェクト `<root>/.sai/sci/`（あとが強い）。
//! 古い書式のファイルと読めないファイルは止めずに飛ばし、[`Defs::warnings`] に 1 行入れる。

use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use rusqlite::Connection;

use super::store::{self, NewNode};
use super::{now_text, Kind, Result, RtError};

/// 型の `## Q` の 1 行。
#[derive(Debug, Clone, PartialEq)]
pub struct TypeQ {
    pub key: String,
    /// `+` の行（O が足されるたびに作る Q）。
    pub per_option: bool,
    pub children: Vec<TypeQ>,
}

/// 研究の型。
#[derive(Debug, Clone, PartialEq)]
pub struct TypeDef {
    pub id: String,
    pub name: String,
    pub root: String,
    pub explain: String,
    pub tree: Vec<TypeQ>,
    pub seed: String,
}

/// Q ごとのガイド。
#[derive(Debug, Clone, PartialEq)]
pub struct GuideDef {
    pub key: String,
    pub name: String,
    pub explain: String,
    pub question: String,
    pub child_candidates: Vec<String>,
    pub extract: Vec<String>,
    pub pitfalls: Vec<String>,
    pub target: i64,
}

/// 読み込んだ型とガイド。
#[derive(Debug, Clone, Default)]
pub struct Defs {
    pub types: Vec<TypeDef>,
    pub guides: Vec<GuideDef>,
    pub warnings: Vec<String>,
}

impl Defs {
    #[must_use]
    pub fn type_def(&self, id: &str) -> Option<&TypeDef> {
        self.types.iter().find(|t| t.id == id)
    }

    #[must_use]
    pub fn guide(&self, key: &str) -> Option<&GuideDef> {
        self.guides.iter().find(|g| g.key == key)
    }

    /// 型 `type_id` の中で、`guide_key` の Q の下にある `+` の子（O ごとに作る Q）。
    #[must_use]
    pub fn per_option_children(&self, type_id: &str, guide_key: &str) -> Vec<&TypeQ> {
        fn find<'a>(qs: &'a [TypeQ], key: &str) -> Option<&'a TypeQ> {
            qs.iter().find_map(|q| if q.key == key { Some(q) } else { find(&q.children, key) })
        }
        self.type_def(type_id)
            .and_then(|t| find(&t.tree, guide_key))
            .map(|q| q.children.iter().filter(|c| c.per_option).collect())
            .unwrap_or_default()
    }

    /// Q の文。ガイドの question が空でなければそれ、無ければ `<name>は？`、ガイドが無ければ key。
    #[must_use]
    pub fn question_text(&self, key: &str) -> String {
        match self.guide(key) {
            Some(g) if !g.question.is_empty() => g.question.clone(),
            Some(g) => format!("{}は？", g.name),
            None => key.to_string(),
        }
    }
}

/// 型の並び。これ以外は id 順で後ろに置く。
pub const TYPE_ORDER: [&str; 5] = ["problem", "phenomenon", "method", "data", "improve"];

/// 埋め込みの一覧を `include_str!` の繰り返しなく書くための補助。
/// 相対パスは `sai-sci/` からの道筋（`types/<id>.md`・`guides/<key>.md`）。
macro_rules! embedded_files {
    ($($relative:literal),* $(,)?) => {
        &[$(($relative, include_str!(concat!("../../../../sai-sci/", $relative)))),*]
    };
}

const EMBEDDED: &[(&str, &str)] = embedded_files![
    // 型（5 本）
    "types/problem.md",
    "types/phenomenon.md",
    "types/method.md",
    "types/data.md",
    "types/improve.md",
    // ガイド（25 本）
    "guides/background.md",
    "guides/background.goal.md",
    "guides/background.problem.md",
    "guides/data.md",
    "guides/data.access.md",
    "guides/data.label.md",
    "guides/data.size.md",
    "guides/data.type.md",
    "guides/existing.md",
    "guides/issue.md",
    "guides/issue.limit.md",
    "guides/issue.root.md",
    "guides/method.md",
    "guides/method.adapt.md",
    "guides/method.analog.md",
    "guides/novelty.md",
    "guides/novelty.closest.md",
    "guides/novelty.diff.md",
    "guides/phenomenon.md",
    "guides/purpose.md",
    "guides/purpose.metric.md",
    "guides/purpose.output.md",
    "guides/purpose.subgoal.md",
    "guides/target.md",
    "guides/target.subgroup.md",
];

/// 埋め込みの一覧（`("types/problem.md", 本文)` など 30 個）。
#[must_use]
pub fn embedded() -> &'static [(&'static str, &'static str)] {
    EMBEDDED
}

// ---- md の読み取り -------------------------------------------------------------------

struct Doc {
    front: Vec<(String, String)>,
    explain: String,
    sections: Vec<(String, Vec<String>)>,
}

impl Doc {
    fn front(&self, name: &str) -> Option<&str> {
        self.front.iter().find(|(k, _)| k == name).map(|(_, v)| v.trim()).filter(|v| !v.is_empty())
    }

    fn section(&self, name: &str) -> Option<&[String]> {
        self.sections.iter().find(|(n, _)| n == name).map(|(_, l)| l.as_slice())
    }

    fn has_section(&self, names: &[&str]) -> bool {
        names.iter().any(|n| self.section(n).is_some())
    }

    fn bullets(&self, name: &str) -> Vec<String> {
        self.section(name)
            .unwrap_or_default()
            .iter()
            .filter_map(|l| l.trim().strip_prefix("- "))
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect()
    }

    /// 節の本文を 1 行につないだもの。
    fn joined(&self, name: &str) -> String {
        let lines = self.section(name).unwrap_or_default();
        lines.iter().map(|l| l.trim()).filter(|l| !l.is_empty()).collect::<Vec<_>>().join("")
    }
}

/// frontmatter・説明・`## ` の節に分ける。``` の中は見出しにしない。
fn read_doc(text: &str) -> Doc {
    let front = crate::runs::read_frontmatter(text);
    let mut body = text;
    if text.starts_with("---") {
        let mut offset = 0usize;
        let mut seen_open = false;
        for line in text.split_inclusive('\n') {
            offset += line.len();
            if line.trim() == "---" {
                if seen_open {
                    body = &text[offset..];
                    break;
                }
                seen_open = true;
            }
        }
    }
    let mut explain: Vec<&str> = Vec::new();
    let mut sections: Vec<(String, Vec<String>)> = Vec::new();
    let mut fenced = false;
    for line in body.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("```") {
            fenced = !fenced;
        } else if !fenced {
            if let Some(name) = trimmed.strip_prefix("## ") {
                sections.push((name.trim().to_string(), Vec::new()));
                continue;
            }
            if sections.is_empty() {
                if !trimmed.starts_with("# ") {
                    explain.push(line);
                }
                continue;
            }
        }
        if let Some((_, lines)) = sections.last_mut() {
            lines.push(line.to_string());
        }
    }
    Doc { front, explain: explain.join("\n").trim().to_string(), sections }
}

fn is_old_type(doc: &Doc) -> bool {
    doc.has_section(&["ITEM_ORDER", "項目の順序", "GATE", "関門"])
}

fn is_old_guide(doc: &Doc) -> bool {
    doc.front("type").is_some()
        || doc.front("parent").is_some()
        || doc.has_section(&["CONDITION", "CHILD_ITEM"])
}

fn invalid<T>(message: impl Into<String>) -> Result<T> {
    Err(RtError::Invalid(message.into()))
}

/// 型の md を読む。
pub fn parse_type(text: &str) -> Result<TypeDef> {
    let doc = read_doc(text);
    let need = |name: &str| -> Result<String> {
        doc.front(name)
            .map(str::to_string)
            .ok_or_else(|| RtError::Invalid(format!("型の frontmatter に {name} がありません")))
    };
    let (id, name, root) = (need("id")?, need("name")?, need("root")?);
    if !id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-') {
        return invalid(format!("型の id に使えない文字があります: {id}"));
    }
    let lines = doc.section("Q").unwrap_or_default();
    let mut tree: Vec<TypeQ> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    let mut prev_depth: Option<usize> = None;
    for (index, line) in lines.iter().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let bad = || RtError::Invalid(format!("型の ## Q の {} 行目が読めません", index + 1));
        let indent = line.len() - line.trim_start_matches(' ').len();
        let rest = &line[indent..];
        let (per_option, key) = if let Some(k) = rest.strip_prefix("- ") {
            (false, k.trim())
        } else if let Some(k) = rest.strip_prefix("+ ") {
            (true, k.trim())
        } else {
            return Err(bad());
        };
        if indent % 2 != 0 || key.is_empty() {
            return Err(bad());
        }
        let depth = indent / 2;
        if depth > prev_depth.map_or(0, |d| d + 1) {
            return Err(bad());
        }
        if depth == 0 && per_option {
            return invalid("型の ## Q の一番浅い行に + は使えません");
        }
        if !seen.insert(key.to_string()) {
            return invalid(format!("型の ## Q に同じ key が 2 回あります: {key}"));
        }
        let mut level = &mut tree;
        for _ in 0..depth {
            level = &mut level.last_mut().expect("depth は直前の行で確かめ済み").children;
        }
        level.push(TypeQ { key: key.to_string(), per_option, children: Vec::new() });
        prev_depth = Some(depth);
    }
    if tree.is_empty() {
        return invalid("型の ## Q が空です");
    }
    Ok(TypeDef { id, name, root, explain: doc.explain.clone(), tree, seed: doc.joined("SEED") })
}

/// ガイドの md を読む。
pub fn parse_guide(text: &str) -> Result<GuideDef> {
    let doc = read_doc(text);
    let need = |name: &str| -> Result<String> {
        doc.front(name)
            .map(str::to_string)
            .ok_or_else(|| RtError::Invalid(format!("ガイドの frontmatter に {name} がありません")))
    };
    let (key, name) = (need("key")?, need("name")?);
    let target = match doc.front("target") {
        None => 2,
        Some(v) => v
            .parse::<i64>()
            .ok()
            .filter(|n| (0..=99).contains(n))
            .ok_or_else(|| RtError::Invalid(format!("ガイド {key} の target は 0〜99 の整数です: {v}")))?,
    };
    Ok(GuideDef {
        key,
        name,
        explain: doc.explain.clone(),
        question: doc.joined("Q"),
        child_candidates: doc.bullets("Child Q"),
        extract: doc.bullets("Extract"),
        pitfalls: doc.bullets("Pitfall"),
        target,
    })
}

// ---- 読み込み ------------------------------------------------------------------------

/// 古い書式のファイルは重ねずに飛ばす（埋め込みの新しい版を残すため）。
fn overlay(map: &mut BTreeMap<String, String>, base: &Path, warnings: &mut Vec<String>) {
    for sub in ["types", "guides"] {
        let Ok(entries) = fs::read_dir(base.join("sci").join(sub)) else { continue };
        for entry in entries.flatten() {
            let path = entry.path();
            let Some(file) = path.file_name().and_then(|f| f.to_str()) else { continue };
            if !file.ends_with(".md") {
                continue;
            }
            let Ok(text) = fs::read_to_string(&path) else { continue };
            let doc = read_doc(&text);
            let relative = format!("{sub}/{file}");
            if (sub == "types" && is_old_type(&doc)) || (sub == "guides" && is_old_guide(&doc)) {
                warnings.push(format!("古い書式のため読みませんでした: {relative}"));
            } else {
                map.insert(relative, text);
            }
        }
    }
}

/// 埋め込み → 利用者全体 → プロジェクトの順に重ねて読む。
#[must_use]
pub fn load(root: &Path) -> Defs {
    load_with_home(root, crate::paths::sai_home().as_deref())
}

fn load_with_home(root: &Path, home: Option<&Path>) -> Defs {
    let mut files: BTreeMap<String, String> =
        EMBEDDED.iter().map(|(p, t)| ((*p).to_string(), (*t).to_string())).collect();
    let mut defs = Defs::default();
    if let Some(home) = home {
        overlay(&mut files, home, &mut defs.warnings);
    }
    overlay(&mut files, &root.join(".sai"), &mut defs.warnings);

    for (path, text) in &files {
        let is_type = path.starts_with("types/");
        if is_type {
            match parse_type(text) {
                Ok(t) => defs.types.push(t),
                Err(e) => defs.warnings.push(format!("読めませんでした: {path}: {e}")),
            }
        } else {
            match parse_guide(text) {
                Ok(g) => defs.guides.push(g),
                Err(e) => defs.warnings.push(format!("読めませんでした: {path}: {e}")),
            }
        }
    }
    let rank = |id: &str| TYPE_ORDER.iter().position(|t| *t == id).unwrap_or(TYPE_ORDER.len());
    defs.types.sort_by(|a, b| (rank(&a.id), &a.id).cmp(&(rank(&b.id), &b.id)));
    defs.guides.sort_by(|a, b| a.key.cmp(&b.key));

    fn keys<'a>(qs: &'a [TypeQ], out: &mut Vec<&'a str>) {
        for q in qs {
            out.push(&q.key);
            keys(&q.children, out);
        }
    }
    let mut missing = Vec::new();
    for t in &defs.types {
        let mut ks = Vec::new();
        keys(&t.tree, &mut ks);
        for k in ks {
            if defs.guide(k).is_none() {
                missing.push(format!("型 {} の Q {k} のガイドがありません", t.id));
            }
        }
    }
    defs.warnings.extend(missing);
    defs
}

// ---- 木を作る ------------------------------------------------------------------------

/// [`init_tree`] の結果。`created` は root を含む作った Q の数。
#[derive(Debug, Clone)]
pub struct InitReport {
    pub root: i64,
    pub created: usize,
    pub warnings: Vec<String>,
}

fn add_q(
    conn: &Connection,
    defs: &Defs,
    parent: i64,
    q: &TypeQ,
    created_by: &str,
    run_id: Option<i64>,
    ids: &mut Vec<i64>,
) -> Result<()> {
    let text = defs.question_text(&q.key);
    let id = store::insert_node(
        conn,
        &NewNode {
            kind: Kind::Q,
            parent_id: Some(parent),
            guide_key: Some(&q.key),
            text: &text,
            body: "",
            answer_kind: None,
            cites: &[],
            paper_id: None,
            run_id,
            report_id: None,
            mlflow_ref: None,
            proposal_id: None,
            created_by,
        },
    )?;
    ids.push(id);
    // `+` の子はその Q に O が足されたときに作る。
    for child in q.children.iter().filter(|c| !c.per_option) {
        add_q(conn, defs, id, child, created_by, run_id, ids)?;
    }
    Ok(())
}

/// 型から一番上の Q と `-` の Q を作る（`+` の Q は作らない）。
pub fn init_tree(conn: &Connection, defs: &Defs, type_id: &str, created_by: &str) -> Result<InitReport> {
    if store::is_initialized(conn)? {
        return invalid("研究方針の木は既にあります");
    }
    let Some(type_def) = defs.type_def(type_id) else {
        return invalid(format!("知らない研究の型です: {type_id}"));
    };
    let tx = conn.unchecked_transaction()?;
    let root = store::insert_node(
        &tx,
        &NewNode {
            kind: Kind::Q,
            parent_id: None,
            guide_key: None,
            text: &type_def.root,
            body: "",
            answer_kind: None,
            cites: &[],
            paper_id: None,
            run_id: None,
            report_id: None,
            mlflow_ref: None,
            proposal_id: None,
            created_by,
        },
    )?;
    let mut ids = vec![root];
    for q in &type_def.tree {
        add_q(&tx, defs, root, q, created_by, None, &mut ids)?;
    }
    store::meta_set(&tx, "type", type_id)?;
    store::meta_set(&tx, "initialized_at", &now_text())?;
    tx.commit()?;
    Ok(InitReport { root, created: ids.len(), warnings: defs.warnings.clone() })
}

/// O の下に、型の `+` の Q（と、その下の `-` の Q）を作る。作った Q の id を返す。
/// 呼ぶ側のトランザクションの中で使う前提で、自分では始めない。
pub fn create_qs_for_option(
    conn: &Connection,
    defs: &Defs,
    o_id: i64,
    created_by: &str,
    run_id: Option<i64>,
) -> Result<Vec<i64>> {
    let o = store::get_node(conn, o_id)?;
    let Some(parent_id) = o.parent_id else { return Ok(Vec::new()) };
    let Some(guide_key) = store::get_node(conn, parent_id)?.guide_key else { return Ok(Vec::new()) };
    let Some(type_id) = store::meta_get(conn, "type")? else { return Ok(Vec::new()) };
    let mut ids = Vec::new();
    for q in defs.per_option_children(&type_id, &guide_key) {
        add_q(conn, defs, o_id, q, created_by, run_id, &mut ids)?;
    }
    Ok(ids)
}

// ---- 写しと利用者の型 ----------------------------------------------------------------

/// 埋め込みのうち `<root>/.sai/sci/types|guides/` に無いものだけを書く。書いた数を返す。
pub fn write_local_copies(root: &Path) -> Result<usize> {
    let base = root.join(".sai").join("sci");
    let mut written = 0;
    for (relative, text) in EMBEDDED {
        let path = base.join(relative);
        if path.exists() {
            continue;
        }
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        fs::write(&path, text)?;
        written += 1;
    }
    Ok(written)
}

/// 型を利用者全体の置き場 `<sai_home>/sci/types/<id>.md` に保存する。
pub fn save_user_type(text: &str) -> Result<PathBuf> {
    let Some(home) = crate::paths::sai_home() else {
        return invalid("利用者全体の置き場が分かりません");
    };
    save_user_type_in(&home, text)
}

fn save_user_type_in(home: &Path, text: &str) -> Result<PathBuf> {
    let type_def = parse_type(text)?;
    let path = home.join("sci").join("types").join(format!("{}.md", type_def.id));
    if path.exists() {
        return invalid(format!("型 {} は既にあります", type_def.id));
    }
    fs::create_dir_all(path.parent().expect("パスには親がある"))?;
    fs::write(&path, text)?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::Database;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn temp_dir() -> PathBuf {
        static N: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "rt-defs-test-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn texts(conn: &Connection, parent: i64) -> Vec<String> {
        store::children(conn, parent)
            .unwrap()
            .into_iter()
            .map(|n| n.guide_key.unwrap_or_default())
            .collect()
    }

    fn type_md(q: &str) -> String {
        format!("---\nid: t1\nname: T\nroot: 根\n---\n# T\n\n## Q\n{q}\n")
    }

    #[test]
    fn embedded_loads_without_warnings() {
        let root = temp_dir();
        let defs = load_with_home(&root, None);
        assert_eq!(embedded().len(), 30);
        assert!(defs.warnings.is_empty(), "{:?}", defs.warnings);
        assert_eq!(defs.types.len(), 5);
        assert_eq!(defs.guides.len(), 25);
        let ids: Vec<_> = defs.types.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(ids, TYPE_ORDER);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn init_tree_makes_static_qs_only() {
        let root = temp_dir();
        let defs = load_with_home(&root, None);
        let db = Database::in_memory().unwrap();
        let conn = db.connection();
        let report = init_tree(conn, &defs, "problem", "test").unwrap();
        let top = store::get_node(conn, report.root).unwrap();
        assert_eq!(top.text, "この研究は、どの現場の、どんな課題を解くか");
        assert_eq!(top.guide_key, None);
        assert_eq!(texts(conn, report.root), ["background", "target", "purpose"]);
        let by_key = |k: &str| {
            store::list_nodes(conn, false).unwrap().into_iter().find(|n| n.guide_key.as_deref() == Some(k))
        };
        assert_eq!(texts(conn, by_key("background").unwrap().id), ["background.goal", "background.problem"]);
        assert_eq!(texts(conn, by_key("target").unwrap().id), ["target.subgroup"]);
        assert_eq!(texts(conn, by_key("purpose").unwrap().id), ["purpose.subgoal"]);
        assert!(texts(conn, by_key("purpose.subgoal").unwrap().id).is_empty());
        assert!(by_key("data").is_none() && by_key("existing").is_none());
        assert_eq!(report.created, 8);
        assert_eq!(store::meta_get(conn, "type").unwrap().as_deref(), Some("problem"));
        assert!(matches!(init_tree(conn, &defs, "problem", "test"), Err(RtError::Invalid(_))));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn create_qs_for_option_makes_per_option_qs() {
        let root = temp_dir();
        let defs = load_with_home(&root, None);
        let db = Database::in_memory().unwrap();
        let conn = db.connection();
        init_tree(conn, &defs, "problem", "test").unwrap();
        let sub = store::list_nodes(conn, false)
            .unwrap()
            .into_iter()
            .find(|n| n.guide_key.as_deref() == Some("purpose.subgoal"))
            .unwrap();
        let o = store::insert_node(
            conn,
            &NewNode {
                kind: Kind::O,
                parent_id: Some(sub.id),
                guide_key: None,
                text: "予後予測",
                body: "",
                answer_kind: Some("answer"),
                cites: &[],
                paper_id: None,
                run_id: None,
                report_id: None,
                mlflow_ref: None,
                proposal_id: None,
                created_by: "test",
            },
        )
        .unwrap();
        let ids = create_qs_for_option(conn, &defs, o, "test", None).unwrap();
        assert_eq!(ids.len(), 17);
        assert_eq!(
            texts(conn, o),
            ["purpose.output", "purpose.metric", "data", "existing", "issue", "method", "novelty"]
        );
        let data = store::children(conn, o).unwrap().into_iter().find(|n| n.guide_key.as_deref() == Some("data")).unwrap();
        assert_eq!(texts(conn, data.id), ["data.type", "data.size", "data.label", "data.access"]);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn old_format_copy_is_skipped_with_warning() {
        let root = temp_dir();
        let dir = root.join(".sai/sci/types");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("problem.md"), "---\nid: problem\nname: X\n---\n## ITEM_ORDER\n- a\n").unwrap();
        let defs = load_with_home(&root, None);
        assert_eq!(defs.warnings, ["古い書式のため読みませんでした: types/problem.md"]);
        assert_eq!(defs.types.len(), 5, "埋め込みの新しい problem が残る");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn user_type_is_saved_and_loaded() {
        let root = temp_dir();
        let home = temp_dir();
        let md = "---\nid: mine\nname: MINE\nroot: 根\n---\n\n## Q\n- background\n";
        let path = save_user_type_in(&home, md).unwrap();
        assert!(path.ends_with("sci/types/mine.md"));
        assert!(matches!(save_user_type_in(&home, md), Err(RtError::Invalid(_))));
        let defs = load_with_home(&root, Some(&home));
        assert!(defs.type_def("mine").is_some());
        assert_eq!(defs.types.last().unwrap().id, "mine");
        let _ = fs::remove_dir_all(&root);
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn parse_type_rejects_bad_input() {
        assert!(parse_type(&type_md("- a\n  - b\n    + c")).is_ok());
        for q in ["- a\n   - b", "+ a", "- a\n    - b", "- a\n- a", "a", ""] {
            assert!(matches!(parse_type(&type_md(q)), Err(RtError::Invalid(_))), "{q:?}");
        }
        for md in ["---\nname: T\nroot: r\n---\n## Q\n- a\n", "---\nid: t\nroot: r\n---\n## Q\n- a\n", "---\nid: t\nname: T\n---\n## Q\n- a\n"] {
            assert!(matches!(parse_type(md), Err(RtError::Invalid(_))));
        }
        assert!(matches!(parse_guide("---\nname: N\n---\n"), Err(RtError::Invalid(_))));
        assert!(matches!(parse_guide("---\nkey: k\nname: N\ntarget: 100\n---\n"), Err(RtError::Invalid(_))));
        assert_eq!(parse_guide("---\nkey: k\nname: N\n---\n").unwrap().target, 2);
    }

    #[test]
    fn write_local_copies_skips_existing() {
        let root = temp_dir();
        assert_eq!(write_local_copies(&root).unwrap(), 30);
        assert_eq!(write_local_copies(&root).unwrap(), 0);
        assert!(load_with_home(&root, None).warnings.is_empty());
        let _ = fs::remove_dir_all(&root);
    }
}
