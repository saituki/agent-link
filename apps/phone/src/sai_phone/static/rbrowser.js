"use strict";
// リモートブラウザ（#/papers/browser?name=…&doi=…&<取り込み対象>）。
// Mac の Chrome が映した論文ページの画像を 1 秒ごとに取って見せ、タップ・スクロール・
// 文字入力を API へ送る。保存された PDF は「Import」で sai-moon-paper に取り込む。
// 一番外側の名前の頭は rb（tests/test_globals.js が重なりを検査する）。

const RB_POLL_MS = 1000; // 画像を取り直す間隔（重ければここを伸ばす）
const RB_SCROLL_PX = 500; // ▲▼ 1 回のスクロール量
const RB_AFTER_INPUT_MS = 400; // 操作を送ってから取り直すまでの待ち
const RB_JOB_POLL_MS = 3000; // 取り込みの進み具合を見に行く間隔
const RB_PATH = "#/papers/browser";

// --- 住所（DOM を使わない） ---

const RB_NUM_KEYS = ["log_id", "position", "paper_id", "cite_paper_id", "ref_no"];

/** 住所を作る。target は {log_id, position} / {paper_id} / {import_id} / {cite_paper_id, ref_no}。 */
function rbBrowserHash(name, doi, target) {
  const q = [`name=${encodeURIComponent(name)}`, `doi=${encodeURIComponent(doi)}`];
  for (const k of Object.keys(target || {})) q.push(`${k}=${encodeURIComponent(target[k])}`);
  return `${RB_PATH}?${q.join("&")}`;
}

/** 住所を {name, doi, target} にする。違う住所・対象が 0 個や 2 個以上なら null。 */
function rbParseHash(hash) {
  const h = String(hash || "");
  if (h !== RB_PATH && !h.startsWith(`${RB_PATH}?`)) return null;
  const q = new URLSearchParams(h.includes("?") ? h.slice(h.indexOf("?") + 1) : "");
  const num = (k) => {
    const v = q.get(k);
    return v != null && /^\d+$/.test(v) ? Number(v) : null;
  };
  const sets = [];
  if (q.has("log_id") || q.has("position")) {
    const a = num("log_id"), b = num("position");
    if (a == null || b == null) return null;
    sets.push({ log_id: a, position: b });
  }
  if (q.has("paper_id")) {
    const a = num("paper_id");
    if (a == null) return null;
    sets.push({ paper_id: a });
  }
  if (q.has("import_id")) {
    const v = q.get("import_id");
    if (!v) return null;
    sets.push({ import_id: v });
  }
  if (q.has("cite_paper_id") || q.has("ref_no")) {
    const a = num("cite_paper_id"), b = num("ref_no");
    if (a == null || b == null) return null;
    sets.push({ cite_paper_id: a, ref_no: b });
  }
  if (sets.length !== 1) return null;
  return { name: q.get("name") || "", doi: q.get("doi") || "", target: sets[0] };
}

/** 呼び出し側（papers.js）が使う。住所へ移る。 */
function rbOpenHash(name, doi, target) {
  location.hash = rbBrowserHash(name, doi, target);
}

// --- 小さな計算（DOM を使わない） ---

function rbClamp01(v) {
  return Math.min(1, Math.max(0, v));
}

/** 画像の表示領域 rect とタップ位置から tap の body を作る（0〜1 に丸める）。 */
function rbTapBody(rect, clientX, clientY) {
  const w = rect.width || 1, h = rect.height || 1;
  return {
    type: "tap",
    x: rbClamp01((clientX - rect.left) / w),
    y: rbClamp01((clientY - rect.top) / h),
  };
}

/** 1.2 MB の形。 */
function rbSizeLabel(bytes) {
  const n = Number(bytes) || 0;
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  return `${(n / 1024 / 1024).toFixed(1)} MB`;
}

/** state.downloads の .pdf だけ。importable は done の物だけ true。 */
function rbPdfDownloads(state) {
  const list = state && Array.isArray(state.downloads) ? state.downloads : [];
  return list
    .filter((d) => d && /\.pdf$/i.test(String(d.name || "")))
    .map((d) => ({ name: d.name, size: d.size, done: !!d.done, importable: !!d.done }));
}

