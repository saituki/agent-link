//! エージェントに渡す `# context` の Markdown（新しい木 Q・O・F・E 版）。
//!
//! 木の一部（範囲）だけを渡す。ファイルは作らず、文字列を返す（`# context` の見出しは呼び出し側が付ける）。
//! 範囲はブロックの `sci_type`（node / manage / support）で決める。
//! 長さは `max_chars` で制限し、はみ出したら level 0〜5 の順に 1 段ずつ削る。
//! 節の見出しの名前（`## TREE` など）は sci-core が指すので変えない。

use std::collections::{HashMap, HashSet};

use rusqlite::{Connection, OptionalExtension};

use super::compute::Tree;
use super::defs::Defs;
use super::ops::{self, Field, Risk, Ty};
use super::store::{self, NodeRow};
use super::{Kind, Result, RtError, RtKeys, action, fmt_id, task_keys};
use sai_paper_core::paths::PaperRoots;

const NO_TREE: &str = "\
## TREE
research-tree がまだありません。利用者に `sai-sci init --type <型>`（または Web の research-tree の画面の「始める」）を頼んでください。";

/// run の task.md から `# context` の Markdown を作る。`reveal` が真なら blind（`sci_mode: blind_first`）でも木を出す。
///
/// # Errors
///
/// run が見つからない、対象の Node が無い、DB のエラー。
pub fn build(
    conn: &Connection,
    roots: &PaperRoots,
    defs: &Defs,
    max_chars: usize,
    run_id: i64,
    reveal: bool,
) -> Result<String> {
    let not_found = || RtError::NotFound(format!("実行 {run_id} が見つかりません"));
    let run = crate::runs::list_run_dirs(&roots.root)
        .into_iter()
        .find(|run| run.run_id == run_id)
        .ok_or_else(not_found)?;
    let text = std::fs::read_to_string(run.path.join("task.md")).map_err(|_| not_found())?;
    let block = crate::runs::read_frontmatter(&text)
        .into_iter()
        .find(|(key, _)| key == "block")
        .map_or_else(String::new, |(_, value)| value.trim().to_string());
    let keys = task_keys(&text);

    if !store::is_initialized(conn)? {
        return Ok(NO_TREE.to_string());
    }
    let tree = Tree::load(conn, roots, defs)?;
    let parts = Parts::collect(conn, roots, defs, &tree, &block, &keys, reveal)?;

    for level in 0..=5u8 {
        let out = parts.render(level);
        if out.chars().count() <= max_chars {
            return Ok(out);
        }
    }
    let out = parts.render(5);
    let kept: String = out.chars().take(max_chars).collect();
    let omitted = out.chars().count() - max_chars;
    Ok(format!("{kept}\n…（{omitted} 字を省きました）"))
}

// ---- ALLOWED_OPS ---------------------------------------------------------------------

fn field_text(f: &Field) -> String {
    let note = match f.ty {
        Ty::Id if f.name == "q" => "Q の ID".to_string(),
        Ty::Id => "Node の ID".to_string(),
        Ty::Text(min, max) => format!("{min}〜{max} 字"),
        Ty::Int => "整数".to_string(),
        Ty::NullInt => "0〜99 か null".to_string(),
        Ty::Enum(values) => values.join(" / "),
        Ty::List(min, max) => format!("文字列の配列、{min}〜{max} 個"),
    };
    format!("{}（{note}）", f.name)
}

/// ブロックが出せる op の説明（ALLOWED_OPS の節の中身）。
#[must_use]
pub fn allowed_ops_text(roots: &PaperRoots, block: &str) -> String {
    let mut lines = Vec::new();
    for name in ops::allowed_ops(roots, block) {
        let Some(spec) = ops::op_spec(&name) else {
            lines.push(format!("- {name}（知らない op です）"));
            continue;
        };
        let how = match spec.risk {
            Risk::Auto => "すぐ入る",
            Risk::Human => "利用者が仕分けてから入る",
            Risk::UserOnly => continue, // 利用者だけの op は PROPOSAL では出せない。
        };
        lines.push(format!("- {}（{how}）: {}。", spec.op, spec.about));
        for (label, required) in [("必須", true), ("任意", false)] {
            let items: Vec<String> =
                spec.fields.iter().filter(|f| f.required == required).map(field_text).collect();
            if !items.is_empty() {
                lines.push(format!("  {label}: {}", items.join(", ")));
            }
        }
    }
    lines.push(
        "出し方: sai-sci propose（標準入力に {\"proposals\": [...]}）。1 件でも違反があれば何も入りません。"
            .to_string(),
    );
    lines.join("\n")
}

