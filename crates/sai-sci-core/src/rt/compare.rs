//! `sai-sci compare <S>…`: 複数の SPROUT（S）を、軸ごとに並べて比べる（C-CMP-1）。
//!
//! 欄は保存しない。S が答える問い（`NodeJson.questions`）のうち、軸がその軸の物の回答から作る。
//! 持ち主が D なら `fixed`（seed の回答）、S なら `novel`（sprout の回答）。

use std::collections::BTreeMap;

use rusqlite::Connection;
use serde::Serialize;

use super::axes::AXES;
use super::compute::Tree;
use super::seedq;
use super::defs::Defs;
use super::ops::invalid;
use super::{Kind, Result, RtError, fmt_id, parse_id};
use sai_paper_core::paths::PaperRoots;

/// 比べられる S の数。
const MIN: usize = 2;
const MAX: usize = 10;

/// 軸 1 つの欄。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Cell {
    /// 最初の回答が path なら、その A（テキストの回答なら null）。
    pub a: Option<String>,
    /// 最初の回答の文（path なら A の文、テキストならその文）。回答が無ければ空。
    pub text: String,
    /// `fixed`（seed の回答）・`novel`（sprout の回答）・`open`（回答が無い）。
    pub role: String,
    /// 同じ軸のほかの path の A（2 つ目以降）。1 つだけなら出さない。
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub more: Vec<String>,
}

/// 比べる S 1 つ。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Sprout {
    pub id: String,
    pub title: String,
    /// 親の D（seed）。無ければ null。
    pub seed: Option<String>,
    /// 軸（キーは [`AXES`] の key）ごとの欄。
    pub cells: BTreeMap<String, Cell>,
}

/// `sai-sci compare` の結果（C-CMP-1）。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Compare {
    pub axes: Vec<String>,
    pub sprouts: Vec<Sprout>,
    /// 軸ごとに、全部の S で最初の A が同じなら true。
    pub same: BTreeMap<String, bool>,
}

