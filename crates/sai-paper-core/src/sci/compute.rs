//! 空欄・印・負債・関門・種の文・次の一手を計算し、`status` と `node` の JSON を作る。
//!
//! ここは LLM を使わない計算である。保存されていない値（空欄の種類・印・負債の数・関門の進み）は
//! 保存せず、木を読み直すたびに同じ規則で同じ答えを出す。
//!
//! 判定の規則（空欄の種類を 1〜5 のどれにするか、印、負債の数え方、次の一手の並び）は
//! 共有契約 C-08 に全部書いてある。ここはその規則を実装するだけの層で、規則そのものは変えない。
//! 節点ごとに SQL を投げず、最初に全部を読んでから数える（木は数百の節点が上限の見込み）。

use std::collections::{HashMap, HashSet, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};

use rusqlite::Connection;

use super::defs::{self, Cond, Defs, NextRule, TypeDef};
use super::store::{self, Decision, Finding, Link, Node, Proposal, Question, SearchRecord, Seed};
use super::{Result, SciError, SciKeys, fmt_ref};
use crate::paths::PaperRoots;

// ---- 表示名 -----------------------------------------------------------------

/// 空欄の種類の表示名（C-01）。
const BLANK_LABELS: [(&str, &str); 5] = [
    ("untouched", "UNTOUCHED"),
    ("in_progress", "IN_PROGRESS"),
    ("insufficient", "INSUFFICIENT"),
    ("awaiting_decision", "AWAITING_DECISION"),
    ("conflict", "CONFLICT"),
];

/// 印の表示名（C-01）。
const MARK_LABELS: [(&str, &str); 2] = [("weak_evidence", "WEAK_EVIDENCE"), ("recheck", "RECHECK")];

/// 負債の行の `why`。
const DEBT_WHY: &str = "DEBT";

/// 関門を全部通ったときの名前。
const GATE_DONE_NAME: &str = "種にできる";

/// 次の一手に出す件数の上限。
const NEXT_LIMIT: usize = 10;

/// 空欄の種類・印の表示名。表に無い値はその値そのもの。
fn label(table: &[(&str, &str)], value: &str) -> String {
    table
        .iter()
        .find(|(key, _)| *key == value)
        .map_or_else(|| value.to_string(), |(_, name)| (*name).to_string())
}

// ---- status の JSON ---------------------------------------------------------

/// 研究の型（`{"id": "problem", "name": "課題駆動"}`）。
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize)]
pub struct TypeInfo {
    /// 型の id。
    pub id: String,
    /// 表示名。
    pub name: String,
}

/// 種の文（`status.seed`）。
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize)]
pub struct SeedSentence {
    /// 全部をつなげて「」で囲んだもの。
    pub text: String,
    /// ひな形を `{…}` で分けたもの。
    pub parts: Vec<SeedPart>,
}

/// 種の文の 1 つの部分。
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize)]
pub struct SeedPart {
    /// この部分の文。
    pub text: String,
    /// 差し込みの項目キー（文字の部分は `None`、`{mind:…}` は `"mind:<見出し>"`）。
    pub key: Option<String>,
    /// 差し込んだ節点（`N12` など。項目が無ければ `None`）。
    pub node: Option<String>,
    /// 埋まっているか。
    pub filled: bool,
    /// 仮決定か。
    pub tentative: bool,
}

/// 関門の進み（`{"current": 2, "total": 7, "name": "小目的", "list": […]}`）。
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize)]
pub struct GateState {
    /// 最初に通らない関門の番号（1 始まり）。全部通れば `total + 1`。
    pub current: i64,
    /// 関門の数。
    pub total: i64,
    /// 今の関門の名前。全部通れば「種にできる」。
    pub name: String,
    /// 関門の一覧（上から順に、判定の結果も入れる）。
    pub list: Vec<GateRow>,
}

/// 関門の 1 つ。
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize)]
pub struct GateRow {
    /// 表示名。
    pub name: String,
    /// 条件の文字列。
    pub when: String,
    /// 満たしているか。
    pub passed: bool,
}

/// 負債の数。
#[derive(Debug, Clone, Copy, Default, PartialEq, serde::Serialize)]
pub struct Debts {
    /// 仕分けていない提案。
    pub pending_proposals: i64,
    /// 読んだが反映していない論文。
    pub unreflected_papers: i64,
    /// 選別していない検索。
    pub unscreened_sets: i64,
    /// 書いていない振り返り。
    pub empty_reviews: i64,
}

/// 節点の行（`status.nodes` の 1 要素）。
///
/// `body` は `node_detail` の `node` に入れるためだけに持つ。空のときは書き出さない
/// （`status` の節点には本文を出さない）。
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize)]
pub struct NodeRow {
    /// 節点の番号（`N12` など）。
    pub id: String,
    /// `item` か `option`。
    pub kind: String,
    /// 項目キー（候補は `None`）。
    pub key: Option<String>,
    /// 題。
    pub title: String,
    /// 項目の型（C-01 の値）。
    pub types: Vec<String>,
    /// 状態（C-01 の値）。
    pub state: String,
    /// 対象外にできない項目か。
    pub fixed: bool,
    /// 親の線（`N11` の形）。複数あり得る。
    pub parents: Vec<String>,
    /// 一番上が 0。
    pub depth: i64,
    /// 同じ親の中での並び。
    pub ord: i64,
    /// 本文（`node_detail` だけに出す）。
    #[serde(skip_serializing_if = "String::is_empty")]
    pub body: String,
    /// 種の文に差し込む値（空欄なら空文字）。
    pub value: String,
    /// 空欄の種類（`open` の項目だけ）。
    pub blank: Option<String>,
    /// 印（`decided` か `tentative` の項目だけ）。
    pub marks: Vec<String>,
    /// 節点ごとの数。
    pub counts: NodeCounts,
}

/// 節点ごとの数。
#[derive(Debug, Clone, Copy, Default, PartialEq, serde::Serialize)]
pub struct NodeCounts {
    /// 答えていない問い。
    pub questions_open: i64,
    /// 下の候補の数。
    pub options: i64,
    /// 支持の知見。
    pub support: i64,
    /// 反証の知見。
    pub refute: i64,
    /// 結び付いた論文。
    pub papers: i64,
    /// 対象の `pending` の提案。
    pub pending: i64,
}

/// `parent` 以外の線（`{"kind": "depends", "from": "N11", "to": "N30"}`）。
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize)]
pub struct EdgeRow {
    /// 線の種類。
    pub kind: String,
    /// 起点。
    pub from: String,
    /// 終点。
    pub to: String,
}

/// 次の一手の 1 行。
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize)]
pub struct NextAction {
    /// 対象の節点（負債の行は `None`）。
    pub node: Option<String>,
    /// 使うブロック（人の作業は `None`）。
    pub block: Option<String>,
    /// 行動の目的。
    pub purpose: String,
    /// 期待する成果。
    pub expect: String,
    /// なぜこれが次の一手か（空欄の種類・印の表示名、または「負債」）。
    pub why: String,
    /// `blank` / `mark` / `debt`。
    pub kind: String,
}

/// 走っている実行（`{"run_id": 120, "node": "N12", "block": "sci-search", "owner": "running"}`）。
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize)]
pub struct RunningRun {
    /// 実行 ID。
    pub run_id: i64,
    /// 対象の節点。
    pub node: Option<String>,
    /// 実行したブロック。
    pub block: String,
    /// 置き場。
    pub owner: String,
}

/// 種の 1 つ。
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize)]
pub struct SeedRow {
    /// 種の番号（`S1`）。
    pub id: String,
    /// 題。
    pub title: String,
    /// 作った時刻。
    pub created_at: String,
}

/// `status` 全体の JSON。
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize)]
pub struct Status {
    /// `sci init` をしたか。
    pub initialized: bool,
    /// 研究の型（始めていなければ `None`）。
    #[serde(rename = "type")]
    pub type_info: Option<TypeInfo>,
    /// 種の文（始めていなければ `None`）。
    pub seed: Option<SeedSentence>,
    /// 関門の進み（始めていなければ `None`）。
    pub gate: Option<GateState>,
    /// 負債の数。
    pub debts: Debts,
    /// 節点（並んだ順）。
    pub nodes: Vec<NodeRow>,
    /// `parent` 以外の線。
    pub edges: Vec<EdgeRow>,
    /// 次の一手（10 件まで）。
    pub next: Vec<NextAction>,
    /// 走っている実行。
    pub running: Vec<RunningRun>,
    /// 種。
    pub seeds: Vec<SeedRow>,
    /// 利用者にそのまま見せる注意。
    pub warnings: Vec<String>,
}

// ---- node の JSON -----------------------------------------------------------

/// 項目のガイド（`node_detail.guide`。ガイドが無ければ `None`）。
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize)]
pub struct GuideDetail {
    /// 項目キー。
    pub key: String,
    /// 表示名。
    pub name: String,
    /// 説明。
    pub explain: String,
    /// 埋まった条件の文字列。
    pub condition: String,
    /// 条件の下の、人向けの説明。
    pub condition_text: String,
    /// 条件を満たしているか。
    pub condition_met: bool,
    /// 項目を作ったときに入る既定の問い。
    pub questions: Vec<String>,
    /// 落とし穴。
    pub pitfalls: Vec<String>,
    /// 抜き出す項目。
    pub extract: Vec<ExtractRow>,
}

/// 抜き出す項目の 1 つ。
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize)]
pub struct ExtractRow {
    /// 論文の役割。
    pub role: String,
    /// 抜き出すもの。
    pub fields: String,
}

/// 問いの行。
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize)]
pub struct QuestionRow {
    /// 問いの番号（`Q7`）。
    pub id: String,
    /// 問いの文。
    pub text: String,
    /// 状態。
    pub state: String,
    /// 答え。
    pub answer: String,
    /// 既定の問いか。
    pub from_guide: bool,
}

/// 候補の行。
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize)]
pub struct OptionRow {
    /// 候補の番号（`N20`）。
    pub id: String,
    /// 題。
    pub title: String,
    /// 状態。
    pub state: String,
    /// 本文。
    pub body: String,
    /// 支持の知見の数。
    pub support: i64,
    /// 反証の知見の数。
    pub refute: i64,
    /// 結び付いた論文の数。
    pub papers: i64,
}

/// 知見の行。
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize)]
pub struct FindingRow {
    /// 知見の番号（`F31`）。
    pub id: String,
    /// 入った節点。
    pub node: String,
    /// 知見の文。
    pub text: String,
    /// 引用先。
    pub cites: Vec<String>,
    /// 向き（C-01 の値）。
    pub polarity: String,
    /// 根拠の段。
    pub level: Option<i64>,
    /// 出どころ。
    pub source: String,
    /// report の番号。
    pub report_id: Option<String>,
    /// 論文の番号。
    pub paper_id: Option<i64>,
    /// 実行の番号。
    pub run_id: Option<i64>,
    /// 答えた問いの番号。
    pub question: Option<String>,
    /// 自動で入ったか。
    pub auto: bool,
    /// 提案の番号。
    pub proposal: Option<String>,
    /// 入れた時刻。
    pub created_at: String,
}

/// 結び付きの行。
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize)]
pub struct LinkRow {
    /// 結び付きの番号（`L3`）。
    pub id: String,
    /// 結び付けた節点。
    pub node: String,
    /// 相手（`paper:37` の形）。
    pub target: String,
    /// 相手の題。
    pub title: String,
    /// 論文の役割。
    pub role: Option<String>,
    /// 読み具合。
    pub read_state: Option<String>,
    /// 自動で入ったか。
    pub auto: bool,
    /// 提案の番号。
    pub proposal: Option<String>,
    /// 理由。
    pub reason: String,
    /// 結んだ時刻。
    pub created_at: String,
}

/// 決定の記録の行。
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize)]
pub struct DecisionRow {
    /// 種類（C-01 の値）。
    pub action: String,
    /// 対象の候補。
    pub option: Option<String>,
    /// 理由。
    pub reason: String,
    /// 根拠にした知見。
    pub evidence: Vec<String>,
    /// 却下した候補。
    pub rejected: Vec<RejectedRow>,
    /// 反証を探したか。
    pub counter_searched: bool,
    /// 実行者。
    pub actor: String,
    /// 記録した時刻。
    pub created_at: String,
}

/// 却下した候補。
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize)]
pub struct RejectedRow {
    /// 候補の番号。
    pub option: String,
    /// 理由。
    pub reason: String,
}

/// 実行の行（`node_detail.actions`）。
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize)]
pub struct ActionRow {
    /// 実行 ID。
    pub run_id: i64,
    /// 実行したブロック。
    pub block: String,
    /// 置き場。
    pub owner: String,
    /// 実行者（`pi` など）。
    pub runner: String,
    /// 行動の目的。
    pub purpose: String,
    /// 作った時刻。
    pub created: String,
}

/// 見つけた候補の流れ（`node_detail.funnel`）。
#[derive(Debug, Clone, Copy, Default, PartialEq, serde::Serialize)]
pub struct Funnel {
    /// 検索で出た論文の重なりを除いた数。
    pub searched: i64,
    /// 選別した候補の数。
    pub screened: i64,
    /// 読んだ論文の数。
    pub read: i64,
    /// 反映した論文の数。
    pub reflected: i64,
}

/// 仕分け待ちの提案の行。
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize)]
pub struct PendingRow {
    /// 提案の番号（`P58`）。
    pub id: String,
    /// 操作の種類。
    pub op: String,
    /// 中身。
    pub payload: serde_json::Value,
    /// 理由。
    pub reason: String,
    /// 実行の番号。
    pub run_id: Option<i64>,
    /// 実行したブロック。
    pub block: String,
    /// 提案した時刻。
    pub created_at: String,
}

/// 使うブロックの行（`node_detail.blocks`）。
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize)]
pub struct BlockRow {
    /// ブロックの id（人の作業は `None`）。
    pub block: Option<String>,
    /// 行動の目的。
    pub purpose: String,
    /// 期待する成果。
    pub expect: String,
    /// なぜこれが次の一手か。
    pub why: String,
}

/// 検索の記録の行。
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize)]
pub struct SearchRow {
    /// 検索語。
    pub query: String,
    /// 範囲。
    pub scope: String,
    /// 見つかった数。
    pub hits: i64,
    /// 検索した時刻。
    pub created_at: String,
}

