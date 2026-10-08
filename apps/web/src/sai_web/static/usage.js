// sai-web の Usage（実装計画 42 §3.5）の本体。
//
// 画面は 2 つ:
//   カード一覧     Account（#/account）の区画。住所 #/usage は #/account へ送る（U05）
//   詳細           #/usage/<provider>/<account>（Activity | Logs）
// カード・切り替え・棒グラフの見た目は Project タブ（06）の .dash-* をそのまま
// 使い回す（角丸カードは .dash-card，切り替えは .dash-seg，数字は stats.js の
// formatMoney / formatTokens）。データは GET /api/usage… から取る（42 §3.3）。
// 外部ライブラリは入れない（グラフは SVG 手描き）。
"use strict";

// --- 表示の状態 ---

let usageData = null;        // カード一覧の中身
let usageMsg = "";           // 読み込めなかったときの 1 行
let usageLoading = false;    // 読み込み中（二重に走らせない）
let usageForced = false;     // ↻ のあと 1 度だけ ?refresh=1 を付ける
let usageTarget = null;      // 詳細中の {provider, account}
let usageMode = "activity";  // activity | logs
let usageMetric = "cost";    // cost | tokens | calls（Activity のグラフ）
let usageNote = "";          // 棒を押したときの 1 行
let usageDetail = null;      // Activity の中身
let usageLogs = [];          // Logs の今まで読んだ分
let usageLogsTotal = 0;      // Logs の全件数
let usageLogsLoading = false;

// --- 小さな表示の道具 ---

/** 金額（負の値は -$0.01 の形・42 §3.1）． */
function usageMoney(n) {
  const v = Number(n);
  if (!Number.isFinite(v)) return "$0.00";
  return v < 0 ? `-$${Math.abs(v).toFixed(2)}` : `$${v.toFixed(2)}`;
}

/** 「◯分前に取得」（一覧と詳細の上に出す・42 §3.1）． */
function usageFetchedAgo(savedAt) {
  const t = Number(savedAt);
  if (!Number.isFinite(t) || t <= 0) return "";
  const diff = Math.max(0, Math.floor(Date.now() / 1000 - t));
  if (diff < 60) return "たった今 取得";
  const min = Math.floor(diff / 60);
  if (min < 60) return `${min}分前に取得`;
  const h = Math.floor(min / 60);
  if (h < 24) return `${h}時間前に取得`;
  return `${Math.floor(h / 24)}日前に取得`;
}

/** ログの時刻（9/24 14:30）． */
function usageClock(tsMs) {
  const d = new Date(Number(tsMs));
  if (isNaN(d)) return "";
  return d.toLocaleString("ja-JP", {
    month: "numeric",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  });
}

/** 詳細画面の住所． */
function usageHash(provider, account) {
  return `#/usage/${encodeURIComponent(provider)}/${encodeURIComponent(account)}`;
}

/** Activity | Logs のような切り替え（.dash-seg を使い回す）． */
function usageSegBox(items, active, onPick, cls = "dash-seg") {
  const box = document.createElement("div");
  box.className = cls;
  for (const [value, label] of items) {
    const b = document.createElement("button");
    b.textContent = label;
    if (value === active) b.className = "active";
    b.addEventListener("click", () => {
      if (value === active) return;
      onPick(value);
    });
    box.appendChild(b);
  }
  return box;
}

/** カードの見出し（押せるものは › を付け，カード全体を押せるようにする）． */
function usageCard(title, onOpen) {
  const card = el("div", "dash-card");
  const head = el("div", "dash-card-head");
  head.appendChild(el("h2", "dash-card-title", title));
  if (typeof onOpen === "function") {
    head.appendChild(el("span", "dash-r-time", "›"));
    card.style.cursor = "pointer";
    card.setAttribute("role", "button");
    card.tabIndex = 0;
    card.addEventListener("click", onOpen);
    card.addEventListener("keydown", (e) => {
      if (e.key === "Enter" || e.key === " ") {
        e.preventDefault();
        onOpen();
      }
    });
  }
  card.appendChild(head);
  return card;
}

// --- カード一覧（Account の区画・U05） ---

