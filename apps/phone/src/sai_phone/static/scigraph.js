"use strict";
// 研究方針の木の「根の図」と「候補の表」（実装計画 U21・共有契約 C-14 / C-08）。
//
//   sgRenderGraph(container, status, onSelect)  … 根の図（DAG）。onSelect(nodeId)
//   sgRenderTable(container, detail)            … 候補の表（選ぶ・並べる項目）
//   sgRunsLayout(dag)                           … 実行の図の配置（純関数）
//   sgRenderRuns(container, dag, metrics, onSelect) … 実行の図。onSelect(runId)
//
// 実行の図（U08）は research-tree の root / seed / branch の 3 層の run の DAG である。
// 入力は共有契約 C-02（`GET /api/projects/{name}/rtree`）の {nodes, edges}、
// 指標は画面が MLflow から取った {run_id: "val_auc 0.871"} の辞書（C-03 / C-04）。
// 中の処理は Rust のコアと MLflow が返した値だけを使う（apps/phone は他の DB を開かない）。
//
// 中の処理は Rust のコアが返した status と node の JSON だけを使う（apps/phone は
// 他の DB を開かない）。このファイルは新しい情報を足さない。
//
// 書き方:
//   - 配置は canvaslayout.js の cvlLayoutBoard を使い回す。親子の線と候補の並びは
//     status の並びで決まるので、同じ status なら開くたびに図が動かない。
//   - 配置に使う線は親子と依存だけ。由来と同型まで入れると層が崩れて読みにくい。
//     描くのは 4 種類とも描く。
//   - 札の大きさは固定（幅 200・高さ 72）。題は 2 行で切って、全部は板で見せる。
//   - 見える範囲（見える場所 tx, ty, 拡大率 z）だけを動かし、札の位置は動かさない。
//     全体表示と拡大は canvaslayout.js の cvFitView / cvZoomAt を typeof で確かめて使う
//     （その関数が無いときは拡大率 1 で左上を起点にする）。
//   - 却下・休眠の候補は消さず薄く出す（不透明度 0.45・点線の枠）。
//   - 依存は足さない。札は HTML の div、線は同じ中の SVG（workflow.js と同じ作り）。
//
// 一番外側の名前は sg で始める（C-14 の test_globals.js が重なりを検査する）。

// --- 定数 ---

// 札の大きさ（世界上の px）
const SG_CARD_W = 200;
const SG_CARD_H = 72;
// この数を超える節点では、候補を隠した状態で始める（見やすさのため）
const SG_MANY_NODES = 60;
// ドラッグとクリックの境（これ以上動いたらドラッグ、未満ならクリック）
const SG_DRAG_SLOP = 4;
// 拡大・縮小の釦を 1 回押したときの拡大率の倍率
const SG_ZOOM_STEP = 1.2;

// 線の種類（C-01）。sgEdgeStyle がこの表を返す。
// [種類, 表示名, 破線, 矢印つきの実線か, 色の CSS 変数]
const SG_EDGE_KINDS = [
  ["parent", "PARENT", "", false, "--sci-edge-parent"],
  ["depends", "DEPENDS", "", true, "--sci-edge-depends"],
  ["derived", "DERIVED", "8 5", false, "--sci-edge-derived"],
  ["analogous", "ANALOGOUS", "2 4", false, "--sci-edge-analogous"],
];

// 節点の状態（C-01）。札の印と枠の色（CSS 変数）に使う。
const SG_ITEM_STATES = {
  open: { mark: "○", label: "OPEN", color: "var(--sci-open)" },
  tentative: { mark: "◐", label: "TENTATIVE", color: "var(--sci-tentative)" },
  decided: { mark: "●", label: "DECIDED", color: "var(--sci-decided)" },
  excluded: { mark: "×", label: "EXCLUDED", color: "var(--sci-excluded)" },
};
// 候補の状態（C-01）
const SG_OPTION_STATES = {
  candidate: { mark: "◇", label: "CANDIDATE", color: "var(--sci-candidate)" },
  selected: { mark: "◆", label: "SELECTED", color: "var(--sci-decided)" },
  rejected: { mark: "✕", label: "REJECTED", color: "var(--sci-rejected)" },
  dormant: { mark: "◌", label: "DORMANT", color: "var(--sci-dormant)" },
};
// 表の並び（選択 → 候補 → 休眠 → 却下）。同じ状態は元の並びのまま
const SG_OPTION_ORDER = { selected: 0, candidate: 1, dormant: 2, rejected: 3 };
// 空欄の種類（C-01）
const SG_BLANK_LABELS = {
  untouched: "UNTOUCHED",
  in_progress: "IN_PROGRESS",
  insufficient: "INSUFFICIENT",
  awaiting_decision: "AWAITING_DECISION",
  conflict: "CONFLICT",
};
// 印（C-01）
const SG_MARK_LABELS = { weak_evidence: "WEAK_EVIDENCE", recheck: "RECHECK" };
// 表を出す項目の型（選ぶ・並べる。C-01）
const SG_TABLE_TYPES = ["choose", "list"];

// --- 実行の図（root / seed / branch の 3 層。U08）---

// 層の帯（上から）。帯の左に書く語。C-02 の layer と 1 対 1
const SG_RUN_LAYERS = [
  ["root", "方針（root）"],
  ["seed", "計画（seed）"],
  ["branch", "実行（branch）"],
];
// C-02 に無い層のための帯（.Root/seed/branch のあとろに足す）
const SG_RUN_LAYER_OTHER = ["other", "その他"];
// 配置に使う線（delegation と waggle_token だけ。task_output は輪になるので使わない）
const SG_RUN_LAYOUT_KINDS = ["delegation", "waggle_token"];
// 描く線。delegation は実線、waggle_token は点線、task_output は破線（C-02 / 要件 R-L2）
const SG_RUN_EDGE_KINDS = [
  ["delegation", "委譲", "", false, "--sci-edge-depends"],
  ["waggle_token", "結び", "2 4", false, "--sci-edge-parent"],
  ["task_output", "返却", "8 5", false, "--sci-edge-analogous"],
];
// run の状態の印と色（根の図の札と同じ CSS 変数を使う）
const SG_RUN_STATES = {
  running: { mark: "▶", label: "実行中", color: "var(--sci-tentative)" },
  completed: { mark: "✓", label: "完了", color: "var(--sci-decided)" },
  failed: { mark: "✗", label: "失敗", color: "var(--sci-rejected)" },
  cancelled: { mark: "✕", label: "中止", color: "var(--sci-excluded)" },
  pending: { mark: "…", label: "待ち", color: "var(--sci-open)" },
};
// 列と行の空き、帯の房地と余り、帯の左の語の幅
const SG_RUN_GAP_X = 44;
const SG_RUN_GAP_Y = 22;
const SG_BAND_PAD = 12;
const SG_BAND_GAP = 16;
const SG_BAND_LABEL_W = 96;

// --- 状態 ---

