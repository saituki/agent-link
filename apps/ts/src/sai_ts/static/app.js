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
const tsModes = ["create", "move", "zoom"];
const tsSideMinW = 260; // サイドバーの幅の最小（px）
const tsSideDefaultW = 360;
const tsSideStep = 16; // ← → で動く幅（px）
let tsSideW = tsSideDefaultW; // 覚えたサイドバーの幅（窓が狭いときの表示は tsSideApply で収める）
let tsEvdefDs = null; // 作成に使う定義を読んだデータセット

// 既定の色（ダークの背景で読める明るさ）。color が空の ch は並びの番号 % 10 の色を使う
const tsPalette = ["#7dd3fc", "#fca5a5", "#86efac", "#fde68a", "#c4b5fd", "#f9a8d4", "#5eead4", "#fdba74", "#a5b4fc", "#bef264"];

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
  // データセットが変わるときは、前のデータセットの表示中の変数を持ち越さない（全部表示から始める）
  if ("dataset" in patch && patch.dataset !== st.dataset && !("vars" in patch)) st.vars = null;
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

// サイドバーの幅を収める（最小 260px・窓の幅の 60% まで。両方が食い違うときは最小が勝つ）
function tsClampSide(w, winW) {
  const v = Number.isFinite(w) ? w : tsSideDefaultW;
  return Math.min(Math.max(tsSideMinW, winW * 0.6), Math.max(tsSideMinW, v));
}

// 保存の ch 設定（G1-C1）と記録の変数を合わせる。記録に無い名前は出さず、保存に無い変数は既定の値で後ろへ
function tsMergeChannels(stored, metaVars) {
  const meta = Array.isArray(metaVars) ? metaVars : [];
  const has = new Set(meta.map((v) => v.name));
  const unit = new Map(meta.map((v) => [v.name, v.unit || ""]));
  const picked = [];
  const seen = new Set();
  for (const c of Array.isArray(stored) ? stored : []) {
    if (!has.has(c.name) || seen.has(c.name)) continue;
    seen.add(c.name);
    picked.push(c);
  }
  for (const v of meta) {
    if (seen.has(v.name)) continue;
    seen.add(v.name);
    picked.push({ name: v.name });
  }
  return picked.map((c, i) => {
    const color = typeof c.color === "string" ? c.color : "";
    return {
      name: c.name,
      label: typeof c.label === "string" ? c.label : "",
      color,
      drawColor: color || tsPalette[i % tsPalette.length],
      gain: typeof c.gain === "number" ? c.gain : 1,
      spectrum: c.spectrum ?? null,
      unit: unit.get(c.name),
    };
  });
}

// 表示する名前を channels の順に並べ直す（visible が null なら全部）
function tsOrderVisible(channels, visible) {
  const names = channels.map((c) => c.name);
  if (!Array.isArray(visible)) return names;
  const on = new Set(visible);
  return names.filter((n) => on.has(n));
}

