// agent-link-web の統計画面の本体．
//
// app.js は「…/stats の住所への分岐」と「10秒ごとの読み直し」だけを担当し，
// 画面の中身はすべてこのファイルにまとめてある（同時に 05 が app.js の
// 質問カード部分を触るため，衝突を避けるため・04 の依頼の指定）。
// グラフは SVG を手描きで作る（外部ライブラリは入れない）。
// データは GET /api/runs/{project}/{run_id} の stats をそのまま使う。
"use strict";

// --- 数字の整形（node のテストからも使う） ---

/** トークン数を短く表示する（606000 → 606K，1234567 → 1.2M）． */
function formatTokens(n) {
  const v = Number(n);
  if (!Number.isFinite(v) || v <= 0) return "0";
  const units = [
    [1e9, "B"],
    [1e6, "M"],
    [1e3, "K"],
  ];
  for (const [size, suffix] of units) {
    if (v >= size) {
      // 小数1桁まで（606000 → 606.0 → 606K，1234567 → 1.2M）
      const x = Math.round((v / size) * 10) / 10;
      return `${x}${suffix}`;
    }
  }
  return String(Math.round(v));
}

/** 実行時間を日本語で表示する（845000ms → 14分05秒）． */
function formatDuration(ms) {
  let total = Math.floor(Number(ms) / 1000);
  if (!Number.isFinite(total) || total < 0) return "0s";
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = total % 60;
  const secText = `${String(s).padStart(2, "0")}s`;
  if (h > 0) return `${h}h ${m}m ${secText}`;
  if (m > 0) return `${m}m ${secText}`;
  return `${s}s`;
}

/** そのままの数を桁区切りで（凡例では 606,000 と正確に出す）． */
function formatCount(n) {
  const v = Number(n);
  if (!Number.isFinite(v)) return "0";
  return Math.round(v).toLocaleString("en-US");
}

/** 金額（$1.27）． */
function formatMoney(n) {
  return `$${Number(n).toFixed(2)}`;
}

/** 日時（2026/09/23 09:02）．端末の現地時刻で表示する． */
function formatDateTime(iso) {
  if (!iso) return "";
  const d = new Date(iso);
  if (isNaN(d)) return "";
  return d.toLocaleString("en-US", {
    year: "numeric",
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
  });
}

function num(v) {
  const x = Number(v);
  return Number.isFinite(x) ? x : 0;
}

/** stats はあるが全部 0 のとき true（「統計データがありません」表示）． */
function isEmptyStats(stats) {
  if (!stats || typeof stats !== "object") return true;
  const t = stats.tokens || {};
  const tokenSum = ["input", "output", "cache_read", "cache_write"]
    .reduce((a, k) => a + num(t[k]), 0);
  return (
    tokenSum === 0 &&
    num((stats.cost_usd || {}).total) === 0 &&
    num(stats.tool_call_count) === 0 &&
    num(stats.turn_count) === 0
  );
}

/** トークン内訳の4項目（円グラフ・凡例で共通）． */
function tokenParts(tokens) {
  const t = tokens || {};
  return [
    { label: "Input", cls: "st-in", n: num(t.input) },
    { label: "Output", cls: "st-out", n: num(t.output) },
    { label: "Cache read", cls: "st-cr", n: num(t.cache_read) },
    { label: "Cache write", cls: "st-cw", n: num(t.cache_write) },
  ];
}

/** 棒グラフの行：多い順に上位 limit 個，残りは「その他」にまとめる． */
function toolBarRows(stats, limit = 10) {
  const byName = (stats && stats.tool_calls_by_name) || {};
  const entries = Object.entries(byName)
    .filter(([, n]) => num(n) > 0)
    .sort((a, b) => num(b[1]) - num(a[1]));
  if (entries.length === 0) return [];
  const top = entries.slice(0, limit);
  const rest = entries.slice(limit);
  if (rest.length > 0) {
    top.push(["Other", rest.reduce((a, [, n]) => a + num(n), 0)]);
  }
  return top;
}

// --- 状態の表示（app.js に依存せず自己完結させる） ---

