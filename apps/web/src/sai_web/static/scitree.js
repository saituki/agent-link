"use strict";
// research-tree の画面（Q・O・F・E・S の木）。住所は #/research/tree と #/research/tree/<ID>。
//
// 上の帯に Next Action（先頭 3 件）・History（全 Node・折りたたみ）・注意、左に木の一覧、真ん中に
// research-tree の図（scigraph.js）、右に選んだ Node の板（何も選ばなければ出さない）を出す。
// SEED（ID `SEED`）は DB の行を持たず、status.seed から板を作る。
// 狭い画面（幅 390px）の板は下から出る（saiSheet）。
//
// 読むのは sai-paper の `phone sci status` / `types` / `nodes/<ID>` だけ（API は U17）。
// 操作は sciedit.js（seOpenAction・seOpenCard・seQuick・seOpLabel・seProposalRow）、図は
// scigraph.js（sgRenderGraph・sgMarkSelected・sgCenterOn）の関数を typeof で確かめてから呼ぶ。
// DOM を使わない純関数は末尾で node に渡す（tests/test_scitree.js）。
// 画面の名詞（Q・O・F・E・Answer・Issue・SEED・SPROUT・Objectivity・Logicality など）は英語のまま出す。

// --- 定数 ---

// RESEARCH_TYPE（/sci/types が返せば、その説明で置き換える）
const ST_TYPES = [
  ["problem", "PROBLEM_DRIVEN", "最初に課題（困っていること）を決めて、何をなぜやるのかから始める研究です。"],
  ["phenomenon", "PHENOMENON_DRIVEN", "先に現象を捉え、「なぜ起きるのか」を問う研究です。"],
  ["method", "METHOD_DRIVEN", "先に手法を決め、その手法で何が分かるようになるかを考える研究です。"],
  ["data", "DATA_DRIVEN", "手にあるデータから問いを立てる研究です。"],
  ["improve", "IMPROVE_DRIVEN", "いまのものの弱いところから始めた研究です。"],
];
const ST_LOGIC = { ok: ["OK", "ok"], contradiction: ["CONTRADICTION", "warn"], unchecked: ["UNCHECKED", "dim"] };
const ST_KIND_LABEL = { answer: "Answer", issue: "Issue" };
// 左の一覧の絞り込みで切り替えられる種類（図の札と同じ 5 つ）
const ST_ALL_KINDS = ["Q", "O", "E", "F", "S"];
// 境界線をドラッグしたときの幅の範囲（px）
const ST_SPLIT_LIMITS = { list: [200, 600], board: [280, 720] };
// 図の最小の幅（px）。これより狭くしない。
const ST_GRAPH_MIN = 300;
// 境界線 2 本ぶんの幅（図の幅を計算するとき引く）
const ST_SPLIT_BAR = 12;
// SPROUT の PATH に入れられる O の数（sciedit.js の seBuildOp の上限と同じ）
const ST_SPROUT_PATH_MAX = 20;
// 板に出す操作（表示名と送り方は sciedit.js の seOpLabel・seOpenAction・seQuick）
const ST_NODE_OPS = {
  Q: ["add_q", "add_o"],
  O: ["add_q", "add_f", "copy"],
  F: [],
  E: ["add_f", "add_e"],
  // S（SPROUT）は BRANCH を足せて、題と本文を直せる。親が無いので Edit / Delete もここに出す。
  S: ["add_e", "edit", "remove"],
};

// --- 画面の状態 ---

let stData = null;      // 最後に読んだ status
let stTypesData = null; // 最後に読んだ /sci/types
let stNode = null;      // 選んだ Node の NodeDetail
let stNodeId = null;    // 今開いている Node の ID
let stSeedSel = false;  // SEED が選ばれているか（SEED は DB の行を持たない）
let stSheet = null;     // 狭い画面の板（saiSheet が返した物）
let stSheetBody = null; // 板の中の入れ物
let stSheetFor = null;  // 板が出している物の鍵
let stSprout = null;    // SPROUT 作成中の下書き {title, body, paths, error}。null なら作成中ではない
let stSproutBusy = false; // 送信中か（二重に送らない）
let stHistoryOpen = false; // 上の帯の History を開いているか（描き直しても保つ）
let stListOpen = false; // 狭い画面で一覧を出しているか（出している間は図を隠す）
let stFolded = new Set();   // 左の一覧で畳んでいる Node の ID
let stSplit = { list: 320, board: 420 }; // 境界線で変えた幅（px）
let stSplitInit = false;    // 境界線の仕掛けを付けたか
let stFilterInit = false;   // 一番上の絞り込みを作ったか
const stFilter = { word: "", kinds: new Set(ST_ALL_KINDS) }; // 一番上の絞り込み（覚えない）

// ============================================================================
// DOM を使わない純関数（tests/test_scitree.js）
// ============================================================================

/** 住所を読む（`#/research/tree` と `#/research/tree/<ID>`）。合わなければ null。 */
function stParseHash(hash) {
  const m = /^#\/research\/tree(?:\/(SEED|[QOFES][1-9][0-9]{0,8}))?$/.exec(String(hash || ""));
  if (!m) return null;
  return m[1] ? { id: m[1] } : {};
}

/**
 * 古い住所を新しい住所に直す（U14）。古い住所でなければ null。
 *
 * iPhone の通知・ブックマーク・`sessionStorage` の「最後に開いた住所」に古い住所が
 * 残っていても開けるようにするためである。`?` の後ろと ID はそのまま写す。
 *   #/papers/tree[/<ID>]  → #/research/tree[/<ID>]
 *   #/papers/triage       → #/research/tree
 *   #/papers/read/<run>   → #/research/read/<run>
 *   #/mlflow…             → #/research/experiments…
 *   #/papers/canvas・#/canvas/… → #/research/tree
 */
function stLegacyHash(hash) {
  const h = String(hash || "");
  if (h === "#/papers/tree" || h.startsWith("#/papers/tree/")) {
    return "#/research/tree" + h.slice("#/papers/tree".length);
  }
  if (h === "#/papers/triage") return "#/research/tree";
  if (h.startsWith("#/papers/read/")) {
    return "#/research/read" + h.slice("#/papers/read".length);
  }
  if (h === "#/mlflow" || h.startsWith("#/mlflow/") || h.startsWith("#/mlflow?")) {
    return "#/research/experiments" + h.slice("#/mlflow".length);
  }
  if (h === "#/papers/canvas" || h.startsWith("#/canvas/")) return "#/research/tree";
  return null;
}

/** PATH の O を足す／外す。O 以外・重複・21 個目は足さない。 */
function stSproutToggle(paths, id) {
  const list = Array.isArray(paths) ? paths.slice() : [];
  const i = list.indexOf(id);
  if (i >= 0) { list.splice(i, 1); return list; }
  if (!/^O[1-9][0-9]{0,8}$/.test(String(id)) || list.length >= ST_SPROUT_PATH_MAX) return list;
  list.push(id);
  return list;
}

/** パネルに出す PATH の行（O の ID・Issue か・文）。status.nodes に無い ID は文なしで残す。 */
function stSproutRows(paths, status) {
  const nodes = status && Array.isArray(status.nodes) ? status.nodes : [];
  const byId = new Map(nodes.filter((n) => n && n.id != null).map((n) => [String(n.id), n]));
  return (Array.isArray(paths) ? paths : []).map((id) => {
    const n = byId.get(String(id));
    return { id, issue: !!n && /issue/i.test(n.answer_kind || ""), text: n ? String(n.text || n.title || "") : "" };
  });
}

/** 状態の 1 行。Issue の O が 0 個なら、足りないことを言う。 */
function stSproutSummary(paths, status) {
  const rows = stSproutRows(paths, status);
  const issues = rows.filter((r) => r.issue).length;
  if (!issues) return "Issue の O がまだありません";
  return `PATH ${rows.length} 個（Issue の O ${issues} 個）`;
}

/** status.nodes を並びのまま一覧の行にする。 */
function stRows(status) {
  const nodes = status && Array.isArray(status.nodes) ? status.nodes : [];
  return nodes.map((n) => ({
    id: n.id,
    kind: n.kind,
    depth: n.depth || 0,
    text: n.text || "",
    starred: !!n.starred,
    answerKind: n.answer_kind || null,
    onPath: n.on_path !== false,
    bottom: !!n.bottom_q,
    obj: n.objectivity || null,
    logic: n.logicality || null,
    counts: n.counts || null,
    pending: Number(n.pending) || 0,
    node: n,
  }));
}

