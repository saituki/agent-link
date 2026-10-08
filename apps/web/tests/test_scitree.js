"use strict";
// research-tree の画面（scitree.js）の純関数のテスト（計画 U18「変更内容 7」・U06 で Q・A・F・E・S に合わせた）。
// 実行:
//     node apps/web/tests/test_scitree.js
//
// DOM が要る描画・操作はブラウザで確かめる。

const assert = require("assert");
const fs = require("fs");
const path = require("path");

const dir = path.join(__dirname, "..", "src", "sai_web", "static");
const {
  stParseHash, stLegacyHash, stRows, stNextLines, stNextText,
  stOpsFor, ST_NODE_OPS, ST_STANCE, ST_EXPERIMENTS_HASH,
  stVisibleRows, stRowHit, stActionMeta, stClampSplit, ST_ALL_KINDS, ST_SPLIT_LIMITS,
  stFmtAt, stHistoryText, stRangeText, stApprovalText, stStanceLabel,
  stSproutToggle, stSproutRows, stSproutSummary, ST_SPROUT_PATH_MAX,
} = require(path.join(dir, "scitree.js"));

const tests = [];
const test = (fn) => tests.push(fn);

test(function testParseHash() {
  assert.deepStrictEqual(stParseHash("#/research/tree"), {});
  // SEED も S1 も受ける（SEED は札の特別な ID）
  assert.deepStrictEqual(stParseHash("#/research/tree/SEED"), { id: "SEED" });
  for (const id of ["Q12", "A5", "F3", "E2", "S1"]) assert.deepStrictEqual(stParseHash(`#/research/tree/${id}`), { id });
  // O は無くなった ID なので受けない
  for (const h of ["#/research/tree/O5", "#/research/tree/N12", "#/research/tree/Q0", "#/research/tree/S0", "#/research/tree/Q12/x", "#/research/other", "", null]) {
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
    { id: "Q1", kind: "Q", depth: 0, text: "a", starred: false },
    { id: "A2", kind: "A", depth: 1, text: "b", starred: true },
    { id: "F3", kind: "F", depth: 2, text: "c", stance: "deny" },
  ] });
  assert.deepStrictEqual(rows.map((r) => [r.id, r.depth, r.starred, r.stance]),
    [["Q1", 0, false, null], ["A2", 1, true, null], ["F3", 2, false, "deny"]]);
  // 一覧の行に保留・目安・論理・配置の項目は持たせない
  assert.ok(!("pending" in rows[0]) && !("obj" in rows[0]) && !("logic" in rows[0]) && !("onPath" in rows[0]));
});

test(function testStance() {
  // 送る値は API の STANCES（support・deny・issue）。印は表示だけ
  assert.deepStrictEqual(ST_STANCE, [["support", "S", "支持"], ["deny", "D", "反対"], ["issue", "I", "問い"]]);
  assert.deepStrictEqual(stStanceLabel("support"), ["S", "支持"]);
  assert.deepStrictEqual(stStanceLabel("issue"), ["I", "問い"]);
  // 略号や旧い値は送る値として通さない
  for (const v of [null, undefined, "", "S", "s", "X", "constructor"]) assert.strictEqual(stStanceLabel(v), null, String(v));
});

test(function testNextLines() {
  const card = { node: "Q1" };
  const next = [
    { kind: "card", node: "Q1", purpose: "調べる", why: "NO_FINDING", card },
    { kind: "user", node: "Q2", purpose: "決める", why: "UNVERIFIED", card: null },
    { kind: "card", node: "Q4", purpose: "余り", why: "X" },
  ];
  const lines = stNextLines({ next });
  assert.strictEqual(lines.length, 3);
  assert.deepStrictEqual(lines[0], { label: "調べる", why: "NO_FINDING", node: "Q1", block: "", card, kind: "card" });
  assert.strictEqual(stNextLines({ next }, 2).length, 2);
  assert.deepStrictEqual(stNextLines(null), []);
});

