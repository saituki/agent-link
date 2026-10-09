"use strict";
// sai-ts の画面の骨組み（app.js）の純関数のテスト。
// 実行:
//     node apps/ts/tests/test_app.js
//
// DOM が要る描画・操作はブラウザで確かめる。

const assert = require("assert");
const path = require("path");

const src = path.join(__dirname, "..", "src", "sai_ts", "static", "app.js");
const {
  tsValidName,
  tsClampRange,
  tsZoomAt,
  tsPan,
  tsNiceTicks,
  tsDuration,
  tsStripToken,
  tsStreamUrl,
  tsViewBody,
  tsPatchState,
  tsRecordsOf,
  tsNormSchema,
} = require(src);

// 名前の形（C1）
assert.strictEqual(tsValidName("r01"), true);
assert.strictEqual(tsValidName("a".repeat(64)), true);
assert.strictEqual(tsValidName("a".repeat(65)), false);
assert.strictEqual(tsValidName("../x"), false);
assert.strictEqual(tsValidName("a..b"), false);
assert.strictEqual(tsValidName("-a"), false);
assert.strictEqual(tsValidName(""), false);
assert.strictEqual(tsValidName("a\n"), false);

// 範囲の切り詰め: 端、最小幅、記録より短い場合
assert.deepStrictEqual(tsClampRange(-5, 5, 100, 1), { t0: 0, t1: 10 });
assert.deepStrictEqual(tsClampRange(95, 105, 100, 1), { t0: 90, t1: 100 });
assert.deepStrictEqual(tsClampRange(50, 50.2, 100, 10), { t0: 50, t1: 60 });
assert.deepStrictEqual(tsClampRange(0, 5, 3, 10), { t0: 0, t1: 3 });

// 拡大縮小: 中心を保つ、記録の端、最小幅
assert.deepStrictEqual(tsZoomAt(0, 10, 5, 0.5, 100, 0), { t0: 2.5, t1: 7.5 });
assert.deepStrictEqual(tsZoomAt(0, 10, 0, 0.5, 100, 0), { t0: 0, t1: 5 });
assert.deepStrictEqual(tsZoomAt(90, 100, 100, 0.5, 100, 0), { t0: 95, t1: 100 });
const tiny = tsZoomAt(0, 10, 5, 0.0001, 100, 0.5);
assert.ok(Math.abs(tiny.t1 - tiny.t0 - 0.5) < 1e-9, "最小幅は 0.5 秒を下回らない");

// 移動: 幅を保つ、端で止まる
assert.deepStrictEqual(tsPan(0, 10, -3, 100), { t0: 0, t1: 10 });
assert.deepStrictEqual(tsPan(90, 100, 5, 100), { t0: 90, t1: 100 });
assert.deepStrictEqual(tsPan(10, 20, 5, 100), { t0: 15, t1: 25 });

// 目盛り: 1・2・5 × 10^k
assert.deepStrictEqual(tsNiceTicks(0, 10, 5), [0, 2, 4, 6, 8, 10]);
assert.deepStrictEqual(tsNiceTicks(0, 1, 5), [0, 0.2, 0.4, 0.6, 0.8, 1]);
assert.deepStrictEqual(tsNiceTicks(0, 0, 5), []);

// 記録の長さ
assert.strictEqual(tsDuration({ srate: 500, n_samples: 1000 }), 2);
assert.strictEqual(tsDuration(null), 0);

// URL の token
assert.deepStrictEqual(tsStripToken("?token=abc&x=1"), { token: "abc", query: "?x=1" });
assert.deepStrictEqual(tsStripToken("?token=abc"), { token: "abc", query: "" });
assert.deepStrictEqual(tsStripToken(""), { token: "", query: "" });
assert.strictEqual(tsStreamUrl("a b"), "/api/stream?token=a%20b");
assert.strictEqual(tsStreamUrl(""), "/api/stream");

// 状態の写し（C6）
const st = {
  dataset: "demo",
  record: "r01",
  t0: 0,
  t1: 10,
  vars: ["ch01"],
  hiddenCategories: new Set(),
  selection: null,
  selectedEvents: [],
};
assert.strictEqual(tsPatchState(st, { t0: 1 }), false, "記録が変わらなければ false");
assert.strictEqual(st.t0, 1);
assert.strictEqual(tsPatchState(st, { record: "r02" }), true, "記録が変わったら true");
assert.ok(st.hiddenCategories instanceof Set);
tsPatchState(st, { hiddenCategories: ["noise"], selection: { t0: 1, t1: 2, vars: ["ch01"] } });
assert.ok(st.hiddenCategories.has("noise"));
assert.deepStrictEqual(st.selection, { t0: 1, t1: 2, vars: ["ch01"] });
assert.strictEqual(tsPatchState(st, { vars: null }), false);
assert.strictEqual(st.vars, null, "null は「まだ決めていない」を残す");

// PUT /api/view の本文（C6）: 空の項目は送らない
const body = tsViewBody(Object.assign({}, st, { dataset: null, selectedEvents: ["e-1"] }));
assert.deepStrictEqual(body.hidden_categories, ["noise"]);
assert.deepStrictEqual(body.selected_events, ["e-1"]);
assert.ok(!("dataset" in body));
assert.ok(!("vars" in body));

assert.deepStrictEqual(tsRecordsOf([{ name: "demo", records: ["r01"] }], "demo"), ["r01"]);
assert.deepStrictEqual(tsRecordsOf([], "x"), []);
assert.deepStrictEqual(tsNormSchema(null), { version: 1, categories: [], metrics: [] });

console.log("test_app.js: OK");
