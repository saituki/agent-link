"use strict";
// Web の改名（U17）で端末に残る物を新しい名前に移す処理のテスト。
//   - 保存キーの移し替え（ui.js の saiMigrateStorage）：sai_phone_* → agent_link_web_*
//   - 古いキャッシュの判定（sw.js の swIsOldCache）：sai-phone-* と古い sai-web-v*
//   - 画面の版（sw.js の CACHE と index.html の ?v=）がそろっていること
// 実行:
//     node tests/test_migrate.js

const assert = require("assert");
const fs = require("fs");
const path = require("path");

const staticDir = path.join(__dirname, "..", "src", "agent_link_web", "static");
const ui = require(path.join(staticDir, "ui.js"));

// localStorage / sessionStorage の代わりになる、最小の入れ物。
function fakeStorage(entries) {
  const map = new Map(Object.entries(entries));
  return {
    get length() {
      return map.size;
    },
    key(i) {
      return Array.from(map.keys())[i] ?? null;
    },
    getItem(k) {
      return map.has(k) ? map.get(k) : null;
    },
    setItem(k, v) {
      map.set(k, String(v));
    },
    removeItem(k) {
      map.delete(k);
    },
  };
}

// --- 1) 古いキーを新しいキーに写し、古いキーは消す
const s = fakeStorage({ sai_phone_token: "abc", sai_phone_tab_last: "1", other: "x" });
ui.saiMigrateStorage(s);
assert.strictEqual(s.getItem("agent_link_web_token"), "abc", "合言葉が移っていません");
assert.strictEqual(s.getItem("agent_link_web_tab_last"), "1", "タブの記憶が移っていません");
assert.strictEqual(s.getItem("sai_phone_token"), null, "古いキーが残っています");
assert.strictEqual(s.getItem("sai_phone_tab_last"), null, "古いキーが残っています");
assert.strictEqual(s.getItem("other"), "x", "別のキーを触っています");

// --- 2) 新しいキーが既にあれば上書きしない（今の値を残す）
const s2 = fakeStorage({ sai_phone_token: "old", agent_link_web_token: "new" });
ui.saiMigrateStorage(s2);
assert.strictEqual(s2.getItem("agent_link_web_token"), "new", "今の値を上書きしています");
assert.strictEqual(s2.getItem("sai_phone_token"), null, "古いキーが残っています");

// --- 3) 何も無いとき・storage が無いときも落ちない
ui.saiMigrateStorage(fakeStorage({}));
ui.saiMigrateStorage(null);

// --- 4) 古いキャッシュの判定（sw.js から関数だけ取り出す）
const swSrc = fs.readFileSync(path.join(staticDir, "sw.js"), "utf8");
const fn = swSrc.match(/^function swIsOldCache\(name, current\) \{[\s\S]*?^\}/m);
assert.ok(fn, "sw.js に swIsOldCache が見つかりません");
// eslint-disable-next-line no-new-func
const swIsOldCache = new Function(`${fn[0]}; return swIsOldCache;`)();
assert.strictEqual(swIsOldCache("sai-web-v132", "agent-link-web-v142"), true, "sai-moon の画面の倉庫");
assert.strictEqual(swIsOldCache("agent-link-web-v141", "agent-link-web-v142"), true, "古い画面の倉庫（1つ前）");
assert.strictEqual(swIsOldCache("agent-link-web-v142", "agent-link-web-v142"), false, "今の画面の倉庫");
assert.strictEqual(swIsOldCache("sai-phone-v84", "agent-link-web-v142"), true, "旧名の画面の倉庫");
assert.strictEqual(swIsOldCache("sai-phone-radio-audio", "agent-link-web-v142"), true, "旧名の音声の倉庫");
assert.strictEqual(swIsOldCache("agent-link-web-radio-audio", "agent-link-web-v142"), false, "音声の倉庫は消さない");
assert.strictEqual(swIsOldCache("agent-link-web-open", "agent-link-web-v142"), false, "通知の飛び先は消さない");

// --- 5) 画面の版がそろっている（CACHE と ?v=）
assert.match(swSrc, /const CACHE = "agent-link-web-v142";/, "CACHE が agent-link-web-v142 ではありません");
const html = fs.readFileSync(path.join(staticDir, "index.html"), "utf8");
assert.ok(html.includes("?v=142"), "index.html の ?v= が142 ではありません");
assert.ok(!/\?v=(?!142)\d+/.test(html), "index.html に 142 以外の ?v= が残っています");

// --- 6) sw.js の ASSETS の全部のファイルが static/ にある（欠けると install が失敗する）
const assetsLine = swSrc.match(/^const ASSETS = (\[.*\]);/m);
assert.ok(assetsLine, "sw.js に ASSETS が見つかりません");
const assets = JSON.parse(assetsLine[1]);
for (const a of assets) {
  if (a === "/") continue;
  assert.ok(fs.existsSync(path.join(staticDir, a.slice(1))), `ASSETS の ${a} が static/ にありません`);
}
// index.html が読む /xxx.js・/xxx.css も static/ にあり、ASSETS に入っている
for (const m of html.matchAll(/(?:src|href)="(\/[^"?]+\.(?:js|css))\?v=/g)) {
  assert.ok(fs.existsSync(path.join(staticDir, m[1].slice(1))), `index.html の ${m[1]} が static/ にありません`);
  assert.ok(assets.includes(m[1]), `index.html の ${m[1]} が ASSETS にありません`);
}

console.log("test_migrate: すべて通りました");
