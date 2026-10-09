// イベント表（U05）。sidebar の「イベント」タブ。行の選択は TS.openEvent で記録と時刻へ飛ぶ。

const tsEvtMaxRows = 2000;
const tsEvtCols = [
  { key: "record", title: "記録" },
  { key: "type", title: "種類" },
  { key: "time", title: "時刻（秒）" },
  { key: "vars", title: "変数" },
  { key: "category", title: "カテゴリ" },
  { key: "text", title: "テキスト" },
  { key: "source", title: "作成者" },
];

// 時刻（並べ替えと表示に使う）。ライン は t、エリア は t0
function tsEventTime(ev) {
  return ev.type === "area" ? ev.t0 : ev.t;
}

// 時刻の表示。ライン は "12.30"、エリア は "12.30–13.10"
function tsEventSpanText(ev) {
  if (ev.type === "area") return `${Number(ev.t0).toFixed(2)}–${Number(ev.t1).toFixed(2)}`;
  return Number(ev.t).toFixed(2);
}

// 変数の表示。3 つまで、残りは「ほか n」。空は記録全体
function tsEventVarsText(vars) {
  const v = vars || [];
  if (v.length === 0) return "（全体）";
  const head = v.slice(0, 3).join(", ");
  return v.length > 3 ? `${head} ほか ${v.length - 3}` : head;
}

// 作成者の札。source は "human" | "agent" | "script"（script:<name>@… も "script" に縮める）
function tsEvtWho(source) {
  return String(source).startsWith("script:") ? "script" : source;
}

// 絞り込み。f = {record, category, source, q}。空の値は「すべて」
function tsFilterEvents(events, f) {
  const q = (f.q || "").trim().toLowerCase();
  return events.filter((ev) => {
    if (f.record && ev.record !== f.record) return false;
    if (f.category && ev.category !== f.category) return false;
    if (f.source && tsEvtWho(ev.source) !== f.source) return false;
    if (q) {
      const hay = `${ev.text || ""}\n${ev.category || ""}`.toLowerCase();
      if (!hay.includes(q)) return false;
    }
    return true;
  });
}

// 並べ替え。dir は 1（昇順）か -1（降順）。同じ値のときは元の並びを保つ（どちら向きでも）
function tsSortEvents(events, key, dir) {
  const val = {
    record: (ev) => ev.record || "",
    type: (ev) => ev.type || "",
    time: (ev) => tsEventTime(ev),
    vars: (ev) => (ev.vars || []).join("|"),
    category: (ev) => ev.category || "",
    text: (ev) => ev.text || "",
    source: (ev) => ev.source || "",
  }[key] || tsEventTime;
  return events
    .map((ev, i) => ({ ev, i, v: val(ev) }))
    .sort((a, b) => {
      if (a.v < b.v) return -dir;
      if (a.v > b.v) return dir;
      return a.i - b.i;
    })
    .map((x) => x.ev);
}

// openEvent の表示範囲。中心 ± max(5, 長さ × 1.5)。記録の端 [0, dur] に収める（幅は保つ）
function tsFocusRange(ev, dur) {
  const isArea = ev.type === "area";
  const t0 = isArea ? ev.t0 : ev.t;
  const t1 = isArea ? ev.t1 : ev.t;
  const half = Math.max(5, (t1 - t0) * 1.5);
  const c = (t0 + t1) / 2;
  if (!(dur > 0)) return { t0: c - half, t1: c + half };
  if (2 * half >= dur) return { t0: 0, t1: dur };
  let lo = c - half;
  let hi = c + half;
  if (lo < 0) {
    hi -= lo;
    lo = 0;
  }
  if (hi > dur) {
    lo -= hi - dur;
    hi = dur;
  }
  return { t0: lo, t1: hi };
}

// 種類の札（ライン は ラ、エリア は エ）
function tsEventKindMark(type) {
  return type === "area" ? "エ" : "ラ";
}

const tsEvt = {
  scope: "record", // record（今の記録）| all（すべて）
  sort: { key: "time", dir: 1 },
  all: null, // scope が all のときの全記録（GET .../events）
  tbody: null,
  count: null,
  note: null,
  scrollNext: false,
  catSig: "",
};

