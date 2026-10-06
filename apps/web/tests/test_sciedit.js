"use strict";
// 操作の入力欄・カードのフォーム・仕分け箱（sciedit.js）の純関数のテスト（実装計画 U19）。
// 実行:
//     node apps/web/tests/test_sciedit.js
//
// DOM が要る描画・小窓・仕分け箱の操作はブラウザで確かめる（kantoku が統合のあとで行う）。

const assert = require("assert");
const path = require("path");

const src = path.join(__dirname, "..", "src", "sai_web", "static", "sciedit.js");
const {
  seTrim, seOpLabel, seCiteOk, seRefOk, seCountSentences, seNeedsCite, seBuildOp,
  seCardValues, seBuildCard, seProposalNodeKey, seProposalRunKey, seGroupProposals,
  seProposalSummary, seProposalEditKeys, seProposalList, seCardRunId,
  seNearestO, seSproutStartOs, seSproutNext, seChainPlan, seChainPath,
  SE_OPS, SE_DONE, SE_QUICK, SE_BLOCKS,
} = require(src);

/** seBuildOp が通ったときの op。 */
function ok(built) {
  assert.strictEqual(built.ok, true, `通るはずが弾かれました: ${built.error}`);
  assert.strictEqual(built.error, null);
  return built.op;
}

/** seBuildOp が弾いたときの理由。 */
function ng(built) {
  assert.strictEqual(built.ok, false, `弾かれるはず，通りました: ${JSON.stringify(built.op)}`);
  assert.strictEqual(built.op, null);
  assert.ok(built.error && typeof built.error === "string", "理由が返りません");
  return built.error;
}

function test_helpers() {
  assert.strictEqual(seTrim("  あ  "), "あ");
  assert.strictEqual(seTrim(null), "");
  // C4 の表示名
  assert.strictEqual(seOpLabel("add_q"), "Add Q");
  assert.strictEqual(seOpLabel("add_o"), "Add O");
  assert.strictEqual(seOpLabel("add_f"), "Add F");
  assert.strictEqual(seOpLabel("add_e"), "Add E");
  assert.strictEqual(seOpLabel("copy"), "Copy O");
  assert.strictEqual(seOpLabel("edit"), "Edit");
  assert.strictEqual(seOpLabel("remove"), "Delete");
  assert.strictEqual(seOpLabel("add_seed"), "Sprout を作る");
  assert.strictEqual(seOpLabel("mystery_op"), "mystery_op");
  assert.ok(seRefOk("Q12", "Q"));
  assert.ok(seRefOk("E3", "QOFE"));
  assert.ok(!seRefOk("O3", "Q"));
  assert.ok(!seRefOk("N12"));
  assert.ok(!seRefOk("Q01"));
  assert.strictEqual(seCountSentences("あ。い。う。え。"), 4);
  assert.strictEqual(seCountSentences("これは 0.85 です"), 1);
  assert.strictEqual(seNeedsCite("精度は 0.9", []), true);
  assert.strictEqual(seNeedsCite("精度は 0.9", ["#37"]), false);
}

// --- C-CITE ---
function test_cite() {
  for (const c of ["#37", "#37.3", "#37.3.2", "#37.3.2-4", "#37.3.2 注", "#37 §3.2"]) assert.ok(seCiteOk(c), c);
  for (const c of ["37", "#0", "", "#37.0", "#37.1.2.3", "#37." + "x"]) assert.ok(!seCiteOk(c), c);
}

