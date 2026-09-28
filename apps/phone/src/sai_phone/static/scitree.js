"use strict";
// 研究方針の木（Research-tree）の画面（実装計画 U19・共有契約 C-14）。
//
// 住所は #/papers/tree と #/papers/tree/<N>（節点は N12 の形）。上に種の文・関門・負債、
// 左に項目の一覧、右に項目の板を出す。狭い画面（幅 390px）では一覧を押すと板が下から出る
// （saiSheet）。広い画面（幅 900px 以上）は .sai-split の 2 列にする。
//
// 中の処理は sai-paper の `phone sci status` と `phone sci node <N>` だけを使う（C-13）。
// この画面はファイルのパスを受け取らない。
// 板の中の操作（決める・仕分け・カード・読む札）は U20 の sciedit.js / sciread.js が作る
// 関数を typeof で確かめてから呼ぶ（C-14）。関数が無ければその釦は出さないか無効にする。
// 根の図と候補の表は U21 の scigraph.js（sgRenderGraph / sgRenderTable）。
// DOM を使わない純関数（住所・並び・印・種の文・関門・負債・決定の記録）は末尾で node に
// 渡す（tests/test_scitree.js）。

// --- 定数 ---

// 一覧の表示の切り替えを覚える場所（localStorage）
const ST_MODE_KEY = "sai_phone_sci_mode";
// 表示の切り替え（一覧 / 根の図 / 表）
const ST_MODES = [["list", "一覧"], ["graph", "根の図"], ["table", "表"]];
// 研究の型（C-01。C-14 の用語の札の解説に並ぶ順）
const ST_TYPES = [
  ["problem", "課題駆動", "首先要、問いを明らかにする研究です。何をなぜやるのかから決めます。"],
  ["phenomenon", "現象駆動", "先に現象を捉え、「なぜ起きるのか」を問う研究です。"],
  ["method", "手法駆動", "先に手法を決め、その手法で何が分かるようになるかを考える研究です。"],
  ["data", "データ駆動", "手にあるデータから問いを立てる研究です。"],
  ["improve", "改良駆動", "いまのものの弱いところから始めた研究です。"],
];
// 項目の型（C-01。組み合わせてよい）
const ST_ITEM_TYPES = [["choose", "選ぶ"], ["list", "並べる"], ["write", "書く"], ["verify", "確かめる"]];
// 項目の状態の印（色の無くても分かるように記号と文字を両方出す）
const ST_ITEM_MARKS = {
  decided: { mark: "●", label: "決定" },
  tentative: { mark: "◐", label: "仮決定" },
  open: { mark: "○", label: "未決定" },
  excluded: { mark: "×", label: "対象外" },
};
// 候補の状態の印
const ST_OPTION_MARKS = {
  candidate: { mark: "◇", label: "候補" },
  selected: { mark: "◆", label: "選択" },
  rejected: { mark: "✕", label: "却下" },
  dormant: { mark: "◌", label: "休眠" },
};
// 空欄の種類（毎回計算される値・C-01）
const ST_BLANKS = {
  untouched: "未着手",
  in_progress: "調査中",
  insufficient: "情報不足",
  awaiting_decision: "判断待ち",
  conflict: "対立",
};
// 印（毎回計算される値・C-01）
const ST_NODE_MARKS = { weak_evidence: "根拠不足", recheck: "要再確認" };
// 負債（4 つ。0 のものは薄く出す）
const ST_DEBTS = [
  ["pending_proposals", "仕分けていない提案", "#/papers/triage"],
  ["unreflected_papers", "読んだが反映していない論文", ""],
  ["unscreened_sets", "選別していない検索", ""],
  ["empty_reviews", "書いていない振り返り", ""],
];
// 知見の向き（C-01）
const ST_POLARITY = { support: "支持", refute: "反証", neutral: "中立", none: "該当なし" };
// 論文の役割（相手が paper のときだけ・C-01）
const ST_ROLES = {
  exemplar: "代表例",
  compare: "比較対象",
  issue_evidence: "課題の根拠",
  method_source: "手法の供給元",
  data_source: "データの出典",
  background: "背景",
  counter: "反証",
};
// 論文の読み具合（相手が paper のときだけ・C-01）
const ST_READ_STATES = {
  unread: "未読",
  read: "読んだ・未反映",
  reflected: "反映済み",
  irrelevant: "関係なし",
};
// 結び付きの相手（paper / report / note / set / run・C-01）
const ST_LINK_KINDS = {
  paper: "論文",
  report: "報告",
  note: "ノート",
  set: "検索セット",
  run: "実行",
};

// --- 画面の状態 ---

let stData = null;      // 最後に読んだ status の JSON
let stNode = null;      // 最後に読んだ node の JSON（C-08）
let stNodeId = null;    // 今開いている節点（無ければ null）
let stSheet = null;     // 狭い画面の板（saiSheet が返した物）
let stSheetFor = null;  // 板が出している節点
let stSheetBody = null; // 板の中の入れ物（読み直して作り直すため覚えておく）

// ============================================================================
// DOM を使わない純関数（tests/test_scitree.js）
// ============================================================================

/** 住所を読む（`#/papers/tree` と `#/papers/tree/N12` だけ）。合わなければ null。 */
function stParseHash(hash) {
  const m = /^#\/papers\/tree(?:\/(N[1-9][0-9]{0,8}))?$/.exec(String(hash || ""));
  if (!m) return null;
  return { node: m[1] || null };
}

/** 状態の印（記号と表示名）。色の無くても分かるよう両方出す。 */
function stStateMark(kind, state) {
  if (kind === "option") {
    const s = ST_OPTION_MARKS[state] ? state : "candidate";
    const m = ST_OPTION_MARKS[s];
    return { mark: m.mark, label: m.label, cls: `sci-mark-${s}` };
  }
  const s = ST_ITEM_MARKS[state] ? state : "open";
  const m = ST_ITEM_MARKS[s];
  return { mark: m.mark, label: m.label, cls: `sci-mark-${s}` };
}

