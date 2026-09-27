//! Canvas を DB と同じ状態に合わらせる処理と、札・線のめやす（U02）。
//!
//! ここに置いているものは以前は TUI（`crates/sai-paper-tui/src/canvas_view.rs`）の
//! private 関数だったが、Web 版（sai-phone）から使う `sai-paper phone canvas` が
//! 同じ処理を必要にしたため、core へ移した。TUI も CLI もここを使う。
//!
//! - [`sync_nodes`]: タグの札と Canvas のノードをそろえる。消した札の位置は
//!   [`hide_node`] が `saiHidden` に残し、[`take_hidden_rect`] で出し直せる。
//! - [`canvas_lines`]: Canvas 上の札の間の**すべて**の線（引用・使った論文・
//!   元にした report・note の言及・保存された線）を 1 回で返す。
//!
//! 同じ処理を 2 か所に書くと食い違うため、実装は必ずここへ置く。

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::Path;

use rusqlite::Connection;

use crate::canvas::{Canvas, CanvasNode, CanvasNodeKind};
use crate::canvas_cards::{
    CanvasCardError, CanvasCardSource, citation_links, mentions_paper, note_path,
};
use crate::canvas_edit::{CARD_H, CARD_W};

/// 自動配置のカード間隔（ワールド座標）。
pub const AUTO_GAP_X: i64 = 120;
/// 自動配置の行間隔（ワールド座標）。
pub const AUTO_GAP_Y: i64 = 160;

/// `saiHidden` を入れる `extra` のキー。
const HIDDEN_KEY: &str = "saiHidden";

/// 札の間の線 1 本。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum LineKind {
    /// 引用・被引用（`paper_refs`）
    Citation,
    /// report が使った論文（`report_sources` の `kind='paper'`）
    Uses,
    /// 元にした report（`report_sources` の `kind='report'`）
    Derived,
    /// note.md に `#<番号>` と書かれた論文
    Mentions,
    /// `canvas.edges` に保存された線
    Link,
}

impl LineKind {
    /// JSON や CLI 出力に使う名前。
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Citation => "citation",
            Self::Uses => "uses",
            Self::Derived => "derived",
            Self::Mentions => "mentions",
            Self::Link => "link",
        }
    }
}

/// 札と札の間の線 1 本。
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct CanvasLine {
    /// 始点側のノード id
    pub from: String,
    /// 終点側のノード id
    pub to: String,
    /// 線の種類
    pub kind: LineKind,
    /// ラベル（[`LineKind::Link`] のときだけ入る）
    pub label: Option<String>,
}

/// カードの node id を内容から決める（同じカードが二重に増えないようにする）。
#[must_use]
pub fn node_id_for(source: &CanvasCardSource) -> String {
    match (&source.report_id, source.papers_id) {
        (Some(report_id), _) => format!("report-{report_id}"),
        (None, Some(paper_id)) => format!("note-{paper_id}"),
        (None, None) => format!("card-{}", uuid_like()),
    }
}

/// 予測不能な一意 id（依存追加を避けるための簡易実装）。
fn uuid_like() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{nanos:x}")
}

/// カード 1 枚分の JSON Canvas ノードを作る。
#[must_use]
pub fn card_node(node_id: String, source: &CanvasCardSource, x: i64, y: i64) -> CanvasNode {
    let mut extra = BTreeMap::new();
    extra.insert(
        "saiKind".to_string(),
        serde_json::Value::String(source.kind.as_str().to_string()),
    );
    if let Some(paper_id) = source.papers_id {
        extra.insert(
            "saiPaperId".to_string(),
            serde_json::Value::Number(paper_id.into()),
        );
    }
    if let Some(report_id) = &source.report_id {
        extra.insert(
            "saiReportId".to_string(),
            serde_json::Value::String(report_id.clone()),
        );
    }
    CanvasNode {
        id: node_id,
        x,
        y,
        width: CARD_W,
        height: CARD_H,
        color: None,
        kind: CanvasNodeKind::File {
            file: source.md_path.to_string_lossy().to_string(),
            subpath: None,
        },
        extra,
    }
}

