"use strict";
// 反映待ち（実行中に送った文が記録に現れるまで残す印）の純関数 `takeReflected` のテスト。
// 加えて ⏸ 中断は live が working でなくても会話が続いていれば出すことを，ソースで確かめる。
//
// app.js はブラウザ専用なので require できない。関数定義だけ取り出して評価する
// （取り出せなければテストが失敗する）。
// 実行:
//     node tests/test_pending_reflected.js

const assert = require("assert");
const fs = require("fs");
const path = require("path");

const src = fs.readFileSync(
  path.join(__dirname, "..", "src", "sai_web", "static", "app.js"),
  "utf8"
);

const ttl = src.match(/^const REFLECT_TTL_MS = .*;$/m);
assert.ok(ttl, "app.js に REFLECT_TTL_MS が見つかりません");
const fn = src.match(/^function takeReflected\(list, key, items, now\) \{[\s\S]*?^\}/m);
assert.ok(fn, "app.js に takeReflected が見つかりません");
// eslint-disable-next-line no-new-func
const takeReflected = new Function(`${ttl[0]}; ${fn[0]}; return takeReflected;`)();

const NOW = Date.parse("2026-10-11T10:00:00Z");
const MIN = 60 * 1000;
const KEY = "proj/12";

// 送った文は残る（記録にまだ無い）
let list = [{ key: KEY, text: "止めて", at: NOW - MIN }];
assert.deepStrictEqual(takeReflected(list, KEY, [], NOW), list);

// 記録に同じ文（user）が現れたら 1 件消す（前後の空白は無視）
const userItem = { id: "q-1", kind: "user", text: " 止めて\n", time: "" };
assert.deepStrictEqual(takeReflected(list, KEY, [userItem], NOW), []);

// agent・event の項目では消さない
const agentItem = { id: "a-1", kind: "agent", text: "止めて" };
assert.strictEqual(takeReflected(list, KEY, [agentItem], NOW).length, 1);

// 同じ文が 2 件あれば，記録に 1 件出るたびに 1 件ずつ消す
list = [
  { key: KEY, text: "はい", at: NOW - 2 * MIN },
  { key: KEY, text: "はい", at: NOW - MIN },
];
assert.strictEqual(takeReflected(list, KEY, [{ kind: "user", text: "はい" }], NOW).length, 1);

// 別の実行の文は消さない
list = [{ key: "other/3", text: "止めて", at: NOW - MIN }];
assert.strictEqual(takeReflected(list, KEY, [userItem], NOW).length, 1);

// 10 分を過ぎたものは消す（cc が取り込まずに終わったとき残り続けないため）
list = [
  { key: KEY, text: "古い", at: NOW - 11 * MIN },
  { key: KEY, text: "新しい", at: NOW - 9 * MIN },
];
assert.deepStrictEqual(takeReflected(list, KEY, [], NOW).map((p) => p.text), ["新しい"]);

// ⏸ 中断は live が null（会話が終わった）のときだけ隠す。working 以外でも出す
const interrupt = src.match(/hidden = data\.live == null;/);
assert.ok(interrupt, "⏸ の表示条件が data.live == null になっていません");
assert.ok(!/hidden = data\.live !== "working";/.test(src), "旧い ⏸ の条件が残っています");

console.log("test_pending_reflected: すべて通りました");
