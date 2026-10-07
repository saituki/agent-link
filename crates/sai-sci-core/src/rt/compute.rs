//! 木の計算（Objectivity・Logicality・★ の道・進み具合・SEED の下書き・Next Action・注意）と、
//! `status` / `node` の JSON（C-JSON）。CLI・Web・TUI はこの結果を写すだけにする。

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;

use chrono::{DateTime, NaiveDateTime, Utc};
use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;
use serde_json::{Value, json};

use super::defs::{Defs, GuideDef};
use super::store::{self, LogRow, LogicRow, NodeRow, ProposalRow};
use super::{Kind, Result, RtError, SEED_ID, action, fmt_id, fmt_ref, ops, parse_id};
use sai_paper_core::paths::PaperRoots;
use crate::runs::read_frontmatter;

/// 古い形の木だけがあるときの注意。
const LEGACY_WARNING: &str =
    "古い形の木です。`sai-sci init --type <型>` で作り直してください";

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
    pub bottom_q: bool,
    /// SPROUT（kind S）の PATH（終点の O の ID）。S 以外は空。
    pub paths: Vec<String>,
    pub objectivity: Option<Objectivity>,
    pub logicality: Option<Logicality>,
    pub counts: Counts,
    pub cites: Vec<String>,
    pub paper: Option<i64>,
    pub run: Option<i64>,
    pub mlflow: Option<String>,
    pub created_by: String,
    pub created_at: String,
    /// この Node に付いた PENDING の PROPOSAL の数。
    pub pending: usize,
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
    pub actor: String,
    pub at: String,
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

/// SEED（RESEARCH_TYPE と mind.md）の中身。
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct SeedJson {
    #[serde(rename = "type")]
    pub type_info: Option<TypeJson>,
    pub mind: Vec<MindSection>,
}

/// mind.md の `## 見出し` 1 つ。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct MindSection {
    pub heading: String,
    pub body: String,
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
    pub seed: SeedJson,
    pub next: Vec<NextAction>,
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
    /// この run の対象の Node（`O12` など）。
    pub node: Option<String>,
    /// task.md の global_round（無ければ null）。
    pub global_round: Option<i64>,
    pub purpose: String,
    pub occurred_at: String,
}

/// `node_detail.pending` の 1 要素。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PendingJson {
    pub id: String,
    pub op: String,
    pub payload: Value,
    pub run: Option<i64>,
    pub created_at: String,
}

/// `node_detail.searches` の 1 要素。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SearchJson {
    pub id: i64,
    pub run: Option<i64>,
    /// その run の block（sci の run に見つからなければ null）。
    pub block: Option<String>,
    pub tool: String,
    pub query: String,
    pub hits: i64,
    pub set_id: Option<i64>,
    /// その 1 回の検索の履歴（Library の検索結果へのリンク先）。
    pub log_id: Option<i64>,
    pub created_at: String,
}

/// `node_detail.creator`。この Node を作った人と run。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CreatorJson {
    /// created_by のまま（`user` / `run:<N>`）。
    pub by: String,
    /// created_by が `run:<N>` なら N。
    pub run_id: Option<i64>,
    /// その run の block（sci の run に見つからなければ空）。
    pub block: String,
    /// その run のフォルダの親の名前（見つからなければ空）。
    pub owner: String,
    /// その run の global_round（見つからなければ null）。
    pub global_round: Option<i64>,
    /// その run の対象の Node（F を作った run なら親の O など）。
    pub node: Option<String>,
    pub purpose: String,
    /// Node の created_at。
    pub at: String,
}

/// `node_detail.sources` の 1 要素。Node の cites 1 つ分。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SourceJson {
    /// cites の文字のまま（`#54.11.3`）。
    pub cite: String,
    /// 論文の番号（読めなければ null）。
    pub paper: Option<i64>,
    /// 論文の題（Library に無ければ空）。
    pub title: String,
    /// 節の見出し（節の番号が無い・節の表が無ければ null）。
    pub section: Option<String>,
    /// 塊の本文の先頭 200 字（塊の番号が無い・見つからなければ null）。
    pub passage: Option<String>,
}

