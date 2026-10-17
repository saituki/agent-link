// 会話の検索の画面。
//
// `agent-link session search`（エージェントの会話の履歴）と、文書（task / message / waggle）の本文検索を
// Web から使う。住所は `#/project/<name>/sessions`（Project の上部の切り替えの 1 つ）。
//
// - 条件（検索語・run の番号・block・日付・状態・種類）を入れて検索する。
// - 種類の「文書」を選ぶと、文書の本文も探す（文書には状態は効かない）。
// - 結果は run（block_id）ごとにまとめ、会話の行（type・turn・snippet）と文書の行を出す。
// - run の見出しは会話の画面（`#/run/<name>/<block_id>`）へのリンクにする。
//
// 条件の組み立て（sessBuildQuery・sessPlan・sessDocQuery）と結果のまとめ（sessGroup・sessMerge）は
// DOM を使わない純関数で、tests/test_sessionsearch.js が確かめる。
"use strict";

// 選べる状態と種類
const SESS_STATUSES = ["todo", "pending", "running", "completed", "failed", "cancelled"];
const SESS_TYPES = ["user", "agent", "thinking", "tool_call", "tool_name", "tool_result", "tool"];
// 文書の種類（`GET /api/projects/{name}/search` の category）
const SESS_DOC_TYPES = ["task", "message", "waggle"];
// 文書の件数の上限（API の上限 100 より小さくして、多いときは絞り込みを案内する）
const SESS_DOC_LIMIT = 50;

