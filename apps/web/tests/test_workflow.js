"use strict";
// Workflow 盤面（static/workflow.js・U5）の DOM を使わない部分のテスト（assert だけ）。
// 確かめること:
//   - 住所の往復（#/project/<name>/workflow）
//   - 文字マスを px にする矩形と、覚えたずらし量の足し方
//   - 札の題・時刻・説明の行（高さに収まるだけ）・印（running / failed / 要確認）・`<model> <effort>` の行
//   - 線のパス（左右 / 上下の向きと制御点）・色・破線・濃さ
//   - 全体表示（札と round の区切り線が入る拡大率）と題の大きさ
//   - 札の操作ボタン（レーンと状態による出し分け）
//   - Kanban の 3 レーンと round の区切り線
//   - ドラッグでずらした量をこの端末に覚える（localStorage）
// 実行:
//     node tests/test_workflow.js

const assert = require("assert");
const path = require("path");

// 盤面の全体表示と拡大は canvaslayout.js の純関数をそのまま使うので、
// node 側でも同じ関数が引けるようグローバルに入れておく（ブラウザでは script タグで同じになる）。
const layout = require(path.join(__dirname, "..", "src", "agent_link_web", "static", "canvaslayout.js"));
global.cvFitView = layout.cvFitView;
global.cvZoomAt = layout.cvZoomAt;

// node には localStorage / sessionStorage が無いので置き場を用意する
// （ドラッグでずらした量と見える範囲がこの端末にだけ覚えることを試すため）
const local = new Map();
const session = new Map();
global.localStorage = {
  getItem: (k) => (local.has(k) ? local.get(k) : null),
  setItem: (k, v) => { local.set(k, String(v)); },
  removeItem: (k) => { local.delete(k); },
};
global.sessionStorage = {
  getItem: (k) => (session.has(k) ? session.get(k) : null),
  setItem: (k, v) => { session.set(k, String(v)); },
  removeItem: (k) => { session.delete(k); },
};

const wf = require(path.join(__dirname, "..", "src", "agent_link_web", "static", "workflow.js"));

// C-L1 の形そのままの偽データ
const NODE = {
  id: 10,
  run_id: 10,
  name: "📋 plan",
  model: "opus",
  thinking: "high",
  status: "completed",
  created_at: "2026-09-26 10:00:00",
  description: "実装の計画を書く",
  cost: "$0.42",
  orphaned: false,
  parent_run_id: null,
  x: 0,
  y: 3,
  w: 27,
  h: 6,
};
const NODE2 = { ...NODE, id: 11, run_id: 11, x: 31, y: 3, model: "sonnet", thinking: null };

function testHash() {
  assert.strictEqual(wf.wfHash("agent-link-web"), "#/project/agent-link-web/workflow");
  // 日本語と空白を含む名前でも往復する
  const name = "/fonts/本 置き場";
  assert.strictEqual(wf.wfParseHash(wf.wfHash(name)).name, name);
  assert.strictEqual(wf.wfParseHash("#/project/agent-link-web/list"), null);
  assert.strictEqual(wf.wfParseHash("#/project/agent-link-web"), null);
  assert.deepStrictEqual(wf.wfParseHash("#/project/agent-link-web/workflow/"), { name: "agent-link-web" });
}

function testNodeRect() {
  const r = wf.wfNodeRect(NODE, null);
  // 横 1 マス = 10px・縦 1 マス = 20px
  assert.strictEqual(r.x, 0);
  assert.strictEqual(r.y, 60);
  assert.strictEqual(r.width, 270);
  assert.strictEqual(r.height, 120);
  // 覚えたずらし量を足す
  const moved = wf.wfNodeRect(NODE, { 10: [30, -15] });
  assert.strictEqual(moved.x, 30);
  assert.strictEqual(moved.y, 45);
  // 壊れたずらし量は 0 として扱う
  assert.deepStrictEqual(wf.wfNodeRect(NODE, { 10: "x" }), r);
  // 全部の矩形（全体表示の計算に使う）
  assert.strictEqual(wf.wfBoardRects([NODE, NODE2], null).length, 2);
  assert.strictEqual(wf.wfBoardRects(null, null).length, 0);
}

