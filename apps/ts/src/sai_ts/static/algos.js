"use strict";
// アルゴリズムの画面（U08）。sidebar の「アルゴリズム」タブ。仕様（G2-C1）からパラメータの欄を作り、
// 対象（ch・イベント）と記録を選んで実行し（G2-C4）、実行の記録（G2-C3）を見る。
// 純関数（tsAlgoDefaults・tsAlgoRead・tsAlgoTargetIds・tsRunsFinished・tsAlgoFilter）は末尾で node のテストに渡す。
// 欄は evform.js の tsMetricInput・tsMetricControl・tsField を使う（型と検査の決まりを schema と同じにするため）。

const tsAlgoKinds = [
  ["", "全部"],
  ["channel", "ch を足す"],
  ["event", "イベント検出"],
  ["measure", "指標"],
];
const tsAlgoIcons = { running: "⟳", ok: "✓", error: "✗", timeout: "⏱", lost: "?" };
const tsAlgoMaxRows = 200;
const tsAlgoPollMs = 2000;
const tsAlgoStrMax = 2000;
const tsAlgoTargetModes = [
  ["selected", "選んだイベント"],
  ["all", "今の記録の全部"],
  ["category", "カテゴリ"],
  ["range", "今の範囲"],
];

// 既定の値（default のある名前だけ）
function tsAlgoDefaults(spec) {
  const out = {};
  for (const p of spec.params || []) {
    if (p.default !== undefined) out[p.name] = p.default;
  }
  return out;
}

// 入力欄の生の値（文字・真偽）を検査して params にする。空の数値・文字は送らない（既定で埋まる）。
// boolean は常に送る。G2-C5 と同じ決まり
function tsAlgoRead(spec, raw) {
  const params = {};
  for (const p of spec.params || []) {
    const v = raw ? raw[p.name] : undefined;
    const fail = (msg) => ({ ok: false, error: `${p.name}: ${msg}` });
    if (p.type === "boolean") {
      params[p.name] = v === true;
      continue;
    }
    const s = v === null || v === undefined ? "" : String(v).trim();
    if (s === "") continue;
    if (p.type === "number" || p.type === "integer") {
      const n = Number(s);
      if (!Number.isFinite(n)) return fail("数値を入れてください");
      if (p.type === "integer" && !Number.isInteger(n)) return fail("整数を入れてください");
      if (p.min !== undefined && p.min !== null && n < p.min) return fail(`${p.min} 以上にしてください`);
      if (p.max !== undefined && p.max !== null && n > p.max) return fail(`${p.max} 以下にしてください`);
      params[p.name] = n;
    } else if (p.type === "enum") {
      if (!(p.values || []).includes(s)) return fail("選択肢にない値です");
      params[p.name] = s;
    } else {
      if (s.length > tsAlgoStrMax) return fail(`${tsAlgoStrMax} 字までです`);
      params[p.name] = s;
    }
  }
  return { ok: true, params };
}

// イベントの対象の ID。selected は選んだもの、all は空（記録の全部の約束）、
// category はその名前の今の記録のイベント、range は表示中の時間と重なるもの
function tsAlgoTargetIds(mode, st, category) {
  const evs = st.events || [];
  if (mode === "selected") return (st.selectedEvents || []).slice();
  if (mode === "category") return evs.filter((e) => e.category === category).map((e) => e.id);
  if (mode === "range") {
    return evs
      .filter((e) => {
        const t0 = e.type === "area" ? e.t0 : e.t;
        const t1 = e.type === "area" ? e.t1 : e.t;
        return t1 >= st.t0 && t0 <= st.t1;
      })
      .map((e) => e.id);
  }
  return [];
}

// prev で running だった実行のうち、next で ok になったもの（next の行を返す）
function tsRunsFinished(prev, next) {
  const was = new Map((prev || []).map((r) => [r.id, r.status]));
  return (next || []).filter((r) => r.status === "ok" && was.get(r.id) === "running");
}

// 種類の絞り込み。kind が "" なら全部
function tsAlgoFilter(algos, kind) {
  return kind ? algos.filter((a) => a.kind === kind) : algos.slice();
}

// ---- ここから画面（ブラウザだけで動く） ----

