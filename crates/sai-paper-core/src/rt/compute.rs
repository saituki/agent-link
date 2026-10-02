//! 木の計算（Objectivity・Logicality・★ の道・進み具合・SEED の下書き・Next Action・注意）と、
//! `status` / `node` の JSON（C-JSON）。CLI・MCP・Web・TUI はこの結果を写すだけにする。

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;

use rusqlite::Connection;
use serde::Serialize;
use serde_json::{Value, json};

use super::defs::{Defs, GuideDef};
use super::store::{self, LogRow, LogicRow, NodeRow, ProposalRow, SeedRow};
use super::{Kind, Result, RtError, action, fmt_id, fmt_ref, ops, parse_id};
use crate::paths::PaperRoots;
use crate::runs::read_frontmatter;

/// `sci_scope` に書ける範囲の名前。
pub const SCOPES: [&str; 6] = ["node", "path", "subtree", "siblings", "summary", "blind"];

/// 古い形の木だけがあるときの注意。
const LEGACY_WARNING: &str =
    "古い形の木です。`sai-moon-paper sci init --type <型>` で作り直してください";

/// mind.md の見出しの別名（新しい名前, 旧い名前）。
const MIND_ALIASES: [(&str, &str); 7] = [
    ("ORIGIN", "始まり"),
    ("RESEARCH_TYPE", "研究の型"),
    ("GIVEN", "与えられていること"),
    ("CONSTRAINTS", "制約"),
    ("VALUE_AXES", "価値の軸"),
    ("INTERESTS", "興味がある / 興味が無い"),
    ("NOTES", "メモ"),
];

fn canonical_mind_heading(name: &str) -> &str {
    MIND_ALIASES.iter().find(|(_, old)| *old == name).map_or(name, |(new, _)| new)
}

// ---- JSON の型 ----------------------------------------------------------------------

/// Objectivity（目安の数に届いたか）。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Objectivity {
    pub count: i64,
    pub target: i64,
    pub met: bool,
}

/// Logicality（F どうしに矛盾が無いかの確かめ）。`value` は `ok` / `contradiction` / `unchecked`。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Logicality {
    pub value: String,
    pub note: String,
    pub findings: Vec<String>,
    pub checked_at: Option<String>,
}

/// 進み具合。
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize)]
pub struct Progress {
    pub answered: i64,
    pub total: i64,
}

/// SEED の下書きの 1 部分。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SeedPart {
    pub key: Option<String>,
    pub text: String,
    pub filled: bool,
}

/// SEED の下書き。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SeedDraft {
    pub text: String,
    pub parts: Vec<SeedPart>,
}

/// 直の子の種類ごとの数。
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize)]
pub struct Counts {
    pub q: i64,
    pub o: i64,
    pub f: i64,
    pub e: i64,
}

/// C-JSON の NodeRow。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct NodeJson {
    pub id: String,
    pub kind: String,
    pub parent: Option<String>,
    pub depth: usize,
    pub ord: i64,
    pub text: String,
    pub body: String,
    pub guide: Option<String>,
    pub answer_kind: Option<String>,
    pub starred: bool,
    pub on_path: bool,
    pub leaf: bool,
    pub objectivity: Option<Objectivity>,
    pub logicality: Option<Logicality>,
    pub counts: Counts,
    pub cites: Vec<String>,
    pub paper: Option<i64>,
    pub run: Option<i64>,
    pub mlflow: Option<String>,
    pub created_by: String,
    pub created_at: String,
}

/// Next Action 1 件。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct NextAction {
    pub kind: String,
    pub node: Option<String>,
    pub option: Option<String>,
    pub block: Option<String>,
    pub purpose: String,
    pub expect: String,
    pub why: String,
    pub card: Option<Value>,
    pub fix: Option<Value>,
    /// node_detail で「この Node が対象」を選ぶための Node（JSON には出さない）。
    #[serde(skip)]
    targets: Vec<i64>,
}

/// 決定の記録 1 件。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Decision {
    pub node: String,
    pub node_text: String,
    pub action: String,
    pub detail: String,
    pub reason: String,
    pub actor: String,
    pub at: String,
}

/// 種の JSON。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SeedJson {
    pub id: String,
    pub title: String,
    pub sentence: String,
    pub path: Vec<String>,
    pub run: Option<i64>,
    pub created_at: String,
}

/// 走っている run。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RunningJson {
    pub run_id: i64,
    pub node: Option<String>,
    pub block: String,
    pub owner: String,
}

/// 型の id と名前。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TypeJson {
    pub id: String,
    pub name: String,
}

/// `status` 全体の JSON。
#[derive(Debug, Clone, Serialize)]
pub struct Status {
    pub initialized: bool,
    pub legacy: bool,
    #[serde(rename = "type")]
    pub type_info: Option<TypeJson>,
    pub progress: Progress,
    pub nodes: Vec<NodeJson>,
    pub seeds: Vec<SeedJson>,
    pub seed_draft: Option<SeedDraft>,
    pub next: Vec<NextAction>,
    pub debts: Vec<action::Debt>,
    pub debt_count: usize,
    pub pending_proposals: usize,
    pub decisions: Vec<Decision>,
    pub running: Vec<RunningJson>,
    pub warnings: Vec<String>,
}

/// `node_detail.path` の 1 要素。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PathItem {
    pub id: String,
    pub kind: String,
    pub text: String,
}

/// `node_detail.guide`。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct GuideJson {
    pub key: String,
    pub name: String,
    pub explain: String,
    pub question: String,
    pub child_candidates: Vec<String>,
    pub extract: Vec<String>,
    pub pitfalls: Vec<String>,
    pub target: i64,
}

/// `node_detail.logic` の 1 要素。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LogicJson {
    pub value: String,
    pub note: String,
    pub findings: Vec<String>,
    pub f_count: i64,
    pub run: Option<i64>,
    pub created_at: String,
}

/// `node_detail.actions` の 1 要素。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ActionJson {
    pub run_id: i64,
    pub block: String,
    pub owner: String,
    pub purpose: String,
    pub occurred_at: String,
    pub debt: bool,
    pub debt_messages: Vec<String>,
}

/// `node_detail.pending` の 1 要素。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PendingJson {
    pub id: String,
    pub op: String,
    pub payload: Value,
    pub reason: String,
    pub run: Option<i64>,
    pub created_at: String,
}

/// `node_detail.searches` の 1 要素。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SearchJson {
    pub id: i64,
    pub run: Option<i64>,
    pub tool: String,
    pub query: String,
    pub hits: i64,
    pub set_id: Option<i64>,
    pub created_at: String,
}

