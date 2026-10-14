"use strict";
// 操作の入力欄・カードのフォーム（sciedit.js）の純関数のテスト（実装計画 U19・U06 で Q・A・F・E・S に合わせた）。
// 実行:
//     node apps/web/tests/test_sciedit.js
//
// DOM が要る描画・小窓の操作はブラウザで確かめる（kantoku が統合のあとで行う）。

const assert = require("assert");
const path = require("path");

const src = path.join(__dirname, "..", "src", "sai_web", "static", "sciedit.js");
// 画面の軸の判定（ui.js の saiIsAxis）。ブラウザでは一番外側の名前として読まれるので、テストでも同じ名前にする
global.saiIsAxis = require(require("path").join(__dirname, "..", "src", "sai_web", "static", "ui.js")).saiIsAxis;
const {
  seTrim, seOpLabel, seCiteOk, seRefOk, seIdOk, seCountSentences, seNeedsCite, seEditRules, seBuildOp,
  seCardValues, seBuildCard, seCardRunId,
  seChildOptions, seChildOk,
  seParents, seMoveTargets, seCardNode,
  SE_OPS, SE_DONE, SE_QUICK, SE_BLOCKS, SE_STANCES, SE_SEED_AXES, SE_SQ_RE, SE_SA_RE, SE_RELS,
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
  // 表示名（A を足す・A を移す。廃止した op は表に無く、名前のまま返る）
  assert.strictEqual(seOpLabel("add_a"), "A を足す");
  assert.strictEqual(seOpLabel("add_f"), "Add F");
  assert.strictEqual(seOpLabel("add_e"), "Add E");
  assert.strictEqual(seOpLabel("copy"), "Copy A");
  assert.strictEqual(seOpLabel("edit"), "Edit");
  assert.strictEqual(seOpLabel("remove"), "Delete");
  assert.strictEqual(seOpLabel("add_sprout"), "Create SPROUT");
  assert.strictEqual(seOpLabel("add_seed"), "seed（D）を作る");
  assert.strictEqual(seOpLabel("move_node"), "A を移す");
  assert.strictEqual(seOpLabel("add_q"), "add_q");
  assert.strictEqual(seOpLabel("regroup"), "regroup");
  assert.strictEqual(seOpLabel("mystery_op"), "mystery_op");
  assert.ok(seRefOk("Q12", "Q"));
  assert.ok(seRefOk("E3", "QAFE"));
  assert.ok(seRefOk("S4"));
  assert.ok(seRefOk("D1") && seRefOk("D1", "D") && seRefOk("D1", "QAFESD"));
  assert.ok(!seRefOk("D1", "QAFES") && !seRefOk("d1") && !seRefOk("D0"));
  assert.ok(!seRefOk("A3", "Q"));
  assert.ok(!seRefOk("O3"));
  assert.ok(!seRefOk("N12"));
  assert.ok(!seRefOk("Q01"));
  assert.strictEqual(seCountSentences("あ。い。う。え。"), 4);
  assert.strictEqual(seCountSentences("これは 0.85 です"), 1);
  assert.strictEqual(seNeedsCite("精度は 0.9", []), true);
  assert.strictEqual(seNeedsCite("精度は 0.9", ["#37"]), false);
  // 報告書（report）でも数字の文は通る
  assert.strictEqual(seNeedsCite("精度は 0.9", [], "R-0001"), false);
}

// --- C-CITE ---
function test_cite() {
  for (const c of ["#37", "#37.3", "#37.3.2", "#37.3.2-4", "#37.3.2 注", "#37 §3.2"]) assert.ok(seCiteOk(c), c);
  for (const c of ["37", "#0", "", "#37.0", "#37.1.2.3", "#37." + "x"]) assert.ok(!seCiteOk(c), c);
}

