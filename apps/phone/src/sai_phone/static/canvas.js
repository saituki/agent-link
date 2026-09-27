"use strict";
// sai-phone の Canvas（実装計画 U06）。
// 一覧は #/papers/canvas、盤面は #/canvas/<プロジェクト>/<Canvas 名>。
//
// sai-paper-tui の Canvas で挙げられた問題を起こさないことを優先している:
//   - 見える範囲（tx, ty, z）だけを動かし、札は盤面の座標に固定する
//   - 盤面は overflow: hidden。札も線も左右の欄や見出しにはみ出さない
//   - 線は全部出し、選んだ札につながる線だけを濃くする
//   - 矢印キーで見える範囲が上下左右に動く（alt が付いていてもよい）
//   - 左右の欄は見出しのボタンでいつでも開閉できる
//
// 依存は足さない。札・枠・文字は HTML の div、線は同じ中の SVG。
// DOM を使わない純関数（住所の往復・全体表示・拡大・線のパス・濃さ）は
// 末尾で node に渡す（tests/test_canvas.js）。
// 左右の欄の中身は canvaspanel.js（U07）が cvpRender / cvpOnSelect で入れる。

// --- 定数 ---

// 拡大率の上下限（これより外へは出さない）
const CV_ZOOM_MIN = 0.1;
const CV_ZOOM_MAX = 2.5;
// 「全体が入る」ときの余白（画面上の px）
const CV_FIT_PAD = 40;
// 矢印キーで見える範囲を動かす量（画面上の px）
const CV_KEY_PAN = 80;
// ドラッグとクリックの境（これ以上動いたらドラッグ、未満ならクリック）
const CV_DRAG_SLOP = 4;
// 画面がこの幅以上なら左右の欄を盤面の横に並べる（未満は引き出し）
const CV_WIDE_PX = 900;
// 拡大率がこの未満のときは札をitle だけ大きく描く（遠くから見ても読めるように）
const CV_FAR_Z = 0.45;

// 線の種類（[kind, 名前, 色]。C-C と計画 U06 の表）
const CV_EDGE_KINDS = [
  ["citation", "引用", "#2563eb"],
  ["uses", "使った", "#1d7a45"],
  ["derived", "元にした", "#8e44ad"],
  ["mentions", "言及", "#d97706"],
  ["link", "線", "#6b7280"],
];
// 札の種類の色（左の縁の帯）
const CV_CARD_KINDS = { note: "#2563eb", report: "#1d7a45", paper: "#8a8f98" };
// 枠の色（JSON Canvas の色。1〜6 と #rrggbb。それ以外は色なし）
const CV_GROUP_COLORS = {
  "1": "#e5484d", "2": "#f76b15", "3": "#ffc53d",
  "4": "#30a46c", "5": "#05a2c2", "6": "#8e4ec6",
};
// 札・枠・文字の既定の大きさ（width / height が無いとき）
const CV_CARD_W = 240;
const CV_CARD_H = 140;

// 覚える場所（sessionStorage / localStorage）
const CV_VIEW_KEY = "sai_phone_canvas_view";
const CV_KINDS_KEY = "sai_phone_canvas_kinds";
const CV_PANELS_KEY = "sai_phone_canvas_panels";

// --- 状態（共有契約 C-F。U07 が読む。U06 は自分で書き換える） ---

let cvState = {
  project: "",   // プロジェクト名（論文の置き場があるもの）
  name: "",      // Canvas 名
  data: null,    // GET .../canvas/{canvas} の応答（C-C の show の JSON）
  selected: null // 選んでいるノード id（無ければ null）
};

// 見える範囲（画面上の点 = ワールドの点 * z + (tx, ty)）
let cvView = { tx: 0, ty: 0, z: 1 };
// 「元に戻す」用の配置（{id:{x,y,width,height}}）。1 つだけ覚える
let cvUndo = null;
// 出している線の種類（{citation:true, ...}）
let cvKinds = null;
// 左右の欄を開けているか
let cvPanelsOpen = null;
// 道具行の下の赤い 1 行の文言
let cvErrorText = "";
// 線の SVG（cvRenderEdges が使い回す）
let cvSvg = null;
// 押しているポインタ（Map。1 本 = 移動、2 本 = ピンチ）
let cvPtrs = new Map();
// 1 本指で押したままの情報
let cvDown = null;
// 2 本指の情報
let cvPinch = null;
// ノード id -> 描いた要素（ドラッグのとき直接動かすため）
let cvNodeEls = new Map();
// 枠の id -> 描いた要素（選択の印をつけるため）
let cvGroupEls = new Map();

// 一覧の状態
let cvListData = null; // GET の応答
let cvListError = "";  // 失敗の文言

// ============================================================================
// 記憶の読み書き（プライベートブラウズでも動くように try で囲む）
// ============================================================================

/** localStorage から JSON を読む。読めなければ fallback。 */
function cvLoadJson(key, fallback) {
  try {
    const raw = localStorage.getItem(key);
    if (!raw) return fallback;
    const obj = JSON.parse(raw);
    return obj && typeof obj === "object" ? obj : fallback;
  } catch { return fallback; }
}

/** localStorage へ JSON を書く。書けなくても動作は続ける。 */
function cvSaveJson(key, value) {
  try { localStorage.setItem(key, JSON.stringify(value)); } catch { /* 動けばよい */ }
}

/** 見える範囲を覚える場所（同じ Canvas を開き直したら戻す）。 */
function cvViewKey() { return `${CV_VIEW_KEY}:${cvState.project}/${cvState.name}`; }

/** 覚えている見える範囲を読む。無い・壊れている場合は null。 */
function cvLoadView() {
  try {
    const raw = sessionStorage.getItem(cvViewKey());
    if (!raw) return null;
    const v = JSON.parse(raw);
    if (!v || typeof v !== "object") return null;
    const tx = Number(v.tx), ty = Number(v.ty), z = Number(v.z);
    if (!Number.isFinite(tx) || !Number.isFinite(ty) || !Number.isFinite(z)) return null;
    return { tx, ty, z: Math.max(CV_ZOOM_MIN, Math.min(CV_ZOOM_MAX, z)) };
  } catch { return null; }
}

/** 見える範囲を覚える。 */
function cvSaveView() {
  try { sessionStorage.setItem(cvViewKey(), JSON.stringify(cvView)); } catch { /* 動けばよい */ }
}

// ============================================================================
// 純関数（DOM を使わない。node のテストから読む）
// ============================================================================

/**
 * 盤面の住所（#/canvas/<プロジェクト>/<Canvas 名>）を作る。
 * 日本語と空白を含む名前でも往復すると元に戻るように URL エンコードする。
 */
function cvBoardHash(project, name) {
  return `#/canvas/${encodeURIComponent(project || "")}/${encodeURIComponent(name || "")}`;
}

/**
 * 盤面の住所から {project, name} を取り出す。盤面の住所以外なら null。
 */
function cvParseBoardHash(hash) {
  const m = /^#\/canvas\/([^/]+)\/([^/]+)$/.exec(String(hash || ""));
  if (!m) return null;
  try {
    return { project: decodeURIComponent(m[1]), name: decodeURIComponent(m[2]) };
  } catch {
    return null;
  }
}

