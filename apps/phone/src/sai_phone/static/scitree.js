"use strict";
// 研究方針の画面（Q・O・F・E の木）。住所は #/papers/tree と #/papers/tree/<ID>。
//
// 上の帯に SEED・進み具合・Next Action（先頭 3 件）・DEBT・注意、左に木の一覧、真ん中に
// 方針の図（scigraph.js）、右に選んだ Node の板（何も選ばなければ決定の記録）を出す。
// 狭い画面（幅 390px）の板は下から出る（saiSheet）。
//
// 読むのは sai-paper の `phone sci status` / `types` / `nodes/<ID>` だけ（API は U17）。
// 操作は sciedit.js（seOpenAction・seOpenCard・seQuick・seOpLabel・seProposalRow）、図は
// scigraph.js（sgRenderGraph・sgMarkSelected）の関数を typeof で確かめてから呼ぶ。
// DOM を使わない純関数は末尾で node に渡す（tests/test_scitree.js）。
// 画面の名詞（Q・O・F・E・Answer・Issue・SEED・Objectivity・Logicality・DEBT など）は英語のまま出す。

// --- 定数 ---

// 研究の型（/sci/types が返せば、その説明で置き換える）
const ST_TYPES = [
  ["problem", "PROBLEM_DRIVEN", "最初に課題（困っていること）を決めて、何をなぜやるのかから始める研究です。"],
  ["phenomenon", "PHENOMENON_DRIVEN", "先に現象を捉え、「なぜ起きるのか」を問う研究です。"],
  ["method", "METHOD_DRIVEN", "先に手法を決め、その手法で何が分かるようになるかを考える研究です。"],
  ["data", "DATA_DRIVEN", "手にあるデータから問いを立てる研究です。"],
  ["improve", "IMPROVE_DRIVEN", "いまのものの弱いところから始めた研究です。"],
];
const ST_LOGIC = { ok: ["OK", "ok"], contradiction: ["CONTRADICTION", "warn"], unchecked: ["UNCHECKED", "dim"] };
const ST_KIND_LABEL = { answer: "Answer", issue: "Issue" };
// 板に出す操作（表示名と送り方は sciedit.js の seOpLabel・seOpenAction・seQuick）
const ST_NODE_OPS = {
  Q: ["add_q", "add_o", "edit", "set_target", "remove"],
  O: ["star", "set_kind", "add_f", "add_e", "add_q", "copy", "edit", "remove"],
  F: ["edit", "remove"],
  E: ["add_f", "add_e", "edit", "remove"],
};

// --- 画面の状態 ---

let stData = null;      // 最後に読んだ status
let stTypesData = null; // 最後に読んだ /sci/types
let stNode = null;      // 選んだ Node の NodeDetail
let stNodeId = null;    // 今開いている Node の ID
let stSeedId = null;    // 今選んでいる SEED の札（S1 など）
let stSheet = null;     // 狭い画面の板（saiSheet が返した物）
let stSheetBody = null; // 板の中の入れ物
let stSheetFor = null;  // 板が出している物の鍵
let stListOpen = false; // 狭い画面で一覧を出しているか（出している間は図を隠す）

// ============================================================================
// DOM を使わない純関数（tests/test_scitree.js）
// ============================================================================