let sgStatus = null;   // 最後に描いた status の JSON
let sgOnSelect = null; // 札を押したときに呼ぶ関数
let sgRoot = null;     // 図を囲む要素
let sgOptBtn = null;   // 候補の出し入れの釦（ラベルを sgDraw で更新する）
let sgStage = null;    // 見える範囲を出す所
let sgWorld = null;    // その中の世界（transform で動かす）
let sgView = { tx: 0, ty: 0, z: 1 }; // 画面上の点 = 世界の点 * z + (tx, ty)
let sgShowOptions = true; // 候補の札を出すか
let sgRects = new Map();  // 節点 id → 置いた場所（線と札の共通）
let sgExtraRects = [];    // 見える範囲の算出に入れる四角（実行の図の帯。線と札には使わない）
let sgPtrs = new Map();  // 押している指（1 本 = 移動、2 本 = 拡大）
let sgDown = null;       // 1 本指で押したままの情報
let sgPinch = null;      // 2 本指の情報
let sgFitted = false;    // 全体表示を 1 回はしたか
let sgRunDag = null;     // 実行の図の DAG（C-02）
let sgRunMetrics = {};   // 実行の図の指標（{run_id: "val_auc 0.871"}）
let sgShowReturn = false;// 返却の線（task_output）を出すか。既定は隠す

// ============================================================================
// DOM を使わない部品（node のテストから読む）
// ============================================================================

/**
 * 図に並べる札と線を作る。
 * @param status C-08 の status の JSON
 * @param showOptions 候補の札を出すか
 * @returns {cards, layoutEdges, drawEdges}
 *   cards … cvlLayoutBoard に渡す札（{id, type, x, y, width, height, group} と節点の原文）
 *   layoutEdges … 配置に使う線（親子の線と依存）
 *   drawEdges … 描く線（4 種類とも）
 */
function sgGraphInput(status, showOptions) {
  const nodes = status && Array.isArray(status.nodes) ? status.nodes.filter((n) => n && n.id) : [];
  const show = showOptions !== false;
  const cards = [];
  for (const n of nodes) {
    // 候補を隠すときは項目だけを出す（元の並びは保つ）
    if (!show && n.kind === "option") continue;
    cards.push({
      id: n.id,
      type: "card",
      x: 0,
      y: 0,
      width: SG_CARD_W,
      height: SG_CARD_H,
      group: null,
      node: n,
    });
  }
  const ids = new Set(cards.map((c) => c.id));

  // 線（親子の線は nodes[].parents から作る）
  const all = [];
  for (const c of cards) {
    const parents = Array.isArray(c.node.parents) ? c.node.parents : [];
    for (const p of parents) {
      if (ids.has(p)) all.push({ kind: "parent", from: p, to: c.id });
    }
  }
  const edges = status && Array.isArray(status.edges) ? status.edges : [];
  for (const e of edges) {
    if (!e || !ids.has(e.from) || !ids.has(e.to)) continue;
    all.push({ kind: String(e.kind || ""), from: e.from, to: e.to });
  }
  return {
    cards,
    layoutEdges: all.filter((e) => e.kind === "parent" || e.kind === "depends"),
    drawEdges: all,
  };
}

/**
 * 線の種類 → 描き方。
 * 表に無い種類（null や未知の値）も止まらないように、灰色の実線を返す。
 * @returns {dash, arrow, colorVar} dash は ""（実線）か "8 5" のような破線
 */
function sgEdgeStyle(kind) {
  const key = kind == null ? "" : String(kind);
  const row = SG_EDGE_KINDS.find((k) => k[0] === key);
  if (row) return { dash: row[2], arrow: row[3], colorVar: `var(${row[4]})` };
  return { dash: "", arrow: false, colorVar: "var(--sci-edge-parent)" };
}

/**
 * 今の関門の条件に書いてある項目の key を集める。
 * `decided(purpose.subgoal) かつ count(x, 3)` → ["purpose.subgoal", "x"]
 * @returns 文字列の配列（重複なし・並んだ順）
 */
function sgGateKeys(status) {
  const gate = status ? status.gate : null;
  if (!gate || !Array.isArray(gate.list)) return [];
  const list = gate.list.filter((g) => g && typeof g.when === "string");
  if (!list.length) return [];
  // 今の関門は、通っていない最初のもの（gate.current は 1 始まり）
  let cur = list.find((g) => g.passed === false);
  if (!cur) {
    const i = Math.max(0, (Number(gate.current) || 1) - 1);
    cur = list[Math.min(i, list.length - 1)];
  }
  if (!cur) return [];

  // 括弧の中の識別子を key とみなす（数と関数名は入れない）
  const out = [];
  const push = (s) => {
    const key = String(s || "").trim();
    if (!/^[A-Za-z_][A-Za-z0-9_]*(\.[A-Za-z0-9_]+)*$/.test(key)) return;
    if (out.includes(key)) return;
    out.push(key);
  };
  const re = /\(([^()]*)\)/g;
  let m;
  while ((m = re.exec(cur.when)) !== null) {
    for (const arg of String(m[1]).split(",")) push(arg);
  }
  return out;
}

/**
 * 候補の表の行を作る（並びも含む）。
 * @param detail C-08 の node の JSON
 * @returns 配列。各要素 {id, title, state, mark, label, support, refute, papers, summary}
 */
function sgTableRows(detail) {
  const d = detail || {};
  const options = Array.isArray(d.options) ? d.options.filter((o) => o && o.id) : [];
  // 候補の知見（要約の 1 文を出すのに使う）
  const findings = Array.isArray(d.findings) ? d.findings : [];
  const rows = options.map((o, i) => {
    const st = SG_OPTION_STATES[o.state] || SG_OPTION_STATES.candidate;
    return {
      id: o.id,
      title: o.title || "",
      state: String(o.state || "candidate"),
      mark: st.mark,
      label: st.label,
      support: Math.max(0, Number(o.support) || 0),
      refute: Math.max(0, Number(o.refute) || 0),
      papers: Math.max(0, Number(o.papers) || 0),
      summary: sgFirstSentence(sgFirstSupport(findings, o.id)),
      ord: i,
    };
  });
  // 選択 → 候補 → 休眠 → 却下。同じ状態は元の並びのまま
  rows.sort((a, b) => (SG_OPTION_ORDER[a.state] ?? 9) - (SG_OPTION_ORDER[b.state] ?? 9) || a.ord - b.ord);
  return rows;
}

/** その候補の支持の知見の中で、いちばん上の 1 つ。 */
function sgFirstSupport(findings, optionId) {
  const list = (Array.isArray(findings) ? findings : []).filter(
    (f) => f && f.node === optionId && f.polarity === "support"
  );
  return list.length ? String(list[0].text || "") : "";
}

/** 1 番目の文だけを取り出す（句点で切る。句点が無ければ全体）。 */
function sgFirstSentence(text) {
  const s = String(text || "").trim();
  if (!s) return "";
  const i = s.indexOf("。");
  return i < 0 ? s : s.slice(0, i + 1);
}

/** 節点の状態から札の枠と印を出す（表に無い状態は項目として扱う）。 */
function sgStateOf(node) {
  const n = node || {};
  if (n.kind === "option") {
    const st = SG_OPTION_STATES[n.state] || SG_OPTION_STATES.candidate;
    return { mark: st.mark, label: st.label, color: st.color, key: String(n.state || "candidate") };
  }
  const st = SG_ITEM_STATES[n.state] || SG_ITEM_STATES.open;
  return { mark: st.mark, label: st.label, color: st.color, key: String(n.state || "open") };
}

