"use strict";
// Research-tree の操作の入力欄・カードのフォーム・PROPOSAL の仕分け（実装計画 U19）。
//
// 板の釦は scitree.js から次の形で呼ばれる（この関数が無ければその釦は出さない）。
//   seOpenAction(op, nodeId, detail)   op は C-OP の 13 個。detail は node の JSON
//   seQuick(op, payload)               ★・Answer/Issue・目安は入力欄を出さずに送る
//   seOpenCard(nodeId, preset)         preset は C-CARD。値の入ったフォームを開く
//   seProposalRow(proposal, onDone)    → HTMLElement（採用・却下・取り消しの釦つき）
//
// 中の処理は `POST …/sci/apply` / `…/sci/cards` / `…/sci/proposals/{pid}` だけを使う。
// この画面はファイルのパスを受け取らない。送る前に画面でも必須の項目と字数と出典を確かめる。
// 通らなければ送らず、小窓の中に理由を日本語で出す。最後の検査はサーバとコアが行う。
//
// DOM を使わない純関数（seBuildOp / seBuildCard / seCardValues / seProposalSummary など）は
// 末尾で node に渡す（tests/test_sciedit.js）。
// 一番外側の名前は se で始める（tests/test_globals.js）。

// --- 定数 ---

// op の表示名（C-OP の 13 個。表に無い op はそのまま出す。C4 の共有契約）
const SE_OPS = {
  add_q: "Add Q", add_o: "Add O", add_f: "Add F", add_e: "Add E",
  record_logic: "Logicality を記録", add_sprout: "SPROUT を作る", copy: "Copy O", remove: "Delete",
  star: "★ を付ける", unstar: "★ を外す", set_kind: "Answer / Issue を変える",
  edit: "Edit", set_target: "Objectivity の目安を変える",
};
// 反映したときに知らせる言葉（op ごとに 1 つ）
const SE_DONE = {
  add_q: "Q を足しました", add_o: "O を足しました", add_f: "F を足しました", add_e: "E を足しました",
  record_logic: "Logicality を記録しました", add_sprout: "SPROUT を作りました", copy: "O を写しました",
  remove: "消しました", star: "★ を付けました", unstar: "★ を外しました",
  set_kind: "Answer / Issue を変えました", edit: "文を直しました", set_target: "目安を変えました",
};
// 入力欄を出さずに送る op
const SE_QUICK = ["star", "unstar", "set_kind", "set_target"];
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
const SE_ID_RE = /^[QOFES][1-9][0-9]{0,8}$/;
// 文字数の上限
const SE_MAX = { qtext: 300, otext: 120, ftext: 1200, etext: 120, body: 2000, note: 1000, mlflow: 200, title: 120, purpose: 300, expect: 300 };

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
 * 通らないものは日本語で理由を返す（画面では送らない）。
 */
