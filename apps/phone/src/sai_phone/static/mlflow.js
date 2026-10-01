"use strict";
// 実験（MLflow の記録）を見る画面（実装計画 U06・共有契約 C-03 / C-04 / C-07）。
//
// 上のタブ「実験」から入る。MLflow の run を seed → 親（branch）→ 子（trial）の
// 入れ子で見られるようにし、成果物の中身を開けるようにする。
//
// 中の処理は sai-phone の読み取り用の中継（`/api/mlflow/*`・C-03）と
// プロジェクト名の一覧（`GET /api/projects`）だけを使う。MLflow へ直接は話さない
// （ARCHITECTURE.md §2）。この画面はファイルのパスを受け取らない。
//
// 画面をまたがない共通部品は ui.js（saiApi / saiEl / saiSheet / saiEmpty /
// saiSplitShow）と markdown.js（renderMarkdown）だけ。DOM を使わない純関数
// （住所・絞り込みの式・入れ子の木・日本語の文）は末尾で node に渡す（tests/test_mlflow.js）。
//
// 一番外側の名前の頭は mlf / MLF_（C-07）。

// --- 定数 -------------------------------------------------------------

// MLflow の実験 ID（数字 1〜19 桁）と run ID（16 進 32 桁）
const MLF_EXPERIMENT_ID_RE = /^\d{1,19}$/;
const MLF_RUN_ID_RE = /^[0-9a-f]{32}$/;
// 住所の ? に付ける絞り込みの鍵（この順に並べて URL を作る）
const MLF_QUERY_KEYS = ["sai_run", "seed", "study", "status"];
// 比較は 2〜20 件（C-07）
const MLF_COMPARE_MIN = 2;
const MLF_COMPARE_MAX = 20;
// 1 回に取る行の数（MLflow の 1 回の返答の上限と同じ）
const MLF_PAGE_SIZE = 100;
// 選ぶプロジェクトの覚え場所（C-07）
const MLF_PROJECT_KEY = "sai_phone_mlf_project";
// 「すべてのプロジェクト」はこの値として覚える（null は「まだ選んでいない」）
const MLF_PROJECT_ALL = "*";
// 画面の「広いか」の境目（900px 以上）
const MLF_WIDE_PX = 900;
// 指標の一括取得で送れる run の数の上限（MLflow の 1 回の返答の上限）
const MLF_BULK_RUNS = 100;
// 一度に同時に読む run の数（比較画面で使う）
const MLF_READ_PARALLEL = 5;
// 指標の履歴を取る点数の上限（history 25000 点・bulk 2500 点）
const MLF_HISTORY_MAX = 25000;
const MLF_BULK_MAX = 2500;

// 状態の日本語（C-04）
const MLF_STATUS_TEXT = {
  FINISHED: "完了",
  FAILED: "失敗",
  RUNNING: "実行中",
  KILLED: "中止",
  SCHEDULED: "予定",
};
// 画面が読むタグの名前（C-04）
const MLF_TAGS = {
  parent: "mlflow.parentRunId",
  saiProject: "sai.project",
  saiRun: "sai.run_id",
  saiSeed: "sai.seed_run_id",
  study: "exp.study",
  generation: "exp.generation",
  script: "exp.script",
  action: "exp.action",
  role: "exp.role",
  sciNode: "sci.node",
  metric: "exp.metric",
  direction: "exp.direction",
};
// 文字ファイルとしてそのまま見せる拡張子
const MLF_TEXT_EXT = ["txt", "log", "py", "json", "yaml", "yml", "cfg", "toml", "ini", "sh", "rs", "js", "ts", "sql", "xml", "html", "env"];
// 途中まで表にする拡張子
const MLF_TABLE_EXT = ["csv", "tsv"];
// 絵として見せる拡張子
const MLF_IMAGE_EXT = ["png", "jpg", "jpeg", "gif", "svg", "webp"];
// 表に出す値の桁（これ以上は丸める）
const MLF_VALUE_DIGITS = 4;
// 表にする成果物の読み込みの上限（行）
const MLF_CSV_ROWS = 200;

// ============================================================================
// 純関数（DOM を使わない。ここだけを tests/test_mlflow.js で確かめる）
// ============================================================================