// --- 操作の通る例 ---
function test_ops_ok() {
  // add_a: 親は A（軸の A の下も A の下も可）。rel は subtype か condition。空の rel と body は送らない
  assert.deepStrictEqual(ok(seBuildOp("add_a", { parent: "A91", text: "SST-2", body: "b" })), { op: "add_a", parent: "A91", text: "SST-2", body: "b" });
  assert.deepStrictEqual(ok(seBuildOp("add_a", { parent: "A2", text: "下位の答え", rel: "subtype" })), { op: "add_a", parent: "A2", text: "下位の答え", rel: "subtype" });
  assert.deepStrictEqual(ok(seBuildOp("add_a", { parent: "A2", text: "条件", rel: "condition", body: "" })), { op: "add_a", parent: "A2", text: "条件", rel: "condition" });
  assert.deepStrictEqual(ok(seBuildOp("add_a", { parent: "A91", text: "x", rel: " " })), { op: "add_a", parent: "A91", text: "x" });
  assert.deepStrictEqual(ok(seBuildOp("add_a", { parent: "A2", text: "x", reason: "r" })), { op: "add_a", parent: "A2", text: "x" });
  // move_node は A を 1 件・移す先は A（軸の A も可）
  assert.deepStrictEqual(ok(seBuildOp("move_node", { node: "A4", to: "A93" })), { op: "move_node", node: "A4", to: "A93" });
  assert.deepStrictEqual(ok(seBuildOp("move_node", { node: "A4", to: "A3" })), { op: "move_node", node: "A4", to: "A3" });
  // F は stance が要る。出典と論文番号は任意
  assert.deepStrictEqual(
    ok(seBuildOp("add_f", { parent: "A2", stance: "support", text: "精度は 0.9。", cites: ["#37 §3.2", " "], paper: "37" })),
    { op: "add_f", parent: "A2", stance: "support", text: "精度は 0.9。", cites: ["#37 §3.2"], paper: 37 },
  );
  assert.deepStrictEqual(ok(seBuildOp("add_f", { parent: "A1", stance: "issue", text: "出典なしの文" })), { op: "add_f", parent: "A1", stance: "issue", text: "出典なしの文" });
  assert.deepStrictEqual(ok(seBuildOp("add_e", { parent: "E1", text: "実験", mlflow: "run-a" })), { op: "add_e", parent: "E1", text: "実験", mlflow: "run-a" });
  assert.deepStrictEqual(ok(seBuildOp("add_e", { parent: "S2", text: "実験" })), { op: "add_e", parent: "S2", text: "実験" });
  // E のデータセット（任意・空は送らない）。sai-ts の値はそのまま通す（形は engine が見ない）
  assert.deepStrictEqual(ok(seBuildOp("add_e", { parent: "S2", text: "実験", dataset: " ts:hfo-r01-v1@0123456789ab " })), { op: "add_e", parent: "S2", text: "実験", dataset: "ts:hfo-r01-v1@0123456789ab" });
  assert.deepStrictEqual(ok(seBuildOp("add_e", { parent: "S2", text: "実験", dataset: "" })), { op: "add_e", parent: "S2", text: "実験" });
  assert.strictEqual(seOpLabel("set_dataset"), "データセットを変える");
  // seed（D）は題だけで作れる（本文は任意）。SPROUT は seed と題で作り、回答は後から足す（paths は無い）
  assert.deepStrictEqual(ok(seBuildOp("add_seed", { title: "発作部位の特定" })), { op: "add_seed", title: "発作部位の特定" });
  assert.deepStrictEqual(ok(seBuildOp("add_seed", { title: "題", body: "背景と制約" })), { op: "add_seed", title: "題", body: "背景と制約" });
  assert.deepStrictEqual(ok(seBuildOp("add_sprout", { seed: "D1", title: "題" })), { op: "add_sprout", seed: "D1", title: "題" });
  assert.deepStrictEqual(ok(seBuildOp("add_sprout", { seed: "D2", title: "題", body: "計画", paths: ["A2"] })), { op: "add_sprout", seed: "D2", title: "題", body: "計画" });
  // 問い: 足す（軸は任意。空の軸は付けない）・直す（軸の空は「外す」で送る）・消す
  assert.deepStrictEqual(ok(seBuildOp("add_seed_q", { seed: "D1", text: "どの手法を使うか", axis: "method" })), { op: "add_seed_q", seed: "D1", text: "どの手法を使うか", axis: "method" });
  assert.deepStrictEqual(ok(seBuildOp("add_seed_q", { seed: "D1", text: "成功基準は何か", axis: "" })), { op: "add_seed_q", seed: "D1", text: "成功基準は何か" });
  assert.deepStrictEqual(ok(seBuildOp("edit_seed_q", { q: "SQ3", text: "直した問い", axis: "" })), { op: "edit_seed_q", q: "SQ3", text: "直した問い", axis: "" });
  assert.deepStrictEqual(ok(seBuildOp("edit_seed_q", { q: "SQ3", text: "直した問い", axis: "data" })), { op: "edit_seed_q", q: "SQ3", text: "直した問い", axis: "data" });
  assert.deepStrictEqual(ok(seBuildOp("remove_seed_q", { q: "SQ3" })), { op: "remove_seed_q", q: "SQ3" });
  // 回答: A を選ぶか、文を入れるか（どちらか一方）。持ち主は D か S
  assert.deepStrictEqual(ok(seBuildOp("add_answer", { to: "D1", q: "SQ3", text: "テキストの回答" })), { op: "add_answer", to: "D1", q: "SQ3", text: "テキストの回答" });
  assert.deepStrictEqual(ok(seBuildOp("add_answer", { to: "S2", q: "SQ5", a: "A12" })), { op: "add_answer", to: "S2", q: "SQ5", a: "A12" });
  assert.deepStrictEqual(ok(seBuildOp("edit_answer", { answer: "SA7", text: "直した回答" })), { op: "edit_answer", answer: "SA7", text: "直した回答" });
  assert.deepStrictEqual(ok(seBuildOp("remove_answer", { answer: "SA7" })), { op: "remove_answer", answer: "SA7" });
  assert.strictEqual(seOpLabel("add_answer"), "回答を足す");
  // D は remove・star・edit の対象
  assert.deepStrictEqual(ok(seBuildOp("remove", { node: "D1" })), { op: "remove", node: "D1" });
  assert.deepStrictEqual(ok(seBuildOp("star", { node: "D1" })), { op: "star", node: "D1" });
  assert.deepStrictEqual(ok(seBuildOp("edit", { node: "D1", text: "題", body: "## 背景\n空欄" })), { op: "edit", node: "D1", text: "題", body: "## 背景\n空欄" });
  assert.deepStrictEqual(ok(seBuildOp("copy", { node: "A2", to: "A95" })), { op: "copy", node: "A2", to: "A95" });
  assert.deepStrictEqual(ok(seBuildOp("copy", { node: "A2", to: "A5" })), { op: "copy", node: "A2", to: "A5" });
  assert.deepStrictEqual(ok(seBuildOp("remove", { node: "F3", reason: "誤り" })), { op: "remove", node: "F3" });
  assert.deepStrictEqual(ok(seBuildOp("remove", { node: "S1" })), { op: "remove", node: "S1" });
  assert.deepStrictEqual(ok(seBuildOp("edit", { node: "S2", text: "直した" })), { op: "edit", node: "S2", text: "直した" });
  assert.deepStrictEqual(ok(seBuildOp("edit", { node: "A91", text: "新しい文" })), { op: "edit", node: "A91", text: "新しい文" });
  assert.deepStrictEqual(ok(seBuildOp("edit", { node: "A1", body: "本文だけ" })), { op: "edit", node: "A1", body: "本文だけ" });
  // ★ はどの種類の Node にも付く（node で送る）
  assert.deepStrictEqual(ok(seBuildOp("star", { node: "A2" })), { op: "star", node: "A2" });
  assert.deepStrictEqual(ok(seBuildOp("star", { node: "A91" })), { op: "star", node: "A91" });
  assert.deepStrictEqual(ok(seBuildOp("unstar", { node: "F3" })), { op: "unstar", node: "F3" });
  assert.deepStrictEqual(ok(seBuildOp("star", { node: "S1" })), { op: "star", node: "S1" });
  // stance は F だけ
  assert.deepStrictEqual(ok(seBuildOp("set_stance", { node: "F3", stance: "deny" })), { op: "set_stance", node: "F3", stance: "deny" });
  // MLflow の参照。空にすると外す（null）
  assert.deepStrictEqual(ok(seBuildOp("set_mlflow", { node: "E1", mlflow: "run-a" })), { op: "set_mlflow", node: "E1", mlflow: "run-a" });
  // 空文字は外す（API の ztext・空も通す）
  assert.deepStrictEqual(ok(seBuildOp("set_mlflow", { node: "E1", mlflow: "" })), { op: "set_mlflow", node: "E1", mlflow: "" });
  // 報告書（report）で数字の文を通す
  assert.deepStrictEqual(ok(seBuildOp("add_f", { parent: "A2", stance: "deny", text: "精度は 0.9", report: "R-0001" })), { op: "add_f", parent: "A2", stance: "deny", text: "精度は 0.9", report: "R-0001" });
}

