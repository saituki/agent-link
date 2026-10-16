"use strict";
// 新しいチャット（static/newchat.js・U04）の DOM を使わない部分のテスト（assert だけ）。
// 実行:
//     node tests/test_newchat.js

const assert = require("assert");
const path = require("path");

const N = require(path.join(__dirname, "..", "src", "agent_link_web", "static", "newchat.js"));

// 入力の検査（プロジェクト・block・最初の文が空でない）
function testValidate() {
  assert.deepStrictEqual(
    N.ncValidate({ project: "p", block: "main", task_text: "やあ" }),
    { ok: true }
  );
  // プロジェクトが空
  assert.strictEqual(N.ncValidate({ project: "", block: "main", task_text: "x" }).ok, false);
  assert.match(N.ncValidate({ block: "main", task_text: "x" }).error, /project/i);
  // block が空
  assert.strictEqual(N.ncValidate({ project: "p", block: "", task_text: "x" }).ok, false);
  assert.match(N.ncValidate({ project: "p", task_text: "x" }).error, /block/);
  // 最初の文が空（空白だけも空とみなす）
  assert.strictEqual(N.ncValidate({ project: "p", block: "main", task_text: "" }).ok, false);
  assert.strictEqual(N.ncValidate({ project: "p", block: "main", task_text: "  \n" }).ok, false);
  assert.match(N.ncValidate({ project: "p", block: "main", task_text: "" }).error, /first message/i);
  // 空の引数でも落ちない
  assert.strictEqual(N.ncValidate(null).ok, false);
  assert.strictEqual(N.ncValidate(undefined).ok, false);
}

// model の選択肢（cc_models だけ）
function testModelChoices() {
  const options = { cc_models: ["opus", "sonnet", "haiku"] };
  assert.deepStrictEqual(N.ncModelChoices(options), ["opus", "sonnet", "haiku"]);
  // options が無くても落ちない
  assert.deepStrictEqual(N.ncModelChoices(null), []);
  assert.deepStrictEqual(N.ncModelChoices(undefined), []);
  assert.deepStrictEqual(N.ncModelChoices({}), []);
}

// 送る JSON（runner・provider・round・global_round を送らない）
function testBuildPayload() {
  const payload = N.ncBuildPayload({
    project: "p", block: "main", model: "opus", thinking: "high", task_text: "やあ",
    runner: "cc", provider: "x", round: "1", global_round: "1",
  });
  assert.deepStrictEqual(payload, { block: "main", model: "opus", thinking: "high", task_text: "やあ" });
  for (const k of ["runner", "provider", "round", "global_round", "tag"]) {
    assert.strictEqual(Object.prototype.hasOwnProperty.call(payload, k), false, k);
  }
  // 空の項目は空文字
  assert.deepStrictEqual(N.ncBuildPayload({}), { block: "", model: "", thinking: "", task_text: "" });
  assert.deepStrictEqual(N.ncBuildPayload(null), { block: "", model: "", thinking: "", task_text: "" });
}

// 住所 #/newchat?project=&block=（G3-C3）。形の合わない値は ""
function testParsePreset() {
  assert.deepStrictEqual(
    N.ncParsePreset("#/newchat?project=saituki-west&block=ts-engineer"),
    { project: "saituki-west", block: "ts-engineer" }
  );
  assert.deepStrictEqual(N.ncParsePreset("#/newchat?project=&block=main"), { project: "", block: "main" });
  assert.deepStrictEqual(N.ncParsePreset("#/newchat"), { project: "", block: "" });
  // 形の合わない値（経路・空白・65 字以上）は空にする
  assert.deepStrictEqual(N.ncParsePreset("#/newchat?project=../x&block=a b"), { project: "", block: "" });
  assert.deepStrictEqual(N.ncParsePreset(`#/newchat?project=${"a".repeat(65)}&block=main`), { project: "", block: "main" });
  assert.deepStrictEqual(N.ncParsePreset(`#/newchat?project=${"a".repeat(64)}`), { project: "a".repeat(64), block: "" });
  // 他の住所は null
  assert.strictEqual(N.ncParsePreset("#/"), null);
  assert.strictEqual(N.ncParsePreset("#/projects"), null);
  assert.strictEqual(N.ncParsePreset("#/newchatx?project=p"), null);
  assert.strictEqual(N.ncParsePreset(""), null);
  assert.strictEqual(N.ncParsePreset(undefined), null);
}

function main() {
  testValidate();
  testModelChoices();
  testBuildPayload();
  testParsePreset();
  console.log("test_newchat: すべて通りました");
}

main();
