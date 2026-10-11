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
use super::defs::Defs;
use super::seedq;
use super::store::{self, ApprovalRow, NewNode, NodeRow};
use super::{Kind, Result, RtError, fmt_id, fmt_sa, fmt_sq, parse_id, parse_sa, parse_sq};
use sai_paper_core::paths::PaperRoots;
use sai_paper_core::{seen_ids, textcheck};
use std::path::PathBuf;

pub(crate) const MAX_OPS: usize = 20;
const ALL_KINDS: &[Kind] = &[Kind::Q, Kind::A, Kind::F, Kind::E, Kind::S, Kind::D];
const STANCES: &[&str] = &["support", "deny", "issue"];
/// 問いの軸（6 軸の key。空文字は軸を外す）。
const AXIS_OR_EMPTY: &[&str] = &["task", "target", "data", "method", "output", "metric", ""];
/// 文の数の決まり（`sentence_count` と同じ規則。エラー文に出す）。
const SENTENCE_RULE: &str = "文の区切りは「。」「？」「！」と、後ろが空白か行末の「.」「?」";

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
    /// 文（字数の範囲。文字種も検査する）。
    Text(usize, usize),
    /// 整数。
    Int,
    /// 決まった語のどれか。
    Enum(&'static [&'static str]),
    /// 文字列の配列（個数の範囲）。
    List(usize, usize),
    /// 問いの ID（`SQ3`）。
    Sq,
    /// 問いへの回答の ID（`SA7`）。
    Sa,
    /// 問いへの回答の配列（個数の範囲）。各項目は `{q, a}` か `{q, text}`。
    Answers(usize, usize),
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
        about: "Q・A の下に child_Q を足す（軸の Q は sai-sci init が作る）",
        fields: &[req("parent", Ty::Id), req("text", Ty::Text(1, 300)), opt("guide", Ty::Text(1, 80))],
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
        op: "move_node",
        risk: Risk::Approval,
        about: "A を、その下の F・child_Q ごと、ID を保って別の Q の下へ移す。S の paths は壊れない",
        fields: &[req("node", Ty::Id), req("to", Ty::Id)],
    },
    OpSpec {
        op: "regroup",
        risk: Risk::Approval,
        about: "Q の下に中間の A と細分の Q を作り、詳しい A をその下へまとめて移す。ID を保つ",
        fields: &[
            req("q", Ty::Id),
            req("text", Ty::Text(1, 120)),
            opt("body", Ty::Text(0, 2000)),
            req("child_q", Ty::Text(1, 300)),
            opt("child_q_body", Ty::Text(0, 2000)),
            req("members", Ty::List(1, 20)),
        ],
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
        about: "A の下に F（根拠）を足す。text は 1〜3 文。立場（stance）が要る。数字を含む文には出典（cites）か REPORT のトークン（report）が要る",
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
        about: "SPROUT か E の下に E（実験）を足す。SPROUT は完成しているときだけ",
        fields: &[
            req("parent", Ty::Id),
            req("text", Ty::Text(1, 120)),
            opt("body", Ty::Text(0, 2000)),
            opt("mlflow", Ty::Text(0, 200)),
            opt("dataset", Ty::Text(0, 200)),
        ],
    },
    OpSpec {
        op: "add_seed",
        risk: Risk::Free,
        about: "seed（D）を足す（研究の背景・制約の文）。問いは雛形の 7 問が最初から入る（消せる）",
        fields: &[req("title", Ty::Text(1, 120)), opt("body", Ty::Text(0, 2000))],
    },
    OpSpec {
        op: "add_seed_q",
        risk: Risk::Approval,
        about: "seed（D）に問いを足す。未完成の SPROUT にだけ効く（完成した SPROUT は凍結）",
        fields: &[req("seed", Ty::Id), req("text", Ty::Text(1, 300)), opt("axis", Ty::Enum(AXIS_OR_EMPTY))],
    },
    OpSpec {
        op: "edit_seed_q",
        risk: Risk::Approval,
        about: "seed の問いの文か軸を変える（軸は空文字で外す）。text か axis のどちらかが要る",
        fields: &[req("q", Ty::Sq), opt("text", Ty::Text(1, 300)), opt("axis", Ty::Enum(AXIS_OR_EMPTY))],
    },
    OpSpec {
        op: "remove_seed_q",
        risk: Risk::Approval,
        about: "seed の問いを消す（回答は残す）",
        fields: &[req("q", Ty::Sq)],
    },
    OpSpec {
        op: "add_answer",
        risk: Risk::Free,
        about: "問いに回答する。to が seed（D）なら固定の回答、SPROUT（S）なら新規の回答。a（A の ID）か text（文）のどちらか一方",
        fields: &[
            req("to", Ty::Id),
            req("q", Ty::Sq),
            opt("a", Ty::Id),
            opt("text", Ty::Text(1, 300)),
        ],
    },
    OpSpec {
        op: "edit_answer",
        risk: Risk::Free,
        about: "テキストの回答の文を変える（path の回答は remove_answer と add_answer で直す）",
        fields: &[req("answer", Ty::Sa), req("text", Ty::Text(1, 300))],
    },
    OpSpec {
        op: "remove_answer",
        risk: Risk::Approval,
        about: "回答を消す（問いは残す）",
        fields: &[req("answer", Ty::Sa)],
    },
    OpSpec {
        op: "add_sprout",
        risk: Risk::Free,
        about: "seed（D）の下に SPROUT（S）を作る。answers（{q, a} か {q, text} の配列）で問いに答える。全部答えると完成",
        fields: &[
            req("seed", Ty::Id),
            req("title", Ty::Text(1, 120)),
            opt("body", Ty::Text(0, 2000)),
            opt("answers", Ty::Answers(0, 40)),
        ],
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
    OpSpec {
        op: "set_dataset",
        risk: Risk::Free,
        about: "E のデータセットの参照（sai-ts の ts:<name>@<sha12> など）を変える（空文字で外す）",
        fields: &[req("node", Ty::Id), req("dataset", Ty::Text(0, 200))],
    },
    OpSpec { op: "star", risk: Risk::UserOnly, about: "Node に ★ を付ける（目印）", fields: &[req("node", Ty::Id)] },
    OpSpec { op: "unstar", risk: Risk::UserOnly, about: "Node の ★ を外す", fields: &[req("node", Ty::Id)] },
    OpSpec {
        op: "edit",
        risk: Risk::UserOnly,
        about: "Node の文・本文を書き換える。F の text は 3 文まで。seed（D）の本文は自由な文",
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
        Ty::Id => json!({"type": "string", "pattern": "^[QAFESD][1-9][0-9]{0,8}$"}),
        Ty::Text(min, max) => json!({"type": "string", "minLength": min, "maxLength": max}),
        Ty::Int => json!({"type": "integer"}),
        Ty::Enum(values) => json!({"type": "string", "enum": values}),
        Ty::List(min, max) => json!({"type": "array", "items": {"type": "string"}, "minItems": min, "maxItems": max}),
        Ty::Sq => json!({"type": "string", "pattern": "^SQ[1-9][0-9]{0,8}$"}),
        Ty::Sa => json!({"type": "string", "pattern": "^SA[1-9][0-9]{0,8}$"}),
        Ty::Answers(min, max) => json!({"type": "array", "minItems": min, "maxItems": max, "items": {
            "type": "object", "properties": {
                "q": {"type": "string", "pattern": "^SQ[1-9][0-9]{0,8}$"},
                "a": {"type": "string", "pattern": "^A[1-9][0-9]{0,8}$"},
                "text": {"type": "string", "minLength": 1, "maxLength": 300}},
             "required": ["q"], "additionalProperties": false}}),
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
    /// regroup で新しく作った Node（A と Q の順。regroup のときだけ）。
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub created: Vec<String>,
}

/// `sai-sci op` の出力。承認を使ったときだけ `approval` が入る。
#[derive(Debug, Clone, Serialize)]
pub struct BatchResult {
    pub results: Vec<OpResult>,
    pub approval: Option<Value>,
}

pub(super) fn invalid<T>(message: impl Into<String>) -> Result<T> {
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
    seedq::upgrade(conn)?;
    let Some(items) = input.get("ops").and_then(Value::as_array) else {
        return invalid("ops（配列）が必要です");
    };
    if items.is_empty() || items.len() > MAX_OPS {
        let n = items.len();
        return invalid(format!("ops は 1〜{MAX_OPS} 件です（今 {n} 件。{MAX_OPS} 件ずつに分けて出してください）"));
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
        let (result, range, created) = execute(&cx, item).map_err(at)?;
        results.push(OpResult { op: name.to_string(), result, range, created });
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
        .ok_or_else(|| {
            RtError::Conflict(
                "許可がありません: 先に質問ツールで「許可する」を選んでもらってください（この run に、まだ使っていない「許可する」の記録がありません。質問の答えの記録に失敗した可能性もあります。同じ質問をもう一度出してください）".into(),
            )
        })
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

/// 利用者として 1 件の op を行う（どの op も使える）。`by` は記録の actor（`user` か `run:<id>`）。
pub fn apply_user(conn: &Connection, _roots: &PaperRoots, defs: &Defs, input: &Value, by: &str) -> Result<OpResult> {
    seedq::upgrade(conn)?;
    let op = input.get("op").and_then(Value::as_str).unwrap_or("").to_string();
    let tx = conn.unchecked_transaction()?;
    // READ_CARD（runner=user の run）の画面から足す F は、その run の F として残す
    // （`run` は op の項目ではないので、検査に渡す前に外す）
    let run_id = input.get("run").and_then(Value::as_i64).filter(|r| *r > 0);
    let mut input = input.clone();
    if let Some(map) = input.as_object_mut() {
        map.remove("run");
    }
    let cx = Cx { conn: &tx, defs, by: by.into(), run_id, approval: None, ledger: None };
    let (result, range, created) = execute(&cx, &input)?;
    tx.commit()?;
    Ok(OpResult { op, result, range, created })
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
            Ty::Id => {
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
            Ty::Sq | Ty::Sa => {
                v.as_str().ok_or_else(bad)?;
            }
            Ty::Answers(min, max) => {
                let items = v.as_array().ok_or_else(bad)?;
                if items.len() < min || items.len() > max {
                    return invalid(format!("{} は {min}〜{max} 個です", f.name));
                }
                for item in items {
                    let map = item.as_object().ok_or_else(bad)?;
                    if let Some(key) = map.keys().find(|k| !["q", "a", "text"].contains(&k.as_str())) {
                        return invalid(format!("answers の項目に {key} は書けません（書けるのは q・a・text）"));
                    }
                    if !map.get("q").is_some_and(Value::is_string) {
                        return invalid("answers の項目には q（問いの ID）が要ります");
                    }
                    if ["a", "text"].iter().any(|k| map.get(*k).is_some_and(|x| !x.is_string())) {
                        return Err(bad());
                    }
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
        dataset_ref: None,
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
pub(super) fn is_under(conn: &Connection, id: i64, q: i64) -> Result<bool> {
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

/// op を検査して書く。返すのは `result`（作った・変えた Node の ID）と、add_sprout の範囲、regroup で作った Node。
fn execute(cx: &Cx<'_>, op: &Value) -> Result<(String, Option<Range>, Vec<String>)> {
    let name = text(op, "op");
    let Some(spec) = op_spec(name) else { return invalid(format!("知らない op です: {name}")) };
    if name == "add_sprout" && op.get("paths").is_some() {
        return invalid("paths は廃止しました。answers を使ってください（例: {\"q\": \"SQ3\", \"a\": \"A12\"}）");
    }
    check_shape(spec, op)?;
    if name == "regroup" {
        let (parent, child) = regroup(cx, op)?;
        let created = vec![fmt_id(Kind::A, parent), fmt_id(Kind::Q, child)];
        return Ok((fmt_id(Kind::A, parent), None, created));
    }
    let (result, range) = execute_plain(cx, name, op)?;
    Ok((result, range, Vec::new()))
}

/// 形の検査のあと、regroup 以外の op を検査して書く（[`execute`] から呼ぶ）。
fn execute_plain(cx: &Cx<'_>, name: &str, op: &Value) -> Result<(String, Option<Range>)> {
    let conn = cx.conn;
    match name {
        "add_q" => {
            let parent = text(op, "parent");
            let guide = op.get("guide").and_then(Value::as_str);
            check_guide(cx, guide)?;
            // 軸の Q（親の無い Q）は sai-sci init だけが作る。
            if parent.is_empty() || super::is_retired_seed(parent) {
                return invalid("parent は Q か A の ID です（軸の Q は sai-sci init が作ります）");
            }
            let found = node(cx, parent, &[Kind::Q, Kind::A])?;
            if found.kind == Kind::Q && !alive_children(cx, found.id, Kind::A)?.is_empty() {
                return invalid(format!("{} には A があるので、child_Q を足せません", fmt_id(found.kind, found.id)));
            }
            let parent_id = Some(found.id);
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
            let ids = moving.iter().map(|a| fmt_id(Kind::A, a.id)).collect::<Vec<_>>().join(", ");
            log(cx, new_id, "insert_q", &format!("{qid} の A {} 件を移した（{ids}）", moving.len()))?;
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
            let ids = moving.iter().map(|a| fmt_id(Kind::A, a.id)).collect::<Vec<_>>().join(", ");
            log(cx, to.id, "move_a", &format!("{from_id} から A {} 件（{ids}）", moving.len()))?;
            Ok((fmt_id(Kind::Q, to.id), None))
        }
        "move_node" => {
            let moved = node(cx, text(op, "node"), &[Kind::A])?;
            let to = node(cx, text(op, "to"), &[Kind::Q])?;
            let aid = fmt_id(Kind::A, moved.id);
            let to_id = fmt_id(Kind::Q, to.id);
            let Some(from) = moved.parent_id else { return invalid(format!("{aid} に親がありません")) };
            if from == to.id {
                return invalid(format!("{aid} は既に {to_id} の下にあります"));
            }
            if !alive_children(cx, to.id, Kind::Q)?.is_empty() {
                return invalid(format!("{to_id} には child_Q があるので、A を移せません"));
            }
            if is_under(conn, to.id, moved.id)? {
                return invalid(format!("{to_id} は {aid} の下にあるので移せません"));
            }
            store::set_parent(conn, moved.id, to.id)?;
            log(cx, moved.id, "move_node", &format!("{} から {to_id} へ", fmt_id(Kind::Q, from)))?;
            Ok((aid, None))
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
            if n.kind == Kind::D && !alive_children(cx, n.id, Kind::S)?.is_empty() {
                return invalid(format!(
                    "{} の下に SPROUT（S）があるので消せません。先に S を消してください",
                    fmt_id(Kind::D, n.id)
                ));
            }
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
            if [t, text(op, "body")].iter().any(|s| textcheck::find_inline_cite(s).is_some()) {
                return invalid("A の文と本文に出典（#番号）は書けません。出典は add_f の cites に書き、F として足してください");
            }
            if alive_children(cx, q.id, Kind::A)?.iter().any(|a| a.text == t) {
                return invalid(format!("{qid} に同じ文の A があります"));
            }
            let mut n = new_node(cx, Kind::A, Some(q.id), t);
            n.body = text(op, "body");
            let id = store::insert_node(conn, &n)?;
            Ok((fmt_id(Kind::A, id), None))
        }
        "add_f" => {
            let parent = node(cx, text(op, "parent"), &[Kind::A])?;
            let t = text(op, "text");
            let sentences = sentence_count(t);
            if sentences > 3 {
                return invalid(format!("text は 3 文までです（今 {sentences} 文。{SENTENCE_RULE}）"));
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
            if parent.kind == Kind::S {
                let (done, missing) = seedq::is_complete(conn, &parent)?;
                if !done {
                    let ids: Vec<String> = missing.into_iter().map(fmt_sq).collect();
                    return invalid(format!(
                        "{ptext} は未完成です（未回答: {}）。全部の問いに回答してから実験を足してください",
                        ids.join(", ")
                    ));
                }
            }
            let mut n = new_node(cx, Kind::E, Some(parent.id), text(op, "text"));
            n.body = text(op, "body");
            n.mlflow_ref = op.get("mlflow").and_then(Value::as_str).filter(|s| !s.is_empty());
            n.dataset_ref = op.get("dataset").and_then(Value::as_str).filter(|s| !s.is_empty());
            Ok((fmt_id(Kind::E, store::insert_node(conn, &n)?), None))
        }
        "add_seed" => {
            let mut n = new_node(cx, Kind::D, None, text(op, "title"));
            n.body = text(op, "body");
            let id = store::insert_node(conn, &n)?;
            for (axis, question) in seedq::TEMPLATE {
                seedq::insert_question(conn, id, question, axis)?;
            }
            Ok((fmt_id(Kind::D, id), None))
        }
        "add_seed_q" => {
            let d = node(cx, text(op, "seed"), &[Kind::D])?;
            let axis = op.get("axis").and_then(Value::as_str).filter(|a| !a.is_empty());
            let q = seedq::insert_question(conn, d.id, text(op, "text"), axis)?;
            seedq::refresh(conn, d.id)?;
            let sq = fmt_sq(q);
            log(cx, d.id, "edit", &format!("add_seed_q {sq} {}", text(op, "text")))?;
            Ok((sq, None))
        }
        "edit_seed_q" => {
            let q = live_question(conn, text(op, "q"))?;
            let t = op.get("text").and_then(Value::as_str);
            // axis は空文字で外す。項目が無いときは変えない。
            let axis = op.get("axis").and_then(Value::as_str).map(|a| (!a.is_empty()).then_some(a));
            if t.is_none() && axis.is_none() {
                return invalid("text か axis のどちらかが必要です");
            }
            seedq::update_question(conn, q.id, t, axis)?;
            seedq::refresh(conn, q.seed_id)?;
            let sq = fmt_sq(q.id);
            log(cx, q.seed_id, "edit", &format!("edit_seed_q {sq}"))?;
            Ok((sq, None))
        }
        "remove_seed_q" => {
            let q = live_question(conn, text(op, "q"))?;
            seedq::remove_question(conn, q.id)?;
            seedq::refresh(conn, q.seed_id)?;
            let sq = fmt_sq(q.id);
            log(cx, q.seed_id, "edit", &format!("remove_seed_q {sq}"))?;
            Ok((sq, None))
        }
        "add_answer" => {
            let owner = node(cx, text(op, "to"), &[Kind::D, Kind::S])?;
            let id = put_answer(cx, &owner, text(op, "q"), op.get("a").and_then(Value::as_str), op.get("text").and_then(Value::as_str))?;
            Ok((fmt_sa(id), None))
        }
        "edit_answer" => {
            let id = parse_sa(text(op, "answer"))?;
            let ans = seedq::answer(conn, id)?;
            if ans.removed_at.is_some() {
                return invalid(format!("{} は消した回答です", fmt_sa(id)));
            }
            if ans.a_id.is_some() {
                return invalid("path の回答は remove_answer と add_answer で直してください");
            }
            let t = text(op, "text");
            check_answer_text(t)?;
            seedq::update_answer_text(conn, id, t)?;
            refresh_owner(conn, ans.owner_id)?;
            log(cx, ans.owner_id, "edit", &format!("edit_answer {}", fmt_sa(id)))?;
            Ok((fmt_sa(id), None))
        }
        "remove_answer" => {
            let id = parse_sa(text(op, "answer"))?;
            let ans = seedq::answer(conn, id)?;
            if ans.removed_at.is_some() {
                return invalid(format!("{} は既に消えています", fmt_sa(id)));
            }
            seedq::remove_answer(conn, id)?;
            refresh_owner(conn, ans.owner_id)?;
            log(cx, ans.owner_id, "edit", &format!("remove_answer {}", fmt_sa(id)))?;
            Ok((fmt_sa(id), None))
        }
        "add_sprout" => {
            let seed_text = text(op, "seed");
            let seed = match parse_id(seed_text) {
                Ok((Kind::D, _)) => node(cx, seed_text, &[Kind::D])?,
                _ => return invalid(format!("seed には seed（D）の ID を書いてください: {seed_text}")),
            };
            let s = NewNode {
                kind: Kind::S,
                parent_id: Some(seed.id),
                guide_key: None,
                text: text(op, "title"),
                body: text(op, "body"),
                stance: None,
                cites: &[],
                paper_id: None,
                run_id: cx.run_id,
                report_id: None,
                mlflow_ref: None,
                dataset_ref: None,
                paths: &[],
                created_by: &cx.by,
            };
            let id = store::insert_node(conn, &s)?;
            let answers = op.get("answers").and_then(Value::as_array).cloned().unwrap_or_default();
            for item in &answers {
                let owner = store::get_node(conn, id)?;
                put_answer(
                    cx,
                    &owner,
                    item.get("q").and_then(Value::as_str).unwrap_or(""),
                    item.get("a").and_then(Value::as_str),
                    item.get("text").and_then(Value::as_str),
                )?;
            }
            seedq::refresh(conn, seed.id)?;
            let row = store::get_node(conn, id)?;
            let range = sprout_range(conn, &row.paths)?;
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
        "set_dataset" => {
            let e = node(cx, text(op, "node"), &[Kind::E])?;
            let value = text(op, "dataset");
            let next = (!value.is_empty()).then_some(value);
            if e.dataset_ref.as_deref() != next {
                store::set_dataset(conn, e.id, next)?;
                log(cx, e.id, "set_dataset", value)?;
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
                    Kind::A | Kind::E | Kind::S | Kind::D => 120,
                    Kind::F => 1200,
                };
                if t.chars().count() > max {
                    return invalid(format!("text は {max} 字までです"));
                }
                let sentences = sentence_count(t);
                if n.kind == Kind::F && sentences > 3 {
                    return invalid(format!("text は 3 文までです（今 {sentences} 文。{SENTENCE_RULE}）"));
                }
            }
            if b.is_some() && !matches!(n.kind, Kind::A | Kind::E | Kind::S | Kind::D) {
                return invalid("body は A と E と SPROUT と seed（D）だけに付けられます");
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

/// 生きた問い 1 つ（ID の形と、消していないことを確かめる）。
fn live_question(conn: &Connection, id_text: &str) -> Result<seedq::QuestionRow> {
    let id = parse_sq(id_text)?;
    let q = seedq::question(conn, id).or_else(|_| invalid(format!("{id_text} はありません")))?;
    if q.removed_at.is_some() {
        return invalid(format!("{id_text} は消した問いです"));
    }
    Ok(q)
}

/// テキストの回答の文の検査（1〜300 字・外国語の文字種・出典（#番号）は書けない）。
fn check_answer_text(t: &str) -> Result<()> {
    let n = t.chars().count();
    if n == 0 {
        return invalid("text が短すぎます（1 字以上）");
    }
    if n > 300 {
        return invalid("text は 300 字までです");
    }
    if let Some(msg) = textcheck::foreign_message(t) {
        return invalid(format!("text: {msg}"));
    }
    if textcheck::find_inline_cite(t).is_some() {
        return invalid("回答の文に出典（#番号）は書けません。出典は add_f の cites に書き、F として足してください");
    }
    Ok(())
}

/// 持ち主（D か S）の seed の完成・凍結・paths を直す。seed の無い古い S は何もしない。
fn refresh_owner(conn: &Connection, owner: i64) -> Result<()> {
    let row = store::get_node(conn, owner)?;
    if let Some(seed) = seedq::seed_of(conn, &row) {
        seedq::refresh(conn, seed)?;
    }
    Ok(())
}

/// 回答を 1 つ足す（add_answer と、add_sprout の answers）。`a` と `text` のちょうど一方。
/// 問いは持ち主の seed の問いで、消していないこと。S には、その S が答える問い（relevant）だけ。D で答えた問いには S から答えない。
fn put_answer(cx: &Cx<'_>, owner: &NodeRow, q_text: &str, a_text: Option<&str>, t: Option<&str>) -> Result<i64> {
    let conn = cx.conn;
    let Some(seed) = seedq::seed_of(conn, owner) else {
        return invalid(format!("{} は seed の下にないので回答できません", fmt_id(owner.kind, owner.id)));
    };
    let q = live_question(conn, q_text)?;
    if q.seed_id != seed {
        return invalid(format!("{q_text} は {} の問いではありません", fmt_id(Kind::D, seed)));
    }
    let owner_id = fmt_id(owner.kind, owner.id);
    if owner.kind == Kind::S {
        if !seedq::relevant(conn, owner)?.contains(&q.id) {
            let done = seedq::is_complete(conn, owner)?.0;
            return invalid(if done {
                format!("{owner_id} は完成済みで、{q_text} は対象外です")
            } else {
                format!("{q_text} は {owner_id} の対象外です")
            });
        }
        if seedq::has_answer(conn, seed, q.id)? {
            return invalid(format!("{q_text} は seed で回答済みです"));
        }
    }
    let (a_id, t) = match (a_text, t) {
        (Some(a), None) => (Some(node(cx, a, &[Kind::A])?.id), None),
        (None, Some(t)) => {
            check_answer_text(t)?;
            (None, Some(t))
        }
        _ => return invalid("回答は a（A の ID）か text（文）のどちらか一方を書いてください"),
    };
    if let Some(a) = a_id {
        if seedq::has_path(conn, owner.id, q.id, a)? {
            return invalid(format!("{q_text} に同じ A（{}）は既に答えてあります", fmt_id(Kind::A, a)));
        }
    }
    let id = seedq::insert_answer(conn, owner.id, q.id, a_id, t, &cx.by)?;
    let detail = match (a_id, t) {
        (Some(a), _) => format!("add_answer {q_text} {}", fmt_id(Kind::A, a)),
        (None, Some(t)) => format!("add_answer {q_text} {t}"),
        (None, None) => format!("add_answer {q_text}"),
    };
    log(cx, owner.id, "edit", &detail)?;
    refresh_owner(conn, owner.id)?;
    Ok(id)
}

/// Q の下に中間の A と細分の Q を作り、Q の直下の A（`members`）を細分の Q の下へまとめて移す（ID は保つ）。
/// 返すのは (中間の A の ID, 細分の Q の ID)。途中の検査で断られたら、何も書かない（検査は書く前に全部済ませる）。
fn regroup(cx: &Cx<'_>, op: &Value) -> Result<(i64, i64)> {
    let q = node(cx, text(op, "q"), &[Kind::Q])?;
    let qid = fmt_id(Kind::Q, q.id);
    if !alive_children(cx, q.id, Kind::Q)?.is_empty() {
        return invalid(format!("{qid} には child_Q があるので、regroup できません"));
    }
    let t = text(op, "text");
    if [t, text(op, "body")].iter().any(|s| textcheck::find_inline_cite(s).is_some()) {
        return invalid("A の文と本文に出典（#番号）は書けません。出典は add_f の cites に書き、F として足してください");
    }
    if alive_children(cx, q.id, Kind::A)?.iter().any(|a| a.text == t) {
        return invalid(format!("{qid} に同じ文の A があります"));
    }
    let mut members: Vec<i64> = Vec::new();
    for m in strings(op, "members") {
        let found = node(cx, &m, &[Kind::A])?;
        if found.parent_id != Some(q.id) {
            return invalid(format!("{m} は {qid} の直下の A ではありません"));
        }
        if members.contains(&found.id) {
            return invalid(format!("同じ A が 2 回あります: {m}"));
        }
        members.push(found.id);
    }
    let mut a = new_node(cx, Kind::A, Some(q.id), t);
    a.body = text(op, "body");
    let a_id = store::insert_node(cx.conn, &a)?;
    let mut sub = new_node(cx, Kind::Q, Some(a_id), text(op, "child_q"));
    sub.body = text(op, "child_q_body");
    let sub_id = store::insert_node(cx.conn, &sub)?;
    for m in &members {
        store::set_parent(cx.conn, *m, sub_id)?;
    }
    // 記録は移し先の Q に書く（move_a と同じ。steps は移した A を detail の（）から読む）。
    let ids = members.iter().map(|m| fmt_id(Kind::A, *m)).collect::<Vec<_>>().join(", ");
    log(cx, sub_id, "regroup", &format!("{qid} から A {} 件（{ids}）", members.len()))?;
    Ok((a_id, sub_id))
}

// ---- テスト -----------------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rt::{axes, defs, seedq};
    use sai_paper_core::database::Database;
    use std::path::PathBuf;

    /// 6 軸の見出しをそろえた seed（D）の本文。軸は全部「空欄」（まだ決めていない）。
    const SEED_BODY: &str = "## 背景\n空欄\n## 解く問題（task）\n空欄\n## 対象（target）\n空欄\n## データ（data）\n空欄\n## 手法（method）\n空欄\n## 出力（output）\n空欄\n## 評価指標（metric）\n空欄\n## 成功基準\n空欄\n## 制約\n空欄";

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
            .find(|n| n.guide_key.as_deref() == Some("task"))
            .unwrap()
            .id;
        let blocks = dir.join(".sai/blocks");
        std::fs::create_dir_all(&blocks).unwrap();
        std::fs::write(
            blocks.join("t-all.md"),
            "---\nsci_ops: add_q, insert_q, move_a, move_node, regroup, copy, remove, add_a, add_f, add_e, add_seed, add_seed_q, edit_seed_q, remove_seed_q, add_answer, edit_answer, remove_answer, add_sprout, set_stance, set_mlflow, set_dataset\n---\n# t\n",
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
            apply_user(self.conn(), &self.roots, &self.defs, &op, "user")
        }
        /// run 7 の質問に回答を 1 件記録する（許可か、許可しない）。
        fn approve(&self, qid: &str, answer: &str) {
            store::insert_approval(self.conn(), 7, qid, "作ってよいか", answer, "saituki").unwrap();
        }
        /// seed（D）を 1 つ足し、ID を返す（agent として）。
        fn seed(&self) -> String {
            self.agent1(json!({"op": "add_seed", "title": "研究", "body": SEED_BODY})).unwrap()
        }
        /// sub の下に A を足す。
        fn a(&self, text: &str) -> String {
            self.agent1(json!({"op": "add_a", "q": qid(self.sub), "text": text})).unwrap()
        }
        fn count(&self, table: &str) -> i64 {
            self.conn().query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0)).unwrap()
        }
        /// seed の問いのうち、軸が `axis` の問いの ID（`SQ<n>`）。
        fn sq(&self, seed: &str, axis: &str) -> String {
            let seed_id = parse_id(seed).unwrap().1;
            let qs = seedq::questions_of(self.conn(), seed_id, false).unwrap();
            fmt_sq(qs.iter().find(|q| q.axis.as_deref() == Some(axis)).unwrap().id)
        }
        /// seed の下に、A（path）で答えた SPROUT を足す（A の軸の問いに答える）。
        fn sprout(&self, seed: &str, a: &str) -> String {
            let aid = parse_id(a).unwrap().1;
            let axis = axes::axis_of(self.conn(), aid).unwrap().unwrap();
            let q = self.sq(seed, &axis);
            self.agent1(json!({"op": "add_sprout", "seed": seed, "title": "種", "answers": [{"q": q, "a": a}]})).unwrap()
        }
        /// SPROUT の残りの問いに、テキストで答えて完成させる。
        fn fill(&self, s: &str) {
            let sid = parse_id(s).unwrap().1;
            let (_, missing) = seedq::is_complete(self.conn(), &store::get_node(self.conn(), sid).unwrap()).unwrap();
            for q in missing {
                self.agent1(json!({"op": "add_answer", "to": s, "q": fmt_sq(q), "text": "答え"})).unwrap();
            }
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
        // 利用者でも、廃止した SEED の下や空の親では軸の Q を作れない（軸の Q は sai-sci init が作る）。
        assert!(err(e.user(json!({"op": "add_q", "parent": "SEED", "text": "x"}))).contains("軸の Q は sai-sci init が作ります"));
        assert!(err(e.user(json!({"op": "add_q", "parent": "", "text": "x"}))).contains("軸の Q は sai-sci init が作ります"));
        // Q の下なら child_Q を作れる。親は必ず付く。
        let u = e.user(json!({"op": "add_q", "parent": qid(e.by_key("data")), "text": "もう一つ"})).unwrap().result;
        let (_, uid) = parse_id(&u).unwrap();
        assert!(store::get_node(e.conn(), uid).unwrap().parent_id.is_some());
        // 知らない ID は弾く。
        assert!(err(e.user(json!({"op": "add_q", "parent": "NOPE", "text": "x"}))).contains("ID"));
    }

    /// 記録の actor: Professor の代行（`SAI_PARENT_RUN_ID` の run）は `run:<id>`、利用者は `user`。
    #[test]
    fn apply_user_records_given_actor() {
        let e = env();
        apply_user(e.conn(), &e.roots, &e.defs, &json!({"op": "add_q", "parent": qid(e.sub), "text": "代行の問い"}), "run:7").unwrap();
        apply_user(e.conn(), &e.roots, &e.defs, &json!({"op": "add_q", "parent": qid(e.sub), "text": "利用者の問い"}), "user").unwrap();
        let actors: Vec<String> = store::list_log(e.conn(), Some(2)).unwrap().into_iter().map(|row| row.actor).collect();
        assert_eq!(actors, vec!["user".to_string(), "run:7".to_string()]);
    }

    /// 軸の Q に A を足せる。A を足しても型の Q は増えない（A の下に Q を作らない）。
    #[test]
    fn add_a_creates_no_qs() {
        let e = env();
        let a = e.agent1(json!({"op": "add_a", "q": qid(e.sub), "text": "予後予測"})).unwrap();
        let (_, aid) = parse_id(&a).unwrap();
        assert!(store::children(e.conn(), aid).unwrap().is_empty());
        assert_eq!(store::get_node(e.conn(), aid).unwrap().created_by, "run:7");
        assert!(err(e.agent1(json!({"op": "add_a", "q": qid(e.sub), "text": "予後予測"}))).contains("同じ文"));
        // A のある Q に child_Q は足せず、child_Q のある Q に A は足せない。
        assert!(err(e.user(json!({"op": "add_q", "parent": qid(e.sub), "text": "子"}))).contains("A があるので"));
        let target = e.by_key("target");
        e.user(json!({"op": "add_q", "parent": qid(target), "text": "子"})).unwrap();
        assert!(err(e.agent1(json!({"op": "add_a", "q": qid(target), "text": "x"}))).contains("child_Q があるので"));
    }

    #[test]
    fn add_f_rules() {
        let e = env();
        let a = e.a("A");
        let f = |t: &str, cites: Value| e.agent1(json!({"op": "add_f", "parent": a, "text": t, "stance": "support", "cites": cites}));
        assert!(err(f("精度は 85 %", json!([]))).contains("数字を含む F には出典"));
        assert!(err(f("한 国", json!([]))).contains("text: ハングル"));
        assert!(err(f("一。二。三。四。", json!([]))).contains("3 文"));
        assert!(err(f("一。二。三。四。", json!([]))).contains("今 4 文"));
        assert!(f("数字なし", json!([])).is_ok());
        // stance が無いと拒否する。
        assert!(err(e.agent1(json!({"op": "add_f", "parent": a, "text": "x"}))).contains("stance が必要です"));
        // 親が E だと拒否する（A の ID でない）。
        let s = e.sprout(&e.seed(), &a);
        e.fill(&s);
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
        let s = e.sprout(&e.seed(), &a);
        e.fill(&s);
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
    fn add_sprout_answers_and_report_range() {
        let e = env();
        let a = e.a("A");
        let (_, aid) = parse_id(&a).unwrap();
        let b = e.agent1(json!({"op": "add_a", "q": qid(e.by_key("target")), "text": "B"})).unwrap();
        let (_, bid) = parse_id(&b).unwrap();
        let seed = e.seed();
        let (task, target) = (e.sq(&seed, "task"), e.sq(&seed, "target"));
        // paths は廃止（断る）。同じ問いに同じ A は 2 回書けない。問いの ID の形は確かめる。
        assert!(err(e.agent1(json!({"op": "add_sprout", "seed": seed, "title": "種", "paths": [a]}))).contains("paths は廃止しました"));
        assert!(err(e.agent1(json!({"op": "add_sprout", "seed": seed, "title": "種", "answers": [{"q": task, "a": a}, {"q": task, "a": a}]}))).contains("同じ A"));
        assert!(err(e.agent1(json!({"op": "add_sprout", "seed": seed, "title": "種", "answers": [{"q": "Q1", "a": a}]}))).contains("ID の形"));
        assert!(err(e.agent1(json!({"op": "add_sprout", "seed": seed, "title": "種", "answers": [{"q": task, "a": a, "text": "両方"}]}))).contains("どちらか一方"));
        // 通り、範囲（A は 2 つ・F は 0）が返る。paths は回答の順（問いの順）。
        let r = e.agent(&[json!({"op": "add_sprout", "seed": seed, "title": "種", "body": "本文", "answers": [{"q": target, "a": b}, {"q": task, "a": a}]})]).unwrap();
        let (skind, sid) = parse_id(&r.results[0].result).unwrap();
        assert_eq!(skind, Kind::S);
        let node = store::get_node(e.conn(), sid).unwrap();
        assert_eq!(node.paths, vec![aid, bid], "paths は問いの順（task の A、target の A）");
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
            json!({"op": "move_node", "node": q, "to": q}),
            json!({"op": "regroup", "q": q, "text": "x", "child_q": "y", "members": ["A1"]}),
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
        let batch = || vec![json!({"op": "add_q", "parent": qid(e.by_key("output")), "text": "根"}), json!({"op": "remove", "node": a})];
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
    fn insert_q_and_move_a_log_the_moved_ids() {
        let e = env();
        let a1 = e.a("一つ目");
        let a2 = e.a("二つ目");
        e.approve("q3", "許可する");
        let nq = e.agent1(json!({"op": "insert_q", "q": qid(e.sub), "text": "条件を分ける"})).unwrap();
        let last = |action: &str| {
            store::list_log(e.conn(), None).unwrap().into_iter().find(|l| l.action == action).unwrap().detail
        };
        assert_eq!(last("insert_q"), format!("{} の A 2 件を移した（{a1}, {a2}）", qid(e.sub)));
        let other = qid(e.by_key("target"));
        e.user(json!({"op": "move_a", "from": nq, "to": other})).unwrap();
        assert_eq!(last("move_a"), format!("{nq} から A 2 件（{a1}, {a2}）"));
    }

    #[test]
    fn move_a_rejects_child_q_and_same_q() {
        let e = env();
        e.a("A");
        // 許可は 1 件あれば足りる（失敗した op は許可を消費しない）。
        e.approve("q4", "許可する");
        // child_Q を持つ Q へは移せない。
        let target = e.by_key("target");
        e.user(json!({"op": "add_q", "parent": qid(target), "text": "子"})).unwrap();
        assert!(err(e.agent1(json!({"op": "move_a", "from": qid(e.sub), "to": qid(target)}))).contains("child_Q があるので"));
        assert!(err(e.agent1(json!({"op": "move_a", "from": qid(e.sub), "to": qid(e.sub)}))).contains("別の Q"));
    }

    #[test]
    fn copy_keeps_stance_and_only_f() {
        let e = env();
        let a = e.a("A");
        e.agent1(json!({"op": "add_f", "parent": a, "text": "根拠", "stance": "deny"})).unwrap();
        let other = qid(e.by_key("target"));
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
        let s = e.sprout(&e.seed(), &a);
        e.fill(&s);
        let ex = e.agent1(json!({"op": "add_e", "parent": s, "text": "実験", "mlflow": "exp/1"})).unwrap();
        e.agent1(json!({"op": "set_mlflow", "node": ex, "mlflow": ""})).unwrap();
        let (_, eid) = parse_id(&ex).unwrap();
        assert_eq!(store::get_node(e.conn(), eid).unwrap().mlflow_ref, None);
        let last = &store::list_log(e.conn(), Some(1)).unwrap()[0];
        assert_eq!((last.action.as_str(), last.detail.as_str()), ("set_mlflow", ""));
    }

    #[test]
    fn add_e_takes_dataset_and_set_dataset_changes_it() {
        let e = env();
        let a = e.a("A");
        let s = e.sprout(&e.seed(), &a);
        e.fill(&s);
        let ex = e.agent1(json!({"op": "add_e", "parent": s, "text": "実験", "dataset": "ts:a@0123456789ab"})).unwrap();
        let (_, eid) = parse_id(&ex).unwrap();
        assert_eq!(store::get_node(e.conn(), eid).unwrap().dataset_ref.as_deref(), Some("ts:a@0123456789ab"));
        let logs = |conn| store::list_log(conn, None).unwrap().iter().filter(|l| l.action == "set_dataset").count();
        // 同じ値は log を書かない。
        e.agent1(json!({"op": "set_dataset", "node": ex, "dataset": "ts:a@0123456789ab"})).unwrap();
        assert_eq!(logs(e.conn()), 0);
        e.agent1(json!({"op": "set_dataset", "node": ex, "dataset": "ts:b@ba9876543210"})).unwrap();
        assert_eq!(store::get_node(e.conn(), eid).unwrap().dataset_ref.as_deref(), Some("ts:b@ba9876543210"));
        assert_eq!(logs(e.conn()), 1);
        e.agent1(json!({"op": "set_dataset", "node": ex, "dataset": ""})).unwrap();
        assert_eq!(store::get_node(e.conn(), eid).unwrap().dataset_ref, None);
        let last = &store::list_log(e.conn(), Some(1)).unwrap()[0];
        assert_eq!((last.action.as_str(), last.detail.as_str()), ("set_dataset", ""));
        // E 以外（S・A）と、201 字は断る。
        assert!(err(e.agent1(json!({"op": "set_dataset", "node": s, "dataset": "x"}))).contains("E の ID"));
        assert!(err(e.agent1(json!({"op": "set_dataset", "node": a, "dataset": "x"}))).contains("E の ID"));
        assert!(e.agent1(json!({"op": "set_dataset", "node": ex, "dataset": "x".repeat(201)})).is_err());
    }

    /// 範囲は A の祖先の Q と A、A の子の F だけを数える（A の子の Q 以下・祖先の A の F は入れない）。
    #[test]
    fn sprout_range_counts_ancestors_and_own_f() {
        let e = env();
        // Q1 ─ A2 ─ Q3 ─ A4 ─ {F(support), F(deny)}、A2 ─ F(issue)
        let q1 = qid(e.sub);
        let a2 = e.agent1(json!({"op": "add_a", "q": q1, "text": "A2"})).unwrap();
        let q3 = e.user(json!({"op": "add_q", "parent": a2, "text": "Q3"})).unwrap().result;
        let a4 = e.agent1(json!({"op": "add_a", "q": q3, "text": "A4"})).unwrap();
        e.agent1(json!({"op": "add_f", "parent": a4, "text": "支持", "stance": "support"})).unwrap();
        e.agent1(json!({"op": "add_f", "parent": a4, "text": "反対", "stance": "deny"})).unwrap();
        e.agent1(json!({"op": "add_f", "parent": a2, "text": "懸念", "stance": "issue"})).unwrap();
        let seed = e.seed();
        let r = e.agent(&[json!({"op": "add_sprout", "seed": seed, "title": "種", "answers": [{"q": e.sq(&seed, "task"), "a": a4}]})]).unwrap();
        assert_eq!(r.results[0].range, Some(Range { q: 2, a: 2, f_support: 1, f_deny: 1, f_issue: 0 }));
    }

    #[test]
    fn batch_size_error_says_count() {
        let e = env();
        let q = qid(e.sub);
        let n = e.count("rt_nodes");
        let over = vec![json!({"op": "add_a", "q": q, "text": "A"}); 21];
        let m = err(e.agent(&over));
        assert!(m.contains("1〜20 件です（今 21 件。"), "{m}");
        assert_eq!(e.count("rt_nodes"), n);
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
        let other = qid(e.by_key("target"));
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
        let tg = e.by_key("target");
        e.user(json!({"op": "remove", "node": qid(tg)})).unwrap();
        assert!(store::get_node(e.conn(), tg).unwrap().removed_at.is_some());
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
            ["add_q", "insert_q", "move_a", "move_node", "regroup", "copy", "remove", "add_a", "add_f", "add_e", "add_seed", "add_seed_q", "edit_seed_q", "remove_seed_q", "add_answer", "edit_answer", "remove_answer", "add_sprout", "set_stance", "set_mlflow", "set_dataset"]
        );
        assert_eq!(op_specs().len(), 24, "op は 24 種類（利用者だけの 3 つを含む）");
        assert!(is_user_only_op("star") && !is_user_only_op("add_a") && is_known_op("edit"));
    }

    #[test]
    fn seed_and_sprout_under_seed() {
        let e = env();
        let (a, a2) = (e.a("A"), e.a("A2"));
        // seed（D）の本文は自由な文（書式は要らない）。問いは雛形の 7 問が最初から入る。
        let d1 = e.agent1(json!({"op": "add_seed", "title": "自分の研究", "body": "背景の文"})).unwrap();
        let (dk, did) = parse_id(&d1).unwrap();
        assert_eq!(dk, Kind::D);
        let seed_row = store::get_node(e.conn(), did).unwrap();
        assert_eq!((seed_row.body.as_str(), seed_row.parent_id), ("背景の文", None));
        let qs = seedq::questions_of(e.conn(), did, false).unwrap();
        assert_eq!(qs.len(), 7);
        assert_eq!(qs[6].axis, None);
        // seed の無い add_sprout と、seed に Q を書いた add_sprout は断られる。
        assert!(err(e.agent1(json!({"op": "add_sprout", "title": "種"}))).contains("seed が必要です"));
        assert!(err(e.agent1(json!({"op": "add_sprout", "seed": qid(e.sub), "title": "種"}))).contains("seed には seed（D）の ID を書いてください"));
        // 1 つの D に S を 2 つ作れる。両方の親は D。
        let s1 = parse_id(&e.sprout(&d1, &a)).unwrap().1;
        let s2 = parse_id(&e.sprout(&d1, &a2)).unwrap().1;
        assert_eq!(store::get_node(e.conn(), s1).unwrap().parent_id, Some(did));
        assert_eq!(store::get_node(e.conn(), s2).unwrap().parent_id, Some(did));
        // 子の S がある D は消せない。子の無い D は消せる。
        assert!(err(e.user(json!({"op": "remove", "node": d1}))).contains("SPROUT（S）があるので消せません"));
        let d2 = e.user(json!({"op": "add_seed", "title": "空の研究"})).unwrap().result;
        e.user(json!({"op": "remove", "node": d2})).unwrap();
        assert!(store::get_node(e.conn(), parse_id(&d2).unwrap().1).unwrap().removed_at.is_some());
        // 利用者は D の text と body を直せる（自由な文）。
        e.user(json!({"op": "edit", "node": d1, "text": "直した題"})).unwrap();
        e.user(json!({"op": "edit", "node": d1, "body": "直した本文"})).unwrap();
        let edited = store::get_node(e.conn(), did).unwrap();
        assert_eq!((edited.text.as_str(), edited.body.as_str()), ("直した題", "直した本文"));
    }

    #[test]
    fn add_a_rejects_inline_cite() {
        let e = env();
        let q = qid(e.sub);
        assert!(err(e.agent1(json!({"op": "add_a", "q": q, "text": "精度は #37 による"}))).contains("出典（#番号）は書けません"));
        assert!(err(e.agent1(json!({"op": "add_a", "q": q, "text": "候補", "body": "根拠は #37.5.2"}))).contains("出典"));
        // Markdown の見出しは通る。
        e.agent1(json!({"op": "add_a", "q": q, "text": "候補", "body": "## 見出し"})).unwrap();
    }

    /// move_node: A を F・child_Q ごと別の Q へ。ID は変わらず、その A を持つ S の paths もそのまま。
    #[test]
    fn move_node_moves_subtree_and_keeps_ids() {
        let e = env();
        let a = e.a("移す A");
        let (_, aid) = parse_id(&a).unwrap();
        let f = e.agent1(json!({"op": "add_f", "parent": a, "text": "根拠の文", "stance": "support"})).unwrap();
        let (_, fid) = parse_id(&f).unwrap();
        e.user(json!({"op": "add_q", "parent": a, "text": "A の下の問い"})).unwrap();
        let s = e.sprout(&e.seed(), &a);
        e.fill(&s);
        let (_, sid) = parse_id(&s).unwrap();
        let to = qid(e.by_key("target"));
        // 許可が無い agent は終了コード 4。
        assert!(conflict(e.agent1(json!({"op": "move_node", "node": a, "to": to}))).contains("許可がありません"));
        e.approve("m1", "許可する");
        assert_eq!(e.agent1(json!({"op": "move_node", "node": a, "to": to})).unwrap(), a);
        // ID は変わらず、F と child_Q は A の下に付いたまま。S の paths もそのまま。
        assert_eq!(store::get_node(e.conn(), aid).unwrap().parent_id, Some(parse_id(&to).unwrap().1));
        assert_eq!(store::get_node(e.conn(), fid).unwrap().parent_id, Some(aid));
        assert_eq!(store::get_node(e.conn(), sid).unwrap().paths, vec![aid]);
        assert_eq!(store::children(e.conn(), aid).unwrap().iter().filter(|n| n.kind == Kind::Q).count(), 1);
        // 記録は移した A に付き、承認の印が入る。
        let row = store::list_log(e.conn(), None).unwrap().into_iter().find(|l| l.action == "move_node").unwrap();
        assert_eq!((row.node_id, row.approved_by.as_deref()), (aid, Some("saituki")));
    }

    /// move_node の 3 つの断り（同じ親・子の Q がある移し先・移し先が A の子孫）。
    #[test]
    fn move_node_rejects_same_parent_child_q_and_descendant() {
        let e = env();
        let a = e.a("A");
        let to = qid(e.by_key("target"));
        e.approve("m2", "許可する");
        e.agent1(json!({"op": "move_node", "node": a, "to": to})).unwrap();
        // 既に同じ Q の下にある。
        e.approve("m3", "許可する");
        assert!(err(e.agent1(json!({"op": "move_node", "node": a, "to": to}))).contains("は既に"));
        // 移し先に child_Q がある（user で作る）。
        let data = qid(e.by_key("data"));
        e.user(json!({"op": "add_q", "parent": data, "text": "子の問い"})).unwrap();
        e.approve("m4", "許可する");
        assert!(err(e.agent1(json!({"op": "move_node", "node": a, "to": data}))).contains("child_Q があるので"));
        // 移し先が A の子孫（A の下の Q）。
        let under = e.user(json!({"op": "add_q", "parent": a, "text": "A の下の問い"})).unwrap().result;
        e.approve("m5", "許可する");
        assert!(err(e.agent1(json!({"op": "move_node", "node": a, "to": under}))).contains("の下にあるので移せません"));
    }

    /// regroup（r4 §1 の例）: 側頭葉・IESS・LGS・大田原が兄弟のとき、中間の A と細分の Q を作り、3 つをその下へ移す。ID は保つ。
    #[test]
    fn regroup_makes_middle_answer_and_moves_members() {
        let e = env();
        let temporal = e.a("側頭葉てんかん");
        let members = [e.a("IESS（West 症候群）"), e.a("Lennox-Gastaut 症候群"), e.a("大田原症候群")];
        let parent_before: Vec<i64> = members.iter().map(|m| store::get_node(e.conn(), parse_id(m).unwrap().1).unwrap().parent_id.unwrap()).collect();
        e.approve("g1", "許可する");
        let r = e
            .agent(&[json!({
                "op": "regroup", "q": qid(e.sub), "text": "発達性てんかん性脳症", "body": "発作の始まりが早い",
                "child_q": "発達性てんかん性脳症には、どんな種類があるか。", "child_q_body": "分け方の基準: 発症の時期",
                "members": members,
            })])
            .unwrap();
        let res = &r.results[0];
        assert_eq!(res.created.len(), 2, "中間の A と細分の Q");
        assert_eq!(res.result, res.created[0]);
        let (ak, mid) = parse_id(&res.created[0]).unwrap();
        let (qk, subq) = parse_id(&res.created[1]).unwrap();
        assert_eq!((ak, qk), (Kind::A, Kind::Q));
        let mid_row = store::get_node(e.conn(), mid).unwrap();
        assert_eq!((mid_row.parent_id, mid_row.text.as_str()), (Some(e.sub), "発達性てんかん性脳症"));
        let sub_row = store::get_node(e.conn(), subq).unwrap();
        assert_eq!((sub_row.parent_id, sub_row.body.as_str()), (Some(mid), "分け方の基準: 発症の時期"));
        // 3 つの A は ID を保ち、細分の Q の下へ移る。兄弟の A（側頭葉）は動かない。
        for (m, before) in members.iter().zip(&parent_before) {
            let row = store::get_node(e.conn(), parse_id(m).unwrap().1).unwrap();
            assert_eq!(row.parent_id, Some(subq), "{m}");
            assert_ne!(Some(*before), row.parent_id);
        }
        assert_eq!(store::get_node(e.conn(), parse_id(&temporal).unwrap().1).unwrap().parent_id, Some(e.sub));
        // 記録は細分の Q に 1 行、承認の印付き。
        let row = store::list_log(e.conn(), None).unwrap().into_iter().find(|l| l.action == "regroup").unwrap();
        assert_eq!((row.node_id, row.approved_by.as_deref()), (subq, Some("saituki")));
        // 途中の検査で落ちたら何も書かない（同じ文の A・同じ A を 2 回・直下でない A・child_Q のある Q）。
        let data = qid(e.by_key("data"));
        e.user(json!({"op": "add_q", "parent": data, "text": "子の問い"})).unwrap();
        let count = e.count("rt_nodes");
        e.approve("g2", "許可する");
        let same_text = json!({"op": "regroup", "q": qid(e.sub), "text": "側頭葉てんかん", "child_q": "別の問い", "members": [temporal]});
        assert!(err(e.agent(&[same_text])).contains("同じ文の A"));
        let twice = json!({"op": "regroup", "q": qid(e.sub), "text": "新しい中間", "child_q": "別の問い", "members": [temporal, temporal]});
        assert!(err(e.agent(&[twice])).contains("同じ A が 2 回"));
        let not_direct = json!({"op": "regroup", "q": qid(e.by_key("output")), "text": "x", "child_q": "y", "members": [temporal]});
        assert!(err(e.agent(&[not_direct])).contains("直下の A ではありません"));
        // child_Q を持つ Q（data）には regroup できない。
        let has_child_q = json!({"op": "regroup", "q": data, "text": "x", "child_q": "y", "members": [temporal]});
        assert!(err(e.agent(&[has_child_q])).contains("child_Q があるので"));
        assert_eq!(e.count("rt_nodes"), count, "何も増えていない");
    }

    /// seed の問いへの回答の決まり（承認・D で答えた問い・対象外・消した問い・回答の直し）。
    #[test]
    fn seed_question_rules_for_answers() {
        let e = env();
        let seed = e.seed();
        let (task, target) = (e.sq(&seed, "task"), e.sq(&seed, "target"));
        let a = e.a("A");
        // 問いの追加・直し・消す・回答を消すは agent に承認が要る（承認が無ければ終了コード 4）。
        assert!(conflict(e.agent1(json!({"op": "add_seed_q", "seed": seed, "text": "新しい問い"}))).contains("許可がありません"));
        assert!(conflict(e.agent1(json!({"op": "edit_seed_q", "q": task, "text": "直す"}))).contains("許可がありません"));
        assert!(conflict(e.agent1(json!({"op": "remove_seed_q", "q": target}))).contains("許可がありません"));
        // 回答の追加・直し（テキスト）は agent でも出せる。
        let d_answer = e.agent1(json!({"op": "add_answer", "to": seed, "q": task, "text": "発作部位の特定"})).unwrap();
        assert!(d_answer.starts_with("SA"), "{d_answer}");
        // 利用者は問いを足せる。未完成の S にだけ効く。
        let added = e.user(json!({"op": "add_seed_q", "seed": seed, "text": "新しい問い", "axis": ""})).unwrap().result;
        assert!(added.starts_with("SQ"));
        // S: D で答えた問い（task）には答えられない。
        let s = e.agent1(json!({"op": "add_sprout", "seed": seed, "title": "種"})).unwrap();
        assert!(err(e.agent1(json!({"op": "add_answer", "to": s, "q": task, "a": a}))).contains("seed で回答済みです"));
        // 消した問いには答えられない。D の回答は消した問いにも足せない。
        e.user(json!({"op": "remove_seed_q", "q": target})).unwrap();
        assert!(err(e.agent1(json!({"op": "add_answer", "to": s, "q": target, "text": "x"}))).contains("消した問いです"));
        // 回答の文は text か a のどちらか一方。テキストの回答は文を直せる。path の回答は直せない。
        assert!(err(e.agent1(json!({"op": "add_answer", "to": s, "q": e.sq(&seed, "method")}))).contains("どちらか一方"));
        let method = e.sq(&seed, "method");
        let path_answer = e.agent1(json!({"op": "add_answer", "to": s, "q": method, "a": a})).unwrap();
        let text_answer = e.agent1(json!({"op": "add_answer", "to": s, "q": e.sq(&seed, "output"), "text": "表"})).unwrap();
        assert!(err(e.agent1(json!({"op": "edit_answer", "answer": path_answer, "text": "直す"}))).contains("path の回答は remove_answer"));
        e.agent1(json!({"op": "edit_answer", "answer": text_answer, "text": "グラフ"})).unwrap();
        // 回答を消す（承認が要る）。消すと S は未完成に戻る。
        assert!(conflict(e.agent1(json!({"op": "remove_answer", "answer": text_answer}))).contains("許可がありません"));
        e.user(json!({"op": "remove_answer", "answer": text_answer})).unwrap();
        assert!(err(e.user(json!({"op": "remove_answer", "answer": text_answer}))).contains("消えています"));
    }

    /// S の paths は D と S の path の回答の和（問いの順）。D の path の回答は S の paths に入る。
    #[test]
    fn sprout_paths_are_seed_and_sprout_answers() {
        let e = env();
        let seed = e.seed();
        let a = e.a("A");
        let b = e.agent1(json!({"op": "add_a", "q": qid(e.by_key("target")), "text": "B"})).unwrap();
        let m = e.agent1(json!({"op": "add_a", "q": qid(e.by_key("method")), "text": "M"})).unwrap();
        let (_, bid) = parse_id(&b).unwrap();
        let (_, mid) = parse_id(&m).unwrap();
        let _ = a;
        // seed の回答（target の B）。S は method に M で答える。
        e.user(json!({"op": "add_answer", "to": seed, "q": e.sq(&seed, "target"), "a": b})).unwrap();
        let s = e.agent1(json!({"op": "add_sprout", "seed": seed, "title": "種", "answers": [{"q": e.sq(&seed, "method"), "a": m}]})).unwrap();
        let (_, sid) = parse_id(&s).unwrap();
        assert_eq!(store::get_node(e.conn(), sid).unwrap().paths, vec![bid, mid]);
        // 完成すると凍結される（問いの一覧を持つ）。
        assert!(store::get_node(e.conn(), sid).unwrap().frozen_qs.is_none(), "未完成");
    }

    /// 未完成の SPROUT には実験を足せない。完成すると足せる（承認は要らない）。
    #[test]
    fn sprout_must_be_complete_before_experiment() {
        let e = env();
        let a = e.a("A");
        let s = e.sprout(&e.seed(), &a);
        let err_text = err(e.agent1(json!({"op": "add_e", "parent": s, "text": "実験"})));
        assert!(err_text.contains(&format!("{s} は未完成です（未回答: SQ")), "{err_text}");
        assert!(err_text.contains("全部の問いに回答してから実験を足してください"));
        e.fill(&s);
        assert!(e.agent1(json!({"op": "add_e", "parent": s, "text": "実験"})).unwrap().starts_with('E'));
    }

    /// 完成した SPROUT は凍結する。後から D に問いを足しても完成のまま。回答を消すと未完成に戻り、凍結も外れる。
    #[test]
    fn complete_sprout_freezes_and_unfreezes() {
        let e = env();
        let seed = e.seed();
        let a = e.a("A");
        let s = e.sprout(&seed, &a);
        let (_, sid) = parse_id(&s).unwrap();
        let row = |e: &Env| store::get_node(e.conn(), sid).unwrap();
        assert!(row(&e).frozen_qs.is_none());
        // 未完成の S には、後から足す問いが missing に出る。
        let new_q = e.user(json!({"op": "add_seed_q", "seed": seed, "text": "後の問い"})).unwrap().result;
        assert!(seedq::is_complete(e.conn(), &row(&e)).unwrap().1.contains(&parse_sq(&new_q).unwrap()));
        e.fill(&s);
        assert!(row(&e).frozen_qs.is_some(), "完成して凍結");
        // 完成後に問いを足しても、完成のまま。
        e.user(json!({"op": "add_seed_q", "seed": seed, "text": "さらに後の問い"})).unwrap();
        assert!(seedq::is_complete(e.conn(), &row(&e)).unwrap().0);
        assert!(e.agent1(json!({"op": "add_e", "parent": s, "text": "実験"})).is_ok());
        // 回答（S の task の path）を消すと未完成に戻り、凍結が外れる。
        let answer: i64 = e.conn().query_row("SELECT id FROM rt_answers WHERE owner_id = ?1 ORDER BY id LIMIT 1", params![sid], |r| r.get(0)).unwrap();
        e.user(json!({"op": "remove_answer", "answer": fmt_sa(answer)})).unwrap();
        assert!(row(&e).frozen_qs.is_none());
        assert!(!seedq::is_complete(e.conn(), &row(&e)).unwrap().0);
    }

    /// 取り出し: status の axes と path_roles、steps の move_node と regroup（1 op = 1 step）。
    #[test]
    fn status_and_steps_show_questions_roles_and_new_ops() {
        let e = env();
        let method_q = qid(e.by_key("method"));
        let a94 = e.agent1(json!({"op": "add_a", "q": method_q, "text": "視床皮質モデル"})).unwrap();
        let child = e.a("A");
        let seed = e.agent1(json!({"op": "add_seed", "title": "研究", "body": "背景"})).unwrap();
        // D の method の問いに、A94 を path で答える（seed の回答＝固定）。
        e.user(json!({"op": "add_answer", "to": seed, "q": e.sq(&seed, "method"), "a": a94})).unwrap();
        let s = e.sprout(&seed, &child);
        let status = serde_json::to_value(crate::rt::compute::status(e.conn(), &e.roots, &e.defs).unwrap()).unwrap();
        let nodes = status["nodes"].as_array().unwrap();
        // D は問いを 7 問持つ（軸の 6 つと、成功基準の null）。complete・missing は出さない。
        let d = nodes.iter().find(|n| n["id"] == seed.as_str()).unwrap();
        let axes: Vec<Value> = d["questions"].as_array().unwrap().iter().map(|q| q["axis"].clone()).collect();
        assert_eq!(axes, vec![json!("task"), json!("target"), json!("data"), json!("method"), json!("output"), json!("metric"), Value::Null]);
        assert!(d["axes"].is_null() && d["complete"].is_null());
        // D の method の問いは、seed の回答（owner D1・path の A94）を持つ。
        let method = &d["questions"][3];
        assert_eq!(method["answers"][0]["owner"], seed.as_str());
        assert_eq!(method["answers"][0]["a"], a94.as_str());
        // S は完成していない（未回答の問いが missing）。paths の区分は回答から導く（method の A94 は D の回答＝固定、child は S の回答＝新規）。
        let sp = nodes.iter().find(|n| n["id"] == s.as_str()).unwrap();
        assert_eq!(sp["complete"], json!(false));
        assert_eq!(sp["missing"].as_array().unwrap().len(), 5, "target・data・output・metric・成功基準");
        assert_eq!(sp["path_roles"], json!([
            {"a": child, "axis": "task", "role": "novel", "question": sp["questions"][0]["id"]},
            {"a": a94, "axis": "method", "role": "fixed", "question": method["id"]},
        ]));
        // D と S 以外は questions が []、complete と path_roles は null・[]。
        let plain = nodes.iter().find(|n| n["id"] == child.as_str()).unwrap();
        assert!(plain["questions"] == json!([]) && plain["complete"].is_null() && plain["path_roles"] == json!([]));
        // move_node と regroup の step。regroup は作成（中間の A と細分の Q）も移動も 1 step。
        e.approve("s1", "許可する");
        e.agent1(json!({"op": "move_node", "node": child, "to": qid(e.by_key("target"))})).unwrap();
        e.approve("s2", "許可する");
        e.agent1(json!({"op": "regroup", "q": qid(e.by_key("target")), "text": "中間", "child_q": "細分の問い", "members": [child]})).unwrap();
        let steps = crate::rt::steps::build(e.conn()).unwrap();
        let ops: Vec<&str> = steps.list.iter().map(|s| s.op.as_str()).collect();
        assert!(ops.contains(&"move_node"), "{ops:?}");
        assert_eq!(ops.iter().filter(|o| **o == "regroup").count(), 1, "{ops:?}");
        let moved = steps.nodes.iter().find(|n| n.id == child).unwrap();
        assert_eq!(moved.parents.last().unwrap().parent.as_deref(), Some(qid(e.by_key("target")).as_str()));
    }
}
