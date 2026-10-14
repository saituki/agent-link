//! seed（D）の問いと、その回答（テキストか path の A）。SPROUT（S）は D の問いに答えて完成する。
//!
//! 表は `rt_seed_questions`（問い）と `rt_answers`（回答）。回答の持ち主は D（seed の回答＝固定）か S（sprout の回答＝新規）。
//! S の `paths` は回答から導く（[`sync_paths`]）。完成した S は `frozen_qs` に完成時の問いを凍結する（後から足した問いは効かない）。
//! 変換（[`upgrade`]）は今の 6 軸の本文と S の paths を、この模型に 1 回だけ移す。

use rusqlite::{Connection, OptionalExtension, Row, params};
use serde::Serialize;

use super::axes;
use super::store::{self, NodeRow};
use super::{Kind, Result, RtError, fmt_id, fmt_sa, fmt_sq, now_text};

/// 雛形の問い（軸と文）。seed を足すと、この 7 問が ord 1〜7 で最初から入る（消せる）。
pub const TEMPLATE: [(Option<&str>, &str); 7] = [
    (Some("task"), "解く問題は何か"),
    (Some("target"), "対象は誰・何か"),
    (Some("data"), "どのデータを使うか"),
    (Some("method"), "どの手法を使うか"),
    (Some("output"), "何を出力するか"),
    (Some("metric"), "どの指標で評価するか"),
    (None, "成功基準は何か（指標の値と向き）"),
];

/// 問い 1 行（`rt_seed_questions`）。
#[derive(Debug, Clone, PartialEq)]
pub struct QuestionRow {
    pub id: i64,
    pub seed_id: i64,
    pub text: String,
    pub axis: Option<String>,
    pub ord: i64,
    pub removed_at: Option<String>,
}

/// 回答 1 行（`rt_answers`）。`a_id` と `text` のどちらか一方だけが入る。
#[derive(Debug, Clone, PartialEq)]
pub struct AnswerRow {
    pub id: i64,
    pub owner_id: i64,
    pub question_id: i64,
    pub a_id: Option<i64>,
    pub text: Option<String>,
    pub removed_at: Option<String>,
}

/// SPROUT の path 1 つの区分。`paths` と同じ順。`role` は持ち主が D なら `fixed`、S なら `novel`。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PathRole {
    pub a: String,
    /// 問いの軸。問いに軸が無ければ、A の一番上の軸の A の guide_key。分からなければ null。
    pub axis: Option<String>,
    /// `fixed`（seed の回答）か `novel`（sprout の回答）。
    pub role: String,
    /// path の回答の問い（`SQ<n>`）。
    pub question: String,
}

/// JSON の回答 1 件（`NodeJson.questions[].answers`）。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AnswerJson {
    pub id: String,
    /// 持ち主（`D1` か `S2`）。
    pub owner: String,
    /// path の回答の A の ID。テキストの回答なら null。
    pub a: Option<String>,
    /// path の回答の A の文。テキストの回答なら null。
    pub a_text: Option<String>,
    /// テキストの回答の文。path の回答なら null。
    pub text: Option<String>,
}

/// JSON の問い 1 件（`NodeJson.questions`）。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct QuestionJson {
    pub id: String,
    pub text: String,
    pub axis: Option<String>,
    /// 消した問い（S の凍結に入っているときだけ出る）。
    pub removed: bool,
    pub answers: Vec<AnswerJson>,
}

/// D か S の問いの表示（`NodeJson` の `questions`・`complete`・`missing`）。D は `complete`・`missing` を持たない。
#[derive(Debug, Clone, PartialEq, Serialize, Default)]
pub struct SeedView {
    pub questions: Vec<QuestionJson>,
    pub complete: Option<bool>,
    pub missing: Vec<String>,
}

const Q_COLUMNS: &str = "id, seed_id, text, axis, ord, removed_at";
const A_COLUMNS: &str = "id, owner_id, question_id, a_id, text, removed_at";

fn question_from_row(row: &Row<'_>) -> rusqlite::Result<QuestionRow> {
    Ok(QuestionRow {
        id: row.get(0)?,
        seed_id: row.get(1)?,
        text: row.get(2)?,
        axis: row.get(3)?,
        ord: row.get(4)?,
        removed_at: row.get(5)?,
    })
}

fn answer_from_row(row: &Row<'_>) -> rusqlite::Result<AnswerRow> {
    Ok(AnswerRow {
        id: row.get(0)?,
        owner_id: row.get(1)?,
        question_id: row.get(2)?,
        a_id: row.get(3)?,
        text: row.get(4)?,
        removed_at: row.get(5)?,
    })
}

/// seed（D）の問い。`ord`・`id` の順。`include_removed` が偽なら消した問いを除く。
pub fn questions_of(conn: &Connection, seed_id: i64, include_removed: bool) -> Result<Vec<QuestionRow>> {
    let live = if include_removed { "" } else { "AND removed_at IS NULL" };
    let sql = format!("SELECT {Q_COLUMNS} FROM rt_seed_questions WHERE seed_id = ?1 {live} ORDER BY ord, id");
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params![seed_id], question_from_row)?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

