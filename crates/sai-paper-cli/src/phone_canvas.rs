//! `sai-paper phone canvas` — sai-phone（Web）の Canvas 画面用の JSON 出力（U03）。
//!
//! sai-phone は sai-paper の DB を直接開かず、このコマンドの JSON だけを読む
//! （`sai-phone/src/sai_phone/papers.py:3`、`phone.rs:1-10`）。既存の
//! `sai-paper canvas`（`crates/sai-paper-cli/src/canvas.rs`）はエージェント用で、
//! 下書きの保護（`--force`）があり出力が文字の 1 行なので、画面用の命令を別に作る。
//!
//! 共通ルール（`crates/sai-paper-cli/src/phone.rs` と同じ）:
//!
//! - 成功: 終了コード 0、標準出力に JSON 1 行
//! - 失敗: 終了コード 1、標準出力に `{"error": "<日本語の文>"}`
//! - Canvas 名やノード id は `-` で始まるとオプションと読まれるため、必ず `--` を
//!   置いて呼ぶ（例: `sai-paper phone canvas show -- 前処理`）
//!
//! 同期（DB とそろえる）・隠した位置（`saiHidden`）・線は
//! [`sai_paper_core::canvas_sync`] が担当する。ここでは JSON にすることだけする。

use std::collections::{HashMap, HashSet};
use std::io::Read as _;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use clap::Subcommand;
use rusqlite::Connection;
use sai_paper_core::canvas::{
    Canvas, CanvasNode, CanvasNodeKind, canvas_file_stem, canvas_path_for_tag,
};
use sai_paper_core::canvas_cards::{CanvasCardSource, PaperCardInfo, SidebarPaper};
use sai_paper_core::paths::paper_file_names;
use sai_paper_core::{Database, Paths, canvas_cards, canvas_edit, canvas_sync, reports};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::open_database;

/// `sai-paper phone canvas` のサブコマンド。
#[derive(Subcommand, Debug)]
pub enum PhoneCanvasCmd {
    /// Canvas の一覧（通常の Canvas と「地図（下書き）」に分けて出す）
    List,
    /// Canvas を新建する（タグも作る。ユーザーの Canvas なので下書きにはしない）
    New {
        /// Canvas 名（= タグ名）
        name: String,
    },
    /// Canvas の中身（ノード・線・隠した札・右の欄）を JSON で返す
    Show {
        /// Canvas 名（= タグ名）
        name: String,
    },
    /// 標準入力の JSON の座標でノードを動かす
    /// （`{"nodes":[{"id":"note-37","x":120,"y":40}]}`）
    Layout {
        /// Canvas 名（= タグ名）
        name: String,
    },
    /// 札を出す・隠す（Library のタグは変えない）
    Toggle {
        /// Canvas 名（= タグ名）
        name: String,
        /// 札のノード id（`note-<論文番号>` / `paper-<論文番号>` / `report-<report id>`）
        node_id: String,
        /// 出したときに、この札の横に置く
        #[arg(long)]
        near: Option<String>,
    },
    /// 選んだ札のつながりのうち、Canvas に札が無いもの
    Missing {
        /// Canvas 名（= タグ名）
        name: String,
        /// 選んだ札のノード id
        node_id: String,
    },
    /// 札の中身（右の欄に出す本文）
    Body {
        /// Canvas 名（= タグ名）
        name: String,
        /// 札のノード id
        node_id: String,
    },
}

/// `sai-paper phone canvas` を実行する（実際の処理は [`run_with`]）。
pub fn run(cmd: PhoneCanvasCmd) -> Result<Value> {
    let paths = Paths::discover().context("プロジェクトのパスを解決できません")?;
    let data_dir = paths.data_dir();
    let db = open_database()?;
    run_with(&db, &data_dir, cmd)
}

/// DB と data_dir を渡して処理する（テストから直接呼べるように分ける）。
pub(crate) fn run_with(db: &Database, data_dir: &Path, cmd: PhoneCanvasCmd) -> Result<Value> {
    match cmd {
        PhoneCanvasCmd::List => list(db, data_dir),
        PhoneCanvasCmd::New { name } => new_canvas(db, data_dir, &name),
        PhoneCanvasCmd::Show { name } => show(db, data_dir, &name),
        PhoneCanvasCmd::Layout { name } => layout(data_dir, &name),
        PhoneCanvasCmd::Toggle {
            name,
            node_id,
            near,
        } => toggle(db, data_dir, &name, &node_id, near.as_deref()),
        PhoneCanvasCmd::Missing { name, node_id } => missing(db, data_dir, &name, &node_id),
        PhoneCanvasCmd::Body { name, node_id } => body(db, data_dir, &name, &node_id),
    }
}

// ---------------------------------------------------------------- 共通の部品

/// Canvas 名（= タグ名）を確かめて `.canvas` のパスを返す。
fn canvas_path_checked(data_dir: &Path, name: &str) -> Result<PathBuf> {
    if canvas_file_stem(name).is_none() {
        bail!("タグ '{name}' は Canvas 名にできません");
    }
    Ok(canvas_path_for_tag(data_dir, name))
}

/// ノードの `extra.saiPaperId` を読む。
fn sai_paper_id(node: &CanvasNode) -> Option<i64> {
    node.extra.get("saiPaperId").and_then(Value::as_i64)
}

/// ノードの `extra.saiReportId` を読む。
fn sai_report_id(node: &CanvasNode) -> Option<String> {
    node.extra
        .get("saiReportId")
        .and_then(Value::as_str)
        .map(str::to_string)
}

/// ノードの `extra.saiKind` を読む。
fn sai_kind(node: &CanvasNode) -> Option<&str> {
    node.extra.get("saiKind").and_then(Value::as_str)
}

/// 論文のタイトルを引く（無ければ空文字）。
fn paper_title_or_empty(db: &Database, paper_id: i64) -> Result<String> {
    use rusqlite::OptionalExtension;
    let title: Option<Option<String>> = db
        .connection()
        .query_row(
            "SELECT title FROM papers WHERE id = ?1",
            [paper_id],
            |row| row.get(0),
        )
        .optional()?;
    Ok(title.flatten().unwrap_or_default())
}

/// 論文の札の情報（無ければ `None`）。
fn paper_card_info(db: &Database, paper_id: i64) -> Result<Option<PaperCardInfo>> {
    canvas_cards::paper_card_info(db.connection(), paper_id)
        .context("論文の情報の取得に失敗しました")
}

/// 論文フォルダの `note.md`（あるときだけ）。
fn note_path_if_exists(data_dir: &Path, paper_id: i64, title: &str) -> Option<PathBuf> {
    let path =
        sai_paper_core::paths::paper_folder(data_dir, paper_id, title).join(paper_file_names::NOTE);
    path.is_file().then_some(path)
}

/// 論文 P の札の id（note.md があれば `note-P`、無ければ `paper-P`）。
fn paper_node_id(data_dir: &Path, db: &Database, paper_id: i64) -> Result<String> {
    let title = paper_title_or_empty(db, paper_id)?;
    let prefix = if note_path_if_exists(data_dir, paper_id, &title).is_some() {
        "note"
    } else {
        "paper"
    };
    Ok(format!("{prefix}-{paper_id}"))
}

/// Canvas にある論文の札の id（`note-P` / `paper-P`）。report の札は数えない。
fn canvas_paper_ids(canvas: &Canvas) -> HashSet<i64> {
    canvas
        .nodes
        .iter()
        .filter(|node| matches!(sai_kind(node), Some("note" | "paper")))
        .filter_map(sai_paper_id)
        .collect()
}

/// Canvas にある report の札の id。
fn canvas_report_ids(canvas: &Canvas) -> HashSet<String> {
    canvas.nodes.iter().filter_map(sai_report_id).collect()
}

