//! Canvas 画面（JSON Canvas 1.0）の状態・描画・操作。
//!
//! - カードは 2 種類のみ: `note_card`（`note.md`）と `report_card`（agent の report）
//! - カード内テキストの編集は行わない（任意のタイミングで書き換わることを防ぐ）
//! - 矢印は 2 種類（同一論文の note/report・引用関係）を `s` で切り替える
//! - 矢印の手動作成・中点ラベルは提供しない
//!
//! 描画・操作は ratatui 0.29 向けに新規実装した（ekphos は ratatui 0.30 依存のため
//! 描画・操作コードは流用できない）。JSON Canvas の読み書きのみ
//! [`papr_core::canvas`]（ekphos から取り込み）を使う。

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use papr_core::canvas::{
    Canvas as JsonCanvas, CanvasColor, CanvasNode, CanvasNodeKind, canvas_file_stem, parse_canvas,
};
use papr_core::canvas_cards::{
    CanvasCardKind, CanvasCardSource, Connections, SidebarPaper, note_card_sources, paper_card_info,
    report_card_sources, sidebar_papers,
};
// Canvas の同期・自動配置・位置決めは core にある（TUI と同じ処理を使うため）。
use papr_core::canvas_sync::{auto_layout_nodes, next_free_position, place_near, sync_nodes};
use papr_core::{Database, canvas_file_path};
use ratatui::buffer::Buffer;
use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph};

use crate::theme::Theme;

/// ワールド座標 1 セル分の大きさ（ekphos と同じ数値を参考にする）。
pub const WORLD_CELL_W: f64 = 20.0;
/// ワールド座標 1 セル分の高さ。
pub const WORLD_CELL_H: f64 = 40.0;
/// カードの既定サイズ（ワールド座標）。
const DEFAULT_CARD_W: i64 = 600;
/// カードの既定高さ（ワールド座標）。
const DEFAULT_CARD_H: i64 = 280;
/// カード内に表示する行数（タイトル / paper_id / タグ / summary）。
const CARD_INNER_LINES: usize = 4;
/// カードの最小サイズ。
const MIN_CARD_W: i64 = 300;
/// カードの最小高さ。
const MIN_CARD_H: i64 = 200;
/// ズームの範囲（ekphos と同じ）。
const ZOOM_MIN: f64 = 0.25;
/// ズームの上限。
const ZOOM_MAX: f64 = 4.0;

/// 矢印の種類（`s` で切り替え、同時には表示しない）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArrowMode {
    /// 同一論文の note_card ↔ report_card（+ report 間の関係）
    PaperReport,
    /// note_card 同士の引用・被引用関係
    Citation,
    /// 選んだ札につながるものだけ（V04、既定）
    Selected,
}

impl ArrowMode {
    /// 切り替える。
    #[must_use]
    pub fn toggled(self) -> Self {
        match self {
            Self::PaperReport => Self::Citation,
            Self::Citation => Self::Selected,
            Self::Selected => Self::PaperReport,
        }
    }

    /// 画面表示用の名前。
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::PaperReport => "note↔report",
            Self::Citation => "citation",
            Self::Selected => "選んだ札のつながり",
        }
    }
}

/// ワールド座標の矩形。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorldRect {
    /// 左上 x
    pub x: i64,
    /// 左上 y
    pub y: i64,
    /// 幅
    pub w: i64,
    /// 高さ
    pub h: i64,
}

/// Canvas 上のカード 1 枚。
#[derive(Debug, Clone)]
pub struct CanvasCard {
    /// JSON Canvas のノード id
    pub node_id: String,
    /// カード種別
    pub kind: CanvasCardKind,
    /// 対応する論文
    pub papers_id: Option<i64>,
    /// 対応する report
    pub report_id: Option<String>,
    /// タイトル
    pub title: String,
    /// サブタイトル（`paper_id` など）
    pub subtitle: String,
    /// タグ
    pub tags: String,
    /// md 先頭数行
    pub summary: String,
    /// md の実パス
    pub md_path: PathBuf,
    /// ワールド座標の矩形
    pub rect: WorldRect,
}

/// Canvas 上の枠（グループ）・文字の札（テキスト）の種別（V02）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShapeKind {
    /// JSON Canvas の `group`（トピックの枠）
    Group,
    /// JSON Canvas の `text`（文字の札）
    Text,
}

/// Canvas 上の枠・文字の札 1 つ（V02）。
///
/// note / report / paper の札（[`CanvasCard`]）とは別に扱う。中身は編集しない。
#[derive(Debug, Clone)]
pub struct CanvasShape {
    /// JSON Canvas のノード id
    pub node_id: String,
    /// 枠か文字か
    pub shape: ShapeKind,
    /// ワールド座標の矩形
    pub rect: WorldRect,
    /// グループの見出し（`label`）
    pub label: String,
    /// 文字の本文（`text`）
    pub text: String,
    /// 色（JSON Canvas の `"1"`〜`"6"` またはカラーコード）
    pub color: Option<String>,
}

/// Canvas に札が無いつながり 1 件（V04）。
///
/// 右の欄の上に出して、`Enter` で札として足せる。
#[derive(Debug, Clone)]
pub struct MissingConnection {
    /// つながりの種類（表示用。`使った report` / `引用` / `note が言及` など）
    pub relation: String,
    /// 足す札の種類（[`CanvasCardKind::Paper`] か [`CanvasCardKind::Report`]）
    pub kind: CanvasCardKind,
    /// 論文の札なら `papers.id`
    pub paper_id: Option<i64>,
    /// report の札なら report_id
    pub report_id: Option<String>,
    /// 表示用タイトル
    pub title: String,
    /// 論文の `note.md`（あれば note の札として足す）
    pub note_md: Option<PathBuf>,
    /// report の md パス
    pub md_path: Option<PathBuf>,
}

/// right_sidebar の 1 行（フラット化した木）。
#[derive(Debug, Clone)]
pub struct SidebarRow {
    /// インデント段（0 = 論文、1 = report）
    pub depth: usize,
    /// 論文行なら `papers.id`
    pub papers_id: Option<i64>,
    /// report 行なら report_id
    pub report_id: Option<String>,
    /// 表示ラベル
    pub label: String,
    /// 行が表すカード種別（`None` は論文の見出し行）
    pub kind: Option<CanvasCardKind>,
    /// md の実パス（葉のみ）
    pub md_path: Option<PathBuf>,
    /// この論文に note.md があるか
    pub has_note: bool,
    /// 論文行のとき、畳める子を持つか
    pub foldable: bool,
}

/// マウスドラッグの状態。
#[derive(Debug, Clone)]
pub enum DragState {
    /// カードの移動
    MoveCard {
        node_id: String,
        start_col: i32,
        start_row: i32,
        orig: WorldRect,
    },
    /// カードのリサイズ
    ResizeCard {
        node_id: String,
        start_col: i32,
        start_row: i32,
        orig: WorldRect,
    },
    /// 背景のパン
    Pan {
        start_col: i32,
        start_row: i32,
        orig_x: f64,
        orig_y: f64,
    },
}

/// 開いている Canvas の状態。
#[derive(Debug)]
pub struct OpenCanvas {
    /// Canvas 名（拡張子なし）
    pub name: String,
    /// `.canvas` のパス
    pub path: PathBuf,
    /// JSON Canvas データ（ノードの座標が真実）
    pub canvas: JsonCanvas,
    /// 表示用カード（ノード id で canvas.nodes と対応）
    pub cards: Vec<CanvasCard>,
    /// 枠・文字の札（ノード id で canvas.nodes と対応、V02）
    pub shapes: Vec<CanvasShape>,
    /// Canvas が下書き（`extra.saiDraft: true`）か、V02
    pub draft: bool,
    /// 選択中のノード id
    pub selected: Option<String>,
    /// 画面左上のワールド x
    pub origin_x: f64,
    /// 画面左上のワールド y
    pub origin_y: f64,
    /// ズーム倍率
    pub zoom: f64,
    /// 矢印の種類
    pub arrow_mode: ArrowMode,
    /// right_sidebar の候補（フラット化前の木）
    pub sidebar: Vec<SidebarPaper>,
    /// right_sidebar のフラット化結果（描画時に更新）
    pub sidebar_rows: Vec<SidebarRow>,
    /// right_sidebar の選択位置
    pub sidebar_selected: usize,
    /// right_sidebar のスクロール
    pub sidebar_scroll: usize,
    /// 折りたたんだ論文
    pub sidebar_folded: HashSet<i64>,
    /// right_sidebar にフォーカスがあるか
    pub sidebar_focused: bool,
    /// 選んだ札のつながり（キャッシュ）。`connections_for` の札に対する結果（V04）
    pub connections: Option<Connections>,
    /// `connections` が対応する選択ノード id（V04）
    pub connections_for: Option<String>,
    /// Canvas に札が無いつながり（右の欄の上、V04）
    pub missing: Vec<MissingConnection>,
    /// `missing` の選択位置（V04）
    pub missing_selected: usize,
    /// 右の欄で `missing` を見ているか（V04）
    pub missing_focused: bool,
    /// ドラッグ状態
    pub drag: Option<DragState>,
    /// 保存が必要か
    pub dirty: bool,
    /// 初回描画でフィットさせるか
    pub pending_fit: bool,
    /// 直近に描画したキャンバス領域（キー操作の座標計算に使う）。
    pub last_area: Rect,
    /// 現在表示中の矢印（ノード id の組）。キー操作時に再計算する。
    pub edges: Vec<(String, String)>,
}

/// Canvas 画面全体の状態。
#[derive(Debug, Default)]
pub struct CanvasState {
    /// Canvas 名一覧（= タグ名。通常の Canvas → 地図（下書き）の順）
    pub names: Vec<String>,
    /// `names` の末尾にある地図（下書き）の枚数（V05）
    pub map_count: usize,
    /// 一覧の選択位置
    pub list_selected: usize,
    /// 一覧のスクロール
    pub list_scroll: usize,
    /// 開いている Canvas
    pub open: Option<OpenCanvas>,
    /// 新規作成／名前変更の入力中フラグ
    pub name_input: Option<String>,
    /// 名前変更の対象（None なら新規作成）
    pub name_input_target: Option<String>,
    /// 削除確認中の Canvas 名
    pub delete_confirm: Option<String>,
    /// 一覧（タグ）を読み直す必要があるか
    pub names_stale: bool,
}

impl CanvasState {
    /// Canvas 一覧（タグ名）と地図の枚数を入れ直す（V05）。
    ///
    /// 表示順は「通常の Canvas → 地図（下書き）」。選択位置が一覧の外に行かない
    /// ように丸める。
    pub fn set_canvas_list(&mut self, list: CanvasList) {
        self.map_count = list.maps.len();
        self.names = list.canvases.into_iter().chain(list.maps).collect();
        self.list_selected = self.list_selected.min(self.names.len().saturating_sub(1));
    }
}

// =============================================================================
// ファイル操作
// =============================================================================

/// Canvas 一覧の仕分け（V05）。
///
/// エージェントが作った下書きの地図と、ユーザーの Canvas を混ぜないために分ける。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CanvasList {
    /// 通常の Canvas（名前順）
    pub canvases: Vec<String>,
    /// 地図（下書き）（名前順）
    pub maps: Vec<String>,
}

impl CanvasList {
    /// 表示順のタグ名（通常の Canvas → 地図（下書き））。
    #[must_use]
    pub fn names(&self) -> Vec<String> {
        self.canvases.iter().chain(&self.maps).cloned().collect()
    }
}

/// 一覧で「地図（下書き）」に分けるか（V05）。
///
/// タグ名が `map-` で始まり、`.canvas` に `saiDraft: true` があるものだけを
/// 地図にする。下書きでなくなった `map-` の Canvas は通常の一覧に戻る。
#[must_use]
pub fn is_map_draft(name: &str, draft: bool) -> bool {
    name.starts_with("map-") && draft
}

