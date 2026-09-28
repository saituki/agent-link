"use strict";
// 図の配置・全体表示・拡大の純関数のテスト（cvlLayoutBoard・cvFitView・cvZoomAt）。
// 実行:
//     node tests/test_canvaslayout.js

const assert = require("assert");
const path = require("path");

const {
  cvlLayoutBoard,
  cvFitView,
  cvZoomAt,
} = require(path.join(
  __dirname,
  "..",
  "src",
  "sai_phone",
  "static",
  "canvaslayout.js"
));
// 拡大率の上下限（canvaslayout.js の CVL_ZOOM_MIN / CVL_ZOOM_MAX と同じ値。範囲は超えない）
const ZOOM_MIN = 0.1;
const ZOOM_MAX = 2.5;

// --- 試作用の入力を作る ---
function card(id, x, y, w, h, group) {
  return { id, type: "card", x, y, width: w, height: h, group: group === undefined ? null : group };
}
function groupNode(id, x, y, w, h) {
  return { id, type: "group", x, y, width: w, height: h, group: null };
}

// 12 枚の札・10 本の線・2 つの枠（g1 の中に輪がある）
function sample() {
  return {
    nodes: [
      card("a", 0, 0, 120, 40),
      card("b", 0, 0, 120, 40),
      card("c", 0, 0, 140, 60),
      card("d", 0, 0, 100, 30),
      card("e", 0, 0, 110, 50),
      card("f", 0, 0, 90, 30),
      card("h", 0, 0, 130, 40),
      card("i", 0, 0, 120, 40),
      card("j", 0, 0, 100, 30),
      card("k", 0, 0, 110, 40),
      card("m", 0, 0, 120, 40),
      card("n", 0, 0, 100, 50),
      groupNode("g1", 40, 40, 300, 200),
      groupNode("g2", 900, 40, 200, 150),
      card("x1", 0, 0, 120, 40, "g1"),
      card("x2", 0, 0, 120, 40, "g1"),
      card("x3", 0, 0, 120, 40, "g1"),
      card("y1", 0, 0, 120, 40, "g2"),
    ],
    edges: [
      { from: "a", to: "c" },
      { from: "a", to: "d" },
      { from: "b", to: "c" },
      { from: "b", to: "e" },
      { from: "c", to: "f" },
      { from: "c", to: "e" },
      { from: "d", to: "f" },
      { from: "e", to: "h" },
      { from: "f", to: "h" },
      { from: "h", to: "i" },
      // 枠の外の札の輪（止まらないことの確認）
      { from: "x1", to: "x2" },
      { from: "x2", to: "x3" },
      { from: "x3", to: "x1" },
      { from: "y1", to: "i" }, // 枠をまたぐ線（層の計算に使わない）
    ],
  };
}

function byId(result) {
  const map = new Map();
  for (const r of result) map.set(r.id, r);
  return map;
}

function typeOf(nodes, id) {
  return nodes.find((n) => n.id === id).type;
}

// 2 つの矩形が重なるか
function overlaps(a, b) {
  return a.x < b.x + b.width && b.x < a.x + a.width && a.y < b.y + b.height && b.y < a.y + a.height;
}

// --- 1. 札どうしが重ならない ---
function testNoOverlap() {
  const { nodes, edges } = sample();
  const result = cvlLayoutBoard(nodes, edges);
  const cards = result.filter((r) => typeOf(nodes, r.id) !== "group");
  assert.strictEqual(cards.length, nodes.filter((n) => n.type !== "group").length);
  for (let i = 0; i < cards.length; i += 1) {
    for (let j = i + 1; j < cards.length; j += 1) {
      assert.ok(
        !overlaps(cards[i], cards[j]),
        `${cards[i].id} と ${cards[j].id} が重なっています: ${JSON.stringify(cards[i])} / ${JSON.stringify(cards[j])}`
      );
    }
  }
  // 同じ値の並びで的配置になっていること（幅・高さは入力のまま）
  for (const r of cards) {
    const src = nodes.find((n) => n.id === r.id);
    assert.strictEqual(r.width, src.width, `${r.id} の幅が変わっています`);
    assert.strictEqual(r.height, src.height, `${r.id} の高さが変わっています`);
  }
}