function seBuildOp(kind, values) {
  const v = values || {};
  const name = seTrim(kind);
  const bad = (error) => ({ ok: false, op: null, json: null, error });
  const good = (op) => ({ ok: true, op, json: op, error: null });
  // 必須の ID を確かめる。prefix は許す頭の文字
  const id = (key, prefix, label) => (seRefOk(v[key], prefix) ? "" : `${label}を選んでください`);
  // add_q の親は SEED（ID を持たない特別な節点）か Q・O の ID
  const qParentOk = (value) => seTrim(value) === "SEED" || seRefOk(value, "QO");
  // 字数を確かめる。min が 1 なら必須
  const len = (key, label, min, max) => {
    const n = seTrim(v[key]).length;
    if (n < min) return `${label}を入力してください`;
    if (n > max) return `${label}は ${max} 字までです`;
    return "";
  };
  const first = (...msgs) => msgs.find(Boolean) || "";

  switch (name) {
    case "add_q": {
      const msg = first(qParentOk(v.parent) ? "" : "親（SEED・Q・O）を選んでください", len("text", "文", 1, SE_MAX.qtext));
      if (msg) return bad(msg);
      return good({ op: name, parent: seTrim(v.parent), text: seTrim(v.text) });
    }
    case "add_o": {
      const kindV = seTrim(v.kind);
      const msg = first(id("q", "Q", "Q"), len("text", "文", 1, SE_MAX.otext), len("body", "本文", 0, SE_MAX.body));
      if (msg) return bad(msg);
      if (kindV && !SE_KINDS.some((x) => x[0] === kindV)) return bad("種類は Answer か Issue です");
      const op = { op: name, q: seTrim(v.q), text: seTrim(v.text) };
      if (seTrim(v.body)) op.body = seTrim(v.body);
      if (kindV) op.kind = kindV;
      return good(op);
    }
    case "add_f": {
      const msg = first(id("parent", "OE", "親（O か E）"), len("text", "文", 1, SE_MAX.ftext));
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
      return good(op);
    }
    case "add_e": {
      const msg = first(id("parent", "SE", "親（S か E）"), len("text", "文", 1, SE_MAX.etext), len("body", "本文", 0, SE_MAX.body), len("mlflow", "MLflow の名前", 0, SE_MAX.mlflow));
      if (msg) return bad(msg);
      const op = { op: name, parent: seTrim(v.parent), text: seTrim(v.text) };
      if (seTrim(v.body)) op.body = seTrim(v.body);
      if (seTrim(v.mlflow)) op.mlflow = seTrim(v.mlflow);
      return good(op);
    }
    case "record_logic": {
      const value = seTrim(v.value);
      const msg = first(id("q", "Q", "Q"), len("note", "指摘の文", 0, SE_MAX.note));
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
      return good(op);
    }
    case "add_sprout": {
      const msg = first(len("title", "題", 1, SE_MAX.title), len("body", "本文", 0, SE_MAX.body));
      if (msg) return bad(msg);
      const paths = seList(v.paths);
      if (!paths.length) return bad("PATH の O を 1 つ以上選んでください");
      if (paths.length > 20) return bad("PATH は 20 個までです");
      for (const o of paths) if (!seRefOk(o, "O")) return bad(`O の形が正しくありません: ${o}`);
      if (new Set(paths).size !== paths.length) return bad("同じ O を 2 回選んでいます");
      const list = Array.isArray(v.nodes) ? v.nodes : null;
      if (list) {
        const issues = new Set(
          list.filter((n) => n && /issue/i.test(n.answer_kind || "")).map((n) => String(n.id)),
        );
        if (!paths.some((o) => issues.has(o))) return bad("Issue の O を 1 つ以上選んでください");
      }
      const op = { op: name, title: seTrim(v.title), paths };
      if (seTrim(v.body)) op.body = seTrim(v.body);
      return good(op);
    }
    case "copy": {
      const msg = first(id("node", "O", "写す O"), id("to", "Q", "写す先の Q"));
      return msg ? bad(msg) : good({ op: name, node: seTrim(v.node), to: seTrim(v.to) });
    }
    case "remove": {
      const msg = id("node", "QOFES", "消す Node");
      return msg ? bad(msg) : good({ op: name, node: seTrim(v.node) });
    }
    case "star":
    case "unstar": {
      const msg = id("o", "O", "O");
      return msg ? bad(msg) : good({ op: name, o: seTrim(v.o) });
    }
    case "set_kind": {
      const msg = id("o", "O", "O");
      if (msg) return bad(msg);
      const kindV = seTrim(v.kind);
      if (!SE_KINDS.some((x) => x[0] === kindV)) return bad("種類は Answer か Issue です");
      return good({ op: name, o: seTrim(v.o), kind: kindV });
    }
    case "edit": {
      const msg = first(id("node", "QOFES", "直す Node"), len("text", "文", 0, SE_MAX.ftext), len("body", "本文", 0, SE_MAX.body));
      if (msg) return bad(msg);
      if (!seTrim(v.text) && !seTrim(v.body)) return bad("文か本文のどちらかを入力してください");
      const op = { op: name, node: seTrim(v.node) };
      if (seTrim(v.text)) op.text = seTrim(v.text);
      if (seTrim(v.body)) op.body = seTrim(v.body);
      return good(op);
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
    mode: SE_MODES.some((m) => m[0] === p.mode) ? p.mode : "purpose_first",
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
  if (node !== "SEED" && !seRefOk(node)) return bad("Node を選んでください");
  if (!SE_BLOCKS.some((b) => b[0] === v.block)) return bad("ブロックを選んでください");
  const purpose = seTrim(v.purpose);
  if (!purpose) return bad("目的を入力してください（空の目的では作りません）");
  if (purpose.length > SE_MAX.purpose) return bad("目的は 300 字までです");
  const expect = seTrim(v.expect);
  if (expect.length > SE_MAX.expect) return bad("期待する成果は 300 字までです");
  const runner = SE_RUNNERS.some((r) => r[0] === v.runner) ? v.runner : "pi";
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
    if (!/^[QO]/.test(node)) return bad("読む札の対象は Q か O にしてください");
    if (papers.length !== 1) return bad("読む札の論文はちょうど 1 本にしてください");
    card.depth = SE_DEPTHS.some((d) => d[0] === v.depth) ? v.depth : "full";
  }
  if (expect) card.expect = expect;
  if (v.block === "sci-read" && seTrim(v.mode)) card.mode = seTrim(v.mode);
  return { ok: true, card, error: null };
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

/** 子を持たない Q（bottom_Q。node の `bottom_q` を見る）。 */
function seBottomQs(nodes) {
  return (Array.isArray(nodes) ? nodes : []).filter((n) => n && seRefOk(n.id, "Q") && n.bottom_q);
}

/** O から上へたどった一番上の Q（top_Q）の ID（無ければ null）。巡回しても止まらないように見た ID を覚える。 */
function seTopQOf(nodes, nodeId) {
  const byId = new Map((Array.isArray(nodes) ? nodes : []).filter((n) => n && n.id != null).map((n) => [String(n.id), n]));
  let cur = byId.get(String(nodeId));
  let top = null;
  const seen = new Set();
  while (cur && !seen.has(String(cur.id))) {
    seen.add(String(cur.id));
    if (seRefOk(cur.id, "Q")) top = String(cur.id);
    cur = cur.parent == null ? null : byId.get(String(cur.parent));
  }
  return top;
}

/** 短く切った文（表示用）。 */
function seShort(text, n) {
  return Array.from(seTrim(text)).slice(0, n).join("");
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
      fields.push(text);
      const fromSeed = id === "SEED";
      return { title: fromSeed ? "top_Q を足す" : `Q を足す（${id}）`, values: { parent: fromSeed ? "SEED" : id }, fields };
    }
    case "add_o":
      return {
        title: `O を足す（${id}）`,
        values: { q: id },
        fields: [
          seFieldText("text", "文", "", "必須・120 字まで"),
          seFieldArea("body", "本文", "", 4, "任意・2000 字まで"),
          seFieldSelect("kind", "種類", SE_KINDS, "answer", null, false),
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
    case "add_sprout": {
      // 生きている O を top_Q ごとにまとめる（Issue の O を先に）。値は O の ID。
      const os = nodes
        .filter((n) => n && seRefOk(n.id, "O"))
        .map((o) => ({ o, top: seTopQOf(nodes, o.id), issue: /issue/i.test(o.answer_kind || "") }))
        .sort((a, b) => {
          if (a.top !== b.top) return String(a.top || "").localeCompare(String(b.top || ""));
          if (a.issue !== b.issue) return a.issue ? -1 : 1;
          return String(a.o.id).localeCompare(String(b.o.id));
        });
      const options = os.map(({ o, issue }) => [String(o.id), `${o.id} ${issue ? "Issue" : "Answer"} ${seShort(o.text || o.title || "", 40)}`.trim()]);
      return {
        title: "SPROUT を作る",
        values: { nodes },
        fields: [
          seFieldText("title", "題", "", "必須・120 字まで"),
          seFieldArea("body", "本文", "", 4, "任意・実験の計画の要点・2000 字まで"),
          seFieldChecks("paths", "PATH（終点の O）", options, [], "1 つ以上・Issue の O を必ず含める"),
        ],
      };
    }
    case "copy":
      return {
        title: `O を写す（${id}）`,
        values: { node: id },
        fields: [seFieldSelect("to", "写す先の Q", seBottomQs(nodes).map(seNodeOption), "", "子を持たない Q", false)],
      };
    case "remove":
      return {
        title: `消す（${id}）`,
        values: { node: id },
        fields: [],
      };
    case "edit":
      return {
        title: `文を直す（${id}）`,
        values: { node: id },
        fields: [
          seFieldArea("text", "文", node.text || "", 3, "文か本文のどちらかは必須"),
          seFieldArea("body", "本文", node.body || "", 5, "2000 字まで"),
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
 * op の小窓を出す。SE_QUICK の op は seQuick に回す。remove は確認を必ず出す。
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
  // E は SPROUT（S）か E の下にしか作れない。O から開いたときは断る。
  if (name === "add_e" && !/^[SE]/.test(seTrim(nodeId))) {
    saiToast("親（S か E）になる Node から開いてください", "error");
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
  const wantId = seTrim(nodeId);
  let detail = null;
  if (wantId && wantId !== "SEED") {
    try {
      detail = await paperApi(stApi(`/nodes/${encodeURIComponent(wantId)}`));
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
  if (!wantId) {
    const picks = [["SEED", "SEED（research-tree の全体）"], ...seStatusNodes().filter((n) => n && seRefOk(n.id)).map(seNodeOption)];
    fields.push(seFieldSelect("node", "Node", picks, init.node, "カードを向ける Node", false));
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
    mode,
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
      mode: v.mode, depth: v.depth,
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
// PROPOSAL の仕分け（seProposalRow）
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
    const ok = await saiConfirm({
      title: "却下しますか",
      body: `${p.id || "この PROPOSAL"} を却下します。`,
      ok: "却下",
      danger: true,
    });
    if (!ok) return;
    b.disabled = true;
    try {
      await seTriageOne(p.id, { action: "reject" });
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

// --- node からの読み込み（tests/test_sciedit.js） ---
if (typeof module !== "undefined" && module.exports) {
  module.exports = {
    seTrim, seOpLabel, seCiteOk, seRefOk, seCountSentences, seNeedsCite, seBuildOp,
    seCardValues, seBuildCard, seProposalSummary, seProposalEditKeys, seProposalList, seCardRunId,
    seBottomQs,
    SE_OPS, SE_DONE, SE_QUICK, SE_BLOCKS, SE_CITE_RE, SE_ID_RE,
  };
}
