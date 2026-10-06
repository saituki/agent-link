"use strict";
// U03: Library 内の検索（内部 DB）の DOM を使わない関数のテスト。
// 実行:
//     node tests/test_papers_find.js

const assert = require("assert");
const path = require("path");

const {
  lfindBody,
  lfindSummary,
  findHash,
  parseFindHash,
  parseLibraryHash,
  libraryFindHash,
} = require(path.join(
  __dirname,
  "..",
  "src",
  "sai_web",
  "static",
  "papers.js"
));

function testLfindBodyOk() {
  // 既定（検索語だけ）
  assert.deepStrictEqual(
    lfindBody({ query: "epilepsy EEG", mode: "", scope: "", count: 10, papers: [], node: "", subtree: false, each: "" }),
    { ok: true, body: { query: "epilepsy EEG", top_k: 10 }, error: "" }
  );
  // 全部指定
  assert.deepStrictEqual(
    lfindBody({ query: " eeg ", mode: "hybrid", scope: "all", count: 20, papers: [3, 7], node: "Q12", subtree: true, each: "5" }),
    {
      ok: true,
      body: { query: "eeg", mode: "hybrid", scope: "all", top_k: 20, papers: [3, 7], node: "Q12", subtree: true, each: 5 },
      error: "",
    }
  );
  // subtree が false のときは送らない
  assert.deepStrictEqual(
    lfindBody({ query: "eeg", mode: "", scope: "", count: 10, papers: [], node: "Q12", subtree: false, each: "" }).body,
    { query: "eeg", top_k: 10, node: "Q12" }
  );
  // Node は Q・O・F・E のどれでも送れる（契約の形）
  for (const id of ["O5", "F3", "E2"]) {
    assert.strictEqual(
      lfindBody({ query: "eeg", mode: "", scope: "", count: 10, papers: [], node: id, subtree: false, each: "" }).body.node,
      id
    );
  }
  assert.strictEqual(
    lfindBody({ query: "eeg", mode: "", scope: "", count: 10, papers: [], node: "X1", subtree: false, each: "" }).ok,
    false
  );
  // each は論文だけでも使える
  assert.deepStrictEqual(
    lfindBody({ query: "eeg", mode: "", scope: "", count: 10, papers: [3], node: "", subtree: false, each: "3" }).body.each,
    3
  );
}

function testLfindBodyNg() {
  // 空
  const empty = lfindBody({ query: "   ", mode: "", scope: "", count: 10, papers: [], node: "", subtree: false, each: "" });
  assert.strictEqual(empty.ok, false);
  assert.ok(empty.error);
  // 201 字
  const long = lfindBody({ query: "あ".repeat(201), mode: "", scope: "", count: 10, papers: [], node: "", subtree: false, each: "" });
  assert.strictEqual(long.ok, false);
  // 200 字は通る
  assert.strictEqual(
    lfindBody({ query: "あ".repeat(200), mode: "", scope: "", count: 10, papers: [], node: "", subtree: false, each: "" }).ok,
    true
  );
  // 不正な mode・scope
  assert.strictEqual(lfindBody({ query: "a", mode: "bad", scope: "", count: 10, papers: [], node: "", subtree: false, each: "" }).ok, false);
  assert.strictEqual(lfindBody({ query: "a", mode: "", scope: "bad", count: 10, papers: [], node: "", subtree: false, each: "" }).ok, false);
  // count が範囲外
  assert.strictEqual(lfindBody({ query: "a", mode: "", scope: "", count: 0, papers: [], node: "", subtree: false, each: "" }).ok, false);
  assert.strictEqual(lfindBody({ query: "a", mode: "", scope: "", count: 101, papers: [], node: "", subtree: false, each: "" }).ok, false);
  // 論文が 21 本
  const many = Array.from({ length: 21 }, (_, i) => i + 1);
  assert.strictEqual(lfindBody({ query: "a", mode: "", scope: "", count: 10, papers: many, node: "", subtree: false, each: "" }).ok, false);
  // Node の形が違う
  assert.strictEqual(lfindBody({ query: "a", mode: "", scope: "", count: 10, papers: [], node: "12", subtree: false, each: "" }).ok, false);
  assert.strictEqual(lfindBody({ query: "a", mode: "", scope: "", count: 10, papers: [], node: "Q0", subtree: false, each: "" }).ok, false);
  // each が範囲なし・0・21・数字でない
  for (const each of ["3", "0", "21", "abc"]) {
    const r = lfindBody({ query: "a", mode: "", scope: "", count: 10, papers: [], node: "", subtree: false, each });
    assert.strictEqual(r.ok, false, `each=${each}`);
    assert.ok(r.error);
  }
  // each ありで Node も論文も無い
  assert.strictEqual(lfindBody({ query: "a", mode: "", scope: "", count: 10, papers: [], node: "", subtree: false, each: "3" }).ok, false);
}

function testSummary() {
  assert.strictEqual(lfindSummary({ mode: "hybrid", scope: "all", count: 10, node: "Q12" }), "両方 · 全部 · 10 件 · Q12");
  assert.strictEqual(lfindSummary({ mode: "", scope: "", count: 10, node: "" }), "10 件");
  assert.strictEqual(lfindSummary({ mode: "word", scope: "papers", count: 20, node: "" }), "単語 · 論文 · 20 件");
}

function testHash() {
  assert.strictEqual(findHash("sai-paper", 12), "#/papers/find/sai-paper/12");
  assert.deepStrictEqual(parseFindHash("#/papers/find/sai-paper/12"), { name: "sai-paper", id: "12" });
  assert.strictEqual(parseFindHash("#/papers/find/sai-paper/"), null);
  assert.strictEqual(parseFindHash("#/papers/find/sai-paper/abc"), null);
  assert.strictEqual(parseFindHash("#/papers/log/sai-paper/12"), null);
  assert.strictEqual(libraryFindHash(12), "#/papers/library/find/12");
  // parseLibraryHash に find が入った形
  assert.deepStrictEqual(parseLibraryHash("#/papers/library/find/12"), { paper: null, log: null, find: 12 });
  assert.deepStrictEqual(parseLibraryHash("#/papers/library/log/12"), { paper: null, log: 12, find: null });
  assert.deepStrictEqual(parseLibraryHash("#/papers/library"), { paper: null, log: null, find: null });
}

function main() {
  testLfindBodyOk();
  testLfindBodyNg();
  testSummary();
  testHash();
  console.log("test_papers_find: すべて通りました");
}

main();