/**
 * 全部のノードが入る見える範囲を返す。z は 1 を超えない。
 * ノードが無ければ {tx:0, ty:0, z:1}。
 */
function cvFitView(nodes, width, height) {
  const list = (Array.isArray(nodes) ? nodes : [])
    .filter((n) => n && Number.isFinite(Number(n.x)) && Number.isFinite(Number(n.y)));
  if (list.length === 0) return { tx: 0, ty: 0, z: 1 };
  let minX = Infinity, minY = Infinity, maxX = -Infinity, maxY = -Infinity;
  for (const n of list) {
    const x = Number(n.x), y = Number(n.y);
    const w = Number(n.width) || 0, h = Number(n.height) || 0;
    if (x < minX) minX = x;
    if (y < minY) minY = y;
    if (x + w > maxX) maxX = x + w;
    if (y + h > maxY) maxY = y + h;
  }
  const bw = Math.max(1, maxX - minX);
  const bh = Math.max(1, maxY - minY);
  const availW = Math.max(40, Number(width) - CV_FIT_PAD * 2);
  const availH = Math.max(40, Number(height) - CV_FIT_PAD * 2);
  let z = Math.min(1, availW / bw, availH / bh);
  z = Math.max(CV_ZOOM_MIN, Math.min(CV_ZOOM_MAX, z));
  return {
    tx: (Number(width) - bw * z) / 2 - minX * z,
    ty: (Number(height) - bh * z) / 2 - minY * z,
    z,
  };
}

/**
 * (px, py) の下のワールドの点が動かないように拡大率を変える。
 * 新しい見える範囲を返す。z は CV_ZOOM_MIN〜CV_ZOOM_MAX に収まる。
 */
function cvZoomAt(view, px, py, factor) {
  const v = view || {};
  const z0 = Math.max(CV_ZOOM_MIN, Math.min(CV_ZOOM_MAX, Number(v.z) || 1));
  const tx0 = Number(v.tx) || 0;
  const ty0 = Number(v.ty) || 0;
  const x = Number(px) || 0;
  const y = Number(py) || 0;
  let z = z0 * (Number(factor) || 1);
  z = Math.max(CV_ZOOM_MIN, Math.min(CV_ZOOM_MAX, z));
  const wx = (x - tx0) / z0;
  const wy = (y - ty0) / z0;
  return { tx: x - wx * z, ty: y - wy * z, z };
}

// 辺の中点。side は right / left / top / bottom
function cvSidePoint(r, side) {
  const x = Number(r && r.x) || 0;
  const y = Number(r && r.y) || 0;
  const w = Number(r && r.width) || 0;
  const h = Number(r && r.height) || 0;
  if (side === "left") return { x, y: y + h / 2 };
  if (side === "top") return { x: x + w / 2, y };
  if (side === "bottom") return { x: x + w / 2, y: y + h };
  return { x: x + w, y: y + h / 2 }; // right
}

// 辺から外へ出る向き（制御点に使う）
const CV_SIDE_NORMAL = { left: [-1, 0], right: [1, 0], top: [0, -1], bottom: [0, 1] };

// 点から四角までの距離（点が入っていれば 0）
function cvPointRectDist(p, r) {
  const x = Number(r && r.x) || 0, y = Number(r && r.y) || 0;
  const w = Number(r && r.width) || 0, h = Number(r && r.height) || 0;
  const dx = Math.max(x - p.x, 0, p.x - (x + w));
  const dy = Math.max(y - p.y, 0, p.y - (y + h));
  return Math.hypot(dx, dy);
}

/**
 * 2 つの四角を結ぶ辺の組を返す（a 側の辺, b 側の辺）。
 * 相手に最も近い辺どうしを返すので、横に並んだ札は ["right","left"] になる。
 */
function cvClosestSides(a, b) {
  const order = ["right", "left", "top", "bottom"];
  let bestA = "right", bestB = "left";
  let da = Infinity, db = Infinity;
  for (const side of order) {
    const d = cvPointRectDist(cvSidePoint(a, side), b);
    if (d < da) { da = d; bestA = side; }
  }
  for (const side of order) {
    const d = cvPointRectDist(cvSidePoint(b, side), a);
    if (d < db) { db = d; bestB = side; }
  }
  return [bestA, bestB];
}

// 小数 1 位に丸める（path の文字列を短くする）
function cvR1(n) { return Math.round(n * 10) / 10; }

/** 制御点 2 つの計算（cvEdgePath と link のラベルの置き場で使う）。 */
function cvEdgeControls(a, b) {
  const sides = cvClosestSides(a, b);
  const p1 = cvSidePoint(a, sides[0]);
  const p2 = cvSidePoint(b, sides[1]);
  const n1 = CV_SIDE_NORMAL[sides[0]] || [1, 0];
  const n2 = CV_SIDE_NORMAL[sides[1]] || [-1, 0];
  const dist = Math.hypot(p2.x - p1.x, p2.y - p1.y);
  const k = Math.max(40, dist * 0.4);
  return {
    p1,
    p2,
    c1: { x: p1.x + n1[0] * k, y: p1.y + n1[1] * k },
    c2: { x: p2.x + n2[0] * k, y: p2.y + n2[1] * k },
  };
}

/**
 * 2 つの四角を結ぶ 3 次ベジェ曲線の d を返す（"M ... C ..."）。
 * 制御点は辺から max(40, 2 端の距離 * 0.4) だけ外へ出る。
 */
function cvEdgePath(a, b) {
  const { p1, p2, c1, c2 } = cvEdgeControls(a, b);
  return `M ${cvR1(p1.x)} ${cvR1(p1.y)} C ${cvR1(c1.x)} ${cvR1(c1.y)}, ${cvR1(c2.x)} ${cvR1(c2.y)}, ${cvR1(p2.x)} ${cvR1(p2.y)}`;
}

/**
 * 線の種類の色。表に無い種類（null や未知の値）も灰色 #6b7280 で返す。
 * null の kind で画面が止まらないため、例外は投げない。
 */
function cvEdgeKindColor(kind) {
  const key = kind == null ? "" : String(kind);
  const row = CV_EDGE_KINDS.find((k) => k[0] === key);
  return row ? row[2] : CV_EDGE_KINDS[4][2]; // link の灰色
}

/**
 * 線の種類。{key, known, color} を返す。
 * 表に無い種類は known:false（矢印の定義が無いので丸で終点を示す）。
 */
function cvEdgeKind(kind) {
  const key = kind == null ? "" : String(kind);
  const row = CV_EDGE_KINDS.find((k) => k[0] === key);
  if (row) return { key: row[0], known: true, color: row[2] };
  return { key: "", known: false, color: CV_EDGE_KINDS[4][2] };
}

/** 線のパスの終点の x（終点の丸を置くため）。 */
function cvEdgePathEndX(a, b) { return cvEdgeControls(a, b).p2.x; }

/** 線のパスの終点の y。 */
function cvEdgePathEndY(a, b) { return cvEdgeControls(a, b).p2.y; }

/** 3 次ベジェの t=0.5 の点（link のラベルの置き場）。 */
function cvBezierMid(p1, c1, c2, p2) {
  return {
    x: 0.125 * p1.x + 0.375 * c1.x + 0.375 * c2.x + 0.125 * p2.x,
    y: 0.125 * p1.y + 0.375 * c1.y + 0.375 * c2.y + 0.125 * p2.y,
  };
}

