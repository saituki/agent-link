"use strict";
// Canvas の左右の欄（canvaspanel.js）の純関数のテスト（計画 U07「変更内容 4」）。
// 実行:
//     node tests/test_canvaspanel.js
//
// ここに無いもの（DOM が要る描画・操作）はブラウザで確かめる（計画 U07 の AC2〜AC6）。

const assert = require("assert");
const path = require("path");

const staticDir = path.join(__dirname, "..", "src", "sai_phone", "static");

// 画面では <script> で同じ範囲に読まれるので、canvas.js の定数がそのまま使える。
// node ではファイルごとにスコープが分かれるので、canvas.js の表を global に置いて同じ状態を作る
// （無いつながりの色が U06 の CV_EDGE_KINDS と一致することを確かめるため）。
const canvas = require(path.join(staticDir, "canvas.js"));
globalThis.CV_EDGE_KINDS = canvas.CV_EDGE_KINDS;

const {
  cvpFilterSidebar,
  cvpIsOnBoard,
  cvpPaperOnBoard,
  cvpRows,
  cvpRelationColor,
} = require(path.join(staticDir, "canvaspanel.js"));

// CV_EDGE_KINDS が読めたか（無いつながりの色が U06 と揃っているかを確かめるため）
assert.ok(Array.isArray(globalThis.CV_EDGE_KINDS) && globalThis.CV_EDGE_KINDS.length > 0,
  "canvas.js の CV_EDGE_KINDS を読み込めませんでした");

// show の応答の sidebar と同じ形（node / papers.js のテストの書き方にそろえる）
const SIDEBAR = [
  {
    paper_id: 37,
    title: "Attention Is All You Need",
    note: { node_id: "note-37", summary: "Sequence to Sequence / arXiv: ..." },
    reports: [
      { report_id: "R-0007", node_id: "report-R-0007", title: "前処理の比較", summary: "split を比べた" },
      { report_id: "R-0008", node_id: "report-R-0008", title: "位置埋め込み", summary: "sinusoidal" },
    ],
  },
  {
    paper_id: 18,
    title: "causal discovery",
    note: { node_id: "note-18", summary: "CausaLab / Authors: ..." },
    reports: [],
  },
  {
    paper_id: 25,
    title: "LLM for Data Science Workflows",
    note: null,
    reports: [{ report_id: "R-0009", node_id: "report-R-0009", title: "調査", summary: "survey" }],
  },
];

// --- cvpFilterSidebar（題名で絞り込む） ---

function testFilterKeepsAllWhenEmpty() {
  assert.deepStrictEqual(cvpFilterSidebar(SIDEBAR, ""), SIDEBAR);
  assert.deepStrictEqual(cvpFilterSidebar(SIDEBAR, "   "), SIDEBAR);
  assert.deepStrictEqual(cvpFilterSidebar(SIDEBAR, null), SIDEBAR);
  // 元の配列は変えない
  const copy = SIDEBAR.slice();
  cvpFilterSidebar(SIDEBAR, "a");
  assert.deepStrictEqual(SIDEBAR, copy);
}

function testFilterByTitle() {
  assert.deepStrictEqual(cvpFilterSidebar(SIDEBAR, "Attention").map((p) => p.paper_id), [37]);
  assert.deepStrictEqual(cvpFilterSidebar(SIDEBAR, "causal").map((p) => p.paper_id), [18]);
}

function testFilterIgnoresCase() {
  assert.deepStrictEqual(cvpFilterSidebar(SIDEBAR, "ATTENTION").map((p) => p.paper_id), [37]);
  assert.deepStrictEqual(cvpFilterSidebar(SIDEBAR, "llm for").map((p) => p.paper_id), [25]);
}

function testFilterTrimsAndFindsNothing() {
  assert.deepStrictEqual(cvpFilterSidebar(SIDEBAR, "  You Need ").map((p) => p.paper_id), [37]);
  assert.deepStrictEqual(cvpFilterSidebar(SIDEBAR, "TTTT"), []);
  // note.md の先頭行には出る文字が베어도、題名で無ければ出さない
  assert.deepStrictEqual(cvpFilterSidebar(SIDEBAR, "Authors"), []);
}

