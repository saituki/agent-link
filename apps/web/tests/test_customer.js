"use strict";
// サポート（static/customer.js・U03）の DOM を使わない部分のテスト（assert だけ）。
// 実行:
//     node tests/test_customer.js

const assert = require("assert");
const path = require("path");

const C = require(path.join(__dirname, "..", "src", "sai_web", "static", "customer.js"));

// 住所からプロジェクト名を取る（一覧にある名前だけ）
function testProjectFromHash() {
  const names = ["west", "sai-tree"];
  assert.strictEqual(C.csProjectFromHash("#/run/west/42", names), "west");
  assert.strictEqual(C.csProjectFromHash("#/list/west?view=dash", names), "west");
  assert.strictEqual(C.csProjectFromHash("#/run/sai-tree/7/stats", names), "sai-tree");
  // 日本語などの名前は住所では %エンコードされている
  assert.strictEqual(C.csProjectFromHash("#/run/%E8%A5%BF/1", ["西"]), "西");
  // 住所に名前が無い・一覧に無い名前は ""
  assert.strictEqual(C.csProjectFromHash("#/", names), "");
  assert.strictEqual(C.csProjectFromHash("#/customer", names), "");
  assert.strictEqual(C.csProjectFromHash("#/run/other/3", names), "");
  assert.strictEqual(C.csProjectFromHash("", names), "");
  // 壊れた %エンコードでも落ちない
  assert.strictEqual(C.csProjectFromHash("#/run/%E0%A4%A/1", names), "");
}

testProjectFromHash();
assert.strictEqual(C.CS_RUN_KEY, "sai_web_newchat_run", "起動待ちの印の名前が app.js と違います");
console.log("test_customer: すべて通りました");
