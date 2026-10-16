"use strict";
// task.md 作成フォーム（計画 15・U09・U08）の DOM を使わない関数の小さなテスト（assert だけ）。
// 確かめること:
// - 手順の並びと番号（5 つ。block → model → thinking → task_text → confirm）
// - 進む・戻る・スキップ（先頭の戻るは閉じる）
// - 送る JSON の形（block・model・thinking・task_text の 4 つだけ）
// - 書きかけの保存（プロジェクトごと・作成後に消す・古い runner・rounds の書きかけは読み替える）
// - 5 画面それぞれに説明がある
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
  "agent_link_web",
  "static",
  "cardform.js"
));

function testStepLabel() {
  assert.deepStrictEqual(
    cf.cfStepInfo(cf.cfNewDraft()),
    { id: "block", no: 1, title: "Block", label: "Step 1/5" }
  );
  assert.strictEqual(cf.cfStepInfo({ step: "confirm" }).label, "Step 5/5");
  assert.strictEqual(cf.cfStepInfo({ step: "model" }).label, "Step 2/5");
  assert.deepStrictEqual(
    cf.CF_STEPS.map((s) => s.id),
    ["block", "model", "thinking", "task_text", "confirm"]
  );
  // 知らない手順は先頭の扱い
  assert.strictEqual(cf.cfStepInfo({ step: "runner" }).label, "Step 1/5");
}

function testNext() {
  const s = cf.cfNewDraft();
  assert.strictEqual(s.step, "block");
  for (const want of ["model", "thinking", "task_text", "confirm"]) {
    assert.strictEqual(cf.cfNext(s), null);
    assert.strictEqual(s.step, want);
  }
  // 確認の次は無い
  assert.strictEqual(typeof cf.cfNext(s), "string");
  assert.strictEqual(s.step, "confirm");
}

function testBack() {
  // 最初の画面（block）の戻る = 閉じる（"close" が返る）
  assert.strictEqual(cf.cfBack(cf.cfNewDraft()), "close");
  const want = { confirm: "task_text", task_text: "thinking", thinking: "model", model: "block" };
  for (const [from, to] of Object.entries(want)) {
    const s = cf.cfNewDraft();
    s.step = from;
    assert.strictEqual(cf.cfBack(s), null);
    assert.strictEqual(s.step, to, from);
  }
}

function testSkip() {
  // 確認はスキップできない
  const confirm = cf.cfNewDraft();
  confirm.step = "confirm";
  assert.strictEqual(cf.cfSkip(confirm), "Cannot skip this step.");
  assert.strictEqual(cf.cfCanSkip(confirm), false);
  // 途中の画面はスキップできる
  for (const step of ["model", "thinking", "task_text"]) {
    assert.strictEqual(cf.cfCanSkip({ step }), true, step);
  }
  // block は必須（スキップできない）
  const b = cf.cfNewDraft();
  b.block = "sub";
  assert.strictEqual(cf.cfCanSkip(b), false);
  assert.strictEqual(cf.cfSkip(b), "Cannot skip this step.");
  assert.strictEqual(b.block, "sub");
  // model をスキップすると thinking へ
  const s = cf.cfNewDraft();
  s.step = "model";
  s.model = "opus";
  assert.strictEqual(cf.cfSkip(s), null);
  assert.strictEqual(s.model, "");
  assert.strictEqual(s.step, "thinking");
  // thinking をスキップすると task_text へ
  const t = cf.cfNewDraft();
  t.step = "thinking";
  t.thinking = "high";
  assert.strictEqual(cf.cfSkip(t), null);
  assert.strictEqual(t.thinking, "");
  assert.strictEqual(t.step, "task_text");
}

