"use strict";
// 新しいチャット（static/newchat.js・U04）の DOM を使わない部分のテスト（assert だけ）。
// 実行:
//     node tests/test_newchat.js

const assert = require("assert");
const path = require("path");

const N = require(path.join(__dirname, "..", "src", "sai_web", "static", "newchat.js"));

// 入力の検査（プロジェクト・block・最初の文が空でない）
function testValidate() {
  assert.deepStrictEqual(
    N.ncValidate({ project: "p", block: "main", task_text: "やあ" }),
    { ok: true }
  );
  // プロジェクトが空
  assert.strictEqual(N.ncValidate({ project: "", block: "main", task_text: "x" }).ok, false);
  assert.match(N.ncValidate({ block: "main", task_text: "x" }).error, /プロジェクト/);
  // block が空
  assert.strictEqual(N.ncValidate({ project: "p", block: "", task_text: "x" }).ok, false);
  assert.match(N.ncValidate({ project: "p", task_text: "x" }).error, /block/);
  // 最初の文が空（空白だけも空とみなす）
  assert.strictEqual(N.ncValidate({ project: "p", block: "main", task_text: "" }).ok, false);
  assert.strictEqual(N.ncValidate({ project: "p", block: "main", task_text: "  \n" }).ok, false);
  assert.match(N.ncValidate({ project: "p", block: "main", task_text: "" }).error, /最初の文/);
  // 空の引数でも落ちない
  assert.strictEqual(N.ncValidate(null).ok, false);
  assert.strictEqual(N.ncValidate(undefined).ok, false);
}

// runner に合わせた model の選択肢
function testModelChoices() {
  const options = {
    cc_models: ["opus", "sonnet", "haiku"],
    providers: { "openrouter-x": ["a/b", "c/d"], local: ["c/d", "e"] },
  };
  assert.deepStrictEqual(N.ncModelChoices(options, "cc"), ["opus", "sonnet", "haiku"]);
  // runner が空（スキップ）なら cc 扱い（pi は廃止）
  assert.deepStrictEqual(N.ncModelChoices(options, ""), ["opus", "sonnet", "haiku"]);
  assert.deepStrictEqual(N.ncModelChoices(options, "pi"), []);
  // user / gl はモデル無し
  assert.deepStrictEqual(N.ncModelChoices(options, "user"), []);
  assert.deepStrictEqual(N.ncModelChoices(options, "gl"), []);
  // options が無くても落ちない
  assert.deepStrictEqual(N.ncModelChoices(null, "cc"), []);
  assert.deepStrictEqual(N.ncModelChoices(undefined, ""), []);
}

function main() {
  testValidate();
  testModelChoices();
  console.log("test_newchat: すべて通りました");
}

main();
