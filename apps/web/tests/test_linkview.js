"use strict";
// List 画面（計画 14）の DOM を使わない関数の小さなテスト（assert だけ）・計画 14 §6。
// 確かめること:
// - 住所の読み書き（#/project/<name>/list と …/<run_id>/<表示>）
// - Dashboard | List の選択をプロジェクトごとに覚える
// - 最後に見ていた表示の記憶と todo run の task 固定（session は覚えない）
// - ツリーの開閉（覚えが無い＝全部開く・1 つ閉じたら次も同じ）
// - ツリーの組み立て（字下げ・同じ run が両方にあっても最初の 1 件）
// - Log / Stats / History の表示の組み立て
// - review → done → Workflow の移動ボタンの出し分け（計画書 A の U10）
// - エージェントが起こすカード（executer が空でなく human でない）の判定（U1・U12）
// 実行:
//     node tests/test_linkview.js

const assert = require("assert");
const path = require("path");

// node には localStorage が無いので置き場を用意する（保存の仕様を同じように試す）
const store = new Map();
global.localStorage = {
  getItem: (k) => (store.has(k) ? store.get(k) : null),
  setItem: (k, v) => { store.set(k, String(v)); },
  removeItem: (k) => { store.delete(k); },
};

const lv = require(path.join(
  __dirname,
  "..",
  "src",
  "agent_link_web",
  "static",
  "linkview.js"
));

// ツリーの例（agent-link `phone tree` の形・計画 12 §3.2）
function sampleTree() {
  return [
    {
      type: "folder", id: "todo", label: "todo", depth: 0,
      children: [
        { type: "run", run_id: 14, label: "ID14/sub", lane: "todo", status: "todo" },
      ],
    },
    {
      type: "folder", id: "running", label: "running", depth: 0,
      children: [
        { type: "run", run_id: 25, label: "ID25/main", lane: "running", status: "running" },
      ],
    },
    {
      type: "folder", id: "R2", label: "R2", depth: 0,
      children: [
        {
          type: "folder", id: "R2/r1", label: "r1", depth: 1,
          children: [
            { type: "run", run_id: 11, label: "ID11/sub", lane: "done", status: "completed" },
          ],
        },
      ],
    },
  ];
}

function testHash() {
  // 住所を作る（run と表示は省略できる・計画 14 §6）
  assert.strictEqual(lv.listHash("agent-link-web"), "#/project/agent-link-web/list");
  assert.strictEqual(
    lv.listHash("agent-link-web", 25, "log"),
    "#/project/agent-link-web/list/25/log"
  );
  assert.strictEqual(lv.modeHash("p", "dashboard"), "#/project/p");
  assert.strictEqual(lv.modeHash("p", "list"), "#/project/p/list");
  // 共有契約 C-F7：切り替えの住所（U10 でコマンド・会話検索を足した）
  assert.strictEqual(lv.modeHash("p", "workflow"), "#/project/p/workflow");
  assert.strictEqual(lv.modeHash("p", "defs"), "#/project/p/defs");
  assert.strictEqual(lv.modeHash("p", "commands"), "#/project/p/commands");
  assert.strictEqual(lv.modeHash("p", "sessions"), "#/project/p/sessions");
  // 不明・未指定は Dashboard
  assert.strictEqual(lv.modeHash("p", "oreason"), "#/project/p");
  assert.strictEqual(lv.modeHash("p", null), "#/project/p");
  assert.deepStrictEqual(
    lv.LV_MODES.map((m) => m[0]),
    ["dashboard", "workflow", "list", "defs", "commands", "sessions"]
  );
  assert.deepStrictEqual(
    lv.LV_MODES.map((m) => m[1]),
    ["Dashboard", "Workflow", "List", "Settings", "コマンド", "会話検索"]
  );

  // 読み取る
  assert.deepStrictEqual(lv.parseListHash("#/project/agent-link-web/list"), {
    name: "agent-link-web",
    runId: null,
    view: null,
  });
  assert.deepStrictEqual(
    lv.parseListHash("#/project/agent-link-web/list/25/stats"),
    { name: "agent-link-web", runId: 25, view: "stats" }
  );
  // 名前は URL の形のまま decode する
  assert.deepStrictEqual(lv.parseListHash("#/project/my%20proj/list/3/task"), {
    name: "my proj",
    runId: 3,
    view: "task",
  });
  // List 以外の住所は null（Dashboard などは project.js が受け持つ）
  assert.strictEqual(lv.parseListHash("#/"), null);
  assert.strictEqual(lv.parseListHash("#/projects"), null);
  assert.strictEqual(lv.parseListHash("#/project/agent-link-web"), null);
  // run_id が数字でない住所は「run 未選択」として扱う（Agent 画面に落ちない）
  assert.deepStrictEqual(lv.parseListHash("#/project/p/list/abc/log"), {
    name: "p",
    runId: null,
    view: null,
  });
  // 作った住所は読める（往復）
  const hash = lv.listHash("p", 7, "history");
  assert.deepStrictEqual(lv.parseListHash(hash), {
    name: "p",
    runId: 7,
    view: "history",
  });
}