// 画面の状態。list は GET /api/algos、runs は GET /api/datasets/{ds}/runs の結果
const tsAlgo = {
  list: [],
  name: null, // 選んでいるアルゴリズムの名前
  spec: null, // 選んでいる一覧の 1 件
  specKey: "", // 仕様の変わり目を見るための写し
  kind: "", // 種類の絞り込み
  vars: null, // 選ぶ前に ch のタブから来た ch の初期値（選んだとき使って消す）
  runs: [],
  ds: null, // 実行の記録を読んだデータセット
  openRuns: new Set(), // 開いている実行の ID
  ctl: {}, // パラメータの入力部品（名前 → 要素）
  tgt: {}, // 対象の部品（vars・mode・cat・rec・err・run）
  kindsEl: null,
  listEl: null,
  detailEl: null,
  runsEl: null,
  timer: null,
  listSeq: 0,
  runSeq: 0,
};

function tsAlgoStyle() {
  if (document.getElementById("ts-style-algos.js")) return;
  const st = document.createElement("style");
  st.id = "ts-style-algos.js";
  st.textContent = `
.ts-algo-kinds { display: flex; gap: 4px; flex-wrap: wrap; margin-bottom: 6px; }
.ts-algo-kinds button, .ts-algo-btns button { padding: 3px 8px; border: 1px solid var(--ts-border); background: var(--ts-panel); color: var(--ts-text); border-radius: 6px; cursor: pointer; }
.ts-algo-kinds button[aria-pressed="true"] { border-color: var(--ts-accent); color: var(--ts-accent); }
.ts-algo-list { border: 1px solid var(--ts-border); border-radius: 6px; max-height: 180px; overflow: auto; margin-bottom: 8px; }
.ts-algo-item { display: flex; gap: 8px; align-items: baseline; width: 100%; padding: 4px 8px; border: 0; background: transparent; color: var(--ts-text); text-align: left; cursor: pointer; flex-wrap: wrap; }
.ts-algo-item[aria-current="true"] { background: var(--ts-hover); color: var(--ts-accent); }
.ts-algo-item:disabled { cursor: default; }
.ts-algo-muted { color: var(--ts-muted); font-size: 12px; }
.ts-algo-warn { color: var(--ts-danger); font-size: 12px; }
.ts-algo-desc { color: var(--ts-muted); margin: 4px 0 8px; }
.ts-algo-btns { display: flex; gap: 4px; flex-wrap: wrap; margin: 4px 0; }
.ts-algo-head { margin: 12px 0 4px; font-weight: 600; }
.ts-algo-run { display: flex; gap: 8px; align-items: baseline; flex-wrap: wrap; width: 100%; padding: 4px 0; border: 0; border-bottom: 1px solid var(--ts-border); background: transparent; color: var(--ts-text); text-align: left; cursor: pointer; }
.ts-algo-st-ok { color: var(--ts-accent); }
.ts-algo-st-error, .ts-algo-st-timeout, .ts-algo-st-lost { color: var(--ts-danger); }
.ts-algo-msg { flex: 1 1 100%; color: var(--ts-muted); font-size: 12px; overflow-wrap: anywhere; }
.ts-algo-body { padding: 6px 0 10px; font-size: 12px; }
.ts-algo-pre { white-space: pre-wrap; overflow-wrap: anywhere; margin: 2px 0 6px; color: var(--ts-text); }
.ts-algo-err { color: var(--ts-danger); }
.ts-algo-tablewrap { overflow: auto; max-width: 100%; margin-top: 4px; }
.ts-algo-table { border-collapse: collapse; }
.ts-algo-table th, .ts-algo-table td { border: 1px solid var(--ts-border); padding: 2px 6px; white-space: nowrap; text-align: left; }
.ts-algo-table tr.ts-algo-link { cursor: pointer; }
.ts-algo-table tr.ts-algo-link:hover td { background: var(--ts-hover); }
`;
  document.head.appendChild(st);
}

// 仕様が変わったかを見る写し（一覧の 1 件の中身）
function tsAlgoSpecKey(a) {
  return JSON.stringify([a.title, a.kind, a.target, a.desc, a.params, a.error, a.sha256]);
}

function tsAlgoSavedKey(name) {
  return "ts_algo:" + name;
}

// 前に入れた値（localStorage）。読めなければ null
function tsAlgoSaved(name) {
  try {
    return JSON.parse(localStorage.getItem(tsAlgoSavedKey(name)));
  } catch {
    return null;
  }
}

