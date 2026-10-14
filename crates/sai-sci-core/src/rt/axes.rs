//! seed（D）の本文の 6 軸の状態。変換（`seedq::upgrade`）だけが読む。
//!
//! 新しい模型は本文の書式を見ない（問いと回答が正）。古い D の本文を 1 回だけ移すために、ゆるく読む（[`parse_axes`]）。

use std::collections::BTreeMap;

use rusqlite::Connection;
use serde::Serialize;

use super::store;
use super::{Kind, Result};

/// 軸の順と、本文の見出し（古い D の本文の見出し。変換だけが読む）。
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
    /// 固定の A（`A: A<n>`）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub a: Option<String>,
    /// 新規性・記載なしの理由。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

fn open() -> Axis {
    Axis { state: "open".into(), value: None, a: None, reason: None }
}

/// 見出しごとに、空でない行を分ける（見出しの次から、次の `## ` の前まで）。
pub(super) fn sections(body: &str) -> BTreeMap<&str, Vec<&str>> {
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

/// path の A の軸（一番上の軸の A の guide_key）。上が軸の A でなければ `None`。
pub(super) fn axis_of(conn: &Connection, a: i64) -> Result<Option<String>> {
    let mut cur = store::get_node(conn, a)?;
    while let Some(p) = cur.parent_id {
        cur = store::get_node(conn, p)?;
    }
    Ok(if cur.kind == Kind::A { cur.guide_key } else { None })
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
}