// --- 13 op の通る例 ---
function test_ops_ok() {
  assert.deepStrictEqual(ok(seBuildOp("add_q", { parent: "Q1", text: "データの量は？" })), { op: "add_q", parent: "Q1", text: "データの量は？" });
  assert.strictEqual(ok(seBuildOp("add_q", { parent: "O2", text: "x", reason: "r" })).reason, "r");
  assert.deepStrictEqual(ok(seBuildOp("add_o", { q: "Q1", text: "SST-2", kind: "issue", body: "b" })), { op: "add_o", q: "Q1", text: "SST-2", body: "b", kind: "issue" });
  assert.deepStrictEqual(ok(seBuildOp("add_f", { parent: "O2", text: "精度は 0.9。", cites: ["#37 §3.2", " "], paper: "37" })), { op: "add_f", parent: "O2", text: "精度は 0.9。", cites: ["#37 §3.2"], paper: 37 });
  assert.deepStrictEqual(ok(seBuildOp("add_f", { parent: "E1", text: "出典なしの文" })), { op: "add_f", parent: "E1", text: "出典なしの文" });
  assert.deepStrictEqual(ok(seBuildOp("add_e", { parent: "O2", text: "実験", mlflow: "run-a" })), { op: "add_e", parent: "O2", text: "実験", mlflow: "run-a" });
  assert.deepStrictEqual(ok(seBuildOp("record_logic", { q: "Q1", value: "ok" })), { op: "record_logic", q: "Q1", value: "ok" });
  assert.deepStrictEqual(
    ok(seBuildOp("record_logic", { q: "Q1", value: "contradiction", note: "食い違う", findings: ["F1", "F2"] })),
    { op: "record_logic", q: "Q1", value: "contradiction", note: "食い違う", findings: ["F1", "F2"] },
  );
  assert.deepStrictEqual(ok(seBuildOp("add_seed", { title: "題", sentence: "文", path: ["O1", "O2"] })), { op: "add_seed", title: "題", sentence: "文", path: ["O1", "O2"] });
  assert.deepStrictEqual(ok(seBuildOp("copy", { node: "O2", to: "Q5" })), { op: "copy", node: "O2", to: "Q5" });
  assert.deepStrictEqual(ok(seBuildOp("remove", { node: "F3", reason: "誤り" })), { op: "remove", node: "F3", reason: "誤り" });
  assert.deepStrictEqual(ok(seBuildOp("star", { o: "O2" })), { op: "star", o: "O2" });
  assert.deepStrictEqual(ok(seBuildOp("unstar", { o: "O2" })), { op: "unstar", o: "O2" });
  assert.deepStrictEqual(ok(seBuildOp("set_kind", { o: "O2", kind: "issue" })), { op: "set_kind", o: "O2", kind: "issue" });
  assert.deepStrictEqual(ok(seBuildOp("edit", { node: "Q1", text: "新しい文" })), { op: "edit", node: "Q1", text: "新しい文" });
  assert.deepStrictEqual(ok(seBuildOp("edit", { node: "O1", body: "本文だけ" })), { op: "edit", node: "O1", body: "本文だけ" });
  assert.deepStrictEqual(ok(seBuildOp("set_target", { q: "Q1", target: "30" })), { op: "set_target", q: "Q1", target: 30 });
  assert.deepStrictEqual(ok(seBuildOp("set_target", { q: "Q1", target: 0 })), { op: "set_target", q: "Q1", target: 0 });
  assert.deepStrictEqual(ok(seBuildOp("set_target", { q: "Q1", target: "" })), { op: "set_target", q: "Q1", target: null });
}

// --- NG の例 ---
function test_ops_ng() {
  // 必須・形
  ng(seBuildOp("add_q", { parent: "F1", text: "x" })); // 親は Q か O
  ng(seBuildOp("add_q", { parent: "Q1", text: "" }));
  ng(seBuildOp("add_o", { q: "O1", text: "x" }));
  ng(seBuildOp("add_o", { q: "Q1", text: "x", kind: "other" }));
  ng(seBuildOp("add_f", { parent: "Q1", text: "x" })); // 親は O か E
  ng(seBuildOp("add_e", { parent: "Q1", text: "x" }));
  ng(seBuildOp("copy", { node: "O2", to: "O3" }));
  ng(seBuildOp("star", {}));
  ng(seBuildOp("set_kind", { o: "O2", kind: "" }));
  ng(seBuildOp("add_seed", { title: "題", sentence: "文", path: [] }));
  ng(seBuildOp("add_seed", { title: "題", sentence: "文", path: ["Q1"] }));
  ng(seBuildOp("edit", { node: "Q1" })); // text も body も無い
  // 字数
  ng(seBuildOp("add_q", { parent: "Q1", text: "あ".repeat(301) }));
  assert.ok(ok(seBuildOp("add_q", { parent: "Q1", text: "あ".repeat(300) })));
  ng(seBuildOp("add_o", { q: "Q1", text: "あ".repeat(121) }));
  ng(seBuildOp("add_e", { parent: "O1", text: "あ".repeat(121) }));
  ng(seBuildOp("add_seed", { title: "あ".repeat(121), sentence: "文", path: ["O1"] }));
  ng(seBuildOp("add_seed", { title: "題", sentence: "あ".repeat(601), path: ["O1"] }));
  ng(seBuildOp("add_q", { parent: "Q1", text: "x", reason: "あ".repeat(501) }));
  // C-CITE
  ng(seBuildOp("add_f", { parent: "O2", text: "文", cites: ["37"] }));
  ng(seBuildOp("add_f", { parent: "O2", text: "文", cites: Array(11).fill("#37") }));
  // 1〜3 文
  ng(seBuildOp("add_f", { parent: "O2", text: "あ。い。う。え。" }));
  assert.ok(ok(seBuildOp("add_f", { parent: "O2", text: "あ。い。う。" })));
  // 数字と出典
  ng(seBuildOp("add_f", { parent: "O2", text: "精度は 0.9" }));
  // remove は理由が要る
  ng(seBuildOp("remove", { node: "F3" }));
  ng(seBuildOp("remove", { node: "F3", reason: " " }));
  // contradiction の条件
  ng(seBuildOp("record_logic", { q: "Q1", value: "contradiction", findings: ["F1", "F2"] })); // note 無し
  ng(seBuildOp("record_logic", { q: "Q1", value: "contradiction", note: "n", findings: ["F1"] })); // 1 つ
  ng(seBuildOp("record_logic", { q: "Q1", value: "maybe" }));
  // set_target の範囲
  ng(seBuildOp("set_target", { q: "Q1", target: "100" }));
  ng(seBuildOp("set_target", { q: "Q1", target: "-1" }));
  ng(seBuildOp("set_target", { q: "Q1", target: "1.5" }));
  ng(seBuildOp("set_target", { q: "O1", target: "5" }));
}

