"use strict";
// sai-phone の Canvas の左右の欄（実装計画 U07）。
// 左の欄は論文と、その note.md / report の木。題名は論文画面（#/paper/<プロジェクト>/<論文番号>）へ行く。
// 右の欄は選んだ札の中身（note.md / report の本文）と「Canvas に無いつながり」の一覧。
//
// canvas.js（U06）の共有契約 C-F を使う。
//   読むだけ : cvState / cvApi / cvCardKind / CV_EDGE_KINDS
//   頼む     : cvSelect（選択の変え方）/ cvReload（読み直し）
// このファイルから cvState は書き換えない。
//
// ユーザーから挙げられた問題を起こさないことを優先している:
//   - 題名は note.md ではなく論文画面へ飛ばす
//   - 出し入れは toggle のあとに cvReload() するだけ（位置は sai-paper 側が覚えている）
//   - 右の欄を閉じているときも中身は作る（◧ で開いたときにすぐ見えるように）
//
// DOM を使わない純関数（絞り込み・盤面にあるか・描く行・無いつながりの色）は
// 末尾で node に渡す（tests/test_canvaspanel.js）。

// --- 定数・状態 ---

// 畳んだ論文を覚える場所（localStorage。プロジェクトと Canvas ごとに分ける）
const CVP_FOLDS_KEY = "sai_phone_canvas_folds";

// 一覧の絞り込みに入力している文字（画面の中だけ。サーバーには聞かない）
let cvpFilterText = "";
// 最後に描いた盤面（プロジェクトと Canvas のPairsが変わったら絞り込みを消す）
let cvpBoardKey = "";
// 送信中の札（同じ札を二度押させない）
const cvpBusy = new Set();
// 右の欄を作り直した回数（古い応答で上書きしないために使う）
let cvpSeq = 0;
// 出し入れの失敗の文言（左右の欄の上に出す）
let cvpErrorText = "";

// ============================================================================
// DOM を使わない純関数（node のテストからも使う）
// ============================================================================

/**
 * 題名に text を含む論文だけを残す（大文字・小文字を区別しない。空なら全部）。
 * 判定は題名だけ。note.md の先頭行や report の題名は見ない。
 * @param {Array} sidebar show の応答の sidebar
 * @param {string} text 入力した文字
 * @returns {Array} 残す論文（元の要素をそのまま返す）
 */
function cvpFilterSidebar(sidebar, text) {
  const list = Array.isArray(sidebar) ? sidebar : [];
  const q = String(text == null ? "" : text).trim().toLowerCase();
  if (!q) return list.slice();
  return list.filter((p) => {
    if (!p) return false;
    return String(p.title == null ? "" : p.title).toLowerCase().includes(q);
  });
}

/**
 * その行の札が盤面にあるか（● か ○ か）。
 * 同じ論文の 2 種類の札（note-<P> / paper-<P>）は、どちらが出ても「ある」とみなす。
 * @param {string} nodeId 行の node_id（note-<P> / report-<R>）
 * @param {number|string} paperId その行が属する論文の paper_id
 * @param {Array} nodes show の応答の nodes（隠した札は含まれない）
 * @returns {boolean} 盤面にあるか
 */
function cvpIsOnBoard(nodeId, paperId, nodes) {
  const ids = new Set();
  for (const n of Array.isArray(nodes) ? nodes : []) {
    if (n && n.id != null) ids.add(String(n.id));
  }
  const id = nodeId == null ? "" : String(nodeId);
  if (id && ids.has(id)) return true;
  // note.md の行（note-<P>）は、note.md の無い論文の札（paper-<P>）が
  // 盤面にあっても ● にする（同じ論文の札とみなす）。
  if (id.startsWith("note-") && paperId != null && String(paperId) !== "") {
    return ids.has(`paper-${paperId}`);
  }
  return false;
}

/**
 * 左の欄で描く行を順番に並べる（畳んだ論文の子は出さない）。
 * 1 論文につき「論文の行」→「note.md の行」→「report の行（0 枚以上）」の順。
 * @param {Array} sidebar show の応答の sidebar
 * @param {Array} folds 畳んでいる論文番号の配列
 * @returns {Array} {type:"paper"|"note"|"report", paper_id, node_id, label, summary, title}
 *          title は report の題名（其它は空文字列）
 */
