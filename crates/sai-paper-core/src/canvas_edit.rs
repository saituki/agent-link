//! Canvas を組み立てる部品（エージェントが CLI から使う、V03）。
//!
//! エージェントは JSON を直接書かず、`sai-paper canvas` を通して Canvas を
//! 作る。このモジュールはその土台で、読み書き（[`load_or_new`] / [`save`]）と
//! ノード・線の追加（`add_*` / [`link`]）を提供する。
//!
//! ノードの id・種類・大きさは TUI（`crates/sai-paper-tui/src/canvas_view.rs` の
//! `card_node` / `node_id_for`）と揃える。揃えないと TUI の同期で同じカードが
//! 二重に増えるためである。CLI が足すノードには必ず `saiPinned: true` を付ける
//! （TUI の `sync_nodes` が消さないようにする）。
//!
//! 置き場所は決まった簡単なものにする。グループは横に並べ、中の札は縦に積む。
//! グループに入れない札はグループの下に横に並べる。既にあるノードは動かさず、
//! 足すノードだけを空いている所に置く。

use crate::canvas::{
    Canvas, CanvasColor, CanvasEdge, CanvasEnd, CanvasNode, CanvasNodeKind, canvas_path_for_tag,
    parse_canvas,
};
use crate::understanding::UnderstandingEntry;
use std::collections::BTreeMap;
use std::path::Path;
use thiserror::Error;

/// 札（note / report / paper）の既定の幅。TUI の `DEFAULT_CARD_W` と同じ。
pub const CARD_W: i64 = 600;
/// 札（note / report / paper）の既定の高さ。TUI の `DEFAULT_CARD_H` と同じ。
pub const CARD_H: i64 = 280;
/// グループの中の札を縦に並べるときの間隔。
pub const CARD_GAP_Y: i64 = 40;
/// グループを横に並べるときの間隔。
pub const GROUP_GAP_X: i64 = 120;
/// グループの内側の余白（見出し・札のまわり）。
pub const GROUP_PAD: i64 = 40;
/// グループに入れない札を横に並べるときの間隔。
pub const UNGROUPED_GAP_X: i64 = 120;
/// グループに入れない札をグループの下に置くときの間隔。
pub const UNGROUPED_GAP_Y: i64 = 160;
/// 札が 1 枚も無いグループの高さ（見出しが入る大きさ）。
pub const GROUP_MIN_H: i64 = 200;