/** OpenRouter の 1 アカウント（42 §3.1）． */
function usageOpenrouterCard(account) {
  const card = usageCard(`OpenRouter · ${account.account}`, () => {
    location.hash = usageHash(account.provider, account.account);
  });
  if (account.error) {
    card.appendChild(el("p", "dash-empty", account.error));
    return card;
  }
  const u = account.usage || {};
  if (account.balance != null) {
    const buy = account.total_credits != null ? ` / 購入 ${formatMoney(account.total_credits)}` : "";
    card.appendChild(el("div", "usage-line", `残高 ${usageMoney(account.balance)}${buy}`));
  }
  card.appendChild(
    el(
      "div",
      "usage-line",
      `使用料 今日 ${formatMoney(u.daily || 0)} · 週 ${formatMoney(u.weekly || 0)} · 月 ${formatMoney(u.monthly || 0)}`
    )
  );
  const free = account.free_model_daily_requests;
  if (free) {
    card.appendChild(
      el(
        "div",
        "usage-line",
        `無料モデル 今日 ${Number(free.used) || 0} / ${Number(free.limit) || 0} 回`
      )
    );
  }
  if (account.limit == null) {
    card.appendChild(el("div", "usage-line muted", "キーの上限 なし"));
  } else {
    const remain = account.limit_remaining == null ? "" : `（残り ${usageMoney(account.limit_remaining)}）`;
    card.appendChild(el("div", "usage-line muted", `キーの上限 ${usageMoney(account.limit)}${remain}`));
  }
  return card;
}

/** DeepSeek（残高は API・使用料は pi の記録から・42 §3.1）． */
function usageDeepseekCard(d) {
  const card = usageCard("DeepSeek", () => {
    location.hash = usageHash("deepseek", "deepseek");
  });
  if (d.error) {
    card.appendChild(el("p", "dash-empty", d.error));
    return card;
  }
  const line = el("div", "usage-line");
  line.textContent = d.balance == null ? "残高 不明" : `残高 ${usageMoney(d.balance)}`;
  if (d.is_available === false) {
    line.textContent += "（使用不可）";
    line.className = "usage-line usage-danger";
  }
  card.appendChild(line);
  const u = d.usage || {};
  card.appendChild(
    el("div", "usage-line", `使用料 今日 ${formatMoney(u.today || 0)} · 月 ${formatMoney(u.month || 0)}`)
  );
  card.appendChild(el("div", "usage-line muted", "使用料は pi の記録から（公開された固定の制限なし）"));
  return card;
}

/** 設定に無い provider（別枠・42 §3.2）． */
function usageUnknownCard(list) {
  const card = el("div", "dash-card");
  card.appendChild(el("h2", "dash-card-title", "設定に無い provider"));
  for (const p of list) {
    const row = el("div", "usage-log");
    row.appendChild(el("span", "usage-log-model", p.provider || "(不明)"));
    row.appendChild(el("span", "usage-log-num", `${Number(p.calls) || 0}回`));
    row.appendChild(el("span", "usage-log-num", formatTokens(p.tokens)));
    row.appendChild(el("span", "usage-log-cost", formatMoney(p.cost || 0)));
    card.appendChild(row);
  }
  card.appendChild(
    el("div", "usage-line muted", "models.json / auth.json と .env の対応が取れない provider")
  );
  return card;
}

/** カード一覧を描く． */
function usageRender() {
  const box = document.getElementById("usage-list");
  if (!box) return;
  box.innerHTML = "";
  const updated = document.getElementById("usage-updated");
  if (updated) updated.textContent = usageData ? usageFetchedAgo(usageData.saved_at) : "";
  if (!usageData) {
    if (usageMsg) {
      // 読めなかったときは案内と「読み直す」を出す（U4 §4）
      box.appendChild(
        saiEmpty({
          icon: "⚠️",
          title: "使用量を読めませんでした",
          body: usageMsg,
          action: { label: "読み直す", onClick: () => { usageForced = true; void usageLoad(); } },
        })
      );
    } else {
      box.appendChild(el("p", "dash-empty", "読み込み中…"));
    }
    return;
  }
  // 広い画面では 2〜3 列に並べる（style.css の .usage-cards）
  const cards = el("div", "dash-cards usage-cards");
  for (const account of usageData.openrouter || []) {
    cards.appendChild(usageOpenrouterCard(account));
  }
  if (usageData.deepseek) cards.appendChild(usageDeepseekCard(usageData.deepseek));
  if ((usageData.unknown || []).length > 0) {
    cards.appendChild(usageUnknownCard(usageData.unknown));
  }
  box.appendChild(cards);
}