function cvpRows(sidebar, folds) {
  const list = Array.isArray(sidebar) ? sidebar : [];
  const folded = new Set((Array.isArray(folds) ? folds : []).map((x) => String(x)));
  const rows = [];
  for (const p of list) {
    if (!p) continue;
    const paperId = p.paper_id == null ? null : p.paper_id;
    rows.push({
      type: "paper",
      paper_id: paperId,
      node_id: null,
      label: String(p.title == null ? "" : p.title),
      summary: "",
      title: "",
    });
    if (folded.has(paperId == null ? "" : String(paperId))) continue;
    const note = p.note;
    if (note && note.node_id) {
      rows.push({
        type: "note",
        paper_id: paperId,
        node_id: String(note.node_id),
        label: "note.md",
        summary: String(note.summary == null ? "" : note.summary),
        title: "",
      });
    }
    for (const r of Array.isArray(p.reports) ? p.reports : []) {
      if (!r || !r.node_id) continue;
      rows.push({
        type: "report",
        paper_id: paperId,
        node_id: String(r.node_id),
        label: String(r.report_id == null ? "" : r.report_id),
        summary: String(r.summary == null ? "" : r.summary),
        title: String(r.title == null ? "" : r.title),
      });
    }
  }
  return rows;
}

/**
 * その論文自身の札（note-<P> / paper-<P>）が盤面にあるか。
 * 左の欄の論文の行を少し濃くするときに使う（report の札は数えない）。
 * @param {number|string} paperId 論文番号
 * @param {Array} nodes show の応答の nodes
 */
function cvpPaperOnBoard(paperId, nodes) {
  const key = paperId == null ? "" : String(paperId);
  if (!key) return false;
  for (const n of Array.isArray(nodes) ? nodes : []) {
    const id = n && n.id != null ? String(n.id) : "";
    if (id === `note-${key}` || id === `paper-${key}`) return true;
  }
  return false;
}

/** 線の種類の色（canvas.js の表 CV_EDGE_KINDS を読む。表に無い種類は灰色）。 */
function cvpEdgeColorOf(kind) {
  const table = typeof CV_EDGE_KINDS !== "undefined" && Array.isArray(CV_EDGE_KINDS) ? CV_EDGE_KINDS : [];
  for (const row of table) {
    if (row && row[0] === kind) return row[2];
  }
  return "#8a8f98";
}

/**
 * 「Canvas に無いつながり」の 1 行の色（線の種類と同じ表で決める）。
 * 引用 / 使った / 元にした / 言及 / それ以外（線）。
 * @param {string} relation missing の items[].relation
 */
function cvpRelationColor(relation) {
  const t = String(relation == null ? "" : relation);
  if (t.includes("引用")) return cvpEdgeColorOf("citation");
  if (t.includes("使った")) return cvpEdgeColorOf("uses");
  if (t.includes("元にした")) return cvpEdgeColorOf("derived");
  if (t.includes("言及")) return cvpEdgeColorOf("mentions");
  return cvpEdgeColorOf("link");
}

// ============================================================================
// 小さな DOM ヘルパ（canvas.js の el に依存しない）
// ============================================================================

/** 要素を作る。text を渡せば textContent に入れる（HTML にはしない）。 */
function cvpEl(tag, cls, text) {
  const node = document.createElement(tag);
  if (cls) node.className = cls;
  if (text != null) node.textContent = String(text);
  return node;
}

/** 押せるボタンを作る。 */
function cvpBtn(cls, text, aria) {
  const b = cvpEl("button", cls, text);
  b.type = "button";
  if (aria) b.setAttribute("aria-label", aria);
  return b;
}

// ============================================================================
// 記憶（畳んだ論文）
// ============================================================================

/** 畳みを覚える場所（同じ Canvas なら覚えておく）。 */
function cvpFoldKey() {
  const s = typeof cvState !== "undefined" && cvState ? cvState : { project: "", name: "" };
  return `${CVP_FOLDS_KEY}:${s.project}/${s.name}`;
}

/** 覚えている畳み（論文番号の配列）を読む。読めなければ空。 */
function cvpLoadFolds() {
  try {
    const raw = localStorage.getItem(cvpFoldKey());
    if (!raw) return [];
    const obj = JSON.parse(raw);
    return Array.isArray(obj) ? obj : [];
  } catch { return []; }
}

