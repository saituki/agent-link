//! エージェントに渡す `# context` の Markdown（新しい research-tree Q・A・F・E・S 版）。
//!
//! research-tree の一部（範囲）だけを渡す。ファイルは作らず、文字列を返す（`# context` の見出しは呼び出し側が付ける）。
//! 範囲はブロックの `sci_type`（node / manage / support）で決める。
//! 長さは `max_chars` で制限し、はみ出したら level 0〜5 の順に 1 段ずつ削る。
//! 節の見出しの名前（`## TREE` など）は sci-core が指すので変えない。

use std::collections::{HashMap, HashSet};

use rusqlite::{Connection, OptionalExtension};

use super::axes;
use super::compute::Tree;
use super::defs::Defs;
use super::ops::{self, Field, Risk, Ty};
use super::store::{self, NodeRow};
use super::{Kind, Result, RtError, RtKeys, action, fmt_id, task_keys};
use sai_paper_core::paths::PaperRoots;

const NO_TREE: &str = "\
## TREE
research-tree がまだありません。利用者に `sai-sci init --type <型>`（または Web の research-tree の画面の「始める」）を頼んでください。";

/// 親の無い古い SPROUT（seed を持たない S）の `## STUDY_SEED`。
const NO_SEED: &str = "（この SPROUT には seed（D）がありません）";

/// run の task.md から `# context` の Markdown を作る。`reveal` が真なら blind（`sci_mode: blind_first`）でも research-tree を出す。
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
        Ty::Id if f.name == "parent" => "Q か A の ID".to_string(),
        Ty::Id if f.name == "seed" => "seed（D）の ID".to_string(),
        Ty::Id => "Node の ID".to_string(),
        Ty::Text(min, max) => format!("{min}〜{max} 字"),
        Ty::Int => "整数".to_string(),
        Ty::Enum(values) => values.join(" / "),
        Ty::List(min, max) => format!("文字列の配列、{min}〜{max} 個"),
        Ty::Paths(min, max) => format!("A の ID の配列（オブジェクトも可）、{min}〜{max} 個"),
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
        // 利用者だけの op は agent の一覧に出さない。
        if spec.risk == Risk::UserOnly {
            continue;
        }
        let mark = if spec.risk == Risk::Approval { "（承認が要る）" } else { "" };
        lines.push(format!("- {}{mark}: {}。", spec.op, spec.about));
        for (label, required) in [("必須", true), ("任意", false)] {
            let items: Vec<String> =
                spec.fields.iter().filter(|f| f.required == required).map(field_text).collect();
            if !items.is_empty() {
                lines.push(format!("  {label}: {}", items.join(", ")));
            }
        }
    }
    lines.push(
        format!(
            "出し方: sai-sci op（標準入力に {{\"ops\": [...]}}、1 回は {} 件まで）。1 件でも違反があれば何も入りません。",
            ops::MAX_OPS
        ),
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
    /// 対象が SPROUT のときの PATH の行（`  PATH: Q2 > … > A15 …`）。
    paths: Vec<String>,
}

struct Parts {
    tree: Option<Vec<Line>>,
    /// (Q の文, Child Q, Extract, Pitfall)
    guide: Option<GuideText>,
    searches: Vec<String>,
    ops: String,
    outlines: Vec<String>,
    outline_gists: Vec<Vec<String>>,
    papers: Vec<String>,
    /// 対象の足す子の型（対象が無いときや未指定のときは空。C7）。
    child: String,
    /// 対象が SPROUT のときの親の SEED（D）。`## STUDY_SEED` の中身（無ければ `None`）。
    study_seed: Option<String>,
    /// 対象が木の全体のときの `## PROJECT`（型と対象。無ければ `None`）。
    project: Option<String>,
}

struct GuideText {
    title: String,
    question: String,
    child: Vec<String>,
    extract: Vec<String>,
    pitfalls: Vec<String>,
}