// 入れ直すときの初期値。同じ名前の params だけ残し、無いものは既定
function tsAlgoFormValues(spec, prior) {
  const base = tsAlgoDefaults(spec);
  const out = {};
  for (const p of spec.params || []) {
    if (prior && prior[p.name] !== undefined) out[p.name] = prior[p.name];
    else if (base[p.name] !== undefined) out[p.name] = base[p.name];
  }
  return out;
}

// 画面の入力欄を生の値に（検査は tsAlgoRead）
function tsAlgoRawParams() {
  const out = {};
  const spec = tsAlgo.spec;
  if (!spec) return out;
  for (const p of spec.params || []) {
    const c = tsAlgo.ctl[p.name];
    if (c) out[p.name] = p.type === "boolean" ? c.checked : c.value;
  }
  return out;
}

// 入っている ch の名前（欄が無いときは null）
function tsAlgoRawVars() {
  const v = tsAlgo.tgt.vars;
  return v ? v.filter((x) => x.el.checked).map((x) => x.name) : null;
}

// 検査が通った値を覚える（選び直したときに残す）
function tsAlgoSaveParams() {
  const spec = tsAlgo.spec;
  if (!spec) return;
  const r = tsAlgoRead(spec, tsAlgoRawParams());
  if (!r.ok) return;
  try {
    localStorage.setItem(tsAlgoSavedKey(spec.name), JSON.stringify(r.params));
  } catch {
    // 保存できなくても画面は動く
  }
}

function tsAlgoRender(el) {
  tsAlgo.kindsEl = tsEl("div", "ts-algo-kinds");
  tsAlgo.listEl = tsEl("div", "ts-algo-list");
  tsAlgo.detailEl = tsEl("div", "ts-algo-detail");
  tsAlgo.runsEl = tsEl("div", "ts-algo-runs");
  el.append(tsAlgo.kindsEl, tsAlgo.listEl, tsAlgo.detailEl, tsEl("div", "ts-algo-head", "実行の記録"), tsAlgo.runsEl);
  tsAlgoDrawFilter();
  tsAlgoDrawList();
  tsAlgoBuildDetail(tsAlgo.spec ? tsAlgoSaved(tsAlgo.spec.name) : null, null);
  tsAlgoDrawRuns();
  tsAlgoLoadList();
  tsAlgoLoadRuns();
}

function tsAlgoDrawFilter() {
  const box = tsAlgo.kindsEl;
  if (!box) return;
  box.replaceChildren();
  for (const [v, label] of tsAlgoKinds) {
    const b = tsEl("button", "", label);
    b.type = "button";
    b.setAttribute("aria-pressed", String(tsAlgo.kind === v));
    b.onclick = () => {
      tsAlgo.kind = v;
      tsAlgoDrawFilter();
      tsAlgoDrawList();
    };
    box.appendChild(b);
  }
}

function tsAlgoDrawList() {
  const box = tsAlgo.listEl;
  if (!box) return;
  box.replaceChildren();
  const items = tsAlgoFilter(tsAlgo.list, tsAlgo.kind);
  if (items.length === 0) {
    box.appendChild(tsEl("div", "ts-algo-muted", "アルゴリズムがありません"));
    return;
  }
  for (const a of items) {
    const b = tsEl("button", "ts-algo-item");
    b.type = "button";
    const sel = a.name === tsAlgo.name;
    b.setAttribute("aria-current", String(sel));
    b.disabled = a.error !== "";
    b.append(
      tsEl("span", "", sel ? "●" : " "),
      tsEl("span", "", a.title || a.name),
      tsEl("span", "ts-algo-muted", a.name),
    );
    if (a.error) b.appendChild(tsEl("span", "ts-algo-warn", "⚠ " + a.error));
    b.appendChild(tsEl("span", "ts-algo-muted", a.origin === "builtin" ? "組込" : "プロジェクト"));
    b.onclick = () => tsAlgoPick(a.name);
    box.appendChild(b);
  }
}

// 一覧の 1 件を選ぶ。入れた値は覚えてあるものを使う
function tsAlgoPick(name) {
  const a = tsAlgo.list.find((x) => x.name === name);
  if (!a) return;
  tsAlgo.name = name;
  tsAlgo.spec = a;
  tsAlgo.specKey = tsAlgoSpecKey(a);
  const vars = tsAlgo.vars;
  tsAlgo.vars = null;
  tsAlgoBuildDetail(tsAlgoSaved(name), vars);
  tsAlgoDrawList();
}

