"use strict";
// 実験タブ（mlflow.js）の純関数のテスト（実装計画 U06「完了条件」AC1〜AC3）。
// 実行:
//     node apps/web/tests/test_mlflow.js
//
// DOM が要る描画・操作（一覧を開く・成果物を押す・2 列に並べる）はブラウザで確かめる
// （AC5）。ここでは共有契約 C-07 の住所・検索式・入れ子の木だけを確かめる。

const assert = require("assert");
const path = require("path");

const src = path.join(__dirname, "..", "src", "sai_web", "static", "mlflow.js");
const {
  mlfParseHash,
  mlfHash,
  mlfParseQuery,
  mlfQueryString,
  mlfFilterFromQuery,
  mlfBuildTree,
  mlfFlatRows,
  mlfOrderOptions,
  mlfIsMetricOrder,
  mlfTag,
  mlfMetricValue,
  mlfMetricList,
  mlfHeadMetric,
  mlfParentId,
  mlfSproutId,
  mlfStatusText,
  mlfDurationText,
  mlfDurationMs,
  mlfValueText,
  mlfArtifactKind,
  mlfProjectChoices,
  mlfCompareRows,
  mlfCompareSectionTitle,
  mlfCompareHeadName,
  mlfMetricKeys,
  mlfMetricKeysOf,
  MLF_QUERY_KEYS,
  MLF_COMPARE_MIN,
  MLF_COMPARE_MAX,
  MLF_BULK_RUNS,
} = require(src);

const RUN_A = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const RUN_B = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const RUN_C = "cccccccccccccccccccccccccccccccc";
const RUN_D = "dddddddddddddddddddddddddddddddd";
const MISSING = "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee";

// 解析の返り値を、比較しやすい形に揃える物を作る
function parsed(hash) {
  const p = mlfParseHash(hash);
  assert.ok(p, `${hash} は解析できませんでした`);
  return p;
}

// --- 見本の run -------------------------------------------------------

/** テスト用の run を作る。{id, seed, parent, role, metric, value, status} */
function sampleRun(o) {
  const tags = [];
  if (o.sprout != null) tags.push({ key: "sai.sprout_run_id", value: String(o.sprout) });
  if (o.seed != null) tags.push({ key: "sai.seed_run_id", value: String(o.seed) });
  if (o.direction) tags.push({ key: "exp.direction", value: o.direction });
  if (o.action) tags.push({ key: "exp.action", value: o.action });
  if (o.model) tags.push({ key: "exp.model", value: o.model });
  if (o.parentBranch != null) tags.push({ key: "exp.parent_branch", value: String(o.parentBranch) });
  if (o.parent != null) tags.push({ key: "mlflow.parentRunId", value: o.parent });
  if (o.role) tags.push({ key: "exp.role", value: o.role });
  if (o.metric) tags.push({ key: "exp.metric", value: o.metric });
  if (o.saiRun != null) tags.push({ key: "sai.run_id", value: String(o.saiRun) });
  const metrics = [];
  if (o.metric) metrics.push({ key: o.metric, value: o.value, step: 0, timestamp: 1 });
  if (o.otherMetric) metrics.push({ key: o.otherMetric, value: o.otherValue, step: 0, timestamp: 1 });
  return {
    info: {
      run_id: o.id,
      run_name: o.name || `run ${o.id.slice(0, 4)}`,
      status: o.status || "FINISHED",
      start_time: o.start || "2026-09-29T10:00:00Z",
      end_time: o.end || "2026-09-29T10:01:00Z",
      experiment_id: o.experiment || "1",
      user: "sai",
    },
    data: {
      metrics,
      params: o.params || [{ key: "epochs", value: "3" }],
      tags,
    },
  };
}

// ============================================================================
// AC1: 住所の往復
// ============================================================================

function testParseExperiments() {
  assert.deepStrictEqual(mlfParseHash("#/research/experiments"), {
    page: "experiments",
    experimentId: null,
    runId: null,
    runIds: null,
    query: {},
  });
}