/// `node_detail` 全体の JSON。
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize)]
pub struct NodeDetail {
    /// 節点（`status.nodes` の 1 要素に本文を加えたもの）。
    pub node: NodeRow,
    /// 項目ガイド（無ければ `None`）。
    pub guide: Option<GuideDetail>,
    /// この節点の問い。
    pub questions: Vec<QuestionRow>,
    /// 下の候補。
    pub options: Vec<OptionRow>,
    /// この節点と候補の知見。
    pub findings: Vec<FindingRow>,
    /// この節点と候補の結び付き。
    pub links: Vec<LinkRow>,
    /// 決定の記録。
    pub decisions: Vec<DecisionRow>,
    /// 結び付いた実行。
    pub actions: Vec<ActionRow>,
    /// 見つけた候補の流れ。
    pub funnel: Funnel,
    /// 仕分け待ちの提案。
    pub pending: Vec<PendingRow>,
    /// 使うブロックの候補。
    pub blocks: Vec<BlockRow>,
    /// 検索の記録。
    pub searches: Vec<SearchRow>,
    /// 価値の軸（mind.md の「価値の軸」の節）。
    pub axes: Vec<String>,
}

// ---- 実行 -------------------------------------------------------------------

/// sci キーを持つ実行 1 つ。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SciRun {
    /// 実行 ID。
    pub run_id: i64,
    /// 置き場（`todo` / `running` / `review` / `done`）。
    pub owner: String,
    /// 実行したブロックの id。
    pub block: String,
    /// 実行者（`pi` など）。
    pub runner: String,
    /// task.md の sci キー。
    pub keys: SciKeys,
    /// 作った時刻（frontmatter の `creation_date`）。
    pub created: String,
    /// 実行フォルダ。
    pub path: PathBuf,
}

/// `.sai/outputs/*/*/task.md` のうち sci キーを持つ実行（新しい順）。
#[must_use]
pub fn sci_runs(roots: &PaperRoots) -> Vec<SciRun> {
    let mut runs = Vec::new();
    for run in crate::runs::list_run_dirs(&roots.root) {
        let Ok(text) = fs::read_to_string(run.path.join("task.md")) else {
            continue;
        };
        let keys = super::task_sci_keys(&text);
        if !keys.is_sci() {
            continue;
        }
        let front = crate::runs::read_frontmatter(&text);
        let get = |name: &str| -> String {
            front
                .iter()
                .find(|(key, _)| key == name)
                .map_or_else(String::new, |(_, value)| value.trim().to_string())
        };
        runs.push(SciRun {
            run_id: run.run_id,
            owner: run.owner,
            block: get("block"),
            runner: get("runner"),
            keys,
            created: get("creation_date"),
            path: run.path,
        });
    }
    runs
}

/// `status` を作る（`sci status --json` と `phone sci status` が返すもの）。
pub fn status(conn: &Connection, roots: &PaperRoots, defs: &Defs) -> Result<Status> {
    Tree::load(conn, roots, defs)?.status()
}

/// `node_detail` を作る（`sci node <N> --json` と `phone sci node <N>` が返すもの）。
///
/// 節点が無ければ [`SciError::NotFound`]。
pub fn node_detail(
    conn: &Connection,
    roots: &PaperRoots,
    defs: &Defs,
    node_id: i64,
) -> Result<NodeDetail> {
    Tree::load(conn, roots, defs)?.node_detail(node_id)
}

/// 条件 1 つを判定する（関門・埋まった条件で使う）。
///
/// 条件に書いた項目のキーが木に無いとき、その条件は偽にして
/// 「関門の条件の項目がありません: <key>」を `warnings` に足す。
/// 書き方が読めない条件は [`defs::parse_condition`] が [`SciError::Invalid`] を返すので、
/// 呼び出し側が「条件を読めません: <文字列>」を `warnings` に足す。
pub fn eval_conditions(
    conn: &Connection,
    roots: &PaperRoots,
    conds: &[Cond],
    warnings: &mut Vec<String>,
) -> Result<bool> {
    Tree::load(conn, roots, &Defs::default())?.eval_all(conds, warnings)
}

/// `mind.md` の `## 見出し` ごとの本文（コメントと空行を除く）。
///
/// ファイルが無ければ空。中身が空の節（コメントだけの節も）は出さない。
#[must_use]
pub fn mind_sections(data_dir: &Path) -> Vec<(String, String)> {
    let Ok(text) = fs::read_to_string(crate::paths::mind_path(data_dir)) else {
        return Vec::new();
    };
    let mut sections: Vec<(String, Vec<String>)> = Vec::new();
    let mut current: Option<(String, Vec<String>)> = None;
    for line in text.lines() {
        let trimmed = line.trim();
        if let Some(name) = trimmed.strip_prefix("## ") {
            if let Some(section) = current.take() {
                sections.push(section);
            }
            current = Some((name.trim().to_string(), Vec::new()));
            continue;
        }
        // 最初の見出しより前の本文（前書き）は捨てる。
        if let Some((_, lines)) = current.as_mut() {
            lines.push(line.to_string());
        }
    }
    if let Some(section) = current.take() {
        sections.push(section);
    }
    sections
        .into_iter()
        .map(|(name, lines)| (canonical_name(name), strip_comments(&lines)))
        .filter(|(_, body)| !body.is_empty())
        .collect()
}

/// 見出しを新しい名前にそろえた文字列にする。
fn canonical_name(name: String) -> String {
    defs::canonical_mind_heading(&name).to_string()
}

/// 行の列から `<!-- … -->` と空行を除いた 1 本の文字列に束ねる。
///
/// コメントだけだった行は落とす（コメントを取り除いて空になった行）。
fn strip_comments(lines: &[String]) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut in_comment = false;
    for line in lines {
        let mut piece = String::new();
        let mut rest = line.as_str();
        loop {
            if in_comment {
                match rest.find("-->") {
                    Some(index) => {
                        rest = &rest[index + 3..];
                        in_comment = false;
                    }
                    None => break,
                }
            } else if let Some(index) = rest.find("<!--") {
                piece.push_str(&rest[..index]);
                rest = &rest[index + 4..];
                in_comment = true;
            } else {
                piece.push_str(rest);
                break;
            }
        }
        if piece.trim().is_empty() {
            continue;
        }
        out.push(piece.trim_end().to_string());
    }
    out.join("\n").trim().to_string()
}

// ---- 木を 1 度だけ読む ------------------------------------------------------

/// 木を読み込んだ状態。数えるものはここから取る（節点ごとに SQL を投げない）。
struct Tree {
    /// 型と項目ガイド。
    defs: Defs,
    /// プロジェクトルート（`empty_reviews` に見せる）。
    root: PathBuf,
    /// `sci_meta` の `type`（始めていなければ `None`）。
    type_id: Option<String>,
    /// 全節点（番号の昇順）。
    nodes: Vec<Node>,
    /// 全線。
    edges: Vec<store::Edge>,
    /// 全問い。
    questions: Vec<Question>,
    /// 全知見（外したものは除く）。
    findings: Vec<Finding>,
    /// 全結び付き（外したものは除く）。
    links: Vec<Link>,
    /// 全決定の記録（古い順）。
    decisions: Vec<Decision>,
    /// 仕分け待ちの提案（新しい順）。
    proposals: Vec<Proposal>,
    /// 全検索の記録（新しい順）。
    searches: Vec<SearchRecord>,
    /// 全種。
    seeds: Vec<Seed>,
    /// sci キーを持つ実行（新しい順）。
    runs: Vec<SciRun>,
    /// 節点番号 → `nodes` の位置。
    position: HashMap<i64, usize>,
    /// 節点番号 → `parent` の線の from。
    parents: HashMap<i64, Vec<i64>>,
    /// 節点番号 → `parent` の線の to（`ord`・id の順）。
    children: HashMap<i64, Vec<i64>>,
    /// 節点番号 → `depends` の線の from。
    upstream: HashMap<i64, Vec<i64>>,
    /// 節点番号 → `findings` の位置。
    findings_of: HashMap<i64, Vec<usize>>,
    /// 節点番号 → `links` の位置。
    links_of: HashMap<i64, Vec<usize>>,
    /// 節点番号 → `questions` の位置。
    questions_of: HashMap<i64, Vec<usize>>,
    /// 節点番号 → `decisions` の位置。
    decisions_of: HashMap<i64, Vec<usize>>,
    /// 並んだ節点番号。
    ordered: Vec<i64>,
    /// 節点番号 → 深さ（一番上が 0）。
    depth: HashMap<i64, i64>,
    /// 選別していない検索セット（`unscreened_sets` 用）。
    unscreened: Vec<(i64, String, i64)>,
    /// 節点番号 → 選別済みの候補の数（`funnel.screened` 用）。
    screened: HashMap<i64, i64>,
    /// 結び付きの相手の題（`paper` と `report`）。
    titles: HashMap<(String, i64), String>,
    /// mind.md の節。
    sections: Vec<(String, String)>,
}

impl Tree {
    /// 木を全部読む。
    fn load(conn: &Connection, roots: &PaperRoots, defs: &Defs) -> Result<Self> {
        let nodes = store::list_nodes(conn)?;
        let edges = store::list_edges(conn)?;
        let questions = store::list_questions(conn, None)?;
        let findings = store::list_findings(conn, None, false)?;
        let links = store::list_links(conn, None, false)?;
        let decisions = store::list_decisions(conn, None)?;
        let proposals = store::list_proposals(conn, Some("pending"))?;
        let searches = store::list_searches(conn, None)?;
        let seeds = store::list_seeds(conn)?;
        let runs = sci_runs(roots);

        let mut position = HashMap::new();
        for (index, node) in nodes.iter().enumerate() {
            position.insert(node.id, index);
        }

        // 線の行き先ごとにまとめる。
        let mut parents: HashMap<i64, Vec<i64>> = HashMap::new();
        let mut children: HashMap<i64, Vec<i64>> = HashMap::new();
        let mut upstream: HashMap<i64, Vec<i64>> = HashMap::new();
        for edge in &edges {
            match edge.kind.as_str() {
                "parent" => {
                    parents.entry(edge.to).or_default().push(edge.from);
                    children.entry(edge.from).or_default().push(edge.to);
                }
                "depends" => {
                    upstream.entry(edge.to).or_default().push(edge.from);
                }
                _ => {}
            }
        }
        let order_of = |id: i64| -> (i64, i64) {
            position
                .get(&id)
                .map(|index| (nodes[*index].ord, nodes[*index].id))
                .unwrap_or((i64::MAX, i64::MAX))
        };
        // 同じ親の中は `ord`・id の順にする。
        for list in children.values_mut() {
            list.sort_by_key(|id| order_of(*id));
        }
        for list in parents.values_mut() {
            list.sort_unstable();
        }
        for list in upstream.values_mut() {
            list.sort_unstable();
        }

        let mut findings_of: HashMap<i64, Vec<usize>> = HashMap::new();
        for (index, finding) in findings.iter().enumerate() {
            findings_of.entry(finding.node_id).or_default().push(index);
        }
        let mut links_of: HashMap<i64, Vec<usize>> = HashMap::new();
        for (index, link) in links.iter().enumerate() {
            links_of.entry(link.node_id).or_default().push(index);
        }
        let mut questions_of: HashMap<i64, Vec<usize>> = HashMap::new();
        for (index, question) in questions.iter().enumerate() {
            questions_of.entry(question.node_id).or_default().push(index);
        }
        let mut decisions_of: HashMap<i64, Vec<usize>> = HashMap::new();
        for (index, decision) in decisions.iter().enumerate() {
            decisions_of.entry(decision.node_id).or_default().push(index);
        }

        let titles = link_titles(conn, &links)?;
        let mut tree = Self {
            defs: defs.clone(),
            root: roots.root.clone(),
            type_id: store::meta_get(conn, "type")?,
            nodes,
            edges,
            questions,
            findings,
            links,
            decisions,
            proposals,
            searches,
            seeds,
            runs,
            position,
            parents,
            children,
            upstream,
            findings_of,
            links_of,
            questions_of,
            decisions_of,
            ordered: Vec::new(),
            depth: HashMap::new(),
            unscreened: unscreened_sets(conn)?,
            screened: screened_counts(conn)?,
            titles,
            sections: mind_sections(&roots.data_dir),
        };
        tree.order_nodes();
        Ok(tree)
    }

    /// 節点を行儀よく並べる（一番上の項目は型の「項目の順序」の順、その下は深さ優先）。
    fn order_nodes(&mut self) {
        let order: Vec<&str> = self
            .defs
            .type_def(self.type_id.as_deref().unwrap_or_default())
            .map(|found| found.order.iter().map(String::as_str).collect())
            .unwrap_or_default();
        let place = |node: &Node| -> usize {
            node.item_key
                .as_deref()
                .and_then(|key| order.iter().position(|known| *known == key))
                .unwrap_or(order.len())
        };

        let mut top: Vec<&Node> = self
            .nodes
            .iter()
            .filter(|node| !self.parents.contains_key(&node.id))
            .collect();
        top.sort_by_key(|node| (place(node), node.ord, node.id));

        let mut ordered: Vec<i64> = Vec::new();
        let mut seen: HashSet<i64> = HashSet::new();
        // 深さ優先（C-08）。スタックなので、子を逆順に積んで先頭から取り出す。
        let mut stack: Vec<(i64, i64)> = top.iter().rev().map(|node| (node.id, 0)).collect();
        while let Some((id, depth)) = stack.pop() {
            // 2 回目に出会った節点は飛ばす。
            if !seen.insert(id) {
                continue;
            }
            self.depth.insert(id, depth);
            ordered.push(id);
            // 子は `ord`・id の順で並ぶ（`children` はその順に並んでいる）。
            // ただし候補は子の項目より先に置く（一覧で、候補がどの項目のものか分かるように）。
            let mut kids: Vec<i64> = self.children.get(&id).cloned().unwrap_or_default();
            kids.sort_by_key(|child| self.node(*child).map(|node| node.kind != "option").unwrap_or(true));
            let mut pushed: Vec<(i64, i64)> = kids
                .into_iter()
                .rev()
                .map(|child| (child, depth + 1))
                .collect();
            pushed.retain(|(child, _)| !seen.contains(child));
            stack.append(&mut pushed);
        }
        // 親から辿れなかった節点は後ろに足す（線の向きがおかしくても落とさない）。
        for node in &self.nodes {
            if seen.insert(node.id) {
                self.depth.insert(node.id, 0);
                ordered.push(node.id);
            }
        }
        self.ordered = ordered;
    }

    /// 節点を番号で探す（無ければ `None`）。
    fn node(&self, id: i64) -> Option<&Node> {
        self.position.get(&id).map(|index| &self.nodes[*index])
    }

    /// 節点を番号で探す（無ければ [`SciError::NotFound`]）。
    fn need_node(&self, id: i64) -> Result<&Node> {
        self.node(id)
            .ok_or_else(|| SciError::NotFound(format!("NODE {} がありません", fmt_ref('N', id))))
    }

