"use strict";
// イベントの入力欄（共有契約 C4・C10）。TS.editEvent(id, draft) で選択の近くに小さな浮き窓を開く。
// 純関数（tsMetricInput・tsBuildEvent・tsPlacePanel・tsDraftFromEvent・tsNewDraft）は末尾で node のテストに渡す。

const tsNewCat = "__new__";

// schema のメトリクス 1 つから入力部品の種類と属性を決める
function tsMetricInput(def) {
  const base = { unit: def.unit || "", desc: def.desc || "" };
  if (def.type === "number" || def.type === "integer") {
    const o = { control: "number", step: def.type === "integer" ? 1 : "any", ...base };
    if (def.min !== undefined && def.min !== null) o.min = def.min;
    if (def.max !== undefined && def.max !== null) o.max = def.max;
    return o;
  }
  if (def.type === "enum") return { control: "select", values: (def.values || []).slice(), ...base };
  if (def.type === "boolean") return { control: "checkbox", ...base };
  return { control: "text", ...base };
}

// 入力値から送る JSON を作る。空の数値・文字のメトリクスは送らない（boolean は常に送る）
// form: {type, record, t, t0, t1, vars, category, text, metrics: {名前: 生の値}}
function tsBuildEvent(form, draft, schema) {
  const fail = (error) => ({ ok: false, error });
  const toNum = (raw) => {
    const s = raw === null || raw === undefined ? "" : String(raw).trim();
    return s === "" ? NaN : Number(s);
  };
  const record = form.record || draft.record;
  if (!record) return fail("記録が決まっていません");
  const type = form.type === "area" ? "area" : form.type === "line" ? "line" : null;
  if (!type) return fail("種類は ライン か エリア を選んでください");
  const json = { type, record, vars: Array.isArray(form.vars) ? form.vars.slice() : [] };
  if (type === "line") {
    const t = toNum(form.t);
    if (!Number.isFinite(t)) return fail("時刻（秒）を数値で入れてください");
    json.t = t;
  } else {
    const t0 = toNum(form.t0);
    const t1 = toNum(form.t1);
    if (!Number.isFinite(t0) || !Number.isFinite(t1)) return fail("開始と終了を数値で入れてください");
    if (t1 <= t0) return fail("終了は開始より後にしてください");
    json.t0 = t0;
    json.t1 = t1;
  }
  const category = String(form.category || "").trim();
  if (!category) return fail("カテゴリを選んでください");
  if (!(schema.categories || []).some((c) => c.name === category)) return fail(`カテゴリ「${category}」がありません`);
  json.category = category;
  json.text = String(form.text || "");
  const metrics = {};
  for (const def of schema.metrics || []) {
    const raw = form.metrics ? form.metrics[def.name] : undefined;
    if (def.type === "boolean") {
      metrics[def.name] = raw === true;
      continue;
    }
    const s = raw === null || raw === undefined ? "" : String(raw).trim();
    if (s === "") continue;
    if (def.type === "number" || def.type === "integer") {
      const v = Number(s);
      if (!Number.isFinite(v)) return fail(`${def.name}: 数値を入れてください`);
      if (def.type === "integer" && !Number.isInteger(v)) return fail(`${def.name}: 整数を入れてください`);
      if (def.min !== undefined && def.min !== null && v < def.min) return fail(`${def.name}: ${def.min} 以上にしてください`);
      if (def.max !== undefined && def.max !== null && v > def.max) return fail(`${def.name}: ${def.max} 以下にしてください`);
      metrics[def.name] = v;
    } else if (def.type === "enum") {
      if (!(def.values || []).includes(s)) return fail(`${def.name}: 選択肢にない値です`);
      metrics[def.name] = s;
    } else {
      metrics[def.name] = s;
    }
  }
  json.metrics = metrics;
  return { ok: true, json };
}

