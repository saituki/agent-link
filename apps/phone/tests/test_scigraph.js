"use strict";
// 研究方針の木の「根の図」と「候補の表」（scigraph.js）の純関数のテスト（計画 U21「変更内容 5」）。
// 実行:
//     node apps/phone/tests/test_scigraph.js
//
// ここに無いもの（DOM が要る描画・拡大・指での操作）はブラウザで確かめる（計画 U21 の AC1〜AC4）。

const assert = require("assert");
const path = require("path");

// 図の配置は canvaslayout.js の cvlLayoutBoard を使い回すので、
// node 側でも同じ関数が引けるようグローバルに入れておく（ブラウザでは script タグで同じになる）。
const { cvlLayoutBoard } = require(path.join(
  __dirname,
  "..",
  "src",
  "sai_phone",
  "static",
  "canvaslayout.js"
));
global.cvlLayoutBoard = cvlLayoutBoard;

const {
  sgGraphInput,
  sgEdgeStyle,
  sgTableRows,
  sgGateKeys,
  sgStateOf,
  sgEdgePath,
  SG_CARD_W,
  SG_CARD_H,
  SG_EDGE_KINDS,
} = require(path.join(__dirname, "..", "src", "sai_phone", "static", "scigraph.js"));

// --- C-08 の status の見本（項目 3 つ・候補 4 つ・4 種類の線） ---

function item(id, key, title, state, parents) {
  return {
    id, kind: "item", key, title, types: ["choose"], state, fixed: false,
    parents: parents || [], depth: 0, ord: 0, value: "", blank: null, marks: [],
    counts: { questions_open: 0, options: 0, support: 0, refute: 0, papers: 0, pending: 0 },
  };
}

function option(id, title, state, parents) {
  return {
    id, kind: "option", key: null, title, types: [], state, fixed: false,
    parents: parents || [], depth: 1, ord: 0, value: "", blank: null, marks: [],
    counts: { questions_open: 0, options: 0, support: 0, refute: 0, papers: 0, pending: 0 },
  };
}

function sampleStatus() {
  return {
    initialized: true,
    type: { id: "problem", name: "課題駆動" },
    seed: { text: "「頭皮脳波で［？小目的］を行う。」", parts: [] },
    gate: {
      current: 2,
      total: 7,
      name: "小目的",
      list: [
        { name: "出発点", when: "decided(background.goal)", passed: true },
        { name: "小目的", when: "decided(purpose.subgoal) かつ count(x, 3)", passed: false },
      ],
    },
    debts: { pending_proposals: 0, unreflected_papers: 0, unscreened_sets: 0, empty_reviews: 0 },
    nodes: [
      item("N9", "background.goal", "kaho", "decided"),
      item("N12", "purpose.subgoal", "koumokuteki", "open", ["N9"]),
      option("N20", "hasatsu", "candidate", ["N12"]),
      option("N21", "hankyo", "rejected", ["N12"]),
      option("N22", "kaitei", "dormant", ["N12"]),
      option("N23", "selected_b", "selected", ["N12"]),
      item("N30", "method.compare", "hikaku", "open", ["N9"]),
    ],
    edges: [
      { kind: "depends", from: "N9", to: "N30" },
      { kind: "derived", from: "N20", to: "N30" },
      { kind: "analogous", from: "N12", to: "N30" },
    ],
    next: [],
    running: [],
    seeds: [],
    warnings: [],
  };
}

// --- 図の入力 ---

