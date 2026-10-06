"use strict";
// 会話の検索の画面（U10）の DOM を使わない関数の小さなテスト（assert だけ）。
// 確かめること:
// - 住所の読み取り（#/project/<name>/sessions）
// - 条件 → クエリ文字列（空は入れない・状態と種類は許可した物だけ・条件なしは空）
// - 結果のまとめ（block_id ごと）
// 実行:
//     node apps/web/tests/test_sessionsearch.js

const assert = require("assert");
const path = require("path");

const sess = require(path.join(
  __dirname,
  "..",
  "src",
  "sai_web",
  "static",
  "sessionsearch.js"
));

function testParseHash() {
  assert.deepStrictEqual(sess.sessParseHash("#/project/sai-moon/sessions"), { name: "sai-moon" });
  assert.deepStrictEqual(sess.sessParseHash("#/project/a%20b/sessions/"), { name: "a b" });
  assert.strictEqual(sess.sessParseHash("#/project/sai-moon/commands"), null);
}

function testBuildQuery() {
  // 条件が 1 つも無ければ空
  assert.strictEqual(sess.sessBuildQuery({}), "");
  assert.strictEqual(sess.sessBuildQuery({ word: "  " }), "");

  const q = sess.sessBuildQuery({
    word: " a,b ",
    global_round: "2-4",
    round: "1",
    id: "10-20",
    block: "worker",
    date: "2026/09/01-2026/09/30",
    status: ["running", "todo"],
    type: ["agent", "user"],
  });
  const params = new URLSearchParams(q);
  assert.strictEqual(params.get("word"), "a,b");
  assert.strictEqual(params.get("global_round"), "2-4");
  assert.strictEqual(params.get("round"), "1");
  assert.strictEqual(params.get("id"), "10-20");
  assert.strictEqual(params.get("block"), "worker");
  assert.strictEqual(params.get("date"), "2026/09/01-2026/09/30");
  assert.strictEqual(params.get("status"), "running,todo");
  assert.strictEqual(params.get("type"), "agent,user");

  // 知らない状態・種類は入れない
  const bad = new URLSearchParams(
    sess.sessBuildQuery({ status: ["bogus", "running"], type: ["nope"] })
  );
  assert.strictEqual(bad.get("status"), "running");
  assert.strictEqual(bad.get("type"), null);
}

function testGroup() {
  const groups = sess.sessGroup([
    { block_id: 5, type: "user" },
    { block_id: 5, type: "agent" },
    { block_id: 6, type: "tool" },
  ]);
  assert.deepStrictEqual(groups.map((g) => g.blockId), ["5", "6"]);
  assert.strictEqual(groups[0].items.length, 2);
  assert.deepStrictEqual(sess.sessGroup([]), []);
  assert.deepStrictEqual(sess.sessGroup(null), []);
}

function main() {
  testParseHash();
  testBuildQuery();
  testGroup();
  console.log("test_sessionsearch: すべて通りました");
}

main();
