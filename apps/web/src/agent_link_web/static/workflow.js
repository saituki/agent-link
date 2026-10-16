"use strict";
// agent-link-web の Workflow 盤面（実装計画 U5・住所 #/project/<name>/workflow）。
//
// agent-link-tui の Workflow（実行の DAG）と Kanban（todo / running / review の
// 3 レーン）を、図の盤面と同じ操作で見て、その場で操作できるようにしたもの。
//   - 見える範囲（tx, ty, z）だけを動かし、札は agent-link が決めた場所に固定する
//   - 盤面は overflow: hidden。札も線も帯や見出しにはみ出さない
//   - 札のドラッグでずらした量だけこの端末（localStorage）に覚える
//     （agent-link の配置は変えない。ユーザーが「ドラッグは端末に覚える」と決めたため）
//   - 線は全部出し、選んだ札につながる線だけを濃くする
//   - 1 本指 / マウスで移動、2 本指・ctrl + ホイールで拡大、ホイールで縦横移動
//
// 依存は足さない。札は HTML の div、線は同じ中の SVG。
// canvaslayout.js の純関数（cvFitView / cvZoomAt）を全体表示と拡大に使わせてもらう。
// DOM を使わない純関数は末尾で node に渡す（tests/test_workflow.js）。
// 一番外側の名前は wf / WF_ で始める（C-F6）。

// --- 定数 ---

// TUI の文字マスを px に直す（端末の文字は縦長なので横 1 マス = 縦 2 マス）
const WF_CELL_W = 10;
const WF_CELL_H = 20;
// 10 秒ごとに読み直す
const WF_INTERVAL_MS = 10000;
// 拡大率がこの未満のときは題だけ大きく描く（遠くから見ても読めるように）
const WF_FAR_Z = 0.6;
// 題の画面上の大きさ（px）。拡大率に逆比例させて、どれだけ縮んでもこの大きさで読めるようにする
const WF_TITLE_PX = 14;
// 題の世界上の大きさの上限（px）。これ以上広げると札の高さ（h=6 → 120px）からはみ出す
const WF_TITLE_MAX_PX = 88;
// 線の太さ（画面上の px）。世界上の値は拡大率で割るので、ここは画面上の太さ
const WF_EDGE_W_PX = 1.6;
const WF_EDGE_W_SEL_PX = 2.5;
// 線の世界上の太さの上限（px）
const WF_EDGE_W_MAX_PX = 12;
// 区切り線を盤面の右端からさらに伸ばす分（世界上の px）
const WF_SEP_PAD_PX = 40;
// 選んだ札につながらない線を薄くする濃さ。これでも線として 3:1 以上を保つ
const WF_EDGE_DIM = 0.8;
// ドラッグとクリックの境（これ以上動いたらドラッグ、未満ならクリック）
const WF_DRAG_SLOP = 4;
// 矢印キーで見える範囲を動かす量（画面上の px）
const WF_KEY_PAN = 80;
// 札の中身の 1 行目の高さの見積り（px・説明の行を何行積めるかの計算に使う）
const WF_CARD_HEAD_PX = 34;
// 札の中身の 1 行の高さ（px）
const WF_CARD_LINE_PX = 14;
// 札の説明の 1 文字の幅の見積り（px・全角は 2 文字分として数える）
const WF_CARD_CHAR_W = 6.6;

// 覚える場所（sessionStorage / localStorage）
const WF_VIEW_KEY = "agent_link_web_wf_view";        // 見える範囲（sessionStorage）
const WF_OFFSETS_KEY = "agent_link_web_wf_offsets";  // ドラッグでずらした量（localStorage）
const WF_KANBAN_KEY = "agent_link_web_wf_kanban";    // Kanban の帯の開閉（localStorage）
const WF_KINDS_KEY = "agent_link_web_wf_kinds";      // 線の種類の出し入れ（localStorage）
const WF_SUBTASKS_KEY = "agent_link_web_wf_subtasks"; // 子の表示（localStorage・"1" で表示）

// 選んだ札の枠の色（TUI と同じ黄色）
const WF_SEL_COLOR = "var(--sel)";

// 線の種類（[kind, 表示名, 色, 破線か]）。
const WF_EDGE_KINDS = [
  ["delegation", "Delegation", "var(--edge-delegation)", false],
  ["waggle_token", "Document", "var(--edge-waggle)", false],
];
// 表に無い種類の灰色
const WF_EDGE_UNKNOWN = "var(--text-soft)";

// Kanban の 3 レーン（TUI と同じ並び）
const WF_LANES = ["todo", "running", "review"];

// --- 状態 ---

let wfName = "";        // 開いているプロジェクト名
let wfData = null;      // GET /api/projects/<name>/workflow の応答
let wfRaw = "";         // 前回の応答の JSON（同じなら描き直さない）
let wfError = "";       // 道具の行の下に出す赤い 1 行
let wfBusy = false;     // 操作の途中（ボタンを二重に押さない）
let wfView = { tx: 0, ty: 0, z: 1 };   // 見える範囲（画面の点 = ワールドの点 * z + (tx, ty)）
let wfSelected = null;  // 選んでいるノード id
let wfOffsets = {};     // ドラッグでずらした量（{ノード id: [dx, dy]}）
let wfKinds = null;     // 線の種類の出し入れ
let wfSubtasks = false; // 子（task create で作った run）を出すか
let wfKanban = null;    // Kanban の帯の開閉
let wfPtrs = new Map(); // 押しているポインタ（1 本 = 移動、2 本 = ピンチ）
let wfDown = null;      // 1 本指で押したままの情報
let wfPinch = null;     // 2 本指の情報
let wfSvg = null;       // 線を描く SVG
let wfCardEls = new Map(); // ノード id -> 描いた要素（ドラッグで直接動かすため）
let wfSheetHandle = null; // 開いている札の中身の板（操作が終わったら閉じる）

// ============================================================================
// DOM を使わない部品（node のテストから読む）
// ============================================================================

/** Workflow の住所。 */
function wfHash(name) {
  return `#/project/${encodeURIComponent(name || "")}/workflow`;
}

/** Workflow の住所からプロジェクト名を取り出す。別の画面なら null。 */
function wfParseHash(hash) {
  const m = /^#\/project\/([^/]+)\/workflow\/?$/.exec(String(hash || ""));
  if (!m) return null;
  try {
    return { name: decodeURIComponent(m[1]) };
  } catch {
    return null;
  }
}

/** ノードの大きさを px の矩形にする（文字マス × WF_CELL_* ＋ 覚えたずらし量）。 */
function wfNodeRect(node, offsets) {
  const n = node || {};
  const off = (offsets && offsets[n.id]) || null;
  const dx = Array.isArray(off) ? Number(off[0]) || 0 : 0;
  const dy = Array.isArray(off) ? Number(off[1]) || 0 : 0;
  return {
    x: (Number(n.x) || 0) * WF_CELL_W + dx,
    y: (Number(n.y) || 0) * WF_CELL_H + dy,
    width: Math.max(WF_CELL_W, (Number(n.w) || 0) * WF_CELL_W),
    height: Math.max(WF_CELL_H, (Number(n.h) || 0) * WF_CELL_H),
  };
}

/** 描いている全部のノードの矩形（全体表示の計算に使う）。 */
function wfBoardRects(nodes, offsets) {
  return (Array.isArray(nodes) ? nodes : [])
    .filter((n) => n && n.id != null)
    .map((n) => wfNodeRect(n, offsets));
}

/**
 * 全部の矩形が入る見える範囲を返す（canvaslayout.js の cvFitView にそのまま渡す形）。
 * node のテストでは canvaslayout.js を読み込んで cvFitView をグローバルに入れてある
 * （tests/test_workflow.js）。入れ忘れても盤面が止まらないよう{z:1} を返す。
 */
function wfFitView(rects, width, height) {
  if (typeof cvFitView !== "function") return { tx: 0, ty: 0, z: 1 };
  return cvFitView(rects || [], width, height);
}

/**
 * 全体表示の計算に入れる矩形。札の外側に round の区切り線も足す
 * （区切り線とラベルは札の外にあるので、入れないと上端で切れる）。
 * レイアウトの計算はしない。agent-link が返した y を px に直しただけのもの。
 */
