"use strict";
// 研究方針の木の図（実装計画 U20）。一番上の Q から下へ伸びる Q・O・F・E の木 1 つ。
//
//   sgRenderGraph(container, {status}, onSelect) … 図を描く
//   sgTreeLayout({status}, opts)                 … 配置（純関数）
//   sgMarkSelected(id)                           … 選んでいる札の印（Node の ID か S<n>）
//
// onSelect には {kind: "node", id} / {kind: "seed", id} を渡す（札の data-sg-id と同じ ID）。
// SEED の札は、その SEED の道（★ の O の並び）の一番下の O の部分木の下に出す。
//
// 中の処理は Rust のコアが返した JSON だけを使う（apps/phone は他の DB を開かない）。
// 札の大きさは固定。見える範囲（tx, ty, z）だけを動かし、札の位置は動かさない。
// 全体表示と拡大は canvaslayout.js の cvFitView / cvZoomAt を typeof で確かめて使う。
// 依存は足さない。札は HTML の div、線は同じ中の SVG。
//
// 一番外側の名前は sg で始める（test_globals.js が重なりを検査する）。

// --- 定数 ---

// 札の大きさ（世界上の px）
const SG_CARD_W = 200;
const SG_CARD_H = 72;
const SG_TREE_STEP = 120; // 段と段の縦の間隔（札の上端どうし）
const SG_TREE_GAP = 24;   // 同じ段の札と札の間
// この数を超える節点では、F を隠した状態で始める（見やすさのため）
const SG_MANY_NODES = 60;
// ドラッグとクリックの境（これ以上動いたらドラッグ、未満ならクリック）
const SG_DRAG_SLOP = 4;
// 拡大・縮小の釦を 1 回押したときの拡大率の倍率
const SG_ZOOM_STEP = 1.2;

// 線の種類。sgEdgeStyle がこの表を返す。
// [種類, 表示名, 破線, 矢印つきの実線か, 色の CSS 変数]
const SG_EDGE_KINDS = [
  ["tree", "親子", "", false, "--sci-edge-parent"],
  ["seed", "SEED の道", "2 4", false, "--sci-edge-depends"],
];

// --- 状態 ---

let sgStatus = null;   // 最後に描いた status の JSON
let sgSelMap = new Map(); // 札の id → onSelect に渡す sel
let sgSelectedKey = null; // 選んでいる札の id
let sgOnSelect = null; // 札を押したときに呼ぶ関数
let sgRoot = null;     // 図を囲む要素
let sgFBtn = null;     // F の出し入れの釦（ラベルを sgDraw で更新する）
let sgStage = null;    // 見える範囲を出す所
let sgWorld = null;    // その中の世界（transform で動かす）
let sgView = { tx: 0, ty: 0, z: 1 }; // 画面上の点 = 世界の点 * z + (tx, ty)
let sgHideF = false;   // F の札を隠すか
let sgRects = new Map();  // 札の id → 置いた場所（線と札の共通）
let sgExtraRects = [];    // 見える範囲の算出に入れる四角（今は使わない。sgFit・sgSetViewBox が読む）
let sgPtrs = new Map();  // 押している指（1 本 = 移動、2 本 = 拡大）
let sgDown = null;       // 1 本指で押したままの情報
let sgPinch = null;      // 2 本指の情報
let sgFitted = false;    // 全体表示を 1 回はしたか

// ============================================================================
// DOM を使わない部品（node のテストから読む）
// ============================================================================

/** 線の種類 → 描き方。未知の値も止まらないように、灰色の実線を返す。 */
function sgEdgeStyle(kind) {
  const key = kind == null ? "" : String(kind);
  const row = SG_EDGE_KINDS.find((k) => k[0] === key);
  if (row) return { dash: row[2], arrow: row[3], colorVar: `var(${row[4]})` };
  return { dash: "", arrow: false, colorVar: "var(--sci-edge-parent)" };
}