function testFitView() {
  // canvaslayout.js の cvFitView にそのまま渡せる（全体が入る矩形が返る）
  const rects = wf.wfBoardRects([NODE, NODE2], null);
  const view = wf.wfFitView(rects, 400, 300);
  assert.ok(view.z > 0 && view.z <= 1);
  // 拡大しても中心を保つ形（cvZoomAt と同じ関数を使う）
  const zoom = wf.wfZoomAt(view, 200, 150, 2);
  assert.ok(zoom.z > view.z);
}

function testFitRects() {
  // 札 2 枚 + round の区切り線 2 本。区切り線も全体表示の計算に入れる
  const seps = [{ y: 0, label: "round1" }, { y: 27, label: "round2" }];
  const rects = wf.wfFitRects([NODE, NODE2], null, seps);
  assert.strictEqual(rects.length, 4);
  // 区切り線は y を px にしただけの点（幅・高さは 0）
  assert.deepStrictEqual(rects[2], { x: 0, y: 0, width: 0, height: 0 });
  assert.deepStrictEqual(rects[3], { x: 0, y: 540, width: 0, height: 0 });
  // 区切り線を入れないと全体表示の下にはみ出して切れる
  const only = wf.wfFitRects([NODE, NODE2], null, null);
  assert.strictEqual(only.length, 2);
  const v1 = wf.wfFitView(only, 400, 300);
  const v2 = wf.wfFitView(rects, 400, 300);
  const last = rects[rects.length - 1];
  const bottom = last.y * v2.z + v2.ty;
  const bottomAlone = last.y * v1.z + v1.ty;
  assert.ok(bottomAlone > 300, "区切り線を入れないと画面の下にはみ出す");
  assert.ok(bottom <= 300, "区切り線を入れると画面の中に収まる");
  // 壊れた値でも止めない
  assert.strictEqual(wf.wfFitRects(null, null, "x").length, 0);
}

function testBoardRight() {
  // 盤面の右端（区切り線をそこまで引く）
  const rects = wf.wfBoardRects([NODE, NODE2], null);
  assert.strictEqual(wf.wfBoardRight(rects), 580);
  // ドラッグで右にずらしたら右端も伸びる
  assert.strictEqual(wf.wfBoardRight(wf.wfBoardRects([NODE, NODE2], { 11: [100, 0] })), 680);
  // 何も無ければ 0
  assert.strictEqual(wf.wfBoardRight(null), 0);
  assert.strictEqual(wf.wfBoardRight([null, {}]), 0);
}

function testZoomInv() {
  // 拡大率の逆数（題と線の大きさを画面上の px で保つために使う）
  assert.strictEqual(wf.wfZoomInv(0.5), 2);
  assert.strictEqual(wf.wfZoomInv(1), 1);
  // 壊れた拡大率で 0 にならないこと
  assert.strictEqual(wf.wfZoomInv(0), 1);
  assert.strictEqual(wf.wfZoomInv(-1), 1);
  assert.strictEqual(wf.wfZoomInv("x"), 1);
}

function testCardTitle() {
  assert.strictEqual(wf.wfCardTitle(NODE), "ID10/📋 plan");
  // 壊れた札でも止めない
  assert.strictEqual(wf.wfCardTitle(null), "");
}

function testStampText() {
  // 2026-09-26 10:00:00 は UTC と読む（project.js の parseStamp と同じ）
  const t = wf.wfStamp("2026-09-26 10:00:00");
  assert.strictEqual(new Date(t).toISOString(), "2026-09-26T10:00:00.000Z");
  assert.ok(/^\d{2}\/\d{2} \d{2}:\d{2}:\d{2}$/.test(wf.wfStampText("2026-09-26 10:00:00")));
  assert.strictEqual(wf.wfStampText(null), "");
  assert.strictEqual(wf.wfStamp("おかしな日"), null);
}

function testCardMarks() {
  // runner・review の印は無い（チップは要確認だけ）
  const m = wf.wfCardMarks(NODE);
  assert.strictEqual("runner" in m, false);
  assert.strictEqual(m.chips.length, 0);
  assert.deepStrictEqual(wf.wfCardMarks({ review: "empty" }).chips, []);
  // 実行中・失敗の点
  assert.strictEqual(wf.wfCardMarks({ status: "running" }).running, true);
  assert.strictEqual(wf.wfCardMarks({ status: "failed" }).failed, true);
  // running の取り残し
  assert.deepStrictEqual(wf.wfCardMarks({ orphaned: true }).chips, [{ text: "要確認", cls: "warn" }]);
}