/**
 * 一覧に出す行（並びは status.nodes のまま）。
 * 候補は項目の 1 段下に下げる（indent が 1 増える）。
 */
function stRows(status) {
  const nodes = status && Array.isArray(status.nodes) ? status.nodes : [];
  const out = [];
  let lastItemIndent = -1; // いちばん近い項目。候補はこれを基準に 1 段下げる
  for (const n of nodes) {
    if (!n) continue;
    const kind = n.kind === "option" ? "option" : "item";
    const depth = Math.max(0, Number(n.depth) || 0);
    const st = stStateMark(kind, n.state);
    const counts = n.counts || {};
    const indent = kind === "option" ? Math.max(depth, lastItemIndent + 1) : depth;
    if (kind === "item") lastItemIndent = indent;
    out.push({
      id: n.id || "",
      kind,
      title: n.title || "",
      state: n.state || "",
      mark: st.mark,
      label: st.label,
      cls: st.cls,
      depth,
      indent,
      fixed: !!n.fixed,
      blank: kind === "item" && n.blank ? ST_BLANKS[n.blank] || n.blank : "",
      marks: (Array.isArray(n.marks) ? n.marks : []).filter((m) => ST_NODE_MARKS[m]).map((m) => ST_NODE_MARKS[m]),
      papers: Math.max(0, Number(counts.papers) || 0),
      pending: Math.max(0, Number(counts.pending) || 0),
    });
  }
  return out;
}

/** 種の文の部分。押せる（canOpen）のは filled が false で節点の立っている部分だけ。 */
function stSeedPieces(status) {
  const seed = (status && status.seed) || null;
  const parts = seed && Array.isArray(seed.parts) ? seed.parts : [];
  const out = [];
  for (const p of parts) {
    if (!p) continue;
    out.push({
      text: p.text || "",
      key: p.key || null,
      node: p.node || null,
      filled: p.filled !== false,
      tentative: !!p.tentative,
      canOpen: p.filled === false && !!p.node,
    });
  }
  return out;
}

/** 関門。全部通っていれば all が真（current が total を超える）。 */
function stGateInfo(status) {
  const g = (status && status.gate) || null;
  if (!g) return { text: "関門はまだありません", current: 0, total: 0, name: "", all: false, list: [] };
  const current = Math.max(0, Number(g.current) || 0);
  const total = Math.max(0, Number(g.total) || 0);
  const all = total > 0 && current > total;
  const name = g.name || "";
  return {
    current,
    total,
    name: all ? "種にできる" : name,
    all,
    list: Array.isArray(g.list) ? g.list : [],
    text: all ? `関門 ${total}/${total}　種にできる` : `関門 ${current}/${total}　${name}`,
  };
}

/** 負債の数（0 のものは薄く出す dim: true）。 */
function stDebtChips(status) {
  const d = (status && status.debts) || {};
  const out = [];
  for (const [key, label, hash] of ST_DEBTS) {
    const count = Math.max(0, Number(d[key]) || 0);
    out.push({ key, label, count, dim: count === 0, hash: count > 0 ? hash : "" });
  }
  return out;
}

/** 一覧が空のときに出す「次の一手」の文（status.next の先頭 few 件）。 */
function stNextLines(status, max) {
  const next = status && Array.isArray(status.next) ? status.next : [];
  const limit = Math.max(0, Number(max) || 3);
  const out = [];
  for (const s of next) {
    if (!s || out.length >= limit) break;
    const text = s.title || s.purpose || s.expect || "（次の一手なし）";
    const why = s.why ? `（${s.why}）` : "";
    out.push(`${s.node ? `${s.node} ` : ""}${text}${why}`);
  }
  return out;
}

/** 決定の記録（新しい順に並べる）。 */
function stDecisionRows(decisions) {
  const list = Array.isArray(decisions) ? decisions.filter((d) => d) : [];
  return list
    .map((d, i) => ({ d, i, t: Date.parse(d.created_at || "") || 0 }))
    .sort((a, b) => (b.t - a.t) || (b.i - a.i))
    .map((x) => x.d);
}

// ============================================================================
// 小さい DOM を作る手（ui.js の saiEl をそのまま使ってもよいが、ここでも同じ形で作る）
// ============================================================================

/** 要素を 1 つ作る（class は空白で分けた文字列、text は textContent）。 */
function stEl(tag, cls, text) {
  const e = document.createElement(tag);
  if (cls) e.className = cls;
  if (text != null) e.textContent = text;
  return e;
}

/** 用語の札（ui.js の saiTerm が無ければ null）。 */
function stTerm(key, label) {
  if (typeof saiTerm !== "function") return null;
  return saiTerm(key, label);
}

/** 空の案内（ui.js の saiEmpty が無ければそのものの文だけ）。 */
function stEmpty(opts) {
  if (typeof saiEmpty === "function") return saiEmpty(opts);
  const box = stEl("div", "sai-empty");
  if (opts && opts.title) box.appendChild(stEl("p", "sai-empty-title", opts.title));
  if (opts && opts.body) box.appendChild(stEl("p", "sai-empty-body", opts.body));
  return box;
}

/** 小さい釦。onClick が無ければ無効にする。 */
function stBtn(label, onClick, cls) {
  const b = stEl("button", `sai-btn small ${cls || ""}`.trim(), label);
  b.type = "button";
  if (typeof onClick === "function") b.addEventListener("click", onClick);
  else b.disabled = true;
  return b;
}

/** 札（sai-chip と同じ見た目。色は muted / warn / ok / danger）。 */
function stChip(text, tone) {
  return stEl("span", `sai-chip ${tone || "muted"}`, text);
}

/** 状態の印の札（記号＋文字）。 */
function stMarkChip(kind, state) {
  const m = stStateMark(kind, state);
  const chip = stChip(`${m.mark} ${m.label}`, "muted");
  chip.className = `sci-state ${m.cls}`;
  return chip;
}