/// 札の欄を 1 回だけ集める表（札ごとに DB を引かないため）。
struct CardTables {
    /// 論文 id → note の札
    notes: HashMap<i64, CanvasCardSource>,
    /// report id → report の札
    reports: HashMap<String, CanvasCardSource>,
    /// report id → DB に記録されたタイトル
    report_titles: HashMap<String, Option<String>>,
    /// 論文 id → 論文の札に表示する情報
    papers: HashMap<i64, PaperCardInfo>,
}

impl CardTables {
    /// note / report / 論文の札の情報を 1 回ずつ読んで表にする。
    fn collect(conn: &Connection, data_dir: &Path) -> Result<Self> {
        let notes: HashMap<i64, CanvasCardSource> = canvas_cards::note_card_sources(conn, data_dir)
            .context("note の札の候補を読み込めませんでした")?
            .into_iter()
            .filter_map(|source| Some((source.papers_id?, source)))
            .collect();
        let reports: HashMap<String, CanvasCardSource> = canvas_cards::report_card_sources(conn)
            .context("report の札の候補を読み込めませんでした")?
            .into_iter()
            .filter_map(|source| Some((source.report_id.clone()?, source)))
            .collect();
        let mut report_titles = HashMap::new();
        {
            let mut stmt = conn.prepare("SELECT id, title FROM reports")?;
            let rows = stmt.query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
            })?;
            for row in rows {
                let (id, title) = row?;
                report_titles.insert(id, title);
            }
        }
        Ok(Self {
            notes,
            reports,
            report_titles,
            papers: HashMap::new(),
        })
    }

    /// 論文の札の情報（同じ論文は 2 回引かない）。
    fn paper(&mut self, conn: &Connection, paper_id: i64) -> Option<PaperCardInfo> {
        if let Some(info) = self.papers.get(&paper_id) {
            return Some(info.clone());
        }
        let info = canvas_cards::paper_card_info(conn, paper_id)
            .ok()
            .flatten()?;
        self.papers.insert(paper_id, info.clone());
        Some(info)
    }
}

/// ノードの `color` を JSON Canvas の色（文字列）にする。
fn color_json(node: &CanvasNode) -> Value {
    node.color
        .as_ref()
        .map(|color| Value::String(color.as_str()))
        .unwrap_or(Value::Null)
}

/// 札のノード（`type: "card"`）の `kind` を決める。
///
/// `saiKind` があればそれを優先し、無ければ DB に札があるか（note / paper /
/// report）で決める。DB に無い札（外の道具が作ったもの）は `None`。
fn card_kind(node: &CanvasNode, tables: &CardTables) -> Option<&'static str> {
    match sai_kind(node) {
        Some("note") => return Some("note"),
        Some("paper") => return Some("paper"),
        Some("report") => return Some("report"),
        _ => {}
    }
    if sai_report_id(node).is_some_and(|id| tables.reports.contains_key(&id)) {
        return Some("report");
    }
    match sai_paper_id(node) {
        Some(id) if tables.notes.contains_key(&id) => Some("note"),
        Some(id) if tables.papers.contains_key(&id) => Some("paper"),
        _ => None,
    }
}

/// 札の `title` / `subtitle` / `summary` / `tags` を作る（C-C の表）。
fn card_fields(
    kind: &str,
    node: &CanvasNode,
    db: &Database,
    tables: &mut CardTables,
) -> (String, String, String, String) {
    let conn = db.connection();
    let paper_id = sai_paper_id(node);
    // DB に無い札の見出しは `extra.saiTitle`（無ければ "(不明な札)"）。
    let fallback_title = node
        .extra
        .get("saiTitle")
        .and_then(Value::as_str)
        .unwrap_or("(不明な札)")
        .to_string();

    if kind == "report" {
        let report_id = sai_report_id(node).unwrap_or_default();
        let title = tables
            .report_titles
            .get(&report_id)
            .cloned()
            .flatten()
            .filter(|title| !title.is_empty())
            .unwrap_or_else(|| report_id.clone());
        let paper_title = paper_id
            .and_then(|id| tables.paper(conn, id))
            .map_or_else(String::new, |info| info.title);
        let subtitle = if paper_title.is_empty() {
            report_id.clone()
        } else {
            format!("{report_id} · {paper_title}")
        };
        let summary = tables
            .reports
            .get(&report_id)
            .map_or_else(String::new, |source| source.summary.clone());
        return (title, subtitle, summary, String::new());
    }

    let Some(paper_id) = paper_id else {
        return (fallback_title, String::new(), String::new(), String::new());
    };
    let info = tables.paper(conn, paper_id);
    let title = match info.as_ref().map(|info| info.title.as_str()) {
        Some("") | None => fallback_title,
        Some(title) => title.to_string(),
    };
    let year = info
        .as_ref()
        .map_or_else(String::new, |info| info.year.clone());
    let subtitle = if year.is_empty() {
        format!("#{paper_id}")
    } else {
        format!("#{paper_id}  {year}")
    };
    let tags = info.map_or_else(String::new, |info| info.tags);
    let summary = if kind == "note" {
        tables
            .notes
            .get(&paper_id)
            .map_or_else(String::new, |source| source.summary.clone())
    } else {
        String::new()
    };
    (title, subtitle, summary, tags)
}

/// 1 つのノードを JSON にする。
fn node_json(canvas: &Canvas, node: &CanvasNode, db: &Database, tables: &mut CardTables) -> Value {
    let node_type = match &node.kind {
        CanvasNodeKind::Group { .. } => "group",
        CanvasNodeKind::Text { .. } => "text",
        _ => "card",
    };
    let mut value = json!({
        "id": node.id,
        "type": node_type,
        "x": node.x,
        "y": node.y,
        "width": node.width,
        "height": node.height,
        "color": color_json(node),
        "group": crate::canvas::group_of(canvas, node),
    });
    match node_type {
        "group" => {
            let label = match &node.kind {
                CanvasNodeKind::Group { label, .. } => label.clone().unwrap_or_default(),
                _ => String::new(),
            };
            value["label"] = json!(label);
        }
        "text" => {
            let text = match &node.kind {
                CanvasNodeKind::Text { text } => text.clone(),
                _ => String::new(),
            };
            value["text"] = json!(text);
        }
        _ => {
            let kind = card_kind(node, tables);
            let (title, subtitle, summary, tags) = match kind {
                Some(kind) => card_fields(kind, node, db, tables),
                None => (
                    node.extra
                        .get("saiTitle")
                        .and_then(Value::as_str)
                        .unwrap_or("(不明な札)")
                        .to_string(),
                    String::new(),
                    String::new(),
                    String::new(),
                ),
            };
            value["kind"] = match kind {
                Some(kind) => json!(kind),
                None => Value::Null,
            };
            value["paper_id"] = json!(sai_paper_id(node));
            value["report_id"] = match sai_report_id(node) {
                Some(report_id) => json!(report_id),
                None => Value::Null,
            };
            value["title"] = json!(title);
            value["subtitle"] = json!(subtitle);
            value["tags"] = json!(tags);
            value["summary"] = json!(summary);
        }
    }
    value
}

/// 右の欄（sidebar）の 1 行。
fn sidebar_json(paper: &SidebarPaper) -> Value {
    let note = if paper.note_md.is_some() {
        json!({
            "node_id": format!("note-{}", paper.papers_id),
            "summary": paper.note_summary,
        })
    } else {
        Value::Null
    };
    let reports: Vec<Value> = paper
        .reports
        .iter()
        .map(|report| {
            json!({
                "report_id": report.report_id,
                "node_id": format!("report-{}", report.report_id),
                "title": report.title,
                "summary": report.summary,
            })
        })
        .collect();
    json!({
        "paper_id": paper.papers_id,
        "title": paper.title,
        "note": note,
        "reports": reports,
    })
}

// ---------------------------------------------------------------- list / new

