// 拡張の読み込み（U05）。ts/ext/<name>/ の JS を画面に直接 import する（C8・C10）。
// 拡張ごとに包んだ ts を渡し、足したボタン・キー・層・タブ・購読は外すときに全部外す。

// 読むもの・外すもの・そのままのものを決める。
// current: {name: version}（読み込み済み）、list: GET /api/ext の exts、force: 全部を読み直す
function tsExtPlan(current, list, force = false) {
  const load = [];
  const keep = [];
  const unload = [];
  const want = new Set();
  for (const info of list) {
    // 無効・誤り・フォルダ無し（version が null）は読まない
    if (!info.enabled || info.error || info.version == null) continue;
    want.add(info.name);
    const same = Object.prototype.hasOwnProperty.call(current, info.name) && current[info.name] === info.version;
    if (same && !force) keep.push(info.name);
    else load.push(info);
  }
  for (const name of Object.keys(current)) {
    if (!want.has(name)) unload.push(name);
  }
  return { load, keep, unload };
}

// import する URL（版を付けて、変わったら取り直させる）。base は画面の基準（C1）
function tsExtUrl(info, token, base) {
  const path = String(info.entry).split("/").map(encodeURIComponent).join("/");
  const tok = token ? `&token=${encodeURIComponent(token)}` : "";
  return tsUrl(`/ext/${encodeURIComponent(info.name)}/${path}?v=${encodeURIComponent(info.version)}${tok}`, base);
}

const tsExt = {
  loaded: new Map(), // name -> {version, ts, regs: [dispose], dispose: fn|null, error: string|null}
  list: [], // 直近の GET /api/ext
  panel: null,
};

function tsExtStyle() {
  if (document.getElementById("ts-style-exthost.js")) return;
  const st = document.createElement("style");
  st.id = "ts-style-exthost.js";
  st.textContent = `
.ts-ext-row { padding: 6px 0; border-bottom: 1px solid #ddd; }
.ts-ext-row .ts-ext-ver { color: #777; font-size: 12px; margin-left: 6px; }
.ts-ext-row .ts-ext-err { color: #b00; font-size: 12px; white-space: pre-wrap; }
`;
  document.head.appendChild(st);
}

// 拡張用の ts。window.TS の写しで、登録と購読だけ覚えて外せるようにする
function tsExtMakeTs(rec) {
  const track = (dispose) => {
    rec.regs.push(dispose);
    return dispose;
  };
  const ui = Object.create(TS.ui);
  ui.addHeaderButton = (o) => track(TS.ui.addHeaderButton(o));
  ui.addKey = (o) => track(TS.ui.addKey(o));
  ui.addLayer = (o) => track(TS.ui.addLayer(o));
  ui.addSidebarTab = (o) => track(TS.ui.addSidebarTab(o));
  const ts = Object.create(TS);
  ts.ui = ui;
  ts.on = (kind, fn) => track(TS.on(kind, fn));
  return ts;
}

// 覚えた登録を全部外す（外せなかった物は捨てて続ける）
function tsExtRelease(rec) {
  for (const d of rec.regs.splice(0)) {
    try {
      d();
    } catch (e) {
      console.warn("拡張の登録を外せません", e);
    }
  }
}

function tsExtVersions() {
  return Object.fromEntries([...tsExt.loaded].map(([name, rec]) => [name, rec.version]));
}

async function tsLoadExt(info) {
  tsUnloadExt(info.name);
  const rec = { version: info.version, regs: [], dispose: null, error: null, ts: null };
  rec.ts = tsExtMakeTs(rec);
  tsExt.loaded.set(info.name, rec);
  try {
    const mod = await import(tsExtUrl(info, tsToken()));
    if (typeof mod.default !== "function") throw new Error("export default の関数（activate）が無い");
    const d = await mod.default(rec.ts);
    // activate の途中で外されていたら、今の登録を全部外す
    if (tsExt.loaded.get(info.name) !== rec) {
      if (typeof d === "function") d();
      tsExtRelease(rec);
      return;
    }
    if (typeof d === "function") rec.dispose = d;
  } catch (e) {
    rec.error = e.message || String(e);
    tsExtRelease(rec);
    TS.ui.toast(`拡張 ${info.name} を読めません: ${rec.error}`);
  }
  TS.redraw();
  tsExtDraw();
}

function tsUnloadExt(name) {
  const rec = tsExt.loaded.get(name);
  if (!rec) return;
  tsExt.loaded.delete(name);
  if (rec.dispose) {
    try {
      rec.dispose();
    } catch (e) {
      console.warn(`拡張 ${name} の後始末で誤り（無視して続ける）`, e);
    }
  }
  tsExtRelease(rec);
  TS.redraw();
  tsExtDraw();
}

// GET /api/ext を読んで、読む・外すを決めて実行する
async function tsExtSync(force = false) {
  let list;
  try {
    list = (await TS.api("/api/ext")).exts || [];
  } catch (e) {
    TS.ui.toast(`拡張の一覧を読めません: ${e.message}`);
    return;
  }
  tsExt.list = list;
  const plan = tsExtPlan(tsExtVersions(), list, force);
  for (const name of plan.unload) tsUnloadExt(name);
  for (const info of plan.load) await tsLoadExt(info);
  tsExtDraw();
}

function tsExtState(info) {
  const rec = tsExt.loaded.get(info.name);
  if (!info.enabled) return { label: "無効", err: "" };
  if (info.error) return { label: "誤り", err: info.error };
  if (rec && rec.error) return { label: "誤り", err: rec.error };
  if (rec) return { label: "読み込み済み", err: "" };
  return { label: "未読み込み", err: "" };
}

function tsExtDraw() {
  const box = tsExt.panel;
  if (!box || !box.isConnected) return;
  box.replaceChildren();
  if (tsExt.list.length === 0) {
    const p = document.createElement("p");
    p.textContent = "拡張はありません（ts/ext/<名前>/manifest.json を置く）";
    box.appendChild(p);
    return;
  }
  for (const info of tsExt.list) {
    const row = document.createElement("div");
    row.className = "ts-ext-row";
    const st = tsExtState(info);
    const title = document.createElement("b");
    title.textContent = info.title || info.name;
    const meta = document.createElement("span");
    meta.className = "ts-ext-ver";
    meta.textContent = `${info.name} · 版 ${info.version ? info.version.slice(0, 12) : "—"} · ${st.label}`;
    row.append(title, meta);
    if (st.err) {
      const e = document.createElement("div");
      e.className = "ts-ext-err";
      e.textContent = st.err;
      row.appendChild(e);
    }
    box.appendChild(row);
  }
}

function tsExtRender(el) {
  tsExtStyle();
  el.replaceChildren();
  const btn = document.createElement("button");
  btn.type = "button";
  btn.textContent = "読み直す";
  btn.addEventListener("click", () => tsExtSync(true));
  const box = document.createElement("div");
  el.append(btn, box);
  tsExt.panel = box;
  tsExtDraw();
}

function tsExtBoot() {
  TS.ui.addSidebarTab({ id: "ext", title: "拡張", render: tsExtRender });
  TS.on("ext", (msg) => {
    // 消えた拡張は外すだけ。変わった拡張は一覧を取り直して読み直す
    if (msg.version == null) {
      tsUnloadExt(msg.name);
      tsExtDraw();
    } else {
      tsExtSync();
    }
  });
  tsExtSync();
}

if (typeof window !== "undefined" && window.TS) {
  window.TS.on("boot", tsExtBoot);
}

if (typeof module !== "undefined") {
  module.exports = { tsExtPlan, tsExtUrl };
}