function testParseRuns() {
  const p = parsed("#/research/experiments/e/1");
  assert.strictEqual(p.page, "runs");
  assert.strictEqual(p.experimentId, "1");
  assert.deepStrictEqual(p.query, {});

  const all = parsed("#/research/experiments/e/all");
  assert.strictEqual(all.experimentId, "all");
}

function testParseRunsQuery() {
  const p = parsed("#/research/experiments/e/3?sai_run=121&sprout=120&study=N12-s120&status=FINISHED");
  assert.deepStrictEqual(p.query, {
    sai_run: "121",
    sprout: "120",
    study: "N12-s120",
    status: "FINISHED",
  });
}

function testParseRunDetail() {
  const p = parsed(`#/research/experiments/run/${RUN_A}`);
  assert.strictEqual(p.page, "run");
  assert.strictEqual(p.runId, RUN_A);
}

function testParseCompare() {
  const p = parsed(`#/research/experiments/compare/${RUN_A},${RUN_B}`);
  assert.strictEqual(p.page, "compare");
  assert.deepStrictEqual(p.runIds, [RUN_A, RUN_B]);
}

function testUnknownIsNull() {
  // 知らないタブ・知らない画面・形の違う ID はすべて null
  for (const hash of [
    "#/",
    "#/home",
    "#/papers/tree",
    "#/research/experimentsx",
    "#/research/experiments/e",
    "#/research/experiments/e/",
    "#/research/experiments/e/abc",
    "#/research/experiments/e/1/2",
    "#/research/experiments/run/zzz",
    "#/research/experiments/run/abc",
    `#/research/experiments/compare/${RUN_A}`, // 1 件だけの比較
    "#/research/experiments/compare/a,b",
  ]) {
    assert.strictEqual(mlfParseHash(hash), null, `${hash} は null のはず`);
  }
}

function testRoundTrip() {
  const states = [
    { page: "experiments" },
    { page: "runs", experimentId: "1" },
    { page: "runs", experimentId: "all" },
    { page: "runs", experimentId: "7", query: { sprout: "120" } },
    { page: "runs", experimentId: "7", query: { sai_run: "121", study: "N12-s120", status: "RUNNING" } },
    { page: "run", runId: RUN_A },
    { page: "compare", runIds: [RUN_A, RUN_B] },
  ];
  for (const state of states) {
    const hash = mlfHash(state);
    assert.ok(hash, `${JSON.stringify(state)} から住所を作りませんでした`);
    assert.deepStrictEqual(
      mlfParseHash(hash),
      {
        page: state.page,
        experimentId: state.experimentId ?? null,
        runId: state.runId ?? null,
        runIds: state.runIds ?? null,
        query: state.query ?? {},
      },
      `${hash} の往復が違います`
    );
  }
}

function testHashRejectsBadState() {
  assert.strictEqual(mlfHash(null), null);
  assert.strictEqual(mlfHash({ page: "runs", experimentId: "abc" }), null);
  assert.strictEqual(mlfHash({ page: "run", runId: "zzz" }), null);
  assert.strictEqual(mlfHash({ page: "compare", runIds: [RUN_A] }), null);
  assert.strictEqual(mlfHash({ page: "unknown" }), null);
}

function testQueryStringOrder() {
  // 順序は MLF_QUERY_KEYS に固定する（往復の安定のため）
  assert.strictEqual(
    mlfQueryString({ status: "FINISHED", sprout: "120", sai_run: "121" }),
    "sai_run=121&sprout=120&status=FINISHED"
  );
  assert.strictEqual(mlfQueryString({}), "");
  // 知らない鍵と空の値は捨てられる
  assert.strictEqual(mlfQueryString({ unknown: "x", sprout: "" }), "");
  assert.deepStrictEqual(mlfParseQuery("sai_run=1&unknown=2&sprout="), { sai_run: "1" });
}

// ============================================================================
// AC2: 絞り込みの検索式
// ============================================================================