/** 1 行の容器。 */
function stLine(cls) {
  return stEl("div", `sci-line ${cls || ""}`.trim());
}

// ============================================================================
// 覚えるもの（一覧の表示の切り替え）
// ============================================================================

/** 一覧の表示の切り替え（list / graph / table）を読み戻す。 */
function stLoadMode() {
  try {
    const v = localStorage.getItem(ST_MODE_KEY);
    for (const [key] of ST_MODES) if (key === v) return v;
  } catch { /* 保存できなくても動く */ }
  return "list";
}

/** 一覧の表示の切り替えを覚える。 */
function stSaveMode(mode) {
  try { localStorage.setItem(ST_MODE_KEY, mode); } catch { /* 保存できなくても動く */ }
}

// ============================================================================
// 読み書き（C-13 の住所）
// ============================================================================

/** この画面の API の住所（プロジェクト名を入れる）。 */
function stApi(path) {
  const base = typeof papersApiBase === "function"
    ? papersApiBase(selectedPaperProject())
    : `/api/projects/${encodeURIComponent(selectedPaperProject())}/papers`;
  return `${base}/sci${path || ""}`;
}

/** 最後に読んだ status の JSON（無ければ null）。 */
function stStatus() {
  return stData;
}

/** status と、開いている節点を読み直して描き直す。 */
async function stReload() {
  if (typeof document === "undefined") return;
  if (!selectedPaperProject()) {
    stRenderNoProject();
    return;
  }
  try {
    stData = await paperApi(stApi(""));
  } catch (e) {
    stData = null;
    stNode = null;
    stRenderError(e);
    return;
  }
  stNode = null;
  if (stNodeId) {
    try {
      stNode = await paperApi(stApi(`/nodes/${encodeURIComponent(stNodeId)}`));
    } catch {
      stNode = null; // 消えた節点。再読み込みで外す
    }
  }
  stRender();
}

/** 節点を選ぶ（板を開く）。住所だけ置き換えて、画面を作り直さない。 */
async function stSelect(nodeId) {
  stNodeId = nodeId || null;
  stSetHash();
  await stReload();
}

/** 住所を今の節点に合わせて置き換える（hashchange は起こさない）。 */
function stSetHash() {
  if (typeof history === "undefined" || typeof location === "undefined") return;
  const next = stNodeId ? `#/papers/tree/${stNodeId}` : "#/papers/tree";
  if (location.hash === next) return;
  history.replaceState(null, "", next);
}

/** 画面を開く（app.js の route から呼ばれる）。 */
async function stOpen(params) {
  const p = params || {};
  stNodeId = p.node || null;
  stData = null;
  stNode = null;
  stMode = stLoadMode();
  await ensurePaperProjects();
  fillSegment("sci-seg", "tree");
  updateProjectChips();
  stSyncSplit();
  if (!selectedPaperProject()) {
    stRenderNoProject();
    return;
  }
  await stReload();
}

/** 2 列 / 1 列のクラス（style.css の .sai-split の決まりに合わせる）。 */
function stSyncSplit() {
  const body = document.getElementById("sci-body");
  const list = document.getElementById("sci-list");
  const board = document.getElementById("sci-board");
  if (body) body.classList.add("sai-split");
  if (list) list.classList.add("sai-split-list");
  if (board) board.classList.add("sai-split-main");
}

// --- 表示の切り替え（一覧 / 根の図 / 表） ---

let stMode = "list";

function stRenderModes() {
  const box = document.getElementById("sci-modes");
  if (!box) return;
  box.innerHTML = "";
  const seg = stEl("div", "dash-seg small sci-modes-seg");
  for (const [key, label] of ST_MODES) {
    const b = stEl("button", stMode === key ? "active" : "", label);
    b.type = "button";
    b.dataset.mode = key;
    b.addEventListener("click", () => {
      stMode = key;
      stSaveMode(key);
      stRenderModes();
      stRenderList();
      stRenderBoard(); // 表のときは選んだ節点の表を出す
    });
    seg.appendChild(b);
  }
  box.appendChild(seg);
}

/** U21 の関数が無ければ、その旨の 1 行を出す。 */
function stNotYet(what) {
  return stEl("p", "sci-note", `${what}の画面はまだありません。`);
}

// ============================================================================
// 上の帯（種の文・関門・負債）
// ============================================================================

/** 画面的全部（上の帯・切り替え・一覧・板）を描き直す。 */
function stRender() {
  stSyncSplit();
  stRenderTop();
  stRenderModes();
  stRenderList();
  stRenderBoard();
}

function stRenderTop() {
  const box = document.getElementById("sci-top");
  if (!box) return;
  box.innerHTML = "";
  const s = stData;
  if (!s) return;
  if (!s.initialized) {
    box.appendChild(stEmpty({
      icon: "🌱",
      title: "研究方針の木はまだありません",
      body: "研究の型を選んで始めます。",
      action: { label: "始める", onClick: () => { void stStartTree(); } },
    }));
    return;
  }
  box.appendChild(stSeedBand(s));
  box.appendChild(stGateBand(s));
  box.appendChild(stDebtBand(s));
}

/** 種の文（空欄の部分を押すとその項目を開く）。 */
function stSeedBand(status) {
  const band = stEl("div", "sci-band");
  const head = stLine("sci-band-head");
  head.appendChild(stEl("h2", "sci-band-title", "種の文"));
  const term = stTerm("sci_seed");
  if (term) head.appendChild(term);
  band.appendChild(head);
  const p = stEl("p", "sci-seed");
  const pieces = stSeedPieces(status);
  if (!pieces.length) {
    p.appendChild(stEl("span", "sci-seed-text", "（まだ種の文はありません）"));
  }
  for (const piece of pieces) {
    if (piece.canOpen) {
      const b = stEl("button", "sci-seed-blank", piece.text);
      b.type = "button";
      b.setAttribute("aria-label", `${piece.text} を埋める（項目 ${piece.node}）`);
      b.addEventListener("click", () => { void stSelect(piece.node); });
      p.appendChild(b);
    } else {
      const span = stEl("span", piece.tentative ? "sci-seed-tentative" : "sci-seed-text", piece.text);
      p.appendChild(span);
    }
  }
  band.appendChild(p);
  return band;
}

