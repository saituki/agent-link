"use strict";
// 仕分けの画面（sciedit.js）の純関数のテスト（実装計画 U20「変更内容 4」）。
// 実行:
//     node apps/phone/tests/test_sciedit.js
//
// ここに無いもの（DOM が要る描画・小窓・仕分け箱の操作）はブラウザで確かめる
// （計画 U20 の AC1〜AC5 と C-17 の手順）。

const assert = require("assert");
const path = require("path");

const src = path.join(__dirname, "..", "src", "sai_phone", "static", "sciedit.js");
const {
  seTrim,
  seOpLabel,
  seCiteOk,
  seRefOk,
  seTargetOk,
  seCountSentences,
  seNeedsCite,
  seOptionStateLabel,
  seBuildOp,
  seProposalNodeKey,
  seProposalRunKey,
  seGroupProposals,
  seProposalSummary,
  seProposalEditKeys,
  seProposalList,
  seCardRunId,
  SE_OPS,
  SE_DONE,
  SE_ONE_TOUCH,
  SE_BLOCKS,
  SE_OPTION_STATES,
} = require(src);

/** seBuildOp が通ったときの JSON。 */
function ok(built) {
  assert.strictEqual(built.ok, true, `通るはずが弾かれました: ${built.error}`);
  assert.strictEqual(built.error, null);
  return built.json;
}

/** seBuildOp が弾いたときの理由。 */
function ng(built) {
  assert.strictEqual(built.ok, false, `弾かれるはず，通りました: ${JSON.stringify(built.json)}`);
  assert.strictEqual(built.json, null);
  assert.ok(built.error && typeof built.error === "string", "理由が日本語で返りません");
  return built.error;
}

// --- 小さい手 ---
function test_helpers() {
  assert.strictEqual(seTrim("  あ  "), "あ");
  assert.strictEqual(seTrim(null), "");
  assert.strictEqual(seTrim(undefined), "");

  assert.strictEqual(seOpLabel("add_finding"), "Add finding");
  assert.strictEqual(seOpLabel("decide"), "Decide");
  assert.strictEqual(seOpLabel("mystery_op"), "mystery_op"); // 表に無い op はそのまま

  assert.ok(seCiteOk("#37"));
  assert.ok(seCiteOk("#37 §3.2"));
  assert.ok(!seCiteOk("37"));
  assert.ok(!seCiteOk("#0"));
  assert.ok(!seCiteOk(""));

  assert.ok(seRefOk("N12", "N"));
  assert.ok(!seRefOk("12", "N")); // 数だけ受け付けない
  assert.ok(!seRefOk("N01", "N"));
  assert.ok(seRefOk("F310", "F"));
  assert.ok(!seRefOk("Q0", "Q"));

  assert.ok(seTargetOk("paper:37"));
  assert.ok(seTargetOk("report:R-0012"));
  assert.ok(seTargetOk("set:5"));
  assert.ok(!seTargetOk("paper:0"));
  assert.ok(!seTargetOk("candidate:abc"));
  assert.ok(!seTargetOk(""));

  assert.strictEqual(seOptionStateLabel("rejected"), "REJECTED");
  assert.strictEqual(seOptionStateLabel("dormant"), "DORMANT");
  assert.strictEqual(seOptionStateLabel("candidate"), "Back to CANDIDATE");
}

// --- 文の数を数える（C-07） ---
function test_sentences() {
  assert.strictEqual(seCountSentences("あ。"), 1);
  assert.strictEqual(seCountSentences("あ。い。う。"), 3);
  assert.strictEqual(seCountSentences("あ。い。う。え。"), 4); // 4 文は弾かれる
  assert.strictEqual(seCountSentences("これは 0.85 です"), 1); // 数字の点は数えない
  assert.strictEqual(seCountSentences("§3.2 に書いてある"), 1);
  assert.strictEqual(seCountSentences("あ？"), 1);
  assert.strictEqual(seCountSentences("あ!"), 1);
  assert.strictEqual(seCountSentences("This is one. And two."), 2);
  assert.strictEqual(seCountSentences("one. two."), 2);
  assert.strictEqual(seCountSentences(""), 0);
  assert.strictEqual(seCountSentences("   "), 0);
}