function testCardModel() {
  assert.strictEqual(wf.wfCardModel({ model: "opus", thinking: "high" }), "opus high");
  assert.strictEqual(wf.wfCardModel({ model: "sonnet", thinking: null }), "sonnet");
  assert.strictEqual(wf.wfCardModel({ model: null, thinking: "low" }), "low");
  assert.strictEqual(wf.wfCardModel({ model: "", thinking: "" }), "");
  assert.strictEqual(wf.wfCardModel({}), "");
  assert.strictEqual(wf.wfCardModel(null), "");
}

function testDataPath() {
  assert.strictEqual(wf.wfDataPath("p", false), "/api/projects/p/workflow");
  assert.strictEqual(wf.wfDataPath("p", true), "/api/projects/p/workflow?subtasks=1");
  assert.strictEqual(wf.wfDataPath("a b/本", true), "/api/projects/a%20b%2F%E6%9C%AC/workflow?subtasks=1");
}

function testCardLines() {
  const lines = wf.wfCardLines(NODE);
  assert.strictEqual(lines.title, "ID10/📋 plan");
  assert.strictEqual(lines.model, "opus high");
  assert.strictEqual(lines.cost, "$0.42");
  assert.ok(lines.time.length > 0);
  // 高さ（h=6 → 120px）に収まる行数だけ説明を出す（model の行がある分 1 行減る）
  assert.ok(lines.desc.length <= Math.floor((120 - 34 - 14) / 14));
  assert.strictEqual(wf.wfCardLines({ ...NODE, model: null, thinking: null }).model, "");
  // 説明は横（w=27 → 270px）に収まるように折り返す
  const long = wf.wfCardLines({ ...NODE, description: "あ".repeat(200) });
  assert.ok(long.desc.length > 1);
  for (const line of long.desc) assert.ok(line.length <= 60, `1 行が長すぎます: ${line}`);
  // 説明が空でも止めない
  assert.deepStrictEqual(wf.wfCardLines({ ...NODE, description: "" }).desc, [""]);
  // 費用の無い実行
  assert.strictEqual(wf.wfCardLines({ ...NODE, cost: null }).cost, "");
}

function testWrapText() {
  assert.deepStrictEqual(wf.wfWrapText("abc", 10), ["abc"]);
  // 全角は 2 幅として数える（幅 4 なら全角 2 文字で折り返す）
  assert.deepStrictEqual(wf.wfWrapText("あああ", 4), ["ああ", "あ"]);
  // 改行も分ける
  assert.deepStrictEqual(wf.wfWrapText("a\nb", 10), ["a", "b"]);
}

function testEdgeColor() {
  // 色は CSS 変数。表の値と一致することだけ確かめる（値は style.css の :root にある）。
  assert.deepStrictEqual(wf.WF_EDGE_KINDS.map((r) => r[0]), ["delegation", "waggle_token"]);
  const table = Object.fromEntries(wf.WF_EDGE_KINDS.map((r) => [r[0], r[2]]));
  assert.strictEqual(wf.wfEdgeColor("delegation"), table.delegation);
  assert.strictEqual(wf.wfEdgeColor("waggle_token"), table.waggle_token);
  // 表に無い種類・消えた種類・null・未知の値は灰色（描画が止まらない）
  assert.strictEqual(wf.wfEdgeColor("task_output"), wf.WF_EDGE_UNKNOWN);
  assert.strictEqual(wf.wfEdgeColor("oreason"), wf.WF_EDGE_UNKNOWN);
  assert.strictEqual(wf.wfEdgeColor(null), wf.WF_EDGE_UNKNOWN);
  // 色は CSS 変数（16 進の直書きは無い）
  for (const [kind, color] of Object.entries({ ...table, unknown: wf.WF_EDGE_UNKNOWN })) {
    assert.ok(/^var\(--[A-Za-z0-9-]+\)$/.test(color), `線が CSS 変数でない: ${kind} ${color}`);
  }
  // 破線の種類は無い
  assert.strictEqual(wf.wfEdgeDashed("delegation"), false);
  assert.strictEqual(wf.wfEdgeDashed(null), false);
}

