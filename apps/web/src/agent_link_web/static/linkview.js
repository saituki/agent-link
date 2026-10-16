// agent-link-web の List 画面（実装計画 14）の本体。
//
// 画面は 1 つ（住所は 2 つ）:
//   List（run 未選択）  #/project/<name>/list
//   List（run 選択済み） #/project/<name>/list/<run_id>/<表示>
// Project 上部の Dashboard | List の切り替えもこのファイルが持つ
// （Dashboard 側の帯の描き方も fillLinkSeg としてここに置く・計画 14 §1.1）。
// ツリーは agent-link TUI の左サイドバーと同じ並び（`GET …/tree` のそのまま・計画 14 §1.2）。
// run の中身は 8 つの表示（Log / Stats / History / task / message / report / system / waggle）で、
// 最後に見ていた表示をプロジェクトをまたいで覚える（別の run に切り替えても同じ表示・§1.3）。
// データはすべて /api/projects/<name>/… 経由で，agent-link-web 自身は DB を開かない（00 §2）。
// グラフ（Stats の推移・History の円）は SVG を手描きで作る（外部ライブラリは入れない・00 §2）。
// md の整形は markdown.js の renderMarkdown，数字・SVG の作りは stats.js / project.js の
// グローバル関数を使い回す（13 §5・14 §6）。
// DOM を使わない小さな関数は末尾で node に渡す（tests/test_linkview.js）。
"use strict";

// --- 定数 ---

// run の中身の表示の値（住所の <view> が受け付ける値）。帯のタブには出さない値も含む。
// stats・history は Summary（log）へ、waggle（トークン無し）は Outputs へ移す（openList）。
// session は Agent タブのメッセージ画面へ飛ぶので、sessionHash が作る（住所だけの入口）。
const LV_VIEWS = [
  ["log", "Summary"],
  ["stats", "Stats"],
  ["history", "History"],
  ["outputs", "Outputs"],
  ["task", "task"],
  ["message", "message"],
  ["report", "Report"],
  // 固定部分（system prompt。起動時に run フォルダの system-prompt.md から渡す・U6）
  ["system", "system"],
  ["waggle", "waggle"],
];
// 帯のタブ（Summary | Outputs の 2 つだけ）。並びと表示名はここ 1 か所。
const LV_TABS = [
  ["log", "Summary"],
  ["outputs", "Outputs"],
];
// Outputs に入る表示（成果物と md）。それ以外は Summary（log）に入る。
const LV_OUTPUT_VIEWS = ["outputs", "task", "message", "report", "system", "waggle"];
// Project 上部の 4 つの切り替え（共有契約 C-F7）。
// 並びと表示名はここ 1 か所。dataset の値は変えない。
const LV_MODES = [
  ["dashboard", "Dashboard"],
  ["workflow", "Workflow"],
  ["list", "List"],
  ["defs", "Settings"],
  // 専用の画面（プロジェクトごとの命令・共有契約 C-F7）
  ["sessions", "Session search"],
];
// 最後に見ていた Project の画面をプロジェクトごとに覚える（localStorage・計画 14 §6・C-F7）
const LV_MODE_KEY = "agent_link_web_link_mode";
// 最後に見ていた表示を覚える（localStorage・計画 14 §1.3）
const LV_VIEW_KEY = "agent_link_web_list_view";
// Summary の下の切り替え（Per-message table かタイムライン）を覚える（localStorage）
const LV_DETAIL_KEY = "agent_link_web_list_detail";
// ツリーの開閉をプロジェクトごとに覚える（localStorage・計画 14 §1.2）
const LV_FOLDS_KEY = "agent_link_web_list_folds";
// History の種類別の色（5 色・project.js のドーナツと同じ考え方）。色は CSS 変数。
const LV_TYPE_COLORS = ["var(--series-1)", "var(--series-2)", "var(--series-3)", "var(--series-4)", "var(--series-5)", "var(--series-6)"];
// History の種類（表示順）
const LV_TYPES = ["user", "agent", "toolCall", "toolResult", "thinking"];
// Stats のグラフの種類（TUI の 1 / 2 / 3 と同じ・4 つ目の「表」は常に下に出す・計画 14 §1.3）
const LV_CHARTS = [
  ["turn", "Turn"],
  ["cumpct", "Cumulative %"],
  ["cum", "Cumulative"],
];
// レーン名 → 用語の key（見出しの横に置く用語の札用）
const LV_TERMS = { todo: "todo", running: "running", review: "review", workflow: "workflow" };

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
    case "sessions": return `${base}/sessions`;
    default: return base;
  }
}

/**
 * List の住所（run と表示は付けなくてもよい・計画 14 §6）。
 * waggle の表示だけは、開く文書の waggle トークンも付けられる（`…/<run_id>/waggle/<トークン>`）。
 */
function listHash(name, runId, view, waggleToken) {
  const base = `#/project/${encodeURIComponent(name)}/list`;
  if (runId == null) return base;
  const head = `${base}/${encodeURIComponent(String(runId))}/${encodeURIComponent(view || "log")}`;
  return view === "waggle" && waggleToken ? `${head}/${encodeURIComponent(waggleToken)}` : head;
}

/**
 * List の住所を読む。List でなければ null。
 * run_id が数字でない住所は「run 未選択」として扱う（Agent 画面に落ちないため）。
 */
