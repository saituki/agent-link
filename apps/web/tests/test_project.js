"use strict";
// Project タブの DOM を使わない関数の小さなテスト（assert だけ）・06 §5。
// 実行:
//     node tests/test_project.js

const assert = require("assert");
const path = require("path");

const { relativeTime, donutSlices, todoFolderRuns, projectRowFacts, prjValidShortName, prjShowsTs } = require(path.join(
  __dirname,
  "..",
  "src",
  "sai_web",
  "static",
  "project.js"
));

function testRelativeTime() {
  // 06 §4.1 の表示例（たった今 / 5分前 / 3時間前 / 2日前 / 9/12）
  const now = Date.parse("2026-09-23T10:00:00Z");
  assert.strictEqual(relativeTime("2026-09-23 10:00:00", now), "たった今");
  assert.strictEqual(relativeTime("2026-09-23 09:55:00", now), "5分前");
  assert.strictEqual(relativeTime("2026-09-23 07:00:00", now), "3時間前");
  assert.strictEqual(relativeTime("2026-09-21 10:00:00", now), "2日前");
  // 7 日を超えたら日付に戻る（DB の日時は UTC として読む）
  assert.strictEqual(
    relativeTime("2025-09-12 00:00:00", Date.parse("2025-10-12T00:00:00Z")),
    "9/12"
  );
  // 読めない値は空（壊れた日付で画面が崩れない）
  assert.strictEqual(relativeTime("", now), "");
  assert.strictEqual(relativeTime("不明", now), "");
}

function testDonutSlices() {
  // 上位 4 件＋その他（06 §4.2-5 / §4.4）
  const rows = [
    { label: "opus", value: 50 },
    { label: "sonnet", value: 30 },
    { label: "gpt", value: 10 },
    { label: "gemini", value: 5 },
    { label: "mistral", value: 5 },
  ];
  const slices = donutSlices(rows, 4);
  assert.strictEqual(slices.length, 5); // 上位4 + その他
  assert.strictEqual(slices[0].label, "opus");
  assert.strictEqual(slices[3].label, "gemini");
  assert.strictEqual(slices[4].label, "その他");
  assert.strictEqual(slices[4].value, 5); // 5 + 5 のうち残り1件
  assert.ok(Math.abs(slices[0].ratio - 0.5) < 1e-9); // 50 / 100
  assert.ok(Math.abs(slices.reduce((a, s) => a + s.ratio, 0) - 1) < 1e-9);

  // 入れ混んでも多い順に並び，上位だけが選ばれる
  const shuffled = donutSlices([rows[4], rows[0], rows[2], rows[1], rows[3]], 4);
  assert.strictEqual(shuffled[0].label, "opus");
  assert.strictEqual(shuffled[4].label, "その他");

  // 合計 0 は空配列（「データがありません」表示）
  assert.deepStrictEqual(donutSlices([], 4), []);
  assert.deepStrictEqual(donutSlices([{ label: "x", value: 0 }], 4), []);
}

function testTodoFolderRuns() {
  // 新形式 todo の一覧は `GET …/tree` の todo フォルダの中身（計画 15）
  const items = [
    {
      type: "folder", id: "todo", label: "todo", depth: 0,
      children: [
        { type: "run", run_id: 14, label: "ID14/sub", lane: "todo", status: "todo" },
        { type: "run", run_id: 15, label: "ID15/sub", lane: "todo", status: "todo" },
      ],
    },
    {
      type: "folder", id: "running", label: "running", depth: 0,
      children: [
        { type: "run", run_id: 25, label: "ID25/main", lane: "running", status: "running" },
      ],
    },
  ];
  assert.deepStrictEqual(
    todoFolderRuns(items).map((r) => r.run_id),
    [14, 15]
  );
  // todo が動いたあと（中身が空）は空配列
  const moved = items.map((f) =>
    f.id === "todo" ? { ...f, children: [] } : f
  );
  assert.deepStrictEqual(todoFolderRuns(moved), []);
  // 形が違うもの全部に耐える
  assert.deepStrictEqual(todoFolderRuns([]), []);
  assert.deepStrictEqual(todoFolderRuns(null), []);
  assert.deepStrictEqual(todoFolderRuns([{ type: "run", run_id: 3 }]), []);
}

