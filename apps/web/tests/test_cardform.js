"use strict";
// task.md 作成フォーム（計画 15・U09）の DOM を使わない関数の小さなテスト（assert だけ）。
// 確かめること:
// - 手順の並びと番号（8 固定。runner → block → provider → model → thinking
//   → rounds → task_text → confirm）
// - runner ごとの飛ばし方（user / gl は 2 → 6、cc は provider を飛ばす）
// - 戻る（runner 別の戻り先・先頭は閉じる）
// - 送る JSON の形（入力ファイルの手順が無くなり，inputs を送らない）
// - 書きかけの保存（プロジェクトごと・作成後に消す・古い inputs の手順は無し扱い）
// - 8 画面それぞれに日本語の説明がある
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
  "sai_web",
  "static",
  "cardform.js"
));

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
    { id: "runner", no: 1, title: "runner", label: "Step 1/8" }
  );
  assert.strictEqual(cf.cfStepInfo({ step: "confirm" }).label, "Step 8/8");
  assert.strictEqual(cf.cfStepInfo({ step: "provider" }).label, "Step 3/8");
  // 手順の並びは TUI と同じ（runner → block → provider → model → thinking → rounds → task_text）
  assert.deepStrictEqual(
    cf.CF_STEPS.map((s) => s.id),
    ["runner", "block", "provider", "model", "thinking", "rounds", "task_text", "confirm"]
  );
}

function testRunnerSkips() {
  // user / gl は 2（ブロック）から 6（ラウンド）へ飛ぶ（provider〜thinking は対象外）
  for (const runner of ["user", "gl"]) {
    const s = atBlock(runner);
    assert.strictEqual(cf.cfNext(s), null, runner);
    assert.strictEqual(s.step, "rounds", `${runner} が ${s.step} になった`);
    assert.strictEqual(cf.cfStepInfo(s).label, "Step 6/8");
    assert.strictEqual(s.provider, "");
    assert.strictEqual(s.model, "");
    // 戻る先はブロック（TUI の rounds の戻り先と同じ）
    assert.strictEqual(cf.cfBack(s), null);
    assert.strictEqual(s.step, "block");
  }

  // cc は provider が無い（ブロックのあと 4 モデルへ）
  const cc = atBlock("cc");
  assert.strictEqual(cf.cfNext(cc), null);
  assert.strictEqual(cc.step, "model");
  assert.strictEqual(cf.cfStepInfo(cc).label, "Step 4/8");
  assert.strictEqual(cc.provider, "");

  // pi は 3 provider → 4 モデル → 5 thinking → 6 ラウンド
  const pi = atBlock("pi");
  assert.strictEqual(cf.cfNext(pi), null);
  assert.strictEqual(pi.step, "provider");
  assert.strictEqual(cf.cfNext(pi), null);
  assert.strictEqual(pi.step, "model");
  assert.strictEqual(cf.cfNext(pi), null);
  assert.strictEqual(pi.step, "thinking");
  assert.strictEqual(cf.cfNext(pi), null);
  assert.strictEqual(pi.step, "rounds");
  assert.strictEqual(cf.cfNext(pi), null);
  assert.strictEqual(pi.step, "task_text");
  assert.strictEqual(cf.cfNext(pi), null);
  assert.strictEqual(pi.step, "confirm");

  // runner をスキップした（空文字）ときも pi 扱い（TUI と同じ）
  const skipRunner = cf.cfNewDraft();
  skipRunner.step = "runner";
  skipRunner.runner = "";
  assert.strictEqual(cf.cfSkip(skipRunner), null);
  assert.strictEqual(skipRunner.step, "block");
  assert.strictEqual(cf.cfNext(skipRunner), null);
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
  const cf8 = cf.cfNewDraft(); cf8.step = "confirm";
  assert.strictEqual(cf.cfBack(cf8), null);
  assert.strictEqual(cf8.step, "task_text");
}

function testSkip() {
  // 引き継ぎと確認はスキップできない
  const confirm = cf.cfNewDraft();
  confirm.step = "confirm";
  assert.strictEqual(cf.cfSkip(confirm), "スキップできません");
  assert.strictEqual(cf.cfCanSkip(confirm), false);
  // 途中の画面はスキップできる
  for (const step of ["runner", "block", "provider", "model", "thinking", "rounds", "task_text"]) {
    assert.strictEqual(cf.cfCanSkip({ step }), true, step);
  }
  // provider をスキップするとモデルへ
  const s = atBlock("pi");
  s.step = "provider";
  assert.strictEqual(cf.cfSkip(s), null);
  assert.strictEqual(s.provider, "");
  assert.strictEqual(s.step, "model");
}

function testPayload() {
  const s = cf.cfNewDraft();
  s.runner = "cc";
  s.block = "sub";
  s.model = "opus";
  s.thinking = "high";
  s.rounds = { global_round: "5", round: "2", tag: "" };
  s.task_text = "計画を読んで実装せよ";
  const payload = cf.cfBuildPayload(s);
  // 入力ファイルの手順は無い（inputs を送らない・U09）
  assert.deepStrictEqual(Object.keys(payload), [
    "runner", "block", "provider", "model",
    "thinking", "global_round", "round", "tag", "task_text",
  ]);
  assert.strictEqual(Object.prototype.hasOwnProperty.call(payload, "inputs"), false);
  // スキップした項目は空文字（sai-link が自動値にする）
  assert.strictEqual(payload.provider, "");
  assert.strictEqual(payload.tag, "");
  assert.strictEqual(payload.runner, "cc");
  assert.strictEqual(payload.task_text, "計画を読んで実装せよ");

  // 初期状態（全部スキップ）でも形が壊れない
  const empty = cf.cfBuildPayload(cf.cfNewDraft());
  assert.strictEqual(empty.task_text, "");
  assert.strictEqual(empty.block, "");
  assert.strictEqual(empty.model, "");
}

