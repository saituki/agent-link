"use strict";
// Web の改名（U17）で端末に残る物を新しい名前に移す処理のテスト。
//   - 保存キーの移し替え（ui.js の saiMigrateStorage）：sai_phone_* → sai_web_*
//   - 古いキャッシュの判定（sw.js の swIsOldCache）：sai-phone-* と古い sai-web-v*
//   - 画面の版（sw.js の CACHE と index.html の ?v=）がそろっていること
// 実行:
//     node tests/test_migrate.js

const assert = require("assert");
const fs = require("fs");
const path = require("path");

const staticDir = path.join(__dirname, "..", "src", "sai_web", "static");
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
assert.strictEqual(s.getItem("sai_web_token"), "abc", "合言葉が移っていません");
assert.strictEqual(s.getItem("sai_web_tab_last"), "1", "タブの記憶が移っていません");
assert.strictEqual(s.getItem("sai_phone_token"), null, "古いキーが残っています");
assert.strictEqual(s.getItem("sai_phone_tab_last"), null, "古いキーが残っています");
assert.strictEqual(s.getItem("other"), "x", "別のキーを触っています");

// --- 2) 新しいキーが既にあれば上書きしない（今の値を残す）
const s2 = fakeStorage({ sai_phone_token: "old", sai_web_token: "new" });
ui.saiMigrateStorage(s2);
assert.strictEqual(s2.getItem("sai_web_token"), "new", "今の値を上書きしています");
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
assert.strictEqual(swIsOldCache("sai-web-v85", "sai-web-v110"), true, "古い画面の倉庫");
assert.strictEqual(swIsOldCache("sai-web-v109", "sai-web-v110"), true, "古い画面の倉庫（1つ前）");
assert.strictEqual(swIsOldCache("sai-web-v110", "sai-web-v110"), false, "今の画面の倉庫");
assert.strictEqual(swIsOldCache("sai-phone-v84", "sai-web-v110"), true, "旧名の画面の倉庫");
assert.strictEqual(swIsOldCache("sai-phone-radio-audio", "sai-web-v110"), true, "旧名の音声の倉庫");
assert.strictEqual(swIsOldCache("sai-web-radio-audio", "sai-web-v110"), false, "音声の倉庫は消さない");
assert.strictEqual(swIsOldCache("sai-web-open", "sai-web-v110"), false, "通知の飛び先は消さない");

// --- 5) 画面の版がそろっている（CACHE と ?v=）
assert.match(swSrc, /const CACHE = "sai-web-v110";/, "CACHE が sai-web-v110 ではありません");
const html = fs.readFileSync(path.join(staticDir, "index.html"), "utf8");
assert.ok(html.includes("?v=110"), "index.html の ?v= が 110 ではありません");
assert.ok(!html.includes("?v=104"), "index.html に古い ?v=104 が残っています");

console.log("test_migrate: すべて通りました");
