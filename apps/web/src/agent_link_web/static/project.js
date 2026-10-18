// agent-link-web の Project タブの本体。
//
// 画面は2つ:
//   プロジェクト一覧   #/projects
//   カード型 Dashboard #/project/<name>
// 中身は Apple のヘルスケア / スクリーンタイムのように、余白の多いカードを
// 縦に積んだ形（style.css の .dash-card）。グラフは SVG を手描きで作る
// （外部ライブラリは入れない）。データはすべて /api/projects/… 経由
// で，agent-link-web 自身は DB を開かない。
// DOM を使わない小さな関数は末尾で node に渡す（tests/test_project.js）。
"use strict";

// --- 定数 ---

// 期間の選択をタブごとに覚える
const DASH_PERIOD_KEY = "agent_link_web_dash_period";
const DASH_PERIODS = [
  ["week", "Week"],
  ["month", "Month"],
  ["all", "All time"],
];
// モデル別ドーナツの色（5色）。色は CSS 変数。暗い画面では自動的に変わる。
const MODEL_COLORS = ["var(--series-1)", "var(--series-2)", "var(--series-3)", "var(--series-4)", "var(--series-5)", "var(--series-6)"];
const WEEKDAY_EN = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
const MONTH_EN = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
// 一覧のランプ（稼働中は緑・停止は輪郭のみ）
const LAMP_ON = "var(--ok)";

// --- DOM を使わない小さな関数（node のテストからも使う） ---

/** DB の文字列（タイムゾーン無し = UTC）を時刻（ms）にする。読めなければ null。 */
function parseStamp(value) {
  if (!value) return null;
  let s = String(value).trim();
  if (!s) return null;
  // "2026-09-23 10:28:21" → ISO にそろえる（DB の日時は UTC とみなす）
  const m = s.match(/^(\d{4}-\d{2}-\d{2})[ T](\d{2}:\d{2}(?::\d{2})?(?:\.\d+)?)$/);
  if (m) s = `${m[1]}T${m[2]}Z`;
  else if (/^\d{4}-\d{2}-\d{2}$/.test(s)) s = `${s}T00:00:00Z`;
  else if (!/[zZ]|[+-]\d{2}:?\d{2}$/.test(s)) s = `${s}Z`;
  const t = Date.parse(s);
  return Number.isNaN(t) ? null : t;
}

/** 「たった今」「5分前」「3時間前」「2日前」「9/12」。 */
function relativeTime(iso, nowMs) {
  const t = parseStamp(iso);
  if (t == null) return "";
  const now = Number.isFinite(nowMs) ? nowMs : Date.now();
  const diff = Math.max(0, now - t);
  const min = Math.floor(diff / 60000);
  if (min < 1) return "just now";
  if (min < 60) return `${min}m ago`;
  const h = Math.floor(min / 60);
  if (h < 24) return `${h}h ago`;
  const d = Math.floor(h / 24);
  if (d < 7) return `${d}d ago`;
  const dt = new Date(t);
  return `${dt.getMonth() + 1}/${dt.getDate()}`;
}

/**
 * ドーナツ用に上位 N 件＋「その他」へまとめる。
 * rows は {label, value}（または [label, value]）の配列（多い順でも入れてもよい）。
 * 返り値は {label, value, ratio}。合計が 0 なら空配列。
 */
function donutSlices(rows, topN) {
  const list = (rows || [])
    .map((r) => {
      const pair = Array.isArray(r) ? { label: r[0], value: r[1] } : { label: r && r.label, value: r && r.value };
      const v = Number(pair.value);
      return { label: String(pair.label == null ? "" : pair.label), value: Number.isFinite(v) ? v : 0 };
    })
    .filter((x) => x.value > 0)
    .sort((a, b) => b.value - a.value);
  const total = list.reduce((a, x) => a + x.value, 0);
  if (total <= 0) return [];
  const top = list.slice(0, Math.max(0, topN));
  const rest = list.slice(Math.max(0, topN));
  const out = top.map((x) => ({ ...x, ratio: x.value / total }));
  if (rest.length > 0) {
    const v = rest.reduce((a, x) => a + x.value, 0);
    out.push({ label: "Other", value: v, ratio: v / total });
  }
  return out;
}

