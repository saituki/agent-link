"use strict";
// 検索セットの画面（sciset.js）の純関数のテスト（U21）。
// 実行: node apps/web/tests/test_sciset.js

const assert = require("assert");
const path = require("path");

const { ssParseHash, ssCountsLabel, ssFilter, ssMarkBody } = require(
  path.join(__dirname, "..", "src", "sai_web", "static", "sciset.js"),
);

// 住所
assert.deepStrictEqual(ssParseHash("#/papers/sets"), {});
assert.deepStrictEqual(ssParseHash("#/papers/sets/12"), { setId: 12 });
assert.deepStrictEqual(ssParseHash("#/papers/sets/999999999"), { setId: 999999999 });
for (const h of ["#/papers/sets/0", "#/papers/sets/012", "#/papers/sets/", "#/papers/sets/x",
  "#/papers/sets/1/2", "#/papers/sets/1000000000", "#/papers/read/1", "", null]) {
  assert.strictEqual(ssParseHash(h), null, String(h));
}

// 件数
assert.strictEqual(ssCountsLabel({ pending: 11, keep: 3, drop: 20, imported: 0 }), "未選別 11 / keep 3 / drop 20");
assert.strictEqual(
  ssCountsLabel({ pending: 11, keep: 3, drop: 20, imported: 1 }),
  "未選別 11 / keep 3 / drop 20 / 取り込み済み 1",
);
assert.strictEqual(ssCountsLabel(null), "未選別 0 / keep 0 / drop 0");

// 絞り込み
const items = [
  { position: 1, decision: "pending" },
  { position: 2, decision: "keep" },
  { position: 3, decision: "drop" },
  { position: 4, decision: "imported" },
];
assert.deepStrictEqual(ssFilter(items, "pending").map((x) => x.position), [1]);
assert.deepStrictEqual(ssFilter(items, "drop").map((x) => x.position), [3]);
assert.deepStrictEqual(ssFilter(items, "all").map((x) => x.position), [1, 2, 3, 4]);
assert.deepStrictEqual(ssFilter(null, "all"), []);

// mark の本文
assert.deepStrictEqual(ssMarkBody([1, 2], "keep", ""), { positions: [1, 2], decision: "keep" });
assert.deepStrictEqual(ssMarkBody([3], "drop", " 範囲外 "), { positions: [3], decision: "drop", reason: "範囲外" });
assert.strictEqual(ssMarkBody([], "keep", ""), null);
assert.strictEqual(ssMarkBody(Array.from({ length: 501 }, (_, i) => i + 1), "keep", ""), null);
assert.notStrictEqual(ssMarkBody(Array.from({ length: 500 }, (_, i) => i + 1), "keep", ""), null);
assert.strictEqual(ssMarkBody([1], "pending", ""), null);
assert.strictEqual(ssMarkBody([1], "keep", "あ".repeat(201)), null);
assert.notStrictEqual(ssMarkBody([1], "keep", "あ".repeat(200)), null);

console.log("test_sciset: すべて通りました");