/** 畳みを覚える。 */
function cvpSaveFolds(list) {
  try {
    localStorage.setItem(cvpFoldKey(), JSON.stringify(Array.isArray(list) ? list : []));
  } catch { /* 動けばよい */ }
}

/** 論文を畳む / 開く。 */
function cvpToggleFold(paperId) {
  const key = String(paperId == null ? "" : paperId);
  const list = cvpLoadFolds().map((x) => String(x));
  const at = list.indexOf(key);
  if (at >= 0) list.splice(at, 1);
  else list.push(key);
  cvpSaveFolds(list);
}

// ============================================================================
// 札の出し入れ（POST /toggle → cvReload → cvSelect）
// ============================================================================

/**
 * 札の出し入れをする。送っている間は同じ札を二度押させない。
 * 位置は sai-paper 側（saiHidden）が覚えているので、ここでは計算しない。
 * @param {string} nodeId 出したり隠したりする札
 * @param {string|null} near 近くに出す札（無ければ null）
 * @param {{select?:string|null, focus?:boolean}} after 出したあとに選ぶ札と、画面の中央へ動かすか
 * @returns {Promise<boolean>} 成功したか
 */
async function cvpToggleNode(nodeId, near, after) {
  const id = nodeId == null ? "" : String(nodeId);
  if (!id || cvpBusy.has(id)) return false;
  cvpBusy.add(id);
  cvpErrorText = "";
  let ok = false;
  let shown = false;
  try {
    const res = await apiFetch(cvApi("/toggle"), {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ node_id: id, near: near == null ? null : String(near) }),
    });
    if (!res.ok) {
      cvpErrorText = await resDetail(res, "出し入れできませんでした");
      ok = false;
    } else {
      const body = await res.json().catch(() => ({}));
      shown = !body || body.shown !== false;
      ok = true;
    }
  } catch (e) {
    cvpErrorText = (e && e.message) || "出し入れできませんでした";
    ok = false;
  }
  cvpBusy.delete(id);
  if (!ok) {
    // 失敗したぶんだけ左右の欄を作り直す（このファイルの中だけで直す）
    cvpRenderLeft();
    cvpRenderRight(cvState.selected == null ? null : cvState.selected);
    return false;
  }
  // 取り直して描き直してから選ぶ（cvReload のあとで cvSelect する）
  await cvReload();
  if (!shown) cvSelect(null);
  else if (after && after.select != null) cvSelect(String(after.select), { focus: !!after.focus });
  else cvSelect(null);
  return true;
}

// ============================================================================
// 左の欄
// ============================================================================

/** 左右の欄を作り直す。cvState.data が変わるたびに U06 から呼ばれる。 */
function cvpRender() {
  const key = `${cvState.project}/${cvState.name}`;
  if (key !== cvpBoardKey) {
    // 別の盤面に移った。絞り込みは残さない
    cvpBoardKey = key;
    cvpFilterText = "";
  }
  if (cvpBusy.size === 0) cvpErrorText = "";
  cvpRenderLeft();
  cvpRenderRight(cvState.selected == null ? null : cvState.selected);
}

/** 選択が変わったときに U06 から呼ばれる。右の欄だけ入れ替える。 */
function cvpOnSelect(nodeId) {
  if (cvpBusy.size === 0) cvpErrorText = "";
  cvpRenderRight(nodeId == null ? null : nodeId);
}

/** 左の欄（絞り込み欄と論文の木）を作り直す。 */
function cvpRenderLeft() {
  const box = document.getElementById("cv-left");
  if (!box) return;
  box.innerHTML = "";
  cvpErrorLine(box);
  if (!cvState.data) {
    box.appendChild(cvpEl("p", "cvp-hint", "読み込んでいます…"));
    return;
  }
  // 絞り込み欄。入力のたびに論文の列だけを作り直すので、入力欄は残らない
  const filter = cvpEl("input", "cvp-filter");
  filter.type = "search";
  filter.placeholder = "題名で絞り込み";
  filter.value = cvpFilterText;
  filter.setAttribute("aria-label", "題名で絞り込み");
  const list = cvpEl("div", "cvp-list");
  filter.addEventListener("input", () => {
    cvpFilterText = filter.value;
    cvpFillList(list);
  });
  box.appendChild(filter);
  box.appendChild(list);
  cvpFillList(list);
}