function testEdgePathSide() {
  const a = wf.wfNodeRect(NODE, null);   // x 0..270, y 60..180
  const b = wf.wfNodeRect(NODE2, null);  // x 310..580
  // 左右につなぐ（side=true）: from が左なら from の右辺 → to の左辺
  const right = wf.wfEdgePath(a, b, true);
  assert.ok(right.startsWith("M 270 120 C"), right);
  assert.ok(right.includes("310 120"), right);
  // 制御点は 40px 外へ（270 + 40 = 310 / 310 - 40 = 270）
  assert.ok(right.includes("310 120, 270 120,"), right);
  // from が右なら逆
  const left = wf.wfEdgePath(b, a, true);
  assert.ok(left.startsWith("M 310 120 C"), left);
  // 終点の印を置く場所
  assert.strictEqual(wf.wfEdgeEndX(a, b, true), 310);
  assert.strictEqual(wf.wfEdgeEndY(a, b, true), 120);
  assert.strictEqual(wf.wfEdgeEndX(b, a, true), 270);
}

function testEdgePathVertical() {
  const a = wf.wfNodeRect(NODE, null);       // y 60..180
  const b = wf.wfNodeRect({ ...NODE, x: 0, y: 12 }, null); // y 240..360
  // 上下につなぐ（side=false）: from が上であれば from の下辺 → to の上辺
  const down = wf.wfEdgePath(a, b, false);
  assert.ok(down.startsWith("M 135 180 C"), down);
  assert.ok(down.includes("135 240"), down);
  assert.strictEqual(wf.wfEdgeEndY(a, b, false), 240);
  assert.strictEqual(wf.wfEdgeEndX(a, b, false), 135);
  // from が下なら逆
  const up = wf.wfEdgePath(b, a, false);
  assert.ok(up.startsWith("M 135 240 C"), up);
  // 縦横が同じ高さなら from の下辺から始める（<= なので from を先に置く）
  const same = wf.wfEdgePath(a, { ...a, x: 400 }, false);
  assert.ok(same.startsWith(`M ${a.x + a.width / 2} ${a.y + a.height} C`), same);
}

function testEdgeOpacity() {
  // 選んでいなければ全部 1（線として読めるように 3:1 以上を保つ）
  assert.strictEqual(wf.wfEdgeOpacity({ from: 1, to: 2 }, null), 1);
  // 選んだ札につながる線は 1
  assert.strictEqual(wf.wfEdgeOpacity({ from: 1, to: 2 }, 1), 1);
  assert.strictEqual(wf.wfEdgeOpacity({ from: 1, to: 2 }, 2), 1);
  // それ以外の線だけ WF_EDGE_DIM まで薄くする（それ以上は薄くしない）
  assert.strictEqual(wf.wfEdgeOpacity({ from: 1, to: 2 }, 3), wf.WF_EDGE_DIM);
  assert.strictEqual(wf.wfEdgeOpacity(null, 3), wf.WF_EDGE_DIM);
  assert.ok(wf.WF_EDGE_DIM >= 0.8, "薄くしすぎて 3:1 を下回る値にしない");
}

function testLinkedTo() {
  const edges = [{ from: 1, to: 2 }, { from: 3, to: 4 }];
  assert.strictEqual(wf.wfLinkedTo(edges, 1, 2), true);
  assert.strictEqual(wf.wfLinkedTo(edges, 2, 1), true);
  assert.strictEqual(wf.wfLinkedTo(edges, 1, 3), false);
  assert.strictEqual(wf.wfLinkedTo(edges, 1, 1), true);
  assert.strictEqual(wf.wfLinkedTo(null, 1, 2), false);
}

function testSeparator() {
  // y は文字マスなので px にする
  assert.deepStrictEqual(wf.wfSeparatorLine({ y: 27, label: "round2" }), { y: 540, label: "round2" });
  assert.deepStrictEqual(wf.wfSeparatorLine(null), { y: 0, label: "" });
}

