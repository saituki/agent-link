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
      todo_runs: [
        { run_id: 56, label: "ID56/kantoku" },
        { run_id: 57, label: "ID57/👷 worker", scheduled_at: "2026-10-02 09:00" },
      ],
    },
    { name: "sai-old", daemon_alive: false, running: 0, review: [], todo: 3, todo_runs: [{ run_id: 60, label: "ID60", scheduled_at: "2026-10-01 09:00" }] },
  ],
  papers: [
    { name: "sai-paper", attention: 5, items: [{ key: "question:41:x", kind: "question", title: "この論文は?" }] },
  ],
  errors: ["sai-old: sai-link が失敗しました"],
};

// GET /api/home/overview の返答そのままの形
const OVERVIEW = {
  period: "week",
  since: "2026-09-21",
  until: "2026-09-27",
  projects: 2,
  summary: {
    executions: 12,
    completed: 10,
    failed: 1,
    running: 1,
    success_rate: 0.83,
    duration_ms: 3600000,
    tokens_total: 1234567,
    cost_usd: 3.21,
  },
  daily: [
    { date: "2026-09-21", executions: 3, tokens: 120000 },
    { date: "2026-09-22", executions: 0, tokens: 0 },
    { date: "2026-09-23", executions: 9, tokens: 1114567 },
  ],
  recent: [
    {
      project: "sai-moon",
      run_id: 42,
      block: "worker",
      status: "completed",
      started_at: "2026-09-27 10:28:21",
      description: "U02 ホーム",
      linkable: true,
    },
    {
      project: "sai-old",
      run_id: 41,
      block: "plan",
      status: "failed",
      started_at: "2026-09-26 09:00:00",
      description: "",
      linkable: false,
    },
  ],
  errors: [],
};

const keys = (data) => H.hmSections(data).map((s) => s.key);