function testProjectRowFacts() {
  // 一覧の 1 枚が表すもの（U5 §3-1）
  const alive = projectRowFacts({
    name: "sai-web", root_exists: true, db_exists: true,
    daemon_alive: true, running: 2, last_executed_at: "2026-09-26 10:00:00",
  });
  assert.strictEqual(alive.name, "sai-web");
  assert.strictEqual(alive.alive, true);
  assert.strictEqual(alive.running, 2);
  assert.strictEqual(alive.usable, true);
  assert.ok(alive.line2.includes("動いています"));
  assert.ok(alive.line2.includes("実行中 2 件"));
  assert.ok(alive.line2.includes("最後に動いたのは"));
  // 止まっているとき・0 件のときは「実行中」を出さない
  const stopped = projectRowFacts({ name: "x", root_exists: true, db_exists: true, daemon_alive: false, running: 0, last_executed_at: null });
  assert.strictEqual(stopped.alive, false);
  assert.ok(stopped.line2.includes("止まっています"));
  assert.ok(!stopped.line2.includes("実行中"));
  assert.strictEqual(stopped.ago, "");
  // root か db が無い行は薄くして押せない
  assert.strictEqual(projectRowFacts({ root_exists: false, db_exists: true }).usable, false);
  assert.strictEqual(projectRowFacts({ root_exists: true, db_exists: false }).usable, false);
  // 壊れた応答でも止めない
  assert.strictEqual(projectRowFacts(null).line2, "常駐 止まっています");
}

function testValidShortName() {
  // 共有契約 C4: ^[a-z0-9][a-z0-9-]{0,29}$（U11）
  assert.strictEqual(prjValidShortName("x"), true, "1 文字はよい");
  assert.strictEqual(prjValidShortName("a-b"), true, "途中のハイフンはよい");
  assert.strictEqual(prjValidShortName("a" + "1".repeat(29)), true, "30 文字はよい");
  assert.strictEqual(prjValidShortName("a" + "1".repeat(30)), false, "31 文字は長い");
  assert.strictEqual(prjValidShortName("-abc"), false, "先頭のハイフンはだめ");
  assert.strictEqual(prjValidShortName("Abc"), false, "大文字はだめ");
  assert.strictEqual(prjValidShortName("aBc"), false, "大文字を含まない");
  assert.strictEqual(prjValidShortName("a_b"), false, "アンダースコアはだめ");
  assert.strictEqual(prjValidShortName("a b"), false, "空白はだめ");
  assert.strictEqual(prjValidShortName(""), false, "空はだめ");
  assert.strictEqual(prjValidShortName("1a"), true, "数字で始まってもよい");
  assert.strictEqual(prjValidShortName(null), false);
  assert.strictEqual(prjValidShortName(undefined), false);
  assert.strictEqual(prjValidShortName(12), false);
}

function testPrjShowsTs() {
  // 時系列（sai-ts）のボタンは動いているコンテナの行だけ（U05 §6）
  assert.strictEqual(prjShowsTs({ container_state: "running" }), true, "動いていれば出す");
  assert.strictEqual(prjShowsTs({ container_state: "exited" }), false, "止まっていれば出さない");
  assert.strictEqual(prjShowsTs({ container_state: "" }), false, "状態が無ければ出さない");
  assert.strictEqual(prjShowsTs(null), false, "壊れた行でも止めない");
}

function main() {
  testRelativeTime();
  testDonutSlices();
  testTodoFolderRuns();
  testProjectRowFacts();
  testValidShortName();
  testPrjShowsTs();
  console.log("test_project: すべて通りました");
}

main();
