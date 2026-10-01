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
  sgTreeLayout,
  sgSelectionOf,
  sgEdgeStyle,
  sgGateKeys,
  sgStateOf,
  sgEdgePath,
  sgRunsLayout,
  sgRunLayer,
  sgRunLayerLabel,
  sgRunStateOf,
  sgRunTimeLabel,
  sgRunEdgeStyle,
  SG_CARD_W,
  SG_CARD_H,
  SG_EDGE_KINDS,
  SG_RUN_LAYERS,
  SG_RUN_EDGE_KINDS,
  SG_RUN_STATES,
  SG_RUN_LAYOUT_KINDS,
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
    type: { id: "problem", name: "PROBLEM_DRIVEN" },
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

// --- 実行の図（C-02 の 3 層の DAG） ---

/** C-02 の node 1 枚。 */
function runNode(runId, layer, extra) {
  return {
    run_id: runId,
    block: `sci-${layer}`,
    name: `🌱 ${layer}-${runId}`,
    layer,
    status: "completed",
    created_at: "2026-09-30 01:02:03",
    finished_at: null,
    parent_run_id: null,
    description: "",
    sci_node: null,
    global_round: null,
    round: null,
    ...(extra || {}),
  };
}

/** C-02 の edge 1 本。 */
function runEdge(from, to, kind) {
  return { from, to, kind };
}

/** AC1 の例（root 2 → seed 1 → branch 3 → 次の seed 1。task_output 3 本）。 */
function sampleRtree() {
  return {
    nodes: [
      runNode(100, "root"),
      runNode(101, "root"),
      runNode(110, "seed"),
      runNode(120, "branch"),
      runNode(121, "branch"),
      runNode(122, "branch"),
      runNode(130, "seed"),
    ],
    edges: [
      runEdge(100, 101, "waggle_token"),
      runEdge(101, 110, "waggle_token"),
      runEdge(110, 120, "delegation"),
      runEdge(110, 121, "delegation"),
      runEdge(110, 122, "delegation"),
      runEdge(122, 130, "waggle_token"),
      // 返却の線（delegation の逆向き）。配置に使わないので 3 本とも無視される
      runEdge(120, 110, "task_output"),
      runEdge(121, 110, "task_output"),
      runEdge(122, 110, "task_output"),
    ],
  };
}

function colOf(layout, runId) {
  const card = layout.cards.find((c) => String(c.run_id) === String(runId));
  assert.ok(card, `run ${runId} の札がありません`);
  return card.col;
}

function testRunsLayoutColumns() {
  const layout = sgRunsLayout(sampleRtree());
  // 列は深さ（その run までの最長の道）。root 0,1・seed 2・branch 3・次の seed 4
  assert.strictEqual(colOf(layout, 100), 0);
  assert.strictEqual(colOf(layout, 101), 1);
  assert.strictEqual(colOf(layout, 110), 2);
  assert.strictEqual(colOf(layout, 120), 3);
  assert.strictEqual(colOf(layout, 121), 3);
  assert.strictEqual(colOf(layout, 122), 3);
  assert.strictEqual(colOf(layout, 130), 4);
  // 札の大きさは根の図にそろえる
  for (const c of layout.cards) {
    assert.strictEqual(c.w, SG_CARD_W);
    assert.strictEqual(c.h, SG_CARD_H);
  }
}

function testRunsLayoutBands() {
  const layout = sgRunsLayout(sampleRtree());
  // 帯は札のある層だけ、上から branch / sprout / seed / root の順
  assert.deepStrictEqual(layout.bands.map((b) => b.layer), ["branch", "seed", "root"]);
  // 帯は上から順に積まれている（重ならない）
  for (let i = 1; i < layout.bands.length; i += 1) {
    assert.ok(layout.bands[i].y >= layout.bands[i - 1].y + layout.bands[i - 1].h, "帯が重っています");
  }
  // 札は自分の層の帯の中に入る
  for (const c of layout.cards) {
    const band = layout.bands.find((b) => b.layer === c.layer);
    assert.ok(c.y >= band.y && c.y + c.h <= band.y + band.h, `札が帯の外です: ${JSON.stringify(c)}`);
  }
  // 同じ層の同じ列の 3 つの branch は行が 0,1,2
  const branch = layout.cards.filter((c) => c.layer === "branch").sort((a, b) => a.row - b.row);
  assert.deepStrictEqual(branch.map((c) => [c.run_id, c.row]), [[120, 0], [121, 1], [122, 2]]);
  // 図の大きさは札和外側 Community に入る大きさ
  assert.ok(layout.width > 0 && layout.height > 0);
  for (const c of layout.cards) {
    assert.ok(c.x + c.w <= layout.width, "札が幅の外に出ています");
    assert.ok(c.y + c.h <= layout.height, "札が高さの外に出ています");
  }
}

