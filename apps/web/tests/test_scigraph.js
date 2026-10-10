"use strict";
// research-tree の図（scigraph.js）の純関数のテスト（計画 U33・U06 で Q・A・F・E・S に合わせた）。
// 実行:
//     node apps/web/tests/test_scigraph.js
//
// DOM が要る描画・拡大・指での操作はブラウザで確かめる。

const assert = require("assert");
const path = require("path");

const {
  sgTreeLayout,
  sgSelectionOf,
  sgSproutKeep,
  sgEdgeStyle,
  sgEdgePath,
  sgCenterView,
  sgTextScale,
  sgTitleLines,
  SG_CARD_W,
  SG_CARD_H,
  SG_TREE_STEP,
} = require(path.join(__dirname, "..", "src", "sai_web", "static", "scigraph.js"));

// --- 4 layer の図の木 ---
// 下から ROOT（Q0 → Q1 → A1・A2 → Q2・Q3 → A3 → F1・F2）、SEED の札、
// SPROUT（S1・S2）、その上の BRANCH（S1 → E1 → E2・F3、S2 → E4）。
// A2 は、どの PATH にも入っていない答え。

function node(id, kind, parent, depth, extra) {
  return Object.assign({
    id, kind, parent, depth, text: `${id} の文`, starred: false, stance: null,
    counts: { q: 0, a: 0, f: 0, e: 0 }, mlflow: null, paths: [], bottom_q: false,
  }, extra || {});
}

function sampleStatus() {
  return {
    seed: { type: { id: "problem", name: "PROBLEM_DRIVEN" }, mind: [] },
    nodes: [
      node("Q0", "Q", null, 0),
      node("Q1", "Q", "Q0", 1),
      node("A1", "A", "Q1", 2, { starred: true, counts: { q: 1, a: 0, f: 1, e: 0 } }),
      node("A2", "A", "Q1", 2),
      node("Q2", "Q", "A1", 3),
      node("Q3", "Q", "A1", 3),
      node("A3", "A", "Q2", 4, { starred: true, counts: { q: 0, a: 0, f: 2, e: 0 } }),
      node("F1", "F", "A3", 5, { stance: "support" }),
      node("F2", "F", "A3", 5, { stance: "deny" }),
      node("S1", "S", null, 1, { text: "最初の構想", paths: ["A3"] }),
      node("E1", "E", "S1", 2, { mlflow: "run-a" }),
      node("E2", "E", "E1", 3),
      node("F3", "F", "E1", 3, { stance: "issue" }),
      node("S2", "S", null, 1, { text: "二つ目の構想", paths: ["A1"] }),
      node("E4", "E", "S2", 2),
    ],
  };
}

const byId = (l, id) => l.cards.find((c) => c.id === id);

function overlap(a, b) {
  return a.x < b.x + b.w && b.x < a.x + a.w && a.y < b.y + b.h && b.y < a.y + a.h;
}

const hasEdge = (l, from, to, kind) => l.edges.some((e) => e.from === from && e.to === to && e.kind === kind);

function testLayers() {
  const l = sgTreeLayout({ status: sampleStatus() }, {});
  // SEED が y=0、top_Q が +STEP、S が -STEP、S の下の E が -2×STEP
  assert.strictEqual(byId(l, "SEED").kind, "SEED");
  assert.strictEqual(byId(l, "SEED").y, 0);
  assert.strictEqual(byId(l, "Q0").y, SG_TREE_STEP);
  assert.strictEqual(byId(l, "Q1").y, 2 * SG_TREE_STEP);
  assert.strictEqual(byId(l, "S1").y, -SG_TREE_STEP);
  assert.strictEqual(byId(l, "S2").y, -SG_TREE_STEP);
  assert.strictEqual(byId(l, "E1").y, -2 * SG_TREE_STEP);
  assert.strictEqual(byId(l, "E2").y, -3 * SG_TREE_STEP);
  assert.strictEqual(byId(l, "F3").y, -3 * SG_TREE_STEP);
  // 札の中身
  assert.strictEqual(byId(l, "SEED").text, "SEED");
  assert.strictEqual(byId(l, "SEED").sub, "PROBLEM_DRIVEN");
  assert.strictEqual(byId(l, "S1").text, "S1 最初の構想");
  assert.strictEqual(byId(l, "S1").sub, "PATH 1");
}