function testModeMemory() {
  store.clear();
  // 最初は覚えていない
  assert.strictEqual(lv.wantsListView("alpha"), false);
  // プロジェクトごとに覚える（計画 14 §6）
  lv.saveMode("alpha", "list");
  assert.strictEqual(lv.wantsListView("alpha"), true);
  assert.strictEqual(lv.wantsListView("beta"), false);
  lv.saveMode("alpha", "dashboard");
  assert.strictEqual(lv.wantsListView("alpha"), false);
  lv.saveMode("beta", "list");
  assert.strictEqual(lv.wantsListView("beta"), true);
  assert.strictEqual(lv.wantsListView("alpha"), false);
}

function testModeMemoryFour() {
  store.clear();
  // 覚えた画面が 4 つ分ある（C-F7）。覚えていない項目は Dashboard。
  assert.strictEqual(lv.rememberedMode("alpha"), "dashboard");
  for (const mode of ["dashboard", "workflow", "list", "defs"]) {
    lv.saveMode("alpha", mode);
    assert.strictEqual(lv.rememberedMode("alpha"), mode);
    assert.strictEqual(lv.wantsListView("alpha"), mode === "list");
  }
  // プロジェクトごとに覚える
  lv.saveMode("beta", "defs");
  assert.strictEqual(lv.rememberedMode("beta"), "defs");
  // 壊れた値・未知の値は Dashboard として扱う（画面が止まらない）
  assert.strictEqual(lv.rememberedMode("gamma"), "dashboard");
  lv.saveMode("gamma", "oreason");
  assert.strictEqual(lv.rememberedMode("gamma"), "dashboard");
  // 今までの値（dashboard / list）は読み替えなしでそのまま使える
  store.set("agent_link_web_link_mode", JSON.stringify({ delta: "list" }));
  assert.strictEqual(lv.rememberedMode("delta"), "list");
  store.set("agent_link_web_link_mode", "壊れた JSON");
  assert.strictEqual(lv.rememberedMode("delta"), "dashboard");
}

function testViewMemory() {
  store.clear();
  // 最初は Log
  assert.strictEqual(lv.loadViewMemory(), "log");
  lv.saveViewMemory("task");
  assert.strictEqual(lv.loadViewMemory(), "outputs");
  // 覚えるのは Summary か Outputs の 2 つ。古い値（stats）は Summary に直す
  lv.saveViewMemory("stats");
  assert.strictEqual(lv.loadViewMemory(), "log");
  // 知らない表示を覚えさせても Summary に落ちる
  lv.saveViewMemory("bogus");
  assert.strictEqual(lv.loadViewMemory(), "log");
}

