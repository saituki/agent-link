"use strict";
// 読む札（#/papers/read/<run_id>）の画面（実装計画 U20・共有契約 C-14）。
//
// 読む札は人が読む札（C-09 の runner = user）。答えは知見（出どころ = 利用者）になる。
// 知見が 1 つも無いまま終えるときは、反映しない理由を必ず聞く（R-A5）。
//
// 使う関数の決まりは C-14:
//   srOpen(params)             … params.runId
//   srOpenReadCard(nodeId, detail)  … 読む札を作る小窓（論文・役割・深さを選ぶ）
// scitree.js はこの 2 つの関数を typeof で確かめてから釦を出す（無ければ出さない）。
//
// 中の処理は `phone sci read` / `phone sci card` / `phone sci apply` と、
// 読む札の「始め」「終え」の 2 つだけを使う（C-13）。ファイルのパスは受け取らない。
//
// DOM を使わない純関数（srParseHash / srBuildAnswer / srFinishBody）は末尾で node に渡す
// （tests/test_sciread.js）。一番外側の名前は sr で始める（tests/test_globals.js）。

// --- 定数（C-01 / C-16 / C-09。sciedit.js の SE_* とは別の物） ---

// 読む札の型（C-01 の sci_mode）
const SR_MODES = [["purpose_first", "目的先行"], ["blind_first", "目的後付け"]];
// 読む深さ（C-01）
const SR_DEPTHS = [["abstract", "抄録"], ["figures", "図"], ["full", "精読"]];
// 論文の役割（C-01）
const SR_ROLES = [
  ["exemplar", "代表例"],
  ["compare", "比較対象"],
  ["issue_evidence", "課題の根拠"],
  ["method_source", "手法の供給元"],
  ["data_source", "データの出典"],
  ["background", "背景"],
  ["counter", "反証"],
];
// 知見の向き（C-01。既定は支持）
const SR_POLARITY = [["support", "支持"], ["refute", "反証"], ["neutral", "中立"], ["none", "該当なし"]];
// 置き場の表示（C-01 / C-12。owner が todo なら「読み始める」を出す）
const SR_OWNERS = {
  todo: "まだ読んでいない",
  running: "読んでいる",
  review: "振り返りを待っている",
  done: "終わり",
};
// 出典の形（C-07。`#37` か `#37 §3.2`）
const SR_CITE_RE = /^#[1-9][0-9]{0,8}( .{1,80})?$/;

// --- 画面の状態 ---

let srCard = null;     // 最後に読んだ読む札の JSON（C-08 の read）
let srRunId = null;    // 今開いている読む札の run の番号
let srHost = null;     // 小窓（saiSheet が返した物）

// ============================================================================
// DOM を使わない純関数（tests/test_sciread.js）
// ============================================================================

/** 前後の空白を落とした文字（null は空文字）。 */
function srTrim(value) {
  return value == null ? "" : String(value).trim();
}

/** 住所を読む（`#/papers/read/130` だけ）。合わなければ null。 */
function srParseHash(hash) {
  const m = /^#\/papers\/read\/([1-9][0-9]{0,8})$/.exec(String(hash || ""));
  if (!m) return null;
  return { runId: Number(m[1]) };
}

/** 置く節点の ID（C-08 の read の node.id）。無ければ空文字。 */
function srCardNode(card) {
  const node = card && card.node;
  if (!node) return "";
  return srTrim(node.id);
}

/** 読む論文の ID（C-08 の read の paper.id）。無ければ 0。 */
function srCardPaper(card) {
  const p = card && card.paper;
  const id = Number(p && p.id);
  return Number.isInteger(id) && id > 0 ? id : 0;
}

/**
 * 答え（C-08 の read の 1 つ分から add_finding の JSON を作る。
 * 出典は `#<論文>`。場所が空でなければ `#37 §3.2` の形になる（C-07）。
 *   → { ok: true, json: {...}, error: null } または { ok: false, json: null, error: "理由" }
 */