// ============================================================================
// 実行の図（root / seed / branch の 3 層・U08）
// ============================================================================

/** C-02 の layer を帯の層に直す。知らない値は「その他」の帯にする。 */
function sgRunLayer(layer) {
  const key = String(layer == null ? "" : layer);
  return SG_RUN_LAYERS.some(([id]) => id === key) ? key : SG_RUN_LAYER_OTHER[0];
}

/** 帯の左に書く語（「方針（root）」など）。知らない層は「その他」。 */
function sgRunLayerLabel(layer) {
  const key = sgRunLayer(layer);
  const row = SG_RUN_LAYERS.find(([id]) => id === key);
  return row ? row[1] : SG_RUN_LAYER_OTHER[1];
}

/** 実行の状態の印と色（表に無い状態は「待ち」）。 */
function sgRunStateOf(status) {
  const st = SG_RUN_STATES[String(status || "")] || SG_RUN_STATES.pending;
  return { mark: st.mark, label: st.label, color: st.color, key: String(status || "pending") };
}

/** 実行の図の札の 2 行目に出す時刻（`2026-09-30 01:02:03` → `09-30 01:02`）。 */
function sgRunTimeLabel(text) {
  const s = String(text || "").trim();
  const m = /^(\d{4})-(\d{2})-(\d{2})[T ](\d{2}):(\d{2})/.exec(s);
  if (!m) return s ? s.slice(0, 16) : "";
  return `${m[2]}-${m[3]} ${m[4]}:${m[5]}`;
}

/** 実行の図の線の描き方。表に無い種類でも止まらない。 */
function sgRunEdgeStyle(kind) {
  const key = kind == null ? "" : String(kind);
  const row = SG_RUN_EDGE_KINDS.find((k) => k[0] === key);
  if (row) return { dash: row[2], arrow: row[3], colorVar: `var(${row[4]})` };
  return { dash: "", arrow: false, colorVar: "var(--sci-edge-parent)" };
}

/**
 * 実行の図の配置（純関数。DOM を使わない）。
 *
 * - 列は、その run までの**最長の道**の長さ（深さ）。配置に使うのは
 *   `delegation` と `waggle_token` だけ（`task_output` は同じ 2 つを逆向きに
 *   結ぶので使わない）。輪があっても止まらないよう、訪問済みの印で切る。
 * - 行は、その層の帯の中の同じ列の何番目か。
 * - 帯は root → seed → branch の順に上から。node の無い帯は出さない。
 *
 * @param dag C-02 の `{nodes, edges}`
 * @returns `{cards, bands, width, height}`
 *   cards は {run_id, layer, col, row, x, y, w, h}（x, y は世界上の px）
 */
function sgRunsLayout(dag) {
  const d = dag || {};
  const nodes = (Array.isArray(d.nodes) ? d.nodes : []).filter((n) => n && n.run_id != null);
  const edges = Array.isArray(d.edges) ? d.edges : [];
  const ids = new Set(nodes.map((n) => String(n.run_id)));

  // 親（配置に使う線だけ）。両端が node に入っているものだけ採る
  const parents = new Map();
  for (const id of ids) parents.set(id, []);
  for (const e of edges) {
    if (!e) continue;
    if (!SG_RUN_LAYOUT_KINDS.includes(String(e.kind || ""))) continue;
    const from = String(e.from);
    const to = String(e.to);
    if (!ids.has(from) || !ids.has(to)) continue;
    parents.get(to).push(from);
  }

  // 列（深さ）。再帰で、輪は訪問済みの印で切って打ち切る
  const memo = new Map();
  const visiting = new Set();
  const depthOf = (id) => {
    if (memo.has(id)) return memo.get(id);
    if (visiting.has(id)) return 0; // 輪の中。短く切って打ち切る
    visiting.add(id);
    let best = 0;
    for (const p of parents.get(id) || []) {
      const v = depthOf(p) + 1;
      if (v > best) best = v;
    }
    visiting.delete(id);
    memo.set(id, best);
    return best;
  };

  // 行（層の帯の中の何番目か）。元の並び（run_id の昇順）のまま
  const placed = [];
  const rowCount = new Map();
  const rowsIn = new Map();
  for (const n of nodes) {
    const id = String(n.run_id);
    const layer = sgRunLayer(n.layer);
    const col = depthOf(id);
    const key = `${layer} ${col}`;
    const row = rowCount.get(key) || 0;
    rowCount.set(key, row + 1);
    rowsIn.set(layer, Math.max(rowsIn.get(layer) || 0, row + 1));
    placed.push({ node: n, id, layer, col, row });
  }

  // 帯（上から）。node の無い層は出さない
  const bands = [];
  let y = 0;
  const order = SG_RUN_LAYERS.map(([id]) => id);
  if (rowsIn.has(SG_RUN_LAYER_OTHER[0])) order.push(SG_RUN_LAYER_OTHER[0]);
  for (const layer of order) {
    const rows = rowsIn.get(layer) || 0;
    if (!rows) continue;
    const h = rows * SG_CARD_H + (rows - 1) * SG_RUN_GAP_Y + SG_BAND_PAD * 2;
    bands.push({ layer, y, h });
    y += h + SG_BAND_GAP;
  }
  const bandOf = new Map(bands.map((b) => [b.layer, b]));

  // 札の位置
  let maxCol = 0;
  const cards = [];
  for (const p of placed) {
    const band = bandOf.get(p.layer);
    if (!band) continue;
    maxCol = Math.max(maxCol, p.col);
    cards.push({
      run_id: p.node.run_id,
      layer: p.layer,
      col: p.col,
      row: p.row,
      x: SG_BAND_LABEL_W + p.col * (SG_CARD_W + SG_RUN_GAP_X),
      y: band.y + SG_BAND_PAD + p.row * (SG_CARD_H + SG_RUN_GAP_Y),
      w: SG_CARD_W,
      h: SG_CARD_H,
    });
  }
  const width = SG_BAND_LABEL_W + maxCol * (SG_CARD_W + SG_RUN_GAP_X) + SG_CARD_W + SG_BAND_PAD;
  const height = bands.length ? y - SG_BAND_GAP : 0;
  return { cards, bands, width, height };
}

// ============================================================================
// 小さな部品
// ============================================================================

/** 要素を作る（text があれば textContent に入れる）。 */
function sgEl(tag, cls, text) {
  const el = document.createElement(tag);
  if (cls) el.className = cls;
  if (text != null) el.textContent = String(text);
  return el;
}

/** SVG の要素を作る。 */
function sgSvgEl(tag, attrs) {
  const el = document.createElementNS("http://www.w3.org/2000/svg", tag);
  for (const [k, v] of Object.entries(attrs || {})) el.setAttribute(k, String(v));
  return el;
}

/** 小さい 1 行の札（空欄の種類・印・数）。 */
function sgChip(text, kind) {
  return sgEl("span", `sai-chip ${kind || "muted"}`, text);
}

/** 1 行の注。 */
function sgNote(text) {
  return sgEl("p", "sci-graph-note", text);
}

