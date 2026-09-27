"use strict";
// sai-phone のホーム画面（実装計画 U4・住所 #/home・枠 view-home・タブ home・U02 で組み直し）。
//
// 端末（ターミナル）に慣れていない人が最初に開く画面。ここに
//   1. 全プロジェクトの合計（今週 / 今月 / 全期間・実行数・成功率・token・料金・日ごとの棒）
//   2. 質問がある会話（質問と承認）
//   3. todo
//   4. 最近のアクティビティ
//   5. 確認を待っているもの（review）
//   6. 動いているもの（実行中）
//   7. 止まっている常駐
//   8. 論文の要対応
// を並べ、下に読めなかったプロジェクトを出す。1〜4 は 0 件でもカードを出す。
//
// データは GET /api/home（質問・承認・プロジェクト・論文）と
// GET /api/home/overview（全プロジェクトの合計）の 2 本（U02 §1・U3 C-A3）。
// 合計のほうだけ期間を変えられるので、別の住所にしてある。
// 15 秒ごとに読み直す。DOM を使わない関数（hmSections など）は
// 末尾で node に渡して tests/test_home.js で確かめる。
// 共通部品は ui.js（saiApi / saiEmpty / saiTerm / saiIsWide）を使う。
// 一番外側の名前は hm / HM_ で始める（C-F6）。

// --- 定数 ---

// ホームを読み直す間隔（ミリ秒）
const HM_INTERVAL_MS = 15000;
// 論文の要対応で出す件数
const HM_PAPERS_MAX = 3;
// Papers を開いたとき_LAST プロジェクトを覚えておく場所（papers.js と同じ C-F1）
const HM_PAPERS_KEY = "sai_phone_papers_project";
// 合計の期間（画面の切り替え・project.js の DASH_PERIODS と同じ並び）
const HM_PERIODS = [
  ["week", "今週"],
  ["month", "今月"],
  ["all", "全期間"],
];
// 合計の期間を覚えておく場所（project.js は sessionStorage・ホームは localStorage）
const HM_PERIOD_KEY = "sai_phone_home_period";
// todo とアクティビティははじめ 10 件、「すべて表示」を押したときの件数
const HM_TODO_MAX = 10;
const HM_RECENT_MAX = 10;
const HM_RECENT_ALL = 20;
// 実行の状態の印（project.js の recentRow と同じ）
const HM_STATUS_MARK = { completed: "✓", running: "●", failed: "✗", cancelled: "✗" };
// 曜日の日本語（project.js と同じ）
const HM_WEEKDAY_JA = ["日", "月", "火", "水", "木", "金", "土"];

// --- 表示の状態 ---
let hmData = null;        // 直前に読んだ /api/home の内容
let hmOverview = null;    // 直前に読んだ /api/home/overview の内容
let hmOverviewError = ""; // 合計だけ読めなかったときの 1 行
let hmLoading = false;    // 二重に走らせない
let hmError = "";         // 読めなかったときの 1 行
// 「すべて表示」を押したあとかどうか（todo とアクティビティ）
let hmShowAll = { todo: false, recent: false };

// --- DOM を使わない部品（テストする） ---

/** DB の日時（タイムゾーン無しは UTC）を ms に直す。読めなければ null。 */
function hmStamp(value) {
  if (!value) return null;
  let s = String(value).trim();
  if (!s) return null;
  const m = s.match(/^(\d{4}-\d{2}-\d{2})[ T](\d{2}:\d{2}(?::\d{2})?(?:\.\d+)?)$/);
  if (m) s = `${m[1]}T${m[2]}Z`;
  else if (!/[zZ]|[+-]\d{2}:?\d{2}$/.test(s)) s = `${s}Z`;
  const t = Date.parse(s);
  return Number.isNaN(t) ? null : t;
}

