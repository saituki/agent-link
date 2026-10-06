"use strict";
// Web の「コマンド」画面（U10）の DOM を使わない関数の小さなテスト（assert だけ）。
// 確かめること:
// - 引数 → 入力欄の種類（flag はチェック・enum は選択・int/float は数・multiple は複数行）
// - 入力欄の値 → 送る中身（数への変換・複数行・空は送らない・必須・path・stdin）
// - 命令の並びのまとめ（path の 1 語目）
// 実行:
//     node apps/phone/tests/test_commands.js

const assert = require("assert");
const path = require("path");

const cmd = require(path.join(
  __dirname,
  "..",
  "src",
  "sai_phone",
  "static",
  "commands.js"
));

function arg(name, kind, type, extra) {
  return Object.assign(
    { name, kind, type, required: false, multiple: false, values: [], web: true },
    extra || {}
  );
}

function testFieldType() {
  assert.deepStrictEqual(cmd.cmdFieldType(arg("x", "flag", "enum", { values: ["true", "false"] })), { kind: "check" });
  assert.deepStrictEqual(cmd.cmdFieldType(arg("x", "option", "enum", { values: ["a", "b"] })), { kind: "select", values: ["a", "b"] });
  assert.deepStrictEqual(cmd.cmdFieldType(arg("x", "option", "int")), { kind: "number" });
  assert.deepStrictEqual(cmd.cmdFieldType(arg("x", "option", "float")), { kind: "number" });
  assert.deepStrictEqual(cmd.cmdFieldType(arg("x", "positional", "string")), { kind: "text" });
  assert.deepStrictEqual(cmd.cmdFieldType(arg("x", "option", "string", { multiple: true })), { kind: "lines" });
  // 壊れた値でも落ちない
  assert.deepStrictEqual(cmd.cmdFieldType(null), { kind: "text" });
}

function testScalar() {
  assert.strictEqual(cmd.cmdScalar(arg("n", "option", "int"), " 12 "), 12);
  assert.strictEqual(cmd.cmdScalar(arg("n", "option", "int"), "-3"), -3);
  assert.strictEqual(cmd.cmdScalar(arg("n", "option", "float"), "1.5"), 1.5);
  assert.strictEqual(cmd.cmdScalar(arg("s", "option", "string"), "abc"), "abc");
  assert.throws(() => cmd.cmdScalar(arg("n", "option", "int"), "1.5"));
  assert.throws(() => cmd.cmdScalar(arg("n", "option", "float"), "abc"));
}

function testBuildRequest() {
  // 計画の例（paper set create）
  const command = {
    path: ["set", "create"],
    json: true,
    stdin: "none",
    args: [
      arg("name", "positional", "string", { required: true }),
      arg("query", "option", "string"),
      arg("file", "option", "path", { web: false }),
    ],
  };
  assert.deepStrictEqual(
    cmd.cmdBuildRequest("paper", command, { name: "demo", query: "attention" }, ""),
    { cli: "paper", path: ["set", "create"], args: { name: "demo", query: "attention" } }
  );
  // 空の入力は送らない
  assert.deepStrictEqual(
    cmd.cmdBuildRequest("paper", command, { name: "demo", query: "  " }, ""),
    { cli: "paper", path: ["set", "create"], args: { name: "demo" } }
  );
  // web: false の引数は入っていても送らない
  assert.deepStrictEqual(
    cmd.cmdBuildRequest("paper", command, { name: "demo", file: "x" }, ""),
    { cli: "paper", path: ["set", "create"], args: { name: "demo" } }
  );

  // 数・フラグ・複数行・enum
  const find = {
    path: ["find"],
    json: true,
    stdin: "none",
    args: [
      arg("query", "option", "string", { required: true }),
      arg("paper", "option", "int", { multiple: true }),
      arg("fts_expr", "option", "enum", { values: ["a", "b"] }),
      arg("force", "flag", "enum", { values: ["true", "false"] }),
    ],
  };
  const req = cmd.cmdBuildRequest("paper", find, {
    query: "x",
    paper: "1\n2\n\n3",
    fts_expr: "b",
    force: true,
  }, "");
  assert.deepStrictEqual(req.args, { query: "x", paper: [1, 2, 3], fts_expr: "b", force: true });
  // フラグは false でも送る（サーバーが真偽として確かめる）
  assert.strictEqual(cmd.cmdBuildRequest("paper", find, { query: "x", force: false }, "").args.force, false);
  // multiple で 1 つも無ければ送らない
  assert.strictEqual("paper" in cmd.cmdBuildRequest("paper", find, { query: "x" }, "").args, false);

  // stdin は空なら付けない、あれば付ける
  const add = { path: ["add"], json: true, stdin: "text", args: [arg("name", "positional", "string", { required: true })] };
  assert.deepStrictEqual(cmd.cmdBuildRequest("paper", add, { name: "x" }, ""), {
    cli: "paper", path: ["add"], args: { name: "x" },
  });
  assert.deepStrictEqual(cmd.cmdBuildRequest("paper", add, { name: "x" }, "本文"), {
    cli: "paper", path: ["add"], args: { name: "x" }, stdin: "本文",
  });

  assert.throws(() => cmd.cmdBuildRequest("paper", find, { query: "x", paper: "notnum" }, ""));
}

function testGroup() {
  const groups = cmd.cmdGroup([
    { path: ["set", "create"] },
    { path: ["set", "list"] },
    { path: ["find"] },
  ]);
  assert.deepStrictEqual(groups.map((g) => g.key), ["set", "find"]);
  assert.strictEqual(groups[0].items.length, 2);
  assert.deepStrictEqual(cmd.cmdGroup([]), []);
}

function testParseHash() {
  assert.deepStrictEqual(cmd.cmdParseHash("#/project/sai-moon/commands"), { name: "sai-moon" });
  assert.deepStrictEqual(cmd.cmdParseHash("#/project/a%20b/commands"), { name: "a b" });
  assert.strictEqual(cmd.cmdParseHash("#/project/sai-moon/list"), null);
  assert.strictEqual(cmd.cmdParseHash("#/project/sai-moon/commands/x"), null);
}

function main() {
  testFieldType();
  testScalar();
  testBuildRequest();
  testGroup();
  testParseHash();
  console.log("test_commands: すべて通りました");
}

main();
