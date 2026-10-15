"use strict";
// サポート（U03）: 右下のアイコン・Customer の一覧（#/customer）。
// 種類（Report・Question・Opinion）を選ぶと Customer のチャットを作り、その run の画面へ移る。
// 一番外側の名前は cs / CS_ で始める（tests/test_globals.js）。
// DOM を使わない関数（csProjectFromHash）は末尾で node に渡す（tests/test_customer.js）。

// 起動待ちの見分け（app.js の NEWCHAT_RUN_KEY と同じ値）
const CS_RUN_KEY = "sai_web_newchat_run";
// 前回選んだプロジェクト（newchat.js の NC_PROJECT_KEY と同じ値）
const CS_PROJECT_KEY = "sai_web_newchat_project";
const CS_KINDS = [
  ["report", "Report", "バグを報告する"],
  ["question", "Question", "使い方を質問する"],
  ["opinion", "Opinion", "意見・要望を送る"],
];

// 住所からプロジェクト名を取る（#/run/west/42 → west・#/list/west?… → west）。無ければ ""
function csProjectFromHash(hash, names) {
  // 先頭の「#」と種類（run・list など）の次の部分がプロジェクト名
  const part = String(hash || "").split("?")[0].split("/")[2];
  if (!part) return "";
  let name = part;
  try { name = decodeURIComponent(part); } catch { /* 読めなければそのまま */ }
  return names.includes(name) ? name : "";
}

function csSavedProject() {
  try { return localStorage.getItem(CS_PROJECT_KEY) || ""; } catch { return ""; }
}

// チャットの画面では入力欄と「新着 ↓」の上に出す（CSS の body.cs-on-run）
function csSyncRunClass() {
  document.body.classList.toggle("cs-on-run", location.hash.startsWith("#/run/"));
}

// 種類を選ぶシート。選ぶと Customer のチャットを作って、その run の画面へ移る
async function csOpenSheet() {
  let names = [];
  try {
    const data = await saiApi("/api/projects");
    names = (data.projects || []).map((p) => p && p.name).filter(Boolean);
  } catch (e) {
    saiToast(e.message || "プロジェクトを読めませんでした", "error");
    return;
  }
  if (names.length === 0) {
    saiToast("プロジェクトがありません", "error");
    return;
  }
  // 既定: 今の住所のプロジェクト → 前回選んだ物 → 一覧の先頭
  const saved = csSavedProject();
  const project = csProjectFromHash(location.hash, names) || (names.includes(saved) ? saved : names[0]);

  const body = saiEl("div", "cs-body");
  const buttons = [];
  let select = null;
  const sheet = saiSheet({ title: "サポート", body });
  for (const [kind, name, desc] of CS_KINDS) {
    const b = document.createElement("button");
    b.type = "button";
    b.className = "cs-kind";
    b.append(saiEl("span", "cs-kind-name", name), saiEl("span", "cs-kind-desc", desc));
    b.addEventListener("click", () => void csStart(kind, select ? select.value : project, sheet, buttons));
    buttons.push(b);
    body.appendChild(b);
  }
  body.appendChild(saiEl("p", "cs-note", "あなたの Claude を使ってチャットします。"));
  // プロジェクトが 2 つ以上あるときだけ選べる
  if (names.length >= 2) {
    const row = saiEl("label", "cs-project-row", "プロジェクト");
    select = document.createElement("select");
    select.className = "cs-project";
    for (const n of names) {
      const opt = document.createElement("option");
      opt.value = n;
      opt.textContent = n;
      select.appendChild(opt);
    }
    select.value = project;
    row.appendChild(select);
    body.appendChild(row);
  }
}

async function csStart(kind, project, sheet, buttons) {
  // 二度押しを防ぐ（起動が終わるまで 3 つとも押せない）
  buttons.forEach((b) => { b.disabled = true; });
  try {
    const out = await saiApi(`/api/projects/${encodeURIComponent(project)}/customer`, {
      method: "POST",
      body: {
        kind,
        screen: location.hash || "#/",
        width: window.innerWidth,
        ua: navigator.userAgent.slice(0, 300),
      },
    });
    // 起動待ちの見分け（app.js が 60 秒まで「起動を待っています…」を出す）
    try {
      sessionStorage.setItem(CS_RUN_KEY, JSON.stringify({ run_id: out.run_id, at: Date.now() }));
    } catch { /* 保存できなくても動く */ }
    sheet.close();
    location.hash = `#/run/${encodeURIComponent(out.project)}/${encodeURIComponent(out.run_id)}`;
  } catch (e) {
    buttons.forEach((b) => { b.disabled = false; });
    saiToast(e.message || "始められませんでした", "error");
  }
}

// Customer の一覧（#/customer）。新しい順
async function csLoad() {
  if (location.hash !== "#/customer") return;
  const box = document.getElementById("customer-list");
  if (!box) return;
  let data;
  try {
    data = await saiApi("/api/customer");
  } catch {
    return; // 取れなければ前の一覧のまま（5 秒ごとに読み直すので、トーストは出さない）
  }
  if (location.hash !== "#/customer") return; // 読んでいるあいだに別の画面へ移った
  const runs = (data.runs || []).slice().sort((a, b) => b.run_id - a.run_id);
  box.innerHTML = "";
  if (runs.length === 0) {
    box.appendChild(
      saiEmpty({
        icon: "💬",
        title: "まだ Customer のチャットはありません",
        body: "右下のアイコンからバグの報告・質問・意見を送れます。",
      })
    );
    return;
  }
  for (const run of runs) box.appendChild(runRow(run, true));
}

if (typeof window !== "undefined") {
  (window.SaiRoutes = window.SaiRoutes || []).push({
    match(hash) {
      return hash === "#/customer" ? {} : null;
    },
    view: "view-customer",
    tab: "agent",
    hideTabbar: false,
    interval: 5000,
    async open() {
      await csLoad();
    },
    async poll() {
      await csLoad();
    },
  });
}

if (typeof document !== "undefined" && document.body) {
  // 右下のアイコン（どの画面にも出す）
  const fab = document.createElement("button");
  fab.id = "cs-fab";
  fab.className = "cs-fab";
  fab.type = "button";
  fab.setAttribute("aria-label", "サポート（バグ・質問・意見）");
  fab.title = "サポート（バグ・質問・意見）";
  fab.textContent = "?";
  fab.addEventListener("click", () => void csOpenSheet());
  document.body.appendChild(fab);
  csSyncRunClass();
  window.addEventListener("hashchange", csSyncRunClass);
  const back = document.getElementById("cs-back");
  if (back) back.addEventListener("click", () => { location.hash = "#/"; });
}

// --- node からの読み込み（tests/test_customer.js） ---
if (typeof module !== "undefined" && module.exports) {
  module.exports = { csProjectFromHash, CS_RUN_KEY };
}
