//! research-tree（Q・A・F・E・S）を変える操作（op）。C3 の表が正。
//!
//! 入口は 2 つ。agent の [`op`]（`sai-sci op`。1 件から 20 件まで、全部を 1 つのトランザクションで行う）と、
//! 利用者の [`apply_user`]（`sai-sci apply`）。
//! 表（[`OP_SPECS`]）→ 形の検査 → 中身の検査 → 書く、を [`execute`] 1 本で行う。
//! 検査が全部済んでから書くので、途中で弾かれた op は何も残さない。

use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;
use serde_json::{Map, Value, json};

// 範囲の数え方は compute.rs の 1 か所にある（U03 の読み出しと同じ関数）。ここからも同じ名前で使えるようにする。
pub use super::compute::{Range, sprout_range};
use super::defs::{self, Defs};
use super::store::{self, ApprovalRow, NewNode, NodeRow};
use super::{Kind, Result, RtError, fmt_id, parse_id};
use sai_paper_core::paths::PaperRoots;
use sai_paper_core::{seen_ids, textcheck};
use std::path::PathBuf;

const MAX_OPS: usize = 20;
const ALL_KINDS: &[Kind] = &[Kind::Q, Kind::A, Kind::F, Kind::E, Kind::S];
const STANCES: &[&str] = &["support", "deny", "issue"];

// ---- 表 -------------------------------------------------------------------------------

/// 誰が出せるか。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Risk {
    /// agent がそのまま出せる。
    Free,
    /// agent は承認（質問で取った許可）が要る。1 件の承認で、その `op` の呼び出しの中の許可が要る op を全部通す。
    Approval,
    /// 利用者だけが行う。
    UserOnly,
}

/// 項目の型。
#[derive(Debug, Clone, Copy)]
pub enum Ty {
    /// Node の ID（`Q12`）。
    Id,
    /// Node の ID か `SEED`（DB の行を持たない起点）。
    SeedOrId,
    /// 文（字数の範囲。文字種も検査する）。
    Text(usize, usize),
    /// 整数。
    Int,
    /// 決まった語のどれか。
    Enum(&'static [&'static str]),
    /// 文字列の配列（個数の範囲）。
    List(usize, usize),
}

/// op の項目 1 つ。
#[derive(Debug, Clone, Copy)]
pub struct Field {
    pub name: &'static str,
    pub ty: Ty,
    pub required: bool,
}

/// op 1 つの決まり。
#[derive(Debug, Clone, Copy)]
pub struct OpSpec {
    pub op: &'static str,
    pub risk: Risk,
    pub about: &'static str,
    pub fields: &'static [Field],
}

const fn req(name: &'static str, ty: Ty) -> Field {
    Field { name, ty, required: true }
}
const fn opt(name: &'static str, ty: Ty) -> Field {
    Field { name, ty, required: false }
}

static OP_SPECS: &[OpSpec] = &[
    OpSpec {
        op: "add_q",
        risk: Risk::Approval,
        about: "SEED・Q・A の下に child_Q を足す。SEED の下なら top_Q になる",
        fields: &[req("parent", Ty::SeedOrId), req("text", Ty::Text(1, 300)), opt("guide", Ty::Text(1, 80))],
    },
    OpSpec {
        op: "insert_q",
        risk: Risk::Approval,
        about: "Q の下に新しい child_Q を挟み、Q の生きた A を全部その下へ移す",
        fields: &[req("q", Ty::Id), req("text", Ty::Text(1, 300)), opt("guide", Ty::Text(1, 80))],
    },
    OpSpec {
        op: "move_a",
        risk: Risk::Approval,
        about: "Q の生きた A を全部、別の Q の下へ移す",
        fields: &[req("from", Ty::Id), req("to", Ty::Id)],
    },
    OpSpec {
        op: "copy",
        risk: Risk::Approval,
        about: "A と、その生きた F を別の Q の下へ写す（Q と E は写さない）",
        fields: &[req("node", Ty::Id), req("to", Ty::Id)],
    },
    OpSpec {
        op: "remove",
        risk: Risk::Approval,
        about: "Node を下の Node ごと消す（top_Q も消せる）",
        fields: &[req("node", Ty::Id)],
    },
    OpSpec {
        op: "add_a",
        risk: Risk::Free,
        about: "Q の下に A（答え）を足す",
        fields: &[req("q", Ty::Id), req("text", Ty::Text(1, 120)), opt("body", Ty::Text(0, 2000))],
    },
    OpSpec {
        op: "add_f",
        risk: Risk::Free,
        about: "A の下に F（根拠）を足す。立場（stance）が要る。数字を含む文には出典（cites）か REPORT のトークン（report）が要る",
        fields: &[
            req("parent", Ty::Id),
            req("text", Ty::Text(1, 1200)),
            req("stance", Ty::Enum(STANCES)),
            opt("cites", Ty::List(0, 10)),
            opt("paper", Ty::Int),
            opt("report", Ty::Text(1, 100)),
        ],
    },
    OpSpec {
        op: "add_e",
        risk: Risk::Free,
        about: "SPROUT か E の下に E（実験）を足す",
        fields: &[
            req("parent", Ty::Id),
            req("text", Ty::Text(1, 120)),
            opt("body", Ty::Text(0, 2000)),
            opt("mlflow", Ty::Text(0, 200)),
        ],
    },
    OpSpec {
        op: "add_sprout",
        risk: Risk::Free,
        about: "A を 1 つ以上選んで SPROUT を作る（同じ A は 2 回書けない）",
        fields: &[req("title", Ty::Text(1, 120)), opt("body", Ty::Text(0, 2000)), req("paths", Ty::List(1, 20))],
    },
    OpSpec {
        op: "set_stance",
        risk: Risk::Free,
        about: "F の立場（support / deny / issue）を変える",
        fields: &[req("node", Ty::Id), req("stance", Ty::Enum(STANCES))],
    },
    OpSpec {
        op: "set_mlflow",
        risk: Risk::Free,
        about: "E の MLflow の結び付けを変える（空文字で外す）",
        fields: &[req("node", Ty::Id), req("mlflow", Ty::Text(0, 200))],
    },
    OpSpec { op: "star", risk: Risk::UserOnly, about: "Node に ★ を付ける（目印）", fields: &[req("node", Ty::Id)] },
    OpSpec { op: "unstar", risk: Risk::UserOnly, about: "Node の ★ を外す", fields: &[req("node", Ty::Id)] },
    OpSpec {
        op: "edit",
        risk: Risk::UserOnly,
        about: "Node の文・本文を書き換える",
        fields: &[req("node", Ty::Id), opt("text", Ty::Text(1, 1200)), opt("body", Ty::Text(0, 2000))],
    },
];

#[must_use]
pub fn op_specs() -> &'static [OpSpec] {
    OP_SPECS
}

#[must_use]
pub fn op_spec(op: &str) -> Option<&'static OpSpec> {
    OP_SPECS.iter().find(|s| s.op == op)
}

#[must_use]
pub fn is_known_op(op: &str) -> bool {
    op_spec(op).is_some()
}

#[must_use]
pub fn is_user_only_op(op: &str) -> bool {
    op_spec(op).is_some_and(|s| s.risk == Risk::UserOnly)
}

/// agent が承認（許可）を要る op か。
#[must_use]
pub fn needs_approval(op: &str) -> bool {
    op_spec(op).is_some_and(|s| s.risk == Risk::Approval)
}

fn field_schema(f: &Field) -> Value {
    match f.ty {
        Ty::Id => json!({"type": "string", "pattern": "^[QAFES][1-9][0-9]{0,8}$"}),
        Ty::SeedOrId => json!({"type": "string", "pattern": "^(SEED|[QAFES][1-9][0-9]{0,8})$"}),
        Ty::Text(min, max) => json!({"type": "string", "minLength": min, "maxLength": max}),
        Ty::Int => json!({"type": "integer"}),
        Ty::Enum(values) => json!({"type": "string", "enum": values}),
        Ty::List(min, max) => json!({"type": "array", "items": {"type": "string"}, "minItems": min, "maxItems": max}),
    }
}

