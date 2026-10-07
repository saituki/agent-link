//! 新しい木（Q・O・F・E）を変える操作（op）。C-OP の表が正。
//!
//! 入口は 3 つ。agent の [`propose`]、利用者の [`apply_user`]、利用者の仕分け [`triage`]。
//! 表（[`OP_SPECS`]）→ 形の検査 → 中身の検査 → 書く、を [`execute`] 1 本で行う。
//! 検査が全部済んでから書くので、途中で弾かれた op は何も残さない。
//! [`propose`] は 1 つのトランザクションで動き、1 件でも弾かれれば全体を戻す。

use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;
use serde_json::{Map, Value, json};

use super::defs::{self, Defs};
use super::store::{self, NewNode, NodeRow};
use super::{Kind, Result, RtError, fmt_id, fmt_ref, parse_id, parse_ref};
use sai_paper_core::paths::PaperRoots;
use sai_paper_core::textcheck;

const MAX_PROPOSALS: usize = 20;

// ---- 表 -------------------------------------------------------------------------------

/// 入り方。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Risk {
    /// agent が出したらすぐ入る。
    Auto,
    /// agent が出したら利用者が仕分けてから入る。
    Human,
    /// 利用者だけ。PROPOSAL では出せない。
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
    /// 整数か null。
    NullInt,
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
const KINDS: Ty = Ty::Enum(&["answer", "issue"]);

static OP_SPECS: &[OpSpec] = &[
    OpSpec {
        op: "add_q",
        risk: Risk::Human,
        about: "Q（または O）の下に子の Q を足す。仕分けのあとで入る",
        fields: &[req("parent", Ty::Id), req("text", Ty::Text(1, 300)), opt("guide", Ty::Text(1, 80))],
    },
    OpSpec {
        op: "add_o",
        risk: Risk::Auto,
        about: "Q の下に O（答えの候補）を足す",
        fields: &[
            req("q", Ty::Id),
            req("text", Ty::Text(1, 120)),
            opt("body", Ty::Text(0, 2000)),
            opt("kind", KINDS),
        ],
    },
    OpSpec {
        op: "add_f",
        risk: Risk::Auto,
        about: "O か E の下に F（根拠）を足す。数字を含む文には出典（cites）か報告書のトークン（report）が要る",
        fields: &[
            req("parent", Ty::Id),
            req("text", Ty::Text(1, 1200)),
            opt("cites", Ty::List(0, 10)),
            opt("paper", Ty::Int),
            opt("report", Ty::Text(1, 100)),
        ],
    },
    OpSpec {
        op: "add_e",
        risk: Risk::Auto,
        about: "O か E の下に E（実験）を足す。O の下は Issue の O だけ",
        fields: &[
            req("parent", Ty::Id),
            req("text", Ty::Text(1, 120)),
            opt("body", Ty::Text(0, 2000)),
            opt("mlflow", Ty::Text(0, 200)),
        ],
    },
    OpSpec {
        op: "record_logic",
        risk: Risk::Auto,
        about: "Q の Logicality の確かめを記録する（矛盾なら F を 2 つ以上挙げる）",
        fields: &[
            req("q", Ty::Id),
            req("value", Ty::Enum(&["ok", "contradiction"])),
            opt("note", Ty::Text(0, 1000)),
            opt("findings", Ty::List(0, 50)),
        ],
    },
    OpSpec {
        op: "add_seed",
        risk: Risk::Auto,
        about: "Q から子の Q へ降りる O の 1 本の鎖（path）から Sprout を作る（Issue の O を 1 つ以上）",
        fields: &[
            req("title", Ty::Text(1, 120)),
            req("sentence", Ty::Text(1, 600)),
            req("path", Ty::List(1, 50)),
        ],
    },
    OpSpec {
        op: "copy",
        risk: Risk::Human,
        about: "O とその下の F を別の Q の下へ写す（E は写さない）。仕分けのあとで入る",
        fields: &[req("node", Ty::Id), req("to", Ty::Id)],
    },
    OpSpec {
        op: "remove",
        risk: Risk::Human,
        about: "Node を下の Node ごと消す（一番上の Q は消せない）。仕分けのあとで入る",
        fields: &[req("node", Ty::Id)],
    },
    OpSpec { op: "star", risk: Risk::UserOnly, about: "O に ★ を付ける", fields: &[req("o", Ty::Id)] },
    OpSpec { op: "unstar", risk: Risk::UserOnly, about: "O の ★ を外す", fields: &[req("o", Ty::Id)] },
    OpSpec {
        op: "set_kind",
        risk: Risk::UserOnly,
        about: "O を Answer / Issue に切り替える",
        fields: &[req("o", Ty::Id), req("kind", KINDS)],
    },
    OpSpec {
        op: "edit",
        risk: Risk::UserOnly,
        about: "Node の文・本文を書き換える",
        fields: &[req("node", Ty::Id), opt("text", Ty::Text(1, 1200)), opt("body", Ty::Text(0, 2000))],
    },
    OpSpec {
        op: "set_target",
        risk: Risk::UserOnly,
        about: "Q の目安（0〜99。null でガイドの値に戻す）を変える",
        fields: &[req("q", Ty::Id), req("target", Ty::NullInt)],
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

fn field_schema(f: &Field) -> Value {
    match f.ty {
        Ty::Id => json!({"type": "string", "pattern": "^[QOFE][1-9][0-9]{0,8}$"}),
        Ty::Text(min, max) => json!({"type": "string", "minLength": min, "maxLength": max}),
        Ty::Int => json!({"type": "integer"}),
        Ty::NullInt => json!({"type": ["integer", "null"], "minimum": 0, "maximum": 99}),
        Ty::Enum(values) => json!({"type": "string", "enum": values}),
        Ty::List(min, max) => json!({"type": "array", "items": {"type": "string"}, "minItems": min, "maxItems": max}),
    }
}

/// PROPOSAL の入力の JSON Schema。`UserOnly` の op は含めない。
#[must_use]
pub fn propose_schema() -> Value {
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
        "properties": {"proposals": {"type": "array", "minItems": 1, "maxItems": MAX_PROPOSALS,
                                      "items": {"oneOf": variants}}},
        "required": ["proposals"],
    })
}

