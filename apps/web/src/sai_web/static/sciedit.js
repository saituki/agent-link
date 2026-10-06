"use strict";
// 仕分け箱（#/research/triage）と、Research-tree の操作の入力欄・カードのフォーム（実装計画 U19）。
//
// 板の釦は scitree.js から次の形で呼ばれる（この関数が無ければその釦は出さない）。
//   seOpenAction(op, nodeId, detail)   op は C-OP の 13 個。detail は node の JSON
//   seQuick(op, payload)               ★・Answer/Issue・目安は入力欄を出さずに送る
//   seOpenCard(nodeId, preset)         preset は C-CARD。値の入ったフォームを開く
//   seProposalRow(proposal, onDone)    → HTMLElement（採用・却下・取り消しの釦つき）
//   seOpenTriage()                     #/research/triage を開く
//
// 中の処理は `POST …/sci/apply` / `…/sci/cards` / `…/sci/proposals` だけを使う。
// この画面はファイルのパスを受け取らない。送る前に画面でも必須の項目と字数と出典を確かめる。
// 通らなければ送らず、小窓の中に理由を日本語で出す。最後の検査はサーバとコアが行う。
//
// DOM を使わない純関数（seBuildOp / seBuildCard / seCardValues / seGroupProposals など）は
// 末尾で node に渡す（tests/test_sciedit.js）。
// 一番外側の名前は se で始める（tests/test_globals.js）。

// --- 定数 ---

// op の表示名（C-OP の 13 個。表に無い op はそのまま出す。C4 の共有契約）
const SE_OPS = {
  add_q: "Add Q", add_o: "Add O", add_f: "Add F", add_e: "Add E",
  record_logic: "Logicality を記録", add_seed: "Sprout を作る", copy: "Copy O", remove: "Delete",
  star: "★ を付ける", unstar: "★ を外す", set_kind: "Answer / Issue を変える",
  edit: "Edit", set_target: "Objectivity の目安を変える",
};
// 反映したときに知らせる言葉（op ごとに 1 つ）
const SE_DONE = {
  add_q: "Q を足しました", add_o: "O を足しました", add_f: "F を足しました", add_e: "E を足しました",
  record_logic: "Logicality を記録しました", add_seed: "Sprout を作りました", copy: "O を写しました",
  remove: "消しました", star: "★ を付けました", unstar: "★ を外しました",
  set_kind: "Answer / Issue を変えました", edit: "文を直しました", set_target: "目安を変えました",
};
// 入力欄を出さずに送る op
const SE_QUICK = ["star", "unstar", "set_kind", "set_target"];
// 範囲
const SE_SCOPES = [
  ["node", "node"], ["path", "path"], ["subtree", "subtree"],
  ["siblings", "siblings"], ["summary", "summary"], ["blind", "blind"],
];
// 読む深さ
const SE_DEPTHS = [["abstract", "abstract"], ["figures", "figures"], ["full", "full"]];
// 実行するもの（user は読む札）
const SE_RUNNERS = [["pi", "pi"], ["cc", "cc"], ["user", "自分で読む（読む札）"]];
// 読む札の型
const SE_MODES = [["purpose_first", "purpose_first"], ["blind_first", "blind_first"]];
// O の種類
const SE_KINDS = [["answer", "Answer"], ["issue", "Issue"]];
// カードで選べる 21 本のブロック（Next Action に出るもの）
const SE_BLOCKS = [
  ["sci-start", "研究の入口: 最初の Q を立てる"],
  ["sci-split", "Q を分ける: 子の Q の案を出す"],
  ["sci-restore", "戻す: 消した Node を探して戻す案を出す"],
  ["sci-ideate", "O を増やす: Q の答えの候補を出す"],
  ["sci-search", "外を探す: 論文を探して集める"],
  ["sci-screen", "選別: 集めた論文を絞る"],
  ["sci-read", "1 本を読む: 論文から F を抜き出す"],
  ["sci-cite-check", "出典を確かめる: F の出典を照らす"],
  ["sci-debate", "議論: 賛否を出し合う"],
  ["sci-review", "見直す: 木を点検する"],
  ["sci-compare", "比べる: O を比べる"],
  ["sci-seed", "Sprout をつくる: Q から O を降りる道で Sprout を作る"],
  ["sci-digest", "まとめる: 今の状態をまとめる"],
  ["sci-coach", "理解の確認: 問いかけて確かめる"],
  ["sci-typify", "型に当てる: 型を当てはめる"],
  ["sci-exp-sprout", "実験の芽: 実験の案を出す"],
  ["sci-exp-branch", "実験を分ける: 実験を枝分かれさせる"],
  ["sci-exp-review", "実験を見直す: 結果を点検する"],
  ["sci-result-compare", "結果を比べる: 実験の結果を比べる"],
  ["sci-data-explore", "データを調べる: データ表・EDA・前処理"],
  ["sci-data-test", "データで確かめる: 計画どおりに検定して報告"],
];
// 出典の形（C-CITE。`#37`・`#37.3.2`・`#37.3.2-4 注`）
const SE_CITE_RE = /^#([1-9][0-9]{0,8})((?:\.[1-9][0-9]*){1,2}(?:-[1-9][0-9]*)?)?( .{1,80})?$/;
// ID の形（Q・O・F・E）
const SE_ID_RE = /^[QOFE][1-9][0-9]{0,8}$/;
// 仮置きの形（`Q9=値`）
const SE_ASSUME_RE = /^[QOFE][1-9][0-9]{0,8}=.+$/;
// 文字数の上限
const SE_MAX = { qtext: 300, otext: 120, ftext: 1200, etext: 120, body: 2000, reason: 500, note: 1000, mlflow: 200, title: 120, sentence: 600, purpose: 300, expect: 300 };

// --- 画面の状態 ---

let seTriageBy = "node";  // 仕分け箱のまとめ方（"node" か "run"）
let seBulkErrors = [];    // まとめて採用・却下で失敗した件（理由を出す）
// 仕分け箱の表示の切り替えを覚える場所（localStorage）
const SE_BY_KEY = "sai_web_sci_triage_by";

// ============================================================================
// DOM を使わない純関数（tests/test_sciedit.js）
// ============================================================================

/** 前後の空白を落とした文字（null は空文字）。 */
function seTrim(value) {
  return value == null ? "" : String(value).trim();
}

/** op の表示名。表に無い op はそのまま返す。 */
function seOpLabel(op) {
  const name = seTrim(op);
  return SE_OPS[name] || name;
}

/** 出典の形か。 */
function seCiteOk(cite) {
  return SE_CITE_RE.test(seTrim(cite));
}

/** ID の形か。prefix を渡すと、その頭の文字（"QO" のように複数可）だけ受け付ける。 */
function seRefOk(value, prefix) {
  const v = seTrim(value);
  return SE_ID_RE.test(v) && (!prefix || prefix.includes(v[0]));
}

/** 文の数を数える（`0.85` や `§3.2` の「.」は数えない。0 なら 1 文）。 */
function seCountSentences(text) {
  const s = seTrim(text);
  if (!s) return 0;
  const jp = (s.match(/[。？！]/g) || []).length;
  const en = (s.match(/[.?!](?=\s|$)/g) || []).length;
  return jp + en || 1;
}

/** 数字を含み出典が無いとき true。 */
function seNeedsCite(text, cites) {
  const list = (Array.isArray(cites) ? cites : []).filter((c) => seTrim(c) !== "");
  if (list.length) return false;
  return /[0-9]/.test(seTrim(text));
}

/** 文字の配列（空を落とす）。 */
function seList(value) {
  return (Array.isArray(value) ? value : []).map(seTrim).filter(Boolean);
}

/**
 * 小窓の値（values）から op の JSON を作る。
 *   → { ok: true, op: {...}, json: {...}, error: null } または { ok: false, op: null, json: null, error: "理由" }
 * 通らないものは日本語で理由を返す（画面では送らない）。reason は空でなければ付ける。
 */