/// `sai-sci op` の入力の JSON Schema。`UserOnly` の op は含めない。
#[must_use]
pub fn op_schema() -> Value {
    let variants: Vec<Value> = OP_SPECS
        .iter()
        .filter(|s| s.risk != Risk::UserOnly)
        .map(|s| {
            let mut props = Map::new();
            props.insert("op".into(), json!({"const": s.op}));
            for f in s.fields {
                props.insert(f.name.into(), field_schema(f));
            }
            let mut required = vec!["op"];
            required.extend(s.fields.iter().filter(|f| f.required).map(|f| f.name));
            json!({"description": s.about, "type": "object", "properties": props,
                   "required": required, "additionalProperties": false})
        })
        .collect();
    json!({
        "type": "object",
        "properties": {"ops": {"type": "array", "minItems": 1, "maxItems": MAX_OPS,
                                "items": {"oneOf": variants}}},
        "required": ["ops"],
    })
}

/// ブロックが出せる op（frontmatter の `sci_ops`）。書いていなければ `Free` の op 全部。
#[must_use]
pub fn allowed_ops(roots: &PaperRoots, block: &str) -> Vec<String> {
    let free = || OP_SPECS.iter().filter(|s| s.risk == Risk::Free).map(|s| s.op.to_string()).collect::<Vec<_>>();
    let Some(pairs) = super::action::block_frontmatter(roots, block) else { return free() };
    let found: Vec<String> = pairs
        .into_iter()
        .find(|(key, _)| key == "sci_ops")
        .map(|(_, v)| v.split(',').map(str::trim).filter(|p| !p.is_empty()).map(str::to_string).collect())
        .unwrap_or_default();
    if found.is_empty() { free() } else { found }
}

// ---- 入口の型 ---------------------------------------------------------------------------

/// op を出した側（agent の run と block）。
#[derive(Debug, Clone, Default)]
pub struct Actor {
    pub run_id: Option<i64>,
    pub block: Option<String>,
}

/// 1 件の op の結果（`sai-sci op` の results・`sai-sci apply` の出力）。
#[derive(Debug, Clone, Serialize)]
pub struct OpResult {
    pub op: String,
    pub result: String,
    /// SPROUT の範囲（add_sprout のときだけ）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub range: Option<Range>,
}

/// `sai-sci op` の出力。承認を使ったときだけ `approval` が入る。
#[derive(Debug, Clone, Serialize)]
pub struct BatchResult {
    pub results: Vec<OpResult>,
    pub approval: Option<Value>,
}

fn invalid<T>(message: impl Into<String>) -> Result<T> {
    Err(RtError::Invalid(message.into()))
}

/// 書く側の情報。
struct Cx<'a> {
    conn: &'a Connection,
    defs: &'a Defs,
    /// Node の `created_by` と `rt_log` の `actor`。
    by: String,
    run_id: Option<i64>,
    /// この op の呼び出しで使う許可（許可が要る op があったときだけ）。
    approval: Option<ApprovalRow>,
    /// `add_f` の `cites` を照合する台帳（agent の op だけ `Some`。利用者の apply は `None`）。
    ledger: Option<Ledger>,
}

/// 台帳（seen_ids）の置き場と照合する run。`op` は環境変数から作り、試験は一時フォルダを渡す。
struct Ledger {
    root: PathBuf,
    run_id: i64,
}

/// `SAI_PARENT_RUN_ID` と project root が両方あるときだけ台帳を使う。
fn ledger_from_env() -> Option<Ledger> {
    let run_id = seen_ids::run_id_from_env()?;
    let root = sai_paper_core::paths::project_root()?;
    Some(Ledger { root, run_id })
}

fn run_by(run_id: Option<i64>) -> String {
    run_id.map_or_else(|| "agent".to_string(), |id| format!("run:{id}"))
}

// ---- 入口 -------------------------------------------------------------------------------

/// agent の op。`input` は `{"ops": [ {op…}, … ]}`（20 件まで）。
///
/// 1 件でも通らなければ何も保存せず、最初の違反を `Invalid("<n> 件目（<op>）: <理由>")` で返す。
pub fn op(conn: &Connection, roots: &PaperRoots, defs: &Defs, actor: &Actor, input: &Value) -> Result<BatchResult> {
    op_in(conn, roots, defs, actor, input, ledger_from_env())
}

/// [`op`] の本体。台帳を引数で受け、試験では一時フォルダの台帳を渡す。
fn op_in(
    conn: &Connection,
    roots: &PaperRoots,
    defs: &Defs,
    actor: &Actor,
    input: &Value,
    ledger: Option<Ledger>,
) -> Result<BatchResult> {
    let Some(items) = input.get("ops").and_then(Value::as_array) else {
        return invalid("ops（配列）が必要です");
    };
    if items.is_empty() || items.len() > MAX_OPS {
        return invalid(format!("ops は 1〜{MAX_OPS} 件です"));
    }
    // 許可が要る op が 1 つでもあれば、承認を先に確かめる（無ければ入力を見る前に終了コード 4）。
    let gated: Vec<String> = items
        .iter()
        .filter_map(|item| item.get("op").and_then(Value::as_str))
        .filter(|name| needs_approval(name))
        .map(str::to_string)
        .collect();
    let tx = conn.unchecked_transaction()?;
    let approval = if gated.is_empty() { None } else { Some(take_approval_for(&tx, actor)?) };
    let block = actor.block.as_deref().unwrap_or("");
    let allowed = allowed_ops(roots, block);
    let cx = Cx { conn: &tx, defs, by: run_by(actor.run_id), run_id: actor.run_id, approval, ledger };
    let mut results = Vec::new();
    for (index, item) in items.iter().enumerate() {
        let name = item.get("op").and_then(Value::as_str).unwrap_or("");
        let at = |e: RtError| match e {
            RtError::Invalid(m) => RtError::Invalid(format!("{} 件目（{name}）: {m}", index + 1)),
            other => other,
        };
        let Some(spec) = op_spec(name) else {
            return Err(at(RtError::Invalid(format!("知らない op です: {name}"))));
        };
        if spec.risk == Risk::UserOnly {
            return Err(at(RtError::Invalid("USER_OP です（利用者だけが行います）".into())));
        }
        if !allowed.iter().any(|a| a == name) {
            return Err(at(RtError::Invalid(format!("このブロックでは {name} を出せません"))));
        }
        let (result, range) = execute(&cx, item).map_err(at)?;
        results.push(OpResult { op: name.to_string(), result, range });
    }
    // 全部通ったので、許可を使い切る（途中で弾かれたときは tx ごと戻り、許可も残る）。
    if let Some(row) = &cx.approval {
        store::consume_approval(&tx, row.id, &gated)?;
    }
    let approval = cx.approval.as_ref().map(|a| json!({"qid": a.qid, "answered_by": a.answered_by}));
    tx.commit()?;
    Ok(BatchResult { results, approval })
}

/// agent の run の許可を 1 件取る（消費はしない）。無ければ終了コード 4。
fn take_approval_for(conn: &Connection, actor: &Actor) -> Result<ApprovalRow> {
    let Some(run) = actor.run_id else {
        return Err(RtError::Conflict("run_id がありません（--run か SAI_PARENT_RUN_ID）".into()));
    };
    store::take_approval(conn, run)?
        .ok_or_else(|| RtError::Conflict("許可がありません: 先に質問ツールで「許可する」を選んでもらってください".into()))
}

