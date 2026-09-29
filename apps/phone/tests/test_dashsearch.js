"use strict";
// Dashboard の検索（static/dashsearch.js）の DOM を使わない関数の小さなテスト（assert だけ）。
// 実行:
//     node tests/test_dashsearch.js

const assert = require("assert");
const path = require("path");

const ds = require(path.join(__dirname, "..", "src", "sai_phone", "static", "dashsearch.js"));

// 入力欄 → API の検索文字列
function testQuery() {
  assert.strictEqual(ds.dsrQuery({ q: "  " }), null, "検索語が空なら検索しない");
  assert.strictEqual(ds.dsrQuery(null), null);
  const p = new URLSearchParams(ds.dsrQuery({ q: " mlflow run " }));
  assert.strictEqual(p.get("q"), "mlflow run");
  assert.strictEqual(p.get("category"), null, "種類を選んでいない・全部選んでいるときは絞らない");
  assert.strictEqual(p.get("limit"), String(ds.DSR_LIMIT));
  assert.strictEqual(new URLSearchParams(ds.dsrQuery({ q: "a", categories: ["task", "message", "waggle"] })).get("category"), null);

  const full = new URLSearchParams(
    ds.dsrQuery({
      q: "x",
      categories: ["waggle", "bogus", "task"],
      since: "2026-09-01",
      until: "2026-09-30",
      block: " worker ",
      fromId: "10",
      toId: 99,
    })
  );
  assert.strictEqual(full.get("category"), "waggle,task", "知らない種類は捨てる");
  assert.strictEqual(full.get("since"), "2026-09-01");
  assert.strictEqual(full.get("until"), "2026-09-30");
  assert.strictEqual(full.get("block"), "worker");
  assert.strictEqual(full.get("from_id"), "10");
  assert.strictEqual(full.get("to_id"), "99");
  // 空の欄は付けない
  const some = new URLSearchParams(ds.dsrQuery({ q: "x", since: "", block: "", fromId: "" }));
  assert.deepStrictEqual([...some.keys()].sort(), ["limit", "q"]);
}

// 結果を押したときの行き先
function testRowHash() {
  assert.strictEqual(ds.dsrRowHash("sai moon", { run_id: 5, category: "task" }), "#/project/sai%20moon/list/5/task");
  assert.strictEqual(ds.dsrRowHash("p", { run_id: 5, category: "message" }), "#/project/p/list/5/message");
  assert.strictEqual(
    ds.dsrRowHash("p", { run_id: 5, category: "waggle", token: "AXANSMpo", file: "a.md" }),
    "#/project/p/list/5/waggle/AXANSMpo"
  );
  // トークンが無い waggle は task の表示に落とさず、run の task へ（壊れた住所を作らない）
  assert.strictEqual(ds.dsrRowHash("p", { run_id: 5, category: "waggle" }), "#/project/p/list/5/task");
  assert.strictEqual(ds.dsrRowTitle({ run_id: 89, block: "worker" }), "#89 worker");
  assert.strictEqual(ds.dsrRowTitle({ run_id: 89 }), "#89");
}

testQuery();
testRowHash();
console.log("test_dashsearch: すべて通りました");
