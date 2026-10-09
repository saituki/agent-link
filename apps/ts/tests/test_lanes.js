"use strict";
// 波形のレーン（lanes.js）の純関数のテスト。
// 実行:
//     node apps/ts/tests/test_lanes.js

const assert = require("assert");
const path = require("path");

const src = path.join(__dirname, "..", "src", "sai_ts", "static", "lanes.js");
const { tsLaneLayout, tsAutoRange, tsM4Path, tsRawPath, tsHitEvent, tsSelectionFromDrag, tsRgba } = require(src);

// レーンの配置: 最小の高さ、変数 0 個
assert.deepStrictEqual(tsLaneLayout(3, 300, 24, 24).laneH, 92);
assert.deepStrictEqual(tsLaneLayout(3, 300, 24, 24).rows[2], { y0: 184, y1: 276 });
assert.strictEqual(tsLaneLayout(3, 300, 24, 24).total, 300);
assert.strictEqual(tsLaneLayout(2, 20, 24, 24).laneH, 24, "足りないときは最小の高さ");
assert.strictEqual(tsLaneLayout(2, 20, 24, 24).total, 72, "足りないときは縦にスクロール");
assert.deepStrictEqual(tsLaneLayout(0, 300, 24, 24), { laneH: 0, rows: [], total: 24 });

// 縦の範囲: 全部 null、1% と 99% 点、値が同じ
assert.deepStrictEqual(tsAutoRange([null, null]), [-1, 1]);
assert.deepStrictEqual(tsAutoRange([]), [-1, 1]);
const seq = Array.from({ length: 101 }, (_, i) => i);
assert.deepStrictEqual(tsAutoRange(seq), [1, 99]);
assert.deepStrictEqual(tsAutoRange([5, 5, null, 5]), [4, 6]);

// M4 の列: 縦線（first→min→max→last）と隣の列への線。null の列で線を切る
const id = (v) => v;
const m4 = tsM4Path(
  { first: [1, null, 3], min: [0, null, 2], max: [2, null, 4], last: [1, null, 3] },
  id,
  id,
);
assert.deepStrictEqual(
  m4.map((p) => [p.x, p.y, p.move]),
  [
    [0, 1, true],
    [0, 0, false],
    [0, 2, false],
    [0, 1, false],
    [2, 3, true],
    [2, 2, false],
    [2, 4, false],
    [2, 3, false],
  ],
);
// 隣り合う列は線でつながる（move が false）
const joined = tsM4Path({ first: [1, 2], min: [1, 2], max: [1, 2], last: [1, 2] }, id, id);
assert.strictEqual(joined[4].move, false);
assert.deepStrictEqual(tsM4Path({ first: [null], min: [null], max: [null], last: [null] }, id, id), []);

// raw: null で線を切る
const raw = tsRawPath([1, null, 3], 10, 0.5, (t) => (t - 10) * 10, id);
assert.deepStrictEqual(raw.map((p) => [p.x, p.y, p.move]), [
  [0, 1, true],
  [10, 3, true],
]);

// 当たり判定: ラインは許容幅の中で一番近い、無ければエリアの中で一番狭い
const evs = [
  { id: "L", type: "line", t: 10, vars: [] },
  { id: "A", type: "area", t0: 5, t1: 15, vars: ["Fp1"] },
  { id: "B", type: "line", t: 12, vars: ["Fp2"] },
  { id: "C", type: "area", t0: 9, t1: 11, vars: [] },
];
assert.strictEqual(tsHitEvent(evs, 10.1, "Fp1", 0.05).id, "C", "ライン L は外れ、狭いエリア C");
assert.strictEqual(tsHitEvent(evs, 10.02, "Fp1", 0.05).id, "L", "ライン L に当たる");
assert.strictEqual(tsHitEvent(evs, 12, "Fp1", 0.05).id, "A", "B は別の変数のレーン");
assert.strictEqual(tsHitEvent(evs, 30, "Fp1", 0.1), null);

// ドラッグの範囲: 触れたレーンすべて。触れなければ一番近いレーン
const frame = {
  t0: 0,
  t1: 100,
  toT: (x) => (x - 80) / 10,
  lanes: [
    { var: "a", y0: 0, y1: 50 },
    { var: "b", y0: 50, y1: 100 },
  ],
};
assert.deepStrictEqual(tsSelectionFromDrag(280, 180, 20, 70, frame), { t0: 10, t1: 20, vars: ["a", "b"] });
assert.deepStrictEqual(tsSelectionFromDrag(180, 280, 60, 80, frame).vars, ["b"]);
assert.deepStrictEqual(tsSelectionFromDrag(180, 280, 120, 130, frame).vars, ["b"]);
// 記録の外へ出たドラッグは範囲の中に切る
assert.deepStrictEqual(tsSelectionFromDrag(0, 180, 10, 10, frame).t0, 0);

// 色に透明度を付ける
assert.strictEqual(tsRgba("#d9534f", 0.5), "rgba(217,83,79,0.5)");
assert.strictEqual(tsRgba("red", 0.5), "rgba(100,116,139,0.5)");

console.log("test_lanes.js: OK");
