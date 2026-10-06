"use strict";
// Dashboard（#/project/<name>）の検索カード。task / message / waggle の文書の本文を探す。
//
// 検索語 + 種類（task / message / waggle）+ 日付の範囲 + block + run_id の範囲。
// データは GET /api/projects/{name}/search（中身は `sai-link phone search`）。
// カードは project.js の openDashboard が 1 度だけ置く（10 秒ごとの描き直しで
// 入力が消えないよう、.dash-cards の外に置く）。
// DOM を使わない関数（dsrQuery / dsrRowHash など）は末尾で node に渡して
// tests/test_dashsearch.js で確かめる。一番外側の名前は dsr / DSR_ で始める。

// --- 定数 ---

const DSR_CATEGORIES = [
  ["task", "task"],
  ["message", "message"],
  ["waggle", "waggle"],
];
// 1 度に出す件数（API の上限 100 より小さくして、多いときは「絞り込んでください」と案内する）
const DSR_LIMIT = 30;

// --- 状態 ---

let dsrName = "";
// 「block」の候補（Dashboard の by_block の名前）
let dsrBlocks = [];
let dsrSeq = 0;

// --- DOM を使わない関数 ---

/** 入力欄の値から API の検索文字列（?q=…）を作る。検索語が空なら null。 */
function dsrQuery(form) {
  const f = form || {};
  const q = String(f.q || "").trim();
  if (!q) return null;
  const p = new URLSearchParams();
  p.set("q", q);
  const cats = (Array.isArray(f.categories) ? f.categories : []).filter((c) =>
    DSR_CATEGORIES.some((x) => x[0] === c)
  );
  // 全部選んでいる・1 つも選んでいないときは種類で絞らない
  if (cats.length > 0 && cats.length < DSR_CATEGORIES.length) p.set("category", cats.join(","));
  for (const [key, value] of [
    ["since", f.since],
    ["until", f.until],
    ["block", f.block],
    ["from_id", f.fromId],
    ["to_id", f.toId],
  ]) {
    const v = String(value == null ? "" : value).trim();
    if (v) p.set(key, v);
  }
  p.set("limit", String(DSR_LIMIT));
  return p.toString();
}

/** 結果 1 件を押したときの行き先（task / message は List の表示、waggle は文書そのもの）。 */
function dsrRowHash(name, item) {
  const base = `#/project/${encodeURIComponent(name)}/list/${encodeURIComponent(String(item.run_id))}`;
  if (item.category === "waggle" && item.token) {
    return `${base}/waggle/${encodeURIComponent(item.token)}`;
  }
  return `${base}/${item.category === "message" ? "message" : "task"}`;
}

/** 結果 1 行の見出し（例: `#89 worker`）。 */
function dsrRowTitle(item) {
  return `#${item.run_id}${item.block ? ` ${item.block}` : ""}`;
}

// --- DOM ---

function dsrField(label, control) {
  const wrap = document.createElement("label");
  wrap.className = "dsr-field";
  wrap.appendChild(el("span", "dsr-label", label));
  wrap.appendChild(control);
  return wrap;
}

function dsrInput(type, name, attrs) {
  const i = document.createElement("input");
  i.type = type;
  i.name = name;
  i.className = "dsr-input";
  for (const [k, v] of Object.entries(attrs || {})) i.setAttribute(k, v);
  return i;
}