    /// 項目キーで項目を探す。
    fn item_of_key(&self, key: &str) -> Option<&Node> {
        self.nodes
            .iter()
            .find(|node| node.item_key.as_deref() == Some(key))
    }

    /// 下の候補（`parent` の線の先で `kind = option` の節点）。
    /// 親の線は項目の子にも引かれるので、候補だけを数える。
    fn options(&self, id: i64) -> Vec<&Node> {
        self.children
            .get(&id)
            .map(|ids| {
                ids.iter()
                    .filter_map(|child| self.node(*child))
                    .filter(|node| node.kind == "option")
                    .collect()
            })
            .unwrap_or_default()
    }

    /// 下の候補のうち、生きているもの（`candidate` か `selected`）。
    fn live_options(&self, id: i64) -> Vec<&Node> {
        self.options(id)
            .into_iter()
            .filter(|option| matches!(option.state.as_str(), "candidate" | "selected"))
            .collect()
    }

    /// 自分の下（子孫）。輪があっても止まる。
    fn below(&self, id: i64) -> Vec<i64> {
        let mut found: Vec<i64> = Vec::new();
        let mut seen: HashSet<i64> = HashSet::new();
        let mut queue: VecDeque<i64> = self.children.get(&id).cloned().unwrap_or_default().into();
        while let Some(next) = queue.pop_front() {
            if !seen.insert(next) {
                continue;
            }
            found.push(next);
            queue.extend(self.children.get(&next).cloned().unwrap_or_default());
        }
        found
    }

    /// この節点と、その下の候補。
    fn with_options(&self, id: i64) -> Vec<i64> {
        let mut ids = vec![id];
        ids.extend(self.options(id).iter().map(|node| node.id));
        ids
    }

    /// この節点と、その下の候補の知見。
    fn own_findings(&self, id: i64) -> Vec<&Finding> {
        self.with_options(id)
            .iter()
            .flat_map(|target| self.findings_of.get(target))
            .flatten()
            .map(|index| &self.findings[*index])
            .collect()
    }

    /// この節点と、その下の候補の結び付き。
    fn own_links(&self, id: i64) -> Vec<&Link> {
        self.with_options(id)
            .iter()
            .flat_map(|target| self.links_of.get(target))
            .flatten()
            .map(|index| &self.links[*index])
            .collect()
    }

    /// この節点の問い。
    fn questions_of_node(&self, id: i64) -> Vec<&Question> {
        self.questions_of
            .get(&id)
            .map(|indexes| indexes.iter().map(|index| &self.questions[*index]).collect())
            .unwrap_or_default()
    }

    /// この節点の決定の記録。
    fn decisions_of_node(&self, id: i64) -> Vec<&Decision> {
        self.decisions_of
            .get(&id)
            .map(|indexes| indexes.iter().map(|index| &self.decisions[*index]).collect())
            .unwrap_or_default()
    }

    /// この節点と、その下の候補の（結線した）論文の結び付き。
    fn own_paper_links(&self, id: i64) -> Vec<&Link> {
        self.own_links(id)
            .into_iter()
            .filter(|link| link.target_kind == "paper")
            .collect()
    }

    /// 上流（`parent` の親と `depends` の起点）。たどれるだけ上へ。
    fn upstreams(&self, id: i64) -> Vec<i64> {
        let mut found: Vec<i64> = Vec::new();
        let mut seen: HashSet<i64> = HashSet::new();
        let mut queue: VecDeque<i64> = VecDeque::new();
        queue.extend(self.parents.get(&id).into_iter().flatten());
        queue.extend(self.upstream.get(&id).into_iter().flatten());
        while let Some(next) = queue.pop_front() {
            if !seen.insert(next) {
                continue;
            }
            found.push(next);
            queue.extend(self.parents.get(&next).into_iter().flatten());
            queue.extend(self.upstream.get(&next).into_iter().flatten());
        }
        found
    }

    /// 一番上の祖先（親の親をたどり切った節点）。
    fn top_ancestor(&self, id: i64) -> i64 {
        let mut current = id;
        for _ in 0..64 {
            let Some(parent) = self.parents
                .get(&current)
                .and_then(|list| list.first())
                .copied() else {
                break;
            };
            current = parent;
        }
        current
    }

    /// 節点の表示名（ガイドの名前 → 題 → キー）。
    fn display_name(&self, node: &Node) -> String {
        if let Some(key) = node.item_key.as_deref()
            && let Some(guide) = self.defs.guide(key)
            && !guide.name.is_empty()
        {
            return guide.name.clone();
        }
        if !node.title.is_empty() {
            return node.title.clone();
        }
        node.item_key.clone().unwrap_or_default()
    }

    /// この項目のガイドにある「情報不足」の目安（無ければ 2）。
    fn min_findings(&self, node: &Node) -> usize {
        node.item_key
            .as_deref()
            .and_then(|key| self.defs.guide(key))
            .map_or(2, |guide| guide.min_findings)
    }

    // ---- 空欄と印 ------------------------------------------------------------

    /// 空欄の種類（`open` の項目だけ。1〜5 を上から順に当てて、最初に当てはまったものを 1 つだけ出す）。
    fn blank_of(&self, node: &Node) -> String {
        let findings = self.own_findings(node.id);
        let links = self.own_links(node.id);
        let options = self.options(node.id);
        let questions = self.questions_of_node(node.id);
        let min = self.min_findings(node);

        // 1. まだ誰も触っていない。
        let answered = questions
            .iter()
            .any(|question| question.state == "answered" || question.state == "withdrawn");
        let hand_made = questions.iter().any(|question| !question.from_guide);
        if findings.is_empty() && links.is_empty() && options.is_empty() && !answered && !hand_made
        {
            return "untouched".to_string();
        }

        // 2. 誰かが動き出している（走っている run、または仕分け待ちの提案）。
        let running_link = links.iter().any(|link| {
            link.target_kind == "run"
                && self.runs.iter().any(|run| {
                    run.run_id == link.target_ref.trim().parse::<i64>().unwrap_or(0)
                        && matches!(run.owner.as_str(), "todo" | "running")
                })
        });
        let waiting = self.proposals.iter().any(|proposal| {
            proposal
                .node_id
                .is_some_and(|target| self.with_options(node.id).contains(&target))
        });
        if running_link || waiting {
            return "in_progress".to_string();
        }

        // 3. 項目そのもの、または候補のどれかに、支持と反証が両方ある。
        let conflicted = self.two_sides(node.id)
            || options
                .iter()
                .filter(|option| matches!(option.state.as_str(), "candidate" | "selected"))
                .any(|option| self.two_sides(option.id));
        if conflicted {
            return "conflict".to_string();
        }

        // 4. 決められるだけ揃った。
        if node.types.iter().any(|kind| kind == "choose") {
            let candidates: Vec<&Node> = options
                .iter()
                .filter(|option| option.state == "candidate")
                .copied()
                .collect();
            let filled = |option: &Node| -> usize {
                self.findings_of.get(&option.id).map_or(0, Vec::len)
            };
            let ready = if candidates.len() >= 2 {
                candidates.iter().all(|option| filled(option) >= 1)
            } else if candidates.len() == 1 {
                filled(candidates[0]) >= min
            } else {
                false
            };
            if ready {
                return "awaiting_decision".to_string();
            }
        } else if findings.len() >= min {
            return "awaiting_decision".to_string();
        }

        // 5. 上のどれでもない。
        "insufficient".to_string()
    }

    /// この節点（候補のときはその候補）に、支持と反証が両方あるか。
    fn two_sides(&self, id: i64) -> bool {
        let has = |polarity: &str| -> bool {
            self.findings_of
                .get(&id)
                .is_some_and(|indexes| {
                    indexes
                        .iter()
                        .any(|index| self.findings[*index].polarity == polarity)
                })
        };
        has("support") && has("refute")
    }

    /// 印（`decided` か `tentative` の項目だけ）。
    fn marks_of(&self, node: &Node) -> Vec<String> {
        let mut marks = Vec::new();

        // 支持の知見が 1 つも無い、または反証を探していない。
        let mut targets = vec![node.id];
        targets.extend(
            self.options(node.id)
                .iter()
                .filter(|option| option.state == "selected")
                .map(|option| option.id),
        );
        let support = targets
            .iter()
            .filter_map(|target| self.findings_of.get(target))
            .flatten()
            .filter(|index| self.findings[**index].polarity == "support")
            .count();
        // 決定の記録は古い順なので、いちばん新しい decide / tentative は後ろにある。
        let last_decide = self
            .decisions_of_node(node.id)
            .into_iter()
            .rfind(|decision| decision.action == "decide" || decision.action == "tentative");
        if support == 0 || last_decide.is_some_and(|decision| !decision.counter_searched) {
            marks.push("weak_evidence".to_string());
        }

        // 上流が後から変わった。
        let mine = node.state_changed_at.as_str();
        if self.upstreams(node.id).iter().any(|up| {
            self.node(*up).is_some_and(|other| other.state_changed_at.as_str() > mine)
        }) {
            marks.push("recheck".to_string());
        }
        marks
    }

    // ---- 条件 ---------------------------------------------------------------

    /// 条件を 1 つずつ判定する（すべて真なら全体も真）。
    fn eval_all(&self, conds: &[Cond], warnings: &mut Vec<String>) -> Result<bool> {
        let mut all = true;
        for cond in conds {
            all &= self.eval_one(cond, warnings)?;
        }
        Ok(all)
    }

    /// 条件を 1 つ判定する。
    fn eval_one(&self, cond: &Cond, warnings: &mut Vec<String>) -> Result<bool> {
        // mind.md は木には無関係なので先に返す。
        if let Cond::Mind(heading) = cond {
            let heading = defs::canonical_mind_heading(heading);
            return Ok(self
                .sections
                .iter()
                .any(|(name, body)| name == heading && !body.is_empty()));
        }
        let key = match cond {
            Cond::Decided(key)
            | Cond::Count(key, _)
            | Cond::Saturated(key)
            | Cond::Evidence(key, _)
            | Cond::Level(key, _)
            | Cond::NotEmpty(key) => key.as_str(),
            Cond::Mind(_) => unreachable!("上で返す"),
        };
        let Some(item) = self.item_of_key(key) else {
            warnings.push(format!("GATE の条件の ITEM がありません: {key}"));
            return Ok(false);
        };
        let live = self.live_options(item.id);
        let support_of = |id: i64| -> Vec<&Finding> {
            self.findings_of
                .get(&id)
                .map(|indexes| {
                    indexes
                        .iter()
                        .map(|index| &self.findings[*index])
                        .filter(|finding| finding.polarity == "support")
                        .collect()
                })
                .unwrap_or_default()
        };
        Ok(match cond {
            Cond::Decided(_) => item.state == "decided",
            Cond::NotEmpty(_) => !item.body.trim().is_empty(),
            Cond::Count(_, need) => live.len() >= *need,
            Cond::Mind(_) => unreachable!("上で返す"),
            // 候補がある項目は候補ごと、無い項目は項目そのものを見る。
            Cond::Evidence(_, need) => {
                if live.is_empty() {
                    support_of(item.id).len() >= *need
                } else {
                    live.iter().all(|option| support_of(option.id).len() >= *need)
                }
            }
            Cond::Level(_, need) => {
                let deep = |id: i64| -> bool {
                    support_of(id)
                        .iter()
                        .any(|finding| finding.level.is_some_and(|level| level >= *need))
                };
                if live.is_empty() {
                    deep(item.id)
                } else {
                    live.iter().all(|option| deep(option.id))
                }
            }
            Cond::Saturated(_) => !live.is_empty() && self.saturated(item.id, &live),
        })
    }

    /// 候補が出たあとに検索が 2 回以上あり、候補に代表例があるか。
    fn saturated(&self, item_id: i64, live: &[&Node]) -> bool {
        // いちばん新しい候補ができた時刻（項目そのものは含めない）。
        let newest = self
            .live_options(item_id)
            .into_iter()
            .map(|option| option.created_at.as_str())
            .max()
            .unwrap_or_default()
            .to_string();
        let targets: HashSet<i64> = std::iter::once(item_id).chain(self.below(item_id)).collect();
        let after = self
            .searches
            .iter()
            .filter(|search| {
                search
                    .node_id
                    .is_some_and(|node| targets.contains(&node))
                    && search.created_at.as_str() > newest.as_str()
            })
            .count();
        if after < 2 {
            return false;
        }
        // 候補と選んだ候補に、代表例の論文が 1 つ以上ある。
        live.iter().all(|option| {
            self.links_of
                .get(&option.id)
                .is_some_and(|indexes| {
                    indexes.iter().any(|index| {
                        let link = &self.links[*index];
                        link.target_kind == "paper" && link.role.as_deref() == Some("exemplar")
                    })
                })
        })
    }

    // ---- status -------------------------------------------------------------

    /// `status` の JSON を作る。
    fn status(&self) -> Result<Status> {
        let warnings = self.defs.warnings.clone();
        let Some(type_id) = self.type_id.clone() else {
            return Ok(Status {
                initialized: false,
                warnings,
                ..Status::default()
            });
        };
        let Some(type_def) = self.defs.type_def(&type_id).cloned() else {
            let mut warnings = warnings;
            warnings.push(format!("知らない RESEARCH_TYPE です: {type_id}"));
            return Ok(Status {
                initialized: true,
                type_info: Some(TypeInfo {
                    id: type_id,
                    name: String::new(),
                }),
                warnings,
                ..Status::default()
            });
        };
        let mut warnings = warnings;

        let seed = self.seed_sentence(&type_def);
        let gate = self.gate_state(&type_def, &mut warnings);
        let debts = self.debts();
        let nodes = self.node_rows(&type_def);
        let edges = self
            .edges
            .iter()
            .filter(|edge| edge.kind != "parent")
            .map(|edge| EdgeRow {
                kind: edge.kind.clone(),
                from: fmt_ref('N', edge.from),
                to: fmt_ref('N', edge.to),
            })
            .collect();
        let next = self.next_actions(&type_def, &gate, &debts);
        let running = self
            .runs
            .iter()
            .filter(|run| matches!(run.owner.as_str(), "todo" | "running"))
            .map(|run| RunningRun {
                run_id: run.run_id,
                node: run.keys.node.map(|id| fmt_ref('N', id)),
                block: run.block.clone(),
                owner: run.owner.clone(),
            })
            .collect();
        let seeds = self
            .seeds
            .iter()
            .map(|seed| SeedRow {
                id: fmt_ref('S', seed.id),
                title: seed.title.clone(),
                created_at: seed.created_at.clone(),
            })
            .collect();

        Ok(Status {
            initialized: true,
            type_info: Some(TypeInfo {
                id: type_def.id.clone(),
                name: type_def.name.clone(),
            }),
            seed,
            gate,
            debts,
            nodes,
            edges,
            next,
            running,
            seeds,
            warnings,
        })
    }

