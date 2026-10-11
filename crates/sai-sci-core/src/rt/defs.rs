//! RESEARCH_TYPE と GUIDE の読み込み、RESEARCH_TYPE から Q の research-tree を作る処理（C-DEF）。
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

/// RESEARCH_TYPE の `## Q` の 1 行。
#[derive(Debug, Clone, PartialEq)]
pub struct TypeQ {
    pub key: String,
    pub children: Vec<TypeQ>,
}

/// RESEARCH_TYPE。
#[derive(Debug, Clone, PartialEq)]
pub struct TypeDef {
    pub id: String,
    pub name: String,
    pub explain: String,
    pub tree: Vec<TypeQ>,
}

/// Q ごとの GUIDE。
#[derive(Debug, Clone, PartialEq)]
pub struct GuideDef {
    pub key: String,
    pub name: String,
    pub explain: String,
    pub question: String,
    pub child_candidates: Vec<String>,
    pub extract: Vec<String>,
    pub pitfalls: Vec<String>,
}

/// 読み込んだ RESEARCH_TYPE と GUIDE。
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

    /// Q の文。GUIDE の question が空でなければそれ、無ければ `<name>は？`、GUIDE が無ければ key。
    #[must_use]
    pub fn question_text(&self, key: &str) -> String {
        match self.guide(key) {
            Some(g) if !g.question.is_empty() => g.question.clone(),
            Some(g) => format!("{}は？", g.name),
            None => key.to_string(),
        }
    }
}

/// RESEARCH_TYPE の並び。これ以外は id 順で後ろに置く。
pub const TYPE_ORDER: [&str; 5] = ["problem", "phenomenon", "method", "data", "improve"];

/// 埋め込みの一覧を `include_str!` の繰り返しなく書くための補助。
/// 相対パスは `sai-sci/` を起点にする（`types/<id>.md`・`guides/<key>.md`）。
macro_rules! embedded_files {
    ($($relative:literal),* $(,)?) => {
        &[$(($relative, include_str!(concat!("../../../../sai-sci/", $relative)))),*]
    };
}

const EMBEDDED: &[(&str, &str)] = embedded_files![
    // RESEARCH_TYPE（5 本。`## Q` は軸の 6 つの平らな並び）
    "types/problem.md",
    "types/phenomenon.md",
    "types/method.md",
    "types/data.md",
    "types/improve.md",
    // GUIDE（軸の 6 本）
    "guides/task.md",
    "guides/target.md",
    "guides/data.md",
    "guides/method.md",
    "guides/output.md",
    "guides/metric.md",
];

/// 埋め込みの一覧（`("types/problem.md", 本文)` など 11 個）。
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

/// RESEARCH_TYPE の md を読む。
pub fn parse_type(text: &str) -> Result<TypeDef> {
    let doc = read_doc(text);
    let need = |name: &str| -> Result<String> {
        doc.front(name)
            .map(str::to_string)
            .ok_or_else(|| RtError::Invalid(format!("RESEARCH_TYPE の frontmatter に {name} がありません")))
    };
    let (id, name) = (need("id")?, need("name")?);
    if !id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-') {
        return invalid(format!("RESEARCH_TYPE の id に使えない文字があります: {id}"));
    }
    let lines = doc.section("Q").unwrap_or_default();
    let mut tree: Vec<TypeQ> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    let mut prev_depth: Option<usize> = None;
    for (index, line) in lines.iter().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let bad = || RtError::Invalid(format!("RESEARCH_TYPE の ## Q の {} 行目が読めません", index + 1));
        let indent = line.len() - line.trim_start_matches(' ').len();
        let rest = &line[indent..];
        // `+` は `- ` と同じに読む（保存された古い型を読めるように）。
        let Some(key) = rest.strip_prefix("- ").or_else(|| rest.strip_prefix("+ ")).map(str::trim) else {
            return Err(bad());
        };
        if indent % 2 != 0 || key.is_empty() {
            return Err(bad());
        }
        let depth = indent / 2;
        if depth > prev_depth.map_or(0, |d| d + 1) {
            return Err(bad());
        }
        if !seen.insert(key.to_string()) {
            return invalid(format!("RESEARCH_TYPE の ## Q に同じ key が 2 回あります: {key}"));
        }
        let mut level = &mut tree;
        for _ in 0..depth {
            level = &mut level.last_mut().expect("depth は直前の行で確かめ済み").children;
        }
        level.push(TypeQ { key: key.to_string(), children: Vec::new() });
        prev_depth = Some(depth);
    }
    if tree.is_empty() {
        return invalid("RESEARCH_TYPE の ## Q が空です");
    }
    Ok(TypeDef { id, name, explain: doc.explain.clone(), tree })
}

