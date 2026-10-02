//! エージェントに渡す `# context` の Markdown（新しい木 Q・O・F・E 版）。
//!
//! 木の一部（範囲）だけを渡す。ファイルは作らず、文字列を返す（`# context` の見出しは呼び出し側が付ける）。
//! 長さは `max_chars` で制限し、はみ出したら level 0〜5 の順に 1 段ずつ削る。
//! 節の見出しの名前（`## CURRENT_ACTION` など）は sci-core が指すので変えない。

use std::collections::{HashMap, HashSet};

use rusqlite::{Connection, OptionalExtension};

use super::compute::{SCOPES, Tree};
use super::defs::Defs;
use super::ops::{self, Field, Risk, Ty};
use super::store::{self, NodeRow};
use super::{Kind, Result, RtError, RtKeys, action, fmt_id, task_keys};
use crate::paths::PaperRoots;

const NO_TREE: &str = "\
## CURRENT_ACTION
研究方針の木がまだありません。利用者に `sai-moon-paper sci init --type <型>`（または Web の Research-tree の画面の「始める」）を頼んでください。";

/// run の task.md から `# context` の Markdown を作る。`scope_override` があれば task.md の `sci_scope` より優先する。
///
/// # Errors
///
/// run が見つからない、対象の Node が無い、知らない範囲、DB のエラー。
pub fn build(
    conn: &Connection,
    roots: &PaperRoots,
    defs: &Defs,
    max_chars: usize,
    run_id: i64,
    scope_override: Option<&str>,
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
    let scope = resolve_scope(roots, &block, &keys.scope, scope_override)?;
    let tree = Tree::load(conn, roots, defs)?;
    let parts = Parts::collect(conn, roots, defs, &tree, run_id, &block, &keys, &scope)?;

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

/// 範囲を決める。`scope_override` → task.md の `sci_scope` → ブロック定義の `sci_scope` → `path`。
fn resolve_scope(
    roots: &PaperRoots,
    block: &str,
    keys_scope: &str,
    scope_override: Option<&str>,
) -> Result<String> {
    let pick = |s: &str| (!s.trim().is_empty()).then(|| s.trim().to_string());
    let scope = scope_override
        .and_then(pick)
        .or_else(|| pick(keys_scope))
        .or_else(|| {
            action::block_frontmatter(roots, block)?
                .into_iter()
                .find(|(key, _)| key == "sci_scope")
                .and_then(|(_, value)| pick(&value))
        })
        .unwrap_or_else(|| "path".to_string());
    if !SCOPES.contains(&scope.as_str()) {
        return Err(RtError::Invalid(format!("知らない SCOPE です: {scope}")));
    }
    Ok(scope)
}

// ---- ALLOWED_OPS ---------------------------------------------------------------------

fn field_text(f: &Field) -> String {
    if f.name == "reason" {
        return f.name.to_string();
    }
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
        "出し方: sai-moon-paper sci propose（標準入力に {\"proposals\": [...]}。MCP は sci_propose）。1 件でも違反があれば何も入りません。"
            .to_string(),
    );
    lines.join("\n")
}

// ---- 節の材料 -------------------------------------------------------------------------

/// Node の 1 行（`full` は全文、`first` は 1 行目だけ）。
struct Line {
    kind: Kind,
    full: String,
    first: String,
}

struct Parts {
    action: Vec<String>,
    progress: Vec<String>,
    target_head: String,
    target_body: String,
    target_kids: Vec<Line>,
    upstream: Option<Vec<Line>>,
    tree: Option<Vec<Line>>,
    assume: Vec<String>,
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
    let flat = |t: &str| t.split_whitespace().collect::<Vec<_>>().join(" ");
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
        full: make(flat(&n.text)),
        first: make(n.text.lines().next().unwrap_or("").trim().to_string()),
    }
}

