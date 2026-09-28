//! 研究方針の木（Research-tree）の表の読み書き。行の型と基本の操作だけを持つ。
//!
//! ここには「毎回計算する」もの（空欄・印・負債・関門）は置かない。保存する値だけを書く。
//! 空欄や印は [`super::compute`] が作る。
//!
//! JSON の文字列の列（`cites` `evidence` `rejected` `path` `hit_papers` `payload`）は
//! Rust では `Vec` にして持つ。読めない JSON は空として扱う。

use std::collections::{BTreeSet, HashSet};

use rusqlite::{Connection, OptionalExtension, Row, params};
use serde::{Deserialize, Serialize};

use super::{Result, SciError, now_text};

// ---- 検査に使う定数（C-01） ----------------------------------------------------

/// 項目の型。DB の `item_types` にはこの順でカンマ区切りで書く。
const TYPE_ORDER: [&str; 4] = ["choose", "list", "write", "verify"];
/// 項目が持てる状態。
const ITEM_STATES: [&str; 4] = ["open", "tentative", "decided", "excluded"];
/// 候補が持てる状態。
const OPTION_STATES: [&str; 4] = ["candidate", "selected", "rejected", "dormant"];

const NODE_COLUMNS: &str = "id, kind, item_key, item_types, title, body, state, fixed, ord, \
     created_by, created_at, updated_at, state_changed_at";
const EDGE_COLUMNS: &str = "id, kind, from_node, to_node, note";
const QUESTION_COLUMNS: &str =
    "id, node_id, text, state, answer, from_guide, created_by, created_at, updated_at";
const FINDING_COLUMNS: &str = "id, node_id, question_id, text, cites, polarity, level, source, \
     report_id, paper_id, run_id, proposal_id, auto, removed_at, created_by, created_at";
const LINK_COLUMNS: &str = "id, node_id, target_kind, target_ref, role, read_state, reason, \
     proposal_id, auto, removed_at, created_by, created_at, updated_at";
const DECISION_COLUMNS: &str = "id, node_id, action, option_id, reason, evidence, rejected, \
     counter_searched, actor, proposal_id, created_at";
const PROPOSAL_COLUMNS: &str = "id, run_id, block, op, node_id, payload, reason, risk, status, \
     result_ref, decided_by, decided_at, decide_note, created_at";
const TOMBSTONE_COLUMNS: &str = "id, kind, paper_id, doi, arxiv_id, dedupe_key, title, reason, \
     node_id, run_id, created_by, created_at";
const SEED_COLUMNS: &str = "id, title, sentence, path, doc_ref, run_id, created_at";
const SEARCH_COLUMNS: &str = "id, run_id, node_id, question_id, tool, query, scope, widen_reason, \
     hits, hit_papers, set_id, created_at";

// ---- JSON の文字列の列を<Vec> にする手 ---------------------------------------

/// 文字列の JSON 配列を読む。読めないときは空。
fn parse_vec<T: for<'de> Deserialize<'de>>(text: &str) -> Vec<T> {
    serde_json::from_str(text).unwrap_or_default()
}

/// 文字列の JSON 配列を書く。
fn write_vec<T: Serialize>(items: &[T]) -> Result<String> {
    Ok(serde_json::to_string(items)?)
}

/// どの JSON 列でも無いときは `Value::Null` として扱う。
fn parse_value(text: &str) -> serde_json::Value {
    serde_json::from_str(text).unwrap_or(serde_json::Value::Null)
}

/// 項目の型の並びを C-01 の順にそろえてカンマ区切りにする。
fn types_to_text(types: &[String]) -> String {
    let mut sorted: Vec<&String> = types.iter().collect();
    sorted.sort_by_key(|value| {
        TYPE_ORDER
            .iter()
            .position(|order| *order == value.as_str())
            .unwrap_or(TYPE_ORDER.len())
    });
    sorted
        .iter()
        .map(|value| value.as_str())
        .collect::<Vec<_>>()
        .join(",")
}

/// `item_types` の文字列を `Vec` にする。空文字なら空の Vec。
fn types_from_text(text: &str) -> Vec<String> {
    text.split(',')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(str::to_string)
        .collect()
}

// ---- 行の型 -----------------------------------------------------------------

/// 節点（項目と候補）。消さず、対象外・却下・休眠の状態にする。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Node {
    /// 番号（外に出すときは `N12`）。
    pub id: i64,
    /// `item` / `option`。
    pub kind: String,
    /// 項目ガイドのキー。手で足した項目は `None`。
    pub item_key: Option<String>,
    /// 項目の型（C-01 の順で並べる）。
    pub types: Vec<String>,
    /// 題名。
    pub title: String,
    /// 本文。
    pub body: String,
    /// 項目の状態か、候補の状態。
    pub state: String,
    /// 「動かさない項目」の印。
    pub fixed: bool,
    /// 兄弟の中での並び。
    pub ord: i64,
    /// 作った者（`user` / `run:<run_id>`）。
    pub created_by: String,
    /// 作った時刻（UTC）。
    pub created_at: String,
    /// 最後の更新の時刻（UTC）。
    pub updated_at: String,
    /// 最後の状態変更の時刻（UTC）。
    pub state_changed_at: String,
}

/// 節点を足すときの値。`item_key` は項目ガイドから作るときだけ入れる。
#[derive(Debug, Clone, PartialEq)]
pub struct NewNode<'a> {
    /// `item` / `option`。
    pub kind: &'a str,
    /// 項目ガイドのキー。
    pub item_key: Option<&'a str>,
    /// 項目の型。書き込むときに C-01 の順に並べ直す。
    pub types: &'a [String],
    /// 題名。空にはできない。
    pub title: &'a str,
    /// 本文。
    pub body: &'a str,
    /// 項目の状態か、候補の状態。kind に合わない状態は受け付けない。
    pub state: &'a str,
    /// 「動かさない項目」の印。
    pub fixed: bool,
    /// 兄弟の中での並び。
    pub ord: i64,
    /// 作った者。
    pub created_by: &'a str,
}

/// 線（`parent` 親子・`depends` 依存・`derived` 由来・`analogous` 同型）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Edge {
    /// 番号（外に出すときは `L3`）。
    pub id: i64,
    /// 線の種類。
    pub kind: String,
    /// 元の節点。
    pub from: i64,
    /// 先の節点。
    pub to: i64,
    /// 補足。
    pub note: String,
}

/// 問い。答えか引下げで終わり、答えは `answer` に残る。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Question {
    /// 番号（外に出すときは `Q7`）。
    pub id: i64,
    /// 問いが付く節点。
    pub node_id: i64,
    /// 問いの文。
    pub text: String,
    /// `open` / `answered` / `withdrawn`。
    pub state: String,
    /// 答え。
    pub answer: String,
    /// 項目ガイドが置いた問いか。
    pub from_guide: bool,
    /// 作った者。
    pub created_by: String,
    /// 作った時刻（UTC）。
    pub created_at: String,
    /// 最後の更新の時刻（UTC）。
    pub updated_at: String,
}

/// 知見。外すときは `removed_at` が入る（行は消さない）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Finding {
    /// 番号（外に出すときは `F31`）。
    pub id: i64,
    /// 知見が付く節点。
    pub node_id: i64,
    /// 答えられた問い。
    pub question_id: Option<i64>,
    /// 知見の文。
    pub text: String,
    /// 引用の文字列（例 `["#37 §3.2"]`）。
    pub cites: Vec<String>,
    /// `support` / `refute` / `neutral` / `none`。
    pub polarity: String,
    /// 強さ（1〜4）。
    pub level: Option<i64>,
    /// `report` / `note` / `user` / `run`。
    pub source: String,
    /// 出所の報告の番号。
    pub report_id: Option<String>,
    /// 出所の論文の番号。
    pub paper_id: Option<i64>,
    /// 出した実行の番号。
    pub run_id: Option<i64>,
    /// 出した提案の番号。
    pub proposal_id: Option<i64>,
    /// 自動で入ったものか。
    pub auto: bool,
    /// 外した時刻（UTC）。`None` はまだ外していない。
    pub removed_at: Option<String>,
    /// 入れた者。
    pub created_by: String,
    /// 入れた時刻（UTC）。
    pub created_at: String,
}

/// 知見を足すときの値。
#[derive(Debug, Clone, PartialEq)]
pub struct NewFinding<'a> {
    /// 知見が付く節点。
    pub node_id: i64,
    /// 答えられた問い。
    pub question_id: Option<i64>,
    /// 知見の文。空にはできない。
    pub text: &'a str,
    /// 引用の文字列。
    pub cites: &'a [String],
    /// 向き（`support` など）。
    pub polarity: &'a str,
    /// 強さ（1〜4）。
    pub level: Option<i64>,
    /// 出所（`report` など）。
    pub source: &'a str,
    /// 出所の報告の番号。
    pub report_id: Option<&'a str>,
    /// 出所の論文の番号。
    pub paper_id: Option<i64>,
    /// 出した実行の番号。
    pub run_id: Option<i64>,
    /// 出した提案の番号。
    pub proposal_id: Option<i64>,
    /// 自動で入れるか。
    pub auto: bool,
    /// 入れた者。
    pub created_by: &'a str,
}