/** 実行時間（秒は省く・「4時間45分」）。 */
function shortDuration(ms) {
  const total = Math.floor(Number(ms) / 1000);
  if (!Number.isFinite(total) || total < 0) return "0";
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  if (h > 0) return `${h}h ${m}m`;
  if (m > 0) return `${m}m`;
  return `${total}s`;
}

/** 時刻の文字列 → 端末の現地時刻の「HH:MM」（DB の日時は UTC のため現地時刻に直す）。 */
function clockOf(stamp) {
  const t = parseStamp(stamp);
  if (t == null) return "";
  return new Date(t).toLocaleTimeString("en-US", { hour: "2-digit", minute: "2-digit" });
}

/** 棒を押したときに出す1行（「9/22（月） 1.2M」）。 */
function chartNoteText(row, metric) {
  const date = String((row && row.date) || "");
  if (!date) return "";
  const md = `${Number(date.slice(5, 7))}/${Number(date.slice(8, 10))}`;
  const wd = WEEKDAY_EN[new Date(`${date}T00:00:00`).getDay()] || "";
  const n = Number((row && row.tokens) || 0);
  // 数字の整形は stats.js の関数（ブラウザでだけ読み込まれるため typeof で確かめる）
  const v = metric === "count"
    ? `${Number((row && row.executions) || 0)} runs`
    : typeof formatTokens === "function" ? formatTokens(n) : String(n);
  return `${md} (${wd}) ${v}`;
}

/** 横軸ラベル（今週=曜日・今月=1/5/10/…・全期間=月が変わる所だけ）。 */
function axisLabels(period, daily) {
  const rows = daily || [];
  return rows.map((row, i) => {
    const date = String(row.date || "");
    const day = Number(date.slice(8, 10));
    if (period === "week") {
      return WEEKDAY_EN[new Date(`${date}T00:00:00`).getDay()] || "";
    }
    if (period === "month") {
      return [1, 5, 10, 15, 20, 25, 30].includes(day) ? String(day) : "";
    }
    const cur = date.slice(0, 7);
    const prev = i > 0 ? String(rows[i - 1].date || "").slice(0, 7) : null;
    return i === 0 || cur !== prev ? MONTH_EN[Number(cur.slice(5, 7)) - 1] : "";
  });
}

/** モデル名は `/` の後ろだけ（「xiaomi/mimo」→「mimo」）。 */
function shortModel(name) {
  const s = String(name || "");
  const i = s.lastIndexOf("/");
  return i >= 0 ? s.slice(i + 1) : s;
}

/**
 * ツリー（`GET …/tree`）の todo フォルダの中身（新形式 todo の一覧）。
 * 旧形式カード（`card-*.md`）は見ない。無ければ空配列。
 */
function todoFolderRuns(items) {
  const out = [];
  for (const node of items || []) {
    if (!node || typeof node !== "object" || node.type !== "folder") continue;
    if (String(node.id || "") !== "todo") continue;
    for (const c of node.children || []) {
      if (c && c.type === "run" && c.run_id != null) out.push(c);
    }
  }
  return out;
}

// --- 画面の状態 ---

let dashName = "";            // 開いているプロジェクト名
let dashData = null;          // 最新の Dashboard（GET の応答そのまま）
let todoRuns = [];            // 未実行の todo run（ツリーの todo フォルダの中身）
let todoLoaded = false;       // todo を読んだか（描き直しの合図）
let chartMetric = "tokens";   // 日ごとのグラフの指標（tokens | count）
let chartNote = "";           // 棒を押して出した1行
let daemonBusy = false;       // 常駐操作の送信中
let runBusy = false;          // カード実行の送信中
let msgText = "";             // カード位置に出す1行（成功の知らせ・失敗の detail）
let msgTimer = null;

function periodValue() {
  try {
    const p = sessionStorage.getItem(DASH_PERIOD_KEY);
    if (p === "week" || p === "month" || p === "all") return p;
  } catch { /* プライベートブラウズでも動くように */ }
  return "week";
}

function savePeriod(p) {
  try { sessionStorage.setItem(DASH_PERIOD_KEY, p); } catch { /* 保存できなくても動く */ }
}