function srBuildAnswer(card, values) {
  const c = card || {};
  const v = values || {};
  const bad = (error) => ({ ok: false, json: null, error });
  const good = (json) => ({ ok: true, json, error: null });
  const node = srCardNode(c);
  const paper = srCardPaper(c);
  if (!node) return bad("NODE が分かりません");
  if (!paper) return bad("論文が分かりません");
  const text = srTrim(v.text);
  if (!text) return bad("文を入力してください");
  if (text.length > 1200) return bad("文は 1200 字までです");
  const place = srTrim(v.place);
  if (place.length > 80) return bad("場所は 80 字までです");
  const cite = place ? `#${paper} ${place}` : `#${paper}`;
  if (!SR_CITE_RE.test(cite)) return bad("出典の形が正しくありません");
  const polarity = SR_POLARITY.some((x) => x[0] === srTrim(v.polarity)) ? srTrim(v.polarity) : "support";
  const json = {
    op: "add_finding",
    node,
    text,
    cites: [cite],
    polarity,
    paper,
    run: Number(c.run_id) || undefined,
  };
  if (json.run === undefined) delete json.run;
  // 問いの ID（Q12 の形）だけを送る。読む札の問いは文なので送らない（送るとサーバが ID の形で断る）
  if (/^Q[1-9][0-9]{0,8}$/.test(srTrim(v.question))) json.question = srTrim(v.question);
  return good(json);
}

/**
 * 終えるときの本文。
 * 知見が 1 つ以上なら {}。0 なら理由（A5）を聞く。理由が空なら null（送らない）。
 */
function srFinishBody(findings, reason) {
  const list = Array.isArray(findings) ? findings : [];
  if (list.length > 0) return {};
  const why = srTrim(reason);
  if (!why) return null;
  return { no_reflect: why };
}

/** 出典の行（#37 §3.2 の形）。表示と入力のどちらにも使う。 */
function srCite(paperId, place) {
  const id = Number(paperId) || 0;
  const p = srTrim(place);
  return p ? `#${id} ${p}` : `#${id}`;
}

// ============================================================================
// 小さい DOM を作る手（ui.js の saiEl と同じ形）
// ============================================================================

function srEl(tag, cls, text) {
  const e = document.createElement(tag);
  if (cls) e.className = cls;
  if (text != null) e.textContent = text;
  return e;
}

function srBtn(label, onClick, cls) {
  const b = srEl("button", `sai-btn ${cls || ""}`.trim(), label);
  b.type = "button";
  if (typeof onClick === "function") b.addEventListener("click", onClick);
  else b.disabled = true;
  return b;
}

function srChip(text, tone) {
  return srEl("span", `sai-chip ${tone || "muted"}`, text);
}

function srLine(cls) {
  return srEl("div", `sci-read-line ${cls || ""}`.trim());
}

function srNote(text) {
  return srEl("p", "sci-read-note", text || "");
}

function srLabel(text) {
  return srEl("span", "sci-read-label", text);
}

/** 用語の札（ui.js の saiTerm が無ければ null）。 */
function srTerm(key) {
  if (typeof saiTerm !== "function") return null;
  return saiTerm(key);
}

/** 節（見出しつき）。 */
function srSec(title, termKey) {
  const sec = srEl("section", "sci-read-sec");
  if (title) {
    const head = srLine("sci-read-sec-head");
    head.appendChild(srEl("h2", "sci-read-sec-title", title));
    const term = termKey ? srTerm(termKey) : null;
    if (term) head.appendChild(term);
    sec.appendChild(head);
  }
  return sec;
}

/** 節点ごとの詳細へ戻る釦。 */
function srBackHash(card) {
  const id = srCardNode(card);
  return id ? `#/papers/tree/${id}` : "#/papers/tree";
}