function testToggleRef() {
  const ref = "waggle:ABCD1234:path:.sai/outputs/*/3/report.md";
  assert.strictEqual(cf.cfToggleRef("", ref, true), ref);
  assert.strictEqual(cf.cfToggleRef(`本文\n${ref}`, ref, true), `本文\n${ref}`);
  assert.strictEqual(cf.cfToggleRef(`本文\n${ref}`, ref, false), "本文");
  assert.strictEqual(cf.cfToggleRef("本文", "", true), "本文");
}

function testAppendTaskText() {
  const line = "対象の論文: #12「Attention」（2021・DOI 10.1000/xyz）";
  // 空ならそのまま，書きかけがあれば空行をはさんで末尾に
  assert.strictEqual(cf.cfAppendTaskText("", line), line);
  assert.strictEqual(cf.cfAppendTaskText("本文", line), `本文\n\n${line}`);
  // 同じ文は 2 度足さない
  assert.strictEqual(cf.cfAppendTaskText(`本文\n\n${line}`, line), `本文\n\n${line}`);
  // 書きかけは消さない（末尾の空白は整える）
  assert.strictEqual(cf.cfAppendTaskText("本文\n", line), `本文\n\n${line}`);
  // 空の文は足さない
  assert.strictEqual(cf.cfAppendTaskText("本文", "  "), "本文");
}

function testTargetName() {
  // opts.project があれば List の住所に頼らない（論文の詳細から開ける・U10）
  assert.strictEqual(cf.cfTargetName({ project: "sai-moon" }, "#/paper/x/1", ""), "sai-moon");
  global.parseListHash = (h) => {
    const m = String(h || "").match(/^#\/project\/([^/]+)\/list/);
    return m ? { name: decodeURIComponent(m[1]), runId: null, view: null } : null;
  };
  try {
    assert.strictEqual(cf.cfTargetName({}, "#/project/res/list", "old"), "res");
    assert.strictEqual(cf.cfTargetName({}, "#/paper/x/1", "old"), "old");
    assert.strictEqual(cf.cfTargetName({}, "#/paper/x/1", ""), "");
  } finally {
    delete global.parseListHash;
  }
}

function testShouldClose() {
  global.parseListHash = (h) =>
    /\/list/.test(String(h)) ? { name: "x", runId: null, view: null } : null;
  try {
    // List で開いた: List のままなら閉じない，離れたら閉じる
    assert.strictEqual(cf.cfShouldClose("#/project/x/list", "#/project/x/list/3/log"), false);
    assert.strictEqual(cf.cfShouldClose("#/project/x/list", "#/project/x/sessions"), true);
    // List でない住所で開いた: 変わったら閉じる（論文の詳細から開いたとき）
    assert.strictEqual(cf.cfShouldClose("#/paper/x/1", "#/paper/x/1"), false);
    assert.strictEqual(cf.cfShouldClose("#/paper/x/1", "#/project/x/list"), true);
    // 住所が無ければ閉じない
    assert.strictEqual(cf.cfShouldClose("", "#/paper/x/1"), false);
  } finally {
    delete global.parseListHash;
  }
}

function testDraft() {
  store.clear();
  // 最初は無し
  assert.strictEqual(cf.cfLoadDraft("sai-web"), null);
  // プロジェクトごとに別々
  const s = cf.cfNewDraft();
  s.step = "rounds";
  s.runner = "pi";
  s.block = "sub";
  s.rounds.round = "3";
  cf.cfSaveDraft("sai-web", s);
  const back = cf.cfLoadDraft("sai-web");
  assert.strictEqual(back.step, "rounds");
  assert.strictEqual(back.runner, "pi");
  assert.strictEqual(back.rounds.round, "3");
  assert.strictEqual(cf.cfLoadDraft("other-project"), null);
  // 作成に成功したら消える（計画 15 §1.5）
  cf.cfClearDraft("sai-web");
  assert.strictEqual(cf.cfLoadDraft("sai-web"), null);
  // 形が崩れた書きかけは無し扱い
  store.set(cf.cfDraftKey("broken"), "これは JSON ではありません");
  assert.strictEqual(cf.cfLoadDraft("broken"), null);
  store.set(cf.cfDraftKey("no-step"), JSON.stringify({ runner: "pi" }));
  assert.strictEqual(cf.cfLoadDraft("no-step"), null);
  // U09 より前の「入力ファイル」の手順は無し扱い（番号が変わったため）
  store.set(cf.cfDraftKey("old-inputs"), JSON.stringify({ step: "inputs", runner: "pi", inputs: { a: ["x"] } }));
  assert.strictEqual(cf.cfLoadDraft("old-inputs"), null);
}

function testStepHints() {
  // 8 画面それぞれに説明がある（端末に慣れていない人向け・U5 §3）
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
  assert.strictEqual(cf.cfStepHint("inputs"), "");
  // 説明は日本語だけ（英語だけの文は入れていない）
  for (const [id, hint] of Object.entries(cf.CF_STEP_HINTS)) {
    assert.ok(/[\u3040-\u30ff\u4e00-\u9fff]/.test(hint), `${id} の説明に日本語がありません`);
  }
}

function main() {
  testStepLabel();
  testRunnerSkips();
  testBack();
  testSkip();
  testPayload();
  testToggleRef();
  testAppendTaskText();
  testTargetName();
  testShouldClose();
  testDraft();
  testStepHints();
  console.log("test_cardform: すべて通りました");
}

main();
