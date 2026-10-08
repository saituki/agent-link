"use strict";
// pi の質問ツール `ask_user.ts` の小さな関数のテスト。
//
// 補足: 拡張は TypeScript で、pi が読み込むときに型が消えるので、
// ここでは「型注釈のない小さい関数」だけを取り出して node 上で動かす。
// 取り出せなければテストが失敗する＝関数が見つからないのは異常。
// 確かめるもの:
//   - POLL_MAX_COUNT がサーバーの期限（600 秒）とそろっている（B6）
//   - 子の run だけ親に通知する（共有契約 C6）
//   - サーバーが保存した答えを pi の形に変えられる
//   - 端末で選ばれても、サーバーに答えがあればそちらを採る（B6）
//   - 答えの承認を sai-sci approval record で記録する（U04・C8）
// 実行:
//     node apps/web/tests/test_askuserphone.js

const assert = require("assert");
const fs = require("fs");
const os = require("os");
const path = require("path");
const { spawnSync } = require("child_process");

const SRC_PATH = path.join(
  __dirname,
  "..",
  "src",
  "sai_web",
  "extensions",
  "ask_user.ts"
);
const src = fs.readFileSync(SRC_PATH, "utf8");

// `const 名 = 値;` を 1 行取り出す（設定の値の確認に使う）
function extractConst(name) {
  const m = src.match(new RegExp(`^const ${name} = ([^;]+);`, "m"));
  assert.ok(m, `ask_user.ts に const ${name} が見つかりません`);
  return `const ${name} = ${m[1]};`;
}

// `function 名( … )` の本体と、仮引数の型注釈を落としたものを返す。
// 仮引数の型注釈（`: 型`）を落とすのは、node の Function で評価するため。
function extractFunction(name) {
  const start = src.search(new RegExp(`^(async )?function ${name}\\(`, "m"));
  assert.ok(start >= 0, `ask_user.ts に function ${name} が見つかりません`);
  // 仮引数の閉じ括弧まで開く括弧を数える
  let i = src.indexOf("(", start);
  let depth = 0;
  let end = -1;
  for (; i < src.length; i += 1) {
    if (src[i] === "(") depth += 1;
    else if (src[i] === ")") {
      depth -= 1;
      if (depth === 0) {
        end = i;
        break;
      }
    }
  }
  assert.ok(end > 0, `${name} の仮引数が閉じません`);
  // 戻り値の型（`): 型 {` の部分）は取り込まず、本体の `{` から始める
  const bodyStart = src.indexOf("{", end);
  assert.ok(bodyStart > 0, `${name} の本体が見つかりません`);
  let head = src.slice(start, end + 1);
  head = head.replace(/([(,]\s*)([A-Za-z_$][\w$]*)\s*:\s*[A-Za-z_$][\w$<>|.,\[\] ]*?(?=\s*[,)=])/g, "$1$2");
  let body = "";
  depth = 0;
  for (let j = bodyStart; j < src.length; j += 1) {
    if (src[j] === "{") depth += 1;
    else if (src[j] === "}") {
      depth -= 1;
      if (depth === 0) {
        body = src.slice(bodyStart, j + 1);
        break;
      }
    }
  }
  assert.ok(body, `${name} の本体が空です`);
  // 変数宣言の型注釈（`const x: 型 = …`）も落とす
  body = body.replace(/(\b(?:const|let)\s+[A-Za-z_$][\w$]*)\s*:\s*[A-Za-z_$][\w$<>|,.\[\] ]*?(?=\s*=)/g, "$1");
  return `${head} ${body}`;
}

// spawnSync は node の関数なので、引数で渡す（Function の中では require が使えない）
const api = new Function(
  "spawnSync",
  [
    extractConst("SAI_WEB_URL"),
    extractConst("SAI_WEB_INTERNAL_TOKEN"),
    extractConst("POLL_MAX_COUNT"),
    extractConst("POLL_INTERVAL_MS"),
    extractFunction("authHeaders"),
    extractFunction("shouldNotifyParent"),
    extractFunction("parentNotifyArgs"),
    extractFunction("approvalRecordArgs"),
    extractFunction("recordApproval"),
    extractFunction("answersFromState"),
    extractFunction("answeredByOf"),
    extractFunction("answeredOnServer"),
    "return { shouldNotifyParent, parentNotifyArgs, approvalRecordArgs, recordApproval, answersFromState, answeredByOf, answeredOnServer, POLL_MAX_COUNT };",
  ].join("\n\n")
)(spawnSync);

// --- 1) ポーリングの回数はサーバーの期限（questions.py の _PI_EXPIRE_SEC = 600）とそろう
assert.strictEqual(api.POLL_MAX_COUNT, 600, "POLL_MAX_COUNT が 600 ではありません（B6）");
assert.match(src, /POLL_MAX_COUNT = 600;/, "拡張の POLL_MAX_COUNT の定義が見つかりません");

