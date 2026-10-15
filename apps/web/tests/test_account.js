"use strict";
// アカウント画面（static/account.js）の登録のテスト。
// 登録する route は #/account だけ（ログイン・パスワード・管理の画面は無い）。
// 実行:
//     node tests/test_account.js

const assert = require("assert");
const path = require("path");

global.window = {};
require(path.join(__dirname, "..", "src", "agent_link_web", "static", "account.js"));
const routes = global.window.SaiRoutes;

assert.strictEqual(routes.length, 1, "route は 1 つだけ");
const r = routes[0];
assert.deepStrictEqual(r.match("#/account"), {});
for (const h of ["#/login", "#/password", "#/admin", "#/account/x", "#/"]) {
  assert.strictEqual(r.match(h), null, `${h} は開かない`);
}
assert.strictEqual(r.view, "view-account");
assert.strictEqual(r.tab, "account");
assert.strictEqual(r.hideTabbar, false);

console.log("test_account: すべて通りました");