/** 節点に結び付いている論文（[{id, link, title, role}]）。`id` は論文の番号。 */
function srPaperLinks(detail) {
  const links = detail && Array.isArray(detail.links) ? detail.links : [];
  return links
    .filter((l) => l && /^paper:[1-9][0-9]{0,8}$/.test(String(l.target || "")))
    .map((l) => ({
      id: Number(String(l.target).split(":")[1]),
      link: l.id || "",
      title: l.title || l.target,
      role: l.role || "",
    }));
}

/** 論文の画面を開く（scitree.js の stPaperHash と同じ形）。 */
function srPaperHash(paperId) {
  if (typeof stPaperHash === "function" && typeof selectedPaperProject === "function") {
    return stPaperHash(paperId);
  }
  const name = typeof selectedPaperProject === "function" ? selectedPaperProject() : "";
  return name
    ? `#/paper/${encodeURIComponent(name)}/${encodeURIComponent(paperId)}`
    : `#/paper/${encodeURIComponent(paperId)}`;
}

// ============================================================================
// 読む札の画面
// ============================================================================

/** 画面を開く（app.js の route から呼ばれる）。 */
async function srOpen(params) {
  if (typeof document === "undefined") return;
  const p = params || {};
  srRunId = Number(p.runId) || Number(p.run_id) || 0;
  srCard = null;
  const body = document.getElementById("sr-body");
  const title = document.getElementById("sr-title");
  if (title) title.textContent = "READ_CARD";
  if (typeof ensurePaperProjects === "function") await ensurePaperProjects();
  if (!body) return;
  if (!srRunId) {
    body.innerHTML = "";
    body.appendChild(saiEmpty({ icon: "❓", title: "READ_CARD の番号が分かりません", body: "住所が「#/papers/read/130」の形ではありません。" }));
    return;
  }
  if (typeof selectedPaperProject !== "function" || !selectedPaperProject()) {
    body.innerHTML = "";
    const msg = typeof NO_PROJECT_MSG === "string" ? NO_PROJECT_MSG : "論文の置き場があるプロジェクトがありません";
    body.appendChild(saiEmpty({ icon: "📁", title: "論文の置き場がありません", body: msg }));
    return;
  }
  await srReload();
}

/** 読む札を読み直して描き直す。 */
async function srReload() {
  const body = document.getElementById("sr-body");
  if (!body || !srRunId) return;
  body.innerHTML = "";
  let card = null;
  try {
    card = await paperApi(stApi(`/reads/${encodeURIComponent(srRunId)}`));
  } catch (e) {
    body.appendChild(saiEmpty({
      icon: "⚠️",
      title: "READ_CARD を読めませんでした",
      body: (e && e.message) || "もう一度読み込んでください。",
    }));
    return;
  }
  srCard = card || null;
  srRender(card || {});
}

/** 読む札を描く。 */
function srRender(card) {
  const body = document.getElementById("sr-body");
  if (!body) return;
  const title = document.getElementById("sr-title");
  const paper = card.paper || {};
  if (title) title.textContent = paper.title ? `READ_CARD #${card.run_id || srRunId}` : "READ_CARD";
  const box = srEl("div", "sci-read-card");
  box.appendChild(srHead(card));
  if (card.owner === "todo") box.appendChild(srStartRow(card));
  box.appendChild(srQuestions(card));
  box.appendChild(srFindings(card));
  box.appendChild(srFinishRow(card));
  body.appendChild(box);
}

