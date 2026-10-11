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
  tsBase,
  tsUrl,
  tsStripToken,
  tsStreamUrl,
  tsViewBody,
  tsPatchState,
  tsRecordsOf,
  tsNormSchema,
  tsRangeFromInput,
  tsClampSide,
  tsMergeChannels,
  tsOrderVisible,
  tsEvdefLabel,
  tsPalette,
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

// 住所の基準（C1）。node では location が無いので "/"
assert.strictEqual(tsBase(), "/");
assert.strictEqual(tsUrl("/api/view", "/"), "/api/view");
assert.strictEqual(tsUrl("/api/view", "/p/saituki-x/"), "/p/saituki-x/api/view");
assert.strictEqual(tsUrl("/ext/hello/main.js?v=1", "/p/saituki-x/"), "/p/saituki-x/ext/hello/main.js?v=1");
assert.strictEqual(tsUrl("/api/view"), "/api/view", "基準を省くと tsBase()（node では /）");
assert.strictEqual(tsUrl("api/view", "/p/saituki-x/"), "api/view", "/ で始まらない");
assert.strictEqual(tsUrl("//evil.example/x", "/p/saituki-x/"), "//evil.example/x", "//");
assert.strictEqual(tsUrl("https://x.example/y", "/p/saituki-x/"), "https://x.example/y", "http(s)");
assert.strictEqual(tsStreamUrl("a b", "/"), "/api/stream?token=a%20b");
assert.strictEqual(tsStreamUrl("a b", "/p/saituki-x/"), "/p/saituki-x/api/stream?token=a%20b");
assert.strictEqual(tsStreamUrl(""), "/api/stream");

// 合言葉: ?token= と #token= の両方。#token= が優先。token を消し、残りは残す（C1）
assert.deepStrictEqual(tsStripToken("?x=1", "#token=abc"), { token: "abc", query: "?x=1", hash: "" });
assert.deepStrictEqual(tsStripToken("?token=abc&x=1", ""), { token: "abc", query: "?x=1", hash: "" });
assert.deepStrictEqual(tsStripToken("", "#token=abc&k=2"), { token: "abc", query: "", hash: "#k=2" });
assert.deepStrictEqual(tsStripToken("", ""), { token: "", query: "", hash: "" });
assert.deepStrictEqual(tsStripToken("?token=old", "#token=new"), { token: "new", query: "", hash: "" });
assert.deepStrictEqual(tsStripToken("", "#section"), { token: "", query: "", hash: "#section" }, "token の無い断片は触らない");

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

// 記録が変わったら、前の記録の選択を消す（イベントの選択は残す）。選択を一緒に送れば残す
const sel = {
  dataset: "demo",
  record: "r02",
  t0: 0,
  t1: 10,
  vars: null,
  hiddenCategories: new Set(),
  selection: { t0: 1, t1: 2, vars: [] },
  selectedEvents: ["e-1"],
};
assert.strictEqual(tsPatchState(sel, { record: "r03" }), true);
assert.strictEqual(sel.selection, null, "記録が変わり selection が無ければ消す");
assert.deepStrictEqual(sel.selectedEvents, ["e-1"]);
tsPatchState(sel, { record: "r04", selection: { t0: 3, t1: 4, vars: [] } });
assert.deepStrictEqual(sel.selection, { t0: 3, t1: 4, vars: [] }, "一緒に送った selection はそのまま");
tsPatchState(sel, { t0: 5 });
assert.deepStrictEqual(sel.selection, { t0: 3, t1: 4, vars: [] }, "記録が変わらなければ残す");

// PUT /api/view の本文（C6）: 空の項目は送らない
const body = tsViewBody(Object.assign({}, st, { dataset: null, selectedEvents: ["e-1"] }));
assert.deepStrictEqual(body.hidden_categories, ["noise"]);
assert.deepStrictEqual(body.selected_events, ["e-1"]);
assert.ok(!("dataset" in body));
assert.ok(!("vars" in body));