function tsEvtStyle() {
  if (document.getElementById("ts-style-evtable.js")) return;
  const st = document.createElement("style");
  st.id = "ts-style-evtable.js";
  st.textContent = `
.ts-evt-bar { display: flex; flex-wrap: wrap; gap: 6px; align-items: center; margin-bottom: 6px; }
.ts-evt-bar input[type=search] { flex: 1 1 120px; min-width: 0; }
/* 幅 360px の欄に全部の列を収める（横に動かさない）。列幅を決め、長い文字は省略し title で全文を出す */
.ts-evt-table { width: 100%; border-collapse: collapse; font-size: 12px; table-layout: fixed; }
.ts-evt-table th, .ts-evt-table td { padding: 3px 2px; border-bottom: 1px solid #ddd; text-align: left; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
.ts-evt-table th { cursor: pointer; user-select: none; background: #f4f4f4; position: sticky; top: 0; white-space: normal; }
.ts-evt-table th[data-key=record] { width: 26px; }
.ts-evt-table th[data-key=type] { width: 22px; }
.ts-evt-table th[data-key=time] { width: 50px; }
.ts-evt-table th[data-key=vars] { width: 50px; }
.ts-evt-table th[data-key=category] { width: 50px; }
.ts-evt-table th[data-key=text] { width: 70px; }
.ts-evt-table th[data-key=source] { width: 40px; }
.ts-evt-table tr { cursor: pointer; }
.ts-evt-table tr.ts-sel { background: #fff3c4; }
.ts-evt-chip { display: inline-block; padding: 0 6px; border-radius: 3px; color: #fff; font-size: 12px; }
.ts-evt-del { border: 0; background: none; color: #b00; cursor: pointer; }
.ts-evt-note { color: #b00; margin: 4px 0; }
.ts-evt-wrap { overflow: auto; max-height: 60vh; }
`;
  document.head.appendChild(st);
}

function tsEvtDs() {
  return encodeURIComponent(TS.state.dataset || "");
}

async function tsEvtFetchAll() {
  try {
    const res = await TS.api(`/api/datasets/${tsEvtDs()}/events`);
    tsEvt.all = res.events || [];
  } catch (e) {
    TS.ui.toast(`イベントを読めません: ${e.message}`);
    tsEvt.all = tsEvt.all || [];
  }
}

// 表に出す元の行。今の記録は state.events、すべては API の全記録
function tsEvtRows() {
  if (tsEvt.scope === "all") return tsEvt.all || [];
  return TS.state.events || [];
}

function tsEvtFind(id) {
  const local = (TS.state.events || []).find((e) => e.id === id);
  if (local) return local;
  return (tsEvt.all || []).find((e) => e.id === id) || null;
}

// イベントの記録と時刻へ飛び、そのイベントを選ぶ
async function tsEvtOpen(id) {
  let ev = tsEvtFind(id);
  if (!ev) {
    await tsEvtFetchAll();
    ev = tsEvtFind(id);
  }
  if (!ev) {
    TS.ui.toast("イベントが見つかりません");
    return;
  }
  try {
    const dsEnc = tsEvtDs();
    const meta =
      ev.record === TS.state.record && TS.state.meta
        ? TS.state.meta
        : await TS.api(`/api/datasets/${dsEnc}/records/${encodeURIComponent(ev.record)}/meta`);
    const dur = meta.srate > 0 ? meta.n_samples / meta.srate : 0;
    const r = tsFocusRange(ev, dur);
    tsEvt.scrollNext = true;
    TS.setView({ record: ev.record, t0: r.t0, t1: r.t1, selectedEvents: [id] });
  } catch (e) {
    TS.ui.toast(`飛べません: ${e.message}`);
  }
}