/// 問い 1 つ。無ければ NotFound。
pub fn question(conn: &Connection, id: i64) -> Result<QuestionRow> {
    conn.query_row(
        &format!("SELECT {Q_COLUMNS} FROM rt_seed_questions WHERE id = ?1"),
        params![id],
        question_from_row,
    )
    .optional()?
    .ok_or_else(|| RtError::NotFound(format!("{} はありません", fmt_sq(id))))
}

/// 問いの生きた回答（問いの ID の順。`answers_of` は問いごとに読む）。
pub fn answers_of(conn: &Connection, question_ids: &[i64]) -> Result<Vec<AnswerRow>> {
    let mut out = Vec::new();
    let mut stmt = conn.prepare(&format!(
        "SELECT {A_COLUMNS} FROM rt_answers WHERE question_id = ?1 AND removed_at IS NULL ORDER BY id"
    ))?;
    for &q in question_ids {
        out.extend(stmt.query_map(params![q], answer_from_row)?.collect::<rusqlite::Result<Vec<_>>>()?);
    }
    Ok(out)
}

/// 回答 1 つ。無ければ NotFound（消した回答も読む）。
pub fn answer(conn: &Connection, id: i64) -> Result<AnswerRow> {
    conn.query_row(&format!("SELECT {A_COLUMNS} FROM rt_answers WHERE id = ?1"), params![id], answer_from_row)
        .optional()?
        .ok_or_else(|| RtError::NotFound(format!("{} はありません", fmt_sa(id))))
}

/// S の親の D（親が D のときだけ）。親が無い・D でない古い S は `None`。
pub fn parent_seed(conn: &Connection, s: &NodeRow) -> Result<Option<i64>> {
    let Some(p) = s.parent_id else { return Ok(None) };
    Ok((store::get_node(conn, p)?.kind == Kind::D).then_some(p))
}

/// D の id（D なら自分、S なら親の D）。どちらでもなければ `None`。
#[must_use]
pub fn seed_of(conn: &Connection, n: &NodeRow) -> Option<i64> {
    match n.kind {
        Kind::D => Some(n.id),
        Kind::S => parent_seed(conn, n).ok().flatten(),
        _ => None,
    }
}

/// 持ち主（D か S）に、その問いの生きた回答が 1 つでもあるか（テキストでも path でも）。
pub fn has_answer(conn: &Connection, owner: i64, q: i64) -> Result<bool> {
    let n: i64 = conn.query_row(
        "SELECT COUNT(*) FROM rt_answers WHERE owner_id = ?1 AND question_id = ?2 AND removed_at IS NULL",
        params![owner, q],
        |row| row.get(0),
    )?;
    Ok(n > 0)
}

/// 持ち主に、問いの生きた path の回答（A）があるか。
pub fn has_path(conn: &Connection, owner: i64, q: i64, a: i64) -> Result<bool> {
    let n: i64 = conn.query_row(
        "SELECT COUNT(*) FROM rt_answers WHERE owner_id = ?1 AND question_id = ?2 AND a_id = ?3 AND removed_at IS NULL",
        params![owner, q, a],
        |row| row.get(0),
    )?;
    Ok(n > 0)
}

/// S が答えるべき問い。凍結があればその ID（消した問いも含む）、無ければ親 D の生きた問い。親が D でなければ空。
pub fn relevant(conn: &Connection, s: &NodeRow) -> Result<Vec<i64>> {
    if let Some(frozen) = &s.frozen_qs {
        return Ok(frozen.clone());
    }
    match parent_seed(conn, s)? {
        Some(d) => Ok(questions_of(conn, d, false)?.into_iter().map(|q| q.id).collect()),
        None => Ok(Vec::new()),
    }
}

/// S が完成しているか（relevant の問いの全部に、親 D か S の回答が 1 つ以上ある）と、未回答の問い。
pub fn is_complete(conn: &Connection, s: &NodeRow) -> Result<(bool, Vec<i64>)> {
    let seed = parent_seed(conn, s)?;
    let mut missing = Vec::new();
    for q in relevant(conn, s)? {
        let answered = seed.is_some_and(|d| has_answer(conn, d, q).unwrap_or(false)) || has_answer(conn, s.id, q)?;
        if !answered {
            missing.push(q);
        }
    }
    Ok((missing.is_empty(), missing))
}

/// S の path の回答 1 件（A・持ち主・問い）。
struct PathAnswer {
    a: i64,
    owner: i64,
    question: i64,
}