/** 実線の端点（side は right / left / top / bottom）。 */
function sgSidePoint(r, side) {
  const x = Number(r && r.x) || 0;
  const y = Number(r && r.y) || 0;
  const w = Number(r && r.width) || 0;
  const h = Number(r && r.height) || 0;
  if (side === "left") return { x, y: y + h / 2 };
  if (side === "top") return { x: x + w / 2, y };
  if (side === "bottom") return { x: x + w / 2, y: y + h };
  return { x: x + w, y: y + h / 2 }; // right
}

// 辺から外へ出る向き（制御点に使う）
const SG_SIDE_NORMAL = { left: [-1, 0], right: [1, 0], top: [0, -1], bottom: [0, 1] };

/** 小数 1 位に丸める（path を短くするため）。 */
function sgR1(n) {
  return Math.round((Number(n) || 0) * 10) / 10;
}

/** 2 つの四角を結ぶ 3 次ベジェ曲線の d。縦に並ぶときは上から下、横に並ぶときは左から右。 */
function sgEdgePath(a, b) {
  const ra = a || {};
  const rb = b || {};
  const aTop = (Number(ra.y) || 0) + (Number(ra.height) || 0) / 2;
  const bTop = (Number(rb.y) || 0) + (Number(rb.height) || 0) / 2;
  const horizontal = Math.abs(aTop - bTop) < Math.abs(
    (Number(ra.x) || 0) + (Number(ra.width) || 0) / 2 - ((Number(rb.x) || 0) + (Number(rb.width) || 0) / 2)
  );
  const aFirst = horizontal
    ? (Number(ra.x) || 0) + (Number(ra.width) || 0) / 2 <= (Number(rb.x) || 0) + (Number(rb.width) || 0) / 2
    : aTop <= bTop;
  const s1 = horizontal ? (aFirst ? "right" : "left") : (aFirst ? "bottom" : "top");
  const s2 = horizontal ? (aFirst ? "left" : "right") : (aFirst ? "top" : "bottom");
  const p1 = sgSidePoint(ra, s1);
  const p2 = sgSidePoint(rb, s2);
  const n1 = SG_SIDE_NORMAL[s1];
  const n2 = SG_SIDE_NORMAL[s2];
  const k = Math.max(40, Math.hypot(p2.x - p1.x, p2.y - p1.y) * 0.4);
  const c1 = { x: p1.x + n1[0] * k, y: p1.y + n1[1] * k };
  const c2 = { x: p2.x + n2[0] * k, y: p2.y + n2[1] * k };
  return `M ${sgR1(p1.x)} ${sgR1(p1.y)} C ${sgR1(c1.x)} ${sgR1(c1.y)}, ${sgR1(c2.x)} ${sgR1(c2.y)}, ${sgR1(p2.x)} ${sgR1(p2.y)}`;
}

// ============================================================================
// 根の図
// ============================================================================

/**
 * 根の図を描く（container の中身を入れ替える）。
 * @param container 入れる先（#sci-list など）
 * @param status C-08 の status の JSON
 * @param onSelect 札を押したときに呼ぶ関数（無ければ押しても何もしない）
 */
function sgRenderGraph(container, status, onSelect) {
  if (!container) return;
  container.innerHTML = "";
  sgStatus = status || null;
  sgOnSelect = typeof onSelect === "function" ? onSelect : null;
  // 実行の図の残骸を捨てる（どちらの図でも同じ函数の変数を使うため）
  sgRunDag = null;
  sgRunMetrics = {};
  sgShowReturn = false;
  sgView = { tx: 0, ty: 0, z: 1 };
  sgFitted = false;
  sgPtrs = new Map();
  sgDown = null;
  sgPinch = null;

  // 節点が多いときは、候補を隠した状態で始める
  const nodes = sgStatus && Array.isArray(sgStatus.nodes) ? sgStatus.nodes : [];
  sgShowOptions = nodes.length <= SG_MANY_NODES;

  sgRoot = sgEl("div", "sci-graph");

  // 1. 道具の行（候補の出し入れ・拡大・縮小・全体を表示）
  const tools = sgEl("div", "sci-graph-tools sai-toolbar");
  const optBtn = sgEl("button", "sci-graph-btn");
  optBtn.type = "button";
  optBtn.addEventListener("click", () => {
    sgShowOptions = !sgShowOptions;
    sgFitted = false;
    sgDraw();
  });
  tools.appendChild(optBtn);
  sgOptBtn = optBtn;
  for (const [label, title, onClick] of [
    ["＋", "拡大", () => sgZoom(SG_ZOOM_STEP)],
    ["－", "縮小", () => sgZoom(1 / SG_ZOOM_STEP)],
    ["⤢", "全体を表示", () => sgFit()],
  ]) {
    const b = sgEl("button", "sci-graph-btn", label);
    b.type = "button";
    b.title = title;
    b.setAttribute("aria-label", title);
    b.addEventListener("click", onClick);
    tools.appendChild(b);
  }
  sgRoot.appendChild(tools);

  // 2. 図そのもの
  sgStage = sgEl("div", "sci-graph-stage");
  sgStage.tabIndex = 0;
  sgWorld = sgEl("div", "sci-graph-world");
  sgStage.appendChild(sgWorld);
  sgRoot.appendChild(sgStage);
  sgStage.addEventListener("pointerdown", (e) => sgOnPointerDown(e));
  sgStage.addEventListener("pointermove", (e) => sgOnPointerMove(e));
  sgStage.addEventListener("pointerup", (e) => sgOnPointerUp(e));
  sgStage.addEventListener("pointercancel", (e) => sgOnPointerUp(e));
  sgStage.addEventListener("wheel", (e) => sgOnWheel(e), { passive: false });

  // 3. 凡例
  sgRoot.appendChild(sgLegend());

  container.appendChild(sgRoot);
  sgDraw();
}