/** Objectivity の印（`■■□ 2/3`）。マスは target 個まで、10 を超えれば 10 マスに縮める。 */
function stObjBar(o) {
  if (!o) return "";
  const target = Math.max(0, Number(o.target) || 0);
  const count = Math.max(0, Number(o.count) || 0);
  const cells = Math.min(target, 10);
  const filled = target > 10 ? Math.round((Math.min(count, target) * 10) / target) : Math.min(count, cells);
  return `${"■".repeat(filled)}${"□".repeat(cells - filled)} ${count}/${target}`;
}

/** Logicality の札（[表示, 色の名]）。 */
function stLogicLabel(l) {
  const v = l && l.value;
  return ST_LOGIC[v] || ST_LOGIC.unchecked;
}

/** Next Action の先頭 n 件。 */
function stNextLines(status, n = 3) {
  const list = status && Array.isArray(status.next) ? status.next : [];
  return list.slice(0, n).map((x) => ({
    label: x.purpose || "",
    why: x.why || "",
    node: x.node || null,
    block: x.block || "",
    card: x.card || null,
    kind: x.kind,
  }));
}

/**
 * Next Action の 1 行の文。`[<why>] <node> <block>「<node の文>」`。
 * node が status.nodes に無い（SEED など）ときは今までの label（purpose）を使う。
 * why が空なら `[...]` は付けない。
 */
function stNextText(x, status) {
  const o = x || {};
  const nodes = status && Array.isArray(status.nodes) ? status.nodes : [];
  const n = o.node ? nodes.find((r) => r.id === o.node) : null;
  const head = o.why ? `[${o.why}] ` : "";
  if (!n) return `${head}${o.label || ""}`;
  return `${head}${[o.node, o.block].filter((t) => t).join(" ")}「${n.text || ""}」`;
}

/** 板に出す操作の並び（出さないものを除く）。 */
function stOpsFor(node, children) {
  if (!node) return [];
  const kids = Array.isArray(children) ? children : [];
  return (ST_NODE_OPS[node.kind] || []).filter((op) => {
    if (op === "add_o") return !kids.some((c) => c.kind === "Q");
    if (op === "add_q") return !(node.kind === "Q" && kids.some((c) => c.kind === "O"));
    if (op === "add_e") return node.kind === "E" || node.kind === "S";
    return true;
  });
}

/** 並びの値（Set・配列・それ以外）を Set にする。 */
function stToSet(v, fallback) {
  if (v instanceof Set) return v;
  if (Array.isArray(v)) return new Set(v);
  return new Set(fallback || []);
}

/** 行の親の ID（無ければ null）。 */
function stRowParentId(r) {
  const p = r && r.node ? r.node.parent : null;
  return p == null || p === "" ? null : String(p);
}

/** 絞り込みの文字が ID か文に入っているか（大文字・小文字を区別しない）。 */
function stRowHasWord(r, word) {
  if (!word) return true;
  return String(r.id || "").toLowerCase().includes(word) || String(r.text || "").toLowerCase().includes(word);
}

/**
 * 一覧に出す行を純関数だけで決める。
 * 絞り込みが無いとき: 畳んだ行の子孫を除いた全部。
 * 絞り込みがあるとき（絞り込み中は畳みを無視する）: 合う行と、合う行の ancestor（ctx: true）。
 * 返すのは {row, hasKids, ctx} の並び（rows の順のまま）。
 */
function stVisibleRows(rows, opts) {
  const o = opts || {};
  const list = Array.isArray(rows) ? rows : [];
  const folded = stToSet(o.folded);
  const kinds = stToSet(o.kinds, ST_ALL_KINDS);
  const word = String(o.word || "").trim().toLowerCase();
  const filtering = word !== "" || !ST_ALL_KINDS.every((k) => kinds.has(k));

  const byId = new Map();
  for (const r of list) byId.set(r.id, r);
  const kids = new Map();
  for (const r of list) {
    const p = stRowParentId(r);
    if (p != null && byId.has(p)) {
      if (!kids.has(p)) kids.set(p, []);
      kids.get(p).push(r);
    }
  }
  const kidList = (id) => kids.get(id) || [];

  // ancestor をたどる（自分自身は含めない）
  const ancestorsOf = (r) => {
    const out = [];
    const seen = new Set([r.id]);
    let p = stRowParentId(r);
    while (p != null && byId.has(p) && !seen.has(p)) {
      seen.add(p);
      out.push(byId.get(p));
      p = stRowParentId(byId.get(p));
    }
    return out;
  };

  if (!filtering) {
    const out = [];
    for (const r of list) {
      if (ancestorsOf(r).some((a) => folded.has(a.id))) continue;
      out.push({ row: r, hasKids: kidList(r.id).length > 0, ctx: false });
    }
    return out;
  }

  // 絞り込み中は畳みを無視する。下から上へ「子孫に合う行があるか」を覚えて、上の行へ伝える
  const sub = new Map();
  const out = [];
  for (let i = list.length - 1; i >= 0; i--) {
    const r = list[i];
    const hit = kinds.has(r.kind) && stRowHasWord(r, word);
    const kidHit = kidList(r.id).some((k) => sub.get(k.id) === true);
    sub.set(r.id, hit || kidHit);
    if (hit) out.push({ row: r, hasKids: kidList(r.id).length > 0, ctx: false });
    else if (kidHit) out.push({ row: r, hasKids: kidList(r.id).length > 0, ctx: true });
  }
  out.reverse();
  return out;
}

/** run の移り先。Project の List 画面でその run を開く（session の画面ではない）。 */
function stRunListHash(project, runId) {
  return typeof listHash === "function"
    ? listHash(project, runId, null)
    : `#/run/${encodeURIComponent(project)}/${runId}`;
}

/** Action の履歴の頭の部分（`run 5 sci-read R2 O12`）。空の部分は詰めて飛ばす。 */
function stActionMeta(a) {
  const o = a || {};
  const parts = [];
  if (o.run_id != null && o.run_id !== "") parts.push(`run ${o.run_id}`);
  const block = String(o.block == null ? "" : o.block).trim();
  if (block) parts.push(block);
  const round = o.global_round != null && o.global_round !== "" ? `R${o.global_round}` : String(o.owner == null ? "" : o.owner).trim();
  if (round) parts.push(round);
  const node = String(o.node == null ? "" : o.node).trim();
  if (node) parts.push(node);
  return parts.join(" ");
}

/**
 * 境界線をドラッグしたときの幅を決める。
 * which は "list" か "board"。otherW はもう一方の幅、bodyW は sci-body の幅。
 * 図（bodyW − this − otherW − 12）が ST_GRAPH_MIN を割らないように抑える。
 */
function stClampSplit(which, px, bodyW, otherW) {
  const lim = ST_SPLIT_LIMITS[which] || ST_SPLIT_LIMITS.list;
  const other = Math.max(0, Number(otherW) || 0);
  const body = Math.max(0, Number(bodyW) || 0);
  let v = Math.max(lim[0], Math.min(lim[1], Math.round(Number(px) || 0)));
  const room = body - other - ST_SPLIT_BAR - ST_GRAPH_MIN;
  if (v > room) v = Math.max(lim[0], Math.min(v, room));
  return Math.round(v);
}

