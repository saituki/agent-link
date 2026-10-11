"use strict";
// イベント定義の欄の純関数（検査 tsEvdefCheck・表示の文 tsEvdefLabel）のテスト。
// 実行:
//     node apps/ts/tests/test_evdefs.js
//
// 欄の描画と保存は DOM と API が要るのでブラウザで確かめる（kantoku が統合のあとに行う）。

const assert = require("assert");
const path = require("path");

// tsEvdefCheck は名前の検査に app.js の tsValidName を使う（ブラウザでは同じ窓の関数）
global.tsValidName = require(path.join(__dirname, "..", "src", "sai_ts", "static", "app.js")).tsValidName;
const src = path.join(__dirname, "..", "src", "sai_ts", "static", "evdefs.js");
const { tsEvdefCheck, tsEvdefLabel } = require(src);

const ok = { name: "HFO", color: "#f59e0b", type: "area", ch: "single" };

// 正しい定義は空の文
assert.strictEqual(tsEvdefCheck(ok, []), "");
assert.strictEqual(tsEvdefCheck({ name: "spike", color: "#AbCdEf" }, []), "", "type・ch は省ける（自由）");

// 名前: 形（tsValidName）と重なり
assert.notStrictEqual(tsEvdefCheck({ ...ok, name: "" }, []), "");
assert.notStrictEqual(tsEvdefCheck({ ...ok, name: "../x" }, []), "");
assert.notStrictEqual(tsEvdefCheck({ ...ok, name: "a b" }, []), "");
assert.notStrictEqual(tsEvdefCheck({ ...ok, name: "x".repeat(65) }, []), "");
assert.notStrictEqual(tsEvdefCheck(ok, [{ name: "HFO" }]), "", "同じ名前は不可");
assert.strictEqual(tsEvdefCheck(ok, [{ name: "hfo" }]), "", "大文字と小文字は別の名前");

// 色は #RRGGBB
assert.notStrictEqual(tsEvdefCheck({ ...ok, color: "red" }, []), "");
assert.notStrictEqual(tsEvdefCheck({ ...ok, color: "#fff" }, []), "");
assert.notStrictEqual(tsEvdefCheck({ ...ok, color: "" }, []), "");

// 種類・ch は決まった値だけ
assert.notStrictEqual(tsEvdefCheck({ ...ok, type: "span" }, []), "");
assert.notStrictEqual(tsEvdefCheck({ ...ok, ch: "two" }, []), "");

// 表示の文
assert.strictEqual(tsEvdefLabel({ type: "area", ch: "single" }), "エリア・1ch");
assert.strictEqual(tsEvdefLabel({ type: "line", ch: "all" }), "ポイント・全ch");
assert.strictEqual(tsEvdefLabel({ type: "area" }), "エリア");
assert.strictEqual(tsEvdefLabel({}), "自由");

console.log("test_evdefs.js: OK");