/// 初回の自動配置（論文ごとにまとめて格子状に並べる）。
#[must_use]
pub fn auto_layout_nodes(sources: &[CanvasCardSource]) -> Vec<CanvasNode> {
    let mut nodes = Vec::new();
    let mut column: i64 = 0;
    let mut row: i64 = 0;
    let mut last_paper: Option<i64> = None;
    const COLUMNS: i64 = 3;
    for (index, source) in sources.iter().enumerate() {
        // 論文が変わったら行を進める（論文ごとのまとまりを作る）
        if last_paper.is_some() && last_paper != source.papers_id {
            column = 0;
            row += 1;
        }
        last_paper = source.papers_id;
        if index > 0 && column >= COLUMNS {
            column = 0;
            row += 1;
        }
        let x = column * (CARD_W + AUTO_GAP_X);
        let y = row * (CARD_H + AUTO_GAP_Y);
        column += 1;
        nodes.push(card_node(node_id_for(source), source, x, y));
    }
    nodes
}

/// 未使用の位置（既存カードの右端の次の列、無ければ原点）。
#[must_use]
pub fn next_free_position(nodes: &[CanvasNode]) -> (i64, i64) {
    if nodes.is_empty() {
        return (0, 0);
    }
    let min_y = nodes.iter().map(|n| n.y).min().unwrap_or(0);
    let max_x = nodes
        .iter()
        .filter(|n| n.y == min_y)
        .map(|n| n.x + n.width)
        .max()
        .unwrap_or(0);
    (max_x + AUTO_GAP_X, min_y)
}

/// `sync_nodes` で消してはいけないノードか（V02）。
///
/// - `extra.saiPinned: true` のノード（エージェントが作った地図・ユーザーの並べ直しを守る）
/// - `saiKind` を持たない `group` / `text`（ユーザーが外の道具で足したものを守る）
#[must_use]
pub fn is_protected_node(node: &CanvasNode) -> bool {
    if node
        .extra
        .get("saiPinned")
        .and_then(serde_json::Value::as_bool)
        == Some(true)
    {
        return true;
    }
    let has_kind = node.extra.contains_key("saiKind");
    !has_kind
        && matches!(
            node.kind,
            CanvasNodeKind::Group { .. } | CanvasNodeKind::Text { .. }
        )
}

/// アンカーの右隣の空いた所を返す（重なるときは下へ押し出す）。
///
/// `anchor` は `(x, y, width, height)`。
#[must_use]
pub fn place_near(nodes: &[CanvasNode], node_id: &str, anchor: (i64, i64, i64, i64)) -> (i64, i64) {
    let x = anchor.0 + anchor.2 + AUTO_GAP_X;
    let mut y = anchor.1;
    loop {
        let overlapping: Vec<&CanvasNode> = nodes
            .iter()
            .filter(|node| node.id != node_id && rects_overlap(x, y, CARD_W, CARD_H, node))
            .collect();
        if overlapping.is_empty() {
            return (x, y);
        }
        let next_y = overlapping
            .iter()
            .map(|node| node.y + node.height + AUTO_GAP_Y)
            .max()
            .unwrap_or(y);
        if next_y == y {
            return (x, y);
        }
        y = next_y;
    }
}

/// 矩形とノードが重なるか。
fn rects_overlap(x: i64, y: i64, w: i64, h: i64, node: &CanvasNode) -> bool {
    x < node.x + node.width && node.x < x + w && y < node.y + node.height && node.y < y + h
}

