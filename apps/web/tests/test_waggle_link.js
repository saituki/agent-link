"use strict";
// md の中の `waggle:<トークン>` を押せるリンクにする（markdown.js）と、
// その行き先の住所（linkview.js の listHash / parseListHash）の小さなテスト。
// 実行:
//     node tests/test_waggle_link.js

const assert = require("assert");
const path = require("path");

const store = new Map();
global.localStorage = {
  getItem: (k) => (store.has(k) ? store.get(k) : null),
  setItem: (k, v) => { store.set(k, String(v)); },
  removeItem: (k) => { store.delete(k); },
};
const dir = path.join(__dirname, "..", "src", "agent_link_web", "static");
const { renderMarkdown } = require(path.join(dir, "markdown.js"));
const lv = require(path.join(dir, "linkview.js"));

function links(html) {
  return [...html.matchAll(/<a href="#" class="md-waggle" data-waggle-token="([^"]+)"(?: data-waggle-run="(\d+)")?>([^<]*)<\/a>/g)].map(
    (m) => ({ token: m[1], run: m[2] || null, text: m[3] })
  );
}

function testLinks() {
  // トークンだけ
  assert.deepStrictEqual(links(renderMarkdown("入力は waggle:AXANSMpo を読む")), [
    { token: "AXANSMpo", run: null, text: "waggle:AXANSMpo" },
  ]);
  // パスつき: run_id を取り、`*` が斜体にならず、文末の点は含めない
  const ref = "waggle:AXANSMpo:path:.agent-link/outputs/*/70/report-U01.md";
  const html = renderMarkdown(`- ${ref}。\n- 文末の点: ${ref}.`);
  assert.deepStrictEqual(links(html), [
    { token: "AXANSMpo", run: "70", text: ref },
    { token: "AXANSMpo", run: "70", text: ref },
  ]);
  assert.ok(!html.includes("<em>"), "パスの * が斜体にならない");
  // 1 行に 2 つあっても * で斜体にならない
  const two = renderMarkdown(`${ref} と waggle:BBBBBBBB:path:.agent-link/outputs/*/71/x.md`);
  assert.strictEqual(links(two).length, 2);
  assert.ok(!two.includes("<em>"));
  // コードの中・トークンの長さが違うもの・前が英数字のものはリンクにしない
  assert.strictEqual(links(renderMarkdown("`waggle:AXANSMpo`")).length, 0);
  assert.strictEqual(links(renderMarkdown("waggle:short")).length, 0);
  assert.strictEqual(links(renderMarkdown("xwaggle:AXANSMpo")).length, 0);
  assert.strictEqual(links(renderMarkdown("waggle:AXANSMpo9")).length, 0, "9 文字目も英数字ならトークンではない");
  // 属性に入るのは英数字だけ（HTML の注入にならない）
  const evil = renderMarkdown('waggle:AXANSMpo:path:"><img src=x onerror=1>');
  assert.ok(!evil.includes("<img"), evil);
}

function testHash() {
  assert.strictEqual(lv.listHash("p", 70, "waggle", "AXANSMpo"), "#/project/p/list/70/waggle/AXANSMpo");
  assert.strictEqual(lv.listHash("p", 70, "waggle"), "#/project/p/list/70/waggle");
  assert.strictEqual(lv.listHash("p", 70, "task", "AXANSMpo"), "#/project/p/list/70/task", "waggle 以外はトークンを付けない");
  assert.deepStrictEqual(lv.parseListHash("#/project/p/list/70/waggle/AXANSMpo"), {
    name: "p",
    runId: 70,
    view: "waggle",
    waggleToken: "AXANSMpo",
  });
  assert.deepStrictEqual(lv.parseListHash("#/project/p/list/70/waggle"), { name: "p", runId: 70, view: "waggle" });
  // 形の合わないトークン・waggle 以外の表示ではトークンを無いことにする
  assert.deepStrictEqual(lv.parseListHash("#/project/p/list/70/waggle/bad"), { name: "p", runId: 70, view: "waggle" });
  assert.deepStrictEqual(lv.parseListHash("#/project/p/list/70/task/AXANSMpo"), { name: "p", runId: 70, view: "task" });
  // 今までの住所はそのまま読める
  assert.deepStrictEqual(lv.parseListHash("#/project/p/list"), { name: "p", runId: null, view: null });
  assert.deepStrictEqual(lv.parseListHash("#/project/p/list/12/log"), { name: "p", runId: 12, view: "log" });
}

testLinks();
testHash();
console.log("test_waggle_link: すべて通りました");
