"use strict";
// 研究方針の木の画面（scitree.js）の純関数のテスト（計画 U19「変更内容 7」）。
// 実行:
//     node apps/phone/tests/test_scitree.js
//
// ここに無いもの（DOM が要る描画・操作）はブラウザで確かめる（計画 U19 の AC1〜AC4）。

const assert = require("assert");
const fs = require("fs");
const path = require("path");

const src = path.join(__dirname, "..", "src", "sai_phone", "static", "scitree.js");
const {
  stParseHash,
  stStateMark,
  stRows,
  stSeedPieces,
  stGateInfo,
  stDebtChips,
  stNextLines,
  stDecisionRows,
  stGraphBoxId,
  stQuestionLabel,
  stTableEmptyHint,
  stLoadMode,
  stSaveMode,
  stRunMetrics,
  stMlfTag,
  stMlfMetricValue,
  stRunStatusLabel,
  ST_MODE_KEY,
  ST_RUN_LAYERS,
  ST_RUN_LAYER_LABELS,
  ST_RUN_PAGE_SIZE,
  ST_RUN_MAX_PAGES,
  ST_RUN_NO_MLF,
  ST_RUN_NO_METRIC,
  ST_MODES,
  ST_TYPES,
  ST_ITEM_TYPES,
  ST_ITEM_MARKS,
  ST_OPTION_MARKS,
  ST_BLANKS,
  ST_NODE_MARKS,
} = require(src);

/** C-08 の status の見本（途中だけ使う）。 */
function sampleStatus() {
  return {
    initialized: true,
    type: { id: "problem", name: "課題駆動" },
    seed: {
      text: "「頭皮脳波で［？小目的］を行う。…」",
      parts: [
        { text: "頭皮脳波", key: "data.type", node: "N9", filled: true, tentative: false },
        { text: "で", key: null, node: null, filled: true, tentative: false },
        { text: "［？小目的］", key: "purpose.subgoal", node: "N12", filled: false, tentative: false },
        { text: "を行う", key: "purpose.verb", node: "N13", filled: true, tentative: true },
      ],
    },
    gate: {
      current: 2,
      total: 7,
      name: "小目的",
      list: [
        { name: "出発点", when: "decided(background.goal)", passed: true },
        { name: "小目的", when: "decided(purpose.subgoal)", passed: false },
      ],
    },
    debts: { pending_proposals: 3, unreflected_papers: 2, unscreened_sets: 0, empty_reviews: 0 },
    nodes: [
      {
        id: "N9", kind: "item", key: "data.type", title: "頭皮脳波", types: ["choose"],
        state: "decided", fixed: true, parents: [], depth: 0, ord: 0,
        value: "頭皮脳波", blank: null, marks: [],
        counts: { questions_open: 0, options: 1, support: 2, refute: 0, papers: 1, pending: 0 },
      },
      {
        id: "N12", kind: "item", key: "purpose.subgoal", title: "小目的", types: ["choose"],
        state: "open", fixed: false, parents: ["N9"], depth: 1, ord: 0,
        value: "", blank: "awaiting_decision", marks: [],
        counts: { questions_open: 1, options: 2, support: 3, refute: 0, papers: 4, pending: 1 },
      },
      {
        id: "N20", kind: "option", key: null, title: "発作部位の予測", types: [],
        state: "candidate", fixed: false, parents: ["N12"], depth: 2, ord: 0,
        value: "", blank: null, marks: [],
        counts: { questions_open: 0, options: 0, support: 2, refute: 1, papers: 3, pending: 0 },
      },
      {
        id: "N21", kind: "option", key: null, title: "応答潜時との比較", types: [],
        state: "rejected", fixed: false, parents: ["N12"], depth: 2, ord: 1,
        value: "", blank: null, marks: [],
        counts: { questions_open: 0, options: 0, support: 0, refute: 2, papers: 1, pending: 0 },
      },
      {
        id: "N13", kind: "item", key: "purpose.verb", title: "を行う", types: ["write"],
        state: "tentative", fixed: false, parents: [], depth: 0, ord: 1,
        value: "行う", blank: null, marks: ["weak_evidence"],
        counts: { questions_open: 0, options: 0, support: 0, refute: 0, papers: 0, pending: 0 },
      },
    ],
    edges: [{ kind: "depends", from: "N9", to: "N13" }],
    next: [
      { node: "N12", block: "sci-compare", purpose: "小目的の候補を価値の軸で比べる", expect: "比較表と推奨", why: "判断待ち", kind: "blank" },
      { node: null, block: null, purpose: "仕分けていない提案が 3 件あります", expect: "", why: "負債", kind: "debt" },
      { node: "N13", block: "sci-verify", purpose: "根拠を確かめる", expect: "根拠の一覧", why: "根拠不足", kind: "blank" },
    ],
    running: [],
    seeds: [{ id: "S1", title: "…" }],
    warnings: [],
  };
}