// --- 2) 子の run だけ親に知らせる（C6）
assert.strictEqual(api.shouldNotifyParent({ SAI_IS_SUBTASK: "1" }), true);
assert.strictEqual(api.shouldNotifyParent({}), false, "親のない run で親を呼んではいけません");
assert.strictEqual(api.shouldNotifyParent({ SAI_IS_SUBTASK: "0" }), false);
assert.strictEqual(api.shouldNotifyParent(undefined), false);

const qs = [
  { question: "資料はどれですか", header: "資料", multiSelect: false, options: [{ label: "A" }, { label: "B" }] },
  { question: "epoch はいくつですか", header: "epoch", multiSelect: false, options: [{ label: "10" }, { label: "30" }] },
];
const cmd = api.parentNotifyArgs("q123", qs, { SAI_IS_SUBTASK: "1", SAI_LINK_BIN: "/opt/sai-link" });
assert.strictEqual(cmd.bin, "/opt/sai-link", "SAI_LINK_BIN が使われていません");
assert.deepStrictEqual(cmd.args, [
  "task",
  "question",
  "--qid",
  "q123",
  "--text",
  "資料はどれですか / epoch はいくつですか",
]);
// SAI_LINK_BIN が無ければ既定の実行ファイル名
assert.strictEqual(api.parentNotifyArgs("q1", qs, { SAI_IS_SUBTASK: "1" }).bin, "sai-link");
// 見出しは 1 行に畳んで 400 文字で切る
const long = [{ question: "あ\nい " + "う".repeat(500), header: "h", multiSelect: false, options: [] }];
const longArgs = api.parentNotifyArgs("q1", long, { SAI_IS_SUBTASK: "1" });
assert.strictEqual(longArgs.args[5].length, 400, "見出しが 400 文字で切られていません");
assert.ok(!longArgs.args[5].includes("\n"), "見出しが 1 行になっていません");

// --- 3) サーバーが保存した答えを変換する
const questions = [
  { question: "どれですか", header: "h", multiSelect: false, options: [{ label: "A" }, { label: "B" }] },
  { question: "どれ2", header: "h", multiSelect: true, options: [{ label: "い" }, { label: "ろ" }, { label: "は" }] },
  { question: "補足", header: "h", multiSelect: false, options: [{ label: "x" }] },
];
// 全問まとめて返る新形式
assert.deepStrictEqual(
  api.answersFromState({
    state: "answered",
    questions,
    answer: { answers: [{ choice_index: 1 }, { choice_indices: [0, 2] }, { text: "30 epoch で止める" }] },
  }),
  { "どれですか": "B", "どれ2": "い, は", "補足": "30 epoch で止める" }
);
// 古い 1 問だけの形（後方互換）
assert.deepStrictEqual(
  api.answersFromState({ state: "answered", questions: questions.slice(0, 1), answer: { choice_index: 0 } }),
  { "どれですか": "A" }
);
// 答えの無いときは null
assert.strictEqual(api.answersFromState({ state: "open", questions }), null);
assert.strictEqual(api.answersFromState({ state: "answered", questions }), null);

