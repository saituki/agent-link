"use strict";
// 研究方針の木の「根の図」と「実行の図」（実装計画 U11・共有契約 C-14）。
//
//   sgRenderGraph(container, {status, papers, dag, metrics}, onSelect) … 根の図（4 層の樹）
//   sgTreeLayout(input, opts)                  … 根の図の配置（純関数）
//   sgRunsLayout(dag)                          … 実行の図の配置（純関数）
//   sgRenderRuns(container, dag, metrics, onSelect) … 実行の図（帯）
//   sgMarkSelected(key)                        … 選んでいる札の印（どちらの図でも）
//
// 根の図は、幹（seed）を中心に、下へ ITEM の木と論文の葉（root）、上へ実験のアイデア
// （sprout）と実験（branch）が伸びる上下の向きの樹である。onSelect には
//   {kind: "seed"} / {kind: "node", id} / {kind: "paper", node, paper} / {kind: "run", id}
// を渡す。札の key は "seed" / "N12" / "P:N12:37" / "R125"（data-sg-id と同じ）。
// 実行の図は研究方針の run の DAG を、上から branch / sprout / seed / root の帯で並べる。
//
// 中の処理は Rust のコアが返した JSON と MLflow が返した値だけを使う
// （apps/phone は他の DB を開かない）。このファイルは新しい情報を足さない。
//
// 書き方:
//   - 根の図の配置は専用の単純な木の配置（sgTreeLayout）。同じ入力なら開くたびに動かない。
//   - 樹に描く線は親子だけ。depends などは樹の形を崩すので詳細の板の「結び」で見る。
//   - 札の大きさは固定。題は 2 行で切って、全部は板で見せる。
//   - 見える範囲（tx, ty, z）だけを動かし、札の位置は動かさない。
//     全体表示と拡大は canvaslayout.js の cvFitView / cvZoomAt を typeof で確かめて使う。
//   - 却下・休眠の候補は消さず薄く出す。
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

// --- 実行の図（branch / sprout / seed / root の 4 層）---

// 層の帯（上から）。帯の左に書く語。C-02 の layer と 1 対 1
const SG_RUN_LAYERS = [
  ["branch", "実行（branch）"],
  ["sprout", "実験のアイデア（sprout）"],
  ["seed", "構想（seed）"],
  ["root", "方針（root）"],
];
// C-02 に無い層のための帯（4 層のあとに足す）
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
let sgInput = null;    // 根の図の入力 {status, papers, dag, metrics}
let sgSelMap = new Map(); // 札の key → onSelect に渡す sel
let sgSelectedKey = null; // 選んでいる札の key
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
// 実行の図（4 層）
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
 * - 帯は branch → sprout → seed → root の順に上から（SG_RUN_LAYERS の並び）。node の無い帯は出さない。
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
// 根の図の配置（4 層の樹・純関数）
// ============================================================================

// 樹の大きさ・間隔（世界上の px）
const SG_SEED_W = 280;
const SG_SEED_H = 96;
const SG_PAPER_W = 160;
const SG_PAPER_H = 52;
const SG_RUN_W = 180;
const SG_RUN_H = 60;
const SG_TREE_STEP = 120; // 段と段の縦の間隔（札の上端どうし）
const SG_TREE_GAP = 24;   // 同じ段の札と札の間
// 樹の層の並び（上から）
const SG_TREE_LAYERS = ["branch", "sprout", "seed", "root"];

const sgIsReview = (n) => n.block === "sci-exp-review";
// 旧い名前（sci-exp-seed）の run も sprout として出す（既にある実験を消さないため）
const sgIsSprout = (n) => n.layer === "sprout" || n.block === "sci-exp-seed";
const sgIsBranch = (n) => n.layer === "branch" && !sgIsReview(n);

/**
 * 根の図の配置。幹を (0, 0) の中心に、下へ ITEM の木と論文の葉、上へ sprout と branch を置く。
 * @param input {status, papers, dag, metrics}
 * @param opts {showOptions}
 * @returns {cards: [{key, kind, x, y, width, height, data}], edges: [{from, to, kind}], bands: [{layer, top, bottom}]}
 */
