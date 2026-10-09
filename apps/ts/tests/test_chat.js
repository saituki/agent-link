"use strict";
// チャットの純関数（コードの囲みの分け方・選択の要約・増えた分）のテスト。
// 実行:
//     node apps/ts/tests/test_chat.js
//
// 吹き出しの描画と送受信は DOM と sai-ts サーバが要るのでブラウザで確かめる（kantoku が統合のあとに行う）。

const assert = require("assert");
const path = require("path");

const src = path.join(__dirname, "..", "src", "sai_ts", "static", "chat.js");
const { tsSplitCode, tsSelectionSummary, tsChatDiff, tsChatStarted, tsChatStartLabel, tsChatMaxChars } = require(src);

// 囲みが無い文は 1 つの文字の塊
assert.deepStrictEqual(tsSplitCode("こんにちは\n2 行目"), [{ kind: "text", lang: "", text: "こんにちは\n2 行目" }]);

// 囲みで分ける。言語名は囲みの後ろ
assert.deepStrictEqual(tsSplitCode("前\n```python\nx = 1\n```\n後"), [
  { kind: "text", lang: "", text: "前" },
  { kind: "code", lang: "python", text: "x = 1" },
  { kind: "text", lang: "", text: "後" },
]);

// 閉じていない囲みは終わりまでコード（GFM と同じ）
assert.deepStrictEqual(tsSplitCode("前\n```\n<b>x</b>\n途中"), [
  { kind: "text", lang: "", text: "前" },
  { kind: "code", lang: "", text: "<b>x</b>\n途中" },
]);

// 囲みの中の文字は HTML として扱わない（分けるだけ）。空の文字は出さない
assert.deepStrictEqual(tsSplitCode("```\n```"), [{ kind: "code", lang: "", text: "" }]);
assert.deepStrictEqual(tsSplitCode(""), []);
assert.deepStrictEqual(tsSplitCode("  \n"), []);

// 囲みが 2 つ続く
const two = tsSplitCode("```a\n1\n```\n```b\n2\n```");
assert.deepStrictEqual(
  two.map((p) => [p.kind, p.lang, p.text]),
  [
    ["code", "a", "1"],
    ["code", "b", "2"],
  ],
);

// 選択の要約。範囲は小数 1 桁、変数は ", " でつなぐ
assert.strictEqual(
  tsSelectionSummary({
    record: "r01",
    selection: { t0: 12, t1: 13.5, vars: ["Fp1"] },
    selectedEvents: ["e-1"],
  }),
  "選択: r01 12.0–13.5 秒 Fp1 / イベント 1 件",
);
assert.strictEqual(
  tsSelectionSummary({ record: "r01", selection: { t0: 1, t1: 2, vars: ["Fp1", "F3"] }, selectedEvents: [] }),
  "選択: r01 1.0–2.0 秒 Fp1, F3",
);
// 選択が無くてイベントだけ
assert.strictEqual(tsSelectionSummary({ record: "r01", selection: null, selectedEvents: ["a", "b"] }), "選択: なし / イベント 2 件");
// 何も無い
assert.strictEqual(tsSelectionSummary({ record: "r01", selection: null }), "選択: なし");

// 増えた分。順番を保つ。減ったら空
const a = { role: "user", n: 1, text: "a" };
const b = { role: "assistant", n: 1, text: "b" };
const c = { role: "user", n: 2, text: "c" };
assert.deepStrictEqual(tsChatDiff([], [a, b]), [a, b]);
assert.deepStrictEqual(tsChatDiff([a], [a, b, c]), [b, c]);
assert.deepStrictEqual(tsChatDiff([a, b], [a, b]), []);
assert.deepStrictEqual(tsChatDiff([a, b], [a]), []);

// 始まっているか: run_id が無い 200（チャットが無い）は「まだ始まっていない」。ボタンは「チャットを始める」
assert.strictEqual(tsChatStarted({ run_id: null, status: null, messages: [] }), false);
assert.strictEqual(tsChatStarted({ messages: [] }), false);
assert.strictEqual(tsChatStarted({ run_id: 7, status: "running", messages: [] }), true);
assert.strictEqual(tsChatStartLabel(tsChatStarted({ run_id: null, status: null, messages: [] })), "チャットを始める");
assert.strictEqual(tsChatStartLabel(tsChatStarted({ run_id: 7, status: "done", messages: [] })), "チャットは終わっています。始め直す");

assert.strictEqual(tsChatMaxChars, 20000);

if (__filename === process.argv[1]) console.log("test_chat: ok");