function parseListHash(hash) {
  const m = String(hash || "").match(
    /^#\/project\/(.+?)\/list(?:\/([^/?#]+)\/([^/?#]+)(?:\/([^/?#]+))?)?\/?$/
  );
  if (!m) return null;
  const name = decodeURIComponent(m[1]);
  if (m[2] == null || !/^\d+$/.test(m[2])) return { name, runId: null, view: null };
  const view = decodeURIComponent(m[3] || "");
  const out = { name, runId: Number(m[2]), view };
  // waggle の表示だけトークンを持てる（形が合わないものは無いことにする）
  const token = m[4] ? decodeURIComponent(m[4]) : "";
  if (view === "waggle" && /^[A-Za-z0-9]{8}$/.test(token)) out.waggleToken = token;
  return out;
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

/** 表示が光らせる帯のタブ（"log" か "outputs"）。古い値（stats など）もここで直す。 */
function tabOf(view) {
  return LV_OUTPUT_VIEWS.includes(view) ? "outputs" : "log";
}

/** 覚えている表示（Summary か Outputs の 2 つだけ）。 */
function loadViewMemory() {
  try {
    return tabOf(localStorage.getItem(LV_VIEW_KEY));
  } catch {
    return "log";
  }
}

/** 最後に見ていた表示を覚える（todo run では呼ばない・計画 14 §1.3）。Summary か Outputs に直して覚える。 */
function saveViewMemory(view) {
  try {
    localStorage.setItem(LV_VIEW_KEY, tabOf(view));
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

/** 帯のタブ（Summary・Outputs の 2 つ）。todo run は Outputs だけ選べる（計画 14 §1.3）。 */
function viewTabs(todo) {
  return LV_TABS.map(([key, label]) => ({
    key,
    label,
    ok: !todo || key === "outputs",
  }));
}

/**
 * 開く表示を決める。todo run では task に固定する（表示は URL に残し，記憶は変えない・計画 14 §1.3）。
 * stats・history は Summary（log）に直す。それ以外は覚えていた表示をそのまま使う。
 */
function pickView(todo, remembered) {
  if (todo) return "task";
  if (remembered === "stats" || remembered === "history") return "log";
  return LV_VIEWS.some((x) => x[0] === remembered) ? remembered : "log";
}

/** todo の run（lane / status が todo）か（▶ 実行と task 固定の判定・計画 14 §1.3）。 */
function isTodoRun(run) {
  if (!run || typeof run !== "object") return false;
  return run.lane === "todo" || run.status === "todo";
}

/**
 * エージェントが起こすカードか（`executer` が空でなく `human` でない・U1・U12）。
 * 人が起こす道（▶ 実行）では動かさないので、ボタンを出さない。
 * 古い書き出しに残った `professor` も同じく人が起こさないカードとして扱う。
 */
function isAgentRun(run) {
  if (!run || typeof run !== "object") return false;
  const executer = String(run.executer || "").trim();
  return executer !== "" && executer !== "human";
}

/**
 * ツリーを「今開いているフォルダ・run だけ」並べた行に組み替える（描き方の元・計画 14 §1.2）。
 * open === null は「全部開く」（探す処理用）。depth は行ごとの字下げの段。
 * run は子（children）を持てる。開閉の id は `run:<run_id>`（行の id と同じ）。
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
        const id = `run:${node.run_id}`;
        const kids = Array.isArray(node.children) ? node.children : [];
        const isOpen = openSet === null || openSet.has(id);
        rows.push({
          type: "run",
          id,
          label: String(node.label || `ID${node.run_id}`),
          depth,
          runId: Number(node.run_id),
          lane: String(node.lane || ""),
          status: String(node.status || ""),
          executer: String(node.executer || ""),
          hasChildren: kids.length > 0,
          open: isOpen,
        });
        if (isOpen) walk(kids, depth + 1);
      }
    }
  };
  walk(items, 0);
  return rows;
}

/** ツリーの中から run を 1 つ探す（入れ子の中も探す。同じ run は 1 か所だけに出る）。 */
function findRun(items, runId) {
  const want = Number(runId);
  if (!Number.isFinite(want)) return null;
  for (const row of flattenTree(items, null)) {
    if (row.type === "run" && row.runId === want) return row;
  }
  return null;
}

/** その run が出る lane を集める（同じ run は 1 か所だけに出るので、多くて 1 つ）。 */
function runLanes(items, runId) {
  const run = findRun(items, runId);
  return new Set(run && run.lane ? [run.lane] : []);
}

/** 移動ボタンの出分と表示（review → Workflow）。review 以外は null。 */
function advanceAction(items, runId) {
  if (runLanes(items, runId).has("review")) {
    return { from: "review", to: "workflow", label: "→ Workflow" };
  }
  return null;
}

/**
 * 覚えが無いときに開くフォルダ id（todo・running・review と、一番新しい日付フォルダ 1 つ）。
 * 子を持つ run は閉じておく。日付フォルダの id は数字 6 桁（yymmdd）。
 */
function defaultOpenIds(items) {
  const ids = ["todo", "running", "review"];
  let latest = "";
  for (const node of items || []) {
    if (!node || node.type !== "folder") continue;
    const id = String(node.id || node.label || "");
    if (/^\d{6}$/.test(id) && id > latest) latest = id;
  }
  if (latest) ids.push(latest);
  return ids;
}

/** Summary（run の log 表示）を「節 × 行（ラベル / 値の文字列）」に組み替える（計画 14 §1.3）。 */
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

  // 2. 設定（モデル / thinking / 時刻 / commit）
  const conf = [];
  if (data.description) conf.push(["Description", text(data.description)]);
  if (data.user_block_description) conf.push(["Description", text(data.user_block_description)]);
  if (data.model) conf.push(["Model", text(data.model)]);
  if (data.thinking) conf.push(["thinking", text(data.thinking)]);
  if (data.started_at) conf.push(["Started", text(data.started_at)]);
  if (data.finished_at) conf.push(["Finished", text(data.finished_at)]);
  if (data.git_commit_before || data.git_commit_after) {
    const before = text(data.git_commit_before) || "-";
    const after = text(data.git_commit_after);
    conf.push(["commit", after && after !== before ? `${before} → ${after}` : before]);
  }
  if (conf.length > 0) out.push({ title: "Settings", rows: conf });

  // 3. Connection（時系列順・行ごとに run へ飛べる）。統計は Stats の表示へ移した
  const items = connectionItems(data.connections);
  if (items.length > 0) out.push({ title: "Connection", items });

  // 4. 出力トークン（成果物）
  const outs = (data.output_waggle_tokens || []).map(text).filter(Boolean);
  if (outs.length > 0) out.push({ title: "Outputs", rows: outs.map((t) => ["waggle", t]) });

  return out;
}

/**
 * Connection の行（`connections` は Rust が作られた順で返す・時刻は出さない）。
 *   head: `from [task_text] ID86/kantoku`（押すとその run の表示へ飛ぶ）
 *   sub:  task_text は `waggle:<token> file:<path>`、task_create は説明、task_output は無し
 */
function connectionItems(list) {
  const out = [];
  for (const c of Array.isArray(list) ? list : []) {
    if (!c || typeof c !== "object" || c.run_id == null) continue;
    const head = `${c.direction} [${c.kind}] ID${c.run_id}/${c.block_name || ""}`;
    let sub = "";
    if (c.kind === "task_text" && c.waggle_token) {
      sub = `waggle:${c.waggle_token}${c.file_path ? ` file:${c.file_path}` : ""}`;
    } else if (c.kind === "task_create" && c.description) {
      sub = String(c.description);
    }
    out.push({ head, sub, runId: Number(c.run_id) });
  }
  return out;
}

/** Stats の表示の先頭に出す session の数字（turn / tool / token・log の表示から移した）。 */
function sessionStatRows(st) {
  if (!st || typeof st !== "object") return [];
  const rows = [];
  const text = (v) => String(v);
  if (st.turn_count != null) rows.push(["Turns", text(st.turn_count)]);
  if (st.tool_call_count != null) rows.push(["Tool calls", text(st.tool_call_count)]);
  for (const [key, label] of [
    ["input_tokens", "Input tokens"],
    ["output_tokens", "Output tokens"],
    ["thinking_tokens", "Thinking tokens"],
    ["total_tokens", "Total tokens"],
  ]) {
    if (st[key] != null) rows.push([label, text(st[key])]);
  }
  return rows;
}

/** Stats の大きな数字（入力 / 出力 / 読 / 書 / 費用 / ヒット率・計画 14 §1.3）。 */
function statsCells(totals) {
  const t = totals && typeof totals === "object" ? totals : {};
  const cost = t.cost && typeof t.cost === "object" ? t.cost : {};
  const denom = lvNum(t.cache_read) + lvNum(t.input);
  return [
    { label: "Input", value: lvNum(t.input), kind: "tokens" },
    { label: "Output", value: lvNum(t.output), kind: "tokens" },
    { label: "Cache read", value: lvNum(t.cache_read), kind: "tokens" },
    { label: "Cache write", value: lvNum(t.cache_write), kind: "tokens" },
    {
      label: "Cost",
      value: cost.total == null ? null : Number(cost.total),
      kind: "money",
    },
    {
      label: "Hit rate",
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
        name: "Hit rate per turn",
        color: "var(--series-1)",
        pct: true,
        values: line(msgs.map((m) => (m || {}).cache_hit_percent)),
      },
    ];
  }
  if (mode === "cumpct") {
    return [
      {
        name: "Cumulative hit rate",
        color: "var(--series-2)",
        pct: true,
        values: line(cum.cum_hit_percent),
      },
    ];
  }
  return [
    { name: "Input", color: "var(--series-1)", pct: false, values: line(cum.cum_input) },
    { name: "Cache read", color: "var(--series-2)", pct: false, values: line(cum.cum_cache_read) },
    { name: "Cache write", color: "var(--series-3)", pct: false, values: line(cum.cum_cache_write) },
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

/** task.md の frontmatter から `description:` を読む（無ければ空）。Outputs の task の行の説明。 */
function lvTaskDescription(text) {
  const fm = String(text || "").match(/^---\n([\s\S]*?)^---[ \t]*$/m);
  const m = fm ? fm[1].match(/^description:[ \t]*(.*)$/m) : null;
  return m ? m[1].trim() : "";
}

// ===========================================================================
// ここから下はブラウザだけで動く部分（node のテストでは読まれない）
// ===========================================================================

// --- 画面の状態 ---

let lvName = "";           // 開いているプロジェクト名
let lvRunId = null;        // 選んでいる run（null＝未選択）
let lvView = null;         // 今見ている表示（LV_VIEWS のどれか）
let lvRun = null;          // 選んでいる run のツリーの行（label / lane / status）
let lvTree = null;         // ツリー（GET …/tree の items・null＝未読）
let lvTreeError = "";      // ツリーを読めなかったときの 1 行
let lvFolds = null;        // 開いているフォルダ・run（null＝覚えが無い＝defaultOpenIds）
let lvMsg = "";            // 見出し下に出す 1 行
let lvMsgErr = false;      // その行を赤で出すか（失敗の理由）
let lvBusy = false;        // ▶ 実行の送信中
let lvSeq = 0;             // 画面が入れ替わった合図（古い応答を捨てる）
let lvWaggleList = null;   // 成果物の一覧（同じ run の間だけ使い回す・Outputs と文書の ＋ で使う）
let lvWaggleToken = null;  // 開いている成果物（住所のトークン・null＝文書を開いていない）
let lvChartMode = "turn";  // Summary のグラフの種類
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
  if (!res.ok) throw new Error(body.detail || "Could not load.");
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
    lvTreeError = (e && e.message) || "Could not load the tree.";
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
    // session は List には描かず，Agent タブのメッセージ画面へ移る（U11）。
    // location.replace にするのは，戻ると同じ session 住所に戻ってまた移ってしまうため。
    // todo run では session を選べないので、下の pickView で task に固定する。
    if (params.view === "session" && !todo) {
      location.replace(sessionHash(name, runId));
      return;
    }
    const wanted =
      params.view && LV_VIEWS.some((v) => v[0] === params.view)
        ? params.view
        : loadViewMemory();
    view = pickView(todo, wanted);
    // todo でも Outputs を明示した住所（‹ Outputs・Outputs タブ）は一覧を出す（task 以外は押せない）
    if (todo && params.view === "outputs") view = "outputs";
    // 文書の住所（waggle）にトークンが無ければ、成果物の一覧（Outputs）へ
    if (view === "waggle" && !(params.waggleToken)) view = "outputs";
    // 覚えるのは「選んだ表示」だけ（todo で task に固定した分は覚えず・計画 14 §1.3）。
    // 覚えるのは Summary か Outputs の 2 つ（saveViewMemory が直す）。
    if (!todo) saveViewMemory(view);
  }

  // 住所を正しい形に直す（数字でない run_id・todo run の task 以外・古い表示の値など）
  const wantToken = view === "waggle" && params && params.waggleToken ? params.waggleToken : null;
  const target = listHash(name, runId, view, wantToken);
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
  }
  // 住所に文書のトークンがあれば、その文書を開く（Outputs の一覧・md の中の waggle リンクから来た）。
  // 住所からトークンが消えたら（‹ Outputs で戻ったとき）、その文書は閉じる。
  lvWaggleToken = wantToken;
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
        el("p", "st-note", "Choose a run in the tree on the left to see its contents.")
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
    b.className = "lv-tab" + (t.key === tabOf(lvView) ? " active" : "");
    b.textContent = t.label;
    b.disabled = !t.ok;
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
    if (isAgentRun(lvRun)) {
      // エージェントが起こすカードは人が動かさない（理由を出す・U1）。
      const note = document.createElement("span");
      note.className = "lv-run-note";
      note.appendChild(document.createTextNode("Started by an agent "));
      // 用語集に `agent` の項目があれば札を付け、無ければ付けない（U12）。
      if (
        typeof saiTerm === "function" &&
        typeof saiTermText === "function" &&
        saiTermText("agent")
      ) {
        note.appendChild(saiTerm("agent", "Started by an agent"));
      }
      box.appendChild(note);
    } else {
      add("▶ Run", lvRunTodo);
    }
    add("⏰ Schedule", lvSchedule, "ghost");
  } else {
    // review の run だけ review → Workflow の移動を出す
    const adv = advanceAction(lvTree, lvRunId);
    if (adv) add(adv.label, () => lvAdvance(adv));
  }
  // 動いている run は消せない（実行中のため）
  if (!["running", "pending"].includes(String(lvRun.status || lvRun.lane || ""))) {
    add("🗑 Delete", lvDelete, "ghost");
  }
  // 会話（Agent タブのメッセージ画面）への入口。todo にはまだ会話が無いので出さない
  if (!isTodoRun(lvRun)) {
    add("💬 Chat", () => {
      location.hash = sessionHash(lvName, lvRunId);
    }, "ghost");
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
    title: "Scheduled time",
    body: "The daemon on the Mac runs this card at this time. Leave empty to cancel the schedule.",
    value: current,
    placeholder: "e.g. 2026-10-01 09:00",
    ok: "Schedule",
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
    if (!res.ok) throw new Error(out.detail || "Could not schedule.");
    window.SaiShell.ok();
    lvSetMsg(out.scheduled_at ? `Scheduled for ${out.scheduled_at}.` : "Schedule cancelled.", false);
  } catch (e) {
    window.SaiShell.fail();
    lvSetMsg((e && e.message) || "Could not schedule.", true);
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
    title: "Delete",
    body: `Delete ${label}? Its subtask runs and output folders are deleted too. This cannot be undone.`,
    ok: "Delete",
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
    if (!res.ok) throw new Error(out.detail || "Could not delete.");
    window.SaiShell.ok();
    saiToast(`Deleted ${label}.`);
    location.hash = listHash(lvName, null, null);
  } catch (e) {
    window.SaiShell.fail();
    lvSetMsg((e && e.message) || "Could not delete.", true);
  } finally {
    lvBusy = false;
    lvRenderActions();
  }
}

