// sai-phone の Project タブ（実装計画 06）の本体。
//
// 画面は2つ:
//   プロジェクト一覧   #/projects
//   カード型 Dashboard #/project/<name>
// 中身は Apple のヘルスケア / スクリーンタイムのように、余白の多いカードを
// 縦に積んだ形（style.css の .dash-card）。グラフは SVG を手描きで作る
// （外部ライブラリは入れない・00 §2）。データはすべて /api/projects/… 経由
// で，sai-phone 自身は DB を開かない（00 §2）。
// DOM を使わない小さな関数は末尾で node に渡す（tests/test_project.js）。
"use strict";

// --- 定数 ---

// 期間の選択をタブごとに覚える（06 §4.2）
const DASH_PERIOD_KEY = "sai_phone_dash_period";
const DASH_PERIODS = [
  ["week", "Week"],
  ["month", "Month"],
  ["all", "All time"],
];
// モデル別ドーナツの色（5色・06 §4.2-5）。色は CSS 変数。暗い画面では自動的に変わる。
const MODEL_COLORS = ["var(--series-1)", "var(--series-2)", "var(--series-3)", "var(--series-4)", "var(--series-5)", "var(--series-6)"];
const WEEKDAY_JA = ["日", "月", "火", "水", "木", "金", "土"];
// 一覧のランプ（稼働中は緑・停止は輪郭のみ・06 §4.1）
const LAMP_ON = "var(--ok)";

// --- DOM を使わない小さな関数（node のテストからも使う・06 §4.4） ---

/** DB の文字列（タイムゾーン無し = UTC・00 §7）を時刻（ms）にする。読めなければ null。 */
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

/** 「たった今」「5分前」「3時間前」「2日前」「9/12」（06 §4.1・§4.4）。 */
function relativeTime(iso, nowMs) {
  const t = parseStamp(iso);
  if (t == null) return "";
  const now = Number.isFinite(nowMs) ? nowMs : Date.now();
  const diff = Math.max(0, now - t);
  const min = Math.floor(diff / 60000);
  if (min < 1) return "たった今";
  if (min < 60) return `${min}分前`;
  const h = Math.floor(min / 60);
  if (h < 24) return `${h}時間前`;
  const d = Math.floor(h / 24);
  if (d < 7) return `${d}日前`;
  const dt = new Date(t);
  return `${dt.getMonth() + 1}/${dt.getDate()}`;
}

/**
 * ドーナツ用に上位 N 件＋「その他」へまとめる（06 §4.4）。
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
    out.push({ label: "その他", value: v, ratio: v / total });
  }
  return out;
}

/** 実行時間（秒は省く・「4時間45分」。06 §4.2-3）。 */
function shortDuration(ms) {
  const total = Math.floor(Number(ms) / 1000);
  if (!Number.isFinite(total) || total < 0) return "0";
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  if (h > 0) return `${h}時間${m}分`;
  if (m > 0) return `${m}分`;
  return `${total}秒`;
}

/** 時刻の文字列 → 端末の現地時刻の「HH:MM」（DB の日時は UTC のため現地時刻に直す）。 */
function clockOf(stamp) {
  const t = parseStamp(stamp);
  if (t == null) return "";
  return new Date(t).toLocaleTimeString("ja-JP", { hour: "2-digit", minute: "2-digit" });
}

/** 棒を押したときに出す1行（「9/22（月） 1.2M」・06 §4.2-4）。 */
function chartNoteText(row, metric) {
  const date = String((row && row.date) || "");
  if (!date) return "";
  const md = `${Number(date.slice(5, 7))}/${Number(date.slice(8, 10))}`;
  const wd = WEEKDAY_JA[new Date(`${date}T00:00:00`).getDay()] || "";
  const n = Number((row && row.tokens) || 0);
  // 数字の整形は stats.js の関数（ブラウザでだけ読み込まれるため typeof で確かめる）
  const v = metric === "count"
    ? `${Number((row && row.executions) || 0)}回`
    : typeof formatTokens === "function" ? formatTokens(n) : String(n);
  return `${md}（${wd}） ${v}`;
}