/** Objectivity の棒（■■□ 2/3）。target が 0 以下なら count だけ。 */
function sgObjBar(o) {
  if (!o) return "";
  const count = Number(o.count) || 0;
  const target = Number(o.target) || 0;
  if (target <= 0) return String(count);
  const n = Math.min(target, 8);
  const on = Math.min(n, count);
  return `${"■".repeat(on)}${"□".repeat(n - on)} ${count}/${target}`;
}

/** 文を n 字で切る。 */
function sgClip(text, n) {
  const s = String(text == null ? "" : text);
  const a = Array.from(s);
  return a.length > n ? `${a.slice(0, n).join("")}…` : s;
}

/**
 * 木の配置。
 * @param input {status}
 * @param opts {hideF} 真なら F の札を出さず、親の札の sub に `F <n>` を書く
 * @returns {cards, edges, width, height}
 */
function sgTreeLayout(input, opts) {
  const status = (input && input.status) || {};
  const hideF = !!(opts && opts.hideF);
  const all = (Array.isArray(status.nodes) ? status.nodes : []).filter((n) => n && n.id);
  const byId = new Map(all.map((n) => [String(n.id), n]));
  const shown = hideF ? all.filter((n) => n.kind !== "F") : all;
  const shownIds = new Set(shown.map((n) => String(n.id)));
  // 見える祖先（隠した F を飛ばして、一番近い見える親）
  const visParent = (n) => {
    let p = n.parent == null ? null : byId.get(String(n.parent));
    const seen = new Set();
    while (p && !shownIds.has(String(p.id)) && !seen.has(p.id)) {
      seen.add(p.id);
      p = p.parent == null ? null : byId.get(String(p.parent));
    }
    return p ? String(p.id) : null;
  };
  const kids = new Map();
  const roots = [];
  for (const n of shown) {
    const p = visParent(n);
    if (p == null) roots.push(n);
    else {
      if (!kids.has(p)) kids.set(p, []);
      kids.get(p).push(n);
    }
  }
  const fCount = new Map();
  if (hideF) {
    for (const n of all) {
      if (n.kind !== "F") continue;
      const p = visParent(n);
      if (p != null) fCount.set(p, (fCount.get(p) || 0) + 1);
    }
  }

  const pos = new Map(); // id → {x, row}
  const cards = [];
  let slot = 0;
  const place = (n, row, guard) => {
    const id = String(n.id);
    const ch = guard.has(id) ? [] : kids.get(id) || [];
    const g2 = new Set(guard).add(id);
    let x;
    if (!ch.length) {
      x = slot * (SG_CARD_W + SG_TREE_GAP);
      slot += 1;
    } else {
      const xs = ch.map((c) => place(c, row + 1, g2));
      x = (xs[0] + xs[xs.length - 1]) / 2;
    }
    pos.set(id, { x, row });
    cards.push(sgNodeCard(n, x, row, fCount.get(id) || 0));
    return x;
  };
  for (const r of roots) place(r, 0, new Set());

  const edges = [];
  for (const n of shown) {
    const p = visParent(n);
    if (p != null && pos.has(p)) edges.push({ from: p, to: String(n.id), kind: "tree" });
  }

  // SEED の札: 道の一番深い O の部分木の一番下の段 + 1
  const rowOf = (id) => (pos.get(String(id)) || {}).row;
  const subtreeBottom = (id) => {
    let m = rowOf(id);
    for (const c of kids.get(String(id)) || []) m = Math.max(m, subtreeBottom(c.id));
    return m;
  };
  const seedsAt = new Map();
  let maxRow = cards.reduce((m, c) => Math.max(m, Math.round(c.y / SG_TREE_STEP)), 0);
  for (const s of Array.isArray(status.seeds) ? status.seeds : []) {
    if (!s || s.id == null) continue;
    let deep = null;
    for (const oid of Array.isArray(s.path) ? s.path : []) {
      const r = rowOf(oid);
      if (r != null && (deep == null || r > rowOf(deep))) deep = String(oid);
    }
    const key = deep == null ? "" : deep;
    if (!seedsAt.has(key)) seedsAt.set(key, []);
    seedsAt.get(key).push(s);
  }
  for (const [oid, list] of seedsAt) {
    const base = oid ? pos.get(oid).x : 0;
    let row = oid ? subtreeBottom(oid) + 1 : maxRow + 1;
    const w = SG_CARD_W + SG_TREE_GAP;
    const hit = (x0, r) => cards.some((c) => Math.round(c.y / SG_TREE_STEP) === r && c.x < x0 + list.length * w - SG_TREE_GAP && c.x + c.w > x0);
    while (hit(base, row)) row += 1; // 別の部分木の札と重なるときは、さらに下の段へ
    list.forEach((s, i) => {
      const id = `S${s.id}`;
      cards.push({
        id, kind: "S", x: base + i * w, y: row * SG_TREE_STEP, w: SG_CARD_W, h: SG_CARD_H,
        text: `${id} ${s.title || ""}`.trim(), sub: sgClip(s.sentence || "", 40),
        starred: false, answerKind: null, onPath: true, dim: false,
      });
      if (oid) edges.push({ from: oid, to: id, kind: "seed" });
    });
    maxRow = Math.max(maxRow, row);
  }

  const width = cards.reduce((m, c) => Math.max(m, c.x + c.w), 0);
  const height = cards.reduce((m, c) => Math.max(m, c.y + c.h), 0);
  return { cards, edges, width, height };
}