function testFilterTwoKeys() {
  assert.strictEqual(
    mlfFilterFromQuery({ sai_run: "121", study: 'a"b' }),
    'tags."sai.run_id" = "121" AND tags."exp.study" = "a\\"b"'
  );
}

function testFilterAllKeys() {
  assert.strictEqual(
    mlfFilterFromQuery({ sai_run: "121", sprout: "120", study: "N12-s120", status: "FINISHED" }),
    'tags."sai.run_id" = "121" AND tags."sai.sprout_run_id" = "120" ' +
      'AND tags."exp.study" = "N12-s120" AND attributes.status = "FINISHED"'
  );
}

function testFilterBackslash() {
  // 逆向きの棒線も逃がす
  assert.strictEqual(mlfFilterFromQuery({ study: "a\\b" }), 'tags."exp.study" = "a\\\\b"');
}

function testFilterSproutAndBranch() {
  assert.strictEqual(mlfFilterFromQuery({ sprout: "120" }), 'tags."sai.sprout_run_id" = "120"');
  assert.strictEqual(
    mlfFilterFromQuery({ sai_run: "125", status: "FAILED" }),
    'tags."sai.run_id" = "125" AND attributes.status = "FAILED"'
  );
  // 旧い鍵 seed は読まない
  assert.deepStrictEqual(mlfParseQuery("seed=120"), {});
}

function testFilterEmpty() {
  assert.strictEqual(mlfFilterFromQuery({}), "");
  assert.strictEqual(mlfFilterFromQuery(null), "");
  assert.strictEqual(mlfFilterFromQuery({ sprout: "", status: "" }), "");
}

// ============================================================================
// AC3: 入れ子の木
// ============================================================================

/** sprout 2・branch 3・run 7（1 本は FAILED・1 本は sai.run_id 無し）を作る。 */
function treeSample() {
  const id = (n) => String(n).repeat(32).slice(0, 32);
  return [
    sampleRun({ id: id(1), name: "ID125-p02", sprout: 120, saiRun: 125, action: "tune", model: "cnn-static", parentBranch: 121, metric: "test_acc", value: 0.8, direction: "maximize" }),
    sampleRun({ id: id(2), name: "ID125-p01", sprout: 120, saiRun: 125, metric: "test_acc", value: 0.871, direction: "maximize" }),
    sampleRun({ id: id(3), name: "ID125-p03", sprout: 120, saiRun: 125, status: "FAILED" }),
    sampleRun({ id: id(4), name: "ID121-p01", sprout: 120, saiRun: 121, metric: "val_loss", value: 0.5, direction: "minimize" }),
    sampleRun({ id: id(5), name: "ID140-p01", sprout: 130, saiRun: 140, metric: "val_loss", value: 0.4, direction: "minimize" }),
    sampleRun({ id: id(6), name: "ID140-p02", sprout: 130, saiRun: 140, metric: "val_loss", value: 0.2, direction: "minimize" }),
    sampleRun({ id: id(7), name: "loose", sprout: 130 }),
  ];
}

function testBuildTreeShape() {
  const tree = mlfBuildTree(treeSample());
  assert.deepStrictEqual(tree.map((b) => b.label), ["sprout ID120", "sprout ID130"]);
  assert.strictEqual(tree[0].sprout, "120");
  // branch は番号の昇順
  assert.deepStrictEqual(tree[0].branches.map((b) => b.label), ["ID121", "ID125"]);
  const b125 = tree[0].branches[1];
  assert.strictEqual(b125.branch, "125");
  // run は run 名の昇順（FAILED も入る）
  assert.deepStrictEqual(b125.runs.map((r) => r.info.run_name), ["ID125-p01", "ID125-p02", "ID125-p03"]);
  // action・model・parentBranch は先頭の run のタグ（p01 には無い）
  assert.strictEqual(b125.action, "");
}