/// Canvas のノードを DB の状態（そのタグのカード一覧）に合わせる。
///
/// 残るノードには一切触れないので、手で動かした座標は保たれる。
/// `saiHidden`（[`hide_node`] で隠した札）にはいる札は足さない。
///
/// # Returns
///
/// 追加・削除が起きたら `true`。
pub fn sync_nodes(canvas: &mut Canvas, sources: &[CanvasCardSource]) -> bool {
    let wanted: HashSet<String> = sources.iter().map(node_id_for).collect();
    let before = canvas.nodes.len();
    // 同期で消さないノード（`saiPinned` 付き・saiKind の無い枠/文字）を残す。
    canvas
        .nodes
        .retain(|node| wanted.contains(&node.id) || is_protected_node(node));
    let mut changed = canvas.nodes.len() != before;
    let hidden = hidden_ids(canvas);
    let present: HashSet<String> = canvas.nodes.iter().map(|node| node.id.clone()).collect();
    for source in sources {
        let id = node_id_for(source);
        if present.contains(&id) || hidden.contains(&id) {
            continue;
        }
        let (x, y) = next_free_position(&canvas.nodes);
        canvas.nodes.push(card_node(id, source, x, y));
        changed = true;
    }
    changed
}

/// `saiHidden` にあるノード id の集合。
///
/// 形が壊れている要素（オブジェクトでない・数でない）は無視する。
#[must_use]
pub fn hidden_ids(canvas: &Canvas) -> BTreeSet<String> {
    let Some(object) = canvas
        .extra
        .get(HIDDEN_KEY)
        .and_then(serde_json::Value::as_object)
    else {
        return BTreeSet::new();
    };
    object
        .iter()
        .filter(|(_, rect)| hidden_rect(rect).is_some())
        .map(|(id, _)| id.clone())
        .collect()
}

/// `saiHidden` から `(x, y, width, height)` を読む（壊れていれば `None`）。
fn hidden_rect(value: &serde_json::Value) -> Option<(i64, i64, i64, i64)> {
    let object = value.as_object()?;
    let field = |name: &str| object.get(name).and_then(serde_json::Value::as_i64);
    Some((field("x")?, field("y")?, field("width")?, field("height")?))
}

/// `saiHidden` に位置を入れる（`saiHidden` が無ければ作る）。
fn put_hidden_rect(canvas: &mut Canvas, node_id: &str, rect: (i64, i64, i64, i64)) {
    let (x, y, width, height) = rect;
    let mut object = canvas
        .extra
        .get(HIDDEN_KEY)
        .and_then(serde_json::Value::as_object)
        .cloned()
        .unwrap_or_default();
    object.insert(
        node_id.to_string(),
        serde_json::json!({ "x": x, "y": y, "width": width, "height": height }),
    );
    canvas
        .extra
        .insert(HIDDEN_KEY.to_string(), serde_json::Value::Object(object));
}

/// ノードを `nodes` から外し、位置と大きさを `saiHidden` に残す。
///
/// 端に持つ `canvas.edges` の線は消さない（出し直したときに戻るように）。
///
/// # Returns
///
/// 消したノードが有れば `true`、無ければ `false`。
pub fn hide_node(canvas: &mut Canvas, node_id: &str) -> bool {
    let Some(index) = canvas.nodes.iter().position(|node| node.id == node_id) else {
        return false;
    };
    let node = canvas.nodes.remove(index);
    put_hidden_rect(canvas, node_id, (node.x, node.y, node.width, node.height));
    true
}

/// `saiHidden` からノードを外し、覚えていた位置と大きさを返す。
///
/// `saiHidden` が空になったらキーごと消す。
pub fn take_hidden_rect(canvas: &mut Canvas, node_id: &str) -> Option<(i64, i64, i64, i64)> {
    let object = canvas
        .extra
        .get(HIDDEN_KEY)
        .and_then(serde_json::Value::as_object)?
        .clone();
    let rect = hidden_rect(object.get(node_id)?);
    let mut rest = object;
    rest.remove(node_id);
    if rest.is_empty() {
        canvas.extra.remove(HIDDEN_KEY);
    } else {
        canvas
            .extra
            .insert(HIDDEN_KEY.to_string(), serde_json::Value::Object(rest));
    }
    rect
}