/// Canvas 名（= タグ名）を一覧する（名前順）。
///
/// Canvas はタグ 1 つにつき 1 枚なので、一覧はディレクトリではなく DB のタグから作る。
/// 以前の `main.canvas` のような、タグに対応しないファイルは一覧に出さない
/// （ファイル自体は消さずにそのまま残す）。
///
/// 地図（下書き）は `map-` で始まり `.canvas` に `saiDraft: true` があるもの
/// （[`is_map_draft`]）。通常の Canvas と地図はそれぞれ名前順に並べる。
#[must_use]
pub fn list_canvases(database: &Database, data_dir: &Path) -> CanvasList {
    let mut list = CanvasList::default();
    for tag in database.tags().unwrap_or_default() {
        let name = tag.name;
        // ファイル名にできないタグ（空・`.`・`..` 等）は Canvas を作れないので除く。
        if canvas_file_stem(&name).is_none() {
            continue;
        }
        if is_map_draft(&name, canvas_file_is_draft(data_dir, &name)) {
            list.maps.push(name);
        } else {
            list.canvases.push(name);
        }
    }
    list.canvases.sort();
    list.maps.sort();
    list
}

/// タグの `.canvas` が下書き（`extra.saiDraft: true`）か（V05）。
///
/// ファイルが無い・読めないときは `false`（通常の一覧に入れる）。
fn canvas_file_is_draft(data_dir: &Path, name: &str) -> bool {
    let path = papr_core::canvas::canvas_path_for_tag(data_dir, name);
    if path.as_os_str().is_empty() || !path.is_file() {
        return false;
    }
    let Ok(source) = std::fs::read_to_string(&path) else {
        return false;
    };
    parse_canvas(&source)
        .map(|(canvas, _diagnostics)| {
            canvas
                .extra
                .get("saiDraft")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false)
        })
        .unwrap_or(false)
}

/// タグ名に対応する `.canvas` のパス（ファイル名にできないタグは空パス）。
#[must_use]
pub fn canvas_path(data_dir: &Path, tag: &str) -> PathBuf {
    papr_core::canvas::canvas_path_for_tag(data_dir, tag)
}

/// Canvas を開く（ファイルが無ければ自動配置で新規作成する）。
///
/// 開くたびに DB と同期する。タグが付いた論文のカードを足し、タグが外れた
/// 論文や消えた note / report のカードを除く。残るカードの座標は変えない。
///
/// # Errors
///
/// ファイル読み書き・DB エラー。
pub fn open_canvas(database: &Database, data_dir: &Path, name: &str) -> anyhow::Result<OpenCanvas> {
    let stem = canvas_file_stem(name)
        .ok_or_else(|| anyhow::anyhow!("タグ '{name}' は Canvas 名にできません"))?;
    let path = canvas_file_path(data_dir, &stem);
    let mut canvas = if path.is_file() {
        let source = std::fs::read_to_string(&path)?;
        parse_canvas(&source)
            .map(|(canvas, _diagnostics)| canvas)
            .map_err(|e| anyhow::anyhow!("canvas の解析に失敗: {e}"))?
    } else {
        JsonCanvas {
            nodes: Vec::new(),
            edges: Vec::new(),
            extra: std::collections::BTreeMap::new(),
        }
    };
    // ファイルからタグへ戻せるように、タグ名を Canvas 本体に残す。
    canvas.extra.insert(
        "saiTag".to_string(),
        serde_json::Value::String(name.to_string()),
    );
    let sources = papr_core::canvas_cards::tag_card_sources(
        database.connection(),
        database.path_base(),
        name,
    )?;
    let is_new = !path.is_file();
    let mut changed = if is_new {
        canvas.nodes = auto_layout_nodes(&sources);
        true
    } else {
        sync_nodes(&mut canvas, &sources)
    };
    // 理解の記録で枠の色を決め直す（V05）。開くたびに最新の状態に合わせる。
    let states = papr_core::canvas_edit::states_by_topic(
        &papr_core::understanding::latest_per_topic(database.connection())?,
    );
    if papr_core::canvas_edit::refresh_topic_colors(&mut canvas, &states) {
        changed = true;
    }
    // Canvas 本体の `saiDraft: true` を下書きの印として覚える（V02）。
    let draft = canvas
        .extra
        .get("saiDraft")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);

    let mut open = OpenCanvas {
        name: name.to_string(),
        path,
        canvas,
        cards: Vec::new(),
        shapes: Vec::new(),
        draft,
        selected: None,
        origin_x: 0.0,
        origin_y: 0.0,
        zoom: 1.0,
        arrow_mode: ArrowMode::Selected,
        sidebar: Vec::new(),
        sidebar_rows: Vec::new(),
        sidebar_selected: 0,
        sidebar_scroll: 0,
        sidebar_folded: HashSet::new(),
        sidebar_focused: false,
        connections: None,
        connections_for: None,
        missing: Vec::new(),
        missing_selected: 0,
        missing_focused: false,
        drag: None,
        dirty: false,
        pending_fit: true,
        last_area: Rect::default(),
        edges: Vec::new(),
    };
    reload_cards(database, &mut open)?;
    reload_sidebar(database, &mut open)?;
    // 初回はカードが画面に収まるようフィットさせる（描画時に領域が決まるため保留）。
    if open.cards.is_empty() {
        open.selected = None;
    } else {
        open.selected = open.cards.first().map(|card| card.node_id.clone());
    }
    // 選んだ札のつながり・矢印は、選択を決めてから求める（V04）。
    refresh_edges(database, &mut open);
    // 新規作成分・同期で増減した分をそのまま保存する。
    if changed {
        save_canvas(&mut open)?;
    }
    Ok(open)
}

/// キャンバスのノード座標からカード情報（タイトル等）を再構築する。
///
/// # Errors
///
/// DB エラー。
pub fn reload_cards(database: &Database, open: &mut OpenCanvas) -> anyhow::Result<()> {
    let conn = database.connection();
    let notes = note_card_sources(conn, database.path_base())?;
    let reports = report_card_sources(conn)?;
    let mut note_by_paper: HashMap<i64, CanvasCardSource> = HashMap::new();
    for source in notes {
        if let Some(id) = source.papers_id {
            note_by_paper.insert(id, source);
        }
    }
    let mut report_by_id: HashMap<String, CanvasCardSource> = HashMap::new();
    for source in reports {
        if let Some(id) = &source.report_id {
            report_by_id.insert(id.clone(), source);
        }
    }

    let mut cards = Vec::new();
    let mut shapes = Vec::new();
    for node in &open.canvas.nodes {
        let rect = WorldRect {
            x: node.x,
            y: node.y,
            w: node.width,
            h: node.height,
        };
        // 枠・文字の札はカードにせず、図形として描く（V02）。
        match &node.kind {
            CanvasNodeKind::Group { label, .. } => {
                shapes.push(CanvasShape {
                    node_id: node.id.clone(),
                    shape: ShapeKind::Group,
                    rect,
                    label: label.clone().unwrap_or_default(),
                    text: String::new(),
                    color: node.color.as_ref().map(CanvasColor::as_str),
                });
                continue;
            }
            CanvasNodeKind::Text { text } => {
                shapes.push(CanvasShape {
                    node_id: node.id.clone(),
                    shape: ShapeKind::Text,
                    rect,
                    label: String::new(),
                    text: text.clone(),
                    color: node.color.as_ref().map(CanvasColor::as_str),
                });
                continue;
            }
            _ => {}
        }
        let papers_id = node
            .extra
            .get("saiPaperId")
            .and_then(serde_json::Value::as_i64);
        let report_id = node
            .extra
            .get("saiReportId")
            .and_then(|v| v.as_str())
            .map(str::to_string);
        let sai_kind = node.extra.get("saiKind").and_then(|v| v.as_str());

        // 論文の札（`saiKind: "paper"`、V02）。note.md があれば note の札と同じ表示にする。
        if sai_kind == Some("paper") {
            let Some(pid) = papers_id else {
                continue;
            };
            let (kind, title, subtitle, tags, summary, md_path) =
                if let Some(source) = note_by_paper.get(&pid) {
                    (
                        CanvasCardKind::Note,
                        source.paper_title.clone(),
                        format!("paper_id: {pid}"),
                        source.tags.clone(),
                        source.summary.clone(),
                        source.md_path.clone(),
                    )
                } else if let Some(info) = paper_card_info(conn, pid)? {
                    let subtitle = if info.year.is_empty() {
                        format!("#{}", info.papers_id)
                    } else {
                        format!("#{}  {}", info.papers_id, info.year)
                    };
                    (
                        CanvasCardKind::Paper,
                        info.title.clone(),
                        subtitle,
                        info.tags.clone(),
                        "（note なし）".to_string(),
                        PathBuf::new(),
                    )
                } else {
                    // DB に無い論文 id。タイトル不明の札として表示する。
                    (
                        CanvasCardKind::Paper,
                        node.extra
                            .get("saiTitle")
                            .and_then(|v| v.as_str())
                            .unwrap_or("(不明な論文)")
                            .to_string(),
                        format!("#{pid}"),
                        String::new(),
                        "（note なし）".to_string(),
                        PathBuf::new(),
                    )
                };
            cards.push(CanvasCard {
                node_id: node.id.clone(),
                kind,
                papers_id: Some(pid),
                report_id: None,
                title,
                subtitle,
                tags,
                summary,
                md_path,
                rect,
            });
            continue;
        }

        let kind = sai_kind.and_then(CanvasCardKind::parse);
        let (kind, source) = match (kind, &report_id) {
            (Some(CanvasCardKind::Report), Some(id)) => {
                (CanvasCardKind::Report, report_by_id.get(id).cloned())
            }
            (_, Some(id)) if report_by_id.contains_key(id) => {
                (CanvasCardKind::Report, report_by_id.get(id).cloned())
            }
            (_, _) => (
                CanvasCardKind::Note,
                papers_id.and_then(|id| note_by_paper.get(&id).cloned()),
            ),
        };
        let (title, subtitle, tags, summary, md_path, papers_id) = match (&source, kind) {
            (Some(source), _) => (
                source.paper_title.clone(),
                source
                    .papers_id
                    .map(|id| format!("paper_id: {id}"))
                    .unwrap_or_default(),
                source.tags.clone(),
                source.summary.clone(),
                source.md_path.clone(),
                source.papers_id,
            ),
            (None, _) => (
                node.extra
                    .get("saiTitle")
                    .and_then(|v| v.as_str())
                    .unwrap_or("(不明なカード)")
                    .to_string(),
                papers_id
                    .map(|id| format!("paper_id: {id}"))
                    .unwrap_or_default(),
                String::new(),
                String::new(),
                node.extra
                    .get("saiMdPath")
                    .and_then(|v| v.as_str())
                    .map(PathBuf::from)
                    .unwrap_or_default(),
                papers_id,
            ),
        };
        cards.push(CanvasCard {
            node_id: node.id.clone(),
            kind,
            papers_id,
            report_id,
            title,
            subtitle,
            tags,
            summary,
            md_path,
            rect,
        });
    }
    open.cards = cards;
    open.shapes = shapes;
    Ok(())
}

/// right_sidebar の候補を読み直し、フラット化する。
///
/// # Errors
///
/// DB エラー。
pub fn reload_sidebar(database: &Database, open: &mut OpenCanvas) -> anyhow::Result<()> {
    open.sidebar = sidebar_papers(database.connection(), database.path_base())?;
    rebuild_sidebar_rows(open);
    Ok(())
}