/// `node` の JSON。
#[derive(Debug, Clone, Serialize)]
pub struct NodeDetail {
    pub node: NodeJson,
    pub path: Vec<PathItem>,
    pub children: Vec<NodeJson>,
    pub subtree: Vec<NodeJson>,
    pub guide: Option<GuideJson>,
    pub logic: Vec<LogicJson>,
    pub decisions: Vec<Decision>,
    pub actions: Vec<ActionJson>,
    pub pending: Vec<PendingJson>,
    pub next: Vec<NextAction>,
    pub searches: Vec<SearchJson>,
}

// ---- 木 -------------------------------------------------------------------------------

/// 木を 1 回読んで、計算を使い回す。
pub struct Tree {
    defs: Defs,
    type_id: Option<String>,
    nodes: Vec<NodeRow>,
    index: HashMap<i64, usize>,
    kids: HashMap<i64, Vec<i64>>,
    depths: HashMap<i64, usize>,
    path: HashSet<i64>,
    f_under: HashMap<i64, i64>,
    objectivity: HashMap<i64, Objectivity>,
    /// reverted でない確かめの記録（新しい順）。
    logic_rows: Vec<LogicRow>,
    logicality: HashMap<i64, Logicality>,
    seeds: Vec<SeedRow>,
    log: Vec<LogRow>,
    pending: Vec<ProposalRow>,
    /// 消した Node も含む id → (kind, 文)。決定の記録の表示に使う。
    labels: HashMap<i64, (Kind, String)>,
}

fn f_ids(ids: &[i64]) -> Vec<String> {
    ids.iter().map(|id| fmt_id(Kind::F, *id)).collect()
}

impl Tree {
    /// 木を読む。
    pub fn load(conn: &Connection, _roots: &PaperRoots, defs: &Defs) -> Result<Tree> {
        let all = store::list_nodes(conn, true)?;
        let labels: HashMap<i64, (Kind, String)> =
            all.iter().map(|n| (n.id, (n.kind, n.text.clone()))).collect();
        // `(parent_id, ord, id)` の順なので、親ごとに積めば兄弟は ord 順。
        let mut by_parent: HashMap<Option<i64>, Vec<NodeRow>> = HashMap::new();
        for n in all.into_iter().filter(|n| n.removed_at.is_none()) {
            by_parent.entry(n.parent_id).or_default().push(n);
        }

        let mut nodes: Vec<NodeRow> = Vec::new();
        let mut depths = HashMap::new();
        let mut path = HashSet::new();
        let mut kids: HashMap<i64, Vec<i64>> = HashMap::new();
        let roots_q: Vec<NodeRow> = by_parent
            .remove(&None)
            .unwrap_or_default()
            .into_iter()
            .filter(|n| n.kind == Kind::Q)
            .collect();
        // 一番上の Q は最小の id のもの 1 つだけ（`store::root` と同じ）。
        let mut stack: Vec<(NodeRow, usize)> =
            roots_q.into_iter().take(1).map(|n| (n, 0)).collect();
        while let Some((n, depth)) = stack.pop() {
            let on = match n.parent_id {
                None => true,
                Some(p) if path.contains(&p) => {
                    let parent_kind = labels[&p].0;
                    match (parent_kind, n.kind) {
                        (Kind::Q, Kind::Q) => true,
                        (Kind::Q, Kind::O) => n.starred,
                        (Kind::O, _) => true,
                        (Kind::E, Kind::F | Kind::E) => true,
                        _ => false,
                    }
                }
                Some(_) => false,
            };
            if on {
                path.insert(n.id);
            }
            depths.insert(n.id, depth);
            let children = by_parent.remove(&Some(n.id)).unwrap_or_default();
            kids.insert(n.id, children.iter().map(|c| c.id).collect());
            for c in children.into_iter().rev() {
                stack.push((c, depth + 1));
            }
            nodes.push(n);
        }
        let index: HashMap<i64, usize> = nodes.iter().enumerate().map(|(i, n)| (n.id, i)).collect();

        // 子孫の F の数（子が親より後ろに並ぶので、逆順に積む）。
        let mut f_under: HashMap<i64, i64> = HashMap::new();
        for n in nodes.iter().rev() {
            let total = kids[&n.id]
                .iter()
                .map(|k| i64::from(nodes[index[k]].kind == Kind::F) + f_under[k])
                .sum();
            f_under.insert(n.id, total);
        }

        let mut tree = Tree {
            defs: defs.clone(),
            type_id: store::meta_get(conn, "type")?,
            nodes,
            index,
            kids,
            depths,
            path,
            f_under,
            objectivity: HashMap::new(),
            logic_rows: Vec::new(),
            logicality: HashMap::new(),
            seeds: store::list_seeds(conn)?,
            log: store::list_log(conn, None)?,
            pending: store::list_proposals(conn, Some("pending"))?,
            labels,
        };

        // Objectivity（子の Q の判定を使うので逆順）。
        let order: Vec<i64> =
            tree.nodes.iter().rev().filter(|n| n.kind == Kind::Q).map(|n| n.id).collect();
        for id in order {
            let value = tree.compute_objectivity(id);
            tree.objectivity.insert(id, value);
        }

        // Logicality。
        let reverted: HashSet<i64> = store::list_proposals(conn, Some("reverted"))?
            .into_iter()
            .map(|p| p.id)
            .collect();
        tree.logic_rows = store::list_logic(conn, None)?
            .into_iter()
            .filter(|r| !r.proposal_id.is_some_and(|p| reverted.contains(&p)))
            .collect();
        for row in &tree.logic_rows {
            if tree.index.contains_key(&row.q_id) && !tree.logicality.contains_key(&row.q_id) {
                let fresh = tree.f_under[&row.q_id] == row.f_count;
                tree.logicality.insert(
                    row.q_id,
                    Logicality {
                        value: if fresh { row.value.clone() } else { "unchecked".into() },
                        note: row.note.clone(),
                        findings: f_ids(&row.findings),
                        checked_at: Some(row.created_at.clone()),
                    },
                );
            }
        }
        Ok(tree)
    }

    fn kid_nodes(&self, id: i64) -> impl Iterator<Item = &NodeRow> {
        self.kids.get(&id).into_iter().flatten().map(|k| &self.nodes[self.index[k]])
    }

    /// O（または E）に付いた F の数。O・E の下だけをたどり、Q の下には入らない。
    fn f_direct(&self, id: i64) -> i64 {
        self.kid_nodes(id)
            .map(|k| match k.kind {
                Kind::F => 1,
                Kind::E => self.f_direct(k.id),
                _ => 0,
            })
            .sum()
    }