/** 図の絵を描く（札と線）。 */
function sgDraw() {
  if (!sgWorld) return;
  // 候補の出し入れの釦の文言を、今の状態に合わせて書き直す
  if (sgOptBtn) {
    sgOptBtn.textContent = sgShowOptions ? "OPTION を隠す" : "OPTION を出す";
    sgOptBtn.title = sgShowOptions
      ? "OPTION の札を隠します（ITEM だけの図になります）"
      : "OPTION の札を出します";
  }
  sgWorld.innerHTML = "";
  sgRects = new Map();
  sgExtraRects = [];

  const gateKeys = new Set(sgGateKeys(sgStatus));
  const input = sgGraphInput(sgStatus, sgShowOptions);
  if (!input.cards.length) {
    // 0 件のときは直接 舞台に入れる（世界は 0 幅。absolute の箱は文字を 1 文字ずつ縦に並べてしまう）
    sgPutEmpty(sgEmptyBox({
      icon: "🌱",
      title: "ITEM はまだありません",
      body: "種類を選んで木を始めると、ここに図が出ます。",
    }));
    sgApplyView();
    return;
  }

  // 1. 配置（cvlLayoutBoard を使い回す。無いときは縦横の格子に並べる）
  const placed = typeof cvlLayoutBoard === "function"
    ? cvlLayoutBoard(input.cards, input.layoutEdges)
    : sgFallbackLayout(input.cards);
  for (const p of placed) {
    const card = input.cards.find((c) => c.id === p.id);
    if (card) sgRects.set(p.id, { x: p.x, y: p.y, width: p.width, height: p.height, node: card.node });
  }

  // 2. 線（札の下）
  const svg = sgSvgEl("svg", { class: "sci-graph-edges" });
  sgWorld.appendChild(svg);
  sgSetViewBox(svg);
  const defs = sgSvgEl("defs", {});
  for (const [key] of SG_EDGE_KINDS) {
    if (!sgEdgeStyle(key).arrow) continue;
    const marker = sgSvgEl("marker", {
      id: `sg-arrow-${key}`,
      viewBox: "0 0 10 10",
      refX: 9,
      refY: 5,
      markerWidth: 6,
      markerHeight: 6,
      orient: "auto-start-reverse",
    });
    const p = sgSvgEl("path", { d: "M 0 0 L 10 5 L 0 10 z", fill: sgEdgeStyle(key).colorVar });
    marker.appendChild(p);
    defs.appendChild(marker);
  }
  svg.appendChild(defs);
  for (const e of input.drawEdges) {
    const a = sgRects.get(e.from);
    const b = sgRects.get(e.to);
    if (!a || !b) continue;
    const st = sgEdgeStyle(e.kind);
    const path = sgSvgEl("path", {
      d: sgEdgePath(a, b),
      fill: "none",
      stroke: st.colorVar,
      "stroke-width": 1.6,
      class: `sci-graph-edge sci-graph-edge-${e.kind || "other"}`,
    });
    // 縮小しても線が消えないよう、画面の px で太さを保つ
    path.style.strokeWidth = "min(calc(1.6px * var(--sg-z-inv, 1)), 10px)";
    if (st.dash) path.setAttribute("stroke-dasharray", st.dash);
    if (st.arrow) path.setAttribute("marker-end", `url(#sg-arrow-${e.kind})`);
    svg.appendChild(path);
  }

  // 3. 札（一番上）
  for (const card of input.cards) {
    const r = sgRects.get(card.id);
    if (!r) continue;
    sgWorld.appendChild(sgCardEl(card.node, r, gateKeys));
  }

  // 4. 全体表示（最初だけ。画面の大きさが分かるときだけ動かす）
  if (!sgFitted) {
    if (!sgFit() && typeof requestAnimationFrame === "function") {
      // まだ幅が分からないとき（読み込み直後）は次のフレームで試す
      requestAnimationFrame(() => sgFit());
    }
  }
  sgApplyView();
}

/** cvlLayoutBoard が無いときの並べる方法（横一列の格子）。 */
function sgFallbackLayout(cards) {
  const cols = Math.max(1, Math.ceil(Math.sqrt(cards.length || 1)));
  return cards.map((c, i) => ({
    id: c.id,
    x: (i % cols) * (SG_CARD_W + 40),
    y: Math.floor(i / cols) * (SG_CARD_H + 40),
    width: c.width,
    height: c.height,
  }));
}

/** 線の SVG に viewBox を付ける（描きたい範囲いっぱい）。 */
function sgSetViewBox(svg) {
  let minX = Infinity, minY = Infinity, maxX = -Infinity, maxY = -Infinity;
  for (const r of [...sgRects.values(), ...sgExtraRects]) {
    minX = Math.min(minX, r.x);
    minY = Math.min(minY, r.y);
    maxX = Math.max(maxX, r.x + r.width);
    maxY = Math.max(maxY, r.y + r.height);
  }
  if (!Number.isFinite(minX)) { minX = 0; minY = 0; maxX = 1; maxY = 1; }
  const pad = 40;
  minX -= pad; minY -= pad; maxX += pad; maxY += pad;
  svg.setAttribute("viewBox", `${sgR1(minX)} ${sgR1(minY)} ${sgR1(maxX - minX)} ${sgR1(maxY - minY)}`);
  svg.setAttribute("width", String(sgR1(maxX - minX)));
  svg.setAttribute("height", String(sgR1(maxY - minY)));
  svg.style.left = `${sgR1(minX)}px`;
  svg.style.top = `${sgR1(minY)}px`;
}

/** 札 1 枚。押すと onSelect が呼ばれる。 */
function sgCardEl(node, r, gateKeys) {
  const n = node || {};
  const st = sgStateOf(n);
  const b = sgEl("button", `sci-graph-card sci-graph-state-${st.key}`);
  b.type = "button";
  b.setAttribute("data-sg-id", String(n.id || ""));
  b.style.left = `${r.x}px`;
  b.style.top = `${r.y}px`;
  b.style.width = `${r.width}px`;
  b.style.height = `${r.height}px`;
  b.style.setProperty("--sg-color", st.color);
  // 却下・休眠の候補は薄く出す（消さない）
  if (n.kind === "option" && (n.state === "rejected" || n.state === "dormant")) {
    b.classList.add("sci-graph-faint");
  }
  // 今の関門の条件に出てくる項目は太い枠で目立たせる
  if (n.key && gateKeys.has(n.key)) b.classList.add("sci-graph-gate");

  const head = sgEl("span", "sci-graph-card-head");
  if (n.fixed) head.appendChild(sgEl("span", "sci-graph-pin", "📌"));
  head.appendChild(sgEl("span", "sci-graph-card-mark", st.mark));
  head.appendChild(sgEl("span", "sci-graph-card-id", String(n.id || "")));
  head.appendChild(sgEl("span", "sci-graph-card-state", st.label));
  b.appendChild(head);

  b.appendChild(sgEl("span", "sci-graph-card-title", n.title || "（題のない ITEM）"));

  const foot = sgEl("span", "sci-graph-card-foot");
  if (n.blank && SG_BLANK_LABELS[n.blank]) {
    foot.appendChild(sgChip(SG_BLANK_LABELS[n.blank], n.blank === "conflict" ? "danger" : "warn"));
  }
  const marks = Array.isArray(n.marks) ? n.marks : [];
  for (const m of marks) {
    if (SG_MARK_LABELS[m]) foot.appendChild(sgChip(SG_MARK_LABELS[m], "muted"));
  }
  b.appendChild(foot);
  return b;
}

/** 凡例（図の下）。 */
function sgLegend() {
  const box = sgEl("div", "sci-graph-legend");
  const head = sgEl("span", "sci-graph-legend-title", "線");
  box.appendChild(head);
  for (const [key, label, dash, arrow] of SG_EDGE_KINDS) {
    const item = sgEl("span", "sci-graph-legend-item");
    const line = sgEl("span", "sci-graph-legend-line");
    line.style.setProperty("--sg-color", sgEdgeStyle(key).colorVar);
    if (dash) line.classList.add("dashed");
    if (dash === "2 4") line.classList.add("dotted");
    if (arrow) line.classList.add("arrow");
    item.appendChild(line);
    item.appendChild(sgEl("span", null, label));
    box.appendChild(item);
  }
  return box;
}

// --- 見える範囲の操作 ---

/** 見える範囲を transform にする。 */
function sgApplyView() {
  if (!sgWorld) return;
  sgWorld.style.transform = `translate(${sgView.tx}px, ${sgView.ty}px) scale(${sgView.z})`;
  sgWorld.style.setProperty("--sg-z", String(sgView.z));
  sgWorld.style.setProperty("--sg-z-inv", String(sgZoomInv(sgView.z)));
}

/** 拡大率の逆数（0 にならないように 1 を下限にする）。 */
function sgZoomInv(z) {
  const n = Number(z);
  if (!Number.isFinite(n) || n <= 0) return 1;
  return 1 / n;
}