/// `phone canvas list`: タグから Canvas の一覧を作る（TUI の `list_canvases` と同じ分け方）。
fn list(db: &Database, data_dir: &Path) -> Result<Value> {
    let mut canvases: Vec<Value> = Vec::new();
    let mut maps: Vec<Value> = Vec::new();
    for tag in db.tags().context("タグの取得に失敗しました")? {
        let name = tag.name;
        // ファイル名にできないタグは Canvas を作れないので除く。
        if canvas_file_stem(&name).is_none() {
            continue;
        }
        let path = canvas_path_for_tag(data_dir, &name);
        let (draft, nodes) = if path.is_file() {
            match canvas_edit::load_or_new(data_dir, &name) {
                Ok(canvas) => (canvas_edit::is_draft(&canvas), canvas.nodes.len()),
                // 読めなかった Canvas は通常の一覧に入れる（ノード数は 0）。
                Err(_) => (false, 0),
            }
        } else {
            (false, 0)
        };
        let entry = json!({ "name": name, "draft": draft, "nodes": nodes });
        // 「地図（下書き）」は名前が `map-` で始まり `saiDraft: true` があるもの。
        if name.starts_with("map-") && draft {
            maps.push(entry);
        } else {
            canvases.push(entry);
        }
    }
    let by_name = |a: &Value, b: &Value| {
        a["name"]
            .as_str()
            .unwrap_or_default()
            .cmp(b["name"].as_str().unwrap_or_default())
    };
    canvases.sort_by(by_name);
    maps.sort_by(by_name);
    Ok(json!({ "canvases": canvases, "maps": maps }))
}

/// `phone canvas new`: タグと空の Canvas を作る（ユーザーの Canvas なので下書きにはしない）。
fn new_canvas(db: &Database, data_dir: &Path, name: &str) -> Result<Value> {
    let path = canvas_path_checked(data_dir, name)?;
    if path.is_file() {
        bail!("Canvas '{name}' は既にあります");
    }
    db.ensure_tag(name).context("タグを作れませんでした")?;
    let canvas = canvas_edit::new_canvas(name, false);
    canvas_edit::save(data_dir, name, &canvas).context("Canvas を保存できませんでした")?;
    Ok(json!({ "name": name }))
}

// ---------------------------------------------------------------- show

/// `phone canvas show`: 開くたびに DB と同期してから JSON を返す。
fn show(db: &Database, data_dir: &Path, name: &str) -> Result<Value> {
    let path = canvas_path_checked(data_dir, name)?;
    // タグが DB に無い Canvas は開けない（TUI と同じ）。
    let tags: i64 = db
        .connection()
        .query_row(
            "SELECT COUNT(*) FROM tags WHERE name = ?1 COLLATE NOCASE",
            [name],
            |row| row.get(0),
        )
        .context("タグの確認に失敗しました")?;
    if tags == 0 {
        bail!("Canvas '{name}' がありません");
    }
    let conn = db.connection();
    let sources = canvas_cards::tag_card_sources(conn, data_dir, name)
        .context("札の候補を読み込めませんでした")?;
    let mut canvas =
        canvas_edit::load_or_new(data_dir, name).context("Canvas を読み込めませんでした")?;
    // ファイルが無いときは自動配置、あれば同期する（残りのノードは動かさない）。
    let mut changed = if path.is_file() {
        canvas_sync::sync_nodes(&mut canvas, &sources)
    } else {
        canvas.nodes = canvas_sync::auto_layout_nodes(&sources);
        true
    };
    // 理解の記録で枠の色を決め直す（TUI の `open_canvas` と同じ）。
    let states = canvas_edit::states_by_topic(
        &sai_paper_core::understanding::latest_per_topic(conn)
            .context("理解の記録の読み込みに失敗しました")?,
    );
    if canvas_edit::refresh_topic_colors(&mut canvas, &states) {
        changed = true;
    }
    if changed {
        // `canvas.edges` は消さない（エージェントが `canvas link` で引いた線を残す）。
        canvas_edit::save(data_dir, name, &canvas).context("Canvas を保存できませんでした")?;
    }

    let mut tables = CardTables::collect(conn, data_dir)?;
    let nodes: Vec<Value> = canvas
        .nodes
        .iter()
        .map(|node| node_json(&canvas, node, db, &mut tables))
        .collect();
    let edges: Vec<Value> = canvas_sync::canvas_lines(conn, data_dir, &canvas)
        .context("線を決められませんでした")?
        .into_iter()
        .map(|line| {
            json!({
                "from": line.from,
                "to": line.to,
                "kind": line.kind.as_str(),
                "label": line.label,
            })
        })
        .collect();
    let hidden: Vec<Value> = canvas_sync::hidden_ids(&canvas)
        .into_iter()
        .map(Value::String)
        .collect();
    let sidebar: Vec<Value> = canvas_cards::sidebar_papers(conn, data_dir)
        .context("右の欄の候補を読み込めませんでした")?
        .iter()
        .map(sidebar_json)
        .collect();
    Ok(json!({
        "name": name,
        "draft": canvas_edit::is_draft(&canvas),
        "nodes": nodes,
        "edges": edges,
        "hidden": hidden,
        "sidebar": sidebar,
    }))
}

// ---------------------------------------------------------------- layout

/// `layout` の入力（標準入力の JSON）。
#[derive(Debug, Deserialize)]
struct LayoutInput {
    /// 動かすノード
    nodes: Vec<LayoutNode>,
}

/// 動かすノード 1 つ。
#[derive(Debug, Deserialize)]
struct LayoutNode {
    /// ノード id
    id: String,
    /// 新しい x
    x: i64,
    /// 新しい y
    y: i64,
    /// 新しい幅（無ければ変えない）
    #[serde(default)]
    width: Option<i64>,
    /// 新しい高さ（無ければ変えない）
    #[serde(default)]
    height: Option<i64>,
}

/// `phone canvas layout`: 標準入力の JSON の座標でノードを動かす。
fn layout(data_dir: &Path, name: &str) -> Result<Value> {
    let mut input = String::new();
    std::io::stdin()
        .read_to_string(&mut input)
        .context("標準入力を読めませんでした")?;
    let parsed = parse_layout(&input)?;
    apply_layout(data_dir, name, &parsed)
}

/// 標準入力の JSON を [`LayoutInput`] にする（読めなければ日本語の文でエラー）。
fn parse_layout(input: &str) -> Result<LayoutInput> {
    serde_json::from_str(input)
        .map_err(|error| anyhow::anyhow!("入力の JSON を読めません: {error}"))
}

/// 読み込んだ JSON の座標でノードを書き換えて保存する（[`layout`] の本体）。
fn apply_layout(data_dir: &Path, name: &str, input: &LayoutInput) -> Result<Value> {
    let path = canvas_path_checked(data_dir, name)?;
    if !path.is_file() {
        bail!("Canvas '{name}' がありません");
    }
    let mut canvas =
        canvas_edit::load_or_new(data_dir, name).context("Canvas を読み込めませんでした")?;
    let mut updated = 0usize;
    let mut missing: Vec<Value> = Vec::new();
    for wanted in &input.nodes {
        // 大きさは 1 以上の整数。書ける前に確かめる（途中で失敗しても保存しない）。
        for size in [wanted.width, wanted.height] {
            if size.is_some_and(|size| size < 1) {
                bail!("大きさは 1 以上の整数にしてください");
            }
        }
        let Some(node) = canvas.nodes.iter_mut().find(|node| node.id == wanted.id) else {
            // Canvas に無い id は飛ばして `missing` に入れる。
            missing.push(Value::String(wanted.id.clone()));
            continue;
        };
        node.x = wanted.x;
        node.y = wanted.y;
        if let Some(width) = wanted.width {
            node.width = width;
        }
        if let Some(height) = wanted.height {
            node.height = height;
        }
        updated += 1;
    }
    // 書いてあるノード・線・`extra` 以外は触らない。
    canvas_edit::save(data_dir, name, &canvas).context("Canvas を保存できませんでした")?;
    Ok(json!({ "updated": updated, "missing": missing }))
}

