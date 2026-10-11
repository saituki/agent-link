"use strict";
// 患者タブ（sidebar の「患者」）。患者＝記録。一覧を押すと記録を切り替え、meta.json の info を表に出す。
// 純関数（DOM を使わない）は末尾で node のテストに渡す。

const tsPat = { host: null, input: null, count: null, list: null, title: null, note: null, table: null };

// 記録の名前を絞り込む（部分一致・大文字小文字を区別しない）
function tsFilterRecords(names, q) {
  const k = String(q === undefined || q === null ? "" : q).trim().toLowerCase();
  return k ? names.filter((n) => n.toLowerCase().includes(k)) : names.slice();
}

// info の値を文字にする（文字と数はそのまま。それ以外は JSON の文字）
function tsPatVal(v) {
  return typeof v === "string" || typeof v === "number" ? String(v) : JSON.stringify(v);
}

// 情報の表の行 [名前, 値]。meta.info の key/value、続けて srate・長さ・変数の数
function tsPatientRows(meta) {
  if (!meta) return [];
  const rows = [];
  if (tsPatHasInfo(meta)) {
    for (const k of Object.keys(meta.info)) rows.push([k, tsPatVal(meta.info[k])]);
  }
  const dur = meta.srate ? meta.n_samples / meta.srate : 0;
  rows.push(["srate", `${meta.srate} Hz`]);
  rows.push(["長さ", `${dur.toFixed(3)} s`]);
  rows.push(["変数", `${(meta.vars || []).length} 個`]);
  return rows;
}

// meta.info が object（配列ではない）か
function tsPatHasInfo(meta) {
  const info = meta && meta.info;
  return !!info && typeof info === "object" && !Array.isArray(info);
}

function tsPatRender(el) {
  const count = tsEl("div", "ts-pat-count");
  const input = tsEl("input");
  input.type = "text";
  input.placeholder = "絞り込み";
  input.setAttribute("aria-label", "患者の絞り込み");
  input.oninput = () => tsPatDraw();
  const head = tsEl("div", "ts-pat-head");
  head.append(count, input);
  const list = tsEl("ul", "ts-pat-list");
  const title = tsEl("h3", "ts-pat-title");
  const note = tsEl("div", "ts-pat-note");
  const table = tsEl("table", "ts-pat-info");
  el.replaceChildren(head, list, title, note, table);
  Object.assign(tsPat, { host: el, input, count, list, title, note, table });
  tsPatDraw();
}

// 状態が変わったら描き直す（絞り込みの入力欄は作り直さない。打ち込みの途中で消えないように）
function tsPatDraw() {
  if (!tsPat.host) return;
  const st = window.TS.state;
  const names = tsRecordsOf(st.datasets, st.dataset);
  const shown = tsFilterRecords(names, tsPat.input.value);
  tsPat.count.textContent = `患者（${names.length}）`;
  const items = shown.map((n) => {
    const b = tsEl("button", "", n);
    b.type = "button";
    b.setAttribute("aria-current", n === st.record ? "true" : "false");
    b.onclick = () => window.TS.setView({ record: n, selectedEvents: [] });
    const li = tsEl("li");
    li.append(b);
    return li;
  });
  if (!shown.length) {
    items.push(tsEl("li", "ts-pat-none", names.length ? "一致する患者がありません" : "記録がありません"));
  }
  tsPat.list.replaceChildren(...items);
  tsPatDrawInfo(st);
}

// 選んでいる患者の情報。meta が今の記録のものでなければ、読み終わるまで待つ
function tsPatDrawInfo(st) {
  tsPat.title.textContent = st.record ? `${st.record} の情報` : "情報";
  let rows = [];
  let note = "";
  if (!st.record) {
    note = "記録を選んでください";
  } else if (!st.meta) {
    note = "記録を読めません";
  } else if (st.metaKey !== st.dataset + "\n" + st.record) {
    note = "読み込み中…";
  } else {
    rows = tsPatientRows(st.meta);
    if (!tsPatHasInfo(st.meta)) note = "情報はありません（meta.json の info に書くと出ます）";
  }
  tsPat.note.textContent = note;
  tsPat.table.replaceChildren(
    ...rows.map(([k, v]) => {
      const tr = tsEl("tr");
      tr.append(tsEl("th", "", k), tsEl("td", "", v));
      return tr;
    }),
  );
}

function tsPatBoot() {
  TS.ui.addSidebarTab({ id: "patients", title: "患者", render: tsPatRender });
  TS.on("state", tsPatDraw);
}

if (typeof window !== "undefined" && window.TS) {
  window.TS.on("boot", tsPatBoot);
}

if (typeof module !== "undefined") {
  module.exports = { tsFilterRecords, tsPatientRows };
}