/**
 * 線の濃さ。選んでいなければ 0.7、選んだ札につながる線は 1、それ以外は 0.12。
 */
function cvEdgeOpacity(edge, selected) {
  if (!selected) return 0.7;
  if (!edge) return 0.12;
  if (edge.from === selected || edge.to === selected) return 1;
  return 0.12;
}

/**
 * 枠の色。JSON Canvas の色（1〜6 か #rrggbb）。それ以外は ""。
 * 表の要素は Object.prototype のどれとも衝突しないチェックをしてから取り出す。
 */
function cvGroupColor(color) {
  const key = String(color == null ? "" : color).trim();
  if (!key) return "";
  if (Object.prototype.hasOwnProperty.call(CV_GROUP_COLORS, key)) return CV_GROUP_COLORS[key];
  return /^#[0-9a-fA-F]{6}$/.test(key) ? key : "";
}

/**
 * 札の種類の色と表示名。U03 の show は DB に無い札の kind を null で返すことがある
 * （title は "(不明な札)"）。null や表に無い種類でも画面が止まらないように、
 * どちらも paper の灰色（#8a8f98）で描く。
 * @returns {{key:string, label:string, color:string}}
 */
function cvCardKind(kind) {
  const key = kind == null ? "" : String(kind).trim();
  if (key && Object.prototype.hasOwnProperty.call(CV_CARD_KINDS, key)) {
    return { key, label: key, color: CV_CARD_KINDS[key] };
  }
  // null / 空 / 未知の種類。帯の色も種類名も paper にそろえる
  return { key: "", label: "paper", color: CV_CARD_KINDS.paper };
}

/** "#rrggbb" を "rgba(r,g,b,a)" に。色は "#rrggbb" だけを受け付ける。 */
function cvAlpha(hex, alpha) {
  const m = /^#([0-9a-fA-F]{6})$/.exec(String(hex || ""));
  if (!m) return "";
  const v = parseInt(m[1], 16);
  return `rgba(${(v >> 16) & 255}, ${(v >> 8) & 255}, ${v & 255}, ${alpha})`;
}

// ============================================================================
// 盤面のデータ（C-C の応答を読む）
// ============================================================================

/** 盤面の API の住所（共有契約 C-F）。 */
function cvApi(path) {
  return `/api/projects/${encodeURIComponent(cvState.project)}/papers/canvas/${encodeURIComponent(cvState.name)}${path || ""}`;
}

/** 一覧の API の住所。 */
function cvListApi() {
  return `/api/projects/${encodeURIComponent(selectedPaperProject())}/papers/canvas`;
}

/** 描いているノード（隠している札は入れない）。 */
function cvNodes() {
  const data = cvState.data;
  if (!data || !Array.isArray(data.nodes)) return [];
  const hidden = new Set(Array.isArray(data.hidden) ? data.hidden : []);
  return data.nodes.filter((n) => n && n.id != null && !hidden.has(n.id));
}

/** id のノード（無ければ null）。 */
function cvNodeById(id) {
  if (!id) return null;
  return cvNodes().find((n) => n.id === id) || null;
}

/** 出している線（両端に札があるものだけ）。 */
function cvVisibleEdges() {
  const data = cvState.data;
  if (!data || !Array.isArray(data.edges)) return [];
  const on = cvVisibleKinds();
  const ids = new Set(cvNodes().map((n) => n.id));
  return data.edges.filter((e) => e && ids.has(e.from) && ids.has(e.to) && on[e.kind] !== false);
}

/** stage（#cv-stage）の幅と高さ。 */
function cvStageSize() {
  const stage = document.getElementById("cv-stage");
  if (!stage) return { w: 0, h: 0 };
  return { w: stage.clientWidth || 0, h: stage.clientHeight || 0 };
}

// ============================================================================
// 一覧（#/papers/canvas）
// ============================================================================

/**
 * Canvas の一覧を開く。opts.force で一覧を取り直す。
 */
async function cvOpenList(opts) {
  fillSegment("canvas-seg", "canvas");
  // 切り替えが 5 つになるので少し小さくする（canvas.css の .cv-seg-row）
  const segRow = document.getElementById("canvas-seg");
  if (segRow) segRow.classList.add("cv-seg-row");
  updateProjectChips();
  await ensurePaperProjects(opts && opts.force);
  if (location.hash !== "#/papers/canvas") return; // 既に別の画面へ移っている
  if (!selectedPaperProject()) {
    // 論文の置き場があるプロジェクトが 1 つも無い（Library と同じ案内文）
    cvListData = null;
    cvListError = "";
    cvRenderList();
    return;
  }
  if (cvListData === null) cvRenderList();
  await cvLoadList();
}

/** 一覧を取り直す。 */
async function cvLoadList() {
  const hash = location.hash;
  const name = selectedPaperProject();
  if (!name) return;
  try {
    const res = await apiFetch(cvListApi());
    const body = await res.json().catch(() => ({}));
    if (!res.ok) throw new Error(body.detail || "読み込めませんでした");
    if (location.hash !== hash || selectedPaperProject() !== name) return;
    cvListData = body && typeof body === "object" ? body : {};
    cvListError = "";
    window.SaiShell.ok();
  } catch (e) {
    if (location.hash !== hash || selectedPaperProject() !== name) return;
    cvListError = (e && e.message) || "読み込めませんでした";
    window.SaiShell.fail();
    cvRenderList();
    return;
  }
  cvRenderList();
}

/** 一覧を描く（見出し 2 つ: Canvas / 地図（下書き））。 */
function cvRenderList() {
  const box = document.getElementById("canvas-list-body");
  if (!box) return;
  box.innerHTML = "";
  if (cvListData === null) {
    box.appendChild(el("p", "lib-empty", cvListError || paperProjectsError || "読み込み中..."));
    return;
  }
  const sections = [
    ["Canvas", Array.isArray(cvListData.canvases) ? cvListData.canvases : []],
    ["地図（下書き）", Array.isArray(cvListData.maps) ? cvListData.maps : []],
  ];
  let total = 0;
  for (const [title, items] of sections) {
    total += items.length;
    box.appendChild(el("div", "cv-list-head", `${title}（${items.length}）`));
    if (items.length === 0) {
      box.appendChild(el(
        "p", "lib-empty",
        title === "Canvas" ? "Canvas がありません" : "地図がありません"
      ));
      continue;
    }
    for (const item of items) cvListRow(box, item);
  }
  if (total === 0) box.appendChild(el("p", "lib-empty", "まだ Canvas がありません。＋ から作れます。"));
}

/** 一覧の 1 行（名前と「◯ 枚」）。押すと盤面へ移る。 */
function cvListRow(box, item) {
  const name = String((item && item.name) || "");
  if (!name) return;
  const btn = document.createElement("button");
  btn.className = "cv-list-row";
  btn.appendChild(el("span", "cv-list-name", name));
  btn.appendChild(el("span", "cv-list-count", `${Number((item && item.nodes) || 0)} 枚`));
  btn.addEventListener("click", () => {
    location.hash = cvBoardHash(selectedPaperProject(), name);
  });
  box.appendChild(btn);
}

