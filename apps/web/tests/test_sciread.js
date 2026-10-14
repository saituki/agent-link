"use strict";
// READ_CARD の画面（sciread.js）の純関数のテスト（U21・U06 で A と stance に合わせた）。
// 実行:
//     node apps/web/tests/test_sciread.js
//
// DOM が要る描画・操作はブラウザで確かめる（kantoku が統合のあとに行う）。

const assert = require("assert");
const path = require("path");

const src = path.join(__dirname, "..", "src", "sai_web", "static", "sciread.js");
// 画面の軸の判定（ui.js の saiIsAxis）。ブラウザでは一番外側の名前として読まれるので、テストでも同じ名前にする
global.saiIsAxis = require(require("path").join(__dirname, "..", "src", "sai_web", "static", "ui.js")).saiIsAxis;
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
  SR_STANCES,
} = require(src);

function sampleCard(kind = "A", id = "A12") {
  return {
    run_id: 130,
    owner: "todo",
    node: { id, kind, text: "小目的", parent: "A90" },
    purpose: "A12 のために #37 を読む",
    paper: { id: 37, title: "Scalp EEG for seizure prediction", year: 2021 },
    depth: "full",
    questions: ["データ名は何か"],
    extract: ["データ名", "N"],
    findings: [],
    note: false,
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
  assert.deepStrictEqual(srParseHash("#/research/read/130"), { runId: 130 });
  assert.deepStrictEqual(srParseHash("#/research/read/999999999"), { runId: 999999999 });
  for (const h of ["#/research/read/0", "#/research/read/012", "#/research/read/", "#/research/read/abc",
    "#/research/read/130/x", "#/research/tree/A12", "#/research/read/1300000000", "#/papers/read/130", "", null]) {
    assert.strictEqual(srParseHash(h), null, String(h));
  }
}

function test_card_parts() {
  const card = sampleCard();
  assert.strictEqual(srTrim("  あ "), "あ");
  assert.strictEqual(srCardNode(card), "A12");
  assert.strictEqual(srCardNode({}), "");
  assert.strictEqual(srCardPaper(card), 37);
  assert.strictEqual(srCardPaper({ paper: { id: 0 } }), 0);
  assert.strictEqual(srCardPaper(null), 0);
  assert.strictEqual(srBackHash(card), "#/research/tree/A12");
  assert.strictEqual(srBackHash({}), "#/research/tree");
}

function test_parents() {
  assert.deepStrictEqual(srParents(sampleCard("A", "A12"), null), ["A12"]);
  // F は A だけに付く（E の下に F は置かない）
  assert.deepStrictEqual(srParents(sampleCard("E", "E3"), null), []);
  // 軸の A（親の無い A）は子の A だけ。★ を先に
  const detail = { children: [
    { id: "A1", kind: "A", starred: false },
    { id: "F2", kind: "F" },
    { id: "A5", kind: "A", starred: true },
    { id: "E9", kind: "E" },
  ] };
  const axis = sampleCard("A", "A90");
  axis.node.parent = null;
  assert.deepStrictEqual(srParents(axis, detail), ["A5", "A1"]);
  assert.deepStrictEqual(srParents(axis, null), []);
  assert.deepStrictEqual(srParents(sampleCard("F", "F1"), detail), []);
}

function test_build_answer() {
  const card = sampleCard();
  assert.deepStrictEqual(ok(srBuildAnswer(card, { parent: "A12", text: "改善が見られた。", stance: "support" })), {
    op: "add_f", parent: "A12", text: "改善が見られた。", stance: "support", cites: ["#37"], paper: 37, run: 130,
  });
  assert.deepStrictEqual(ok(srBuildAnswer(card, { parent: "A3", text: "あ", place: " §3.2 ", stance: "deny" })).cites, ["#37 §3.2"]);
  // 3 文まで
  ok(srBuildAnswer(card, { parent: "A12", text: "一。二。三。", stance: "issue" }));
  assert.match(ng(srBuildAnswer(card, { parent: "A12", text: "一。二。三。四。", stance: "support" })), /3 文/);
  // 付ける先は A か E
  assert.match(ng(srBuildAnswer(card, { parent: "Q1", text: "あ", stance: "support" })), /付ける先/);
  // E の下に F は置かない
  assert.match(ng(srBuildAnswer(card, { parent: "E3", text: "あ", stance: "support" })), /付ける先/);
  assert.match(ng(srBuildAnswer(card, { text: "あ", stance: "support" })), /付ける先/);
  // stance は必須（既定は無し）
  assert.match(ng(srBuildAnswer(card, { parent: "A12", text: "あ" })), /stance/);
  assert.match(ng(srBuildAnswer(card, { parent: "A12", text: "あ", stance: "X" })), /stance/);
  // 字数・場所・論文
  assert.match(ng(srBuildAnswer(card, { parent: "A12", text: "  ", stance: "support" })), /文を入力/);
  assert.match(ng(srBuildAnswer(card, { parent: "A12", text: "あ".repeat(1201), stance: "support" })), /1200/);
  ok(srBuildAnswer(card, { parent: "A12", text: "あ".repeat(1200), stance: "support" }));
  assert.match(ng(srBuildAnswer(card, { parent: "A12", text: "あ", place: "あ".repeat(81), stance: "support" })), /80/);
  assert.match(ng(srBuildAnswer({ ...card, paper: {} }, { parent: "A12", text: "あ", stance: "support" })), /論文/);
}

function test_values() {
  assert.deepStrictEqual(SR_DEPTHS.map((x) => x[0]), ["abstract", "figures", "full"]);
  assert.deepStrictEqual(SR_STANCES.map((x) => x[0]), ["support", "deny", "issue"]);
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
