"use strict";
// ch 設定のタブ（U05）。sidebar の「ch」タブ。ch の並び・表示・label・色・倍率を変えて channels.json と view.json に残す。
// 波形の描き方は lanes.js。ここは値を保存して TS.state を変えるだけ。

const tsChSaveDelay = 300; // 続けて変えたときは、この ms まとめて 1 回に保存する
let tsChPending = null; // 保存を待つ変更 {patch: {name: {label?, color?, gain?, spectrum?}}, order: [名前]|null}
let tsChTimer = null;
let tsChBusy = Promise.resolve(); // 保存を 1 つずつ行う（読んでから書く間に別の保存が割り込まないため）
let tsChBox = null; // タブの中身の箱
let tsChSig = null; // 最後に描いた内容の印（同じなら作り直さない）
let tsChDragFrom = -1; // ドラッグ中の行の番号（-1 は無し）

// ---- 純関数（DOM を使わない。node のテストが読む） ----

// 並べ替え: names の from 番目を to 番目へ動かした並び（範囲外・同じ位置は動かさない）
function tsChMove(names, from, to) {
  const out = names.slice();
  if (from === to || from < 0 || to < 0 || from >= out.length || to >= out.length) return out;
  const [x] = out.splice(from, 1);
  out.splice(to, 0, x);
  return out;
}

// 保存する配列（G1-C1）。今の記録の ch を order の順（無ければ merged の順）に並べ、patch を当てる。
// stored にあって今の記録に無い ch は、後ろへそのまま残す（記録に無いからといって設定を消さない）
function tsChNext(stored, merged, patch, order) {
  const byName = new Map(merged.map((c) => [c.name, c]));
  const names = [];
  const seen = new Set();
  for (const n of Array.isArray(order) ? order : []) {
    if (byName.has(n) && !seen.has(n)) {
      names.push(n);
      seen.add(n);
    }
  }
  for (const c of merged) {
    if (seen.has(c.name)) continue;
    names.push(c.name);
    seen.add(c.name);
  }
  const out = names.map((n) => {
    const c = byName.get(n);
    const p = (patch && patch[n]) || {};
    const o = {
      name: n,
      label: "label" in p ? p.label : c.label,
      color: "color" in p ? p.color : c.color,
      gain: "gain" in p ? p.gain : c.gain,
    };
    const sp = "spectrum" in p ? p.spectrum : c.spectrum;
    if (sp) o.spectrum = sp;
    return o;
  });
  for (const s of Array.isArray(stored) ? stored : []) {
    if (byName.has(s.name) || seen.has(s.name)) continue;
    seen.add(s.name);
    out.push(Object.assign({}, s));
  }
  return out;
}

// スペクトルの値の検査（channels.py の _spectrum と同じ）。誤りの文、正しければ空の文字
function tsChSpecErr(sp) {
  if (!(sp.fmin >= 0)) return "fmin は 0 以上の数にしてください";
  if (!(sp.fmax > sp.fmin)) return "fmax は fmin より大きい数にしてください";
  if (!(sp.win_ms > 0 && sp.win_ms <= 10000)) return "窓は 0 より大きく 10000 ms 以下にしてください";
  return "";
}

// ---- 画面（ブラウザだけで動く） ----

function tsChStyle() {
  if (document.getElementById("ts-style-chpanel.js")) return;
  const st = document.createElement("style");
  st.id = "ts-style-chpanel.js";
  st.textContent = `
.ts-ch-head { display: flex; flex-wrap: wrap; gap: 6px; align-items: center; margin-bottom: 6px; }
.ts-ch-row { display: flex; flex-wrap: wrap; gap: 4px 6px; align-items: center; padding: 4px 2px; border-bottom: 1px solid var(--ts-border); }
.ts-ch-row:focus-visible { outline: 2px solid var(--ts-accent); }
.ts-ch-drop-before { box-shadow: inset 0 2px 0 var(--ts-accent); }
.ts-ch-drop-after { box-shadow: inset 0 -2px 0 var(--ts-accent); }
.ts-ch-grip { cursor: grab; color: var(--ts-muted); user-select: none; }
.ts-ch-name { min-width: 52px; font-weight: 600; }
.ts-ch-label { flex: 1 1 80px; min-width: 60px; }
.ts-ch-gain { width: 64px; }
.ts-ch-color { width: 28px; height: 24px; padding: 0; border: 1px solid var(--ts-border); background: none; cursor: pointer; }
.ts-ch-unit, .ts-ch-tag { color: var(--ts-muted); font-size: 12px; }
.ts-ch-tag { border: 1px solid var(--ts-border); border-radius: 3px; padding: 0 4px; }
.ts-ch-empty { color: var(--ts-muted); }
.ts-ch-spec-btn[aria-pressed="true"] { border-color: var(--ts-accent); color: var(--ts-accent); }
.ts-ch-spec { flex: 1 1 100%; display: flex; flex-wrap: wrap; gap: 4px 12px; padding-left: 24px; color: var(--ts-muted); font-size: 12px; }
.ts-ch-spec input { width: 84px; }
`;
  document.head.appendChild(st);
}