/** 検索の住所からプロジェクト名を取る。違う住所なら null。 */
function sessParseHash(hash) {
  const m = String(hash || "").match(/^#\/project\/([^/]+)\/sessions\/?$/);
  return m ? { name: decodeURIComponent(m[1]) } : null;
}

/**
 * 入力欄の値から検索のクエリ文字列を作る（先頭の `?` は付けない）。
 * values は `{word, id, block, date, status:[…], type:[…]}`。
 * 空の値は入れない。条件が 1 つも無ければ空文字を返す。
 */
function sessBuildQuery(values) {
  const v = values || {};
  const params = new URLSearchParams();
  for (const key of ["word", "id", "block", "date"]) {
    const text = String(v[key] == null ? "" : v[key]).trim();
    if (text) params.set(key, text);
  }
  for (const [key, allowed] of [["status", SESS_STATUSES], ["type", SESS_TYPES]]) {
    const picked = Array.isArray(v[key]) ? v[key].filter((x) => allowed.includes(x)) : [];
    if (picked.length) params.set(key, picked.join(","));
  }
  return params.toString();
}

/** 検索語をカンマで分け、空でない語だけを返す。 */
function sessWords(text) {
  return String(text == null ? "" : text).split(",").map((s) => s.trim()).filter(Boolean);
}

/** `YYYY/M/D` を `YYYY-MM-DD` にする。形が違えば null。 */
function sessIsoDay(text) {
  const m = String(text == null ? "" : text).trim().match(/^(\d{4})\/(\d{1,2})\/(\d{1,2})$/);
  if (!m || +m[2] < 1 || +m[2] > 12 || +m[3] < 1 || +m[3] > 31) return null;
  return `${m[1]}-${m[2].padStart(2, "0")}-${m[3].padStart(2, "0")}`;
}

/**
 * 何を探すかを決める。`{sessions: bool, docs: string[]}`。
 * 会話の種類にも文書の種類にも印が無ければ、両方を探す。印があれば、印のある方だけを探す。
 * 文書は検索語が無いと探さない（API が検索語を要るため）。
 */
function sessPlan(values) {
  const v = values || {};
  const types = (Array.isArray(v.type) ? v.type : []).filter((x) => SESS_TYPES.includes(x));
  const docPicked = (Array.isArray(v.doc) ? v.doc : []).filter((x) => SESS_DOC_TYPES.includes(x));
  const none = types.length === 0 && docPicked.length === 0;
  const docs = none ? SESS_DOC_TYPES.slice() : docPicked;
  return {
    sessions: none || types.length > 0,
    docs: sessWords(v.word).length ? docs : [],
  };
}

/**
 * 文書の検索の文字列（先頭の `?` は付けない）。作れなければ null
 * （検索語が無い・run の番号か日付の形が違う）。検索語のカンマは空白に替える。
 * 種類は 3 つ全部なら絞らない。
 */
function sessDocQuery(values, docs) {
  const v = values || {};
  const words = sessWords(v.word);
  if (!words.length) return null;
  const p = new URLSearchParams();
  p.set("q", words.join(" "));
  const picked = (Array.isArray(docs) ? docs : []).filter((x) => SESS_DOC_TYPES.includes(x));
  if (picked.length && picked.length < SESS_DOC_TYPES.length) p.set("category", picked.join(","));

  const id = String(v.id == null ? "" : v.id).trim();
  if (id) {
    const single = id.match(/^(\d+)$/);
    const range = id.match(/^(\d*)-(\d*)$/);
    if (single) {
      p.set("from_id", single[1]);
      p.set("to_id", single[1]);
    } else if (range && (range[1] || range[2])) {
      if (range[1]) p.set("from_id", range[1]);
      if (range[2]) p.set("to_id", range[2]);
    } else {
      return null;
    }
  }

  const date = String(v.date == null ? "" : v.date).trim();
  if (date) {
    const days = date.split("-").map(sessIsoDay);
    if (days.length === 1 && days[0]) {
      p.set("since", days[0]);
      p.set("until", days[0]);
    } else if (days.length === 2 && days[0] && days[1]) {
      const [a, b] = days[0] <= days[1] ? days : [days[1], days[0]];
      p.set("since", a);
      p.set("until", b);
    } else {
      return null;
    }
  }

  const block = String(v.block == null ? "" : v.block).trim();
  if (block) p.set("block", block);
  p.set("limit", String(SESS_DOC_LIMIT));
  return p.toString();
}

/** 文書の結果 1 件を押したときの行き先（task / message は List の表示、waggle は文書そのもの）。 */
function sessDocHash(name, item) {
  const base = `#/project/${encodeURIComponent(name)}/list/${encodeURIComponent(String(item.run_id))}`;
  if (item.category === "waggle" && item.token) {
    return `${base}/waggle/${encodeURIComponent(item.token)}`;
  }
  return `${base}/${item.category === "message" ? "message" : "task"}`;
}

/** 結果を run（block_id）ごとにまとめる（順は来たまま）。 */
function sessGroup(items) {
  const groups = [];
  const byId = new Map();
  for (const item of items || []) {
    const id = item && item.block_id != null ? String(item.block_id) : "";
    let g = byId.get(id);
    if (!g) {
      g = { blockId: id, items: [] };
      byId.set(id, g);
      groups.push(g);
    }
    g.items.push(item);
  }
  return groups;
}

/**
 * 会話の結果と文書の結果を run ごとにまとめる。`[{blockId, items, docs}]`。
 * 並びは、会話の結果が来た順の後に、文書だけにある run を来た順に足す。
 */
function sessMerge(sessionItems, docItems) {
  const out = [];
  const byId = new Map();
  const slot = (id) => {
    let g = byId.get(id);
    if (!g) {
      g = { blockId: id, items: [], docs: [] };
      byId.set(id, g);
      out.push(g);
    }
    return g;
  };
  for (const group of sessGroup(sessionItems)) {
    slot(group.blockId).items.push(...group.items);
  }
  for (const item of docItems || []) {
    if (item && item.run_id != null) slot(String(item.run_id)).docs.push(item);
  }
  return out;
}

// --- 画面（ブラウザだけ） ---

let sessName = "";
let sessBusy = false;

function sessEl(tag, cls, text) {
  const node = document.createElement(tag);
  if (cls) node.className = cls;
  if (text != null) node.textContent = String(text);
  return node;
}

function sessInput(id, label, placeholder) {
  const wrap = sessEl("div", "sess-field");
  const lab = sessEl("label", "sess-label", label);
  lab.htmlFor = id;
  const input = document.createElement("input");
  input.type = "text";
  input.id = id;
  input.className = "sess-input";
  if (placeholder) input.placeholder = placeholder;
  wrap.append(lab, input);
  return wrap;
}

function sessChecks(container, items, prefix) {
  for (const value of items) {
    const lab = document.createElement("label");
    lab.className = "sess-check";
    const box = document.createElement("input");
    box.type = "checkbox";
    box.value = value;
    box.dataset[prefix] = "1";
    lab.append(box, document.createTextNode(value));
    container.appendChild(lab);
  }
}

async function sessOpen(params) {
  sessName = params.name;
  const title = document.getElementById("sess-title");
  if (title) title.textContent = sessName;
  if (typeof fillLinkSeg === "function") fillLinkSeg("sess-mode-seg", "sessions", sessName);
  const body = document.getElementById("sess-body");
  if (!body) return;
  body.innerHTML = "";
  body.appendChild(sessForm());
  body.appendChild(sessEl("div", "sess-results"));
}

function sessForm() {
  const form = sessEl("div", "sess-form");
  form.appendChild(sessInput("sess-word", "Search words (comma-separated, AND)", "e.g. attention, loss"));
  const grid = sessEl("div", "sess-grid");
  grid.appendChild(sessInput("sess-id", "Run ID", "e.g. 10-20"));
  grid.appendChild(sessInput("sess-block", "Block name", "e.g. worker"));
  grid.appendChild(sessInput("sess-date", "Date", "e.g. 2026/09/01-2026/09/30"));
  form.appendChild(grid);

  const statusRow = sessEl("div", "sess-check-row");
  statusRow.appendChild(sessEl("span", "sess-label", "Status"));
  sessChecks(statusRow, SESS_STATUSES, "sessStatus");
  form.appendChild(statusRow);

  const typeRow = sessEl("div", "sess-check-row");
  typeRow.appendChild(sessEl("span", "sess-label", "Type"));
  sessChecks(typeRow, SESS_TYPES, "sessType");
  form.appendChild(typeRow);

  const docRow = sessEl("div", "sess-check-row");
  docRow.appendChild(sessEl("span", "sess-label", "Documents"));
  sessChecks(docRow, SESS_DOC_TYPES, "sessDoc");
  form.append(
    docRow,
    sessEl("p", "sess-note", "Only search words, run ID, block (exact match) and date apply to documents. Status does not.")
  );

  const row = sessEl("div", "sess-run-row");
  const go = sessEl("button", "sai-btn primary", "Search");
  go.type = "button";
  go.addEventListener("click", () => { void sessSearch(); });
  row.appendChild(go);
  form.appendChild(row);
  return form;
}

function sessReadValues() {
  const val = (id) => {
    const node = document.getElementById(id);
    return node ? node.value : "";
  };
  const checks = (attr) =>
    Array.from(document.querySelectorAll(`input[data-${attr}]`))
      .filter((x) => x.checked)
      .map((x) => x.value);
  return {
    word: val("sess-word"),
    id: val("sess-id"),
    block: val("sess-block"),
    date: val("sess-date"),
    status: checks("sess-status"),
    type: checks("sess-type"),
    doc: checks("sess-doc"),
  };
}

async function sessSearch() {
  if (sessBusy) return;
  const values = sessReadValues();
  const plan = sessPlan(values);
  const sessQuery = plan.sessions ? sessBuildQuery(values) : "";
  const docQuery = plan.docs.length ? sessDocQuery(values, plan.docs) : null;
  if (plan.sessions && !sessQuery) {
    saiToast("Enter at least one search condition.", "error");
    return;
  }
  if (!plan.sessions && !plan.docs.length) {
    saiToast("Enter a search word to search documents.", "error");
    return;
  }
  if (plan.docs.length && !docQuery) {
    saiToast("The run ID or date format for the document search is invalid.", "error");
    return;
  }
  const results = document.querySelector(".sess-results");
  if (results) results.textContent = "Searching…";
  const name = encodeURIComponent(sessName);
  sessBusy = true;
  try {
    const [s, d] = await Promise.allSettled([
      plan.sessions ? saiApi(`/api/projects/${name}/sessions/search?${sessQuery}`) : Promise.resolve(null),
      docQuery ? saiApi(`/api/projects/${name}/search?${docQuery}`) : Promise.resolve(null),
    ]);
    if (results) {
      results.innerHTML = "";
      const sessOut = s.status === "fulfilled" ? s.value : null;
      const docOut = d.status === "fulfilled" ? d.value : null;
      if (s.status === "rejected") results.appendChild(sessEl("p", "sess-error", `Sessions: ${(s.reason && s.reason.message) || "Search failed."}`));
      if (d.status === "rejected") results.appendChild(sessEl("p", "sess-error", `Documents: ${(d.reason && d.reason.message) || "Search failed."}`));
      const sessItems = (sessOut && sessOut.items) || [];
      const docItems = (docOut && docOut.items) || [];
      sessRenderResults(results, sessMerge(sessItems, docItems), {
        sessCount: sessOut ? sessItems.length : null,
        sessTruncated: !!(sessOut && sessOut.truncated),
        docCount: docOut ? docItems.length : null,
        docTruncated: !!(docOut && docOut.truncated),
      });
    }
  } finally {
    sessBusy = false;
  }
}

/** info は `{sessCount, docCount}`（探していなければ・失敗したら null）と打ち切りの印。 */
function sessRenderResults(container, groups, info) {
  const { sessCount, docCount, sessTruncated, docTruncated } = info;
  if (sessCount == null && docCount == null) return; // 両方とも失敗した（理由は上に出してある）
  const total = (sessCount || 0) + (docCount || 0);
  if (!total) {
    container.appendChild(sessEl("p", "lib-empty", "No results found."));
    return;
  }
  const parts = [];
  if (sessCount != null) parts.push(`Sessions: ${sessCount}${sessTruncated ? " (first 200)" : ""}`);
  if (docCount != null) parts.push(`Documents: ${docCount}${docTruncated ? ` (first ${SESS_DOC_LIMIT})` : ""}`);
  container.appendChild(sessEl("p", "sess-count", parts.join(" · ")));
  for (const group of groups) {
    const section = sessEl("section", "sess-run");
    const head = sessEl("div", "sess-run-head");
    const a = document.createElement("a");
    a.href = `#/run/${encodeURIComponent(sessName)}/${encodeURIComponent(group.blockId)}`;
    a.textContent = `run ${group.blockId}`;
    head.appendChild(a);
    head.appendChild(sessEl("span", "sess-run-count", `${group.items.length + group.docs.length} hits`));
    section.appendChild(head);
    for (const item of group.items) {
      const row = sessEl("div", "sess-hit");
      row.appendChild(sessEl("span", "sess-hit-type", item.type || ""));
      if (item.turn != null) row.appendChild(sessEl("span", "sess-hit-turn", `turn ${item.turn}`));
      const snippet = sessEl("div", "sess-hit-text", item.snippet || "");
      section.append(row, snippet);
    }
    for (const doc of group.docs) section.appendChild(sessDocRow(doc));
    container.appendChild(section);
  }
}

/** 文書の結果 1 件（押すと行き先へ）。 */
function sessDocRow(item) {
  const a = document.createElement("a");
  a.className = "sess-doc";
  a.href = sessDocHash(sessName, item);
  a.append(
    sessEl("span", "sess-hit-type", `[${item.category}] `),
    sessEl("span", "sess-doc-file", `${item.file || ""} ${item.date || ""}`),
    sessEl("div", "sess-hit-text", item.snippet || "")
  );
  return a;
}

// --- 画面の登録（ブラウザだけ） ---

if (typeof window !== "undefined") {
  (window.SaiRoutes = window.SaiRoutes || []).push({
    match(hash) {
      return sessParseHash(hash);
    },
    view: "view-sessions",
    tab: "project",
    hideTabbar: false,
    interval: 0,
    async open(params) {
      await sessOpen(params);
    },
    async poll() {},
  });

  const back = document.getElementById("sess-back");
  if (back) back.addEventListener("click", () => { location.hash = "#/projects"; });
}

// --- node からの読み込み（tests/test_sessionsearch.js） ---
if (typeof module !== "undefined" && module.exports) {
  module.exports = {
    SESS_STATUSES,
    SESS_TYPES,
    SESS_DOC_TYPES,
    SESS_DOC_LIMIT,
    sessParseHash,
    sessBuildQuery,
    sessGroup,
    sessWords,
    sessIsoDay,
    sessPlan,
    sessDocQuery,
    sessMerge,
    sessDocHash,
  };
}
