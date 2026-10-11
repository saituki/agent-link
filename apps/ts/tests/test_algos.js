"use strict";
// アルゴリズムの画面（algos.js）の純関数のテスト。
// 実行:
//     node apps/ts/tests/test_algos.js
//
// 欄・一覧・実行の記録の描画は DOM が要るのでブラウザで確かめる（kantoku が統合のあとに行う）。

const assert = require("assert");
const path = require("path");

const src = path.join(__dirname, "..", "src", "sai_ts", "static", "algos.js");
const { tsAlgoDefaults, tsAlgoRead, tsAlgoTargetIds, tsRunsFinished, tsAlgoFilter } = require(src);

// 既定の値は default のある名前だけ（boolean も default が無ければ入れない）
const bp = {
  name: "bandpass",
  params: [
    { name: "low_hz", type: "number", default: 80, min: 0, unit: "Hz" },
    { name: "high_hz", type: "number", default: 250, min: 0 },
    { name: "envelope", type: "boolean", default: true },
    { name: "mode", type: "enum", values: ["fft", "box"], default: "fft" },
    { name: "note", type: "string" },
    { name: "flag", type: "boolean" },
  ],
};
assert.deepStrictEqual(tsAlgoDefaults(bp), { low_hz: 80, high_hz: 250, envelope: true, mode: "fft" });
assert.deepStrictEqual(tsAlgoDefaults({ params: [] }), {});

// 検査: 既定で埋まる・空は送らない・boolean は常に送る
const ok = tsAlgoRead(bp, { low_hz: "100", high_hz: "", envelope: false, mode: "box", note: " hi ", flag: true });
assert.deepStrictEqual(ok, {
  ok: true,
  params: { low_hz: 100, envelope: false, mode: "box", note: "hi", flag: true },
});
assert.deepStrictEqual(tsAlgoRead(bp, { low_hz: "  ", envelope: true, mode: "", flag: false }), {
  ok: true,
  params: { envelope: true, flag: false },
}, "空の数値・enum は送らない");

// 型・範囲・選択肢の誤り（G2-C5 と同じ文言の形）
const n = { name: "n", type: "integer", min: 1, max: 5 };
assert.deepStrictEqual(tsAlgoRead({ params: [n] }, { n: "2.5" }), { ok: false, error: "n: 整数を入れてください" });
assert.deepStrictEqual(tsAlgoRead({ params: [n] }, { n: "0" }), { ok: false, error: "n: 1 以上にしてください" });
assert.deepStrictEqual(tsAlgoRead({ params: [n] }, { n: "9" }), { ok: false, error: "n: 5 以下にしてください" });
assert.deepStrictEqual(tsAlgoRead({ params: [n] }, { n: "abc" }), { ok: false, error: "n: 数値を入れてください" });
assert.deepStrictEqual(tsAlgoRead(bp, { mode: "zzz" }), { ok: false, error: "mode: 選択肢にない値です" });
assert.deepStrictEqual(tsAlgoRead({ params: [{ name: "s", type: "string" }] }, { s: "x".repeat(2001) }), {
  ok: false,
  error: "s: 2000 字までです",
});
assert.deepStrictEqual(tsAlgoRead({ params: [] }, {}), { ok: true, params: {} });

// 対象の ID（selected・all・category・range）
const st = {
  selectedEvents: ["e-1", "e-9"],
  t0: 10,
  t1: 20,
  events: [
    { id: "e-1", type: "line", t: 5, category: "HFO" },
    { id: "e-2", type: "line", t: 12, category: "HFO" },
    { id: "e-3", type: "area", t0: 18, t1: 25, category: "spike" },
    { id: "e-4", type: "area", t0: 30, t1: 31, category: "HFO" },
  ],
};
assert.deepStrictEqual(tsAlgoTargetIds("selected", st, ""), ["e-1", "e-9"]);
assert.deepStrictEqual(tsAlgoTargetIds("all", st, ""), [], "全部は空（記録の全部の約束）");
assert.deepStrictEqual(tsAlgoTargetIds("category", st, "HFO"), ["e-1", "e-2", "e-4"]);
assert.deepStrictEqual(tsAlgoTargetIds("range", st, ""), ["e-2", "e-3"], "表示中の時間と重なるもの（エリアは端を含む）");
assert.deepStrictEqual(tsAlgoTargetIds("selected", { events: [] }, ""), [], "選んでいなければ空");
// 選んだ配列の写しを返す（入力を書き換えない）
const picked = tsAlgoTargetIds("selected", st, "");
picked.push("x");
assert.deepStrictEqual(st.selectedEvents, ["e-1", "e-9"]);

// 終わった実行（running → ok）だけを返す
const prev = [
  { id: "a", status: "running" },
  { id: "b", status: "running" },
  { id: "c", status: "ok" },
  { id: "d", status: "running" },
];
const next = [
  { id: "a", status: "ok", result: { vars: ["x"] } },
  { id: "b", status: "error" },
  { id: "c", status: "ok" },
  { id: "e", status: "ok" },
  { id: "d", status: "running" },
];
assert.deepStrictEqual(
  tsRunsFinished(prev, next).map((r) => r.id),
  ["a"],
  "running から ok になったものだけ（新しく見えた ok は含めない）",
);
assert.deepStrictEqual(tsRunsFinished([], []), []);
assert.deepStrictEqual(tsRunsFinished(undefined, undefined), []);

// 種類の絞り込み（"" は全部）・入力を書き換えない
const algos = [
  { name: "a", kind: "channel" },
  { name: "b", kind: "event" },
  { name: "c", kind: "measure" },
];
assert.deepStrictEqual(tsAlgoFilter(algos, "").map((x) => x.name), ["a", "b", "c"]);
assert.deepStrictEqual(tsAlgoFilter(algos, "event").map((x) => x.name), ["b"]);
assert.deepStrictEqual(tsAlgoFilter(algos, "none"), []);
assert.strictEqual(algos.length, 3);

if (__filename === process.argv[1]) console.log("test_algos: ok");
