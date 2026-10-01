"use strict";
// 読む札の画面（sciread.js）の純関数のテスト（実装計画 U20「変更内容 4」）。
// 実行:
//     node apps/phone/tests/test_sciread.js
//
// ここに無いもの（DOM が要る描画・小窓・読む札の操作）はブラウザで確かめる
// （計画 U20 の AC4 と C-17 の手順）。

const assert = require("assert");
const path = require("path");

const src = path.join(__dirname, "..", "src", "sai_phone", "static", "sciread.js");
const {
  srTrim,
  srParseHash,
  srCardNode,
  srCardPaper,
  srBuildAnswer,
  srFinishBody,
  srCite,
  srBackHash,
  SR_DEPTHS,
  SR_ROLES,
  SR_POLARITY,
  SR_OWNERS,
} = require(src);

/** C-08 の read の見本。 */
function sampleCard() {
  return {
    run_id: 130,
    owner: "todo",
    node: { id: "N12", title: "小目的" },
    purpose: "小目的の候補を比べる",
    paper: { id: 37, title: "Scalp EEG for seizure prediction", year: "2021" },
    role: "exemplar",
    depth: "full",
    questions: ["データ名は何か", "N はいくつか"],
    findings: [],
  };
}

function ok(built) {
  assert.strictEqual(built.ok, true, `通るはずが弾かれました: ${built.error}`);
  assert.strictEqual(built.error, null);
  return built.json;
}

function ng(built) {
  assert.strictEqual(built.ok, false, `弾かれるはず，通りました: ${JSON.stringify(built.json)}`);
  assert.strictEqual(built.json, null);
  assert.ok(built.error && typeof built.error === "string", "理由が日本語で返りません");
  return built.error;
}

// --- 住所 ---
function test_parse_hash() {
  assert.deepStrictEqual(srParseHash("#/papers/read/130"), { runId: 130 });
  assert.deepStrictEqual(srParseHash("#/papers/read/1"), { runId: 1 });
  assert.deepStrictEqual(srParseHash("#/papers/read/999999999"), { runId: 999999999 });
  // 合わなければ null
  assert.strictEqual(srParseHash("#/papers/read/0"), null);
  assert.strictEqual(srParseHash("#/papers/read/012"), null);
  assert.strictEqual(srParseHash("#/papers/read/"), null);
  assert.strictEqual(srParseHash("#/papers/read/abc"), null);
  assert.strictEqual(srParseHash("#/papers/read/130/x"), null);
  assert.strictEqual(srParseHash("#/papers/tree/N12"), null);
  assert.strictEqual(srParseHash("#/papers/read/1300000000"), null);
  assert.strictEqual(srParseHash(""), null);
  assert.strictEqual(srParseHash(null), null);
}

// --- 札の中身 ---
function test_card_parts() {
  const card = sampleCard();
  assert.strictEqual(srTrim("  あ "), "あ");
  assert.strictEqual(srCardNode(card), "N12");
  assert.strictEqual(srCardNode({}), "");
  assert.strictEqual(srCardPaper(card), 37);
  assert.strictEqual(srCardPaper({ paper: { id: 0 } }), 0);
  assert.strictEqual(srCardPaper(null), 0);
  assert.strictEqual(srBackHash(card), "#/papers/tree/N12");
  assert.strictEqual(srBackHash({}), "#/papers/tree");
  // 出典の行
  assert.strictEqual(srCite(37, ""), "#37");
  assert.strictEqual(srCite(37, "§3.2"), "#37 §3.2");
  assert.strictEqual(srCite(37, " Fig.3 "), "#37 Fig.3");
}