function isListScreen() {
  return location.hash === "#/projects";
}

function isDashScreen() {
  // "#/projects" は一覧の住所（この判定には入らない）
  return location.hash.startsWith("#/project/");
}

// --- 一覧（#/projects） ---

/** list === null は「読み込み中」（前の表示は消さない）。 */
function renderProjects(list) {
  const box = document.getElementById("projects-list");
  if (!box) return;
  if (list === null) {
    if (!box.childElementCount) box.innerHTML = "<p class='dash-empty'>Loading…</p>";
    return;
  }
  box.innerHTML = "";
  if (list.length === 0) {
    // まだ 1 つも無いときは、案内だけ出す
    const box2 = document.createElement("div");
    box2.className = "sai-empty";
    box2.appendChild(el("p", "sai-empty-title", "No projects yet"));
    box2.appendChild(el("p", "sai-empty-body", "Run `agent-link init` on the Mac and the project will show up here."));
    box.appendChild(box2);
    return;
  }
  for (const p of list) box.appendChild(projectRow(p));
}

/** 一覧の 1 枚が表すもの（node のテストで確かめる）。 */
function projectRowFacts(p) {
  const d = p || {};
  const running = Number(d.running) || 0;
  const parts = [];
  parts.push(d.daemon_alive ? "Daemon running" : "Daemon stopped");
  if (running > 0) parts.push(`${running} running`);
  const ago = relativeTime(d.last_executed_at);
  if (ago) parts.push(`Last ran ${ago}`);
  return {
    name: String(d.name || ""),
    alive: !!d.daemon_alive,
    running,
    ago,
    usable: !!(d.root_exists && d.db_exists),
    line2: parts.join(" · "),
  };
}

function projectRow(p) {
  const facts = projectRowFacts(p);
  // 多人数モードでは行の外に起動 / 停止のボタンを置くため、1 枚を包む
  // （button の中に button は置けない）
  const item = document.createElement("div");
  item.className = "proj-item";

  const btn = document.createElement("button");
  btn.className = "proj-row";
  // root か db が無い行は薄くして押せない
  if (!facts.usable) btn.classList.add("dim");

  const line1 = document.createElement("div");
  line1.className = "proj-line1";
  const lamp = document.createElement("span");
  lamp.className = "proj-lamp" + (facts.alive ? " on" : "");
  lamp.setAttribute("aria-hidden", "true");
  const name = document.createElement("span");
  name.className = "proj-name";
  name.textContent = facts.name;
  const time = document.createElement("span");
  time.className = "proj-time";
  time.textContent = facts.ago;
  const chev = document.createElement("span");
  chev.className = "proj-chev";
  chev.textContent = "›";
  line1.append(lamp, name);
  line1.append(time, chev);

  const line2 = document.createElement("div");
  line2.className = "proj-line2";
  if (!p.root_exists) line2.textContent = "Folder not found";
  else if (!p.db_exists) line2.textContent = "No run records";
  else line2.textContent = facts.line2;

  btn.append(line1, line2);
  if (facts.usable) {
    btn.addEventListener("click", () => {
      // 覚えた画面へ移るのは Dashboard 側の open がやる（List だけ，今までの通り）
      location.hash = `#/project/${encodeURIComponent(p.name)}`;
    });
  } else {
    btn.disabled = true;
  }
  item.appendChild(btn);
  return item;
}

async function loadProjects() {
  if (!isListScreen()) return;
  let data;
  try {
    const res = await apiFetch("/api/projects");
    const body = await res.json().catch(() => ({}));
    if (!res.ok) throw new Error(body.detail || "Could not load.");
    data = body;
  } catch (e) {
    // 失敗しても前の表示は残し，帯だけ出す
    window.SaiShell.fail();
    return;
  }
  renderProjects(data.projects || []);
  window.SaiShell.ok();
}

// --- Dashboard（#/project/<name>） ---

function dashCardsBox() {
  return document.querySelector("#dashboard-body .dash-cards");
}

function renderMsg() {
  const msg = document.querySelector("#dashboard-body .dash-msg");
  if (!msg) return;
  msg.textContent = msgText;
  msg.hidden = !msgText;
}