/// 質問の回答を許可として記録する（`sai-sci approval record`）。同じ `qid` は 2 回書けない（終了コード 4）。
pub fn record_approval(
    conn: &Connection,
    run_id: i64,
    qid: &str,
    question: &str,
    answer: &str,
    answered_by: &str,
) -> Result<i64> {
    match store::insert_approval(conn, run_id, qid, question, answer, answered_by) {
        Err(RtError::Db(rusqlite::Error::SqliteFailure(e, _))) if e.code == rusqlite::ErrorCode::ConstraintViolation => {
            Err(RtError::Conflict(format!("この質問の承認は記録済みです: {qid}")))
        }
        other => other,
    }
}

/// 利用者として 1 件の op を行う（どの op も使える）。
pub fn apply_user(conn: &Connection, _roots: &PaperRoots, defs: &Defs, input: &Value) -> Result<OpResult> {
    let op = input.get("op").and_then(Value::as_str).unwrap_or("").to_string();
    let tx = conn.unchecked_transaction()?;
    // READ_CARD（runner=user の run）の画面から足す F は、その run の F として残す
    // （`run` は op の項目ではないので、検査に渡す前に外す）
    let run_id = input.get("run").and_then(Value::as_i64).filter(|r| *r > 0);
    let mut input = input.clone();
    if let Some(map) = input.as_object_mut() {
        map.remove("run");
    }
    let cx = Cx { conn: &tx, defs, by: "user".into(), run_id, approval: None, ledger: None };
    let (result, range) = execute(&cx, &input)?;
    tx.commit()?;
    Ok(OpResult { op, result, range })
}

// ---- 検査の部品 -----------------------------------------------------------------------------

/// 項目の値。`null` は無いものとして扱う。
fn get<'a>(op: &'a Value, f: &Field) -> Option<&'a Value> {
    op.get(f.name).filter(|v| !v.is_null())
}

/// 表のとおりの形か（余計な項目・足りない項目・型・字数・文字種）。
fn check_shape(spec: &OpSpec, op: &Value) -> Result<()> {
    let Some(map) = op.as_object() else { return invalid("op は JSON のオブジェクトです") };
    for key in map.keys() {
        if key != "op" && !spec.fields.iter().any(|f| f.name == key) {
            return invalid(format!("{key} は {} の項目ではありません", spec.op));
        }
    }
    for f in spec.fields {
        let Some(v) = get(op, f) else {
            if f.required {
                return invalid(format!("{} が必要です", f.name));
            }
            continue;
        };
        let bad = || RtError::Invalid(format!("{} の型が正しくありません", f.name));
        match f.ty {
            Ty::Id | Ty::SeedOrId => {
                v.as_str().ok_or_else(bad)?;
            }
            Ty::Text(min, max) => {
                let s = v.as_str().ok_or_else(bad)?;
                let n = s.chars().count();
                if n < min {
                    return invalid(format!("{} が短すぎます（{min} 字以上）", f.name));
                }
                if n > max {
                    return invalid(format!("{} は {max} 字までです", f.name));
                }
                if let Some(msg) = textcheck::foreign_message(s) {
                    return invalid(format!("{}: {msg}", f.name));
                }
            }
            Ty::Int => {
                if v.as_i64().is_none_or(|n| n < 1) {
                    return invalid(format!("{} は 1 以上の整数です", f.name));
                }
            }
            Ty::Enum(values) => {
                if !v.as_str().is_some_and(|s| values.contains(&s)) {
                    return invalid(format!("{} は {} のどれかです", f.name, values.join(" / ")));
                }
            }
            Ty::List(min, max) => {
                let items = v.as_array().ok_or_else(bad)?;
                if items.iter().any(|i| !i.is_string()) {
                    return Err(bad());
                }
                if items.len() < min || items.len() > max {
                    return invalid(format!("{} は {min}〜{max} 個です", f.name));
                }
            }
        }
    }
    Ok(())
}

fn text<'a>(op: &'a Value, key: &str) -> &'a str {
    op.get(key).and_then(Value::as_str).unwrap_or("")
}

fn strings(op: &Value, key: &str) -> Vec<String> {
    op.get(key)
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
        .unwrap_or_default()
}

/// 生きた Node を、頭文字を確かめて取る。
fn node(cx: &Cx<'_>, id_text: &str, kinds: &[Kind]) -> Result<NodeRow> {
    let (kind, id) = parse_id(id_text)?;
    if !kinds.contains(&kind) {
        let names: Vec<_> = kinds.iter().map(|k| k.as_str()).collect();
        return invalid(format!("{id_text} は {} の ID ではありません", names.join(" か ")));
    }
    match store::get_node_of(cx.conn, kind, id) {
        Ok(n) if n.removed_at.is_none() => Ok(n),
        Ok(_) | Err(RtError::NotFound(_)) => invalid(format!("{id_text} はありません")),
        Err(e) => Err(e),
    }
}

fn alive_children(cx: &Cx<'_>, id: i64, kind: Kind) -> Result<Vec<NodeRow>> {
    Ok(store::children(cx.conn, id)?.into_iter().filter(|n| n.kind == kind).collect())
}

/// 文の数（「。」「？」「！」と、後ろが空白・行末の `.` `?`）。0 なら 1 文。
fn sentence_count(text: &str) -> usize {
    let chars: Vec<char> = text.chars().collect();
    let n = chars
        .iter()
        .enumerate()
        .filter(|(i, c)| match c {
            '。' | '？' | '！' => true,
            '.' | '?' => chars.get(i + 1).is_none_or(|next| next.is_whitespace()),
            _ => false,
        })
        .count();
    n.max(1)
}

fn paper_exists(conn: &Connection, id: i64) -> Result<bool> {
    Ok(conn.query_row("SELECT 1 FROM papers WHERE id = ?1", params![id], |_| Ok(())).optional()?.is_some())
}

/// 引用を確かめ、論文の番号を返す（C-CITE）。
fn check_cites(conn: &Connection, cites: &[String]) -> Result<Vec<i64>> {
    let mut papers = Vec::new();
    for value in cites {
        let Some(cite) = textcheck::parse_cite(value) else {
            return invalid(format!(
                "引用は #<論文番号> の形（例 #37 §3.2・#37.5.2）で書いてください: {value}"
            ));
        };
        if !paper_exists(conn, cite.paper)? {
            return invalid(format!("論文 {} が Library にありません", cite.paper));
        }
        if let Some(sec) = cite.section {
            let found = sai_paper_core::sections::passage_exists(conn, cite.paper, sec, cite.passage)
                .map_err(|e| RtError::Invalid(format!("塊の確認に失敗しました: {e}")))?;
            if found == Some(false) {
                return invalid(format!("塊 {value} は論文 {} にありません", cite.paper));
            }
        }
        papers.push(cite.paper);
    }
    Ok(papers)
}

/// GUIDE の key が無ければ拒否する（`None` は通す）。
fn check_guide(cx: &Cx<'_>, guide: Option<&str>) -> Result<()> {
    if let Some(key) = guide.filter(|k| cx.defs.guide(k).is_none()) {
        return invalid(format!("知らない GUIDE です: {key}"));
    }
    Ok(())
}

fn new_node<'a>(cx: &'a Cx<'_>, kind: Kind, parent: Option<i64>, text: &'a str) -> NewNode<'a> {
    NewNode {
        kind,
        parent_id: parent,
        guide_key: None,
        text,
        body: "",
        stance: None,
        cites: &[],
        paper_id: None,
        run_id: cx.run_id,
        report_id: None,
        mlflow_ref: None,
        paths: &[],
        created_by: &cx.by,
    }
}