function testViewTabs() {
  // 帯は Summary | Outputs の 2 つだけ（計画 14 §1.3 の改定）
  assert.deepStrictEqual(
    lv.viewTabs(false).map((t) => t.key),
    ["log", "outputs"]
  );
  // todo run は Outputs（task の一覧）だけ選べる。Summary は押せない
  const todoTabs = lv.viewTabs(true);
  assert.deepStrictEqual(
    todoTabs.filter((t) => t.ok).map((t) => t.key),
    ["outputs"]
  );

  // 光らせるタブ: Summary（log・stats・history）と Outputs（その他）
  assert.strictEqual(lv.tabOf("log"), "log");
  assert.strictEqual(lv.tabOf("stats"), "log");
  assert.strictEqual(lv.tabOf("history"), "log");
  assert.strictEqual(lv.tabOf("outputs"), "outputs");
  assert.strictEqual(lv.tabOf("task"), "outputs");
  assert.strictEqual(lv.tabOf("waggle"), "outputs");
  assert.strictEqual(lv.tabOf(null), "log");

  // 表示の選び方: stats と history は Summary に直す。その他は住所の値をそのまま使う
  assert.strictEqual(lv.pickView(false, "stats"), "log");
  assert.strictEqual(lv.pickView(false, "history"), "log");
  assert.strictEqual(lv.pickView(false, "outputs"), "outputs");
  assert.strictEqual(lv.pickView(false, "waggle"), "waggle");
  assert.strictEqual(lv.pickView(false, "bogus"), "log");
  // todo run は task に固定（記憶は変えないので，次の run では元の表示のまま）
  assert.strictEqual(lv.pickView(true, "stats"), "task");
  assert.strictEqual(lv.pickView(true, "log"), "task");
  assert.strictEqual(lv.pickView(true, "task"), "task");
  // todo run から session は選べない（session は住所の入口だけ・todo では task）
  assert.strictEqual(lv.pickView(true, "session"), "task");
}

function testSessionHash() {
  // session を選んだときの行き先（Agent タブのメッセージ画面・U11）
  assert.strictEqual(lv.sessionHash("agent-link-web", 58), "#/run/agent-link-web/58");
  assert.strictEqual(lv.sessionHash("my proj", "12"), "#/run/my%20proj/12");
}

function testTodoRun() {
  assert.strictEqual(lv.isTodoRun({ lane: "todo", status: "todo" }), true);
  assert.strictEqual(lv.isTodoRun({ lane: "running", status: "todo" }), true);
  assert.strictEqual(lv.isTodoRun({ lane: "review", status: "completed" }), false);
  assert.strictEqual(lv.isTodoRun(null), false);
}

/** エージェントが起こすカード（executer が空でなく human でない）は人が動かさない・U1/U12。 */
function testAgentRun() {
  assert.strictEqual(lv.isAgentRun({ executer: "agent" }), true, "agent");
  // 古い書き出しに残った professor も同じ（人が起こさない）
  assert.strictEqual(lv.isAgentRun({ executer: "professor" }), true, "旧 professor");
  // 未記載・human・空・空白だけは人が起こす
  assert.strictEqual(lv.isAgentRun({ executer: "human" }), false, "human");
  assert.strictEqual(lv.isAgentRun({}), false, "未記載");
  assert.strictEqual(lv.isAgentRun({ executer: "" }), false, "空");
  assert.strictEqual(lv.isAgentRun({ executer: "  " }), false, "空白だけ");
  assert.strictEqual(lv.isAgentRun(null), false, "null");
  // ツリーの行に executer が届く（agent-link `phone tree` の項目）
  const tree = [
    {
      type: "folder", id: "todo", label: "todo", depth: 0,
      children: [
        { type: "run", run_id: 14, label: "ID14/sub", lane: "todo", status: "todo", executer: "agent" },
        { type: "run", run_id: 15, label: "ID15/sub", lane: "todo", status: "todo", executer: "professor" },
        { type: "run", run_id: 16, label: "ID16/sub", lane: "todo", status: "todo", executer: "human" },
        { type: "run", run_id: 17, label: "ID17/sub", lane: "todo", status: "todo" },
      ],
    },
  ];
  const rows = lv.flattenTree(tree, null);
  const byId = (id) => rows.find((r) => r.runId === id);
  assert.strictEqual(lv.isAgentRun(byId(14)), true, "agent の行");
  assert.strictEqual(lv.isAgentRun(byId(15)), true, "旧 professor の行");
  assert.strictEqual(lv.isAgentRun(byId(16)), false, "人が起こす行");
  assert.strictEqual(lv.isAgentRun(byId(17)), false, "未記載の行");
}