/// 結び付き（論文・報告・ノート・検索セット・実行）。外すときは `removed_at` が入る。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Link {
    /// 番号（外に出すときは `L3`）。
    pub id: i64,
    /// 結び付く節点。
    pub node_id: i64,
    /// `paper` / `report` / `note` / `set` / `run`。
    pub target_kind: String,
    /// 相手の番号。
    pub target_ref: String,
    /// 相手が `paper` のときだけ入る（`exemplar` など）。
    pub role: Option<String>,
    /// 相手が `paper` のときだけ入る（`unread` など）。
    pub read_state: Option<String>,
    /// なぜ結んだか・外したか。
    pub reason: String,
    /// 出した提案の番号。
    pub proposal_id: Option<i64>,
    /// 自動で結んだか。
    pub auto: bool,
    /// 外した時刻（UTC）。`None` はまだ外していない。
    pub removed_at: Option<String>,
    /// 結んだ者。
    pub created_by: String,
    /// 結んだ時刻（UTC）。
    pub created_at: String,
    /// 最後の更新の時刻（UTC）。
    pub updated_at: String,
}

/// 結び付きを足すときの値。
#[derive(Debug, Clone, PartialEq)]
pub struct NewLink<'a> {
    /// 結び付く節点。
    pub node_id: i64,
    /// 相手の種類。
    pub target_kind: &'a str,
    /// 相手の番号。
    pub target_ref: &'a str,
    /// 相手が `paper` のときだけ入れる。
    pub role: Option<&'a str>,
    /// 相手が `paper` のときだけ入れる。
    pub read_state: Option<&'a str>,
    /// なぜ結んだか。
    pub reason: &'a str,
    /// 出した提案の番号。
    pub proposal_id: Option<i64>,
    /// 自動で結ぶか。
    pub auto: bool,
    /// 結んだ者。
    pub created_by: &'a str,
}

/// 決定の記録。追記だけ（直す・消すのは DB のトリガーが止める）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Decision {
    /// 番号。
    pub id: i64,
    /// 決めた節点。
    pub node_id: i64,
    /// `decide` / `tentative` / `reopen` / `exclude_item` / `reject_option` / `dormant_option` / `restore_option`。
    pub action: String,
    /// 対象にした候補。
    pub option_id: Option<i64>,
    /// 決めた理由。
    pub reason: String,
    /// 根拠にした知見の番号。
    pub evidence: Vec<i64>,
    /// 却下した候補と理由。
    pub rejected: Vec<RejectedOption>,
    /// 反証を探したか。
    pub counter_searched: bool,
    /// 実行者（`user` / `run:<run_id>`）。
    pub actor: String,
    /// 出した提案の番号。
    pub proposal_id: Option<i64>,
    /// 記録した時刻（UTC）。
    pub created_at: String,
}

/// 却下した候補と、その理由。DB では `{"option":14,"reason":"…"}` の配列。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RejectedOption {
    /// 却下した候補の番号。
    pub option: i64,
    /// 却下した理由。
    pub reason: String,
}

/// 決定の記録を足すときの値。
#[derive(Debug, Clone, PartialEq)]
pub struct NewDecision<'a> {
    /// 決めた節点。
    pub node_id: i64,
    /// 決めた種類。
    pub action: &'a str,
    /// 対象にした候補。
    pub option_id: Option<i64>,
    /// 決めた理由。
    pub reason: &'a str,
    /// 根拠にした知見の番号。
    pub evidence: &'a [i64],
    /// 却下した候補と理由。
    pub rejected: &'a [RejectedOption],
    /// 反証を探したか。
    pub counter_searched: bool,
    /// 実行者。
    pub actor: &'a str,
    /// 出した提案の番号。
    pub proposal_id: Option<i64>,
}

/// 提案。自動で反映したもの・人の判断を待つものどちらもここで持つ。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Proposal {
    /// 番号（外に出すときは `P55`）。
    pub id: i64,
    /// 出した実行の番号。
    pub run_id: Option<i64>,
    /// 出したブロック。
    pub block: String,
    /// する操作（`add_finding` など）。
    pub op: String,
    /// 対象の節点。
    pub node_id: Option<i64>,
    /// 操作の中身。
    pub payload: serde_json::Value,
    /// なぜそうするか。
    pub reason: String,
    /// `auto` / `human`。
    pub risk: String,
    /// `applied` / `pending` / `accepted` / `rejected` / `reverted`。
    pub status: String,
    /// 反映した結果の参照（例 `F9`）。
    pub result_ref: Option<String>,
    /// 仕分けた者。
    pub decided_by: Option<String>,
    /// 仕分けた時刻（UTC）。
    pub decided_at: Option<String>,
    /// 仕分けのメモ。
    pub decide_note: String,
    /// 出した時刻（UTC）。
    pub created_at: String,
}

/// 提案を足すときの値。
#[derive(Debug, Clone, PartialEq)]
pub struct NewProposal<'a> {
    /// 出した実行の番号。
    pub run_id: Option<i64>,
    /// 出したブロック。
    pub block: &'a str,
    /// する操作。
    pub op: &'a str,
    /// 対象の節点。
    pub node_id: Option<i64>,
    /// 操作の中身。
    pub payload: &'a serde_json::Value,
    /// なぜそうするか。
    pub reason: &'a str,
    /// `auto` / `human`。
    pub risk: &'a str,
    /// `applied` / `pending` / `accepted` / `rejected` / `reverted`。
    pub status: &'a str,
}

/// 墓標（外した論文・検索の候補）。再検索・再読を防ぐために残す。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Tombstone {
    /// 番号。
    pub id: i64,
    /// `paper` / `candidate`。
    pub kind: String,
    /// 論文の番号。
    pub paper_id: Option<i64>,
    /// DOI。
    pub doi: Option<String>,
    /// arXiv の番号。
    pub arxiv_id: Option<String>,
    /// 重複を見るためのキー。
    pub dedupe_key: Option<String>,
    /// 題名。
    pub title: String,
    /// 外した理由。空にはできない。
    pub reason: String,
    /// 外した節点。
    pub node_id: Option<i64>,
    /// 外した実行の番号。
    pub run_id: Option<i64>,
    /// 外した者。
    pub created_by: String,
    /// 外した時刻（UTC）。
    pub created_at: String,
}

/// 墓標を足すときの値。`paper_id`・`doi`・`arxiv_id`・`dedupe_key` のどれかが要る。
#[derive(Debug, Clone, PartialEq)]
pub struct NewTombstone<'a> {
    /// `paper` / `candidate`。
    pub kind: &'a str,
    /// 論文の番号。
    pub paper_id: Option<i64>,
    /// DOI。
    pub doi: Option<&'a str>,
    /// arXiv の番号。
    pub arxiv_id: Option<&'a str>,
    /// 重複を見るためのキー。
    pub dedupe_key: Option<&'a str>,
    /// 題名。
    pub title: &'a str,
    /// 外した理由。
    pub reason: &'a str,
    /// 外した節点。
    pub node_id: Option<i64>,
    /// 外した実行の番号。
    pub run_id: Option<i64>,
    /// 外した者。
    pub created_by: &'a str,
}

/// 種（研究の種。次に手を入れる場面 Saying）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Seed {
    /// 番号（外に出すときは `S1`）。
    pub id: i64,
    /// 題名。
    pub title: String,
    /// 一文で書いた Saying。
    pub sentence: String,
    /// 根からの節点の番号。
    pub path: Vec<i64>,
    /// 書いた場所（ノートのパスなど）。
    pub doc_ref: String,
    /// 出した実行の番号。
    pub run_id: Option<i64>,
    /// 出した時刻（UTC）。
    pub created_at: String,
}

/// 問いに結び付けた検索の記録（`find` / `search` / `expand`）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SearchRecord {
    /// 番号。
    pub id: i64,
    /// 検索した実行の番号。
    pub run_id: Option<i64>,
    /// 検索した節点。
    pub node_id: Option<i64>,
    /// 検索した問い。
    pub question_id: Option<i64>,
    /// `find` / `search` / `expand`。
    pub tool: String,
    /// 検索語。
    pub query: String,
    /// `node:N7` / `subtree:N7` / `papers:37,40` / `set:5` / `library` / `external:s2,pubmed`。
    pub scope: String,
    /// 範囲を広げた理由。
    pub widen_reason: String,
    /// 当たった件数。
    pub hits: i64,
    /// 当たった論文の番号。
    pub hit_papers: Vec<i64>,
    /// 作った検索セットの番号。
    pub set_id: Option<i64>,
    /// 検索した時刻（UTC）。
    pub created_at: String,
}