// --- 出典が要るか ---
function test_needs_cite() {
  assert.strictEqual(seNeedsCite("精度は 0.9 に達した", []), true);
  assert.strictEqual(seNeedsCite("精度は 0.9 に達した", ["#37 §3.2"]), false);
  assert.strictEqual(seNeedsCite("精度は 0.9 に達した", ["  "]), true);
  assert.strictEqual(seNeedsCite("発作部位の予測_headers", []), false);
  assert.strictEqual(seNeedsCite("", ["#37"]), false);
}

// --- decide ---
function test_decide() {
  assert.deepStrictEqual(
    ok(seBuildOp("decide", { node: "N12", option: "N20", reason: "根拠がそろった" })),
    { op: "decide", node: "N12", tentative: false, counter_searched: false, option: "N20", reason: "根拠がそろった" }
  );
  // 仮決定と反証を探した
  const tentative = ok(seBuildOp("decide", {
    node: "N12", option: "N20", tentative: true, counter_searched: true, reason: "もう少し",
  }));
  assert.strictEqual(tentative.tentative, true);
  assert.strictEqual(tentative.counter_searched, true);

  // 理由が空なら送らない
  assert.match(ng(seBuildOp("decide", { node: "N12", option: "N20" })), /reason/i);
  // 候補があるのに選んでいない
  assert.match(ng(seBuildOp("decide", { node: "N12", hasOptions: true, reason: "たぶん" })), /OPTION/);
  // 候補が無いときは空でもよい
  assert.ok(ok(seBuildOp("decide", { node: "N12", hasOptions: false, reason: "まだ無い" })).option === undefined);
  // 節点が無い
  assert.match(ng(seBuildOp("decide", { reason: "x" })), /ITEM/);
  // 根拠の知見
  const withEvidence = ok(seBuildOp("decide", { node: "N12", reason: "r", evidence: ["F31", " F32 ", ""] }));
  assert.deepStrictEqual(withEvidence.evidence, ["F31", "F32"]);
  assert.match(ng(seBuildOp("decide", { node: "N12", reason: "r", evidence: ["31"] })), /F31|format/);
  // 外す候補と理由
  const rejected = ok(seBuildOp("decide", {
    node: "N12", reason: "r", rejected: [{ option: "N21", reason: "別の目的" }, { option: "", reason: "—" }],
  }));
  assert.deepStrictEqual(rejected.rejected, [{ option: "N21", reason: "別の目的" }]);
  // 外すのに理由が空
  assert.match(ng(seBuildOp("decide", { node: "N12", reason: "r", rejected: [{ option: "N21", reason: "" }] })), /N21/);
}

// --- reopen / exclude_item / option_state ---
function test_simple_decisions() {
  assert.deepStrictEqual(ok(seBuildOp("reopen", { node: "N12" })), { op: "reopen", node: "N12" });
  assert.deepStrictEqual(ok(seBuildOp("exclude_item", { node: "N12", reason: "やめる" })), {
    op: "exclude_item", node: "N12", reason: "やめる",
  });
  assert.match(ng(seBuildOp("exclude_item", { node: "N12" })), /reason/i);

  // 候補の状態を変える（scitree.js が target に option と state を入れる）
  assert.deepStrictEqual(
    ok(seBuildOp("option_state", { node: "N12", reason: "違う" }, { option: "N20", state: "rejected" })),
    { op: "option_state", node: "N20", state: "rejected", reason: "違う" }
  );
  assert.deepStrictEqual(
    ok(seBuildOp("option_state", { node: "N12", reason: "戻る" }, { option: "N20", state: "candidate" })),
    { op: "option_state", node: "N20", state: "candidate", reason: "戻る" }
  );
  assert.match(ng(seBuildOp("option_state", { node: "N12", reason: "r" }, { option: "N20" })), /state/i);
  assert.match(ng(seBuildOp("option_state", { node: "N12", reason: "r" }, { option: "20", state: "rejected" })), /OPTION/);
  // C-07 の 3 つ以外の状態は受け付けない
  assert.match(ng(seBuildOp("option_state", { node: "N12", reason: "r" }, { option: "N20", state: "selected" })), /state/i);
}