function wfFitRects(nodes, offsets, seps) {
  const rects = wfBoardRects(nodes, offsets);
  for (const s of Array.isArray(seps) ? seps : []) {
    const line = wfSeparatorLine(s);
    if (line) rects.push({ x: 0, y: line.y, width: 0, height: 0 });
  }
  return rects;
}

/** 盤面の右端（round の区切り線を端まで引くために使う）。 */
function wfBoardRight(rects) {
  let right = 0;
  for (const r of Array.isArray(rects) ? rects : []) {
    if (!r) continue;
    const x = (Number(r.x) || 0) + (Number(r.width) || 0);
    if (x > right) right = x;
  }
  return right;
}

/** (px, py) の下のワールドの点が動かないように拡大率を変える（canvaslayout.js の cvZoomAt）。 */
function wfZoomAt(view, px, py, factor) {
  if (typeof cvZoomAt !== "function") return { tx: 0, ty: 0, z: 1 };
  return cvZoomAt(view, px, py, factor);
}

/** 札の題（ID<run_id>/<name>）。 */
function wfCardTitle(node) {
  const n = node || {};
  const name = String(n.name == null ? "" : n.name);
  if (n.run_id == null) return name;
  return `ID${n.run_id}/${name}`;
}

/** DB の日時（タイムゾーン無しは UTC とみなす・project.js の parseStamp と同じ）を ms に。 */
function wfStamp(value) {
  if (!value) return null;
  let s = String(value).trim();
  if (!s) return null;
  const m = s.match(/^(\d{4}-\d{2}-\d{2})[ T](\d{2}:\d{2}(?::\d{2})?(?:\.\d+)?)$/);
  if (m) s = `${m[1]}T${m[2]}Z`;
  else if (!/[zZ]|[+-]\d{2}:?\d{2}$/.test(s)) s = `${s}Z`;
  const t = Date.parse(s);
  return Number.isNaN(t) ? null : t;
}

/** 札の 2 行目に出す時刻（端末のローカル時刻で "MM/DD HH:MM:SS"）。 */
function wfStampText(value) {
  const t = wfStamp(value);
  if (t == null) return "";
  const d = new Date(t);
  const p2 = (v) => String(v).padStart(2, "0");
  return `${p2(d.getMonth() + 1)}/${p2(d.getDate())} ${p2(d.getHours())}:${p2(d.getMinutes())}:${p2(d.getSeconds())}`;
}

/** 文字列を「全角は 2 文字分として数えた幅」で折り返す（説明の行に分ける）。 */
function wfWrapText(text, maxUnits) {
  const limit = Math.max(4, Math.floor(maxUnits));
  const lines = [];
  for (const raw of String(text == null ? "" : text).split(/\r?\n/)) {
    let line = "";
    let units = 0;
    for (const ch of raw) {
      const u = /[⺀-鿿＀-￯　-〿]/.test(ch) ? 2 : 1;
      if (units + u > limit) {
        lines.push(line);
        line = "";
        units = 0;
      }
      line += ch;
      units += u;
    }
    lines.push(line);
  }
  return lines;
}

/** 札に出す印（running / failed の点と、チップ）。 */
function wfCardMarks(node) {
  const n = node || {};
  const chips = [];
  if (n.orphaned === true) chips.push({ text: "要確認", cls: "warn" });
  return {
    running: n.status === "running",
    failed: n.status === "failed",
    chips,
  };
}

/** 札の `<model> <effort>` の行。どちらも無ければ空文字（その行を出さない）。 */
function wfCardModel(node) {
  const n = node || {};
  return [n.model, n.thinking]
    .filter((v) => v != null && String(v).trim() !== "")
    .map((v) => String(v).trim())
    .join(" ");
}

/** 札の中に出す行（題・model・時刻・費用・説明の行）。高さに収まる分だけ説明を出す。 */
function wfCardLines(node) {
  const n = node || {};
  const rect = wfNodeRect(n, null);
  const model = wfCardModel(n);
  const used = WF_CARD_HEAD_PX + (model ? WF_CARD_LINE_PX : 0);
  const room = Math.max(0, Math.floor((rect.height - used) / WF_CARD_LINE_PX));
  const maxUnits = Math.floor((rect.width - 12) / WF_CARD_CHAR_W);
  return {
    title: wfCardTitle(n),
    model,
    time: wfStampText(n.created_at),
    cost: n.cost == null || n.cost === "" ? "" : String(n.cost),
    desc: wfWrapText(n.description, maxUnits).slice(0, room),
    marks: wfCardMarks(n),
  };
}

/** 線の種類の色。表に無い種類（null / 未知の値）も灰色で返す。 */
function wfEdgeColor(kind) {
  const key = kind == null ? "" : String(kind);
  const row = WF_EDGE_KINDS.find((k) => k[0] === key);
  return row ? row[2] : WF_EDGE_UNKNOWN;
}

/** 線が破線かどうか。破線の種類は無いので常に偽。 */
function wfEdgeDashed() {
  return false;
}

// 小数 1 位に丸める（path の文字列を短くする）
function wfR1(n) { return Math.round(n * 10) / 10; }

/** 辺の中点（side は right / left / top / bottom）。 */
function wfSidePoint(r, side) {
  const x = Number(r && r.x) || 0;
  const y = Number(r && r.y) || 0;
  const w = Number(r && r.width) || 0;
  const h = Number(r && r.height) || 0;
  if (side === "left") return { x, y: y + h / 2 };
  if (side === "top") return { x: x + w / 2, y };
  if (side === "bottom") return { x: x + w / 2, y: y + h };
  return { x: x + w, y: y + h / 2 }; // right
}

/** 辺から外へ出る向き（制御点に使う）。 */
const WF_SIDE_NORMAL = { left: [-1, 0], right: [1, 0], top: [0, -1], bottom: [0, 1] };

/**
 * 2 つの矩形を結ぶ 3 次ベジェ曲線の d を返す。
 * side === true  → 左右の辺の中点どうし（from が左なら from の右辺 → to の左辺）
 * side === false → 上下の辺の中点どうし（from が上であれば from の下辺 → to の上辺）
 * 制御点は向きに合わせて 40px ずつ外へ出す。
 */
function wfEdgePath(a, b, side) {
  const ra = a || {};
  const rb = b || {};
  const horizontal = side === true;
  const aFirst = horizontal
    ? (Number(ra.x) || 0) + (Number(ra.width) || 0) / 2 <= (Number(rb.x) || 0) + (Number(rb.width) || 0) / 2
    : (Number(ra.y) || 0) + (Number(ra.height) || 0) / 2 <= (Number(rb.y) || 0) + (Number(rb.height) || 0) / 2;
  const s1 = horizontal ? (aFirst ? "right" : "left") : (aFirst ? "bottom" : "top");
  const s2 = horizontal ? (aFirst ? "left" : "right") : (aFirst ? "top" : "bottom");
  const p1 = wfSidePoint(ra, s1);
  const p2 = wfSidePoint(rb, s2);
  const n1 = WF_SIDE_NORMAL[s1];
  const n2 = WF_SIDE_NORMAL[s2];
  const c1 = { x: p1.x + n1[0] * 40, y: p1.y + n1[1] * 40 };
  const c2 = { x: p2.x + n2[0] * 40, y: p2.y + n2[1] * 40 };
  return `M ${wfR1(p1.x)} ${wfR1(p1.y)} C ${wfR1(c1.x)} ${wfR1(c1.y)}, ${wfR1(c2.x)} ${wfR1(c2.y)}, ${wfR1(p2.x)} ${wfR1(p2.y)}`;
}

/** 線の終点の x（矢印の印を置く場所）。 */
function wfEdgeEndX(a, b, side) {
  const ra = a || {};
  const rb = b || {};
  const horizontal = side === true;
  const aFirst = horizontal
    ? (Number(ra.x) || 0) + (Number(ra.width) || 0) / 2 <= (Number(rb.x) || 0) + (Number(rb.width) || 0) / 2
    : (Number(ra.y) || 0) + (Number(ra.height) || 0) / 2 <= (Number(rb.y) || 0) + (Number(rb.height) || 0) / 2;
  return horizontal ? (aFirst ? (Number(rb.x) || 0) : (Number(rb.x) || 0) + (Number(rb.width) || 0))
    : (Number(rb.x) || 0) + (Number(rb.width) || 0) / 2;
}

