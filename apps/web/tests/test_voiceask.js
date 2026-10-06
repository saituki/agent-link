"use strict";
// 質問カードの読み上げ・音声回答（U01 / voiceask.js）の DOM を使わない関数のテスト（assert だけ）。
// 確かめること:
// - vaSpeechText: 読み上げる文の組み立てと「(Recommended)」の読み替え
// - vaNormalize: 全角半角・空白・句読点を無視して比べられること
// - vaMatchAnswer: 番号（算用数字・漢数字・かな・数え方）・選択肢名・自由入力の判定
// 実行:
//     node tests/test_voiceask.js

const assert = require("assert");
const path = require("path");

const va = require(path.join(
  __dirname,
  "..",
  "src",
  "sai_web",
  "static",
  "voiceask.js"
));

// 計画書 §1-1 の例
function testSpeechText() {
  const one = {
    header: "保存",
    question: "保存しますか？",
    options: [
      { label: "保存する (Recommended)", description: "今すぐ保存" },
      { label: "やめる" },
    ],
  };
  assert.strictEqual(
    va.vaSpeechText(one, 0, 1),
    "保存。保存しますか？。1番、保存する、おすすめ。今すぐ保存。2番、やめる。番号か、選択肢の名前で答えてください。"
  );
  // 2 問以上なら「何問中か」を先に言う
  const two = { question: "色は？", options: [{ label: "青" }, { label: "赤" }] };
  assert.ok(va.vaSpeechText(two, 1, 2).startsWith("2問中2問目。"));
  // 複数選択は「いくつでも選べます」を足す
  const multi = { question: "選ぶ", options: [{ label: "A" }], multiSelect: true };
  assert.ok(va.vaSpeechText(multi, 0, 1).includes("いくつでも選べます。"));
  // 「（推奨）」の読み替え
  const rec = { question: "いい？", options: [{ label: "いい（推奨）" }] };
  assert.ok(va.vaSpeechText(rec, 0, 1).includes("1番、いい、おすすめ。"));
  // 選択肢も header も無いときは「質問1」くらいを話す
  assert.ok(va.vaSpeechText({}, 0, 1).includes("質問1。"));
}

function testNormalize() {
  assert.strictEqual(va.vaNormalize(" 保存 する "), "保存する");
  assert.strictEqual(va.vaNormalize("Ａ１"), "a1");
  assert.strictEqual(va.vaNormalize("やめる。"), "やめる");
  assert.strictEqual(va.vaNormalize("(Recommended)"), "recommended");
}

function testMatchAnswer() {
  // 計画書 §1-3 の表
  const options = ["保存する (Recommended)", "やめる", "あとで決める"];
  const hit = [
    ["1番", false, { choice_index: 0 }],
    ["にばん", false, { choice_index: 1 }],
    ["3", false, { choice_index: 2 }],
    ["やめる", false, { choice_index: 1 }],
    ["あとで決めるにします", false, { choice_index: 2 }],
    ["5番", false, { text: "5番" }],
    ["1番と3番", true, { choice_indices: [0, 2] }],
    ["理由を書いてから決めたい", false, { text: "理由を書いてから決めたい" }],
    ["  ", false, null],
  ];
  for (const [spoken, multi, want] of hit) {
    assert.deepStrictEqual(va.vaMatchAnswer(spoken, options, multi), want, `spoken=${spoken}`);
  }
  // 数え方・漢数字・かな
  const three = ["いち", "に", "さん"];
  assert.deepStrictEqual(va.vaMatchAnswer("ふたつめ", three, false), { choice_index: 1 });
  assert.deepStrictEqual(va.vaMatchAnswer("三", three, false), { choice_index: 2 });
  assert.deepStrictEqual(va.vaMatchAnswer("さん", three, false), { choice_index: 2 });
  // 複数選択: 名前の指定も足す（昇順・重複なし）
  assert.deepStrictEqual(
    va.vaMatchAnswer("3番と やめる", options, true),
    { choice_indices: [1, 2] }
  );
  // 複数選択で何も無ければ自由入力
  assert.deepStrictEqual(va.vaMatchAnswer("後で", options, true), { text: "後で" });
  // 単一選択で 2 つ一致したら決められないので自由入力
  assert.deepStrictEqual(va.vaMatchAnswer("あ", ["あれ", "これ"], false), { text: "あ" });
  // 選択肢のオブジェクト形でも同じ
  assert.deepStrictEqual(
    va.vaMatchAnswer("2番", [{ label: "A" }, { label: "B" }], false),
    { choice_index: 1 }
  );
}

function main() {
  testSpeechText();
  testNormalize();
  testMatchAnswer();
  console.log("test_voiceask: すべて通りました");
}

main();