/**
 * 図を全体表示する（画面の中が全部入る）。
 * 画面の大きさがまだ分からないときは false を返して、やり直さない。
 */
function sgFit() {
  if (!sgStage || !(sgRects.size || sgExtraRects.length)) return false;
  const w = sgStage.clientWidth || 0;
  const h = sgStage.clientHeight || 0;
  if (w <= 0 || h <= 0) return false;
  const rects = [...sgRects.values(), ...sgExtraRects].map((r) => ({ x: r.x, y: r.y, width: r.width, height: r.height }));
  if (typeof cvFitView === "function") {
    sgView = cvFitView(rects, w, h);
  } else {
    // cvFitView が無いときは、等倍の左上を起点にする
    sgView = { tx: 0, ty: 0, z: 1 };
  }
  sgFitted = true;
  sgApplyView();
  return true;
}

/** 画面の中央を中心に拡大・縮小する。 */
function sgZoom(factor) {
  if (!sgStage) return;
  const w = sgStage.clientWidth || 0;
  const h = sgStage.clientHeight || 0;
  if (w <= 0 || h <= 0) return;
  sgZoomAtPx(w / 2, h / 2, factor);
}

/** (px, py) の下の世界上の点が動かないように、見える範囲を変える。 */
function sgZoomAtPx(px, py, factor) {
  if (typeof cvZoomAt === "function") {
    sgView = cvZoomAt(sgView, px, py, factor);
  } else {
    const z0 = Math.max(0.1, Math.min(2.5, Number(sgView.z) || 1));
    const z = Math.max(0.1, Math.min(2.5, z0 * (Number(factor) || 1)));
    const wx = (px - sgView.tx) / z0;
    const wy = (py - sgView.ty) / z0;
    sgView = { tx: px - wx * z, ty: py - wy * z, z };
  }
  sgApplyView();
}

/** 押している指から節点 id を取り出す（無ければ null）。 */
function sgHitNode(target) {
  if (!target || typeof target.closest !== "function") return null;
  const box = target.closest("[data-sg-id]");
  if (!box) return null;
  return box.getAttribute("data-sg-id");
}

function sgOnPointerDown(e) {
  if (!sgStage) return;
  try { sgStage.setPointerCapture(e.pointerId); } catch { /* 捕まえなくてもよい */ }
  sgPtrs.set(e.pointerId, { x: e.clientX, y: e.clientY });

  if (sgPtrs.size === 2) {
    const [p, q] = [...sgPtrs.values()];
    const rect = sgStage.getBoundingClientRect();
    sgPinch = {
      dist: Math.hypot(q.x - p.x, q.y - p.y) || 1,
      z0: sgView.z,
      wx: ((p.x + q.x) / 2 - rect.left - sgView.tx) / sgView.z,
      wy: ((p.y + q.y) / 2 - rect.top - sgView.ty) / sgView.z,
    };
    sgDown = null;
    return;
  }
  if (sgPtrs.size > 2) return;

  sgDown = {
    id: e.pointerId,
    sx: e.clientX,
    sy: e.clientY,
    view: { ...sgView },
    nodeId: sgHitNode(e.target),
    mode: null,
  };
}

function sgOnPointerMove(e) {
  if (!sgPtrs.has(e.pointerId)) return;
  sgPtrs.set(e.pointerId, { x: e.clientX, y: e.clientY });

  // 2 本指（拡大）
  if (sgPtrs.size >= 2 && sgPinch) {
    const [p, q] = [...sgPtrs.values()];
    const dist = Math.hypot(q.x - p.x, q.y - p.y) || 1;
    const rect = sgStage ? sgStage.getBoundingClientRect() : { left: 0, top: 0 };
    const z = Math.max(0.1, Math.min(2.5, sgPinch.z0 * (dist / sgPinch.dist)));
    sgView = {
      tx: (p.x + q.x) / 2 - rect.left - sgPinch.wx * z,
      ty: (p.y + q.y) / 2 - rect.top - sgPinch.wy * z,
      z,
    };
    sgApplyView();
    return;
  }

  const d = sgDown;
  if (!d || d.id !== e.pointerId) return;
  const dx = e.clientX - d.sx;
  const dy = e.clientY - d.sy;
  if (!d.mode) {
    if (Math.hypot(dx, dy) < SG_DRAG_SLOP) return; // クリックか移動か決まらない
    d.mode = "pan";
  }
  sgView = { tx: d.view.tx + dx, ty: d.view.ty + dy, z: d.view.z };
  sgApplyView();
}

function sgOnPointerUp(e) {
  sgPtrs.delete(e.pointerId);
  if (sgPtrs.size < 2) sgPinch = null;
  if (sgStage) {
    try { sgStage.releasePointerCapture(e.pointerId); } catch { /* 解放できなくてもよい */ }
  }
  const d = sgDown;
  if (!d || d.id !== e.pointerId) {
    if (sgPtrs.size === 0) sgDown = null;
    return;
  }
  sgDown = null;
  if (e.type === "pointercancel") return; // 途中で指が離れた
  if (d.mode) return;                     // 動かした（移動扱い）

  // 動かしていない -> 札を押した
  if (!d.nodeId) return;
  if (typeof sgOnSelect === "function") sgOnSelect(d.nodeId);
}

/** ホイール（縦・横・Ctrl で拡大）。 */
function sgOnWheel(e) {
  if (!sgStage) return;
  e.preventDefault();
  const rect = sgStage.getBoundingClientRect();
  const px = e.clientX - rect.left;
  const py = e.clientY - rect.top;
  if (e.ctrlKey || e.metaKey) {
    sgZoomAtPx(px, py, Math.exp(-e.deltaY * 0.01));
  } else {
    sgView = {
      tx: sgView.tx - (e.shiftKey ? e.deltaY : (e.deltaX || 0)),
      ty: sgView.ty - (e.shiftKey ? 0 : e.deltaY),
      z: sgView.z,
    };
    sgApplyView();
  }
}

// ============================================================================
// 実行の図（root / seed / branch の 3 層・U08）
// ============================================================================

/**
 * 空の案内を「見える範囲の所」（sgStage）に入れる。
 * 世界（sgWorld）は 0 幅なので、そこに絶対配置の箱を置くと文字が縦に並ぶ。
 * 案内だけ世界から出して、箱ごと作り直す。
 */
function sgPutEmpty(box) {
  if (!sgStage) return;
  sgStage.innerHTML = "";
  sgWorld = sgEl("div", "sci-graph-world");
  const wrap = sgEl("div", "sci-graph-empty");
  wrap.appendChild(box);
  sgStage.appendChild(sgWorld);
  sgStage.appendChild(wrap);
}

/** 空の案内（ui.js の saiEmpty が無ければただの文にする）。 */
function sgEmptyBox(opts) {
  const o = opts || {};
  if (typeof saiEmpty === "function") {
    return saiEmpty({ icon: o.icon, title: o.title, body: o.body });
  }
  const box = sgEl("div", "sai-empty");
  if (o.title) box.appendChild(sgEl("p", "sai-empty-title", o.title));
  if (o.body) box.appendChild(sgEl("p", "sai-empty-body", o.body));
  return box;
}

