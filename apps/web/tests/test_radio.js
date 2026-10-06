"use strict";
// Radio の DOM を使わない関数（formatSec / 音声の名前の分け方・プロジェクトの絞り込み）の小さなテスト
//（assert だけ）・09 §5・18 §6。
// 実行:
//     node tests/test_radio.js

const assert = require("assert");
const path = require("path");

const {
  formatSec,
  sectionAt,
  isRadioPaperId,
  splitRadioName,
  radioProjectOf,
  radioInProject,
} = require(path.join(
  __dirname,
  "..",
  "src",
  "sai_web",
  "static",
  "radio.js"
));

function testFormatSec() {
  // 09 §4.11 の表示例
  assert.strictEqual(formatSec(0), "0:00");
  assert.strictEqual(formatSec(754), "12:34");   // 12:34
  assert.strictEqual(formatSec(3723), "1:02:03"); // 1 時間以上は "1:02:03"
  assert.strictEqual(formatSec(59 * 60 + 9), "59:09");
  assert.strictEqual(formatSec(3600), "1:00:00");
  // 壊れた値・負の値でも崩れない
  assert.strictEqual(formatSec(-5), "0:00");
  assert.strictEqual(formatSec(null), "0:00");
  assert.strictEqual(formatSec(undefined), "0:00");
  assert.strictEqual(formatSec("abc"), "0:00");
  assert.strictEqual(formatSec(90.9), "1:30"); // 小数は切り捨て
}

function testRadioInProject() {
  // 選んだプロジェクト（root のフォルダ名）の音声だけ。古い音声（16進・番号だけ）は入れない
  assert.strictEqual(radioInProject("sai-paper-30", "sai-paper"), true);
  assert.strictEqual(radioInProject("my-proj-2-5", "my-proj-2"), true);
  assert.strictEqual(radioInProject("my-proj-2-5", "my-proj"), false);
  assert.strictEqual(radioInProject("other-1", "sai-paper"), false);
  assert.strictEqual(radioInProject("4ee6474932af", "sai-paper"), false);
  assert.strictEqual(radioInProject("30", "sai-paper"), false);
  assert.strictEqual(radioInProject("sai-paper-30", null), false); // プロジェクト未選択
  assert.strictEqual(radioInProject(null, "sai-paper"), false);
}

function testSectionAt() {
  // 章の境目ちょうど・最後の章・範囲外（10 §5）
  const sections = [
    { start: 0, end: 75.5, title: "序論" },
    { start: 75.5, end: 194, title: "手法" },
    { start: 194, end: null, title: "実験" },  // end が無い章は最後まで
  ];
  // 最初の章
  assert.strictEqual(sectionAt(sections, 0), sections[0]);
  assert.strictEqual(sectionAt(sections, 75.4), sections[0]);
  // 境目ちょうどは次の章（start ≦ sec < end）
  assert.strictEqual(sectionAt(sections, 75.5), sections[1]);
  assert.strictEqual(sectionAt(sections, 194), sections[2]);
  // 最後の章（end が null）はいくらでも先まで
  assert.strictEqual(sectionAt(sections, 100000), sections[2]);
  // 範囲外（最初の章より前）・壊れた値
  assert.strictEqual(sectionAt(sections, -1), null);
  assert.strictEqual(sectionAt(sections, "abc"), null);
  assert.strictEqual(sectionAt(sections, null), null);
  assert.strictEqual(sectionAt([], 10), null);
  assert.strictEqual(sectionAt(null, 10), null);
  // 章が全部終わっている（範囲外）
  const closed = [{ start: 0, end: 10, title: "短い" }];
  assert.strictEqual(sectionAt(closed, 10), null);
  assert.strictEqual(sectionAt(closed, 9.99), closed[0]);
}

function testIsRadioPaperId() {
  // sai-radio の論文 ID（16進の英数字）は通す（10 の直し）
  assert.strictEqual(isRadioPaperId("4ee6474932af"), true);
  assert.strictEqual(isRadioPaperId("abc-123_X"), true);
  // 音声の名前（フォルダ名に . が入ることがある）も通す（18 §3.3）
  assert.strictEqual(isRadioPaperId("sai-paper-30"), true);
  assert.strictEqual(isRadioPaperId("my.proj-30"), true);
  // 空と Path の混入，英数字のない . だけの ID は通さない
  assert.strictEqual(isRadioPaperId(""), false);
  assert.strictEqual(isRadioPaperId("../x"), false);
  assert.strictEqual(isRadioPaperId("a/b"), false);
  assert.strictEqual(isRadioPaperId(".."), false);
  assert.strictEqual(isRadioPaperId("."), false);
  assert.strictEqual(isRadioPaperId(null), false);
  assert.strictEqual(isRadioPaperId(undefined), false);
}

// 音声の名前の分け方（<root のフォルダ名>-<論文番号>・計画 18 §3.3・§6）
function testSplitRadioName() {
  assert.deepStrictEqual(splitRadioName("sai-paper-30"), {
    root_name: "sai-paper",
    paper_id: "30",
  });
  // フォルダ名に - が複数あっても最後の -<数字> が論文番号
  assert.deepStrictEqual(splitRadioName("my-proj-2-5"), {
    root_name: "my-proj-2",
    paper_id: "5",
  });
  // 分けられないもの（16進の ID・番号だけ・空・末尾が数字でない）
  assert.strictEqual(splitRadioName("4ee6474932af"), null);
  assert.strictEqual(splitRadioName("30"), null);
  assert.strictEqual(splitRadioName(""), null);
  assert.strictEqual(splitRadioName(null), null);
  assert.strictEqual(splitRadioName(undefined), null);
  assert.strictEqual(splitRadioName("abc-"), null);
}

// 名札の引き方（root_name が一致するプロジェクト・計画 18 §1.2・§6）
function testRadioProjectOf() {
  const projects = [
    { name: "sai-paper", root_name: "sai-paper" },
    { name: "research", root_name: "my-proj-2" },
  ];
  assert.strictEqual(radioProjectOf(projects, "sai-paper-30"), projects[0]);
  assert.strictEqual(radioProjectOf(projects, "my-proj-2-5"), projects[1]);
  // 分けられない音声は名札なし（18 §1.2）
  assert.strictEqual(radioProjectOf(projects, "4ee6474932af"), null);
  assert.strictEqual(radioProjectOf(projects, "30"), null);
  // フォルダ名が一致しない音声は名札なし（18 §1.2）
  assert.strictEqual(radioProjectOf(projects, "other-1"), null);
  // 一覧が無いときも壊れない
  assert.strictEqual(radioProjectOf(null, "sai-paper-30"), null);
  assert.strictEqual(radioProjectOf(projects, null), null);
}

function main() {
  testFormatSec();
  testRadioInProject();
  testSectionAt();
  testIsRadioPaperId();
  testSplitRadioName();
  testRadioProjectOf();
  console.log("test_radio: すべて通りました");
}

main();