test(function testNextText() {
  const status = { nodes: [{ id: "Q3", kind: "Q", text: "この研究が目指している大きな目的は何か。" }] };
  // Q あり: node と block と文
  assert.strictEqual(
    stNextText({ why: "NO_CHILD", node: "Q3", block: "lab-grow", label: "「…」の子を作る" }, status),
    "[NO_CHILD] Q3 lab-grow「この研究が目指している大きな目的は何か。」",
  );
  // block が空なら node だけ
  assert.strictEqual(stNextText({ why: "X", node: "Q3", block: "" }, status), "[X] Q3「この研究が目指している大きな目的は何か。」");
  // Node が見つからない（SEED など）: 今までの label
  assert.strictEqual(stNextText({ why: "X", node: "SEED", block: "sci-seed", label: "top_Q を作る" }, status), "[X] top_Q を作る");
  assert.strictEqual(stNextText({ why: "X", node: "Q9", label: "ラベル" }, null), "[X] ラベル");
  // why が空: [...] を付けない
  assert.strictEqual(stNextText({ why: "", node: "Q3", block: "sci-verify" }, status), "Q3 sci-verify「この研究が目指している大きな目的は何か。」");
});

test(function testNodeOps() {
  assert.deepStrictEqual(Object.keys(ST_NODE_OPS), ["Q", "A", "F", "E", "S"]);
  assert.deepStrictEqual(ST_NODE_OPS, {
    Q: ["add_q", "add_a", "insert_q", "move_a"],
    A: ["add_q", "add_f", "copy"],
    F: [],
    E: ["add_e", "set_mlflow"],
    S: ["add_e", "edit", "remove"],
  });
  // F は何も出さない（stance は板の別の節）
  assert.deepStrictEqual(stOpsFor({ kind: "F", parent: "A1" }, []), []);
  // Q に子が無ければ add_q と add_a。insert_q と move_a は出さない
  assert.deepStrictEqual(stOpsFor({ kind: "Q", parent: null }, []), ["add_q", "add_a"]);
  // Q で子に A があれば add_q を出さず、insert_q と move_a を出す
  assert.deepStrictEqual(stOpsFor({ kind: "Q", parent: null }, [{ kind: "A" }]), ["add_a", "insert_q", "move_a"]);
  // Q で子に Q があれば add_a を出さない
  assert.deepStrictEqual(stOpsFor({ kind: "Q", parent: null }, [{ kind: "Q" }]), ["add_q"]);
  // add_e は S（SPROUT）と E だけ
  assert.ok(!stOpsFor({ kind: "A" }, []).includes("add_e"));
  assert.ok(stOpsFor({ kind: "S" }, []).includes("add_e"));
  assert.ok(stOpsFor({ kind: "E" }, []).includes("add_e"));
  assert.ok(!stOpsFor({ kind: "E" }, []).includes("add_q"));
  // E には MLflow の変更
  assert.ok(stOpsFor({ kind: "E" }, []).includes("set_mlflow"));
  // E の下に F は置かない（結果は mlflow で結ぶ）
  assert.ok(!stOpsFor({ kind: "E" }, []).includes("add_f"));
  // A の add_q は常に出す
  assert.deepStrictEqual(stOpsFor({ kind: "A" }, []), ["add_q", "add_f", "copy"]);
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
    stHistoryText({ action: "Add F", by: "run:210", at, run_id: 210, block: "lab-read" }),
    `Add F ${date} run 210 lab-read`,
  );
  assert.strictEqual(
    stHistoryText({ action: "Run", at, run_id: 215, block: "lab-read", global_round: 12, round: 2 }),
    `Run 215 lab-read R12.2 ${date}`,
  );
  assert.strictEqual(
    stHistoryText({ action: "Run", at, run_id: 216, block: "sci-verify", global_round: null, round: null }),
    `Run 216 sci-verify ${date}`,
  );
  assert.strictEqual(stHistoryText({ action: "edit", by: "user", at }), `edit ${date} human`);
  // block が空なら run の番号だけ
  assert.strictEqual(
    stHistoryText({ action: "Add A", by: "run:3", at, run_id: 3, block: "" }),
    `Add A ${date} run 3`,
  );
  // round が null なら R<global_round> だけ
  assert.strictEqual(
    stHistoryText({ action: "Run", at, run_id: 1, block: "lab-read", global_round: 4, round: null }),
    `Run 1 lab-read R4 ${date}`,
  );
});