/// GUIDE の md を読む。
pub fn parse_guide(text: &str) -> Result<GuideDef> {
    let doc = read_doc(text);
    let need = |name: &str| -> Result<String> {
        doc.front(name)
            .map(str::to_string)
            .ok_or_else(|| RtError::Invalid(format!("GUIDE の frontmatter に {name} がありません")))
    };
    let (key, name) = (need("key")?, need("name")?);
    Ok(GuideDef {
        key,
        name,
        explain: doc.explain.clone(),
        question: doc.joined("Q"),
        child_candidates: doc.bullets("Child Q"),
        extract: doc.bullets("Extract"),
        pitfalls: doc.bullets("Pitfall"),
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
    load_with_home(root, sai_paper_core::paths::sai_home().as_deref())
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
                missing.push(format!("RESEARCH_TYPE {} の Q {k} の GUIDE がありません", t.id));
            }
        }
    }
    defs.warnings.extend(missing);
    defs
}

// ---- research-tree を作る ------------------------------------------------------------------------

/// [`init_tree`] の結果。`created` は作った Q の数。
#[derive(Debug, Clone)]
pub struct InitReport {
    pub created: usize,
    pub warnings: Vec<String>,
}

fn add_q(
    conn: &Connection,
    defs: &Defs,
    parent: Option<i64>,
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
            parent_id: parent,
            guide_key: Some(&q.key),
            text: &text,
            body: "",
            stance: None,
            cites: &[],
            paper_id: None,
            run_id,
            report_id: None,
            mlflow_ref: None,
            dataset_ref: None,
            paths: &[],
            created_by,
        },
    )?;
    ids.push(id);
    for child in &q.children {
        add_q(conn, defs, Some(id), child, created_by, run_id, ids)?;
    }
    Ok(())
}

/// RESEARCH_TYPE の `## Q` の 1 段目を top_Q（`parent_id` は NULL）として作り、その下の Q も作る。
pub fn init_tree(conn: &Connection, defs: &Defs, type_id: &str, created_by: &str) -> Result<InitReport> {
    if store::is_initialized(conn)? {
        return invalid("research-tree は既にあります");
    }
    let Some(type_def) = defs.type_def(type_id) else {
        return invalid(format!("知らない RESEARCH_TYPE です: {type_id}"));
    };
    let tx = conn.unchecked_transaction()?;
    let mut ids = Vec::new();
    for q in &type_def.tree {
        add_q(&tx, defs, None, q, created_by, None, &mut ids)?;
    }
    store::meta_set(&tx, "type", type_id)?;
    store::meta_set(&tx, "initialized_at", &now_text())?;
    tx.commit()?;
    Ok(InitReport { created: ids.len(), warnings: defs.warnings.clone() })
}

// ---- 写しと利用者の RESEARCH_TYPE ----------------------------------------------------------------

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

/// RESEARCH_TYPE を利用者全体の置き場 `<sai_home>/sci/types/<id>.md` に保存する。
pub fn save_user_type(text: &str) -> Result<PathBuf> {
    let Some(home) = sai_paper_core::paths::sai_home() else {
        return invalid("利用者全体の置き場が分かりません");
    };
    save_user_type_in(&home, text)
}

