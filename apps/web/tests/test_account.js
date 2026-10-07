"use strict";
// ログイン・アカウント・管理の画面（static/account.js・U07）の DOM を使わない部分のテスト。
// 名前・メール・パスワードの検査（境界）を確かめる。
// 実行:
//     node tests/test_account.js

const assert = require("assert");
const path = require("path");

const A = require(path.join(__dirname, "..", "src", "sai_web", "static", "account.js"));

// --- 短い名前: ^[a-z][a-z0-9]{1,15}$ （2〜16 文字） ---
function testValidName() {
  // 長さの境目
  assert.strictEqual(A.acValidName("ab"), true, "2 文字はよい");
  assert.strictEqual(A.acValidName("a"), false, "1 文字は短い");
  assert.strictEqual(A.acValidName(""), false, "空は短い");
  assert.strictEqual(A.acValidName("a" + "1".repeat(15)), true, "16 文字はよい");
  assert.strictEqual(A.acValidName("a" + "1".repeat(16)), false, "17 文字は長い");
  // 先頭は英小文字だけ
  assert.strictEqual(A.acValidName("1a"), false, "数字で始まらない");
  assert.strictEqual(A.acValidName("Ab"), false, "大文字で始まらない");
  assert.strictEqual(A.acValidName("aB"), false, "大文字を含まない");
  // 使える文字は英小文字と数字だけ
  assert.strictEqual(A.acValidName("a-b"), false, "ハイフンは使わない");
  assert.strictEqual(A.acValidName("a_b"), false, "アンダースコアは使わない");
  assert.strictEqual(A.acValidName("a b"), false, "空白は使わない");
  assert.strictEqual(A.acValidName("saito2"), true, "英小文字と数字はよい");
  // 文字列以外
  assert.strictEqual(A.acValidName(null), false);
  assert.strictEqual(A.acValidName(undefined), false);
  assert.strictEqual(A.acValidName(12), false);
}

// --- メールアドレス: ^[^@\s]+@[^@\s]+\.[^@\s]+$ ---
function testValidEmail() {
  assert.strictEqual(A.acValidEmail("a@b.c"), true, "短い形もよい");
  assert.strictEqual(A.acValidEmail("saito@university.ac.jp"), true, "大学のメールはよい");
  assert.strictEqual(A.acValidEmail("saito+lab@university.ac.jp"), true, "＋付きもよい");
  assert.strictEqual(A.acValidEmail("@b.c"), false, "先頭の @ の前が空");
  assert.strictEqual(A.acValidEmail("a@b"), false, "ドットが無い");
  assert.strictEqual(A.acValidEmail("a@b."), false, "ドットの後が空");
  assert.strictEqual(A.acValidEmail("a b@c.d"), false, "空白を含む");
  assert.strictEqual(A.acValidEmail("a@b c.d"), false, "空白を含む");
  assert.strictEqual(A.acValidEmail("a@@b.c"), false, "@ が 2 つ");
  assert.strictEqual(A.acValidEmail(""), false, "空");
  assert.strictEqual(A.acValidEmail(null), false);
  assert.strictEqual(A.acValidEmail(undefined), false);
}

// --- パスワード: 8〜128 文字 ---
function testValidPassword() {
  assert.strictEqual(A.acValidPassword("1234567"), false, "7 文字は短い");
  assert.strictEqual(A.acValidPassword("12345678"), true, "8 文字はよい");
  assert.strictEqual(A.acValidPassword("x".repeat(128)), true, "128 文字はよい");
  assert.strictEqual(A.acValidPassword("x".repeat(129)), false, "129 文字は長い");
  assert.strictEqual(A.acValidPassword(""), false, "空");
  // 空白や記号もそのまま数える（長さだけを見る）
  assert.strictEqual(A.acValidPassword("a b c d "), true, "空白を含む 8 文字");
  // 文字列以外
  assert.strictEqual(A.acValidPassword(null), false);
  assert.strictEqual(A.acValidPassword(undefined), false);
  assert.strictEqual(A.acValidPassword(12345678), false, "数値は文字列ではない");
}

function main() {
  testValidName();
  testValidEmail();
  testValidPassword();
  console.log("test_account: すべて通りました");
}

main();
