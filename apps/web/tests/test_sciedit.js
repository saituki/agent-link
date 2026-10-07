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
  seCardValues, seBuildCard,
  seProposalSummary, seProposalEditKeys, seProposalList, seCardRunId,
  seBottomQs,
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
  assert.strictEqual(seOpLabel("add_sprout"), "SPROUT を作る");
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
  assert.deepStrictEqual(ok(seBuildOp("add_q", { parent: "SEED", text: "最初の問い" })), { op: "add_q", parent: "SEED", text: "最初の問い" });
  assert.deepStrictEqual(ok(seBuildOp("add_q", { parent: "O2", text: "x", reason: "r" })), { op: "add_q", parent: "O2", text: "x" });
  assert.deepStrictEqual(ok(seBuildOp("add_o", { q: "Q1", text: "SST-2", kind: "issue", body: "b" })), { op: "add_o", q: "Q1", text: "SST-2", body: "b", kind: "issue" });
  assert.deepStrictEqual(ok(seBuildOp("add_f", { parent: "O2", text: "精度は 0.9。", cites: ["#37 §3.2", " "], paper: "37" })), { op: "add_f", parent: "O2", text: "精度は 0.9。", cites: ["#37 §3.2"], paper: 37 });
  assert.deepStrictEqual(ok(seBuildOp("add_f", { parent: "E1", text: "出典なしの文" })), { op: "add_f", parent: "E1", text: "出典なしの文" });
  assert.deepStrictEqual(ok(seBuildOp("add_e", { parent: "E1", text: "実験", mlflow: "run-a" })), { op: "add_e", parent: "E1", text: "実験", mlflow: "run-a" });
  assert.deepStrictEqual(ok(seBuildOp("add_e", { parent: "S2", text: "実験" })), { op: "add_e", parent: "S2", text: "実験" });
  assert.deepStrictEqual(ok(seBuildOp("record_logic", { q: "Q1", value: "ok" })), { op: "record_logic", q: "Q1", value: "ok" });
  assert.deepStrictEqual(
    ok(seBuildOp("record_logic", { q: "Q1", value: "contradiction", note: "食い違う", findings: ["F1", "F2"] })),
    { op: "record_logic", q: "Q1", value: "contradiction", note: "食い違う", findings: ["F1", "F2"] },
  );
  assert.deepStrictEqual(ok(seBuildOp("add_sprout", { title: "題", paths: ["O1", "O2"] })), { op: "add_sprout", title: "題", paths: ["O1", "O2"] });
  assert.deepStrictEqual(ok(seBuildOp("add_sprout", { title: "題", body: "計画", paths: ["O2"] })), { op: "add_sprout", title: "題", paths: ["O2"], body: "計画" });
  const sproutNodes = [
    { id: "Q1", parent: null, kind: "Q" },
    { id: "O1", parent: "Q1", kind: "O", answer_kind: "answer" },
    { id: "O2", parent: "Q1", kind: "O", answer_kind: "issue" },
  ];
  assert.deepStrictEqual(
    ok(seBuildOp("add_sprout", { title: "題", paths: ["O1", "O2"], nodes: sproutNodes })),
    { op: "add_sprout", title: "題", paths: ["O1", "O2"] },
  );
  assert.deepStrictEqual(ok(seBuildOp("copy", { node: "O2", to: "Q5" })), { op: "copy", node: "O2", to: "Q5" });
  assert.deepStrictEqual(ok(seBuildOp("remove", { node: "F3", reason: "誤り" })), { op: "remove", node: "F3" });
  assert.deepStrictEqual(ok(seBuildOp("remove", { node: "S1" })), { op: "remove", node: "S1" });
  assert.deepStrictEqual(ok(seBuildOp("edit", { node: "S2", text: "直した" })), { op: "edit", node: "S2", text: "直した" });
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
  ng(seBuildOp("add_q", { parent: "F1", text: "x" })); // 親は SEED か Q か O
  ng(seBuildOp("add_q", { parent: "E1", text: "x" }));
  ng(seBuildOp("add_q", { parent: "Q1", text: "" }));
  ng(seBuildOp("add_o", { q: "O1", text: "x" }));
  ng(seBuildOp("add_o", { q: "Q1", text: "x", kind: "other" }));
  ng(seBuildOp("add_f", { parent: "Q1", text: "x" })); // 親は O か E
  ng(seBuildOp("add_e", { parent: "Q1", text: "x" }));
  ng(seBuildOp("add_e", { parent: "O2", text: "x" }));
  ng(seBuildOp("copy", { node: "O2", to: "O3" }));
  ng(seBuildOp("star", {}));
  ng(seBuildOp("set_kind", { o: "O2", kind: "" }));
  ng(seBuildOp("add_sprout", { title: "題", paths: [] }));
  ng(seBuildOp("add_sprout", { title: "題", paths: ["Q1"] }));
  ng(seBuildOp("add_sprout", { title: "題", paths: ["O1", "O1"] }));
  ng(seBuildOp("add_sprout", { title: "題", paths: Array.from({ length: 21 }, (_, i) => `O${i + 1}`) }));
  ng(seBuildOp("add_sprout", { title: "題", paths: ["O1"], nodes: [{ id: "O1", answer_kind: "answer" }] })); // Issue なし
  ng(seBuildOp("edit", { node: "Q1" })); // text も body も無い
  // 字数
  ng(seBuildOp("add_q", { parent: "Q1", text: "あ".repeat(301) }));
  assert.ok(ok(seBuildOp("add_q", { parent: "Q1", text: "あ".repeat(300) })));
  ng(seBuildOp("add_o", { q: "Q1", text: "あ".repeat(121) }));
  ng(seBuildOp("add_e", { parent: "O1", text: "あ".repeat(121) }));
  ng(seBuildOp("add_sprout", { title: "あ".repeat(121), paths: ["O1"] }));
  ng(seBuildOp("add_sprout", { title: "題", body: "あ".repeat(2001), paths: ["O1"] }));
  // C-CITE
  ng(seBuildOp("add_f", { parent: "O2", text: "文", cites: ["37"] }));
  ng(seBuildOp("add_f", { parent: "O2", text: "文", cites: Array(11).fill("#37") }));
  // 1〜3 文
  ng(seBuildOp("add_f", { parent: "O2", text: "あ。い。う。え。" }));
  assert.ok(ok(seBuildOp("add_f", { parent: "O2", text: "あ。い。う。" })));
  // 数字と出典
  ng(seBuildOp("add_f", { parent: "O2", text: "精度は 0.9" }));
  // remove は理由を付けずに送れる
  assert.ok(ok(seBuildOp("remove", { node: "F3" })));
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
  assert.ok(keys.includes("add_sprout"));
  for (const gone of ["sci-draft", "sci-probe", "sci-outline", "sci-tree-review"]) assert.ok(!ids.includes(gone), gone);
}