function testBuildTreeBest() {
  const tree = mlfBuildTree(treeSample());
  const b125 = tree[0].branches[1];
  assert.strictEqual(b125.best.value, 0.871, "maximize は大きい方");
  assert.strictEqual(b125.best.run.info.run_name, "ID125-p01");
  assert.strictEqual(tree[1].branches[0].best.value, 0.2, "minimize は小さい方");
  assert.strictEqual(tree[1].branches[1].best, null, "指標が無ければ null");
}

function testBuildTreeNoBranchBundle() {
  const tree = mlfBuildTree(treeSample());
  const last = tree[1].branches[tree[1].branches.length - 1];
  assert.strictEqual(last.label, "branch なし");
  assert.strictEqual(last.runs[0].info.run_name, "loose");
}

function testBuildTreeMeta() {
  const [one] = mlfBuildTree([
    sampleRun({ id: RUN_A, sprout: 1, saiRun: 5, name: "ID5-p01", action: "tune", model: "cnn-static", parentBranch: 4 }),
  ]);
  const br = one.branches[0];
  assert.strictEqual(br.action, "tune");
  assert.strictEqual(br.model, "cnn-static");
  assert.strictEqual(br.parentBranch, "4");
}

function testBuildTreeLegacySeed() {
  // 旧い記録: seed の束に入れ子を見ず平らに入る
  const tree = mlfBuildTree([
    sampleRun({ id: RUN_A, seed: 120, saiRun: 121, name: "a", metric: "acc", value: 0.8 }),
    sampleRun({ id: RUN_B, seed: 120, saiRun: 121, name: "b", parent: RUN_A }),
  ]);
  assert.strictEqual(tree.length, 1);
  assert.strictEqual(tree[0].label, "seed ID120（旧）");
  assert.deepStrictEqual(tree[0].branches[0].runs.map((r) => r.info.run_name), ["a", "b"]);
}

function testBuildTreeWithoutSproutTag() {
  const tree = mlfBuildTree([sampleRun({ id: RUN_A })]);
  assert.strictEqual(tree.length, 1);
  assert.strictEqual(tree[0].label, "sprout なし");
  assert.strictEqual(tree[0].branches[0].label, "branch なし");
}

function testBuildTreeEmpty() {
  assert.deepStrictEqual(mlfBuildTree([]), []);
  assert.deepStrictEqual(mlfBuildTree(null), []);
  assert.deepStrictEqual(mlfBuildTree([{ info: {} }]), []);
}

function testFlatRows() {
  const rows = mlfFlatRows(treeSample());
  assert.strictEqual(rows.length, 7, "7 本すべてが平らに出る");
  assert.ok(rows.every((r) => r.depth === 0));
}

// ============================================================================
// タグ・指標・日本語
// ============================================================================

function testTagHelpers() {
  const run = sampleRun({ id: RUN_A, sprout: 120, parent: RUN_B, role: "script", saiRun: 121 });
  assert.strictEqual(mlfSproutId(run), "120");
  assert.strictEqual(mlfParentId(run), RUN_B);
  assert.strictEqual(mlfTag(run, "exp.role"), "script");
  assert.strictEqual(mlfTag(run, "sai.run_id"), "121");
  assert.strictEqual(mlfTag(run, "sci.node"), "", "無いタグは空文字列");
  assert.strictEqual(mlfTag(null, "exp.role"), "");
}

function testMetricHelpers() {
  const run = sampleRun({ id: RUN_A, metric: "acc", value: 0.8, otherMetric: "loss", otherValue: 0.2 });
  assert.strictEqual(mlfMetricValue(run, "acc"), 0.8);
  assert.strictEqual(mlfMetricValue(run, "nope"), null);
  assert.deepStrictEqual(mlfMetricList(run).map((m) => m.key), ["acc", "loss"], "名前の順");
  // `exp.metric` のタグがあるときはその指標を出す
  assert.deepStrictEqual(mlfHeadMetric(run), { key: "acc", value: 0.8 });
  // タグが無いときは最初の指標（名前の順）
  const noTag = sampleRun({ id: RUN_B });
  noTag.data.metrics = [{ key: "zzz", value: 1 }, { key: "aaa", value: 2 }];
  assert.deepStrictEqual(mlfHeadMetric(noTag), { key: "aaa", value: 2 });
  // 指標が 1 つも無いときは null
  assert.strictEqual(mlfHeadMetric(sampleRun({ id: RUN_C })), null);
}

