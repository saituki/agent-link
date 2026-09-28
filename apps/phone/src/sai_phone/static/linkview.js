// sai-phone の List 画面（実装計画 14）の本体。
//
// 画面は 1 つ（住所は 2 つ）:
//   List（run 未選択）  #/project/<name>/list
//   List（run 選択済み） #/project/<name>/list/<run_id>/<表示>
// Project 上部の Dashboard | List の切り替えもこのファイルが持つ
// （Dashboard 側の帯の描き方も fillLinkSeg としてここに置く・計画 14 §1.1）。
// ツリーは sai-link TUI の左サイドバーと同じ並び（`GET …/tree` のそのまま・計画 14 §1.2）。
// run の中身は 6 つの表示（Log / Stats / History / task / message / waggle）で、
// 最後に見ていた表示をプロジェクトをまたいで覚える（別の run に切り替えても同じ表示・§1.3）。
// データはすべて /api/projects/<name>/… 経由で，sai-phone 自身は DB を開かない（00 §2）。
// グラフ（Stats の推移・History の円）は SVG を手描きで作る（外部ライブラリは入れない・00 §2）。
// md の整形は markdown.js の renderMarkdown，数字・SVG の作りは stats.js / project.js の
// グローバル関数を使い回す（13 §5・14 §6）。
// DOM を使わない小さな関数は末尾で node に渡す（tests/test_linkview.js）。
"use strict";

// --- 定数 ---

// run の中身の表示（帯の並び＝TUI のショートカットと同じ・計画 14 §1.3）。
// 最後の session だけは Agent タブのメッセージ画面へ飛ぶので、sessionHash が作る。
const LV_VIEWS = [
  ["log", "Log"],
  ["stats", "Stats"],
  ["history", "History"],
  ["task", "task"],
  ["message", "message"],
  ["waggle", "waggle"],
  ["session", "session"],
];
// Project 上部の 4 つの切り替え（共有契約 C-F7）。
// 並びと表示名はここ 1 か所。dataset の値は変えない。
const LV_MODES = [
  ["dashboard", "Dashboard"],
  ["workflow", "Workflow"],
  ["list", "List"],
  ["defs", "設定"],
];
// 最後に見ていた Project の画面をプロジェクトごとに覚える（localStorage・計画 14 §6・C-F7）
const LV_MODE_KEY = "sai_phone_link_mode";
// 最後に見ていた表示を覚える（localStorage・計画 14 §1.3）
const LV_VIEW_KEY = "sai_phone_list_view";
// ツリーの開閉をプロジェクトごとに覚える（localStorage・計画 14 §1.2）
const LV_FOLDS_KEY = "sai_phone_list_folds";
// History の種類別の色（5 色・project.js のドーナツと同じ考え方）。色は CSS 変数。
const LV_TYPE_COLORS = ["var(--series-1)", "var(--series-2)", "var(--series-3)", "var(--series-4)", "var(--series-5)", "var(--series-6)"];
// History の種類（表示順）
const LV_TYPES = ["user", "agent", "toolCall", "toolResult", "thinking"];
// Stats のグラフの種類（TUI の 1 / 2 / 3 と同じ・4 つ目の「表」は常に下に出す・計画 14 §1.3）
const LV_CHARTS = [
  ["turn", "ターン"],
  ["cumpct", "累積 %"],
  ["cum", "累積量"],
];
// レーン名 → 用語の key（見出しの横に置く用語の札用）
const LV_TERMS = { todo: "todo", running: "running", review: "review", done: "done" };
// 表示名 → 用語の key（多すぎないように見出しの所だけ置く）
const LV_VIEW_TERMS = { log: "run", task: "card", waggle: "waggle", message: "run" };

// --- 小さな助け舟 ---

/** 数として読む（読めなければ 0）。stats.js の num とは名前を分ける（定義の順番に依存しないため）。 */
function lvNum(v) {
  const n = Number(v);
  return Number.isFinite(n) ? n : 0;
}

/** Project 上部の 4 つの切り替えのどれか（不明・無ければ dashboard）。 */
function lvMode(key) {
  return LV_MODES.some((m) => m[0] === key) ? key : "dashboard";
}

/** 1 つ上の階層の住所を作る（登録・テストから同じ形で作れるように）。 */
function modeHash(name, mode) {
  const base = `#/project/${encodeURIComponent(name)}`;
  switch (lvMode(mode)) {
    case "list": return listHash(name, null, null);
    case "workflow": return `${base}/workflow`;
    case "defs": return `${base}/defs`;
    default: return base;
  }
}

/** List の住所（run と表示は付けなくてもよい・計画 14 §6）。 */
function listHash(name, runId, view) {
  const base = `#/project/${encodeURIComponent(name)}/list`;
  if (runId == null) return base;
  return `${base}/${encodeURIComponent(String(runId))}/${encodeURIComponent(view || "log")}`;
}

/**
 * List の住所を読む。List でなければ null。
 * run_id が数字でない住所は「run 未選択」として扱う（Agent 画面に落ちないため）。
 */
function parseListHash(hash) {
  const m = String(hash || "").match(
    /^#\/project\/(.+?)\/list(?:\/([^/?#]+)\/([^/?#]+))?\/?$/
  );
  if (!m) return null;
  const name = decodeURIComponent(m[1]);
  if (m[2] == null || !/^\d+$/.test(m[2])) return { name, runId: null, view: null };
  return { name, runId: Number(m[2]), view: decodeURIComponent(m[3] || "") };
}

/** Agent タブのメッセージ画面の住所（session の表示から移るときに使う・U11）。 */
function sessionHash(name, runId) {
  return `#/run/${encodeURIComponent(name)}/${encodeURIComponent(String(runId))}`;
}

// --- ブラウザの保存場所（プライベートブラウズでも動くように必ず失敗に備える） ---

function loadModeMap() {
  try {
    const raw = localStorage.getItem(LV_MODE_KEY);
    const obj = raw ? JSON.parse(raw) : null;
    return obj && typeof obj === "object" ? obj : {};
  } catch {
    return {};
  }
}

/** 最後に見ていた Project の画面をプロジェクトごとに覚える（計画 14 §6・C-F7）。 */
function saveMode(name, mode) {
  try {
    const map = loadModeMap();
    map[name] = lvMode(mode);
    localStorage.setItem(LV_MODE_KEY, JSON.stringify(map));
  } catch { /* 保存できなくても動く */ }
}

/** このプロジェクトで最後に見ていた画面（dashboard / workflow / list / defs）。 */
function rememberedMode(name) {
  return lvMode(loadModeMap()[name]);
}

/** このプロジェクトは List を選んでいたか（Dashboard の open が最初に見る・計画 14 §1.1）。 */
function wantsListView(name) {
  return rememberedMode(name) === "list";
}

function loadViewMemory() {
  try {
    const v = localStorage.getItem(LV_VIEW_KEY);
    return LV_VIEWS.some((x) => x[0] === v) ? v : "log";
  } catch {
    return "log";
  }
}

/** 最後に見ていた表示を覚える（todo run では呼ばない・計画 14 §1.3）。 */
function saveViewMemory(view) {
  try {
    localStorage.setItem(LV_VIEW_KEY, view);
  } catch { /* 保存できなくても動く */ }
}

function lvLoadFolds(name) {
  try {
    const raw = localStorage.getItem(LV_FOLDS_KEY);
    const map = raw ? JSON.parse(raw) : null;
    if (!map || typeof map !== "object") return null;
    const list = map[name];
    // 覚えが無ければ null（＝最初から全部開いた状態・計画 14 §1.2）
    return Array.isArray(list) ? list : null;
  } catch {
    return null;
  }
}

function lvSaveFolds(name, open) {
  try {
    const raw = localStorage.getItem(LV_FOLDS_KEY);
    let map = null;
    try { map = raw ? JSON.parse(raw) : null; } catch { map = null; }
    if (!map || typeof map !== "object") map = {};
    map[name] = Array.isArray(open) ? open : [];
    localStorage.setItem(LV_FOLDS_KEY, JSON.stringify(map));
  } catch { /* 保存できなくても動く */ }
}