/**
 * 実行の図を描く（container の中身を入れ替える）。
 * @param container 入れる先（広い画面は #sci-board・狭い画面は #sci-list）
 * @param dag C-02 の `{nodes, edges}`（`GET /api/projects/<name>/rtree` の返答）
 * @param metrics `{run_id: "val_auc 0.871"}` の辞書（無ければ {} でよい）
 * @param onSelect 札を押したときに呼ぶ関数（引数は run_id の数）
 */
function sgRenderRuns(container, dag, metrics, onSelect) {
  if (!container) return;
  container.innerHTML = "";
  sgRunDag = dag || { nodes: [], edges: [] };
  sgRunMetrics = metrics && typeof metrics === "object" ? metrics : {};
  // 札を押したときの種類は data-sg-id と同じ文字列で来るので、数に直して渡す
  sgOnSelect = typeof onSelect === "function" ? (id) => onSelect(Number(id)) : null;
  sgView = { tx: 0, ty: 0, z: 1 };
  sgFitted = false;
  sgPtrs = new Map();
  sgDown = null;
  sgPinch = null;
  sgOptBtn = null;
  sgShowReturn = false;

  sgRoot = sgEl("div", "sci-graph sci-graph-runs");

  // 1. 道具の行（返却の線・拡大・縮小・全体を表示）
  const tools = sgEl("div", "sci-graph-tools sai-toolbar");
  const retBtn = sgEl("button", "sci-graph-btn");
  retBtn.type = "button";
  retBtn.addEventListener("click", () => {
    sgShowReturn = !sgShowReturn;
    sgDrawRuns();
  });
  tools.appendChild(retBtn);
  sgOptBtn = retBtn;
  for (const [label, title, onClick] of [
    ["＋", "拡大", () => sgZoom(SG_ZOOM_STEP)],
    ["－", "縮小", () => sgZoom(1 / SG_ZOOM_STEP)],
    ["⤢", "全体を表示", () => sgFit()],
  ]) {
    const b = sgEl("button", "sci-graph-btn", label);
    b.type = "button";
    b.title = title;
    b.setAttribute("aria-label", title);
    b.addEventListener("click", onClick);
    tools.appendChild(b);
  }
  sgRoot.appendChild(tools);

  // 2. 指標が読めなかったときの注（図の上に 1 行）
  const noteBox = sgEl("div", "sci-graph-runs-note");
  sgRoot.appendChild(noteBox);

  // 3. 図そのもの（見える範囲の動かし方は根の図と同じ）
  sgStage = sgEl("div", "sci-graph-stage");
  sgStage.tabIndex = 0;
  sgWorld = sgEl("div", "sci-graph-world");
  sgStage.appendChild(sgWorld);
  sgRoot.appendChild(sgStage);
  sgStage.addEventListener("pointerdown", (e) => sgOnPointerDown(e));
  sgStage.addEventListener("pointermove", (e) => sgOnPointerMove(e));
  sgStage.addEventListener("pointerup", (e) => sgOnPointerUp(e));
  sgStage.addEventListener("pointercancel", (e) => sgOnPointerUp(e));
  sgStage.addEventListener("wheel", (e) => sgOnWheel(e), { passive: false });

  // 4. 凡例
  sgRoot.appendChild(sgRunsLegend());

  container.appendChild(sgRoot);
  sgDrawRuns();
}

/** 実行の図の絵を描く（帯・線・札）。 */
function sgDrawRuns() {
  if (!sgWorld) return;
  if (sgOptBtn) {
    sgOptBtn.textContent = sgShowReturn ? "返却の線を隠す" : "返却の線を出す";
    sgOptBtn.title = sgShowReturn
      ? "返却の線（task_output）を隠します（delegation の逆向きなので輪に見えます）"
      : "返却の線（task_output）を出します";
  }
  sgWorld.innerHTML = "";
  sgRects = new Map();
  sgExtraRects = [];

  const nodes = Array.isArray(sgRunDag && sgRunDag.nodes) ? sgRunDag.nodes : [];
  const nodeById = new Map();
  for (const n of nodes) if (n && n.run_id != null) nodeById.set(String(n.run_id), n);

  // 0 件のときは saiEmpty で案内する
  if (!nodeById.size) {
    sgPutEmpty(sgEmptyBox({
      icon: "🧪",
      title: "実行の図はまだありません",
      body: "層（root / seed / branch）を持つブロックの実行がまだありません。sci-* や sci-exp-seed を動かすと出ます",
    }));
    sgApplyView();
    return;
  }

  // 1. 配置（帯の行と深さの列）
  const layout = sgRunsLayout(sgRunDag);
  for (const c of layout.cards) {
    const n = nodeById.get(String(c.run_id));
    if (!n) continue;
    sgRects.set(String(c.run_id), { x: c.x, y: c.y, width: c.w, height: c.h, node: n });
  }

  // 2. 帯（札より後ろに敷く。文字は帯の中に置く。帯も見える範囲の算出に入れる）
  const bandBox = sgEl("div", "sci-graph-bands");
  sgExtraRects = layout.bands.map((b) => ({ x: 0, y: b.y, width: layout.width, height: b.h }));
  for (const b of layout.bands) {
    const box = sgEl("div", `sci-graph-band sci-graph-band-${b.layer}`);
    box.style.top = `${sgR1(b.y)}px`;
    box.style.left = "0px";
    box.style.width = `${sgR1(layout.width)}px`;
    box.style.height = `${sgR1(b.h)}px`;
    box.appendChild(sgEl("span", "sci-graph-band-label", sgRunLayerLabel(b.layer)));
    bandBox.appendChild(box);
  }
  sgWorld.appendChild(bandBox);

  // 3. 線（札の下）
  const svg = sgSvgEl("svg", { class: "sci-graph-edges" });
  sgWorld.appendChild(svg);
  sgSetViewBox(svg);
  const defs = sgSvgEl("defs", {});
  svg.appendChild(defs);
  for (const e of Array.isArray(sgRunDag && sgRunDag.edges) ? sgRunDag.edges : []) {
    if (!e) continue;
    const kind = String(e.kind || "");
    // 返却の線（task_output）は既定で隠す（delegation の逆向きで輪に見えるため）
    if (kind === "task_output" && !sgShowReturn) continue;
    const a = sgRects.get(String(e.from));
    const b = sgRects.get(String(e.to));
    if (!a || !b) continue;
    const st = sgRunEdgeStyle(kind);
    const path = sgSvgEl("path", {
      d: sgEdgePath(a, b),
      fill: "none",
      stroke: st.colorVar,
      "stroke-width": 1.6,
      class: `sci-graph-edge sci-graph-edge-run sci-graph-edge-${kind || "other"}`,
    });
    path.style.strokeWidth = "min(calc(1.6px * var(--sg-z-inv, 1)), 10px)";
    if (st.dash) path.setAttribute("stroke-dasharray", st.dash);
    svg.appendChild(path);
  }

  // 4. 札（一番上）
  for (const c of layout.cards) {
    const n = nodeById.get(String(c.run_id));
    const r = sgRects.get(String(c.run_id));
    if (!n || !r) continue;
    sgWorld.appendChild(sgRunCardEl(n, r));
  }

  // 5. 全体表示（最初だけ）
  if (!sgFitted) {
    if (!sgFit() && typeof requestAnimationFrame === "function") {
      requestAnimationFrame(() => sgFit());
    }
  }
  sgApplyView();
}