function testOrderOptions() {
  const runs = treeSample();
  const options = mlfOrderOptions(runs);
  assert.deepStrictEqual(options[0], ["attributes.start_time DESC", "開始（新しい順）"]);
  assert.deepStrictEqual(options[1], ["attributes.start_time ASC", "開始（古い順）"]);
  // run の `exp.metric` の名前が並ぶ
  assert.deepStrictEqual(options.slice(2), [
    ["metrics.test_acc DESC", "指標 test_acc（大きい順）"],
    ["metrics.test_acc ASC", "指標 test_acc（小さい順）"],
    ["metrics.val_loss DESC", "指標 val_loss（大きい順）"],
    ["metrics.val_loss ASC", "指標 val_loss（小さい順）"],
  ]);
  assert.strictEqual(mlfIsMetricOrder("metrics.acc DESC"), true);
  assert.strictEqual(mlfIsMetricOrder("attributes.start_time DESC"), false);
}

function testStatusText() {
  assert.strictEqual(mlfStatusText("FINISHED"), "完了");
  assert.strictEqual(mlfStatusText("FAILED"), "失敗");
  assert.strictEqual(mlfStatusText("RUNNING"), "実行中");
  assert.strictEqual(mlfStatusText("KILLED"), "中止");
  assert.strictEqual(mlfStatusText("SCHEDULED"), "予定");
  assert.strictEqual(mlfStatusText(""), "—");
}

function testDurationText() {
  assert.strictEqual(mlfDurationText(500), "1 秒未満");
  assert.strictEqual(mlfDurationText(42000), "42 秒");
  assert.strictEqual(mlfDurationText(123000), "2 分 3 秒");
  assert.strictEqual(mlfDurationText(120000), "2 分");
  assert.strictEqual(mlfDurationText(3600000), "1 時間");
  assert.strictEqual(mlfDurationText(3720000), "1 時間 2 分");
  assert.strictEqual(mlfDurationText(NaN), "—");
  assert.strictEqual(mlfDurationMs(sampleRun({ id: RUN_A })), 60000);
  assert.ok(Number.isNaN(mlfDurationMs({ info: { start_time: "2026-09-29T10:00:00Z" } })));
  // 本物の MLflow はミリ秒の数値で返す
  assert.strictEqual(mlfDurationMs({ info: { start_time: 1790666918697, end_time: 1790666932882 } }), 14185);
}

function testValueText() {
  assert.strictEqual(mlfValueText(0.8123456), "0.8123");
  assert.strictEqual(mlfValueText(3), "3");
  assert.strictEqual(mlfValueText(null), "—");
  assert.strictEqual(mlfValueText("abc"), "abc");
}

function testArtifactKind() {
  assert.strictEqual(mlfArtifactKind("note.md"), "markdown");
  assert.strictEqual(mlfArtifactKind("agent-check.txt"), "text");
  assert.strictEqual(mlfArtifactKind("out.log"), "text");
  assert.strictEqual(mlfArtifactKind("table.csv"), "table");
  assert.strictEqual(mlfArtifactKind("table.tsv"), "table");
  assert.strictEqual(mlfArtifactKind("graph.png"), "image");
  assert.strictEqual(mlfArtifactKind("graph.SVG"), "image");
  assert.strictEqual(mlfArtifactKind("model.pkl"), "unknown");
  assert.strictEqual(mlfArtifactKind("README"), "unknown");
}

function testProjectChoices() {
  // 先頭は「すべてのプロジェクト」
  const list = mlfProjectChoices({ projects: [{ name: "west" }, { name: "east" }] });
  assert.deepStrictEqual(list.map((x) => x.name), ["", "west", "east"]);
  assert.strictEqual(list[0].label, "すべてのプロジェクト");
  // 絞り込みの鍵の順番は往復の安定のために決めてある
}