// --- 項目・候補・問い・知見 ---
function test_content_ops() {
  assert.deepStrictEqual(ok(seBuildOp("add_option", { node: "N12", title: "前頭葉" })), {
    op: "add_option", node: "N12", title: "前頭葉",
  });
  assert.deepStrictEqual(ok(seBuildOp("add_option", { node: "N12", title: "前頭葉", body: "… " })), {
    op: "add_option", node: "N12", title: "前頭葉", body: "…",
  });
  assert.match(ng(seBuildOp("add_option", { node: "N12" })), /title/i);
  assert.match(ng(seBuildOp("add_option", { node: "N12", title: "あ".repeat(121) })), /120/);
  assert.match(ng(seBuildOp("add_option", { node: "N12", title: "x", body: "あ".repeat(2001) })), /2000/);

  assert.deepStrictEqual(ok(seBuildOp("add_child", { node: "N12", title: "手順", types: ["list", "verify"] })), {
    op: "add_child", node: "N12", title: "手順", types: ["list", "verify"],
  });
  assert.match(ng(seBuildOp("add_child", { node: "N12", title: "手順" })), /type/i);
  assert.match(ng(seBuildOp("add_child", { node: "N12", title: "手順", types: ["everything"] })), /type/i);

  assert.deepStrictEqual(ok(seBuildOp("add_question", { node: "N12", text: "N はいくつか" })), {
    op: "add_question", node: "N12", text: "N はいくつか",
  });
  assert.match(ng(seBuildOp("add_question", { node: "N12" })), /QUESTION/);
  assert.match(ng(seBuildOp("add_question", { node: "N12", text: "あ".repeat(301) })), /300/);

  assert.deepStrictEqual(ok(seBuildOp("answer_question", { node: "N12", text: "3 人" }, "Q7")), {
    op: "answer_question", question: "Q7", text: "3 人",
  });
  assert.match(ng(seBuildOp("answer_question", { text: "3 人" }, null)), /QUESTION/);
  assert.match(ng(seBuildOp("answer_question", { text: "" }, "Q7")), /answer/i);

  // 知見
  const f = ok(seBuildOp("add_finding", {
    node: "N12", text: "前頭葉の 3 例で改善が見られた。", cites: ["#37 §3.2"], polarity: "refute", level: "3",
  }));
  assert.deepStrictEqual(f, {
    op: "add_finding", node: "N12", text: "前頭葉の 3 例で改善が見られた。", cites: ["#37 §3.2"], polarity: "refute", level: 3,
  });
  // 数字があっても出典があれば通る
  assert.ok(ok(seBuildOp("add_finding", { node: "N12", text: "1 例だけ", cites: ["#37"] })));
  // 数字があって出典が無い
  assert.match(ng(seBuildOp("add_finding", { node: "N12", text: "1 例だけ", cites: [] })), /CITE/);
  // 数字が無くても出典は要らない
  assert.ok(ok(seBuildOp("add_finding", { node: "N12", text: "改善が見られた。", cites: [] })));
  // 出典の形
  assert.match(ng(seBuildOp("add_finding", { node: "N12", text: "あ", cites: ["37"] })), /CITE/);
  // 4 文は弾かれる
  assert.match(ng(seBuildOp("add_finding", { node: "N12", text: "あ。い。う。え。" })), /3 sentences/);
  // 出典は 10 個まで
  assert.match(ng(seBuildOp("add_finding", { node: "N12", text: "あ", cites: Array(11).fill("#1") })), /10 CITE/);
  // 任意の項目
  const full = ok(seBuildOp("add_finding", {
    node: "N12", text: "あ", question: "Q7", paper: "37", run: "130", level: "4",
  }));
  assert.strictEqual(full.question, "Q7");
  assert.strictEqual(full.paper, 37);
  assert.strictEqual(full.run, 130);
  assert.strictEqual(full.level, 4);
}