test(function testApprovalText() {
  // 許可した人は `許可: <名前>`。無ければ空
  assert.strictEqual(stApprovalText({ approved_by: "saituki" }), "許可: saituki");
  assert.strictEqual(stApprovalText({ approved_by: " 山田 " }), "許可: 山田");
  for (const h of [{}, { approved_by: null }, { approved_by: "" }, null]) assert.strictEqual(stApprovalText(h), "", JSON.stringify(h));
});

// Q1 → A2 → F3 と Q1 → A4 の 4 行（並びは status.nodes と同じ順）。A2 は ★
function treeRows() {
  return [
    { id: "Q1", kind: "Q", depth: 0, text: "問い", starred: false, node: { id: "Q1", parent: null } },
    { id: "A2", kind: "A", depth: 1, text: "二つ目", starred: true, node: { id: "A2", parent: "Q1" } },
    { id: "F3", kind: "F", depth: 2, text: "三つ目", stance: "support", starred: false, node: { id: "F3", parent: "A2" } },
    { id: "A4", kind: "A", depth: 1, text: "四つ目", starred: false, node: { id: "A4", parent: "Q1" } },
  ];
}

test(function testVisibleRowsFold() {
  const rows = treeRows();
  // 絞り込みも畳みも無ければ全部の行（ctx は全部 false）
  const all = stVisibleRows(rows, { folded: new Set(), word: "", kinds: new Set(ST_ALL_KINDS) });
  assert.deepStrictEqual(all.map((x) => [x.row.id, x.hasKids, x.ctx]), [
    ["Q1", true, false], ["A2", true, false], ["F3", false, false], ["A4", false, false],
  ]);
  // A2 を畳むと F3 だけが出なくなる
  const folded = stVisibleRows(rows, { folded: new Set(["A2"]) });
  assert.deepStrictEqual(folded.map((x) => x.row.id), ["Q1", "A2", "A4"]);
  assert.deepStrictEqual(folded.map((x) => x.hasKids), [true, true, false]);
  // 絞り込み中は畳みを無視する
  assert.deepStrictEqual(
    stVisibleRows(rows, { folded: new Set(["Q1", "A2"]), word: "F3" }).map((x) => x.row.id),
    ["Q1", "A2", "F3"],
  );
  // 絞り込みの無いときは全部出す（word も kinds も無ければ全部）
  assert.deepStrictEqual(stVisibleRows(rows, {}).map((x) => x.row.id), ["Q1", "A2", "F3", "A4"]);
  assert.deepStrictEqual(stVisibleRows([], {}), []);
  assert.deepStrictEqual(stVisibleRows(null, null), []);
});

test(function testVisibleRowsWord() {
  const out = stVisibleRows(treeRows(), { word: "F3" });
  assert.deepStrictEqual(out.map((x) => [x.row.id, x.ctx]), [["Q1", true], ["A2", true], ["F3", false]]);
  // 大文字・小文字を区別しない。ancestor も出す
  const ci = stVisibleRows(treeRows(), { word: "f3" });
  assert.deepStrictEqual(ci.map((x) => x.row.id), ["Q1", "A2", "F3"]);
  // どこにも無い文字なら、どの行も出さない
  assert.deepStrictEqual(stVisibleRows(treeRows(), { word: "存在しない" }), []);
});

test(function testVisibleRowsStar() {
  // `*` だけなら ★ の Node。その祖先は ctx で出す
  assert.deepStrictEqual(stVisibleRows(treeRows(), { word: "*" }).map((x) => [x.row.id, x.ctx]), [["Q1", true], ["A2", false]]);
  // `*` と文字の混ざりは普通の文字として扱う（★ では絞らない）
  assert.deepStrictEqual(stVisibleRows(treeRows(), { word: "*x" }), []);
  // 種類と合わせる
  assert.deepStrictEqual(stVisibleRows(treeRows(), { word: "*", kinds: new Set(["F"]) }), []);
  assert.strictEqual(stRowHit({ id: "A2", text: "x", starred: true }, "*"), true);
  assert.strictEqual(stRowHit({ id: "A2", text: "x", starred: false }, "*"), false);
});

