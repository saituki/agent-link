"use strict";
// research-tree の図（実装計画 U20）。一番上の Q から下へ伸びる Q・A・F・E・S の木 1 つ。
//
//   sgRenderGraph(container, {status, key}, onSelect) … 図を描く
//   sgTreeLayout({status}, opts)                       … 配置（純関数）
//   sgMarkSelected(id)                                 … 選んでいる札の印（Node の ID か S<n>）。SPROUT を選ぶと他を薄くする
//   sgMarkPicked(ids)                                  … SPROUT 作成中の PATH に入れた A の札に破線の印（ID の配列か Set）
//   sgSproutKeep(status, id)                           … SPROUT に含まれる ID の Set（SPROUT でなければ null）
//   sgCenterOn(id)                                     … その札を画面の中央に出す
//
// key は文字（プロジェクト名）。その図の最後の表示位置・倍率・F と A と E の出し入れ・畳んだ札を覚えて、
// もう一度開いたときに同じ状態で出す（画面の中の Map と localStorage の sai-sg-view:<key>）。
// onSelect には {kind: "node", id} / {kind: "seed", id} を渡す（札の data-sg-id と同じ ID）。
// 段は、下から ROOT（top_Q とその下の Q・A・F）→ SEED の札 → SPROUT（kind S）→ BRANCH（E）の順。
// 線は SEED → top_Q（seed）・SEED → SPROUT（sprout）・Parent / Child（tree）の 3 つ。
//
// 中の処理は Rust のコアが返した JSON だけを使う（apps/web は他の DB を開かない）。
// 札の大きさは固定。見える範囲（tx, ty, z）だけを動かし、札の位置は動かさない。
// 全体表示と拡大は canvaslayout.js の cvFitView / cvZoomAt を typeof で確かめて使う。
// 依存は足さない。札は HTML の div、線は同じ中の SVG。
//
// 一番外側の名前は sg で始める（test_globals.js が重なりを検査する）。

// --- 定数 ---

// 札の大きさ（世界上の px）
const SG_CARD_W = 140;
const SG_CARD_H = 180;    // 縦長（題を多く見せる）
const SG_TREE_STEP = 230; // 段と段の縦の間隔（札の上端どうし。H + 50 で線の矢印と札の印が入る）
const SG_TREE_GAP = 20;   // 同じ段の札と札の間
// この数を超える Node では、F を隠した状態で始める（見やすさのため）
const SG_MANY_NODES = 60;
// ドラッグとクリックの境（これ以上動いたらドラッグ、未満ならクリック）
const SG_DRAG_SLOP = 4;
// 拡大・縮小の釦を 1 回押したときの拡大率の倍率
const SG_ZOOM_STEP = 1.2;
// 位置・倍率・F と A・畳みを覚える localStorage の鍵の前
const SG_MEMO_PREFIX = "sai-sg-view:";
// localStorage へ書くのをまとめるミリ秒
const SG_MEMO_WAIT = 120;

// F の stance の値（support/deny/issue）→ 札の印（S/D/I）
const SG_STANCE_LETTER = [["support", "S"], ["deny", "D"], ["issue", "I"]];

// 線の種類。sgEdgeStyle がこの表を返す。
// [種類, 表示名, 破線, 矢印つきの実線か, 色の CSS 変数]
const SG_EDGE_KINDS = [
  ["tree", "Parent / Child", "", false, "--sci-edge-parent"],
  ["seed", "SEED → top_Q", "2 4", false, "--sci-edge-depends"],
  ["sprout", "SEED → SPROUT", "2 4", false, "--sci-edge-analogous"],
];

// --- 状態 ---

