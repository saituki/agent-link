// sai-web PWA本体．素のJS（フレームワークなし）．
//
// 画面は URL の # 部分で切り替える（02 §3.1，統計は 04）:
//   日付一覧   #/
//   日の実行一覧 #/day/2026-09-23
//   実行（メッセージ） #/run/sai-web/58
//   統計 #/run/sai-web/58/stats（中身は stats.js・04）
// 戻るボタンは「1つ上の階層のURL」へ移動する（履歴に頼らない．
// 通知から直接開いた場合も同じ動きになるため）。
"use strict";

/* 合言葉の保存場所．初回に ?token=… で開いてもらい，以降は自動で付ける．
   プライベートブラウズでは例外が出るので，読み書きは必ず失敗に備える． */
const TOKEN_KEY = "sai_web_token";

function loadToken() {
  try {
    const fromUrl = new URLSearchParams(location.search).get("token");
    if (fromUrl) {
      try { localStorage.setItem(TOKEN_KEY, fromUrl); } catch { /* 保存できなくても動く */ }
      // 住所から合言葉を消す（共有時に漏らさないため）
      history.replaceState(null, "", location.pathname + location.hash);
      return fromUrl;
    }
    return localStorage.getItem(TOKEN_KEY) || "";
  } catch {
    return "";
  }
}

let apiToken = loadToken();

async function apiFetch(path, options = {}) {
  const headers = { ...(options.headers || {}) };
  if (apiToken) headers["Authorization"] = `Bearer ${apiToken}`;
  let res = await fetch(path, { ...options, headers });
  if (res.status === 401) {
    // 多人数モードには合言葉が無い。今の住所を覚えてログイン画面へ移す（U07）。
    // 単独モードは今までどおり合言葉を聞く。
    if (window.SaiAuth && window.SaiAuth.mode === "multi") {
      if (typeof acRememberThenLogin === "function") acRememberThenLogin();
      return res;
    }
    const input = prompt("合言葉を入力してください");
    if (input) {
      apiToken = input;
      try { localStorage.setItem(TOKEN_KEY, input); } catch { /* 動けばよい */ }
      headers["Authorization"] = `Bearer ${input}`;
      res = await fetch(path, { ...options, headers });
    }
  }
  // 初回のパスワード変更前は変更画面へ移す（U07。detail はサーバの返しそのまま）。
  if (res.status === 403) {
    try {
      const text = await res.clone().text();
      if (text.includes("パスワードを変えてください") && location.hash !== "#/password") {
        location.hash = "#/password";
      }
    } catch { /* 読めなければそのまま返す */ }
  }
  return res;
}

// --- 通信の状態と再試行の間隔 ---
// 失敗したら間隔を伸ばしながら再試行する（2秒→4秒→8秒→最大30秒）．
// 成功したら元の間隔に戻す．
let offline = false;

function setOffline(value) {
  offline = value;
  // 画面ごとに帯の id が違ったが，クラスでまとめて出し入れする（05 §3.2-6）
  document.querySelectorAll(".offline-bar").forEach((el) => { el.hidden = !value; });
}

// --- 画面切り替え（1画面1ビュー） ---

let current = { name: "days" };   // 今見ている画面
let refreshTimer = null;          // 表示中画面の更新予約
let intervalMs = 5000;            // 現在の更新間隔（失敗時に伸びる）
let routeSeq = 0;                 // 画面切り替えの連番（古い route が後から終わっても無視する）

function parseRoute(hashArg) {
  const hash = hashArg || location.hash || "#/";
  // タブのファイルが登録した画面を先に判定する（05 §3.2-1）。合えばパラメータ，
  //合わなければ今までの判定へ進む。
  for (const def of window.SaiRoutes || []) {
    if (!def || typeof def.match !== "function") continue;
    const params = def.match(hash);
    if (params) return { name: "ext", def, params };
  }
  if (hash.startsWith("#/day/")) return { name: "day", date: decodeURIComponent(hash.slice(6)) };
  if (hash.startsWith("#/run/")) {
    const rest = hash.slice(6);
    const i = rest.indexOf("/");
    if (i > 0) {
      let idPart = rest.slice(i + 1);
      let stats = false;
      // 統計画面の住所（…/stats）は ID から分ける（中身は 04・03 §1-7）
      if (idPart.endsWith("/stats")) {
        stats = true;
        idPart = idPart.slice(0, -"/stats".length);
      }
      return {
        name: "run",
        project: decodeURIComponent(rest.slice(0, i)),
        id: decodeURIComponent(idPart),
        stats,
      };
    }
  }
  if (hash.startsWith("#/session/")) return { name: "legacy" }; // 旧画面の住所
  return { name: "days" };
}

function showView(id) {
  for (const v of document.querySelectorAll(".view")) v.hidden = v.id !== id;
  window.scrollTo(0, 0);
}

function baseInterval() {
  // 登録された画面は自分の間隔を持つ（05 §3.2-3）。0 は更新しない画面
  if (current.name === "ext") return current.def.interval || 0;
  // 更新間隔は一覧5秒・実行2秒（02 §3.1）．統計画面は10秒（04 §3）
  if (current.name === "run" && current.stats) return 10000;
  if (current.name === "run") {
    // 生成中（live が working）は 1 秒ごとに読む（U04 §2）
    return currentRun && currentRun.live === "working" ? 1000 : 2000;
  }
  return 5000;
}

function rearm() {
  if (refreshTimer) clearInterval(refreshTimer);
  refreshTimer = null;
  // 間隔 0 は定期更新をしない画面（05 §3.2-2）
  if (intervalMs <= 0) return;
  refreshTimer = setInterval(tick, intervalMs);
}

function tick() {
  if (current.name === "ext") {
    if (typeof current.def.poll === "function") void current.def.poll(current.params);
    return;
  }
  if (current.name === "days") void loadDays();
  else if (current.name === "day") void loadDay();
  else if (current.name === "run" && current.stats) void SaiStats.poll(); // 統計画面（04 §3）
  else if (current.name === "run") void pollRun();
}

function onOk() { intervalMs = baseInterval(); rearm(); }

function onFail() {
  intervalMs = Math.min(intervalMs * 2, 30000);
  rearm();
}

// タブのファイル（project.js など）が使う共通の入口（05 §3.2-5）。
// 通信に成功したら ok()，失敗したら fail() を呼ぶ。
window.SaiShell = {
  ok() { setOffline(false); onOk(); },
  fail() { setOffline(true); onFail(); },
};

async function route() {
  // 連番をとっておく。途中で別の route（location.replace や hashchange）が始まったら、
  // 古い route はタブの保存などをしない（U01 §5）
  const seq = ++routeSeq;
  // 住所なしで開かれたときはホームにする（# は今までどおり Agent の日付一覧・C-F1）
  if (!location.hash) {
    location.replace(HOME_URL);
    return;
  }
  const r = parseRoute();
  if (r.name === "legacy") {
    // 旧 #/session/… の住所は日付一覧へ（通知の古いURL対策）
    location.replace("#/");
    return;
  }
  current = r;
  if (refreshTimer) { clearInterval(refreshTimer); refreshTimer = null; }
  setOffline(false);
  intervalMs = baseInterval();
  if (r.name === "ext") {
    // タブのファイルが登録した画面（05 §3.2-2）
    showView(r.def.view);
    if (typeof r.def.open === "function") await r.def.open(r.params);
    if (r.def.interval > 0) rearm();
  } else if (r.name === "days") {
    showView("view-days");
    await loadDays();
    rearm();
  } else if (r.name === "day") {
    showView("view-day");
    // 狭い画面では右（その日の実行）だけを出す。広い画面は左の日付一覧も出す（U4 §4）
    saiSplitShow(document.getElementById("day-split"), "main");
    await loadDay();
    rearm();
  } else if (r.name === "run" && r.stats) {
    // 統計画面（04）．中身は stats.js にまとめてある
    showView("view-stats");
    await SaiStats.open(r);
    rearm();
  } else if (r.name === "run") {
    showView("view-run");
    await openRun(r);
    rearm();
  }
  // 読み込みのあいだに別の route が始まっていたら何もしない（古い route が
  // タブの「最後の住所」を上書きしないため・U01 §5）
  if (seq !== routeSeq) return;
  // タブバーの見た目を今の画面に合わせる（05 §3.2-7）
  updateTabbar();
  // 今の画面を知らせる（小さな再生バーの出し入れに使う・09 §4.1）
  window.dispatchEvent(new CustomEvent("sai-route", { detail: { hash: location.hash } }));
}

window.addEventListener("hashchange", route);

// --- タブバー（05 §3.3） ---

// 各タブの「最初の画面」（U4: 先頭にホームを足した。U06 で実験を足し，U14 で Research に替えた。U05 で Papers を Research に入れ，Usage を Account に入れた）
const TAB_FIRST_URL = {
  home: "#/home",
  project: "#/projects",
  agent: "#/",
  research: "#/papers/dashboard",
  account: "#/account",
};
// ホームの住所（app.js 自身が見る。home.js も使うので外に出す）
const HOME_URL = "#/home";
// 各タブで「最後に開いた URL」（タブごとに別々に覚える）
const TAB_LAST_KEY = "sai_web_tab_last";

function loadTabLast() {
  try {
    const raw = sessionStorage.getItem(TAB_LAST_KEY);
    const obj = raw ? JSON.parse(raw) : {};
    return obj && typeof obj === "object" ? obj : {};
  } catch { return {}; } // プライベートブラウズでも動くように
}

function saveTabLast(obj) {
  try { sessionStorage.setItem(TAB_LAST_KEY, JSON.stringify(obj)); }
  catch { /* 保存できなくても動く */ }
}