/** RFC3339 などの時刻を端末の地方時の `YYYY-MM-DD HH:MM` にする。読めなければそのまま。 */
function stFmtAt(at) {
  if (at == null || at === "") return at == null ? "" : String(at);
  const d = new Date(at);
  if (Number.isNaN(d.getTime())) return String(at);
  const p = (n) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())} ${p(d.getHours())}:${p(d.getMinutes())}`;
}

/**
 * History の 1 行の文（node_detail.history の 1 要素から）。
 *   init / Add * / 決定 … `<action> <date> <who>`
 *   Run               … `Run <run_id> <block> R<global_round>.<round> <date>`
 * who は by が user なら `human`、それ以外は `run <run_id> <block>`（block が空なら `run <run_id>`）。
 * R の部分は global_round が null なら付けない。round が null なら `R<global_round>` だけ。
 */
function stHistoryText(h) {
  const o = h || {};
  const date = stFmtAt(o.at);
  if (o.action === "Run") {
    const parts = ["Run"];
    if (o.run_id != null && o.run_id !== "") parts.push(String(o.run_id));
    if (o.block != null && String(o.block).trim() !== "") parts.push(String(o.block).trim());
    if (o.global_round != null && o.global_round !== "") {
      parts.push(o.round != null && o.round !== "" ? `R${o.global_round}.${o.round}` : `R${o.global_round}`);
    }
    if (date) parts.push(date);
    return parts.join(" ");
  }
  let who = "";
  if (o.by === "user") who = "human";
  else if (o.run_id != null && o.run_id !== "") {
    who = o.block != null && String(o.block).trim() !== ""
      ? `run ${o.run_id} ${String(o.block).trim()}`
      : `run ${o.run_id}`;
  }
  return [String(o.action || ""), date, who].filter((x) => x !== "").join(" ");
}

// ============================================================================
// 小さな DOM の部品
// ============================================================================

function stEl(tag, cls, text) {
  const e = document.createElement(tag);
  if (cls) e.className = cls;
  if (text != null) e.textContent = text;
  return e;
}

function stBtn(label, onClick, cls) {
  const b = stEl("button", `sai-btn small ${cls || ""}`.trim(), label);
  b.type = "button";
  if (typeof onClick === "function") b.addEventListener("click", onClick);
  else b.disabled = true;
  return b;
}

function stEmpty(opts) {
  if (typeof saiEmpty === "function") return saiEmpty(opts);
  const box = stEl("div", "sai-empty");
  if (opts && opts.title) box.appendChild(stEl("p", "sai-empty-title", opts.title));
  if (opts && opts.body) box.appendChild(stEl("p", "sai-empty-body", opts.body));
  return box;
}

function stToast(msg, kind) {
  if (typeof saiToast === "function") saiToast(msg, kind);
}

function stGo(hash) {
  if (hash) location.hash = hash;
}

/** sciedit.js / scigraph.js の関数を名前で呼ぶ。無ければ false。 */
function stCall(name, ...args) {
  const fn = typeof window !== "undefined" ? window[name] : undefined;
  if (typeof fn !== "function") {
    stToast(`${name} がまだありません`, "error");
    return false;
  }
  fn(...args);
  return true;
}

// ============================================================================
// 読み書き
// ============================================================================

/** この画面の API の住所。 */
function stApi(path) {
  return `${papersApiBase(selectedPaperProject())}/sci${path || ""}`;
}

function stStatus() {
  return stData;
}

/** status と、開いている Node を読み直して描き直す。 */
async function stReload() {
  if (typeof document === "undefined") return;
  if (!selectedPaperProject()) { stRenderNoProject(); return; }
  try {
    stData = await paperApi(stApi("/status"));
  } catch (e) {
    stData = null;
    stNode = null;
    stRenderError(e);
    return;
  }
  if (stData.initialized && !stTypesData) {
    try { stTypesData = await paperApi(stApi("/types")); } catch { stTypesData = null; }
  }
  await stLoadNode();
  stRender();
}

/** 開いている Node を読む。消えていれば選びを外す。 */
async function stLoadNode() {
  stNode = null;
  if (!stNodeId) return;
  try {
    stNode = await paperApi(stApi(`/nodes/${encodeURIComponent(stNodeId)}`));
  } catch {
    stNodeId = null;
    stSetHash();
  }
}

/** SEED か Node（SPROUT の S1 も含む）を選ぶ。 */
async function stSelect(id) {
  if (String(id || "") === "SEED") {
    stSeedSel = true;
    stNodeId = null;
    stNode = null;
  } else {
    stSeedSel = false;
    stNodeId = id || null;
    await stLoadNode();
  }
  stSetHash();
  stListOpen = false;
  stSyncListOpen();
  stRenderSidebar(document.getElementById("sci-list"));
  stRenderBoard();
  stMarkGraph();
}

function stSetHash() {
  const id = stNodeId || (stSeedSel ? "SEED" : null);
  const next = id ? `#/research/tree/${id}` : "#/research/tree";
  if (location.hash !== next) history.replaceState(null, "", next);
}

/** 画面を開く（app.js の route から呼ばれる）。 */
async function stOpen(params) {
  const id = (params && params.id) || null;
  stSeedSel = id === "SEED";
  stNodeId = id && id !== "SEED" ? id : null;
  stSprout = null; // 前の下書きは残さない
  stData = null;
  stNode = null;
  stListOpen = false;
  await ensurePaperProjects();
  fillSegment("sci-seg", "tree", RESEARCH_SEG);
  updateProjectChips();
  const body = document.getElementById("sci-body");
  if (body) body.classList.add("sai-split");
  stInitSplit();
  stInitFilter();
  stLoadFold();
  await stReload();
}

// --- 境界線のドラッグ（広い画面だけ。幅は localStorage の sai-sci-split に覚える） ---

/** 一覧と板の幅を #sci-body の CSS 変数に入れる。 */
function stApplySplit() {
  const body = document.getElementById("sci-body");
  if (!body) return;
  body.style.setProperty("--sci-list-w", `${stSplit.list}px`);
  body.style.setProperty("--sci-board-w", `${stSplit.board}px`);
}

/** 境界線 2 本にドラッグの仕掛けを 1 回だけ付け、覚えた幅を戻す。 */
function stInitSplit() {
  if (stSplitInit) return;
  stSplitInit = true;
  try {
    const v = JSON.parse(localStorage.getItem("sai-sci-split") || "null");
    if (v && Number(v.list) > 0) stSplit.list = Number(v.list);
    if (v && Number(v.board) > 0) stSplit.board = Number(v.board);
  } catch { /* 読めなければ既定の幅 */ }
  stApplySplit();
  const body = document.getElementById("sci-body");
  if (!body) return;
  for (const bar of body.querySelectorAll(".sci-split-handle")) {
    const which = bar.dataset.split === "board" ? "board" : "list";
    const other = which === "list" ? "board" : "list";
    bar.addEventListener("pointerdown", (e) => {
      e.preventDefault();
      try { bar.setPointerCapture(e.pointerId); } catch { /* 捕まえなくてもよい */ }
      bar.classList.add("dragging");
    });
    bar.addEventListener("pointermove", (e) => {
      if (!bar.classList.contains("dragging")) return;
      const r = body.getBoundingClientRect();
      const px = which === "list" ? e.clientX - r.left : r.right - e.clientX;
      // 板を出していないときは、板の幅を図の残りに数えない
      const otherW = other === "board" && !body.classList.contains("has-board") ? 0 : stSplit[other];
      stSplit[which] = stClampSplit(which, px, r.width, otherW);
      stApplySplit();
    });
    const end = (e) => {
      if (!bar.classList.contains("dragging")) return;
      bar.classList.remove("dragging");
      try { bar.releasePointerCapture(e.pointerId); } catch { /* 解放できなくてもよい */ }
      try { localStorage.setItem("sai-sci-split", JSON.stringify(stSplit)); } catch { /* 保存できなくてもよい */ }
    };
    bar.addEventListener("pointerup", end);
    bar.addEventListener("pointercancel", end);
  }
}

function stSyncListOpen() {
  const body = document.getElementById("sci-body");
  if (body) body.classList.toggle("sci-list-open", stListOpen);
}

// ============================================================================
// 上の帯（.sci-top）
// ============================================================================

function stRender() {
  stSyncListOpen();
  stRenderTop();
  stRenderSprout();
  stRenderSidebar(document.getElementById("sci-list"));
  stRenderBoard();
  stRenderGraph();
}

/** 「古い形の木」「Next Action」「History」「注意」の帯。 */
function stRenderTop() {
  const box = document.getElementById("sci-top");
  if (!box) return;
  box.innerHTML = "";
  const s = stData;
  if (!s) return;
  if (!s.initialized) {
    box.appendChild(stEmpty({
      icon: "🌱",
      title: "research-tree がまだありません",
      body: "RESEARCH_TYPE を選んで始めます。",
      action: { label: "始める", onClick: () => { void stStartTree(); } },
    }));
    return;
  }
  if (s.legacy) {
    box.appendChild(stEl("p", "sci-note sci-legacy", "古い形の tree です。新しい tree（Q・O・F・E）には読み替えていません。"));
  }
  box.appendChild(stNextBand(s));
  box.appendChild(stHistoryBand(s));
  box.appendChild(stChipBand(s));
}

/** 解く釦。kind が card なら Create CARD、user なら Q を選ぶ。それ以外は出さない。 */
function stActionBtn(x) {
  const card = (node, preset) => stBtn("Create CARD", () => { stCall("seOpenCard", node, preset); });
  if (x.kind === "card") return card(x.node, x.card);
  if (x.kind === "user") return stBtn("選ぶ", () => { void stSelect(x.node); });
  return null;
}