/** 見出し（論文・節点・目的・役割・深さ）。 */
function srHead(card) {
  const sec = srSec("この READ_CARD", "sci_scope");
  const node = card.node || {};
  const paper = card.paper || {};
  const head = srLine("sci-read-head");
  if (paper.title) {
    const b = srEl("button", "sci-read-paper", paper.title);
    b.type = "button";
    b.addEventListener("click", () => { location.hash = srPaperHash(paper.id); });
    head.appendChild(b);
  }
  if (paper.year) head.appendChild(srChip(String(paper.year), "muted"));
  if (card.owner && SR_OWNERS[card.owner]) {
    head.appendChild(srChip(SR_OWNERS[card.owner], card.owner === "todo" ? "warn" : "ok"));
  }
  sec.appendChild(head);

  const meta = srLine("sci-read-meta");
  if (node.id) {
    const b = srEl("button", "sci-read-node", `${node.id} ${node.title || ""}`.trim());
    b.type = "button";
    b.addEventListener("click", () => { location.hash = `#/papers/tree/${node.id}`; });
    meta.appendChild(b);
  }
  const role = (SR_ROLES.find((x) => x[0] === card.role) || [])[1] || card.role;
  if (role) meta.appendChild(srChip(`役割 ${role}`, "muted"));
  const depth = (SR_DEPTHS.find((x) => x[0] === card.depth) || [])[1] || card.depth;
  if (depth) meta.appendChild(srChip(`深さ ${depth}`, "muted"));
  if (meta.childNodes.length) sec.appendChild(meta);

  if (card.purpose) sec.appendChild(srEl("p", "sci-read-purpose", card.purpose));
  const back = srBtn("研究方針へ戻る", () => { location.hash = srBackHash(card); }, "ghost");
  sec.appendChild(back);
  return sec;
}

/** 「読み始める」（owner が todo のときだけ）。 */
function srStartRow(card) {
  const row = srLine("sci-read-start");
  const b = srBtn("読み始める", async () => {
    b.disabled = true;
    try {
      await paperApi(stApi(`/reads/${encodeURIComponent(card.run_id || srRunId)}/start`), { method: "POST" });
      saiToast("読み始めました", "ok");
      await srReload();
    } catch (e) {
      saiToast((e && e.message) || "始められませんでした", "error");
      b.disabled = false;
    }
  }, "primary");
  row.appendChild(b);
  return row;
}

/** 問いごとの答えの欄。問いが無ければ 1 つだけ出す。 */
function srQuestions(card) {
  const list = Array.isArray(card.questions) ? card.questions.filter(Boolean) : [];
  const sec = srSec("QUESTION に答える", "sci_finding");
  if (!list.length) {
    sec.appendChild(srNote("この READ_CARD には QUESTION がありません。分かったことを 1 つだけ下に書いてください。"));
    sec.appendChild(srAnswerForm(card, "", list));
    return sec;
  }
  for (const q of list) {
    sec.appendChild(srAnswerForm(card, q, list));
  }
  return sec;
}

/**
 * 答え 1 つの欄（問い・文・場所・向き）。
 * 送るものは add_finding（C-07）。node は節点、paper は読む論文、run はこの run、
 * cites は `#<論文> <場所>`（場所が空なら `#<論文>`）。向きの既定は支持。
 */