/** review → Workflow を iPhone から移す（POST …/runs/{run_id}/advance）。 */
async function lvAdvance(adv) {
  if (lvBusy || lvRunId == null || !adv) return;
  const label = (lvRun && lvRun.label) || `ID${lvRunId}`;
  if (!confirm(`Move ${label} to ${adv.to}?`)) return;
  lvBusy = true;
  lvRenderActions();
  try {
    const res = await apiFetch(
      `/api/projects/${encodeURIComponent(lvName)}/runs/${lvRunId}/advance`,
      { method: "POST" }
    );
    const out = await res.json().catch(() => ({}));
    if (!res.ok) throw new Error(out.detail || "Could not move.");
    window.SaiShell.ok();
    lvSetMsg(`Moved ID${lvRunId} to Workflow (${out.to || adv.to}).`, false);
    await lvLoadTree(lvSeq); // 置き場所が変わったのでツリーを取り直す
    lvRun = findRun(lvTree, lvRunId);
    lvRenderHead();
  } catch (e) {
    window.SaiShell.fail();
    lvSetMsg((e && e.message) || "Could not move.", true);
  } finally {
    lvBusy = false;
    lvRenderActions();
  }
}

async function lvRunTodo() {
  if (lvBusy || lvRunId == null) return;
  const label = (lvRun && lvRun.label) || `ID${lvRunId}`;
  // 確認は既存の ▶ 実行と同じ動き（実行はエージェントを動かすため必ず聞く・計画 15）
  if (!confirm(`Run this card?\n\n${label}`)) return;
  lvBusy = true;
  lvRenderActions();
  try {
    const res = await apiFetch(
      `/api/projects/${encodeURIComponent(lvName)}/runs/${lvRunId}/start`,
      { method: "POST" }
    );
    const out = await res.json().catch(() => ({}));
    if (!res.ok) throw new Error(out.detail || "Could not send.");
    window.SaiShell.ok();
    lvSetMsg(out.message || "Run requested.", false);
    await lvLoadTree(lvSeq); // todo から動いたはずなのでツリーを取り直す
    lvRenderHead();
  } catch (e) {
    window.SaiShell.fail();
    // 404「未実行のカードが見つかりません」も 502（常駐停止など）もここに 1 行で出る
    lvSetMsg((e && e.message) || "Could not send.", true);
  } finally {
    lvBusy = false;
    lvRenderActions();
  }
}