function testQuestionLabel() {
  assert.strictEqual(stQuestionLabel({ state: "open" }), "答え待ち");
  assert.strictEqual(stQuestionLabel({ state: "answered", answer: "x" }), "答えた");
  assert.strictEqual(stQuestionLabel({ state: "withdrawn", answer: "x" }), "取り下げた");
}

function testParseHash() {
  assert.deepStrictEqual(stParseHash("#/papers/tree"), { node: null });
  assert.deepStrictEqual(stParseHash("#/papers/tree/N12"), { node: "N12" });
  // 数だけの形（N の無い 12）と、0 や 10 桁以上ぎみのものは受け付けない
  assert.strictEqual(stParseHash("#/papers/tree/12"), null);
  assert.strictEqual(stParseHash("#/papers/tree/N0"), null);
  assert.strictEqual(stParseHash("#/papers/tree/Q7"), null);
  assert.strictEqual(stParseHash("#/papers/tree/N12/x"), null);
  assert.strictEqual(stParseHash("#/papers/library"), null);
  assert.strictEqual(stParseHash(""), null);
}

function testStateMark() {
  // 項目と候補で別の記号・表示名
  assert.deepStrictEqual(stStateMark("item", "decided"), { mark: "●", label: "決定", cls: "sci-mark-decided" });
  assert.deepStrictEqual(stStateMark("item", "tentative"), { mark: "◐", label: "仮決定", cls: "sci-mark-tentative" });
  assert.deepStrictEqual(stStateMark("item", "open"), { mark: "○", label: "未決定", cls: "sci-mark-open" });
  assert.deepStrictEqual(stStateMark("item", "excluded"), { mark: "×", label: "対象外", cls: "sci-mark-excluded" });
  assert.deepStrictEqual(stStateMark("option", "candidate"), { mark: "◇", label: "候補", cls: "sci-mark-candidate" });
  assert.deepStrictEqual(stStateMark("option", "selected"), { mark: "◆", label: "選択", cls: "sci-mark-selected" });
  assert.deepStrictEqual(stStateMark("option", "rejected"), { mark: "✕", label: "却下", cls: "sci-mark-rejected" });
  assert.deepStrictEqual(stStateMark("option", "dormant"), { mark: "◌", label: "休眠", cls: "sci-mark-dormant" });
  // 知らない状態は未決定・候補に寄せる（表示が壊れないように）
  assert.deepStrictEqual(stStateMark("item", "???"), stStateMark("item", "open"));
  assert.deepStrictEqual(stStateMark("option", undefined), stStateMark("option", "candidate"));
}

function testRowsOrderAndDepth() {
  const rows = stRows(sampleStatus());
  assert.deepStrictEqual(rows.map((r) => r.id), ["N9", "N12", "N20", "N21", "N13"]);
  // 候補は項目の 1 段下
  const item = rows[1];
  const opt = rows[2];
  assert.strictEqual(item.indent, 1);
  assert.strictEqual(opt.indent, item.indent + 1);
  // 一番上の項目は 0
  assert.strictEqual(rows[0].indent, 0);
}

function testRowsMarks() {
  const rows = stRows(sampleStatus());
  const n12 = rows[1];
  assert.strictEqual(n12.mark, "○");
  assert.strictEqual(n12.label, "未決定");
  assert.strictEqual(n12.blank, "判断待ち"); // awaiting_decision → 日本語
  assert.strictEqual(n12.papers, 4);
  assert.strictEqual(n12.pending, 1);
  // 固定の項目は📌、印（weak_evidence）は日本語
  assert.strictEqual(rows[0].fixed, true);
  assert.deepStrictEqual(rows[4].marks, ["根拠不足"]);
  // 対象外・休眠など、候補以外は空欄の種類を持たない
  assert.strictEqual(rows[3].blank, "");
  assert.strictEqual(rows[3].mark, "✕");
}

