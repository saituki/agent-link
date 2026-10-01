"use strict";
// リモートブラウザ（rbrowser.js）の DOM を使わない部分のテスト（assert だけ）。
// 実行:
//     node tests/test_rbrowser.js

const assert = require("assert");
const path = require("path");
const rb = require(path.join(__dirname, "..", "src", "sai_phone", "static", "rbrowser.js"));

// 住所の往復（対象 4 通り・DOI の / は符号化される）
const doi = "10.1038/s41586-020-2649-2";
for (const target of [
  { log_id: 3, position: 2 },
  { paper_id: 12 },
  { import_id: "abc-1" },
  { cite_paper_id: 12, ref_no: 3 },
]) {
  const h = rb.rbBrowserHash("my proj", doi, target);
  assert.ok(!h.includes("10.1038/"), h);
  assert.deepStrictEqual(rb.rbParseHash(h), { name: "my proj", doi, target });
}
assert.strictEqual(rb.rbParseHash("#/papers/import"), null);
assert.strictEqual(rb.rbParseHash("#/papers/browser?name=a&doi=x"), null); // 対象 0 個
assert.strictEqual(rb.rbParseHash("#/papers/browser?name=a&doi=x&paper_id=1&import_id=z"), null); // 2 個
assert.strictEqual(rb.rbParseHash("#/papers/browser?name=a&doi=x&log_id=3"), null); // 片方だけ

// タップ位置（範囲外は丸める）
const rect = { left: 10, top: 20, width: 200, height: 400 };
assert.deepStrictEqual(rb.rbTapBody(rect, 110, 220), { type: "tap", x: 0.5, y: 0.5 });
assert.deepStrictEqual(rb.rbTapBody(rect, 0, 1000), { type: "tap", x: 0, y: 1 });

// サイズ表示
assert.strictEqual(rb.rbSizeLabel(1.2 * 1024 * 1024), "1.2 MB");
assert.strictEqual(rb.rbSizeLabel(500), "500 B");

// PDF だけ・done の物だけ Import 可
const list = rb.rbPdfDownloads({ downloads: [
  { name: "a.PDF", size: 1, done: true },
  { name: "b.pdf", size: 1, done: false },
  { name: "c.html", size: 1, done: true },
] });
assert.deepStrictEqual(list.map((d) => [d.name, d.importable]), [["a.PDF", true], ["b.pdf", false]]);
assert.deepStrictEqual(rb.rbPdfDownloads(null), []);

// 呼び出しが契約どおり（fetch を記録用に差し替える）
(async () => {
  const calls = [];
  const fake = (status, json) => async (url, opt) => {
    calls.push({ url, method: opt.method, body: JSON.parse(opt.body) });
    return { ok: status < 400, status, json: async () => json };
  };
  const base = "/api/projects/p/papers";
  let r = await rb.rbCallOpen(fake(200, { open: true }), base, doi);
  assert.ok(r.ok);
  assert.deepStrictEqual(calls[0], { url: `${base}/browser/open`, method: "POST", body: { doi } });
  r = await rb.rbCallOpen(fake(503, { detail: "Chrome が無い" }), base, doi);
  assert.deepStrictEqual(r, { ok: false, detail: "Chrome が無い" });

  await rb.rbCallInput(fake(200, { ok: true }), base, rb.rbTapBody(rect, 110, 220));
  assert.deepStrictEqual(calls[2], { url: `${base}/browser/input`, method: "POST", body: { type: "tap", x: 0.5, y: 0.5 } });
  r = await rb.rbCallInput(fake(409, { detail: "閉じています" }), base, { type: "back" });
  assert.strictEqual(r.closed, true);

  r = await rb.rbCallImport(fake(202, { key: "manual:paper:12" }), base, { paper_id: 12 }, "x.pdf");
  assert.deepStrictEqual(r, { ok: true, key: "manual:paper:12" });
  assert.deepStrictEqual(calls[4], { url: `${base}/browser/import`, method: "POST", body: { paper_id: 12, file_name: "x.pdf" } });
  r = await rb.rbCallImport(fake(409, { detail: "取り込み中" }), base, { paper_id: 12 }, "x.pdf");
  assert.deepStrictEqual(r, { ok: false, detail: "取り込み中" });
  console.log("test_rbrowser: すべて通りました");
})();