let sgStatus = null;   // 最後に描いた status の JSON
let sgSelMap = new Map(); // 札の id → onSelect に渡す sel
let sgSelectedKey = null; // 選んでいる札の id
let sgPicked = new Set(); // SPROUT の PATH に入れた A の id（再描画のあとも印を戻す）
let sgOnSelect = null; // 札を押したときに呼ぶ関数
let sgRoot = null;     // 図を囲む要素
let sgFBtn = null;     // F の出し入れの釦（文と色を sgDraw で更新する）
let sgABtn = null;     // A の出し入れの釦（同じ作り）
let sgEBtn = null;     // E の出し入れの釦（同じ作り）
let sgStage = null;    // 見える範囲を出す所
let sgWorld = null;    // その中の世界（transform で動かす）
let sgView = { tx: 0, ty: 0, z: 1 }; // 画面上の点 = 世界の点 * z + (tx, ty)
let sgHideF = false;   // F の札を隠すか
let sgHideA = false;   // A の札を隠すか
let sgHideE = false;   // E の札を隠すか
let sgCollapsed = new Set(); // 畳んだ札の id（その子孫を見ない）
let sgCollapsible = new Set(); // 子孫があるので畳める札の id
let sgRects = new Map();  // 札の id → 置いた場所（線と札の共通）
let sgExtraRects = [];    // 見える範囲の算出に入れる四角（今は使わない。sgFit・sgSetViewBox が読む）
let sgPtrs = new Map();  // 押している指（1 本 = 移動、2 本 = 拡大）
let sgDown = null;       // 1 本指で押したままの情報
let sgPinch = null;      // 2 本指の情報
let sgFitted = false;    // 全体表示を 1 回はしたか
let sgKey = null;        // 今の図の記憶の入れ物（プロジェクト名。無いときは記憶しない）
let sgMemo = new Map();  // key → {view, hideF, hideA, hideE, collapsed}
let sgMemoTimer = null;  // localStorage へ書くのをまとめる計時
let sgMemoPending = null; // localStorage に書く、まだ書いていない最新の物
let sgKeysBound = false; // document の keydown を 1 回だけ付けたか

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

/** F の stance（support/deny/issue）の印（S/D/I）。違う値は空文字。 */
function sgStanceLetter(v) {
  const hit = SG_STANCE_LETTER.find((s) => s[0] === v);
  return hit ? hit[1] : "";
}

/** 文を n 字で切る。 */
function sgClip(text, n) {
  const s = String(text == null ? "" : text);
  const a = Array.from(s);
  return a.length > n ? `${a.slice(0, n).join("")}…` : s;
}

/** 畳んだ札の入れ物を、配列でも Set でも同じ形にする。 */
function sgIdSet(v) {
  if (!v) return new Set();
  if (v instanceof Set) return new Set([...v].map(String));
  if (Array.isArray(v)) return new Set(v.filter((x) => x != null).map(String));
  return new Set();
}

/**
 * 木の配置。
 * @param input {status}
 * @param opts {hideF, hideA, hideE, collapsed}
 *   hideF … 真なら F の札を出さず、親の札の sub に `F <n>` を書く
 *   hideA … 真なら A の札を出さない
 *   hideE … 真なら E の札を出さない
 *   collapsed … 畳んだ札の id（Set でも配列でもよい）。その札の子孫を見ない
 * @returns {cards, edges, width, height}
 */