// --- 2. 各線は左から右へ ---
function testDirection() {
  // 輪のある線（x3 → x1）は層の計算から外すので、向きの検査から外す
  const { nodes } = sample();
  const edges = sample().edges.filter((e) => !(e.from === "x3" && e.to === "x1"));
  const result = cvlLayoutBoard(nodes, edges);
  const map = byId(result);
  const inner = edges.filter(
    (e) => e.from !== e.to && typeOf(nodes, e.from) !== "group" && typeOf(nodes, e.to) !== "group"
  );
  assert.ok(inner.length > 0);
  for (const e of inner) {
    const same = nodes.find((n) => n.id === e.from).group;
    const sameTo = nodes.find((n) => n.id === e.to).group;
    if (same !== sameTo) continue; // 枠をまたぐ線は解決できない
    const from = map.get(e.from);
    const to = map.get(e.to);
    assert.ok(from.x < to.x, `${e.from}(x=${from.x}) → ${e.to}(x=${to.x}) が右向きになっていません`);
  }
}

// --- 3. 枠が中身を含む ---
function testGroupContains() {
  const { nodes, edges } = sample();
  const result = cvlLayoutBoard(nodes, edges);
  const map = byId(result);
  for (const node of nodes) {
    if (node.type !== "group") continue;
    const g = map.get(node.id);
    const members = nodes.filter((n) => n.group === node.id);
    for (const m of members) {
      const p = map.get(m.id);
      assert.ok(p.x >= g.x && p.y >= g.y, `${m.id} が枠 ${node.id} の左上に出ています`);
      assert.ok(
        p.x + p.width <= g.x + g.width && p.y + p.height <= g.y + g.height,
        `${m.id} が枠 ${node.id} からはみ出しています: ${JSON.stringify(p)} / ${JSON.stringify(g)}`
      );
    }
  }
}

// --- 4. 同じ入力にはいつも同じ結果・並び替えに強い ---
function testDeterministic() {
  const { nodes, edges } = sample();
  const a = cvlLayoutBoard(nodes, edges);
  const b = cvlLayoutBoard(nodes, edges);
  assert.deepStrictEqual(a, b);

  // 入力の並びを入れ替えても id ごとの座標が同じ
  const reversed = nodes.slice().reverse();
  const c = cvlLayoutBoard(reversed, edges);
  const ma = byId(a);
  const mc = byId(c);
  for (const r of a) {
    assert.deepStrictEqual(mc.get(r.id), r, `${r.id} の位置が並び替えで変わっています`);
  }
  // 返り値は入力の並び順
  assert.deepStrictEqual(c.map((r) => r.id), reversed.map((n) => n.id));
}

// --- 5. 輪で止まらない ---
function testCycle() {
  const nodes = [card("a", 0, 0, 100, 40), card("b", 0, 0, 100, 40), card("c", 0, 0, 100, 40)];
  const edges = [
    { from: "a", to: "b" },
    { from: "b", to: "c" },
    { from: "c", to: "a" },
  ];
  const result = cvlLayoutBoard(nodes, edges);
  assert.strictEqual(result.length, 3);
  for (let i = 0; i < result.length; i += 1) {
    for (let j = i + 1; j < result.length; j += 1) {
      assert.ok(!overlaps(result[i], result[j]), `${result[i].id} と ${result[j].id} が重なっています`);
    }
  }
  // 自分自身への線と重複した線が混ざっても平気
  const dirty = cvlLayoutBoard(nodes, edges.concat([{ from: "a", to: "a" }, { from: "b", to: "c" }]));
  assert.deepStrictEqual(dirty, result);
}

// --- 6. 入力を書き換えない ---
function testNoMutation() {
  const { nodes, edges } = sample();
  const nodesBefore = JSON.stringify(nodes);
  const edgesBefore = JSON.stringify(edges);
  cvlLayoutBoard(nodes, edges);
  assert.strictEqual(JSON.stringify(nodes), nodesBefore, "nodes を書き換えています");
  assert.strictEqual(JSON.stringify(edges), edgesBefore, "edges を書き換えています");
}

