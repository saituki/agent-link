"use strict";
// research-tree の画面（scitree.js）の純関数のテスト（計画 U18「変更内容 7」）。
// 実行:
//     node apps/web/tests/test_scitree.js
//
// DOM が要る描画・操作はブラウザで確かめる。

const assert = require("assert");
const fs = require("fs");
const path = require("path");

const dir = path.join(__dirname, "..", "src", "sai_web", "static");
const {
  stParseHash, stLegacyHash, stRows, stObjBar, stLogicLabel, stProgress, stNextLines,
  stDecisionRows, stOpsFor, ST_NODE_OPS, ST_LOGIC,
  stVisibleRows, stActionMeta, stClampSplit, ST_ALL_KINDS, ST_SPLIT_LIMITS,
  stFmtAt, stHistoryText,
} = require(path.join(dir, "scitree.js"));

const tests = [];
const test = (fn) => tests.push(fn);

test(function testParseHash() {
  assert.deepStrictEqual(stParseHash("#/research/tree"), {});
  // SEED も S1 も受ける（SEED は札の特別な ID）
  assert.deepStrictEqual(stParseHash("#/research/tree/SEED"), { id: "SEED" });
  for (const id of ["Q12", "O5", "F3", "E2", "S1"]) assert.deepStrictEqual(stParseHash(`#/research/tree/${id}`), { id });
  for (const h of ["#/research/tree/N12", "#/research/tree/Q0", "#/research/tree/S0", "#/research/tree/Q12/x", "#/research/other", "", null]) {
    assert.strictEqual(stParseHash(h), null, String(h));
  }
});

test(function testLegacyHash() {
  // 古い住所 6 種 → 新しい住所（? の後ろと ID はそのまま写す）
  assert.strictEqual(stLegacyHash("#/papers/tree"), "#/research/tree");
  assert.strictEqual(stLegacyHash("#/papers/tree/Q1"), "#/research/tree/Q1");
  assert.strictEqual(stLegacyHash("#/papers/triage"), "#/research/tree");
  assert.strictEqual(stLegacyHash("#/papers/read/12"), "#/research/read/12");
  assert.strictEqual(stLegacyHash("#/mlflow/e/all?sai_run=5"), "#/research/experiments/e/all?sai_run=5");
  assert.strictEqual(stLegacyHash("#/mlflow"), "#/research/experiments");
  assert.strictEqual(stLegacyHash("#/papers/canvas"), "#/research/tree");
  assert.strictEqual(stLegacyHash("#/canvas/Q1"), "#/research/tree");
  // 関係ない住所は null
  for (const h of ["#/research/tree", "#/papers/dashboard", "#/papers/sets/7", "#/home", "", null]) {
    assert.strictEqual(stLegacyHash(h), null, String(h));
  }
});

test(function testRows() {
  assert.deepStrictEqual(stRows(null), []);
  assert.deepStrictEqual(stRows({ nodes: [] }), []);
  const rows = stRows({ nodes: [
    { id: "Q1", kind: "Q", depth: 0, text: "a", on_path: true, pending: 2, bottom_q: true, objectivity: { count: 1, target: 2, met: false } },
    { id: "O2", kind: "O", depth: 1, text: "b", starred: true, answer_kind: "issue", on_path: false },
  ] });
  assert.deepStrictEqual(rows.map((r) => [r.id, r.depth, r.onPath, r.starred, r.answerKind]),
    [["Q1", 0, true, false, null], ["O2", 1, false, true, "issue"]]);
  assert.strictEqual(rows[0].obj.target, 2);
  assert.strictEqual(rows[0].pending, 2);
  assert.strictEqual(rows[1].pending, 0);
  // leaf は bottom_q に改名した
  assert.strictEqual(rows[0].bottom, true);
  assert.strictEqual(rows[1].bottom, false);
});

test(function testObjBar() {
  assert.strictEqual(stObjBar({ count: 2, target: 3 }), "■■□ 2/3");
  assert.strictEqual(stObjBar({ count: 0, target: 2 }), "□□ 0/2");
  assert.strictEqual(stObjBar({ count: 12, target: 10 }), "■■■■■■■■■■ 12/10");
  assert.strictEqual(stObjBar(null), "");
  assert.strictEqual(stObjBar({ count: 10, target: 20 }), "■■■■■□□□□□ 10/20");
});

