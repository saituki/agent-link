// データセットのタブ（U12）。sidebar の「データセット」（id snapshots）。書き出し・一覧・写す・来歴・結ぶ。
// 純関数（tsSnapDefaultName・tsSnapLineage・tsSnapNodeOk）は node で試験する。描画はブラウザで確かめる。

const tsSnapNodeRe = /^[SE][1-9][0-9]{0,8}$/;
const tsSnap = { rec: "cur", ch: "vis", cats: {}, key: "", name: null, opts: null, list: null };

// 書き出しの名前の既定（例 demo-r01-20261011）
function tsSnapDefaultName(ds, rec, date) {
  const p2 = (n) => String(n).padStart(2, "0");
  return `${ds}-${rec}-${date.getFullYear()}${p2(date.getMonth() + 1)}${p2(date.getDate())}`;
}

// S か E の ID の形（G3-C2 の node）
function tsSnapNodeOk(text) {
  return tsSnapNodeRe.test(String(text ?? ""));
}

// ch の来歴の 1 行（raw か、派生の script・sha8・params・入力・run）
function tsSnapLineage(v) {
  const d = v && v.derived;
  if (!d) return "raw";
  const sha = d.sha256 ? `@${String(d.sha256).slice(0, 8)}` : "";
  const params = Object.entries(d.params || {}).map(([k, x]) => `${k}=${typeof x === "string" ? x : JSON.stringify(x)}`);
  let s = [d.script, sha, ...params].filter(Boolean).join(" ");
  if (Array.isArray(d.inputs) && d.inputs.length) s += ` ← ${d.inputs.join(", ")}`;
  if (d.run) s += `（${d.run}）`;
  return s;
}

function tsSnapStyle() {
  if (document.getElementById("ts-style-snapshots.js")) return;
  const st = document.createElement("style");
  st.id = "ts-style-snapshots.js";
  st.textContent = `
.ts-snap-box, .ts-snap-item { border: 1px solid var(--ts-border); border-radius: 6px; padding: 8px; margin-bottom: 8px; background: var(--ts-panel); }
.ts-snap-box fieldset { border: none; padding: 0; margin: 4px 0; }
.ts-snap-box legend { color: var(--ts-muted); font-size: 12px; }
.ts-snap-box label { margin-right: 8px; }
.ts-snap-row { display: flex; gap: 6px; flex-wrap: wrap; align-items: center; margin-top: 6px; }
.ts-snap-item code { font-size: 12px; word-break: break-all; }
.ts-snap-muted { color: var(--ts-muted); font-size: 12px; }
`;
  document.head.appendChild(st);
}

// 表示中の ch（chpanel と同じ決め方）
function tsSnapVisible(st) {
  return st.channels.map((c) => c.name).filter((n) => !Array.isArray(st.vars) || st.vars.includes(n));
}

function tsSnapChoice(title, key, options) {
  const fs = document.createElement("fieldset");
  fs.append(tsEl("legend", "", title));
  for (const [val, text] of options) {
    const lab = document.createElement("label");
    const r = document.createElement("input");
    r.type = "radio";
    r.name = `ts-snap-${key}`;
    r.value = val;
    r.checked = tsSnap[key] === val;
    r.onchange = () => {
      if (r.checked) tsSnap[key] = val;
    };
    lab.append(r, ` ${text}`);
    fs.append(lab);
  }
  return fs;
}

function tsSnapOpts(st, vis, cats) {
  const wrap = document.createElement("div");
  wrap.append(
    tsSnapChoice("記録", "rec", [["cur", `今の記録 ${st.record ?? "（なし）"}`], ["all", "全部"]]),
    tsSnapChoice("ch", "ch", [["vis", `表示中の ch（${vis.length}）`], ["all", "全部"]]),
  );
  const cb = document.createElement("fieldset");
  cb.append(tsEl("legend", "", "イベント"));
  for (const c of cats) {
    const lab = document.createElement("label");
    const input = document.createElement("input");
    input.type = "checkbox";
    input.checked = tsSnap.cats[c] !== false;
    input.onchange = () => {
      tsSnap.cats[c] = input.checked;
    };
    lab.append(input, ` ${c}`);
    cb.append(lab);
  }
  wrap.append(cb);
  return wrap;
}

// 選びが変わったときだけ作り直す（開いたままの選択を閉じないため）
function tsSnapSync(force = false) {
  if (!tsSnap.opts) return;
  const st = window.TS.state;
  const vis = tsSnapVisible(st);
  const cats = (st.schema.categories || []).map((c) => c.name);
  const key = JSON.stringify([st.dataset, st.record, vis, cats]);
  if (!force && key === tsSnap.key) return;
  tsSnap.key = key;
  for (const c of cats) if (!(c in tsSnap.cats)) tsSnap.cats[c] = true;
  if (!tsSnap.name.value.trim() && st.dataset && st.record) {
    tsSnap.name.value = tsSnapDefaultName(st.dataset, st.record, new Date());
  }
  tsSnap.opts.replaceChildren(tsSnapOpts(st, vis, cats));
}