function testRowsWithoutStatus() {
  assert.deepStrictEqual(stRows(null), []);
  assert.deepStrictEqual(stRows({}), []);
  assert.deepStrictEqual(stRows({ nodes: [null, undefined] }), []);
}

function testSeedPieces() {
  const pieces = stSeedPieces(sampleStatus());
  assert.strictEqual(pieces.length, 4);
  // 押せるのは filled が false の部分だけ
  assert.deepStrictEqual(pieces.map((p) => p.canOpen), [false, false, true, false]);
  assert.strictEqual(pieces[2].canOpen, true);
  assert.strictEqual(pieces[2].node, "N12");
  // 仮決定は印が付く
  assert.strictEqual(pieces[3].tentative, true);
  assert.strictEqual(pieces[0].tentative, false);
}

function testSeedPiecesBlankWithoutNode() {
  // 空欄でも節点が立っていないものは押せない
  const pieces = stSeedPieces({ seed: { parts: [{ text: "［？］", key: "k", node: null, filled: false }] } });
  assert.strictEqual(pieces.length, 1);
  assert.strictEqual(pieces[0].canOpen, false);
  assert.strictEqual(stSeedPieces(null).length, 0);
  assert.strictEqual(stSeedPieces({ seed: {} }).length, 0);
}

function testGateInfo() {
  const g = stGateInfo(sampleStatus());
  assert.strictEqual(g.current, 2);
  assert.strictEqual(g.total, 7);
  assert.strictEqual(g.all, false);
  assert.strictEqual(g.name, "小目的");
  assert.strictEqual(g.list.length, 2);
  // 全部通っていれば all が真で「種にできる」
  const done = stGateInfo({ gate: { current: 8, total: 7, name: "小目的", list: [] } });
  assert.strictEqual(done.all, true);
  assert.strictEqual(done.name, "種にできる");
  // 関門が無いとき
  const none = stGateInfo(null);
  assert.strictEqual(none.all, false);
  assert.strictEqual(none.total, 0);
  assert.strictEqual(stGateInfo({ gate: { current: 0, total: 0, name: "x" } }).all, false);
}

function testDebtChips() {
  const chips = stDebtChips(sampleStatus());
  assert.strictEqual(chips.length, 4);
  assert.deepStrictEqual(chips.map((c) => c.key), [
    "pending_proposals", "unreflected_papers", "unscreened_sets", "empty_reviews",
  ]);
  // 0 の負債は薄くする
  assert.deepStrictEqual(chips.map((c) => c.dim), [false, false, true, true]);
  // 仕分けていない提案だけは、押すと仕分けの画面へ
  assert.strictEqual(chips[0].hash, "#/papers/triage");
  assert.strictEqual(chips[1].hash, "");
  // debts が無ければ全部 0 で薄く
  for (const c of stDebtChips({})) assert.strictEqual(c.dim, true);
}

function testNextLines() {
  const lines = stNextLines(sampleStatus(), 3);
  assert.strictEqual(lines.length, 3);
  assert.ok(lines[0].startsWith("N12 "));
  assert.ok(lines[0].includes("判断待ち"));
  assert.ok(lines[1].includes("負債"));
  // 0 件までで切れる
  assert.strictEqual(stNextLines(sampleStatus(), 2).length, 2);
  assert.strictEqual(stNextLines({ next: [] }, 3).length, 0);
  assert.deepStrictEqual(stNextLines(null, 3), []);
}

function testDecisionRowsNewestFirst() {
  const rows = stDecisionRows([
    { action: "decide", option: "N20", created_at: "2026-09-20 10:00:00" },
    { action: "tentative", option: "N21", created_at: "2026-09-28 10:00:00" },
    { action: "reopen", option: "N20", created_at: "2026-09-25 10:00:00" },
  ]);
  assert.deepStrictEqual(rows.map((r) => r.action), ["tentative", "reopen", "decide"]);
  assert.deepStrictEqual(stDecisionRows(null), []);
  assert.deepStrictEqual(stDecisionRows([null, undefined]), []);
}

function testDecisionRowsNoTime() {
  // .created_at が無いときは並び決めができないので、入力の後勝ち（新しい順と同じ）に並べる
  const rows = stDecisionRows([{ action: "a" }, { action: "b" }, { action: "c" }]);
  assert.deepStrictEqual(rows.map((r) => r.action), ["c", "b", "a"]);
}