function tsChBtn(label, title, onClick) {
  const b = tsEl("button", "", label);
  b.type = "button";
  b.title = title || label;
  b.onclick = onClick;
  return b;
}

// 変えた値を覚え、300ms 後にまとめて保存する（保存は 1 つずつ直列に行う）
function tsChQueue(patch, order) {
  if (!tsChPending) tsChPending = { patch: {}, order: null };
  for (const [n, d] of Object.entries(patch)) tsChPending.patch[n] = Object.assign({}, tsChPending.patch[n], d);
  if (order) tsChPending.order = order;
  clearTimeout(tsChTimer);
  tsChTimer = setTimeout(() => {
    tsChTimer = null;
    tsChBusy = tsChBusy.then(tsChFlush).catch((e) => window.TS.ui.toast("ch 設定を保存できません: " + e.message));
  }, tsChSaveDelay);
}

// 覚えた変更を、いまの保存（GET）と合わせて PUT する（G1-C1）
async function tsChFlush() {
  const p = tsChPending;
  tsChPending = null;
  const ds = window.TS.state.dataset;
  if (!p || !ds) return;
  const cur = await window.TS.api(`/api/datasets/${tsEnc(ds)}/channels`).catch((e) => {
    window.TS.ui.toast("ch 設定を読めません: " + e.message);
    return null;
  });
  if (!cur) return;
  await window.TS.saveChannels(tsChNext(cur.channels, window.TS.state.channels, p.patch, p.order));
}

// ch の表示と非表示（view.json の vars）。並びは channels の順に合わせる
function tsChToggle(name, on) {
  const st = window.TS.state;
  const cur = new Set(Array.isArray(st.vars) ? st.vars : st.channels.map((c) => c.name));
  if (on) cur.add(name);
  else cur.delete(name);
  window.TS.setView({ vars: tsOrderVisible(st.channels, [...cur]) });
}

// 行を動かす。画面の順番は先に変え（保存を待たない）、channels と view.json の vars を保存する
function tsChReorder(from, to, focusName) {
  const st = window.TS.state;
  const names = st.channels.map((c) => c.name);
  const next = tsChMove(names, from, to);
  if (next.join("\n") === names.join("\n")) return;
  const byName = new Map(st.channels.map((c) => [c.name, c]));
  st.channels = next.map((n) => byName.get(n));
  window.TS.setView({ vars: tsOrderVisible(st.channels, st.vars) });
  tsChQueue({}, next);
  window.TS.emit("channels");
  if (focusName) tsChFocusRow(focusName);
}

function tsChFocusRow(name) {
  if (!tsChBox) return;
  const row = [...tsChBox.querySelectorAll(".ts-ch-row")].find((r) => r.dataset.name === name);
  if (row) row.focus();
}

// 入力の途中（文字や数の欄にフォーカスがある）は書き換えない
function tsChEditing() {
  const a = document.activeElement;
  return !!a && !!tsChBox && tsChBox.contains(a) && a.tagName === "INPUT" && (a.type === "text" || a.type === "number");
}

// スペクトルの段を入れる・切る・値を変える。入れたときに spectrum が無ければ既定で作る。
// 切っても値は残す（on だけ false）。画面は先に変え、保存は tsChQueue が行う
function tsChSpecSet(name, patch) {
  const st = window.TS.state;
  const cur = st.channels.find((c) => c.name === name);
  const base = cur.spectrum || { on: false, fmin: 0, fmax: Math.min(250, st.meta.srate / 2), win_ms: 250 };
  const sp = Object.assign({}, base, patch);
  if (sp.on && tsChSpecErr(sp)) {
    window.TS.ui.toast(tsChSpecErr(sp));
    return;
  }
  st.channels = st.channels.map((c) => (c.name === name ? Object.assign({}, c, { spectrum: sp }) : c));
  tsChQueue({ [name]: { spectrum: sp } });
  window.TS.emit("channels");
}