/// S の path の回答（`paths` と同じ順）。relevant の問いの順、同じ問いは D の回答を先に、同じ A は 1 回だけ。
fn path_answers(conn: &Connection, s: &NodeRow) -> Result<Vec<PathAnswer>> {
    let Some(seed) = parent_seed(conn, s)? else { return Ok(Vec::new()) };
    let mut out: Vec<PathAnswer> = Vec::new();
    for q in relevant(conn, s)? {
        let mut answers: Vec<AnswerRow> = answers_of(conn, &[q])?
            .into_iter()
            .filter(|a| a.a_id.is_some() && (a.owner_id == seed || a.owner_id == s.id))
            .collect();
        answers.sort_by_key(|a| a.owner_id != seed);
        for answer in answers {
            let Some(a) = answer.a_id else { continue };
            if !out.iter().any(|p| p.a == a) {
                out.push(PathAnswer { a, owner: answer.owner_id, question: q });
            }
        }
    }
    Ok(out)
}

/// S の `paths` を回答から書き直す（[`path_answers`] の A の順）。
pub fn sync_paths(conn: &Connection, s: &NodeRow) -> Result<()> {
    let ids: Vec<i64> = path_answers(conn, s)?.iter().map(|p| p.a).collect();
    store::set_paths(conn, s.id, &ids)
}

/// S の path の区分（`paths` と同じ順）。持ち主が D なら `fixed`、S なら `novel`。
pub fn path_roles(conn: &Connection, s: &NodeRow) -> Result<Vec<PathRole>> {
    let seed = parent_seed(conn, s)?;
    let mut out = Vec::new();
    for p in path_answers(conn, s)? {
        let q = question(conn, p.question)?;
        let axis = match q.axis {
            Some(key) => Some(key),
            None => axes::axis_of(conn, p.a)?,
        };
        let role = if Some(p.owner) == seed { "fixed" } else { "novel" };
        out.push(PathRole {
            a: fmt_id(Kind::A, p.a),
            axis,
            role: role.into(),
            question: fmt_sq(p.question),
        });
    }
    Ok(out)
}

/// S 1 つの凍結と paths を直す。完成していて凍結が無ければ、今の relevant を凍結する。未完成なら凍結を外す。
fn settle(conn: &Connection, sid: i64) -> Result<()> {
    let mut s = store::get_node(conn, sid)?;
    if s.frozen_qs.is_some() && !is_complete(conn, &s)?.0 {
        store::set_frozen(conn, sid, None)?;
        s.frozen_qs = None;
    }
    if s.frozen_qs.is_none() && is_complete(conn, &s)?.0 {
        let ids = relevant(conn, &s)?;
        store::set_frozen(conn, sid, Some(&ids))?;
        s.frozen_qs = Some(ids);
    }
    sync_paths(conn, &s)
}

/// D の下の生きた S を、問いと回答に合わせて直す（凍結と paths）。D の問いか回答を変える op の最後で必ず呼ぶ。
pub fn refresh(conn: &Connection, seed_id: i64) -> Result<()> {
    for s in store::children(conn, seed_id)?.into_iter().filter(|n| n.kind == Kind::S) {
        settle(conn, s.id)?;
    }
    Ok(())
}

/// 問いを 1 つ足す（ord は今の最大 + 1。消した問いも数える）。
pub fn insert_question(conn: &Connection, seed_id: i64, text: &str, axis: Option<&str>) -> Result<i64> {
    let ord: i64 = conn.query_row(
        "SELECT COALESCE(MAX(ord), 0) + 1 FROM rt_seed_questions WHERE seed_id = ?1",
        params![seed_id],
        |row| row.get(0),
    )?;
    conn.execute(
        "INSERT INTO rt_seed_questions (seed_id, text, axis, ord) VALUES (?1, ?2, ?3, ?4)",
        params![seed_id, text, axis, ord],
    )?;
    Ok(conn.last_insert_rowid())
}

/// 問いの文と軸を変える。`axis` は `Some(None)` で軸を外す（`None` は変えない）。
pub fn update_question(conn: &Connection, id: i64, text: Option<&str>, axis: Option<Option<&str>>) -> Result<()> {
    let now = now_text();
    if let Some(text) = text {
        conn.execute("UPDATE rt_seed_questions SET text = ?2, updated_at = ?3 WHERE id = ?1", params![id, text, now])?;
    }
    if let Some(axis) = axis {
        conn.execute("UPDATE rt_seed_questions SET axis = ?2, updated_at = ?3 WHERE id = ?1", params![id, axis, now])?;
    }
    Ok(())
}

/// 問いを消す（回答は残す）。
pub fn remove_question(conn: &Connection, id: i64) -> Result<()> {
    conn.execute(
        "UPDATE rt_seed_questions SET removed_at = ?2, updated_at = ?2 WHERE id = ?1",
        params![id, now_text()],
    )?;
    Ok(())
}