    fn compute_objectivity(&self, id: i64) -> Objectivity {
        let q_kids: Vec<&NodeRow> = self.kid_nodes(id).filter(|k| k.kind == Kind::Q).collect();
        if q_kids.is_empty() {
            let node = &self.nodes[self.index[&id]];
            let count = self.kid_nodes(id).filter(|k| k.kind == Kind::O).map(|o| self.f_direct(o.id)).sum();
            let target = node
                .target
                .or_else(|| node.guide_key.as_deref().and_then(|k| self.defs.guide(k)).map(|g| g.target))
                .unwrap_or(2);
            Objectivity { count, target, met: count >= target }
        } else {
            let count = q_kids.iter().filter(|k| self.objectivity[&k.id].met).count() as i64;
            let target = q_kids.len() as i64;
            Objectivity { count, target, met: count == target }
        }
    }

    /// 生きた Node（木の順）。
    #[must_use]
    pub fn nodes(&self) -> &[NodeRow] {
        &self.nodes
    }

    /// 生きた Node 1 つ。
    #[must_use]
    pub fn node(&self, id: i64) -> Option<&NodeRow> {
        self.index.get(&id).map(|i| &self.nodes[*i])
    }

    /// 一番上の Q が 0。
    #[must_use]
    pub fn depth(&self, id: i64) -> usize {
        self.depths.get(&id).copied().unwrap_or(0)
    }

    /// 生きた子の Q を持たない Q か。
    #[must_use]
    pub fn is_leaf_q(&self, id: i64) -> bool {
        self.node(id).is_some_and(|n| n.kind == Kind::Q)
            && !self.kid_nodes(id).any(|k| k.kind == Kind::Q)
    }

    /// ★ の道の上か。
    #[must_use]
    pub fn on_path(&self, id: i64) -> bool {
        self.path.contains(&id)
    }

    /// Q の Objectivity。
    #[must_use]
    pub fn objectivity(&self, id: i64) -> Option<Objectivity> {
        self.objectivity.get(&id).cloned()
    }

    /// Q の Logicality。
    #[must_use]
    pub fn logicality(&self, id: i64) -> Option<Logicality> {
        self.node(id).filter(|n| n.kind == Kind::Q)?;
        Some(self.logicality.get(&id).cloned().unwrap_or(Logicality {
            value: "unchecked".into(),
            note: String::new(),
            findings: Vec::new(),
            checked_at: None,
        }))
    }

    /// 子孫の生きた F の数。
    #[must_use]
    pub fn f_count_under(&self, id: i64) -> i64 {
        self.f_under.get(&id).copied().unwrap_or(0)
    }

    /// 進み具合。
    #[must_use]
    pub fn progress(&self) -> Progress {
        let mut p = Progress::default();
        for n in &self.nodes {
            if n.kind == Kind::Q && self.on_path(n.id) && self.is_leaf_q(n.id) {
                p.total += 1;
                if self.kid_nodes(n.id).any(|k| k.kind == Kind::O && k.starred) {
                    p.answered += 1;
                }
            }
        }
        p
    }

    fn top_q(&self) -> Option<&NodeRow> {
        self.nodes.first()
    }

    /// SEED の下書き（型の `seed` の `{key}` と `{mind:見出し}` を埋める）。
    #[must_use]
    pub fn seed_draft(&self, data_dir: &Path) -> Option<SeedDraft> {
        let seed = &self.defs.type_def(self.type_id.as_deref()?)?.seed;
        if seed.trim().is_empty() {
            return None;
        }
        let sections = mind_sections(data_dir);
        let mut parts: Vec<SeedPart> = Vec::new();
        let literal = |parts: &mut Vec<SeedPart>, text: &str| {
            if !text.is_empty() {
                parts.push(SeedPart { key: None, text: text.to_string(), filled: true });
            }
        };
        let mut rest = seed.as_str();
        loop {
            let Some(open) = rest.find('{') else {
                literal(&mut parts, rest);
                break;
            };
            let Some(close) = rest[open..].find('}').map(|i| i + open) else {
                literal(&mut parts, rest);
                break;
            };
            literal(&mut parts, &rest[..open]);
            let inner = &rest[open + 1..close];
            let (key, text) = if let Some(heading) = inner.strip_prefix("mind:") {
                let heading = canonical_mind_heading(heading);
                let body = sections.iter().find(|(name, _)| name == heading).map(|(_, b)| b.clone());
                (format!("mind:{heading}"), body)
            } else {
                let titles: Vec<&str> = self
                    .nodes
                    .iter()
                    .filter(|n| n.kind == Kind::Q && self.on_path(n.id) && n.guide_key.as_deref() == Some(inner))
                    .flat_map(|q| self.kid_nodes(q.id))
                    .filter(|o| o.kind == Kind::O && o.starred)
                    .map(|o| o.text.as_str())
                    .collect();
                (inner.to_string(), (!titles.is_empty()).then(|| titles.join("・")))
            };
            parts.push(match text {
                Some(text) => SeedPart { key: Some(key), text, filled: true },
                None => SeedPart { key: Some(key), text: format!("{{{inner}}}"), filled: false },
            });
            rest = &rest[close + 1..];
        }
        let text = parts.iter().map(|p| p.text.as_str()).collect();
        Some(SeedDraft { text, parts })
    }

    /// C-JSON の NodeRow。`id` は生きた Node であること。
    #[must_use]
    pub fn row(&self, id: i64) -> NodeJson {
        let n = &self.nodes[self.index[&id]];
        let mut counts = Counts::default();
        for k in self.kid_nodes(id) {
            match k.kind {
                Kind::Q => counts.q += 1,
                Kind::O => counts.o += 1,
                Kind::F => counts.f += 1,
                Kind::E => counts.e += 1,
            }
        }
        let is_q = n.kind == Kind::Q;
        NodeJson {
            id: fmt_id(n.kind, n.id),
            kind: n.kind.as_str().to_string(),
            parent: n.parent_id.and_then(|p| self.labels.get(&p).map(|(k, _)| fmt_id(*k, p))),
            depth: self.depth(id),
            ord: n.ord,
            text: n.text.clone(),
            body: n.body.clone(),
            guide: if is_q { n.guide_key.clone() } else { None },
            answer_kind: if n.kind == Kind::O { n.answer_kind.clone() } else { None },
            starred: n.kind == Kind::O && n.starred,
            on_path: self.on_path(id),
            leaf: is_q && self.is_leaf_q(id),
            objectivity: self.objectivity(id),
            logicality: self.logicality(id),
            counts,
            cites: n.cites.clone(),
            paper: n.paper_id,
            run: n.run_id,
            mlflow: n.mlflow_ref.clone(),
            created_by: n.created_by.clone(),
            created_at: n.created_at.clone(),
        }
    }

