"use strict";
// イベントの入力欄（evform.js）の純関数のテスト。
// 実行:
//     node apps/ts/tests/test_evform.js

const assert = require("assert");
const path = require("path");

const src = path.join(__dirname, "..", "src", "sai_ts", "static", "evform.js");
const { tsMetricInput, tsBuildEvent, tsPlacePanel, tsDraftFromEvent, tsNewDraft } = require(src);

// メトリクスの入力部品
assert.deepStrictEqual(tsMetricInput({ name: "amp", type: "number", min: 0, max: 100, unit: "uV", desc: "倍率" }), {
  control: "number",
  step: "any",
  min: 0,
  max: 100,
  unit: "uV",
  desc: "倍率",
});
assert.deepStrictEqual(tsMetricInput({ name: "n", type: "integer" }), { control: "number", step: 1, unit: "", desc: "" });
assert.deepStrictEqual(tsMetricInput({ name: "kind", type: "enum", values: ["emg", "move"] }).values, ["emg", "move"]);
assert.strictEqual(tsMetricInput({ name: "ok", type: "boolean" }).control, "checkbox");
assert.strictEqual(tsMetricInput({ name: "memo", type: "string" }).control, "text");

const schema = {
  version: 1,
  categories: [{ name: "noise", color: "#d9534f", visible: true, desc: "" }],
  metrics: [
    { name: "amp", type: "number", min: 0, max: 100 },
    { name: "n", type: "integer" },
    { name: "kind", type: "enum", values: ["emg", "move"] },
    { name: "ok", type: "boolean" },
    { name: "memo", type: "string" },
  ],
};
const base = { type: "line", t: "", t0: "", t1: "", vars: [], category: "noise", text: "", metrics: {} };
const draft = { record: "r01" };

// 正しい入力: 数値の変換、空は送らない、boolean は常に送る
const ok = tsBuildEvent(
  Object.assign({}, base, {
    t: "40.25",
    vars: ["Fp1"],
    text: "前頭部",
    metrics: { amp: "3.2", n: "", kind: "emg", ok: false, memo: "" },
  }),
  draft,
  schema,
);
assert.strictEqual(ok.ok, true);
assert.deepStrictEqual(ok.json, {
  type: "line",
  record: "r01",
  vars: ["Fp1"],
  t: 40.25,
  category: "noise",
  text: "前頭部",
  metrics: { amp: 3.2, ok: false, kind: "emg" },
});

// エリア: 開始と終了を送る。終了が開始以下なら誤り
const area = tsBuildEvent(Object.assign({}, base, { type: "area", t0: "12.3", t1: "13.1" }), draft, schema);
assert.deepStrictEqual([area.json.t0, area.json.t1], [12.3, 13.1]);
assert.ok(!("t" in area.json));
const areaBad = tsBuildEvent(Object.assign({}, base, { type: "area", t0: "13", t1: "13" }), draft, schema);
assert.strictEqual(areaBad.ok, false);
assert.match(areaBad.error, /終了は開始より後/);

// 端の値: 空の時刻、数でない文字、記録が無い、カテゴリが無い
assert.strictEqual(tsBuildEvent(base, draft, schema).ok, false);
assert.strictEqual(tsBuildEvent(Object.assign({}, base, { t: "abc" }), draft, schema).ok, false);
assert.strictEqual(tsBuildEvent(Object.assign({}, base, { t: "1" }), {}, schema).ok, false);
assert.strictEqual(tsBuildEvent(Object.assign({}, base, { t: "1", category: "x" }), draft, schema).ok, false);

// メトリクスの誤り: 範囲外・整数でない・数でない・選択肢の外
const withMetric = (metrics) => tsBuildEvent(Object.assign({}, base, { t: "1", metrics }), draft, schema);
assert.match(withMetric({ amp: "101" }).error, /100 以下/);
assert.match(withMetric({ amp: "-1" }).error, /0 以上/);
assert.match(withMetric({ amp: "abc" }).error, /数値/);
assert.match(withMetric({ n: "2.5" }).error, /整数/);
assert.match(withMetric({ kind: "other" }).error, /選択肢/);
assert.strictEqual(withMetric({ kind: "" }).ok, true, "enum の空は送らない");

// 配置: 画面の外にはみ出さない
const p1 = tsPlacePanel({ x: 1270, y: 880 }, 320, 400, 1280, 900);
assert.ok(p1.left >= 8 && p1.left <= 1280 - 320 - 8, "右の端でも外へ出ない");
assert.ok(p1.top >= 8 && p1.top <= 900 - 400 - 8, "下の端でも外へ出ない");
const p2 = tsPlacePanel({ x: 100, y: 100 }, 320, 200, 1280, 900);
assert.deepStrictEqual(p2, { left: 112, top: 112 });
assert.deepStrictEqual(tsPlacePanel(null, 320, 200, 1280, 900).top, 56);

// 下書き: イベントから、新規から
assert.deepStrictEqual(
  tsDraftFromEvent({ type: "line", record: "r01", t: 3, vars: ["a"], category: "spike", text: "x", metrics: { amp: 1 } }),
  { type: "line", record: "r01", t: 3, t0: undefined, t1: undefined, vars: ["a"], category: "spike", text: "x", metrics: { amp: 1 } },
);
const nd = tsNewDraft({ type: "area", t0: 5, t1: 9, vars: ["a"] }, "r01", schema);
assert.deepStrictEqual([nd.type, nd.t, nd.t0, nd.t1, nd.category], ["area", 5, 5, 9, "noise"]);
assert.strictEqual(tsNewDraft(null, "r01", { categories: [], metrics: [] }).category, "");

console.log("test_evform.js: OK");