/** 線の終点の y（矢印の印を置く場所）。 */
function wfEdgeEndY(a, b, side) {
  const ra = a || {};
  const rb = b || {};
  const horizontal = side === true;
  const aFirst = horizontal
    ? (Number(ra.x) || 0) + (Number(ra.width) || 0) / 2 <= (Number(rb.x) || 0) + (Number(rb.width) || 0) / 2
    : (Number(ra.y) || 0) + (Number(ra.height) || 0) / 2 <= (Number(rb.y) || 0) + (Number(rb.height) || 0) / 2;
  if (horizontal) return (Number(rb.y) || 0) + (Number(rb.height) || 0) / 2;
  return aFirst ? (Number(rb.y) || 0) : (Number(rb.y) || 0) + (Number(rb.height) || 0);
}

/**
 * 線の濃さ。選んでいなければ全部 1、選んだ札につながる線は 1、
 * それ以外の線だけを WF_EDGE_DIM まで薄くする。
 * 薄くしても線として読めるように 3:1 以上を保つ（apps/web/tests/test_contrast.js が検査する）。
 */
function wfEdgeOpacity(edge, selected) {
  if (!selected) return 1;
  if (!edge) return WF_EDGE_DIM;
  if (edge.from === selected || edge.to === selected) return 1;
  return WF_EDGE_DIM;
}

/** 2 つのノードが線でつながっているか（つながっていない札を薄くする）。 */
function wfLinkedTo(edges, a, b) {
  if (a == null || b == null) return false;
  if (a === b) return true;
  return (Array.isArray(edges) ? edges : []).some(
    (e) => e && ((e.from === a && e.to === b) || (e.from === b && e.to === a))
  );
}

/** round の区切り線（y は文字マスなので px に直す）。 */
function wfSeparatorLine(sep) {
  const s = sep || {};
  return { y: (Number(s.y) || 0) * WF_CELL_H, label: String(s.label == null ? "" : s.label) };
}

/** Kanban の 3 レーンを（必ず 3 つ・空は空配列で）並べる。 */
function wfLanes(data) {
  const lanes = (data && data.lanes) || {};
  return WF_LANES.map((key) => ({
    key,
    cards: (Array.isArray(lanes[key]) ? lanes[key] : []).filter((c) => c && c.run_id != null),
  }));
}

/** Kanban の札が 1 つも無いか（「Kanban に札はありません」の出し入れに使う）。 */
function wfLanesEmpty(data) {
  return wfLanes(data).every((l) => l.cards.length === 0);
}

/** ある札の状態の表示（レーンか status）。 */
function wfStateOf(card) {
  const c = card || {};
  if (c.lane) return String(c.lane);
  return String(c.status == null ? "" : c.status);
}

/** 状態の日本語（札の題の横に出す）。 */
function wfStateLabel(state) {
  const key = String(state == null ? "" : state);
  if (WF_LANES.indexOf(key) >= 0) return key;
  if (key === "completed") return "完了";
  if (key === "running") return "実行中";
  if (key === "failed") return "失敗";
  if (key === "cancelled") return "取り消し";
  if (key === "orphaned") return "要確認";
  return key;
}

/**
 * 札（中身）の板に出す操作ボタン。
 * card は Workflow のノードでも Kanban の札でもよい（run_id / status / parent_run_id だけ使う）。
 * lane は "todo" / "running" / "review" / null。
 * project が分かるときの操作の住所まで作る。
 */
function wfActions(card, lane, project) {
  const c = card || {};
  const runId = c.run_id;
  if (runId == null) return [];
  const p = encodeURIComponent(project == null ? "" : project);
  const base = `/api/projects/${p}/runs/${encodeURIComponent(runId)}`;
  const out = [];
  if (lane === "todo") {
    out.push({ key: "start", label: "▶ Run", method: "POST", path: `${base}/start` });
  } else if (lane === "review") {
    out.push({ key: "advance", label: "→ Workflow", method: "POST", path: `${base}/advance`, to: "workflow" });
  }
  // running で親が無いもの（メインの実行）だけ ■ 終了を出す
  // （Kanban の札には status が無いのでレーンで判断する）
  const running = c.status === "running" || lane === "running";
  if (running && c.parent_run_id == null) {
    out.push({
      key: "finalize",
      label: "■ 終了",
      method: "POST",
      path: `/api/runs/${p}/${encodeURIComponent(runId)}/finalize`,
      danger: true,
      confirm: "この実行を完了にしますか？ 実行中のエージェントは止まります。",
    });
  }
  return out;
}

/** 凡例（線の色・印の意味）。? の小窓に出す。 */
function wfLegend() {
  return {
    edges: WF_EDGE_KINDS.map(([key, label, color]) => ({ key, label, color })),
    marks: [
      { text: "実行中（青い点・点滅）", cls: "dot-running" },
      { text: "失敗（赤い点）", cls: "dot-failed" },
      { text: "要確認（running の取り残し）", cls: "warn" },
    ],
  };
}

// ============================================================================
// 記憶（プライベートブラウズでも動くように必ず失敗に備える）
// ============================================================================

/** localStorage / sessionStorage から JSON を読む。無ければ fallback。 */
function wfLoadJson(store, key, fallback) {
  try {
    const raw = store.getItem(key);
    if (!raw) return fallback;
    const obj = JSON.parse(raw);
    return obj && typeof obj === "object" ? obj : fallback;
  } catch {
    return fallback;
  }
}

/** localStorage / sessionStorage へ JSON を書く。書けなくても動作は続ける。 */
function wfSaveJson(store, key, value) {
  try { store.setItem(key, JSON.stringify(value)); } catch { /* 動けばよい */ }
}

/** localStorage / sessionStorage があれば使う（node のテストからは無いので null が返る）。 */
function wfStore(which) {
  const name = which === "session" ? "sessionStorage" : "localStorage";
  try {
    const g = typeof globalThis !== "undefined" ? globalThis : null;
    if (g && g[name]) return g[name];
    if (typeof window === "undefined") return null;
    return window[name] || null;
  } catch {
    return null;
  }
}

/** このプロジェクトの見える範囲を覚える場所。 */
function wfViewKey(name) { return `${WF_VIEW_KEY}:${name || ""}`; }

/** 覚えている見える範囲を読む。無い・壊れている場合は null。 */
function wfLoadView(name) {
  const store = wfStore("session");
  if (!store) return null;
  const v = wfLoadJson(store, wfViewKey(name), null);
  if (!v) return null;
  const tx = Number(v.tx), ty = Number(v.ty), z = Number(v.z);
  if (!Number.isFinite(tx) || !Number.isFinite(ty) || !Number.isFinite(z)) return null;
  return { tx, ty, z: Math.max(0.1, Math.min(2.5, z)) };
}

/** 見える範囲を覚える。 */
function wfSaveView(name, view) {
  const store = wfStore("session");
  if (!store) return;
  wfSaveJson(store, wfViewKey(name), view);
}

/** 覚えているずらし量を全部読む（{<プロジェクト>: {<ノード id>: [dx, dy]}}）。 */
function wfLoadOffsets(name) {
  const store = wfStore("local");
  if (!store) return {};
  const all = wfLoadJson(store, WF_OFFSETS_KEY, {});
  const one = all[name];
  return one && typeof one === "object" ? one : {};
}

/** 覚えているずらし量を全部消す（並びを戻す）。 */
function wfClearOffsets(name) {
  const store = wfStore("local");
  if (!store) return;
  const all = wfLoadJson(store, WF_OFFSETS_KEY, {});
  if (!all[name]) return;
  delete all[name];
  wfSaveJson(store, WF_OFFSETS_KEY, all);
}

/** 1 個のノードのずらし量を覚える。 */
function wfSaveOffset(name, id, dx, dy) {
  const store = wfStore("local");
  if (!store) return;
  const all = wfLoadJson(store, WF_OFFSETS_KEY, {});
  if (!all[name] || typeof all[name] !== "object") all[name] = {};
  all[name][id] = [Math.round(dx), Math.round(dy)];
  wfSaveJson(store, WF_OFFSETS_KEY, all);
}