// 作成に使う定義の表示（「● 名前（エリア・1ch）」）。type か ch が無いときは受けるものを書く
function tsEvdefLabel(c) {
  const type = c.type === "line" ? "ポイント" : c.type === "area" ? "エリア" : "エリア/ポイント";
  const ch = c.ch === "single" ? "1ch" : c.ch === "all" ? "記録全体" : "自由";
  return `● ${c.name}（${type}・${ch}）`;
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
  // データセットが変わったら、そのデータセットの schema（イベント定義）に読み直す
  if (st.dataset !== tsSchemaDs) {
    tsSchemaDs = st.dataset;
    await tsReloadSchema();
    if (seq !== tsLoadSeq) return;
  }
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
  await tsReloadChannels();
  if (seq !== tsLoadSeq) return;
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

let tsSchemaDs = null; // schema を読んだデータセット

async function tsReloadSchema() {
  const st = window.TS.state;
  const ds = st.dataset;
  if (!ds) return;
  const s = await tsApi(`/api/datasets/${tsEnc(ds)}/schema`).catch(() => null);
  if (st.dataset !== ds) return;
  tsSchemaDs = ds;
  st.schema = tsNormSchema(s);
  if (st.eventDef && !st.schema.categories.some((c) => c.name === st.eventDef)) st.eventDef = null;
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
  else if (msg.kind === "channels" && same) job = tsReloadChannels().then(() => tsEmit("state"));
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
  tsOn("state", tsEvdefSync);
  tsOn("state", tsRenderHeader);
  tsOn("mode", tsRenderHeader);
  tsBindHeader();
  tsRangeInit();
  tsSplitInit();
  tsObserveLanes();
  tsLanesInit();
  tsEvformInit();
  window.addEventListener("keydown", tsOnKeyDown);

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
  tsRenderModes();
  tsRenderEvdef();
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

// 作成のモードの釦（押されている物が aria-pressed）
function tsRenderModes() {
  const st = window.TS.state;
  for (const b of document.querySelectorAll("#ts-modes button")) {
    b.setAttribute("aria-pressed", String(b.dataset.mode === st.mode));
  }
}

// 作成に使う定義の選び（先頭は「定義なし」）。項目が変わったときだけ作り直す（開いたままの選択欄を閉じないため）
function tsRenderEvdef() {
  const st = window.TS.state;
  const sel = document.getElementById("ts-evdef");
  const cats = st.schema.categories;
  const key = cats.map(tsEvdefLabel).join("\n");
  if (sel.dataset.key !== key) {
    sel.dataset.key = key;
    const none = tsEl("option", "", "（定義なし）");
    none.value = "";
    const opts = cats.map((c) => {
      const o = tsEl("option", "", tsEvdefLabel(c));
      o.value = c.name;
      if (c.color) o.style.color = c.color;
      return o;
    });
    sel.replaceChildren(none, ...opts);
  }
  sel.value = st.eventDef || "";
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
  for (const b of document.querySelectorAll("#ts-modes button")) {
    b.onclick = () => window.TS.setMode(b.dataset.mode);
  }
  document.getElementById("ts-evdef").onchange = (e) => window.TS.setEventDef(e.target.value || null);
  tsInstallModeKeys();
}

function tsInstallModeKeys() {
  const ui = window.TS.ui;
  ui.addKey({ key: "e", desc: "作成モード", handler: () => window.TS.setMode("create") });
  ui.addKey({ key: "v", desc: "移動モード", handler: () => window.TS.setMode("move") });
  ui.addKey({ key: "z", desc: "拡大モード", handler: () => window.TS.setMode("zoom") });
}

// ---- 境界線（サイドバーの幅）。ドラッグと ← → でだけ変わる（窓の大きさは tsClampSide で収める） ----

// 表示の幅を #ts-app に書く
function tsSideApply() {
  const app = document.getElementById("ts-app");
  if (app) app.style.setProperty("--ts-side-w", tsClampSide(tsSideW, window.innerWidth) + "px");
}

function tsSplitInit() {
  const raw = tsStoredGet("ts_side_w");
  tsSideW = raw === null ? tsSideDefaultW : Number(raw);
  const split = document.getElementById("ts-split");
  let drag = false;
  const setW = (w) => {
    tsSideW = tsClampSide(w, window.innerWidth);
    tsSideApply();
  };
  split.addEventListener("pointerdown", (e) => {
    if (e.button !== 0) return;
    e.preventDefault();
    split.focus();
    drag = true;
    split.setPointerCapture(e.pointerId);
  });
  split.addEventListener("pointermove", (e) => {
    if (drag) setW(window.innerWidth - e.clientX);
  });
  const end = () => {
    if (!drag) return;
    drag = false;
    tsStoredSet("ts_side_w", String(tsSideW));
  };
  split.addEventListener("pointerup", end);
  split.addEventListener("pointercancel", end);
  split.addEventListener("keydown", (e) => {
    if (e.key !== "ArrowLeft" && e.key !== "ArrowRight") return;
    e.preventDefault();
    e.stopPropagation(); // 矢印は波形の移動に使わない（window の矢印の操作より先に止める）
    setW(tsClampSide(tsSideW, window.innerWidth) + (e.key === "ArrowLeft" ? tsSideStep : -tsSideStep));
    tsStoredSet("ts_side_w", String(tsSideW));
  });
  window.addEventListener("resize", tsSideApply);
  tsSideApply();
}

// 波形の欄の大きさを測る。幅か高さが変わったときだけ描き直して "resize" を知らせる（取り直しは lanes.js）
function tsObserveLanes() {
  const lanes = document.getElementById("ts-lanes");
  if (!lanes || typeof ResizeObserver === "undefined") return;
  let w = -1;
  let h = -1;
  new ResizeObserver((entries) => {
    const r = entries[0].contentRect;
    if (r.width === w && r.height === h) return;
    w = r.width;
    h = r.height;
    tsRedraw();
    window.TS.emit("resize");
  }).observe(lanes);
}

// ---- 作成の状態（モード・定義・ch 設定） ----

// 保存（localStorage）。使えなくても画面は動く
function tsStoredGet(key) {
  try {
    return localStorage.getItem(key);
  } catch {
    return null;
  }
}

function tsStoredSet(key, value) {
  try {
    if (value === null) localStorage.removeItem(key);
    else localStorage.setItem(key, value);
  } catch {
    // 保存できなくても画面は動く
  }
}

function tsStoredMode() {
  const m = tsStoredGet("ts_mode");
  return tsModes.includes(m) ? m : "move";
}

// データセットが変わったら、作成に使う定義を覚えた値から読む（schema を読んだあと tsReloadSchema が確かめる）
function tsEvdefSync() {
  const st = window.TS.state;
  if (!st.dataset || st.dataset === tsEvdefDs) return;
  tsEvdefDs = st.dataset;
  st.eventDef = tsStoredGet("ts_evdef:" + st.dataset) || null;
}

function tsSetMode(mode) {
  if (!tsModes.includes(mode)) return;
  window.TS.state.mode = mode;
  tsStoredSet("ts_mode", mode);
  window.TS.emit("mode");
}

// schema に無い名前は null（作成しない）。データセットごとに覚える
function tsSetEventDef(name) {
  const st = window.TS.state;
  const next = name && st.schema.categories.some((c) => c.name === name) ? name : null;
  st.eventDef = next;
  if (st.dataset) tsStoredSet("ts_evdef:" + st.dataset, next);
  window.TS.emit("mode");
}

// ch 設定を読み直す（G1-C1）。404 は空の channels として扱う（API が無いとき）。
// 並びは meta の変数と合わせ、表示中の変数も channels の順に並べ直す
async function tsReloadChannels() {
  const st = window.TS.state;
  const ds = st.dataset;
  const r = ds
    ? await tsApi(`/api/datasets/${tsEnc(ds)}/channels`).catch((e) => {
        if (e.status !== 404) tsToast("ch 設定を読めません: " + e.message);
        return null;
      })
    : null;
  if (st.dataset !== ds) return;
  const stored = r && Array.isArray(r.channels) ? r.channels : [];
  st.channels = tsMergeChannels(stored, st.meta ? st.meta.vars : []);
  if (Array.isArray(st.vars)) st.vars = tsOrderVisible(st.channels, st.vars);
  tsRedraw();
  tsEmit("channels");
}

// ch 設定を保存する（list は G1-C1 の形）。成功したら読み直す。失敗は false
async function tsSaveChannels(list) {
  const ds = window.TS.state.dataset;
  if (!ds) return false;
  const channels = list.map((c) => {
    const o = { name: c.name, label: c.label, color: c.color, gain: c.gain };
    if (c.spectrum) o.spectrum = c.spectrum;
    return o;
  });
  try {
    await tsApi(`/api/datasets/${tsEnc(ds)}/channels`, { method: "PUT", body: { version: 1, channels } });
  } catch (e) {
    tsToast("ch 設定を保存できません: " + e.message);
    return false;
  }
  await tsReloadChannels();
  return true;
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
      mode: tsStoredMode(), // "create"・"move"・"zoom"
      eventDef: null, // 作成に使う定義の名前（null は定義なし）
      channels: [], // ch 設定（G1-C1 と記録の変数を合わせたもの）
    },
    setView: tsSetView,
    setMode: tsSetMode,
    setEventDef: tsSetEventDef,
    reloadChannels: tsReloadChannels,
    saveChannels: tsSaveChannels,
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
    tsClampSide,
    tsMergeChannels,
    tsOrderVisible,
    tsEvdefLabel,
    tsPalette,
  };
}
