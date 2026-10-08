"use strict";
// Download の見張り（paperimport.js）の、DOM を使わない関数のテスト（assert だけ）。
// 実行:
//     node tests/test_paperimport.js

const assert = require("assert");
const path = require("path");

const {
  PIM_POLL_MS,
  pimLogKey,
  pimJobFor,
  pimActiveJobs,
  pimJobLabel,
  pimJobTone,
  pimJobErrorNotFound,
  pimLongText,
  pimKeepMessage,
} = require(path.join(__dirname, "..", "src", "sai_web", "static", "paperimport.js"));

// .bib の名前（サーバの _BIB_NAME_RE と同じ規則）
function testKeys() {
  assert.strictEqual(pimLogKey(3, 2), "log:3:2");
  assert.strictEqual(pimLogKey("3", "2"), "log:3:2");
}

function testJobFor() {
  const jobs = [
    { key: "log:1:2", status: "downloading" },
    { key: "import:DOI:x", status: "done", paper_id: 7 },
  ];
  assert.strictEqual(pimJobFor(jobs, "log:1:2").status, "downloading");
  assert.strictEqual(pimJobFor(jobs, "import:DOI:x").paper_id, 7);
  assert.strictEqual(pimJobFor(jobs, "import:DOI:y"), null);
  assert.strictEqual(pimJobFor(null, "log:1:2"), null);
}

function testActiveJobs() {
  const jobs = [
    { key: "a", status: "downloading" },
    { key: "b", status: "processing" },
    { key: "c", status: "done" },
    { key: "d", status: "failed" },
  ];
  assert.deepStrictEqual(pimActiveJobs(jobs).map((j) => j.key), ["a", "b"]);
  assert.deepStrictEqual(pimActiveJobs([]), []);
  assert.deepStrictEqual(pimActiveJobs(null), []);
  // 壊れた値は無視
  assert.deepStrictEqual(pimActiveJobs([null, { status: "downloading" }, { key: "x" }]).length, 1);
  // 走っている仕事のあいだだけ、3 秒ごとに読み直す
  assert.strictEqual(PIM_POLL_MS, 3000);
}

function testJobLabelAndTone() {
  assert.strictEqual(pimJobLabel("downloading"), "取り込み中…");
  assert.strictEqual(pimJobLabel("processing"), "保存しました。図の取り出しと翻訳をしています");
  assert.strictEqual(pimJobLabel("processing", "bundle"), "前処理の束を取り込んでいます");
  assert.strictEqual(pimJobLabel("done"), "保存しました");
  assert.strictEqual(pimJobLabel("failed"), "保存できませんでした");
  assert.strictEqual(pimJobLabel(null), "");
  assert.strictEqual(pimJobTone("done"), "ok");
  assert.strictEqual(pimJobTone("failed"), "danger");
  assert.strictEqual(pimJobTone("downloading"), "busy");
  assert.strictEqual(pimJobTone(null), "");
}

function testJobErrorNotFound() {
  // 「PDF の場所が見つかりません…」のときだけ、論文のサイトを開く案内を出す
  assert.strictEqual(pimJobErrorNotFound("PDF の場所が見つかりません。DOI から取ってください"), true);
  assert.strictEqual(pimJobErrorNotFound("PDF を取れませんでした: 500"), false);
  assert.strictEqual(pimJobErrorNotFound(""), false);
  assert.strictEqual(pimJobErrorNotFound(null), false);
}

function testLongText() {
  // 40 行を超える md は「はじめはたたんで全部見る」にする
  const short = Array.from({ length: 40 }, (_, i) => `行 ${i + 1}`).join("\n");
  const long = `${short}\n41 行目`;
  assert.strictEqual(pimLongText(short).long, false);
  assert.strictEqual(pimLongText(long).long, true);
  assert.strictEqual(pimLongText(long).lines, 41);
  // たたんだときに見せるのは最初の 8 行
  assert.strictEqual(pimLongText(long).peek.split("\n").length, 8);
  assert.strictEqual(pimLongText(long).peek, "行 1\n行 2\n行 3\n行 4\n行 5\n行 6\n行 7\n行 8");
  // 空・短いもの
  assert.strictEqual(pimLongText("").long, false);
  assert.strictEqual(pimLongText(null).long, false);
  assert.strictEqual(pimLongText("1 行だけ").lines, 1);
}

// --- Library に入れる（U13 §2） ---

function testKeep() {
  // 結果 → 知らせる文
  assert.strictEqual(
    pimKeepMessage("added"),
    "Library に入れました（PDF は sai-prep から送ってください）"
  );
  assert.strictEqual(pimKeepMessage("exists"), "もう Library にあります");
  assert.strictEqual(
    pimKeepMessage(null),
    "Library に入れました（PDF は sai-prep から送ってください）"
  );
}

function main() {
  testKeys();
  testJobFor();
  testActiveJobs();
  testJobLabelAndTone();
  testJobErrorNotFound();
  testLongText();
  testKeep();
  console.log("test_paperimport: すべて通りました");
}

main();