/** カード一覧を読み直す（42 §3.3 の GET /api/usage）． */
async function usageLoad() {
  if (usageLoading) return;
  usageLoading = true;
  try {
    const url = `/api/usage${usageForced ? "?refresh=1" : ""}`;
    usageForced = false;
    const res = await apiFetch(url);
    if (!res.ok) throw new Error(await res.text());
    usageData = await res.json();
    usageMsg = "";
    window.SaiShell.ok();
    usageRender();
  } catch {
    usageMsg = "読み込めませんでした";
    window.SaiShell.fail();
    usageRender();
  } finally {
    usageLoading = false;
  }
}

/** Usage の区画を container に描き，カード一覧を開く（acAccountOpen から呼ぶ）． */
function usageMountInto(container) {
  const card = el("div", "sai-card ac-card");
  const head = el("div", "ac-card-head");
  head.appendChild(el("h2", "ac-title", "Usage"));
  // ↻ で外部 API を取り直す（?refresh=1）
  const reload = el("button", "sai-btn small", "↻");
  reload.type = "button";
  reload.id = "usage-reload";
  reload.setAttribute("aria-label", "Reload");
  reload.addEventListener("click", () => {
    usageForced = true;
    void usageLoad();
  });
  head.appendChild(reload);
  card.appendChild(head);
  // 「◯分前に取得」の 1 行（42 §3.1）
  const updated = el("div", "dash-chart-note");
  updated.id = "usage-updated";
  card.appendChild(updated);
  const list = el("div", "groups");
  list.id = "usage-list";
  card.appendChild(list);
  container.appendChild(card);
  void usageOpen();
}

/** カード一覧を開く． */
async function usageOpen() {
  usageData = null;
  usageMsg = "";
  document.getElementById("usage-list").innerHTML = "<p class='dash-empty'>読み込み中…</p>";
  const updated = document.getElementById("usage-updated");
  if (updated) updated.textContent = "";
  await usageLoad();
}

// --- 詳細（Activity | Logs・42 §3.1・§3.5） ---

/** 棒を押したときの 1 行（「9/24 $0.51」）． */
function usageBarNote(row) {
  const date = String((row && row.date) || "");
  if (!date) return "";
  const md = `${Number(date.slice(5, 7))}/${Number(date.slice(8, 10))}`;
  let value;
  if (usageMetric === "tokens") value = formatTokens(row.tokens);
  else if (usageMetric === "calls") value = `${Number(row.calls) || 0}回`;
  else value = formatMoney(row.cost || 0);
  return `${md} ${value}`;
}

/** 日ごとの棒グラフ（共通部品の saiDailyBars を使う。Project と同じ形・42 §3.5）． */
function usageDailySvg(days) {
  const valueOf = (row) => {
    if (usageMetric === "tokens") return Number(row.tokens) || 0;
    if (usageMetric === "calls") return Number(row.calls) || 0;
    return Number(row.cost) || 0;
  };
  const unit = usageMetric === "tokens" ? "tokens" : usageMetric === "calls" ? "count" : "money";
  const what =
    usageMetric === "tokens" ? "日ごとの token" : usageMetric === "calls" ? "日ごとの回数" : "日ごとの費用";
  return saiDailyBars({
    rows: Array.isArray(days) ? days : [],
    valueOf,
    unit,
    what,
    labelOf: (row) => String(String((row && row.date) || "").slice(8, 10)),
    onPick: (row) => {
      usageNote = usageBarNote(row);
      usageRenderDetail();
    },
  });
}

