"use strict";
// マウスの操作（mouse.js）の純関数のテスト。
// 実行:
//     node apps/ts/tests/test_mouse.js

const assert = require("assert");
const path = require("path");

const dir = path.join(__dirname, "..", "src", "sai_ts", "static");
// tsMouseDraft は lanes.js の tsSelectionFromDrag を使う（画面では同じ窓の中の関数）
global.tsSelectionFromDrag = require(path.join(dir, "lanes.js")).tsSelectionFromDrag;
const { tsMouseDraft } = require(path.join(dir, "mouse.js"));

// 4 本のレーン（100px ずつ）。時刻 0 が x=120、1 秒 = 10px
const frame = {
  t0: 0,
  t1: 100,
  plotW: 1000,
  toT: (x) => (x - 120) / 10,
  lanes: ["ch01", "ch02", "ch03", "ch04"].map((v, i) => ({ var: v, y0: i * 100, y1: (i + 1) * 100 })),
};
const single = { name: "HFO", type: "area", ch: "single" };
const all = { name: "artifact", type: "area", ch: "all" };
const point = { name: "spike", type: "line", ch: "single" };
const pointAll = { name: "spike", type: "line", ch: "all" };
const noCh = { name: "x", type: "area" };
const typeless = { name: "x" };
const a = { x: 320, y: 150 }; // t=20・ch02
const b = { x: 520, y: 350 }; // t=40・ch04

// 例 1: single のエリアを ch02 から ch04 までドラッグ → ch02 だけ
assert.deepStrictEqual(tsMouseDraft(single, a, b, frame, true), { type: "area", t0: 20, t1: 40, vars: ["ch02"] });
// all のエリアは記録全体（vars は空）
assert.deepStrictEqual(tsMouseDraft(all, a, b, frame, true), { type: "area", t0: 20, t1: 40, vars: [] });
// ch 無しのエリアは触れたレーン全部
assert.deepStrictEqual(tsMouseDraft(noCh, a, b, frame, true).vars, ["ch02", "ch03", "ch04"]);
// 例 2: all のポイントをクリック → 記録全体
assert.deepStrictEqual(tsMouseDraft(pointAll, a, a, frame, false), { type: "line", t: 20, vars: [] });
// single のポイントは、クリックしたレーンだけ
assert.deepStrictEqual(tsMouseDraft(point, a, a, frame, false), { type: "line", t: 20, vars: ["ch02"] });
// ch 無しのポイントは、クリックしたレーン
assert.deepStrictEqual(tsMouseDraft({ name: "x", type: "line" }, a, a, frame, false), { type: "line", t: 20, vars: ["ch02"] });
// ch 無しのエリアをクリック → エラー
assert.deepStrictEqual(tsMouseDraft(noCh, a, a, frame, false), { error: "この定義はドラッグで作ります" });
// 例 3: point の定義でドラッグ → エラー
assert.deepStrictEqual(tsMouseDraft(point, a, b, frame, true), { error: "この定義はクリックで作ります" });
// エリアの定義をクリック → エラー（クリックはイベントの選びに使う）
assert.deepStrictEqual(tsMouseDraft(single, a, a, frame, false), { error: "この定義はドラッグで作ります" });
// type の無い定義: クリックはポイント、ドラッグはエリア
assert.deepStrictEqual(tsMouseDraft(typeless, a, a, frame, false), { type: "line", t: 20, vars: ["ch02"] });
assert.deepStrictEqual(tsMouseDraft(typeless, a, b, frame, true).type, "area");
// single で、レーンの外（y=450）から始めたら作らない
assert.ok(tsMouseDraft(single, { x: 320, y: 450 }, b, frame, true).error);
// 記録の外へ出たら範囲を収める（t0 は 0 まで）
assert.deepStrictEqual(tsMouseDraft(all, { x: 20, y: 150 }, b, frame, true), { type: "area", t0: 0, t1: 40, vars: [] });
// 幅ゼロのエリアは作らない
assert.ok(tsMouseDraft(single, a, a, frame, true).error);
// 定義が無いときは作らない
assert.ok(tsMouseDraft(null, a, a, frame, false).error);

console.log("test_mouse.js: OK");