/** Kanban の帯を開けているか（localStorage から。最初は開く）。 */
function wfLoadKanban(name) {
  const store = wfStore("local");
  if (!store) return true;
  const all = wfLoadJson(store, WF_KANBAN_KEY, {});
  return typeof all[name] === "boolean" ? all[name] : true;
}

/** Kanban の帯の開閉を覚える。 */
function wfSaveKanban(name, open) {
  const store = wfStore("local");
  if (!store) return;
  const all = wfLoadJson(store, WF_KANBAN_KEY, {});
  all[name] = !!open;
  wfSaveJson(store, WF_KANBAN_KEY, all);
}

/** 出している線の種類（localStorage から。最初は全部出す）。 */
function wfVisibleKinds() {
  if (wfKinds) return wfKinds;
  const on = {};
  for (const [key] of WF_EDGE_KINDS) on[key] = true;
  const store = wfStore("local");
  if (store) {
    const stored = wfLoadJson(store, WF_KINDS_KEY, null);
    if (stored) {
      for (const [key] of WF_EDGE_KINDS) {
        if (typeof stored[key] === "boolean") on[key] = stored[key];
      }
    }
  }
  wfKinds = on;
  return wfKinds;
}

/** 線の種類の出し入れを覚える。 */
function wfSaveKinds(on) {
  const store = wfStore("local");
  if (!store) return;
  wfSaveJson(store, WF_KINDS_KEY, { ...on });
}

/** Workflow の JSON の住所。子を出すときだけ ?subtasks=1 を付ける。 */
function wfDataPath(name, subtasks) {
  const base = `/api/projects/${encodeURIComponent(name == null ? "" : name)}/workflow`;
  return subtasks ? `${base}?subtasks=1` : base;
}

/** 子を出すか（localStorage から。最初は出さない）。 */
function wfSubtasksOn() {
  const store = wfStore("local");
  if (!store) return false;
  try { return store.getItem(WF_SUBTASKS_KEY) === "1"; } catch { return false; }
}

/** 子の出し入れを覚える。 */
function wfSaveSubtasks(on) {
  const store = wfStore("local");
  if (!store) return;
  try { store.setItem(WF_SUBTASKS_KEY, on ? "1" : "0"); } catch { /* 動けばよい */ }
}

// ============================================================================
// index.html への読み込みの保険（C-F2）
// ============================================================================
// 共通部品の ui.js と、この画面の CSS は index.html から読む（U4 が足す）。
// 読み込みがまだ無い環境でも画面が動くように、ここでも足してから開く。
// index.html に読み込みが足されたあとは何も起きない（いるものしか足さない）。

/** この画面の CSS を足す（既にあるなら何もしない）。 */
function wfStyle(href) {
  if (typeof document === "undefined") return;
  if (document.querySelector(`link[rel="stylesheet"][href^="${href}"]`)) return;
  const link = document.createElement("link");
  link.rel = "stylesheet";
  link.href = href;
  document.head.appendChild(link);
}

let wfUiPromise = null;

/** 共通部品の ui.js がなければ読み込む（待つ）。 */
function wfUi() {
  if (typeof saiApi === "function") return Promise.resolve();
  if (wfUiPromise) return wfUiPromise;
  wfUiPromise = new Promise((resolve) => {
    const s = document.createElement("script");
    s.src = "/ui.js?v=28";
    s.onload = () => resolve();
    s.onerror = () => resolve(); // 読めなくてもエラーはここで止める
    document.head.appendChild(s);
  });
  return wfUiPromise;
}

// ============================================================================
// 描く（DOM)
// ============================================================================

/** 小さい要素を 1 つ作る。 */
function wfEl(tag, cls, text) {
  const el = document.createElement(tag);
  if (cls) el.className = cls;
  if (text != null) el.textContent = text;
  return el;
}

/** Workflow のデータ（無ければ空）。 */
function wfNodes() {
  return wfData && Array.isArray(wfData.nodes) ? wfData.nodes.filter((n) => n && n.id != null) : [];
}

/** 出している線（両端に札があるもの・種類で出し入れするものだけ）。 */
function wfVisibleEdges() {
  const all = wfData && Array.isArray(wfData.edges) ? wfData.edges : [];
  const on = wfVisibleKinds();
  const ids = new Set(wfNodes().map((n) => n.id));
  return all.filter((e) => e && ids.has(e.from) && ids.has(e.to) && on[e.kind] !== false);
}

/** 盤面（#wf-stage）の幅と高さ。 */
function wfStageSize() {
  const stage = document.getElementById("wf-stage");
  if (!stage) return { w: 0, h: 0 };
  return { w: stage.clientWidth || 0, h: stage.clientHeight || 0 };
}

/** 画面の帯（← 戻る・プロジェクト名・Dashboard|Workflow|List|設定）。 */
function wfRenderBar() {
  const root = document.getElementById("workflow-root");
  if (!root) return;
  let bar = root.querySelector(".appbar");
  if (!bar) {
    // 見出しは 4 画面共通の .proj-bar（← 戻る・プロジェクト名・切り替え・U01 §3）
    bar = wfEl("header", "appbar proj-bar");
    bar.appendChild(wfEl("button", "proj-back", "‹"));
    bar.appendChild(wfEl("h1", "proj-title", wfName));
    const seg = wfEl("div", "list-mode");
    seg.id = "wf-mode-seg";
    bar.appendChild(seg);
    root.appendChild(bar);
  }
  bar.querySelector(".proj-title").textContent = wfName;
  const back = bar.querySelector(".proj-back");
  back.setAttribute("aria-label", "Back to projects");
  back.onclick = () => { location.hash = "#/projects"; };
  if (typeof fillLinkSeg === "function") fillLinkSeg("wf-mode-seg", "workflow", wfName);
}

/** 道具の行（全体表示・＋・－・並びを戻す・Kanban・線の種類・?）。 */
function wfRenderTools() {
  const root = document.getElementById("workflow-root");
  if (!root) return;
  let tools = root.querySelector(".wf-tools");
  if (!tools) {
    tools = wfEl("div", "sai-toolbar wf-tools");
    tools.id = "wf-tools";
    root.appendChild(tools);
  }
  tools.innerHTML = "";
  tools.appendChild(wfToolBtn("全体を表示", () => wfFit(), "画面の中が全部入ります"));
  tools.appendChild(wfToolBtn("＋", () => wfZoomCenter(1.2), "拡大"));
  tools.appendChild(wfToolBtn("－", () => wfZoomCenter(1 / 1.2), "縮小"));
  tools.appendChild(wfToolBtn("並びを戻す", () => wfResetLayout(), "ドラッグでずらした位置を全部元に戻します"));
  const kb = wfToolBtn("Kanban", () => {
    wfKanban = !wfKanban;
    wfSaveKanban(wfName, wfKanban);
    wfRenderTools();
    wfRenderKanban();
  }, "上の 3 段の札の帯を出したり消したりします");
  kb.classList.toggle("active", !!wfKanban);
  tools.appendChild(kb);
  // 線の種類（研究方針の図と同じ見た目。色つきで出す）
  const kinds = wfEl("span", "wf-kinds");
  for (const [key, label, color, dashed] of WF_EDGE_KINDS) {
    const b = wfEl("button", "wf-kind");
    b.type = "button";
    b.textContent = label;
    b.style.setProperty("--wf-kind-color", color);
    b.classList.toggle("active", wfVisibleKinds()[key] !== false);
    if (dashed) b.classList.add("dashed");
    b.addEventListener("click", () => {
      const on = wfVisibleKinds();
      on[key] = on[key] === false;
      wfKinds = on;
      wfSaveKinds(on);
      wfRenderTools();
      wfRenderEdges();
    });
    kinds.appendChild(b);
  }
  tools.appendChild(kinds);
  const sub = wfToolBtn(`Subtasks: ${wfSubtasks ? "on" : "off"}`, () => wfToggleSubtasks(),
    "Show or hide child runs made by task create");
  sub.setAttribute("aria-pressed", wfSubtasks ? "true" : "false");
  sub.classList.toggle("active", wfSubtasks);
  tools.appendChild(sub);
  tools.appendChild(wfToolBtn("？", () => wfOpenLegend(), "凡例（色と印の意味）"));
}

