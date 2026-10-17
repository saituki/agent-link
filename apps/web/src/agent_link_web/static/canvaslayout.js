"use strict";
// 図の配置・全体表示・拡大の純関数（研究方針の図と Workflow が使う）。
// 読み込みは index.html で行う（workflow.js と scigraph.js より先に読む）。
//
// 取り決め（cvlLayoutBoard）:
//   cvlLayoutBoard(nodes, edges) → [{id, x, y, width, height}, …]
//     nodes: {id, type: "card"|"group"|"text", x, y, width, height, group: string|null}
//     edges: {from, to}
//   ・同じ入力にはいつも同じ結果。入力を書き換えない。
//   ・札の width / height は変えない。枠の width / height は中身に合わせて変える。
//
// cvFitView と cvZoomAt は、見える範囲（画面上の点 = 図の点 * z + (tx, ty)）を
// 全体表示と拡大のための純関数。どちらの画面も同じ 2 つを呼んで同じ動きにする。

// --- 定数（計画 U05「定数」） ---
const CVL_GAP_X = 160; // 層と層の間の横の間隔
const CVL_GAP_Y = 60; // 同じ層の札の縦の間隔
const CVL_PAD = 40; // 枠の内側の余白
const CVL_HEAD = 60; // 枠の見出しの高さ
const CVL_CLUSTER_GAP = 200; // 枠と枠の横の間隔
const CVL_BELOW_GAP = 240; // 枠の段と、枠の外の札の段の間隔
const CVL_GRID_COLS = 4; // 線の無い札を並べる列の数
const CVL_SWEEPS = 2; // 重心を動かす往復回数
const CVL_MIN_GROUP_W = 400; // 中身が無い枠の最小の幅
const CVL_MIN_GROUP_H = 200; // 中身が無い枠の最小の高さ
const CVL_ZOOM_MIN = 0.1; // 拡大率の下限（これより外へは出さない）
const CVL_ZOOM_MAX = 2.5; // 拡大率の上限（これより外へは出さない）
const CVL_FIT_PAD = 40; // 「全体が入る」ときの余白（画面上の px）

// --- 小さな道具 ---

// id の昇順（数値の id は数値として比べる。文字列の id は辞書順）
function cvlCmpId(a, b) {
  if (typeof a === "number" && typeof b === "number") return a - b;
  const x = String(a);
  const y = String(b);
  if (x < y) return -1;
  if (x > y) return 1;
  return 0;
}

// 「layout.x を id で引けるようにする」ための一次元の連想配列（Map でなく plain を使う）
function cvlIndexed(list) {
  const map = new Map();
  for (const item of list) map.set(item.id, item);
  return map;
}

// 同じ値のときの決め方（値の後に今の順番、その後に id を見る）
function cvlOrderByValue(items) {
  return items
    .slice()
    .sort((a, b) => {
      if (a.value !== b.value) return a.value - b.value;
      if (a.secondary !== b.secondary) return a.secondary - b.secondary;
      return cvlCmpId(a.node.id, b.node.id);
    })
    .map((e) => e.node);
}

/**
 * 同じまとまりの札どうしを結ぶ線だけを取り出す。
 * 自分自身への線と、from と to が同じで重複した線は捨てる。
 * @param ids まとまりの札の id（Map）
 * @param edges すべての線
 * @returns 整形済みの線の配列
 */
function cvlInnerEdges(ids, edges) {
  const seen = new Set();
  const list = [];
  for (const edge of edges) {
    if (!edge || edge.from === edge.to) continue;
    if (!ids.has(edge.from) || !ids.has(edge.to)) continue;
    const key = String(edge.from) + "\u0000" + String(edge.to);
    if (seen.has(key)) continue;
    seen.add(key);
    list.push({ from: edge.from, to: edge.to });
  }
  // 入力の線の並びで結果が変わらないよう、from と to の id で並べる
  list.sort((a, b) => cvlCmpId(a.from, b.from) || cvlCmpId(a.to, b.to));
  return list;
}

/**
 * つながった札に層（0 始まり）を付ける。輪は id の昇順の深さ優先で切る。
 * @returns 層ごとの札の配列（層 0 からの昇順）
 */