/** 関門（押すと一覧。全部通っていれば種の札の釦を出す）。 */
function stGateBand(status) {
  const band = stEl("div", "sci-band");
  const head = stLine("sci-band-head");
  head.appendChild(stEl("h2", "sci-band-title", "関門"));
  const term = stTerm("sci_gate");
  if (term) head.appendChild(term);
  band.appendChild(head);
  const g = stGateInfo(status);
  const b = stEl("button", "sci-gate", g.text);
  b.type = "button";
  b.addEventListener("click", () => stOpenGateSheet(g));
  head.appendChild(b);
  if (g.all && typeof seOpenCard === "function") {
    const seedBtn = stBtn("種の札を作る", () => {
      seOpenCard(null, { block: "sci-seed", purpose: "関門を確かめて種の札を作る", expect: "種の札" });
    }, "primary");
    head.appendChild(seedBtn);
  }
  return band;
}

/** 関門の一覧を下から出す。 */
function stOpenGateSheet(gate) {
  const body = stEl("div", "sci-gate-list");
  if (!gate.list.length) {
    body.appendChild(stEl("p", "sci-note", "関門の行はまだありません。"));
  }
  for (const g of gate.list) {
    const row = stLine("sci-gate-row");
    row.appendChild(stChip(g.passed ? "通った" : "まだ", g.passed ? "ok" : "warn"));
    const main = stEl("div", "sci-line-main");
    main.appendChild(stEl("p", "sci-line-title", g.name || "（名前のない関門）"));
    if (g.when) main.appendChild(stEl("p", "sci-line-sub", `通る条件: ${g.when}`));
    row.appendChild(main);
    body.appendChild(row);
  }
  if (typeof saiSheet === "function") saiSheet({ title: "関門", body });
}

/** 負債（4 つの数。0 のものは薄くする）。 */
function stDebtBand(status) {
  const band = stEl("div", "sci-band");
  const head = stLine("sci-band-head");
  head.appendChild(stEl("h2", "sci-band-title", "負債"));
  const term = stTerm("sci_debt");
  if (term) head.appendChild(term);
  band.appendChild(head);
  const row = stLine("sci-debts");
  for (const d of stDebtChips(status)) {
    const text = `${d.label} ${d.count}`;
    if (d.hash && typeof location !== "undefined") {
      const b = stEl("button", `sci-debt sai-chip ${d.dim ? "muted" : "warn"}`, text);
      b.type = "button";
      b.addEventListener("click", () => { location.hash = d.hash; });
      row.appendChild(b);
    } else {
      row.appendChild(stEl("span", `sci-debt sai-chip ${d.dim ? "muted" : "warn"}`, text));
    }
  }
  band.appendChild(row);
  return band;
}

/** 研究の型を選んで木を始める。 */
async function stStartTree() {
  const body = stEl("div", "sci-types");
  for (const [id, name, explain] of ST_TYPES) {
    const b = stEl("button", "sci-type");
    b.type = "button";
    b.appendChild(stEl("span", "sci-type-name", name));
    b.appendChild(stEl("span", "sci-type-text", explain));
    b.addEventListener("click", () => {
      if (typeof saiSheet === "function") saiOverlayShut();
      void stInitTree(id, name);
    });
    body.appendChild(b);
  }
  if (typeof saiSheet === "function") saiSheet({ title: "研究の型を選ぶ", body });
}

/** 木始める（確認をして POST /init）。 */
async function stInitTree(typeId, typeName) {
  if (typeof saiConfirm === "function") {
    const ok = await saiConfirm({
      title: "始めますか",
      body: `${typeName || "この型"}で研究方針の木を始めます。`,
      ok: "始める",
    });
    if (!ok) return;
  }
  try {
    await paperApi(stApi("/init"), { method: "POST", body: { type: typeId } });
  } catch (e) {
    if (typeof saiToast === "function") saiToast((e && e.message) || "始められませんでした", "error");
    return;
  }
  stNodeId = null;
  stSetHash();
  await stReload();
}

// ============================================================================
// 項目の一覧（R-U3・R-U6）
// ============================================================================

function stRenderList() {
  const box = document.getElementById("sci-list");
  if (!box) return;
  box.innerHTML = "";
  if (stMode === "graph") {
    if (typeof sgRenderGraph === "function" && stData) {
      sgRenderGraph(box, stData, (nodeId) => { void stSelect(nodeId); });
    } else {
      box.appendChild(stNotYet("根の図"));
    }
    return;
  }
  if (stMode === "table") {
    if (typeof sgRenderTable === "function" && stNode) {
      sgRenderTable(box, stNode);
    } else {
      box.appendChild(stNotYet("候補の表"));
    }
    return;
  }
  const rows = stRows(stData);
  const head = stLine("sci-list-head");
  head.appendChild(stEl("span", "sci-list-head-title", "項目"));
  const term = stTerm("sci_blank");
  if (term) head.appendChild(term);
  box.appendChild(head);
  if (!rows.length) {
    const next = stNextLines(stData, 3);
    const body = next.length ? next.join("／") : "項目はまだありません。";
    box.appendChild(stEmpty({ icon: "🌱", title: "項目がまだありません", body }));
    return;
  }
  for (const r of rows) {
    box.appendChild(stListRow(r));
  }
}