// 選んだアルゴリズムの欄（パラメータ・対象・記録・実行）を作り直す
function tsAlgoBuildDetail(prior, vars) {
  const box = tsAlgo.detailEl;
  if (!box) return;
  box.replaceChildren();
  tsAlgo.ctl = {};
  tsAlgo.tgt = {};
  const spec = tsAlgo.spec;
  if (!spec) {
    box.appendChild(tsEl("div", "ts-algo-desc", "アルゴリズムを選んでください"));
    return;
  }
  if (spec.error) {
    box.appendChild(tsEl("div", "ts-err", "⚠ " + spec.error));
    return;
  }
  const st = window.TS.state;
  box.appendChild(tsEl("div", "ts-form-title", `${spec.title || spec.name}（${spec.name} @${spec.sha256.slice(0, 8)}）`));
  if (spec.desc) box.appendChild(tsEl("div", "ts-algo-desc", spec.desc));

  // パラメータ（schema のメトリクスと同じ部品）
  const vals = tsAlgoFormValues(spec, prior);
  for (const p of spec.params || []) {
    const c = tsMetricControl(tsMetricInput(p), vals[p.name]);
    tsAlgo.ctl[p.name] = c;
    box.appendChild(tsField(p.name + (p.unit ? ` (${p.unit})` : ""), c));
    if (p.desc) box.appendChild(tsEl("div", "ts-desc", p.desc));
  }

  // 対象
  if (spec.target === "vars") {
    const want = vars || st.vars || [];
    const chk = tsEl("div", "ts-checks");
    tsAlgo.tgt.vars = (st.channels || []).map((c) => {
      const lab = tsEl("label", "ts-chk");
      const cb = tsEl("input");
      cb.type = "checkbox";
      cb.value = c.name;
      cb.checked = want.includes(c.name);
      lab.append(cb, " " + (c.label || c.name));
      chk.appendChild(lab);
      return { name: c.name, el: cb };
    });
    const setAll = (on) => tsAlgo.tgt.vars.forEach((x) => (x.el.checked = on));
    const pick = (names) => tsAlgo.tgt.vars.forEach((x) => (x.el.checked = names.includes(x.name)));
    const btns = tsEl("div", "ts-algo-btns");
    btns.append(
      tsAlgoBtn("表示中", () => pick(st.vars || [])),
      tsAlgoBtn("選択の ch", () => pick((st.selection && st.selection.vars) || [])),
      tsAlgoBtn("なし", () => setAll(false)),
    );
    box.append(tsField("対象の ch", chk), btns);
  } else if (spec.target === "events") {
    const mode = tsEl("select");
    tsAlgoTargetModes.forEach(([v, label]) => {
      const o = tsEl("option", "", v === "selected" ? `選んだイベント（${(st.selectedEvents || []).length} 件）` : label);
      o.value = v;
      mode.appendChild(o);
    });
    const cat = tsEl("select");
    (st.schema.categories || []).forEach((c) => {
      const o = tsEl("option", "", c.name);
      o.value = c.name;
      cat.appendChild(o);
    });
    const catField = tsField("カテゴリ", cat);
    const note = tsEl("div", "ts-desc");
    const sync = () => {
      catField.hidden = mode.value !== "category";
      const n = tsAlgoTargetIds(mode.value, st, cat.value).length;
      note.textContent = mode.value === "all" ? "今の記録の全部" : `対象 ${n} 件`;
    };
    mode.onchange = sync;
    cat.onchange = sync;
    sync();
    tsAlgo.tgt.mode = mode;
    tsAlgo.tgt.cat = cat;
    box.append(tsField("対象のイベント", mode), catField, note);
  }

  // 記録（今の記録か、全部）
  const now = tsEl("input");
  now.type = "radio";
  now.name = "ts-algo-rec";
  now.value = "now";
  now.disabled = !st.record;
  now.checked = !!st.record;
  const all = tsEl("input");
  all.type = "radio";
  all.name = "ts-algo-rec";
  all.value = "all";
  all.checked = !st.record;
  const n = tsRecordsOf(st.datasets, st.dataset).length;
  const recBox = tsEl("div", "ts-checks");
  recBox.append(tsAlgoLabel(now, `今の記録（${st.record || "なし"}）`), tsAlgoLabel(all, `全部（${n}）`));
  tsAlgo.tgt.rec = { now };
  box.appendChild(tsField("記録", recBox));

  const err = tsEl("div", "ts-err");
  err.setAttribute("role", "alert");
  err.hidden = true;
  tsAlgo.tgt.err = err;
  box.appendChild(err);

  const bar = tsEl("div", "ts-form-bar");
  const reset = tsEl("button", "", "既定に戻す");
  reset.type = "button";
  reset.onclick = () => {
    try {
      localStorage.removeItem(tsAlgoSavedKey(spec.name));
    } catch {
      // 消せなくても画面は既定に戻す
    }
    tsAlgoBuildDetail(null, tsAlgoRawVars());
  };
  const run = tsEl("button", "ts-primary", "実行");
  run.type = "button";
  run.onclick = tsAlgoRun;
  tsAlgo.tgt.run = run;
  bar.append(reset, run);
  box.appendChild(bar);

  // 入れた値は覚える（検査が通ったものだけ）
  box.addEventListener("input", tsAlgoSaveParams);
  box.addEventListener("change", tsAlgoSaveParams);
}