    fn label(&self, id: i64) -> (String, String) {
        self.labels
            .get(&id)
            .map_or_else(|| (format!("N{id}"), String::new()), |(k, t)| (fmt_id(*k, id), t.clone()))
    }

    fn decision(&self, row: &LogRow) -> Decision {
        let (node, node_text) = self.label(row.node_id);
        Decision {
            node,
            node_text,
            action: row.action.clone(),
            detail: row.detail.clone(),
            reason: row.reason.clone(),
            actor: row.actor.clone(),
            at: row.created_at.clone(),
        }
    }
}

// ---- mind.md --------------------------------------------------------------------------

/// mind.md の `## 見出し` ごとの本文（コメントと空行を除く。空の節は出さない）。
#[must_use]
pub fn mind_sections(data_dir: &Path) -> Vec<(String, String)> {
    let Ok(text) = fs::read_to_string(crate::paths::mind_path(data_dir)) else {
        return Vec::new();
    };
    let mut sections: Vec<(String, Vec<String>)> = Vec::new();
    for line in text.lines() {
        if let Some(name) = line.trim().strip_prefix("## ") {
            sections.push((name.trim().to_string(), Vec::new()));
        } else if let Some((_, lines)) = sections.last_mut() {
            // 最初の見出しより前の前書きは捨てる。
            lines.push(line.to_string());
        }
    }
    sections
        .into_iter()
        .map(|(name, lines)| (canonical_mind_heading(&name).to_string(), strip_comments(&lines)))
        .filter(|(_, body)| !body.is_empty())
        .collect()
}

/// 行の列から `<!-- … -->` と空行を除いて 1 本にする。
fn strip_comments(lines: &[String]) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut in_comment = false;
    for line in lines {
        let mut piece = String::new();
        let mut rest = line.as_str();
        loop {
            if in_comment {
                match rest.find("-->") {
                    Some(i) => {
                        rest = &rest[i + 3..];
                        in_comment = false;
                    }
                    None => break,
                }
            } else if let Some(i) = rest.find("<!--") {
                piece.push_str(&rest[..i]);
                rest = &rest[i + 4..];
                in_comment = true;
            } else {
                piece.push_str(rest);
                break;
            }
        }
        if !piece.trim().is_empty() {
            out.push(piece.trim_end().to_string());
        }
    }
    out.join("\n").trim().to_string()
}

// ---- Next Action ----------------------------------------------------------------------

fn debt_action(debt: &action::Debt) -> NextAction {
    let fix = debt.fix.clone();
    let card = (fix.get("kind").and_then(Value::as_str) == Some("card"))
        .then(|| fix.get("card").cloned())
        .flatten();
    let block = card
        .as_ref()
        .and_then(|c| c.get("block").and_then(Value::as_str))
        .map_or_else(|| debt.block.clone(), str::to_string);
    let targets = debt.node.as_deref().and_then(|t| parse_id(t).ok()).map(|(_, id)| id).into_iter().collect();
    NextAction {
        kind: "debt".into(),
        node: debt.node.clone(),
        option: None,
        block: Some(block),
        purpose: debt.message.clone(),
        expect: String::new(),
        why: "DEBT".into(),
        card,
        fix: Some(fix),
        targets,
    }
}

#[allow(clippy::too_many_arguments)]
fn card_action(
    node: (Kind, i64),
    option: Option<i64>,
    block: &str,
    purpose: String,
    expect: String,
    why: &str,
    q: i64,
) -> NextAction {
    let card_node = fmt_id(if option.is_some() { Kind::O } else { node.0 }, option.unwrap_or(node.1));
    NextAction {
        kind: "card".into(),
        node: Some(card_node.clone()),
        option: option.map(|o| fmt_id(Kind::O, o)),
        block: Some(block.into()),
        card: Some(json!({
            "node": card_node, "block": block, "purpose": purpose, "expect": expect, "runner": "pi",
        })),
        purpose,
        expect,
        why: why.into(),
        fix: None,
        targets: [Some(q), option].into_iter().flatten().collect(),
    }
}

impl Tree {
    /// 道の上の葉の Q 1 つに出す Next Action（順 2〜6 の最初に当てはまる 1 つ）。
    fn q_action(&self, q: &NodeRow) -> Option<(u8, NextAction)> {
        if !self.is_leaf_q(q.id) {
            return None;
        }
        let me = (Kind::Q, q.id);
        let options: Vec<&NodeRow> = self.kid_nodes(q.id).filter(|k| k.kind == Kind::O).collect();
        let candidates = q
            .guide_key
            .as_deref()
            .and_then(|k| self.defs.guide(k))
            .is_some_and(|g: &GuideDef| !g.child_candidates.is_empty());
        if candidates && options.is_empty() {
            return Some((2, card_action(me, None, "sci-split", format!("「{}」を子の Q に分ける", q.text), "子の Q の案を 2 つ以上".into(), "NO_CHILD_Q", q.id)));
        }
        if options.is_empty() {
            return Some((3, card_action(me, None, "sci-ideate", format!("「{}」の答えの候補（O）を出す", q.text), "O を 2 つ以上".into(), "NO_OPTION", q.id)));
        }
        let obj = self.objectivity(q.id)?;
        if !obj.met {
            let o = options.iter().min_by_key(|o| (self.f_direct(o.id), !o.starred, o.ord))?;
            return Some((4, card_action(me, Some(o.id), "sci-search", format!("「{}」の F を探す", o.text), format!("F を {} つ以上", obj.target - obj.count), "FEW_FINDINGS", q.id)));
        }
        if self.f_count_under(q.id) >= 2 {
            let value = self.logicality(q.id)?.value;
            if value != "ok" {
                let why = if value == "contradiction" { "CONTRADICTION" } else { "UNCHECKED" };
                return Some((5, card_action(me, None, "sci-debate", format!("「{}」の F どうしに矛盾が無いか確かめる", q.text), "Logicality を記録する".into(), why, q.id)));
            }
        }
        if !options.iter().any(|o| o.starred) {
            return Some((6, NextAction {
                kind: "user".into(),
                node: Some(fmt_id(Kind::Q, q.id)),
                option: None,
                block: None,
                purpose: format!("「{}」の ★ を選ぶ（あなたが選ぶ）", q.text),
                expect: "★ を 1 つ以上".into(),
                why: "NO_STAR".into(),
                card: None,
                fix: None,
                targets: vec![q.id],
            }));
        }
        None
    }
}