test(function testVisibleRowsKinds() {
  // A だけ: F3 は出す必要が無い（A の子だが自身が種類で弾かれている）
  const out = stVisibleRows(treeRows(), { kinds: new Set(["A"]) });
  assert.deepStrictEqual(out.map((x) => [x.row.id, x.ctx]), [["Q1", true], ["A2", false], ["A4", false]]);
  // 種類と文字は両方の条件（両方満たす行だけ出す）
  const both = stVisibleRows(treeRows(), { word: "四", kinds: new Set(["A"]) });
  assert.deepStrictEqual(both.map((x) => x.row.id), ["Q1", "A4"]);
  // 全部切ると何も出ない
  assert.deepStrictEqual(stVisibleRows(treeRows(), { kinds: new Set() }), []);
});

test(function testActionMeta() {
  assert.strictEqual(stActionMeta({ run_id: 5, block: "lab-read", global_round: 2, owner: "R2", node: "A12" }), "run 5 lab-read R2 A12");
  assert.strictEqual(stActionMeta({ run_id: 7, block: "lab-search", global_round: null, owner: "review", node: null }), "run 7 lab-search review");
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

test(function testSproutToggle() {
  assert.deepStrictEqual(stSproutToggle([], "A1"), ["A1"]);
  assert.deepStrictEqual(stSproutToggle(["A1"], "A2"), ["A1", "A2"]);
  assert.deepStrictEqual(stSproutToggle(["A1", "A2"], "A1"), ["A2"]); // もう一度で外れる
  // A 以外は足さない（O は無くなった ID）
  for (const id of ["Q1", "F2", "E3", "S4", "O1", "A0", "A", "", null]) assert.deepStrictEqual(stSproutToggle(["A1"], id), ["A1"], String(id));
  // 元の配列は変えない
  const base = ["A1"];
  stSproutToggle(base, "A2");
  assert.deepStrictEqual(base, ["A1"]);
  // 21 個目は足さない。20 個のときの「外す」は効く
  const full = Array.from({ length: ST_SPROUT_PATH_MAX }, (_, i) => `A${i + 1}`);
  assert.strictEqual(ST_SPROUT_PATH_MAX, 20);
  assert.deepStrictEqual(stSproutToggle(full, "A99"), full);
  assert.strictEqual(stSproutToggle(full, "A5").length, 19);
  assert.deepStrictEqual(stSproutToggle(null, "A1"), ["A1"]);
});

test(function testSproutRowsAndSummary() {
  const status = { nodes: [
    { id: "A1", kind: "A", text: "答え" },
    { id: "A2", kind: "A", text: "課題" },
  ] };
  assert.deepStrictEqual(stSproutRows(["A2", "A1", "A9"], status), [
    { id: "A2", text: "課題" },
    { id: "A1", text: "答え" },
    { id: "A9", text: "" }, // status に無い ID は文なしで残す
  ]);
  assert.deepStrictEqual(stSproutRows([], status), []);
  assert.deepStrictEqual(stSproutRows(["A1"], null), [{ id: "A1", text: "" }]);
  // Issue の数は数えない（検査は無くなった）
  assert.strictEqual(stSproutSummary([]), "A をまだ選んでいません");
  assert.strictEqual(stSproutSummary(["A1"]), "PATH 1 個");
  assert.strictEqual(stSproutSummary(["A1", "A2"]), "PATH 2 個");
});

test(function testRangeText() {
  // SPROUT の範囲の数（作成の結果と S の板で同じ文）
  assert.strictEqual(
    stRangeText({ q: 3, a: 4, f_support: 2, f_deny: 1, f_issue: 0 }),
    "Q 3・A 4・F S 2 / D 1 / I 0",
  );
  assert.strictEqual(stRangeText({}), "Q 0・A 0・F S 0 / D 0 / I 0");
  assert.strictEqual(stRangeText(null), "");
  assert.strictEqual(stRangeText("x"), "");
});

let failed = 0;
for (const fn of tests) {
  try { fn(); } catch (e) { failed++; console.error(`NG ${fn.name}:`, e.message); }
}
if (failed) process.exit(1);
console.log(`test_scitree: ${tests.length} 件すべて通りました`);
