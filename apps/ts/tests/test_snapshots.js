"use strict";
// データセットのタブの純関数（既定の名前・来歴の 1 行・S か E の ID の形）のテスト。
// 実行:
//     node apps/ts/tests/test_snapshots.js
//
// 書き出し・一覧・写す・結ぶの描画は DOM が要るのでブラウザで確かめる。

const assert = require("assert");
const path = require("path");

const src = path.join(__dirname, "..", "src", "sai_ts", "static", "snapshots.js");
const { tsSnapDefaultName, tsSnapLineage, tsSnapNodeOk } = require(src);

// 既定の名前（月・日は 2 桁）
assert.strictEqual(tsSnapDefaultName("demo", "r01", new Date(2026, 9, 11)), "demo-r01-20261011");
assert.strictEqual(tsSnapDefaultName("demo", "r01", new Date(2026, 0, 3)), "demo-r01-20260103");

// 来歴の 1 行
assert.strictEqual(tsSnapLineage({ name: "ch01", source: "raw", derived: null }), "raw");
assert.strictEqual(
  tsSnapLineage({
    name: "ch01_env80-250",
    derived: {
      script: "bandpass",
      sha256: "1a2b3c4d5e6f" + "0".repeat(52),
      params: { low_hz: 80, high_hz: 250 },
      inputs: ["ch01"],
      run: "a-0123",
    },
  }),
  "bandpass @1a2b3c4d low_hz=80 high_hz=250 ← ch01（a-0123）",
);
assert.strictEqual(
  tsSnapLineage({ derived: { script: "", sha256: "", params: { mode: "x" }, inputs: [], run: "" } }),
  "mode=x",
  "空の項目は出さない",
);
assert.strictEqual(
  tsSnapLineage({ derived: { script: "s", sha256: "ab", params: { w: [1, 2] }, inputs: ["a", "b"], run: "" } }),
  "s @ab w=[1,2] ← a, b",
);

// S か E の ID
for (const ok of ["S1", "S3", "E12", "E999999999"]) assert.ok(tsSnapNodeOk(ok), ok);
for (const ng of ["", "S", "S0", "E01", "X3", "s3", " S3", "S3a", "E1234567890", null, undefined]) {
  assert.ok(!tsSnapNodeOk(ng), String(ng));
}

console.log("test_snapshots.js: OK");