// --- 操作の NG の例 ---
function test_ops_ng() {
  // 廃止した op（add_q・insert_q・move_a・regroup）は通らない（表に無い op）
  for (const old of ["add_q", "insert_q", "move_a", "regroup"]) {
    assert.strictEqual(ng(seBuildOp(old, { parent: "A91", q: "A91", from: "A91", to: "A92", text: "x", members: ["A2"], child_q: "c" })), "Unknown operation", old);
  }
  // add_a: 親は A（F・E・SEED は不可）。文は必須。旧い項目 q だけでは親が無いので通らない
  assert.strictEqual(ng(seBuildOp("add_a", { parent: "F1", text: "x" })), "親（A）を選んでください");
  assert.strictEqual(ng(seBuildOp("add_a", { parent: "SEED", text: "最初の答え" })), "親（A）を選んでください");
  assert.strictEqual(ng(seBuildOp("add_a", { q: "A91", text: "x" })), "親（A）を選んでください");
  assert.strictEqual(ng(seBuildOp("add_a", { parent: "A91", text: "" })), "文を入力してください");
  assert.strictEqual(ng(seBuildOp("add_a", { parent: "A91", text: "あ".repeat(121) })), "文は 120 字までです");
  assert.strictEqual(ng(seBuildOp("add_a", { parent: "A1", text: "x", body: "あ".repeat(2001) })), "本文は 2000 字までです");
  // rel は subtype か condition だけ（空は送らない）
  assert.strictEqual(ng(seBuildOp("add_a", { parent: "A1", text: "x", rel: "other" })), "種類（subtype か condition）が正しくありません");
  // move_node は A を 1 件・移す先は A（軸の A も可）
  assert.strictEqual(ng(seBuildOp("move_node", { node: "F4", to: "A93" })), "移す Aを選んでください");
  assert.strictEqual(ng(seBuildOp("move_node", { node: "A4" })), "移す先（A）を選んでください");
  assert.strictEqual(ng(seBuildOp("move_node", { node: "A4", to: "F3" })), "移す先（A）を選んでください");
  assert.strictEqual(ng(seBuildOp("move_node", { node: "A4", to: "E1" })), "移す先（A）を選んでください");
  ng(seBuildOp("add_f", { parent: "E1", stance: "issue", text: "x" }));
  // F は A だけ（E の下に F は置かない）
  ng(seBuildOp("add_f", { parent: "E1", stance: "support", text: "x" })); // 親は A だけ（E の下に F は置かない）
  ng(seBuildOp("add_e", { parent: "A91", text: "x" }));
  ng(seBuildOp("add_e", { parent: "A2", text: "x" }));
  ng(seBuildOp("copy", { node: "A2", to: "F3" }));
  ng(seBuildOp("copy", { node: "F2", to: "A93" }));
  ng(seBuildOp("star", {}));
  ng(seBuildOp("set_stance", { node: "A2", stance: "support" })); // stance は F だけ
  ng(seBuildOp("set_stance", { node: "F3", stance: "" }));
  ng(seBuildOp("set_stance", { node: "F3", stance: "X" }));
  // SPROUT は seed（D）を選ばないと送れない
  assert.strictEqual(ng(seBuildOp("add_sprout", { title: "題" })), "seed（D）を選んでください");
  assert.strictEqual(ng(seBuildOp("add_sprout", { seed: "A91", title: "題" })), "seed（D）を選んでください");
  assert.strictEqual(ng(seBuildOp("add_sprout", { seed: "", title: "題" })), "seed（D）を選んでください");
  ng(seBuildOp("add_seed", {})); // 題が無い
  ng(seBuildOp("add_seed", { title: "あ".repeat(121) }));
  assert.match(ng(seBuildOp("add_seed", { title: "題", body: "あ".repeat(2001) })), /本文が 2000 字を超えています（今 2001 字）/);
  ng(seBuildOp("add_sprout", { seed: "D1" })); // 題が無い
  // 問い・回答（U02）の NG: ID の形・文の字数・軸・a と text の組
  assert.strictEqual(ng(seBuildOp("add_seed_q", { seed: "D1", text: "" })), "問いを入力してください");
  assert.strictEqual(ng(seBuildOp("add_seed_q", { seed: "D1", text: "あ".repeat(301) })), "問いは 300 字までです");
  assert.strictEqual(ng(seBuildOp("add_seed_q", { seed: "A91", text: "問い" })), "seed（D）を選んでください");
  assert.strictEqual(ng(seBuildOp("add_seed_q", { seed: "D1", text: "問い", axis: "goal" })), "軸が正しくありません");
  assert.strictEqual(ng(seBuildOp("edit_seed_q", { q: "A93", text: "問い" })), "問いを選んでください");
  assert.strictEqual(ng(seBuildOp("edit_seed_q", { q: "SQ3", text: "" })), "問いを入力してください");
  assert.strictEqual(ng(seBuildOp("edit_seed_q", { q: "SQ3", text: "問い", axis: "goal" })), "軸が正しくありません");
  assert.strictEqual(ng(seBuildOp("remove_seed_q", { q: "SA3" })), "問いを選んでください");
  assert.strictEqual(ng(seBuildOp("add_answer", { to: "A91", q: "SQ3", text: "文" })), "回答の持ち主（D か S）を選んでください");
  assert.strictEqual(ng(seBuildOp("add_answer", { to: "D1", q: "A3", text: "文" })), "問いを選んでください");
  assert.strictEqual(ng(seBuildOp("add_answer", { to: "D1", q: "SQ3" })), "A を選ぶか、文を入力するかのどちらか一方にしてください");
  assert.strictEqual(ng(seBuildOp("add_answer", { to: "D1", q: "SQ3", a: "A1", text: "文" })), "A を選ぶか、文を入力するかのどちらか一方にしてください");
  assert.strictEqual(ng(seBuildOp("add_answer", { to: "D1", q: "SQ3", a: "F91" })), "A の形が正しくありません: F91");
  assert.strictEqual(ng(seBuildOp("add_answer", { to: "S2", q: "SQ3", text: "あ".repeat(301) })), "回答の文は 300 字までです");
  assert.strictEqual(ng(seBuildOp("edit_answer", { answer: "SQ3", text: "文" })), "回答を選んでください");
  assert.strictEqual(ng(seBuildOp("edit_answer", { answer: "SA3", text: "" })), "回答の文を入力してください");
  assert.strictEqual(ng(seBuildOp("remove_answer", { answer: "A3" })), "回答を選んでください");
  ng(seBuildOp("edit", { node: "A91" })); // text も body も無い
  assert.ok(ok(seBuildOp("edit", { node: "A91", text: "x", body: "本文" }))); // 軸の A も A と同じく本文が付く
  ng(seBuildOp("edit", { node: "F1", text: "x", body: "本文" })); // F に本文は付かない
  ng(seBuildOp("edit", { node: "A1", text: "あ".repeat(121) })); // A は 120 字まで
  ng(seBuildOp("edit", { node: "F3", text: "あ。い。う。え。" })); // F は 3 文まで
  ng(seBuildOp("edit", { node: "F3", text: "あ".repeat(1201) }));
  assert.ok(ok(seBuildOp("edit", { node: "F3", text: "あ".repeat(1200) })));
  ng(seBuildOp("edit", { node: "A91", text: "あ".repeat(301) }));
  ng(seBuildOp("set_mlflow", { node: "A91", mlflow: "run" }));
  // 字数（A は 120 字まで）
  assert.ok(ok(seBuildOp("add_a", { parent: "A91", text: "あ".repeat(120) })));
  ng(seBuildOp("add_a", { parent: "A91", text: "あ".repeat(121) }));
  ng(seBuildOp("add_e", { parent: "A1", text: "あ".repeat(121) }));
  ng(seBuildOp("add_sprout", { seed: "D1", title: "あ".repeat(121) }));
  ng(seBuildOp("add_sprout", { seed: "D1", title: "題", body: "あ".repeat(2001) }));
  ng(seBuildOp("set_mlflow", { node: "E1", mlflow: "あ".repeat(201) }));
  ng(seBuildOp("add_e", { parent: "S2", text: "実験", dataset: "あ".repeat(201) }));
  // C-CITE
  ng(seBuildOp("add_f", { parent: "A2", stance: "support", text: "文", cites: ["37"] }));
  ng(seBuildOp("add_f", { parent: "A2", stance: "support", text: "文", cites: Array(11).fill("#37") }));
  // 1〜3 文
  ng(seBuildOp("add_f", { parent: "A2", stance: "support", text: "あ。い。う。え。" }));
  assert.ok(ok(seBuildOp("add_f", { parent: "A2", stance: "support", text: "あ。い。う。" })));
  // 数字と出典
  ng(seBuildOp("add_f", { parent: "A2", stance: "support", text: "精度は 0.9" }));
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
  // 表の op は 20 種類。廃止した 4 つ（add_q・insert_q・move_a・regroup）は無い。完了の言葉も同じ集まり
  assert.strictEqual(keys.length, 20);
  assert.deepStrictEqual(Object.keys(SE_DONE).sort(), keys);
  for (const old of ["add_q", "insert_q", "move_a", "regroup"]) assert.ok(!keys.includes(old), old);
  assert.ok(keys.includes("set_dataset"));
  assert.ok(keys.includes("move_node") && keys.includes("add_a") && keys.includes("copy"));
  for (const q of SE_QUICK) assert.ok(keys.includes(q));
  assert.deepStrictEqual([...SE_QUICK].sort(), ["set_stance", "star", "unstar"]);
  assert.deepStrictEqual(SE_STANCES.map((x) => x[0]), ["support", "deny", "issue"]);
  assert.deepStrictEqual(SE_RELS.map((x) => x[0]), ["subtype", "condition"]);
  assert.ok(keys.includes("add_sprout") && keys.includes("set_mlflow"));

  // カードのブロック（C9）: 22 本。lab-review を足した（木を見直す）。sci-seed は消えたまま
  const ids = SE_BLOCKS.map(([id]) => id);
  assert.strictEqual(ids.length, 22);
  assert.strictEqual(new Set(ids).size, 22);
  for (const id of ids) assert.match(id, /^(sci|lab)-[a-z-]{2,30}$/);
  assert.ok(ids.includes("lab-read") && ids.includes("sci-verify") && ids.includes("lab-progress"));
  assert.ok(ids.includes("lab-map") && ids.includes("lab-sprout") && ids.includes("sci-exp-sprout"));
  assert.ok(ids.includes("lab-review"));
  assert.ok(!ids.includes("lab-regroup"));
  assert.ok(!ids.includes("sci-seed"));
  for (const gone of ["sci-read", "sci-split", "sci-search", "sci-ideate", "sci-debate", "sci-review", "sci-compare", "sci-draft", "sci-probe"]) {
    assert.ok(!ids.includes(gone), gone);
  }
}