/** 取り込み対象の仕事の箱の key は 202 の応答で決まる。ここでは仕事から paper_id を取る。 */
function rbJobPaperId(job) {
  return job && job.paper_id != null && job.paper_id !== "" ? job.paper_id : null;
}

// --- API の呼び出し（apiFetch 相当を引数で受ける。tests から差し替える） ---

function rbBase(name) {
  return `/api/projects/${encodeURIComponent(name)}/papers`;
}

async function rbJson(res) {
  return res.json().catch(() => ({}));
}

/** 開く。{ok, state, detail}。 */
async function rbCallOpen(fetchFn, base, doi) {
  const res = await fetchFn(`${base}/browser/open`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ doi }),
  });
  const body = await rbJson(res);
  return res.ok ? { ok: true, state: body } : { ok: false, detail: body.detail || "開けませんでした" };
}

/** 操作を送る。{ok, closed, detail}。 */
async function rbCallInput(fetchFn, base, input) {
  const res = await fetchFn(`${base}/browser/input`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(input),
  });
  if (res.ok) return { ok: true };
  const body = await rbJson(res);
  return { ok: false, closed: res.status === 409, detail: body.detail || "操作を送れませんでした" };
}

/** Import を依頼する。{ok, key, detail}。 */
async function rbCallImport(fetchFn, base, target, fileName) {
  const res = await fetchFn(`${base}/browser/import`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ ...target, file_name: fileName }),
  });
  const body = await rbJson(res);
  return res.ok
    ? { ok: true, key: body.key }
    : { ok: false, detail: body.detail || "取り込みを始められませんでした" };
}

// --- 画面（ブラウザでのみ） ---

let rbTimer = null; // 更新のタイマー（1 つだけ）
let rbBusy = false; // 取得の最中は次を始めない
let rbCtx = null; // {name, doi, target, base}
let rbState = null; // 最後に取った state
let rbImportKeys = []; // 取り込みを始めた仕事の key
let rbImgUrl = ""; // 今の画像の blob URL
let rbTick = 0;
let rbJobAt = 0;

function rbStop() {
  if (rbTimer) clearInterval(rbTimer);
  rbTimer = null;
}

function rbHere() {
  return location.hash.startsWith(RB_PATH);
}

function rbRoot() {
  return document.getElementById("rb-root");
}

function rbEl(tag, cls, text) {
  return typeof el === "function" ? el(tag, cls, text) : Object.assign(document.createElement(tag), { className: cls || "", textContent: text || "" });
}

function rbButton(label, onClick, cls) {
  const b = rbEl("button", cls ? `rb-btn ${cls}` : "rb-btn", label);
  b.type = "button";
  b.addEventListener("click", onClick);
  return b;
}

async function rbOpen(params) {
  rbStop();
  rbCtx = { name: params.name, doi: params.doi, target: params.target, base: rbBase(params.name) };
  rbState = null;
  rbImportKeys = [];
  const root = rbRoot();
  root.innerHTML = "";
  root.appendChild(rbEl("p", "rb-msg", "Mac のブラウザを開いています…"));
  let r;
  try {
    r = await rbCallOpen(apiFetch, rbCtx.base, rbCtx.doi);
  } catch {
    r = { ok: false, detail: "接続できていません" };
  }
  if (!rbHere()) return;
  if (!r.ok) {
    root.innerHTML = "";
    root.appendChild(rbEl("p", "rb-error", r.detail));
    root.appendChild(rbButton("← 戻る", () => history.back()));
    return;
  }
  rbState = r.state;
  rbBuild();
  void rbRefresh();
  rbTimer = setInterval(() => { void rbRefresh(); }, RB_POLL_MS);
}