/** 開いているフォルダの id の一覧を toggle する（node のテストからも使う）。 */
function toggleFold(open, id) {
  const set = new Set(Array.isArray(open) ? open : []);
  if (set.has(id)) set.delete(id);
  else set.add(id);
  return Array.from(set);
}

// --- 表示の決め方（DOM を使わない・tests/test_linkview.js） ---

/** 6 つの表示の帯。todo run は task だけ選べる（計画 14 §1.3）。 */
function viewTabs(todo) {
  return LV_VIEWS.map(([key, label]) => ({
    key,
    label,
    ok: !todo || key === "task",
  }));
}

/**
 * 開く表示を決める。最後に見ていた表示をそのまま使い、
 * todo run では task に固定する（表示は URL に残し，記憶は変えない・計画 14 §1.3）。
 */
function pickView(todo, remembered) {
  const tabs = viewTabs(!!todo);
  const hit = tabs.find((t) => t.key === remembered && t.ok);
  if (hit) return hit.key;
  return todo ? "task" : "log";
}

/** todo の run（lane / status が todo）か（▶ 実行と task 固定の判定・計画 14 §1.3）。 */
function isTodoRun(run) {
  if (!run || typeof run !== "object") return false;
  return run.lane === "todo" || run.status === "todo";
}

/**
 * ツリーを「今開いているフォルダだけ」並べた行に組み替える（描き方の元・計画 14 §1.2）。
 * open === null は「覚えが無い＝全部開く」。depth は 行ごとの字下げの段。
 */
function flattenTree(items, open) {
  const openSet = open == null ? null : new Set(Array.isArray(open) ? open : []);
  const rows = [];
  const walk = (list, depth) => {
    for (const node of list || []) {
      if (!node || typeof node !== "object") continue;
      if (node.type === "folder") {
        const id = String(node.id || node.label || "");
        const isOpen = openSet === null || openSet.has(id);
        rows.push({
          type: "folder",
          id,
          label: String(node.label || id),
          depth,
          open: isOpen,
        });
        if (isOpen) walk(node.children, depth + 1);
      } else if (node.type === "run" && node.run_id != null) {
        rows.push({
          type: "run",
          id: `run:${node.run_id}`,
          label: String(node.label || `ID${node.run_id}`),
          depth,
          runId: Number(node.run_id),
          lane: String(node.lane || ""),
          status: String(node.status || ""),
        });
      }
    }
  };
  walk(items, 0);
  return rows;
}

/** ツリーの中から run を 1 つ探す（同じ run が状態フォルダと R/r の両方に出るため最初の 1 件）。 */
function findRun(items, runId) {
  const want = Number(runId);
  if (!Number.isFinite(want)) return null;
  for (const row of flattenTree(items, null)) {
    if (row.type === "run" && row.runId === want) return row;
  }
  return null;
}

/**
 * その run が出るフォルダの lane を全部集める（同じ run が review/done と R/r の
 * 両方に出るため最初の 1 件では決められない・計画書 A の U10）。
 * R/r 側の lane も sai-link の owner_for が review / done を返すので使える。
 */
function runLanes(items, runId) {
  const want = Number(runId);
  const lanes = new Set();
  if (!Number.isFinite(want)) return lanes;
  for (const row of flattenTree(items, null)) {
    if (row.type === "run" && row.runId === want && row.lane) lanes.add(row.lane);
  }
  return lanes;
}

/**
 * 移動ボタンの出分と表示（review → done → Workflow・計画書 A の U10）。
 * どちらにも出ない（todo / running / 既に Workflow）ときは null。
 */
function advanceAction(items, runId) {
  const lanes = runLanes(items, runId);
  if (lanes.has("review")) return { from: "review", to: "done", label: "→ done" };
  if (lanes.has("done")) return { from: "done", to: "workflow", label: "→ Workflow" };
  return null;
}

/** ツリーに並ぶ全部のフォルダ id（覚えが無い状態から 1 つ閉じるときに入替える・計画 14 §1.2）。 */
function allFolderIds(items) {
  const ids = [];
  const walk = (list) => {
    for (const node of list || []) {
      if (!node || typeof node !== "object") continue;
      if (node.type === "folder") {
        ids.push(String(node.id || node.label || ""));
        walk(node.children);
      }
    }
  };
  walk(items);
  return ids;
}

/** Log の表示を「節 × 行（ラベル / 値の文字列）」に組み替える（計画 14 §1.3）。 */
function logSections(d) {
  const data = d && typeof d === "object" ? d : {};
  const out = [];
  const text = (v) => (v == null ? "" : String(v));

  // 1. 入力（Inputs）。task_text は task の表示で読めるので出さない
  const inputs = (data.input_fields || []).filter(
    (f) => f && typeof f === "object" && (f.label || f.value) && f.label !== "task_text"
  );
  if (inputs.length > 0) {
    out.push({
      title: "Inputs",
      rows: inputs.map((f) => [text(f.label) || "input", text(f.value)]),
    });
  }

  // 2. 設定（ラウンド / モデル / thinking / tag / 時刻 / commit）
  const conf = [];
  if (data.description) conf.push(["説明", text(data.description)]);
  if (data.user_block_description) conf.push(["説明", text(data.user_block_description)]);
  if (data.round != null) conf.push(["ラウンド", `r${data.round}`]);
  if (data.global_round != null) conf.push(["グローバル", `R${data.global_round}`]);
  if (data.tag) conf.push(["タグ", text(data.tag)]);
  if (data.model) conf.push(["モデル", text(data.model)]);
  if (data.thinking) conf.push(["thinking", text(data.thinking)]);
  if (data.started_at) conf.push(["開始", text(data.started_at)]);
  if (data.finished_at) conf.push(["終了", text(data.finished_at)]);
  if (data.git_commit_before || data.git_commit_after) {
    const before = text(data.git_commit_before) || "-";
    const after = text(data.git_commit_after);
    conf.push(["commit", after && after !== before ? `${before} → ${after}` : before]);
  }
  if (conf.length > 0) out.push({ title: "設定", rows: conf });

  // 3. 統計（ターン数・ツール回数・トークン）
  const st = data.session_stats;
  if (st && typeof st === "object") {
    const rows = [];
    if (st.turn_count != null) rows.push(["ターン数", text(st.turn_count)]);
    if (st.tool_call_count != null) rows.push(["ツール", `${text(st.tool_call_count)} 回`]);
    for (const [key, label] of [
      ["input_tokens", "入力トークン"],
      ["output_tokens", "出力トークン"],
      ["thinking_tokens", "思考トークン"],
      ["total_tokens", "合計トークン"],
    ]) {
      if (st[key] != null) rows.push([label, text(st[key])]);
    }
    if (rows.length > 0) out.push({ title: "統計", rows });
  }

  // 4. つながり（どの run から / どの run へ・親子と waggle の受け渡し）
  const links = [];
  for (const v of data.connections_from || []) links.push(["から", text(v)]);
  for (const v of data.connections_to || []) links.push(["へ", text(v)]);
  if (links.length > 0) out.push({ title: "つながり", rows: links });

  // 5. 出力トークン（成果物）
  const outs = (data.output_waggle_tokens || []).map(text).filter(Boolean);
  if (outs.length > 0) out.push({ title: "出力トークン", rows: outs.map((t) => ["waggle", t]) });

  return out;
}

/** Stats の大きな数字（入力 / 出力 / 読 / 書 / 費用 / ヒット率・計画 14 §1.3）。 */
function statsCells(totals) {
  const t = totals && typeof totals === "object" ? totals : {};
  const cost = t.cost && typeof t.cost === "object" ? t.cost : {};
  const denom = lvNum(t.cache_read) + lvNum(t.input);
  return [
    { label: "入力", value: lvNum(t.input), kind: "tokens" },
    { label: "出力", value: lvNum(t.output), kind: "tokens" },
    { label: "キャッシュ読み", value: lvNum(t.cache_read), kind: "tokens" },
    { label: "キャッシュ書き", value: lvNum(t.cache_write), kind: "tokens" },
    {
      label: "費用",
      value: cost.total == null ? null : Number(cost.total),
      kind: "money",
    },
    {
      label: "ヒット率",
      value: denom > 0 ? (lvNum(t.cache_read) / denom) * 100 : null,
      kind: "percent",
    },
  ];
}

