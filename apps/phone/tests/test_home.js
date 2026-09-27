"use strict";
// ホーム画面（static/home.js・U4）の DOM を使わない部分のテスト（assert だけ）。
// 実行:
//     node tests/test_home.js

const assert = require("assert");
const path = require("path");

const H = require(path.join(__dirname, "..", "src", "sai_phone", "static", "home.js"));

// C-A3 の形そのままの偽データ
const SAMPLE = {
  questions: [
    { id: "q-1", agent: "cc", title: "どちらにしますか？", created_at: "2026-09-26 10:00:00", project: "sai-phone", run_id: 55 },
  ],
  approvals: [
    { id: "a-1", tool_name: "Bash", summary: "git push origin main", created_at: "2026-09-26 09:00:00", project: "sai-phone", run_id: 55 },
  ],
  projects: [
    {
      name: "sai-phone",
      daemon_alive: true,
      running: 2,
      last_executed_at: "2026-09-26 10:12:03",
      review: [{ run_id: 54, label: "ID54/👷 worker" }],
      todo: 2,
    },
    { name: "sai-old", daemon_alive: false, running: 0, review: [], todo: 3 },
  ],
  papers: [
    { name: "sai-paper", attention: 5, items: [{ key: "question:41:x", kind: "question", title: "この論文は?" }] },
  ],
  errors: ["sai-old: sai-link が失敗しました"],
};

const keys = (data) => H.hmSections(data).map((s) => s.key);

// 5 種類のカードが C-A3 の並びで出ること
function testSectionOrder() {
  assert.deepStrictEqual(keys(SAMPLE), ["wait", "review", "running", "daemon", "papers", "errors"]);
  const byKey = {};
  for (const s of H.hmSections(SAMPLE)) byKey[s.key] = s;
  assert.strictEqual(byKey.wait.title, "答えを待っています");
  assert.strictEqual(byKey.review.title, "確認を待っています");
  assert.strictEqual(byKey.running.title, "動いています");
  assert.strictEqual(byKey.daemon.title, "止まっている常駐");
  assert.strictEqual(byKey.papers.title, "論文の要対応");
  assert.strictEqual(byKey.errors.title, "読めなかったプロジェクト");
}

// 0 件のカードは出さない
function testEmptySectionsOmitted() {
  const empty = { questions: [], approvals: [], projects: [], papers: [], errors: [] };
  assert.deepStrictEqual(H.hmSections(empty), []);
  // 質問だけなら「答えを待っています」だけ
  const q = { questions: SAMPLE.questions, approvals: [], projects: [], papers: [], errors: [] };
  assert.deepStrictEqual(keys(q), ["wait"]);
  // 実行中が 0 のプロジェクトは「動いています」に出さない
  const p = { questions: [], approvals: [], projects: [{ name: "a", running: 0, review: [] }], papers: [], errors: [] };
  assert.deepStrictEqual(keys(p), []);
  // 常駐が生きているプロジェクトは「止まっている常駐」に出さない
  const alive = { projects: [{ name: "a", daemon_alive: true, todo: 5, running: 0, review: [] }] };
  assert.deepStrictEqual(keys(alive), []);
  // todo が 0 なら常駐のカードを出さない
  const noTodo = { projects: [{ name: "a", daemon_alive: false, todo: 0, running: 0, review: [] }] };
  assert.deepStrictEqual(keys(noTodo), []);
}

// 質問・承認の 1 行（プロジェクト名 #run_id・題・何分前）と行き先
function testWaitRows() {
  const sec = H.hmSections(SAMPLE).find((s) => s.key === "wait");
  assert.strictEqual(sec.count, 2);
  const [q, a] = sec.items;
  assert.strictEqual(q.where, "sai-phone #55");
  assert.strictEqual(q.title, "どちらにしますか？");
  assert.ok(q.ago, "何分前が付く");
  assert.strictEqual(q.hash, "#/run/sai-phone/55");
  assert.strictEqual(a.title, "Bash: git push origin main", "承認は道具名と中身");
  assert.strictEqual(a.hash, "#/run/sai-phone/55");

  // project / run_id が分からないときは会話の一覧（#/）へ
  assert.strictEqual(H.hmWaitHash({ project: null, run_id: 1 }), "#/");
  assert.strictEqual(H.hmWaitHash({ project: "p", run_id: null }), "#/");
  assert.strictEqual(H.hmWaitWhere({ project: "p", run_id: null }), "p");
  assert.strictEqual(H.hmWaitWhere({ project: null }), "");
  // 名の無い質問にも何か文言を出す
  assert.strictEqual(H.hmWaitTitle({}, "question"), "（題のない質問）");
  assert.strictEqual(H.hmWaitTitle({ tool_name: "Bash" }, "approval"), "Bash");
  // プロジェクト名に読点や斜線があっても正しく URL になる
  assert.strictEqual(H.hmWaitHash({ project: "a/b, c", run_id: 1 }), `#/run/${encodeURIComponent("a/b, c")}/1`);
}