function seBuildOp(kind, values) {
  const v = values || {};
  const name = seTrim(kind);
  const bad = (error) => ({ ok: false, op: null, json: null, error });
  const good = (op) => ({ ok: true, op, json: op, error: null });
  // 必須の ID を確かめる。prefix は許す頭の文字
  const id = (key, prefix, label) => (seRefOk(v[key], prefix) ? "" : `${label}を選んでください`);
  // 字数を確かめる。min が 1 なら必須
  const len = (key, label, min, max) => {
    const n = seTrim(v[key]).length;
    if (n < min) return `${label}を入力してください`;
    if (n > max) return `${label}は ${max} 字までです`;
    return "";
  };
  const withReason = (op) => {
    const reason = seTrim(v.reason);
    if (reason) op.reason = reason;
    return op;
  };
  const first = (...msgs) => msgs.find(Boolean) || "";

  switch (name) {
    case "add_q": {
      const msg = first(id("parent", "QO", "親（Q か O）"), len("text", "文", 1, SE_MAX.qtext), len("reason", "理由", 0, SE_MAX.reason));
      if (msg) return bad(msg);
      return good(withReason({ op: name, parent: seTrim(v.parent), text: seTrim(v.text) }));
    }
    case "add_o": {
      const kindV = seTrim(v.kind);
      const msg = first(id("q", "Q", "Q"), len("text", "文", 1, SE_MAX.otext), len("body", "本文", 0, SE_MAX.body), len("reason", "理由", 0, SE_MAX.reason));
      if (msg) return bad(msg);
      if (kindV && !SE_KINDS.some((x) => x[0] === kindV)) return bad("種類は Answer か Issue です");
      const op = { op: name, q: seTrim(v.q), text: seTrim(v.text) };
      if (seTrim(v.body)) op.body = seTrim(v.body);
      if (kindV) op.kind = kindV;
      return good(withReason(op));
    }
    case "add_f": {
      const msg = first(id("parent", "OE", "親（O か E）"), len("text", "文", 1, SE_MAX.ftext), len("reason", "理由", 0, SE_MAX.reason));
      if (msg) return bad(msg);
      const text = seTrim(v.text);
      if (seCountSentences(text) > 3) return bad("文は 1〜3 文にしてください");
      const cites = seList(v.cites);
      if (cites.length > 10) return bad("出典は 10 個までです");
      for (const c of cites) if (!seCiteOk(c)) return bad(`出典「${c}」の形が正しくありません（#37 §3.2）`);
      if (seNeedsCite(text, cites)) return bad("数字を含む F には出典が要ります");
      const op = { op: name, parent: seTrim(v.parent), text };
      if (cites.length) op.cites = cites;
      const paper = seTrim(v.paper);
      if (paper) {
        if (!/^[1-9][0-9]{0,8}$/.test(paper)) return bad("論文番号は数字で入れてください");
        op.paper = Number(paper);
      }
      if (seTrim(v.report)) op.report = seTrim(v.report);
      return good(withReason(op));
    }
    case "add_e": {
      const msg = first(id("parent", "OE", "親（O か E）"), len("text", "文", 1, SE_MAX.etext), len("body", "本文", 0, SE_MAX.body), len("mlflow", "MLflow の名前", 0, SE_MAX.mlflow), len("reason", "理由", 0, SE_MAX.reason));
      if (msg) return bad(msg);
      const op = { op: name, parent: seTrim(v.parent), text: seTrim(v.text) };
      if (seTrim(v.body)) op.body = seTrim(v.body);
      if (seTrim(v.mlflow)) op.mlflow = seTrim(v.mlflow);
      return good(withReason(op));
    }
    case "record_logic": {
      const value = seTrim(v.value);
      const msg = first(id("q", "Q", "Q"), len("note", "指摘の文", 0, SE_MAX.note), len("reason", "理由", 0, SE_MAX.reason));
      if (msg) return bad(msg);
      if (value !== "ok" && value !== "contradiction") return bad("値は OK か CONTRADICTION です");
      const findings = seList(v.findings);
      for (const f of findings) if (!seRefOk(f, "F")) return bad(`F の形が正しくありません: ${f}`);
      if (value === "contradiction") {
        if (!seTrim(v.note)) return bad("CONTRADICTION には指摘の文が要ります");
        if (findings.length < 2) return bad("CONTRADICTION には F を 2 つ以上選んでください");
      }
      const op = { op: name, q: seTrim(v.q), value };
      if (seTrim(v.note)) op.note = seTrim(v.note);
      if (findings.length) op.findings = findings;
      return good(withReason(op));
    }
    case "add_seed": {
      const msg = first(len("title", "題", 1, SE_MAX.title), len("sentence", "文", 1, SE_MAX.sentence), len("reason", "理由", 0, SE_MAX.reason));
      if (msg) return bad(msg);
      const path = seList(v.path);
      if (!path.length) return bad("道の O を 1 つ以上選んでください");
      for (const o of path) if (!seRefOk(o, "O")) return bad(`O の形が正しくありません: ${o}`);
      return good(withReason({ op: name, title: seTrim(v.title), sentence: seTrim(v.sentence), path }));
    }
    case "copy": {
      const msg = first(id("node", "O", "写す O"), id("to", "Q", "写す先の Q"), len("reason", "理由", 0, SE_MAX.reason));
      return msg ? bad(msg) : good(withReason({ op: name, node: seTrim(v.node), to: seTrim(v.to) }));
    }
    case "remove": {
      const msg = first(id("node", "QOFE", "消す Node"), len("reason", "理由", 1, SE_MAX.reason));
      return msg ? bad(msg) : good({ op: name, node: seTrim(v.node), reason: seTrim(v.reason) });
    }
    case "star":
    case "unstar": {
      const msg = first(id("o", "O", "O"), len("reason", "理由", 0, SE_MAX.reason));
      return msg ? bad(msg) : good(withReason({ op: name, o: seTrim(v.o) }));
    }
    case "set_kind": {
      const msg = first(id("o", "O", "O"), len("reason", "理由", 0, SE_MAX.reason));
      if (msg) return bad(msg);
      const kindV = seTrim(v.kind);
      if (!SE_KINDS.some((x) => x[0] === kindV)) return bad("種類は Answer か Issue です");
      return good(withReason({ op: name, o: seTrim(v.o), kind: kindV }));
    }
    case "edit": {
      const msg = first(id("node", "QOFE", "直す Node"), len("text", "文", 0, SE_MAX.ftext), len("body", "本文", 0, SE_MAX.body), len("reason", "理由", 0, SE_MAX.reason));
      if (msg) return bad(msg);
      if (!seTrim(v.text) && !seTrim(v.body)) return bad("文か本文のどちらかを入力してください");
      const op = { op: name, node: seTrim(v.node) };
      if (seTrim(v.text)) op.text = seTrim(v.text);
      if (seTrim(v.body)) op.body = seTrim(v.body);
      return good(withReason(op));
    }
    case "set_target": {
      const msg = id("q", "Q", "Q");
      if (msg) return bad(msg);
      const t = seTrim(v.target);
      if (t === "") return good({ op: name, q: seTrim(v.q), target: null });
      if (!/^[0-9]{1,2}$/.test(t)) return bad("目安は 0〜99 の整数か空です");
      return good({ op: name, q: seTrim(v.q), target: Number(t) });
    }
    default:
      return bad("Unknown operation");
  }
}

/**
 * カードの初めの値（C-CARD の preset から）。無い欄は空。
 * runner が user でブロックが空なら、ブロックの既定は sci-read。
 */