/// Canvas 上の**すべて**の線（引用・使った論文・元にした report・note の言及・
/// 保存された線）を `(from, to, kind)` の昇順で返す（重複は除く）。
///
/// Web 版は全部の線を出し、選んだ札の線を強調する。そのためモードごとの
/// 一部（TUI の `edges_for_mode`）ではなく、ここを 1 回で通す。
///
/// # Errors
///
/// SQLite エラー。
pub fn canvas_lines(
    conn: &Connection,
    data_dir: &Path,
    canvas: &Canvas,
) -> Result<Vec<CanvasLine>, CanvasCardError> {
    let mut paper_cards: BTreeMap<i64, &str> = BTreeMap::new();
    let mut report_cards: BTreeMap<&str, &str> = BTreeMap::new();
    for node in &canvas.nodes {
        // 論文の札は `saiKind` が note / paper のもの（report の札は saiPaperId を持つ）。
        if matches!(sai_kind(node), Some("note" | "paper")) {
            if let Some(paper_id) = sai_paper_id(node) {
                // 同じ論文の札が複数あれば並び順で最初のものを使う。
                paper_cards.entry(paper_id).or_insert(&node.id);
            }
        }
        if let Some(report_id) = sai_report_id(node) {
            report_cards.entry(report_id).or_insert(&node.id);
        }
    }

    let mut lines = Vec::new();
    let mut push = |from: &str, to: &str, kind: LineKind, label: Option<String>| {
        if from == to {
            return;
        }
        lines.push(CanvasLine {
            from: from.to_string(),
            to: to.to_string(),
            kind,
            label,
        });
    };

    // 引用・被引用（引用した論文 → 引用された論文）
    for (from_paper, to_paper) in citation_links(conn)? {
        if let (Some(from), Some(to)) = (paper_cards.get(&from_paper), paper_cards.get(&to_paper)) {
            push(from, to, LineKind::Citation, None);
        }
    }

    // report が使った論文 / 元にした report
    for (report_id, node_id) in &report_cards {
        for (kind, ref_id) in report_source_refs(conn, report_id)? {
            match kind.as_str() {
                "paper" => {
                    if let Ok(paper_id) = ref_id.parse::<i64>() {
                        if let Some(to) = paper_cards.get(&paper_id) {
                            push(node_id, to, LineKind::Uses, None);
                        }
                    }
                }
                "report" => {
                    if ref_id != *report_id {
                        if let Some(from) = report_cards.get(ref_id.as_str()) {
                            // 元にした report → 元にされた report
                            push(from, node_id, LineKind::Derived, None);
                        }
                    }
                }
                _ => {}
            }
        }
    }

    // note.md に `#<番号>` と書かれた論文（書いた論文 → 書かれた論文）
    let note_cards: Vec<(i64, &str)> = canvas
        .nodes
        .iter()
        .filter(|node| sai_kind(node) == Some("note"))
        .filter_map(|node| sai_paper_id(node).map(|id| (id, node.id.as_str())))
        .collect();
    for (paper_id, node_id) in &note_cards {
        let Some(body) = read_note_markdown(conn, data_dir, *paper_id) else {
            continue;
        };
        for (other_id, other_node) in &paper_cards {
            if *other_id != *paper_id && mentions_paper(&body, *other_id) {
                push(node_id, other_node, LineKind::Mentions, None);
            }
        }
    }

    // 保存された線（`canvas.edges`）
    let ids: HashSet<&str> = canvas.nodes.iter().map(|node| node.id.as_str()).collect();
    for edge in &canvas.edges {
        if ids.contains(edge.from_node.as_str()) && ids.contains(edge.to_node.as_str()) {
            lines.push(CanvasLine {
                from: edge.from_node.clone(),
                to: edge.to_node.clone(),
                kind: LineKind::Link,
                label: edge.label.clone(),
            });
        }
    }

    lines.sort();
    lines.dedup();
    Ok(lines)
}

/// ノードの `extra.saiKind`。
fn sai_kind(node: &CanvasNode) -> Option<&str> {
    node.extra
        .get("saiKind")
        .and_then(serde_json::Value::as_str)
}

/// ノードの `extra.saiPaperId`。
fn sai_paper_id(node: &CanvasNode) -> Option<i64> {
    node.extra
        .get("saiPaperId")
        .and_then(serde_json::Value::as_i64)
}