/// 木構造を折りたたみ状態に従ってフラット化する。
pub fn rebuild_sidebar_rows(open: &mut OpenCanvas) {
    let mut rows = Vec::new();
    for paper in &open.sidebar {
        let foldable = !paper.reports.is_empty();
        rows.push(SidebarRow {
            depth: 0,
            papers_id: Some(paper.papers_id),
            report_id: None,
            label: format!("{} (paper_id: {})", paper.title, paper.papers_id),
            kind: None,
            md_path: paper.note_md.clone(),
            has_note: paper.note_md.is_some(),
            foldable,
        });
        if open.sidebar_folded.contains(&paper.papers_id) {
            continue;
        }
        // note.md の行
        if let Some(note) = &paper.note_md {
            rows.push(SidebarRow {
                depth: 1,
                papers_id: Some(paper.papers_id),
                report_id: None,
                label: format!("note.md  {}", short_summary(&paper.note_summary)),
                kind: Some(CanvasCardKind::Note),
                md_path: Some(note.clone()),
                has_note: true,
                foldable: false,
            });
        }
        for report in &paper.reports {
            rows.push(SidebarRow {
                depth: 1,
                papers_id: Some(paper.papers_id),
                report_id: Some(report.report_id.clone()),
                label: format!("{}  {}", report.report_id, short_summary(&report.summary)),
                kind: Some(CanvasCardKind::Report),
                md_path: Some(report.md_path.clone()),
                has_note: false,
                foldable: false,
            });
        }
    }
    open.sidebar_rows = rows;
    if open.sidebar_selected >= open.sidebar_rows.len() {
        open.sidebar_selected = open.sidebar_rows.len().saturating_sub(1);
    }
}

/// 1 行に収まる長さへ縮める。
fn short_summary(summary: &str) -> String {
    let trimmed = summary.trim();
    if trimmed.is_empty() {
        return "(要約なし)".to_string();
    }
    let text: String = trimmed.chars().take(48).collect();
    if trimmed.chars().count() > 48 {
        format!("{text}…")
    } else {
        text
    }
}

/// Canvas をファイルへ保存する（変更即保存）。
///
/// # Errors
///
/// ファイル書き込みエラー。
pub fn save_canvas(open: &mut OpenCanvas) -> anyhow::Result<()> {
    // ノードの座標・サイズをカード・枠・文字の状態から書き戻す。
    for node in open.canvas.nodes.iter_mut() {
        if let Some(card) = open.cards.iter().find(|c| c.node_id == node.id) {
            node.x = card.rect.x;
            node.y = card.rect.y;
            node.width = card.rect.w;
            node.height = card.rect.h;
        } else if let Some(shape) = open.shapes.iter().find(|s| s.node_id == node.id) {
            node.x = shape.rect.x;
            node.y = shape.rect.y;
            node.width = shape.rect.w;
            node.height = shape.rect.h;
        }
    }
    // 矢印は paper_id / 引用関係から毎回生成するため保存しない（要件 R5.2）。
    open.canvas.edges.clear();
    if let Some(parent) = open.path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let text = open
        .canvas
        .to_json_pretty()
        .map_err(|e| anyhow::anyhow!("canvas の保存に失敗: {e}"))?;
    std::fs::write(&open.path, text)?;
    open.dirty = false;
    Ok(())
}

// =============================================================================
// 座標変換・レイアウト
// =============================================================================

impl OpenCanvas {
    /// ワールド x → 画面列。
    #[must_use]
    pub fn world_to_col(&self, area: Rect, x: f64) -> i32 {
        area.x as i32 + ((x - self.origin_x) * self.zoom / WORLD_CELL_W).round() as i32
    }

    /// ワールド y → 画面上（カーソル行オフセット込み）。
    #[must_use]
    pub fn world_to_row(&self, area: Rect, y: f64) -> i32 {
        area.y as i32 + ((y - self.origin_y) * self.zoom / WORLD_CELL_H).round() as i32
    }

    /// 画面列 → ワールド x。
    #[must_use]
    pub fn col_to_world(&self, area: Rect, col: i32) -> f64 {
        self.origin_x + f64::from(col - area.x as i32) * WORLD_CELL_W / self.zoom
    }

    /// 画面上 → ワールド y。
    #[must_use]
    pub fn row_to_world(&self, area: Rect, row: i32) -> f64 {
        self.origin_y + f64::from(row - area.y as i32) * WORLD_CELL_H / self.zoom
    }

    /// ワールド矩形の画面矩形（枠線を含む）。
    #[must_use]
    pub fn world_rect_screen_rect(&self, area: Rect, world: WorldRect) -> Rect {
        let x = self.world_to_col(area, world.x as f64);
        let y = self.world_to_row(area, world.y as f64);
        let w = (world.w as f64 * self.zoom / WORLD_CELL_W).round().max(6.0) as i32;
        let h = (world.h as f64 * self.zoom / WORLD_CELL_H).round().max(4.0) as i32;
        Rect {
            x: x.clamp(i32::from(u16::MIN), i32::from(i16::MAX)) as u16,
            y: y.clamp(i32::from(u16::MIN), i32::from(i16::MAX)) as u16,
            width: w.clamp(2, i32::from(u16::MAX)) as u16,
            height: h.clamp(2, i32::from(u16::MAX)) as u16,
        }
    }

    /// カードの画面矩形（枠線を含む）。
    #[must_use]
    pub fn card_screen_rect(&self, area: Rect, card: &CanvasCard) -> Rect {
        self.world_rect_screen_rect(area, card.rect)
    }

    /// 枠・文字の札の画面矩形（枠線を含む、V02）。
    #[must_use]
    pub fn shape_screen_rect(&self, area: Rect, shape: &CanvasShape) -> Rect {
        self.world_rect_screen_rect(area, shape.rect)
    }

    /// 画面座標での選択可能な項目（カード・枠・文字）の node id と矩形。
    #[must_use]
    pub fn screen_items(&self, area: Rect) -> Vec<(String, Rect)> {
        let mut items: Vec<(String, Rect)> = self
            .cards
            .iter()
            .map(|card| (card.node_id.clone(), self.card_screen_rect(area, card)))
            .collect();
        items.extend(
            self.shapes
                .iter()
                .map(|shape| (shape.node_id.clone(), self.shape_screen_rect(area, shape))),
        );
        items
    }

    /// 画面座標からカードを当たり判定する。
    #[must_use]
    pub fn hit_test(&self, area: Rect, col: u16, row: u16) -> Option<&CanvasCard> {
        self.cards.iter().find(|card| {
            let rect = self.card_screen_rect(area, card);
            col >= rect.x
                && col < rect.x.saturating_add(rect.width)
                && row >= rect.y
                && row < rect.y.saturating_add(rect.height)
        })
    }

    /// 全カード・枠・文字が収まるようビューポートとズームを合わせる。
    pub fn fit(&mut self, area: Rect) {
        let rects: Vec<WorldRect> = self
            .cards
            .iter()
            .map(|card| card.rect)
            .chain(self.shapes.iter().map(|shape| shape.rect))
            .collect();
        if rects.is_empty() || area.width == 0 || area.height == 0 {
            return;
        }
        let min_x = rects.iter().map(|r| r.x).min().unwrap_or(0);
        let min_y = rects.iter().map(|r| r.y).min().unwrap_or(0);
        let max_x = rects.iter().map(|r| r.x + r.w).max().unwrap_or(0);
        let max_y = rects.iter().map(|r| r.y + r.h).max().unwrap_or(0);
        let world_w = (max_x - min_x).max(1) as f64;
        let world_h = (max_y - min_y).max(1) as f64;
        // 余白 0.84 倍（ekphos の全体表示に倣う）。
        let zoom_x = (f64::from(area.width) * WORLD_CELL_W * 0.84) / world_w;
        let zoom_y = (f64::from(area.height) * WORLD_CELL_H * 0.84) / world_h;
        // カードが少ないときに巨大化しすぎないよう上限を 1.5 にする。
        self.zoom = zoom_x.min(zoom_y).clamp(ZOOM_MIN, 1.5);
        self.origin_x = min_x as f64;
        self.origin_y = min_y as f64;
    }

    /// 選択カード・枠・文字を画面内に入るようビューポートを動かす。
    pub fn focus_card(&mut self, area: Rect, node_id: &str) {
        let rect = if let Some(card) = self.cards.iter().find(|c| c.node_id == node_id) {
            self.card_screen_rect(area, card)
        } else if let Some(shape) = self.shapes.iter().find(|s| s.node_id == node_id) {
            self.shape_screen_rect(area, shape)
        } else {
            return;
        };
        if rect.x < area.x {
            self.origin_x -= f64::from(area.x - rect.x) * WORLD_CELL_W / self.zoom;
        }
        let right = rect.x.saturating_add(rect.width);
        let area_right = area.x.saturating_add(area.width);
        if right > area_right {
            self.origin_x += f64::from(right - area_right) * WORLD_CELL_W / self.zoom;
        }
        if rect.y < area.y {
            self.origin_y -= f64::from(area.y - rect.y) * WORLD_CELL_H / self.zoom;
        }
        let bottom = rect.y.saturating_add(rect.height);
        let area_bottom = area.y.saturating_add(area.height);
        if bottom > area_bottom {
            self.origin_y += f64::from(bottom - area_bottom) * WORLD_CELL_H / self.zoom;
        }
    }

    /// 指定方向へ最も近いカード・枠・文字へ選択を移す。
    pub fn move_selection(&mut self, area: Rect, dx: i32, dy: i32) {
        let items = self.screen_items(area);
        let Some(current_id) = self.selected.clone() else {
            self.selected = items.first().map(|(id, _)| id.clone());
            return;
        };
        let Some(current_rect) = items
            .iter()
            .find(|(id, _)| *id == current_id)
            .map(|(_, rect)| *rect)
        else {
            self.selected = items.first().map(|(id, _)| id.clone());
            return;
        };
        let (cx, cy) = center_of(current_rect);
        let mut best: Option<(f64, String)> = None;
        for (id, rect) in &items {
            if *id == current_id {
                continue;
            }
            let (x, y) = center_of(*rect);
            let (ddx, ddy) = (x - cx, y - cy);
            // 方向フィルタ（射影）
            let forward = ddx * f64::from(dx) + ddy * f64::from(dy);
            if forward <= 1.0 {
                continue;
            }
            let lateral = (ddx * f64::from(dy) - ddy * f64::from(dx)).abs();
            let score = forward + lateral * 2.0;
            if best
                .as_ref()
                .is_none_or(|(best_score, _)| score < *best_score)
            {
                best = Some((score, id.clone()));
            }
        }
        if let Some((_, id)) = best {
            self.selected = Some(id.clone());
            self.focus_card(area, &id);
        }
    }

    /// 選択中のカード・枠・文字を移動する（ワールド座標）。
    pub fn nudge_selected(&mut self, dx: i64, dy: i64) {
        let Some(id) = self.selected.clone() else {
            return;
        };
        if let Some(card) = self.cards.iter_mut().find(|c| c.node_id == id) {
            card.rect.x += dx;
            card.rect.y += dy;
            self.dirty = true;
            return;
        }
        if let Some(shape) = self.shapes.iter_mut().find(|s| s.node_id == id) {
            shape.rect.x += dx;
            shape.rect.y += dy;
            self.dirty = true;
        }
    }

    /// 選択中のカード・枠・文字のサイズを変更する。
    pub fn resize_selected(&mut self, dw: i64, dh: i64) {
        let Some(id) = self.selected.clone() else {
            return;
        };
        if let Some(card) = self.cards.iter_mut().find(|c| c.node_id == id) {
            card.rect.w = (card.rect.w + dw).max(MIN_CARD_W);
            card.rect.h = (card.rect.h + dh).max(MIN_CARD_H);
            self.dirty = true;
            return;
        }
        if let Some(shape) = self.shapes.iter_mut().find(|s| s.node_id == id) {
            shape.rect.w = (shape.rect.w + dw).max(MIN_CARD_W);
            shape.rect.h = (shape.rect.h + dh).max(MIN_CARD_H);
            self.dirty = true;
        }
    }

    /// ズームを変更する（画面中心基準）。
    pub fn zoom_by(&mut self, factor: f64) {
        self.zoom = (self.zoom * factor).clamp(ZOOM_MIN, ZOOM_MAX);
    }

    /// 指定した画面位置を固定したままズームする（マウスホイール用）。
    pub fn zoom_at(&mut self, area: Rect, col: i32, row: i32, factor: f64) {
        let world_x = self.col_to_world(area, col);
        let world_y = self.row_to_world(area, row);
        self.zoom = (self.zoom * factor).clamp(ZOOM_MIN, ZOOM_MAX);
        // ポインタ下のワールド座標が動かないよう origin を補正する。
        self.origin_x = world_x - f64::from(col - area.x as i32) * WORLD_CELL_W / self.zoom;
        self.origin_y = world_y - f64::from(row - area.y as i32) * WORLD_CELL_H / self.zoom;
    }