function tsAlgoBtn(label, onClick) {
  const b = tsEl("button", "", label);
  b.type = "button";
  b.onclick = onClick;
  return b;
}

function tsAlgoLabel(input, text) {
  const lab = tsEl("label", "ts-chk");
  lab.append(input, " " + text);
  return lab;
}

function tsAlgoShowErr(msg) {
  const err = tsAlgo.tgt.err;
  if (!err) return;
  err.textContent = msg;
  err.hidden = false;
}

// 実行。画面で検査して送る。400 は欄の下に理由を出す
async function tsAlgoRun() {
  const st = window.TS.state;
  const spec = tsAlgo.spec;
  if (!spec || spec.error || !st.dataset) return;
  tsAlgo.tgt.err.hidden = true;
  const r = tsAlgoRead(spec, tsAlgoRawParams());
  if (!r.ok) return tsAlgoShowErr(r.error);
  const target = { vars: [], events: [] };
  if (spec.target === "vars") {
    target.vars = tsAlgoRawVars() || [];
    if (target.vars.length === 0) return tsAlgoShowErr("ch を 1 つ以上選んでください");
  } else if (spec.target === "events") {
    const mode = tsAlgo.tgt.mode.value;
    target.events = tsAlgoTargetIds(mode, st, tsAlgo.tgt.cat.value);
    if (mode !== "all" && target.events.length === 0) return tsAlgoShowErr("対象のイベントがありません");
  }
  // 今の記録を選んでいれば [記録]、全部なら []（G2-C4）
  let records = [];
  if (tsAlgo.tgt.rec.now.checked) {
    if (!st.record) return tsAlgoShowErr("記録を選んでください");
    records = [st.record];
  }
  tsAlgo.tgt.run.disabled = true;
  try {
    await window.TS.api(`/api/datasets/${tsEnc(st.dataset)}/algos/${tsEnc(spec.name)}/runs`, {
      method: "POST",
      body: { records, params: r.params, target },
    });
    tsAlgoLoadRuns();
  } catch (e) {
    tsAlgoShowErr(e.message);
  } finally {
    tsAlgo.tgt.run.disabled = false;
  }
}

async function tsAlgoLoadList() {
  const seq = ++tsAlgo.listSeq;
  const r = await window.TS.api("/api/algos").catch((e) => {
    window.TS.ui.toast(`アルゴリズムを読めません: ${e.message}`);
    return null;
  });
  if (!r || seq !== tsAlgo.listSeq) return;
  tsAlgo.list = Array.isArray(r.algos) ? r.algos : [];
  tsAlgoDrawList();
  // 選んでいる仕様が変わったら欄を作り直す（入れた値は同じ名前のものだけ残す）
  if (!tsAlgo.name) return;
  const cur = tsAlgo.list.find((a) => a.name === tsAlgo.name);
  if (!cur) {
    tsAlgo.name = null;
    tsAlgo.spec = null;
    tsAlgoBuildDetail(null, null);
    return;
  }
  const key = tsAlgoSpecKey(cur);
  if (key === tsAlgo.specKey) return;
  const prior = tsAlgoRawParams();
  const vars = tsAlgoRawVars();
  tsAlgo.spec = cur;
  tsAlgo.specKey = key;
  tsAlgoBuildDetail(prior, vars);
  tsAlgoDrawList();
}