async function tsSnapCreate() {
  const st = window.TS.state;
  const toast = window.TS.ui.toast;
  const name = tsSnap.name.value.trim();
  if (!st.dataset) return toast("データセットを選んでください");
  if (!name) return toast("名前を書いてください");
  if (tsSnap.rec === "cur" && !st.record) return toast("記録を選んでください");
  const vis = tsSnapVisible(st);
  if (tsSnap.ch === "vis" && !vis.length) return toast("表示中の ch がありません");
  const cats = (st.schema.categories || []).map((c) => c.name);
  const picked = cats.filter((c) => tsSnap.cats[c] !== false);
  if (cats.length && !picked.length) return toast("イベントの種類を 1 つ以上選んでください");
  // 空は全部。全部選んだときも空で送る
  const body = {
    name,
    records: tsSnap.rec === "cur" ? [st.record] : [],
    vars: tsSnap.ch === "vis" ? vis : [],
    categories: picked.length === cats.length ? [] : picked,
  };
  try {
    const m = await window.TS.api(`/api/datasets/${tsEnc(st.dataset)}/snapshots`, { method: "POST", body });
    toast(`${m.name} を書き出しました`);
    tsSnap.name.value = "";
    tsSnapSync(true);
    tsSnapReload();
  } catch (e) {
    toast(e.message);
  }
}

async function tsSnapLink(name, node, text) {
  const toast = window.TS.ui.toast;
  if (!tsSnapNodeOk(node)) return toast("S か E の ID を書いてください（例 S3・E12）");
  try {
    const r = await window.TS.api(`/api/snapshots/${tsEnc(name)}/link`, { method: "POST", body: { node, text } });
    toast(`${r.node} に結びました`);
    tsSnapReload();
  } catch (e) {
    toast(e.message);
  }
}

function tsSnapItem(s) {
  const item = tsEl("div", "ts-snap-item");
  const head = tsEl("div", "ts-snap-row");
  head.append(tsEl("b", "", s.name), tsEl("code", "", s.ref));
  const copy = tsEl("button", "", "写す");
  copy.type = "button";
  copy.title = "参照の文字を写す";
  copy.onclick = async () => {
    try {
      await navigator.clipboard.writeText(s.ref);
      window.TS.ui.toast("写しました");
    } catch (e) {
      window.TS.ui.toast("写せませんでした");
    }
  };
  head.append(copy);
  const meta = tsEl("div", "ts-snap-muted", `${s.dataset} / ${s.records.join(", ")} / ${s.vars.length} ch / イベント ${s.events_count} / ${s.created_at}`);
  const linked = tsEl("div", "", s.links.length ? `結んだ先: ${s.links.map((l) => `${l.node}（${l.op}）`).join("、")}` : "結んだ先: なし");

  const node = document.createElement("input");
  node.type = "text";
  node.placeholder = "S3";
  node.size = 6;
  node.setAttribute("aria-label", "S か E の ID");
  const text = document.createElement("input");
  text.type = "text";
  text.value = `データセット ${s.name}`;
  text.setAttribute("aria-label", "E の文（S に結ぶとき）");
  const go = tsEl("button", "", "結ぶ");
  go.type = "button";
  go.onclick = () => tsSnapLink(s.name, node.value.trim(), text.value);
  const row = tsEl("div", "ts-snap-row");
  row.append(tsEl("span", "ts-snap-muted", "S か E の ID"), node, tsEl("span", "ts-snap-muted", "文"), text, go);

  const det = document.createElement("details");
  const sum = document.createElement("summary");
  sum.textContent = "来歴（ch ごとの出所と実行の記録）";
  det.append(sum);
  det.addEventListener("toggle", async () => {
    if (!det.open || det.dataset.done) return;
    det.dataset.done = "1";
    try {
      const m = await window.TS.api(`/api/snapshots/${tsEnc(s.name)}`);
      det.append(...m.manifest.vars.map((v) => tsEl("div", "ts-snap-muted", `${v.name}: ${tsSnapLineage(v)}`)));
    } catch (e) {
      det.append(tsEl("div", "ts-snap-muted", `読めません: ${e.message}`));
    }
  });
  item.append(head, meta, linked, row, det);
  return item;
}

async function tsSnapReload() {
  const list = tsSnap.list;
  if (!list) return;
  try {
    const body = await window.TS.api("/api/snapshots");
    const items = body.snapshots.map(tsSnapItem);
    list.replaceChildren(...(items.length ? items : [tsEl("p", "ts-snap-muted", "スナップショットはありません")]));
  } catch (e) {
    list.replaceChildren(tsEl("p", "ts-snap-muted", `一覧を読めません: ${e.message}`));
  }
}

function tsSnapRender(el) {
  tsSnapStyle();
  el.replaceChildren();
  const box = tsEl("div", "ts-snap-box");
  box.append(tsEl("b", "", "書き出す"));
  const name = document.createElement("input");
  name.type = "text";
  name.placeholder = "名前（例 hfo-r01-v1）";
  name.setAttribute("aria-label", "スナップショットの名前");
  const nameRow = tsEl("div", "ts-snap-row");
  nameRow.append(tsEl("span", "ts-snap-muted", "名前"), name);
  const opts = document.createElement("div");
  const go = tsEl("button", "", "書き出す");
  go.type = "button";
  go.onclick = tsSnapCreate;
  box.append(nameRow, opts, go);
  const list = document.createElement("div");
  el.append(box, list);
  Object.assign(tsSnap, { name, opts, list, key: "" });
  tsSnapSync(true);
  tsSnapReload();
}

function tsSnapBoot() {
  window.TS.ui.addSidebarTab({ id: "snapshots", title: "データセット", render: tsSnapRender });
  window.TS.on("snapshots", () => tsSnapReload());
  window.TS.on("state", () => tsSnapSync());
}

if (typeof window !== "undefined" && window.TS) {
  window.TS.on("boot", tsSnapBoot);
}

if (typeof module !== "undefined") {
  module.exports = { tsSnapDefaultName, tsSnapLineage, tsSnapNodeOk };
}