function testFolds() {
  store.clear();
  // 覚えが無ければ null（＝全部開く・計画 14 §1.2）
  assert.strictEqual(lv.lvLoadFolds("p"), null);
  // 1 つ閉じたら，開いている id の一覧として覚える
  let open = lv.toggleFold(lv.allFolderIds(sampleTree()), "running");
  assert.deepStrictEqual(open.sort(), ["R2", "R2/r1", "todo"]);
  lv.lvSaveFolds("p", open);
  assert.deepStrictEqual(lv.lvLoadFolds("p").sort(), ["R2", "R2/r1", "todo"]);
  // 開いているものが無い状態も覚える（全部閉じたまま）
  lv.lvSaveFolds("p", []);
  assert.deepStrictEqual(lv.lvLoadFolds("p"), []);
  // プロジェクトごとに別々（覚えが無ければ全部開く）
  assert.strictEqual(lv.lvLoadFolds("q"), null);
  // toggle: 押すごとに開閉が反転する
  assert.deepStrictEqual(lv.toggleFold([], "a"), ["a"]);
  assert.deepStrictEqual(lv.toggleFold(["a"], "a"), []);
  assert.deepStrictEqual(lv.toggleFold(["a"], "b").sort(), ["a", "b"]);
}

function testFlatten() {
  const items = sampleTree();
  // 覚えが無い＝全部開く（字下げも nesting の深さどおり）
  const all = lv.flattenTree(items, null);
  assert.deepStrictEqual(
    all.map((r) => [r.type, r.label, r.depth]),
    [
      ["folder", "todo", 0],
      ["run", "ID14/sub", 1],
      ["folder", "running", 0],
      ["run", "ID25/main", 1],
      ["folder", "R2", 0],
      ["folder", "r1", 1],
      ["run", "ID11/sub", 2],
    ]
  );
  // 1 つも開いていなければフォルダだけ
  const closed = lv.flattenTree(items, []);
  assert.deepStrictEqual(closed.map((r) => r.type), ["folder", "folder", "folder"]);
  assert.strictEqual(closed.every((r) => r.open === false), true);
  // R2 だけ開く → r1 は見えて，中身（run）は見えない
  const some = lv.flattenTree(items, ["R2"]);
  assert.deepStrictEqual(
    some.map((r) => r.label),
    ["todo", "running", "R2", "r1"]
  );
  // run の行には lane / status が付く（todo 判定と色分けに使う）
  const run = lv.flattenTree(items, null).find((r) => r.runId === 14);
  assert.strictEqual(run.lane, "todo");
  assert.strictEqual(run.status, "todo");
}

function testFindRun() {
  const items = sampleTree();
  // 同じ run が両方に出ても最初の 1 件を返す
  assert.strictEqual(lv.findRun(items, 25).runId, 25);
  assert.strictEqual(lv.findRun(items, 11).lane, "done");
  assert.strictEqual(lv.findRun(items, 999), null);
  assert.strictEqual(lv.findRun(items, "abc"), null);
  assert.deepStrictEqual(lv.allFolderIds(items), ["todo", "running", "R2", "R2/r1"]);
}

function testAdvanceAction() {
  // 状態フォルダ（review / done）と R/r 側の両方から lane を見る（計画書 A の U10）
  const items = [
    {
      type: "folder", id: "review", label: "review", depth: 0,
      children: [
        { type: "run", run_id: 41, label: "ID41/main", lane: "review", status: "completed" },
      ],
    },
    {
      type: "folder", id: "done", label: "done", depth: 0,
      children: [
        { type: "run", run_id: 40, label: "ID40/main", lane: "done", status: "completed" },
      ],
    },
    {
      type: "folder", id: "R2", label: "R2", depth: 0,
      children: [
        { type: "run", run_id: 40, label: "ID40/main", lane: "done", status: "completed" },
        { type: "run", run_id: 41, label: "ID41/main", lane: "review", status: "completed" },
        { type: "run", run_id: 39, label: "ID39/main", lane: "R2", status: "completed" },
      ],
    },
  ];
  // review の run は → done へ、done の run は → Workflow へ
  assert.deepStrictEqual(lv.advanceAction(items, 41), {
    from: "review", to: "done", label: "→ done",
  });
  assert.deepStrictEqual(lv.advanceAction(items, 40), {
    from: "done", to: "workflow", label: "→ Workflow",
  });
  // 既に Workflow（R<gr>）や todo / running には出さない
  assert.strictEqual(lv.advanceAction(items, 39), null);
  assert.strictEqual(lv.advanceAction(sampleTree(), 25), null); // running
  assert.strictEqual(lv.advanceAction(sampleTree(), 14), null); // todo
  assert.strictEqual(lv.advanceAction(items, 999), null);
  assert.strictEqual(lv.advanceAction(null, 41), null);

  // lane は重複を消して全部取る（最初の 1 件だけで決めない）
  assert.deepStrictEqual(Array.from(lv.runLanes(items, 41)), ["review"]);
  assert.deepStrictEqual(Array.from(lv.runLanes(items, 40)), ["done"]);
  assert.deepStrictEqual(Array.from(lv.runLanes(items, 39)), ["R2"]);
  assert.deepStrictEqual(Array.from(lv.runLanes(items, 999)), []);
}