/** Stats のグラフの系列（TUI の 1 / 2 / 3 と同じ 3 種・計画 14 §1.3）。 */
function statsSeries(data, mode) {
  const d = data && typeof data === "object" ? data : {};
  const msgs = Array.isArray(d.messages) ? d.messages : [];
  const cum = d.cumulative && typeof d.cumulative === "object" ? d.cumulative : {};
  const line = (values) => (Array.isArray(values) ? values.map(lvNum) : []);
  if (mode === "turn") {
    return [
      {
        name: "ターンごとのヒット率",
        color: "var(--series-1)",
        pct: true,
        values: line(msgs.map((m) => (m || {}).cache_hit_percent)),
      },
    ];
  }
  if (mode === "cumpct") {
    return [
      {
        name: "累積ヒット率",
        color: "var(--series-2)",
        pct: true,
        values: line(cum.cum_hit_percent),
      },
    ];
  }
  return [
    { name: "入力", color: "var(--series-1)", pct: false, values: line(cum.cum_input) },
    { name: "キャッシュ読み", color: "var(--series-2)", pct: false, values: line(cum.cum_cache_read) },
    { name: "キャッシュ書き", color: "var(--series-3)", pct: false, values: line(cum.cum_cache_write) },
  ];
}

/** History の種類別（user / agent / toolCall / toolResult / thinking）の割合（計画 14 §1.3）。 */
function typeSlices(typeTokens) {
  const tt = typeTokens && typeof typeTokens === "object" ? typeTokens : {};
  const list = LV_TYPES.map((key) => ({ label: key, value: lvNum(tt[key]) })).filter(
    (x) => x.value > 0
  );
  const total = list.reduce((a, x) => a + x.value, 0);
  if (total <= 0) return [];
  return list.map((x) => ({ ...x, ratio: x.value / total }));
}

/** task.md の frontmatter から `scheduled_at:` を読む（無ければ空）。 */
function lvScheduledAt(text) {
  const m = String(text || "").match(/^---\n[\s\S]*?^scheduled_at:[ \t]*(.*)$/m);
  return m ? m[1].trim() : "";
}

/** waggle が 1 件だけなら一覧を通さず直接開く（計画 14 §1.3）。 */
function waggleDirect(items) {
  return Array.isArray(items) && items.length === 1;
}

// ===========================================================================
// ここから下はブラウザだけで動く部分（node のテストでは読まれない）
// ===========================================================================

// --- 画面の状態 ---

let lvName = "";           // 開いているプロジェクト名
let lvRunId = null;        // 選んでいる run（null＝未選択）
let lvView = null;         // 今見ている表示（7 つのどれか）
let lvRun = null;          // 選んでいる run のツリーの行（label / lane / status）
let lvTree = null;         // ツリー（GET …/tree の items・null＝未読）
let lvTreeError = "";      // ツリーを読めなかったときの 1 行
let lvFolds = null;        // 開いているフォルダ（null＝全部開く）
let lvMsg = "";            // 見出し下に出す 1 行
let lvMsgErr = false;      // その行を赤で出すか（失敗の理由）
let lvBusy = false;        // ▶ 実行の送信中
let lvSeq = 0;             // 画面が入れ替わった合図（古い応答を捨てる）
let lvWaggleList = null;   // 成果物の一覧（同じ run の間だけ使い回す）
let lvWaggleToken = null;  // 開いている成果物（null＝一覧）
let lvChartMode = "turn";  // Stats のグラフの種類
let lvMdText = "";         // 表示中の md（task / message）の本文
let lvEditing = false;     // task を書き換えているか
let lvDraft = "";          // 書きかけの本文
let lvSaving = false;      // 保存中

function lvBody() {
  return document.getElementById("lv-body");
}

function lvSetMsg(text, isErr) {
  lvMsg = String(text || "");
  lvMsgErr = !!isErr;
  lvRenderMsg();
}

function lvRenderMsg() {
  const box = document.getElementById("lv-msg");
  if (!box) return;
  box.textContent = lvMsg;
  box.className = lvMsgErr ? "lv-msg err" : "lv-msg";
  box.hidden = !lvMsg;
}

// --- データの読み込み（失敗しても画面は止めない・計画 14 §1.3） ---

async function lvFetchJson(path) {
  const res = await apiFetch(path);
  const body = await res.json().catch(() => ({}));
  if (!res.ok) throw new Error(body.detail || "読み込めませんでした");
  return body;
}

async function lvLoadTree(seq) {
  try {
    const data = await lvFetchJson(
      `/api/projects/${encodeURIComponent(lvName)}/tree`
    );
    if (seq != null && seq !== lvSeq) return false;
    lvTree = Array.isArray(data.items) ? data.items : [];
    lvTreeError = "";
    window.SaiShell.ok();
    return true;
  } catch (e) {
    if (seq != null && seq !== lvSeq) return false;
    lvTreeError = (e && e.message) || "ツリーを読み込めませんでした";
    if (!Array.isArray(lvTree)) lvTree = [];
    window.SaiShell.fail();
    return false;
  }
}

// --- List の入口（住所 → 状態 → 描画・計画 14 §6） ---

async function openList(params) {
  const seq = ++lvSeq;
  const name = params && params.name ? params.name : lvName;
  if (!name) return;
  const nameChanged = name !== lvName;
  lvName = name;
  // Dashboard | List の選択を List 側として覚える（計画 14 §6）
  saveMode(name, "list");
  fillLinkSeg("list-mode-seg", "list", name);
  // 見出しのプロジェクト名を入れる（4 画面共通の見出し・U01 §3）
  const title = document.getElementById("list-title");
  if (title) title.textContent = name;

  await lvLoadTree(seq);
  if (seq !== lvSeq) return;

  const run = params && params.runId != null ? findRun(lvTree, params.runId) : null;
  const runId = run ? run.runId : null;
  const todo = run ? isTodoRun(run) : false;
  let view = null;
  if (runId != null) {
    const wanted =
      params.view && LV_VIEWS.some((v) => v[0] === params.view)
        ? params.view
        : loadViewMemory();
    view = pickView(todo, wanted);
    // 覚えるのは「選んだ表示」だけ（todo で task に固定した分は覚えず・計画 14 §1.3）。
    // session は Agent タブへ飛ぶ画面なので最後に見ていた表示には残さない（U11）。
    if (!todo && params.view === view && view !== "session") saveViewMemory(view);
  }

  // session は List には描かず，Agent タブのメッセージ画面へ移る（U11）。
  // location.replace にするのは，戻ると同じ session 住所に戻ってまた移ってしまうため。
  if (view === "session") {
    location.replace(sessionHash(name, runId));
    return;
  }

  // 住所を正しい形に直す（数字でない run_id・todo run の task 以外など）
  const target = listHash(name, runId, view);
  if (target !== location.hash) {
    location.replace(target); // hashchange で openList をやり直す
    return;
  }

  const runChanged = nameChanged || lvRunId !== runId;
  lvRunId = runId;
  lvView = view;
  lvRun = run;
  if (runChanged) {
    lvWaggleList = null;
    lvWaggleToken = null;
  }
  lvFolds = lvLoadFolds(name);

  lvRenderHead();
  // まだ何も選んでいないときは最初からツリーを開いた状態で見せる（計画 14 §1.2）
  lvApplyWide();
  lvSetDrawer(runId == null || lvWideOn === true);

  if (runId == null) {
    lvSetMsg("");
    const body = lvBody();
    if (body) {
      body.innerHTML = "";
      body.appendChild(
        el("p", "st-note", "左のツリーから実行を選ぶと中身が出ます")
      );
    }
    return;
  }
  await lvLoadContent(seq);
}