// 5 種類のカードが C-A3 の並びで出ること
function testSectionOrder() {
  assert.deepStrictEqual(keys(SAMPLE), ["wait", "running", "daemon", "papers", "errors"]);
  const byKey = {};
  for (const s of H.hmSections(SAMPLE)) byKey[s.key] = s;
  assert.strictEqual(byKey.wait.title, "質問がある会話");
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

// 「確認を待っています」（review）はホームに出さない（review が 1 件あっても sections に入らない）
function testNoReviewSection() {
  assert.ok(SAMPLE.projects[0].review.length > 0, "サンプルに review がある");
  assert.ok(!H.hmSections(SAMPLE).some((s) => s.key === "review"));
}

// 予約した todo（scheduled_at がある行だけ・時刻の早い順）
function testScheduledItems() {
  const items = H.hmScheduledItems(SAMPLE);
  assert.deepStrictEqual(items.map((i) => [i.title, i.scheduledAt, i.where]), [
    ["ID60", "2026-10-01 09:00", "sai-old"],
    ["ID57/👷 worker", "2026-10-02 09:00", "sai-phone"],
  ]);
  assert.strictEqual(items[0].hash, "#/project/sai-old/list/60/task");
  // 予約の無い todo は todo の行に scheduledAt を付けない
  assert.strictEqual(H.hmTodoItems(SAMPLE)[0].scheduledAt, undefined);
  assert.deepStrictEqual(H.hmScheduledItems({ projects: [{ name: "p", todo_runs: [{ run_id: 1 }] }] }), []);
  assert.deepStrictEqual(H.hmScheduledItems(null), []);
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

// --- 合計のダッシュボード（U02 §3） ---

// 大きな数字 4 つ（実行数・成功率・トークン・料金）
function testOverviewCards() {
  const cards = H.hmOverviewCards(OVERVIEW);
  assert.deepStrictEqual(
    cards.map((c) => c.label),
    ["実行", "成功率", "トークン", "料金"]
  );
  assert.strictEqual(cards[0].value, "12 回");
  assert.strictEqual(cards[1].value, "83%");
  assert.strictEqual(cards[2].value, "1.2M");
  assert.strictEqual(cards[3].value, "$3.21");
  // 合計が読めてないとき（0 件でもカードを出すので）
  const zero = H.hmOverviewCards(null);
  assert.deepStrictEqual(zero.map((c) => c.value), ["0 回", "0%", "0", "$0.00"]);
  // 補足（プロジェクト数・期間・読めなかった件数）
  assert.strictEqual(H.hmOverviewNote(OVERVIEW), "2 プロジェクトの合計・2026-09-21〜2026-09-27");
  assert.strictEqual(
    H.hmOverviewNote({ projects: 0, since: null, until: null, errors: ["a: 失敗"] }),
    "0 プロジェクトの合計・一部のプロジェクトを読めませんでした（1）"
  );
}

// 期間の切り替え（week / month / all。覚えていなければ week）
function testPeriod() {
  const store = {};
  global.localStorage = {
    getItem: (k) => (k in store ? store[k] : null),
    setItem: (k, v) => { store[k] = String(v); },
  };
  assert.strictEqual(H.HM_PERIOD_KEY, "sai_phone_home_period");
  assert.deepStrictEqual(
    H.HM_PERIODS,
    [["week", "今週"], ["month", "今月"], ["all", "全期間"]]
  );
  assert.strictEqual(H.hmPeriodValue(), "week", "覚えていなければ week");
  H.hmSavePeriod("month");
  assert.strictEqual(H.hmPeriodValue(), "month");
  H.hmSavePeriod("all");
  assert.strictEqual(H.hmPeriodValue(), "all");
  H.hmSavePeriod("bad"); // 知らない値は覚えない
  assert.strictEqual(H.hmPeriodValue(), "all");
  store[H.HM_PERIOD_KEY] = "broken";
  assert.strictEqual(H.hmPeriodValue(), "week", "壊れた値は week に戻す");
  delete global.localStorage;
  assert.strictEqual(H.hmPeriodValue(), "week", "localStorage が無くても week");
}

// todo の行（label とプロジェクト名・List の task への行き先）
function testTodoItems() {
  const items = H.hmTodoItems(SAMPLE);
  assert.strictEqual(items.length, 3, items);
  assert.deepStrictEqual(items[0], {
    title: "ID56/kantoku",
    where: "sai-phone",
    hash: "#/project/sai-phone/list/56/task",
  });
  assert.strictEqual(items[2].hash, "#/project/sai-old/list/60/task");
  // label が無い行は ID<run_id> を出す
  const noLabel = H.hmTodoItems({ projects: [{ name: "p", todo_runs: [{ run_id: 7 }] }] });
  assert.strictEqual(noLabel[0].title, "ID7");
  // todo_runs が無い（古い /api/home でも）ときは 0 件
  assert.deepStrictEqual(H.hmTodoItems({ projects: [{ name: "p", todo: 3 }] }), []);
  assert.deepStrictEqual(H.hmTodoItems(null), []);
  assert.strictEqual(H.HM_TODO_MAX, 10, "はじめは 10 件");
}

// 最近のアクティビティの行（状態の印・時刻・書き出しの有無で行き先が変わる）
function testRecentItems() {
  const items = H.hmRecentItems(OVERVIEW);
  assert.strictEqual(items.length, 2, items);
  assert.strictEqual(items[0].title, "#42 worker ✓");
  assert.strictEqual(items[0].sub, "U02 ホーム");
  assert.strictEqual(items[0].where, "sai-moon");
  assert.ok(items[0].ago, "時刻が出る");
  assert.strictEqual(items[0].hash, "#/run/sai-moon/42", "書き出しがあれば会話画面");
  // 書き出しが無い実行は List へ
  assert.strictEqual(items[1].title, "#41 plan ✗");
  assert.strictEqual(items[1].hash, "#/project/sai-old/list/41");
  // 状態の印（project.js の recentRow と同じ）
  const marks = [
    ["completed", "✓"],
    ["running", "●"],
    ["failed", "✗"],
    ["cancelled", "✗"],
  ];
  for (const [status, mark] of marks) {
    const one = H.hmRecentItems({ recent: [{ project: "p", run_id: 1, status }] });
    assert.strictEqual(one[0].title, `#1 ${mark}`, status);
  }
  // 0 件でも 0 行になる（カードはまだ出す）
  assert.deepStrictEqual(H.hmRecentItems({ recent: [] }), []);
  assert.deepStrictEqual(H.hmRecentItems(null), []);
  assert.strictEqual(H.HM_RECENT_MAX, 10, "はじめは 10 件");
  assert.strictEqual(H.HM_RECENT_ALL, 20, "すべて表示しても 20 件まで");
}

// 質問・承認の行（待ちのカードで 0 件でも出すので、行だけ外に出す）
function testWaitItems() {
  const items = H.hmWaitItems(SAMPLE);
  assert.strictEqual(items.length, 2, items);
  assert.strictEqual(items[0].kind, "question");
  assert.strictEqual(items[1].kind, "approval");
  assert.deepStrictEqual(H.hmWaitItems({ questions: [], approvals: [] }), []);
  assert.deepStrictEqual(H.hmWaitItems(null), []);
}

// 日ごとのグラフの横軸（project.js の axisLabels と同じ規則）
function testAxisLabels() {
  const week = [
    { date: "2026-09-21" },
    { date: "2026-09-22" },
    { date: "2026-09-23" },
  ];
  assert.deepStrictEqual(H.hmAxisLabels("week", week), ["月", "火", "水"]);
  assert.deepStrictEqual(H.hmAxisLabels("month", week), ["", "", ""], "1・5・10…の日はだけ出す");
  assert.deepStrictEqual(
    H.hmAxisLabels("month", [{ date: "2026-09-01" }, { date: "2026-09-05" }, { date: "2026-09-10" }]),
    ["1", "5", "10"]
  );
  const many = Array.from({ length: 30 }, (_, i) => ({ date: `2026-08-${String(i + 1).padStart(2, "0")}` }));
  const all = H.hmAxisLabels("all", many);
  assert.ok(all.filter(Boolean).length <= 8, "棒が多すぎるときは間能干して出す");
  assert.deepStrictEqual(H.hmAxisLabels("week", []), []);
}

function main() {
  testSectionOrder();
  testEmptySectionsOmitted();
  testWaitRows();
  testNoReviewSection();
  testScheduledItems();
  testRunning();
  testDaemon();
  testPapers();
  testAgo();
  testEmptyKind();
  testInterval();
  testOverviewCards();
  testPeriod();
  testTodoItems();
  testRecentItems();
  testWaitItems();
  testAxisLabels();
  console.log("test_home: すべて通りました");
}

main();