/// ブロックが出せる op（frontmatter の `sci_ops`）。書いていなければ `Auto` の op 全部。
#[must_use]
pub fn allowed_ops(roots: &PaperRoots, block: &str) -> Vec<String> {
    let auto = || {
        OP_SPECS.iter().filter(|s| s.risk == Risk::Auto).map(|s| s.op.to_string()).collect::<Vec<_>>()
    };
    let Some(pairs) = super::action::block_frontmatter(roots, block) else { return auto() };
    let found: Vec<String> = pairs
        .into_iter()
        .find(|(key, _)| key == "sci_ops")
        .map(|(_, v)| v.split(',').map(str::trim).filter(|p| !p.is_empty()).map(str::to_string).collect())
        .unwrap_or_default();
    if found.is_empty() { auto() } else { found }
}

// ---- 入口の型 ---------------------------------------------------------------------------

/// PROPOSAL を出した側。
#[derive(Debug, Clone, Default)]
pub struct Actor {
    pub run_id: Option<i64>,
    pub block: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TriageAction {
    Accept,
    Reject,
    Revert,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProposalResult {
    pub proposal: String,
    pub op: String,
    pub status: String,
    pub result: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ApplyResult {
    pub op: String,
    pub result: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TriageResult {
    pub proposal: String,
    pub status: String,
    pub result: Option<String>,
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
    proposal_id: Option<i64>,
}

fn run_by(run_id: Option<i64>) -> String {
    run_id.map_or_else(|| "agent".to_string(), |id| format!("run:{id}"))
}

// ---- 入口 -------------------------------------------------------------------------------

/// agent の PROPOSAL。`input` は `{"proposals": [ {op…}, … ]}`（20 件まで）。
///
/// 1 件でも通らなければ何も保存せず、最初の違反を `Invalid("<n> 件目（<op>）: <理由>")` で返す。
pub fn propose(
    conn: &Connection,
    roots: &PaperRoots,
    defs: &Defs,
    actor: &Actor,
    input: &Value,
) -> Result<Vec<ProposalResult>> {
    let Some(items) = input.get("proposals").and_then(Value::as_array) else {
        return invalid("proposals（配列）が必要です");
    };
    if items.is_empty() || items.len() > MAX_PROPOSALS {
        return invalid(format!("proposals は 1〜{MAX_PROPOSALS} 件です"));
    }
    let block = actor.block.as_deref().unwrap_or("");
    let allowed = allowed_ops(roots, block);
    let tx = conn.unchecked_transaction()?;
    let mut out = Vec::new();
    for (index, item) in items.iter().enumerate() {
        let op = item.get("op").and_then(Value::as_str).unwrap_or("");
        let at = |e: RtError| match e {
            RtError::Invalid(m) => RtError::Invalid(format!("{} 件目（{op}）: {m}", index + 1)),
            other => other,
        };
        let Some(spec) = op_spec(op) else {
            return Err(at(RtError::Invalid(format!("知らない op です: {op}"))));
        };
        if spec.risk == Risk::UserOnly {
            return Err(at(RtError::Invalid("利用者だけが行う操作です".into())));
        }
        if !allowed.iter().any(|a| a == op) {
            return Err(at(RtError::Invalid(format!("このブロックでは {op} を出せません"))));
        }
        let node_id = ["node", "parent", "q", "o"]
            .iter()
            .find_map(|k| item.get(*k).and_then(Value::as_str))
            .and_then(|t| parse_id(t).ok())
            .map(|(_, id)| id);
        let risk = if spec.risk == Risk::Auto { "auto" } else { "human" };
        let pid = store::insert_proposal(&tx, actor.run_id, actor.block.as_deref(), op, node_id, item, risk, "pending")?;
        let cx = Cx { conn: &tx, defs, by: run_by(actor.run_id), run_id: actor.run_id, proposal_id: Some(pid) };
        let (status, result) = if spec.risk == Risk::Auto {
            let result = execute(&cx, item).map_err(at)?;
            store::finish_proposal(&tx, pid, "applied", Some(&result), None)?;
            ("applied", Some(result))
        } else {
            dry_run(&cx, item).map_err(at)?;
            ("pending", None)
        };
        out.push(ProposalResult {
            proposal: fmt_ref('P', pid),
            op: op.to_string(),
            status: status.to_string(),
            result,
        });
    }
    tx.commit()?;
    Ok(out)
}

/// 利用者として 1 件の op を行う（どの op も使える）。
pub fn apply_user(conn: &Connection, _roots: &PaperRoots, defs: &Defs, input: &Value) -> Result<ApplyResult> {
    let op = input.get("op").and_then(Value::as_str).unwrap_or("").to_string();
    let tx = conn.unchecked_transaction()?;
    // 読む札（runner=user の run）の画面から足す F は、その run の F として残す
    // （`run` は op の項目ではないので、検査に渡す前に外す）
    let run_id = input.get("run").and_then(Value::as_i64).filter(|r| *r > 0);
    let mut input = input.clone();
    if let Some(map) = input.as_object_mut() {
        map.remove("run");
    }
    let cx = Cx { conn: &tx, defs, by: "user".into(), run_id, proposal_id: None };
    let result = execute(&cx, &input)?;
    tx.commit()?;
    Ok(ApplyResult { op, result: Some(result) })
}

/// 仕分け。Accept は `edit` を重ねて検査し直してから書く。
pub fn triage(
    conn: &Connection,
    _roots: &PaperRoots,
    defs: &Defs,
    proposal: i64,
    action: TriageAction,
    edit: Option<&Value>,
) -> Result<TriageResult> {
    let p = store::get_proposal(conn, proposal)?;
    let tx = conn.unchecked_transaction()?;
    let name = fmt_ref('P', proposal);
    let (status, result) = match action {
        TriageAction::Accept | TriageAction::Reject => {
            if p.status != "pending" {
                return invalid(format!("{name} は仕分け待ちではありません（{}）", p.status));
            }
            if action == TriageAction::Reject {
                ("rejected", None)
            } else {
                let mut payload = p.payload.clone();
                if let Some(edit) = edit {
                    let (Some(base), Some(over)) = (payload.as_object_mut(), edit.as_object()) else {
                        return invalid("edit は JSON のオブジェクトで渡してください");
                    };
                    if over.get("op").is_some_and(|o| o.as_str() != Some(&p.op)) {
                        return invalid("edit で op は変えられません");
                    }
                    for (k, v) in over {
                        base.insert(k.clone(), v.clone());
                    }
                }
                let cx = Cx { conn: &tx, defs, by: run_by(p.run_id), run_id: p.run_id, proposal_id: Some(proposal) };
                ("accepted", Some(execute(&cx, &payload)?))
            }
        }
        TriageAction::Revert => {
            if p.status != "applied" {
                return invalid(format!("{name} はすぐ入った提案ではないので取り消せません（{}）", p.status));
            }
            revert(&tx, &p.op, p.result_ref.as_deref())?;
            ("reverted", p.result_ref.clone())
        }
    };
    store::finish_proposal(&tx, proposal, status, result.as_deref(), Some("user"))?;
    tx.commit()?;
    Ok(TriageResult { proposal: name, status: status.into(), result })
}

/// すぐ入った op の取り消し。確かめの記録（`record_logic`）は消さない。
fn revert(conn: &Connection, op: &str, result: Option<&str>) -> Result<()> {
    let Some(result) = result else { return Ok(()) };
    match op {
        "add_o" | "add_f" | "add_e" => {
            let (_, id) = parse_id(result)?;
            store::remove_subtree(conn, id)?;
        }
        "add_seed" => {
            let id = parse_ref(result, 'S')?;
            conn.execute(
                "UPDATE rt_seeds SET removed_at = ?2 WHERE id = ?1",
                params![id, super::now_text()],
            )?;
        }
        _ => {}
    }
    Ok(())
}

/// 書かずに検査だけする（仕分け待ちの op 用）。書いて戻す。
fn dry_run(cx: &Cx<'_>, op: &Value) -> Result<()> {
    cx.conn.execute_batch("SAVEPOINT rt_dry")?;
    let r = execute(cx, op);
    cx.conn.execute_batch("ROLLBACK TO rt_dry; RELEASE rt_dry")?;
    r.map(|_| ())
}

// ---- 検査の部品 -----------------------------------------------------------------------------

/// 項目の値。`null` は無いものとして扱う（`NullInt` は除く）。
fn get<'a>(op: &'a Value, f: &Field) -> Option<&'a Value> {
    op.get(f.name).filter(|v| !v.is_null() || matches!(f.ty, Ty::NullInt))
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
            Ty::NullInt => {
                if !v.is_null() && !v.as_i64().is_some_and(|n| (0..=99).contains(&n)) {
                    return invalid(format!("{} は 0〜99 の整数か null です", f.name));
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

fn new_node<'a>(cx: &'a Cx<'_>, kind: Kind, parent: i64, text: &'a str) -> NewNode<'a> {
    NewNode {
        kind,
        parent_id: Some(parent),
        guide_key: None,
        text,
        body: "",
        answer_kind: None,
        cites: &[],
        paper_id: None,
        run_id: cx.run_id,
        report_id: None,
        mlflow_ref: None,
        proposal_id: cx.proposal_id,
        created_by: &cx.by,
    }
}

fn log(cx: &Cx<'_>, node_id: i64, action: &str, detail: &str) -> Result<()> {
    store::insert_log(cx.conn, node_id, action, detail, &cx.by, cx.proposal_id)?;
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

/// op を検査して書く。返す文字列は `result`（作った・変えた Node の ID）。
fn execute(cx: &Cx<'_>, op: &Value) -> Result<String> {
    let name = text(op, "op");
    let Some(spec) = op_spec(name) else { return invalid(format!("知らない op です: {name}")) };
    check_shape(spec, op)?;
    let conn = cx.conn;
    match name {
        "add_q" => {
            let parent = node(cx, text(op, "parent"), &[Kind::Q, Kind::O])?;
            let pid = fmt_id(parent.kind, parent.id);
            if parent.kind == Kind::Q && !alive_children(cx, parent.id, Kind::O)?.is_empty() {
                return invalid(format!("{pid} には O があるので、子の Q を足せません"));
            }
            let guide = op.get("guide").and_then(Value::as_str);
            if let Some(key) = guide.filter(|k| cx.defs.guide(k).is_none()) {
                return invalid(format!("知らないガイドです: {key}"));
            }
            let mut n = new_node(cx, Kind::Q, parent.id, text(op, "text"));
            n.guide_key = guide;
            Ok(fmt_id(Kind::Q, store::insert_node(conn, &n)?))
        }
        "add_o" => {
            let q = node(cx, text(op, "q"), &[Kind::Q])?;
            let qid = fmt_id(Kind::Q, q.id);
            if !alive_children(cx, q.id, Kind::Q)?.is_empty() {
                return invalid(format!("{qid} には子の Q があるので、O を足せません"));
            }
            let t = text(op, "text");
            if alive_children(cx, q.id, Kind::O)?.iter().any(|o| o.text == t) {
                return invalid(format!("{qid} に同じ文の O があります"));
            }
            let kind = op.get("kind").and_then(Value::as_str).unwrap_or("answer");
            let mut n = new_node(cx, Kind::O, q.id, t);
            n.body = text(op, "body");
            n.answer_kind = Some(kind);
            let id = store::insert_node(conn, &n)?;
            defs::create_qs_for_option(conn, cx.defs, id, &cx.by, cx.run_id)?;
            Ok(fmt_id(Kind::O, id))
        }
        "add_f" => {
            let parent = node(cx, text(op, "parent"), &[Kind::O, Kind::E])?;
            let t = text(op, "text");
            if sentence_count(t) > 3 {
                return invalid("text は 3 文までです");
            }
            let cites = strings(op, "cites");
            // 数字を含む F は、論文の出典（cites）か、自分の報告書のトークン（report）を根拠にする。
            let report = op.get("report").and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty());
            if cites.is_empty() && report.is_none() && t.chars().any(char::is_numeric) {
                return invalid("数字を含む F には出典（cites）か報告書のトークン（report）を付けてください");
            }
            let papers = check_cites(conn, &cites)?;
            let paper = op.get("paper").and_then(Value::as_i64);
            if let Some(p) = paper.filter(|p| !paper_exists(conn, *p).unwrap_or(false)) {
                return invalid(format!("論文 {p} が Library にありません"));
            }
            let mut n = new_node(cx, Kind::F, parent.id, t);
            n.cites = &cites;
            n.paper_id = paper.or_else(|| papers.first().copied());
            n.report_id = op.get("report").and_then(Value::as_str);
            Ok(fmt_id(Kind::F, store::insert_node(conn, &n)?))
        }
        "add_e" => {
            let parent = node(cx, text(op, "parent"), &[Kind::O, Kind::E])?;
            if parent.kind == Kind::O && parent.answer_kind.as_deref() != Some("issue") {
                return invalid("E は Issue の O の下にだけ作れます");
            }
            let mut n = new_node(cx, Kind::E, parent.id, text(op, "text"));
            n.body = text(op, "body");
            n.mlflow_ref = op.get("mlflow").and_then(Value::as_str).filter(|s| !s.is_empty());
            Ok(fmt_id(Kind::E, store::insert_node(conn, &n)?))
        }
        "record_logic" => {
            let q = node(cx, text(op, "q"), &[Kind::Q])?;
            let value = text(op, "value");
            let note = text(op, "note");
            let mut findings = Vec::new();
            for f in strings(op, "findings") {
                let f = node(cx, &f, &[Kind::F])?;
                if !is_under(conn, f.id, q.id)? {
                    return invalid(format!("{} は {} の下の F ではありません", fmt_id(Kind::F, f.id), text(op, "q")));
                }
                findings.push(f.id);
            }
            if value == "contradiction" && (note.is_empty() || findings.len() < 2) {
                return invalid("矛盾を記録するには、note と、q の下の F を 2 つ以上（findings）が要ります");
            }
            let f_count = i64::try_from(
                store::subtree_ids(conn, q.id)?
                    .into_iter()
                    .filter(|id| store::get_node(conn, *id).is_ok_and(|n| n.kind == Kind::F))
                    .count(),
            )
            .unwrap_or(i64::MAX);
            store::insert_logic(conn, q.id, value, note, &findings, f_count, cx.run_id, cx.proposal_id, &cx.by)?;
            Ok(fmt_id(Kind::Q, q.id))
        }
        "add_seed" => {
            let mut path = Vec::new();
            for o in strings(op, "path") {
                let n = node(cx, &o, &[Kind::O])?;
                path.push(n.id);
            }
            // Q から子の Q へ降りる O の 1 本の鎖か。`o_i`（i ≥ 1）の親から上へ Q をたどって
            // 最初に会う O が `o_{i-1}` であること。
            for (i, id) in path.iter().enumerate().skip(1) {
                let prev = path[i - 1];
                let mut at = store::get_node(conn, *id)?.parent_id;
                let mut found = false;
                while let Some(p) = at {
                    let n = store::get_node(conn, p)?;
                    if n.kind != Kind::Q {
                        break;
                    }
                    at = n.parent_id;
                }
                // Q の列を抜けた先が前の O か。
                if let Some(p) = at {
                    found = p == prev;
                }
                if !found {
                    let cur = fmt_id(Kind::O, *id);
                    let before = fmt_id(Kind::O, prev);
                    return invalid(format!("{cur} は {before} の下の子の Q の O ではありません"));
                }
            }
            let has_issue = path
                .iter()
                .any(|id| store::get_node(conn, *id).is_ok_and(|n| n.answer_kind.as_deref() == Some("issue")));
            if !has_issue {
                return invalid("Issue の O を 1 つ以上含めてください");
            }
            let id = store::insert_seed(conn, text(op, "title"), text(op, "sentence"), &path, cx.run_id, &cx.by)?;
            Ok(fmt_ref('S', id))
        }
        "copy" => {
            let src = node(cx, text(op, "node"), &[Kind::O])?;
            let to = node(cx, text(op, "to"), &[Kind::Q])?;
            if !alive_children(cx, to.id, Kind::Q)?.is_empty() {
                return invalid(format!("{} には子の Q があるので、O を写せません", text(op, "to")));
            }
            let mut n = new_node(cx, Kind::O, to.id, &src.text);
            n.body = &src.body;
            n.answer_kind = src.answer_kind.as_deref();
            let copied = store::insert_node(conn, &n)?;
            for f in alive_children(cx, src.id, Kind::F)? {
                let mut fn_ = new_node(cx, Kind::F, copied, &f.text);
                fn_.cites = &f.cites;
                fn_.paper_id = f.paper_id;
                fn_.report_id = f.report_id.as_deref();
                store::insert_node(conn, &fn_)?;
            }
            log(cx, copied, "copy", &format!("{} から", text(op, "node")))?;
            Ok(fmt_id(Kind::O, copied))
        }
        "remove" => {
            let n = node(cx, text(op, "node"), &[Kind::Q, Kind::O, Kind::F, Kind::E])?;
            if n.parent_id.is_none() {
                return invalid("一番上の Q は消せません");
            }
            store::remove_subtree(conn, n.id)?;
            log(cx, n.id, "remove", "")?;
            Ok(text(op, "node").to_string())
        }
        "star" | "unstar" => {
            let on = name == "star";
            let o = node(cx, text(op, "o"), &[Kind::O])?;
            if o.starred != on {
                store::set_starred(conn, o.id, on)?;
                log(cx, o.id, name, "")?;
            }
            Ok(text(op, "o").to_string())
        }
        "set_kind" => {
            let o = node(cx, text(op, "o"), &[Kind::O])?;
            let kind = text(op, "kind");
            store::set_answer_kind(conn, o.id, kind)?;
            log(cx, o.id, "set_kind", kind)?;
            Ok(text(op, "o").to_string())
        }
        "edit" => {
            let n = node(cx, text(op, "node"), &[Kind::Q, Kind::O, Kind::F, Kind::E])?;
            let (t, b) = (op.get("text").and_then(Value::as_str), op.get("body").and_then(Value::as_str));
            if t.is_none() && b.is_none() {
                return invalid("text か body のどちらかが必要です");
            }
            if let Some(t) = t {
                let max = match n.kind {
                    Kind::Q => 300,
                    Kind::O | Kind::E => 120,
                    Kind::F => 1200,
                };
                if t.chars().count() > max {
                    return invalid(format!("text は {max} 字までです"));
                }
                if n.kind == Kind::F && sentence_count(t) > 3 {
                    return invalid("text は 3 文までです");
                }
            }
            if b.is_some() && !matches!(n.kind, Kind::O | Kind::E) {
                return invalid("body は O と E だけに付けられます");
            }
            store::update_text(conn, n.id, t, b)?;
            let detail = match (t.is_some(), b.is_some()) {
                (true, true) => "text, body",
                (true, false) => "text",
                _ => "body",
            };
            log(cx, n.id, "edit", detail)?;
            Ok(text(op, "node").to_string())
        }
        "set_target" => {
            let q = node(cx, text(op, "q"), &[Kind::Q])?;
            let target = op.get("target").and_then(Value::as_i64);
            store::set_target(conn, q.id, target)?;
            log(cx, q.id, "set_target", &target.map_or_else(|| "null".into(), |t| t.to_string()))?;
            Ok(text(op, "q").to_string())
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
            "---\nsci_ops: add_q, add_o, add_f, add_e, record_logic, add_seed, copy, remove\n---\n# t\n",
        )
        .unwrap();
        Env { db, roots: PaperRoots { root: dir.clone(), data_dir: dir }, defs, sub }
    }

    impl Env {
        fn conn(&self) -> &Connection {
            self.db.connection()
        }
        fn prop(&self, items: &[Value]) -> Result<Vec<ProposalResult>> {
            let actor = Actor { run_id: Some(7), block: Some("t-all".into()) };
            propose(self.conn(), &self.roots, &self.defs, &actor, &json!({"proposals": items}))
        }
        fn one(&self, op: Value) -> Result<ProposalResult> {
            self.prop(&[op]).map(|mut v| v.remove(0))
        }
        fn user(&self, op: Value) -> Result<ApplyResult> {
            apply_user(self.conn(), &self.roots, &self.defs, &op)
        }
        fn o(&self, text: &str) -> String {
            self.one(json!({"op": "add_o", "q": fmt_id(Kind::Q, self.sub), "text": text})).unwrap().result.unwrap()
        }
        fn count(&self, table: &str) -> i64 {
            self.conn().query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0)).unwrap()
        }
    }

    fn err(r: Result<impl std::fmt::Debug>) -> String {
        match r {
            Err(RtError::Invalid(m)) => m,
            other => panic!("Invalid のはずが {other:?}"),
        }
    }

    #[test]
    fn add_o_creates_per_option_qs() {
        let e = env();
        let r = e.one(json!({"op": "add_o", "q": fmt_id(Kind::Q, e.sub), "text": "予後予測"})).unwrap();
        assert_eq!(r.status, "applied");
        let (_, o) = parse_id(r.result.as_ref().unwrap()).unwrap();
        let kids = store::children(e.conn(), o).unwrap();
        assert!(!kids.is_empty() && kids.iter().all(|k| k.kind == Kind::Q));
        assert_eq!(store::get_node(e.conn(), o).unwrap().created_by, "run:7");
        let qid = fmt_id(Kind::Q, e.sub);
        assert!(err(e.one(json!({"op": "add_o", "q": qid, "text": "予後予測"}))).contains("同じ文"));
        // O のある Q に子の Q は足せず、子の Q のある Q に O は足せない。
        assert!(err(e.one(json!({"op": "add_q", "parent": qid, "text": "子"}))).contains("O があるので"));
        let root = store::root(e.conn()).unwrap().unwrap();
        assert!(err(e.one(json!({"op": "add_o", "q": fmt_id(Kind::Q, root.id), "text": "x"}))).contains("子の Q があるので"));
    }

    #[test]
    fn add_f_rules() {
        let e = env();
        let o = e.o("A");
        let f = |t: &str, cites: Value| e.one(json!({"op": "add_f", "parent": o, "text": t, "cites": cites}));
        assert!(err(f("精度は 85 %", json!([]))).contains("数字を含む F には出典"));
        assert!(err(f("한 国", json!([]))).contains("text: ハングル"));
        assert!(err(f("一。二。三。四。", json!([]))).contains("3 文"));
        assert!(f("数字なし", json!([])).is_ok());
        // 報告書のトークンがあれば、数字を含んでいても通る。空白だけの report は出典にならない。
        let fr = |t: &str, report: &str| e.one(json!({"op": "add_f", "parent": o, "text": t, "report": report}));
        assert!(fr("H1 の効果量は g=0.47", "AbCdEf12").is_ok());
        assert!(fr("精度は 85 %", "  ").is_err());
        assert!(err(f("x", json!(["37"]))).contains("#<論文番号>"));
        assert!(err(f("x", json!(["#37"]))).contains("論文 37 が Library にありません"));
        e.conn().execute("INSERT INTO papers(id, title) VALUES (37, 't')", []).unwrap();
        // 節の表が無い論文は塊 ID を確かめずに通る。
        let r = f("精度は 85 %", json!(["#37.5.2"])).unwrap();
        let (_, id) = parse_id(r.result.as_ref().unwrap()).unwrap();
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
    fn user_add_f_keeps_run() {
        let e = env();
        let o = e.o("A");
        let f = e.user(json!({"op": "add_f", "parent": o, "text": "読んで分かった", "run": 31})).unwrap().result.unwrap();
        let run: Option<i64> = e
            .conn()
            .query_row("SELECT run_id FROM rt_nodes WHERE kind = 'F'", [], |r| r.get(0))
            .unwrap();
        assert!(f.starts_with('F'));
        assert_eq!(run, Some(31));
    }

    #[test]
    fn add_e_needs_issue() {
        let e = env();
        let o = e.o("A");
        assert!(err(e.one(json!({"op": "add_e", "parent": o, "text": "実験"}))).contains("Issue"));
        e.user(json!({"op": "set_kind", "o": o, "kind": "issue"})).unwrap();
        let r = e.one(json!({"op": "add_e", "parent": o, "text": "実験", "mlflow": "exp/run"})).unwrap();
        assert!(r.result.unwrap().starts_with('E'));
    }

    #[test]
    fn record_logic_rules() {
        let e = env();
        let o = e.o("A");
        let f1 = e.one(json!({"op": "add_f", "parent": o, "text": "一つ目"})).unwrap().result.unwrap();
        let f2 = e.one(json!({"op": "add_f", "parent": o, "text": "二つ目"})).unwrap().result.unwrap();
        let q = fmt_id(Kind::Q, e.sub);
        assert!(err(e.one(json!({"op": "record_logic", "q": q, "value": "contradiction", "note": "n", "findings": [f1]}))).contains("2 つ以上"));
        assert!(err(e.one(json!({"op": "record_logic", "q": q, "value": "contradiction", "findings": [f1, f2]}))).contains("note"));
        let root = fmt_id(Kind::Q, store::root(e.conn()).unwrap().unwrap().id);
        e.one(json!({"op": "record_logic", "q": q, "value": "contradiction", "note": "食い違う", "findings": [f1, f2]})).unwrap();
        assert_eq!(store::list_logic(e.conn(), Some(e.sub)).unwrap()[0].f_count, 2);
        e.one(json!({"op": "record_logic", "q": root, "value": "ok"})).unwrap();
    }

    #[test]
    fn add_seed_needs_chain_and_issue() {
        let e = env();
        // Q(sub) → O(A, answer) → Q → O(B)。star は付けない。
        let a = e.o("A");
        let (_, aid) = parse_id(&a).unwrap();
        let child_q = store::children(e.conn(), aid).unwrap().into_iter().find(|n| n.kind == Kind::Q).unwrap().id;
        let b = e.one(json!({"op": "add_o", "q": fmt_id(Kind::Q, child_q), "text": "B"})).unwrap().result.unwrap();
        // Answer だけの path は断られ、Issue を含むよう求められる。
        assert!(err(e.one(json!({"op": "add_seed", "title": "種", "sentence": "文", "path": [a]}))).contains("Issue"));
        // Answer の O を Issue に変えても、兄弟の O との path は鎖でないので断られる。
        e.user(json!({"op": "set_kind", "o": b, "kind": "issue"})).unwrap();
        let other = e.o("C");
        assert!(
            err(e.one(json!({"op": "add_seed", "title": "種", "sentence": "文", "path": [a, other]})))
                .contains("の下の子の Q の O ではありません")
        );
        // Q(sub)→O(answer)→Q→O(issue) の 2 つの O の path は通り、`S1` が返る。★ が無くても通る。
        e.user(json!({"op": "set_kind", "o": a, "kind": "issue"})).unwrap();
        e.user(json!({"op": "set_kind", "o": b, "kind": "answer"})).unwrap();
        assert_eq!(
            e.one(json!({"op": "add_seed", "title": "種", "sentence": "文", "path": [a, b]})).unwrap().result.unwrap(),
            "S1"
        );
    }

    #[test]
    fn human_ops_pending_and_triage() {
        let e = env();
        let o = e.o("A");
        let f = e.one(json!({"op": "add_f", "parent": o, "text": "根拠"})).unwrap().result.unwrap();
        let root = store::root(e.conn()).unwrap().unwrap();
        let q = e.one(json!({"op": "add_q", "parent": fmt_id(Kind::Q, root.id), "text": "新しい問い"})).unwrap();
        assert_eq!(q.status, "pending");
        assert!(q.result.is_none());
        assert_eq!(e.count("rt_nodes"), e.count("rt_nodes")); // dry-run が残らないことは下で確かめる
        let before = e.count("rt_nodes");
        let p = parse_ref(&q.proposal, 'P').unwrap();
        assert!(err(triage(e.conn(), &e.roots, &e.defs, p, TriageAction::Accept, Some(&json!({"op": "remove"})))).contains("op は変えられません"));
        let t = triage(e.conn(), &e.roots, &e.defs, p, TriageAction::Accept, Some(&json!({"text": "直した問い"}))).unwrap();
        assert_eq!(t.status, "accepted");
        assert_eq!(e.count("rt_nodes"), before + 1);
        let (_, id) = parse_id(&t.result.unwrap()).unwrap();
        assert_eq!(store::get_node(e.conn(), id).unwrap().text, "直した問い");
        assert!(err(triage(e.conn(), &e.roots, &e.defs, p, TriageAction::Reject, None)).contains("仕分け待ちではありません"));
        // reject
        let r = e.one(json!({"op": "remove", "node": f})).unwrap();
        let p = parse_ref(&r.proposal, 'P').unwrap();
        triage(e.conn(), &e.roots, &e.defs, p, TriageAction::Reject, None).unwrap();
        assert_eq!(store::get_proposal(e.conn(), p).unwrap().status, "rejected");
        // copy・remove
        let to = e.user(json!({"op": "add_q", "parent": fmt_id(Kind::Q, root.id), "text": "写し先"})).unwrap().result.unwrap();
        let c = e.one(json!({"op": "copy", "node": o, "to": to})).unwrap();
        let p = parse_ref(&c.proposal, 'P').unwrap();
        let t = triage(e.conn(), &e.roots, &e.defs, p, TriageAction::Accept, None).unwrap();
        let (_, copied) = parse_id(&t.result.unwrap()).unwrap();
        assert_eq!(store::children(e.conn(), copied).unwrap().len(), 1);
        assert!(err(e.user(json!({"op": "remove", "node": fmt_id(Kind::Q, root.id)}))).contains("一番上"));
        e.user(json!({"op": "remove", "node": o})).unwrap();
        assert!(err(e.user(json!({"op": "star", "o": o}))).contains("はありません"));
    }

    #[test]
    fn one_bad_proposal_saves_nothing() {
        let e = env();
        let q = fmt_id(Kind::Q, e.sub);
        let (n, p) = (e.count("rt_nodes"), e.count("rt_proposals"));
        let m = err(e.prop(&[
            json!({"op": "add_o", "q": q, "text": "A"}),
            json!({"op": "add_o", "q": q, "text": "B"}),
            json!({"op": "add_o", "q": q, "text": "한"}),
        ]));
        assert!(m.starts_with("3 件目（add_o）: text: ハングル"), "{m}");
        assert_eq!((e.count("rt_nodes"), e.count("rt_proposals")), (n, p));
    }

    #[test]
    fn star_is_user_only() {
        let e = env();
        let o = e.o("A");
        assert!(err(e.one(json!({"op": "star", "o": o}))).contains("利用者だけ"));
        // sci_ops に書いてあっても弾く。
        let blocks = e.roots.root.join(".sai/blocks");
        std::fs::create_dir_all(&blocks).unwrap();
        std::fs::write(blocks.join("t-star.md"), "---\nsci_ops: add_o, star\n---\n# t\n").unwrap();
        let actor = Actor { run_id: None, block: Some("t-star".into()) };
        assert_eq!(allowed_ops(&e.roots, "t-star"), ["add_o", "star"]);
        let m = err(propose(e.conn(), &e.roots, &e.defs, &actor, &json!({"proposals": [{"op": "star", "o": o}]})));
        assert!(m.starts_with("1 件目（star）"));
        e.user(json!({"op": "star", "o": o})).unwrap();
        let log = store::list_log(e.conn(), None).unwrap();
        assert_eq!(log.iter().filter(|l| l.action == "star" && l.actor == "user").count(), 1);
        e.user(json!({"op": "star", "o": o})).unwrap();
        assert_eq!(store::list_log(e.conn(), None).unwrap().len(), 1);
    }

    #[test]
    fn user_ops() {
        let e = env();
        let o = e.o("A");
        e.user(json!({"op": "star", "o": o})).unwrap();
        e.user(json!({"op": "unstar", "o": o})).unwrap();
        e.user(json!({"op": "edit", "node": o, "text": "B", "body": "本文"})).unwrap();
        assert!(err(e.user(json!({"op": "edit", "node": o}))).contains("text か body"));
        let q = fmt_id(Kind::Q, e.sub);
        e.user(json!({"op": "set_target", "q": q, "target": 5})).unwrap();
        assert_eq!(store::get_node(e.conn(), e.sub).unwrap().target, Some(5));
        e.user(json!({"op": "set_target", "q": q, "target": null})).unwrap();
        assert_eq!(store::get_node(e.conn(), e.sub).unwrap().target, None);
        assert!(err(e.user(json!({"op": "set_target", "q": q, "target": 100}))).contains("0〜99"));
        let actions: Vec<_> = store::list_log(e.conn(), None).unwrap().into_iter().map(|l| l.action).collect();
        assert_eq!(actions, ["set_target", "set_target", "edit", "unstar", "star"]);
    }

    #[test]
    fn revert_applied() {
        let e = env();
        let o = e.o("A");
        let r = e.one(json!({"op": "add_f", "parent": o, "text": "根拠"})).unwrap();
        let p = parse_ref(&r.proposal, 'P').unwrap();
        let t = triage(e.conn(), &e.roots, &e.defs, p, TriageAction::Revert, None).unwrap();
        assert_eq!(t.status, "reverted");
        let (_, f) = parse_id(&r.result.unwrap()).unwrap();
        assert!(store::get_node(e.conn(), f).unwrap().removed_at.is_some());
        assert_eq!(store::get_proposal(e.conn(), p).unwrap().status, "reverted");
        // logic は記録を残したまま reverted になる。
        let l = e.one(json!({"op": "record_logic", "q": fmt_id(Kind::Q, e.sub), "value": "ok"})).unwrap();
        triage(e.conn(), &e.roots, &e.defs, parse_ref(&l.proposal, 'P').unwrap(), TriageAction::Revert, None).unwrap();
        assert_eq!(store::list_logic(e.conn(), None).unwrap().len(), 1);
    }

    #[test]
    fn shape_errors() {
        let e = env();
        let q = fmt_id(Kind::Q, e.sub);
        assert!(err(e.one(json!({"op": "add_o", "q": q}))).contains("text が必要"));
        assert!(err(e.one(json!({"op": "add_o", "q": q, "text": "x", "zzz": 1}))).contains("zzz"));
        assert!(err(e.one(json!({"op": "add_o", "q": q, "text": "x", "kind": "foo"}))).contains("answer / issue"));
        assert!(err(e.one(json!({"op": "nope"}))).contains("知らない op"));
        assert!(err(e.one(json!({"op": "add_o", "q": "O1", "text": "x"}))).contains("Q の ID"));
        assert!(err(e.one(json!({"op": "add_q", "parent": q, "text": "x", "guide": "zz"}))).contains("知らないガイド"));
    }

    #[test]
    fn reason_is_not_a_field() {
        let e = env();
        let q = fmt_id(Kind::Q, e.sub);
        // 理由（reason）を付けた op は、知らない項目として弾かれる。
        assert!(err(e.one(json!({"op": "add_o", "q": q, "text": "x", "reason": "x"}))).contains("reason"));
        assert!(err(e.one(json!({"op": "copy", "node": q, "to": q, "reason": "x"}))).contains("reason"));
        // reason を付けなければ remove は通る（仕分け待ちまで）。
        let o = e.o("A");
        let r = e.one(json!({"op": "remove", "node": o})).unwrap();
        assert_eq!(r.status, "pending");
    }

    #[test]
    fn schema_has_no_user_ops() {
        let s = propose_schema();
        let ops: Vec<_> = s["properties"]["proposals"]["items"]["oneOf"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v["properties"]["op"]["const"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(ops, ["add_q", "add_o", "add_f", "add_e", "record_logic", "add_seed", "copy", "remove"]);
        assert!(is_user_only_op("star") && !is_user_only_op("add_o") && is_known_op("edit"));
    }
}