function testLanes() {
  const data = {
    lanes: {
      todo: [{ run_id: 56, title: "ID56/kantoku", date: "09/26 11:00", description: "x", executer: "" }],
      running: [{ run_id: 55, title: "ID55/📋 plan", executer: "cc" }],
    },
  };
  const lanes = wf.wfLanes(data);
  // 必ず 3 つ・並びは todo / running / review
  assert.deepStrictEqual(lanes.map((l) => l.key), ["todo", "running", "review"]);
  assert.strictEqual(lanes[0].cards.length, 1);
  assert.strictEqual(lanes[1].cards[0].executer, "cc");
  assert.strictEqual(lanes[2].cards.length, 0);
  assert.strictEqual(wf.wfLanesEmpty(data), false);
  // 全部空なら案内を出す
  assert.strictEqual(wf.wfLanesEmpty({ lanes: {} }), true);
  assert.strictEqual(wf.wfLanesEmpty(null), true);
  // run_id の無い物は落とす
  assert.strictEqual(wf.wfLanes({ lanes: { todo: [{ title: "x" }, null] } })[0].cards.length, 0);
}

function testStateLabel() {
  assert.strictEqual(wf.wfStateOf({ lane: "review" }), "review");
  assert.strictEqual(wf.wfStateOf({ status: "running" }), "running");
  assert.strictEqual(wf.wfStateOf(null), "");
  assert.strictEqual(wf.wfStateLabel("review"), "review");
  assert.strictEqual(wf.wfStateLabel("completed"), "完了");
  assert.strictEqual(wf.wfStateLabel("failed"), "失敗");
}

function testActions() {
  // todo は ▶ 実行
  const todo = wf.wfActions({ run_id: 56, status: "todo" }, "todo", "sai phone");
  assert.deepStrictEqual(todo.map((a) => a.key), ["start"]);
  assert.strictEqual(todo[0].method, "POST");
  assert.strictEqual(todo[0].path, "/api/projects/sai%20phone/runs/56/start");
  // review は Workflow へ移す
  const review = wf.wfActions({ run_id: 59, status: "completed" }, "review", "p");
  assert.deepStrictEqual(review.map((a) => a.key), ["advance"]);
  assert.strictEqual(review[0].path, "/api/projects/p/runs/59/advance");
  assert.strictEqual(review[0].to, "workflow");
  assert.strictEqual(review[0].label, "→ Workflow");
  assert.strictEqual(review[0].method, "POST");
  // done のレーンは無い（操作なし）
  assert.deepStrictEqual(wf.wfActions({ run_id: 60, status: "completed" }, "done", "p"), []);
  // 実行中で親が無いものだけ ■ 終了
  const run = wf.wfActions({ run_id: 61, status: "running", parent_run_id: null }, "running", "p");
  assert.deepStrictEqual(run.map((a) => a.key), ["finalize"]);
  assert.strictEqual(run[0].path, "/api/runs/p/61/finalize");
  assert.strictEqual(run[0].danger, true);
  // 親がある（サブの実行）は終了しない
  const sub = wf.wfActions({ run_id: 62, status: "running", parent_run_id: 61 }, "running", "p");
  assert.deepStrictEqual(sub, []);
  // Kanban の札には status が無いのでレーンで判断する
  const lane = wf.wfActions({ run_id: 63, title: "ID63/👷 worker", executer: "cc" }, "running", "p");
  assert.deepStrictEqual(lane.map((a) => a.key), ["finalize"]);
  // run_id が無い札は操作なし
  assert.deepStrictEqual(wf.wfActions({}, "todo", "p"), []);
}

function testLegend() {
  const legend = wf.wfLegend();
  assert.strictEqual("tags" in legend, false);
  assert.deepStrictEqual(legend.edges.map((e) => e.key), ["delegation", "waggle_token"]);
  assert.ok(legend.edges.every((e) => !e.dashed));
  assert.ok(legend.marks.some((m) => m.cls === "warn"));
  assert.ok(legend.marks.every((m) => !/review/.test(m.text)));
}

function testOffsetsMemory() {
  // 覚えたずらし量はこの端末（localStorage）にだけ覚える（agent-link へは送らない）
  assert.deepStrictEqual(wf.wfLoadOffsets("p"), {});
  wf.wfSaveOffset("p", 10, 30, -15);
  assert.deepStrictEqual(wf.wfLoadOffsets("p"), { 10: [30, -15] });
  // 別のプロジェクトは別
  assert.deepStrictEqual(wf.wfLoadOffsets("q"), {});
  // 並びを戻す = 覚えた量を全部消す
  wf.wfClearOffsets("p");
  assert.deepStrictEqual(wf.wfLoadOffsets("p"), {});
  // 記憶のキーが計画書と一致している
  assert.strictEqual(wf.WF_OFFSETS_KEY, "agent_link_web_wf_offsets");
  assert.strictEqual(wf.WF_VIEW_KEY, "agent_link_web_wf_view");
  assert.strictEqual(wf.WF_KANBAN_KEY, "agent_link_web_wf_kanban");
  assert.strictEqual(wf.WF_KINDS_KEY, "agent_link_web_wf_kinds");
  assert.strictEqual(wf.WF_SUBTASKS_KEY, "agent_link_web_wf_subtasks");
}

