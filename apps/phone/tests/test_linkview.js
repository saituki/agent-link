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
  "sai_phone",
  "static",
  "linkview.js"
));

// ツリーの例（sai-link `phone tree` の形・計画 12 §3.2）
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
  assert.strictEqual(lv.listHash("sai-phone"), "#/project/sai-phone/list");
  assert.strictEqual(
    lv.listHash("sai-phone", 25, "log"),
    "#/project/sai-phone/list/25/log"
  );
  assert.strictEqual(lv.modeHash("p", "dashboard"), "#/project/p");
  assert.strictEqual(lv.modeHash("p", "list"), "#/project/p/list");
  // 共有契約 C-F7：4 つの切り替えの住所
  assert.strictEqual(lv.modeHash("p", "workflow"), "#/project/p/workflow");
  assert.strictEqual(lv.modeHash("p", "defs"), "#/project/p/defs");
  // 不明・未指定は Dashboard
  assert.strictEqual(lv.modeHash("p", "oreason"), "#/project/p");
  assert.strictEqual(lv.modeHash("p", null), "#/project/p");
  assert.deepStrictEqual(
    lv.LV_MODES.map((m) => m[0]),
    ["dashboard", "workflow", "list", "defs"]
  );
  assert.deepStrictEqual(
    lv.LV_MODES.map((m) => m[1]),
    ["Dashboard", "Workflow", "List", "設定"]
  );

  // 読み取る
  assert.deepStrictEqual(lv.parseListHash("#/project/sai-phone/list"), {
    name: "sai-phone",
    runId: null,
    view: null,
  });
  assert.deepStrictEqual(
    lv.parseListHash("#/project/sai-phone/list/25/stats"),
    { name: "sai-phone", runId: 25, view: "stats" }
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
  assert.strictEqual(lv.parseListHash("#/project/sai-phone"), null);
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
  store.set("sai_phone_link_mode", JSON.stringify({ delta: "list" }));
  assert.strictEqual(lv.rememberedMode("delta"), "list");
  store.set("sai_phone_link_mode", "壊れた JSON");
  assert.strictEqual(lv.rememberedMode("delta"), "dashboard");
}

function testViewMemory() {
  store.clear();
  // 最初は Log
  assert.strictEqual(lv.loadViewMemory(), "log");
  lv.saveViewMemory("stats");
  assert.strictEqual(lv.loadViewMemory(), "stats");
  // 知らない表示を覚えさせても Log に落ちる
  lv.saveViewMemory("bogus");
  assert.strictEqual(lv.loadViewMemory(), "log");
}

function testViewTabs() {
  // 8 つの表示（帯の並び・計画 14 §1.3。session は Agent 画面への入口・U11）
  assert.deepStrictEqual(
    lv.viewTabs(false).map((t) => t.key),
    ["log", "stats", "history", "task", "review", "message", "waggle", "session"]
  );
  // todo run は task だけ選べる（session も出さない）
  const todoTabs = lv.viewTabs(true);
  assert.deepStrictEqual(
    todoTabs.filter((t) => t.ok).map((t) => t.key),
    ["task"]
  );

  // 表示の選び方: 最後に見た表示をそのまま使う
  assert.strictEqual(lv.pickView(false, "stats"), "stats");
  assert.strictEqual(lv.pickView(false, "waggle"), "waggle");
  assert.strictEqual(lv.pickView(false, "session"), "session");
  assert.strictEqual(lv.pickView(false, "bogus"), "log");
  // todo run は task に固定（記憶は変えないので，次の run では元の表示のまま）
  assert.strictEqual(lv.pickView(true, "stats"), "task");
  assert.strictEqual(lv.pickView(true, "log"), "task");
  assert.strictEqual(lv.pickView(true, "task"), "task");
  // todo run から session は選べない
  assert.strictEqual(lv.pickView(true, "session"), "task");
}

function testSessionHash() {
  // session を選んだときの行き先（Agent タブのメッセージ画面・U11）
  assert.strictEqual(lv.sessionHash("sai-phone", 58), "#/run/sai-phone/58");
  assert.strictEqual(lv.sessionHash("my proj", "12"), "#/run/my%20proj/12");
}