function srAnswerForm(card, question, allQuestions) {
  const paper = card.paper || {};
  const wrap = srEl("div", "sci-read-answer");
  const head = srLine("sci-read-answer-head");
  head.appendChild(srEl("span", "sci-read-answer-q", question ? String(question) : "QUESTION はありません"));
  wrap.appendChild(head);

  // 問いは 1 つ決まっているときは欄を出さない（答えの形が分かるので）。無いときだけ選べるようにする
  const qSelect = srEl("select", "sai-input sci-read-select");
  const none = document.createElement("option");
  none.value = "";
  none.textContent = "なし";
  qSelect.appendChild(none);
  for (const q of allQuestions || []) {
    const o = document.createElement("option");
    o.value = String(q);
    o.textContent = String(q);
    qSelect.appendChild(o);
  }
  qSelect.value = question ? String(question) : "";
  const qWrap = srEl("div", "sci-read-form-field");
  qWrap.appendChild(srEl("label", "sci-read-form-label", "QUESTION"));
  qWrap.appendChild(qSelect);
  if (!question) wrap.appendChild(qWrap);

  const text = document.createElement("textarea");
  text.className = "sai-input sci-read-area";
  text.rows = 3;
  text.placeholder = "分かったことを 1〜3 文で書く（必須）";
  const textWrap = srEl("div", "sci-read-form-field");
  textWrap.appendChild(srEl("label", "sci-read-form-label", "文"));
  textWrap.appendChild(text);
  wrap.appendChild(textWrap);

  const place = document.createElement("input");
  place.type = "text";
  place.className = "sai-input sci-read-input";
  place.placeholder = paper.id ? `§3.2（空なら #${paper.id}）` : "§3.2";
  const placeWrap = srEl("div", "sci-read-form-field");
  placeWrap.appendChild(srEl("label", "sci-read-form-label", "場所"));
  placeWrap.appendChild(place);
  wrap.appendChild(placeWrap);

  const pol = srEl("select", "sai-input sci-read-select");
  for (const [value, text2] of SR_POLARITY) {
    const o = document.createElement("option");
    o.value = value;
    o.textContent = text2;
    pol.appendChild(o);
  }
  pol.value = "support";
  const polWrap = srEl("div", "sci-read-form-field");
  polWrap.appendChild(srEl("label", "sci-read-form-label", "向き"));
  polWrap.appendChild(pol);
  wrap.appendChild(polWrap);

  const err = srEl("p", "sci-read-error");
  err.hidden = true;
  err.setAttribute("role", "alert");
  wrap.appendChild(err);

  const actions = srEl("div", "sci-read-actions");
  const send = srBtn("FINDING を足す", async () => {
    send.disabled = true;
    const values = { text: text.value, place: place.value, polarity: pol.value, question: qSelect.value };
    const res = await srSendAnswer(card, values, (why) => {
      err.textContent = why;
      err.hidden = false;
    });
    if (res) {
      text.value = "";
      place.value = "";
      err.hidden = true;
    } else {
      send.disabled = false;
    }
  }, "primary");
  actions.appendChild(send);
  wrap.appendChild(actions);
  return wrap;
}

/** 入れた知見の一覧。 */
function srFindings(card) {
  const sec = srSec("入れた FINDING", "sci_finding");
  const list = Array.isArray(card.findings) ? card.findings.filter(Boolean) : [];
  if (!list.length) {
    sec.appendChild(srNote("まだ FINDING を置いていません。1 つも無いまま終えると、反映しない理由を聞かれます。"));
    return sec;
  }
  for (const f of list) {
    const row = srLine("sci-read-finding");
    const main = srEl("div", "sci-read-finding-main");
    const h = srLine("sci-read-answer-head");
    if (f.id) h.appendChild(srEl("span", "sci-id", f.id));
    const pol = (SR_POLARITY.find((x) => x[0] === f.polarity) || [])[1] || f.polarity;
    h.appendChild(srChip(pol || "支持", f.polarity === "refute" ? "danger" : "muted"));
    main.appendChild(h);
    main.appendChild(srEl("p", "sci-read-finding-text", f.text || ""));
    row.appendChild(main);
    sec.appendChild(row);
  }
  return sec;
}

/** 「終える」。 */
function srFinishRow(card) {
  const sec = srSec("終える", "sci_assume");
  sec.appendChild(srNote("終えると実行は review へ移ります。分かったことが 1 つも無いときは、反映しない理由を書いてもらいます。"));
  const row = srLine("sci-read-finish");
  row.appendChild(srBtn("終える", () => { void srFinish(card); }, "primary"));
  sec.appendChild(row);
  return sec;
}

// ============================================================================
// 読む札の答えと終え
// ============================================================================

/** 答えを送る。 */
async function srSendAnswer(card, values, onError) {
  const built = srBuildAnswer(card, values);
  if (!built.ok) {
    if (typeof onError === "function") onError(built.error);
    return null;
  }
  try {
    const res = await paperApi(stApi("/apply"), { method: "POST", body: built.json });
    saiToast("FINDING を足しました", "ok");
    await srReload();
    return res;
  } catch (e) {
    if (typeof onError === "function") onError((e && e.message) || "FINDING を足せませんでした");
    else saiToast((e && e.message) || "FINDING を足せませんでした", "error");
  }
  return null;
}