// --- カードの初めの値（C-CARD）と検査 ---
function test_card() {
  const preset = {
    node: "Q12", block: "sci-ideate", purpose: "「データの量は？」の答えの候補（O）を出す",
    expect: "O を 2 つ以上", scope: "path", assume: ["Q9=SST-2 を使う（仮）"], mode: "purpose_first",
    papers: [37], runner: "pi", depth: "full", origin_run: 120,
  };
  // 範囲と仮置きは U22 で消した。preset に入っていても返さない・送らない。
  assert.deepStrictEqual(seCardValues(preset), {
    node: "Q12", block: "sci-ideate", purpose: preset.purpose, expect: "O を 2 つ以上", runner: "pi",
    mode: "purpose_first", papers: [37], depth: "full",
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
    expect: "O を 2 つ以上",
  });
  // 範囲と仮置きを直接渡しても、送るカードには入らない。
  const stray = seBuildCard(Object.assign(seCardValues(preset), { scope: "blind", assume: ["Q1=あ"] }));
  assert.strictEqual(stray.ok, true);
  assert.ok(!("scope" in stray.card) && !("assume" in stray.card), JSON.stringify(stray.card));
  // 打ち込んだ番号（# 付き・重なり）
  assert.deepStrictEqual(seBuildCard(Object.assign(seCardValues(preset), { papers: [37, "#37", "40"] })).card.papers, [37, 40]);

  const bad = (over) => { const r = seBuildCard(Object.assign(seCardValues(preset), over)); assert.strictEqual(r.ok, false); return r.error; };
  bad({ purpose: " " });
  bad({ purpose: "あ".repeat(301) });
  bad({ expect: "あ".repeat(301) });
  bad({ block: "sci-draft" });
  bad({ papers: Array.from({ length: 21 }, (_, i) => i + 1) });
  bad({ papers: ["abc"] });

  // 読む札: 論文はちょうど 1 本・対象は Q か O
  const read = { node: "O2", block: "sci-read", purpose: "読む", runner: "user", papers: [37], depth: "figures" };
  assert.deepStrictEqual(seBuildCard(read).card, { node: "O2", block: "sci-read", purpose: "読む", runner: "user", papers: [37], depth: "figures" });
  bad(Object.assign({}, read, { papers: [37, 38] }));
  bad(Object.assign({}, read, { papers: [] }));
  assert.match(bad(Object.assign({}, read, { node: "F4" })), /Q か O/);
  // 読む札は SEED を選べない
  assert.match(bad(Object.assign({}, read, { node: "SEED" })), /Q か O/);
  // SEED（research-tree の全体）もカードの対象にできる
  assert.strictEqual(seCardValues(null, "SEED").node, "SEED");
  assert.deepStrictEqual(
    seBuildCard({ node: "SEED", block: "sci-review", purpose: "木全体を見直す", runner: "pi" }).card,
    { node: "SEED", block: "sci-review", purpose: "木全体を見直す", runner: "pi", papers: [] },
  );
}

// --- 仕分け ---
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

// --- bottom_Q（子を持たない Q。node の bottom_q を見る） ---
function test_bottom_qs() {
  const nodes = [
    { id: "Q1", parent: null, bottom_q: false },
    { id: "Q2", parent: "O1", bottom_q: true },
    { id: "O1", parent: "Q1", bottom_q: true },
    { id: "Q3", parent: "Q2", bottom_q: false },
  ];
  assert.deepStrictEqual(seBottomQs(nodes).map((n) => n.id), ["Q2"]);
  assert.deepStrictEqual(seBottomQs([]), []);
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
  test_bottom_qs();
  test_proposal_text();
  test_response_shapes();
  console.log("test_sciedit: すべて通りました");
}

main();
