"use strict";
// sai-ts の画面の骨組み（共有契約 C10）。window.TS を作り、状態・取得・ヘッダ・変化の知らせを受ける。
// 波形と重ね描きは lanes.js、イベントの入力欄は evform.js。sidebar（U05）は TS.ui.addSidebarTab などで足す。
// 画面の一番外側の名前はすべて ts で始める（ファイルが重ならないように）。

const tsMinSamples = 10; // 範囲の最小幅（サンプル数）
const tsNameRe = /^[A-Za-z0-9][A-Za-z0-9_.-]{0,63}$/;
const tsListeners = {};
const tsLayers = [];
const tsKeys = [];
let tsSaveTimer = null;
let tsLoadSeq = 0;
let tsBootDone = false;

// ---- 純関数（DOM を使わない。node のテストが読む） ----

// 名前の形（C1）。`..` を含まず、正規表現に合う
function tsValidName(name) {
  return typeof name === "string" && !name.includes("..") && tsNameRe.test(name);
}

// 範囲を [0, dur] に収める。幅は minSpan より狭くしない
function tsClampRange(t0, t1, dur, minSpan) {
  const span = Math.min(Math.max(t1 - t0, minSpan), dur);
  const a = Math.min(Math.max(t0, 0), dur - span);
  return { t0: a, t1: a + span };
}

// tc を支点に幅を factor 倍する（factor < 1 で拡大）
function tsZoomAt(t0, t1, tc, factor, dur, minSpan) {
  const ratio = t1 > t0 ? (tc - t0) / (t1 - t0) : 0.5;
  const span = (t1 - t0) * factor;
  const a = tc - ratio * span;
  return tsClampRange(a, a + span, dur, minSpan || 0);
}

// 幅を保ったまま dt 秒ずらす
function tsPan(t0, t1, dt, dur) {
  return tsClampRange(t0 + dt, t1 + dt, dur, t1 - t0);
}

// 1・2・5 × 10^k 秒の目盛り。maxTicks 個前後
function tsNiceTicks(t0, t1, maxTicks) {
  const span = t1 - t0;
  if (!(span > 0) || !(maxTicks > 0)) return [];
  const raw = span / maxTicks;
  const mag = Math.pow(10, Math.floor(Math.log10(raw)));
  const step = [1, 2, 5, 10].map((m) => m * mag).find((s) => s >= raw);
  const out = [];
  for (let i = Math.ceil(t0 / step - 1e-9); i * step <= t1 + 1e-9; i++) {
    out.push(+(i * step).toPrecision(12));
  }
  return out;
}

// 範囲の入力（秒の文字）を範囲にする。数でない・幅が 0 以下・記録が無いときは null。記録の外は収める
function tsRangeFromInput(startText, winText, dur, minSpan) {
  const s = tsNumText(startText);
  const w = tsNumText(winText);
  if (!Number.isFinite(s) || !Number.isFinite(w) || w <= 0 || !(dur > 0)) return null;
  return tsClampRange(s, s + w, dur, minSpan);
}

// 入力の文字を数にする。空は NaN（0 と読まない）
function tsNumText(text) {
  const t = String(text === null || text === undefined ? "" : text).trim();
  return t === "" ? NaN : Number(t);
}

// 記録の長さ（秒）
function tsDuration(meta) {
  return meta && meta.srate ? meta.n_samples / meta.srate : 0;
}

// 画面が置かれた場所（"/" か "/p/<名前>/"）。住所はここを基準にする
function tsBase() {
  return typeof location === "undefined" ? "/" : new URL(".", location.href).pathname;
}

// "/api/…" → 基準 + "api/…"。"/" で始まらない・http(s): はそのまま
function tsUrl(path, base) {
  const b = base === undefined ? tsBase() : base;
  return typeof path === "string" && path.startsWith("/") && !path.startsWith("//") ? b + path.slice(1) : path;
}

