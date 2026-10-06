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
  pimProgressView,
  pimElapsedLabel,
  PIM_UPLOAD_MAX,
  pimJobKindLabel,
  pimManualQuery,
  pimManualKey,
  pimTargetFromKey,
  pimCheckUpload,
  pimFileSize,
  pimJobTarget,
} = require(path.join(__dirname, "..", "src", "sai_web", "static", "paperimport.js"));

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
  assert.strictEqual(pimAttentionKindLabel("empty_review"), "Review not written");
  assert.strictEqual(pimAttentionKindLabel("question"), "Question for the user");
  assert.strictEqual(pimAttentionKindLabel("state_update"), "Proposed State update");
  assert.strictEqual(pimAttentionKindLabel("recommend_card"), "Recommendation card");
  assert.strictEqual(pimAttentionKindLabel("report_issue"), "Source needs checking");
  assert.strictEqual(pimAttentionKindLabel("new_set_items"), "New items in a search set");
  // U19: 研究方針の木から出る要対応
  assert.strictEqual(pimAttentionKindLabel("sci_decision"), "Q has no star");
  assert.strictEqual(pimAttentionKindLabel("sci_triage"), "PROPOSAL awaiting TRIAGE");
  // 知らない種類は札だけを出し、説明は 「要対応」 に落とす
  assert.strictEqual(pimAttentionKindLabel("mystery"), "Attention");
  assert.strictEqual(pimAttentionKindLabel(null), "Attention");
  // 札の 8 種類が揃っていること（toast と札の両方で使う）
  assert.strictEqual(Object.keys(PIM_ATTENTION_KINDS).length, 8);
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

function testProgressView() {
  // 段 0 / 1 / 2 / 3 → 0% / 33% / 66% / 100%（サーバの DOWNLOAD_STEPS と同じ）
  const steps = ["PDF取得", "図抽出", "翻訳"];
  for (const [step, percent, label] of [
    [0, 0, "PDF取得"],
    [1, 33, "図抽出"],
    [2, 66, "翻訳"],
    [3, 100, undefined],
  ]) {
    const v = pimProgressView(
      {
        status: step === 0 ? "downloading" : step === 3 ? "done" : "processing",
        steps,
        step,
        percent,
        messages: [],
        started_at: "2026-09-27T10:00:00+09:00",
        finished_at: step === 3 ? "2026-09-27T10:03:12+09:00" : null,
      },
      Date.parse("2026-09-27T10:00:00+09:00")
    );
    assert.strictEqual(v.percent, percent, `${step} 段の %`);
    if (label) assert.strictEqual(v.stepLabel, label, `${step} 段の名`);
    else assert.strictEqual(v.stepLabel, "完了", `${step} 段の名`);
  }
  // 経過時間（終わっていれば finished_at まで・_running 中は今まで）
  const running = pimProgressView(
    { steps, step: 1, percent: 33, started_at: "2026-09-27T10:00:00+09:00" },
    Date.parse("2026-09-27T10:00:00+09:00") + 192000
  );
  assert.strictEqual(running.elapsed, "3:12", running.elapsed);
  // ログは最新 5 行。残りは allLines に全部残る
  const messages = Array.from({ length: 8 }, (_, i) => `行 ${i + 1}`);
  const logs = pimProgressView(
    { steps, step: 2, percent: 66, messages, started_at: "2026-09-27T10:00:00+09:00" },
    Date.parse("2026-09-27T10:00:00+09:00")
  );
  assert.strictEqual(logs.lines.length, 5, logs.lines);
  assert.strictEqual(logs.lines[0], "行 4", logs.lines[0]);
  assert.strictEqual(logs.allLines.length, 8, logs.allLines.length);
  // 壊れた値でも止まらない
  const bad = pimProgressView(null, Date.now());
  assert.strictEqual(bad.percent, 0, bad.percent);
  assert.strictEqual(bad.stepLabel, "", bad.stepLabel);
  assert.deepStrictEqual(bad.lines, [], bad.lines);
  assert.strictEqual(pimProgressView({ percent: 300 }, 0).percent, 100);
  assert.strictEqual(pimProgressView({ percent: -5 }, 0).percent, 0);
}

function testElapsedLabel() {
  const start = "2026-09-27T10:00:00+09:00";
  const base = Date.parse(start);
  assert.strictEqual(pimElapsedLabel(start, base + 12000), "0:12");
  assert.strictEqual(pimElapsedLabel(start, base + 3723000), "1:02:03");
  // 読めない時刻は空文字、時計が戻ったときは 0
  assert.strictEqual(pimElapsedLabel("", base), "");
  assert.strictEqual(pimElapsedLabel(null, base), "");
  assert.strictEqual(pimElapsedLabel("壊れた", base), "");
  assert.strictEqual(pimElapsedLabel(start, base - 5000), "0:00");
}