/// 検索の記録を足すときの値。
#[derive(Debug, Clone, PartialEq)]
pub struct NewSearch<'a> {
    /// 検索した実行の番号。
    pub run_id: Option<i64>,
    /// 検索した節点。
    pub node_id: Option<i64>,
    /// 検索した問い。
    pub question_id: Option<i64>,
    /// `find` / `search` / `expand`。
    pub tool: &'a str,
    /// 検索語。
    pub query: &'a str,
    /// 検索の範囲。
    pub scope: &'a str,
    /// 範囲を広げた理由。
    pub widen_reason: &'a str,
    /// 当たった件数。
    pub hits: i64,
    /// 当たった論文の番号。
    pub hit_papers: &'a [i64],
    /// 作った検索セットの番号。
    pub set_id: Option<i64>,
}

// ---- 行を型にする手 ---------------------------------------------------------

fn node_of(row: &Row<'_>) -> rusqlite::Result<Node> {
    let item_types: String = row.get(3)?;
    Ok(Node {
        id: row.get(0)?,
        kind: row.get(1)?,
        item_key: row.get(2)?,
        types: types_from_text(&item_types),
        title: row.get(4)?,
        body: row.get(5)?,
        state: row.get(6)?,
        fixed: row.get::<_, i64>(7)? != 0,
        ord: row.get(8)?,
        created_by: row.get(9)?,
        created_at: row.get(10)?,
        updated_at: row.get(11)?,
        state_changed_at: row.get(12)?,
    })
}

fn edge_of(row: &Row<'_>) -> rusqlite::Result<Edge> {
    Ok(Edge {
        id: row.get(0)?,
        kind: row.get(1)?,
        from: row.get(2)?,
        to: row.get(3)?,
        note: row.get(4)?,
    })
}

fn question_of(row: &Row<'_>) -> rusqlite::Result<Question> {
    Ok(Question {
        id: row.get(0)?,
        node_id: row.get(1)?,
        text: row.get(2)?,
        state: row.get(3)?,
        answer: row.get(4)?,
        from_guide: row.get::<_, i64>(5)? != 0,
        created_by: row.get(6)?,
        created_at: row.get(7)?,
        updated_at: row.get(8)?,
    })
}

fn finding_of(row: &Row<'_>) -> rusqlite::Result<Finding> {
    let cites: String = row.get(4)?;
    Ok(Finding {
        id: row.get(0)?,
        node_id: row.get(1)?,
        question_id: row.get(2)?,
        text: row.get(3)?,
        cites: parse_vec(&cites),
        polarity: row.get(5)?,
        level: row.get(6)?,
        source: row.get(7)?,
        report_id: row.get(8)?,
        paper_id: row.get(9)?,
        run_id: row.get(10)?,
        proposal_id: row.get(11)?,
        auto: row.get::<_, i64>(12)? != 0,
        removed_at: row.get(13)?,
        created_by: row.get(14)?,
        created_at: row.get(15)?,
    })
}

fn link_of(row: &Row<'_>) -> rusqlite::Result<Link> {
    Ok(Link {
        id: row.get(0)?,
        node_id: row.get(1)?,
        target_kind: row.get(2)?,
        target_ref: row.get(3)?,
        role: row.get(4)?,
        read_state: row.get(5)?,
        reason: row.get(6)?,
        proposal_id: row.get(7)?,
        auto: row.get::<_, i64>(8)? != 0,
        removed_at: row.get(9)?,
        created_by: row.get(10)?,
        created_at: row.get(11)?,
        updated_at: row.get(12)?,
    })
}

fn decision_of(row: &Row<'_>) -> rusqlite::Result<Decision> {
    let evidence: String = row.get(5)?;
    let rejected: String = row.get(6)?;
    Ok(Decision {
        id: row.get(0)?,
        node_id: row.get(1)?,
        action: row.get(2)?,
        option_id: row.get(3)?,
        reason: row.get(4)?,
        evidence: parse_vec(&evidence),
        rejected: parse_vec(&rejected),
        counter_searched: row.get::<_, i64>(7)? != 0,
        actor: row.get(8)?,
        proposal_id: row.get(9)?,
        created_at: row.get(10)?,
    })
}

fn proposal_of(row: &Row<'_>) -> rusqlite::Result<Proposal> {
    let payload: String = row.get(5)?;
    Ok(Proposal {
        id: row.get(0)?,
        run_id: row.get(1)?,
        block: row.get(2)?,
        op: row.get(3)?,
        node_id: row.get(4)?,
        payload: parse_value(&payload),
        reason: row.get(6)?,
        risk: row.get(7)?,
        status: row.get(8)?,
        result_ref: row.get(9)?,
        decided_by: row.get(10)?,
        decided_at: row.get(11)?,
        decide_note: row.get(12)?,
        created_at: row.get(13)?,
    })
}

fn tombstone_of(row: &Row<'_>) -> rusqlite::Result<Tombstone> {
    Ok(Tombstone {
        id: row.get(0)?,
        kind: row.get(1)?,
        paper_id: row.get(2)?,
        doi: row.get(3)?,
        arxiv_id: row.get(4)?,
        dedupe_key: row.get(5)?,
        title: row.get(6)?,
        reason: row.get(7)?,
        node_id: row.get(8)?,
        run_id: row.get(9)?,
        created_by: row.get(10)?,
        created_at: row.get(11)?,
    })
}

fn seed_of(row: &Row<'_>) -> rusqlite::Result<Seed> {
    let path: String = row.get(3)?;
    Ok(Seed {
        id: row.get(0)?,
        title: row.get(1)?,
        sentence: row.get(2)?,
        path: parse_vec(&path),
        doc_ref: row.get(4)?,
        run_id: row.get(5)?,
        created_at: row.get(6)?,
    })
}

fn search_of(row: &Row<'_>) -> rusqlite::Result<SearchRecord> {
    let hit_papers: String = row.get(9)?;
    Ok(SearchRecord {
        id: row.get(0)?,
        run_id: row.get(1)?,
        node_id: row.get(2)?,
        question_id: row.get(3)?,
        tool: row.get(4)?,
        query: row.get(5)?,
        scope: row.get(6)?,
        widen_reason: row.get(7)?,
        hits: row.get(8)?,
        hit_papers: parse_vec(&hit_papers),
        set_id: row.get(10)?,
        created_at: row.get(11)?,
    })
}

