"use strict";
// メッセージ画面の MCP の道具の色（U12）を決める小さな関数 `isMcpTool` のテスト。
//
// app.js は ブラウザ専用（読み込んだだけで document に触る）ので node からは
// require できない。そのため関数定義の行だけを取り出して評価する
// （取り出せなければテストが失敗する＝関数が見つからないのは異常）。
// 実行:
//     node tests/test_mcp_tool.js

const assert = require("assert");
const fs = require("fs");
const path = require("path");

const src = fs.readFileSync(
  path.join(__dirname, "..", "src", "agent_link_web", "static", "app.js"),
  "utf8"
);

const m = src.match(/^\/\/ MCP の道具かどうか[\s\S]*?^function isMcpTool\(item\) \{[\s\S]*?^\}/m);
assert.ok(m, "app.js に isMcpTool が見つかりません（U12）");
// eslint-disable-next-line no-new-func
const isMcpTool = new Function(`${m[0]}; return isMcpTool;`)();

// pi の adapter の道具（mcp）と MCP 名前空間（mcp__donsetch__web_search など）
assert.strictEqual(isMcpTool({ tool: "mcp" }), true);
assert.strictEqual(isMcpTool({ tool: "mcp__donsetch__web_search" }), true);
assert.strictEqual(isMcpTool({ tool: "mcp__other__tool" }), true);
assert.strictEqual(isMcpTool({ tool: "mcp.something" }), true);

// sai の CLI を Bash で呼んだ項目（item.cli が入る・U17）
assert.strictEqual(isMcpTool({ tool: "Bash", cli: "agent-link" }), true);
assert.strictEqual(isMcpTool({ tool: "Bash" }), false);

// ふつうの道具
assert.strictEqual(isMcpTool({ tool: "bash" }), false);
assert.strictEqual(isMcpTool({ tool: "edit" }), false);
assert.strictEqual(isMcpTool({ tool: "mcpx" }), false);
assert.strictEqual(isMcpTool({ tool: "" }), false);
assert.strictEqual(isMcpTool({}), false);
assert.strictEqual(isMcpTool(null), false);

console.log("test_mcp_tool: すべて通りました");
