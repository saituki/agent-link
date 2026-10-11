// sidebar のタブ（U05）。TS.ui.addSidebarTab でタブを足し、#ts-tabs と #ts-panels に出す。
// 既定の順は 変数・イベント・アルゴリズム・スナップショット・患者・拡張。それ以外は拡張が足した順に後ろへ並ぶ。

const tsSidebarBuiltins = ["channels", "events", "algos", "snapshots", "patients", "ext"];
const tsSidebarPrefKey = "ts_tab";

// タブの並び。builtins の順を先にし、残りは足された順のまま（安定）
function tsTabOrder(tabs, builtins) {
  const rank = (id) => {
    const i = builtins.indexOf(id);
    return i < 0 ? builtins.length : i;
  };
  return tabs
    .map((t, i) => ({ t, i }))
    .sort((a, b) => rank(a.t.id) - rank(b.t.id) || a.i - b.i)
    .map((x) => x.t);
}

// 画面に出すタブの一覧（{id, title, render, btn, panel, rendered}）
const tsSidebar = { tabs: [], cur: null, picked: false };

function tsSidebarStyle() {
  if (document.getElementById("ts-style-sidebar.js")) return;
  const st = document.createElement("style");
  st.id = "ts-style-sidebar.js";
  st.textContent = `
#ts-tabs { display: flex; gap: 4px; flex-wrap: wrap; margin-bottom: 8px; }
#ts-tabs button { padding: 4px 10px; border: 1px solid var(--ts-border); background: var(--ts-panel); border-radius: 6px; cursor: pointer; }
#ts-tabs button[aria-selected="true"] { background: var(--ts-hover); border-color: var(--ts-accent); color: var(--ts-accent); font-weight: 600; }
#ts-panels > div[hidden] { display: none; }
`;
  document.head.appendChild(st);
}

function tsSidebarLayout() {
  const bar = document.getElementById("ts-tabs");
  if (!bar) return;
  for (const t of tsTabOrder(tsSidebar.tabs, tsSidebarBuiltins)) bar.appendChild(t.btn);
}

// save は利用者が押したときだけ true（自動で選んだものを覚えると、保存してある選択を上書きする）
function tsSidebarSelect(id, save = false) {
  const t = tsSidebar.tabs.find((x) => x.id === id);
  if (!t) return;
  tsSidebar.cur = id;
  for (const x of tsSidebar.tabs) {
    x.panel.hidden = x !== t;
    if (x.btn) x.btn.setAttribute("aria-selected", x === t ? "true" : "false");
  }
  if (!t.rendered) {
    t.rendered = true;
    try {
      t.render(t.panel);
    } catch (e) {
      t.panel.textContent = `タブ ${t.title} を出せません: ${e.message}`;
    }
  }
  if (save) {
    tsSidebar.picked = true;
    try {
      localStorage.setItem(tsSidebarPrefKey, id);
    } catch (e) {
      // 保存できなくても画面は動く
    }
  }
}

// TS.ui.addSidebarTab({id, title, render(el)}) -> dispose
function tsAddSidebarTab({ id, title, render }) {
  tsSidebarStyle();
  // 同じ id が残っていれば先に外す（読み直しで二重に出ないように）
  const wasCur = tsSidebar.cur === id;
  if (tsSidebar.tabs.some((x) => x.id === id)) tsRemoveSidebarTab(id);

  const btn = document.createElement("button");
  btn.type = "button";
  btn.textContent = title;
  btn.setAttribute("aria-selected", "false");
  btn.addEventListener("click", () => tsSidebarSelect(id, true));

  const panel = document.createElement("div");
  panel.hidden = true;
  const t = { id, title, render, btn, panel, rendered: false };
  tsSidebar.tabs.push(t);

  const bar = document.getElementById("ts-tabs");
  const holder = document.getElementById("ts-panels");
  if (bar) bar.appendChild(btn);
  if (holder) holder.appendChild(panel);
  tsSidebarLayout();

  // 利用者が押すまでは、保存してあるタブ（無ければ既定の順の最初）を開く。
  // 読み込みの順で選ばれないように、毎回この順で決める
  let pref = null;
  try {
    pref = localStorage.getItem(tsSidebarPrefKey);
  } catch (e) {
    pref = null;
  }
  if (!tsSidebar.picked) {
    const ids = tsTabOrder(tsSidebar.tabs, tsSidebarBuiltins).map((x) => x.id);
    tsSidebarSelect(pref && ids.includes(pref) ? pref : ids[0]);
  } else if (wasCur) {
    tsSidebarSelect(id);
  }

  return () => tsRemoveSidebarTab(id);
}

function tsRemoveSidebarTab(id) {
  const t = tsSidebar.tabs.find((x) => x.id === id);
  if (!t) return;
  tsSidebar.tabs = tsSidebar.tabs.filter((x) => x !== t);
  t.btn.remove();
  t.panel.remove();
  if (tsSidebar.cur === id) {
    tsSidebar.cur = null;
    const first = tsTabOrder(tsSidebar.tabs, tsSidebarBuiltins)[0];
    if (first) tsSidebarSelect(first.id);
  }
}

if (typeof window !== "undefined" && window.TS) {
  window.TS.ui.addSidebarTab = tsAddSidebarTab;
}

if (typeof module !== "undefined") {
  module.exports = { tsTabOrder, tsSidebarBuiltins };
}
