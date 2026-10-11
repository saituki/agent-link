"use strict";
// 波形のレーン（lanes.js）の純関数のテスト。
// 実行:
//     node apps/ts/tests/test_lanes.js

const assert = require("assert");
const path = require("path");

const dir = path.join(__dirname, "..", "src", "sai_ts", "static");
// tsValueTicks は app.js の tsNiceTicks を使う（画面では同じ窓の中の関数）
global.tsNiceTicks = require(path.join(dir, "app.js")).tsNiceTicks;
const {
  tsLaneLayout,
  tsAutoRange,
  tsM4Path,
  tsRawPath,
  tsHitEvent,
  tsSelectionFromDrag,
  tsRgba,
  tsFmtVal,
  tsValueTicks,
  tsGainSteps,
  tsGainStep,
  tsGainParse,
} = require(path.join(dir, "lanes.js"));

// レーンの配置: 最小の高さ、変数 0 個（時間軸は含まない）
assert.strictEqual(tsLaneLayout(3, 300, 24).laneH, 100);
assert.deepStrictEqual(tsLaneLayout(3, 300, 24).rows[2], { y0: 200, y1: 300 });
assert.strictEqual(tsLaneLayout(3, 300, 24).total, 300);
assert.strictEqual(tsLaneLayout(2, 20, 24).laneH, 24, "足りないときは最小の高さ");
assert.strictEqual(tsLaneLayout(2, 20, 24).total, 48, "足りないときは縦にスクロール");
assert.deepStrictEqual(tsLaneLayout(0, 300, 24), { laneH: 0, rows: [], total: 0 });

// 縦軸の目盛り: 上端と下端の値の間に入る（toV が線形のレーン: 上が 24・下が -24）
const lane = { y0: 0, y1: 100, toV: (y) => (50 - y) / 2 };
assert.deepStrictEqual(tsValueTicks(lane, 4), [-20, 0, 20]);

// 値の書き方: 有効数字 3 桁、大きすぎる・小さすぎる値は指数表記
assert.strictEqual(tsFmtVal(1234.5), "1230");
assert.strictEqual(tsFmtVal(-0.5), "-0.5");
assert.strictEqual(tsFmtVal(0), "0");
assert.strictEqual(tsFmtVal(0.00012), "1.2e-4");
assert.strictEqual(tsFmtVal(25000), "2.5e+4");

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

// 倍率の段: 上げ下げは段ごと、端で止まる、段に無い値は一番近い段から動く
assert.deepStrictEqual(tsGainSteps, [0.1, 0.2, 0.5, 1, 2, 5, 10]);
assert.strictEqual(tsGainStep(1, 1), 2);
assert.strictEqual(tsGainStep(1, -1), 0.5);
assert.strictEqual(tsGainStep(10, 1), 10, "上の端で止まる");
assert.strictEqual(tsGainStep(0.1, -1), 0.1, "下の端で止まる");
assert.strictEqual(tsGainStep(1.3, 1), 2, "1.3 は 1 の段から上がる");
assert.strictEqual(tsGainStep(1.3, -1), 0.5, "1.3 は 1 の段から下がる");
assert.strictEqual(tsGainStep(0.9, 1), 2, "0.9 は 1 の段から上がる");
assert.strictEqual(tsGainStep(NaN, 1), 2, "読めない値は 1 として動く");
assert.strictEqual(tsGainStep(-3, -1), 0.5, "負の値は 1 として動く");

// 保存の読み: 読めない値は 1。段に無い値も 1。古い保存の ch は読み捨てる
assert.deepStrictEqual(tsGainParse(null), { all: 1 });
assert.deepStrictEqual(tsGainParse("{壊れ"), { all: 1 });
assert.deepStrictEqual(tsGainParse("[1,2]"), { all: 1 });
assert.deepStrictEqual(tsGainParse('{"all": 2, "ch": {"ch01": 5}}'), { all: 2 });
assert.deepStrictEqual(tsGainParse('{"all": 1.3}'), { all: 1 });
assert.deepStrictEqual(tsGainParse('{"all": "x", "ch": []}'), { all: 1 });

console.log("test_lanes.js: OK");
