"use strict";
// 仕分け箱（#/papers/triage）と、項目の板の中の操作（実装計画 U20・共有契約 C-14）。
//
// 板の釦は scitree.js から次の形で呼ばれる（この関数が無ければその釦は出さない=C-14）。
//   seOpenAction(kind, nodeId, detail, target)   kind は C-07 の op。detail は C-08 の node の JSON
//   seOpenCard(nodeId, suggestion)               suggestion は C-08 の next / blocks の 1 要素
//   seProposalRow(proposal, onDone)              → HTMLElement（採用・却下・取り消しの釦つき）
//   seOpenTriage()                               #/papers/triage を開く
//
// 中の処理は `phone sci apply` / `phone sci card` / `phone sci proposals` /
// `phone sci triage` だけを使う（C-13）。この画面はファイルのパスを受け取らない。
// 送る前に画面でも C-07 の必須の項目と字数と出典を確かめる。通らなければ送らず、
// 小窓の中に理由を日本語で出す。最後の検査はサーバとコアが行う。
//
// DOM を使わない純関数（seBuildOp / seGroupProposals / seOpLabel / seNeedsCite）は
// 末尾で node に渡す（tests/test_sciedit.js）。
// 一番外側の名前は se で始める（tests/test_globals.js）。

// --- 定数（C-01 の値。scitree.js の ST_* とは別の物） ---

// op の表示名（C-07 の op の一覧。表に無い op はそのまま出す）
const SE_OPS = {
  add_finding: "FINDING を足す",
  add_question: "QUESTION を足す",
  answer_question: "QUESTION に答える",
  link: "結ぶ",
  add_seed: "SEED を足す",
  add_option: "OPTION を足す",
  add_child: "ITEM を足す",
  decide: "決める",
  reopen: "開き直す",
  exclude_item: "対象外にする",
  option_state: "OPTION の状態を変える",
  exclude: "外す（TOMBSTONE）",
  edit_text: "文を直す",
  add_edge: "線を引く",
  unlink: "LINK を外す",
  remove_finding: "FINDING を外す",
  set_read_state: "読み具合を変える",
  withdraw_question: "QUESTION を取り下げる",
  remove_edge: "線を外す",
};
// 反映したときに知らせる言葉（op ごとに 1 つ）
const SE_DONE = {
  add_finding: "FINDING を足しました",
  add_question: "QUESTION を足しました",
  answer_question: "答えました",
  link: "LINK を作りました",
  add_option: "OPTION を足しました",
  add_child: "ITEM を足しました",
  decide: "決めました",
  reopen: "開き直しました",
  exclude_item: "対象外にしました",
  option_state: "OPTION の状態を変えました",
  exclude: "外しました（記録は TOMBSTONE に残ります）",
  edit_text: "直しました",
  add_edge: "線を引きました",
  unlink: "外しました",
  remove_finding: "外しました",
  set_read_state: "読み具合を変えました",
  withdraw_question: "QUESTION を取り下げました",
  remove_edge: "線を外しました",
};
// 1 押しで送る op（確かめは出さない。R-P4 の「1 押しで外せる」）
const SE_ONE_TOUCH = { remove_finding: 1, unlink: 1, withdraw_question: 1, remove_edge: 1 };
// 項目の型（C-01。組み合わせてよい）
const SE_ITEM_TYPES = [["choose", "CHOOSE"], ["list", "LIST"], ["write", "WRITE"], ["verify", "VERIFY"]];
// 候補の状態（C-07 の option_state。3 つだけ）
const SE_OPTION_STATES = [["rejected", "REJECTED"], ["dormant", "DORMANT"], ["candidate", "CANDIDATE に戻す"]];
// 知見の向き（C-01）
const SE_POLARITY = [["support", "SUPPORT"], ["refute", "REFUTE"], ["neutral", "NEUTRAL"], ["none", "NONE"]];
// 根拠の段（C-01 の level。1〜4・任意）
const SE_LEVELS = [
  ["1", "① 著者自身の限界"],
  ["2", "② 別の研究が指摘"],
  ["3", "③ 比較実験で示す"],
  ["4", "④ 自分で確かめた"],
];
// 論文の役割（C-01）
const SE_ROLES = [
  ["exemplar", "EXEMPLAR"],
  ["compare", "COMPARE"],
  ["issue_evidence", "ISSUE_EVIDENCE"],
  ["method_source", "METHOD_SOURCE"],
  ["data_source", "DATA_SOURCE"],
  ["background", "BACKGROUND"],
  ["counter", "COUNTER"],
];
// 論文の読み具合（C-01）
const SE_READ_STATES = [
  ["unread", "UNREAD"],
  ["read", "READ"],
  ["reflected", "REFLECTED"],
  ["irrelevant", "IRRELEVANT"],
];
// 線の種類（C-07。parent は指定できない）
const SE_EDGE_KINDS = [["depends", "DEPENDS"], ["derived", "DERIVED"], ["analogous", "ANALOGOUS"]];
// 結び付きの相手（C-13 の target の形）
const SE_TARGET_KINDS = [
  ["paper", "論文"],
  ["report", "報告"],
  ["note", "ノート"],
  ["set", "SEARCH_SET"],
  ["run", "実行"],
];
// 範囲（C-01）
const SE_SCOPES = [
  ["node", "NODE"],
  ["path", "PATH"],
  ["subtree", "SUBTREE"],
  ["siblings", "SIBLINGS"],
  ["summary", "SUMMARY"],
  ["blind", "BLIND"],
];
// 読む深さ（C-01）
const SE_DEPTHS = [["abstract", "ABSTRACT"], ["figures", "FIGURES"], ["full", "FULL"]];
// 実行するもの（C-09。読む札の user は sciread.js 側で扱う）
const SE_RUNNERS = [["pi", "pi"], ["cc", "cc"]];
// 16 本のブロック（C-16）
const SE_BLOCKS = [
  ["sci-start", "🌱 研究の入口"],
  ["sci-outline", "🧭 全体の見直し"],
  ["sci-digest", "🗒 まとめ"],
  ["sci-seed", "🌰 種づくり"],
  ["sci-read", "📖 1 本を読む"],
  ["sci-compare", "⚖ 比べる"],
  ["sci-probe", "🔍 確かめる"],
  ["sci-draft", "✍ 文を書く"],
  ["sci-ideate", "💡 候補を増やす"],
  ["sci-debate", "🗣 議論"],
  ["sci-coach", "🎓 理解の確認"],
  ["sci-search", "🔎 外部を探す"],
  ["sci-screen", "🧹 選別"],
  ["sci-extract", "📋 抜き出す"],
  ["sci-lookup", "📚 Library で答える"],
  ["sci-analogy", "🔁 同型を探す"],
];
// 読む札の型（C-01 の sci_mode）
const SE_MODES = [["purpose_first", "PURPOSE_FIRST"], ["blind_first", "BLIND_FIRST"]];
// 出典の形（C-07。`#37` か `#37 §3.2`）
const SE_CITE_RE = /^#[1-9][0-9]{0,8}( .{1,80})?$/;
// 結び付きの相手の形（C-13）
const SE_TARGET_RE = /^(paper:[1-9][0-9]{0,8}|report:R-[0-9]{4,8}|note:[1-9][0-9]{0,8}|set:[1-9][0-9]{0,8}|run:[1-9][0-9]{0,8})$/;
// 仮置きの形（C-13。`N12=値`）
const SE_ASSUME_RE = /^N[1-9][0-9]{0,8}=.{1,120}$/;
// 文字数の上限（C-07 / C-13）
const SE_MAX = { title: 120, body: 2000, text: 1200, question: 300, expect: 300, note: 200 };