/** 見出しの ＋。名前が空ならやめる。 */
async function cvCreateCanvas() {
  const name = prompt("新しい Canvas の名前");
  if (!name) return;
  const project = selectedPaperProject();
  if (!project) return;
  try {
    const res = await apiFetch(cvListApi(), {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ name }),
    });
    if (!res.ok) {
      cvListError = await resDetail(res, "Canvas を作れませんでした");
      cvRenderList();
      return;
    }
    window.SaiShell.ok();
  } catch {
    cvListError = "Canvas を作れませんでした";
    cvRenderList();
    return;
  }
  location.hash = cvBoardHash(project, name);
}

// ============================================================================
// 盤面（#/canvas/<プロジェクト>/<Canvas 名>）
// ============================================================================

/** 出している線の種類を返す（localStorage から。最初は全部出す）。 */
function cvVisibleKinds() {
  if (cvKinds) return cvKinds;
  const on = {};
  for (const [kind] of CV_EDGE_KINDS) on[kind] = true;
  const stored = cvLoadJson(CV_KINDS_KEY, null);
  if (stored) {
    for (const [kind] of CV_EDGE_KINDS) {
      if (typeof stored[kind] === "boolean") on[kind] = stored[kind];
    }
  }
  cvKinds = on;
  return cvKinds;
}

/** 左右の欄を開けているか（localStorage から。最初は幅で決める）。 */
function cvVisiblePanels() {
  if (cvPanelsOpen) return cvPanelsOpen;
  const wide = window.innerWidth >= CV_WIDE_PX;
  const stored = cvLoadJson(CV_PANELS_KEY, null);
  if (stored && typeof stored.left === "boolean" && typeof stored.right === "boolean") {
    cvPanelsOpen = { left: stored.left, right: stored.right };
  } else {
    cvPanelsOpen = { left: wide, right: wide };
  }
  return cvPanelsOpen;
}

/** 盤面を開く。見える範囲は sessionStorage に覚え、無ければ全体が入るように合わせる。 */
async function cvOpenBoard(params) {
  cvState.project = params.project;
  cvState.name = params.name;
  cvState.data = null;
  cvState.selected = null;
  cvUndo = null;
  cvErrorText = "";
  cvPtrs = new Map();
  cvDown = null;
  cvPinch = null;
  cvKinds = null;
  cvNodeEls = new Map();

  const title = document.getElementById("cv-title");
  if (title) title.textContent = cvState.name;
  const mark = document.getElementById("cv-draft");
  if (mark) mark.hidden = true;

  cvApplyPanelClass();
  cvRenderKindButtons();
  cvRenderError();

  // 左右の欄を空にしてから「読み込み中...」を出す
  const left = document.getElementById("cv-left");
  const right = document.getElementById("cv-right");
  if (left) left.innerHTML = "";
  if (right) right.innerHTML = "";
  const stage = document.getElementById("cv-stage");
  if (stage) {
    const world = document.getElementById("cv-world");
    if (world) world.innerHTML = "";
    stage.appendChild(el("p", "cv-loading", "読み込み中..."));
  }

  try {
    const res = await apiFetch(cvApi(""));
    const body = await res.json().catch(() => ({}));
    if (!res.ok) throw new Error(body.detail || "読み込めませんでした");
    cvState.data = body && typeof body === "object" ? body : {};
    window.SaiShell.ok();
  } catch (e) {
    cvState.data = null;
    cvErrorText = (e && e.message) || "読み込めませんでした";
    window.SaiShell.fail();
    cvRenderError();
    return;
  }

  const draft = document.getElementById("cv-draft");
  if (draft) draft.hidden = !cvState.data.draft;

  const size = cvStageSize();
  const saved = cvLoadView();
  cvView = saved || cvFitView(cvNodes(), size.w, size.h);

  cvRender();
  if (typeof cvpRender === "function") cvpRender();
  cvSaveView();
}

/** show を取り直して描き直す。見える範囲は変えない。 */
async function cvReload() {
  if (!cvState.name) return;
  const project = cvState.project;
  const name = cvState.name;
  try {
    const res = await apiFetch(cvApi(""));
    const body = await res.json().catch(() => ({}));
    if (!res.ok) throw new Error(body.detail || "読み込めませんでした");
    if (cvState.project !== project || cvState.name !== name) return; // 別の盤面に移動した
    cvState.data = body && typeof body === "object" ? body : {};
    window.SaiShell.ok();
  } catch (e) {
    if (cvState.project !== project || cvState.name !== name) return;
    cvErrorText = (e && e.message) || "読み込めませんでした";
    window.SaiShell.fail();
    cvRenderError();
    return;
  }
  cvRenderError();
  if (cvState.selected && !cvNodeById(cvState.selected)) {
    // 選んでいた札が無くなっていたら選択を外す
    cvState.selected = null;
    if (typeof cvpOnSelect === "function") cvpOnSelect(null);
  }
  cvRender();
  if (typeof cvpRender === "function") cvpRender();
}

/**
 * 札を選ぶ（null で選択を外す）。opts.focus が真ならその札を画面の中央に動かす。
 * 見える範囲の移動だけで、拡大率は変えない。
 */
function cvSelect(nodeId, opts) {
  cvState.selected = nodeId == null ? null : nodeId;
  for (const [id, node] of cvNodeEls) {
    node.classList.toggle("cv-sel", id === cvState.selected);
    // 選んだ札とつながっていない札は薄くする
    node.classList.toggle("cv-dim", !!cvState.selected && !cvLinkedTo(cvState.selected, id));
  }
  for (const [id, node] of cvGroupEls) {
    node.classList.toggle("cv-sel", id === cvState.selected);
  }
  cvRenderEdges();
  if (opts && opts.focus && cvState.selected) {
    const n = cvNodeById(cvState.selected);
    if (n) {
      const size = cvStageSize();
      cvView = {
        tx: size.w / 2 - ((Number(n.x) || 0) + (Number(n.width) || 0) / 2) * cvView.z,
        ty: size.h / 2 - ((Number(n.y) || 0) + (Number(n.height) || 0) / 2) * cvView.z,
        z: cvView.z,
      };
      cvApplyView();
    }
  }
  if (typeof cvpOnSelect === "function") cvpOnSelect(cvState.selected);
}

// --- 描く ---

/** 盤面全体を書き直す。 */
function cvRender() {
  const world = document.getElementById("cv-world");
  if (!world) return;
  const stage = document.getElementById("cv-stage");
  if (stage) {
    for (const p of stage.querySelectorAll(".cv-loading")) p.remove();
  }
  world.innerHTML = "";
  cvNodeEls = new Map();
  cvGroupEls = new Map();
  const nodes = cvNodes();

  // 1. 枠（線より下に置く）
  for (const n of nodes) {
    if (n.type !== "group") continue;
    const el = cvGroupEl(n);
    world.appendChild(el);
    cvGroupEls.set(n.id, el);
  }
  // 2. 線（枠の上・札の下）
  const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
  svg.setAttribute("id", "cv-edges");
  svg.classList.add("cv-edges");
  world.appendChild(svg);
  cvSvg = svg;
  // 3. 文字と札（一番上）
  for (const n of nodes) {
    if (n.type === "group") continue;
    const node = n.type === "text" ? cvTextEl(n) : cvCardEl(n);
    world.appendChild(node);
    cvNodeEls.set(n.id, node);
  }

  cvRenderEdges();
  cvRenderZoom();
  cvApplyView();
}