// --- カードの初めの値（C-CARD）と検査 ---
function test_card() {
  const preset = {
    node: "A912", block: "lab-grow", purpose: "「データの量は？」の答えの候補（A）を出す",
    expect: "A を 2 つ以上", scope: "path", assume: ["A99=SST-2 を使う（仮）"], mode: "purpose_first",
    papers: [37], runner: "pi", depth: "full", origin_run: 120, child: "A",
  };
  // 範囲と仮置きは U22 で消した。preset に入っていても返さない・送らない。
  assert.deepStrictEqual(seCardValues(preset), {
    node: "A912", block: "lab-grow", purpose: preset.purpose, expect: "A を 2 つ以上", runner: "cc",
    mode: "purpose_first", model: "", thinking: "", papers: [37], depth: "full", child: "A",
  });
  // 無ければ空（nodeId だけ入る）
  const empty = seCardValues(null, "A93");
  assert.strictEqual(empty.node, "A93");
  assert.strictEqual(empty.block, "");
  assert.strictEqual(empty.purpose, "");
  assert.strictEqual(empty.child, "");
  assert.deepStrictEqual(empty.papers, []);
  // 対象に合わない子の型は空にする（E の下には子を足せない）
  assert.strictEqual(seCardValues({ child: "A" }, "E1").child, "");
  // user ならブロックの既定は lab-read
  assert.strictEqual(seCardValues({ runner: "user" }, "A2").block, "lab-read");

  const built = seBuildCard(seCardValues(preset));
  assert.strictEqual(built.ok, true);
  assert.deepStrictEqual(built.card, {
    node: "A912", block: "lab-grow", purpose: preset.purpose, runner: "cc", papers: [37],
    expect: "A を 2 つ以上", child: "A",
  });
  // pi は廃止（空・未知の実行者は cc に寄せる）。model・thinking は選んだ物だけ送る
  assert.strictEqual(seCardValues({ runner: "cc" }, "A93").runner, "cc");
  const withMt = seBuildCard({ node: "A93", block: "lab-grow", purpose: "目的", runner: "cc", model: "opus", thinking: "high" });
  assert.strictEqual(withMt.card.model, "opus");
  assert.strictEqual(withMt.card.thinking, "high");
  const noMt = seBuildCard({ node: "A93", block: "lab-grow", purpose: "目的", runner: "cc", model: "", thinking: "" });
  assert.ok(!("model" in noMt.card) && !("thinking" in noMt.card), JSON.stringify(noMt.card));
  const readMt = seBuildCard({ node: "A2", block: "lab-read", purpose: "読む", runner: "user", papers: [37], model: "opus" });
  assert.ok(!("model" in readMt.card), JSON.stringify(readMt.card));
  // 範囲と仮置きを直接渡しても、送るカードには入らない。
  const stray = seBuildCard(Object.assign(seCardValues(preset), { scope: "blind", assume: ["A91=あ"] }));
  assert.strictEqual(stray.ok, true);
  assert.ok(!("scope" in stray.card) && !("assume" in stray.card), JSON.stringify(stray.card));
  // 子の型が空なら送るカードに child は入らない
  const noChild = seBuildCard({ node: "A93", block: "lab-grow", purpose: "目的", runner: "cc", child: "" });
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
  // 足す子の型は対象に合うものだけ（軸の A なら A、A なら A か F）
  assert.match(bad({ node: "E1", child: "F" }), /足す子の型/);
  assert.match(bad({ node: "A912", child: "Q" }), /足す子の型/);
  assert.match(bad({ node: "SEED", child: "Q" }), /足す子の型/);
  assert.strictEqual(seBuildCard({ node: "A2", block: "lab-grow", purpose: "目的", runner: "cc", child: "F" }).card.child, "F");
  assert.strictEqual(seBuildCard({ node: "A2", block: "lab-grow", purpose: "目的", runner: "cc", child: "A" }).card.child, "A");

  // READ_CARD: 論文はちょうど 1 本・対象は A
  const read = { node: "A2", block: "lab-read", purpose: "読む", runner: "user", papers: [37], depth: "figures" };
  assert.deepStrictEqual(seBuildCard(read).card, { node: "A2", block: "lab-read", purpose: "読む", runner: "user", papers: [37], depth: "figures" });
  bad(Object.assign({}, read, { papers: [37, 38] }));
  bad(Object.assign({}, read, { papers: [] }));
  assert.match(bad(Object.assign({}, read, { node: "F4", child: "" })), /A にしてください/);
  // READ_CARD は SEED を選べない
  assert.match(bad(Object.assign({}, read, { node: "SEED", child: "" })), /A にしてください/);
  // mode は lab-read のときだけ送る
  assert.deepStrictEqual(seBuildCard({ node: "A2", block: "lab-read", purpose: "読む", runner: "cc", mode: "blind_first" }).card.mode, "blind_first");
  assert.ok(!("mode" in seBuildCard({ node: "A2", block: "lab-grow", purpose: "目的", runner: "cc", mode: "blind_first" }).card));
  // Node を選ばない（空）は木の全体。送るカードに node は入らない（旧い SEED の印も空として扱う）
  assert.strictEqual(seCardValues(null, "").node, "");
  assert.strictEqual(seCardValues(null, "SEED").node, "");
  assert.strictEqual(seCardNode(" A91 "), "A91");
  assert.deepStrictEqual(
    seBuildCard({ node: "", block: "sci-digest", purpose: "木全体を見直す", runner: "cc" }).card,
    { block: "sci-digest", purpose: "木全体を見直す", runner: "cc", papers: [] },
  );
  assert.deepStrictEqual(
    seBuildCard({ node: "SEED", block: "sci-digest", purpose: "木全体を見直す", runner: "cc" }).card,
    { block: "sci-digest", purpose: "木全体を見直す", runner: "cc", papers: [] },
  );
  // 木の全体は READ_CARD の対象にできない（A が要る）
  assert.match(bad({ node: "", child: "", runner: "user", papers: [37], block: "lab-read" }), /A にしてください/);
  // 形の合わない ID は通らない
  assert.match(bad({ node: "N1" }), /Node を選んでください/);
}