// review の行き先（#\/project\/…\/list\/…\/review）
function testReviewHash() {
  const sec = H.hmSections(SAMPLE).find((s) => s.key === "review");
  assert.strictEqual(sec.count, 1);
  assert.strictEqual(sec.items[0].where, "sai-phone");
  assert.strictEqual(sec.items[0].title, "ID54/👷 worker");
  assert.strictEqual(sec.items[0].hash, "#/project/sai-phone/list/54/review");
  assert.strictEqual(sec.term, "review", "「確認待ち」の札が付く");
}

// 動いているプロジェクト（件数と行き先）
function testRunning() {
  const sec = H.hmSections(SAMPLE).find((s) => s.key === "running");
  assert.strictEqual(sec.items[0].title, "sai-phone");
  assert.strictEqual(sec.items[0].sub, "2 件 実行中");
  assert.strictEqual(sec.items[0].hash, "#/project/sai-phone/workflow");
}

// 止まっている常駐（案内と Dashboard への行き先）
function testDaemon() {
  const sec = H.hmSections(SAMPLE).find((s) => s.key === "daemon");
  assert.strictEqual(sec.items.length, 1);
  assert.strictEqual(sec.items[0].title, "sai-old");
  assert.ok(sec.items[0].sub.includes("常駐が止まっています"));
  assert.ok(sec.items[0].sub.includes("起動してください"));
  assert.strictEqual(sec.items[0].hash, "#/project/sai-old", "Dashboard（常駐の起動ボタンがある）");
  assert.strictEqual(sec.term, "daemon");
}

// 論文の要対応（先頭 3 件と dashboard への行き先）
function testPapers() {
  const many = {
    papers: [
      {
        name: "p",
        attention: 9,
        items: Array.from({ length: 6 }, (_, i) => ({ key: `k${i}`, kind: "question", title: `T${i}` })),
      },
    ],
  };
  const sec = H.hmSections(many).find((s) => s.key === "papers");
  assert.strictEqual(sec.items[0].count, 9);
  assert.strictEqual(sec.items[0].sub.length, H.HM_PAPERS_MAX, "先頭 3 件だけ");
  assert.deepStrictEqual(sec.items[0].sub, ["T0", "T1", "T2"]);
  assert.strictEqual(sec.items[0].hash, "#/papers/dashboard");
  assert.strictEqual(sec.items[0].project, "p", "プロジェクト名を覚えておく");
  assert.strictEqual(sec.term, "attention");
  // papers.js と同じ場所（共有契約 C-F1）
  assert.strictEqual(H.HM_PAPERS_KEY, "sai_phone_papers_project");
}

// 「何分前」
function testAgo() {
  const now = Date.parse("2026-09-26T12:00:00Z");
  assert.strictEqual(H.hmAgo("2026-09-26 11:59:30", now), "たった今");
  assert.strictEqual(H.hmAgo("2026-09-26 11:58:00", now), "2分前");
  assert.strictEqual(H.hmAgo("2026-09-26 09:00:00", now), "3時間前");
  assert.strictEqual(H.hmAgo("2026-09-23 12:00:00", now), "3日前");
  assert.strictEqual(H.hmAgo("2026-09-01 12:00:00", now), "9/1");
  assert.strictEqual(H.hmAgo("", now), "");
  assert.strictEqual(H.hmAgo(null, now), "");
  assert.strictEqual(H.hmAgo("これは日時ではない", now), "");
  // タイムゾーン無しの DB の日時は UTC とみなす（project.js の parseStamp と同じ読み方）
  assert.strictEqual(H.hmStamp("2026-09-26 10:00:00"), Date.parse("2026-09-26T10:00:00Z"));
  assert.strictEqual(H.hmStamp("2026-09-26T10:00:00Z"), Date.parse("2026-09-26T10:00:00Z"));
  assert.strictEqual(H.hmStamp(""), null);
}

// 空のときの案内（2 種類）
function testEmptyKind() {
  assert.strictEqual(H.hmEmptyKind({ projects: [], papers: [] }), "no-projects");
  assert.strictEqual(H.hmEmptyKind({}), "no-projects");
  assert.strictEqual(H.hmEmptyKind({ projects: [{ name: "a" }] }), "none");
  assert.strictEqual(H.hmEmptyKind({ papers: [{ name: "p" }] }), "none");
  assert.strictEqual(H.hmEmptyKind(null), "no-projects");
}

// 15 秒ごとに読み直す（サーバー側で 10 秒覚えるので、それより短くしない）
function testInterval() {
  assert.strictEqual(H.HM_INTERVAL_MS, 15000);
}

function main() {
  testSectionOrder();
  testEmptySectionsOmitted();
  testWaitRows();
  testReviewHash();
  testRunning();
  testDaemon();
  testPapers();
  testAgo();
  testEmptyKind();
  testInterval();
  console.log("test_home: すべて通りました");
}

main();
