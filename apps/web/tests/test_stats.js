"use strict";
// 統計画面の数字整形関数の小さなテスト（assert だけ）・04 §5．
// 実行:
//     node tests/test_stats.js

const assert = require("assert");
const path = require("path");

const {
  formatTokens,
  formatDuration,
  formatCount,
  toolBarRows,
  isEmptyStats,
} = require(path.join(__dirname, "..", "src", "agent_link_web", "static", "stats.js"));

function testFormatTokens() {
  // トークンは K / M で短く（04 §1-2 の2番 / §5 の例）
  assert.strictEqual(formatTokens(606000), "606K");
  assert.strictEqual(formatTokens(1234567), "1.2M");
  assert.strictEqual(formatTokens(999), "999");
  assert.strictEqual(formatTokens(1500), "1.5K");
  assert.strictEqual(formatTokens(1000), "1K");
  assert.strictEqual(formatTokens(13732856), "13.7M");
  assert.strictEqual(formatTokens(0), "0");
  assert.strictEqual(formatTokens(null), "0");
}

function testFormatDuration() {
  // 時間表示（04 §5 の例: 845000ms → 14分05秒）
  assert.strictEqual(formatDuration(845000), "14分05秒");
  assert.strictEqual(formatDuration(45000), "45秒");
  assert.strictEqual(formatDuration(60000), "1分00秒");
  assert.strictEqual(formatDuration(3661000), "1時間1分01秒");
  assert.strictEqual(formatDuration(0), "0秒");
  assert.strictEqual(formatDuration(null), "0秒");
}

function testFormatCount() {
  // 凡例では正確な値を桁区切りで（04 §1-2）
  assert.strictEqual(formatCount(606000), "606,000");
  assert.strictEqual(formatCount(0), "0");
}

function testToolBarRows() {
  // 多い順，上位10個まで，残りは「その他」（04 §1-2 の4番）
  const many = {};
  for (let i = 0; i < 12; i++) many[`Tool${String(i).padStart(2, "0")}`] = 20 - i;
  const rows = toolBarRows({ tool_calls_by_name: many });
  assert.strictEqual(rows.length, 11); // 上位10 + その他
  assert.strictEqual(rows[0][0], "Tool00");
  assert.strictEqual(rows[9][0], "Tool09");
  assert.strictEqual(rows[10][0], "その他");
  // その他 = Tool10(10) + Tool11(9)
  assert.strictEqual(rows[10][1], 19);

  // 0 回のツールは出さない
  const few = toolBarRows({ tool_calls_by_name: { Bash: 52, Read: 0 } });
  assert.deepStrictEqual(few, [["Bash", 52]]);
}

function testIsEmptyStats() {
  // stats はあるが全部 0 → 「統計データがありません」（04 §1-4）
  const zero = {
    cost_usd: { input: 0, output: 0, cache_read: 0, cache_write: 0, total: 0 },
    tokens: { input: 0, output: 0, cache_read: 0, cache_write: 0, total: 0 },
    elapsed_ms: 1000,
    turn_count: 0,
    tool_call_count: 0,
    tool_calls_by_name: {},
  };
  assert.strictEqual(isEmptyStats(zero), true);
  assert.strictEqual(isEmptyStats(null), true);
  // 数字が1つでも入っていれば表示する
  const real = { ...zero, tool_call_count: 3 };
  assert.strictEqual(isEmptyStats(real), false);
}

function main() {
  testFormatTokens();
  testFormatDuration();
  testFormatCount();
  testToolBarRows();
  testIsEmptyStats();
  console.log("test_stats: すべて通りました");
}

main();
