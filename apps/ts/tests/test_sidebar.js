"use strict";
// sidebar のタブの並び（tsTabOrder）のテスト。
// 実行:
//     node apps/ts/tests/test_sidebar.js
//
// タブの表示・選択は DOM が要るのでブラウザで確かめる（kantoku が統合のあとに行う）。

const assert = require("assert");
const path = require("path");

const src = path.join(__dirname, "..", "src", "sai_ts", "static", "sidebar.js");
const { tsTabOrder, tsSidebarBuiltins } = require(src);

const ids = (tabs) => tabs.map((t) => t.id);
const tab = (id) => ({ id, title: id });

// 既定の順は patients・chat・events・ext。足す順が逆でも既定の順になる
assert.deepStrictEqual(ids(tsTabOrder([tab("ext"), tab("events"), tab("chat")], tsSidebarBuiltins)), [
  "chat",
  "events",
  "ext",
]);
assert.deepStrictEqual(ids(tsTabOrder([tab("ext"), tab("chat"), tab("patients")], tsSidebarBuiltins)), [
  "patients",
  "chat",
  "ext",
], "患者は先頭");

// 拡張が足したタブは後ろへ、足した順のまま
assert.deepStrictEqual(
  ids(tsTabOrder([tab("hello"), tab("events"), tab("zeta"), tab("chat"), tab("alpha")], tsSidebarBuiltins)),
  ["chat", "events", "hello", "zeta", "alpha"],
);

// 組み込みが一部だけ（まだ足されていない）でも順は崩れない
assert.deepStrictEqual(ids(tsTabOrder([tab("ext"), tab("x")], tsSidebarBuiltins)), ["ext", "x"]);

// 空
assert.deepStrictEqual(tsTabOrder([], tsSidebarBuiltins), []);

// 入力の配列を書き換えない
const input = [tab("ext"), tab("chat")];
tsTabOrder(input, tsSidebarBuiltins);
assert.deepStrictEqual(ids(input), ["ext", "chat"]);

if (__filename === process.argv[1]) console.log("test_sidebar: ok");