    /// 節点の行（並んだ順）。
    fn node_rows(&self, type_def: &TypeDef) -> Vec<NodeRow> {
        let _ = type_def;
        self.ordered
            .iter()
            .filter_map(|id| self.node(*id))
            .map(|node| self.node_row(node, false))
            .collect()
    }

    /// 節点の行を作る（`with_body` が true のときだけ本文を入れる）。
    fn node_row(&self, node: &Node, with_body: bool) -> NodeRow {
        let (value, _filled) = self.item_value(node);
        let blank = self.blank_of_row(node);
        let marks = self.marks_row(node);
        let findings = self.own_findings(node.id);
        let papers: Vec<&Link> = self
            .own_links(node.id)
            .into_iter()
            .filter(|link| {
                link.target_kind == "paper" && link.read_state.as_deref() != Some("irrelevant")
            })
            .collect();
        NodeRow {
            id: fmt_ref('N', node.id),
            kind: node.kind.clone(),
            key: node.item_key.clone(),
            title: node.title.clone(),
            types: node.types.clone(),
            state: node.state.clone(),
            fixed: node.fixed,
            parents: self
                .parents
                .get(&node.id)
                .map(|list| list.iter().map(|id| fmt_ref('N', *id)).collect())
                .unwrap_or_default(),
            depth: self.depth.get(&node.id).copied().unwrap_or(0),
            ord: node.ord,
            body: if with_body {
                node.body.clone()
            } else {
                String::new()
            },
            value,
            blank,
            marks,
            counts: NodeCounts {
                questions_open: self
                    .questions_of_node(node.id)
                    .iter()
                    .filter(|question| question.state == "open")
                    .count() as i64,
                options: self.options(node.id).len() as i64,
                support: findings
                    .iter()
                    .filter(|finding| finding.polarity == "support")
                    .count() as i64,
                refute: findings
                    .iter()
                    .filter(|finding| finding.polarity == "refute")
                    .count() as i64,
                papers: papers.len() as i64,
                pending: self
                    .proposals
                    .iter()
                    .filter(|proposal| proposal.node_id == Some(node.id))
                    .count() as i64,
            },
        }
    }

    /// 空欄の種類（`open` の項目だけ。それ以外は `None`）。
    fn blank_of_row(&self, node: &Node) -> Option<String> {
        if node.kind == "item" && node.state == "open" {
            Some(self.blank_of(node))
        } else {
            None
        }
    }

    /// 印（`decided` か `tentative` の項目だけ。それ以外は空）。
    fn marks_row(&self, node: &Node) -> Vec<String> {
        if node.kind == "item" && matches!(node.state.as_str(), "decided" | "tentative") {
            self.marks_of(node)
        } else {
            Vec::new()
        }
    }

    /// 種の文（C-03 の差し込み）。
    fn seed_sentence(&self, type_def: &TypeDef) -> Option<SeedSentence> {
        if type_def.seed.trim().is_empty() {
            return None;
        }
        let mut parts: Vec<SeedPart> = Vec::new();
        let mut rest = type_def.seed.as_str();
        loop {
            let Some(open) = rest.find('{') else {
                push_literal(&mut parts, rest);
                break;
            };
            let Some(close) = rest[open..].find('}').map(|index| index + open) else {
                push_literal(&mut parts, rest);
                break;
            };
            push_literal(&mut parts, &rest[..open]);
            let inner = &rest[open + 1..close];
            if let Some(heading) = inner.strip_prefix("mind:") {
                let heading = defs::canonical_mind_heading(heading);
                let body = self
                    .sections
                    .iter()
                    .find(|(name, _)| name == heading)
                    .map(|(_, body)| body.as_str())
                    .unwrap_or_default();
                let first = body.lines().map(str::trim).find(|line| !line.is_empty());
                let text = first.unwrap_or("");
                parts.push(SeedPart {
                    text: if text.is_empty() {
                        format!("［？{heading}］")
                    } else {
                        text.to_string()
                    },
                    key: Some(format!("mind:{heading}")),
                    node: None,
                    filled: !text.is_empty(),
                    tentative: false,
                });
            } else {
                let item = self.item_of_key(inner);
                let (text, filled) = item.map_or_else(
                    || (format!("［？{inner}］"), false),
                    |node| self.item_value(node),
                );
                parts.push(SeedPart {
                    text: text.clone(),
                    key: Some(inner.to_string()),
                    node: item.map(|node| fmt_ref('N', node.id)),
                    filled,
                    tentative: filled && item.is_some_and(|node| node.state == "tentative"),
                });
            }
            rest = &rest[close + 1..];
        }
        let joined: String = parts.iter().map(|part| part.text.as_str()).collect();
        Some(SeedSentence {
            text: format!("「{joined}」"),
            parts,
        })
    }

    /// 種の文に差し込む値（空欄なら `［？名前］`）。
    fn item_value(&self, node: &Node) -> (String, bool) {
        let name = self.display_name(node);
        if matches!(node.state.as_str(), "open" | "excluded") {
            return (format!("［？{name}］"), false);
        }
        let value = if node.types.iter().any(|kind| kind == "choose") {
            // 選ぶ項目は選んだ候補の題。
            self.options(node.id)
                .iter()
                .filter(|option| option.state == "selected")
                .map(|option| option.title.as_str())
                .collect::<Vec<_>>()
                .join("・")
        } else if node.types.iter().any(|kind| kind == "list") {
            // 並べる項目は候補と選んだ候補の題（3 つまで。4 つ以上は「など」を付ける）。
            let mut titles: Vec<&str> = self
                .live_options(node.id)
                .iter()
                .map(|option| option.title.as_str())
                .collect();
            if titles.len() > 3 {
                titles.truncate(3);
                format!("{}など", titles.join("・"))
            } else {
                titles.join("・")
            }
        } else {
            // 書く・確かめるだけの項目は本文の最初の行（本文が空なら題）。
            node.body
                .lines()
                .map(str::trim)
                .find(|line| !line.is_empty())
                .unwrap_or(node.title.as_str())
                .to_string()
        };
        if value.trim().is_empty() {
            return (format!("［？{name}］"), false);
        }
        // 仮決定は値のうしろに「（仮）」を付ける。
        let value = if node.state == "tentative" {
            format!("{value}（仮）")
        } else {
            value
        };
        (value, true)
    }

    /// 関門の進み。
    fn gate_state(&self, type_def: &TypeDef, warnings: &mut Vec<String>) -> Option<GateState> {
        if type_def.gates.is_empty() {
            return None;
        }
        let mut list = Vec::new();
        let mut first_blocked: Option<(i64, String)> = None;
        for (index, gate) in type_def.gates.iter().enumerate() {
            let passed = match defs::parse_condition(&gate.when) {
                Ok(conds) => self.eval_all(&conds, warnings).unwrap_or(false),
                Err(_) => {
                    warnings.push(format!("条件を読めません: {}", gate.when));
                    false
                }
            };
            if !passed && first_blocked.is_none() {
                first_blocked = Some((i64::try_from(index).unwrap_or(0) + 1, gate.name.clone()));
            }
            list.push(GateRow {
                name: gate.name.clone(),
                when: gate.when.clone(),
                passed,
            });
        }
        let total = i64::try_from(type_def.gates.len()).unwrap_or(0);
        let (current, name) = first_blocked.unwrap_or((total + 1, GATE_DONE_NAME.to_string()));
        Some(GateState {
            current,
            total,
            name,
            list,
        })
    }

    /// 負債の数。
    fn debts(&self) -> Debts {
        let unreflected_papers = self
            .links
            .iter()
            .filter(|link| {
                link.target_kind == "paper" && link.read_state.as_deref() == Some("read")
            })
            .count();
        // 振り返りを書いていない実行のうち、sci キーを持つもの。
        let known: HashSet<i64> = self.runs.iter().map(|run| run.run_id).collect();
        let empty_reviews = crate::attention::empty_reviews(&self.root)
            .iter()
            .filter(|item| item.run_id.is_some_and(|run| known.contains(&run)))
            .count();
        Debts {
            pending_proposals: i64::try_from(self.proposals.len()).unwrap_or(0),
            unreflected_papers: i64::try_from(unreflected_papers).unwrap_or(0),
            unscreened_sets: i64::try_from(self.unscreened.len()).unwrap_or(0),
            empty_reviews: i64::try_from(empty_reviews).unwrap_or(0),
        }
    }

    // ---- 次の一手 -----------------------------------------------------------

    /// 次の一手（負債 → 関門 → 印 → その他の空欄。同じ節点は 1 回だけ、10 件まで）。
    fn next_actions(
        &self,
        type_def: &TypeDef,
        gate: &Option<GateState>,
        debts: &Debts,
    ) -> Vec<NextAction> {
        let mut out: Vec<NextAction> = Vec::new();
        let mut seen: HashSet<i64> = HashSet::new();

        // ① 負債。
        if debts.pending_proposals > 0 {
            push_debt(
                &mut out,
                format!("PENDING_PROPOSALS が {} 件あります", debts.pending_proposals),
                None,
                None,
                "",
            );
        }
        if debts.unreflected_papers > 0 {
            push_debt(
                &mut out,
                format!(
                    "UNREFLECTED_PAPERS が {} 本あります",
                    debts.unreflected_papers
                ),
                None,
                None,
                "",
            );
        }
        for (node, name, _count) in &self.unscreened {
            push_debt(
                &mut out,
                format!("SEARCH_SET「{name}」を選別する"),
                Some(*node),
                Some("sci-screen".to_string()),
                "読む論文と外す論文",
            );
        }
        if debts.empty_reviews > 0 {
            push_debt(
                &mut out,
                format!(
                    "振り返りを書いていない実行が {} 件あります",
                    debts.empty_reviews
                ),
                None,
                None,
                "",
            );
        }

        // ② 今の関門の条件に出てくる項目と、その上流。
        for id in self.gate_items(type_def, gate) {
            self.push_blank(&mut out, &mut seen, id);
        }

        // ③ 印（根拠不足、要再確認の順に）。
        for id in &self.ordered {
            let Some(node) = self.node(*id) else {
                continue;
            };
            if node.kind != "item" || !matches!(node.state.as_str(), "decided" | "tentative") {
                continue;
            }
            for mark in self.marks_of(node) {
                if seen.contains(&node.id) {
                    break;
                }
                if let Some(action) = self.next_of(node, &mark, "mark") {
                    seen.insert(node.id);
                    out.push(action);
                    break;
                }
            }
        }

        // ④ その他の空欄。
        for id in &self.ordered {
            self.push_blank(&mut out, &mut seen, *id);
        }

        out.truncate(NEXT_LIMIT);
        out
    }

    /// 空欄のある項目を 1 行足す（同じ節点は 1 回だけ）。
    fn push_blank(&self, out: &mut Vec<NextAction>, seen: &mut HashSet<i64>, id: i64) {
        if seen.contains(&id) {
            return;
        }
        let Some(node) = self.node(id) else {
            return;
        };
        if node.kind != "item" || node.state != "open" {
            return;
        }
        let blank = self.blank_of(node);
        // 調査中は待つ（行動を勧めない）。
        if blank == "in_progress" {
            return;
        }
        if let Some(action) = self.next_of(node, &blank, "blank") {
            seen.insert(id);
            out.push(action);
        }
    }

    /// ガイドの「次の一手」から 1 行を作る（合う行が無ければ `None`）。
    fn next_of(&self, node: &Node, when: &str, kind: &str) -> Option<NextAction> {
        let guide = self.defs.guide(node.item_key.as_deref()?)?;
        let rule: &NextRule = guide.next.iter().find(|rule| rule.when == when)?;
        Some(NextAction {
            node: Some(fmt_ref('N', node.id)),
            block: rule.block.clone(),
            purpose: rule.purpose.replace("{title}", &self.display_name(node)),
            expect: rule.expect.clone(),
            why: if kind == "mark" {
                label(&MARK_LABELS, when)
            } else {
                label(&BLANK_LABELS, when)
            },
            kind: kind.to_string(),
        })
    }

    /// 今の関門の条件に出てくる項目と、その上流（型の順序 → 深さ → `ord` → id）。
    fn gate_items(&self, type_def: &TypeDef, gate: &Option<GateState>) -> Vec<i64> {
        let Some(gate) = gate else {
            return Vec::new();
        };
        let index = usize::try_from(gate.current.saturating_sub(1).max(0)).unwrap_or(0);
        let Some(row) = gate.list.get(index) else {
            return Vec::new();
        };
        let Ok(conds) = defs::parse_condition(&row.when) else {
            return Vec::new();
        };
        let mut keys: Vec<&str> = Vec::new();
        for cond in &conds {
            let key = match cond {
                Cond::Decided(key)
                | Cond::Count(key, _)
                | Cond::Saturated(key)
                | Cond::Evidence(key, _)
                | Cond::Level(key, _)
                | Cond::NotEmpty(key) => key.as_str(),
                Cond::Mind(_) => "",
            };
            if !key.is_empty() && !keys.contains(&key) {
                keys.push(key);
            }
        }
        let mut ids: Vec<i64> = Vec::new();
        for key in keys {
            let Some(item) = self.item_of_key(key) else {
                continue;
            };
            for target in std::iter::once(item.id).chain(self.upstreams(item.id)) {
                if !ids.contains(&target) {
                    ids.push(target);
                }
            }
        }
        let rank = |id: &i64| -> (usize, i64, i64, i64) {
            let top = self.top_ancestor(*id);
            let place = self
                .node(top)
                .and_then(|node| node.item_key.as_deref())
                .and_then(|key| type_def.order.iter().position(|known| known == key))
                .unwrap_or(type_def.order.len());
            (
                place,
                self.depth.get(id).copied().unwrap_or(0),
                self.node(*id).map_or(0, |node| node.ord),
                *id,
            )
        };
        ids.sort_by_key(rank);
        ids
    }

    // ---- node ---------------------------------------------------------------