function testEdgeKinds() {
  const l = sgTreeLayout({ status: sampleStatus() }, {});
  // SEED → top_Q は seed、SEED → S は sprout、Parent / Child は tree
  assert.ok(hasEdge(l, "SEED", "Q0", "seed"), JSON.stringify(l.edges));
  assert.ok(hasEdge(l, "SEED", "S1", "sprout"), JSON.stringify(l.edges));
  assert.ok(hasEdge(l, "SEED", "S2", "sprout"), JSON.stringify(l.edges));
  assert.ok(hasEdge(l, "Q0", "Q1", "tree"));
  assert.ok(hasEdge(l, "S1", "E1", "tree"));
  assert.ok(hasEdge(l, "E1", "E2", "tree"));
  // S を ROOT の Parent / Child としてつながない
  assert.ok(!hasEdge(l, "Q0", "S1", "tree"));
  // 線の種類は 3 つ
  assert.deepStrictEqual(l.edges.map((e) => e.kind).filter((k) => !["tree", "seed", "sprout"].includes(k)), []);
}

function testSelection() {
  const l = sgTreeLayout({ status: sampleStatus() }, {});
  assert.deepStrictEqual(sgSelectionOf(byId(l, "SEED")), { kind: "seed", id: "SEED" });
  assert.deepStrictEqual(sgSelectionOf(byId(l, "S1")), { kind: "node", id: "S1" });
  assert.deepStrictEqual(sgSelectionOf(byId(l, "Q0")), { kind: "node", id: "Q0" });
}

function testChildDirection() {
  const l = sgTreeLayout({ status: sampleStatus() }, {});
  // ROOT は子が親より下
  for (const [c, p] of [["Q1", "Q0"], ["A1", "Q1"], ["Q2", "A1"], ["A3", "Q2"], ["F1", "A3"], ["F2", "A3"]]) {
    assert.ok(byId(l, c).y > byId(l, p).y, `${c} > ${p}`);
  }
  // BRANCH は子が親より上
  for (const [c, p] of [["E1", "S1"], ["E2", "E1"], ["F3", "E1"], ["E4", "S2"]]) {
    assert.ok(byId(l, c).y < byId(l, p).y, `${c} < ${p}`);
  }
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
  for (const id of ["Q1", "A1", "Q2", "S1", "E1", "S2"]) {
    const ks = kids(id).sort((a, b) => a.x - b.x);
    assert.strictEqual(byId(l, id).x, (ks[0].x + ks[ks.length - 1].x) / 2, id);
  }
}

function testSproutKeep() {
  // S1 を選ぶと、S1 自身・BRANCH（E1・E2・F3）・PATH（A3 から top_Q までの Q・A）が残る
  const keep = sgSproutKeep(sampleStatus(), "S1");
  assert.deepStrictEqual(
    [...keep].sort(),
    ["S1", "E1", "E2", "F3", "A3", "Q2", "A1", "Q1", "Q0"].sort(),
  );
  // PATH の外の F1・F2・Q3・A2 と、別の SPROUT は残らない
  for (const id of ["F1", "F2", "Q3", "A2", "S2", "E4"]) assert.ok(!keep.has(id), id);
  // S2 は A1 までの道だけ
  const keep2 = sgSproutKeep(sampleStatus(), "S2");
  assert.deepStrictEqual([...keep2].sort(), ["S2", "E4", "A1", "Q1", "Q0"].sort());
  // SPROUT でない ID には null
  assert.strictEqual(sgSproutKeep(sampleStatus(), "Q0"), null);
  assert.strictEqual(sgSproutKeep(sampleStatus(), "S9"), null);
  assert.strictEqual(sgSproutKeep(sampleStatus(), null), null);
}

function testHideE() {
  const l = sgTreeLayout({ status: sampleStatus() }, { hideE: true });
  assert.strictEqual(byId(l, "E1"), undefined);
  assert.strictEqual(byId(l, "E2"), undefined);
  assert.strictEqual(byId(l, "E4"), undefined);
  // 隠した E の下の F は一番近い見える祖先（S1）につなぐ
  assert.ok(hasEdge(l, "S1", "F3", "tree"), JSON.stringify(l.edges));
}