/** Next Action の先頭 3 件（.sci-next-row）。 */
function stNextRows(list, status) {
  const wrap = stEl("div", "sci-next");
  for (const x of list) {
    const row = stEl("div", "sci-line sci-next-row");
    row.appendChild(stEl("span", "sci-line-main", stNextText(x, status)));
    const b = stActionBtn(x);
    if (b) row.appendChild(b);
    wrap.appendChild(row);
  }
  return wrap;
}

function stNextBand(s) {
  const list = stNextLines(s, 3);
  const band = stEl("div", "sci-band sci-nextband");
  band.appendChild(stEl("span", "sci-band-title", "Next Action"));
  if (!list.length) band.appendChild(stEl("p", "sci-note", "今すぐやることはありません"));
  else band.appendChild(stNextRows(list, s));
  return band;
}

/** 全 Node の History（折りたたみ・古い順）。status.history を 1 行ずつ出す。 */
function stHistoryBand(s) {
  const det = document.createElement("details");
  det.className = "sci-band sci-historyband";
  det.open = stHistoryOpen;
  det.addEventListener("toggle", () => { stHistoryOpen = det.open; });
  const list = Array.isArray(s.history) ? s.history : [];
  det.appendChild(stEl("summary", "sci-band-title", `History（${list.length}）`));
  const box = stEl("div", "sci-history-list");
  if (!list.length) box.appendChild(stEl("p", "sci-note", "なし"));
  for (const h of list) box.appendChild(stHistoryRowEl(h, h && h.node ? `${h.node} ` : ""));
  det.appendChild(box);
  return det;
}

/** 注意の札と、狭い画面の一覧の出し入れ（.sci-chips）。 */
function stChipBand(s) {
  const row = stEl("div", "sci-chips");
  const w = s.warnings || [];
  if (w.length) {
    const b = stEl("button", "sci-chip warn", `⚠ 注意 ${w.length}`);
    b.type = "button";
    b.addEventListener("click", () => {
      const body = stEl("div", "sci-sheet-body");
      for (const t of w) body.appendChild(stEl("p", "sci-note", String(t)));
      if (typeof saiSheet === "function") saiSheet({ title: "注意", body });
    });
    row.appendChild(b);
  }
  const mk = stBtn(stSprout ? "Creating SPROUT" : "Create SPROUT", () => { stStartSprout(); });
  mk.disabled = !!stSprout;
  row.appendChild(mk);
  const t = stEl("button", `sci-list-toggle${stListOpen ? " active" : ""}`, "一覧");
  t.type = "button";
  t.setAttribute("aria-pressed", String(stListOpen));
  t.addEventListener("click", () => { stListOpen = !stListOpen; stSyncListOpen(); stRenderTop(); });
  row.appendChild(t);
  return row;
}

// ============================================================================
// SPROUT 作成パネル（#sci-sprout。History と図の間。作成中も図・一覧・板を操作できる）
// ============================================================================

/** パネル・一覧・板・図の印・上の帯の釦を、下書きの今に合わせる。 */
function stSproutRefresh() {
  stRenderTop();
  stRenderSprout();
  stRenderSidebar(document.getElementById("sci-list"));
  stRenderBoard();
  stMarkGraph();
}

/** PATH の O を足す／外す（O の板の釦）。21 個目は足さない。 */
function stSproutPick(id) {
  if (stSprout === null) return;
  const next = stSproutToggle(stSprout.paths, id);
  if (next.length === stSprout.paths.length && !stSprout.paths.includes(id)) {
    stToast(`PATH は ${ST_SPROUT_PATH_MAX} 個までです`, "error");
    return;
  }
  stSprout.paths = next;
  stSprout.error = "";
  stSproutRefresh();
}

/** 作成を始める。作成中なら題の欄へ戻るだけ。 */
function stStartSprout() {
  const focusTitle = () => {
    const box = document.getElementById("sci-sprout");
    if (!box) return;
    if (typeof box.scrollIntoView === "function") box.scrollIntoView({ block: "nearest" });
    const t = box.querySelector('[data-field="title"]');
    if (t) t.focus();
  };
  if (stSprout === null) {
    stSprout = { title: "", body: "", paths: [], error: "" };
    stRenderTop(); // 帯の釦を「作成中」にする
    stRenderSprout();
    stRenderBoard();
    stMarkGraph();
  }
  focusTitle();
}

async function stSproutCancel() {
  if (stSprout === null || stSproutBusy) return;
  const dirty = stSprout.title.trim() || stSprout.body.trim() || stSprout.paths.length;
  if (dirty && typeof saiConfirm === "function") {
    const ok = await saiConfirm({ title: "作成をやめますか", body: "入力した内容は消えます。", ok: "やめる", danger: true });
    if (!ok) return;
  }
  stSprout = null;
  stSproutRefresh();
}

/** 検査して送る。成功したらパネルを閉じて新しい S を選ぶ。失敗したら入力を残す。 */
async function stSproutSend() {
  if (stSprout === null || stSproutBusy) return;
  const built = typeof seBuildOp === "function"
    ? seBuildOp("add_sprout", { title: stSprout.title, body: stSprout.body, paths: stSprout.paths, nodes: stData ? stData.nodes : [] })
    : { ok: false, error: "SPROUT の検査がまだ使えません" };
  if (!built.ok) { stSprout.error = built.error; stRenderSprout(); return; }
  stSprout.error = "";
  stSproutBusy = true;
  stRenderSprout();
  let res = null;
  try {
    res = typeof seApply === "function" ? await seApply(built.op, null) : null;
  } finally {
    stSproutBusy = false;
  }
  if (!res) { stRenderSprout(); return; } // 失敗の理由は toast に出ている
  stSprout = null;
  stSproutRefresh();
  if (res.result && /^S[1-9][0-9]*$/.test(String(res.result))) await stSelect(String(res.result));
}

/** パネルを作り直す。題・本文は下書きから戻し、直前にフォーカスしていた欄へ戻す。 */
function stRenderSprout() {
  const box = document.getElementById("sci-sprout");
  if (!box) return;
  const active = document.activeElement;
  const field = active && box.contains(active) && active.dataset ? active.dataset.field : null;
  box.innerHTML = "";
  if (stSprout === null) { box.hidden = true; return; }
  box.hidden = false;
  const d = stSprout;
  const band = stEl("section", "sci-band sci-sprout-band");
  band.appendChild(stEl("span", "sci-band-title", "Create SPROUT"));
  band.appendChild(stEl("p", "sci-note", "DAG か一覧で O を選び、右の板の「Add PATH」を押します（PATH の終点になれるのは O だけです）"));

  const title = stEl("input", "sai-input");
  title.type = "text";
  title.maxLength = 120;
  title.placeholder = "題（必須）";
  title.setAttribute("aria-label", "題");
  title.dataset.field = "title";
  title.value = d.title;
  title.addEventListener("input", () => { d.title = title.value; });
  band.appendChild(title);

  const body = stEl("textarea", "sai-input");
  body.maxLength = 2000;
  body.placeholder = "本文（任意・実験の計画の要点）";
  body.setAttribute("aria-label", "本文");
  body.dataset.field = "body";
  body.value = d.body;
  body.addEventListener("input", () => { d.body = body.value; });
  band.appendChild(body);

  const list = stEl("div", "sci-sprout-paths");
  const rows = stSproutRows(d.paths, stData);
  if (!rows.length) list.appendChild(stEl("p", "sci-note", "O をまだ選んでいません"));
  for (const r of rows) {
    const line = stEl("div", "sci-sprout-path");
    const main = stEl("button", "sci-sprout-path-main", `${r.id} ${r.issue ? "Issue" : "Answer"} ${r.text}`.trim());
    main.type = "button";
    main.addEventListener("click", () => { void stSelect(r.id); });
    line.appendChild(main);
    const x = stEl("button", "sci-sprout-path-x", "✕");
    x.type = "button";
    x.setAttribute("aria-label", `Delete PATH: ${r.id}`);
    x.addEventListener("click", () => { d.paths = d.paths.filter((p) => p !== r.id); d.error = ""; stSproutRefresh(); });
    line.appendChild(x);
    list.appendChild(line);
  }
  band.appendChild(list);
  band.appendChild(stEl("p", "sci-note", stSproutSummary(d.paths, stData)));
  const err = stEl("p", "sci-sprout-error", d.error);
  err.setAttribute("role", "alert");
  band.appendChild(err);

  const actions = stEl("div", "sci-sprout-actions");
  const ok = stBtn("Create SPROUT", () => { void stSproutSend(); }, "primary");
  const no = stBtn("やめる", () => { void stSproutCancel(); }, "ghost");
  ok.disabled = no.disabled = stSproutBusy;
  actions.appendChild(ok);
  actions.appendChild(no);
  band.appendChild(actions);
  box.appendChild(band);

  const back = field ? box.querySelector(`[data-field="${field}"]`) : null;
  if (back) {
    back.focus();
    if (typeof back.setSelectionRange === "function") back.setSelectionRange(back.value.length, back.value.length);
  }
}

