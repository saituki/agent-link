"use strict";
// 実行の切り替えの古い応答を捨てることと、定期の更新を重ねないことの小さなテスト（app.js の関数を取り出して試す）。
// 実行:
//     node tests/test_run_switch.js

const assert = require("assert");
const fs = require("fs");
const path = require("path");
const vm = require("vm");

const src = fs.readFileSync(
  path.join(__dirname, "..", "src", "agent_link_web", "static", "app.js"),
  "utf8"
);
function pick(re, what) {
  const m = src.match(re);
  assert.ok(m, `app.js に ${what} が見つかりません`);
  return m[0];
}
const code = [
  pick(/^let runSeq = 0;.*$/m, "runSeq"),
  pick(/^let runPolling = false;.*$/m, "runPolling"),
  pick(/^function runApiBase\(\) \{[\s\S]*?^\}/m, "runApiBase"),
  pick(/^async function loadChatMore\(\) \{[\s\S]*?^\}/m, "loadChatMore"),
  pick(/^async function pollRun\(\) \{[\s\S]*?^\}/m, "pollRun"),
].join("\n");

// 返事を後で返せる通信（apiFetch の代わり）
const pending = [];
const appended = [];
let metaCalls = 0;
let metaGate = null; // pollRun の loadRunMeta を止めるための待ち
const sandbox = {
  current: { name: "run" },
  currentRun: { project_name: "p", run_id: 1 },
  chatCursor: null,
  apiFetch: () => new Promise((resolve) => pending.push(resolve)),
  appendItems: (items) => appended.push(...items),
  loadQuestions: async () => {},
  flushPendingSends: async () => {},
  applyPartial: () => {},
  applyError: () => {},
  setOffline: () => {},
  onOk: () => {},
  onFail: () => {},
  loadRunMeta: () => {
    metaCalls++;
    return new Promise((resolve) => { metaGate = () => resolve(null); });
  },
  runStarting: false,
};
vm.createContext(sandbox);
// openRun が連番を進めるのと同じ操作を、テストから呼べるように出す
vm.runInContext(code + "\n;globalThis.bumpRun = () => ++runSeq; globalThis.pollRunning = () => runPolling;", sandbox);

function jsonRes(items) {
  return { ok: true, json: async () => ({ items, next_cursor: null }) };
}
const tick = () => new Promise((r) => setImmediate(r));

async function testStaleResponseDropped() {
  const p = vm.runInContext("loadChatMore()", sandbox);
  await tick();
  // openRun が別の実行に移った（連番が進んだ）あとで応答が届く
  sandbox.bumpRun();
  pending.shift()(jsonRes([{ id: "old" }]));
  await p;
  assert.deepStrictEqual(appended, [], "古い実行の応答が会話に足されました");
}

async function testCurrentResponseKept() {
  const p = vm.runInContext("loadChatMore()", sandbox);
  await tick();
  pending.shift()(jsonRes([{ id: "new" }]));
  await p;
  assert.deepStrictEqual(appended, [{ id: "new" }], "今の実行の応答が足されません");
}

async function testPollNotOverlapping() {
  const first = vm.runInContext("pollRun()", sandbox);
  await tick();
  // 前の呼び出しが終わっていないあいだは、もう一度呼んでも何もしない
  await vm.runInContext("pollRun()", sandbox);
  assert.strictEqual(metaCalls, 1, "前の更新が終わる前に重ねて呼ばれました");
  metaGate();
  await tick();
  pending.shift()(jsonRes([]));
  await first;
  // 終わったら次の更新は動く（falseに戻っている）
  assert.strictEqual(sandbox.pollRunning(), false, "runPolling が戻っていません");
  const second = vm.runInContext("pollRun()", sandbox);
  await tick();
  assert.strictEqual(metaCalls, 2, "終わったあとに更新が動きません");
  metaGate();
  await tick();
  pending.shift()(jsonRes([]));
  await second;
}

(async () => {
  await testStaleResponseDropped();
  await testCurrentResponseKept();
  await testPollNotOverlapping();
  console.log("test_run_switch: すべて通りました");
})().catch((e) => {
  console.error(e);
  process.exit(1);
});
