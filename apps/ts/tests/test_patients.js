"use strict";
// 患者タブ（patients.js）の純関数のテスト。
// 実行:
//     node apps/ts/tests/test_patients.js
//
// 一覧の押下と情報の表の描画は DOM が要るのでブラウザで確かめる。

const assert = require("assert");
const path = require("path");

const src = path.join(__dirname, "..", "src", "sai_ts", "static", "patients.js");
const { tsFilterRecords, tsPatientRows } = require(src);

// 絞り込み: 部分一致、大文字小文字を区別しない、前後の空白は無視、空なら全部
const names = ["P001", "P002", "Alpha-01", "beta"];
assert.deepStrictEqual(tsFilterRecords(names, ""), names);
assert.deepStrictEqual(tsFilterRecords(names, undefined), names);
assert.deepStrictEqual(tsFilterRecords(names, "  "), names);
assert.deepStrictEqual(tsFilterRecords(names, "p00"), ["P001", "P002"]);
assert.deepStrictEqual(tsFilterRecords(names, "ALPHA"), ["Alpha-01"]);
assert.deepStrictEqual(tsFilterRecords(names, " 01 "), ["P001", "Alpha-01"]);
assert.deepStrictEqual(tsFilterRecords(names, "zzz"), []);
assert.deepStrictEqual(tsFilterRecords([], "a"), []);
// 入力の配列を書き換えない
tsFilterRecords(names, "p");
assert.deepStrictEqual(names, ["P001", "P002", "Alpha-01", "beta"]);

// 情報の表: info の key/value（文字・数・それ以外は JSON）、続けて srate・長さ・変数の数
const meta = {
  version: 1,
  srate: 256,
  n_samples: 153600,
  vars: [{ name: "ch01" }, { name: "ch02" }],
  info: { age: 63, diagnosis: "focal epilepsy", tags: ["a", "b"], ok: true, memo: null },
};
assert.deepStrictEqual(tsPatientRows(meta), [
  ["age", "63"],
  ["diagnosis", "focal epilepsy"],
  ["tags", '["a","b"]'],
  ["ok", "true"],
  ["memo", "null"],
  ["srate", "256 Hz"],
  ["長さ", "600.000 s"],
  ["変数", "2 個"],
]);

// info が無い・object でない・配列のときは info の行を出さない（諸元は出す）
const bare = { version: 1, srate: 500, n_samples: 500, vars: [] };
assert.deepStrictEqual(tsPatientRows(bare), [
  ["srate", "500 Hz"],
  ["長さ", "1.000 s"],
  ["変数", "0 個"],
]);
assert.deepStrictEqual(tsPatientRows(Object.assign({}, bare, { info: "x" })).length, 3);
assert.deepStrictEqual(tsPatientRows(Object.assign({}, bare, { info: [1, 2] })).length, 3);
assert.deepStrictEqual(tsPatientRows(Object.assign({}, bare, { info: {} })).length, 3, "空の info");

// 記録が無いとき（読めていない・選んでいない）は空
assert.deepStrictEqual(tsPatientRows(null), []);

console.log("test_patients.js: OK");