/** ↻ と開き直しで呼ぶ（住所のまま中身を取り直す・計画 14 §1.2）。 */
async function lvReload() {
  const p = parseListHash(location.hash);
  if (p) await openList(p);
}

// --- 見出しと帯（計画 14 §1.3） ---

function lvRenderHead() {
  const head = document.getElementById("lv-head");
  if (!head) return;
  if (lvRunId == null || !lvRun) {
    head.hidden = true;
    return;
  }
  head.hidden = false;
  document.getElementById("lv-run-label").textContent =
    lvRun.label || `ID${lvRunId}`;
  const state = document.getElementById("lv-run-state");
  const lane = (lvRun.lane || "").replace(/[^a-z]/gi, "");
  state.textContent = lvRun.status || lvRun.lane || "";
  state.className = `lv-run-state lv-lane-${lane || "none"}`;
  // 状態の横に用語の札を出す（見出しの所だけ）
  if (typeof saiTerm === "function" && LV_TERMS[lvRun.lane || lvRun.status]) {
    state.appendChild(document.createTextNode(" "));
    state.appendChild(saiTerm(LV_TERMS[lvRun.lane || lvRun.status]));
  }

  const tabs = document.getElementById("lv-tabs");
  tabs.innerHTML = "";
  for (const t of viewTabs(isTodoRun(lvRun))) {
    const b = document.createElement("button");
    b.className = "lv-tab" + (t.key === lvView ? " active" : "");
    b.textContent = t.label;
    b.disabled = !t.ok;
    if (LV_VIEW_TERMS[t.key]) {
      b.appendChild(document.createTextNode(" "));
      b.appendChild(saiTerm(LV_VIEW_TERMS[t.key]));
    }
    if (t.ok) {
      b.addEventListener("click", () => {
        location.hash = listHash(lvName, lvRunId, t.key);
      });
    }
    tabs.appendChild(b);
  }
  lvRenderActions();
  lvRenderMsg();
}

// --- ▶ 実行（todo の run のみ・新 API POST …/runs/{run_id}/start を使う・計画 15） ---

function lvRenderActions() {
  const box = document.getElementById("lv-actions");
  if (!box) return;
  box.innerHTML = "";
  if (!lvRun) return;
  const add = (label, onClick, cls) => {
    const btn = document.createElement("button");
    btn.className = "lv-run-btn" + (cls ? ` ${cls}` : "");
    btn.textContent = label;
    btn.disabled = lvBusy;
    btn.addEventListener("click", onClick);
    box.appendChild(btn);
  };
  if (isTodoRun(lvRun)) {
    add("▶ 実行", lvRunTodo);
    add("⏰ 予約", lvSchedule, "ghost");
  } else {
    // review / done の run だけ review → done → Workflow の移動を出す（計画書 A の U10）
    const adv = advanceAction(lvTree, lvRunId);
    if (adv) add(adv.label, () => lvAdvance(adv));
  }
  // 動いている run は消せない（pane が生きているため）
  if (!["running", "pending"].includes(String(lvRun.status || lvRun.lane || ""))) {
    add("🗑 削除", lvDelete, "ghost");
  }
}

/** todo の実行予定時刻を決める（空で取り消し）。時刻になると常駐が実行する。 */
async function lvSchedule() {
  if (lvBusy || lvRunId == null) return;
  const base = `/api/projects/${encodeURIComponent(lvName)}/runs/${lvRunId}`;
  let current = "";
  try {
    const md = await lvFetchJson(`${base}/md/task`);
    current = lvScheduledAt(md.text || "");
  } catch { /* 読めなくても決められる */ }
  const at = await saiPrompt({
    title: "実行予定時刻",
    body: "この時刻になると Mac の常駐が実行します。空にすると予約を取り消します。",
    value: current,
    placeholder: "例: 2026-10-01 09:00",
    ok: "予約",
  });
  if (at === null) return;
  lvBusy = true;
  lvRenderActions();
  try {
    const res = await apiFetch(`${base}/schedule`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ at: at.trim() }),
    });
    const out = await res.json().catch(() => ({}));
    if (!res.ok) throw new Error(out.detail || "予約できませんでした");
    window.SaiShell.ok();
    lvSetMsg(out.scheduled_at ? `${out.scheduled_at} に実行します` : "予約を取り消しました", false);
  } catch (e) {
    window.SaiShell.fail();
    lvSetMsg((e && e.message) || "予約できませんでした", true);
  } finally {
    lvBusy = false;
    lvRenderActions();
  }
}

/** List の項目を消す（子の実行と run フォルダも消える）。 */
async function lvDelete() {
  if (lvBusy || lvRunId == null) return;
  const label = (lvRun && lvRun.label) || `ID${lvRunId}`;
  const ok = await saiConfirm({
    title: "削除",
    body: `${label} を消します。子の実行と成果物のフォルダも消え、元に戻せません。`,
    ok: "削除",
    danger: true,
  });
  if (!ok) return;
  lvBusy = true;
  lvRenderActions();
  try {
    const res = await apiFetch(
      `/api/projects/${encodeURIComponent(lvName)}/runs/${lvRunId}`,
      { method: "DELETE" }
    );
    const out = await res.json().catch(() => ({}));
    if (!res.ok) throw new Error(out.detail || "消せませんでした");
    window.SaiShell.ok();
    saiToast(`${label} を消しました`);
    location.hash = listHash(lvName, null, null);
  } catch (e) {
    window.SaiShell.fail();
    lvSetMsg((e && e.message) || "消せませんでした", true);
  } finally {
    lvBusy = false;
    lvRenderActions();
  }
}

/** review → done / done → Workflow を iPhone から移す（POST …/runs/{run_id}/advance）。 */
async function lvAdvance(adv) {
  if (lvBusy || lvRunId == null || !adv) return;
  const label = (lvRun && lvRun.label) || `ID${lvRunId}`;
  if (!confirm(`${label} を ${adv.to} へ移します。よろしいですか？`)) return;
  lvBusy = true;
  lvRenderActions();
  try {
    const res = await apiFetch(
      `/api/projects/${encodeURIComponent(lvName)}/runs/${lvRunId}/advance`,
      { method: "POST" }
    );
    const out = await res.json().catch(() => ({}));
    if (!res.ok) throw new Error(out.detail || "移せませんでした");
    window.SaiShell.ok();
    lvSetMsg(`ID${lvRunId} を ${out.to || adv.to} へ移しました`, false);
    await lvLoadTree(lvSeq); // 置き場所が変わったのでツリーを取り直す
    lvRun = findRun(lvTree, lvRunId);
    lvRenderHead();
  } catch (e) {
    window.SaiShell.fail();
    lvSetMsg((e && e.message) || "移せませんでした", true);
  } finally {
    lvBusy = false;
    lvRenderActions();
  }
}

async function lvRunTodo() {
  if (lvBusy || lvRunId == null) return;
  const label = (lvRun && lvRun.label) || `ID${lvRunId}`;
  // 確認は既存の ▶ 実行と同じ動き（実行はエージェントを動かすため必ず聞く・計画 15）
  if (!confirm(`このカードを実行しますか？\n\n${label}`)) return;
  lvBusy = true;
  lvRenderActions();
  try {
    const res = await apiFetch(
      `/api/projects/${encodeURIComponent(lvName)}/runs/${lvRunId}/start`,
      { method: "POST" }
    );
    const out = await res.json().catch(() => ({}));
    if (!res.ok) throw new Error(out.detail || "送れませんでした");
    window.SaiShell.ok();
    lvSetMsg(out.message || "実行を要求しました", false);
    await lvLoadTree(lvSeq); // todo から動いたはずなのでツリーを取り直す
    lvRenderHead();
  } catch (e) {
    window.SaiShell.fail();
    // 404「未実行のカードが見つかりません」も 502（常駐停止など）もここに 1 行で出る
    lvSetMsg((e && e.message) || "送れませんでした", true);
  } finally {
    lvBusy = false;
    lvRenderActions();
  }
}

// --- 中身（7 つの表示・計画 14 §1.3） ---

