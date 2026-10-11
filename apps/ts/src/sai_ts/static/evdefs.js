"use strict";
// イベント定義の欄（U05）。イベントのタブの上に置く（evtable.js の tsEvtRender が呼ぶ）。
// 定義（schema の categories の 1 件・G1-C2）を作る・直す・消す・表示を切り替える・作成に使う定義を選ぶ。

let tsEdefBox = null; // 欄の箱（evtable.js が作る）
let tsEdefSig = null; // 最後に描いた内容の印（同じなら作り直さない）
let tsEdefOpen = null; // 編集の欄 {ds, name（新しい定義は null）, draft, count, err}。開いていないなら null

// ---- 純関数（DOM を使わない。node のテストが読む） ----

// 定義の検査（G1-C2）。others は同じ dataset の他の定義。返り値はエラーの文か ""
function tsEvdefCheck(def, others) {
  if (!tsValidName(def.name)) return "名前は英字か数字で始め、英数字と _ . - だけの 64 字以内にしてください";
  if ((others || []).some((o) => o.name === def.name)) return `同じ名前の定義があります: ${def.name}`;
  if (!/^#[0-9A-Fa-f]{6}$/.test(def.color || "")) return "色は #RRGGBB の形にしてください";
  if (def.type && !["area", "line"].includes(def.type)) return "種類は エリア・ポイント・自由 のどれかにしてください";
  if (def.ch && !["single", "all"].includes(def.ch)) return "1ch・全ch は 1ch・全ch・自由 のどれかにしてください";
  return "";
}

// 表示の文（「エリア・1ch」「ポイント・全ch」「自由」）。app.js の tsEvdefLabel（ヘッダの選び）とは別の名前
function tsEvdefKindText(def) {
  const parts = [];
  if (def.type === "area") parts.push("エリア");
  else if (def.type === "line") parts.push("ポイント");
  if (def.ch === "single") parts.push("1ch");
  else if (def.ch === "all") parts.push("全ch");
  return parts.length ? parts.join("・") : "自由";
}

// ---- 画面（ブラウザだけで動く） ----

function tsEdefStyle() {
  if (document.getElementById("ts-style-evdefs.js")) return;
  const st = document.createElement("style");
  st.id = "ts-style-evdefs.js";
  st.textContent = `
.ts-edef-head { display: flex; align-items: center; gap: 6px; margin-bottom: 4px; }
.ts-edef-head b { flex: 1; }
.ts-edef-row { display: flex; align-items: center; gap: 4px; padding: 3px 0; border-bottom: 1px solid var(--ts-border); }
.ts-edef-row .ts-edef-name { font-weight: 600; }
.ts-edef-row .ts-edef-kind { color: var(--ts-muted); font-size: 12px; flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.ts-edef-row button { border: 0; background: none; color: var(--ts-text); cursor: pointer; padding: 0 2px; }
.ts-edef-row button.off { opacity: 0.35; }
.ts-edef-dot { width: 10px; height: 10px; border-radius: 50%; flex: none; }
.ts-edef-edit { display: grid; gap: 6px; padding: 6px; margin: 4px 0; border: 1px solid var(--ts-border); border-radius: 6px; background: var(--ts-hover); }
.ts-edef-edit label { display: flex; align-items: center; gap: 6px; }
.ts-edef-edit label > span { width: 52px; flex: none; color: var(--ts-muted); }
.ts-edef-edit input[type=text], .ts-edef-edit select { flex: 1; min-width: 0; }
.ts-edef-edit .ts-edef-btns { display: flex; gap: 6px; justify-content: flex-end; }
.ts-edef-small { color: var(--ts-muted); font-size: 12px; margin: 0; }
.ts-edef-err { color: var(--ts-danger); font-size: 12px; margin: 0; }
`;
  document.head.appendChild(st);
}

function tsEdefBtn(label, title, onClick) {
  const b = tsEl("button", "", label);
  b.type = "button";
  b.title = title || label;
  b.onclick = onClick;
  return b;
}

// 保存（schema の PUT は全体を置き換える）。失敗は Error で返す（detail をそのまま使う）
async function tsEdefPut(categories) {
  const st = TS.state;
  await TS.api(`/api/datasets/${tsEnc(st.dataset)}/schema`, {
    method: "PUT",
    body: { version: 1, categories, metrics: st.schema.metrics },
  });
  await TS.reloadSchema();
  tsEdefOpen = null;
  tsEdefDraw(true);
}

// その定義を使うイベントの件数（全記録）。編集の欄を開いたときに 1 回だけ数える
async function tsEdefCount(ds, name) {
  const r = await TS.api(`/api/datasets/${tsEnc(ds)}/events`).catch(() => ({ events: [] }));
  return (r.events || []).filter((e) => e.category === name).length;
}

function tsEdefOpenFor(cat) {
  const st = TS.state;
  const ds = st.dataset;
  tsEdefOpen = cat
    ? {
        ds,
        name: cat.name,
        draft: { name: cat.name, color: cat.color, desc: cat.desc || "", type: cat.type || "", ch: cat.ch || "" },
        count: null,
        err: "",
      }
    : {
        ds,
        name: null,
        draft: { name: "", color: "#f59e0b", desc: "", type: "area", ch: "single" },
        count: 0,
        err: "",
      };
  if (cat) tsEdefCount(ds, cat.name).then((n) => {
    if (tsEdefOpen && tsEdefOpen.ds === ds && tsEdefOpen.name === cat.name) {
      tsEdefOpen.count = n;
      tsEdefDraw(true);
    }
  });
  tsEdefDraw(true);
}

// 保存する定義を作る。元の定義の他の項目（visible など）は残す。自由は項目を消す
function tsEdefDefFromDraft(d, orig) {
  const def = Object.assign({ visible: true }, orig || {}, { name: d.name.trim(), color: d.color, desc: d.desc });
  if (d.type) def.type = d.type;
  else delete def.type;
  if (d.ch) def.ch = d.ch;
  else delete def.ch;
  return def;
}

async function tsEdefSubmit(orig) {
  const st = TS.state;
  const cats = st.schema.categories;
  const def = tsEdefDefFromDraft(tsEdefOpen.draft, orig);
  const others = cats.filter((c) => !orig || c.name !== orig.name);
  const err = tsEvdefCheck(def, others);
  if (err) {
    tsEdefOpen.err = err;
    tsEdefDraw(true);
    return;
  }
  const next = orig ? cats.map((c) => (orig && c.name === orig.name ? def : c)) : [...cats, def];
  try {
    await tsEdefPut(next);
  } catch (e) {
    // サーバの理由（使われているカテゴリの消去・名前の変更など）をそのまま出す
    if (tsEdefOpen) {
      tsEdefOpen.err = e.message;
      tsEdefDraw(true);
    }
  }
}

async function tsEdefRemove(cat) {
  if (!confirm(`定義 ${cat.name} を消します。使っているイベントがあれば消せません。よいですか？`)) return;
  try {
    await tsEdefPut(TS.state.schema.categories.filter((c) => c.name !== cat.name));
  } catch (e) {
    TS.ui.toast(`消せません: ${e.message}`);
  }
}

function tsEdefToggleVisible(name) {
  const next = new Set(TS.state.hiddenCategories);
  if (next.has(name)) next.delete(name);
  else next.add(name);
  TS.setView({ hiddenCategories: [...next] });
}

function tsEdefRow(c) {
  const st = TS.state;
  const hidden = st.hiddenCategories.has(c.name);
  const row = tsEl("div", "ts-edef-row");

  const pick = document.createElement("input");
  pick.type = "radio";
  pick.name = "ts-edef-pick";
  pick.checked = st.eventDef === c.name;
  pick.title = "この定義で作成する";
  pick.setAttribute("aria-label", `${c.name} で作成する`);
  pick.onchange = () => TS.setEventDef(c.name);

  const dot = tsEl("span", "ts-edef-dot");
  dot.style.background = c.color;
  const name = tsEl("span", "ts-edef-name", c.name);
  const kind = tsEl("span", "ts-edef-kind", tsEvdefKindText(c));
  kind.title = c.desc || tsEvdefKindText(c);

  const eye = tsEdefBtn("👁", hidden ? "表示する" : "隠す", () => tsEdefToggleVisible(c.name));
  eye.className = hidden ? "off" : "";
  eye.setAttribute("aria-pressed", String(!hidden));
  const edit = tsEdefBtn("✎", "この定義を直す", () => tsEdefOpenFor(c));
  const del = tsEdefBtn("🗑", "この定義を消す", () => tsEdefRemove(c));

  row.append(pick, dot, name, kind, eye, edit, del);
  return row;
}

// 編集の欄（新しい定義か既存の定義）。入力は draft に写すので、描き直しても打ちかけは残る
function tsEdefEditor(orig) {
  const d = tsEdefOpen.draft;
  const box = tsEl("div", "ts-edef-edit");

  const field = (label, el) => {
    const l = document.createElement("label");
    l.append(tsEl("span", "", label), el);
    return l;
  };
  const text = (key, placeholder) => {
    const i = document.createElement("input");
    i.type = "text";
    i.value = d[key];
    i.placeholder = placeholder || "";
    i.oninput = () => (d[key] = i.value);
    return i;
  };
  const choose = (key, opts) => {
    const s = document.createElement("select");
    for (const [v, t] of opts) {
      const o = tsEl("option", "", t);
      o.value = v;
      s.appendChild(o);
    }
    s.value = d[key];
    s.onchange = () => (d[key] = s.value);
    return s;
  };

  const name = text("name", "英字で始める（例 HFO）");
  const color = document.createElement("input");
  color.type = "color";
  color.value = d.color;
  color.oninput = () => (d.color = color.value);
  const desc = text("desc", "説明（任意）");

  box.append(
    field("name", name),
    field("color", color),
    field("種類", choose("type", [["", "自由"], ["area", "エリア"], ["line", "ポイント"]])),
    field("1ch・全ch", choose("ch", [["", "自由"], ["single", "1ch"], ["all", "全ch"]])),
    field("説明", desc),
  );

  if (orig) {
    box.append(
      tsEl("p", "ts-edef-small", `既存の ${tsEdefOpen.count ?? "…"} 件はそのまま（種類と 1ch・全ch は、今のイベントを直しません）`),
      tsEl("p", "ts-edef-small", "名前を変えると、使っているイベントがあれば保存できません。名前の変更の代わりに、新しい定義を足してください。"),
    );
  }
  if (tsEdefOpen.err) box.append(tsEl("p", "ts-edef-err", tsEdefOpen.err));

  const btns = tsEl("div", "ts-edef-btns");
  btns.append(
    tsEdefBtn("保存", "定義を保存する", () => tsEdefSubmit(orig)),
    tsEdefBtn("取り消し", "編集をやめる", () => {
      tsEdefOpen = null;
      tsEdefDraw(true);
    }),
  );
  box.append(btns);
  return box;
}

function tsEdefList() {
  const st = TS.state;
  const list = tsEl("div", "ts-edef-list");
  const cats = st.schema.categories;
  const none = document.createElement("div");
  none.className = "ts-edef-row";
  const nonePick = document.createElement("input");
  nonePick.type = "radio";
  nonePick.name = "ts-edef-pick";
  nonePick.checked = !st.eventDef;
  nonePick.title = "定義なしで作成する（作るのは定義を選んだとき）";
  nonePick.setAttribute("aria-label", "定義なし");
  nonePick.onchange = () => TS.setEventDef(null);
  none.append(nonePick, tsEl("span", "ts-edef-kind", "定義なし"));
  list.append(none);

  for (const c of cats) {
    list.append(tsEdefRow(c));
    if (tsEdefOpen && tsEdefOpen.name === c.name) list.append(tsEdefEditor(c));
  }
  if (!cats.length) list.append(tsEl("p", "ts-edef-small", "定義がありません。「＋ 定義」で足します"));
  if (tsEdefOpen && tsEdefOpen.name === null) list.append(tsEdefEditor(null));
  return list;
}

// 変化があったときだけ作り直す。入力の途中は作り直さない（離れたら描く）
function tsEdefDraw(force) {
  if (!tsEdefBox || !tsEdefBox.isConnected) return;
  const st = TS.state;
  if (tsEdefOpen && tsEdefOpen.ds !== st.dataset) tsEdefOpen = null;
  const sig = JSON.stringify([st.dataset, st.schema.categories, st.eventDef, [...st.hiddenCategories], tsEdefOpen]);
  if (!force && (sig === tsEdefSig || tsEdefTyping())) return;
  tsEdefSig = sig;
  const head = tsEl("div", "ts-edef-head");
  head.append(tsEl("b", "", "イベント定義"), tsEdefBtn("＋ 定義", "定義を足す", () => tsEdefOpenFor(null)));
  tsEdefBox.replaceChildren(head, tsEdefList());
}

function tsEdefTyping() {
  const a = document.activeElement;
  return !!a && !!tsEdefBox && tsEdefBox.contains(a) && a.tagName === "INPUT" && a.type === "text";
}

// イベントのタブが描くときに呼ぶ（evtable.js の tsEvtRender）
function tsEvdefRender(el) {
  tsEdefStyle();
  el.replaceChildren();
  tsEdefBox = el;
  tsEdefSig = null;
  tsEdefOpen = null;
  el.addEventListener("focusout", () => setTimeout(() => tsEdefDraw(false), 0));
  tsEdefDraw(true);
}

function tsEvdefBoot() {
  TS.on("schema", () => tsEdefDraw(false));
  TS.on("mode", () => tsEdefDraw(false));
  TS.on("state", () => tsEdefDraw(false));
}

if (typeof window !== "undefined" && window.TS) {
  window.TS.on("boot", tsEvdefBoot);
}

if (typeof module !== "undefined") {
  module.exports = { tsEvdefCheck, tsEvdefLabel: tsEvdefKindText };
}