// 行の下の 3 つの欄（fmin・fmax・win_ms）。変えたら検査して保存する
function tsChSpecBox(c) {
  const sp = c.spectrum;
  const box = tsEl("div", "ts-ch-spec");
  const field = (key, text, min) => {
    const inp = document.createElement("input");
    inp.type = "number";
    inp.step = "any";
    inp.min = String(min);
    inp.value = String(sp[key]);
    inp.setAttribute("aria-label", `${c.name} のスペクトル ${key}`);
    inp.onchange = () => {
      const v = inp.value.trim() === "" ? NaN : Number(inp.value);
      const err = tsChSpecErr(Object.assign({}, sp, { [key]: v }));
      if (err) {
        window.TS.ui.toast(err);
        inp.value = String(sp[key]);
        return;
      }
      tsChSpecSet(c.name, { [key]: v });
    };
    const lab = tsEl("label", "", text);
    lab.append(inp);
    return lab;
  };
  box.append(field("fmin", "fmin（Hz）", 0), field("fmax", "fmax（Hz）", 0), field("win_ms", "win_ms（ms）", 0));
  return box;
}

function tsChRowEl(c, i) {
  const st = window.TS.state;
  const vis = !Array.isArray(st.vars) || st.vars.includes(c.name);
  const derived = (st.meta ? st.meta.vars : []).some((v) => v.name === c.name && v.source === "derived");

  const row = tsEl("div", "ts-ch-row");
  row.draggable = true;
  row.tabIndex = 0;
  row.dataset.name = c.name;
  row.dataset.i = String(i);

  const grip = tsEl("span", "ts-ch-grip", "⋮⋮");
  grip.title = "ドラッグで並べ替え（Alt＋↑／↓ でも動く）";

  const shown = document.createElement("input");
  shown.type = "checkbox";
  shown.checked = vis;
  shown.title = "表示";
  shown.setAttribute("aria-label", `${c.name} を表示`);
  shown.onchange = () => tsChToggle(c.name, shown.checked);

  const color = document.createElement("input");
  color.type = "color";
  color.className = "ts-ch-color";
  color.value = c.drawColor;
  color.title = "色";
  color.setAttribute("aria-label", `${c.name} の色`);
  color.onchange = () => tsChQueue({ [c.name]: { color: color.value } });

  const reset = tsChBtn("↺", "色を既定の色に戻す", () => tsChQueue({ [c.name]: { color: "" } }));

  const name = tsEl("span", "ts-ch-name", c.name);

  const label = document.createElement("input");
  label.type = "text";
  label.maxLength = 64;
  label.className = "ts-ch-label";
  label.value = c.label;
  label.placeholder = c.name;
  label.title = "表示名（空なら変数の名前）";
  label.setAttribute("aria-label", `${c.name} の表示名`);
  label.onchange = () => tsChQueue({ [c.name]: { label: label.value.trim() } });

  const gain = document.createElement("input");
  gain.type = "number";
  gain.step = "any";
  gain.min = "0.01";
  gain.max = "1000";
  gain.className = "ts-ch-gain";
  gain.value = String(c.gain);
  gain.title = "倍率（全体の倍率と掛ける）";
  gain.setAttribute("aria-label", `${c.name} の倍率`);
  gain.onchange = () => {
    const v = Number(gain.value);
    if (!(v > 0 && v <= 1000)) {
      window.TS.ui.toast("倍率は 0 より大きく 1000 以下の数にしてください");
      gain.value = String(c.gain);
      return;
    }
    tsChQueue({ [c.name]: { gain: v } });
  };

  row.append(grip, shown, color, reset, name, label, gain);
  if (c.unit) row.append(tsEl("span", "ts-ch-unit", c.unit));
  if (derived) row.append(tsEl("span", "ts-ch-tag", "派生"));

  const on = !!(c.spectrum && c.spectrum.on);
  const spBtn = tsChBtn("〰 スペクトル", "時間周波数の図を、この ch の波形の下に出す（切っても値は残る）", () =>
    tsChSpecSet(c.name, { on: !on }),
  );
  spBtn.className = "ts-ch-spec-btn";
  spBtn.setAttribute("aria-pressed", String(on));
  row.append(spBtn);
  if (on) row.append(tsChSpecBox(c));

  row.addEventListener("dragstart", (e) => {
    tsChDragFrom = i;
    e.dataTransfer.effectAllowed = "move";
    e.dataTransfer.setData("text/plain", c.name);
  });
  row.addEventListener("dragover", (e) => {
    if (tsChDragFrom < 0) return;
    e.preventDefault();
    const r = row.getBoundingClientRect();
    tsChMark(row, e.clientY > r.top + r.height / 2);
  });
  row.addEventListener("drop", (e) => {
    if (tsChDragFrom < 0) return;
    e.preventDefault();
    const r = row.getBoundingClientRect();
    const at = i + (e.clientY > r.top + r.height / 2 ? 1 : 0); // 落とす位置（この行の前か後ろ）
    const from = tsChDragFrom;
    tsChDragFrom = -1;
    tsChReorder(from, from < at ? at - 1 : at, c.name);
  });
  row.addEventListener("dragend", () => {
    tsChDragFrom = -1;
    tsChMark(null);
  });
  row.addEventListener("keydown", (e) => {
    if (e.target !== row || !e.altKey || (e.key !== "ArrowUp" && e.key !== "ArrowDown")) return;
    e.preventDefault();
    const to = e.key === "ArrowUp" ? i - 1 : i + 1;
    tsChReorder(i, to, c.name);
  });
  return row;
}