/** 一覧の 1 行。押すとその項目の板が開く。 */
function stListRow(r) {
  const b = stEl("button", "sci-row" + (r.id && r.id === stNodeId ? " active" : ""));
  b.type = "button";
  b.style.setProperty("--sci-indent", String(r.indent));
  const head = stLine("sci-row-head");
  const mark = stEl("span", `sci-mark ${r.cls}`, r.mark);
  mark.setAttribute("aria-label", r.label);
  head.appendChild(mark);
  if (r.fixed) head.appendChild(stEl("span", "sci-pin", "📌"));
  head.appendChild(stEl("span", "sci-id", r.id));
  head.appendChild(stEl("span", "sci-row-title", r.title || "（題のない項目）"));
  b.appendChild(head);
  const sub = stLine("sci-row-sub");
  if (r.blank) sub.appendChild(stChip(r.blank, r.blank === "対立" ? "danger" : "warn"));
  for (const m of r.marks) sub.appendChild(stChip(ST_NODE_MARKS[m], "muted"));
  if (r.papers > 0) sub.appendChild(stChip(`論文 ${r.papers}`, "muted"));
  if (r.pending > 0) sub.appendChild(stChip(`仕分け ${r.pending}`, "warn"));
  if (sub.childNodes.length) b.appendChild(sub);
  b.addEventListener("click", () => { void stSelect(r.id); });
  return b;
}

// ============================================================================
// 項目の板
// ============================================================================

/** 板を出す（広い画面は右の欄、狭い画面は下から）。 */
function stRenderBoard() {
  const box = document.getElementById("sci-board");
  if (box && saiIsWide()) {
    box.innerHTML = "";
    if (stSheet) stCloseSheet();
    if (!stNode) {
      box.appendChild(stEmpty({
        icon: "👈",
        title: "項目を選んでください",
        body: "左の一覧から項目を選ぶと、ここにその項目の板が出ます。",
      }));
      return;
    }
    box.appendChild(stBuildBoard(stNode));
    return;
  }
  if (box) box.innerHTML = "";
  if (!stNode) {
    stCloseSheet();
    return;
  }
  // 狭い画面では下から出す。同じ節点なら中身だけ入れ替える
  if (typeof saiSheet !== "function") return;
  const node = stNode.node || {};
  if (stSheet && stSheetFor === stNodeId && stSheetBody) {
    stSheetBody.innerHTML = "";
    stSheetBody.appendChild(stBuildBoard(stNode));
    return;
  }
  stCloseSheet();
  const body = stEl("div", "sci-sheet-body");
  body.appendChild(stBuildBoard(stNode));
  stSheet = saiSheet({ title: `${node.id || stNodeId || ""} ${node.title || ""}`.trim(), body });
  stSheetFor = stNodeId;
  stSheetBody = body;
}

/** 狭い画面の板を閉じる。 */
function stCloseSheet() {
  if (stSheet && typeof stSheet.close === "function") stSheet.close();
  stSheet = null;
  stSheetFor = null;
  stSheetBody = null;
}

/** 節の見出し。 */
function stSec(title, termKey) {
  const sec = stEl("section", "sci-sec");
  if (title) {
    const head = stLine("sci-sec-head");
    head.appendChild(stEl("h3", "sci-sec-title", title));
    const term = termKey ? stTerm(termKey) : null;
    if (term) head.appendChild(term);
    sec.appendChild(head);
  }
  return sec;
}

/** 板の本体（上から下の順）。 */
function stBuildBoard(detail) {
  const d = detail || {};
  const node = d.node || {};
  const box = stEl("div", "sci-card");
  box.appendChild(stBoardHead(d, node));
  box.appendChild(stBoardNext(d, node));
  box.appendChild(stBoardQuestions(d, node));
  box.appendChild(stBoardOptions(d, node));
  box.appendChild(stBoardFindings(d, node));
  box.appendChild(stBoardLinks(d, node));
  box.appendChild(stBoardFunnel(d));
  box.appendChild(stBoardPending(d));
  box.appendChild(stBoardActions(d));
  box.appendChild(stBoardDecisions(d));
  return box;
}

/** Markdown を入れる（renderMarkdown が無いときはただの文章にする）。 */
function stMd(src, cls) {
  if (typeof renderMarkdown === "function") {
    const div = document.createElement("div");
    div.className = `sci-md ${cls || ""}`.trim();
    div.innerHTML = renderMarkdown(src);
    return div;
  }
  return stEl("p", `sci-md ${cls || ""}`.trim(), String(src == null ? "" : src));
}

/** 1. 見出し */
function stBoardHead(d, node) {
  const sec = stSec("", "sci_blank");
  const head = stEl("div", "sci-node-head");
  head.appendChild(stMarkChip("item", node.state));
  head.appendChild(stEl("span", "sci-id", node.id || ""));
  head.appendChild(stEl("h2", "sci-node-title", node.title || "（題のない項目）"));
  sec.appendChild(head);
  const chips = stLine("sci-node-chips");
  for (const t of Array.isArray(node.types) ? node.types : []) {
    const label = (ST_ITEM_TYPES.find((x) => x[0] === t) || [])[1] || t;
    chips.appendChild(stChip(label, "muted"));
  }
  if (node.fixed) chips.appendChild(stChip("固定", "muted"));
  if (node.blank) chips.appendChild(stChip(ST_BLANKS[node.blank] || node.blank, "warn"));
  for (const m of Array.isArray(node.marks) ? node.marks : []) {
    if (ST_NODE_MARKS[m]) chips.appendChild(stChip(ST_NODE_MARKS[m], "muted"));
  }
  if (chips.childNodes.length) sec.appendChild(chips);
  sec.appendChild(stOpRow(d, node));
  if (node.body) sec.appendChild(stMd(node.body));
  if (d.guide) sec.appendChild(stGuide(d.guide));
  return sec;
}