function testLogSections() {
  // 実際の `phone run --view log` の形を小さくした例（計画 12 §3.2）
  const data = {
    input_fields: [
      { label: "task_text", value: "計画書を作成して" },
      { label: "plan_doc", value: "waggle:abc" },
    ],
    round: 1,
    global_round: 5,
    tag: null,
    model: "opus",
    thinking: "high",
    started_at: "2026-09-23 13:38:37",
    finished_at: "2026-09-23 14:00:00",
    git_commit_before: "aaaaaaa",
    git_commit_after: "bbbbbbb",
    session_stats: {
      turn_count: 1,
      tool_call_count: 78,
      input_tokens: 248034,
      output_tokens: 1068,
      thinking_tokens: 190864,
      total_tokens: 439966,
    },
    connections: [
      { direction: "from", kind: "task_text", run_id: 29, global_round: 5, round: 1, block_name: "plan", waggle_token: "abcd1234", file_path: ".agent-link/outputs/R5/29/plan.md" },
      { direction: "to", kind: "task_create", run_id: 36, global_round: 5, round: 1, block_name: "worker", description: "実装する" },
      { direction: "from", kind: "task_output", run_id: 40, global_round: null, round: 2, block_name: "sub" },
    ],
    review_from: [],
    review_to: [],
    output_waggle_tokens: ["H9sPKWNJ"],
    session_dir: "/パスは出さない",
  };
  const sections = lv.logSections(data);
  assert.deepStrictEqual(
    sections.map((s) => s.title),
    ["Inputs", "Settings", "Connection", "Outputs"]
  );
  // 入力は label / value の対で。task_text は task の表示で読めるので出さない
  assert.deepStrictEqual(sections[0].rows, [["plan_doc", "waggle:abc"]]);
  // task_text しか無ければ Inputs 自体を出さない
  assert.deepStrictEqual(
    lv.logSections({ input_fields: [{ label: "task_text", value: "x" }] }).map((s) => s.title),
    []
  );
  const conf = Object.fromEntries(sections[1].rows);
  assert.strictEqual(conf["Round"], "r1");
  assert.strictEqual(conf["Global round"], "R5");
  assert.strictEqual(conf["Model"], "opus");
  assert.strictEqual(conf["thinking"], "high");
  assert.strictEqual(conf["commit"], "aaaaaaa → bbbbbbb");
  // Connection は作られた順のまま、head と sub と run_id を持つ（統計は Stats の表示へ移した）
  assert.deepStrictEqual(sections[2].items, [
    { head: "from [task_text] R5.1/ID29/plan", sub: "waggle:abcd1234 file:.agent-link/outputs/R5/29/plan.md", runId: 29 },
    { head: "to [task_create] R5.1/ID36/worker", sub: "実装する", runId: 36 },
    { head: "from [task_output] R?.2/ID40/sub", sub: "", runId: 40 },
  ]);
  assert.deepStrictEqual(sections[3].rows, [["waggle", "H9sPKWNJ"]]);
  assert.strictEqual(JSON.stringify(sections).includes("turn_count"), false);
  // session の数字は Stats の表示に出す
  const sess = Object.fromEntries(lv.sessionStatRows(data.session_stats));
  assert.strictEqual(sess["Turns"], "1");
  assert.strictEqual(sess["Tool calls"], "78");
  assert.strictEqual(sess["Total tokens"], "439966");
  assert.deepStrictEqual(lv.sessionStatRows(null), []);
  // session_dir（パス）は出さない（iPhone に場所を送らない・計画 12 §1.4 と同じ考え）
  assert.strictEqual(
    JSON.stringify(sections).includes("session_dir"),
    false
  );
  // 中身が無ければ空配列（画面は「項目がありません」を出す）
  assert.deepStrictEqual(lv.logSections({}), []);
  assert.deepStrictEqual(lv.logSections(null), []);
}

