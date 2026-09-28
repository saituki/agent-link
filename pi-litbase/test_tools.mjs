// ============================================================================
// test_tools.mjs — pi-litbase の道具が組み立てる CLI の引数と標準入力を確かめる
// ============================================================================
//
// 本物の CLI も DB も使わず、一時フォルダに偽の `sai-moon-paper` を置いて
// 組み立てた引数だけを確かめる。偽の命令は、受け取った引数を空白でつないで
// `{"args": "…"}` として標準出力へ出し、標準入力を同じフォルダの stdin.txt に書く。
//
//   node pi-litbase/test_tools.mjs

import assert from "node:assert/strict";
import { chmodSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));

/** 偽の sai-moon-paper。引数と標準入力をそのまま記録するだけのもの。 */
const FAKE = `#!/usr/bin/env node
import { appendFileSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
appendFileSync(join(here, "calls.log"), "call\\n");
let input = "";
try {
  input = readFileSync(0, "utf-8");
} catch {
  input = "";
}
writeFileSync(join(here, "stdin.txt"), input);
process.stdout.write(JSON.stringify({ args: process.argv.slice(2).join(" ") }));
`;

/** 道具の返り値を JSON として読む。 */
function readResult(result) {
  return JSON.parse(result.content[0].text);
}

const dir = mkdtempSync(join(tmpdir(), "pi-litbase-test-"));
const fakeBin = join(dir, "sai-moon-paper");
const stdinPath = join(dir, "stdin.txt");
const callsPath = join(dir, "calls.log");

// sai-link の中から走らせると SAI_PARENT_RUN_ID が入っている。実行番号が
// 無い場合を確かめるため、テストのあいだは外しておく（最後に戻す）。
const savedRunId = process.env.SAI_PARENT_RUN_ID;
delete process.env.SAI_PARENT_RUN_ID;

try {
  writeFileSync(fakeBin, FAKE);
  chmodSync(fakeBin, 0o755);

  // 偽の命令を先に置いて、道具が組み立てる引数と標準入力を見る。
  process.env.SAI_PAPER_BIN = fakeBin;
  process.env.SAI_PAPER_ROOT = dir;

  const { findTool, sciProposeTool, sciContextTool, litbaseTools } = await import(
    join(here, "tools.ts")
  );

  // 1. find の新しい引数が `--papers 37,40` の形に組まれる。
  const found = readResult(
    await findTool.execute("call-1", {
      query: "eeg",
      papers: [37, 40],
      node: "N7",
      subtree: true,
      each: 2,
      question: "Q3",
    }),
  );
  assert.equal(
    found.args,
    "find eeg --papers 37,40 --node N7 --subtree --each 2 --question Q3 --json",
  );

  // 2. 論文の集合が空なら `--papers` は付けない（空文字列を送ると CLI が怒る）。
  const noPapers = readResult(
    await findTool.execute("call-1b", { query: "eeg", papers: [], each: 2 }),
  );
  assert.equal(noPapers.args, "find eeg --each 2 --json");

  // 3. propose は --run を付け、標準入力に提案の JSON を書く。
  const proposals = [
    { op: "add_question", node: "N7", text: "問いの文", reason: "理由" },
  ];
  const proposed = readResult(
    await sciProposeTool.execute("call-2", { proposals, run_id: 5 }),
  );
  assert.equal(proposed.args, "sci propose --run 5");
  assert.equal(readFileSync(stdinPath, "utf-8"), JSON.stringify({ proposals }));

  // 4. 実行番号が無くても propose は --run を付けずに呼ぶ（run 不明として扱う）。
  const unknownRun = readResult(
    await sciProposeTool.execute("call-3", { proposals }),
  );
  assert.equal(unknownRun.args, "sci propose");

  // 5. context は実行番号が無いなら CLI を呼ばずに落とす。
  const callsBefore = readFileSync(callsPath, "utf-8").split("\n").length - 1;
  const noRun = readResult(await sciContextTool.execute("call-4", {}));
  assert.equal(noRun.error, "実行番号がありません（run_id か環境変数 SAI_PARENT_RUN_ID）");
  const callsAfter = readFileSync(callsPath, "utf-8").split("\n").length - 1;
  assert.equal(callsAfter, callsBefore, "context は実行番号が無いなら CLI を呼ばない");

  // 6. context は環境変数の実行番号を使う。
  process.env.SAI_PARENT_RUN_ID = "7";
  const fromEnv = readResult(
    await sciContextTool.execute("call-5", { scope: "path" }),
  );
  assert.equal(fromEnv.args, "sci context --run 7 --scope path");
  delete process.env.SAI_PARENT_RUN_ID;

  // 7. 道具の名前に重なりが無い。
  const names = litbaseTools.map((t) => t.name);
  assert.equal(new Set(names).size, names.length, `道具名が重なる: ${names.join(", ")}`);

  for (const name of [
    "sai_paper_sci_context",
    "sai_paper_sci_status",
    "sai_paper_sci_node",
    "sai_paper_sci_propose",
  ]) {
    assert.ok(names.includes(name), `${name} が登録されていない`);
  }

  console.log("test_tools: すべて通った");
} finally {
  if (savedRunId !== undefined) {
    process.env.SAI_PARENT_RUN_ID = savedRunId;
  }
  rmSync(dir, { recursive: true, force: true });
}