// 今の画面の所属タブ。既存の4画面（days / day / run / stats）はすべて Agent（05 §3.2-4）
function currentTab() {
  if (current.name === "ext") return current.def.tab || "agent";
  return "agent";
}

// 住所からその画面のタブを求める。タブの「最後の住所」をどこへ入れるかの判定に使う
// （表示中の current ではなく住所その物を見るので、どの route から呼ばれても正しい・U01 §5）
function routeTab(hash) {
  const r = parseRoute(hash);
  if (r.name === "ext") return r.def.tab || "agent";
  return "agent";
}

// 画面が広いか（900px 以上）。左の縦のタブ帯を出すかどうかに使う（U4 §1）
function isWide() {
  if (typeof window === "undefined") return false;
  return window.innerWidth >= 900;
}

// 広い画面ではタブバーを隠さない（左の縦の帯は常に出しておく）
function tabbarHidden() {
  if (isWide()) return false;
  if (current.name === "ext") return !!current.def.hideTabbar;
  return current.name === "run"; // メッセージ画面と統計画面
}

// 下に重なる部品の高さを CSS 変数 --bottom-stack-h に入れる（05 §5）
function updateBottomStack() {
  const stack = document.getElementById("bottom-stack");
  if (!stack) return;
  const mini = document.getElementById("mini-player");
  const bar = document.getElementById("tabbar");
  const barVisible = !!(bar && !bar.hidden);
  const miniVisible = !!(mini && !mini.hidden);
  // 両方見えないときは重ね部ごと隠す（線が余計に残らないように）
  stack.hidden = !barVisible && !miniVisible;
  document.body.classList.toggle("mini-visible", miniVisible);
  // 中身が隠れて高さ 0 のときは 0px（05 §5）
  // 広い画面ではタブが左の縦の帯で下に重ならないので 0px にする（U4 §1）
  const h = stack.hidden || isWide() ? 0 : stack.offsetHeight;
  document.documentElement.style.setProperty("--bottom-stack-h", `${h}px`);
}

function updateTabbar() {
  const hidden = tabbarHidden();
  document.body.classList.toggle("tabbar-hidden", hidden);
  const bar = document.getElementById("tabbar");
  if (bar) {
    bar.hidden = hidden;
    bar.querySelectorAll("button").forEach((b) => {
      b.classList.toggle("active", b.dataset.tab === currentTab());
    });
  }
  // 今のタブの「最後に開いた URL」を今の住所で更新する（05 §3.3）。
  // 保存先は「その住所が属するタブ」（表示中の current ではない）。こうすると
  // 古い route が後から updateTabbar を呼んでも、違うタブの住所を入れ替えない（U01 §5）。
  const last = loadTabLast();
  last[routeTab(location.hash)] = location.hash || "#/";
  saveTabLast(last);
  updateBottomStack();
}

// タブを押す（05 §3.3）：別のタブへは最後に見た画面，同じタブへは最初の画面
document.querySelectorAll("#tabbar button").forEach((btn) => {
  btn.addEventListener("click", () => {
    const tab = btn.dataset.tab;
    if (!tab) return;
    if (tab === currentTab()) {
      location.hash = TAB_FIRST_URL[tab] || "#/";
      return;
    }
    const last = loadTabLast();
    location.hash = last[tab] || TAB_FIRST_URL[tab] || "#/";
  });
});

// 重ね部の中身が変わったら高さを測り直す（09 で再生バーの高さが変わる）
const bottomStackEl = document.getElementById("bottom-stack");
if (bottomStackEl && "ResizeObserver" in window) {
  new ResizeObserver(() => updateBottomStack()).observe(bottomStackEl);
}

// 画面の幅が変わったら（広い⇔狭い）タブの出しかたと余白を取り直す（U4 §1）
if (typeof window !== "undefined") {
  let wideTimer = null;
  window.addEventListener("resize", () => {
    if (wideTimer) clearTimeout(wideTimer);
    wideTimer = setTimeout(() => {
      wideTimer = null;
      updateTabbar();
    }, 120);
  });
}

// --- 状態表示の共通部品 ---

const STATUS_LAMP = { working: "🟡", blocked: "🔵", idle: "⚪", done: "⚪" };
const RUN_LAMP = {
  running: "🟡", completed: "✅", failed: "⚠", cancelled: "⚠",
};

// 実行中のときだけ live の状態を優先して lamp を選ぶ
function runLamp(run) {
  if (run.status === "running" && run.live) return STATUS_LAMP[run.live] || "🟡";
  return RUN_LAMP[run.status] || "⚪";
}

// 行に並べる状態表示（U4 §4：日本語にする。記号は今までと同じ）
function runStatusText(run) {
  switch (run.status) {
    case "running": return { text: "●Running", cls: "st-running" };
    case "completed": return { text: "✓", cls: "st-done" };
    case "failed": return { text: "✗ Failed", cls: "st-failed" };
    case "cancelled": return { text: "✗ Cancelled", cls: "st-failed" };
    default: return { text: run.status || "", cls: "" };
  }
}

function timeLabel(iso) {
  const d = new Date(iso);
  if (isNaN(d)) return "";
  return d.toLocaleString("ja-JP", hourMinuteOptions());
}

function hourMinuteOptions() {
  return { hour: "2-digit", minute: "2-digit" };
}

// --- 日付一覧画面（#/） ---

let daysCache = [];
let searchText = "";

async function loadDays() {
  if (current.name !== "days") return;
  const kw = searchText.trim();
  const url = kw ? `/api/days?q=${encodeURIComponent(kw)}` : "/api/days";
  try {
    const res = await apiFetch(url);
    if (!res.ok) throw new Error(await res.text());
    const data = await res.json();
    daysCache = data.days || [];
    setOffline(false);
    onOk();
    renderDays(data.results || null);
  } catch {
    setOffline(true);
    onFail();
  }
}

function dayLabel(d) {
  // 2026-09-23 Tue → 2026/09/23 Tue（日付はサーバーが日本時間で切っている・00 §3.6）
  return `${String(d.date).replace(/-/g, "/")} ${d.weekday || ""}`.trim();
}

function dayStats(d) {
  const parts = [];
  const done = d.done || {};
  const doneTotal = Object.values(done).reduce((a, b) => a + b, 0);
  if (doneTotal > 0) {
    // 0 の runner は出さない（02 §1 の1番）
    let s = "確認済み";
    for (const [runner, n] of Object.entries(done)) s += ` ${runner} ${n}`;
    parts.push(s);
  }
  if (d.running > 0) parts.push(`実行中 ${d.running}`);
  if (d.failed > 0) parts.push(`失敗 ${d.failed}`);
  if (d.cost_usd > 0) {
    parts.push(`${d.cost_is_estimate ? "約" : ""}$${Number(d.cost_usd).toFixed(2)}`);
  }
  return parts.join(" · ") || "—";
}

function renderDays(searchResults) {
  const box = document.getElementById("days-list");
  box.innerHTML = "";
  if (searchResults) {
    // 検索中は日付をまたいで一致した実行を直接並べる（02 §3.2）
    if (searchResults.length === 0) {
      box.appendChild(
        saiEmpty({
          icon: "🔍",
          title: "一致するものがありません",
          body: "別の言葉で探してみてください。",
        })
      );
      return;
    }
    for (const run of searchResults) box.appendChild(runRow(run, true));
    return;
  }
  if (daysCache.length === 0) {
    box.appendChild(
      saiEmpty({
        icon: "💬",
        title: "まだ会話がありません",
        body: "プロジェクトでカードを実行すると、ここにエージェントとの会話が出ます。",
        action: { label: "プロジェクトへ", onClick: () => { location.hash = "#/projects"; } },
      })
    );
    return;
  }
  for (const d of daysCache) {
    const btn = document.createElement("button");
    btn.className = "day-row";
    const dot = document.createElement("span");
    // 実行中・質問待ちの日は左に色付きの点（質問待ちは目立つ色）
    dot.className = "dot" + (d.has_question ? " q" : d.running > 0 ? " running" : "");
    const body = document.createElement("span");
    body.className = "day-body";
    const dateSpan = document.createElement("span");
    dateSpan.className = "day-date";
    dateSpan.textContent = dayLabel(d);
    const statsSpan = document.createElement("span");
    statsSpan.className = "day-stats";
    statsSpan.textContent = dayStats(d);
    body.append(dateSpan, statsSpan);
    btn.append(dot, body);
    btn.addEventListener("click", () => { location.hash = `#/day/${d.date}`; });
    box.appendChild(btn);
  }
}

// 広い画面の「その日の実行」画面の左に置く日付一覧（U4 §4）
async function loadDaySide() {
  const box = document.getElementById("day-side");
  if (!box) return;
  if (!isWide()) { box.innerHTML = ""; return; }   // 狭い画面では使わない
  try {
    const res = await apiFetch("/api/days");
    if (!res.ok) return;
    const data = await res.json();
    box.innerHTML = "";
    const title = document.createElement("h2");
    title.className = "sai-section-title";
    title.textContent = "Date";
    box.appendChild(title);
    for (const d of data.days || []) {
      const btn = document.createElement("button");
      btn.type = "button";
      btn.className = "day-row" + (d.date === current.date ? " current" : "");
      const body = document.createElement("span");
      body.className = "day-body";
      const dateSpan = document.createElement("span");
      dateSpan.className = "day-date";
      dateSpan.textContent = dayLabel(d);
      const statsSpan = document.createElement("span");
      statsSpan.className = "day-stats";
      statsSpan.textContent = dayStats(d);
      body.append(dateSpan, statsSpan);
      btn.appendChild(body);
      btn.addEventListener("click", () => { location.hash = `#/day/${d.date}`; });
      box.appendChild(btn);
    }
  } catch {
    // 日付一覧が読めなくてもその日の実行は出せるので、ここは何もしない
  }
}