function testStatsCells() {
  const cells = lv.statsCells({
    input: 100,
    output: 50,
    cache_read: 300,
    cache_write: 10,
    cost: { total: 0.25 },
  });
  assert.deepStrictEqual(
    cells.map((c) => c.label),
    ["Input", "Output", "Cache read", "Cache write", "Cost", "Hit rate"]
  );
  assert.strictEqual(cells[0].value, 100);
  assert.strictEqual(cells[0].kind, "tokens");
  assert.strictEqual(cells[4].value, 0.25);
  assert.strictEqual(cells[4].kind, "money");
  // ヒット率 = 読 / (読 + 入力)（TUI のメッセージ行と同じ算法）
  assert.strictEqual(Math.round(cells[5].value), 75);
  assert.strictEqual(cells[5].kind, "percent");

  // 何も無ければ「—」が出る形（null）
  const empty = lv.statsCells(null);
  assert.deepStrictEqual(
    empty.map((c) => c.value),
    [0, 0, 0, 0, null, null]
  );
}

function testStatsSeries() {
  const data = {
    messages: [
      { sequence: 1, cache_hit_percent: 66.7 },
      { sequence: 2, cache_hit_percent: 80 },
    ],
    cumulative: {
      cum_hit_percent: [50, 66.7],
      cum_input: [10, 30],
      cum_cache_read: [20, 60],
      cum_cache_write: [0, 5],
    },
  };
  // 1: ターンごと（%）
  const turn = lv.statsSeries(data, "turn");
  assert.strictEqual(turn.length, 1);
  assert.strictEqual(turn[0].pct, true);
  assert.deepStrictEqual(turn[0].values, [66.7, 80]);
  // 2: 累積（%）
  const pct = lv.statsSeries(data, "cumpct");
  assert.strictEqual(pct[0].pct, true);
  assert.deepStrictEqual(pct[0].values, [50, 66.7]);
  // 3: 累積量（入力 / 読 / 書の 3 本）
  const cum = lv.statsSeries(data, "cum");
  assert.strictEqual(cum.length, 3);
  assert.deepStrictEqual(cum.map((s) => s.name), ["Input", "Cache read", "Cache write"]);
  assert.deepStrictEqual(cum[1].values, [20, 60]);
  // データが無いときも壊れない
  assert.deepStrictEqual(lv.statsSeries(null, "turn")[0].values, []);
}

function testTypeSlices() {
  const slices = lv.typeSlices({
    user: 10,
    agent: 30,
    toolCall: 0,
    toolResult: 20,
    thinking: 40,
  });
  assert.deepStrictEqual(
    slices.map((s) => s.label),
    ["user", "agent", "toolResult", "thinking"]
  );
  const total = slices.reduce((a, s) => s.ratio + a, 0);
  assert.ok(Math.abs(total - 1) < 1e-9, `合計が 1 でない: ${total}`);
  // 0 の種類は出さない。全部 0 なら空（「データがありません」）
  assert.deepStrictEqual(lv.typeSlices({ user: 0, agent: 0 }), []);
  assert.deepStrictEqual(lv.typeSlices(null), []);
}

function testCanEdit() {
  // 編集できるのは未実行の todo の task だけ。message と report（finalize-report.md）は不可
  const todo = { lane: "todo", status: "todo" };
  const done = { lane: "done", status: "completed" };
  assert.strictEqual(lv.lvCanEdit("task", todo), true);
  assert.strictEqual(lv.lvCanEdit("task", done), false);
  assert.strictEqual(lv.lvCanEdit("message", done), false);
  assert.strictEqual(lv.lvCanEdit("report", done), false);
  assert.strictEqual(lv.lvCanEdit("report", todo), false);
  // 固定部分（system prompt）は起動時に書いたものなので書き換えない（U6）
  assert.strictEqual(lv.lvCanEdit("system", done), false);
  assert.strictEqual(lv.lvCanEdit("system", todo), false);
}

function testTaskDescription() {
  // Outputs の task の行の説明は front matter の description（本文の同じ語は見ない）
  assert.strictEqual(
    lv.lvTaskDescription("---\ntitle: x\ndescription: 作業報告書 U01: DONE\n---\n本文 description: no\n"),
    "作業報告書 U01: DONE"
  );
  assert.strictEqual(lv.lvTaskDescription("---\nscheduled_at: 2026-10-09\n---\n本文\n"), "");
  assert.strictEqual(lv.lvTaskDescription("本文だけ description: no"), "");
  assert.strictEqual(lv.lvTaskDescription(null), "");
}