/** 枠の要素（角丸 16px。見出しは左上の外側）。 */
function cvGroupEl(n) {
  const box = document.createElement("div");
  box.className = "cv-group";
  box.setAttribute("data-cv-id", n.id);
  box.setAttribute("data-cv-group", "1");
  box.style.left = `${Number(n.x) || 0}px`;
  box.style.top = `${Number(n.y) || 0}px`;
  box.style.width = `${Number(n.width) || 0}px`;
  box.style.height = `${Number(n.height) || 0}px`;
  const color = cvGroupColor(n.color);
  if (color) {
    box.style.borderColor = color;
    box.style.background = cvAlpha(color, 0.12);
  }
  // 縁だけ掴めるようにする（内側を押したら盤面の移動になる）
  const grip = document.createElement("div");
  grip.className = "cv-group-grip";
  box.appendChild(grip);
  // 見出しは枠の外側（左上）
  const label = document.createElement("div");
  label.className = "cv-group-label";
  label.textContent = String(n.label || "");
  if (color) label.style.color = color;
  box.appendChild(label);
  return box;
}

/** 札の要素。 */
function cvCardEl(n) {
  const box = document.createElement("div");
  box.className = "cv-card";
  box.setAttribute("data-cv-id", n.id);
  box.style.left = `${Number(n.x) || 0}px`;
  box.style.top = `${Number(n.y) || 0}px`;
  box.style.width = `${Number(n.width) || CV_CARD_W}px`;
  box.style.height = `${Number(n.height) || CV_CARD_H}px`;
  // kind が null や未知の値（DB に無い札）でも paper の灰色で描く
  const kind = cvCardKind(n.kind);
  const color = kind.color;
  box.style.borderLeftColor = color;
  if (!kind.key) box.classList.add("cv-unknown-kind");

  const head = document.createElement("div");
  head.className = "cv-head";
  const mark = document.createElement("span");
  mark.className = "cv-kind";
  mark.textContent = kind.label;
  mark.style.color = color;
  head.appendChild(mark);
  head.appendChild(el("span", "cv-sub", String(n.subtitle || "")));
  box.appendChild(head);

  box.appendChild(el("div", "cv-title", String(n.title || "(無題)")));
  if (n.tags) box.appendChild(el("div", "cv-tags", String(n.tags)));
  if (n.summary) box.appendChild(el("div", "cv-sum", String(n.summary)));
  return box;
}

/** 文字の要素（付箋の見た目）。 */
function cvTextEl(n) {
  const box = document.createElement("div");
  box.className = "cv-text";
  box.setAttribute("data-cv-id", n.id);
  box.style.left = `${Number(n.x) || 0}px`;
  box.style.top = `${Number(n.y) || 0}px`;
  box.style.width = `${Number(n.width) || CV_CARD_W}px`;
  box.style.height = `${Number(n.height) || CV_CARD_H}px`;
  box.textContent = String(n.text || "");
  return box;
}

/** 線を描き直す（選択・ドラッグ・読み直し・種類の切り替えの後）。 */
function cvRenderEdges() {
  const svg = cvSvg || document.getElementById("cv-edges");
  if (!svg) return;
  cvSvg = svg;
  while (svg.firstChild) svg.removeChild(svg.firstChild);

  const nodes = cvNodes();
  const byId = new Map(nodes.map((n) => [n.id, n]));

  // 線の外接（viewBox 用）
  let minX = Infinity, minY = Infinity, maxX = -Infinity, maxY = -Infinity;
  for (const n of nodes) {
    const x = Number(n.x) || 0, y = Number(n.y) || 0;
    const w = Number(n.width) || 0, h = Number(n.height) || 0;
    if (x < minX) minX = x;
    if (y < minY) minY = y;
    if (x + w > maxX) maxX = x + w;
    if (y + h > maxY) maxY = y + h;
  }
  if (!Number.isFinite(minX)) { minX = 0; minY = 0; maxX = 1; maxY = 1; }
  const pad = 60;
  minX -= pad; minY -= pad; maxX += pad; maxY += pad;
  svg.setAttribute("viewBox", `${cvR1(minX)} ${cvR1(minY)} ${cvR1(maxX - minX)} ${cvR1(maxY - minY)}`);
  svg.setAttribute("width", String(cvR1(maxX - minX)));
  svg.setAttribute("height", String(cvR1(maxY - minY)));
  svg.style.left = `${cvR1(minX)}px`;
  svg.style.top = `${cvR1(minY)}px`;

  // 矢印（種類ごとに 1 つ）
  const defs = document.createElementNS("http://www.w3.org/2000/svg", "defs");
  for (const [kind] of CV_EDGE_KINDS) {
    const marker = document.createElementNS("http://www.w3.org/2000/svg", "marker");
    marker.setAttribute("id", `cv-arrow-${kind}`);
    marker.setAttribute("viewBox", "0 0 10 10");
    marker.setAttribute("refX", "9");
    marker.setAttribute("refY", "5");
    marker.setAttribute("markerWidth", "6");
    marker.setAttribute("markerHeight", "6");
    marker.setAttribute("orient", "auto-start-reverse");
    const path = document.createElementNS("http://www.w3.org/2000/svg", "path");
    path.setAttribute("d", "M 0 0 L 10 5 L 0 10 z");
    path.setAttribute("fill", cvEdgeKindColor(kind));
    marker.appendChild(path);
    defs.appendChild(marker);
  }
  svg.appendChild(defs);

  for (const e of cvVisibleEdges()) {
    const a = byId.get(e.from);
    const b = byId.get(e.to);
    if (!a || !b) continue;
    // kind が未知の値でも描けるようにする（表に無い種類は link と同じ灰色）
    const kind = cvEdgeKind(e.kind);
    const color = kind.color;
    const opacity = cvEdgeOpacity(e, cvState.selected);
    const on = e.from === cvState.selected || e.to === cvState.selected;
    const path = document.createElementNS("http://www.w3.org/2000/svg", "path");
    path.setAttribute("d", cvEdgePath(a, b));
    path.setAttribute("fill", "none");
    path.setAttribute("stroke", color);
    path.setAttribute("stroke-width", on ? "2.5" : "1.6");
    // 縮小しても線が細くなって消えないよう、画面上の太さで描く（矢印も同じ大きさのまま）
    path.style.strokeWidth = `calc(${on ? 2.5 : 1.6}px / var(--cv-z, 1))`;
    path.setAttribute("stroke-opacity", String(opacity));
    if (kind.known) {
      path.setAttribute("marker-end", `url(#cv-arrow-${kind.key})`);
    } else {
      // 矢印の定義が無い種類は・印なしの線にする（壊れた参照を出さない）
      const dot = document.createElementNS("http://www.w3.org/2000/svg", "circle");
      dot.setAttribute("cx", String(cvR1(cvEdgePathEndX(a, b, kind))));
      dot.setAttribute("cy", String(cvR1(cvEdgePathEndY(a, b, kind))));
      dot.setAttribute("r", "2.5");
      dot.setAttribute("fill", color);
      dot.setAttribute("fill-opacity", String(opacity));
      svg.appendChild(dot);
    }
    if (e.kind === "link") path.setAttribute("stroke-dasharray", "6 4");
    svg.appendChild(path);

    // link だけ線の中点にラベルを出す
    if (e.kind === "link" && e.label) {
      const { p1, p2, c1, c2 } = cvEdgeControls(a, b);
      const mid = cvBezierMid(p1, c1, c2, p2);
      const label = document.createElementNS("http://www.w3.org/2000/svg", "text");
      label.setAttribute("x", String(cvR1(mid.x)));
      label.setAttribute("y", String(cvR1(mid.y)));
      label.setAttribute("text-anchor", "middle");
      label.setAttribute("dominant-baseline", "middle");
      label.setAttribute("font-size", "12");
      label.setAttribute("fill", color);
      label.setAttribute("fill-opacity", String(Math.max(opacity, 0.75)));
      label.textContent = String(e.label);
      svg.appendChild(label);
    }
  }
}