/** 横軸ラベル（今週=曜日・今月=1/5/10/…・全期間=月が変わる所だけ・06 §4.2-4）。 */
function axisLabels(period, daily) {
  const rows = daily || [];
  return rows.map((row, i) => {
    const date = String(row.date || "");
    const day = Number(date.slice(8, 10));
    if (period === "week") {
      return WEEKDAY_JA[new Date(`${date}T00:00:00`).getDay()] || "";
    }
    if (period === "month") {
      return [1, 5, 10, 15, 20, 25, 30].includes(day) ? String(day) : "";
    }
    const cur = date.slice(0, 7);
    const prev = i > 0 ? String(rows[i - 1].date || "").slice(0, 7) : null;
    return i === 0 || cur !== prev ? `${Number(cur.slice(5, 7))}月` : "";
  });
}

/** モデル名は `/` の後ろだけ（「xiaomi/mimo」→「mimo」・06 §4.2-5）。 */
function shortModel(name) {
  const s = String(name || "");
  const i = s.lastIndexOf("/");
  return i >= 0 ? s.slice(i + 1) : s;
}

/**
 * ツリー（`GET …/tree`）の todo フォルダの中身（新形式 todo の一覧・計画 15）。
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
let todoRuns = [];            // 未実行の todo run（ツリーの todo フォルダの中身・計画 15）
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

// --- 一覧（#/projects・06 §4.1） ---

/** list === null は「読み込み中」（前の表示は消さない・06 §4.2）。 */
function renderProjects(list) {
  const box = document.getElementById("projects-list");
  if (!box) return;
  if (list === null) {
    if (!box.childElementCount) box.innerHTML = "<p class='dash-empty'>読み込み中…</p>";
    return;
  }
  box.innerHTML = "";
  if (list.length === 0) {
    // まだ 1 つも無いときは、案内だけ出す
    const box2 = document.createElement("div");
    box2.className = "sai-empty";
    box2.appendChild(el("p", "sai-empty-title", "まだプロジェクトがありません"));
    box2.appendChild(el("p", "sai-empty-body", "Mac で `sai-link init` を実行するとここに出ます。"));
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
  parts.push(d.daemon_alive ? "常駐 動いています" : "常駐 止まっています");
  if (running > 0) parts.push(`実行中 ${running} 件`);
  const ago = relativeTime(d.last_executed_at);
  if (ago) parts.push(`最後に動いたのは ${ago}`);
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
  const btn = document.createElement("button");
  btn.className = "proj-row";
  // root か db が無い行は薄くして押せない（06 §4.1）
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
  line1.append(lamp, name, time, chev);

  const line2 = document.createElement("div");
  line2.className = "proj-line2";
  if (!p.root_exists) line2.textContent = "フォルダが見つかりません";
  else if (!p.db_exists) line2.textContent = "実行の記録なし";
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
  return btn;
}

async function loadProjects() {
  if (!isListScreen()) return;
  let data;
  try {
    const res = await apiFetch("/api/projects");
    const body = await res.json().catch(() => ({}));
    if (!res.ok) throw new Error(body.detail || "読み込めませんでした");
    data = body;
  } catch (e) {
    // 失敗しても前の表示は残し，帯だけ出す（00 §9）
    window.SaiShell.fail();
    return;
  }
  renderProjects(data.projects || []);
  window.SaiShell.ok();
}

// --- Dashboard（#/project/<name>・06 §4.2） ---

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
  // 最後に見ていた画面をプロジェクトごとに見る（C-F7）。
  // List / Workflow / 設定を選んでいたプロジェクトはそちらへ移る（linkview.js が覚えておく）。
  if (typeof rememberedMode === "function" && rememberedMode(name) !== "dashboard") {
    location.replace(modeHash(name, rememberedMode(name)));
    return;
  }
  // 上部の Dashboard | Workflow | List | 設定 の切り替えを置く（C-F7）
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
  cards.innerHTML = "<p class='dash-empty'>読み込み中…</p>";
  const todo = document.createElement("div");
  todo.id = "dash-todo";
  todo.innerHTML = "<section class='dash-card'><p class='dash-empty'>読み込み中…</p></section>";
  // 検索カードは 10 秒ごとの描き直し（.dash-cards）で入力が消えないよう、外に置く
  const search = document.createElement("div");
  search.className = "dash-search";
  if (typeof dsrMount === "function") dsrMount(search, name);
  body.append(msg, search, cards, todo);
  dashApplyWide();

  // Dashboard と todo は同時に読む（todo は開いたときだけ読む・06 §4.2）
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
    if (!res.ok) throw new Error(body.detail || "読み込めませんでした");
    data = body;
  } catch (e) {
    // 失敗は detail をカードの位置に出す（06 §4.2）
    showDashMessage(e && e.message ? e.message : "読み込めませんでした");
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
    // todo 一覧は `GET …/tree` の todo フォルダの中身（別 CLI は使わない・計画 15）
    const res = await apiFetch(`/api/projects/${encodeURIComponent(dashName)}/tree`);
    const body = await res.json().catch(() => ({}));
    if (!res.ok) throw new Error(body.detail || "読み込めませんでした");
    items = body.items;
  } catch (e) {
    showDashMessage(e && e.message ? e.message : "todo を読み込めませんでした");
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
  if (typeof dsrSetBlocks === "function") dsrSetBlocks((d.by_block || []).map((r) => r.block));
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
  left.append(lamp, document.createTextNode(d.daemon_alive ? " 稼働中" : " 停止中"));

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
    ? "常駐は Mac 側で動き続ける係です。止めると実行中のカードも止まります。"
    : "常駐が止まっています。押して起動すると、このプロジェクトのカードを実行できます。";
  card.appendChild(note);
  return card;
}