// 浮き窓を anchor の近くに置く。画面の外にはみ出さない
function tsPlacePanel(anchor, w, h, vw, vh) {
  if (!anchor) return { left: Math.max(8, vw - w - 16), top: 56 };
  let left = anchor.x + 12;
  let top = anchor.y + 12;
  if (left + w > vw - 8) left = anchor.x - w - 12;
  if (top + h > vh - 8) top = anchor.y - h - 12;
  return {
    left: Math.min(Math.max(8, left), Math.max(8, vw - w - 8)),
    top: Math.min(Math.max(8, top), Math.max(8, vh - h - 8)),
  };
}

function tsDraftFromEvent(ev) {
  return {
    type: ev.type,
    record: ev.record,
    t: ev.t,
    t0: ev.t0,
    t1: ev.t1,
    vars: (ev.vars || []).slice(),
    category: ev.category || "",
    text: ev.text || "",
    metrics: Object.assign({}, ev.metrics || {}),
  };
}

// 新規の下書き。draft（ドラッグ・Alt+クリックの結果）の値を使い、無いものは既定にする
function tsNewDraft(draft, record, schema) {
  const d = draft || {};
  const t = Number.isFinite(d.t) ? d.t : Number.isFinite(d.t0) ? d.t0 : 0;
  return {
    type: d.type === "area" ? "area" : "line",
    record,
    t,
    t0: Number.isFinite(d.t0) ? d.t0 : t,
    t1: Number.isFinite(d.t1) ? d.t1 : t,
    vars: Array.isArray(d.vars) ? d.vars.slice() : [],
    category: schema.categories.length ? schema.categories[0].name : "",
    text: "",
    metrics: {},
  };
}

// ---- ここから画面（ブラウザだけで動く） ----

function tsEvformInit() {
  window.TS.editEvent = tsEditEvent;
}

function tsCloseForm() {
  const el = document.getElementById("ts-form");
  if (el) el.remove();
}

function tsEditEvent(id, draft) {
  const st = window.TS.state;
  if (!st.dataset) {
    window.TS.ui.toast("データセットがありません");
    return;
  }
  let d;
  if (id) {
    const ev = st.events.find((e) => e.id === id);
    if (!ev) {
      window.TS.ui.toast("そのイベントが見つかりません");
      return;
    }
    d = tsDraftFromEvent(ev);
  } else {
    d = tsNewDraft(draft, st.record, st.schema);
  }
  tsOpenForm(id || null, d, (draft && draft.anchor) || null);
}

function tsField(label, control) {
  const f = tsEl("div", "ts-field");
  f.append(tsEl("div", "ts-lbl", label), control);
  return f;
}

function tsNumInput(v) {
  const i = tsEl("input");
  i.type = "number";
  i.step = "any";
  if (Number.isFinite(v)) i.value = String(v);
  return i;
}

// メトリクスの入力部品。値は保存された値（無ければ空）から入れる
function tsMetricControl(spec, value) {
  const has = value !== undefined && value !== null;
  if (spec.control === "number") {
    const i = tsNumInput(NaN);
    i.step = String(spec.step);
    if (spec.min !== undefined) i.min = String(spec.min);
    if (spec.max !== undefined) i.max = String(spec.max);
    if (has) i.value = String(value);
    return i;
  }
  if (spec.control === "select") {
    const s = tsEl("select");
    const empty = tsEl("option", "", "（未選択）");
    empty.value = "";
    s.appendChild(empty);
    spec.values.forEach((v) => {
      const o = tsEl("option", "", v);
      o.value = v;
      s.appendChild(o);
    });
    s.value = has ? String(value) : "";
    return s;
  }
  if (spec.control === "checkbox") {
    const c = tsEl("input");
    c.type = "checkbox";
    c.checked = value === true;
    return c;
  }
  const t = tsEl("input");
  t.type = "text";
  t.value = has ? String(value) : "";
  return t;
}