    /// パン（画面セル単位）。
    pub fn pan(&mut self, delta_cols: i32, delta_rows: i32) {
        self.origin_x -= f64::from(delta_cols) * WORLD_CELL_W / self.zoom;
        self.origin_y -= f64::from(delta_rows) * WORLD_CELL_H / self.zoom;
    }
}

// =============================================================================
// カードの追加・削除
// =============================================================================

/// sidebar の選択行に対応するカードを Canvas に出し入れする。
///
/// 既に配置済みなら削除する。論文の見出し行（`note.md` を持たない）は何もしない。
///
/// # Returns
/// カードを追加したら `true`（削除なら `false`）。変化が無ければ `None`。
pub fn toggle_sidebar_card(database: &Database, open: &mut OpenCanvas) -> Option<bool> {
    let row = open.sidebar_rows.get(open.sidebar_selected)?.clone();
    let report_id = row.report_id.clone();
    let kind = row.kind?;
    let papers_id = row.papers_id;
    let node_id = match (&report_id, papers_id) {
        (Some(report_id), _) => format!("report-{report_id}"),
        (None, Some(paper_id)) => format!("note-{paper_id}"),
        _ => return None,
    };

    // 既にあれば削除
    if open.canvas.nodes.iter().any(|n| n.id == node_id) {
        open.canvas.nodes.retain(|n| n.id != node_id);
        open.cards.retain(|c| c.node_id != node_id);
        if open.selected.as_deref() == Some(node_id.as_str()) {
            open.selected = open.cards.first().map(|c| c.node_id.clone());
        }
        open.dirty = true;
        refresh_edges(database, open);
        return Some(false);
    }

    let conn_source = row.md_path.clone().unwrap_or_default();
    let paper_title = papers_id
        .and_then(|id| {
            open.sidebar
                .iter()
                .find(|p| p.papers_id == id)
                .map(|p| p.title.clone())
        })
        .unwrap_or_default();

    // 空いている位置に置く（既存カードの右端の次の列）
    let (x, y) = next_free_position(&open.canvas.nodes);
    let mut extra = std::collections::BTreeMap::new();
    extra.insert(
        "saiKind".to_string(),
        serde_json::Value::String(kind.as_str().to_string()),
    );
    if let Some(paper_id) = papers_id {
        extra.insert(
            "saiPaperId".to_string(),
            serde_json::Value::Number(paper_id.into()),
        );
    }
    if let Some(report_id) = &report_id {
        extra.insert(
            "saiReportId".to_string(),
            serde_json::Value::String(report_id.clone()),
        );
    }
    extra.insert(
        "saiTitle".to_string(),
        serde_json::Value::String(paper_title),
    );
    extra.insert(
        "saiMdPath".to_string(),
        serde_json::Value::String(conn_source.to_string_lossy().to_string()),
    );
    open.canvas.nodes.push(CanvasNode {
        id: node_id.clone(),
        x,
        y,
        width: DEFAULT_CARD_W,
        height: DEFAULT_CARD_H,
        color: None,
        kind: CanvasNodeKind::File {
            file: conn_source.to_string_lossy().to_string(),
            subpath: None,
        },
        extra,
    });
    open.selected = Some(node_id);
    open.dirty = true;
    refresh_edges(database, open);
    Some(true)
}

/// 選択中カードを Canvas から削除する。
pub fn remove_selected_card(database: &Database, open: &mut OpenCanvas) {
    let Some(id) = open.selected.clone() else {
        return;
    };
    open.canvas.nodes.retain(|n| n.id != id);
    open.cards.retain(|c| c.node_id != id);
    open.shapes.retain(|s| s.node_id != id);
    open.selected = open.cards.first().map(|c| c.node_id.clone());
    open.dirty = true;
    refresh_edges(database, open);
}

// =============================================================================
// 矢印
// =============================================================================

/// 表示中の矢印を再計算して `open.edges` に保存する。
///
/// 選んだ札のつながり（V04）も同時に求め直す。
pub fn refresh_edges(database: &Database, open: &mut OpenCanvas) {
    refresh_connections(database, open);
    let edges = edges_for_mode(database, open);
    open.edges = edges;
}

/// 選択中の札のつながりを求める（キャッシュを使わない）。
///
/// 選んでいる札が無い / 枠や文字の札 / DB で引けないときは `None`。
fn compute_connections(database: &Database, open: &OpenCanvas) -> Option<(String, Connections)> {
    let selected = open.selected.clone()?;
    let card = open.cards.iter().find(|c| c.node_id == selected)?;
    let papers_id = card.papers_id;
    let report_id = card.report_id.clone();
    // 選ぶ札が変わっていなければ覚えている結果を使う（note.md を読み直さない）。
    if open.connections_for.as_deref() == Some(selected.as_str()) {
        if let Some(cached) = &open.connections {
            return Some((selected, cached.clone()));
        }
    }
    let conn = database.connection();
    let connections = if let Some(report_id) = report_id {
        papr_core::canvas_cards::connections_of_report(conn, &report_id).ok()?
    } else {
        papr_core::canvas_cards::connections_of_paper(conn, database.path_base(), papers_id?)
            .ok()?
    };
    Some((selected, connections))
}

/// 選んだ札のつながりと、Canvas に無いつながりを求め直す（V04）。
///
/// モードが「選んだ札のつながり」のときだけ求める。
pub fn refresh_connections(database: &Database, open: &mut OpenCanvas) {
    if open.arrow_mode != ArrowMode::Selected {
        open.connections = None;
        open.connections_for = None;
        open.missing.clear();
        open.missing_focused = false;
        return;
    }
    let computed = compute_connections(database, open);
    let missing = match &computed {
        Some((_, connections)) => missing_connections(database, open, connections),
        None => Vec::new(),
    };
    open.connections = computed.as_ref().map(|(_, connections)| connections.clone());
    open.connections_for = computed.map(|(node_id, _)| node_id);
    open.missing = missing;
    if open.missing_selected >= open.missing.len() {
        open.missing_selected = open.missing.len().saturating_sub(1);
    }
    if open.missing.is_empty() {
        open.missing_focused = false;
    }
}

/// つながりのうち Canvas に札が無いものを集める（V04）。
fn missing_connections(
    database: &Database,
    open: &OpenCanvas,
    connections: &Connections,
) -> Vec<MissingConnection> {
    let conn = database.connection();
    let selected = open.selected.as_deref();
    let mut out = Vec::new();
    let mut seen_papers: HashSet<i64> = HashSet::new();
    for paper_id in connections
        .papers
        .iter()
        .chain(connections.notes_mentioning.iter())
    {
        if !seen_papers.insert(*paper_id) {
            continue;
        }
        // 論文の札（note / paper）が既にあれば線の相手になるので、ここには出さない。
        if open.cards.iter().any(|card| {
            card.papers_id == Some(*paper_id)
                && matches!(card.kind, CanvasCardKind::Note | CanvasCardKind::Paper)
                && Some(card.node_id.as_str()) != selected
        }) {
            continue;
        }
        let info = paper_card_info(conn, *paper_id).ok().flatten();
        let title = info
            .as_ref()
            .map(|info| info.title.clone())
            .unwrap_or_else(|| format!("#{paper_id}"));
        let note_md = papr_core::paths::paper_folder(database.path_base(), *paper_id, &title)
            .join(papr_core::paths::paper_file_names::NOTE);
        let note_md = note_md.is_file().then_some(note_md);
        let relation = if connections.notes_mentioning.contains(paper_id) {
            "note が言及"
        } else {
            "引用"
        };
        out.push(MissingConnection {
            relation: relation.to_string(),
            kind: CanvasCardKind::Paper,
            paper_id: Some(*paper_id),
            report_id: None,
            title,
            note_md: note_md.clone(),
            md_path: note_md,
        });
    }
    for report_id in &connections.reports {
        let node_id = format!("report-{report_id}");
        if open
            .cards
            .iter()
            .any(|card| card.node_id == node_id || card.report_id.as_deref() == Some(report_id))
        {
            continue;
        }
        let report = papr_core::reports::get_report(conn, report_id).ok().flatten();
        let title = report
            .as_ref()
            .and_then(|report| report.title.clone())
            .unwrap_or_else(|| report_id.clone());
        let md_path = report.as_ref().and_then(papr_core::reports::resolve_md_path);
        out.push(MissingConnection {
            relation: "report".to_string(),
            kind: CanvasCardKind::Report,
            paper_id: None,
            report_id: Some(report_id.clone()),
            title,
            note_md: None,
            md_path,
        });
    }
    out
}

/// 右の欄の「Canvas に無いつながり」から 1 件を札として足す（V04）。
///
/// 足した札は選んでいる札の近くの空いた所へ動かし、カードを読み直して
/// つながり・矢印を求め直す。足したら `true`。
pub fn add_missing_connection(database: &Database, open: &mut OpenCanvas, index: usize) -> bool {
    let Some(item) = open.missing.get(index).cloned() else {
        return false;
    };
    let anchor = open
        .selected
        .as_deref()
        .and_then(|id| open.cards.iter().find(|card| card.node_id == id))
        .map(|card| card.rect);
    let node_id = match item.kind {
        CanvasCardKind::Report => {
            let report_id = item.report_id.clone().unwrap_or_default();
            papr_core::canvas_edit::add_report(
                &mut open.canvas,
                &report_id,
                item.md_path.as_deref(),
                None,
            )
        }
        _ => {
            let paper_id = item.paper_id.unwrap_or(0);
            papr_core::canvas_edit::add_paper(
                &mut open.canvas,
                paper_id,
                item.note_md.as_deref(),
                None,
            )
        }
    };
    // 選んでいる札の近くの空いた所へ動かす。
    if let Some(rect) = anchor {
        let (x, y) = place_near(
            &open.canvas.nodes,
            &node_id,
            (rect.x, rect.y, rect.w, rect.h),
        );
        if let Some(node) = open.canvas.nodes.iter_mut().find(|node| node.id == node_id) {
            node.x = x;
            node.y = y;
        }
    }
    if reload_cards(database, open).is_err() {
        return false;
    }
    open.selected = Some(node_id);
    open.dirty = true;
    refresh_edges(database, open);
    true
}

/// 矢印（ノード id の組）を現在のモードで求める。
#[must_use]
pub fn edges_for_mode(database: &Database, open: &OpenCanvas) -> Vec<(String, String)> {
    let mut edges: Vec<(String, String)> = Vec::new();
    match open.arrow_mode {
        ArrowMode::PaperReport => {
            // 同一論文の note_card ↔ report_card
            for a in &open.cards {
                for b in &open.cards {
                    if a.node_id >= b.node_id {
                        continue;
                    }
                    if a.kind != b.kind && a.papers_id.is_some() && a.papers_id == b.papers_id {
                        edges.push((a.node_id.clone(), b.node_id.clone()));
                    }
                }
            }
            // report 間の関係（report_sources kind='report'）
            let conn = database.connection();
            for a in &open.cards {
                let Some(report_id) = a.report_id.as_deref() else {
                    continue;
                };
                if let Ok(children) = papr_core::canvas_cards::child_reports(conn, report_id) {
                    for child in children {
                        let child_node = format!("report-{child}");
                        if open.cards.iter().any(|c| c.node_id == child_node) {
                            edges.push((a.node_id.clone(), child_node));
                        }
                    }
                }
            }
        }
        ArrowMode::Citation => {
            let conn = database.connection();
            let links = papr_core::canvas_cards::citation_links(conn).unwrap_or_default();
            for (from_paper, to_paper) in links {
                if let (Some(from), Some(to)) = (
                    open.cards
                        .iter()
                        .find(|c| c.papers_id == Some(from_paper))
                        .map(|c| c.node_id.clone()),
                    open.cards
                        .iter()
                        .find(|c| c.papers_id == Some(to_paper))
                        .map(|c| c.node_id.clone()),
                ) {
                    if from != to {
                        edges.push((from, to));
                    }
                }
            }
        }
        ArrowMode::Selected => {
            // 選んだ札につながるものだけを線にする（V04）。
            // 枠・文字の札や、選んでいないときは線を出さない。
            if let Some(selected) = open.selected.clone() {
                if let Some((_, connections)) = compute_connections(database, open) {
                    for paper_id in connections
                        .papers
                        .iter()
                        .chain(connections.notes_mentioning.iter())
                    {
                        if let Some(card) = open.cards.iter().find(|card| {
                            card.papers_id == Some(*paper_id)
                                && card.node_id != selected
                                && matches!(card.kind, CanvasCardKind::Note | CanvasCardKind::Paper)
                        }) {
                            edges.push((selected.clone(), card.node_id.clone()));
                        }
                    }
                    for report_id in &connections.reports {
                        let node_id = format!("report-{report_id}");
                        if node_id != selected
                            && open.cards.iter().any(|card| card.node_id == node_id)
                        {
                            edges.push((selected.clone(), node_id));
                        }
                    }
                }
            }
        }
    }
    edges.sort();
    edges.dedup();
    edges
}