/** カードの位置に1行出す（成功の知らせは数秒で消す・失敗の detail も同じ場所）。 */
function showDashMessage(text) {
  msgText = String(text || "");
  renderMsg();
  if (msgTimer) clearTimeout(msgTimer);
  if (msgText) {
    msgTimer = setTimeout(() => {
      msgText = "";
      renderMsg();
    }, 8000);
  }
}

function dashCard(cls) {
  const card = document.createElement("section");
  card.className = cls ? `dash-card ${cls}` : "dash-card";
  return card;
}

function cardTitle(text) {
  return el("h3", "dash-card-title", text);
}

async function openDashboard(name) {
  // 最後に見ていた画面をプロジェクトごとに見る。
  // List / Workflow / 設定を選んでいたプロジェクトはそちらへ移る（linkview.js が覚えておく）。
  if (typeof rememberedMode === "function" && rememberedMode(name) !== "dashboard") {
    location.replace(modeHash(name, rememberedMode(name)));
    return;
  }
  // 上部の Dashboard | Workflow | List | 設定 の切り替えを置く
  if (typeof fillLinkSeg === "function") fillLinkSeg("dash-mode-seg", "dashboard", name);
  dashName = name;
  dashData = null;
  todoRuns = [];
  todoLoaded = false;
  chartMetric = "tokens";
  chartNote = "";
  msgText = "";
  if (msgTimer) { clearTimeout(msgTimer); msgTimer = null; }

  document.getElementById("dash-title").textContent = name;
  const lamp = document.getElementById("dash-lamp");
  lamp.className = "proj-lamp";

  const body = document.getElementById("dashboard-body");
  body.innerHTML = "";
  const msg = document.createElement("div");
  msg.className = "dash-msg";
  msg.hidden = true;
  const cards = document.createElement("div");
  cards.className = "dash-cards";
  cards.innerHTML = "<p class='dash-empty'>Loading…</p>";
  const todo = document.createElement("div");
  todo.id = "dash-todo";
  todo.innerHTML = "<section class='dash-card'><p class='dash-empty'>Loading…</p></section>";
  body.append(msg, cards, todo);
  dashApplyWide();

  // Dashboard と todo は同時に読む（todo は開いたときだけ読む）
  await Promise.all([loadDashboard(), loadTodo()]);
}

// --- 広い画面ではカードを 2 列に並べる（style.css は触らず JS のインラインスタイルで組む） ---

function dashApplyWide() {
  const cards = document.querySelector("#dashboard-body .dash-cards");
  if (!cards) return;
  const wide = typeof saiIsWide === "function" ? saiIsWide() : false;
  if (wide) {
    cards.style.display = "grid";
    cards.style.gridTemplateColumns = "repeat(2, minmax(0, 1fr))";
    cards.style.gap = "12px";
    cards.style.alignItems = "start";
  } else {
    cards.style.display = "";
    cards.style.gridTemplateColumns = "";
    cards.style.alignItems = "";
  }
}

async function loadDashboard() {
  if (!dashName || !isDashScreen()) return;
  let data;
  try {
    const res = await apiFetch(
      `/api/projects/${encodeURIComponent(dashName)}/dashboard?period=${encodeURIComponent(periodValue())}`
    );
    const body = await res.json().catch(() => ({}));
    if (!res.ok) throw new Error(body.detail || "Could not load.");
    data = body;
  } catch (e) {
    // 失敗は detail をカードの位置に出す
    showDashMessage(e && e.message ? e.message : "Could not load.");
    window.SaiShell.fail();
    return;
  }
  dashData = data;
  renderDashboard();
  window.SaiShell.ok();
}

async function loadTodo() {
  if (!dashName || !isDashScreen()) return;
  let items;
  try {
    // todo 一覧は `GET …/tree` の todo フォルダの中身（別 CLI は使わない）
    const res = await apiFetch(`/api/projects/${encodeURIComponent(dashName)}/tree`);
    const body = await res.json().catch(() => ({}));
    if (!res.ok) throw new Error(body.detail || "Could not load.");
    items = body.items;
  } catch (e) {
    showDashMessage(e && e.message ? e.message : "Could not load todo.");
    window.SaiShell.fail();
    return;
  }
  todoRuns = todoFolderRuns(items);
  todoLoaded = true;
  renderTodo();
  window.SaiShell.ok();
}

