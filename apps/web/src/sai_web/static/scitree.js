"use strict";
// research-tree の画面（軸・A・F・E・S の木。旧形式は Q を含む）。住所は #/research/tree と #/research/tree/<ID>。
//
// 上の帯に注意の札を出し、左に木の一覧、真ん中に
// research-tree の図（scigraph.js）、右に選んだ Node の板（何も選ばなければ出さない）を出す。
// 何も選んでいないときは一覧の上に「プロジェクト」（型と mind.md・status.project）を出す。
// 狭い画面（幅 390px）の板は下から出る（saiSheet）。
// 複数の S を軸ごとに並べる比べる画面（#/research/compare?s=S1,S2）も、この画面の中に出す。
//
// 読むのは sai-paper の `phone sci status` / `types` / `nodes/<ID>` / `compare` だけ（API は U17）。
// 操作は sciedit.js（seOpenAction・seOpenCard・seQuick・seOpLabel）、図は
// scigraph.js（sgRenderGraph・sgMarkSelected・sgCenterOn）の関数を typeof で確かめてから呼ぶ。
// DOM を使わない純関数は末尾で node に渡す（tests/test_scitree.js）。
// 画面の名詞（Q・A・F・E・S・seed・SPROUT・stance など）は英語のまま出す。

// --- 定数 ---

// RESEARCH_TYPE（/sci/types が返せば、その説明で置き換える）
const ST_TYPES = [
  ["problem", "PROBLEM_DRIVEN", "最初に課題（困っていること）を決めて、何をなぜやるのかから始める研究です。"],
  ["phenomenon", "PHENOMENON_DRIVEN", "先に現象を捉え、「なぜ起きるのか」を問う研究です。"],
  ["method", "METHOD_DRIVEN", "先に手法を決め、その手法で何が分かるようになるかを考える研究です。"],
  ["data", "DATA_DRIVEN", "手にあるデータから問いを立てる研究です。"],
  ["improve", "IMPROVE_DRIVEN", "いまのものの弱いところから始めた研究です。"],
];
// F の stance（[送る値, 印, 名]）。値は API の STANCES（support/deny/issue）。印は一覧・図・板の札に出す（色は sci-stance-S/D/I の CSS）。
const ST_STANCE = [["support", "S", "支持"], ["deny", "D", "反対"], ["issue", "I", "問い"]];
// 左の一覧の絞り込みで切り替えられる種類（図の札と同じ 6 つ。D は seed）
const ST_ALL_KINDS = ["Q", "A", "F", "E", "S", "D"];
// 境界線をドラッグしたときの幅の範囲（px）
const ST_SPLIT_LIMITS = { list: [200, 600], board: [280, 720] };
// 図の最小の幅（px）。これより狭くしない。
const ST_GRAPH_MIN = 300;
// 境界線 2 本ぶんの幅（図の幅を計算するとき引く）
const ST_SPLIT_BAR = 12;
// MLflow の参照を押したときに開く画面（Experiments の一覧。run の名前では絞らない）
const ST_EXPERIMENTS_HASH = "#/research/experiments";
// 板に出す操作（表示名と送り方は sciedit.js の seOpLabel・seOpenAction・seQuick）。
// 軸（Q）の板は add_a だけ。A の板は子の A（add_a）・F・move_node・copy。旧 op は出さない（stOpLabel だけ持つ）。
const ST_NODE_OPS = {
  Q: ["add_a"],
  A: ["add_a", "add_f", "move_node", "copy"],
  F: [],
  E: ["add_e", "set_mlflow"],
  // S（SPROUT）は BRANCH を足せて、題と本文を直せる。親が無いので Edit / Delete もここに出す。
  S: ["add_e", "edit", "remove"],
  // D（seed）は親が無い。Edit / Delete はここに出す（SPROUT の作成は板の釦で行う）。
  D: ["edit", "remove"],
};
// 軸の表示名（問いの軸のキー → 比べる画面の列の見出し）
const ST_AXIS_LABEL = { task: "解く問題", target: "対象", data: "データ", method: "手法", output: "出力", metric: "評価指標" };
// 回答の札（固定・新規は持ち主で決まる。未回答は問いに回答が無いとき。実験は S の下の E）。札の文字は必ず出す（色だけで示さない）
const ST_ROLE_LABEL = { fixed: "固定", novel: "新規", open: "未回答", exp: "実験" };
// 比べるに足せる S の数（sai-sci compare の上限と同じ）
const ST_COMPARE_MAX = 10;

// --- 画面の状態 ---

let stData = null;      // 最後に読んだ status
let stStep = null;      // 見ている step（null は最新。過去の step は表示だけ・編集できない）
let stStepEl = null;    // 図の道具の行に入れた step の札（stBuildStepper が作る）
let stTypesData = null; // 最後に読んだ /sci/types
let stNode = null;      // 選んだ Node の NodeDetail
let stNodeId = null;    // 今開いている Node の ID
let stSheet = null;     // 狭い画面の板（saiSheet が返した物）
let stSheetBody = null; // 板の中の入れ物
let stSheetFor = null;  // 板が出している物の鍵
let stSprout = null;    // SPROUT 作成中の下書き {seed, title, body, error}。null なら作成中ではない
let stSproutBusy = false; // 送信中か（二重に送らない）
let stPick = null;      // 問いに root path（A）を割り当てている最中 {owner, q, text}。owner は D か S。null なら選んでいない
let stSeed = null;      // seed（D）作成中の下書き {form}（入力欄は sciedit.js の seSeedFields）。null なら作成中ではない
let stSeedBusy = false; // 送信中か（二重に送らない）
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
  const m = /^#\/research\/tree(?:\/([QAFESD][1-9][0-9]{0,8}))?$/.exec(String(hash || ""));
  if (!m) return null;
  return m[1] ? { id: m[1] } : {};
}

/**
 * 比べる画面の住所（`#/research/compare?s=S1,S2`）を読む。合わなければ null。
 * S の ID だけを受け、重複は外し、ST_COMPARE_MAX 個までにする。
 */
function stParseCompareHash(hash) {
  const m = /^#\/research\/compare(?:\?(.*))?$/.exec(String(hash || ""));
  if (!m) return null;
  const raw = new URLSearchParams(m[1] || "").get("s") || "";
  const ids = [];
  for (const t of raw.split(",")) {
    const id = t.trim();
    if (/^S[1-9][0-9]{0,8}$/.test(id) && !ids.includes(id)) ids.push(id);
  }
  return { ids: ids.slice(0, ST_COMPARE_MAX) };
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
  // SEED は木の Node ではなくなった（根の型は一覧の上の「プロジェクト」に出す・U12）
  if (h === "#/research/tree/SEED") return "#/research/tree";
  return null;
}

/** SPROUT の範囲の数の文（`Q 3・A 4・F S 2 / D 1 / I 0`）。range が無ければ空文字。 */
function stRangeText(range) {
  if (!range || typeof range !== "object") return "";
  const n = (v) => Number(v) || 0;
  // 旧形式（Q を持つ SPROUT）だけ Q の数を出す
  const q = n(range.q) ? `Q ${n(range.q)}・` : "";
  return `${q}A ${n(range.a)}・F S ${n(range.f_support)} / D ${n(range.f_deny)} / I ${n(range.f_issue)}`;
}

/** status.nodes の seed（D）だけ。 */
function stSeedNodes(status) {
  const nodes = status && Array.isArray(status.nodes) ? status.nodes : [];
  return nodes.filter((n) => n && n.kind === "D");
}

/** seed（seedId）の子の SPROUT（親が seedId の S）。古い S は親が無いので含まない。 */
function stSeedChildren(status, seedId) {
  const nodes = status && Array.isArray(status.nodes) ? status.nodes : [];
  return nodes.filter((n) => n && n.kind === "S" && n.parent != null && String(n.parent) === String(seedId));
}

/** S の親の seed（D）。親が無い（古い S）か D が見つからなければ null。 */
function stSeedOf(status, sprout) {
  const pid = sprout && sprout.parent != null ? String(sprout.parent) : "";
  if (!pid) return null;
  return stSeedNodes(status).find((n) => String(n.id) === pid) || null;
}

/** seed（D）の入力に書いた物があるか（「空欄」の選びと空の欄は数えない）。 */
function stSeedDirty(values) {
  return Object.entries(values || {}).some(([k, v]) => !(k.endsWith("_mode") && v === "empty") && String(v == null ? "" : v).trim() !== "");
}

/**
 * S（SPROUT）の板の材料（純関数）。S でなければ null。
 * seed は親の D（古い S は null）。questions は S が答える問い（凍結した S は完成した時点の問い）。
 * complete は完成か（真偽）、missing は未回答の問いの ID。exps は S の子孫の E（生きた Node だけ）。
 */