/// 画面矩形の中心。
fn center_of(rect: Rect) -> (f64, f64) {
    (
        f64::from(rect.x) + f64::from(rect.width) / 2.0,
        f64::from(rect.y) + f64::from(rect.height) / 2.0,
    )
}

/// カード矩形の辺（外向き 1 セル）のアンカー。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Side {
    Top,
    Right,
    Bottom,
    Left,
}

impl Side {
    fn anchor(self, rect: Rect) -> (i32, i32) {
        let cx = i32::from(rect.x) + i32::from(rect.width) / 2;
        let cy = i32::from(rect.y) + i32::from(rect.height) / 2;
        match self {
            Self::Top => (cx, i32::from(rect.y) - 1),
            Self::Bottom => (cx, i32::from(rect.y) + i32::from(rect.height)),
            Self::Left => (i32::from(rect.x) - 1, cy),
            Self::Right => (i32::from(rect.x) + i32::from(rect.width), cy),
        }
    }

    fn arrow_glyph(self) -> char {
        match self {
            Self::Top => '▼',
            Self::Bottom => '▲',
            Self::Left => '▶',
            Self::Right => '◀',
        }
    }
}

/// 2 矩形の最も近い辺同士を選ぶ（全 16 組を距離で比較）。
fn closest_sides(from: Rect, to: Rect) -> (Side, Side) {
    const SIDES: [Side; 4] = [Side::Top, Side::Right, Side::Bottom, Side::Left];
    let mut best: Option<(f64, Side, Side)> = None;
    for from_side in SIDES {
        let (ax, ay) = from_side.anchor(from);
        for to_side in SIDES {
            let (bx, by) = to_side.anchor(to);
            let distance = f64::from(ax - bx).hypot(f64::from(ay - by));
            if best
                .as_ref()
                .is_none_or(|(best_distance, _, _)| distance < *best_distance)
            {
                best = Some((distance, from_side, to_side));
            }
        }
    }
    best.map(|(_, from_side, to_side)| (from_side, to_side))
        .unwrap_or((Side::Right, Side::Left))
}

/// 交差を箱罫線で合成するためのラインバッファ。
#[derive(Default)]
struct LineGrid {
    cells: HashMap<(i32, i32), u8>,
}

const BIT_UP: u8 = 1;
const BIT_DOWN: u8 = 2;
const BIT_LEFT: u8 = 4;
const BIT_RIGHT: u8 = 8;

impl LineGrid {
    fn add(&mut self, x: i32, y: i32, bits: u8) {
        *self.cells.entry((x, y)).or_insert(0) |= bits;
    }

    fn hline(&mut self, x0: i32, x1: i32, y: i32) {
        let (lo, hi) = if x0 <= x1 { (x0, x1) } else { (x1, x0) };
        for x in lo..=hi {
            let mut bits = 0;
            if x > lo {
                bits |= BIT_LEFT;
            }
            if x < hi {
                bits |= BIT_RIGHT;
            }
            self.add(x, y, bits);
        }
    }

    fn vline(&mut self, y0: i32, y1: i32, x: i32) {
        let (lo, hi) = if y0 <= y1 { (y0, y1) } else { (y1, y0) };
        for y in lo..=hi {
            let mut bits = 0;
            if y > lo {
                bits |= BIT_UP;
            }
            if y < hi {
                bits |= BIT_DOWN;
            }
            self.add(x, y, bits);
        }
    }

    fn glyph(bits: u8) -> char {
        match bits {
            0 => ' ',
            1 | 2 | 3 => '│',
            4 | 8 | 12 => '─',
            5 => '┘',
            9 => '└',
            6 => '┐',
            10 => '┌',
            7 => '┤',
            11 => '├',
            13 => '┴',
            14 => '┬',
            _ => '┼',
        }
    }

    fn flush(&self, buf: &mut Buffer, style: Style) {
        for ((x, y), bits) in &self.cells {
            if *x < 0 || *y < 0 {
                continue;
            }
            let (x, y) = (*x as u16, *y as u16);
            if let Some(cell) = buf.cell_mut((x, y)) {
                cell.set_char(Self::glyph(*bits));
                cell.set_style(style);
            }
        }
    }
}

// =============================================================================
// 描画
// =============================================================================

/// Canvas ページ全体（一覧 or 本体）を描画する。
pub fn render_canvas(frame: &mut ratatui::Frame<'_>, app: &mut crate::state::App, theme: &Theme) {
    if app.mode == crate::state::AppMode::CanvasView && app.canvas.open.is_some() {
        render_canvas_view(frame, app, theme);
    } else {
        render_canvas_list(frame, app, theme);
        // 一覧画面の上にも名前入力 / 削除確認のモーダルを出す。
        render_canvas_modals(frame, frame.area(), app, theme);
    }
}

/// Canvas 一覧画面を描画する。
pub fn render_canvas_list(
    frame: &mut ratatui::Frame<'_>,
    app: &mut crate::state::App,
    theme: &Theme,
) {
    let area = frame.area();
    frame.render_widget(Clear, area);
    frame.render_widget(
        Block::default().style(Style::default().bg(theme.background)),
        area,
    );
    let outer = area.inner(ratatui::layout::Margin {
        horizontal: 3,
        vertical: 1,
    });
    let rows = ratatui::layout::Layout::vertical([
        ratatui::layout::Constraint::Length(2),
        ratatui::layout::Constraint::Min(4),
        ratatui::layout::Constraint::Length(1),
    ])
    .split(outer);
    frame.render_widget(
        Paragraph::new(format!("CANVAS  ({} 枚)", app.canvas.names.len()))
            .style(
                Style::default()
                    .fg(theme.accent)
                    .add_modifier(Modifier::BOLD),
            )
            .block(
                Block::default()
                    .borders(Borders::BOTTOM)
                    .border_style(Style::default().fg(theme.border)),
            ),
        rows[0],
    );

    if app.canvas.names.is_empty() {
        frame.render_widget(
            Paragraph::new("タグがありません。Canvas はタグ 1 つにつき 1 枚、自動で並びます。")
                .alignment(Alignment::Center)
                .style(Style::default().fg(theme.muted)),
            rows[1],
        );
    } else {
        // 通常の Canvas と地図（下書き）を見出しで分ける（V05）。
        // 見出しの行は選べないので、選択の番号（`list_selected`）は名前の並び
        // （`names`）のままで数え、見出し分だけ項目の位置をずらす。
        let names = &app.canvas.names;
        let map_count = app.canvas.map_count.min(names.len());
        let normal_count = names.len() - map_count;
        let selected_name = app.canvas.list_selected.min(names.len().saturating_sub(1));
        let header = |title: &str| {
            ListItem::new(Line::styled(
                title.to_string(),
                Style::default()
                    .fg(theme.muted)
                    .add_modifier(Modifier::BOLD),
            ))
        };
        let mut items: Vec<ListItem<'_>> = Vec::new();
        let mut selected_item = 0usize;
        if normal_count > 0 {
            items.push(header("Canvas"));
            for (index, name) in names[..normal_count].iter().enumerate() {
                if index == selected_name {
                    selected_item = items.len();
                }
                items.push(ListItem::new(Line::styled(
                    name.clone(),
                    Style::default().fg(theme.text),
                )));
            }
        }
        if map_count > 0 {
            items.push(header("地図（下書き）"));
            for (offset, name) in names[normal_count..].iter().enumerate() {
                if normal_count + offset == selected_name {
                    selected_item = items.len();
                }
                items.push(ListItem::new(Line::styled(
                    name.clone(),
                    Style::default().fg(theme.text),
                )));
            }
        }
        let mut state = ListState::default();
        state.select(Some(selected_item));
        let list = List::new(items)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_type(ratatui::widgets::BorderType::Rounded)
                    .border_style(Style::default().fg(theme.border)),
            )
            .highlight_style(
                Style::default()
                    .bg(theme.surface)
                    .fg(theme.accent)
                    .add_modifier(Modifier::BOLD),
            )
            .highlight_symbol("> ");
        frame.render_stateful_widget(list, rows[1], &mut state);
    }
    render_footer(
        frame,
        rows[2],
        theme,
        "j/k 選択  Enter 開く（タグごと）  x レイアウト削除  h back",
    );
}