async function onDaemonClick() {
  if (daemonBusy || !dashData) return;
  const action = dashData.daemon_alive ? "stop" : "start";
  const question = action === "stop"
    ? `${dashName} の常駐を停止しますか？`
    : `${dashName} の常駐を起動しますか？`;
  if (!confirm(question)) return;
  daemonBusy = true;
  renderDashboard();
  try {
    const res = await apiFetch(`/api/projects/${encodeURIComponent(dashName)}/daemon`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ action }),
    });
    const body = await res.json().catch(() => ({}));
    if (!res.ok) throw new Error(body.detail || "送れませんでした");
    showDashMessage(body.message || (action === "stop" ? "停止を要求しました" : "起動を要求しました"));
    window.SaiShell.ok();
  } catch (e) {
    showDashMessage(e && e.message ? e.message : "送れませんでした");
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
    [`${Number(s.executions) || 0} 回`, "Run"],
    [`${Math.round((Number(s.success_rate) || 0) * 100)}%`, "成功率"],
    [formatTokens(s.tokens_total), "トークン"],
    [shortDuration(s.duration_ms), "時間"],
  ];
  for (const [value, label] of items) {
    const cell = document.createElement("div");
    cell.className = "dash-cell";
    cell.appendChild(el("div", "dash-big", value));
    cell.appendChild(el("div", "dash-big-label", label));
    grid.appendChild(cell);
  }
  card.appendChild(grid);

  // 費用・失敗・実行中（0 の項目は省く・06 §4.2-3）
  const parts = [];
  const cost = Number(s.cost_usd) || 0;
  if (cost > 0) parts.push(`費用 ${formatMoney(cost)}`);
  if (Number(s.failed) > 0) parts.push(`失敗 ${Number(s.failed)}`);
  if (Number(s.running) > 0) parts.push(`実行中 ${Number(s.running)}`);
  if (parts.length) card.appendChild(el("div", "dash-sub", parts.join(" · ")));
  return card;
}

// 4. 日ごとのグラフ（token / 回数）
function chartCard(d) {
  const card = dashCard("dash-chart-card");
  const head = document.createElement("div");
  head.className = "dash-card-head";
  head.appendChild(cardTitle(chartMetric === "tokens" ? "日ごとの token" : "日ごとの実行数"));
  const seg = document.createElement("div");
  seg.className = "dash-seg small";
  for (const [value, label] of [["tokens", "token"], ["count", "回数"]]) {
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
    what: chartMetric === "count" ? "日ごとの実行数" : "日ごとの token",
    onPick: (row) => {
      chartNote = chartNoteText(row, chartMetric);
      renderDashboard();
    },
  });
}