function testGraphInputShowsOptions() {
  const input = sgGraphInput(sampleStatus(), true);
  // 項目と候補が 1 つずつ札になる（元の並びのまま）
  assert.deepStrictEqual(input.cards.map((c) => c.id), ["N9", "N12", "N20", "N21", "N22", "N23", "N30"]);
  // 札の大きさは固定
  for (const c of input.cards) {
    assert.strictEqual(c.type, "card");
    assert.strictEqual(c.width, SG_CARD_W);
    assert.strictEqual(c.height, SG_CARD_H);
    assert.strictEqual(c.group, null);
  }
  // 描く線は 4 種類とも（親子の線は parents から作る）
  const kinds = new Set(input.drawEdges.map((e) => e.kind));
  assert.deepStrictEqual([...kinds].sort(), ["analogous", "depends", "derived", "parent"]);
  // 親子の線（parents から作ったもの）が入っている
  assert.ok(input.drawEdges.some((e) => e.kind === "parent" && e.from === "N12" && e.to === "N20"));
}

function testGraphInputHidesOptions() {
  const input = sgGraphInput(sampleStatus(), false);
  // 候補を隠すと項目だけになる
  assert.deepStrictEqual(input.cards.map((c) => c.id), ["N9", "N12", "N30"]);
  // 隠した候補につながる線は描かない（始点と終点が札の無い線は落とす）
  for (const e of input.drawEdges) {
    assert.ok(["N9", "N12", "N30"].includes(e.from), `線のもう片方が消えています: ${JSON.stringify(e)}`);
    assert.ok(["N9", "N12", "N30"].includes(e.to), `線のもう片方が消えています: ${JSON.stringify(e)}`);
  }
  // showOptions を省くと候補を出す
  assert.strictEqual(sgGraphInput(sampleStatus()).cards.length, 7);
}

function testLayoutEdgesExcludeDerivedAndAnalogous() {
  const input = sgGraphInput(sampleStatus(), true);
  // 配置に使うのは親子の線と依存だけ
  const kinds = new Set(input.layoutEdges.map((e) => e.kind));
  assert.deepStrictEqual([...kinds].sort(), ["depends", "parent"]);
  assert.ok(!input.layoutEdges.some((e) => e.kind === "derived"));
  assert.ok(!input.layoutEdges.some((e) => e.kind === "analogous"));
  // 描く線よりは少ない（由来と同型は配置に入れない）
  assert.ok(input.drawEdges.length > input.layoutEdges.length);
}

function testLayoutIsStable() {
  // 同じ status なら 2 回呼んでも同じ位置（開くたびに図が動かない）
  const a = sgGraphInput(sampleStatus(), true);
  const b = sgGraphInput(sampleStatus(), true);
  assert.deepStrictEqual(cvlLayoutBoard(a.cards, a.layoutEdges), cvlLayoutBoard(b.cards, b.layoutEdges));
  // 別の呼び出しでも同じ（入力を壊さない）
  const c = sgGraphInput(sampleStatus(), true);
  assert.deepStrictEqual(cvlLayoutBoard(c.cards, c.layoutEdges), cvlLayoutBoard(a.cards, a.layoutEdges));
}

function testGraphInputEmpty() {
  // status が空でもJF を出さない
  assert.deepStrictEqual(sgGraphInput(null, true), { cards: [], layoutEdges: [], drawEdges: [] });
  assert.deepStrictEqual(sgGraphInput({}, true).cards, []);
  assert.deepStrictEqual(sgGraphInput({ nodes: [null, {}] }, true).cards, []);
}

// --- 線の種類 ---