function testTablesAreShared() {
  // 項目の型は 4 つ（選ぶ・並べる・書く・確かめる）
  assert.deepStrictEqual(ST_ITEM_TYPES.map((x) => x[0]), ["choose", "list", "write", "verify"]);
  // 表示の切り替えは 4 つ（一覧・根の図・表・実行の図）
  assert.deepStrictEqual(ST_MODES, [
    ["list", "一覧"], ["graph", "根の図"], ["table", "表"], ["runs", "実行の図"],
  ]);
  // 研究の型は 5 つ、C-14 の用語の札の解説の順
  assert.deepStrictEqual(ST_TYPES.map((x) => x[0]), ["problem", "phenomenon", "method", "data", "improve"]);
  // 印と表示名は 1 対 1
  assert.deepStrictEqual(Object.keys(ST_ITEM_MARKS), ["decided", "tentative", "open", "excluded"]);
  assert.deepStrictEqual(Object.keys(ST_OPTION_MARKS), ["candidate", "selected", "rejected", "dormant"]);
  assert.deepStrictEqual(Object.keys(ST_BLANKS), [
    "untouched", "in_progress", "insufficient", "awaiting_decision", "conflict",
  ]);
  assert.deepStrictEqual(Object.keys(ST_NODE_MARKS), ["weak_evidence", "recheck"]);
}

function testGraphBoxId() {
  // 指摘 3: 根の図は広い画面なら板（#sci-board）、狭い画面なら一覧の欄（#sci-list）
  assert.strictEqual(stGraphBoxId(true), "sci-board");
  assert.strictEqual(stGraphBoxId(false), "sci-list");
}

function testTableEmptyHint() {
  // 指摘 4: 節点が選ばれていないときは「まだ無い」ではなく「選んでください」
  assert.strictEqual(stTableEmptyHint({ id: "N12" }), null);
  const hint = stTableEmptyHint(null);
  assert.strictEqual(hint.title, "項目を選んでください");
  assert.ok(!/まだありません/.test(hint.body), hint.body);
  // 節点が無くても「表の画面はまだありません」とは言わない
  assert.ok(!/画面はまだありません/.test(hint.title + hint.body), hint);
}

function testListIsFullWidthOnNarrow() {
  // 指摘 6: 一覧の欄を 320px に固定するのは 900px 以上だけ（iPhone 390px で右に隙間を空けない）
  const css = fs.readFileSync(
    path.join(__dirname, "..", "src", "sai_phone", "static", "scitree.css"),
    "utf8"
  );
  // 固定の宣言が @media (min-width: 900px) の中にあるか
  const at = css.indexOf(".sci-body .sci-list {\n    flex: 0 0 320px;");
  assert.ok(at > 0, "一覧の欄の固定が @media の中に無い");
  const before = css.slice(0, at);
  const media = before.lastIndexOf("@media (min-width: 900px)");
  const close = before.lastIndexOf("}");
  assert.ok(media > close, "一覧の欄の固定が media query の外にある");
}

// --- 実行の図（U08） ---

/** localStorage の代わり（記憶した表示の切り替えを読み書きする）。 */
function fakeLocalStorage() {
  const map = new Map();
  return {
    getItem: (k) => (map.has(String(k)) ? map.get(String(k)) : null),
    setItem: (k, v) => { map.set(String(k), String(v)); },
    removeItem: (k) => { map.delete(String(k)); },
  };
}

function withLocalStorage(store, fn) {
  const had = Object.prototype.hasOwnProperty.call(global, "localStorage");
  const before = had ? global.localStorage : null;
  global.localStorage = store;
  try {
    return fn(store);
  } finally {
    if (had) global.localStorage = before;
    else delete global.localStorage;
  }
}