test(function testLogicLabel() {
  assert.deepStrictEqual(stLogicLabel({ value: "ok" }), ["OK", "ok"]);
  assert.deepStrictEqual(stLogicLabel({ value: "contradiction" }), ["CONTRADICTION", "warn"]);
  assert.deepStrictEqual(stLogicLabel(null), ["UNCHECKED", "dim"]);
  assert.deepStrictEqual(Object.keys(ST_LOGIC), ["ok", "contradiction", "unchecked"]);
});

test(function testProgress() {
  assert.deepStrictEqual(stProgress({ progress: { answered: 12, total: 24 } }), { answered: 12, total: 24, ratio: 0.5 });
  assert.deepStrictEqual(stProgress({ progress: { answered: 0, total: 0 } }), { answered: 0, total: 0, ratio: 0 });
  assert.strictEqual(stProgress(null).ratio, 0);
});

test(function testNextLines() {
  const card = { node: "Q1" };
  const next = [
    { kind: "card", node: "Q1", purpose: "調べる", why: "NO_EVIDENCE", card },
    { kind: "user", node: "Q2", purpose: "決める", why: "UNANSWERED", card: null },
    { kind: "card", node: "Q4", purpose: "余り", why: "X" },
  ];
  const lines = stNextLines({ next });
  assert.strictEqual(lines.length, 3);
  assert.deepStrictEqual(lines[0], { label: "調べる", why: "NO_EVIDENCE", node: "Q1", card, kind: "card" });
  assert.strictEqual(stNextLines({ next }, 2).length, 2);
  assert.deepStrictEqual(stNextLines(null), []);
});

test(function testDecisionRows() {
  const rows = stDecisionRows([{ at: "2026-01-01", n: 1 }, { at: "2026-03-01", n: 2 }, { at: "2026-01-01", n: 3 }]);
  assert.deepStrictEqual(rows.map((r) => r.n), [2, 1, 3]);
  assert.deepStrictEqual(stDecisionRows(null), []);
});

test(function testNodeOps() {
  assert.deepStrictEqual(Object.keys(ST_NODE_OPS), ["Q", "O", "F", "E", "S"]);
  assert.deepStrictEqual(ST_NODE_OPS, {
    Q: ["add_q", "add_o"],
    O: ["add_q", "add_f", "copy"],
    F: [],
    E: ["add_f", "add_e"],
    S: ["add_e", "edit", "remove"],
  });
  // F は何も出さない
  assert.deepStrictEqual(stOpsFor({ kind: "F", parent: "O1" }, []), []);
  // Q で子に O があれば add_q を出さない
  assert.ok(stOpsFor({ kind: "Q", parent: null }, []).includes("add_q"));
  assert.ok(!stOpsFor({ kind: "Q", parent: null }, [{ kind: "O" }]).includes("add_q"));
  // 子に Q があれば add_o を出さない
  assert.ok(stOpsFor({ kind: "Q", parent: null }, []).includes("add_o"));
  assert.ok(!stOpsFor({ kind: "Q", parent: null }, [{ kind: "Q" }]).includes("add_o"));
  // add_e は S（SPROUT）と E だけ。O からは消した
  assert.ok(!stOpsFor({ kind: "O", answer_kind: "answer" }, []).includes("add_e"));
  assert.ok(!stOpsFor({ kind: "O", answer_kind: "issue" }, []).includes("add_e"));
  assert.ok(stOpsFor({ kind: "S" }, []).includes("add_e"));
  assert.ok(stOpsFor({ kind: "E" }, []).includes("add_e"));
  assert.ok(!stOpsFor({ kind: "E" }, []).includes("add_q"));
  // O の add_q は常に出す
  assert.deepStrictEqual(stOpsFor({ kind: "O", answer_kind: "answer" }, []), ["add_q", "add_f", "copy"]);
  // S は BRANCH と Edit / Delete
  assert.deepStrictEqual(stOpsFor({ kind: "S" }, []), ["add_e", "edit", "remove"]);
});

test(function testFmtAt() {
  // 読めない文字はそのまま返す（地方時に依らない）
  assert.strictEqual(stFmtAt("読めない"), "読めない");
  assert.strictEqual(stFmtAt(""), "");
  assert.strictEqual(stFmtAt(null), "");
  assert.strictEqual(stFmtAt(undefined), "");
  // 読める時刻は YYYY-MM-DD HH:MM の形になる
  const v = stFmtAt("2026-10-02T00:00:00Z");
  assert.ok(/^\d{4}-\d{2}-\d{2} \d{2}:\d{2}$/.test(v), v);
});

