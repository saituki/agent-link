//! `sai-sci compare <S>…`: 複数の SPROUT（S）を、軸ごとに並べて比べる（C-CMP-1）。
//!
//! 欄の区分は保存しない。親の D の軸の状態（[`axes::parse_axes`]）と、S の path の区分（[`Tree::roles_of`]）から導く。

use std::collections::BTreeMap;

use rusqlite::Connection;
use serde::Serialize;

use super::axes::{self, AXES};
use super::compute::Tree;
use super::defs::Defs;
use super::ops::invalid;
use super::{Kind, Result, RtError, fmt_id, parse_id, parse_id_of};
use sai_paper_core::paths::PaperRoots;

/// 比べられる S の数。
const MIN: usize = 2;
const MAX: usize = 10;

/// 軸 1 つの欄。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Cell {
    /// 最初の path の A（無ければ null）。
    pub a: Option<String>,
    /// A の文。記載なしの軸は理由。無ければ空。
    pub text: String,
    /// `fixed`（決まっている）・`novel`（新しく考える）・`none`（記載なし）・`open`（空欄）。
    pub role: String,
    /// 固定の A（範囲の A と同じか、その子孫のとき）。無ければ null。
    pub fixed_at: Option<String>,
    /// 同じ軸のほかの path の A。1 つだけなら出さない。
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

/// A の文（ID で引く。無ければ空）。
fn text_of(tree: &Tree, a: &str) -> String {
    parse_id_of(a, Kind::A).ok().and_then(|id| tree.node(id)).map(|n| flat(&n.text)).unwrap_or_default()
}

/// S 1 つの欄。
fn sprout(tree: &Tree, id: i64) -> Result<Sprout> {
    let node = tree
        .node(id)
        .filter(|n| n.kind == Kind::S)
        .ok_or_else(|| RtError::NotFound(format!("{} はありません", fmt_id(Kind::S, id))))?;
    let seed = node.parent_id.and_then(|p| tree.node(p)).filter(|p| p.kind == Kind::D);
    let seed_axes = seed.map(|d| axes::parse_axes(&d.body)).unwrap_or_default();
    let roles = tree.roles_of(id);
    let mut cells = BTreeMap::new();
    for (key, _) in AXES {
        let mine: Vec<&axes::PathRole> = roles.iter().filter(|r| r.axis.as_deref() == Some(key)).collect();
        let cell = match mine.split_first() {
            Some((first, rest)) => Cell {
                a: Some(first.a.clone()),
                text: text_of(tree, &first.a),
                role: first.role.clone(),
                fixed_at: first.fixed_at.clone(),
                more: rest.iter().map(|r| r.a.clone()).collect(),
            },
            None => match seed_axes.get(key) {
                Some(axis) if axis.state == "none" => {
                    Cell { a: None, text: axis.reason.clone().unwrap_or_default(), role: "none".into(), fixed_at: None, more: Vec::new() }
                }
                _ => Cell { a: None, text: String::new(), role: "open".into(), fixed_at: None, more: Vec::new() },
            },
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
    let same = AXES
        .iter()
        .map(|(key, _)| {
            let first = sprouts[0].cells[*key].a.clone();
            ((*key).to_string(), sprouts.iter().all(|s| s.cells[*key].a == first))
        })
        .collect();
    Ok(Compare { axes: AXES.iter().map(|(key, _)| (*key).to_string()).collect(), sprouts, same })
}

#[cfg(test)]
mod tests {
    use super::*;
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
        Env { roots: PaperRoots { root: dir, data_dir }, db }
    }

    fn add(e: &Env, kind: Kind, parent: Option<i64>, text: &str, body: &str, paths: &[i64]) -> i64 {
        add_guided(e, kind, parent, None, text, body, paths)
    }

    fn add_guided(e: &Env, kind: Kind, parent: Option<i64>, guide: Option<&str>, text: &str, body: &str, paths: &[i64]) -> i64 {
        let n = NewNode {
            kind, parent_id: parent, guide_key: guide, text, body, stance: None, cites: &[],
            paper_id: None, run_id: None, report_id: None, mlflow_ref: None, dataset_ref: None, paths, created_by: "user",
        };
        store::insert_node(e.db.connection(), &n).unwrap()
    }

    fn ids(list: &[i64], kind: Kind) -> Vec<String> {
        list.iter().map(|id| fmt_id(kind, *id)).collect()
    }

    /// 軸 method の Q（下に A94 → A385）と、task・method の D を 2 つ。S1 は A385、S2 は path が無い。
    fn fixture(e: &Env) -> (i64, i64, i64) {
        let q_method = add_guided(e, Kind::Q, None, Some("method"), "手法の問い", "", &[]);
        let a94 = add(e, Kind::A, Some(q_method), "Costa 型", "", &[]);
        let a385 = add(e, Kind::A, Some(a94), "細かい手法", "", &[]);
        let body = "## 解く問題（task）\n決定: 発作部位の特定\nA: A7\n## 対象（target）\n空欄\n新規性: 患者ごとに違う\n## データ（data）\n空欄\n記載なし: 実測データを使わない\n## 手法（method）\n空欄\n## 出力（output）\n空欄\n## 評価指標（metric）\n空欄";
        let d = add(e, Kind::D, None, "題", body, &[]);
        let s1 = add(e, Kind::S, Some(d), "一つ目", "", &[a385]);
        let s2 = add(e, Kind::S, Some(d), "二つ目", "", &[]);
        (s1, s2, a385)
    }

    #[test]
    fn cells_and_same_follow_seed_and_paths() {
        let e = env("cells");
        let (s1, s2, a385) = fixture(&e);
        let got = compare(e.db.connection(), &e.roots, &Defs::default(), &ids(&[s1, s2], Kind::S)).unwrap();
        assert_eq!(got.axes, ["task", "target", "data", "method", "output", "metric"]);
        // S1 の method は A385（新規性の path）。task は D の決定の A7 を指す path が無いので空。
        let method = &got.sprouts[0].cells["method"];
        assert_eq!((method.a.clone(), method.role.as_str()), (Some(fmt_id(Kind::A, a385)), "novel"));
        assert_eq!(method.text, "細かい手法");
        assert_eq!(got.sprouts[0].cells["data"].role, "none");
        assert_eq!(got.sprouts[0].cells["data"].text, "実測データを使わない");
        assert_eq!(got.sprouts[1].cells["method"].role, "open");
        // method だけ違う。data は両方とも記載なしで a が無いので同じ。
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
