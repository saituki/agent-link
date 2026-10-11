//! seed（D）の本文の 6 軸の状態と、SPROUT（S）の path の区分（固定・新規）。
//!
//! 状態は保存しない。D の本文と A の位置から毎回導く（D を直したときにずれないように）。
//! [`parse_axes`] はゆるく読む（古い D も読める）。[`check_axes`] は新しい書式だけを通す。

use std::collections::BTreeMap;

use rusqlite::Connection;
use serde::Serialize;

use super::ops::invalid;
use super::store;
use super::{Kind, Result, fmt_id, parse_id_of};

/// 軸の順と、本文の見出し（共有契約。見出しの文字は変えない。`sciedit.js` の `SE_SEED_AXES` と同じ）。
pub const AXES: [(&str, &str); 6] = [
    ("task", "## 解く問題（task）"),
    ("target", "## 対象（target）"),
    ("data", "## データ（data）"),
    ("method", "## 手法（method）"),
    ("output", "## 出力（output）"),
    ("metric", "## 評価指標（metric）"),
];

/// 軸 1 つの状態。`None` の項目は JSON に出さない。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Axis {
    /// `fixed`（決定）・`range`（範囲）・`novel`（空欄・新規性）・`none`（空欄・記載なし）・`open`（まだ決めていない）。
    pub state: String,
    /// 決定・範囲の値。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    /// 固定の A（`A: A<n>`）。範囲は必ず付く。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub a: Option<String>,
    /// 新規性・記載なしの理由。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// SPROUT の path 1 つの区分。保存せず、D の軸の状態から導く。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PathRole {
    pub a: String,
    /// その A の軸（一番上の Q の guide_key）。分からなければ null。
    pub axis: Option<String>,
    /// `fixed`（決まっている）か `novel`（新しく考える）。
    pub role: String,
    /// 固定の A（範囲の `a` と同じか、その子孫のとき）。無ければ null。
    pub fixed_at: Option<String>,
}

fn open() -> Axis {
    Axis { state: "open".into(), value: None, a: None, reason: None }
}

/// 見出しごとに、空でない行を分ける（見出しの次から、次の `## ` の前まで）。
fn sections(body: &str) -> BTreeMap<&str, Vec<&str>> {
    let mut out: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    let mut at: Option<&str> = None;
    for line in body.lines() {
        let t = line.trim();
        if t.starts_with("## ") {
            at = Some(t);
            out.entry(t).or_default();
        } else if let (Some(h), false) = (at, t.is_empty()) {
            out.entry(h).or_default().push(t);
        }
    }
    out
}

/// 1 軸の節（空でない行）から状態を読む。形が合わなければ `open`。
fn read_axis(lines: &[&str]) -> Axis {
    let Some((first, rest)) = lines.split_first() else { return open() };
    let anchor = || rest.iter().find_map(|l| l.strip_prefix("A:")).map(|v| v.trim().to_string()).filter(|v| !v.is_empty());
    if let Some(value) = first.strip_prefix("決定:").map(str::trim).filter(|v| !v.is_empty()) {
        return Axis { state: "fixed".into(), value: Some(value.into()), a: anchor(), reason: None };
    }
    if let Some(value) = first.strip_prefix("範囲:").map(str::trim).filter(|v| !v.is_empty()) {
        return Axis { state: "range".into(), value: Some(value.into()), a: anchor(), reason: None };
    }
    if *first == "空欄" {
        let reason = |label: &str| rest.first().and_then(|l| l.strip_prefix(label)).map(|r| r.trim().to_string()).filter(|r| !r.is_empty());
        if let Some(reason) = reason("新規性:") {
            return Axis { state: "novel".into(), value: None, a: None, reason: Some(reason) };
        }
        if let Some(reason) = reason("記載なし:") {
            return Axis { state: "none".into(), value: None, a: None, reason: Some(reason) };
        }
    }
    open()
}

/// 6 軸の状態（ゆるい読み）。見出しが無い軸・形が知らない軸は `open`。古い D も読める。
#[must_use]
pub fn parse_axes(body: &str) -> BTreeMap<String, Axis> {
    let secs = sections(body);
    AXES.iter()
        .map(|(key, heading)| ((*key).to_string(), read_axis(secs.get(heading).map_or(&[][..], Vec::as_slice))))
        .collect()
}

/// `A<n>` の形か（生きたかどうかは見ない）。
fn is_a_text(text: &str) -> bool {
    text.strip_prefix('A').is_some_and(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()))
}