/** 検索式の値として使う引用符（`\` と `"` を逃がす）。 */
function mlfEscape(value) {
  return String(value == null ? "" : value)
    .replace(/\\/g, "\\\\")
    .replace(/"/g, '\\"');
}

/**
 * 住所の ? 以降を、絞り込みの物（{sai_run, seed, study, status}）にする。
 * 知らない鍵と空の値は捨てる（往復を保つため）。
 */
function mlfParseQuery(qs) {
  const query = {};
  for (const part of String(qs == null ? "" : qs).split("&")) {
    if (!part) continue;
    const i = part.indexOf("=");
    const key = i < 0 ? part : part.slice(0, i);
    if (!MLF_QUERY_KEYS.includes(key)) continue;
    const raw = i < 0 ? "" : part.slice(i + 1);
    const value = decodeURIComponent(raw.replace(/\+/g, " "));
    if (value) query[key] = value;
  }
  return query;
}

/** 絞り込みの物を "?sai_run=1&seed=2" の形にする（順序は MLF_QUERY_KEYS 順）。 */
function mlfQueryString(query) {
  const q = query || {};
  const parts = [];
  for (const key of MLF_QUERY_KEYS) {
    const value = q[key];
    if (value == null || value === "") continue;
    parts.push(`${key}=${encodeURIComponent(String(value))}`);
  }
  return parts.join("&");
}

/**
 * 住所を解析する（C-07）。
 *   #/mlflow                                                … 実験の一覧
 *   #/mlflow/e/<実験 ID|all>?sai_run=&seed=&study=&status=  … run の表
 *   #/mlflow/run/<run ID>                                   … run の詳細
 *   #/mlflow/compare/<run ID>,<run ID>,…                    … 比較（画面は U07）
 * 返り値は {page, experimentId, runId, runIds, query}。知らない住所は null。
 */
function mlfParseHash(hash) {
  const raw = String(hash == null ? "" : hash);
  const head = "#/mlflow";
  if (raw !== head && !raw.startsWith(`${head}/`) && !raw.startsWith(`${head}?`)) return null;
  const restAll = raw.slice(head.length);
  const qi = restAll.indexOf("?");
  const rest = qi >= 0 ? restAll.slice(0, qi) : restAll;
  const query = qi >= 0 ? mlfParseQuery(restAll.slice(qi + 1)) : {};

  if (rest === "") {
    return { page: "experiments", experimentId: null, runId: null, runIds: null, query: {} };
  }
  let m = rest.match(/^\/e\/([^/]+)$/);
  if (m) {
    const id = decodeURIComponent(m[1]);
    if (id !== "all" && !MLF_EXPERIMENT_ID_RE.test(id)) return null;
    return { page: "runs", experimentId: id, runId: null, runIds: null, query };
  }
  m = rest.match(/^\/run\/([^/]+)$/);
  if (m) {
    const id = decodeURIComponent(m[1]);
    if (!MLF_RUN_ID_RE.test(id)) return null;
    return { page: "run", experimentId: null, runId: id, runIds: null, query: {} };
  }
  m = rest.match(/^\/compare\/([^/]+)$/);
  if (m) {
    const ids = decodeURIComponent(m[1]).split(",");
    if (ids.length < MLF_COMPARE_MIN || ids.length > MLF_COMPARE_MAX) return null;
    if (ids.some((one) => !MLF_RUN_ID_RE.test(one))) return null;
    return { page: "compare", experimentId: null, runId: null, runIds: ids, query: {} };
  }
  return null;
}

/** 住所の作り手（`mlfParseHash` と往復する）。形が違えば null。 */
function mlfHash(state) {
  const s = state || {};
  if (s.page === "experiments") return "#/mlflow";
  if (s.page === "runs") {
    const id = s.experimentId == null ? "" : String(s.experimentId);
    if (id !== "all" && !MLF_EXPERIMENT_ID_RE.test(id)) return null;
    const q = mlfQueryString(s.query);
    return `#/mlflow/e/${id}${q ? `?${q}` : ""}`;
  }
  if (s.page === "run") {
    const id = String(s.runId == null ? "" : s.runId);
    if (!MLF_RUN_ID_RE.test(id)) return null;
    return `#/mlflow/run/${id}`;
  }
  if (s.page === "compare") {
    const ids = (Array.isArray(s.runIds) ? s.runIds : []).map((one) => String(one));
    if (ids.length < MLF_COMPARE_MIN || ids.length > MLF_COMPARE_MAX) return null;
    if (ids.some((one) => !MLF_RUN_ID_RE.test(one))) return null;
    return `#/mlflow/compare/${ids.join(",")}`;
  }
  return null;
}

/**
 * 住所の絞り込みを、MLflow の検索式にする。
 *   sai_run=121     → tags."sai.run_id" = "121"
 *   seed=120        → tags."sai.seed_run_id" = "120"
 *   study=N12-s120  → tags."exp.study" = "N12-s120"
 *   status=FINISHED → attributes.status = "FINISHED"
 * 複数あるときは AND でつなぐ。何も無ければ空文字列。
 */
function mlfFilterFromQuery(query) {
  const q = query || {};
  const parts = [];
  for (const [key, tag] of [["sai_run", "sai.run_id"], ["seed", "sai.seed_run_id"], ["study", "exp.study"]]) {
    const value = q[key];
    if (value == null || value === "") continue;
    parts.push(`tags."${tag}" = "${mlfEscape(value)}"`);
  }
  if (q.status != null && q.status !== "") {
    parts.push(`attributes.status = "${mlfEscape(q.status)}"`);
  }
  return parts.join(" AND ");
}

/** run の中のタグの値（無ければ空文字列）。 */
function mlfTag(run, key) {
  const tags = (run && run.data && run.data.tags) || [];
  for (const one of tags) if (one && one.key === key) return one.value == null ? "" : String(one.value);
  return "";
}

/** run の中の指標の値（無ければ null）。同じ名前は最後の点を返す。 */
function mlfMetricValue(run, key) {
  const metrics = (run && run.data && run.data.metrics) || [];
  let found = null;
  for (const one of metrics) {
    if (!one || one.key !== key) continue;
    const value = Number(one.value);
    found = Number.isFinite(value) ? value : null;
  }
  return found;
}

/** run の中の指標を全部（[{key, value}]・名前の順）。 */
function mlfMetricList(run) {
  const metrics = (run && run.data && run.data.metrics) || [];
  const out = [];
  for (const one of metrics) {
    if (!one || one.key == null) continue;
    if (out.some((x) => x.key === one.key)) continue;
    out.push({ key: String(one.key), value: mlfMetricValue(run, one.key) });
  }
  return out.sort((a, b) => (a.key < b.key ? -1 : a.key > b.key ? 1 : 0));
}

/** run の中の親 run の ID（無ければ空文字列）。 */
function mlfParentId(run) {
  return mlfTag(run, MLF_TAGS.parent);
}

/** run の中の seed の ID（無ければ空文字列）。 */
function mlfSeedId(run) {
  return mlfTag(run, MLF_TAGS.saiSeed);
}

/**
 * 表に出す指標の値（C-04）。`exp.metric` のタグで指標を決め、
 * 無ければ最初の指標を使う（MLflow の画面と同じ）。
 */
function mlfHeadMetric(run) {
  const name = mlfTag(run, MLF_TAGS.metric);
  if (name) {
    const value = mlfMetricValue(run, name);
    if (value != null) return { key: name, value };
  }
  const list = mlfMetricList(run);
  return list.length ? list[0] : null;
}

/**
 * 読み込んだ run を seed → 親 → 子の木にする。
 *   段 1: `sai.seed_run_id` ごとの束（「seed #120」「seed なし」）
 *   段 2: 親 run（`mlflow.parentRunId` が無いもの）と、親が読み込まれていない孤立した子
 *   段 3: 親に属する子
 * 返り値は [{key, seed, label, rows: [{kind, run, children}]}]。kind は "parent" / "orphan"。
 */
function mlfBuildTree(runs) {
  const list = (Array.isArray(runs) ? runs : []).filter((r) => r && r.info && r.info.run_id);
  const bundles = new Map();
  const bundleOf = (run) => {
    const seed = mlfSeedId(run);
    const key = seed ? `seed:${seed}` : "seed:none";
    if (!bundles.has(key)) {
      bundles.set(key, { key, seed, label: seed ? `seed #${seed}` : "seed なし", rows: [] });
    }
    return bundles.get(key);
  };
  const known = new Set(list.map((r) => String(r.info.run_id)));
  const childMap = new Map(); // 親 ID → 子の run（読み込んだ順）
  for (const run of list) {
    const pid = mlfParentId(run);
    if (!pid || !known.has(pid)) continue;
    if (!childMap.has(pid)) childMap.set(pid, []);
    childMap.get(pid).push(run);
  }
  for (const run of list) {
    const pid = mlfParentId(run);
    // 親が読み込まれていない子だけは段 2 へ出す。親のある子は段 3 にだけ出る
    if (pid && !known.has(pid)) {
      bundleOf(run).rows.push({ kind: "orphan", run, children: [] });
      continue;
    }
    if (pid) continue; // 親がある子。前の処理で親の子にまとめている
    const bundle = bundleOf(run);
    bundle.rows.push({
      kind: "parent",
      run,
      children: childMap.get(String(run.info.run_id)) || [],
    });
  }
  return [...bundles.values()].filter((b) => b.rows.length > 0);
}

/**
 * 入れ子を解いた平らな行（指標で並べ替えたとき用）。
 * 返り値は [{run, depth}]。depth は段 2 が 0、段 3 が 1。
 */
function mlfFlatRows(runs) {
  const out = [];
  for (const bundle of mlfBuildTree(runs)) {
    for (const row of bundle.rows) {
      out.push({ run: row.run, depth: 0 });
      for (const child of row.children) out.push({ run: child, depth: 1 });
    }
  }
  return out;
}

/** 並べ替えの選択肢（[[order_by, 日本語]]）。 */
function mlfOrderOptions(runs) {
  const options = [
    ["attributes.start_time DESC", "開始（新しい順）"],
    ["attributes.start_time ASC", "開始（古い順）"],
  ];
  const names = new Set();
  for (const run of Array.isArray(runs) ? runs : []) {
    if (mlfParentId(run)) continue; // 指標は親が持つ（C-04）
    const name = mlfTag(run, MLF_TAGS.metric);
    if (name) names.add(name);
  }
  for (const name of [...names].sort()) {
    options.push([`metrics.${name} DESC`, `指標 ${name}（大きい順）`]);
    options.push([`metrics.${name} ASC`, `指標 ${name}（小さい順）`]);
  }
  return options;
}

/** 指標で並べたときか（このときは入れ子を解く）。 */
function mlfIsMetricOrder(orderBy) {
  return /^metrics\./.test(String(orderBy || ""));
}

/** 状態（RUNNING など）を日本語にする。 */
function mlfStatusText(status) {
  const key = String(status == null ? "" : status).toUpperCase();
  return MLF_STATUS_TEXT[key] || (status ? String(status) : "—");
}

/** 状態ごとのクラス（CSS の色と対応させる）。 */
function mlfStatusClass(status) {
  const key = String(status == null ? "" : status).toUpperCase();
  if (key === "FINISHED") return "mlf-st-ok";
  if (key === "FAILED" || key === "KILLED") return "mlf-st-bad";
  if (key === "RUNNING") return "mlf-st-run";
  if (key === "SCHEDULED") return "mlf-st-wait";
  return "";
}

/** ミリ秒を「1 時間 2 分」のような日本語にする。 */
function mlfDurationText(ms) {
  const n = Number(ms);
  if (!Number.isFinite(n) || n < 0) return "—";
  if (n < 1000) return "1 秒未満";
  const sec = Math.round(n / 1000);
  if (sec < 1) return "1 秒未満";
  if (sec < 60) return `${sec} 秒`;
  const min = Math.floor(sec / 60);
  const restSec = sec % 60;
  if (min < 60) return restSec ? `${min} 分 ${restSec} 秒` : `${min} 分`;
  const hour = Math.floor(min / 60);
  const restMin = min % 60;
  return restMin ? `${hour} 時間 ${restMin} 分` : `${hour} 時間`;
}

/** 時間の長さ（始点と終点。無ければ不定の値）。 */
function mlfDurationMs(run) {
  const info = (run && run.info) || {};
  // MLflow は時刻をミリ秒の数値で返す（文字列の ISO も受ける）
  const toMs = (v) => (typeof v === "number" ? v : Date.parse(v || ""));
  const start = toMs(info.start_time);
  const end = toMs(info.end_time);
  if (!Number.isFinite(start) || !Number.isFinite(end)) return NaN;
  return Math.max(0, end - start);
}

/** ISO の時刻を「2026/09/29 14:03」にする（読めなければ —）。 */
function mlfTimeText(iso) {
  if (!iso) return "—";
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return "—";
  const p = (n) => String(n).padStart(2, "0");
  return `${d.getFullYear()}/${p(d.getMonth() + 1)}/${p(d.getDate())} ${p(d.getHours())}:${p(d.getMinutes())}`;
}

/** 日付だけ（実験の一覧で使う）。 */
function mlfDayText(iso) {
  if (!iso) return "—";
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return "—";
  const p = (n) => String(n).padStart(2, "0");
  return `${d.getFullYear()}/${p(d.getMonth() + 1)}/${p(d.getDate())}`;
}

/** 指標の値などを画面に出す文字列にする。 */
function mlfValueText(value) {
  if (value == null || value === "") return "—";
  const n = Number(value);
  if (!Number.isFinite(n)) return String(value);
  if (Number.isInteger(n)) return n.toLocaleString("ja-JP");
  return String(Number(n.toFixed(MLF_VALUE_DIGITS)));
}

/** 拡張子（無ければ空文字列）。 */
function mlfExt(path) {
  const name = String(path || "").split("/").pop() || "";
  const i = name.lastIndexOf(".");
  if (i <= 0) return "";
  return name.slice(i + 1).toLowerCase();
}

/** 成果物の拡張子から「どう見せるか」を決める。 */
function mlfArtifactKind(path) {
  const ext = mlfExt(path);
  if (ext === "md") return "markdown";
  if (MLF_TEXT_EXT.includes(ext)) return "text";
  if (MLF_TABLE_EXT.includes(ext)) return "table";
  if (MLF_IMAGE_EXT.includes(ext)) return "image";
  return "unknown";
}

/** プロジェクトの選択肢（先頭は「すべてのプロジェクト」）。 */
function mlfProjectChoices(projects) {
  const list = [{ name: "", label: "すべてのプロジェクト", note: "" }];
  for (const one of (projects && projects.projects) || []) {
    if (!one || !one.name) continue;
    list.push({ name: one.name, label: one.name, note: one.description || "" });
  }
  return list;
}

/** run の中の指標の名前のならんだ列（重複無し・名前順）。 */
function mlfMetricKeys(run) {
  const out = [];
  for (const one of (run && run.data && run.data.metrics) || []) {
    if (!one || one.key == null) continue;
    const key = String(one.key);
    if (!out.includes(key)) out.push(key);
  }
  return out.sort((a, b) => (a < b ? -1 : a > b ? 1 : 0));
}

/** 複数の run にある指標の名前の和集合（名前順）。 */
function mlfMetricKeysOf(runs) {
  const out = [];
  for (const run of Array.isArray(runs) ? runs : []) {
    for (const key of mlfMetricKeys(run)) if (!out.includes(key)) out.push(key);
  }
  return out.sort((a, b) => (a < b ? -1 : a > b ? 1 : 0));
}

/**
 * 比較の表の行（U07）を作る。
 *   runs: 選んだ run の並んだもの
 *   diffOnly: true なら「値が全部同じ行」を捨てる
 * 返り値は `[{section, key, values: [...], same}]`。
 *   section は "param" / "metric" / "tag"（日本語の見出しは mlfCompareSectionTitle）
 */
function mlfCompareRows(runs, diffOnly) {
  const list = (Array.isArray(runs) ? runs : []).filter((r) => r && r.info && r.info.run_id);
  const rows = [];
  const push = (section, key, valueOf) => {
    const values = list.map((run) => {
      const raw = valueOf(run);
      return raw == null || raw === "" ? "—" : String(raw);
    });
    const first = values.length ? values[0] : "";
    const same = values.every((v) => v === first);
    if (diffOnly && same) return;
    rows.push({ section, key, values, same });
  };

  // --- 設定（param） ---
  const paramKeys = new Set();
  for (const run of list) {
    for (const one of (run.data && run.data.params) || []) {
      if (one && one.key != null) paramKeys.add(String(one.key));
    }
  }
  for (const key of [...paramKeys].sort()) {
    push("param", key, (run) => {
      for (const one of (run.data && run.data.params) || []) {
        if (one && String(one.key) === key) return one.value;
      }
      return null;
    });
  }

  // --- 指標（metric・最後の値） ---
  for (const key of mlfMetricKeysOf(list)) {
    push("metric", key, (run) => {
      const v = mlfMetricValue(run, key);
      return v == null ? null : mlfValueText(v);
    });
  }

  // --- タグ ---
  const tagKeys = new Set();
  for (const run of list) {
    for (const one of (run.data && run.data.tags) || []) {
      if (one && one.key != null) tagKeys.add(String(one.key));
    }
  }
  for (const key of [...tagKeys].sort()) {
    push("tag", key, (run) => mlfTag(run, key));
  }
  return rows;
}

/** 比較の表の節の見出し。 */
function mlfCompareSectionTitle(section) {
  if (section === "param") return "設定（param）";
  if (section === "metric") return "指標（metric・最後の値）";
  if (section === "tag") return "タグ";
  return String(section || "");
}

/** 比較の表の見出しの run の名前（無いときは run_id の先頭 8 字）。 */
function mlfCompareHeadName(run) {
  const info = (run && run.info) || {};
  return info.run_name || String(info.run_id || "").slice(0, 8) || "—";
}

// ============================================================================
// 状態
// ============================================================================

let mlfMounted = false;        // 枠（見出しと 2 つの欄）を一度作ったか
let mlfState = null;           // 今開いている画面（mlfParseHash の返り値）
let mlfProjects = null;        // GET /api/projects の返り値
let mlfProjectError = "";      // プロジェクト一覧を読めなかったときの理由
let mlfProject = null;         // 選んでいるプロジェクト名（"" は「すべて」・null は未選択）
let mlfExperiments = null;     // 実験の一覧
let mlfExperimentsError = "";  // 実験一覧の読み込みの失敗
let mlfTable = null;           // run の表の状態
let mlfDetail = null;          // run の詳細
let mlfCompare = null;         // run の比較
let mlfArt = null;             // 成果物
let mlfOpen = new Set();       // 開いている束・親の鍵（画面の中だけ覚える）
let mlfWasWide = null;         // 直前の「広いか」

/** 画面の枠（見出し・プロジェクト名の札・2 つの欄）を作る。2 回目以降は作らない。 */
function mlfBuild() {
  const root = document.getElementById("mlf-root");
  if (!root || mlfMounted) return;
  root.innerHTML = "";

  const bar = saiEl("header", "appbar");
  const back = saiEl("button", "", "‹");
  back.type = "button";
  back.id = "mlf-back";
  back.setAttribute("aria-label", "Back");
  back.addEventListener("click", () => void mlfGoBack());
  const title = saiEl("h1", "", "Experiments");
  title.id = "mlf-title";
  const reload = saiEl("button", "", "↻");
  reload.type = "button";
  reload.id = "mlf-reload";
  reload.setAttribute("aria-label", "読み直す");
  reload.addEventListener("click", () => void mlfOpenPage(mlfState));
  bar.append(back, title, reload);

  const projRow = saiEl("div", "mlf-proj-row");
  const chip = saiEl("button", "mlf-chip");
  chip.type = "button";
  chip.id = "mlf-project";
  chip.addEventListener("click", () => void mlfOpenProjectSheet());
  projRow.appendChild(chip);

  const body = saiEl("main", "mlf-body sai-split");
  body.id = "mlf-body";
  const list = saiEl("div", "sai-split-list");
  list.id = "mlf-list";
  const main = saiEl("div", "sai-split-main");
  main.id = "mlf-main";
  body.append(list, main);

  root.append(bar, projRow, body);
  mlfMounted = true;
}

/** 画面の幅が 900px 以上か。 */
function mlfIsWideNow() {
  return typeof window === "undefined" ? false : window.innerWidth >= MLF_WIDE_PX;
}

/** 見出しと「戻る」を、今の画面に合わせる。 */
function mlfSyncBar(title) {
  const head = document.getElementById("mlf-title");
  const back = document.getElementById("mlf-back");
  if (head) head.textContent = title || "Experiments";
  if (back) back.hidden = !title || title === "Experiments";
}

// ============================================================================
// プロジェクトの選び方
// ============================================================================

/** 覚えている選択を読む（無ければ null）。 */
function mlfLoadProject() {
  try {
    const raw = localStorage.getItem(MLF_PROJECT_KEY);
    if (raw == null) return null;
    return raw === MLF_PROJECT_ALL ? "" : raw;
  } catch {
    return null; // 保存できない環境でも動く
  }
}

/** 選択を覚える。 */
function mlfSaveProject(name) {
  mlfProject = name;
  try {
    localStorage.setItem(MLF_PROJECT_KEY, name === "" ? MLF_PROJECT_ALL : name);
  } catch { /* 覚えられなくても動く */ }
}

/** サーバーへ送るプロジェクト名（すべてなら null＝送らない）。 */
function mlfProjectName() {
  if (mlfProject == null) return null;
  return mlfProject === "" ? null : mlfProject;
}

/** 上の札の文言。 */
function mlfProjectLabel() {
  if (mlfProject == null) return "プロジェクトを選ぶ";
  return mlfProject === "" ? "すべてのプロジェクト" : mlfProject;
}

/** 上の札を描き直す。 */
function mlfRenderProjectChip() {
  const chip = document.getElementById("mlf-project");
  if (!chip) return;
  chip.textContent = `📁 ${mlfProjectLabel()} ▾`;
  chip.hidden = mlfProjects == null; // 読んでから出す
}

/** プロジェクト一覧を読む。force が true のときは読み直す。 */
async function mlfEnsureProjects(force) {
  if (mlfProjects != null && !force) return;
  if (force || mlfProjects == null) {
    try {
      const data = await saiApi("/api/projects");
      mlfProjects = data && Array.isArray(data.projects) ? data : { projects: [] };
      mlfProjectError = "";
    } catch (e) {
      mlfProjectError = (e && e.message) || "プロジェクト一覧を読めませんでした";
      mlfProjects = { projects: [] };
    }
  }
  const saved = mlfLoadProject();
  if (saved != null) {
    mlfSaveProject(saved);
  } else {
    // 既定は一覧の最初（1 つも無ければ「すべて」）
    const first = (mlfProjects.projects || []).find((one) => one && one.name);
    mlfSaveProject(first ? first.name : "");
  }
  mlfRenderProjectChip();
}

/** プロジェクトを選ぶ板を出す。 */
async function mlfOpenProjectSheet() {
  await mlfEnsureProjects(true);
  const body = saiEl("div", "mlf-sheet");
  for (const one of mlfProjectChoices(mlfProjects)) {
    const btn = saiEl("button", `mlf-sheet-item${one.name === mlfProject ? " active" : ""}`);
    btn.type = "button";
    btn.append(saiEl("span", "mlf-sheet-check", one.name === mlfProject ? "✓" : ""));
    const col = saiEl("span", "mlf-sheet-text");
    col.appendChild(saiEl("span", "mlf-sheet-name", one.label));
    if (one.note) col.appendChild(saiEl("span", "mlf-sheet-note", String(one.note)));
    btn.appendChild(col);
    btn.addEventListener("click", () => {
      mlfSaveProject(one.name);
      mlfRenderProjectChip();
      sheet.close();
      void mlfOpenPage(mlfState);
    });
    body.appendChild(btn);
  }
  if (mlfProjectError) body.appendChild(saiEl("p", "mlf-note", mlfProjectError));
  const sheet = saiSheet({ title: "プロジェクトを選ぶ", body });
}

// ============================================================================
// 実験の一覧（#/mlflow）
// ============================================================================

/** 実験の一覧を読む。 */
async function mlfLoadExperiments() {
  try {
    const data = await saiApi("/api/mlflow/experiments");
    mlfExperiments = data && Array.isArray(data.experiments) ? data.experiments : [];
    mlfExperimentsError = "";
  } catch (e) {
    mlfExperiments = [];
    mlfExperimentsError = (e && e.message) || "実験の一覧を読めませんでした";
  }
}

/** 実験の一覧を描く。 */
function mlfRenderExperiments() {
  const box = document.getElementById("mlf-list");
  if (!box) return;
  box.innerHTML = "";
  box.classList.remove("mlf-narrow-list");
  const body = document.getElementById("mlf-body");
  if (body) {
    body.classList.add("mlf-solo");
    saiSplitShow(body, "list");
  }
  mlfSyncBar("Experiments");

  if (mlfExperimentsError) {
    box.appendChild(saiEmpty({ icon: "⚠", title: "実験の一覧を読めませんでした", body: mlfExperimentsError }));
    return;
  }
  const all = saiEl("button", "mlf-exp");
  all.type = "button";
  all.append(
    saiEl("span", "mlf-exp-name", "すべての実験"),
    saiEl("span", "mlf-exp-note", "すべての実験を 1 つの表にまとめて見ます")
  );
  all.addEventListener("click", () => { location.hash = mlfHash({ page: "runs", experimentId: "all" }); });
  box.appendChild(all);

  const list = mlfExperiments || [];
  if (list.length === 0) {
    box.appendChild(
      saiEmpty({
        icon: "🧪",
        title: "まだ実験がありません",
        body: "MLflow に記録されるようになると、ここに並びます。",
      })
    );
    return;
  }
  for (const one of list) {
    const id = one && one.experiment_id != null ? String(one.experiment_id) : "";
    const btn = saiEl("button", "mlf-exp");
    btn.type = "button";
    btn.append(
      saiEl("span", "mlf-exp-name", (one && one.name) || `実験 ${id}`),
      saiEl("span", "mlf-exp-note", `作った日 ${mlfDayText(one && one.creation_time)} ・ 最後に更新 ${mlfDayText(one && one.last_update_time)}`)
    );
    btn.addEventListener("click", () => {
      if (!MLF_EXPERIMENT_ID_RE.test(id)) return;
      location.hash = mlfHash({ page: "runs", experimentId: id });
    });
    box.appendChild(btn);
  }
}

// ============================================================================
// run の表（#/mlflow/e/<実験 ID>）
// ============================================================================

/** 検索の本文を組み立てる。 */
function mlfSearchBody(experimentId, query, orderBy, pageToken) {
  const ids = experimentId === "all"
    ? (mlfExperiments || []).map((one) => String(one && one.experiment_id)).filter((id) => MLF_EXPERIMENT_ID_RE.test(id))
    : [String(experimentId)];
  const body = {
    experiment_ids: ids,
    filter: mlfFilterFromQuery(query),
    order_by: [orderBy || "attributes.start_time DESC"],
    max_results: MLF_PAGE_SIZE,
  };
  const project = mlfProjectName();
  if (project) body.project = project;
  if (pageToken) body.page_token = pageToken;
  return body;
}

/** 検索の本文を送って、行を足す。 */
async function mlfLoadRuns(reset) {
  if (!mlfTable) return;
  if (reset) {
    mlfTable.runs = [];
    mlfTable.nextPageToken = "";
    mlfTable.error = "";
  }
  mlfTable.loading = true;
  mlfRenderRuns();
  try {
    const body = mlfSearchBody(mlfTable.experimentId, mlfTable.query, mlfTable.orderBy, reset ? "" : mlfTable.nextPageToken);
    const data = await saiApi("/api/mlflow/runs/search", { method: "POST", body });
    const rows = data && Array.isArray(data.runs) ? data.runs : [];
    if (reset) {
      mlfTable.runs = rows;
    } else {
      const seen = new Set(mlfTable.runs.map((r) => String(r && r.info && r.info.run_id)));
      for (const run of rows) {
        const id = String(run && run.info && run.info.run_id);
        if (seen.has(id)) continue;
        seen.add(id);
        mlfTable.runs.push(run);
      }
    }
    mlfTable.nextPageToken = (data && data.next_page_token) || "";
    mlfTable.error = "";
  } catch (e) {
    mlfTable.error = (e && e.message) || "run を読み込めませんでした";
  } finally {
    mlfTable.loading = false;
    mlfRenderRuns();
  }
}

/** 絞り込みの札（消すとその条件だけ外す）。 */
function mlfRenderFilterChips() {
  const q = mlfTable.query || {};
  const labels = { sai_run: "sai の実行", seed: "seed", study: "研究", status: "状態" };
  let any = false;
  const row = saiEl("div", "mlf-chips");
  for (const key of MLF_QUERY_KEYS) {
    const value = q[key];
    if (value == null || value === "") continue;
    any = true;
    const chip = saiEl("button", "mlf-chip small");
    chip.type = "button";
    const text = key === "status" ? `${labels[key]} ${mlfStatusText(value)}` : `${labels[key]} ${value}`;
    chip.setAttribute("aria-label", `${text}の絞り込みを外す`);
    chip.append(saiEl("span", "", text), saiEl("span", "mlf-chip-x", "✕"));
    const next = { ...q };
    delete next[key];
    chip.addEventListener("click", () => {
      location.hash = mlfHash({ page: "runs", experimentId: mlfTable.experimentId, query: next });
    });
    row.appendChild(chip);
  }
  if (any) {
    const clear = saiEl("button", "mlf-chip small ghost", "条件をすべて外す");
    clear.type = "button";
    clear.addEventListener("click", () => {
      location.hash = mlfHash({ page: "runs", experimentId: mlfTable.experimentId, query: {} });
    });
    row.appendChild(clear);
  }
  return any ? row : null;
}

/** 並べ替えの札（押すと選択肢の板が出る）。 */
function mlfRenderSortChip() {
  const found = mlfOrderOptions(mlfTable.runs).find((one) => one[0] === mlfTable.orderBy);
  const label = found ? found[1] : "開始（新しい順）";
  const btn = saiEl("button", "mlf-chip small");
  btn.type = "button";
  btn.setAttribute("aria-label", "並び順を変える");
  btn.append(saiEl("span", "", `並び ${label}`), saiEl("span", "mlf-chip-x", "▾"));
  btn.addEventListener("click", () => mlfOpenSortSheet());
  return btn;
}

/** 並べ替えの板。 */
function mlfOpenSortSheet() {
  const body = saiEl("div", "mlf-sheet");
  for (const [order, label] of mlfOrderOptions(mlfTable.runs)) {
    const btn = saiEl("button", `mlf-sheet-item${order === mlfTable.orderBy ? " active" : ""}`);
    btn.type = "button";
    btn.append(
      saiEl("span", "mlf-sheet-check", order === mlfTable.orderBy ? "✓" : ""),
      saiEl("span", "mlf-sheet-text", label)
    );
    btn.addEventListener("click", () => {
      sheet.close();
      mlfTable.orderBy = order;
      void mlfLoadRuns(true);
    });
    body.appendChild(btn);
  }
  const sheet = saiSheet({ title: "並び順", body });
}

/**
 * 1 本の run の行（狭い画面では札、広い画面では表の 1 行）。
 * o.pick が true のときは左端に比較のためのチェックが付く（run の表だけ）。
 */
function mlfRunRow(run, opts) {
  const o = opts || {};
  const info = (run && run.info) || {};
  const wide = o.wide === true;
  const wrap = saiEl("div", wide ? "mlf-trow" : "mlf-tcard");
  if (o.nested) wrap.classList.add("nested");
  const runId = String(info.run_id || "");
  if (o.pick) {
    wrap.classList.add("pickable");
    wrap.dataset.runId = runId;
    wrap.appendChild(mlfPickCheck(runId, info.run_name));
  }
  const name = saiEl("span", "mlf-c-name", info.run_name || info.run_id || "—");
  const status = saiEl("span", `mlf-status ${mlfStatusClass(info.status)}`, mlfStatusText(info.status));
  if (wide) {
    wrap.appendChild(name);
  } else {
    const head = saiEl("div", "mlf-tcard-head");
    head.append(name, status);
    wrap.appendChild(head);
  }
  const metric = mlfHeadMetric(run);
  const metricCell = saiEl("span", "mlf-c-metric");
  if (metric) {
    metricCell.append(
      saiEl("span", "mlf-c-metric-key", metric.key),
      saiEl("span", "mlf-c-metric-val", mlfValueText(metric.value))
    );
  } else if (!wide) {
    metricCell.appendChild(saiEl("span", "mlf-c-metric-key", "指標なし"));
  }
  const params = ((run && run.data && run.data.params) || []).filter((one) => one && one.key != null).slice(0, wide ? 8 : 3);
  const paramCell = saiEl("span", "mlf-c-params");
  for (const one of params) {
    paramCell.appendChild(saiEl("span", "mlf-tagline", `${one.key} = ${one.value}`));
  }
  wrap.append(
    saiEl("span", "mlf-c-time", mlfTimeText(info.start_time)),
    saiEl("span", "mlf-c-dur", mlfDurationText(mlfDurationMs(run))),
    metricCell,
    paramCell
  );
  if (wide) wrap.appendChild(status);
  wrap.addEventListener("click", () => {
    if (MLF_RUN_ID_RE.test(runId)) location.hash = mlfHash({ page: "run", runId });
  });
  return wrap;
}

/** 比較のためのチェック（押しても run の詳細には跳ばない）。 */
function mlfPickCheck(runId, label) {
  const on = mlfTable && mlfTable.pick && mlfTable.pick.has(runId);
  const box = saiEl("button", `mlf-pick${on ? " on" : ""}`);
  box.type = "button";
  box.setAttribute("role", "checkbox");
  box.setAttribute("aria-checked", on ? "true" : "false");
  box.setAttribute("aria-label", `${label || "この run"}を比較加える`);
  box.textContent = on ? "✓" : "";
  box.addEventListener("click", (e) => {
    e.stopPropagation();
    mlfTogglePick(runId);
  });
  return box;
}

/** 比較のために選ぶ run を加える・外す（2〜20 件・C-07）。 */
function mlfTogglePick(runId) {
  if (!mlfTable || !MLF_RUN_ID_RE.test(String(runId || ""))) return;
  if (!mlfTable.pick) mlfTable.pick = new Set();
  const pick = mlfTable.pick;
  if (pick.has(runId)) {
    pick.delete(runId);
  } else {
    if (pick.size >= MLF_COMPARE_MAX) {
      saiToast(`比較は ${MLF_COMPARE_MAX} 件までです。1 つ選び直してください。`, "warn");
      return;
    }
    pick.add(runId);
  }
  mlfRenderRuns();
}

/** 選択した run の番号（選ぶ順に）。 */
function mlfPickIds() {
  if (!mlfTable || !mlfTable.pick) return [];
  return [...mlfTable.pick].filter((id) => MLF_RUN_ID_RE.test(id));
}

/** 一番下に固定される帯（選んだ件数と「比べる」ボタン）。 */
function mlfPickBar() {
  const ids = mlfPickIds();
  if (ids.length === 0) return null;
  const bar = saiEl("div", "mlf-pickbar");
  const can = ids.length >= MLF_COMPARE_MIN;
  const btn = saiEl("button", "sai-btn primary", `${ids.length} 件を比較`);
  btn.type = "button";
  btn.disabled = !can;
  if (!can) btn.title = `比較は ${MLF_COMPARE_MIN} 件以上から`;
  btn.addEventListener("click", () => {
    const list = mlfPickIds();
    if (list.length < MLF_COMPARE_MIN || list.length > MLF_COMPARE_MAX) return;
    const hash = mlfHash({ page: "compare", runIds: list });
    if (hash) location.hash = hash;
  });
  const clear = saiEl("button", "mlf-pick-clear", "選び直す");
  clear.type = "button";
  clear.addEventListener("click", () => {
    if (mlfTable) mlfTable.pick = new Set();
    mlfRenderRuns();
  });
  bar.append(
    saiEl("span", "mlf-pick-note", can ? "選んでいます" : `${MLF_COMPARE_MIN} 件以上選んでください`),
    clear,
    btn
  );
  return bar;
}

/** 束の見出し（開閉できる）。 */
function mlfBundleHead(bundle, open, count) {
  const btn = saiEl("button", "mlf-bundle");
  btn.type = "button";
  btn.setAttribute("aria-expanded", open ? "true" : "false");
  btn.append(
    saiEl("span", "mlf-bundle-caret", open ? "▾" : "▸"),
    saiEl("span", "mlf-bundle-name", bundle.label),
    saiEl("span", "mlf-bundle-count", `${count} 本`)
  );
  btn.addEventListener("click", (e) => {
    e.stopPropagation();
    if (mlfOpen.has(bundle.key)) mlfOpen.delete(bundle.key);
    else mlfOpen.add(bundle.key);
    mlfRenderRuns();
  });
  return btn;
}

/** 入れ子の表を描く。 */
function mlfRenderTree(box) {
  const bundles = mlfBuildTree(mlfTable.runs);
  if (bundles.length === 0) {
    box.appendChild(
      saiEmpty({
        icon: "🔍",
        title: "当てはまる run がありません",
        body: "絞り込みの条件が正しいかも確かめてください。",
      })
    );
    return;
  }
  for (const bundle of bundles) {
    const group = saiEl("div", "mlf-bundle-box");
    const count = bundle.rows.reduce((sum, row) => sum + 1 + row.children.length, 0);
    const open = mlfOpen.size === 0 ? true : mlfOpen.has(bundle.key);
    group.appendChild(mlfBundleHead(bundle, open, count));
    if (open) {
      const rowsBox = saiEl("div", "mlf-rows");
      for (const row of bundle.rows) {
        if (row.kind === "orphan") {
          const card = saiEl("div", "mlf-orphan");
          card.appendChild(mlfRunRow(row.run, { pick: true }));
          card.appendChild(saiEl("p", "mlf-note tiny", "親は読み込まれていません"));
          rowsBox.appendChild(card);
          continue;
        }
        const key = `${bundle.key}:${String(row.run.info.run_id)}`;
        const open2 = mlfOpen.size === 0 ? true : mlfOpen.has(key);
        const box2 = saiEl("div", "mlf-parent");
        if (row.children.length) {
          const head = saiEl("button", "mlf-parent-head");
          head.type = "button";
          head.setAttribute("aria-expanded", open2 ? "true" : "false");
          head.append(
            saiEl("span", "mlf-bundle-caret", open2 ? "▾" : "▸"),
            saiEl("span", "mlf-parent-name", (row.run.info && row.run.info.run_name) || row.run.info.run_id),
            saiEl("span", "mlf-bundle-count", `${row.children.length} 試行`)
          );
          head.addEventListener("click", (e) => {
            e.stopPropagation();
            if (mlfOpen.has(key)) mlfOpen.delete(key);
            else mlfOpen.add(key);
            mlfRenderRuns();
          });
          box2.appendChild(head);
        }
        box2.appendChild(mlfRunRow(row.run, { pick: true }));
        if (open2) {
          const kids = saiEl("div", "mlf-children");
          for (const child of row.children) kids.appendChild(mlfRunRow(child, { nested: true, pick: true }));
          box2.appendChild(kids);
        }
        rowsBox.appendChild(box2);
      }
      group.appendChild(rowsBox);
    }
    box.appendChild(group);
  }
}

/** 平らな行（指標で並べ替えたとき）。 */
function mlfRenderFlat(box) {
  const rows = mlfFlatRows(mlfTable.runs);
  if (rows.length === 0) {
    box.appendChild(
      saiEmpty({ icon: "🔍", title: "当てはまる run がありません", body: "絞り込みの条件が正しいかも確かめてください。" })
    );
    return;
  }
  box.appendChild(saiEl("p", "mlf-note tiny", "指標で並べたので親子の入れ子は解いています（MLflow の画面と同じ扱い）。"));
  const listBox = saiEl("div", "mlf-rows");
  for (const one of rows) {
    const wrap = saiEl("div", "mlf-parent");
    wrap.appendChild(mlfRunRow(one.run, { nested: one.depth === 1, pick: true }));
    listBox.appendChild(wrap);
  }
  box.appendChild(listBox);
}

/** run の表を描く。 */
function mlfRenderRuns() {
  const box = document.getElementById("mlf-list");
  if (!box || !mlfTable) return;
  box.innerHTML = "";
  const body = document.getElementById("mlf-body");
  if (body) body.classList.remove("mlf-solo");

  const chips = saiEl("div", "mlf-chips");
  const filterChips = mlfRenderFilterChips();
  if (filterChips) chips.appendChild(filterChips);
  chips.appendChild(mlfRenderSortChip());
  box.appendChild(chips);
  box.appendChild(saiEl("p", "mlf-note tiny", "比べる run は左の印で選べます（2〜20 件）。"));

  if (mlfTable.error && mlfTable.runs.length === 0) {
    box.appendChild(saiEmpty({ icon: "⚠", title: "run を読めませんでした", body: mlfTable.error }));
    return;
  }
  if (mlfTable.runs.length === 0) {
    if (mlfTable.loading) {
      box.appendChild(saiEl("p", "mlf-note", "読み込んでいます…"));
      return;
    }
    mlfRenderTree(box);
    return;
  }
  const listBox = saiEl("div", "mlf-table");
  if (mlfIsMetricOrder(mlfTable.orderBy)) mlfRenderFlat(listBox);
  else mlfRenderTree(listBox);
  box.appendChild(listBox);
  const bar = mlfPickBar();
  if (bar) box.appendChild(bar);
  if (mlfTable.nextPageToken) {
    const more = saiEl("button", "sai-btn primary mlf-more", mlfTable.loading ? "読み込んでいます…" : "続きを読む");
    more.type = "button";
    more.disabled = !!mlfTable.loading;
    more.addEventListener("click", () => void mlfLoadRuns(false));
    box.appendChild(more);
  }
}

// ============================================================================
// run の詳細（#/mlflow/run/<run ID>）
// ============================================================================

/** 詳細の画面を出す。 */
async function mlfShowRun(params) {
  const runId = params.runId;
  mlfDetail = { runId, run: null, children: null, error: "", xKind: "step", charts: null, chartError: "", chartLoading: false };
  mlfArt = { runId, path: "", files: [], rootUri: "", loaded: false, loading: false, error: "", open: false, file: null };
  mlfSyncBar("run の詳細");
  // 広い画面では左の表を残したいので、直前の表を出し直す
  if (mlfTable) mlfRenderRuns();
  const box = document.getElementById("mlf-main");
  if (box) {
    box.innerHTML = "";
    box.appendChild(saiEl("p", "mlf-note", "読み込んでいます…"));
  }
  try {
    const data = await saiApi(`/api/mlflow/runs/${encodeURIComponent(runId)}`);
    mlfDetail.run = (data && data.run) || null;
    mlfDetail.error = mlfDetail.run ? "" : "run が返ってきませんでした";
  } catch (e) {
    mlfDetail.error = (e && e.message) || "run を読み込めませんでした";
  }
  mlfRenderRun();
  if (mlfDetail.run) {
    const info = mlfDetail.run.info || {};
    const experimentId = info.experiment_id != null ? String(info.experiment_id) : "";
    if (MLF_EXPERIMENT_ID_RE.test(experimentId)) {
      // 子（`mlflow.parentRunId` がこの run のもの）
      const body = {
        experiment_ids: [experimentId],
        filter: `tags."${MLF_TAGS.parent}" = "${mlfEscape(runId)}"`,
        order_by: ["attributes.start_time ASC"],
        max_results: MLF_PAGE_SIZE,
      };
      const project = mlfProjectName();
      if (project) body.project = project;
      try {
        const data = await saiApi("/api/mlflow/runs/search", { method: "POST", body });
        mlfDetail.children = data && Array.isArray(data.runs) ? data.runs : [];
      } catch {
        mlfDetail.children = [];
      }
    }
    mlfRenderRun();
  }
  await mlfLoadCharts();
}

/**
 * 詳細の指標の時系列を読む。
 *   子の無い run: 1 本ずつ get-history
 *   子のある run: 子のまとめて get-history-bulk-interval（曲線を重ねる）
 * 間引きは MLflow のサーバに任せる（最初に記録した点と最後の点を残して間引くため）。
 * 画面では間引かない。
 */
async function mlfLoadCharts() {
  const d = mlfDetail;
  if (!d || !d.run) return;
  const children = Array.isArray(d.children) ? d.children : [];
  const childIds = children
    .map((one) => String((one && one.info && one.info.run_id) || ""))
    .filter((id) => MLF_RUN_ID_RE.test(id));
  // 子の指標の和集合。子の無いときは自分の指標
  const keys = childIds.length ? mlfMetricKeysOf(children) : mlfMetricKeys(d.run);
  if (keys.length === 0) {
    d.charts = null;
    return;
  }
  d.charts = {};
  d.chartError = "";
  d.chartLoading = true;
  mlfRenderRun();
  for (const key of keys) {
    if (!mlfDetail || mlfDetail.runId !== d.runId) return; // 画面を離れた
    try {
      let metrics = [];
      if (childIds.length) {
        const qs = new URLSearchParams();
        for (const id of childIds.slice(0, MLF_BULK_RUNS)) qs.append("run_ids", id);
        qs.set("metric_key", key);
        qs.set("max_results", String(MLF_BULK_MAX));
        const data = await saiApi(`/api/mlflow/metrics/bulk?${qs.toString()}`);
        metrics = data && Array.isArray(data.metrics) ? data.metrics : [];
      } else {
        const qs = new URLSearchParams({
          run_id: d.runId,
          metric_key: key,
          max_results: String(MLF_HISTORY_MAX),
        });
        const data = await saiApi(`/api/mlflow/metrics/history?${qs.toString()}`);
        metrics = data && Array.isArray(data.metrics) ? data.metrics : [];
      }
      if (!mlfDetail || mlfDetail.runId !== d.runId) return;
      d.charts[key] = { series: mlfSeriesFromMetrics(metrics, d.xKind), metrics };
    } catch (e) {
      if (!mlfDetail || mlfDetail.runId !== d.runId) return;
      d.charts[key] = { series: [], metrics: [], error: (e && e.message) || "指標の時系列を読めませんでした" };
    }
    mlfRenderRun();
  }
  if (!mlfDetail || mlfDetail.runId !== d.runId) return;
  d.chartLoading = false;
  mlfRenderRun();
}

/** x の種類（step か経過時間か）を入れ替える。読み直さずに描き直す。 */
function mlfToggleXKind(kind) {
  if (!mlfDetail || !mlfDetail.charts) return;
  const next = kind === "time" ? "time" : "step";
  if (mlfDetail.xKind === next) return;
  mlfDetail.xKind = next;
  for (const key of Object.keys(mlfDetail.charts)) {
    const one = mlfDetail.charts[key];
    one.series = mlfSeriesFromMetrics(one.metrics, next);
  }
  mlfRenderRun();
}

/** 値を並んだ 2 つの欄で 1 行にする。 */
function mlfKvRow(key, value) {
  const row = saiEl("div", "mlf-kv-row");
  row.append(saiEl("span", "mlf-kv-k", key), saiEl("span", "mlf-kv-v", value == null || value === "" ? "—" : String(value)));
  return row;
}

/** 詳細を描く。 */
function mlfRenderRun() {
  const box = document.getElementById("mlf-main");
  if (!box || !mlfDetail) return;
  const body = document.getElementById("mlf-body");
  if (body) {
    // 広い画面で表が残っていないときは、詳細だけを全面に出す
    body.classList.toggle("mlf-solo", !mlfTable);
    saiSplitShow(body, "main");
  }
  box.innerHTML = "";
  if (mlfDetail.error) {
    box.appendChild(saiEmpty({ icon: "⚠", title: "run を読み込めませんでした", body: mlfDetail.error }));
    return;
  }
  const run = mlfDetail.run;
  if (!run) return;
  const info = run.info || {};
  const data = run.data || {};

  const head = saiEl("div", "mlf-detail");
  head.append(
    saiEl("h2", "mlf-detail-name", info.run_name || info.run_id || "—"),
    saiEl("p", "mlf-detail-sub", `${mlfStatusText(info.status)} ・ ${mlfTimeText(info.start_time)} ・ ${mlfDurationText(mlfDurationMs(run))}`)
  );
  box.appendChild(head);

  const kv = saiEl("div", "mlf-kv");
  kv.append(
    mlfKvRow("Experiments", info.experiment_id != null ? String(info.experiment_id) : ""),
    mlfKvRow("run ID", info.run_id || ""),
    mlfKvRow("開始", mlfTimeText(info.start_time)),
    mlfKvRow("終了", mlfTimeText(info.end_time)),
    mlfKvRow("時間", mlfDurationText(mlfDurationMs(run))),
    mlfKvRow("作った人", info.user || "")
  );
  box.appendChild(kv);

  // --- 行き来のリンク ---
  const links = saiEl("div", "mlf-links");
  const addLink = (label, hash) => {
    if (!hash) return;
    const btn = saiEl("button", "mlf-link");
    btn.type = "button";
    btn.append(saiEl("span", "mlf-link-arrow", "→"), saiEl("span", "", label));
    btn.addEventListener("click", () => { location.hash = hash; });
    links.appendChild(btn);
  };
  const saiRunId = mlfTag(run, MLF_TAGS.saiRun);
  if (saiRunId) {
    // プロジェクトの作りは run に付けた `sai.project` を優先する（無ければいま選んでいるもの）
    const own = mlfTag(run, MLF_TAGS.saiProject);
    const project = own || mlfProjectName() || ((mlfProjects && mlfProjects.projects[0]) || {}).name || "";
    addLink(`sai の実行 #${saiRunId} を開く`, `#/run/${encodeURIComponent(project)}/${encodeURIComponent(saiRunId)}`);
  }
  const seedId = mlfSeedId(run);
  if (seedId) addLink(`seed #${seedId} の run を見る`, mlfHash({ page: "runs", experimentId: "all", query: { seed: seedId } }));
  const sciNode = mlfTag(run, MLF_TAGS.sciNode);
  if (sciNode) addLink(`Research tree ${sciNode}`, `#/papers/tree/${encodeURIComponent(sciNode)}`);
  const parentId = mlfParentId(run);
  if (parentId) addLink("親を開く", mlfHash({ page: "run", runId: parentId }));
  if (links.children.length) box.appendChild(links);

  // --- 子 ---
  if (mlfDetail.children) {
    const sec = saiEl("div", "mlf-section");
    sec.appendChild(saiEl("h3", "mlf-section-title", `この run の子（${mlfDetail.children.length}）`));
    if (mlfDetail.children.length === 0) {
      sec.appendChild(saiEl("p", "mlf-note", "子の run はありません。"));
    } else {
      const rows = saiEl("div", "mlf-rows");
      for (const child of mlfDetail.children) rows.appendChild(mlfRunRow(child, {}));
      sec.appendChild(rows);
    }
    box.appendChild(sec);
  }

  // --- 設定（param） ---
  const params = (data.params || []).filter((one) => one && one.key != null);
  if (params.length) {
    const sec = saiEl("div", "mlf-section");
    sec.appendChild(saiEl("h3", "mlf-section-title", `設定（param）${params.length} 件`));
    const table = saiEl("div", "mlf-table-box");
    for (const one of params) table.appendChild(mlfKvRow(String(one.key), one.value));
    sec.appendChild(table);
    box.appendChild(sec);
  }

  // --- 指標（metric・最後の値） ---
  const metrics = mlfMetricList(run);
  if (metrics.length) {
    const sec = saiEl("div", "mlf-section");
    sec.appendChild(saiEl("h3", "mlf-section-title", `指標（metric）${metrics.length} 件`));
    const table = saiEl("div", "mlf-table-box");
    for (const one of metrics) table.appendChild(mlfKvRow(one.key, mlfValueText(one.value)));
    sec.appendChild(table);
    box.appendChild(sec);
  }

  // --- 指標のグラフ（時系列） ---
  if (metrics.length || mlfDetail.children) box.appendChild(mlfChartsBox());

  // --- タグ（`mlflow.` で始まるものは畳む） ---
  const tags = (data.tags || []).filter((one) => one && one.key != null);
  const own = tags.filter((one) => !String(one.key).startsWith("mlflow."));
  const given = tags.filter((one) => String(one.key).startsWith("mlflow."));
  if (own.length) {
    const sec = saiEl("div", "mlf-section");
    sec.appendChild(saiEl("h3", "mlf-section-title", `タグ ${own.length} 件`));
    const table = saiEl("div", "mlf-table-box");
    for (const one of own) table.appendChild(mlfKvRow(String(one.key), one.value));
    sec.appendChild(table);
    box.appendChild(sec);
  }
  if (given.length) {
    const sec = saiEl("div", "mlf-section");
    const fold = saiEl("button", "mlf-fold");
    fold.type = "button";
    fold.setAttribute("aria-expanded", "false");
    const caret = saiEl("span", "mlf-bundle-caret", "▸");
    fold.append(caret, saiEl("span", "", `MLflow が付けたタグ ${given.length} 件`));
    const inner = saiEl("div", "mlf-table-box");
    inner.hidden = true;
    for (const one of given) inner.appendChild(mlfKvRow(String(one.key), one.value));
    fold.addEventListener("click", () => {
      inner.hidden = !inner.hidden;
      fold.setAttribute("aria-expanded", inner.hidden ? "false" : "true");
      caret.textContent = inner.hidden ? "▸" : "▾";
    });
    sec.append(fold, inner);
    box.appendChild(sec);
  }

  box.appendChild(mlfArtifactsBox());
}

/**
 * 指標のグラフの節（詳細画面）。
 * 上に「step / 経過時間」の切り替えを置き（既定は step）、
 * metric ごとに 1 つずつ折れ線グラフを出す。
 */
function mlfChartsBox() {
  const d = mlfDetail;
  const sec = saiEl("div", "mlf-section");
  const head = saiEl("div", "mlf-charts-head");
  head.appendChild(saiEl("h3", "mlf-section-title", "グラフ（時系列）"));
  const toggle = saiEl("div", "mlf-seg");
  for (const [kind, label] of [["step", "step"], ["time", "経過時間"]]) {
    const btn = saiEl("button", `mlf-seg-btn${(d && d.xKind) === kind ? " active" : ""}`);
    btn.type = "button";
    btn.textContent = label;
    btn.addEventListener("click", () => mlfToggleXKind(kind));
    toggle.appendChild(btn);
  }
  head.appendChild(toggle);
  sec.appendChild(head);

  if (!d || !d.charts) {
    sec.appendChild(saiEl("p", "mlf-note", "この run には指標の記録がありません。"));
    return sec;
  }
  if (d.chartError) {
    sec.appendChild(saiEl("p", "mlf-note", d.chartError));
    return sec;
  }
  const keys = Object.keys(d.charts);
  if (keys.length === 0) {
    sec.appendChild(saiEl("p", "mlf-note", d.chartLoading ? "読み込んでいます…" : "この run には指標の記録がありません。"));
    return sec;
  }

  const grid = saiEl("div", "mlf-chart-grid");
  for (const key of keys) {
    const one = d.charts[key] || {};
    const cell = saiEl("div", "mlf-chart-cell");
    cell.appendChild(saiEl("h4", "mlf-chart-title", key));
    const body = saiEl("div", "mlf-chart-body");
    if (one.error) {
      body.appendChild(saiEl("p", "mlf-note tiny", one.error));
    } else if (d.chartLoading && !mlfHasPoints(one.series)) {
      body.appendChild(saiEl("p", "mlf-note tiny", "読み込んでいます…"));
    } else if (!mlfHasPoints(one.series)) {
      // 最後の値しか記録されていない（get-history が 1 点しか返さない）とき
      body.appendChild(saiEl("p", "mlf-note tiny", "時系列の記録がありません（最後の値だけ）"));
    } else {
      const labeled = mlfLabelSeries(one.series, d.children, d.run);
      mlfLineChart(body, { series: labeled, xKind: d.xKind, title: key });
    }
    cell.appendChild(body);
    grid.appendChild(cell);
  }
  sec.appendChild(grid);
  if (d.chartLoading) sec.appendChild(saiEl("p", "mlf-note tiny", "指標を読み込んでいます…"));
  return sec;
}

/**
 * series に run の名前（無いものは run_id の先頭 8 字）を付ける。
 * run_id が無い系列（自分の履歴）は、fallback（その run 自身）の名前を 1 つだけ使う。
 */
function mlfLabelSeries(series, children, fallback) {
  const names = new Map();
  for (const one of Array.isArray(children) ? children : []) {
    const id = String((one && one.info && one.info.run_id) || "");
    if (!id) continue;
    const info = one.info || {};
    names.set(id, info.run_name || id.slice(0, 8));
  }
  const fb = (fallback && fallback.info) || {};
  const fbName = fb.run_name || String(fb.run_id || "").slice(0, 8) || "";
  return (Array.isArray(series) ? series : []).map((one, i) => {
    const id = one.runId || "";
    let label = names.get(id) || "";
    if (!label && !id) label = fbName;
    if (!label) label = id ? id.slice(0, 8) : `run ${i + 1}`;
    return { runId: id, label, points: one.points || [] };
  });
}

/** 成果物の欄（開くまでは中身を読まない）。 */
function mlfArtifactsBox() {
  const sec = saiEl("div", "mlf-section");
  const fold = saiEl("button", "mlf-fold");
  fold.type = "button";
  fold.setAttribute("aria-expanded", mlfArt && mlfArt.open ? "true" : "false");
  const caret = saiEl("span", "mlf-bundle-caret", mlfArt && mlfArt.open ? "▾" : "▸");
  fold.append(caret, saiEl("span", "", "成果物"));
  const inner = saiEl("div", "mlf-art");
  inner.hidden = !(mlfArt && mlfArt.open);
  fold.addEventListener("click", () => {
    const openNow = inner.hidden;
    if (!mlfArt) return;
    mlfArt.open = openNow;
    inner.hidden = !openNow;
    fold.setAttribute("aria-expanded", openNow ? "true" : "false");
    caret.textContent = openNow ? "▾" : "▸";
    if (openNow && !mlfArt.loaded) void mlfLoadArtifacts("");
  });
  sec.append(fold, inner);
  if (mlfArt && mlfArt.open) mlfRenderArtifacts();
  return sec;
}

// ============================================================================
// 成果物
// ============================================================================

/** フォルダの中身を読む。 */
async function mlfLoadArtifacts(path) {
  if (!mlfArt) return;
  mlfArt.path = path || "";
  mlfArt.loading = true;
  mlfArt.error = "";
  mlfRenderArtifacts();
  try {
    const qs = new URLSearchParams({ run_id: mlfArt.runId });
    if (mlfArt.path) qs.set("path", mlfArt.path);
    const data = await saiApi(`/api/mlflow/artifacts?${qs.toString()}`);
    mlfArt.files = data && Array.isArray(data.files) ? data.files : [];
    mlfArt.rootUri = (data && data.root_uri) || "";
    mlfArt.loaded = true;
  } catch (e) {
    mlfArt.error = (e && e.message) || "成果物を読めませんでした";
    mlfArt.files = [];
  } finally {
    mlfArt.loading = false;
    mlfRenderArtifacts();
  }
}

/** 成果物 1 件の本文を読む。 */
async function mlfLoadArtifactFile(path) {
  if (!mlfArt) return;
  const kind = mlfArtifactKind(path);
  mlfArt.file = { path, kind, text: "", url: "", error: "", loading: false };
  if (kind === "unknown") {
    mlfArt.file.error = "この形式は表示できません";
    mlfRenderArtifacts();
    return;
  }
  mlfArt.file.loading = true;
  mlfRenderArtifacts();
  const qs = new URLSearchParams({ run_id: mlfArt.runId, path });
  try {
    const res = await apiFetch(`/api/mlflow/artifact?${qs.toString()}`);
    if (!res.ok) {
      let detail = "";
      try {
        const j = await res.json();
        detail = j && j.detail ? String(j.detail) : "";
      } catch { /* 読めなければ下の既定の文 */ }
      if (!detail) {
        detail = res.status === 413
          ? "大きすぎるので表示しません（5MB まで）"
          : "成果物を読み込めませんでした";
      }
      mlfArt.file.error = detail;
    } else if (kind === "image") {
      mlfArt.file.url = URL.createObjectURL(await res.blob());
    } else {
      mlfArt.file.text = await res.text();
    }
  } catch (e) {
    mlfArt.file.error = (e && e.message) || "成果物を読み込めませんでした";
  } finally {
    mlfArt.file.loading = false;
    mlfRenderArtifacts();
  }
}

/** 成果物の中身を描く。 */
function mlfRenderArtifactBody() {
  const box = saiEl("div", "mlf-art-body");
  const file = mlfArt.file;
  if (!file) return box;
  box.appendChild(saiEl("p", "mlf-art-path", file.path));
  if (file.loading) {
    box.appendChild(saiEl("p", "mlf-note", "読み込んでいます…"));
    return box;
  }
  if (file.error) {
    box.appendChild(saiEl("p", "mlf-note", file.error));
    return box;
  }
  if (file.kind === "image") {
    if (file.url) {
      const img = document.createElement("img");
      img.className = "mlf-img";
      img.alt = "成果物の絵";
      img.src = file.url;
      box.appendChild(img);
    }
    return box;
  }
  if (file.kind === "markdown") {
    const doc = document.createElement("div");
    doc.className = "mlf-md";
    doc.innerHTML = renderMarkdown(file.text);
    box.appendChild(doc);
    return box;
  }
  if (file.kind === "table") {
    const all = String(file.text).split(/\r\n?/);
    const sep = mlfExt(file.path) === "tsv" ? "\t" : ",";
    const rows = all.slice(0, MLF_CSV_ROWS).map((line) => line.split(sep));
    const table = document.createElement("table");
    table.className = "mlf-csv";
    if (rows.length) {
      const thead = document.createElement("thead");
      const tr = document.createElement("tr");
      for (const cell of rows[0]) {
        const th = document.createElement("th");
        th.textContent = cell;
        tr.appendChild(th);
      }
      thead.appendChild(tr);
      table.appendChild(thead);
    }
    const tbody = document.createElement("tbody");
    for (const line of rows.slice(1)) {
      const tr = document.createElement("tr");
      for (const cell of line) {
        const td = document.createElement("td");
        td.textContent = cell;
        tr.appendChild(td);
      }
      tbody.appendChild(tr);
    }
    table.appendChild(tbody);
    box.appendChild(table);
    if (all.length > MLF_CSV_ROWS) {
      box.appendChild(saiEl("p", "mlf-note tiny", `先頭 ${MLF_CSV_ROWS} 行までを表示しています。`));
    }
    return box;
  }
  const pre = document.createElement("pre");
  pre.className = "mlf-pre";
  pre.textContent = file.text;
  box.appendChild(pre);
  return box;
}

/** 成果物を描く。 */
function mlfRenderArtifacts() {
  const box = document.querySelector(".mlf-art");
  if (!box || !mlfArt) return;
  box.innerHTML = "";

  // フォルダの中にいるときだけ、上へ戻る道のしるしを出す
  const parts = mlfArt.path ? mlfArt.path.split("/") : [];
  if (parts.length) {
    const crumbs = saiEl("div", "mlf-crumbs");
    const top = saiEl("button", "mlf-crumb", "↩ いちばん上");
    top.type = "button";
    top.addEventListener("click", () => {
      mlfArt.file = null;
      void mlfLoadArtifacts("");
    });
    crumbs.appendChild(top);
    let acc = "";
    parts.forEach((part, i) => {
      acc = acc ? `${acc}/${part}` : part;
      const target = acc;
      const crumb = saiEl("button", "mlf-crumb", part);
      crumb.type = "button";
      if (i === parts.length - 1) crumb.classList.add("active");
      crumb.addEventListener("click", () => {
        mlfArt.file = null;
        void mlfLoadArtifacts(target);
      });
      crumbs.appendChild(saiEl("span", "mlf-crumb-sep", "/"), crumb);
    });
    box.appendChild(crumbs);
  }

  if (mlfArt.error) {
    box.appendChild(saiEl("p", "mlf-note", mlfArt.error));
  } else if (mlfArt.loading && !mlfArt.loaded) {
    box.appendChild(saiEl("p", "mlf-note", "読み込んでいます…"));
  } else if (!mlfArt.files.length) {
    box.appendChild(saiEl("p", "mlf-note", "成果物はありません。"));
  } else {
    const list = saiEl("div", "mlf-art-list");
    for (const one of mlfArt.files) {
      if (!one) continue;
      const path = String(one.path || "");
      const btn = saiEl("button", "mlf-art-item");
      btn.type = "button";
      btn.append(
        saiEl("span", "mlf-art-icon", one.is_dir ? "📁" : "📄"),
        saiEl("span", "mlf-art-name", path.split("/").pop() || path),
        saiEl("span", "mlf-art-size", one.is_dir ? "" : `${mlfValueText(one.file_size)} バイト`)
      );
      btn.addEventListener("click", () => {
        if (one.is_dir) {
          mlfArt.file = null;
          void mlfLoadArtifacts(path);
        } else {
          void mlfLoadArtifactFile(path);
        }
      });
      list.appendChild(btn);
    }
    box.appendChild(list);
  }
  if (mlfArt.file) box.appendChild(mlfRenderArtifactBody());
}

// ============================================================================
// 画面を開く
// ============================================================================

/**
 * 比較の画面（#/mlflow/compare/<run ID>,…）。
 * 選んだ run を 1 つずつ読み（同時に最大 5 本）、表と重ねたグラフを出す。
 */
async function mlfShowCompare(params) {
  const runIds = (params.runIds || []).filter((id) => MLF_RUN_ID_RE.test(String(id)));
  mlfCompare = { runIds, runs: [], loading: true, error: "", diffOnly: true, showMlflowTags: false, charts: null };
  mlfSyncBar("run の比較");
  if (mlfTable) mlfRenderRuns();
  const box = document.getElementById("mlf-main");
  if (!box) return;
  const body = document.getElementById("mlf-body");
  if (body) {
    body.classList.toggle("mlf-solo", !mlfTable);
    saiSplitShow(body, "main");
  }
  box.innerHTML = "";
  box.appendChild(saiEl("p", "mlf-note", "読み込んでいます…"));
  try {
    mlfCompare.runs = await mlfReadRuns(runIds);
    const missing = mlfCompare.runs.filter((one) => !one).length;
    mlfCompare.runs = mlfCompare.runs.filter(Boolean);
    mlfCompare.error = missing ? `${missing} 本は読み込めませんでした。` : "";
    mlfCompare.loading = false;
    mlfRenderCompare();
  } catch (e) {
    mlfCompare.loading = false;
    mlfCompare.error = (e && e.message) || "run を読み込めませんでした";
    mlfRenderCompare();
    return;
  }
  if (mlfCompare.runs.length >= 2) void mlfLoadCompareCharts();
}

/** run を 1 つずつ読む（同時に最大 5 本）。読めなかった所は null。 */
async function mlfReadRuns(runIds) {
  const out = new Array(runIds.length).fill(null);
  let next = 0;
  const worker = async () => {
    while (next < runIds.length) {
      const i = next;
      next += 1;
      try {
        const data = await saiApi(`/api/mlflow/runs/${encodeURIComponent(runIds[i])}`);
        out[i] = (data && data.run) || null;
      } catch {
        out[i] = null;
      }
    }
  };
  const workers = [];
  for (let i = 0; i < Math.min(MLF_READ_PARALLEL, runIds.length); i += 1) workers.push(worker());
  await Promise.all(workers);
  return out;
}

/** 選んだ run に共通の指標の曲線を、重ねて取る（1 指標 1 回）。 */
async function mlfLoadCompareCharts() {
  const c = mlfCompare;
  if (!c) return;
  const ids = c.runs.map((one) => String(one.info && one.info.run_id)).filter((id) => MLF_RUN_ID_RE.test(id));
  const keys = c.runs.length
    ? mlfMetricKeysOf(c.runs).filter((key) => c.runs.every((one) => mlfMetricKeys(one).includes(key)))
    : [];
  if (ids.length < 2 || keys.length === 0) {
    c.charts = {};
    return;
  }
  c.charts = {};
  for (const key of keys) {
    if (mlfCompare !== c) return; // 画面を離れた
    try {
      const qs = new URLSearchParams();
      for (const id of ids.slice(0, MLF_BULK_RUNS)) qs.append("run_ids", id);
      qs.set("metric_key", key);
      qs.set("max_results", String(MLF_BULK_MAX));
      const data = await saiApi(`/api/mlflow/metrics/bulk?${qs.toString()}`);
      c.charts[key] = { series: mlfSeriesFromMetrics((data && data.metrics) || [], "step"), metrics: (data && data.metrics) || [] };
    } catch (e) {
      c.charts[key] = { series: [], metrics: [], error: (e && e.message) || "指標の時系列を読めませんでした" };
    }
    mlfRenderCompare();
  }
}

/** 比較の画面を描く。 */
function mlfRenderCompare() {
  const c = mlfCompare;
  const box = document.getElementById("mlf-main");
  if (!box || !c) return;
  box.innerHTML = "";
  mlfSyncBar("run の比較");
  if (c.loading) {
    box.appendChild(saiEl("p", "mlf-note", "読み込んでいます…"));
    return;
  }
  if (c.runs.length < MLF_COMPARE_MIN) {
    box.appendChild(
      saiEmpty({
        icon: "⚖",
        title: "比べられる run がありません",
        body: c.error || "2 件以上を選んでください。",
      })
    );
    return;
  }

  const head = saiEl("div", "mlf-detail");
  head.append(
    saiEl("h2", "mlf-detail-name", `${c.runIds.length} 本の run を比べる`),
    saiEl("p", "mlf-detail-sub", "設定・指標・タグの違いと、指標の推移を並べて見ます。")
  );
  box.appendChild(head);

  // 戻る（表へ）与ばすリンク
  const links = saiEl("div", "mlf-links");
  if (mlfTable) {
    const back = saiEl("button", "mlf-link");
    back.type = "button";
    back.append(saiEl("span", "mlf-link-arrow", "‹"), saiEl("span", "", "run の表に戻る"));
    back.addEventListener("click", () => {
      location.hash = mlfHash({ page: "runs", experimentId: mlfTable.experimentId, query: mlfTable.query });
    });
    links.appendChild(back);
  }
  if (c.error) links.appendChild(saiEl("p", "mlf-note", c.error));
  box.appendChild(links);

  // 切り替えの札（「差分だけ」と「MLflow のタグ」）
  const chips = saiEl("div", "mlf-chips");
  const diffBtn = saiEl("button", `mlf-chip small${c.diffOnly ? " active" : ""}`);
  diffBtn.type = "button";
  diffBtn.setAttribute("aria-pressed", c.diffOnly ? "true" : "false");
  diffBtn.textContent = c.diffOnly ? "✓ 差分だけ" : "差分だけを隠す ▾";
  diffBtn.addEventListener("click", () => {
    c.diffOnly = !c.diffOnly;
    mlfRenderCompare();
  });
  const tagBtn = saiEl("button", `mlf-chip small${c.showMlflowTags ? " active" : ""}`);
  tagBtn.type = "button";
  tagBtn.setAttribute("aria-pressed", c.showMlflowTags ? "true" : "false");
  tagBtn.textContent = c.showMlflowTags ? "✓ MLflow のタグ" : "MLflow のタグを隠す";
  tagBtn.addEventListener("click", () => {
    c.showMlflowTags = !c.showMlflowTags;
    mlfRenderCompare();
  });
  chips.append(diffBtn, tagBtn);
  box.appendChild(chips);

  box.appendChild(mlfCompareTable());
  box.appendChild(mlfCompareChartsBox());
}

/** 比較の表（最初の列は行の名前を固定し、横に流せるようにする）。 */
function mlfCompareTable() {
  const c = mlfCompare;
  const wrap = saiEl("div", "mlf-cmp");
  const rows = mlfCompareRows(c.runs, c.diffOnly).filter((row) => {
    if (c.showMlflowTags) return true;
    return !(row.section === "tag" && String(row.key).startsWith("mlflow."));
  });
  if (rows.length === 0) {
    wrap.appendChild(saiEl("p", "mlf-note", "違いはありません（すべての値が同じです）。"));
    return wrap;
  }
  const table = saiEl("table", "mlf-cmp-table");
  const thead = document.createElement("thead");
  const headRow = document.createElement("tr");
  const corner = document.createElement("th");
  corner.textContent = "項目";
  headRow.appendChild(corner);
  for (const run of c.runs) {
    const info = run.info || {};
    const th = document.createElement("th");
    const name = saiEl("span", "mlf-cmp-name", mlfCompareHeadName(run));
    const id = saiEl("span", "mlf-cmp-id", String(info.run_id || "").slice(0, 8));
    th.append(name, id);
    headRow.appendChild(th);
  }
  thead.appendChild(headRow);
  table.appendChild(thead);

  const tbody = document.createElement("tbody");
  let section = "";
  for (const row of rows) {
    if (row.section !== section) {
      section = row.section;
      const tr = document.createElement("tr");
      const td = document.createElement("td");
      td.colSpan = c.runs.length + 1;
      td.className = "mlf-cmp-sec";
      td.textContent = mlfCompareSectionTitle(section);
      tr.appendChild(td);
      tbody.appendChild(tr);
    }
    const tr = document.createElement("tr");
    if (row.same) tr.className = "same";
    const th = document.createElement("th");
    th.className = "mlf-cmp-key";
    th.textContent = row.key;
    tr.appendChild(th);
    for (const value of row.values) {
      const td = document.createElement("td");
      td.textContent = value;
      tr.appendChild(td);
    }
    tbody.appendChild(tr);
  }
  table.appendChild(tbody);
  wrap.appendChild(table);
  return wrap;
}

/** 比較の下に置くグラフ（選んだ run に共通の指標ごとに 1 つ）。 */function mlfCompareChartsBox() {
  const c = mlfCompare;
  const sec = saiEl("div", "mlf-section");
  sec.appendChild(saiEl("h3", "mlf-section-title", "指標のグラフ（重ねる）"));
  const keys = Object.keys(c.charts || {});
  if (keys.length === 0) {
    sec.appendChild(
      saiEl("p", "mlf-note", c.loading ? "読み込んでいます…" : "選んだ run に共通の指標がありません。")
    );
    return sec;
  }
  const grid = saiEl("div", "mlf-chart-grid");
  for (const key of keys) {
    const one = c.charts[key] || {};
    const cell = saiEl("div", "mlf-chart-cell");
    cell.appendChild(saiEl("h4", "mlf-chart-title", key));
    const body = saiEl("div", "mlf-chart-body");
    if (one.error) {
      body.appendChild(saiEl("p", "mlf-note tiny", one.error));
    } else if (!mlfHasPoints(one.series)) {
      body.appendChild(saiEl("p", "mlf-note tiny", "時系列の記録がありません（最後の値だけ）"));
    } else {
      mlfLineChart(body, { series: mlfLabelSeries(one.series, c.runs), xKind: "step", title: key });
    }
    cell.appendChild(body);
    grid.appendChild(cell);
  }
  sec.appendChild(grid);
  return sec;
}

/** 画面を開く（app.js の SaiRoutes から呼ばれる）。 */
async function mlfOpenPage(params) {
  const p = params || mlfParseHash(location.hash) || { page: "experiments" };
  mlfState = p;
  mlfBuild();
  await mlfEnsureProjects(false);
  mlfRenderProjectChip();
  if (p.page === "experiments") {
    mlfTable = null;
    mlfDetail = null;
    mlfArt = null;
    if (mlfExperiments == null) await mlfLoadExperiments();
    mlfRenderExperiments();
  } else if (p.page === "runs") {
    mlfDetail = null;
    mlfArt = null;
    if (p.experimentId === "all" && mlfExperiments == null) await mlfLoadExperiments();
    mlfTable = {
      experimentId: p.experimentId,
      query: p.query || {},
      orderBy: "attributes.start_time DESC",
      runs: [],
      nextPageToken: "",
      loading: false,
      error: "",
      pick: new Set(),
    };
    const main = document.getElementById("mlf-main");
    if (main) main.innerHTML = "";
    mlfSyncBar(p.experimentId === "all" ? "すべての実験" : `実験 ${p.experimentId}`);
    await mlfLoadRuns(true);
  } else if (p.page === "run") {
    await mlfShowRun(p);
  } else if (p.page === "compare") {
    await mlfShowCompare(p);
  }
  mlfWasWide = mlfIsWideNow();
}

/** 「戻る」ボタン。 */
function mlfGoBack() {
  const page = mlfState ? mlfState.page : "experiments";
  if (page === "run" && mlfTable) {
    location.hash = mlfHash({ page: "runs", experimentId: mlfTable.experimentId, query: mlfTable.query });
    return;
  }
  if (page === "experiments") return;
  location.hash = "#/mlflow";
}

/** 幅が変わったとき（広い⇔狭い）。描き直す。 */
function mlfOnResize() {
  if (typeof window === "undefined") return;
  if (mlfWasWide === mlfIsWideNow()) return;
  mlfWasWide = mlfIsWideNow();
  void mlfOpenPage(mlfState);
}

// ============================================================================
// 画面の登録（window.SaiRoutes）
// ============================================================================

if (typeof window !== "undefined") {
  (window.SaiRoutes = window.SaiRoutes || []).push({
    match(hash) {
      return mlfParseHash(hash);
    },
    view: "view-mlflow",
    tab: "mlflow",
    hideTabbar: false,
    interval: 0,
    async open(params) {
      await mlfOpenPage(params);
    },
    async poll() {},
  });
  window.addEventListener("resize", mlfOnResize);
}

// --- node からの読み込み（tests/test_mlflow.js） ---
if (typeof module !== "undefined" && module.exports) {
  module.exports = {
    MLF_QUERY_KEYS,
    MLF_STATUS_TEXT,
    MLF_TAGS,
    MLF_PAGE_SIZE,
    MLF_WIDE_PX,
    mlfParseHash,
    mlfHash,
    mlfParseQuery,
    mlfQueryString,
    mlfFilterFromQuery,
    mlfBuildTree,
    mlfFlatRows,
    mlfOrderOptions,
    mlfIsMetricOrder,
    mlfTag,
    mlfMetricValue,
    mlfMetricList,
    mlfHeadMetric,
    mlfParentId,
    mlfSeedId,
    mlfStatusText,
    mlfDurationText,
    mlfDurationMs,
    mlfTimeText,
    mlfValueText,
    mlfArtifactKind,
    mlfExt,
    mlfProjectChoices,
    // U07（指標の時系列と run の比較）
    mlfMetricKeys,
    mlfMetricKeysOf,
    mlfCompareRows,
    mlfCompareSectionTitle,
    mlfCompareHeadName,
    MLF_COMPARE_MIN,
    MLF_COMPARE_MAX,
    MLF_BULK_RUNS,
  };
}
