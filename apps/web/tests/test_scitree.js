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
  stSeedNodes, stSeedChildren, stSeedOf, stSeedDirty,
  stStepView, stStepNodeView, stStepParse,
} = require(path.join(dir, "scitree.js"));

const tests = [];
const test = (fn) => tests.push(fn);

test(function testParseHash() {
  assert.deepStrictEqual(stParseHash("#/research/tree"), {});
  // SEED も S1 も受ける（SEED は札の特別な ID）
  assert.deepStrictEqual(stParseHash("#/research/tree/SEED"), { id: "SEED" });
  for (const id of ["Q12", "A5", "F3", "E2", "S1", "D1"]) assert.deepStrictEqual(stParseHash(`#/research/tree/${id}`), { id });
  // D は数字の付いた ID だけ（小文字・0 は受けない）
  for (const h of ["#/research/tree/D0", "#/research/tree/d1"]) assert.strictEqual(stParseHash(h), null, h);
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
  assert.deepStrictEqual(Object.keys(ST_NODE_OPS), ["Q", "A", "F", "E", "S", "D"]);
  assert.deepStrictEqual(ST_NODE_OPS, {
    Q: ["add_q", "add_a", "insert_q", "move_a"],
    A: ["add_q", "add_f", "copy"],
    F: [],
    E: ["add_e", "set_mlflow"],
    S: ["add_e", "edit", "remove"],
    D: ["edit", "remove"],
  });
  // 絞り込みの種類に D（seed）が入る
  assert.deepStrictEqual(ST_ALL_KINDS, ["Q", "A", "F", "E", "S", "D"]);
  // D の板は Edit / Delete だけ（SPROUT の作成は板の釦で別に出す）
  assert.deepStrictEqual(stOpsFor({ kind: "D", parent: null }, []), ["edit", "remove"]);
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

test(function testSeedHelpers() {
  const status = { nodes: [
    { id: "Q1", kind: "Q", parent: null, text: "問い" },
    { id: "D1", kind: "D", parent: null, text: "発作部位" },
    { id: "D2", kind: "D", parent: null, text: "予後" },
    { id: "S1", kind: "S", parent: "D1", text: "1 本目" },
    { id: "S2", kind: "S", parent: "D1", text: "2 本目" },
    { id: "S3", kind: "S", parent: null, text: "古い S" },
    { id: "E1", kind: "E", parent: "S1", text: "実験" },
  ] };
  // seed の行は D だけ（ID の順のまま）
  assert.deepStrictEqual(stSeedNodes(status).map((n) => n.id), ["D1", "D2"]);
  assert.deepStrictEqual(stSeedNodes(null), []);
  // 子の SPROUT は親が D の S だけ。古い S（親が無い）は含まない
  assert.deepStrictEqual(stSeedChildren(status, "D1").map((n) => n.id), ["S1", "S2"]);
  assert.deepStrictEqual(stSeedChildren(status, "D2"), []);
  // S の親の D を引く。親が無い古い S は null
  assert.strictEqual(stSeedOf(status, { id: "S1", parent: "D1" }).id, "D1");
  assert.strictEqual(stSeedOf(status, { id: "S3", parent: null }), null);
  assert.strictEqual(stSeedOf(status, { id: "S9", parent: "D9" }), null);
  // SEED の入力に書いた物があるか（「空欄」の選びと空の欄は数えない）
  assert.strictEqual(stSeedDirty({ title: "", background: "", task_mode: "empty", task_value: "", success: "  " }), false);
  assert.strictEqual(stSeedDirty({ title: "", task_mode: "decided" }), true);
  assert.strictEqual(stSeedDirty({ title: "題" }), true);
  assert.strictEqual(stSeedDirty(null), false);
});

// --- 過去の step（stStepView・stStepNodeView・stStepParse）---
// 8 の step: add_q Q1・add_a A1・add_f F1・insert_q Q2（A1 を移す）・add_a A2・remove F1・move_a A2・add_f F2。
// 今の木は step 8 の後（F1 は step 6 に消えて steps.removed に入る）。
function stepStatus() {
  const T = (n, s = 0) => `2026-10-02T01:${String(n).padStart(2, "0")}:${String(s).padStart(2, "0")}+00:00`;
  const nj = (id, kind, parent, extra) => Object.assign({
    id, kind, parent, depth: 0, text: `${id} の文`, starred: false, stance: null,
    counts: { q: 0, a: 0, f: 0, e: 0 }, bottom_q: false, range: null, ord: 0,
  }, extra || {});
  const ops = [["add_q", "Q1"], ["add_a", "A1"], ["add_f", "F1"], ["insert_q", "Q2"], ["add_a", "A2"], ["remove", "F1"], ["move_a", "A2"], ["add_f", "F2"]];
  return {
    initialized: true,
    type: null,
    warnings: [],
    next: [{ kind: "user", node: "Q2" }],
    decisions: [{ node: "A1", action: "approve", at: T(2, 30) }],
    history: [
      { action: "init", at: "2026-10-02T00:59:00+00:00" },
      { action: "Add Q", at: T(1) },
      { action: "Add A", at: T(2) },
      { action: "Run", at: T(2, 30) },
      { action: "Add F", at: T(3) },
    ],
    nodes: [
      nj("Q1", "Q", null),
      nj("Q2", "Q", "Q1"),
      nj("A1", "A", "Q2", { ord: 1 }),
      nj("A2", "A", "Q2", { ord: 0 }),
      nj("F2", "F", "A1", { stance: "deny" }),
    ],
    steps: {
      total: 8,
      list: ops.map(([op, node], i) => ({ n: i + 1, op, node, at: T(i + 1), by: "user" })),
      nodes: [
        { id: "Q1", born: 1, gone: null, parents: [{ step: 1, parent: null }] },
        { id: "A1", born: 2, gone: null, parents: [{ step: 2, parent: "Q1" }, { step: 4, parent: "Q2" }] },
        { id: "F1", born: 3, gone: 6, parents: [{ step: 3, parent: "A1" }] },
        { id: "Q2", born: 4, gone: null, parents: [{ step: 4, parent: "Q1" }] },
        { id: "A2", born: 5, gone: null, parents: [{ step: 5, parent: "Q1" }, { step: 7, parent: "Q2" }] },
        { id: "F2", born: 8, gone: null, parents: [{ step: 8, parent: "A1" }] },
      ],
      removed: [nj("F1", "F", "A1", { stance: "support" })],
    },
  };
}

test(function testStepViewCreateOnly() {
  const st = stepStatus();
  const before = JSON.stringify(st);
  const ids = (v) => v.nodes.map((n) => n.id);
  assert.deepStrictEqual(ids(stStepView(st, 1)), ["Q1"]);
  assert.deepStrictEqual(ids(stStepView(st, 2)), ["Q1", "A1"]);
  const v3 = stStepView(st, 3);
  assert.deepStrictEqual(ids(v3), ["Q1", "A1", "F1"]);
  assert.strictEqual(v3.nodes[2].parent, "A1");
  assert.strictEqual(v3.nodes[2].depth, 2);
  assert.deepStrictEqual(v3.counts, { q: 1, a: 1, f_support: 1, f_deny: 0, f_issue: 0, e: 0, s: 0 });
  // 元の status は書き換えない
  assert.strictEqual(JSON.stringify(st), before);
});

test(function testStepViewInsertQ() {
  const st = stepStatus();
  // insert_q の前: 新しい Q2 はまだ無く、A1 は旧 Q1 の下
  const v3 = stStepView(st, 3);
  assert.ok(!v3.nodes.some((n) => n.id === "Q2"));
  assert.strictEqual(v3.nodes.find((n) => n.id === "A1").parent, "Q1");
  // insert_q の後: A1 の親は新しい Q2。Q1 の下は Q2 だけ
  const v4 = stStepView(st, 4);
  assert.strictEqual(v4.nodes.find((n) => n.id === "A1").parent, "Q2");
  assert.deepStrictEqual(v4.nodes.map((n) => n.id), ["Q1", "Q2", "A1", "F1"]);
  assert.deepStrictEqual(v4.nodes[0].counts, { q: 1, a: 0, f: 0, e: 0 });
  assert.strictEqual(v4.nodes[1].bottom_q, true, "Q2 の下に Q が無い");
  assert.strictEqual(v4.nodes[0].bottom_q, false, "Q1 の下に Q2 がある");
});

test(function testStepViewRemove() {
  const st = stepStatus();
  // gone > k では消えた Node が見える（steps.removed の中身）。gone <= k では見えない
  const f1 = stStepView(st, 5).nodes.find((n) => n.id === "F1");
  assert.ok(f1, "gone > k なら見える");
  assert.strictEqual(f1.text, "F1 の文");
  assert.strictEqual(stStepView(st, 5).counts.f_support, 1);
  assert.ok(!stStepView(st, 6).nodes.some((n) => n.id === "F1"), "gone <= k では見えない");
});

test(function testStepViewMoveA() {
  const st = stepStatus();
  // move_a の前（step 6）: A2 は Q1 の下。Q1 の数は Q2 と A2
  const v6 = stStepView(st, 6);
  const a2 = v6.nodes.find((n) => n.id === "A2");
  assert.strictEqual(a2.parent, "Q1");
  assert.strictEqual(a2.depth, 1);
  assert.deepStrictEqual(v6.nodes.find((n) => n.id === "Q1").counts, { q: 1, a: 1, f: 0, e: 0 });
  // move_a の後（step 7）: A2 は Q2 の下。Q2 は Q を持たないので bottom_Q
  const v7 = stStepView(st, 7);
  assert.strictEqual(v7.nodes.find((n) => n.id === "A2").parent, "Q2");
  assert.strictEqual(v7.nodes.find((n) => n.id === "A2").depth, 2);
  assert.deepStrictEqual(v7.nodes.find((n) => n.id === "Q1").counts, { q: 1, a: 0, f: 0, e: 0 });
  assert.deepStrictEqual(v7.nodes.find((n) => n.id === "Q2").counts, { q: 0, a: 2, f: 0, e: 0 });
  assert.strictEqual(v7.nodes.find((n) => n.id === "Q2").bottom_q, true);
  assert.strictEqual(v7.nodes.find((n) => n.id === "Q1").bottom_q, false);
  assert.deepStrictEqual(v7.counts, { q: 2, a: 2, f_support: 0, f_deny: 0, f_issue: 0, e: 0, s: 0 });
});

test(function testStepViewLatest() {
  const st = stepStatus();
  // 総数以上・step 無し・steps 無しは、元の status をそのまま返す
  assert.strictEqual(stStepView(st, 8), st);
  assert.strictEqual(stStepView(st, 9), st);
  assert.strictEqual(stStepView(st, null), st);
  // steps が無い status（始めていない木）も、そのまま返す
  const plain = { nodes: [{ id: "Q1", kind: "Q" }] };
  assert.strictEqual(stStepView(plain, 1), plain);
});

test(function testStepNodeView() {
  const st = stepStatus();
  // 板の元の detail（node は id だけでよい。中身は step の木から作る）
  const det = (id) => ({ node: { id }, history: st.history, decisions: st.decisions, next: st.next, guide: { question: "q" } });
  const q1 = stStepNodeView(det("Q1"), stStepView(st, 4), 4);
  assert.deepStrictEqual(q1.children.map((n) => n.id), ["Q2"]);
  assert.deepStrictEqual(q1.subtree.map((n) => n.id), ["Q2", "A1", "F1"]);
  assert.deepStrictEqual(q1.path.map((p) => p.id), ["Q1"]);
  assert.deepStrictEqual(q1.next, []);
  assert.strictEqual(q1.guide.question, "q", "guide はそのまま");
  const a1 = stStepNodeView(det("A1"), stStepView(st, 4), 4);
  assert.deepStrictEqual(a1.path.map((p) => p.id), ["Q1", "Q2", "A1"]);
  assert.deepStrictEqual(a1.path[2], { id: "A1", kind: "A", text: "A1 の文" });
  assert.deepStrictEqual(a1.children.map((n) => n.id), ["F1"]);
  // History は step の時刻より後の Add・Run を落とす（init は残す）。決定も同じ
  const h2 = stStepNodeView(det("Q1"), stStepView(st, 2), 2);
  assert.deepStrictEqual(h2.history.map((h) => h.action), ["init", "Add Q", "Add A"]);
  assert.deepStrictEqual(h2.decisions, []);
  const h3 = stStepNodeView(det("Q1"), stStepView(st, 3), 3);
  assert.deepStrictEqual(h3.history.map((h) => h.action), ["init", "Add Q", "Add A", "Run", "Add F"]);
  assert.strictEqual(h3.decisions.length, 1);
  // その step に無い Node（まだ無い・消えた後）は null
  assert.strictEqual(stStepNodeView(det("F1"), stStepView(st, 2), 2), null);
  assert.strictEqual(stStepNodeView(det("F1"), stStepView(st, 6), 6), null);
  assert.strictEqual(stStepNodeView(det("Q2"), stStepView(st, 3), 3), null);
  // 元の detail は書き換えない
  const d0 = det("Q1");
  stStepNodeView(d0, stStepView(st, 2), 2);
  assert.strictEqual(d0.history.length, 5);
});

test(function testStepParse() {
  assert.strictEqual(stStepParse(" 15 ", 30), 15);
  assert.strictEqual(stStepParse("0", 30), 1);
  assert.strictEqual(stStepParse("99", 30), 30);
  assert.strictEqual(stStepParse("007", 30), 7);
  for (const t of ["abc", "1e3", "-1", "1.5", ""]) assert.strictEqual(stStepParse(t, 30), null, t);
  assert.strictEqual(stStepParse("1", 0), null, "総数が 0");
  assert.strictEqual(stStepParse(null, 30), null);
});

let failed = 0;
for (const fn of tests) {
  try { fn(); } catch (e) { failed++; console.error(`NG ${fn.name}:`, e.message); }
}
if (failed) process.exit(1);
console.log(`test_scitree: ${tests.length} 件すべて通りました`);