/// 記録を 1 行足す。許可が要る op（rt_log の action は op 名と同じ）のときだけ、許可の印を付ける。
fn log(cx: &Cx<'_>, node_id: i64, action: &str, detail: &str) -> Result<()> {
    let approval = cx.approval.as_ref().filter(|_| needs_approval(action));
    store::insert_log(cx.conn, node_id, action, detail, &cx.by, approval)?;
    Ok(())
}

/// `rt_nodes` の上で `id` が `q` の子孫か。
fn is_under(conn: &Connection, id: i64, q: i64) -> Result<bool> {
    let mut at = store::get_node(conn, id)?.parent_id;
    while let Some(p) = at {
        if p == q {
            return Ok(true);
        }
        at = store::get_node(conn, p)?.parent_id;
    }
    Ok(false)
}

// ---- 検査して書く -------------------------------------------------------------------------

/// op を検査して書く。返すのは `result`（作った・変えた Node の ID）と、add_sprout の範囲。
fn execute(cx: &Cx<'_>, op: &Value) -> Result<(String, Option<Range>)> {
    let name = text(op, "op");
    let Some(spec) = op_spec(name) else { return invalid(format!("知らない op です: {name}")) };
    check_shape(spec, op)?;
    let conn = cx.conn;
    match name {
        "add_q" => {
            let parent = text(op, "parent");
            let guide = op.get("guide").and_then(Value::as_str);
            check_guide(cx, guide)?;
            // `parent: SEED` は top_Q を作る（親は NULL）。
            let parent_id = if parent == super::SEED_ID {
                None
            } else {
                let found = node(cx, parent, &[Kind::Q, Kind::A])?;
                if found.kind == Kind::Q && !alive_children(cx, found.id, Kind::A)?.is_empty() {
                    return invalid(format!("{} には A があるので、child_Q を足せません", fmt_id(found.kind, found.id)));
                }
                Some(found.id)
            };
            let mut n = new_node(cx, Kind::Q, parent_id, text(op, "text"));
            n.guide_key = guide;
            let id = store::insert_node(conn, &n)?;
            log(cx, id, "add_q", "")?;
            Ok((fmt_id(Kind::Q, id), None))
        }
        "insert_q" => {
            let q = node(cx, text(op, "q"), &[Kind::Q])?;
            let qid = fmt_id(Kind::Q, q.id);
            let guide = op.get("guide").and_then(Value::as_str);
            check_guide(cx, guide)?;
            let moving = alive_children(cx, q.id, Kind::A)?;
            if moving.is_empty() {
                return invalid(format!("{qid} に A がないので、child_Q を挟めません"));
            }
            let mut n = new_node(cx, Kind::Q, Some(q.id), text(op, "text"));
            n.guide_key = guide;
            let new_id = store::insert_node(conn, &n)?;
            for a in &moving {
                store::set_parent(conn, a.id, new_id)?;
            }
            log(cx, new_id, "insert_q", &format!("{qid} の A {} 件を移した", moving.len()))?;
            Ok((fmt_id(Kind::Q, new_id), None))
        }
        "move_a" => {
            let from = node(cx, text(op, "from"), &[Kind::Q])?;
            let to = node(cx, text(op, "to"), &[Kind::Q])?;
            let from_id = fmt_id(Kind::Q, from.id);
            if from.id == to.id {
                return invalid("from と to は別の Q にしてください");
            }
            let moving = alive_children(cx, from.id, Kind::A)?;
            if moving.is_empty() {
                return invalid(format!("{from_id} に A がありません"));
            }
            if !alive_children(cx, to.id, Kind::Q)?.is_empty() {
                return invalid(format!("{} には child_Q があるので、A を移せません", fmt_id(Kind::Q, to.id)));
            }
            for a in &moving {
                if is_under(conn, to.id, a.id)? {
                    return invalid(format!("{} は {from_id} の A の下にあるので移せません", fmt_id(Kind::Q, to.id)));
                }
            }
            for a in &moving {
                store::set_parent(conn, a.id, to.id)?;
            }
            log(cx, to.id, "move_a", &format!("{from_id} から A {} 件", moving.len()))?;
            Ok((fmt_id(Kind::Q, to.id), None))
        }
        "copy" => {
            let src = node(cx, text(op, "node"), &[Kind::A])?;
            let to = node(cx, text(op, "to"), &[Kind::Q])?;
            if !alive_children(cx, to.id, Kind::Q)?.is_empty() {
                return invalid(format!("{} には child_Q があるので、A を写せません", text(op, "to")));
            }
            let mut n = new_node(cx, Kind::A, Some(to.id), &src.text);
            n.body = &src.body;
            let copied = store::insert_node(conn, &n)?;
            for f in alive_children(cx, src.id, Kind::F)? {
                let mut nf = new_node(cx, Kind::F, Some(copied), &f.text);
                nf.stance = f.stance.as_deref();
                nf.cites = &f.cites;
                nf.paper_id = f.paper_id;
                nf.report_id = f.report_id.as_deref();
                store::insert_node(conn, &nf)?;
            }
            log(cx, copied, "copy", &format!("{} から", text(op, "node")))?;
            Ok((fmt_id(Kind::A, copied), None))
        }
        "remove" => {
            let n = node(cx, text(op, "node"), ALL_KINDS)?;
            store::remove_subtree(conn, n.id)?;
            log(cx, n.id, "remove", "")?;
            Ok((text(op, "node").to_string(), None))
        }
        "add_a" => {
            let q = node(cx, text(op, "q"), &[Kind::Q])?;
            let qid = fmt_id(Kind::Q, q.id);
            if !alive_children(cx, q.id, Kind::Q)?.is_empty() {
                return invalid(format!("{qid} には child_Q があるので、A を足せません"));
            }
            let t = text(op, "text");
            if alive_children(cx, q.id, Kind::A)?.iter().any(|a| a.text == t) {
                return invalid(format!("{qid} に同じ文の A があります"));
            }
            let mut n = new_node(cx, Kind::A, Some(q.id), t);
            n.body = text(op, "body");
            let id = store::insert_node(conn, &n)?;
            defs::create_qs_for_answer(conn, cx.defs, id, &cx.by, cx.run_id)?;
            Ok((fmt_id(Kind::A, id), None))
        }
        "add_f" => {
            let parent = node(cx, text(op, "parent"), &[Kind::A])?;
            let t = text(op, "text");
            if sentence_count(t) > 3 {
                return invalid("text は 3 文までです");
            }
            let cites = strings(op, "cites");
            // 数字を含む F は、論文の出典（cites）か、自分の REPORT のトークン（report）を根拠にする。
            let report = op.get("report").and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty());
            if cites.is_empty() && report.is_none() && t.chars().any(char::is_numeric) {
                return invalid("数字を含む F には出典（cites）か REPORT のトークン（report）を付けてください");
            }
            let papers = check_cites(conn, &cites)?;
            // agent が見ていない番号は書かない（台帳に無い番号は終了コード 2）。
            if let Some(ledger) = &cx.ledger {
                seen_ids::check_for(&ledger.root, ledger.run_id, &cites).map_err(RtError::Invalid)?;
            }
            let paper = op.get("paper").and_then(Value::as_i64);
            if let Some(p) = paper.filter(|p| !paper_exists(conn, *p).unwrap_or(false)) {
                return invalid(format!("論文 {p} が Library にありません"));
            }
            let mut n = new_node(cx, Kind::F, Some(parent.id), t);
            n.stance = Some(text(op, "stance"));
            n.cites = &cites;
            n.paper_id = paper.or_else(|| papers.first().copied());
            n.report_id = op.get("report").and_then(Value::as_str);
            Ok((fmt_id(Kind::F, store::insert_node(conn, &n)?), None))
        }
        "add_e" => {
            let ptext = text(op, "parent");
            let (pkind, _) = parse_id(ptext)?;
            if !matches!(pkind, Kind::S | Kind::E) {
                return invalid("E は SPROUT（S）か E の下にだけ作れます");
            }
            let parent = node(cx, ptext, &[Kind::S, Kind::E])?;
            let mut n = new_node(cx, Kind::E, Some(parent.id), text(op, "text"));
            n.body = text(op, "body");
            n.mlflow_ref = op.get("mlflow").and_then(Value::as_str).filter(|s| !s.is_empty());
            Ok((fmt_id(Kind::E, store::insert_node(conn, &n)?), None))
        }
        "add_sprout" => {
            let mut paths = Vec::new();
            for a in strings(op, "paths") {
                let n = node(cx, &a, &[Kind::A])?;
                if paths.contains(&n.id) {
                    return invalid(format!("同じ A が 2 回あります: {}", fmt_id(Kind::A, n.id)));
                }
                paths.push(n.id);
            }
            let s = NewNode {
                kind: Kind::S,
                parent_id: None,
                guide_key: None,
                text: text(op, "title"),
                body: text(op, "body"),
                stance: None,
                cites: &[],
                paper_id: None,
                run_id: cx.run_id,
                report_id: None,
                mlflow_ref: None,
                paths: &paths,
                created_by: &cx.by,
            };
            let id = store::insert_node(conn, &s)?;
            let range = sprout_range(conn, &paths)?;
            Ok((fmt_id(Kind::S, id), Some(range)))
        }
        "set_stance" => {
            let f = node(cx, text(op, "node"), &[Kind::F])?;
            let stance = text(op, "stance");
            if f.stance.as_deref() != Some(stance) {
                let before = f.stance.clone().unwrap_or_default();
                store::set_stance(conn, f.id, stance)?;
                log(cx, f.id, "set_stance", &format!("{before}→{stance}"))?;
            }
            Ok((text(op, "node").to_string(), None))
        }
        "set_mlflow" => {
            let e = node(cx, text(op, "node"), &[Kind::E])?;
            let value = text(op, "mlflow");
            let next = (!value.is_empty()).then_some(value);
            if e.mlflow_ref.as_deref() != next {
                store::set_mlflow(conn, e.id, next)?;
                log(cx, e.id, "set_mlflow", value)?;
            }
            Ok((text(op, "node").to_string(), None))
        }
        "star" | "unstar" => {
            let on = name == "star";
            let n = node(cx, text(op, "node"), ALL_KINDS)?;
            if n.starred != on {
                store::set_starred(conn, n.id, on)?;
                log(cx, n.id, name, "")?;
            }
            Ok((text(op, "node").to_string(), None))
        }
        "edit" => {
            let n = node(cx, text(op, "node"), ALL_KINDS)?;
            let (t, b) = (op.get("text").and_then(Value::as_str), op.get("body").and_then(Value::as_str));
            if t.is_none() && b.is_none() {
                return invalid("text か body のどちらかが必要です");
            }
            if let Some(t) = t {
                let max = match n.kind {
                    Kind::Q => 300,
                    Kind::A | Kind::E | Kind::S => 120,
                    Kind::F => 1200,
                };
                if t.chars().count() > max {
                    return invalid(format!("text は {max} 字までです"));
                }
                if n.kind == Kind::F && sentence_count(t) > 3 {
                    return invalid("text は 3 文までです");
                }
            }
            if b.is_some() && !matches!(n.kind, Kind::A | Kind::E | Kind::S) {
                return invalid("body は A と E と SPROUT だけに付けられます");
            }
            store::update_text(conn, n.id, t, b)?;
            let detail = match (t.is_some(), b.is_some()) {
                (true, true) => "text, body",
                (true, false) => "text",
                _ => "body",
            };
            log(cx, n.id, "edit", detail)?;
            Ok((text(op, "node").to_string(), None))
        }
        _ => invalid(format!("知らない op です: {name}")),
    }
}