/** RESEARCH_TYPE を選んで木を始める。 */
async function stStartTree() {
  if (typeof saiSheet !== "function") return;
  const types = (stTypesData && Array.isArray(stTypesData.types) && stTypesData.types) || null;
  const list = types
    ? types.map((t) => [t.id, t.name || t.id, t.description || ""])
    : ST_TYPES;
  const body = stEl("div", "sci-types");
  for (const [id, name, explain] of list) {
    const b = stEl("button", "sci-type");
    b.type = "button";
    b.appendChild(stEl("span", "sci-type-name", name));
    b.appendChild(stEl("span", "sci-type-text", explain));
    b.addEventListener("click", () => { saiOverlayShut(); void stInitTree(id, name); });
    body.appendChild(b);
  }
  saiSheet({ title: "RESEARCH_TYPE を選ぶ", body });
}

async function stInitTree(typeId, typeName) {
  if (typeof saiConfirm === "function") {
    const ok = await saiConfirm({ title: "始めますか？", body: `${typeName} で research-tree を始めます。`, ok: "始める" });
    if (!ok) return;
  }
  try {
    await paperApi(stApi("/init"), { method: "POST", body: { type: typeId } });
  } catch (e) {
    stToast((e && e.message) || "始められませんでした", "error");
    return;
  }
  stNodeId = null;
  stSetHash();
  await stReload();
}

// ============================================================================
// 左の一覧（.sci-list の中）
// ============================================================================

/** 木の 1 行（.sci-row）。一覧と板の子で共通。 */
function stRowEl(r, opts) {
  const o = opts || {};
  const picked = stSprout && stSprout.paths.includes(r.id) ? " sci-row-picked" : "";
  const row = stEl("div", `sci-row sci-row-${r.kind}${r.onPath ? "" : " sci-dim"}${r.id === stNodeId ? " selected" : ""}${picked}${o.ctx ? " sci-row-ctx" : ""}`);
  row.dataset.id = r.id;
  row.style.paddingLeft = `${8 + (o.flat ? 0 : r.depth) * 14}px`;
  // 畳み・開くの釦（一覧だけ。板の子には付けない）
  if (o.twisty) {
    if (o.twisty.hasKids) {
      const t = stEl("button", "sci-twisty", o.twisty.folded ? "▸" : "▾");
      t.type = "button";
      t.setAttribute("aria-expanded", String(!o.twisty.folded));
      t.setAttribute("aria-label", `${r.id} を${o.twisty.folded ? "開く" : "畳む"}`);
      t.addEventListener("click", () => { stFoldToggle(r.id); });
      row.appendChild(t);
    } else {
      row.appendChild(stEl("span", "sci-twisty-space"));
    }
  }
  const main = stEl("button", "sci-row-main");
  main.type = "button";
  main.addEventListener("click", async () => {
    await stSelect(r.id);
    // 一覧から選んだときだけ、図の真ん中にその Node を出す（図や板からなら寄せない）
    if (o.center && typeof sgCenterOn === "function") sgCenterOn(r.id);
  });
  const head = stEl("span", "sci-row-head");
  head.appendChild(stEl("span", "sci-id", r.id));
  let title = r.text;
  if (r.kind === "F") title = title.slice(0, 60);
  head.appendChild(stEl("span", "sci-row-title", title));
  main.appendChild(head);
  const sub = stEl("span", "sci-row-sub");
  if (r.kind === "E") {
    if (r.node.mlflow) sub.appendChild(stEl("span", "sci-id", String(r.node.mlflow)));
  }
  main.appendChild(sub);
  row.appendChild(main);
  if (r.kind === "O") {
    const star = stEl("button", `sci-star${r.starred ? " on" : ""}`, r.starred ? "★" : "☆");
    star.type = "button";
    star.setAttribute("aria-label", r.starred ? "★ を外す" : "★ を付ける");
    star.addEventListener("click", () => { stCall("seQuick", r.starred ? "unstar" : "star", { o: r.id }); });
    row.appendChild(star);
    if (r.answerKind) {
      const k = stEl("button", `sci-kind sci-kind-${r.answerKind}`, ST_KIND_LABEL[r.answerKind] || r.answerKind);
      k.type = "button";
      k.addEventListener("click", () => { void stToggleKind(r); });
      row.appendChild(k);
    }
  }
  if (r.pending > 0) {
    const mark = stEl("span", "sci-row-pending", `P${r.pending}`);
    mark.title = `PENDING の PROPOSAL が ${r.pending} 件`;
    row.appendChild(mark);
  }
  return row;
}

/** Answer と Issue を入れ替える（確認は seQuick が出す）。 */
function stToggleKind(r) {
  const next = r.answerKind === "answer" ? "issue" : "answer";
  stCall("seQuick", "set_kind", { o: r.id, kind: next });
}

// --- 左の一覧の絞り込み（一番上・作り直さない） ---

/** 一番上の絞り込みを 1 回だけ作る（入力中に一覧を描き直しても入力欄は残る）。 */
function stInitFilter() {
  if (stFilterInit) return;
  stFilterInit = true;
  const box = document.getElementById("sci-filter");
  if (!box) return;
  box.innerHTML = "";
  const input = stEl("input", "sci-filter-input");
  input.type = "search";
  input.placeholder = "ID・文で絞り込み";
  input.setAttribute("aria-label", "ID・文で絞り込む");
  input.addEventListener("input", () => {
    stFilter.word = input.value || "";
    stRenderSidebar(document.getElementById("sci-list"));
  });
  box.appendChild(input);
  const row = stEl("div", "sci-filter-kinds");
  for (const k of ST_ALL_KINDS) {
    const b = stEl("button", `sci-filter-kind sci-filter-kind-${k}`, k);
    b.type = "button";
    b.setAttribute("aria-pressed", String(stFilter.kinds.has(k)));
    b.addEventListener("click", () => {
      if (stFilter.kinds.has(k)) stFilter.kinds.delete(k);
      else stFilter.kinds.add(k);
      b.setAttribute("aria-pressed", String(stFilter.kinds.has(k)));
      stRenderSidebar(document.getElementById("sci-list"));
    });
    row.appendChild(b);
  }
  box.appendChild(row);
}

// --- 左の一覧の畳み（図の畳みとは連動させない） ---

/** 畳んだ ID を覚える場所（プロジェクトごとに別）。 */
function stFoldKey() {
  return `sai-sci-fold:${selectedPaperProject() || ""}`;
}

/** 畳んだ ID を覚える（読めなくても書けなくても画面は動く）。 */
function stSaveFold() {
  try { localStorage.setItem(stFoldKey(), JSON.stringify([...stFolded])); } catch { /* 保存できなくてもよい */ }
}

/** 覚えた畳みを読む。読み込みは stOpen。 */
function stLoadFold() {
  stFolded = new Set();
  try {
    const raw = localStorage.getItem(stFoldKey());
    if (!raw) return;
    const v = JSON.parse(raw);
    if (Array.isArray(v)) for (const id of v) stFolded.add(String(id));
  } catch { /* 読めなければ畳まない */ }
}

/** 今の木に無い ID を覚えておく（消えた Node の畳みを捨てる）。 */
function stPruneFolded(rows) {
  const ids = new Set(rows.map((r) => r.id));
  let changed = false;
  for (const id of [...stFolded]) {
    if (!ids.has(id)) { stFolded.delete(id); changed = true; }
  }
  if (changed) stSaveFold();
}

/** 畳み・開くを反転して、一覧だけ描き直す（図と板は変えない）。 */
function stFoldToggle(id) {
  if (stFolded.has(id)) stFolded.delete(id);
  else stFolded.add(id);
  stSaveFold();
  stRenderSidebar(document.getElementById("sci-list"));
}

