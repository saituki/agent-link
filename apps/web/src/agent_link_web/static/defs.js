"use strict";
// agent-link-web の設定画面（実装計画 U5・共有契約 C-F1）。
//
// 住所は 2 つ:
//   一覧   #/project/<name>/defs
//   編集   #/project/<name>/defs/<kind>/<scope>/<def_name>
// 中身は .agent-link/blocks/*.md・.agent-link/template/*.md・.agent-link/settings.json を、
// このプロジェクトと全プロジェクト共通（~/.agent-link）の両方で一覧・編集・
// 新規作成・削除できるもの。壊れた定義は保存されない（検査は agent-link がする・
// main.py の phone_def_write）。画面では名前の規則だけ見る。
//
// 画面は素の JavaScript。md の整形は markdown.js の renderMarkdown。
// DOM を使わない純関数（dfValidName / dfTemplate / dfGroups / dfHash / dfParseHash）は
// 末尾で node に渡す（tests/test_defs.js）。
// 一番外側の名前は df / DF_ で始める（C-F6）。

// --- 定数 ---

// 定義の種類（並び順もこれ）
const DF_KINDS = ["block", "template", "settings"];
// 置き場（並び順もこれ）
const DF_SCOPES = ["project", "global"];
// settings の定義名（これ 1 つだけ）
const DF_SETTINGS_NAME = "settings";
// 定義名の規則（agent-link の検査と同じ形・C-A1）
const DF_NAME_RE = /^[A-Za-z0-9][A-Za-z0-9_-]{0,63}$/;

// --- 状態 ---

let dfName = "";         // 開いているプロジェクト名
let dfData = null;       // 一覧の応答（GET …/defs）
let dfError = "";        // 赤い 1 行
let dfBusy = false;      // 保存・作成・削除の途中
let dfCur = null;        // 編集している定義 {kind, scope, name}
let dfText = "";         // サーバーが返した本文
let dfDraft = "";        // 編集中の本文
let dfWarnings = [];     // 保存を止めない注意
let dfSaveError = "";    // 保存に失敗したときの理由（本文は残す）
let dfPreview = false;   // プレビューを出すか（狭い画面だけ）
let dfLastHash = "";     // 前の住所（保存せずに戻るため）
let dfGoHash = "";       // いま許可した移動先

// ============================================================================
// DOM を使わない部品（node のテストから読む）
// ============================================================================

/** 一覧の住所。 */
function dfListHash(name) {
  return `#/project/${encodeURIComponent(name || "")}/defs`;
}

/** 編集の住所。defName が無ければ一覧の住所。 */
function dfHash(name, kind, scope, defName) {
  const base = dfListHash(name);
  if (!kind || !scope || !defName) return base;
  return `${base}/${encodeURIComponent(kind)}/${encodeURIComponent(scope)}/${encodeURIComponent(defName)}`;
}

/**
 * 設定の住所から {name, kind, scope, defName} を取り出す。
 * 設定のアドレスでなければ null（kind / scope / defName は null = 一覧）。
 */