/// 回答を 1 つ足す。`a` と `text` のちょうど一方。検査は ops が行う。
pub fn insert_answer(conn: &Connection, owner: i64, q: i64, a: Option<i64>, text: Option<&str>, by: &str) -> Result<i64> {
    conn.execute(
        "INSERT INTO rt_answers (owner_id, question_id, a_id, text, created_by) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![owner, q, a, text, by],
    )?;
    Ok(conn.last_insert_rowid())
}

/// テキストの回答の文を変える。
pub fn update_answer_text(conn: &Connection, id: i64, text: &str) -> Result<()> {
    conn.execute(
        "UPDATE rt_answers SET text = ?2, updated_at = ?3 WHERE id = ?1",
        params![id, text, now_text()],
    )?;
    Ok(())
}

/// 回答を消す（問いは残す）。
pub fn remove_answer(conn: &Connection, id: i64) -> Result<()> {
    conn.execute(
        "UPDATE rt_answers SET removed_at = ?2, updated_at = ?2 WHERE id = ?1",
        params![id, now_text()],
    )?;
    Ok(())
}

fn owner_label(conn: &Connection, owner: i64) -> Result<String> {
    let n = store::get_node(conn, owner)?;
    Ok(fmt_id(n.kind, n.id))
}

/// 問い 1 つを JSON にする。回答は `owners` の順（D が先）。
fn question_json(conn: &Connection, q: &QuestionRow, owners: &[i64]) -> Result<QuestionJson> {
    let mut answers: Vec<AnswerRow> = answers_of(conn, &[q.id])?
        .into_iter()
        .filter(|a| owners.contains(&a.owner_id))
        .collect();
    answers.sort_by_key(|a| owners.iter().position(|o| *o == a.owner_id));
    let mut out = Vec::new();
    for a in answers {
        let (a_id, a_text) = match a.a_id {
            Some(id) => (Some(fmt_id(Kind::A, id)), Some(store::get_node(conn, id)?.text)),
            None => (None, None),
        };
        out.push(AnswerJson {
            id: fmt_sa(a.id),
            owner: owner_label(conn, a.owner_id)?,
            a: a_id,
            a_text,
            text: a.text,
        });
    }
    Ok(QuestionJson {
        id: fmt_sq(q.id),
        text: q.text.clone(),
        axis: q.axis.clone(),
        removed: q.removed_at.is_some(),
        answers: out,
    })
}

/// D か S の問い・回答・完成の表示（JSON）。D は生きた問い全部。S は relevant の問い（消した問いは `removed`）と、親 D と S の回答。
pub fn view(conn: &Connection, n: &NodeRow) -> Result<SeedView> {
    match n.kind {
        Kind::D => {
            let mut questions = Vec::new();
            for q in questions_of(conn, n.id, false)? {
                questions.push(question_json(conn, &q, &[n.id])?);
            }
            Ok(SeedView { questions, complete: None, missing: Vec::new() })
        }
        Kind::S => {
            let owners: Vec<i64> = parent_seed(conn, n)?.into_iter().chain([n.id]).collect();
            let mut questions = Vec::new();
            for id in relevant(conn, n)? {
                questions.push(question_json(conn, &question(conn, id)?, &owners)?);
            }
            let (done, missing) = is_complete(conn, n)?;
            Ok(SeedView {
                questions,
                complete: Some(done),
                missing: missing.into_iter().map(fmt_sq).collect(),
            })
        }
        _ => Ok(SeedView::default()),
    }
}

/// 変換を済ませたか（`rt_meta` の `seed_questions` が `1`）。
fn upgraded(conn: &Connection) -> Result<bool> {
    Ok(store::meta_get(conn, "seed_questions")?.as_deref() == Some("1"))
}

/// 今ある D と S を、問いと回答の模型に 1 回だけ移す。`rt_meta` の `seed_questions` が `1` なら何もしない。
/// 1 回のトランザクションで行い、最後に `seed_questions = 1` を書く。
pub fn upgrade(conn: &Connection) -> Result<()> {
    if upgraded(conn)? {
        return Ok(());
    }
    if conn.is_autocommit() {
        let tx = conn.unchecked_transaction()?;
        upgrade_in(&tx)?;
        tx.commit()?;
        Ok(())
    } else {
        upgrade_in(conn)
    }
}