/// 1 本の SELECT を型付きの行の列に直す。
fn collect<T, P, F>(conn: &Connection, sql: &str, params: P, make: F) -> Result<Vec<T>>
where
    P: rusqlite::Params,
    F: Fn(&Row<'_>) -> rusqlite::Result<T>,
{
    let mut statement = conn.prepare(sql)?;
    let rows = statement.query_map(params, make)?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

// ---- 節点 -------------------------------------------------------------------

/// 節点を足して、できた番号を返す。
pub fn insert_node(conn: &Connection, node: &NewNode<'_>) -> Result<i64> {
    if node.title.trim().is_empty() {
        return Err(SciError::Invalid(format!(
            "節点の題名は空にできません: {}",
            node.title
        )));
    }
    let allowed: &[&str] = if node.kind == "item" {
        &ITEM_STATES
    } else {
        &OPTION_STATES
    };
    if !allowed.contains(&node.state) {
        return Err(SciError::Invalid(format!(
            "この節点にはその状態を入れられません: {}",
            node.state
        )));
    }
    conn.execute(
        "INSERT INTO sci_nodes
           (kind, item_key, item_types, title, body, state, fixed, ord, created_by)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            node.kind,
            node.item_key,
            types_to_text(node.types),
            node.title,
            node.body,
            node.state,
            i64::from(node.fixed),
            node.ord,
            node.created_by,
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

/// 節点を 1 つ読む。無ければ `NotFound`。
pub fn get_node(conn: &Connection, id: i64) -> Result<Node> {
    conn.query_row(
        &format!("SELECT {NODE_COLUMNS} FROM sci_nodes WHERE id = ?1"),
        params![id],
        node_of,
    )
    .optional()?
    .ok_or_else(|| SciError::NotFound(format!("節点 {} がありません", super::fmt_ref('N', id))))
}

/// 節点を全部読む（番号の昇順）。
pub fn list_nodes(conn: &Connection) -> Result<Vec<Node>> {
    collect(
        conn,
        &format!("SELECT {NODE_COLUMNS} FROM sci_nodes ORDER BY id"),
        [],
        node_of,
    )
}

/// 項目ガイドのキーで項目を探す。
pub fn find_item_by_key(conn: &Connection, key: &str) -> Result<Option<Node>> {
    conn.query_row(
        &format!("SELECT {NODE_COLUMNS} FROM sci_nodes WHERE item_key = ?1"),
        params![key],
        node_of,
    )
    .optional()
    .map_err(SciError::from)
}

/// 題名と本文を入れ直す。`None` のほうはそのままにする。
pub fn update_node_text(
    conn: &Connection,
    id: i64,
    title: Option<&str>,
    body: Option<&str>,
) -> Result<()> {
    let node = get_node(conn, id)?;
    let title = title.unwrap_or(node.title.as_str());
    let body = body.unwrap_or(node.body.as_str());
    if title.trim().is_empty() {
        return Err(SciError::Invalid(format!(
            "節点 {} の題名は空にできません",
            super::fmt_ref('N', id)
        )));
    }
    conn.execute(
        "UPDATE sci_nodes SET title = ?1, body = ?2, updated_at = ?3 WHERE id = ?4",
        params![title, body, now_text(), id],
    )?;
    Ok(())
}

/// 状態を入れ直す。kind に合わない状態は `Invalid`。`state_changed_at` と `updated_at` を今にする。
pub fn set_node_state(conn: &Connection, id: i64, state: &str) -> Result<()> {
    let node = get_node(conn, id)?;
    let allowed: &[&str] = if node.kind == "item" {
        &ITEM_STATES
    } else {
        &OPTION_STATES
    };
    if !allowed.contains(&state) {
        return Err(SciError::Invalid(format!(
            "この節点にはその状態を入れられません: {state}"
        )));
    }
    // 状態が同じなら何もしない（時刻も変えない）。
    if node.state == state {
        return Ok(());
    }
    let now = now_text();
    conn.execute(
        "UPDATE sci_nodes SET state = ?1, state_changed_at = ?2, updated_at = ?2 WHERE id = ?3",
        params![state, now, id],
    )?;
    Ok(())
}

/// 「動かさない項目」の印を出し入れする。
pub fn set_node_fixed(conn: &Connection, id: i64, fixed: bool) -> Result<()> {
    get_node(conn, id)?;
    conn.execute(
        "UPDATE sci_nodes SET fixed = ?1, updated_at = ?2 WHERE id = ?3",
        params![i64::from(fixed), now_text(), id],
    )?;
    Ok(())
}

// ---- 線 ---------------------------------------------------------------------

/// 線を足す。同じ (kind, from, to) があればその id を返す（増やさない）。
pub fn insert_edge(conn: &Connection, kind: &str, from: i64, to: i64) -> Result<i64> {
    conn.execute(
        "INSERT OR IGNORE INTO sci_edges (kind, from_node, to_node) VALUES (?1, ?2, ?3)",
        params![kind, from, to],
    )?;
    conn.query_row(
        "SELECT id FROM sci_edges WHERE kind = ?1 AND from_node = ?2 AND to_node = ?3",
        params![kind, from, to],
        |row| row.get(0),
    )
    .map_err(SciError::from)
}

/// 線を全部読む（番号の昇順）。
pub fn list_edges(conn: &Connection) -> Result<Vec<Edge>> {
    collect(
        conn,
        &format!("SELECT {EDGE_COLUMNS} FROM sci_edges ORDER BY id"),
        [],
        edge_of,
    )
}

/// 親（`parent` の線の from）。
pub fn parents_of(conn: &Connection, id: i64) -> Result<Vec<i64>> {
    let mut statement = conn.prepare(
        "SELECT from_node FROM sci_edges WHERE kind = 'parent' AND to_node = ?1 ORDER BY from_node",
    )?;
    let rows = statement.query_map(params![id], |row| row.get::<_, i64>(0))?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

/// 子（`parent` の線の to）。`ord`、番号の順。
pub fn children_of(conn: &Connection, id: i64) -> Result<Vec<i64>> {
    let mut statement = conn.prepare(
        "SELECT e.to_node FROM sci_edges e
           JOIN sci_nodes n ON n.id = e.to_node
          WHERE e.kind = 'parent' AND e.from_node = ?1
          ORDER BY n.ord, e.to_node",
    )?;
    let rows = statement.query_map(params![id], |row| row.get::<_, i64>(0))?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

/// 自分を含めない子孫（親子の線を幅優先でたどる）。輪があっても止まる。
pub fn descendants(conn: &Connection, id: i64) -> Result<Vec<i64>> {
    let mut visited: HashSet<i64> = HashSet::new();
    visited.insert(id);
    let mut out: Vec<i64> = Vec::new();
    let mut queue: Vec<i64> = vec![id];
    while let Some(current) = queue.first().copied() {
        queue.remove(0);
        for child in children_of(conn, current)? {
            if !visited.insert(child) {
                continue;
            }
            out.push(child);
            queue.push(child);
        }
    }
    Ok(out)
}

// ---- 問い -------------------------------------------------------------------

/// 問いを足して、できた番号を返す。
pub fn insert_question(
    conn: &Connection,
    node_id: i64,
    text: &str,
    from_guide: bool,
    created_by: &str,
) -> Result<i64> {
    if text.trim().is_empty() {
        return Err(SciError::Invalid("問いは空にできません".to_string()));
    }
    conn.execute(
        "INSERT INTO sci_questions (node_id, text, from_guide, created_by)
         VALUES (?1, ?2, ?3, ?4)",
        params![node_id, text, i64::from(from_guide), created_by],
    )?;
    Ok(conn.last_insert_rowid())
}

/// 問いを 1 つ読む。無ければ `NotFound`。
pub fn get_question(conn: &Connection, id: i64) -> Result<Question> {
    conn.query_row(
        &format!("SELECT {QUESTION_COLUMNS} FROM sci_questions WHERE id = ?1"),
        params![id],
        question_of,
    )
    .optional()?
    .ok_or_else(|| SciError::NotFound(format!("問い {} がありません", super::fmt_ref('Q', id))))
}

/// 問いを読む。`node_id` が `Some` ならその節点のものだけ（番号の昇順）。
pub fn list_questions(conn: &Connection, node_id: Option<i64>) -> Result<Vec<Question>> {
    match node_id {
        Some(id) => collect(
            conn,
            &format!("SELECT {QUESTION_COLUMNS} FROM sci_questions WHERE node_id = ?1 ORDER BY id"),
            params![id],
            question_of,
        ),
        None => collect(
            conn,
            &format!("SELECT {QUESTION_COLUMNS} FROM sci_questions ORDER BY id"),
            [],
            question_of,
        ),
    }
}

/// 問いに答える（`state` は answered）。
pub fn answer_question(conn: &Connection, id: i64, answer: &str) -> Result<()> {
    get_question(conn, id)?;
    conn.execute(
        "UPDATE sci_questions SET answer = ?1, state = 'answered', updated_at = ?2 WHERE id = ?3",
        params![answer, now_text(), id],
    )?;
    Ok(())
}

/// 問いを引く（`state` は withdrawn）。
pub fn withdraw_question(conn: &Connection, id: i64) -> Result<()> {
    get_question(conn, id)?;
    conn.execute(
        "UPDATE sci_questions SET state = 'withdrawn', updated_at = ?1 WHERE id = ?2",
        params![now_text(), id],
    )?;
    Ok(())
}

// ---- 知見 -------------------------------------------------------------------

/// 知見を足して、できた番号を返す。
pub fn insert_finding(conn: &Connection, finding: &NewFinding<'_>) -> Result<i64> {
    if finding.text.trim().is_empty() {
        return Err(SciError::Invalid("知見は空にできません".to_string()));
    }
    conn.execute(
        "INSERT INTO sci_findings
           (node_id, question_id, text, cites, polarity, level, source,
            report_id, paper_id, run_id, proposal_id, auto, created_by)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
        params![
            finding.node_id,
            finding.question_id,
            finding.text,
            write_vec(finding.cites)?,
            finding.polarity,
            finding.level,
            finding.source,
            finding.report_id,
            finding.paper_id,
            finding.run_id,
            finding.proposal_id,
            i64::from(finding.auto),
            finding.created_by,
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

/// 知見を 1 つ読む。無ければ `NotFound`。
pub fn get_finding(conn: &Connection, id: i64) -> Result<Finding> {
    conn.query_row(
        &format!("SELECT {FINDING_COLUMNS} FROM sci_findings WHERE id = ?1"),
        params![id],
        finding_of,
    )
    .optional()?
    .ok_or_else(|| SciError::NotFound(format!("知見 {} がありません", super::fmt_ref('F', id))))
}

/// 知見を読む。`include_removed` が false なら外したものは除く（番号の昇順）。
pub fn list_findings(
    conn: &Connection,
    node_id: Option<i64>,
    include_removed: bool,
) -> Result<Vec<Finding>> {
    let removed = if include_removed {
        ""
    } else {
        " AND removed_at IS NULL"
    };
    match node_id {
        Some(id) => collect(
            conn,
            &format!(
                "SELECT {FINDING_COLUMNS} FROM sci_findings WHERE node_id = ?1{removed} ORDER BY id"
            ),
            params![id],
            finding_of,
        ),
        None => collect(
            conn,
            &format!("SELECT {FINDING_COLUMNS} FROM sci_findings WHERE 1 = 1{removed} ORDER BY id"),
            [],
            finding_of,
        ),
    }
}

/// 知見を外す（行は消さず `removed_at` を入れる）。
pub fn remove_finding(conn: &Connection, id: i64) -> Result<()> {
    get_finding(conn, id)?;
    conn.execute(
        "UPDATE sci_findings SET removed_at = ?1 WHERE id = ?2",
        params![now_text(), id],
    )?;
    Ok(())
}

// ---- 結び付き ---------------------------------------------------------------

/// 結び付ける。同じ (node, target_kind, target_ref, role) があれば、外したものを戻してその id を返す。
pub fn upsert_link(conn: &Connection, link: &NewLink<'_>) -> Result<i64> {
    if link.target_ref.trim().is_empty() {
        return Err(SciError::Invalid(
            "結び付けの相手を指定してください".to_string(),
        ));
    }
    let role_key = link.role.unwrap_or("");
    let existing: Option<i64> = conn
        .query_row(
            "SELECT id FROM sci_links
              WHERE node_id = ?1 AND target_kind = ?2 AND target_ref = ?3
                AND COALESCE(role, '') = ?4",
            params![link.node_id, link.target_kind, link.target_ref, role_key],
            |row| row.get(0),
        )
        .optional()?;
    let now = now_text();
    if let Some(id) = existing {
        conn.execute(
            "UPDATE sci_links
                SET removed_at = NULL, reason = ?1,
                    read_state = COALESCE(?2, read_state),
                    updated_at = ?3
              WHERE id = ?4",
            params![link.reason, link.read_state, now, id],
        )?;
        return Ok(id);
    }
    conn.execute(
        "INSERT INTO sci_links
           (node_id, target_kind, target_ref, role, read_state, reason, proposal_id, auto, created_by)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            link.node_id,
            link.target_kind,
            link.target_ref,
            link.role,
            link.read_state,
            link.reason,
            link.proposal_id,
            i64::from(link.auto),
            link.created_by,
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

/// 結び付きを 1 つ読む。無ければ `NotFound`。
pub fn get_link(conn: &Connection, id: i64) -> Result<Link> {
    conn.query_row(
        &format!("SELECT {LINK_COLUMNS} FROM sci_links WHERE id = ?1"),
        params![id],
        link_of,
    )
    .optional()?
    .ok_or_else(|| SciError::NotFound(format!("結び付き {} がありません", super::fmt_ref('L', id))))
}

/// 結び付きを読む。`node_id` が `Some` ならその節点のものだけ（番号の昇順）。
pub fn list_links(
    conn: &Connection,
    node_id: Option<i64>,
    include_removed: bool,
) -> Result<Vec<Link>> {
    let removed = if include_removed {
        ""
    } else {
        " AND removed_at IS NULL"
    };
    match node_id {
        Some(id) => collect(
            conn,
            &format!(
                "SELECT {LINK_COLUMNS} FROM sci_links WHERE node_id = ?1{removed} ORDER BY id"
            ),
            params![id],
            link_of,
        ),
        None => collect(
            conn,
            &format!("SELECT {LINK_COLUMNS} FROM sci_links WHERE 1 = 1{removed} ORDER BY id"),
            [],
            link_of,
        ),
    }
}

/// 結び付きを外す（行は消さず `removed_at` を入れる）。
pub fn remove_link(conn: &Connection, id: i64) -> Result<()> {
    get_link(conn, id)?;
    conn.execute(
        "UPDATE sci_links SET removed_at = ?1, updated_at = ?1 WHERE id = ?2",
        params![now_text(), id],
    )?;
    Ok(())
}

/// 読む札の読み具合（読み-ness）を入れ直す。論文の結び付きだけ。
pub fn set_read_state(conn: &Connection, id: i64, state: &str) -> Result<()> {
    let link = get_link(conn, id)?;
    if link.target_kind != "paper" {
        return Err(SciError::Invalid(format!(
            "結び付き {} は論文ではないため、読み具合を決められません",
            super::fmt_ref('L', id)
        )));
    }
    conn.execute(
        "UPDATE sci_links SET read_state = ?1, updated_at = ?2 WHERE id = ?3",
        params![state, now_text(), id],
    )?;
    Ok(())
}

/// その節点（`subtree` なら子孫も）に結び付いた論文の番号。
///
/// 外したもの・`irrelevant`・墓標の論文は除く。昇順で重複なし。
pub fn node_paper_ids(conn: &Connection, node_id: i64, subtree: bool) -> Result<Vec<i64>> {
    let mut targets = vec![node_id];
    if subtree {
        targets.extend(descendants(conn, node_id)?);
    }
    let tombstones = tombstoned_paper_ids(conn)?;
    let mut statement = conn.prepare(
        "SELECT target_ref FROM sci_links
          WHERE node_id = ?1 AND target_kind = 'paper' AND removed_at IS NULL
            AND read_state != 'irrelevant'",
    )?;
    let mut found: BTreeSet<i64> = BTreeSet::new();
    for target in targets {
        let rows = statement.query_map(params![target], |row| row.get::<_, String>(0))?;
        for row in rows {
            let text = row?;
            let Ok(id) = text.trim().parse::<i64>() else {
                continue;
            };
            if id > 0 && !tombstones.contains(&id) {
                found.insert(id);
            }
        }
    }
    Ok(found.into_iter().collect())
}

// ---- 決定の記録 -------------------------------------------------------------

/// 決定の記録を足して、できた番号を返す。
pub fn insert_decision(conn: &Connection, decision: &NewDecision<'_>) -> Result<i64> {
    conn.execute(
        "INSERT INTO sci_decisions
           (node_id, action, option_id, reason, evidence, rejected, counter_searched, actor, proposal_id)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            decision.node_id,
            decision.action,
            decision.option_id,
            decision.reason,
            write_vec(decision.evidence)?,
            write_vec(decision.rejected)?,
            i64::from(decision.counter_searched),
            decision.actor,
            decision.proposal_id,
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

/// 決定の記録を読む。`node_id` が `Some` ならその節点のものだけ（古い順）。
pub fn list_decisions(conn: &Connection, node_id: Option<i64>) -> Result<Vec<Decision>> {
    match node_id {
        Some(id) => collect(
            conn,
            &format!(
                "SELECT {DECISION_COLUMNS} FROM sci_decisions WHERE node_id = ?1 ORDER BY created_at, id"
            ),
            params![id],
            decision_of,
        ),
        None => collect(
            conn,
            &format!("SELECT {DECISION_COLUMNS} FROM sci_decisions ORDER BY created_at, id"),
            [],
            decision_of,
        ),
    }
}

// ---- 提案 -------------------------------------------------------------------

/// 提案を足して、できた番号を返す。
pub fn insert_proposal(conn: &Connection, proposal: &NewProposal<'_>) -> Result<i64> {
    let payload = serde_json::to_string(proposal.payload)?;
    conn.execute(
        "INSERT INTO sci_proposals (run_id, block, op, node_id, payload, reason, risk, status)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            proposal.run_id,
            proposal.block,
            proposal.op,
            proposal.node_id,
            payload,
            proposal.reason,
            proposal.risk,
            proposal.status,
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

/// 提案を 1 つ読む。無ければ `NotFound`。
pub fn get_proposal(conn: &Connection, id: i64) -> Result<Proposal> {
    conn.query_row(
        &format!("SELECT {PROPOSAL_COLUMNS} FROM sci_proposals WHERE id = ?1"),
        params![id],
        proposal_of,
    )
    .optional()?
    .ok_or_else(|| SciError::NotFound(format!("提案 {} がありません", super::fmt_ref('P', id))))
}

/// 提案を読む。`status` が `Some` ならその状態のものだけ（新しい順）。
pub fn list_proposals(conn: &Connection, status: Option<&str>) -> Result<Vec<Proposal>> {
    match status {
        Some(state) => collect(
            conn,
            &format!(
                "SELECT {PROPOSAL_COLUMNS} FROM sci_proposals WHERE status = ?1 ORDER BY created_at DESC, id DESC"
            ),
            params![state],
            proposal_of,
        ),
        None => collect(
            conn,
            &format!(
                "SELECT {PROPOSAL_COLUMNS} FROM sci_proposals ORDER BY created_at DESC, id DESC"
            ),
            [],
            proposal_of,
        ),
    }
}

/// 提案に仕分けを向ける。`decided_at` は今。
pub fn finish_proposal(
    conn: &Connection,
    id: i64,
    status: &str,
    result_ref: Option<&str>,
    decided_by: Option<&str>,
    note: &str,
) -> Result<()> {
    get_proposal(conn, id)?;
    conn.execute(
        "UPDATE sci_proposals
            SET status = ?1, result_ref = ?2, decided_by = ?3, decide_note = ?4, decided_at = ?5
          WHERE id = ?6",
        params![status, result_ref, decided_by, note, now_text(), id],
    )?;
    Ok(())
}

// ---- 墓標 -------------------------------------------------------------------

/// 墓標を足して、できた番号を返す。
pub fn insert_tombstone(conn: &Connection, tombstone: &NewTombstone<'_>) -> Result<i64> {
    if tombstone.reason.trim().is_empty() {
        return Err(SciError::Invalid(
            "外した論文・候補には理由を書く必要があります".to_string(),
        ));
    }
    if tombstone.paper_id.is_none()
        && tombstone.doi.is_none()
        && tombstone.arxiv_id.is_none()
        && tombstone.dedupe_key.is_none()
    {
        return Err(SciError::Invalid(
            "外した論文・候補は、論文番号・DOI・arXiv・重複キーのどれかで特定してください"
                .to_string(),
        ));
    }
    conn.execute(
        "INSERT INTO sci_tombstones
           (kind, paper_id, doi, arxiv_id, dedupe_key, title, reason, node_id, run_id, created_by)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        params![
            tombstone.kind,
            tombstone.paper_id,
            tombstone.doi,
            tombstone.arxiv_id,
            tombstone.dedupe_key,
            tombstone.title,
            tombstone.reason,
            tombstone.node_id,
            tombstone.run_id,
            tombstone.created_by,
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

/// 墓標を全部読む（番号の昇順）。
pub fn list_tombstones(conn: &Connection) -> Result<Vec<Tombstone>> {
    collect(
        conn,
        &format!("SELECT {TOMBSTONE_COLUMNS} FROM sci_tombstones ORDER BY id"),
        [],
        tombstone_of,
    )
}

/// どれかが一致する最初の墓標（`paper_id` / DOI（大文字小文字を区別しない）/ `arxiv_id` / `dedupe_key`）。
pub fn find_tombstone(
    conn: &Connection,
    paper_id: Option<i64>,
    doi: Option<&str>,
    arxiv_id: Option<&str>,
    dedupe_key: Option<&str>,
) -> Result<Option<Tombstone>> {
    if paper_id.is_none() && doi.is_none() && arxiv_id.is_none() && dedupe_key.is_none() {
        return Ok(None);
    }
    conn.query_row(
        &format!(
            "SELECT {TOMBSTONE_COLUMNS} FROM sci_tombstones
              WHERE (?1 IS NOT NULL AND paper_id = ?1)
                 OR (?2 IS NOT NULL AND lower(doi) = lower(?2))
                 OR (?3 IS NOT NULL AND arxiv_id = ?3)
                 OR (?4 IS NOT NULL AND dedupe_key = ?4)
              ORDER BY id LIMIT 1"
        ),
        params![paper_id, doi, arxiv_id, dedupe_key],
        tombstone_of,
    )
    .optional()
    .map_err(SciError::from)
}

/// 墓標にある論文番号の集合。
pub fn tombstoned_paper_ids(conn: &Connection) -> Result<HashSet<i64>> {
    let mut statement =
        conn.prepare("SELECT paper_id FROM sci_tombstones WHERE paper_id IS NOT NULL")?;
    let rows = statement.query_map([], |row| row.get::<_, i64>(0))?;
    let mut out = HashSet::new();
    for row in rows {
        out.insert(row?);
    }
    Ok(out)
}

// ---- 種 ---------------------------------------------------------------------

/// 種を足して、できた番号を返す。
pub fn insert_seed(
    conn: &Connection,
    title: &str,
    sentence: &str,
    path: &[i64],
    doc_ref: &str,
    run_id: Option<i64>,
) -> Result<i64> {
    if title.trim().is_empty() {
        return Err(SciError::Invalid("種の題名は空にできません".to_string()));
    }
    conn.execute(
        "INSERT INTO sci_seeds (title, sentence, path, doc_ref, run_id) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![title, sentence, write_vec(path)?, doc_ref, run_id],
    )?;
    Ok(conn.last_insert_rowid())
}

/// 種を全部読む（番号の昇順）。
pub fn list_seeds(conn: &Connection) -> Result<Vec<Seed>> {
    collect(
        conn,
        &format!("SELECT {SEED_COLUMNS} FROM sci_seeds ORDER BY id"),
        [],
        seed_of,
    )
}

// ---- 検索の記録 -------------------------------------------------------------

/// 検索の記録を足して、できた番号を返す。
pub fn insert_search(conn: &Connection, search: &NewSearch<'_>) -> Result<i64> {
    conn.execute(
        "INSERT INTO sci_searches
           (run_id, node_id, question_id, tool, query, scope, widen_reason, hits, hit_papers, set_id)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        params![
            search.run_id,
            search.node_id,
            search.question_id,
            search.tool,
            search.query,
            search.scope,
            search.widen_reason,
            search.hits,
            write_vec(search.hit_papers)?,
            search.set_id,
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

/// 検索の記録を読む。`node_id` が `Some` ならその節点のものだけ（新しい順）。
pub fn list_searches(conn: &Connection, node_id: Option<i64>) -> Result<Vec<SearchRecord>> {
    match node_id {
        Some(id) => collect(
            conn,
            &format!(
                "SELECT {SEARCH_COLUMNS} FROM sci_searches WHERE node_id = ?1 ORDER BY created_at DESC, id DESC"
            ),
            params![id],
            search_of,
        ),
        None => collect(
            conn,
            &format!("SELECT {SEARCH_COLUMNS} FROM sci_searches ORDER BY created_at DESC, id DESC"),
            [],
            search_of,
        ),
    }
}

// ---- プロジェクトの値 -----------------------------------------------------------

/// `sci_meta` の値を 1 つ読む。
pub fn meta_get(conn: &Connection, key: &str) -> Result<Option<String>> {
    conn.query_row(
        "SELECT value FROM sci_meta WHERE key = ?1",
        params![key],
        |row| row.get::<_, String>(0),
    )
    .optional()
    .map_err(SciError::from)
}

/// `sci_meta` に値を 1 つ書く（同じキーは上書き）。
pub fn meta_set(conn: &Connection, key: &str, value: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO sci_meta (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![key, value],
    )?;
    Ok(())
}

/// `sci_meta` に `type` があれば true。
pub fn is_initialized(conn: &Connection) -> Result<bool> {
    Ok(meta_get(conn, "type")?.is_some())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::Database;

    /// 全表のメモリ DB。DB ごと持ち、接続は `db.connection()` で取る。
    fn db() -> Database {
        Database::in_memory().expect("メモリ DB")
    }

    /// 項目を足す。
    fn add_item(conn: &Connection, title: &str) -> i64 {
        insert_node(
            conn,
            &NewNode {
                kind: "item",
                item_key: None,
                types: &[],
                title,
                body: "",
                state: "open",
                fixed: false,
                ord: 0,
                created_by: "user",
            },
        )
        .expect("項目を足す")
    }

    fn add_option(conn: &Connection, node_id: i64, title: &str) -> i64 {
        insert_node(
            conn,
            &NewNode {
                kind: "option",
                item_key: None,
                types: &[],
                title,
                body: "",
                state: "candidate",
                fixed: false,
                ord: 0,
                created_by: "user",
            },
        )
        .expect("候補を足す")
        .max(node_id)
    }

    /// 問いを 1 つだけ足ってその番号を返す（テストの便宜）。
    fn question_of(conn: &Connection, node: i64) -> i64 {
        insert_question(conn, node, "問い", false, "user").unwrap()
    }

    fn link_paper(
        conn: &Connection,
        node_id: i64,
        paper_id: &str,
        role: &str,
        read_state: &str,
    ) -> i64 {
        upsert_link(
            conn,
            &NewLink {
                node_id,
                target_kind: "paper",
                target_ref: paper_id,
                role: Some(role),
                read_state: Some(read_state),
                reason: "",
                proposal_id: None,
                auto: false,
                created_by: "user",
            },
        )
        .expect("論文を結ぶ")
    }

    #[test]
    fn migration_creates_sci_tables() {
        let database = db();
        let conn = database.connection();
        let mut statement = conn
            .prepare("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name")
            .expect("表を一覧する準備");
        let tables: Vec<String> = statement
            .query_map([], |row| row.get::<_, String>(0))
            .expect("表を一覧する")
            .map(std::result::Result::unwrap)
            .collect();
        for name in [
            "sci_meta",
            "sci_nodes",
            "sci_edges",
            "sci_questions",
            "sci_findings",
            "sci_links",
            "sci_decisions",
            "sci_proposals",
            "sci_tombstones",
            "sci_seeds",
            "sci_searches",
        ] {
            assert!(
                tables.contains(&name.to_string()),
                "{name} が無い: {tables:?}"
            );
        }

        // search_sets には 4 列が足されている。
        let mut statement = conn
            .prepare("PRAGMA table_info(search_sets)")
            .expect("列を見る");
        let names: Vec<String> = statement
            .query_map([], |row| row.get::<_, String>(1))
            .expect("列を見る")
            .map(std::result::Result::unwrap)
            .collect();
        for name in ["sci_node", "sci_question", "purpose", "extract"] {
            assert!(
                names.contains(&name.to_string()),
                "search_sets に {name} が無い"
            );
        }
    }

    #[test]
    fn node_state_check_rejects_wrong_kind() {
        let database = db();
        let conn = database.connection();
        let id = add_item(conn, "小目的");

        // Rust の側では日本語の文で止める。
        let err = set_node_state(conn, id, "candidate").unwrap_err();
        let SciError::Invalid(message) = err else {
            panic!("Invalid ではない");
        };
        assert_eq!(message, "この節点にはその状態を入れられません: candidate");
        // 変わっていない。
        assert_eq!(get_node(conn, id).unwrap().state, "open");

        // SQL の CHECK にも当たる（最後の守り）。
        let sql = conn.execute(
            "UPDATE sci_nodes SET state = 'candidate' WHERE id = ?1",
            params![id],
        );
        assert!(sql.is_err(), "CHECK が効いていない");
    }

    #[test]
    fn node_state_accepts_kind_states_and_keeps_time() {
        let database = db();
        let conn = database.connection();
        let id = add_item(conn, "小目的");
        set_node_state(conn, id, "decided").unwrap();
        let after = get_node(conn, id).unwrap();
        assert_eq!(after.state, "decided");

        // 同じ状態を入れ直しても時刻は変わらない。
        let again = get_node(conn, id).unwrap();
        set_node_state(conn, id, "decided").unwrap();
        let same = get_node(conn, id).unwrap();
        assert_eq!(same.state_changed_at, again.state_changed_at);
        assert_eq!(same.updated_at, again.updated_at);

        let option = add_option(conn, id, "候補 1");
        set_node_state(conn, option, "rejected").unwrap();
        assert_eq!(get_node(conn, option).unwrap().state, "rejected");
        assert!(set_node_state(conn, option, "open").is_err());
    }

    #[test]
    fn node_types_are_written_in_canonical_order() {
        let database = db();
        let conn = database.connection();
        let id = insert_node(
            conn,
            &NewNode {
                kind: "item",
                item_key: Some("small-goal"),
                types: &["verify".to_string(), "choose".to_string()],
                title: "小目的",
                body: "",
                state: "open",
                fixed: false,
                ord: 0,
                created_by: "user",
            },
        )
        .unwrap();
        let node = get_node(conn, id).unwrap();
        assert_eq!(node.types, vec!["choose".to_string(), "verify".to_string()]);
        let raw: String = conn
            .query_row(
                "SELECT item_types FROM sci_nodes WHERE id = ?1",
                params![id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(raw, "choose,verify");
        assert_eq!(
            find_item_by_key(conn, "small-goal").unwrap().unwrap().id,
            id
        );
    }

    #[test]
    fn missing_ids_are_not_found() {
        let database = db();
        let conn = database.connection();
        let messages = [
            get_node(conn, 5).unwrap_err().to_string(),
            get_question(conn, 5).unwrap_err().to_string(),
            get_finding(conn, 5).unwrap_err().to_string(),
            get_link(conn, 5).unwrap_err().to_string(),
            get_proposal(conn, 5).unwrap_err().to_string(),
        ];
        assert_eq!(messages[0], "節点 N5 がありません");
        assert_eq!(messages[1], "問い Q5 がありません");
        assert_eq!(messages[2], "知見 F5 がありません");
        assert_eq!(messages[3], "結び付き L5 がありません");
        assert_eq!(messages[4], "提案 P5 がありません");
    }

    #[test]
    fn decisions_are_append_only() {
        let database = db();
        let conn = database.connection();
        let node = add_item(conn, "小目的");
        let id = insert_decision(
            conn,
            &NewDecision {
                node_id: node,
                action: "decide",
                option_id: None,
                reason: "価値が高い",
                evidence: &[3, 4],
                rejected: &[RejectedOption {
                    option: 9,
                    reason: "再現ができない".to_string(),
                }],
                counter_searched: true,
                actor: "user",
                proposal_id: None,
            },
        )
        .unwrap();
        let decision = list_decisions(conn, Some(node)).unwrap().remove(0);
        assert_eq!(decision.id, id);
        assert_eq!(decision.evidence, vec![3, 4]);
        assert_eq!(decision.rejected[0].option, 9);
        assert!(decision.counter_searched);

        let updated = conn.execute(
            "UPDATE sci_decisions SET reason = 'x' WHERE id = ?1",
            params![id],
        );
        let deleted = conn.execute("DELETE FROM sci_decisions WHERE id = ?1", params![id]);
        for outcome in [updated, deleted] {
            let err = outcome.expect_err("追記だけのはず");
            assert!(
                err.to_string().contains("sci_decisions は追記だけです"),
                "{err}"
            );
        }
    }

    #[test]
    fn link_upsert_restores_removed() {
        let database = db();
        let conn = database.connection();
        let node = add_item(conn, "小目的");
        let id = link_paper(conn, node, "37", "exemplar", "unread");
        remove_link(conn, id).unwrap();
        assert!(get_link(conn, id).unwrap().removed_at.is_some());
        assert!(list_links(conn, Some(node), false).unwrap().is_empty());

        let again = upsert_link(
            conn,
            &NewLink {
                node_id: node,
                target_kind: "paper",
                target_ref: "37",
                role: Some("exemplar"),
                read_state: Some("read"),
                reason: "読み直した",
                proposal_id: None,
                auto: false,
                created_by: "user",
            },
        )
        .unwrap();
        assert_eq!(again, id, "同じ組は増やさない");
        let link = get_link(conn, id).unwrap();
        assert!(link.removed_at.is_none());
        assert_eq!(link.read_state.as_deref(), Some("read"));
        assert_eq!(link.reason, "読み直した");
    }

    #[test]
    fn paper_link_needs_role() {
        let database = db();
        let conn = database.connection();
        let node = add_item(conn, "小目的");
        let err = upsert_link(
            conn,
            &NewLink {
                node_id: node,
                target_kind: "paper",
                target_ref: "37",
                role: None,
                read_state: Some("unread"),
                reason: "",
                proposal_id: None,
                auto: false,
                created_by: "user",
            },
        )
        .expect_err("論文には役割が要る");
        assert!(err.to_string().contains("CHECK constraint failed"), "{err}");

        // 論文でなければ役割も読み具合も要らない。
        let run = upsert_link(
            conn,
            &NewLink {
                node_id: node,
                target_kind: "run",
                target_ref: "120",
                role: None,
                read_state: None,
                reason: "",
                proposal_id: None,
                auto: false,
                created_by: "user",
            },
        )
        .expect("実行は役割なしで結べる");
        assert!(get_link(conn, run).unwrap().role.is_none());
        assert!(set_read_state(conn, run, "read").is_err());
    }

    #[test]
    fn descendants_stop_on_cycle() {
        let database = db();
        let conn = database.connection();
        let a = add_item(conn, "A");
        let b = add_item(conn, "B");
        let c = add_item(conn, "C");
        insert_edge(conn, "parent", a, b).unwrap();
        insert_edge(conn, "parent", b, c).unwrap();
        // 輪（元に戻る線）。上の線と同じ組の重複は増えない。
        insert_edge(conn, "parent", c, a).unwrap();
        assert_eq!(descendants(conn, a).unwrap(), vec![b, c]);
        assert_eq!(descendants(conn, c).unwrap(), vec![a, b]);
        assert!(descendants(conn, 999).unwrap().is_empty());
        assert_eq!(parents_of(conn, b).unwrap(), vec![a]);
        assert_eq!(children_of(conn, a).unwrap(), vec![b]);
    }

    #[test]
    fn insert_edge_is_idempotent() {
        let database = db();
        let conn = database.connection();
        let a = add_item(conn, "A");
        let b = add_item(conn, "B");
        let first = insert_edge(conn, "parent", a, b).unwrap();
        let second = insert_edge(conn, "parent", a, b).unwrap();
        assert_eq!(first, second);
        assert_eq!(list_edges(conn).unwrap().len(), 1);
    }

    #[test]
    fn node_paper_ids_skip_tombstoned_and_irrelevant() {
        let database = db();
        let conn = database.connection();
        let root = add_item(conn, "小目的");
        let child = add_item(conn, "下");
        insert_edge(conn, "parent", root, child).unwrap();

        let unread = link_paper(conn, root, "40", "compare", "unread");
        let irrelevant = link_paper(conn, root, "41", "counter", "irrelevant");
        let child_paper = link_paper(conn, child, "42", "exemplar", "unread");
        // 対象でない節点の論文。
        let other = add_item(conn, "別の節点");
        link_paper(conn, other, "43", "exemplar", "unread");
        // 外した論文。
        let removed = link_paper(conn, root, "44", "background", "unread");
        remove_link(conn, removed).unwrap();

        // 41 は墓標に入れる。
        insert_tombstone(
            conn,
            &NewTombstone {
                kind: "paper",
                paper_id: Some(41),
                doi: None,
                arxiv_id: None,
                dedupe_key: None,
                title: "関係なし",
                reason: "他の研究が無い",
                node_id: Some(root),
                run_id: None,
                created_by: "user",
            },
        )
        .unwrap();

        assert_eq!(node_paper_ids(conn, root, false).unwrap(), vec![40]);
        assert_eq!(node_paper_ids(conn, root, true).unwrap(), vec![40, 42]);
        assert_eq!(node_paper_ids(conn, child, false).unwrap(), vec![42]);

        // 読み具合は入れ直せる。
        set_read_state(conn, unread, "reflected").unwrap();
        assert_eq!(
            get_link(conn, unread).unwrap().read_state.as_deref(),
            Some("reflected")
        );
        assert_eq!(
            get_link(conn, irrelevant).unwrap().read_state.as_deref(),
            Some("irrelevant")
        );
        assert_eq!(get_link(conn, child_paper).unwrap().node_id, child);
    }

    #[test]
    fn find_tombstone_matches_any_key() {
        let database = db();
        let conn = database.connection();
        insert_tombstone(
            conn,
            &NewTombstone {
                kind: "paper",
                paper_id: None,
                doi: Some("10.1234/ABC"),
                arxiv_id: Some("2401.00001"),
                dedupe_key: None,
                title: "外した論文",
                reason: "主題が違う",
                node_id: None,
                run_id: None,
                created_by: "user",
            },
        )
        .unwrap();
        assert!(
            find_tombstone(conn, Some(7), None, None, None)
                .unwrap()
                .is_none()
        );
        assert!(
            find_tombstone(conn, None, Some("10.1234/abc"), None, None)
                .unwrap()
                .is_some()
        );
        assert!(
            find_tombstone(conn, None, None, Some("2401.00001"), None)
                .unwrap()
                .is_some()
        );
        assert!(
            find_tombstone(conn, None, None, None, None)
                .unwrap()
                .is_none()
        );
        assert!(tombstoned_paper_ids(conn).unwrap().is_empty());

        // 理由の無い墓標は作れない。
        assert!(
            insert_tombstone(
                conn,
                &NewTombstone {
                    kind: "paper",
                    paper_id: Some(9),
                    doi: None,
                    arxiv_id: None,
                    dedupe_key: None,
                    title: "",
                    reason: "   ",
                    node_id: None,
                    run_id: None,
                    created_by: "user",
                }
            )
            .is_err()
        );
    }

    #[test]
    fn json_columns_round_trip_and_tolerate_garbage() {
        let database = db();
        let conn = database.connection();
        let node = add_item(conn, "小目的");
        let question = insert_question(conn, node, "何を見るか", true, "user").unwrap();
        let finding = insert_finding(
            conn,
            &NewFinding {
                node_id: node,
                question_id: Some(question),
                text: "波形を見るのが効く",
                cites: &["#37 §3.2".to_string()],
                polarity: "support",
                level: Some(2),
                source: "run",
                report_id: None,
                paper_id: Some(37),
                run_id: Some(120),
                proposal_id: None,
                auto: true,
                created_by: "run:120",
            },
        )
        .unwrap();
        let loaded = get_finding(conn, finding).unwrap();
        assert_eq!(loaded.cites, vec!["#37 §3.2".to_string()]);
        assert!(loaded.auto);
        assert_eq!(loaded.level, Some(2));
        assert!(list_findings(conn, Some(node), false).unwrap().len() == 1);
        remove_finding(conn, finding).unwrap();
        assert!(list_findings(conn, Some(node), false).unwrap().is_empty());
        assert_eq!(list_findings(conn, Some(node), true).unwrap().len(), 1);

        // 読めない JSON は空として扱う。
        conn.execute("UPDATE sci_findings SET cites = 'oops'", [])
            .unwrap();
        assert!(get_finding(conn, finding).unwrap().cites.is_empty());

        answer_question(conn, question, "波形").unwrap();
        assert_eq!(get_question(conn, question).unwrap().state, "answered");
        withdraw_question(conn, question).unwrap();
        assert_eq!(get_question(conn, question).unwrap().state, "withdrawn");
    }

    #[test]
    fn proposals_seeds_searches_and_meta_round_trip() {
        let database = db();
        let conn = database.connection();
        let node = add_item(conn, "小目的");
        assert!(!is_initialized(conn).unwrap());
        meta_set(conn, "type", "problem").unwrap();
        assert!(is_initialized(conn).unwrap());
        assert_eq!(meta_get(conn, "type").unwrap().as_deref(), Some("problem"));
        meta_set(conn, "type", "method").unwrap();
        assert_eq!(meta_get(conn, "type").unwrap().as_deref(), Some("method"));
        assert_eq!(meta_get(conn, "無い").unwrap(), None);

        let payload = serde_json::json!({"text": "波形", "polarity": "support"});
        let first = insert_proposal(
            conn,
            &NewProposal {
                run_id: Some(120),
                block: "sci-compare",
                op: "add_finding",
                node_id: Some(node),
                payload: &payload,
                reason: "先行の報告がそう言った",
                risk: "human",
                status: "pending",
            },
        )
        .unwrap();
        let second = insert_proposal(
            conn,
            &NewProposal {
                run_id: Some(121),
                block: "sci-compare",
                op: "add_question",
                node_id: Some(node),
                payload: &payload,
                reason: "",
                risk: "auto",
                status: "applied",
            },
        )
        .unwrap();
        // 新しい順。
        let listed = list_proposals(conn, None).unwrap();
        assert_eq!(listed[0].id, second);
        assert_eq!(listed[1].id, first);
        assert_eq!(list_proposals(conn, Some("pending")).unwrap().len(), 1);
        let one = get_proposal(conn, first).unwrap();
        assert_eq!(one.payload["polarity"], serde_json::json!("support"));

        finish_proposal(conn, first, "accepted", Some("F9"), Some("user"), "採る").unwrap();
        let done = get_proposal(conn, first).unwrap();
        assert_eq!(done.status, "accepted");
        assert_eq!(done.result_ref.as_deref(), Some("F9"));
        assert_eq!(done.decided_by.as_deref(), Some("user"));
        assert!(done.decided_at.is_some());

        // 読めない payload は Null として扱う。
        conn.execute(
            "UPDATE sci_proposals SET payload = 'oops' WHERE id = ?1",
            params![second],
        )
        .unwrap();
        assert_eq!(
            get_proposal(conn, second).unwrap().payload,
            serde_json::Value::Null
        );

        let seed = insert_seed(
            conn,
            "発作予測",
            "次の利用者",
            &[node, 7],
            "doc.md",
            Some(120),
        )
        .unwrap();
        let seeds = list_seeds(conn).unwrap();
        assert_eq!(seeds[0].id, seed);
        assert_eq!(seeds[0].path, vec![node, 7]);
        assert_eq!(seeds[0].doc_ref, "doc.md");
        assert!(insert_seed(conn, "  ", "", &[], "", None).is_err());

        let search = insert_search(
            conn,
            &NewSearch {
                run_id: Some(120),
                node_id: Some(node),
                question_id: Some(question_of(conn, node)),
                tool: "search",
                query: "EEG 癲癇 予測",
                scope: "node:1",
                widen_reason: "Hits が少なかったため",
                hits: 3,
                hit_papers: &[37, 40],
                set_id: None,
            },
        )
        .unwrap();
        let searches = list_searches(conn, Some(node)).unwrap();
        assert_eq!(searches[0].id, search);
        assert_eq!(searches[0].hit_papers, vec![37, 40]);
        assert!(!list_searches(conn, None).unwrap().is_empty());
    }

    #[test]
    fn migration_46_runs_once() {
        let dir = std::env::temp_dir().join(format!("sai-sci-46-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("一時フォルダを作る");
        let path = dir.join("papr.db");

        {
            let db = Database::open(&path).expect("1 回目に開く");
            let applied: i64 = db
                .connection()
                .query_row(
                    "SELECT count(*) FROM schema_migrations WHERE version = 46",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(applied, 1);
            // 2 回目の途中まで同じプロセスの中で開いても 1 行だけのはず。
            drop(db);
        }
        {
            let db = Database::open(&path).expect("2 回目に開く");
            let applied: i64 = db
                .connection()
                .query_row(
                    "SELECT count(*) FROM schema_migrations WHERE version = 46",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(applied, 1, "46 は 1 回だけ流される");
        }

        let _ = std::fs::remove_dir_all(&dir);
    }
}