// --- 答え（C-07 の add_finding） ---
function test_build_answer() {
  const card = sampleCard();
  // 場所が空なら出典は #<論文> だけ
  const plain = ok(srBuildAnswer(card, { text: "改善が見られた。" }));
  assert.deepStrictEqual(plain, {
    op: "add_finding",
    node: "N12",
    text: "改善が見られた。",
    cites: ["#37"],
    polarity: "support",
    paper: 37,
    run: 130,
  });
  // 場所があれば #<論文> <場所>
  const placed = ok(srBuildAnswer(card, { text: "改善が見られた。", place: "§3.2" }));
  assert.deepStrictEqual(placed.cites, ["#37 §3.2"]);
  // 向きと問い
  const full = ok(srBuildAnswer(card, {
    text: "改善が見られた。", place: " Fig.3", polarity: "refute", question: "データ名は何か",
  }));
  assert.strictEqual(full.polarity, "refute");
  // 読む札の問いは文なので question には入れない。Q の ID だけ入れる
  assert.strictEqual(full.question, undefined);
  assert.strictEqual(ok(srBuildAnswer(card, { text: "あ", question: "Q7" })).question, "Q7");
  assert.deepStrictEqual(full.cites, ["#37 Fig.3"]);
  // 節点・論文・run が入る
  assert.strictEqual(full.node, "N12");
  assert.strictEqual(full.paper, 37);
  assert.strictEqual(full.run, 130);
  // 向きが分からないときは支持
  assert.strictEqual(ok(srBuildAnswer(card, { text: "あ", polarity: "hoge" })).polarity, "support");
  // 空・長すぎる・節点や論文が無い
  assert.match(ng(srBuildAnswer(card, { text: "  " })), /文を入力/);
  assert.match(ng(srBuildAnswer(card, { text: "あ".repeat(1201) })), /1200/);
  assert.match(ng(srBuildAnswer({ ...card, node: {} }, { text: "あ" })), /NODE/);
  assert.match(ng(srBuildAnswer({ ...card, paper: {} }, { text: "あ" })), /論文/);
  // 場所は 80 字まで
  assert.match(ng(srBuildAnswer(card, { text: "あ", place: "あ".repeat(81) })), /80/);
}

// --- 終えるときの本文（R-A5） ---
function test_finish_body() {
  // 知見が 1 つ以上なら普通
  assert.deepStrictEqual(srFinishBody([{ id: "F40", text: "あ" }], ""), {});
  assert.deepStrictEqual(srFinishBody([{ id: "F40" }], ""), {});
  assert.deepStrictEqual(srFinishBody([{ id: "F40" }], "選んだが関係なかった"), {});
  // 0 件で理由が空なら送らない
  assert.strictEqual(srFinishBody([], ""), null);
  assert.strictEqual(srFinishBody([], "   "), null);
  assert.strictEqual(srFinishBody([], null), null);
  assert.strictEqual(srFinishBody(null, undefined), null);
  // 0 件で理由があればそれで終える
  assert.deepStrictEqual(srFinishBody([], "主題が違った"), { no_reflect: "主題が違った" });
  assert.deepStrictEqual(srFinishBody([], "  主題が違った  "), { no_reflect: "主題が違った" });
}

// --- C-01 の値 ---
function test_values() {
  assert.deepStrictEqual(SR_DEPTHS.map((x) => x[0]), ["abstract", "figures", "full"]);
  assert.deepStrictEqual(SR_POLARITY.map((x) => x[0]), ["support", "refute", "neutral", "none"]);
  for (const [id, label] of SR_ROLES) {
    assert.match(id, /^[a-z_]+$/, label);
    assert.ok(label, `${id} に表示名がありません`);
  }
  for (const [key, label] of Object.entries(SR_OWNERS)) {
    assert.match(key, /^[a-z_]+$/);
    assert.ok(label, `${key} に表示名がありません`);
  }
  // 既定の向きは支持（C-01）
  assert.ok(SR_POLARITY.some(([id]) => id === "support"));
}

// --- 実行 ---
function main() {
  test_parse_hash();
  test_card_parts();
  test_build_answer();
  test_finish_body();
  test_values();
  console.log("test_sciread: すべて通りました");
}

main();