// --- 画面の状態 ---

let seTriageBy = "node";  // 仕分け箱のまとめ方（"node" か "run"）
let seBulkErrors = [];    // まとめて採用・却下で失敗した件（理由を出す）
// 仕分け箱の表示の切り替えを覚える場所（localStorage）
const SE_BY_KEY = "sai_phone_sci_triage_by";

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

/** 出典の形か（`#37` か `#37 §3.2`）。 */
function seCiteOk(cite) {
  return SE_CITE_RE.test(seTrim(cite));
}

/** ID の形か（C-01 の N / Q / F / L / P）。 */
function seRefOk(value, prefix) {
  return new RegExp(`^${prefix}[1-9][0-9]{0,8}$`).test(seTrim(value));
}

/** 結び付きの相手の形か（C-13）。 */
function seTargetOk(target) {
  return SE_TARGET_RE.test(seTrim(target));
}

/** 文の数を数える（C-07。`0.85` や `§3.2` の「.」は数えない。0 なら 1 文）。 */
function seCountSentences(text) {
  const s = seTrim(text);
  if (!s) return 0;
  const jp = (s.match(/[。？！]/g) || []).length;
  const en = (s.match(/[.?!](?=\s|$)/g) || []).length;
  return jp + en || 1;
}

/** 数値を含み出典が無いとき true（C-07 の「数値を含む知見には出典が要ります」）。 */
function seNeedsCite(text, cites) {
  const list = (Array.isArray(cites) ? cites : []).filter((c) => seTrim(c) !== "");
  if (list.length) return false;
  return /[0-9]/.test(seTrim(text));
}

/** 候補の状態を反転させる（却下した候補を「未決」にするなど）。 */
function seOptionStateLabel(state) {
  return (SE_OPTION_STATES.find((x) => x[0] === seTrim(state)) || [])[1] || seTrim(state);
}

/**
 * 小窓の値（values）と行の相手（target）から C-07 の op の JSON を作る。
 *   → { ok: true, json: {...}, error: null } または { ok: false, json: null, error: "理由" }
 * values は op が使うキーのみ（node / title / body / text / reason …）。
 * 通らないものは日本語で理由を返し null を出す（画面では送らない）。
 */