// 実行の記録を読む。終わったものは結果を画面に反映する
async function tsAlgoLoadRuns() {
  const st = window.TS.state;
  const ds = st.dataset;
  const seq = ++tsAlgo.runSeq;
  if (!ds) {
    tsAlgo.runs = [];
    tsAlgoDrawRuns();
    return;
  }
  const r = await window.TS.api(`/api/datasets/${tsEnc(ds)}/runs?limit=20`).catch((e) => {
    window.TS.ui.toast(`実行の記録を読めません: ${e.message}`);
    return null;
  });
  if (!r || seq !== tsAlgo.runSeq || st.dataset !== ds) return;
  const prev = tsAlgo.runs;
  const next = Array.isArray(r.runs) ? r.runs : [];
  tsAlgo.runs = next;
  tsAlgoDrawRuns();
  tsAlgoPoll();
  await tsAlgoReflect(tsRunsFinished(prev, next));
}

// 動いているものがあるあいだは、2 秒ごとに読む
function tsAlgoPoll() {
  const busy = tsAlgo.runs.some((r) => r.status === "running");
  if (busy && !tsAlgo.timer) {
    tsAlgo.timer = setInterval(tsAlgoLoadRuns, tsAlgoPollMs);
  } else if (!busy && tsAlgo.timer) {
    clearInterval(tsAlgo.timer);
    tsAlgo.timer = null;
  }
}

// 終わった実行が ch を足したなら、記録を読み直して表示に加える（今の記録に関わるものだけ）
async function tsAlgoReflect(done) {
  const st = window.TS.state;
  const add = [];
  for (const r of done) {
    if (!r.result || !Array.isArray(r.result.vars)) continue;
    if (r.records && r.records.length > 0 && !r.records.includes(st.record)) continue;
    add.push(...r.result.vars);
  }
  if (add.length === 0) return;
  await window.TS.reloadMeta();
  window.TS.setView({ vars: tsOrderVisible(st.channels, [...(st.vars || []), ...add]) });
}

function tsAlgoDrawRuns() {
  const box = tsAlgo.runsEl;
  if (!box) return;
  box.replaceChildren();
  if (tsAlgo.runs.length === 0) {
    box.appendChild(tsEl("div", "ts-algo-muted", "実行の記録がありません"));
    return;
  }
  for (const r of tsAlgo.runs) box.appendChild(tsAlgoRunEl(r));
}

function tsAlgoTime(iso) {
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return "";
  const p = (n) => String(n).padStart(2, "0");
  return `${p(d.getHours())}:${p(d.getMinutes())}`;
}

// 1 件の実行。▸ で開くと、パラメータ・対象・表・エラーの全文を出す
function tsAlgoRunEl(r) {
  const open = tsAlgo.openRuns.has(r.id);
  const wrap = tsEl("div", "ts-algo-runwrap");
  const head = tsEl("button", "ts-algo-run");
  head.type = "button";
  head.setAttribute("aria-expanded", String(open));
  const msg =
    r.status === "ok"
      ? (r.result && r.result.message) || ""
      : r.error || { running: "実行中", timeout: "時間切れ", lost: "終わりの記録がありません" }[r.status] || "";
  const elapsed = r.elapsed_s !== null && r.elapsed_s !== undefined ? `${Number(r.elapsed_s).toFixed(1)} s` : "";
  const recs = (r.records || []).join(", ") || "全部";
  head.append(
    tsEl("span", `ts-algo-st-${r.status}`, tsAlgoIcons[r.status] || "?"),
    tsEl("span", "", r.algo),
    tsEl("span", "ts-algo-muted", recs),
    tsEl("span", "ts-algo-muted", elapsed),
    tsEl("span", "ts-algo-muted", tsAlgoTime(r.started_at)),
    tsEl("span", "ts-algo-msg", msg),
    tsEl("span", "ts-algo-muted", open ? "▾" : "▸"),
  );
  head.onclick = () => {
    if (open) tsAlgo.openRuns.delete(r.id);
    else tsAlgo.openRuns.add(r.id);
    tsAlgoDrawRuns();
  };
  wrap.appendChild(head);
  if (open) wrap.appendChild(tsAlgoRunBody(r));
  return wrap;
}