async function lvLoadContent(seq) {
  const body = lvBody();
  if (!body) return;
  lvEditing = false;
  lvSaving = false;
  lvSetMsg("");
  body.innerHTML = "<p class='st-note'>読み込み中…</p>";
  const base = `/api/projects/${encodeURIComponent(lvName)}/runs/${lvRunId}`;
  try {
    if (lvView === "waggle") {
      await lvLoadWaggle(seq);
      if (seq !== lvSeq) return;
      return;
    }
    const part =
      lvView === "log" || lvView === "stats" || lvView === "history"
        ? `view/${lvView}`
        : `md/${lvView}`;
    const data = await lvFetchJson(`${base}/${part}`);
    if (seq !== lvSeq) return; // その間に別の表示を開いた
    window.SaiShell.ok();
    body.innerHTML = "";
    if (lvView === "log") lvRenderLog(data);
    else if (lvView === "stats") lvRenderStats(data);
    else if (lvView === "history") lvRenderHistory(data);
    else lvRenderMd(data);
  } catch (e) {
    if (seq !== lvSeq) return;
    window.SaiShell.fail();
    const msg = (e && e.message) || "読み込めませんでした";
    lvSetMsg(msg, true); // 「まだありません」（404）もここに 1 行で出る
    body.innerHTML = "";
    body.appendChild(el("p", "st-note", msg));
  }
}

/**
 * つながりの行から run_id を取り出す（sai-link の `R9.10/ID56` 形式・block_log.rs:374）。
 * 「/ID<数字>」の最初の数字を返す。無ければ null（リンクにしない）。
 */
function lvRunIdOfRef(value) {
  const m = String(value == null ? "" : value).match(/\/ID(\d+)(?!\d)/);
  if (!m) return null;
  const id = parseInt(m[1], 10);
  return Number.isFinite(id) ? id : null;
}

function lvCard(title) {
  const card = el("section", "lv-card");
  if (title) card.appendChild(el("h3", "lv-card-title", title));
  return card;
}

// 1. Log: 入力 / 設定 / 統計 / つながり / 出力トークン（計画 14 §1.3）
function lvRenderLog(data) {
  const body = lvBody();
  const sections = logSections(data);
  if (sections.length === 0) {
    body.appendChild(el("p", "st-note", "表示できる項目がありません"));
    return;
  }
  for (const s of sections) {
    const card = lvCard(s.title);
    for (const [label, value] of s.rows) {
      const row = el("div", "lv-kv");
      row.appendChild(el("span", "lv-kv-label", label));
      // 「つながり」の行は /ID<数字> を含むとき、その run の List へ飛べるようにする（U01）
      const linkId = s.title === "つながり" ? lvRunIdOfRef(value) : null;
      if (linkId != null) {
        const btn = el("button", "lv-kv-link", value);
        btn.type = "button";
        btn.addEventListener("click", () => {
          location.hash = listHash(lvName, linkId, pickView(false, lvView || loadViewMemory()));
        });
        row.appendChild(btn);
      } else {
        row.appendChild(el("span", "lv-kv-value", value));
      }
      card.appendChild(row);
    }
    body.appendChild(card);
  }
}

// 2. Stats: 大きな数字 + メッセージごとの推移グラフ（SVG 手描き）+ 一覧（計画 14 §1.3）
function lvRenderStats(data) {
  const body = lvBody();
  if (data.error) lvSetMsg(String(data.error), true);

  // 大きな数字（入力 / 出力 / 読 / 書 / 費用 / ヒット率）
  const bigs = el("div", "st-bigs");
  for (const c of statsCells(data.totals)) {
    const cell = el("div", "st-big");
    let value = "—";
    if (c.value != null) {
      if (c.kind === "tokens") value = formatTokens(c.value);
      else if (c.kind === "money") value = formatMoney(c.value);
      else value = `${Math.round(c.value)}%`;
    }
    cell.appendChild(el("div", "st-big-value", value));
    cell.appendChild(el("div", "st-big-label", c.label));
    bigs.appendChild(cell);
  }
  body.appendChild(bigs);

  const counts = data.counts || {};
  body.appendChild(
    el(
      "div",
      "lv-sub",
      [
        `user ${lvNum(counts.user)}`,
        `agent ${lvNum(counts.assistant)}`,
        `thinking ${lvNum(counts.thinking)}`,
        `toolCall ${lvNum(counts.toolcall)}`,
      ].join(" · ")
    )
  );

  // メッセージごとの推移グラフ（TUI の 1 / 2 / 3 と同じ 3 種・SVG 手描き）
  const chart = lvCard("メッセージごとの推移");
  const seg = el("div", "dash-seg small lv-chart-seg");
  for (const [key, label] of LV_CHARTS) {
    const b = document.createElement("button");
    b.textContent = label;
    if (key === lvChartMode) b.className = "active";
    b.addEventListener("click", () => {
      lvChartMode = key;
      const box = lvBody();
      box.innerHTML = "";
      lvRenderStats(data);
    });
    seg.appendChild(b);
  }
  chart.appendChild(seg);
  const series = statsSeries(data, lvChartMode);
  const values = series.flatMap((s) => s.values);
  if (values.length === 0) {
    chart.appendChild(el("p", "st-note", "データがありません"));
  } else {
    chart.appendChild(lvLineSvg(series));
    const legend = el("div", "lv-legend");
    for (const s of series) {
      const row = el("div", "lv-leg");
      const dot = el("span", "lv-dot-swatch");
      dot.style.background = s.color;
      row.appendChild(dot);
      row.appendChild(el("span", "lv-leg-name", s.name));
      // 色だけに頼らず、最後の値も文字で出す
      const last = s.values.length > 0 ? lvNum(s.values[s.values.length - 1]) : 0;
      row.appendChild(
        el("span", "lv-leg-num", `最終 ${saiAxisValue(last, s.pct ? "percent" : "tokens")}`)
      );
      legend.appendChild(row);
    }
    chart.appendChild(legend);
  }
  body.appendChild(chart);

  // メッセージごとの一覧（表は横にはみ出す分だけスクロール・計画 14 §1.4）
  const msgs = Array.isArray(data.messages) ? data.messages : [];
  const tblCard = lvCard("メッセージごとの一覧");
  if (msgs.length === 0) {
    tblCard.appendChild(el("p", "st-note", "データがありません"));
  } else {
    const wrap = el("div", "lv-scroll-x");
    const table = el("table", "lv-table");
    const head = el("tr");
    for (const h of ["#", "時刻", "入力", "出力", "読", "書", "ヒット", "費用"]) {
      head.appendChild(el("th", null, h));
    }
    table.appendChild(head);
    for (const m of msgs) {
      const row = el("tr");
      row.appendChild(el("td", null, String(lvNum(m.sequence))));
      row.appendChild(el("td", null, lvClock(m.timestamp)));
      row.appendChild(el("td", null, formatTokens(m.input)));
      row.appendChild(el("td", null, formatTokens(m.output)));
      row.appendChild(el("td", null, formatTokens(m.cache_read)));
      row.appendChild(el("td", null, formatTokens(m.cache_write)));
      row.appendChild(
        el("td", null, `${Math.round(lvNum(m.cache_hit_percent))}%`)
      );
      const cost = m.cost && m.cost.total;
      row.appendChild(el("td", null, cost == null ? "—" : formatMoney(cost)));
      table.appendChild(row);
    }
    wrap.appendChild(table);
    tblCard.appendChild(wrap);
  }
  body.appendChild(tblCard);
}

