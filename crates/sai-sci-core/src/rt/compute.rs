//! research-tree の計算（Node の数・SPROUT の範囲・SEED の下書き・Next Action・注意）と、
//! `status` / `node` の JSON（C-JSON）。CLI・Web・TUI はこの結果を写すだけにする。

use std::collections::{BTreeSet, HashMap, HashSet};
use std::fs;
use std::path::Path;

use chrono::{DateTime, NaiveDateTime, Utc};
use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;
use serde_json::{Value, json};

use super::defs::Defs;
use super::store::{self, LogRow, NodeRow};
use super::{Kind, Result, RtError, action, fmt_id, ops, parse_id};
use sai_paper_core::paths::PaperRoots;
use crate::runs::read_frontmatter;

/// 古い形の research-tree だけがあるときの注意。
const LEGACY_WARNING: &str =
    "古い形の research-tree です。`sai-sci init --type <型>` で作り直してください";

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

/// SPROUT の範囲（C5）。Q・A の数と、F の立場ごとの数。
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Range {
    pub q: i64,
    pub a: i64,
    pub f_support: i64,
    pub f_deny: i64,
    pub f_issue: i64,
}

impl Range {
    /// `Q 2・A 3・F 4（S 1 / D 2 / I 1）` の形（人が読む文）。
    #[must_use]
    pub fn line(&self) -> String {
        format!("Q {}・A {}・{}", self.q, self.a, f_line(self.f_support, self.f_deny, self.f_issue))
    }
}

/// F の立場ごとの数を `F 4（S 1 / D 2 / I 1）` の形にする（S は支持、D は反対、I は課題）。
fn f_line(support: i64, deny: i64, issue: i64) -> String {
    format!("F {}（S {support} / D {deny} / I {issue}）", support + deny + issue)
}

/// 直の子の種類ごとの数。
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize)]
pub struct Counts {
    pub q: i64,
    pub a: i64,
    pub f: i64,
    pub e: i64,
}

/// `status.counts`。生きた Node の総数（種類と F の立場ごと）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize)]
pub struct StatusCounts {
    pub q: i64,
    pub a: i64,
    pub f_support: i64,
    pub f_deny: i64,
    pub f_issue: i64,
    pub e: i64,
    pub s: i64,
}

impl StatusCounts {
    /// `Q 3・A 2・F 4（S 1 / D 2 / I 1）・E 0・S 1` の形（`state.md`・`tree.md`・`sci status` の文）。
    #[must_use]
    pub fn line(&self) -> String {
        format!(
            "Q {}・A {}・{}・E {}・S {}",
            self.q,
            self.a,
            f_line(self.f_support, self.f_deny, self.f_issue),
            self.e,
            self.s
        )
    }
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
    /// F の立場（F 以外は null）。
    pub stance: Option<String>,
    pub starred: bool,
    pub bottom_q: bool,
    /// SPROUT（kind S）の PATH（A の ID）。S 以外は空。
    pub paths: Vec<String>,
    /// SPROUT の範囲（S だけ。他は null）。
    pub range: Option<Range>,
    pub counts: Counts,
    pub cites: Vec<String>,
    pub paper: Option<i64>,
    pub run: Option<i64>,
    /// MLflow の結び付け（E だけ）。
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
    /// node_detail で「この Node が対象」を選ぶための Node（JSON には出さない）。
    #[serde(skip)]
    targets: Vec<i64>,
}

/// DECISIONS 1 件。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Decision {
    pub node: String,
    pub node_text: String,
    pub action: String,
    pub detail: String,
    pub actor: String,
    pub at: String,
    pub approved_by: Option<String>,
    pub qid: Option<String>,
    pub question: Option<String>,
}

/// 走っている run。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RunningJson {
    pub run_id: i64,
    pub node: Option<String>,
    pub block: String,
    pub owner: String,
}

/// RESEARCH_TYPE の id と名前。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TypeJson {
    pub id: String,
    pub name: String,
}

/// 木の型と mind.md（RESEARCH_TYPE と mind.md の見出し）。
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct ProjectJson {
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
    pub counts: StatusCounts,
    pub nodes: Vec<NodeJson>,
    /// 木の型と mind.md（`status --json` の `project`）。
    pub project: ProjectJson,
    pub next: Vec<NextAction>,
    pub decisions: Vec<Decision>,
    /// 全 Node の History（`Add *` を除き、`at` の昇順）。
    pub history: Vec<StatusHistory>,
    /// 時系列（始めているときだけ。`steps.rs`）。
    pub steps: super::steps::StepsJson,
    pub running: Vec<RunningJson>,
    pub warnings: Vec<String>,
}