const STATUS_LABEL = {
  running: { text: "🟡 Running", cls: "st-running" },
  completed: { text: "✅ Done", cls: "st-done" },
  failed: { text: "⚠ Failed", cls: "st-failed" },
  cancelled: { text: "⚠ Cancelled", cls: "st-failed" },
};

// 表示中の実行の住所（戻るボタンでメッセージ画面に戻るため）
const STATS_POS = { project: "", id: "" };
// 読み終わった実行は10秒ごとに読み直さない（開いたときに1回だけ）
let statsFinished = false;
// 同じ内容を描き直してグラフが毎回動き直さないようにする
let statsLastJson = "";

// SVG の要素は共通部品（ui.js の saiSvg）が作る。ここでは呼び出すだけにして、
// 同じ作り方を 2 か所に置かない。
function svgEl(tag, attrs) {
  return saiSvg(tag, attrs);
}

function el(tag, cls, text, hideFromReader) {
  const n = document.createElement(tag);
  if (cls) n.className = cls;
  if (text != null) n.textContent = text;
  // hideFromReader が true のときは、手元にある図の aria-label が同じことを
  // 言うので、読み上げでは 1 回だけ聞かせるようにする（画面には出す）。
  if (hideFromReader) n.setAttribute("aria-hidden", "true");
  return n;
}

async function openStats(r) {
  STATS_POS.project = r.project;
  STATS_POS.id = r.id;
  statsFinished = false;
  statsLastJson = "";
  document.getElementById("stats-title").textContent = "Stats";
  document.getElementById("stats-desc").textContent = "";
  document.getElementById("stats-head").innerHTML = "";
  document.getElementById("stats-cards").innerHTML =
    "<p class='st-note'>Loading…</p>";
  await loadStats();
}

async function loadStats() {
  let data;
  try {
    const res = await apiFetch(
      `/api/runs/${encodeURIComponent(STATS_POS.project)}/${encodeURIComponent(STATS_POS.id)}`
    );
    if (!res.ok) throw new Error("bad");
    data = await res.json();
  } catch {
    // 切れている間も他の画面と同じく帯だけ出して次に回す
    setOffline(true);
    onFail();
    return;
  }
  setOffline(false);
  onOk();
  if (JSON.stringify(data) === statsLastJson) return; // 変わっていないので描き直さない
  statsLastJson = JSON.stringify(data);
  renderStats(data);
}

/** 10秒ごとに呼ばれる読み直し（実行中のみ）． */
async function pollStats() {
  if (statsFinished) return;
  await loadStats();
}

// --- 描画 ---

function renderStats(data) {
  // 見出しは実行中でも出す
  document.getElementById("stats-title").textContent =
    `#${data.run_id} ${data.block_name || ""}`.trim();
  document.getElementById("stats-desc").textContent = data.description || "";
  renderHead(data);

  const cards = document.getElementById("stats-cards");
  cards.innerHTML = "";

  // 実行中は「未集計」
  if (data.status === "running") {
    statsFinished = false;
    cards.appendChild(
      el("p", "st-note", "Not counted yet because the run is still going (agent-link counts it after the run ends)")
    );
    return;
  }
  statsFinished = true;

  const stats = data.stats;
  if (isEmptyStats(stats)) {
    // stats 無し・全部 0
    cards.appendChild(el("p", "st-note", "No stats data"));
    return;
  }
  renderCards(cards, stats, data);
}

function renderHead(data) {
  const box = document.getElementById("stats-head");
  box.innerHTML = "";
  const stats = data.stats || {};
  const model = data.model || stats.model || "Unknown model";
  const parts = [];
  if (data.project_name) parts.push(data.project_name);
  parts.push(model);
  box.appendChild(el("div", "st-head-line", parts.join(" · ")));

  const range = data.finished_at
    ? `${formatDateTime(data.started_at)} – ${formatDateTime(data.finished_at)}`
    : `${formatDateTime(data.started_at)} –`;
  box.appendChild(el("div", "st-head-line", range));

  const st = STATUS_LABEL[data.status] || { text: data.status || "", cls: "" };
  box.appendChild(el("div", `st-status ${st.cls}`, st.text));
}

