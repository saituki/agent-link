"use strict";
// 引用一覧の画面（cite.js）の、DOM を使わない関数のテスト。
// 手動取り込みの一枚の純関数（pimManualQuery / pimManualKey / pimCheckUpload）と
// 進み具合の一枚の種類表示は tests/test_paperimport.js で確かめている。
// 実行:
//     node tests/test_cite.js

const assert = require("assert");
const path = require("path");

const staticDir = path.join(__dirname, "..", "src", "sai_phone", "static");
const cite = require(path.join(staticDir, "cite.js"));
const papers = require(path.join(staticDir, "papers.js"));

// --- 住所 ---

function testHash() {
  assert.deepStrictEqual(cite.ctParseHash("#/paper/test-moon/12/cites"), {
    name: "test-moon",
    id: "12",
  });
  assert.strictEqual(cite.ctHash("test-moon", 12), "#/paper/test-moon/12/cites");
  // 日本語や空白の入ったプロジェクト名も往復する
  const name = "本 の 置き場";
  assert.deepStrictEqual(cite.ctParseHash(cite.ctHash(name, 7)), { name, id: "7" });
  // 别的 画面ではない
  for (const bad of [
    "",
    "#/paper/test-moon/12",
    "#/paper/test-moon/12/text",
    "#/paper/12/cites",
    "#/paper/test-moon/abc/cites",
    "#/papers/log/test-moon/1/2",
    "#/paper/test-moon/12/cites/3",
  ]) {
      assert.strictEqual(cite.ctParseHash(bad), null, bad);
  }
  // papers.js の論文情報・本文の住所は cites を読まない（画面の並び順に依存しない）
  assert.strictEqual(papers.parsePaperHash("#/paper/test-moon/12/cites"), null);
}

function testExtractJob() {
  const jobs = [
    { key: "cite:12", status: "processing", step: 1, percent: 50 },
    { key: "cite:13", status: "done", step: 2, percent: 100 },
  ];
  assert.strictEqual(cite.ctExtractJob(jobs, 12).status, "processing");
  assert.strictEqual(cite.ctExtractJob(jobs, 13).status, "done");
  assert.strictEqual(cite.ctExtractJob(jobs, 99), null);
  assert.strictEqual(cite.ctExtractJob(null, 12), null);
}

// --- 一覧の行 ---

const REFS = [
  {
    ref_no: 1,
    title: "Attention Is All You Need",
    first_author: "Vaswani",
    year: 2017,
    journal: "NeurIPS",
    local_paper_id: 34,
    doi: "10.5555/3295222",
    url: "https://arxiv.org/abs/1706.03762",
    can_download: true,
    can_import: true,
    raw_text: "Vaswani et al. 2017",
    sentences: [
      { sent_no: 5, text: "We propose the Transformer." },
      { sent_no: 9, text: "Vaswani ら好自己の構造带来了。" },
    ],
  },
  {
    ref_no: 2,
    title: "BERT: Pre-training of Deep Bidirectional Transformers",
    first_author: "Devlin",
    year: 2019,
    journal: null,
    local_paper_id: null,
    doi: null,
    url: null,
    can_download: false,
    can_import: true,
    raw_text: "Devlin, Chang, Lee, Toutanova. 2019.",
    sentences: [],
  },
];

function testInLibrary() {
  assert.strictEqual(cite.ctInLibrary(REFS[0]), true);
  assert.strictEqual(cite.ctInLibrary(REFS[1]), false);
  // local_paper_id が 0 のときは「無い」扱い（0 番は論文ではない）
  assert.strictEqual(cite.ctInLibrary({ local_paper_id: 0 }), false);
  assert.strictEqual(cite.ctInLibrary({ local_paper_id: "0" }), false);
  assert.strictEqual(cite.ctInLibrary({}), false);
  assert.strictEqual(cite.ctInLibrary(null), false);
}