/** 終える（知見が 0 なら理由を聞く）。 */
async function srFinish(card) {
  const findings = Array.isArray(card.findings) ? card.findings.filter(Boolean) : [];
  let body = srFinishBody(findings, "");
  if (!body) {
    const why = await saiPrompt({
      title: "反映しない理由",
      body: "分かったことが 1 つもありません。反映しない理由を書いてください（1 語でも構いません）。",
      ok: "この理由で終える",
    });
    if (why === null) return; // やめた
    body = srFinishBody(findings, why);
    if (!body) {
      saiToast("理由を書いてから終えてください", "error");
      return;
    }
  }
  const runId = card.run_id || srRunId;
  let res = null;
  try {
    res = await paperApi(stApi(`/reads/${encodeURIComponent(runId)}/finish`), { method: "POST", body });
  } catch (e) {
    saiToast((e && e.message) || "終えられませんでした", "error");
    return;
  }
  const status = (res && res.status) || "";
  saiToast(status === "review" ? `run ${runId} を review へ移しました` : "終えました", "ok");
  if (typeof stReload === "function") { try { await stReload(); } catch { /* 木は後で読み直す */ } }
  location.hash = srBackHash(card);
}

// ============================================================================
// 読む札を作る小窓（scitree.js の「読む札を作る」から呼ばれる）
// ============================================================================