function bigCard(label, value, note) {
  const card = el("div", "st-big");
  card.appendChild(el("div", "st-big-value", value));
  card.appendChild(el("div", "st-big-label", label));
  if (note) card.appendChild(el("div", "st-big-note", note));
  return card;
}

function renderCards(cards, stats, data) {
  // 1. 大きな数字3つ
  const bigs = el("div", "st-bigs");
  const cost = (stats.cost_usd || {}).total;
  const costValue =
    cost == null ? "—" : `${stats.cost_is_estimate ? "≈" : ""}${formatMoney(cost)}`;
  // コストの出どころの注記（概算 / cc 集計 / pi 集計）
  let costNote = "";
  if (stats.cost_is_estimate) costNote = "estimate";
  else if (stats.cost_source === "cc") costNote = "counted by cc";
  else if (stats.cost_source === "pi") costNote = "counted by pi";
  bigs.appendChild(bigCard("Cost", costValue, costNote));

  const ms = stats.elapsed_ms != null ? stats.elapsed_ms : data.duration_ms;
  bigs.appendChild(bigCard("Duration", ms == null ? "—" : formatDuration(ms), ""));
  bigs.appendChild(bigCard("Total tokens", formatTokens((stats.tokens || {}).total), ""));
  cards.appendChild(bigs);

  // 2. 円グラフ（ドーナツ型）: トークン内訳
  cards.appendChild(tokenCard(stats));
  // 3. 横棒グラフ: ツール別使用回数
  cards.appendChild(toolCard(stats));
  // 4. 小さな補足行
  cards.appendChild(footCard(stats));
  animateCharts();
}

const DONUT_R = 38; // 半径（viewBox 100×100）
const DONUT_C = 2 * Math.PI * DONUT_R;

function tokenCard(stats) {
  const card = el("section", "st-card");
  card.appendChild(el("h3", "st-card-title", "Token breakdown"));
  const parts = tokenParts(stats.tokens);
  const sum = parts.reduce((a, p) => a + p.n, 0);
  if (sum <= 0) {
    card.appendChild(el("p", "st-note", "No data"));
    return card;
  }

  const wrap = el("div", "st-donut-wrap");
  const biggest = parts.filter((p) => p.n > 0).sort((a, b) => b.n - a.n)[0];
  const svg = svgEl("svg", {
    viewBox: "0 0 100 100",
    class: "st-donut",
    role: "img",
    "aria-label":
      `Donut chart of the token breakdown. Total ${formatTokens(sum)} tokens` +
      (biggest
        ? `. Largest is ${biggest.label} ${formatCount(biggest.n)} (${((biggest.n / sum) * 100).toFixed(1)}%)`
        : ""),
  });
  let start = 0;
  for (const p of parts) {
    if (p.n <= 0) continue;
    const frac = p.n / sum;
    const len = Math.max(frac * DONUT_C - 1.5, 0.5); // ギャップは 1.5 単位
    const seg = svgEl("circle", {
      cx: "50", cy: "50", r: String(DONUT_R),
      class: `st-seg ${p.cls}`,
      transform: "rotate(-90 50 50)",
      "stroke-dasharray": `0 ${DONUT_C.toFixed(1)}`,
      "stroke-dashoffset": String(-start * DONUT_C),
    });
    seg.dataset.dash = `${len.toFixed(2)} ${(DONUT_C - len).toFixed(2)}`;
    svg.appendChild(seg);
    start += frac;
  }
  wrap.appendChild(svg);
  const center = el("div", "st-donut-center");
  center.appendChild(el("strong", "st-donut-total", formatTokens(sum)));
  center.appendChild(el("span", "st-donut-cap", "Total"));
  wrap.appendChild(center);
  card.appendChild(wrap);

  const legend = el("div", "st-legend");
  for (const p of parts) {
    const row = el("div", "st-leg");
    row.appendChild(el("span", `st-dot ${p.cls}`));
    row.appendChild(el("span", "st-leg-name", p.label));
    row.appendChild(el("span", "st-leg-num", formatCount(p.n)));
    row.appendChild(
      el("span", "st-leg-pct", `${((p.n / sum) * 100).toFixed(1)}%`)
    );
    legend.appendChild(row);
  }
  card.appendChild(legend);
  return card;
}