/** 左の欄の論文の列を描く。 */
function cvpFillList(box) {
  const data = cvState.data || {};
  const all = Array.isArray(data.sidebar) ? data.sidebar : [];
  const sidebar = cvpFilterSidebar(all, cvpFilterText);
  const nodes = Array.isArray(data.nodes) ? data.nodes : [];
  const folds = cvpLoadFolds();
  const folded = new Set(folds.map((x) => String(x)));
  box.innerHTML = "";
  if (!sidebar.length) {
    box.appendChild(cvpEl("p", "cvp-hint",
      cvpFilterText ? "その文字を含む論文はありません" : "note か report のある論文がありません"));
    return;
  }
  for (const row of cvpRows(sidebar, folds)) {
    box.appendChild(row.type === "paper"
      ? cvpPaperRow(row, folded.has(String(row.paper_id)), nodes)
      : cvpChildRow(row, nodes));
  }
}

/** 論文の行（▾ / 題名 → 論文画面 / #論文番号）。 */
function cvpPaperRow(row, isFolded, nodes) {
  const head = cvpEl("div", "cvp-paper");
  const fold = cvpBtn("cvp-fold", isFolded ? "▸" : "▾", isFolded ? "開く" : "畳む");
  fold.addEventListener("click", () => {
    cvpToggleFold(row.paper_id);
    cvpRenderLeft();
  });
  head.appendChild(fold);
  // 題名は論文画面へ。論文画面の ‹ で盤面に戻れるよう、移る前に戻る先を覚える
  if (row.paper_id == null) {
    head.appendChild(cvpEl("span", "cvp-paper-title", row.label || "(無題)"));
  } else {
    const link = cvpEl("a", "cvp-paper-title", row.label || "(無題)");
    link.href = paperHash(cvState.project, row.paper_id);
    link.addEventListener("click", () => { savePapersList(location.hash); });
    head.appendChild(link);
  }
  if (row.paper_id != null) head.appendChild(cvpEl("span", "cvp-paper-id", `#${row.paper_id}`));
  // 同じ論文の札が盤面にあるときだけ題名を少し濃くする（● の行と重複させない）
  if (cvpPaperOnBoard(row.paper_id, nodes)) head.classList.add("on");
  return head;
}

/** 子の行（● / ○ のボタンと、押せる本体）。 */
function cvpChildRow(row, nodes) {
  const on = cvpIsOnBoard(row.node_id, row.paper_id, nodes);
  const box = cvpEl("div", "cvp-row");
  if (cvState.selected != null && String(cvState.selected) === row.node_id) box.classList.add("selected");

  // ● / ○。送っている間は押せない
  const dot = cvpBtn("cvp-dot", on ? "●" : "○", on ? "盤面から隠す" : "盤面に出す");
  dot.setAttribute("aria-pressed", on ? "true" : "false");
  dot.title = on ? "押すと盤面から隠します" : "押すと盤面に出します";
  if (on) dot.classList.add("on");
  if (cvpBusy.has(row.node_id)) dot.disabled = true;
  dot.addEventListener("click", () => {
    // 近くの札は、今選んでいる札（自分自身は近くに置かない）
    const sel = cvState.selected == null ? null : cvState.selected;
    const near = String(sel) === row.node_id ? null : sel;
    void cvpToggleNode(row.node_id, near, { select: row.node_id, focus: true });
  });
  box.appendChild(dot);

  // 本体。押したらその札へ移す。盤面に無いときは動かさない
  const main = cvpBtn("cvp-row-main", null, on ? "選ぶ" : "○ を押すと盤面に出します");
  const line = cvpEl("span", "cvp-row-line");
  line.appendChild(cvpEl("span", "cvp-row-label", row.label));
  if (row.title) line.appendChild(cvpEl("span", "cvp-row-title", row.title));
  if (!on) line.appendChild(cvpEl("span", "cvp-row-off", "○ で盤面へ"));
  main.appendChild(line);
  if (row.summary) main.appendChild(cvpEl("span", "cvp-row-sum", row.summary));
  if (on) main.addEventListener("click", () => { cvSelect(row.node_id, { focus: true }); });
  box.appendChild(main);
  return box;
}