// --- 中身（8 つの表示・計画 14 §1.3） ---

async function lvLoadContent(seq) {
  const body = lvBody();
  if (!body) return;
  lvEditing = false;
  lvSaving = false;
  lvSetMsg("");
  body.innerHTML = "<p class='st-note'>Loading…</p>";
  const base = `/api/projects/${encodeURIComponent(lvName)}/runs/${lvRunId}`;
  try {
    if (lvView === "log") {
      await lvLoadSummary(seq, base);
      return;
    }
    if (lvView === "outputs") {
      await lvLoadOutputs(seq, base);
      return;
    }
    if (lvView === "waggle") {
      await lvLoadWaggleDoc(seq, base);
      return;
    }
    const data = await lvFetchJson(`${base}/md/${lvView}`);
    if (seq !== lvSeq) return; // その間に別の表示を開いた
    window.SaiShell.ok();
    body.innerHTML = "";
    lvRenderMd(data);
  } catch (e) {
    if (seq !== lvSeq) return;
    window.SaiShell.fail();
    const msg = (e && e.message) || "Could not load.";
    lvSetMsg(msg, true); // 「まだありません」（404）もここに 1 行で出る
    body.innerHTML = "";
    body.appendChild(el("p", "st-note", msg));
  }
}

/**
 * Summary（log・stats・history を 1 枚に・計画 14 §1.3 の改定）。
 * 3 つは別々に取り、失敗した部分にだけ理由を 1 行出す。全部失敗なら呼び元が失敗を出す。
 */
async function lvLoadSummary(seq, base) {
  const body = lvBody();
  const parts = await Promise.all(
    ["log", "stats", "history"].map((v) =>
      lvFetchJson(`${base}/view/${v}`).then(
        (d) => ({ d }),
        (e) => ({ err: (e && e.message) || "Could not load." })
      )
    )
  );
  if (seq !== lvSeq) return;
  const [log, stats, hist] = parts;
  if (parts.every((p) => p.err)) throw new Error(log.err);
  window.SaiShell.ok();
  body.innerHTML = "";
  const failed = (title, p) => body.appendChild(el("p", "st-note", `${title}: ${p.err}`));

  // 1. Summary（Inputs / Settings / Connection / Outputs）
  if (log.d) lvRenderLog(log.d);
  else failed("Summary", log);

  // 2. session の数字と大きな数字 ／ 3. Per-message trend
  if (stats.d) {
    if (stats.d.error) lvSetMsg(String(stats.d.error), true);
    body.appendChild(lvBigsEl(stats.d));
    const sess = lvSessionCard(log.d && log.d.session_stats);
    if (sess) body.appendChild(sess);
    body.appendChild(lvCountsLine(stats.d));
    body.appendChild(lvChartCard(stats.d));
  } else failed("Stats", stats);

  // 4. 種類別トークン（History のドーナツ）と CLI
  if (hist.d) {
    body.appendChild(lvHistHeadEl(hist.d));
    body.appendChild(lvTypeCard(hist.d));
    const cli = lvCliSummary(hist.d.cli);
    if (cli) body.appendChild(lvCliCard(cli));
  } else failed("History", hist);

  // 5. Per-message table と タイムラインの切り替え
  body.appendChild(lvDetailCard(stats, hist));
}