function sgTreeLayout(input, opts) {
  const status = (input && input.status) || {};
  const o = opts || {};
  const hideF = !!o.hideF;
  const hideA = !!o.hideA;
  const hideE = !!o.hideE;
  const collapsed = sgIdSet(o.collapsed);
  const all = (Array.isArray(status.nodes) ? status.nodes : []).filter((n) => n && n.id);
  const byId = new Map(all.map((n) => [String(n.id), n]));

  // 出さない種類（隠した F・A・E。隠した E の下の F・E は一番近い見える祖先につなぐ）
  const kindHidden = (n) => (hideF && n.kind === "F") || (hideA && n.kind === "A") || (hideE && n.kind === "E");

  // 畳んだ祖先があるか（自分自身は数えない。畳んだ札の自身は見えるので）
  const underCollapsed = (n) => {
    let p = n.parent == null ? null : byId.get(String(n.parent));
    const seen = new Set();
    while (p && !seen.has(String(p.id))) {
      if (collapsed.has(String(p.id))) return true;
      seen.add(String(p.id));
      p = p.parent == null ? null : byId.get(String(p.parent));
    }
    return false;
  };

  // 全部の子孫（元の木の中。種類を隠す前には出す）
  const allKids = new Map();
  for (const n of all) {
    const p = n.parent == null ? null : String(n.parent);
    if (p == null) continue;
    if (!allKids.has(p)) allKids.set(p, []);
    allKids.get(p).push(String(n.id));
  }
  const descCount = new Map(); // id → 畳みを無視したときに見える子孫の数
  for (const n of all) {
    let count = 0;
    const stack = [...(allKids.get(String(n.id)) || [])];
    const seen = new Set();
    while (stack.length) {
      const cur = stack.pop();
      if (seen.has(cur)) continue;
      seen.add(cur);
      const c = byId.get(cur);
      if (c && !kindHidden(c)) count += 1;
      for (const k of allKids.get(cur) || []) stack.push(k);
    }
    descCount.set(String(n.id), count);
  }
  // 畳んだ札の中の印（子孫が無いものは畳んだことにしない）
  const collapseMark = new Map();
  for (const id of collapsed) {
    const n = descCount.get(id);
    if (n != null && n > 0) collapseMark.set(id, n);
  }

  const shown = all.filter((n) => !kindHidden(n) && !underCollapsed(n));
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
      if (underCollapsed(n)) continue; // 畳んだ部分木の中の F は `F <n>` に入れない（`▸ +N` に入る）
      const p = visParent(n);
      if (p != null) fCount.set(p, (fCount.get(p) || 0) + 1);
    }
  }

  // SPROUT（kind S）とその子孫（BRANCH）を、ROOT（top_Q 以下）から分ける
  const branchIds = new Set();
  for (const n of shown) {
    let cur = n;
    const seen = new Set();
    while (cur && !seen.has(String(cur.id))) {
      seen.add(String(cur.id));
      if (cur.kind === "S") { branchIds.add(String(n.id)); break; }
      const pid = visParent(cur);
      cur = pid == null ? null : byId.get(pid);
    }
  }

  const pos = new Map(); // id → {x, y}
  const cards = [];
  const slotW = SG_CARD_W + SG_TREE_GAP;

  // ROOT: top_Q を row 0（y = +STEP）にして下へ。葉を左から詰め、親は子の真ん中
  let slot = 0;
  const placeDown = (n, row, guard) => {
    const id = String(n.id);
    const ch = guard.has(id) ? [] : kids.get(id) || [];
    const g2 = new Set(guard).add(id);
    let x;
    if (!ch.length) {
      x = slot * slotW;
      slot += 1;
    } else {
      const xs = ch.map((c) => placeDown(c, row + 1, g2));
      x = (xs[0] + xs[xs.length - 1]) / 2;
    }
    const y = (row + 1) * SG_TREE_STEP;
    pos.set(id, { x, y });
    cards.push(sgNodeCard(n, x, y, fCount.get(id) || 0, collapseMark.get(id) || 0, (descCount.get(id) || 0) > 0));
    return x;
  };
  const rootRoots = roots.filter((n) => n.kind !== "S");
  for (const r of rootRoots) placeDown(r, 0, new Set());

  // BRANCH: SPROUT を row 0（y = -STEP）にして上へ。葉を左から詰め、親は子の真ん中
  let bslot = 0;
  const placeUp = (n, row, guard) => {
    const id = String(n.id);
    const ch = guard.has(id) ? [] : kids.get(id) || [];
    const g2 = new Set(guard).add(id);
    let x;
    if (!ch.length) {
      x = bslot * slotW;
      bslot += 1;
    } else {
      const xs = ch.map((c) => placeUp(c, row + 1, g2));
      x = (xs[0] + xs[xs.length - 1]) / 2;
    }
    const y = -(row + 1) * SG_TREE_STEP;
    pos.set(id, { x, y });
    cards.push(sgNodeCard(n, x, y, fCount.get(id) || 0, collapseMark.get(id) || 0, (descCount.get(id) || 0) > 0));
    return x;
  };
  const sproutRoots = roots.filter((n) => n.kind === "S")
    .sort((a, b) => (String(a.id) < String(b.id) ? -1 : String(a.id) > String(b.id) ? 1 : 0));
  for (const s of sproutRoots) placeUp(s, 0, new Set());

  // 横の真ん中: ROOT の真ん中に SEED を置き、SPROUT の並びの真ん中を SEED にそろえる
  const spanOf = (ids) => {
    let min = Infinity, max = -Infinity;
    for (const id of ids) {
      const p = pos.get(id);
      if (!p) continue;
      min = Math.min(min, p.x);
      max = Math.max(max, p.x + SG_CARD_W);
    }
    return min === Infinity ? null : { min, max };
  };
  // SEED は ROOT（top_Q 以下）全体の真ん中に置き、SPROUT の並びの真ん中を SEED にそろえる
  const rootIds = [...pos.keys()].filter((id) => !branchIds.has(id));
  const rootSpan = spanOf(rootIds);
  const sproutSpan = spanOf(sproutRoots.map((n) => String(n.id)));
  const seedCenter = rootSpan
    ? (rootSpan.min + rootSpan.max) / 2
    : (sproutSpan ? (sproutSpan.min + sproutSpan.max) / 2 : 0);
  const seedX = seedCenter - SG_CARD_W / 2;
  if (sproutSpan) {
    const shift = seedCenter - (sproutSpan.min + sproutSpan.max) / 2;
    if (shift !== 0) {
      for (const c of cards) {
        if (!branchIds.has(String(c.id))) continue;
        c.x += shift;
        const p = pos.get(String(c.id));
        if (p) p.x += shift;
      }
    }
  }

  // SEED の札（DB の行を持たない。status.seed から作る）
  if (all.length) {
    const typeName = status.seed && status.seed.type && status.seed.type.name ? String(status.seed.type.name) : "";
    cards.push({
      id: "SEED", kind: "SEED", x: seedX, y: 0, w: SG_CARD_W, h: SG_CARD_H,
      text: "SEED", sub: typeName, starred: false, stance: "", collapsed: false, canCollapse: false,
    });
    pos.set("SEED", { x: seedX, y: 0 });
  }

  const edges = [];
  for (const n of shown) {
    const p = visParent(n);
    if (p != null && pos.has(p)) edges.push({ from: p, to: String(n.id), kind: "tree" });
  }
  if (pos.has("SEED")) {
    for (const n of rootRoots) edges.push({ from: "SEED", to: String(n.id), kind: "seed" });
    for (const n of sproutRoots) edges.push({ from: "SEED", to: String(n.id), kind: "sprout" });
  }

  const width = cards.reduce((m, c) => Math.max(m, c.x + c.w), 0);
  const minX = cards.reduce((m, c) => Math.min(m, c.x), 0);
  const minY = cards.reduce((m, c) => Math.min(m, c.y), 0);
  const height = cards.reduce((m, c) => Math.max(m, c.y + c.h), 0);
  return { cards, edges, width, height, minX, minY };
}