/** 項目ガイド（説明はたたむ。埋まった条件を出す）。 */
function stGuide(guide) {
  const g = guide || {};
  const det = document.createElement("details");
  det.className = "sci-guide";
  const sum = document.createElement("summary");
  sum.textContent = `項目ガイド${g.name ? `（${g.name}）` : ""}`;
  det.appendChild(sum);
  if (g.explain) det.appendChild(stMd(g.explain));
  if (g.condition_text) {
    det.appendChild(stChip(g.condition_met ? "満たした" : "まだ", g.condition_met ? "ok" : "warn"));
    det.appendChild(stEl("p", "sci-note", `埋まった条件: ${g.condition_text}`));
  }
  for (const [label, arr] of [["問い", g.questions], ["つまずき", g.pitfalls]]) {
    if (!Array.isArray(arr) || !arr.length) continue;
    det.appendChild(stEl("p", "sci-note", label));
    const ul = document.createElement("ul");
    ul.className = "sci-bullets";
    for (const t of arr) ul.appendChild(stEl("li", null, String(t)));
    det.appendChild(ul);
  }
  return det;
}

/** 板の上の操作の釦（U20 の seOpenAction が無いときは出さない）。 */
function stOpRow(d, node) {
  if (typeof seOpenAction !== "function") return document.createDocumentFragment();
  const row = stLine("sci-ops");
  const id = node.id || null;
  const has = (t) => Array.isArray(node.types) && node.types.indexOf(t) >= 0;
  const add = (label, op) => row.appendChild(stBtn(label, () => seOpenAction(op, id, d, null)));
  if (node.state === "excluded") add("再開", "reopen");
  else {
    add("決める", "decide");
    if (node.state !== "open") add("再開", "reopen");
    add("対象外", "exclude_item");
  }
  if (has("choose") || has("list")) add("候補を足す", "add_option");
  add("子の項目を足す", "add_child");
  add("問いを足す", "add_question");
  add("知見を足す", "add_finding");
  add("結ぶ", "link");
  add("文を直す", "edit_text");
  add("線を引く", "add_edge");
  return row;
}

/** 2. 次の一手 */
function stBoardNext(d, node) {
  const sec = stSec("次の一手");
  const blocks = Array.isArray(d.blocks) ? d.blocks.slice(0, 3) : [];
  if (!blocks.length) {
    sec.appendChild(stEl("p", "sci-note", "次の一手はありません。"));
    return sec;
  }
  const id = node.id || null;
  for (const b of blocks) {
    const row = stLine("sci-block");
    const main = stEl("div", "sci-line-main");
    main.appendChild(stEl("p", "sci-line-title", b.purpose || b.block || ""));
    const sub = [b.expect, b.why].filter(Boolean).join("　");
    if (sub) main.appendChild(stEl("p", "sci-line-sub", sub));
    row.appendChild(main);
    const can = typeof seOpenCard === "function";
    row.appendChild(stBtn(can ? "カードを作る" : `${b.block || "カード"} はまだありません`, can
      ? () => seOpenCard(id, b)
      : null));
    sec.appendChild(row);
  }
  const canRead = typeof srOpenReadCard === "function";
  const read = stBtn(canRead ? "読む札を作る" : "読む札はまだありません", canRead
    ? () => srOpenReadCard(id, d)
    : null);
  read.classList.add("primary");
  sec.appendChild(read);
  return sec;
}

/** 3. 問い */
function stBoardQuestions(d, node) {
  const sec = stSec("問い");
  const list = Array.isArray(d.questions) ? d.questions : [];
  if (!list.length) {
    sec.appendChild(stEl("p", "sci-note", "問いはまだありません。"));
    return sec;
  }
  for (const q of list) {
    const open = q.state === "open";
    const row = stLine("sci-item");
    const main = stEl("div", "sci-line-main");
    main.appendChild(stEl("span", "sci-id", q.id || ""));
    main.appendChild(stEl("p", "sci-line-title", q.text || ""));
    if (q.answer) main.appendChild(stEl("p", "sci-line-sub", `答え: ${q.answer}`));
    row.appendChild(main);
    row.appendChild(stChip(open ? "答え待ち" : (q.answer ? "答えた" : "閉じた"), open ? "warn" : "muted"));
    if (typeof seOpenAction === "function" && q.id) {
      if (open) {
        row.appendChild(stBtn("答える", () => seOpenAction("answer_question", node.id || null, d, q.id)));
        row.appendChild(stBtn("取り下げる", () => seOpenAction("withdraw_question", node.id || null, d, q.id)));
      }
    }
    sec.appendChild(row);
  }
  return sec;
}

/** 4. 候補（表。表示の切り替えが「表」なら U21 の表にする） */
function stBoardOptions(d, node) {
  const sec = stSec("候補");
  const list = Array.isArray(d.options) ? d.options : [];
  if (stMode === "table" && typeof sgRenderTable === "function") {
    const box = stEl("div", "sci-table-box");
    sgRenderTable(box, d);
    sec.appendChild(box);
    return sec;
  }
  if (!list.length) {
    sec.appendChild(stEl("p", "sci-note", "候補はまだありません。"));
    return sec;
  }
  const axes = Array.isArray(d.axes) ? d.axes : [];
  const wrap = stEl("div", "sci-table-wrap");
  const table = document.createElement("table");
  table.className = "sci-table";
  const thead = document.createElement("thead");
  const hrow = document.createElement("tr");
  for (const h of ["候補", "状態", "支持", "反証", "論文"]) {
    hrow.appendChild(stEl("th", null, h));
  }
  thead.appendChild(hrow);
  table.appendChild(thead);
  const tbody = document.createElement("tbody");
  for (const o of list) {
    const tr = document.createElement("tr");
    tr.className = o.id === stNodeId ? "active" : "";
    const name = document.createElement("td");
    name.appendChild(stEl("span", "sci-id", o.id || ""));
    name.appendChild(stEl("span", "sci-cell-title", o.title || ""));
    if (o.body) name.appendChild(stMd(o.body, "sci-cell-body"));
    tr.appendChild(name);
    const state = document.createElement("td");
    state.appendChild(stMarkChip("option", o.state));
    tr.appendChild(state);
    tr.appendChild(stEl("td", "sci-num", String(Number(o.support) || 0)));
    tr.appendChild(stEl("td", "sci-num", String(Number(o.refute) || 0)));
    tr.appendChild(stEl("td", "sci-num", String(Number(o.papers) || 0)));
    if (typeof seOpenAction === "function" && o.id) {
      const ops = document.createElement("td");
      ops.className = "sci-cell-ops";
      ops.appendChild(stBtn("選ぶ", () => seOpenAction("decide", node.id || null, d, o.id)));
      ops.appendChild(stBtn("却下", () => seOpenAction("option_state", node.id || null, d, { option: o.id, state: "rejected" })));
      ops.appendChild(stBtn("休眠", () => seOpenAction("option_state", node.id || null, d, { option: o.id, state: "dormant" })));
      ops.appendChild(stBtn("戻す", () => seOpenAction("option_state", node.id || null, d, { option: o.id, state: "candidate" })));
      tr.appendChild(ops);
    }
    tbody.appendChild(tr);
  }
  table.appendChild(tbody);
  wrap.appendChild(table);
  sec.appendChild(wrap);
  if (axes.length) {
    const p = stEl("p", "sci-note", `価値の軸: ${axes.join("／")}`);
    sec.appendChild(p);
  }
  return sec;
}