// --- 結ぶ / 文を直す / 線 ---
function test_link_text_edge() {
  assert.deepStrictEqual(ok(seBuildOp("link", { node: "N12", target: "paper:37", role: "exemplar" })), {
    op: "link", node: "N12", target: "paper:37", role: "exemplar",
  });
  assert.deepStrictEqual(
    ok(seBuildOp("link", { node: "N12", target: "paper:37", role: "compare", read_state: "read" })),
    { op: "link", node: "N12", target: "paper:37", role: "compare", read_state: "read" }
  );
  // 論文のときは役割が必須
  assert.match(ng(seBuildOp("link", { node: "N12", target: "paper:37" })), /ROLE/);
  // 論文でなければ役割は不要
  assert.deepStrictEqual(ok(seBuildOp("link", { node: "N12", target: "report:R-0012" })), {
    op: "link", node: "N12", target: "report:R-0012",
  });
  assert.match(ng(seBuildOp("link", { node: "N12", target: "paper:abc" })), /format/);
  assert.match(ng(seBuildOp("link", { node: "N12", target: "" })), /target/i);

  assert.deepStrictEqual(ok(seBuildOp("edit_text", { node: "N12", title: "小目的", body: "本文" })), {
    op: "edit_text", node: "N12", title: "小目的", body: "本文",
  });
  // どちらかが 1 つは必要
  assert.match(ng(seBuildOp("edit_text", { node: "N12", title: "", body: " " })), /title or a body/);
  assert.match(ng(seBuildOp("edit_text", { node: "N12", title: "あ".repeat(121) })), /120/);

  assert.deepStrictEqual(ok(seBuildOp("add_edge", { node: "N11", kind: "depends", to: "N30", note: "要る" })), {
    op: "add_edge", kind: "depends", from: "N11", to: "N30", note: "要る",
  });
  // parent は指定できない
  assert.match(ng(seBuildOp("add_edge", { node: "N11", kind: "parent", to: "N30" })), /edge kind/i);
  // 自分自身
  assert.match(ng(seBuildOp("add_edge", { node: "N11", kind: "depends", to: "N11" })), /itself/);
  assert.match(ng(seBuildOp("add_edge", { node: "N11", kind: "depends" })), /target/i);
}

// --- 外す系（1 押し）と読み具合 ---
function test_remove_ops() {
  assert.deepStrictEqual(ok(seBuildOp("remove_finding", {}, "F31")), { op: "remove_finding", finding: "F31" });
  assert.match(ng(seBuildOp("remove_finding", {}, "31")), /FINDING/);
  assert.deepStrictEqual(ok(seBuildOp("unlink", {}, "L3")), { op: "unlink", link: "L3" });
  assert.deepStrictEqual(ok(seBuildOp("withdraw_question", {}, "Q7")), { op: "withdraw_question", question: "Q7" });
  assert.deepStrictEqual(
    ok(seBuildOp("remove_edge", {}, { kind: "depends", from: "N11", to: "N30" })),
    { op: "remove_edge", kind: "depends", from: "N11", to: "N30" }
  );
  assert.match(ng(seBuildOp("remove_edge", {}, { kind: "parent", from: "N11", to: "N30" })), /edge kind/i);
  assert.match(ng(seBuildOp("remove_edge", {}, { kind: "depends", from: "N11" })), /Both ends/);

  assert.deepStrictEqual(ok(seBuildOp("set_read_state", { state: "reflected" }, "L3")), {
    op: "set_read_state", link: "L3", state: "reflected",
  });
  assert.match(ng(seBuildOp("set_read_state", { state: "unreaded" }, "L3")), /READ_STATE/);

  // 外す（墓標）
  assert.deepStrictEqual(
    ok(seBuildOp("exclude", { node: "N12", target: "paper:37", title: "Scalp EEG", reason: "主題が違う" })),
    { op: "exclude", target: "paper:37", node: "N12", title: "Scalp EEG", reason: "主題が違う" }
  );
  assert.match(ng(seBuildOp("exclude", { node: "N12", target: "paper:37" })), /reason/i);
  assert.match(ng(seBuildOp("exclude", { node: "N12", target: "", reason: "x" })), /target/i);
}