/// 空白を 1 つにまとめる。
fn flat(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// S 1 つの欄。
fn sprout(tree: &Tree, id: i64) -> Result<Sprout> {
    let node = tree
        .node(id)
        .filter(|n| n.kind == Kind::S)
        .ok_or_else(|| RtError::NotFound(format!("{} はありません", fmt_id(Kind::S, id))))?;
    let seed = node.parent_id.and_then(|p| tree.node(p)).filter(|p| p.kind == Kind::D);
    let row = tree.row(id);
    let mut cells = BTreeMap::new();
    for (key, _) in AXES {
        // S が答える問いのうち、この軸の物の回答（問いの順・回答の順）。
        let mine: Vec<&seedq::AnswerJson> = row
            .questions
            .iter()
            .filter(|q| q.axis.as_deref() == Some(key))
            .flat_map(|q| q.answers.iter())
            .collect();
        let cell = match mine.split_first() {
            Some((first, rest)) => Cell {
                a: first.a.clone(),
                text: flat(first.a_text.as_deref().or(first.text.as_deref()).unwrap_or_default()),
                // 持ち主の D の回答は seed の回答（固定）、S の回答は sprout の回答（新規）。
                role: if first.owner.starts_with('D') { "fixed" } else { "novel" }.into(),
                more: rest.iter().filter_map(|r| r.a.clone()).collect(),
            },
            None => Cell { a: None, text: String::new(), role: "open".into(), more: Vec::new() },
        };
        cells.insert(key.to_string(), cell);
    }
    Ok(Sprout {
        id: fmt_id(Kind::S, id),
        title: flat(&node.text),
        seed: seed.map(|d| fmt_id(Kind::D, d.id)),
        cells,
    })
}

/// S の ID を 2〜10 個受けて、軸ごとの欄を作る。S 以外・個数の誤りは入力の誤り（終了コード 2）。
///
/// # Errors
///
/// 個数が 2〜10 でない、S でない ID、存在しない S、DB のエラー。
pub fn compare(conn: &Connection, roots: &PaperRoots, defs: &Defs, ids: &[String]) -> Result<Compare> {
    if !(MIN..=MAX).contains(&ids.len()) {
        return invalid(format!("compare は S を {MIN}〜{MAX} 個指定してください（{} 個）", ids.len()));
    }
    let tree = Tree::load(conn, roots, defs)?;
    let mut sprouts = Vec::new();
    for text in ids {
        let (kind, num) = parse_id(text)?;
        if kind != Kind::S {
            return invalid(format!("compare は S の ID だけを受けます: {text}"));
        }
        sprouts.push(sprout(&tree, num)?);
    }
    // 同じ軸の欄は、最初の A と文が全部の S で同じなら同じ。
    let same = AXES
        .iter()
        .map(|(key, _)| {
            let first = &sprouts[0].cells[*key];
            ((*key).to_string(), sprouts.iter().all(|s| s.cells[*key].a == first.a && s.cells[*key].text == first.text))
        })
        .collect();
    Ok(Compare { axes: AXES.iter().map(|(key, _)| (*key).to_string()).collect(), sprouts, same })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rt::seedq;
    use crate::rt::store;
    use crate::rt::store::NewNode;
    use sai_paper_core::database::Database;
    use std::fs;

    struct Env {
        roots: PaperRoots,
        db: Database,
    }

    fn env(tag: &str) -> Env {
        let dir = std::env::temp_dir().join(format!("sai-rt-compare-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let data_dir = dir.join(".sai/paper");
        fs::create_dir_all(&data_dir).unwrap();
        let db = Database::open(&data_dir.join("papr.db")).unwrap();
        store::meta_set(db.connection(), "initialized_at", "2026-10-02 00:00:00").unwrap();
        store::meta_set(db.connection(), "seed_questions", "1").unwrap();
        Env { roots: PaperRoots { root: dir, data_dir }, db }
    }

    fn add(e: &Env, kind: Kind, parent: Option<i64>, guide: Option<&str>, text: &str, body: &str) -> i64 {
        let n = NewNode {
            kind, parent_id: parent, guide_key: guide, text, body, stance: None, cites: &[],
            paper_id: None, run_id: None, report_id: None, mlflow_ref: None, dataset_ref: None, paths: &[], created_by: "user",
        };
        store::insert_node(e.db.connection(), &n).unwrap()
    }

    fn ids(list: &[i64], kind: Kind) -> Vec<String> {
        list.iter().map(|id| fmt_id(kind, *id)).collect()
    }

    /// 軸 method の Q（下に A94 → A385）と task の Q（A7）。D は雛形の 7 問（task は決定の文、data は記載なし）。
    /// S1 は method に A385 で答え、S2 は答えていない。
    fn fixture(e: &Env) -> (i64, i64, i64) {
        let conn = e.db.connection();
        let q_task = add(e, Kind::Q, None, Some("task"), "解く問題の問い", "");
        add(e, Kind::A, Some(q_task), None, "発作部位", "");
        let q_method = add(e, Kind::Q, None, Some("method"), "手法の問い", "");
        let a94 = add(e, Kind::A, Some(q_method), None, "Costa 型", "");
        let a385 = add(e, Kind::A, Some(a94), None, "細かい手法", "");
        let d = add(e, Kind::D, None, None, "題", "");
        let mut questions = Vec::new();
        for (axis, text) in seedq::TEMPLATE {
            questions.push(seedq::insert_question(conn, d, text, axis).unwrap());
        }
        seedq::insert_answer(conn, d, questions[0], None, Some("発作部位の特定"), "user").unwrap();
        seedq::insert_answer(conn, d, questions[2], None, Some("記載なし: 実測データを使わない"), "user").unwrap();
        let s1 = add(e, Kind::S, Some(d), None, "一つ目", "");
        seedq::insert_answer(conn, s1, questions[3], Some(a385), None, "user").unwrap();
        let s2 = add(e, Kind::S, Some(d), None, "二つ目", "");
        seedq::refresh(conn, d).unwrap();
        (s1, s2, a385)
    }

    #[test]
    fn cells_and_same_follow_questions_and_answers() {
        let e = env("cells");
        let (s1, s2, a385) = fixture(&e);
        let got = compare(e.db.connection(), &e.roots, &Defs::default(), &ids(&[s1, s2], Kind::S)).unwrap();
        assert_eq!(got.axes, ["task", "target", "data", "method", "output", "metric"]);
        // S1 の method は A385（sprout の回答＝新規）。
        let method = &got.sprouts[0].cells["method"];
        assert_eq!((method.a.clone(), method.role.as_str()), (Some(fmt_id(Kind::A, a385)), "novel"));
        assert_eq!(method.text, "細かい手法");
        // task は seed のテキストの回答（固定）。A は無い。
        let task = &got.sprouts[0].cells["task"];
        assert_eq!((task.a.clone(), task.role.as_str(), task.text.as_str()), (None, "fixed", "発作部位の特定"));
        // data は seed の「記載なし」の文が固定の欄として出る（記載なし の role は無い）。
        assert_eq!(got.sprouts[0].cells["data"].role, "fixed");
        assert_eq!(got.sprouts[0].cells["data"].text, "記載なし: 実測データを使わない");
        // S2 の method は空欄。
        assert_eq!(got.sprouts[1].cells["method"].role, "open");
        // method だけ違う。data は両方とも同じ seed の回答。
        assert!(!got.same["method"]);
        assert!(got.same["data"]);
    }

    #[test]
    fn one_s_or_non_s_is_invalid() {
        let e = env("count");
        let (s1, s2, _) = fixture(&e);
        let conn = e.db.connection();
        let one = compare(conn, &e.roots, &Defs::default(), &ids(&[s1], Kind::S)).unwrap_err();
        assert!(matches!(one, RtError::Invalid(_)));
        let not_s = compare(conn, &e.roots, &Defs::default(), &["A1".to_string(), fmt_id(Kind::S, s2)]).unwrap_err();
        assert!(matches!(not_s, RtError::Invalid(_)));
    }
}