impl Parts {
    #[allow(clippy::too_many_arguments)]
    fn collect(
        conn: &Connection,
        roots: &PaperRoots,
        defs: &Defs,
        tree: &Tree,
        run_id: i64,
        block: &str,
        keys: &RtKeys,
        scope: &str,
    ) -> Result<Parts> {
        let blind = scope == "blind";
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
        let kids_of = |id: i64| kids.get(&id).cloned().unwrap_or_default();
        let subtree = |id: i64| {
            let mut out = vec![id];
            let mut i = 0;
            while i < out.len() {
                out.extend(kids_of(out[i]));
                i += 1;
            }
            out
        };
        // 一番上から対象の親まで。
        let mut chain: Vec<i64> = Vec::new();
        let mut cur = target.parent_id;
        while let Some(id) = cur {
            chain.push(id);
            cur = tree.node(id).and_then(|n| n.parent_id);
        }
        chain.reverse();

        let target_text = format!("{} {}", fmt_id(target.kind, target.id), target.text);

        // CURRENT_ACTION
        let mut action = vec!["## CURRENT_ACTION".to_string(), format!("- run: {run_id}")];
        for (label, value) in [
            ("ブロック", block),
            ("目的", &keys.purpose),
            ("期待する成果", &keys.expect),
            ("読み方", &keys.mode),
            ("範囲", scope),
        ] {
            if !value.is_empty() {
                action.push(format!("- {label}: {value}"));
            }
        }
        action.push(format!("- 対象: {target_text}"));

        // PROGRESS
        let progress_info = tree.progress();
        let type_name = store::meta_get(conn, "type")?
            .map(|id| defs.type_def(&id).map_or(id.clone(), |t| t.name.clone()))
            .unwrap_or_default();
        let mut progress = vec!["## PROGRESS".to_string()];
        if !type_name.is_empty() {
            progress.push(format!("- 研究の型: {type_name}"));
        }
        progress.push(format!("- 進み具合: {}/{}", progress_info.answered, progress_info.total));
        if let Some(draft) = tree.seed_draft(&roots.data_dir) {
            progress.push(format!("- SEED の下書き: {}", draft.text.split_whitespace().collect::<Vec<_>>().join(" ")));
        }
        progress.push(format!("- DEBT: {} 件", action::debts(conn, roots)?.len()));
        progress.push(format!("- 仕分け待ち: {} 件", store::list_proposals(conn, Some("pending"))?.len()));

        // 範囲の Node。
        let mut in_scope: Vec<i64> = Vec::new();
        let path_set = |in_scope: &mut Vec<i64>| {
            for id in &chain {
                in_scope.push(*id);
                if tree.node(*id).is_some_and(|n| n.kind == Kind::Q) {
                    in_scope.extend(kids_of(*id).into_iter().filter(|k| tree.node(*k).is_some_and(|n| n.kind == Kind::O)));
                }
            }
            in_scope.extend(subtree(target.id));
        };
        match scope {
            "node" => {
                in_scope.push(target.id);
                in_scope.extend(kids_of(target.id));
            }
            "path" => path_set(&mut in_scope),
            "subtree" => in_scope.extend(subtree(target.id)),
            "siblings" => {
                path_set(&mut in_scope);
                for sib in target.parent_id.map(kids_of).unwrap_or_default() {
                    in_scope.push(sib);
                    in_scope.extend(kids_of(sib));
                }
            }
            "summary" => in_scope.extend(
                tree.nodes()
                    .iter()
                    .filter(|n| (n.kind == Kind::Q && tree.on_path(n.id)) || (n.kind == Kind::O && n.starred))
                    .map(|n| n.id),
            ),
            _ => {}
        }
        let set: HashSet<i64> = in_scope.iter().copied().collect();
        let base = tree.nodes().iter().filter(|n| set.contains(&n.id)).map(|n| tree.depth(n.id)).min().unwrap_or(0);
        let tree_lines: Vec<Line> = tree
            .nodes()
            .iter()
            .filter(|n| set.contains(&n.id))
            .map(|n| node_line(tree, n, tree.depth(n.id) - base))
            .collect();

        let target_kids: Vec<Line> =
            kids_of(target.id).iter().filter_map(|id| tree.node(*id)).map(|n| node_line(tree, n, 0)).collect();
        let upstream: Vec<Line> = chain
            .iter()
            .filter_map(|id| tree.node(*id))
            .enumerate()
            .map(|(i, n)| node_line(tree, n, i))
            .collect();

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

        // SEARCH_HISTORY（blind では出さない）。
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
            match crate::sections::outline(conn, &roots.data_dir, *id) {
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
            action,
            progress,
            target_head: target_text,
            target_body: target.body.clone(),
            target_kids,
            upstream: (!blind).then_some(upstream),
            tree: (!blind).then_some(tree_lines),
            assume: keys.assume.iter().map(|((k, id), t)| format!("- {}: {t}", fmt_id(*k, *id))).collect(),
            guide: if blind { None } else { guide },
            searches,
            ops: allowed_ops_text(roots, block),
            outlines,
            outline_gists,
            papers,
        })
    }

    fn render(&self, level: u8) -> String {
        let mut blocks: Vec<String> = vec![self.action.join("\n"), self.progress.join("\n")];

        let mut lines = vec!["## TARGET_NODE".to_string(), format!("- {}", self.target_head)];
        let body = self.target_body.trim();
        if !body.is_empty() {
            let shown: String = if level >= 3 { body.chars().take(300).collect() } else { body.to_string() };
            lines.push(shown);
        }
        lines.extend(self.target_kids.iter().map(|l| l.full.clone()));
        blocks.push(lines.join("\n"));

        if let Some(up) = &self.upstream {
            let mut lines = vec!["## UPSTREAM_PATH".to_string()];
            if up.is_empty() {
                lines.push("（なし。一番上です）".to_string());
            }
            lines.extend(up.iter().map(|l| l.full.clone()));
            blocks.push(lines.join("\n"));
        }
        if let Some(tree) = &self.tree {
            let mut lines = vec!["## TREE".to_string()];
            let mut hidden = 0;
            for l in tree {
                if l.kind == Kind::F && level >= 4 {
                    hidden += 1;
                } else {
                    lines.push(if l.kind == Kind::F && level >= 2 { l.first.clone() } else { l.full.clone() });
                }
            }
            if hidden > 0 {
                lines.push(format!("（F は {hidden} 件あります。省きました）"));
            }
            blocks.push(lines.join("\n"));
        }
        if !self.assume.is_empty() {
            blocks.push(format!("## ASSUMPTION\n{}", self.assume.join("\n")));
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
    use crate::database::Database;
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

    fn task(e: &Env, run: i64, front: &str) {
        let p = e.dir.join(format!(".sai/outputs/review/{run}"));
        fs::create_dir_all(&p).unwrap();
        fs::write(p.join("task.md"), format!("---\nrunner: pi\nblock: b1\n{front}---\n\n本文\n")).unwrap();
    }

    fn build_with(e: &Env, max: usize, scope: Option<&str>) -> Result<String> {
        build(e.db.connection(), &e.roots, &e.defs, max, 1, scope)
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

    #[test]
    fn sections_come_in_order() {
        let e = env("order");
        task(&e, 1, "sci_node: Q2\nsci_purpose: 背景を決める\nsci_scope: path\n");
        let text = build_with(&e, 100_000, None).unwrap();
        let heads = ["CURRENT_ACTION", "PROGRESS", "TARGET_NODE", "UPSTREAM_PATH", "TREE", "GUIDE", "ALLOWED_OPS"];
        let at: Vec<usize> = heads.iter().map(|h| text.find(&format!("## {h}")).unwrap_or_else(|| panic!("{h}"))).collect();
        assert!(at.windows(2).all(|w| w[0] < w[1]), "{text}");
        assert!(text.contains("- 対象: Q2 "));
        assert!(text.contains("[Objectivity"));
        let _ = fs::remove_dir_all(&e.dir);
    }

    #[test]
    fn scopes_pick_nodes() {
        let e = env("scopes");
        let o1 = add(&e, Kind::O, 2, "案 A", Some("answer"));
        let f1 = add(&e, Kind::F, o1, "根拠", None);
        store::set_starred(e.db.connection(), o1, true).unwrap();
        // Q3 の下にも O（path では出ない）。
        let o2 = add(&e, Kind::O, 3, "案 B", Some("answer"));
        task(&e, 1, "sci_node: Q3\nsci_purpose: p\n");
        let ids = |scope: &str| tree_ids(&build_with(&e, 100_000, Some(scope)).unwrap());
        let sorted = |mut v: Vec<String>| {
            v.sort();
            v
        };
        let f = format!("F{f1}");
        let (oa, ob) = (format!("O{o1}"), format!("O{o2}"));
        assert_eq!(ids("node"), sorted(vec!["Q3".into(), ob.clone()]));
        assert_eq!(ids("subtree"), sorted(vec!["Q3".into(), ob.clone()]));
        // path: 祖先 Q1・Q2 と、通った Q の O（O1）と、対象とその子孫。F は O の下なので出ない。
        assert_eq!(ids("path"), sorted(vec!["Q1".into(), "Q2".into(), oa.clone(), "Q3".into(), ob.clone()]));
        // siblings: path に兄弟 Q4 とその子が足される。
        let sib = ids("siblings");
        assert!(sib.contains(&"Q4".to_string()) && sib.contains(&"Q3".to_string()), "{sib:?}");
        // summary: ★ の道の Q と ★ の O だけ。
        let sum = ids("summary");
        assert!(sum.contains(&oa) && !sum.contains(&ob) && !sum.contains(&f), "{sum:?}");
        let blind = build_with(&e, 100_000, Some("blind")).unwrap();
        for h in ["## TREE", "## UPSTREAM_PATH", "## GUIDE"] {
            assert!(!blind.contains(h), "{h}");
        }
        assert!(blind.contains("## TARGET_NODE") && blind.contains("## ALLOWED_OPS"));
        let _ = fs::remove_dir_all(&e.dir);
    }

    #[test]
    fn small_limit_cuts_by_level() {
        let e = env("levels");
        let o1 = add(&e, Kind::O, 3, "案 A", Some("answer"));
        add(&e, Kind::F, o1, "根拠 1 行目\n2 行目", None);
        task(&e, 1, "sci_node: Q3\nsci_purpose: p\n");
        let len = |max| build_with(&e, max, None).unwrap();
        let full = len(100_000);
        assert!(full.contains("- Pitfall:") && full.contains("- Extract:"));
        let l0 = full.chars().count();
        let cut1 = len(l0 - 1);
        assert!(!cut1.contains("- Pitfall:"));
        for max in [l0 - 1, 800, 400, 100] {
            let t = len(max);
            assert!(t.contains("## CURRENT_ACTION"), "{max}");
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
        assert!(t.contains("任意: body（0〜2000 字）, kind（answer / issue）, reason"), "{t}");
        assert!(!t.contains("- add_f"));
        let none = allowed_ops_text(&e.roots, "nothing");
        for op in ["add_o", "add_f", "add_e"] {
            assert!(none.contains(&format!("- {op}（すぐ入る）")), "{op}");
        }
        assert!(!none.contains("add_q（"));
        let _ = fs::remove_dir_all(&e.dir);
    }

    #[test]
    fn unknown_scope_and_node() {
        let e = env("errors");
        task(&e, 1, "sci_node: Q2\nsci_purpose: p\n");
        match build_with(&e, 1000, Some("wide")) {
            Err(RtError::Invalid(m)) => assert_eq!(m, "知らない SCOPE です: wide"),
            other => panic!("{other:?}"),
        }
        task(&e, 2, "sci_node: Q999\nsci_purpose: p\n");
        let r = build(e.db.connection(), &e.roots, &e.defs, 1000, 2, None);
        assert!(matches!(r, Err(RtError::NotFound(m)) if m == "Q999 はありません"));
        let _ = fs::remove_dir_all(&e.dir);
    }
}