function stRenderSidebar(box) {
  if (!box) return;
  stInitFilter();
  const filterBox = document.getElementById("sci-filter");
  const ready = !!(stData && stData.initialized);
  if (filterBox) filterBox.hidden = !ready;
  box.innerHTML = "";
  if (!ready) return;
  const rows = stRows(stData);
  if (!rows.length) {
    box.appendChild(stEmpty({ title: "Node がありません", body: "" }));
    return;
  }
  stPruneFolded(rows);
  const list = stVisibleRows(rows, { folded: stFolded, word: stFilter.word, kinds: stFilter.kinds });
  if (!list.length) {
    box.appendChild(stEl("p", "sci-note", "合う行がありません"));
    return;
  }
  for (const x of list) {
    box.appendChild(stRowEl(x.row, {
      twisty: { hasKids: x.hasKids, folded: stFolded.has(x.row.id) },
      center: true,   // 一覧から押したら図の真ん中に出す
      ctx: x.ctx,     // 絞り込みの ancestor は薄くする
    }));
  }
}

// ============================================================================
// 真ん中の図
// ============================================================================

function stRenderGraph() {
  const box = document.getElementById("sci-graph");
  if (!box) return;
  box.innerHTML = "";
  if (!stData || !stData.initialized) return;
  if (typeof sgRenderGraph !== "function") {
    box.appendChild(stEl("p", "sci-note", "research-tree の図はまだ使えません"));
    return;
  }
  sgRenderGraph(box, { status: stData, key: selectedPaperProject() }, (sel) => {
    if (sel && (sel.kind === "node" || sel.kind === "seed")) void stSelect(sel.id);
  });
  stMarkGraph();
}

function stMarkGraph() {
  if (typeof sgMarkPicked === "function") sgMarkPicked(stSprout ? stSprout.paths : []);
  if (typeof sgMarkSelected === "function") sgMarkSelected(stNodeId || (stSeedSel ? "SEED" : null));
}

// ============================================================================
// 右の板（.sci-board / saiSheet）
// ============================================================================

function stBoardKey() {
  return stNodeId || (stSeedSel ? "SEED" : null);
}

/** 板の中身を作る（何も選んでいなければ null）。 */
function stSelBoard() {
  if (stNodeId && stNode) return { title: `${stNode.node.id}`, el: stBuildBoard(stNode) };
  if (stSeedSel) return { title: "SEED", el: stBuildSeedPanel() };
  return null;
}

function stRenderBoard() {
  const box = document.getElementById("sci-board");
  const body = document.getElementById("sci-body");
  if (!stData || !stData.initialized) {
    stCloseSheet();
    if (box) { box.innerHTML = ""; box.hidden = true; }
    return;
  }
  const board = stSelBoard();
  const key = stBoardKey();
  // 何も選んでいなければ板は出さない（図がその分広がる）
  if (!board) {
    stCloseSheet();
    if (box) { box.innerHTML = ""; box.hidden = true; }
    if (body) body.classList.remove("has-board");
    return;
  }
  if (box && saiIsWide()) {
    stCloseSheet();
    box.innerHTML = "";
    box.hidden = false;
    if (body) body.classList.add("has-board");
    const head = stEl("div", "sci-line sci-board-head");
    head.appendChild(stEl("h2", "sci-board-title", board.title));
    const x = stEl("button", "sci-board-close", "✕");
    x.type = "button";
    x.setAttribute("aria-label", "閉じる");
    x.addEventListener("click", () => { void stClearSelection(); });
    head.appendChild(x);
    box.appendChild(head);
    box.appendChild(board.el);
    return;
  }
  if (box) { box.innerHTML = ""; box.hidden = true; }
  if (body) body.classList.remove("has-board");
  if (typeof saiSheet !== "function") return;
  if (stSheet && stSheetFor === key && stSheetBody) {
    stSheetBody.innerHTML = "";
    stSheetBody.appendChild(board.el);
    return;
  }
  stCloseSheet();
  const sheetBody = stEl("div", "sci-sheet-body");
  sheetBody.appendChild(board.el);
  const sheet = saiSheet({
    title: board.title,
    body: sheetBody,
    onClose: () => {
      if (stSheet !== sheet) return;
      stSheet = null; stSheetFor = null; stSheetBody = null;
      void stClearSelection();
    },
  });
  stSheet = sheet;
  stSheetFor = key;
  stSheetBody = sheetBody;
}

async function stClearSelection() {
  stNodeId = null; stSeedSel = false; stNode = null;
  stSetHash();
  stRenderSidebar(document.getElementById("sci-list"));
  stRenderBoard();
  stMarkGraph();
}

function stCloseSheet() {
  const sheet = stSheet;
  stSheet = null; stSheetFor = null; stSheetBody = null;
  if (sheet && typeof sheet.close === "function") sheet.close();
}

function stSec(title) {
  const sec = stEl("section", "sci-sec");
  if (title) sec.appendChild(stEl("h3", "sci-sec-title", title));
  return sec;
}

function stMd(text) {
  const d = stEl("div", "sci-md");
  if (typeof renderMarkdown === "function") d.innerHTML = renderMarkdown(text || "");
  else d.textContent = text || "";
  return d;
}

/**
 * SEED を選んだときの板。RESEARCH_TYPE と mind.md の節、top_Q・SPROUT・CARD の釦を出す。
 * SEED は DB の行を持たないので、node SEED ではなく status.seed から作る。
 */
function stBuildSeedPanel() {
  const box = stEl("div", "sci-card");
  const seed = (stData && stData.seed) || {};
  const sec = stSec("SEED");
  const typeName = seed.type && seed.type.name ? String(seed.type.name) : "";
  const typeLine = stEl("p", "sci-seed-type");
  typeLine.appendChild(stEl("strong", "", "RESEARCH_TYPE: "));
  typeLine.appendChild(document.createTextNode(typeName || "未設定"));
  sec.appendChild(typeLine);
  const mind = Array.isArray(seed.mind) ? seed.mind : [];
  if (!mind.length) {
    sec.appendChild(stEl("p", "sci-note", "mind.md に書かれた節がありません（.sai/paper/mind.md を利用者が書きます）"));
  } else {
    for (const m of mind) {
      if (m && m.heading) sec.appendChild(stEl("h4", "sci-mind-head", String(m.heading)));
      sec.appendChild(stMd((m && m.body) || ""));
    }
  }
  box.appendChild(sec);
  const ops = stSec("OP");
  const row = stEl("div", "sci-ops");
  row.appendChild(stBtn("top_Q を足す", () => { stCall("seOpenAction", "add_q", "SEED"); }));
  row.appendChild(stBtn("Create SPROUT", () => { stStartSprout(); }));
  row.appendChild(stBtn("Create CARD", () => { stCall("seOpenCard", "SEED"); }));
  ops.appendChild(row);
  box.appendChild(ops);
  return box;
}

/** S（SPROUT）の PATH の節。paths の O ごとに top_Q から O までの道を 1 行で出す。 */
function stBoardPaths(node) {
  const sec = stSec("PATH");
  const byId = new Map();
  for (const n of (stData && stData.nodes) || []) {
    if (n && n.id != null) byId.set(String(n.id), n);
  }
  const paths = Array.isArray(node.paths) ? node.paths : [];
  if (!paths.length) { sec.appendChild(stEl("p", "sci-note", "なし")); return sec; }
  for (const pid of paths) {
    const end = byId.get(String(pid));
    if (!end) continue;
    const chain = [];
    let cur = end;
    const seen = new Set();
    while (cur && !seen.has(String(cur.id))) {
      seen.add(String(cur.id));
      chain.push(cur);
      cur = cur.parent == null ? null : byId.get(String(cur.parent));
    }
    chain.reverse();
    const parts = chain.map((n) => (/issue/i.test((n.kind === "O" && n.answer_kind) || "") ? `${n.id} Issue` : `${n.id}`));
    const line = stEl("p", "sci-path-line", parts.join(" › "));
    if (end.text) line.appendChild(document.createTextNode(` ${end.text}`));
    sec.appendChild(line);
  }
  return sec;
}

/** S（SPROUT）の BRANCH の節。子の E を行で出す。 */
function stBoardBranch(children) {
  const sec = stSec("BRANCH");
  const rows = stRows({ nodes: children });
  if (!rows.length) sec.appendChild(stEl("p", "sci-note", "なし"));
  for (const r of rows) sec.appendChild(stRowEl(r, { flat: true }));
  return sec;
}