/** 5. 知見 */
function stBoardFindings(d, node) {
  const sec = stSec("知見", "sci_finding");
  const list = Array.isArray(d.findings) ? d.findings : [];
  if (!list.length) {
    sec.appendChild(stEl("p", "sci-note", "知見はまだありません。"));
    return sec;
  }
  for (const f of list) {
    const row = stLine("sci-item sci-finding");
    const main = stEl("div", "sci-line-main");
    const head = stLine("sci-finding-head");
    head.appendChild(stEl("span", "sci-id", f.id || ""));
    head.appendChild(stChip(ST_POLARITY[f.polarity] || "中立", f.polarity === "refute" ? "danger" : "muted"));
    if (f.node && f.node !== (node.id || "")) head.appendChild(stEl("span", "sci-id", f.node));
    if (f.auto) head.appendChild(stEl("span", "sci-auto", "🤖 自動で入りました"));
    main.appendChild(head);
    main.appendChild(stEl("p", "sci-line-text", f.text || ""));
    if (Array.isArray(f.cites) && f.cites.length) {
      main.appendChild(stEl("p", "sci-line-sub", `出典: ${f.cites.join("、")}`));
    }
    row.appendChild(main);
    if (typeof seOpenAction === "function" && f.id) {
      row.appendChild(stBtn("外す", () => seOpenAction("remove_finding", node.id || null, d, f.id)));
    }
    sec.appendChild(row);
  }
  return sec;
}

/** 6. 結び付き */
function stBoardLinks(d, node) {
  const sec = stSec("結び付き");
  const list = Array.isArray(d.links) ? d.links : [];
  if (!list.length) {
    sec.appendChild(stEl("p", "sci-note", "結び付きはまだありません。"));
    return sec;
  }
  for (const l of list) {
    const target = String(l.target || "");
    const colon = target.indexOf(":");
    const kind = colon > 0 ? target.slice(0, colon) : "";
    const row = stLine("sci-item sci-link");
    const main = stEl("div", "sci-line-main");
    const head = stLine("sci-link-head");
    head.appendChild(stEl("span", "sci-id", l.id || ""));
    head.appendChild(stChip(ST_LINK_KINDS[kind] || kind || "結び付き", "muted"));
    if (kind === "paper" && l.role) head.appendChild(stChip(ST_ROLES[l.role] || l.role, "muted"));
    if (kind === "paper" && l.read_state) head.appendChild(stChip(ST_READ_STATES[l.read_state] || l.read_state, l.read_state === "unread" ? "warn" : "ok"));
    if (l.auto) head.appendChild(stEl("span", "sci-auto", "🤖 自動で入りました"));
    main.appendChild(head);
    const title = l.title || target;
    if (kind === "paper" && typeof location !== "undefined") {
      const b = stEl("button", "sci-link-title", title);
      b.type = "button";
      const paperId = target.slice(colon + 1);
      b.addEventListener("click", () => { location.hash = stPaperHash(paperId); });
      main.appendChild(b);
    } else {
      main.appendChild(stEl("p", "sci-line-title", title));
    }
    if (l.reason) main.appendChild(stEl("p", "sci-line-sub", l.reason));
    row.appendChild(main);
    if (typeof seOpenAction === "function" && l.id && kind === "paper") {
      row.appendChild(stBtn("読み具合", () => seOpenAction("set_read_state", node.id || null, d, l.id)));
      row.appendChild(stBtn("外す（墓標）", () => seOpenAction("exclude", node.id || null, d, l.id)));
    }
    sec.appendChild(row);
  }
  return sec;
}

/** 論文の画面を開く住所（papers.js の paperHash と同じ形）。 */
function stPaperHash(paperId) {
  const name = selectedPaperProject();
  if (typeof paperHash === "function") return paperHash(name, paperId);
  return name
    ? `#/paper/${encodeURIComponent(name)}/${encodeURIComponent(paperId)}`
    : `#/paper/${encodeURIComponent(paperId)}`;
}

/** 7. 文献の流れ */
function stBoardFunnel(d) {
  const sec = stSec("文献の流れ");
  const f = d.funnel || {};
  const row = stLine("sci-funnel");
  for (const [label, key] of [["検索", "searched"], ["選別", "screened"], ["読む", "read"], ["反映", "reflected"]]) {
    const box = stEl("div", "sci-funnel-cell");
    box.appendChild(stEl("span", "sci-funnel-num", String(Math.max(0, Number(f[key]) || 0))));
    box.appendChild(stEl("span", "sci-funnel-label", label));
    row.appendChild(box);
  }
  sec.appendChild(row);
  return sec;
}