/** Node 1 つ分の札。 */
function sgNodeCard(n, x, row, hiddenF) {
  let text = n.text || "";
  let sub = "";
  const counts = n.counts || {};
  const f = hiddenF || Number(counts.f) || 0;
  if (n.kind === "Q") {
    const bar = sgObjBar(n.objectivity);
    const lg = n.logicality && n.logicality.value && n.logicality.value !== "unchecked" ? ` L:${n.logicality.value}` : "";
    sub = `${bar}${lg}`.trim();
  } else if (n.kind === "O") {
    const kind = /issue/i.test(n.answer_kind || "") ? "Issue" : "Answer";
    sub = `${n.starred ? "★ " : ""}${kind}${f ? ` F ${f}` : ""}`;
  } else if (n.kind === "F") {
    text = sgClip(text, 40);
  } else if (n.kind === "E") {
    sub = n.mlflow ? String(n.mlflow) : "";
  }
  if (hiddenF && n.kind !== "O") sub = `${sub} F ${hiddenF}`.trim();
  return {
    id: String(n.id), kind: n.kind, x, y: row * SG_TREE_STEP, w: SG_CARD_W, h: SG_CARD_H,
    text, sub, starred: !!n.starred, answerKind: n.answer_kind || null,
    onPath: n.on_path !== false, dim: n.on_path === false,
  };
}

/** 札から onSelect に渡す値を作る。 */
function sgSelectionOf(card) {
  if (card.kind === "S") return { kind: "seed", id: card.id };
  return { kind: "node", id: card.id };
}

// ============================================================================
// 小さな部品
// ============================================================================

/** 要素を作る（text があれば textContent に入れる）。 */
function sgEl(tag, cls, text) {
  const el = document.createElement(tag);
  if (cls) el.className = cls;
  if (text != null) el.textContent = String(text);
  return el;
}

/** SVG の要素を作る。 */
function sgSvgEl(tag, attrs) {
  const el = document.createElementNS("http://www.w3.org/2000/svg", tag);
  for (const [k, v] of Object.entries(attrs || {})) el.setAttribute(k, String(v));
  return el;
}

/** 小さい 1 行の札（空欄の種類・印・数）。 */
function sgChip(text, kind) {
  return sgEl("span", `sai-chip ${kind || "muted"}`, text);
}

