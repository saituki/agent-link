"use strict";
// 指標グラフの部品（mlfchart.js）の純関数のテスト（実装計画 U07「完了条件」AC1・AC2）。
// 実行:
//     node apps/web/tests/test_mlfchart.js
//
// SVG を描いて操作する部分（点を押して値を見る・2 列に並べる）はブラウザで確かめる（AC5）。
// ここでは series に分ける関数と目盛りの写し方だけを確かめる。

const assert = require("assert");
const path = require("path");

// ブラウザでは ui.js が読み込む前に mlfchart.js があるため、目盛りの関数
// （saiNiceScale）は全体にある。ここでは node なので ui.js から持ってきて全体に入れておく。
const U = require(path.join(__dirname, "..", "src", "sai_web", "static", "ui.js"));
global.saiNiceScale = U.saiNiceScale;
global.saiAxisValue = U.saiAxisValue;

const src = path.join(__dirname, "..", "src", "sai_web", "static", "mlfchart.js");
const {
  mlfSeriesFromMetrics,
  mlfChartScale,
  mlfPathD,
  mlfChartNum,
  mlfHasPoints,
} = require(src);

const RUN_A = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const RUN_B = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

/** 見本の点を作る。[{key, value, timestamp, step, run_id}] */
function points(list, runId, key, t0) {
  const base = t0 || 1000000;
  return list.map((v, i) => ({
    key: key || "loss",
    value: v,
    timestamp: base + i * 1000,
    step: i,
    run_id: runId,
  }));
}

// --- AC1: 指標を run ごとに分ける ---

function testSplitsRuns() {
  const metrics = [
    ...points([0.9, 0.5], RUN_A, "loss", 1000000),
    ...points([0.8, 0.4], RUN_B, "loss", 2000000),
  ];
  const series = mlfSeriesFromMetrics(metrics, "step");
  assert.strictEqual(series.length, 2, "run ごとに 1 つずつ");
  assert.deepStrictEqual(series.map((s) => s.runId), [RUN_A, RUN_B]);
  assert.deepStrictEqual(series[0].points, [
    { x: 0, y: 0.9, step: 0, t: 1000000 },
    { x: 1, y: 0.5, step: 1, t: 1001000 },
  ]);
  assert.deepStrictEqual(series[1].points.map((p) => p.x), [0, 1]);
}

function testSortedByX() {
  // 届いた順に並ばないときがある（MLflow は概ね昇順だが、保証ではない）
  const metrics = [
    { key: "loss", value: 0.3, timestamp: 1002000, step: 2, run_id: RUN_A },
    { key: "loss", value: 0.1, timestamp: 1000000, step: 0, run_id: RUN_A },
    { key: "loss", value: 0.2, timestamp: 1001000, step: 1, run_id: RUN_A },
  ];
  const series = mlfSeriesFromMetrics(metrics, "step");
  assert.deepStrictEqual(series[0].points.map((p) => p.x), [0, 1, 2]);
  assert.deepStrictEqual(series[0].points.map((p) => p.y), [0.1, 0.2, 0.3]);
}

function testDropsBadValues() {
  const metrics = [
    { key: "loss", value: 0.1, timestamp: 1000000, step: 0, run_id: RUN_A },
    { key: "loss", value: NaN, timestamp: 1001000, step: 1, run_id: RUN_A },
    { key: "loss", value: Infinity, timestamp: 1002000, step: 2, run_id: RUN_A },
    { key: "loss", value: -Infinity, timestamp: 1003000, step: 3, run_id: RUN_A },
    { key: "loss", value: "x", timestamp: 1004000, step: 4, run_id: RUN_A },
    { key: "loss", value: null, timestamp: 1005000, step: 5, run_id: RUN_A },
    { key: "loss", value: 0.2, timestamp: 1006000, step: 6, run_id: RUN_A },
  ];
  const series = mlfSeriesFromMetrics(metrics, "step");
  assert.deepStrictEqual(series[0].points.map((p) => p.x), [0, 6]);
}

function testTimeIsSecondsFromFirst() {
  // 「経過時間」はその run の最初の timestamp からの秒数
  const metrics = [
    { key: "loss", value: 0.1, timestamp: 2000000, step: 0, run_id: RUN_A },
    { key: "loss", value: 0.2, timestamp: 2001500, step: 1, run_id: RUN_A },
    { key: "loss", value: 0.3, timestamp: 2006000, step: 2, run_id: RUN_A },
  ];
  const series = mlfSeriesFromMetrics(metrics, "time");
  assert.deepStrictEqual(series[0].points.map((p) => p.x), [0, 1.5, 6]);
  // 経過時間はどの run でも自分の最初の点が 0 になる
  const both = mlfSeriesFromMetrics(
    [
      ...metrics,
      { key: "loss", value: 0.9, timestamp: 9000000, step: 0, run_id: RUN_B },
    ],
    "time"
  );
  assert.deepStrictEqual(both[1].points.map((p) => p.x), [0]);
}

