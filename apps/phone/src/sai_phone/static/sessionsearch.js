// 会話の検索の画面（U10）。
//
// `sai-link session search`（エージェントの会話の履歴）を Web から使う。
// 住所は `#/project/<name>/sessions`（Project の上部の切り替えの 1 つ）。
//
// - 条件（検索語・Round・round・run の番号・block・日付・状態・種類）を入れて検索する。
// - 結果は run（block_id）ごとにまとめ、type・turn・snippet を出す。
// - run の見出しは会話の画面（`#/run/<name>/<block_id>`）へのリンクにする。
//
// 条件の組み立て（sessBuildQuery）と結果のまとめ（sessGroup）は DOM を使わない純関数で、
// tests/test_sessionsearch.js が確かめる。
"use strict";

// 選べる状態と種類（共有契約「会話の検索の口」）
const SESS_STATUSES = ["todo", "pending", "running", "completed", "failed", "cancelled"];
const SESS_TYPES = ["user", "agent", "thinking", "tool_call", "tool_name", "tool_result", "tool"];

/** 検索の住所からプロジェクト名を取る。違う住所なら null。 */
function sessParseHash(hash) {
  const m = String(hash || "").match(/^#\/project\/([^/]+)\/sessions\/?$/);
  return m ? { name: decodeURIComponent(m[1]) } : null;
}

/**
 * 入力欄の値から検索のクエリ文字列を作る（先頭の `?` は付けない）。
 * values は `{word, global_round, round, id, block, date, status:[…], type:[…]}`。
 * 空の値は入れない。条件が 1 つも無ければ空文字を返す。
 */
function sessBuildQuery(values) {
  const v = values || {};
  const params = new URLSearchParams();
  for (const key of ["word", "global_round", "round", "id", "block", "date"]) {
    const text = String(v[key] == null ? "" : v[key]).trim();
    if (text) params.set(key, text);
  }
  for (const [key, allowed] of [["status", SESS_STATUSES], ["type", SESS_TYPES]]) {
    const picked = Array.isArray(v[key]) ? v[key].filter((x) => allowed.includes(x)) : [];
    if (picked.length) params.set(key, picked.join(","));
  }
  return params.toString();
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
  form.appendChild(sessInput("sess-word", "検索語（カンマ区切りで AND）", "例: attention, mlflow"));
  const grid = sessEl("div", "sess-grid");
  grid.appendChild(sessInput("sess-global-round", "Round（N または N-M）", "例: 2-4"));
  grid.appendChild(sessInput("sess-round", "round", "例: 1"));
  grid.appendChild(sessInput("sess-id", "run の番号", "例: 10-20"));
  grid.appendChild(sessInput("sess-block", "block の名前", "例: worker"));
  grid.appendChild(sessInput("sess-date", "日付", "例: 2026/09/01-2026/09/30"));
  form.appendChild(grid);

  const statusRow = sessEl("div", "sess-check-row");
  statusRow.appendChild(sessEl("span", "sess-label", "状態"));
  sessChecks(statusRow, SESS_STATUSES, "sessStatus");
  form.appendChild(statusRow);

  const typeRow = sessEl("div", "sess-check-row");
  typeRow.appendChild(sessEl("span", "sess-label", "種類"));
  sessChecks(typeRow, SESS_TYPES, "sessType");
  form.appendChild(typeRow);

  const row = sessEl("div", "sess-run-row");
  const go = sessEl("button", "sai-btn primary", "検索");
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
    global_round: val("sess-global-round"),
    round: val("sess-round"),
    id: val("sess-id"),
    block: val("sess-block"),
    date: val("sess-date"),
    status: checks("sess-status"),
    type: checks("sess-type"),
  };
}

async function sessSearch() {
  if (sessBusy) return;
  const query = sessBuildQuery(sessReadValues());
  if (!query) {
    saiToast("検索の条件を 1 つ以上入れてください", "error");
    return;
  }
  const results = document.querySelector(".sess-results");
  if (results) results.textContent = "検索中…";
  sessBusy = true;
  try {
    const out = await saiApi(
      `/api/projects/${encodeURIComponent(sessName)}/sessions/search?${query}`
    );
    if (results) {
      results.innerHTML = "";
      sessRenderResults(results, (out && out.items) || [], !!(out && out.truncated));
    }
  } catch (e) {
    if (results) results.textContent = (e && e.message) || "検索できませんでした";
    saiToast((e && e.message) || "検索できませんでした", "error");
  } finally {
    sessBusy = false;
  }
}

function sessRenderResults(container, items, truncated) {
  if (!items.length) {
    container.appendChild(sessEl("p", "lib-empty", "見つかりませんでした"));
    return;
  }
  container.appendChild(sessEl("p", "sess-count", `${items.length} 件${truncated ? "（先頭 200 件）" : ""}`));
  for (const group of sessGroup(items)) {
    const section = sessEl("section", "sess-run");
    const head = sessEl("div", "sess-run-head");
    const a = document.createElement("a");
    a.href = `#/run/${encodeURIComponent(sessName)}/${encodeURIComponent(group.blockId)}`;
    a.textContent = `run ${group.blockId}`;
    head.appendChild(a);
    head.appendChild(sessEl("span", "sess-run-count", `${group.items.length} 件`));
    section.appendChild(head);
    for (const item of group.items) {
      const row = sessEl("div", "sess-hit");
      row.appendChild(sessEl("span", "sess-hit-type", item.type || ""));
      if (item.turn != null) row.appendChild(sessEl("span", "sess-hit-turn", `turn ${item.turn}`));
      const snippet = sessEl("div", "sess-hit-text", item.snippet || "");
      section.append(row, snippet);
    }
    container.appendChild(section);
  }
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
    sessParseHash,
    sessBuildQuery,
    sessGroup,
  };
}