/** Dashboard の body に検索カードを置く（開くたびに作り直す）。 */
function dsrMount(box, name) {
  dsrName = name;
  dsrBlocks = [];
  dsrSeq += 1;
  const card = document.createElement("section");
  card.className = "dash-card dsr-card";
  card.appendChild(el("h3", "dash-card-title", "文書を検索"));

  const form = document.createElement("form");
  form.className = "dsr-form";
  form.setAttribute("role", "search");
  const q = dsrInput("search", "q", { placeholder: "検索語（空白で AND）", "aria-label": "検索語", maxlength: "200" });
  q.className = "dsr-input dsr-q";
  const go = el("button", "paper-btn dsr-go", "Search");
  go.type = "submit";
  const top = el("div", "dsr-top");
  top.append(q, go);
  form.appendChild(top);

  const det = document.createElement("details");
  det.className = "dsr-filters";
  det.appendChild(el("summary", null, "絞り込み"));
  const cats = el("div", "dsr-cats");
  for (const [key, label] of DSR_CATEGORIES) {
    const l = document.createElement("label");
    l.className = "dsr-cat";
    const c = dsrInput("checkbox", "category", { value: key });
    c.className = "";
    c.checked = true;
    l.append(c, document.createTextNode(label));
    cats.appendChild(l);
  }
  det.appendChild(dsrField("種類", cats));
  const since = dsrInput("date", "since", {});
  const until = dsrInput("date", "until", {});
  const dates = el("div", "dsr-pair");
  dates.append(since, el("span", "dsr-sep", "〜"), until);
  det.appendChild(dsrField("Date", dates));
  const block = dsrInput("text", "block", { list: "dsr-blocks", placeholder: "block 名（完全一致）", autocomplete: "off" });
  const dl = document.createElement("datalist");
  dl.id = "dsr-blocks";
  det.append(dsrField("block", block), dl);
  const fromId = dsrInput("number", "from_id", { min: "1", step: "1", inputmode: "numeric", placeholder: "から" });
  const toId = dsrInput("number", "to_id", { min: "1", step: "1", inputmode: "numeric", placeholder: "まで" });
  const ids = el("div", "dsr-pair");
  ids.append(fromId, el("span", "dsr-sep", "〜"), toId);
  det.appendChild(dsrField("run_id", ids));
  form.appendChild(det);

  const out = el("div", "dsr-out");
  out.setAttribute("aria-live", "polite");
  card.append(form, out);
  box.appendChild(card);

  form.addEventListener("submit", (e) => {
    e.preventDefault();
    void dsrRun(
      {
        q: q.value,
        categories: [...form.querySelectorAll("input[name=category]:checked")].map((c) => c.value),
        since: since.value,
        until: until.value,
        block: block.value,
        fromId: fromId.value,
        toId: toId.value,
      },
      out
    );
  });
}

/** block の候補を入れる（Dashboard を読めたときに project.js から呼ぶ）。 */
function dsrSetBlocks(names) {
  dsrBlocks = (Array.isArray(names) ? names : []).filter((n) => typeof n === "string" && n);
  const dl = document.getElementById("dsr-blocks");
  if (!dl) return;
  dl.innerHTML = "";
  for (const n of dsrBlocks) {
    const o = document.createElement("option");
    o.value = n;
    dl.appendChild(o);
  }
}

async function dsrRun(form, out) {
  const qs = dsrQuery(form);
  out.innerHTML = "";
  if (qs === null) {
    out.appendChild(el("p", "dash-empty", "検索語を入れてください"));
    return;
  }
  const seq = ++dsrSeq;
  const name = dsrName;
  out.appendChild(el("p", "dash-empty", "検索中…"));
  let body;
  try {
    const res = await apiFetch(`/api/projects/${encodeURIComponent(name)}/search?${qs}`);
    body = await res.json().catch(() => ({}));
    if (!res.ok) throw new Error(body.detail || "検索できませんでした");
  } catch (e) {
    if (seq !== dsrSeq) return;
    out.innerHTML = "";
    out.appendChild(el("p", "dash-empty dsr-error", (e && e.message) || "検索できませんでした"));
    return;
  }
  if (seq !== dsrSeq) return; // その間に新しい検索を始めた・画面を開き直した
  out.innerHTML = "";
  const items = Array.isArray(body.items) ? body.items : [];
  if (items.length === 0) {
    out.appendChild(el("p", "dash-empty", "見つかりませんでした"));
    return;
  }
  for (const item of items) out.appendChild(dsrRow(name, item));
  if (body.truncated) {
    out.appendChild(el("p", "dash-empty", `${items.length} 件で打ち切りました。日付・block・run_id で絞り込んでください`));
  }
}

function dsrRow(name, item) {
  const btn = document.createElement("button");
  btn.type = "button";
  btn.className = "dsr-row press";
  const head = el("span", "dsr-row-head");
  head.append(
    el("span", "dsr-row-id", dsrRowTitle(item)),
    el("span", `dsr-tag dsr-tag-${item.category}`, item.category),
    el("span", "dsr-row-file", item.file || ""),
    el("span", "dsr-row-date", item.date || "")
  );
  btn.append(head, el("span", "dsr-row-snippet", item.snippet || ""));
  btn.addEventListener("click", () => {
    location.hash = dsrRowHash(name, item);
  });
  return btn;
}

// --- node からの読み込み（tests/test_dashsearch.js。DOM を使わない部分だけ） ---
if (typeof module !== "undefined" && module.exports) {
  module.exports = { DSR_CATEGORIES, DSR_LIMIT, dsrQuery, dsrRowHash, dsrRowTitle };
}