/// Canvas 本体（+ right_sidebar）を描画する。
pub fn render_canvas_view(
    frame: &mut ratatui::Frame<'_>,
    app: &mut crate::state::App,
    theme: &Theme,
) {
    let area = frame.area();
    frame.render_widget(Clear, area);
    frame.render_widget(
        Block::default().style(Style::default().bg(theme.background)),
        area,
    );
    let outer = area.inner(ratatui::layout::Margin {
        horizontal: 1,
        vertical: 1,
    });
    // 下書きの Canvas は上部を 1 行広くする（V02）。
    let draft = app.canvas.open.as_ref().is_some_and(|open| open.draft);
    let rows = ratatui::layout::Layout::vertical([
        ratatui::layout::Constraint::Length(if draft { 3 } else { 2 }),
        ratatui::layout::Constraint::Min(4),
        ratatui::layout::Constraint::Length(1),
    ])
    .split(outer);

    let Some(open) = app.canvas.open.as_mut() else {
        render_footer(frame, rows[2], theme, "h back");
        return;
    };

    let mut header = format!(
        "CANVAS: {}   zoom {:.2}   arrows: {} (s)   {}/{} cards",
        open.name,
        open.zoom,
        open.arrow_mode.label(),
        open.cards.len(),
        open.canvas.nodes.len()
    );
    if open.draft {
        header.push('\n');
        header.push_str("（下書き: エージェントが作った Canvas。並べ直してよい）");
    }
    frame.render_widget(
        Paragraph::new(header)
            .style(
                Style::default()
                    .fg(theme.accent)
                    .add_modifier(Modifier::BOLD),
            )
            .block(
                Block::default()
                    .borders(Borders::BOTTOM)
                    .border_style(Style::default().fg(theme.border)),
            ),
        rows[0],
    );

    // Canvas 領域と right_sidebar（Canvas 画面のときのみ）
    let columns = ratatui::layout::Layout::horizontal([
        ratatui::layout::Constraint::Min(20),
        ratatui::layout::Constraint::Length(40),
    ])
    .split(rows[1]);
    let canvas_area = columns[0];
    let sidebar_area = columns[1];

    // 初回描画でフィットさせる（カードが画面外だと何も見えないため）
    if open.pending_fit {
        open.fit(canvas_area);
        open.pending_fit = false;
    }

    frame.render_widget(
        Block::default()
            .borders(Borders::ALL)
            .border_type(ratatui::widgets::BorderType::Rounded)
            .border_style(Style::default().fg(theme.border)),
        canvas_area,
    );
    let inner = canvas_area.inner(ratatui::layout::Margin {
        horizontal: 1,
        vertical: 1,
    });
    open.last_area = inner;

    // 1) 矢印（カードの下に描く）
    let edges = open.edges.clone();
    if !edges.is_empty() {
        let mut grid = LineGrid::default();
        let mut heads: Vec<(i32, i32, char)> = Vec::new();
        for (from_id, to_id) in &edges {
            let Some(from_card) = open.cards.iter().find(|c| &c.node_id == from_id) else {
                continue;
            };
            let Some(to_card) = open.cards.iter().find(|c| &c.node_id == to_id) else {
                continue;
            };
            let from_rect = open.card_screen_rect(inner, from_card);
            let to_rect = open.card_screen_rect(inner, to_card);
            let (from_side, to_side) = closest_sides(from_rect, to_rect);
            let (ax, ay) = from_side.anchor(from_rect);
            let (bx, by) = to_side.anchor(to_rect);
            // 直交ルータ: 出る辺の向きに応じて水平→垂直 or 垂直→水平
            match from_side {
                Side::Left | Side::Right => {
                    grid.hline(ax, bx, ay);
                    grid.vline(ay, by, bx);
                }
                Side::Top | Side::Bottom => {
                    grid.vline(ay, by, ax);
                    grid.hline(ax, bx, by);
                }
            }
            heads.push((bx, by, to_side.arrow_glyph()));
        }
        let style = Style::default().fg(theme.muted);
        let buf = frame.buffer_mut();
        grid.flush(buf, style);
        for (x, y, glyph) in heads {
            if x < 0 || y < 0 {
                continue;
            }
            if let Some(cell) = buf.cell_mut((x as u16, y as u16)) {
                cell.set_char(glyph);
                cell.set_style(Style::default().fg(theme.accent));
            }
        }
    }

    // 2) グループの枠（カードより先に描いて後ろに置く、V02）
    let selected = open.selected.clone();
    for shape in &open.shapes {
        if shape.shape != ShapeKind::Group {
            continue;
        }
        let rect = open.shape_screen_rect(inner, shape);
        if rect.x >= inner.x.saturating_add(inner.width)
            || rect.y >= inner.y.saturating_add(inner.height)
        {
            continue;
        }
        let is_selected = selected.as_deref() == Some(shape.node_id.as_str());
        render_group(frame, inner, rect, shape, theme, is_selected);
    }

    // 3) 文字の札（V02）
    for shape in &open.shapes {
        if shape.shape != ShapeKind::Text {
            continue;
        }
        let rect = open.shape_screen_rect(inner, shape);
        if rect.x >= inner.x.saturating_add(inner.width)
            || rect.y >= inner.y.saturating_add(inner.height)
        {
            continue;
        }
        let is_selected = selected.as_deref() == Some(shape.node_id.as_str());
        render_text_shape(frame, inner, rect, shape, theme, is_selected);
    }

    // 4) カード（note / report / paper）
    for card in &open.cards {
        let rect = open.card_screen_rect(inner, card);
        if rect.x >= inner.x.saturating_add(inner.width)
            || rect.y >= inner.y.saturating_add(inner.height)
        {
            continue;
        }
        let is_selected = selected.as_deref() == Some(card.node_id.as_str());
        render_card(frame, inner, rect, card, theme, is_selected);
    }

    // 5) right_sidebar（上: Canvas に無いつながり、下: candidates、V04）
    if open.missing.is_empty() {
        render_sidebar(frame, sidebar_area, open, theme);
    } else {
        let missing_h = (u16::try_from(open.missing.len()).unwrap_or(u16::MAX) + 2)
            .clamp(3, 10)
            .min(sidebar_area.height.saturating_sub(3).max(3));
        let areas = ratatui::layout::Layout::vertical([
            ratatui::layout::Constraint::Length(missing_h),
            ratatui::layout::Constraint::Min(3),
        ])
        .split(sidebar_area);
        render_missing(frame, areas[0], open, theme);
        render_sidebar(frame, areas[1], open, theme);
    }

    render_footer(
        frame,
        rows[2],
        theme,
        "hjkl 選択  Alt+矢印 移動  Alt+Shift+矢印 サイズ  +/- zoom  f fit  s 矢印切替  a 追加/削除  d カード削除  Tab sidebar  Enter md  h/Esc back",
    );
    render_canvas_modals(frame, area, app, theme);
}

/// Canvas 上のモーダル（名前入力・削除確認）を描画する。
fn render_canvas_modals(
    frame: &mut ratatui::Frame<'_>,
    area: Rect,
    app: &crate::state::App,
    theme: &Theme,
) {
    match app.mode {
        crate::state::AppMode::CanvasName => {
            let Some(input) = app.canvas.name_input.as_deref() else {
                return;
            };
            let action = if app.canvas.name_input_target.is_some() {
                "名前変更"
            } else {
                "新規作成"
            };
            let popup = centered(60, 5, area);
            frame.render_widget(Clear, popup);
            frame.render_widget(
                Paragraph::new(format!("Canvas 名（{action}）: {input}_"))
                    .wrap(ratatui::widgets::Wrap { trim: true })
                    .block(
                        Block::default()
                            .borders(Borders::ALL)
                            .border_type(ratatui::widgets::BorderType::Rounded)
                            .border_style(Style::default().fg(theme.accent)),
                    ),
                popup,
            );
        }
        crate::state::AppMode::CanvasConfirmDelete => {
            let Some(name) = app.canvas.delete_confirm.as_deref() else {
                return;
            };
            let popup = centered(60, 5, area);
            frame.render_widget(Clear, popup);
            frame.render_widget(
                Paragraph::new(format!(
                    "Canvas '{name}' の配置を削除しますか？（次に開くと自動配置に戻ります）  y/Enter で削除  Esc で中止"
                ))
                .wrap(ratatui::widgets::Wrap { trim: true })
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .border_type(ratatui::widgets::BorderType::Rounded)
                        .border_style(Style::default().fg(theme.error)),
                ),
                popup,
            );
        }
        _ => {}
    }
}

/// 画面中央の矩形（モーダル用）。
fn centered(width: u16, height: u16, area: Rect) -> Rect {
    let width = width.min(area.width);
    let height = height.min(area.height);
    Rect {
        x: area.x + (area.width.saturating_sub(width)) / 2,
        y: area.y + (area.height.saturating_sub(height)) / 2,
        width,
        height,
    }
}

/// カード 1 枚を描画する。
fn render_card(
    frame: &mut ratatui::Frame<'_>,
    area: Rect,
    rect: Rect,
    card: &CanvasCard,
    theme: &Theme,
    selected: bool,
) {
    // 画面外・小さすぎる場合はスキップ
    if rect.width < 6 || rect.height < 4 {
        return;
    }
    let visible = rect.intersection(area);
    if visible.width == 0 || visible.height == 0 {
        return;
    }
    let kind_label = match card.kind {
        CanvasCardKind::Note => "note",
        CanvasCardKind::Report => "report",
        CanvasCardKind::Paper => "paper",
    };
    let border_style = if selected {
        Style::default().fg(theme.accent)
    } else {
        Style::default().fg(theme.border)
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(ratatui::widgets::BorderType::Rounded)
        .border_style(border_style)
        .title(Span::styled(
            format!(" {kind_label} "),
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        ));
    let inner = block.inner(rect);
    frame.render_widget(block, rect);

    let lines = card_lines(card, inner.width as usize, theme);
    if inner.height > 0 {
        frame.render_widget(Paragraph::new(lines), inner);
    }
}

/// カードの中身（4 行）を組み立てる（V02 でテストできるよう切り出し）。
fn card_lines(card: &CanvasCard, width: usize, theme: &Theme) -> Vec<Line<'static>> {
    let mut lines: Vec<Line<'static>> = vec![
        Line::styled(
            truncate_chars(&card.title, width),
            Style::default().fg(theme.text).add_modifier(Modifier::BOLD),
        ),
        Line::styled(
            truncate_chars(&card.subtitle, width),
            Style::default().fg(theme.muted),
        ),
        Line::styled(
            truncate_chars(&card.tags, width),
            Style::default().fg(theme.secondary),
        ),
        Line::styled(
            truncate_chars(&card.summary, width),
            Style::default().fg(theme.muted),
        ),
    ];
    lines.truncate(CARD_INNER_LINES);
    lines
}

/// グループ（トピックの枠）1 つを描画する（V02）。
fn render_group(
    frame: &mut ratatui::Frame<'_>,
    area: Rect,
    rect: Rect,
    shape: &CanvasShape,
    theme: &Theme,
    selected: bool,
) {
    if rect.width < 6 || rect.height < 3 {
        return;
    }
    let visible = rect.intersection(area);
    if visible.width == 0 || visible.height == 0 {
        return;
    }
    let color = canvas_color(shape.color.as_deref()).unwrap_or(theme.border);
    let border = if selected { theme.accent } else { color };
    let title = if shape.label.is_empty() {
        String::new()
    } else {
        format!(" {} ", shape.label)
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(ratatui::widgets::BorderType::Rounded)
        .border_style(Style::default().fg(border))
        .title(Span::styled(
            title,
            Style::default()
                .fg(color)
                .add_modifier(Modifier::BOLD),
        ));
    frame.render_widget(block, rect);
}

/// 文字の札（テキスト）1 つを描画する（V02）。
fn render_text_shape(
    frame: &mut ratatui::Frame<'_>,
    area: Rect,
    rect: Rect,
    shape: &CanvasShape,
    theme: &Theme,
    selected: bool,
) {
    if rect.width < 6 || rect.height < 3 {
        return;
    }
    let visible = rect.intersection(area);
    if visible.width == 0 || visible.height == 0 {
        return;
    }
    let color = canvas_color(shape.color.as_deref()).unwrap_or(theme.border);
    let border = if selected { theme.accent } else { color };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(ratatui::widgets::BorderType::Rounded)
        .border_style(Style::default().fg(border))
        .title(Span::styled(
            " text ",
            Style::default()
                .fg(color)
                .add_modifier(Modifier::BOLD),
        ));
    let inner = block.inner(rect);
    frame.render_widget(block, rect);
    if inner.height > 0 && inner.width > 0 {
        let lines = text_shape_lines(&shape.text, inner.width as usize, theme);
        frame.render_widget(Paragraph::new(lines), inner);
    }
}

/// 文字の札の本文を行に組み立てる（折り返し、V02）。
fn text_shape_lines(text: &str, width: usize, theme: &Theme) -> Vec<Line<'static>> {
    wrap_text(text, width)
        .into_iter()
        .map(|line| Line::styled(line, Style::default().fg(theme.text)))
        .collect()
}

/// 文字数で折り返す（単純な文字単位の折り返し）。
fn wrap_text(text: &str, width: usize) -> Vec<String> {
    if width == 0 {
        return Vec::new();
    }
    let mut out = Vec::new();
    for raw in text.lines() {
        if raw.is_empty() {
            out.push(String::new());
            continue;
        }
        let mut current = String::new();
        for ch in raw.chars() {
            current.push(ch);
            if current.chars().count() >= width {
                out.push(std::mem::take(&mut current));
            }
        }
        if !current.is_empty() {
            out.push(current);
        }
    }
    out
}

/// JSON Canvas の色（`"1"`〜`"6"` またはカラーコード）を ratatui の色にする（V02）。
///
/// `"1"`=赤 / `"2"`=橙 / `"3"`=黄 / `"4"`=緑 / `"5"`=水色 / `"6"`=紫。
/// 解釈できない値（灰など）は `None`（theme の既定色を使う）。
fn canvas_color(value: Option<&str>) -> Option<Color> {
    let value = value?;
    match value {
        "1" => Some(Color::Red),
        "2" => Some(Color::Rgb(255, 165, 0)),
        "3" => Some(Color::Yellow),
        "4" => Some(Color::Green),
        "5" => Some(Color::Cyan),
        "6" => Some(Color::Magenta),
        hex if hex.len() == 7 && hex.starts_with('#') => {
            let red = u8::from_str_radix(&hex[1..3], 16).ok()?;
            let green = u8::from_str_radix(&hex[3..5], 16).ok()?;
            let blue = u8::from_str_radix(&hex[5..7], 16).ok()?;
            Some(Color::Rgb(red, green, blue))
        }
        _ => None,
    }
}

