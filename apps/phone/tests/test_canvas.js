"use strict";
// Canvas の画面（canvas.js）の純関数のテスト（計画 U06「変更内容 6」）。
// 実行:
//     node tests/test_canvas.js
//
// ここに無いもの（DOM が要る描画・操作）はブラウザで確かめる（計画 U06 の AC2〜AC6）。

const assert = require("assert");
const path = require("path");

const {
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
} = require(path.join(__dirname, "..", "src", "sai_phone", "static", "canvas.js"));

// --- cvBoardHash / cvParseBoardHash（住所の往復） ---

function testBoardHashRoundTrip() {
  const cases = [
    { project: "sai-paper", name: "前処理" },
    { project: "論文 プロジェクト", name: "map-preproc 20260926" },
    { project: "a", name: "b" },
    { project: "プロジェクト/2026", name: "Canvas 名" },
    { project: "100%", name: "a+b&c=d?e" },
  ];
  for (const c of cases) {
    const hash = cvBoardHash(c.project, c.name);
    assert.ok(hash.startsWith("#/canvas/"), `住所の形が違います: ${hash}`);
    const back = cvParseBoardHash(hash);
    assert.deepStrictEqual(back, c, `往復で元に戻りません: ${c.name}`);
  }
}

function testParseBoardHashRejects() {
  assert.strictEqual(cvParseBoardHash("#/papers/canvas"), null);
  assert.strictEqual(cvParseBoardHash("#/canvas/only-one"), null);
  assert.strictEqual(cvParseBoardHash("#/canvas/"), null);
  assert.strictEqual(cvParseBoardHash(""), null);
  assert.strictEqual(cvParseBoardHash(null), null);
  // 斜線は存入できないので、1 段の住所とは区別できる
  assert.strictEqual(cvParseBoardHash("#/canvas/a/b/c"), null);
}

// --- cvFitView（全体が入るように合わせる） ---

function testFitViewEmpty() {
  assert.deepStrictEqual(cvFitView([], 800, 600), { tx: 0, ty: 0, z: 1 });
  assert.deepStrictEqual(cvFitView(null, 800, 600), { tx: 0, ty: 0, z: 1 });
}

function testFitViewContainsAll() {
  // 3x3 に並んだ 9 枚の札（0,0 から 800,800 まで）
  const nodes = [];
  for (let i = 0; i < 9; i++) {
    nodes.push({ id: `n${i}`, x: (i % 3) * 400, y: Math.floor(i / 3) * 400, width: 300, height: 300 });
  }
  const w = 1000, h = 800;
  const v = cvFitView(nodes, w, h);
  assert.ok(v.z <= 1, `z は 1 を超えない: ${v.z}`);
  assert.ok(v.z >= CV_ZOOM_MIN, `z が下限より小さい: ${v.z}`);
  // 4 隅が画面 안에収まる
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
  // 1 枚だけの小さな札は 100% で中央に置く
  const v = cvFitView([{ id: "a", x: 1000, y: 2000, width: 100, height: 60 }], 800, 600);
  assert.strictEqual(v.z, 1);
  assert.strictEqual(v.tx, 800 / 2 - 1000 - 100 / 2);
  assert.strictEqual(v.ty, 600 / 2 - 2000 - 60 / 2);
}

function testFitViewKeepsWithinZoomRange() {
  // 4000x4000 の盤面を 200x200 で見る -> 下限に丸められる
  const v = cvFitView([{ id: "a", x: 0, y: 0, width: 4000, height: 4000 }], 200, 200);
  assert.ok(v.z >= CV_ZOOM_MIN, `z が下限より小さい: ${v.z}`);
  assert.ok(v.z > 0, "z が 0 ではない");
}

// --- cvZoomAt（指の位置の下のワールドの点が動かない） ---

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
  assert.strictEqual(big.z, CV_ZOOM_MAX);
  const small = cvZoomAt(view, 100, 100, 0.0001);
  assert.strictEqual(small.z, CV_ZOOM_MIN);
  // 大きすぎる拡大率から縮めても下限で止まる（2.5 * 0.01 = 0.025 -> 0.1）
  const fromMax = cvZoomAt({ tx: 0, ty: 0, z: CV_ZOOM_MAX }, 0, 0, 0.01);
  assert.strictEqual(fromMax.z, CV_ZOOM_MIN);
  // 入力を壊さない
  assert.deepStrictEqual(view, { tx: 0, ty: 0, z: 1 });
}

function testZoomAtAtOrigin() {
  // 原点が画面左上なら tx, ty は 0 のまま
  const z = cvZoomAt({ tx: 0, ty: 0, z: 1 }, 0, 0, 1.5);
  assert.strictEqual(z.tx, 0);
  assert.strictEqual(z.ty, 0);
  assert.strictEqual(z.z, 1.5);
}

// --- cvClosestSides（横・縦の並び） ---

function rect(x, y, w, h) { return { x, y, width: w, height: h }; }