// 5. モデル別の内訳（ドーナツ・上位 4 件＋その他）
function modelCard(d) {
  const card = dashCard("dash-model");
  card.appendChild(cardTitle("モデル別"));
  const slices = donutSlices(
    (d.by_model || []).map((m) => ({
      label: m.model || "(不明)",
      value: Number(m.tokens) || 0,
    })),
    4
  );
  if (slices.length === 0) {
    card.appendChild(el("p", "dash-empty", "データがありません"));
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
      `モデル別のトークン割合のドーナツ。合計 ${formatTokens(total)} トークン` +
      `。最も多いのは${shortModel(slices[0].label)} ${formatTokens(slices[0].value)}` +
      `（${Math.round(slices[0].ratio * 100)}%）`,
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
  center.appendChild(el("span", "dash-donut-cap", "合計"));
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
  card.appendChild(cardTitle("ブロック別"));
  const rows = (d.by_block || []).slice(0, 8);
  if (rows.length === 0) {
    card.appendChild(el("p", "dash-empty", "データがありません"));
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
      el("span", "dash-bar-num", `${formatTokens(values[i])} · ${Number(r.executions) || 0}回`)
    );
    card.appendChild(row);
  });
  return card;
}

// 7. 直近の実行（押すとメッセージ画面へ）
function recentCard(d) {
  const card = dashCard("dash-recent-card");
  card.appendChild(cardTitle("直近の実行"));
  const rows = d.recent || [];
  if (rows.length === 0) {
    card.appendChild(el("p", "dash-empty", "この期間の実行はありません"));
    return card;
  }
  for (const r of rows) card.appendChild(recentRow(r));
  return card;
}

function recentRow(r) {
  const btn = document.createElement("button");
  btn.className = "dash-recent";
  // 書き出しが無い実行はメッセージ画面が開けない（06 §4.2-7）
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
  head.appendChild(cardTitle("todo カード"));
  card.appendChild(head);
  // 「実行」を押すとどうなるかの説明（端末に慣れていない人向け）
  const note = document.createElement("p");
  note.className = "dash-hint";
  note.textContent = "「実行」を押すと、Mac 側のエージェントがそのカードを動かし始めます。";
  card.appendChild(note);

  const alive = !!(dashData && dashData.daemon_alive);
  if (!alive) {
    // 常駐が止まっているときは実行ボタンを押せない（06 §4.2-8）
    card.appendChild(el("div", "dash-hint", "常駐が止まっています。上の「起動」を押してください"));
  }
  if (todoRuns.length === 0) {
    card.appendChild(el("p", "dash-empty", todoLoaded ? "未実行のカードはありません" : "読み込み中…"));
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
  // 1 行目: 作成日 ID block runner model / 2 行目: 説明
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
  // 確認は既存の動きと同じ（実行はエージェントを動かすため必ず聞く・計画 15）
  const question = `このカードを実行しますか？\n\n${label}`;
  if (!confirm(question)) return;
  runBusy = true;
  renderTodo(); // 送信中はボタンを押せない
  try {
    const res = await apiFetch(
      `/api/projects/${encodeURIComponent(dashName)}/runs/${encodeURIComponent(String(c.run_id))}/start`,
      { method: "POST" }
    );
    const body = await res.json().catch(() => ({}));
    if (!res.ok) throw new Error(body.detail || "送れませんでした");
    showDashMessage(body.message || "実行を要求しました");
    window.SaiShell.ok();
    // todo と Dashboard を読み直す（計画 15）
    await Promise.all([loadDashboard(), loadTodo()]);
  } catch (e) {
    showDashMessage(e && e.message ? e.message : "送れませんでした");
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
      // カード型 Dashboard（/list は linkview.js の List 画面が受け持つ・計画 14）
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
        await loadDashboard(); // todo は開いたときと実行後だけ読む（06 §4.2）
      },
    }
  );

  // 戻るボタンは一覧へ（履歴に頼らない・02 §3.1）
  const back = document.getElementById("dash-back");
  if (back) back.addEventListener("click", () => { location.hash = "#/projects"; });
  // 画面幅が変わったら 2 列 / 1 列を切り替える
  window.addEventListener("resize", dashApplyWide);
}

// --- node からの読み込み（tests/test_project.js・06 §5） ---
if (typeof module !== "undefined" && module.exports) {
  module.exports = { relativeTime, donutSlices, shortDuration, axisLabels, chartNoteText, todoFolderRuns, projectRowFacts };
}