/// `status.history` の 1 行。`node_detail.history` の 1 行に、Node の ID と文を付けたもの。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct StatusHistory {
    /// `Q3` など。
    pub node: String,
    /// その Node の文。
    pub node_text: String,
    #[serde(flatten)]
    pub entry: HistoryJson,
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
}

/// `node_detail.actions` の 1 要素。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ActionJson {
    pub run_id: i64,
    pub block: String,
    pub owner: String,
    /// この run の対象の Node（`A12` など）。
    pub node: Option<String>,
    /// task.md の global_round（無ければ null）。
    pub global_round: Option<i64>,
    pub purpose: String,
    pub occurred_at: String,
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
    /// その run の対象の Node（F を作った run なら親の A など）。
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
    /// `init` / `Add Q` / `Add A` / `Add F` / `Add E` / `Run` / rt_log の action（`edit` など）
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
    /// 承認を使った行の承認した人（無ければ null）。
    pub approved_by: Option<String>,
    /// 承認の質問の ID（無ければ null）。
    pub qid: Option<String>,
    /// 承認の質問の文（無ければ null）。
    pub question: Option<String>,
}

/// `node` の JSON。
#[derive(Debug, Clone, Serialize)]
pub struct NodeDetail {
    pub node: NodeJson,
    pub path: Vec<PathItem>,
    pub children: Vec<NodeJson>,
    pub subtree: Vec<NodeJson>,
    pub guide: Option<GuideJson>,
    pub decisions: Vec<Decision>,
    pub actions: Vec<ActionJson>,
    pub creator: CreatorJson,
    pub next: Vec<NextAction>,
    pub searches: Vec<SearchJson>,
    pub sources: Vec<SourceJson>,
    pub history: Vec<HistoryJson>,
}

// ---- research-tree -------------------------------------------------------------------------------

/// research-tree を 1 回読んで、計算を使い回す。
pub struct Tree {
    defs: Defs,
    type_id: Option<String>,
    nodes: Vec<NodeRow>,
    index: HashMap<i64, usize>,
    kids: HashMap<i64, Vec<i64>>,
    depths: HashMap<i64, usize>,
    f_under: HashMap<i64, i64>,
    /// SPROUT（S）ごとの範囲。
    sprout_ranges: HashMap<i64, Range>,
    /// 対象の Node ごとの、一番新しい `sci-verify` の run の時刻（Next Action の規則 3）。
    verified: HashMap<i64, DateTime<Utc>>,
    log: Vec<LogRow>,
    /// 消した Node も含む id → (kind, 文)。DECISIONS の表示に使う。
    labels: HashMap<i64, (Kind, String)>,
}

fn live_node(conn: &Connection, id: i64) -> Option<NodeRow> {
    store::get_node(conn, id).ok().filter(|n| n.removed_at.is_none())
}

/// SPROUT の範囲。`paths` の各 A について、その A・A の祖先の Q と A・A の子の生きた F を集め（重ならないように数える）、
/// Q・A の数と F の立場ごとの数を出す。A の子の Q 以下は入れない。消えた A は数えない。
pub fn sprout_range(conn: &Connection, paths: &[i64]) -> Result<Range> {
    let mut ids: BTreeSet<i64> = BTreeSet::new();
    for &a in paths {
        let Some(node) = live_node(conn, a) else { continue };
        ids.insert(a);
        let mut at = node.parent_id;
        while let Some(p) = at {
            let Some(parent) = live_node(conn, p) else { break };
            if matches!(parent.kind, Kind::Q | Kind::A) {
                ids.insert(p);
            }
            at = parent.parent_id;
        }
        for f in store::children(conn, a)?.into_iter().filter(|k| k.kind == Kind::F) {
            ids.insert(f.id);
        }
    }
    let mut range = Range::default();
    for id in ids {
        let Some(n) = live_node(conn, id) else { continue };
        match (n.kind, n.stance.as_deref()) {
            (Kind::Q, _) => range.q += 1,
            (Kind::A, _) => range.a += 1,
            (Kind::F, Some("support")) => range.f_support += 1,
            (Kind::F, Some("deny")) => range.f_deny += 1,
            (Kind::F, _) => range.f_issue += 1,
            _ => {}
        }
    }
    Ok(range)
}