function tsOpenForm(id, d, anchor) {
  tsCloseForm();
  const st = window.TS.state;
  const schema = st.schema;
  const box = tsEl("div", "ts-form");
  box.id = "ts-form";
  box.setAttribute("role", "dialog");
  box.setAttribute("aria-label", id ? "イベントを編集" : "新しいイベント");
  box.appendChild(tsEl("div", "ts-form-title", id ? "イベントを編集" : "新しいイベント"));

  // 種類と時刻（種類で見せる欄を変える）
  const type = tsEl("select");
  [["line", "ライン（時刻 1 つ）"], ["area", "エリア（開始・終了）"]].forEach(([v, label]) => {
    const o = tsEl("option", "", label);
    o.value = v;
    type.appendChild(o);
  });
  type.value = d.type;
  box.appendChild(tsField("種類", type));
  const lineBox = tsEl("div");
  const tIn = tsNumInput(d.t);
  lineBox.appendChild(tsField("時刻（秒）", tIn));
  const areaBox = tsEl("div");
  const t0In = tsNumInput(d.t0);
  const t1In = tsNumInput(d.t1);
  areaBox.append(tsField("開始（秒）", t0In), tsField("終了（秒）", t1In));
  lineBox.hidden = d.type === "area";
  areaBox.hidden = d.type !== "area";
  box.append(lineBox, areaBox);
  type.onchange = () => {
    const area = type.value === "area";
    if (area) {
      if (t0In.value === "") t0In.value = tIn.value;
      if (t1In.value === "") t1In.value = tIn.value;
    } else if (tIn.value === "") {
      tIn.value = t0In.value;
    }
    lineBox.hidden = area;
    areaBox.hidden = !area;
  };

  // 変数（空 = 記録全体）
  const names = st.meta ? st.meta.vars.map((v) => v.name) : [];
  const varBox = tsEl("div", "ts-checks");
  const varBoxes = names.map((n) => {
    const lab = tsEl("label", "ts-chk");
    const cb = tsEl("input");
    cb.type = "checkbox";
    cb.value = n;
    cb.checked = d.vars.includes(n);
    lab.append(cb, " " + n);
    varBox.appendChild(lab);
    return cb;
  });
  box.appendChild(tsField("変数（選ばないと記録全体）", varBox));

  // カテゴリ（最後の「新しいカテゴリ」で名前と色を足す）
  const cats = schema.categories.map((c) => c.name);
  const cat = tsEl("select");
  const catNames = cats.includes(d.category) || !d.category ? cats : [d.category, ...cats];
  catNames.forEach((n) => {
    const o = tsEl("option", "", n);
    o.value = n;
    cat.appendChild(o);
  });
  const newOpt = tsEl("option", "", "＋ 新しいカテゴリ");
  newOpt.value = tsNewCat;
  cat.appendChild(newOpt);
  cat.value = d.category || (cats[0] || tsNewCat);
  const newBox = tsEl("div", "ts-newcat");
  const newName = tsEl("input");
  newName.type = "text";
  newName.placeholder = "名前（例 noise）";
  const newColor = tsEl("input");
  newColor.type = "color";
  newColor.value = "#d9534f";
  newBox.append(tsField("新しいカテゴリの名前", newName), tsField("色", newColor));
  newBox.hidden = cat.value !== tsNewCat;
  cat.onchange = () => {
    newBox.hidden = cat.value !== tsNewCat;
  };
  box.append(tsField("カテゴリ", cat), newBox);

  const textEl = tsEl("textarea");
  textEl.rows = 2;
  textEl.value = d.text;
  box.appendChild(tsField("テキスト", textEl));

  // メトリクス（schema から作る）
  const metrics = {};
  schema.metrics.forEach((def) => {
    const spec = tsMetricInput(def);
    const el = tsMetricControl(spec, d.metrics[def.name]);
    metrics[def.name] = el;
    box.appendChild(tsField(def.name + (spec.unit ? ` (${spec.unit})` : ""), el));
    if (spec.desc) box.appendChild(tsEl("div", "ts-desc", spec.desc));
  });

  const err = tsEl("div", "ts-err");
  err.setAttribute("role", "alert");
  err.hidden = true;
  box.appendChild(err);
  const showErr = (msg) => {
    err.textContent = msg;
    err.hidden = false;
  };

  // 画面の入力を生の値の形にする
  const read = () => ({
    type: type.value,
    record: d.record || st.record,
    t: tIn.value,
    t0: t0In.value,
    t1: t1In.value,
    vars: varBoxes.filter((b) => b.checked).map((b) => b.value),
    category: cat.value,
    text: textEl.value,
    metrics: Object.fromEntries(
      schema.metrics.map((def) => [def.name, def.type === "boolean" ? metrics[def.name].checked : metrics[def.name].value]),
    ),
  });

  const ds = st.dataset;
  const submit = async () => {
    const form = read();
    let s = schema;
    const isNew = form.category === tsNewCat;
    if (isNew) {
      const name = newName.value.trim();
      if (!tsValidName(name)) return showErr("カテゴリ名は英数字で始め、英数字と _ . - だけ（64 字まで）");
      if (schema.categories.some((c) => c.name === name)) return showErr("同じ名前のカテゴリがあります");
      s = Object.assign({}, schema, {
        categories: schema.categories.concat([{ name, color: newColor.value, visible: true, desc: "" }]),
      });
      form.category = name;
    }
    const r = tsBuildEvent(form, d, s);
    if (!r.ok) return showErr(r.error);
    save.disabled = true;
    try {
      if (isNew) await window.TS.api(`/api/datasets/${tsEnc(ds)}/schema`, { method: "PUT", body: s });
      let row;
      if (id) {
        const body = Object.assign({}, r.json);
        delete body.record; // 記録は動かさない
        row = await window.TS.api(`/api/datasets/${tsEnc(ds)}/events/${tsEnc(id)}`, { method: "PATCH", body });
      } else {
        row = await window.TS.api(`/api/datasets/${tsEnc(ds)}/events`, { method: "POST", body: r.json });
      }
      tsCloseForm();
      if (isNew) await window.TS.reloadSchema();
      await window.TS.reloadEvents();
      if (row && row.id) tsSelectEvents([row.id]);
    } catch (e) {
      save.disabled = false;
      showErr(e.message);
    }
  };

  const remove = async () => {
    if (!window.confirm("このイベントを消しますか？")) return;
    try {
      await window.TS.api(`/api/datasets/${tsEnc(ds)}/events/${tsEnc(id)}`, { method: "DELETE" });
      tsCloseForm();
      await window.TS.reloadEvents();
      window.TS.setView({ selectedEvents: [] });
    } catch (e) {
      showErr(e.message);
    }
  };

  const bar = tsEl("div", "ts-form-bar");
  const save = tsEl("button", "ts-primary", "保存");
  save.type = "button";
  save.onclick = submit;
  bar.appendChild(save);
  if (id) {
    const del = tsEl("button", "ts-danger", "削除");
    del.type = "button";
    del.onclick = remove;
    bar.appendChild(del);
  }
  const cancel = tsEl("button", "", "取り消し");
  cancel.type = "button";
  // 取り消しても選択は残す（Escape も同じ）
  cancel.onclick = tsCloseForm;
  bar.appendChild(cancel);
  box.appendChild(bar);

  // 窓の中のキーは外へ流さない（Escape で選択を外さない・入力中に画面の操作をしない）
  box.addEventListener("keydown", (e) => {
    e.stopPropagation();
    if (e.key === "Escape") tsCloseForm();
    else if (e.key === "Enter" && (e.ctrlKey || e.metaKey)) submit();
  });

  document.body.appendChild(box);
  const rc = box.getBoundingClientRect();
  const pos = tsPlacePanel(anchor, rc.width, rc.height, window.innerWidth, window.innerHeight);
  box.style.left = pos.left + "px";
  box.style.top = pos.top + "px";
  (d.type === "area" ? t0In : tIn).focus();
}

if (typeof module !== "undefined") {
  module.exports = { tsMetricInput, tsBuildEvent, tsPlacePanel, tsDraftFromEvent, tsNewDraft };
}