/** Node 1 つ分の札。hiddenF は隠した F の数、collapsedCount は畳んで消えた子孫の数。 */
function sgNodeCard(n, x, y, hiddenF, collapsedCount, canCollapse) {
  let text = n.text || "";
  let sub = "";
  const counts = n.counts || {};
  const f = hiddenF || Number(counts.f) || 0;
  // F の stance は文字（S・D・I）で出す（色だけに頼らない）。送る値は support・deny・issue
  const stance = n.kind === "F" ? sgStanceLetter(n.stance) : "";
  if (n.kind === "A") {
    sub = `${n.starred ? "★ " : ""}${f ? `F ${f}` : ""}`.trim();
  } else if (n.kind === "F") {
    text = sgClip(text, 40);
    sub = stance;
  } else if (n.kind === "E") {
    sub = n.mlflow ? String(n.mlflow) : "";
  } else if (n.kind === "S") {
    // SPROUT の札。題の前に ID を置き、副題に PATH の数を出す。
    const paths = Array.isArray(n.paths) ? n.paths.length : 0;
    text = `${n.id} ${text}`.trim();
    sub = paths > 0 ? `PATH ${paths}` : "";
  }
  if (hiddenF && n.kind !== "A") sub = `${sub} F ${hiddenF}`.trim();
  const collapsed = !!(canCollapse && collapsedCount > 0);
  if (collapsed) sub = `${sub} ▸ +${collapsedCount}`.trim();
  return {
    id: String(n.id), kind: n.kind, x, y, w: SG_CARD_W, h: SG_CARD_H,
    text, sub, starred: !!n.starred, stance,
    collapsed, canCollapse: !!canCollapse,
  };
}

/**
 * SPROUT（kind S）を選んだときに濃く残す ID の Set。SPROUT でなければ null。
 * 残すのは、その S 自身・S の下の E と F（BRANCH）・paths の終点の A から top_Q までの Q・A（PATH）。
 */