function testModeRunsIsRemembered() {
  // AC2: 「実行の図」を覚えて開き直すと、その表示になる
  const store = fakeLocalStorage();
  withLocalStorage(store, () => {
    assert.strictEqual(stLoadMode(), "list"); // 覚えていなければ一覧
    stSaveMode("runs");
    assert.strictEqual(store.getItem(ST_MODE_KEY), "runs");
    // 他の表示も覚える
    for (const [key] of ST_MODES) {
      stSaveMode(key);
      assert.strictEqual(stLoadMode(), key);
    }
    // 表に無い値は覚えない（一覧に寄る）
    store.setItem(ST_MODE_KEY, "???");
    assert.strictEqual(stLoadMode(), "list");
  });
  // 開き直す（読み直しのたびに同じ記憶を読む）
  assert.strictEqual(withLocalStorage(store, () => stLoadMode()), "list");
  withLocalStorage(store, () => stSaveMode("runs"));
  assert.strictEqual(withLocalStorage(store, () => stLoadMode()), "runs");
  // localStorage が無くても止まらない
  const had = Object.prototype.hasOwnProperty.call(global, "localStorage");
  const before = had ? global.localStorage : null;
  delete global.localStorage;
  try {
    assert.strictEqual(stLoadMode(), "list");
    stSaveMode("runs");
  } finally {
    if (had) global.localStorage = before;
  }
}

/** MLflow の親 run 1 枚（run_id は sai.run_id のタグ、値は exp.metric の名前の指標）。 */
function mlfParent(saiRunId, metric, value, direction) {
  const tags = [{ key: "exp.role", value: "script" }];
  if (saiRunId != null) tags.push({ key: "sai.run_id", value: String(saiRunId) });
  if (metric != null) tags.push({ key: "exp.metric", value: metric });
  if (direction != null) tags.push({ key: "exp.direction", value: direction });
  const metrics = metric != null && value != null ? [{ key: metric, value }] : [];
  return { info: { run_id: `m${saiRunId}-${metric}` }, data: { tags, metrics } };
}

function testRunMetrics() {
  // 親 run → {"<sai.run_id>": "<指標名> <値>"}
  const one = stRunMetrics([mlfParent(120, "val_auc", 0.871, "maximize")]);
  assert.deepStrictEqual(one, { 120: "val_auc 0.871" });
  // 別の指標名も同じ形
  assert.deepStrictEqual(stRunMetrics([mlfParent(121, "val_f1", 0.42, "minimize")]), { 121: "val_f1 0.42" });
  // 別の run は別のキー
  const two = stRunMetrics([mlfParent(120, "val_auc", 0.871, "maximize"), mlfParent(121, "val_auc", 0.9, "maximize")]);
  assert.deepStrictEqual(two, { 120: "val_auc 0.871", 121: "val_auc 0.9" });
}

function testRunMetricsMaximize() {
  // 同じ sai.run_id の親 run が 2 つ。maximize なら大きいほう
  const runs = [
    mlfParent(120, "val_auc", 0.871, "maximize"),
    mlfParent(120, "val_auc", 0.913, "maximize"),
  ];
  assert.deepStrictEqual(stRunMetrics(runs), { 120: "val_auc 0.913" });
  // 並びが逆でも同じ（方向で決める）
  const rev = [mlfParent(120, "val_auc", 0.913, "maximize"), mlfParent(120, "val_auc", 0.871, "maximize")];
  assert.deepStrictEqual(stRunMetrics(rev), { 120: "val_auc 0.913" });
}

function testRunMetricsMinimize() {
  // minimize なら小さいほう
  const runs = [
    mlfParent(120, "val_loss", 0.31, "minimize"),
    mlfParent(120, "val_loss", 0.17, "minimize"),
  ];
  assert.deepStrictEqual(stRunMetrics(runs), { 120: "val_loss 0.17" });
  // 方向のタグが無ければ maximize として扱う
  const none = [mlfParent(120, "val_auc", 0.1, null), mlfParent(120, "val_auc", 0.7, null)];
  assert.deepStrictEqual(stRunMetrics(none), { 120: "val_auc 0.7" });
  // 知らない方向の値（"minimize" 以外）は maximize 側
  const odd = [mlfParent(120, "val_auc", 0.1, "minimise"), mlfParent(120, "val_auc", 0.7, "minimise")];
  assert.deepStrictEqual(stRunMetrics(odd), { 120: "val_auc 0.7" });
}