// --- 日の実行一覧画面（#/day/…） ---

async function loadDay() {
  if (current.name !== "day") return;
  try {
    const res = await apiFetch(`/api/days/${encodeURIComponent(current.date)}`);
    if (!res.ok) throw new Error(await res.text());
    const data = await res.json();
    setOffline(false);
    onOk();
    document.getElementById("day-title").textContent = dayLabel({ date: data.date, weekday: weekdayOf(data.date) });
    renderDay(data);
    await loadDaySide();
  } catch {
    setOffline(true);
    onFail();
  }
}

// サーバーが weekday を返さない場合の保険（表示は英字3文字）
function weekdayOf(dateStr) {
  const d = new Date(`${dateStr}T00:00:00+09:00`);
  if (isNaN(d)) return "";
  return d.toLocaleDateString("en-US", { weekday: "short" });
}

function renderDay(data) {
  const box = document.getElementById("day-list");
  box.innerHTML = "";
  const projects = data.projects || [];
  if (projects.length === 0) {
    box.appendChild(
      saiEmpty({
        icon: "📅",
        title: "この日の実行はありません",
        body: "カードが動くと、この日に会話が出ます。",
        action: { label: "ほかの日付を見る", onClick: () => { location.hash = "#/"; } },
      })
    );
    return;
  }
  for (const p of projects) {
    const h = document.createElement("h2");
    h.className = "project-title";
    h.textContent = p.project_name || "(不明)";
    box.appendChild(h);
    for (const run of p.runs || []) box.appendChild(runRow(run, false));
  }
}

// 実行の1行（日の一覧・検索結果で共通）
function runRow(run, showDate) {
  const btn = document.createElement("button");
  btn.className = "run-row";
  if (run.has_question || (run.status === "running" && run.live === "blocked")) {
    btn.classList.add("question"); // 質問待ちの実行は行を強調する
  }
  if (run.indent) btn.classList.add("indent");

  if (showDate && run.date) {
    const dateSpan = document.createElement("span");
    dateSpan.className = "rdate";
    dateSpan.textContent = String(run.date).slice(5).replace("-", "/");
    btn.appendChild(dateSpan);
  }
  const timeSpan = document.createElement("span");
  timeSpan.className = "rtime";
  timeSpan.textContent = run.started_at ? timeLabel(run.started_at) : "";
  const idSpan = document.createElement("span");
  idSpan.className = "rid";
  idSpan.textContent = `#${run.run_id}`;
  const blockSpan = document.createElement("span");
  blockSpan.className = "rblock";
  blockSpan.textContent = run.block_name || "";
  const descSpan = document.createElement("span");
  descSpan.className = "rdesc";
  // 長い説明は1行で省略（02 §1 の2番）
  descSpan.textContent = run.description || "";
  const runnerSpan = document.createElement("span");
  runnerSpan.className = "rrunner";
  runnerSpan.textContent = `[${run.runner}]`;
  const st = runStatusText(run);
  const statusSpan = document.createElement("span");
  statusSpan.className = `rstatus ${st.cls}`;
  statusSpan.textContent = st.text;
  btn.append(timeSpan, idSpan, blockSpan, descSpan, runnerSpan, statusSpan);

  if (run.show_parent && run.parent_run_id != null) {
    // 親の実行がこの日の一覧に無いサブタスク（02 §1 の2番）
    const parentSpan = document.createElement("span");
    parentSpan.className = "rparent";
    parentSpan.textContent = `↳ #${run.parent_run_id}`;
    btn.appendChild(parentSpan);
  }
  btn.addEventListener("click", () => {
    location.hash = `#/run/${encodeURIComponent(run.project_name)}/${encodeURIComponent(run.run_id)}`;
  });
  return btn;
}

// --- 実行（メッセージ）画面（#/run/…） ---
// 全項目を折りたたみで表示し，開閉は画面内で id で覚える（03 §4）。
// 見出しは全項目に出し（発言者＋時刻），日付が変わると区切り線を入れる。

let currentRun = null;   // Run（サーバーが返した現在の1件）
let chatCursor = null;   // どこまで読んだか（サーバーが返す目印）
let knownIds = new Set(); // 既に描いた発言のID（重複防止）
let openIds = new Set();  // 開いている項目のID（自動更新で描き直しても保持・03 §4）
let chatRowEls = [];      // 会話の行の要素（時刻順・日付区切り線は含まない・CP3）
let chatRowTimes = [];    // その行の時刻（ISO 文字列・chatRowEls と同じ並び）
let stickToBottom = true; // 下端にいるか（自動追従の要否）
let firstLoadDone = false;
let chatPartialText = ""; // 今出している「生成中」の文（空 = 出していない）
let chatErrorText = "";   // 今出している error の文（空 = 出していない）
let runStarting = false;  // 新しいチャットの実行がまだ書き出されていない（起動待ち）

function runApiBase() {
  return `/api/runs/${encodeURIComponent(currentRun.project_name)}/${encodeURIComponent(currentRun.run_id)}`;
}

// 実験タブの run の表への小さなリンク（#run-desc の後ろ・U08）。
// 作り直さないので、何度和読み込んでも同じ要素を使い回す。
// 実験タブの表は 0 件でも案内を出すので、この run の層は見ていない。
function setRunMlfLink(runId) {
  const desc = document.getElementById("run-desc");
  if (!desc || !desc.parentNode) return;
  let box = document.getElementById("run-mlf");
  if (!box) {
    box = document.createElement("div");
    box.id = "run-mlf";
    box.className = "run-mlf";
    const a = document.createElement("a");
    a.className = "run-mlf-link";
    a.textContent = "実験の記録";
    box.appendChild(a);
    desc.parentNode.insertBefore(box, desc.nextSibling);
  }
  const a = box.querySelector("a");
  if (a) a.href = `#/research/experiments/e/all?sai_run=${encodeURIComponent(runId)}`;
}

async function openRun(r) {
  currentRun = {
    project_name: r.project,
    run_id: r.id,
    status: "",
    live: null,
    date: null,
    question_session_id: null,
  };
  chatCursor = null;
  knownIds = new Set();
  // 前の実行で覚えた「カードの中身」は捨てる（同じ実行を開いたときに
  // カードが描かれなくなるのを防ぐため，描き直せるようにしておく・CP3）
  cardsSignature = "";
  openIds = new Set();    // 別の実行に移ったら開閉状態は忘れてよい（03 §4）
  chatRowEls = [];
  chatRowTimes = [];
  chatLastAgentAt = 0;   // 別の実行に移ったら「続けて書かれた」の判定は切り直す
  stickToBottom = true;
  firstLoadDone = false;
  chatPartialText = "";
  chatErrorText = "";
  runStarting = false;
  document.getElementById("chat-list").innerHTML = "";
  document.getElementById("run-title").textContent = "読み込み中…";
  document.getElementById("run-desc").textContent = "";
  document.getElementById("run-lamp").textContent = "";
  // ■ 終了と ⏸ 中断は実行中のときだけ出すので，前の実行の残りを持ち越さない
  document.getElementById("finalize-button").hidden = true;
  document.getElementById("interrupt-button").hidden = true;
  await loadRunMeta();
  if (!runStarting) await loadChatMore();
  firstLoadDone = true;
}

// 新しいチャット（newchat.js）から来た実行かどうかの見分け。
// sessionStorage に run_id と時刻を覚えておき，60 秒までは書き出しを待つ（U04 §5）。
const NEWCHAT_RUN_KEY = "sai_web_newchat_run";
const NEWCHAT_WAIT_MS = 60000;

function newChatWaiting(runId) {
  try {
    const raw = sessionStorage.getItem(NEWCHAT_RUN_KEY);
    if (!raw) return false;
    const data = JSON.parse(raw);
    if (!data || String(data.run_id) !== String(runId)) return false;
    return Date.now() - Number(data.at || 0) <= NEWCHAT_WAIT_MS;
  } catch {
    return false;
  }
}

function clearNewChatWaiting(runId) {
  try {
    const raw = sessionStorage.getItem(NEWCHAT_RUN_KEY);
    if (!raw) return;
    const data = JSON.parse(raw);
    if (data && String(data.run_id) === String(runId)) sessionStorage.removeItem(NEWCHAT_RUN_KEY);
  } catch { /* 消せなくてもよい */ }
}

// 入力欄の出し入れ（U04 §4）: live が null（終わった）のときだけ使えなくする。
// 起動待ちのあいだも使えなくする。status が completed でも live があれば送れる（C6）。
function updateComposer() {
  const live = currentRun ? currentRun.live : null;
  const waiting = runStarting;
  const finished = !waiting && live == null;
  const ta = document.getElementById("chat-input");
  const send = document.getElementById("send-button");
  const mic = document.getElementById("mic-button");
  if (ta) {
    ta.disabled = waiting || finished;
    ta.placeholder = waiting ? "起動を待っています…" : finished ? "この会話は終了しています" : "メッセージを入力";
  }
  if (send) send.disabled = waiting || finished;
  if (mic) mic.disabled = waiting || finished;
}