/// right_sidebar を描画する。
fn render_sidebar(
    frame: &mut ratatui::Frame<'_>,
    area: Rect,
    open: &mut OpenCanvas,
    theme: &Theme,
) {
    let items: Vec<ListItem<'_>> =
        open.sidebar_rows
            .iter()
            .map(|row| {
                let marker = match row.kind {
                    Some(_) => {
                        if open.cards.iter().any(|c| {
                            sidebar_row_node_id(row).as_deref() == Some(c.node_id.as_str())
                        }) {
                            "●"
                        } else {
                            "○"
                        }
                    }
                    None => {
                        if row.foldable {
                            if open.sidebar_folded.contains(&row.papers_id.unwrap_or(-1)) {
                                "▸"
                            } else {
                                "▾"
                            }
                        } else {
                            " "
                        }
                    }
                };
                let indent = "  ".repeat(row.depth);
                let style = if row.kind.is_none() {
                    Style::default().fg(theme.text)
                } else {
                    Style::default().fg(theme.muted)
                };
                ListItem::new(Line::styled(
                    format!("{indent}{marker} {}", row.label),
                    style,
                ))
            })
            .collect();

    let mut state = ListState::default();
    if !items.is_empty() {
        state.select(Some(
            open.sidebar_selected.min(items.len().saturating_sub(1)),
        ));
    }
    let highlight = if open.sidebar_focused && !open.missing_focused {
        Style::default()
            .bg(theme.surface)
            .fg(theme.accent)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().bg(theme.surface)
    };
    let list = List::new(items)
        .block(
            Block::default()
                .title(" candidates ")
                .borders(Borders::ALL)
                .border_type(ratatui::widgets::BorderType::Rounded)
                .border_style(Style::default().fg(theme.border)),
        )
        .highlight_style(highlight)
        .highlight_symbol("> ");
    frame.render_stateful_widget(list, area, &mut state);
}

/// 右の欄の上「Canvas に無いつながり」を描画する（V04）。
fn render_missing(
    frame: &mut ratatui::Frame<'_>,
    area: Rect,
    open: &OpenCanvas,
    theme: &Theme,
) {
    let items: Vec<ListItem<'_>> = open
        .missing
        .iter()
        .map(|item| {
            let number = match (&item.report_id, item.paper_id) {
                (Some(report_id), _) => report_id.clone(),
                (None, Some(paper_id)) => format!("#{paper_id}"),
                (None, None) => String::new(),
            };
            ListItem::new(Line::styled(
                format!("{}  {}  {}", item.relation, number, item.title),
                Style::default().fg(theme.muted),
            ))
        })
        .collect();
    let mut state = ListState::default();
    if !items.is_empty() {
        state.select(Some(
            open.missing_selected.min(items.len().saturating_sub(1)),
        ));
    }
    let highlight = if open.missing_focused {
        Style::default()
            .bg(theme.surface)
            .fg(theme.accent)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().bg(theme.surface)
    };
    let title = format!(
        " Canvas に無いつながり: {} 件（Enter で足す） ",
        open.missing.len()
    );
    let list = List::new(items)
        .block(
            Block::default()
                .title(title)
                .borders(Borders::ALL)
                .border_type(ratatui::widgets::BorderType::Rounded)
                .border_style(Style::default().fg(if open.missing_focused {
                    theme.accent
                } else {
                    theme.border
                })),
        )
        .highlight_style(highlight)
        .highlight_symbol("> ");
    frame.render_stateful_widget(list, area, &mut state);
}

/// sidebar 行が対応するカードの node id。
#[must_use]
pub fn sidebar_row_node_id(row: &SidebarRow) -> Option<String> {
    match (&row.report_id, row.papers_id, row.kind) {
        (Some(report_id), _, Some(CanvasCardKind::Report)) => Some(format!("report-{report_id}")),
        (None, Some(paper_id), Some(CanvasCardKind::Note)) => Some(format!("note-{paper_id}")),
        _ => None,
    }
}

/// フッターを描画する（トーストがあれば右寄せで重ねる）。
fn render_footer(frame: &mut ratatui::Frame<'_>, area: Rect, theme: &Theme, hint: &str) {
    frame.render_widget(
        Paragraph::new(hint).style(Style::default().fg(theme.muted)),
        area,
    );
}