/** Node の板。Q・O・E は「見出し（Edit/Delete を含む）→ Parent → OP → [Q だけ: Objectivity・Logicality・ガイド] → Child → Next Action → History → PENDING」。S（SPROUT）は Parent の代わりに PATH、Child の代わりに BRANCH。F は「見出し（Edit/Delete を含む）→ Parent → OP → 出典 → History」だけ。 */
function stBuildBoard(d) {
  const node = d.node || {};
  const box = stEl("div", "sci-card");
  box.appendChild(stBoardHead(d));
  if (node.kind !== "S") box.appendChild(stBoardPath(d));
  box.appendChild(stBoardOps(d));
  if (node.kind === "F") {
    box.appendChild(stBoardSources(d));
    box.appendChild(stBoardHistory(d));
    return box;
  }
  if (node.kind === "Q") {
    box.appendChild(stBoardObjectivity(node));
    box.appendChild(stBoardLogic(node));
    box.appendChild(stBoardGuide(d.guide));
  }
  if (node.kind === "S") {
    box.appendChild(stBoardPaths(node));
    box.appendChild(stBoardBranch(d.children));
  } else {
    box.appendChild(stBoardChildren(d.children));
  }
  const next = stSec("Next Action");
  const nl = stNextLines({ next: d.next }, 3);
  if (nl.length) next.appendChild(stNextRows(nl, stData));
  else next.appendChild(stEl("p", "sci-note", "なし"));
  box.appendChild(next);
  box.appendChild(stBoardHistory(d));
  const pend = stBoardPending(d.pending);
  if (pend) box.appendChild(pend);
  return box;
}

/** 操作の表示名。seOpLabel が読めなければ op をそのまま。 */
function stOpLabel(op) {
  const label = (typeof seOpLabel === "function") ? seOpLabel(op) : null;
  if (label) return label;
  const fall = { add_q: "Add Q", add_o: "Add O", add_f: "Add F", add_e: "Add E", copy: "Copy O", edit: "Edit", remove: "Delete" };
  return fall[op] || op;
}

/** 見出し（ID・種類・文・印）と本文。O の頭は `[☆|★] O12 [Answer] / [Issue]`。文と本文の後に Edit / Delete。 */
function stBoardHead(d) {
  const node = (d && d.node) || {};
  const sec = stEl("section", "sci-sec sci-node-head");
  const line = stEl("div", "sci-line");
  if (node.kind === "O") {
    const star = stEl("button", `sci-head-star${node.starred ? " on" : ""}`, node.starred ? "★" : "☆");
    star.type = "button";
    star.setAttribute("aria-pressed", String(!!node.starred));
    star.setAttribute("aria-label", node.starred ? "★ を外す" : "★ を付ける");
    star.addEventListener("click", () => { stCall("seQuick", node.starred ? "unstar" : "star", { o: node.id }); });
    line.appendChild(star);
    line.appendChild(stEl("span", "sci-id", `${node.id}`));
    const ans = stEl("button", `sci-head-kind${node.answer_kind === "answer" ? " on" : ""}`, "Answer");
    ans.type = "button";
    if (node.answer_kind === "answer") ans.setAttribute("aria-pressed", "true");
    else ans.addEventListener("click", () => { stCall("seQuick", "set_kind", { o: node.id, kind: "answer" }); });
    line.appendChild(ans);
    line.appendChild(document.createTextNode("/"));
    const iss = stEl("button", `sci-head-kind${node.answer_kind === "issue" ? " on" : ""}`, "Issue");
    iss.type = "button";
    if (node.answer_kind === "issue") iss.setAttribute("aria-pressed", "true");
    else iss.addEventListener("click", () => { stCall("seQuick", "set_kind", { o: node.id, kind: "issue" }); });
    line.appendChild(iss);
  } else {
    let headText = `${node.id} ${node.kind}`;
    if (node.kind === "Q") {
      if (node.objectivity) headText += ` Objectivity ${stObjBar(node.objectivity)}`;
      const [logicLabel] = stLogicLabel(node.logicality);
      headText += ` Logicality ${logicLabel}`;
    }
    line.appendChild(stEl("span", "sci-id", headText));
  }
  sec.appendChild(line);
  sec.appendChild(stEl("p", "sci-node-title", node.text || ""));
  if (node.body) sec.appendChild(stMd(node.body));
  // S（SPROUT）は親が無く、OP の節に Edit / Delete を出すのでここでは出さない
  if (node.kind !== "S") {
    const editRow = stEl("div", "sci-node-edit");
    editRow.appendChild(stBtn(stOpLabel("edit"), () => { stCall("seOpenAction", "edit", node.id, d); }));
    if (node.parent) editRow.appendChild(stBtn(stOpLabel("remove"), () => { stCall("seOpenAction", "remove", node.id, d); }));
    sec.appendChild(editRow);
  }
  return sec;
}

/** Parent（Child と同じ作り）。d.path から最後の 1 つ（自分）を除いた ID を、上から順に行にする。 */
function stBoardPath(d) {
  const sec = stSec("Parent");
  const path = (d && d.path) || [];
  const ids = path.map((p) => (p && typeof p === "object" ? p.id : p)).filter(Boolean);
  // path の最後が自分なら除く
  const nodeId = d && d.node ? d.node.id : null;
  const parents = ids.filter((id, i) => !(i === ids.length - 1 && String(id) === String(nodeId)));
  if (!parents.length) { sec.appendChild(stEl("p", "sci-note", "なし")); return sec; }
  const byId = new Map();
  for (const n of (stData && stData.nodes) || []) {
    if (n && n.id != null) byId.set(String(n.id), n);
  }
  const pathById = new Map();
  for (const p of path) {
    if (p && typeof p === "object" && p.id != null) pathById.set(String(p.id), p);
  }
  const nodes = [];
  for (const id of parents) {
    const n = byId.get(String(id));
    if (n) { nodes.push(n); continue; }
    const p = pathById.get(String(id));
    if (p) nodes.push({ id: p.id, kind: p.kind || "Q", depth: 0, text: p.text || "" });
  }
  const rows = stRows({ nodes });
  if (!rows.length) { sec.appendChild(stEl("p", "sci-note", "なし")); return sec; }
  for (const r of rows) sec.appendChild(stRowEl(r, { flat: true }));
  return sec;
}

/** OP。stOpsFor の op に表示名を付けた釦と、最後に Card。 */
function stBoardOps(d) {
  const node = d.node;
  const sec = stSec("OP");
  const ops = stEl("div", "sci-ops");
  if (stSprout !== null && node.kind === "O") {
    const on = stSprout.paths.includes(node.id);
    ops.appendChild(stBtn(on ? "Delete PATH" : "Add PATH", () => { stSproutPick(node.id); }, on ? "" : "primary"));
  }
  for (const op of stOpsFor(node, d.children)) {
    ops.appendChild(stBtn(stOpLabel(op), () => { stCall("seOpenAction", op, node.id, d); }));
  }
  ops.appendChild(stBtn("Card", () => { stCall("seOpenCard", node.id, null); }));
  sec.appendChild(ops);
  return sec;
}

/** Objectivity と目安の入力欄（.sci-target）。 */
function stBoardObjectivity(node) {
  const sec = stSec("Objectivity");
  const o = node.objectivity;
  sec.appendChild(stEl("span", `sci-obj${o && o.met ? " met" : ""}`, stObjBar(o) || "—"));
  const line = stEl("label", "sci-line sci-target", "目安");
  const input = stEl("input", "sci-target-input");
  input.type = "number";
  input.min = "1";
  input.inputMode = "numeric";
  input.value = o ? String(o.target) : "";
  input.placeholder = "ガイドの値";
  input.addEventListener("change", () => {
    const v = input.value.trim();
    const n = v === "" ? null : Number(v);
    if (n !== null && !(Number.isInteger(n) && n >= 1)) { stToast("目安は 1 以上の整数にしてください", "error"); return; }
    stCall("seQuick", "set_target", { q: node.id, target: n });
  });
  line.appendChild(input);
  sec.appendChild(line);
  return sec;
}

/** Logicality の値・確かめた日・note・findings（.sci-logic-sec）。 */
function stBoardLogic(node) {
  const sec = stSec("Logicality");
  sec.classList.add("sci-logic-sec");
  const l = node.logicality;
  const [label, tone] = stLogicLabel(l);
  const line = stEl("div", "sci-line");
  line.appendChild(stEl("span", `sci-logic sci-logic-${tone}`, label));
  if (l && l.checked_at) line.appendChild(stEl("span", "sci-id", String(l.checked_at)));
  sec.appendChild(line);
  if (l && l.note) sec.appendChild(stEl("p", "sci-line-sub", l.note));
  const row = stEl("div", "sci-chips");
  for (const id of (l && l.findings) || []) {
    const b = stEl("button", "sci-chip", String(id));
    b.type = "button";
    b.addEventListener("click", () => { void stSelect(id); });
    row.appendChild(b);
  }
  sec.appendChild(row);
  return sec;
}