fn save_user_type_in(home: &Path, text: &str) -> Result<PathBuf> {
    let type_def = parse_type(text)?;
    let path = home.join("sci").join("types").join(format!("{}.md", type_def.id));
    if path.exists() {
        return invalid(format!("RESEARCH_TYPE {} は既にあります", type_def.id));
    }
    fs::create_dir_all(path.parent().expect("パスには親がある"))?;
    fs::write(&path, text)?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sai_paper_core::database::Database;
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
        format!("---\nid: t1\nname: T\n---\n# T\n\n## Q\n{q}\n")
    }

    #[test]
    fn embedded_loads_without_warnings() {
        let root = temp_dir();
        let defs = load_with_home(&root, None);
        assert_eq!(embedded().len(), 11);
        assert!(defs.warnings.is_empty(), "{:?}", defs.warnings);
        assert_eq!(defs.types.len(), 5);
        assert_eq!(defs.guides.len(), 6);
        let ids: Vec<_> = defs.types.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(ids, TYPE_ORDER);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn init_tree_makes_static_qs_only() {
        let root = temp_dir();
        let defs = load_with_home(&root, None);
        let axes = ["task", "target", "data", "method", "output", "metric"];
        for type_id in TYPE_ORDER {
            let db = Database::in_memory().unwrap();
            let conn = db.connection();
            let report = init_tree(conn, &defs, type_id, "test").unwrap();
            // どの型でも top_Q は軸の 6 つ。子は作らない。
            let tops = store::list_nodes(conn, false)
                .unwrap()
                .into_iter()
                .filter(|n| n.kind == Kind::Q && n.parent_id.is_none())
                .collect::<Vec<_>>();
            let keys: Vec<_> = tops.iter().map(|n| n.guide_key.clone().unwrap_or_default()).collect();
            assert_eq!(keys, axes, "{type_id}");
            assert_eq!(report.created, 6, "{type_id}");
            for top in &tops {
                assert!(texts(conn, top.id).is_empty());
                assert_eq!(top.text, defs.question_text(top.guide_key.as_deref().unwrap()));
            }
            let want = [
                "この分野で解かれている問題（タスク）には、どんな種類があるか。",
                "研究の対象には、どんな種類があるか。",
                "使われているデータには、どんな種類があるか。",
                "使われている手法には、どんな種類があるか。",
                "研究が出す出力の形には、どんな種類があるか。",
                "使われている評価指標には、どんな種類があるか。",
            ];
            let got: Vec<_> = tops.iter().map(|n| n.text.as_str()).collect();
            assert_eq!(got, want, "{type_id}");
        }
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn types_are_flat_axes_with_seed_section() {
        let root = temp_dir();
        let defs = load_with_home(&root, None);
        for t in &defs.types {
            let keys: Vec<_> = t.tree.iter().map(|q| q.key.as_str()).collect();
            assert_eq!(keys, ["task", "target", "data", "method", "output", "metric"], "{}", t.id);
            assert!(t.tree.iter().all(|q| q.children.is_empty()), "{}", t.id);
        }
        // `## SEED` は engine が読まないが、型の md には入っている（sci-start と利用者が読む）。
        for (path, text) in EMBEDDED.iter().filter(|(p, _)| p.starts_with("types/")) {
            assert!(text.contains("\n## SEED\n"), "{path}");
            assert!(!text.lines().any(|l| l.starts_with("+ ") || l.starts_with("  + ")), "{path}");
        }
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
        let md = "---\nid: mine\nname: MINE\n---\n\n## Q\n- background\n";
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
        // `+` は `-` と同じに読む（一番浅い `+` も通る）。
        assert!(parse_type(&type_md("- a\n  - b\n    + c")).is_ok());
        assert_eq!(parse_type(&type_md("+ a")).unwrap().tree, parse_type(&type_md("- a")).unwrap().tree);
        assert_eq!(parse_type(&type_md("- a\n  + b")).unwrap().tree[0].children[0].key, "b");
        for q in ["- a\n   - b", "- a\n    - b", "- a\n- a", "- a\n+ a", "a", ""] {
            assert!(matches!(parse_type(&type_md(q)), Err(RtError::Invalid(_))), "{q:?}");
        }
        for md in [
            // id がない。
            "---\nname: T\nroot: r\n---\n## Q\n- a\n",
            // name がない。
            "---\nid: t\nroot: r\n---\n## Q\n- a\n",
            // root が無くても通る（古い md の root: は読み捨てる）。
        ] {
            assert!(matches!(parse_type(md), Err(RtError::Invalid(_))));
        }
        assert!(parse_type("---\nid: t\nname: T\n---\n## Q\n- a\n").is_ok());
        assert!(parse_type("---\nid: t\nname: T\nroot: 古い根\n---\n## Q\n- a\n## SEED\n文\n").is_ok());
        assert!(matches!(parse_guide("---\nname: N\n---\n"), Err(RtError::Invalid(_))));
        // target は読まない（書いてあっても無視する）。
        assert!(parse_guide("---\nkey: k\nname: N\ntarget: 100\n---\n").is_ok());
    }

    #[test]
    fn write_local_copies_skips_existing() {
        let root = temp_dir();
        assert_eq!(write_local_copies(&root).unwrap(), 11);
        assert_eq!(write_local_copies(&root).unwrap(), 0);
        assert!(load_with_home(&root, None).warnings.is_empty());
        let _ = fs::remove_dir_all(&root);
    }
}