/** 道具の行のボタン 1 つ。 */
function wfToolBtn(label, onClick, title) {
  const b = wfEl("button", "wf-tool", label);
  b.type = "button";
  if (title) b.title = title;
  b.addEventListener("click", onClick);
  return b;
}

/** 赤い 1 行（読めなかったときの文言）。前の表示は残す。 */
function wfSetError(text) {
  wfError = text || "";
  wfRenderError();
}

function wfRenderError() {
  const box = document.getElementById("wf-error");
  if (!box) return;
  box.textContent = wfError;
  box.hidden = !wfError;
}

// --- Kanban の帯 ---

function wfRenderKanban() {
  const root = document.getElementById("workflow-root");
  if (!root) return;
  let wrap = document.getElementById("wf-kanban");
  if (!wrap) {
    wrap = wfEl("div", "wf-kanban");
    wrap.id = "wf-kanban";
    root.appendChild(wrap);
  }
  wrap.innerHTML = "";
  wrap.hidden = !wfKanban;
  if (!wfKanban) return;

  const lanes = wfLanes(wfData);
  if (wfData && wfLanesEmpty(wfData)) {
    wrap.appendChild(wfEl("p", "wf-kanban-empty", "Kanban に札はありません"));
    return;
  }
  const row = wfEl("div", "wf-lanes");
  for (const lane of lanes) {
    const col = wfEl("div", "wf-lane");
    const head = wfEl("div", "wf-lane-head");
    head.appendChild(wfEl("span", "wf-lane-name", lane.key));
    head.appendChild(wfEl("span", "wf-lane-count", String(lane.cards.length)));
    col.appendChild(head);
    if (lane.cards.length === 0) {
      col.appendChild(wfEl("p", "wf-lane-empty", "なし"));
    } else {
      for (const card of lane.cards) col.appendChild(wfLaneCard(card, lane.key));
    }
    row.appendChild(col);
  }
  wrap.appendChild(row);
}

/** Kanban の札 1 枚。押すと中身の板が出る。 */
function wfLaneCard(card, lane) {
  const b = wfEl("button", "wf-lane-card");
  b.type = "button";
  b.appendChild(wfEl("span", "wf-lane-card-title", String(card.title || `ID${card.run_id}`)));
  if (card.date) b.appendChild(wfEl("span", "wf-lane-card-date", String(card.date)));
  if (card.description) b.appendChild(wfEl("span", "wf-lane-card-desc", String(card.description)));
  b.addEventListener("click", () => wfOpenSheet(card, lane));
  return b;
}

// --- 盤面 ---

/** 盤面（キャンバスと同じ作り。overflow: hidden の中のワールドを transform で動かす）。 */
function wfRenderStage() {
  const root = document.getElementById("workflow-root");
  if (!root) return;
  let stage = document.getElementById("wf-stage");
  if (!stage) {
    stage = wfEl("div", "wf-stage");
    stage.id = "wf-stage";
    stage.tabIndex = 0;
    const world = wfEl("div", "wf-world");
    world.id = "wf-world";
    stage.appendChild(world);
    const loading = wfEl("p", "wf-loading", "読み込み中…");
    loading.id = "wf-loading";
    stage.appendChild(loading);
    root.appendChild(stage);

    stage.addEventListener("pointerdown", (e) => wfOnPointerDown(e));
    stage.addEventListener("pointermove", (e) => wfOnPointerMove(e));
    stage.addEventListener("pointerup", (e) => wfOnPointerUp(e));
    stage.addEventListener("pointercancel", (e) => wfOnPointerUp(e));
    stage.addEventListener("wheel", (e) => wfOnWheel(e), { passive: false });
  }
}

/** 盤面を描く。 */
function wfRender() {
  const stage = document.getElementById("wf-stage");
  const world = document.getElementById("wf-world");
  if (!stage || !world) return;
  const loading = document.getElementById("wf-loading");
  if (loading) loading.remove();
  world.innerHTML = "";
  wfCardEls = new Map();

  const nodes = wfNodes();
  if (nodes.length === 0) {
    const box = wfEl("div", "wf-board-empty");
    if (typeof saiEmpty === "function") {
      box.appendChild(saiEmpty({
        icon: "🗂",
        title: "Workflow にはまだ何もありません",
        body: "review を確認して「Workflow へ移す」と、ここに並びます。",
      }));
    }
    world.appendChild(box);
    wfApplyView();
    return;
  }

  // 1. round の区切り線（札と線より下）
  const seps = wfData && Array.isArray(wfData.separators) ? wfData.separators : [];
  if (seps.length > 0) {
    // .wf-world は幅が 0 なので、区切り線に長さがないと線が出ない。
    // 盤面の右端までの長さをここで決めておく。
    const right = wfBoardRight(wfBoardRects(nodes, wfOffsets)) + WF_SEP_PAD_PX;
    const sep = wfEl("div", "wf-seps");
    sep.style.width = `${right}px`;
    for (const s of seps) {
      const line = wfSeparatorLine(s);
      const row = wfEl("div", "wf-sep");
      row.style.top = `${line.y}px`;
      row.appendChild(wfEl("span", "wf-sep-label", line.label));
      sep.appendChild(row);
    }
    world.appendChild(sep);
  }

  // 2. 線（札の下）
  const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
  svg.setAttribute("id", "wf-edges");
  svg.classList.add("wf-edges");
  world.appendChild(svg);
  wfSvg = svg;

  // 3. 札（一番上）
  for (const n of nodes) {
    const card = wfCardEl(n);
    world.appendChild(card);
    wfCardEls.set(n.id, card);
  }

  wfRenderEdges();
  wfApplyView();
}

/** 札の要素。 */
function wfCardEl(n) {
  const box = wfEl("div", "wf-card");
  box.setAttribute("data-wf-id", String(n.id));
  const r = wfNodeRect(n, wfOffsets);
  box.style.left = `${r.x}px`;
  box.style.top = `${r.y}px`;
  box.style.width = `${r.width}px`;
  box.style.height = `${r.height}px`;
  if (wfSelectedRunId() === n.run_id) box.classList.add("wf-sel");

  const lines = wfCardLines(n);
  const head = wfEl("div", "wf-card-title");
  if (lines.marks.running) head.appendChild(wfEl("span", "wf-dot wf-dot-run"));
  if (lines.marks.failed) head.appendChild(wfEl("span", "wf-dot wf-dot-fail"));
  head.appendChild(wfEl("span", "wf-card-id", lines.title));
  if (lines.marks.chips.length > 0) {
    const chips = wfEl("span", "wf-card-chips");
    for (const chip of lines.marks.chips) chips.appendChild(wfEl("span", `sai-chip ${chip.cls}`, chip.text));
    head.appendChild(chips);
  }
  box.appendChild(head);

  if (lines.model) box.appendChild(wfEl("div", "wf-card-model", lines.model));
  if (lines.time || lines.cost) {
    const meta = wfEl("div", "wf-card-meta");
    meta.appendChild(wfEl("span", "wf-card-time", lines.time));
    if (lines.cost) meta.appendChild(wfEl("span", "wf-card-cost", lines.cost));
    box.appendChild(meta);
  }
  if (lines.desc.length > 0) {
    const desc = wfEl("div", "wf-card-desc");
    for (const line of lines.desc) desc.appendChild(wfEl("span", "wf-card-desc-line", line));
    box.appendChild(desc);
  }
  return box;
}