// ============================================================================
// 右の欄
// ============================================================================

/**
 * 右の欄（選んだ札の中身と「Canvas に無いつながり」）を作り直す。
 * 閉じているとき（display:none）でも中身は作る（◧ で開いたときにすぐ見える）。
 */
function cvpRenderRight(nodeId) {
  const box = document.getElementById("cv-right");
  if (!box) return;
  cvpSeq += 1;
  const seq = cvpSeq;
  box.innerHTML = "";
  cvpErrorLine(box);

  const node = cvpNodeById(nodeId);
  if (!node) {
    box.appendChild(cvpEl("p", "cvp-hint", "札を選ぶと中身が出ます"));
    box.appendChild(cvpLegend());
    return;
  }

  box.appendChild(cvpRightHead(node));

  const body = cvpEl("div", "cvp-body");
  body.appendChild(cvpEl("p", "cvp-hint", "読み込み中…"));
  box.appendChild(body);
  void cvpLoadBody(nodeId, seq, body, node);

  // 「Canvas に無いつながり」は札（card）のときだけ
  if (node.type === "card") {
    const miss = cvpEl("div", "cvp-missing");
    miss.appendChild(cvpEl("h3", "cvp-h3", "Canvas に無いつながり"));
    miss.appendChild(cvpEl("p", "cvp-hint", "読み込み中…"));
    box.appendChild(miss);
    void cvpLoadMissing(nodeId, seq, miss);
  }
}

/** 線の種類の凡例（選んでいないときに出す）。 */
function cvpLegend() {
  const box = cvpEl("ul", "cvp-legend");
  const table = typeof CV_EDGE_KINDS !== "undefined" && Array.isArray(CV_EDGE_KINDS) ? CV_EDGE_KINDS : [];
  for (const row of table) {
    if (!row) continue;
    const li = cvpEl("li", "cvp-legend-row");
    const dot = cvpEl("span", "cvp-legend-dot");
    dot.style.background = row[2];
    li.appendChild(dot);
    li.appendChild(cvpEl("span", "cvp-legend-label", String(row[1])));
    box.appendChild(li);
  }
  return box;
}

/** 見出し（種類の印・題名・副題・論文画面へ / 盤面から隠す）。 */
function cvpRightHead(node) {
  const head = cvpEl("div", "cvp-head");
  const kind = typeof cvCardKind === "function" ? cvCardKind(node.kind) : { key: "", label: "", color: "#8a8f98" };
  const mark = cvpEl("span", "cvp-kind", kind.label);
  mark.style.color = kind.color;
  head.appendChild(mark);

  const text = cvpEl("div", "cvp-head-text");
  text.appendChild(cvpEl("div", "cvp-head-title", String(node.title || "(無題)")));
  if (node.subtitle) text.appendChild(cvpEl("div", "cvp-head-sub", String(node.subtitle)));
  head.appendChild(text);

  const actions = cvpEl("div", "cvp-head-actions");
  // 論文の札には論文画面へ（note.md ではなく論文そのものへ行く）
  if ((kind.key === "note" || kind.key === "paper") && node.paper_id != null) {
    const link = cvpEl("a", "cvp-link", "論文画面へ ›");
    link.href = paperHash(cvState.project, node.paper_id);
    link.addEventListener("click", () => { savePapersList(location.hash); });
    actions.appendChild(link);
  }
  // 札（note / paper / report）は盤面から隠せる
  if (node.type === "card") {
    const hide = cvpBtn("cvp-hide", "盤面から隠す");
    if (cvpBusy.has(node.id)) hide.disabled = true;
    hide.addEventListener("click", () => { void cvpToggleNode(node.id, null, {}); });
    actions.appendChild(hide);
  }
  head.appendChild(actions);
  return head;
}