function sgSproutKeep(status, id) {
  const want = id == null ? "" : String(id);
  const all = (status && Array.isArray(status.nodes) ? status.nodes : []).filter((n) => n && n.id);
  const byId = new Map(all.map((n) => [String(n.id), n]));
  const sprout = byId.get(want);
  if (!sprout || sprout.kind !== "S") return null;
  const keep = new Set([want]);
  // BRANCH: parent をたどって want に行き着く Node（S の下の E・F）
  for (const n of all) {
    let p = n.parent == null ? null : String(n.parent);
    const seen = new Set();
    while (p != null && !seen.has(p)) {
      seen.add(p);
      if (p === want) { keep.add(String(n.id)); break; }
      const pn = byId.get(p);
      p = pn && pn.parent != null ? String(pn.parent) : null;
    }
  }
  // PATH: paths の終点の A から top_Q までの祖先（Q・A）
  const paths = Array.isArray(sprout.paths) ? sprout.paths : [];
  for (const pid of paths) {
    let cur = byId.get(String(pid));
    const seen = new Set();
    while (cur && !seen.has(String(cur.id))) {
      seen.add(String(cur.id));
      keep.add(String(cur.id));
      cur = cur.parent == null ? null : byId.get(String(cur.parent));
    }
  }
  return keep;
}

/** 札から onSelect に渡す値を作る。SEED は `seed`、S を含む Node は `node`。 */
function sgSelectionOf(card) {
  if (card.kind === "SEED") return { kind: "seed", id: "SEED" };
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
 * @param input {status, key} key は文字（プロジェクト名）。同じ key なら前回の位置と倍率で描く
 * @param onSelect 札を押したときに呼ぶ関数。引数は {kind: "node" | "seed", id}
 */
function sgRenderGraph(container, input, onSelect) {
  if (!container) return;
  container.innerHTML = "";
  sgStatus = (input && input.status) || null;
  sgOnSelect = typeof onSelect === "function" ? onSelect : null;
  sgPtrs = new Map();
  sgDown = null;
  sgPinch = null;

  // Node が多いときは、F を隠した状態で始める
  const nodes = sgStatus && Array.isArray(sgStatus.nodes) ? sgStatus.nodes : [];

  // 覚えていた表示（位置・倍率・F と A と E・畳み）。key が無いときは全体をそのまま出す
  sgKey = input && input.key ? String(input.key) : null;
  const memo = sgReadMemo(sgKey);
  if (memo) {
    sgView = { tx: memo.view.tx, ty: memo.view.ty, z: memo.view.z };
    sgHideF = memo.hideF;
    sgHideA = memo.hideA;
    sgHideE = memo.hideE;
    sgCollapsed = new Set(memo.collapsed);
    // 今の status に無い id は捨てる
    const ids = new Set(nodes.filter((n) => n && n.id).map((n) => String(n.id)));
    for (const id of [...sgCollapsed]) if (!ids.has(id)) sgCollapsed.delete(id);
    sgFitted = true; // 全体表示に戻さない
  } else {
    sgView = { tx: 0, ty: 0, z: 1 };
    sgFitted = false;
    sgHideF = nodes.length > SG_MANY_NODES;
    sgHideA = false;
    sgHideE = false;
    sgCollapsed = new Set();
  }

  sgRoot = sgEl("div", "sci-graph");

  // 1. 道具の行（F と A と E の出し入れ・拡大・縮小・全体を表示）
  const tools = sgEl("div", "sci-graph-tools sai-toolbar");
  sgFBtn = sgToggleBtn("F", "sg-btn-F", () => {
    sgHideF = !sgHideF;
    sgWriteMemo();
    sgDraw(sgSelectedKey);
  });
  sgABtn = sgToggleBtn("A", "sg-btn-A", () => {
    sgHideA = !sgHideA;
    sgWriteMemo();
    sgDraw(sgSelectedKey);
  });
  sgEBtn = sgToggleBtn("E", "sg-btn-E", () => {
    sgHideE = !sgHideE;
    sgWriteMemo();
    sgDraw(sgSelectedKey);
  });
  tools.appendChild(sgFBtn);
  tools.appendChild(sgABtn);
  tools.appendChild(sgEBtn);
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
  // macOS では ctrl+click で右クリックの一覧が出る。札の上だけ止める
  sgStage.addEventListener("contextmenu", (e) => {
    if (sgHitNode(e.target)) e.preventDefault();
  });
  sgBindKeys();

  // 3. 凡例
  sgRoot.appendChild(sgLegend());

  container.appendChild(sgRoot);
  sgDraw();
}

/**
 * 図の絵を描く（線・札）。
 * @param anchorId 描画後も画面上の位置に留めたい札の id（無い・消えたときは動かさない）
 */
function sgDraw(anchorId) {
  if (!sgWorld) return;
  // 描き直しの前の、その札の世界上の位置（画面上の位置に留めるため）
  const anchor = anchorId == null ? null : String(anchorId);
  const before = anchor ? sgRects.get(anchor) : null;
  const view0 = { ...sgView };
  const willFit = !sgFitted;
  sgUpdateToggles();
  sgWorld.innerHTML = "";
  sgRects = new Map();
  sgExtraRects = [];
  sgSelMap = new Map();
  sgCollapsible = new Set();

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

  const layout = sgTreeLayout({ status: sgStatus }, { hideF: sgHideF, hideA: sgHideA, hideE: sgHideE, collapsed: sgCollapsed });
  for (const c of layout.cards) {
    sgRects.set(c.id, { x: c.x, y: c.y, width: c.w, height: c.h });
    sgSelMap.set(c.id, sgSelectionOf(c));
    if (c.canCollapse) sgCollapsible.add(c.id);
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
      "data-from": e.from,
      "data-to": e.to,
    });
    path.style.strokeWidth = "min(calc(1.6px * var(--sg-z-inv, 1)), 10px)";
    if (st.dash) path.setAttribute("stroke-dasharray", st.dash);
    svg.appendChild(path);
  }

  // 札（一番上）
  for (const c of layout.cards) sgWorld.appendChild(sgCardEl(c));
  sgMarkSelected(sgSelectedKey);

  // 全体表示（記憶が無い最初の描画と ⤢ だけ）
  if (!sgFitted) {
    if (!sgFit() && typeof requestAnimationFrame === "function") {
      requestAnimationFrame(() => sgFit());
    }
  }

  // その札を画面上の同じ場所に留める（全体表示のとき・その札が消えたときは動かさない）
  const after = anchor ? sgRects.get(anchor) : null;
  if (!willFit && before && after) {
    sgView = {
      tx: view0.tx + (before.x - after.x) * sgView.z,
      ty: view0.ty + (before.y - after.y) * sgView.z,
      z: sgView.z,
    };
  }
  sgApplyView();
}