function testFilterHandlesBrokenInput() {
  assert.deepStrictEqual(cvpFilterSidebar(null, "a"), []);
  assert.deepStrictEqual(cvpFilterSidebar("x", "a"), []);
  assert.deepStrictEqual(cvpFilterSidebar([null, { title: "A" }], "").length, 2);
  assert.deepStrictEqual(cvpFilterSidebar([{ title: "A" }, { title: null }], "a").length, 1);
}

// --- cvpIsOnBoard（● か ○ か） ---

function testIsOnBoardById() {
  const nodes = [{ id: "note-37" }, { id: "report-R-0007" }];
  assert.strictEqual(cvpIsOnBoard("note-37", 37, nodes), true);
  assert.strictEqual(cvpIsOnBoard("report-R-0007", 37, nodes), true);
  assert.strictEqual(cvpIsOnBoard("note-18", 18, nodes), false);
  assert.strictEqual(cvpIsOnBoard("report-R-0008", 37, nodes), false);
}

function testIsOnBoardNoteTreatsPaperCardAsSame() {
  // note.md の無い論文は paper-<P> の札になる。note-<P> の行も ● にする
  const nodes = [{ id: "paper-37" }];
  assert.strictEqual(cvpIsOnBoard("note-37", 37, nodes), true);
  assert.strictEqual(cvpIsOnBoard("note-18", 18, nodes), false);
  // paper-<P> の札は別の論文の note-<P> とは一致しない
  assert.strictEqual(cvpIsOnBoard("note-371", 371, nodes), false);
}

function testIsOnBoardDoesNotCrossReports() {
  // 同じ論文の note-<P> が出ても、report の行までは ● にしない
  const nodes = [{ id: "note-37" }];
  assert.strictEqual(cvpIsOnBoard("report-R-0007", 37, nodes), false);
  assert.strictEqual(cvpIsOnBoard("report-R-0007", null, nodes), false);
}

function testIsOnBoardHandlesBrokenInput() {
  assert.strictEqual(cvpIsOnBoard(null, 37, [{ id: "note-37" }]), false);
  assert.strictEqual(cvpIsOnBoard("", 37, []), false);
  assert.strictEqual(cvpIsOnBoard("note-37", 37, null), false);
  assert.strictEqual(cvpIsOnBoard("note-37", 37, [null, { id: null }]), false);
}

// --- cvpPaperOnBoard（論文の行を濃くするか） ---

function testPaperOnBoard() {
  assert.strictEqual(cvpPaperOnBoard(37, [{ id: "note-37" }]), true);
  assert.strictEqual(cvpPaperOnBoard(37, [{ id: "paper-37" }]), true);
  // report の札は論文自身の札と数えない
  assert.strictEqual(cvpPaperOnBoard(37, [{ id: "report-R-0007" }]), false);
  assert.strictEqual(cvpPaperOnBoard(null, [{ id: "note-1" }]), false);
  assert.strictEqual(cvpPaperOnBoard(18, null), false);
}

// --- cvpRows（描く行） ---

function testRowsOrder() {
  const rows = cvpRows(SIDEBAR, []);
  assert.deepStrictEqual(rows.map((r) => `${r.type}:${r.label}`), [
    "paper:Attention Is All You Need",
    "note:note.md",
    "report:R-0007",
    "report:R-0008",
    "paper:causal discovery",
    "note:note.md",
    "paper:LLM for Data Science Workflows",
    "report:R-0009",
  ]);
}

function testRowsFields() {
  const rows = cvpRows([SIDEBAR[0]], []);
  assert.deepStrictEqual(rows[0], {
    type: "paper", paper_id: 37, node_id: null,
    label: "Attention Is All You Need", summary: "", title: "",
  });
  assert.deepStrictEqual(rows[1], {
    type: "note", paper_id: 37, node_id: "note-37",
    label: "note.md", summary: "Sequence to Sequence / arXiv: ...", title: "",
  });
  assert.deepStrictEqual(rows[2], {
    type: "report", paper_id: 37, node_id: "report-R-0007",
    label: "R-0007", summary: "split を比べた", title: "前処理の比較",
  });
}