// --- 知らない op ---
function test_unknown() {
  assert.match(ng(seBuildOp("add_seed", { node: "N1", title: "x" })), /Unknown/);
  assert.match(ng(seBuildOp("", {})), /Unknown/);
}

// --- 19 の op のうち Web から使える 18 が全部定義されていること ---
function test_all_ops() {
  // C-07 の op の一覧（add_seed 以外は Web から使える）
  const web = [
    "add_finding", "add_question", "answer_question", "link", "add_option", "add_child", "decide",
    "reopen", "exclude_item", "option_state", "exclude", "edit_text", "add_edge", "unlink",
    "remove_finding", "set_read_state", "withdraw_question", "remove_edge",
  ];
  assert.strictEqual(web.length, 18);
  for (const op of web) {
    assert.ok(Object.prototype.hasOwnProperty.call(SE_OPS, op), `SE_OPS に ${op} がありません`);
    assert.ok(Object.prototype.hasOwnProperty.call(SE_DONE, op), `SE_DONE に ${op} がありません`);
  }
  // 1 押しで外せる 4 つ
  for (const op of ["remove_finding", "unlink", "withdraw_question", "remove_edge"]) {
    assert.ok(SE_ONE_TOUCH[op], `${op} は 1 押しのはずです`);
  }
  assert.strictEqual(Object.keys(SE_ONE_TOUCH).length, 4);
  // 候補の状態は C-07 の 3 つ
  assert.deepStrictEqual(SE_OPTION_STATES.map((x) => x[0]), ["rejected", "dormant", "candidate"]);
  // 16 本のブロック（C-16）
  assert.strictEqual(SE_BLOCKS.length, 16);
  for (const [id, name] of SE_BLOCKS) {
    assert.match(id, /^sci-[a-z-]{2,30}$/, name);
  }
  assert.ok(SE_BLOCKS.some(([id]) => id === "sci-read"));
  assert.ok(SE_BLOCKS.some(([id]) => id === "sci-seed"));
}