function test_unknown() {
  assert.strictEqual(ng(seBuildOp("mystery_op", {})), "Unknown operation");
  // 古い op は通らない
  for (const old of ["decide", "add_option", "add_child", "link", "add_finding"]) assert.strictEqual(ng(seBuildOp(old, {})), "Unknown operation");
}

function test_tables() {
  const keys = Object.keys(SE_OPS).sort();
  assert.strictEqual(keys.length, 13);
  assert.deepStrictEqual(Object.keys(SE_DONE).sort(), keys);
  for (const q of SE_QUICK) assert.ok(keys.includes(q));
  const ids = SE_BLOCKS.map(([id]) => id);
  assert.strictEqual(ids.length, 21);
  assert.strictEqual(new Set(ids).size, 21);
  for (const id of ids) assert.match(id, /^sci-[a-z-]{2,30}$/);
  assert.ok(ids.includes("sci-read") && ids.includes("sci-split"));
  for (const gone of ["sci-draft", "sci-probe", "sci-outline", "sci-tree-review"]) assert.ok(!ids.includes(gone), gone);
}

// --- カードの初めの値（C-CARD）と検査 ---
function test_card() {
  const preset = {
    node: "Q12", block: "sci-ideate", purpose: "「データの量は？」の答えの候補（O）を出す",
    expect: "O を 2 つ以上", scope: "path", assume: ["Q9=SST-2 を使う（仮）"], mode: "purpose_first",
    papers: [37], runner: "pi", depth: "full", origin_run: 120,
  };
  assert.deepStrictEqual(seCardValues(preset), {
    node: "Q12", block: "sci-ideate", purpose: preset.purpose, expect: "O を 2 つ以上", runner: "pi",
    scope: "path", mode: "purpose_first", assume: ["Q9=SST-2 を使う（仮）"], papers: [37], depth: "full",
  });
  // 無ければ空（nodeId だけ入る）
  const empty = seCardValues(null, "Q3");
  assert.strictEqual(empty.node, "Q3");
  assert.strictEqual(empty.block, "");
  assert.strictEqual(empty.purpose, "");
  assert.deepStrictEqual(empty.papers, []);
  // user ならブロックの既定は sci-read
  assert.strictEqual(seCardValues({ runner: "user" }, "O2").block, "sci-read");

  const built = seBuildCard(seCardValues(preset));
  assert.strictEqual(built.ok, true);
  assert.deepStrictEqual(built.card, {
    node: "Q12", block: "sci-ideate", purpose: preset.purpose, runner: "pi", papers: [37],
    expect: "O を 2 つ以上", scope: "path", assume: ["Q9=SST-2 を使う（仮）"],
  });
  // 打ち込んだ番号（# 付き・重なり）
  assert.deepStrictEqual(seBuildCard(Object.assign(seCardValues(preset), { papers: [37, "#37", "40"] })).card.papers, [37, 40]);

  const bad = (over) => { const r = seBuildCard(Object.assign(seCardValues(preset), over)); assert.strictEqual(r.ok, false); return r.error; };
  bad({ purpose: " " });
  bad({ purpose: "あ".repeat(301) });
  bad({ expect: "あ".repeat(301) });
  bad({ block: "sci-draft" });
  bad({ assume: ["N12=x"] });
  bad({ assume: ["Q1=a", "Q2=a", "Q3=a", "Q4=a", "Q5=a", "Q6=a"] });
  bad({ papers: Array.from({ length: 21 }, (_, i) => i + 1) });
  bad({ papers: ["abc"] });

  // 読む札: 論文はちょうど 1 本・対象は Q か O
  const read = { node: "O2", block: "sci-read", purpose: "読む", runner: "user", papers: [37], depth: "figures" };
  assert.deepStrictEqual(seBuildCard(read).card, { node: "O2", block: "sci-read", purpose: "読む", runner: "user", papers: [37], depth: "figures" });
  bad(Object.assign({}, read, { papers: [37, 38] }));
  bad(Object.assign({}, read, { papers: [] }));
  assert.match(bad(Object.assign({}, read, { node: "F4" })), /Q か O/);
}