function dfParseHash(hash) {
  const m = String(hash || "").match(/^#\/project\/([^/]+)\/defs(?:\/([^/]+)\/([^/]+)\/([^/?#]+))?\/?$/);
  if (!m) return null;
  const out = {
    name: decodeURIComponent(m[1]),
    kind: null,
    scope: null,
    defName: null,
  };
  if (m[2] == null) return out;
  const kind = decodeURIComponent(m[2]);
  const scope = decodeURIComponent(m[3]);
  const defName = decodeURIComponent(m[4]);
  if (DF_KINDS.indexOf(kind) < 0 || DF_SCOPES.indexOf(scope) < 0) return out;
  out.kind = kind;
  out.scope = scope;
  out.defName = defName;
  return out;
}

/**
 * 定義名の検査。{ok, message} を返す。
 * settings は名前が settings だけ。block / template は英数字・-・_ の 64 文字まで。
 */
function dfValidName(kind, name) {
  const key = String(name == null ? "" : name);
  if (!key) return { ok: false, message: "Enter a name." };
  if (kind === "settings") {
    return key === DF_SETTINGS_NAME
      ? { ok: true, message: "" }
      : { ok: false, message: `The settings file name must be ${DF_SETTINGS_NAME}.` };
  }
  if (!DF_NAME_RE.test(key)) {
    return {
      ok: false,
      message: "The name must start with a letter or digit and use only letters, digits, - and _ (up to 64 characters).",
    };
  }
  return { ok: true, message: "" };
}

/** block の空のひな形。sai-base がこのプロジェクトにも共通にも無ければ templates: の行を入れない。 */
function dfTemplate(kind, name, hasSaiBase) {
  if (kind === "settings") return "{}\n";
  if (kind === "template") return `# ${name}\n\n`;
  const lines = [
    "---",
    `name: ${name}`,
    `id: ${name}`,
    "description: One sentence on what this block does",
  ];
  if (hasSaiBase) lines.push("templates: sai-base");
  lines.push("---", "", "# Instructions", "", "Write the instructions for this block's agent here.", "");
  return lines.join("\n");
}

/** 種類と置き場の日本語。 */
function dfKindLabel(kind) {
  if (kind === "block") return "block";
  if (kind === "template") return "template";
  if (kind === "settings") return "settings";
  return String(kind || "");
}

function dfScopeLabel(scope) {
  return scope === "global" ? "All projects (~/.agent-link)" : "This project";
}

/**
 * 一覧を「種類 × 置き場」にまとめる（DOM は使わない）。
 * [{kind, kindLabel, scopes: [{scope, scopeLabel, items}]}] の並びを返す。
 * 並びは kind（block → template → settings）→ scope（project → global）。
 * 該当が 0 個の組は入れない（settings の 2 つは必ず出る）。
 */
function dfGroups(items) {
  const list = Array.isArray(items) ? items : [];
  const out = [];
  for (const kind of DF_KINDS) {
    const scopes = [];
    for (const scope of DF_SCOPES) {
      const rows = list.filter((i) => i && i.kind === kind && i.scope === scope);
      if (rows.length === 0) continue;
      rows.sort((a, b) => String(a.name || "").localeCompare(String(b.name || "")));
      scopes.push({ scope, scopeLabel: dfScopeLabel(scope), items: rows });
    }
    if (scopes.length === 0) continue;
    out.push({ kind, kindLabel: dfKindLabel(kind), scopes });
  }
  return out;
}

/** 定義の題（無ければ名前）。 */
function dfTitle(item) {
  if (!item) return "";
  return String(item.title || item.name || "");
}

/** 一覧の 1 行に出す札（[{text, cls}]）。 */
function dfRowChips(item) {
  const chips = [];
  if (!item) return chips;
  if (item.exists === false) chips.push({ text: "Not created yet (you can create it)", cls: "muted" });
  else if (item.scope === "global") chips.push({ text: "Shared", cls: "ok" });
  if (item.shadowed) chips.push({ text: "The project copy is used", cls: "muted" });
  if (item.error) chips.push({ text: "⚠ Cannot read", cls: "danger" });
  const used = Array.isArray(item.used_by) ? item.used_by : [];
  if (used.length > 0) chips.push({ text: `Used by ${used.length} block${used.length === 1 ? "" : "s"}`, cls: "muted" });
  return chips;
}

/** 「使っている block」の文言。 */
function dfUsedByText(usedBy) {
  const list = Array.isArray(usedBy) ? usedBy.filter((x) => x != null && x !== "") : [];
  if (list.length === 0) return "";
  return `Used by: ${list.join(", ")}`;
}

// ============================================================================
// 通信
// ============================================================================

/** 一覧の API の住所。 */
function dfListApi(name) {
  return `/api/projects/${encodeURIComponent(name)}/defs`;
}

/** 1 件の API の住所。 */
function dfItemApi(name, kind, scope, defName) {
  return `/api/projects/${encodeURIComponent(name)}/defs/${encodeURIComponent(kind)}/${encodeURIComponent(scope)}/${encodeURIComponent(defName)}`;
}

// ============================================================================
// index.html への読み込みの保険（C-F2）
// ============================================================================
// 共通部品の ui.js と、この画面の CSS は index.html から読む（U4 が足す）。
// 読み込みがまだ無い環境でも画面が動くように、ここでも足してから開く。
// index.html に読み込みが足されたあとは何も起きない（いるものしか足さない）。

/** この画面の CSS を足す（既にあるなら何もしない）。 */
function dfStyle(href) {
  if (typeof document === "undefined") return;
  if (document.querySelector(`link[rel="stylesheet"][href^="${href}"]`)) return;
  const link = document.createElement("link");
  link.rel = "stylesheet";
  link.href = href;
  document.head.appendChild(link);
}

let dfUiPromise = null;

/** 共通部品の ui.js がなければ読み込む（待つ）。 */
function dfUi() {
  if (typeof saiApi === "function") return Promise.resolve();
  if (dfUiPromise) return dfUiPromise;
  dfUiPromise = new Promise((resolve) => {
    const s = document.createElement("script");
    s.src = "/ui.js?v=28";
    s.onload = () => resolve();
    s.onerror = () => resolve(); // 読めなくてもエラーはここで止める
    document.head.appendChild(s);
  });
  return dfUiPromise;
}

// ============================================================================
// 一覧の画面
// ============================================================================

/** 画面を組み立てる（帯・赤い 1 行・見出し）。 */
function dfBuild() {
  const root = document.getElementById("defs-root");
  if (!root) return;
  root.innerHTML = "";
  const bar = document.createElement("header");
  const editing = dfCur !== null && dfCur !== undefined;
  bar.className = editing ? "appbar df-bar" : "appbar df-bar proj-bar";
  const back = document.createElement("button");
  back.className = editing ? "df-back" : "proj-back";
  back.type = "button";
  back.textContent = "‹";
  if (editing) {
    // 編集の画面は今のまま（一覧へ戻る・切り替えは出さない）
    back.setAttribute("aria-label", "Back to list");
    back.addEventListener("click", () => dfLeave(dfListHash(dfName)));
  } else {
    // 一覧の画面は 4 画面共通の見出し（‹ プロジェクト名 切り替え・U01 §3）
    back.setAttribute("aria-label", "Back to projects");
    back.addEventListener("click", () => { location.hash = "#/projects"; });
  }
  const h1 = document.createElement("h1");
  h1.className = editing ? "df-title" : "proj-title";
  h1.textContent = editing ? "Settings" : dfName;
  bar.append(back, h1);
  if (!editing) {
    // 切り替えは 4 画面の共有の見出しの中に置く（下の段には出さない）
    const seg = document.createElement("div");
    seg.className = "list-mode";
    seg.id = "df-mode-seg";
    bar.appendChild(seg);
  }
  root.appendChild(bar);

  const err = document.createElement("div");
  err.className = "df-error";
  err.id = "df-error";
  err.hidden = true;
  root.appendChild(err);

  const body = document.createElement("main");
  body.className = "df-body";
  body.id = "defs-body";
  root.appendChild(body);
  if (!editing && typeof fillLinkSeg === "function") fillLinkSeg("df-mode-seg", "defs", dfName);
}

/** 一覧を描く。 */
function dfRenderList() {
  const body = document.getElementById("defs-body");
  if (!body) return;
  body.innerHTML = "";
  const err = document.getElementById("df-error");
  if (err) {
    err.textContent = dfError;
    err.hidden = !dfError;
  }
  if (dfData === null) {
    body.appendChild(dfNote("Loading…"));
    return;
  }
  const items = dfData && Array.isArray(dfData.items) ? dfData.items : [];
  if (items.length === 0) {
    body.appendChild(saiEmpty({
      icon: "⚙️",
      title: "No definitions",
      body: "Create a block with \"+ New\".",
    }));
    return;
  }

  const top = document.createElement("div");
  top.className = "df-top";
  const add = document.createElement("button");
  add.className = "sai-btn primary";
  add.type = "button";
  add.textContent = "+ New";
  add.addEventListener("click", () => dfOpenCreate());
  top.appendChild(add);
  body.appendChild(top);

  for (const group of dfGroups(items)) {
    const head = document.createElement("div");
    head.className = "sai-section-title";
    const title = document.createElement("span");
    title.textContent = group.kindLabel;
    head.appendChild(title);
    if (group.kind === "block") head.appendChild(saiTerm("block"));
    else if (group.kind === "template") head.appendChild(saiTerm("template"));
    else head.appendChild(saiTerm("settings"));
    body.appendChild(head);

    for (const scope of group.scopes) {
      const sub = document.createElement("p");
      sub.className = "df-scope";
      sub.textContent = scope.scopeLabel;
      body.appendChild(sub);
      for (const item of scope.items) body.appendChild(dfRow(item));
    }
  }
}

/** 1 行（.sai-row）。押すと編集の住所へ。 */
function dfRow(item) {
  const row = document.createElement("button");
  row.className = "sai-row press";
  row.type = "button";
  const bodyBox = document.createElement("span");
  bodyBox.className = "sai-row-body";
  const title = document.createElement("span");
  title.className = "sai-row-title";
  title.textContent = dfTitle(item) || item.name || "";
  bodyBox.appendChild(title);
  if (item.description) {
    const desc = document.createElement("span");
    desc.className = "sai-row-sub";
    desc.textContent = String(item.description);
    bodyBox.appendChild(desc);
  }
  if (item.error) {
    const err = document.createElement("span");
    err.className = "sai-row-sub df-row-error";
    err.textContent = String(item.error);
    bodyBox.appendChild(err);
  }
  const chips = dfRowChips(item);
  if (chips.length > 0) {
    const box = document.createElement("span");
    box.className = "df-row-chips";
    for (const chip of chips) {
      const c = document.createElement("span");
      c.className = `sai-chip ${chip.cls}`;
      c.textContent = chip.text;
      box.appendChild(c);
    }
    bodyBox.appendChild(box);
  }
  row.appendChild(bodyBox);
  row.addEventListener("click", () => dfLeave(dfHash(dfName, item.kind, item.scope, item.name)));
  return row;
}

function dfNote(text) {
  const p = document.createElement("p");
  p.className = "df-note";
  p.textContent = text;
  return p;
}

// ============================================================================
// 編集の画面
// ============================================================================

/** 編集の画面を描く（帯・注意・見出し・切り替え・編集・プレビュー・保存・削除）。 */
function dfRenderEdit() {
  const root = document.getElementById("defs-root");
  if (!root) return;
  root.innerHTML = "";
  dfLastHash = location.hash;
  const cur = dfCur || { kind: null, scope: null, name: "" };
  const wide = typeof saiIsWide === "function" ? saiIsWide() : false;

  const bar = document.createElement("header");
  bar.className = "appbar df-bar";
  const back = document.createElement("button");
  back.className = "df-back";
  back.type = "button";
  back.textContent = "‹";
  back.setAttribute("aria-label", "Back to list");
  back.addEventListener("click", () => dfLeave(dfListHash(dfName)));
  const h1 = document.createElement("h1");
  h1.className = "df-title";
  h1.textContent = dfTitle(dfData && dfData.items && dfData.items.find((i) => i.kind === cur.kind && i.scope === cur.scope && i.name === cur.name)) || cur.name || "";
  bar.append(back, h1);
  const chip = document.createElement("span");
  chip.className = `sai-chip ${cur.scope === "global" ? "warn" : "muted"}`;
  chip.textContent = dfScopeLabel(cur.scope);
  bar.appendChild(chip);
  root.appendChild(bar);

  // 共通の定義は黄色の帯を出す（変えるとすべてのプロジェクトに効く）
  if (cur.scope === "global") {
    const warn = document.createElement("div");
    warn.className = "df-global-warn";
    warn.textContent = "This definition is shared by all projects. Changes affect every project.";
    root.appendChild(warn);
  }

  const used = dfUsedByText(dfCurUsedBy());
  if (used) {
    const box = document.createElement("div");
    box.className = "df-used";
    const label = document.createElement("span");
    label.className = "df-used-label";
    label.textContent = used;
    box.appendChild(label);
    for (const b of dfCurUsedBy()) {
      const btn = document.createElement("button");
      btn.className = "sai-btn small";
      btn.type = "button";
      btn.textContent = String(b);
      btn.addEventListener("click", () => dfLeave(dfHash(dfName, "block", "project", b)));
      box.appendChild(btn);
    }
    root.appendChild(box);
  }

  // 道具の行（編集 | プレビュー・保存・削除）
  const tools = document.createElement("div");
  tools.className = "sai-toolbar df-tools";
  if (!wide) {
    const seg = document.createElement("div");
    seg.className = "dash-seg df-seg";
    for (const [key, label] of [["edit", "Edit"], ["preview", "Preview"]]) {
      const b = document.createElement("button");
      b.type = "button";
      b.textContent = label;
      if ((key === "preview") === dfPreview) b.className = "active";
      b.addEventListener("click", () => {
        dfPreview = key === "preview";
        dfRenderEdit();
      });
      seg.appendChild(b);
    }
    tools.appendChild(seg);
  }
  const save = document.createElement("button");
  save.className = "sai-btn primary";
  save.type = "button";
  save.textContent = "Save";
  save.disabled = dfBusy;
  save.addEventListener("click", () => void dfSave());
  tools.appendChild(save);
  if (cur.kind !== "settings") {
    const del = document.createElement("button");
    del.className = "sai-btn danger";
    del.type = "button";
    del.textContent = "Delete";
    del.disabled = dfBusy;
    del.addEventListener("click", () => void dfDelete());
    tools.appendChild(del);
  }
  root.appendChild(tools);

  // 保存の失敗（赤い箱）と保存の注意（黄色の箱）
  const msgs = document.createElement("div");
  msgs.className = "df-msgs";
  msgs.id = "df-msgs";
  root.appendChild(msgs);
  dfRenderMessages();

  // 編集とプレビュー（広い画面では左右に並べる）
  const pane = document.createElement("div");
  pane.className = wide ? "df-panes df-panes--wide" : "df-panes";
  pane.id = "df-panes";
  root.appendChild(pane);

  const editBox = document.createElement("div");
  editBox.className = "df-edit";
  editBox.id = "df-edit";
  pane.appendChild(editBox);
  const ta = document.createElement("textarea");
  ta.className = "df-text";
  ta.value = dfDraft;
  ta.spellcheck = false;
  ta.setAttribute("autocapitalize", "off");
  ta.setAttribute("autocorrect", "off");
  ta.disabled = dfBusy;
  ta.addEventListener("input", () => {
    dfDraft = ta.value;
    dfClearMessages();
  });
  editBox.appendChild(ta);
  if (!wide && dfPreview) editBox.hidden = true;

  const prevBox = document.createElement("div");
  prevBox.className = "df-preview";
  prevBox.id = "df-preview";
  pane.appendChild(prevBox);
  if (!wide && !dfPreview) prevBox.hidden = true;
  dfDrawPreview(prevBox);
}

/** 編集中の定義が使われている block の id。 */
function dfCurUsedBy() {
  if (!dfData || !Array.isArray(dfData.items)) return [];
  const cur = dfCur || {};
  const hit = dfData.items.find((i) => i && i.kind === cur.kind && i.scope === cur.scope && i.name === cur.name);
  return hit && Array.isArray(hit.used_by) ? hit.used_by : [];
}

/** プレビューを描く。 */
function dfDrawPreview(box) {
  if (!box) return;
  box.innerHTML = "";
  const cur = dfCur || {};
  if (cur.kind === "settings") {
    let pretty = dfDraft;
    try {
      pretty = JSON.stringify(JSON.parse(dfDraft || "{}"), null, 2);
    } catch (e) {
      const bad = document.createElement("p");
      bad.className = "df-bad";
      bad.textContent = `Cannot parse as JSON: ${(e && e.message) || ""}`;
      box.appendChild(bad);
    }
    const pre = document.createElement("pre");
    pre.className = "df-json";
    pre.textContent = pretty;
    box.appendChild(pre);
    return;
  }
  if (!dfDraft) {
    box.appendChild(dfNote("(No content)"));
    return;
  }
  const html = renderMarkdown(dfDraft, { toolbar: true });
  box.innerHTML = html || "";
}

/** 保存の失敗（赤い箱）と注意（黄色の箱）を描く。 */
function dfRenderMessages() {
  const box = document.getElementById("df-msgs");
  if (!box) return;
  box.innerHTML = "";
  if (dfSaveError) {
    const bad = document.createElement("div");
    bad.className = "df-box df-box--bad";
    bad.textContent = dfSaveError;
    box.appendChild(bad);
  }
  for (const w of dfWarnings) {
    const warn = document.createElement("div");
    warn.className = "df-box df-box--warn";
    warn.textContent = String(w);
    box.appendChild(warn);
  }
}

function dfClearMessages() {
  dfSaveError = "";
  const box = document.getElementById("df-msgs");
  if (!box) return;
  const bad = box.querySelector(".df-box--bad");
  if (bad) bad.remove();
}

/** 保存していない変更があるか。 */
function dfDirty() {
  if (!dfCur) return false;
  return dfDraft !== dfText;
}

/** 保存せずに住所を移る（変更が残っていたら<saiConfirm>で確認する）。 */
function dfLeave(hash) {
  if (dfDirty()) {
    saiConfirm({
      title: "You have unsaved changes",
      body: "Discard them and leave?",
      ok: "Discard",
      danger: true,
    }).then((ok) => {
      if (!ok) return;
      dfDiscard();
      dfGoHash = hash;
      location.hash = hash;
    });
    return;
  }
  dfGoHash = hash;
  location.hash = hash;
}

/** 編集をやめて、覚えた本文に戻す。 */
function dfDiscard() {
  dfDraft = dfText;
  dfSaveError = "";
  dfWarnings = [];
  dfPreview = false;
}

// --- 保存・削除 ---

async function dfSave() {
  const cur = dfCur;
  if (!cur || dfBusy) return;
  dfBusy = true;
  dfSaveError = "";
  try {
    const out = await saiApi(dfItemApi(dfName, cur.kind, cur.scope, cur.name), {
      method: "PUT",
      body: { text: dfDraft, create: false },
    });
    dfText = typeof (out && out.text) === "string" ? out.text : dfDraft;
    dfDraft = dfText;
    dfWarnings = (out && Array.isArray(out.warnings)) ? out.warnings : [];
    saiToast("Saved.", "ok");
  } catch (e) {
    // 本文はそのまま残す（保存されていない）
    dfSaveError = (e && e.message) || "Could not save.";
  } finally {
    dfBusy = false;
    dfRenderEdit();
  }
}

async function dfDelete() {
  const cur = dfCur;
  if (!cur || dfBusy) return;
  const used = dfCurUsedBy();
  const body = used.length > 0
    ? `Used by: ${used.join(", ")}. Deleting it will stop those blocks from running.`
    : `Delete the ${dfKindLabel(cur.kind)} "${cur.name}"? This cannot be undone.`;
  const ok = await saiConfirm({
    title: "Delete?",
    body,
    ok: "Delete",
    danger: true,
  });
  if (!ok) return;
  dfBusy = true;
  try {
    await saiApi(dfItemApi(dfName, cur.kind, cur.scope, cur.name), { method: "DELETE" });
    saiToast("Deleted.", "ok");
  } catch (e) {
    dfBusy = false;
    saiToast((e && e.message) || "Could not delete.", "error");
    return;
  }
  dfBusy = false;
  dfCur = null;
  dfDraft = "";
  dfText = "";
  dfGoHash = dfListHash(dfName);
  location.hash = dfGoHash;
}

// ============================================================================
// 新規作成（小窓）
// ============================================================================

/** 「＋ 新しく作る」の小窓。 */
function dfOpenCreate() {
  const body = document.createElement("div");
  body.className = "df-create";

  const state = { kind: "block", scope: "project", name: "", from: "empty", source: "" };

  const kindBox = document.createElement("div");
  kindBox.className = "df-choices";
  for (const [key, label] of [["block", "Block"], ["template", "Template"]]) {
    const b = document.createElement("button");
    b.className = "wf-tool" + (key === state.kind ? " active" : "");
    b.type = "button";
    b.textContent = label;
    b.addEventListener("click", () => {
      state.kind = key;
      for (const el of kindBox.children) el.classList.remove("active");
      b.classList.add("active");
      nameMsg.textContent = dfValidName(state.kind, state.name).message;
    });
    kindBox.appendChild(b);
  }
  body.appendChild(dfFieldLabel("Kind", kindBox));

  const scopeBox = document.createElement("div");
  scopeBox.className = "df-choices";
  for (const [key, label] of [["project", "This project"], ["global", "All projects"]]) {
    const b = document.createElement("button");
    b.className = "wf-tool" + (key === state.scope ? " active" : "");
    b.type = "button";
    b.textContent = label;
    b.addEventListener("click", () => {
      state.scope = key;
      for (const el of scopeBox.children) el.classList.remove("active");
      b.classList.add("active");
    });
    scopeBox.appendChild(b);
  }
  body.appendChild(dfFieldLabel("Location", scopeBox));

  const nameInput = document.createElement("input");
  nameInput.type = "text";
  nameInput.className = "sai-input";
  nameInput.placeholder = "plan-kantoku";
  nameInput.addEventListener("input", () => {
    state.name = nameInput.value;
    const v = dfValidName(state.kind, state.name);
    nameMsg.textContent = v.message;
    nameMsg.classList.toggle("df-bad", !v.ok && !!state.name);
  });
  const nameMsg = document.createElement("p");
  nameMsg.className = "df-name-msg";
  const nameField = document.createElement("div");
  nameField.append(nameInput, nameMsg);
  body.appendChild(dfFieldLabel("Name (letters, digits, - and _; up to 64 characters)", nameField));

  const fromBox = document.createElement("div");
  fromBox.className = "df-choices";
  for (const [key, label] of [["empty", "Empty template"], ["copy", "Copy an existing definition"]]) {
    const b = document.createElement("button");
    b.className = "wf-tool" + (key === state.from ? " active" : "");
    b.type = "button";
    b.textContent = label;
    b.addEventListener("click", () => {
      state.from = key;
      for (const el of fromBox.children) el.classList.remove("active");
      b.classList.add("active");
      copyField.hidden = key !== "copy";
    });
    fromBox.appendChild(b);
  }
  body.appendChild(dfFieldLabel("Start from", fromBox));

  // 写す元の一覧（今ある block / template）
  const copySelect = document.createElement("select");
  copySelect.className = "sai-input";
  const items = dfData && Array.isArray(dfData.items) ? dfData.items : [];
  for (const it of items) {
    if (it.kind !== "block" && it.kind !== "template") continue;
    if (it.exists === false) continue;
    const opt = document.createElement("option");
    opt.value = `${it.kind}/${it.scope}/${it.name}`;
    opt.textContent = `${dfTitle(it) || it.name} (${dfScopeLabel(it.scope)})`;
    copySelect.appendChild(opt);
  }
  copySelect.addEventListener("change", () => { state.source = copySelect.value; });
  const copyField = document.createElement("div");
  copyField.appendChild(copySelect);
  copyField.hidden = true;
  body.appendChild(dfFieldLabel("Copy from", copyField));

  const err = document.createElement("p");
  err.className = "df-box df-box--bad";
  err.hidden = true;
  body.appendChild(err);

  const row = document.createElement("div");
  row.className = "sai-panel-row";
  const cancel = document.createElement("button");
  cancel.className = "sai-btn ghost";
  cancel.type = "button";
  cancel.textContent = "Cancel";
  cancel.addEventListener("click", () => sheet.close());
  const make = document.createElement("button");
  make.className = "sai-btn primary";
  make.type = "button";
  make.textContent = "Create";
  make.addEventListener("click", () => void dfCreate(state, err, make, sheet));
  row.append(cancel, make);
  body.appendChild(row);

  const sheet = saiSheet({ title: "New definition", body });
  nameInput.focus();
}

/** 小窓の見出しと中身。 */
function dfFieldLabel(label, content) {
  const box = document.createElement("div");
  box.className = "df-field";
  const p = document.createElement("p");
  p.className = "df-field-label";
  p.textContent = label;
  box.append(p, content);
  return box;
}

/** 定義を作る。 */
async function dfCreate(state, errBox, button, sheet) {
  errBox.hidden = true;
  const v = dfValidName(state.kind, state.name);
  if (!v.ok) {
    errBox.textContent = v.message;
    errBox.hidden = false;
    return;
  }
  let text = dfTemplate(state.kind, state.name, dfHasSaiBase());
  if (state.from === "copy") {
    const src = String(state.source || "").split("/");
    if (src.length === 3) {
      try {
        const out = await saiApi(dfItemApi(dfName, src[0], src[1], src[2]));
        if (out && typeof out.text === "string") text = out.text;
      } catch (e) {
        errBox.textContent = `Could not load the source: ${(e && e.message) || ""}`;
        errBox.hidden = false;
        return;
      }
    } else {
      errBox.textContent = "Choose a source to copy.";
      errBox.hidden = false;
      return;
    }
  }
  button.disabled = true;
  try {
    await saiApi(dfItemApi(dfName, state.kind, state.scope, state.name), {
      method: "PUT",
      body: { text, create: true },
    });
    saiToast("Created.", "ok");
  } catch (e) {
    errBox.textContent = (e && e.message) || "Could not create.";
    errBox.hidden = false;
    button.disabled = false;
    return;
  }
  button.disabled = false;
  if (sheet && typeof sheet.close === "function") sheet.close();
  await dfLoadList();
  location.hash = dfHash(dfName, state.kind, state.scope, state.name);
}

/** このプロジェクトか共通かに sai-base があるか（ひな形の templates: の出し入れに使う）。 */
function dfHasSaiBase() {
  const items = dfData && Array.isArray(dfData.items) ? dfData.items : [];
  return items.some((i) => i && i.kind === "template" && i.name === "sai-base" && i.exists !== false);
}

// ============================================================================
// 開く・読み直す
// ============================================================================

/** 一覧を読み直す。 */
async function dfLoadList() {
  if (!dfName) return;
  const name = dfName;
  try {
    const data = await saiApi(dfListApi(name));
    if (dfName !== name) return;
    dfData = data && typeof data === "object" ? data : { items: [] };
    dfError = "";
  } catch (e) {
    if (dfName !== name) return;
    dfError = (e && e.message) || "Could not load.";
    dfData = { items: [] };
  }
  dfRenderList();
}

/** 編集している定義を読み直す。 */
async function dfLoadOne() {
  const cur = dfCur;
  if (!cur) return;
  try {
    const out = await saiApi(dfItemApi(dfName, cur.kind, cur.scope, cur.name));
    dfText = (out && typeof out.text === "string") ? out.text : "";
    dfWarnings = (out && Array.isArray(out.warnings)) ? out.warnings : [];
    dfError = "";
  } catch (e) {
    dfText = "";
    dfWarnings = [];
    dfError = (e && e.message) || "Could not load.";
  }
  dfDraft = dfText;
}

/** 一覧の画面を開く。 */
async function dfOpenList(name) {
  dfStyle("/defs.css?v=28");
  await dfUi();
  dfName = name;
  dfCur = null;
  dfDraft = "";
  dfText = "";
  dfSaveError = "";
  dfWarnings = [];
  dfPreview = false;
  dfError = "";
  dfData = null;
  dfBuild();
  await dfLoadList();
}

/** 編集の画面を開く。 */
async function dfOpenEdit(name, kind, scope, defName) {
  dfStyle("/defs.css?v=28");
  await dfUi();
  const first = dfName !== name || dfCur === null || dfCur.kind !== kind
    || dfCur.scope !== scope || dfCur.name !== defName;
  dfName = name;
  if (first) {
    dfCur = { kind, scope, name: defName };
    dfText = "";
    dfDraft = "";
    dfSaveError = "";
    dfWarnings = [];
    dfPreview = false;
  }
  dfBuild();
  if (!dfData) await dfLoadList();   // 題と「使っている block」のため
  await dfLoadOne();
  if (dfError) {
    const err = document.getElementById("df-error");
    if (err) {
      err.textContent = dfError;
      err.hidden = false;
    }
  }
  dfRenderEdit();
}

// ============================================================================
// 画面の登録（window.SaiRoutes）
// ============================================================================

if (typeof window !== "undefined") {
  (window.SaiRoutes = window.SaiRoutes || []).push({
    match(hash) {
      return dfParseHash(hash);
    },
    view: "view-defs",
    tab: "project",
    hideTabbar: false,
    interval: 0, // 定期更新はしない（保存・削除のあとだけ読み直す）
    async open(params) {
      if (params && params.kind) {
        await dfOpenEdit(params.name, params.kind, params.scope, params.defName);
      } else {
        await dfOpenList(params && params.name);
      }
    },
    async poll() {},
  });

  // 変更が残ったまま別の画面へ移ろうとしたときの確認と、ページを閉じるときの確認
  window.addEventListener("hashchange", () => {
    if (!dfDirty()) {
      dfLastHash = location.hash;
      dfGoHash = "";
      return;
    }
    const to = location.hash;
    if (to === dfGoHash) { // 自分が許可した移動
      dfLastHash = to;
      dfGoHash = "";
      return;
    }
    saiConfirm({
      title: "You have unsaved changes",
      body: "Discard them and leave?",
      ok: "Discard",
      danger: true,
    }).then((ok) => {
      if (ok) {
        dfDiscard();
        dfGoHash = to;
        if (to !== location.hash) location.hash = to;
        else dfLastHash = to;
        return;
      }
      if (dfLastHash && dfLastHash !== location.hash) {
        dfGoHash = dfLastHash;
        location.hash = dfLastHash;
      }
    });
  });

  window.addEventListener("beforeunload", (e) => {
    if (!dfDirty()) return;
    e.preventDefault();
    e.returnValue = "";
    return "";
  });
}

// --- node からの読み込み（tests/test_defs.js） ---
if (typeof module !== "undefined" && module.exports) {
  module.exports = {
    DF_KINDS,
    DF_SCOPES,
    DF_SETTINGS_NAME,
    DF_NAME_RE,
    dfListHash,
    dfHash,
    dfParseHash,
    dfValidName,
    dfTemplate,
    dfGroups,
    dfKindLabel,
    dfScopeLabel,
    dfTitle,
    dfRowChips,
    dfUsedByText,
  };
}