/** 「12分前」（読めない日時は空）。 */
function hmAgo(value, nowMs) {
  const t = hmStamp(value);
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

/** 質問・承認の 1 行の題（U4 §3-1）。 */
function hmWaitTitle(item, kind) {
  if (kind === "approval") {
    const tool = item.tool_name || "承認";
    const sum = item.summary || "";
    return sum ? `${tool}: ${sum}` : tool;
  }
  return item.title || "（題のない質問）";
}

/** 質問・承認の 1 行の左（プロジェクト名と #run_id）。 */
function hmWaitWhere(item) {
  if (!item.project) return "";
  return item.run_id == null ? item.project : `${item.project} #${item.run_id}`;
}

/** 質問・承認を押した先の住所（プロジェクトと run が分かれば会話の画面へ）。 */
function hmWaitHash(item) {
  if (!item.project || item.run_id == null) return "#/";
  return `#/run/${encodeURIComponent(item.project)}/${encodeURIComponent(item.run_id)}`;
}

/** 質問・承認の行（新しい順・質問が先で承認が後）。 */
function hmWaitItems(data) {
  const d = data || {};
  const out = [];
  for (const q of Array.isArray(d.questions) ? d.questions : []) {
    out.push({
      kind: "question",
      title: hmWaitTitle(q, "question"),
      where: hmWaitWhere(q),
      ago: hmAgo(q.created_at),
      hash: hmWaitHash(q),
    });
  }
  for (const a of Array.isArray(d.approvals) ? d.approvals : []) {
    out.push({
      kind: "approval",
      title: hmWaitTitle(a, "approval"),
      where: hmWaitWhere(a),
      ago: hmAgo(a.created_at),
      hash: hmWaitHash(a),
    });
  }
  return out;
}

/** 合計の期間（week / month / all。覚えていなければ week）。 */
function hmPeriodValue() {
  try {
    const p = localStorage.getItem(HM_PERIOD_KEY);
    for (const [value] of HM_PERIODS) if (p === value) return p;
  } catch { /* プライベートブラウズでも動くように */ }
  return "week";
}

/** 合計の期間を覚える（覚えられなくても画面は動く）。 */
function hmSavePeriod(period) {
  for (const [value] of HM_PERIODS) {
    if (period !== value) continue;
    try { localStorage.setItem(HM_PERIOD_KEY, value); } catch { /* 保存できなくても動く */ }
    return;
  }
}

/** token の埋め方（ページでは stats.js の共通部品を使う・テストだけ下の実装になる）。 */
function hmTokens(value) {
  if (typeof formatTokens === "function") return formatTokens(value);
  const v = Number(value);
  if (!Number.isFinite(v) || v <= 0) return "0";
  for (const [size, suffix] of [[1e9, "B"], [1e6, "M"], [1e3, "K"]]) {
    if (v >= size) return `${(v / size).toFixed(1).replace(/\.0$/, "")}${suffix}`;
  }
  return String(Math.round(v));
}

/** 料金の埋め方（USD・2 桁まで。ページでは stats.js の共通部品を使う）。 */
function hmMoney(value) {
  if (typeof formatMoney === "function") return formatMoney(Number(value) || 0);
  return `$${(Number(value) || 0).toFixed(2)}`;
}

/**
 * 合計カードに出す大きな数字 4 つ（実行数・成功率・token・料金）。
 * 返り値は `{label, value}` の並び（DOM は使わない）。
 */
function hmOverviewCards(overview) {
  const o = overview || {};
  const s = o.summary || {};
  return [
    { label: "実行", value: `${Number(s.executions) || 0} 回` },
    { label: "成功率", value: `${Math.round((Number(s.success_rate) || 0) * 100)}%` },
    { label: "トークン", value: hmTokens(s.tokens_total) },
    { label: "料金", value: hmMoney(s.cost_usd) },
  ];
}

/** 合計カードの補足（「<N> プロジェクトの合計・<since>〜<until>」と読めなかった件数）。 */
function hmOverviewNote(overview) {
  const o = overview || {};
  const n = Number(o.projects) || 0;
  let note = `${n} プロジェクトの合計`;
  if (o.since && o.until) note += `・${o.since}〜${o.until}`;
  const errors = Array.isArray(o.errors) ? o.errors.filter((e) => e) : [];
  if (errors.length > 0) note += `・一部のプロジェクトを読めませんでした（${errors.length}）`;
  return note;
}

/** todo の行（プロジェクトごとに並べた `todo_runs`・{title, where, hash}）。 */
function hmTodoItems(data) {
  const d = data || {};
  const out = [];
  for (const p of Array.isArray(d.projects) ? d.projects : []) {
    const name = p.name || "";
    for (const r of Array.isArray(p.todo_runs) ? p.todo_runs : []) {
      const runId = r && r.run_id;
      if (runId == null) continue;
      out.push({
        title: (r && r.label) || `ID${runId}`,
        where: name,
        hash: `#/project/${encodeURIComponent(name)}/list/${encodeURIComponent(runId)}/task`,
      });
    }
  }
  return out;
}

/** 最近のアクティビティの行（{title, sub, where, ago, hash}）。 */
function hmRecentItems(overview) {
  const o = overview || {};
  const out = [];
  for (const r of Array.isArray(o.recent) ? o.recent : []) {
    if (!r || r.run_id == null) continue;
    const project = r.project || "";
    const block = r.block || "";
    const mark = HM_STATUS_MARK[r.status] || "";
    out.push({
      title: `#${r.run_id}${block ? ` ${block}` : ""}${mark ? ` ${mark}` : ""}`,
      sub: r.description || "",
      where: project,
      ago: hmAgo(r.started_at),
      // 書き出しが無い実行は会話画面が開けないので List へ（project.js の recentRow と同じ）
      hash:
        r.linkable === true
          ? `#/run/${encodeURIComponent(project)}/${encodeURIComponent(r.run_id)}`
          : `#/project/${encodeURIComponent(project)}/list/${encodeURIComponent(r.run_id)}`,
    });
  }
  return out;
}

/** 日ごとのグラフの横軸の文字（project.js の axisLabels と同じ規則）。 */
function hmAxisLabels(period, daily) {
  const rows = Array.isArray(daily) ? daily : [];
  const n = rows.length;
  // 棒が多すぎるときは間能干して出す（week なら 7 日・all なら長年になりうる）
  const step = n > 14 ? Math.ceil(n / 8) : 1;
  return rows.map((row, i) => {
    if (i % step !== 0) return "";
    const date = String((row && row.date) || "");
    if (!date) return "";
    if (period === "week") return HM_WEEKDAY_JA[new Date(`${date}T00:00:00`).getDay()] || "";
    if (period === "month") {
      const day = Number(date.slice(8, 10));
      return [1, 5, 10, 15, 20, 25, 30].includes(day) ? String(day) : "";
    }
    const cur = date.slice(0, 7);
    const prev = i > 0 ? String((rows[i - 1] && rows[i - 1].date) || "").slice(0, 7) : null;
    return i === 0 || cur !== prev ? `${Number(cur.slice(5, 7))}月` : "";
  });
}

/**
 * GET /api/home の結果から力の並びを作る（DOM は使わない）。
 * [{key, title, count, items, term}] の並びを返す。0 件のカードは入れない。
 */
function hmSections(data) {
  const d = data || {};
  const out = [];

  // 1) 質問がある会話（質問と承認を 1 枚のカードにまとめる・画面は 0 件でも出す）
  const wait = hmWaitItems(d);
  if (wait.length > 0) {
    out.push({ key: "wait", title: "質問がある会話", count: wait.length, items: wait });
  }

  // 2) 確認を待っています（プロジェクトごとの review）
  const review = [];
  for (const p of Array.isArray(d.projects) ? d.projects : []) {
    for (const r of Array.isArray(p.review) ? p.review : []) {
      review.push({
        kind: "review",
        title: r.label || `run ${r.run_id}`,
        where: p.name || "",
        runId: r.run_id,
        ago: "",
        hash: `#/project/${encodeURIComponent(p.name || "")}/list/${encodeURIComponent(r.run_id)}/review`,
      });
    }
  }
  if (review.length > 0) {
    out.push({ key: "review", title: "確認を待っています", count: review.length, items: review, term: "review" });
  }

  // 3) 動いています（実行中のプロジェクト）
  const running = [];
  for (const p of Array.isArray(d.projects) ? d.projects : []) {
    const n = Number(p.running) || 0;
    if (n <= 0) continue;
    running.push({
      kind: "running",
      title: p.name || "(名前のないプロジェクト)",
      sub: `${n} 件 実行中`,
      count: n,
      ago: hmAgo(p.last_executed_at),
      hash: `#/project/${encodeURIComponent(p.name || "")}/workflow`,
    });
  }
  if (running.length > 0) {
    out.push({ key: "running", title: "動いています", count: running.length, items: running });
  }

  // 4) 止まっている常駐（daemon_alive が偽で todo がある）
  const daemon = [];
  for (const p of Array.isArray(d.projects) ? d.projects : []) {
    if (p.daemon_alive !== false) continue;
    if ((Number(p.todo) || 0) <= 0) continue;
    daemon.push({
      kind: "daemon",
      title: p.name || "(名前のないプロジェクト)",
      sub: "常駐が止まっています。カードを実行するには起動してください",
      count: Number(p.todo) || 0,
      ago: "",
      hash: `#/project/${encodeURIComponent(p.name || "")}`,
    });
  }
  if (daemon.length > 0) {
    out.push({ key: "daemon", title: "止まっている常駐", count: daemon.length, items: daemon, term: "daemon" });
  }

  // 5) 論文の要対応（プロジェクトごとに先頭 3 件）
  const papers = [];
  for (const p of Array.isArray(d.papers) ? d.papers : []) {
    const n = Number(p.attention) || 0;
    const items = Array.isArray(p.items) ? p.items : [];
    if (n <= 0 && items.length === 0) continue;
    papers.push({
      kind: "papers",
      title: p.name || "(名前のないプロジェクト)",
      count: n,
      sub: items.slice(0, HM_PAPERS_MAX).map((i) => i.title || "").filter(Boolean),
      ago: "",
      hash: "#/papers/dashboard",
      project: p.name || "",
    });
  }
  if (papers.length > 0) {
    out.push({ key: "papers", title: "論文の要対応", count: papers.length, items: papers, term: "attention" });
  }

  // 6) 読めなかったプロジェクト（一番下にたたんで出す）
  const errors = Array.isArray(d.errors) ? d.errors.filter((e) => e) : [];
  if (errors.length > 0) {
    out.push({ key: "errors", title: "読めなかったプロジェクト", count: errors.length, items: errors });
  }

  return out;
}

/** カードが 1 枚も無いときの案内（"none" なら待つものなし／"no-projects" ならプロジェクト自体が無い）。 */
function hmEmptyKind(data) {
  const d = data || {};
  const projects = Array.isArray(d.projects) ? d.projects : [];
  const papers = Array.isArray(d.papers) ? d.papers : [];
  if (projects.length === 0 && papers.length === 0) return "no-projects";
  return "none";
}

// --- 画面を描く（DOM） ---

/** 小さい要素を 1 つ作る。 */
function hmEl(tag, cls, text) {
  const el = document.createElement(tag);
  if (cls) el.className = cls;
  if (text != null) el.textContent = text;
  return el;
}

/** 1 行（.sai-row が右の › を持つ。where を渡すと右にプロジェクト名が出る）。 */
function hmRow(title, sub, ago, onClick, where) {
  const row = hmEl("button", "sai-row press");
  row.type = "button";
  const body = hmEl("span", "sai-row-body");
  body.appendChild(hmEl("span", "sai-row-title", title));
  if (sub) body.appendChild(hmEl("span", "sai-row-sub", sub));
  if (ago) body.appendChild(hmEl("span", "sai-row-sub", ago));
  row.appendChild(body);
  if (where) row.appendChild(hmEl("span", "hm-where", where));
  if (typeof onClick === "function") row.addEventListener("click", onClick);
  return row;
}

/** カード 1 枚（見出しと中身）。 */
function hmCard(title, term) {
  const card = hmEl("section", "hm-card sai-card");
  const head = hmEl("div", "hm-card-head");
  head.appendChild(hmEl("h2", "hm-card-title", title));
  if (term && typeof saiTerm === "function") head.appendChild(saiTerm(term));
  card.appendChild(head);
  return card;
}

/** 待ち（質問・承認）の 1 行。 */
function hmWaitRow(item) {
  const sub = item.kind === "approval" ? "承認をしてほしいです" : "";
  return hmRow(item.where || "（プロジェクト不明）", `${item.title}${sub ? ` — ${sub}` : ""}`, item.ago, () => {
    location.hash = item.hash;
  });
}

// --- 合計のダッシュボード（U02 §3-1） ---

/** SVG の要素を 1 つ作る（属性は {名前: 値}）。 */
function hmSvg(tag, attrs) {
  const el = document.createElementNS("http://www.w3.org/2000/svg", tag);
  for (const [k, v] of Object.entries(attrs || {})) el.setAttribute(k, String(v));
  return el;
}

/** 日ごとの棒グラフ（SVG を手で描く・見た目のクラスは style.css の .dash-* を使い回す）。 */
function hmDailyChart(overview) {
  const o = overview || {};
  const daily = Array.isArray(o.daily) ? o.daily : [];
  const svg = hmSvg("svg", {
    viewBox: "0 0 320 140",
    class: "dash-chart",
    role: "img",
    "aria-label": "日ごとの実行数",
  });
  if (daily.length === 0) {
    const t = hmSvg("text", { x: "160", y: "70", class: "dash-axis", "text-anchor": "middle" });
    t.textContent = "この期間に実行はありません";
    svg.appendChild(t);
    return svg;
  }
  const values = daily.map((row) => Number(row && row.executions) || 0);
  const max = Math.max(1, ...values);
  const n = daily.length;
  const slot = 320 / n;
  const barW = Math.max(2, Math.min(slot * 0.6, 26));
  const baseY = 116; // 棒の底
  const topY = 8;    // 最大のときの上端
  values.forEach((v, i) => {
    const h = v > 0 ? Math.max(3, (v / max) * (baseY - topY)) : 3;
    const x = i * slot + (slot - barW) / 2;
    const rect = hmSvg("rect", {
      x: x.toFixed(2),
      y: (baseY - h).toFixed(2),
      width: barW.toFixed(2),
      height: h.toFixed(2),
      rx: "3",
      class: v > 0 ? "dash-bar" : "dash-bar zero",
    });
    const tip = hmSvg("title", {});
    tip.textContent = `${(daily[i] && daily[i].date) || ""} ${v} 回`;
    rect.appendChild(tip);
    svg.appendChild(rect);
  });
  hmAxisLabels(o.period || hmPeriodValue(), daily).forEach((text, i) => {
    if (!text) return;
    const t = hmSvg("text", {
      x: (i * slot + slot / 2).toFixed(2),
      y: "134",
      class: "dash-axis",
      "text-anchor": "middle",
    });
    t.textContent = text;
    svg.appendChild(t);
  });
  return svg;
}

/** 期間の切り替え（.dash-seg の見た目）。 */
function hmPeriodSeg() {
  const seg = hmEl("div", "dash-seg small");
  const current = hmPeriodValue();
  for (const [value, label] of HM_PERIODS) {
    const b = hmEl("button", value === current ? "active" : "", label);
    b.type = "button";
    b.addEventListener("click", async () => {
      if (hmPeriodValue() === value) return;
      hmSavePeriod(value);
      // 合計だけ読み直す（ほかのカードは変わらない）
      await hmLoadOverview();
      hmRender();
    });
    seg.appendChild(b);
  }
  return seg;
}

/** 1. 全プロジェクトの合計のカード。 */
function hmOverviewCard() {
  const card = hmEl("section", "hm-card sai-card hm-overview");
  const head = hmEl("div", "hm-card-head");
  head.appendChild(hmEl("h2", "hm-card-title", "全プロジェクトの合計"));
  head.appendChild(hmPeriodSeg());
  card.appendChild(head);

  if (hmOverviewError) {
    card.appendChild(hmEl("p", "hm-note warn", hmOverviewError));
  }

  const grid = hmEl("div", "dash-grid");
  for (const c of hmOverviewCards(hmOverview)) {
    const cell = hmEl("div", "dash-cell");
    cell.appendChild(hmEl("div", "dash-big", c.value));
    cell.appendChild(hmEl("div", "dash-big-label", c.label));
    grid.appendChild(cell);
  }
  card.appendChild(grid);
  card.appendChild(hmDailyChart(hmOverview));
  card.appendChild(hmEl("p", "dash-sub", hmOverviewNote(hmOverview)));
  return card;
}

// --- 待ち・todo・アクティビティ（U02 §3-2〜4） ---

/** カードの中に 1 行だけの案内を出す。 */
function hmNote(text) {
  return hmEl("p", "hm-note", text);
}

/** 「すべて表示（N）」のボタン。 */
function hmMoreButton(count, onClick) {
  const b = hmEl("button", "sai-btn hm-more", `すべて表示（${count}）`);
  b.type = "button";
  b.addEventListener("click", onClick);
  return b;
}

/** 2. 質問がある会話のカード（0 件でも出す）。 */
function hmWaitCard(section) {
  const card = hmCard("質問がある会話");
  const items = section ? section.items : [];
  if (items.length === 0) {
    card.appendChild(hmNote("今、答えを待っている会話はありません"));
    return card;
  }
  for (const item of items) card.appendChild(hmWaitRow(item));
  return card;
}

/** 3. todo のカード（0 件でも出す）。 */
function hmTodoCard() {
  const items = hmTodoItems(hmData);
  const card = hmCard("todo");
  if (items.length === 0) {
    card.appendChild(hmNote("todo はありません"));
    return card;
  }
  const shown = hmShowAll.todo ? items : items.slice(0, HM_TODO_MAX);
  for (const item of shown) {
    card.appendChild(hmRow(item.title, "", "", () => { location.hash = item.hash; }, item.where));
  }
  if (items.length > shown.length) {
    card.appendChild(hmMoreButton(items.length, () => { hmShowAll.todo = true; hmRender(); }));
  }
  return card;
}

/** 4. 最近のアクティビティのカード（0 件でも出す）。 */
function hmRecentCard() {
  const items = hmRecentItems(hmOverview);
  const card = hmCard("最近のアクティビティ");
  if (items.length === 0) {
    card.appendChild(hmNote("最近の実行はありません"));
    return card;
  }
  const shown = hmShowAll.recent ? items.slice(0, HM_RECENT_ALL) : items.slice(0, HM_RECENT_MAX);
  for (const item of shown) {
    card.appendChild(hmRow(item.title, item.sub, item.ago, () => { location.hash = item.hash; }, item.where));
  }
  if (items.length > shown.length) {
    card.appendChild(hmMoreButton(items.length, () => { hmShowAll.recent = true; hmRender(); }));
  }
  return card;
}

/** 画面を描く（#/home）。 */
function hmRender() {
  const root = document.getElementById("home-root");
  if (!root) return;
  root.innerHTML = "";

  const bar = hmEl("header", "appbar");
  bar.appendChild(hmEl("h1", null, "ホーム"));
  root.appendChild(bar);

  if (hmError) {
    root.appendChild(hmEl("div", "offline-bar", hmError));
  }

  const body = hmEl("div", "hm-body");

  if (!hmData) {
    // まだ読めていない
    const box = hmEl("div", "sai-empty");
    box.appendChild(hmEl("p", "sai-empty-body", hmError ? "読み込めませんでした。" : "読み込み中…"));
    body.appendChild(box);
    root.appendChild(body);
    return;
  }

  const sections = hmSections(hmData);
  // プロジェクトが 1 つも無いときだけ、大きな空の案内を出す。
  // 下のカード（合計・質問・todo・アクティビティ）は 0 件でも出す。
  if (hmEmptyKind(hmData) === "no-projects") {
    const box = hmEl("div", "sai-empty");
    box.appendChild(hmEl("div", "sai-empty-icon", "📁"));
    box.appendChild(hmEl("p", "sai-empty-title", "まだプロジェクトがありません"));
    box.appendChild(
      hmEl("p", "sai-empty-body", "Mac で `sai-link init` を実行するとここに出ます。")
    );
    body.appendChild(box);
  }

  // 1〜4（合計・質問がある会話・todo・最近のアクティビティ）は 0 件でも出す。
  body.appendChild(hmOverviewCard());
  body.appendChild(hmWaitCard(sections.find((s) => s.key === "wait") || null));
  body.appendChild(hmTodoCard());
  body.appendChild(hmRecentCard());

  for (const sec of sections) {
    // 質問がある会話は上で出済み（0 件でも出すため）
    if (sec.key === "wait") continue;
    const card = hmCard(sec.title, sec.term);

    if (sec.key === "errors") {
      // 読めなかったプロジェクトは一番下でたたんで出す
      const det = document.createElement("details");
      det.className = "hm-errors";
      det.appendChild(hmEl("summary", null, `${sec.title}（${sec.count}）`));
      for (const e of sec.items) det.appendChild(hmEl("p", "hm-error-line", e));
      card.appendChild(det);
      body.appendChild(card);
      continue;
    }

    for (const item of sec.items) {
      if (sec.key === "review") {
        card.appendChild(hmRow(item.where || "(不明)", item.title, "", () => { location.hash = item.hash; }));
      } else if (sec.key === "running") {
        card.appendChild(hmRow(item.title, item.sub, item.ago, () => { location.hash = item.hash; }));
      } else if (sec.key === "daemon") {
        const row = hmRow(item.title, item.sub, "", () => { location.hash = item.hash; });
        card.appendChild(row);
      } else if (sec.key === "papers") {
        const sub = item.sub.length > 0 ? `要対応 ${item.count} 件 — ${item.sub.join(" / ")}` : "要対応 0 件";
        card.appendChild(hmRow(item.title, sub, "", () => {
          // Papers を開く前にプロジェクトを覚えておく（C-F1）
          try { localStorage.setItem(HM_PAPERS_KEY, item.project); } catch { /* 覚えてなくてもよい */ }
          location.hash = item.hash;
        }));
      }
    }
    body.appendChild(card);
  }

  root.appendChild(body);
}

/** 合計だけ読む（GET /api/home/overview）。期間の切り替えのときもこれだけを読み直す。 */
async function hmLoadOverview() {
  const period = hmPeriodValue();
  try {
    hmOverview = await saiApi(`/api/home/overview?period=${encodeURIComponent(period)}`);
    hmOverview = hmOverview || null;
    hmOverviewError = "";
  } catch (e) {
    // 合計だけ読めなくても、ほかのカードは出す
    hmOverviewError = e && e.message ? e.message : "合計を読めませんでした";
  }
}

/** ホームを読む（GET /api/home と GET /api/home/overview）。 */
async function hmLoad() {
  if (hmLoading) return;
  hmLoading = true;
  try {
    // 2 本を並べる。合計が失敗しても全体の読み込みは落とさないため、失敗も値として受け取る。
    const [data, overview] = await Promise.all([
      saiApi("/api/home").then(
        (v) => ({ ok: true, v }),
        (e) => ({ ok: false, e })
      ),
      saiApi(`/api/home/overview?period=${encodeURIComponent(hmPeriodValue())}`).then(
        (v) => ({ ok: true, v }),
        (e) => ({ ok: false, e })
      ),
    ]);
    if (overview.ok) {
      hmOverview = overview.v || null;
      hmOverviewError = "";
    } else {
      hmOverviewError =
        overview.e && overview.e.message ? overview.e.message : "合計を読めませんでした";
    }
    if (!data.ok) throw data.e;
    hmData = data.v || {};
    hmError = "";
    hmRender();
  } catch (e) {
    // 404（U3 の API がまだ無いとき）は「読めなかった」の案内にする
    hmError = e && e.message ? e.message : "読み込めませんでした";
    if (e && e.status === 404) hmError = "ホームの情報がまだありません（サーバーの返しが 404 でした）";
    hmData = null;
    hmRender();
  } finally {
    hmLoading = false;
  }
}

// --- 画面の登録（ブラウザでのみ） ---

if (typeof window !== "undefined") {
  (window.SaiRoutes = window.SaiRoutes || []).push({
    match(hash) {
      return hash === "#/home" ? {} : null;
    },
    view: "view-home",
    tab: "home",
    hideTabbar: false,
    interval: HM_INTERVAL_MS,
    async open() {
      hmLoading = false;
      await hmLoad();
    },
    async poll() {
      await hmLoad();
    },
  });
}

// --- node からの読み込み（tests/test_home.js。DOM を使わない部分だけ） ---
if (typeof module !== "undefined" && module.exports) {
  module.exports = {
    HM_INTERVAL_MS,
    HM_PAPERS_MAX,
    HM_PAPERS_KEY,
    HM_PERIODS,
    HM_PERIOD_KEY,
    HM_TODO_MAX,
    HM_RECENT_MAX,
    HM_RECENT_ALL,
    hmStamp,
    hmAgo,
    hmWaitTitle,
    hmWaitWhere,
    hmWaitHash,
    hmWaitItems,
    hmSections,
    hmEmptyKind,
    hmPeriodValue,
    hmSavePeriod,
    hmOverviewCards,
    hmOverviewNote,
    hmTodoItems,
    hmRecentItems,
    hmAxisLabels,
  };
}