function sgTreeLayout(input, opts) {
  const inp = input || {};
  const showOptions = !opts || opts.showOptions !== false;
  const status = inp.status || {};
  const papers = Array.isArray(inp.papers) ? inp.papers.filter((p) => p && p.node != null) : [];
  const dag = inp.dag || {};

  // 木の節点: {key, kind, w, h, data, children}
  const mk = (key, kind, w, h, data) => ({ key, kind, w, h, data, children: [] });
  const edges = [];

  // --- 下: ITEM の木と論文の葉 ---
  const nodes = (Array.isArray(status.nodes) ? status.nodes : [])
    .filter((n) => n && n.id && (showOptions || n.kind !== "option"));
  const ids = new Set(nodes.map((n) => n.id));
  const down = new Map(nodes.map((n) => [n.id, mk(n.id, "node", SG_CARD_W, SG_CARD_H, n)]));
  const downRoots = [];
  for (const n of nodes) {
    const parents = (Array.isArray(n.parents) ? n.parents : []).filter((p) => ids.has(p) && p !== n.id);
    if (!parents.length) { downRoots.push(down.get(n.id)); continue; }
    down.get(parents[0]).children.push(down.get(n.id));
    for (const p of parents.slice(1)) edges.push({ from: p, to: n.id, kind: "parent2" });
  }
  // 論文の葉は、節点の子の ITEM のあとに並べる
  for (const p of papers) {
    const holder = down.get(p.node);
    if (holder) holder.children.push(mk(`P:${p.node}:${p.paper}`, "paper", SG_PAPER_W, SG_PAPER_H, p));
  }

  // --- 上: sprout と branch ---
  const runs = (Array.isArray(dag.nodes) ? dag.nodes : []).filter((n) => n && n.run_id != null);
  const byId = new Map(runs.map((n) => [String(n.run_id), n]));
  const shown = runs.filter((n) => sgIsSprout(n) || sgIsBranch(n));
  const up = new Map(shown.map((n) => [String(n.run_id), mk(`R${n.run_id}`, sgIsSprout(n) ? "sprout" : "branch", SG_RUN_W, SG_RUN_H, n)]));
  const incoming = new Map();
  for (const e of Array.isArray(dag.edges) ? dag.edges : []) {
    if (!e || !SG_RUN_LAYOUT_KINDS.includes(String(e.kind || ""))) continue;
    const to = String(e.to);
    if (!incoming.has(to)) incoming.set(to, []);
    incoming.get(to).push(String(e.from));
  }
  // branch の親: 入る線の始点が sprout か branch ならそれ。review なら、その review に入る線の始点
  const parentOf = (id) => {
    const cand = [];
    for (const f of incoming.get(id) || []) {
      const n = byId.get(f);
      if (!n || f === id) continue;
      if (sgIsReview(n)) {
        for (const g of incoming.get(f) || []) {
          const m = byId.get(g);
          if (m && g !== id && sgIsBranch(m)) cand.push(g);
        }
      } else if (up.has(f)) cand.push(f);
    }
    cand.sort((a, b) => Number(b) - Number(a));
    return cand.length ? cand[0] : null;
  };
  const upRoots = [];
  for (const n of shown) {
    const id = String(n.run_id);
    const p = sgIsBranch(n) ? parentOf(id) : null;
    if (p) up.get(p).children.push(up.get(id)); else upRoots.push(up.get(id));
  }
  // sprout は幹の子、親の無い branch も幹の子。sprout を先に並べる
  upRoots.sort((a, b) => (a.kind === "sprout" ? 0 : 1) - (b.kind === "sprout" ? 0 : 1));

  // --- 配置 ---
  const cards = [{ key: "seed", kind: "seed", x: -SG_SEED_W / 2, y: 0, width: SG_SEED_W, height: SG_SEED_H, data: status.seed || null }];
  const seen = new Set(["seed"]);
  const kidsWidth = (t) => t.children.reduce((a, c) => a + width(c), 0) + Math.max(0, t.children.length - 1) * SG_TREE_GAP;
  const width = (t) => {
    if (t.sw == null) t.sw = Math.max(t.w, kidsWidth(t));
    return t.sw;
  };
  // 循環で幹から届かない節点は描かない（seen に入れた物だけ置く）
  const place = (t, left, depth, dir, parentKey) => {
    if (seen.has(t.key)) return;
    seen.add(t.key);
    const cx = left + width(t) / 2;
    cards.push({ key: t.key, kind: t.kind, x: cx - t.w / 2, y: dir * depth * SG_TREE_STEP, width: t.w, height: t.h, data: t.data });
    edges.push({ from: parentKey, to: t.key, kind: "parent" });
    let x = cx - kidsWidth(t) / 2;
    for (const c of t.children) { place(c, x, depth + 1, dir, t.key); x += width(c) + SG_TREE_GAP; }
  };
  const placeForest = (roots, dir) => {
    let x = -(roots.reduce((a, c) => a + width(c), 0) + Math.max(0, roots.length - 1) * SG_TREE_GAP) / 2;
    for (const r of roots) { place(r, x, 1, dir, "seed"); x += width(r) + SG_TREE_GAP; }
  };
  placeForest(downRoots, 1);
  placeForest(upRoots, -1);
  // 両端が置かれた線だけ残す
  const placedKeys = new Set(cards.map((c) => c.key));
  const outEdges = edges.filter((e) => placedKeys.has(e.from) && placedKeys.has(e.to));

  const bands = [];
  for (const layer of SG_TREE_LAYERS) {
    const list = cards.filter((c) => (layer === "root" ? c.kind === "node" || c.kind === "paper" : c.kind === layer));
    if (!list.length) continue;
    bands.push({ layer, top: Math.min(...list.map((c) => c.y)), bottom: Math.max(...list.map((c) => c.y + c.height)) });
  }
  return { cards, edges: outEdges, bands };
}