function testEdgeStyle() {
  // 4 種類（親子・依存・由来・同型）
  assert.strictEqual(SG_EDGE_KINDS.length, 4);
  assert.deepStrictEqual(sgEdgeStyle("parent"), { dash: "", arrow: false, colorVar: "var(--sci-edge-parent)" });
  assert.deepStrictEqual(sgEdgeStyle("depends"), { dash: "", arrow: true, colorVar: "var(--sci-edge-depends)" });
  assert.deepStrictEqual(sgEdgeStyle("derived"), { dash: "8 5", arrow: false, colorVar: "var(--sci-edge-derived)" });
  assert.deepStrictEqual(sgEdgeStyle("analogous"), { dash: "2 4", arrow: false, colorVar: "var(--sci-edge-analogous)" });
  // 4 種類は色も破線も違っている（見分けられる）
  const colors = SG_EDGE_KINDS.map((k) => sgEdgeStyle(k[0]).colorVar);
  assert.strictEqual(new Set(colors).size, 4);
  const dashes = SG_EDGE_KINDS.map((k) => sgEdgeStyle(k[0]).dash);
  assert.strictEqual(new Set(dashes).size, 3); // 実線 1 つ・破線 1 つ・点線 1 つ
  // 矢印は依存だけ
  assert.deepStrictEqual(SG_EDGE_KINDS.filter((k) => sgEdgeStyle(k[0]).arrow).map((k) => k[0]), ["depends"]);
  // 表に無い種類・null でも止まらない
  assert.strictEqual(sgEdgeStyle(null).colorVar, "var(--sci-edge-parent)");
  assert.strictEqual(sgEdgeStyle("???").arrow, false);
}

function testEdgePath() {
  // 縦に並ぶ 2 つの矩形は上から下へ出る
  const a = { x: 0, y: 0, width: 200, height: 72 };
  const b = { x: 0, y: 200, width: 200, height: 72 };
  const d = sgEdgePath(a, b);
  assert.ok(d.startsWith("M "), d);
  assert.ok(d.includes("C "), d);
  // 端点は矩形の上にあり、下にある（数字だけ見た並びではない）
  assert.ok(/^M 200 72 /.test(d) || /^M 100 72 /.test(d), d);
}

// --- 札の状態 ---

function testStateOf() {
  // 項目と候補で別の記号・色
  assert.strictEqual(sgStateOf({ kind: "item", state: "decided" }).mark, "●");
  assert.strictEqual(sgStateOf({ kind: "item", state: "open" }).mark, "○");
  assert.strictEqual(sgStateOf({ kind: "option", state: "selected" }).mark, "◆");
  assert.strictEqual(sgStateOf({ kind: "option", state: "rejected" }).mark, "✕");
  assert.strictEqual(sgStateOf({ kind: "option", state: "dormant" }).mark, "◌");
  // 知らない状態でも止まらない
  assert.strictEqual(sgStateOf({ kind: "item", state: "???" }).mark, "○");
  assert.strictEqual(sgStateOf(null).mark, "○");
}

// --- 関門の条件 ---

function testGateKeys() {
  // decided(purpose.subgoal) かつ count(x, 3) → purpose.subgoal と x
  assert.deepStrictEqual(sgGateKeys(sampleStatus()), ["purpose.subgoal", "x"]);
  // 全部通っているとき（passed が false のものが無い）は 1 番目の関門を見る
  const all = sampleStatus();
  all.gate.list[1].passed = true;
  all.gate.current = 1;
  all.gate.list[0].passed = false;
  all.gate.list[0].when = "count(data.quality, 2)";
  assert.deepStrictEqual(sgGateKeys(all), ["data.quality"]);
  // 関門が無いときは空
  assert.deepStrictEqual(sgGateKeys(null), []);
  assert.deepStrictEqual(sgGateKeys({}), []);
  assert.deepStrictEqual(sgGateKeys({ gate: { list: [] } }), []);
  // 同じ key は 1 回だけ
  const dup = sampleStatus();
  dup.gate.list[1].when = "decided(purpose.subgoal) かつ at_least(purpose.subgoal, 2)";
  assert.deepStrictEqual(sgGateKeys(dup), ["purpose.subgoal"]);
}

// --- 候補の表 ---