// --- 足す子の型（対象の種類で選べる物）---
function test_child_options() {
  // 軸の A（親の無い A）の下は A だけ。軸でない A の下は A と F（A の下に A を足せる）
  const axis = { id: "A91", kind: "A", parent: null };
  const sub = { id: "A2", kind: "A", parent: "A91" };
  assert.deepStrictEqual(seChildOptions("A91", axis), [["A", "A"]]);
  assert.deepStrictEqual(seChildOptions("A2", sub), [["A", "A"], ["F", "F"]]);
  // Node の情報が無いときは ID の頭文字で決める（A は A か F）
  assert.deepStrictEqual(seChildOptions("A2"), [["A", "A"], ["F", "F"]]);
  assert.deepStrictEqual(seChildOptions("SEED"), []);
  assert.deepStrictEqual(seChildOptions("F1"), []);
  assert.deepStrictEqual(seChildOptions(""), []);
  assert.ok(seChildOk("A91", "A", axis));
  assert.ok(!seChildOk("A91", "F", axis), "軸の A の下に F は置かない（CLI も断る）");
  assert.ok(seChildOk("A2", "F", sub));
  assert.ok(seChildOk("A2", "A", sub));
  assert.ok(seChildOk("A91", ""));
  assert.ok(seChildOk("A91", "", axis));
  assert.ok(!seChildOk("SEED", "A"));
}