function stSproutSections(status, sproutId) {
  const nodes = status && Array.isArray(status.nodes) ? status.nodes : [];
  const byId = new Map(nodes.filter((n) => n && n.id != null).map((n) => [String(n.id), n]));
  const sprout = byId.get(String(sproutId));
  if (!sprout || sprout.kind !== "S") return null;
  const seed = stSeedOf(status, sprout);
  // S の子孫の E（生きた Node だけ）
  const under = (n) => {
    const seen = new Set();
    for (let p = n.parent; p != null && !seen.has(String(p)); p = (byId.get(String(p)) || {}).parent) {
      if (String(p) === String(sproutId)) return true;
      seen.add(String(p));
    }
    return false;
  };
  const exps = nodes
    .filter((n) => n && n.kind === "E" && under(n))
    .map((n) => ({ id: String(n.id), title: String(n.text || ""), mlflow: n.mlflow || null }));
  return {
    seed: seed ? { id: String(seed.id), title: String(seed.text || "") } : null,
    questions: Array.isArray(sprout.questions) ? sprout.questions : [],
    complete: sprout.complete === true,
    missing: Array.isArray(sprout.missing) ? sprout.missing.map(String) : [],
    exps,
  };
}

/**
 * 問いの節の行（純関数）。D は自分の問いと自分の回答、S は答える問いと D・S の回答。
 * 回答の札は持ち主で決める（D なら固定、S なら新規）。
 *   canAnswer: 回答を足せるか（D は生きた問い全部。S は D が答えていない問い）
 *   canEdit:   問いを直す・消せるか（D の板の生きた問い）
 *   answers[].mine: 持ち主が今の Node（✕ と文の直しを出せる）
 */
function stQuestionRows(node) {
  const id = String((node && node.id) || "");
  const isSeed = !!node && node.kind === "D";
  const list = node && Array.isArray(node.questions) ? node.questions : [];
  return list.filter((q) => q && q.id != null).map((q) => {
    const removed = q.removed === true;
    const answers = (Array.isArray(q.answers) ? q.answers : []).map((a) => {
      const owner = String(a.owner || "");
      const role = owner.charAt(0) === "D" ? "fixed" : "novel";
      return {
        id: String(a.id),
        owner,
        role,
        label: ST_ROLE_LABEL[role],
        a: a.a ? String(a.a) : null,
        aText: a.a_text == null ? "" : String(a.a_text),
        text: a.text == null ? "" : String(a.text),
        mine: owner === id,
      };
    });
    const seedAnswered = answers.some((a) => a.owner.charAt(0) === "D");
    return {
      id: String(q.id),
      text: String(q.text || ""),
      axis: q.axis || null,
      removed,
      answers,
      canAnswer: !removed && (isSeed || !seedAnswered),
      canEdit: isSeed && !removed,
    };
  });
}

/** 問いの節の見出し。S は完成か、残りの未回答の数を出す。 */
function stQuestionsTitle(node) {
  const id = String((node && node.id) || "");
  if (!node || node.kind !== "S") return `問い（${id}）`;
  if (node.complete === true) return `問い（${id}・完成）`;
  const n = Array.isArray(node.missing) ? node.missing.length : 0;
  return `問い（${id}・未完成: 残り ${n}）`;
}

/** 子の SPROUT の札（完成・未完成の残り）。S でなければ空文字。 */
function stSproutStateText(node) {
  if (!node || node.kind !== "S") return "";
  if (node.complete === true) return "完成";
  return `未完成 ${Array.isArray(node.missing) ? node.missing.length : 0}`;
}

/** 比べる画面の表（C-CMP-1）を描く形にする。同じ列で同じ A は同じ印（①②…）。same が false の軸は diff。 */
function stCompareRows(json) {
  const o = json || {};
  const axes = Array.isArray(o.axes) ? o.axes.map(String) : [];
  const same = o.same || {};
  const marks = new Map(axes.map((ax) => [ax, new Map()]));
  const sprouts = Array.isArray(o.sprouts) ? o.sprouts : [];
  const rows = sprouts.map((s) => {
    const cells = s.cells || {};
    return {
      id: String(s.id),
      title: String(s.title || ""),
      seed: s.seed || null,
      cells: axes.map((ax) => {
        const c = cells[ax] || {};
        const a = c.a ? String(c.a) : null;
        let mark = null;
        if (a) {
          const m = marks.get(ax);
          if (!m.has(a)) m.set(a, m.size);
          mark = stCircled(m.get(a));
        }
        const role = ST_ROLE_LABEL[c.role] ? c.role : "open";
        return { axis: ax, a, mark, role, label: ST_ROLE_LABEL[role], text: String(c.text || "") };
      }),
    };
  });
  return {
    axes: axes.map((ax) => ({ key: ax, label: ST_AXIS_LABEL[ax] || ax, diff: same[ax] === false })),
    rows,
  };
}

/** 印の番号（0 から）を丸数字にする。20 を越えたら (21) の形。 */
function stCircled(i) {
  return i < 20 ? String.fromCharCode(0x2460 + i) : `(${i + 1})`;
}

/** 文を 1 行の短い文にする（空白をまとめ、n 字を越えたら … を付ける）。 */
function stShort(text, n = 40) {
  const t = String(text == null ? "" : text).replace(/\s+/g, " ").trim();
  return t.length > n ? `${t.slice(0, n)}…` : t;
}

/** 比べるの並びに S を足す／外す。S 以外・重複・上限を超えるものは足さない。 */
function stCompareToggle(list, id, max = ST_COMPARE_MAX) {
  const out = Array.isArray(list) ? list.slice() : [];
  const i = out.indexOf(id);
  if (i >= 0) { out.splice(i, 1); return out; }
  if (!/^S[1-9][0-9]{0,8}$/.test(String(id)) || out.length >= max) return out;
  out.push(id);
  return out;
}

/** 許可した人の文（`許可: <名前>`）。無ければ空文字。 */
function stApprovalText(h) {
  const who = h && h.approved_by != null ? String(h.approved_by).trim() : "";
  return who ? `許可: ${who}` : "";
}

/** F の stance の札（[印, 名]）。support/deny/issue 以外は null。 */
function stStanceLabel(v) {
  const hit = ST_STANCE.find((s) => s[0] === v);
  return hit ? [hit[1], hit[2]] : null;
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
    stance: n.stance || null,
    rel: n.rel || null,
    node: n,
  }));
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
 * node が status.nodes に無い（seed など）ときは今までの label（purpose）を使う。
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

/**
 * 板に出す操作の並び（出さないものを除く）。
 * S（SPROUT）は、答える問いの全部に回答が付いたとき（complete）だけ BRANCH（add_e）を出す。
 */
