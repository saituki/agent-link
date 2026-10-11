"use strict";
// ch タブの純関数（並べ替え tsChMove・保存の形 tsChNext）のテスト。
// 実行:
//     node apps/ts/tests/test_chpanel.js
//
// 行の描画・ドラッグは DOM が要るのでブラウザで確かめる（kantoku が統合のあとに行う）。

const assert = require("assert");
const path = require("path");

const src = path.join(__dirname, "..", "src", "sai_ts", "static", "chpanel.js");
const { tsChMove, tsChNext, tsChSpecErr } = require(src);

// 並べ替え
assert.deepStrictEqual(tsChMove(["a", "b", "c"], 0, 2), ["b", "c", "a"]);
assert.deepStrictEqual(tsChMove(["a", "b", "c"], 2, 0), ["c", "a", "b"]);
assert.deepStrictEqual(tsChMove(["a", "b", "c"], 1, 1), ["a", "b", "c"], "同じ位置は動かさない");
assert.deepStrictEqual(tsChMove(["a", "b", "c"], 0, 3), ["a", "b", "c"], "範囲の外は動かさない");
assert.deepStrictEqual(tsChMove(["a", "b", "c"], -1, 1), ["a", "b", "c"]);
const orig = ["a", "b"];
tsChMove(orig, 0, 1);
assert.deepStrictEqual(orig, ["a", "b"], "元の配列は書き換えない");

// 保存の形（G1-C1）。merged は tsMergeChannels の 1 件（drawColor・unit を含む）
const m = (name, extra = {}) => ({
  name,
  label: "",
  color: "",
  drawColor: "#7dd3fc",
  gain: 1,
  spectrum: null,
  unit: "µV",
  ...extra,
});
const merged = [m("a"), m("b", { label: "Fp1", color: "#fca5a5", gain: 0.5 })];

// 並びの指定があれば、その順。stored にあって今の記録に無いものは後ろに残す（AC2）
const stored = [{ name: "x", label: "old", color: "#86efac", gain: 2 }];
assert.deepStrictEqual(tsChNext(stored, merged, {}, ["b", "a"]), [
  { name: "b", label: "Fp1", color: "#fca5a5", gain: 0.5 },
  { name: "a", label: "", color: "", gain: 1 },
  { name: "x", label: "old", color: "#86efac", gain: 2 },
]);

// 並びの指定が無ければ merged の順。palette の色（drawColor）は保存しない
assert.deepStrictEqual(tsChNext([], merged, {}, null), [
  { name: "a", label: "", color: "", gain: 1 },
  { name: "b", label: "Fp1", color: "#fca5a5", gain: 0.5 },
]);

// patch は当てた項目だけ変える。他は今の値のまま
assert.deepStrictEqual(tsChNext([], merged, { a: { color: "#123456" }, b: { label: "" } }, null), [
  { name: "a", label: "", color: "#123456", gain: 1 },
  { name: "b", label: "", color: "#fca5a5", gain: 0.5 },
]);

// 色を既定に戻す（空）と、倍率
assert.deepStrictEqual(tsChNext([], merged, { b: { color: "", gain: 3 } }, null)[1], {
  name: "b",
  label: "Fp1",
  color: "",
  gain: 3,
});

// 今の記録の ch の spectrum は残す。無いものは項目を作らない
const withSpec = [m("a", { spectrum: { on: true, fmin: 0, fmax: 250, win_ms: 250 } }), m("b")];
const out = tsChNext([], withSpec, {}, null);
assert.deepStrictEqual(out[0].spectrum, { on: true, fmin: 0, fmax: 250, win_ms: 250 });
assert.ok(!("spectrum" in out[1]));

// 並びの指定に無い名前・二重の名前は無視する。今の記録の ch は必ず 1 回ずつ入る
assert.deepStrictEqual(
  tsChNext([], merged, {}, ["zz", "b", "b"]).map((c) => c.name),
  ["b", "a"],
);

// 保存の中身を stored の項目から増やさない（stored の object を書き換えない）
const s2 = [{ name: "x", label: "", color: "", gain: 1 }];
tsChNext(s2, merged, {}, null)[2];
assert.deepStrictEqual(s2, [{ name: "x", label: "", color: "", gain: 1 }]);

// スペクトルの変更（patch の spectrum）は、その ch の spectrum を丸ごと置き換える。切ったあとも値は残る
const spOn = { on: true, fmin: 10, fmax: 120, win_ms: 500 };
assert.deepStrictEqual(tsChNext([], merged, { a: { spectrum: spOn } }, null)[0].spectrum, spOn);
const spOff = Object.assign({}, spOn, { on: false });
assert.deepStrictEqual(tsChNext([], withSpec, { a: { spectrum: spOff } }, null)[0].spectrum, spOff);

// スペクトルの値の検査（channels.py と同じ）
assert.strictEqual(tsChSpecErr({ fmin: 0, fmax: 250, win_ms: 250 }), "");
assert.notStrictEqual(tsChSpecErr({ fmin: -1, fmax: 250, win_ms: 250 }), "");
assert.notStrictEqual(tsChSpecErr({ fmin: 100, fmax: 100, win_ms: 250 }), "");
assert.notStrictEqual(tsChSpecErr({ fmin: 0, fmax: 250, win_ms: 0 }), "");
assert.notStrictEqual(tsChSpecErr({ fmin: 0, fmax: 250, win_ms: 10001 }), "");
assert.notStrictEqual(tsChSpecErr({ fmin: NaN, fmax: 250, win_ms: 250 }), "");

console.log("test_chpanel.js: OK");