/** 折れ線グラフ（外部ライブラリなしで SVG を手描く・計画 14 §6）。 */
function lvLineSvg(series) {
  const W = 320;
  const H = 130;
  const L = 40; // 左は目盛りの文字のぶん
  const R = 8;
  const T = 10;
  const B = 20;
  const baseY = H - B;
  const svg = svgEl("svg", {
    viewBox: `0 0 ${W} ${H}`,
    class: "lv-chart",
    role: "img",
    "aria-label": "メッセージごとの推移グラフ。データがありません",
  });
  const all = series.flatMap((s) => s.values);
  if (all.length === 0) return svg;
  const pct = series.some((s) => s.pct);
  const unit = pct ? "percent" : "tokens";
  const dataMax = Math.max(0, ...all);
  const scale = saiNiceScale(dataMax, 3, 0);
  const n = Math.max(1, ...series.map((s) => s.values.length));
  const xOf = (i) => (n <= 1 ? L + (W - L - R) / 2 : L + (i * (W - L - R)) / (n - 1));
  const yOf = (v) =>
    T + (1 - Math.max(0, Math.min(v, scale.max)) / scale.max) * (baseY - T);

  // 縦の目盛りと格子線（0 のところだけ実線）
  for (const t of scale.values) {
    const y = yOf(t).toFixed(2);
    svg.appendChild(
      svgEl("line", {
        x1: String(L),
        y1: y,
        x2: String(W - R),
        y2: y,
        class: t === 0 ? "lv-base" : "lv-grid",
      })
    );
    const label = svgEl("text", {
      x: String(L - 5),
      y,
      dy: "4",
      class: "lv-tick",
      "text-anchor": "end",
    });
    label.textContent = saiAxisValue(t, unit);
    svg.appendChild(label);
  }

  // 折れ線
  for (const s of series) {
    if (!Array.isArray(s.values) || s.values.length === 0) continue;
    const pts = s.values
      .map((v, i) => `${xOf(i).toFixed(1)},${yOf(lvNum(v)).toFixed(1)}`)
      .join(" ");
    svg.appendChild(
      svgEl("polyline", {
        points: pts,
        fill: "none",
        stroke: s.color,
        "stroke-width": "2",
        "stroke-linejoin": "round",
        "stroke-linecap": "round",
      })
    );
  }

  // 横のラベル（メッセージの通し番号。狭い画面では間引く）
  const minGap = saiIsWide() ? 32 : 24;
  const picks = [];
  let lastX = -Infinity;
  for (let i = 0; i < n; i += 1) {
    const x = xOf(i);
    const isEnd = i === 0 || i === n - 1;
    if (x - lastX < (isEnd ? minGap * 0.6 : minGap)) continue;
    lastX = x;
    picks.push(i);
  }
  // 最後（何通目までか）は必ず出す。潰れていたら直前のラベルを溃す
  if (n > 1 && picks[picks.length - 1] !== n - 1) {
    const xLast = xOf(n - 1);
    while (picks.length > 1 && xLast - xOf(picks[picks.length - 1]) < minGap * 0.6) {
      picks.pop();
    }
    picks.push(n - 1);
  }
  for (const i of picks) {
    const t = svgEl("text", {
      x: (i === n - 1 ? W - R : xOf(i)).toFixed(1),
      y: String(baseY + 15),
      class: "lv-tick",
      // 最後の番号は右端で切れないように右寄せにする
      "text-anchor": i === n - 1 ? "end" : "middle",
    });
    t.textContent = String(i + 1);
    svg.appendChild(t);
  }

  // 要点を 1 文で（画面読み上げ用）。最大になるのは何通目か。
  let at = 0;
  if (dataMax > 0) {
    for (const s of series) {
      const i = (s.values || []).findIndex((v) => lvNum(v) === dataMax);
      if (i >= 0) {
        at = i + 1;
        break;
      }
    }
  }
  svg.setAttribute(
    "aria-label",
    `${series.map((s) => s.name).join("・")}の推移（${n} 通）。` +
      (at > 0 ? `最大 ${saiAxisValue(dataMax, unit)}（${at} 通目）。` : "")
  );
  return svg;
}

// 3. History: 種類別のカードと円グラフ（SVG 手描き）+ タイムライン（計画 14 §1.3）
function lvRenderHistory(data) {
  const body = lvBody();
  const head = el("div", "lv-hist-head");
  head.appendChild(el("div", "lv-hist-title", String(data.title || "")));
  head.appendChild(el("div", "lv-sub", String(data.subtitle || "")));
  body.appendChild(head);

  const slices = typeSlices(data.type_tokens);
  const card = lvCard("種類別トークン");
  if (slices.length === 0) {
    card.appendChild(el("p", "st-note", "データがありません"));
  } else {
    const R = 38;
    const C = 2 * Math.PI * R;
    const total = slices.reduce((a, s) => a + s.value, 0);
    // 並びは種類の順番で決まっているので、割合が最大のものは自分で探す
    const biggest = slices.reduce((a, s) => (s.value > a.value ? s : a), slices[0]);
    const wrap = el("div", "st-donut-wrap");
    const svg = svgEl("svg", {
      viewBox: "0 0 100 100",
      class: "st-donut",
      role: "img",
      "aria-label":
        `種類別トークンの割合のドーナツ。合計 ${formatTokens(total)} トークン` +
        `。最も多いのは${biggest.label} ${formatTokens(biggest.value)}` +
        `（${Math.round(biggest.ratio * 100)}%）`,
    });
    let start = 0;
    slices.forEach((s, i) => {
      const len = Math.max(s.ratio * C - 1.5, 0.5); // ギャップは 1.5 単位
      svg.appendChild(
        svgEl("circle", {
          cx: "50",
          cy: "50",
          r: String(R),
          fill: "none",
          stroke: LV_TYPE_COLORS[i % LV_TYPE_COLORS.length],
          "stroke-width": "14",
          "stroke-dasharray": `${len.toFixed(2)} ${(C - len).toFixed(2)}`,
          "stroke-dashoffset": String(-start * C),
          transform: "rotate(-90 50 50)",
        })
      );
      start += s.ratio;
    });
    wrap.appendChild(svg);
    const center = el("div", "st-donut-center");
    center.appendChild(el("strong", "st-donut-total", formatTokens(total)));
    center.appendChild(el("span", "st-donut-cap", "合計"));
    wrap.appendChild(center);
    card.appendChild(wrap);

    const legend = el("div", "lv-legend");
    slices.forEach((s, i) => {
      const row = el("div", "lv-leg");
      const dot = el("span", "lv-dot-swatch");
      dot.style.background = LV_TYPE_COLORS[i % LV_TYPE_COLORS.length];
      row.append(
        dot,
        el("span", "lv-leg-name", s.label),
        el("span", "lv-leg-pct", `${Math.round(s.ratio * 100)}%`),
        el("span", "lv-leg-num", formatTokens(s.value))
      );
      legend.appendChild(row);
    });
    card.appendChild(legend);
  }
  body.appendChild(card);

  const events = Array.isArray(data.events) ? data.events : [];
  const tlCard = lvCard(`タイムライン（${events.length}）`);
  if (events.length === 0) {
    tlCard.appendChild(el("p", "st-note", "データがありません"));
  } else {
    for (const ev of events) {
      const row = el("div", "lv-tl");
      row.appendChild(el("span", "lv-tl-time", lvClock(ev && ev.time)));
      const type = String((ev && ev.type) || "");
      const idx = Math.max(0, LV_TYPES.indexOf(type));
      const chip = el("span", "lv-tl-type", type || "-");
      if (LV_TYPES.includes(type)) chip.style.background = LV_TYPE_COLORS[idx];
      row.appendChild(chip);
      row.appendChild(el("span", "lv-tl-name", String((ev && ev.name) || "")));
      row.appendChild(
        el("span", "lv-tl-tokens", formatTokens(lvNum(ev && ev.tokens)))
      );
      tlCard.appendChild(row);
    }
  }
  body.appendChild(tlCard);
}

/** ISO の時刻を「HH:MM:SS」（タイムラインと一覧で使う）。 */
function lvClock(iso) {
  const d = new Date(String(iso || ""));
  if (isNaN(d)) return "";
  return d.toLocaleTimeString("ja-JP", {
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
  });
}

/**
 * その表示を書き換えられるか（DOM を使わない・tests/test_linkview.js）。
 * task は未実行の todo だけ。message は不可。
 */
function lvCanEdit(view, run) {
  return view === "task" && isTodoRun(run);
}