function tsEvtRowEl(ev, selected) {
  const tr = document.createElement("tr");
  tr.dataset.id = ev.id;
  if (selected) tr.classList.add("ts-sel");
  // cls のある欄は省略されることがあるので、全文を title に入れる
  const cell = (text, cls) => {
    const td = document.createElement("td");
    td.textContent = text;
    if (cls) {
      td.className = cls;
      td.title = text;
    }
    return td;
  };
  tr.appendChild(cell(ev.record));
  tr.appendChild(cell(tsEventKindMark(ev.type)));
  tr.appendChild(cell(tsEventSpanText(ev)));
  tr.appendChild(cell(tsEventVarsText(ev.vars), "ts-vars"));

  const catTd = document.createElement("td");
  if (ev.category) {
    const chip = document.createElement("span");
    chip.className = "ts-evt-chip";
    chip.textContent = ev.category;
    const cat = tsEvtCategories().find((c) => c.name === ev.category);
    chip.style.background = (cat && cat.color) || "#777";
    catTd.appendChild(chip);
  }
  tr.appendChild(catTd);

  tr.appendChild(cell(ev.text || "", "ts-text"));
  const who = cell(tsEvtWho(ev.source), "ts-who");
  who.title = ev.source;
  tr.appendChild(who);

  const delTd = document.createElement("td");
  const del = document.createElement("button");
  del.type = "button";
  del.className = "ts-evt-del";
  del.textContent = "×";
  del.title = "このイベントを消す";
  del.addEventListener("click", (e) => {
    e.stopPropagation();
    tsEvtDelete(ev.id);
  });
  delTd.appendChild(del);
  tr.appendChild(delTd);

  tr.addEventListener("click", () => TS.openEvent(ev.id));
  tr.addEventListener("dblclick", () => TS.editEvent(ev.id, null));
  return tr;
}

function tsEvtCategories() {
  return (TS.state.schema && TS.state.schema.categories) || [];
}

async function tsEvtDelete(id) {
  if (!confirm(`イベント ${id} を消します。よいですか？`)) return;
  try {
    await TS.api(`/api/datasets/${tsEvtDs()}/events/${encodeURIComponent(id)}`, { method: "DELETE" });
  } catch (e) {
    TS.ui.toast(`消せません: ${e.message}`);
    return;
  }
  if (tsEvt.scope === "all") await tsEvtFetchAll();
  TS.reloadEvents();
  tsEvtDraw();
}

// 絞り込みの値を読む
function tsEvtFilterNow() {
  const v = (k) => tsEvt.bar.querySelector(`[data-k="${k}"]`).value;
  return {
    record: tsEvt.scope === "record" ? TS.state.record : null,
    category: v("cat"),
    source: v("who"),
    q: v("q"),
  };
}

function tsEvtSyncCats() {
  const sel = tsEvt.bar.querySelector('[data-k="cat"]');
  const cats = tsEvtCategories();
  const sig = cats.map((c) => c.name).join("\n");
  if (sig === tsEvt.catSig) return;
  tsEvt.catSig = sig;
  const keep = sel.value;
  sel.replaceChildren();
  const all = document.createElement("option");
  all.value = "";
  all.textContent = "すべて";
  sel.appendChild(all);
  for (const c of cats) {
    const o = document.createElement("option");
    o.value = c.name;
    o.textContent = c.name;
    sel.appendChild(o);
  }
  sel.value = keep;
}

function tsEvtDraw() {
  if (!tsEvt.tbody || !tsEvt.tbody.isConnected) return;
  tsEvtSyncCats();
  const rows = tsEvtRows();
  const hit = tsSortEvents(tsFilterEvents(rows, tsEvtFilterNow()), tsEvt.sort.key, tsEvt.sort.dir);
  const shown = hit.slice(0, tsEvtMaxRows);
  const sel = new Set(TS.state.selectedEvents || []);
  tsEvt.tbody.replaceChildren(...shown.map((ev) => tsEvtRowEl(ev, sel.has(ev.id))));
  tsEvt.count.textContent = `${hit.length} 件（全 ${rows.length} 件）`;
  tsEvt.note.textContent = hit.length > tsEvtMaxRows ? `${tsEvtMaxRows} 件を超えました。絞り込んでください` : "";
  tsEvtHeaders();
  if (tsEvt.scrollNext) {
    tsEvt.scrollNext = false;
    const first = tsEvt.tbody.querySelector("tr.ts-sel");
    if (first && first.scrollIntoView) first.scrollIntoView({ block: "nearest" });
  }
}

function tsEvtHeaders() {
  for (const th of tsEvt.heads) {
    const on = th.dataset.key === tsEvt.sort.key;
    th.textContent = th.dataset.title + (on ? (tsEvt.sort.dir > 0 ? " ▲" : " ▼") : "");
  }
}

