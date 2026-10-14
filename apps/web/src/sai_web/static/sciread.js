"use strict";
// READ_CARD（#/research/read/<run_id>）の画面（U21）。
//
// READ_CARD は人が READ_CARD（runner = user）。利用者は論文を読み、A（または E）に F を付け、
// note.md を書いて終える。読み終わりの判定はコアが行い、終える操作は止めない。
//
// 中の処理は `GET …/sci/reads/<run_id>`・`…/start`・`…/finish`・`GET …/sci/nodes/<ID>`・
// `POST …/sci/apply`（add_f）だけを使う。ファイルのパスは受け取らない。
//
// DOM を使わない純関数（srParseHash / srBuildAnswer / srParents など）は末尾で node に渡す
// （tests/test_sciread.js）。一番外側の名前は sr で始める（tests/test_globals.js）。

// --- 定数 ---

// 読む深さ
const SR_DEPTHS = [["abstract", "抄録"], ["figures", "図"], ["full", "精読"]];
// F の stance（必須。値は API の STANCES。S 支持・D 反対・I 問い）
const SR_STANCES = [["support", "S 支持"], ["deny", "D 反対"], ["issue", "I 問い"]];
// 置き場の表示（owner が todo なら「始める」を出す）
const SR_OWNERS = {
  todo: "まだ読んでいない",
  running: "読んでいる",
  review: "振り返りを待っている",
  done: "終わり",
};
// 出典の形（C-CITE。`#37` / `#37.2.1-3` / `#37 §3.2`）
const SR_CITE_RE = /^#([1-9][0-9]{0,8})((?:\.[1-9][0-9]*){1,2}(?:-[1-9][0-9]*)?)?( .{1,80})?$/;

// --- 画面の状態 ---

let srCard = null;     // 最後に読んだ READ_CARD の JSON
let srDetail = null;   // 対象 Node の詳細（軸の A のとき、付ける先の A の候補に使う）
let srRunId = null;    // 今開いている READ_CARD の run の番号

// ============================================================================
// DOM を使わない純関数（tests/test_sciread.js）
// ============================================================================

/** 前後の空白を落とした文字（null は空文字）。 */
function srTrim(value) {
  return value == null ? "" : String(value).trim();
}

/** 住所を読む（`#/research/read/130` だけ）。合わなければ null。 */
function srParseHash(hash) {
  const m = /^#\/research\/read\/([1-9][0-9]{0,8})$/.exec(String(hash || ""));
  if (!m) return null;
  return { runId: Number(m[1]) };
}

/** 対象 Node の ID（card.node.id）。無ければ空文字。 */
function srCardNode(card) {
  const node = card && card.node;
  if (!node) return "";
  return srTrim(node.id);
}

/** 読む論文の ID（card.paper.id）。無ければ 0。 */
function srCardPaper(card) {
  const p = card && card.paper;
  const id = Number(p && p.id);
  return Number.isInteger(id) && id > 0 ? id : 0;
}

/**
 * F を付ける先の候補（Node の ID の配列）。
 * 対象が軸でない A ならそれ 1 つ（F は E の下に置かない）。軸の A なら detail.children の A（★ を先に）。
 */
function srParents(card, detail) {
  const node = (card && card.node) || {};
  const id = srTrim(node.id);
  if (saiIsAxis(node)) {
    const kids = detail && Array.isArray(detail.children) ? detail.children : [];
    const os = kids.filter((k) => k && k.kind === "A" && srTrim(k.id));
    return [...os.filter((k) => k.starred), ...os.filter((k) => !k.starred)].map((k) => srTrim(k.id));
  }
  if (node.kind === "A") return id ? [id] : [];
  return [];
}

/** 文の数（。．.！？!? の区切りで数える。末尾に句点が無い文も 1 つ）。 */
function srSentences(text) {
  return srTrim(text).split(/[。．.!?！？]+/).filter((s) => s.trim()).length;
}

/**
 * 答え（add_f の JSON）を作る。出典は `#<論文>`。場所が空でなければ `#37 §3.2` の形。
 *   → { ok: true, json, error: null } または { ok: false, json: null, error: "理由" }
 */
function srBuildAnswer(card, values) {
  const v = values || {};
  const bad = (error) => ({ ok: false, json: null, error });
  const paper = srCardPaper(card);
  if (!paper) return bad("論文が分かりません");
  const parent = srTrim(v.parent);
  if (!/^A[1-9][0-9]{0,8}$/.test(parent)) return bad("F を付ける先（A）を選んでください");
  if (!SR_STANCES.some((x) => x[0] === v.stance)) return bad("stance（S・D・I）を選んでください");
  const text = srTrim(v.text);
  if (!text) return bad("文を入力してください");
  if (text.length > 1200) return bad("文は 1200 字までです");
  if (srSentences(text) > 3) return bad("文は 3 文までです");
  const place = srTrim(v.place);
  if (place.length > 80) return bad("場所は 80 字までです");
  const cite = place ? `#${paper} ${place}` : `#${paper}`;
  if (!SR_CITE_RE.test(cite)) return bad("出典の形が正しくありません");
  const json = { op: "add_f", parent, text, cites: [cite], paper, stance: v.stance };
  // この札（run）で入れた F として残す（「この札で入れた F」に数える）
  const run = Number(card && card.run_id);
  if (Number.isInteger(run) && run > 0) json.run = run;
  return { ok: true, json, error: null };
}