/** 画面の骨組みを作る。画像と一覧は rbRefresh が差し替える。 */
function rbBuild() {
  const root = rbRoot();
  root.innerHTML = "";
  const head = rbEl("div", "rb-head");
  head.appendChild(rbButton("← 戻る", () => history.back()));
  const title = rbEl("span", "rb-title", "");
  title.id = "rb-title";
  head.appendChild(title);
  head.appendChild(rbButton("閉じる", () => { void rbClose(); }, "danger"));
  root.appendChild(head);

  const wrap = rbEl("div", "rb-wrap");
  const screen = rbEl("div", "rb-screen-col");
  const img = document.createElement("img");
  img.className = "rb-screen";
  img.id = "rb-screen";
  img.alt = "Mac のブラウザの画面";
  img.addEventListener("click", (e) => {
    void rbSend(rbTapBody(img.getBoundingClientRect(), e.clientX, e.clientY));
  });
  screen.appendChild(img);
  const closed = rbEl("div", "rb-closed", "");
  closed.id = "rb-closed";
  closed.hidden = true;
  closed.appendChild(rbEl("p", "rb-error", "ブラウザが閉じています"));
  closed.appendChild(rbButton("もう一度開く", () => { void rbOpen(rbCtx); }, "primary"));
  screen.appendChild(closed);
  wrap.appendChild(screen);

  const side = rbEl("div", "rb-side");
  const ops = rbEl("div", "rb-ops");
  ops.appendChild(rbButton("▲", () => { void rbSend({ type: "scroll", dy: -RB_SCROLL_PX }); }));
  ops.appendChild(rbButton("▼", () => { void rbSend({ type: "scroll", dy: RB_SCROLL_PX }); }));
  ops.appendChild(rbButton("戻る", () => { void rbSend({ type: "back" }); }));
  ops.appendChild(rbButton("再読込", () => { void rbSend({ type: "reload" }); }));
  side.appendChild(ops);

  const form = rbEl("div", "rb-text");
  const input = document.createElement("input");
  input.type = "text";
  input.maxLength = 200;
  input.className = "rb-input";
  input.placeholder = "ログインや検索欄に入れる文字";
  const sendText = () => {
    const t = input.value;
    if (!t) return;
    input.value = "";
    void rbSend({ type: "text", text: t });
  };
  input.addEventListener("keydown", (e) => { if (e.key === "Enter") { e.preventDefault(); sendText(); } });
  form.appendChild(input);
  form.appendChild(rbButton("送信", sendText, "primary"));
  form.appendChild(rbButton("Enter", () => { void rbSend({ type: "key", key: "Enter" }); }));
  form.appendChild(rbButton("⌫", () => { void rbSend({ type: "key", key: "Backspace" }); }));
  side.appendChild(form);

  const dl = rbEl("section", "rb-downloads");
  dl.id = "rb-downloads";
  side.appendChild(dl);
  const jobs = rbEl("div", "rb-jobs");
  jobs.id = "rb-jobs";
  side.appendChild(jobs);
  wrap.appendChild(side);
  root.appendChild(wrap);
  rbDrawState();
}

async function rbSend(input) {
  if (!rbCtx) return;
  let r;
  try {
    r = await rbCallInput(apiFetch, rbCtx.base, input);
  } catch {
    saiToast("接続できていません", "error");
    return;
  }
  if (!r.ok) {
    if (r.closed) rbShowClosed(true);
    else saiToast(r.detail, "error");
    return;
  }
  setTimeout(() => { void rbRefresh(); }, RB_AFTER_INPUT_MS);
}

function rbShowClosed(on) {
  const c = document.getElementById("rb-closed");
  const img = document.getElementById("rb-screen");
  if (c) c.hidden = !on;
  if (img) img.hidden = on;
}

async function rbClose() {
  rbStop();
  try {
    await apiFetch(`${rbCtx.base}/browser`, { method: "DELETE" });
  } catch { /* 閉じられなくても戻る。Chrome は 10 分で自動終了する */ }
  history.back();
}

/** 1 秒ごとの更新。画面を離れていたら止める。 */
async function rbRefresh() {
  if (!rbHere()) { rbStop(); return; }
  if (document.hidden || rbBusy || !rbCtx) return;
  rbBusy = true;
  rbTick += 1;
  try {
    const res = await apiFetch(`${rbCtx.base}/browser/screen`);
    if (res.status === 409) {
      rbShowClosed(true);
    } else if (res.ok) {
      const url = URL.createObjectURL(await res.blob());
      const img = document.getElementById("rb-screen");
      if (img) { rbShowClosed(false); img.src = url; }
      if (rbImgUrl) URL.revokeObjectURL(rbImgUrl);
      rbImgUrl = url;
    }
    if (rbTick % 2 === 0) {
      const s = await apiFetch(`${rbCtx.base}/browser/state`);
      if (s.ok) { rbState = await s.json(); rbDrawState(); }
    }
  } catch { /* 次の周期でやり直す */ }
  rbBusy = false;
  rbPollJobs();
}