/// ノードの `extra.saiReportId`。
fn sai_report_id(node: &CanvasNode) -> Option<&str> {
    node.extra
        .get("saiReportId")
        .and_then(serde_json::Value::as_str)
}

/// report が参照しているもの（`report_sources` の `(kind, ref_id)`）。
fn report_source_refs(
    conn: &Connection,
    report_id: &str,
) -> Result<Vec<(String, String)>, CanvasCardError> {
    let mut stmt = conn.prepare("SELECT kind, ref_id FROM report_sources WHERE report_id = ?1")?;
    let rows = stmt
        .query_map([report_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// 論文の `note.md` を読む（無ければ `None`）。
fn read_note_markdown(conn: &Connection, data_dir: &Path, paper_id: i64) -> Option<String> {
    let title: String = conn
        .query_row(
            "SELECT COALESCE(title, '') FROM papers WHERE id = ?1",
            [paper_id],
            |row| row.get(0),
        )
        .ok()?;
    let path = note_path(data_dir, paper_id, &title)?;
    std::fs::read_to_string(path).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canvas::CanvasColor;
    use std::path::PathBuf;

    fn note_source(paper_id: i64) -> CanvasCardSource {
        CanvasCardSource {
            kind: crate::canvas_cards::CanvasCardKind::Note,
            papers_id: Some(paper_id),
            report_id: None,
            paper_title: format!("P{paper_id}"),
            tags: String::new(),
            summary: String::new(),
            md_path: PathBuf::from(format!("/tmp/{paper_id}/note.md")),
        }
    }

    fn file_node(id: &str) -> CanvasNode {
        CanvasNode {
            id: id.to_string(),
            x: 0,
            y: 0,
            width: 600,
            height: 280,
            color: None,
            kind: CanvasNodeKind::File {
                file: format!("/tmp/{id}.md"),
                subpath: None,
            },
            extra: BTreeMap::new(),
        }
    }

    /// テストごとに使い捨てのフォルダを作る。
    fn temp_dir(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "sai_canvas_sync_{label}_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// 論文の note.md を書く。
    fn write_note(dir: &Path, paper_id: i64, title: &str, body: &str) {
        let folder = crate::paths::paper_folder(dir, paper_id, title);
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join(crate::paths::paper_file_names::NOTE), body).unwrap();
    }

    /// `canvas_lines` のテスト用 DB（引用の `paper_refs` を持つ）。
    fn setup_lines() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE papers (id INTEGER PRIMARY KEY, title TEXT, pdf_path TEXT,
                 abstract TEXT, published_at TEXT, arxiv_id TEXT, doi TEXT, s2_paper_id TEXT);
             CREATE TABLE tags (id INTEGER PRIMARY KEY, name TEXT);
             CREATE TABLE paper_tags (paper_id INTEGER, tag_id INTEGER);",
        )
        .unwrap();
        conn.execute_batch(include_str!("../migrations/0028_reports.sql"))
            .unwrap();
        conn.execute_batch(
            "CREATE TABLE paper_refs (paper_id TEXT NOT NULL, ref_no INTEGER NOT NULL,
                 title TEXT, doi TEXT, s2_paper_id TEXT, papers_id INTEGER,
                 PRIMARY KEY (paper_id, ref_no));",
        )
        .unwrap();
        conn
    }

    fn add_report(conn: &Connection, dir: &Path, name: &str, sources: Vec<(&str, &str)>) -> String {
        let md = dir.join(name);
        std::fs::write(&md, format!("# {name}\n\nbody\n")).unwrap();
        crate::reports::create_report(
            conn,
            &crate::reports::NewReport {
                md_path: md.to_string_lossy().to_string(),
                sources: sources
                    .into_iter()
                    .map(|(kind, ref_id)| crate::reports::ReportSource {
                        kind: kind.to_string(),
                        ref_id: ref_id.to_string(),
                        role: None,
                    })
                    .collect(),
                ..Default::default()
            },
        )
        .unwrap()
    }

    fn line_of(from: &str, to: &str, kind: LineKind) -> CanvasLine {
        CanvasLine {
            from: from.to_string(),
            to: to.to_string(),
            kind,
            label: None,
        }
    }

    #[test]
    fn auto_layout_groups_papers_without_overlap() {
        let sources = vec![
            CanvasCardSource {
                kind: crate::canvas_cards::CanvasCardKind::Note,
                papers_id: Some(1),
                report_id: None,
                paper_title: "A".into(),
                tags: String::new(),
                summary: String::new(),
                md_path: PathBuf::from("/tmp/a/note.md"),
            },
            CanvasCardSource {
                kind: crate::canvas_cards::CanvasCardKind::Report,
                papers_id: Some(1),
                report_id: Some("R-0001".into()),
                paper_title: "A".into(),
                tags: String::new(),
                summary: String::new(),
                md_path: PathBuf::from("/tmp/a/r.md"),
            },
            CanvasCardSource {
                kind: crate::canvas_cards::CanvasCardKind::Note,
                papers_id: Some(2),
                report_id: None,
                paper_title: "B".into(),
                tags: String::new(),
                summary: String::new(),
                md_path: PathBuf::from("/tmp/b/note.md"),
            },
        ];
        let nodes = auto_layout_nodes(&sources);
        assert_eq!(nodes.len(), 3);
        // 同じ node id が重複しない
        let ids: HashSet<&String> = nodes.iter().map(|n| &n.id).collect();
        assert_eq!(ids.len(), 3);
        // どの 2 枚も重ならない
        for (i, a) in nodes.iter().enumerate() {
            for b in nodes.iter().skip(i + 1) {
                let overlap_x = a.x < b.x + b.width && b.x < a.x + a.width;
                let overlap_y = a.y < b.y + b.height && b.y < a.y + a.height;
                assert!(!(overlap_x && overlap_y), "cards overlap: {a:?} {b:?}");
            }
        }
    }

    #[test]
    fn sync_nodes_keeps_moved_positions_and_adds_removes_cards() {
        // 最初はタグに論文 1・2 が付いている状態で自動配置する。
        let mut canvas = Canvas {
            nodes: auto_layout_nodes(&[note_source(1), note_source(2)]),
            edges: Vec::new(),
            extra: BTreeMap::new(),
        };
        // 論文 1 のカードを手で動かす。
        let moved = canvas
            .nodes
            .iter_mut()
            .find(|n| n.id == "note-1")
            .expect("note-1");
        moved.x = 4242;
        moved.y = -777;

        // 論文 2 のタグが外れ、論文 3 にタグが付いた状態で開き直す。
        let changed = sync_nodes(&mut canvas, &[note_source(1), note_source(3)]);
        assert!(changed);
        let ids: HashSet<&str> = canvas.nodes.iter().map(|n| n.id.as_str()).collect();
        assert_eq!(ids, HashSet::from(["note-1", "note-3"]));
        // 手で動かした座標は保たれる。
        let kept = canvas.nodes.iter().find(|n| n.id == "note-1").unwrap();
        assert_eq!((kept.x, kept.y), (4242, -777));
        // 追加分は既存カードと重ならない位置に置かれる。
        let added = canvas.nodes.iter().find(|n| n.id == "note-3").unwrap();
        assert!(added.x >= kept.x + kept.width);

        // 変化が無ければ false（＝保存しない）。
        assert!(!sync_nodes(&mut canvas, &[note_source(1), note_source(3)]));
    }

    #[test]
    fn sync_nodes_keeps_pinned_and_loose_shapes() {
        let mut pinned = file_node("pinned-1");
        pinned
            .extra
            .insert("saiPinned".to_string(), serde_json::Value::Bool(true));

        let mut canvas = Canvas {
            nodes: vec![
                file_node("note-2"),
                pinned,
                CanvasNode {
                    id: "group-1".to_string(),
                    x: 0,
                    y: 0,
                    width: 900,
                    height: 600,
                    color: Some(CanvasColor::Preset(4)),
                    kind: CanvasNodeKind::Group {
                        label: Some("Topic".to_string()),
                        background: None,
                        background_style: None,
                    },
                    extra: BTreeMap::new(),
                },
                CanvasNode {
                    id: "text-1".to_string(),
                    x: 0,
                    y: 0,
                    width: 300,
                    height: 120,
                    color: None,
                    kind: CanvasNodeKind::Text {
                        text: "memo".to_string(),
                    },
                    extra: BTreeMap::new(),
                },
            ],
            edges: Vec::new(),
            extra: BTreeMap::new(),
        };

        assert!(sync_nodes(&mut canvas, &[note_source(1)]));
        let ids: HashSet<&str> = canvas.nodes.iter().map(|n| n.id.as_str()).collect();
        assert!(ids.contains("note-1"));
        assert!(!ids.contains("note-2"), "タグ外れの note は消える");
        assert!(ids.contains("pinned-1"), "saiPinned は消えない");
        assert!(ids.contains("group-1"), "saiKind 無しの group は消えない");
        assert!(ids.contains("text-1"), "saiKind 無しの text は消えない");
    }

    #[test]
    fn sync_nodes_skips_hidden_cards() {
        let mut canvas = Canvas {
            nodes: Vec::new(),
            edges: Vec::new(),
            extra: BTreeMap::new(),
        };
        // note-1 を隠す（位置だけが残る）。
        canvas.nodes = auto_layout_nodes(&[note_source(1)]);
        assert!(hide_node(&mut canvas, "note-1"));
        assert!(canvas.nodes.is_empty());

        // 開き直しても-hidden の札は戻らない。
        let changed = sync_nodes(&mut canvas, &[note_source(1), note_source(2)]);
        assert!(changed);
        let ids: HashSet<&str> = canvas.nodes.iter().map(|n| n.id.as_str()).collect();
        assert_eq!(ids, HashSet::from(["note-2"]));
        // 2 回目は変化が無い。
        assert!(!sync_nodes(&mut canvas, &[note_source(1), note_source(2)]));
    }

    #[test]
    fn hide_and_restore_keeps_rect() {
        let mut node = file_node("note-1");
        node.x = 720;
        node.y = 440;
        node.width = 600;
        node.height = 280;
        let mut canvas = Canvas {
            nodes: vec![node],
            edges: Vec::new(),
            extra: BTreeMap::new(),
        };

        assert!(hide_node(&mut canvas, "note-1"));
        assert!(canvas.nodes.is_empty());
        assert_eq!(hidden_ids(&canvas), BTreeSet::from(["note-1".to_string()]));
        // 線は消さない（出し直したときに戻すため）。
        assert_eq!(
            take_hidden_rect(&mut canvas, "note-1"),
            Some((720, 440, 600, 280))
        );
        // 空になったら `saiHidden` ごと消える。
        assert!(!canvas.extra.contains_key(HIDDEN_KEY));
        assert!(take_hidden_rect(&mut canvas, "note-1").is_none());
        // 無いノードは隠せない。
        assert!(!hide_node(&mut canvas, "note-1"));
    }

    #[test]
    fn hidden_ids_ignores_broken_entries() {
        let canvas = Canvas {
            nodes: Vec::new(),
            edges: Vec::new(),
            extra: BTreeMap::from([(
                HIDDEN_KEY.to_string(),
                serde_json::json!({
                    "note-1": { "x": 0, "y": 0, "width": 600, "height": 280 },
                    "note-2": { "x": 0, "y": 0, "width": 600 },
                    "note-3": "memo",
                }),
            )]),
        };
        assert_eq!(hidden_ids(&canvas), BTreeSet::from(["note-1".to_string()]));
    }

    #[test]
    fn canvas_lines_all_kinds() {
        let conn = setup_lines();
        let dir = temp_dir("lines");
        conn.execute_batch(
            "INSERT INTO papers (id, title, pdf_path, s2_paper_id) VALUES
                 (1, 'Paper A', '/tmp/a.pdf', 's2a'),
                 (2, 'Paper B', '/tmp/b.pdf', 's2b'),
                 (3, 'Paper C', '/tmp/c.pdf', 's2c');",
        )
        .unwrap();
        // 引用: 1 → 2
        conn.execute(
            "INSERT INTO paper_refs (paper_id, ref_no, s2_paper_id, papers_id)
             VALUES ('1', 1, 's2b', 1)",
            [],
        )
        .unwrap();
        // R-0001 は論文 1 を使い、R-0000 を元にした。
        let base = add_report(&conn, &dir, "base.md", vec![("paper", "3")]);
        let derived = add_report(
            &conn,
            &dir,
            "derived.md",
            vec![("paper", "1"), ("report", base.as_str())],
        );
        // 論文 1 の note が #3 を書く（論文 3 は #1 を書かない）。
        write_note(&dir, 1, "Paper A", "# Note A\n\nsee #3\n");
        write_note(&dir, 2, "Paper B", "# Note B\n");
        write_note(&dir, 3, "Paper C", "# Note C\n");

        let mut canvas = Canvas {
            nodes: vec![
                card_node("note-1".into(), &note_source(1), 0, 0),
                card_node("note-2".into(), &note_source(2), 800, 0),
                card_node("note-3".into(), &note_source(3), 1600, 0),
                card_node(
                    format!("report-{base}"),
                    &CanvasCardSource {
                        kind: crate::canvas_cards::CanvasCardKind::Report,
                        papers_id: Some(3),
                        report_id: Some(base.clone()),
                        paper_title: "Paper C".into(),
                        tags: String::new(),
                        summary: String::new(),
                        md_path: dir.join("base.md"),
                    },
                    0,
                    400,
                ),
                card_node(
                    format!("report-{derived}"),
                    &CanvasCardSource {
                        kind: crate::canvas_cards::CanvasCardKind::Report,
                        papers_id: Some(1),
                        report_id: Some(derived.clone()),
                        paper_title: "Paper A".into(),
                        tags: String::new(),
                        summary: String::new(),
                        md_path: dir.join("derived.md"),
                    },
                    800,
                    400,
                ),
            ],
            edges: vec![crate::canvas::CanvasEdge {
                id: "e1".into(),
                from_node: "note-2".into(),
                from_side: None,
                from_end: crate::canvas::CanvasEnd::None,
                to_node: "note-3".into(),
                to_side: None,
                to_end: crate::canvas::CanvasEnd::Arrow,
                color: None,
                label: Some("memo".into()),
                extra: BTreeMap::new(),
            }],
            extra: BTreeMap::new(),
        };
        // 存在しないノードへ向かう線は出さない。
        canvas.edges.push(crate::canvas::CanvasEdge {
            id: "e2".into(),
            from_node: "note-1".into(),
            from_side: None,
            from_end: crate::canvas::CanvasEnd::None,
            to_node: "note-404".into(),
            to_side: None,
            to_end: crate::canvas::CanvasEnd::Arrow,
            color: None,
            label: None,
            extra: BTreeMap::new(),
        });

        let lines = canvas_lines(&conn, &dir, &canvas).unwrap();
        let mut link = line_of("note-2", "note-3", LineKind::Link);
        link.label = Some("memo".into());
        let want = vec![
            line_of("note-1", "note-2", LineKind::Citation),
            line_of("note-1", "note-3", LineKind::Mentions),
            link,
            line_of(&format!("report-{base}"), "note-3", LineKind::Uses),
            line_of(
                &format!("report-{base}"),
                &format!("report-{derived}"),
                LineKind::Derived,
            ),
            line_of(&format!("report-{derived}"), "note-1", LineKind::Uses),
        ];
        assert_eq!(lines, want);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn line_kind_names() {
        assert_eq!(LineKind::Citation.as_str(), "citation");
        assert_eq!(LineKind::Uses.as_str(), "uses");
        assert_eq!(LineKind::Derived.as_str(), "derived");
        assert_eq!(LineKind::Mentions.as_str(), "mentions");
        assert_eq!(LineKind::Link.as_str(), "link");
    }
}