/// Canvas の組み立てに失敗した。
#[derive(Debug, Error)]
pub enum CanvasEditError {
    /// ファイルの読み書きに失敗した。
    #[error("canvas ファイルの読み書きに失敗しました: {0}")]
    Io(#[from] std::io::Error),
    /// JSON Canvas として読めなかった。
    #[error("canvas の解析に失敗しました: {0}")]
    Parse(String),
    /// タグ名を Canvas のファイル名にできなかった。
    #[error("タグ '{0}' は Canvas 名にできません")]
    InvalidTag(String),
    /// JSON の生成に失敗した。
    #[error("JSON の生成に失敗しました: {0}")]
    Json(#[from] serde_json::Error),
    /// 指定したノードが Canvas に無い。
    #[error("ノード '{0}' が Canvas にありません")]
    NodeNotFound(String),
    /// 知らない色の名前。
    #[error("色の名前 '{0}' を知りません（red/orange/yellow/green/cyan/purple/gray）")]
    UnknownColor(String),
    /// 下書きでない Canvas を書き換えようとした。
    #[error("下書きではない Canvas です。ユーザーの Canvas に足すには --force を付けてください")]
    NotDraft,
}

/// タグ名に対応する Canvas を読む（ファイルが無ければ空の Canvas）。
///
/// 読み書きの向き先は [`canvas_path_for_tag`] が決める。読んだ・作った Canvas に
/// は必ず `saiTag`（タグ名）を入れる。ファイルからタグへ戻せるようにするため。
///
/// # Errors
///
/// タグがファイル名にできない場合、読み込みに失敗した場合。
pub fn load_or_new(data_dir: &Path, tag: &str) -> Result<Canvas, CanvasEditError> {
    let path = canvas_path_for_tag(data_dir, tag);
    if path.as_os_str().is_empty() {
        return Err(CanvasEditError::InvalidTag(tag.to_string()));
    }
    let mut canvas = if path.is_file() {
        let source = std::fs::read_to_string(&path)?;
        parse_canvas(&source)
            .map(|(canvas, _diagnostics)| canvas)
            .map_err(|error| CanvasEditError::Parse(error.to_string()))?
    } else {
        empty_canvas()
    };
    canvas.extra.insert(
        "saiTag".to_string(),
        serde_json::Value::String(tag.to_string()),
    );
    Ok(canvas)
}

/// 空の Canvas を作る（`sai-paper canvas new` 用）。
///
/// `draft` が `true` なら `saiDraft: true` を付ける。付いていない Canvas は
/// 「ユーザーの Canvas」とみなし、書き換えには `--force` が要る。
#[must_use]
pub fn new_canvas(tag: &str, draft: bool) -> Canvas {
    let mut canvas = empty_canvas();
    canvas.extra.insert(
        "saiTag".to_string(),
        serde_json::Value::String(tag.to_string()),
    );
    if draft {
        canvas
            .extra
            .insert("saiDraft".to_string(), serde_json::Value::Bool(true));
    }
    canvas
}

/// 空の Canvas（ノードも線も無い）。
fn empty_canvas() -> Canvas {
    Canvas {
        nodes: Vec::new(),
        edges: Vec::new(),
        extra: BTreeMap::new(),
    }
}

/// Canvas を `.canvas` へ保存する（一時ファイル → rename。書きかけを残さない）。
///
/// # Errors
///
/// タグがファイル名にできない場合、書き込みに失敗した場合。
pub fn save(data_dir: &Path, tag: &str, canvas: &Canvas) -> Result<(), CanvasEditError> {
    let path = canvas_path_for_tag(data_dir, tag);
    if path.as_os_str().is_empty() {
        return Err(CanvasEditError::InvalidTag(tag.to_string()));
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json = canvas.to_json_pretty()?;
    let tmp = path.with_extension("canvas.tmp");
    std::fs::write(&tmp, json)?;
    std::fs::rename(&tmp, &path)?;
    Ok(())
}

/// Canvas が下書き（`extra.saiDraft: true`）か。
#[must_use]
pub fn is_draft(canvas: &Canvas) -> bool {
    canvas
        .extra
        .get("saiDraft")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
}

/// 下書きでない Canvas への追加を許すか確かめる。
///
/// エージェントがユーザーの Canvas を書き換えないための歯止め。`force` が
/// `true` なら常に許す。
///
/// # Errors
///
/// 下書きでなく `force` も `false` のとき [`CanvasEditError::NotDraft`]。
pub fn ensure_editable(canvas: &Canvas, force: bool) -> Result<(), CanvasEditError> {
    if force || is_draft(canvas) {
        Ok(())
    } else {
        Err(CanvasEditError::NotDraft)
    }
}

/// 色の名前を [`CanvasColor`] にする（`gray` は色なしで `Ok(None)`）。
///
/// # Errors
///
/// 知らない名前のとき [`CanvasEditError::UnknownColor`]。
pub fn parse_color_name(name: &str) -> Result<Option<CanvasColor>, CanvasEditError> {
    let preset = match name {
        "red" => Some(1),
        "orange" => Some(2),
        "yellow" => Some(3),
        "green" => Some(4),
        "cyan" => Some(5),
        "purple" => Some(6),
        "gray" | "grey" => return Ok(None),
        other => return Err(CanvasEditError::UnknownColor(other.to_string())),
    };
    Ok(preset.map(CanvasColor::Preset))
}

/// `extra` に `saiPinned: true` を入れる（TUI の同期で消えないようにする）。
fn pinned_extra() -> BTreeMap<String, serde_json::Value> {
    BTreeMap::from([("saiPinned".to_string(), serde_json::Value::Bool(true))])
}

/// グループの枠を追加して id を返す。
///
/// `color` が `None` なら色なし。`topic` は `saiTopic` に入れる。`color` を
/// 指定したときは `saiColorFixed: true` を付け、TUI が開くたびに理解の記録から
/// 色を決め直さないようにする（V05）。
#[must_use]
pub fn add_group(
    canvas: &mut Canvas,
    label: &str,
    color: Option<CanvasColor>,
    topic: Option<&str>,
) -> String {
    let color_fixed = color.is_some();
    add_group_inner(canvas, label, color, topic, color_fixed)
}

/// 理解の記録から色を決めてグループの枠を追加して id を返す（V05）。
///
/// `state` が `ok` なら緑、`vague` なら黄、それ以外（`unknown` / 記録なし）は
/// 色なし。`saiColorFixed` は付けない（TUI が開くたびに決め直す）。
#[must_use]
pub fn add_group_from_understanding(
    canvas: &mut Canvas,
    label: &str,
    topic: &str,
    state: Option<&str>,
) -> String {
    add_group_inner(
        canvas,
        label,
        understanding_color(state),
        Some(topic),
        false,
    )
}

/// グループの枠を追加する本体（色を固定するかを明示する）。
fn add_group_inner(
    canvas: &mut Canvas,
    label: &str,
    color: Option<CanvasColor>,
    topic: Option<&str>,
    color_fixed: bool,
) -> String {
    let id = next_numbered_id(canvas, "group");
    // 既にあるグループの右端の次に置く（横に並べる）。
    let x = canvas
        .nodes
        .iter()
        .filter(|node| is_group(node))
        .map(|node| node.x + node.width)
        .max()
        .map_or(0, |right| right + GROUP_GAP_X);
    let mut extra = pinned_extra();
    if let Some(topic) = topic {
        extra.insert(
            "saiTopic".to_string(),
            serde_json::Value::String(topic.to_string()),
        );
    }
    if color_fixed {
        extra.insert("saiColorFixed".to_string(), serde_json::Value::Bool(true));
    }
    canvas.nodes.push(CanvasNode {
        id: id.clone(),
        x,
        y: 0,
        width: CARD_W + GROUP_PAD * 2,
        height: GROUP_MIN_H,
        color,
        kind: CanvasNodeKind::Group {
            label: Some(label.to_string()),
            background: None,
            background_style: None,
        },
        extra,
    });
    id
}

/// 理解の状態からグループの色を決める（V05）。
///
/// - `ok`（理解している）→ `"4"` 緑
/// - `vague`（あいまい）→ `"3"` 黄
/// - `unknown` または記録なし → 色なし（灰）
#[must_use]
pub fn understanding_color(state: Option<&str>) -> Option<CanvasColor> {
    match state {
        Some("ok") => Some(CanvasColor::Preset(4)),
        Some("vague") => Some(CanvasColor::Preset(3)),
        _ => None,
    }
}

/// 理解の記録から「トピック名 → 状態」の対応を作る（V05）。
///
/// [`UnderstandingEntry`] の並び（[`crate::understanding::latest_per_topic`] の
/// 結果など）から、トピックごとに 1 つの状態を引ける表を作る。
#[must_use]
pub fn states_by_topic(entries: &[UnderstandingEntry]) -> BTreeMap<String, String> {
    entries
        .iter()
        .map(|entry| (entry.topic.clone(), entry.state.clone()))
        .collect()
}

/// グループの色を理解の記録から決め直す（V05）。
///
/// `saiTopic` を持ち `saiColorFixed` の無いグループだけを対象にする。
/// `saiColorFixed` のグループ（ユーザーや `--color` が決めた色）は触らない。
/// 1 つでも色が変わったら `true` を返す（呼び出し側が保存に使う）。
#[must_use]
pub fn refresh_topic_colors(canvas: &mut Canvas, states: &BTreeMap<String, String>) -> bool {
    let mut changed = false;
    for node in &mut canvas.nodes {
        if !is_group(node) {
            continue;
        }
        if node
            .extra
            .get("saiColorFixed")
            .and_then(serde_json::Value::as_bool)
            == Some(true)
        {
            continue;
        }
        let Some(topic) = node
            .extra
            .get("saiTopic")
            .and_then(serde_json::Value::as_str)
        else {
            continue;
        };
        let color = understanding_color(states.get(topic).map(String::as_str));
        if node.color != color {
            node.color = color;
            changed = true;
        }
    }
    changed
}

/// 論文の札（note または paper）を追加して id を返す。
///
/// `note_md` があれば note の札（`saiKind: "note"`、`file` は note.md のパス）、
/// 無ければ論文の札（`saiKind: "paper"`、`file` は空文字）。
/// 同じ論文の札が既にあれば足さず、その id を返す。
#[must_use]
pub fn add_paper(
    canvas: &mut Canvas,
    paper_id: i64,
    note_md: Option<&Path>,
    group: Option<&str>,
) -> String {
    if let Some(existing) = existing_paper_card(canvas, paper_id) {
        return existing;
    }
    match note_md {
        Some(note_md) => push_note_card(canvas, paper_id, note_md, group),
        None => push_paper_card(canvas, paper_id, group),
    }
}

/// note の札（`note.md`）を追加して id を返す。
///
/// 同じ論文の札が既にあれば足さず、その id を返す。
#[must_use]
pub fn add_note(canvas: &mut Canvas, paper_id: i64, note_md: &Path, group: Option<&str>) -> String {
    if let Some(existing) = existing_paper_card(canvas, paper_id) {
        return existing;
    }
    push_note_card(canvas, paper_id, note_md, group)
}

/// report の札を追加して id を返す。
///
/// 同じ report の札が既にあれば足さず、その id を返す。`md_path` は札の `file`
/// （TUI が表示に使う。無くても `saiReportId` から DB で引ける）。
#[must_use]
pub fn add_report(
    canvas: &mut Canvas,
    report_id: &str,
    md_path: Option<&Path>,
    group: Option<&str>,
) -> String {
    let id = format!("report-{report_id}");
    if canvas.nodes.iter().any(|node| node.id == id) {
        return id;
    }
    if let Some(existing) = existing_report_card(canvas, report_id) {
        return existing;
    }
    let mut extra = pinned_extra();
    extra.insert(
        "saiKind".to_string(),
        serde_json::Value::String("report".to_string()),
    );
    extra.insert(
        "saiReportId".to_string(),
        serde_json::Value::String(report_id.to_string()),
    );
    let node = CanvasNode {
        id: id.clone(),
        x: 0,
        y: 0,
        width: CARD_W,
        height: CARD_H,
        color: None,
        kind: CanvasNodeKind::File {
            file: md_path
                .map(|path| path.to_string_lossy().to_string())
                .unwrap_or_default(),
            subpath: None,
        },
        extra,
    };
    place_card(canvas, node, group);
    id
}

/// 文字の札を追加して id を返す。
#[must_use]
pub fn add_text(canvas: &mut Canvas, text: &str, group: Option<&str>) -> String {
    let id = next_numbered_id(canvas, "text");
    let node = CanvasNode {
        id: id.clone(),
        x: 0,
        y: 0,
        width: CARD_W,
        height: CARD_H,
        color: None,
        kind: CanvasNodeKind::Text {
            text: text.to_string(),
        },
        extra: pinned_extra(),
    };
    place_card(canvas, node, group);
    id
}

/// 2 つのノードを線でつなぎ、線の id を返す。
///
/// # Errors
///
/// `from` または `to` のノードが Canvas に無いとき [`CanvasEditError::NodeNotFound`]。
pub fn link(
    canvas: &mut Canvas,
    from: &str,
    to: &str,
    label: Option<&str>,
) -> Result<String, CanvasEditError> {
    if !canvas.nodes.iter().any(|node| node.id == from) {
        return Err(CanvasEditError::NodeNotFound(from.to_string()));
    }
    if !canvas.nodes.iter().any(|node| node.id == to) {
        return Err(CanvasEditError::NodeNotFound(to.to_string()));
    }
    let id = next_edge_id(canvas);
    canvas.edges.push(CanvasEdge {
        id: id.clone(),
        from_node: from.to_string(),
        from_side: None,
        from_end: CanvasEnd::None,
        to_node: to.to_string(),
        to_side: None,
        to_end: CanvasEnd::Arrow,
        color: None,
        label: label.map(str::to_string),
        extra: BTreeMap::new(),
    });
    Ok(id)
}

/// 同じ論文の札（note / paper）が既にあればその id。
#[must_use]
pub fn existing_paper_card(canvas: &Canvas, paper_id: i64) -> Option<String> {
    canvas
        .nodes
        .iter()
        .find(|node| {
            let kind = node
                .extra
                .get("saiKind")
                .and_then(serde_json::Value::as_str);
            let same_paper = node
                .extra
                .get("saiPaperId")
                .and_then(serde_json::Value::as_i64)
                == Some(paper_id);
            same_paper && matches!(kind, Some("note") | Some("paper"))
        })
        .map(|node| node.id.clone())
}

/// 同じ report の札が既にあればその id。
#[must_use]
pub fn existing_report_card(canvas: &Canvas, report_id: &str) -> Option<String> {
    canvas
        .nodes
        .iter()
        .find(|node| {
            node.extra
                .get("saiReportId")
                .and_then(serde_json::Value::as_str)
                == Some(report_id)
        })
        .map(|node| node.id.clone())
}

/// note の札を実際に積む。
fn push_note_card(
    canvas: &mut Canvas,
    paper_id: i64,
    note_md: &Path,
    group: Option<&str>,
) -> String {
    let id = format!("note-{paper_id}");
    let mut extra = pinned_extra();
    extra.insert(
        "saiKind".to_string(),
        serde_json::Value::String("note".to_string()),
    );
    extra.insert(
        "saiPaperId".to_string(),
        serde_json::Value::Number(paper_id.into()),
    );
    let node = CanvasNode {
        id: id.clone(),
        x: 0,
        y: 0,
        width: CARD_W,
        height: CARD_H,
        color: None,
        kind: CanvasNodeKind::File {
            file: note_md.to_string_lossy().to_string(),
            subpath: None,
        },
        extra,
    };
    place_card(canvas, node, group);
    id
}

/// 論文の札（note.md が無いときの札）を実際に積む。
fn push_paper_card(canvas: &mut Canvas, paper_id: i64, group: Option<&str>) -> String {
    let id = format!("paper-{paper_id}");
    let mut extra = pinned_extra();
    extra.insert(
        "saiKind".to_string(),
        serde_json::Value::String("paper".to_string()),
    );
    extra.insert(
        "saiPaperId".to_string(),
        serde_json::Value::Number(paper_id.into()),
    );
    let node = CanvasNode {
        id: id.clone(),
        x: 0,
        y: 0,
        width: CARD_W,
        height: CARD_H,
        color: None,
        kind: CanvasNodeKind::File {
            file: String::new(),
            subpath: None,
        },
        extra,
    };
    place_card(canvas, node, group);
    id
}

/// ノードを空いている所へ置く。
///
/// `group` のグループが Canvas にあれば、その内側の空き（既にある札の下）に置き、
/// グループの高さを伸ばす。無ければグループの下に横に並べる。既にあるノードの
/// 座標は変えない。
fn place_card(canvas: &mut Canvas, mut node: CanvasNode, group: Option<&str>) {
    let group_frame = group.and_then(|group_id| {
        canvas
            .nodes
            .iter()
            .find(|candidate| candidate.id == group_id && is_group(candidate))
            .map(|candidate| (candidate.id.clone(), candidate.x, candidate.y))
    });
    let (x, y, group_id) = match group_frame {
        Some((group_id, group_x, group_y)) => {
            let (x, y) = find_free_spot(
                canvas,
                group_x + GROUP_PAD,
                group_y + GROUP_PAD,
                node.width,
                node.height,
                &[group_id.as_str()],
            );
            (x, y, Some(group_id))
        }
        None => {
            // グループの下に横に並べる。
            let start_y = canvas
                .nodes
                .iter()
                .filter(|candidate| is_group(candidate))
                .map(|candidate| candidate.y + candidate.height)
                .max()
                .map_or(0, |bottom| bottom + UNGROUPED_GAP_Y);
            let start_x = canvas
                .nodes
                .iter()
                .filter(|candidate| !is_group(candidate) && candidate.y >= start_y)
                .map(|candidate| candidate.x + candidate.width)
                .max()
                .map_or(0, |right| right + UNGROUPED_GAP_X);
            let (x, y) = find_free_spot(canvas, start_x, start_y, node.width, node.height, &[]);
            (x, y, None)
        }
    };
    node.x = x;
    node.y = y;
    let height = node.height;
    canvas.nodes.push(node);
    if let Some(group_id) = group_id {
        grow_group(canvas, &group_id, y, height);
    }
}

/// グループを伸ばして、置いた札（`child_y` から高さ `child_h`）が枠に入るようにする。
fn grow_group(canvas: &mut Canvas, group_id: &str, child_y: i64, child_h: i64) {
    if let Some(group) = canvas.nodes.iter_mut().find(|node| node.id == group_id) {
        let needed = child_y + child_h + GROUP_PAD - group.y;
        if needed > group.height {
            group.height = needed;
        }
    }
}

/// 矩形が重ならない `y` を探す（`x` は固定。下へ押し出す）。
///
/// `ignore` に入れた id のノードは重なり判定から除く（グループの枠自身など）。
fn find_free_spot(
    canvas: &Canvas,
    x: i64,
    mut y: i64,
    width: i64,
    height: i64,
    ignore: &[&str],
) -> (i64, i64) {
    loop {
        let mut next_y = y;
        for node in &canvas.nodes {
            if ignore.contains(&node.id.as_str()) {
                continue;
            }
            if overlaps(x, y, width, height, node) {
                next_y = next_y.max(node.y + node.height + CARD_GAP_Y);
            }
        }
        if next_y == y {
            return (x, y);
        }
        y = next_y;
    }
}

/// 2 つの矩形が重なるか。
fn overlaps(x: i64, y: i64, width: i64, height: i64, node: &CanvasNode) -> bool {
    x < node.x + node.width && node.x < x + width && y < node.y + node.height && node.y < y + height
}

/// そのノードがグループの枠か。
fn is_group(node: &CanvasNode) -> bool {
    matches!(node.kind, CanvasNodeKind::Group { .. })
}

/// `prefix-<番号>` の空いている番号を返す（`group-1` / `text-1`）。
fn next_numbered_id(canvas: &Canvas, prefix: &str) -> String {
    let head = format!("{prefix}-");
    let max = canvas
        .nodes
        .iter()
        .filter_map(|node| node.id.strip_prefix(&head))
        .filter_map(|rest| rest.parse::<i64>().ok())
        .max()
        .unwrap_or(0);
    format!("{prefix}-{}", max + 1)
}

/// `edge-<番号>` の空いている番号を返す。
fn next_edge_id(canvas: &Canvas) -> String {
    let max = canvas
        .edges
        .iter()
        .filter_map(|edge| edge.id.strip_prefix("edge-"))
        .filter_map(|rest| rest.parse::<i64>().ok())
        .max()
        .unwrap_or(0);
    format!("edge-{}", max + 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::paper_file_names;

    fn temp_dir(label: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "sai_canvas_edit_{label}_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn card_rects(canvas: &Canvas) -> Vec<(String, i64, i64, i64, i64)> {
        canvas
            .nodes
            .iter()
            .filter(|node| !is_group(node))
            .map(|node| (node.id.clone(), node.x, node.y, node.width, node.height))
            .collect()
    }

    fn assert_no_card_overlap(canvas: &Canvas) {
        let cards = card_rects(canvas);
        for (index, a) in cards.iter().enumerate() {
            for b in &cards[index + 1..] {
                let overlap =
                    a.1 < b.1 + b.3 && b.1 < a.1 + a.3 && a.2 < b.2 + b.4 && b.2 < a.2 + a.4;
                assert!(!overlap, "{} と {} が重なっています", a.0, b.0);
            }
        }
    }

    #[test]
    fn layout_does_not_overlap_and_keeps_existing_positions() {
        let mut canvas = new_canvas("map-test", true);
        let g1 = add_group(
            &mut canvas,
            "前処理",
            parse_color_name("yellow").unwrap(),
            Some("前処理の違い"),
        );
        let g2 = add_group(
            &mut canvas,
            "モデル",
            parse_color_name("cyan").unwrap(),
            None,
        );
        let g3 = add_group(&mut canvas, "評価", None, None);
        // グループ 3 つに札を 7 枚（3 + 2 + 2）。
        for paper_id in 1..=3 {
            let _ = add_paper(&mut canvas, paper_id, None, Some(&g1));
        }
        for paper_id in 4..=5 {
            let _ = add_paper(&mut canvas, paper_id, None, Some(&g2));
        }
        for report_index in 6..=7 {
            let _ = add_report(
                &mut canvas,
                &format!("R-{report_index:04}"),
                None,
                Some(&g3),
            );
        }
        assert_no_card_overlap(&canvas);
        assert_eq!(card_rects(&canvas).len(), 7);

        // ここまでの札の座標を覚えて、さらに足しても動かないことを確かめる。
        let before = card_rects(&canvas);
        let _ = add_text(&mut canvas, "研究の量 高 / 手薄 低", Some(&g1));
        let _ = add_paper(&mut canvas, 99, None, None);
        assert_no_card_overlap(&canvas);
        let after = card_rects(&canvas);
        for (id, x, y, w, h) in &before {
            let found = after
                .iter()
                .find(|card| &card.0 == id)
                .unwrap_or_else(|| panic!("{id} が消えている"));
            assert_eq!(
                (found.1, found.2, found.3, found.4),
                (*x, *y, *w, *h),
                "{id} が動いた"
            );
        }
        // グループの枠は横に並び、互いに重ならない。
        let groups: Vec<_> = canvas.nodes.iter().filter(|n| is_group(n)).collect();
        for (index, a) in groups.iter().enumerate() {
            for b in &groups[index + 1..] {
                assert!(
                    a.x + a.width <= b.x || b.x + b.width <= a.x,
                    "{} と {} が横に重なっています",
                    a.id,
                    b.id
                );
            }
        }

        // 同じ論文を 2 回足しても 1 枚。
        let count_before = canvas.nodes.len();
        let first = add_paper(&mut canvas, 1, None, Some(&g1));
        let second = add_paper(&mut canvas, 1, None, Some(&g2));
        assert_eq!(first, "paper-1");
        assert_eq!(second, "paper-1");
        assert_eq!(canvas.nodes.len(), count_before);
    }

    #[test]
    fn note_card_uses_note_kind_and_file() {
        let dir = temp_dir("note");
        let note_dir = dir.join("papers").join("0001_demo");
        std::fs::create_dir_all(&note_dir).unwrap();
        let note = note_dir.join(paper_file_names::NOTE);
        std::fs::write(&note, "# タイトル\n\n本文\n").unwrap();

        let mut canvas = new_canvas("topic", true);
        let id = add_paper(&mut canvas, 1, Some(&note), None);
        assert_eq!(id, "note-1");
        let node = canvas.nodes.iter().find(|n| n.id == "note-1").unwrap();
        assert_eq!(node.extra.get("saiKind").unwrap(), "note");
        assert_eq!(
            node.extra
                .get("saiPaperId")
                .and_then(serde_json::Value::as_i64),
            Some(1)
        );
        assert_eq!(
            node.extra.get("saiPinned").unwrap(),
            &serde_json::Value::Bool(true)
        );
        match &node.kind {
            CanvasNodeKind::File { file, .. } => assert_eq!(file, &note.to_string_lossy()),
            other => panic!("file の札ではありません: {other:?}"),
        }
        // note.md が無いときの論文の札との使い分け。
        let paper = add_paper(&mut canvas, 2, None, None);
        assert_eq!(paper, "paper-2");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn non_draft_canvas_needs_force() {
        let canvas = new_canvas("user-map", false);
        assert!(!is_draft(&canvas));
        let error = ensure_editable(&canvas, false).unwrap_err();
        assert!(
            matches!(error, CanvasEditError::NotDraft),
            "NotDraft ではありません: {error:?}"
        );
        assert!(ensure_editable(&canvas, true).is_ok());

        let draft = new_canvas("draft-map", true);
        assert!(is_draft(&draft));
        assert!(ensure_editable(&draft, false).is_ok());
    }

    #[test]
    fn saved_canvas_round_trips_through_parse_canvas() {
        let dir = temp_dir("round_trip");
        let mut canvas = new_canvas("map-test", true);
        let g1 = add_group(
            &mut canvas,
            "前処理",
            parse_color_name("yellow").unwrap(),
            Some("前処理"),
        );
        let _ = add_paper(&mut canvas, 7, None, Some(&g1));
        let _ = add_text(&mut canvas, "研究の量 高 / 手薄 低", Some(&g1));
        let from = add_text(&mut canvas, "A", Some(&g1));
        let to = add_text(&mut canvas, "B", Some(&g1));
        link(&mut canvas, &from, &to, Some("注")).unwrap();
        save(&dir, "map-test", &canvas).unwrap();

        let path = canvas_path_for_tag(&dir, "map-test");
        let source = std::fs::read_to_string(&path).unwrap();
        let (parsed, diagnostics) = parse_canvas(&source).unwrap();
        assert!(diagnostics.is_empty(), "診断が出ました: {diagnostics:?}");
        assert_eq!(parsed.nodes.len(), canvas.nodes.len());
        assert_eq!(parsed.edges.len(), 1);
        assert_eq!(
            parsed.extra.get("saiTag"),
            Some(&serde_json::Value::String("map-test".to_string()))
        );
        assert_eq!(
            parsed.extra.get("saiDraft"),
            Some(&serde_json::Value::Bool(true))
        );
        // load_or_new でも同じものが読める。
        let reloaded = load_or_new(&dir, "map-test").unwrap();
        assert_eq!(reloaded.nodes.len(), canvas.nodes.len());
        assert_eq!(reloaded.edges.len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn link_rejects_missing_nodes() {
        let mut canvas = new_canvas("map-test", true);
        let error = link(&mut canvas, "group-1", "group-2", None).unwrap_err();
        assert!(matches!(error, CanvasEditError::NodeNotFound(_)));
    }

    /// AC1: 理解の状態から色を決める（記録なし / あいまい / 理解している）。
    #[test]
    fn understanding_color_follows_state() {
        assert_eq!(understanding_color(None), None, "記録なしは色なし（灰）");
        assert_eq!(understanding_color(Some("unknown")), None);
        assert_eq!(
            understanding_color(Some("vague")),
            Some(CanvasColor::Preset(3)),
            "あいまいは黄"
        );
        assert_eq!(
            understanding_color(Some("ok")),
            Some(CanvasColor::Preset(4)),
            "理解しているは緑"
        );
        // 知らない値も色なしとして扱う。
        assert_eq!(understanding_color(Some("???")), None);
    }

    /// AC1: `add-group` の 3 通り（指定なし / `--color` / 理解の記録から）。
    #[test]
    fn add_group_color_and_fixed_flag() {
        // 記録なし + トピック: 色なし、`saiColorFixed` もなし（TUI が決め直す）。
        let mut canvas = new_canvas("map-test", true);
        let id = add_group_from_understanding(&mut canvas, "前処理", "前処理の違い", None);
        let group = canvas.nodes.iter().find(|node| node.id == id).unwrap();
        assert_eq!(group.color, None);
        assert!(!group.extra.contains_key("saiColorFixed"));

        // 理解している + トピック: 緑。`saiColorFixed` はなし。
        let id = add_group_from_understanding(&mut canvas, "モデル", "中間表現", Some("ok"));
        let group = canvas.nodes.iter().find(|node| node.id == id).unwrap();
        assert_eq!(group.color, Some(CanvasColor::Preset(4)));
        assert!(!group.extra.contains_key("saiColorFixed"));

        // `--color` 指定: その色、`saiColorFixed: true`。
        let id = add_group(
            &mut canvas,
            "評価",
            parse_color_name("cyan").unwrap(),
            Some("評価の分け方"),
        );
        let group = canvas.nodes.iter().find(|node| node.id == id).unwrap();
        assert_eq!(group.color, Some(CanvasColor::Preset(5)));
        assert_eq!(
            group.extra.get("saiColorFixed"),
            Some(&serde_json::Value::Bool(true))
        );
    }

    /// AC2: 開き直したときに理解の記録で色が変わる / `saiColorFixed` は変わらない。
    #[test]
    fn refresh_topic_colors_recomputes_without_fixed() {
        let mut canvas = new_canvas("map-test", true);
        // 理解の記録から決まる枠（トピック付き）。
        let dynamic = add_group_from_understanding(&mut canvas, "前処理", "前処理の違い", None);
        // ユーザーが色を決めた枠（トピック付き・固定）。
        let fixed = add_group(
            &mut canvas,
            "評価",
            parse_color_name("cyan").unwrap(),
            Some("評価の分け方"),
        );
        // トピックの無い枠は対象外。
        let _ = add_group(&mut canvas, "その他", None, None);

        // あいまいが記録された → 動的な枠だけ黄になる。
        let states = BTreeMap::from([
            ("前処理の違い".to_string(), "vague".to_string()),
            ("評価の分け方".to_string(), "vague".to_string()),
        ]);
        assert!(refresh_topic_colors(&mut canvas, &states), "色が変わる");
        let color_of = |canvas: &Canvas, id: &str| {
            canvas
                .nodes
                .iter()
                .find(|node| node.id == id)
                .unwrap()
                .color
                .clone()
        };
        assert_eq!(color_of(&canvas, &dynamic), Some(CanvasColor::Preset(3)));
        assert_eq!(
            color_of(&canvas, &fixed),
            Some(CanvasColor::Preset(5)),
            "saiColorFixed は決め直さない"
        );

        // 変わらなければ false（無駄に保存しない）。
        assert!(!refresh_topic_colors(&mut canvas, &states));

        // 理解しているに変わった → 動的な枠は緑。
        let states = BTreeMap::from([("前処理の違い".to_string(), "ok".to_string())]);
        assert!(refresh_topic_colors(&mut canvas, &states));
        assert_eq!(color_of(&canvas, &dynamic), Some(CanvasColor::Preset(4)));
        // 記録が消えた（未知のトピック）→ 色なし（灰）に戻る。
        let states = BTreeMap::new();
        assert!(refresh_topic_colors(&mut canvas, &states));
        assert_eq!(color_of(&canvas, &dynamic), None);
        assert_eq!(color_of(&canvas, &fixed), Some(CanvasColor::Preset(5)));
    }

    /// `states_by_topic` がトピックごとに 1 つ引けること。
    #[test]
    fn states_by_topic_maps_entries() {
        let entries = vec![
            UnderstandingEntry {
                id: 1,
                topic: "前処理の違い".to_string(),
                state: "vague".to_string(),
                evidence: String::new(),
                run_id: None,
                created_at: "2026-09-25 12:00:00".to_string(),
            },
            UnderstandingEntry {
                id: 2,
                topic: "評価の分け方".to_string(),
                state: "ok".to_string(),
                evidence: String::new(),
                run_id: None,
                created_at: "2026-09-25 12:01:00".to_string(),
            },
        ];
        let states = states_by_topic(&entries);
        assert_eq!(
            states.get("前処理の違い").map(String::as_str),
            Some("vague")
        );
        assert_eq!(states.get("評価の分け方").map(String::as_str), Some("ok"));
        assert_eq!(states.get("無いトピック"), None);
    }
}