// ---- 節の材料 -------------------------------------------------------------------------

/// 空白を 1 つにまとめて 1 行にする。
fn flat_text(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Node の 1 行（`full` は全文、`first` は 1 行目だけ）。対象の行だけ `body` を持つ。
struct Line {
    kind: Kind,
    full: String,
    first: String,
    indent: usize,
    /// 対象の本文（1 行にした物）。無ければ `None`。
    body: Option<String>,
}

struct Parts {
    tree: Option<Vec<Line>>,
    /// (Q の文, Objectivity の目安, Child Q, Extract, Pitfall)
    guide: Option<GuideText>,
    searches: Vec<String>,
    ops: String,
    outlines: Vec<String>,
    outline_gists: Vec<Vec<String>>,
    papers: Vec<String>,
}

struct GuideText {
    title: String,
    question: String,
    target: i64,
    child: Vec<String>,
    extract: Vec<String>,
    pitfalls: Vec<String>,
}

fn node_line(tree: &Tree, n: &NodeRow, indent: usize) -> Line {
    let pad = " ".repeat(indent * 2);
    let id = fmt_id(n.kind, n.id);
    let make = |text: String| {
        let tail = match n.kind {
            Kind::Q => {
                let o = tree.objectivity(n.id);
                let obj = o.map_or_else(String::new, |o| {
                    format!(" [Objectivity {}/{}{}]", o.count, o.target, if o.met { " met" } else { "" })
                });
                let logic = tree.logicality(n.id).map_or_else(String::new, |l| format!(" [Logicality {}]", l.value));
                format!("{obj}{logic}")
            }
            Kind::O => format!(" [F {}]", tree.f_count_under(n.id)),
            Kind::F if !n.cites.is_empty() => format!(" [{}]", n.cites.join(", ")),
            Kind::E => n.mlflow_ref.as_deref().filter(|m| !m.is_empty()).map_or_else(String::new, |m| format!(" [mlflow: {m}]")),
            Kind::F => String::new(),
        };
        let head = match n.kind {
            Kind::O => {
                let star = if n.starred { " ★" } else { "" };
                let ak = if n.answer_kind.as_deref() == Some("issue") { "Issue" } else { "Answer" };
                format!("{id}{star} {ak} {text}")
            }
            _ => format!("{id} {text}"),
        };
        format!("{pad}- {head}{tail}")
    };
    Line {
        kind: n.kind,
        full: make(flat_text(&n.text)),
        first: make(n.text.lines().next().unwrap_or("").trim().to_string()),
        indent,
        body: None,
    }
}

impl Parts {
    #[allow(clippy::too_many_arguments)]
    fn collect(
        conn: &Connection,
        roots: &PaperRoots,
        defs: &Defs,
        tree: &Tree,
        block: &str,
        keys: &RtKeys,
        reveal: bool,
    ) -> Result<Parts> {
        // 読み方（`sci_mode: blind_first`）は木を見ずに読む型。`--tree` で木を出す。
        let blind = keys.mode == "blind_first" && !reveal;
        // ブロックの分類（node / manage / support）。無い・知らない値は一番狭い `node`。
        let sci_type = action::block_sci_type(roots, block)
            .filter(|t| action::SCI_TYPES.contains(&t.as_str()))
            .unwrap_or_else(|| "node".to_string());

        let top = tree.nodes().iter().find(|n| n.parent_id.is_none() && n.kind == Kind::Q);
        let target = match keys.node {
            Some((kind, id)) => tree
                .node(id)
                .filter(|n| n.kind == kind)
                .ok_or_else(|| RtError::NotFound(format!("{} はありません", fmt_id(kind, id))))?,
            None => top.ok_or_else(|| RtError::NotFound("一番上の Q がありません".into()))?,
        };
        let mut kids: HashMap<i64, Vec<i64>> = HashMap::new();
        for n in tree.nodes() {
            if let Some(p) = n.parent_id {
                kids.entry(p).or_default().push(n.id);
            }
        }
        let subtree = |id: i64| {
            let mut out = vec![id];
            let mut i = 0;
            while i < out.len() {
                out.extend(kids.get(&out[i]).cloned().unwrap_or_default());
                i += 1;
            }
            out
        };
        // 一番上から対象の親まで（manage で祖先を出すのに使う）。
        let mut chain: Vec<i64> = Vec::new();
        let mut cur = target.parent_id;
        while let Some(id) = cur {
            chain.push(id);
            cur = tree.node(id).and_then(|n| n.parent_id);
        }
        chain.reverse();

        // TREE に入れる Node。node は対象と子孫、manage は祖先も、support は無し。
        let mut in_scope: Vec<i64> = Vec::new();
        if !blind {
            match sci_type.as_str() {
                "manage" => {
                    in_scope.extend(chain.iter().copied());
                    in_scope.extend(subtree(target.id));
                }
                "support" => {}
                _ => in_scope.extend(subtree(target.id)),
            }
        }
        let set: HashSet<i64> = in_scope.into_iter().collect();
        let base = tree
            .nodes()
            .iter()
            .filter(|n| set.contains(&n.id))
            .map(|n| tree.depth(n.id))
            .min()
            .unwrap_or(0);
        let mut tree_lines: Vec<Line> = Vec::new();
        for n in tree.nodes().iter().filter(|n| set.contains(&n.id)) {
            let mut line = node_line(tree, n, tree.depth(n.id) - base);
            if n.id == target.id {
                line.full.push_str(" ← TARGET");
                line.first.push_str(" ← TARGET");
                let body = flat_text(&target.body);
                if !body.is_empty() {
                    line.body = Some(body);
                }
            }
            tree_lines.push(line);
        }

        // GUIDE: 対象か一番近い祖先の Q。
        let guide_q = std::iter::once(target.id)
            .chain(chain.iter().rev().copied())
            .filter_map(|id| tree.node(id))
            .find(|n| n.kind == Kind::Q);
        let guide = guide_q
            .and_then(|q| q.guide_key.as_deref())
            .and_then(|key| defs.guide(key))
            .map(|g| GuideText {
                title: format!("{}（{}）", g.name, g.key),
                question: defs.question_text(&g.key),
                target: g.target,
                child: g.child_candidates.clone(),
                extract: g.extract.clone(),
                pitfalls: g.pitfalls.clone(),
            });

        // SEARCH_HISTORY（blind では出さない）。support は対象だけ。
        let mut searches = Vec::new();
        if !blind {
            let mut ids = set.clone();
            ids.insert(target.id);
            for s in store::list_searches(conn, None)?.into_iter().filter(|s| s.node_id.is_some_and(|n| ids.contains(&n))) {
                let set_text = s.set_id.map_or_else(String::new, |id| format!("（セット {id}）"));
                searches.push(format!("- {} {}「{}」{} 件{set_text}", s.created_at, s.tool, s.query, s.hits));
                if searches.len() == 10 {
                    break;
                }
            }
        }

        // 論文。
        let mut outlines = Vec::new();
        let mut outline_gists = Vec::new();
        for id in keys.papers.iter().take(5) {
            match sai_paper_core::sections::outline(conn, &roots.data_dir, *id) {
                Ok(o) => {
                    let mut short = vec![format!("### #{} {}", o.paper_id, o.title)];
                    let mut gist = short.clone();
                    for s in &o.sections {
                        if s.kind == "references" {
                            short.push(format!("- {} {}（参考文献。読まない）", s.id, s.heading));
                            gist.push(short.last().cloned().unwrap_or_default());
                            continue;
                        }
                        let line = format!("- {} {}（{} 文）", s.id, s.heading, s.n_sentences);
                        let g = s.summary.as_deref().filter(|t| !t.is_empty()).unwrap_or(&s.lead);
                        let g: String = g.chars().take(80).collect();
                        gist.push(if g.is_empty() { line.clone() } else { format!("{line}— {g}") });
                        short.push(line);
                    }
                    outlines.push(short.join("\n"));
                    outline_gists.push(gist);
                }
                Err(e) => {
                    let note = format!("- #{id} 節の一覧を作れませんでした: {e}");
                    outlines.push(note.clone());
                    outline_gists.push(vec![note]);
                }
            }
        }
        let mut papers = Vec::new();
        if blind {
            for id in &keys.papers {
                let title: Option<String> = conn
                    .query_row("SELECT COALESCE(title, '') FROM papers WHERE id = ?1", [id], |r| r.get(0))
                    .optional()?;
                papers.push(format!("- #{id} {}", title.unwrap_or_default()));
            }
        }

        Ok(Parts {
            tree: (!blind && sci_type != "support").then_some(tree_lines),
            guide: if blind { None } else { guide },
            searches,
            ops: allowed_ops_text(roots, block),
            outlines,
            outline_gists,
            papers,
        })
    }

    fn render(&self, level: u8) -> String {
        let mut blocks: Vec<String> = Vec::new();

        if let Some(tree) = &self.tree {
            let mut lines = vec!["## TREE".to_string()];
            let mut hidden = 0;
            for l in tree {
                if l.kind == Kind::F && level >= 4 {
                    hidden += 1;
                    continue;
                }
                lines.push(if l.kind == Kind::F && level >= 2 { l.first.clone() } else { l.full.clone() });
                if let Some(body) = &l.body {
                    let shown: String = if level >= 3 { body.chars().take(300).collect() } else { body.clone() };
                    lines.push(format!("{}> {shown}", " ".repeat((l.indent + 1) * 2)));
                }
            }
            if hidden > 0 {
                lines.push(format!("（F は {hidden} 件あります。省きました）"));
            }
            blocks.push(lines.join("\n"));
        }
        if let Some(g) = &self.guide {
            let mut lines = vec!["## GUIDE".to_string(), format!("### {}", g.title), format!("- Q: {}", g.question)];
            lines.push(format!("- Objectivity の目安: {}", g.target));
            if level < 5 && !g.child.is_empty() {
                lines.push(format!("- Child Q: {}", g.child.join(" / ")));
            }
            if !g.extract.is_empty() {
                lines.push(format!("- Extract: {}", g.extract.join(" / ")));
            }
            if level < 1 && !g.pitfalls.is_empty() {
                lines.push(format!("- Pitfall: {}", g.pitfalls.join(" / ")));
            }
            blocks.push(lines.join("\n"));
        }
        if !self.searches.is_empty() {
            let n = if level >= 3 { 3 } else { 10 };
            let shown: Vec<&str> = self.searches.iter().take(n).map(String::as_str).collect();
            blocks.push(format!("## SEARCH_HISTORY\n{}", shown.join("\n")));
        }
        blocks.push(format!("## ALLOWED_OPS\n{}", self.ops));
        if level < 4 && !self.outlines.is_empty() {
            let src = if level <= 1 { self.outline_gists.iter().map(|g| g.join("\n")).collect::<Vec<_>>() } else { self.outlines.clone() };
            blocks.push(format!("## PAPER_OUTLINE\n{}", src.join("\n")));
        }
        if !self.papers.is_empty() {
            blocks.push(format!("## TARGET_PAPERS\n{}", self.papers.join("\n")));
        }
        blocks.join("\n\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sai_paper_core::database::Database;
    use super::super::defs;
    use std::fs;
    use std::path::PathBuf;

    struct Env {
        dir: PathBuf,
        roots: PaperRoots,
        db: Database,
        defs: Defs,
    }

    /// problem の型の木を作る。Q1 が一番上、Q2 が background、Q3 が background.goal。
    fn env(tag: &str) -> Env {
        let dir = std::env::temp_dir().join(format!("sai-rt-context-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let data_dir = dir.join(".sai/paper");
        fs::create_dir_all(&data_dir).unwrap();
        fs::create_dir_all(dir.join(".sai/blocks")).unwrap();
        let db = Database::open(&data_dir.join("papr.db")).unwrap();
        let defs = defs::load(&dir);
        defs::init_tree(db.connection(), &defs, "problem", "test").unwrap();
        Env { roots: PaperRoots { root: dir.clone(), data_dir }, dir, db, defs }
    }

    fn add(e: &Env, kind: Kind, parent: i64, text: &str, answer: Option<&str>) -> i64 {
        store::insert_node(
            e.db.connection(),
            &store::NewNode {
                kind, parent_id: Some(parent), guide_key: None, text, body: "", answer_kind: answer, cites: &[],
                paper_id: None, run_id: None, report_id: None, mlflow_ref: None, proposal_id: None, created_by: "user",
            },
        )
        .unwrap()
    }

    /// ブロック b1 の `sci_type` を決める。
    fn block(e: &Env, sci_type: &str) {
        fs::write(e.dir.join(".sai/blocks/b1.md"), format!("---\nsci_ops: add_o\nsci_type: {sci_type}\n---\n")).unwrap();
    }

    fn task(e: &Env, run: i64, front: &str) {
        let p = e.dir.join(format!(".sai/outputs/review/{run}"));
        fs::create_dir_all(&p).unwrap();
        fs::write(p.join("task.md"), format!("---\nrunner: pi\nblock: b1\n{front}---\n\n本文\n")).unwrap();
    }

    fn build_with(e: &Env, max: usize, reveal: bool) -> Result<String> {
        build(e.db.connection(), &e.roots, &e.defs, max, 1, reveal)
    }

    /// `## TREE` の節にある ID の集まり。
    fn tree_ids(text: &str) -> Vec<String> {
        let section = text.split("\n\n").find(|b| b.starts_with("## TREE")).unwrap_or("");
        let mut ids: Vec<String> = section
            .lines()
            .filter_map(|l| l.trim_start().strip_prefix("- "))
            .filter_map(|l| l.split(' ').next())
            .map(str::to_string)
            .collect();
        ids.sort();
        ids
    }

    fn sorted(mut v: Vec<String>) -> Vec<String> {
        v.sort();
        v
    }

    #[test]
    fn sections_come_in_order() {
        let e = env("order");
        block(&e, "node");
        task(&e, 1, "sci_node: Q2\nsci_purpose: 背景を決める\nsci_mode: purpose_first\n");
        let text = build_with(&e, 100_000, false).unwrap();
        // 出る節は TREE・GUIDE・ALLOWED_OPS だけ（消した節が混ざらない。順もこの順）。
        let heads: Vec<&str> = text.lines().filter_map(|l| l.strip_prefix("## ")).collect();
        assert_eq!(heads, vec!["TREE", "GUIDE", "ALLOWED_OPS"], "{text}");
        assert!(text.contains("[Objectivity"));
        let _ = fs::remove_dir_all(&e.dir);
    }

    #[test]
    fn sci_type_picks_tree_range() {
        let e = env("scopes");
        let o1 = add(&e, Kind::O, 2, "案 A", Some("answer"));
        let o2 = add(&e, Kind::O, 3, "案 B", Some("answer"));
        let f1 = add(&e, Kind::F, o2, "根拠", None);
        task(&e, 1, "sci_node: Q3\nsci_purpose: p\n");

        // node: 対象と子孫だけ。祖先 Q1・Q2 は出ない。
        block(&e, "node");
        let node_ids = tree_ids(&build_with(&e, 100_000, false).unwrap());
        assert_eq!(node_ids, sorted(vec!["Q3".into(), format!("O{o2}"), format!("F{f1}")]), "{node_ids:?}");

        // manage: 祖先 Q1・Q2 も出る。祖先の兄弟の O1 は出ない。
        block(&e, "manage");
        let manage_ids = tree_ids(&build_with(&e, 100_000, false).unwrap());
        for id in ["Q1", "Q2", "Q3"] {
            assert!(manage_ids.contains(&id.to_string()), "{manage_ids:?}");
        }
        assert!(!manage_ids.contains(&format!("O{o1}")), "{manage_ids:?}");

        // support: TREE を出さない。GUIDE と ALLOWED_OPS は出る。
        block(&e, "support");
        let support = build_with(&e, 100_000, false).unwrap();
        assert!(!support.contains("## TREE"), "{support}");
        assert!(support.contains("## ALLOWED_OPS"), "{support}");

        let _ = fs::remove_dir_all(&e.dir);
    }

    #[test]
    fn target_marker_and_body() {
        let e = env("target");
        block(&e, "node");
        e.db.connection().execute("UPDATE rt_nodes SET body = ?1 WHERE id = 3", ["現場の課題\n2 行目"]).unwrap();
        task(&e, 1, "sci_node: Q3\nsci_purpose: p\n");
        let text = build_with(&e, 100_000, false).unwrap();
        let target_line = text.lines().find(|l| l.contains("← TARGET")).unwrap_or("");
        assert!(target_line.starts_with("- Q3 "), "{target_line}");
        assert!(text.contains("> 現場の課題 2 行目"), "{text}");
        // 本文が無ければ `>` の行を出さない。
        e.db.connection().execute("UPDATE rt_nodes SET body = '' WHERE id = 3", []).unwrap();
        let none = build_with(&e, 100_000, false).unwrap();
        assert!(!none.contains("> "), "{none}");
        let _ = fs::remove_dir_all(&e.dir);
    }

    #[test]
    fn blind_hides_tree_and_shows_papers() {
        let e = env("blind");
        block(&e, "node");
        e.db.connection()
            .execute("INSERT INTO papers (id, title, published_at) VALUES (37, '論文 37', '2021-05-01')", [])
            .unwrap();
        task(&e, 1, "sci_node: Q2\nsci_purpose: p\nsci_mode: blind_first\nsci_papers: 37\n");
        let text = build_with(&e, 100_000, false).unwrap();
        for h in ["## TREE", "## GUIDE", "## SEARCH_HISTORY"] {
            assert!(!text.contains(h), "{h}\n{text}");
        }
        assert!(text.contains("## TARGET_PAPERS"), "{text}");
        assert!(text.contains("- #37 論文 37"), "{text}");

        // `--tree` で木を出し、TARGET_PAPERS は出さない。
        let revealed = build_with(&e, 100_000, true).unwrap();
        assert!(revealed.contains("## TREE"), "{revealed}");
        assert!(!revealed.contains("## TARGET_PAPERS"), "{revealed}");
        let _ = fs::remove_dir_all(&e.dir);
    }

    #[test]
    fn small_limit_cuts_by_level() {
        let e = env("levels");
        block(&e, "node");
        let o1 = add(&e, Kind::O, 3, "案 A", Some("answer"));
        add(&e, Kind::F, o1, "根拠 1 行目\n2 行目", None);
        task(&e, 1, "sci_node: Q3\nsci_purpose: p\n");
        let len = |max| build_with(&e, max, false).unwrap();
        let full = len(100_000);
        assert!(full.contains("- Pitfall:") && full.contains("- Extract:"));
        let l0 = full.chars().count();
        let cut1 = len(l0 - 1);
        assert!(!cut1.contains("- Pitfall:"));
        for max in [l0 - 1, 800, 400, 100] {
            let t = len(max);
            assert!(t.contains("## ALLOWED_OPS") || t.contains("字を省きました"), "{max}");
        }
        let small = len(800);
        assert!(small.contains("## ALLOWED_OPS") || small.contains("字を省きました"));
        let _ = fs::remove_dir_all(&e.dir);
    }

    #[test]
    fn allowed_ops_text_uses_sci_ops() {
        let e = env("ops");
        fs::write(e.dir.join(".sai/blocks/b1.md"), "---\nsci_ops: add_o, add_q\n---\n").unwrap();
        let t = allowed_ops_text(&e.roots, "b1");
        assert!(t.contains("- add_o（すぐ入る）"), "{t}");
        assert!(t.contains("- add_q（利用者が仕分けてから入る）"), "{t}");
        assert!(t.contains("必須: q（Q の ID）, text（1〜120 字）"), "{t}");
        assert!(t.contains("任意: body（0〜2000 字）, kind（answer / issue）"), "{t}");
        assert!(!t.contains("- add_f"));
        let none = allowed_ops_text(&e.roots, "nothing");
        for op in ["add_o", "add_f", "add_e"] {
            assert!(none.contains(&format!("- {op}（すぐ入る）")), "{op}");
        }
        assert!(!none.contains("add_q（"));
        let _ = fs::remove_dir_all(&e.dir);
    }

    #[test]
    fn unknown_node_is_not_found() {
        let e = env("errors");
        task(&e, 2, "sci_node: Q999\nsci_purpose: p\n");
        crate::link::forget(&e.dir);
        let r = build(e.db.connection(), &e.roots, &e.defs, 1000, 2, false);
        assert!(matches!(r, Err(RtError::NotFound(m)) if m == "Q999 はありません"));
        let _ = fs::remove_dir_all(&e.dir);
    }
}