function testHideF() {
  const l = sgTreeLayout({ status: sampleStatus() }, { hideF: true });
  assert.strictEqual(byId(l, "F1"), undefined);
  // A3 の下の F 2 つは A3 の sub に出る
  assert.ok(byId(l, "A3").sub.includes("F 2"), byId(l, "A3").sub);
  // E1 の下の F3 は E1 の sub に足される
  assert.ok(byId(l, "E1").sub.includes("F 1"), byId(l, "E1").sub);
}

function testCardContent() {
  const l = sgTreeLayout({ status: sampleStatus() }, {});
  // F は stance の文字を sub に出し、stance を札に持つ
  assert.strictEqual(byId(l, "F1").sub, "S");
  assert.strictEqual(byId(l, "F1").stance, "S");
  assert.strictEqual(byId(l, "F2").stance, "D");
  assert.strictEqual(byId(l, "F3").sub, "I");
  // A は ★ と F の数。Q は印を出さない（Objectivity・Logicality は無い）
  assert.ok(byId(l, "A1").sub.includes("★"));
  assert.ok(byId(l, "A3").sub.includes("F 2"), byId(l, "A3").sub);
  assert.strictEqual(byId(l, "Q0").sub, "");
  assert.strictEqual(byId(l, "E1").sub, "run-a");
  // PATH の外の A も淡くしない（on_path は無くなった）
  assert.ok(!("dim" in byId(l, "A2")));
  assert.strictEqual(byId(l, "A2").stance, "");
}

function testNoPending() {
  // 保留の印は無くなったので、札に pending の項目は持たせない
  const l = sgTreeLayout({ status: sampleStatus() }, {});
  for (const c of l.cards) assert.ok(!("pending" in c), c.id);
}

function testEmpty() {
  const l = sgTreeLayout({ status: { nodes: [] } }, {});
  assert.deepStrictEqual(l.cards, []);
}

function testEdgeStyle() {
  assert.strictEqual(sgEdgeStyle("tree").dash, "");
  assert.strictEqual(sgEdgeStyle("seed").dash, "2 4");
  assert.strictEqual(sgEdgeStyle("sprout").dash, "2 4");
  assert.strictEqual(sgEdgeStyle(null).dash, "");
}

function testEdgePath() {
  const a = { x: 0, y: 0, width: SG_CARD_W, height: SG_CARD_H };
  const b = { x: 0, y: 200, width: SG_CARD_W, height: SG_CARD_H };
  const d = sgEdgePath(a, b);
  assert.ok(d.startsWith("M "), d);
  assert.ok(d.includes("C "), d);
  // 縦に並ぶ 2 枚なので、上の札の下の真ん中から出る
  assert.ok(d.startsWith(`M ${SG_CARD_W / 2} ${SG_CARD_H} `), d);
}

function testTextScale() {
  // 拡大では文字を 1/z に、縮小と等倍では 1 のまま
  assert.strictEqual(sgTextScale(0.5), 1);
  assert.strictEqual(sgTextScale(1), 1);
  assert.strictEqual(sgTextScale(2), 0.5);
  assert.strictEqual(sgTextScale(2.5), 0.4);
  assert.strictEqual(sgTextScale(null), 1);
  // 画面の上の文字（13px × z × 倍率）は、どの拡大率でも 13px を超えない
  for (const z of [0.5, 1, 1.2, 1.44, 2, 2.5]) {
    assert.ok(13 * z * sgTextScale(z) <= 13 + 1e-9, `z=${z}`);
  }
}

function testTitleLines() {
  // 等倍で題が 6 行以上見える。拡大すると行が増え、どの拡大率でも 2 行を下回らない
  assert.ok(sgTitleLines(1) >= 6, String(sgTitleLines(1)));
  assert.ok(sgTitleLines(2.5) > sgTitleLines(1));
  assert.ok(sgTitleLines(2.5) >= sgTitleLines(2));
  assert.ok(sgTitleLines(0.3) >= 2);
  assert.ok(SG_CARD_H > SG_CARD_W, "Card は縦長");
}

// --- A の出し入れと畳み（U02）: Q1 → A2 → F3・F4、Q1 → A5 ---

function miniStatus() {
  return {
    nodes: [
      node("Q1", "Q", null, 0),
      node("A2", "A", "Q1", 1),
      node("F3", "F", "A2", 2, { stance: "support" }),
      node("F4", "F", "A2", 2, { stance: "issue" }),
      node("A5", "A", "Q1", 1),
    ],
  };
}