/** 線を描き直す（選択・ドラッグ・読み直し・種類の切り替えの後）。 */
function wfRenderEdges() {
  const svg = wfSvg || document.getElementById("wf-edges");
  if (!svg) return;
  wfSvg = svg;
  while (svg.firstChild) svg.removeChild(svg.firstChild);

  const byId = new Map(wfNodes().map((n) => [n.id, n]));
  const rects = new Map();
  for (const [id, n] of byId) rects.set(id, wfNodeRect(n, wfOffsets));

  // 線の外接（viewBox 用）
  let minX = Infinity, minY = Infinity, maxX = -Infinity, maxY = -Infinity;
  for (const r of rects.values()) {
    if (r.x < minX) minX = r.x;
    if (r.y < minY) minY = r.y;
    if (r.x + r.width > maxX) maxX = r.x + r.width;
    if (r.y + r.height > maxY) maxY = r.y + r.height;
  }
  if (!Number.isFinite(minX)) { minX = 0; minY = 0; maxX = 1; maxY = 1; }
  const pad = 60;
  minX -= pad; minY -= pad; maxX += pad; maxY += pad;
  svg.setAttribute("viewBox", `${wfR1(minX)} ${wfR1(minY)} ${wfR1(maxX - minX)} ${wfR1(maxY - minY)}`);
  svg.setAttribute("width", String(wfR1(maxX - minX)));
  svg.setAttribute("height", String(wfR1(maxY - minY)));
  svg.style.left = `${wfR1(minX)}px`;
  svg.style.top = `${wfR1(minY)}px`;

  // 矢印（種類ごとに 1 つ）
  const defs = document.createElementNS("http://www.w3.org/2000/svg", "defs");
  for (const [key, , color] of WF_EDGE_KINDS) {
    const marker = document.createElementNS("http://www.w3.org/2000/svg", "marker");
    marker.setAttribute("id", `wf-arrow-${key}`);
    marker.setAttribute("viewBox", "0 0 10 10");
    marker.setAttribute("refX", "9");
    marker.setAttribute("refY", "5");
    marker.setAttribute("markerWidth", "6");
    marker.setAttribute("markerHeight", "6");
    marker.setAttribute("orient", "auto-start-reverse");
    const path = document.createElementNS("http://www.w3.org/2000/svg", "path");
    path.setAttribute("d", "M 0 0 L 10 5 L 0 10 z");
    path.setAttribute("fill", color);
    marker.appendChild(path);
    defs.appendChild(marker);
  }
  svg.appendChild(defs);

  for (const e of wfVisibleEdges()) {
    const a = rects.get(e.from);
    const b = rects.get(e.to);
    if (!a || !b) continue;
    const color = wfEdgeColor(e.kind);
    const opacity = wfEdgeOpacity(e, wfSelected);
    const on = e.from === wfSelected || e.to === wfSelected;
    const w = on ? WF_EDGE_W_SEL_PX : WF_EDGE_W_PX;
    const path = document.createElementNS("http://www.w3.org/2000/svg", "path");
    path.setAttribute("d", wfEdgePath(a, b, e.side === true));
    path.setAttribute("fill", "none");
    path.setAttribute("stroke", color);
    path.setAttribute("stroke-width", String(w));
    // 縮小しても線が消えないよう、画面上の太さで描く
    path.style.strokeWidth = `min(calc(${w}px * var(--wf-z-inv, 1)), ${WF_EDGE_W_MAX_PX}px)`;
    path.setAttribute("stroke-opacity", String(opacity));
    path.setAttribute("marker-end", `url(#wf-arrow-${e.kind || "other"})`);
    if (wfEdgeDashed(e.kind)) path.setAttribute("stroke-dasharray", "8 5");
    svg.appendChild(path);
  }
}

/** 見えている範囲を transform にする。 */
function wfApplyView() {
  const world = document.getElementById("wf-world");
  if (!world) return;
  world.style.transform = `translate(${wfView.tx}px, ${wfView.ty}px) scale(${wfView.z})`;
  world.classList.toggle("wf-far", wfView.z < WF_FAR_Z);
  world.style.setProperty("--wf-z", String(wfView.z));
  // 拡大率の逆数（題・線・ラベルの大きさを画面上の px で保つために使う）
  world.style.setProperty("--wf-z-inv", wfZoomInv(wfView.z));
  world.style.setProperty("--wf-title-px", `${WF_TITLE_PX}px`);
  world.style.setProperty("--wf-title-max", `${WF_TITLE_MAX_PX}px`);
  wfSaveView(wfName, wfView);
}

/** 拡大率の逆数（0 にならないように 1 を下限にする）。 */
function wfZoomInv(z) {
  const n = Number(z);
  if (!Number.isFinite(n) || n <= 0) return 1;
  return 1 / n;
}

/** 全体を表示する（区切り線とラベルも切れないよう、札の外側の分も含める）。 */
function wfFit() {
  const size = wfStageSize();
  if (size.w <= 0 || size.h <= 0) return;
  const seps = wfData && Array.isArray(wfData.separators) ? wfData.separators : [];
  wfView = wfFitView(wfFitRects(wfNodes(), wfOffsets, seps), size.w, size.h);
  wfApplyView();
}

/** 盤面の中央を中心に拡大・縮小。 */
function wfZoomCenter(factor) {
  const size = wfStageSize();
  if (size.w <= 0 || size.h <= 0) return;
  wfView = wfZoomAt(wfView, size.w / 2, size.h / 2, factor);
  wfApplyView();
}

/** ドラッグでずらした位置を全部元に戻す（agent-link の配置に戻す）。 */
async function wfResetLayout() {
  const ok = await saiConfirm({
    title: "並びを戻しますか？",
    body: "この端末でドラッグしてずらした札の位置を全部元に戻します。Mac 側の並びは変わりません。",
    ok: "戻す",
  });
  if (!ok) return;
  wfClearOffsets(wfName);
  wfOffsets = {};
  wfRender();
  saiToast("並びを戻しました", "ok");
}

/** 選んでいる札の run_id（同じ実行の合成ノードも一緒に黄にするため）。 */
function wfSelectedRunId() {
  if (wfSelected == null) return null;
  const n = wfNodes().find((x) => x.id === wfSelected);
  return n ? n.run_id : null;
}

/** 札を選ぶ（同じ run_id の札も一緒に黄にする）。 */
function wfSelect(id) {
  wfSelected = id == null ? null : id;
  const runId = wfSelectedRunId();
  const edges = wfVisibleEdges();
  for (const [nid, card] of wfCardEls) {
    const node = wfNodes().find((n) => n.id === nid);
    card.classList.toggle("wf-sel", !!node && node.run_id === runId);
    card.classList.toggle("wf-dim", wfSelected != null && !!node && node.run_id !== runId
      && !wfLinkedTo(edges, wfSelected, nid));
  }
  wfRenderEdges();
}

// ============================================================================
// 札の中身（板）
// ============================================================================

/** 板の中身を作る。card は Workflow のノードでも Kanban の札でもよい。 */
function wfSheetBody(card, lane) {
  const c = card || {};
  const runId = c.run_id;
  const box = wfEl("div", "wf-sheet");
  const head = wfEl("div", "wf-sheet-head-body");
  head.appendChild(wfEl("p", "wf-sheet-title", String(c.title || wfCardTitle(c) || `ID${runId}`)));
  const meta = wfEl("div", "wf-sheet-meta");
  const state = wfStateOf(c);
  if (state) {
    const chip = wfEl("span", "sai-chip", wfStateLabel(state));
    meta.appendChild(chip);
  }
  const model = wfCardModel(c);
  if (model) meta.appendChild(wfEl("span", "wf-sheet-line", `Model ${model}`));
  if (c.created_at) {
    meta.appendChild(wfEl("span", "wf-sheet-line", `作成 ${wfStampText(c.created_at)}`));
  }
  if (c.cost) meta.appendChild(wfEl("span", "wf-sheet-line", `費用 ${c.cost}`));
  if (c.description) meta.appendChild(wfEl("p", "wf-sheet-desc", String(c.description)));
  head.appendChild(meta);
  box.appendChild(head);

  // 見るボタン（List のその表示へ移る／会話へ移る）
  if (runId != null) {
    const go = wfEl("div", "wf-sheet-go");
    for (const [key, label] of [["log", "Log"], ["task", "task"], ["review", "review"],
      ["message", "message"], ["waggle", "waggle"], ["stats", "Stats"], ["history", "History"]]) {
      const b = wfEl("button", "sai-btn small", label);
      b.type = "button";
      b.addEventListener("click", () => {
        if (typeof listHash === "function") location.hash = listHash(wfName, runId, key);
      });
      go.appendChild(b);
    }
    const talk = wfEl("button", "sai-btn small", "Chats");
    talk.type = "button";
    talk.addEventListener("click", () => {
      if (typeof sessionHash === "function") location.hash = sessionHash(wfName, runId);
    });
    go.appendChild(talk);
    box.appendChild(go);
  }

  // 操作ボタン
  const acts = wfActions(c, lane, wfName);
  if (acts.length > 0) {
    const row = wfEl("div", "wf-sheet-actions");
    for (const act of acts) {
      const b = wfEl("button", `sai-btn ${act.danger ? "danger" : "primary"}`, act.label);
      b.type = "button";
      b.disabled = wfBusy;
      b.addEventListener("click", () => wfDoAction(act, c));
      row.appendChild(b);
    }
    box.appendChild(row);
  }
  return box;
}