    /// `node_detail` の JSON を作る。
    fn node_detail(&self, node_id: i64) -> Result<NodeDetail> {
        let node = self.need_node(node_id)?;
        let marks = self.marks_row(node);
        let findings = self.own_findings(node.id);
        let papers = self.own_paper_links(node.id);
        let mut warnings = Vec::new();

        let guide = node.item_key.as_deref().and_then(|key| {
            let found = self.defs.guide(key)?;
            let met = defs::parse_condition(&found.condition)
                .map(|conds| self.eval_all(&conds, &mut warnings).unwrap_or(false))
                .unwrap_or(false);
            Some(GuideDetail {
                key: found.key.clone(),
                name: found.name.clone(),
                explain: found.explain.clone(),
                condition: found.condition.clone(),
                condition_text: found.condition_text.clone(),
                condition_met: met,
                questions: found.questions.clone(),
                pitfalls: found.pitfalls.clone(),
                extract: found
                    .extract
                    .iter()
                    .map(|(role, fields)| ExtractRow {
                        role: role.clone(),
                        fields: fields.clone(),
                    })
                    .collect(),
            })
        });

        let questions = self
            .questions_of_node(node.id)
            .into_iter()
            .map(|question| QuestionRow {
                id: fmt_ref('Q', question.id),
                text: question.text.clone(),
                state: question.state.clone(),
                answer: question.answer.clone(),
                from_guide: question.from_guide,
            })
            .collect();

        let options = self
            .options(node.id)
            .into_iter()
            .map(|option| {
                let option_findings: Vec<&Finding> = self
                    .findings_of
                    .get(&option.id)
                    .map(|indexes| indexes.iter().map(|index| &self.findings[*index]).collect())
                    .unwrap_or_default();
                let option_papers: Vec<&Link> = self
                    .links_of
                    .get(&option.id)
                    .map(|indexes| indexes.iter().map(|index| &self.links[*index]).collect())
                    .unwrap_or_default();
                OptionRow {
                    id: fmt_ref('N', option.id),
                    title: option.title.clone(),
                    state: option.state.clone(),
                    body: option.body.clone(),
                    support: option_findings
                        .iter()
                        .filter(|finding| finding.polarity == "support")
                        .count() as i64,
                    refute: option_findings
                        .iter()
                        .filter(|finding| finding.polarity == "refute")
                        .count() as i64,
                    papers: option_papers
                        .iter()
                        .filter(|link| {
                            link.target_kind == "paper"
                                && link.read_state.as_deref() != Some("irrelevant")
                        })
                        .count() as i64,
                }
            })
            .collect();

        let finding_rows = findings
            .iter()
            .map(|finding| FindingRow {
                id: fmt_ref('F', finding.id),
                node: fmt_ref('N', finding.node_id),
                text: finding.text.clone(),
                cites: finding.cites.clone(),
                polarity: finding.polarity.clone(),
                level: finding.level,
                source: finding.source.clone(),
                report_id: finding.report_id.clone(),
                paper_id: finding.paper_id,
                run_id: finding.run_id,
                question: finding.question_id.map(|id| fmt_ref('Q', id)),
                auto: finding.auto,
                proposal: finding.proposal_id.map(|id| fmt_ref('P', id)),
                created_at: finding.created_at.clone(),
            })
            .collect();

        let link_rows = self
            .own_links(node.id)
            .into_iter()
            .map(|link| {
                let id = link.target_ref.trim().parse::<i64>().unwrap_or(0);
                LinkRow {
                    id: fmt_ref('L', link.id),
                    node: fmt_ref('N', link.node_id),
                    target: format!("{}:{}", link.target_kind, link.target_ref),
                    title: self
                        .titles
                        .get(&(link.target_kind.clone(), id))
                        .cloned()
                        .unwrap_or_else(|| link.target_ref.clone()),
                    role: link.role.clone(),
                    read_state: link.read_state.clone(),
                    auto: link.auto,
                    proposal: link.proposal_id.map(|id| fmt_ref('P', id)),
                    reason: link.reason.clone(),
                    created_at: link.created_at.clone(),
                }
            })
            .collect();

        let decisions = self
            .decisions_of_node(node.id)
            .into_iter()
            .map(|decision| DecisionRow {
                action: decision.action.clone(),
                option: decision.option_id.map(|id| fmt_ref('N', id)),
                reason: decision.reason.clone(),
                evidence: decision.evidence.iter().map(|id| fmt_ref('F', *id)).collect(),
                rejected: decision
                    .rejected
                    .iter()
                    .map(|rejected| RejectedRow {
                        option: fmt_ref('N', rejected.option),
                        reason: rejected.reason.clone(),
                    })
                    .collect(),
                counter_searched: decision.counter_searched,
                actor: decision.actor.clone(),
                created_at: decision.created_at.clone(),
            })
            .collect();

        let actions = self
            .runs
            .iter()
            .filter(|run| run.keys.node == Some(node.id))
            .map(|run| ActionRow {
                run_id: run.run_id,
                block: run.block.clone(),
                owner: run.owner.clone(),
                runner: run.runner.clone(),
                purpose: run.keys.purpose.clone(),
                created: run.created.clone(),
            })
            .collect();

        let mut searched: HashSet<i64> = HashSet::new();
        for search in &self.searches {
            if search.node_id == Some(node.id) {
                searched.extend(search.hit_papers.iter().copied());
            }
        }
        let funnel = Funnel {
            searched: i64::try_from(searched.len()).unwrap_or(0),
            screened: self.screened.get(&node.id).copied().unwrap_or(0),
            read: papers
                .iter()
                .filter(|link| matches!(link.read_state.as_deref(), Some("read") | Some("reflected")))
                .count() as i64,
            reflected: papers
                .iter()
                .filter(|link| link.read_state.as_deref() == Some("reflected"))
                .count() as i64,
        };

        let pending = self
            .proposals
            .iter()
            .filter(|proposal| {
                proposal
                    .node_id
                    .is_some_and(|target| self.with_options(node.id).contains(&target))
            })
            .map(|proposal| PendingRow {
                id: fmt_ref('P', proposal.id),
                op: proposal.op.clone(),
                payload: proposal.payload.clone(),
                reason: proposal.reason.clone(),
                run_id: proposal.run_id,
                block: proposal.block.clone(),
                created_at: proposal.created_at.clone(),
            })
            .collect();

        // 使うブロックは 3 つまで（同じブロックは 1 回）。
        let wants: Vec<String> = self
            .blank_of_row(node)
            .into_iter()
            .chain(marks.iter().cloned())
            .collect();
        let blocks = self
            .blocks_for(node, &wants, &marks);

        let searches = self
            .searches
            .iter()
            .filter(|search| search.node_id == Some(node.id))
            .map(|search| SearchRow {
                query: search.query.clone(),
                scope: search.scope.clone(),
                hits: search.hits,
                created_at: search.created_at.clone(),
            })
            .collect();

        Ok(NodeDetail {
            node: self.node_row(node, true),
            guide,
            questions,
            options,
            findings: finding_rows,
            links: link_rows,
            decisions,
            actions,
            funnel,
            pending,
            blocks,
            searches,
            axes: self.axes(),
        })
    }

    /// 空欄の種類・印に合う「次の一手」の行（3 つまで、同じブロックは 1 回）。
    fn blocks_for(&self, node: &Node, wants: &[String], marks: &[String]) -> Vec<BlockRow> {
        let Some(guide) = self
            .node_item_key(node)
            .and_then(|key| self.defs.guide(key))
        else {
            return Vec::new();
        };
        let mut rows: Vec<BlockRow> = Vec::new();
        let mut names: Vec<String> = Vec::new();
        for rule in &guide.next {
            if !wants.contains(&rule.when) {
                continue;
            }
            let name = rule.block.clone().unwrap_or_default();
            if names.contains(&name) {
                continue;
            }
            names.push(name);
            rows.push(BlockRow {
                block: rule.block.clone(),
                purpose: rule.purpose.replace("{title}", &self.display_name(node)),
                expect: rule.expect.clone(),
                why: if marks.contains(&rule.when) {
                    label(&MARK_LABELS, &rule.when)
                } else {
                    label(&BLANK_LABELS, &rule.when)
                },
            });
            if rows.len() == 3 {
                break;
            }
        }
        rows
    }

    /// 項目のキー（候補なら `None`）。
    fn node_item_key<'a>(&'a self, node: &'a Node) -> Option<&'a str> {
        node.item_key.as_deref()
    }

    /// 価値の軸（mind.md の「価値の軸」の節の行）。
    fn axes(&self) -> Vec<String> {
        let Some((_, body)) = self.sections.iter().find(|(name, _)| name == "VALUE_AXES") else {
            return Vec::new();
        };
        body.lines()
            .map(|line| {
                let trimmed = line.trim();
                for mark in ["1.", "2.", "3.", "4.", "-", "*"] {
                    if let Some(rest) = trimmed.strip_prefix(mark) {
                        return rest.trim().to_string();
                    }
                }
                trimmed.to_string()
            })
            .filter(|line| !line.is_empty() && !line.starts_with("<!--"))
            .collect()
    }
}

/// 文字の部分を 1 つ足す。
fn push_literal(parts: &mut Vec<SeedPart>, text: &str) {
    if text.is_empty() {
        return;
    }
    parts.push(SeedPart {
        text: text.to_string(),
        key: None,
        node: None,
        filled: true,
        tentative: false,
    });
}

/// 負債の 1 行を足す。
fn push_debt(
    out: &mut Vec<NextAction>,
    purpose: String,
    node: Option<i64>,
    block: Option<String>,
    expect: &str,
) {
    out.push(NextAction {
        node: node.map(|id| fmt_ref('N', id)),
        block,
        purpose,
        expect: expect.to_string(),
        why: DEBT_WHY.to_string(),
        kind: "debt".to_string(),
    });
}

// ---- SQL を 1 度だけ投げる ---------------------------------------------------

/// 選別していない検索セット（`unscreened_sets` の負債に使う）。
fn unscreened_sets(conn: &Connection) -> Result<Vec<(i64, String, i64)>> {
    let mut statement = conn.prepare(
        "SELECT s.id, s.name, s.sci_node FROM search_sets s
          WHERE s.sci_node IS NOT NULL
            AND EXISTS (SELECT 1 FROM search_set_items i
                         WHERE i.set_id = s.id AND i.decision = 'pending')
          ORDER BY s.id",
    )?;
    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, i64>(2)?,
        ))
    })?;
    let mut found = Vec::new();
    for row in rows {
        found.push(row?);
    }
    Ok(found)
}

/// 節点ごとの選別済みの候補の数（`funnel.screened`）。
fn screened_counts(conn: &Connection) -> Result<HashMap<i64, i64>> {
    let mut statement = conn.prepare(
        "SELECT s.sci_node, COUNT(i.dedupe_key) FROM search_sets s
           JOIN search_set_items i ON i.set_id = s.id
          WHERE s.sci_node IS NOT NULL AND i.decision != 'pending'
          GROUP BY s.sci_node",
    )?;
    let rows = statement.query_map([], |row| {
        Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?))
    })?;
    let mut found = HashMap::new();
    for row in rows {
        let (node, count) = row?;
        found.insert(node, count);
    }
    Ok(found)
}

/// 結び付きの相手（論文と report）の題。
fn link_titles(conn: &Connection, links: &[Link]) -> Result<HashMap<(String, i64), String>> {
    let mut papers: HashSet<i64> = HashSet::new();
    let mut reports: HashSet<i64> = HashSet::new();
    for link in links {
        let Ok(id) = link.target_ref.trim().parse::<i64>() else {
            continue;
        };
        match link.target_kind.as_str() {
            "paper" => {
                papers.insert(id);
            }
            "report" => {
                reports.insert(id);
            }
            _ => {}
        }
    }
    let mut found = HashMap::new();
    if !papers.is_empty() {
        let mut statement = conn.prepare("SELECT id, title FROM papers")?;
        let rows = statement.query_map([], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })?;
        for row in rows {
            let (id, title) = row?;
            if papers.contains(&id) {
                found.insert(("paper".to_string(), id), title);
            }
        }
    }
    if !reports.is_empty() {
        // report の id は文字列なので、数で読んで元の文字列のキーに戻す。
        let mut statement = conn.prepare("SELECT CAST(id AS INTEGER), title FROM reports")?;
        let rows = statement.query_map([], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, Option<String>>(1)?))
        })?;
        for row in rows {
            let (id, title) = row?;
            if reports.contains(&id) {
                found.insert(("report".to_string(), id), title.unwrap_or_default());
            }
        }
    }
    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::Database;
    use crate::sci::store::{NewDecision, NewFinding, NewLink, NewNode, NewProposal};
    use rusqlite::params;

    /// テスト用の型（関門 2 つ、種の文 1 つ）。
    const TYPE_MD: &str = r#"---
id: problem
name: 課題駆動
given: background
---
# 課題駆動

説明。

## 項目の順序
- background.goal
- target
- data
- purpose

## 種の文
{target}の{data.type}で{purpose.subgoal}を行う。

