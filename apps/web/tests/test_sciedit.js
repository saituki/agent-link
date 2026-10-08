"use strict";
// 操作の入力欄・カードのフォーム（sciedit.js）の純関数のテスト（実装計画 U19・U06 で Q・A・F・E・S に合わせた）。
// 実行:
//     node apps/web/tests/test_sciedit.js
//
// DOM が要る描画・小窓の操作はブラウザで確かめる（kantoku が統合のあとで行う）。

const assert = require("assert");
const path = require("path");

const src = path.join(__dirname, "..", "src", "sai_web", "static", "sciedit.js");
const {
  seTrim, seOpLabel, seCiteOk, seRefOk, seCountSentences, seNeedsCite, seBuildOp,
  seCardValues, seBuildCard, seCardRunId,
  seChildOptions, seChildOk,
  seBottomQs,
  SE_OPS, SE_DONE, SE_QUICK, SE_BLOCKS, SE_STANCES,
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
  // 表示名（O の名は無くなり、A になった）
  assert.strictEqual(seOpLabel("add_q"), "Add Q");
  assert.strictEqual(seOpLabel("add_a"), "Add A");
  assert.strictEqual(seOpLabel("add_f"), "Add F");
  assert.strictEqual(seOpLabel("add_e"), "Add E");
  assert.strictEqual(seOpLabel("copy"), "Copy A");
  assert.strictEqual(seOpLabel("edit"), "Edit");
  assert.strictEqual(seOpLabel("remove"), "Delete");
  assert.strictEqual(seOpLabel("add_sprout"), "Create SPROUT");
  assert.strictEqual(seOpLabel("mystery_op"), "mystery_op");
  assert.ok(seRefOk("Q12", "Q"));
  assert.ok(seRefOk("E3", "QAFE"));
  assert.ok(seRefOk("S4"));
  assert.ok(!seRefOk("A3", "Q"));
  assert.ok(!seRefOk("O3"));
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

// --- 操作の通る例 ---
function test_ops_ok() {
  assert.deepStrictEqual(ok(seBuildOp("add_q", { parent: "Q1", text: "データの量は？" })), { op: "add_q", parent: "Q1", text: "データの量は？" });
  assert.deepStrictEqual(ok(seBuildOp("add_q", { parent: "SEED", text: "最初の問い" })), { op: "add_q", parent: "SEED", text: "最初の問い" });
  assert.deepStrictEqual(ok(seBuildOp("add_q", { parent: "A2", text: "x", reason: "r" })), { op: "add_q", parent: "A2", text: "x" });
  assert.deepStrictEqual(ok(seBuildOp("add_a", { q: "Q1", text: "SST-2", body: "b" })), { op: "add_a", q: "Q1", text: "SST-2", body: "b" });
  // F は stance が要る。出典と論文番号は任意
  assert.deepStrictEqual(
    ok(seBuildOp("add_f", { parent: "A2", stance: "S", text: "精度は 0.9。", cites: ["#37 §3.2", " "], paper: "37" })),
    { op: "add_f", parent: "A2", stance: "S", text: "精度は 0.9。", cites: ["#37 §3.2"], paper: 37 },
  );
  assert.deepStrictEqual(ok(seBuildOp("add_f", { parent: "E1", stance: "I", text: "出典なしの文" })), { op: "add_f", parent: "E1", stance: "I", text: "出典なしの文" });
  assert.deepStrictEqual(ok(seBuildOp("add_e", { parent: "E1", text: "実験", mlflow: "run-a" })), { op: "add_e", parent: "E1", text: "実験", mlflow: "run-a" });
  assert.deepStrictEqual(ok(seBuildOp("add_e", { parent: "S2", text: "実験" })), { op: "add_e", parent: "S2", text: "実験" });
  // SPROUT は PATH の A だけで作る（Issue の検査は無い）
  assert.deepStrictEqual(ok(seBuildOp("add_sprout", { title: "題", paths: ["A1", "A2"] })), { op: "add_sprout", title: "題", paths: ["A1", "A2"] });
  assert.deepStrictEqual(ok(seBuildOp("add_sprout", { title: "題", body: "計画", paths: ["A2"] })), { op: "add_sprout", title: "題", paths: ["A2"], body: "計画" });
  assert.deepStrictEqual(ok(seBuildOp("copy", { node: "A2", to: "Q5" })), { op: "copy", node: "A2", to: "Q5" });
  assert.deepStrictEqual(ok(seBuildOp("remove", { node: "F3", reason: "誤り" })), { op: "remove", node: "F3" });
  assert.deepStrictEqual(ok(seBuildOp("remove", { node: "S1" })), { op: "remove", node: "S1" });
  assert.deepStrictEqual(ok(seBuildOp("edit", { node: "S2", text: "直した" })), { op: "edit", node: "S2", text: "直した" });
  assert.deepStrictEqual(ok(seBuildOp("edit", { node: "Q1", text: "新しい文" })), { op: "edit", node: "Q1", text: "新しい文" });
  assert.deepStrictEqual(ok(seBuildOp("edit", { node: "A1", body: "本文だけ" })), { op: "edit", node: "A1", body: "本文だけ" });
  // ★ はどの種類の Node にも付く（node で送る）
  assert.deepStrictEqual(ok(seBuildOp("star", { node: "A2" })), { op: "star", node: "A2" });
  assert.deepStrictEqual(ok(seBuildOp("star", { node: "Q1" })), { op: "star", node: "Q1" });
  assert.deepStrictEqual(ok(seBuildOp("unstar", { node: "F3" })), { op: "unstar", node: "F3" });
  assert.deepStrictEqual(ok(seBuildOp("star", { node: "S1" })), { op: "star", node: "S1" });
  // stance は F だけ
  assert.deepStrictEqual(ok(seBuildOp("set_stance", { node: "F3", stance: "D" })), { op: "set_stance", node: "F3", stance: "D" });
  // Q の間に Q を入れる・A を別の Q へ移す
  assert.deepStrictEqual(ok(seBuildOp("insert_q", { q: "Q1", text: "間の問い" })), { op: "insert_q", q: "Q1", text: "間の問い" });
  assert.deepStrictEqual(ok(seBuildOp("move_a", { node: "A2", to: "Q5" })), { op: "move_a", node: "A2", to: "Q5" });
  // MLflow の参照。空にすると外す（null）
  assert.deepStrictEqual(ok(seBuildOp("set_mlflow", { node: "E1", mlflow: "run-a" })), { op: "set_mlflow", node: "E1", mlflow: "run-a" });
  assert.deepStrictEqual(ok(seBuildOp("set_mlflow", { node: "E1", mlflow: "" })), { op: "set_mlflow", node: "E1", mlflow: null });
}

// --- 操作の NG の例 ---
function test_ops_ng() {
  // 必須・形
  ng(seBuildOp("add_q", { parent: "F1", text: "x" })); // 親は SEED・Q・A
  ng(seBuildOp("add_q", { parent: "E1", text: "x" }));
  ng(seBuildOp("add_q", { parent: "Q1", text: "" }));
  ng(seBuildOp("add_a", { q: "A1", text: "x" }));
  ng(seBuildOp("add_f", { parent: "Q1", stance: "S", text: "x" })); // 親は A か E
  ng(seBuildOp("add_e", { parent: "Q1", text: "x" }));
  ng(seBuildOp("add_e", { parent: "A2", text: "x" }));
  ng(seBuildOp("copy", { node: "A2", to: "A3" }));
  ng(seBuildOp("star", {}));
  ng(seBuildOp("set_stance", { node: "A2", stance: "S" })); // stance は F だけ
  ng(seBuildOp("set_stance", { node: "F3", stance: "" }));
  ng(seBuildOp("set_stance", { node: "F3", stance: "X" }));
  ng(seBuildOp("add_sprout", { title: "題", paths: [] }));
  ng(seBuildOp("add_sprout", { title: "題", paths: ["Q1"] }));
  ng(seBuildOp("add_sprout", { title: "題", paths: ["O1"] })); // O は無くなった ID
  ng(seBuildOp("add_sprout", { title: "題", paths: ["A1", "A1"] }));
  ng(seBuildOp("add_sprout", { title: "題", paths: Array.from({ length: 21 }, (_, i) => `A${i + 1}`) }));
  ng(seBuildOp("edit", { node: "Q1" })); // text も body も無い
  ng(seBuildOp("insert_q", { q: "A1", text: "x" }));
  ng(seBuildOp("insert_q", { q: "Q1", text: "" }));
  ng(seBuildOp("move_a", { node: "Q1", to: "Q5" }));
  ng(seBuildOp("move_a", { node: "A1" })); // 移す先が無い
  ng(seBuildOp("set_mlflow", { node: "Q1", mlflow: "run" }));
  // 字数
  ng(seBuildOp("add_q", { parent: "Q1", text: "あ".repeat(301) }));
  assert.ok(ok(seBuildOp("add_q", { parent: "Q1", text: "あ".repeat(300) })));
  ng(seBuildOp("add_a", { q: "Q1", text: "あ".repeat(121) }));
  ng(seBuildOp("add_e", { parent: "A1", text: "あ".repeat(121) }));
  ng(seBuildOp("add_sprout", { title: "あ".repeat(121), paths: ["A1"] }));
  ng(seBuildOp("add_sprout", { title: "題", body: "あ".repeat(2001), paths: ["A1"] }));
  ng(seBuildOp("insert_q", { q: "Q1", text: "あ".repeat(301) }));
  ng(seBuildOp("set_mlflow", { node: "E1", mlflow: "あ".repeat(201) }));
  // C-CITE
  ng(seBuildOp("add_f", { parent: "A2", stance: "S", text: "文", cites: ["37"] }));
  ng(seBuildOp("add_f", { parent: "A2", stance: "S", text: "文", cites: Array(11).fill("#37") }));
  // 1〜3 文
  ng(seBuildOp("add_f", { parent: "A2", stance: "S", text: "あ。い。う。え。" }));
  assert.ok(ok(seBuildOp("add_f", { parent: "A2", stance: "S", text: "あ。い。う。" })));
  // 数字と出典
  ng(seBuildOp("add_f", { parent: "A2", stance: "S", text: "精度は 0.9" }));
  // stance は必須（既定は無し）
  assert.match(ng(seBuildOp("add_f", { parent: "A2", text: "文" })), /stance/);
  // remove は理由を付けずに送れる
  assert.ok(ok(seBuildOp("remove", { node: "F3" })));
}

function test_unknown() {
  assert.strictEqual(ng(seBuildOp("mystery_op", {})), "Unknown operation");
  // 古い op は通らない（O の増減・Logicality・Objectivity・Answer/Issue は無くなった）
  for (const old of ["decide", "add_option", "add_child", "link", "add_finding"]) {
    assert.strictEqual(ng(seBuildOp(old, {})), "Unknown operation", old);
  }
}

function test_tables() {
  const keys = Object.keys(SE_OPS).sort();
  assert.strictEqual(keys.length, 14);
  assert.deepStrictEqual(Object.keys(SE_DONE).sort(), keys);
  for (const q of SE_QUICK) assert.ok(keys.includes(q));
  assert.deepStrictEqual([...SE_QUICK].sort(), ["set_stance", "star", "unstar"]);
  assert.deepStrictEqual(SE_STANCES.map((x) => x[0]), ["S", "D", "I"]);
  assert.ok(keys.includes("add_sprout") && keys.includes("insert_q") && keys.includes("move_a") && keys.includes("set_mlflow"));

  // カードのブロック（C9）: 20 本。消えた 7 本は出さない
  const ids = SE_BLOCKS.map(([id]) => id);
  assert.strictEqual(ids.length, 20);
  assert.strictEqual(new Set(ids).size, 20);
  for (const id of ids) assert.match(id, /^(sci|lab)-[a-z-]{2,30}$/);
  assert.ok(ids.includes("lab-read") && ids.includes("sci-verify") && ids.includes("lab-progress"));
  for (const gone of ["sci-read", "sci-split", "sci-search", "sci-ideate", "sci-debate", "sci-review", "sci-compare", "sci-draft", "sci-probe"]) {
    assert.ok(!ids.includes(gone), gone);
  }
}

// --- カードの初めの値（C-CARD）と検査 ---
function test_card() {
  const preset = {
    node: "Q12", block: "lab-grow", purpose: "「データの量は？」の答えの候補（A）を出す",
    expect: "A を 2 つ以上", scope: "path", assume: ["Q9=SST-2 を使う（仮）"], mode: "purpose_first",
    papers: [37], runner: "pi", depth: "full", origin_run: 120, child: "A",
  };
  // 範囲と仮置きは U22 で消した。preset に入っていても返さない・送らない。
  assert.deepStrictEqual(seCardValues(preset), {
    node: "Q12", block: "lab-grow", purpose: preset.purpose, expect: "A を 2 つ以上", runner: "pi",
    mode: "purpose_first", papers: [37], depth: "full", child: "A",
  });
  // 無ければ空（nodeId だけ入る）
  const empty = seCardValues(null, "Q3");
  assert.strictEqual(empty.node, "Q3");
  assert.strictEqual(empty.block, "");
  assert.strictEqual(empty.purpose, "");
  assert.strictEqual(empty.child, "");
  assert.deepStrictEqual(empty.papers, []);
  // 対象に合わない子の型は空にする（F は Q の子ではない）
  assert.strictEqual(seCardValues({ child: "F" }, "Q3").child, "");
  // user ならブロックの既定は lab-read
  assert.strictEqual(seCardValues({ runner: "user" }, "A2").block, "lab-read");

  const built = seBuildCard(seCardValues(preset));
  assert.strictEqual(built.ok, true);
  assert.deepStrictEqual(built.card, {
    node: "Q12", block: "lab-grow", purpose: preset.purpose, runner: "pi", papers: [37],
    expect: "A を 2 つ以上", child: "A",
  });
  // 範囲と仮置きを直接渡しても、送るカードには入らない。
  const stray = seBuildCard(Object.assign(seCardValues(preset), { scope: "blind", assume: ["Q1=あ"] }));
  assert.strictEqual(stray.ok, true);
  assert.ok(!("scope" in stray.card) && !("assume" in stray.card), JSON.stringify(stray.card));
  // 子の型が空なら送るカードに child は入らない
  const noChild = seBuildCard({ node: "Q3", block: "lab-grow", purpose: "目的", runner: "pi", child: "" });
  assert.ok(noChild.ok && !("child" in noChild.card), JSON.stringify(noChild.card));
  // 打ち込んだ番号（# 付き・重なり）
  assert.deepStrictEqual(seBuildCard(Object.assign(seCardValues(preset), { papers: [37, "#37", "40"] })).card.papers, [37, 40]);

  const bad = (over) => { const r = seBuildCard(Object.assign(seCardValues(preset), over)); assert.strictEqual(r.ok, false); return r.error; };
  bad({ purpose: " " });
  bad({ purpose: "あ".repeat(301) });
  bad({ expect: "あ".repeat(301) });
  bad({ block: "sci-draft" });
  bad({ block: "sci-read" }); // 消えた block
  bad({ papers: Array.from({ length: 21 }, (_, i) => i + 1) });
  bad({ papers: ["abc"] });
  // 足す子の型は対象に合うものだけ（Q なら Q か A、A なら Q か F）
  assert.match(bad({ node: "Q12", child: "F" }), /足す子の型/);
  assert.match(bad({ node: "A2", child: "A" }), /足す子の型/);
  assert.match(bad({ node: "SEED", child: "Q" }), /足す子の型/);
  assert.strictEqual(seBuildCard({ node: "A2", block: "lab-grow", purpose: "目的", runner: "pi", child: "F" }).card.child, "F");

  // READ_CARD: 論文はちょうど 1 本・対象は Q か A
  const read = { node: "A2", block: "lab-read", purpose: "読む", runner: "user", papers: [37], depth: "figures" };
  assert.deepStrictEqual(seBuildCard(read).card, { node: "A2", block: "lab-read", purpose: "読む", runner: "user", papers: [37], depth: "figures" });
  bad(Object.assign({}, read, { papers: [37, 38] }));
  bad(Object.assign({}, read, { papers: [] }));
  assert.match(bad(Object.assign({}, read, { node: "F4" })), /Q か A/);
  // READ_CARD は SEED を選べない
  assert.match(bad(Object.assign({}, read, { node: "SEED" })), /Q か A/);
  // mode は lab-read のときだけ送る
  assert.deepStrictEqual(seBuildCard({ node: "A2", block: "lab-read", purpose: "読む", runner: "pi", mode: "blind_first" }).card.mode, "blind_first");
  assert.ok(!("mode" in seBuildCard({ node: "A2", block: "lab-grow", purpose: "目的", runner: "pi", mode: "blind_first" }).card));
  // SEED（research-tree の全体）もカードの対象にできる
  assert.strictEqual(seCardValues(null, "SEED").node, "SEED");
  assert.deepStrictEqual(
    seBuildCard({ node: "SEED", block: "sci-digest", purpose: "木全体を見直す", runner: "pi" }).card,
    { node: "SEED", block: "sci-digest", purpose: "木全体を見直す", runner: "pi", papers: [] },
  );
}

// --- 足す子の型（対象の種類で選べる物）---
function test_child_options() {
  assert.deepStrictEqual(seChildOptions("Q5"), [["Q", "Q"], ["A", "A"]]);
  assert.deepStrictEqual(seChildOptions("A2"), [["Q", "Q"], ["F", "F"]]);
  assert.deepStrictEqual(seChildOptions("SEED"), []);
  assert.deepStrictEqual(seChildOptions("F1"), []);
  assert.deepStrictEqual(seChildOptions(""), []);
  assert.ok(seChildOk("Q1", "A"));
  assert.ok(seChildOk("A2", "F"));
  assert.ok(seChildOk("Q1", ""));
  assert.ok(!seChildOk("Q1", "F"));
  assert.ok(!seChildOk("A2", "A"));
  assert.ok(!seChildOk("SEED", "Q"));
}

// --- bottom_Q（child_Q を持たない Q。node の bottom_q を見る） ---
function test_bottom_qs() {
  const nodes = [
    { id: "Q1", parent: null, bottom_q: false },
    { id: "Q2", parent: "A1", bottom_q: true },
    { id: "A1", parent: "Q1", bottom_q: true },
    { id: "Q3", parent: "Q2", bottom_q: false },
  ];
  assert.deepStrictEqual(seBottomQs(nodes).map((n) => n.id), ["Q2"]);
  assert.deepStrictEqual(seBottomQs([]), []);
}

function test_response_shapes() {
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
  test_child_options();
  test_bottom_qs();
  test_response_shapes();
  console.log("test_sciedit: すべて通りました");
}

main();
