"use strict";
// 検索セットの画面（#/papers/sets・#/papers/sets/<set_id>）（U21）。
//
// 検索で集めた論文を Web で見て選別する（keep / drop・未選別をまとめて drop）。
// 検索の Action の完了基準は「未選別が 0」（DEBT の set_screened）。
//
// 中の処理は `GET …/papers/sets`・`GET …/papers/sets/<id>`・`POST …/mark`・`POST …/drop-pending` だけ。
// DOM を使わない純関数（ssParseHash / ssCountsLabel / ssFilter / ssMarkBody）は末尾で node に渡す
// （tests/test_sciset.js）。一番外側の名前は ss で始める（tests/test_globals.js）。

// --- 定数 ---

// 絞り込み（既定は未選別）
const SS_FILTERS = [["pending", "未選別"], ["keep", "keep"], ["drop", "drop"], ["all", "すべて"]];

// --- 画面の状態 ---

let ssFilterValue = "pending";   // 選別の画面の絞り込み
let ssSetId = 0;                 // 開いているセットの番号（0 なら一覧）

// ============================================================================
// DOM を使わない純関数
// ============================================================================

/** 住所を読む。`#/papers/sets` → {}、`#/papers/sets/12` → {setId: 12}、それ以外 null。 */
function ssParseHash(hash) {
  const h = String(hash || "");
  if (h === "#/papers/sets") return {};
  const m = /^#\/papers\/sets\/([1-9][0-9]{0,8})$/.exec(h);
  return m ? { setId: Number(m[1]) } : null;
}

/** 件数の文字（`未選別 11 / keep 3 / drop 20`。imported があれば `/ 取り込み済み 1` を足す）。 */
function ssCountsLabel(counts) {
  const c = counts || {};
  const n = (k) => Number(c[k]) || 0;
  let s = `未選別 ${n("pending")} / keep ${n("keep")} / drop ${n("drop")}`;
  if (n("imported") > 0) s += ` / 取り込み済み ${n("imported")}`;
  return s;
}

/** 絞り込み。decision が all なら全部。 */
function ssFilter(items, decision) {
  const list = Array.isArray(items) ? items : [];
  return decision === "all" ? list.slice() : list.filter((it) => it && it.decision === decision);
}

/** mark の本文。通らなければ null（送らない）。 */
function ssMarkBody(positions, decision, reason) {
  if (!Array.isArray(positions) || positions.length < 1 || positions.length > 500) return null;
  if (decision !== "keep" && decision !== "drop") return null;
  const why = reason == null ? "" : String(reason).trim();
  if (why.length > 200) return null;
  const body = { positions, decision };
  if (why) body.reason = why;
  return body;
}

// ============================================================================
// 小さい DOM を作る手
// ============================================================================

function ssEl(tag, cls, text) {
  const e = document.createElement(tag);
  if (cls) e.className = cls;
  if (text != null) e.textContent = text;
  return e;
}

function ssBtn(label, onClick, cls) {
  const b = ssEl("button", `sai-btn ${cls || ""}`.trim(), label);
  b.type = "button";
  b.addEventListener("click", onClick);
  return b;
}

/** セットの API の住所。 */
function ssApi(path) {
  return `${papersApiBase(selectedPaperProject())}/sets${path || ""}`;
}

/** 空の枠（無ければ作る）。view-sci-set の中は自分で作る（index.html には空の section だけ）。 */
function ssRoot() {
  const view = document.getElementById("view-sci-set");
  if (!view) return null;
  let body = document.getElementById("ss-body");
  if (!body) {
    const head = ssEl("header", "appbar");
    const back = ssEl("button", "", "‹");
    back.id = "ss-back";
    back.setAttribute("aria-label", "戻る");
    back.addEventListener("click", () => { location.hash = ssSetId ? "#/papers/sets" : "#/papers"; });
    head.appendChild(back);
    const h1 = ssEl("h1", "", "検索セット");
    h1.id = "ss-title";
    head.appendChild(h1);
    view.appendChild(head);
    body = ssEl("main", "ss-body");
    body.id = "ss-body";
    view.appendChild(body);
  }
  return body;
}