function renderDashboard() {
  const box = dashCardsBox();
  if (!box) return;
  const d = dashData;
  if (!d) return; // 読み込み中の表示はそのまま残す
  const lamp = document.getElementById("dash-lamp");
  if (lamp) lamp.className = "proj-lamp" + (d.daemon_alive ? " on" : "");
  box.innerHTML = "";
  box.append(
    statusCard(d),
    periodCard(d),
    summaryCard(d),
    chartCard(d),
    modelCard(d),
    blockCard(d),
    recentCard(d)
  );
  // todo は先に読んでいるなら状態（常駐の有無）に合わせて描き直す
  if (todoLoaded) renderTodo();
}

// 1. 常駐の状態と起動 / 停止ボタン
function statusCard(d) {
  const card = dashCard("dash-status");
  const left = document.createElement("span");
  left.className = "dash-status-text";
  const lamp = document.createElement("span");
  lamp.className = "proj-lamp" + (d.daemon_alive ? " on" : "");
  lamp.setAttribute("aria-hidden", "true");
  left.append(lamp, document.createTextNode(d.daemon_alive ? " Running" : " Stopped"));

  const btn = document.createElement("button");
  btn.className = "dash-btn" + (d.daemon_alive ? " stop" : "");
  btn.textContent = d.daemon_alive ? "Stop" : "Start";
  btn.disabled = daemonBusy;
  btn.addEventListener("click", onDaemonClick);
  card.append(left, btn);
  // 端末に慣れていない人向けの説明（何を押すとどうなるか）
  const note = document.createElement("div");
  note.className = "dash-hint";
  note.textContent = d.daemon_alive
    ? "The daemon keeps running on the Mac. If you stop it, running cards stop too."
    : "The daemon is stopped. Start it to run this project's cards.";
  card.appendChild(note);
  return card;
}

async function onDaemonClick() {
  if (daemonBusy || !dashData) return;
  const action = dashData.daemon_alive ? "stop" : "start";
  const question = action === "stop"
    ? `Stop the daemon for ${dashName}?`
    : `Start the daemon for ${dashName}?`;
  if (!(await saiConfirm({ title: question, danger: action === "stop" }))) return;
  daemonBusy = true;
  renderDashboard();
  try {
    const res = await apiFetch(`/api/projects/${encodeURIComponent(dashName)}/daemon`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ action }),
    });
    const body = await res.json().catch(() => ({}));
    if (!res.ok) throw new Error(body.detail || "Could not send.");
    showDashMessage(body.message || (action === "stop" ? "Asked the daemon to stop" : "Asked the daemon to start"));
    window.SaiShell.ok();
  } catch (e) {
    showDashMessage(e && e.message ? e.message : "Could not send.");
    window.SaiShell.fail();
  } finally {
    daemonBusy = false;
    await loadDashboard();
  }
}

// 2. 期間の切り替え（今週 / 今月 / 全期間）
function periodCard() {
  const card = dashCard("dash-period");
  const seg = document.createElement("div");
  seg.className = "dash-seg";
  const now = periodValue();
  for (const [value, label] of DASH_PERIODS) {
    const b = document.createElement("button");
    b.textContent = label;
    b.className = value === now ? "active" : "";
    b.addEventListener("click", () => {
      if (periodValue() === value) return;
      savePeriod(value);
      chartNote = "";
      msgText = "";
      renderMsg();
      void loadDashboard();
    });
    seg.appendChild(b);
  }
  card.appendChild(seg);
  return card;
}

