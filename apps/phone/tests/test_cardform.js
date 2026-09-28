"use strict";
// task.md 作成フォーム（計画 15）の DOM を使わない関数の小さなテスト（assert だけ）。
// 確かめること:
// - 手順番号の出し方（10 固定・入力は 4.1, 4.2）
// - runner ごとの飛ばし方（user / gl は 3 → 8、cc は provider なし、入力が無ければ 4 / 5 を飛ばす）
// - 戻る（runner 別の戻り先・入力は 1 つ前、先頭はブロック、引き継ぎ画面で閉じる）
// - 必須入力で止まる（候補 0 件なら許容・スキップも止める）
// - トークン選択（単一選択は 1 個・max_tokens の上限）
// - 送る JSON の形（waggle: 形式・スキップは空文字）
// - 書きかけの保存（プロジェクトごと・作成後に消す）
// 実行:
//     node tests/test_cardform.js

const assert = require("assert");
const path = require("path");

// node には localStorage が無いので置き場を用意する（test_linkview.js と同じやり方）
const store = new Map();
global.localStorage = {
  getItem: (k) => (store.has(k) ? store.get(k) : null),
  setItem: (k, v) => { store.set(k, String(v)); },
  removeItem: (k) => { store.delete(k); },
};

const cf = require(path.join(
  __dirname,
  "..",
  "src",
  "sai_phone",
  "static",
  "cardform.js"
));

// --- 使う道具（card/inputs の形を小さくした例） ---

// 必須・単一選択の入力 1 つ
const REQUIRED_FIELD = {
  id: "plan_doc",
  label: "実装計画書",
  required: true,
  multiple: false,
  max_tokens: 1,
  runs: [
    {
      run_id: 36,
      label: "ID36/sub",
      tokens: [
        { token: "aaa", label: "plan.md" },
        { token: "bbb", label: "other.md" },
      ],
    },
  ],
};
// 任意・複数選択・上限 2
const OPT_FIELD = {
  id: "refs",
  label: "参考資料",
  required: false,
  multiple: true,
  max_tokens: 2,
  runs: [
    {
      run_id: 36,
      label: "ID36/sub",
      tokens: [
        { token: "aaa", label: "plan.md" },
        { token: "bbb", label: "other.md" },
        { token: "ccc", label: "third.md" },
      ],
    },
  ],
};
// 必須だが候補が 1 件も無い入力（選べないので許容される・TUI と同じ）
const EMPTY_REQUIRED = {
  id: "note",
  label: "メモ",
  required: true,
  multiple: false,
  max_tokens: null,
  runs: [],
};

/** runner → block まで進んだ状態を作る。 */
function atBlock(runner) {
  const s = cf.cfNewDraft();
  s.runner = runner;
  s.step = "block";
  s.block = "sub";
  return s;
}

function testStepLabel() {
  assert.deepStrictEqual(
    cf.cfStepInfo(cf.cfNewDraft()),
    { id: "runner", no: 1, title: "runner", label: "Step 1/9" }
  );
  // 手順 3 は 3.1, 3.2 と続く（番号は 3 のまま）
  const s = atBlock("pi");
  cf.cfNext(s, [REQUIRED_FIELD]);
  assert.strictEqual(s.step, "inputs");
  assert.strictEqual(cf.cfStepInfo(s).label, "Step 3.1/9");
  // 必須の入力を選ばないと 4.2 へ進めない（先に選ぶ）
  assert.strictEqual(cf.cfToggleToken(s, [REQUIRED_FIELD], "plan_doc", "aaa"), null);
  cf.cfNext(s, [REQUIRED_FIELD, OPT_FIELD]);
  assert.strictEqual(cf.cfStepInfo(s).label, "Step 3.2/9");
  assert.strictEqual(cf.cfStepInfo({ step: "confirm" }).label, "Step 9/9");
}