/** 板を開く。 */
function wfOpenSheet(card, lane) {
  if (typeof saiSheet !== "function") return;
  const body = wfSheetBody(card, lane);
  wfSheetHandle = saiSheet({
    title: String(card.title || wfCardTitle(card) || ""),
    body,
    onClose: () => { wfSheetHandle = null; },
  });
}

/** 操作を実行する（押したら知らせと盤面の読み直し）。 */
async function wfDoAction(act, card) {
  if (!act || wfBusy) return;
  if (act.confirm) {
    const ok = await saiConfirm({ title: act.label, body: act.confirm, ok: "はい", danger: !!act.danger });
    if (!ok) return;
  }
  wfBusy = true;
  try {
    await saiApi(act.path, { method: act.method || "POST" });
    saiToast(`${act.label} しました`, "ok");
  } catch (e) {
    saiToast((e && e.message) || "できませんでした", "error");
    return;
  } finally {
    wfBusy = false;
  }
  // 動かし終わったら板を閉じて、盤面を新しい状態へ描き直す
  if (wfSheetHandle) {
    const handle = wfSheetHandle;
    wfSheetHandle = null;
    handle.close();
  }
  await wfReload();
}

/** 凡例（? の小窓）。 */
function wfOpenLegend() {
  const legend = wfLegend();
  const box = wfEl("div", "wf-legend");
  const t2 = wfEl("p", "wf-legend-title", "線（下の始点から終点へ）");
  box.appendChild(t2);
  const edges = wfEl("div", "wf-legend-items");
  for (const e of legend.edges) {
    const chip = wfEl("span", "sai-chip", e.label);
    chip.style.borderColor = e.color;
    chip.style.color = e.color;
    edges.appendChild(chip);
  }
  box.appendChild(edges);
  const t3 = wfEl("p", "wf-legend-title", "札の印");
  box.appendChild(t3);
  const marks = wfEl("div", "wf-legend-items");
  for (const m of legend.marks) {
    const chip = wfEl("span", `sai-chip ${m.cls === "dot-running" || m.cls === "dot-failed" ? "" : m.cls}`, m.text);
    if (m.cls === "dot-running") chip.prepend(wfEl("span", "wf-dot wf-dot-run"));
    if (m.cls === "dot-failed") chip.prepend(wfEl("span", "wf-dot wf-dot-fail"));
    marks.appendChild(chip);
  }
  box.appendChild(marks);
  if (typeof saiSheet === "function") saiSheet({ title: "凡例", body: box });
}

// ============================================================================
// 操作（Pointer Events。マウスと指を同じに扱う）
// ============================================================================

/** 押した要素が属するノード（data-wf-id を持つ最も近い要素）。 */
function wfHitNode(target) {
  if (!target || typeof target.closest !== "function") return null;
  const box = target.closest("[data-wf-id]");
  if (!box) return null;
  const raw = box.getAttribute("data-wf-id");
  const id = Number(raw);
  return Number.isFinite(id) ? id : raw;
}

function wfOnPointerDown(e) {
  const stage = document.getElementById("wf-stage");
  if (!stage) return;
  try { stage.setPointerCapture(e.pointerId); } catch { /* 捕まえなくてもよい */ }
  wfPtrs.set(e.pointerId, { x: e.clientX, y: e.clientY });

  if (wfPtrs.size === 2) {
    const [p, q] = [...wfPtrs.values()];
    const rect = stage.getBoundingClientRect();
    wfPinch = {
      dist: Math.hypot(q.x - p.x, q.y - p.y) || 1,
      z0: wfView.z,
      wx: ((p.x + q.x) / 2 - rect.left - wfView.tx) / wfView.z,
      wy: ((p.y + q.y) / 2 - rect.top - wfView.ty) / wfView.z,
    };
    wfDown = null;
    return;
  }
  if (wfPtrs.size > 2) return;

  const id = wfHitNode(e.target);
  wfDown = {
    id: e.pointerId,
    sx: e.clientX,
    sy: e.clientY,
    view: { ...wfView },
    nodeId: id,
    mode: null, // "pan" | "card"
  };
}

function wfOnPointerMove(e) {
  if (!wfPtrs.has(e.pointerId)) return;
  wfPtrs.set(e.pointerId, { x: e.clientX, y: e.clientY });

  // ピンチ（2 本）
  if (wfPtrs.size >= 2 && wfPinch) {
    const [p, q] = [...wfPtrs.values()];
    const dist = Math.hypot(q.x - p.x, q.y - p.y) || 1;
    const stage = document.getElementById("wf-stage");
    const rect = stage ? stage.getBoundingClientRect() : { left: 0, top: 0 };
    const z = Math.max(0.1, Math.min(2.5, wfPinch.z0 * (dist / wfPinch.dist)));
    wfView = {
      tx: (p.x + q.x) / 2 - rect.left - wfPinch.wx * z,
      ty: (p.y + q.y) / 2 - rect.top - wfPinch.wy * z,
      z,
    };
    wfApplyView();
    return;
  }

  const d = wfDown;
  if (!d || d.id !== e.pointerId) return;
  const dx = e.clientX - d.sx;
  const dy = e.clientY - d.sy;
  if (!d.mode) {
    if (Math.hypot(dx, dy) < WF_DRAG_SLOP) return; // クリックかドラッグか決まらない
    d.mode = d.nodeId == null ? "pan" : "card";
  }

  if (d.mode === "pan") {
    wfView = { tx: d.view.tx + dx, ty: d.view.ty + dy, z: d.view.z };
    wfApplyView();
    return;
  }

  // 札を動かす（ワールドの座標に直す）
  const wx = dx / d.view.z;
  const wy = dy / d.view.z;
  const card = wfCardEls.get(d.nodeId);
  if (card) {
    const node = wfNodes().find((n) => n.id === d.nodeId);
    if (node) {
      const r = wfNodeRect(node, wfOffsets);
      card.style.left = `${r.x + wx}px`;
      card.style.top = `${r.y + wy}px`;
      wfRenderEdges();
    }
  }
}

function wfOnPointerUp(e) {
  wfPtrs.delete(e.pointerId);
  if (wfPtrs.size < 2) wfPinch = null;
  const stage = document.getElementById("wf-stage");
  if (stage) {
    try { stage.releasePointerCapture(e.pointerId); } catch { /* 解放できなくてもよい */ }
  }
  const d = wfDown;
  if (!d || d.id !== e.pointerId) {
    if (wfPtrs.size === 0) wfDown = null;
    return;
  }
  wfDown = null;
  if (e.type === "pointercancel") return; // 途中で指が離れた（クリックにも保存にもしない）

  if (!d.mode) {
    // 動かしていない（4px 未満）-> 札を押した
    if (d.nodeId == null) {
      wfSelect(null);
      return;
    }
    const node = wfNodes().find((n) => n.id === d.nodeId);
    if (!node) return;
    wfSelect(d.nodeId);
    wfOpenSheet(node, null);
    return;
  }
  if (d.mode === "pan" || d.nodeId == null) return;

  // ドラッグした -> ずらした量をこの端末に覚える
  const node = wfNodes().find((n) => n.id === d.nodeId);
  if (!node) return;
  const dx = (e.clientX - d.sx) / d.view.z;
  const dy = (e.clientY - d.sy) / d.view.z;
  const prev = (wfOffsets && wfOffsets[d.nodeId]) || [0, 0];
  const nx = (Array.isArray(prev) ? Number(prev[0]) || 0 : 0) + dx;
  const ny = (Array.isArray(prev) ? Number(prev[1]) || 0 : 0) + dy;
  const card = wfCardEls.get(d.nodeId);
  if (card) {
    const r = wfNodeRect(node, wfOffsets);
    card.style.left = `${r.x + nx}px`;
    card.style.top = `${r.y + ny}px`;
  }
  wfOffsets[d.nodeId] = [nx, ny];
  wfSaveOffset(wfName, d.nodeId, nx, ny);
  wfRenderEdges();
}