fn upgrade_in(conn: &Connection) -> Result<()> {
    for d in store::list_nodes(conn, false)?.into_iter().filter(|n| n.kind == Kind::D) {
        if questions_of(conn, d.id, true)?.is_empty() {
            seed_from_body(conn, &d)?;
        }
    }
    // 1 段目: S の path を回答にする（seed の回答に足すと、同じ D の下の他の S の paths にも効くので、同期は 2 段目で行う）。
    let sprouts: Vec<NodeRow> = store::list_nodes(conn, false)?.into_iter().filter(|n| n.kind == Kind::S).collect();
    for s in &sprouts {
        let Some(seed) = parent_seed(conn, s)? else { continue };
        let qs = questions_of(conn, seed, false)?;
        for a in s.paths.clone() {
            let Some(q) = axes::axis_of(conn, a)?.and_then(|key| qs.iter().find(|q| q.axis.as_deref() == Some(key.as_str()))) else {
                log_path(conn, &s, a, "は回答にしなかった（軸が分からない、または雛形の問いに軸が無い）")?;
                continue;
            };
            // 同じ path が既にある（S の回答でも seed の回答でも）なら、二重に足さない。
            if has_path(conn, s.id, q.id, a)? || has_path(conn, seed, q.id, a)? {
                continue;
            }
            if has_answer(conn, seed, q.id)? {
                // seed が答えている問い（決定など）。この A は seed の path の回答（固定）として並べて足す。
                insert_answer(conn, seed, q.id, Some(a), None, "upgrade")?;
                log_path(conn, s, a, &format!("を seed の path の回答に足した（{}）", fmt_sq(q.id)))?;
                continue;
            }
            // seed が答えていない問い（範囲・新規性・空欄）は、S の回答（新規）にする。
            insert_answer(conn, s.id, q.id, Some(a), None, "user")?;
        }
    }
    // 2 段目: 凍結（生きた E を持つ S は、答えた問いだけ凍結する。実験中の S に E を足せるように）と paths の同期。
    for s in &sprouts {
        let Some(seed) = parent_seed(conn, s)? else { continue };
        let has_e = store::children(conn, s.id)?.iter().any(|k| k.kind == Kind::E);
        if has_e {
            let mut answered = Vec::new();
            for q in questions_of(conn, seed, false)? {
                if has_answer(conn, seed, q.id)? || has_answer(conn, s.id, q.id)? {
                    answered.push(q.id);
                }
            }
            store::set_frozen(conn, s.id, Some(&answered))?;
        }
        settle(conn, s.id)?;
    }
    store::meta_set(conn, "seed_questions", "1")
}

/// 変換で path を扱ったときの記録（rt_log の detail。`what` は `upgrade: A454 <what>` の後ろに続く文）。
fn log_path(conn: &Connection, s: &NodeRow, a: i64, what: &str) -> Result<()> {
    let detail = format!("upgrade: {} {what}", fmt_id(Kind::A, a));
    store::insert_log(conn, s.id, "edit", &detail, "upgrade", None)?;
    Ok(())
}

/// 生きた A の ID（`A<n>` の形で、生きている A）。違えば `None`。
fn live_a(conn: &Connection, text: &str) -> Option<i64> {
    let id = super::parse_id_of(text, Kind::A).ok()?;
    store::get_node_of(conn, Kind::A, id).ok().filter(|n| n.removed_at.is_none()).map(|n| n.id)
}