function testRunnerSkips() {
  // user / gl は 2（ブロック）から 7（ラウンド）へ飛ぶ（3〜6 は対象外）
  for (const runner of ["user", "gl"]) {
    const s = atBlock(runner);
    assert.strictEqual(cf.cfNext(s, [REQUIRED_FIELD]), null, runner);
    assert.strictEqual(s.step, "rounds", `${runner} が ${s.step} になった`);
    assert.strictEqual(cf.cfStepInfo(s).label, "Step 7/9");
    // 一度も選ばない → provider / model も空のまま
    assert.strictEqual(s.provider, "");
    assert.strictEqual(s.model, "");
    // 戻る先はブロック（TUI の rounds の戻り先と同じ）
    assert.strictEqual(cf.cfBack(s), null);
    assert.strictEqual(s.step, "block");
  }

  // cc は provider が無い（入力のあと 6 モデルへ）
  const cc = atBlock("cc");
  assert.strictEqual(cf.cfNext(cc, [REQUIRED_FIELD]), null);
  assert.strictEqual(cc.step, "inputs");
  cc.inputs.plan_doc = ["aaa"];
  assert.strictEqual(cf.cfNext(cc, [REQUIRED_FIELD]), null);
  assert.strictEqual(cc.step, "model");
  assert.strictEqual(cf.cfStepInfo(cc).label, "Step 5/9");

  // cc で入力が無ければ 4 を飛ばして 6 へ
  const ccNoInput = atBlock("cc");
  assert.strictEqual(cf.cfNext(ccNoInput, []), null);
  assert.strictEqual(ccNoInput.step, "model");

  // pi は入力のあと 5 provider → 6 モデル → 7 thinking → 8 ラウンド
  const pi = atBlock("pi");
  assert.strictEqual(cf.cfNext(pi, [REQUIRED_FIELD]), null);
  assert.strictEqual(pi.step, "inputs");
  pi.inputs.plan_doc = ["aaa"];
  assert.strictEqual(cf.cfNext(pi, [REQUIRED_FIELD]), null);
  assert.strictEqual(pi.step, "provider");
  assert.strictEqual(cf.cfNext(pi, []), null);
  assert.strictEqual(pi.step, "model");
  assert.strictEqual(cf.cfNext(pi, []), null);
  assert.strictEqual(pi.step, "thinking");
  assert.strictEqual(cf.cfNext(pi, []), null);
  assert.strictEqual(pi.step, "rounds");

  // pi で入力が無ければ 4 を飛ばして 5 へ
  const piNoInput = atBlock("pi");
  assert.strictEqual(cf.cfNext(piNoInput, []), null);
  assert.strictEqual(piNoInput.step, "provider");

  // runner をスキップした（空文字）ときも pi 扱い（TUI と同じ）
  const skipRunner = cf.cfNewDraft();
  skipRunner.step = "runner";
  skipRunner.runner = "";
  assert.strictEqual(cf.cfSkip(skipRunner, []), null);
  assert.strictEqual(skipRunner.step, "block");
  assert.strictEqual(cf.cfNext(skipRunner, []), null);
  assert.strictEqual(skipRunner.step, "provider");
}

function testBack() {
  // 最初の画面（runner）の戻る = 閉じる（"close" が返る）
  assert.strictEqual(cf.cfBack(cf.cfNewDraft()), "close");

  // ブロック → runner
  const b = cf.cfNewDraft();
  b.step = "block";
  assert.strictEqual(cf.cfBack(b), null);
  assert.strictEqual(b.step, "runner");

  // 入力は 1 つ前、先頭（4.1）からはブロックへ
  const i = atBlock("pi");
  i.step = "inputs";
  i.inputIdx = 1;
  assert.strictEqual(cf.cfBack(i), null);
  assert.strictEqual(i.inputIdx, 0);
  assert.strictEqual(cf.cfBack(i), null);
  assert.strictEqual(i.step, "block");

  // provider → block、model → cc なら block で pi なら provider
  const p = atBlock("pi"); p.step = "provider";
  assert.strictEqual(cf.cfBack(p), null);
  assert.strictEqual(p.step, "block");
  const mc = atBlock("cc"); mc.step = "model";
  assert.strictEqual(cf.cfBack(mc), null);
  assert.strictEqual(mc.step, "block");
  const mp = atBlock("pi"); mp.step = "model";
  assert.strictEqual(cf.cfBack(mp), null);
  assert.strictEqual(mp.step, "provider");

  // thinking → model、ラウンド → thinking（user / gl はブロック）
  const t = atBlock("pi"); t.step = "thinking";
  assert.strictEqual(cf.cfBack(t), null);
  assert.strictEqual(t.step, "model");
  const rd = atBlock("pi"); rd.step = "rounds";
  assert.strictEqual(cf.cfBack(rd), null);
  assert.strictEqual(rd.step, "thinking");
  const ru = atBlock("user"); ru.step = "rounds";
  assert.strictEqual(cf.cfBack(ru), null);
  assert.strictEqual(ru.step, "block");

  // task_text ⇄ ラウンド、確認 → task_text
  const tt = cf.cfNewDraft(); tt.step = "task_text";
  assert.strictEqual(cf.cfBack(tt), null);
  assert.strictEqual(tt.step, "rounds");
  const cf10 = cf.cfNewDraft(); cf10.step = "confirm";
  assert.strictEqual(cf.cfBack(cf10), null);
  assert.strictEqual(cf10.step, "task_text");
}