/// `node_detail.history` の 1 行。init が先頭、残りは古い順。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct HistoryJson {
    /// `init` / `Add Q` / `Add O` / `Add F` / `Add E` / `Run` / rt_log の action（`edit` など）
    pub action: String,
    /// RFC3339（UTC）。DB の `YYYY-MM-DD HH:MM:SS` は UTC として読み替える。
    pub at: String,
    /// `user` か `run:<N>`（Run の行は `run:<run_id>`）。
    pub by: String,
    pub run_id: Option<i64>,
    /// その run の block（見つからなければ空）。
    pub block: String,
    pub global_round: Option<i64>,
    pub round: Option<i64>,
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
    pub creator: CreatorJson,
    pub pending: Vec<PendingJson>,
    pub next: Vec<NextAction>,
    pub searches: Vec<SearchJson>,
    pub sources: Vec<SourceJson>,
    pub history: Vec<HistoryJson>,
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
        let roots_none: Vec<NodeRow> = by_parent.remove(&None).unwrap_or_default();
        let mut stack: Vec<(NodeRow, usize)> = Vec::new();
        // 親の無い生きた Q（top_Q）を (ord, id) の順に DFS の起点にし、そのあとに SPROUT（kind S）を並べる。
        // `roots_none` は `(parent_id, ord, id)` 順なので、Q も S もこの順のまま。
        let mut roots: Vec<NodeRow> = Vec::new();
        roots.extend(roots_none.iter().filter(|n| n.kind == Kind::Q).cloned());
        roots.extend(roots_none.iter().filter(|n| n.kind == Kind::S).cloned());
        for n in roots.into_iter().rev() {
            stack.push((n, 0));
        }
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
                        (Kind::S, Kind::E) => true,
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

    /// O（または E・S）に付いた F の数。O・E・S の下だけをたどり、Q の下には入らない。
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

    /// top_Q（親の無い Q）が 0。
    #[must_use]
    pub fn depth(&self, id: i64) -> usize {
        self.depths.get(&id).copied().unwrap_or(0)
    }

    /// 生きた子の Q を持たない Q（bottom_Q）か。
    #[must_use]
    pub fn is_bottom_q(&self, id: i64) -> bool {
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
            if n.kind == Kind::Q && self.on_path(n.id) && self.is_bottom_q(n.id) {
                p.total += 1;
                if self.kid_nodes(n.id).any(|k| k.kind == Kind::O && k.starred) {
                    p.answered += 1;
                }
            }
        }
        p
    }

    /// RESEARCH_TYPE の表示名（型が分からなければ id、それも無ければ `None`）。
    #[must_use]
    pub fn type_name(&self) -> Option<String> {
        let id = self.type_id.as_deref()?;
        Some(self.defs.type_def(id).map_or_else(|| id.to_string(), |t| t.name.clone()))
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
                // SPROUT（S）は Q・O・F・E の数のどれにも入れない。
                Kind::S => {}
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
            bottom_q: is_q && self.is_bottom_q(id),
            paths: if n.kind == Kind::S {
                n.paths.iter().map(|o| self.label(*o).0).collect()
            } else {
                Vec::new()
            },
            objectivity: self.objectivity(id),
            logicality: self.logicality(id),
            counts,
            cites: n.cites.clone(),
            paper: n.paper_id,
            run: n.run_id,
            mlflow: n.mlflow_ref.clone(),
            created_by: n.created_by.clone(),
            created_at: n.created_at.clone(),
            pending: self.pending.iter().filter(|p| p.node_id == Some(id)).count(),
        }
    }

    /// 生きた SPROUT（kind S）の `paths` に入っている O の id。
    #[must_use]
    pub fn sprout_o_ids(&self) -> HashSet<i64> {
        self.nodes
            .iter()
            .filter(|n| n.kind == Kind::S)
            .flat_map(|n| n.paths.iter().copied())
            .collect()
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
            actor: row.actor.clone(),
            at: row.created_at.clone(),
        }
    }
}

// ---- mind.md --------------------------------------------------------------------------

/// mind.md の `## 見出し` ごとの本文（コメントと空行を除く。空の節は出さない）。
#[must_use]
pub fn mind_sections(data_dir: &Path) -> Vec<(String, String)> {
    let Ok(text) = fs::read_to_string(sai_paper_core::paths::mind_path(data_dir)) else {
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
        targets: [Some(q), option].into_iter().flatten().collect(),
    }
}

impl Tree {
    /// 道の上の葉の Q 1 つに出す Next Action（順 2〜6 の最初に当てはまる 1 つ）。
    fn q_action(&self, q: &NodeRow) -> Option<(u8, NextAction)> {
        if !self.is_bottom_q(q.id) {
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
                targets: vec![q.id],
            }));
        }
        None
    }
}