function testMissingRunIdIsOneSeries() {
  // 履歴 API は run_id を返さない。1 つの series として扱う
  const series = mlfSeriesFromMetrics(
    [
      { key: "loss", value: 0.1, timestamp: 1000000, step: 0 },
      { key: "loss", value: 0.2, timestamp: 1001000, step: 1 },
    ],
    "step"
  );
  assert.strictEqual(series.length, 1);
  assert.strictEqual(series[0].runId, "");
  assert.strictEqual(series[0].points.length, 2);
}

function testEmptyInput() {
  assert.deepStrictEqual(mlfSeriesFromMetrics([], "step"), []);
  assert.deepStrictEqual(mlfSeriesFromMetrics(null, "time"), []);
  assert.deepStrictEqual(mlfSeriesFromMetrics(undefined, "step"), []);
}

// --- AC2: 目盛り（NaN を出さない） ---

function testScaleOnePoint() {
  const series = [{ runId: RUN_A, points: [{ x: 3, y: 0.5, step: 3, t: 1 }] }];
  const s = mlfChartScale(series, 320, 160);
  assert.ok(Number.isFinite(s.xOf(3)), "x は有限の数");
  assert.ok(Number.isFinite(s.yOf(0.5)), "y は有限の数");
  assert.ok(s.xMax > s.xMin, "x の幅がある");
  assert.ok(s.yMax > s.yMin, "y の幅がある");
  for (const v of s.xTicks) assert.ok(Number.isFinite(s.xOf(v)), `目盛り ${v}`);
  for (const v of s.yTicks) assert.ok(Number.isFinite(s.yOf(v)), `目盛り ${v}`);
}

function testScaleSameY() {
  // 全部同じ値のとき（-acc が平らななど）も NaN を出さない
  const points2 = [1, 1, 1, 1].map((v, i) => ({ x: i, y: v, step: i, t: i }));
  const s = mlfChartScale([{ runId: RUN_A, points: points2 }], 320, 160);
  assert.ok(s.yMax > s.yMin);
  assert.ok(Number.isFinite(s.yOf(1)));
  for (const v of s.yTicks) assert.ok(Number.isFinite(s.yOf(v)), `目盛り ${v}`);
}

function testScaleSameX() {
  const points2 = [1, 2, 3].map((v) => ({ x: 0, y: v, step: 0, t: 0 }));
  const s = mlfChartScale([{ runId: RUN_A, points: points2 }], 320, 160);
  assert.ok(s.xMax > s.xMin);
  assert.ok(Number.isFinite(s.xOf(0)));
}

function testScaleZero() {
  // 全部 0
  const points2 = [0, 0].map((v, i) => ({ x: i, y: v, step: i, t: i }));
  const s = mlfChartScale([{ runId: RUN_A, points: points2 }], 320, 160);
  assert.ok(s.yMax > s.yMin);
  assert.ok(Number.isFinite(s.yOf(0)));
}

function testScaleEmpty() {
  // 点が 1 つも無いとき
  const s = mlfChartScale([], 320, 160);
  assert.ok(Number.isFinite(s.xOf(0)));
  assert.ok(Number.isFinite(s.yOf(0)));
  assert.ok(s.yMax > s.yMin);
  assert.deepStrictEqual(mlfPathD([], s), "");
}

function testScaleNegative() {
  // 負の値（0 より小さい指標もある）
  const points2 = [-3, -1, 2].map((v, i) => ({ x: i, y: v, step: i, t: i }));
  const s = mlfChartScale([{ runId: RUN_A, points: points2 }], 320, 160);
  assert.ok(s.yMin < 0, "負の値にも下端がある");
  assert.ok(s.yMax > 0);
  for (const v of s.yTicks) assert.ok(Number.isFinite(s.yOf(v)), `目盛り ${v}`);
  // 0 の目盛りがある（負の値と正の値が同じ図に載る）
  assert.ok(s.yTicks.some((v) => Math.abs(v) < 1e-9), "0 の目盛りがある");
  const top = s.yOf(s.yMax);
  const bottom = s.yOf(s.yMin);
  assert.ok(s.yOf(0) > top && s.yOf(0) < bottom, "0 は上下の端の間");
}

function testScaleBadSize() {
  const points2 = [{ x: 0, y: 1 }, { x: 1, y: 2 }];
  for (const size of [[0, 0], [null, null], [-5, -5], ["x", "y"]]) {
    const s = mlfChartScale([{ runId: RUN_A, points: points2 }], size[0], size[1]);
    assert.ok(s.width > 0 && s.height > 0, `幅 ${size[0]}・高さ ${size[1]} でも正`);
    assert.ok(Number.isFinite(s.xOf(1)));
    assert.ok(Number.isFinite(s.yOf(1)));
  }
}