async function loadRunMeta() {
  if (current.name !== "run") return;
  try {
    const res = await apiFetch(`/api/runs/${encodeURIComponent(current.project)}/${encodeURIComponent(current.id)}`);
    if (res.status === 404 && newChatWaiting(current.id)) {
      // 新しいチャットから来た実行がまだ書き出されていない（約 5 秒の遅れ）
      runStarting = true;
      document.getElementById("run-title").textContent = "起動を待っています…";
      document.getElementById("run-desc").textContent = "";
      document.getElementById("run-lamp").textContent = "";
      document.getElementById("finalize-button").hidden = true;
      document.getElementById("interrupt-button").hidden = true;
      updateComposer();
      intervalMs = 2000;
      rearm();
      return null;
    }
    if (!res.ok) throw new Error("bad");
    const data = await res.json();
    const prevLive = currentRun ? currentRun.live : undefined;
    runStarting = false;
    currentRun = data;
    setOffline(false);
    // 見出し1行目は #run_id block_name，2行目に description（03 §1-7）
    document.getElementById("run-title").textContent =
      `#${data.run_id} ${data.block_name || ""}`.trim();
    document.getElementById("run-desc").textContent = data.description || "";
    document.getElementById("run-lamp").textContent = runLamp(data);
    // 実験タブの run の表への小さなリンク（#run-desc の後ろに 1 つだけ置く・U08）
    setRunMlfLink(data.run_id);
    // ■ 終了は「実行中のメインブロック」のときだけ出す（子ブロックは task_report で完了する）
    document.getElementById("finalize-button").hidden = !(
      data.status === "running" && !data.parent_run_id
    );
    // ⏸ 中断は会話が続いているあいだ出す。live は状態ファイルの都合で working とずれるため，
    // 終わった（null）ときだけ隠す。idle に esc を送っても入力欄が空なら何も起きない（U04 §3）
    document.getElementById("interrupt-button").hidden = data.live == null;
    updateComposer();
    // live が変わったら更新の間隔を取り直す（生成中は 1 秒ごと・U04 §2）
    if (prevLive !== data.live) { intervalMs = baseInterval(); rearm(); }
    // 起動待ちはここまで（見分けの印を消す）
    clearNewChatWaiting(data.run_id);
    return data;
  } catch {
    if (document.getElementById("run-title").textContent === "起動を待っています…") {
      document.getElementById("run-title").textContent = "読み込めませんでした";
    }
    setOffline(true);
    return null;
  }
}

async function loadChatMore() {
  if (current.name !== "run" || !currentRun) return;
  const url = chatCursor
    ? `${runApiBase()}/items?after=${encodeURIComponent(chatCursor)}`
    : `${runApiBase()}/items`;
  try {
    const res = await apiFetch(url);
    if (!res.ok) throw new Error("bad");
    const data = await res.json();
    chatCursor = data.next_cursor || chatCursor;
    appendItems(data.items || []);
    await loadQuestions();
    await flushPendingSends();
    applyPartial(data.partial);
    applyError(data.error);
    setOffline(false);
    onOk();
  } catch {
    // 切れている間は読みかけの会話を残し，帯だけ出す．次回に回す．
    setOffline(true);
    onFail();
    return;
  }
}

// 表示中の画面だけ更新する（実行画面は Meta → 会話 の順で2秒ごと）
async function pollRun() {
  await loadRunMeta();
  // 起動待ちのあいだは会話を読みに行かない（まだ書き出されていない・U04 §5）
  if (runStarting) return;
  await loadChatMore();
}

// --- 質問カード ---
// 未解決の質問（cc / pi 共通の見た目）を会話の末尾に
// 常に開いた状態で出す（05 §1）。
// - open: 選ぶ → 送信 の2段階。全問に答えられるまで送信ボタンは押せない。
// - answered: 「回答済み: …」の1行に変わる。
// - resolved_elsewhere: 「PC で回答済み」の1行に変わる（先にPCで答えた）。
let openQuestions = [];
let answering = false;
// 前回描いたカードの内容（同じなら描き直さない）。
// 描き直すと入力中の自由入力欄・拒否理由欄が毎2秒消えるため。
let cardsSignature = "";

async function loadQuestions() {
  const sid = currentRun && currentRun.question_session_id;
  if (!sid) {
    // 質問と対応づけられない実行はカードを出さない（前の実行の残りも消す）
    openQuestions = [];
    if (cardsSignature !== "[]") {
      cardsSignature = "[]";
      renderQuestions();
    }
    return;
  }
  try {
    const res = await apiFetch(`/api/questions?session_id=${encodeURIComponent(sid)}`);
    if (res.ok) openQuestions = (await res.json()).questions || [];
  } catch { /* 取れなければ前回のまま */ }
  const sig = JSON.stringify(openQuestions);
  if (sig === cardsSignature) return; // 中身が変わっていないので描き直さない
  cardsSignature = sig;
  renderQuestions();
}

function renderQuestions() {
  // 古いカードを消して作り直す（解決済み・回答済みの表示に切り替わるため）
  document.querySelectorAll(".question-card").forEach((el) => el.remove());
  const box = document.getElementById("chat-list");
  for (const q of openQuestions) box.appendChild(questionCard(q));
  // 決まった位置へ置き直す（settled は質問された時刻の位置へ，未回答は一番下へ・CP3）
  placeOverlays();
  // 次回の比較用に今の内容を覚える（送信直後の描き直しでも同じように効く）
  cardsSignature = JSON.stringify(openQuestions);
  if (openQuestions.length > 0 && stickToBottom) {
    window.scrollTo(0, document.body.scrollHeight);
  }
}

// 解決済み（PC で先に答えた）のカードを1行の帯にする
function settledCard(card, className, line) {
  const el = document.createElement("div");
  el.className = `question-card settled ${className}`;
  el.dataset.qid = card.id || "";
  // 回答済みの印と，質問された時刻（UNIX 秒）．カードを置く位置はこの時刻で決める（CP3）
  el.dataset.settled = "1";
  el.dataset.createdAt = card.created_at != null ? String(card.created_at) : "";
  el.textContent = line;
  return el;
}

// 回答済みカードの外枠．settled の印と時刻は帯と同じ 3 つを付ける（置く位置はこれで決まる）
function doneFrame(card, extraClass) {
  const el = document.createElement("div");
  el.className = `question-card q-done${extraClass ? ` ${extraClass}` : ""}`;
  el.dataset.qid = card.id || "";
  el.dataset.settled = "1";
  el.dataset.createdAt = card.created_at != null ? String(card.created_at) : "";
  return el;
}

// 押せない選択肢（button ではなく div）．selected なら選んだ印の色になる
function doneOption(labelText, descText, selected, multi) {
  const opt = document.createElement("div");
  opt.className = "q-option static" + (multi ? " multi" : "") + (selected ? " selected" : "");
  const label = document.createElement("div");
  label.className = "q-label";
  label.textContent = labelText;
  opt.appendChild(label);
  if (descText) {
    const desc = document.createElement("div");
    desc.className = "q-desc";
    desc.textContent = descText;
    opt.appendChild(desc);
  }
  return opt;
}

// 回答済みの 1 問で，どの選択肢が選ばれたか。戻り値は { picked: 選んだ index の配列, text: 自由入力の文字 }
function answeredPicks(answer) {
  const a = answer || {};
  if (typeof a.text === "string" && a.text.trim()) return { picked: [], text: a.text.trim() };
  if (Array.isArray(a.choice_indices)) return { picked: a.choice_indices.slice(), text: "" };
  if (a.choice_index != null) return { picked: [a.choice_index], text: "" };
  return { picked: [], text: "" };
}

// 回答済みの質問カード．質問時と同じ選択肢をすべて残し，選んだものを強調する
function answeredCard(q) {
  const el = doneFrame(q, "");
  const answers = (q.answer && q.answer.answers) || [];
  const qs = q.questions || [];
  const picks = qs.map((_, i) => answeredPicks(answers[i]));
  const any = picks.some((p) => p.picked.length || p.text);
  const title = document.createElement("div");
  title.className = "q-settled-title";
  title.textContent = any ? "回答済み" : "回答済み（内容なし）";
  el.appendChild(title);
  qs.forEach((one, qi) => {
    const block = document.createElement("div");
    block.className = "q-block";
    const head = document.createElement("div");
    head.className = "q-header";
    head.textContent = one.header || `質問${qi + 1}`;
    const body = document.createElement("div");
    body.className = "q-body";
    body.textContent = one.question || "";
    block.append(head, body);
    const multi = one.multiSelect === true;
    (one.options || []).forEach((opt, oi) => {
      const on = picks[qi].picked.includes(oi);
      const mark = multi ? (on ? "☑ " : "☐ ") : (on ? "✓ " : "");
      const desc = opt.description && opt.description !== opt.label ? opt.description : "";
      block.appendChild(doneOption(`${mark}${opt.label || ""}`, desc, on, multi));
    });
    if (picks[qi].text) block.appendChild(doneOption(`✎ その他: ${picks[qi].text}`, "", true, false));
    el.appendChild(block);
  });
  return el;
}