// ---------------------------------------------------------------- toggle

/// 出し入れできる札の id（`note-<P>` / `paper-<P>` / `report-<R>`）を解く。
fn parse_toggle_id(node_id: &str) -> Result<ToggleTarget> {
    if let Some(rest) = node_id.strip_prefix("note-") {
        if let Ok(paper_id) = rest.parse::<i64>() {
            return Ok(ToggleTarget::Note(paper_id));
        }
    }
    if let Some(rest) = node_id.strip_prefix("paper-") {
        if let Ok(paper_id) = rest.parse::<i64>() {
            return Ok(ToggleTarget::Paper(paper_id));
        }
    }
    if let Some(rest) = node_id.strip_prefix("report-") {
        if !rest.is_empty() {
            return Ok(ToggleTarget::Report(rest.to_string()));
        }
    }
    bail!("出し入れできる札ではありません")
}

/// 出し入れの対象。
enum ToggleTarget {
    /// note の札（`note-<論文番号>`）
    Note(i64),
    /// note.md の無い論文の札（`paper-<論文番号>`）
    Paper(i64),
    /// report の札（`report-<report id>`）
    Report(String),
}

/// `phone canvas toggle`: 札を出す・隠す（Library のタグは変えない）。
fn toggle(
    db: &Database,
    data_dir: &Path,
    name: &str,
    node_id: &str,
    near: Option<&str>,
) -> Result<Value> {
    canvas_path_checked(data_dir, name)?;
    let target = parse_toggle_id(node_id)?;
    let mut canvas =
        canvas_edit::load_or_new(data_dir, name).context("Canvas を読み込めませんでした")?;
    // Canvas にあれば隠す（位置と大きさは `saiHidden` に残る）。
    if canvas.nodes.iter().any(|node| node.id == node_id) {
        if canvas_sync::hide_node(&mut canvas, node_id) {
            canvas_edit::save(data_dir, name, &canvas).context("Canvas を保存できませんでした")?;
        }
        return Ok(json!({ "node_id": node_id, "shown": false }));
    }
    // 無ければ足す。`add_*` は `saiPinned` を付けるので Library のタグは変わらない。
    let actual = match &target {
        ToggleTarget::Note(paper_id) => {
            let title = paper_title_or_empty(db, *paper_id)?;
            let note_md = note_path_if_exists(data_dir, *paper_id, &title)
                .ok_or_else(|| anyhow::anyhow!("論文 #{paper_id} に note.md がありません"))?;
            canvas_edit::add_note(&mut canvas, *paper_id, &note_md, None)
        }
        ToggleTarget::Paper(paper_id) => {
            if paper_card_info(db, *paper_id)?.is_none() {
                bail!("論文 #{paper_id} がライブラリにありません");
            }
            let title = paper_title_or_empty(db, *paper_id)?;
            let note_md = note_path_if_exists(data_dir, *paper_id, &title);
            canvas_edit::add_paper(&mut canvas, *paper_id, note_md.as_deref(), None)
        }
        ToggleTarget::Report(report_id) => {
            let report = reports::get_report(db.connection(), report_id)
                .context("report の取得に失敗しました")?
                .ok_or_else(|| anyhow::anyhow!("report '{report_id}' がありません"))?;
            let md_path = reports::resolve_md_path(&report);
            canvas_edit::add_report(&mut canvas, report_id, md_path.as_deref(), None)
        }
    };
    // 位置を決める: 1) 隠した位置 → 2) `--near` の札の横 → 3) 空いている所。
    // `--near` の札が無くてもエラーにしない（3 に進む）。
    let hidden_rect = canvas_sync::take_hidden_rect(&mut canvas, node_id)
        .or_else(|| canvas_sync::take_hidden_rect(&mut canvas, &actual));
    let near_rect = near
        .and_then(|near| canvas.nodes.iter().find(|node| node.id == near))
        .map(|node| (node.x, node.y, node.width, node.height));
    let (x, y) = match (hidden_rect, near_rect) {
        (Some((x, y, width, height)), _) => {
            if let Some(node) = canvas.nodes.iter_mut().find(|node| node.id == actual) {
                node.width = width;
                node.height = height;
            }
            (x, y)
        }
        (None, Some(anchor)) => canvas_sync::place_near(&canvas.nodes, &actual, anchor),
        (None, None) => canvas_sync::next_free_position(&canvas.nodes),
    };
    if let Some(node) = canvas.nodes.iter_mut().find(|node| node.id == actual) {
        node.x = x;
        node.y = y;
    }
    let (width, height) = canvas
        .nodes
        .iter()
        .find(|node| node.id == actual)
        .map_or((canvas_edit::CARD_W, canvas_edit::CARD_H), |node| {
            (node.width, node.height)
        });
    canvas_edit::save(data_dir, name, &canvas).context("Canvas を保存できませんでした")?;
    Ok(json!({
        "node_id": actual,
        "shown": true,
        "x": x,
        "y": y,
        "width": width,
        "height": height,
    }))
}

// ---------------------------------------------------------------- missing

/// `phone canvas missing`: 選んだ札のつながりのうち Canvas に札が無いもの。
fn missing(db: &Database, data_dir: &Path, name: &str, node_id: &str) -> Result<Value> {
    canvas_path_checked(data_dir, name)?;
    let conn = db.connection();
    let canvas =
        canvas_edit::load_or_new(data_dir, name).context("Canvas を読み込めませんでした")?;
    let empty = || json!({ "node_id": node_id, "items": [] });
    // 枠・文字・存在しない id には何もしない。
    let Some(selected) = canvas.nodes.iter().find(|node| node.id == node_id) else {
        return Ok(empty());
    };
    if !matches!(selected.kind, CanvasNodeKind::File { .. }) {
        return Ok(empty());
    }
    // Canvas にある札（選んだ札自身は除く）。
    let mut on_canvas_papers = canvas_paper_ids(&canvas);
    on_canvas_papers.retain(|paper_id| Some(*paper_id) != sai_paper_id(selected));
    let selected_report = sai_report_id(selected);
    let mut on_canvas_reports = canvas_report_ids(&canvas);
    if let Some(report_id) = &selected_report {
        on_canvas_reports.remove(report_id);
    }

    // report の札か論文の札かでつながりの出し方が変わる。
    let (paper_relation, report_relation, connections) = match selected_report {
        Some(report_id) => {
            let connections = canvas_cards::connections_of_report(conn, &report_id)
                .context("つながりを読み込めませんでした")?;
            ("使った論文", "関係する report", connections)
        }
        None => {
            let Some(paper_id) = sai_paper_id(selected) else {
                return Ok(empty());
            };
            let connections = canvas_cards::connections_of_paper(conn, data_dir, paper_id)
                .context("つながりを読み込めませんでした")?;
            ("引用", "使った report", connections)
        }
    };

    let mut items: Vec<Value> = Vec::new();
    // 論文（paper_id の昇順）。引用と note の言及は 1 つだけ出て、両方にあれば
    // 「note が言及」を優先する（TUI の `missing_connections` と同じ）。
    let mut paper_ids: Vec<i64> = connections
        .papers
        .iter()
        .chain(connections.notes_mentioning.iter())
        .copied()
        .collect();
    paper_ids.sort_unstable();
    paper_ids.dedup();
    for paper_id in paper_ids {
        if on_canvas_papers.contains(&paper_id) {
            continue;
        }
        let title = paper_card_info(db, paper_id)?.map_or_else(
            || format!("#{paper_id}"),
            |info| {
                if info.title.is_empty() {
                    format!("#{paper_id}")
                } else {
                    info.title
                }
            },
        );
        let relation = if connections.notes_mentioning.contains(&paper_id) {
            "note が言及"
        } else {
            paper_relation
        };
        items.push(json!({
            "relation": relation,
            "kind": "paper",
            "node_id": paper_node_id(data_dir, db, paper_id)?,
            "paper_id": paper_id,
            "report_id": Value::Null,
            "title": title,
        }));
    }
    // report（report_id の昇順）。
    for report_id in &connections.reports {
        if on_canvas_reports.contains(report_id) {
            continue;
        }
        let report = reports::get_report(conn, report_id).context("report の取得に失敗しました")?;
        let title = report
            .as_ref()
            .and_then(|report| report.title.clone())
            .filter(|title| !title.is_empty())
            .unwrap_or_else(|| report_id.clone());
        items.push(json!({
            "relation": report_relation,
            "kind": "report",
            "node_id": format!("report-{report_id}"),
            "paper_id": Value::Null,
            "report_id": report_id,
            "title": title,
        }));
    }
    Ok(json!({ "node_id": node_id, "items": items }))
}