/** 8. 仕分け待ち */
function stBoardPending(d) {
  const sec = stSec("仕分け待ち", "sci_triage");
  const list = Array.isArray(d.pending) ? d.pending : [];
  if (!list.length) {
    sec.appendChild(stEl("p", "sci-note", "仕分け待ちはありません。"));
    return sec;
  }
  if (typeof seProposalRow !== "function") {
    sec.appendChild(stNotYet("仕分けの欄"));
    for (const p of list) {
      sec.appendChild(stEl("p", "sci-line-sub", `${p.id || ""} ${p.op || ""}　${p.reason || ""}`));
    }
    return sec;
  }
  for (const p of list) sec.appendChild(seProposalRow(p, () => { void stReload(); }));
  return sec;
}

/** 9. 行動の履歴 */
function stBoardActions(d) {
  const sec = stSec("行動の履歴");
  const list = Array.isArray(d.actions) ? d.actions : [];
  if (!list.length) {
    sec.appendChild(stEl("p", "sci-note", "行動の履歴はありません。"));
    return sec;
  }
  for (const a of list) {
    const row = stLine("sci-item");
    const main = stEl("div", "sci-line-main");
    const head = stLine("sci-link-head");
    head.appendChild(stEl("span", "sci-id", `#${a.run_id == null ? "" : a.run_id}`));
    head.appendChild(stChip(a.block || "", "muted"));
    if (a.owner) head.appendChild(stChip(a.owner, "muted"));
    main.appendChild(head);
    main.appendChild(stEl("p", "sci-line-text", a.purpose || ""));
    if (a.created) main.appendChild(stEl("p", "sci-line-sub", a.created));
    row.appendChild(main);
    if (a.run_id != null && typeof location !== "undefined") {
      const b = stBtn("実行を開く", () => {
        location.hash = `#/project/${encodeURIComponent(selectedPaperProject())}/list/${encodeURIComponent(a.run_id)}/task`;
      });
      row.appendChild(b);
    }
    sec.appendChild(row);
  }
  return sec;
}

/** 10. 決定の記録（新しい順） */
function stBoardDecisions(d) {
  const sec = stSec("決定の記録");
  const rows = stDecisionRows(d.decisions);
  if (!rows.length) {
    sec.appendChild(stEl("p", "sci-note", "決定の記録はまだありません。"));
    return sec;
  }
  for (const dec of rows) {
    const row = stLine("sci-item");
    const main = stEl("div", "sci-line-main");
    main.appendChild(stEl("span", "sci-id", dec.action || ""));
    if (dec.option) main.appendChild(stEl("p", "sci-line-title", `${dec.option} を選んだ`));
    if (dec.reason) main.appendChild(stEl("p", "sci-line-text", dec.reason));
    const bits = [];
    if (Array.isArray(dec.evidence) && dec.evidence.length) bits.push(`根拠: ${dec.evidence.join("、")}`);
    if (Array.isArray(dec.rejected) && dec.rejected.length) {
      bits.push(`却下: ${dec.rejected.map((r) => r.option).filter(Boolean).join("、")}`);
    }
    if (dec.counter_searched === false) bits.push("反対の意見は探していません");
    if (dec.actor) bits.push(dec.actor);
    if (dec.created_at) bits.push(dec.created_at);
    if (bits.length) main.appendChild(stEl("p", "sci-line-sub", bits.join("　")));
    row.appendChild(main);
    sec.appendChild(row);
  }
  return sec;
}

// --- 読めなかったとき / 論文の置き場が無いとき ---

function stRenderError(e) {
  stCloseSheet();
  const top = document.getElementById("sci-top");
  const modes = document.getElementById("sci-modes");
  const list = document.getElementById("sci-list");
  if (top) top.innerHTML = "";
  if (modes) modes.innerHTML = "";
  if (list) {
    list.innerHTML = "";
    list.appendChild(stEmpty({
      icon: "⚠️",
      title: "研究方針を読めませんでした",
      body: (e && e.message) || "もう一度読み込んでください。",
    }));
  }
}

function stRenderNoProject() {
  stCloseSheet();
  const top = document.getElementById("sci-top");
  const modes = document.getElementById("sci-modes");
  const list = document.getElementById("sci-list");
  if (top) top.innerHTML = "";
  if (modes) modes.innerHTML = "";
  if (!list) return;
  list.innerHTML = "";
  const msg = typeof NO_PROJECT_MSG === "string" ? NO_PROJECT_MSG : "論文の置き場があるプロジェクトがありません";
  list.appendChild(stEmpty({ icon: "📁", title: "論文の置き場がありません", body: msg }));
}

// ============================================================================
// 画面の登録（window.SaiRoutes・app.js §3.2-2）
// ============================================================================

if (typeof window !== "undefined" && typeof document !== "undefined") {
  (window.SaiRoutes = window.SaiRoutes || []).push({
    // 研究方針の木（#/papers/tree と #/papers/tree/N12）
    match(hash) {
      return stParseHash(hash);
    },
    view: "view-sci-tree",
    tab: "papers",
    hideTabbar: false,
    interval: 0, // 定期更新はしない（↻ で読み直す）
    async open(params) {
      await stOpen(params);
    },
    async poll() {},
  });

  // 一覧側のボタン（論文タブのどの画面でも同じ物を使う）
  const stReloadBtn = document.getElementById("sci-reload");
  if (stReloadBtn) stReloadBtn.addEventListener("click", () => { void stReload(); });
  // プロジェクト名のボタン（sci-project）は papers.js 側で openProjectSheet を登録している
}

// --- node からの読み込み（tests/test_scitree.js） ---
if (typeof module !== "undefined" && module.exports) {
  module.exports = {
    stParseHash,
    stStateMark,
    stRows,
    stSeedPieces,
    stGateInfo,
    stDebtChips,
    stNextLines,
    stDecisionRows,
    stPaperHash,
    ST_MODES,
    ST_TYPES,
    ST_ITEM_TYPES,
    ST_ITEM_MARKS,
    ST_OPTION_MARKS,
    ST_BLANKS,
    ST_NODE_MARKS,
    ST_DEBTS,
  };
}