function questionCard(q) {
  // 回答済みは選択肢を残して選んだものを強調，PC で回答済みは1行だけ表示（05 §1 の2番）
  if (q.state === "answered") return answeredCard(q);
  if (q.state === "resolved_elsewhere") {
    return settledCard(q, "q-elsewhere", "PC で回答済み");
  }

  const card = document.createElement("div");
  card.className = "question-card";
  card.dataset.qid = q.id;
  const qs = q.questions || [];
  // 各問の回答状態．null は未回答．{choice_index} / {choice_indices} / {text} が入る．
  const state = new Array(qs.length).fill(null);
  // 問ごとの部品（読み上げ・音声回答から .click() と input で触るため・U01）
  const controls = [];

  const submitBtn = document.createElement("button");
  submitBtn.className = "q-submit";
  submitBtn.textContent = "Send";
  submitBtn.disabled = true;

  const answered = (qi) => {
    const a = state[qi];
    if (!a) return false;
    if (Array.isArray(a.choice_indices)) return a.choice_indices.length > 0;
    return a.choice_index != null || !!(a.text && a.text.trim());
  };
  const updateSubmit = () => {
    // 全問に答えているまで押せない（05 §1 の2番）
    submitBtn.disabled = qs.some((_, qi) => !answered(qi));
  };

  qs.forEach((one, qi) => {
    const block = document.createElement("div");
    block.className = "q-block";
    const title = document.createElement("div");
    title.className = "q-header";
    title.textContent = one.header || `質問${qi + 1}`;
    const body = document.createElement("div");
    body.className = "q-body";
    body.textContent = one.question || "";
    block.append(title, body);

    const opts = one.options || [];
    const multi = one.multiSelect === true;
    const optionButtons = [];
    opts.forEach((opt, oi) => {
      const btn = document.createElement("button");
      btn.className = "q-option" + (multi ? " multi" : "");
      const label = document.createElement("div");
      label.className = "q-label";
      // 複数選択は選んだ印を先頭に出す（閉じた状態でも選択中が分かるように）
      label.textContent = multi ? `☐ ${opt.label || ""}` : (opt.label || "");
      btn.appendChild(label);
      if (opt.description && opt.description !== opt.label) {
        const desc = document.createElement("div");
        desc.className = "q-desc";
        desc.textContent = opt.description;
        btn.appendChild(desc);
      }
      btn.addEventListener("click", () => {
        if (multi) {
          // 複数選択: タップで選んだ/選ばないを切り替える（何度でも選び直せる）
          freeInput.value = ""; // 自由入力の途中なら選択肢クリックで消す
          const prev = (state[qi] && state[qi].choice_indices) || [];
          const cur = new Set(Array.isArray(prev) ? prev : []);
          if (cur.has(oi)) cur.delete(oi);
          else cur.add(oi);
          const picked = [...cur].sort((x, y) => x - y);
          state[qi] = picked.length ? { choice_indices: picked } : null;
          optionButtons.forEach((b, bj) => {
            const on = Array.isArray(state[qi] && state[qi].choice_indices)
              ? state[qi].choice_indices.includes(bj) : false;
            b.classList.toggle("selected", on);
            b.querySelector(".q-label").textContent = `${on ? "☑" : "☐"} ${(opts[bj] || {}).label || ""}`;
          });
        } else {
          // 単一選択: タップで選ぶ（もう一度押すと選択は維持，別のボタンで選び直せる）
          state[qi] = { choice_index: oi };
          optionButtons.forEach((b) => b.classList.toggle("selected", b === btn));
          freeInput.value = "";
        }
        updateSubmit();
      });
      optionButtons.push(btn);
      block.appendChild(btn);
    });

    // ボタン以外（自由文）でも答えられる
    const freeInput = document.createElement("input");
    freeInput.type = "text";
    freeInput.className = "q-free";
    freeInput.placeholder = "その他（自由入力）";
    freeInput.addEventListener("input", () => {
      const v = freeInput.value.trim();
      if (v) {
        state[qi] = { text: v };
        optionButtons.forEach((b) => {
          b.classList.remove("selected");
          const bj = optionButtons.indexOf(b);
          b.querySelector(".q-label").textContent = `${multi ? "☐" : ""}${(opts[bj] || {}).label || ""}`;
        });
      } else {
        state[qi] = null;
      }
      updateSubmit();
    });
    block.appendChild(freeInput);
    controls[qi] = { optionButtons, freeInput, multi };
    card.appendChild(block);
  });

  submitBtn.addEventListener("click", () => submitAnswers(q, state, card, submitBtn));
  card.appendChild(submitBtn);

  // 読み上げ・音声回答の 1 行を足す（U01・voiceask.js）。答えは既存のボタンと入力欄の処理を通す
  if (typeof vaAttach === "function") {
    vaAttach(card, qs, {
      apply: (qi, answer) => {
        const c = controls[qi];
        if (!c || !answer) return;
        if (answer.choice_index != null) {
          const b = c.optionButtons[answer.choice_index];
          if (b) b.click();
        } else if (Array.isArray(answer.choice_indices)) {
          // 選び直せるよう，入っているものだけ外してから指定のものを選び直す
          const prev = (state[qi] && state[qi].choice_indices) || [];
          prev.forEach((n) => { const b = c.optionButtons[n]; if (b) b.click(); });
          answer.choice_indices.forEach((n) => { const b = c.optionButtons[n]; if (b) b.click(); });
        } else if (typeof answer.text === "string") {
          c.freeInput.value = answer.text;
          c.freeInput.dispatchEvent(new Event("input"));
        }
      },
      isAnswered: (qi) => answered(qi),
    });
  }
  return card;
}

async function submitAnswers(q, state, card, submitBtn) {
  if (answering) return;
  answering = true;
  // 送信中はボタン・入力欄を押せなくする
  card.querySelectorAll("button, input").forEach((el) => { el.disabled = true; });
  try {
    const res = await apiFetch(`/api/questions/${encodeURIComponent(q.id)}/answer`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ answers: state }),
    });
    if (!res.ok) {
      const err = await res.json().catch(() => ({}));
      alert(err.detail || "送れませんでした");
      card.querySelectorAll("button, input").forEach((el) => { el.disabled = false; });
      submitBtn.disabled = true;
      return;
    }
    // 送信後は「回答済み: …」の1行に変わる（05 §1 の2番）
    openQuestions = openQuestions.map((x) =>
      x.id === q.id
        ? { ...x, state: "answered", answer: { answers: state } }
        : x
    );
    renderQuestions();
  } finally {
    answering = false;
  }
}

function appendItems(items) {
  // 記録に現れた文の「反映待ち」の印は消す（描き直しは末尾へ寄るが，下の placeOverlays で一番下に戻る）
  if (settleReflectedSends(items)) renderPendingMarks();
  // 最初は末尾200項目だけ（数千項目を一度に描くと固まるため）。
  // 上スクロールでの追加読み込みは後の段階で足す（今は全件末尾寄せ）。
  const fresh = items.filter((i) => i && i.id && !knownIds.has(i.id));
  if (fresh.length === 0) return;
  // 初回で200超の場合は末尾200だけ残す
  let toAdd = fresh;
  if (!firstLoadDone && fresh.length > 200) toAdd = fresh.slice(-200);
  const box = document.getElementById("chat-list");
  for (const item of toAdd) {
    knownIds.add(item.id);
    const node = renderItem(item);
    // 開いていた項目は自動更新で描き直されても開いたまま（03 §4）
    if (openIds.has(item.id)) node.classList.add("open");
    // 時刻順の位置に挿む（ふつうは末尾．古い時刻の行が混じっても並びが崩れない・CP3）
    const idx = firstIndexAfter(chatRowTimes, item.time);
    box.insertBefore(node, chatRowEls[idx] || null);
    chatRowEls.splice(idx, 0, node);
    chatRowTimes.splice(idx, 0, item.time || "");
  }
  // 行の並びに合わせて区切り線を作り直し，カードも決めた位置へ置き直す（CP3）
  rebuildDateSeps();
  placeOverlays();
  updateNewArrival();
  // 下端にいる時だけ自動で追従する
  if (stickToBottom) window.scrollTo(0, document.body.scrollHeight);
}

// --- 生成中の文と error（U04 §1） ---

// 生成中の文を 1 つだけ吹き出しとして出す（扱いは chatPartialPlan が決める）。
// 確定した行やカードより後ろ（末尾）に置き直す。
function applyPartial(partial) {
  const next = typeof partial === "string" ? partial : "";
  const plan = chatPartialPlan(chatPartialText, next);
  if (plan === "none") return;
  const box = document.getElementById("chat-list");
  if (!box) return;
  let el = document.getElementById("chat-partial");
  if (plan === "hide") {
    chatPartialText = "";
    if (el) el.remove();
    return;
  }
  chatPartialText = next;
  if (!el) {
    el = document.createElement("div");
    el.id = "chat-partial";
    el.className = "fold-item kind-agent partial";
    const head = document.createElement("div");
    head.className = "fold-head";
    const who = document.createElement("span");
    who.className = "fold-who";
    who.textContent = agentName();
    // 点滅する印（生成中を示す・U04 §1）
    const dot = document.createElement("span");
    dot.className = "partial-dot";
    dot.setAttribute("role", "img");
    dot.setAttribute("aria-label", "生成中");
    head.append(who, dot);
    const body = document.createElement("div");
    body.className = "fold-body bubble-agent";
    el.append(head, body);
  }
  const body = el.querySelector(".fold-body");
  if (body) body.innerHTML = renderMarkdown(next);
  box.appendChild(el);
  if (stickToBottom) window.scrollTo(0, document.body.scrollHeight);
}

// error の文を 1 回だけ赤い帯で出す（同じ文は 2 回出さない・U04 §1）。
function applyError(error) {
  const next = typeof error === "string" ? error.trim() : "";
  const box = document.getElementById("chat-list");
  if (!box) return;
  let el = document.getElementById("chat-error");
  if (!next) {
    chatErrorText = "";
    if (el) el.remove();
    return;
  }
  if (next === chatErrorText) {
    if (el) box.appendChild(el); // 新しい行より後ろに置き直す
    return;
  }
  chatErrorText = next;
  if (el) el.remove();
  el = renderEvent({
    id: "chat-error",
    kind: "event",
    status: "error",
    title: "エラー",
    summary: next,
    time: new Date().toISOString(),
  });
  el.id = "chat-error";
  box.appendChild(el);
  if (stickToBottom) window.scrollTo(0, document.body.scrollHeight);
}

// --- 日付の区切り線（── 9月23日(火) ──・03 §4） ---

function dateKey(iso) {
  if (!iso) return "";
  const d = new Date(iso);
  if (isNaN(d)) return "";
  return `${d.getFullYear()}-${d.getMonth() + 1}-${d.getDate()}`;
}

function dateSepLabel(iso) {
  const d = new Date(iso);
  const wd = ["日", "月", "火", "水", "木", "金", "土"][d.getDay()];
  return `${d.getMonth() + 1}月${d.getDate()}日(${wd})`;
}