/// SPROUT の PATH（終点の O）ごとに、一番上の Q から終点の O までの 1 行を作る。行の頭に区分の札（`[固定]`・`[新規]`）を付ける。
fn path_lines(tree: &Tree, n: &NodeRow) -> Vec<String> {
    let roles = tree.roles_of(n.id);
    n.paths
        .iter()
        .enumerate()
        .map(|(i, o)| {
            let mut chain: Vec<&NodeRow> = Vec::new();
            let mut cur = Some(*o);
            while let Some(id) = cur {
                let Some(node) = tree.node(id) else { break };
                chain.push(node);
                cur = node.parent_id;
            }
            chain.reverse();
            let parts: Vec<String> = chain
                .iter()
                .map(|node| {
                    format!("{} {}", fmt_id(node.kind, node.id), flat_text(&node.text))
                })
                .collect();
            let tag = match roles.get(i).map(|r| r.role.as_str()) {
                Some("fixed") => "[固定] ",
                Some(_) => "[新規] ",
                None => "",
            };
            format!("  {tag}PATH: {}", parts.join(" > "))
        })
        .collect()
}

/// Q の深さ（一番上の Q を 1 とした Q の段。Q→A→Q は 2）。
fn q_depth(tree: &Tree, n: &NodeRow) -> usize {
    let mut depth = 1;
    let mut cur = n.parent_id;
    while let Some(p) = cur.and_then(|p| tree.node(p)) {
        depth += usize::from(p.kind == Kind::Q);
        cur = p.parent_id;
    }
    depth
}

/// SPROUT の行の区分（`[固定: task=A12 / 新規: method=A94 / 記載なし: data / 実験: E 3 件]`）。空の区分は出さない。
fn sprout_tail(tree: &Tree, n: &NodeRow) -> String {
    let roles = tree.roles_of(n.id);
    let seed = n.parent_id.and_then(|p| tree.node(p)).filter(|p| p.kind == Kind::D).map(|d| axes::parse_axes(&d.body));
    let mut parts: Vec<String> = Vec::new();
    let fixed: Vec<String> = roles
        .iter()
        .filter(|r| r.role == "fixed")
        .map(|r| format!("{}={}", r.axis.as_deref().unwrap_or("?"), r.a))
        .collect();
    if !fixed.is_empty() {
        parts.push(format!("固定: {}", fixed.join(", ")));
    }
    let novel: Vec<String> = roles
        .iter()
        .filter(|r| r.role == "novel")
        .map(|r| {
            let until = r.fixed_at.as_ref().map_or_else(String::new, |f| format!("（{f} まで固定）"));
            format!("{}={}{until}", r.axis.as_deref().unwrap_or("?"), r.a)
        })
        .collect();
    if !novel.is_empty() {
        parts.push(format!("新規: {}", novel.join(", ")));
    }
    if let Some(seed) = &seed {
        let none: Vec<&str> = axes::AXES.iter().map(|(key, _)| *key).filter(|key| seed.get(*key).is_some_and(|a| a.state == "none")).collect();
        if !none.is_empty() {
            parts.push(format!("記載なし: {}", none.join(", ")));
        }
    }
    let experiments = tree.count_under(n.id, Kind::E);
    if experiments > 0 {
        parts.push(format!("実験: E {experiments} 件"));
    }
    if parts.is_empty() { String::new() } else { format!(" [{}]", parts.join(" / ")) }
}

/// seed（D）の 6 軸の状態の表（`## STUDY_SEED` に足す。行は [`axes::AXES`] の順）。
fn seed_axes_table(body: &str) -> String {
    let parsed = axes::parse_axes(body);
    let mut lines = vec!["| 軸 | 状態 | 値・理由 | A |".to_string(), "|---|---|---|---|".to_string()];
    for (key, _) in axes::AXES {
        let axis = &parsed[key];
        let (label, text) = match axis.state.as_str() {
            "fixed" => ("決定", axis.value.as_deref()),
            "range" => ("範囲", axis.value.as_deref()),
            "novel" => ("空欄（新規性）", axis.reason.as_deref()),
            "none" => ("空欄（記載なし）", axis.reason.as_deref()),
            _ => ("空欄", None),
        };
        let cell = |text: Option<&str>| text.unwrap_or_default().replace('|', "\\|");
        lines.push(format!("| {key} | {label} | {} | {} |", cell(text), cell(axis.a.as_deref())));
    }
    lines.join("\n")
}