/** 実行の図の札 1 枚（1 行目: run とブロック、2 行目: 状態と時刻、3 行目: 指標）。 */
function sgRunCardEl(node, r) {
  const n = node || {};
  const st = sgRunStateOf(n.status);
  const layer = sgRunLayer(n.layer);
  const b = sgEl("button", `sci-graph-card sci-graph-run sci-graph-run-${layer} sci-graph-state-${st.key}`);
  b.type = "button";
  b.setAttribute("data-sg-id", String(n.run_id == null ? "" : n.run_id));
  b.setAttribute("data-sg-layer", layer);
  b.style.left = `${sgR1(r.x)}px`;
  b.style.top = `${sgR1(r.y)}px`;
  b.style.width = `${r.width}px`;
  b.style.height = `${r.height}px`;
  b.style.setProperty("--sg-color", st.color);

  // 1 行目
  const head = sgEl("span", "sci-graph-card-head");
  head.appendChild(sgEl("span", "sci-graph-card-id", `#${n.run_id == null ? "" : n.run_id}`));
  head.appendChild(sgEl("span", "sci-graph-card-title", n.name || n.block || "（名前のない実行）"));
  b.appendChild(head);

  // 2 行目（状態と開始時刻）
  const line = sgEl("span", "sci-graph-run-line");
  line.appendChild(sgEl("span", "sci-graph-card-mark", `${st.mark} ${st.label}`));
  const time = sgRunTimeLabel(n.created_at);
  if (time) line.appendChild(sgEl("span", "sci-graph-card-state", time));
  b.appendChild(line);

  // 3 行目（実行の層だけ。指標の無いときは出さない）
  if (layer === "branch") {
    const metric = sgRunMetrics[String(n.run_id)] || "";
    if (metric) b.appendChild(sgEl("span", "sci-graph-run-metric", String(metric)));
  }
  return b;
}

/** 実行の図の凡例（図の下）。 */
function sgRunsLegend() {
  const box = sgEl("div", "sci-graph-legend");
  box.appendChild(sgEl("span", "sci-graph-legend-title", "線"));
  for (const [key, label, dash] of SG_RUN_EDGE_KINDS) {
    const item = sgEl("span", "sci-graph-legend-item");
    const line = sgEl("span", "sci-graph-legend-line");
    line.style.setProperty("--sg-color", sgRunEdgeStyle(key).colorVar);
    if (dash) line.classList.add("dashed");
    if (dash === "2 4") line.classList.add("dotted");
    item.appendChild(line);
    item.appendChild(sgEl("span", null, label));
    box.appendChild(item);
  }
  const tip = sgEl("span", "sci-graph-legend-item sci-graph-legend-note");
  tip.textContent = "実行の層は指標（MLflow の最良値）を出します";
  box.appendChild(tip);
  return box;
}

// ============================================================================
// 候補の表
// ============================================================================

/**
 * 候補の表を描く（container の中身を入れ替える）。
 * 選ぶ・並べる項目のときだけ表を出し、ほかは 1 行の注に置く。
 * @param container 入れる先（.sci-table-box など）
 * @param detail C-08 の node の JSON
 */
function sgRenderTable(container, detail) {
  if (!container) return;
  container.innerHTML = "";
  const d = detail || {};
  const node = d.node || {};
  const types = Array.isArray(node.types) ? node.types : [];
  const has = SG_TABLE_TYPES.some((t) => types.includes(t));
  if (!has) {
    container.appendChild(sgNote("この ITEM には OPTION の表がありません。"));
    return;
  }

  // 1. 評価の軸（mind.md の VALUE_AXES）
  const axes = Array.isArray(d.axes) ? d.axes.filter((a) => String(a || "").trim()) : [];
  const head = sgEl("div", "sci-graph-axes");
  if (axes.length) {
    for (let i = 0; i < axes.length; i += 1) {
      head.appendChild(sgEl("div", "sci-graph-axis", `${i + 1}. ${String(axes[i]).trim()}`));
    }
  } else {
    head.appendChild(sgNote("mind.md の VALUE_AXES に書くと、ここに出ます。"));
  }
  container.appendChild(head);

  // 2. 表（行を押すと stSelect で板が開く）
  const rows = sgTableRows(d);
  if (!rows.length) {
    container.appendChild(sgNote("OPTION はまだありません。"));
    return;
  }
  const wrap = sgEl("div", "sci-table-wrap sci-graph-table-wrap");
  const table = document.createElement("table");
  table.className = "sci-table sci-graph-table";
  const thead = document.createElement("thead");
  const hrow = document.createElement("tr");
  for (const h of ["OPTION", "状態", "SUPPORT", "REFUTE", "論文", "FINDING"]) {
    const th = document.createElement("th");
    th.textContent = h;
    hrow.appendChild(th);
  }
  thead.appendChild(hrow);
  table.appendChild(thead);

  const tbody = document.createElement("tbody");
  for (const r of rows) {
    const tr = document.createElement("tr");
    tr.className = `sci-graph-row sci-graph-row-${r.state}`;
    tr.setAttribute("data-sg-id", String(r.id));

    const name = document.createElement("td");
    name.appendChild(sgEl("span", "sci-id", r.id));
    name.appendChild(sgEl("span", "sci-cell-title", r.title || "（題のない OPTION）"));
    if (r.summary) name.appendChild(sgEl("p", "sci-cell-body", r.summary));
    tr.appendChild(name);

    const state = document.createElement("td");
    state.appendChild(sgEl("span", "sci-graph-state-mark", `${r.mark} ${r.label}`));
    tr.appendChild(state);

    for (const n of [r.support, r.refute, r.papers]) {
      const td = document.createElement("td");
      td.className = "sci-num";
      td.textContent = String(n);
      tr.appendChild(td);
    }
    tbody.appendChild(tr);
  }
  table.appendChild(tbody);
  wrap.appendChild(table);
  container.appendChild(wrap);

  // 行を押したときの動作（板を開く関数が無いときは動かさない）
  if (typeof stSelect !== "function") return;
  tbody.addEventListener("click", (e) => {
    const tr = e.target && typeof e.target.closest === "function" ? e.target.closest("[data-sg-id]") : null;
    if (!tr) return;
    void stSelect(tr.getAttribute("data-sg-id"));
  });
}

// --- node からの読み込み（tests/test_scigraph.js） ---
if (typeof module !== "undefined" && module.exports) {
  module.exports = {
    sgRenderGraph,
    sgRenderTable,
    sgRenderRuns,
    sgRunsLayout,
    sgRunLayer,
    sgRunLayerLabel,
    sgRunStateOf,
    sgRunTimeLabel,
    sgRunEdgeStyle,
    sgGraphInput,
    sgEdgeStyle,
    sgTableRows,
    sgGateKeys,
    sgStateOf,
    sgEdgePath,
    SG_CARD_W,
    SG_CARD_H,
    SG_EDGE_KINDS,
    SG_OPTION_ORDER,
    SG_RUN_LAYERS,
    SG_RUN_EDGE_KINDS,
    SG_RUN_STATES,
    SG_RUN_LAYOUT_KINDS,
  };
}