// --- 手動で取り込むときの対象（U05 §5） ---

function testManualQuery() {
  assert.strictEqual(pimManualQuery({ log_id: 3, position: 2 }), "?log_id=3&position=2");
  assert.strictEqual(pimManualQuery({ import_id: "DOI:10.1/x" }), "?import_id=DOI%3A10.1%2Fx");
  assert.strictEqual(pimManualQuery({ cite_paper_id: 12, ref_no: 3 }), "?cite_paper_id=12&ref_no=3");
  assert.strictEqual(pimManualQuery({ paper_id: 12 }), "?paper_id=12");
  // 題名は任意（あれば付ける）
  assert.strictEqual(pimManualQuery({ paper_id: 12 }, "題名"), "?paper_id=12&title=%E9%A1%8C%E5%90%8D");
  // 対象が無ければ空文字（URL を作らない）
  assert.strictEqual(pimManualQuery({}), "");
  assert.strictEqual(pimManualQuery(null), "");
  // 対象が 2 つあっても 1 つだけ送る（log を優先）
  assert.strictEqual(
    pimManualQuery({ log_id: 1, position: 1, cite_paper_id: 2, ref_no: 3 }),
    "?log_id=1&position=1"
  );
}

function testManualKey() {
  // サーバの manual:<対象のキー> と同じ形
  assert.strictEqual(pimManualKey({ log_id: 3, position: 2 }), "manual:log:3:2");
  assert.strictEqual(pimManualKey({ import_id: "DOI:1" }), "manual:import:DOI:1");
  assert.strictEqual(pimManualKey({ cite_paper_id: 12, ref_no: 3 }), "manual:cite:12:3");
  assert.strictEqual(pimManualKey({ paper_id: 12 }), "manual:paper:12");
  assert.strictEqual(pimManualKey({}), "");
}

function testTargetFromKey() {
  // ダウンロードのキーから「手動で取り込む」の対象を読み戻す
  assert.deepStrictEqual(pimTargetFromKey("log:3:2"), { log_id: 3, position: 2 });
  assert.deepStrictEqual(pimTargetFromKey("import:DOI:1"), { import_id: "DOI:1" });
  assert.deepStrictEqual(pimTargetFromKey("cite:12:3"), { cite_paper_id: 12, ref_no: 3 });
  assert.deepStrictEqual(pimTargetFromKey("paper:12"), { paper_id: 12 });
  assert.strictEqual(pimTargetFromKey("radio:9"), null);
  assert.strictEqual(pimTargetFromKey("cite-extract:1"), null);
  assert.strictEqual(pimTargetFromKey(""), null);
  // 往復：キー → 対象 → キーが戻る
  for (const target of [
    { log_id: 3, position: 2 },
    { import_id: "DOI:1" },
    { cite_paper_id: 12, ref_no: 3 },
    { paper_id: 12 },
  ]) {
    assert.deepStrictEqual(
      pimTargetFromKey(pimManualKey(target)),
      target,
      JSON.stringify(target)
    );
  }
}

function testCheckUpload() {
  assert.deepStrictEqual(
    pimCheckUpload({ size: 1024, type: "application/pdf", name: "a.pdf" }),
    { ok: true, error: "" }
  );
  // 種類が無くても名前が .pdf なら通す
  assert.strictEqual(pimCheckUpload({ size: 10, type: "", name: "a.PDF" }).ok, true);
  // 空
  assert.strictEqual(pimCheckUpload(null).ok, false);
  assert.strictEqual(pimCheckUpload({ size: 0, type: "application/pdf", name: "a.pdf" }).ok, false);
  // 100MB まで（上限そのものまでは通す）
  assert.strictEqual(
    pimCheckUpload({ size: PIM_UPLOAD_MAX, type: "application/pdf", name: "a.pdf" }).ok,
    true
  );
  const over = pimCheckUpload({ size: PIM_UPLOAD_MAX + 1, type: "application/pdf", name: "a.pdf" });
  assert.strictEqual(over.ok, false);
  assert.ok(over.error.includes("100MB"), over.error);
  // PDF 以外
  const notPdf = pimCheckUpload({ size: 10, type: "text/plain", name: "a.txt" });
  assert.strictEqual(notPdf.ok, false);
  assert.ok(notPdf.error.includes("PDF"), notPdf.error);
  // 上限は 100MB（サーバの papers.py の UPLOAD_MAX_BYTES と同じ）
  assert.strictEqual(PIM_UPLOAD_MAX, 100 * 1024 * 1024);
}