function tsEvtSortBy(key) {
  if (tsEvt.sort.key === key) tsEvt.sort.dir = -tsEvt.sort.dir;
  else tsEvt.sort = { key, dir: 1 };
  tsEvtDraw();
}

function tsEvtRender(el) {
  tsEvtStyle();
  el.replaceChildren();

  const bar = document.createElement("div");
  bar.className = "ts-evt-bar";
  tsEvt.bar = bar;

  const sel = (k, opts) => {
    const s = document.createElement("select");
    s.dataset.k = k;
    for (const [value, text] of opts) {
      const o = document.createElement("option");
      o.value = value;
      o.textContent = text;
      s.appendChild(o);
    }
    s.addEventListener("change", () => tsEvtOnChange(k));
    return s;
  };
  const scope = sel("scope", [
    ["record", "今の記録"],
    ["all", "すべて"],
  ]);
  scope.value = tsEvt.scope;
  bar.append(
    scope,
    sel("cat", [["", "すべて"]]),
    sel("who", [
      ["", "すべて"],
      ["human", "human"],
      ["agent", "agent"],
      ["script", "script"],
    ]),
  );
  const q = document.createElement("input");
  q.type = "search";
  q.dataset.k = "q";
  q.placeholder = "テキスト・カテゴリ";
  q.addEventListener("input", () => tsEvtDraw());
  bar.appendChild(q);
  tsEvt.count = document.createElement("span");
  bar.appendChild(tsEvt.count);
  const tsv = document.createElement("button");
  tsv.type = "button";
  tsv.textContent = "TSV";
  tsv.addEventListener("click", () => {
    const tok = tsToken();
    const qs = tok ? `?token=${encodeURIComponent(tok)}` : "";
    window.open(tsUrl(`/api/datasets/${tsEvtDs()}/events.tsv${qs}`), "_blank");
  });
  bar.appendChild(tsv);

  const wrap = document.createElement("div");
  wrap.className = "ts-evt-wrap";
  const table = document.createElement("table");
  table.className = "ts-evt-table";
  const thead = document.createElement("thead");
  const headRow = document.createElement("tr");
  tsEvt.heads = tsEvtCols.map((c) => {
    const th = document.createElement("th");
    th.dataset.key = c.key;
    th.dataset.title = c.title;
    th.addEventListener("click", () => tsEvtSortBy(c.key));
    headRow.appendChild(th);
    return th;
  });
  headRow.appendChild(document.createElement("th"));
  thead.appendChild(headRow);
  tsEvt.tbody = document.createElement("tbody");
  table.append(thead, tsEvt.tbody);
  wrap.appendChild(table);
  tsEvt.note = document.createElement("p");
  tsEvt.note.className = "ts-evt-note";

  el.append(bar, wrap, tsEvt.note);
  if (tsEvt.scope === "all" && !tsEvt.all) tsEvtFetchAll().then(tsEvtDraw);
  tsEvtDraw();
}

function tsEvtOnChange(k) {
  if (k !== "scope") {
    tsEvtDraw();
    return;
  }
  tsEvt.scope = tsEvt.bar.querySelector('[data-k="scope"]').value;
  if (tsEvt.scope === "all") tsEvtFetchAll().then(tsEvtDraw);
  else tsEvtDraw();
}

function tsEvtBoot() {
  TS.ui.addSidebarTab({ id: "events", title: "イベント", render: tsEvtRender });
  TS.on("events", async () => {
    if (tsEvt.scope === "all") await tsEvtFetchAll();
    tsEvtDraw();
  });
  TS.on("schema", () => tsEvtDraw());
  TS.on("select", () => {
    tsEvt.scrollNext = true;
    tsEvtDraw();
  });
  TS.on("state", () => tsEvtDraw());
}

if (typeof window !== "undefined" && window.TS) {
  window.TS.openEvent = tsEvtOpen;
  window.TS.on("boot", tsEvtBoot);
}

if (typeof module !== "undefined") {
  module.exports = {
    tsEventTime,
    tsEventSpanText,
    tsEventVarsText,
    tsFilterEvents,
    tsSortEvents,
    tsFocusRange,
    tsEventKindMark,
    tsEvtWho,
    tsEvtMaxRows,
  };
}