function ssNote(text) {
  return ssEl("p", "ss-note", text);
}

// ============================================================================
// 画面
// ============================================================================

/** 画面を開く。setId があれば選別、無ければ一覧。 */
async function ssOpen(params) {
  if (typeof document === "undefined") return;
  ssSetId = Number(params && params.setId) || 0;
  ssFilterValue = "pending";
  if (typeof ensurePaperProjects === "function") await ensurePaperProjects();
  await ssReload();
}

/** 読み直して描き直す。 */
async function ssReload() {
  const body = ssRoot();
  if (!body) return;
  const title = document.getElementById("ss-title");
  if (title) title.textContent = ssSetId ? "検索セットの選別" : "検索セット";
  body.innerHTML = "";
  if (typeof selectedPaperProject !== "function" || !selectedPaperProject()) {
    body.appendChild(saiEmpty({ icon: "📁", title: "論文の置き場がありません", body: "論文の置き場があるプロジェクトがありません" }));
    return;
  }
  try {
    if (ssSetId) ssRenderSet(body, await paperApi(ssApi(`/${ssSetId}`)));
    else ssRenderList(body, await paperApi(ssApi("")));
  } catch (e) {
    body.appendChild(saiEmpty({ icon: "⚠️", title: "検索セットを読めませんでした", body: (e && e.message) || "もう一度読み込んでください。" }));
  }
}

/** 結んだ Node の釦（node は ID の文字か {id}）。 */
function ssNodeBtn(node) {
  const id = node && typeof node === "object" ? node.id : node;
  if (!id) return null;
  const b = ssEl("button", "ss-node", String(id));
  b.type = "button";
  b.addEventListener("click", (ev) => { ev.stopPropagation(); location.hash = `#/papers/tree/${id}`; });
  return b;
}

/** 一覧（未選別のあるセットを上に）。 */
function ssRenderList(body, data) {
  const sets = ((data && data.sets) || []).slice();
  if (!sets.length) {
    body.appendChild(saiEmpty({ icon: "🔎", title: "検索セットがありません", body: "検索を実行するとここに並びます。" }));
    return;
  }
  const pend = (s) => (Number(s.counts && s.counts.pending) > 0 ? 1 : 0);
  sets.sort((a, b) => pend(b) - pend(a)); // 同じ側の中は元の順（sort は安定）
  const list = ssEl("div", "ss-list");
  for (const s of sets) {
    const row = ssEl("div", "ss-set");
    row.tabIndex = 0;
    row.setAttribute("role", "link");
    const open = () => { location.hash = `#/papers/sets/${s.id}`; };
    row.addEventListener("click", open);
    row.addEventListener("keydown", (ev) => { if (ev.key === "Enter") open(); });
    row.appendChild(ssEl("div", "ss-set-name", s.name || `セット ${s.id}`));
    if (s.query) row.appendChild(ssEl("div", "ss-set-query", s.query));
    const meta = ssEl("div", "ss-set-meta");
    if (s.created_at) meta.appendChild(ssEl("span", "ss-set-date", String(s.created_at).slice(0, 10)));
    meta.appendChild(ssEl("span", pend(s) ? "ss-counts warn" : "ss-counts", ssCountsLabel(s.counts)));
    const nb = ssNodeBtn(s.node);
    if (nb) meta.appendChild(nb);
    row.appendChild(meta);
    list.appendChild(row);
  }
  body.appendChild(list);
}

/** 送って読み直す。失敗は文を出す。 */
async function ssSend(path, payload, okText) {
  try {
    await paperApi(ssApi(path), { method: "POST", body: payload });
    saiToast(okText, "ok");
  } catch (e) {
    saiToast((e && e.message) || "送れませんでした", "error");
    return;
  }
  await ssReload();
}