function testFileSize() {
  assert.strictEqual(pimFileSize(512), "512B");
  assert.strictEqual(pimFileSize(2048), "2.0KB");
  assert.strictEqual(pimFileSize(5 * 1024 * 1024), "5.0MB");
  assert.strictEqual(pimFileSize(0), "");
  assert.strictEqual(pimFileSize(null), "");
  assert.strictEqual(pimFileSize("x"), "");
}

// --- 進み具合の一枚（種類と行き先・U05 §6） ---

function testJobKindLabel() {
  assert.strictEqual(pimJobKindLabel("download"), "Download");
  assert.strictEqual(pimJobKindLabel("manual"), "Manual import");
  assert.strictEqual(pimJobKindLabel("cite-extract"), "Citations");
  assert.strictEqual(pimJobKindLabel("radio"), "Audio");
  assert.strictEqual(pimJobKindLabel("translate"), "Abstract translation");
  // 知らない種類は表示しない
  assert.strictEqual(pimJobKindLabel("unknown"), "");
  assert.strictEqual(pimJobKindLabel(undefined), "");
}

function testProgressViewUnknown() {
  // `percent` が null の仕事は unknown=true（画面は動く帯で出す）
  const radio = {
    key: "radio:9",
    kind: "radio",
    title: "題名",
    status: "processing",
    paper_id: 9,
    steps: [],
    step: 0,
    percent: null,
    messages: [],
    started_at: "2026-09-27T12:00:00+09:00",
  };
  const view = pimProgressView(radio, Date.parse("2026-09-27T12:00:30+09:00"));
  assert.strictEqual(view.unknown, true, view);
  assert.strictEqual(view.stepLabel, "", view);
  assert.strictEqual(view.elapsed, "0:30", view);
  // 普通のダウンロードは今まで通り %
  const dl = {
    ...radio,
    kind: "download",
    steps: ["PDF取得", "図抽出", "翻訳"],
    step: 1,
    percent: 33,
  };
  const dlView = pimProgressView(dl, Date.now());
  assert.strictEqual(dlView.unknown, false, dlView);
  assert.strictEqual(dlView.percent, 33, dlView);
  assert.strictEqual(dlView.stepLabel, "図抽出", dlView);
  // 引用の抜き出しの段（0 = PDF確認 / 1 = 引用抽出）
  const cite = {
    ...radio,
    kind: "cite-extract",
    steps: ["PDF確認", "引用抽出"],
    step: 1,
    percent: 50,
  };
  const citeView = pimProgressView(cite, Date.now());
  assert.strictEqual(citeView.stepLabel, "引用抽出", citeView);
}

function testJobTarget() {
  // 引用の抜き出しは引用一覧へ
  assert.strictEqual(
    pimJobTarget({ kind: "cite-extract", paper_id: 12 }, "test-moon"),
    "#/paper/test-moon/12/cites"
  );
  // それ以外は論文情報
  assert.strictEqual(
    pimJobTarget({ kind: "download", paper_id: 12 }, "test-moon"),
    "#/paper/test-moon/12"
  );
  assert.strictEqual(
    pimJobTarget({ kind: "manual", paper_id: 12 }, "test-moon"),
    "#/paper/test-moon/12"
  );
  // 音声化は root_name が分かるなら再生画面へ（分からないときは論文情報）
  const real = global.paperProjectByName;
  global.paperProjectByName = (name) => ({ name, root_name: "my-project" });
  try {
    assert.strictEqual(
      pimJobTarget({ kind: "radio", paper_id: 9 }, "test-moon"),
      "#/radio/my-project-9"
    );
  } finally {
    global.paperProjectByName = real;
  }
  assert.strictEqual(pimJobTarget({ kind: "radio", paper_id: 9 }, "test-moon"), "#/paper/test-moon/9");
  // 論文の番号やプロジェクトが分からなければリンクは出さない
  assert.strictEqual(pimJobTarget({ kind: "download", paper_id: null }, "x"), "");
  assert.strictEqual(pimJobTarget({ kind: "download", paper_id: 1 }, ""), "");
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
  testProgressView();
  testElapsedLabel();
  testManualQuery();
  testManualKey();
  testTargetFromKey();
  testCheckUpload();
  testFileSize();
  testJobKindLabel();
  testProgressViewUnknown();
  testJobTarget();
  console.log("test_paperimport: すべて通りました");
}

main();