function testPayload() {
  const s = cf.cfNewDraft();
  s.block = "sub";
  s.model = "opus";
  s.thinking = "high";
  s.task_text = "計画を読んで実装せよ";
  // 古い項目が state に残っていても送らない
  s.runner = "cc";
  s.rounds = { global_round: "5", round: "2", tag: "fix" };
  const payload = cf.cfBuildPayload(s);
  assert.deepStrictEqual(payload, {
    block: "sub", model: "opus", thinking: "high", task_text: "計画を読んで実装せよ",
  });
  for (const k of ["runner", "provider", "global_round", "round", "tag", "inputs"]) {
    assert.strictEqual(Object.prototype.hasOwnProperty.call(payload, k), false, k);
  }
  // 初期状態（全部スキップ）でも形が壊れない
  assert.deepStrictEqual(cf.cfBuildPayload(cf.cfNewDraft()), {
    block: "", model: "", thinking: "", task_text: "",
  });
}

function testToggleRef() {
  const ref = "waggle:ABCD1234:path:.agent-link/outputs/*/3/report.md";
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
  assert.strictEqual(cf.cfLoadDraft("agent-link-web"), null);
  // プロジェクトごとに別々
  const s = cf.cfNewDraft();
  s.step = "thinking";
  s.block = "sub";
  s.model = "opus";
  cf.cfSaveDraft("agent-link-web", s);
  const back = cf.cfLoadDraft("agent-link-web");
  assert.strictEqual(back.step, "thinking");
  assert.strictEqual(back.block, "sub");
  assert.strictEqual(back.model, "opus");
  assert.strictEqual(cf.cfLoadDraft("other-project"), null);
  // 作成に成功したら消える（計画 15 §1.5）
  cf.cfClearDraft("agent-link-web");
  assert.strictEqual(cf.cfLoadDraft("agent-link-web"), null);
  // 形が崩れた書きかけは無し扱い
  store.set(cf.cfDraftKey("broken"), "これは JSON ではありません");
  assert.strictEqual(cf.cfLoadDraft("broken"), null);
  // 手順が無い書きかけは block から
  store.set(cf.cfDraftKey("no-step"), JSON.stringify({ block: "sub" }));
  assert.deepStrictEqual(cf.cfLoadDraft("no-step"), Object.assign(cf.cfNewDraft(), { block: "sub" }));
}

function testOldDraft() {
  store.clear();
  // 消えた runner の手順は block へ、rounds の手順は task_text へ。消えた項目は拾わない
  store.set(cf.cfDraftKey("old-runner"), JSON.stringify({
    step: "runner", runner: "cc", block: "sub", model: "opus",
    rounds: { global_round: "5", round: "2", tag: "fix" }, task_text: "x",
  }));
  assert.deepStrictEqual(cf.cfLoadDraft("old-runner"), {
    step: "block", block: "sub", model: "opus", thinking: "", task_text: "x",
  });
  store.set(cf.cfDraftKey("old-rounds"), JSON.stringify({
    step: "rounds", runner: "user", rounds: { tag: "fix" }, thinking: "high",
  }));
  assert.deepStrictEqual(cf.cfLoadDraft("old-rounds"), {
    step: "task_text", block: "", model: "", thinking: "high", task_text: "",
  });
  // U09 より前の「入力ファイル」の手順も block から
  store.set(cf.cfDraftKey("old-inputs"), JSON.stringify({ step: "inputs", inputs: { a: ["x"] } }));
  assert.strictEqual(cf.cfLoadDraft("old-inputs").step, "block");
}

function testStepHints() {
  // 5 画面それぞれに説明がある（端末に慣れていない人向け・U5 §3）
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
  assert.strictEqual(cf.cfStepHint("runner"), "");
  assert.strictEqual(cf.cfStepHint("rounds"), "");
}

function main() {
  testStepLabel();
  testNext();
  testBack();
  testSkip();
  testPayload();
  testToggleRef();
  testAppendTaskText();
  testTargetName();
  testShouldClose();
  testDraft();
  testOldDraft();
  testStepHints();
  console.log("test_cardform: すべて通りました");
}

main();