function testClosestSidesHorizontal() {
  const a = rect(0, 0, 100, 80);
  const b = rect(200, 10, 100, 80);
  assert.deepStrictEqual(cvClosestSides(a, b), ["right", "left"]);
  // 左右を逆にすると逆の辺
  assert.deepStrictEqual(cvClosestSides(b, a), ["left", "right"]);
}

function testClosestSidesVertical() {
  const a = rect(10, 0, 100, 80);
  const b = rect(20, 200, 100, 80);
  assert.deepStrictEqual(cvClosestSides(a, b), ["bottom", "top"]);
  assert.deepStrictEqual(cvClosestSides(b, a), ["top", "bottom"]);
}

function testClosestSidesDiagonalPicksDominantAxis() {
  const a = rect(0, 0, 100, 100);
  const b = rect(400, 30, 100, 100);
  // 横の差のほうが縦の差より大きい -> 左右
  assert.deepStrictEqual(cvClosestSides(a, b), ["right", "left"]);
}

// --- cvEdgePath（SVG の d） ---

function testEdgePathShape() {
  const a = rect(0, 0, 100, 100);
  const b = rect(300, 0, 100, 100);
  const d = cvEdgePath(a, b);
  assert.ok(d.startsWith("M "), `M で始まらない: ${d}`);
  assert.ok(d.includes(" C "), `C を含まない: ${d}`);
  // M の後は a の右辺の中点 (100, 50)
  assert.ok(d.startsWith("M 100 50 "), `始点が右辺の中点でない: ${d}`);
  // 終点は b の左辺の中点 (300, 50)
  assert.ok(d.trim().endsWith("300 50"), `終点が左辺の中点でない: ${d}`);
}

function testEdgePathControlPointsPerpendicular() {
  const a = rect(0, 0, 100, 100);
  const b = rect(400, 0, 100, 100);
  const nums = cvEdgePath(a, b).match(/-?\d+(\.\d+)?/g).map(Number);
  // M x y C c1x c1y c2x c2y x y
  const [x1, y1, c1x, c1y, c2x, c2y, x2, y2] = nums;
  assert.strictEqual(x1, 100);
  assert.strictEqual(y1, 50);
  assert.strictEqual(x2, 400);
  assert.strictEqual(y2, 50);
  // 制御点は水平（a の右辺の外へ出る）
  assert.ok(c1x > x1, "1 つ目の制御点が右へ出ていない");
  assert.strictEqual(c1y, y1);
  assert.ok(c2x < x2, "2 つ目の制御点が左へ出ていない");
  assert.strictEqual(c2y, y2);
  // 距離 = 300 -> 制御点まで 120 (300*0.4)
  assert.ok(Math.abs(c1x - x1 - 120) < 0.2, `制御点までの距離が違う: ${c1x - x1}`);
}

function testEdgePathMinControlDistance() {
  // 近い 2 枚は 40 を下限として外へ出る
  const a = rect(0, 0, 100, 100);
  const b = rect(110, 0, 100, 100);
  const nums = cvEdgePath(a, b).match(/-?\d+(\.\d+)?/g).map(Number);
  const [, , c1x] = nums;
  assert.ok(Math.abs(c1x - 100 - 40) < 0.2, `下限の 40 になっていない: ${c1x - 100}`);
}

// --- cvEdgeOpacity（選んだ札の線を濃くする） ---

function testEdgeOpacityNoSelection() {
  assert.strictEqual(cvEdgeOpacity({ from: "a", to: "b" }, null), 0.7);
  assert.strictEqual(cvEdgeOpacity({ from: "a", to: "b" }, undefined), 0.7);
}

function testEdgeOpacityWithSelection() {
  const hit = { from: "note-1", to: "note-2" };
  const miss = { from: "note-3", to: "note-4" };
  assert.strictEqual(cvEdgeOpacity(hit, "note-1"), 1);
  assert.strictEqual(cvEdgeOpacity(hit, "note-2"), 1);
  assert.strictEqual(cvEdgeOpacity(miss, "note-1"), 0.12);
  assert.strictEqual(cvEdgeOpacity(hit, "note-9"), 0.12);
}

// --- 色の表 ---

function testGroupColor() {
  for (const [key, hex] of Object.entries(CV_GROUP_COLORS)) {
    assert.strictEqual(cvGroupColor(key), hex, `色 ${key} が違う`);
    assert.strictEqual(cvAlpha(cvGroupColor(key), 0.12).startsWith("rgba("), true);
  }
  assert.strictEqual(cvGroupColor("#123456"), "#123456");
  assert.strictEqual(cvGroupColor("7"), "");   // 表に無い数字
  assert.strictEqual(cvGroupColor(""), "");
  assert.strictEqual(cvGroupColor(null), "");
  assert.strictEqual(cvGroupColor("#abc"), ""); // 6 桁でない
}