// 3. 数字のまとめ（2×2）
function summaryCard(d) {
  const s = d.summary || {};
  const card = dashCard("dash-summary");
  const grid = document.createElement("div");
  grid.className = "dash-grid";
  const items = [
    [`${Number(s.executions) || 0}`, "Runs"],
    [`${Math.round((Number(s.success_rate) || 0) * 100)}%`, "Success rate"],
    [formatTokens(s.tokens_total), "Tokens"],
    [shortDuration(s.duration_ms), "Duration"],
  ];
  for (const [value, label] of items) {
    const cell = document.createElement("div");
    cell.className = "dash-cell";
    cell.appendChild(el("div", "dash-big", value));
    cell.appendChild(el("div", "dash-big-label", label));
    grid.appendChild(cell);
  }
  card.appendChild(grid);

  // 費用・失敗・実行中（0 の項目は省く）
  const parts = [];
  const cost = Number(s.cost_usd) || 0;
  if (cost > 0) parts.push(`cost ${formatMoney(cost)}`);
  if (Number(s.failed) > 0) parts.push(`failed ${Number(s.failed)}`);
  if (Number(s.running) > 0) parts.push(`running ${Number(s.running)}`);
  if (parts.length) card.appendChild(el("div", "dash-sub", parts.join(" · ")));
  return card;
}

// 4. 日ごとのグラフ（token / 回数）
function chartCard(d) {
  const card = dashCard("dash-chart-card");
  const head = document.createElement("div");
  head.className = "dash-card-head";
  head.appendChild(cardTitle(chartMetric === "tokens" ? "Tokens per day" : "Runs per day"));
  const seg = document.createElement("div");
  seg.className = "dash-seg small";
  for (const [value, label] of [["tokens", "token"], ["count", "runs"]]) {
    const b = document.createElement("button");
    b.textContent = label;
    b.className = value === chartMetric ? "active" : "";
    b.addEventListener("click", () => {
      if (chartMetric === value) return;
      chartMetric = value;
      chartNote = "";
      renderDashboard();
    });
    seg.appendChild(b);
  }
  head.appendChild(seg);
  card.appendChild(head);
  card.appendChild(el("div", "dash-chart-note", chartNote));
  card.appendChild(dailySvg(d));
  return card;
}

function dailySvg(d) {
  const daily = Array.isArray(d.daily) ? d.daily : [];
  const labels = axisLabels(d.period || periodValue(), daily);
  const unit = chartMetric === "count" ? "count" : "tokens";
  return saiDailyBars({
    rows: daily,
    valueOf: (row) =>
      Number(chartMetric === "count" ? row.executions : row.tokens) || 0,
    labelOf: (row, i) => labels[i] || "",
    unit,
    what: chartMetric === "count" ? "Runs per day" : "Tokens per day",
    onPick: (row) => {
      chartNote = chartNoteText(row, chartMetric);
      renderDashboard();
    },
  });
}

// 5. モデル別の内訳（ドーナツ・上位 4 件＋その他）
function modelCard(d) {
  const card = dashCard("dash-model");
  card.appendChild(cardTitle("By model"));
  const slices = donutSlices(
    (d.by_model || []).map((m) => ({
      label: m.model || "(unknown)",
      value: Number(m.tokens) || 0,
    })),
    4
  );
  if (slices.length === 0) {
    card.appendChild(el("p", "dash-empty", "No data"));
    return card;
  }
  const R = 38;
  const C = 2 * Math.PI * R;
  const total = slices.reduce((a, s) => a + s.value, 0);
  const wrap = document.createElement("div");
  wrap.className = "dash-donut-wrap";
  const svg = svgEl("svg", {
    viewBox: "0 0 100 100",
    class: "dash-donut",
    role: "img",
    "aria-label":
      `Donut chart of token share by model. Total ${formatTokens(total)} tokens` +
      `. Largest is ${shortModel(slices[0].label)} ${formatTokens(slices[0].value)}` +
      ` (${Math.round(slices[0].ratio * 100)}%)`,
  });
  let start = 0;
  slices.forEach((s, i) => {
    const len = Math.max(s.ratio * C - 1.5, 0.5); // ギャップは 1.5 単位
    svg.appendChild(svgEl("circle", {
      cx: "50",
      cy: "50",
      r: String(R),
      fill: "none",
      stroke: MODEL_COLORS[i % MODEL_COLORS.length],
      "stroke-width": "14",
      "stroke-dasharray": `${len.toFixed(2)} ${(C - len).toFixed(2)}`,
      "stroke-dashoffset": String(-start * C),
      transform: "rotate(-90 50 50)",
    }));
    start += s.ratio;
  });
  wrap.appendChild(svg);
  const center = document.createElement("div");
  center.className = "dash-donut-center";
  center.appendChild(el("strong", "dash-donut-total", formatTokens(total)));
  center.appendChild(el("span", "dash-donut-cap", "Total"));
  wrap.appendChild(center);
  card.appendChild(wrap);

  const legend = document.createElement("div");
  legend.className = "dash-legend";
  slices.forEach((s, i) => {
    const row = document.createElement("div");
    row.className = "dash-leg";
    const dot = document.createElement("span");
    dot.className = "dash-dot";
    dot.style.background = MODEL_COLORS[i % MODEL_COLORS.length];
    row.append(
      dot,
      el("span", "dash-leg-name", shortModel(s.label)),
      el("span", "dash-leg-pct", `${Math.round(s.ratio * 100)}%`),
      el("span", "dash-leg-num", formatTokens(s.value))
    );
    legend.appendChild(row);
  });
  card.appendChild(legend);
  return card;
}