/** 取り込みの進み具合は既存の仕事の見張りに任せる（3 秒ごと）。 */
function rbPollJobs() {
  if (!rbImportKeys.length || Date.now() - rbJobAt < RB_JOB_POLL_MS) return;
  rbJobAt = Date.now();
  if (typeof pimPollJobs === "function") void pimPollJobs();
  if (typeof papersPollExtras === "function") void papersPollExtras();
  rbDrawJobs();
}

function rbDrawState() {
  const t = document.getElementById("rb-title");
  if (t && rbState) t.textContent = rbState.title || rbState.url || "";
  const box = document.getElementById("rb-downloads");
  if (!box) return;
  box.innerHTML = "";
  box.appendChild(rbEl("h3", "rb-sub", "ダウンロードした PDF"));
  const list = rbPdfDownloads(rbState);
  if (!list.length) {
    box.appendChild(rbEl("p", "rb-msg", "出版社のページで PDF のダウンロードを押すと、ここに出ます"));
    return;
  }
  for (const d of list) {
    const row = rbEl("div", "rb-dl");
    const info = rbEl("div", "rb-dl-info");
    info.appendChild(rbEl("div", "rb-dl-name", d.name));
    info.appendChild(rbEl("div", "rb-dl-meta", `${rbSizeLabel(d.size)} · ${d.done ? "保存済み" : "ダウンロード中…"}`));
    row.appendChild(info);
    if (d.importable) row.appendChild(rbButton("Import", () => { void rbImport(d.name); }, "primary"));
    box.appendChild(row);
  }
}

async function rbImport(fileName) {
  const ok = await saiConfirm({
    title: "Import",
    body: `この PDF を sai-moon-paper に取り込みますか？ ${fileName}`,
    ok: "取り込む",
    cancel: "やめる",
  });
  if (!ok) return;
  let r;
  try {
    r = await rbCallImport(apiFetch, rbCtx.base, rbCtx.target, fileName);
  } catch {
    saiToast("接続できていません", "error");
    return;
  }
  if (!r.ok) { saiToast(r.detail, "error"); return; }
  saiToast("取り込みを始めました", "ok");
  if (r.key && !rbImportKeys.includes(r.key)) rbImportKeys.push(r.key);
  rbJobAt = 0;
  rbDrawJobs();
  rbPollJobs();
}

/** 取り込みの箱と、終わった物の「論文情報を開く」を出す。 */
function rbDrawJobs() {
  const box = document.getElementById("rb-jobs");
  if (!box || !rbCtx) return;
  box.innerHTML = "";
  const jobs = typeof pimJobsList === "function" ? pimJobsList() : [];
  for (const key of rbImportKeys) {
    if (typeof jobBox === "function") box.appendChild(jobBox(key));
    const job = typeof pimJobFor === "function" ? pimJobFor(jobs, key) : null;
    const pid = rbJobPaperId(job);
    if (pid != null) {
      const a = rbEl("a", "rb-open-paper", "論文情報を開く");
      a.href = typeof paperHash === "function" ? paperHash(rbCtx.name, pid, false) : `#/paper/${encodeURIComponent(rbCtx.name)}/${pid}`;
      box.appendChild(a);
    }
  }
}

if (typeof window !== "undefined" && typeof document !== "undefined") {
  (window.SaiRoutes = window.SaiRoutes || []).push({
    match: rbParseHash,
    view: "view-papers-browser",
    tab: "papers",
    hideTabbar: true,
    interval: 0,
    async open(params) {
      await rbOpen(params);
    },
    async poll() {
      // 更新は rbTimer が行う。ここは何もしない
    },
  });
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = {
    RB_POLL_MS, RB_SCROLL_PX, rbBrowserHash, rbParseHash, rbTapBody, rbSizeLabel,
    rbPdfDownloads, rbJobPaperId, rbBase, rbCallOpen, rbCallInput, rbCallImport,
  };
}