// --- 仕分け ---
function test_group_proposals() {
  const list = [
    { id: "P60", op: "add_f", run_id: 121, payload: { parent: "O12", text: "a" }, status: "pending" },
    { id: "P59", op: "add_o", run_id: 121, payload: { q: "O12" }, status: "pending" },
    { id: "P58", op: "star", run_id: 120, payload: { o: "O9" }, status: "pending" },
  ];
  const byNode = seGroupProposals(list, "node");
  assert.deepStrictEqual(byNode.map((g) => g.key), ["O12", "O9"]);
  assert.deepStrictEqual(byNode.map((g) => g.items.length), [2, 1]);
  const byRun = seGroupProposals(list, "run");
  assert.deepStrictEqual(byRun.map((g) => g.key), ["121", "120"]);
  assert.strictEqual(byRun[0].label, "run 121");

  // payload の parent・q・node・o の順
  assert.strictEqual(seProposalNodeKey({ payload: { node: "F3", q: "Q1" } }), "Q1");
  assert.strictEqual(seProposalNodeKey({ payload: { node: "F3", o: "O1" } }), "F3");
  assert.strictEqual(seProposalNodeKey({ payload: { parent: "Q5", q: "Q1" } }), "Q5");
  assert.strictEqual(seProposalNodeKey({ payload: { o: "O1" } }), "O1");
  assert.strictEqual(seProposalNodeKey({ payload: { title: "x" } }), "");
  assert.strictEqual(seProposalNodeKey({}), "");
  assert.strictEqual(seProposalRunKey({ run_id: 120 }), "120");
  assert.strictEqual(seProposalRunKey({ run_id: 0 }), "");

  const orphan = seGroupProposals([{ id: "P1", op: "add_seed", payload: {} }], "node");
  assert.strictEqual(orphan[0].label, "Node なし");
  assert.strictEqual(seGroupProposals([{ id: "P1" }], "run")[0].label, "run 不明");
  assert.strictEqual(seGroupProposals([{ id: "P1", payload: { q: "Q12" }, node_title: "小目的" }], "node")[0].label, "Q12 小目的");
  assert.deepStrictEqual(seGroupProposals(null, "node"), []);
  assert.deepStrictEqual(seGroupProposals([null, undefined], "node"), []);
}

function test_proposal_text() {
  const p = { id: "P59", op: "add_o", payload: { q: "Q12", text: "前頭葉", body: "…" } };
  assert.strictEqual(seProposalSummary(p), "前頭葉");
  assert.strictEqual(seProposalSummary({ payload: { parent: "Q1", text: "文" } }), "文　Q1");
  assert.deepStrictEqual(seProposalEditKeys(p), ["text", "body"]);
  assert.deepStrictEqual(seProposalEditKeys({ payload: { title: "a", sentence: "b", text: "c" } }), ["text", "title", "sentence"]);
  assert.deepStrictEqual(seProposalEditKeys({ payload: { o: "O1" } }), []);
  assert.ok(seProposalSummary({ payload: { text: "あ".repeat(200) } }).length <= 91);
  assert.ok(seProposalSummary({}).includes("中身がありません"));
}

