"use strict";
// 共通部品（static/ui.js・U4・共有契約 C-F3）の DOM を使わない部分のテスト。
// 実行:
//     node tests/test_ui.js

const assert = require("assert");
const path = require("path");

const U = require(path.join(__dirname, "..", "src", "sai_web", "static", "ui.js"));

// 広いと思う境目（900px）
function testWideAt() {
  assert.strictEqual(U.SAI_WIDE_PX, 900);
  assert.strictEqual(U.saiWideAt(1280), true, "1280px は広い");
  assert.strictEqual(U.saiWideAt(900), true, "ちょうど 900px は広い");
  assert.strictEqual(U.saiWideAt(899), false, "899px は狭い");
  assert.strictEqual(U.saiWideAt(390), false, "iPhone の幅は狭い");
  assert.strictEqual(U.saiWideAt(0), false);
  assert.strictEqual(U.saiWideAt(-1), false);
  assert.strictEqual(U.saiWideAt(NaN), false);
  assert.strictEqual(U.saiWideAt("1280"), true, "文字列でも数値として読む");
  assert.strictEqual(U.saiWideAt(undefined), false);
}

// C-F5 の用語が全部あって、1 つ 1 つ名前と説明がある
// U19 が research-tree の用語を足し、U41 が今の用語 19 個に作り直した（共有契約 C-14）。
const C_F5 = [
  "run", "todo", "running", "review", "done", "workflow", "kanban", "block",
  "template", "settings", "runner", "round", "global_round", "waggle", "daemon",
  "card", "task_text", "phase", "bibtex", "attention", "state", "mind",
  "sci_research_tree", "sci_seed", "sci_root", "sci_sprout", "sci_branch",
  "sci_path", "sci_node", "sci_top_q", "sci_star", "sci_rel",
  "sci_proposal", "sci_triage", "sci_card", "sci_guide",
  "sci_research_type", "sci_lab_agent",
];

function testTerms() {
  for (const key of C_F5) {
    assert.ok(U.SAI_TERMS[key], `${key} が SAI_TERMS にあること`);
    const t = U.SAI_TERMS[key];
    assert.ok(typeof t.label === "string" && t.label.length > 0, `${key} に表示名があること`);
    assert.ok(typeof t.text === "string" && t.text.length >= 10, `${key} に説明があること`);
  }
  assert.deepStrictEqual(Object.keys(U.SAI_TERMS).sort(), C_F5.slice().sort(), "用語が余りなく揃う");
  // 表を引き出す関数も同じ値を返す
  assert.strictEqual(U.saiTermText("daemon"), U.SAI_TERMS.daemon.text);
  assert.strictEqual(U.saiTermLabel("daemon"), U.SAI_TERMS.daemon.label);
  assert.strictEqual(U.saiTermText("ない用語"), "");
  assert.strictEqual(U.saiTermLabel("ない用語"), "");
  // 全部の用語の表（[{key,label,text}]）
  const list = U.saiTermList();
  assert.strictEqual(list.length, C_F5.length);
  assert.deepStrictEqual(list.map((x) => x.key).sort(), C_F5.slice().sort());
  for (const x of list) {
    assert.ok(x.label && x.text, `${x.key} に表示名と説明があること`);
  }
  // research-tree の用語は今の語（軸・A）で書く。旧い語（top_Q から・Answer の O）は残さない
  assert.strictEqual(U.SAI_TERMS.sci_top_q.label, "軸");
  assert.ok(U.SAI_TERMS.sci_rel.text.includes("condition"), "sci_rel の説明が condition に触れること");
  assert.ok(!("sci_bottom_q" in U.SAI_TERMS), "旧い用語 sci_bottom_q は無いこと（Q の全面廃止）");
  assert.ok(!/Q/.test(U.SAI_TERMS.sci_top_q.text + U.SAI_TERMS.sci_guide.text), "軸と GUIDE の説明に Q が無いこと");
  for (const key of ["sci_node", "sci_root", "sci_star"]) {
    assert.ok(!/top_Q|\bO\b|Answer/.test(U.SAI_TERMS[key].text), `${key} に旧い語が残っていないこと`);
  }
}

// 用語の説明が日本語の文として読めること（1〜3 文・必ず句点がある）
function testTermTextsAreSentences() {
  for (const { key, text } of U.saiTermList()) {
    assert.ok(/[。？！]/.test(text), `${key} の説明が文になっていること: ${text}`);
    // 日本語（かな・漢字）と、実際に使う英単語・記号と空白だけ
    // （？ は文の句点として上の検査にも使っている。research-tree の「［？］」の記号にも使う、〜 は範囲の記号として許す）
    const stripped = text.replace(/[一-鿿぀-ゟ゠-ヿA-Za-z0-9\s.,:;!?/()（）「」【】［］→·…#@'"=\-_*`+%$&^~|<>{}\[\]。、？！〜～]/g, "");
    assert.strictEqual(stripped, "", `${key} の説明に読みでない文字があること: ${text}`);
    assert.ok(text.length <= 140, `${key} の説明が長すぎないこと`);
  }
}

// 函数の形（C-F3 の名前が全部用意されていること）
function testExports() {
  for (const name of [
    "saiApi", "saiToast", "saiConfirm", "saiPrompt", "saiEmpty", "saiTerm",
    "saiIsWide", "saiSheet", "saiSplitShow", "saiIsAxis",
  ]) {
    assert.strictEqual(typeof U[name], "function", `${name} があること`);
  }
}

// 軸の A（親の無い A）の判定。research-tree の画面はこの 1 か所で見分ける
function testIsAxis() {
  assert.strictEqual(U.saiIsAxis({ id: "A1", kind: "A", parent: null }), true, "親の無い A は軸");
  assert.strictEqual(U.saiIsAxis({ id: "A2", kind: "A", parent: "A1" }), false, "親のある A は軸でない");
  assert.strictEqual(U.saiIsAxis({ id: "F3", kind: "F", parent: null }), false, "親の無い F は軸でない");
  assert.strictEqual(U.saiIsAxis({ id: "D1", kind: "D", parent: null }), false, "seed（D）は軸でない");
  assert.strictEqual(U.saiIsAxis({ id: "S1", kind: "S", parent: null }), false, "SPROUT は軸でない");
  assert.strictEqual(U.saiIsAxis(null), false);
  assert.strictEqual(U.saiIsAxis(undefined), false);
}

function main() {
  testWideAt();
  testTerms();
  testTermTextsAreSentences();
  testExports();
  testIsAxis();
  console.log("test_ui: すべて通りました");
}

main();