function toolCard(stats) {
  const card = el("section", "st-card");
  card.appendChild(el("h3", "st-card-title", "Tool calls by tool"));
  const rows = toolBarRows(stats);
  if (rows.length === 0) {
    card.appendChild(el("p", "st-note", "No data"));
    return card;
  }
  const max = num(rows[0][1]);
  card.appendChild(
    el("p", "st-hint", `Bar length is relative to the first row (${String(rows[0][0])}: ${formatCount(max)} calls) as 100%`)
  );
  for (const [name, count] of rows) {
    const n = num(count);
    const frac = max > 0 ? Math.max(n / max, 0.01) : 0;
    const row = el("div", "st-bar-row");
    // 名前と回数は 図の aria-label に入れる（読上げが二重にならないよう隠す）
    row.appendChild(el("span", "st-bar-name", String(name), true));
    const svg = svgEl("svg", {
      viewBox: "0 0 100 8",
      preserveAspectRatio: "none",
      class: "st-bar-svg",
      role: "img",
      "aria-label":
        `${String(name)} ${formatCount(n)} calls` +
        ` (${Math.round(frac * 100)}% of ${formatCount(max)} calls in the first row)`,
    });
    const rect = svgEl("rect", {
      x: "0", y: "0", width: "100", height: "8", rx: "2",
      class: "st-bar-rect",
    });
    rect.style.transform = "scaleX(0)";
    rect.dataset.frac = max > 0 ? String(Math.max(num(count) / max, 0.01)) : "0";
    svg.appendChild(rect);
    row.appendChild(svg);
    row.appendChild(el("span", "st-bar-num", formatCount(count), true));
    card.appendChild(row);
  }
  return card;
}

function footCard(stats) {
  const card = el("section", "st-card st-foot");
  const summary = el("div", "st-foot-line");
  summary.appendChild(el("b", null, `Turns ${formatCount(stats.turn_count)}`));
  summary.appendChild(document.createTextNode(" · "));
  summary.appendChild(
    el("b", null, `Tool calls ${formatCount(stats.tool_call_count)}`)
  );
  card.appendChild(summary);

  const c = stats.cost_usd || {};
  const seg = (label, v) =>
    `${label} ${v == null ? "—" : formatMoney(v)}`;
  const line = el(
    "div",
    "st-foot-line",
    [
      seg("Input", c.input),
      seg("Output", c.output),
      seg("Cache read", c.cache_read),
      seg("Cache write", c.cache_write),
    ].join(" · ")
  );
  card.appendChild(line);

  if (stats.cost_source === "cc") {
    // cc 自身の金額は合計しか持たないため内訳は概算
    card.appendChild(el("div", "st-foot-note", "Breakdown is an estimate"));
  }
  return card;
}

// 描いたばかりの棒とドーナツを少し動かして出す（CSS transition）。
// 「動きを減らす」と言っているときは最初から最後の姿で描く（アニメーションの規則）。
function animateCharts() {
  const finish = () => {
    document.querySelectorAll("#stats-cards .st-bar-rect").forEach((r) => {
      r.style.transform = `scaleX(${r.dataset.frac || 0})`;
    });
    document.querySelectorAll("#stats-cards .st-seg").forEach((s) => {
      if (s.dataset.dash) s.setAttribute("stroke-dasharray", s.dataset.dash);
    });
  };
  if (saiReducedMotion()) {
    finish();
    return;
  }
  requestAnimationFrame(finish);
}

// --- node からの読み込み（tests/test_stats.js） ---
if (typeof module !== "undefined" && module.exports) {
  module.exports = {
    formatTokens,
    formatDuration,
    formatCount,
    formatMoney,
    tokenParts,
    toolBarRows,
    isEmptyStats,
  };
}

// --- ブラウザだけで動く初期化（node には window が無い） ---
if (typeof window !== "undefined") {
  // app.js のルーティングから呼ばれる入口
  window.SaiStats = { open: openStats, poll: pollStats };
  // 戻るボタンはメッセージ画面へ
  document.getElementById("stats-back").addEventListener("click", () => {
    location.hash = `#/run/${encodeURIComponent(STATS_POS.project)}/${encodeURIComponent(STATS_POS.id)}`;
  });
}