function tsAlgoRunBody(r) {
  const body = tsEl("div", "ts-algo-body");
  const t = r.target || {};
  if ((t.vars || []).length) body.appendChild(tsEl("div", "ts-algo-muted", `対象 ch: ${t.vars.join(", ")}`));
  if ((t.events || []).length) body.appendChild(tsEl("div", "ts-algo-muted", `対象 イベント: ${t.events.length} 件`));
  body.appendChild(tsEl("pre", "ts-algo-pre", JSON.stringify(r.params || {})));
  if (r.status !== "ok" && r.error) body.appendChild(tsEl("pre", "ts-algo-pre ts-algo-err", r.error));
  const res = r.result || {};
  if (r.status === "ok" && res.message) body.appendChild(tsEl("div", "", res.message));
  if (res.table) body.appendChild(tsAlgoTableEl(res.table));
  return body;
}

// 表（200 行まで）。1 列目が id なら、行を押すとそのイベントへ移る
function tsAlgoTableEl(table) {
  const cols = Array.isArray(table.columns) ? table.columns : [];
  const rows = Array.isArray(table.rows) ? table.rows : [];
  const wrap = tsEl("div", "ts-algo-tablewrap");
  const n = rows.length > tsAlgoMaxRows ? `全 ${rows.length} 件（先頭 ${tsAlgoMaxRows} 件）` : `${rows.length} 件`;
  wrap.appendChild(tsEl("div", "ts-desc", n));
  const t = tsEl("table", "ts-algo-table");
  const head = tsEl("tr");
  cols.forEach((c) => head.appendChild(tsEl("th", "", String(c))));
  t.appendChild(head);
  const link = cols[0] === "id";
  for (const row of rows.slice(0, tsAlgoMaxRows)) {
    const tr = tsEl("tr");
    if (link) {
      tr.className = "ts-algo-link";
      tr.title = "イベントへ移る";
      tr.onclick = () => window.TS.openEvent(String(row[0]));
    }
    (Array.isArray(row) ? row : []).forEach((v) => tr.appendChild(tsEl("td", "", tsAlgoCell(v))));
    t.appendChild(tr);
  }
  wrap.appendChild(t);
  return wrap;
}

function tsAlgoCell(v) {
  if (v === null || v === undefined) return "";
  if (typeof v === "number" && !Number.isInteger(v)) return String(Number(v.toPrecision(6)));
  return String(v);
}

// 種類の絞り込み・ch のタブの「＋ ch を足す」から開く。vars は ch の対象の初期値
function tsAlgoOpen({ kind, vars } = {}) {
  tsAlgo.kind = kind || "";
  tsSidebarSelect("algos", true);
  tsAlgoDrawFilter();
  tsAlgoDrawList();
  // 選んでいれば欄を作り直す（入れた値は残す）。選んでいなければ、次に選んだとき使う
  const v = Array.isArray(vars) ? vars : null;
  if (tsAlgo.spec) {
    tsAlgoBuildDetail(tsAlgoRawParams(), v);
  } else {
    tsAlgo.vars = v;
  }
}

function tsAlgoOnState() {
  const ds = window.TS.state.dataset;
  if (ds === tsAlgo.ds) return;
  tsAlgo.ds = ds;
  tsAlgo.runs = [];
  tsAlgoLoadRuns();
}

function tsAlgoBoot() {
  tsAlgoStyle();
  window.TS.ui.addSidebarTab({ id: "algos", title: "アルゴリズム", render: tsAlgoRender });
  window.TS.on("algos", tsAlgoLoadList);
  window.TS.on("runs", (m) => {
    if (m.dataset === window.TS.state.dataset) tsAlgoLoadRuns();
  });
  window.TS.on("state", tsAlgoOnState);
}

if (typeof window !== "undefined" && window.TS) {
  window.TS.ui.openAlgos = tsAlgoOpen;
  window.TS.on("boot", tsAlgoBoot);
}

if (typeof module !== "undefined") {
  module.exports = { tsAlgoDefaults, tsAlgoRead, tsAlgoTargetIds, tsRunsFinished, tsAlgoFilter };
}