const idsOf = (l) => l.cards.map((c) => c.id);
const treeEdges = (l) => l.edges.filter((e) => e.kind === "tree").map((e) => `${e.from}→${e.to}`);

function testCollapsed() {
  const l = sgTreeLayout({ status: miniStatus() }, { collapsed: ["A2"] });
  assert.deepStrictEqual(idsOf(l).sort(), ["A2", "A5", "Q1", "SEED"].sort());
  assert.strictEqual(byId(l, "F3"), undefined);
  assert.strictEqual(byId(l, "F4"), undefined);
  assert.strictEqual(byId(l, "A2").collapsed, true);
  assert.ok(byId(l, "A2").sub.endsWith("▸ +2"), byId(l, "A2").sub);
  assert.strictEqual(byId(l, "A5").collapsed, false);
  assert.deepStrictEqual(treeEdges(l).sort(), ["Q1→A2", "Q1→A5"]);
  assert.ok(hasEdge(l, "SEED", "Q1", "seed"));
}

function testHideA() {
  const l = sgTreeLayout({ status: miniStatus() }, { hideA: true });
  assert.deepStrictEqual(idsOf(l).sort(), ["F3", "F4", "Q1", "SEED"].sort());
  // A を隠すとその下は A の父の Q につなぐ
  assert.deepStrictEqual(treeEdges(l).sort(), ["Q1→F3", "Q1→F4"]);
}

function testHideFAndA() {
  const l = sgTreeLayout({ status: miniStatus() }, { hideF: true, hideA: true });
  assert.deepStrictEqual(idsOf(l), ["Q1", "SEED"]);
  assert.ok(byId(l, "Q1").sub.includes("F 2"), byId(l, "Q1").sub);
  assert.deepStrictEqual(treeEdges(l), []);
}

function testCollapsedWithoutChildren() {
  // 子孫の無い札を畳みに入れても印はつかない
  const l = sgTreeLayout({ status: miniStatus() }, { collapsed: ["A5"] });
  assert.strictEqual(byId(l, "A5").collapsed, false);
  assert.strictEqual(byId(l, "A5").canCollapse, false);
  assert.ok(!byId(l, "A5").sub.includes("▸"), byId(l, "A5").sub);
  assert.deepStrictEqual(idsOf(l).sort(), ["A2", "A5", "F3", "F4", "Q1", "SEED"].sort());

  // 子孫のある Q1 を畳むと、他は子孫と印だけ
  const l2 = sgTreeLayout({ status: miniStatus() }, { collapsed: ["Q1"] });
  assert.deepStrictEqual(idsOf(l2), ["Q1", "SEED"]);
  assert.strictEqual(byId(l2, "Q1").collapsed, true);
  assert.ok(byId(l2, "Q1").sub.includes("▸ +4"), byId(l2, "Q1").sub);
}

function testCenterView() {
  const v = sgCenterView({ x: 100, y: 50, width: 200, height: 72 }, 800, 600, 2);
  assert.deepStrictEqual(v, { tx: 400 - 200 * 2, ty: 300 - 86 * 2, z: 2 });
  assert.strictEqual(v.tx, 0);
  assert.strictEqual(v.ty, 128);
}

// --- seed（D）の層: SEED の札の右に D を並べ、D の子の S をその真上に置く ---

function seedStatus() {
  return {
    seed: { type: { id: "problem", name: "PROBLEM_DRIVEN" }, mind: [] },
    nodes: [
      node("Q0", "Q", null, 0),
      node("Q1", "Q", "Q0", 1),
      node("A1", "A", "Q1", 2),
      node("D1", "D", null, 0, { text: "発作部位の特定" }),
      node("D2", "D", null, 0, { text: "予後" }),
      node("S1", "S", "D1", 1, { text: "最初の構想", paths: ["A1"] }),
      node("S2", "S", "D1", 1, { text: "二つ目", paths: ["A1"] }),
      node("S3", "S", "D2", 1, { text: "予後の構想", paths: ["A1"] }),
      node("S4", "S", null, 1, { text: "古い S", paths: ["A1"] }),
      node("E1", "E", "S1", 2),
    ],
  };
}

