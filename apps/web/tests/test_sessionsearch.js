"use strict";
// 会話の検索の画面（U10）の DOM を使わない関数の小さなテスト（assert だけ）。
// 確かめること:
// - 住所の読み取り（#/project/<name>/sessions）
// - 条件 → クエリ文字列（空は入れない・状態と種類は許可した物だけ・条件なしは空）
// - 何を探すか（会話・文書の印と検索語）と、文書の検索の条件（run の番号・日付・種類）
// - 結果のまとめ（block_id ごと・会話と文書を同じ run に並べる）・文書の行き先
// 実行:
//     node apps/web/tests/test_sessionsearch.js

const assert = require("assert");
const path = require("path");

const sess = require(path.join(
  __dirname,
  "..",
  "src",
  "agent_link_web",
  "static",
  "sessionsearch.js"
));

function testParseHash() {
  assert.deepStrictEqual(sess.sessParseHash("#/project/sai-moon/sessions"), { name: "sai-moon" });
  assert.deepStrictEqual(sess.sessParseHash("#/project/a%20b/sessions/"), { name: "a b" });
  assert.strictEqual(sess.sessParseHash("#/project/sai-moon/commands"), null);
}

function testBuildQuery() {
  // 条件が 1 つも無ければ空
  assert.strictEqual(sess.sessBuildQuery({}), "");
  assert.strictEqual(sess.sessBuildQuery({ word: "  " }), "");

  const q = sess.sessBuildQuery({
    word: " a,b ",
    global_round: "2-4", // 廃止した項目はクエリに入れない
    round: "1",
    id: "10-20",
    block: "worker",
    date: "2026/09/01-2026/09/30",
    status: ["running", "todo"],
    type: ["agent", "user"],
  });
  const params = new URLSearchParams(q);
  assert.strictEqual(params.get("word"), "a,b");
  assert.strictEqual(params.get("global_round"), null);
  assert.strictEqual(params.get("round"), null);
  assert.strictEqual(params.get("id"), "10-20");
  assert.strictEqual(params.get("block"), "worker");
  assert.strictEqual(params.get("date"), "2026/09/01-2026/09/30");
  assert.strictEqual(params.get("status"), "running,todo");
  assert.strictEqual(params.get("type"), "agent,user");

  // 知らない状態・種類は入れない
  const bad = new URLSearchParams(
    sess.sessBuildQuery({ status: ["bogus", "running"], type: ["nope"] })
  );
  assert.strictEqual(bad.get("status"), "running");
  assert.strictEqual(bad.get("type"), null);
}

function testGroup() {
  const groups = sess.sessGroup([
    { block_id: 5, type: "user" },
    { block_id: 5, type: "agent" },
    { block_id: 6, type: "tool" },
  ]);
  assert.deepStrictEqual(groups.map((g) => g.blockId), ["5", "6"]);
  assert.strictEqual(groups[0].items.length, 2);
  assert.deepStrictEqual(sess.sessGroup([]), []);
  assert.deepStrictEqual(sess.sessGroup(null), []);
}

function testPlan() {
  // 印が無ければ、会話も文書も探す（検索語が無いと文書は探さない）
  assert.deepStrictEqual(sess.sessPlan({ word: "a" }), { sessions: true, docs: ["task", "message", "waggle"] });
  assert.deepStrictEqual(sess.sessPlan({ word: "" }), { sessions: true, docs: [] });
  // 会話の種類だけ・文書の種類だけ
  assert.deepStrictEqual(sess.sessPlan({ word: "a", type: ["agent"] }), { sessions: true, docs: [] });
  assert.deepStrictEqual(sess.sessPlan({ word: "a", doc: ["waggle"] }), { sessions: false, docs: ["waggle"] });
  // 両方に印があれば、両方
  assert.deepStrictEqual(sess.sessPlan({ word: "a", type: ["user"], doc: ["task"] }), { sessions: true, docs: ["task"] });
  // 文書だけ印があり、検索語が無ければ何も探さない
  assert.deepStrictEqual(sess.sessPlan({ word: " ", doc: ["task"] }), { sessions: false, docs: [] });
  // 知らない種類は印が無いのと同じ（両方を探す）
  assert.deepStrictEqual(sess.sessPlan({ word: "a", doc: ["bogus"] }), { sessions: true, docs: ["task", "message", "waggle"] });
  assert.deepStrictEqual(sess.sessPlan(null), { sessions: true, docs: [] });
}