/** F・A・E の釦（文は 1 文字。出しているときだけ色を付ける）。 */
function sgToggleBtn(label, cls, onClick) {
  const b = sgEl("button", `sci-graph-btn ${cls}`);
  b.type = "button";
  b.textContent = label;
  b.addEventListener("click", onClick);
  return b;
}

/** F・A・E の釦の色と title を今の状態に合わせて更新する。 */
function sgUpdateToggles() {
  for (const [btn, hide, what] of [[sgFBtn, sgHideF, "F"], [sgABtn, sgHideA, "A"], [sgEBtn, sgHideE, "E"]]) {
    if (!btn) continue;
    btn.classList.toggle("on", !hide);
    btn.setAttribute("aria-pressed", hide ? "false" : "true");
    btn.title = hide ? `${what} の札を出します` : `${what} の札を隠します`;
    btn.setAttribute("aria-label", btn.title);
  }
}

/** 選んでいる札に枠の色を付ける（id が無い・null なら外す）。SPROUT を選ぶと、含まれない札と線を薄くする。 */
function sgMarkSelected(id) {
  sgSelectedKey = id == null ? null : String(id);
  if (!sgWorld || typeof sgWorld.querySelectorAll !== "function") return;
  const keep = sgSproutKeep(sgStatus, sgSelectedKey);
  for (const el of sgWorld.querySelectorAll("[data-sg-id]")) {
    const cardId = el.getAttribute("data-sg-id");
    el.classList.toggle("is-selected", cardId === sgSelectedKey);
    el.classList.toggle("is-picked", sgPicked.has(cardId));
    el.classList.toggle("sci-graph-faded", !!keep && !keep.has(cardId));
  }
  for (const el of sgWorld.querySelectorAll(".sci-graph-edge[data-from]")) {
    const from = el.getAttribute("data-from");
    const to = el.getAttribute("data-to");
    el.classList.toggle("sci-graph-faded", !!keep && !(keep.has(from) && keep.has(to)));
  }
}