impl Tree {
    /// research-tree を読む。
    pub fn load(conn: &Connection, roots: &PaperRoots, defs: &Defs) -> Result<Tree> {
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
        let mut kids: HashMap<i64, Vec<i64>> = HashMap::new();
        let roots_none: Vec<NodeRow> = by_parent.remove(&None).unwrap_or_default();
        let mut stack: Vec<(NodeRow, usize)> = Vec::new();
        // 親の無い生きた Q（top_Q）→ 親の無い D（seed）→ 親の無い S（古い SPROUT）の順に DFS の起点にする。
        // D の子の S は DFS で D の下に入る。`roots_none` は `(parent_id, ord, id)` 順なので、各群の中もこの順のまま。
        let mut top: Vec<NodeRow> = Vec::new();
        top.extend(roots_none.iter().filter(|n| n.kind == Kind::Q).cloned());
        top.extend(roots_none.iter().filter(|n| n.kind == Kind::D).cloned());
        top.extend(roots_none.iter().filter(|n| n.kind == Kind::S).cloned());
        for n in top.into_iter().rev() {
            stack.push((n, 0));
        }
        while let Some((n, depth)) = stack.pop() {
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

        let sprouts: Vec<(i64, Vec<i64>)> =
            nodes.iter().filter(|n| n.kind == Kind::S).map(|n| (n.id, n.paths.clone())).collect();
        let mut sprout_ranges = HashMap::new();
        for (id, paths) in sprouts {
            sprout_ranges.insert(id, sprout_range(conn, &paths)?);
        }

        let mut verified: HashMap<i64, DateTime<Utc>> = HashMap::new();
        for run in action::sci_runs(roots).iter().filter(|r| r.block == "sci-verify") {
            let (Some((_, id)), Some(at)) = (run.keys.node, utc_at(&run.occurred_at)) else { continue };
            let slot = verified.entry(id).or_insert(at);
            *slot = (*slot).max(at);
        }

        Ok(Tree {
            defs: defs.clone(),
            type_id: store::meta_get(conn, "type")?,
            nodes,
            index,
            kids,
            depths,
            f_under,
            sprout_ranges,
            verified,
            log: store::list_log(conn, None)?,
            labels,
        })
    }

    fn kid_nodes(&self, id: i64) -> impl Iterator<Item = &NodeRow> {
        self.kids.get(&id).into_iter().flatten().map(|k| &self.nodes[self.index[k]])
    }

    /// 生きた Node（research-tree の順）。
    #[must_use]
    pub fn nodes(&self) -> &[NodeRow] {
        &self.nodes
    }

    /// 生きた Node 1 つ。
    #[must_use]
    pub fn node(&self, id: i64) -> Option<&NodeRow> {
        self.index.get(&id).map(|i| &self.nodes[*i])
    }

    /// 深さ（top_Q が 0）。
    #[must_use]
    pub fn depth(&self, id: i64) -> usize {
        self.depths.get(&id).copied().unwrap_or(0)
    }

    /// 生きた bottom_Q か（生きた子の Q が無い Q）。
    #[must_use]
    pub fn is_bottom_q(&self, id: i64) -> bool {
        self.node(id).is_some_and(|n| n.kind == Kind::Q)
            && !self.kid_nodes(id).any(|k| k.kind == Kind::Q)
    }

    /// 子孫の生きた F の数。
    #[must_use]
    pub fn f_count_under(&self, id: i64) -> i64 {
        self.f_under.get(&id).copied().unwrap_or(0)
    }

    /// SPROUT（S）の範囲。S 以外や無いときは `None`。
    #[must_use]
    pub fn range(&self, id: i64) -> Option<&Range> {
        self.sprout_ranges.get(&id)
    }

    /// 生きた Node の総数（`status.counts`）。
    #[must_use]
    pub fn counts(&self) -> StatusCounts {
        let mut c = StatusCounts::default();
        for n in &self.nodes {
            match (n.kind, n.stance.as_deref()) {
                (Kind::Q, _) => c.q += 1,
                (Kind::A, _) => c.a += 1,
                (Kind::F, Some("support")) => c.f_support += 1,
                (Kind::F, Some("deny")) => c.f_deny += 1,
                (Kind::F, _) => c.f_issue += 1,
                (Kind::E, _) => c.e += 1,
                (Kind::S, _) => c.s += 1,
                // seed（D）は数に入れない。
                (Kind::D, _) => {}
            }
        }
        c
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
                Kind::A => counts.a += 1,
                Kind::F => counts.f += 1,
                Kind::E => counts.e += 1,
                // SPROUT（S）は Q・A・F・E の数のどれにも入れない。
                Kind::S => {}
                // seed（D）の子は数えない（画面の表示は別の作業で直す）。
                Kind::D => {}
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
            stance: n.stance.clone(),
            starred: n.starred,
            bottom_q: is_q && self.is_bottom_q(id),
            paths: if n.kind == Kind::S {
                n.paths.iter().map(|a| self.label(*a).0).collect()
            } else {
                Vec::new()
            },
            range: self.sprout_ranges.get(&id).cloned(),
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
            actor: row.actor.clone(),
            at: row.created_at.clone(),
            approved_by: row.approved_by.clone(),
            qid: row.qid.clone(),
            question: row.question.clone(),
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

/// Next Action 1 件（C6）。`child` は規則 2 だけが足す子の型（`F`）。
fn card_action(node: &NodeRow, block: &str, purpose: String, expect: &str, why: &str, child: Option<&str>) -> NextAction {
    let card_node = fmt_id(node.kind, node.id);
    let mut card = json!({"node": card_node, "block": block, "purpose": purpose, "expect": expect, "runner": "cc"});
    if let Some(child) = child {
        card["child"] = json!(child);
    }
    NextAction {
        kind: "card".into(),
        node: Some(card_node),
        option: None,
        block: Some(block.into()),
        card: Some(card),
        purpose,
        expect: expect.into(),
        why: why.into(),
        targets: vec![node.id],
    }
}

impl Tree {
    /// Node 1 つに出す Next Action（C6 の 3 規則の最初に当てはまる 1 つ）。
    fn action_of(&self, n: &NodeRow) -> Option<(u8, NextAction)> {
        match n.kind {
            Kind::Q if self.kid_nodes(n.id).next().is_none() => Some((
                1,
                card_action(n, "lab-grow", format!("「{}」の子（A か child_Q）を作る", n.text), "子を 1 つ以上", "NO_CHILD", None),
            )),
            // ★の A は先頭に出す（rank 0）。
            Kind::A if !self.kid_nodes(n.id).any(|k| k.kind == Kind::F) => Some((
                if n.starred { 0 } else { 2 },
                card_action(n, "lab-search", format!("「{}」の F を探す", n.text), "F を 1 つ以上", "NO_FINDING", Some("F")),
            )),
            Kind::Q | Kind::A if self.unverified(n) => Some((
                3,
                card_action(n, "sci-verify", format!("「{}」と子の関係を確かめる", n.text), "評価の報告と op", "UNVERIFIED", None),
            )),
            _ => None,
        }
    }

    /// 生きた子が 1 つ以上あり、`sci-verify` の run がその Node を対象にしたことが無い、
    /// または一番新しい子の `created_at` が一番新しい検証の時刻より後（規則 3）。
    fn unverified(&self, n: &NodeRow) -> bool {
        let kids: Vec<&NodeRow> = self.kid_nodes(n.id).collect();
        if kids.is_empty() {
            return false;
        }
        match self.verified.get(&n.id) {
            None => true,
            Some(run) => kids.iter().filter_map(|k| utc_at(&k.created_at)).max().is_some_and(|child| child > *run),
        }
    }
}

/// Next Action（順の番号 → research-tree の順）を `limit` 件まで。
#[must_use]
pub fn next_actions(tree: &Tree, limit: usize) -> Vec<NextAction> {
    let mut ranked: Vec<(u8, usize, NextAction)> = Vec::new();
    for (pos, n) in tree.nodes.iter().enumerate() {
        if let Some((rank, a)) = tree.action_of(n) {
            ranked.push((rank, pos, a));
        }
    }
    ranked.sort_by_key(|(rank, pos, _)| (*rank, *pos));
    ranked.into_iter().map(|(_, _, a)| a).take(limit).collect()
}

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

/// DB の時刻（`YYYY-MM-DD HH:MM:SS`、UTC）や RFC3339 の時刻を UTC の RFC3339 に直す。読めなければ元の文字のまま。
pub(super) fn rfc3339_utc(text: &str) -> String {
    let text = text.trim();
    if let Ok(naive) = NaiveDateTime::parse_from_str(text, "%Y-%m-%d %H:%M:%S") {
        return DateTime::<Utc>::from_naive_utc_and_offset(naive, Utc).to_rfc3339();
    }
    if let Ok(dt) = DateTime::parse_from_rfc3339(text) {
        return dt.with_timezone(&Utc).to_rfc3339();
    }
    text.to_string()
}

/// 時刻の文（DB の UTC か RFC3339）を UTC の時刻にする。読めなければ `None`。
fn utc_at(text: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(&rfc3339_utc(text)).ok().map(|at| at.with_timezone(&Utc))
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
            node: r.keys.node.map(|(k, id)| fmt_id(k, id)),
            block: r.block,
            owner: r.owner,
        })
        .collect()
}

// ---- status / node_detail -------------------------------------------------------------

/// `status` の JSON（Next Action は 10 件まで）。
pub fn status(conn: &Connection, roots: &PaperRoots, defs: &Defs) -> Result<Status> {
    status_with_limit(conn, roots, defs, 10)
}

/// `status` の JSON。Next Action は `limit` 件まで。
pub fn status_with_limit(conn: &Connection, roots: &PaperRoots, defs: &Defs, limit: usize) -> Result<Status> {
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
        counts: StatusCounts::default(),
        nodes: Vec::new(),
        project: ProjectJson::default(),
        next: Vec::new(),
        decisions: Vec::new(),
        history: Vec::new(),
        steps: super::steps::StepsJson::default(),
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
                out.warnings.push(format!("知らない RESEARCH_TYPE です: {id}"));
            }
        }
    }
    out.counts = tree.counts();
    out.nodes = tree.nodes.iter().map(|n| tree.row(n.id)).collect();
    out.project = ProjectJson {
        type_info: out.type_info.clone(),
        mind: mind_sections(&roots.data_dir)
            .into_iter()
            .map(|(heading, body)| MindSection { heading, body })
            .collect(),
    };
    out.next = next_actions(&tree, limit);
    out.decisions = tree.log.iter().take(30).map(|r| tree.decision(r)).collect();
    // 全 Node の History。`Add *` は子の `init` と同じ出来事なので捨てる。at の安定ソートで古い順にする。
    let runs = action::sci_runs(roots);
    for n in &tree.nodes {
        for entry in history_json(n, n.id, &tree, &runs) {
            if entry.action.starts_with("Add ") {
                continue;
            }
            out.history.push(StatusHistory { node: fmt_id(n.kind, n.id), node_text: n.text.clone(), entry });
        }
    }
    out.history.sort_by(|a, b| a.entry.at.cmp(&b.entry.at));
    out.steps = super::steps::build(conn)?;
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
        entry: HistoryJson,
    }
    let plain = |action: String, at: String, by: String, run_id: Option<i64>, run: Option<&action::SciRun>| {
        let (block, global_round, round) = fill(run);
        HistoryJson {
            action,
            at,
            by,
            run_id,
            block,
            global_round,
            round,
            approved_by: None,
            qid: None,
            question: None,
        }
    };
    let mut rows: Vec<Row> = Vec::new();
    let mut pos = 0;
    // Add: この Node の生きている直の子ごとに 1 行。
    for kid in tree.kids.get(&num).into_iter().flatten() {
        let Some(k) = tree.node(*kid) else { continue };
        let by = k.created_by.clone();
        let run_id = run_id_of(&by);
        let entry = plain(format!("Add {}", k.kind.as_str()), rfc3339_utc(&k.created_at), by, run_id, run_of(run_id));
        rows.push(Row { rank: 0, at: entry.at.clone(), pos, entry });
        pos += 1;
    }
    // Run: この Node を対象にする run ごとに 1 行（子孫の run は入れない）。
    for r in runs.iter().filter(|r| r.keys.node.is_some_and(|(_, n)| n == num)) {
        let entry = HistoryJson {
            action: "Run".into(),
            at: rfc3339_utc(&r.occurred_at),
            by: format!("run:{}", r.run_id),
            run_id: Some(r.run_id),
            block: r.block.clone(),
            global_round: r.global_round,
            round: r.round,
            approved_by: None,
            qid: None,
            question: None,
        };
        rows.push(Row { rank: 1, at: entry.at.clone(), pos, entry });
        pos += 1;
    }
    // 決定: `rt_log` のうちこの Node の行ごとに 1 行。
    for row in tree.log.iter().filter(|r| r.node_id == num) {
        let run_id = run_id_of(&row.actor);
        let mut entry = plain(row.action.clone(), rfc3339_utc(&row.created_at), row.actor.clone(), run_id, run_of(run_id));
        entry.approved_by = row.approved_by.clone();
        entry.qid = row.qid.clone();
        entry.question = row.question.clone();
        rows.push(Row { rank: 2, at: entry.at.clone(), pos, entry });
        pos += 1;
    }
    rows.sort_by(|a, b| a.at.cmp(&b.at).then(a.rank.cmp(&b.rank)).then(a.pos.cmp(&b.pos)));
    // init: 必ず 1 行・先頭。
    let by = node.created_by.clone();
    let run_id = run_id_of(&by);
    let init = plain("init".into(), rfc3339_utc(&node.created_at), by, run_id, run_of(run_id));
    let mut out = vec![init];
    out.extend(rows.into_iter().map(|r| r.entry));
    out
}