function testRequiredBlocks() {
  const s = atBlock("pi");
  assert.strictEqual(cf.cfNext(s, [REQUIRED_FIELD]), null); // ブロック → 手順 4
  assert.strictEqual(s.step, "inputs");
  // 必須で未選択 → 進めない（理由が返り，手順は変わらない）
  const err = cf.cfNext(s, [REQUIRED_FIELD]);
  assert.strictEqual(typeof err, "string");
  assert.ok(err.includes("plan_doc"), err);
  assert.strictEqual(s.step, "inputs");
  // スキップも同じく止まる（TUI と同じ文言）
  const skipErr = cf.cfSkip(s, [REQUIRED_FIELD]);
  assert.ok(skipErr && skipErr.includes("必須のためスキップできません"), skipErr);
  assert.strictEqual(s.step, "inputs");

  // 選べば進む（単一選択なので 1 個で足りる・入力は 1 つなので次は 5 provider）
  assert.strictEqual(cf.cfToggleToken(s, [REQUIRED_FIELD], "plan_doc", "aaa"), null);
  assert.strictEqual(cf.cfNext(s, [REQUIRED_FIELD]), null);
  assert.strictEqual(s.step, "provider");

  // 必須でも候補 0 件なら選べないので進める（TUI と同じ）
  const e = atBlock("pi");
  assert.strictEqual(cf.cfNext(e, [EMPTY_REQUIRED]), null); // 手順 4 へ
  assert.strictEqual(e.step, "inputs");
  assert.strictEqual(cf.cfNext(e, [EMPTY_REQUIRED]), null);
  assert.strictEqual(e.step, "provider");

  // 任意の入力は未選択でも進める
  const o = atBlock("pi");
  assert.strictEqual(cf.cfNext(o, [OPT_FIELD]), null); // 手順 4 へ
  assert.strictEqual(o.step, "inputs");
  assert.strictEqual(cf.cfNext(o, [OPT_FIELD]), null);
  assert.strictEqual(o.step, "provider");
}

function testToggle() {
  const s = cf.cfNewDraft();
  // 単一選択（multiple: false）は 1 個に絞る
  assert.strictEqual(cf.cfToggleToken(s, [REQUIRED_FIELD], "plan_doc", "aaa"), null);
  assert.deepStrictEqual(s.inputs.plan_doc, ["aaa"]);
  assert.strictEqual(cf.cfToggleToken(s, [REQUIRED_FIELD], "plan_doc", "bbb"), null);
  assert.deepStrictEqual(s.inputs.plan_doc, ["bbb"]);
  // 押しなおすと外れる
  assert.strictEqual(cf.cfToggleToken(s, [REQUIRED_FIELD], "plan_doc", "bbb"), null);
  assert.deepStrictEqual(s.inputs.plan_doc, []);

  // 複数選択は上限（max_tokens: 2）まで
  assert.strictEqual(cf.cfToggleToken(s, [OPT_FIELD], "refs", "aaa"), null);
  assert.strictEqual(cf.cfToggleToken(s, [OPT_FIELD], "refs", "bbb"), null);
  const over = cf.cfToggleToken(s, [OPT_FIELD], "refs", "ccc");
  assert.ok(over && over.includes("max 2"), over);
  assert.deepStrictEqual(s.inputs.refs, ["aaa", "bbb"]);
  // 定義に無い入力名は弾く
  assert.ok(cf.cfToggleToken(s, [OPT_FIELD], "bogus", "aaa"));
}