function testQueryKeysAreFixed() {
  // 絞り込みの鍵の順番は往復の安定のために決めてある
  assert.deepStrictEqual(MLF_QUERY_KEYS, ["sai_run", "sprout", "study", "status"]);
}

// --- U07: 指標のグラフと比較の表 -------------------------------------

/** 比較用の run を作る。{id, name, params, metrics, tags} */
function cmpRun(o) {
  return {
    info: { run_id: o.id, run_name: o.name || `run ${o.id.slice(0, 4)}`, status: "FINISHED" },
    data: {
      params: (o.params || []).map(([key, value]) => ({ key, value })),
      metrics: (o.metrics || []).map(([key, value]) => ({ key, value, step: 0, timestamp: 1 })),
      tags: (o.tags || []).map(([key, value]) => ({ key, value })),
    },
  };
}

// AC3: 3 本で param `lr` が同じ・`depth` が違うとき、差分だけなら `depth` の行を返す
function testCompareRowsDiffOnly() {
  const runs = [
    cmpRun({ id: RUN_A, params: [["lr", "0.01"], ["depth", "2"]] }),
    cmpRun({ id: RUN_B, params: [["lr", "0.01"], ["depth", "4"]] }),
    cmpRun({ id: RUN_C, params: [["lr", "0.01"], ["depth", "8"]] }),
  ];
  const rows = mlfCompareRows(runs, true);
  assert.deepStrictEqual(rows.map((r) => r.key), ["depth"]);
  assert.strictEqual(rows[0].section, "param");
  assert.deepStrictEqual(rows[0].values, ["2", "4", "8"]);
  assert.strictEqual(rows[0].same, false);

  // 差分を隠さないときは全部の行が並ぶ（lr は同じなので same）
  const all = mlfCompareRows(runs, false);
  assert.deepStrictEqual(all.map((r) => r.key), ["depth", "lr"]);
  assert.strictEqual(all[1].same, true);
}

function testCompareRowsAllSame() {
  // 3 本すべて同じなら「差分だけ」では 0 行
  const runs = [
    cmpRun({ id: RUN_A, params: [["lr", "0.01"]] }),
    cmpRun({ id: RUN_B, params: [["lr", "0.01"]] }),
    cmpRun({ id: RUN_C, params: [["lr", "0.01"]] }),
  ];
  assert.deepStrictEqual(mlfCompareRows(runs, true), []);
  assert.strictEqual(mlfCompareRows(runs, false).length, 1);
}

function testCompareRowsSections() {
  // 設定・指標・タグの 3 つの節が順に並ぶ
  const runs = [
    cmpRun({
      id: RUN_A,
      params: [["lr", "0.01"]],
      metrics: [["loss", 0.5]],
      tags: [["sai.project", "west"], ["mlflow.runName", "a"]],
    }),
    cmpRun({
      id: RUN_B,
      params: [["lr", "0.02"]],
      metrics: [["loss", 0.4]],
      tags: [["sai.project", "west"], ["mlflow.runName", "b"]],
    }),
  ];
  const rows = mlfCompareRows(runs, false);
  assert.deepStrictEqual(rows.map((r) => r.section), ["param", "metric", "tag", "tag"]);
  assert.deepStrictEqual(rows.map((r) => r.key), ["lr", "loss", "mlflow.runName", "sai.project"]);
  // 指標は最後の値
  assert.deepStrictEqual(rows[1].values, ["0.5", "0.4"]);
  // sai.project だけは同じなので「差分だけ」では消える
  assert.deepStrictEqual(mlfCompareRows(runs, true).map((r) => r.key), ["lr", "loss", "mlflow.runName"]);
  assert.strictEqual(mlfCompareSectionTitle("param"), "設定（param）");
  assert.strictEqual(mlfCompareSectionTitle("metric"), "指標（metric・最後の値）");
  assert.strictEqual(mlfCompareSectionTitle("tag"), "タグ");
}