/// Next Action（DEBT → 順の番号 → 木の順）を `limit` 件まで。
#[must_use]
pub fn next_actions(tree: &Tree, debts: &[action::Debt], limit: usize) -> Vec<NextAction> {
    let mut ranked: Vec<(u8, usize, NextAction)> = Vec::new();
    for (pos, n) in tree.nodes.iter().enumerate() {
        if n.kind != Kind::Q || !tree.on_path(n.id) {
            continue;
        }
        if let Some((rank, a)) = tree.q_action(n) {
            ranked.push((rank, pos, a));
        }
        if tree.is_leaf_q(n.id) {
            for o in tree.kid_nodes(n.id) {
                let issue = o.kind == Kind::O && o.answer_kind.as_deref() == Some("issue");
                if issue && !tree.kid_nodes(o.id).any(|k| k.kind == Kind::E) {
                    let a = card_action((Kind::O, o.id), Some(o.id), "sci-exp-sprout", format!("「{}」を解く実験を作る", o.text), "実験仕様書と最初の E".into(), "NO_EXPERIMENT", n.id);
                    ranked.push((7, tree.index[&o.id], a));
                }
            }
        }
    }
    let progress = tree.progress();
    if let Some(top) = tree.top_q().filter(|_| progress.total > 0 && progress.answered == progress.total) {
        let value = tree.logicality(top.id).map(|l| l.value).unwrap_or_default();
        if value != "ok" {
            let why = if value == "contradiction" { "CONTRADICTION" } else { "UNCHECKED" };
            ranked.push((5, 0, card_action((Kind::Q, top.id), None, "sci-review", "研究全体の F どうしに矛盾が無いか確かめる".into(), "一番上の Q に Logicality を記録する".into(), why, top.id)));
        }
    }
    ranked.sort_by_key(|(rank, pos, _)| (*rank, *pos));
    debts
        .iter()
        .map(debt_action)
        .chain(ranked.into_iter().map(|(_, _, a)| a))
        .take(limit)
        .collect()
}

// ---- ブロック定義の注意 -----------------------------------------------------------------

fn split_csv(value: &str) -> Vec<String> {
    value.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()
}

/// ブロック定義の書き間違い（R5）を 1 行ずつ返す。ブロック id 順、同じ文は 1 回だけ。
#[must_use]
pub fn block_def_warnings(roots: &PaperRoots) -> Vec<String> {
    const KEYS: [&str; 6] = ["sci_ops", "sci_scope", "sci_action", "sci_support", "sci_done_agent", "sci_done_user"];
    let mut dirs = vec![roots.root.join(".sai").join("blocks")];
    dirs.extend(crate::paths::sai_home_path("blocks"));
    // 同じ id は先に見つけた `<root>` 側だけ。
    let mut blocks: std::collections::BTreeMap<String, Vec<(String, String)>> = Default::default();
    for dir in dirs {
        let Ok(entries) = fs::read_dir(dir) else { continue };
        for path in entries.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "md")) {
            let Some(id) = path.file_stem().map(|s| s.to_string_lossy().to_string()) else { continue };
            let Ok(text) = fs::read_to_string(&path) else { continue };
            blocks.entry(id).or_insert_with(|| read_frontmatter(&text));
        }
    }
    let mut out: Vec<String> = Vec::new();
    for (id, pairs) in &blocks {
        if !pairs.iter().any(|(k, _)| KEYS.contains(&k.as_str())) {
            continue;
        }
        for (key, value) in pairs {
            match key.as_str() {
                "sci_ops" => {
                    for op in split_csv(value) {
                        if !ops::is_known_op(&op) {
                            out.push(format!("{id}: sci_ops に知らない op があります: {op}"));
                        }
                        if ops::is_user_only_op(&op) {
                            out.push(format!("{id}: sci_ops に利用者だけの op があります: {op}"));
                        }
                    }
                }
                "sci_scope" if !SCOPES.contains(&value.trim()) => {
                    out.push(format!("{id}: 知らない sci_scope です: {}", value.trim()));
                }
                _ => {}
            }
        }
    }
    out.extend(action::block_warnings(roots));
    let mut seen = HashSet::new();
    out.retain(|line| seen.insert(line.clone()));
    out
}

// ---- status / node_detail -------------------------------------------------------------

fn seed_json(tree: &Tree, s: &SeedRow) -> SeedJson {
    SeedJson {
        id: fmt_ref('S', s.id),
        title: s.title.clone(),
        sentence: s.sentence.clone(),
        path: s.path.iter().map(|id| tree.label(*id).0).collect(),
        run: s.run_id,
        created_at: s.created_at.clone(),
    }
}

fn running_runs(roots: &PaperRoots) -> Vec<RunningJson> {
    action::sci_runs(roots)
        .into_iter()
        .filter(|r| r.owner == "todo" || r.owner == "running")
        .map(|r| RunningJson {
            run_id: r.run_id,
            node: r.keys.node.map(|(k, id)| fmt_id(k, id)),
            block: r.block,
            owner: r.owner,
        })
        .collect()
}

/// `status` の JSON。
pub fn status(conn: &Connection, roots: &PaperRoots, defs: &Defs) -> Result<Status> {
    let initialized = store::is_initialized(conn)?;
    let legacy = !initialized && store::is_legacy_only(conn)?;
    let mut warnings = Vec::new();
    if legacy {
        warnings.push(LEGACY_WARNING.to_string());
    }
    warnings.extend(defs.warnings.iter().cloned());
    warnings.extend(block_def_warnings(roots));
    let mut out = Status {
        initialized,
        legacy,
        type_info: None,
        progress: Progress::default(),
        nodes: Vec::new(),
        seeds: Vec::new(),
        seed_draft: None,
        next: Vec::new(),
        debts: Vec::new(),
        debt_count: 0,
        pending_proposals: 0,
        decisions: Vec::new(),
        running: Vec::new(),
        warnings,
    };
    if !initialized {
        return Ok(out);
    }
    let tree = Tree::load(conn, roots, defs)?;
    if let Some(id) = &tree.type_id {
        match defs.type_def(id) {
            Some(t) => out.type_info = Some(TypeJson { id: id.clone(), name: t.name.clone() }),
            None => {
                out.type_info = Some(TypeJson { id: id.clone(), name: id.clone() });
                out.warnings.push(format!("知らない研究の型です: {id}"));
            }
        }
    }
    let debts = action::debts(conn, roots)?;
    out.progress = tree.progress();
    out.nodes = tree.nodes.iter().map(|n| tree.row(n.id)).collect();
    out.seeds = tree.seeds.iter().map(|s| seed_json(&tree, s)).collect();
    out.seed_draft = tree.seed_draft(&roots.data_dir);
    out.next = next_actions(&tree, &debts, 10);
    out.debt_count = debts.len();
    out.debts = debts;
    out.pending_proposals = tree.pending.len();
    out.decisions = tree.log.iter().take(30).map(|r| tree.decision(r)).collect();
    out.running = running_runs(roots);
    Ok(out)
}