/// Next Action（順の番号 → 木の順）を `limit` 件まで。
#[must_use]
pub fn next_actions(tree: &Tree, limit: usize) -> Vec<NextAction> {
    let mut ranked: Vec<(u8, usize, NextAction)> = Vec::new();
    // どの生きた SPROUT の paths にも入っていない Issue の O にだけ NO_EXPERIMENT を出す。
    let sprouted = tree.sprout_o_ids();
    for (pos, n) in tree.nodes.iter().enumerate() {
        if n.kind != Kind::Q || !tree.on_path(n.id) {
            continue;
        }
        if let Some((rank, a)) = tree.q_action(n) {
            ranked.push((rank, pos, a));
        }
        if tree.is_bottom_q(n.id) {
            for o in tree.kid_nodes(n.id) {
                let issue = o.kind == Kind::O && o.answer_kind.as_deref() == Some("issue");
                if issue && !sprouted.contains(&o.id) {
                    // SPROUT を作るのは SEED 全体の仕事なので、CARD の対象は SEED。
                    let purpose = format!("「{}」（{}）を含む SPROUT を作る", o.text, fmt_id(Kind::O, o.id));
                    let card = json!({
                        "node": SEED_ID, "block": "sci-exp-sprout", "purpose": purpose,
                        "expect": "SPROUT と最初の BRANCH", "runner": "pi",
                    });
                    let a = NextAction {
                        kind: "card".into(),
                        node: Some(SEED_ID.to_string()),
                        option: Some(fmt_id(Kind::O, o.id)),
                        block: Some("sci-exp-sprout".into()),
                        purpose,
                        expect: "SPROUT と最初の BRANCH".into(),
                        why: "NO_EXPERIMENT".into(),
                        card: Some(card),
                        targets: vec![n.id, o.id],
                    };
                    ranked.push((7, tree.index[&o.id], a));
                }
            }
        }
    }
    // 全体の確かめ: 進み具合が済んでいて、Logicality が ok でない top_Q が 1 つ以上あるとき。
    let progress = tree.progress();
    if progress.total > 0 && progress.answered == progress.total {
        let targets: Vec<i64> = tree
            .nodes
            .iter()
            .filter(|n| n.kind == Kind::Q && n.parent_id.is_none() && tree.on_path(n.id))
            .filter(|n| tree.logicality(n.id).is_some_and(|l| l.value != "ok"))
            .map(|n| n.id)
            .collect();
        if !targets.is_empty() {
            let contradiction = targets
                .iter()
                .any(|id| tree.logicality(*id).is_some_and(|l| l.value == "contradiction"));
            let why = if contradiction { "CONTRADICTION" } else { "UNCHECKED" };
            let purpose = "research-tree の全体の F どうしに矛盾が無いか確かめる".to_string();
            let card = json!({
                "node": SEED_ID, "block": "sci-review", "purpose": purpose,
                "expect": "各 top_Q に Logicality を記録する", "runner": "pi",
            });
            ranked.push((5, 0, NextAction {
                kind: "card".into(),
                node: Some(SEED_ID.to_string()),
                option: None,
                block: Some("sci-review".into()),
                purpose,
                expect: "各 top_Q に Logicality を記録する".into(),
                why: why.into(),
                card: Some(card),
                targets,
            }));
        }
    }
    ranked.sort_by_key(|(rank, pos, _)| (*rank, *pos));
    ranked.into_iter().map(|(_, _, a)| a).take(limit).collect()
}

// ---- ブロック定義の注意 -----------------------------------------------------------------

fn split_csv(value: &str) -> Vec<String> {
    value.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()
}

/// ブロック定義の書き間違い（R5）を 1 行ずつ返す。ブロック id 順、同じ文は 1 回だけ。
#[must_use]
pub fn block_def_warnings(roots: &PaperRoots) -> Vec<String> {
    const KEYS: [&str; 2] = ["sci_ops", "sci_type"];
    let mut dirs = vec![roots.root.join(".sai").join("blocks")];
    dirs.extend(sai_paper_core::paths::sai_home_path("blocks"));
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
            if key.as_str() == "sci_ops" {
                for op in split_csv(value) {
                    if !ops::is_known_op(&op) {
                        out.push(format!("{id}: sci_ops に知らない op があります: {op}"));
                    }
                    if ops::is_user_only_op(&op) {
                        out.push(format!("{id}: sci_ops に利用者だけの op があります: {op}"));
                    }
                }
            }
        }
    }
    out.extend(action::block_warnings(roots));
    let mut seen = HashSet::new();
    out.retain(|line| seen.insert(line.clone()));
    out
}

// ---- status / node_detail -------------------------------------------------------------

/// DB の時刻（`YYYY-MM-DD HH:MM:SS`、UTC）や RFC3339 の時刻を UTC の RFC3339 に直す。読めなければ元の文字のまま。
fn rfc3339_utc(text: &str) -> String {
    let text = text.trim();
    if let Ok(naive) = NaiveDateTime::parse_from_str(text, "%Y-%m-%d %H:%M:%S") {
        return DateTime::<Utc>::from_naive_utc_and_offset(naive, Utc).to_rfc3339();
    }
    if let Ok(dt) = DateTime::parse_from_rfc3339(text) {
        return dt.with_timezone(&Utc).to_rfc3339();
    }
    text.to_string()
}

/// `run:<N>` から N を読む（`creator_json` と同じ読み方）。
fn run_id_of(by: &str) -> Option<i64> {
    by.strip_prefix("run:").and_then(|s| s.trim().parse::<i64>().ok())
}