// ---------------------------------------------------------------- body

/// `phone canvas body`: 札の中身（右の欄に出す本文）。
fn body(db: &Database, data_dir: &Path, name: &str, node_id: &str) -> Result<Value> {
    canvas_path_checked(data_dir, name)?;
    let canvas =
        canvas_edit::load_or_new(data_dir, name).context("Canvas を読み込めませんでした")?;
    let Some(node) = canvas.nodes.iter().find(|node| node.id == node_id) else {
        bail!("ノード '{node_id}' が Canvas にありません");
    };
    let paper_id = sai_paper_id(node);
    let report_id = sai_report_id(node);
    let saved_title = node
        .extra
        .get("saiTitle")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let (kind, title, markdown) = match &node.kind {
        CanvasNodeKind::Text { text } => (
            "text",
            text.lines().next().unwrap_or_default().to_string(),
            Some(text.clone()),
        ),
        CanvasNodeKind::Group { label, .. } => (
            "group",
            label.clone().unwrap_or_else(|| saved_title.clone()),
            None,
        ),
        CanvasNodeKind::File { .. } => match report_id.as_deref() {
            Some(report_id) => {
                let report = reports::get_report(db.connection(), report_id)
                    .context("report の取得に失敗しました")?;
                let title = report
                    .as_ref()
                    .and_then(|report| report.title.clone())
                    .filter(|title| !title.is_empty())
                    .unwrap_or_else(|| report_id.to_string());
                // md が読めなければ null。
                let markdown = report
                    .as_ref()
                    .and_then(reports::resolve_md_path)
                    .and_then(|path| std::fs::read_to_string(path).ok());
                ("report", title, markdown)
            }
            None => {
                let paper_title = match paper_id {
                    Some(paper_id) => paper_title_or_empty(db, paper_id)?,
                    None => String::new(),
                };
                match paper_id
                    .and_then(|paper_id| note_path_if_exists(data_dir, paper_id, &paper_title))
                {
                    Some(path) => {
                        let markdown = std::fs::read_to_string(path).ok();
                        ("note", paper_title, markdown)
                    }
                    None => ("paper", paper_title, None),
                }
            }
        },
        _ => ("file", saved_title, None),
    };
    Ok(json!({
        "node_id": node_id,
        "kind": kind,
        "title": title,
        "paper_id": paper_id,
        "report_id": report_id,
        "markdown": markdown,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser as _;
    use sai_paper_core::reports::{NewReport, ReportSource, create_report};

    /// テストごとに使い捨てのフォルダを作る。
    fn temp_dir(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "sai_cli_phone_canvas_{label}_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// 論文 1 件を DB に足す（`s2_paper_id` は引用の照合に使う）。
    fn add_paper(db: &Database, id: i64, title: &str, year: &str) {
        db.connection()
            .execute(
                "INSERT INTO papers (id, title, pdf_path, published_at, s2_paper_id, abstract)
                 VALUES (?1, ?2, ?3, ?4, ?5, '')",
                rusqlite::params![
                    id,
                    title,
                    format!("/tmp/{id}.pdf"),
                    format!("{year}-05-01"),
                    format!("s2-{id}")
                ],
            )
            .unwrap();
    }

    /// 論文の `note.md` を書く。
    fn write_note(data_dir: &Path, id: i64, title: &str, body: &str) {
        let folder = sai_paper_core::paths::paper_folder(data_dir, id, title);
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join(paper_file_names::NOTE), body).unwrap();
    }

    /// テスト用の環境（作業フォルダ・data_dir・DB）。
    fn setup(label: &str) -> (PathBuf, PathBuf, Database) {
        let dir = temp_dir(label);
        let data_dir = dir.join(".sai/paper");
        std::fs::create_dir_all(&data_dir).unwrap();
        let db = Database::open(&data_dir.join("papr.db")).unwrap();
        (dir, data_dir, db)
    }

    /// tag 名 `test` の Canvas を作る（論文 1・2 の note の札が入る）。
    fn setup_canvas_with_notes(label: &str) -> (PathBuf, PathBuf, Database) {
        let (dir, data_dir, db) = setup(label);
        add_paper(&db, 1, "Paper A", "2017");
        add_paper(&db, 2, "Paper B", "2019");
        add_paper(&db, 3, "Paper C", "2020");
        write_note(
            &data_dir,
            1,
            "Paper A",
            "# Note A\n\nA のメモ。#3 も見る。\n",
        );
        write_note(&data_dir, 2, "Paper B", "# Note B\n\nB のメモ。\n");
        db.ensure_tag("test").unwrap();
        db.add_paper_tag(1, "test").unwrap();
        db.add_paper_tag(2, "test").unwrap();
        (dir, data_dir, db)
    }

    /// report を作る（使う論文を 1 つ指定する）。
    fn add_report(db: &Database, dir: &Path, name: &str, paper_id: i64) -> String {
        let md = dir.join(name);
        std::fs::write(&md, format!("# {name}\n\nREPORT_BODY\n")).unwrap();
        create_report(
            db.connection(),
            &NewReport {
                md_path: md.to_string_lossy().to_string(),
                sources: vec![ReportSource {
                    kind: "paper".to_string(),
                    ref_id: paper_id.to_string(),
                    role: None,
                }],
                ..Default::default()
            },
        )
        .unwrap()
    }

    /// 命令を 1 個呼ぶ（テストは JSON だけを見る）。
    fn call(db: &Database, data_dir: &Path, cmd: PhoneCanvasCmd) -> Result<Value> {
        run_with(db, data_dir, cmd)
    }

    /// `show` を呼ぶ（よく使うのでまとめる）。
    fn show(db: &Database, data_dir: &Path, name: &str) -> Value {
        call(
            db,
            data_dir,
            PhoneCanvasCmd::Show {
                name: name.to_string(),
            },
        )
        .unwrap()
    }

    /// `toggle` を呼ぶ（よく使うのでまとめる）。
    fn toggle(
        db: &Database,
        data_dir: &Path,
        name: &str,
        node_id: &str,
        near: Option<&str>,
    ) -> Result<Value> {
        call(
            db,
            data_dir,
            PhoneCanvasCmd::Toggle {
                name: name.to_string(),
                node_id: node_id.to_string(),
                near: near.map(str::to_string),
            },
        )
    }

    /// 論文 P の `paper_tags` の行数を数える（タグの行数）。
    fn tag_rows(db: &Database, tag: &str) -> i64 {
        db.connection()
            .query_row(
                "SELECT COUNT(*) FROM paper_tags pt JOIN tags t ON t.id = pt.tag_id
                 WHERE t.name = ?1",
                [tag],
                |row| row.get(0),
            )
            .unwrap()
    }

    /// `show` の `nodes` の中から id で 1 つ探す。
    fn node_of<'a>(value: &'a Value, id: &str) -> &'a Value {
        value["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|node| node["id"] == id)
            .unwrap_or_else(|| panic!("{id} が nodes にありません: {value}"))
    }

    #[test]
    fn show_returns_nodes_edges_hidden_and_sidebar() {
        let (dir, data_dir, db) = setup_canvas_with_notes("show");
        let value = show(&db, &data_dir, "test");

        for key in ["nodes", "edges", "hidden", "sidebar"] {
            assert!(value.get(key).is_some(), "show に {key} がありません");
        }
        // 論文 1・2 の札（note.md がある）は note の札。
        let note = node_of(&value, "note-1");
        assert_eq!(note["type"], "card");
        assert_eq!(note["kind"], "note");
        assert_eq!(note["title"], "Paper A");
        assert_eq!(note["subtitle"], "#1  2017");
        assert_eq!(note["paper_id"], 1);
        assert_eq!(note["report_id"], Value::Null);
        assert_eq!(note["tags"], "test");
        assert!(note["summary"].as_str().unwrap().contains("A のメモ"));
        // 枠で囲まれるまでは group は null。
        assert_eq!(note["group"], Value::Null);
        // note.md の無い論文は paper の札。
        assert_eq!(value["nodes"].as_array().unwrap().len(), 2);

        // 枠・文字・隠した札。
        let mut canvas = canvas_edit::load_or_new(&data_dir, "test").unwrap();
        let group_id = canvas_edit::add_group(&mut canvas, "前処理", None, None);
        let text_id = canvas_edit::add_text(&mut canvas, "研究の量 高 / 手薄さ 低", None);
        // 枠を note-1 の上に置く（手で動かした状態と同じ）。
        if let Some(node) = canvas.nodes.iter_mut().find(|node| node.id == group_id) {
            (node.x, node.y, node.width, node.height) = (-40, -80, 680, 400);
        }
        assert!(canvas_sync::hide_node(&mut canvas, "note-2"));
        canvas_edit::save(&data_dir, "test", &canvas).unwrap();
        let value = show(&db, &data_dir, "test");
        let nodes = value["nodes"].as_array().unwrap();
        let group = nodes.iter().find(|node| node["type"] == "group").unwrap();
        assert_eq!(group["id"], group_id);
        assert_eq!(group["label"], "前処理");
        assert_eq!(group["group"], Value::Null);
        let text = nodes
            .iter()
            .find(|node| node["id"] == text_id)
            .expect("文字の札がありません");
        assert_eq!(text["text"], "研究の量 高 / 手薄さ 低");
        // 隠した札は hidden に出て、nodes には出ない。
        assert_eq!(value["hidden"], json!(["note-2"]));
        assert!(!nodes.iter().any(|node| node["id"] == "note-2"));
        // 枠の中に完全に入っている札だけが group を持つ。
        let in_group: Vec<&str> = nodes
            .iter()
            .filter(|node| node["group"] == "group-1")
            .map(|node| node["id"].as_str().unwrap())
            .collect();
        // 枠の中に完全に入っている札（note-2 は隠した）だけ。
        assert_eq!(in_group, vec!["note-1"]);

        // 右の欄（note か report を持つ論文だけ）。
        let sidebar = value["sidebar"].as_array().unwrap();
        assert!(sidebar.iter().any(|paper| paper["paper_id"] == 1));
        let paper = sidebar.iter().find(|paper| paper["paper_id"] == 1).unwrap();
        assert_eq!(paper["note"]["node_id"], "note-1");
        assert!(!sidebar.iter().any(|paper| paper["paper_id"] == 3));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn show_keeps_saved_lines_and_reports_missing_tag() {
        let (dir, data_dir, db) = setup_canvas_with_notes("show_edges");
        // 論文 3 に note.md を足して tag を付ける（note の言及の線を出すため）。
        write_note(&data_dir, 3, "Paper C", "# Note C\n");
        db.add_paper_tag(3, "test").unwrap();
        // 開く（note の札ができる）。
        show(&db, &data_dir, "test");
        // 手で線を 1 本引く（エージェントの `sai-paper canvas link` と同じ）。
        let mut canvas = canvas_edit::load_or_new(&data_dir, "test").unwrap();
        canvas_edit::link(&mut canvas, "note-1", "note-2", Some("注")).unwrap();
        canvas_edit::save(&data_dir, "test", &canvas).unwrap();

        let value = show(&db, &data_dir, "test");
        let edges = value["edges"].as_array().unwrap();
        // 保存された線は消えない。
        let link = edges
            .iter()
            .find(|edge| edge["kind"] == "link")
            .expect("保存された線が消えています");
        assert_eq!(link["from"], "note-1");
        assert_eq!(link["to"], "note-2");
        assert_eq!(link["label"], "注");
        // note の言及の線（`canvas_lines` の全部の種類）。
        assert!(
            edges.iter().any(|edge| edge["from"] == "note-1"
                && edge["to"] == "note-3"
                && edge["kind"] == "mentions"),
            "note の言及の線がありません: {edges:?}"
        );

        // DB に無いタグはエラー。
        let error = call(
            &db,
            &data_dir,
            PhoneCanvasCmd::Show {
                name: "no-such".to_string(),
            },
        )
        .unwrap_err();
        assert_eq!(format!("{error}"), "Canvas 'no-such' がありません");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn list_splits_canvases_and_maps() {
        let (dir, data_dir, db) = setup("list");
        // ファイルの無いタグは nodes 0 の通常の Canvas。
        db.ensure_tag("前処理").unwrap();
        // 下書きの「地図」。
        db.ensure_tag("map-preproc-20260926").unwrap();
        let map = canvas_edit::new_canvas("map-preproc-20260926", true);
        canvas_edit::save(&data_dir, "map-preproc-20260926", &map).unwrap();
        // `map-` でも下書きでなければ通常の Canvas。
        db.ensure_tag("map-fixed").unwrap();
        let fixed = canvas_edit::new_canvas("map-fixed", false);
        canvas_edit::save(&data_dir, "map-fixed", &fixed).unwrap();

        let value = call(&db, &data_dir, PhoneCanvasCmd::List).unwrap();
        // DB が最初に作るタグ（`reading-queue` など）はこの作業と関係ないため、
        // 自分の作ったタグだけを見る。
        let picked = |key: &str| -> Vec<Value> {
            value[key]
                .as_array()
                .unwrap()
                .iter()
                .filter(|entry| entry["name"] != "reading-queue")
                .cloned()
                .collect()
        };
        assert_eq!(
            picked("maps"),
            vec![json!({ "name": "map-preproc-20260926", "draft": true, "nodes": 0 })]
        );
        // 名前の昇順（`map-fixed` → `前処理`）。
        assert_eq!(
            picked("canvases"),
            vec![
                json!({ "name": "map-fixed", "draft": false, "nodes": 0 }),
                json!({ "name": "前処理", "draft": false, "nodes": 0 }),
            ]
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn new_creates_tag_and_refuses_duplicates() {
        let (dir, data_dir, db) = setup("new");
        let value = call(
            &db,
            &data_dir,
            PhoneCanvasCmd::New {
                name: "u03-test".to_string(),
            },
        )
        .unwrap();
        assert_eq!(value, json!({ "name": "u03-test" }));
        assert!(canvas_path_for_tag(&data_dir, "u03-test").is_file());
        // タグも作る（ユーザーの Canvas なので下書きにはしない）。
        let tags: i64 = db
            .connection()
            .query_row(
                "SELECT COUNT(*) FROM tags WHERE name = 'u03-test'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(tags, 1);
        assert!(!canvas_edit::is_draft(
            &canvas_edit::load_or_new(&data_dir, "u03-test").unwrap()
        ));
        // 2 回目は既にある。
        let error = call(
            &db,
            &data_dir,
            PhoneCanvasCmd::New {
                name: "u03-test".to_string(),
            },
        )
        .unwrap_err();
        assert_eq!(format!("{error}"), "Canvas 'u03-test' は既にあります");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn toggle_hides_then_restores_the_same_position_and_keeps_tags() {
        let (dir, data_dir, db) = setup_canvas_with_notes("toggle");
        let before_rows = tag_rows(&db, "test");
        let start = node_of(&show(&db, &data_dir, "test"), "note-1").clone();
        let x = start["x"].clone();
        let y = start["y"].clone();

        // 隠す。
        let hidden = toggle(&db, &data_dir, "test", "note-1", None).unwrap();
        assert_eq!(hidden, json!({ "node_id": "note-1", "shown": false }));
        assert_eq!(
            tag_rows(&db, "test"),
            before_rows,
            "隠すだけでタグが変わっています"
        );

        // もう一度出す。同じ位置に戻る（`saiHidden` に覚えていた位置）。
        let shown = toggle(&db, &data_dir, "test", "note-1", None).unwrap();
        assert_eq!(shown["node_id"], "note-1");
        assert_eq!(shown["shown"], true);
        assert_eq!(shown["x"], x);
        assert_eq!(shown["y"], y);
        assert_eq!(shown["width"], start["width"]);
        assert_eq!(shown["height"], start["height"]);
        assert_eq!(
            tag_rows(&db, "test"),
            before_rows,
            "出すだけでタグが変わっています"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn toggle_rejects_other_ids_and_places_near() {
        let (dir, data_dir, db) = setup_canvas_with_notes("toggle_near");
        // 開く（論文 1・2 の note の札ができる）。
        show(&db, &data_dir, "test");
        // 出し入れできない id。
        let error = toggle(&db, &data_dir, "test", "group-1", None).unwrap_err();
        assert_eq!(format!("{error}"), "出し入れできる札ではありません");
        // 論文が無い。
        let error = toggle(&db, &data_dir, "test", "paper-404", None).unwrap_err();
        assert_eq!(format!("{error}"), "論文 #404 がライブラリにありません");
        // note.md が無い論文に note の札は出せない。
        let error = toggle(&db, &data_dir, "test", "note-3", None).unwrap_err();
        assert_eq!(format!("{error}"), "論文 #3 に note.md がありません");
        // report が無い。
        let error = toggle(&db, &data_dir, "test", "report-R-9999", None).unwrap_err();
        assert_eq!(format!("{error}"), "report 'R-9999' がありません");

        // note.md の無い論文は paper の札になり、`--near` の札の横に置かれる。
        let value = toggle(&db, &data_dir, "test", "paper-3", Some("note-1")).unwrap();
        assert_eq!(value["node_id"], "paper-3");
        assert_eq!(value["shown"], true);
        let canvas = canvas_edit::load_or_new(&data_dir, "test").unwrap();
        let node = canvas.nodes.iter().find(|n| n.id == "paper-3").unwrap();
        let anchor = canvas.nodes.iter().find(|n| n.id == "note-1").unwrap();
        assert_eq!(node.x, anchor.x + anchor.width + canvas_sync::AUTO_GAP_X);
        assert_eq!(value["x"], node.x);
        // Library のタグは変わらない（`test` の行数は 2 のまま。論文 3 は tag が無い）。
        assert_eq!(tag_rows(&db, "test"), 2);

        let show = show(&db, &data_dir, "test");
        let node = node_of(&show, "paper-3");
        assert_eq!(node["kind"], "paper");
        assert_eq!(node["title"], "Paper C");
        assert_eq!(node["subtitle"], "#3  2020");
        assert_eq!(node["summary"], "");
        // タグが無くても同期で消えない（`saiPinned`）。
        assert!(
            show["nodes"]
                .as_array()
                .unwrap()
                .iter()
                .any(|node| node["id"] == "paper-3")
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn layout_moves_only_written_nodes_and_reports_missing() {
        let (dir, data_dir, db) = setup_canvas_with_notes("layout");
        show(&db, &data_dir, "test");
        let mut canvas = canvas_edit::load_or_new(&data_dir, "test").unwrap();
        canvas_edit::link(&mut canvas, "note-1", "note-2", None).unwrap();
        canvas_edit::save(&data_dir, "test", &canvas).unwrap();
        let before = show(&db, &data_dir, "test");
        let before_edges = before["edges"].as_array().unwrap().len();
        let before_note2 = node_of(&before, "note-2").clone();

        // 書いてあるノードだけ動き、無い id は missing に入る。
        let input: LayoutInput = serde_json::from_str(
            r#"{"nodes":[{"id":"note-1","x":120,"y":40,"width":700,"height":300},
                          {"id":"note-404","x":0,"y":0}]}"#,
        )
        .unwrap();
        let value = apply_layout(&data_dir, "test", &input).unwrap();
        assert_eq!(value, json!({ "updated": 1, "missing": ["note-404"] }));

        let after = show(&db, &data_dir, "test");
        let moved = node_of(&after, "note-1");
        assert_eq!(moved["x"], 120);
        assert_eq!(moved["y"], 40);
        assert_eq!(moved["width"], 700);
        assert_eq!(moved["height"], 300);
        // 書いていないノードは動かない。線も消えない。
        assert_eq!(node_of(&after, "note-2")["x"], before_note2["x"]);
        assert_eq!(node_of(&after, "note-2")["y"], before_note2["y"]);
        assert_eq!(after["edges"].as_array().unwrap().len(), before_edges);

        // 大きさが 1 未満ならエラー（保存しない）。
        let bad: LayoutInput =
            serde_json::from_str(r#"{"nodes":[{"id":"note-1","x":0,"y":0,"height":0}]}"#).unwrap();
        let error = apply_layout(&data_dir, "test", &bad).unwrap_err();
        assert_eq!(format!("{error}"), "大きさは 1 以上の整数にしてください");
        assert_eq!(node_of(&show(&db, &data_dir, "test"), "note-1")["x"], 120);

        // 壊れた JSON と、ファイルが無い Canvas。
        let error = parse_layout("{").unwrap_err();
        assert!(
            format!("{error}").starts_with("入力の JSON を読めません: "),
            "{error}"
        );
        let error = parse_layout(r#"{"nodes":{}}"#).unwrap_err();
        assert!(format!("{error}").starts_with("入力の JSON を読めません: "));
        let error = apply_layout(&data_dir, "no-such", &input).unwrap_err();
        assert_eq!(format!("{error}"), "Canvas 'no-such' がありません");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_drops_connections_already_on_the_canvas() {
        let (dir, data_dir, db) = setup("missing");
        add_paper(&db, 1, "Paper A", "2017");
        add_paper(&db, 2, "Paper B", "2019");
        add_paper(&db, 3, "Paper C", "2020");
        write_note(&data_dir, 1, "Paper A", "# Note A\n\nA のメモ。\n");
        write_note(&data_dir, 2, "Paper B", "# Note B\n");
        // 論文 3 の note が #1 を書く（3 は tag が無く Canvas に札が無い）。
        write_note(&data_dir, 3, "Paper C", "# Note C\n\nsee #1\n");
        // 引用: 論文 1 → 論文 2（2 は Canvas に札を置く）。
        db.connection()
            .execute(
                "INSERT INTO paper_refs (paper_id, ref_no, s2_paper_id, papers_id, raw_text)
                 VALUES ('1', 1, 's2-2', 2, '')",
                [],
            )
            .unwrap();
        db.ensure_tag("test").unwrap();
        db.add_paper_tag(1, "test").unwrap();
        // 論文 1・2 の札を Canvas に置く。
        let mut canvas = canvas_edit::new_canvas("test", false);
        let note2 = note_path_if_exists(&data_dir, 2, "Paper B").unwrap();
        assert_eq!(
            canvas_edit::add_note(&mut canvas, 2, &note2, None),
            "note-2"
        );
        let note1 = note_path_if_exists(&data_dir, 1, "Paper A").unwrap();
        assert_eq!(
            canvas_edit::add_note(&mut canvas, 1, &note1, None),
            "note-1"
        );
        canvas_edit::save(&data_dir, "test", &canvas).unwrap();

        let value = call(
            &db,
            &data_dir,
            PhoneCanvasCmd::Missing {
                name: "test".to_string(),
                node_id: "note-1".to_string(),
            },
        )
        .unwrap();
        // Canvas に札のある論文 1・2 は除かれ、3（note が言及）だけが残る。
        let items = value["items"].as_array().unwrap();
        assert_eq!(items.len(), 1, "{items:?}");
        assert_eq!(items[0]["node_id"], "note-3");
        assert_eq!(items[0]["relation"], "note が言及");
        assert_eq!(items[0]["kind"], "paper");
        assert_eq!(items[0]["paper_id"], 3);
        assert_eq!(items[0]["report_id"], Value::Null);
        assert_eq!(items[0]["title"], "Paper C");

        // 枠・文字・無い id は空。
        for id in ["group-1", "text-1", "note-404"] {
            let value = call(
                &db,
                &data_dir,
                PhoneCanvasCmd::Missing {
                    name: "test".to_string(),
                    node_id: id.to_string(),
                },
            )
            .unwrap();
            assert_eq!(value, json!({ "node_id": id, "items": [] }));
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_of_a_report_lists_papers_then_reports() {
        let (dir, data_dir, db) = setup("missing_report");
        add_paper(&db, 1, "Paper A", "2017");
        add_paper(&db, 2, "Paper B", "2019");
        write_note(&data_dir, 2, "Paper B", "# Note B\n");
        // R-1 は論文 1 を使う。R-2 は R-1 を元にして論文 2 を使う。
        let first = add_report(&db, &dir, "first.md", 1);
        let md = dir.join("second.md");
        std::fs::write(&md, "# second\n\nbody\n").unwrap();
        let second = create_report(
            db.connection(),
            &NewReport {
                md_path: md.to_string_lossy().to_string(),
                sources: vec![
                    ReportSource {
                        kind: "report".to_string(),
                        ref_id: first.clone(),
                        role: None,
                    },
                    ReportSource {
                        kind: "paper".to_string(),
                        ref_id: "2".to_string(),
                        role: None,
                    },
                ],
                ..Default::default()
            },
        )
        .unwrap();
        // Canvas には R-2 の札だけ（R-1 は無いので missing に出る）。
        let mut canvas = canvas_edit::new_canvas("test", false);
        assert_eq!(
            canvas_edit::add_report(&mut canvas, &second, None, None),
            format!("report-{second}")
        );
        canvas_edit::save(&data_dir, "test", &canvas).unwrap();

        let value = call(
            &db,
            &data_dir,
            PhoneCanvasCmd::Missing {
                name: "test".to_string(),
                node_id: format!("report-{second}"),
            },
        )
        .unwrap();
        let items = value["items"].as_array().unwrap();
        let found: Vec<(&str, &str)> = items
            .iter()
            .map(|item| {
                (
                    item["relation"].as_str().unwrap(),
                    item["node_id"].as_str().unwrap(),
                )
            })
            .collect();
        // 論文（paper_id 昇順）→ report（report_id 昇順）。
        assert_eq!(
            found,
            vec![
                ("使った論文", "note-2"),
                ("関係する report", format!("report-{first}").as_str()),
            ],
            "report のつながりが違います: {found:?}"
        );
        assert_eq!(items[0]["paper_id"], 2);
        assert_eq!(items[0]["report_id"], Value::Null);
        assert_eq!(items[1]["report_id"], first);
        assert_eq!(items[1]["paper_id"], Value::Null);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn body_returns_markdown_per_kind() {
        let (dir, data_dir, db) = setup_canvas_with_notes("body");
        let report_id = add_report(&db, &dir, "body-report.md", 1);
        show(&db, &data_dir, "test");
        let mut canvas = canvas_edit::load_or_new(&data_dir, "test").unwrap();
        let group_id = canvas_edit::add_group(&mut canvas, "前処理", None, None);
        let text_id = canvas_edit::add_text(&mut canvas, "研究の量 高", None);
        assert_eq!(
            canvas_edit::add_report(&mut canvas, &report_id, None, None),
            format!("report-{report_id}")
        );
        canvas_edit::save(&data_dir, "test", &canvas).unwrap();

        let body_of = |node_id: &str| {
            call(
                &db,
                &data_dir,
                PhoneCanvasCmd::Body {
                    name: "test".to_string(),
                    node_id: node_id.to_string(),
                },
            )
            .unwrap()
        };
        let note = body_of("note-1");
        assert_eq!(note["kind"], "note");
        assert_eq!(note["title"], "Paper A");
        assert_eq!(note["paper_id"], 1);
        assert_eq!(note["report_id"], Value::Null);
        assert!(note["markdown"].as_str().unwrap().contains("A のメモ"));

        let report = body_of(&format!("report-{report_id}"));
        assert_eq!(report["kind"], "report");
        assert!(report["markdown"].as_str().unwrap().contains("REPORT_BODY"));

        let text = body_of(&text_id);
        assert_eq!(text["kind"], "text");
        assert_eq!(text["markdown"], "研究の量 高");

        let group = body_of(&group_id);
        assert_eq!(group["kind"], "group");
        assert_eq!(group["title"], "前処理");
        assert_eq!(group["markdown"], Value::Null);

        // 無いノードはエラー。
        let error = call(
            &db,
            &data_dir,
            PhoneCanvasCmd::Body {
                name: "test".to_string(),
                node_id: "note-404".to_string(),
            },
        )
        .unwrap_err();
        assert_eq!(
            format!("{error}"),
            "ノード 'note-404' が Canvas にありません"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cli_reads_a_name_starting_with_dash_after_double_dash() {
        // sai-phone（U04）は `-` で始まるタグ名も渡せるように `--` を挟んで呼ぶ。
        let cli = crate::Cli::try_parse_from(["sai-paper", "phone", "canvas", "show", "--", "-x"])
            .expect("`--` の後ろの位置引数が読めません");
        let Some(crate::Cmd::Phone {
            cmd:
                crate::phone::PhoneCmd::Canvas {
                    cmd: PhoneCanvasCmd::Show { name },
                },
        }) = cli.cmd
        else {
            panic!("phone canvas show になっていません: {:?}", cli.cmd);
        };
        assert_eq!(name, "-x");

        // `--near` を先に渡しても読める（U04 が使う引数の順）。
        let cli = crate::Cli::try_parse_from([
            "sai-paper",
            "phone",
            "canvas",
            "toggle",
            "--near",
            "note-1",
            "--",
            "-x",
            "note-2",
        ])
        .expect("toggle の引数が読めません");
        let Some(crate::Cmd::Phone {
            cmd:
                crate::phone::PhoneCmd::Canvas {
                    cmd:
                        PhoneCanvasCmd::Toggle {
                            name,
                            node_id,
                            near,
                        },
                },
        }) = cli.cmd
        else {
            panic!("phone canvas toggle になっていません: {:?}", cli.cmd);
        };
        assert_eq!(
            (name.as_str(), node_id.as_str(), near.as_deref()),
            ("-x", "note-2", Some("note-1"))
        );

        // `phone canvas --help` に出る 7 つの命令。
        let help =
            <PhoneCanvasCmd as clap::Subcommand>::augment_subcommands(clap::Command::new("canvas"))
                .render_long_help()
                .to_string();
        for command in ["list", "new", "show", "layout", "toggle", "missing", "body"] {
            assert!(
                help.contains(command),
                "{command} が --help にありません: {help}"
            );
        }
    }
}