function testPathD() {
  const points2 = [
    { x: 0, y: 0, step: 0, t: 0 },
    { x: 1, y: 1, step: 1, t: 1 },
    { x: 2, y: 0.5, step: 2, t: 2 },
  ];
  const s = mlfChartScale([{ runId: RUN_A, points: points2 }], 320, 160);
  const d = mlfPathD(points2, s);
  assert.ok(d.startsWith("M "), "M で始まる");
  assert.strictEqual((d.match(/L /g) || []).length, 2, "点が 3 つなら L が 2 つ");
  for (const num of d.slice(1).split(/[ ,]/)) {
    if (num === "L" || num === "M") continue; // 絵命令
    assert.ok(Number.isFinite(Number(num)), `座標 ${num} が有限`);
  }
  // 点が 1 つでも壊れない
  assert.ok(mlfPathD(points2.slice(0, 1), s).startsWith("M "));
  // 壊れた点が混ざっても、その点だけ飛ばす
  const mixed = mlfPathD([...points2, { x: NaN, y: 1 }], s);
  assert.ok(!mixed.includes("NaN"), "NaN は書かない");
}

function testTickCountShrinksWithWidth() {
  const points2 = Array.from({ length: 20 }, (_, i) => ({ x: i, y: i, step: i, t: i }));
  const series = [{ runId: RUN_A, points: points2 }];
  const narrow = mlfChartScale(series, 300, 160);
  const wide = mlfChartScale(series, 900, 160);
  assert.ok(narrow.xTicks.length <= wide.xTicks.length, "狭い画面のほうが目盛りは少ない");
  assert.ok(narrow.xTicks.length >= 2, "最低 2 本はある");
  // 目盛りは昇順
  for (const arr of [narrow.xTicks, narrow.yTicks, wide.yTicks]) {
    for (let i = 1; i < arr.length; i += 1) {
      assert.ok(arr[i] > arr[i - 1], "目盛りは昇順");
    }
  }
}

function testTicksAreNice() {
  // 6.333 のような切りの悪い数字は出さない（saiNiceScale に任せる）
  const points2 = Array.from({ length: 20 }, (_, i) => ({ x: i, y: 1, step: i, t: i }));
  const s = mlfChartScale([{ runId: RUN_A, points: points2 }], 900, 160);
  assert.deepStrictEqual(s.xTicks, [0, 5, 10, 15, 20]);
  assert.ok(s.xMax >= 19, "最大はデータの最大以上");
  // 経過時間も同じ（0.7 秒刻みなら 0 / 5 / 10 / 15）
  const timePts = Array.from({ length: 20 }, (_, i) => ({ x: i * 0.7, y: 1, step: i, t: i }));
  assert.deepStrictEqual(mlfChartScale([{ runId: RUN_A, points: timePts }], 300, 160).xTicks, [0, 5, 10, 15]);
  // 点が 1 つだけなら、その点までが読める目盛りになる
  const one = mlfChartScale([{ runId: RUN_A, points: [{ x: 3, y: 1 }] }], 300, 160);
  assert.deepStrictEqual(one.xTicks, [0, 1, 2, 3]);
}

// --- 目盛り・点の数字の書き方 ---

function testChartNum() {
  assert.strictEqual(mlfChartNum(0), "0");
  assert.strictEqual(mlfChartNum(0.8123456), "0.8123");
  assert.strictEqual(mlfChartNum(1.5), "1.5");
  assert.strictEqual(mlfChartNum(-0.8123456), "-0.8123");
  assert.strictEqual(mlfChartNum(12345), "12.3K"); // 4 桁を超えると K にする（saiAxisValue と同じ）
  assert.strictEqual(mlfChartNum(NaN), "0");
  assert.strictEqual(mlfChartNum(null), "0");
  assert.strictEqual(mlfChartNum("x"), "0");
  assert.strictEqual(mlfChartNum(0.0000123), "1.23e-5");
}

function testHasPoints() {
  // 「点が 1 つだけの指標」の判定（時系列が無いときの表示で使う）
  assert.strictEqual(mlfHasPoints([]), false);
  assert.strictEqual(mlfHasPoints([{ runId: RUN_A, points: [] }]), false);
  assert.strictEqual(mlfHasPoints([{ runId: RUN_A, points: [{ x: 0, y: 1 }] }]), false);
  assert.strictEqual(
    mlfHasPoints([{ runId: RUN_A, points: [{ x: 0, y: 1 }] }, { runId: RUN_B, points: [] }]),
    false
  );
  assert.strictEqual(
    mlfHasPoints([{ runId: RUN_A, points: [{ x: 0, y: 1 }] }, { runId: RUN_B, points: [{ x: 0, y: 2 }, { x: 1, y: 3 }] }]),
    true
  );
}

// --- 実行 ------------------------------------------------------------

const tests = [
  testSplitsRuns,
  testSortedByX,
  testDropsBadValues,
  testTimeIsSecondsFromFirst,
  testMissingRunIdIsOneSeries,
  testEmptyInput,
  testScaleOnePoint,
  testScaleSameY,
  testScaleSameX,
  testScaleZero,
  testScaleEmpty,
  testScaleNegative,
  testScaleBadSize,
  testPathD,
  testTickCountShrinksWithWidth,
  testTicksAreNice,
  testChartNum,
  testHasPoints,
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
  console.error(`test_mlfchart: ${failed} 件が失敗しました`);
  process.exit(1);
}
console.log(`test_mlfchart: ${tests.length} 件すべて通りました`);