// --- Sprout の鎖（Q1 → O2 → Q3 → O4(Issue)、Q3 → Q5 → O6、Q1 → O7） ---
function sproutNodes() {
  const n = (id, parent, extra) => Object.assign({ id, parent, text: `${id} の文` }, extra || {});
  return [
    n("Q1", null),
    n("O2", "Q1", { answer_kind: "answer" }),
    n("Q3", "O2"),
    n("O4", "Q3", { answer_kind: "issue" }),
    n("Q5", "Q3"),
    n("O6", "Q5", { answer_kind: "answer" }),
    n("O7", "Q1", { answer_kind: "answer" }),
  ];
}

function test_sprout_chain() {
  const nodes = sproutNodes();
  // seNearestO: O から上へ Q をたどって最初に会う O
  assert.strictEqual(seNearestO(nodes, "O4"), "O2");
  assert.strictEqual(seNearestO(nodes, "O6"), "O2");
  assert.strictEqual(seNearestO(nodes, "O2"), null);
  assert.strictEqual(seNearestO(nodes, "O7"), null);
  assert.strictEqual(seNearestO(nodes, "Q1"), null);
  // seSproutStartOs: Q の直の子の O
  assert.deepStrictEqual(seSproutStartOs(nodes, "Q1").map((o) => o.id), ["O2", "O7"]);
  assert.deepStrictEqual(seSproutStartOs(nodes, "Q3").map((o) => o.id), ["O4"]);
  assert.deepStrictEqual(seSproutStartOs(nodes, "Q9"), []);
  // seSproutNext: 直前の O の下の O
  assert.deepStrictEqual(seSproutNext(nodes, "O2").map((o) => o.id), ["O4", "O6"]);
  assert.deepStrictEqual(seSproutNext(nodes, "O4"), []);
  assert.deepStrictEqual(seSproutNext(nodes, "O7"), []);
}

function test_chain_plan() {
  const nodes = sproutNodes();
  // 開始の Q を選んでいないときは 1 段だけ
  assert.strictEqual(seChainPlan(nodes, ["", ""]).rows.length, 1);
  // Q1 → 2 段目（O2・O7）
  const r1 = seChainPlan(nodes, ["Q1", ""]);
  assert.strictEqual(r1.rows.length, 2);
  assert.deepStrictEqual(r1.rows[1].options.map((x) => x[0]), ["O2", "O7"]);
  // Q1 → O2 → 3 段目（O4・O6）
  const r2 = seChainPlan(nodes, ["Q1", "O2", ""]);
  assert.strictEqual(r2.rows.length, 3);
  assert.deepStrictEqual(r2.rows[2].options.map((x) => x[0]), ["O4", "O6"]);
  // Q1 → O2 → O4 → 候補が無いので 3 段で止まる
  const r3 = seChainPlan(nodes, ["Q1", "O2", "O4"]);
  assert.strictEqual(r3.rows.length, 3);
  // seChainPath: 空で止める
  assert.deepStrictEqual(seChainPath(nodes, ["Q1", "O2", "O4"]), ["O2", "O4"]);
  assert.deepStrictEqual(seChainPath(nodes, ["Q1", "O2", ""]), ["O2"]);
  assert.deepStrictEqual(seChainPath(nodes, ["Q1", ""]), []);
}

function test_response_shapes() {
  assert.deepStrictEqual(seProposalList([{ id: "P1" }]), [{ id: "P1" }]);
  assert.deepStrictEqual(seProposalList({ proposals: [{ id: "P1" }] }), [{ id: "P1" }]);
  assert.deepStrictEqual(seProposalList({ pending: [{ id: "P1" }] }), [{ id: "P1" }]);
  assert.deepStrictEqual(seProposalList(null), []);
  assert.strictEqual(seCardRunId({ run_id: 130 }), 130);
  assert.strictEqual(seCardRunId({ id: 7 }), 7);
  assert.strictEqual(seCardRunId({ run: { run_id: 9 } }), 9);
  assert.strictEqual(seCardRunId(null), null);
  assert.strictEqual(seCardRunId({}), null);
}

function main() {
  test_helpers();
  test_cite();
  test_ops_ok();
  test_ops_ng();
  test_unknown();
  test_tables();
  test_card();
  test_sprout_chain();
  test_chain_plan();
  test_group_proposals();
  test_proposal_text();
  test_response_shapes();
  console.log("test_sciedit: すべて通りました");
}

main();