/** ホイール（縦・横・Ctrl で拡大）。 */
function wfOnWheel(e) {
  const stage = document.getElementById("wf-stage");
  if (!stage) return;
  e.preventDefault();
  const rect = stage.getBoundingClientRect();
  const px = e.clientX - rect.left;
  const py = e.clientY - rect.top;
  if (e.ctrlKey || e.metaKey) {
    wfView = wfZoomAt(wfView, px, py, Math.exp(-e.deltaY * 0.01));
  } else {
    wfView = {
      tx: wfView.tx - (e.shiftKey ? e.deltaY : (e.deltaX || 0)),
      ty: wfView.ty - (e.shiftKey ? 0 : e.deltaY),
      z: wfView.z,
    };
  }
  wfApplyView();
}

/** キー。盤面の画面が出ていて入力欄に打っていないときだけ受ける。 */
function wfOnKeyDown(e) {
  const sec = document.getElementById("view-workflow");
  if (!sec || sec.hidden) return;
  const t = e.target;
  const tag = t && t.tagName ? t.tagName.toUpperCase() : "";
  if (tag === "INPUT" || tag === "TEXTAREA" || tag === "SELECT") return;
  if (t && t.isContentEditable) return;
  if (e.metaKey || e.ctrlKey) return;
  const pan = (dx, dy) => {
    wfView = { tx: wfView.tx + dx, ty: wfView.ty + dy, z: wfView.z };
    wfApplyView();
  };
  switch (e.key) {
    case "+": case "=": wfZoomCenter(1.2); break;
    case "-": case "_": wfZoomCenter(1 / 1.2); break;
    case "0": wfFit(); break;
    case "ArrowUp": pan(0, WF_KEY_PAN); break;
    case "ArrowDown": pan(0, -WF_KEY_PAN); break;
    case "ArrowLeft": pan(WF_KEY_PAN, 0); break;
    case "ArrowRight": pan(-WF_KEY_PAN, 0); break;
    case "Escape": wfSelect(null); break;
    default: return;
  }
  e.preventDefault();
}

/** 画面幅が変わったときの出し入れ（全体表示はしない・見える範囲は保つ）。 */
function wfOnResize() {
  /* 見える範囲は覚えたまま使う（画面幅が変わっても動かさない） */
}

// ============================================================================
// 開く・読み直す
// ============================================================================

/** 画面を組み立てる（帯・道具の行・盤面）。 */
function wfBuild() {
  const root = document.getElementById("workflow-root");
  if (!root) return;
  root.innerHTML = "";
  wfRenderBar();
  const err = wfEl("div", "wf-error");
  err.id = "wf-error";
  err.hidden = true;
  root.appendChild(err);
  wfRenderTools();
  wfRenderStage();
  wfRenderError();
}

/** Workflow を開く。 */
async function wfOpen(params) {
  wfStyle("/workflow.css?v=28");
  await wfUi();
  wfName = (params && params.name) || "";
  wfData = null;
  wfRaw = "";
  wfError = "";
  wfSelected = null;
  wfBusy = false;
  wfPtrs = new Map();
  wfDown = null;
  wfPinch = null;
  wfKinds = null;
  wfSubtasks = wfSubtasksOn();
  wfOffsets = wfLoadOffsets(wfName);
  wfKanban = wfLoadKanban(wfName);

  wfBuild();
  // 見える範囲は覚えたものを使い、覚えていなければ全体を入れる
  const saved = wfLoadView(wfName);
  await wfLoad();
  if (!saved) wfFit();
}

/** 子の表示を切り替える（覚えて、JSON を取り直して描き直す。配置は Rust が子の有無で決める）。 */
async function wfToggleSubtasks() {
  wfSubtasks = !wfSubtasks;
  wfSaveSubtasks(wfSubtasks);
  wfRenderTools();
  wfRaw = ""; // 同じ JSON でも描き直す
  await wfLoad();
}

/** 10 秒ごとの読み直し（同じ JSON なら描き直さない）。 */
async function wfPoll() {
  await wfLoad();
}

/** 読み直す。前回の JSON が同じ文字なら描き直さない。 */
async function wfLoad() {
  if (!wfName) return;
  const name = wfName;
  try {
    const data = await saiApi(wfDataPath(name, wfSubtasks));
    if (wfName !== name) return; // 別の画面に動いていた
    const raw = JSON.stringify(data);
    const same = raw === wfRaw;
    wfData = data && typeof data === "object" ? data : {};
    wfRaw = raw;
    wfError = "";
    if (same) return; // 前と同じなら描き直さない
  } catch (e) {
    if (wfName !== name) return;
    wfError = (e && e.message) || "読み込めませんでした";
    wfRenderError();
    // 前の表示は残す
    if (wfData) return;
    wfData = { nodes: [], edges: [], separators: [], lanes: {} };
  }
  wfRenderError();
  // 選んでいた札が無くなっていたら選択を外す
  if (wfSelected != null && !wfNodes().some((n) => n.id === wfSelected)) {
    wfSelected = null;
    if (wfSheetHandle) {
      const handle = wfSheetHandle;
      wfSheetHandle = null;
      handle.close();
    }
  }
  wfRender();
  wfRenderKanban();
}

// ============================================================================
// 画面の登録（window.SaiRoutes）
// ============================================================================

if (typeof window !== "undefined") {
  (window.SaiRoutes = window.SaiRoutes || []).push({
    match(hash) {
      return wfParseHash(hash);
    },
    view: "view-workflow",
    tab: "project",
    hideTabbar: false,
    interval: WF_INTERVAL_MS,
    async open(params) {
      await wfOpen(params);
    },
    async poll() {
      await wfPoll();
    },
  });

  window.addEventListener("keydown", wfOnKeyDown);
  window.addEventListener("resize", wfOnResize);
}

// --- node からの読み込み（tests/test_workflow.js） ---
if (typeof module !== "undefined" && module.exports) {
  module.exports = {
    WF_CELL_W,
    WF_CELL_H,
    WF_INTERVAL_MS,
    WF_FAR_Z,
    WF_TITLE_PX,
    WF_TITLE_MAX_PX,
    WF_EDGE_W_PX,
    WF_EDGE_W_SEL_PX,
    WF_EDGE_DIM,
    WF_SEP_PAD_PX,
    WF_DRAG_SLOP,
    WF_EDGE_KINDS,
    WF_LANES,
    WF_SEL_COLOR,
    WF_VIEW_KEY,
    WF_OFFSETS_KEY,
    WF_KANBAN_KEY,
    WF_KINDS_KEY,
    WF_SUBTASKS_KEY,
    WF_EDGE_UNKNOWN,
    wfHash,
    wfParseHash,
    wfNodeRect,
    wfBoardRects,
    wfFitView,
    wfFitRects,
    wfBoardRight,
    wfZoomInv,
    wfZoomAt,
    wfCardTitle,
    wfStamp,
    wfStampText,
    wfWrapText,
    wfCardMarks,
    wfCardModel,
    wfDataPath,
    wfCardLines,
    wfEdgeColor,
    wfEdgeDashed,
    wfEdgePath,
    wfEdgeEndX,
    wfEdgeEndY,
    wfEdgeOpacity,
    wfLinkedTo,
    wfSeparatorLine,
    wfLanes,
    wfLanesEmpty,
    wfStateOf,
    wfStateLabel,
    wfActions,
    wfLegend,
    wfViewKey,
    wfLoadView,
    wfSaveView,
    wfLoadOffsets,
    wfSaveOffset,
    wfClearOffsets,
    wfLoadKanban,
    wfSaveKanban,
    wfVisibleKinds,
    wfSaveKinds,
  };
}