/** 実線の端点（side は right / left / top / bottom）。 */
function sgSidePoint(r, side) {
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
const SG_SIDE_NORMAL = { left: [-1, 0], right: [1, 0], top: [0, -1], bottom: [0, 1] };

/** 小数 1 位に丸める（path を短くするため）。 */
function sgR1(n) {
  return Math.round((Number(n) || 0) * 10) / 10;
}

/** 2 つの四角を結ぶ 3 次ベジェ曲線の d。縦に並ぶときは上から下、横に並ぶときは左から右。 */
function sgEdgePath(a, b) {
  const ra = a || {};
  const rb = b || {};
  const aTop = (Number(ra.y) || 0) + (Number(ra.height) || 0) / 2;
  const bTop = (Number(rb.y) || 0) + (Number(rb.height) || 0) / 2;
  const horizontal = Math.abs(aTop - bTop) < Math.abs(
    (Number(ra.x) || 0) + (Number(ra.width) || 0) / 2 - ((Number(rb.x) || 0) + (Number(rb.width) || 0) / 2)
  );
  const aFirst = horizontal
    ? (Number(ra.x) || 0) + (Number(ra.width) || 0) / 2 <= (Number(rb.x) || 0) + (Number(rb.width) || 0) / 2
    : aTop <= bTop;
  const s1 = horizontal ? (aFirst ? "right" : "left") : (aFirst ? "bottom" : "top");
  const s2 = horizontal ? (aFirst ? "left" : "right") : (aFirst ? "top" : "bottom");
  const p1 = sgSidePoint(ra, s1);
  const p2 = sgSidePoint(rb, s2);
  const n1 = SG_SIDE_NORMAL[s1];
  const n2 = SG_SIDE_NORMAL[s2];
  const k = Math.max(40, Math.hypot(p2.x - p1.x, p2.y - p1.y) * 0.4);
  const c1 = { x: p1.x + n1[0] * k, y: p1.y + n1[1] * k };
  const c2 = { x: p2.x + n2[0] * k, y: p2.y + n2[1] * k };
  return `M ${sgR1(p1.x)} ${sgR1(p1.y)} C ${sgR1(c1.x)} ${sgR1(c1.y)}, ${sgR1(c2.x)} ${sgR1(c2.y)}, ${sgR1(p2.x)} ${sgR1(p2.y)}`;
}

// ============================================================================
// 図
// ============================================================================

/**
 * 図を描く（container の中身を入れ替える）。
 * @param container 入れる先
 * @param input {status}
 * @param onSelect 札を押したときに呼ぶ関数。引数は {kind: "node" | "seed", id}
 */
function sgRenderGraph(container, input, onSelect) {
  if (!container) return;
  container.innerHTML = "";
  sgStatus = (input && input.status) || null;
  sgOnSelect = typeof onSelect === "function" ? onSelect : null;
  sgView = { tx: 0, ty: 0, z: 1 };
  sgFitted = false;
  sgPtrs = new Map();
  sgDown = null;
  sgPinch = null;

  // 節点が多いときは、F を隠した状態で始める
  const nodes = sgStatus && Array.isArray(sgStatus.nodes) ? sgStatus.nodes : [];
  sgHideF = nodes.length > SG_MANY_NODES;

  sgRoot = sgEl("div", "sci-graph");

  // 1. 道具の行（F の出し入れ・拡大・縮小・全体を表示）
  const tools = sgEl("div", "sci-graph-tools sai-toolbar");
  const fBtn = sgEl("button", "sci-graph-btn");
  fBtn.type = "button";
  fBtn.addEventListener("click", () => {
    sgHideF = !sgHideF;
    sgFitted = false;
    sgDraw();
  });
  tools.appendChild(fBtn);
  sgFBtn = fBtn;
  for (const [label, title, onClick] of [
    ["＋", "拡大", () => sgZoom(SG_ZOOM_STEP)],
    ["－", "縮小", () => sgZoom(1 / SG_ZOOM_STEP)],
    ["⤢", "全体を表示", () => sgFit()],
  ]) {
    const b = sgEl("button", "sci-graph-btn", label);
    b.type = "button";
    b.title = title;
    b.setAttribute("aria-label", title);
    b.addEventListener("click", onClick);
    tools.appendChild(b);
  }
  sgRoot.appendChild(tools);

  // 2. 図そのもの
  sgStage = sgEl("div", "sci-graph-stage");
  sgStage.tabIndex = 0;
  sgWorld = sgEl("div", "sci-graph-world");
  sgStage.appendChild(sgWorld);
  sgRoot.appendChild(sgStage);
  sgStage.addEventListener("pointerdown", (e) => sgOnPointerDown(e));
  sgStage.addEventListener("pointermove", (e) => sgOnPointerMove(e));
  sgStage.addEventListener("pointerup", (e) => sgOnPointerUp(e));
  sgStage.addEventListener("pointercancel", (e) => sgOnPointerUp(e));
  sgStage.addEventListener("wheel", (e) => sgOnWheel(e), { passive: false });

  // 3. 凡例
  sgRoot.appendChild(sgLegend());

  container.appendChild(sgRoot);
  sgDraw();
}

/** 図の絵を描く（線・札）。 */
function sgDraw() {
  if (!sgWorld) return;
  if (sgFBtn) {
    sgFBtn.textContent = sgHideF ? "F を出す" : "F を隠す";
    sgFBtn.title = sgHideF ? "F の札を出します" : "F の札を隠します（Q・O・E だけの図になります）";
  }
  sgWorld.innerHTML = "";
  sgRects = new Map();
  sgExtraRects = [];
  sgSelMap = new Map();

  const hasNodes = sgStatus && Array.isArray(sgStatus.nodes) && sgStatus.nodes.some((n) => n && n.id);
  if (!hasNodes) {
    // 0 件のときは直接 舞台に入れる（世界は 0 幅。absolute の箱は文字を 1 文字ずつ縦に並べてしまう）
    sgPutEmpty(sgEmptyBox({
      icon: "🌱",
      title: "Node はまだありません",
      body: "最初の Q を作ると、ここに図が出ます。",
    }));
    sgApplyView();
    return;
  }

  const layout = sgTreeLayout({ status: sgStatus }, { hideF: sgHideF });
  for (const c of layout.cards) {
    sgRects.set(c.id, { x: c.x, y: c.y, width: c.w, height: c.h });
    sgSelMap.set(c.id, sgSelectionOf(c));
  }

  // 線（札の下）
  const svg = sgSvgEl("svg", { class: "sci-graph-edges" });
  sgWorld.appendChild(svg);
  sgSetViewBox(svg);
  for (const e of layout.edges) {
    const a = sgRects.get(e.from);
    const b = sgRects.get(e.to);
    if (!a || !b) continue;
    const st = sgEdgeStyle(e.kind);
    const path = sgSvgEl("path", {
      d: sgEdgePath(a, b),
      fill: "none",
      stroke: st.colorVar,
      "stroke-width": 1.6,
      class: `sci-graph-edge sci-graph-edge-${e.kind}`,
    });
    path.style.strokeWidth = "min(calc(1.6px * var(--sg-z-inv, 1)), 10px)";
    if (st.dash) path.setAttribute("stroke-dasharray", st.dash);
    svg.appendChild(path);
  }

  // 札（一番上）
  for (const c of layout.cards) sgWorld.appendChild(sgCardEl(c));
  sgMarkSelected(sgSelectedKey);

  // 全体表示（最初だけ。画面の大きさが分かるときだけ動かす）
  if (!sgFitted) {
    if (!sgFit() && typeof requestAnimationFrame === "function") {
      requestAnimationFrame(() => sgFit());
    }
  }
  sgApplyView();
}

/** 選んでいる札に枠の色を付ける（id が無い・null なら外す）。 */
function sgMarkSelected(id) {
  sgSelectedKey = id == null ? null : String(id);
  if (!sgWorld || typeof sgWorld.querySelectorAll !== "function") return;
  for (const el of sgWorld.querySelectorAll("[data-sg-id]")) {
    el.classList.toggle("is-selected", el.getAttribute("data-sg-id") === sgSelectedKey);
  }
}

/** 線の SVG に viewBox を付ける（描きたい範囲いっぱい）。 */
function sgSetViewBox(svg) {
  let minX = Infinity, minY = Infinity, maxX = -Infinity, maxY = -Infinity;
  for (const r of [...sgRects.values(), ...sgExtraRects]) {
    minX = Math.min(minX, r.x);
    minY = Math.min(minY, r.y);
    maxX = Math.max(maxX, r.x + r.width);
    maxY = Math.max(maxY, r.y + r.height);
  }
  if (!Number.isFinite(minX)) { minX = 0; minY = 0; maxX = 1; maxY = 1; }
  const pad = 40;
  minX -= pad; minY -= pad; maxX += pad; maxY += pad;
  svg.setAttribute("viewBox", `${sgR1(minX)} ${sgR1(minY)} ${sgR1(maxX - minX)} ${sgR1(maxY - minY)}`);
  svg.setAttribute("width", String(sgR1(maxX - minX)));
  svg.setAttribute("height", String(sgR1(maxY - minY)));
  svg.style.left = `${sgR1(minX)}px`;
  svg.style.top = `${sgR1(minY)}px`;
}

/** 札 1 枚。押すと onSelect が呼ばれる。 */
function sgCardEl(c) {
  const b = sgEl("button", `sci-graph-card sci-graph-${c.kind}`);
  b.type = "button";
  b.setAttribute("data-sg-id", c.id);
  b.style.left = `${sgR1(c.x)}px`;
  b.style.top = `${sgR1(c.y)}px`;
  b.style.width = `${c.w}px`;
  b.style.height = `${c.h}px`;
  if (c.starred) b.classList.add("sci-graph-starred");
  if (c.dim) b.classList.add("sci-graph-dim");
  const head = sgEl("span", "sci-graph-card-head");
  head.appendChild(sgEl("span", "sci-graph-card-id", c.kind === "S" ? "SEED" : c.id));
  b.appendChild(head);
  b.appendChild(sgEl("span", "sci-graph-card-title", c.text || "（文のない札）"));
  if (c.sub) b.appendChild(sgEl("span", "sci-graph-card-foot", c.sub));
  return b;
}

/** 凡例（図の下）。 */
function sgLegend() {
  const box = sgEl("div", "sci-graph-legend");
  box.appendChild(sgEl("span", "sci-graph-legend-title", "線"));
  for (const [key, label] of SG_EDGE_KINDS) {
    const item = sgEl("span", "sci-graph-legend-item");
    const line = sgEl("span", "sci-graph-legend-line");
    line.style.setProperty("--sg-color", sgEdgeStyle(key).colorVar);
    if (sgEdgeStyle(key).dash) line.classList.add("dotted");
    item.appendChild(line);
    item.appendChild(sgEl("span", null, label));
    box.appendChild(item);
  }
  return box;
}

// --- 見える範囲の操作 ---

/** 見える範囲を transform にする。 */
function sgApplyView() {
  if (!sgWorld) return;
  sgWorld.style.transform = `translate(${sgView.tx}px, ${sgView.ty}px) scale(${sgView.z})`;
  sgWorld.style.setProperty("--sg-z", String(sgView.z));
  sgWorld.style.setProperty("--sg-z-inv", String(sgZoomInv(sgView.z)));
}

/** 拡大率の逆数（0 にならないように 1 を下限にする）。 */
function sgZoomInv(z) {
  const n = Number(z);
  if (!Number.isFinite(n) || n <= 0) return 1;
  return 1 / n;
}

/**
 * 図を全体表示する（画面の中が全部入る）。
 * 画面の大きさがまだ分からないときは false を返して、やり直さない。
 */
function sgFit() {
  if (!sgStage || !(sgRects.size || sgExtraRects.length)) return false;
  const w = sgStage.clientWidth || 0;
  const h = sgStage.clientHeight || 0;
  if (w <= 0 || h <= 0) return false;
  const rects = [...sgRects.values(), ...sgExtraRects].map((r) => ({ x: r.x, y: r.y, width: r.width, height: r.height }));
  if (typeof cvFitView === "function") {
    sgView = cvFitView(rects, w, h);
    // 札が多い樹は、共通の下限（0.1）では全体が入らず左右が切れる。入りきるまで下限を下げる
    const xs = rects.flatMap((r) => [r.x, r.x + r.width]);
    const ys = rects.flatMap((r) => [r.y, r.y + r.height]);
    const bw = Math.max(1, Math.max(...xs) - Math.min(...xs));
    const bh = Math.max(1, Math.max(...ys) - Math.min(...ys));
    const z = Math.max(0.02, Math.min(sgView.z, (w - 80) / bw, (h - 80) / bh));
    if (z < sgView.z) {
      sgView = { tx: (w - bw * z) / 2 - Math.min(...xs) * z, ty: (h - bh * z) / 2 - Math.min(...ys) * z, z };
    }
  } else {
    // cvFitView が無いときは、等倍の左上を起点にする
    sgView = { tx: 0, ty: 0, z: 1 };
  }
  sgFitted = true;
  sgApplyView();
  return true;
}

/** 画面の中央を中心に拡大・縮小する。 */
function sgZoom(factor) {
  if (!sgStage) return;
  const w = sgStage.clientWidth || 0;
  const h = sgStage.clientHeight || 0;
  if (w <= 0 || h <= 0) return;
  sgZoomAtPx(w / 2, h / 2, factor);
}

/** (px, py) の下の世界上の点が動かないように、見える範囲を変える。 */
function sgZoomAtPx(px, py, factor) {
  if (typeof cvZoomAt === "function") {
    sgView = cvZoomAt(sgView, px, py, factor);
  } else {
    const z0 = Math.max(0.1, Math.min(2.5, Number(sgView.z) || 1));
    const z = Math.max(0.1, Math.min(2.5, z0 * (Number(factor) || 1)));
    const wx = (px - sgView.tx) / z0;
    const wy = (py - sgView.ty) / z0;
    sgView = { tx: px - wx * z, ty: py - wy * z, z };
  }
  sgApplyView();
}

/** 押している指から節点 id を取り出す（無ければ null）。 */
function sgHitNode(target) {
  if (!target || typeof target.closest !== "function") return null;
  const box = target.closest("[data-sg-id]");
  if (!box) return null;
  return box.getAttribute("data-sg-id");
}

function sgOnPointerDown(e) {
  if (!sgStage) return;
  try { sgStage.setPointerCapture(e.pointerId); } catch { /* 捕まえなくてもよい */ }
  sgPtrs.set(e.pointerId, { x: e.clientX, y: e.clientY });

  if (sgPtrs.size === 2) {
    const [p, q] = [...sgPtrs.values()];
    const rect = sgStage.getBoundingClientRect();
    sgPinch = {
      dist: Math.hypot(q.x - p.x, q.y - p.y) || 1,
      z0: sgView.z,
      wx: ((p.x + q.x) / 2 - rect.left - sgView.tx) / sgView.z,
      wy: ((p.y + q.y) / 2 - rect.top - sgView.ty) / sgView.z,
    };
    sgDown = null;
    return;
  }
  if (sgPtrs.size > 2) return;

  sgDown = {
    id: e.pointerId,
    sx: e.clientX,
    sy: e.clientY,
    view: { ...sgView },
    nodeId: sgHitNode(e.target),
    mode: null,
  };
}

function sgOnPointerMove(e) {
  if (!sgPtrs.has(e.pointerId)) return;
  sgPtrs.set(e.pointerId, { x: e.clientX, y: e.clientY });

  // 2 本指（拡大）
  if (sgPtrs.size >= 2 && sgPinch) {
    const [p, q] = [...sgPtrs.values()];
    const dist = Math.hypot(q.x - p.x, q.y - p.y) || 1;
    const rect = sgStage ? sgStage.getBoundingClientRect() : { left: 0, top: 0 };
    const z = Math.max(0.1, Math.min(2.5, sgPinch.z0 * (dist / sgPinch.dist)));
    sgView = {
      tx: (p.x + q.x) / 2 - rect.left - sgPinch.wx * z,
      ty: (p.y + q.y) / 2 - rect.top - sgPinch.wy * z,
      z,
    };
    sgApplyView();
    return;
  }

  const d = sgDown;
  if (!d || d.id !== e.pointerId) return;
  const dx = e.clientX - d.sx;
  const dy = e.clientY - d.sy;
  if (!d.mode) {
    if (Math.hypot(dx, dy) < SG_DRAG_SLOP) return; // クリックか移動か決まらない
    d.mode = "pan";
  }
  sgView = { tx: d.view.tx + dx, ty: d.view.ty + dy, z: d.view.z };
  sgApplyView();
}

function sgOnPointerUp(e) {
  sgPtrs.delete(e.pointerId);
  if (sgPtrs.size < 2) sgPinch = null;
  if (sgStage) {
    try { sgStage.releasePointerCapture(e.pointerId); } catch { /* 解放できなくてもよい */ }
  }
  const d = sgDown;
  if (!d || d.id !== e.pointerId) {
    if (sgPtrs.size === 0) sgDown = null;
    return;
  }
  sgDown = null;
  if (e.type === "pointercancel") return; // 途中で指が離れた
  if (d.mode) return;                     // 動かした（移動扱い）

  // 動かしていない -> 札を押した
  if (!d.nodeId) return;
  const sel = sgSelMap.get(d.nodeId);
  if (sel && typeof sgOnSelect === "function") sgOnSelect(sel);
}

/** ホイール（縦・横・Ctrl で拡大）。 */
function sgOnWheel(e) {
  if (!sgStage) return;
  e.preventDefault();
  const rect = sgStage.getBoundingClientRect();
  const px = e.clientX - rect.left;
  const py = e.clientY - rect.top;
  if (e.ctrlKey || e.metaKey) {
    sgZoomAtPx(px, py, Math.exp(-e.deltaY * 0.01));
  } else {
    sgView = {
      tx: sgView.tx - (e.shiftKey ? e.deltaY : (e.deltaX || 0)),
      ty: sgView.ty - (e.shiftKey ? 0 : e.deltaY),
      z: sgView.z,
    };
    sgApplyView();
  }
}

/**
 * 空の案内を「見える範囲の所」（sgStage）に入れる。
 * 世界（sgWorld）は 0 幅なので、そこに絶対配置の箱を置くと文字が縦に並ぶ。
 * 案内だけ世界から出して、箱ごと作り直す。
 */
function sgPutEmpty(box) {
  if (!sgStage) return;
  sgStage.innerHTML = "";
  sgWorld = sgEl("div", "sci-graph-world");
  const wrap = sgEl("div", "sci-graph-empty");
  wrap.appendChild(box);
  sgStage.appendChild(sgWorld);
  sgStage.appendChild(wrap);
}

/** 空の案内（ui.js の saiEmpty が無ければただの文にする）。 */
function sgEmptyBox(opts) {
  const o = opts || {};
  if (typeof saiEmpty === "function") {
    return saiEmpty({ icon: o.icon, title: o.title, body: o.body });
  }
  const box = sgEl("div", "sai-empty");
  if (o.title) box.appendChild(sgEl("p", "sai-empty-title", o.title));
  if (o.body) box.appendChild(sgEl("p", "sai-empty-body", o.body));
  return box;
}

// --- node からの読み込み（tests/test_scigraph.js） ---
if (typeof module !== "undefined" && module.exports) {
  module.exports = {
    sgRenderGraph,
    sgTreeLayout,
    sgSelectionOf,
    sgMarkSelected,
    sgEdgeStyle,
    sgEdgePath,
    sgObjBar,
    SG_CARD_W,
    SG_CARD_H,
    SG_TREE_STEP,
    SG_EDGE_KINDS,
  };
}