function sampleDetail() {
  return {
    node: { id: "N12", kind: "item", key: "purpose.subgoal", title: "小目的", types: ["choose"], state: "open" },
    questions: [],
    options: [
      { id: "N20", title: "発作部位の予測", state: "candidate", support: 2, refute: 1, papers: 3 },
      { id: "N21", title: "応答潜時との比較", state: "rejected", support: 0, refute: 2, papers: 1 },
      { id: "N22", title: "部位の同定", state: "dormant", support: 0, refute: 0, papers: 0 },
      { id: "N23", title: "波形の特徴", state: "selected", support: 5, refute: 0, papers: 4 },
      { id: "N24", title: "睡眠段階", state: "candidate", support: 1, refute: 0, papers: 1 },
    ],
    findings: [
      { id: "F31", node: "N20", text: "前頭葉の兆候が出た。側頭葉は出にくかった。ほかも調べた。", polarity: "support" },
      { id: "F32", node: "N20", text: "別の研究では逆だった。", polarity: "refute" },
      { id: "F33", node: "N23", text: "波形の長さが区別の目印になった。あと 1 文。", polarity: "support" },
    ],
    axes: ["臨床で使える", "少ないデータで学べる"],
  };
}

function testTableRowsOrder() {
  const rows = sgTableRows(sampleDetail());
  // 選択 → 候補 → 休眠 → 却下。同じ状態は元の並びのまま
  assert.deepStrictEqual(rows.map((r) => r.id), ["N23", "N20", "N24", "N22", "N21"]);
}

function testTableRowsFields() {
  const rows = sgTableRows(sampleDetail());
  const n23 = rows[0];
  assert.strictEqual(n23.title, "波形の特徴");
  assert.strictEqual(n23.mark, "◆");
  assert.strictEqual(n23.label, "選択");
  assert.strictEqual(n23.support, 5);
  assert.strictEqual(n23.refute, 0);
  assert.strictEqual(n23.papers, 4);
  // 知見の要約は、その候補の支持の知見の先頭の 1 文だけ
  assert.strictEqual(n23.summary, "波形の長さが区別の目印になった。");
  // 2 番目以降の文は出さない
  const n20 = rows[1];
  assert.strictEqual(n20.summary, "前頭葉の兆候が出た。");
  // 支持の知見が無い候補は空
  const n22 = rows[3];
  assert.strictEqual(n22.summary, "");
  // Till the end
  assert.strictEqual(rows[4].mark, "✕");
}

function testTableRowsEmpty() {
  assert.deepStrictEqual(sgTableRows(null), []);
  assert.deepStrictEqual(sgTableRows({}), []);
  assert.deepStrictEqual(sgTableRows({ options: [] }), []);
  assert.deepStrictEqual(sgTableRows({ options: [null, {}] }), []); // id の無い候補は出さない
  // 状態不明の候補は最後に置く（表が止まらない）
  const odd = sgTableRows({ options: [{ id: "N90", title: "x", state: "???" }] });
  assert.strictEqual(odd.length, 1);
  assert.strictEqual(odd[0].state, "???");
}

// --- 実行 ---

const tests = [
  ["図の入力（候補を出す）", testGraphInputShowsOptions],
  ["図の入力（候補を隠す）", testGraphInputHidesOptions],
  ["配置に使う線（由来と同型は外す）", testLayoutEdgesExcludeDerivedAndAnalogous],
  ["同じ status なら同じ配置", testLayoutIsStable],
  ["空の status", testGraphInputEmpty],
  ["線の種類", testEdgeStyle],
  ["線のパス", testEdgePath],
  ["札の状態", testStateOf],
  ["関門の条件の key", testGateKeys],
  ["表の行の並び", testTableRowsOrder],
  ["表の行の中身", testTableRowsFields],
  ["表の行（空）", testTableRowsEmpty],
];

let failed = 0;
for (const [name, fn] of tests) {
  try {
    fn();
    console.log(`  ok  ${name}`);
  } catch (e) {
    failed += 1;
    console.error(`  NG  ${name}: ${e && e.message}`);
  }
}
if (failed) {
  console.error(`test_scigraph: ${failed} 件が失敗しました`);
  process.exit(1);
}
console.log("test_scigraph: すべて通りました");