fn node_line(tree: &Tree, n: &NodeRow, indent: usize) -> Line {
    let pad = " ".repeat(indent * 2);
    let id = fmt_id(n.kind, n.id);
    let make = |text: String| {
        let tail = match n.kind {
            Kind::Q => format!(" [A {}・深さ {}]", tree.a_count(n.id), q_depth(tree, n)),
            Kind::A => format!(" [F {}]", tree.f_count_under(n.id)),
            Kind::F if !n.cites.is_empty() => format!(" [{}]", n.cites.join(", ")),
            Kind::E => {
                let mlflow = n.mlflow_ref.as_deref().filter(|m| !m.is_empty()).map_or_else(String::new, |m| format!(" [mlflow: {m}]"));
                let dataset = n.dataset_ref.as_deref().filter(|d| !d.is_empty()).map_or_else(String::new, |d| format!(" [dataset: {d}]"));
                format!("{mlflow}{dataset}")
            }
            Kind::S => {
                let range = tree.range(n.id).map_or_else(String::new, |r| format!(" [範囲: {}]", r.line()));
                format!("{}{range}", sprout_tail(tree, n))
            }
            Kind::F => String::new(),
            Kind::D => String::new(),
        };
        let star = if n.starred { " ★" } else { "" };
        let head = match n.kind {
            Kind::F => format!("{id}{star} [{}] {text}", n.stance.as_deref().unwrap_or("")),
            _ => format!("{id}{star} {text}"),
        };
        format!("{pad}- {head}{tail}")
    };
    Line {
        kind: n.kind,
        full: make(flat_text(&n.text)),
        first: make(n.text.lines().next().unwrap_or("").trim().to_string()),
        indent,
        body: None,
        paths: Vec::new(),
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
        // 読み方（`sci_mode: blind_first`）は research-tree を見ずに読む型。`--tree` で research-tree を出す。
        let blind = keys.mode == "blind_first" && !reveal;
        // ブロックの分類（node / manage / support）。無い・知らない値は一番狭い `node`。
        let sci_type = action::block_sci_type(roots, block)
            .filter(|t| action::SCI_TYPES.contains(&t.as_str()))
            .unwrap_or_else(|| "node".to_string());

        // 対象。無いとき（`sci_node` が空・無し）は木の全体。
        let target: Option<&NodeRow> = match keys.node {
            Some((kind, id)) => Some(
                tree.node(id)
                    .filter(|n| n.kind == kind)
                    .ok_or_else(|| RtError::NotFound(format!("{} はありません", fmt_id(kind, id))))?,
            ),
            None => None,
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
        // top_Q から対象の親まで（manage で祖先を出すのに使う）。
        let mut chain: Vec<i64> = Vec::new();
        let mut cur = target.and_then(|t| t.parent_id);
        while let Some(id) = cur {
            chain.push(id);
            cur = tree.node(id).and_then(|n| n.parent_id);
        }
        chain.reverse();

        // TREE に入れる Node。木の全体は全部、node は対象と子孫、manage は祖先も、support は無し。
        let mut in_scope: Vec<i64> = Vec::new();
        if !blind {
            match (target, sci_type.as_str()) {
                (_, "support") => {}
                (None, _) => in_scope.extend(tree.nodes().iter().map(|n| n.id)),
                // seed（D）を対象にした manage は木の全体（`lab-sprout` が root 全体から path を選ぶため）。
                (Some(t), "manage") if t.kind == Kind::D => in_scope.extend(tree.nodes().iter().map(|n| n.id)),
                (Some(t), "manage") => {
                    in_scope.extend(chain.iter().copied());
                    in_scope.extend(subtree(t.id));
                }
                (Some(t), _) => in_scope.extend(subtree(t.id)),
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
            if Some(n.id) == target.map(|t| t.id) {
                line.full.push_str(" ← TARGET");
                line.first.push_str(" ← TARGET");
                if n.kind == Kind::S {
                    line.paths = path_lines(tree, n);
                }
                let body = flat_text(&n.body);
                if !body.is_empty() {
                    line.body = Some(body);
                }
            }
            tree_lines.push(line);
        }

        // STUDY_SEED: 対象が SPROUT のとき、親の SEED（D）の題と本文を省かずに出す。親の無い古い SPROUT は断り書き。
        let study_seed = target.filter(|t| t.kind == Kind::S).map(|t| {
            match t.parent_id.and_then(|p| tree.node(p)).filter(|p| p.kind == Kind::D) {
                Some(d) => {
                    let head = format!("{} {}", fmt_id(Kind::D, d.id), flat_text(&d.text));
                    let table = seed_axes_table(&d.body);
                    if d.body.trim().is_empty() {
                        format!("{head}\n\n{table}")
                    } else {
                        format!("{head}\n{}\n\n{table}", d.body.trim_end())
                    }
                }
                None => NO_SEED.to_string(),
            }
        });
        // blind・support は research-tree を出さないので、STUDY_SEED も出さない。
        let study_seed = if blind || sci_type == "support" { None } else { study_seed };

        // GUIDE: 対象か一番近い祖先の Q。木の全体のときは出さない。
        let guide = target.and_then(|t| {
            let guide_q = std::iter::once(t.id)
                .chain(chain.iter().rev().copied())
                .filter_map(|id| tree.node(id))
                .find(|n| n.kind == Kind::Q);
            guide_q
                .and_then(|q| q.guide_key.as_deref())
                .and_then(|key| defs.guide(key))
                .map(|g| GuideText {
                    title: format!("{}（{}）", g.name, g.key),
                    question: defs.question_text(&g.key),
                    child: g.child_candidates.clone(),
                    extract: g.extract.clone(),
                    pitfalls: g.pitfalls.clone(),
                })
        });

        // SEARCH_HISTORY（blind では出さない）。support は対象だけ。
        let mut searches = Vec::new();
        if !blind {
            let mut ids = set.clone();
            if let Some(t) = target {
                ids.insert(t.id);
            }
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

        // 木の全体が対象のときは、TREE の前に型と対象を 1 節で出す（型の行は TREE に入れない）。
        let project = (target.is_none() && !blind && sci_type != "support").then(|| {
            format!("## PROJECT\n- 型: {}\n- 対象: 木の全体 ← TARGET", tree.type_name().unwrap_or_default())
        });
        Ok(Parts {
            child: target.map_or_else(String::new, |_| keys.child.clone()),
            project,
            tree: (!blind && sci_type != "support").then_some(tree_lines),
            study_seed,
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

        if let Some(project) = &self.project {
            blocks.push(project.clone());
        }
        if let Some(tree) = &self.tree {
            let mut lines = vec!["## TREE".to_string()];
            let mut hidden = 0;
            for l in tree {
                if l.kind == Kind::F && level >= 4 {
                    hidden += 1;
                    continue;
                }
                lines.push(if l.kind == Kind::F && level >= 2 { l.first.clone() } else { l.full.clone() });
                for path in &l.paths {
                    lines.push(path.clone());
                }
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
        if let Some(seed) = &self.study_seed {
            blocks.push(format!("## STUDY_SEED\n{seed}"));
        }
        if !self.child.is_empty() {
            blocks.push(format!("## TARGET\n- 足す子の型: {}", self.child));
        }
        if let Some(g) = &self.guide {
            let mut lines = vec!["## GUIDE".to_string(), format!("### {}", g.title), format!("- Q: {}", g.question)];
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

    /// RESEARCH_TYPE problem の research-tree を作る。top_Q は Q1（task）・Q2（target）・Q3（data）・Q4（method）・Q5（output）・Q6（metric）で、子は無い。
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

    fn add(e: &Env, kind: Kind, parent: i64, text: &str, stance: Option<&str>) -> i64 {
        store::insert_node(
            e.db.connection(),
            &store::NewNode {
                kind, parent_id: Some(parent), guide_key: None, text, body: "", stance, cites: &[],
                paper_id: None, run_id: None, report_id: None, mlflow_ref: None, dataset_ref: None,
                paths: &[], created_by: "user",
            },
        )
        .unwrap()
    }

    /// seed（D）を親 NULL で足す。`body` は本文（空ならそのまま空）。
    fn seed(e: &Env, text: &str, body: &str) -> i64 {
        store::insert_node(
            e.db.connection(),
            &store::NewNode {
                kind: Kind::D, parent_id: None, guide_key: None, text, body, stance: None, cites: &[],
                paper_id: None, run_id: None, report_id: None, mlflow_ref: None, dataset_ref: None,
                paths: &[], created_by: "user",
            },
        )
        .unwrap()
    }

    /// SPROUT（S）を足す。`parent` が `None` なら古い SPROUT（seed を持たない）。`o` は PATH の終点の A。
    fn sprout(e: &Env, parent: Option<i64>, o: i64, text: &str) -> i64 {
        store::insert_node(
            e.db.connection(),
            &store::NewNode {
                kind: Kind::S, parent_id: parent, guide_key: None, text, body: "", stance: None, cites: &[],
                paper_id: None, run_id: None, report_id: None, mlflow_ref: None, dataset_ref: None,
                paths: &[o], created_by: "user",
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
        assert!(!text.contains("Objectivity"));
        let _ = fs::remove_dir_all(&e.dir);
    }

    #[test]
    fn sci_type_picks_tree_range() {
        let e = env("scopes");
        // Q3（data）の下に子の Q（Q7）を足し、その下に A と F を置く。兄弟の Q2（target）には A を 1 つ。
        let o1 = add(&e, Kind::A, 2, "案 A", None);
        let q = add(&e, Kind::Q, 3, "下位の問い", None);
        let o2 = add(&e, Kind::A, q, "案 B", None);
        let f1 = add(&e, Kind::F, o2, "根拠", Some("support"));
        let node = fmt_id(Kind::Q, q);
        task(&e, 1, &format!("sci_node: {node}\nsci_purpose: p\n"));

        // node: 対象と子孫だけ。祖先 Q3 は出ない。
        block(&e, "node");
        let node_ids = tree_ids(&build_with(&e, 100_000, false).unwrap());
        assert_eq!(node_ids, sorted(vec![node.clone(), format!("A{o2}"), format!("F{f1}")]), "{node_ids:?}");

        // manage: 祖先 Q3 も出る。祖先でない兄弟 Q2 と、その A1 は出ない。
        block(&e, "manage");
        let manage_ids = tree_ids(&build_with(&e, 100_000, false).unwrap());
        for id in [fmt_id(Kind::Q, 3), node.clone()] {
            assert!(manage_ids.contains(&id), "{manage_ids:?}");
        }
        assert!(!manage_ids.contains(&format!("A{o1}")), "{manage_ids:?}");
        assert!(!manage_ids.contains(&fmt_id(Kind::Q, 2)), "{manage_ids:?}");

        // support: TREE を出さない。GUIDE と ALLOWED_OPS は出る。
        block(&e, "support");
        let support = build_with(&e, 100_000, false).unwrap();
        assert!(!support.contains("## TREE"), "{support}");
        assert!(support.contains("## ALLOWED_OPS"), "{support}");

        let _ = fs::remove_dir_all(&e.dir);
    }

    #[test]
    fn sprout_target_shows_paths() {
        let e = env("sprout");
        block(&e, "node");
        // Q3 の下に A を 1 つ。seed（D）の下の SPROUT の paths に入れ、E を SPROUT の下に置く。
        let o = add(&e, Kind::A, 3, "課題", None);
        let d = seed(&e, "研究", "");
        let s = sprout(&e, Some(d), o, "種");
        let ex = add(&e, Kind::E, s, "実験", None);
        task(&e, 1, &format!("sci_node: {}\nsci_purpose: p\n", fmt_id(Kind::S, s)));
        let text = build_with(&e, 100_000, false).unwrap();
        // S の行に paths が出る。
        assert!(text.contains(&format!("- {} 種 [新規: ", fmt_id(Kind::S, s))), "{text}");
        // S の行の直後に、top の Q から終点の O までの PATH が出る。
        assert!(text.contains("[新規] PATH: ") && text.contains(&format!("{} 課題", fmt_id(Kind::A, o))), "{text}");
        // 下の E も出る。
        assert!(text.contains(&format!("{}", fmt_id(Kind::E, ex))), "{text}");
        let _ = fs::remove_dir_all(&e.dir);
    }

    #[test]
    fn target_shows_child_kind() {
        let e = env("child");
        block(&e, "node");
        task(&e, 1, "sci_node: Q2\nsci_purpose: p\nsci_child: A\n");
        let text = build_with(&e, 100_000, false).unwrap();
        assert!(text.contains("## TARGET\n- 足す子の型: A"), "{text}");
        let _ = fs::remove_dir_all(&e.dir);
        // 足す子の型が無いときは節も行も出さない。GUIDE の見出し「### TARGET」は数えない。
        let e = env("child-none");
        block(&e, "node");
        task(&e, 1, "sci_node: Q2\nsci_purpose: p\n");
        let text = build_with(&e, 100_000, false).unwrap();
        assert!(!text.contains("\n## TARGET\n") && !text.contains("足す子の型"), "{text}");
        let _ = fs::remove_dir_all(&e.dir);
    }

    /// 対象が木の全体（`sci_node` 無し・古い `sci_node: SEED`）のとき、## PROJECT と全部の Node（top_Q から並ぶ）が入り、GUIDE は出ない。
    #[test]
    fn seed_target_shows_whole_tree() {
        let e = env("seed");
        block(&e, "manage");
        // Q3（background.problem）の下に A を 1 つ。seed（D）の下の SPROUT の paths に入れ、E を SPROUT の下に置く。
        let o = add(&e, Kind::A, 3, "課題", None);
        let d = seed(&e, "研究", "");
        let s = sprout(&e, Some(d), o, "種");
        let ex = add(&e, Kind::E, s, "実験", None);
        // 古い `sci_node: SEED` でも、無し（既定）でも同じ全体が出る。SEED の行は出ない。
        for front in ["sci_node: SEED\nsci_purpose: p\n", "sci_purpose: p\n"] {
            task(&e, 1, front);
            let text = build_with(&e, 100_000, false).unwrap();
            assert!(text.contains("## PROJECT\n- 型: PROBLEM_DRIVEN\n- 対象: 木の全体 ← TARGET\n"), "{text}");
            assert!(text.contains("## TREE\n- Q1 "), "{text}");
            assert!(!text.contains("SEED PROBLEM"), "{text}");
            // 軸の Q は親の無い行（字下げ 0）から並ぶ。
            assert!(text.contains("\n- Q4 ") && text.contains("\n- Q6 "), "{text}");
            assert!(text.contains(&format!("- {} 種 [新規: ", fmt_id(Kind::S, s))), "{text}");
            assert!(text.contains(&format!("{}", fmt_id(Kind::E, ex))), "{text}");
            assert!(!text.contains("## GUIDE"), "{text}");
        }
        let _ = fs::remove_dir_all(&e.dir);
    }

    /// 対象が SPROUT のとき、`## STUDY_SEED` が `## TREE` の直後に親 D の題と本文をそのまま出す。blind では出さない。
    #[test]
    fn sprout_target_shows_study_seed() {
        let e = env("study");
        block(&e, "node");
        let o = add(&e, Kind::A, 3, "課題", None);
        let body = "## 背景\n現場の要請\n\n## 対象（target）\n決定: 側頭葉てんかん患者";
        let d = seed(&e, "自分の研究", body);
        let s = sprout(&e, Some(d), o, "種");
        task(&e, 1, &format!("sci_node: {}\nsci_purpose: p\n", fmt_id(Kind::S, s)));
        let text = build_with(&e, 100_000, false).unwrap();
        let seed_at = text.find("## STUDY_SEED\n").unwrap_or_else(|| panic!("{text}"));
        assert!(text.find("## TREE").unwrap() < seed_at && seed_at < text.find("## ALLOWED_OPS").unwrap(), "{text}");
        assert!(text.contains(&format!("## STUDY_SEED\n{} 自分の研究\n{body}", fmt_id(Kind::D, d))), "{text}");

        // 親の無い古い SPROUT は断り書き。
        let old = sprout(&e, None, o, "古い種");
        task(&e, 1, &format!("sci_node: {}\nsci_purpose: p\n", fmt_id(Kind::S, old)));
        let text = build_with(&e, 100_000, false).unwrap();
        assert!(text.contains(&format!("## STUDY_SEED\n{NO_SEED}")), "{text}");

        // blind は research-tree を出さないので STUDY_SEED も出さない。
        task(&e, 1, &format!("sci_node: {}\nsci_purpose: p\nsci_mode: blind_first\n", fmt_id(Kind::S, s)));
        let text = build_with(&e, 100_000, false).unwrap();
        assert!(!text.contains("## STUDY_SEED"), "{text}");
        let _ = fs::remove_dir_all(&e.dir);
    }

    /// 対象が D の manage は TREE が木の全体になり、D の行に `← TARGET` と本文が付く。node は D とその下だけ。
    #[test]
    fn seed_manage_target_shows_whole_tree() {
        let e = env("seed-manage");
        block(&e, "manage");
        let o = add(&e, Kind::A, 3, "課題", None);
        let d = seed(&e, "研究", "本文の制約");
        let d2 = seed(&e, "別の研究", "");
        let s = sprout(&e, Some(d), o, "種");
        task(&e, 1, &format!("sci_node: {}\nsci_purpose: p\n", fmt_id(Kind::D, d)));
        let text = build_with(&e, 100_000, false).unwrap();
        assert!(text.contains(&format!("- {} 研究 ← TARGET", fmt_id(Kind::D, d))), "{text}");
        assert!(text.contains("  > 本文の制約"), "{text}");
        assert!(text.contains("\n- Q1 ") && text.contains("\n- Q4 "), "{text}");
        assert!(text.contains(&format!("- {} 別の研究", fmt_id(Kind::D, d2))), "{text}");
        assert!(text.contains(&format!("- {} 種 [新規: ", fmt_id(Kind::S, s))), "{text}");
        assert!(!text.contains("## STUDY_SEED"), "{text}");

        // sci_type が node なら D とその下の S だけ。
        block(&e, "node");
        let text = build_with(&e, 100_000, false).unwrap();
        assert!(!text.contains("\n- Q1 "), "{text}");
        assert!(!text.contains("別の研究"), "{text}");
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
        assert!(target_line.trim_start().starts_with("- Q3 "), "{target_line}");
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

        // `--tree` で research-tree を出し、TARGET_PAPERS は出さない。
        let revealed = build_with(&e, 100_000, true).unwrap();
        assert!(revealed.contains("## TREE"), "{revealed}");
        assert!(!revealed.contains("## TARGET_PAPERS"), "{revealed}");
        let _ = fs::remove_dir_all(&e.dir);
    }

    #[test]
    fn small_limit_cuts_by_level() {
        let e = env("levels");
        block(&e, "node");
        let o1 = add(&e, Kind::A, 3, "案 A", None);
        add(&e, Kind::F, o1, "根拠 1 行目\n2 行目", Some("support"));
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
        fs::write(e.dir.join(".sai/blocks/b1.md"), "---\nsci_ops: add_a, add_q\n---\n").unwrap();
        let t = allowed_ops_text(&e.roots, "b1");
        assert!(t.contains("- add_a: "), "{t}");
        assert!(t.contains("- add_q（承認が要る）: "), "{t}");
        assert!(t.contains("必須: q（Q の ID）, text（1〜120 字）"), "{t}");
        assert!(t.contains("任意: body（0〜2000 字）"), "{t}");
        assert!(!t.contains("- add_f"));
        let none = allowed_ops_text(&e.roots, "nothing");
        for op in ["add_a", "add_f", "add_e"] {
            assert!(none.contains(&format!("- {op}: ")), "{op}");
        }
        assert!(!none.contains("add_q（"));
        assert!(none.contains("- add_f: A の下に F（根拠）を足す。text は 1〜3 文。"), "{none}");
        assert!(none.contains("1 回は 20 件まで"), "{none}");
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

    /// TREE の Q は `[A n・深さ d]`、S は区分の札（`[新規: …]`・`[記載なし]`・`実験`）。STUDY_SEED の表に 6 軸が並ぶ。
    #[test]
    fn tree_shows_q_depth_and_sprout_roles() {
        let e = env("roles");
        block(&e, "node");
        let q_method = store::insert_node(
            e.db.connection(),
            &store::NewNode {
                kind: Kind::Q, parent_id: None, guide_key: Some("method"), text: "手法の問い", body: "", stance: None, cites: &[],
                paper_id: None, run_id: None, report_id: None, mlflow_ref: None, dataset_ref: None, paths: &[], created_by: "user",
            },
        )
        .unwrap();
        let a94 = add(&e, Kind::A, q_method, "Costa 型", None);
        let q60 = add(&e, Kind::Q, a94, "下位の問い", None);
        let a385 = add(&e, Kind::A, q60, "細かい手法", None);
        let body = format!(
            "## 解く問題（task）\n決定: 発作部位の特定\n## 対象（target）\n空欄\n## データ（data）\n空欄\n記載なし: 実測データを使わない\n## 手法（method）\n範囲: Costa 型を土台にする\nA: {}\n## 出力（output）\n空欄\n## 評価指標（metric）\n空欄\n",
            fmt_id(Kind::A, a94)
        );
        let d = seed(&e, "自分の研究", &body);
        let s = sprout(&e, Some(d), a385, "種");
        add(&e, Kind::E, s, "実験", None);
        task(&e, 1, &format!("sci_node: {}\nsci_purpose: p\n", fmt_id(Kind::S, s)));
        let text = build_with(&e, 100_000, false).unwrap();
        let roles = format!("[新規: method={}（{} まで固定） / 記載なし: data / 実験: E 1 件]", fmt_id(Kind::A, a385), fmt_id(Kind::A, a94));
        assert!(text.contains(&format!("- {} 種 {roles}", fmt_id(Kind::S, s))), "{text}");
        let path = format!("[新規] PATH: {} 手法の問い > {} Costa 型 > {} 下位の問い > {} 細かい手法", fmt_id(Kind::Q, q_method), fmt_id(Kind::A, a94), fmt_id(Kind::Q, q60), fmt_id(Kind::A, a385));
        assert!(text.contains(&path), "{text}");
        assert!(text.contains(&format!("| method | 範囲 | Costa 型を土台にする | {} |", fmt_id(Kind::A, a94))), "{text}");
        assert!(text.contains("| task | 決定 | 発作部位の特定 |  |"), "{text}");
        assert!(text.contains("| data | 空欄（記載なし） | 実測データを使わない |  |"), "{text}");
        // 対象が無いと木の全体。Q の行に [A n・深さ d]。
        task(&e, 1, "sci_purpose: p\n");
        let whole = build_with(&e, 100_000, false).unwrap();
        assert!(whole.contains("手法の問い [A 1・深さ 1]"), "{whole}");
        assert!(whole.contains("下位の問い [A 1・深さ 2]"), "{whole}");
        let _ = fs::remove_dir_all(&e.dir);
    }
}
