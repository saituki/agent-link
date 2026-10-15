"use strict";
// 回答済みカードで「どの選択肢が選ばれたか」を決める純関数 `answeredPicks` のテスト。
//
// app.js はブラウザ専用なので require できない。関数定義だけ取り出して評価する
// （取り出せなければテストが失敗する）。
// 実行:
//     node tests/test_answered_card.js

const assert = require("assert");
const fs = require("fs");
const path = require("path");

const src = fs.readFileSync(
  path.join(__dirname, "..", "src", "agent_link_web", "static", "app.js"),
  "utf8"
);

const m = src.match(/^function answeredPicks\(answer\) \{[\s\S]*?^\}/m);
assert.ok(m, "app.js に answeredPicks が見つかりません");
// eslint-disable-next-line no-new-func
const answeredPicks = new Function(`${m[0]}; return answeredPicks;`)();

// 単一選択
assert.deepStrictEqual(answeredPicks({ choice_index: 1 }), { picked: [1], text: "" });
assert.deepStrictEqual(answeredPicks({ choice_index: 0 }), { picked: [0], text: "" });
// 複数選択
assert.deepStrictEqual(answeredPicks({ choice_indices: [0, 2] }), { picked: [0, 2], text: "" });
// 自由入力（前後の空白は落とす）
assert.deepStrictEqual(answeredPicks({ text: "  その他の案 " }), { picked: [], text: "その他の案" });
// 空
assert.deepStrictEqual(answeredPicks(null), { picked: [], text: "" });
assert.deepStrictEqual(answeredPicks({}), { picked: [], text: "" });
assert.deepStrictEqual(answeredPicks({ text: "   " }), { picked: [], text: "" });

console.log("test_answered_card: すべて通りました");