// 会話の行から日付の区切り線を作り直す（03 §4）。
// 行を時刻順に置いたりカードが移動したりしても崩れないように毎回作り直す。
// 区切り線だけなので高さは変わらず，スクロールの位置も動かない。
function rebuildDateSeps() {
  const box = document.getElementById("chat-list");
  box.querySelectorAll(".date-sep").forEach((el) => el.remove());
  let prev = "";
  for (let i = 0; i < chatRowEls.length; i += 1) {
    const key = dateKey(chatRowTimes[i]);
    // 時刻の無い行では区切りを挟まない（直前の日付のまま扱う）
    if (!key || key === prev) continue;
    const sep = document.createElement("div");
    sep.className = "date-sep";
    sep.textContent = dateSepLabel(chatRowTimes[i]);
    box.insertBefore(sep, chatRowEls[i]);
    prev = key;
  }
}

// --- 質問カード・送信待ちの置き場（確認点 CP3 の直し） ---
// 会話の行に対する位置を毎回ここで決める。描き直さず move するだけなので，
// 入力中のカード（選択・自由入力の途中）の中身は消えない。
// - 回答済み・PC で回答済み（settled）: 質問された時刻より後の最初の行の直前（無ければ末尾）
// - まだ答えていないカード: 一番下（今までどおり）
// - 送信中の吹き出し（pending-send）: 一番下
function placeOverlays() {
  const box = document.getElementById("chat-list");
  const settled = [];
  const openCards = [];
  const pending = [];
  for (const el of Array.from(box.children)) {
    if (el.classList.contains("question-card")) {
      if (el.dataset.settled === "1") settled.push(el);
      else openCards.push(el);
    } else if (el.classList.contains("pending-send")) {
      pending.push(el);
    }
  }
  // 質問された時刻の古い順に置く（同じ時刻なら今の並びを添字で保つ）
  const createdAt = (el) => {
    const raw = el.dataset.createdAt;
    if (!raw) return Infinity; // 時刻が無ければ末尾寄り
    const n = Number(raw);
    return Number.isFinite(n) ? n : Infinity;
  };
  const order = settled.map((el, i) => [el, i]);
  order.sort((a, b) => (createdAt(a[0]) - createdAt(b[0])) || (a[1] - b[1]));
  for (const [card] of order) {
    const raw = card.dataset.createdAt;
    const idx = firstIndexAfter(chatRowTimes, raw ? Number(raw) : null);
    let target = chatRowEls[idx] || null;
    // 目標の行が「新しい日の最初の行」で，カードの時刻が前の日のものなら，
    // 日付の区切り線の上に置く（翌日の区切りの下に出さないようにする）
    const cardMs = raw ? Number(raw) * 1000 : NaN;
    if (target && Number.isFinite(cardMs)) {
      const prev = target.previousElementSibling;
      const rowDay = dateKey(chatRowTimes[idx]);
      const cardDay = dateKey(new Date(cardMs).toISOString());
      if (
        prev && prev.classList && prev.classList.contains("date-sep") &&
        rowDay && cardDay && rowDay !== cardDay
      ) {
        target = prev;
      }
    }
    box.insertBefore(card, target);
  }
  // 未回答カードと送信待ちは一番下（settled より後ろに来る）
  for (const card of openCards) box.appendChild(card);
  for (const div of pending) box.appendChild(div);
}

// --- チャットのまとまり（U01 §2） ---

// 直前のエージェント発言の時刻。続けて 5 分以内に書かれたものは
// 同じまとまりとして、名前とアイコンを繰り返さない（Slack と同じ）
let chatLastAgentAt = 0;

// 次のエージェントの発言が前のものと 5 分以内なら同じまとまり（renderItem が使う）
function chatAgentGrouped(iso) {
  const t = iso ? new Date(iso).getTime() : NaN;
  const prev = chatLastAgentAt;
  chatLastAgentAt = Number.isFinite(t) ? t : 0;
  if (!Number.isFinite(t) || !prev) return false;
  return t >= prev && t - prev <= 5 * 60 * 1000;
}

// --- 折りたたみ（全項目共通・03 §4） ---

// 見出し（発言者＋時刻）の付いた折りたたみ箱を作る．タップで開閉し，
// 開閉の状態は openIds に id で覚えておく（自動更新でも保持するため）。
function foldItem(id, iso) {
  const wrap = document.createElement("div");
  wrap.className = "fold-item";
  wrap.dataset.id = id;
  const head = document.createElement("div");
  head.className = "fold-head";
  const who = document.createElement("span");
  who.className = "fold-who";
  // 時刻は Slack のように灰色で小さく（03 §4）
  const tm = document.createElement("span");
  tm.className = "fold-time";
  tm.textContent = timeLabel(iso);
  head.append(who, tm);
  wrap.appendChild(head);
  wrap.addEventListener("click", () => {
    const open = wrap.classList.toggle("open");
    if (open) openIds.add(id);
    else openIds.delete(id);
  });
  return { wrap, who };
}