// --- 7. 空の入力 ---
function testEmpty() {
  assert.deepStrictEqual(cvlLayoutBoard([], []), []);
}

// --- 8. 札 200 枚・線 400 本が 1 秒以内に終わる ---
function testPerformance() {
  // シードを固定した簡単な乱数
  let seed = 12345;
  const rand = () => {
    seed = (seed * 1103515245 + 12345) % 2147483648;
    return seed / 2147483648;
  };
  const nodes = [];
  for (let i = 0; i < 200; i += 1) {
    nodes.push(card(`n${i}`, Math.floor(rand() * 1000), Math.floor(rand() * 1000), 80 + Math.floor(rand() * 60), 30 + Math.floor(rand() * 30), i < 20 ? "gA" : i < 40 ? "gB" : null));
  }
  nodes.push(groupNode("gA", 100, 0, 300, 200));
  nodes.push(groupNode("gB", 800, 0, 300, 200));
  const edges = [];
  for (let i = 0; i < 400; i += 1) {
    const from = Math.floor(rand() * 200);
    let to = Math.floor(rand() * 200);
    if (to === from) to = (to + 1) % 200;
    edges.push({ from: `n${from}`, to: `n${to}` });
  }
  const start = Date.now();
  const result = cvlLayoutBoard(nodes, edges);
  const elapsed = Date.now() - start;
  assert.strictEqual(result.length, nodes.length);
  assert.ok(elapsed < 1000, `200 枚・400 本で ${elapsed}ms かかりました（1 秒以内のはず）`);
  console.log(`  200 枚・400 本: ${elapsed}ms`);
}

// --- 中身が無い枠 ---
function testEmptyGroup() {
  const nodes = [groupNode("g1", 0, 0, 10, 10), card("a", 0, 0, 100, 40)];
  const result = cvlLayoutBoard(nodes, []);
  const map = byId(result);
  assert.strictEqual(map.get("g1").width, 400);
  assert.strictEqual(map.get("g1").height, 200);
  assert.strictEqual(map.get("a").x, 0);
  assert.strictEqual(map.get("a").y, 200 + 240);
}

// --- 9. ひとりが多く、層の一番右が最後でない場合 ---
// 折り返しの行が埋まりきらないとき・一番広い層が最後でないときに
// 枠太小になり、枠の外にはみ出すことがある（その不具合のふりかえり）
function testWideCluster() {
  const nodes = [groupNode("g1", 0, 0, 100, 100)];
  for (let i = 0; i < 9; i += 1) {
    nodes.push(card(`s${i}`, 0, 0, 60 + i * 7, 30 + (i % 3) * 8, "g1"));
  }
  // 一番広い層を真ん中に置く（c1 の幅が一番大きい）
  const edges = [
    { from: "s0", to: "s1" },
    { from: "s0", to: "s2" },
  ];
  const result = cvlLayoutBoard(nodes, edges);
  const map = byId(result);
  const g = map.get("g1");
  for (const node of nodes) {
    if (node.type === "group") continue;
    const p = map.get(node.id);
    assert.ok(p.x >= g.x && p.y >= g.y, `${node.id} が枠から出ています（左・上）`);
    assert.ok(
      p.x + p.width <= g.x + g.width && p.y + p.height <= g.y + g.height,
      `${node.id} が枠からはみ出しています: ${JSON.stringify(p)} / ${JSON.stringify(g)}`
    );
  }
}

// --- cvFitView（全体が入るように合わせる） ---

function testFitViewEmpty() {
  assert.deepStrictEqual(cvFitView([], 800, 600), { tx: 0, ty: 0, z: 1 });
  assert.deepStrictEqual(cvFitView(null, 800, 600), { tx: 0, ty: 0, z: 1 });
}