/// 新しい書式の D か（きびしい検査）。6 見出しがすべてあり、1 行目が「決定: 値」「範囲: 値」「空欄」のどれかで、
/// 「範囲」には `A: A<n>` がある。エラーは利用者に見せる日本語。
pub fn check_axes(body: &str) -> std::result::Result<BTreeMap<String, Axis>, String> {
    let secs = sections(body);
    let mut out = BTreeMap::new();
    for (key, heading) in AXES {
        let name = heading.trim_start_matches("## ");
        let Some(lines) = secs.get(heading) else {
            return Err(format!("seed の本文: 見出し「{heading}」がありません"));
        };
        let first = lines.first().copied().unwrap_or("");
        let shape = || format!("seed の本文: {name}の 1 行目は「決定: 値」「範囲: 値」「空欄」のどれかです");
        if !(first.strip_prefix("決定:").is_some_and(|v| !v.trim().is_empty())
            || first.strip_prefix("範囲:").is_some_and(|v| !v.trim().is_empty())
            || first == "空欄")
        {
            return Err(shape());
        }
        let axis = read_axis(lines);
        if axis.state == "range" && axis.a.is_none() {
            return Err(format!("seed の本文: {name}は「範囲」なので、次の行に「A: A<n>」が要ります"));
        }
        if let Some(a) = axis.a.as_deref().filter(|a| !is_a_text(a)) {
            return Err(format!("seed の本文: {name}の A: は A<n> の形で書いてください: {a}"));
        }
        out.insert(key.to_string(), axis);
    }
    Ok(out)
}

/// 書式の検査（[`check_axes`]）と、`A:` に書いた ID が生きた A であることの確認。
pub fn check_seed_body(conn: &Connection, body: &str) -> Result<BTreeMap<String, Axis>> {
    let axes = check_axes(body).or_else(invalid)?;
    for (key, axis) in &axes {
        let Some(a) = axis.a.as_deref() else { continue };
        let name = AXES.iter().find(|(k, _)| *k == key.as_str()).map_or(key.as_str(), |(_, h)| h.trim_start_matches("## "));
        let id = parse_id_of(a, Kind::A).or_else(|_| invalid(format!("seed の本文: {name}の A: は A<n> の形で書いてください: {a}")))?;
        match store::get_node_of(conn, Kind::A, id) {
            Ok(n) if n.removed_at.is_none() => {}
            _ => return invalid(format!("seed の本文: {name}の A: {a} はありません")),
        }
    }
    Ok(axes)
}

/// path の A の軸（一番上の Q の guide_key）。上が Q でなければ `None`。
fn axis_of(conn: &Connection, a: i64) -> Result<Option<String>> {
    let mut cur = store::get_node(conn, a)?;
    while let Some(p) = cur.parent_id {
        cur = store::get_node(conn, p)?;
    }
    Ok(if cur.kind == Kind::Q { cur.guide_key } else { None })
}

/// path 1 つの区分（`axes` は [`parse_axes`] か [`check_axes`] の結果）。
fn role_of(conn: &Connection, axes: &BTreeMap<String, Axis>, a: i64) -> Result<PathRole> {
    let axis = axis_of(conn, a)?;
    let state = axis.as_deref().and_then(|k| axes.get(k));
    let anchor = state
        .filter(|s| s.state == "range")
        .and_then(|s| s.a.as_deref())
        .and_then(|t| parse_id_of(t, Kind::A).ok());
    let (role, fixed_at) = match (state.map(|s| s.state.as_str()), anchor) {
        (Some("fixed"), _) => ("fixed", None),
        (Some("range"), Some(anchor)) if anchor == a => ("fixed", Some(fmt_id(Kind::A, anchor))),
        (Some("range"), Some(anchor)) if super::ops::is_under(conn, a, anchor)? => {
            ("novel", Some(fmt_id(Kind::A, anchor)))
        }
        _ => ("novel", None),
    };
    Ok(PathRole { a: fmt_id(Kind::A, a), axis, role: role.into(), fixed_at })
}

/// S の paths の区分（D の本文を [`parse_axes`] で読む。保存しない）。
pub fn path_roles(conn: &Connection, seed_body: &str, paths: &[i64]) -> Result<Vec<PathRole>> {
    let axes = parse_axes(seed_body);
    paths.iter().map(|&a| role_of(conn, &axes, a)).collect()
}

