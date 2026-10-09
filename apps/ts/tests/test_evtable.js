"use strict";
// イベント表の純関数（絞り込み・並べ替え・表示範囲）のテスト。
// 実行:
//     node apps/ts/tests/test_evtable.js
//
// 表の描画と行の操作は DOM が要るのでブラウザで確かめる（kantoku が統合のあとに行う）。

const assert = require("assert");
const path = require("path");

const src = path.join(__dirname, "..", "src", "sai_ts", "static", "evtable.js");
const {
  tsEventTime,
  tsEventSpanText,
  tsEventVarsText,
  tsFilterEvents,
  tsSortEvents,
  tsFocusRange,
  tsEventKindMark,
  tsEvtWho,
} = require(src);

const line = (id, record, t, extra = {}) => ({
  id,
  type: "line",
  record,
  t,
  vars: [],
  category: "",
  text: "",
  source: "human",
  ...extra,
});
const area = (id, record, t0, t1, extra = {}) => ({
  id,
  type: "area",
  record,
  t0,
  t1,
  vars: [],
  category: "",
  text: "",
  source: "human",
  ...extra,
});

const EVS = [
  line("e1", "r01", 40.25, { category: "spike", text: "大きい", source: "agent" }),
  area("e2", "r01", 12.3, 13.1, { category: "noise", text: "前頭部の広帯域", source: "human", vars: ["Fp1", "F3"] }),
  line("e3", "r02", 5, { category: "spike", text: "", source: "script:bp@1a2b3c4d" }),
  area("e4", "r02", 5, 9, { category: "noise", text: "Spike 群", source: "human" }),
];
const ids = (xs) => xs.map((e) => e.id);

// 絞り込みの組み合わせ
const none = { record: null, category: "", source: "", q: "" };
assert.deepStrictEqual(ids(tsFilterEvents(EVS, none)), ["e1", "e2", "e3", "e4"]);
assert.deepStrictEqual(ids(tsFilterEvents(EVS, { ...none, record: "r01" })), ["e1", "e2"]);
assert.deepStrictEqual(ids(tsFilterEvents(EVS, { ...none, category: "spike" })), ["e1", "e3"]);
assert.deepStrictEqual(ids(tsFilterEvents(EVS, { ...none, record: "r02", category: "spike" })), ["e3"]);
// 作成者: script:<name>@… は script に入る
assert.deepStrictEqual(ids(tsFilterEvents(EVS, { ...none, source: "script" })), ["e3"]);
assert.deepStrictEqual(ids(tsFilterEvents(EVS, { ...none, source: "human" })), ["e2", "e4"]);
assert.deepStrictEqual(ids(tsFilterEvents(EVS, { ...none, source: "agent" })), ["e1"]);
// 文字の検索はテキストとカテゴリ。大文字小文字を区別しない。前後の空白は無視
assert.deepStrictEqual(ids(tsFilterEvents(EVS, { ...none, q: "  spike " })), ["e1", "e3", "e4"]);
assert.deepStrictEqual(ids(tsFilterEvents(EVS, { ...none, q: "広帯域" })), ["e2"]);
assert.deepStrictEqual(ids(tsFilterEvents(EVS, { ...none, q: "   " })), ["e1", "e2", "e3", "e4"]);
// 組み合わせ全部が効く（記録 AND カテゴリ AND 作成者 AND 文字）
assert.deepStrictEqual(
  ids(tsFilterEvents(EVS, { record: "r02", category: "noise", source: "human", q: "spike" })),
  ["e4"],
);
assert.deepStrictEqual(ids(tsFilterEvents(EVS, { record: "r02", category: "noise", source: "agent", q: "" })), []);
// 元の配列は変えない
assert.strictEqual(EVS.length, 4);

// 時刻: ライン は t、エリア は t0
assert.strictEqual(tsEventTime(EVS[0]), 40.25);
assert.strictEqual(tsEventTime(EVS[1]), 12.3);

// 並べ替え: 時刻の昇順・降順。同じ時刻は元の並びのまま（e3 と e4 は時刻 5）
assert.deepStrictEqual(ids(tsSortEvents(EVS, "time", 1)), ["e3", "e4", "e2", "e1"]);
assert.deepStrictEqual(ids(tsSortEvents(EVS, "time", -1)), ["e1", "e2", "e3", "e4"]);
// 同じ値は降順でも元の順（安定）
assert.deepStrictEqual(ids(tsSortEvents(EVS, "category", 1)), ["e2", "e4", "e1", "e3"]);
assert.deepStrictEqual(ids(tsSortEvents(EVS, "category", -1)), ["e1", "e3", "e2", "e4"]);
assert.deepStrictEqual(ids(tsSortEvents(EVS, "source", 1)), ["e1", "e2", "e4", "e3"]);
// 変数は | でつないで比べる
assert.deepStrictEqual(ids(tsSortEvents(EVS, "vars", 1)), ["e1", "e3", "e4", "e2"]);
// 並べ替えは入力を変えない
const copy = EVS.slice();
tsSortEvents(EVS, "text", 1);
assert.deepStrictEqual(EVS, copy);

// 表示: 時刻は小数 2 桁、エリアは範囲
assert.strictEqual(tsEventSpanText(EVS[0]), "40.25");
assert.strictEqual(tsEventSpanText(EVS[1]), "12.30–13.10");
// 変数: 3 つまで、残りは「ほか n」。空は記録全体
assert.strictEqual(tsEventVarsText([]), "（全体）");
assert.strictEqual(tsEventVarsText(["Fp1", "F3"]), "Fp1, F3");
assert.strictEqual(tsEventVarsText(["a", "b", "c", "d", "e"]), "a, b, c ほか 2");
assert.strictEqual(tsEventKindMark("line"), "ラ");
assert.strictEqual(tsEventKindMark("area"), "エ");

// 表示範囲: 中心 ± max(5, 長さ × 1.5)。記録 [0, 100]
const mid = tsFocusRange(line("x", "r", 50), 100);
assert.deepStrictEqual(mid, { t0: 45, t1: 55 });
// 長いエリアは 長さ × 1.5 が半幅（10 秒のエリア → 中心 45・半幅 15）
const long = tsFocusRange(area("y", "r", 40, 50), 100);
assert.deepStrictEqual(long, { t0: 30, t1: 60 });
// 記録の始まりに近いときは幅を保って右へずらす
assert.deepStrictEqual(tsFocusRange(line("x", "r", 2), 100), { t0: 0, t1: 10 });
// 記録の終わりに近いときは左へずらす
assert.deepStrictEqual(tsFocusRange(line("x", "r", 99), 100), { t0: 90, t1: 100 });
// 記録より幅が広いときは記録全体
assert.deepStrictEqual(tsFocusRange(line("x", "r", 3), 8), { t0: 0, t1: 8 });
// 長さ 0 のエリアでも半幅は 5 秒
assert.deepStrictEqual(tsFocusRange(area("z", "r", 50, 50), 100), { t0: 45, t1: 55 });

// 作成者の札（表の欄）: script:<name>@… は script、それ以外はそのまま
assert.strictEqual(tsEvtWho("script:bp@1a2b3c4d"), "script");
assert.strictEqual(tsEvtWho("human"), "human");

if (__filename === process.argv[1]) console.log("test_evtable: ok");