/** SPROUT 作成中の PATH に入れた A の札に印を付ける（空なら外す）。 */
function sgMarkPicked(ids) {
  sgPicked = new Set(Array.from(ids || [], String));
  sgMarkSelected(sgSelectedKey);
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
  if (c.stance) b.classList.add(`sci-graph-stance-${c.stance}`);
  if (c.collapsed) b.classList.add("sci-graph-collapsed");
  const head = sgEl("span", "sci-graph-card-head");
  head.appendChild(sgEl("span", "sci-graph-card-id", c.kind === "S" ? "SPROUT" : c.id));
  b.appendChild(head);
  // SEED は見出しの SEED をそのまま出す（題は無い）
  if (c.kind !== "SEED") b.appendChild(sgEl("span", "sci-graph-card-title", c.text || "（文のない CARD）"));
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

// --- 表示位置・倍率の記憶（画面の中の Map と localStorage） ---
// 別の画面へ移ってから戻ったときや、↻・再読み込みのあとでも同じ図を出す。

/** 覚えている表示を返す。key が無い・読めない・壊れているときは null。 */
function sgReadMemo(key) {
  if (!key) return null;
  const hit = sgMemo.get(key);
  if (hit) return hit;
  let raw = null;
  try { raw = localStorage.getItem(SG_MEMO_PREFIX + key); } catch { return null; }
  if (!raw) return null;
  let o = null;
  try { o = JSON.parse(raw); } catch { return null; }
  if (!o || typeof o !== "object" || !o.view || typeof o.view !== "object") return null;
  const z = Number(o.view.z);
  const out = {
    view: { tx: Number(o.view.tx) || 0, ty: Number(o.view.ty) || 0, z: Number.isFinite(z) && z > 0 ? z : 1 },
    hideF: !!o.hideF,
    hideA: !!o.hideA,
    hideE: !!o.hideE,
    collapsed: Array.isArray(o.collapsed) ? o.collapsed.filter((x) => x != null).map(String) : [],
  };
  sgMemo.set(key, out);
  return out;
}

/** 今の表示（位置・倍率・F と A と E・畳み）を覚える。key が無いときは何もしない。 */
function sgWriteMemo() {
  if (!sgKey) return;
  // 画面の中ではすぐ次の読み出しに使う
  sgMemoPending = {
    key: sgKey,
    memo: {
      view: { tx: sgView.tx, ty: sgView.ty, z: sgView.z },
      hideF: sgHideF,
      hideA: sgHideA,
      hideE: sgHideE,
      collapsed: [...sgCollapsed],
    },
  };
  sgMemo.set(sgKey, sgMemoPending.memo);
  if (sgMemoTimer != null) return; // 連続した操作はまとめる
  sgMemoTimer = setTimeout(() => {
    sgMemoTimer = null;
    const p = sgMemoPending;
    sgMemoPending = null;
    if (!p) return;
    try { localStorage.setItem(SG_MEMO_PREFIX + p.key, JSON.stringify(p.memo)); } catch { /* 使えなくてもよい */ }
  }, SG_MEMO_WAIT);
}

/** 見える範囲を transform にする。 */
function sgApplyView() {
  if (!sgWorld) return;
  sgWorld.style.transform = `translate(${sgView.tx}px, ${sgView.ty}px) scale(${sgView.z})`;
  sgWorld.style.setProperty("--sg-z", String(sgView.z));
  sgWorld.style.setProperty("--sg-z-inv", String(sgZoomInv(sgView.z)));
  sgWorld.style.setProperty("--sg-text", String(sgTextScale(sgView.z)));
  sgWorld.style.setProperty("--sg-title-lines", String(sgTitleLines(sgView.z)));
  sgWriteMemo();
}

/**
 * 見える範囲の平行移動（その札を画面の中央に置く）。純関数。
 * @param rect 世界上の札（x, y, width, height）
 * @param w 画面の幅 / h 画面の高さ / z 今の拡大率
 */
function sgCenterView(rect, w, h, z) {
  const r = rect || {};
  const n = Number(z);
  const zz = Number.isFinite(n) && n > 0 ? n : 1;
  const cx = (Number(r.x) || 0) + (Number(r.width) || 0) / 2;
  const cy = (Number(r.y) || 0) + (Number(r.height) || 0) / 2;
  return { tx: (Number(w) || 0) / 2 - cx * zz, ty: (Number(h) || 0) / 2 - cy * zz, z: zz };
}

/**
 * その札を画面の中央に出す（拡大率は動かさない）。見つからなければ false。
 * 図に無い札（隠した F・A・畳んだ部分木）は、一番近い見える祖先を中央に出す。
 */
function sgCenterOn(id) {
  if (!sgStage || !sgStatus || id == null) return false;
  const w = sgStage.clientWidth || 0;
  const h = sgStage.clientHeight || 0;
  if (w <= 0 || h <= 0) return false;
  const want = String(id);
  let rect = sgRects.get(want);
  if (!rect) {
    // 図に無いときは、親をたどって一番近い見える祖先を探す
    const byId = new Map(
      (Array.isArray(sgStatus.nodes) ? sgStatus.nodes : []).filter((n) => n && n.id).map((n) => [String(n.id), n])
    );
    let p = (byId.get(want) || {}).parent;
    const seen = new Set([want]);
    while (p != null && !seen.has(String(p))) {
      seen.add(String(p));
      rect = sgRects.get(String(p));
      if (rect) break;
      p = (byId.get(String(p)) || {}).parent;
    }
  }
  if (!rect) return false;
  sgView = sgCenterView(rect, w, h, sgView.z);
  sgApplyView();
  return true;
}

/** 拡大率の逆数（0 にならないように 1 を下限にする）。 */
function sgZoomInv(z) {
  const n = Number(z);
  if (!Number.isFinite(n) || n <= 0) return 1;
  return 1 / n;
}

/**
 * 文字の倍率（純関数）。拡大（z > 1）では文字を大きくしない（画面の上で今の大きさのまま）。
 * 縮小（z < 1）は世界ごと小さくなるので、ここは 1 のまま。
 */
function sgTextScale(z) {
  const n = Number(z);
  if (!Number.isFinite(n) || n <= 0) return 1;
  return Math.min(1, 1 / n);
}

/**
 * 題を何行まで見せるか（純関数）。札の高さから決める（Card は縦長・札の外へ出さない）。
 * 13px の題（行の高さ 1.35）が、head（11px・1.5）と foot（1 行・12px・1.6）と上下の余白の残りに入る行数。
 */
function sgTitleLines(z) {
  const s = sgTextScale(z);
  const avail = SG_CARD_H - 12 - 11 * 1.5 * s - 12 * 1.6 * s;
  return Math.max(2, Math.floor(avail / (13 * 1.35 * s)));
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

/** 押している指から Node id を取り出す（無ければ null）。 */
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
    mod: !!(e.ctrlKey || e.metaKey),
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
  // ctrl+click・cmd+click は、子孫を畳む（onSelect は呼ばない）
  if (d.mod) {
    sgToggleCollapsed(d.nodeId);
    return;
  }
  const sel = sgSelMap.get(d.nodeId);
  if (sel && typeof sgOnSelect === "function") sgOnSelect(sel);
}

/** その札の子孫を畳む・広げる（子孫が無い札では何もしない）。 */
function sgToggleCollapsed(id) {
  const key = String(id);
  if (!sgCollapsible.has(key)) return;
  if (sgCollapsed.has(key)) sgCollapsed.delete(key);
  else sgCollapsed.add(key);
  sgWriteMemo();
  sgDraw(key); // その札は画面上の位置に留める
}

/** document に 1 回だけ付けるキー操作（+ / - で拡大・縮小）。 */
function sgBindKeys() {
  if (sgKeysBound) return;
  if (typeof document === "undefined" || typeof document.addEventListener !== "function") return;
  sgKeysBound = true;
  document.addEventListener("keydown", (e) => {
    // 図が画面に出ているときだけ
    if (!sgStage || !sgStage.isConnected || sgStage.offsetParent === null) return;
    if (e.ctrlKey || e.metaKey || e.altKey) return;
    const t = e.target;
    const tag = t && t.tagName ? String(t.tagName).toLowerCase() : "";
    if (tag === "input" || tag === "textarea" || tag === "select") return;
    if (t && t.isContentEditable) return;
    if (e.key === "+" || e.key === "=") {
      sgZoom(SG_ZOOM_STEP);
      e.preventDefault();
    } else if (e.key === "-") {
      sgZoom(1 / SG_ZOOM_STEP);
      e.preventDefault();
    }
  });
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
    sgMarkPicked,
    sgSproutKeep,
    sgCenterOn,
    sgCenterView,
    sgEdgeStyle,
    sgEdgePath,
    sgTextScale,
    sgTitleLines,
    SG_CARD_W,
    SG_CARD_H,
    SG_TREE_STEP,
    SG_EDGE_KINDS,
  };
}