// 閉じている時に見せる1行目．Markdown の記号は取り，続きがあれば … を付ける（03 §4）
function previewLine(text) {
  const lines = String(text || "").split("\n").map((s) => s.trim()).filter(Boolean);
  if (lines.length === 0) return "";
  const first = lines[0]
    .replace(/^#{1,6}\s+/, "")
    .replace(/^[-*+]\s+/, "")
    .replace(/^\d+\.\s+/, "")
    .replace(/^>\s+/, "")
    .replace(/\*\*/g, "")
    .replace(/`/g, "")
    .trim();
  // 続きがある時だけ … を付ける（1行が長すぎる分は CSS で切る）
  return lines.length > 1 ? `${first} …` : first;
}

// プレビュー行と中身（開いた時だけ出る）を折りたたみ箱に入れる
function foldBody(wrap, preview, body) {
  if (preview) {
    const prev = document.createElement("div");
    prev.className = "fold-preview";
    prev.textContent = preview;
    wrap.appendChild(prev);
  }
  wrap.appendChild(body);
}

// MCP の道具かどうか（cc は mcp__donsetch__web_search、pi は adapter の mcp / mcp__…・U12）。
// sai の CLI を Bash で呼んだ項目も同じ緑系で出す（item.cli が入る・U17）。
function isMcpTool(item) {
  if (item && item.cli) return true;
  const tool = (item && item.tool) || "";
  return tool === "mcp" || tool.startsWith("mcp__") || tool.startsWith("mcp.");
}

function renderItem(item) {
  if (item.kind === "event") return renderEvent(item);
  const fold = foldItem(item.id, item.time);
  const body = document.createElement("div");
  if (item.kind === "user") {
    fold.wrap.classList.add("kind-user");
    fold.who.textContent = "あなた";
    chatLastAgentAt = 0; // 自分の発言を挟んだら、次のエージェントの発言は名前から出し直す
    body.className = "fold-body bubble-user";
    body.textContent = item.text || "";
    foldBody(fold.wrap, previewLine(item.text), body);
    return fold.wrap;
  }
  if (item.kind === "agent") {
    fold.wrap.classList.add("kind-agent");
    const who = agentName();
    fold.who.textContent = who;
    // 最初の 1 行だけ名前・時刻・丸いアイコンを出す（続けて 5 分以内なら繰り返さない）
    if (chatAgentGrouped(item.time)) {
      fold.wrap.classList.add("grouped");
    } else {
      fold.wrap.classList.add("has-avatar");
      fold.wrap.dataset.avatar = who.slice(0, 1);
    }
    body.className = "fold-body bubble-agent";
    body.innerHTML = renderMarkdown(item.text || "");
    foldBody(fold.wrap, previewLine(item.text), body);
    return fold.wrap;
  }
  // tool：見出しはツール名（失敗は ⚠ と色）．中身はタップで開く（03 §1）
  fold.wrap.classList.add("kind-tool");
  // MCP の道具と sai の CLI の呼び出しだけは緑系にする（cc は mcp__…、pi は mcp / mcp__…・U12・U17）
  if (isMcpTool(item)) fold.wrap.classList.add("mcp");
  if (item.ok === false) fold.wrap.classList.add("failed");
  fold.who.textContent = (item.ok === false ? "⚠ " : "⚙ ") + (item.label || item.tool || "ツール");
  body.className = "fold-body fold-detail";
  body.textContent = item.detail || "(中身なし)";
  fold.wrap.appendChild(body);
  return fold.wrap;
}

// event カード：吹き出しではなく，左に色の帯がある薄い箱（03 §4）。
// ok=緑，error=赤，info=灰。閉じた時の見出しは title（＋ summary の1行目）。
function renderEvent(item) {
  const fold = foldItem(item.id, item.time);
  fold.wrap.classList.add("event-card", `ev-${item.status || "info"}`);
  fold.who.textContent = item.title || "通知";
  const body = document.createElement("div");
  body.className = "fold-body ev-body";
  if (item.summary) {
    const s = document.createElement("div");
    s.className = "ev-summary";
    s.textContent = item.summary;
    body.appendChild(s);
  }
  if (Array.isArray(item.fields) && item.fields.length > 0) {
    const fields = document.createElement("div");
    fields.className = "ev-fields";
    for (const pair of item.fields) {
      const ks = document.createElement("span");
      ks.className = "ev-k";
      ks.textContent = Array.isArray(pair) ? String(pair[0]) : "";
      const vs = document.createElement("span");
      vs.className = "ev-v";
      vs.textContent = Array.isArray(pair) && pair.length > 1 ? String(pair[1]) : "";
      fields.append(ks, vs);
    }
    body.appendChild(fields);
  }
  if (item.detail) {
    const d = document.createElement("div");
    d.className = "ev-detail";
    d.textContent = item.detail;
    body.appendChild(d);
  }
  foldBody(fold.wrap, previewLine(item.summary), body);
  return fold.wrap;
}

function agentName() {
  if (currentRun && currentRun.runner === "pi") return "π";
  if (currentRun && currentRun.runner === "cc") return "Claude";
  return "エージェント";
}

// md の整形表示は static/markdown.js の renderMarkdown にまとめた（計画 13 §1.4）．
// 見出し・箇条書き・表・コードの枠・frontmatter まで同じ部品で出せる．

// 上にいる間のみ「新着 ↓」を出す
function updateNewArrival() {
  const btn = document.getElementById("new-arrival");
  btn.hidden = stickToBottom;
}

window.addEventListener("scroll", () => {
  const nearBottom = window.innerHeight + window.scrollY >= document.body.scrollHeight - 120;
  stickToBottom = nearBottom;
  updateNewArrival();
}, { passive: true });

document.getElementById("new-arrival").addEventListener("click", () => {
  stickToBottom = true;
  updateNewArrival();
  window.scrollTo({ top: document.body.scrollHeight, behavior: "smooth" });
});

// 戻るボタン：1つ上の階層のURLへ（履歴に頼らない・02 §3.1）
document.getElementById("day-back").addEventListener("click", () => {
  location.hash = "#/";
});

document.getElementById("back-button").addEventListener("click", () => {
  // 実行 → その実行があった日 → 日付一覧 の順に戻る
  if (currentRun && currentRun.date) location.hash = `#/day/${currentRun.date}`;
  else location.hash = "#/";
});

// 統計画面へのボタン（U11 で List の run 画面に移る Stats に向いた。統計は List の Stats で見る。
// 旧住所 #/run/…/stats もそのまま開けるので消していない）。
// 行き先は「その run の List の表示」（最後に見ていた表示は linkview.js の loadViewMemory）。
document.getElementById("stats-button").addEventListener("click", () => {
  const p = currentRun ? currentRun.project_name : current.project;
  const id = currentRun ? currentRun.run_id : current.id;
  if (p && id != null && id !== "") {
    const view = typeof loadViewMemory === "function" ? loadViewMemory() : "log";
    location.hash = `#/project/${encodeURIComponent(p)}/list/${encodeURIComponent(
      String(id)
    )}/${encodeURIComponent(view || "log")}`;
  }
});

// ■ 終了（メッセージ画面・計画書 A の D2）。
// 完了の処理は sai-link のデーモンが行うので、ここは要求を出すだけ。
document.getElementById("finalize-button").addEventListener("click", async () => {
  if (!currentRun) return;
  if (!confirm("この実行を終了します。よろしいですか？")) return;
  const res = await apiFetch(`${runApiBase()}/finalize`, { method: "POST" });
  const out = await res.json().catch(() => ({}));
  if (!res.ok) {
    alert(out.detail || "終了できませんでした");
    return;
  }
  alert(out.message || "終了を要求しました");
  await loadRunMeta();
});

// ⏸ 中断（U04 §3）。headless の実行に止まれと伝える。
document.getElementById("interrupt-button").addEventListener("click", async () => {
  if (!currentRun) return;
  const res = await apiFetch(`${runApiBase()}/interrupt`, { method: "POST" });
  const out = await res.json().catch(() => ({}));
  if (!res.ok) {
    alert(out.detail || "中断できませんでした");
    return;
  }
  if (out.status === "interrupted") saiToast("中断しました");
  else if (out.status === "still_working") {
    saiToast("まだ止まっていません。もう一度押すか、■ で終了してください");
  } else {
    saiToast(out.status || "中断しました");
  }
  await loadRunMeta();
});

// --- 版の確認と入れ替え（ⓘボタン） ---
// 画面の版は index.html が付けた app.js の ?v=（無ければ古い index.html から読んだ画面）。
const APP_SCRIPT_SRC = (document.currentScript && document.currentScript.src) || "";

// 保存済みの画面の倉庫かどうか（旧名 sai-phone-v… と新名 sai-web-v… の両方を見る・U17）
function isSavedScreenCache(name) {
  return name.startsWith("sai-web-v") || name.startsWith("sai-phone-v");
}

document.getElementById("version-button").addEventListener("click", async () => {
  const page = new URL(APP_SCRIPT_SRC || location.href).searchParams.get("v") || "版なし（古い画面）";
  let saved = "なし";
  try {
    const keys = (await caches.keys()).filter(isSavedScreenCache);
    if (keys.length) saved = keys.join(", ");
  } catch { /* 読めなければ「なし」 */ }
  const worker = navigator.serviceWorker && navigator.serviceWorker.controller ? "動いている" : "動いていない";
  let server = "読めません";
  try {
    const res = await apiFetch("/api/health");
    const b = await res.json();
    server = `${b.server || "?"}（${b.started_at || "?"} に起動）`;
  } catch { /* 読めなければそのまま */ }
  const ok = confirm(
    `画面: ${page}\n保存済みの画面: ${saved}\n通知の受け取り役: ${worker}\nMac のサーバー: ${server}\n\n` +
    "最新の画面に入れ替えますか？（通知の登録はそのまま）"
  );
  if (!ok) return;
  // 受け取り役（Service Worker）は登録を消さずに更新だけする（消すと通知の登録も消えるため）
  try {
    const reg = await navigator.serviceWorker.getRegistration();
    if (reg) await reg.update();
  } catch { /* 更新できなくても下で読み直す */ }
  // 保存済みの画面だけ消す（ダウンロードした音声の倉庫は消さない）
  try {
    const keys = (await caches.keys()).filter(isSavedScreenCache);
    await Promise.all(keys.map((k) => caches.delete(k)));
  } catch { /* 消せなくても読み直す */ }
  location.replace(`/?r=${Date.now()}${location.hash}`);
});

// --- 通知の登録（🔔ボタン） ---
// アプリを開いた直後ではなく，利用者が押した時に許可を求める．
// iOS は一度断られると再度聞けないため．
document.getElementById("notify-button").addEventListener("click", async () => {
  if (!("Notification" in window) || !("serviceWorker" in navigator)) {
    alert("この端末では通知を使えません");
    return;
  }
  const perm = await Notification.requestPermission();
  if (perm !== "granted") return;
  try {
    const reg = await navigator.serviceWorker.ready;
    const keyRes = await apiFetch("/api/push/key");
    if (!keyRes.ok) throw new Error("鍵が取れませんでした");
    const { public_key } = await keyRes.json();
    const sub = await reg.pushManager.subscribe({
      userVisibleOnly: true,
      applicationServerKey: urlBase64ToUint8Array(public_key),
    });
    const res = await apiFetch("/api/push/subscribe", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(sub.toJSON()),
    });
    if (!res.ok) throw new Error("登録できませんでした");
    const body = await res.json().catch(() => ({}));
    // 登録と同時に Mac がテスト通知を 1 通送る（届かなければ Mac 側のログを見る）
    alert(body.test_sent
      ? "通知を受け取ります（テスト通知を送りました）"
      : "通知を受け取ります（ただしテスト通知を送れませんでした）");
  } catch (e) {
    alert(`通知の登録に失敗しました: ${e.message || e}`);
  }
});

function urlBase64ToUint8Array(base64) {
  const pad = "=".repeat((4 - (base64.length % 4)) % 4);
  const bin = atob(base64.replace(/-/g, "+").replace(/_/g, "/") + pad);
  const out = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i);
  return out;
}

// --- 送信待ち（圏外で送った文章を残し，復帰後に自動で送る） ---
// 最大5件まで．古い物から捨てる．
// 溜め込んで意図しない指示が後から飛ぶ方が危険なため．
const PENDING_KEY = "sai_web_pending_sends";
const PENDING_MAX = 5;

function loadPendingSends() {
  try {
    const raw = localStorage.getItem(PENDING_KEY);
    const arr = raw ? JSON.parse(raw) : [];
    if (!Array.isArray(arr)) return [];
    // 旧形式（session_id）は捨てる（実行の住所は project/id になったため）
    return arr.filter((p) => p && typeof p.key === "string");
  } catch { return []; }
}

function savePendingSends(arr) {
  try { localStorage.setItem(PENDING_KEY, JSON.stringify(arr.slice(-PENDING_MAX))); }
  catch { /* 保存できなくても動く */ }
}

function currentRunKey() {
  return currentRun ? `${currentRun.project_name}/${currentRun.run_id}` : "";
}

function queuePendingSend(text) {
  const arr = loadPendingSends();
  arr.push({ key: currentRunKey(), text, at: Date.now() });
  savePendingSends(arr);
  renderPendingMarks();
}

function renderPendingMarks() {
  document.querySelectorAll(".pending-send").forEach((el) => el.remove());
  const key = currentRunKey();
  const marks = [
    ...loadPendingSends().map((p) => ({ ...p, label: "送信待ち" })),
    ...takeReflected(loadReflectedSends(), key, [], Date.now()).map((p) => ({ ...p, label: "反映待ち" })),
  ].filter((p) => p.key === key).sort((a, b) => (a.at || 0) - (b.at || 0));
  if (marks.length === 0) return;
  const box = document.getElementById("chat-list");
  for (const p of marks) {
    const div = document.createElement("div");
    div.className = "bubble-user pending-send";
    const body = document.createElement("div");
    body.textContent = p.text;
    const mark = document.createElement("div");
    mark.className = "pending-mark";
    mark.textContent = p.label;
    div.append(body, mark);
    box.appendChild(div);
  }
  if (stickToBottom) window.scrollTo(0, document.body.scrollHeight);
}