function cvlAssignLayers(nodes, edges) {
  const out = new Map(); // id → 行先の id の配列
  const ins = new Map(); // id → 行きの id の配列
  for (const node of nodes) {
    out.set(node.id, []);
    ins.set(node.id, []);
  }
  for (const edge of edges) {
    out.get(edge.from).push(edge.to);
    ins.get(edge.to).push(edge.from);
  }

  // 輪を切る（深さ優先・帰りがけ順が層の計算の順番になる）
  const state = new Map(); // id → 0: たどっている途中 / 1: 済み
  const back = new Set(); // "from\u0000to"
  const order = [];
  for (const node of nodes) {
    if (state.has(node.id)) continue;
    state.set(node.id, 0);
    const stack = [{ id: node.id, i: 0 }];
    while (stack.length) {
      const top = stack[stack.length - 1];
      const targets = out.get(top.id);
      if (top.i < targets.length) {
        const next = targets[top.i];
        top.i += 1;
        const st = state.get(next);
        if (st === undefined) {
          state.set(next, 0);
          stack.push({ id: next, i: 0 });
        } else if (st === 0) {
          back.add(String(top.id) + "\u0000" + String(next));
        }
      } else {
        state.set(top.id, 1);
        order.push(top.id);
        stack.pop();
      }
    }
  }

  // 層を決める（後ろ向きの線を除いた DAG の最長路）
  // 帰りがけ順を逆にたどると「行きの元の層が決まっている」順番になる
  const layer = new Map();
  for (let i = order.length - 1; i >= 0; i -= 1) {
    const id = order[i];
    let value = 0;
    for (const from of ins.get(id)) {
      const key = String(from) + "\u0000" + String(id);
      if (back.has(key)) continue;
      const next = (layer.get(from) || 0) + 1;
      if (next > value) value = next;
    }
    layer.set(id, value);
  }
  let maxLayer = 0;
  for (const value of layer.values()) if (value > maxLayer) maxLayer = value;
  const layers = [];
  for (let k = 0; k <= maxLayer; k += 1) layers.push([]);
  for (const node of nodes) layers[layer.get(node.id)].push(node);
  for (const bucket of layers) bucket.sort((a, b) => cvlCmpId(a.id, b.id));
  return layers;
}

/**
 * 層の中の順番を決める。最初は id の昇順、下向きと上向きを往復で動かす。
 * 重心＝隣の層にいる相手の順番の平均。相手が無い札は今の順番の値を使う。
 */
function cvlOrderLayers(layers, edges) {
  const pos = new Map(); // id → その札の今の順番
  const refresh = () => {
    for (const bucket of layers) bucket.forEach((node, i) => pos.set(node.id, i));
  };
  refresh();

  const sweep = (from, to, step) => {
    for (let k = from; step > 0 ? k <= to : k >= to; k += step) {
      const bucket = layers[k];
      const values = bucket.map((node) => {
        const ns = pos.get(node.id);
        const neighbors = edges
          .filter((e) => (step > 0 ? e.to === node.id : e.from === node.id))
          .map((e) => (step > 0 ? e.from : e.to))
          .filter((id) => pos.has(id));
        if (!neighbors.length) return ns;
        let sum = 0;
        for (const id of neighbors) sum += pos.get(id);
        return sum / neighbors.length;
      });
      const items = bucket.map((node, i) => ({ node, value: values[i], secondary: pos.get(node.id) }));
      layers[k] = cvlOrderByValue(items);
      refresh();
    }
  };

  const last = layers.length - 1;
  for (let s = 0; s < CVL_SWEEPS; s += 1) {
    sweep(1, last, 1); // 下向き
    if (last >= 1) sweep(last - 1, 0, -1); // 上向き
  }
  return layers;
}

/**
 * まとまりの中（相対座標・左上を (0, 0) とする）に札を並べる。
 * @param nodes まとまりの札の配列
 * @param edges すべての線
 * @returns {placed: Map(id → {x, y, width, height}), contentW, contentH}（左上を (0, 0) とした相対座標）
 */
