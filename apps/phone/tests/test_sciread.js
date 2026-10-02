"use strict";
// 読む札の画面（sciread.js）の純関数のテスト（U21）。
// 実行:
//     node apps/phone/tests/test_sciread.js
//
// DOM が要る描画・操作はブラウザで確かめる（kantoku が統合のあとに行う）。

const assert = require("assert");
const path = require("path");

const src = path.join(__dirname, "..", "src", "sai_phone", "static", "sciread.js");
const {
  srTrim,
  srParseHash,
  srCardNode,
  srCardPaper,
  srParents,
  srBuildAnswer,
  srBackHash,
  SR_DEPTHS,
  SR_OWNERS,
  SR_CITE_RE,
} = require(src);

function sampleCard(kind = "O", id = "O12") {
  return {
    run_id: 130,
    owner: "todo",
    node: { id, kind, text: "小目的" },
    purpose: "O12 のために #37 を読む",
    paper: { id: 37, title: "Scalp EEG for seizure prediction", year: 2021 },
    depth: "full",
    questions: ["データ名は何か"],
    extract: ["データ名", "N"],
    findings: [],
    note: false,
    debts: [],
  };
}

function ok(built) {
  assert.strictEqual(built.ok, true, `通るはずが弾かれました: ${built.error}`);
  assert.strictEqual(built.error, null);
  return built.json;
}

function ng(built) {
  assert.strictEqual(built.ok, false, `弾かれるはず: ${JSON.stringify(built.json)}`);
  assert.strictEqual(built.json, null);
  assert.ok(built.error && typeof built.error === "string");
  return built.error;
}

function test_parse_hash() {
  assert.deepStrictEqual(srParseHash("#/papers/read/130"), { runId: 130 });
  assert.deepStrictEqual(srParseHash("#/papers/read/999999999"), { runId: 999999999 });
  for (const h of ["#/papers/read/0", "#/papers/read/012", "#/papers/read/", "#/papers/read/abc",
    "#/papers/read/130/x", "#/papers/tree/O12", "#/papers/read/1300000000", "", null]) {
    assert.strictEqual(srParseHash(h), null, String(h));
  }
}

function test_card_parts() {
  const card = sampleCard();
  assert.strictEqual(srTrim("  あ "), "あ");
  assert.strictEqual(srCardNode(card), "O12");
  assert.strictEqual(srCardNode({}), "");
  assert.strictEqual(srCardPaper(card), 37);
  assert.strictEqual(srCardPaper({ paper: { id: 0 } }), 0);
  assert.strictEqual(srCardPaper(null), 0);
  assert.strictEqual(srBackHash(card), "#/papers/tree/O12");
  assert.strictEqual(srBackHash({}), "#/papers/tree");
}

function test_parents() {
  assert.deepStrictEqual(srParents(sampleCard("O", "O12"), null), ["O12"]);
  assert.deepStrictEqual(srParents(sampleCard("E", "E3"), null), ["E3"]);
  // Q は子の O だけ。★ を先に
  const detail = { children: [
    { id: "O1", kind: "O", starred: false },
    { id: "F2", kind: "F" },
    { id: "O5", kind: "O", starred: true },
    { id: "E9", kind: "E" },
  ] };
  assert.deepStrictEqual(srParents(sampleCard("Q", "Q12"), detail), ["O5", "O1"]);
  assert.deepStrictEqual(srParents(sampleCard("Q", "Q12"), null), []);
  assert.deepStrictEqual(srParents(sampleCard("F", "F1"), detail), []);
}

function test_build_answer() {
  const card = sampleCard();
  assert.deepStrictEqual(ok(srBuildAnswer(card, { parent: "O12", text: "改善が見られた。" })), {
    op: "add_f", parent: "O12", text: "改善が見られた。", cites: ["#37"], paper: 37, run: 130,
  });
  assert.deepStrictEqual(ok(srBuildAnswer(card, { parent: "E3", text: "あ", place: " §3.2 " })).cites, ["#37 §3.2"]);
  // 3 文まで
  ok(srBuildAnswer(card, { parent: "O12", text: "一。二。三。" }));
  assert.match(ng(srBuildAnswer(card, { parent: "O12", text: "一。二。三。四。" })), /3 文/);
  // 付ける先は O か E
  assert.match(ng(srBuildAnswer(card, { parent: "Q1", text: "あ" })), /O か E/);
  assert.match(ng(srBuildAnswer(card, { text: "あ" })), /O か E/);
  // 字数・場所・論文
  assert.match(ng(srBuildAnswer(card, { parent: "O12", text: "  " })), /文を入力/);
  assert.match(ng(srBuildAnswer(card, { parent: "O12", text: "あ".repeat(1201) })), /1200/);
  ok(srBuildAnswer(card, { parent: "O12", text: "あ".repeat(1200) }));
  assert.match(ng(srBuildAnswer(card, { parent: "O12", text: "あ", place: "あ".repeat(81) })), /80/);
  assert.match(ng(srBuildAnswer({ ...card, paper: {} }, { parent: "O12", text: "あ" })), /論文/);
}

function test_values() {
  assert.deepStrictEqual(SR_DEPTHS.map((x) => x[0]), ["abstract", "figures", "full"]);
  for (const [key, label] of Object.entries(SR_OWNERS)) assert.ok(label, key);
  assert.ok(SR_CITE_RE.test("#37"));
  assert.ok(SR_CITE_RE.test("#37.2.1-3"));
  assert.ok(SR_CITE_RE.test("#37 §3.2"));
  assert.ok(!SR_CITE_RE.test("37"));
  assert.ok(!SR_CITE_RE.test("#0"));
}

test_parse_hash();
test_card_parts();
test_parents();
test_build_answer();
test_values();
console.log("test_sciread: すべて通りました");