function lvCard(title) {
  const card = el("section", "lv-card");
  if (title) card.appendChild(el("h3", "lv-card-title", title));
  return card;
}

// 1. Summary: Inputs / Settings / Connection / Outputs（計画 14 §1.3）
function lvRenderLog(data) {
  const body = lvBody();
  const sections = logSections(data);
  if (sections.length === 0) {
    body.appendChild(el("p", "st-note", "Nothing to show."));
    return;
  }
  for (const s of sections) {
    const card = lvCard(s.title);
    for (const item of s.items || []) {
      // Connection の 1 件: 押すとその run の同じ表示へ飛ぶ（U01）
      const row = el("div", "lv-conn");
      const btn = el("button", "lv-kv-link", item.head);
      btn.type = "button";
      btn.addEventListener("click", () => {
        location.hash = listHash(lvName, item.runId, pickView(false, lvView || loadViewMemory()));
      });
      row.appendChild(btn);
      if (item.sub) row.appendChild(el("div", "lv-conn-sub", item.sub));
      card.appendChild(row);
    }
    for (const [label, value] of s.rows || []) {
      const row = el("div", "lv-kv");
      row.appendChild(el("span", "lv-kv-label", label));
      row.appendChild(el("span", "lv-kv-value", value));
      card.appendChild(row);
    }
    body.appendChild(card);
  }
}

// Summary の表（2〜5 の部品）。カードを作る小さな関数に分けて使い回す。
const LV_DETAILS = [
  ["table", "Per-message table"],
  ["timeline", "Timeline"],
];

// 2. 大きな数字（入力 / 出力 / 読 / 書 / 費用 / ヒット率）
function lvBigsEl(data) {
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
  return bigs;
}

/** session の数字の札（取れなければ null）。 */
function lvSessionCard(session) {
  const rows = sessionStatRows(session);
  if (rows.length === 0) return null;
  const sess = lvCard("Session");
  for (const [label, value] of rows) {
    const row = el("div", "lv-kv");
    row.appendChild(el("span", "lv-kv-label", label));
    row.appendChild(el("span", "lv-kv-value", value));
    sess.appendChild(row);
  }
  return sess;
}

/** 件数の 1 行（user / agent / thinking / toolCall）。 */
function lvCountsLine(data) {
  const counts = data.counts || {};
  return el(
    "div",
    "lv-sub",
    [
      `user ${lvNum(counts.user)}`,
      `agent ${lvNum(counts.assistant)}`,
      `thinking ${lvNum(counts.thinking)}`,
      `toolCall ${lvNum(counts.toolcall)}`,
    ].join(" · ")
  );
}

// 3. メッセージごとの推移（TUI の 1 / 2 / 3 と同じ 3 種・SVG 手描き）。
// 切り替えはこのカードの中だけ描き直す。
function lvChartCard(data) {
  const chart = lvCard("Per-message trend");
  const seg = el("div", "dash-seg small lv-chart-seg");
  for (const [key, label] of LV_CHARTS) {
    const b = document.createElement("button");
    b.textContent = label;
    if (key === lvChartMode) b.className = "active";
    b.addEventListener("click", () => {
      lvChartMode = key;
      chart.replaceWith(lvChartCard(data));
    });
    seg.appendChild(b);
  }
  chart.appendChild(seg);
  const series = statsSeries(data, lvChartMode);
  const values = series.flatMap((s) => s.values);
  if (values.length === 0) {
    chart.appendChild(el("p", "st-note", "No data."));
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
        el("span", "lv-leg-num", `Last ${saiAxisValue(last, s.pct ? "percent" : "tokens")}`)
      );
      legend.appendChild(row);
    }
    chart.appendChild(legend);
  }
  return chart;
}

/** Per-message table（表は横にはみ出す分だけスクロール・計画 14 §1.4）。 */
function lvTableBody(data) {
  const msgs = Array.isArray(data.messages) ? data.messages : [];
  if (msgs.length === 0) return el("p", "st-note", "No data.");
  const wrap = el("div", "lv-scroll-x");
  const table = el("table", "lv-table");
  const head = el("tr");
  for (const h of ["#", "Time", "Input", "Output", "Read", "Write", "Hit", "Cost"]) {
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
    row.appendChild(el("td", null, `${Math.round(lvNum(m.cache_hit_percent))}%`));
    const cost = m.cost && m.cost.total;
    row.appendChild(el("td", null, cost == null ? "—" : formatMoney(cost)));
    table.appendChild(row);
  }
  wrap.appendChild(table);
  return wrap;
}

/** 切り替えの選びを覚える（table か timeline・既定は table）。 */
function lvDetailMode() {
  try {
    return localStorage.getItem(LV_DETAIL_KEY) === "timeline" ? "timeline" : "table";
  } catch {
    return "table";
  }
}

/** 5. Per-message table かタイムラインを 1 つだけ出す。釦で切り替えて、選びは覚える。 */
function lvDetailCard(stats, hist) {
  const card = lvCard(null);
  const pick = lvDetailMode();
  const seg = el("div", "dash-seg small lv-chart-seg");
  for (const [key, label] of LV_DETAILS) {
    const b = document.createElement("button");
    b.textContent = label;
    if (key === pick) b.className = "active";
    b.addEventListener("click", () => {
      try { localStorage.setItem(LV_DETAIL_KEY, key); } catch { /* 保存できなくても動く */ }
      card.replaceWith(lvDetailCard(stats, hist));
    });
    seg.appendChild(b);
  }
  card.appendChild(seg);
  const part = pick === "timeline" ? hist : stats;
  if (part.d) card.appendChild(pick === "timeline" ? lvTimelineBody(part.d) : lvTableBody(part.d));
  else card.appendChild(el("p", "st-note", part.err));
  return card;
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
    "aria-label": "Per-message trend chart. No data",
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
    `Trend of ${series.map((s) => s.name).join(", ")} (${n} messages). ` +
      (at > 0 ? `Max ${saiAxisValue(dataMax, unit)} (message ${at}).` : "")
  );
  return svg;
}

// 4. History の見出し・種類別トークン（ドーナツ・SVG 手描き）・タイムラインの本体
function lvHistHeadEl(data) {
  const head = el("div", "lv-hist-head");
  head.appendChild(el("div", "lv-hist-title", String(data.title || "")));
  head.appendChild(el("div", "lv-sub", String(data.subtitle || "")));
  return head;
}