// --- 4) 端末で選ばれてもサーバーに答えがあればそちらを採る（B6）
async function main() {
  const originalFetch = global.fetch;
  const calls = [];
  try {
    // サーバーに答えがあるとき: 1 回だけ読んで変換した結果になる
    global.fetch = async (url) => {
      calls.push(url);
      return {
        ok: true,
        json: async () => ({
          state: "answered",
          questions: questions.slice(0, 1),
          answer: { answers: [{ choice_index: 0 }] },
        }),
      };
    };
    const answered = await api.answeredOnServer("q9");
    assert.deepStrictEqual(answered.answers, { "どれですか": "A" }, "Web の答えを採っていません（B6）");
    assert.strictEqual(answered.answeredBy, "user", "answered_by が無いときは user です");
    assert.strictEqual(calls.length, 1, "読みに行く回数が 1 回ではありません");
    assert.ok(calls[0].endsWith("/api/questions/q9"), `読み取った先が違います: ${calls[0]}`);

    // まだ答えられていないとき: 端末の選択を採るために null を返す
    calls.length = 0;
    global.fetch = async (url) => {
      calls.push(url);
      return { ok: true, json: async () => ({ state: "open", questions }) };
    };
    assert.strictEqual(await api.answeredOnServer("q9"), null);

    // 通信できないとき: 今までと同じ（端末の選択を採る）
    global.fetch = async () => {
      throw new Error("つながらない");
    };
    assert.strictEqual(await api.answeredOnServer("q9"), null);
  } finally {
    global.fetch = originalFetch;
  }

  // --- 5) 組み込みの確認: 通知と Web の答えの読み込みが実際の流れにあること
  assert.match(
    src,
    /const serverId = await registerToServer\([\s\S]*?if \(serverId\) notifyParent\(serverId, questions, env\);/,
    "登録のあとに親への通知が入っていません（C6）"
  );
  assert.match(
    src,
    /if \(selected\) \{[\s\S]*?const fromWeb = serverId \? await answeredOnServer\(serverId\) : null;/,
    "端末の選択の前にサーバーの答えを読んでいません（B6）"
  );
  assert.match(src, /import \{ execFile, spawnSync \} from "node:child_process";/, "execFile・spawnSync の読み込みがありません");

  // --- 6) 答えの承認の記録（U04・C8）
  // 答えた人の名前（answered_by）を読む
  global.fetch = async () => ({
    ok: true,
    json: async () => ({
      state: "answered",
      answered_by: "saituki",
      questions: questions.slice(0, 1),
      answer: { answers: [{ choice_index: 1 }] },
    }),
  });
  try {
    assert.deepStrictEqual(await api.answeredOnServer("q9"), { answers: { "どれですか": "B" }, answeredBy: "saituki" });
  } finally {
    global.fetch = originalFetch;
  }

  // 引数の組み立て: 親の run が無ければ呼ばない・あれば SAI_SCI_BIN か sai-sci
  const approvalQs = [
    { question: "次の op を行ってよいですか？", header: "許可", multiSelect: false, options: [{ label: "許可する" }, { label: "許可しない" }] },
    { question: "補足は？", header: "補足", multiSelect: false, options: [{ label: "x" }, { label: "y" }] },
  ];
  const approvalAnswers = { "次の op を行ってよいですか？": "許可する", "補足は？": "x" };
  assert.strictEqual(
    api.approvalRecordArgs("q1", approvalQs, approvalAnswers, "saituki", {}),
    null,
    "親の run が無いのに承認を記録しようとしています"
  );
  assert.deepStrictEqual(api.approvalRecordArgs("q1", approvalQs, approvalAnswers, "saituki", { SAI_PARENT_RUN_ID: "61" }), {
    bin: "sai-sci",
    args: [
      "approval",
      "record",
      "--qid",
      "q1",
      "--question",
      "次の op を行ってよいですか？\n補足は？",
      "--answer",
      "許可する\nx",
      "--answered-by",
      "saituki",
    ],
  });
  assert.strictEqual(
    api.approvalRecordArgs("q1", approvalQs, approvalAnswers, "terminal", { SAI_PARENT_RUN_ID: "61", SAI_SCI_BIN: "/opt/sai-sci" }).bin,
    "/opt/sai-sci",
    "SAI_SCI_BIN が使われていません"
  );

  // 実際に偽の sai-sci を呼ぶ（一時フォルダのシェルスクリプト）
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "ask-user-approval-"));
  try {
    const log = path.join(tmp, "args.log");
    const fake = path.join(tmp, "sai-sci");
    const writeFake = (name, code) => {
      const file = path.join(tmp, name);
      fs.writeFileSync(file, `#!/bin/sh\nprintf '%s\\n' "$@" > "${log}"\nexit ${code}\n`);
      fs.chmodSync(file, 0o755);
      return file;
    };
    const oneQ = [approvalQs[0]];
    const oneAnswer = { "次の op を行ってよいですか？": "許可する" };

    // 親の run があるとき: 渡った引数が log に残る
    writeFake("sai-sci", 0);
    api.recordApproval("q1", oneQ, oneAnswer, "saituki", { SAI_PARENT_RUN_ID: "61", SAI_SCI_BIN: fake });
    assert.deepStrictEqual(fs.readFileSync(log, "utf8").split("\n").slice(0, -1), [
      "approval",
      "record",
      "--qid",
      "q1",
      "--question",
      "次の op を行ってよいですか？",
      "--answer",
      "許可する",
      "--answered-by",
      "saituki",
    ]);

    // 親の run が無いときは呼ばない
    fs.rmSync(log, { force: true });
    api.recordApproval("q1", oneQ, oneAnswer, "saituki", { SAI_SCI_BIN: fake });
    assert.ok(!fs.existsSync(log), "親の run が無いのに sai-sci を呼びました");

    // 失敗（終了コード 4）・呼べない場所でも投げない（警告は 1 行だけ）
    const warned = [];
    const originalWarn = console.warn;
    console.warn = (...args) => warned.push(args.join(" "));
    try {
      writeFake("sai-sci", 4);
      assert.doesNotThrow(() =>
        api.recordApproval("q1", oneQ, oneAnswer, "saituki", { SAI_PARENT_RUN_ID: "61", SAI_SCI_BIN: fake })
      );
      assert.strictEqual(warned.length, 1, "失敗の警告が 1 行ではありません");
      assert.ok(warned[0].includes("終了コード 4"), warned[0]);
      assert.doesNotThrow(() =>
        api.recordApproval("q1", oneQ, oneAnswer, "saituki", { SAI_PARENT_RUN_ID: "61", SAI_SCI_BIN: path.join(tmp, "no-such") })
      );
    } finally {
      console.warn = originalWarn;
    }
  } finally {
    fs.rmSync(tmp, { recursive: true, force: true });
  }

  console.log("test_askuserphone: すべて通りました");
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