/// 文字数で切り詰める（`…` 付き）。
fn truncate_chars(value: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    let count = value.chars().count();
    if count <= width {
        return value.to_string();
    }
    let text: String = value.chars().take(width.saturating_sub(1)).collect();
    format!("{text}…")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn closest_sides_prefers_nearest_edges() {
        // 右隣の矩形とは Right ↔ Left になる
        let a = Rect::new(0, 0, 10, 5);
        let b = Rect::new(20, 0, 10, 5);
        assert_eq!(closest_sides(a, b), (Side::Right, Side::Left));
        // 下の矩形とは Bottom ↔ Top になる
        let c = Rect::new(0, 20, 10, 5);
        assert_eq!(closest_sides(a, c), (Side::Bottom, Side::Top));
    }

    #[test]
    fn line_grid_glyphs_combine_crossings() {
        let mut grid = LineGrid::default();
        grid.hline(0, 2, 0);
        grid.vline(0, 2, 1);
        assert_eq!(LineGrid::glyph(grid.cells[&(0, 0)]), '─');
        assert_eq!(LineGrid::glyph(grid.cells[&(1, 0)]), '┬');
        assert_eq!(LineGrid::glyph(grid.cells[&(1, 1)]), '│');
        assert_eq!(LineGrid::glyph(grid.cells[&(1, 2)]), '│');
    }

    #[test]
    fn sidebar_row_node_id_matches_card_ids() {
        let row = SidebarRow {
            depth: 1,
            papers_id: Some(7),
            report_id: Some("R-0003".into()),
            label: String::new(),
            kind: Some(CanvasCardKind::Report),
            md_path: None,
            has_note: false,
            foldable: false,
        };
        assert_eq!(sidebar_row_node_id(&row).as_deref(), Some("report-R-0003"));
        let note = SidebarRow {
            depth: 1,
            papers_id: Some(7),
            report_id: None,
            label: String::new(),
            kind: Some(CanvasCardKind::Note),
            md_path: None,
            has_note: true,
            foldable: false,
        };
        assert_eq!(sidebar_row_node_id(&note).as_deref(), Some("note-7"));
    }

    #[test]
    fn canvas_file_stem_sanitizes_tag_names() {
        assert_eq!(canvas_file_stem("nlp/llm").as_deref(), Some("nlp_llm"));
        assert_eq!(canvas_file_stem("  ").as_deref(), None);
        assert_eq!(canvas_file_stem("..").as_deref(), None);
    }

    #[test]
    fn truncate_chars_adds_ellipsis() {
        assert_eq!(truncate_chars("abc", 5), "abc");
        assert_eq!(truncate_chars("abcdef", 4), "abc…");
        assert_eq!(truncate_chars("abcdef", 0), "");
    }

    #[test]
    fn canvas_round_trip_preserves_node_rects() {
        let mut canvas = papr_core::canvas::Canvas {
            nodes: vec![CanvasNode {
                id: "note-1".into(),
                x: 10,
                y: 20,
                width: 600,
                height: 280,
                color: None,
                kind: CanvasNodeKind::File {
                    file: "/tmp/note.md".into(),
                    subpath: None,
                },
                extra: std::collections::BTreeMap::new(),
            }],
            edges: Vec::new(),
            extra: std::collections::BTreeMap::new(),
        };
        let text = canvas.to_json_pretty().unwrap();
        let (parsed, diagnostics) = parse_canvas(&text).unwrap();
        assert!(diagnostics.is_empty());
        assert_eq!(parsed.nodes[0].x, 10);
        assert_eq!(parsed.nodes[0].y, 20);
        canvas.nodes[0].x = 30;
        assert_ne!(canvas.nodes[0].x, parsed.nodes[0].x);
    }

    fn test_theme() -> Theme {
        Theme::load("dracula").unwrap()
    }

    fn line_text(line: &Line<'_>) -> String {
        line.spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect()
    }

    #[test]
    fn card_lines_show_paper_card_fields() {
        let theme = test_theme();
        let card = CanvasCard {
            node_id: "paper-7".to_string(),
            kind: CanvasCardKind::Paper,
            papers_id: Some(7),
            report_id: None,
            title: "Paper G".to_string(),
            subtitle: "#7  2024".to_string(),
            tags: "nlp, llm".to_string(),
            summary: "（note なし）".to_string(),
            md_path: PathBuf::new(),
            rect: WorldRect {
                x: 0,
                y: 0,
                w: 600,
                h: 280,
            },
        };
        let lines = card_lines(&card, 40, &theme);
        assert_eq!(lines.len(), CARD_INNER_LINES);
        assert_eq!(line_text(&lines[0]), "Paper G");
        assert_eq!(line_text(&lines[1]), "#7  2024");
        assert_eq!(line_text(&lines[2]), "nlp, llm");
        assert_eq!(line_text(&lines[3]), "（note なし）");
    }

    #[test]
    fn text_shape_lines_wrap_at_width() {
        let theme = test_theme();
        let lines = text_shape_lines("abcdef\nxy", 3, &theme);
        let text: Vec<String> = lines.iter().map(line_text).collect();
        assert_eq!(text, vec!["abc", "def", "xy"]);
    }

    #[test]
    fn render_group_and_text_shape_draw_border_and_label() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        fn row_text(buffer: &ratatui::buffer::Buffer, y: u16) -> String {
            (0..buffer.area.width)
                .map(|x| buffer[(x, y)].symbol().chars().next().unwrap_or(' '))
                .collect()
        }

        let theme = test_theme();
        let group = CanvasShape {
            node_id: "group-1".to_string(),
            shape: ShapeKind::Group,
            rect: WorldRect {
                x: 0,
                y: 0,
                w: 600,
                h: 280,
            },
            label: "Topic".to_string(),
            text: String::new(),
            color: Some("4".to_string()),
        };
        let text = CanvasShape {
            node_id: "text-1".to_string(),
            shape: ShapeKind::Text,
            rect: WorldRect {
                x: 0,
                y: 0,
                w: 600,
                h: 280,
            },
            label: String::new(),
            text: "hello".to_string(),
            color: None,
        };

        let mut terminal = Terminal::new(TestBackend::new(40, 16)).unwrap();
        terminal
            .draw(|frame| {
                let area = frame.area();
                render_group(frame, area, Rect::new(0, 0, 30, 8), &group, &theme, false);
                render_text_shape(frame, area, Rect::new(0, 9, 30, 5), &text, &theme, false);
            })
            .unwrap();
        let buffer = terminal.backend().buffer().clone();
        // 枠線と見出し、文字の本文が出ている。
        assert!(
            row_text(&buffer, 0).contains("Topic"),
            "{}",
            row_text(&buffer, 0)
        );
        assert!(
            row_text(&buffer, 10).contains("hello"),
            "{}",
            row_text(&buffer, 10)
        );
    }

    #[test]
    fn canvas_color_maps_presets_and_hex() {
        assert_eq!(canvas_color(Some("1")), Some(Color::Red));
        assert_eq!(canvas_color(Some("4")), Some(Color::Green));
        assert_eq!(
            canvas_color(Some("#123456")),
            Some(Color::Rgb(0x12, 0x34, 0x56))
        );
        // 灰など解釈できない値は None（theme の既定色を使う）。
        assert_eq!(canvas_color(Some("")), None);
        assert_eq!(canvas_color(None), None);
    }

    #[test]
    fn open_canvas_keeps_pinned_shapes_and_paper_card_round_trip() {
        let dir = std::env::temp_dir().join(format!(
            "sai_canvas_v02_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let data_dir = dir.join(".sai");
        std::fs::create_dir_all(&data_dir).unwrap();
        let database = Database::in_memory_with_data_dir(&data_dir).unwrap();
        database
            .connection()
            .execute(
                "INSERT INTO papers (id, title, published_at, pdf_path) VALUES (7, 'Paper G', '2024-05-01', '/tmp/g.pdf')",
                [],
            )
            .unwrap();

        let path = canvas_path(&data_dir, "topic");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        // グループ・文字・論文の札（固定）と、同期で消える note の札を混ぜる。
        let source = r##"{
          "saiDraft": true,
          "nodes": [
            {"id":"group-1","type":"group","x":0,"y":0,"width":900,"height":600,"label":"Topic","color":"4"},
            {"id":"text-1","type":"text","x":40,"y":40,"width":300,"height":120,"text":"memo"},
            {"id":"paper-7","type":"file","file":"","x":100,"y":100,"width":600,"height":280,"color":"2","saiKind":"paper","saiPaperId":7,"saiPinned":true},
            {"id":"note-99","type":"file","file":"/tmp/x.md","x":0,"y":700,"width":600,"height":280,"saiKind":"note","saiPaperId":99}
          ],
          "edges": []
        }"##;
        std::fs::write(&path, source).unwrap();

        let mut open = open_canvas(&database, &data_dir, "topic").unwrap();
        assert!(open.draft, "saiDraft が下書きの印になる");
        assert_eq!(open.cards.len(), 1, "消えるのは note-99 だけ、paper-7 は残る");
        let card = &open.cards[0];
        assert_eq!(card.kind, CanvasCardKind::Paper);
        assert_eq!(card.title, "Paper G");
        assert_eq!(card.subtitle, "#7  2024");
        assert_eq!(card.summary, "（note なし）");
        assert_eq!(open.shapes.len(), 2, "group-1 と text-1");

        // 枠と論文の札を動かして保存 → 開き直しても座標が残る。
        open.selected = Some("group-1".to_string());
        open.nudge_selected(240, 0);
        open.selected = Some("paper-7".to_string());
        open.nudge_selected(0, 80);
        save_canvas(&mut open).unwrap();

        let reopened = open_canvas(&database, &data_dir, "topic").unwrap();
        assert!(reopened.draft);
        assert_eq!(reopened.cards.len(), 1);
        assert_eq!(reopened.shapes.len(), 2);
        assert_eq!(reopened.cards[0].rect.y, 180);
        let group = reopened
            .shapes
            .iter()
            .find(|shape| shape.node_id == "group-1")
            .expect("group-1");
        assert_eq!(group.rect.x, 240);

        // note.md ができたら note の札として表示する（ノードは paper のまま）。
        let note_dir = papr_core::paths::paper_folder(&data_dir, 7, "Paper G");
        std::fs::create_dir_all(&note_dir).unwrap();
        let note_path = note_dir.join("note.md");
        std::fs::write(&note_path, "# Note title\n\nFirst line.\n").unwrap();
        let with_note = open_canvas(&database, &data_dir, "topic").unwrap();
        assert_eq!(with_note.cards.len(), 1);
        assert_eq!(with_note.cards[0].kind, CanvasCardKind::Note);
        assert_eq!(with_note.cards[0].md_path, note_path);
        assert!(
            with_note.canvas.nodes.iter().any(|n| n.id == "paper-7"),
            "ノードの種別は変えない"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// AC3: 一覧を通常の Canvas と地図（下書き）に分ける。
    #[test]
    fn list_canvases_splits_map_drafts() {
        assert!(is_map_draft("map-test", true));
        assert!(
            !is_map_draft("map-test", false),
            "下書きでなければ地図でない"
        );
        assert!(
            !is_map_draft("普通のタグ", true),
            "`map-` で始まらなければ地図でない"
        );

        let dir = std::env::temp_dir().join(format!(
            "sai_canvas_v05_list_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let data_dir = dir.join(".sai");
        std::fs::create_dir_all(&data_dir).unwrap();
        let database = Database::in_memory_with_data_dir(&data_dir).unwrap();

        // 通常の Canvas: ファイルが無いタグと、`map-` でも下書きでないタグ。
        for name in ["notes", "z-topic"] {
            database.ensure_tag(name).unwrap();
        }
        database.ensure_tag("map-old").unwrap();
        let settled = papr_core::canvas_edit::new_canvas("map-old", false);
        papr_core::canvas_edit::save(&data_dir, "map-old", &settled).unwrap();

        // 地図（下書き）。
        for name in ["map-test", "map-alpha"] {
            database.ensure_tag(name).unwrap();
            let canvas = papr_core::canvas_edit::new_canvas(name, true);
            papr_core::canvas_edit::save(&data_dir, name, &canvas).unwrap();
        }

        let list = list_canvases(&database, &data_dir);
        assert_eq!(list.maps, vec!["map-alpha", "map-test"]);
        for name in ["map-old", "notes", "z-topic"] {
            assert!(
                list.canvases.contains(&name.to_string()),
                "{name} が通常の一覧にない: {:?}",
                list.canvases
            );
        }
        assert!(
            !list.canvases.iter().any(|name| list.maps.contains(name)),
            "地図が通常の一覧に混ざっている: {:?}",
            list.canvases
        );
        // それぞれ名前順に並ぶ。
        let mut sorted = list.canvases.clone();
        sorted.sort();
        assert_eq!(list.canvases, sorted, "通常の Canvas は名前順");
        // 表示順の末尾が地図（下書き）。
        let total = list.names().len();
        assert_eq!(&list.names()[total - 2..], &list.maps[..]);

        // `set_canvas_list` は選択位置を一覧の中に丸める。
        let mut state = CanvasState {
            list_selected: 99,
            ..CanvasState::default()
        };
        state.set_canvas_list(list);
        assert_eq!(state.map_count, 2);
        assert_eq!(state.names.len(), total);
        assert_eq!(state.list_selected, total - 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// AC2: 開き直すと理解の記録で枠の色が決め直される / `saiColorFixed` は変わらない。
    #[test]
    fn open_canvas_recolors_topics_from_understanding() {
        let dir = std::env::temp_dir().join(format!(
            "sai_canvas_v05_color_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let data_dir = dir.join(".sai");
        std::fs::create_dir_all(&data_dir).unwrap();
        let database = Database::in_memory_with_data_dir(&data_dir).unwrap();
        database.ensure_tag("map-test").unwrap();

        let path = canvas_path(&data_dir, "map-test");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let source = r##"{
          "saiDraft": true,
          "nodes": [
            {"id":"group-1","type":"group","x":0,"y":0,"width":900,"height":600,"label":"前処理","saiTopic":"前処理の違い","saiPinned":true},
            {"id":"group-2","type":"group","x":1000,"y":0,"width":900,"height":600,"label":"評価","color":"5","saiTopic":"評価の分け方","saiColorFixed":true,"saiPinned":true}
          ],
          "edges": []
        }"##;
        std::fs::write(&path, source).unwrap();

        let color_of = |open: &OpenCanvas, id: &str| {
            open.shapes
                .iter()
                .find(|shape| shape.node_id == id)
                .and_then(|shape| shape.color.clone())
        };

        // 記録なし → 色なし（灰）。
        let open = open_canvas(&database, &data_dir, "map-test").unwrap();
        assert_eq!(color_of(&open, "group-1"), None);

        // あいまいを記録して開き直す → 黄。固定の枠は cyan のまま。
        papr_core::understanding::add(database.connection(), "前処理の違い", "vague", "", None)
            .unwrap();
        let open = open_canvas(&database, &data_dir, "map-test").unwrap();
        assert_eq!(color_of(&open, "group-1").as_deref(), Some("3"));
        assert_eq!(color_of(&open, "group-2").as_deref(), Some("5"));

        // 理解しているを記録 → 緑。
        papr_core::understanding::add(database.connection(), "前処理の違い", "ok", "", None)
            .unwrap();
        let open = open_canvas(&database, &data_dir, "map-test").unwrap();
        assert_eq!(color_of(&open, "group-1").as_deref(), Some("4"));
        assert_eq!(color_of(&open, "group-2").as_deref(), Some("5"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 線の向きを無視して組を揃える（小さい id を先にする）。
    fn normalized_edges(edges: &[(String, String)]) -> Vec<(String, String)> {
        let mut out: Vec<(String, String)> = edges
            .iter()
            .map(|(a, b)| {
                if a <= b {
                    (a.clone(), b.clone())
                } else {
                    (b.clone(), a.clone())
                }
            })
            .collect();
        out.sort();
        out
    }

    #[test]
    fn arrow_mode_cycles_three_modes() {
        assert_eq!(ArrowMode::PaperReport.toggled(), ArrowMode::Citation);
        assert_eq!(ArrowMode::Citation.toggled(), ArrowMode::Selected);
        assert_eq!(ArrowMode::Selected.toggled(), ArrowMode::PaperReport);
        assert_eq!(ArrowMode::Selected.label(), "選んだ札のつながり");
    }

    #[test]
    fn selected_mode_edges_only_to_cards_on_canvas() {
        let dir = std::env::temp_dir().join(format!(
            "sai_canvas_v04_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let data_dir = dir.join(".sai");
        std::fs::create_dir_all(&data_dir).unwrap();
        let database = Database::in_memory_with_data_dir(&data_dir).unwrap();
        database
            .connection()
            .execute_batch(
                "INSERT INTO papers (id, title, pdf_path) VALUES
                   (1, 'Paper A', '/tmp/a.pdf'),
                   (2, 'Paper B', '/tmp/b.pdf');",
            )
            .unwrap();
        // 論文 2 の note が `#1` を書いている（→ 論文 1 の note のつながり）。
        let folder = papr_core::paths::paper_folder(&data_dir, 2, "Paper B");
        std::fs::create_dir_all(&folder).unwrap();
        let note2 = folder.join(papr_core::paths::paper_file_names::NOTE);
        std::fs::write(&note2, "# Note B\n\nsee #1\n").unwrap();
        // 論文 1 を使った report（Canvas には置かない）。
        let report_md = dir.join("R-0001.md");
        std::fs::write(&report_md, "# Report\n").unwrap();
        papr_core::reports::create_report(
            database.connection(),
            &papr_core::reports::NewReport {
                md_path: report_md.to_string_lossy().to_string(),
                sources: vec![papr_core::reports::ReportSource {
                    kind: "paper".into(),
                    ref_id: "1".into(),
                    role: None,
                }],
                ..Default::default()
            },
        )
        .unwrap();

        // Canvas には論文 1 の札と、論文 2 の note の札だけを置く。
        let mut canvas = papr_core::canvas_edit::new_canvas("topic", false);
        let _ = papr_core::canvas_edit::add_paper(&mut canvas, 1, None, None);
        let _ = papr_core::canvas_edit::add_paper(&mut canvas, 2, Some(&note2), None);
        papr_core::canvas_edit::save(&data_dir, "topic", &canvas).unwrap();

        let mut open = open_canvas(&database, &data_dir, "topic").unwrap();
        assert_eq!(open.arrow_mode, ArrowMode::Selected, "既定は選んだ札のつながり");
        assert_eq!(open.cards.len(), 2);

        open.selected = Some("paper-1".to_string());
        refresh_edges(&database, &mut open);
        // 論文 2 の note の札へだけ線が出る（向きは描画しないので組で見る）。
        assert_eq!(
            normalized_edges(&open.edges),
            vec![("note-2".to_string(), "paper-1".to_string())],
            "edges: {:?}",
            open.edges
        );
        // Canvas に無い report は線にならず、右の欄の数に入る。
        assert_eq!(open.missing.len(), 1);
        assert_eq!(open.missing[0].kind, CanvasCardKind::Report);
        assert_eq!(open.missing[0].report_id.as_deref(), Some("R-0001"));

        // 一覧から足すと札が増え、つながりから外れる。
        assert!(add_missing_connection(&database, &mut open, 0));
        assert!(
            open.cards.iter().any(|c| c.node_id == "report-R-0001"),
            "cards: {:?}",
            open.cards.iter().map(|c| &c.node_id).collect::<Vec<_>>()
        );
        assert!(open.missing.is_empty());
        assert_eq!(
            normalized_edges(&open.edges),
            vec![("paper-1".to_string(), "report-R-0001".to_string())],
            "edges: {:?}",
            open.edges
        );

        // つながりの無い札を選ぶと線は出ない。
        open.selected = Some("note-2".to_string());
        refresh_edges(&database, &mut open);
        assert!(open.edges.is_empty(), "edges: {:?}", open.edges);
        assert!(open.missing.is_empty());

        let _ = std::fs::remove_dir_all(&dir);
    }
}
