"use strict";
// 拡張の読み込み（tsExtPlan・tsExtUrl）の純関数のテスト。
// 実行:
//     node apps/ts/tests/test_exthost.js
//
// import・登録の外しは画面が要るのでブラウザで確かめる（拡張の読み直しの試験。kantoku が統合のあとに行う）。

const assert = require("assert");
const path = require("path");

const src = path.join(__dirname, "..", "src", "sai_ts", "static", "exthost.js");
const { tsExtPlan, tsExtUrl } = require(src);

const info = (name, version, extra = {}) => ({
  name,
  title: name,
  entry: "main.js",
  enabled: true,
  version,
  error: null,
  ...extra,
});
const names = (xs) => xs.map((x) => x.name);

// 何も読み込んでいない: 全部読む
let p = tsExtPlan({}, [info("hello", "aaa"), info("bye", "bbb")]);
assert.deepStrictEqual(names(p.load), ["hello", "bye"]);
assert.deepStrictEqual(p.keep, []);
assert.deepStrictEqual(p.unload, []);

// 版が同じなら、そのまま（読み直さない）
p = tsExtPlan({ hello: "aaa" }, [info("hello", "aaa")]);
assert.deepStrictEqual(p.load, []);
assert.deepStrictEqual(p.keep, ["hello"]);

// 版が変わったら読み直す（そのまま keep にはしない）
p = tsExtPlan({ hello: "aaa" }, [info("hello", "ccc")]);
assert.deepStrictEqual(names(p.load), ["hello"]);
assert.deepStrictEqual(p.keep, []);
assert.deepStrictEqual(p.unload, []);

// 一覧から消えた、または無効になった拡張は外す
p = tsExtPlan({ hello: "aaa", gone: "zzz", off: "yyy" }, [info("hello", "aaa"), info("off", "yyy", { enabled: false })]);
assert.deepStrictEqual(p.load, []);
assert.deepStrictEqual(p.keep, ["hello"]);
assert.deepStrictEqual(p.unload.sort(), ["gone", "off"]);

// 誤りのある拡張（manifest が読めない）は読まず、読み込み済みなら外す
p = tsExtPlan({ bad: "aaa" }, [info("bad", "aaa", { error: "manifest.json が壊れている" })]);
assert.deepStrictEqual(p.load, []);
assert.deepStrictEqual(p.unload, ["bad"]);

// 版が null（フォルダが無い）は読まない
p = tsExtPlan({}, [info("gone", null)]);
assert.deepStrictEqual(p.load, []);

// 強制の読み直し: 版が同じでも全部読み直す。消えた物は外す
p = tsExtPlan({ hello: "aaa", gone: "zzz" }, [info("hello", "aaa")], true);
assert.deepStrictEqual(names(p.load), ["hello"]);
assert.deepStrictEqual(p.keep, []);
assert.deepStrictEqual(p.unload, ["gone"]);

// 入力を変えない
const cur = { hello: "aaa" };
tsExtPlan(cur, [info("hello", "bbb")]);
assert.deepStrictEqual(cur, { hello: "aaa" });

// URL: 版と token を付ける。token が空なら付けない。entry の / は残す
assert.strictEqual(tsExtUrl(info("hello", "aaa"), ""), "/ext/hello/main.js?v=aaa");
assert.strictEqual(tsExtUrl(info("hello", "aaa"), "t k"), "/ext/hello/main.js?v=aaa&token=t%20k");
assert.strictEqual(
  tsExtUrl(info("hello", "aaa", { entry: "js/main.js" }), ""),
  "/ext/hello/js/main.js?v=aaa",
);

if (__filename === process.argv[1]) console.log("test_exthost: ok");