function testCompareRowsMissingValue() {
  // 無い値は — になる
  const runs = [
    cmpRun({ id: RUN_A, params: [["lr", "0.01"], ["depth", "2"]], metrics: [["loss", 1]] }),
    cmpRun({ id: RUN_B, params: [["lr", "0.01"]], metrics: [] }),
  ];
  const rows = mlfCompareRows(runs, true);
  const depth = rows.find((r) => r.key === "depth");
  assert.deepStrictEqual(depth.values, ["2", "—"]);
  const loss = rows.find((r) => r.key === "loss");
  assert.deepStrictEqual(loss.values, ["1", "—"]);
}

function testCompareRowsEmpty() {
  assert.deepStrictEqual(mlfCompareRows([], true), []);
  assert.deepStrictEqual(mlfCompareRows(null, false), []);
  // run として読めないものは外す
  assert.deepStrictEqual(mlfCompareRows([{ info: {} }], false), []);
}

function testCompareHeadName() {
  assert.strictEqual(mlfCompareHeadName(cmpRun({ id: RUN_A, name: "試し 1" })), "試し 1");
  // 名前が無ければ run_id の先頭 8 字
  assert.strictEqual(mlfCompareHeadName({ info: { run_id: RUN_A } }), "aaaaaaaa");
  assert.strictEqual(mlfCompareHeadName(null), "—");
}

function testMetricKeys() {
  // 名前順・重複無し
  const run = cmpRun({ id: RUN_A, metrics: [["loss", 1], ["acc", 0.9], ["loss", 0.8]] });
  assert.deepStrictEqual(mlfMetricKeys(run), ["acc", "loss"]);
  // 和集合
  const keys = mlfMetricKeysOf([
    run,
    cmpRun({ id: RUN_B, metrics: [["acc", 0.7], ["f1", 0.5]] }),
  ]);
  assert.deepStrictEqual(keys, ["acc", "f1", "loss"]);
  assert.deepStrictEqual(mlfMetricKeysOf([]), []);
  assert.deepStrictEqual(mlfMetricKeys(null), []);
}

function testCompareLimits() {
  // 比較は 2〜20 件（C-07）。画面と pure 関数の両方で同じ数を使う
  assert.strictEqual(MLF_COMPARE_MIN, 2);
  assert.strictEqual(MLF_COMPARE_MAX, 20);
  // 一括取得は 100 件まで（MLflow の 1 回の返答の上限）
  assert.strictEqual(MLF_BULK_RUNS, 100);
}

// --- 実行 ------------------------------------------------------------

const tests = [
  testParseExperiments,
  testParseRuns,
  testParseRunsQuery,
  testParseRunDetail,
  testParseCompare,
  testUnknownIsNull,
  testRoundTrip,
  testHashRejectsBadState,
  testQueryStringOrder,
  testFilterTwoKeys,
  testFilterAllKeys,
  testFilterBackslash,
  testFilterEmpty,
  testBuildTreeShape,
  testBuildTreeBest,
  testBuildTreeNoBranchBundle,
  testBuildTreeMeta,
  testBuildTreeLegacySeed,
  testBuildTreeWithoutSproutTag,
  testFilterSproutAndBranch,
  testBuildTreeEmpty,
  testFlatRows,
  testTagHelpers,
  testMetricHelpers,
  testOrderOptions,
  testStatusText,
  testDurationText,
  testValueText,
  testArtifactKind,
  testProjectChoices,
  testQueryKeysAreFixed,
  testCompareRowsDiffOnly,
  testCompareRowsAllSame,
  testCompareRowsSections,
  testCompareRowsMissingValue,
  testCompareRowsEmpty,
  testCompareHeadName,
  testMetricKeys,
  testCompareLimits,
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
  console.error(`test_mlflow: ${failed} 件が失敗しました`);
  process.exit(1);
}
console.log(`test_mlflow: ${tests.length} 件すべて通りました`);