function testDocQuery() {
  // 検索語が無ければ null（API が 400 を返すため）
  assert.strictEqual(sess.sessDocQuery({ word: " , " }, ["task"]), null);
  // カンマは空白に替える・3 つ全部なら種類で絞らない・limit は 50
  const all = new URLSearchParams(sess.sessDocQuery({ word: "a, b c" }, ["task", "message", "waggle"]));
  assert.strictEqual(all.get("q"), "a b c");
  assert.strictEqual(all.get("category"), null);
  assert.strictEqual(all.get("limit"), String(sess.SESS_DOC_LIMIT));
  const some = new URLSearchParams(sess.sessDocQuery({ word: "a" }, ["waggle", "task"]));
  assert.strictEqual(some.get("category"), "waggle,task");

  // run の番号: N・N-M・N-・-M
  const one = new URLSearchParams(sess.sessDocQuery({ word: "a", id: "7" }, ["task"]));
  assert.strictEqual(one.get("from_id"), "7");
  assert.strictEqual(one.get("to_id"), "7");
  const span = new URLSearchParams(sess.sessDocQuery({ word: "a", id: "10-20" }, ["task"]));
  assert.strictEqual(span.get("from_id"), "10");
  assert.strictEqual(span.get("to_id"), "20");
  const open = new URLSearchParams(sess.sessDocQuery({ word: "a", id: "10-" }, ["task"]));
  assert.strictEqual(open.get("from_id"), "10");
  assert.strictEqual(open.get("to_id"), null);
  const upto = new URLSearchParams(sess.sessDocQuery({ word: "a", id: "-20" }, ["task"]));
  assert.strictEqual(upto.get("from_id"), null);
  assert.strictEqual(upto.get("to_id"), "20");
  // run の番号の誤り
  assert.strictEqual(sess.sessDocQuery({ word: "a", id: "x1" }, ["task"]), null);
  assert.strictEqual(sess.sessDocQuery({ word: "a", id: "-" }, ["task"]), null);

  // 日付: 1 日・範囲・逆順は入れ替える・誤り
  const day = new URLSearchParams(sess.sessDocQuery({ word: "a", date: "2026/9/1" }, ["task"]));
  assert.strictEqual(day.get("since"), "2026-09-01");
  assert.strictEqual(day.get("until"), "2026-09-01");
  const range = new URLSearchParams(sess.sessDocQuery({ word: "a", date: "2026/09/01-2026/09/30" }, ["task"]));
  assert.strictEqual(range.get("since"), "2026-09-01");
  assert.strictEqual(range.get("until"), "2026-09-30");
  const rev = new URLSearchParams(sess.sessDocQuery({ word: "a", date: "2026/09/30-2026/09/01" }, ["task"]));
  assert.strictEqual(rev.get("since"), "2026-09-01");
  assert.strictEqual(rev.get("until"), "2026-09-30");
  assert.strictEqual(sess.sessDocQuery({ word: "a", date: "2026-09-01" }, ["task"]), null);
  assert.strictEqual(sess.sessDocQuery({ word: "a", date: "2026/13/1" }, ["task"]), null);
  assert.strictEqual(sess.sessDocQuery({ word: "a", date: "2026/09/01-" }, ["task"]), null);

  // block はそのまま（完全一致は API が見る）
  assert.strictEqual(new URLSearchParams(sess.sessDocQuery({ word: "a", block: " worker " }, ["task"])).get("block"), "worker");
  // 状態は文書に入れない（global_round・round も無い）
  const ignored = new URLSearchParams(sess.sessDocQuery({ word: "a", global_round: "2", round: "1", status: ["running"] }, ["task"]));
  assert.strictEqual(ignored.get("round"), null);
  assert.strictEqual(ignored.get("global_round"), null);
  assert.strictEqual(ignored.get("status"), null);
}

function testMerge() {
  const merged = sess.sessMerge(
    [
      { block_id: 5, type: "user" },
      { block_id: 6, type: "agent" },
    ],
    [
      { run_id: 5, category: "task" },
      { run_id: 9, category: "message" },
      { run_id: 6, category: "waggle" },
    ]
  );
  // 会話の run の順が先・文書だけの run は後ろ・同じ run は 1 つにまとめる
  assert.deepStrictEqual(merged.map((g) => g.blockId), ["5", "6", "9"]);
  assert.strictEqual(merged[0].items.length, 1);
  assert.strictEqual(merged[0].docs.length, 1);
  assert.strictEqual(merged[2].items.length, 0);
  assert.strictEqual(merged[2].docs[0].category, "message");
  assert.deepStrictEqual(sess.sessMerge([], []), []);
  assert.deepStrictEqual(sess.sessMerge(null, null), []);
}

function testDocHash() {
  assert.strictEqual(sess.sessDocHash("sai moon", { run_id: 5, category: "task" }), "#/project/sai%20moon/list/5/task");
  assert.strictEqual(sess.sessDocHash("p", { run_id: 5, category: "message" }), "#/project/p/list/5/message");
  assert.strictEqual(
    sess.sessDocHash("p", { run_id: 5, category: "waggle", token: "AXANSMpo" }),
    "#/project/p/list/5/waggle/AXANSMpo"
  );
  // token が無い waggle は List の task へ（旧 Dashboard の検索と同じ規則）
  assert.strictEqual(sess.sessDocHash("p", { run_id: 5, category: "waggle" }), "#/project/p/list/5/task");
}

function main() {
  testParseHash();
  testBuildQuery();
  testGroup();
  testPlan();
  testDocQuery();
  testMerge();
  testDocHash();
  console.log("test_sessionsearch: すべて通りました");
}

main();