// 4〜5. task / message: md の整形表示（13 §5 の renderMarkdown をそのまま）
// task（未実行の todo）は「編集」でその場で書き換えて保存できる。
function lvRenderMd(data) {
  lvMdText = (data && data.text) || "";
  lvDrawMd();
}

function lvDrawMd() {
  const body = lvBody();
  if (!body) return;
  body.innerHTML = "";
  if (lvEditing) {
    const ta = document.createElement("textarea");
    ta.className = "md-edit";
    ta.value = lvDraft;
    ta.disabled = lvSaving;
    ta.addEventListener("input", () => { lvDraft = ta.value; });
    body.appendChild(ta);
    const row = el("div", "lv-edit-row");
    const save = el("button", "paper-btn", lvSaving ? "保存中…" : "保存");
    save.disabled = lvSaving;
    save.addEventListener("click", () => void lvSaveMd());
    const cancel = el("button", "paper-btn", "やめる");
    cancel.disabled = lvSaving;
    cancel.addEventListener("click", () => {
      lvEditing = false;
      lvSetMsg("");
      lvDrawMd();
    });
    row.append(save, cancel);
    body.appendChild(row);
    return;
  }
  if (lvCanEdit(lvView, lvRun)) {
    const row = el("div", "lv-edit-row");
    const edit = el("button", "paper-btn", lvMdText ? "編集" : "書く");
    edit.addEventListener("click", () => {
      lvEditing = true;
      lvDraft = lvMdText || "";
      lvDrawMd();
    });
    row.appendChild(edit);
    body.appendChild(row);
  }
  if (lvMdText) {
    const md = el("div", "lv-md");
    // 見出しが 2 つ以上あるときは道具の行（全部たたむ / 全部開く / 目次）も出す
    md.innerHTML = renderMarkdown(lvMdText, { toolbar: true });
    body.appendChild(md);
  } else {
    body.appendChild(el("p", "st-note", "まだありません"));
  }
}

async function lvSaveMd() {
  if (lvSaving) return;
  const seq = lvSeq;
  const view = lvView;
  lvSaving = true;
  lvSetMsg("");
  lvDrawMd();
  try {
    const res = await apiFetch(
      `/api/projects/${encodeURIComponent(lvName)}/runs/${lvRunId}/md/${view}`,
      {
        method: "PUT",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ text: lvDraft }),
      }
    );
    const data = await res.json().catch(() => ({}));
    if (!res.ok) throw new Error(data.detail || "保存できませんでした");
    if (seq !== lvSeq) return; // その間に別の表示を開いた
    lvSaving = false;
    lvEditing = false;
    lvSetMsg("保存しました");
    lvRenderMd(data);
  } catch (e) {
    if (seq !== lvSeq) return;
    // 書きかけは消さずに残す（直してもう一度押せる）
    lvSaving = false;
    lvSetMsg((e && e.message) || "保存できませんでした", true);
    lvDrawMd();
  }
}

/** 成果物の表示名 `<waggle_token>/<file_name>`。 */
function lvWaggleName(item) {
  return `${item.token || ""}/${item.label || ""}`;
}

/** ＋: この文書を task_text に貼った状態で task.md の作成フォームを開く。 */
function lvWaggleAdd(item) {
  const b = el("button", "lv-waggle-add", "＋");
  b.type = "button";
  b.setAttribute("aria-label", "この文書で task.md を作る");
  b.title = "この文書で task.md を作る";
  b.addEventListener("click", (e) => {
    e.stopPropagation();
    if (typeof cfOpen === "function") void cfOpen({ ref: item.ref });
  });
  return b;
}

// 7. waggle: 成果物の一覧 → タップで本文、‹ で一覧へ（計画 14 §1.3）
async function lvLoadWaggle(seq) {
  const body = lvBody();
  const base = `/api/projects/${encodeURIComponent(lvName)}/runs/${lvRunId}/waggle`;
  try {
    if (lvWaggleToken) {
      // 本文（開いているもの）
      const data = await lvFetchJson(
        `${base}/${encodeURIComponent(lvWaggleToken)}`
      );
      if (seq !== lvSeq) return;
      window.SaiShell.ok();
      body.innerHTML = "";
      const back = document.createElement("button");
      back.className = "lv-md-back";
      back.textContent = "‹ 一覧";
      back.addEventListener("click", () => {
        lvWaggleToken = null;
        const s = ++lvSeq;
        void lvLoadContent(s);
      });
      body.appendChild(back);
      const item = (lvWaggleList || []).find((w) => w.token === lvWaggleToken);
      const title = el("div", "lv-waggle-title", item ? lvWaggleName(item) : String(data.label || lvWaggleToken));
      if (item) title.appendChild(lvWaggleAdd(item));
      body.appendChild(title);
      body.appendChild(el("div", "lv-waggle-text", ""));
      body.lastChild.innerHTML = renderMarkdown(data.text || "", { toolbar: true });
      return;
    }
    // 一覧（まだ読んでいなければ）
    if (!Array.isArray(lvWaggleList)) {
      const data = await lvFetchJson(base);
      if (seq !== lvSeq) return;
      lvWaggleList = Array.isArray(data.items) ? data.items : [];
      window.SaiShell.ok();
    }
    // 1 件だけなら直接開く（計画 14 §1.3）
    if (waggleDirect(lvWaggleList)) {
      lvWaggleToken = lvWaggleList[0].token;
      await lvLoadWaggle(seq);
      return;
    }
    if (seq !== lvSeq) return;
    body.innerHTML = "";
    if (lvWaggleList.length === 0) {
      body.appendChild(el("p", "st-note", "成果物はありません"));
      return;
    }
    for (const item of lvWaggleList) {
      const line = el("div", "lv-waggle-line");
      const row = document.createElement("button");
      row.className = "lv-waggle-row";
      row.appendChild(el("span", "lv-waggle-label", lvWaggleName(item)));
      row.appendChild(
        el("span", "lv-waggle-desc", String(item.description || ""))
      );
      row.addEventListener("click", () => {
        lvWaggleToken = item.token;
        const s = ++lvSeq;
        void lvLoadContent(s);
      });
      line.append(row, lvWaggleAdd(item));
      body.appendChild(line);
    }
  } catch (e) {
    if (seq !== lvSeq) return;
    window.SaiShell.fail();
    const msg = (e && e.message) || "読み込めませんでした";
    lvSetMsg(msg, true);
    body.innerHTML = "";
    body.appendChild(el("p", "st-note", msg));
  }
}

// --- 引き出し式ツリー（計画 14 §1.2） ---

function lvSetDrawer(open) {
  const wrap = document.getElementById("lv-drawer-wrap");
  if (!wrap) return;
  // 広い画面ではツリーは左の列に常に出ている（引き出しは使わない）
  if (lvWideOn === true) {
    lvRenderDrawer();
    wrap.hidden = true;
    return;
  }
  if (open) lvRenderDrawer();
  wrap.hidden = !open;
}

function lvRenderDrawer() {
  const nav = document.getElementById("lv-drawer");
  if (!nav) return;
  nav.innerHTML = "";
  // ツリーの行より先に「実行［＋］［↻］」の見出しの行を出す（U01 §4）。
  // ツリーが空・読み込み中・読み込み失敗のときも残す（↻ で読み直せるように）。
  nav.appendChild(lvSideHead());
  const rows = flattenTree(lvTree || [], lvFolds);
  if (rows.length === 0) {
    nav.appendChild(
      el("p", "lv-drawer-note", lvTreeError || (lvTree ? "実行がありません" : "読み込み中…"))
    );
    return;
  }
  for (const r of rows) {
    if (r.type === "folder") {
      const b = document.createElement("button");
      b.className = "lv-fold";
      b.style.paddingLeft = `${10 + r.depth * 14}px`;
      b.appendChild(el("span", "lv-fold-arrow", r.open ? "▾" : "▸"));
      b.appendChild(el("span", "lv-fold-label", r.label));
      b.addEventListener("click", () => {
        // 覚えが無ければ「全部開く」から 1 つ閉じる形に組み替える
        const base = lvFolds === null ? allFolderIds(lvTree || []) : lvFolds;
        lvFolds = toggleFold(base, r.id);
        lvSaveFolds(lvName, lvFolds);
        lvRenderDrawer();
      });
      nav.appendChild(b);
    } else {
      const b = document.createElement("button");
      b.className = "lv-run-item" + (r.runId === lvRunId ? " active" : "");
      b.style.paddingLeft = `${14 + r.depth * 14}px`;
      const dot = document.createElement("span");
      dot.className = `lv-dot lv-${(r.lane || r.status || "none").replace(/[^a-z]/gi, "") || "none"}`;
      dot.setAttribute("aria-hidden", "true");
      b.appendChild(dot);
      b.appendChild(el("span", "lv-run-item-label", r.label));
      b.addEventListener("click", () => {
        // run を選ぶと閉じ，最後に見ていた表示のまま中身だけ変わる（計画 14 §1.3）
        const view = pickView(isTodoRun(r), lvView || loadViewMemory());
        location.hash = listHash(lvName, r.runId, view);
        lvSetDrawer(false);
      });
      nav.appendChild(b);
    }
  }
}