/// 新しい書式の D に対する S の path の検査（`check_axes` が通った D だけ呼ぶ）。区分を返す。
pub fn check_paths(conn: &Connection, axes: &BTreeMap<String, Axis>, paths: &[i64]) -> Result<Vec<PathRole>> {
    let roles: Vec<PathRole> = paths.iter().map(|&a| role_of(conn, axes, a)).collect::<Result<_>>()?;
    for r in &roles {
        let Some(key) = r.axis.as_deref() else { continue };
        let Some(state) = axes.get(key) else { continue };
        match state.state.as_str() {
            "none" => return invalid(format!("{key} は「記載なし」の軸なので path を置けません")),
            "range" if r.fixed_at.is_none() => {
                let anchor = state.a.as_deref().unwrap_or_default();
                return invalid(format!("{key} の path {} は {anchor} の下にありません（範囲: {anchor}）", r.a));
            }
            _ => {}
        }
    }
    for (key, _) in AXES {
        let Some(state) = axes.get(key) else { continue };
        if state.state == "novel" && !roles.iter().any(|r| r.axis.as_deref() == Some(key)) {
            let name = AXES.iter().find(|(k, _)| *k == key).map_or(key, |(_, h)| h.trim_start_matches("## "));
            return invalid(format!("{key} は新規性の軸ですが path がありません（{name}）"));
        }
    }
    Ok(roles)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 5 つの状態を全部持つ本文（軸の見出しは 6 つそろえる）。
    fn body() -> String {
        [
            "## 背景", "自由な文",
            "## 解く問題（task）", "決定: 発作部位の特定", "A: A7",
            "## 対象（target）", "空欄", "新規性: 患者ごとに違う",
            "## データ（data）", "空欄", "記載なし: 実測データを使わない",
            "## 手法（method）", "範囲: Costa 型の視床皮質神経質量モデルを土台にする", "A: A94",
            "## 出力（output）", "空欄",
            "## 評価指標（metric）", "空欄",
            "## 成功基準", "空欄",
        ]
        .join("\n")
    }

    #[test]
    fn five_states_read_loosely() {
        let axes = parse_axes(&body());
        assert_eq!(axes["task"], Axis { state: "fixed".into(), value: Some("発作部位の特定".into()), a: Some("A7".into()), reason: None });
        assert_eq!(axes["target"].state, "novel");
        assert_eq!(axes["target"].reason.as_deref(), Some("患者ごとに違う"));
        assert_eq!(axes["data"].state, "none");
        assert_eq!(axes["method"], Axis {
            state: "range".into(),
            value: Some("Costa 型の視床皮質神経質量モデルを土台にする".into()),
            a: Some("A94".into()),
            reason: None,
        });
        assert_eq!(axes["output"], open());
        // 空の値の「決定:」は決めていない（open）。
        assert_eq!(parse_axes("## 手法（method）\n決定:\n")["method"], open());
    }

    #[test]
    fn old_format_reads_as_open() {
        // 古い書式（見出しが無い・1 行目が知らない形）も落とさず、6 軸すべて open で読む。
        let axes = parse_axes("## 背景\n前の研究の続き\n## 手法（method）\n手法は未定です");
        assert_eq!(axes.len(), 6);
        assert!(axes.values().all(|a| a.state == "open"));
    }

    #[test]
    fn json_leaves_out_empty_items() {
        let axes = parse_axes(&body());
        let value = serde_json::to_value(&axes["data"]).unwrap();
        assert_eq!(value, serde_json::json!({"state": "none", "reason": "実測データを使わない"}));
        let value = serde_json::to_value(&axes["output"]).unwrap();
        assert_eq!(value, serde_json::json!({"state": "open"}));
    }

    #[test]
    fn check_accepts_new_format_and_rejects_bad_ones() {
        assert_eq!(check_axes(&body()).unwrap()["method"].state, "range");
        // 見出しが 1 つでも無ければ断る。
        let missing = body().replace("## 出力（output）\n", "");
        assert!(check_axes(&missing).unwrap_err().contains("見出し「## 出力（output）」がありません"));
        // 1 行目が 3 つの形のどれでもなければ断る。
        let shape = body().replace("## 手法（method）\n範囲: Costa 型の視床皮質神経質量モデルを土台にする", "## 手法（method）\n手法: 未定");
        assert!(check_axes(&shape).unwrap_err().contains("手法（method）の 1 行目は「決定: 値」「範囲: 値」「空欄」のどれかです"));
        // 「範囲」に A: が無いと断る。
        let no_a = body().replace("\nA: A94", "");
        assert!(check_axes(&no_a).unwrap_err().contains("手法（method）は「範囲」なので"));
        // A: の値は A<n> の形。
        let bad_a = body().replace("A: A94", "A: 94");
        assert!(check_axes(&bad_a).unwrap_err().contains("A<n> の形"));
    }
}