function seBuildOp(kind, values, target) {
  const v = values || {};
  const name = seTrim(kind);
  const node = seTrim(v.node);
  const bad = (error) => ({ ok: false, json: null, error });
  const good = (json) => ({ ok: true, json, error: null });
  const needNode = () => (node ? "" : "ITEM を選んでください");
  // reason は共通項目（C-07）。空でなければ付ける
  const withReason = (json) => {
    const reason = seTrim(v.reason);
    if (reason) json.reason = reason;
    return json;
  };

  switch (name) {
    case "decide": {
      const msg = needNode();
      if (msg) return bad(msg);
      if (!seTrim(v.reason)) return bad("理由を入力してください");
      const option = seTrim(v.option);
      if (option && !seRefOk(option, "N")) return bad("OPTION の形が正しくありません");
      if (!option && v.hasOptions) return bad("OPTION を選んでください");
      const json = { op: "decide", node, tentative: !!v.tentative, counter_searched: !!v.counter_searched };
      if (option) json.option = option;
      const evidence = (Array.isArray(v.evidence) ? v.evidence : []).map(seTrim).filter(Boolean);
      for (const f of evidence) if (!seRefOk(f, "F")) return bad(`根拠 ${f} の形が正しくありません`);
      if (evidence.length) json.evidence = evidence;
      const rejected = [];
      for (const r of Array.isArray(v.rejected) ? v.rejected : []) {
        if (!r) continue;
        const opt = seTrim(r.option);
        if (!opt) continue;
        if (!seRefOk(opt, "N")) return bad(`外す OPTION ${opt} の形が正しくありません`);
        const why = seTrim(r.reason);
        if (!why) return bad(`${opt} を外す理由を入力してください`);
        if (rejected.length >= 10) return bad("まとめて外す OPTION は 10 個までです");
        rejected.push({ option: opt, reason: why });
      }
      if (rejected.length) json.rejected = rejected;
      return good(withReason(json));
    }

    case "reopen": {
      const msg = needNode();
      return msg ? bad(msg) : good({ op: "reopen", node });
    }

    case "exclude_item": {
      const msg = needNode();
      if (msg) return bad(msg);
      if (!seTrim(v.reason)) return bad("理由を入力してください");
      return good(withReason({ op: "exclude_item", node }));
    }

    case "option_state": {
      const t = target && typeof target === "object" ? target : {};
      const opt = seTrim(t.option || target) || node;
      const state = seTrim(t.state || v.state);
      if (!seRefOk(opt, "N")) return bad("OPTION を選んでください");
      if (!SE_OPTION_STATES.some((x) => x[0] === state)) return bad("状態を選んでください");
      if (!seTrim(v.reason)) return bad("理由を入力してください");
      return good(withReason({ op: "option_state", node: opt, state }));
    }

    case "add_option": {
      const msg = needNode();
      if (msg) return bad(msg);
      const title = seTrim(v.title);
      if (!title) return bad("題を入力してください");
      if (title.length > SE_MAX.title) return bad("題は 120 字までです");
      const body = seTrim(v.body);
      if (body.length > SE_MAX.body) return bad("本文は 2000 字までです");
      const json = { op: "add_option", node, title };
      if (body) json.body = body;
      return good(json);
    }

    case "add_child": {
      const msg = needNode();
      if (msg) return bad(msg);
      const title = seTrim(v.title);
      if (!title) return bad("題を入力してください");
      if (title.length > SE_MAX.title) return bad("題は 120 字までです");
      const types = (Array.isArray(v.types) ? v.types : []).map(seTrim).filter(Boolean);
      if (!types.length) return bad("型を 1 つ以上選んでください");
      for (const t of types) if (!SE_ITEM_TYPES.some((x) => x[0] === t)) return bad(`型「${t}」が分かりません`);
      return good({ op: "add_child", node, title, types });
    }

    case "add_question": {
      const msg = needNode();
      if (msg) return bad(msg);
      const text = seTrim(v.text);
      if (!text) return bad("QUESTION を入力してください");
      if (text.length > SE_MAX.question) return bad("QUESTION は 300 字までです");
      return good({ op: "add_question", node, text });
    }

    case "answer_question": {
      const question = seTrim(target && typeof target === "object" ? target.question : target);
      if (!seRefOk(question, "Q")) return bad("QUESTION が見つかりません");
      const text = seTrim(v.text);
      if (!text) return bad("答えを入力してください");
      if (text.length > SE_MAX.text) return bad("答えは 1200 字までです");
      return good({ op: "answer_question", question, text });
    }

    case "add_finding": {
      const msg = needNode();
      if (msg) return bad(msg);
      const text = seTrim(v.text);
      if (!text) return bad("文を入力してください");
      if (text.length > SE_MAX.text) return bad("文は 1200 字までです");
      if (seCountSentences(text) > 3) return bad("文は 1〜3 文までにしてください");
      const cites = (Array.isArray(v.cites) ? v.cites : []).map(seTrim).filter(Boolean);
      if (cites.length > 10) return bad("CITE は 10 個までです");
      for (const c of cites) if (!seCiteOk(c)) return bad(`CITE「${c}」の形が正しくありません（例 #37 §3.2）`);
      if (seNeedsCite(text, cites)) return bad("数値を含む FINDING には CITE が要ります");
      const json = { op: "add_finding", node, text, cites };
      if (seRefOk(v.question, "Q")) json.question = seTrim(v.question);
      if (seTrim(v.polarity)) json.polarity = seTrim(v.polarity);
      const level = Number(seTrim(v.level));
      if (level >= 1 && level <= 4) json.level = level;
      if (Number(seTrim(v.paper)) > 0) json.paper = Number(seTrim(v.paper));
      if (Number(seTrim(v.run)) > 0) json.run = Number(seTrim(v.run));
      return good(json);
    }

    case "link": {
      const msg = needNode();
      if (msg) return bad(msg);
      const t = seTrim(v.target);
      if (!t) return bad("LINK する相手を入力してください");
      if (!seTargetOk(t)) return bad("相手の形が正しくありません（例 paper:37）");
      const json = { op: "link", node, target: t };
      if (t.startsWith("paper:")) {
        if (!seTrim(v.role)) return bad("論文の役割を選んでください");
        json.role = seTrim(v.role);
        if (seTrim(v.read_state)) json.read_state = seTrim(v.read_state);
      }
      return good(withReason(json));
    }

    case "edit_text": {
      const msg = needNode();
      if (msg) return bad(msg);
      const title = seTrim(v.title);
      const body = seTrim(v.body);
      if (!title && !body) return bad("題か本文のどちらかを入力してください");
      if (title.length > SE_MAX.title) return bad("題は 120 字までです");
      if (body.length > SE_MAX.body) return bad("本文は 2000 字までです");
      const json = { op: "edit_text", node };
      if (title) json.title = title;
      if (body) json.body = body;
      return good(json);
    }

    case "add_edge": {
      const kind = seTrim(v.kind);
      if (!node) return bad("元の NODE を選んでください");
      if (!SE_EDGE_KINDS.some((x) => x[0] === kind)) return bad("線の種類を選んでください");
      const to = seTrim(v.to);
      if (!seRefOk(to, "N")) return bad("相手の NODE を選んでください");
      if (to === node) return bad("自分自身へは線を引けません");
      const note = seTrim(v.note);
      if (note.length > SE_MAX.note) return bad("メモは 200 字までです");
      const json = { op: "add_edge", kind, from: node, to };
      if (note) json.note = note;
      return good(json);
    }

    case "exclude": {
      const t = seTrim(v.target || (target && typeof target === "object" ? target.target : target));
      if (!seTargetOk(t)) return bad("外す相手が見つかりません");
      if (!seTrim(v.reason)) return bad("理由を入力してください");
      const json = { op: "exclude", target: t };
      if (seRefOk(node, "N")) json.node = node;
      if (seTrim(v.title)) json.title = seTrim(v.title);
      return good(withReason(json));
    }

    case "remove_finding": {
      const finding = seTrim(target && typeof target === "object" ? target.finding : target);
      if (!seRefOk(finding, "F")) return bad("FINDING が見つかりません");
      return good({ op: "remove_finding", finding });
    }

    case "unlink": {
      const link = seTrim(target && typeof target === "object" ? target.link : target);
      if (!seRefOk(link, "L")) return bad("LINK が見つかりません");
      return good({ op: "unlink", link });
    }

    case "withdraw_question": {
      const question = seTrim(target && typeof target === "object" ? target.question : target);
      if (!seRefOk(question, "Q")) return bad("QUESTION が見つかりません");
      return good({ op: "withdraw_question", question });
    }

    case "remove_edge": {
      const t = target && typeof target === "object" ? target : {};
      const kind = seTrim(t.kind || v.kind);
      const from = seTrim(t.from || v.from);
      const to = seTrim(t.to || v.to);
      if (!SE_EDGE_KINDS.some((x) => x[0] === kind)) return bad("線の種類が要ります");
      if (!seRefOk(from, "N") || !seRefOk(to, "N")) return bad("線の両端が要ります");
      return good({ op: "remove_edge", kind, from, to });
    }

    case "set_read_state": {
      const link = seTrim(target && typeof target === "object" ? target.link : target);
      const state = seTrim(v.state);
      if (!seRefOk(link, "L")) return bad("LINK が見つかりません");
      if (!SE_READ_STATES.some((x) => x[0] === state)) return bad("READ_STATE を選んでください");
      return good({ op: "set_read_state", link, state });
    }

    default:
      return bad("知らない操作です");
  }
}