function testEdgeKindTable() {
  assert.deepStrictEqual(
    CV_EDGE_KINDS.map((k) => k[0]),
    ["citation", "uses", "derived", "mentions", "link"]
  );
  // 5 種類すべてに名前と 16 進の色がある
  for (const [kind, label, color] of CV_EDGE_KINDS) {
    assert.ok(kind && label, "kind か名前が空");
    assert.ok(/^#[0-9a-f]{6}$/i.test(color), `色が 16 進でない: ${kind} ${color}`);
  }
  // 札の 3 種類
  assert.deepStrictEqual(Object.keys(CV_CARD_KINDS).sort(), ["note", "paper", "report"]);
}

// --- 札の kind が null のとき（U03: DB に無い札の kind は null になる） ---

function testCardKindNullIsPaperGrey() {
  const paper = CV_CARD_KINDS.paper;
  for (const k of [null, undefined, "", "   "]) {
    const r = cvCardKind(k);
    assert.strictEqual(r.color, paper, `null の kind が paper の灰色でない: ${JSON.stringify(r)}`);
    assert.strictEqual(r.key, "");
    assert.strictEqual(r.label, "paper");
  }
  // 画面が止まらないこと（例外を投げない）
  assert.doesNotThrow(() => cvCardKind(null));
}

function testCardKindUnknownIsPaperGrey() {
  const paper = CV_CARD_KINDS.paper;
  // body の kind は "file" にもなる。表に無い種類は灰色
  for (const k of ["file", "unknown", "Note", "NOTE", "note2"]) {
    const r = cvCardKind(k);
    assert.strictEqual(r.color, paper, `${k} が paper の灰色でない: ${JSON.stringify(r)}`);
    assert.strictEqual(r.key, "", `${k} が既知として扱われている`);
  }
  // 数字やオブジェクトのような値でも落ちない
  for (const k of [0, 42, {}, [], true]) {
    assert.doesNotThrow(() => cvCardKind(k), `kind=${JSON.stringify(k)} で落ちた`);
    assert.strictEqual(cvCardKind(k).color, paper);
  }
}

function testCardKindKnownUnchanged() {
  assert.deepStrictEqual(cvCardKind("note"), { key: "note", label: "note", color: "#2563eb" });
  assert.deepStrictEqual(cvCardKind("report"), { key: "report", label: "report", color: "#1d7a45" });
  assert.deepStrictEqual(cvCardKind("paper"), { key: "paper", label: "paper", color: "#8a8f98" });
}

// --- 線の kind が未知のとき ---

function testEdgeKindUnknown() {
  const grey = "#6b7280"; // link の色
  for (const k of [null, undefined, "", "weird"]) {
    const r = cvEdgeKind(k);
    assert.strictEqual(r.known, false, `${k} が既知として扱われている`);
    assert.strictEqual(r.color, grey);
    assert.strictEqual(cvEdgeKindColor(k), grey);
  }
  assert.deepStrictEqual(cvEdgeKind("citation"), { key: "citation", known: true, color: "#2563eb" });
  assert.strictEqual(cvEdgeKindColor("uses"), "#1d7a45");
}

function testTablesRejectPrototypeKeys() {
  // "toString" や "constructor" を渡しても表の値と取り違えない
  for (const k of ["toString", "constructor", "hasOwnProperty", "__proto__"]) {
    assert.strictEqual(cvCardKind(k).color, CV_CARD_KINDS.paper, `cvCardKind(${k})`);
    assert.strictEqual(cvGroupColor(k), "", `cvGroupColor(${k})`);
  }
}

// --- 実行 ---

const tests = [
  testBoardHashRoundTrip,
  testParseBoardHashRejects,
  testFitViewEmpty,
  testFitViewContainsAll,
  testFitViewSmallBoardIsCentered,
  testFitViewKeepsWithinZoomRange,
  testZoomAtKeepsAnchor,
  testZoomAtClamps,
  testZoomAtAtOrigin,
  testClosestSidesHorizontal,
  testClosestSidesVertical,
  testClosestSidesDiagonalPicksDominantAxis,
  testEdgePathShape,
  testEdgePathControlPointsPerpendicular,
  testEdgePathMinControlDistance,
  testEdgeOpacityNoSelection,
  testEdgeOpacityWithSelection,
  testGroupColor,
  testCardKindNullIsPaperGrey,
  testCardKindUnknownIsPaperGrey,
  testCardKindKnownUnchanged,
  testEdgeKindUnknown,
  testTablesRejectPrototypeKeys,
  testEdgeKindTable,
];

let failed = 0;
for (const fn of tests) {
  try {
    fn();
    console.log(`ok  ${fn.name}`);
  } catch (e) {
    failed += 1;
    console.error(`NG  ${fn.name}: ${e && e.message}`);
  }
}
if (failed) {
  console.error(`test_canvas: ${failed} 件が失敗しました`);
  process.exit(1);
}
console.log(`test_canvas: ${tests.length} 件すべて通りました`);