/** 対象 Node の画面へ戻る住所。 */
function srBackHash(card) {
  const id = srCardNode(card);
  return id ? `#/research/tree/${id}` : "#/research/tree";
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

// ============================================================================
// READ_CARD の画面
// ============================================================================

/** 画面を開く（app.js の route から呼ばれる）。 */
async function srOpen(params) {
  if (typeof document === "undefined") return;
  const p = params || {};
  srRunId = Number(p.runId) || Number(p.run_id) || 0;
  srCard = null;
  srDetail = null;
  const body = document.getElementById("sr-body");
  const title = document.getElementById("sr-title");
  if (title) title.textContent = "READ_CARD";
  if (typeof ensurePaperProjects === "function") await ensurePaperProjects();
  if (!body) return;
  if (!srRunId) {
    body.innerHTML = "";
    body.appendChild(saiEmpty({ icon: "❓", title: "READ_CARD の番号が分かりません", body: "住所が「#/research/read/130」の形ではありません。" }));
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

/** READ_CARD を読み直して描き直す。 */
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
  // 対象が軸の A のときだけ、付ける先の A を探すために Node の詳細を読む
  srDetail = null;
  if (card && card.node && saiIsAxis(card.node)) {
    try {
      srDetail = await paperApi(stApi(`/nodes/${encodeURIComponent(card.node.id)}`));
    } catch { /* 候補が空になるだけ。F を足す枠でそう伝える */ }
  }
  srRender(card || {});
}

/** READ_CARD を描く（見出し → 始める → 問いと抜き出すもの → F を足す → F → note.md → 終える）。 */
function srRender(card) {
  const body = document.getElementById("sr-body");
  if (!body) return;
  const title = document.getElementById("sr-title");
  if (title) title.textContent = `READ_CARD #${card.run_id || srRunId}`;
  const box = srEl("div", "sci-read-card");
  box.appendChild(srHead(card));
  if (card.owner === "todo") box.appendChild(srStartRow(card));
  box.appendChild(srQuestions(card));
  box.appendChild(srAddForm(card));
  box.appendChild(srFindings(card));
  box.appendChild(srNoteSec(card));
  box.appendChild(srFinishRow(card));
  body.appendChild(box);
}

/** 見出し（論文の題・年・owner・対象 Node・深さ・目的）。 */
function srHead(card) {
  const sec = srSec("この READ_CARD");
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
    const b = srEl("button", "sci-read-node", `${node.id} ${node.text || ""}`.trim());
    b.type = "button";
    b.addEventListener("click", () => { location.hash = srBackHash(card); });
    meta.appendChild(b);
  }
  const depth = (SR_DEPTHS.find((x) => x[0] === card.depth) || [])[1] || card.depth;
  if (depth) meta.appendChild(srChip(`深さ ${depth}`, "muted"));
  if (meta.childNodes.length) sec.appendChild(meta);

  if (card.purpose) sec.appendChild(srEl("p", "sci-read-purpose", card.purpose));
  sec.appendChild(srBtn("research-tree へ戻る", () => { location.hash = srBackHash(card); }, "ghost"));
  return sec;
}

/** 論文の画面を開く住所。 */
function srPaperHash(paperId) {
  const name = typeof selectedPaperProject === "function" ? selectedPaperProject() : "";
  return paperHash(name, paperId, false);
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

/** 読むときの問いと抜き出すもの。 */
function srQuestions(card) {
  const sec = srSec("読むときの問い");
  const qs = Array.isArray(card.questions) ? card.questions.filter(Boolean) : [];
  const ex = Array.isArray(card.extract) ? card.extract.filter(Boolean) : [];
  if (!qs.length && !ex.length) {
    sec.appendChild(srNote("この CARD には問いも抜き出すものもありません。"));
    return sec;
  }
  for (const q of qs) sec.appendChild(srEl("p", "sci-read-answer-q", String(q)));
  if (ex.length) {
    sec.appendChild(srLabel("抜き出すもの"));
    for (const x of ex) sec.appendChild(srNote(`・${typeof x === "string" ? x : JSON.stringify(x)}`));
  }
  return sec;
}

/** F を足す枠（付ける先・文・場所）。 */
function srAddForm(card) {
  const paper = card.paper || {};
  const sec = srSec("F を足す");
  const parents = srParents(card, srDetail);
  if (!parents.length) {
    sec.appendChild(srNote("F を付ける A がありません。research-tree で A を足してから戻ってください。"));
    return sec;
  }
  const field = (label, input) => {
    const w = srEl("div", "sci-read-form-field");
    w.appendChild(srEl("label", "sci-read-form-label", label));
    w.appendChild(input);
    sec.appendChild(w);
  };
  const parent = srEl("select", "sai-input sci-read-select");
  for (const id of parents) {
    const o = document.createElement("option");
    o.value = id;
    o.textContent = id;
    parent.appendChild(o);
  }
  if (parents.length > 1) field("付ける先", parent);

  // stance（必須。押した釦が選ばれる。既定は無しで、選ぶまで送れない）
  let stance = "";
  const stanceRow = srEl("div", "sci-read-stance");
  const stanceBtns = SR_STANCES.map(([code, name]) => {
    const b = srBtn(`${code} ${name}`, () => {
      stance = code;
      for (const x of stanceBtns) x.classList.toggle("primary", x === b);
    }, "ghost");
    stanceRow.appendChild(b);
    return b;
  });
  field("stance（必須）", stanceRow);

  const text = document.createElement("textarea");
  text.className = "sai-input sci-read-area";
  text.rows = 3;
  text.placeholder = "分かったことを 1〜3 文で書く（必須）";
  field("文", text);

  const place = document.createElement("input");
  place.type = "text";
  place.className = "sai-input sci-read-input";
  place.placeholder = paper.id ? `§3.2（空なら #${paper.id}）` : "§3.2";
  field("場所", place);

  const err = srEl("p", "sci-read-error");
  err.hidden = true;
  err.setAttribute("aria-live", "assertive");
  sec.appendChild(err);

  const send = srBtn("F を足す", async () => {
    send.disabled = true;
    const built = srBuildAnswer(card, { parent: parent.value, text: text.value, place: place.value, stance });
    if (!built.ok) {
      err.textContent = built.error;
      err.hidden = false;
      send.disabled = false;
      return;
    }
    try {
      await paperApi(stApi("/apply"), { method: "POST", body: built.json });
      saiToast("F を足しました", "ok");
      await srReload();
    } catch (e) {
      err.textContent = (e && e.message) || "F を足せませんでした";
      err.hidden = false;
      send.disabled = false;
    }
  }, "primary");
  sec.appendChild(send);
  return sec;
}

/** この CARD で入れた F。 */
function srFindings(card) {
  const sec = srSec("この CARD で入れた F");
  const list = Array.isArray(card.findings) ? card.findings.filter(Boolean) : [];
  if (!list.length) {
    sec.appendChild(srNote("まだ F を入れていません。"));
    return sec;
  }
  for (const f of list) {
    const row = srLine("sci-read-finding");
    const main = srEl("div", "sci-read-finding-main");
    const h = srLine("sci-read-answer-head");
    if (f.id) h.appendChild(srEl("span", "sci-id", f.id));
    if (f.parent) h.appendChild(srChip(`→ ${f.parent}`, "muted"));
    main.appendChild(h);
    main.appendChild(srEl("p", "sci-read-finding-text", f.text || ""));
    row.appendChild(main);
    sec.appendChild(row);
  }
  return sec;
}

/** note.md（無ければ論文のノートの画面へ誘う）。 */
function srNoteSec(card) {
  const sec = srSec("note.md");
  if (card.note) {
    sec.appendChild(srNote("note.md があります。"));
    return sec;
  }
  sec.appendChild(srNote("note.md がまだありません。"));
  const paper = card.paper || {};
  if (paper.id) {
    sec.appendChild(srBtn("論文のノートを開く", () => { location.hash = srPaperHash(paper.id); }, "ghost"));
  }
  return sec;
}

/** 「終える」。F が 0 でも押せる。 */
function srFinishRow(card) {
  const sec = srSec("終える");
  sec.appendChild(srNote("終えると実行は review へ移ります。"));
  const row = srLine("sci-read-finish");
  const b = srBtn("終える", async () => {
    b.disabled = true;
    const runId = card.run_id || srRunId;
    try {
      await paperApi(stApi(`/reads/${encodeURIComponent(runId)}/finish`), { method: "POST", body: {} });
    } catch (e) {
      saiToast((e && e.message) || "終えられませんでした", "error");
      b.disabled = false;
      return;
    }
    saiToast(`run ${runId} を review へ移しました`, "ok");
    if (typeof stReload === "function") { try { await stReload(); } catch { /* 木は後で読み直す */ } }
    location.hash = srBackHash(card);
  }, "primary");
  row.appendChild(b);
  sec.appendChild(row);
  return sec;
}

// ============================================================================
// 画面の登録（window.SaiRoutes）
// ============================================================================

if (typeof window !== "undefined" && typeof document !== "undefined") {
  (window.SaiRoutes = window.SaiRoutes || []).push({
    // READ_CARD（#/research/read/130）
    match(hash) {
      return srParseHash(hash);
    },
    view: "view-sci-read",
    tab: "research",
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
    srParents,
    srBuildAnswer,
    srBackHash,
    SR_DEPTHS,
    SR_STANCES,
    SR_OWNERS,
    SR_CITE_RE,
  };
}