function testTodoRun() {
  assert.strictEqual(lv.isTodoRun({ lane: "todo", status: "todo" }), true);
  assert.strictEqual(lv.isTodoRun({ lane: "running", status: "todo" }), true);
  assert.strictEqual(lv.isTodoRun({ lane: "review", status: "completed" }), false);
  assert.strictEqual(lv.isTodoRun(null), false);
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
    connections_from: ["[out] R5.1/ID29"],
    connections_to: ["[task] R5.1/ID36"],
    review_from: [],
    review_to: [],
    output_waggle_tokens: ["H9sPKWNJ"],
    session_dir: "/パスは出さない",
  };
  const sections = lv.logSections(data);
  assert.deepStrictEqual(
    sections.map((s) => s.title),
    ["Inputs", "設定", "統計", "つながり", "出力トークン"]
  );
  // 入力は label / value の対で。task_text は task の表示で読めるので出さない
  assert.deepStrictEqual(sections[0].rows, [["plan_doc", "waggle:abc"]]);
  // task_text しか無ければ Inputs 自体を出さない
  assert.deepStrictEqual(
    lv.logSections({ input_fields: [{ label: "task_text", value: "x" }] }).map((s) => s.title),
    []
  );
  const conf = Object.fromEntries(sections[1].rows);
  assert.strictEqual(conf["ラウンド"], "r1");
  assert.strictEqual(conf["グローバル"], "R5");
  assert.strictEqual(conf["モデル"], "opus");
  assert.strictEqual(conf["thinking"], "high");
  assert.strictEqual(conf["commit"], "aaaaaaa → bbbbbbb");
  // 統計・つながり・出力
  const stats = Object.fromEntries(sections[2].rows);
  assert.strictEqual(stats["ターン数"], "1");
  assert.strictEqual(stats["ツール"], "78 回");
  assert.strictEqual(stats["合計トークン"], "439966");
  assert.deepStrictEqual(sections[3].rows, [
    ["から", "[out] R5.1/ID29"],
    ["へ", "[task] R5.1/ID36"],
  ]);
  assert.deepStrictEqual(sections[4].rows, [["waggle", "H9sPKWNJ"]]);
  // session_dir（パス）は出さない（iPhone に場所を送らない・計画 12 §1.4 と同じ考え）
  assert.strictEqual(
    JSON.stringify(sections).includes("session_dir"),
    false
  );
  // 中身が無ければ空配列（画面は「項目がありません」を出す）
  assert.deepStrictEqual(lv.logSections({}), []);
  assert.deepStrictEqual(lv.logSections(null), []);
}

function testRunIdOfRef() {
  // つながりの行から run_id を取り出す（sai-link の `R9.10/ID56` 形式・U01）
  assert.strictEqual(lv.lvRunIdOfRef("FNPsuvtk R9.10/ID56"), 56);
  assert.strictEqual(lv.lvRunIdOfRef("[task] R3.1/ID12"), 12);
  assert.strictEqual(lv.lvRunIdOfRef("R?.?/ID56/id:7"), 56);
  assert.strictEqual(lv.lvRunIdOfRef("ID56"), null);
  assert.strictEqual(lv.lvRunIdOfRef("R1.2/ID5x"), 5);
  assert.strictEqual(lv.lvRunIdOfRef(""), null);
  assert.strictEqual(lv.lvRunIdOfRef(null), null);
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
    ["入力", "出力", "キャッシュ読み", "キャッシュ書き", "費用", "ヒット率"]
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
  assert.deepStrictEqual(cum.map((s) => s.name), ["入力", "キャッシュ読み", "キャッシュ書き"]);
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
  // task は未実行の todo だけ，review は実行済みの run だけ書き換えられる。message は不可
  const todo = { lane: "todo", status: "todo" };
  const done = { lane: "done", status: "completed" };
  assert.strictEqual(lv.lvCanEdit("task", todo), true);
  assert.strictEqual(lv.lvCanEdit("task", done), false);
  assert.strictEqual(lv.lvCanEdit("review", done), true);
  assert.strictEqual(lv.lvCanEdit("review", todo), false);
  assert.strictEqual(lv.lvCanEdit("review", null), false);
  assert.strictEqual(lv.lvCanEdit("message", done), false);
}

function testWaggleDirect() {
  // 1 件だけなら一覧を通さず直接開く（計画 14 §1.3）
  assert.strictEqual(lv.waggleDirect([{ token: "a" }]), true);
  assert.strictEqual(lv.waggleDirect([]), false);
  assert.strictEqual(lv.waggleDirect([{ token: "a" }, { token: "b" }]), false);
  assert.strictEqual(lv.waggleDirect(null), false);
}

function main() {
  testHash();
  testModeMemory();
  testModeMemoryFour();
  testViewMemory();
  testViewTabs();
  testSessionHash();
  testTodoRun();
  testFolds();
  testFlatten();
  testFindRun();
  testAdvanceAction();
  testLogSections();
  testRunIdOfRef();
  testStatsCells();
  testStatsSeries();
  testTypeSlices();
  testWaggleDirect();
  testCanEdit();
  console.log("test_linkview: すべて通りました");
}

main();