function stOpsFor(node) {
  if (!node) return [];
  return (ST_NODE_OPS[node.kind] || []).filter((op) => {
    if (op === "add_e") return node.kind === "E" || (node.kind === "S" && node.complete === true);
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

/** 絞り込みの文字に行が合うか。`*` だけなら ★ の Node、それ以外は ID か文に入っているか。 */
function stRowHit(r, word) {
  if (word === "*") return !!r.starred;
  return stRowHasWord(r, word);
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
    const hit = kinds.has(r.kind) && stRowHit(r, word);
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

/**
 * 過去の step（step k）の木を作る。status は書き換えず、新しい status を返す。
 * step が無い・steps が無い・k が総数以上なら status をそのまま返す（今の木）。
 * 見えるのは born <= k かつ（gone が無い か gone > k）で、親も見えている Node（親が見えないものは隠す）。
 * 親は parents のうち step <= k の最後の parent。並びは深さ順（根は Q → D → S の順）。
 */
function stStepView(status, step) {
  const steps = status && status.steps;
  if (step == null || !steps || !(step < steps.total)) return status;
  const k = step;
  const all = new Map();
  for (const n of [...(status.nodes || []), ...(steps.removed || [])]) {
    if (n && n.id != null) all.set(String(n.id), n);
  }
  const ts = new Map((steps.nodes || []).map((t) => [String(t.id), t]));
  // step k で効く親の行（{step, parent}）。生まれる前なら null
  const at = (id) => {
    let hit = null;
    for (const p of (ts.get(id) || {}).parents || []) if (p.step <= k) hit = p;
    return hit;
  };
  const memo = new Map();
  const shown = (id) => {
    if (memo.has(id)) return memo.get(id);
    memo.set(id, false); // 親をたどる循環を止める
    const t = ts.get(id);
    const p = at(id);
    let ok = !!(t && all.has(id) && p && t.born <= k && (t.gone == null || t.gone > k));
    if (ok && p.parent != null) ok = shown(String(p.parent));
    memo.set(id, ok);
    return ok;
  };
  const ids = [...all.keys()].filter(shown);
  const kids = new Map();
  const roots = [];
  for (const id of ids) {
    const p = at(id).parent;
    if (p == null) roots.push(id);
    else if (kids.has(String(p))) kids.get(String(p)).push(id);
    else kids.set(String(p), [id]);
  }
  const num = (id) => Number(String(id).replace(/^\D+/, "")) || 0;
  const ord = (id) => Number(all.get(id).ord) || 0;
  // 同じ親の下の並び: 親の下に来た step → ord → ID の数
  const cmp = (a, b) => at(a).step - at(b).step || ord(a) - ord(b) || num(a) - num(b);
  const rank = (id) => ({ Q: 0, D: 1, S: 2 })[all.get(id).kind] ?? 3;
  roots.sort((a, b) => rank(a) - rank(b) || cmp(a, b));
  const kidsOf = (id) => (kids.get(id) || []).slice().sort(cmp);
  const order = [];
  const walk = (id, depth) => {
    order.push({ id, depth });
    for (const c of kidsOf(id)) walk(c, depth + 1);
  };
  for (const r of roots) walk(r, 0);
  const nodes = order.map(({ id, depth }) => {
    const n = all.get(id);
    const kinds = kidsOf(id).map((c) => all.get(c).kind);
    const cnt = (kind) => kinds.filter((x) => x === kind).length;
    return {
      ...n,
      parent: at(id).parent,
      depth,
      counts: { q: cnt("Q"), a: cnt("A"), f: cnt("F"), e: cnt("E") },
      bottom_q: n.kind === "Q" && cnt("Q") === 0,
      range: n.kind === "S" ? null : n.range,
    };
  });
  const tally = { q: 0, a: 0, f_support: 0, f_deny: 0, f_issue: 0, e: 0, s: 0 };
  const fKey = { support: "f_support", deny: "f_deny", issue: "f_issue" };
  for (const n of nodes) {
    if (n.kind === "Q") tally.q++;
    else if (n.kind === "A") tally.a++;
    else if (n.kind === "F" && fKey[n.stance]) tally[fKey[n.stance]]++;
    else if (n.kind === "E") tally.e++;
    else if (n.kind === "S") tally.s++;
  }
  return { ...status, nodes, counts: tally, next: [], decisions: [], history: [] };
}

/**
 * 板の NodeDetail を step の木に合わせる（detail は書き換えない）。view は stStepView の戻り値。
 * その step に無い Node は null。history と decisions は step の時刻より後の行を落とす
 * （history は Add・Run だけ。init は残す）。
 */
function stStepNodeView(detail, view, step) {
  const nodes = view && Array.isArray(view.nodes) ? view.nodes : [];
  const id = detail && detail.node && detail.node.id != null ? String(detail.node.id) : null;
  const node = nodes.find((n) => String(n.id) === id);
  if (!node) return null;
  const byId = new Map(nodes.map((n) => [String(n.id), n]));
  const parentNode = (n) => (n.parent == null ? null : byId.get(String(n.parent)) || null);
  const subtree = nodes.filter((n) => {
    for (let p = parentNode(n); p; p = parentNode(p)) if (String(p.id) === id) return true;
    return false;
  });
  const path = [];
  for (let p = node; p; p = parentNode(p)) path.unshift({ id: p.id, kind: p.kind, text: p.text });
  const list = view.steps && Array.isArray(view.steps.list) ? view.steps.list : [];
  const cut = Date.parse((list[step - 1] || {}).at);
  const later = (h) => Number.isFinite(cut) && Date.parse(h && h.at) > cut;
  return {
    ...detail,
    node,
    children: nodes.filter((n) => n.parent != null && String(n.parent) === id),
    subtree,
    path,
    history: (detail.history || []).filter((h) => !(later(h) && (/^Add /.test(String(h.action || "")) || h.action === "Run"))),
    decisions: (detail.decisions || []).filter((h) => !later(h)),
    next: [],
  };
}

/** step の番号の入力を 1〜総数の数にする。数でなければ null（総数が 0 のときも null）。 */
function stStepParse(text, total) {
  const t = String(text == null ? "" : text).trim();
  const max = Number(total) || 0;
  if (max < 1 || !/^[0-9]+$/.test(t)) return null;
  return Math.min(max, Math.max(1, Number(t)));
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
  if (stWriteBlocked()) return false;
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

/** 過去の step を見ているあいだは書き込まない（止めたら true。理由は toast に出す）。 */
function stWriteBlocked() {
  if (!stFrozen()) return false;
  stToast("過去の step は表示だけです。最新に戻ると編集できます", "error");
  return true;
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
  // step の総数が減った（消えた・始め直した）ときは最新に戻す
  const total = stStepTotal();
  if (stStep != null && (total < 1 || stStep >= total)) stStep = null;
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
    // 過去の step では、消えた Node は今の DB に無いので step の記録から板を作る
    const gone = stFrozen() ? stRemovedDetail(stNodeId) : null;
    if (gone) { stNode = gone; return; }
    stNodeId = null;
    stSetHash();
  }
}

/** 消えた Node（過去の step にだけ出る）の板の元。記録の NodeJson から作り、history は空。無ければ null。 */
function stRemovedDetail(id) {
  const removed = (stData && stData.steps && stData.steps.removed) || [];
  const hit = removed.find((n) => String(n.id) === String(id));
  return hit ? { node: hit, children: [], subtree: [], path: [], history: [], decisions: [], next: [] } : null;
}

/** Node（SPROUT の S1 も含む）を選ぶ。 */
async function stSelect(id) {
  stNodeId = id || null;
  await stLoadNode();
  stSetHash();
  stListOpen = false;
  stSyncListOpen();
  stRenderSidebar(document.getElementById("sci-list"));
  stRenderBoard();
  stMarkGraph();
}

function stSetHash() {
  const next = stNodeId ? `#/research/tree/${stNodeId}` : "#/research/tree";
  if (location.hash !== next) history.replaceState(null, "", next);
}

/** 画面を開く（app.js の route から呼ばれる）。 */
async function stOpen(params) {
  const id = (params && params.id) || null;
  stNodeId = id;
  stSprout = null; // 前の下書きは残さない
  stPick = null;
  stSeed = null;
  stStep = null;   // 開き直したら最新から
  stData = null;
  stNode = null;
  stListOpen = false;
  stLeaveCompare();
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
  stRenderSeed();
  stRenderSidebar(document.getElementById("sci-list"));
  stRenderBoard();
  stRenderGraph();
}

/** 「古い形の木」と注意の帯（Next Action・History は Dashboard と右の板にある・U06）。 */
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
    box.appendChild(stEl("p", "sci-note sci-legacy", "古い形の tree です。新しい tree（軸・A・F・E・S）には読み替えていません。"));
  }
  box.appendChild(stChipBand(s));
}

/**
 * 解く釦。kind が card なら Create CARD、user なら Q を選ぶ。それ以外は出さない。
 * onUser があれば「選ぶ」の押し方を変える（Dashboard は Node の画面へ移る）。
 */
function stActionBtn(x, onUser) {
  const card = (node, preset) => stBtn("Create CARD", () => { stCall("seOpenCard", node, preset); });
  if (x.kind === "card") return card(x.node, x.card);
  if (x.kind === "user") return stBtn("選ぶ", () => { if (onUser) onUser(x); else void stSelect(x.node); });
  return null;
}

/** Next Action の先頭 3 件（.sci-next-row）。onUser は stActionBtn と同じ。 */
function stNextRows(list, status, onUser) {
  const wrap = stEl("div", "sci-next");
  for (const x of list) {
    const row = stEl("div", "sci-line sci-next-row");
    row.appendChild(stEl("span", "sci-line-main", stNextText(x, status)));
    const b = stActionBtn(x, onUser);
    if (b) row.appendChild(b);
    wrap.appendChild(row);
  }
  return wrap;
}

/** 注意の札と、狭い画面の一覧の出し入れ（.sci-chips）。 */
function stChipBand(s) {
  const row = stEl("div", "sci-chips");
  if (stFrozen()) {
    row.appendChild(stEl("span", "sci-chip warn", `過去の状態（step ${stStep}/${stStepTotal()}）を表示中・編集できません`));
    row.appendChild(stBtn("最新へ", () => { void stSetStep(null, false); }));
  }
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
  mk.disabled = !!stSprout || stFrozen();
  row.appendChild(mk);
  // 比べるに足した S が 2 つ以上あれば、比べる画面へ（sessionStorage の並び）
  const cmpIds = stCompareList();
  if (cmpIds.length >= 2) row.appendChild(stBtn(`比べる（${cmpIds.length}）`, () => { stGo(stCompareHash(cmpIds)); }, "primary"));
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

/**
 * 作成を始める。作成中なら題の欄へ戻る。seedId があれば、その seed（D）を選んだ状態にする（D の板の「この seed で」）。
 */
function stStartSprout(seedId) {
  if (stWriteBlocked()) return;
  const focusTitle = () => {
    const box = document.getElementById("sci-sprout");
    if (!box) return;
    if (typeof box.scrollIntoView === "function") box.scrollIntoView({ block: "nearest" });
    const t = box.querySelector('[data-field="title"]');
    if (t) t.focus();
  };
  if (stSprout === null) {
    stSprout = { seed: "", title: "", body: "", error: "" };
    stRenderTop(); // 帯の釦を「作成中」にする
    stRenderBoard();
    stMarkGraph();
  }
  if (seedId) stSprout.seed = String(seedId);
  stRenderSprout();
  focusTitle();
}

async function stSproutCancel() {
  if (stSprout === null || stSproutBusy) return;
  const dirty = stSprout.title.trim() || stSprout.body.trim();
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
    ? seBuildOp("add_sprout", { seed: stSprout.seed, title: stSprout.title, body: stSprout.body })
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
  const range = stRangeText(res.range);
  if (range) stToast(range, "ok");
  if (res.result && /^S[1-9][0-9]*$/.test(String(res.result))) await stSelect(String(res.result));
}

// --- Add root path（問いに path の A を割り当てる。帯は #sci-sprout の一番上） ---

/**
 * Add root path を始める。owner は D か S、q は問い（SQ）。帯に問いの文を出し、A を選ぶまで step の釦を止める。
 */
function stStartPick(owner, q, text) {
  if (stWriteBlocked()) return;
  stPick = { owner: String(owner), q: String(q), text: String(text || "") };
  stRenderSprout();
  stMarkGraph();
  const box = document.getElementById("sci-sprout");
  if (box && typeof box.scrollIntoView === "function") box.scrollIntoView({ block: "nearest" });
}

/** Add root path をやめる（入力は無いので確かめない）。 */
function stPickCancel() {
  stPick = null;
  stRenderSprout();
  stMarkGraph();
}

/** 選ぶ帯（問いの文と「やめる」）。 */
function stPickBand() {
  const band = stEl("section", "sci-band sci-pick-band");
  band.appendChild(stEl("span", "sci-band-title", "Add root path"));
  band.appendChild(stEl("p", "sci-note", `${stPick.q}『${stPick.text}』の path を選ぶ: DAG か一覧で A を選び、板の『この A で回答』を押します`));
  band.appendChild(stBtn("やめる", () => { stPickCancel(); }, "ghost"));
  return band;
}

/**
 * A の板の「この A で回答」。add_answer で問いに path を割り当て、成功したら持ち主（D か S）の板を開く。
 * 失敗の理由は toast に出る（seApply）。
 */
async function stAnswerPicked(a) {
  const pick = stPick;
  if (pick === null || stWriteBlocked()) return;
  const built = seBuildOp("add_answer", { to: pick.owner, q: pick.q, a: String(a) });
  if (!built.ok) { stToast(built.error, "error"); return; }
  const res = await seApply(built.op, null);
  if (!res) return;
  stPick = null;
  stRenderSprout();
  await stSelect(pick.owner);
}

/** Add root path の最中に、その問いに今ある path の A（図の札に破線の印を付ける）。 */
function stPickedIds() {
  if (stPick === null) return [];
  const owner = ((stView() && stView().nodes) || []).find((n) => String(n.id) === stPick.owner);
  const q = owner && Array.isArray(owner.questions) ? owner.questions.find((x) => String(x.id) === stPick.q) : null;
  return q && Array.isArray(q.answers) ? q.answers.filter((x) => x.a).map((x) => String(x.a)) : [];
}

/** パネルを作り直す。題・本文は下書きから戻し、直前にフォーカスしていた欄へ戻す。 */
function stRenderSprout() {
  stSyncStepper(); // 下書きの有無で step の釦を止める・戻す
  const box = document.getElementById("sci-sprout");
  if (!box) return;
  const active = document.activeElement;
  const field = active && box.contains(active) && active.dataset ? active.dataset.field : null;
  box.innerHTML = "";
  if (stSprout === null && stPick === null) { box.hidden = true; return; }
  box.hidden = false;
  if (stPick !== null) box.appendChild(stPickBand());
  if (stSprout === null) return;
  const d = stSprout;
  const band = stEl("section", "sci-band sci-sprout-band");
  band.appendChild(stEl("span", "sci-band-title", "Create SPROUT"));

  // 1 つの SPROUT は 1 つの seed（D）の下に作る。seed が無ければ作り方を出す
  const seeds = stSeedNodes(stData);
  if (d.seed && !seeds.some((n) => String(n.id) === d.seed)) d.seed = "";
  if (!seeds.length) {
    band.appendChild(stEl("p", "sci-note", "先に seed（D）を作ってください（seed は研究ごとの制約です）"));
    band.appendChild(stBtn("Create seed（D）", () => { stStartSeed(); }));
  } else {
    const pick = stEl("select", "sai-input sci-sprout-seed");
    pick.dataset.field = "seed";
    pick.setAttribute("aria-label", "seed（D・必須）");
    const none = stEl("option", "", "seed（D）を選ぶ（必須）");
    none.value = "";
    pick.appendChild(none);
    for (const n of seeds) {
      const o = stEl("option", "", `${n.id} ${n.text || ""}`.trim());
      o.value = String(n.id);
      pick.appendChild(o);
    }
    pick.value = d.seed || "";
    pick.addEventListener("change", () => { d.seed = pick.value; d.error = ""; stRenderSprout(); });
    band.appendChild(pick);
  }
  band.appendChild(stEl("p", "sci-note", "作った後で、S の板の問いに root path か回答を割り当てます。全部の問いに回答すると完成し、実験（BRANCH）を足せます。"));

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

// ============================================================================
// seed（D）作成パネル（#sci-seed。SPROUT のパネルの下。入力欄は sciedit.js の seSeedFields）
// ============================================================================

/** seed（D）の作成を始める。作成中なら入力の欄へ戻るだけ。 */
function stStartSeed() {
  if (stWriteBlocked()) return;
  if (typeof seForm !== "function" || typeof seSeedFields !== "function") {
    stToast("seed（D）の入力欄がまだ使えません", "error");
    return;
  }
  if (stSeed === null) {
    const form = seForm(seSeedFields({}));
    form.actions.appendChild(stBtn("やめる", () => { void stSeedCancel(); }, "ghost"));
    form.actions.appendChild(stBtn("Create seed（D）", () => { void stSeedSend(); }, "primary"));
    stSeed = { form };
  }
  stRenderSeed();
  const box = document.getElementById("sci-seed");
  if (box && typeof box.scrollIntoView === "function") box.scrollIntoView({ block: "nearest" });
}

/** seed（D）のパネルを作り直す。入力欄の DOM は置き直すので、打った物は残る。 */
function stRenderSeed() {
  stSyncStepper();
  const box = document.getElementById("sci-seed");
  if (!box) return;
  box.innerHTML = "";
  if (stSeed === null) { box.hidden = true; return; }
  box.hidden = false;
  const band = stEl("section", "sci-band sci-seed-band");
  band.appendChild(stEl("span", "sci-band-title", "Create seed（D）"));
  band.appendChild(stEl("p", "sci-note", "研究ごとの seed を作ります。作ると、解く問題・対象・データ・手法・出力・評価指標・成功基準の 7 つの問いが入ります。問いは後から足す・直す・消すことができます。"));
  band.appendChild(stSeed.form.el);
  box.appendChild(band);
}

/** 検査して送る。成功したら作った D を選ぶ。失敗したら入力を残して理由を出す。 */
async function stSeedSend() {
  if (stSeed === null || stSeedBusy) return;
  const v = stSeed.form.read();
  const built = typeof seBuildOp === "function"
    ? seBuildOp("add_seed", { title: v.title, body: v.body })
    : { ok: false, error: "seed（D）の検査がまだ使えません" };
  if (!built.ok) { stSeed.form.say(built.error); return; }
  stSeed.form.say("");
  stSeedBusy = true;
  let res = null;
  try {
    res = typeof seApply === "function" ? await seApply(built.op, null) : null;
  } finally {
    stSeedBusy = false;
  }
  if (!res) return; // 失敗の理由は toast に出ている
  stSeed = null;
  stRenderSeed();
  const id = String(res.result || "");
  if (/^D[1-9][0-9]*$/.test(id)) await stSelect(id);
}

/** 作成をやめる。入力が残っていれば確かめる。 */
async function stSeedCancel() {
  if (stSeed === null || stSeedBusy) return;
  if (stSeedDirty(stSeed.form.read()) && typeof saiConfirm === "function") {
    const ok = await saiConfirm({ title: "作成をやめますか", body: "入力した内容は消えます。", ok: "やめる", danger: true });
    if (!ok) return;
  }
  stSeed = null;
  stRenderSeed();
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

/** F の stance の印（S・D・I の札。F 以外・stance が無いときは null）。 */
function stStanceBadge(stance) {
  const hit = stStanceLabel(stance);
  if (!hit) return null;
  const b = stEl("span", `sci-stance sci-stance-${hit[0]}`, hit[0]);
  b.title = `stance: ${hit[1]}`;
  b.setAttribute("aria-label", `stance ${hit[1]}`);
  return b;
}

/** MLflow の参照（押すと Experiments の画面へ。run の名前では絞らない）。 */
function stMlflowBtn(name) {
  const b = stEl("button", "sci-link-title sci-mlflow", String(name));
  b.type = "button";
  b.title = "Experiments を開く";
  b.addEventListener("click", () => stGo(ST_EXPERIMENTS_HASH));
  return b;
}

/** 木の 1 行（.sci-row）。一覧と板の子で共通。 */
function stRowEl(r, opts) {
  const o = opts || {};
  const row = stEl("div", `sci-row sci-row-${r.kind}${r.id === stNodeId ? " selected" : ""}${o.ctx ? " sci-row-ctx" : ""}`);
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
  const badge = r.kind === "F" ? stStanceBadge(r.stance) : null;
  if (badge) head.appendChild(badge);
  head.appendChild(stEl("span", "sci-id", r.id));
  // 条件の A の目印（一覧と板の子の行。行は button の中なので用語の札は使わず文字だけ）
  if (r.rel === "condition") head.appendChild(stEl("span", "sci-rel-cond", "条件"));
  let title = r.text;
  if (r.kind === "F") title = title.slice(0, 60);
  head.appendChild(stEl("span", "sci-row-title", title));
  main.appendChild(head);
  row.appendChild(main);
  if (r.kind === "E" && r.node.mlflow) row.appendChild(stMlflowBtn(r.node.mlflow));
  // ★ は目印（どの種類の Node にも付けられる）
  const star = stEl("button", `sci-star${r.starred ? " on" : ""}`, r.starred ? "★" : "☆");
  star.type = "button";
  star.setAttribute("aria-label", r.starred ? "★ を外す" : "★ を付ける");
  star.disabled = stFrozen();
  star.addEventListener("click", () => { stCall("seQuick", r.starred ? "unstar" : "star", { node: r.id }); });
  row.appendChild(star);
  return row;
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
  input.placeholder = "ID・文で絞り込み（* で ★ だけ）";
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
  // 何も選んでいないときは、木の上に根の型と mind.md を出す（seed の板は無くした・U12）
  if (!stNodeId) box.appendChild(stProjectBlock());
  const rows = stRows(stView());
  if (!rows.length) {
    box.appendChild(stEmpty({ title: "Node がありません", body: "" }));
    return;
  }
  // 過去の step の一覧は一部の木なので、畳みの記憶を捨てない（stPruneFolded は最新だけ）
  if (!stFrozen()) stPruneFolded(rows);
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
  sgRenderGraph(box, { status: stData, key: selectedPaperProject(), tools: [stBuildStepper()] }, (sel) => {
    if (sel && sel.kind === "node") void stSelect(sel.id);
  });
  // 過去の step は札だけ入れ替える（畳みと拡大率は sgRenderGraph が覚えたまま）
  if (stFrozen() && typeof sgUpdate === "function") sgUpdate(stView());
  stSyncStepper();
  stMarkGraph();
}

function stMarkGraph() {
  if (typeof sgMarkPicked === "function") sgMarkPicked(stPickedIds());
  if (typeof sgMarkSelected === "function") sgMarkSelected(stNodeId);
}

// --- step の札（図の道具の行。◀ 番号 /総数 ▶）---

/** 今の step（最新は総数）。 */
function stCurStep() { return stStep == null ? stStepTotal() : stStep; }

/** step の総数（始めていないときは 0）。 */
function stStepTotal() { return stData && stData.steps && stData.steps.total > 0 ? stData.steps.total : 0; }

/** 過去の step を見ているか（最新は false）。 */
function stFrozen() { return stStep != null && stStep < stStepTotal(); }

/** 画面に出す status（過去の step なら、その step の木）。 */
function stView() { return stFrozen() ? stStepView(stData, stStep) : stData; }

/** 図の道具の行に入れる step の札（◀ 番号 /総数 ▶）。stRenderGraph が作る。 */
function stBuildStepper() {
  const box = stEl("div", "st-stepper");
  const prev = stEl("button", "sci-graph-btn", "◀︎");
  prev.type = "button";
  prev.setAttribute("aria-label", "1 つ前の step");
  prev.addEventListener("click", () => { void stSetStep(stCurStep() - 1, false); });
  const input = stEl("input", "st-step-input");
  input.type = "text";
  input.setAttribute("inputmode", "numeric");
  input.maxLength = 6;
  input.setAttribute("aria-label", "step の番号");
  input.addEventListener("keydown", (e) => {
    if (e.key === "Enter") {
      e.preventDefault();
      const n = stStepParse(input.value, stStepTotal());
      if (n != null) void stSetStep(n, true);
      else stSyncStepper();
    } else if (e.key === "Escape") {
      stSyncStepper();
    }
  });
  input.addEventListener("blur", () => { stSyncStepper(); });
  const total = stEl("span", "st-step-total");
  const next = stEl("button", "sci-graph-btn", "▶︎");
  next.type = "button";
  next.setAttribute("aria-label", "1 つ先の step");
  next.addEventListener("click", () => { void stSetStep(stCurStep() + 1, true); });
  box.append(prev, input, total, next);
  stStepEl = box;
  return box;
}

/** 札の番号・総数・釦の使えるかを今の状態に合わせる（下書き中と step が無いときは動かせない）。 */
function stSyncStepper() {
  const box = stStepEl;
  if (!box) return;
  const total = stStepTotal();
  const cur = stCurStep();
  const drafting = stSprout !== null || stSeed !== null || stPick !== null;
  box.hidden = total === 0;
  box.querySelector(".st-step-input").value = String(cur);
  box.querySelector(".st-step-input").disabled = drafting;
  box.querySelector(".st-step-total").textContent = `/${total}`;
  box.firstElementChild.disabled = cur <= 1 || drafting;
  box.lastElementChild.disabled = stStep == null || drafting;
}

/**
 * step を動かす（null か総数以上なら最新）。center が真なら、その step の Node を図の真ん中に出す。
 * 消えた Node の選びは stLoadNode が直す（過去では読めて、最新では外れる）。
 */
async function stSetStep(next, center) {
  stStep = next == null || next >= stStepTotal() ? null : next;
  if (stNodeId) await stLoadNode();
  stSyncStepper();
  stRenderTop();
  stRenderBoard();
  stRenderSidebar(document.getElementById("sci-list"));
  if (stData && stData.initialized && typeof sgUpdate === "function") sgUpdate(stView());
  stMarkGraph();
  const entry = center && stData && stData.steps && Array.isArray(stData.steps.list) ? stData.steps.list[stCurStep() - 1] : null;
  if (entry && entry.node && typeof sgCenterOn === "function") sgCenterOn(entry.node);
}

// ============================================================================
// 右の板（.sci-board / saiSheet）
// ============================================================================

function stBoardKey() {
  return stNodeId;
}

/** 板の中身を作る（何も選んでいなければ null）。 */
function stSelBoard() {
  if (stNodeId && stNode) {
    const d = stFrozen() ? stStepNodeView(stNode, stView(), stStep) : stNode;
    if (d) return { title: `${d.node.id}`, el: stBuildBoard(d) };
    // その step には無い Node（まだ無い・消えた後）は、選びを外して板を閉じる
    stNodeId = null;
    stNode = null;
    stSetHash();
  }
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
  stNodeId = null; stNode = null;
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
 * 何も選んでいないときの一覧の上の「プロジェクト」。根の型（RESEARCH_TYPE）と mind.md を
 * status.project から出す。過去の step では作る釦を出さない。
 */
function stProjectBlock() {
  const box = stEl("section", "sci-project");
  const proj = (stView() && stView().project) || {};
  box.appendChild(stEl("h3", "sci-project-title", "プロジェクト"));
  const typeName = proj.type && proj.type.name ? String(proj.type.name) : "";
  const typeLine = stEl("p", "sci-seed-type");
  typeLine.appendChild(stEl("strong", "", "RESEARCH_TYPE: "));
  typeLine.appendChild(document.createTextNode(typeName || "未設定"));
  box.appendChild(typeLine);
  const mind = Array.isArray(proj.mind) ? proj.mind : [];
  if (!mind.length) {
    box.appendChild(stEl("p", "sci-note", "mind.md に書かれた節がありません（.sai/paper/mind.md を利用者が書きます）"));
  } else {
    for (const m of mind) {
      if (m && m.heading) box.appendChild(stEl("h4", "sci-mind-head", String(m.heading)));
      box.appendChild(stMd((m && m.body) || ""));
    }
  }
  // seed（D）は木の根。作る釦だけ置く（過去の step では出さない）
  if (!stFrozen()) {
    const row = stEl("div", "sci-ops");
    row.appendChild(stBtn("Create seed（D）", () => { stStartSeed(); }));
    box.appendChild(row);
  }
  return box;
}

/** 区分の札（固定・新規・実験・記載なし）。色だけで示さないよう、文字を必ず出す。 */
function stRoleBadge(role) {
  return stEl("span", `sci-badge sci-badge-${role}`, ST_ROLE_LABEL[role] || role);
}

/**
 * 問いの節（D と S の板の共通）。問いを箇条書きにし、回答（固定・新規・未回答）と釦を出す。
 * 釦は今の板に合うものだけ。過去の step では釦を出さない（stFrozen）。
 */
function stBoardQuestions(node) {
  const frozen = stFrozen();
  const sec = stSec(stQuestionsTitle(node));
  const rows = stQuestionRows(node);
  if (!rows.length) sec.appendChild(stEl("p", "sci-note", "なし"));
  for (const q of rows) sec.appendChild(stQuestionEl(node, q, frozen));
  if (node.kind === "D" && !frozen) {
    const ops = stEl("div", "sci-ops");
    ops.appendChild(stBtn("Add question", () => { stCall("seOpenAction", "add_seed_q", node.id); }));
    sec.appendChild(ops);
  }
  return sec;
}

/** 問い 1 つ（文・軸・Edit / Delete・回答の行・Add root path / Add Answer）。 */
function stQuestionEl(node, q, frozen) {
  const box = stEl("div", `sci-q${q.removed ? " sci-q-removed" : ""}`);
  const head = stEl("div", "sci-line sci-q-head");
  head.appendChild(stEl("span", "sci-q-text", q.text));
  if (q.axis) head.appendChild(stEl("span", "sci-axis-tag", `[${q.axis}]`));
  if (q.removed) head.appendChild(stEl("span", "sci-badge sci-badge-none", "消した問い"));
  if (q.canEdit && !frozen) {
    head.appendChild(stBtn("Edit", () => { stCall("seOpenAction", "edit_seed_q", q.id, { node: { text: q.text, axis: q.axis || "" } }); }));
    head.appendChild(stBtn("Delete", () => { stQuestionRemove(q); }));
  }
  box.appendChild(head);
  if (!q.answers.length) {
    const none = stEl("div", "sci-line sci-q-answer");
    none.appendChild(stRoleBadge("open"));
    box.appendChild(none);
  }
  for (const a of q.answers) box.appendChild(stAnswerEl(a, frozen));
  if (q.canAnswer && !frozen) {
    const ops = stEl("div", "sci-ops sci-q-ops");
    ops.appendChild(stBtn("Add root path", () => { stStartPick(node.id, q.id, q.text); }));
    ops.appendChild(stBtn("Add Answer", () => { stCall("seOpenAction", "add_answer", node.id, { q: q.id }); }));
    box.appendChild(ops);
  }
  return box;
}

/** 回答 1 つ（札・path の A と文、または文。持ち主が今の Node なら文を直せて、✕ で消せる）。 */
function stAnswerEl(a, frozen) {
  const line = stEl("div", "sci-line sci-q-answer");
  line.appendChild(stRoleBadge(a.role));
  const editable = a.mine && !frozen;
  if (a.a) {
    const b = stEl("button", "sci-path-id", a.a);
    b.type = "button";
    b.addEventListener("click", () => { void stSelect(a.a); });
    line.appendChild(b);
    line.appendChild(stEl("span", "sci-line-main", a.aText));
  } else if (editable) {
    const t = stEl("button", "sci-q-answer-text", a.text);
    t.type = "button";
    t.addEventListener("click", () => { stCall("seOpenAction", "edit_answer", a.id, { node: { text: a.text } }); });
    line.appendChild(t);
  } else {
    line.appendChild(stEl("span", "sci-line-main", a.text));
  }
  if (editable) {
    const x = stBtn("✕", () => { stAnswerRemove(a); }, "ghost");
    x.setAttribute("aria-label", `回答を消す（${a.id}）`);
    line.appendChild(x);
  }
  return line;
}

/** 問いを消す（確かめてから。回答は残る）。 */
function stQuestionRemove(q) {
  if (stWriteBlocked() || typeof seRemoveQuick !== "function") return;
  void seRemoveQuick("remove_seed_q", { q: q.id }, "問いを消しますか", "問いは板から消えます。付いている回答は残ります。");
}

/** 回答を消す（確かめてから。あとから足し直せる）。 */
function stAnswerRemove(a) {
  if (stWriteBlocked() || typeof seRemoveQuick !== "function") return;
  void seRemoveQuick("remove_answer", { answer: a.id }, "回答を消しますか", "回答は消えます。あとから足し直せます。");
}

/**
 * S（SPROUT）の実験（BRANCH）の節。未完成のあいだは、残りの問いを出す（BRANCH の釦は stOpsFor が出さない）。
 * 実験は S の子孫の E（stSproutSections が決める。板は描くだけ）。
 */
function stBoardExperiments(node) {
  const sec = stSec("実験（BRANCH）");
  const s = stSproutSections(stView(), node.id);
  if (!s) return sec;
  if (!s.complete) sec.appendChild(stEl("p", "sci-note", `未完成: 残り ${s.missing.length} 問（${s.missing.join("・")}）`));
  const byId = new Map(((stView() && stView().nodes) || []).map((n) => [String(n.id), n]));
  for (const e of s.exps) {
    const n = byId.get(e.id);
    if (!n) continue;
    const row = stRowEl(stRows({ nodes: [n] })[0], { flat: true });
    row.insertBefore(stRoleBadge("exp"), row.firstChild);
    sec.appendChild(row);
  }
  if (!s.exps.length) sec.appendChild(stEl("p", "sci-note", "なし"));
  return sec;
}

/**
 * Node の板。Q・A・E は「見出し（Edit/Delete を含む）→ Parent → OP → [Q だけ: ガイド] → Child → Next Action → History → Decision」。
 * S（SPROUT）は Parent の代わりに「OP → 範囲 → 問い → 実験（BRANCH）」を出す。
 * D（seed）は Parent も無い。本文（見出し）→ OP（SPROUT・CARD・Edit・Delete）→ 問い → 子の SPROUT（比べる）。
 * F は「見出し（Edit/Delete を含む）→ Parent → OP → Stance → 出典 → History」。E は MLflow の節を OP の後に出す。
 */
function stBuildBoard(d) {
  const node = d.node || {};
  const box = stEl("div", "sci-card");
  box.appendChild(stBoardHead(d));
  if (node.kind !== "S" && node.kind !== "D") box.appendChild(stBoardPath(d));
  if (!stFrozen()) box.appendChild(stBoardOps(d)); // 過去の step では OP の節を出さない
  if (node.kind === "F") {
    box.appendChild(stBoardStance(node));
    box.appendChild(stBoardSources(d));
    box.appendChild(stBoardHistory(d));
    return box;
  }
  if (node.kind === "E") {
    box.appendChild(stBoardMlflow(node));
    box.appendChild(stBoardDataset(node));
  }
  if (node.kind === "Q") box.appendChild(stBoardGuide(d.guide));
  if (node.kind === "S") {
    box.appendChild(stBoardRange(node));
    box.appendChild(stBoardQuestions(node));
    box.appendChild(stBoardExperiments(node));
  } else if (node.kind === "D") {
    box.appendChild(stBoardQuestions(node));
    box.appendChild(stBoardSprouts(node.id));
  } else {
    box.appendChild(stBoardChildren(d.children));
  }
  const next = stSec("Next Action");
  const nl = stNextLines({ next: d.next }, 3);
  if (nl.length) next.appendChild(stNextRows(nl, stData));
  else next.appendChild(stEl("p", "sci-note", "なし"));
  box.appendChild(next);
  box.appendChild(stBoardHistory(d));
  const decisions = stBoardDecisions(d);
  if (decisions) box.appendChild(decisions);
  return box;
}

/** 操作の表示名。seOpLabel が読めなければ op をそのまま。 */
function stOpLabel(op) {
  const label = (typeof seOpLabel === "function") ? seOpLabel(op) : null;
  if (label) return label;
  // 旧形式の op（add_q・insert_q・move_a・regroup）は履歴と過去の step に出るので、ここで名前を持つ
  const fall = { add_q: "Add Q", add_a: "Add A", add_f: "Add F", add_e: "Add E", copy: "Copy A", edit: "Edit", remove: "Delete", insert_q: "Q を間に入れる", move_a: "A を移す", regroup: "Regroup", set_mlflow: "MLflow を変える" };
  return fall[op] || op;
}

/** 見出し（★・ID・種類・stance・文・本文）。文と本文の後に Edit / Delete（S 以外）。 */
function stBoardHead(d) {
  const node = (d && d.node) || {};
  const sec = stEl("section", "sci-sec sci-node-head");
  const line = stEl("div", "sci-line");
  // 過去の step では ★ は今の印を文字で出すだけ（押せない）
  if (stFrozen()) {
    line.appendChild(stEl("span", `sci-head-star${node.starred ? " on" : ""}`, node.starred ? "★" : "☆"));
  } else {
    const star = stEl("button", `sci-head-star${node.starred ? " on" : ""}`, node.starred ? "★" : "☆");
    star.type = "button";
    star.setAttribute("aria-pressed", String(!!node.starred));
    star.setAttribute("aria-label", node.starred ? "★ を外す" : "★ を付ける");
    star.addEventListener("click", () => { stCall("seQuick", node.starred ? "unstar" : "star", { node: node.id }); });
    line.appendChild(star);
  }
  line.appendChild(stEl("span", "sci-id", `${node.id} ${node.kind}`));
  // 条件の A（種類の分割でない子）は用語の札で示す。subtype は印を出さない
  if (node.kind === "A" && node.rel === "condition") line.appendChild(saiTerm("sci_rel", "条件"));
  if (node.kind === "F") {
    const badge = stStanceBadge(node.stance);
    if (badge) line.appendChild(badge);
  }
  sec.appendChild(line);
  sec.appendChild(stEl("p", "sci-node-title", node.text || ""));
  if (node.body) sec.appendChild(stMd(node.body));
  // D（seed）は OP の節に Edit / Delete を出すのでここでは出さない。過去の step では出さない
  if (node.kind !== "S" && node.kind !== "D" && !stFrozen()) {
    const editRow = stEl("div", "sci-node-edit");
    editRow.appendChild(stBtn(stOpLabel("edit"), () => { stCall("seOpenAction", "edit", node.id, d); }));
    if (node.parent) editRow.appendChild(stBtn(stOpLabel("remove"), () => { stCall("seOpenAction", "remove", node.id, d); }));
    sec.appendChild(editRow);
  }
  return sec;
}

/** F の stance（S・D・I の 3 釦。押すと set_stance）。押した釦が今の stance。 */
function stBoardStance(node) {
  const sec = stSec("Stance");
  if (stFrozen()) {
    // 過去の step では釦を出さず、今の stance を文字で出すだけ
    const hit = stStanceLabel(node.stance);
    sec.appendChild(stEl("p", "sci-note", hit ? `${hit[0]} ${hit[1]}` : "なし"));
    return sec;
  }
  const row = stEl("div", "sci-ops sci-stance-row");
  for (const [value, code, name] of ST_STANCE) {
    const on = node.stance === value;
    const b = stBtn(`${code} ${name}`, () => { stCall("seQuick", "set_stance", { node: node.id, stance: value }); }, on ? "primary" : "");
    b.setAttribute("aria-pressed", String(on));
    row.appendChild(b);
  }
  sec.appendChild(row);
  return sec;
}

/** E の MLflow の節（参照があれば Experiments へ飛ぶ釦。無ければ「なし」）。 */
function stBoardMlflow(node) {
  const sec = stSec("MLflow");
  if (node.mlflow) sec.appendChild(stMlflowBtn(node.mlflow));
  else sec.appendChild(stEl("p", "sci-note", "なし"));
  return sec;
}

/** E の dataset の見え方（空は null）。`ts:<名前>@<sha>` なら sai-ts のスナップショットの名前を取り出す（G3-C1）。 */
function stDatasetParts(value) {
  const text = value == null ? "" : String(value).trim();
  if (!text) return null;
  const m = /^ts:([^@\s]+)@/.exec(text);
  return { text, snapshot: m ? m[1] : null };
}

/** E の Dataset の節（dataset があれば等幅で出して写す釦を付ける。無ければ「なし」）。 */
function stBoardDataset(node) {
  const sec = stSec("Dataset");
  const parts = stDatasetParts(node && node.dataset);
  if (!parts) { sec.appendChild(stEl("p", "sci-note", "なし")); return sec; }
  const row = stEl("div", "sci-line");
  const text = stEl("code", "sci-dataset", parts.text);
  text.style.overflowWrap = "anywhere"; // 長い ts:… も 390px の幅に収める
  row.appendChild(text);
  row.appendChild(stBtn("写す", () => {
    if (!navigator.clipboard || !navigator.clipboard.writeText) { stToast("写せませんでした", "error"); return; }
    navigator.clipboard.writeText(parts.text).then(
      () => stToast("Dataset を写しました"),
      () => stToast("写せませんでした", "error"),
    );
  }));
  sec.appendChild(row);
  if (parts.snapshot) {
    sec.appendChild(stEl("p", "sci-note", `sai-ts のデータセット（${parts.snapshot}）。中身は sai-ts の『データセット』タブか \`sai-ts snapshot show ${parts.snapshot}\``));
  }
  return sec;
}

/** D（seed）の子の SPROUT（親が D の S）。押すと、その SPROUT の板を開く。その D の S を全部比べる釦も出す。 */
function stBoardSprouts(seedId) {
  const sec = stSec("SPROUT");
  const kids = stSeedChildren(stView(), seedId);
  const cmp = stBtn("比べる", () => { stGo(stCompareHash(kids.map((n) => n.id))); });
  cmp.disabled = kids.length < 2;
  if (kids.length < 2) cmp.title = "比べるには S が 2 つ以上要ります";
  sec.appendChild(cmp);
  const rows = stRows({ nodes: kids });
  if (!rows.length) sec.appendChild(stEl("p", "sci-note", "なし"));
  for (const r of rows) {
    // 完成か、未完成の残りの数を札で出す（文字で示す）
    const row = stRowEl(r, { flat: true });
    const state = stSproutStateText(r.node);
    const complete = r.node.complete === true;
    row.insertBefore(stEl("span", `sci-badge sci-badge-${complete ? "complete" : "open"}`, state), row.querySelector(".sci-star"));
    sec.appendChild(row);
  }
  return sec;
}

/** S（SPROUT）の範囲の節（作成のときと同じ数を出す）。範囲が無ければ「なし」。 */
function stBoardRange(node) {
  const sec = stSec("範囲");
  const text = stRangeText(node.range);
  sec.appendChild(stEl("p", "sci-note", text || "なし"));
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
  for (const n of (stView() && stView().nodes) || []) {
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
  // Add root path の最中は、A の板にこの釦を出す（押すと、選んだ問いの回答になる）
  if (stPick !== null && node.kind === "A") {
    ops.appendChild(stBtn(`この A で回答（${stPick.q}）`, () => { void stAnswerPicked(node.id); }, "primary"));
  }
  // D（seed）から SPROUT を作る（seed を入れた状態で開く）。CARD は SPROUT を作るブロックから始める
  if (node.kind === "D") ops.appendChild(stBtn("Create SPROUT（この seed で）", () => { stStartSprout(node.id); }, "primary"));
  for (const op of stOpsFor(node)) {
    ops.appendChild(stBtn(stOpLabel(op), () => { stCall("seOpenAction", op, node.id, d); }));
  }
  // S は比べるに足す／外す（選んだ S は sessionStorage に覚え、Tree の上の「比べる（n）」で並べる）
  if (node.kind === "S") {
    const on = stCompareList().includes(node.id);
    ops.appendChild(stBtn(on ? "比べるから外す" : "比べるに足す", () => { stCompareFlip(node.id); }, on ? "" : "primary"));
  }
  if (node.kind === "D") ops.appendChild(stBtn("Create CARD", () => { stCall("seOpenCard", node.id, { block: "lab-sprout" }); }));
  else ops.appendChild(stBtn("Card", () => { stCall("seOpenCard", node.id, null); }));
  sec.appendChild(ops);
  return sec;
}

/** ガイド（question・細分の候補（child_q）・Extract・Pitfall）。 */
function stBoardGuide(g) {
  const sec = stSec("GUIDE");
  if (!g) { sec.appendChild(stEl("p", "sci-note", "なし")); return sec; }
  const items = [["question", g.question], ["細分の候補", g.child_q], ["Extract", g.extract], ["Pitfall", g.pitfall]];
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

/** History。d.history の順のまま 1 行ずつ出す（並べ替えない）。 */
function stBoardHistory(d) {
  const sec = stSec("History");
  const list = d && Array.isArray(d.history) ? d.history : [];
  if (!list.length) { sec.appendChild(stEl("p", "sci-note", "なし")); return sec; }
  for (const h of list) sec.appendChild(stHistoryRowEl(h, ""));
  return sec;
}

/** Decision（許可を受けた決定）の節。0 件なら null（節を出さない）。 */
function stBoardDecisions(d) {
  const list = d && Array.isArray(d.decisions) ? d.decisions : [];
  if (!list.length) return null;
  const sec = stSec("Decision");
  for (const h of list) sec.appendChild(stHistoryRowEl(h, ""));
  return sec;
}

/** 質問文を小窓で出す（問いに答えた History・Decision の行から）。 */
function stShowQuestion(h) {
  if (typeof saiSheet !== "function") return;
  const body = stEl("div", "sci-sheet-body");
  const meta = [stHistoryText(h), stApprovalText(h)].filter(Boolean).join(" ");
  if (meta) body.appendChild(stEl("p", "sci-note", meta));
  body.appendChild(stMd(String(h.question || "")));
  saiSheet({ title: h.qid ? `問い ${h.qid}` : "問い", body });
}

/**
 * History・Decision の 1 行。問い（question）があれば押すと質問文を小窓で出す。
 * 問いが無く run_id がある行は押すとその run へ。許可した人（approved_by）は `許可: <名前>` として文の後に足す。
 * prefix は文の先頭に付ける（Node の ID など）。
 */
function stHistoryRowEl(h, prefix) {
  const approved = stApprovalText(h);
  const text = `${prefix || ""}${stHistoryText(h)}${approved ? ` ${approved}` : ""}`;
  if (h && h.question) {
    const q = stEl("button", "sci-history-row", text);
    q.type = "button";
    q.addEventListener("click", () => stShowQuestion(h));
    return q;
  }
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

// ============================================================================
// 失敗・プロジェクト無しの表示
// ============================================================================

function stClearAll() {
  stCloseSheet();
  stSprout = null;
  stPick = null;
  stSeed = null;
  for (const id of ["sci-graph", "sci-top", "sci-sprout", "sci-seed", "sci-list"]) {
    const e = document.getElementById(id);
    if (e) { e.innerHTML = ""; if (id === "sci-sprout" || id === "sci-seed") e.hidden = true; }
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
// 比べる画面（#/research/compare?s=S1,S2。#view-sci-tree の中に作る入れ物に出す）
// ============================================================================

let stCmpEl = null; // 比べる画面の入れ物（#view-sci-tree の中。初めて開くときに作る）
let stCmpSeq = 0;   // 読み込みの番号（古い応答で新しい表を消さない）

/** 比べるに足した S の並び（プロジェクトごとに sessionStorage へ覚える）。 */
function stCompareKey() {
  return `sai-sci-compare:${selectedPaperProject() || ""}`;
}

function stCompareList() {
  try {
    const v = JSON.parse(sessionStorage.getItem(stCompareKey()) || "null");
    return Array.isArray(v) ? v.map(String).filter((id) => /^S[1-9][0-9]{0,8}$/.test(id)) : [];
  } catch { return []; }
}

function stCompareSave(list) {
  try { sessionStorage.setItem(stCompareKey(), JSON.stringify(list)); } catch { /* 保存できなくてもよい */ }
}

/** 板の「比べるに足す／外す」。上限を超えるときは足さずに言う。 */
function stCompareFlip(id) {
  const before = stCompareList();
  const next = stCompareToggle(before, id);
  if (next.length === before.length && !before.includes(id)) {
    stToast(`比べるは ${ST_COMPARE_MAX} 個までです`, "error");
    return;
  }
  stCompareSave(next);
  stRenderTop();
  stRenderBoard();
}

/** 比べる画面の住所。 */
function stCompareHash(ids) {
  return `#/research/compare?s=${ids.join(",")}`;
}

/** 比べる画面から S を外す（×）。残りが無ければ Tree へ戻る。 */
function stCompareDrop(ids, id) {
  stCompareSave(stCompareList().filter((x) => x !== id));
  const next = ids.filter((x) => x !== id);
  stGo(next.length ? stCompareHash(next) : "#/research/tree");
}

/** 比べる画面の入れ物を出す。Tree の部品は隠す（戻るときは stLeaveCompare が出し直す）。 */
function stCompareBox() {
  const view = document.getElementById("view-sci-tree");
  if (!view) return null;
  if (!stCmpEl) {
    stCmpEl = stEl("section", "sci-compare");
    view.appendChild(stCmpEl);
  }
  for (const id of ["sci-top", "sci-sprout", "sci-seed", "sci-body"]) {
    const e = document.getElementById(id);
    if (e) e.hidden = true;
  }
  stCmpEl.hidden = false;
  return stCmpEl;
}

/** 比べる画面から Tree へ戻すときに、Tree の部品を出し直す（入れ物は隠す）。 */
function stLeaveCompare() {
  if (stCmpEl) stCmpEl.hidden = true;
  const show = { "sci-top": true, "sci-body": true, "sci-sprout": stSprout !== null || stPick !== null, "sci-seed": stSeed !== null };
  for (const id of Object.keys(show)) {
    const e = document.getElementById(id);
    if (e) e.hidden = !show[id];
  }
}

/** 比べる画面の上の帯（← Tree と、選んだ S の札・外す ×）。 */
function stCompareBar(ids) {
  const bar = stEl("div", "sci-compare-bar");
  bar.appendChild(stBtn("← Tree", () => { stGo("#/research/tree"); }, "ghost"));
  bar.appendChild(stEl("h2", "sci-compare-title", `比べる（${ids.length}）`));
  const chips = stEl("div", "sci-compare-chips");
  for (const id of ids) {
    const b = stEl("button", "sci-compare-chip", `${id} ×`);
    b.type = "button";
    b.setAttribute("aria-label", `${id} を比べるから外す`);
    b.addEventListener("click", () => { stCompareDrop(ids, id); });
    chips.appendChild(b);
  }
  bar.appendChild(chips);
  return bar;
}

/** 比べる表（行が S、列が 6 軸）。列の見出しに「違う」の札、セルに区分の札と同じ A の印。 */
function stCompareTable(t) {
  const wrap = stEl("div", "sci-compare-wrap");
  const table = stEl("table", "sci-compare-table");
  const head = stEl("tr");
  head.appendChild(stEl("th", "sci-compare-s", "S"));
  for (const ax of t.axes) {
    const th = stEl("th", ax.diff ? "sci-compare-diff" : "");
    th.appendChild(stEl("span", "", ax.label));
    if (ax.diff) th.appendChild(stEl("span", "sci-badge sci-badge-diff", "違う"));
    head.appendChild(th);
  }
  table.appendChild(stEl("thead")).appendChild(head);
  const tbody = table.appendChild(stEl("tbody"));
  for (const r of t.rows) {
    const tr = stEl("tr");
    const sh = stEl("th", "sci-compare-s");
    const b = stEl("button", "sci-path-id", r.id);
    b.type = "button";
    b.addEventListener("click", () => { stGo(`#/research/tree/${r.id}`); });
    sh.appendChild(b);
    if (r.title) sh.appendChild(stEl("span", "sci-compare-stitle", stShort(r.title, 30)));
    tr.appendChild(sh);
    for (const c of r.cells) {
      const td = stEl("td", `sci-cmp-${c.role}`);
      if (c.text) td.title = c.text;
      const top = stEl("div", "sci-line");
      top.appendChild(stRoleBadge(c.role));
      if (c.mark) top.appendChild(stEl("span", "sci-cmp-mark", c.mark));
      td.appendChild(top);
      if (c.a) {
        const ab = stEl("button", "sci-path-id", c.a);
        ab.type = "button";
        ab.addEventListener("click", () => { stGo(`#/research/tree/${c.a}`); });
        td.appendChild(ab);
      }
      if (c.text) td.appendChild(stEl("p", "sci-cmp-text", stShort(c.text, 60)));
      tr.appendChild(td);
    }
    tbody.appendChild(tr);
  }
  wrap.appendChild(table);
  return wrap;
}

/** 比べる画面を開く（SaiRoutes の open から）。結果は sai-sci compare の C-CMP-1 を描く。 */
async function stCompareOpen(params) {
  const box = stCompareBox();
  if (!box) return;
  const seq = ++stCmpSeq;
  const ids = (params && params.ids) || [];
  await ensurePaperProjects();
  box.innerHTML = "";
  box.appendChild(stCompareBar(ids));
  if (!selectedPaperProject()) {
    box.appendChild(stEl("p", "sci-note", "論文の置き場がありません"));
    return;
  }
  if (ids.length < 2) {
    box.appendChild(stEl("p", "sci-note", "比べるには S を 2 つ以上選んでください（Tree の板の「比べるに足す」）"));
    return;
  }
  const body = stEl("div", "sci-compare-body", "読んでいます…");
  box.appendChild(body);
  let json = null;
  try {
    json = await paperApi(stApi(`/compare?s=${ids.join(",")}`));
  } catch (e) {
    if (seq !== stCmpSeq) return;
    body.innerHTML = "";
    body.appendChild(stEmpty({ icon: "⚠️", title: "比べられませんでした", body: (e && e.message) || "読み直してください" }));
    return;
  }
  if (seq !== stCmpSeq) return; // 後から開いた比べるを優先する
  body.innerHTML = "";
  body.appendChild(stCompareTable(stCompareRows(json)));
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

  // 比べる画面（#/research/compare?s=S1,S2。入れ物は #view-sci-tree の中）
  window.SaiRoutes.push({
    match(hash) { return stParseCompareHash(hash); },
    view: "view-sci-tree",
    tab: "research",
    hideTabbar: false,
    interval: 0,
    async open(params) { await stCompareOpen(params); },
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
    stParseHash, stParseCompareHash, stLegacyHash, stRows, stNextLines, stNextText,
    stOpsFor, stRangeText, stApprovalText, stStanceLabel,
    stSeedNodes, stSeedChildren, stSeedOf, stSeedDirty,
    stSproutSections, stQuestionRows, stQuestionsTitle, stSproutStateText,
    stCompareRows, stCompareToggle, stShort, stCircled,
    stVisibleRows, stRowHit, stActionMeta, stClampSplit, stFmtAt, stHistoryText,
    stStepView, stStepNodeView, stStepParse, stDatasetParts,
    ST_TYPES, ST_STANCE, ST_NODE_OPS, ST_ALL_KINDS, ST_SPLIT_LIMITS, ST_EXPERIMENTS_HASH,
    ST_AXIS_LABEL, ST_ROLE_LABEL, ST_COMPARE_MAX,
  };
}