// --- 親の候補（A を木の順で）と、A を移す先の候補 ---
function test_parents() {
  const nodes = [
    { id: "A91", parent: null },
    { id: "A1", parent: "A91" },
    { id: "F1", parent: "A1" },
    { id: "A92", parent: "A1" },
    { id: "A2", parent: "A92" },
    { id: "A3", parent: "A91", removed: true },
    { id: "A4", parent: "A91" },
  ];
  // A と Q だけ・消した A は外す・F は入らない
  assert.deepStrictEqual(seParents(nodes).map((n) => n.id), ["A91", "A1", "A92", "A2", "A4"]);
  assert.deepStrictEqual(seParents([]), []);
  // 移す A 自身・その子孫（A1 の下の A92 と A2）・今の親（A91）は候補から外す
  assert.deepStrictEqual(seMoveTargets(nodes, "A1").map((n) => n.id), ["A4"]);
  // A2 の今の親は A92。A92 と A2 自身は外し、A1 と A91 と A4 は選べる
  assert.deepStrictEqual(seMoveTargets(nodes, "A2").map((n) => n.id), ["A91", "A1", "A4"]);
  assert.deepStrictEqual(seMoveTargets([], "A1"), []);
}

function test_response_shapes() {
  assert.strictEqual(seCardRunId({ run_id: 130 }), 130);
  assert.strictEqual(seCardRunId({ id: 7 }), 7);
  assert.strictEqual(seCardRunId({ run: { run_id: 9 } }), 9);
  assert.strictEqual(seCardRunId(null), null);
  assert.strictEqual(seCardRunId({}), null);
}