function lvTypeCard(data) {
  const slices = typeSlices(data.type_tokens);
  const card = lvCard("Tokens by type");
  if (slices.length === 0) {
    card.appendChild(el("p", "st-note", "No data."));
    return card;
  }
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
      `Donut chart of the token share by type. Total ${formatTokens(total)} tokens` +
      `. Largest: ${biggest.label} ${formatTokens(biggest.value)}` +
      ` (${Math.round(biggest.ratio * 100)}%)`,
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
  center.appendChild(el("span", "st-donut-cap", "Total"));
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
  return card;
}

/** タイムラインの本体（行は lvTimelineRow）。 */
function lvTimelineBody(data) {
  const events = Array.isArray(data.events) ? data.events : [];
  const box = el("div", null);
  box.appendChild(el("div", "lv-sub", `Timeline (${events.length})`));
  if (events.length === 0) {
    box.appendChild(el("p", "st-note", "No data."));
  } else {
    for (const ev of events) box.appendChild(lvTimelineRow(ev));
  }
  return box;
}

/** Timeline の 1 行（DOM）。2 段目以降（番号・本文の先頭）は行の幅いっぱいに折り返す。 */
function lvTimelineRow(ev) {
  const info = lvEventRow(ev);
  const row = el("div", "lv-tl");
  row.appendChild(el("span", "lv-tl-time", lvClock(ev && ev.time)));
  const type = String((ev && ev.type) || "");
  const idx = Math.max(0, LV_TYPES.indexOf(type));
  const chip = el("span", "lv-tl-type", type || "-");
  if (LV_TYPES.includes(type)) chip.style.background = LV_TYPE_COLORS[idx];
  row.appendChild(chip);
  row.appendChild(el("span", "lv-tl-name", info.name));
  if (info.failed) row.appendChild(el("span", "lv-tl-fail", "failed"));
  row.appendChild(el("span", "lv-tl-tokens", formatTokens(lvNum(ev && ev.tokens))));

  // 全文の釦: 番号が無いときは 1 段目に、あるときは 2 段目（番号のチップと並べる）
  const openBtn = info.canOpen ? el("button", "lv-tl-open", "Full text") : null;
  if (openBtn) {
    openBtn.type = "button";
    openBtn.addEventListener("click", () => void lvOpenCall(ev));
  }
  if (info.ids.shown.length > 0) {
    const foot = el("div", "lv-tl-foot");
    for (const id of info.ids.shown) foot.appendChild(el("span", "lv-tl-id", id));
    if (info.ids.rest > 0) foot.appendChild(el("span", "lv-tl-id", `+${info.ids.rest}`));
    if (openBtn) foot.appendChild(openBtn);
    row.appendChild(foot);
  } else if (openBtn) {
    row.appendChild(openBtn);
  }
  // 3 段目: 結果の先頭（2 行まで・押すと全文）
  if (info.preview) {
    const pv = info.canOpen ? el("button", "lv-tl-preview", info.preview) : el("div", "lv-tl-preview", info.preview);
    if (info.canOpen) {
      pv.type = "button";
      pv.addEventListener("click", () => void lvOpenCall(ev));
    }
    row.appendChild(pv);
  }
  return row;
}

/** `cli`（C11）の集計のカード: 命令の表・失敗の分類。 */
function lvCliCard(cli) {
  const card = lvCard("CLI calls");
  if (cli.calls.length === 0) {
    card.appendChild(el("p", "st-note", "No CLI calls."));
  } else {
    const table = el("table", "lv-cli-table");
    const head = el("tr");
    for (const t of ["Command", "Count", "Failed"]) head.appendChild(el("th", "", t));
    table.appendChild(head);
    for (const c of cli.calls) {
      const tr = el("tr");
      tr.appendChild(el("td", "lv-cli-cmd", c.command));
      tr.appendChild(el("td", "lv-cli-num", String(c.count)));
      tr.appendChild(
        el("td", c.errors > 0 ? "lv-cli-num lv-fail" : "lv-cli-num", c.errors > 0 ? String(c.errors) : "0")
      );
      table.appendChild(tr);
    }
    card.appendChild(table);
  }
  card.appendChild(el("div", "lv-cli-sub", "Failure classes"));
  if (cli.failures.length === 0) {
    card.appendChild(el("p", "st-note", "No failures."));
  } else {
    const legend = el("div", "lv-legend");
    for (const f of cli.failures) {
      const row = el("div", "lv-leg");
      row.append(el("span", "lv-leg-name", f.label), el("span", "lv-leg-num", String(f.count)));
      legend.appendChild(row);
    }
    card.appendChild(legend);
  }
  return card;
}

/** 呼び出しの全文を小窓で開く。押したときだけ取る（History の JSON を小さく保つ・U14）。 */
async function lvOpenCall(ev) {
  const body = el("div", "lv-call");
  body.appendChild(el("p", "st-note", "Loading…"));
  saiSheet({ title: lvCallName(ev) || "Call", body });
  const base = `/api/projects/${encodeURIComponent(lvName)}/runs/${lvRunId}`;
  try {
    const d = await lvFetchJson(`${base}/history/calls/${encodeURIComponent(ev.call_id)}`);
    body.innerHTML = "";
    body.appendChild(el("div", "lv-call-label", "Input"));
    body.appendChild(el("pre", "lv-call-pre", lvPrettyInput(d.input)));
    body.appendChild(el("div", "lv-call-label", d.is_error === true ? "Result (failed)" : "Result"));
    body.appendChild(el("pre", "lv-call-pre", String(d.result || "")));
    if (d.truncated === true) body.appendChild(el("p", "st-note", "(Truncated at 20,000 characters.)"));
  } catch (e) {
    body.innerHTML = "";
    body.appendChild(el("p", "st-note", (e && e.message) || "Could not load."));
  }
}

// --- History の呼び出しの表示（DOM を使わない・tests/test_linkview.js） ---

/** 行の名前: 命令（`command`）があれば命令名、無ければ道具の名前（`name`）。 */
function lvCallName(ev) {
  if (ev && ev.command) return String(ev.command);
  return String((ev && ev.name) || "");
}

/** 呼び出しの番号が全文の住所に使えるか（`history/calls/{call_id}` と同じ形）。 */
function lvCallIdOk(id) {
  return typeof id === "string" && /^[A-Za-z0-9_-]{1,80}$/.test(id);
}

/** 番号のチップ: 先頭 `max` 個（既定 5）と、残りの数（`+n`）。 */
function lvIdChips(ids, max) {
  const list = Array.isArray(ids) ? ids.map(String) : [];
  const n = max == null ? 5 : max;
  return { shown: list.slice(0, n), rest: Math.max(0, list.length - n) };
}

const LV_FAILURE_LABELS = {
  input: "Invalid input (2)",
  not_found: "Not found (3)",
  conflict: "State conflict (4)",
  other: "Other",
};