fn running_runs(roots: &PaperRoots) -> Vec<RunningJson> {
    action::sci_runs(roots)
        .into_iter()
        .filter(|r| r.owner == "todo" || r.owner == "running")
        .map(|r| RunningJson {
            run_id: r.run_id,
            node: if r.keys.seed { Some(SEED_ID.to_string()) } else { r.keys.node.map(|(k, id)| fmt_id(k, id)) },
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
        seed: SeedJson::default(),
        next: Vec::new(),
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
    out.progress = tree.progress();
    out.nodes = tree.nodes.iter().map(|n| tree.row(n.id)).collect();
    out.seed = SeedJson {
        type_info: out.type_info.clone(),
        mind: mind_sections(&roots.data_dir)
            .into_iter()
            .map(|(heading, body)| MindSection { heading, body })
            .collect(),
    };
    out.next = next_actions(&tree, 10);
    out.pending_proposals = tree.pending.len();
    out.decisions = tree.log.iter().take(30).map(|r| tree.decision(r)).collect();
    out.running = running_runs(roots);
    Ok(out)
}

/// `node_detail.history` を組み立てる。init が先頭、残り（Add・Run・決定）は at の昇順。
/// at が同じときは Add → Run → 決定の順、その中は元の順にする（安定ソート）。
fn history_json(node: &NodeRow, num: i64, tree: &Tree, runs: &[action::SciRun]) -> Vec<HistoryJson> {
    let run_of = |run_id: Option<i64>| run_id.and_then(|id| runs.iter().find(|r| r.run_id == id));
    let fill = |run: Option<&action::SciRun>| {
        (
            run.map(|r| r.block.clone()).unwrap_or_default(),
            run.and_then(|r| r.global_round),
            run.and_then(|r| r.round),
        )
    };
    // 行の種類（Add → Run → 決定の順）と元の順。init は先頭に置くので別扱い。
    struct Row {
        rank: u8,
        at: String,
        pos: usize,
        action: String,
        by: String,
        run_id: Option<i64>,
        block: String,
        global_round: Option<i64>,
        round: Option<i64>,
    }
    let mut rows: Vec<Row> = Vec::new();
    let mut pos = 0;
    // Add: この Node の生きている直の子ごとに 1 行。
    for kid in tree.kids.get(&num).into_iter().flatten() {
        let Some(k) = tree.node(*kid) else { continue };
        let by = k.created_by.clone();
        let run_id = run_id_of(&by);
        let (block, global_round, round) = fill(run_of(run_id));
        rows.push(Row {
            rank: 0,
            at: rfc3339_utc(&k.created_at),
            pos,
            action: format!("Add {}", k.kind.as_str()),
            by,
            run_id,
            block,
            global_round,
            round,
        });
        pos += 1;
    }
    // Run: この Node を対象にする run ごとに 1 行（子孫の run は入れない）。
    for r in runs.iter().filter(|r| r.keys.node.is_some_and(|(_, n)| n == num)) {
        rows.push(Row {
            rank: 1,
            at: rfc3339_utc(&r.occurred_at),
            pos,
            action: "Run".into(),
            by: format!("run:{}", r.run_id),
            run_id: Some(r.run_id),
            block: r.block.clone(),
            global_round: r.global_round,
            round: r.round,
        });
        pos += 1;
    }
    // 決定: `rt_log` のうちこの Node の行ごとに 1 行。
    for row in tree.log.iter().filter(|r| r.node_id == num) {
        let run_id = run_id_of(&row.actor);
        let (block, global_round, round) = fill(run_of(run_id));
        rows.push(Row {
            rank: 2,
            at: rfc3339_utc(&row.created_at),
            pos,
            action: row.action.clone(),
            by: row.actor.clone(),
            run_id,
            block,
            global_round,
            round,
        });
        pos += 1;
    }
    rows.sort_by(|a, b| a.at.cmp(&b.at).then(a.rank.cmp(&b.rank)).then(a.pos.cmp(&b.pos)));
    // init: 必ず 1 行・先頭。
    let by = node.created_by.clone();
    let run_id = run_id_of(&by);
    let (block, global_round, round) = fill(run_of(run_id));
    let mut out = vec![HistoryJson {
        action: "init".into(),
        at: rfc3339_utc(&node.created_at),
        by,
        run_id,
        block,
        global_round,
        round,
    }];
    out.extend(rows.into_iter().map(|r| HistoryJson {
        action: r.action,
        at: r.at,
        by: r.by,
        run_id: r.run_id,
        block: r.block,
        global_round: r.global_round,
        round: r.round,
    }));
    out
}

/// この Node を作った人と run を並べる。
fn creator_json(node: &NodeRow, runs: &[action::SciRun]) -> CreatorJson {
    let by = node.created_by.clone();
    let run_id = by.strip_prefix("run:").and_then(|s| s.trim().parse::<i64>().ok());
    let run = run_id.and_then(|id| runs.iter().find(|r| r.run_id == id));
    CreatorJson {
        by,
        run_id,
        block: run.map(|r| r.block.clone()).unwrap_or_default(),
        owner: run.map(|r| r.owner.clone()).unwrap_or_default(),
        global_round: run.and_then(|r| r.global_round),
        node: run.and_then(|r| r.keys.node.map(|(k, n)| fmt_id(k, n))),
        purpose: run.map(|r| r.keys.purpose.clone()).unwrap_or_default(),
        at: node.created_at.clone(),
    }
}

/// cites 1 つから論文・節・塊を引く。表が無い・無ければ空や null にする。
fn source_json(conn: &Connection, cite: &str) -> SourceJson {
    let parsed = sai_paper_core::textcheck::parse_cite(cite);
    let paper = parsed.as_ref().map(|c| c.paper);
    let sec = parsed.as_ref().and_then(|c| c.section);
    let title = paper
        .and_then(|id| {
            conn.query_row("SELECT title FROM papers WHERE id = ?1", params![id], |r| r.get::<_, String>(0))
                .optional()
                .ok()
                .flatten()
        })
        .unwrap_or_default();
    let section = match (paper, sec) {
        (Some(id), Some(sec)) => conn
            .query_row(
                "SELECT heading FROM paper_sections WHERE paper_id = ?1 AND sec_no = ?2",
                params![id, sec],
                |r| r.get::<_, String>(0),
            )
            .optional()
            .ok()
            .flatten(),
        _ => None,
    };
    let passage = match (paper, sec, parsed.as_ref().and_then(|c| c.passage)) {
        (Some(id), Some(sec), Some(pas)) => conn
            .query_row(
                "SELECT text FROM paper_passages WHERE paper_id = ?1 AND sec_no = ?2 AND pas_no = ?3",
                params![id, sec, pas],
                |r| r.get::<_, String>(0),
            )
            .optional()
            .ok()
            .flatten()
            .map(|t| t.chars().take(200).collect()),
        _ => None,
    };
    SourceJson { cite: cite.to_string(), paper, title, section, passage }
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

    let runs = action::sci_runs(roots);
    let actions: Vec<ActionJson> = runs
        .iter()
        .filter(|r| r.keys.node.is_some_and(|(_, n)| family_set.contains(&n)))
        .map(|r| ActionJson {
            run_id: r.run_id,
            block: r.block.clone(),
            owner: r.owner.clone(),
            node: r.keys.node.map(|(k, n)| fmt_id(k, n)),
            global_round: r.global_round,
            purpose: r.keys.purpose.clone(),
            occurred_at: r.occurred_at.clone(),
        })
        .collect();
    let creator = creator_json(node, &runs);
    let history = history_json(node, num, &tree, &runs);

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
        creator,
        pending: tree
            .pending
            .iter()
            .filter(|p| p.node_id == Some(num))
            .map(|p| PendingJson {
                id: fmt_ref('P', p.id),
                op: p.op.clone(),
                payload: p.payload.clone(),
                run: p.run_id,
                created_at: p.created_at.clone(),
            })
            .collect(),
        next: next_actions(&tree, usize::MAX).into_iter().filter(|a| a.targets.contains(&num)).take(3).collect(),
        searches: store::list_searches(conn, Some(num))?
            .into_iter()
            .map(|s| {
                let block = s.run_id.and_then(|id| runs.iter().find(|r| r.run_id == id)).map(|r| r.block.clone());
                SearchJson { id: s.id, run: s.run_id, block, tool: s.tool, query: s.query, hits: s.hits, set_id: s.set_id, log_id: s.log_id, created_at: s.created_at }
            })
            .collect(),
        sources: node.cites.iter().map(|c| source_json(conn, c)).collect(),
        history,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use sai_paper_core::database::Database;
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
            paper_id: None, run_id: None, report_id: None, mlflow_ref: None, proposal_id: None,
            paths: &[], created_by: "user",
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
        assert!(row(q0).on_path && !row(q0).bottom_q);
        assert!(row(qd).on_path && row(qd).bottom_q);
        assert!(!row(qx).on_path);
        // Q データ: O 2 つ + 各 F 1 つ → count は F の数 2、目安は 2。
        assert_eq!(row(qd).objectivity, Some(Objectivity { count: 2, target: 2, met: true }));
        // Q 課題: O の F 1 + E の F 1 → 2。
        assert_eq!(row(qi).objectivity, Some(Objectivity { count: 2, target: 2, met: true }));
        // Q1 は子の Q が 0 にならない: 「★ の下の Q」は子ではない（O の下）ので葉で、O は 2 つ・F は 0。
        assert!(row(q1).bottom_q);
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
        let p = store::insert_proposal(conn, None, None, "record_logic", Some(q0), &json!({}), "auto", "reverted").unwrap();
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
        let a = whys(&next_actions(&t(&e), 10)).into_iter().map(|(w, b)| (w.to_string(), b.map(String::from))).collect::<Vec<_>>();
        assert_eq!(a, vec![("NO_CHILD_Q".to_string(), Some("sci-split".to_string()))]);
        // ガイドが無ければ NO_OPTION。
        let e2 = env("next2");
        let r = q(&e2, None, "Q", None);
        let t2 = Tree::load(e2.db.connection(), &e2.roots, &Defs::default()).unwrap();
        let acts = next_actions(&t2, 10);
        assert_eq!((acts[0].why.as_str(), acts[0].block.as_deref()), ("NO_OPTION", Some("sci-ideate")));
        let _ = r;
        // O が 1 つで F が無い → FEW_FINDINGS（option 付き）。
        let o1 = o(&e, q0, "O1");
        let acts = next_actions(&t(&e), 10);
        assert_eq!(acts[0].why, "FEW_FINDINGS");
        assert_eq!(acts[0].block.as_deref(), Some("sci-search"));
        assert_eq!(acts[0].option, Some(fmt_id(Kind::O, o1)));
        assert_eq!(acts[0].expect, "F を 2 つ以上");
        // F が 2 つで Objectivity は満たす → UNCHECKED。
        f(&e, o1);
        f(&e, o1);
        let acts = next_actions(&t(&e), 10);
        assert_eq!((acts[0].why.as_str(), acts[0].block.as_deref()), ("UNCHECKED", Some("sci-debate")));
        // 確かめて ok → NO_STAR（card は null）。
        store::insert_logic(conn, q0, "contradiction", "", &[], 2, None, None, "user").unwrap();
        assert_eq!(next_actions(&t(&e), 10)[0].why, "CONTRADICTION");
        store::insert_logic(conn, q0, "ok", "", &[], 2, None, None, "user").unwrap();
        let acts = next_actions(&t(&e), 10);
        assert_eq!(acts[0].why, "NO_STAR");
        assert_eq!(acts[0].kind, "user");
        assert!(acts[0].card.is_none());
        // ★ を付け、Issue の O を足す → NO_EXPERIMENT。review は全部済んでいて Logicality ok なので出ない。
        store::set_starred(conn, o1, true).unwrap();
        assert!(next_actions(&t(&e), 10).is_empty());
        let issue = add(&e, Kind::O, Some(q0), "課題", None, Some("issue"));
        let acts = next_actions(&t(&e), 10);
        assert_eq!((acts[0].why.as_str(), acts[0].block.as_deref()), ("NO_EXPERIMENT", Some("sci-exp-sprout")));
        // CARD の対象は SEED（SPROUT は全体から作る）。option は Issue の O。
        assert_eq!(acts[0].card.as_ref().unwrap()["node"], json!("SEED"));
        assert_eq!(acts[0].node.as_deref(), Some("SEED"));
        assert_eq!(acts[0].option, Some(fmt_id(Kind::O, issue)));
        assert!(acts[0].purpose.contains(&fmt_id(Kind::O, issue)));
    }

    #[test]
    fn review_first_and_limits() {
        let e = env("review");
        let conn = e.db.connection();
        let q0 = q(&e, None, "Q", None);
        let a = o(&e, q0, "a");
        store::set_starred(conn, a, true).unwrap();
        f(&e, a);
        f(&e, a);
        // 子の Q が無い唯一の Q で Logicality 未確認 → debate が先に出る。review は一番上の Q が葉なら debate と並ぶ。
        let t = tree(&e);
        let acts = next_actions(&t, 10);
        assert!(acts.iter().any(|x| x.block.as_deref() == Some("sci-review") && x.why == "UNCHECKED"));
        assert_eq!(next_actions(&t, 2).len(), 2);
    }

    /// `nodes[]` の `pending` は、その Node に付いた PENDING の PROPOSAL の数。
    #[test]
    fn nodes_json_counts_pending_proposals() {
        let e = env("pending");
        let conn = e.db.connection();
        let q0 = q(&e, None, "Q0", None);
        let q1 = q(&e, Some(q0), "Q1", None);
        let o1 = o(&e, q0, "O");
        // Q1 宛ての PENDING が 2 件、Q0 宛てが 1 件。applied は数えない。
        store::insert_proposal(conn, None, None, "add_q", Some(q1), &json!({}), "auto", "pending").unwrap();
        store::insert_proposal(conn, None, None, "add_q", Some(q1), &json!({}), "auto", "pending").unwrap();
        store::insert_proposal(conn, None, None, "add_o", Some(q0), &json!({}), "auto", "pending").unwrap();
        store::insert_proposal(conn, None, None, "add_o", Some(o1), &json!({}), "auto", "applied").unwrap();
        let s = status(conn, &e.roots, &Defs::default()).unwrap();
        assert_eq!(s.pending_proposals, 3);
        let row = |id: i64| s.nodes.iter().find(|n| n.id == fmt_id(Kind::Q, id)).unwrap();
        assert_eq!(row(q1).pending, 2);
        assert_eq!(row(q0).pending, 1);
    }

    #[test]
    fn status_seed_type_and_mind() {
        let e = env("seed");
        // problem 型として top_Q を 3 つ置き、mind.md に見出しを 2 つ置く。
        let defs = crate::rt::defs::load(&e.roots.root);
        store::meta_set(e.db.connection(), "type", "problem").unwrap();
        for key in ["background", "target", "purpose"] {
            q(&e, None, key, Some(key));
        }
        fs::write(
            sai_paper_core::paths::mind_path(&e.roots.data_dir),
            "前書き\n## ORIGIN\nIESS の Hyps 脳波\n## CONSTRAINTS\n<!-- 消える -->\n症例は 20 例\n",
        )
        .unwrap();
        let s = status(e.db.connection(), &e.roots, &defs).unwrap();
        let seed = &s.seed;
        assert_eq!(seed.type_info.as_ref().map(|t| t.id.as_str()), Some("problem"));
        assert_eq!(seed.type_info.as_ref().map(|t| t.name.as_str()), Some("PROBLEM_DRIVEN"));
        assert_eq!(
            seed.mind.iter().map(|m| m.heading.as_str()).collect::<Vec<_>>(),
            ["ORIGIN", "CONSTRAINTS"]
        );
        assert_eq!(seed.mind[0].body, "IESS の Hyps 脳波");
        assert_eq!(seed.mind[1].body, "症例は 20 例");
        // top_Q は 3 つで、一番上の Q（型の root）は無い。
        let tops: Vec<&str> = s
            .nodes
            .iter()
            .filter(|n| n.kind == "Q" && n.parent.is_none())
            .map(|n| n.guide.as_deref().unwrap_or_default())
            .collect();
        assert_eq!(tops, ["background", "target", "purpose"]);
    }

    #[test]
    fn block_warnings_reports_bad_sci_ops() {
        let e = env("warn");
        // 古い範囲のキーが残っていても読まない（名前は文字列を分けて組み立てる）。
        let old_scope = format!("sci_{}", "scope");
        fs::write(
            e.dir.join(".sai/blocks/x.md"),
            format!("---\nname: x\nsci_ops: add_o, star, foo\n{old_scope}: wide\n---\n本文\n"),
        )
        .unwrap();
        let lines = block_def_warnings(&e.roots);
        let mine: Vec<&String> = lines.iter().filter(|l| l.starts_with("x:")).collect();
        assert_eq!(
            mine,
            vec![
                "x: sci_ops に利用者だけの op があります: star",
                "x: sci_ops に知らない op があります: foo",
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
            want(&["initialized", "legacy", "type", "progress", "nodes", "seed", "next", "pending_proposals", "decisions", "running", "warnings"])
        );

        let e = env("keys");
        figure(&e);
        let s = status(e.db.connection(), &e.roots, &Defs::default()).unwrap();
        let v = serde_json::to_value(&s).unwrap();
        assert_eq!(
            keys(&v["nodes"][0]),
            want(&["id", "kind", "parent", "depth", "ord", "text", "body", "guide", "answer_kind", "starred", "on_path", "bottom_q", "paths", "objectivity", "logicality", "counts", "cites", "paper", "run", "mlflow", "created_by", "created_at", "pending"])
        );
        assert_eq!(keys(&v["next"][0]), want(&["kind", "node", "option", "block", "purpose", "expect", "why", "card"]));
        let d = node_detail(e.db.connection(), &e.roots, &Defs::default(), "Q2").unwrap();
        let dv = serde_json::to_value(&d).unwrap();
        assert_eq!(
            keys(&dv),
            want(&["node", "path", "children", "subtree", "guide", "logic", "decisions", "actions", "creator", "pending", "next", "searches", "sources", "history"])
        );
        assert!(d.next.len() <= 3);
        assert!(matches!(
            node_detail(e.db.connection(), &e.roots, &Defs::default(), "Q999"),
            Err(RtError::NotFound(m)) if m == "Q999 はありません"
        ));
    }

    #[test]
    fn node_detail_creator_sources_and_action_node() {
        let e = env("detail2");
        let conn = e.db.connection();
        conn.execute("INSERT INTO papers (id, title) VALUES (?1, ?2)", params![54i64, "GABA 系の study"]).unwrap();
        let q0 = q(&e, None, "Q", None);
        let o1 = o(&e, q0, "O");
        // cites を持つ F。run:7 が作ったことにする。
        let n = store::NewNode {
            kind: Kind::F, parent_id: Some(o1), guide_key: None, text: "根拠", body: "", answer_kind: None,
            cites: &["#54.11.3".to_string()], paper_id: None, run_id: Some(7), report_id: None, mlflow_ref: None, proposal_id: None,
            paths: &[], created_by: "run:7",
        };
        let f1 = store::insert_node(conn, &n).unwrap();
        // 対象が O の sci の run。global_round は 2。
        let dir = e.dir.join(".sai/outputs/R2/7");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("task.md"),
            &format!(
                "---\nrunner: pi\nblock: sci-read\nglobal_round: 2\nround: 3\nexecution_date: 2026/10/02 09:16:54\nsci_node: {}\nsci_purpose: 読む\n---\n本文\n",
                fmt_id(Kind::O, o1)
            ),
        )
        .unwrap();

        // F の詳細：作成者が run:7。block・owner が入る。出典は論文の題だけ分かる（節の表が無いので null）。
        let d = node_detail(conn, &e.roots, &Defs::default(), &fmt_id(Kind::F, f1)).unwrap();
        assert_eq!(d.creator.by, "run:7");
        assert_eq!(d.creator.run_id, Some(7));
        assert_eq!(d.creator.block, "sci-read");
        assert_eq!(d.creator.owner, "R2");
        assert_eq!(d.creator.global_round, Some(2));
        assert_eq!(d.creator.node.as_deref(), Some(fmt_id(Kind::O, o1).as_str()));
        assert_eq!(d.creator.purpose, "読む");
        assert_eq!(d.sources.len(), 1);
        assert_eq!(d.sources[0].cite, "#54.11.3");
        assert_eq!(d.sources[0].paper, Some(54));
        assert_eq!(d.sources[0].title, "GABA 系の study");
        assert_eq!(d.sources[0].section, None);
        assert_eq!(d.sources[0].passage, None);
        // run の対象は親の O1 なので、F の actions には入らない。
        assert!(d.actions.is_empty());

        // O1 の詳細：init の行は user（round なし）、Run の行には task.md の round が出る。
        let d = node_detail(conn, &e.roots, &Defs::default(), &fmt_id(Kind::O, o1)).unwrap();
        assert_eq!(d.creator.by, "user");
        assert_eq!(d.creator.run_id, None);
        assert_eq!(d.creator.block, "");
        assert_eq!(d.creator.global_round, None);
        assert_eq!(d.actions.len(), 1);
        assert_eq!(d.actions[0].node, Some(fmt_id(Kind::O, o1)));
        assert_eq!(d.actions[0].global_round, Some(2));
        // cites が無いので出典は空。
        assert!(d.sources.is_empty());
        assert_eq!(d.history[0].action, "init");
        assert_eq!(d.history[0].round, None);
        let run_row = d.history.iter().find(|h| h.action == "Run").unwrap();
        assert_eq!(run_row.by, "run:7");
        assert_eq!(run_row.round, Some(3));
        assert_eq!(run_row.global_round, Some(2));
        // F1 の init の行は run:7 が作ったので round が出る。
        let d = node_detail(conn, &e.roots, &Defs::default(), &fmt_id(Kind::F, f1)).unwrap();
        assert_eq!(d.history[0].action, "init");
        assert_eq!(d.history[0].run_id, Some(7));
        assert_eq!(d.history[0].round, Some(3));
        assert_eq!(d.history[0].block, "sci-read");
    }

    #[test]
    fn node_detail_history_init_first() {
        let e = env("history");
        let conn = e.db.connection();
        let q0 = q(&e, None, "Q", None);
        let o1 = o(&e, q0, "O");
        let f1 = f(&e, o1);
        // `rt_log` に `edit` を 1 行入れる（created_at は DB の UTC の形）。
        store::insert_log(conn, f1, "edit", "text", "user", None).unwrap();
        let d = node_detail(conn, &e.roots, &Defs::default(), &fmt_id(Kind::F, f1)).unwrap();
        assert_eq!(d.history[0].action, "init");
        assert!(d.history[0].at.ends_with("+00:00"));
        assert!(d.history.iter().any(|h| h.action == "edit" && h.by == "user"));
        // O の詳細：子の F の Add 行と、edit の行は無い（edit は F の行）。at は昇順。
        let d = node_detail(conn, &e.roots, &Defs::default(), &fmt_id(Kind::O, o1)).unwrap();
        assert_eq!(d.history[0].action, "init");
        assert!(d.history.iter().any(|h| h.action == "Add F"));
        let ats: Vec<&str> = d.history.iter().skip(1).map(|h| h.at.as_str()).collect();
        assert!(ats.windows(2).all(|w| w[0] <= w[1]), "{ats:?}");
    }

    #[test]
    fn sprout_node_and_paths() {
        // Q1 → O2(Issue)。SPROUT S の paths は [O2]。E は S の下に置く。
        let e = env("sprout");
        let conn = e.db.connection();
        let q1 = q(&e, None, "Q1", None);
        let o2 = add(&e, Kind::O, Some(q1), "課題", None, Some("issue"));
        let s = store::insert_node(
            conn,
            &store::NewNode {
                kind: Kind::S,
                parent_id: None,
                guide_key: None,
                text: "種",
                body: "本文",
                answer_kind: None,
                cites: &[],
                paper_id: None,
                run_id: None,
                report_id: None,
                mlflow_ref: None,
                proposal_id: None,
                paths: &[o2],
                created_by: "user",
            },
        )
        .unwrap();
        let e3 = add(&e, Kind::E, Some(s), "実験", None, None);
        add(&e, Kind::F, Some(e3), "根拠", None, None);

        let st = status(conn, &e.roots, &Defs::default()).unwrap();
        let srow = st.nodes.iter().find(|n| n.id == fmt_id(Kind::S, s)).unwrap();
        assert_eq!(srow.kind, "S");
        assert!(srow.parent.is_none());
        assert_eq!(srow.paths, vec![fmt_id(Kind::O, o2)]);
        assert!(srow.on_path);
        // E の親は S。道の上にあり、S の下に並ぶ。
        let erow = st.nodes.iter().find(|n| n.id == fmt_id(Kind::E, e3)).unwrap();
        assert_eq!(erow.parent.as_deref(), Some(fmt_id(Kind::S, s).as_str()));
        assert!(erow.on_path);
        // O2 は SPROUT に入っているので NO_EXPERIMENT は出ない。
        assert!(!next_actions(&tree(&e), 10).iter().any(|a| a.why == "NO_EXPERIMENT"));
        // node_detail で S を見られる（path は S だけ、子は下の E）。
        let d = node_detail(conn, &e.roots, &Defs::default(), &fmt_id(Kind::S, s)).unwrap();
        assert_eq!(d.path.len(), 1);
        assert_eq!(d.path[0].id, fmt_id(Kind::S, s));
        assert_eq!(d.children.len(), 1);
    }
}