function testRunsLayoutTaskOutputIgnored() {
  // task_output が 3 本あっても止まらない（配置に使わない）
  const layout = sgRunsLayout(sampleRtree());
  assert.strictEqual(layout.cards.length, 7);
  // task_output を除いても列は変わらない（逆向きの線なので）
  const dag = sampleRtree();
  const without = sgRunsLayout({ nodes: dag.nodes, edges: dag.edges.filter((e) => e.kind !== "task_output") });
  assert.deepStrictEqual(layout.cards, without.cards);
  // 同じ 2 つを結ぶ返却の線を 1 本増やしても場所は変わらない
  const more = { nodes: dag.nodes, edges: dag.edges.concat([runEdge(100, 120, "task_output")]) };
  assert.deepStrictEqual(sgRunsLayout(more).cards, layout.cards);
}

function testRunsLayoutCycles() {
  // 輪があっても止まらない（訪問済みの印で切る）
  const dag = {
    nodes: [runNode(1, "root"), runNode(2, "seed"), runNode(3, "branch")],
    edges: [runEdge(1, 2, "waggle_token"), runEdge(2, 3, "delegation"), runEdge(3, 1, "waggle_token")],
  };
  const layout = sgRunsLayout(dag);
  assert.strictEqual(layout.cards.length, 3);
  // 輪：同じ node を 2 度踏むとそこで剪断するので、1 → 2 → 3 → 1 の列は 3・1・2
  // （実際に長い道だけを数えているので、輪の長さ + 1 になる）
  assert.deepStrictEqual(layout.cards.map((c) => c.col), [3, 1, 2]);
  // 同じ輪でも 2 回呼んでも同じ（止まらない・同じ場所）
  assert.deepStrictEqual(sgRunsLayout(dag), layout);
  // 2 つの node が互いを指すだけの輪
  const two = {
    nodes: [runNode(1, "root"), runNode(2, "seed")],
    edges: [runEdge(1, 2, "waggle_token"), runEdge(2, 1, "waggle_token")],
  };
  assert.deepStrictEqual(sgRunsLayout(two).cards.map((c) => c.col), [2, 1]);
}


function testRunsLayoutEdgeCases() {
  // 0 件・壊れた JSON でも止まらない
  assert.deepStrictEqual(sgRunsLayout(null).cards, []);
  assert.deepStrictEqual(sgRunsLayout({}).cards, []);
  assert.deepStrictEqual(sgRunsLayout({ nodes: [null, {}], edges: [null, {}] }).cards, []);
  assert.deepStrictEqual(sgRunsLayout(null).bands, []);
  // 両端が node に無い線は使わない
  const layout = sgRunsLayout({
    nodes: [runNode(1, "root"), runNode(2, "seed")],
    edges: [runEdge(1, 999, "waggle_token"), runEdge(888, 2, "waggle_token")],
  });
  assert.deepStrictEqual(layout.cards.map((c) => c.col), [0, 0]);
  // 1 層だけなら帯も 1 本
  assert.deepStrictEqual(sgRunsLayout({ nodes: [runNode(1, "branch")], edges: [] }).bands.map((b) => b.layer), ["branch"]);
  // 同じ入力を 2 回呼んでも同じ位置（開くたびに図が動かない）
  const a = sgRunsLayout(sampleRtree());
  const b = sgRunsLayout(sampleRtree());
  assert.deepStrictEqual(a, b);
}