/// `node` の JSON。
pub fn node_detail(conn: &Connection, roots: &PaperRoots, defs: &Defs, id: &str) -> Result<NodeDetail> {
    let (kind, num) = parse_id(id)?;
    let tree = Tree::load(conn, roots, defs)?;
    let node = tree
        .node(num)
        .filter(|n| n.kind == kind)
        .ok_or_else(|| RtError::NotFound(format!("{id} はありません")))?;

    let mut path = Vec::new();
    let mut cursor = Some(num);
    while let Some(c) = cursor.and_then(|c| tree.node(c)) {
        path.push(PathItem { id: fmt_id(c.kind, c.id), kind: c.kind.as_str().into(), text: c.text.clone() });
        cursor = c.parent_id;
    }
    path.reverse();

    // 自分を含む子孫（木の順）。
    let mut family = vec![num];
    let mut i = 0;
    while i < family.len() {
        family.extend(tree.kids.get(&family[i]).into_iter().flatten().copied());
        i += 1;
    }
    let subtree_ids: Vec<i64> = tree.nodes.iter().map(|n| n.id).filter(|n| *n != num && family.contains(n)).collect();
    let family_set: HashSet<i64> = family.iter().copied().collect();

    let debts = action::debts(conn, roots)?;
    let actions = action::sci_runs(roots)
        .into_iter()
        .filter(|r| r.keys.node.is_some_and(|(_, n)| family_set.contains(&n)))
        .map(|r| {
            let messages: Vec<String> =
                debts.iter().filter(|d| d.run_id == r.run_id).map(|d| d.message.clone()).collect();
            ActionJson {
                run_id: r.run_id,
                block: r.block,
                owner: r.owner,
                purpose: r.keys.purpose,
                occurred_at: r.occurred_at,
                debt: !messages.is_empty(),
                debt_messages: messages,
            }
        })
        .collect();

    let is_q = node.kind == Kind::Q;
    Ok(NodeDetail {
        node: tree.row(num),
        path,
        children: tree.kids[&num].iter().map(|k| tree.row(*k)).collect(),
        subtree: subtree_ids.iter().map(|k| tree.row(*k)).collect(),
        guide: node.guide_key.as_deref().and_then(|k| defs.guide(k)).filter(|_| is_q).map(|g| GuideJson {
            key: g.key.clone(),
            name: g.name.clone(),
            explain: g.explain.clone(),
            question: g.question.clone(),
            child_candidates: g.child_candidates.clone(),
            extract: g.extract.clone(),
            pitfalls: g.pitfalls.clone(),
            target: g.target,
        }),
        logic: tree
            .logic_rows
            .iter()
            .filter(|r| is_q && r.q_id == num)
            .map(|r| LogicJson {
                value: r.value.clone(),
                note: r.note.clone(),
                findings: f_ids(&r.findings),
                f_count: r.f_count,
                run: r.run_id,
                created_at: r.created_at.clone(),
            })
            .collect(),
        decisions: tree.log.iter().filter(|r| family_set.contains(&r.node_id)).take(30).map(|r| tree.decision(r)).collect(),
        actions,
        pending: tree
            .pending
            .iter()
            .filter(|p| p.node_id == Some(num))
            .map(|p| PendingJson {
                id: fmt_ref('P', p.id),
                op: p.op.clone(),
                payload: p.payload.clone(),
                reason: p.reason.clone(),
                run: p.run_id,
                created_at: p.created_at.clone(),
            })
            .collect(),
        next: next_actions(&tree, &debts, usize::MAX).into_iter().filter(|a| a.targets.contains(&num)).take(3).collect(),
        searches: store::list_searches(conn, Some(num))?
            .into_iter()
            .map(|s| SearchJson { id: s.id, run: s.run_id, tool: s.tool, query: s.query, hits: s.hits, set_id: s.set_id, created_at: s.created_at })
            .collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::Database;
    use std::path::PathBuf;

    struct Env {
        dir: PathBuf,
        roots: PaperRoots,
        db: Database,
    }

    fn env(tag: &str) -> Env {
        let dir = std::env::temp_dir().join(format!("sai-rt-compute-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let data_dir = dir.join(".sai/paper");
        fs::create_dir_all(&data_dir).unwrap();
        fs::create_dir_all(dir.join(".sai/blocks")).unwrap();
        let db = Database::open(&data_dir.join("papr.db")).unwrap();
        store::meta_set(db.connection(), "initialized_at", "2026-10-02 00:00:00").unwrap();
        Env { roots: PaperRoots { root: dir.clone(), data_dir }, dir, db }
    }

    fn add(e: &Env, kind: Kind, parent: Option<i64>, text: &str, guide: Option<&str>, answer: Option<&str>) -> i64 {
        let n = store::NewNode {
            kind, parent_id: parent, guide_key: guide, text, body: "", answer_kind: answer, cites: &[],
            paper_id: None, run_id: None, report_id: None, mlflow_ref: None, proposal_id: None, created_by: "user",
        };
        store::insert_node(e.db.connection(), &n).unwrap()
    }

    fn q(e: &Env, parent: Option<i64>, text: &str, guide: Option<&str>) -> i64 {
        add(e, Kind::Q, parent, text, guide, None)
    }
    fn o(e: &Env, parent: i64, text: &str) -> i64 {
        add(e, Kind::O, Some(parent), text, None, Some("answer"))
    }
    fn f(e: &Env, parent: i64) -> i64 {
        add(e, Kind::F, Some(parent), "根拠", None, None)
    }

    fn tree(e: &Env) -> Tree {
        Tree::load(e.db.connection(), &e.roots, &Defs::default()).unwrap()
    }

    /// Q0 → Q1 → O★ 予後予測 → (Q2 データ, Q3 既存手法の課題) と、★ の無い O の下の Q。
    fn figure(e: &Env) -> [i64; 8] {
        let q0 = q(e, None, "大目的は？", None);
        let q1 = q(e, Some(q0), "何を予測する？", None);
        let star = o(e, q1, "予後予測");
        store::set_starred(e.db.connection(), star, true).unwrap();
        let other = o(e, q1, "診断");
        let qd = q(e, Some(star), "データは？", None);
        let qi = q(e, Some(star), "既存手法の課題は？", None);
        let qx = q(e, Some(other), "診断のデータは？", None);
        for _ in 0..2 {
            let od = o(e, qd, "MRI");
            f(e, od);
        }
        let oi = add(e, Kind::O, Some(qi), "精度が低い", None, Some("issue"));
        let ex = add(e, Kind::E, Some(oi), "実験", None, None);
        f(e, ex);
        f(e, oi);
        [q0, q1, star, other, qd, qi, qx, oi]
    }

    #[test]
    fn figure_path_leaf_objectivity() {
        let e = env("figure");
        let [q0, q1, _, _, qd, qi, qx, _] = figure(&e);
        let s = status(e.db.connection(), &e.roots, &Defs::default()).unwrap();
        let row = |id: i64| s.nodes.iter().find(|n| n.id == fmt_id(Kind::Q, id)).unwrap();
        assert!(row(q0).on_path && !row(q0).leaf);
        assert!(row(qd).on_path && row(qd).leaf);
        assert!(!row(qx).on_path);
        // Q データ: O 2 つ + 各 F 1 つ → count は F の数 2、目安は 2。
        assert_eq!(row(qd).objectivity, Some(Objectivity { count: 2, target: 2, met: true }));
        // Q 課題: O の F 1 + E の F 1 → 2。
        assert_eq!(row(qi).objectivity, Some(Objectivity { count: 2, target: 2, met: true }));
        // Q1 は子の Q が 0 にならない: 「★ の下の Q」は子ではない（O の下）ので葉で、O は 2 つ・F は 0。
        assert!(row(q1).leaf);
        assert_eq!(row(q1).objectivity, Some(Objectivity { count: 0, target: 2, met: false }));
        // Q0 は子の Q が Q1 だけ。
        assert_eq!(row(q0).objectivity, Some(Objectivity { count: 0, target: 1, met: false }));
        // 道の上の葉の Q は Q1・Q データ・Q 課題。
        assert_eq!(s.progress, Progress { answered: 1, total: 3 });
    }

    #[test]
    fn logicality_resets_and_ignores_reverted() {
        let e = env("logic");
        let q0 = q(&e, None, "Q", None);
        let o1 = o(&e, q0, "a");
        f(&e, o1);
        let conn = e.db.connection();
        assert_eq!(tree(&e).logicality(q0).unwrap().value, "unchecked");
        assert!(tree(&e).logicality(o1).is_none());
        store::insert_logic(conn, q0, "ok", "", &[], 1, None, None, "user").unwrap();
        let t = tree(&e);
        assert_eq!(t.logicality(q0).unwrap().value, "ok");
        assert!(t.logicality(q0).unwrap().checked_at.is_some());
        f(&e, o1);
        assert_eq!(tree(&e).logicality(q0).unwrap().value, "unchecked");
        // reverted の記録は数えない。
        let p = store::insert_proposal(conn, None, None, "record_logic", Some(q0), &json!({}), "", "auto", "reverted").unwrap();
        store::insert_logic(conn, q0, "contradiction", "", &[], 2, None, Some(p), "user").unwrap();
        assert_eq!(tree(&e).logicality(q0).unwrap().value, "unchecked");
        store::insert_logic(conn, q0, "contradiction", "", &[], 2, None, None, "user").unwrap();
        assert_eq!(tree(&e).logicality(q0).unwrap().value, "contradiction");
    }

    #[test]
    fn progress_counts_star() {
        let e = env("progress");
        let q0 = q(&e, None, "Q", None);
        let a = o(&e, q0, "a");
        assert_eq!(tree(&e).progress(), Progress { answered: 0, total: 1 });
        store::set_starred(e.db.connection(), a, true).unwrap();
        assert_eq!(tree(&e).progress(), Progress { answered: 1, total: 1 });
    }

    fn defs_with_guide() -> Defs {
        let mut d = Defs::default();
        d.guides.push(GuideDef {
            key: "SPLIT".into(), name: "分ける".into(), explain: String::new(), question: String::new(),
            child_candidates: vec!["a".into()], extract: vec![], pitfalls: vec![], target: 2,
        });
        d
    }

    fn whys(a: &[NextAction]) -> Vec<(&str, Option<&str>)> {
        a.iter().map(|x| (x.why.as_str(), x.block.as_deref())).collect()
    }

    #[test]
    fn next_actions_each_rule() {
        let e = env("next");
        let conn = e.db.connection();
        let defs = defs_with_guide();
        let t = |e: &Env| Tree::load(e.db.connection(), &e.roots, &defs).unwrap();
        let q0 = q(&e, None, "葉の Q", Some("SPLIT"));
        let a = whys(&next_actions(&t(&e), &[], 10)).into_iter().map(|(w, b)| (w.to_string(), b.map(String::from))).collect::<Vec<_>>();
        assert_eq!(a, vec![("NO_CHILD_Q".to_string(), Some("sci-split".to_string()))]);
        // ガイドが無ければ NO_OPTION。
        let e2 = env("next2");
        let r = q(&e2, None, "Q", None);
        let t2 = Tree::load(e2.db.connection(), &e2.roots, &Defs::default()).unwrap();
        let acts = next_actions(&t2, &[], 10);
        assert_eq!((acts[0].why.as_str(), acts[0].block.as_deref()), ("NO_OPTION", Some("sci-ideate")));
        let _ = r;
        // O が 1 つで F が無い → FEW_FINDINGS（option 付き）。
        let o1 = o(&e, q0, "O1");
        let acts = next_actions(&t(&e), &[], 10);
        assert_eq!(acts[0].why, "FEW_FINDINGS");
        assert_eq!(acts[0].block.as_deref(), Some("sci-search"));
        assert_eq!(acts[0].option, Some(fmt_id(Kind::O, o1)));
        assert_eq!(acts[0].expect, "F を 2 つ以上");
        // F が 2 つで Objectivity は満たす → UNCHECKED。
        f(&e, o1);
        f(&e, o1);
        let acts = next_actions(&t(&e), &[], 10);
        assert_eq!((acts[0].why.as_str(), acts[0].block.as_deref()), ("UNCHECKED", Some("sci-debate")));
        // 確かめて ok → NO_STAR（card は null）。
        store::insert_logic(conn, q0, "contradiction", "", &[], 2, None, None, "user").unwrap();
        assert_eq!(next_actions(&t(&e), &[], 10)[0].why, "CONTRADICTION");
        store::insert_logic(conn, q0, "ok", "", &[], 2, None, None, "user").unwrap();
        let acts = next_actions(&t(&e), &[], 10);
        assert_eq!(acts[0].why, "NO_STAR");
        assert_eq!(acts[0].kind, "user");
        assert!(acts[0].card.is_none());
        // ★ を付け、Issue の O を足す → NO_EXPERIMENT。review は全部済んでいて Logicality ok なので出ない。
        store::set_starred(conn, o1, true).unwrap();
        assert!(next_actions(&t(&e), &[], 10).is_empty());
        let issue = add(&e, Kind::O, Some(q0), "課題", None, Some("issue"));
        let acts = next_actions(&t(&e), &[], 10);
        assert_eq!((acts[0].why.as_str(), acts[0].block.as_deref()), ("NO_EXPERIMENT", Some("sci-exp-sprout")));
        assert_eq!(acts[0].card.as_ref().unwrap()["node"], json!(fmt_id(Kind::O, issue)));
    }

    #[test]
    fn review_and_debt_first_and_limits() {
        let e = env("review");
        let conn = e.db.connection();
        let q0 = q(&e, None, "Q", None);
        let a = o(&e, q0, "a");
        store::set_starred(conn, a, true).unwrap();
        f(&e, a);
        f(&e, a);
        // 子の Q が無い唯一の Q で Logicality 未確認 → debate が先に出る。review は一番上の Q が葉なら debate と並ぶ。
        let debt = action::Debt {
            run_id: 7, block: "sci-read".into(), node: Some(fmt_id(Kind::O, a)), side: "user".into(),
            criterion: "note".into(), message: "note がありません".into(), fix: json!({"kind": "paper", "paper_id": 37}),
        };
        let t = tree(&e);
        let acts = next_actions(&t, &[debt.clone()], 10);
        assert_eq!(acts[0].kind, "debt");
        assert_eq!(acts[0].why, "DEBT");
        assert!(acts[0].card.is_none());
        assert!(acts.iter().any(|x| x.block.as_deref() == Some("sci-review") && x.why == "UNCHECKED"));
        assert_eq!(next_actions(&t, &[debt.clone(), debt.clone(), debt], 2).len(), 2);
    }

    #[test]
    fn seed_draft_fills_keys_and_mind() {
        let e = env("seed");
        let mut defs = Defs::default();
        defs.types.push(crate::rt::defs::TypeDef {
            id: "problem".into(), name: "課題".into(), root: String::new(), explain: String::new(),
            tree: vec![], seed: "{PURPOSE}を{NOPE}で。{mind:メモ}".into(),
        });
        store::meta_set(e.db.connection(), "type", "problem").unwrap();
        let q0 = q(&e, None, "目的は？", Some("PURPOSE"));
        let a = o(&e, q0, "予後予測");
        let b = o(&e, q0, "診断");
        store::set_starred(e.db.connection(), a, true).unwrap();
        store::set_starred(e.db.connection(), b, true).unwrap();
        fs::write(crate::paths::mind_path(&e.roots.data_dir), "前書き\n## メモ\n<!-- 消える -->\n本文です\n").unwrap();
        let t = Tree::load(e.db.connection(), &e.roots, &defs).unwrap();
        let d = t.seed_draft(&e.roots.data_dir).unwrap();
        assert_eq!(d.text, "予後予測・診断を{NOPE}で。本文です");
        assert!(d.parts[0].filled && !d.parts[2].filled);
        assert_eq!(d.parts[4].key.as_deref(), Some("mind:NOTES"));
    }

    #[test]
    fn block_warnings_three_lines() {
        let e = env("warn");
        fs::write(
            e.dir.join(".sai/blocks/x.md"),
            "---\nname: x\nsci_ops: add_o, star, foo\nsci_scope: wide\n---\n本文\n",
        )
        .unwrap();
        let lines = block_def_warnings(&e.roots);
        let mine: Vec<&String> = lines.iter().filter(|l| l.starts_with("x:")).collect();
        assert_eq!(
            mine,
            vec![
                "x: sci_ops に利用者だけの op があります: star",
                "x: sci_ops に知らない op があります: foo",
                "x: 知らない sci_scope です: wide",
            ]
        );
    }

    #[test]
    fn legacy_status_and_json_keys() {
        let dir = std::env::temp_dir().join(format!("sai-rt-compute-legacy-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let data_dir = dir.join(".sai/paper");
        fs::create_dir_all(&data_dir).unwrap();
        let db = Database::open(&data_dir.join("papr.db")).unwrap();
        let roots = PaperRoots { root: dir, data_dir };
        db.connection()
            .execute_batch("CREATE TABLE IF NOT EXISTS sci_meta (key TEXT PRIMARY KEY, value TEXT); INSERT INTO sci_meta VALUES ('initialized_at', 'x');")
            .unwrap();
        let s = status(db.connection(), &roots, &Defs::default()).unwrap();
        assert!(!s.initialized && s.legacy);
        assert_eq!(s.warnings[0], LEGACY_WARNING);
        let keys = |v: &Value| {
            let mut k: Vec<String> = v.as_object().unwrap().keys().cloned().collect();
            k.sort();
            k
        };
        let want = |names: &[&str]| {
            let mut k: Vec<String> = names.iter().map(|s| s.to_string()).collect();
            k.sort();
            k
        };
        assert_eq!(
            keys(&serde_json::to_value(&s).unwrap()),
            want(&["initialized", "legacy", "type", "progress", "nodes", "seeds", "seed_draft", "next", "debts", "debt_count", "pending_proposals", "decisions", "running", "warnings"])
        );

        let e = env("keys");
        figure(&e);
        let s = status(e.db.connection(), &e.roots, &Defs::default()).unwrap();
        let v = serde_json::to_value(&s).unwrap();
        assert_eq!(
            keys(&v["nodes"][0]),
            want(&["id", "kind", "parent", "depth", "ord", "text", "body", "guide", "answer_kind", "starred", "on_path", "leaf", "objectivity", "logicality", "counts", "cites", "paper", "run", "mlflow", "created_by", "created_at"])
        );
        assert_eq!(keys(&v["next"][0]), want(&["kind", "node", "option", "block", "purpose", "expect", "why", "card", "fix"]));
        let d = node_detail(e.db.connection(), &e.roots, &Defs::default(), "Q2").unwrap();
        let dv = serde_json::to_value(&d).unwrap();
        assert_eq!(
            keys(&dv),
            want(&["node", "path", "children", "subtree", "guide", "logic", "decisions", "actions", "pending", "next", "searches"])
        );
        assert!(d.next.len() <= 3);
        assert!(matches!(
            node_detail(e.db.connection(), &e.roots, &Defs::default(), "Q999"),
            Err(RtError::NotFound(m)) if m == "Q999 はありません"
        ));
    }
}
