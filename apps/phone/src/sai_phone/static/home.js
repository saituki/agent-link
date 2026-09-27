"use strict";
// sai-phone のホーム画面（実装計画 U4・住所 #/home・枠 view-home・タブ home）。
//
// 端末（ターミナル）に慣れていない人が最初に開く画面。ここに
//   1. 答えを待っているもの（質問・承認）
//   2. 確認を待っているもの（review）
//   3. 動いているもの（実行中）
//   4. 止まっている常駐
//   5. 論文の要対応
// を並べ、下に読めなかったプロジェクトを出す。どれも 0 件ならそのカードを出さない。
//
// データは GET /api/home 1 本だけ（共有契約 C-A3。U3 が作る）。
// 15 秒ごとに読み直す。DOM を使わない関数（hmSections など）は
// 末尾で node に渡して tests/test_home.js で確かめる。
// 共通部品は ui.js（saiApi / saiEmpty / saiTerm）を使う。
// 一番外側の名前は hm / HM_ で始める（C-F6）。

// --- 定数 ---

// ホームを読み直す間隔（ミリ秒）
const HM_INTERVAL_MS = 15000;
// 論文の要対応で出す件数
const HM_PAPERS_MAX = 3;
// Papers を開いたとき_LAST プロジェクトを覚えておく場所（papers.js と同じ C-F1）
const HM_PAPERS_KEY = "sai_phone_papers_project";

// --- 表示の状態 ---
let hmData = null;      // 直前に読んだ /api/home の内容
let hmLoading = false;  // 二重に走らせない
let hmError = "";       // 読めなかったときの 1 行

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

/**
 * GET /api/home の結果から力の並びを作る（DOM は使わない）。
 * [{key, title, count, items, term}] の並びを返す。0 件のカードは入れない。
 */
function hmSections(data) {
  const d = data || {};
  const out = [];

  // 1) 答えを待っています（質問と承認を 1 枚のカードにまとめる）
  const wait = [];
  for (const q of Array.isArray(d.questions) ? d.questions : []) {
    wait.push({
      kind: "question",
      title: hmWaitTitle(q, "question"),
      where: hmWaitWhere(q),
      ago: hmAgo(q.created_at),
      hash: hmWaitHash(q),
    });
  }
  for (const a of Array.isArray(d.approvals) ? d.approvals : []) {
    wait.push({
      kind: "approval",
      title: hmWaitTitle(a, "approval"),
      where: hmWaitWhere(a),
      ago: hmAgo(a.created_at),
      hash: hmWaitHash(a),
    });
  }
  if (wait.length > 0) {
    out.push({ key: "wait", title: "答えを待っています", count: wait.length, items: wait });
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

/** 1 行（.sai-row が右の › を持つ）。 */
function hmRow(title, sub, ago, onClick) {
  const row = hmEl("button", "sai-row press");
  row.type = "button";
  const body = hmEl("span", "sai-row-body");
  body.appendChild(hmEl("span", "sai-row-title", title));
  if (sub) body.appendChild(hmEl("span", "sai-row-sub", sub));
  if (ago) body.appendChild(hmEl("span", "sai-row-sub", ago));
  row.appendChild(body);
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
  if (sections.length === 0) {
    const kind = hmEmptyKind(hmData);
    const box = hmEl("div", "sai-empty");
    if (kind === "no-projects") {
      box.appendChild(hmEl("div", "sai-empty-icon", "📁"));
      box.appendChild(hmEl("p", "sai-empty-title", "まだプロジェクトがありません"));
      box.appendChild(
        hmEl("p", "sai-empty-body", "Mac で `sai-link init` を実行するとここに出ます。")
      );
    } else {
      box.appendChild(hmEl("div", "sai-empty-icon", "✅"));
      box.appendChild(hmEl("p", "sai-empty-title", "今、答えることはありません"));
      box.appendChild(hmEl("p", "sai-empty-body", "プロジェクトでカードを実行すると、ここに出ます。"));
      const btn = hmEl("button", "sai-btn primary", "プロジェクトを見る");
      btn.type = "button";
      btn.addEventListener("click", () => { location.hash = "#/projects"; });
      box.appendChild(btn);
    }
    body.appendChild(box);
    root.appendChild(body);
    return;
  }

  for (const sec of sections) {
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
      if (sec.key === "wait") {
        card.appendChild(hmWaitRow(item));
      } else if (sec.key === "review") {
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

/** ホームを読む（GET /api/home）。 */
async function hmLoad() {
  if (hmLoading) return;
  hmLoading = true;
  try {
    const data = await saiApi("/api/home");
    hmData = data || {};
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
    hmStamp,
    hmAgo,
    hmWaitTitle,
    hmWaitWhere,
    hmWaitHash,
    hmSections,
    hmEmptyKind,
  };
}