// URL の #token= か ?token= を取り出し、token を消した問い合わせと断片を返す（#token= が優先）
function tsStripToken(search, hash) {
  const sq = new URLSearchParams(search || "");
  const hq = new URLSearchParams(String(hash || "").replace(/^#/, ""));
  const token = hq.get("token") || sq.get("token") || "";
  sq.delete("token");
  let frag = hash || "";
  if (hq.has("token")) {
    hq.delete("token");
    frag = hq.toString() ? "#" + hq.toString() : "";
  }
  const rest = sq.toString();
  return { token, query: rest ? "?" + rest : "", hash: frag };
}

function tsStreamUrl(token, base) {
  return tsUrl("/api/stream", base) + (token ? "?token=" + encodeURIComponent(token) : "");
}

// PUT /api/view に送る形（C6）。空の項目は送らない
function tsViewBody(st) {
  const b = {
    t0: st.t0,
    t1: st.t1,
    hidden_categories: [...st.hiddenCategories],
    selection: st.selection,
    selected_events: st.selectedEvents.slice(),
  };
  if (st.dataset) b.dataset = st.dataset;
  if (st.record) b.record = st.record;
  if (Array.isArray(st.vars)) b.vars = st.vars.slice();
  return b;
}

// patch（画面の状態の名前）を st に写す。dataset か record が変わったら true。
// 記録が変わって selection が無いときは、前の記録の選択を消す（古い選択を送り続けない）
function tsPatchState(st, patch) {
  const before = st.dataset + "\n" + st.record;
  if ("dataset" in patch) st.dataset = patch.dataset;
  if ("record" in patch) st.record = patch.record;
  if ("t0" in patch) st.t0 = patch.t0;
  if ("t1" in patch) st.t1 = patch.t1;
  if ("vars" in patch) st.vars = patch.vars === null ? null : patch.vars.slice();
  if ("hiddenCategories" in patch) st.hiddenCategories = new Set(patch.hiddenCategories);
  if ("selection" in patch) st.selection = patch.selection ? Object.assign({}, patch.selection) : null;
  if ("selectedEvents" in patch) st.selectedEvents = patch.selectedEvents.slice();
  const changed = before !== st.dataset + "\n" + st.record;
  if (changed && !("selection" in patch)) st.selection = null;
  return changed;
}

function tsRecordsOf(datasets, ds) {
  const d = datasets.find((x) => x.name === ds);
  return d ? d.records : [];
}

// schema.json の形を揃える（無いときの既定を含む）
function tsNormSchema(s) {
  return {
    version: 1,
    categories: s && Array.isArray(s.categories) ? s.categories : [],
    metrics: s && Array.isArray(s.metrics) ? s.metrics : [],
  };
}

// ---- ここから画面（ブラウザだけで動く） ----

function tsEnc(s) {
  return encodeURIComponent(s);
}

function tsEl(tag, cls, text) {
  const el = document.createElement(tag);
  if (cls) el.className = cls;
  if (text !== undefined) el.textContent = text;
  return el;
}

let tsAuthToastAt = 0;

// fetch の包み。Bearer を付け、HTTP エラーは detail を持つ Error にする。401 は合言葉の知らせ（30 秒に 1 回）
function tsApi(path, opts) {
  const o = Object.assign({}, opts || {});
  const headers = Object.assign({}, o.headers || {});
  const tok = tsToken();
  if (tok) headers.Authorization = "Bearer " + tok;
  if (o.body !== undefined) {
    headers["Content-Type"] = "application/json";
    o.body = JSON.stringify(o.body);
  }
  o.headers = headers;
  return fetch(tsUrl(path), o).then(async (r) => {
    const text = await r.text();
    let body = null;
    try {
      body = text ? JSON.parse(text) : null;
    } catch {
      body = null;
    }
    if (!r.ok) {
      if (r.status === 401 && Date.now() - tsAuthToastAt > 30000) {
        tsAuthToastAt = Date.now();
        tsToast("時系列の合言葉が無効です。sai-web から開き直してください");
      }
      const d = body && body.detail;
      const err = new Error(typeof d === "string" ? d : "HTTP " + r.status);
      err.status = r.status;
      throw err;
    }
    return body;
  });
}

// 合言葉は タブごとの sessionStorage（同じオリジンの別プロジェクトと共有しない・C1）
function tsToken() {
  try {
    return sessionStorage.getItem("ts_token") || "";
  } catch {
    return "";
  }
}

function tsEmit(kind, payload) {
  (tsListeners[kind] || []).slice().forEach((fn) => {
    try {
      fn(payload);
    } catch (e) {
      console.error(e);
    }
  });
}

function tsOn(kind, fn) {
  (tsListeners[kind] = tsListeners[kind] || []).push(fn);
  return () => {
    tsListeners[kind] = tsListeners[kind].filter((f) => f !== fn);
  };
}

function tsRedraw() {
  if (typeof tsDrawAll === "function") tsDrawAll();
}

// 画面の状態を変えて描き直し、500ms まとめて保存する
function tsSetView(patch) {
  const st = window.TS.state;
  const recChanged = tsPatchState(st, patch);
  tsRedraw();
  tsEmit("state");
  if (recChanged) tsLoadRecord();
  tsScheduleSave();
}

function tsScheduleSave() {
  clearTimeout(tsSaveTimer);
  tsSaveTimer = setTimeout(() => {
    tsSaveTimer = null;
    tsApi("/api/view", { method: "PUT", body: tsViewBody(window.TS.state) }).catch((e) =>
      tsToast("画面の状態を保存できません: " + e.message),
    );
  }, 500);
}

// 記録の meta と、範囲・変数・イベントを揃える（記録を切り替えたとき・変化を受けたとき）
async function tsLoadRecord() {
  const st = window.TS.state;
  const seq = ++tsLoadSeq;
  const key = st.dataset + "\n" + st.record; // 読み始めた記録（meta がどの記録のものかを表す）
  let meta = null;
  if (st.dataset && st.record) {
    meta = await tsApi(`/api/datasets/${tsEnc(st.dataset)}/records/${tsEnc(st.record)}/meta`).catch((e) => {
      tsToast("記録を読めません: " + e.message);
      return null;
    });
  }
  if (seq !== tsLoadSeq) return;
  st.meta = meta;
  st.metaKey = meta ? key : null;
  const names = meta ? meta.vars.map((v) => v.name) : [];
  st.vars = st.vars ? st.vars.filter((n) => names.includes(n)) : names;
  const dur = tsDuration(meta);
  const minSpan = meta && meta.srate ? tsMinSamples / meta.srate : 0;
  if (st.t1 > st.t0) {
    const r = tsClampRange(st.t0, st.t1, dur, minSpan);
    st.t0 = r.t0;
    st.t1 = r.t1;
  } else {
    st.t0 = 0;
    st.t1 = dur;
  }
  await tsReloadEvents();
  if (seq !== tsLoadSeq) return;
  tsRedraw();
  tsEmit("state");
}

async function tsReloadEvents() {
  const st = window.TS.state;
  const ds = st.dataset;
  const rec = st.record;
  if (!ds) {
    st.events = [];
    return;
  }
  const q = rec ? `?record=${tsEnc(rec)}` : "";
  const r = await tsApi(`/api/datasets/${tsEnc(ds)}/events${q}`).catch(() => ({ events: [] }));
  if (st.dataset !== ds || st.record !== rec) return;
  st.events = Array.isArray(r.events) ? r.events : [];
  tsRedraw();
}

async function tsReloadSchema() {
  const st = window.TS.state;
  const ds = st.dataset;
  if (!ds) return;
  const s = await tsApi(`/api/datasets/${tsEnc(ds)}/schema`).catch(() => null);
  if (st.dataset !== ds) return;
  st.schema = tsNormSchema(s);
  tsRenderHeader();
  tsRedraw();
}

// view（C6）の内容を画面に写す。記録が変わったら meta などを取り直す
function tsApplyView(view) {
  const st = window.TS.state;
  const names = st.datasets.map((d) => d.name);
  const dataset = names.includes(view.dataset) ? view.dataset : names.includes(st.dataset) ? st.dataset : names[0] || null;
  const recs = tsRecordsOf(st.datasets, dataset);
  const record = recs.includes(view.record) ? view.record : recs.includes(st.record) ? st.record : recs[0] || null;
  tsPatchState(st, {
    dataset,
    record,
    t0: view.t0 ?? st.t0,
    t1: view.t1 ?? st.t1,
    vars: view.vars ?? st.vars,
    hiddenCategories: view.hidden_categories ?? [...st.hiddenCategories],
    selection: view.selection ?? null,
    selectedEvents: view.selected_events ?? [],
  });
  return tsLoadRecord();
}

// agent が view を書いたときだけ取り込む（画面自身の保存の知らせは無視する）
async function tsPullView() {
  const view = await tsApi("/api/view").catch(() => null);
  if (view && view.updated_by === "agent") await tsApplyView(view);
}

function tsOnChange(msg) {
  const st = window.TS.state;
  const same = msg.dataset === st.dataset;
  let job = Promise.resolve();
  if (msg.kind === "events" && same) job = tsReloadEvents();
  else if (msg.kind === "schema" && same) job = tsReloadSchema();
  else if (msg.kind === "vars" && same && msg.record === st.record) job = tsLoadRecord();
  else if (msg.kind === "view") job = tsPullView();
  job.then(() => tsEmit(msg.kind, msg));
}

function tsConnectStream() {
  if (typeof EventSource === "undefined") return;
  const es = new EventSource(tsStreamUrl(tsToken()));
  es.onmessage = (m) => {
    let msg = null;
    try {
      msg = JSON.parse(m.data);
    } catch {
      return;
    }
    tsOnChange(msg);
  };
}

async function tsBoot() {
  const tok = tsStripToken(location.search, location.hash);
  if (tok.token) {
    try {
      sessionStorage.setItem("ts_token", tok.token);
    } catch {
      // 保存できなくても画面は開く（その回の要求だけ合言葉が要る）
    }
    history.replaceState(null, "", location.pathname + tok.query + tok.hash);
  }
  tsOn("state", tsRenderHeader);
  tsBindHeader();
  tsRangeInit();
  tsLanesInit();
  tsEvformInit();
  window.addEventListener("keydown", tsOnKeyDown);
  document.addEventListener("mousedown", (e) => {
    if (!e.target.closest("#ts-vars-wrap")) document.getElementById("ts-vars").hidden = true;
  });

  const view = await tsApi("/api/view").catch(() => ({}));
  const list = await tsApi("/api/datasets").catch(() => ({ datasets: [] }));
  window.TS.state.datasets = list.datasets || [];
  await tsApplyView(view || {});
  await tsReloadSchema();
  tsConnectStream();
  if (!tsBootDone) {
    tsBootDone = true;
    tsEmit("boot");
  }
}

// ---- 操作の受け口（window.TS） ----

function tsOnKeyDown(e) {
  if (e.ctrlKey || e.metaKey || e.altKey) return;
  if (tsIsEditing(e.target)) return;
  for (let i = tsKeys.length - 1; i >= 0; i--) {
    if (tsKeys[i].key === e.key) {
      e.preventDefault();
      tsKeys[i].handler(e);
      return;
    }
  }
}

function tsIsEditing(el) {
  if (!el || !el.tagName) return false;
  return ["INPUT", "TEXTAREA", "SELECT"].includes(el.tagName) || el.isContentEditable === true;
}

function tsAddKey(k) {
  const entry = { key: k.key, desc: k.desc || "", handler: k.handler };
  tsKeys.push(entry);
  return () => {
    const i = tsKeys.indexOf(entry);
    if (i >= 0) tsKeys.splice(i, 1);
  };
}

function tsAddLayer(layer) {
  const item = { id: layer.id, draw: layer.draw };
  tsLayers.push(item);
  tsRedraw();
  return () => {
    const i = tsLayers.indexOf(item);
    if (i >= 0) tsLayers.splice(i, 1);
    tsRedraw();
  };
}

// 重ね層を順に描く。1 つが失敗しても他は描く
function tsRunLayers(ctx, frame) {
  tsLayers.forEach((l) => {
    ctx.save();
    try {
      l.draw(ctx, frame);
    } catch (e) {
      console.error(e);
      tsToast(`層「${l.id}」でエラー: ${e.message}`);
    } finally {
      ctx.restore();
    }
  });
}

function tsAddHeaderButton(b) {
  const btn = tsEl("button", "ts-hbtn", b.label);
  btn.type = "button";
  btn.id = "ts-hb-" + b.id;
  btn.title = b.title || b.label;
  btn.onclick = () => b.onClick && b.onClick();
  document.getElementById("ts-buttons").appendChild(btn);
  return () => btn.remove();
}

function tsToast(msg) {
  const el = tsEl("div", "ts-toast", msg);
  document.body.appendChild(el);
  setTimeout(() => el.remove(), 4000);
}

// ---- ヘッダ ----

function tsRenderHeader() {
  const st = window.TS.state;
  if (!document.getElementById("ts-ds")) return;
  tsFillSelect(document.getElementById("ts-ds"), st.datasets.map((d) => d.name), st.dataset, "データセットがありません");
  const recs = tsRecordsOf(st.datasets, st.dataset);
  tsFillSelect(document.getElementById("ts-rec"), recs, st.record, "記録がありません");
  const i = recs.indexOf(st.record);
  document.getElementById("ts-prev").disabled = i <= 0;
  document.getElementById("ts-next").disabled = i < 0 || i >= recs.length - 1;
  const all = st.meta ? st.meta.vars.length : 0;
  document.getElementById("ts-vars-btn").textContent = `変数 ${st.vars ? st.vars.length : 0}/${all}`;
  tsRenderCats();
  tsRenderVarsPop();
}

// 項目が変わったときだけ作り直す（開いたままの選択欄を閉じないため）
function tsFillSelect(sel, names, current, emptyText) {
  const key = names.join("\n");
  if (sel.dataset.key !== key) {
    sel.dataset.key = key;
    const opts = names.length
      ? names.map((n) => {
          const o = tsEl("option", "", n);
          o.value = n;
          return o;
        })
      : [tsEl("option", "", emptyText)];
    sel.replaceChildren(...opts);
  }
  sel.value = current || "";
  sel.disabled = !names.length;
}

function tsRenderCats() {
  const st = window.TS.state;
  const host = document.getElementById("ts-cats");
  host.replaceChildren(
    ...st.schema.categories.map((c) => {
      const b = tsEl("button", st.hiddenCategories.has(c.name) ? "ts-cat off" : "ts-cat");
      b.type = "button";
      const sw = tsEl("span", "ts-sw");
      sw.style.background = c.color;
      b.append(sw, c.name);
      b.onclick = () => {
        const next = new Set(st.hiddenCategories);
        if (next.has(c.name)) next.delete(c.name);
        else next.add(c.name);
        tsSetView({ hiddenCategories: [...next] });
      };
      return b;
    }),
  );
}

function tsRenderVarsPop() {
  const pop = document.getElementById("ts-vars");
  if (pop.hidden) return;
  const st = window.TS.state;
  const vars = st.meta ? st.meta.vars : [];
  const names = vars.map((v) => v.name);
  const on = new Set(st.vars);
  const rows = vars.map((v) => {
    const lab = tsEl("label", "ts-varrow");
    const cb = tsEl("input");
    cb.type = "checkbox";
    cb.checked = on.has(v.name);
    cb.onchange = () => tsSetView({ vars: names.filter((n) => (n === v.name ? cb.checked : on.has(n))) });
    lab.append(cb, " " + v.name + (v.unit ? ` (${v.unit})` : ""));
    if (v.source === "derived") lab.append(tsEl("span", "ts-tag", "派生"));
    return lab;
  });
  const all = tsEl("button", "", "全部");
  all.type = "button";
  all.onclick = () => tsSetView({ vars: names });
  const none = tsEl("button", "", "なし");
  none.type = "button";
  none.onclick = () => tsSetView({ vars: [] });
  pop.replaceChildren(...rows, tsEl("div", "ts-pop-btns"));
  pop.lastChild.append(all, none);
}

// 表示範囲の帯（ヘッダの下）: 今の範囲の表示・start と window の入力・全体の倍率
function tsRangeInit() {
  const start = document.getElementById("ts-start");
  const win = document.getElementById("ts-win");
  const apply = () => {
    const st = window.TS.state;
    const minSpan = st.meta && st.meta.srate ? tsMinSamples / st.meta.srate : 0;
    const r = tsRangeFromInput(start.value, win.value, tsDuration(st.meta), minSpan);
    for (const el of [start, win]) {
      if (r) el.removeAttribute("aria-invalid");
      else el.setAttribute("aria-invalid", "true");
    }
    if (!r) return;
    start.value = tsSecText(r.t0);
    win.value = tsSecText(r.t1 - r.t0);
    window.TS.setView({ t0: r.t0, t1: r.t1 });
  };
  start.onkeydown = (e) => e.key === "Enter" && apply();
  win.onkeydown = (e) => e.key === "Enter" && apply();
  document.getElementById("ts-apply").onclick = apply;
  const gain = document.getElementById("ts-gain");
  gain.replaceChildren(
    ...tsGainSteps.map((s) => {
      const o = tsEl("option", "", `x${s}`);
      o.value = String(s);
      return o;
    }),
  );
  gain.onchange = () => tsGainSetAll(Number(gain.value));
  tsOn("state", tsRenderRange);
  tsOn("gain", () => {
    gain.value = String(tsGain);
  });
}

function tsSecText(t) {
  return String(+t.toPrecision(8));
}

function tsRenderRange() {
  const st = window.TS.state;
  const has = !!st.meta;
  const dur = tsDuration(st.meta);
  const minSpan = has && st.meta.srate ? tsMinSamples / st.meta.srate : 0;
  const start = document.getElementById("ts-start");
  const win = document.getElementById("ts-win");
  document.getElementById("ts-range-view").textContent = has
    ? `表示 ${tsFmtSec(st.t0)} 〜 ${tsFmtSec(st.t1)}（幅 ${tsFmtSec(st.t1 - st.t0)}）／ 全体 ${tsFmtSec(dur)}`
    : "表示 —（記録がありません）";
  // フォーカス中の欄は、状態の更新で書き換えない（打ちかけの値を消さないため）
  if (document.activeElement !== start) start.value = tsSecText(st.t0);
  if (document.activeElement !== win) win.value = tsSecText(st.t1 - st.t0);
  win.min = String(minSpan);
  for (const el of [start, win, document.getElementById("ts-apply")]) el.disabled = !has;
}

function tsStepRecord(dir) {
  const st = window.TS.state;
  const recs = tsRecordsOf(st.datasets, st.dataset);
  const j = recs.indexOf(st.record) + dir;
  if (j < 0 || j >= recs.length) return;
  tsSetView({ record: recs[j], selectedEvents: [] });
}

function tsBindHeader() {
  document.getElementById("ts-ds").onchange = (e) => {
    const ds = e.target.value;
    tsSetView({ dataset: ds, record: tsRecordsOf(window.TS.state.datasets, ds)[0] || null, selectedEvents: [] });
  };
  document.getElementById("ts-rec").onchange = (e) => tsSetView({ record: e.target.value || null, selectedEvents: [] });
  document.getElementById("ts-prev").onclick = () => tsStepRecord(-1);
  document.getElementById("ts-next").onclick = () => tsStepRecord(1);
  document.getElementById("ts-vars-btn").onclick = () => {
    const pop = document.getElementById("ts-vars");
    pop.hidden = !pop.hidden;
    tsRenderVarsPop();
  };
}

if (typeof window !== "undefined") {
  window.TS = {
    api: tsApi,
    state: {
      dataset: null,
      record: null,
      meta: null,
      t0: 0,
      t1: 0,
      metaKey: null, // meta を読んだ記録（"データセット\n記録"）。読込中は別の記録と見分ける
      vars: null, // null = まだ決めていない（記録を読むとき全変数にする）
      hiddenCategories: new Set(),
      selection: null,
      selectedEvents: [],
      events: [],
      schema: { version: 1, categories: [], metrics: [] },
      datasets: [],
    },
    setView: tsSetView,
    on: tsOn,
    emit: tsEmit,
    redraw: tsRedraw,
    frame: () => tsLanesFrame(),
    reloadEvents: tsReloadEvents,
    reloadSchema: tsReloadSchema,
    reloadMeta: tsLoadRecord,
    ui: {
      addHeaderButton: tsAddHeaderButton,
      addKey: tsAddKey,
      addLayer: tsAddLayer,
      toast: tsToast,
    },
  };
}

if (typeof module !== "undefined") {
  module.exports = {
    tsValidName,
    tsClampRange,
    tsZoomAt,
    tsPan,
    tsNiceTicks,
    tsDuration,
    tsBase,
    tsUrl,
    tsStripToken,
    tsStreamUrl,
    tsViewBody,
    tsPatchState,
    tsRecordsOf,
    tsNormSchema,
    tsRangeFromInput,
  };
}