// 落とす位置の線（2px）。row が null なら線を消す
function tsChMark(row, after) {
  if (!tsChBox) return;
  for (const r of tsChBox.querySelectorAll(".ts-ch-row")) r.classList.remove("ts-ch-drop-before", "ts-ch-drop-after");
  if (row) row.classList.add(after ? "ts-ch-drop-after" : "ts-ch-drop-before");
}

function tsChHead() {
  const st = window.TS.state;
  const head = tsEl("div", "ts-ch-head");
  head.append(
    tsChBtn("全部表示", "全部の ch を表示", () => window.TS.setView({ vars: st.channels.map((c) => c.name) })),
    tsChBtn("全部隠す", "全部の ch を隠す", () => window.TS.setView({ vars: [] })),
  );
  if (typeof window.TS.ui.openAlgos === "function") {
    const visible = st.channels.map((c) => c.name).filter((n) => !Array.isArray(st.vars) || st.vars.includes(n));
    head.append(
      tsEl("span", "ts-ch-spacer"),
      tsChBtn("＋ ch を足す", "アルゴリズムで派生の ch を作る", () =>
        window.TS.ui.openAlgos({ kind: "channel", vars: visible }),
      ),
    );
  }
  return head;
}

function tsChList() {
  const st = window.TS.state;
  const list = tsEl("div", "ts-ch-list");
  if (!st.dataset) list.append(tsEl("p", "ts-ch-empty", "データセットがありません"));
  else if (!st.meta) list.append(tsEl("p", "ts-ch-empty", "記録がありません"));
  else if (!st.channels.length) list.append(tsEl("p", "ts-ch-empty", "ch がありません"));
  else st.channels.forEach((c, i) => list.append(tsChRowEl(c, i)));
  return list;
}

// 変化があったときだけ作り直す。入力の途中・ドラッグの途中は作り直さない（終わったら描く）
function tsChDraw(force) {
  if (!tsChBox || !tsChBox.isConnected) return;
  const st = window.TS.state;
  const sig = JSON.stringify([st.dataset, st.record, st.channels, st.vars, st.meta ? st.meta.vars : null]);
  if (!force && (sig === tsChSig || tsChDragFrom >= 0 || tsChEditing())) return;
  tsChSig = sig;
  tsChBox.replaceChildren(tsChHead(), tsChList());
}

function tsChRender(el) {
  tsChStyle();
  el.replaceChildren();
  tsChBox = el;
  tsChSig = null;
  el.addEventListener("focusout", () => setTimeout(() => tsChDraw(false), 0));
  tsChDraw(true);
}

function tsChBoot() {
  window.TS.ui.addSidebarTab({ id: "channels", title: "ch", render: tsChRender });
  window.TS.on("channels", () => tsChDraw(false));
  window.TS.on("state", () => tsChDraw(false));
}

if (typeof window !== "undefined" && window.TS) {
  window.TS.on("boot", tsChBoot);
}

if (typeof module !== "undefined") {
  module.exports = { tsChMove, tsChNext, tsChSpecErr };
}