function testSeedLayer() {
  const l = sgTreeLayout({ status: seedStatus() }, {});
  // D は SEED の札と同じ高さ（y = 0）。SEED の右へ順に並ぶ
  assert.strictEqual(byId(l, "D1").kind, "D");
  assert.strictEqual(byId(l, "D1").y, 0);
  assert.strictEqual(byId(l, "D2").y, 0);
  assert.ok(byId(l, "SEED").x < byId(l, "D1").x && byId(l, "D1").x < byId(l, "D2").x, "SEED < D1 < D2");
  // D の子の S は D の真上（-STEP）、その下の E は -2×STEP
  assert.strictEqual(byId(l, "S1").y, -SG_TREE_STEP);
  assert.strictEqual(byId(l, "S3").y, -SG_TREE_STEP);
  assert.strictEqual(byId(l, "E1").y, -2 * SG_TREE_STEP);
  // 子の S の真ん中が D の真ん中（S1 と S2 の並び）
  const d1Mid = byId(l, "D1").x + SG_CARD_W / 2;
  const s12Mid = (byId(l, "S1").x + byId(l, "S2").x + SG_CARD_W) / 2;
  assert.strictEqual(s12Mid, d1Mid);
  // 札の sub は SEED と子の数。題は札の文
  assert.strictEqual(byId(l, "D1").sub, "SEED・SPROUT 2");
  assert.strictEqual(byId(l, "D2").sub, "SEED・SPROUT 1");
  assert.strictEqual(byId(l, "D1").text, "発作部位の特定");
  // 線: D → S は tree。SEED → D の線は無い（横に並ぶだけ）
  assert.ok(hasEdge(l, "D1", "S1", "tree"));
  assert.ok(hasEdge(l, "D1", "S2", "tree"));
  assert.ok(hasEdge(l, "D2", "S3", "tree"));
  assert.ok(!l.edges.some((e) => e.from === "SEED" && (e.to === "D1" || e.to === "D2")), JSON.stringify(l.edges));
  // 古い S（親が無い）は今のまま SEED の上。SEED → S の sprout の線は残る
  assert.strictEqual(byId(l, "S4").y, -SG_TREE_STEP);
  assert.ok(hasEdge(l, "SEED", "S4", "sprout"));
  // 札が重ならない（古い S・D・その子の S と E）
  const l2 = l.cards;
  for (let i = 0; i < l2.length; i++) {
    for (let j = i + 1; j < l2.length; j++) {
      assert.ok(!overlap(l2[i], l2[j]), `${l2[i].id} と ${l2[j].id}`);
    }
  }
}

function testSeedKeep() {
  // D1 を選ぶと、D1 と子の S（S1・S2）とその下の E1 が残る。D2 の下の S3 と古い S4 は残らない
  const keep = sgSproutKeep(seedStatus(), "D1");
  assert.deepStrictEqual([...keep].sort(), ["D1", "E1", "S1", "S2"]);
  assert.ok(!keep.has("S3") && !keep.has("S4") && !keep.has("SEED"));
  // D でも S でもない ID は null
  assert.strictEqual(sgSproutKeep(seedStatus(), "Q1"), null);
}

const tests = [
  ["4 layer の段", testLayers],
  ["線の種類", testEdgeKinds],
  ["選択の値", testSelection],
  ["ROOT は下へ・BRANCH は上へ", testChildDirection],
  ["札が重ならない", testNoOverlap],
  ["親は子の真ん中", testParentCentered],
  ["sgSproutKeep", testSproutKeep],
  ["hideE", testHideE],
  ["hideF", testHideF],
  ["stance と札の中身", testCardContent],
  ["保留の印は持たない", testNoPending],
  ["空の入力", testEmpty],
  ["線の書き方", testEdgeStyle],
  ["線の形", testEdgePath],
  ["文字の倍率と題の行数", testTextScale],
  ["題の行数と縦長の札", testTitleLines],
  ["畳んだ子孫", testCollapsed],
  ["hideA", testHideA],
  ["hideF と hideA", testHideFAndA],
  ["子孫の無い札の畳み", testCollapsedWithoutChildren],
  ["中央に出す tx と ty", testCenterView],
  ["seed（D）の層", testSeedLayer],
  ["seed を選んだときの残す札", testSeedKeep],
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