test(function testHistoryText() {
  const at = "2026-10-02T00:00:00Z";
  const date = stFmtAt(at);
  assert.strictEqual(stHistoryText({ action: "init", by: "user", at }), `init ${date} human`);
  assert.strictEqual(
    stHistoryText({ action: "Add F", by: "run:210", at, run_id: 210, block: "sci-read" }),
    `Add F ${date} run 210 sci-read`,
  );
  assert.strictEqual(
    stHistoryText({ action: "Run", at, run_id: 215, block: "sci-read", global_round: 12, round: 2 }),
    `Run 215 sci-read R12.2 ${date}`,
  );
  assert.strictEqual(
    stHistoryText({ action: "Run", at, run_id: 216, block: "sci-ideate", global_round: null, round: null }),
    `Run 216 sci-ideate ${date}`,
  );
  assert.strictEqual(stHistoryText({ action: "edit", by: "user", at }), `edit ${date} human`);
  // block が空なら run の番号だけ
  assert.strictEqual(
    stHistoryText({ action: "Add O", by: "run:3", at, run_id: 3, block: "" }),
    `Add O ${date} run 3`,
  );
  // round が null なら R<global_round> だけ
  assert.strictEqual(
    stHistoryText({ action: "Run", at, run_id: 1, block: "sci-read", global_round: 4, round: null }),
    `Run 1 sci-read R4 ${date}`,
  );
});

// Q1 → O2 → F3 と Q1 → O4 の 4 行（並びは status.nodes と同じ順）
function treeRows() {
  return [
    { id: "Q1", kind: "Q", depth: 0, text: "問い", node: { id: "Q1", parent: null } },
    { id: "O2", kind: "O", depth: 1, text: "二つ目", node: { id: "O2", parent: "Q1" } },
    { id: "F3", kind: "F", depth: 2, text: "三つ目", node: { id: "F3", parent: "O2" } },
    { id: "O4", kind: "O", depth: 1, text: "四つ目", node: { id: "O4", parent: "Q1" } },
  ];
}

test(function testVisibleRowsFold() {
  const rows = treeRows();
  // 絞り込みも畳みも無ければ全部の行（ctx は全部 false）
  const all = stVisibleRows(rows, { folded: new Set(), word: "", kinds: new Set(ST_ALL_KINDS) });
  assert.deepStrictEqual(all.map((x) => [x.row.id, x.hasKids, x.ctx]), [
    ["Q1", true, false], ["O2", true, false], ["F3", false, false], ["O4", false, false],
  ]);
  // O2 を畳むと F3 だけが出なくなる
  const folded = stVisibleRows(rows, { folded: new Set(["O2"]) });
  assert.deepStrictEqual(folded.map((x) => x.row.id), ["Q1", "O2", "O4"]);
  assert.deepStrictEqual(folded.map((x) => x.hasKids), [true, true, false]);
  // 絞り込み中は畳みを無視する
  assert.deepStrictEqual(
    stVisibleRows(rows, { folded: new Set(["Q1", "O2"]), word: "F3" }).map((x) => x.row.id),
    ["Q1", "O2", "F3"],
  );
  // 絞り込みの無いときは全部出す（word も kinds も無ければ全部）
  assert.deepStrictEqual(stVisibleRows(rows, {}).map((x) => x.row.id), ["Q1", "O2", "F3", "O4"]);
  assert.deepStrictEqual(stVisibleRows([], {}), []);
  assert.deepStrictEqual(stVisibleRows(null, null), []);
});

test(function testVisibleRowsWord() {
  const out = stVisibleRows(treeRows(), { word: "F3" });
  assert.deepStrictEqual(out.map((x) => [x.row.id, x.ctx]), [["Q1", true], ["O2", true], ["F3", false]]);
  // 大文字・小文字を区別しない。ancestor も出す
  const ci = stVisibleRows(treeRows(), { word: "f3" });
  assert.deepStrictEqual(ci.map((x) => x.row.id), ["Q1", "O2", "F3"]);
  // どこにも無い文字なら、どの行も出さない
  assert.deepStrictEqual(stVisibleRows(treeRows(), { word: "存在しない" }), []);
});