// 6. ブロック別の内訳（横棒・上位 8 件）
function blockCard(d) {
  const card = dashCard("dash-blocks");
  card.appendChild(cardTitle("By block"));
  const rows = (d.by_block || []).slice(0, 8);
  if (rows.length === 0) {
    card.appendChild(el("p", "dash-empty", "No data"));
    return card;
  }
  const values = rows.map((r) => Number(r.tokens) || 0);
  const max = Math.max(1, ...values);
  rows.forEach((r, i) => {
    const row = document.createElement("div");
    row.className = "dash-bar-row";
    row.appendChild(el("span", "dash-bar-name", r.block || ""));
    const track = document.createElement("div");
    track.className = "dash-bar-track";
    const fill = document.createElement("div");
    fill.className = "dash-bar-fill";
    // 最大値に対する比（0 でも見えるように最低 2%）
    fill.style.width = `${Math.max(2, (values[i] / max) * 100)}%`;
    track.appendChild(fill);
    row.append(
      track,
      el("span", "dash-bar-num", `${formatTokens(values[i])} · ${Number(r.executions) || 0} runs`)
    );
    card.appendChild(row);
  });
  return card;
}

// 7. 直近の実行（押すとメッセージ画面へ）
function recentCard(d) {
  const card = dashCard("dash-recent-card");
  card.appendChild(cardTitle("Recent runs"));
  const rows = d.recent || [];
  if (rows.length === 0) {
    card.appendChild(el("p", "dash-empty", "No runs in this period"));
    return card;
  }
  for (const r of rows) card.appendChild(recentRow(r));
  return card;
}

function recentRow(r) {
  const btn = document.createElement("button");
  btn.className = "dash-recent";
  // 書き出しが無い実行はメッセージ画面が開けない
  const linkable = r.linkable === true;
  if (!linkable) btn.classList.add("dim");

  const id = el("span", "dash-r-id", `#${r.run_id}`);
  const block = el("span", "dash-r-block", r.block || "");
  const status = { completed: "✓", running: "●", failed: "✗", cancelled: "✗" }[r.status] || "";
  const st = el("span", `dash-r-status s-${r.status || "none"}`, status);
  const time = el("span", "dash-r-time", clockOf(r.started_at));
  const desc = el("span", "dash-r-desc", r.description || "");
  btn.append(id, block, st, time, desc);
  btn.disabled = !linkable;
  if (linkable) {
    btn.addEventListener("click", () => {
      const project = r.project || dashName;
      location.hash = `#/run/${encodeURIComponent(project)}/${encodeURIComponent(r.run_id)}`;
    });
  }
  return btn;
}