/** 札の中身（GET .../nodes/{id}/body）。 */
async function cvpLoadBody(nodeId, seq, box, node) {
  let data = null;
  try {
    const res = await apiFetch(cvApi(`/nodes/${encodeURIComponent(nodeId)}/body`));
    if (!res.ok) {
      cvpFail(seq, box, await resDetail(res, "中身を読めませんでした"));
      return;
    }
    data = await res.json().catch(() => ({}));
  } catch (e) {
    cvpFail(seq, box, (e && e.message) || "中身を読めませんでした");
    return;
  }
  if (seq !== cvpSeq) return; // 選択が変わったので古い応答は捨てる
  box.innerHTML = "";
  const md = data && typeof data.markdown === "string" ? data.markdown : null;
  if (md != null) {
    // renderMarkdown は文字をすべてエスケープした HTML を返す（文書中の HTML 及其它は文字のまま）
    box.innerHTML = renderMarkdown(md);
    return;
  }
  // md が無いとき
  const kindKey = String((data && data.kind) || node.kind || "");
  if (node.type === "group" || kindKey === "group") {
    box.appendChild(cvpEl("p", "cvp-hint", String(node.label || "枠です")));
    return;
  }
  if (kindKey === "paper" || kindKey === "note") {
    box.appendChild(cvpEl("p", "cvp-hint", "note.md はまだありません（論文画面で書けます）"));
    return;
  }
  box.appendChild(cvpEl("p", "cvp-hint", "本文がありません"));
}

/** 「Canvas に無いつながり」（GET .../nodes/{id}/missing）。 */
async function cvpLoadMissing(nodeId, seq, box) {
  const fail = (text) => {
    if (seq !== cvpSeq) return;
    box.innerHTML = "";
    box.appendChild(cvpEl("h3", "cvp-h3", "Canvas に無いつながり"));
    box.appendChild(cvpEl("p", "cvp-err", String(text)));
  };
  let data = null;
  try {
    const res = await apiFetch(cvApi(`/nodes/${encodeURIComponent(nodeId)}/missing`));
    if (!res.ok) {
      fail(await resDetail(res, "一覧を読めませんでした"));
      return;
    }
    data = await res.json().catch(() => ({}));
  } catch (e) {
    fail((e && e.message) || "一覧を読めませんでした");
    return;
  }
  if (seq !== cvpSeq) return; // 選択が変わったので古い応答は捨てる
  const items = data && Array.isArray(data.items) ? data.items : [];
  box.innerHTML = "";
  box.appendChild(cvpEl("h3", "cvp-h3", "Canvas に無いつながり"));
  const list = cvpEl("div", "cvp-missing-list");
  for (const item of items) {
    if (!item || !item.node_id) continue;
    const row = cvpEl("div", "cvp-missing-row");
    const rel = cvpEl("span", "cvp-rel", String(item.relation || "線"));
    rel.style.color = cvpRelationColor(item.relation);
    row.appendChild(rel);
    row.appendChild(cvpEl("span", "cvp-missing-title", String(item.title || item.node_id)));
    // ＋ で盤面へ足す（選んだ札の近くへ出る。選択は元の札のまま）
    const add = cvpBtn("cvp-add", "＋", "盤面に出す");
    if (cvpBusy.has(String(item.node_id))) add.disabled = true;
    add.addEventListener("click", () => {
      void cvpToggleNode(String(item.node_id), nodeId, { select: nodeId });
    });
    row.appendChild(add);
    list.appendChild(row);
  }
  if (list.childNodes.length) box.appendChild(list);
  else box.appendChild(cvpEl("p", "cvp-hint", "すべて盤面にあります"));
}

// ============================================================================
// 枝（失敗の表示・小さな補助）
// ============================================================================

/** 出し入れの失敗の 1 行。 */
function cvpErrorLine(box) {
  if (cvpErrorText) box.appendChild(cvpEl("p", "cvp-err", cvpErrorText));
}

/** 読み込みの失敗（古い応答なら何もしない）。 */
function cvpFail(seq, box, text) {
  if (seq !== cvpSeq) return;
  box.innerHTML = "";
  box.appendChild(cvpEl("p", "cvp-err", String(text || "読めませんでした")));
}

/** 盤面にあるノード（隠した札は含まれない）。無ければ null。 */
function cvpNodeById(id) {
  if (id == null) return null;
  const data = cvState.data || {};
  const list = Array.isArray(data.nodes) ? data.nodes : [];
  const key = String(id);
  return list.find((n) => n && String(n.id) === key) || null;
}

// --- node からの読み込み（tests/test_canvaspanel.js） ---
if (typeof module !== "undefined" && module.exports) {
  module.exports = {
    cvpFilterSidebar,
    cvpIsOnBoard,
    cvpPaperOnBoard,
    cvpRows,
    cvpRelationColor,
  };
}