function testFitViewContainsAll() {
  // 3x3 に並んだ 9 個の矩形（0,0 から 800,800 まで）
  const nodes = [];
  for (let i = 0; i < 9; i++) {
    nodes.push({ id: `n${i}`, x: (i % 3) * 400, y: Math.floor(i / 3) * 400, width: 300, height: 300 });
  }
  const w = 1000, h = 800;
  const v = cvFitView(nodes, w, h);
  assert.ok(v.z <= 1, `z は 1 を超えない: ${v.z}`);
  assert.ok(v.z >= ZOOM_MIN, `z が下限より小さい: ${v.z}`);
  // 4 隅が画面の中に収まる
  for (const n of nodes) {
    for (const [px, py] of [[n.x, n.y], [n.x + n.width, n.y + n.height]]) {
      const sx = px * v.z + v.tx;
      const sy = py * v.z + v.ty;
      assert.ok(sx >= -0.001 && sx <= w + 0.001, `x が外: ${sx}`);
      assert.ok(sy >= -0.001 && sy <= h + 0.001, `y が外: ${sy}`);
    }
  }
}

function testFitViewSmallBoardIsCentered() {
  // 1 個だけの小さな矩形は 100% で中央に置く
  const v = cvFitView([{ id: "a", x: 1000, y: 2000, width: 100, height: 60 }], 800, 600);
  assert.strictEqual(v.z, 1);
  assert.strictEqual(v.tx, 800 / 2 - 1000 - 100 / 2);
  assert.strictEqual(v.ty, 600 / 2 - 2000 - 60 / 2);
}

function testFitViewKeepsWithinZoomRange() {
  // 4000x4000 の図を 200x200 で見る -> 下限に丸められる
  const v = cvFitView([{ id: "a", x: 0, y: 0, width: 4000, height: 4000 }], 200, 200);
  assert.ok(v.z >= ZOOM_MIN, `z が下限より小さい: ${v.z}`);
  assert.ok(v.z > 0, "z が 0 ではない");
}

// --- cvZoomAt（指の位置の下の図の点が動かない） ---

function testZoomAtKeepsAnchor() {
  const view = { tx: 30, ty: -12, z: 1 };
  const px = 420, py = 260;
  const before = { x: (px - view.tx) / view.z, y: (py - view.ty) / view.z };
  for (const factor of [1.2, 1 / 1.2, 3, 0.2]) {
    const z = cvZoomAt(view, px, py, factor);
    const after = { x: (px - z.tx) / z.z, y: (py - z.ty) / z.z };
    assert.ok(Math.abs(after.x - before.x) < 1e-6, `x が動いた: ${after.x} != ${before.x}`);
    assert.ok(Math.abs(after.y - before.y) < 1e-6, `y が動いた: ${after.y} != ${before.y}`);
  }
}

function testZoomAtClamps() {
  const view = { tx: 0, ty: 0, z: 1 };
  const big = cvZoomAt(view, 100, 100, 1000);
  assert.strictEqual(big.z, ZOOM_MAX);
  const small = cvZoomAt(view, 100, 100, 0.0001);
  assert.strictEqual(small.z, ZOOM_MIN);
  // 大きすぎる拡大率から縮めても下限で止まる（2.5 * 0.01 = 0.025 -> 0.1）
  const fromMax = cvZoomAt({ tx: 0, ty: 0, z: ZOOM_MAX }, 0, 0, 0.01);
  assert.strictEqual(fromMax.z, ZOOM_MIN);
  // 入力を壊さない
  assert.deepStrictEqual(view, { tx: 0, ty: 0, z: 1 });
}

function testZoomAtAtOrigin() {
  // 原点が画面の左上なら tx, ty は 0 のまま
  const z = cvZoomAt({ tx: 0, ty: 0, z: 1 }, 0, 0, 1.5);
  assert.strictEqual(z.tx, 0);
  assert.strictEqual(z.ty, 0);
  assert.strictEqual(z.z, 1.5);
}

function main() {
  testNoOverlap();
  testDirection();
  testGroupContains();
  testDeterministic();
  testCycle();
  testNoMutation();
  testEmpty();
  testPerformance();
  testEmptyGroup();
  testWideCluster();
  testFitViewEmpty();
  testFitViewContainsAll();
  testFitViewSmallBoardIsCentered();
  testFitViewKeepsWithinZoomRange();
  testZoomAtKeepsAnchor();
  testZoomAtClamps();
  testZoomAtAtOrigin();
  console.log("test_canvaslayout: すべて通りました");
}

main();