// --- seed（D）の問いと回答の ID・軸（U02）。6 軸の本文の部品は無い（問いの一覧で足す・直す・消す） ---
function test_seed_questions() {
  assert.deepStrictEqual(SE_SEED_AXES.map((x) => x[0]), ["task", "target", "data", "method", "output", "metric"]);
  assert.ok(seIdOk("SQ3", SE_SQ_RE) && seIdOk(" SQ12 ", SE_SQ_RE));
  assert.ok(!seIdOk("A93", SE_SQ_RE) && !seIdOk("SQ0", SE_SQ_RE) && !seIdOk("SA3", SE_SQ_RE) && !seIdOk("", SE_SQ_RE));
  assert.ok(seIdOk("SA7", SE_SA_RE) && !seIdOk("SQ7", SE_SA_RE));
  // 6 軸の本文の部品は消した（seSeedBody・seSeedParse・seSeedError・seSeedAxis）
  const exported = require(src);
  for (const name of ["seSeedBody", "seSeedParse", "seSeedError", "seSeedAxis", "SE_SEED_PICKS", "SE_SEED_VALUE_LABELS"]) {
    assert.strictEqual(exported[name], undefined, name);
  }
  // 雛形の問い（add_seed のあと）は、軸を 6 つ持つ問いと、軸の無い成功基準を板で足す（seBuildOp は通す）
  assert.deepStrictEqual(ok(seBuildOp("add_seed_q", { seed: "D1", text: "成功基準は何か" })), { op: "add_seed_q", seed: "D1", text: "成功基準は何か" });
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
  test_parents();
  test_response_shapes();
  test_seed_questions();
  console.log("test_sciedit: すべて通りました");
}

main();