/** 1 つのセットの選別。 */
function ssRenderSet(body, data) {
  const set = (data && data.set) || {};
  const items = (data && data.items) || [];
  const box = ssEl("div", "ss-set-page");

  const top = ssEl("section", "ss-top");
  top.appendChild(ssEl("h2", "ss-set-name", set.name || `セット ${ssSetId}`));
  if (set.query) top.appendChild(ssEl("p", "ss-set-query", `検索語: ${set.query}`));
  if (set.purpose) top.appendChild(ssNote(`目的: ${set.purpose}`));
  const ex = Array.isArray(set.extract) ? set.extract.filter(Boolean) : [];
  if (ex.length) top.appendChild(ssNote(`抜き出すもの: ${ex.map((x) => (typeof x === "string" ? x : JSON.stringify(x))).join(" / ")}`));
  top.appendChild(ssEl("p", "ss-counts", ssCountsLabel(set.counts)));
  const tabs = ssEl("div", "ss-filters");
  for (const [value, label] of SS_FILTERS) {
    const b = ssBtn(label, () => { ssFilterValue = value; void ssReload(); }, value === ssFilterValue ? "primary" : "ghost");
    tabs.appendChild(b);
  }
  top.appendChild(tabs);
  box.appendChild(top);

  const shown = ssFilter(items, ssFilterValue);
  if (!shown.length) box.appendChild(ssNote("この絞り込みに合う項目はありません。"));
  for (const it of shown) box.appendChild(ssItemRow(it));

  const pending = items.filter((it) => it && it.decision === "pending").length;
  const bottom = ssEl("div", "ss-bottom");
  const all = ssBtn("未選別をまとめて drop", async () => {
    const ok = await saiConfirm({
      title: "まとめて drop",
      body: `未選別 ${pending} 件をすべて drop にします`,
      ok: "drop にする",
      danger: true,
    });
    if (ok) await ssSend(`/${ssSetId}/drop-pending`, {}, `未選別 ${pending} 件を drop にしました`);
  }, "danger");
  all.disabled = pending === 0;
  bottom.appendChild(all);
  box.appendChild(bottom);
  body.appendChild(box);
}

/** 項目 1 行。 */
function ssItemRow(it) {
  const row = ssEl("article", "ss-item");
  row.appendChild(ssEl("h3", "ss-item-title", it.title || "（題なし）"));
  const meta = [it.authors, it.year, it.venue].filter(Boolean).join(" ・ ");
  if (meta) row.appendChild(ssEl("p", "ss-item-meta", meta));
  if (it.abstract) {
    const ab = ssEl("p", "ss-item-abstract", it.abstract);
    ab.addEventListener("click", () => ab.classList.toggle("open")); // 初めは 3 行で畳む
    row.appendChild(ab);
  }
  const act = ssEl("div", "ss-item-actions");
  if (it.decision === "imported") {
    act.appendChild(ssEl("span", "ss-imported", "取り込み済み"));
  } else {
    for (const d of ["keep", "drop"]) {
      const b = ssBtn(d, () => {
        const body = ssMarkBody([it.position], d, "");
        if (body) void ssSend(`/${ssSetId}/mark`, body, `${d} にしました`);
      }, it.decision === d ? "primary" : "ghost");
      act.appendChild(b);
    }
  }
  row.appendChild(act);
  return row;
}

// ============================================================================
// 画面の登録（window.SaiRoutes）
// ============================================================================

if (typeof window !== "undefined" && typeof document !== "undefined") {
  (window.SaiRoutes = window.SaiRoutes || []).push({
    // 一覧（#/papers/sets）と 1 つのセット（#/papers/sets/12）
    match(hash) {
      return ssParseHash(hash);
    },
    view: "view-sci-set",
    tab: "papers",
    hideTabbar: false,
    interval: 0,
    async open(params) {
      await ssOpen(params);
    },
    async poll() {},
  });
}

// --- node からの読み込み（tests/test_sciset.js） ---
if (typeof module !== "undefined" && module.exports) {
  module.exports = { ssParseHash, ssCountsLabel, ssFilter, ssMarkBody, SS_FILTERS };
}