/** 失敗の分類の名前（`cli.failures[].class`）。表に無い分類は、分類の文字をそのまま出す。 */
function lvFailureLabel(cls) {
  return LV_FAILURE_LABELS[cls] || String(cls || "other");
}

/** Timeline 1 行の表示に要るもの（名前・失敗の印・番号のチップ・結果の先頭・全文を開けるか）。 */
function lvEventRow(ev) {
  const e = ev || {};
  return {
    name: lvCallName(e),
    failed: e.is_error === true,
    ids: lvIdChips(e.ids, 5),
    preview: typeof e.preview === "string" ? e.preview : "",
    canOpen: lvCallIdOk(e.call_id),
  };
}

/** `cli`（C11）の表示用。`cli` が無い JSON（古い CLI）は null（カードを出さない）。 */
function lvCliSummary(cli) {
  if (!cli || typeof cli !== "object") return null;
  const calls = Array.isArray(cli.calls) ? cli.calls : [];
  const failures = Array.isArray(cli.failures) ? cli.failures : [];
  const ids = cli.ids || {};
  return {
    calls: calls.map((c) => ({
      command: String((c && c.command) || ""),
      count: lvNum(c && c.count),
      errors: lvNum(c && c.errors),
    })),
    // 分類と命令の組ごとに来るので、分類の名前で合算する
    failures: [...failures.reduce((m, f) => {
      const label = lvFailureLabel(f && f.class);
      return m.set(label, (m.get(label) || 0) + lvNum(f && f.count));
    }, new Map())].map(([label, count]) => ({ label, count })),
    ids: { seen: lvNum(ids.seen), used: lvNum(ids.used), unused: lvNum(ids.unused) },
  };
}

/** 入力（JSON の文字列）を読みやすく整える。読めなければそのまま。 */
function lvPrettyInput(text) {
  const s = String(text || "");
  try {
    return JSON.stringify(JSON.parse(s), null, 2);
  } catch {
    return s;
  }
}