function testFilter() {
  // 大文字小文字を無視した部分一致（cite TUI の filtered_indices と同じ）
  assert.deepStrictEqual(cite.ctFilter(REFS, "attention").map((r) => r.ref_no), [1]);
  assert.deepStrictEqual(cite.ctFilter(REFS, "VASWANI").map((r) => r.ref_no), [1]);
  assert.deepStrictEqual(cite.ctFilter(REFS, "2019").map((r) => r.ref_no), [2]);
  // 著者
  assert.deepStrictEqual(cite.ctFilter(REFS, "devlin").map((r) => r.ref_no), [2]);
  // raw_text にも当たる
  assert.deepStrictEqual(cite.ctFilter(REFS, "toutanova").map((r) => r.ref_no), [2]);
  // 空・空白だけなら全部を返し、前後の空白は無視する
  assert.strictEqual(cite.ctFilter(REFS, "").length, 2);
  assert.strictEqual(cite.ctFilter(REFS, "   ").length, 2);
  assert.strictEqual(cite.ctFilter(REFS, " BERT ").map((r) => r.ref_no).join(), "2");
  assert.strictEqual(cite.ctFilter(REFS, "該当なし").length, 0);
  // 壊れた値でも落ちない
  assert.strictEqual(cite.ctFilter(null, "x").length, 0);
  assert.strictEqual(cite.ctFilter([null, {}], "").length, 2);
}

function testMetaLine() {
  assert.strictEqual(cite.ctMetaLine(REFS[0]), "Vaswani · 2017 · NeurIPS · 本文で 2 回");
  assert.strictEqual(cite.ctMetaLine(REFS[1]), "Devlin · 2019 · 本文では引用なし");
  // 著者も年も掲載誌も無くても「本文では引用なし」は出る
  assert.strictEqual(cite.ctMetaLine({ sentences: [{ sent_no: 1, text: "x" }] }), "本文で 1 回");
  assert.strictEqual(cite.ctMetaLine(null), "本文では引用なし");
}

function testSentenceText() {
  assert.strictEqual(
    cite.ctSentenceText(REFS[0].sentences[0]),
    "[5] We propose the Transformer."
  );
  // 300 字で切って「…」を付ける
  const long = { sent_no: 3, text: "あ".repeat(cite.CT_SENTENCE_MAX + 50) };
  const out = cite.ctSentenceText(long);
  assert.strictEqual(out.slice(0, 4), "[3] ");
  assert.strictEqual(out.length, "[3] ".length + cite.CT_SENTENCE_MAX + 1, out.length);
  assert.ok(out.endsWith("…"), out.slice(-5));
}

function testSiteUrl() {
  // url があればそれをそのまま使う（cite TUI の do_open_pdf と同じ）
  assert.strictEqual(cite.ctSiteUrl(REFS[0]), "https://arxiv.org/abs/1706.03762");
  // 無ければ DOI
  assert.strictEqual(cite.ctSiteUrl({ doi: "10.5555/1" }), "https://doi.org/10.5555/1");
  // どちらも無ければ題名の Google Scholar 検索
  assert.strictEqual(
    cite.ctSiteUrl({ title: "Attention Is All You Need" }),
    "https://scholar.google.com/scholar?q=Attention%20Is%20All%20You%20Need"
  );
  // 什么都没有ならリンクは出ない
  assert.strictEqual(cite.ctSiteUrl({}), "");
  assert.strictEqual(cite.ctSiteUrl(null), "");
}

function testExtracted() {
  assert.strictEqual(cite.ctExtracted({ extracted: true }), true);
  assert.strictEqual(cite.ctExtracted({ extracted: false }), false);
  assert.strictEqual(cite.ctExtracted(null), false);
  assert.strictEqual(cite.ctExtracted({}), false);
}

function main() {
  testHash();
  testExtractJob();
  testInLibrary();
  testFilter();
  testMetaLine();
  testSentenceText();
  testSiteUrl();
  testExtracted();
  console.log("test_cite: すべて通りました");
}

main();