/// 今の D の 6 軸の本文から、雛形の 7 問と、その回答を作る（変換の 1 件分）。
fn seed_from_body(conn: &Connection, d: &NodeRow) -> Result<()> {
    let states = axes::parse_axes(&d.body);
    // 成功基準: 1 行目が「空欄」なら落とす。残りが空なら回答は付けない。
    let criteria = axes::sections(&d.body).get("## 成功基準").and_then(|lines| {
        let rest: &[&str] = match lines.first() {
            Some(&"空欄") => &lines[1..],
            _ => lines.as_slice(),
        };
        Some(rest.join(" ")).filter(|text| !text.is_empty())
    });
    for (axis, base) in TEMPLATE.iter() {
        let mut text = (*base).to_string();
        // 回答は (path の A, テキスト) の組。テキストと path の両方が入ることがある（固定の決定と、その A）。
        let mut answers: Vec<(Option<i64>, Option<String>)> = Vec::new();
        match axis {
            Some(key) => {
                let state = &states[*key];
                match state.state.as_str() {
                    "fixed" => {
                        if let Some(value) = &state.value {
                            answers.push((None, Some(value.clone())));
                        }
                        if let Some(a) = state.a.as_deref().and_then(|a| live_a(conn, a)) {
                            answers.push((Some(a), None));
                        }
                    }
                    "range" => {
                        let value = state.value.clone().unwrap_or_default();
                        text = match &state.a {
                            Some(a) => format!("{base}（範囲: {value}。{a} の下から選ぶ）"),
                            None => format!("{base}（範囲: {value}）"),
                        };
                    }
                    "novel" => {
                        if let Some(reason) = &state.reason {
                            text = format!("{base}（新規性: {reason}）");
                        }
                    }
                    "none" => {
                        let reason = state.reason.clone().map_or_else(|| "記載なし".to_string(), |r| format!("記載なし: {r}"));
                        answers.push((None, Some(reason)));
                    }
                    _ => {}
                }
            }
            None => {
                if let Some(criteria) = &criteria {
                    answers.push((None, Some(criteria.clone())));
                }
            }
        }
        let q = insert_question(conn, d.id, &text, *axis)?;
        for (a, t) in answers {
            insert_answer(conn, d.id, q, a, t.as_deref(), "user")?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sai_paper_core::database::Database;

    fn db() -> Database {
        Database::in_memory().unwrap()
    }

    fn node(conn: &Connection, kind: Kind, parent: Option<i64>, text: &str, body: &str, paths: &[i64]) -> i64 {
        let n = store::NewNode {
            kind, parent_id: parent, guide_key: None, text, body, stance: None, cites: &[],
            paper_id: None, run_id: None, report_id: None, mlflow_ref: None, dataset_ref: None, paths, created_by: "user",
        };
        store::insert_node(conn, &n).unwrap()
    }

    fn q_node(conn: &Connection, guide: &str) -> i64 {
        let n = store::NewNode {
            kind: Kind::A, parent_id: None, guide_key: Some(guide), text: "軸の問い", body: "", stance: None, cites: &[],
            paper_id: None, run_id: None, report_id: None, mlflow_ref: None, dataset_ref: None, paths: &[], created_by: "user",
        };
        store::insert_node(conn, &n).unwrap()
    }

    /// 5 つの状態を全部持つ D の本文（6 軸の見出しと、成功基準）。
    const BODY: &str = "## 解く問題（task）\n決定: 発作部位の特定\nA: A7\n## 対象（target）\n空欄\n新規性: 患者ごとに違う\n## データ（data）\n空欄\n記載なし: 実測データを使わない\n## 手法（method）\n範囲: Costa 型を土台にする\nA: A94\n## 出力（output）\n空欄\n## 評価指標（metric）\n空欄\n## 成功基準\nAUC が 0.8 以上\n";

    #[test]
    fn upgrade_moves_body_and_paths_once() {
        let database = db();
        let conn = database.connection();
        // 軸の A と A（task の A7・method の A94 の下の A385）。
        let q_task = q_node(conn, "task");
        let a7 = node(conn, Kind::A, Some(q_task), "発作部位", "", &[]);
        let q_method = q_node(conn, "method");
        let a94 = node(conn, Kind::A, Some(q_method), "Costa 型", "", &[]);
        let a385 = node(conn, Kind::A, Some(a94), "細かい手法", "", &[]);
        let q_metric = q_node(conn, "metric");
        let a_auc = node(conn, Kind::A, Some(q_metric), "AUC", "", &[]);
        // 古い S（paths を持つ・E 無し）と、E 付きの S。どちらも D の下。
        // 本文の A7・A94 は ID（ノードの番号）なので、作った A の ID に合わせて書く。
        let body = BODY.replace("A: A7", &format!("A: {}", fmt_id(Kind::A, a7))).replace("A: A94", &format!("A: {}", fmt_id(Kind::A, a94)));
        let d = node(conn, Kind::D, None, "研究", &body, &[]);
        let s_plain = node(conn, Kind::S, Some(d), "種", "", &[]);
        conn.execute("UPDATE rt_nodes SET paths = ?2 WHERE id = ?1", params![s_plain, format!("[{a385},{a_auc}]")]).unwrap();
        let s_e = node(conn, Kind::S, Some(d), "種 E", "", &[]);
        conn.execute("UPDATE rt_nodes SET paths = ?2 WHERE id = ?1", params![s_e, format!("[{a7}]")]).unwrap();
        node(conn, Kind::E, Some(s_e), "実験", "", &[]);
        // 変換の前は問いが無い。
        assert!(questions_of(conn, d, true).unwrap().is_empty());

        upgrade(conn).unwrap();
        let qs = questions_of(conn, d, false).unwrap();
        assert_eq!(qs.len(), 7, "雛形の 7 問");
        let axis_got: Vec<Option<String>> = qs.iter().map(|q| q.axis.clone()).collect();
        let axis_want: Vec<Option<String>> = ["task", "target", "data", "method", "output", "metric"].iter().map(|k| Some((*k).to_string())).chain([None]).collect();
        assert_eq!(axis_got, axis_want);
        assert_eq!(qs[6].axis, None);
        // 決定: テキストの回答と、その A の path の回答（D の回答）。
        let answers = answers_of(conn, &[qs[0].id]).unwrap();
        assert_eq!(answers.len(), 2);
        assert_eq!(answers[0].text.as_deref(), Some("発作部位の特定"));
        assert_eq!(answers[1].a_id, Some(a7));
        // 新規性: 問いの文に「（新規性: …）」が付き、回答は無い。
        assert_eq!(qs[1].text, "対象は誰・何か（新規性: 患者ごとに違う）");
        assert!(answers_of(conn, &[qs[1].id]).unwrap().is_empty());
        // 記載なし: テキストの回答として入る。
        assert_eq!(answers_of(conn, &[qs[2].id]).unwrap()[0].text.as_deref(), Some("記載なし: 実測データを使わない"));
        // 範囲: 問いの文に範囲が付き、D の回答は無い。
        assert_eq!(qs[3].text, format!("どの手法を使うか（範囲: Costa 型を土台にする。{} の下から選ぶ）", fmt_id(Kind::A, a94)));
        assert!(answers_of(conn, &[qs[3].id]).unwrap().iter().all(|a| a.owner_id != d), "D の回答は無い（S の path だけ）");
        // 成功基準（本文の中身）は、成功基準の問いにテキストの回答として入る。
        assert_eq!(answers_of(conn, &[qs[6].id]).unwrap()[0].text.as_deref(), Some("AUC が 0.8 以上"));

        // S（E 無し）: paths は D の回答（task の A7）と S の回答（method の A385・metric の AUC）の和。
        let paths = store::get_node(conn, s_plain).unwrap().paths;
        assert_eq!(paths, vec![a7, a385, a_auc]);
        let roles = path_roles(conn, &store::get_node(conn, s_plain).unwrap()).unwrap();
        assert_eq!(roles[0], PathRole { a: fmt_id(Kind::A, a7), axis: Some("task".into()), role: "fixed".into(), question: fmt_sq(qs[0].id) });
        assert_eq!(roles[1], PathRole { a: fmt_id(Kind::A, a385), axis: Some("method".into()), role: "novel".into(), question: fmt_sq(qs[3].id) });
        assert_eq!(roles[2].role, "novel");
        // 完成: S（E 無し）は target と output に答えていないので未完成。
        let (done, missing) = is_complete(conn, &store::get_node(conn, s_plain).unwrap()).unwrap();
        assert!(!done);
        assert_eq!(missing, vec![qs[1].id, qs[4].id]);

        // E 付きの S は、回答のある問いで凍結して完成する（E を足せる）。
        let e_row = store::get_node(conn, s_e).unwrap();
        assert!(is_complete(conn, &e_row).unwrap().0);
        assert_eq!(e_row.frozen_qs.as_ref().map(Vec::len), Some(3), "回答のある問い（task・data・成功基準）だけ");
        assert_eq!(store::get_node(conn, s_e).unwrap().paths, vec![a7]);

        // 2 回目は何も増えない。
        upgrade(conn).unwrap();
        assert_eq!(questions_of(conn, d, true).unwrap().len(), 7);
        assert_eq!(answers_of(conn, &[qs[0].id]).unwrap().len(), 2);
        assert_eq!(store::get_node(conn, s_plain).unwrap().paths, vec![a7, a385, a_auc]);
        assert_eq!(store::meta_get(conn, "seed_questions").unwrap().as_deref(), Some("1"));
    }

    /// 決定の軸（`A:` が無い）の S の path は、seed の path の回答（固定）として足り、S の paths に残る。
    /// 同じ D の下の別の S が違う A を選んでいたら、どちらも seed の回答に並ぶ（S の paths は D と S の path の回答の和）。
    /// seed が答えていない軸（空欄）の path は S の回答（新規）のまま。
    #[test]
    fn upgrade_keeps_sprout_paths_under_decided_axis() {
        let database = db();
        let conn = database.connection();
        let q_task = q_node(conn, "task");
        let t1 = node(conn, Kind::A, Some(q_task), "発作部位", "", &[]);
        let t2 = node(conn, Kind::A, Some(q_task), "別の部位", "", &[]);
        let q_method = q_node(conn, "method");
        let m1 = node(conn, Kind::A, Some(q_method), "手法", "", &[]);
        // task は「決定」だけで A: が無い（論文から復元した seed の形）。method は空欄。
        let body = "## 解く問題（task）\n決定: 発作部位の特定\n## 手法（method）\n空欄\n";
        let d = node(conn, Kind::D, None, "研究", body, &[]);
        let s1 = node(conn, Kind::S, Some(d), "種 1", "", &[]);
        conn.execute("UPDATE rt_nodes SET paths = ?2 WHERE id = ?1", params![s1, format!("[{t1}]")]).unwrap();
        let s2 = node(conn, Kind::S, Some(d), "種 2", "", &[]);
        conn.execute("UPDATE rt_nodes SET paths = ?2 WHERE id = ?1", params![s2, format!("[{t2}]")]).unwrap();
        let s3 = node(conn, Kind::S, Some(d), "種 3", "", &[]);
        conn.execute("UPDATE rt_nodes SET paths = ?2 WHERE id = ?1", params![s3, format!("[{m1}]")]).unwrap();

        upgrade(conn).unwrap();
        let qs = questions_of(conn, d, false).unwrap();
        let (task_q, method_q) = (qs[0].id, qs[3].id);
        // task: seed の回答（決定の文）と、t1・t2 の path の回答（持ち主 D）。
        let seed_task = answers_of(conn, &[task_q]).unwrap();
        assert!(seed_task.iter().all(|a| a.owner_id == d));
        assert_eq!(seed_task.iter().filter_map(|a| a.a_id).collect::<Vec<_>>(), vec![t1, t2]);
        assert!(seed_task.iter().any(|a| a.text.as_deref() == Some("発作部位の特定")));
        // S の paths は D と S の path の回答の和。同じ D の下の S は seed の path を共有する。
        assert_eq!(store::get_node(conn, s1).unwrap().paths, vec![t1, t2]);
        assert_eq!(store::get_node(conn, s2).unwrap().paths, vec![t1, t2]);
        // method は seed が答えていないので、m1 は S3 の回答（新規）として残る。
        let method_answers = answers_of(conn, &[method_q]).unwrap();
        assert_eq!(method_answers.len(), 1);
        assert_eq!((method_answers[0].owner_id, method_answers[0].a_id), (s3, Some(m1)));
        assert_eq!(store::get_node(conn, s3).unwrap().paths, vec![t1, t2, m1], "D の path の回答も入る（和）");
        // 変換の記録（rt_log）に、seed の path の回答に足したことが残る。
        let logs: Vec<String> = store::list_log(conn, None).unwrap().into_iter().filter(|l| l.actor == "upgrade").map(|l| l.detail).collect();
        assert!(logs.iter().any(|d| d.contains("A") && d.contains("seed の path の回答に足した")), "{logs:?}");
    }

    /// 成功基準: 1 行目の「空欄」は落とす。残りが空なら回答は付けず、残りの文（「記載なし: …」を含む）はテキストの回答にする。
    #[test]
    fn upgrade_criteria_drops_blank_first_line() {
        let database = db();
        let conn = database.connection();
        let blank = node(conn, Kind::D, None, "空欄だけ", "## 成功基準\n空欄\n", &[]);
        let none = node(conn, Kind::D, None, "記載なし", "## 成功基準\n空欄\n記載なし: 数値は出さない\n", &[]);
        let text = node(conn, Kind::D, None, "文", "## 成功基準\nAUC が 0.8 以上\n", &[]);
        upgrade(conn).unwrap();
        let criteria = |d: i64| {
            let q = questions_of(conn, d, false).unwrap()[6].clone();
            answers_of(conn, &[q.id]).unwrap().into_iter().map(|a| a.text.unwrap_or_default()).collect::<Vec<_>>()
        };
        assert_eq!(criteria(blank), Vec::<String>::new());
        assert_eq!(criteria(none), vec!["記載なし: 数値は出さない".to_string()]);
        assert_eq!(criteria(text), vec!["AUC が 0.8 以上".to_string()]);
    }

    #[test]
    fn view_of_sprout_keeps_frozen_questions_and_marks_removed() {
        let database = db();
        let conn = database.connection();
        let d = node(conn, Kind::D, None, "研究", "", &[]);
        let q1 = insert_question(conn, d, "問い 1", Some("task")).unwrap();
        let s = node(conn, Kind::S, Some(d), "種", "", &[]);
        insert_answer(conn, s, q1, None, Some("やる"), "user").unwrap();
        refresh(conn, d).unwrap();
        let row = store::get_node(conn, s).unwrap();
        assert_eq!(row.frozen_qs, Some(vec![q1]), "完成して凍結");
        // 後から足した問いは完成した S に効かない（完成のまま）。
        let q2 = insert_question(conn, d, "問い 2", None).unwrap();
        refresh(conn, d).unwrap();
        let row = store::get_node(conn, s).unwrap();
        assert!(is_complete(conn, &row).unwrap().0);
        assert_eq!(relevant(conn, &row).unwrap(), vec![q1]);
        // 未完成の S には新しい問いが missing に出る（ここでは凍結の無い S を作る）。
        let s2 = node(conn, Kind::S, Some(d), "種 2", "", &[]);
        let row2 = store::get_node(conn, s2).unwrap();
        assert_eq!(is_complete(conn, &row2).unwrap().1, vec![q1, q2]);
        let got = view(conn, &row2).unwrap();
        assert_eq!(got.complete, Some(false));
        assert_eq!(got.missing, vec![fmt_sq(q1), fmt_sq(q2)]);
        // 完成した S の回答を消すと未完成に戻り、凍結も外れる。
        let answer_id: i64 = conn.query_row("SELECT id FROM rt_answers WHERE owner_id = ?1", params![s], |r| r.get(0)).unwrap();
        conn.execute("UPDATE rt_answers SET removed_at = 'x' WHERE id = ?1", params![answer_id]).unwrap();
        refresh(conn, d).unwrap();
        let row = store::get_node(conn, s).unwrap();
        assert!(row.frozen_qs.is_none());
        assert!(!is_complete(conn, &row).unwrap().0);
        // 消した問いは、凍結した S の relevant に残り removed で出る。
        remove_question(conn, q1).unwrap();
        let mut frozen_row = store::get_node(conn, s).unwrap();
        frozen_row.frozen_qs = Some(vec![q1]);
        let got = view(conn, &frozen_row).unwrap();
        assert!(got.questions[0].removed);
    }
}
