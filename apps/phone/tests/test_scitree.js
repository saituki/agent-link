"use strict";
// 研究方針の画面（scitree.js）の純関数のテスト（計画 U18「変更内容 7」）。
// 実行:
//     node apps/phone/tests/test_scitree.js
//
// DOM が要る描画・操作はブラウザで確かめる。

const assert = require("assert");
const fs = require("fs");
const path = require("path");

const dir = path.join(__dirname, "..", "src", "sai_phone", "static");
const {
  stParseHash, stRows, stObjBar, stLogicLabel, stProgress, stNextLines,
  stFixHash, stDecisionRows, stOpsFor, ST_NODE_OPS, ST_LOGIC,
} = require(path.join(dir, "scitree.js"));

const tests = [];
const test = (fn) => tests.push(fn);

test(function testParseHash() {
  assert.deepStrictEqual(stParseHash("#/papers/tree"), {});
  for (const id of ["Q12", "O5", "F3", "E2", "S1"]) assert.deepStrictEqual(stParseHash(`#/papers/tree/${id}`), { id });
  for (const h of ["#/papers/tree/N12", "#/papers/tree/Q0", "#/papers/tree/Q12/x", "#/papers/other", "", null]) {
    assert.strictEqual(stParseHash(h), null, String(h));
  }
});

test(function testRows() {
  assert.deepStrictEqual(stRows(null), []);
  assert.deepStrictEqual(stRows({ nodes: [] }), []);
  const rows = stRows({ nodes: [
    { id: "Q1", kind: "Q", depth: 0, text: "a", on_path: true, objectivity: { count: 1, target: 2, met: false } },
    { id: "O2", kind: "O", depth: 1, text: "b", starred: true, answer_kind: "issue", on_path: false },
  ] });
  assert.deepStrictEqual(rows.map((r) => [r.id, r.depth, r.onPath, r.starred, r.answerKind]),
    [["Q1", 0, true, false, null], ["O2", 1, false, true, "issue"]]);
  assert.strictEqual(rows[0].obj.target, 2);
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
  const fix = { kind: "triage" };
  const next = [
    { kind: "card", node: "Q1", purpose: "調べる", why: "NO_EVIDENCE", card },
    { kind: "user", node: "Q2", purpose: "決める", why: "UNANSWERED", card: null },
    { kind: "debt", node: "Q3", purpose: "直す", why: "DEBT", fix },
    { kind: "card", node: "Q4", purpose: "余り", why: "X" },
  ];
  const lines = stNextLines({ next });
  assert.strictEqual(lines.length, 3);
  assert.deepStrictEqual(lines[0], { label: "調べる", why: "NO_EVIDENCE", node: "Q1", card, kind: "card", fix: null });
  assert.strictEqual(lines[2].fix, fix);
  assert.strictEqual(stNextLines({ next }, 4).length, 4);
  assert.deepStrictEqual(stNextLines(null), []);
});

test(function testFixHash() {
  global.paperHash = (p, id) => `#/paper/${p}/${id}`;
  assert.strictEqual(stFixHash({ kind: "triage" }, "p"), "#/papers/triage");
  assert.strictEqual(stFixHash({ kind: "set", set_id: 7 }, "p"), "#/papers/sets/7");
  assert.strictEqual(stFixHash({ kind: "node", node: "Q3" }, "p"), "#/papers/tree/Q3");
  assert.strictEqual(stFixHash({ kind: "run", run_id: 9 }, "p"), "#/run/p/9");
  assert.strictEqual(stFixHash({ kind: "paper", paper_id: 4 }, "p"), "#/paper/p/4");
  assert.strictEqual(stFixHash({ kind: "card", card: {} }, "p"), null);
  assert.strictEqual(stFixHash(null, "p"), null);
  delete global.paperHash;
  assert.strictEqual(stFixHash({ kind: "paper", paper_id: 4 }, "p"), null);
});

test(function testDecisionRows() {
  const rows = stDecisionRows([{ at: "2026-01-01", n: 1 }, { at: "2026-03-01", n: 2 }, { at: "2026-01-01", n: 3 }]);
  assert.deepStrictEqual(rows.map((r) => r.n), [2, 1, 3]);
  assert.deepStrictEqual(stDecisionRows(null), []);
});

test(function testNodeOps() {
  assert.deepStrictEqual(Object.keys(ST_NODE_OPS), ["Q", "O", "F", "E"]);
  assert.deepStrictEqual(ST_NODE_OPS.F, ["edit", "remove"]);
  // 一番上の Q は remove を出さない。子の Q を持つ Q は add_o を出さない
  assert.ok(!stOpsFor({ kind: "Q", parent: null }, []).includes("remove"));
  assert.ok(stOpsFor({ kind: "Q", parent: "O1" }, []).includes("remove"));
  assert.ok(!stOpsFor({ kind: "Q", parent: "O1" }, [{ kind: "Q" }]).includes("add_o"));
  // add_e は Issue の O と E だけ。★ なら unstar
  assert.ok(!stOpsFor({ kind: "O", answer_kind: "answer" }, []).includes("add_e"));
  assert.ok(stOpsFor({ kind: "O", answer_kind: "issue" }, []).includes("add_e"));
  assert.ok(stOpsFor({ kind: "E" }, []).includes("add_e"));
  assert.ok(stOpsFor({ kind: "O", starred: true }, []).includes("unstar"));
  assert.ok(stOpsFor({ kind: "O", starred: false }, []).includes("star"));
});

test(function testListIsFullWidthOnNarrow() {
  // 一覧 320px・板 420px の固定は 900px 以上だけ（iPhone 390px で右に隙間を空けない）
  const css = fs.readFileSync(path.join(dir, "scitree.css"), "utf8");
  for (const decl of [".sci-body .sci-list {\n    flex: 0 0 320px;", ".sci-body .sci-board {\n    flex: 0 0 420px;"]) {
    const at = css.indexOf(decl);
    assert.ok(at > 0, `${decl} が無い`);
    const before = css.slice(0, at);
    assert.ok(before.lastIndexOf("@media (min-width: 900px)") > before.lastIndexOf("\n}"), `${decl} が media query の外にある`);
  }
});

let failed = 0;
for (const fn of tests) {
  try { fn(); } catch (e) { failed++; console.error(`NG ${fn.name}:`, e.message); }
}
if (failed) process.exit(1);
console.log(`test_scitree: ${tests.length} 件すべて通りました`);