function testRowsSkipNoteWhenMissing() {
  const rows = cvpRows([SIDEBAR[2]], []);
  assert.deepStrictEqual(rows.map((r) => r.type), ["paper", "report"]);
}

function testRowsFoldedPaperHasNoChildren() {
  const rows = cvpRows(SIDEBAR, [37]);
  assert.deepStrictEqual(rows.map((r) => `${r.type}:${r.label}`), [
    "paper:Attention Is All You Need",
    "paper:causal discovery",
    "note:note.md",
    "paper:LLM for Data Science Workflows",
    "report:R-0009",
  ]);
}

function testRowsFoldMatchesNumbersAndStrings() {
  // localStorage から読んだので論文番号が文字列になっていても同じ
  assert.strictEqual(cvpRows(SIDEBAR, ["37"]).length, cvpRows(SIDEBAR, [37]).length);
  // 畳んでいない論文の子は残る
  assert.ok(cvpRows(SIDEBAR, [18]).some((r) => r.node_id === "note-37"));
}

function testRowsHandlesBrokenInput() {
  assert.deepStrictEqual(cvpRows(null, null), []);
  assert.deepStrictEqual(cvpRows([null, { paper_id: 1, title: "A", note: null, reports: null }], null).map((r) => r.type), ["paper"]);
  // node_id の無い note と report は描かない（押せない行を作らない）
  const rows = cvpRows([{ paper_id: 1, title: "A", note: { summary: "x" }, reports: [null, { report_id: "R-1" }] }], []);
  assert.deepStrictEqual(rows.map((r) => r.type), ["paper"]);
}

// --- cvpRelationColor（無いつながりの色） ---

function testRelationColor() {
  // canvas.js の表 CV_EDGE_KINDS と同じ色
  assert.strictEqual(cvpRelationColor("引用"), "#2563eb");
  assert.strictEqual(cvpRelationColor("使った report"), "#1d7a45");
  assert.strictEqual(cvpRelationColor("使った論文"), "#1d7a45");
  assert.strictEqual(cvpRelationColor("note が言及"), "#d97706");
  assert.strictEqual(cvpRelationColor("関係する report"), "#6b7280");
  // 知らない文・null は線（既定の色）
  assert.strictEqual(cvpRelationColor("知らない"), "#6b7280");
  assert.strictEqual(cvpRelationColor(null), "#6b7280");
  // 線の種類の表（U06 の CV_EDGE_KINDS）と 1 項目ずつ一致する
  const table = Object.fromEntries(globalThis.CV_EDGE_KINDS.map((r) => [r[0], r[2]]));
  assert.strictEqual(cvpRelationColor("引用"), table.citation);
  assert.strictEqual(cvpRelationColor("使った report"), table.uses);
  assert.strictEqual(cvpRelationColor("元にした report"), table.derived);
  assert.strictEqual(cvpRelationColor("note が言及"), table.mentions);
}

// --- 実行 ---

const tests = [
  testFilterKeepsAllWhenEmpty,
  testFilterByTitle,
  testFilterIgnoresCase,
  testFilterTrimsAndFindsNothing,
  testFilterHandlesBrokenInput,
  testIsOnBoardById,
  testIsOnBoardNoteTreatsPaperCardAsSame,
  testIsOnBoardDoesNotCrossReports,
  testIsOnBoardHandlesBrokenInput,
  testPaperOnBoard,
  testRowsOrder,
  testRowsFields,
  testRowsSkipNoteWhenMissing,
  testRowsFoldedPaperHasNoChildren,
  testRowsFoldMatchesNumbersAndStrings,
  testRowsHandlesBrokenInput,
  testRelationColor,
];

let failed = 0;
for (const fn of tests) {
  try {
    fn();
    console.log(`ok  ${fn.name}`);
  } catch (e) {
    failed += 1;
    console.error(`NG  ${fn.name}: ${e && e.message}`);
  }
}
if (failed) {
  console.error(`test_canvaspanel: ${failed} 件が失敗しました`);
  process.exit(1);
}
console.log(`test_canvaspanel: ${tests.length} 件すべて通りました`);