// 8. todo カード（押して確認すると実行）
function renderTodo() {
  const box = document.getElementById("dash-todo");
  if (!box) return;
  box.innerHTML = "";
  const card = dashCard("dash-todo-card");
  const head = document.createElement("div");
  head.className = "dash-card-head";
  head.appendChild(cardTitle("todo cards"));
  card.appendChild(head);
  // 「実行」を押すとどうなるかの説明（端末に慣れていない人向け）
  const note = document.createElement("p");
  note.className = "dash-hint";
  note.textContent = "Press \"Run\" and the agent on the Mac starts running that card.";
  card.appendChild(note);

  const alive = !!(dashData && dashData.daemon_alive);
  if (!alive) {
    // 常駐が止まっているときは実行ボタンを押せない
    card.appendChild(el("div", "dash-hint", "The daemon is stopped. Press \"Start\" above."));
  }
  if (todoRuns.length === 0) {
    card.appendChild(el("p", "dash-empty", todoLoaded ? "No todo cards" : "Loading…"));
    box.appendChild(card);
    return;
  }
  for (const c of todoRuns) card.appendChild(todoRow(c, alive));
  box.appendChild(card);
}

function todoRow(c, alive) {
  const row = document.createElement("div");
  row.className = "dash-todo-row";
  const main = document.createElement("div");
  main.className = "dash-todo-main";
  const desc = document.createElement("div");
  desc.className = "dash-todo-desc";
  // 1 行目: 作成日 ID block model / 2 行目: 説明
  const lines = saiTodoLines(c);
  desc.textContent = lines.head;
  const meta = document.createElement("div");
  meta.className = "dash-todo-meta";
  meta.textContent = lines.desc;
  main.append(desc, meta);

  const btn = document.createElement("button");
  btn.className = "dash-run-btn";
  btn.textContent = "Run";
  btn.disabled = !alive || runBusy || c.run_id == null;
  btn.addEventListener("click", () => onRunCardClick(c));
  row.append(main, btn);
  return row;
}

async function onRunCardClick(c) {
  if (runBusy || c.run_id == null) return;
  const label = c.label || `ID${c.run_id}`;
  // 確認は既存の動きと同じ（実行はエージェントを動かすため必ず聞く）
  if (!(await saiConfirm({ title: "Run this card?", body: label }))) return;
  runBusy = true;
  renderTodo(); // 送信中はボタンを押せない
  try {
    const res = await apiFetch(
      `/api/projects/${encodeURIComponent(dashName)}/runs/${encodeURIComponent(String(c.run_id))}/start`,
      { method: "POST" }
    );
    const body = await res.json().catch(() => ({}));
    if (!res.ok) throw new Error(body.detail || "Could not send.");
    showDashMessage(body.message || "Asked to run the card");
    window.SaiShell.ok();
    // todo と Dashboard を読み直す
    await Promise.all([loadDashboard(), loadTodo()]);
  } catch (e) {
    showDashMessage(e && e.message ? e.message : "Could not send.");
    window.SaiShell.fail();
  } finally {
    runBusy = false;
    renderTodo();
  }
}

// --- ブラウザだけで動く部分（node には window / document が無い） ---
if (typeof window !== "undefined") {
  (window.SaiRoutes = window.SaiRoutes || []).push(
    {
      // プロジェクト一覧
      match(hash) {
        return hash === "#/projects" ? {} : null;
      },
      view: "view-projects",
      tab: "project",
      hideTabbar: false,
      interval: 10000,
      async open() {
        renderProjects(null); // 読み込み中（前の表示は残す）
        await loadProjects();
      },
      async poll() {
        await loadProjects();
      },
    },
    {
      // カード型 Dashboard（/list は linkview.js の List 画面が受け持つ）
      match(hash) {
        const m = hash.match(/^#\/project\/([^/]+)$/);
        return m ? { name: decodeURIComponent(m[1]) } : null;
      },
      view: "view-dashboard",
      tab: "project",
      hideTabbar: false,
      interval: 10000,
      async open(params) {
        await openDashboard(params.name);
      },
      async poll() {
        await loadDashboard(); // todo は開いたときと実行後だけ読む
      },
    }
  );

  // 戻るボタンは一覧へ（履歴に頼らない）
  const back = document.getElementById("dash-back");
  if (back) back.addEventListener("click", () => { location.hash = "#/projects"; });
  // 画面幅が変わったら 2 列 / 1 列を切り替える
  window.addEventListener("resize", dashApplyWide);
}

// --- node からの読み込み（tests/test_project.js） ---
if (typeof module !== "undefined" && module.exports) {
  module.exports = { relativeTime, donutSlices, shortDuration, axisLabels, chartNoteText, todoFolderRuns, projectRowFacts };
}