/** 見える範囲を #cv-world の transform にする。 */
function cvApplyView() {
  const world = document.getElementById("cv-world");
  if (!world) return;
  world.style.transform = `translate(${cvView.tx}px, ${cvView.ty}px) scale(${cvView.z})`;
  // 遠くのときは札を簡単に描く
  world.classList.toggle("cv-far", cvView.z < CV_FAR_Z);
  // 線の太さと遠くの題名を画面上で同じ大きさに保つため、CSS に拡大率を渡す
  world.style.setProperty("--cv-z", String(cvView.z));
  cvRenderZoom();
  cvSaveView();
}

/** 右下の − 100% + と「元に戻す」の使えるかどうか。 */
function cvRenderZoom() {
  const pct = document.getElementById("cv-zoom-pct");
  if (pct) pct.textContent = `${Math.round(cvView.z * 100)}%`;
  const undo = document.getElementById("cv-undo");
  if (undo) undo.disabled = !cvUndo;
  cvApplyPanelClass();
}

/** 見えている範囲を実測して全体が入るように合わせる（⤢ / 0 キー / 整列の後）。 */
function cvFit() {
  const size = cvStageSize();
  if (size.w <= 0 || size.h <= 0) return;
  cvView = cvFitView(cvNodes(), size.w, size.h);
  cvApplyView();
}

/** 盤面の中央を中心に 1.2 倍 / 1/1.2 倍（＋ / − キー、右下のボタン）。 */
function cvZoomCenter(factor) {
  const size = cvStageSize();
  if (size.w <= 0 || size.h <= 0) return;
  cvView = cvZoomAt(cvView, size.w / 2, size.h / 2, factor);
  cvApplyView();
}

// --- 赤い 1 行（保存の失敗など） ---

function cvSetError(text) {
  cvErrorText = text || "";
  cvRenderError();
}

function cvRenderError() {
  const box = document.getElementById("cv-error");
  if (!box) return;
  box.textContent = cvErrorText;
  box.hidden = !cvErrorText;
}

// --- 左右の欄（共有契約 C-F。中身は U07 の canvaspanel.js が入れる） ---

/** 見出しのボタンで開閉。状態は localStorage に覚える。 */
function cvTogglePanel(side) {
  const open = cvVisiblePanels();
  open[side] = !open[side];
  // 狭い画面では引き出しが重なるので、片方を開いたらもう片方を閉じる
  if (open[side] && window.innerWidth < CV_WIDE_PX) open[side === "left" ? "right" : "left"] = false;
  cvPanelsOpen = open;
  cvSaveJson(CV_PANELS_KEY, { left: !!open.left, right: !!open.right });
  cvApplyPanelClass();
}

/** 両方閉じる（狭い画面で暗い部分を押したとき）。 */
function cvClosePanels() {
  cvPanelsOpen = { left: false, right: false };
  cvSaveJson(CV_PANELS_KEY, { left: false, right: false });
  cvApplyPanelClass();
}

/** 開閉の class を section に付ける。狭い画面では暗い部分も出す。 */
function cvApplyPanelClass() {
  const sec = document.getElementById("view-canvas-board");
  if (!sec) return;
  const open = cvVisiblePanels();
  // 狭い画面で両方開いた状態（広い画面で覚えた状態など）は、左だけを開いた状態にそろえる
  if (window.innerWidth < CV_WIDE_PX && open.left && open.right) open.right = false;
  sec.classList.toggle("cv-left-open", !!open.left);
  sec.classList.toggle("cv-right-open", !!open.right);
  const scrim = document.getElementById("cv-scrim");
  if (scrim) scrim.hidden = !(window.innerWidth < CV_WIDE_PX && (open.left || open.right));
}

/** 画面幅が変わったときの出し入れ。 */
function cvOnResize() {
  cvApplyPanelClass();
}

// --- 線の種類の切り替え（道具行） ---

function cvRenderKindButtons() {
  const box = document.getElementById("cv-kinds");
  if (!box) return;
  box.innerHTML = "";
  const on = cvVisibleKinds();
  for (const [kind, label, color] of CV_EDGE_KINDS) {
    const b = document.createElement("button");
    b.textContent = label;
    b.dataset.kind = kind;
    b.classList.toggle("active", on[kind] !== false);
    b.style.setProperty("--cv-kind-color", color);
    b.addEventListener("click", () => {
      const next = cvVisibleKinds();
      next[kind] = next[kind] === false;
      cvKinds = next;
      cvSaveJson(CV_KINDS_KEY, { ...next });
      cvRenderKindButtons();
      cvRenderEdges();
    });
    box.appendChild(b);
  }
}

// --- 整列と元に戻す ---

/** 今の配置を「元に戻す用」に覚える。ids を渡せばそのノードだけ。 */
function cvSnapshot(ids) {
  const list = ids ? cvNodes().filter((n) => ids.indexOf(n.id) >= 0) : cvNodes();
  const out = {};
  for (const n of list) {
    out[n.id] = {
      x: Number(n.x) || 0,
      y: Number(n.y) || 0,
      width: Number(n.width) || 0,
      height: Number(n.height) || 0,
    };
  }
  return out;
}

/** snapshot の中身を今のノードへ書き戻す。 */
function cvApplySnapshot(snap) {
  for (const n of cvNodes()) {
    const s = snap && snap[n.id];
    if (!s) continue;
    n.x = s.x;
    n.y = s.y;
    if (s.width) n.width = s.width;
    if (s.height) n.height = s.height;
  }
}

/** 位置を PUT .../layout で保存する。失敗したら revert で元へ戻す。 */
async function cvSaveLayout(list, revert) {
  const nodes = (list || []).map((n) => {
    const out = { id: n.id, x: Math.round(Number(n.x) || 0), y: Math.round(Number(n.y) || 0) };
    if (Number(n.width)) out.width = Math.round(Number(n.width));
    if (Number(n.height)) out.height = Math.round(Number(n.height));
    return out;
  });
  try {
    const res = await apiFetch(cvApi("/layout"), {
      method: "PUT",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ nodes }),
    });
    if (!res.ok) throw new Error(await resDetail(res, "位置を保存できませんでした"));
    window.SaiShell.ok();
    cvSetError("");
    return true;
  } catch (e) {
    window.SaiShell.fail();
    cvSetError((e && e.message) || "位置を保存できませんでした");
    if (typeof revert === "function") {
      revert();
      cvRender();
    }
    return false;
  }
}