/** ISO の時刻を「HH:MM:SS」（タイムラインと一覧で使う）。 */
function lvClock(iso) {
  const d = new Date(String(iso || ""));
  if (isNaN(d)) return "";
  return d.toLocaleTimeString("en-US", {
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
  body.appendChild(lvBackToOutputs());
  if (lvEditing) {
    const ta = document.createElement("textarea");
    ta.className = "md-edit";
    ta.value = lvDraft;
    ta.disabled = lvSaving;
    ta.addEventListener("input", () => { lvDraft = ta.value; });
    body.appendChild(ta);
    const row = el("div", "lv-edit-row");
    const save = el("button", "paper-btn", lvSaving ? "Saving…" : "Save");
    save.disabled = lvSaving;
    save.addEventListener("click", () => void lvSaveMd());
    const cancel = el("button", "paper-btn", "Cancel");
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
    const edit = el("button", "paper-btn", lvMdText ? "Edit" : "Write");
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
    body.appendChild(el("p", "st-note", "None yet."));
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
    if (!res.ok) throw new Error(data.detail || "Could not save.");
    if (seq !== lvSeq) return; // その間に別の表示を開いた
    lvSaving = false;
    lvEditing = false;
    lvSetMsg("Saved.");
    lvRenderMd(data);
  } catch (e) {
    if (seq !== lvSeq) return;
    // 書きかけは消さずに残す（直してもう一度押せる）
    lvSaving = false;
    lvSetMsg((e && e.message) || "Could not save.", true);
    lvDrawMd();
  }
}

/** 成果物の表示名 `<waggle_token>/<file_name>`。 */
function lvWaggleName(item) {
  return `${item.token || ""}/${item.label || ""}`;
}

/** ＋: この文書を task_text に貼った状態で task.md の作成フォームを開く。 */
function lvWaggleAdd(item) {
  const b = el("button", "lv-waggle-add", "+");
  b.type = "button";
  b.setAttribute("aria-label", "Create a task.md from this document");
  b.title = "Create a task.md from this document";
  b.addEventListener("click", (e) => {
    e.stopPropagation();
    if (typeof cfOpen === "function") void cfOpen({ ref: item.ref });
  });
  return b;
}

/** 成果物の一覧（同じ run の間は使い回す）。Outputs と文書の ＋ で使う。 */
async function lvWaggleItems(base) {
  if (!Array.isArray(lvWaggleList)) {
    const data = await lvFetchJson(`${base}/waggle`);
    lvWaggleList = Array.isArray(data.items) ? data.items : [];
  }
  return lvWaggleList;
}

/** ‹ Outputs: 成果物の一覧へ戻る（md と文書の上に置く）。 */
function lvBackToOutputs() {
  const back = el("button", "lv-md-back", "‹ Outputs");
  back.type = "button";
  back.addEventListener("click", () => {
    location.hash = listHash(lvName, lvRunId, "outputs");
  });
  return back;
}

/**
 * Outputs（計画 14 §1.3 の改定）: system・task・message・Report と、waggle の成果物の一覧。
 * todo の run では task だけ開ける。waggle の一覧は取りにいかない。
 */
async function lvLoadOutputs(seq, base) {
  const body = lvBody();
  const todo = isTodoRun(lvRun);
  // task の説明は front matter の description（取れなければ空）
  const [taskText, items] = await Promise.all([
    lvFetchJson(`${base}/md/task`).then((d) => (d && d.text) || "", () => ""),
    todo ? null : lvWaggleItems(base),
  ]);
  if (seq !== lvSeq) return;
  window.SaiShell.ok();
  body.innerHTML = "";

  const fixed = [
    ["system", "system", ""],
    ["task", "task", lvTaskDescription(taskText)],
    ["message", "message", ""],
    ["report", "Report", ""],
  ];
  for (const [view, label, desc] of fixed) {
    const row = document.createElement("button");
    row.type = "button";
    row.className = "lv-waggle-row";
    row.disabled = todo && view !== "task";
    row.appendChild(el("span", "lv-waggle-label", label));
    row.appendChild(el("span", "lv-waggle-desc", desc));
    row.addEventListener("click", () => {
      location.hash = listHash(lvName, lvRunId, view);
    });
    body.appendChild(row);
  }

  const wcard = lvCard("waggle");
  if (todo) {
    wcard.appendChild(el("p", "st-note", "Outputs appear after the run."));
  } else if (items.length === 0) {
    wcard.appendChild(el("p", "st-note", "No outputs."));
  } else {
    for (const item of items) {
      const line = el("div", "lv-waggle-line");
      const row = document.createElement("button");
      row.type = "button";
      row.className = "lv-waggle-row";
      row.appendChild(el("span", "lv-waggle-label", lvWaggleName(item)));
      row.appendChild(el("span", "lv-waggle-desc", String(item.description || "")));
      row.addEventListener("click", () => {
        location.hash = listHash(lvName, lvRunId, "waggle", item.token);
      });
      line.append(row, lvWaggleAdd(item));
      wcard.appendChild(line);
    }
  }
  body.appendChild(wcard);
}

/** waggle の本文（住所のトークン。上に ‹ Outputs）。 */
async function lvLoadWaggleDoc(seq, base) {
  const body = lvBody();
  const [data, items] = await Promise.all([
    lvFetchJson(`${base}/waggle/${encodeURIComponent(lvWaggleToken)}`),
    lvWaggleItems(base),
  ]);
  if (seq !== lvSeq) return;
  window.SaiShell.ok();
  body.innerHTML = "";
  body.appendChild(lvBackToOutputs());
  const item = items.find((w) => w.token === lvWaggleToken);
  const title = el("div", "lv-waggle-title", item ? lvWaggleName(item) : String(data.label || lvWaggleToken));
  if (item) title.appendChild(lvWaggleAdd(item));
  body.appendChild(title);
  body.appendChild(el("div", "lv-waggle-text", ""));
  body.lastChild.innerHTML = renderMarkdown(data.text || "", { toolbar: true });
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
  const rows = flattenTree(lvTree || [], lvFolds === null ? defaultOpenIds(lvTree) : lvFolds);
  if (rows.length === 0) {
    nav.appendChild(
      el("p", "lv-drawer-note", lvTreeError || (lvTree ? "No runs." : "Loading…"))
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
      b.addEventListener("click", () => lvToggleFold(r.id));
      nav.appendChild(b);
    } else {
      // 枠・矢印・名前の幅は style.css（U07 の担当）を触らず、ここで指定する
      const item = el("div", "lv-tree-row");
      item.style.display = "flex";
      // 子を持つ run だけ開閉の矢印を付ける。子の無い run は同じ幅だけ空けて字下げをそろえる
      const arrow = document.createElement("button");
      arrow.type = "button";
      arrow.className = "lv-fold lv-run-arrow";
      arrow.style.paddingLeft = `${10 + r.depth * 14}px`;
      arrow.style.paddingRight = "0";
      arrow.style.width = "auto";
      arrow.style.flex = "none";
      if (r.hasChildren) {
        arrow.appendChild(el("span", "lv-fold-arrow", r.open ? "▾" : "▸"));
        arrow.setAttribute("aria-label", r.open ? "Collapse" : "Expand");
        arrow.setAttribute("aria-expanded", r.open ? "true" : "false");
        arrow.addEventListener("click", () => lvToggleFold(r.id));
      } else {
        arrow.disabled = true;
        arrow.setAttribute("aria-hidden", "true");
        arrow.appendChild(el("span", "lv-fold-arrow", ""));
      }
      item.appendChild(arrow);
      const b = document.createElement("button");
      b.className = "lv-run-item" + (r.runId === lvRunId ? " active" : "");
      b.style.paddingLeft = "6px";
      b.style.width = "auto";
      b.style.flex = "1";
      b.style.minWidth = "0";
      const dot = document.createElement("span");
      dot.className = `lv-dot lv-${(r.lane || r.status || "none").replace(/[^a-z]/gi, "") || "none"}`;
      dot.setAttribute("aria-hidden", "true");
      b.appendChild(dot);
      b.appendChild(el("span", "lv-run-item-label", r.label));
      // エージェントが起こす todo には短い印を出す（一覧で分かるように・U1）。
      if (r.lane === "todo" && isAgentRun(r)) {
        const mark = el("span", "lv-run-item-mark", "AI");
        mark.title = "Started by an agent";
        b.appendChild(mark);
      }
      b.addEventListener("click", () => {
        // run を選ぶと閉じ，最後に見ていた表示のまま中身だけ変わる（計画 14 §1.3）
        const view = pickView(isTodoRun(r), lvView || loadViewMemory());
        location.hash = listHash(lvName, r.runId, view);
        lvSetDrawer(false);
      });
      item.appendChild(b);
      nav.appendChild(item);
    }
  }
}

/** フォルダ・run の開閉（覚えが無ければ defaultOpenIds から 1 つ開け閉めした形を保存する）。 */
function lvToggleFold(id) {
  const base = lvFolds === null ? defaultOpenIds(lvTree) : lvFolds;
  lvFolds = toggleFold(base, id);
  lvSaveFolds(lvName, lvFolds);
  lvRenderDrawer();
}

// --- サイドバーの見出しの行（実行・＋（task.md の作成）・↻（読み直し）・U01 §4） ---

// 見出しの行を 1 本作る。毎回作り直されるので、ボタンは作るたびにイベントを付ける。
// 狭い画面では引き出しの中、広い画面では左の列の一番上に出る。
function lvSideHead() {
  const row = el("div", "lv-side-head");
  row.appendChild(el("span", "lv-side-label", "Run"));
  const add = el("button", "lv-side-btn", "+");
  add.type = "button";
  add.setAttribute("aria-label", "New card");
  add.title = "Create a task.md";
  add.addEventListener("click", () => {
    // 作成フォームは cardform.js のグローバル関数
    if (typeof cfOpen === "function") void cfOpen();
  });
  const reload = el("button", "lv-side-btn", "↻");
  reload.type = "button";
  reload.setAttribute("aria-label", "Reload");
  reload.title = "Reload the tree";
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

// --- md の中の waggle リンク（`waggle:<トークン>`）を押したら、その文書の表示へ移る ---
// 文書に `:path:…/<run_id>/…` があればその run、無ければ今開いている run の下で開く
// （トークンの解決は run に依らない）。List が見えているときだけ動く。
if (typeof document !== "undefined") {
  document.addEventListener("click", (e) => {
    const a = e.target && typeof e.target.closest === "function" ? e.target.closest("a.md-waggle") : null;
    if (!a) return;
    e.preventDefault();
    const list = document.getElementById("view-list");
    if (!list || list.hidden || !lvName) return;
    const run = Number(a.dataset.waggleRun) || lvRunId;
    if (run == null) return;
    location.hash = listHash(lvName, run, "waggle", a.dataset.waggleToken);
  });
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
    isAgentRun,
    lvCanEdit,
    flattenTree,
    findRun,
    runLanes,
    advanceAction,
    defaultOpenIds,
    logSections,
    connectionItems,
    sessionStatRows,
    statsCells,
    statsSeries,
    typeSlices,
    lvCallName,
    lvCallIdOk,
    lvIdChips,
    lvFailureLabel,
    lvEventRow,
    lvCliSummary,
    lvPrettyInput,
    tabOf,
    lvTaskDescription,
    lvScheduledAt,
  };
}