function cvlLayoutCluster(input, edges) {
  // 入力の並び順に依存しないよう、最初に id の昇順に並べる
  const nodes = input.slice().sort((a, b) => cvlCmpId(a.id, b.id));
  const ids = cvlIndexed(nodes);
  const inner = cvlInnerEdges(ids, edges);
  const placed = new Map(); // id → 位置

  // 1 本も繋がりの無い札（ひとりの札）と、繋がった札
  const touch = new Set();
  for (const edge of inner) {
    touch.add(edge.from);
    touch.add(edge.to);
  }
  const linked = nodes.filter((n) => touch.has(n.id));
  const solo = nodes.filter((n) => !touch.has(n.id));

  let contentW = 0;
  let contentH = 0;

  if (linked.length) {
    const layers = cvlAssignLayers(linked, inner);
    cvlOrderLayers(layers, inner);
    // 層ごとの幅・高さ・x
    const widths = layers.map((bucket) => bucket.reduce((m, n) => Math.max(m, n.width || 0), 0));
    const xs = [];
    let x = 0;
    for (let k = 0; k < layers.length; k += 1) {
      xs.push(x);
      x += widths[k] + CVL_GAP_X;
    }
    // 高さ（各層の合計）と中央そろえ
    const heights = layers.map((bucket) => {
      if (!bucket.length) return 0;
      return bucket.reduce((m, n) => m + (n.height || 0), 0) + CVL_GAP_Y * (bucket.length - 1);
    });
    const maxH = heights.reduce((m, h) => Math.max(m, h), 0);
    for (let k = 0; k < layers.length; k += 1) {
      const shift = Math.round((maxH - heights[k]) / 2);
      let y = shift;
      for (const node of layers[k]) {
        placed.set(node.id, { x: xs[k], y, width: node.width || 0, height: node.height || 0 });
        y += (node.height || 0) + CVL_GAP_Y;
      }
    }
    // 右端は一番広い層が決める（最後の層が最大とは限らない）
    contentW = 0;
    for (let k = 0; k < layers.length; k += 1) contentW = Math.max(contentW, xs[k] + widths[k]);
    contentH = maxH;
  }

  // ひとりの札はつながった札の下に格子で並べる
  if (solo.length) {
    const top = linked.length ? contentH + CVL_GAP_Y * 2 : 0;
    const colW = solo.reduce((m, n) => Math.max(m, n.width || 0), 0) + CVL_GAP_X;
    for (let i = 0; i < solo.length; i += 1) {
      const col = i % CVL_GRID_COLS;
      const row = Math.floor(i / CVL_GRID_COLS);
      placed.set(solo[i].id, {
        x: col * colW,
        y: top,
        width: solo[i].width || 0,
        height: solo[i].height || 0,
        row,
        col,
      });
    }
    // 行ごとの高さを合計する
    const rowHeights = new Map();
    for (const node of solo) {
      const p = placed.get(node.id);
      const h = p.height || 0;
      if (!rowHeights.has(p.row)) rowHeights.set(p.row, h);
      else if (h > rowHeights.get(p.row)) rowHeights.set(p.row, h);
    }
    const rows = [...rowHeights.keys()].sort((a, b) => a - b);
    const rowTop = new Map();
    let y = top;
    for (const row of rows) {
      rowTop.set(row, y);
      y += rowHeights.get(row) + CVL_GAP_Y;
    }
    for (const node of solo) {
      const p = placed.get(node.id);
      p.y = rowTop.get(p.row);
      delete p.row;
      delete p.col;
    }
    // 右端はどの札もはみ出さないよう、札を全部見て求める
    let gridW = 0;
    for (const node of solo) {
      const p = placed.get(node.id);
      gridW = Math.max(gridW, p.x + (p.width || 0));
    }
    contentW = Math.max(contentW, gridW);
    contentH = Math.max(contentH, y - CVL_GAP_Y);
  }

  return { placed, contentW, contentH };
}

/**
 * Canvas の札を自動的に並べる。
 * @param nodes 配列。各要素 {id, type: "card"|"group"|"text", x, y, width, height, group: string|null}
 *              （C-C の show の nodes をそのまま渡してよい。余分な欄は無視する）
 * @param edges 配列。各要素 {from, to}（C-C の edges をそのまま渡してよい）
 * @returns 配列。各要素 {id, x, y, width, height}（入力のすべてのノードについて 1 つずつ・入力の並び順）
 */