function testViewMemory() {
  assert.strictEqual(wf.wfLoadView("p"), null);
  wf.wfSaveView("p", { tx: 12, ty: 34, z: 1.5 });
  assert.deepStrictEqual(wf.wfLoadView("p"), { tx: 12, ty: 34, z: 1.5 });
  // 別のプロジェクトは別
  assert.strictEqual(wf.wfLoadView("q"), null);
  // 壊れた値は無視する
  session.set("agent_link_web_wf_view:p", JSON.stringify({ tx: "x" }));
  assert.strictEqual(wf.wfLoadView("p"), null);
  // 拡大率の上下限に収める
  session.set("agent_link_web_wf_view:p", JSON.stringify({ tx: 0, ty: 0, z: 99 }));
  assert.strictEqual(wf.wfLoadView("p").z, 2.5);
}

function testKanbanMemory() {
  // 最初は開いている
  assert.strictEqual(wf.wfLoadKanban("p"), true);
  wf.wfSaveKanban("p", false);
  assert.strictEqual(wf.wfLoadKanban("p"), false);
  assert.strictEqual(wf.wfLoadKanban("q"), true);
}

function testKindMemory() {
  // 消えた種類の覚えは無視する（最初の読み込みの前に古い値を入れておく）
  local.set("agent_link_web_wf_kinds", JSON.stringify({ phase_chain: false, task_output: false, git_commit: false }));
  const on = wf.wfVisibleKinds();
  assert.deepStrictEqual(Object.keys(on), ["delegation", "waggle_token"]);
  local.delete("agent_link_web_wf_kinds");
  // 最初は全部出す
  for (const [key] of wf.WF_EDGE_KINDS) assert.strictEqual(on[key], true);
  // 出し入れはこの端末に覚える（画面側はキャッシュするので、保存したあとの中身を読む）
  wf.wfSaveKinds({ ...on, waggle_token: false });
  const saved = JSON.parse(local.get("agent_link_web_wf_kinds"));
  assert.strictEqual(saved.waggle_token, false);
  assert.strictEqual(saved.delegation, true);
}

function testConstants() {
  assert.strictEqual(wf.WF_CELL_W, 10);
  assert.strictEqual(wf.WF_CELL_H, 20);
  assert.strictEqual(wf.WF_INTERVAL_MS, 10000);
  assert.strictEqual(wf.WF_SEL_COLOR, "var(--sel)");
  assert.strictEqual(wf.WF_DRAG_SLOP, 4);
  // 題は画面上 12px 以上で読めること（WF_TITLE_PX がその大きさ）
  assert.ok(wf.WF_TITLE_PX >= 12, "題は 12px 以上で読めること");
  // 題の世界上の大きさの上限は札の高さ（h=6 → 120px）に収まること
  assert.ok(wf.WF_TITLE_MAX_PX < 120, "題が札の高さからはみ出さないこと");
  // 線の太さは画面上 1.5px 以上
  assert.ok(wf.WF_EDGE_W_PX >= 1.5, "線の太さは 1.5px 以上");
  assert.ok(wf.WF_EDGE_W_SEL_PX > wf.WF_EDGE_W_PX, "選んだ線はもう少し太い");
}

function main() {
  testHash();
  testNodeRect();
  testFitView();
  testFitRects();
  testBoardRight();
  testZoomInv();
  testCardTitle();
  testStampText();
  testCardMarks();
  testCardModel();
  testDataPath();
  testCardLines();
  testWrapText();
  testEdgeColor();
  testEdgePathSide();
  testEdgePathVertical();
  testEdgeOpacity();
  testLinkedTo();
  testSeparator();
  testLanes();
  testStateLabel();
  testActions();
  testLegend();
  testOffsetsMemory();
  testViewMemory();
  testKanbanMemory();
  testKindMemory();
  testConstants();
  console.log("test_workflow: すべて通りました");
}

main();