function testPayload() {
  const s = cf.cfNewDraft();
  s.runner = "cc";
  s.block = "sub";
  s.inputs = { plan_doc: ["aaa"], refs: [] }; // 空の入力は送らない
  s.model = "opus";
  s.thinking = "high";
  s.rounds = { global_round: "5", round: "2", tag: "" };
  s.task_text = "計画を読んで実装せよ";
  const payload = cf.cfBuildPayload(s);
  // 項目名は計画 12 §3.2 の card create の形のまま
  assert.deepStrictEqual(Object.keys(payload), [
    "runner", "block", "inputs", "provider", "model",
    "thinking", "global_round", "round", "tag", "task_text",
  ]);
  // トークンは waggle: 形式のカンマ区切り
  assert.strictEqual(payload.inputs.plan_doc, "waggle:aaa");
  assert.deepStrictEqual(payload.inputs, { plan_doc: "waggle:aaa" });
  // スキップした項目は空文字（sai-link が自動値にする）
  assert.strictEqual(payload.provider, "");
  assert.strictEqual(payload.tag, "");
  assert.strictEqual(payload.runner, "cc");

  // 初期状態（全部スキップ）でも形が壊れない
  const empty = cf.cfBuildPayload(cf.cfNewDraft());
  assert.deepStrictEqual(empty.inputs, {});
  assert.strictEqual(empty.task_text, "");

  // 2 個選んだ入力はカンマ区切り
  const two = cf.cfNewDraft();
  two.inputs = { refs: ["aaa", "bbb"] };
  assert.strictEqual(
    cf.cfBuildPayload(two).inputs.refs,
    "waggle:aaa, waggle:bbb"
  );
}

function testDraft() {
  store.clear();
  // 最初は無し
  assert.strictEqual(cf.cfLoadDraft("sai-phone"), null);
  // プロジェクトごとに別々
  const s = cf.cfNewDraft();
  s.step = "rounds";
  s.runner = "pi";
  s.block = "sub";
  s.rounds.round = "3";
  cf.cfSaveDraft("sai-phone", s);
  const back = cf.cfLoadDraft("sai-phone");
  assert.strictEqual(back.step, "rounds");
  assert.strictEqual(back.runner, "pi");
  assert.strictEqual(back.rounds.round, "3");
  assert.strictEqual(cf.cfLoadDraft("other-project"), null);
  // 作成に成功したら消える（計画 15 §1.5）
  cf.cfClearDraft("sai-phone");
  assert.strictEqual(cf.cfLoadDraft("sai-phone"), null);
  // 形が崩れた書きかけは無し扱い
  store.set(cf.cfDraftKey("broken"), "これは JSON ではありません");
  assert.strictEqual(cf.cfLoadDraft("broken"), null);
  store.set(cf.cfDraftKey("no-step"), JSON.stringify({ runner: "pi" }));
  assert.strictEqual(cf.cfLoadDraft("no-step"), null);
}

function testStepHints() {
  // 10 画面それぞれに説明がある（端末に慣れていない人向け・U5 §3）
  for (const step of cf.CF_STEPS) {
    const hint = cf.cfStepHint(step.id);
    assert.strictEqual(typeof hint, "string");
    assert.ok(hint.length > 0, `${step.id} の説明がありません`);
  }
  // 説明のキーは手順の id と同じだけ
  assert.deepStrictEqual(
    cf.CF_STEP_HINTS,
    Object.fromEntries(cf.CF_STEPS.map((s) => [s.id, cf.CF_STEP_HINTS[s.id]]))
  );
  // 知らない手順には説明を出さない（画面が止まらない）
  assert.strictEqual(cf.cfStepHint("oreason"), "");
  // 説明は日本語だけ（英語だけの文は入れていない）
  for (const [id, hint] of Object.entries(cf.CF_STEP_HINTS)) {
    assert.ok(/[\u3040-\u30ff\u4e00-\u9fff]/.test(hint), `${id} の説明に日本語がありません`);
  }
}

function main() {
  testStepLabel();
  testRunnerSkips();
  testBack();
  testRequiredBlocks();
  testToggle();
  testPayload();
  testDraft();
  testStepHints();
  console.log("test_cardform: すべて通りました");
}

main();
