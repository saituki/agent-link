// sai-web のアカウント画面（#/account）。中身は Usage の区画だけ。
//
// 画面の作り方の約束（AGENTS.md「画面のコード」）:
//   - 登録は window.SaiRoutes（app.js に画面ごとの分岐を足さない）
//   - ui.js の部品（saiEl など）を使う
//   - 一番外側の名前には頭 ac / AC を付ける（tests/test_globals.js が重なりを見る）
//
// 区画: #/account は acSections の関数を順に呼んで描く。
//   1 つ 1 つは (root: HTMLElement) => void。Usage は acAccountOpen の最後に足す（usage.js）。
"use strict";

/** 見出しの帯（他の画面と同じ .appbar）。 */
function acAppbar(title) {
  const bar = saiEl("header", "appbar");
  bar.appendChild(saiEl("h1", null, title));
  return bar;
}

/** #/account の区画（順に呼ぶ。Usage は acAccountOpen の最後に足す）。 */
const acSections = [];

function acAccountOpen() {
  const root = document.getElementById("account-root");
  if (!root) return;
  root.innerHTML = "";
  root.appendChild(acAppbar("Account"));
  const body = saiEl("div", "ac-body");
  for (const fn of acSections) {
    try {
      fn(body);
    } catch { /* 1 つの区画が失敗しても他は出す */ }
  }
  root.appendChild(body);
  // Usage の区画。usageOpen が #usage-list を探すので、body を root に付けてから描く
  if (typeof usageMountInto === "function") {
    try {
      usageMountInto(body);
    } catch { /* 使用量が出せなくても他の区画は出す */ }
  }
}

// --- 画面の登録（ブラウザでのみ） ---

if (typeof window !== "undefined") {
  (window.SaiRoutes = window.SaiRoutes || []).push({
    match(hash) {
      return hash === "#/account" ? {} : null;
    },
    view: "view-account",
    tab: "account",
    hideTabbar: false,
    interval: 60000, // Usage を 60 秒ごとに読み直す（usage.js の一覧と同じ間隔・42 §3.5）
    open() {
      acAccountOpen();
    },
    poll() {
      if (typeof usageLoad === "function") return usageLoad();
    },
  });
}