// --- 仕分けのまとめ方 ---
function test_group_proposals() {
  const list = [
    { id: "P60", op: "add_finding", run_id: 121, node: "N12", payload: {}, reason: "r", status: "pending" },
    { id: "P59", op: "add_option", run_id: 121, node: "N12", payload: { title: "前頭葉" }, reason: "r", status: "pending" },
    { id: "P58", op: "link", run_id: 120, node: "N9", payload: { target: "paper:37" }, reason: "r", status: "pending" },
  ];
  const byNode = seGroupProposals(list, "node");
  assert.deepStrictEqual(byNode.map((g) => g.key), ["N12", "N9"]);
  assert.deepStrictEqual(byNode.map((g) => g.items.length), [2, 1]);
  assert.strictEqual(byNode[0].label, "N12");

  const byRun = seGroupProposals(list, "run");
  assert.deepStrictEqual(byRun.map((g) => g.key), ["121", "120"]);
  assert.strictEqual(byRun[0].label, "run 121");
  assert.deepStrictEqual(byRun[0].items.map((p) => p.id), ["P60", "P59"]);

  // node_id が数で来ても N12 と同じ扱いにする
  assert.strictEqual(seProposalNodeKey({ node_id: 12 }), "N12");
  assert.strictEqual(seProposalNodeKey({ node: "N12" }), "N12");
  assert.strictEqual(seProposalNodeKey({ node: 12 }), ""); // 数だけは受け付けない
  assert.strictEqual(seProposalNodeKey({}), "");
  assert.strictEqual(seProposalRunKey({ run_id: 120 }), "120");
  assert.strictEqual(seProposalRunKey({ run_id: 0 }), "");
  assert.strictEqual(seProposalRunKey({}), "");

  // 節点の無い提案は「節点なし」にまとめる
  const orphan = seGroupProposals([{ id: "P1", op: "add_seed" }], "node");
  assert.deepStrictEqual(orphan.map((g) => g.key), [""]);
  assert.strictEqual(orphan[0].label, "NODE なし");
  const orphanRun = seGroupProposals([{ id: "P1", op: "add_seed" }], "run");
  assert.strictEqual(orphanRun[0].label, "run 不明");

  // 題があれば付ける
  const titled = seGroupProposals([{ id: "P1", node: "N12", node_title: "小目的" }], "node");
  assert.strictEqual(titled[0].label, "N12 小目的");

  // 空でも壊れない
  assert.deepStrictEqual(seGroupProposals([], "node"), []);
  assert.deepStrictEqual(seGroupProposals(null, "node"), []);
  assert.deepStrictEqual(seGroupProposals([null, undefined], "node"), []);
}

// --- 提案の要約と直したとき ---
function test_proposal_text() {
  const p = { id: "P59", op: "add_option", payload: { node: "N12", title: "前頭葉", body: "…" }, reason: "候補が要る" };
  assert.strictEqual(seProposalSummary(p), "前頭葉　…");
  assert.deepStrictEqual(seProposalEditKeys(p), ["title", "body"]);
  assert.deepStrictEqual(seProposalEditKeys({ payload: { title: "a", text: "b" } }), ["title", "text"]);
  assert.deepStrictEqual(seProposalEditKeys({ payload: { target: "paper:37" } }), []);
  // 長い要約は 90 文字で切る
  const long = { payload: { title: "あ".repeat(200) } };
  assert.ok(seProposalSummary(long).length <= 91);
  assert.ok(seProposalSummary({}).includes("中身がありません"));
}

// --- 応答の形 ---
function test_response_shapes() {
  assert.deepStrictEqual(seProposalList([{ id: "P1" }]), [{ id: "P1" }]);
  assert.deepStrictEqual(seProposalList({ proposals: [{ id: "P1" }] }), [{ id: "P1" }]);
  assert.deepStrictEqual(seProposalList({ pending: [{ id: "P1" }] }), [{ id: "P1" }]);
  assert.deepStrictEqual(seProposalList(null), []);
  assert.deepStrictEqual(seProposalList({}), []);

  assert.strictEqual(seCardRunId({ run_id: 130 }), 130);
  assert.strictEqual(seCardRunId({ id: 7 }), 7);
  assert.strictEqual(seCardRunId({ run: { run_id: 9 } }), 9);
  assert.strictEqual(seCardRunId(null), null);
  assert.strictEqual(seCardRunId({}), null);
}

// --- 実行 ---
function main() {
  test_helpers();
  test_sentences();
  test_needs_cite();
  test_decide();
  test_simple_decisions();
  test_content_ops();
  test_link_text_edge();
  test_remove_ops();
  test_unknown();
  test_all_ops();
  test_group_proposals();
  test_proposal_text();
  test_response_shapes();
  console.log("test_sciedit: すべて通りました");
}

main();