/** 住所を読む（`#/papers/tree` と `#/papers/tree/<ID>`）。合わなければ null。 */
function stParseHash(hash) {
  const m = /^#\/papers\/tree(?:\/([QOFES][1-9][0-9]{0,8}))?$/.exec(String(hash || ""));
  if (!m) return null;
  return m[1] ? { id: m[1] } : {};
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
    leaf: !!n.leaf,
    obj: n.objectivity || null,
    logic: n.logicality || null,
    counts: n.counts || null,
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

/** 進み具合（answered / total）。 */
function stProgress(status) {
  const p = (status && status.progress) || {};
  const answered = Number(p.answered) || 0;
  const total = Number(p.total) || 0;
  return { answered, total, ratio: total > 0 ? answered / total : 0 };
}

/** Next Action の先頭 n 件。 */
function stNextLines(status, n = 3) {
  const list = status && Array.isArray(status.next) ? status.next : [];
  return list.slice(0, n).map((x) => ({
    label: x.purpose || "",
    why: x.why || "",
    node: x.node || null,
    card: x.card || null,
    kind: x.kind,
    fix: x.fix || null,
  }));
}

/** DEBT の fix の移り先の住所。カードのフォームを開くもの（card）と住所の無いものは null。 */
function stFixHash(fix, projectName) {
  if (!fix) return null;
  switch (fix.kind) {
    case "triage": return "#/papers/triage";
    case "set": return `#/papers/sets/${fix.set_id}`;
    case "node": return `#/papers/tree/${fix.node}`;
    case "run": return `#/run/${encodeURIComponent(projectName)}/${fix.run_id}`;
    case "paper": return typeof paperHash === "function" ? paperHash(projectName, fix.paper_id, false) : null;
    default: return null;
  }
}

/** 決定の記録を新しい順に（at が同じなら元の順）。 */
function stDecisionRows(decisions) {
  const list = Array.isArray(decisions) ? decisions : [];
  return list
    .map((d, i) => ({ d, i }))
    .sort((a, b) => String(b.d.at || "").localeCompare(String(a.d.at || "")) || a.i - b.i)
    .map((x) => x.d);
}

/** 板に出す操作の並び（出さないものを除く）。 */
function stOpsFor(node, children) {
  if (!node) return [];
  const kids = Array.isArray(children) ? children : [];
  return (ST_NODE_OPS[node.kind] || []).filter((op) => {
    if (op === "add_o") return !kids.some((c) => c.kind === "Q");
    if (op === "add_e") return node.kind === "E" || node.answer_kind === "issue";
    if (op === "remove") return !(node.kind === "Q" && !node.parent);
    return true;
  }).map((op) => (op === "star" && node.starred ? "unstar" : op));
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

/** Node か SEED の札（S1）を選ぶ。 */
async function stSelect(id) {
  if (/^S[1-9][0-9]*$/.test(String(id || ""))) {
    stSeedId = id;
    stNodeId = null;
    stNode = null;
  } else {
    stSeedId = null;
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
  const id = stNodeId || stSeedId;
  const next = id ? `#/papers/tree/${id}` : "#/papers/tree";
  if (location.hash !== next) history.replaceState(null, "", next);
}

/** 画面を開く（app.js の route から呼ばれる）。 */
async function stOpen(params) {
  const id = (params && params.id) || null;
  stSeedId = id && id[0] === "S" ? id : null;
  stNodeId = id && !stSeedId ? id : null;
  stData = null;
  stNode = null;
  stListOpen = false;
  await ensurePaperProjects();
  fillSegment("sci-seg", "tree");
  updateProjectChips();
  const body = document.getElementById("sci-body");
  if (body) body.classList.add("sai-split");
  await stReload();
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
  stRenderSidebar(document.getElementById("sci-list"));
  stRenderBoard();
  stRenderGraph();
}

/** 「古い形の木」「SEED」「進み具合」「Next Action」「DEBT・PROPOSAL・注意」の帯。 */
function stRenderTop() {
  const box = document.getElementById("sci-top");
  if (!box) return;
  box.innerHTML = "";
  const s = stData;
  if (!s) return;
  if (!s.initialized) {
    box.appendChild(stEmpty({
      icon: "🌱",
      title: "研究方針の木がまだありません",
      body: "研究の型を選んで始めます。",
      action: { label: "始める", onClick: () => { void stStartTree(); } },
    }));
    return;
  }
  if (s.legacy) {
    box.appendChild(stEl("p", "sci-note sci-legacy", "古い形の木です。新しい木（Q・O・F・E）には読み替えていません。"));
  }
  box.appendChild(stSeedBand(s));
  box.appendChild(stProgressBand(s));
  box.appendChild(stNextBand(s));
  box.appendChild(stChipBand(s));
}

/** SEED の札と「SEED を作る」釦（.sci-seedband）。 */
function stSeedBand(s) {
  const band = stEl("div", "sci-band sci-seedband");
  const head = stEl("div", "sci-line");
  head.appendChild(stEl("span", "sci-band-title", "SEED"));
  const draft = stEl("p", "sci-seed");
  const parts = (s.seed_draft && s.seed_draft.parts) || [];
  if (parts.length) {
    for (const p of parts) {
      if (p.filled === false && p.key) {
        const b = stEl("button", "sci-seed-blank", p.text);
        b.type = "button";
        b.addEventListener("click", () => { void stSelectKey(p.key); });
        draft.appendChild(b);
      } else {
        draft.appendChild(document.createTextNode(p.text));
      }
    }
  } else {
    draft.textContent = (s.seed_draft && s.seed_draft.text) || "";
  }
  band.appendChild(head);
  band.appendChild(draft);
  const row = stEl("div", "sci-chips");
  for (const seed of s.seeds || []) {
    const b = stEl("button", "sci-chip sci-seed-chip", `${seed.id} ${seed.title || ""}`.trim());
    b.type = "button";
    b.addEventListener("click", () => { void stSelect(seed.id); });
    row.appendChild(b);
  }
  row.appendChild(stBtn("SEED を作る", () => { void stMakeSeed(); }));
  const t = stEl("button", `sci-list-toggle${stListOpen ? " active" : ""}`, "一覧");
  t.type = "button";
  t.setAttribute("aria-pressed", String(stListOpen));
  t.addEventListener("click", () => { stListOpen = !stListOpen; stSyncListOpen(); stRenderTop(); });
  row.appendChild(t);
  band.appendChild(row);
  return band;
}

/** draft の差し込み key の Q を選ぶ（key は Q の guide の名前。見つからなければ何もしない）。 */
async function stSelectKey(key) {
  const q = ((stData && stData.nodes) || []).find((n) => n.kind === "Q" && (n.key === key || (n.guide && n.guide.key === key)));
  if (q) await stSelect(q.id);
}

/** 進み具合が揃っていなければ確認してから add_seed を開く。 */
async function stMakeSeed() {
  const { answered, total } = stProgress(stData);
  if (answered < total && typeof saiConfirm === "function") {
    const ok = await saiConfirm({ title: "SEED を作る", body: `進み具合が揃っていません（${answered}/${total}）。作りますか？`, ok: "作る" });
    if (!ok) return;
  }
  stCall("seOpenAction", "add_seed", null, stData);
}

/** 進み具合のバー（.sci-progress）。 */
function stProgressBand(s) {
  const p = stProgress(s);
  const line = stEl("div", "sci-line sci-progress");
  const bar = stEl("div", "sci-bar");
  const fill = stEl("div", "sci-bar-fill");
  fill.style.width = `${Math.round(p.ratio * 100)}%`;
  bar.appendChild(fill);
  line.appendChild(bar);
  line.appendChild(stEl("span", "sci-id", `${p.answered}/${p.total}`));
  return line;
}

/** 解く釦。kind が card なら Create CARD、user なら Q を選ぶ、debt なら fix に従う。 */
function stActionBtn(x) {
  const project = selectedPaperProject();
  const card = (node, preset) => stBtn("Create CARD", () => { stCall("seOpenCard", node, preset); });
  if (x.kind === "card") return card(x.node, x.card);
  if (x.kind === "user") return stBtn("選ぶ", () => { void stSelect(x.node); });
  const fix = x.fix;
  if (fix && fix.kind === "card") return card(fix.card && fix.card.node, fix.card);
  const hash = stFixHash(fix, project);
  return hash ? stBtn("開く", () => stGo(hash)) : null;
}

/** Next Action の先頭 3 件（.sci-next-row）。 */
function stNextRows(list) {
  const wrap = stEl("div", "sci-next");
  for (const x of list) {
    const row = stEl("div", "sci-line sci-next-row");
    row.appendChild(stEl("span", "sci-line-main", `[${x.why}] ${x.label}`));
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
  else band.appendChild(stNextRows(list));
  return band;
}

/** DEBT・PROPOSAL・注意の札（.sci-chips）。 */
function stChipBand(s) {
  const row = stEl("div", "sci-chips");
  const n = s.debt_count != null ? s.debt_count : (s.debts || []).length;
  const debt = stEl("button", `sci-chip sci-debt ${n ? "warn" : "dim"}`, `DEBT ${n}`);
  debt.type = "button";
  debt.addEventListener("click", () => stDebtSheet(s));
  row.appendChild(debt);
  const pend = stEl("button", "sci-chip sci-proposal", `PROPOSAL ${s.pending_proposals || 0}`);
  pend.type = "button";
  pend.addEventListener("click", () => stGo("#/papers/triage"));
  row.appendChild(pend);
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
  return row;
}

/** DEBT の一覧を saiSheet で出す。 */
function stDebtSheet(s) {
  if (typeof saiSheet !== "function") return;
  const body = stEl("div", "sci-sheet-body");
  const debts = s.debts || [];
  if (!debts.length) body.appendChild(stEl("p", "sci-note", "DEBT はありません"));
  for (const d of debts) {
    const row = stEl("div", "sci-line sci-debt-row");
    row.appendChild(stEl("span", "sci-line-main", `⚠ run ${d.run_id} ${d.block || ""}: ${d.message || ""}`));
    const b = stActionBtn({ kind: "debt", fix: d.fix });
    if (b) row.appendChild(b);
    body.appendChild(row);
  }
  saiSheet({ title: "DEBT", body });
}

/** 研究の型を選んで木を始める。 */
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
  saiSheet({ title: "研究の型を選ぶ", body });
}

async function stInitTree(typeId, typeName) {
  if (typeof saiConfirm === "function") {
    const ok = await saiConfirm({ title: "始めますか？", body: `${typeName} で研究方針の木を始めます。`, ok: "始める" });
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
  const row = stEl("div", `sci-row sci-row-${r.kind}${r.onPath ? "" : " sci-dim"}${r.id === stNodeId ? " selected" : ""}`);
  row.dataset.id = r.id;
  row.style.paddingLeft = `${8 + (o.flat ? 0 : r.depth) * 14}px`;
  const main = stEl("button", "sci-row-main");
  main.type = "button";
  main.addEventListener("click", () => { void stSelect(r.id); });
  const head = stEl("span", "sci-row-head");
  head.appendChild(stEl("span", "sci-id", r.id));
  let title = r.text;
  if (r.kind === "F") title = title.slice(0, 60);
  head.appendChild(stEl("span", "sci-row-title", title));
  main.appendChild(head);
  const sub = stEl("span", "sci-row-sub");
  if (r.kind === "Q") {
    if (r.obj) sub.appendChild(stEl("span", `sci-obj${r.obj.met ? " met" : ""}`, stObjBar(r.obj)));
    const [label, tone] = stLogicLabel(r.logic);
    sub.appendChild(stEl("span", `sci-logic sci-logic-${tone}`, label));
  } else if (r.kind === "O") {
    if (r.counts && r.counts.f != null) sub.appendChild(stEl("span", "sci-id", `F ${r.counts.f}`));
  } else if (r.kind === "F") {
    if (r.node.cites) sub.appendChild(stEl("span", "sci-id", String(Array.isArray(r.node.cites) ? r.node.cites.join(", ") : r.node.cites)));
  } else if (r.kind === "E") {
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
  if (!(r.kind === "Q" && !r.node.parent)) {
    const x = stEl("button", "sci-remove", "✕");
    x.type = "button";
    x.setAttribute("aria-label", `${r.id} を消す`);
    x.addEventListener("click", () => { stCall("seOpenAction", "remove", r.id, r.node); });
    row.appendChild(x);
  }
  return row;
}

/** Answer と Issue を入れ替える（確認ののち）。 */
async function stToggleKind(r) {
  const next = r.answerKind === "answer" ? "issue" : "answer";
  if (typeof saiConfirm === "function") {
    const ok = await saiConfirm({
      title: "種類を変える",
      body: `${r.id} を ${ST_KIND_LABEL[r.answerKind]} から ${ST_KIND_LABEL[next]} に変えます。`,
      ok: "変える",
    });
    if (!ok) return;
  }
  stCall("seQuick", "set_kind", { o: r.id, kind: next });
}

function stRenderSidebar(box) {
  if (!box) return;
  box.innerHTML = "";
  if (!stData || !stData.initialized) return;
  const rows = stRows(stData);
  if (!rows.length) {
    box.appendChild(stEmpty({ title: "Node がありません", body: "" }));
    return;
  }
  for (const r of rows) box.appendChild(stRowEl(r));
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
    box.appendChild(stEl("p", "sci-note", "方針の図はまだ使えません"));
    return;
  }
  sgRenderGraph(box, { status: stData }, (sel) => {
    if (sel && (sel.kind === "node" || sel.kind === "seed")) void stSelect(sel.id);
  });
  stMarkGraph();
}

function stMarkGraph() {
  if (typeof sgMarkSelected === "function") sgMarkSelected(stNodeId || stSeedId || null);
}

// ============================================================================
// 右の板（.sci-board / saiSheet）
// ============================================================================

function stBoardKey() {
  return stNodeId || stSeedId || "decisions";
}

/** 板の中身を作る。 */
function stSelBoard() {
  if (stNodeId && stNode) return { title: `${stNode.node.id}`, el: stBuildBoard(stNode) };
  if (stSeedId) return { title: stSeedId, el: stBuildSeedBoard() };
  return { title: "決定の記録", el: stDecisionsEl(stDecisionRows(stData && stData.decisions)) };
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
  if (box && saiIsWide()) {
    stCloseSheet();
    box.innerHTML = "";
    box.hidden = false;
    if (body) body.classList.add("has-board");
    const head = stEl("div", "sci-line sci-board-head");
    head.appendChild(stEl("h2", "sci-board-title", board.title));
    if (stNodeId || stSeedId) {
      const x = stEl("button", "sci-board-close", "✕");
      x.type = "button";
      x.setAttribute("aria-label", "閉じる");
      x.addEventListener("click", () => { void stClearSelection(); });
      head.appendChild(x);
    }
    box.appendChild(head);
    box.appendChild(board.el);
    return;
  }
  if (box) { box.innerHTML = ""; box.hidden = true; }
  if (body) body.classList.remove("has-board");
  // 狭い画面は、選んだときだけ下から出す
  if (!stNodeId && !stSeedId) { stCloseSheet(); return; }
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
  stNodeId = null; stSeedId = null; stNode = null;
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

/** 決定の記録（.sci-decisions）。 */
function stDecisionsEl(rows) {
  const box = stEl("div", "sci-decisions");
  if (!rows.length) box.appendChild(stEl("p", "sci-note", "まだ決定の記録はありません"));
  for (const d of rows) {
    const line = stEl("div", "sci-line sci-decision");
    line.appendChild(stEl("span", "sci-id", String(d.at || "")));
    const main = stEl("span", "sci-line-main", `${d.node || ""} ${d.node_text || d.text || ""}`.trim());
    line.appendChild(main);
    line.appendChild(stEl("span", "sci-chip", String(d.action || "")));
    if (d.reason) line.appendChild(stEl("p", "sci-line-sub", d.reason));
    box.appendChild(line);
  }
  return box;
}

/** SEED を選んだときの板。 */
function stBuildSeedBoard() {
  const box = stEl("div", "sci-card");
  const seed = ((stData && stData.seeds) || []).find((s) => s.id === stSeedId) || {};
  const sec = stSec(`${stSeedId} ${seed.title || ""}`.trim());
  sec.appendChild(stMd(seed.text || ""));
  box.appendChild(sec);
  const path = seed.path || seed.options || [];
  if (path.length) {
    const p = stSec("道の O");
    const row = stEl("div", "sci-chips");
    for (const id of path) {
      const b = stEl("button", "sci-chip", String(id));
      b.type = "button";
      b.addEventListener("click", () => { void stSelect(id); });
      row.appendChild(b);
    }
    p.appendChild(row);
    box.appendChild(p);
  }
  return box;
}

/** Node の板。節の順は計画 U18 の 4。 */
function stBuildBoard(d) {
  const node = d.node || {};
  const box = stEl("div", "sci-card");
  box.appendChild(stBoardHead(node));
  box.appendChild(stBoardPath(d.path));
  box.appendChild(stBoardOps(d));
  if (node.kind === "Q") {
    box.appendChild(stBoardObjectivity(node));
    box.appendChild(stBoardLogic(node));
    box.appendChild(stBoardGuide(d.guide));
  }
  box.appendChild(stBoardChildren(d.children));
  const next = stSec("Next Action");
  const nl = stNextLines({ next: d.next }, 3);
  if (nl.length) next.appendChild(stNextRows(nl));
  else next.appendChild(stEl("p", "sci-note", "なし"));
  box.appendChild(next);
  box.appendChild(stBoardActions(d.actions));
  box.appendChild(stBoardPending(d.pending));
  box.appendChild(stBoardSearches(d.searches));
  const dec = stSec("決定の記録");
  dec.appendChild(stDecisionsEl(stDecisionRows(d.decisions)));
  box.appendChild(dec);
  return box;
}

/** 見出し（ID・種類・文・印）と本文。 */
function stBoardHead(node) {
  const sec = stEl("section", "sci-sec sci-node-head");
  const line = stEl("div", "sci-line");
  line.appendChild(stEl("span", "sci-id", `${node.id} ${node.kind}`));
  if (node.kind === "O") {
    if (node.starred) line.appendChild(stEl("span", "sci-chip", "★"));
    if (node.answer_kind) line.appendChild(stEl("span", `sci-chip sci-kind-${node.answer_kind}`, ST_KIND_LABEL[node.answer_kind]));
  }
  if (node.kind === "Q") {
    if (node.objectivity) line.appendChild(stEl("span", `sci-obj${node.objectivity.met ? " met" : ""}`, `Objectivity ${stObjBar(node.objectivity)}`));
    const [label, tone] = stLogicLabel(node.logicality);
    line.appendChild(stEl("span", `sci-logic sci-logic-${tone}`, `Logicality ${label}`));
  }
  sec.appendChild(line);
  sec.appendChild(stEl("p", "sci-node-title", node.text || ""));
  if (node.body) sec.appendChild(stMd(node.body));
  return sec;
}

/** 親までの道（.sci-path）。 */
function stBoardPath(path) {
  const sec = stSec("親までの道");
  const row = stEl("div", "sci-chips sci-path");
  for (const p of path || []) {
    const b = stEl("button", "sci-chip", `${p.id} ${(p.text || "").slice(0, 20)}`.trim());
    b.type = "button";
    b.addEventListener("click", () => { void stSelect(p.id); });
    row.appendChild(b);
  }
  sec.appendChild(row);
  return sec;
}

/** 操作の釦と Create CARD（.sci-ops）。 */
function stBoardOps(d) {
  const node = d.node;
  const sec = stSec("操作");
  const ops = stEl("div", "sci-ops");
  for (const op of stOpsFor(node, d.children)) {
    const label = typeof seOpLabel === "function" ? seOpLabel(op) : op;
    ops.appendChild(stBtn(label, () => { stCall("seOpenAction", op, node.id, d); }));
  }
  ops.appendChild(stBtn("Create CARD", () => { stCall("seOpenCard", node.id, null); }));
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
  const sec = stSec("ガイド");
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

/** 子の行（一覧と同じ書き方）。 */
function stBoardChildren(children) {
  const sec = stSec("子");
  const rows = stRows({ nodes: children });
  if (!rows.length) sec.appendChild(stEl("p", "sci-note", "なし"));
  for (const r of rows) sec.appendChild(stRowEl(r, { flat: true }));
  return sec;
}

/** Action の履歴。押すとその run へ。 */
function stBoardActions(actions) {
  const sec = stSec("Action の履歴");
  const project = selectedPaperProject();
  if (!(actions || []).length) sec.appendChild(stEl("p", "sci-note", "なし"));
  for (const a of actions || []) {
    const debts = a.debt_messages || [];
    const b = stEl("button", "sci-action", `${debts.length ? "⚠ " : ""}run ${a.run_id} ${a.block || ""} ${a.owner || ""} ${a.purpose || ""}`.replace(/\s+/g, " ").trim());
    b.type = "button";
    b.addEventListener("click", () => stGo(`#/run/${encodeURIComponent(project)}/${a.run_id}`));
    sec.appendChild(b);
    for (const m of debts) sec.appendChild(stEl("p", "sci-line-sub", m));
  }
  return sec;
}

/** 仕分け待ち。 */
function stBoardPending(pending) {
  const sec = stSec("仕分け待ち");
  if (!(pending || []).length) sec.appendChild(stEl("p", "sci-note", "なし"));
  for (const p of pending || []) {
    sec.appendChild(typeof seProposalRow === "function" ? seProposalRow(p) : stEl("p", "sci-line-sub", p.title || p.id || ""));
  }
  return sec;
}

/** 検索の記録。set_id があれば押すと検索セットへ。 */
function stBoardSearches(searches) {
  const sec = stSec("検索の記録");
  if (!(searches || []).length) sec.appendChild(stEl("p", "sci-note", "なし"));
  for (const s of searches || []) {
    const text = String(s.query || s.title || s.set_id || "");
    if (s.set_id) {
      const b = stEl("button", "sci-action", text);
      b.type = "button";
      b.addEventListener("click", () => stGo(`#/papers/sets/${s.set_id}`));
      sec.appendChild(b);
    } else {
      sec.appendChild(stEl("p", "sci-line-sub", text));
    }
  }
  return sec;
}

// ============================================================================
// 失敗・プロジェクト無しの表示
// ============================================================================

function stClearAll() {
  stCloseSheet();
  for (const id of ["sci-graph", "sci-top", "sci-list"]) {
    const e = document.getElementById(id);
    if (e) e.innerHTML = "";
  }
  const b = document.getElementById("sci-board");
  if (b) { b.innerHTML = ""; b.hidden = true; }
  return document.getElementById("sci-list");
}

function stRenderError(e) {
  const list = stClearAll();
  if (list) list.appendChild(stEmpty({ icon: "⚠️", title: "研究方針を読めませんでした", body: (e && e.message) || "読み直してください" }));
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
    tab: "papers",
    hideTabbar: false,
    interval: 0, // 定期更新はしない（↻ で読み直す）
    async open(params) { await stOpen(params); },
    async poll() {},
  });

  // 消した画面の古い住所（#/papers/canvas・#/canvas/…）は研究方針へ送る
  window.SaiRoutes.push({
    match(hash) {
      const h = String(hash || "");
      return h === "#/papers/canvas" || h.startsWith("#/canvas/") ? {} : null;
    },
    view: "view-sci-tree",
    tab: "papers",
    hideTabbar: false,
    interval: 0,
    async open() { location.replace("#/papers/tree"); },
    async poll() {},
  });

  const stReloadBtn = document.getElementById("sci-reload");
  if (stReloadBtn) stReloadBtn.addEventListener("click", () => { void stReload(); });
}

// --- node からの読み込み（tests/test_scitree.js） ---
if (typeof module !== "undefined" && module.exports) {
  module.exports = {
    stParseHash, stRows, stObjBar, stLogicLabel, stProgress, stNextLines,
    stFixHash, stDecisionRows, stOpsFor,
    ST_TYPES, ST_LOGIC, ST_KIND_LABEL, ST_NODE_OPS,
  };
}