/** 提案の節点（節点ごとのまとめの鍵）。無ければ空文字。 */
function seProposalNodeKey(p) {
  if (!p) return "";
  const direct = seTrim(p.node);
  if (seRefOk(direct, "N")) return direct;
  const num = Number(p.node_id);
  return Number.isInteger(num) && num > 0 ? `N${num}` : "";
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
        : (key ? `${key}${title ? ` ${title}` : ""}` : "NODE なし");
      map.set(key, { key, label, items: [] });
    }
    map.get(key).items.push(p);
  }
  return [...map.values()];
}

/** 提案の中身の要約（payload の題・文・本文など）。 */
function seProposalSummary(p) {
  const payload = (p && p.payload) || {};
  const parts = [];
  for (const k of ["title", "text", "body", "question", "target", "kind", "state"]) {
    const t = seTrim(payload[k]);
    if (t) parts.push(t);
  }
  if (!parts.length) parts.push(seTrim(payload.node));
  const text = parts.join("　") || "（中身がありません）";
  return text.length > 90 ? `${text.slice(0, 90)}…` : text;
}

/** payload の文字の項目（title / text / body）。直すときの小窓に使う。 */
function seProposalEditKeys(p) {
  const payload = (p && p.payload) || {};
  return ["title", "text", "body"].filter((k) => typeof payload[k] === "string" && payload[k] !== "");
}