/** Activity（日ごとのグラフとモデル別・42 §3.1）． */
function usageActivityCard(data) {
  const card = el("div", "dash-card");
  // 費用 / token / 回数 の切り替え
  const head = el("div", "dash-card-head");
  head.appendChild(el("h2", "dash-card-title", usageMetric === "tokens" ? "日ごとの token" : usageMetric === "calls" ? "日ごとの回数" : "日ごとの費用"));
  head.appendChild(
    usageSegBox(
      [["cost", "費用"], ["tokens", "token"], ["calls", "回数"]],
      usageMetric,
      (value) => {
        usageMetric = value;
        usageNote = "";
        usageRenderDetail();
      },
      "dash-seg small usage-seg3"
    )
  );
  card.appendChild(head);
  card.appendChild(el("div", "dash-chart-note", usageNote));
  card.appendChild(usageDailySvg(data.days));

  // モデル別（回数・token・費用・費用の多い順）
  const models = Array.isArray(data.by_model) ? data.by_model : [];
  card.appendChild(el("h2", "dash-card-title", "モデル別"));
  if (models.length === 0) {
    card.appendChild(el("p", "dash-empty", "データがありません"));
    return card;
  }
  const totalCost = models.reduce((a, m) => a + (Number(m.cost) || 0), 0);
  const max = Math.max(1e-9, ...models.map((m) => (totalCost > 0 ? Number(m.cost) || 0 : Number(m.tokens) || 0)));
  for (const m of models.slice(0, 20)) {
    // モデル名は長く似た名前が多いため，棒の上の 1 行に全部出す
    const row = el("div", "dash-bar-row usage-model-row");
    row.appendChild(el("span", "dash-bar-name", m.model || "(不明)"));
    const track = el("div", "dash-bar-track");
    const fill = el("div", "dash-bar-fill");
    const basis = totalCost > 0 ? Number(m.cost) || 0 : Number(m.tokens) || 0;
    fill.style.width = `${Math.max(2, (basis / max) * 100)}%`;
    track.appendChild(fill);
    row.appendChild(track);
    row.appendChild(
      el(
        "span",
        "dash-bar-num",
        `${Number(m.calls) || 0}回 · ${formatTokens(m.tokens)} · ${formatMoney(m.cost || 0)}`
      )
    );
    card.appendChild(row);
  }
  return card;
}

/** Logs（1 回ずつの一覧・42 §3.1）． */
function usageLogsCard() {
  const card = el("div", "dash-card");
  card.appendChild(el("h2", "dash-card-title", `Logs（${usageLogsTotal} 件）`));
  if (usageLogs.length === 0) {
    card.appendChild(el("p", "dash-empty", "記録がありません"));
    return card;
  }
  for (const r of usageLogs) {
    const row = el("div", "usage-log");
    row.appendChild(el("span", "usage-log-time", usageClock(r.timestamp)));
    row.appendChild(el("span", "usage-log-model", r.model || "(不明)"));
    row.appendChild(
      el("span", "usage-log-num", `in ${formatTokens(r.input)} / out ${formatTokens(r.output)}`)
    );
    row.appendChild(el("span", "usage-log-cost", formatMoney(r.cost || 0)));
    row.appendChild(el("span", "usage-log-num", r.provider || ""));
    card.appendChild(row);
  }
  if (usageLogs.length < usageLogsTotal) {
    card.appendChild(el("p", "dash-empty", "下までスクロールで続きを読み込みます"));
  }
  return card;
}

/** 詳細の中身を描く． */
function usageRenderDetail() {
  const modeBox = document.getElementById("usage-detail-mode");
  if (modeBox && usageTarget) {
    modeBox.innerHTML = "";
    modeBox.appendChild(
      usageSegBox(
        [["activity", "Activity"], ["logs", "Logs"]],
        usageMode,
        (value) => {
          usageMode = value;
          usageNote = "";
          if (value === "logs") void usageLoadLogs(false);
          else void usageDetailLoad();
        }
      )
    );
  }
  const body = document.getElementById("usage-detail-body");
  if (!body) return;
  body.innerHTML = "";
  const note = el("div", "usage-line muted", "pi の記録から集計。pi 以外の使用は含まない。");
  body.appendChild(note);
  if (usageMode === "logs") {
    body.appendChild(usageLogsCard());
  } else if (usageDetail) {
    body.appendChild(usageActivityCard(usageDetail));
  } else {
    body.appendChild(el("p", "dash-empty", "読み込み中…"));
  }
}