function testRunsLayoutUsesOnlyTwoEdgeKinds() {
  // 配置に使うのは delegation と waggle_token だけ
  assert.deepStrictEqual(SG_RUN_LAYOUT_KINDS, ["delegation", "waggle_token"]);
  assert.deepStrictEqual(
    SG_RUN_EDGE_KINDS.map((k) => k[0]),
    ["delegation", "waggle_token", "task_output"]
  );
  // 委譲は実線、結びは点線、返却は破線
  assert.strictEqual(sgRunEdgeStyle("delegation").dash, "");
  assert.strictEqual(sgRunEdgeStyle("waggle_token").dash, "2 4");
  assert.strictEqual(sgRunEdgeStyle("task_output").dash, "8 5");
  // 色も 3 つ違う（見分けられる）
  const colors = SG_RUN_EDGE_KINDS.map((k) => sgRunEdgeStyle(k[0]).colorVar);
  assert.strictEqual(new Set(colors).size, 3);
  // 表に無い種類でも止まらない
  assert.strictEqual(sgRunEdgeStyle("???").dash, "");
  assert.strictEqual(sgRunEdgeStyle(null).colorVar, "var(--sci-edge-parent)");
}

function testRunLayerAndState() {
  // 層は 4 つ。帯の語はその表示名
  assert.deepStrictEqual(SG_RUN_LAYERS.map((x) => x[0]), ["branch", "sprout", "seed", "root"]);
  assert.strictEqual(sgRunLayerLabel("root"), "方針（root）");
  assert.strictEqual(sgRunLayerLabel("seed"), "構想（seed）");
  assert.strictEqual(sgRunLayerLabel("branch"), "実行（branch）");
  // 知らない層は「その他」の帯（図が止まらない）
  assert.strictEqual(sgRunLayer("???"), "other");
  assert.strictEqual(sgRunLayer(undefined), "other");
  assert.strictEqual(sgRunLayerLabel("???"), "その他");
  // 実行の状態は日本語
  assert.strictEqual(sgRunStateOf("running").label, "実行中");
  assert.strictEqual(sgRunStateOf("completed").label, "完了");
  assert.strictEqual(sgRunStateOf("failed").label, "失敗");
  assert.strictEqual(sgRunStateOf("cancelled").label, "中止");
  // 知らない状態でも止まらない
  assert.strictEqual(sgRunStateOf("???").label, "待ち");
  assert.strictEqual(sgRunStateOf(null).label, "待ち");
  // 根の図の札と同じ CSS 変数を使う
  assert.ok(Object.values(SG_RUN_STATES).every((s) => /^var\(--sci-/.test(s.color)));
}

function testRunTimeLabel() {
  // 長い形から minute だけ出す
  assert.strictEqual(sgRunTimeLabel("2026-09-30 01:02:03"), "09-30 01:02");
  assert.strictEqual(sgRunTimeLabel("2026-09-30T01:02:03Z"), "09-30 01:02");
  // 読めない形はそのまま切り詰める
  assert.strictEqual(sgRunTimeLabel(""), "");
  assert.strictEqual(sgRunTimeLabel(null), "");
  assert.strictEqual(sgRunTimeLabel("2026/09/30"), "2026/09/30");
  assert.strictEqual(sgRunTimeLabel("2026-09-30 01:02:03.123456+09:00").length, 11);
}


// --- 根の図の樹（sgTreeLayout） ---

function treeInput(extra) {
  return {
    status: sampleStatus(),
    papers: [
      { node: "N12", paper: 37, title: "t", year: 2014, role: "support", read_state: "read", findings: 3 },
      { node: "N20", paper: 38, title: "u", year: 2020, role: "support", read_state: "unread", findings: 0 },
    ],
    dag: { nodes: [], edges: [] },
    metrics: {},
    ...(extra || {}),
  };
}

function treeDag() {
  return {
    nodes: [
      runNode(1, "root"),
      runNode(2, "seed"),
      runNode(10, "sprout", { block: "sci-exp-sprout" }),
      runNode(20, "branch", { block: "sci-exp" }),
      runNode(21, "branch", { block: "sci-exp-review" }),
      runNode(22, "branch", { block: "sci-exp" }),
      runNode(11, "seed", { block: "sci-exp-seed" }),
    ],
    edges: [runEdge(10, 20, "delegation"), runEdge(20, 21, "delegation"), runEdge(21, 22, "waggle_token")],
  };
}

const byKey = (l, k) => l.cards.find((c) => c.key === k);

function testTreeVertical() {
  const l = sgTreeLayout(treeInput(), { showOptions: true });
  assert.strictEqual(byKey(l, "seed").y, 0);
  assert.ok(byKey(l, "N9").y > 0);
  assert.ok(byKey(l, "N12").y > byKey(l, "N9").y);
  assert.ok(byKey(l, "N20").y > byKey(l, "N12").y);
  assert.strictEqual(byKey(l, "P:N12:37").y, byKey(l, "N12").y + 120);
  // 幹の中心は x = 0
  assert.strictEqual(byKey(l, "seed").x + byKey(l, "seed").width / 2, 0);
}

function testTreeUp() {
  const l = sgTreeLayout(treeInput({ dag: treeDag() }), { showOptions: true });
  assert.ok(byKey(l, "R10").y < 0);
  assert.ok(byKey(l, "R20").y < byKey(l, "R10").y);
  // review は描かず、branch → review → branch が親子になる
  assert.strictEqual(byKey(l, "R21"), undefined);
  assert.ok(l.edges.some((e) => e.from === "R20" && e.to === "R22" && e.kind === "parent"));
  assert.ok(byKey(l, "R22").y < byKey(l, "R20").y);
  // 旧い sci-exp-seed は sprout、layer root / seed は描かない
  assert.strictEqual(byKey(l, "R11").kind, "sprout");
  assert.strictEqual(byKey(l, "R1"), undefined);
  assert.strictEqual(byKey(l, "R2"), undefined);
}

function testTreeNoOverlapAndOptions() {
  const l = sgTreeLayout(treeInput({ dag: treeDag() }), { showOptions: true });
  for (const a of l.cards) {
    for (const b of l.cards) {
      if (a === b || a.y !== b.y) continue;
      assert.ok(a.x + a.width <= b.x || b.x + b.width <= a.x, `${a.key} と ${b.key} が重なっています`);
    }
  }
  const h = sgTreeLayout(treeInput(), { showOptions: false });
  assert.ok(!h.cards.some((c) => c.data && c.data.kind === "option"));
  assert.ok(!h.cards.some((c) => c.key === "P:N20:38"));
}

function testTreeBandsAndSelection() {
  const l = sgTreeLayout(treeInput({ dag: treeDag() }), { showOptions: true });
  assert.deepStrictEqual(l.bands.map((b) => b.layer), ["branch", "sprout", "seed", "root"]);
  assert.deepStrictEqual(sgSelectionOf(byKey(l, "seed")), { kind: "seed" });
  assert.deepStrictEqual(sgSelectionOf(byKey(l, "N12")), { kind: "node", id: "N12" });
  assert.deepStrictEqual(sgSelectionOf(byKey(l, "P:N12:37")), { kind: "paper", node: "N12", paper: 37 });
  assert.deepStrictEqual(sgSelectionOf(byKey(l, "R20")), { kind: "run", id: 20 });
  assert.deepStrictEqual(sgTreeLayout(null).bands.map((b) => b.layer), ["seed"]);
}

// --- 実行 ---

const tests = [
  ["樹の縦の並び", testTreeVertical],
  ["樹の上（sprout・branch・review）", testTreeUp],
  ["樹の札が重ならない・OPTION", testTreeNoOverlapAndOptions],
  ["樹の帯と選択", testTreeBandsAndSelection],
  ["線の種類", testEdgeStyle],
  ["線のパス", testEdgePath],
  ["札の状態", testStateOf],
  ["関門の条件の key", testGateKeys],
  ["実行の図の列（深さ）", testRunsLayoutColumns],
  ["実行の図の帯と行", testRunsLayoutBands],
  ["実行の図は task_output を使わない", testRunsLayoutTaskOutputIgnored],
  ["実行の図は輪でも止まらない", testRunsLayoutCycles],
  ["実行の図の配置（空・壊れた入力）", testRunsLayoutEdgeCases],
  ["実行の図の線（delegation と waggle_token）", testRunsLayoutUsesOnlyTwoEdgeKinds],
  ["実行の図の層と状態", testRunLayerAndState],
  ["実行の図の時刻", testRunTimeLabel],
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