/** 仕分けの一覧を取り出す（`{"proposals": […]}` でも、並びの配列でも、`pending` でも）。 */
function seProposalList(res) {
  if (Array.isArray(res)) return res;
  if (res && Array.isArray(res.proposals)) return res.proposals;
  if (res && Array.isArray(res.pending)) return res.pending;
  return [];
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

/** 一番上の項目（候補でない）の選択肢。 */
function seNodePickOptions() {
  const st = typeof stStatus === "function" ? stStatus() : null;
  const list = st && Array.isArray(st.nodes) ? st.nodes : [];
  return list
    .filter((n) => n && n.kind !== "option" && n.id)
    .map((n) => [n.id, `${n.id} ${n.title || ""}`]);
}

/** 節点に結び付いている論文（[{id, link, title, role, read_state}]）。`id` は論文の番号。 */
function sePaperLinks(detail) {
  const links = detail && Array.isArray(detail.links) ? detail.links : [];
  return links
    .filter((l) => l && /^paper:[1-9][0-9]{0,8}$/.test(String(l.target || "")))
    .map((l) => ({
      id: Number(String(l.target).split(":")[1]),
      link: l.id || "",
      title: l.title || l.target,
      role: l.role || "",
      read_state: l.read_state || "",
    }));
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
// 入力欄（1 種類ごとに 1 つの関数。Ui.js の部品を作り直さない）
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
  return { key, el, read: () => input.value };
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
  return { key, el, read: () => area.value };
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
  return { key, el, read: () => sel.value };
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

/** 切り替え（1 つだけ）。 */
function seFieldToggle(key, label, init, hint) {
  const el = seLine("sci-edit-field");
  const row = seEl("label", "sci-edit-check");
  const input = document.createElement("input");
  input.type = "checkbox";
  input.checked = !!init;
  row.appendChild(input);
  row.appendChild(seEl("span", null, label));
  el.appendChild(row);
  if (hint) el.appendChild(seNote(hint));
  return { key, el, read: () => input.checked };
}

/** 候補ごとに「外す」と「理由」（rejected の [{option, reason, on}]）。 */
function seFieldPairs(key, label, options, hint) {
  const el = seLine("sci-edit-field");
  el.appendChild(seEl("label", "sci-edit-label", label));
  const box = seEl("div", "sci-edit-pairs");
  const rows = [];
  for (const [value, text] of options || []) {
    const row = seEl("div", "sci-edit-pair");
    const check = document.createElement("input");
    check.type = "checkbox";
    check.setAttribute("aria-label", `${text} を外す`);
    const why = document.createElement("input");
    why.type = "text";
    why.className = "sai-input sci-edit-input";
    why.placeholder = "外す理由";
    row.appendChild(check);
    row.appendChild(seEl("span", "sci-edit-pair-name", `${value} ${text}`));
    row.appendChild(why);
    box.appendChild(row);
    rows.push({ value, check, why });
  }
  el.appendChild(box);
  if (hint) el.appendChild(seNote(hint));
  return {
    key,
    el,
    read: () => rows.map((r) => ({ option: r.value, reason: r.why.value, on: r.check.checked })),
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
  const add = seBtn("1 行足す", () => {
    if (inputs.length >= max) return;
    push("");
    add.disabled = inputs.length >= max;
  }, "ghost small");
  el.appendChild(box);
  el.appendChild(add);
  if (hint) el.appendChild(seNote(hint));
  return { key, el, read: () => inputs.map((i) => i.value) };
}

/** 相手の種類と番号で target を作る（link 用。論文のときだけ役割と読み具合を出す）。 */
function seFieldTarget(key, init, hint) {
  const el = seLine("sci-edit-field");
  el.appendChild(seEl("label", "sci-edit-label", "相手"));
  const row = seLine("sci-edit-target");
  const kinds = seFieldPick("__kind", "", SE_TARGET_KINDS, seTrim(init).split(":")[0] || "paper");
  row.appendChild(kinds.el);
  const numWrap = seLine("sci-edit-field");
  numWrap.appendChild(seEl("label", "sci-edit-label", "番号"));
  // 報告だけは R-0012 の形なので、数の入力にはしない
  const num = document.createElement("input");
  num.type = "text";
  num.className = "sai-input sci-edit-input";
  num.value = seTrim(init).split(":")[1] || "";
  numWrap.appendChild(num);
  row.appendChild(numWrap);
  el.appendChild(row);
  const role = seFieldSelect("role", "ROLE", SE_ROLES, "", "論文のときは必ず選ぶ", true);
  const readState = seFieldSelect("read_state", "READ_STATE", SE_READ_STATES, "unread", null, true);
  el.appendChild(role.el);
  el.appendChild(readState.el);
  if (hint) el.appendChild(seNote(hint));
  const sync = () => {
    const kind = kinds.read();
    const isPaper = kind === "paper";
    role.el.hidden = !isPaper;
    readState.el.hidden = !isPaper;
    num.placeholder = kind === "report" ? "R-0012" : "37";
  };
  sync();
  return {
    key,
    el,
    read: () => `${kinds.read()}:${seTrim(num.value)}`,
    refresh: sync,
    subs: [kinds, role, readState],
  };
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
// 通信（C-13）
// ============================================================================

/** 人の操作を送って描き直す。送れなかったときの理由は小窓の中に出す。 */
async function seApply(json, form) {
  try {
    const res = await paperApi(stApi("/apply"), { method: "POST", body: json });
    if (form && form.host && typeof form.host.close === "function") form.host.close();
    saiToast(SE_DONE[json.op] || "反映しました", "ok");
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
// 項目の板の中の操作（seOpenAction）
// ============================================================================

/** op ごとの入力欄（表示名と既定値つき）。 */
function seFieldsFor(kind, nodeId, detail, target) {
  const d = detail || {};
  const node = d.node || {};
  const options = Array.isArray(d.options) ? d.options : [];
  const findings = Array.isArray(d.findings) ? d.findings : [];
  const links = Array.isArray(d.links) ? d.links : [];
  const questions = Array.isArray(d.questions) ? d.questions : [];
  const st = typeof stStatus === "function" ? stStatus() : null;
  const otherNodes = (st && Array.isArray(st.nodes) ? st.nodes : [])
    .filter((n) => n && n.id && n.id !== nodeId)
    .map((n) => [n.id, `${n.id} ${n.title || ""}`]);
  const nodeOptions = options.map((o) => [o.id, `${o.id} ${o.title || ""}`]);
  const findingOptions = findings.map((f) => [f.id, `${f.id} ${(f.text || "").slice(0, 40)}`]);
  const questionOptions = questions.map((q) => [q.id, `${q.id} ${(q.text || "").slice(0, 40)}`]);
  const paperOptions = links
    .filter((l) => /^paper:[1-9][0-9]{0,8}$/.test(String(l.target || "")))
    .map((l) => [String(l.target).split(":")[1], `${l.target} ${l.title || ""}`]);
  const base = { node: nodeId || node.id || "" };

  switch (kind) {
    case "decide": {
      const free = options.filter((o) => o.state !== "selected");
      return {
        title: `決める　${base.node}`,
        values: Object.assign({}, base, { hasOptions: options.length > 0 }),
        fields: [
          nodeOptions.length
            ? seFieldSelect("option", "選ぶ OPTION", nodeOptions, "", "OPTION が無いときは空のままにする", true)
            : seFieldText("option", "選ぶ OPTION", "", "OPTION の番号（N20 の形）"),
          seFieldToggle("tentative", "TENTATIVE にする", false, "決め切らず、仮として残す"),
          seFieldChecks("evidence", "根拠の FINDING", findingOptions, [], "この NODE の FINDING から選ぶ（無くてもよい）"),
          seFieldPairs("rejected", "外す OPTION と理由", free.map((o) => [o.id, o.title || ""]), "チェックを付けて理由を書く"),
          seFieldToggle("counter_searched", "反証を探した", false),
          seFieldArea("reason", "理由", "", 3, "この判断の理由（必須）"),
        ],
      };
    }
    case "exclude_item":
      return {
        title: `対象外にする　${base.node}`,
        values: base,
        fields: [seFieldArea("reason", "理由", "", 3, "対象外にする理由（必須）")],
      };
    case "option_state": {
      const t = target && typeof target === "object" ? target : {};
      const opt = seTrim(t.option || target);
      const found = options.find((o) => o.id === opt);
      return {
        title: `OPTION の状態を変える　${opt}`,
        values: Object.assign({}, base, { state: t.state || "rejected" }),
        fields: [
          seFieldPick("state", "状態", SE_OPTION_STATES, t.state || "rejected"),
          seFieldArea("reason", "理由", "", 3, `${opt || "この OPTION"} の状態を ${seOptionStateLabel(t.state)} にする理由（必須）`),
        ],
      };
    }
    case "add_option":
      return {
        title: `OPTION を足す　${base.node}`,
        values: base,
        fields: [
          seFieldText("title", "題", "", "必須・120 字まで"),
          seFieldArea("body", "本文", "", 4, "2000 字まで（無くてもよい）"),
        ],
      };
    case "add_child":
      return {
        title: `ITEM を足す　${base.node}`,
        values: base,
        fields: [
          seFieldText("title", "題", "", "必須・120 字まで"),
          seFieldChecks("types", "ITEM_TYPE", SE_ITEM_TYPES, [], "1 つ以上選ぶ（組み合わせてもよい）"),
        ],
      };
    case "add_question":
      return {
        title: `QUESTION を足す　${base.node}`,
        values: base,
        fields: [seFieldArea("text", "QUESTION", "", 3, "必須・300 字まで")],
      };
    case "answer_question": {
      const qid = seTrim(target);
      const q = questions.find((x) => x.id === qid);
      return {
        title: `QUESTION に答える　${qid}`,
        values: base,
        fields: [seFieldArea("text", "答え", q && q.answer ? q.answer : "", 4, `必須・1200 字まで（${qid}）`)],
      };
    }
    case "add_finding":
      return {
        title: `FINDING を足す　${base.node}`,
        values: Object.assign({}, base, { polarity: "support" }),
        fields: [
          seFieldArea("text", "文", "", 3, "必須・1〜3 文・1200 字まで"),
          seFieldLines("cites", "CITE", [], 10, "1 行 1 つ。`#37 §3.2` の形（数字を書くときは必ず CITE）", "#37 §3.2"),
          seFieldSelect("polarity", "POLARITY", SE_POLARITY, "support", null, true),
          seFieldSelect("level", "根拠の段", SE_LEVELS, "", "任意", true),
          questionOptions.length ? seFieldSelect("question", "QUESTION", questionOptions, "", "この QUESTION への答えなら選ぶ", true) : null,
          paperOptions.length ? seFieldSelect("paper", "論文", paperOptions, "", "任意（この NODE に LINK されている論文）", true) : null,        ],
      };
    case "link":
      return {
        title: `結ぶ　${base.node}`,
        values: Object.assign({}, base, { target: "paper:" }),
        fields: [
          seFieldTarget("target", "paper:", "番号は Library にある番号（論文 37・報告 R-0012）"),          seFieldArea("reason", "理由", "", 3, "任意（記録に残る）"),
        ],
      };
    case "edit_text":
      return {
        title: `文を直す　${base.node}`,
        values: Object.assign({}, base, { title: node.title || "", body: node.body || "" }),
        fields: [
          seFieldText("title", "題", node.title || "", "120 字まで"),
          seFieldArea("body", "本文", node.body || "", 6, "2000 字まで"),
        ],
      };
    case "add_edge":
      return {
        title: `線を引く　${base.node}`,
        values: Object.assign({}, base, { kind: "depends" }),
        fields: [
          seFieldPick("kind", "線の種類", SE_EDGE_KINDS, "depends"),
          seFieldSelect("to", "相手の NODE", otherNodes, "", "どの NODE へ線を引くか", false),
          seFieldText("note", "メモ", "", "200 字まで（任意）"),
        ],
      };
    case "exclude": {
      const lid = seTrim(target);
      const link = links.find((l) => l.id === lid);
      return {
        title: `外す　${link ? link.title || lid : lid}`,
        values: Object.assign({}, base, { target: link ? link.target : "", title: link ? link.title || "" : "" }),
        fields: [
          seEl("p", "sci-edit-note", "外した論文は以後の検索結果に出なくなります（記録は TOMBSTONE に残します）。"),
          seFieldArea("reason", "理由", "", 3, "必須"),
        ],
      };
    }
    case "set_read_state": {
      const lid = seTrim(target);
      const link = links.find((l) => l.id === lid);
      const state = (link && link.read_state) || "unread";
      return {
        title: `読み具合　${link ? link.title || lid : lid}`,
        values: Object.assign({}, base, { state }),
        fields: [seFieldPick("state", "READ_STATE", SE_READ_STATES, state)],
      };
    }
    default:
      return { title: seOpLabel(kind), values: base, fields: [] };
  }
}

/**
 * op の小窓を出す。1 押しで外す op はそのまま送り、reopen は確かめだけ出す。
 * 送る前に画面でも C-07 の必須の項目と字数と出典を確かめる。通らなければ送らない。
 */
function seOpenAction(kind, nodeId, detail, target) {
  if (typeof document === "undefined" || typeof saiSheet !== "function") return;
  const name = seTrim(kind);
  // 小窓の値には op ごとの既定値（候補の有無・外す相手の結び付きなど）も入れ込む
  let spec = { title: seOpLabel(kind), values: { node: seTrim(nodeId) }, fields: [] };
  const values = () => Object.assign({}, spec.values, { node: seTrim(nodeId) }, form.read());

  // 1 押しで外す（確かめは出さない・R-P4）
  if (SE_ONE_TOUCH[name]) {
    const built = seBuildOp(name, { node: seTrim(nodeId) }, target);
    if (!built.ok) { saiToast(built.error, "error"); return; }
    void seApply(built.json, null);
    return;
  }

  // 開き直す（確かめだけ）
  if (name === "reopen") {
    void (async () => {
      const ok = await saiConfirm({
        title: "開き直しますか",
        body: "OPEN に戻し、SELECTED の OPTION を CANDIDATE に戻します。",
        ok: "開く",
      });
      if (!ok) return;
      const built = seBuildOp("reopen", { node: seTrim(nodeId) }, target);
      if (!built.ok) { saiToast(built.error, "error"); return; }
      await seApply(built.json, null);
    })();
    return;
  }

  spec = seFieldsFor(name, nodeId, detail, target);
  const body = seEl("div", "sci-edit-wrap");
  const form = seForm(spec.fields);
  body.appendChild(form.el);
  const host = saiSheet({ title: spec.title, body });
  form.host = host;
  form.actions.appendChild(seBtn("やめる", () => host.close(), "ghost"));

  const send = async () => {
    const v = values();
    // 候補ごとの「外す」は、チェックした組だけを送る
    if (Array.isArray(v.rejected)) v.rejected = v.rejected.filter((r) => r.on);
    const built = seBuildOp(name, v, target);
    if (!built.ok) { form.say(built.error); return; }
    await seApply(built.json, form);
  };

  // 外す（墓標）は確かめを出す
  if (name === "exclude") {
    form.actions.appendChild(seBtn("外す", async () => {
      const ok = await saiConfirm({
        title: "この論文は検索結果に出なくなります",
        body: "外した記録は TOMBSTONE に残ります。もう一度結べば元に戻ります。",
        ok: "外す",
        danger: true,
      });
      if (!ok) return;
      await send();
    }, "danger"));
    return;
  }
  form.actions.appendChild(seBtn("送る", send, "primary"));
}

/**
 * カードを作る小窓（次の一手から 1 押しで作る）。
 * 目的が空のカードは送らない（C-13。サーバも 400 を返す）。
 */
async function seOpenCard(nodeId, suggestion) {
  if (typeof document === "undefined" || typeof saiSheet !== "function") return;
  const s = suggestion || {};
  let node = seTrim(nodeId);
  let detail = null;
  if (node) {
    try {
      detail = await paperApi(stApi(`/nodes/${encodeURIComponent(node)}`));
    } catch (e) {
      saiToast((e && e.message) || "ITEM を読み込めませんでした", "error");
      return;
    }
  } else {
    // 節点を選ばせる（既定は一番上の最初の項目）
    const picks = seNodePickOptions();
    node = picks.length ? String(picks[0][0]) : "";
  }
  const d = detail || {};
  const questions = (Array.isArray(d.questions) ? d.questions : []).map((q) => [q.id, `${q.id} ${(q.text || "").slice(0, 40)}`]);
  const papers = sePaperLinks(d);
  const block = SE_BLOCKS.some((b) => b[0] === s.block) ? s.block : "";
  const picks = seNodePickOptions();

  const fields = [];
  // nodeId が無かったとき（種づくりなど）は節点を選ばせる
  if (!seTrim(nodeId)) {
    if (picks.length) fields.push(seFieldSelect("node", "NODE", picks, node, "CARD を向ける NODE", false));
    else fields.push(seEl("p", "sci-edit-note", "ITEM がまだありません。まず ITEM を足してください。"));
  }
  fields.push(seFieldSelect("block", "ブロック", SE_BLOCKS, block, "実行するブロック（16 本から選ぶ）", false));
  fields.push(seFieldText("purpose", "目的", s.purpose || "", "必須・300 字まで。エージェントが何を達成するか", "何を達成するか"));
  fields.push(seFieldText("expect", "期待する成果", s.expect || "", "300 字まで（任意）"));
  if (questions.length) fields.push(seFieldSelect("question", "QUESTION", questions, "", "この QUESTION に答える（任意）", true));
  fields.push(seFieldSelect("runner", "実行するもの", SE_RUNNERS, "pi", "READ_CARD（人が読む）は「READ_CARD」ボタンから作る", false));
  fields.push(seFieldSelect("scope", "SCOPE", SE_SCOPES, "", "空のときはブロックの既定", true));
  fields.push(seFieldSelect("mode", "MODE", SE_MODES, "purpose_first", "1 本を読むときだけ使う", true));
  fields.push(seFieldLines("assume", "ASSUMPTION", [], 5, "1 行 1 つ。`N12=発作部位` の形（5 個まで）", "N12=発作部位"));
  if (papers.length) {
    fields.push(seFieldChecks("papers", "論文", papers.map((p) => [String(p.id), `#${p.id} ${p.title}`]), [], "この NODE に LINK されている論文から選ぶ"));
  }
  const form = seForm(fields);
  // 「読み方」は sci-read のときだけ出す
  const mode = fields.find((f) => f.key === "mode");
  if (mode) mode.show = (v) => v.block === "sci-read";
  form.refresh();

  const host = saiSheet({ title: "CARD を作る", body: form.el });
  form.host = host;
  form.actions.appendChild(seBtn("やめる", () => host.close(), "ghost"));
  form.actions.appendChild(seBtn("作る", async () => {
    const v = Object.assign({ node }, form.read());
    if (!seRefOk(v.node, "N")) { form.say("NODE を選んでください"); return; }
    if (!SE_BLOCKS.some((b) => b[0] === v.block)) { form.say("ブロックを選んでください"); return; }
    const purpose = seTrim(v.purpose);
    if (!purpose) { form.say("目的を入力してください（空の目的では作りません）"); return; }
    if (purpose.length > SE_MAX.expect) { form.say("目的は 300 字までです"); return; }
    const expect = seTrim(v.expect);
    if (expect.length > SE_MAX.expect) { form.say("期待する成果は 300 字までです"); return; }
    const assume = (Array.isArray(v.assume) ? v.assume : []).map(seTrim).filter(Boolean);
    if (assume.length > 5) { form.say("ASSUMPTION は 5 個までです"); return; }
    for (const a of assume) if (!SE_ASSUME_RE.test(a)) { form.say(`ASSUMPTION「${a}」の形が正しくありません（N12=値）`); return; }
    const paperIds = (Array.isArray(v.papers) ? v.papers : []).map(Number).filter((n) => n > 0);
    if (paperIds.length > 20) { form.say("論文は 20 本までです"); return; }
    const json = { node: v.node, block: v.block, purpose, runner: v.runner || "pi", papers: paperIds };
    if (expect) json.expect = expect;
    if (seRefOk(v.question, "Q")) json.question = seTrim(v.question);
    if (seTrim(v.scope)) json.scope = seTrim(v.scope);
    if (v.block === "sci-read" && seTrim(v.mode)) json.mode = seTrim(v.mode);
    if (assume.length) json.assume = assume;
    let res = null;
    try {
      res = await paperApi(stApi("/cards"), { method: "POST", body: json });
    } catch (e) {
      form.say((e && e.message) || "CARD を作れませんでした");
      return;
    }
    const runId = seCardRunId(res);
    saiToast(runId ? `run ${runId} のCARD を作りました` : "CARD を作りました", "ok");
    // Project の run の画面へ移る釦を出す（小窓の中に残す）
    form.el.innerHTML = "";
    const done = seEl("div", "sci-edit-done");
    done.appendChild(seEl("p", "sci-edit-done-text", runId ? `run ${runId} のCARD を作りました（${json.block}）` : "CARD を作りました"));
    const row = seLine("sci-edit-done-row");
    if (runId) {
      row.appendChild(seBtn(`run ${runId} を開く`, () => {
        host.close();
        location.hash = `#/project/${encodeURIComponent(selectedPaperProject())}/list/${encodeURIComponent(runId)}/task`;
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
  const names = { title: "題", text: "文", body: "本文" };
  const fields = keys.map((k) => (k === "body"
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
  // 仕分け箱（#/papers/triage）
  (window.SaiRoutes = window.SaiRoutes || []).push({
    match(hash) {
      return /^#\/papers\/triage$/.test(String(hash || "")) ? {} : null;
    },
    view: "view-sci-triage",
    tab: "papers",
    hideTabbar: false,
    interval: 0, // 定期更新はしない（↻ で読み直す）
    async open() {
      await seOpenTriage();
    },
    async poll() {},
  });

  const back = document.getElementById("se-triage-back");
  if (back) back.addEventListener("click", () => { location.hash = "#/papers/tree"; });
  const reload = document.getElementById("se-triage-reload");
  if (reload) reload.addEventListener("click", () => { void seTriageReload(); });
}

// --- node からの読み込み（tests/test_sciedit.js） ---
if (typeof module !== "undefined" && module.exports) {
  module.exports = {
    seTrim,
    seOpLabel,
    seCiteOk,
    seRefOk,
    seTargetOk,
    seCountSentences,
    seNeedsCite,
    seOptionStateLabel,
    seBuildOp,
    seProposalNodeKey,
    seProposalRunKey,
    seGroupProposals,
    seProposalSummary,
    seProposalEditKeys,
    seProposalList,
    seCardRunId,
    SE_OPS,
    SE_DONE,
    SE_ONE_TOUCH,
    SE_BLOCKS,
    SE_OPTION_STATES,
    SE_CITE_RE,
    SE_TARGET_RE,
    SE_ASSUME_RE,
  };
}
