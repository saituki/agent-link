"use strict";
// 研究方針の木の図（scigraph.js）の純関数のテスト（計画 U20）。
// 実行:
//     node apps/phone/tests/test_scigraph.js
//
// DOM が要る描画・拡大・指での操作はブラウザで確かめる。

const assert = require("assert");
const path = require("path");

const {
  sgTreeLayout,
  sgSelectionOf,
  sgEdgeStyle,
  sgEdgePath,
  sgObjBar,
  SG_CARD_W,
  SG_CARD_H,
} = require(path.join(__dirname, "..", "src", "sai_phone", "static", "scigraph.js"));

// --- 要件の図の木: Q0 → Q1 → O★・O → Q2・Q2 → O(Issue) → F・E → F・E・E ---

function node(id, kind, parent, depth, extra) {
  return Object.assign({
    id, kind, parent, depth, text: `${id} の文`, starred: false, answer_kind: null,
    on_path: true, objectivity: null, logicality: null, counts: { q: 0, o: 0, f: 0, e: 0 }, mlflow: null,
  }, extra || {});
}

function sampleStatus() {
  return {
    nodes: [
      node("Q0", "Q", null, 0, { objectivity: { count: 2, target: 3, met: false } }),
      node("Q1", "Q", "Q0", 1),
      node("O1", "O", "Q1", 2, { starred: true, answer_kind: "answer", counts: { q: 1, o: 0, f: 1, e: 0 } }),
      node("O2", "O", "Q1", 2, { answer_kind: "issue", on_path: false }),
      node("Q2", "Q", "O1", 3),
      node("Q3", "Q", "O1", 3),
      node("O3", "O", "Q2", 4, { answer_kind: "issue", starred: true, counts: { q: 0, o: 0, f: 2, e: 0 } }),
      node("F1", "F", "O3", 5),
      node("E1", "E", "O3", 5, { mlflow: "run-a" }),
      node("F2", "F", "O3", 5),
      node("E2", "E", "F1", 6),
    ],
    seeds: [{ id: 1, title: "最初の構想", sentence: "文", path: ["O1", "O3"] }],
  };
}

const byId = (l, id) => l.cards.find((c) => c.id === id);

function overlap(a, b) {
  return a.x < b.x + b.w && b.x < a.x + a.w && a.y < b.y + b.h && b.y < a.y + a.h;
}

function testChildBelowParent() {
  const l = sgTreeLayout({ status: sampleStatus() }, {});
  for (const n of sampleStatus().nodes) {
    if (n.parent) assert.ok(byId(l, n.id).y > byId(l, n.parent).y, n.id);
  }
  assert.strictEqual(byId(l, "Q0").y, 0);
}

function testNoOverlap() {
  const l = sgTreeLayout({ status: sampleStatus() }, {});
  for (let i = 0; i < l.cards.length; i++) {
    for (let j = i + 1; j < l.cards.length; j++) {
      assert.ok(!overlap(l.cards[i], l.cards[j]), `${l.cards[i].id} と ${l.cards[j].id}`);
    }
  }
}

function testParentCentered() {
  const l = sgTreeLayout({ status: sampleStatus() }, {});
  const kids = (id) => l.cards.filter((c) => l.edges.some((e) => e.kind === "tree" && e.from === id && e.to === c.id));
  for (const id of ["Q1", "O1", "O3"]) {
    const ks = kids(id).sort((a, b) => a.x - b.x);
    assert.strictEqual(byId(l, id).x, (ks[0].x + ks[ks.length - 1].x) / 2, id);
  }
}

function testSeedBelowPath() {
  const l = sgTreeLayout({ status: sampleStatus() }, {});
  const s = byId(l, "S1");
  assert.strictEqual(s.kind, "S");
  // 道の一番深い O（O3）の部分木（E2 まで）より下
  assert.ok(s.y > byId(l, "E2").y);
  assert.ok(l.edges.some((e) => e.kind === "seed" && e.from === "O3" && e.to === "S1"));
  assert.deepStrictEqual(sgSelectionOf(s), { kind: "seed", id: "S1" });
  assert.deepStrictEqual(sgSelectionOf(byId(l, "Q0")), { kind: "node", id: "Q0" });
}

function testHideF() {
  const l = sgTreeLayout({ status: sampleStatus() }, { hideF: true });
  assert.strictEqual(byId(l, "F1"), undefined);
  assert.ok(byId(l, "O3").sub.includes("F 2"), byId(l, "O3").sub);
  // F の下の E は見える親（O3）の下に残る
  assert.ok(byId(l, "E2").y > byId(l, "O3").y);
  assert.ok(l.edges.some((e) => e.from === "O3" && e.to === "E2"));
}

function testDimAndContent() {
  const l = sgTreeLayout({ status: sampleStatus() }, {});
  assert.strictEqual(byId(l, "O2").dim, true);
  assert.strictEqual(byId(l, "O1").dim, false);
  assert.ok(byId(l, "O1").sub.includes("★"));
  assert.ok(byId(l, "O3").sub.includes("Issue"));
  assert.strictEqual(byId(l, "Q0").sub, "■■□ 2/3");
  assert.strictEqual(byId(l, "E1").sub, "run-a");
}

function testEmpty() {
  const l = sgTreeLayout({ status: { nodes: [], seeds: [] } }, {});
  assert.deepStrictEqual(l.cards, []);
  assert.strictEqual(sgObjBar(null), "");
}

function testEdgeStyle() {
  assert.strictEqual(sgEdgeStyle("seed").dash, "2 4");
  assert.strictEqual(sgEdgeStyle("tree").dash, "");
  assert.strictEqual(sgEdgeStyle(null).dash, "");
}

function testEdgePath() {
  const a = { x: 0, y: 0, width: SG_CARD_W, height: SG_CARD_H };
  const b = { x: 0, y: 200, width: SG_CARD_W, height: SG_CARD_H };
  const d = sgEdgePath(a, b);
  assert.ok(d.startsWith("M "), d);
  assert.ok(d.includes("C "), d);
  assert.ok(/^M 200 72 /.test(d) || /^M 100 72 /.test(d), d);
}

const tests = [
  ["子は親より下", testChildBelowParent],
  ["札が重ならない", testNoOverlap],
  ["親は子の真ん中", testParentCentered],
  ["SEED の札は道の下", testSeedBelowPath],
  ["hideF", testHideF],
  ["dim と札の中身", testDimAndContent],
  ["空の入力", testEmpty],
  ["線の種類", testEdgeStyle],
  ["線の形", testEdgePath],
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