function cvlLayoutBoard(nodes, edges) {
  const input = Array.isArray(nodes) ? nodes : [];
  const allEdges = Array.isArray(edges) ? edges : [];

  // 1. まとまりに分ける
  const groups = input.filter((n) => n && n.type === "group");
  const groupIds = new Set(groups.map((g) => g.id));
  const memberOf = (node) => {
    if (node.group === null || node.group === undefined) return null;
    return groupIds.has(node.group) ? node.group : null;
  };
  const clusters = new Map(); // group id → 札の配列
  const outside = [];
  for (const node of input) {
    if (!node || node.type === "group") continue;
    const g = memberOf(node);
    if (g === null) outside.push(node);
    else {
      if (!clusters.has(g)) clusters.set(g, []);
      clusters.get(g).push(node);
    }
  }

  const pos = new Map(); // id → {x, y, width, height}
  const placedGroups = []; // 枠の結果（x, y, width, height）

  // 2・3. 枠のまとまりを横一列に置く
  const ordered = groups
    .map((g) => g)
    .sort((a, b) => (a.x || 0) - (b.x || 0) || cvlCmpId(a.id, b.id));
  let cursor = 0;
  for (const group of ordered) {
    const members = clusters.get(group.id) || [];
    if (!members.length) {
      // 中身が無い枠は元の大きさ（ただし最小値はある）
      const w = Math.max(group.width || 0, CVL_MIN_GROUP_W);
      const h = Math.max(group.height || 0, CVL_MIN_GROUP_H);
      pos.set(group.id, { x: cursor, y: 0, width: w, height: h });
      placedGroups.push({ x: cursor, y: 0, width: w, height: h });
    } else {
      const laid = cvlLayoutCluster(members, allEdges);
      const w = laid.contentW + CVL_PAD * 2;
      const h = laid.contentH + CVL_HEAD + CVL_PAD;
      for (const node of members) {
        const p = laid.placed.get(node.id);
        pos.set(node.id, {
          x: cursor + CVL_PAD + p.x,
          y: CVL_HEAD + p.y,
          width: p.width,
          height: p.height,
        });
      }
      pos.set(group.id, { x: cursor, y: 0, width: w, height: h });
      placedGroups.push({ x: cursor, y: 0, width: w, height: h });
    }
    cursor += pos.get(group.id).width + CVL_CLUSTER_GAP;
  }

  // 枠の外のまとまりは、枠の段の下に置く（枠が無ければ (0, 0)）
  if (outside.length) {
    const bottom = placedGroups.reduce((m, g) => Math.max(m, g.y + g.height), 0);
    const top = placedGroups.length ? bottom + CVL_BELOW_GAP : 0;
    const laid = cvlLayoutCluster(outside, allEdges);
    for (const node of outside) {
      const p = laid.placed.get(node.id);
      pos.set(node.id, {
        x: p.x,
        y: top + p.y,
        width: p.width,
        height: p.height,
      });
    }
  }

  // 4. 入力の並び順で返す（座標はすべて整数）
  return input.map((node) => {
    const p = pos.get(node.id);
    return {
      id: node.id,
      x: Math.round(p.x),
      y: Math.round(p.y),
      width: Math.round(p.width),
      height: Math.round(p.height),
    };
  });
}

// ============================================================================
// 見える範囲（全体表示と拡大）
// ============================================================================

/**
 * 全部の矩形が入る見える範囲を返す。z は 1 を超えない。
 * 矩形が無いときは {tx: 0, ty: 0, z: 1}。
 * @param nodes 配列。各要素 {x, y, width, height}
 * @param width 画面の幅（画面上の px）
 * @param height 画面の高さ（画面上の px）
 * @returns {tx, ty, z}（画面上の点 = 図の点 * z + (tx, ty)）
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
  const availW = Math.max(40, Number(width) - CVL_FIT_PAD * 2);
  const availH = Math.max(40, Number(height) - CVL_FIT_PAD * 2);
  let z = Math.min(1, availW / bw, availH / bh);
  z = Math.max(CVL_ZOOM_MIN, Math.min(CVL_ZOOM_MAX, z));
  return {
    tx: (Number(width) - bw * z) / 2 - minX * z,
    ty: (Number(height) - bh * z) / 2 - minY * z,
    z,
  };
}

/**
 * (px, py) の下の図の点が動かないように拡大率を変える。
 * 新しい見える範囲を返す。z は CVL_ZOOM_MIN〜CVL_ZOOM_MAX に収まる。
 * @param view 今の見える範囲 {tx, ty, z}
 * @param px, py 画面の点（画面上の px）
 * @param factor 拡大する率（1.2 なら 1.2 倍）
 */
function cvZoomAt(view, px, py, factor) {
  const v = view || {};
  const z0 = Math.max(CVL_ZOOM_MIN, Math.min(CVL_ZOOM_MAX, Number(v.z) || 1));
  const tx0 = Number(v.tx) || 0;
  const ty0 = Number(v.ty) || 0;
  const x = Number(px) || 0;
  const y = Number(py) || 0;
  let z = z0 * (Number(factor) || 1);
  z = Math.max(CVL_ZOOM_MIN, Math.min(CVL_ZOOM_MAX, z));
  const wx = (x - tx0) / z0;
  const wy = (y - ty0) / z0;
  return { tx: x - wx * z, ty: y - wy * z, z };
}

// --- node からの読み込み（tests/test_canvaslayout.js） ---
if (typeof module !== "undefined" && module.exports) {
  module.exports = { cvlLayoutBoard, cvFitView, cvZoomAt };
}
