"use strict";
// 拡張の画面の枠（static/ext.js・U07）の DOM を使わない部分のテスト（assert だけ）。
// 実行:
//     node tests/test_ext.js

const assert = require("assert");
const path = require("path");

const E = require(path.join(__dirname, "..", "src", "agent_link_web", "static", "ext.js"));

// --- extParseHash：住所から名前・プロジェクト・後ろの道を取り出す
assert.deepStrictEqual(E.extParseHash("#/ext/hello", "hello"), { ext: "hello", project: null, path: "" }, "名前だけ");
assert.deepStrictEqual(E.extParseHash("#/ext/hello/demo", "hello"), { ext: "hello", project: "demo", path: "" }, "プロジェクト");
assert.deepStrictEqual(E.extParseHash("#/ext/hello/demo/a/b", "hello"), { ext: "hello", project: "demo", path: "a/b" }, "後ろの道");
assert.deepStrictEqual(
  E.extParseHash("#/ext/hello/my%20project/x", "hello"),
  { ext: "hello", project: "my project", path: "x" },
  "%20 を含むプロジェクト",
);
assert.strictEqual(E.extParseHash("#/ext/other/demo", "hello"), null, "違う名前");
assert.strictEqual(E.extParseHash("#/ext/hellox/demo", "hello"), null, "名前の続き（hellox）は別の画面");
assert.strictEqual(E.extParseHash("#/extra", "hello"), null, "#/extra は拡張の住所ではない");
assert.strictEqual(E.extParseHash("#/ext/hello/", "hello"), null, "プロジェクトが空");
assert.strictEqual(E.extParseHash("#/ext/hello/%E0%A4%A", "hello"), null, "壊れた %xx は合わない");
assert.strictEqual(E.extParseHash("#/home", "hello"), null, "別の画面");

// --- extPickProject：最後に選んだ物 → 先頭 → null
assert.strictEqual(E.extPickProject(["a", "b"], "b"), "b", "最後に選んだ物が使えればそれ");
assert.strictEqual(E.extPickProject(["a", "b"], "z"), "a", "最後の物が無ければ先頭");
assert.strictEqual(E.extPickProject(["a", "b"], null), "a", "最後の物が無ければ先頭（null）");
assert.strictEqual(E.extPickProject([], "a"), null, "1 つも無ければ null");

// --- ctx.cli：input があるときだけ本文に入れる
const sent = [];
global.saiApi = (url, opts) => {
  sent.push({ url, opts });
  return Promise.resolve({});
};
const ctx = E.extCtx("hello", { project: "p q" });
ctx.cli(["greet", "x"]);
ctx.cli(["save", "A1"], { input: "# 見出し\n本文" });
assert.strictEqual(sent[0].url, "/api/projects/p%20q/ext/hello/cli", "住所");
assert.deepStrictEqual(sent[0].opts, { method: "POST", body: { args: ["greet", "x"] } }, "input 無しは args だけ");
assert.deepStrictEqual(
  sent[1].opts.body,
  { args: ["save", "A1"], input: "# 見出し\n本文" },
  "input 付きは本文に入る",
);

console.log("test_ext: すべて通りました");