/** 読む札を作る小窓。論文（1 本）・役割・深さ・目的を選ぶ。 */
function srOpenReadCard(nodeId, detail) {
  if (typeof document === "undefined" || typeof saiSheet !== "function") return;
  const d = detail || {};
  const node = d.node || {};
  const papers = srPaperLinks(d);
  const title = node.title || "この ITEM";
  const body = srEl("div", "sci-read-wrap");
  const el = srEl("div", "sci-read-form");

  // 入力欄（この小窓だけ）
  const mkLabel = (text) => srEl("label", "sci-read-form-label", text);

  const pick = srEl("select", "sai-input sci-read-select");
  for (const p of papers) {
    const o = document.createElement("option");
    o.value = String(p.id);
    o.textContent = `#${p.id} ${p.title}`;
    pick.appendChild(o);
  }
  const pickWrap = srEl("div", "sci-read-form-field");
  pickWrap.appendChild(mkLabel("論文（1 本）"));
  pickWrap.appendChild(pick);
  el.appendChild(pickWrap);

  const roleSel = srEl("select", "sai-input sci-read-select");
  const roleOpts = SR_ROLES.slice();
  if (d.role && !roleOpts.some((x) => x[0] === d.role)) roleOpts.unshift([d.role, "（既定の役割）"]);
  for (const [value, text] of roleOpts) {
    const o = document.createElement("option");
    o.value = value;
    o.textContent = text;
    roleSel.appendChild(o);
  }
  if (d.role) roleSel.value = d.role;
  const roleWrap = srEl("div", "sci-read-form-field");
  roleWrap.appendChild(mkLabel("役割"));
  roleWrap.appendChild(roleSel);
  el.appendChild(roleWrap);
  const depthSel = srEl("select", "sai-input sci-read-select");
  for (const [value, text] of SR_DEPTHS) {
    const o = document.createElement("option");
    o.value = value;
    o.textContent = text;
    depthSel.appendChild(o);
  }
  depthSel.value = "full"; // 既定は精読
  const depthWrap = srEl("div", "sci-read-form-field");
  depthWrap.appendChild(mkLabel("深さ"));
  depthWrap.appendChild(depthSel);
  el.appendChild(depthWrap);

  const purpose = document.createElement("input");
  purpose.type = "text";
  purpose.className = "sai-input sci-read-input";
  purpose.placeholder = `${title}のために #<論文> を読む`;
  const purposeWrap = srEl("div", "sci-read-form-field");
  purposeWrap.appendChild(mkLabel("目的"));
  purposeWrap.appendChild(purpose);
  el.appendChild(purposeWrap);
  /** 選んでいる論文（選択肢の値は論文の番号。並び順ではない）。 */
  const chosenPaper = () => papers.find((x) => String(x.id) === pick.value) || papers[0] || null;
  // 論文を変えたら役割と目的も合わせる（役割の既定は結び付きの役割）
  const syncPaper = () => {
    const p = chosenPaper();
    if (!p) return;
    if (p.role) roleSel.value = p.role;
    purpose.placeholder = `${title}のために #${p.id} を読む`;
  };
  pick.addEventListener("change", syncPaper);
  syncPaper();

  const err = srEl("p", "sci-read-error");
  err.hidden = true;
  err.setAttribute("role", "alert");
  el.appendChild(err);
  const actions = srEl("div", "sci-read-actions");
  el.appendChild(actions);
  body.appendChild(el);

  if (!papers.length) {
    err.textContent = "この NODE に LINK されている論文がありません。先に論文を LINK してください。";
    err.hidden = false;
  }
  const host = saiSheet({ title: "READ_CARD を作る", body });
  srHost = host;
  actions.appendChild(srBtn("やめる", () => host.close(), "ghost"));
  actions.appendChild(srBtn("作る", async () => {
    const pickPaper = chosenPaper();
    if (!pickPaper) { err.textContent = "論文を選んでください"; err.hidden = false; return; }
    const role = srTrim(roleSel.value);
    if (!role) { err.textContent = "役割を選んでください"; err.hidden = false; return; }
    const id = Number(pickPaper.id);
    const why = srTrim(purpose.value) || `${title}のために #${id} を読む`;
    if (why.length > 300) { err.textContent = "目的は 300 字までです"; err.hidden = false; return; }
    const json = {
      node: srTrim(nodeId) || srTrim(node.id),
      block: "sci-read",
      purpose: why,
      papers: [id],
      runner: "user",
      role,
      depth: depthSel.value,
      mode: "purpose_first",
    };
    if (!json.node) { err.textContent = "NODE が分かりません"; err.hidden = false; return; }
    let res = null;
    try {
      res = await paperApi(stApi("/cards"), { method: "POST", body: json });
    } catch (e) {
      err.textContent = (e && e.message) || "READ_CARD を作れませんでした";
      err.hidden = false;
      return;
    }
    const runId = (typeof seCardRunId === "function" && seCardRunId(res)) || (res && res.run_id) || null;
    host.close();
    saiToast(runId ? `run ${runId} の READ_CARD を作りました` : "READ_CARD を作りました", "ok");
    if (runId) location.hash = `#/papers/read/${encodeURIComponent(runId)}`;
  }, "primary"));
}

// ============================================================================
// 画面の登録（window.SaiRoutes）
// ============================================================================

if (typeof window !== "undefined" && typeof document !== "undefined") {
  (window.SaiRoutes = window.SaiRoutes || []).push({
    // 読む札（#/papers/read/130）
    match(hash) {
      return srParseHash(hash);
    },
    view: "view-sci-read",
    tab: "papers",
    hideTabbar: false,
    interval: 0, // 定期更新はしない
    async open(params) {
      await srOpen(params);
    },
    async poll() {},
  });

  const back = document.getElementById("sr-back");
  if (back) back.addEventListener("click", () => { location.hash = srBackHash(srCard); });
}

// --- node からの読み込み（tests/test_sciread.js） ---
if (typeof module !== "undefined" && module.exports) {
  module.exports = {
    srTrim,
    srParseHash,
    srCardNode,
    srCardPaper,
    srBuildAnswer,
    srFinishBody,
    srCite,
    srBackHash,
    SR_MODES,
    SR_DEPTHS,
    SR_ROLES,
    SR_POLARITY,
    SR_OWNERS,
    SR_CITE_RE,
  };
}