/** 札から onSelect に渡す値を作る（根の図の札 1 枚分）。 */
function sgSelectionOf(card) {
  const d = card.data || {};
  if (card.kind === "seed") return { kind: "seed" };
  if (card.kind === "node") return { kind: "node", id: card.key };
  if (card.kind === "paper") return { kind: "paper", node: d.node, paper: d.paper };
  return { kind: "run", id: Number(d.run_id) };
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
 * @param input {status, papers, dag, metrics}
 * @param onSelect 札を押したときに呼ぶ関数。引数は {kind: "seed" | "node" | "paper" | "run", …}
 */
function sgRenderGraph(container, input, onSelect) {
  if (!container) return;
  container.innerHTML = "";
  sgInput = input || {};
  sgStatus = sgInput.status || null;
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

/** 図の絵を描く（帯・線・札）。 */
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
  sgSelMap = new Map();

  const hasNodes = sgStatus && Array.isArray(sgStatus.nodes) && sgStatus.nodes.some((n) => n && n.id);
  if (!hasNodes) {
    // 0 件のときは直接 舞台に入れる（世界は 0 幅。absolute の箱は文字を 1 文字ずつ縦に並べてしまう）
    sgPutEmpty(sgEmptyBox({
      icon: "🌱",
      title: "ITEM はまだありません",
      body: "種類を選んで木を始めると、ここに図が出ます。",
    }));
    sgApplyView();
    return;
  }

  // 1. 配置
  const gateKeys = new Set(sgGateKeys(sgStatus));
  const layout = sgTreeLayout(sgInput, { showOptions: sgShowOptions });
  for (const c of layout.cards) {
    sgRects.set(c.key, { x: c.x, y: c.y, width: c.width, height: c.height, node: c.data });
    sgSelMap.set(c.key, sgSelectionOf(c));
  }

  // 2. 帯（札より後ろに敷く。図の左に層の名前。帯も見える範囲の算出に入れる）
  const minX = Math.min(...layout.cards.map((c) => c.x)) - SG_BAND_PAD;
  const maxX = Math.max(...layout.cards.map((c) => c.x + c.width)) + SG_BAND_PAD;
  const left = minX - SG_BAND_LABEL_W;
  const bandBox = sgEl("div", "sci-graph-bands");
  for (const b of layout.bands) {
    const top = b.top - SG_BAND_PAD;
    const h = b.bottom - b.top + SG_BAND_PAD * 2;
    sgExtraRects.push({ x: left, y: top, width: maxX - left, height: h });
    const box = sgEl("div", `sci-graph-band sci-graph-band-${b.layer}`);
    box.style.top = `${sgR1(top)}px`;
    box.style.left = `${sgR1(left)}px`;
    box.style.width = `${sgR1(maxX - left)}px`;
    box.style.height = `${sgR1(h)}px`;
    box.appendChild(sgEl("span", "sci-graph-band-label", b.layer.toUpperCase()));
    bandBox.appendChild(box);
  }
  sgWorld.appendChild(bandBox);

  // 3. 線（札の下）。親子だけ描く。親が 2 つの節点の 2 本目は点線
  const svg = sgSvgEl("svg", { class: "sci-graph-edges" });
  sgWorld.appendChild(svg);
  sgSetViewBox(svg);
  for (const e of layout.edges) {
    const a = sgRects.get(e.from);
    const b = sgRects.get(e.to);
    if (!a || !b) continue;
    const st = sgEdgeStyle("parent");
    const path = sgSvgEl("path", {
      d: sgEdgePath(a, b),
      fill: "none",
      stroke: st.colorVar,
      "stroke-width": 1.6,
      class: `sci-graph-edge sci-graph-edge-${e.kind}`,
    });
    path.style.strokeWidth = "min(calc(1.6px * var(--sg-z-inv, 1)), 10px)";
    if (e.kind === "parent2") path.setAttribute("stroke-dasharray", "2 4");
    svg.appendChild(path);
  }

  // 4. 札（一番上）
  sgRunMetrics = sgInput && sgInput.metrics && typeof sgInput.metrics === "object" ? sgInput.metrics : {};
  for (const c of layout.cards) {
    const r = sgRects.get(c.key);
    if (c.kind === "seed") sgWorld.appendChild(sgSeedCardEl(c.data, r));
    else if (c.kind === "node") sgWorld.appendChild(sgCardEl(c.data, r, gateKeys));
    else if (c.kind === "paper") sgWorld.appendChild(sgPaperCardEl(c.data, r));
    else sgWorld.appendChild(sgRunCardEl(c.data, r));
  }
  sgMarkSelected(sgSelectedKey);

  // 5. 全体表示（最初だけ。画面の大きさが分かるときだけ動かす）
  if (!sgFitted) {
    if (!sgFit() && typeof requestAnimationFrame === "function") {
      // まだ幅が分からないとき（読み込み直後）は次のフレームで試す
      requestAnimationFrame(() => sgFit());
    }
  }
  sgApplyView();
}

/** 選んでいる札に枠の色を付ける（key が無い・null なら外す）。どちらの図でも呼べる。 */
function sgMarkSelected(key) {
  sgSelectedKey = key == null ? null : String(key);
  if (!sgWorld || typeof sgWorld.querySelectorAll !== "function") return;
  for (const el of sgWorld.querySelectorAll("[data-sg-id]")) {
    el.classList.toggle("is-selected", el.getAttribute("data-sg-id") === sgSelectedKey);
  }
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

/** 幹の札（上に SEED、本文に SEED_SENTENCE、下に型と GATE の進み）。 */
function sgSeedCardEl(seed, r) {
  const st = sgStatus || {};
  const b = sgEl("button", "sci-graph-card sci-graph-seed");
  b.type = "button";
  b.setAttribute("data-sg-id", "seed");
  b.style.left = `${sgR1(r.x)}px`;
  b.style.top = `${sgR1(r.y)}px`;
  b.style.width = `${r.width}px`;
  b.style.height = `${r.height}px`;
  b.appendChild(sgEl("span", "sci-graph-card-head", "SEED"));
  const text = seed && seed.text ? seed.text : "（まだ SEED_SENTENCE はありません）";
  b.appendChild(sgEl("span", "sci-graph-card-title", text));
  const foot = sgEl("span", "sci-graph-card-foot");
  const type = st.type && (st.type.name || st.type.id);
  if (type) foot.appendChild(sgChip(String(type), "muted"));
  const g = st.gate;
  if (g && g.total != null) foot.appendChild(sgChip(`GATE ${g.current}/${g.total}`, "muted"));
  b.appendChild(foot);
  return b;
}

/** 論文の葉の札（#番号と年・題名 2 行・FINDING の数・読んだ状態）。 */
function sgPaperCardEl(p, r) {
  const d = p || {};
  const state = String(d.read_state || "unread");
  const b = sgEl("button", `sci-graph-card sci-graph-paper sci-graph-paper-${state === "unread" ? "unread" : "read"}`);
  b.type = "button";
  b.setAttribute("data-sg-id", `P:${d.node}:${d.paper}`);
  b.style.left = `${sgR1(r.x)}px`;
  b.style.top = `${sgR1(r.y)}px`;
  b.style.width = `${r.width}px`;
  b.style.height = `${r.height}px`;
  const head = sgEl("span", "sci-graph-card-head");
  head.appendChild(sgEl("span", "sci-graph-card-id", `#${d.paper}${d.year ? ` ${d.year}` : ""}`));
  head.appendChild(sgEl("span", "sci-graph-card-state", `FINDING ${Number(d.findings) || 0}`));
  b.appendChild(head);
  b.appendChild(sgEl("span", "sci-graph-card-title", d.title || "（題のない論文）"));
  b.title = `${d.title || ""}（${state}）`;
  return b;
}

/** 凡例（図の下）。樹に描くのは親子の線だけ。 */
function sgLegend() {
  const box = sgEl("div", "sci-graph-legend");
  box.appendChild(sgEl("span", "sci-graph-legend-title", "線"));
  const item = sgEl("span", "sci-graph-legend-item");
  const line = sgEl("span", "sci-graph-legend-line");
  line.style.setProperty("--sg-color", sgEdgeStyle("parent").colorVar);
  item.appendChild(line);
  item.appendChild(sgEl("span", null, "PARENT"));
  box.appendChild(item);
  box.appendChild(sgEl("span", "sci-graph-legend-item sci-graph-legend-note", "ほかの結びは詳細の板の「結び」で見ます"));
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
  const sel = sgSelMap.get(d.nodeId);
  if (sel && typeof sgOnSelect === "function") sgOnSelect(sel);
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
// 実行の図（4 層）
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
  sgInput = null;
  sgOnSelect = typeof onSelect === "function" ? onSelect : null;
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
  sgSelMap = new Map();
  for (const n of nodes) {
    if (!n || n.run_id == null) continue;
    nodeById.set(String(n.run_id), n);
    sgSelMap.set(`R${n.run_id}`, { kind: "run", id: Number(n.run_id) });
  }

  // 0 件のときは saiEmpty で案内する
  if (!nodeById.size) {
    sgPutEmpty(sgEmptyBox({
      icon: "🧪",
      title: "実行の図はまだありません",
      body: "層（seed / root / sprout / branch）を持つブロックの実行がまだありません。sci-* や sci-exp-sprout を動かすと出ます",
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
  b.setAttribute("data-sg-id", `R${n.run_id == null ? "" : n.run_id}`);
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
  if (layer === "branch" && n.block !== "sci-exp-review") {
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

// --- node からの読み込み（tests/test_scigraph.js） ---
if (typeof module !== "undefined" && module.exports) {
  module.exports = {
    sgRenderGraph,
    sgRenderRuns,
    sgTreeLayout,
    sgSelectionOf,
    sgMarkSelected,
    sgRunsLayout,
    sgRunLayer,
    sgRunLayerLabel,
    sgRunStateOf,
    sgRunTimeLabel,
    sgRunEdgeStyle,
    sgEdgeStyle,
    sgGateKeys,
    sgStateOf,
    sgEdgePath,
    SG_CARD_W,
    SG_CARD_H,
    SG_EDGE_KINDS,
    SG_RUN_LAYERS,
    SG_RUN_EDGE_KINDS,
    SG_RUN_STATES,
    SG_RUN_LAYOUT_KINDS,
  };
}