/** Activity を読み直す（GET …/activity）． */
async function usageDetailLoad() {
  if (!usageTarget) return;
  const { provider, account } = usageTarget;
  try {
    const res = await apiFetch(
      `/api/usage/${encodeURIComponent(provider)}/${encodeURIComponent(account)}/activity`
    );
    if (!res.ok) throw new Error(await res.text());
    usageDetail = await res.json();
    usageNote = "";
    window.SaiShell.ok();
    usageRenderDetail();
  } catch {
    window.SaiShell.fail();
  }
}

/** Logs を読む（more は続き・42 §3.1 の 50 件ずつ）． */
async function usageLoadLogs(more) {
  if (!usageTarget || usageLogsLoading) return;
  if (more && (usageLogs.length >= usageLogsTotal || usageLogsTotal === 0)) return;
  usageLogsLoading = true;
  const { provider, account } = usageTarget;
  const offset = more ? usageLogs.length : 0;
  try {
    const res = await apiFetch(
      `/api/usage/${encodeURIComponent(provider)}/${encodeURIComponent(account)}/logs?offset=${offset}&limit=50`
    );
    if (!res.ok) throw new Error(await res.text());
    const data = await res.json();
    usageLogsTotal = Number(data.total) || 0;
    const add = data.logs || [];
    usageLogs = more ? usageLogs.concat(add) : add;
    window.SaiShell.ok();
    usageRenderDetail();
  } catch {
    window.SaiShell.fail();
  } finally {
    usageLogsLoading = false;
  }
}

/** 詳細を開く． */
async function usageDetailOpen(params) {
  usageTarget = { provider: params.provider, account: params.account };
  usageMode = "activity";
  usageMetric = "cost";
  usageNote = "";
  usageDetail = null;
  usageLogs = [];
  usageLogsTotal = 0;
  const title = document.getElementById("usage-detail-title");
  if (title) {
    title.textContent =
      params.provider === "deepseek" ? "DeepSeek" : `OpenRouter · ${params.account}`;
  }
  usageRenderDetail();
  await usageDetailLoad();
}

/** 下まで来たら Logs の続きを読む． */
function usageOnScroll() {
  if (!usageTarget || usageMode !== "logs") return;
  if (usageLogsLoading || usageLogs.length >= usageLogsTotal) return;
  const nearBottom =
    window.innerHeight + window.scrollY >= document.documentElement.scrollHeight - 300;
  if (nearBottom) void usageLoadLogs(true);
}

// --- 画面の登録（ブラウザでのみ・42 §3.5） ---

if (typeof window !== "undefined") {
  (window.SaiRoutes = window.SaiRoutes || []).push(
    {
      // 一覧は Account の区画に入った（U05）。古いリンク・通知のため #/account へ送る
      match(hash) {
        return hash === "#/usage" ? {} : null;
      },
      view: "view-account",
      tab: "account",
      hideTabbar: false,
      interval: 0,
      open() {
        location.replace("#/account");
      },
    },
    {
      // 詳細（Activity | Logs）
      match(hash) {
        const m = hash.match(/^#\/usage\/([^/]+)\/([^/]+)$/);
        return m
          ? { provider: decodeURIComponent(m[1]), account: decodeURIComponent(m[2]) }
          : null;
      },
      view: "view-usage-detail",
      tab: "account",
      hideTabbar: false,
      interval: 60000,
      async open(params) {
        await usageDetailOpen(params);
      },
      async poll() {
        if (!usageTarget) return;
        if (usageMode === "logs") await usageLoadLogs(false);
        else await usageDetailLoad();
      },
    }
  );

  const detailReload = document.getElementById("usage-detail-reload");
  if (detailReload) {
    detailReload.addEventListener("click", () => {
      if (usageMode === "logs") void usageLoadLogs(false);
      else void usageDetailLoad();
    });
  }
  // 詳細の戻りは Account へ（カード一覧は Account の区画）
  const back = document.getElementById("usage-back");
  if (back) {
    back.addEventListener("click", () => {
      location.hash = "#/account";
    });
  }
  window.addEventListener("scroll", usageOnScroll, { passive: true });
}