/** 「整列」。cvlLayoutBoard（U05）を使い、結果を 1 回の PUT で保存する。 */
async function cvDoLayout() {
  if (!cvState.data) return;
  const before = cvSnapshot();
  let laid;
  try {
    laid = cvlLayoutBoard(cvState.data.nodes, cvVisibleEdges());
  } catch (e) {
    cvSetError((e && e.message) || "整列できませんでした");
    return;
  }
  cvUndo = before;
  const byId = new Map(cvNodes().map((n) => [n.id, n]));
  // 隠している札は保存しない（描いていないので戻せない）
  const laidVisible = (laid || []).filter((l) => byId.has(l.id));
  for (const l of laidVisible) {
    const n = byId.get(l.id);
    n.x = l.x;
    n.y = l.y;
    if (l.width) n.width = l.width;
    if (l.height) n.height = l.height;
  }
  cvRender();
  const ok = await cvSaveLayout(laidVisible, () => cvApplySnapshot(before));
  if (!ok) {
    cvUndo = null;
    cvRenderZoom();
    return;
  }
  await cvReload();
  cvFit();
}

/** 「元に戻す」。覚えた配置を保存し、覚えた配置を捨てる。 */
async function cvDoUndo() {
  const snap = cvUndo;
  if (!snap) return;
  const now = cvSnapshot(Object.keys(snap));
  cvUndo = null;
  cvApplySnapshot(snap);
  cvRender();
  const list = cvNodes().filter((n) => snap[n.id]);
  const ok = await cvSaveLayout(list, () => cvApplySnapshot(now));
  cvRenderZoom();
  if (!ok) return;
  await cvReload();
}

// --- 操作（Pointer Events。マウスと指を同じに扱う） ---

/** 2 つのノードが線でつながっているか（つながっていない札を薄くする）。 */
function cvLinkedTo(a, b) {
  if (!a || !b) return false;
  if (a === b) return true;
  return cvVisibleEdges().some((e) => (e.from === a && e.to === b) || (e.from === b && e.to === a));
}

/** 押した要素が属するノード（data-cv-id を持つ最も近い要素）。 */
function cvHitNode(target) {
  if (!target || typeof target.closest !== "function") return null;
  const box = target.closest("[data-cv-id]");
  if (!box) return null;
  return {
    id: box.getAttribute("data-cv-id"),
    group: box.getAttribute("data-cv-group") === "1",
  };
}

/** 動かすノードの id。枠なら中の札も一緒に動かす。 */
function cvMoveIds(id) {
  const n = cvNodeById(id);
  if (!n) return [];
  if (n.type !== "group") return [id];
  const ids = [id];
  for (const m of cvNodes()) {
    if (m.group === id) ids.push(m.id);
  }
  return ids;
}

function cvOnPointerDown(e) {
  const stage = document.getElementById("cv-stage");
  if (!stage) return;
  try { stage.setPointerCapture(e.pointerId); } catch { /* 捕ま Ã なくてもよい */ }
  cvPtrs.set(e.pointerId, { x: e.clientX, y: e.clientY });

  // 2 本になった -> ピンチ
  if (cvPtrs.size === 2) {
    const [p, q] = [...cvPtrs.values()];
    const rect = stage.getBoundingClientRect();
    const z0 = cvView.z;
    cvPinch = {
      dist: Math.hypot(q.x - p.x, q.y - p.y) || 1,
      z0,
      wx: ((p.x + q.x) / 2 - rect.left - cvView.tx) / z0,
      wy: ((p.y + q.y) / 2 - rect.top - cvView.ty) / z0,
    };
    cvDown = null;
    return;
  }
  if (cvPtrs.size > 2) return;

  const hit = cvHitNode(e.target);
  const ids = hit ? cvMoveIds(hit.id) : [];
  cvDown = {
    id: e.pointerId,
    sx: e.clientX,
    sy: e.clientY,
    view: { ...cvView },
    nodeId: hit ? hit.id : null,
    ids,
    mode: null, // "pan" | "node" | "group"。決まるまで null
    orig: ids.length ? cvSnapshot(ids) : null,
  };
}

function cvOnPointerMove(e) {
  if (!cvPtrs.has(e.pointerId)) return;
  cvPtrs.set(e.pointerId, { x: e.clientX, y: e.clientY });

  // ピンチ（2 本）
  if (cvPtrs.size >= 2 && cvPinch) {
    const [p, q] = [...cvPtrs.values()];
    const dist = Math.hypot(q.x - p.x, q.y - p.y) || 1;
    const stage = document.getElementById("cv-stage");
    const rect = stage ? stage.getBoundingClientRect() : { left: 0, top: 0 };
    let z = cvPinch.z0 * (dist / cvPinch.dist);
    z = Math.max(CV_ZOOM_MIN, Math.min(CV_ZOOM_MAX, z));
    cvView = {
      tx: (p.x + q.x) / 2 - rect.left - cvPinch.wx * z,
      ty: (p.y + q.y) / 2 - rect.top - cvPinch.wy * z,
      z,
    };
    cvApplyView();
    return;
  }

  const d = cvDown;
  if (!d || d.id !== e.pointerId) return;
  const dx = e.clientX - d.sx;
  const dy = e.clientY - d.sy;

  if (!d.mode) {
    if (Math.hypot(dx, dy) < CV_DRAG_SLOP) return; // クリックかドラッグか決まらない
    if (!d.orig) d.mode = "pan";
    else {
      const n = cvNodeById(d.nodeId);
      d.mode = n && n.type === "group" ? "group" : "node";
    }
  }

  if (d.mode === "pan") {
    // 見える範囲だけ動かす（札は盤面の座標に固定したまま）
    cvView = { tx: d.view.tx + dx, ty: d.view.ty + dy, z: d.view.z };
    cvApplyView();
    return;
  }

  // 札（または枠と中の札）を動かす。ワールドの座標に直す
  const wx = dx / d.view.z;
  const wy = dy / d.view.z;
  for (const id of d.ids) {
    const n = cvNodeById(id);
    const o = d.orig[id];
    if (!n || !o) continue;
    n.x = o.x + wx;
    n.y = o.y + wy;
    const node = cvNodeEls.get(id);
    if (node) {
      node.style.left = `${n.x}px`;
      node.style.top = `${n.y}px`;
    }
  }
  cvRenderEdges();
}

