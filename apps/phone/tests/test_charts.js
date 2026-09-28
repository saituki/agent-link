"use strict";
// グラフの目盛りと値のテスト（U04）。
// 画面は素の SVG を手描きするが、目盛りの刻み（ui.js の saiNiceScale）と
// 目盛りに書く文字（saiAxisValue）は DOM を使わない純関数に書いてある。
// ここはその 2 つを確かめる（DOM を使う saiDailyBars はブラウザの実測で確かめる）。
//
//   node apps/phone/tests/test_charts.js

const assert = require("assert");
const path = require("path");

const U = require(path.join(__dirname, "..", "src", "sai_phone", "static", "ui.js"));

// --- 目盛りの刻み ---

function testNiceScale() {
  // 12.34 を 3 本に切ると 0 / 5 / 10 / 15。棒は 15 の高さまで描く
  const a = U.saiNiceScale(12.34, 3);
  assert.deepStrictEqual(a.values, [0, 5, 10, 15]);
  assert.strictEqual(a.max, 15);
  assert.strictEqual(a.step, 5);

  // 100 はそのままで 0 / 50 / 100
  assert.deepStrictEqual(U.saiNiceScale(100, 3).values, [0, 50, 100]);

  // 端数は切り上げ（7 は 7.5 まで）
  assert.strictEqual(U.saiNiceScale(7, 3).max, 7.5);

  // 大きさは 1 の位まで（55 は 60 まで、2.5 は 3 まで）
  assert.strictEqual(U.saiNiceScale(55, 3).max, 60);
  assert.strictEqual(U.saiNiceScale(2.5, 3).max, 3);

  // 1 未満でも 0.1 のような細かい刻みにならない
  assert.deepStrictEqual(U.saiNiceScale(0.4, 3).values, [0, 0.2, 0.4]);

  // 金額は 1 cent 未満に切らない（全部 $0.00 になるのを防ぐ）
  const money = U.saiNiceScale(0.0004, 3, 0.01);
  assert.deepStrictEqual(money.values, [0, 0.01]);
  assert.ok(money.max >= 0.0004, "最大値はデータの最大以上");

  // token は 500K ずつなど
  assert.deepStrictEqual(U.saiNiceScale(1200000, 3).values, [0, 500000, 1000000, 1500000]);

  // 0 や壊れた値でも壊れない（目盛りは 0 と 1 の 2 本になる）
  for (const bad of [0, -5, NaN, null, undefined, "x"]) {
    const s = U.saiNiceScale(bad, 3);
    assert.strictEqual(s.values[0], 0, `0 のときの下端は 0（入力 ${bad}）`);
    assert.ok(s.max > 0, `上端は 0 より大きい（入力 ${bad}）`);
  }

  // 目盛りが常に 0 から始まり、最大データの以上で終わること
  for (const v of [0.3, 1, 9, 99, 999, 12345, 606000, 9876543]) {
    const s = U.saiNiceScale(v, 3);
    assert.strictEqual(s.values[0], 0);
    assert.ok(s.max >= v, `${s.max} は ${v} 以上`);
    for (let i = 1; i < s.values.length; i += 1) {
      assert.ok(
        Math.abs(s.values[i] - s.values[i - 1] - s.step) < 1e-6,
        "目盛りは同じ間隔"
      );
    }
  }
}

// --- 目盛りに書く文字 ---

function testAxisValue() {
  // お金は $
  assert.strictEqual(U.saiAxisValue(0, "money"), "$0.00");
  assert.strictEqual(U.saiAxisValue(0.01, "money"), "$0.01");
  assert.strictEqual(U.saiAxisValue(12.5, "money"), "$12.50");
  assert.strictEqual(U.saiAxisValue(1234, "money"), "$1,234");

  // token は K / M / B
  assert.strictEqual(U.saiAxisValue(0, "tokens"), "0");
  assert.strictEqual(U.saiAxisValue(999, "tokens"), "999");
  assert.strictEqual(U.saiAxisValue(1000, "tokens"), "1K");
  assert.strictEqual(U.saiAxisValue(1500, "tokens"), "1.5K");
  assert.strictEqual(U.saiAxisValue(606000, "tokens"), "606K");
  assert.strictEqual(U.saiAxisValue(1234567, "tokens"), "1.2M");
  assert.strictEqual(U.saiAxisValue(13732856, "tokens"), "13.7M");
  assert.strictEqual(U.saiAxisValue(2000000, "tokens"), "2M");

  // 回数は 4 桁を超えると K / M にする（目盛りに長すぎる文字を置かない）
  assert.strictEqual(U.saiAxisValue(999, "count"), "999");
  assert.strictEqual(U.saiAxisValue(1500, "count"), "1,500");
  assert.strictEqual(U.saiAxisValue(12345, "count"), "12.3K");

  // パーセント
  assert.strictEqual(U.saiAxisValue(0, "percent"), "0%");
  assert.strictEqual(U.saiAxisValue(42.4, "percent"), "42%");
  assert.strictEqual(U.saiAxisValue(100, "percent"), "100%");

  // 壊れた値は 0 の 0
  for (const bad of [NaN, null, undefined, "x"]) {
    assert.strictEqual(U.saiAxisValue(bad, "tokens"), "0", `入力 ${bad}`);
  }
}

// --- 日付のラベル ---

function testDayLabel() {
  assert.strictEqual(U.saiDayLabel("2026-09-20"), "9/20");
  assert.strictEqual(U.saiDayLabel("2026-01-05"), "1/5");
  // 読めないものは空文字（空文字のラベルは描かれない）
  assert.strictEqual(U.saiDayLabel(""), "");
  assert.strictEqual(U.saiDayLabel(null), "");
  assert.strictEqual(U.saiDayLabel("2026-09"), "");
}

function main() {
  testNiceScale();
  testAxisValue();
  testDayLabel();
  console.log("test_charts: すべて通りました");
}

main();