// 範囲の入力（秒）: 開始と幅。記録の外は収め、数でない・幅が 0 以下・空・記録が無いときは null
assert.deepStrictEqual(tsRangeFromInput("12", "30", 600, 0.002), { t0: 12, t1: 42 });
assert.deepStrictEqual(tsRangeFromInput(" 12.5 ", "1e1", 600, 0.002), { t0: 12.5, t1: 22.5 }, "空白と指数");
assert.deepStrictEqual(tsRangeFromInput("590", "30", 600, 0.002), { t0: 570, t1: 600 }, "記録の末尾に収める");
assert.deepStrictEqual(tsRangeFromInput("-5", "10", 600, 0.002), { t0: 0, t1: 10 }, "負の開始は 0");
assert.deepStrictEqual(tsRangeFromInput("0", "5000", 600, 0.002), { t0: 0, t1: 600 }, "記録より広い幅は全体");
assert.deepStrictEqual(tsRangeFromInput("0", "0.0001", 600, 0.5), { t0: 0, t1: 0.5 }, "最小幅まで広げる");
assert.strictEqual(tsRangeFromInput("abc", "30", 600, 0), null, "数でない開始");
assert.strictEqual(tsRangeFromInput("1", "", 600, 0), null, "空の幅は 0 と読まない");
assert.strictEqual(tsRangeFromInput("", "5", 600, 0), null, "空の開始は 0 と読まない");
assert.strictEqual(tsRangeFromInput("1", "0", 600, 0), null, "幅 0");
assert.strictEqual(tsRangeFromInput("1", "-3", 600, 0), null, "負の幅");
assert.strictEqual(tsRangeFromInput("1", "Infinity", 600, 0), null, "有限でない幅");
assert.strictEqual(tsRangeFromInput("1", "5", 0, 0), null, "記録が無い");

assert.deepStrictEqual(tsRecordsOf([{ name: "demo", records: ["r01"] }], "demo"), ["r01"]);
assert.deepStrictEqual(tsRecordsOf([], "x"), []);
assert.deepStrictEqual(tsNormSchema(null), { version: 1, categories: [], metrics: [] });

// サイドバーの幅: 最小 260px、窓の幅の 60% まで（最小が勝つ）。読めない値は既定の 360px
assert.strictEqual(tsClampSide(100, 1280), 260);
assert.strictEqual(tsClampSide(900, 1280), 768);
assert.strictEqual(tsClampSide(360, 1280), 360);
assert.strictEqual(tsClampSide(500, 400), 260, "窓が狭くても最小 260px");
assert.strictEqual(tsClampSide(NaN, 1280), 360);

// ch 設定と記録の変数の合わせ（G1-C1）: 記録に無い名前は出さず、保存に無い変数は後ろへ。色が空なら並びの palette
const merged = tsMergeChannels(
  [
    { name: "b", label: "B", color: "", gain: 2 },
    { name: "zz", label: "", color: "", gain: 1 },
  ],
  [
    { name: "a", unit: "uV" },
    { name: "b", unit: "uV" },
  ],
);
assert.deepStrictEqual(merged, [
  { name: "b", label: "B", color: "", drawColor: tsPalette[0], gain: 2, spectrum: null, unit: "uV" },
  { name: "a", label: "", color: "", drawColor: tsPalette[1], gain: 1, spectrum: null, unit: "uV" },
]);
assert.strictEqual(tsMergeChannels(null, []).length, 0, "記録が無ければ空");
assert.strictEqual(
  tsMergeChannels([{ name: "a", color: "#123456" }], [{ name: "a" }])[0].drawColor,
  "#123456",
  "色が決まっていればその色",
);
const many = tsMergeChannels([], Array.from({ length: 11 }, (_, i) => ({ name: "c" + i })));
assert.strictEqual(many[10].drawColor, tsPalette[0], "palette は 10 色で巡回する");

// 表示の並び: channels の順。null は全部。記録に無い名前は落とす
const ch = [{ name: "b" }, { name: "a" }];
assert.deepStrictEqual(tsOrderVisible(ch, ["a", "b"]), ["b", "a"]);
assert.deepStrictEqual(tsOrderVisible(ch, null), ["b", "a"]);
assert.deepStrictEqual(tsOrderVisible(ch, ["x", "a"]), ["a"]);

// イベント定義の表示（G1-C2）。type と ch が無いときは受けるものを書く
assert.strictEqual(tsEvdefLabel({ name: "HFO", type: "area", ch: "single" }), "● HFO（エリア・1ch）");
assert.strictEqual(tsEvdefLabel({ name: "spike", type: "line", ch: "all" }), "● spike（ポイント・記録全体）");
assert.strictEqual(tsEvdefLabel({ name: "x" }), "● x（エリア/ポイント・自由）");

console.log("test_app.js: OK");