function testHistoryRows() {
  // 行の名前: 命令があれば命令名、無ければ道具の名前
  assert.strictEqual(lv.lvCallName({ name: "Bash", command: "agent-link doc read" }), "agent-link doc read");
  assert.strictEqual(lv.lvCallName({ name: "Read" }), "Read");
  assert.strictEqual(lv.lvCallName(null), "");
  // 失敗の印は is_error が true のときだけ
  assert.strictEqual(lv.lvEventRow({ is_error: true }).failed, true);
  assert.strictEqual(lv.lvEventRow({ is_error: false }).failed, false);
  assert.strictEqual(lv.lvEventRow({}).failed, false);
  // 番号のチップは 5 個まで・残りは rest（+n）
  const many = lv.lvEventRow({ ids: ["#1", "#2", "#3", "#4", "#5", "#6", "#7"] }).ids;
  assert.deepStrictEqual(many.shown, ["#1", "#2", "#3", "#4", "#5"]);
  assert.strictEqual(many.rest, 2);
  assert.deepStrictEqual(lv.lvIdChips(["#1"]), { shown: ["#1"], rest: 0 });
  assert.deepStrictEqual(lv.lvIdChips(undefined), { shown: [], rest: 0 });
  // 全文を開けるのは call_id が住所の形のときだけ（`|` や 81 字は開かない）
  assert.strictEqual(lv.lvEventRow({ call_id: "toolu_01-a" }).canOpen, true);
  assert.strictEqual(lv.lvEventRow({ call_id: "a|b" }).canOpen, false);
  assert.strictEqual(lv.lvEventRow({ call_id: "a".repeat(81) }).canOpen, false);
  assert.strictEqual(lv.lvEventRow({}).canOpen, false);
  // 結果の先頭は preview のまま
  assert.strictEqual(lv.lvEventRow({ preview: "結果の先頭" }).preview, "結果の先頭");
}

function testHistoryCli() {
  // cli が無い（古い CLI）ときは null（カードを出さない）
  assert.strictEqual(lv.lvCliSummary(undefined), null);
  assert.strictEqual(lv.lvCliSummary(null), null);
  const s = lv.lvCliSummary({
    calls: [{ command: "agent-link doc read", count: 3, errors: 1 }],
    failures: [{ class: "not_found", count: 1 }, { class: "input", count: 2 }],
    ids: { seen: 40, used: 6, unused: 34 },
  });
  assert.deepStrictEqual(s.calls, [{ command: "agent-link doc read", count: 3, errors: 1 }]);
  assert.deepStrictEqual(s.failures.map((f) => f.label), ["見つからない（3）", "入力の誤り（2）"]);
  assert.deepStrictEqual(s.failures.map((f) => f.count), [1, 2]);
  assert.deepStrictEqual(s.ids, { seen: 40, used: 6, unused: 34 });
  // 中身が空でも壊れない（失敗は無い・命令は無い）
  const empty = lv.lvCliSummary({});
  assert.deepStrictEqual(empty.calls, []);
  assert.deepStrictEqual(empty.failures, []);
  assert.deepStrictEqual(empty.ids, { seen: 0, used: 0, unused: 0 });
  assert.strictEqual(lv.lvFailureLabel("input"), "入力の誤り（2）");
  assert.strictEqual(lv.lvFailureLabel("new_class"), "new_class");
}

function testCallPrettyInput() {
  // 全文の入力は JSON なら整えて出し、読めなければそのまま
  assert.strictEqual(lv.lvPrettyInput('{"a":1}'), '{\n  "a": 1\n}');
  assert.strictEqual(lv.lvPrettyInput("not json"), "not json");
  assert.strictEqual(lv.lvPrettyInput(null), "");
}

function main() {
  testHash();
  testModeMemory();
  testModeMemoryFour();
  testViewMemory();
  testViewTabs();
  testSessionHash();
  testTodoRun();
  testAgentRun();
  testFolds();
  testFlatten();
  testFindRun();
  testAdvanceAction();
  testLogSections();
  testStatsCells();
  testStatsSeries();
  testTypeSlices();
  testTaskDescription();
  testCanEdit();
  testHistoryRows();
  testHistoryCli();
  testCallPrettyInput();
  console.log("test_linkview: すべて通りました");
}

main();
