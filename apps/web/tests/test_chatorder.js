"use strict";
// 会話に行を・回答済みカードを置く位置を決める関数（chatorder.js）の小さなテスト（assert だけ）・CP3。
// 実行:
//     node tests/test_chatorder.js

const assert = require("assert");
const path = require("path");

const { timeMs, firstIndexAfter, chatPartialPlan } = require(path.join(
  __dirname,
  "..",
  "src",
  "agent_link_web",
  "static",
  "chatorder.js"
));

function testTimeMs() {
  // UNIX 秒（質問・承認カードの created_at）→ ミリ秒にそろえる
  assert.strictEqual(timeMs(0), 0);
  assert.strictEqual(timeMs(1758600000), 1758600000 * 1000);
  // ISO 文字列（会話の行の time）
  assert.strictEqual(
    timeMs("2026-09-23T01:00:00.000Z"),
    Date.parse("2026-09-23T01:00:00.000Z")
  );
  // 読めないものは null（挿す位置を決められないため）
  assert.strictEqual(timeMs(null), null);
  assert.strictEqual(timeMs(undefined), null);
  assert.strictEqual(timeMs(""), null);
  assert.strictEqual(timeMs("  "), null);
  assert.strictEqual(timeMs("きょう"), null);
  assert.strictEqual(timeMs(NaN), null);
  assert.strictEqual(timeMs(Infinity), null);
}

function testFirstIndexAfter() {
  const times = [
    "2026-09-23T01:00:00.000Z",
    "2026-09-23T02:00:00.000Z",
    "2026-09-23T03:00:00.000Z",
  ];
  const sec = (iso) => Date.parse(iso) / 1000;

  // 質問された時刻（UNIX 秒）より後の最初の行の前へ（CP3 の核心）
  assert.strictEqual(firstIndexAfter(times, sec("2026-09-23T01:30:00Z")), 1);
  // どの行よりも前 → 先頭
  assert.strictEqual(firstIndexAfter(times, sec("2026-09-23T00:00:00Z")), 0);
  // 最後の行より後 → 末尾（該当する行が無いケース）
  assert.strictEqual(firstIndexAfter(times, sec("2026-09-23T04:00:00Z")), 3);
  // 同じ時刻の行は「より後」ではない（その行の後ろになる）
  assert.strictEqual(firstIndexAfter(times, sec("2026-09-23T02:00:00Z")), 2);
  // 行が1つも無ければ 0（末尾）
  assert.strictEqual(firstIndexAfter([], sec("2026-09-23T01:30:00Z")), 0);
  // 時刻が読めない基準 → 末尾（今の並びを崩さない）
  assert.strictEqual(firstIndexAfter(times, null), 3);
  assert.strictEqual(firstIndexAfter(times, undefined), 3);
  assert.strictEqual(firstIndexAfter(times, "abc"), 3);

  // 会話に行を足すときも同じ関数を使う（ISO 文字列の基準でも動く）
  assert.strictEqual(firstIndexAfter(times, "2026-09-23T01:30:00.000Z"), 1);

  // 時刻の読めない行は数えず，先の行を見る
  const mixed = [null, "", "2026-09-23T05:00:00.000Z"];
  assert.strictEqual(firstIndexAfter(mixed, sec("2026-09-23T04:00:00Z")), 2);
}

function testChatPartialPlan() {
  // 空 → 空: 何もしない
  assert.strictEqual(chatPartialPlan("", ""), "none");
  assert.strictEqual(chatPartialPlan(null, null), "none");
  assert.strictEqual(chatPartialPlan(undefined, ""), "none");
  // 空 → 文: 吹き出しを出す
  assert.strictEqual(chatPartialPlan("", "考え中"), "show");
  assert.strictEqual(chatPartialPlan(null, "考え中"), "show");
  // 文 → 同じ文: 描き直さない
  assert.strictEqual(chatPartialPlan("考え中", "考え中"), "none");
  // 文 → 別の文: 中身を置き換える
  assert.strictEqual(chatPartialPlan("考え", "考え中です"), "update");
  // 文 → 空: 吹き出しを消す
  assert.strictEqual(chatPartialPlan("考え中", ""), "hide");
  assert.strictEqual(chatPartialPlan("考え中", null), "hide");
  // 空白だけの文は空とみなす（見えない吹き出しを出さない）
  assert.strictEqual(chatPartialPlan("", "   \n"), "none");
  assert.strictEqual(chatPartialPlan("考え中", "  "), "hide");
  assert.strictEqual(chatPartialPlan("  ", "考え中"), "show");
}

function main() {
  testTimeMs();
  testFirstIndexAfter();
  testChatPartialPlan();
  console.log("test_chatorder: すべて通りました");
}

main();