async function flushPendingSends() {
  if (current.name !== "run") return;
  const key = currentRunKey();
  const arr = loadPendingSends();
  const mine = arr.filter((p) => p.key === key);
  if (mine.length === 0) return;
  const rest = arr.filter((p) => p.key !== key);
  const failed = [];
  const sent = [];
  for (const p of mine) {
    try {
      const res = await apiFetch(`${runApiBase()}/send`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ text: p.text }),
      });
      if (!res.ok) { failed.push(p); break; }
      sent.push(p);
    } catch { failed.push(p); break; }
  }
  savePendingSends([...rest, ...failed]);
  if (sent.length) queueReflectedSends(sent.map((p) => p.text));
  document.querySelectorAll(".pending-send").forEach((el) => el.remove());
  renderPendingMarks();
}

// --- 反映待ち（送信は届いたが，cc が取り込んで記録に現れるまでの文） ---
// 送信の成功のあと，取り込まれて記録に現れる（user の項目になる）まで残す。
// 取り込まれずに終わったときに残り続けないよう 10 分で消す。
const REFLECT_KEY = "sai_web_sent_unreflected";
const REFLECT_MAX = 5;
const REFLECT_TTL_MS = 10 * 60 * 1000;

function loadReflectedSends() {
  try {
    const raw = localStorage.getItem(REFLECT_KEY);
    const arr = raw ? JSON.parse(raw) : [];
    if (!Array.isArray(arr)) return [];
    return arr.filter((p) => p && typeof p.key === "string" && typeof p.text === "string");
  } catch { return []; }
}

function saveReflectedSends(arr) {
  try { localStorage.setItem(REFLECT_KEY, JSON.stringify(arr.slice(-REFLECT_MAX))); }
  catch { /* 保存できなくても動く */ }
}

// 期限切れを落とし，読んだ項目に同じ文（user）があれば 1 件ずつ消す（純関数・テストから使う）
function takeReflected(list, key, items, now) {
  const rest = list.filter((p) => now - Number(p.at || 0) <= REFLECT_TTL_MS);
  for (const item of items) {
    if (!item || item.kind !== "user") continue;
    const text = String(item.text || "").trim();
    const i = rest.findIndex((p) => p.key === key && p.text.trim() === text);
    if (i >= 0) rest.splice(i, 1);
  }
  return rest;
}

function queueReflectedSends(texts) {
  const now = Date.now();
  const key = currentRunKey();
  const arr = takeReflected(loadReflectedSends(), key, [], now);
  for (const text of texts) arr.push({ key, text, at: now });
  saveReflectedSends(arr);
  renderPendingMarks();
}

// 記録から読んだ項目で反映待ちを消す。消したら true（印を描き直す）
function settleReflectedSends(items) {
  const before = loadReflectedSends();
  const after = takeReflected(before, currentRunKey(), items, Date.now());
  if (after.length === before.length) return false;
  saveReflectedSends(after);
  return true;
}

// --- 入力欄（テキスト送信） ---

let sending = false;

document.getElementById("send-button").addEventListener("click", async () => {
  if (sending || current.name !== "run") return;
  const ta = document.getElementById("chat-input");
  const text = (ta.value || "").trim();
  if (!text) return;
  sending = true;
  try {
    let res;
    try {
      res = await apiFetch(`${runApiBase()}/send`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ text }),
      });
    } catch {
      // 通信失敗時は送信待ちに残し，復帰後に自動で送る
      queuePendingSend(text);
      ta.value = "";
      autoGrow();
      return;
    }
    if (!res.ok) {
      const err = await res.json().catch(() => ({}));
      // 質問待ちなどで断られたときはサーバーの説明をそのまま出す（C6）
      if (res.status === 409) saiToast(err.detail || "今は送れません");
      else alert(err.detail || "送信できなかった");
      return;
    }
    // 記録に現れるまで「反映待ち」として残す（仮の吹き出しは openRun で消えるため）
    queueReflectedSends([text]);
    ta.value = "";
    autoGrow();
  } finally {
    sending = false;
  }
});

function autoGrow() {
  const ta = document.getElementById("chat-input");
  ta.style.height = "auto";
  ta.style.height = Math.min(ta.scrollHeight, parseFloat(getComputedStyle(ta).maxHeight)) + "px";
}

document.getElementById("chat-input").addEventListener("input", autoGrow);

// --- 検索（日付一覧に付ける・02 §3.2） ---

document.getElementById("search-toggle").addEventListener("click", () => {
  const row = document.getElementById("search-row");
  row.hidden = !row.hidden;
  if (!row.hidden) document.getElementById("search-input").focus();
  else {
    document.getElementById("search-input").value = "";
    searchText = "";
    void loadDays();
  }
});

document.getElementById("search-input").addEventListener("input", (e) => {
  searchText = e.target.value || "";
  if (current.name === "days") void loadDays();
});

// --- 音声入力（Web Speech API・Agent の入力欄と task_text で使い回す部品・計画 15 §1.5） ---

/**
 * 🎤 ボタンとテキスト欄を音声入力で結ぶ（部品・新しくは作らない・計画 15 §1.5）。
 * 聞き取った文字はテキスト欄に追記するだけで自動送信はしない。
 * 使えない環境ではボタンを隠し，エラーで画面を止めない。
 * 戻り値は部品に仕込んだ recognizer（使わなければ null）。
 */
function attachVoiceInput(btn, ta, onGrow) {
  if (!btn || !ta) return null;
  const SR = window.SpeechRecognition || window.webkitSpeechRecognition;
  if (!SR) {
    // 使えない環境では隠す．エラーを出して止めない．
    btn.hidden = true;
    return null;
  }
  let recognizing = false;
  const recognizer = new SR();
  recognizer.lang = "ja-JP";
  recognizer.interimResults = false;
  recognizer.onresult = (e) => {
    let text = "";
    for (const r of e.results) text += r[0].transcript;
    // 聞き取った文字は入力欄に入れるだけ．自動送信はしない．
    ta.value = (ta.value ? ta.value + " " : "") + text;
    if (typeof onGrow === "function") onGrow();
  };
  recognizer.onend = () => {
    recognizing = false;
    btn.classList.remove("recording");
  };
  recognizer.onerror = () => {
    recognizing = false;
    btn.classList.remove("recording");
  };
  btn.addEventListener("click", () => {
    if (recognizing) {
      recognizer.stop();
      return;
    }
    try {
      recognizer.start();
      recognizing = true;
      btn.classList.add("recording");
    } catch {
      // 連続タップ時は黙って無視する
    }
  });
  return recognizer;
}
window.attachVoiceInput = attachVoiceInput;

// Agent タブの入力欄への結び付け（cardform.js は自分で呼ぶ・計画 15 §1.5）
function setupMic() {
  attachVoiceInput(
    document.getElementById("mic-button"),
    document.getElementById("chat-input"),
    autoGrow
  );
}

// Service Worker（画面の部品の取得を助ける．会話データは保存しない）
if ("serviceWorker" in navigator) {
  navigator.serviceWorker.register("/sw.js").catch(() => {});
}

// --- 通知を押したときの飛び先（sw.js の notificationclick が渡す） ---
// iOS の PWA は通知から画面を切り替えられないため，飛び先を受け取ってここで移る。
// 飛び先を覚える倉庫の名前は sw.js の OPEN_CACHE / OPEN_KEY と同じ。
const NOTIFY_OPEN_CACHE = "sai-web-open";
const NOTIFY_OPEN_KEY = "/__notification_open";
// 倉庫に残った飛び先を使う期限（押してからアプリが開くまで）
const NOTIFY_OPEN_MAX_MS = 2 * 60 * 1000;

/** 通知の飛び先（"/#/run/…"）へ移る。自分のアプリの住所（#/…）だけを受け付ける。 */
function openFromNotification(url) {
  const hash = String(url || "").replace(/^\//, "");
  // 使い終わった飛び先は消す（次に開いたときにまた飛ばないように）
  if ("caches" in window) caches.open(NOTIFY_OPEN_CACHE).then((c) => c.delete(NOTIFY_OPEN_KEY)).catch(() => {});
  if (hash.startsWith("#/") && location.hash !== hash) location.hash = hash;
}

if ("serviceWorker" in navigator) {
  // アプリが裏で開いていたとき
  navigator.serviceWorker.addEventListener("message", (e) => {
    if (e.data && e.data.type === "open") openFromNotification(e.data.url);
  });
  // addEventListener で受けるときは受け取り開始の合図が要る（無いと届いた連絡が溜まったままになる）
  navigator.serviceWorker.startMessages();
}

// アプリが閉じていて，通知から開き直したとき（iOS は / で起動することがある）
async function openPendingNotification() {
  if (!("caches" in window)) return;
  try {
    const cache = await caches.open(NOTIFY_OPEN_CACHE);
    const res = await cache.match(NOTIFY_OPEN_KEY);
    if (!res) return;
    const data = await res.json();
    if (Date.now() - Number(data.at) <= NOTIFY_OPEN_MAX_MS) openFromNotification(data.url);
    else await cache.delete(NOTIFY_OPEN_KEY);
  } catch { /* 読めなければ普段どおり開く */ }
}

/**
 * 飛び先を何度か見に行く（起動・前面に戻った直後は，倉庫への書き込みがまだ終わっていないことがある）。
 */
function watchPendingNotification() {
  for (const ms of [0, 400, 1200, 2500]) setTimeout(() => void openPendingNotification(), ms);
}
watchPendingNotification();
// アプリが裏から前に戻ったとき（iOS は裏で止まっている間の連絡を捨てることがある）
document.addEventListener("visibilitychange", () => {
  if (document.visibilityState === "visible") watchPendingNotification();
});
window.addEventListener("pageshow", (e) => {
  if (e.persisted) watchPendingNotification();
});

setupMic();
// ログインの状態を先に読んでから画面を開く（多人数モードの判定に使う・U07）。
// account.js が無いとき（単独モードの古い配布）でも今までどおり開けるようにする。
if (typeof acLoadMe === "function") acLoadMe().finally(() => route());
else route();