function seCardValues(preset, nodeId) {
  const p = preset || {};
  const runner = SE_RUNNERS.some((r) => r[0] === p.runner) ? p.runner : "pi";
  let block = SE_BLOCKS.some((b) => b[0] === p.block) ? p.block : "";
  if (runner === "user" && !block) block = "sci-read";
  return {
    node: seTrim(nodeId) || seTrim(p.node),
    block,
    purpose: seTrim(p.purpose),
    expect: seTrim(p.expect),
    runner,
    scope: SE_SCOPES.some((s) => s[0] === p.scope) ? p.scope : "",
    mode: SE_MODES.some((m) => m[0] === p.mode) ? p.mode : "purpose_first",
    assume: seList(p.assume),
    papers: (Array.isArray(p.papers) ? p.papers : []).map(Number).filter((n) => Number.isInteger(n) && n > 0),
    depth: SE_DEPTHS.some((d) => d[0] === p.depth) ? p.depth : "full",
  };
}

/**
 * カードのフォームの値を確かめて、送る JSON にする。
 *   → { ok: true, card: {...}, error: null } または { ok: false, card: null, error: "理由" }
 * papers は番号の配列か、番号を書いた文字の配列（打ち込み）。
 */
function seBuildCard(values) {
  const v = values || {};
  const bad = (error) => ({ ok: false, card: null, error });
  const node = seTrim(v.node);
  if (!seRefOk(node)) return bad("Node を選んでください");
  if (!SE_BLOCKS.some((b) => b[0] === v.block)) return bad("ブロックを選んでください");
  const purpose = seTrim(v.purpose);
  if (!purpose) return bad("目的を入力してください（空の目的では作りません）");
  if (purpose.length > SE_MAX.purpose) return bad("目的は 300 字までです");
  const expect = seTrim(v.expect);
  if (expect.length > SE_MAX.expect) return bad("期待する成果は 300 字までです");
  const runner = SE_RUNNERS.some((r) => r[0] === v.runner) ? v.runner : "pi";
  const assume = seList(v.assume);
  if (assume.length > 5) return bad("仮置きは 5 個までです");
  for (const a of assume) if (!SE_ASSUME_RE.test(a)) return bad(`仮置き「${a}」の形が正しくありません（Q9=値）`);
  const papers = [];
  for (const raw of Array.isArray(v.papers) ? v.papers : []) {
    const s = seTrim(raw).replace(/^#/, "");
    if (!s) continue;
    if (!/^[1-9][0-9]{0,8}$/.test(s)) return bad(`論文の番号「${seTrim(raw)}」が正しくありません`);
    if (!papers.includes(Number(s))) papers.push(Number(s));
  }
  if (papers.length > 20) return bad("論文は 20 本までです");
  const card = { node, block: v.block, purpose, runner, papers };
  if (runner === "user") {
    if (node[0] === "F") return bad("読む札の対象は Q か O にしてください");
    if (papers.length !== 1) return bad("読む札の論文はちょうど 1 本にしてください");
    card.depth = SE_DEPTHS.some((d) => d[0] === v.depth) ? v.depth : "full";
  }
  if (expect) card.expect = expect;
  if (seTrim(v.scope)) card.scope = seTrim(v.scope);
  if (v.block === "sci-read" && seTrim(v.mode)) card.mode = seTrim(v.mode);
  if (assume.length) card.assume = assume;
  return { ok: true, card, error: null };
}

/** 提案の Node（まとめの鍵）。payload の parent・q・node・o の順で最初にある ID。無ければ空文字。 */
function seProposalNodeKey(p) {
  if (!p) return "";
  const payload = p.payload || {};
  for (const k of ["parent", "q", "node", "o"]) {
    if (seRefOk(payload[k])) return seTrim(payload[k]);
  }
  return seRefOk(p.node) ? seTrim(p.node) : "";
}

/** 提案の run の番号。無ければ空文字。 */
function seProposalRunKey(p) {
  const num = Number(p && p.run_id);
  return Number.isInteger(num) && num > 0 ? String(num) : "";
}

/**
 * 仕分け箱のまとめ方（"node" か "run"）。
 *   → [{ key, label, items: [...] }]。出会った順に並べる。
 */
function seGroupProposals(list, by) {
  const items = (Array.isArray(list) ? list : []).filter((p) => p && typeof p === "object");
  const map = new Map();
  for (const p of items) {
    const key = by === "run" ? seProposalRunKey(p) : seProposalNodeKey(p);
    if (!map.has(key)) {
      const title = seTrim(p.node_title);
      const label = by === "run"
        ? (key ? `run ${key}` : "run 不明")
        : (key ? `${key}${title ? ` ${title}` : ""}` : "Node なし");
      map.set(key, { key, label, items: [] });
    }
    map.get(key).items.push(p);
  }
  return [...map.values()];
}

/** 提案の中身の要約（payload の text・title・node・parent から 90 字）。 */
function seProposalSummary(p) {
  const payload = (p && p.payload) || {};
  const parts = [];
  for (const k of ["text", "title", "node", "parent"]) {
    const t = seTrim(payload[k]);
    if (t) parts.push(t);
  }
  const text = parts.join("　") || "（中身がありません）";
  return text.length > 90 ? `${text.slice(0, 90)}…` : text;
}

/** payload の文字の項目。直すときの小窓に使う。 */
function seProposalEditKeys(p) {
  const payload = (p && p.payload) || {};
  return ["text", "body", "title", "sentence"].filter((k) => typeof payload[k] === "string" && payload[k] !== "");
}

/** 仕分けの一覧を取り出す（`{"proposals": […]}` でも、並びの配列でも、`pending` でも）。 */
function seProposalList(res) {
  if (Array.isArray(res)) return res;
  if (res && Array.isArray(res.proposals)) return res.proposals;
  if (res && Array.isArray(res.pending)) return res.pending;
  return [];
}

/** カードを作った応答から run の番号を読む。 */
function seCardRunId(res) {
  if (!res || typeof res !== "object") return null;
  if (res.run_id != null) return res.run_id;
  if (res.id != null) return res.id;
  if (res.run && res.run.id != null) return res.run.id;
  if (res.run && res.run.run_id != null) return res.run.run_id;
  return null;
}

// ============================================================================
// 小さい DOM を作る手（ui.js の saiEl と同じ形）
// ============================================================================

function seEl(tag, cls, text) {
  const e = document.createElement(tag);
  if (cls) e.className = cls;
  if (text != null) e.textContent = text;
  return e;
}

function seBtn(label, onClick, cls) {
  const b = seEl("button", `sai-btn ${cls || ""}`.trim(), label);
  b.type = "button";
  if (typeof onClick === "function") b.addEventListener("click", onClick);
  else b.disabled = true;
  return b;
}

function seChip(text, tone) {
  return seEl("span", `sai-chip ${tone || "muted"}`, text);
}

function seLine(cls) {
  return seEl("div", `sci-edit-line ${cls || ""}`.trim());
}

function seNote(text) {
  return seEl("p", "sci-edit-note", text || "");
}

/** 節点の題を status から引く。 */
function seTitleOf(status, id) {
  const list = status && Array.isArray(status.nodes) ? status.nodes : [];
  const n = list.find((x) => x && x.id === id);
  return n ? n.title || "" : "";
}

// ============================================================================
// 入力欄の部品（1 種類ごとに 1 つの関数）
// ============================================================================

/** 1 行の入力。 */
function seFieldText(key, label, init, hint, placeholder) {
  const el = seLine("sci-edit-field");
  el.appendChild(seEl("label", "sci-edit-label", label));
  const input = document.createElement("input");
  input.type = "text";
  input.className = "sai-input sci-edit-input";
  input.value = init == null ? "" : String(init);
  if (placeholder) input.placeholder = placeholder;
  el.appendChild(input);
  if (hint) el.appendChild(seNote(hint));
  return { key, el, read: () => input.value, set: (x) => { input.value = x; } };
}

/** 大きな入力。 */
function seFieldArea(key, label, init, rows, hint, placeholder) {
  const el = seLine("sci-edit-field");
  el.appendChild(seEl("label", "sci-edit-label", label));
  const area = document.createElement("textarea");
  area.className = "sai-input sci-edit-area";
  area.rows = rows || 4;
  area.value = init == null ? "" : String(init);
  if (placeholder) area.placeholder = placeholder;
  el.appendChild(area);
  if (hint) el.appendChild(seNote(hint));
  return { key, el, read: () => area.value, set: (x) => { area.value = x; } };
}

/** 1 つ選ぶ（allowEmpty で「なし」を付ける）。 */
function seFieldSelect(key, label, options, init, hint, allowEmpty) {
  const el = seLine("sci-edit-field");
  el.appendChild(seEl("label", "sci-edit-label", label));
  const sel = document.createElement("select");
  sel.className = "sai-input sci-edit-select";
  if (allowEmpty) {
    const none = document.createElement("option");
    none.value = "";
    none.textContent = "なし";
    sel.appendChild(none);
  }
  const list = Array.isArray(options) ? options : [];
  for (const [value, text] of list) {
    const o = document.createElement("option");
    o.value = value;
    o.textContent = text;
    sel.appendChild(o);
  }
  const want = seTrim(init);
  sel.value = want;
  if (sel.value !== want) sel.value = allowEmpty || !list.length ? "" : String(list[0][0]);
  el.appendChild(sel);
  if (hint) el.appendChild(seNote(hint));
  return { key, el, read: () => sel.value, set: (x) => { sel.value = x; } };
}

/** 1 つだけ選ぶ（押して切り替える）。 */
function seFieldPick(key, label, options, init) {
  const el = seLine("sci-edit-field");
  el.appendChild(seEl("label", "sci-edit-label", label));
  const box = seEl("div", "sci-edit-picks");
  const buttons = [];
  for (const [value, text] of options || []) {
    const b = seEl("button", `sci-edit-pick${seTrim(init) === value ? " active" : ""}`, text);
    b.type = "button";
    b.dataset.value = value;
    b.addEventListener("click", () => {
      for (const x of buttons) x.el.classList.toggle("active", x.value === value);
      // 1 つ選んだことが分かるように change を起こす（ほかの欄の表示を切り替えるため）
      b.dispatchEvent(new Event("change", { bubbles: true }));
    });
    buttons.push({ value, el: b });
    box.appendChild(b);
  }
  el.appendChild(box);
  const read = () => (buttons.find((x) => x.el.classList.contains("active")) || {}).value || "";
  return { key, el, read, refresh: () => el.classList.toggle("sci-edit-picked", !!read()) };
}

/** 複数選ぶ。 */
function seFieldChecks(key, label, options, init, hint) {
  const el = seLine("sci-edit-field");
  el.appendChild(seEl("label", "sci-edit-label", label));
  const box = seEl("div", "sci-edit-checks");
  const chosen = new Set((Array.isArray(init) ? init : []).map(String));
  const list = Array.isArray(options) ? options : [];
  for (const [value, text] of list) {
    const row = seEl("label", "sci-edit-check");
    const input = document.createElement("input");
    input.type = "checkbox";
    input.checked = chosen.has(String(value));
    row.appendChild(input);
    row.appendChild(seEl("span", null, text));
    box.appendChild(row);
  }
  el.appendChild(box);
  if (hint) el.appendChild(seNote(hint));
  return {
    key,
    el,
    read: () => [...box.querySelectorAll("input")].flatMap((c, i) => (c.checked ? [String(list[i][0])] : [])),
  };
}

/** 文の先頭 n 字（40 字・30 字の表示用。… は付けない）。 */
function seChainHead(text, n) {
  return Array.from(seTrim(text)).slice(0, n).join("");
}

/** Q の選択肢の表示（`Q3 <文の先頭 40 字>`）。 */
function seChainQLabel(n) {
  return `${String(n.id)} ${seChainHead(n.text || n.title || "", 40)}`.trim();
}

/** O の選択肢の表示（`${Q の ID} › ${O の ID} ${文の先頭 30 字}（Answer|Issue）`。Q の ID は O の親）。 */
function seChainOLabel(list, o) {
  const byId = new Map((Array.isArray(list) ? list : []).filter((n) => n && n.id != null).map((n) => [String(n.id), n]));
  const parent = byId.get(String(o.id));
  const q = parent && parent.parent != null ? String(parent.parent) : "";
  const kind = /issue/i.test(o.answer_kind || "") ? "Issue" : "Answer";
  return `${q ? `${q} › ` : ""}${String(o.id)} ${seChainHead(o.text || "", 30)}（${kind}）`.trim();
}

/** O が Issue か。 */
function seChainIsIssue(list, oId) {
  const n = (Array.isArray(list) ? list : []).find((x) => x && String(x.id) === String(oId));
  return !!n && /issue/i.test(n.answer_kind || "");
}

/** 鎖の段の計画（{rows: [{options: [[value, text]]}]}）。
 * rows[0] は開始の Q、rows[1] はその直の子の O、rows[2] からは直前の O の下の O。
 * 直前の段を選んでいない・候補が無いときは、その段を出さない。 */
function seChainPlan(list, picks) {
  const nodes = Array.isArray(list) ? list : [];
  const sel = Array.isArray(picks) ? picks : [];
  const qCands = nodes.filter((n) => n && seRefOk(n.id, "Q") && seSproutStartOs(nodes, n.id).length);
  const rows = [{ options: qCands.map((n) => [String(n.id), seChainQLabel(n)]) }];
  const q = sel[0] ? String(sel[0]) : "";
  if (!q) return { rows };
  rows.push({ options: seSproutStartOs(nodes, q).map((o) => [o.id, seChainOLabel(nodes, o)]) });
  let prev = sel[1] ? String(sel[1]) : "";
  while (prev && rows.length < 52) {
    const next = seSproutNext(nodes, prev);
    if (!next.length) break;
    rows.push({ options: next.map((o) => [o.id, seChainOLabel(nodes, o)]) });
    const nxt = sel[rows.length - 1];
    prev = nxt ? String(nxt) : "";
  }
  return { rows };
}

/** 選んだ道（picks[1] からの O の ID。空で止める）。 */
function seChainPath(list, picks) {
  const sel = Array.isArray(picks) ? picks : [];
  const out = [];
  for (let i = 1; i < sel.length; i += 1) {
    const v = sel[i] ? String(sel[i]) : "";
    if (!v) break;
    out.push(v);
  }
  return out;
}

/** Sprout の鎖を 1 本選ぶ欄（{key, el, read}。read() は選んだ O の ID の配列）。
 * 1 段目は開始の Q、2 段目はその直の子の O、3 段目からは直前の O の下の O（seSproutNext）。
 * どの段を変えても、その下の段を作り直す。node では使わない純粋な部分は seChainPlan が受け持つ。 */
function seFieldChain(key, label, nodes, init) {
  const el = seLine("sci-edit-field sci-edit-chain");
  el.appendChild(seEl("label", "sci-edit-label", label));
  const box = seEl("div", "sci-edit-chain-box");
  const pathLine = seEl("p", "sci-edit-note", "");
  const warn = seEl("p", "sci-edit-error", "Issue の O を 1 つ以上含めてください");
  el.appendChild(box);
  el.appendChild(pathLine);
  el.appendChild(warn);
  const list = Array.isArray(nodes) ? nodes : [];

  // 開始の Q の候補（直の子に O を持つ Q）。表示は `Q3 <文の先頭 40 字>`。
  const qCands = list.filter((n) => n && seRefOk(n.id, "Q") && seSproutStartOs(list, n.id).length);
  // 初めの値（init は {q, o} か {nodeId} か Node の ID の文字。nodeId が Q ならその Q、O なら親の Q とその O）。
  const rawInit = typeof init === "string" ? init : (init && (init.nodeId || init.q) != null ? String(init.nodeId || init.q) : "");
  const rawO = init && typeof init === "object" && init.o != null ? String(init.o) : "";
  let startQ = "";
  if (seRefOk(rawInit, "Q") && qCands.some((n) => String(n.id) === rawInit)) startQ = rawInit;
  else if (seRefOk(rawInit, "O")) {
    const parent = list.find((n) => n && String(n.id) === rawInit);
    const q = parent && parent.parent != null ? String(parent.parent) : "";
    if (q && qCands.some((n) => String(n.id) === q)) startQ = q;
  } else if (seRefOk(rawO, "O")) {
    const parent = list.find((n) => n && String(n.id) === rawO);
    const q = parent && parent.parent != null ? String(parent.parent) : "";
    if (q && qCands.some((n) => String(n.id) === q)) startQ = q;
  }
  if (!startQ && qCands.length) startQ = String(qCands[0].id);
  const startO = seRefOk(rawInit, "O") ? rawInit : (seRefOk(rawO, "O") ? rawO : "");

  // 段ごとの選択: [qId, oId, oId, ...]（1 段目は Q、2 段目から O）。
  const picks = [startQ, startO];
  const stageLabel = (i) => (i === 0 ? "開始の Q" : i === 1 ? "開始の Q の O" : `段 ${i + 1} の O`);

  const refresh = () => {
    const plan = seChainPlan(list, picks);
    // 下の段が消えたときは、その下の選択も空にする。
    for (let i = 0; i < plan.rows.length; i += 1) {
      const row = plan.rows[i];
      if (row.options.length && picks[i] && !row.options.some((x) => x[0] === picks[i])) picks[i] = "";
    }
    while (picks.length > plan.rows.length) picks.pop();
    box.innerHTML = "";
    plan.rows.forEach((row, i) => {
      const line = seLine("sci-edit-chain-row");
      line.style.paddingLeft = `${i * 8}px`;
      const sel = document.createElement("select");
      sel.className = "sai-input sci-edit-select";
      sel.setAttribute("aria-label", stageLabel(i));
      // 1 段目（開始の Q）は必ずどれかを選ぶので、空の選択肢は置かない（候補が 1 つも無いときだけ置く）。
      if (i !== 0 || !row.options.length) {
        const none = document.createElement("option");
        none.value = "";
        none.textContent = i === 1 ? "選ぶ" : "ここで止める";
        sel.appendChild(none);
      }
      for (const [value, text] of row.options) {
        const o = document.createElement("option");
        o.value = value;
        o.textContent = text;
        sel.appendChild(o);
      }
      sel.value = picks[i] || "";
      if (sel.value !== (picks[i] || "")) picks[i] = "";
      sel.addEventListener("change", () => {
        picks[i] = sel.value;
        // その下の段を作り直す。
        for (let j = i + 1; j < picks.length; j += 1) picks[j] = "";
        refresh();
      });
      line.appendChild(sel);
      box.appendChild(line);
    });
    const path = seChainPath(list, picks);
    pathLine.textContent = path.length > 1 ? path.join(" → ") : path.length === 1 ? path[0] : "";
    warn.hidden = path.some((o) => seChainIsIssue(list, o));
  };
  refresh();
  return { key, el, read: () => seChainPath(list, picks) };
}

/** 1 行ずつ何個も足す欄（出典・仮置き）。 */
function seFieldLines(key, label, init, max, hint, placeholder) {
  const el = seLine("sci-edit-field");
  el.appendChild(seEl("label", "sci-edit-label", label));
  const box = seEl("div", "sci-edit-lines");
  const inputs = [];
  const push = (value) => {
    const input = document.createElement("input");
    input.type = "text";
    input.className = "sai-input sci-edit-input";
    input.value = value == null ? "" : String(value);
    if (placeholder) input.placeholder = placeholder;
    box.appendChild(input);
    inputs.push(input);
  };
  const start = (Array.isArray(init) ? init : []).filter((x) => seTrim(x) !== "");
  if (start.length) for (const s of start) push(s);
  else push("");
  const add = seBtn("Add a line", () => {
    if (inputs.length >= max) return;
    push("");
    add.disabled = inputs.length >= max;
  }, "ghost small");
  el.appendChild(box);
  el.appendChild(add);
  if (hint) el.appendChild(seNote(hint));
  return { key, el, read: () => inputs.map((i) => i.value) };
}

/** 入力欄をまとめて小窓の入れ物にする（説明の文のように DOM のままの物も置ける）。 */
function seForm(fields) {
  const el = seEl("div", "sci-edit-form");
  const made = [];
  for (const f of Array.isArray(fields) ? fields : []) {
    if (!f) continue;
    if (f.el) {
      el.appendChild(f.el);
      made.push(f);
    } else if (f.nodeType === 1) {
      el.appendChild(f); // 説明の文のように、そのまま置く要素
    }
  }
  const error = seEl("p", "sci-edit-error");
  error.hidden = true;
  error.setAttribute("role", "alert");
  el.appendChild(error);
  const actions = seEl("div", "sci-edit-actions");
  el.appendChild(actions);
  const form = {
    el,
    actions,
    read() {
      const out = {};
      for (const f of made) {
        out[f.key] = f.read();
        for (const s of f.subs || []) out[s.key] = s.read();
      }
      return out;
    },
    say(text) {
      error.textContent = text || "";
      error.hidden = !text;
    },
    refresh() {
      const cur = form.read();
      for (const f of made) {
        if (typeof f.show === "function") f.el.hidden = !f.show(cur);
        if (typeof f.refresh === "function") f.refresh();
      }
    },
  };
  form.refresh();
  el.addEventListener("change", () => form.refresh());
  return form;
}

// ============================================================================
// 通信
// ============================================================================

/** 人の操作を送って描き直す。送れなかったときの理由は小窓の中に出す。 */
async function seApply(op, form) {
  try {
    const res = await paperApi(stApi("/apply"), { method: "POST", body: op });
    if (form && form.host && typeof form.host.close === "function") form.host.close();
    saiToast(SE_DONE[op.op] || "反映しました", "ok");
    if (typeof stReload === "function") await stReload();
    return res;
  } catch (e) {
    const why = (e && e.message) || "反映できませんでした";
    if (form && typeof form.say === "function") form.say(why);
    else saiToast(why, "error");
  }
  return null;
}

/** 仕分ける（1 件）。 */
function seTriageOne(pid, body) {
  return paperApi(stApi(`/proposals/${encodeURIComponent(pid)}`), { method: "POST", body });
}

// ============================================================================
// Status から引く手（status.nodes は [{id, title, parent, star, ...}]）
// ============================================================================

function seStatusNodes() {
  const st = typeof stStatus === "function" ? stStatus() : null;
  return st && Array.isArray(st.nodes) ? st.nodes : [];
}

const seNodeOption = (n) => [n.id, `${n.id} ${n.title || n.text || ""}`.trim()];

/** 道の上（nodeId から根まで）の ID の集まり。 */
function seAncestors(nodes, nodeId) {
  const byId = new Map(nodes.map((n) => [n.id, n]));
  const out = new Set();
  let cur = byId.get(nodeId);
  while (cur && !out.has(cur.id)) {
    out.add(cur.id);
    cur = byId.get(cur.parent);
  }
  return out;
}

/** 子の Q を持たない Q（O の下の Q も子とみなす）。 */
function seLeafQs(nodes) {
  const qs = nodes.filter((n) => n && seRefOk(n.id, "Q"));
  const parentOf = new Map(nodes.map((n) => [n.id, n.parent]));
  return qs.filter((q) => !qs.some((c) => c.parent === q.id || parentOf.get(c.parent) === q.id));
}

/** O から上へ Q をたどって最初に会う O の ID（無ければ null）。巡回しても止まらないように見る ID を覚える。 */
function seNearestO(nodes, oId) {
  const list = Array.isArray(nodes) ? nodes : [];
  const byId = new Map(list.filter((n) => n && n.id != null).map((n) => [String(n.id), n]));
  const seen = new Set([String(oId)]);
  let cur = byId.get(String(oId));
  cur = cur ? byId.get(cur.parent == null ? "" : String(cur.parent)) : null;
  while (cur && !seen.has(String(cur.id))) {
    seen.add(String(cur.id));
    if (seRefOk(cur.id, "O")) return String(cur.id);
    cur = byId.get(cur.parent == null ? "" : String(cur.parent));
  }
  return null;
}

/** Q の直の子の O（[{id, text, answer_kind}]、並びは nodes の順）。 */
function seSproutStartOs(nodes, qId) {
  const want = qId == null ? "" : String(qId);
  return (Array.isArray(nodes) ? nodes : [])
    .filter((n) => n && seRefOk(n.id, "O") && String(n.parent) === want)
    .map((n) => ({ id: String(n.id), text: n.text || n.title || "", answer_kind: n.answer_kind || "" }));
}

/** 鎖の次に選べる O（seNearestO(nodes, o) === prevO の O。並びは nodes の順）。 */
function seSproutNext(nodes, prevO) {
  const want = prevO == null ? "" : String(prevO);
  return (Array.isArray(nodes) ? nodes : [])
    .filter((n) => n && seRefOk(n.id, "O") && seNearestO(nodes, n.id) === want)
    .map((n) => ({ id: String(n.id), text: n.text || n.title || "", answer_kind: n.answer_kind || "" }));
}

/** detail.subtree の F（[{id, text, paper}]）。 */
function seSubtreeFs(detail) {
  const list = detail && Array.isArray(detail.subtree) ? detail.subtree : [];
  return list.filter((n) => n && seRefOk(n.id, "F"));
}

// ============================================================================
// 操作の入力欄（seOpenAction）
// ============================================================================

/** op ごとの入力欄（{title, values, fields}）。★・Answer/Issue・目安は seQuick で送るので無い。 */
function seFieldsFor(kind, nodeId, detail) {
  const d = detail || {};
  const node = d.node || {};
  const id = seTrim(nodeId) || seTrim(node.id);
  const nodes = seStatusNodes();
  const reason = () => seFieldArea("reason", "理由", "", 2, "任意・500 字まで");

  switch (kind) {
    case "add_q": {
      const cands = (d.guide && Array.isArray(d.guide.child_candidates) ? d.guide.child_candidates : [])
        .map((c) => seTrim(c && typeof c === "object" ? c.text : c)).filter(Boolean);
      const text = seFieldArea("text", "文", "", 3, "必須・300 字まで");
      const fields = [];
      if (cands.length) {
        const guide = seFieldSelect("_guide", "ガイドの候補", cands.map((c) => [c, c]), "", "選ぶと文に入ります", true);
        let last = "";
        guide.refresh = () => {
          const v = guide.read();
          if (v && v !== last) text.set(v);
          last = v;
        };
        fields.push(guide);
      }
      fields.push(text, reason());
      return { title: `Q を足す（${id}）`, values: { parent: id }, fields };
    }
    case "add_o":
      return {
        title: `O を足す（${id}）`,
        values: { q: id },
        fields: [
          seFieldText("text", "文", "", "必須・120 字まで"),
          seFieldArea("body", "本文", "", 4, "任意・2000 字まで"),
          seFieldSelect("kind", "種類", SE_KINDS, "answer", null, false),
          reason(),
        ],
      };
    case "add_f":
      return {
        title: `F を足す（${id}）`,
        values: { parent: id },
        fields: [
          seFieldArea("text", "文", "", 3, "必須・1〜3 文・1200 字まで"),
          seFieldLines("cites", "出典", [], 10, "1 行 1 つ。`#37 §3.2` の形（数字を書くときは必須）", "#37 §3.2"),
          seFieldText("paper", "論文番号", "", "任意（37 のような数字）"),
          reason(),
        ],
      };
    case "add_e":
      return {
        title: `E を足す（${id}）`,
        values: { parent: id },
        fields: [
          seFieldText("text", "文", "", "必須・120 字まで"),
          seFieldArea("body", "本文", "", 4, "任意・2000 字まで"),
          seFieldText("mlflow", "MLflow の名前", "", "任意・200 字まで"),
          reason(),
        ],
      };
    case "record_logic":
      return {
        title: `Logicality を記録（${id}）`,
        values: { q: id },
        fields: [
          seFieldPick("value", "値", [["ok", "OK"], ["contradiction", "CONTRADICTION"]], "ok"),
          seFieldArea("note", "指摘の文", "", 3, "CONTRADICTION のときは必須・1000 字まで"),
          seFieldChecks("findings", "F", seSubtreeFs(d).map((f) => [f.id, `${f.id} ${(f.text || "").slice(0, 40)}`]), [], "CONTRADICTION のときは 2 つ以上"),
        ],
      };
    case "add_seed": {
      const st = typeof stStatus === "function" ? stStatus() : null;
      // nodeId が Q か O なら、鎖の初めの値に使う（C5）。n.star は使わない。
      return {
        title: "Sprout を作る",
        values: {},
        fields: [
          seFieldText("title", "題", "", "必須・120 字まで"),
          seFieldArea("sentence", "文", (st && st.seed_draft && st.seed_draft.text) || "", 4, "必須・600 字まで"),
          seFieldChain("path", "道", nodes, { nodeId: id }),
          reason(),
        ],
      };
    }
    case "copy":
      return {
        title: `O を写す（${id}）`,
        values: { node: id },
        fields: [seFieldSelect("to", "写す先の Q", seLeafQs(nodes).map(seNodeOption), "", "子の Q を持たない Q", false), reason()],
      };
    case "remove":
      return {
        title: `消す（${id}）`,
        values: { node: id },
        fields: [seFieldArea("reason", "理由", "", 3, "必須・500 字まで")],
      };
    case "edit":
      return {
        title: `文を直す（${id}）`,
        values: { node: id },
        fields: [
          seFieldArea("text", "文", node.text || "", 3, "文か本文のどちらかは必須"),
          seFieldArea("body", "本文", node.body || "", 5, "2000 字まで"),
          reason(),
        ],
      };
    default:
      return { title: seOpLabel(kind), values: {}, fields: [] };
  }
}

/** 入力欄を出さずに送る（★・Answer/Issue・目安）。set_kind は確認だけ出す。 */
async function seQuick(op, payload) {
  const built = seBuildOp(op, Object.assign({}, payload));
  if (!built.ok) { saiToast(built.error, "error"); return null; }
  if (op === "set_kind") {
    const label = (SE_KINDS.find((k) => k[0] === built.op.kind) || [])[1] || built.op.kind;
    const ok = await saiConfirm({ title: "Answer / Issue を変えますか", body: `${built.op.o} を ${label} にします。`, ok: "変える" });
    if (!ok) return null;
  }
  return seApply(built.op, null);
}

/**
 * op の小窓を出す。SE_QUICK の op は seQuick に回す。remove は理由と確認を必ず出す。
 * 送る前に画面でも必須の項目と字数と出典を確かめる。通らなければ送らない。
 */
function seOpenAction(op, nodeId, detail) {
  if (typeof document === "undefined" || typeof saiSheet !== "function") return;
  const name = seTrim(op);
  if (SE_QUICK.includes(name)) {
    const id = seTrim(nodeId);
    void seQuick(name, { o: id, q: id });
    return;
  }
  const spec = seFieldsFor(name, nodeId, detail);
  const form = seForm(spec.fields);
  const body = seEl("div", "sci-edit-wrap");
  body.appendChild(form.el);
  const host = saiSheet({ title: spec.title, body });
  form.host = host;
  form.actions.appendChild(seBtn("やめる", () => host.close(), "ghost"));

  const send = async () => {
    const v = Object.assign({}, spec.values, form.read());
    const built = seBuildOp(name, v);
    if (!built.ok) { form.say(built.error); return; }
    if (name === "remove") {
      const ok = await saiConfirm({
        title: "消しますか",
        body: `${built.op.node} とその下の Node をすべて消します。元に戻せません。`,
        ok: "消す",
        danger: true,
      });
      if (!ok) return;
    }
    await seApply(built.op, form);
  };
  form.actions.appendChild(seBtn(name === "remove" ? "消す" : "送る", send, name === "remove" ? "danger" : "primary"));
}

// ============================================================================
// カードのフォーム（seOpenCard）
// ============================================================================

/**
 * カードを作る小窓。preset（C-CARD）があれば各欄の初めの値にする。
 * runner に user を選ぶと読む札になる（論文は 1 本・ブロックの既定は sci-read）。
 */
async function seOpenCard(nodeId, preset) {
  if (typeof document === "undefined" || typeof saiSheet !== "function") return;
  const init = seCardValues(preset, nodeId);
  let detail = null;
  if (seTrim(nodeId)) {
    try {
      detail = await paperApi(stApi(`/nodes/${encodeURIComponent(seTrim(nodeId))}`));
    } catch (e) {
      saiToast((e && e.message) || "Node を読み込めませんでした", "error");
      return;
    }
  }
  const candidates = seSubtreeFs(detail).filter((f) => f.paper != null).map((f) => String(f.paper));
  const uniq = [...new Set(candidates)];
  const known = new Set(uniq);
  const typed = init.papers.filter((n) => !known.has(String(n))).map(String);

  const fields = [];
  if (!seTrim(nodeId)) {
    const picks = seStatusNodes().filter((n) => n && seRefOk(n.id)).map(seNodeOption);
    if (picks.length) fields.push(seFieldSelect("node", "Node", picks, init.node, "カードを向ける Node", false));
    else fields.push(seNote("Node がまだありません。まず Q を足してください。"));
  }
  const block = seFieldSelect("block", "ブロック", SE_BLOCKS, init.block, "必須", false);
  const purpose = seFieldArea("purpose", "目的", init.purpose, 3, "必須・300 字まで。何を達成するか");
  const mode = seFieldSelect("mode", "型", SE_MODES, init.mode, "1 本を読むときだけ使う", false);
  mode.show = (v) => v.block === "sci-read";
  const depth = seFieldSelect("depth", "深さ", SE_DEPTHS, init.depth, "読む札のときだけ使う", false);
  depth.show = (v) => v.runner === "user";
  const runner = seFieldSelect("runner", "実行するもの", SE_RUNNERS, init.runner, "user は自分で読む札", false);
  fields.push(
    block, purpose,
    seFieldArea("expect", "期待する成果", init.expect, 2, "300 字まで（任意）"),
    runner,
    seFieldSelect("scope", "範囲", SE_SCOPES, init.scope, "空のときはブロックの既定", true),
    mode,
    seFieldLines("assume", "仮置き", init.assume, 5, "1 行 1 つ。`Q9=SST-2 を使う` の形（5 個まで）", "Q9=値"),
  );
  if (uniq.length) {
    fields.push(seFieldChecks("papers", "論文", uniq.map((p) => [p, `#${p}`]), init.papers.map(String), "この Node の下の F の論文から選ぶ"));
  }
  fields.push(seFieldLines("papers_typed", "論文の番号", typed, 20, "番号を打って足せます（20 本まで。読む札は 1 本）", "37"), depth);

  const form = seForm(fields);
  // runner を user にしたとき、ブロックが空なら sci-read にする
  let lastRunner = init.runner;
  runner.refresh = () => {
    const r = runner.read();
    if (r === "user" && lastRunner !== "user" && !block.read()) block.set("sci-read");
    lastRunner = r;
  };
  form.refresh();

  const host = saiSheet({ title: "カードを作る", body: form.el });
  form.host = host;
  form.actions.appendChild(seBtn("やめる", () => host.close(), "ghost"));
  form.actions.appendChild(seBtn("作る", async () => {
    const v = form.read();
    const built = seBuildCard({
      node: seTrim(nodeId) || v.node || init.node,
      block: v.block, purpose: v.purpose, expect: v.expect, runner: v.runner,
      scope: v.scope, mode: v.mode, assume: v.assume, depth: v.depth,
      papers: [...(Array.isArray(v.papers) ? v.papers : []), ...(Array.isArray(v.papers_typed) ? v.papers_typed : [])],
    });
    if (!built.ok) { form.say(built.error); return; }
    const json = built.card;
    if (preset && preset.origin_run != null) json.origin_run = preset.origin_run;
    let res = null;
    try {
      res = await paperApi(stApi("/cards"), { method: "POST", body: json });
    } catch (e) {
      form.say((e && e.message) || "カードを作れませんでした");
      return;
    }
    const runId = seCardRunId(res);
    saiToast(runId ? `run ${runId} のカードを作りました` : "カードを作りました", "ok");
    form.el.innerHTML = "";
    const done = seEl("div", "sci-edit-done");
    done.appendChild(seEl("p", "sci-edit-done-text", runId ? `run ${runId} のカードを作りました（${json.block}）` : "カードを作りました"));
    const row = seLine("sci-edit-done-row");
    if (runId) {
      const toRead = json.runner === "user";
      row.appendChild(seBtn(toRead ? "読む札を開く" : `run ${runId} を開く`, () => {
        host.close();
        location.hash = toRead
          ? `#/research/read/${encodeURIComponent(runId)}`
          : `#/run/${encodeURIComponent(selectedPaperProject())}/${encodeURIComponent(runId)}`;
      }, "primary"));
    }
    row.appendChild(seBtn("閉じる", () => host.close()));
    done.appendChild(row);
    form.el.appendChild(done);
    if (typeof stReload === "function") await stReload();
  }, "primary"));
}

// ============================================================================
// 仕分け（seProposalRow・仕分け箱）
// ============================================================================

/** 仕分け待ちの 1 行（採用・直して採用・却下／applied なら取り消しだけ）。 */
function seProposalRow(proposal, onDone) {
  const p = proposal || {};
  const row = seEl("div", "sci-edit-proposal");
  const head = seLine("sci-edit-proposal-head");
  head.appendChild(seEl("span", "sci-edit-op", seOpLabel(p.op)));
  if (p.id) head.appendChild(seEl("span", "sci-id", p.id));
  if (p.status) head.appendChild(seChip(p.status, p.status === "applied" ? "ok" : "warn"));
  const where = [p.run_id != null ? `run ${p.run_id}` : "", seTrim(p.block)].filter(Boolean).join("　");
  if (where) head.appendChild(seEl("span", "sci-edit-where", where));
  row.appendChild(head);
  row.appendChild(seEl("p", "sci-edit-proposal-text", seProposalSummary(p)));
  if (p.reason) row.appendChild(seNote(p.reason));
  if (p.created_at) row.appendChild(seNote(p.created_at));
  const btns = seLine("sci-edit-proposal-btns");
  const done = () => { if (typeof onDone === "function") onDone(); };

  if (p.status === "applied") {
    btns.appendChild(seBtn("取り消す", async (ev) => {
      const b = ev.currentTarget;
      b.disabled = true;
      try {
        await seTriageOne(p.id, { action: "revert" });
        saiToast("取り消しました", "ok");
        done();
      } catch (e) {
        saiToast((e && e.message) || "取り消せませんでした", "error");
        b.disabled = false;
      }
    }, "ghost"));
    row.appendChild(btns);
    return row;
  }

  btns.appendChild(seBtn("採用", async (ev) => {
    const b = ev.currentTarget;
    b.disabled = true;
    try {
      await seTriageOne(p.id, { action: "accept" });
      saiToast("採用しました", "ok");
      done();
    } catch (e) {
      saiToast((e && e.message) || "採用できませんでした", "error");
      b.disabled = false;
    }
  }, "primary"));
  btns.appendChild(seBtn("直して採用", () => seOpenProposalEdit(p, done)));
  btns.appendChild(seBtn("却下", async (ev) => {
    const b = ev.currentTarget;
    let note = "";
    if (typeof saiPrompt === "function") {
      const v = await saiPrompt({
        title: "却下するメモ",
        body: "後で読み返したいときはメモを書いてください（任意）。",
        ok: "却下",
      });
      if (v === null) return;
      note = seTrim(v);
    }
    b.disabled = true;
    try {
      await seTriageOne(p.id, { action: "reject", note });
      saiToast("却下しました", "ok");
      done();
    } catch (e) {
      saiToast((e && e.message) || "却下できませんでした", "error");
      b.disabled = false;
    }
  }, "ghost"));
  row.appendChild(btns);
  return row;
}

/** 「直して採用」の小窓（payload の title / text / body を直してから送る）。 */
function seOpenProposalEdit(p, onDone) {
  const keys = seProposalEditKeys(p);
  if (!keys.length) { saiToast("直す文字がありません", "error"); return; }
  const payload = p.payload || {};
  const names = { title: "題", text: "文", body: "本文", sentence: "文" };
  const fields = keys.map((k) => (k === "body" || k === "sentence"
    ? seFieldArea(k, names[k], payload[k], 5, "直してから採用します")
    : seFieldText(k, names[k], payload[k], "直してから採用します")));
  const form = seForm(fields);
  form.el.insertBefore(seNote(`${p.id || ""} ${seOpLabel(p.op)} を直して採用します。`), form.el.firstChild);
  const host = saiSheet({ title: "直して採用", body: form.el });
  form.host = host;
  form.actions.appendChild(seBtn("やめる", () => host.close(), "ghost"));
  form.actions.appendChild(seBtn("直して採用", async () => {
    const v = form.read();
    const edit = Object.assign({}, payload, { op: p.op });
    for (const k of keys) edit[k] = v[k];
    try {
      await seTriageOne(p.id, { action: "accept", edit });
      host.close();
      saiToast("直して採用しました", "ok");
      if (typeof onDone === "function") onDone();
    } catch (e) {
      form.say((e && e.message) || "採用できませんでした");
    }
  }, "primary"));
}

/** 仕分け箱の表示の切り替え（節点ごと / run ごと）を読み戻す。 */
function seLoadBy() {
  try {
    const v = localStorage.getItem(SE_BY_KEY);
    if (v === "node" || v === "run") return v;
  } catch { /* 保存できなくても動く */ }
  return "node";
}

function seSaveBy(by) {
  try { localStorage.setItem(SE_BY_KEY, by); } catch { /* 保存できなくても動く */ }
}

/** 仕分け箱の画面を開く。 */
async function seOpenTriage() {
  if (typeof document === "undefined") return;
  const box = document.getElementById("se-triage-body");
  if (!box) return;
  if (typeof ensurePaperProjects === "function") await ensurePaperProjects();
  if (typeof selectedPaperProject !== "function" || !selectedPaperProject()) {
    box.innerHTML = "";
    const msg = typeof NO_PROJECT_MSG === "string" ? NO_PROJECT_MSG : "論文の置き場があるプロジェクトがありません";
    box.appendChild(saiEmpty({ icon: "📁", title: "論文の置き場がありません", body: msg }));
    return;
  }
  await seTriageReload();
}

/** 仕分け箱を読み直して描き直す。 */
async function seTriageReload() {
  const box = document.getElementById("se-triage-body");
  if (!box) return;
  box.innerHTML = "";
  let res = null;
  try {
    res = await paperApi(stApi("/proposals"));
  } catch (e) {
    box.appendChild(saiEmpty({
      icon: "⚠️",
      title: "PROPOSAL を読めませんでした",
      body: (e && e.message) || "もう一度読み込んでください。",
    }));
    return;
  }
  const all = seProposalList(res);
  const pending = all.filter((p) => p && p.status !== "accepted" && p.status !== "rejected" && p.status !== "reverted");
  seTriageBy = seLoadBy();
  const st = typeof stStatus === "function" ? stStatus() : null;
  // まとめ替え（節点の題を足す）
  const groups = seGroupProposals(pending, seTriageBy).map((g) => {
    const title = seTriageBy === "node" ? seTitleOf(st, g.key) : "";
    return { key: g.key, label: title ? `${g.key} ${title}` : g.label, items: g.items };
  });

  if (seBulkErrors.length) {
    const failed = seEl("div", "sci-edit-bulk-errors");
    failed.appendChild(seEl("h2", "sci-edit-sub", "まとめて TRIAGE できなかった件"));
    for (const f of seBulkErrors) failed.appendChild(seEl("p", "sci-edit-error", `${f.id} ${f.why}`));
    box.appendChild(failed);
  }

  const seg = seEl("div", "dash-seg small sci-edit-by");
  for (const [key, label] of [["node", "NODE ごと"], ["run", "run ごと"]]) {
    const b = seEl("button", seTriageBy === key ? "active" : "", label);
    b.type = "button";
    b.addEventListener("click", () => {
      seTriageBy = key;
      seSaveBy(key);
      void seTriageReload();
    });
    seg.appendChild(b);
  }
  box.appendChild(seg);

  if (!pending.length) {
    box.appendChild(saiEmpty({ icon: "✅", title: "TRIAGE 待ちの PROPOSAL はありません" }));
    return;
  }
  for (const g of groups) {
    const sec = seEl("section", "sci-edit-group");
    const head = seLine("sci-edit-group-head");
    const label = seLine("sci-edit-group-label");
    label.appendChild(seEl("h2", "sci-edit-group-title", g.label));
    label.appendChild(seChip(`${g.items.length} 件`, "warn"));
    head.appendChild(label);
    const bulk = seLine("sci-edit-group-btns");
    bulk.appendChild(seBtn("まとめて採用", () => { void seBulk(g, "accept"); }));
    bulk.appendChild(seBtn("まとめて却下", () => { void seBulk(g, "reject"); }, "ghost"));
    head.appendChild(bulk);
    sec.appendChild(head);
    for (const p of g.items) sec.appendChild(seProposalRow(p, () => { void seTriageReload(); }));
    box.appendChild(sec);
  }
}

/** まとめて採用・却下（1 件ずつ順に送る。失敗した件は残して理由を出す）。 */
async function seBulk(group, action) {
  const word = action === "accept" ? "採用" : "却下";
  const ok = await saiConfirm({
    title: `まとめて${word}しますか`,
    body: `${group.label} の ${group.items.length} 件を 1 件ずつ${word}します。1 件の失敗で他は止まりません。`,
    ok: word,
    danger: action !== "accept",
  });
  if (!ok) return;
  const failures = [];
  for (const p of group.items) {
    try {
      await seTriageOne(p.id, { action });
    } catch (e) {
      failures.push({ id: p.id || "", why: (e && e.message) || "失敗しました" });
    }
  }
  seBulkErrors = failures;
  if (failures.length) saiToast(`${failures.length} 件は${word}できませんでした`, "error");
  else saiToast(`${group.items.length} 件を${word}しました`, "ok");
  await seTriageReload();
}

// ============================================================================
// 画面の登録（window.SaiRoutes）
// ============================================================================

if (typeof window !== "undefined" && typeof document !== "undefined") {
  // 仕分け箱（#/research/triage）
  (window.SaiRoutes = window.SaiRoutes || []).push({
    match(hash) {
      return /^#\/research\/triage$/.test(String(hash || "")) ? {} : null;
    },
    view: "view-sci-triage",
    tab: "research",
    hideTabbar: false,
    interval: 0, // 定期更新はしない（↻ で読み直す）
    async open() {
      await seOpenTriage();
    },
    async poll() {},
  });

  const back = document.getElementById("se-triage-back");
  if (back) back.addEventListener("click", () => { location.hash = "#/research/tree"; });
  const reload = document.getElementById("se-triage-reload");
  if (reload) reload.addEventListener("click", () => { void seTriageReload(); });
}

// --- node からの読み込み（tests/test_sciedit.js） ---
if (typeof module !== "undefined" && module.exports) {
  module.exports = {
    seTrim, seOpLabel, seCiteOk, seRefOk, seCountSentences, seNeedsCite, seBuildOp,
    seCardValues, seBuildCard, seProposalNodeKey, seProposalRunKey, seGroupProposals,
    seProposalSummary, seProposalEditKeys, seProposalList, seCardRunId,
    seNearestO, seSproutStartOs, seSproutNext, seChainPlan, seChainPath,
    SE_OPS, SE_DONE, SE_QUICK, SE_BLOCKS, SE_CITE_RE, SE_ASSUME_RE, SE_ID_RE,
  };
}