test(function testVisibleRowsKinds() {
  // O だけ: F3 は出す必要が無い（O の子だが自身が種類で弾かれている）
  const out = stVisibleRows(treeRows(), { kinds: new Set(["O"]) });
  assert.deepStrictEqual(out.map((x) => [x.row.id, x.ctx]), [["Q1", true], ["O2", false], ["O4", false]]);
  // 種類と文字は両方の条件（両方満たす行だけ出错）
  const both = stVisibleRows(treeRows(), { word: "四", kinds: new Set(["O"]) });
  assert.deepStrictEqual(both.map((x) => x.row.id), ["Q1", "O4"]);
  // 全部切ると何も出ない
  assert.deepStrictEqual(stVisibleRows(treeRows(), { kinds: new Set() }), []);
});

test(function testActionMeta() {
  assert.strictEqual(stActionMeta({ run_id: 5, block: "sci-read", global_round: 2, owner: "R2", node: "O12" }), "run 5 sci-read R2 O12");
  assert.strictEqual(stActionMeta({ run_id: 7, block: "sci-search", global_round: null, owner: "review", node: null }), "run 7 sci-search review");
  // 空の部分は詰めて飛ばす
  assert.strictEqual(stActionMeta({ run_id: 3, block: "", owner: "", node: null, global_round: null }), "run 3");
  assert.strictEqual(stActionMeta(null), "");
  assert.strictEqual(stActionMeta({}), "");
});

test(function testClampSplit() {
  assert.deepStrictEqual(ST_SPLIT_LIMITS, { list: [200, 600], board: [280, 720] });
  // 範囲の中は何も変えない。下限と上限
  assert.strictEqual(stClampSplit("list", 320, 1280, 420), 320);
  assert.strictEqual(stClampSplit("list", 10, 3000, 420), 200);
  assert.strictEqual(stClampSplit("list", 9999, 3000, 420), 600);
  assert.strictEqual(stClampSplit("board", 10, 3000, 320), 280);
  assert.strictEqual(stClampSplit("board", 9999, 3000, 320), 720);
  // 図（1280 − 一覧 − 板 − 12）が 300 を割らないように抑える
  assert.strictEqual(stClampSplit("list", 900, 1280, 720), 248);   // 1280-720-12-300
  assert.strictEqual(stClampSplit("board", 900, 1280, 600), 368); // 1280-600-12-300
  // 一覧 600px を押しても、図 300px を残すために 248px までしか広げない
  assert.strictEqual(stClampSplit("list", 600, 1280, 720), 248);
  // 図 300px の余裕が無いときは下限を保つ
  assert.strictEqual(stClampSplit("list", 500, 700, 320), 200);
  // 小数も四捨五入で整数になる
  assert.strictEqual(stClampSplit("list", 333.4, 1280, 420), 333);
  assert.strictEqual(stClampSplit("list", null, 1280, 420), 200);
});

test(function testListIsFullWidthOnNarrow() {
  // 一覧・板の幅は var() で 900px 以上だけ（iPhone 390px で右に隙間を空けない）
  const css = fs.readFileSync(path.join(dir, "scitree.css"), "utf8");
  for (const decl of [".sci-body .sci-list {\n    flex: 0 0 var(--sci-list-w, 320px);", ".sci-body .sci-board {\n    flex: 0 0 var(--sci-board-w, 420px);"]) {
    const at = css.indexOf(decl);
    assert.ok(at > 0, `${decl} が無い`);
    const before = css.slice(0, at);
    assert.ok(before.lastIndexOf("@media (min-width: 900px)") > before.lastIndexOf("\n}"), `${decl} が media query の外にある`);
  }
  // 狭い画面では境界線を出さない
  assert.ok(/@media \(max-width: 899px\)[\s\S]*?\.sci-split-handle \{ display: none; \}/.test(css), "狭い画面で境界線を隠す宣言が無い");
  // 板を出していないときは板の境界線も隠す
  assert.ok(/@media \(min-width: 900px\)[\s\S]*?\.sci-body:not\(\.has-board\) \.sci-split-handle\[data-split="board"\] \{ display: none; \}/.test(css),
    "板が無いときの境界線を隠す宣言が無い");
});

let failed = 0;
for (const fn of tests) {
  try { fn(); } catch (e) { failed++; console.error(`NG ${fn.name}:`, e.message); }
}
if (failed) process.exit(1);
console.log(`test_scitree: ${tests.length} 件すべて通りました`);