function testRunMetricsSkips() {
  // exp.metric が無い run は飛ばす
  assert.deepStrictEqual(stRunMetrics([mlfParent(120, null, 0.5, "maximize")]), {});
  // sai.run_id が無い run も飛ばす
  assert.deepStrictEqual(stRunMetrics([mlfParent(null, "val_auc", 0.5, "maximize")]), {});
  // exp.metric の名前の指標が記録されていない run も飛ばす
  const noValue = { info: { run_id: "x" }, data: { tags: [{ key: "sai.run_id", value: "120" }, { key: "exp.metric", value: "val_auc" }], metrics: [] } };
  assert.deepStrictEqual(stRunMetrics([noValue]), {});
  // 壊れた引数でも止まらない
  assert.deepStrictEqual(stRunMetrics(null), {});
  assert.deepStrictEqual(stRunMetrics([]), {});
  assert.deepStrictEqual(stRunMetrics([null, {}, { data: null }]), {});
}

function testMlfTagAndValue() {
  const run = mlfParent(120, "val_auc", 0.871, "maximize");
  assert.strictEqual(stMlfTag(run, "exp.role"), "script");
  assert.strictEqual(stMlfTag(run, "sai.run_id"), "120");
  assert.strictEqual(stMlfTag(run, "exp.direction"), "maximize");
  // 無いタグ・壊れた run でも止まらない
  assert.strictEqual(stMlfTag(run, "exp.metric"), "val_auc");
  assert.strictEqual(stMlfTag(run, "???"), "");
  assert.strictEqual(stMlfTag(null, "exp.role"), "");
  assert.strictEqual(stMlfTag({}, "exp.role"), "");
  // 指標の値はその名前のものだけ
  assert.strictEqual(stMlfMetricValue(run, "val_auc"), "0.871");
  assert.strictEqual(stMlfMetricValue(run, "exec_time_s"), null);
  assert.strictEqual(stMlfMetricValue(null, "val_auc"), null);
}

function testRunStatusLabel() {
  // 状態は日本語
  assert.strictEqual(stRunStatusLabel("running"), "実行中");
  assert.strictEqual(stRunStatusLabel("completed"), "完了");
  assert.strictEqual(stRunStatusLabel("failed"), "失敗");
  assert.strictEqual(stRunStatusLabel("cancelled"), "中止");
  // 知らない状態はそのまま（空なら「待ち」）
  assert.strictEqual(stRunStatusLabel("???"), "???");
  assert.strictEqual(stRunStatusLabel(""), "待ち");
}

function testRunConstants() {
  // 層は 3 つ（C-02）。帯の語も 3 つ
  assert.deepStrictEqual(ST_RUN_LAYERS, ["root", "seed", "branch"]);
  assert.deepStrictEqual(Object.keys(ST_RUN_LAYER_LABELS), ["root", "seed", "branch"]);
  // 1 回の検索は 100 行、頁を繰るのは 10 まで
  assert.strictEqual(ST_RUN_PAGE_SIZE, 100);
  assert.strictEqual(ST_RUN_MAX_PAGES, 10);
  // MLflow が読めなかったときの注（503）と、検索が通らなかったときの注は別にする
  assert.ok(ST_RUN_NO_MLF.includes("MLflow"));
  assert.notStrictEqual(ST_RUN_NO_MLF, ST_RUN_NO_METRIC);
  assert.ok(ST_RUN_NO_METRIC.includes("指標"));
}

const tests = [
  testParseHash,
  testStateMark,
  testRowsOrderAndDepth,
  testRowsMarks,
  testRowsWithoutStatus,
  testSeedPieces,
  testSeedPiecesBlankWithoutNode,
  testGateInfo,
  testDebtChips,
  testNextLines,
  testDecisionRowsNewestFirst,
  testDecisionRowsNoTime,
  testTablesAreShared,
  testGraphBoxId,
  testTableEmptyHint,
  testQuestionLabel,
  testListIsFullWidthOnNarrow,
  testModeRunsIsRemembered,
  testRunMetrics,
  testRunMetricsMaximize,
  testRunMetricsMinimize,
  testRunMetricsSkips,
  testMlfTagAndValue,
  testRunStatusLabel,
  testRunConstants,
];

let failed = 0;
for (const fn of tests) {
  try {
    fn();
    console.log(`ok  ${fn.name}`);
  } catch (e) {
    failed += 1;
    console.error(`NG  ${fn.name}: ${e && e.message}`);
  }
}
if (failed) {
  console.error(`test_scitree: ${failed} 件が失敗しました`);
  process.exit(1);
}
console.log(`test_scitree: ${tests.length} 件すべて通りました`);