// --- サイドバーの見出しの行（実行・＋（task.md の作成）・↻（読み直し）・U01 §4） ---

// 見出しの行を 1 本作る。毎回作り直されるので、ボタンは作るたびにイベントを付ける。
// 狭い画面では引き出しの中、広い画面では左の列の一番上に出る。
function lvSideHead() {
  const row = el("div", "lv-side-head");
  row.appendChild(el("span", "lv-side-label", "実行"));
  const add = el("button", "lv-side-btn", "＋");
  add.type = "button";
  add.setAttribute("aria-label", "タスクを作成");
  add.title = "task.md を作る";
  add.addEventListener("click", () => {
    // 作成フォームは cardform.js のグローバル関数
    if (typeof cfOpen === "function") void cfOpen();
  });
  const reload = el("button", "lv-side-btn", "↻");
  reload.type = "button";
  reload.setAttribute("aria-label", "再読み込み");
  reload.title = "ツリーを読み直す";
  reload.addEventListener("click", () => { void lvReload(); });
  row.append(add, reload);
  return row;
}

// --- Dashboard | Workflow | List | 設定 の切り替え帯（共有契約 C-F7） ---

function fillLinkSeg(containerId, active, name) {
  const box = document.getElementById(containerId);
  if (!box) return;
  box.innerHTML = "";
  const seg = el("div", "dash-seg papers-seg");
  for (const [key, label] of LV_MODES) {
    const b = document.createElement("button");
    b.textContent = label;
    if (key === active) b.className = "active";
    b.addEventListener("click", () => {
      const hash = modeHash(name, key);
      saveMode(name, key); // 押した方を覚える（次に開いたときも同じ・C-F7）
      if (location.hash !== hash) location.hash = hash;
    });
    seg.appendChild(b);
  }
  box.appendChild(seg);
}

// --- 広い画面ではツリーを左に常に出す（style.css の .sai-split を使い回す） ---

let lvWideOn = null;   // いま広い画面用の枠を作っているか

/** 画面の幅に応じて、ツリーを左の列に出すか引き出しに戻すか。 */
function lvApplyWide() {
  const sec = document.getElementById("view-list");
  if (!sec) return;
  const wide = typeof saiIsWide === "function" ? saiIsWide() : false;
  if (wide === lvWideOn) return;
  lvWideOn = wide;

  const appbar = sec.querySelector(".appbar");
  const head = document.getElementById("lv-head");
  const body = document.getElementById("lv-body");
  const wrap = document.getElementById("lv-drawer-wrap");
  const nav = document.getElementById("lv-drawer");
  const menu = document.getElementById("list-menu");
  const old = document.getElementById("lv-split");
  if (old) old.remove();

  if (!wide) {
    // 元の位置へ戻す（引き出しに戻す）
    for (const node of [head, body]) {
      if (node && node.parentElement !== sec) sec.appendChild(node);
    }
    // ツリーは引き出し（#lv-drawer-wrap）の中へ戻す（外へ出すと引き出しが出なくなる）
    if (nav && wrap && nav.parentElement !== wrap) wrap.appendChild(nav);
    if (nav) {
      nav.style.width = "";
      nav.style.borderRight = "";
      nav.style.boxShadow = "";
      nav.style.padding = "";
    }
    if (wrap) wrap.hidden = true;
    if (menu) menu.hidden = false;
    return;
  }

  // 広い画面：左にツリー・右に中身（.sai-split の中の見せる側を両方出す）
  if (!nav || !head || !body) return;
  const split = document.createElement("div");
  split.className = "sai-split";
  split.id = "lv-split";
  split.style.flex = "1";
  split.style.minHeight = "0";
  const left = document.createElement("div");
  left.className = "sai-split-list";
  const right = document.createElement("div");
  right.className = "sai-split-main";
  left.appendChild(nav);
  right.append(head, body);
  split.append(left, right);
  // 見出し（.appbar）と接続切れの帯の後ろへ入れる。一番上には出さない（U01 §3）
  const offline = sec.querySelector(".offline-bar");
  const anchor = offline || appbar;
  if (anchor && anchor.parentElement === sec) anchor.after(split);
  else sec.insertBefore(split, sec.firstChild);
  // 引き出し用の枠は隠す（ツリーは左の列にあるので）
  if (wrap) wrap.hidden = true;
  const mask = document.getElementById("lv-drawer-mask");
  if (mask) mask.hidden = true;
  if (menu) menu.hidden = true;
  // 中の枠が二重に線を出さないように
  nav.style.width = "100%";
  nav.style.borderRight = "none";
  nav.style.boxShadow = "none";
  nav.style.padding = "0";
  lvRenderDrawer();
}

// --- 画面の登録（ブラウザでのみ・計画 14 §6） ---

if (typeof window !== "undefined") {
  (window.SaiRoutes = window.SaiRoutes || []).push({
    // List（run 未選択・run 選択済みの 2 つをまとめてこの画面が受ける）
    match(hash) {
      return parseListHash(hash);
    },
    view: "view-list",
    tab: "project",
    hideTabbar: false,
    interval: 0, // 定期更新はしない（↻ と開き直しで最新にする・計画 14 §1.2）
    async open(params) {
      await openList(params);
    },
    async poll() {},
  });

  // ☰: ツリーを開く（未選択のときは最初から開いている）
  const menu = document.getElementById("list-menu");
  if (menu) menu.addEventListener("click", () => lvSetDrawer(true));
  // ‹: プロジェクト一覧へ戻る（4 画面共通・U01 §3）
  const back = document.getElementById("list-back");
  if (back) back.addEventListener("click", () => { location.hash = "#/projects"; });
  // 暗い部分をタップすると閉じる
  const mask = document.getElementById("lv-drawer-mask");
  if (mask) mask.addEventListener("click", () => lvSetDrawer(false));
  // 画面幅が変わったら広い画面用の並びを組み立て直す
  window.addEventListener("resize", () => {
    const wasWide = lvWideOn;
    lvApplyWide();
    if (wasWide !== lvWideOn) lvSetDrawer(lvRunId == null || lvWideOn === true);
  });
  // ＋（task.md の作成フォーム）はサイドバーの見出しの行の中（lvRenderDrawer・U01 §4）
}

// --- node からの読み込み（tests/test_linkview.js・計画 14 §6） ---
if (typeof module !== "undefined" && module.exports) {
  module.exports = {
    listHash,
    parseListHash,
    sessionHash,
    modeHash,
    lvMode,
    LV_MODES,
    saveMode,
    wantsListView,
    rememberedMode,
    saveViewMemory,
    loadViewMemory,
    lvSaveFolds,
    lvLoadFolds,
    toggleFold,
    viewTabs,
    pickView,
    isTodoRun,
    lvCanEdit,
    flattenTree,
    findRun,
    runLanes,
    advanceAction,
    allFolderIds,
    logSections,
    lvRunIdOfRef,
    statsCells,
    statsSeries,
    typeSlices,
    waggleDirect,
    lvScheduledAt,
  };
}