// ---- テスト -----------------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use sai_paper_core::database::Database;
    use std::path::PathBuf;

    struct Env {
        db: Database,
        roots: PaperRoots,
        defs: Defs,
        sub: i64,
    }

    fn qid(id: i64) -> String {
        fmt_id(Kind::Q, id)
    }

    fn env() -> Env {
        let dir = std::env::temp_dir().join(format!("rt-ops-test-{}-{}", std::process::id(), super::super::now_text().len()));
        let dir: PathBuf = dir.join(format!("{:?}", std::thread::current().id()).replace(['(', ')'], ""));
        std::fs::create_dir_all(&dir).unwrap();
        let defs = defs::load(&dir);
        let db = Database::in_memory().unwrap();
        defs::init_tree(db.connection(), &defs, "problem", "test").unwrap();
        let sub = store::list_nodes(db.connection(), false)
            .unwrap()
            .into_iter()
            .find(|n| n.guide_key.as_deref() == Some("purpose.subgoal"))
            .unwrap()
            .id;
        let blocks = dir.join(".sai/blocks");
        std::fs::create_dir_all(&blocks).unwrap();
        std::fs::write(
            blocks.join("t-all.md"),
            "---\nsci_ops: add_q, insert_q, move_a, copy, remove, add_a, add_f, add_e, add_sprout, set_stance, set_mlflow\n---\n# t\n",
        )
        .unwrap();
        Env { db, roots: PaperRoots { root: dir.clone(), data_dir: dir }, defs, sub }
    }

    impl Env {
        fn conn(&self) -> &Connection {
            self.db.connection()
        }
        /// agent として（run 7・block t-all）batch を流す。
        fn agent(&self, items: &[Value]) -> Result<BatchResult> {
            let actor = Actor { run_id: Some(7), block: Some("t-all".into()) };
            op(self.conn(), &self.roots, &self.defs, &actor, &json!({"ops": items}))
        }
        /// agent として 1 件だけ流し、結果の ID を返す。
        fn agent1(&self, item: Value) -> Result<String> {
            self.agent(&[item]).map(|mut b| b.results.remove(0).result)
        }
        fn user(&self, op: Value) -> Result<OpResult> {
            apply_user(self.conn(), &self.roots, &self.defs, &op)
        }
        /// run 7 の質問に回答を 1 件記録する（許可か、許可しない）。
        fn approve(&self, qid: &str, answer: &str) {
            store::insert_approval(self.conn(), 7, qid, "作ってよいか", answer, "saituki").unwrap();
        }
        /// sub の下に A を足す。
        fn a(&self, text: &str) -> String {
            self.agent1(json!({"op": "add_a", "q": qid(self.sub), "text": text})).unwrap()
        }
        fn count(&self, table: &str) -> i64 {
            self.conn().query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0)).unwrap()
        }
        /// GUIDE の key から Node の id を引く。
        fn by_key(&self, key: &str) -> i64 {
            store::list_nodes(self.conn(), false)
                .unwrap()
                .into_iter()
                .find(|n| n.guide_key.as_deref() == Some(key))
                .unwrap()
                .id
        }
    }

    fn err(r: Result<impl std::fmt::Debug>) -> String {
        match r {
            Err(RtError::Invalid(m)) => m,
            other => panic!("Invalid のはずが {other:?}"),
        }
    }

    fn conflict(r: Result<impl std::fmt::Debug>) -> String {
        match r {
            Err(RtError::Conflict(m)) => m,
            other => panic!("Conflict のはずが {other:?}"),
        }
    }

    #[test]
    fn add_q_needs_approval_for_agent() {
        let e = env();
        // 許可が無い agent の add_q は Conflict（終了コード 4）。
        assert!(conflict(e.agent1(json!({"op": "add_q", "parent": "SEED", "text": "x"}))).contains("許可がありません"));
        // 利用者としては SEED の下に top_Q を作れる。
        let u = e.user(json!({"op": "add_q", "parent": "SEED", "text": "もう一つ"})).unwrap().result;
        let (_, uid) = parse_id(&u).unwrap();
        assert!(store::get_node(e.conn(), uid).unwrap().parent_id.is_none());
        // 知らない ID は弾く。
        assert!(err(e.user(json!({"op": "add_q", "parent": "NOPE", "text": "x"}))).contains("ID"));
    }

    #[test]
    fn add_a_creates_per_answer_qs() {
        let e = env();
        let a = e.agent1(json!({"op": "add_a", "q": qid(e.sub), "text": "予後予測"})).unwrap();
        let (_, aid) = parse_id(&a).unwrap();
        let kids = store::children(e.conn(), aid).unwrap();
        assert!(!kids.is_empty() && kids.iter().all(|k| k.kind == Kind::Q));
        assert_eq!(store::get_node(e.conn(), aid).unwrap().created_by, "run:7");
        assert!(err(e.agent1(json!({"op": "add_a", "q": qid(e.sub), "text": "予後予測"}))).contains("同じ文"));
        // A のある Q に child_Q は足せず、child_Q のある Q に A は足せない。
        assert!(err(e.user(json!({"op": "add_q", "parent": qid(e.sub), "text": "子"}))).contains("A があるので"));
        let bg = e.by_key("background");
        assert!(err(e.agent1(json!({"op": "add_a", "q": qid(bg), "text": "x"}))).contains("child_Q があるので"));
    }

    #[test]
    fn add_f_rules() {
        let e = env();
        let a = e.a("A");
        let f = |t: &str, cites: Value| e.agent1(json!({"op": "add_f", "parent": a, "text": t, "stance": "support", "cites": cites}));
        assert!(err(f("精度は 85 %", json!([]))).contains("数字を含む F には出典"));
        assert!(err(f("한 国", json!([]))).contains("text: ハングル"));
        assert!(err(f("一。二。三。四。", json!([]))).contains("3 文"));
        assert!(f("数字なし", json!([])).is_ok());
        // stance が無いと拒否する。
        assert!(err(e.agent1(json!({"op": "add_f", "parent": a, "text": "x"}))).contains("stance が必要です"));
        // 親が E だと拒否する（A の ID でない）。
        let s = e.agent1(json!({"op": "add_sprout", "title": "種", "paths": [a]})).unwrap();
        let ef = e.agent1(json!({"op": "add_e", "parent": s, "text": "実験"})).unwrap();
        assert!(err(e.agent1(json!({"op": "add_f", "parent": ef, "text": "x", "stance": "deny"}))).contains("A の ID"));
        // REPORT のトークンがあれば、数字を含んでいても通る。空白だけの report は出典にならない。
        let fr = |t: &str, report: &str| e.agent1(json!({"op": "add_f", "parent": a, "text": t, "stance": "deny", "report": report}));
        assert!(fr("H1 の効果量は g=0.47", "AbCdEf12").is_ok());
        assert!(fr("精度は 85 %", "  ").is_err());
        assert!(err(f("x", json!(["37"]))).contains("#<論文番号>"));
        assert!(err(f("x", json!(["#37"]))).contains("論文 37 が Library にありません"));
        e.conn().execute("INSERT INTO papers(id, title) VALUES (37, 't')", []).unwrap();
        // 節の表が無い論文は塊 ID を確かめずに通る。
        let r = f("精度は 85 %", json!(["#37.5.2"])).unwrap();
        let (_, id) = parse_id(&r).unwrap();
        assert_eq!(store::get_node(e.conn(), id).unwrap().paper_id, Some(37));
        // 表があり塊が無ければ弾く。
        e.conn().execute_batch(
            "INSERT INTO paper_section_builds VALUES (37, 'x', 'sha', 'now', NULL);
             INSERT INTO paper_sections VALUES (37, 5, 'h', NULL, 1, 'body', 1, 1, 1, 'l', NULL);
             INSERT INTO paper_passages VALUES (37, 5, 2, 't', 1, 0, 1);",
        ).unwrap();
        assert!(f("精度は 85 %", json!(["#37.5.2"])).is_ok());
        assert!(err(f("精度は 85 %", json!(["#37.5.3"]))).contains("塊 #37.5.3 は論文 37 にありません"));
    }

    #[test]
    fn op_add_f_checks_ledger() {
        let e = env();
        e.conn().execute("INSERT INTO papers(id, title) VALUES (37, 't')", []).unwrap();
        let a = e.a("A");
        // 台帳（run 9）には #37.5.2 だけがある。
        let ledger_dir = e.roots.root.join(".sai/seen_ids");
        std::fs::create_dir_all(&ledger_dir).unwrap();
        std::fs::write(ledger_dir.join("9"), "#37.5.2\n").unwrap();
        let ledger = || Some(Ledger { root: e.roots.root.clone(), run_id: 9 });
        let actor = Actor { run_id: Some(7), block: Some("t-all".into()) };
        let f = |cite: &str| {
            let batch = json!({"ops": [{"op": "add_f", "parent": a, "text": "精度は 85 %", "stance": "support", "cites": [cite]}]});
            op_in(e.conn(), &e.roots, &e.defs, &actor, &batch, ledger())
        };
        assert!(f("#37.5.2").is_ok());
        assert!(f("#37").is_ok(), "論文は `#37.` で始まる行があれば通す");
        let m = err(f("#37.5.3"));
        assert!(m.starts_with("1 件目（add_f）"), "{m}");
        assert!(m.contains("台帳に無い番号です"), "{m}");
        // 台帳が無い（環境変数が無い）ときは照合しない。
        let no_ledger = |cite: &str| {
            let batch = json!({"ops": [{"op": "add_f", "parent": a, "text": "精度は 85 %", "stance": "support", "cites": [cite]}]});
            op_in(e.conn(), &e.roots, &e.defs, &actor, &batch, None)
        };
        assert!(no_ledger("#37.5.3").is_ok());
        // 利用者の apply は台帳を見ない。
        let user = e.user(json!({"op": "add_f", "parent": a, "text": "精度は 85 %", "stance": "support", "cites": ["#37.5.3"]}));
        assert!(user.is_ok(), "{user:?}");
    }

    #[test]
    fn user_add_f_keeps_run() {
        let e = env();
        let a = e.a("A");
        let f = e.user(json!({"op": "add_f", "parent": a, "text": "読んで分かった", "stance": "support", "run": 31})).unwrap().result;
        let run: Option<i64> = e
            .conn()
            .query_row("SELECT run_id FROM rt_nodes WHERE kind = 'F'", [], |r| r.get(0))
            .unwrap();
        assert!(f.starts_with('F'));
        assert_eq!(run, Some(31));
    }

    #[test]
    fn add_e_needs_sprout_and_set_mlflow_needs_e() {
        let e = env();
        let a = e.a("A");
        // A の下には作れない。
        assert!(err(e.agent1(json!({"op": "add_e", "parent": a, "text": "実験"}))).contains("SPROUT"));
        // SPROUT の下には作れる。E の下にも作れる。
        let s = e.agent1(json!({"op": "add_sprout", "title": "種", "paths": [a]})).unwrap();
        let e1 = e.agent1(json!({"op": "add_e", "parent": s, "text": "実験", "mlflow": "exp/run"})).unwrap();
        assert!(e1.starts_with('E'));
        let e2 = e.agent1(json!({"op": "add_e", "parent": e1, "text": "実行 1"})).unwrap();
        e.agent1(json!({"op": "set_mlflow", "node": e2, "mlflow": "exp/2"})).unwrap();
        let (_, e2id) = parse_id(&e2).unwrap();
        assert_eq!(store::get_node(e.conn(), e2id).unwrap().mlflow_ref.as_deref(), Some("exp/2"));
        assert!(err(e.agent1(json!({"op": "set_mlflow", "node": a, "mlflow": "x"}))).contains("E の ID"));
    }

    #[test]
    fn set_stance_changes_only_f() {
        let e = env();
        let a = e.a("A");
        let f = e.agent1(json!({"op": "add_f", "parent": a, "text": "根拠", "stance": "support"})).unwrap();
        e.agent1(json!({"op": "set_stance", "node": f, "stance": "deny"})).unwrap();
        let (_, fid) = parse_id(&f).unwrap();
        assert_eq!(store::get_node(e.conn(), fid).unwrap().stance.as_deref(), Some("deny"));
        assert_eq!(store::list_log(e.conn(), Some(1)).unwrap()[0].detail, "support→deny");
        assert!(err(e.agent1(json!({"op": "set_stance", "node": a, "stance": "deny"}))).contains("F の ID"));
    }

    #[test]
    fn add_sprout_paths_are_answers_and_report_range() {
        let e = env();
        let a = e.a("A");
        let (_, aid) = parse_id(&a).unwrap();
        let child_q = store::children(e.conn(), aid).unwrap().into_iter().find(|n| n.kind == Kind::Q).unwrap().id;
        let b = e.agent1(json!({"op": "add_a", "q": qid(child_q), "text": "B"})).unwrap();
        let (_, bid) = parse_id(&b).unwrap();
        // 同じ A は 2 回書けない。Q は断る。
        assert!(err(e.agent1(json!({"op": "add_sprout", "title": "種", "paths": [b, b]}))).contains("同じ A"));
        assert!(err(e.agent1(json!({"op": "add_sprout", "title": "種", "paths": ["Q1"]}))).contains("A の ID"));
        assert!(err(e.agent1(json!({"op": "add_sprout", "title": "種", "paths": []}))).contains("1〜20 個"));
        // 通り、範囲（A は 2 つ・F は 0）が返る。
        let r = e.agent(&[json!({"op": "add_sprout", "title": "種", "body": "本文", "paths": [a, b]})]).unwrap();
        let (skind, sid) = parse_id(&r.results[0].result).unwrap();
        assert_eq!(skind, Kind::S);
        let node = store::get_node(e.conn(), sid).unwrap();
        assert_eq!(node.paths, vec![aid, bid]);
        assert_eq!(node.body, "本文");
        let range = r.results[0].range.clone().expect("add_sprout は range を返す");
        assert_eq!((range.a, range.f_support, range.f_deny, range.f_issue), (2, 0, 0, 0));
    }

    /// 許可が要る op は、承認が無ければ入力を見る前に Conflict（終了コード 4）。
    #[test]
    fn approval_ops_need_approval_for_agent() {
        let e = env();
        let q = qid(e.sub);
        for item in [
            json!({"op": "insert_q", "q": q, "text": "x"}),
            json!({"op": "move_a", "from": q, "to": q}),
            json!({"op": "copy", "node": q, "to": q}),
            json!({"op": "remove", "node": q}),
        ] {
            assert!(conflict(e.agent1(item)).contains("許可がありません"));
        }
        assert!(needs_approval("remove") && !needs_approval("add_a"));
    }

    #[test]
    fn op_needs_approval_and_consumes_one() {
        let e = env();
        let a = e.a("消す A");
        let batch = || vec![json!({"op": "add_q", "parent": "SEED", "text": "根"}), json!({"op": "remove", "node": a})];
        // 許可が無い・「許可しない」だけ、は Conflict。
        assert!(conflict(e.agent(&batch())).contains("許可がありません"));
        e.approve("q1", "許可しない");
        assert!(conflict(e.agent(&batch())).contains("許可がありません"));
        // run_id が無いときも Conflict。
        let no_run = Actor { run_id: None, block: Some("t-all".into()) };
        assert!(conflict(op(e.conn(), &e.roots, &e.defs, &no_run, &json!({"ops": batch()}))).contains("run_id"));
        // 「許可する」1 件で、add_q と remove を含む 1 回の op が通る。
        e.approve("q2", " 許可する ");
        let done = e.agent(&batch()).unwrap();
        assert_eq!(done.approval, Some(json!({"qid": "q2", "answered_by": "saituki"})));
        assert_eq!(done.results.len(), 2);
        // 使い切ったので、2 回目は通らない。
        assert!(conflict(e.agent(&batch())).contains("許可がありません"));
        // rt_log の add_q と remove の行に承認の印が入り、add_a の行には入らない。
        let logs = store::list_log(e.conn(), None).unwrap();
        for action in ["add_q", "remove"] {
            let row = logs.iter().find(|l| l.action == action).unwrap();
            assert_eq!((row.approved_by.as_deref(), row.qid.as_deref()), (Some("saituki"), Some("q2")));
            assert_eq!(row.question.as_deref(), Some("作ってよいか"));
        }
        assert!(logs.iter().filter(|l| l.action == "add_a").all(|l| l.approved_by.is_none()));
    }

    #[test]
    fn insert_q_moves_all_answers_in_order() {
        let e = env();
        let a1 = e.a("一つ目");
        let a2 = e.a("二つ目");
        e.approve("q3", "許可する");
        let r = e.agent(&[json!({"op": "insert_q", "q": qid(e.sub), "text": "条件を分ける"})]).unwrap();
        let (_, nq) = parse_id(&r.results[0].result).unwrap();
        let kids: Vec<i64> = store::children(e.conn(), nq).unwrap().iter().map(|n| n.id).collect();
        assert_eq!(kids, vec![parse_id(&a1).unwrap().1, parse_id(&a2).unwrap().1]);
        assert_eq!(store::children(e.conn(), e.sub).unwrap().iter().map(|n| n.id).collect::<Vec<_>>(), vec![nq]);
    }

    #[test]
    fn move_a_rejects_cycle_and_child_q() {
        let e = env();
        let a = e.a("A");
        // 許可は 1 件あれば足りる（失敗した op は許可を消費しない）。
        e.approve("q4", "許可する");
        let (_, aid) = parse_id(&a).unwrap();
        let child_q = store::children(e.conn(), aid).unwrap().into_iter().find(|n| n.kind == Kind::Q).unwrap();
        // A の下の Q へ、その A の親 Q から移すのは輪になる。
        let cq = fmt_id(Kind::Q, child_q.id);
        assert!(err(e.agent1(json!({"op": "move_a", "from": qid(e.sub), "to": cq}))).contains("の A の下にあるので移せません"));
        // child_Q を持つ Q へは移せない。
        assert!(err(e.agent1(json!({"op": "move_a", "from": qid(e.sub), "to": qid(e.by_key("background"))}))).contains("child_Q があるので"));
        assert!(err(e.agent1(json!({"op": "move_a", "from": qid(e.sub), "to": qid(e.sub)}))).contains("別の Q"));
    }

    #[test]
    fn copy_keeps_stance_and_only_f() {
        let e = env();
        let a = e.a("A");
        e.agent1(json!({"op": "add_f", "parent": a, "text": "根拠", "stance": "deny"})).unwrap();
        let other = e.user(json!({"op": "add_q", "parent": "SEED", "text": "別の根"})).unwrap().result;
        let c = e.user(json!({"op": "copy", "node": a, "to": other})).unwrap().result;
        let (_, cid) = parse_id(&c).unwrap();
        // 写したのは F だけ（A の下の Q は写さない）。stance も同じ。
        let kids = store::children(e.conn(), cid).unwrap();
        assert_eq!(kids.len(), 1);
        assert_eq!((kids[0].kind, kids[0].stance.as_deref()), (Kind::F, Some("deny")));
    }

    #[test]
    fn set_mlflow_logs_new_value() {
        let e = env();
        let a = e.a("A");
        let s = e.agent1(json!({"op": "add_sprout", "title": "種", "paths": [a]})).unwrap();
        let ex = e.agent1(json!({"op": "add_e", "parent": s, "text": "実験", "mlflow": "exp/1"})).unwrap();
        e.agent1(json!({"op": "set_mlflow", "node": ex, "mlflow": ""})).unwrap();
        let (_, eid) = parse_id(&ex).unwrap();
        assert_eq!(store::get_node(e.conn(), eid).unwrap().mlflow_ref, None);
        let last = &store::list_log(e.conn(), Some(1)).unwrap()[0];
        assert_eq!((last.action.as_str(), last.detail.as_str()), ("set_mlflow", ""));
    }

    /// 範囲は A の祖先の Q と A、A の子の F だけを数える（A の子の Q 以下・祖先の A の F は入れない）。
    #[test]
    fn sprout_range_counts_ancestors_and_own_f() {
        let e = env();
        // Q1 ─ A2 ─ Q3 ─ A4 ─ {F(support), F(deny)}、A2 ─ F(issue)
        let q1 = e.user(json!({"op": "add_q", "parent": "SEED", "text": "Q1"})).unwrap().result;
        let a2 = e.agent1(json!({"op": "add_a", "q": q1, "text": "A2"})).unwrap();
        let q3 = e.user(json!({"op": "add_q", "parent": a2, "text": "Q3"})).unwrap().result;
        let a4 = e.agent1(json!({"op": "add_a", "q": q3, "text": "A4"})).unwrap();
        e.agent1(json!({"op": "add_f", "parent": a4, "text": "支持", "stance": "support"})).unwrap();
        e.agent1(json!({"op": "add_f", "parent": a4, "text": "反対", "stance": "deny"})).unwrap();
        e.agent1(json!({"op": "add_f", "parent": a2, "text": "懸念", "stance": "issue"})).unwrap();
        let r = e.agent(&[json!({"op": "add_sprout", "title": "種", "paths": [a4]})]).unwrap();
        assert_eq!(r.results[0].range, Some(Range { q: 2, a: 2, f_support: 1, f_deny: 1, f_issue: 0 }));
    }

    #[test]
    fn one_bad_batch_saves_nothing() {
        let e = env();
        let q = qid(e.sub);
        let n = e.count("rt_nodes");
        let m = err(e.agent(&[
            json!({"op": "add_a", "q": q, "text": "A"}),
            json!({"op": "add_a", "q": q, "text": "B"}),
            json!({"op": "add_a", "q": q, "text": "한"}),
        ]));
        assert!(m.starts_with("3 件目（add_a）: text: ハングル"), "{m}");
        assert_eq!(e.count("rt_nodes"), n);
    }

    #[test]
    fn star_is_user_only() {
        let e = env();
        let a = e.a("A");
        assert!(err(e.agent1(json!({"op": "star", "node": a}))).contains("USER_OP"));
        // sci_ops に書いてあっても弾く。
        let blocks = e.roots.root.join(".sai/blocks");
        std::fs::create_dir_all(&blocks).unwrap();
        std::fs::write(blocks.join("t-star.md"), "---\nsci_ops: add_a, star\n---\n# t\n").unwrap();
        let actor = Actor { run_id: None, block: Some("t-star".into()) };
        assert_eq!(allowed_ops(&e.roots, "t-star"), ["add_a", "star"]);
        let m = err(op(e.conn(), &e.roots, &e.defs, &actor, &json!({"ops": [{"op": "star", "node": a}]})));
        assert!(m.starts_with("1 件目（star）"));
        e.user(json!({"op": "star", "node": a})).unwrap();
        let log = store::list_log(e.conn(), None).unwrap();
        assert_eq!(log.iter().filter(|l| l.action == "star" && l.actor == "user").count(), 1);
        e.user(json!({"op": "star", "node": a})).unwrap();
        assert_eq!(store::list_log(e.conn(), None).unwrap().len(), 1);
    }

    #[test]
    fn user_structure_ops_and_edit() {
        let e = env();
        let a = e.a("A");
        e.user(json!({"op": "star", "node": a})).unwrap();
        e.user(json!({"op": "unstar", "node": a})).unwrap();
        e.user(json!({"op": "edit", "node": a, "text": "B", "body": "本文"})).unwrap();
        assert!(err(e.user(json!({"op": "edit", "node": a}))).contains("text か body"));
        // insert_q は A を全部、新しい child_Q の下へ移す。
        let nq = e.user(json!({"op": "insert_q", "q": qid(e.sub), "text": "挟む問い"})).unwrap().result;
        let (_, nqid) = parse_id(&nq).unwrap();
        assert_eq!(store::children(e.conn(), e.sub).unwrap().iter().map(|n| n.id).collect::<Vec<_>>(), vec![nqid]);
        let (_, aid) = parse_id(&a).unwrap();
        assert_eq!(store::get_node(e.conn(), aid).unwrap().parent_id, Some(nqid));
        // move_a は A の無い Q からは動かせない。別の根へ移す。
        let other = e.user(json!({"op": "add_q", "parent": "SEED", "text": "別の根"})).unwrap().result;
        assert!(err(e.user(json!({"op": "move_a", "from": qid(e.sub), "to": other}))).contains("A がありません"));
        e.user(json!({"op": "move_a", "from": nq, "to": other})).unwrap();
        assert_eq!(store::get_node(e.conn(), aid).unwrap().parent_id, Some(parse_id(&other).unwrap().1));
        // copy は A と F を写す。
        let c = e.user(json!({"op": "copy", "node": a, "to": other})).unwrap().result;
        assert!(c.starts_with('A'));
        let actions: Vec<String> = store::list_log(e.conn(), None).unwrap().into_iter().map(|l| l.action).collect();
        for want in ["insert_q", "move_a", "copy", "edit", "unstar", "star"] {
            assert!(actions.iter().any(|x| x == want), "{want}");
        }
        assert!(store::list_log(e.conn(), None).unwrap().iter().all(|l| l.approved_by.is_none()));
        // top_Q も消せる。
        let bg = e.by_key("background");
        e.user(json!({"op": "remove", "node": qid(bg)})).unwrap();
        assert!(store::get_node(e.conn(), bg).unwrap().removed_at.is_some());
    }

    #[test]
    fn shape_errors() {
        let e = env();
        let q = qid(e.sub);
        assert!(err(e.agent1(json!({"op": "add_a", "q": q}))).contains("text が必要"));
        assert!(err(e.agent1(json!({"op": "add_a", "q": q, "text": "x", "zzz": 1}))).contains("zzz"));
        assert!(err(e.agent1(json!({"op": "nope"}))).contains("知らない op"));
        assert!(err(e.agent1(json!({"op": "add_a", "q": "A1", "text": "x"}))).contains("Q の ID"));
        assert!(err(e.user(json!({"op": "add_q", "parent": q, "text": "x", "guide": "zz"}))).contains("知らない GUIDE"));
        let a = e.a("A");
        assert!(err(e.agent1(json!({"op": "add_f", "parent": a, "text": "x", "stance": "foo"}))).contains("support / deny / issue"));
    }

    #[test]
    fn reason_is_not_a_field() {
        let e = env();
        let q = qid(e.sub);
        // 理由（reason）を付けた op は、知らない項目として弾かれる。
        assert!(err(e.agent1(json!({"op": "add_a", "q": q, "text": "x", "reason": "x"}))).contains("reason"));
        assert!(err(e.user(json!({"op": "copy", "node": q, "to": q, "reason": "x"}))).contains("reason"));
    }

    #[test]
    fn schema_has_no_user_ops() {
        let s = op_schema();
        let ops: Vec<_> = s["properties"]["ops"]["items"]["oneOf"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v["properties"]["op"]["const"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(
            ops,
            ["add_q", "insert_q", "move_a", "copy", "remove", "add_a", "add_f", "add_e", "add_sprout", "set_stance", "set_mlflow"]
        );
        assert!(is_user_only_op("star") && !is_user_only_op("add_a") && is_known_op("edit"));
    }
}