## 関門
- 出発点: decided(background.goal)
- PURPOSE_SUBGOAL_DECIDED: decided(purpose.subgoal)
"#;

    /// テストに使う項目キー。
    const KEYS: [&str; 8] = [
        "background.goal",
        "target",
        "data",
        "data.type",
        "purpose",
        "purpose.subgoal",
        "existing",
        "issue.limit",
    ];

    /// 一番上の項目の並び。
    const TOP: [&str; 6] = [
        "background.goal",
        "target",
        "data",
        "purpose",
        "existing",
        "issue.limit",
    ];

    /// 項目ガイドの md を作る（種類だけ違わせる）。
    fn guide_md(key: &str) -> String {
        let (name, kind) = match key {
            "background.goal" => ("出発点", "write"),
            "target" => ("対象", "choose"),
            "data" => ("データ", "write"),
            "data.type" => ("データの型", "choose"),
            "purpose" => ("目的", "write"),
            "purpose.subgoal" => ("PURPOSE_SUBGOAL", "choose"),
            "existing" => ("既存の手段", "choose"),
            _ => ("限界", "verify"),
        };
        format!(
            "---\nkey: {key}\nname: {name}\ntype: {kind}\nmin_findings: 2\n---\n# {name}\n\n説明。\n\n## 問い\n- 問うことは何か\n\n## 埋まった条件\n- decided({key})\n\n埋まった説明。\n\n## 抜き出す項目\n- exemplar: 何を達成したか\n\n## 落とし穴\n- よくある誤り\n\n## 次の一手\n- untouched: sci-ideate | {{title}}の候補を出す | 候補の一覧\n- insufficient: sci-search | {{title}}の候補を広げて探す | 追加した候補\n- awaiting_decision: sci-compare | {{title}}の候補を比べる | 比較表と推奨\n- conflict: sci-probe | {{title}}の食い違いを調べる | 理由の説明\n- weak_evidence: sci-probe | {{title}}を確かめる | 確かめた結果\n- recheck: - | 上流が変わったので{{title}}を見直す | 見直した結果\n"
        )
    }

    /// 短い md を読んだ `Defs`（埋め込みは使わない）。
    fn defs() -> Defs {
        Defs {
            types: vec![defs::parse_type(TYPE_MD).expect("型を読む")],
            guides: KEYS
                .iter()
                .map(|key| defs::parse_guide(&guide_md(key)).expect("ガイドを読む"))
                .collect(),
            warnings: Vec::new(),
        }
    }

    /// テスト用の一時プロジェクトルート。
    fn temp_root(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "sai-paper-compute-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join(".sai/paper")).expect("一時フォルダ");
        fs::create_dir_all(dir.join(".sai/outputs")).expect("実行フォルダ");
        dir
    }

    /// テスト用の `PaperRoots`。
    fn roots_of(root: &Path) -> PaperRoots {
        PaperRoots {
            root: root.to_path_buf(),
            data_dir: root.join(".sai/paper"),
        }
    }

    /// ファイルを書く（親フォルダは作る）。
    fn write(path: PathBuf, text: &str) {
        fs::create_dir_all(path.parent().expect("親フォルダ")).expect("親フォルダを作る");
        fs::write(path, text).expect("書く");
    }

    /// 実行フォルダを作る（sci キー付きの `task.md` 付き）。
    fn write_run(root: &Path, owner: &str, run_id: i64, node: i64) {
        write(
            root.join(".sai/outputs")
                .join(owner)
                .join(run_id.to_string())
                .join("task.md"),
            &format!(
                "---\nrunner: pi\ncreation_date: 2026/09/27 10:00:00\nrun_id: {run_id}\nblock: sci-search\nsci_node: N{node}\nsci_purpose: 候補を探す\n---\n\n# task_text\n\n候補を探す\n"
            ),
        );
    }

    /// 項目を作る。
    fn item(conn: &Connection, key: &str, ord: i64) -> i64 {
        let guide = defs().guide(key).expect("ガイド").clone();
        store::insert_node(
            conn,
            &NewNode {
                kind: "item",
                item_key: Some(key),
                types: &guide.types,
                title: &guide.name,
                body: "",
                state: "open",
                fixed: false,
                ord,
                created_by: "user",
            },
        )
        .expect("項目を作る")
    }

    /// キーの無い項目を作る（並びの確認用）。
    fn plain_item(conn: &Connection, title: &str, ord: i64) -> i64 {
        store::insert_node(
            conn,
            &NewNode {
                kind: "item",
                item_key: None,
                types: &["write".to_string()],
                title,
                body: "",
                state: "open",
                fixed: false,
                ord,
                created_by: "user",
            },
        )
        .expect("項目を作る")
    }

    /// 項目キーで項目番号を引く。
    fn key_id(conn: &Connection, key: &str) -> i64 {
        store::find_item_by_key(conn, key)
            .expect("項目を探す")
            .expect("項目がある")
            .id
    }

    /// 候補を作る。
    fn option(conn: &Connection, parent: i64, title: &str, ord: i64) -> i64 {
        let id = store::insert_node(
            conn,
            &NewNode {
                kind: "option",
                item_key: None,
                types: &[],
                title,
                body: "",
                state: "candidate",
                fixed: false,
                ord,
                created_by: "user",
            },
        )
        .expect("候補を作る");
        store::insert_edge(conn, "parent", parent, id).expect("親の線");
        id
    }

    /// 知見を 1 つ足す。
    fn finding(conn: &Connection, node: i64, polarity: &str, level: Option<i64>) -> i64 {
        store::insert_finding(
            conn,
            &NewFinding {
                node_id: node,
                question_id: None,
                text: "知見の文",
                cites: &[],
                polarity,
                level,
                source: "user",
                report_id: None,
                paper_id: None,
                run_id: None,
                proposal_id: None,
                auto: false,
                created_by: "user",
            },
        )
        .expect("知見を足す")
    }

    /// 候補の状態を変える。
    fn set_state(conn: &Connection, id: i64, state: &str) {
        store::set_node_state(conn, id, state).expect("状態を変える");
    }

    /// 節点 1 つについての `status` の行。
    fn row_of(conn: &Connection, roots: &PaperRoots, node: i64) -> NodeRow {
        status(conn, roots, &defs())
            .expect("status")
            .nodes
            .into_iter()
            .find(|row| row.id == fmt_ref('N', node))
            .expect("行がある")
    }

    /// 空欄の種類を 1 つの項目について求める。
    fn blank_of(conn: &Connection, roots: &PaperRoots, node: i64) -> String {
        row_of(conn, roots, node).blank.unwrap_or_default()
    }

    /// 木を作る（型の順序どおりに一番上の項目と、その 2 つの子）。
    fn build_tree(conn: &Connection) {
        store::meta_set(conn, "type", "problem").expect("型を書く");
        for (ord, key) in TOP.iter().enumerate() {
            item(conn, key, i64::try_from(ord).expect("序数"));
        }
        for (parent_key, child_key) in [("data", "data.type"), ("purpose", "purpose.subgoal")] {
            let child = item(conn, child_key, 0);
            store::insert_edge(conn, "parent", key_id(conn, parent_key), child).expect("親の線");
        }
    }

    // ---- 空欄の種類 --------------------------------------------------------

    #[test]
    fn blank_untouched_then_insufficient() {
        let database = Database::in_memory().expect("メモリ DB");
        let conn = database.connection();
        let root = temp_root("untouched");
        let roots = roots_of(&root);
        build_tree(conn);
        let data = key_id(conn, "data");

        // 何も無い項目は未着手。
        assert_eq!(blank_of(conn, &roots, data), "untouched");

        // 知見を 1 つ足すと「情報不足」（min_findings は 2）。
        finding(conn, data, "support", None);
        assert_eq!(blank_of(conn, &roots, data), "insufficient");

        // 2 つで「判断待ち」（選ぶ項目でないので合計で数える）。
        finding(conn, data, "neutral", None);
        assert_eq!(blank_of(conn, &roots, data), "awaiting_decision");

        // 支持と反証が両方そろったら「対立」。
        finding(conn, data, "refute", None);
        assert_eq!(blank_of(conn, &roots, data), "conflict");

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn blank_in_progress_by_running_run() {
        let database = Database::in_memory().expect("メモリ DB");
        let conn = database.connection();
        let root = temp_root("running");
        let roots = roots_of(&root);
        build_tree(conn);
        let target = key_id(conn, "target");
        let option = option(conn, target, "てんかん患者", 0);
        finding(conn, option, "support", None);
        finding(conn, option, "support", None);
        assert_eq!(blank_of(conn, &roots, target), "awaiting_decision");

        // todo の実行と結ぶと調査中。
        write_run(&root, "todo", 7, target);
        store::upsert_link(
            conn,
            &NewLink {
                node_id: target,
                target_kind: "run",
                target_ref: "7",
                role: None,
                read_state: None,
                reason: "",
                proposal_id: None,
                auto: false,
                created_by: "user",
            },
        )
        .expect("結ぶ");
        assert_eq!(blank_of(conn, &roots, target), "in_progress");

        // review に移くと外れる（走っていないので次は「判断待ち」）。
        fs::create_dir_all(root.join(".sai/outputs/review")).expect("置き場を作る");
        fs::rename(
            root.join(".sai/outputs/todo/7"),
            root.join(".sai/outputs/review/7"),
        )
        .expect("置き場を移す");
        assert_eq!(blank_of(conn, &roots, target), "awaiting_decision");

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn blank_in_progress_by_pending_proposal() {
        let database = Database::in_memory().expect("メモリ DB");
        let conn = database.connection();
        let root = temp_root("proposal");
        let roots = roots_of(&root);
        build_tree(conn);
        let target = key_id(conn, "target");
        let option = option(conn, target, "てんかん患者", 0);
        finding(conn, option, "support", None);
        finding(conn, option, "support", None);
        assert_eq!(blank_of(conn, &roots, target), "awaiting_decision");

        // 候補を消す提案が仕分け待ちなら調査中。
        store::insert_proposal(
            conn,
            &NewProposal {
                run_id: None,
                block: "sci-ideate",
                op: "add_option",
                node_id: Some(option),
                payload: &serde_json::json!({"title": "新しい候補"}),
                reason: "候補が出たら足す",
                risk: "human",
                status: "pending",
            },
        )
        .expect("提案を足す");
        assert_eq!(blank_of(conn, &roots, target), "in_progress");

        // 仕分けが終われば外れる。
        let proposal = store::list_proposals(conn, Some("pending")).unwrap();
        store::finish_proposal(conn, proposal[0].id, "rejected", None, Some("user"), "不要")
            .expect("仕分ける");
        assert_eq!(blank_of(conn, &roots, target), "awaiting_decision");

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn blank_conflict_when_support_and_refute() {
        let database = Database::in_memory().expect("メモリ DB");
        let conn = database.connection();
        let root = temp_root("conflict");
        let roots = roots_of(&root);
        build_tree(conn);
        let target = key_id(conn, "target");
        let option = option(conn, target, "てんかん患者", 0);
        finding(conn, option, "support", None);
        assert_eq!(blank_of(conn, &roots, target), "insufficient");
        finding(conn, option, "refute", None);
        assert_eq!(blank_of(conn, &roots, target), "conflict");

        // 外した反証は数えない。
        let refute = store::list_findings(conn, Some(option), false).unwrap()[1].id;
        store::remove_finding(conn, refute).expect("外す");
        assert_eq!(blank_of(conn, &roots, target), "insufficient");

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn blank_awaiting_decision_for_choose_item() {
        let database = Database::in_memory().expect("メモリ DB");
        let conn = database.connection();
        let root = temp_root("choose");
        let roots = roots_of(&root);
        build_tree(conn);
        let target = key_id(conn, "target");
        let first = option(conn, target, "てんかん患者", 0);

        // 候補が 1 つだけなら、知見が min_findings（2）要る。
        finding(conn, first, "support", None);
        assert_eq!(blank_of(conn, &roots, target), "insufficient");
        finding(conn, first, "support", None);
        assert_eq!(blank_of(conn, &roots, target), "awaiting_decision");

        // 候補が 2 つなら、どの候補にも 1 つあればよい。
        let second = option(conn, target, "健常者", 1);
        assert_eq!(blank_of(conn, &roots, target), "insufficient");
        finding(conn, second, "support", None);
        assert_eq!(blank_of(conn, &roots, target), "awaiting_decision");

        let _ = fs::remove_dir_all(&root);
    }

    // ---- 印 ----------------------------------------------------------------

    #[test]
    fn marks_weak_evidence_and_recheck() {
        let database = Database::in_memory().expect("メモリ DB");
        let conn = database.connection();
        let root = temp_root("marks");
        let roots = roots_of(&root);
        build_tree(conn);
        let goal = key_id(conn, "background.goal");
        let data = key_id(conn, "data");
        store::insert_edge(conn, "depends", goal, data).expect("依存の線");
        set_state(conn, data, "decided");

        // 反証を探していない決定の記録なので根拠不足。
        store::insert_decision(
            conn,
            &NewDecision {
                node_id: data,
                action: "decide",
                option_id: None,
                reason: "決めた",
                evidence: &[],
                rejected: &[],
                counter_searched: false,
                actor: "user",
                proposal_id: None,
            },
        )
        .expect("決定の記録");
        assert_eq!(row_of(conn, &roots, data).marks, vec!["weak_evidence"]);

        // 支持の知見を足しても、記録の counter_searched が 0 の間は根拠不足のまま。
        finding(conn, data, "support", None);
        assert_eq!(row_of(conn, &roots, data).marks, vec!["weak_evidence"]);

        // 決定だけを足し直して反証を探した記録が最新なら、根拠不足は外れる。
        store::insert_decision(
            conn,
            &NewDecision {
                node_id: data,
                action: "decide",
                option_id: None,
                reason: "もう一度",
                evidence: &[],
                rejected: &[],
                counter_searched: true,
                actor: "user",
                proposal_id: None,
            },
        )
        .expect("決定の記録");
        assert!(row_of(conn, &roots, data).marks.is_empty());

        // 上流が後から変わると要再確認。
        conn.execute(
            "UPDATE sci_nodes SET state_changed_at = '2999-01-01 00:00:00' WHERE id = ?1",
            params![goal],
        )
        .expect("上流を変える");
        assert_eq!(row_of(conn, &roots, data).marks, vec!["recheck"]);

        let _ = fs::remove_dir_all(&root);
    }

    // ---- 関門と種の文 ------------------------------------------------------

    #[test]
    fn gate_current_and_seed_parts() {
        let database = Database::in_memory().expect("メモリ DB");
        let conn = database.connection();
        let root = temp_root("gate");
        let roots = roots_of(&root);
        build_tree(conn);
        let goal = key_id(conn, "background.goal");
        let target = key_id(conn, "target");
        let data_type = key_id(conn, "data.type");
        let subgoal = key_id(conn, "purpose.subgoal");

        // 1 つめの関門だけ通す。
        set_state(conn, goal, "decided");
        let result = status(conn, &roots, &defs()).expect("status");
        let gate = result.gate.as_ref().expect("関門").clone();
        assert_eq!(gate.total, 2);
        assert_eq!(gate.current, 2);
        assert_eq!(gate.name, "PURPOSE_SUBGOAL_DECIDED");
        assert!(gate.list[0].passed && !gate.list[1].passed);

        // 2 つとも通せば「種にできる」。
        set_state(conn, subgoal, "decided");
        let result = status(conn, &roots, &defs()).expect("status");
        let gate = result.gate.as_ref().expect("関門");
        assert_eq!(gate.current, 3);
        assert_eq!(gate.name, "種にできる");

        // 仮決定の値には「（仮）」が付く。
        set_state(conn, subgoal, "tentative");
        set_state(conn, target, "decided");
        let selected = option(conn, target, "頭皮脳波", 0);
        set_state(conn, selected, "selected");
        set_state(conn, data_type, "tentative");
        let kind = option(conn, data_type, "動脈瘤", 0);
        set_state(conn, kind, "selected");
        let result = status(conn, &roots, &defs()).expect("status");
        let seed = result.seed.expect("種の文");
        assert_eq!(seed.text, "「頭皮脳波の動脈瘤（仮）で［？PURPOSE_SUBGOAL］を行う。」");
        // 差し込み 3 つ＋文字 3 つ。
        assert_eq!(seed.parts.len(), 6);
        assert_eq!(seed.parts[0].text, "頭皮脳波");
        assert_eq!(seed.parts[0].key.as_deref(), Some("target"));
        assert_eq!(seed.parts[0].node.as_deref(), Some(&fmt_ref('N', target)[..]));
        assert!(seed.parts[0].filled && !seed.parts[0].tentative);
        assert_eq!(seed.parts[1].text, "の");
        assert_eq!(seed.parts[1].key, None);
        assert_eq!(seed.parts[2].text, "動脈瘤（仮）");
        assert_eq!(seed.parts[2].key.as_deref(), Some("data.type"));
        assert!(seed.parts[2].filled && seed.parts[2].tentative);
        assert_eq!(seed.parts[4].text, "［？PURPOSE_SUBGOAL］");
        assert_eq!(seed.parts[4].node.as_deref(), Some(&fmt_ref('N', subgoal)[..]));
        assert!(!seed.parts[4].filled);

        // 行の「値」は空欄のときは空文字。
        let row = row_of(conn, &roots, target);
        assert_eq!(row.value, "頭皮脳波");

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn gate_warns_when_key_is_missing() {
        let database = Database::in_memory().expect("メモリ DB");
        let conn = database.connection();
        let root = temp_root("gate-warn");
        let roots = roots_of(&root);
        // 木を作らない（background.goal が無い）。
        store::meta_set(conn, "type", "problem").expect("型を書く");
        let result = status(conn, &roots, &defs()).expect("status");
        assert_eq!(result.gate.expect("関門").current, 1);
        assert!(
            result
                .warnings
                .iter()
                .any(|line| line.contains("GATE の条件の ITEM がありません: background.goal")),
            "{:?}",
            result.warnings
        );

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn saturated_needs_two_searches_after_newest_option_and_exemplars() {
        let database = Database::in_memory().expect("メモリ DB");
        let conn = database.connection();
        let root = temp_root("saturated");
        let roots = roots_of(&root);
        store::meta_set(conn, "type", "problem").unwrap();
        let existing = item(conn, "existing", 0);
        let first = option(conn, existing, "定位手法", 0);
        let second = option(conn, existing, "発作予測", 1);
        // 候補のできた時刻より後の検索。
        set_created_at(conn, first, "2026-08-01 00:00:00");
        set_created_at(conn, second, "2026-08-02 00:00:00");
        add_search(conn, existing, "2026-09-01 10:00:00");
        add_search(conn, existing, "2026-09-02 10:00:00");
        let mut warnings = Vec::new();
        let conds = defs::parse_condition("saturated(existing)").expect("条件を読む");

        // 代表例が無ければ偽。
        assert!(!eval_conditions(conn, &roots, &conds, &mut warnings).unwrap());
        // 候補のうち 1 つしか代表例が無いときも偽。
        exemplar(conn, first);
        assert!(!eval_conditions(conn, &roots, &conds, &mut warnings).unwrap());
        exemplar(conn, second);
        assert!(eval_conditions(conn, &roots, &conds, &mut warnings).unwrap());
        assert!(warnings.is_empty(), "{warnings:?}");

        // いちばん新しい候補より前の検索しか無ければ偽。
        set_created_at(conn, second, "2026-09-05 00:00:00");
        let mut warnings = Vec::new();
        assert!(!eval_conditions(conn, &roots, &conds, &mut warnings).unwrap());
        // 3 つ目を足しても newest より後 1 回なので偽。
        add_search(conn, existing, "2026-09-06 00:00:00");
        let mut warnings = Vec::new();
        assert!(!eval_conditions(conn, &roots, &conds, &mut warnings).unwrap());
        // 4 つ目も入れば newest のあとに 2 回なので真。
        add_search(conn, existing, "2026-09-07 00:00:00");
        let mut warnings = Vec::new();
        assert!(eval_conditions(conn, &roots, &conds, &mut warnings).unwrap());

        // 候補が 0 個なら偽。
        set_state(conn, first, "rejected");
        set_state(conn, second, "rejected");
        let mut warnings = Vec::new();
        assert!(!eval_conditions(conn, &roots, &conds, &mut warnings).unwrap());

        let _ = fs::remove_dir_all(&root);
    }

    /// 検索の記録を 1 件入れる（時刻を指定する）。
    fn add_search(conn: &Connection, node: i64, created_at: &str) {
        conn.execute(
            "INSERT INTO sci_searches (node_id, tool, query, scope, hits, hit_papers, created_at)
             VALUES (?1, 'search', 'modifier', 'library', 0, '[]', ?2)",
            params![node, created_at],
        )
        .expect("検索の記録を足す");
    }

    /// 代表例の論文を 1 つ結ぶ。
    fn exemplar(conn: &Connection, node: i64) {
        store::upsert_link(
            conn,
            &NewLink {
                node_id: node,
                target_kind: "paper",
                target_ref: "37",
                role: Some("exemplar"),
                read_state: Some("unread"),
                reason: "",
                proposal_id: None,
                auto: false,
                created_by: "user",
            },
        )
        .expect("代表例を結ぶ");
    }

    /// 節点の `created_at` を書き換える。
    fn set_created_at(conn: &Connection, id: i64, created_at: &str) {
        conn.execute(
            "UPDATE sci_nodes SET created_at = ?2 WHERE id = ?1",
            params![id, created_at],
        )
        .expect("created_at を書く");
    }

    // ---- 負債 --------------------------------------------------------------

    #[test]
    fn debts_count_each_kind() {
        let database = Database::in_memory().expect("メモリ DB");
        let conn = database.connection();
        let root = temp_root("debts");
        let roots = roots_of(&root);
        build_tree(conn);
        let target = key_id(conn, "target");

        // 仕分けていない提案。
        store::insert_proposal(
            conn,
            &NewProposal {
                run_id: None,
                block: "sci-search",
                op: "add_finding",
                node_id: Some(target),
                payload: &serde_json::json!({}),
                reason: "あとで",
                risk: "human",
                status: "pending",
            },
        )
        .expect("提案を足す");
        // 読んだが反映していない論文。
        store::upsert_link(
            conn,
            &NewLink {
                node_id: target,
                target_kind: "paper",
                target_ref: "37",
                role: Some("exemplar"),
                read_state: Some("read"),
                reason: "",
                proposal_id: None,
                auto: false,
                created_by: "user",
            },
        )
        .expect("結ぶ");
        // 選別していない検索セット（選び終わったものは数えない）。
        add_search_set(conn, "先行研究", Some(target), &["pending", "pending"]);
        add_search_set(conn, "選び終わった集合", Some(target), &["keep"]);

        // 振り返りを書いていない実行（研究用ブロック・sci キー持ち）。
        write(
            root.join(".sai/blocks/sci-search.md"),
            "---\nname: sci-search\ntemplates: project-brief\n---\n\n本文\n",
        );
        write_run(&root, "review", 9, target);
        write(
            root.join(".sai/outputs/review/9/review.md"),
            "---\nrun_id: 9\n---\n\n## 振り返り\n\n### できたこと\n\n### できなかったこと\n",
        );

        let result = status(conn, &roots, &defs()).expect("status");
        assert_eq!(result.debts.pending_proposals, 1);
        assert_eq!(result.debts.unreflected_papers, 1);
        assert_eq!(result.debts.unscreened_sets, 1);
        assert_eq!(result.debts.empty_reviews, 1);

        // まだ走っている実行は数えない。
        write_run(&root, "todo", 10, target);
        write(
            root.join(".sai/outputs/todo/10/review.md"),
            "---\nrun_id: 10\n---\n\n## 振り返り\n\n### できたこと\n\n### できなかったこと\n",
        );
        let result = status(conn, &roots, &defs()).expect("status");
        assert_eq!(result.debts.empty_reviews, 1);

        let _ = fs::remove_dir_all(&root);
    }

    /// 検索セットと候補を入れる。
    fn add_search_set(conn: &Connection, name: &str, node: Option<i64>, decisions: &[&str]) {
        conn.execute(
            "INSERT INTO search_sets (name, source_dbs, query, sci_node) VALUES (?1, '[]', 'q', ?2)",
            params![name, node],
        )
        .expect("検索セットを足す");
        let set_id = conn.last_insert_rowid();
        for (index, decision) in decisions.iter().enumerate() {
            conn.execute(
                "INSERT INTO search_set_items (set_id, position, remote_json, dedupe_key, decision)
                 VALUES (?1, ?2, '{}', ?3, ?4)",
                params![set_id, i64::try_from(index).unwrap_or(0), name, decision],
            )
            .expect("候補を足す");
        }
    }

    // ---- 次の一手 ----------------------------------------------------------

    #[test]
    fn next_orders_debt_gate_mark_other_and_caps_ten() {
        let database = Database::in_memory().expect("メモリ DB");
        let conn = database.connection();
        let root = temp_root("next");
        let roots = roots_of(&root);
        build_tree(conn);
        let goal = key_id(conn, "background.goal");
        let data = key_id(conn, "data");

        // 負債を 1 つだけ作る。
        store::insert_proposal(
            conn,
            &NewProposal {
                run_id: None,
                block: "sci-search",
                op: "add_finding",
                node_id: Some(data),
                payload: &serde_json::json!({}),
                reason: "あとで",
                risk: "human",
                status: "pending",
            },
        )
        .expect("提案を足す");

        let result = status(conn, &roots, &defs()).expect("status");
        // 負債が 1 番目、関門の項目が 2 番目。
        assert_eq!(result.next[0].kind, "debt");
        assert_eq!(result.next[0].why, "DEBT");
        assert!(result.next[0].purpose.contains("PENDING_PROPOSALS"));
        assert_eq!(result.next[1].node.as_deref(), Some("N1"));
        assert_eq!(result.next[1].kind, "blank");
        assert_eq!(result.next[1].why, "UNTOUCHED");
        assert_eq!(result.next[1].purpose, "出発点の候補を出す");
        assert_eq!(result.next[1].block.as_deref(), Some("sci-ideate"));

        // 印のある項目は印の行になる。
        set_state(conn, goal, "decided");
        set_state(conn, data, "decided");
        let result = status(conn, &roots, &defs()).expect("status");
        let marked: Vec<&NextAction> = result
            .next
            .iter()
            .filter(|action| action.kind == "mark")
            .collect();
        assert!(!marked.is_empty());
        assert_eq!(marked[0].why, "WEAK_EVIDENCE");
        // 節点の並びで 1 番目（出発点）の行が先に来る。
        assert_eq!(marked[0].node.as_deref(), Some("N1"));
        assert_eq!(marked[0].purpose, "出発点を確かめる");
        assert!(
            marked.iter().any(|action| action.purpose == "データを確かめる"),
            "{:?}",
            marked.iter().map(|a| a.purpose.clone()).collect::<Vec<_>>()
        );

        // 10 件で止まり、同じ節点は 2 回出ない。
        for ord in 6..20 {
            plain_item(conn, &format!("余りの項目 {ord}"), ord);
        }
        let result = status(conn, &roots, &defs()).expect("status");
        assert!(result.next.len() <= 10, "{}", result.next.len());
        let mut nodes: Vec<&str> = result.next.iter().filter_map(|a| a.node.as_deref()).collect();
        let before = nodes.len();
        nodes.sort_unstable();
        nodes.dedup();
        assert_eq!(before, nodes.len(), "同じ節点が 2 回出ている");

        // 調査中の項目は出さない。
        let other = key_id(conn, "purpose");
        write_run(&root, "running", 20, other);
        store::upsert_link(
            conn,
            &NewLink {
                node_id: other,
                target_kind: "run",
                target_ref: "20",
                role: None,
                read_state: None,
                reason: "",
                proposal_id: None,
                auto: false,
                created_by: "user",
            },
        )
        .expect("結ぶ");
        let result = status(conn, &roots, &defs()).expect("status");
        assert_eq!(blank_of(conn, &roots, other), "in_progress");
        assert!(
            !result
                .next
                .iter()
                .any(|action| action.node.as_deref() == Some(&fmt_ref('N', other)[..])),
            "調査中の項目が次の一手に出る"
        );
        // 走っている実行も出る。
        assert_eq!(result.running.len(), 1);
        assert_eq!(result.running[0].owner, "running");
        assert_eq!(result.running[0].node.as_deref(), Some(&fmt_ref('N', other)[..]));

        let _ = fs::remove_dir_all(&root);
    }

    // ---- node --------------------------------------------------------------

    #[test]
    fn node_detail_funnel_and_blocks() {
        let database = Database::in_memory().expect("メモリ DB");
        let conn = database.connection();
        let root = temp_root("node");
        let roots = roots_of(&root);
        build_tree(conn);
        let target = key_id(conn, "target");
        let first = option(conn, target, "てんかん患者", 0);
        let second = option(conn, target, "健常者", 1);
        let question =
            store::insert_question(conn, target, "誰を対象にするか", true, "guide").expect("問う");
        finding(conn, first, "support", Some(2));
        exemplar(conn, first);
        conn.execute(
            "UPDATE sci_links SET read_state = 'reflected' WHERE node_id = ?1",
            params![first],
        )
        .expect("反映済みにする");
        add_search_set(conn, "先行研究", Some(target), &["pending", "keep", "keep"]);
        add_search(conn, target, "2026-09-01 10:00:00");
        conn.execute(
            "UPDATE sci_searches SET hit_papers = '[37, 40]' WHERE node_id = ?1",
            params![target],
        )
        .expect("見つかった論文を入れる");
        conn.execute(
            "INSERT INTO papers (id, title) VALUES (37, '頭皮脳波による発作部位予測')",
            [],
        )
        .expect("論文を足す");

        let detail = node_detail(conn, &roots, &defs(), target).expect("node");
        // 節点。
        assert_eq!(detail.node.id, fmt_ref('N', target));
        assert_eq!(detail.node.kind, "item");
        assert_eq!(detail.node.blank.as_deref(), Some("insufficient"));
        assert_eq!(detail.node.counts.options, 2);
        assert_eq!(detail.node.counts.questions_open, 1);
        assert_eq!(detail.node.counts.papers, 1);
        // ガイド。
        let guide = detail.guide.as_ref().expect("ガイド");
        assert_eq!(guide.key, "target");
        assert_eq!(guide.name, "対象");
        assert!(!guide.condition_met);
        assert_eq!(guide.extract[0].role, "exemplar");
        assert_eq!(guide.pitfalls.len(), 1);
        // 候補と知見。
        assert_eq!(detail.options.len(), 2);
        assert_eq!(detail.options[0].id, fmt_ref('N', first));
        assert_eq!(detail.options[0].support, 1);
        assert_eq!(detail.findings.len(), 1);
        assert_eq!(detail.findings[0].level, Some(2));
        assert_eq!(detail.findings[0].id, fmt_ref('F', 1));
        // 結び付き（論文の題を取る）。
        assert_eq!(detail.links[0].target, "paper:37");
        assert_eq!(detail.links[0].title, "頭皮脳波による発作部位予測");
        assert_eq!(detail.links[0].read_state.as_deref(), Some("reflected"));
        // 問い。
        assert_eq!(detail.questions[0].id, fmt_ref('Q', question));
        assert!(detail.questions[0].from_guide);
        // 見つけた候補の流れ。
        assert_eq!(detail.funnel.searched, 2);
        assert_eq!(detail.funnel.screened, 2);
        assert_eq!(detail.funnel.read, 1);
        assert_eq!(detail.funnel.reflected, 1);
        // 使うブロック（同じブロックは 1 回）。
        assert_eq!(detail.blocks.len(), 1);
        assert_eq!(detail.blocks[0].block.as_deref(), Some("sci-search"));
        assert_eq!(detail.blocks[0].why, "INSUFFICIENT");
        // 検索の記録。
        assert_eq!(detail.searches.len(), 1);
        // 実行は結び付いた節点だけ。
        write_run(&root, "review", 30, target);
        let detail = node_detail(conn, &roots, &defs(), target).expect("node");
        assert_eq!(detail.actions.len(), 1);
        assert_eq!(detail.actions[0].run_id, 30);
        assert_eq!(detail.actions[0].owner, "review");
        assert_eq!(detail.actions[0].block, "sci-search");
        assert_eq!(detail.actions[0].runner, "pi");
        assert_eq!(detail.actions[0].purpose, "候補を探す");
        assert_eq!(detail.actions[0].created, "2026/09/27 10:00:00");

        // 節点が無ければ NotFound。
        let message = match node_detail(conn, &roots, &defs(), 9999).unwrap_err() {
            SciError::NotFound(message) => message,
            other => panic!("NotFound ではない: {other}"),
        };
        assert!(message.contains("NODE N9999 がありません"), "{message}");

        // 候補にはガイドが無い。
        let second_detail = node_detail(conn, &roots, &defs(), second).expect("node");
        assert_eq!(second_detail.node.id, fmt_ref('N', second));
        assert_eq!(second_detail.node.parents, vec![fmt_ref('N', target)]);
        assert_eq!(second_detail.node.blank, None);
        assert!(second_detail.guide.is_none());

        let _ = fs::remove_dir_all(&root);
    }

    // ---- そのほか ----------------------------------------------------------

    #[test]
    fn status_of_embedded_skeleton() {
        let database = Database::in_memory().expect("メモリ DB");
        let conn = database.connection();
        let root = temp_root("embedded");
        let roots = roots_of(&root);
        // 埋め込みの型とガイドだけを読む（この作業用コピーの `sai-sci/` を使う）。
        let embedded = defs::load(&root.join("not-found"));
        assert!(embedded.warnings.is_empty(), "{:?}", embedded.warnings);
        defs::init_tree(conn, &embedded, "problem").expect("骨組みを作る");

        let result = status(conn, &roots, &embedded).expect("status");
        assert!(result.initialized);
        // 課題駆動の骨組みは一番上 8 個と子 16 個。
        assert_eq!(result.nodes.len(), 24);
        // 子の項目は候補に数えない（骨組みだけなら候補は 0）。
        assert!(result.nodes.iter().all(|row| row.counts.options == 0));
        let top: Vec<&str> = result
            .nodes
            .iter()
            .filter(|row| row.depth == 0)
            .map(|row| row.key.as_deref().unwrap_or_default())
            .collect();
        assert_eq!(
            top,
            vec![
                "background",
                "target",
                "data",
                "purpose",
                "existing",
                "issue",
                "method",
                "novelty"
            ]
        );
        // 何も手を入れてないので全項目が未着手、次の一手も「未着手」の行。
        assert!(result.nodes.iter().all(|row| row.blank.as_deref() == Some("untouched")));
        assert!(result.next.iter().all(|action| action.why == "UNTOUCHED"));
        assert!(result.next.len() <= 10);
        // 関門の 1 つめ（出発点）は通らない。
        let gate = result.gate.as_ref().expect("関門");
        assert_eq!(gate.current, 1);
        assert_eq!(gate.name, "出発点");
        // 種の文は空欄のまま「［？…］」で並ぶ。
        let seed = result.seed.expect("種の文");
        assert!(seed.text.starts_with("「［？対象］の"));
        assert!(seed.parts.iter().any(|part| !part.filled));
        // 節点ごとの行も出る。
        let purpose = key_id(conn, "purpose.subgoal");
        let detail = node_detail(conn, &roots, &embedded, purpose).expect("node");
        assert_eq!(detail.node.blank.as_deref(), Some("untouched"));
        assert!(detail.guide.is_some());
        assert!(!detail.guide.as_ref().expect("ガイド").condition_met);
        assert!(!detail.questions.is_empty());
        assert!(!detail.blocks.is_empty());
        assert_eq!(detail.blocks[0].block.as_deref(), Some("sci-ideate"));

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn mind_headings_old_and_new_are_same() {
        for heading in ["価値の軸", "VALUE_AXES"] {
            let root = temp_root("mind-alias");
            write(root.join(".sai/paper/mind.md"), &format!("## {heading}\n\n1. 臨床\n"));
            let sections = mind_sections(&root.join(".sai/paper"));
            assert_eq!(sections, vec![("VALUE_AXES".to_string(), "1. 臨床".to_string())]);
            for h in ["価値の軸", "VALUE_AXES"] {
                let found = sections
                    .iter()
                    .any(|(n, _)| n == defs::canonical_mind_heading(h));
                assert!(found);
            }
        }
    }

    #[test]
    fn mind_sections_strip_comments() {
        let root = temp_root("mind");
        write(
            root.join(".sai/paper/mind.md"),
            "# 考え\n\n前書き。\n\n## 価値の軸\n\n1. 臨床で使える\n<!-- これは\n   何行もある。 -->\n2. 少ないデータで学べる\n\n## 別の見方\n\n本文。\n\n## まだ書いてない節\n\n<!-- 中身は消える。 -->\n",
        );
        let sections = mind_sections(&root.join(".sai/paper"));
        assert_eq!(sections.len(), 2);
        assert_eq!(sections[0].0, "VALUE_AXES");
        assert_eq!(sections[0].1, "1. 臨床で使える\n2. 少ないデータで学べる");
        assert_eq!(sections[1].0, "別の見方");
        // 中身が空の節は出さない。
        assert_eq!(sections.len(), 2);

        // ファイルが無ければ空。
        let empty = temp_root("mind-empty");
        assert!(mind_sections(&empty.join(".sai/paper")).is_empty());

        // 価値の軸は候補の表の見出しに出る。
        let database = Database::in_memory().expect("メモリ DB");
        let conn = database.connection();
        let roots = roots_of(&root);
        build_tree(conn);
        let target = key_id(conn, "target");
        let detail = node_detail(conn, &roots, &defs(), target).expect("node");
        assert_eq!(detail.axes, vec!["臨床で使える", "少ないデータで学べる"]);

        let _ = fs::remove_dir_all(&root);
        let _ = fs::remove_dir_all(&empty);
    }

    #[test]
    fn status_not_initialized_is_empty() {
        let database = Database::in_memory().expect("メモリ DB");
        let conn = database.connection();
        let root = temp_root("empty");
        let roots = roots_of(&root);

        let result = status(conn, &roots, &defs()).expect("status");
        assert!(!result.initialized);
        assert_eq!(result.type_info, None);
        assert_eq!(result.seed, None);
        assert_eq!(result.gate, None);
        assert!(result.nodes.is_empty());
        assert!(result.edges.is_empty());
        assert!(result.next.is_empty());
        assert!(result.running.is_empty());
        assert!(result.seeds.is_empty());
        assert_eq!(
            result.debts,
            Debts {
                pending_proposals: 0,
                unreflected_papers: 0,
                unscreened_sets: 0,
                empty_reviews: 0
            }
        );

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn status_nodes_are_ordered_depth_first() {
        // C-08: 一番上の項目は型の「項目の順序」の順、その下は深さ優先（`ord`・id の順）。
        let database = Database::in_memory().expect("メモリ DB");
        let conn = database.connection();
        let root = temp_root("depth-first");
        let roots = roots_of(&root);
        build_tree(conn);

        // `data` と `purpose` にだけ子が居る木にする。
        let result = status(conn, &roots, &defs()).expect("status");
        let keys: Vec<&str> = result
            .nodes
            .iter()
            .map(|row| row.key.as_deref().unwrap_or_default())
            .collect();
        assert_eq!(
            keys,
            vec![
                "background.goal",
                "target",
                "data",
                "data.type",
                "purpose",
                "purpose.subgoal",
                "existing",
                "issue.limit",
            ],
            "深さ優先になっていない"
        );
        // 深さも 1 段ごとに 1 増える。
        let depths: Vec<i64> = result.nodes.iter().map(|row| row.depth).collect();
        assert_eq!(depths, vec![0, 0, 0, 1, 0, 1, 0, 0]);

        // 候補は子の項目より先に並ぶ（後から足しても、一覧でどの項目の候補か分かるように）。
        option(conn, key_id(conn, "data"), "公開の脳波", 0);
        let result = status(conn, &roots, &defs()).expect("status");
        let titles: Vec<&str> = result.nodes.iter().map(|row| row.title.as_str()).collect();
        let at = |title: &str| titles.iter().position(|t| *t == title).expect("並びにある");
        let data_title = result.nodes.iter().find(|row| row.key.as_deref() == Some("data")).unwrap().title.clone();
        let type_title = result.nodes.iter().find(|row| row.key.as_deref() == Some("data.type")).unwrap().title.clone();
        assert_eq!(at("公開の脳波"), at(&data_title) + 1, "候補が項目の直後にない");
        assert!(at("公開の脳波") < at(&type_title), "候補が子の項目より後ろ");

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn status_json_keys_match_contract() {
        let database = Database::in_memory().expect("メモリ DB");
        let conn = database.connection();
        let root = temp_root("contract");
        let roots = roots_of(&root);
        build_tree(conn);
        let target = key_id(conn, "target");
        let first = option(conn, target, "てんかん患者", 0);
        finding(conn, first, "support", None);

        let value = serde_json::to_value(status(conn, &roots, &defs()).expect("status"))
            .expect("JSON にする");
        let object = value.as_object().expect("オブジェクト");
        let mut keys: Vec<&str> = object.keys().map(String::as_str).collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            vec![
                "debts",
                "edges",
                "gate",
                "initialized",
                "next",
                "nodes",
                "running",
                "seed",
                "seeds",
                "type",
                "warnings",
            ]
        );

        // nodes[0] の項目名。
        let node = value["nodes"][0].as_object().expect("節点");
        let mut keys: Vec<&str> = node.keys().map(String::as_str).collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            vec![
                "blank",
                "counts",
                "depth",
                "fixed",
                "id",
                "key",
                "kind",
                "marks",
                "ord",
                "parents",
                "state",
                "title",
                "types",
                "value",
            ]
        );
        assert_eq!(node["id"], "N1");
        assert_eq!(node["key"], "background.goal");
        assert_eq!(node["blank"], "untouched");
        assert_eq!(node["counts"]["options"], 0);
        // 節点の並びは型の「項目の順序」のとおり。
        let ids: Vec<&str> = value["nodes"]
            .as_array()
            .expect("節点の配列")
            .iter()
            .filter_map(|row| row["id"].as_str())
            .collect();
        assert_eq!(ids[0], "N1");
        assert_eq!(ids[1], "N2");

        // gate と debts と next の項目名。
        let gate = value["gate"].as_object().expect("関門");
        let mut keys: Vec<&str> = gate.keys().map(String::as_str).collect();
        keys.sort_unstable();
        assert_eq!(keys, vec!["current", "list", "name", "total"]);
        let debts = value["debts"].as_object().expect("負債");
        let mut keys: Vec<&str> = debts.keys().map(String::as_str).collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            vec![
                "empty_reviews",
                "pending_proposals",
                "unreflected_papers",
                "unscreened_sets"
            ]
        );
        let next = value["next"][0].as_object().expect("次の一手");
        let mut keys: Vec<&str> = next.keys().map(String::as_str).collect();
        keys.sort_unstable();
        assert_eq!(keys, vec!["block", "expect", "kind", "node", "purpose", "why"]);

        // node_detail の項目名。
        let detail = serde_json::to_value(node_detail(conn, &roots, &defs(), target).expect("node"))
            .expect("JSON にする");
        let object = detail.as_object().expect("オブジェクト");
        let mut keys: Vec<&str> = object.keys().map(String::as_str).collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            vec![
                "actions",
                "axes",
                "blocks",
                "decisions",
                "findings",
                "funnel",
                "guide",
                "links",
                "node",
                "options",
                "pending",
                "questions",
                "searches",
            ]
        );

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn status_is_fast_enough() {
        let database = Database::in_memory().expect("メモリ DB");
        let conn = database.connection();
        let root = temp_root("fast");
        let roots = roots_of(&root);
        store::meta_set(conn, "type", "problem").expect("型を書く");

        // 600 個の節点（200 の項目と 2 つずつの候補）と 1000 個の知見。
        let tx = conn.unchecked_transaction().expect("トランザクション");
        let mut ids: Vec<i64> = Vec::new();
        for index in 0..200i64 {
            let parent = store::insert_node(
                &tx,
                &NewNode {
                    kind: "item",
                    item_key: None,
                    types: &["write".to_string()],
                    title: &format!("項目 {index}"),
                    body: "",
                    state: "open",
                    fixed: false,
                    ord: index,
                    created_by: "user",
                },
            )
            .expect("項目を作る");
            ids.push(parent);
            for ord in 0..2i64 {
                let option = store::insert_node(
                    &tx,
                    &NewNode {
                        kind: "option",
                        item_key: None,
                        types: &[],
                        title: &format!("候補 {index}-{ord}"),
                        body: "",
                        state: "candidate",
                        fixed: false,
                        ord,
                        created_by: "user",
                    },
                )
                .expect("候補を作る");
                store::insert_edge(&tx, "parent", parent, option).expect("親の線");
                ids.push(option);
            }
        }
        for index in 0..1000i64 {
            let target = ids[usize::try_from(index).unwrap_or(0) % ids.len()];
            store::insert_finding(
                &tx,
                &NewFinding {
                    node_id: target,
                    question_id: None,
                    text: "知見の文",
                    cites: &[],
                    polarity: if index % 2 == 0 { "support" } else { "neutral" },
                    level: Some(1),
                    source: "user",
                    report_id: None,
                    paper_id: None,
                    run_id: None,
                    proposal_id: None,
                    auto: false,
                    created_by: "user",
                },
            )
            .expect("知見を足す");
        }
        tx.commit().expect("書き込む");

        let started = std::time::Instant::now();
        let result = status(conn, &roots, &defs()).expect("status");
        let elapsed = started.elapsed();
        assert_eq!(result.nodes.len(), 600);
        assert!(
            elapsed.as_secs_f64() < 3.0,
            "status が {:.3} 秒で返った",
            elapsed.as_secs_f64()
        );

        let _ = fs::remove_dir_all(&root);
    }
}