/** ガイド（question・Child Q・Extract・Pitfall）。 */
function stBoardGuide(g) {
  const sec = stSec("GUIDE");
  if (!g) { sec.appendChild(stEl("p", "sci-note", "なし")); return sec; }
  const items = [["question", g.question], ["Child Q", g.child_q], ["Extract", g.extract], ["Pitfall", g.pitfall]];
  for (const [k, v] of items) {
    if (!v) continue;
    const p = stEl("p", "sci-guide");
    p.appendChild(stEl("strong", "", `${k}: `));
    p.appendChild(document.createTextNode(Array.isArray(v) ? v.join(" / ") : String(v)));
    sec.appendChild(p);
  }
  return sec;
}

/** Child。 */
function stBoardChildren(children) {
  const sec = stSec("Child");
  const rows = stRows({ nodes: children });
  if (!rows.length) sec.appendChild(stEl("p", "sci-note", "なし"));
  for (const r of rows) sec.appendChild(stRowEl(r, { flat: true }));
  return sec;
}

/** History。d.history の順のまま 1 行ずつ出す（並べ替えない）。run_id がある行は押すとその run へ。 */
function stBoardHistory(d) {
  const sec = stSec("History");
  const list = d && Array.isArray(d.history) ? d.history : [];
  if (!list.length) { sec.appendChild(stEl("p", "sci-note", "なし")); return sec; }
  for (const h of list) sec.appendChild(stHistoryRowEl(h, ""));
  return sec;
}

/** History の 1 行。run_id がある行は押すとその run へ。prefix は文の先頭に付ける（Node の ID など）。 */
function stHistoryRowEl(h, prefix) {
  const text = `${prefix || ""}${stHistoryText(h)}`;
  if (h && h.run_id != null && h.run_id !== "") {
    const project = typeof selectedPaperProject === "function" ? selectedPaperProject() : null;
    const b = stEl("button", "sci-history-row", text);
    b.type = "button";
    b.addEventListener("click", () => stGo(stRunListHash(project, h.run_id)));
    return b;
  }
  return stEl("div", "sci-history-row", text);
}

/** 出典の cite（`#57.3.2` など）の先頭の数を Library の論文の番号として読む（読めなければ null）。 */
function stCitePaperId(cite) {
  const m = /^#?([1-9][0-9]{0,8})(?:\.|$)/.exec(String(cite == null ? "" : cite).trim());
  return m ? Number(m[1]) : null;
}

/** cite の札。論文の番号が分かれば、Library の論文詳細へ飛ぶ釦にする。楕円は使わない。 */
function stCiteChip(cite, paper) {
  const id = paper != null ? paper : stCitePaperId(cite);
  if (id == null || typeof libraryPaperHash !== "function") return stEl("span", "sci-cite", String(cite || ""));
  const b = stEl("button", "sci-link-title sci-cite", String(cite || `論文 ${id}`));
  b.type = "button";
  b.addEventListener("click", () => stGo(libraryPaperHash(id)));
  return b;
}

/** 出典（.sci-source）。F の板に出す。 */
function stBoardSources(d) {
  const sec = stSec("出典");
  const list = d && Array.isArray(d.sources) ? d.sources.filter(Boolean) : [];
  if (!list.length) {
    // 出典の項目が無いときは cites を並べる（どちらも無ければ「なし」）
    const cites = d && d.node ? d.node.cites : null;
    const arr = Array.isArray(cites) ? cites : (cites == null || cites === "" ? [] : [cites]);
    if (!arr.length) { sec.appendChild(stEl("p", "sci-note", "なし")); return sec; }
    const row = stEl("div", "sci-cites");
    for (const c of arr) row.appendChild(stCiteChip(c, null));
    sec.appendChild(row);
    return sec;
  }
  for (const s of list) {
    const item = stEl("div", "sci-source");
    const head = stEl("div", "sci-line");
    head.appendChild(stCiteChip(s.cite, s.paper));
    const title = String(s.title || "") || (s.paper != null ? `論文 ${s.paper}` : "");
    if (title) {
      if (s.paper != null && typeof libraryPaperHash === "function") {
        const b = stEl("button", "sci-link-title", title);
        b.type = "button";
        b.addEventListener("click", () => stGo(libraryPaperHash(s.paper)));
        head.appendChild(b);
      } else {
        head.appendChild(stEl("span", "sci-line-main", title));
      }
    }
    item.appendChild(head);
    if (s.section) item.appendChild(stEl("p", "sci-line-sub", `§ ${s.section}`));
    if (s.passage) item.appendChild(stEl("p", "sci-source-passage", String(s.passage)));
    sec.appendChild(item);
  }
  return sec;
}

/** PENDING（TRIAGE を待っている PROPOSAL）。0 件なら節を出さない。 */
function stBoardPending(pending) {
  const list = Array.isArray(pending) ? pending : [];
  if (!list.length) return null;
  const sec = stSec(`PENDING ${list.length}`);
  for (const p of list) {
    sec.appendChild(typeof seProposalRow === "function" ? seProposalRow(p) : stEl("p", "sci-line-sub", p.title || p.id || ""));
  }
  return sec;
}

// ============================================================================
// 失敗・プロジェクト無しの表示
// ============================================================================

function stClearAll() {
  stCloseSheet();
  stSprout = null;
  for (const id of ["sci-graph", "sci-top", "sci-sprout", "sci-list"]) {
    const e = document.getElementById(id);
    if (e) { e.innerHTML = ""; if (id === "sci-sprout") e.hidden = true; }
  }
  const filter = document.getElementById("sci-filter");
  if (filter) { filter.innerHTML = ""; filter.hidden = true; }
  const b = document.getElementById("sci-board");
  if (b) { b.innerHTML = ""; b.hidden = true; }
  return document.getElementById("sci-list");
}

function stRenderError(e) {
  const list = stClearAll();
  if (list) list.appendChild(stEmpty({ icon: "⚠️", title: "research-tree を読めませんでした", body: (e && e.message) || "読み直してください" }));
}

function stRenderNoProject() {
  const list = stClearAll();
  const msg = typeof NO_PROJECT_MSG === "string" ? NO_PROJECT_MSG : "論文の置き場があるプロジェクトがありません";
  if (list) list.appendChild(stEmpty({ icon: "📁", title: "論文の置き場がありません", body: msg }));
}

// ============================================================================
// 画面の登録（window.SaiRoutes）
// ============================================================================

if (typeof window !== "undefined" && typeof document !== "undefined") {
  (window.SaiRoutes = window.SaiRoutes || []).push({
    match(hash) { return stParseHash(hash); },
    view: "view-sci-tree",
    tab: "research",
    hideTabbar: false,
    interval: 0, // 定期更新はしない（↻ で読み直す）
    async open(params) { await stOpen(params); },
    async poll() {},
  });

  // 古い住所（#/papers/tree・#/papers/triage・#/papers/read/…・#/mlflow…・
  // #/papers/canvas・#/canvas/…）は新しい住所へ送る（U14）。
  window.SaiRoutes.push({
    match(hash) { return stLegacyHash(hash) ? {} : null; },
    view: "view-sci-tree",
    tab: "research",
    hideTabbar: false,
    interval: 0,
    async open() {
      const next = stLegacyHash(location.hash);
      if (next) location.replace(next);
    },
    async poll() {},
  });

  const stReloadBtn = document.getElementById("sci-reload");
  if (stReloadBtn) stReloadBtn.addEventListener("click", () => { void stReload(); });
}

// --- node からの読み込み（tests/test_scitree.js） ---
if (typeof module !== "undefined" && module.exports) {
  module.exports = {
    stParseHash, stLegacyHash, stRows, stObjBar, stLogicLabel, stNextLines, stNextText,
    stOpsFor, stSproutToggle, stSproutRows, stSproutSummary,
    stVisibleRows, stActionMeta, stClampSplit, stFmtAt, stHistoryText,
    ST_TYPES, ST_LOGIC, ST_KIND_LABEL, ST_NODE_OPS, ST_ALL_KINDS, ST_SPLIT_LIMITS, ST_SPROUT_PATH_MAX,
  };
}
