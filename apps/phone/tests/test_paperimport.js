"use strict";
// Papers の Import（paperimport.js）と Download の見張りの、DOM を使わない関数のテスト（assert だけ）。
// 実行:
//     node tests/test_paperimport.js

const assert = require("assert");
const path = require("path");

const {
  PIM_POLL_MS,
  PIM_ATTENTION_KINDS,
  pimValidName,
  pimFileNameFor,
  pimAuthors,
  pimMetaLine,
  pimFilter,
  pimImportKey,
  pimLogKey,
  pimJobFor,
  pimActiveJobs,
  pimJobLabel,
  pimJobTone,
  pimJobErrorNotFound,
  pimAttentionKindLabel,
  pimLongText,
} = require(path.join(__dirname, "..", "src", "sai_phone", "static", "paperimport.js"));

// .bib の名前（サーバの _BIB_NAME_RE と同じ規則）
function testValidName() {
  assert.strictEqual(pimValidName("refs.bib"), true);
  assert.strictEqual(pimValidName("a-b_C.1.bib"), true);
  assert.strictEqual(pimValidName(`${"a".repeat(80)}.bib`), true); // 名前は 80 文字まで
  // 違うもの
  assert.strictEqual(pimValidName(""), false);
  assert.strictEqual(pimValidName("refs"), false); // .bib ではない
  assert.strictEqual(pimValidName("refs.bibx"), false);
  assert.strictEqual(pimValidName("refs.bib "), false);
  assert.strictEqual(pimValidName("なまえ.bib"), false); // 日本語は不可
  assert.strictEqual(pimValidName("a/b.bib"), false);
  assert.strictEqual(pimValidName(null), false);
  assert.strictEqual(pimValidName(12), false);
  assert.strictEqual(pimValidName(`${"a".repeat(81)}.bib`), false); // 81 文字は不可
}

function testFileNameFor() {
  // 規則に合うファイル名はその名前、合わなければ空（サーバが phone-<日時>.bib を作る）
  assert.strictEqual(pimFileNameFor("refs.bib"), "refs.bib");
  assert.strictEqual(pimFileNameFor("Refs-2026.bib"), "Refs-2026.bib");
  assert.strictEqual(pimFileNameFor("refs.txt"), "");
  assert.strictEqual(pimFileNameFor("refs"), "");
  assert.strictEqual(pimFileNameFor(null), "");
}

function testAuthors() {
  // 先頭の 3 人。4 人以上は「ほか」を付ける
  assert.strictEqual(pimAuthors(["A"]), "A");
  assert.strictEqual(pimAuthors(["A", "B"]), "A、B");
  assert.strictEqual(pimAuthors(["A", "B", "C"]), "A、B、C");
  assert.strictEqual(pimAuthors(["A", "B", "C", "D"]), "A、B、C ほか");
  // 文字列でも著者名でもよい（壊れていても止まらない）
  assert.strictEqual(pimAuthors("A, B, C, D"), "A、B、C ほか");
  assert.strictEqual(pimAuthors(""), "");
  assert.strictEqual(pimAuthors(null), "");
  assert.strictEqual(pimAuthors([null, "  ", "B"]), "B");
}

function testMetaLine() {
  assert.strictEqual(pimMetaLine({ year: "2017", journal: "ArXiv" }), "2017 · ArXiv");
  assert.strictEqual(pimMetaLine({}), "");
  assert.strictEqual(pimMetaLine({ year: "", journal: null }), "");
  assert.strictEqual(
    pimMetaLine({ year: "2017", journal: "ArXiv", citation_count: 3 }),
    "2017 · ArXiv · 被引用 3"
  );
}

function testFilter() {
  const entries = [
    { id: "DOI:10.1000/a", title: "Attention Is All You Need", authors: ["Vaswani"] },
    { id: "DOI:10.1000/b", title: "SpongeNet", authors: ["Li Wang", "Jane"] },
    { id: "DOI:10.1000/c", title: "Retriever", authors: ["Bob"] },
  ];
  const ids = (q) => pimFilter(entries, q).map((e) => e.id);
  // 空欄は全部
  assert.deepStrictEqual(ids(""), ["DOI:10.1000/a", "DOI:10.1000/b", "DOI:10.1000/c"]);
  assert.deepStrictEqual(ids("   "), ["DOI:10.1000/a", "DOI:10.1000/b", "DOI:10.1000/c"]);
  // 題名・著者のどちらにも当たる（空白区切り・大文字小文字は無視）
  assert.deepStrictEqual(ids("sponge"), ["DOI:10.1000/b"]);
  assert.deepStrictEqual(ids("SPONGE"), ["DOI:10.1000/b"]);
  assert.deepStrictEqual(ids("vaswani"), ["DOI:10.1000/a"]);
  assert.deepStrictEqual(ids("sponge wang"), ["DOI:10.1000/b"]);
  assert.deepStrictEqual(ids("  sponge   li "), ["DOI:10.1000/b"]);
  // 1 語でも合わなければ空
  assert.deepStrictEqual(ids("retriever bob"), ["DOI:10.1000/c"]);
  assert.deepStrictEqual(ids("ありません"), []);
  // 壊れた値でも動く
  assert.deepStrictEqual(pimFilter(null, "x"), []);
  assert.deepStrictEqual(pimFilter([{ id: 1 }], "a"), []);
}

function testKeys() {
  assert.strictEqual(pimImportKey("DOI:10.1000/xyz"), "import:DOI:10.1000/xyz");
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
  assert.strictEqual(pimJobLabel("downloading"), "ダウンロード中…");
  assert.strictEqual(pimJobLabel("processing"), "保存しました。図の取り出しと翻訳をしています");
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

function testAttentionKindLabel() {
  assert.strictEqual(pimAttentionKindLabel("empty_review"), "書いていない review");
  assert.strictEqual(pimAttentionKindLabel("question"), "ユーザーへの質問");
  assert.strictEqual(pimAttentionKindLabel("state_update"), "現況の更新案");
  assert.strictEqual(pimAttentionKindLabel("recommend_card"), "推薦のカード");
  assert.strictEqual(pimAttentionKindLabel("report_issue"), "出典の要確認");
  assert.strictEqual(pimAttentionKindLabel("new_set_items"), "検索セットの新着");
  // 知らない種類は札だけを出し、説明は 「要対応」 に落とす
  assert.strictEqual(pimAttentionKindLabel("mystery"), "要対応");
  assert.strictEqual(pimAttentionKindLabel(null), "要対応");
  // 札の 6 種類が揃っていること（toast と札の両方で使う）
  assert.strictEqual(Object.keys(PIM_ATTENTION_KINDS).length, 6);
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

function main() {
  testValidName();
  testFileNameFor();
  testAuthors();
  testMetaLine();
  testFilter();
  testKeys();
  testJobFor();
  testActiveJobs();
  testJobLabelAndTone();
  testJobErrorNotFound();
  testAttentionKindLabel();
  testLongText();
  console.log("test_paperimport: すべて通りました");
}

main();