/// `node` の JSON。
pub fn node_detail(conn: &Connection, roots: &PaperRoots, defs: &Defs, id: &str) -> Result<NodeDetail> {
    if super::is_retired_seed(id) {
        return Err(RtError::Invalid(super::RETIRED_SEED_MSG.to_string()));
    }
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

    // 自分を含む子孫（research-tree の順）。
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
        }),
        decisions: tree.log.iter().filter(|r| family_set.contains(&r.node_id)).take(30).map(|r| tree.decision(r)).collect(),
        actions,
        creator,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake_link;
    use sai_paper_core::database::Database;

    struct Env {
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
        Env { roots: PaperRoots { root: dir, data_dir }, db }
    }

    fn add(e: &Env, kind: Kind, parent: Option<i64>, text: &str, stance: Option<&str>) -> i64 {
        let n = store::NewNode {
            kind, parent_id: parent, guide_key: None, text, body: "", stance, cites: &[],
            paper_id: None, run_id: None, report_id: None, mlflow_ref: None, paths: &[], created_by: "user",
        };
        store::insert_node(e.db.connection(), &n).unwrap()
    }

    #[test]
    fn steps_stay_empty_before_init() {
        // 始めていない DB では時系列を作らない（Node があっても total は 0）。
        let e = env("steps-uninit");
        q(&e, None, "根");
        e.db.connection().execute("DELETE FROM rt_meta WHERE key = 'initialized_at'", []).unwrap();
        let s = status(e.db.connection(), &e.roots, &Defs::default()).unwrap();
        assert!(!s.initialized);
        assert_eq!(s.steps.total, 0);
        assert!(s.steps.nodes.is_empty());
    }

    fn q(e: &Env, parent: Option<i64>, text: &str) -> i64 {
        add(e, Kind::Q, parent, text, None)
    }
    fn a(e: &Env, parent: i64, text: &str) -> i64 {
        add(e, Kind::A, Some(parent), text, None)
    }
    fn f(e: &Env, parent: i64, stance: &str) -> i64 {
        add(e, Kind::F, Some(parent), "根拠", Some(stance))
    }

    fn tree(e: &Env) -> Tree {
        Tree::load(e.db.connection(), &e.roots, &Defs::default()).unwrap()
    }

    /// 3 規則の例: Q1 ─ A2（F なし）・Q3（子なし）。`sci-verify` の run は `verified` で足す。
    fn next_fixture(e: &Env) -> (i64, i64, i64) {
        let q1 = q(e, None, "根の問い");
        let a2 = a(e, q1, "答え");
        let q3 = q(e, None, "別の問い");
        (q1, a2, q3)
    }

    /// `sci-verify` の run を 1 つ作る（`creation_date` は local 時刻の書き方）。一覧に出す値を返す。
    fn verify_run(e: &Env, run: i64, node: i64, created: &str) -> serde_json::Value {
        let dir = e.roots.root.join(format!(".sai/outputs/review/{run}"));
        fs::create_dir_all(&dir).unwrap();
        let text = format!(
            "---\nrunner: pi\nblock: sci-verify\nsci_node: {}\nsci_purpose: 確かめる\ncreation_date: {created}\n---\n\n本文\n",
            fmt_id(Kind::Q, node)
        );
        fs::write(dir.join("task.md"), text).unwrap();
        fake_link::run_value(run, "review", &dir, "sci-verify", "", "completed")
    }

    #[test]
    fn next_actions_three_rules() {
        let e = env("three");
        let (q1, a2, q3) = next_fixture(&e);
        let acts = next_actions(&tree(&e), 10);
        let got: Vec<(String, String, String)> = acts
            .iter()
            .map(|x| (x.why.clone(), x.node.clone().unwrap_or_default(), x.block.clone().unwrap_or_default()))
            .collect();
        let want = vec![
            ("NO_CHILD".to_string(), fmt_id(Kind::Q, q3), "lab-grow".to_string()),
            ("NO_FINDING".to_string(), fmt_id(Kind::A, a2), "lab-search".to_string()),
            ("UNVERIFIED".to_string(), fmt_id(Kind::Q, q1), "sci-verify".to_string()),
        ];
        assert_eq!(got, want);
        // 規則 2 だけが child を足す。
        assert_eq!(acts[1].card.as_ref().unwrap()["child"], "F");
        assert!(acts[0].card.as_ref().unwrap().get("child").is_none());
        assert_eq!(acts[0].card.as_ref().unwrap()["runner"], "cc");
    }

    /// ★の A の NO_FINDING は先頭（rank 0）、★の無い A の NO_FINDING は NO_CHILD の Q の後。
    /// `limit` で件数を絞り、`usize::MAX`（`--limit 0`）は全部出す。
    #[test]
    fn starred_no_finding_first_and_limit_cuts() {
        let e = env("starred");
        let qa = q(&e, None, "根の問い");
        let a_plain = a(&e, qa, "普通の答え");
        let a_star = a(&e, qa, "★の答え");
        let q_leaf = q(&e, None, "子の無い問い");
        store::set_starred(e.db.connection(), a_star, true).unwrap();

        let whole = |limit: usize| status_with_limit(e.db.connection(), &e.roots, &Defs::default(), limit).unwrap().next;
        let got: Vec<(String, String)> = whole(usize::MAX).iter().map(|x| (x.why.clone(), x.node.clone().unwrap_or_default())).collect();
        assert_eq!(
            got,
            vec![
                ("NO_FINDING".to_string(), fmt_id(Kind::A, a_star)),
                ("NO_CHILD".to_string(), fmt_id(Kind::Q, q_leaf)),
                ("NO_FINDING".to_string(), fmt_id(Kind::A, a_plain)),
                ("UNVERIFIED".to_string(), fmt_id(Kind::Q, qa)),
            ]
        );
        assert_eq!(whole(3).len(), 3);
        // 既定の `status` は 10 件まで。
        let default = status(e.db.connection(), &e.roots, &Defs::default()).unwrap();
        assert_eq!(default.next.len(), 4);
    }

    #[test]
    fn unverified_clears_only_after_newer_verify_run() {
        let e = env("verify");
        let (q1, a2, _) = next_fixture(&e);
        // 子（A2）は 10/1、検証は 9/30 → まだ確かめていない。
        e.db.connection().execute("UPDATE rt_nodes SET created_at = '2026-10-01 00:00:00' WHERE id = ?1", params![a2]).unwrap();
        let old = verify_run(&e, 91, q1, "2026/09/30 12:00:00");
        fake_link::write_runs(&e.roots.root, std::slice::from_ref(&old));
        crate::link::forget(&e.roots.root);
        assert!(next_actions(&tree(&e), 10).iter().any(|x| x.why == "UNVERIFIED"));
        // 検証を 10/2 に足すと、子より新しいので規則 3 は消える。
        let new = verify_run(&e, 92, q1, "2026/10/02 12:00:00");
        fake_link::write_runs(&e.roots.root, &[old, new]);
        crate::link::forget(&e.roots.root);
        assert!(!next_actions(&tree(&e), 10).iter().any(|x| x.why == "UNVERIFIED"));
    }

    #[test]
    fn status_counts_by_stance() {
        let e = env("counts");
        let root = q(&e, None, "根");
        let ans = a(&e, root, "答え");
        f(&e, ans, "support");
        f(&e, ans, "deny");
        f(&e, ans, "issue");
        add(&e, Kind::E, Some(ans), "実験", None);
        let s = status(e.db.connection(), &e.roots, &Defs::default()).unwrap();
        assert_eq!(
            s.counts,
            StatusCounts { q: 1, a: 1, f_support: 1, f_deny: 1, f_issue: 1, e: 1, s: 0 }
        );
        assert_eq!(s.counts.line(), "Q 1・A 1・F 3（S 1 / D 1 / I 1）・E 1・S 0");
    }

    #[test]
    fn status_json_has_new_keys_only() {
        let e = env("status");
        let root = q(&e, None, "根");
        let ans = a(&e, root, "答え");
        f(&e, ans, "support");
        let s = status(e.db.connection(), &e.roots, &Defs::default()).unwrap();
        let v = serde_json::to_value(&s).unwrap();
        assert_eq!(v["counts"]["q"], 1);
        assert_eq!(v["counts"]["a"], 1);
        assert_eq!(v["counts"]["f_support"], 1);
        for gone in ["progress", "pending_proposals", "seed"] {
            assert!(v.get(gone).is_none(), "{gone}");
        }
        // 木の型と mind.md は `project`（D の seed と取り違えないよう名前を変えた）。
        assert!(v["project"].get("type").is_some() && v["project"]["mind"].is_array(), "{v}");
        let nodes = v["nodes"].as_array().unwrap();
        let answer = nodes.iter().find(|n| n["kind"] == "A").unwrap();
        for gone in ["answer_kind", "objectivity", "logicality", "pending", "target"] {
            assert!(answer.get(gone).is_none(), "{gone}");
        }
        // 道の印（旧 `on_path`）も無い（名前は文字列を分けて組み立てる）。
        assert!(answer.get(format!("on_{}", "path")).is_none());
        assert!(answer["stance"].is_null());
        let fact = nodes.iter().find(|n| n["kind"] == "F").unwrap();
        assert_eq!(fact["stance"], "support");
        assert!(fact["range"].is_null());
    }

    #[test]
    fn sprout_range_counts_each_node_once() {
        let e = env("range");
        let root = q(&e, None, "根");
        let a1 = a(&e, root, "一つ目");
        let a2 = a(&e, root, "二つ目");
        f(&e, a1, "support");
        f(&e, a2, "deny");
        f(&e, a2, "issue");
        // A の子の Q 以下は数えない。
        q(&e, Some(a1), "子の問い");
        let s = add(&e, Kind::S, None, "種", None);
        let conn = e.db.connection();
        conn.execute("UPDATE rt_nodes SET paths = '[' || ?1 || ',' || ?2 || ']' WHERE id = ?3", params![a1, a2, s]).unwrap();
        let range = sprout_range(conn, &[a1, a2]).unwrap();
        assert_eq!(range, Range { q: 1, a: 2, f_support: 1, f_deny: 1, f_issue: 1 });
        let t = tree(&e);
        assert_eq!(t.row(s).range, Some(range));
        assert_eq!(t.row(s).paths, vec![fmt_id(Kind::A, a1), fmt_id(Kind::A, a2)]);
    }

    /// 廃止した `SEED` は node_detail が終了コード 2 の型（Invalid）で断り、案内を返す。
    #[test]
    fn node_detail_rejects_retired_seed() {
        let e = env("retired-seed");
        q(&e, None, "根");
        let err = node_detail(e.db.connection(), &e.roots, &Defs::default(), "SEED").unwrap_err();
        assert!(matches!(err, RtError::Invalid(ref m) if m == super::super::RETIRED_SEED_MSG), "{err}");
    }

    #[test]
    fn seed_nodes_sit_between_top_q_and_old_sprouts() {
        let e = env("seed-order");
        let root = q(&e, None, "根");
        let d1 = add(&e, Kind::D, None, "自分の研究", None);
        let s2 = add(&e, Kind::S, Some(d1), "一つ目", None);
        let s3 = add(&e, Kind::S, Some(d1), "二つ目", None);
        let d2 = add(&e, Kind::D, None, "#12 の研究", None);
        let s4 = add(&e, Kind::S, Some(d2), "三つ目", None);
        let old = add(&e, Kind::S, None, "古い研究", None);
        let s = status(e.db.connection(), &e.roots, &Defs::default()).unwrap();
        let ids: Vec<String> = s.nodes.iter().map(|n| n.id.clone()).collect();
        let want: Vec<String> = [(Kind::Q, root), (Kind::D, d1), (Kind::S, s2), (Kind::S, s3), (Kind::D, d2), (Kind::S, s4), (Kind::S, old)]
            .iter()
            .map(|(k, id)| fmt_id(*k, *id))
            .collect();
        assert_eq!(ids, want);
        let parents: Vec<Option<String>> = s.nodes.iter().map(|n| n.parent.clone()).collect();
        assert_eq!(parents[2], Some(fmt_id(Kind::D, d1)));
        assert_eq!(parents[6], None);
        assert_eq!(s.nodes[1].kind, "D");
        assert_eq!(s.nodes[1].text, "自分の研究");
        // D は数えない。
        assert_eq!(s.counts, StatusCounts { q: 1, s: 4, ..StatusCounts::default() });
    }

    #[test]
    fn decisions_carry_approval() {
        let e = env("approval");
        let root = q(&e, None, "根");
        e.db.connection()
            .execute(
                "INSERT INTO rt_log (node_id, action, detail, actor, approved_by, qid, question) \
                 VALUES (?1, 'add_q', '', 'run:7', 'saituki', 'a1', '作ってよいか')",
                params![root],
            )
            .unwrap();
        let s = status(e.db.connection(), &e.roots, &Defs::default()).unwrap();
        assert_eq!(s.decisions[0].approved_by.as_deref(), Some("saituki"));
        assert_eq!(s.decisions[0].qid.as_deref(), Some("a1"));
        let detail = node_detail(e.db.connection(), &e.roots, &Defs::default(), &fmt_id(Kind::Q, root)).unwrap();
        assert_eq!(detail.decisions[0].question.as_deref(), Some("作ってよいか"));
        assert_eq!(detail.history.last().unwrap().approved_by.as_deref(), Some("saituki"));
    }
}
