"use strict";
// 画面ファイル（static/*.js）の一番外側の関数・変数の名前が重なっていないかの検査。
// 画面ファイルは <script> で順に読むため、同じ名前があると後から読んだ方で上書きされる
// （papers.js の recentRow が project.js の「直近の実行」を「(検索)」に変えていた不具合）。
// 実行:
//     node tests/test_globals.js

const assert = require("assert");
const fs = require("fs");
const path = require("path");

const dir = path.join(__dirname, "..", "src", "agent_link_web", "static");
const owner = new Map(); // 名前 → 最初に見つけたファイル
const dups = [];
for (const file of fs.readdirSync(dir).filter((f) => f.endsWith(".js") && f !== "sw.js")) {
  const text = fs.readFileSync(path.join(dir, file), "utf8");
  const re = /^(?:async\s+)?function\s+([A-Za-z0-9_$]+)|^(?:const|let|var)\s+([A-Za-z0-9_$]+)/gm;
  const seen = new Set(); // 同じファイルの中の重なりは JS 自身がエラーにするので数えない
  for (const m of text.matchAll(re)) {
    const name = m[1] || m[2];
    if (seen.has(name)) continue;
    seen.add(name);
    if (owner.has(name)) dups.push(`${name}: ${owner.get(name)} と ${file}`);
    else owner.set(name, file);
  }
}
assert.deepStrictEqual(dups, [], `一番外側の名前が重なっています:\n${dups.join("\n")}`);
console.log("test_globals: すべて通りました");