function cvOnPointerUp(e) {
  cvPtrs.delete(e.pointerId);
  if (cvPtrs.size < 2) cvPinch = null;
  const stage = document.getElementById("cv-stage");
  if (stage) {
    try { stage.releasePointerCapture(e.pointerId); } catch { /* 解放できなくてもよい */ }
  }
  const d = cvDown;
  if (!d || d.id !== e.pointerId) {
    if (cvPtrs.size === 0) cvDown = null;
    return;
  }
  cvDown = null;

  // 途中で指が離れた（pointercancel）はクリックにも保存にもしない
  if (e.type === "pointercancel") return;

  // 動かしていない（4px 未満）-> クリック
  if (!d.mode) {
    cvSelect(d.nodeId);
    return;
  }
  if (d.mode === "pan" || !d.orig) return;

  // ドラッグした -> 1 回の PUT でその一部を保存する
  const list = cvNodes().filter((n) => d.ids.indexOf(n.id) >= 0);
  if (list.length === 0) return;
  const snap = d.orig;
  cvUndo = snap; // 直前の 1 回だけ元に戻せるようにする
  cvRenderZoom();
  void cvSaveLayout(list, () => cvApplySnapshot(snap)).then((ok) => {
    if (!ok) {
      cvUndo = null;
      cvRenderZoom();
    }
  });
}

/** ホイール（縦・横・Ctrl で拡大）。 */
function cvOnWheel(e) {
  const stage = document.getElementById("cv-stage");
  if (!stage) return;
  e.preventDefault(); // ページ全体が動かないように
  const rect = stage.getBoundingClientRect();
  const px = e.clientX - rect.left;
  const py = e.clientY - rect.top;
  if (e.ctrlKey || e.metaKey) {
    // 指の位置を中心に拡大・縮小
    cvView = cvZoomAt(cvView, px, py, Math.exp(-e.deltaY * 0.01));
  } else {
    // 縦（deltaX と Shift+ホイールは横）
    cvView = {
      tx: cvView.tx - (e.shiftKey ? e.deltaY : (e.deltaX || 0)),
      // 下へ回すと下の札が見える（盤面は上へ動く）。Shift 中は縦に動かさない
      ty: cvView.ty - (e.shiftKey ? 0 : e.deltaY),
      z: cvView.z,
    };
  }
  cvApplyView();
}

/** キー。盤面の画面が出ていて入力欄に打っていないときだけ受ける。 */
function cvOnKeyDown(e) {
  const sec = document.getElementById("view-canvas-board");
  if (!sec || sec.hidden) return;
  const t = e.target;
  const tag = t && t.tagName ? t.tagName.toUpperCase() : "";
  if (tag === "INPUT" || tag === "TEXTAREA" || tag === "SELECT") return;
  if (t && t.isContentEditable) return;
  if (e.metaKey || e.ctrlKey) return;

  const pan = (dx, dy) => {
    cvView = { tx: cvView.tx + dx, ty: cvView.ty + dy, z: cvView.z };
    cvApplyView();
  };
  switch (e.key) {
    case "+": case "=": cvZoomCenter(1.2); break;
    case "-": case "_": cvZoomCenter(1 / 1.2); break;
    case "0": cvFit(); break;
    case "ArrowUp": pan(0, CV_KEY_PAN); break;
    case "ArrowDown": pan(0, -CV_KEY_PAN); break;
    case "ArrowLeft": pan(CV_KEY_PAN, 0); break;
    case "ArrowRight": pan(-CV_KEY_PAN, 0); break;
    case "Escape": cvSelect(null); break;
    default: return;
  }
  e.preventDefault();
}

// ============================================================================
// 画面の登録（window.SaiRoutes・app.js §3.2-2）
// ============================================================================

if (typeof window !== "undefined") {
  (window.SaiRoutes = window.SaiRoutes || []).push(
    {
      // Canvas 一覧（#/papers/canvas）
      match(hash) {
        return hash === "#/papers/canvas" ? {} : null;
      },
      view: "view-canvas-list",
      tab: "papers",
      hideTabbar: false,
      interval: 0, // 定期更新はしない（↻ で読み直す）
      async open() {
        await cvOpenList();
      },
      async poll() {},
    },
    {
      // 盤面（#/canvas/<プロジェクト>/<Canvas 名>）
      match(hash) {
        return cvParseBoardHash(hash);
      },
      view: "view-canvas-board",
      tab: "papers",
      hideTabbar: true, // 盤面を広く使うため
      interval: 0,
      async open(params) {
        await cvOpenBoard(params);
      },
      async poll() {},
    }
  );

  // --- 一覧側のボタン ---
  const listReload = document.getElementById("canvas-list-reload");
  if (listReload) listReload.addEventListener("click", () => { void cvOpenList({ force: true }); });
  const listAdd = document.getElementById("canvas-new");
  if (listAdd) listAdd.addEventListener("click", () => { void cvCreateCanvas(); });
  // プロジェクト名のボタン（canvas-project）は papers.js 側で openProjectSheet を登録している

  // --- 盤面側のボタン ---
  const boardBack = document.getElementById("cv-back");
  if (boardBack) boardBack.addEventListener("click", () => { location.hash = "#/papers/canvas"; });
  const boardReload = document.getElementById("cv-reload");
  if (boardReload) boardReload.addEventListener("click", () => { void cvReload(); });
  const leftToggle = document.getElementById("cv-left-toggle");
  if (leftToggle) leftToggle.addEventListener("click", () => { cvTogglePanel("left"); });
  const rightToggle = document.getElementById("cv-right-toggle");
  if (rightToggle) rightToggle.addEventListener("click", () => { cvTogglePanel("right"); });
  const scrim = document.getElementById("cv-scrim");
  if (scrim) scrim.addEventListener("click", () => { cvClosePanels(); });
  const zoomIn = document.getElementById("cv-zoom-in");
  if (zoomIn) zoomIn.addEventListener("click", () => { cvZoomCenter(1.2); });
  const zoomOut = document.getElementById("cv-zoom-out");
  if (zoomOut) zoomOut.addEventListener("click", () => { cvZoomCenter(1 / 1.2); });
  const zoomFit = document.getElementById("cv-zoom-fit");
  if (zoomFit) zoomFit.addEventListener("click", () => { cvFit(); });
  const layoutBtn = document.getElementById("cv-layout");
  if (layoutBtn) layoutBtn.addEventListener("click", () => { void cvDoLayout(); });
  const undoBtn = document.getElementById("cv-undo");
  if (undoBtn) undoBtn.addEventListener("click", () => { void cvDoUndo(); });

  // 盤面の操作
  const stage = document.getElementById("cv-stage");
  if (stage) {
    stage.addEventListener("pointerdown", (e) => cvOnPointerDown(e));
    stage.addEventListener("pointermove", (e) => cvOnPointerMove(e));
    stage.addEventListener("pointerup", (e) => cvOnPointerUp(e));
    stage.addEventListener("pointercancel", (e) => cvOnPointerUp(e));
    stage.addEventListener("wheel", (e) => cvOnWheel(e), { passive: false });
  }
  window.addEventListener("keydown", cvOnKeyDown);
  window.addEventListener("resize", cvOnResize);
}

// --- node からの読み込み（tests/test_canvas.js） ---
if (typeof module !== "undefined" && module.exports) {
  module.exports = {
    cvBoardHash,
    cvParseBoardHash,
    cvFitView,
    cvZoomAt,
    cvClosestSides,
    cvEdgePath,
    cvEdgeOpacity,
    cvGroupColor,
    cvCardKind,
    cvEdgeKind,
    cvEdgeKindColor,
    cvAlpha,
    CV_EDGE_KINDS,
    CV_CARD_KINDS,
    CV_GROUP_COLORS,
    CV_ZOOM_MIN,
    CV_ZOOM_MAX,
  };
}
