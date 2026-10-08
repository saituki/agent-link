// sai-web の Papers タブ（実装計画 07）の本体。
//
// 画面は7つ（Radio は radio.js が持つ・09）:
//   Dashboard     #/papers/dashboard（数字カードと出来事の一覧。Radio の出来事も混ざる）
//   Library       #/papers/library（左に検索の階層・右に検索結果と論文の中身。旧 Discover の検索欄もここ）
//   過去の検索結果  #/papers/log/<name>/<log_id>（08 §4.2・18 §3.5）
//   未DL論文の情報 #/papers/log/<name>/<log_id>/<position>（08 §4.4・18 §3.5）
//   論文情報       #/paper/<name>/<id>（07 §4.4・18 §3.5）
//   本文           #/paper/<name>/<id>/text（07 §4.5・18 §3.5）
// <name> は「論文の置き場があるプロジェクト」の名前。住所に名前がある画面を開いたら
// 「選んでいるプロジェクト」をその名前に切り替え，名前の無い古い住所（ホーム画面の
// 履歴や古い通知から来る）は選んでいるプロジェクトの同じ画面として開く（18 §3.5）。
// Papers の上部でプロジェクトを切り替え，選んだプロジェクトは覚えておく（18 §1.1・§3.6）。
// Radio（#/papers/radio・#/radio/<radio_id>）の中身は radio.js（09）にある。
// 中身はすべて /api/projects/<name>/papers/… 経由（18 §3.4）で，sai-web 自身は
// sai-paper の DB を開かない（00 §2）。
// 通信は app.js の apiFetch（合言葉を自動で付ける），Markdown の表示は
// markdown.js の renderMarkdown，数字や要素の作りは stats.js の関数を使い回す（00 §9）。
// DOM を使わない小さな関数は末尾で node に渡す（tests/test_papers.js）。
"use strict";

// --- 定数 ---

// Research の上部の切り替え（U14・U06 で Papers を統合して 5 つ）。押すと該当の # へ移る。
const RESEARCH_SEG = [
  ["dashboard", "Dashboard", "#/papers/dashboard"],
  ["library", "Library", "#/papers/library"],
  ["tree", "Tree", "#/research/tree"],
  ["experiments", "Experiments", "#/research/experiments"],
  ["radio", "Radio", "#/papers/radio"],
];
// 検索結果の一覧で「未ダウンロードも表示」の選択を覚える（localStorage・"0" が OFF・既定は ON）
const LIB_REMOTE_KEY = "sai_web_library_remote";
// 階層の開閉を覚える（sessionStorage・07 §4.3）
const TREE_FOLDS_KEY = "sai_web_tree_folds";
// 「最後に見ていた一覧」＝論文情報の戻る先（sessionStorage・07 §4.2）
const PAPERS_LIST_KEY = "sai_web_papers_list";
// 読み状態の札（07 §4.3）
const READING_LABEL = {
  unread: "未読",
  queued: "読む予定",
  reading: "読書中",
  read: "既読",
};
// 階層で最初に出す件数（残りは「すべて表示」・07 §4.4-6）
const REFS_INITIAL = 10;
// Discover: 検索先の選択を覚える場所（localStorage・08 §4.1）
const DISCOVER_DBS_KEY = "sai_web_discover_dbs";
// Discover: 最後に見た結果（sessionStorage・08 §4.1）
const DISCOVER_LAST_KEY = "sai_web_discover_last";
const LIB_FIND_KIND_KEY = "sai_web_library_kind";
// 選んでいるプロジェクトを覚える（localStorage・18 §3.6）
const PAPERS_PROJECT_KEY = "sai_web_papers_project";
// Dashboard の出来事の種類 → [表示名, 色の class]（Mac の sai-paper-tui の RECENT ACTIVITY と同じ分け方）
const ACTIVITY_KINDS = {
  discover: ["Discover search", "ak-discover"],
  download: ["Download", "ak-download"],
  import: ["Import", "ak-import"],
  translate: ["Translate", "ak-translate"],
  notes: ["Notes", "ak-notes"],
  radio: ["Radio create", "ak-radio"],
  radio_play: ["Radio play", "ak-radio-play"],
  radio_question: ["Radio question", "ak-radio-question"],
  // U22: Action（run）の行。色は ak-other のまま
  action: ["Action", "ak-other"],
  // U06: research-tree の History（Recent Activity に混ぜる）
  tree: ["Tree", "ak-tree"],
};
// 論文の置き場があるプロジェクトが1つもないときの案内文（18 §1.1）
const NO_PROJECT_MSG =
  "論文の置き場があるプロジェクトがありません（Mac で sai-paper init を実行してください）";
// 広い画面（900px 以上）で Library と Discover の一覧の横に置く「中身」の欄の id
const PAPER_SIDE_IDS = { library: "library-side" };
// 広い画面の「中身」に出しているもの（"" は何も出していない）
let sideKind = "";       // "paper" | "remote" | "log" | "find" | ""
let sidePaperId = null;  // 論文の id（sideKind = "paper"）
let sideRemote = null;   // {logId, name, position}（sideKind = "remote"）
let sideLog = null;
let sideFind = null;      // {logId, name}（sideKind = "find"・U03: Library 内の検索結果）
// Library・Discover の上に出す「ダウンロード中」の帯（作ったものを覚えておく）
let downloadBand = null;
// Download の表示を入れる箱（画面の中に置く・key で探せるように覚えておく）
const jobBoxes = [];

// --- ブラウザの保存場所（プライベートブラウズでも動くように必ず失敗に備える） ---

function libRemote() {
  try {
    return localStorage.getItem(LIB_REMOTE_KEY) !== "0";
  } catch {
    return true;
  }
}

function saveLibRemote(on) {
  try {
    localStorage.setItem(LIB_REMOTE_KEY, on ? "1" : "0");
  } catch { /* 保存できなくても動く */ }
}

/** 階層の開閉をプロジェクトごとに分けて読む（古い形は sai-paper の分として読み替える・18 §3.6）。 */
function loadFolds(projectName) {
  try {
    const raw = sessionStorage.getItem(TREE_FOLDS_KEY);
    if (!raw) return [];
    const map = migrateTreeFolds(JSON.parse(raw));
    const list = map[projectName || ""];
    return Array.isArray(list) ? list : [];
  } catch {
    return [];
  }
}

/** 開閉をプロジェクトごとに分けて覚える（読み替えた形で書き直す・18 §3.6）。 */
function saveFolds(projectName, folds) {
  try {
    const raw = sessionStorage.getItem(TREE_FOLDS_KEY);
    let parsed = null;
    try { parsed = raw ? JSON.parse(raw) : null; } catch { parsed = null; }
    const map = migrateTreeFolds(parsed);
    map[projectName || ""] = Array.isArray(folds) ? folds : [];
    sessionStorage.setItem(TREE_FOLDS_KEY, JSON.stringify(map));
  } catch { /* 保存できなくても動く */ }
}

function savePapersList(hash) {
  try {
    sessionStorage.setItem(PAPERS_LIST_KEY, hash);
  } catch { /* 保存できなくても動く */ }
}

function loadPapersList() {
  try {
    return sessionStorage.getItem(PAPERS_LIST_KEY) || "#/papers/library";
  } catch {
    return "#/papers/library";
  }
}

function saveDiscoverDbs(list) {
  try {
    localStorage.setItem(DISCOVER_DBS_KEY, JSON.stringify(list));
  } catch { /* 保存できなくても動く */ }
}

/** 切り替えの値を正す（"internal"・"paper" のときだけそれ・それ以外は外部）。 */
function libFindKindOf(kind) {
  return kind === "internal" || kind === "paper" ? kind : "external";
}

/** U03: 覚えていた検索の切り替え（"internal"＝内部・"paper"＝Paper 一覧・それ以外は外部）。 */
function loadLibFindKind() {
  try {
    return libFindKindOf(localStorage.getItem(LIB_FIND_KIND_KEY));
  } catch {
    return "external";
  }
}

function saveLibFindKind(kind) {
  try {
    localStorage.setItem(LIB_FIND_KIND_KEY, libFindKindOf(kind));
  } catch { /* 保存できなくても動く */ }
}

/** 覚えていた検索先（無ければ null＝既定に戻す・08 §4.1）。 */
function loadDiscoverDbs() {
  try {
    const raw = localStorage.getItem(DISCOVER_DBS_KEY);
    const arr = raw ? JSON.parse(raw) : null;
    return Array.isArray(arr) ? arr.filter((x) => typeof x === "string") : null;
  } catch {
    return null;
  }
}

/** 最後に見た検索をプロジェクトごとに分けて覚える（古い形は sai-paper の分として読み替える・18 §3.6）。 */
function saveDiscoverLast(projectName, id) {
  try {
    const raw = sessionStorage.getItem(DISCOVER_LAST_KEY);
    let parsed = null;
    try { parsed = raw != null ? JSON.parse(raw) : null; } catch { parsed = null; }
    const map = migrateDiscoverLast(parsed);
    if (id == null) delete map[projectName || ""];
    else map[projectName || ""] = id;
    sessionStorage.setItem(DISCOVER_LAST_KEY, JSON.stringify(map));
  } catch { /* 保存できなくても動く */ }
}

/** 最後に見た Discover の結果（そのプロジェクトの分・無ければ null・08 §4.1・18 §3.6）。 */
function loadDiscoverLast(projectName) {
  try {
    const raw = sessionStorage.getItem(DISCOVER_LAST_KEY);
    if (raw == null) return null;
    let parsed = null;
    try { parsed = JSON.parse(raw); } catch { parsed = null; }
    const map = migrateDiscoverLast(parsed);
    const v = map[projectName || ""];
    const n = typeof v === "number" ? v : parseInt(v, 10);
    return Number.isFinite(n) ? n : null;
  } catch {
    return null;
  }
}

// --- DOM を使わない小さな関数（node のテストからも使う・07 §4.7・18 §3.5・§3.6） ---

/** Papers の API の住所（プロジェクト名を入れる・18 §3.4）。 */
function papersApiBase(projectName) {
  return `/api/projects/${encodeURIComponent(projectName)}/papers`;
}

/** 論文情報・本文の住所を書く（name が無ければ古い形＝選んでいるプロジェクトで開く・18 §3.5）。 */
function paperHash(name, id, text) {
  const base = name
    ? `#/paper/${encodeURIComponent(name)}/${encodeURIComponent(id)}`
    : `#/paper/${encodeURIComponent(id)}`;
  return text ? `${base}/text` : base;
}

/**
 * 論文情報・本文の住所を読む（新旧どちらも・18 §3.5）。
 * 見つからなければ null。name が null のときは古い住所（選んでいるプロジェクトで開く）。
 * 判定: 1段は数字だけ（旧）／2段の2つ目が "text" は旧本文／2段の2つ目が数字は新（<name>/<id>）／
 * 3段は新本文。数字2つの2段（例 12/3）は旧 <log_id> 側ではなく /paper では存在しない形なので
 * <name>/<id> として扱う必要はなく，/paper には旧形・新形とも上記の規則で一意に分かれる。
 */
function parsePaperHash(hash) {
  const m = /^#\/paper\/([^/?#]+)(?:\/([^/?#]+))?(?:\/([^/?#]+))?$/.exec(String(hash || ""));
  if (!m) return null;
  const [, s1, s2, s3] = m;
  if (s2 === undefined) {
    // 1段（旧 論文情報）は数字のみ
    return /^\d+$/.test(s1) ? { name: null, id: s1, text: false } : null;
  }
  if (s3 === undefined) {
    if (s2 === "text") {
      // 2段で2つ目が text → 旧 本文（#/paper/<id>/text）
      return /^\d+$/.test(s1) ? { name: null, id: s1, text: true } : null;
    }
    // 2段で2つ目が数字 → 新 論文情報（#/paper/<name>/<id>）
    return /^\d+$/.test(s2) ? { name: s1, id: s2, text: false } : null;
  }
  // 3段 → 新 本文（#/paper/<name>/<id>/text）
  if (s3 !== "text" || !/^\d+$/.test(s2)) return null;
  return { name: s1, id: s2, text: true };
}

/** 過去の検索結果の住所を書く（name が無ければ古い形・18 §3.5）。 */
function logHash(name, id, position) {
  const base = name
    ? `#/papers/log/${encodeURIComponent(name)}/${encodeURIComponent(id)}`
    : `#/papers/log/${encodeURIComponent(id)}`;
  return position != null && position !== "" ? `${base}/${encodeURIComponent(position)}` : base;
}

/**
 * 過去の検索結果の住所を読む（新旧どちらも・18 §3.5）。見つからなければ null。
 * 判定: 1段は数字のみ（旧）／2段で1つ目が数字は旧 <log_id>/<position>，
 * 数字でなければ新 <name>/<log_id>／3段は新 <name>/<log_id>/<position>。
 */
function parseLogHash(hash) {
  const m = /^#\/papers\/log\/([^/?#]+)(?:\/([^/?#]+))?(?:\/([^/?#]+))?$/.exec(String(hash || ""));
  if (!m) return null;
  const [, s1, s2, s3] = m;
  if (s2 === undefined) {
    return /^\d+$/.test(s1) ? { name: null, id: s1, position: null } : null;
  }
  if (s3 === undefined) {
    if (/^\d+$/.test(s1)) {
      // 2段で1つ目が数字 → 旧 <log_id>/<position>
      return /^\d+$/.test(s2) ? { name: null, id: s1, position: s2 } : null;
    }
    // 2段で1つ目が数字でない → 新 <name>/<log_id>
    return /^\d+$/.test(s2) ? { name: s1, id: s2, position: null } : null;
  }
  // 3段 → 新 <name>/<log_id>/<position>（旧に3段の形は無い）
  if (!/^\d+$/.test(s2) || !/^\d+$/.test(s3)) return null;
  return { name: s1, id: s2, position: s3 };
}

/**
 * 階層の開閉の記憶を新しい形（{<name>: [...]}）に読み替える（一度だけ・18 §3.6）。
 * 古い形（開閉キーの配列）は sai-paper プロジェクトの分として読む。
 */
function migrateTreeFolds(value) {
  if (Array.isArray(value)) {
    return { "sai-paper": value.filter((x) => typeof x === "string") };
  }
  if (value && typeof value === "object") return value;
  return {};
}

/**
 * Discover の最後の検索の記憶を新しい形（{<name>: log_id}）に読み替える（一度だけ・18 §3.6）。
 * 古い形（数字そのもの／数字の文字列）は sai-paper プロジェクトの分として読む。
 */
function migrateDiscoverLast(value) {
  if (value == null || Array.isArray(value)) return {};
  if (typeof value === "object") return value;
  const n = typeof value === "number" ? value : parseInt(value, 10);
  return Number.isFinite(n) ? { "sai-paper": n } : {};
}

// --- U03: Library 内の検索（内部 DB）の DOM を使わない関数（node のテストからも使う） ---

/** 内部検索の結果の住所を書く（#/papers/find/<name>/<log_id>）。 */
function findHash(name, id) {
  return `#/papers/find/${encodeURIComponent(name)}/${encodeURIComponent(id)}`;
}

/** 内部検索の結果の住所を読む。見つからなければ null。 */
function parseFindHash(hash) {
  const m = /^#\/papers\/find\/([^/?#]+)\/(\d+)$/.exec(String(hash || ""));
  if (!m) return null;
  return { name: m[1], id: m[2] };
}

/** 左の階層を出したまま内部検索の結果を右に開く住所。 */
function libraryFindHash(id) {
  return `#/papers/library/find/${encodeURIComponent(id)}`;
}

/** 内部検索の条件の要約（例: hybrid · all · 10 件 · Q12）。指定が無いものは出さない。 */
function lfindSummary(state) {
  const parts = [];
  const modeLabel = { word: "単語", vector: "意味", hybrid: "両方" }[(state && state.mode) || ""];
  if (modeLabel) parts.push(modeLabel);
  const scopeLabel = { papers: "論文", notes: "ノート", figures: "図", all: "全部" }[(state && state.scope) || ""];
  if (scopeLabel) parts.push(scopeLabel);
  if (state && state.count != null && state.count !== "") parts.push(`${state.count} 件`);
  if (state && state.node) parts.push(String(state.node));
  return parts.join(" · ");
}

/**
 * 内部検索の送る中身を作る。{ok, body, error} を返す。
 * state は {query, mode, scope, count, papers, node, subtree, each}。
 * 不正な組み合わせは ok: false と日本語の理由を返し、そのとき API は呼ばない。
 */
function lfindBody(state) {
  const s = state || {};
  const query = String(s.query == null ? "" : s.query).trim();
  if (!query) return { ok: false, body: null, error: "検索する言葉を入れてください" };
  if (query.length > 200) return { ok: false, body: null, error: "検索する言葉は 200 字までです" };
  const mode = String(s.mode || "");
  if (mode !== "" && mode !== "word" && mode !== "vector" && mode !== "hybrid") {
    return { ok: false, body: null, error: "検索の方式が正しくありません" };
  }
  const scope = String(s.scope || "");
  if (scope !== "" && scope !== "papers" && scope !== "notes" && scope !== "figures" && scope !== "all") {
    return { ok: false, body: null, error: "検索の対象が正しくありません" };
  }
  const count = Number(s.count);
  if (!Number.isFinite(count) || count < 1 || count > 100) {
    return { ok: false, body: null, error: "件数は 1〜100 の数字にしてください" };
  }
  const papers = Array.isArray(s.papers)
    ? s.papers.map((x) => Number(x)).filter((x) => Number.isFinite(x))
    : [];
  if (papers.length > 20) return { ok: false, body: null, error: "論文は 20 本までしか選べません" };
  const node = String(s.node || "").trim();
  if (node !== "" && !/^[QOFE][1-9][0-9]{0,8}$/.test(node)) {
    return { ok: false, body: null, error: "Node の番号が正しくありません（Q12・O5 の形）" };
  }
  const body = { query, top_k: count };
  if (mode) body.mode = mode;
  if (scope) body.scope = scope;
  if (papers.length > 0) body.papers = papers;
  if (node) body.node = node;
  if (node && s.subtree) body.subtree = true;
  const hasEachTarget = node !== "" || papers.length > 0;
  const eachRaw = s.each == null ? "" : String(s.each).trim();
  if (eachRaw !== "") {
    if (!hasEachTarget) return { ok: false, body: null, error: "論文ごとの上限は Node か論文を選んだときだけ使えます" };
    const each = Number(eachRaw);
    if (!Number.isInteger(each) || each < 1 || each > 20) {
      return { ok: false, body: null, error: "論文ごとの上限は 1〜20 の数字にしてください" };
    }
    body.each = each;
  }
  return { ok: true, body, error: "" };
}

/** 著者は最初の 1 人＋「ほか」（1 人ならそのまま，無ければ空）。 */
function authorShort(authors) {
  const s = String(authors || "").trim();
  if (!s) return "";
  const parts = s.split(",").map((x) => x.trim()).filter(Boolean);
  if (parts.length === 0) return "";
  if (parts.length === 1) return parts[0];
  return `${parts[0]} ほか`;
}

/** 絞り込みの言葉（空白区切り・大文字小文字は無視）。 */
function queryWords(query) {
  return String(query || "")
    .toLowerCase()
    .split(/\s+/)
    .filter(Boolean);
}

/** text にすべての言葉が含まれるか（大文字小文字は無視）。 */
function includesAll(text, words) {
  if (words.length === 0) return true;
  const hay = String(text || "").toLowerCase();
  return words.every((w) => hay.includes(w));
}

/** 題名・著者にすべての言葉が含まれる論文だけ残す（画面の中だけで絞る・07 §4.3）。 */
function filterPapers(papers, query) {
  const words = queryWords(query);
  const list = Array.isArray(papers) ? papers : [];
  if (words.length === 0) return list.slice();
  return list.filter((p) =>
    includesAll(`${(p && p.title) || ""} ${(p && p.authors) || ""}`, words)
  );
}

// --- 画面の状態（Library） ---

let libPapers = null;   // null は読み込み中
let libGroups = null;   // 階層（tree を読んだときだけ）
let libFindGroups = null; // U03: 内部検索の履歴（tree の find_groups）
let lfindLoadingNode = false; // U03: Node の一覧の読み込み中
let libFilter = "";     // 絞り込みの言葉（画面の中だけ）
let libError = "";      // 失敗したときの文（表示が空のときだけ出す）
// U03: Library 内の検索（内部 DB）の画面の状態。切り替えだけ localStorage に覚える。
let libFindKind = loadLibFindKind();
let libFindMode = "";
let libFindScope = "";
let libFindCount = 10;
let libFindPapers = [];
let libFindNode = "";
let libFindSubtree = false;
let libFindEach = "";
let libFindNodes = null;
let libFindNodesMsg = "";
const findLogCache = new Map();
let findView = null;     // U03: 開いている内部検索の結果 {id, name, data, error}

// --- 画面の状態（論文情報） ---

let paperIdNum = null;  // 開いている論文の id
let paperData = null;   // GET /api/projects/<name>/papers/<id> の応答
let paperError = "";    // 失敗の detail
let abstractLang = "";  // 要旨の言語（"ja" | "en"）
let abstractMsg = "";   // 訳の作成に失敗したときの detail
let translating = false;// 訳の作成中
let refsAll = false;    // 引用文献を全部出しているか
let refsOpen = new Set(); // 引用文献を開いた行（index）・その場で開く用
let paperChunks = null;   // チャンクの一覧 {key, state, items, error, open}（null＝開いていない）
let noteEditing = false;  // ノートを書き換えているか
let noteDraft = "";       // 書きかけのノート（描き直しても消さない）
let noteSaving = false;   // 保存中
let noteMsg = "";         // 保存に失敗したときの detail
let paperTarget = null;   // 論文情報をどこへ描くか（null は論文情報の画面・U6 の広い画面用）

// --- 画面の状態（Dashboard） ---

let dashboardData = null;  // GET …/papers/dashboard の応答（null＝まだ読んでいない）
let dashboardError = "";   // 読めなかったときの detail

// --- 画面の状態（Project＝要対応・現況・考え・U6） ---

let projectData = null;    // GET …/papers/project の応答（null＝まだ読んでいない）
let projectError = "";     // 読めなかったときの detail
let mindEditing = false;   // Mind を編集しているか（Edit を押した後）
let mindDraft = "";        // 編集中の Mind の全文（描き直しても残す）
let mindSaving = false;    // Mind を保存している途中
let mindMsg = "";          // 保存に失敗したときの理由
let sciStatus = null;      // GET …/papers/sci/status の応答（Next Action・History の元。null＝まだ読んでいない）
let sciError = "";         // 読めなかったときの detail

// --- 画面の状態（Discover / 過去の検索結果・08 §4） ---
let discoverProviders = null;   // {default, available}（null＝まだ読んでいない）
let discoverDbs = null;         // 選んでいる検索先（null＝読み込み前）
let discoverYears = { from: "", to: "" }; // 年の範囲（空欄は送らない・08 §4.1）
let discoverLimit = 20;         // 件数（10 / 20 / 50・既定 20）
let discoverCondOpen = false;   // 条件を開いているか
let discoverQuery = "";         // 検索欄の今の文字（画面を移っても消さない）
let discoverMode = "idle";      // "idle" | "loading" | "error"（結果は右の欄・住所に出す）
let discoverError = "";         // 失敗の detail（mode = "error" に出す）
// 同じ log の結果は画面を移っても取り直さない（08 §4.4）
const logCache = new Map();
let logView = null;    // 開いている過去の検索 {id, name, data, error}
let remoteView = null; // 開いている未ダウンロード論文 {logId, name, position, data, error}

// --- プロジェクトの切り替え（Papers 上部の名前・計画 18 §1.1・§3.5・§3.6） ---

let paperProjects = null;     // 論文の置き場があるプロジェクト一覧（null＝まだ読んでいない）
let paperProjectsError = "";  // 一覧を読めなかったときの detail
let selectedProject = null;   // 今選んでいるプロジェクト名（null＝無し）
let paperProject = null;      // 開いている論文のプロジェクト名（論文情報・本文で使う・18 §3.5）

function loadStoredProject() {
  try { return localStorage.getItem(PAPERS_PROJECT_KEY) || null; }
  catch { return null; } // プライベートブラウズでも動くように
}

function saveStoredProject(name) {
  try {
    if (name) localStorage.setItem(PAPERS_PROJECT_KEY, name);
    else localStorage.removeItem(PAPERS_PROJECT_KEY);
  } catch { /* 保存できなくても動く */ }
}

/** 画面ごとの記憶を捨てる（選んだプロジェクトが変わったので中身を作り直す・18 §3.6）。 */
function resetProjectScreens() {
  dashboardData = null;
  dashboardError = "";
  projectData = null;
  projectError = "";
  mindEditing = false;
  mindDraft = "";
  mindMsg = "";
  sciStatus = null;
  sciError = "";
  sideKind = "";
  sidePaperId = null;
  sideRemote = null;
  sideLog = null;
  sideFind = null;
  jobBoxes.length = 0;
  libPapers = null;
  libGroups = null;
  libFindGroups = null;
  libError = "";
  discoverProviders = null;
  discoverError = "";
  discoverMode = "idle";
  logCache.clear();
  logView = null;
  remoteView = null;
  findLogCache.clear();
  findView = null;
  // 論文情報の音声の読み直しも止める（radio.js が作る関数・読み込み順に関係しない）
  try { if (typeof stopRadioSlotPoll === "function") stopRadioSlotPoll(); } catch { /* 無しでも動く */ }
  savePapersList("#/papers/library"); // 戻る先も新しいプロジェクトのものにする
}

/** 選んでいるプロジェクトを変える（覚えて，画面ごとの記憶を捨てる・18 §1.1）。 */
function setSelectedProject(name) {
  const next = name || null;
  saveStoredProject(next);
  if (next === selectedProject) return;
  selectedProject = next;
  resetProjectScreens();
}

/** 今選んでいるプロジェクト名（無ければ null・radio.js からも使う・18 §1.2）。 */
function selectedPaperProject() { return selectedProject; }

/** 論文の置き場があるプロジェクト一覧（まだなら空の配列・radio.js からも使う・18 §1.1）。 */
function getPaperProjects() { return Array.isArray(paperProjects) ? paperProjects : []; }

/** プロジェクト名から一覧の項目を引く（root_name が分かる・radio.js の名札で使う・18 §1.2）。 */
function paperProjectByName(name) {
  return getPaperProjects().find((p) => p && p.name === name) || null;
}

/** 開いている論文のプロジェクト名（無ければ null・radio.js の音声の置き場で使う・18 §3.5）。 */
function currentPaperProject() { return paperProject; }

/**
 * 論文の置き場があるプロジェクトの一覧を読む（18 §1.1）。
 * 住所が指定した画面以外ではここの結果だけを使い，選んでいるプロジェクトを決める
 * （覚えた名前が一覧に無ければ先頭・§1.1）。読めなかったときは detail を覚えておく。
 */
async function ensurePaperProjects(force) {
  if (force || paperProjects === null) {
    try {
      const res = await apiFetch("/api/paper-projects");
      const body = await res.json().catch(() => ({}));
      if (!res.ok) throw new Error(body.detail || "プロジェクト一覧を読み込めませんでした");
      paperProjects = Array.isArray(body) ? body : [];
      paperProjectsError = "";
      const stored = loadStoredProject();
      const next = stored && paperProjects.some((p) => p && p.name === stored)
        ? stored
        : (paperProjects[0] && paperProjects[0].name) || null;
      setSelectedProject(next); // 異なるときだけ画面の記憶を捨てる
      window.SaiShell.ok();
    } catch (e) {
      paperProjectsError = (e && e.message) || "プロジェクト一覧を読み込めませんでした";
      window.SaiShell.fail();
    }
  }
  // 住所の画面から戻ってきたときも chip が古くならないよう必ず揃える（18 §1.1）
  updateProjectChips();
  return paperProjects;
}

/**
 * 住所のプロジェクト名，なければ選んでいるプロジェクトを返す（18 §3.5）。
 * 住所に名前がある画面を開いたら「選んでいるプロジェクト」もその名前に切り替える。
 */
async function projectForAddress(nameParam) {
  if (nameParam) {
    setSelectedProject(nameParam);
    return nameParam;
  }
  await ensurePaperProjects();
  return selectedProject;
}

/** 上部のプロジェクト名（📁 … ▾）を今の選択に合わせる（18 §1.1）。 */
function updateProjectChips() {
  for (const id of ["dashboard-project", "library-project", "discover-project", "radio-project", "sci-project", "mlf-project"]) {
    const chip = document.getElementById(id);
    if (!chip) continue;
    chip.hidden = paperProjects === null; // 読んでから出す（18 §1.1）
    chip.textContent = `📁 ${selectedProject || "—"}`;
  }
}

let sheetBox = null; // 切り替えの一覧（作ったことがあれば使い回す）

function closeProjectSheet() {
  if (!sheetBox) return;
  sheetBox.remove();
  sheetBox = null;
}

/**
 * 切り替えの一覧を下から出す（18 §1.1）。
 * 出す前に一覧を取り直す（並びは最後に動いた順のため）。置き場が1つも無ければ案内文を出す。
 */
async function openProjectSheet() {
  closeProjectSheet();
  await ensurePaperProjects(true);
  const wrap = document.createElement("div");
  wrap.className = "papers-sheet-wrap";
  wrap.addEventListener("click", (e) => {
    if (e.target === wrap) closeProjectSheet(); // 背景を押すと閉じる
  });
  const panel = document.createElement("div");
  panel.className = "papers-sheet";
  panel.appendChild(el("div", "papers-sheet-title", "プロジェクトを選ぶ"));
  const list = getPaperProjects();
  if (list.length === 0) {
    panel.appendChild(el("p", "papers-sheet-note", paperProjectsError || NO_PROJECT_MSG));
  } else {
    for (const p of list) {
      const item = document.createElement("button");
      item.className = "papers-sheet-item" + (p.name === selectedProject ? " active" : "");
      item.appendChild(el("span", "papers-sheet-check", p.name === selectedProject ? "✓" : ""));
      item.appendChild(el("span", null, p.name));
      item.addEventListener("click", () => {
        closeProjectSheet();
        setSelectedProject(p.name); // 覚えて，画面ごとの記憶を捨てる（18 §1.1・§3.6）
        reopenCurrentScreen();
      });
      panel.appendChild(item);
    }
    if (paperProjectsError) panel.appendChild(el("p", "papers-sheet-note", paperProjectsError));
  }
  wrap.appendChild(panel);
  document.body.appendChild(wrap);
  sheetBox = wrap;
}

/** 切り替えたら今の画面を中身ごと作り直す（chip は Papers の 4 画面にある・18 §1.1）。 */
function reopenCurrentScreen() {
  // research-tree（Node つきでも同じ画面を作り直す）
  if (typeof stParseHash === "function" && stParseHash(location.hash) && typeof stOpen === "function") void stOpen(stParseHash(location.hash));
  else if (location.hash.startsWith("#/research/experiments") && typeof mlfOpenPage === "function") void mlfOpenPage(mlfParseHash(location.hash)); // mlflow.js
  else if (location.hash === "#/papers/dashboard") void openPapersDashboard();
  else if (location.hash === "#/papers/radio") void openRadioList(); // radio.js
  else void openLibrary();
}

// --- Dashboard（#/papers/dashboard） ---

/** 出来事の時刻（ISO）を "9/24 10:05" の形に（今年でなければ年も付ける）。 */
function activityTime(iso) {
  const d = new Date(iso);
  if (isNaN(d)) return "";
  const hm = `${d.getHours()}:${String(d.getMinutes()).padStart(2, "0")}`;
  const md = `${d.getMonth() + 1}/${d.getDate()}`;
  return d.getFullYear() === new Date().getFullYear() ? `${md} ${hm}` : `${d.getFullYear()}/${md} ${hm}`;
}

/** バイト数を "1.23 GB" の形に（Mac の Dashboard と同じく 2 桁）。 */
function bytesLabel(n) {
  const v = Number(n) || 0;
  if (v >= 1024 ** 3) return `${(v / 1024 ** 3).toFixed(2)} GB`;
  if (v >= 1024 ** 2) return `${(v / 1024 ** 2).toFixed(1)} MB`;
  return `${Math.round(v / 1024)} KB`;
}

/**
 * 出来事の行を押したときの飛び先（DOM を使わない・tests/test_papers.js）。
 * Radio の再生・質問は再生画面，論文のある出来事は論文情報，research-tree の History は
 * run の一覧（run が無ければ Node の画面），それ以外は null（押せない）。
 */
function activityHash(item, projectName) {
  if (!item) return null;
  const radioKinds = item.kind === "radio_play" || item.kind === "radio_question";
  if (radioKinds && item.radio_id) return `#/radio/${encodeURIComponent(item.radio_id)}`;
  if (item.paper_id != null && projectName) return paperHash(projectName, item.paper_id, false);
  if (item.kind === "tree") {
    if (item.run_id != null && item.run_id !== "" && projectName && typeof stRunListHash === "function") {
      return stRunListHash(projectName, item.run_id);
    }
    if (item.node) return `#/research/tree/${encodeURIComponent(item.node)}`;
  }
  return null;
}

/**
 * Recent Activity の並び（DOM を使わない・tests/test_papers.js）。
 * 論文の動き（activity）と research-tree の History（history）を 1 つにして、新しい順に並べる。
 * History の 1 件は kind "tree" の行にする。文は textOf（History の 1 件 → 文）で作り、
 * 日付は時刻の欄に出すので文には入れない。
 */
function pdMergeActivity(activity, history, textOf) {
  const papers = Array.isArray(activity) ? activity : [];
  const tree = (Array.isArray(history) ? history : []).map((h) => {
    const o = h || {};
    return {
      kind: "tree",
      label: `${o.node ? `${o.node} ` : ""}${textOf(o)}`,
      occurred_at: o.at,
      run_id: o.run_id,
      node: o.node,
    };
  });
  const when = (x) => {
    const t = Date.parse(x && x.occurred_at);
    return Number.isNaN(t) ? -Infinity : t;
  };
  return [...papers, ...tree].sort((x, y) => {
    const a = when(x);
    const b = when(y);
    return a === b ? 0 : a > b ? -1 : 1;
  });
}

async function openPapersDashboard() {
  savePapersList("#/papers/dashboard"); // 論文情報の戻る先（07 §4.2）
  fillSegment("dashboard-seg", "dashboard");
  await ensurePaperProjects();
  if (location.hash !== "#/papers/dashboard") return;
  const name = selectedProject;
  // Mind の書きかけは別のプロジェクトへ保存しないよう、画面を開くたびに捨てる
  mindEditing = false;
  mindDraft = "";
  mindMsg = "";
  if (!name) {
    dashboardData = null;
    dashboardError = "";
    renderPapersDashboard();
    return;
  }
  if (dashboardData === null) renderPapersDashboard(); // 「読み込み中…」を先に出す
  // Project（State・Mind）と research-tree（Next Action・History）は別々に読む。開くたびに読み直す
  void loadDashboardProject(name, true);
  void loadDashboardSci(name);
  try {
    const res = await apiFetch(`${papersApiBase(name)}/dashboard`);
    const body = await res.json().catch(() => ({}));
    if (!res.ok) throw new Error(body.detail || "読み込めませんでした");
    if (location.hash !== "#/papers/dashboard" || selectedProject !== name) return;
    dashboardData = body;
    dashboardError = "";
    window.SaiShell.ok();
  } catch (e) {
    if (location.hash !== "#/papers/dashboard" || selectedProject !== name) return;
    dashboardError = (e && e.message) || "読み込めませんでした";
    window.SaiShell.fail();
  }
  renderPapersDashboard();
}

/**
 * Dashboard の中身（上から 数字のカード → State・Mind → Next Action → Recent Activity）。
 * 数字のカード以外は折りたためる（<details open>）。
 */
function renderPapersDashboard() {
  const box = document.getElementById("papers-dashboard-body");
  if (!box) return;
  box.innerHTML = "";
  if (!selectedProject) {
    box.appendChild(el("p", "lib-empty", paperProjectsError || NO_PROJECT_MSG));
    return;
  }
  // 1. 数字カード（Mac の Dashboard の 5 つ・折りたたまない）
  if (dashboardData) box.appendChild(numberCard(dashboardData.cards || {}));
  else if (!dashboardError) box.appendChild(el("p", "lib-empty", "読み込み中…"));
  // 2. State・Mind（Project）。読みそこなっても下は出す
  if (projectData !== null || projectError) box.appendChild(projectCards());
  // 3. Next Action（research-tree の先頭 3 件）
  box.appendChild(nextActionCard());
  // 4. Recent Activity（論文の動きと research-tree の History）
  box.appendChild(recentActivityCard());
}

/** 数字のカード（Papers・Tags・Notes・PDF・DB）。 */
function numberCard(c) {
  const card = el("section", "dash-card");
  const grid = el("div", "dash-grid pd-grid");
  for (const [value, label] of [
    [String(Number(c.papers) || 0), "Papers"],
    [String(Number(c.tags) || 0), "Tags"],
    [String(Number(c.notes) || 0), "Notes"],
    [bytesLabel(c.pdf_bytes), "PDF"],
    [bytesLabel(c.db_bytes), "DB"],
  ]) {
    const cell = el("div", "dash-cell");
    cell.appendChild(el("div", "dash-big", value));
    cell.appendChild(el("div", "dash-big-label", label));
    grid.appendChild(cell);
  }
  card.appendChild(grid);
  return card;
}

/** 折りたためるカード（<details open>・見出しは summary・中身は返した要素に足す）。 */
function foldCard(title, term, termLabel) {
  const card = document.createElement("details");
  card.className = "dash-card pd-fold";
  card.open = true;
  const head = document.createElement("summary");
  head.className = "dash-card-head";
  head.appendChild(el("span", "dash-card-title", title));
  if (term) {
    const badge = paperTerm(term, termLabel);
    if (badge) head.appendChild(badge);
  }
  card.appendChild(head);
  return card;
}

// --- Project（State・Mind・U6・sai-paper の phone project） ---

/** Project を読む（GET …/papers/project）。読みそこなっても数字カードと出来事は出す。 */
async function loadDashboardProject(name, force) {
  if (!name) return;
  if (projectData !== null && !force) return;
  try {
    const res = await apiFetch(`${papersApiBase(name)}/project`);
    const body = await res.json().catch(() => ({}));
    if (!res.ok) throw new Error(body.detail || "読み込めませんでした");
    if (selectedProject !== name || location.hash !== "#/papers/dashboard") return;
    projectData = body && typeof body === "object" ? body : {};
    projectError = "";
  } catch (e) {
    if (selectedProject !== name || location.hash !== "#/papers/dashboard") return;
    projectData = null;
    projectError = (e && e.message) || "読み込めませんでした";
  }
  renderPapersDashboard();
}

/** research-tree の状態を読む（GET …/papers/sci/status・Next Action と History）。読めなくても他は出す。 */
async function loadDashboardSci(name) {
  if (!name) return;
  try {
    const res = await apiFetch(`${papersApiBase(name)}/sci/status`);
    const body = await res.json().catch(() => ({}));
    if (!res.ok) throw new Error(body.detail || "読み込めませんでした");
    if (selectedProject !== name || location.hash !== "#/papers/dashboard") return;
    sciStatus = body && typeof body === "object" ? body : {};
    sciError = "";
  } catch (e) {
    if (selectedProject !== name || location.hash !== "#/papers/dashboard") return;
    sciStatus = null;
    sciError = (e && e.message) || "読み込めませんでした";
  }
  renderPapersDashboard();
}

/** State と Mind（読み込み中は「読み込み中…」）。 */
function projectCards() {
  const wrap = el("div", "pd-project");
  if (projectData === null) {
    wrap.appendChild(el("p", "lib-empty", projectError || "Loading…"));
    return wrap;
  }
  wrap.appendChild(stateCard());
  wrap.appendChild(mindCard());
  return wrap;
}

/** 現況（state.md）。読むだけ（sai-paper の決まりでスマホからは書き換えない）。 */
function stateCard() {
  const card = foldCard("State", "state", "State");
  card.appendChild(projectMd(projectData.state, "No state yet. It is written as the agent advances the research"));
  return card;
}

/** 考え（mind.md）。Edit で全文を書き換えて保存できる。 */
function mindCard() {
  const card = foldCard("Mind", "mind", "Mind");
  if (!mindEditing) {
    const edit = document.createElement("button");
    edit.type = "button";
    edit.className = "sai-btn small pd-mind-edit";
    edit.textContent = "Edit";
    edit.addEventListener("click", (e) => {
      // 見出しの中のボタンなので、押したときに折りたたみが変わらないようにする
      e.preventDefault();
      startMindEdit();
    });
    card.querySelector("summary").appendChild(edit);
    card.appendChild(projectMd(projectData.mind, "No mind yet. Press Edit to write one"));
  } else {
    card.appendChild(mindEditor());
  }
  return card;
}

/** Mind の編集（今の全文を入れて、編集の欄を出す）。 */
function startMindEdit() {
  mindDraft = projectData.mind == null ? "" : String(projectData.mind);
  mindMsg = "";
  mindEditing = true;
  renderPapersDashboard();
}

/** Mind の編集欄（全文・保存・やめる）。 */
function mindEditor() {
  const wrap = el("div", "pd-mind-edit-box");
  const text = document.createElement("textarea");
  text.className = "sai-input pd-mind-text";
  text.rows = 12;
  text.value = mindDraft;
  text.addEventListener("input", () => { mindDraft = text.value; });
  const row = el("div", "pd-mind-row");
  const save = document.createElement("button");
  save.type = "button";
  save.className = "sai-btn primary small";
  save.textContent = mindSaving ? "保存しています…" : "保存";
  save.disabled = mindSaving;
  save.addEventListener("click", () => { void saveMind(); });
  const cancel = document.createElement("button");
  cancel.type = "button";
  cancel.className = "sai-btn small";
  cancel.textContent = "やめる";
  cancel.disabled = mindSaving;
  cancel.addEventListener("click", () => { void cancelMindEdit(); });
  row.appendChild(save);
  row.appendChild(cancel);
  wrap.appendChild(text);
  wrap.appendChild(row);
  if (mindMsg) wrap.appendChild(el("p", "pim-error", mindMsg));
  return wrap;
}

/** Mind を保存する（PUT …/project/mind・本文は {"text"}）。成功したら読み直して表示に戻す。 */
async function saveMind() {
  const name = selectedProject;
  if (!name || mindSaving) return;
  mindSaving = true;
  mindMsg = "";
  renderPapersDashboard();
  try {
    await paperApi(`${papersApiBase(name)}/project/mind`, { method: "PUT", body: { text: mindDraft } });
    mindEditing = false;
    mindDraft = "";
    if (typeof saiToast === "function") saiToast("Mind を保存しました", "ok");
    mindSaving = false;
    await loadDashboardProject(name, true);
  } catch (e) {
    mindMsg = (e && e.message) || "保存できませんでした";
    if (typeof saiToast === "function") saiToast(mindMsg, "error");
  } finally {
    mindSaving = false;
    if (location.hash === "#/papers/dashboard") renderPapersDashboard();
  }
}

/** Mind の編集をやめる（書きかけが残っていれば確かめてから捨てる）。 */
async function cancelMindEdit() {
  const saved = projectData.mind == null ? "" : String(projectData.mind);
  if (mindDraft !== saved) {
    const ok = await paperConfirm({
      title: "書きかけの Mind を捨てますか",
      body: "保存していない書き換えは消えます。",
      ok: "捨てる",
      cancel: "続ける",
      danger: true,
    });
    if (!ok) return;
  }
  mindEditing = false;
  mindDraft = "";
  mindMsg = "";
  renderPapersDashboard();
}

/** 考え（md）を表示する（長いものははじめはたたんで「全部見る」）。 */
function projectMd(text, emptyText) {
  const src = String(text == null ? "" : text);
  if (!src.trim()) return el("p", "lib-empty", emptyText);
  const info = typeof pimLongText === "function" ? pimLongText(src) : { long: false, lines: 1, peek: "" };
  const md = el("div", "pd-md");
  md.innerHTML = renderMarkdown(src, { toolbar: true });
  if (!info.long) return md;
  const wrap = el("div", "pd-md-fold");
  const peek = el("pre", "pd-md-peek", info.peek);
  const det = document.createElement("details");
  det.appendChild(el("summary", null, `全部見る（${info.lines} 行）`));
  det.appendChild(md);
  wrap.appendChild(peek);
  wrap.appendChild(det);
  return wrap;
}

// --- Next Action（research-tree の先頭 3 件） ---

/** Next Action。Node の「選ぶ」は Tree の画面の Node へ移る（選ぶ操作は Tree の中にある）。 */
function nextActionCard() {
  const card = foldCard("Next Action");
  if (sciStatus === null) {
    card.appendChild(el("p", "lib-empty", sciError || "読み込み中…"));
    return card;
  }
  if (!sciStatus.initialized) {
    card.appendChild(el("p", "lib-empty", "research-tree がまだありません"));
    const go = document.createElement("button");
    go.type = "button";
    go.className = "sai-btn small";
    go.textContent = "Tree を開く";
    go.addEventListener("click", () => { location.hash = "#/research/tree"; });
    card.appendChild(go);
    return card;
  }
  const list = typeof stNextLines === "function" ? stNextLines(sciStatus, 3) : [];
  if (!list.length) {
    card.appendChild(el("p", "lib-empty", "今すぐやることはありません"));
  } else if (typeof stNextRows === "function") {
    card.appendChild(stNextRows(list, sciStatus, (x) => {
      if (x.node) location.hash = `#/research/tree/${encodeURIComponent(x.node)}`;
    }));
  }
  return card;
}

// --- Recent Activity（論文の動き＋research-tree の History・新しい順） ---

/** Recent Activity。History は stHistoryText（scitree.js）で文にする。 */
function recentActivityCard() {
  const card = foldCard("Recent Activity");
  if (!dashboardData && !dashboardError) {
    card.appendChild(el("p", "lib-empty", "読み込み中…"));
    return card;
  }
  const history = sciStatus && sciStatus.initialized ? sciStatus.history : [];
  const textOf = typeof stHistoryText === "function" ? (h) => stHistoryText({ ...h, at: null }) : () => "";
  const items = pdMergeActivity(dashboardData ? dashboardData.activity : [], history, textOf);
  if (items.length === 0) card.appendChild(el("p", "lib-empty", "No activity yet"));
  for (const item of items) card.appendChild(activityRow(item));
  if (dashboardError) card.appendChild(el("p", "paper-msg", dashboardError));
  return card;
}

function activityRow(item) {
  const [label, cls] = ACTIVITY_KINDS[item && item.kind] || [String((item && item.kind) || ""), "ak-other"];
  const row = document.createElement("button");
  row.className = "pd-act";
  row.appendChild(el("span", `pd-kind ${cls}`, label));
  row.appendChild(el("span", "pd-label", (item && item.label) || ""));
  row.appendChild(el("span", "pd-time", activityTime(item && item.occurred_at)));
  const hash = activityHash(item, selectedProject);
  if (hash) row.addEventListener("click", () => { location.hash = hash; });
  else row.disabled = true;
  return row;
}

// --- 共通: 上部の切り替え（Discover / Library / Radio・07 §4.1） ---

function fillSegment(containerId, active, segs) {
  const box = document.getElementById(containerId);
  if (!box) return;
  box.innerHTML = "";
  // 切り替え帯を行に入れる（帯の外側）
  const row = document.createElement("div");
  row.className = "papers-seg-bar";
  const seg = document.createElement("div");
  seg.className = "dash-seg papers-seg"; // 06 の期間切り替えと同じ見た目
  const items = Array.isArray(segs) ? segs : RESEARCH_SEG;
  for (const [key, label, hash] of items) {
    const b = document.createElement("button");
    b.textContent = label;
    if (key === active) b.className = "active";
    b.addEventListener("click", () => { location.hash = hash; });
    seg.appendChild(b);
  }
  row.appendChild(seg);
  box.appendChild(row);
}

// --- 広い画面の「左の一覧 + 右の中身」（U6・狭い画面では作らない） ---

/**
 * 一覧のコンテナの横に中身の欄を作る（広い画面だけ）。
 * 右の欄（HTMLElement）を返す。狭い画面では null。
 */
function paperSidePane(hostId, mainId) {
  if (typeof saiIsWide !== "function" || !saiIsWide()) return null;
  const host = document.getElementById(hostId);
  if (!host || !host.parentNode) return null;
  let main = document.getElementById(mainId);
  if (!main) {
    main = document.createElement("div");
    main.id = mainId;
    main.className = "sai-split-main paper-side";
    const wrap = document.createElement("div");
    wrap.className = "sai-split papers-split";
    host.parentNode.insertBefore(wrap, host);
    const list = document.createElement("div");
    list.className = "sai-split-list";
    wrap.appendChild(list);
    list.appendChild(host); // 一覧は左の欄の中に入れる（住所は変えない）
    wrap.appendChild(papersSplitHandle(wrap));
    wrap.appendChild(main);
  }
  return main;
}

const PAPERS_SPLIT_KEY = "sai_web_papers_split";

/** 左の階層と右の欄の境界線（ドラッグで左の幅を変え、localStorage に覚える）。 */
function papersSplitHandle(wrap) {
  try {
    const w = Number(localStorage.getItem(PAPERS_SPLIT_KEY));
    if (w > 0) wrap.style.setProperty("--papers-list-w", `${w}px`);
  } catch { /* 読めなければ既定の幅 */ }
  const bar = document.createElement("div");
  bar.className = "papers-split-handle";
  bar.setAttribute("role", "separator");
  bar.setAttribute("aria-orientation", "vertical");
  bar.addEventListener("pointerdown", (e) => {
    e.preventDefault();
    try { bar.setPointerCapture(e.pointerId); } catch { /* 捕まえなくてもよい */ }
    bar.classList.add("dragging");
  });
  bar.addEventListener("pointermove", (e) => {
    if (!bar.classList.contains("dragging")) return;
    const r = wrap.getBoundingClientRect();
    // 左は 200px 以上、右は 320px 以上残す
    const w = Math.max(200, Math.min(e.clientX - r.left, r.width - 320));
    wrap.style.setProperty("--papers-list-w", `${Math.round(w)}px`);
  });
  const end = (e) => {
    if (!bar.classList.contains("dragging")) return;
    bar.classList.remove("dragging");
    try { bar.releasePointerCapture(e.pointerId); } catch { /* 解放できなくてもよい */ }
    try {
      const list = wrap.querySelector(".sai-split-list");
      localStorage.setItem(PAPERS_SPLIT_KEY, String(Math.round(list.getBoundingClientRect().width)));
    } catch { /* 保存できなくてもよい */ }
  };
  bar.addEventListener("pointerup", end);
  bar.addEventListener("pointercancel", end);
  return bar;
}

/** 右の欄を「まだ選んでいません」の案内に戻す。 */
function paperSideEmpty(main, body) {
  if (!main) return;
  main.innerHTML = "";
  main.appendChild(
    papersEmpty("📄", "論文を選んでください", body || "左の一覧から論文を選ぶと、ここに中身が出ます。", null)
  );
}

/** 広い画面なら右の欄に論文を出す（狭い画面なら今まで通り住所を変える）。 */
function showPaperInSide(main, projectName, paperId) {
  if (!main) {
    location.hash = paperHash(projectName, paperId, false);
    return;
  }
  sideKind = "paper";
  sidePaperId = paperId;
  sideRemote = null;
  sideLog = null;
  sideFind = null;
  main.dataset.showing = `paper:${paperId}`;
  // 音声や図の置き場はここ（論文情報の変数）
  paperProject = projectName;
  paperIdNum = paperId;
  main.innerHTML = "";
  main.appendChild(el("p", "lib-empty", "読み込み中…"));
  void loadPaper(paperId, main);
}

/** 検索の実行者の文（human / run 12 · block 名）。log.actor が無い古い応答は空。 */
function searchActorText(log) {
  const a = log && log.actor;
  if (!a) return "";
  if (a.kind !== "agent") return "実行者: human";
  const parts = [a.run_id != null ? `run ${a.run_id}` : "", a.block || ""].filter(Boolean);
  return `実行者: ${parts.join(" · ") || "agent"}`;
}

/** 検索結果の見出しの下の行（日付・検索先・実行者・結び付いた Node へのリンク）。 */
function searchMetaLine(log) {
  const line = el("div", "paper-meta", [log.date, log.source_dbs, searchActorText(log)].filter(Boolean).join(" · "));
  if (log.node && log.node.id) {
    line.appendChild(document.createTextNode(" · "));
    const a = el("a", "paper-doi", log.node.id);
    a.href = `#/research/tree/${log.node.id}`; // research-tree の Node（検索の記録の持ち主）
    line.appendChild(a);
  }
  return line;
}

/** 広い画面なら右の欄に検索結果の一覧を出す（旧 Discover の結果画面）。狭い画面は過去の検索結果の画面へ。 */
async function showLogInSide(main, projectName, logId) {
  if (!main) {
    location.hash = logHash(projectName, logId, null);
    return;
  }
  sideKind = "log";
  sidePaperId = null;
  sideRemote = null;
  sideFind = null;
  sideLog = { logId, name: projectName };
  main.dataset.showing = `log:${logId}`;
  main.innerHTML = "";
  main.appendChild(el("p", "lib-empty", "読み込み中…"));
  let data;
  let error = "";
  try {
    data = await getLog(projectName, logId);
  } catch (e) {
    error = (e && e.message) || "読み込めませんでした";
  }
  if (sideKind !== "log" || !sideLog || sideLog.logId !== logId) return; // 読み込み中に別の物へ移った
  main.innerHTML = "";
  if (error) {
    main.appendChild(el("p", "lib-empty", error));
    return;
  }
  const all = Array.isArray(data.items) ? data.items : [];
  const failedBox = papersFailedBox(data);
  if (failedBox) main.appendChild(failedBox);
  // 「未DLも表示」を切るとダウンロード済みだけにする（左の階層は常にダウンロード済みだけ）
  const showRemote = libRemote();
  const items = showRemote ? all : all.filter((it) => it.local_paper_id != null && it.local_paper_id !== "");
  main.appendChild(el("h2", "papers-heading", `“${(data.log && data.log.query) || ""}” の結果 ${items.length} 件`));
  main.appendChild(searchMetaLine(data.log || {}));
  const label = el("label", "lib-remote-toggle");
  const box = document.createElement("input");
  box.type = "checkbox";
  box.checked = showRemote;
  box.addEventListener("change", () => {
    saveLibRemote(box.checked);
    void showLogInSide(main, projectName, logId);
  });
  label.appendChild(box);
  label.appendChild(document.createTextNode(" 未DLも表示"));
  main.appendChild(label);
  if (items.length === 0) {
    // 全部の検索先が失敗したときは「見つかりません」と言わない（理由を出して終える）
    if (!papersAllFailed(data.failed, data.log && data.log.source_dbs)) {
      main.appendChild(
        papersEmpty("🔍", "見つかりませんでした", "別の言葉で探すか、「Filters」で検索先や年を変えてみてください。", null)
      );
    }
    return;
  }
  for (const item of items) main.appendChild(resultRow(item, projectName, logId));
}

/** 広い画面なら右の欄に検索の結果（未ダウンロードの論文の情報）を出す。 */
async function showRemoteInSide(main, projectName, logId, position) {
  if (!main) {
    location.hash = logHash(projectName, logId, position);
    return;
  }
  sideKind = "remote";
  sidePaperId = null;
  sideLog = null;
  sideFind = null;
  sideRemote = { logId, name: projectName, position };
  main.dataset.showing = `remote:${logId}:${position}`;
  remoteView = {
    logId,
    name: projectName,
    position,
    data: logCache.get(logCacheKey(projectName, logId)) || null,
    error: "",
  };
  renderRemote(main);
  if (remoteView.data) return;
  try {
    remoteView.data = await getLog(projectName, logId);
  } catch (e) {
    remoteView.error = (e && e.message) || "読み込めませんでした";
  }
  renderRemote(main);
}

/** 右の欄は必要なところだけ描き直す（同じものは描き直さない）。 */
function paperSideRender(main) {
  if (!main) return;
  const want = sideKind === "paper" && sidePaperId != null
    ? `paper:${sidePaperId}`
    : sideKind === "remote" && sideRemote
      ? `remote:${sideRemote.logId}:${sideRemote.position}`
      : sideKind === "log" && sideLog
        ? `log:${sideLog.logId}`
        : sideKind === "find" && sideFind
          ? `find:${sideFind.logId}`
          : "";
  if (main.dataset.showing === want) return;
  main.dataset.showing = want;
  if (sideKind === "paper" && sidePaperId != null) {
    showPaperInSide(main, selectedProject, sidePaperId);
    return;
  }
  if (sideKind === "remote" && sideRemote) {
    void showRemoteInSide(main, sideRemote.name, sideRemote.logId, sideRemote.position);
    return;
  }
  if (sideKind === "log" && sideLog) {
    void showLogInSide(main, sideLog.name, sideLog.logId);
    return;
  }
  if (sideKind === "find" && sideFind) {
    void lfindShowInSide(main, sideFind.name, sideFind.logId);
    return;
  }
  paperSideEmpty(main);
}

/** 空の画面の案内（saiEmpty と同じ形。ui.js が無いときも動くように直接作る）。 */
function papersEmpty(icon, title, body, actions) {
  const box = el("div", "sai-empty");
  if (icon) box.appendChild(el("div", "sai-empty-icon", icon));
  if (title) box.appendChild(el("p", "sai-empty-title", title));
  if (body) box.appendChild(el("p", "sai-empty-body", body));
  if (actions && actions.length > 0) {
    const row = el("div", "papers-empty-actions");
    actions.forEach((a, i) => {
      const b = document.createElement("button");
      b.type = "button";
      b.className = i === 0 ? "sai-btn primary" : "sai-btn";
      b.textContent = a.label;
      b.addEventListener("click", a.onClick);
      row.appendChild(b);
    });
    box.appendChild(row);
  }
  return box;
}

// --- Discover の検索の失敗（`phone search` の `failed`・08 §3） ---

/** 検索先の表示名（画面の検索先の札は DB ID そのものの名前で出す）。 */
function papersDbLabel(db) {
  const text = String(db == null ? "" : db).trim();
  return text || "Source";
}

/**
 * 検索の失敗（`{"db": "s2", "reason": "…"}` の列）を利用者に見せる 1 行ずつにする。
 * 理由は 429（混雑）・403（キー不行）・そのほかで分ける。空配列なら空配列。
 */
function papersFailedLines(failed) {
  const list = Array.isArray(failed) ? failed : [];
  const lines = [];
  for (const f of list) {
    if (!f) continue;
    const name = papersDbLabel(f.db);
    const reason = String(f.reason || "");
    if (reason.includes("429")) {
      lines.push(`${name} は混雑していて結果を取れませんでした。少し待ってから、もう一度検索してください。`);
    } else if (reason.includes("403")) {
      lines.push(`${name} の API キーが使えませんでした（~/.sai/paper.env を確かめてください）。`);
    } else {
      lines.push(`${name} で検索できませんでした: ${reason.slice(0, 120)}`);
    }
  }
  return lines;
}

/** 選んだ検索先すべてが失敗したかどうか（「見つかりません」を出さない判定）。 */
function papersAllFailed(failed, sourceDbs) {
  const list = Array.isArray(failed) ? failed : [];
  if (list.length === 0) return false;
  const tried = String(sourceDbs || "")
    .split(",")
    .map((x) => x.trim())
    .filter(Boolean);
  if (tried.length === 0) return false;
  const names = new Set(list.map((f) => papersDbLabel(f && f.db)));
  return tried.every((db) => names.has(db));
}

/** 検索の失敗の帯（注意の文と「もう一度検索」ボタン）。 */
function papersFailedBox(data) {
  const lines = papersFailedLines(data && data.failed);
  if (lines.length === 0) return null;
  const box = document.createElement("div");
  box.className = "papers-failed";
  for (const line of lines) box.appendChild(el("p", "papers-failed-line", line));
  box.appendChild(paperBtn("もう一度検索", () => { void runDiscoverSearch(); }, "primary"));
  return box;
}

/** 用語の札（ui.js の saiTerm。無いときは札を出さない）。 */
function paperTerm(key, label) {
  if (typeof saiTerm !== "function") return null;
  return saiTerm(key, label);
}

/** 確認（ui.js の saiConfirm。無いときはブラウザの confirm で代用する）。 */
async function paperConfirm(opts) {
  if (typeof saiConfirm === "function") return saiConfirm(opts);
  return window.confirm(`${(opts && opts.title) || ""}\n${(opts && opts.body) || ""}`);
}

/** Papers の新しい画面からの読み書き（ui.js の saiApi と同じ動き。app.js の apiFetch を使う）。 */
async function paperApi(path, options) {
  const opts = { ...(options || {}) };
  if (opts.body != null && typeof opts.body !== "string") {
    opts.body = JSON.stringify(opts.body);
    opts.headers = { ...(opts.headers || {}), "Content-Type": "application/json" };
  }
  let res;
  try {
    res = await apiFetch(path, opts);
  } catch {
    throw new Error("Mac につながりません");
  }
  const body = res.status === 204 ? null : await res.json().catch(() => null);
  if (!res.ok) {
    const detail = body && (body.detail || body.message);
    const err = new Error(detail || `読み込めませんでした（${res.status}）`);
    err.status = res.status;
    throw err;
  }
  return body;
}

// --- 仕事の表示（paperimport.js のを見張り・U6・U13） ---

/** Download の状態を入れる箱を作る（key で見つかるように覚えておく）。 */
function jobBox(key) {
  const box = el("div", "pim-job");
  jobBoxes.push({ key, box });
  fillJobBox(box, key);
  return box;
}

/** 1 つの箱に仕事の状態を入れる。 */
function fillJobBox(box, key) {
  box.innerHTML = "";
  const jobs = typeof pimJobsList === "function" ? pimJobsList() : [];
  const job = typeof pimJobFor === "function" ? pimJobFor(jobs, key) : null;
  box.hidden = !job;
  if (job && typeof pimJobBlock === "function") box.appendChild(pimJobBlock(job));
}

/** 画面の中に置いた Download の箱をすべて描き直す（paperimport.js から呼ばれる）。 */
function papersRenderJobBoxes() {
  for (const item of jobBoxes) {
    if (!item.box.isConnected) continue;
    fillJobBox(item.box, item.key);
  }
}

/** Library の画面の上に「取り込み中 N 件」の細い帯を出す（仕事が走るときだけ）。 */
function papersRenderDownloadBand() {
  const view = document.getElementById("view-papers-library");
  if (!view) return;
  if (!downloadBand) {
    downloadBand = el("div", "papers-dl-band");
    downloadBand.hidden = true;
    const tools = view.querySelector(".lib-tools");
    if (tools) view.insertBefore(downloadBand, tools);
    else view.appendChild(downloadBand);
  }
  const jobs = typeof pimActiveJobs === "function"
    ? pimActiveJobs(typeof pimJobsList === "function" ? pimJobsList() : [])
    : [];
  if (jobs.length === 0) {
    downloadBand.hidden = true;
    downloadBand.textContent = "";
    return;
  }
  downloadBand.hidden = false;
  downloadBand.textContent = `取り込み中 ${jobs.length} 件（進み具合は各論文の情報で見られます）`;
}

/** 3 秒ごとの更新で Download の進み具合も見る（paperimport.js があれば）。 */
async function papersPollExtras() {
  if (typeof pimPollJobs === "function") await pimPollJobs();
}

// --- Library（#/papers/library・07 §4.3） ---

/**
 * Library の住所。`#/papers/library`・論文を右の欄に開く `#/papers/library/<論文の番号>`・
 * 検索結果を右の欄に開く `#/papers/library/log/<検索の番号>`・
 * U03: 内部検索の結果を開く `#/papers/library/find/<番号>`。
 */
function parseLibraryHash(hash) {
  const m = /^#\/papers\/library(?:\/((?:log|find)\/)?([1-9][0-9]{0,8}))?$/.exec(String(hash || ""));
  if (!m) return null;
  const n = m[2] ? Number(m[2]) : null;
  const kind = m[1] ? m[1].replace(/\/$/, "") : "";
  return {
    paper: kind === "" ? n : null,
    log: kind === "log" ? n : null,
    find: kind === "find" ? n : null,
  };
}

/** 左の階層を出したまま検索結果を右に開く住所（research-tree の「検索の記録」から使う）。 */
function libraryLogHash(id) {
  return `#/papers/library/log/${encodeURIComponent(id)}`;
}

/** 左の階層を出したまま内部検索の結果を右に開く住所（U03。他の画面が使えるように置く）。 */
function libraryFindHash(id) {
  return `#/papers/library/find/${encodeURIComponent(id)}`;
}

/** 左の階層を出したまま論文の詳細を右に開く住所（他の画面の出典リンクから使う）。 */
function libraryPaperHash(id) {
  return `#/papers/library/${encodeURIComponent(id)}`;
}

async function openLibrary(opts) {
  // 論文情報の戻る先として今の住所を覚える（07 §4.2）
  savePapersList("#/papers/library");
  fillSegment("library-seg", "library");
  const input = document.getElementById("library-filter");
  if (input) input.value = libFilter;
  const dInput = document.getElementById("discover-input");
  if (dInput) dInput.value = discoverQuery;
  syncCondUI();
  // どのプロジェクトの Library を出すか決める（↻ は一覧も取り直す・18 §1.1）
  await ensurePaperProjects(opts && opts.force);
  if (!parseLibraryHash(location.hash)) return; // 既に別の画面へ移っている
  if (opts && opts.log != null && selectedProject) {
    if (typeof saiIsWide === "function" && saiIsWide()) {
      // 広い画面は左の階層を出したまま、右の欄に検索結果を開く
      sideKind = "log";
      sidePaperId = null;
      sideRemote = null;
      sideFind = null;
      sideLog = { logId: opts.log, name: selectedProject };
    } else {
      location.replace(logHash(selectedProject, opts.log, null));
      return;
    }
  }
  if (opts && opts.find != null && selectedProject) {
    if (typeof saiIsWide === "function" && saiIsWide()) {
      // 広い画面は左の階層を出したまま、右の欄に内部検索の結果を開く（U03）
      sideKind = "find";
      sidePaperId = null;
      sideRemote = null;
      sideLog = null;
      sideFind = { logId: opts.find, name: selectedProject };
    } else {
      location.replace(findHash(selectedProject, opts.find));
      return;
    }
  }
  if (opts && opts.paper != null && selectedProject) {
    if (typeof saiIsWide === "function" && saiIsWide()) {
      // 広い画面は左の階層を出したまま、右の欄に論文の詳細を開く
      sideKind = "paper";
      sidePaperId = opts.paper;
      sideRemote = null;
      sideLog = null;
      sideFind = null;
    } else {
      // 狭い画面は右の欄が無いので、論文の詳細の画面へ
      location.replace(paperHash(selectedProject, opts.paper, false));
      return;
    }
  }
  if (!selectedProject) {
    // 置き場があるプロジェクトが1つも無い（案内文・18 §1.1）
    libPapers = null;
    libGroups = null;
    libFindGroups = null;
    libError = "";
    renderLibrary();
    return;
  }
  // まだ何も無いときは「読み込み中…」を先に出す（07 §4.2）
  if (libPapers === null) renderLibrary();
  // 走っている Download があれば上の帯を出す（Library を開いたときの一度きりの読み出し）
  if (typeof pimRefreshJobs === "function") void pimRefreshJobs(selectedProject);
  void loadProviders(); // 検索欄の条件（検索先）
  await loadLibrary();
}

async function loadLibrary() {
  const hash = location.hash;
  const name = selectedProject;
  if (!name) return;
  try {
    const res = await apiFetch(`${papersApiBase(name)}/library`);
    const body = await res.json().catch(() => ({}));
    if (!res.ok) throw new Error(body.detail || "読み込めませんでした");
    const res2 = await apiFetch(`${papersApiBase(name)}/tree`);
    const body2 = await res2.json().catch(() => ({}));
    if (!res2.ok) throw new Error(body2.detail || "階層を読み込めませんでした");
    const groups = Array.isArray(body2.groups) ? body2.groups : [];
    const findGroups = Array.isArray(body2.find_groups) ? body2.find_groups : [];
    // 既に別の画面へ移った，またはその間にプロジェクトが切り替わったら捨てる（18 §1.1）
    if (location.hash !== hash || selectedProject !== name) return;
    libPapers = Array.isArray(body.papers) ? body.papers : [];
    libGroups = groups;
    libFindGroups = findGroups;
    libError = "";
    window.SaiShell.ok();
  } catch (e) {
    if (location.hash !== hash || selectedProject !== name) return;
    // 失敗しても前の表示は残し，帯だけ出す（00 §9）
    libError = (e && e.message) || "読み込めませんでした";
    window.SaiShell.fail();
    renderLibrary();
    return;
  }
  renderLibrary();
}

function renderLibrary() {
  const box = document.getElementById("library-list");
  if (!box) return;
  jobBoxes.length = 0; // Library を描き直したら前の Download の箱はもう無い
  papersRenderDownloadBand(); // 走っている Download の帯
  if (paperProjects !== null && !selectedProject) {
    // 置き場があるプロジェクトが1つも無い（18 §1.1）
    box.innerHTML = "";
    box.appendChild(
      papersEmpty(
        "📁",
        "論文の置き場のあるプロジェクトがありません",
        paperProjectsError || NO_PROJECT_MSG,
        null
      )
    );
    return;
  }
  if (libPapers === null) {
    // まだ何も無いときは失敗の detail をそのまま出す（00 §9）
    box.innerHTML = "";
    box.appendChild(el("p", "lib-empty", libError || paperProjectsError || "読み込み中…"));
    return;
  }
  box.innerHTML = "";
  if (libGroups) renderTree(box);
  else box.appendChild(el("p", "lib-empty", libError || "読み込み中…"));
  // 広い画面は左の階層・右に検索結果や選んだ論文の中身を出す（狭い画面は作らない）
  const side = paperSidePane("library-list", PAPER_SIDE_IDS.library);
  if (side) paperSideRender(side);
}

/** 一覧の行（階層の中は small で小さく・題名 1 行＋年）。withId のときは題名の前に `#番号` を付ける。 */
function paperRow(p, small, withId) {
  const btn = document.createElement("button");
  btn.className = small ? "paper-row small" : "paper-row";
  const title = el("div", "paper-title");
  if (withId) title.appendChild(el("span", "paper-no", `#${p.id}`));
  title.appendChild(document.createTextNode(withId ? ` ${p.title || "(無題)"}` : p.title || "(無題)"));
  btn.appendChild(title);

  if (small) {
    btn.appendChild(el("div", "paper-meta", String(p.year || "")));
  } else {
    const meta = [authorShort(p.authors), p.year, p.journal]
      .map((x) => String(x || "").trim())
      .filter(Boolean)
      .join(" · ");
    if (meta) btn.appendChild(el("div", "paper-meta", meta));

    // 札: 読み状態と音声の有無（07 §4.3）
    const tags = [];
    const reading = READING_LABEL[p.reading_status];
    if (reading) tags.push(reading);
    if (p.podcast === "ready") tags.push("🎧 音声あり");
    else if (p.podcast === "generating") tags.push("音声作成中");
    if (tags.length) {
      const row = document.createElement("div");
      row.className = "paper-tags";
      for (const t of tags) row.appendChild(el("span", "paper-tag", t));
      btn.appendChild(row);
    }
  }

  btn.addEventListener("click", () => {
    // 広い画面は右の欄に出し、狭い画面は今まで通り論文情報の画面へ（U6）
    const side = paperSidePane("library-list", PAPER_SIDE_IDS.library);
    if (side) {
      showPaperInSide(side, selectedProject, p.id);
      return;
    }
    location.hash = paperHash(selectedProject, p.id, false);
  });
  return btn;
}

/** まとまりのキー（開閉を覚えるため・07 §4.3）。U03: 内部検索の組は find:<log_id>。 */
function treeKey(g) {
  if (g.kind === "query") return `query:${g.log_id}`;
  if (g.kind === "find") return `find:${g.log_id}`;
  return `import:${g.date}`;
}

/** U03: 内部検索の履歴（find_groups）を階層の組の形にそろえる。 */
function findGroupRows() {
  return (libFindGroups || []).map((g) => ({
    kind: "find",
    log_id: g.log_id,
    date: g.date,
    query: g.query,
    hit_count: g.hit_count,
    paper_ids: Array.isArray(g.paper_ids) ? g.paper_ids : [],
    actor: g.actor || null,
  }));
}

/** Paper 一覧: Library の論文を番号（papers.id）の昇順に並べる。`#1 題名 年` */
function renderPaperList(box) {
  const words = queryWords(libFilter);
  const rows = [...(libPapers || [])]
    .sort((a, b) => Number(a.id) - Number(b.id))
    .filter((p) => includesAll(`#${p.id} ${p.title || ""} ${p.authors || ""}`, words));
  if (rows.length === 0) {
    const none = (libPapers || []).length === 0;
    box.appendChild(
      papersEmpty(
        "📚",
        none ? "まだ論文がありません" : "一致する論文がありません",
        none ? "論文は各自の Mac の sai-prep から送ります。" : "上の絞り込みの欄の言葉を短くすると出てきます。",
        null
      )
    );
    return;
  }
  for (const p of rows) box.appendChild(paperRow(p, true, true));
}

function renderTree(box) {
  if (libFindKind === "paper") { renderPaperList(box); return; }
  const words = queryWords(libFilter);
  const folds = loadFolds(selectedProject);
  const byId = new Map((libPapers || []).map((p) => [p.id, p]));
  let shown = 0;
  // U03: 内部のときは find_groups、外部のときは kind === "query" の組。Import は常に出す。
  const queryGroups = (libGroups || []).filter((g) => g && g.kind === "query");
  const importGroups = (libGroups || []).filter((g) => g && g.kind !== "query");
  const groups = libFindKind === "internal"
    ? [...findGroupRows(), ...importGroups]
    : [...queryGroups, ...importGroups];

  for (const g of groups) {
    // 見出しの言葉が合う場合は中身を絞らない（全部出す・07 §4.3）
    const heading = g.kind === "find" || g.kind === "query"
      ? `${g.date || ""} ${g.query || ""}`
      : `${g.date || ""} Import`;
    const headingHit = includesAll(heading, words);
    const all = (g.paper_ids || []).map((id) => byId.get(id)).filter(Boolean);
    const rows = headingHit
      ? all
      : all.filter((p) => includesAll(`${p.title || ""} ${p.authors || ""}`, words));

    const key = treeKey(g);
    const open = folds.includes(key);
    const isQuery = g.kind === "query" && g.log_id != null;
    const isFind = g.kind === "find" && g.log_id != null;
    // 合う論文が 0 件のまとまりは出さない（見出しの合致時を除く・07 §4.3）
    if (rows.length === 0 && !headingHit) continue;

    // 見出しの行は「▶ の開閉」と「文字（検索結果を右の欄に出す）」に分ける
    const head = document.createElement("div");
    head.className = "lib-group-head";
    if (isQuery && sideKind === "log" && sideLog && sideLog.logId === g.log_id) head.classList.add("active");
    if (isFind && sideKind === "find" && sideFind && sideFind.logId === g.log_id) head.classList.add("active");
    const fold = () => {
      const cur = loadFolds(selectedProject);
      saveFolds(selectedProject, cur.includes(key) ? cur.filter((x) => x !== key) : [...cur, key]);
      renderLibrary();
    };
    const arrow = document.createElement("button");
    arrow.className = "lib-g-twisty";
    arrow.textContent = open ? "▼" : "▶";
    arrow.setAttribute("aria-expanded", String(open));
    arrow.setAttribute("aria-label", open ? "畳む" : "開く");
    arrow.addEventListener("click", fold);
    head.appendChild(arrow);
    const toggle = document.createElement("button");
    toggle.className = "lib-g-main";
    toggle.appendChild(el("span", "lib-g-date", String(g.date || "")));
    toggle.appendChild(el("span", "lib-g-query", (g.kind === "query" || g.kind === "find") ? String(g.query || "(検索)") : "Import"));
    // U03: 内部の組にも agent の札。外部と同じ作り。
    if ((isQuery || isFind) && g.actor && g.actor.kind === "agent") {
      toggle.appendChild(el("span", "paper-tag", "agent"));
    }
    // query・find は (ダウンロード数/ヒット数)・Import は論文数
    toggle.appendChild(el("span", "lib-g-count", (isQuery || isFind) ? `(${all.length}/${g.hit_count ?? all.length})` : `(${all.length})`));
    toggle.addEventListener("click", () => {
      if (!isQuery && !isFind) { fold(); return; } // Import の見出しは検索結果が無いので開閉だけ
      if (isFind) {
        // U03: 保存された内部検索の結果を開き直す（検索し直さない）
        const side = paperSidePane("library-list", PAPER_SIDE_IDS.library);
        if (side) {
          void lfindShowInSide(side, selectedProject, g.log_id);
          renderLibrary();
        } else {
          location.hash = findHash(selectedProject, g.log_id);
        }
        return;
      }
      // 広い画面は右の欄に検索結果を出し、狭い画面は過去の検索結果の画面へ（左の階層は動かさない）
      const side = paperSidePane("library-list", PAPER_SIDE_IDS.library);
      if (side) {
        void showLogInSide(side, selectedProject, g.log_id);
        renderLibrary(); // 選んでいる見出しの強調を付け替える
      } else {
        location.hash = logHash(selectedProject, g.log_id, null);
      }
    });
    head.appendChild(toggle);
    box.appendChild(head);
    shown += 1;
    if (!open) continue;
    for (const p of rows) box.appendChild(paperRow(p, true));
  }

  if (shown === 0) {
    const internalEmpty = libFindKind === "internal" && (libFindGroups || []).length === 0;
    box.appendChild(
      papersEmpty(
        "📚",
        libPapers.length === 0
          ? "まだ論文がありません"
          : internalEmpty
            ? "まだ内部検索の履歴がありません"
            : "一致する論文がありません",
        libPapers.length === 0
          ? "論文は各自の Mac の sai-prep から送ります。"
          : internalEmpty
            ? "上の欄で Library 内を検索してください。"
            : "上の絞り込みの欄の言葉を短くすると出てきます。",
        null
      )
    );
  }
}

/** 著者の欄（配列でも文字でも 1 本の文字にする）。 */
function authorsText(authors) {
  return Array.isArray(authors) ? authors.join(", ") : String(authors || "");
}

// --- Radio は radio.js（計画 09）が持つ ---
// （かつての準備中画面 openSoon は 09 で radio.js に置き換わった）

// --- 検索（Library の上の検索欄・旧 Discover・08 §4.1） ---

/** 条件の札と要約を今の値に合わせる（08 §4.1）。U03: 外部／内部の切り替えもここで合わせる。 */
function syncCondUI() {
  // U03: 切り替えのボタンの強調と検索欄の言葉
  const kindBox = document.getElementById("library-kind");
  if (kindBox) {
    kindBox.querySelectorAll("button").forEach((b) => {
      b.classList.toggle("active", b.dataset.kind === libFindKind);
    });
  }
  const searchInput = document.getElementById("discover-input");
  if (searchInput) {
    searchInput.placeholder = libFindKind === "internal" ? "🔍 Library 内を検索" : "🔍 論文を検索";
  }
  // Paper 一覧では検索欄の行と条件の開閉を隠す（.lib-tools は display:flex なので hidden では消えない）
  const isPaper = libFindKind === "paper";
  const searchRow = searchInput && searchInput.closest(".lib-tools");
  if (searchRow) searchRow.style.display = isPaper ? "none" : "";
  const condToggle = document.getElementById("discover-cond-toggle");
  const condRow = condToggle && condToggle.closest(".papers-cond-row");
  if (condRow) condRow.style.display = isPaper ? "none" : "";
  const filterInput = document.getElementById("library-filter");
  if (filterInput) filterInput.placeholder = isPaper ? "番号・題名・著者で絞り込み" : "題名・著者で絞り込み";
  const panel = document.getElementById("discover-cond");
  if (panel) panel.hidden = libFindKind === "external" ? !discoverCondOpen : true;
  const findPanel = document.getElementById("find-cond");
  if (findPanel) findPanel.hidden = libFindKind === "internal" ? !discoverCondOpen : true;
  const toggle = document.getElementById("discover-cond-toggle");
  if (toggle) {
    toggle.textContent = discoverCondOpen ? "Filters ▾" : "Filters ▸";
    toggle.setAttribute("aria-expanded", discoverCondOpen ? "true" : "false");
  }
  const summary = document.getElementById("discover-cond-summary");
  if (summary) {
    if (libFindKind === "internal") {
      summary.textContent = lfindSummary({
        mode: libFindMode, scope: libFindScope, count: libFindCount, node: libFindNode,
      });
    } else if (discoverProviders) {
      summary.textContent = (discoverDbs || []).join(" · ");
    }
  }
  // 検索先の札（押すと選択・解除・08 §4.1）
  const dbsBox = document.getElementById("discover-dbs");
  if (dbsBox && discoverProviders) {
    dbsBox.innerHTML = "";
    for (const db of discoverProviders.available) {
      const b = document.createElement("button");
      b.className = "papers-db";
      if ((discoverDbs || []).includes(db)) b.className += " active";
      b.textContent = db;
      b.dataset.db = db;
      dbsBox.appendChild(b);
    }
  }
  const yf = document.getElementById("discover-year-from");
  if (yf && document.activeElement !== yf) yf.value = discoverYears.from;
  const yt = document.getElementById("discover-year-to");
  if (yt && document.activeElement !== yt) yt.value = discoverYears.to;
  const limitBox = document.getElementById("discover-limit");
  if (limitBox) {
    limitBox.querySelectorAll("button").forEach((b) => {
      b.classList.toggle("active", Number(b.dataset.limit) === discoverLimit);
    });
  }
  // U03: 内部検索の条件の見た目
  const modeBox = document.getElementById("find-mode");
  if (modeBox) {
    modeBox.querySelectorAll("button").forEach((b) => {
      b.classList.toggle("active", (b.dataset.mode || "") === (libFindMode || ""));
    });
  }
  const scopeBox = document.getElementById("find-scope");
  if (scopeBox) {
    scopeBox.querySelectorAll("button").forEach((b) => {
      b.classList.toggle("active", (b.dataset.scope || "") === (libFindScope || ""));
    });
  }
  const countBox = document.getElementById("find-count");
  if (countBox) {
    countBox.querySelectorAll("button").forEach((b) => {
      b.classList.toggle("active", Number(b.dataset.count) === libFindCount);
    });
  }
  lfindRenderChips();
  lfindRenderNodeUI();
}

/** 検索先の一覧を読む（08 §4.1）。読めなくても検索自体は可能（sai-paper の既定を使う）。 */
async function loadProviders() {
  if (discoverProviders) return;
  const name = selectedProject;
  if (!name) return;
  let body;
  try {
    const res = await apiFetch(`${papersApiBase(name)}/providers`);
    body = await res.json().catch(() => ({}));
    if (!res.ok) throw new Error(body.detail || "検索先を読み込めませんでした");
  } catch (e) {
    if (selectedProject !== name) return; // その間に切り替わった（18 §1.1）
    const summary = document.getElementById("discover-cond-summary");
    if (summary) summary.textContent = (e && e.message) || "検索先を読み込めませんでした";
    window.SaiShell.fail();
    return;
  }
  if (selectedProject !== name) return; // その間に切り替わった（18 §1.1）
  discoverProviders = {
    default: Array.isArray(body.default) ? body.default : [],
    available: Array.isArray(body.available) ? body.available : [],
  };
  if (discoverDbs === null) {
    // 覚えていた選択 → 無ければ既定（08 §4.1）
    const stored = loadDiscoverDbs() || discoverProviders.default;
    const usable = stored.filter((d) => discoverProviders.available.includes(d));
    discoverDbs = usable.length > 0 ? usable : discoverProviders.default.slice();
    if (discoverDbs.length === 0) discoverDbs = discoverProviders.available.slice(0, 1);
  }
  syncCondUI();
  window.SaiShell.ok();
}

/** log の記憶の鍵（log_id はプロジェクトごとに違うため名前と組にする・18 §3.6）。 */
function logCacheKey(projectName, logId) {
  return `${projectName || ""}/${logId}`;
}

/** log_id の結果を取る（入っているなら取り直さない・08 §4.4）。失敗は throw。 */
async function getLog(projectName, logId) {
  const key = logCacheKey(projectName, logId);
  if (logCache.has(key)) return logCache.get(key);
  const res = await apiFetch(`${papersApiBase(projectName)}/search-history/${encodeURIComponent(logId)}`);
  const body = await res.json().catch(() => ({}));
  if (!res.ok) throw new Error(body.detail || "読み込めませんでした");
  if (!body || !body.log) throw new Error("sai-paper の応答を読めませんでした");
  logCache.set(key, body);
  return body;
}

/** 今の条件で送る検索先（読んでいる available にあるものだけ・08 §4.1）。 */
function selectedDbs() {
  if (!discoverDbs || discoverDbs.length === 0) return null;
  if (!discoverProviders) return discoverDbs.slice();
  const picked = discoverDbs.filter((d) => discoverProviders.available.includes(d));
  return picked.length > 0 ? picked : null;
}

/** 検索の進み具合・失敗を検索欄の下に出す（空文字なら隠す）。 */
function discoverStatus(text, busy) {
  const p = document.getElementById("discover-status");
  if (p) {
    p.hidden = !text;
    p.textContent = text || "";
  }
  const goBtn = document.getElementById("discover-go");
  if (goBtn) goBtn.disabled = !!busy;
}

async function runDiscoverSearch() {
  const input = document.getElementById("discover-input");
  const raw = (input && input.value) || "";
  discoverQuery = raw;
  const query = raw.trim();
  const name = selectedProject;
  if (!name) return; // 置き場が無いときは案内文が出ているだけ（18 §1.1）
  if (!query) {
    discoverMode = "idle";
    discoverStatus("", false);
    return;
  }
  discoverMode = "loading";
  discoverStatus("Mac の sai-paper で検索中…", true);

  const body = { query, limit: discoverLimit };
  const dbs = selectedDbs();
  if (dbs) body.dbs = dbs;
  const yf = parseInt(discoverYears.from, 10);
  if (discoverYears.from !== "" && Number.isFinite(yf)) body.year_from = yf;
  const yt = parseInt(discoverYears.to, 10);
  if (discoverYears.to !== "" && Number.isFinite(yt)) body.year_to = yt;

  let res;
  try {
    res = await apiFetch(`${papersApiBase(name)}/search`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(body),
    });
  } catch {
    if (selectedProject !== name) return; // その間に切り替わった（18 §1.1）
    discoverMode = "error";
    discoverStatus("接続できていません", false);
    window.SaiShell.fail();
    return;
  }
  const data = await res.json().catch(() => ({}));
  if (selectedProject !== name) return; // その間に切り替わった（古いプロジェクトの結果を出さない・18 §1.1）
  if (!res.ok || !data || !data.log) {
    // 失敗の detail を検索欄の下に出す（08 §4.1）
    discoverMode = "error";
    discoverStatus(
      res.ok ? "sai-paper の応答を読めませんでした" : (data.detail || "検索できませんでした"),
      false
    );
    return;
  }
  logCache.set(logCacheKey(name, data.log.id), data);
  discoverMode = "idle";
  discoverStatus("", false);
  window.SaiShell.ok();
  // 新しい検索が階層に増えるので読み直し、広い画面は右の欄に結果を出す（狭い画面は結果の画面へ）
  libGroups = null;
  const side = paperSidePane("library-list", PAPER_SIDE_IDS.library);
  if (side) void showLogInSide(side, name, data.log.id);
  else location.hash = logHash(name, data.log.id, null);
  await loadLibrary();
}

// --- U03: Library 内の検索（内部 DB） ---

/** 内部検索の結果を取る（入っているなら取り直さない）。失敗は throw。 */
async function getFindLog(projectName, logId) {
  const key = logCacheKey(projectName, logId);
  if (findLogCache.has(key)) return findLogCache.get(key);
  const res = await apiFetch(`${papersApiBase(projectName)}/find-history/${encodeURIComponent(logId)}`);
  const body = await res.json().catch(() => ({}));
  if (!res.ok) throw new Error(body.detail || "読み込めませんでした");
  if (!body || !body.log) throw new Error("sai-paper の応答を読めませんでした");
  findLogCache.set(key, body);
  return body;
}

/** U03: 内部検索の実行。検索語が空なら何もしない（外部と同じ）。 */
async function lfindRun() {
  const input = document.getElementById("discover-input");
  const raw = (input && input.value) || "";
  discoverQuery = raw;
  const name = selectedProject;
  if (!name) return;
  const built = lfindBody({
    query: raw,
    mode: libFindMode,
    scope: libFindScope,
    count: libFindCount,
    papers: libFindPapers,
    node: libFindNode,
    subtree: libFindSubtree,
    each: libFindEach,
  });
  if (!built.ok) {
    discoverStatus(built.error, false);
    return;
  }
  if (!built.body.query) {
    discoverStatus("", false);
    return;
  }
  discoverStatus("Mac の sai-paper で検索中…", true);
  let res;
  try {
    res = await apiFetch(`${papersApiBase(name)}/find`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(built.body),
    });
  } catch {
    if (selectedProject !== name) return;
    discoverStatus("接続できていません", false);
    window.SaiShell.fail();
    return;
  }
  const data = await res.json().catch(() => ({}));
  if (selectedProject !== name) return;
  if (!res.ok || !data || !data.log) {
    discoverStatus(
      res.ok ? "sai-paper の応答を読めませんでした" : (data.detail || "検索できませんでした"),
      false
    );
    return;
  }
  findLogCache.set(logCacheKey(name, data.log.id), data);
  discoverStatus("", false);
  window.SaiShell.ok();
  // 新しい検索が階層に増えるので読み直し、広い画面は右の欄に結果を出す（狭い画面は結果の画面へ）
  libGroups = null;
  libFindGroups = null;
  const side = paperSidePane("library-list", PAPER_SIDE_IDS.library);
  if (side) void lfindShowInSide(side, name, data.log.id);
  else location.hash = findHash(name, data.log.id);
  await loadLibrary();
}

/** U03: 内部検索の種別の札の文（source_kind → 日本語）。 */
function lfindKindLabel(kind) {
  const k = String(kind || "");
  if (k === "abstract") return "抄録";
  if (k === "body") return "本文";
  if (k === "note") return "ノート";
  if (k === "document") return "文書";
  if (k === "figure") return "図";
  return k || "断片";
}

/** U03: 内部検索の見出しの下の行（日付・方式・対象・件数・範囲・実行者・Node へのリンク）。 */
function lfindMetaLine(log) {
  const l = log || {};
  const bits = [];
  if (l.date) bits.push(String(l.date));
  const cond = [];
  const modeLabel = { word: "単語", vector: "意味", hybrid: "両方" }[String(l.mode || "")];
  if (modeLabel) cond.push(modeLabel);
  const scopeLabel = { papers: "論文", notes: "ノート", figures: "図", all: "全部" }[String(l.scope || "")];
  if (scopeLabel) cond.push(scopeLabel);
  if (l.top_k != null && l.top_k !== "") cond.push(`${l.top_k} 件`);
  if (cond.length > 0) bits.push(cond.join(" · "));
  if (l.range && l.range !== "library") bits.push(String(l.range));
  const actor = searchActorText(l);
  if (actor) bits.push(actor);
  const line = el("div", "paper-meta", bits.join(" · "));
  if (l.node && l.node.id) {
    line.appendChild(document.createTextNode(" · "));
    const a = el("a", "paper-doi", String(l.node.id));
    a.href = `#/research/tree/${l.node.id}`;
    line.appendChild(a);
  }
  return line;
}

/** U03: 内部検索の結果の 1 行。paper_id が無いものは押せない行にする。 */
function lfindHitRow(hit, projectName) {
  const h = hit || {};
  const hasPaper = h.paper_id != null && h.paper_id !== "";
  const row = hasPaper ? document.createElement("button") : document.createElement("div");
  row.className = "paper-row";
  const line1 = document.createElement("div");
  line1.className = "paper-line1";
  line1.appendChild(el("span", "paper-title", h.title || "(無題)"));
  line1.appendChild(el("span", "paper-tag", lfindKindLabel(h.source_kind)));
  row.appendChild(line1);
  const authors = Array.isArray(h.authors) ? h.authors.join(", ") : String(h.authors || "");
  const score = Number(h.score);
  const meta = [
    authorShort(authors),
    h.year != null && h.year !== "" ? String(h.year) : "",
    h.section ? String(h.section) : "",
    Number.isFinite(score) ? `スコア ${score.toFixed(2)}` : "",
  ].map((x) => String(x || "").trim()).filter(Boolean).join(" · ");
  if (meta) row.appendChild(el("div", "paper-meta", meta));
  const text = String(h.text || "").slice(0, 240);
  if (text) row.appendChild(el("div", "paper-row-abst", text));
  if (hasPaper) {
    row.addEventListener("click", () => {
      const side = paperSidePane("library-list", PAPER_SIDE_IDS.library);
      if (side) {
        showPaperInSide(side, projectName, h.paper_id);
        return;
      }
      location.hash = paperHash(projectName, h.paper_id, false);
    });
  }
  return row;
}

/** U03: 広い画面なら右の欄に内部検索の結果を出す。狭い画面は専用の画面へ。 */
async function lfindShowInSide(main, projectName, logId) {
  if (!main) {
    location.hash = findHash(projectName, logId);
    return;
  }
  sideKind = "find";
  sidePaperId = null;
  sideRemote = null;
  sideLog = null;
  sideFind = { logId, name: projectName };
  main.dataset.showing = `find:${logId}`;
  main.innerHTML = "";
  main.appendChild(el("p", "lib-empty", "読み込み中…"));
  let data;
  let error = "";
  try {
    data = await getFindLog(projectName, logId);
  } catch (e) {
    error = (e && e.message) || "読み込めませんでした";
  }
  if (sideKind !== "find" || !sideFind || sideFind.logId !== logId) return;
  main.innerHTML = "";
  if (error) {
    main.appendChild(el("p", "lib-empty", error));
    return;
  }
  const hits = Array.isArray(data.hits) ? data.hits : [];
  main.appendChild(el("h2", "papers-heading", `“${(data.log && data.log.query) || ""}” の結果 ${hits.length} 件`));
  main.appendChild(lfindMetaLine(data.log || {}));
  if (hits.length === 0) {
    main.appendChild(el("p", "lib-empty", "結果がありません"));
    return;
  }
  for (const hit of hits) main.appendChild(lfindHitRow(hit, projectName));
}

/** U03: 選んだ論文の札を描き直す。 */
function lfindRenderChips() {
  const box = document.getElementById("find-papers-chips");
  if (!box) return;
  box.innerHTML = "";
  const byId = new Map((libPapers || []).map((p) => [Number(p.id), p]));
  for (const id of libFindPapers) {
    const p = byId.get(Number(id));
    const chip = el("span", "find-chip", (p && p.title) || `#${id}`);
    const x = document.createElement("button");
    x.type = "button";
    x.className = "find-chip-x";
    x.textContent = "✕";
    x.setAttribute("aria-label", "外す");
    x.addEventListener("click", () => {
      libFindPapers = libFindPapers.filter((v) => Number(v) !== Number(id));
      syncCondUI();
    });
    chip.appendChild(x);
    box.appendChild(chip);
  }
}

/** U03: Node の欄の見た目（Each は Node か論文を選んだときだけ出す）。 */
function lfindRenderNodeUI() {
  const sel = document.getElementById("find-node");
  const note = document.getElementById("find-node-note");
  const subWrap = document.getElementById("find-subtree-wrap");
  const eachWrap = document.getElementById("find-each-wrap");
  const sub = document.getElementById("find-subtree");
  const each = document.getElementById("find-each");
  if (sel) {
    if (libFindNodes === null && !lfindLoadingNode && !libFindNodesMsg) {
      // 初めて開いたら一覧を読む（描き直しのたびに読まない）
      void lfindLoadNodes();
    }
    if (document.activeElement !== sel) {
      sel.innerHTML = "";
      const first = document.createElement("option");
      first.value = "";
      first.textContent = "（指定しない）";
      sel.appendChild(first);
      for (const n of libFindNodes || []) {
        const o = document.createElement("option");
        o.value = n.id;
        o.textContent = `${n.id} ${n.title || n.text || ""}`.trim();
        sel.appendChild(o);
      }
      sel.value = libFindNode || "";
    }
  }
  if (note) note.hidden = !(libFindNodes !== null && (libFindNodes || []).length === 0);
  if (sub && document.activeElement !== sub) sub.checked = !!libFindSubtree;
  if (each && document.activeElement !== each) each.value = libFindEach || "";
  const showEach = (libFindNode || "") !== "" || libFindPapers.length > 0;
  if (eachWrap) eachWrap.hidden = !showEach;
}

/** U03: research-tree の Node の一覧を読む（木が無ければ欄を隠す）。 */
async function lfindLoadNodes() {
  const name = selectedProject;
  if (!name || lfindLoadingNode) return;
  lfindLoadingNode = true;
  try {
    const res = await apiFetch(`${papersApiBase(name)}/sci/status`);
    const body = await res.json().catch(() => ({}));
    if (selectedProject !== name) return;
    if (!res.ok) throw new Error(body.detail || "");
    const nodes = body && Array.isArray(body.nodes) ? body.nodes : [];
    libFindNodes = nodes.filter((n) => n && n.id).map((n) => ({ id: String(n.id), title: n.title, text: n.text }));
    libFindNodesMsg = "";
  } catch {
    if (selectedProject !== name) return;
    libFindNodes = [];
    libFindNodesMsg = "research-tree がまだありません";
  } finally {
    lfindLoadingNode = false;
    lfindRenderNodeUI();
  }
}

/** U03: 論文を選ぶ小窓（Library の論文から 20 本まで・題名で絞り込める）。 */
function lfindOpenPaperSheet() {
  if (typeof saiSheet !== "function") return;
  const body = document.createElement("div");
  const search = document.createElement("input");
  search.type = "search";
  search.placeholder = "題名で絞り込み";
  search.className = "papers-sheet-filter";
  body.appendChild(search);
  const list = document.createElement("div");
  body.appendChild(list);
  const picked = new Set(libFindPapers.map((x) => Number(x)));
  const draw = () => {
    list.innerHTML = "";
    const q = (search.value || "").trim().toLowerCase();
    const rows = (libPapers || []).filter((p) => {
      if (!q) return true;
      return String(p.title || "").toLowerCase().includes(q);
    }).slice(0, 100);
    for (const p of rows) {
      const item = document.createElement("button");
      item.type = "button";
      item.className = "papers-sheet-item" + (picked.has(Number(p.id)) ? " active" : "");
      const check = document.createElement("span");
      check.className = "papers-sheet-check";
      check.textContent = picked.has(Number(p.id)) ? "✓" : "";
      item.appendChild(check);
      const label = document.createElement("span");
      label.textContent = `${p.title || "(無題)"}${p.year ? ` (${p.year})` : ""}`;
      item.appendChild(label);
      item.addEventListener("click", () => {
        const id = Number(p.id);
        if (picked.has(id)) picked.delete(id);
        else {
          if (picked.size >= 20) return;
          picked.add(id);
        }
        libFindPapers = [...picked];
        draw();
        syncCondUI();
      });
    }
    if (rows.length === 0) {
      const note = document.createElement("p");
      note.className = "papers-sheet-note";
      note.textContent = "一致する論文がありません";
      list.appendChild(note);
    }
  };
  search.addEventListener("input", draw);
  draw();
  saiSheet({ title: "論文を選ぶ（20 本まで）", body });
}

// --- 狭い画面の内部検索の結果（#/papers/find/<name>/<log_id>） ---

async function lfindOpen(params) {
  const hash = location.hash;
  const name = await projectForAddress(params.name);
  if (location.hash !== hash) return;
  const id = parseInt(params.id, 10);
  savePapersList(findHash(name, id));
  findView = {
    id,
    name,
    data: name ? findLogCache.get(logCacheKey(name, id)) || null : null,
    error: "",
  };
  if (!name) {
    findView.data = null;
    findView.error = NO_PROJECT_MSG;
    renderFindLog();
    return;
  }
  renderFindLog();
  if (findView.data) return;
  try {
    const data = await getFindLog(name, id);
    if (location.hash !== hash) return;
    findView.data = data;
  } catch (e) {
    if (location.hash !== hash) return;
    findView.data = null;
    findView.error = (e && e.message) || "読み込めませんでした";
    renderFindLog();
    window.SaiShell.fail();
    return;
  }
  renderFindLog();
  window.SaiShell.ok();
}

function renderFindLog() {
  const title = document.getElementById("find-title");
  const sub = document.getElementById("find-sub");
  const list = document.getElementById("find-list");
  if (!list) return;
  if (title) title.textContent = (findView && findView.data && findView.data.log.query) || "Library 内の検索結果";
  if (sub) sub.textContent = "";
  list.innerHTML = "";
  if (!findView) return;
  if (!findView.data) {
    list.appendChild(el("p", "lib-empty", findView.error || "読み込み中…"));
    return;
  }
  const log = findView.data.log || {};
  const hits = Array.isArray(findView.data.hits) ? findView.data.hits : [];
  if (sub) {
    const bits = [];
    if (log.date) bits.push(String(log.date));
    const cond = lfindSummary({ mode: log.mode, scope: log.scope, count: log.top_k, node: log.node && log.node.id });
    if (cond) bits.push(cond);
    const actor = searchActorText(log);
    if (actor) bits.push(actor);
    bits.push(`${hits.length} 件`);
    sub.textContent = bits.join(" · ");
  }
  if (hits.length === 0) {
    list.appendChild(el("p", "lib-empty", "結果がありません"));
    return;
  }
  for (const hit of hits) list.appendChild(lfindHitRow(hit, findView.name));
}

/** 結果の 1 行（Library の右の欄と過去の検索結果で共通・08 §4.3）。 */
function resultRow(item, projectName, logId) {
  const local = item.local_paper_id != null && item.local_paper_id !== "";
  const btn = document.createElement("button");
  btn.className = "paper-row";
  const line1 = document.createElement("div");
  line1.className = "paper-line1";
  line1.appendChild(el("span", "paper-title", item.title || "(無題)"));
  if (local) line1.appendChild(el("span", "paper-tag", "✓ Library"));
  btn.appendChild(line1);
  const authors = Array.isArray(item.authors)
    ? item.authors.join(", ")
    : String(item.authors || "");
  const cited =
    item.citation_count != null && item.citation_count !== ""
      ? `被引用 ${item.citation_count}`
      : "";
  const meta = [authorShort(authors), item.year, item.journal, cited]
    .map((x) => String(x || "").trim())
    .filter(Boolean)
    .join(" · ");
  if (meta) btn.appendChild(el("div", "paper-meta", meta));
  // 後から Download/Import を選べるよう、DOI と要旨の頭も一覧に出す（押すと開く行なので DOI は文字だけ）
  if (item.doi) btn.appendChild(el("div", "paper-meta paper-row-doi", `DOI ${item.doi}`));
  if (item.abstract) btn.appendChild(el("div", "paper-row-abst", item.abstract));
  btn.addEventListener("click", () => {
    // 広い画面は右の欄に出し、狭い画面は今まで通り論文情報の画面へ（U6）
    const side = paperSidePane("library-list", PAPER_SIDE_IDS.library);
    if (side) {
      if (local) showPaperInSide(side, projectName, item.local_paper_id);
      else void showRemoteInSide(side, projectName, logId, item.position);
      return;
    }
    // Library にあればその論文情報へ，無ければ未ダウンロードの情報へ（08 §4.3・住所は 18 §3.5）
    location.hash = local
      ? paperHash(projectName, item.local_paper_id, false)
      : logHash(projectName, logId, item.position);
  });
  return btn;
}

// --- 過去の検索結果（#/papers/log/<name>/<log_id>・08 §4.2・18 §3.5） ---

async function openLog(params) {
  const hash = location.hash; // 開いたときの住所（読み込み中に移動したら捨てる・08 §4.2）
  const name = await projectForAddress(params.name); // 住所に名前があれば切り替える（18 §3.5）
  if (location.hash !== hash) return;
  const id = parseInt(params.id, 10);
  // 論文情報の戻る先として今の住所を覚える（08 §4）
  savePapersList(logHash(name, id, null));
  logView = {
    id,
    name,
    data: name ? logCache.get(logCacheKey(name, id)) || null : null,
    error: "",
  };
  if (!name) {
    // 置き場があるプロジェクトが1つも無い（18 §1.1）
    logView.data = null;
    logView.error = NO_PROJECT_MSG;
    renderLog();
    return;
  }
  renderLog();
  if (logView.data) return;
  try {
    const data = await getLog(name, id);
    if (location.hash !== hash) return;
    logView.data = data;
  } catch (e) {
    if (location.hash !== hash) return;
    logView.data = null;
    logView.error = (e && e.message) || "読み込めませんでした";
    renderLog();
    window.SaiShell.fail();
    return;
  }
  renderLog();
  window.SaiShell.ok();
}

function renderLog() {
  const title = document.getElementById("log-title");
  const sub = document.getElementById("log-sub");
  const list = document.getElementById("log-list");
  if (!list) return;
  if (title) title.textContent = (logView && logView.data && logView.data.log.query) || "Search results";
  if (sub) sub.textContent = "";
  list.innerHTML = "";
  if (!logView) return;
  if (!logView.data) {
    list.appendChild(el("p", "lib-empty", logView.error || "読み込み中…"));
    return;
  }
  const log = logView.data.log || {};
  const items = Array.isArray(logView.data.items) ? logView.data.items : [];
  if (sub) {
    sub.textContent = [log.date, log.source_dbs, searchActorText(log), `${items.length} 件`]
      .map((x) => String(x || "").trim())
      .filter(Boolean)
      .join(" · ");
  }
  if (items.length === 0) {
    list.appendChild(el("p", "lib-empty", "結果がありません"));
    return;
  }
  for (const item of items) list.appendChild(resultRow(item, logView.name, logView.id));
}

// --- 未ダウンロードの論文情報（#/papers/log/<name>/<log_id>/<position>・08 §4.4・18 §3.5） ---

async function openRemote(params) {
  const hash = location.hash; // 開いたときの住所（読み込み中に移動したら捨てる・08 §4.4）
  const name = await projectForAddress(params.name); // 住所に名前があれば切り替える（18 §3.5）
  if (location.hash !== hash) return;
  const logId = parseInt(params.id, 10);
  const position = parseInt(params.position, 10);
  remoteView = {
    logId,
    name,
    position,
    data: name ? logCache.get(logCacheKey(name, logId)) || null : null,
    error: "",
  };
  if (!name) {
    remoteView.data = null;
    remoteView.error = NO_PROJECT_MSG; // 置き場があるプロジェクトが1つも無い（18 §1.1）
    renderRemote();
    return;
  }
  renderRemote();
  if (remoteView.data) return;
  try {
    const data = await getLog(name, logId);
    if (location.hash !== hash) return;
    remoteView.data = data;
  } catch (e) {
    if (location.hash !== hash) return;
    remoteView.error = (e && e.message) || "読み込めませんでした";
    renderRemote();
    window.SaiShell.fail();
    return;
  }
  renderRemote();
}

function renderRemote(target) {
  const body = target || document.getElementById("remote-body");
  if (!body) return;
  body.innerHTML = "";
  if (!remoteView) return;
  // 右の欄に出しているときは、検索結果の一覧へ戻る釦を付ける（狭い画面は上の ‹ が戻る）
  if (target) {
    body.appendChild(paperBtn("‹ 検索結果", () => {
      void showLogInSide(target, remoteView.name, remoteView.logId);
    }));
  }
  if (!remoteView.data) {
    body.appendChild(el("p", "lib-empty", remoteView.error || "読み込み中…"));
    return;
  }
  const items = Array.isArray(remoteView.data.items) ? remoteView.data.items : [];
  const item = items.find((x) => Number(x.position) === remoteView.position);
  if (!item) {
    body.appendChild(el("p", "lib-empty", "この論文が見つかりませんでした"));
    return;
  }
  // 1. 題名・著者・年・掲載誌・被引用数・DOI（08 §4.4）
  const head = document.createElement("div");
  head.className = "paper-head";
  head.appendChild(el("h2", "paper-title", item.title || "(無題)"));
  const authors = Array.isArray(item.authors)
    ? item.authors.join(", ")
    : String(item.authors || "");
  if (authors) head.appendChild(el("div", "paper-authors", authors));
  const meta = [];
  if (item.year) meta.push(String(item.year));
  if (item.journal) meta.push(String(item.journal));
  if (item.citation_count != null && item.citation_count !== "") {
    meta.push(`被引用 ${item.citation_count}`);
  }
  if (meta.length) head.appendChild(el("div", "paper-metaline", meta.join(" · ")));
  if (item.doi) {
    const a = el("a", "paper-doi", "DOI");
    a.href = doiUrl(item.doi);
    a.target = "_blank";
    a.rel = "noopener";
    head.appendChild(a);
  }
  body.appendChild(head);

  // 2. 要旨（英語だけ・無ければ「要旨がありません」・08 §4.4）
  const absCard = document.createElement("section");
  absCard.className = "dash-card paper-card";
  absCard.appendChild(el("h3", "dash-card-title", "要旨（English）"));
  absCard.appendChild(el("div", "paper-abstract", item.abstract || "要旨がありません"));
  // 訳は Library に入れた論文にしか作れない（`phone translate <id>` は Library だけ）
  absCard.appendChild(
    el("p", "paper-msg", "日本語訳は Library に入れたあとで作れます")
  );
  body.appendChild(absCard);

  // 3. Library に入れる（Library なら開くボタン・無ければ「＋ Library に入れる」・U13 §3）
  const guide = document.createElement("section");
  guide.className = "dash-card";
  const local = item.local_paper_id != null && item.local_paper_id !== "";
  if (local) {
    guide.appendChild(
      paperBtn("Library の論文情報を開く", () => {
        location.hash = paperHash(remoteView.name, item.local_paper_id, false);
      })
    );
    guide.appendChild(el("p", "paper-msg", "この論文は Library に入っています。"));
  } else {
    const actions = el("div", "paper-actions");
    actions.appendChild(
      paperBtn("＋ Library に入れる", () => {
        void pimStartLogKeep(remoteView.name, remoteView.logId, remoteView.position, item.title || "Papers");
      }, "primary")
    );
    guide.appendChild(actions);
    guide.appendChild(
      el("p", "paper-msg", "PDF 無しで Library に入れます。PDF は sai-prep から送ると、図・節・翻訳が入ります。")
    );
  }
  body.appendChild(guide);
}

// --- 論文情報（#/paper/<name>/<id>・07 §4.4・18 §3.5） ---

async function openPaper(params) {
  const hash = location.hash; // 開いたときの住所（読み込み中に移動したら捨てる・07 §4.4）
  // 住所に名前があれば「選んでいるプロジェクト」を切り替える（18 §3.5）
  const name = await projectForAddress(params.name);
  if (location.hash !== hash) return;
  paperProject = name;
  paperIdNum = parseInt(params.id, 10);
  paperTarget = null; // 論文情報の画面に描く（広い画面の欄の続きではない）
  paperData = null;
  paperError = "";
  abstractMsg = "";
  translating = false;
  refsAll = false;
  refsOpen = new Set();
  paperChunks = null;
  noteEditing = false;
  noteDraft = "";
  noteSaving = false;
  noteMsg = "";
  abstractLang = ""; // 読み込んでから（訳の有無で）決める
  // 音声作成中だけ定期更新（既定は更新しない・07 §4.2）
  paperRoute.interval = 0;
  const heading = document.getElementById("paper-heading");
  if (heading) heading.textContent = "";
  const body = document.getElementById("paper-body");
  if (body) {
    body.innerHTML = "";
    if (!paperProject) {
      // 置き場があるプロジェクトが1つも無い（18 §1.1）
      body.appendChild(el("p", "lib-empty", NO_PROJECT_MSG));
      return;
    }
    body.appendChild(el("p", "lib-empty", "読み込み中…"));
  }
  if (!paperProject) return;
  await loadPaper(paperIdNum);
}

async function loadPaper(id, target) {
  const hash = location.hash;
  const name = paperProject;
  if (!name) return;
  let data;
  try {
    const res = await apiFetch(`${papersApiBase(name)}/${encodeURIComponent(id)}`);
    const body = await res.json().catch(() => ({}));
    if (!res.ok) throw new Error(body.detail || "読み込めませんでした");
    data = body;
  } catch (e) {
    if (location.hash !== hash) return;
    paperError = (e && e.message) || "読み込めませんでした";
    paperData = null;
    window.SaiShell.fail();
    renderPaper(target);
    return;
  }
  if (location.hash !== hash) return;
  paperData = data;
  paperError = "";
  if (!abstractLang) {
    const p = data.paper || {};
    abstractLang = p.abstract_ja ? "ja" : "en";
  }
  // 音声作成中だけ 10 秒ごとに読み直す（07 §4.2）
  const generating = !!(data.podcast && data.podcast.status === "generating");
  paperRoute.interval = generating ? 10 : 0;
  // 間隔の決め直しは SaiShell.ok() がやる（baseInterval が def.interval を読む）
  window.SaiShell.ok();
  renderPaper(target);
}

/** 定期更新（音声作成中のときだけ呼ばれる・07 §4.2）。 */
async function pollPaper() {
  // ノートを書いている間は描き直さない（入力中の欄が作り直されてしまうため）
  if (paperIdNum == null || noteEditing) return;
  await loadPaper(paperIdNum);
}

function doiUrl(doi) {
  const s = String(doi || "").trim();
  if (!s) return "";
  return /^https?:\/\//i.test(s) ? s : `https://doi.org/${s}`;
}

/** sai-prep の入れ物（wheel）の住所（合言葉は ?token= で付ける・U13 §4）。 */
function prepDownloadUrl() {
  const q = (typeof apiToken !== "undefined" && apiToken)
    ? `?token=${encodeURIComponent(apiToken)}`
    : "";
  return `/api/prep/download${q}`;
}

/**
 * 論文からカードを作るときの task_text に入れる 1 行（U10）。
 * 題名・年・DOI はある物だけ書く。実行ファイルの名前は書かない
 * （名前は最後のガイドで変わるため・エージェントは番号から読み方を知っている）。
 */
function paperCardText(p, id) {
  const paper = p || {};
  const parts = [];
  if (paper.year) parts.push(String(paper.year));
  if (paper.doi) parts.push(`DOI ${paper.doi}`);
  const tail = parts.length ? `（${parts.join("・")}）` : "";
  return `対象の論文: #${id}「${paper.title || "(無題)"}」${tail}`;
}

function renderPaper(target) {
  // target を省略したとき（翻訳・要旨の言語の切り替え・ノートなど）は
  // 直前に描いた場所を使う（広い画面の欄でも動く）。
  // ここで paperTarget を先に消すと、広い画面の欄で描いた論文の描き直しが
  // 隠れている論文情報の画面に落ちて「切り替わらない」ように見える（07 §4.4）。
  const body = target || paperTarget || document.getElementById("paper-body");
  if (!body) return;
  paperTarget = target || paperTarget || null;
  body.innerHTML = "";
  if (!paperData) {
    body.appendChild(el("p", "lib-empty", paperError || "読み込み中…"));
    return;
  }
  const p = paperData.paper || {};
  const files = paperData.files || {};

  // 1. 題名・著者・メタ・DOI
  const head = document.createElement("div");
  head.className = "paper-head";
  head.appendChild(el("h2", "paper-title", p.title || "(無題)"));
  if (p.authors) head.appendChild(el("div", "paper-authors", p.authors));
  const metaParts = [];
  if (p.year) metaParts.push(String(p.year));
  if (p.journal) metaParts.push(String(p.journal));
  if (p.citation_count != null && p.citation_count !== "") {
    metaParts.push(`被引用 ${Number(p.citation_count)}`);
  }
  if (metaParts.length) head.appendChild(el("div", "paper-metaline", metaParts.join(" · ")));
  if (p.doi) {
    const a = el("a", "paper-doi", "DOI");
    a.href = doiUrl(p.doi);
    a.target = "_blank";
    a.rel = "noopener";
    head.appendChild(a);
  }
  body.appendChild(head);

  // 2. ボタンの行（角丸の小さなボタン）
  const actions = document.createElement("div");
  actions.className = "paper-actions";
  if (files.pdf) {
    actions.appendChild(paperBtn("PDF", () => openPdf(paperIdNum, false)));
  }
  if (files.pdf_ja) {
    actions.appendChild(paperBtn("日本語 PDF", () => openPdf(paperIdNum, true)));
  }
  if (files.extract) {
    actions.appendChild(
      paperBtn("Text", () => { location.hash = paperHash(paperProject, paperIdNum, true); })
    );
  }
  // この論文を書いたカードを作る（作成フォームを開き task_text に論文の行を入れる・U10）
  if (typeof cfOpen === "function" && paperProject) {
    actions.appendChild(
      paperBtn("カードを作る", () => {
        void cfOpen({ project: paperProject, taskText: paperCardText(p, paperIdNum) });
      })
    );
  }
  // PDF 全体を日本語に訳す（pdf_ja があるときは作り直し）
  if (files.pdf) {
    actions.appendChild(
      paperBtn(files.pdf_ja ? "日本語 PDF を作り直す" : "日本語 PDF を作る", () => { void onTranslatePdf(); })
    );
  }
  if (actions.childElementCount) body.appendChild(actions);
  // ボタンの下に「何が起きるか」を 1 行ずつ出す（U6・端末に慣れていない人向け）
  const hints = [];
  if (files.pdf) hints.push("📄 PDF: 新しいタブで、取った PDF がそのまま見えます");
  if (files.pdf_ja) hints.push("📄 日本語 PDF: PDF を日本語に訳したものです");
  if (files.extract) hints.push("本文: 取り出した本文だけを、1 ページで読みます");
  if (files.pdf) hints.push("🈂 日本語 PDF を作る: PDF 全体を日本語に訳します。数分かかります。");
  hints.push("🎙 音声を作る: 数分かかります。できたら通知でお知らせします");
  hints.push("カードを作る: この論文の行を入れた task.md の作成フォームを開きます");
  hints.push("ノート: この論文の覚え書きです。書くとここに残ります");
  if (!files.pdf) hints.push("PDF 待ち: sai-prep から送ると PDF・図・翻訳が入ります");
  for (const text of hints) body.appendChild(el("p", "paper-hint", text));

  // PDF が無い論文は「PDF 待ち」と sai-prep の入れ方を出す（U13 §3）
  if (!files.pdf) {
    const wait = document.createElement("section");
    wait.className = "dash-card paper-card";
    wait.appendChild(el("h3", "dash-card-title", "PDF 待ち"));
    wait.appendChild(
      el("p", "paper-msg", "PDF 待ち — 各自の Mac の sai-prep から送ってください。")
    );
    const a = el("a", "paper-doi", "sai-prep の入れ物を落とす");
    a.href = prepDownloadUrl();
    a.target = "_blank";
    a.rel = "noopener";
    wait.appendChild(a);
    wait.appendChild(
      el("p", "paper-hint", "入れ方: 落としたファイルを `uv tool install <落としたファイル>` で入れます。")
    );
    body.appendChild(wait);
  }

  // 3. 音声の置き場（09・10 が中身を入れる・07 §4.4-3）
  const slot = document.createElement("div");
  slot.id = "paper-audio-slot";
  body.appendChild(slot);
  for (const fn of window.SaiPaperHooks || []) {
    if (typeof fn !== "function") continue;
    try {
      fn(paperData, slot);
    } catch { /* 壊れたフックで画面を止めない */ }
  }

  // 4. 要旨・5. ノート・6. 図（Mac の PAPER 画面の FIGURES と同じ）・7. 引用文献
  body.appendChild(abstractCard());
  body.appendChild(noteCard());
  const figs = figuresCard();
  if (figs) body.appendChild(figs);
  body.appendChild(refsCard());
  body.appendChild(chunksCard());
}

function paperBtn(label, onClick, cls) {
  const b = document.createElement("button");
  b.className = cls ? `paper-btn ${cls}` : "paper-btn";
  b.textContent = label;
  b.addEventListener("click", onClick);
  return b;
}

/** PDF を新しいタブで開く（合言葉は ?token= で付ける・require_token がクエリも受ける）。 */
function openPdf(id, ja) {
  const params = new URLSearchParams();
  if (ja) params.set("lang", "ja");
  if (typeof apiToken !== "undefined" && apiToken) params.set("token", apiToken);
  const q = params.toString();
  const base = papersApiBase(paperProject); // 開いている論文のプロジェクト（18 §3.4）
  window.open(`${base}/${encodeURIComponent(id)}/pdf${q ? `?${q}` : ""}`, "_blank");
}

// 4. 要旨カード（日本語 | English・訳が無ければ作るボタン）
function abstractCard() {
  const p = (paperData && paperData.paper) || {};
  const card = document.createElement("section");
  card.className = "dash-card paper-card";

  const head = document.createElement("div");
  head.className = "dash-card-head";
  head.appendChild(el("h3", "dash-card-title", "要旨"));
  const seg = document.createElement("div");
  seg.className = "dash-seg small";
  for (const [value, label] of [["ja", "日本語"], ["en", "English"]]) {
    const b = document.createElement("button");
    b.textContent = label;
    if (value === abstractLang) b.className = "active";
    b.addEventListener("click", () => {
      if (abstractLang === value) return;
      abstractLang = value;
      abstractMsg = "";
      renderPaper();
    });
    seg.appendChild(b);
  }
  head.appendChild(seg);
  card.appendChild(head);

  const text = document.createElement("div");
  text.className = "paper-abstract";
  if (!p.abstract && !p.abstract_ja) {
    text.textContent = "要旨がありません";
  } else if (abstractLang === "ja") {
    if (p.abstract_ja) {
      text.textContent = p.abstract_ja;
    } else {
      // 日本語訳が無い → 作るボタン（03 が未 merge なら失敗を表示するだけ・07 §2）
      const b = document.createElement("button");
      b.className = "paper-btn";
      b.textContent = translating
        ? "翻訳中…（数十秒かかることがあります）"
        : "日本語訳を作る";
      b.disabled = translating;
      b.addEventListener("click", onTranslate);
      text.appendChild(b);
      const hint = el("p", "paper-hint", "日本語訳を作る: AI が要約を日本語に訳します。数十秒かかることがあります");
      text.appendChild(hint);
    }
  } else {
    text.textContent = p.abstract || "要旨がありません";
  }
  card.appendChild(text);
  if (abstractMsg) card.appendChild(el("div", "paper-msg", abstractMsg));
  return card;
}

/** PDF 全体の翻訳を始める（202 なら仕事は translate・pdf:<id> として進み具合に出る）。 */
async function onTranslatePdf() {
  if (paperIdNum == null || !paperProject) return;
  let res;
  try {
    res = await apiFetch(`${papersApiBase(paperProject)}/${paperIdNum}/translate-pdf`, { method: "POST" });
  } catch {
    saiToast("接続できていません", "error");
    return;
  }
  const body = await res.json().catch(() => ({}));
  if (!res.ok) {
    saiToast(body.detail || "PDF の翻訳を始められませんでした", "error");
    return;
  }
  saiToast("PDF の翻訳を始めました。数分かかります。終わったら通知でお知らせします", "ok");
}

async function onTranslate() {
  if (translating || paperIdNum == null || !paperProject) return;
  translating = true;
  abstractMsg = "";
  renderPaper();
  let res;
  try {
    res = await apiFetch(`${papersApiBase(paperProject)}/${paperIdNum}/translate`, { method: "POST" });
  } catch {
    translating = false;
    abstractMsg = "接続できていません";
    renderPaper();
    window.SaiShell.fail();
    return;
  }
  const body = await res.json().catch(() => ({}));
  translating = false;
  if (!res.ok) {
    // CLI の失敗は detail をそのままカードに出す（00 §9・帯は出さない）
    abstractMsg = body.detail || "日本語訳を作れませんでした";
    renderPaper();
    return;
  }
  if (paperData && paperData.paper) paperData.paper.abstract_ja = body.abstract_ja || "";
  abstractLang = "ja";
  abstractMsg = "";
  window.SaiShell.ok();
  renderPaper();
}

// 5. ノートカード（読む・その場で書き換えて保存する）
function noteCard() {
  const card = document.createElement("section");
  card.className = "dash-card paper-card";
  const head = el("div", "dash-card-head");
  head.appendChild(el("h3", "dash-card-title", "Notes"));
  const note = paperData && paperData.note;
  if (!noteEditing) {
    head.appendChild(paperBtn(note ? "Edit" : "書く", () => {
      noteEditing = true;
      noteDraft = note || "";
      noteMsg = "";
      renderPaper();
    }));
  }
  card.appendChild(head);

  if (noteEditing) {
    const ta = document.createElement("textarea");
    ta.className = "md-edit";
    ta.value = noteDraft;
    ta.placeholder = "この論文のノート（Markdown）";
    ta.disabled = noteSaving;
    ta.addEventListener("input", () => { noteDraft = ta.value; });
    card.appendChild(ta);
    const row = el("div", "paper-actions");
    const save = paperBtn(noteSaving ? "保存中…" : "Save", () => { void onSaveNote(); });
    save.disabled = noteSaving;
    const cancel = paperBtn("Cancel", () => {
      noteEditing = false;
      noteMsg = "";
      renderPaper();
    });
    cancel.disabled = noteSaving;
    row.append(save, cancel);
    card.appendChild(row);
  } else if (note) {
    const div = document.createElement("div");
    div.className = "paper-note";
    div.innerHTML = renderMarkdown(note);
    card.appendChild(div);
  } else {
    card.appendChild(el("p", "lib-empty note-none", "ノートはありません"));
  }
  if (noteMsg) card.appendChild(el("div", "paper-msg", noteMsg));
  return card;
}

async function onSaveNote() {
  if (noteSaving || paperIdNum == null || !paperProject) return;
  noteSaving = true;
  noteMsg = "";
  renderPaper();
  try {
    const res = await apiFetch(`${papersApiBase(paperProject)}/${paperIdNum}/note`, {
      method: "PUT",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ text: noteDraft }),
    });
    const body = await res.json().catch(() => ({}));
    if (!res.ok) throw new Error(body.detail || "保存できませんでした");
    if (paperData) paperData.note = body.note || null;
    noteEditing = false;
    window.SaiShell.ok();
  } catch (e) {
    // 書きかけは消さずに残す（もう一度押せる）
    noteMsg = (e && e.message) || "保存できませんでした";
  }
  noteSaving = false;
  renderPaper();
}

/** 図の画像の住所（<img> はヘッダを付けられないので合言葉は ?token= で付ける）。 */
function figureUrl(figNo) {
  const q = (typeof apiToken !== "undefined" && apiToken) ? `?token=${encodeURIComponent(apiToken)}` : "";
  return `${papersApiBase(paperProject)}/${encodeURIComponent(paperIdNum)}/figures/${encodeURIComponent(figNo)}${q}`;
}

// 6. 図カード（見出し → 画像 → キャプション → 本文中の言及。Mac の PAPER 画面と同じ並び）
// 抽出済みで 0 件なら出さない（Mac と同じ）。押すと画像だけを新しいタブで大きく開く。
function figuresCard() {
  const figs = (paperData && paperData.figures) || {};
  if (figs.state === "empty") return null;
  const items = Array.isArray(figs.items) ? figs.items : [];
  const card = document.createElement("section");
  card.className = "dash-card paper-card";
  card.appendChild(el("h3", "dash-card-title", items.length ? `図（${items.length}）` : "図"));
  if (items.length === 0) {
    card.appendChild(el("p", "lib-empty", "図はまだ取り出されていません（Mac の sai-paper で取り出せます）"));
    return card;
  }
  for (const f of items) {
    const fig = el("figure", "paper-fig");
    const label = f.label || `Figure ${f.fig_no}`;
    fig.appendChild(el("div", "paper-fig-head", f.title ? `${label}. ${f.title}` : `${label}.`));
    const img = document.createElement("img");
    img.loading = "lazy";
    img.alt = label;
    img.src = figureUrl(f.fig_no);
    img.addEventListener("click", () => window.open(img.src, "_blank"));
    fig.appendChild(img);
    if (f.caption) fig.appendChild(el("figcaption", "paper-fig-caption", f.caption));
    if (f.mention) fig.appendChild(el("div", "paper-fig-mention", `本文: "${f.mention}"`));
    card.appendChild(fig);
  }
  return card;
}

// 6. 引用文献カード（最初の 10 件＋「すべて表示」・07 §4.4-6・U05 §4）
function refsCard() {
  const refs = (paperData && paperData.refs) || [];
  const card = document.createElement("section");
  card.className = "dash-card paper-card";
  const head = el("div", "dash-card-head");
  head.appendChild(el("h3", "dash-card-title", `引用文献（${refs.length}）`));
  // 引用一覧の画面（#/paper/<name>/<id>/cites）への入口（U05 §4）。
  // ここに出ているのはBibTeX で入った引用だけなので、全部は引用一覧の画面（cite.js）にある。
  if (paperProject && paperIdNum != null && typeof ctHash === "function") {
    head.appendChild(paperBtn("引用一覧を開く ›", () => {
      location.hash = ctHash(paperProject, paperIdNum);
    }));
  }
  card.appendChild(head);
  if (refs.length === 0) {
    card.appendChild(el("p", "lib-empty", "引用文献は取り出されていません"));
    return card;
  }
  const shown = refsAll ? refs : refs.slice(0, REFS_INITIAL);
  shown.forEach((r, i) => card.appendChild(refRow(r, i)));
  if (!refsAll && refs.length > REFS_INITIAL) {
    const more = document.createElement("button");
    more.className = "ref-more";
    more.textContent = "すべて表示";
    more.addEventListener("click", () => {
      refsAll = true;
      renderPaper();
    });
    card.appendChild(more);
  }
  return card;
}

// 8. チャンク（論文の本文を `#論文.章.段落` ごとに並べる。重いので開いたときに読む）

/** チャンクを章（section）ごとにまとめる。並びは受け取った順のまま。 */
function chunkGroups(chunks) {
  const groups = [];
  const index = new Map();
  for (const c of Array.isArray(chunks) ? chunks : []) {
    if (!c) continue;
    const key = String(c.section || "");
    let g = index.get(key);
    if (!g) {
      g = { section: key, heading: String(c.heading || ""), items: [] };
      index.set(key, g);
      groups.push(g);
    }
    g.items.push(c);
  }
  return groups;
}

/** チャンクを今の論文について読む（別の論文・プロジェクトの結果は捨てる）。 */
async function loadPaperChunks() {
  const name = paperProject;
  const id = paperIdNum;
  if (!name || id == null) return;
  const key = `${name}:${id}`;
  const st = { key, state: "loading", items: [], error: "", open: true };
  paperChunks = st;
  renderPaper();
  try {
    const body = await paperApi(`${papersApiBase(name)}/${encodeURIComponent(id)}/chunks`);
    st.items = Array.isArray(body && body.chunks) ? body.chunks : [];
    st.state = "ok";
  } catch (e) {
    st.state = e && e.status === 404 ? "missing" : "error";
    st.error = (e && e.message) || "読み込めませんでした";
  }
  if (paperChunks === st) renderPaper();
}

/** ID を clipboard へ写す小さな釦。 */
function chunkIdButton(id) {
  const b = el("button", "paper-chunk-id", id);
  b.type = "button";
  b.title = "ID を写す";
  b.addEventListener("click", () => {
    const done = () => { if (typeof saiToast === "function") saiToast(`${id} を写しました`, "ok"); };
    if (navigator.clipboard && navigator.clipboard.writeText) navigator.clipboard.writeText(id).then(done, () => {});
  });
  return b;
}

/** チャンクの一覧の中身（読み込み前後の状態に合わせる）。 */
function chunksBody(st) {
  const box = el("div", "paper-chunks-body");
  if (st.state === "loading") {
    box.appendChild(el("p", "lib-empty", "読み込み中…"));
  } else if (st.state === "missing") {
    box.appendChild(el("p", "lib-empty", "本文がまだ取り出されていません"));
  } else if (st.state === "error") {
    box.appendChild(el("p", "lib-empty", st.error));
  } else if (st.items.length === 0) {
    box.appendChild(el("p", "lib-empty", "チャンクがありません"));
  } else {
    for (const g of chunkGroups(st.items)) {
      box.appendChild(el("h4", "paper-chunk-sec", `${g.section} ${g.heading}`.trim()));
      for (const c of g.items) {
        const row = el("div", "paper-chunk");
        row.id = `chunk-${String(c.id || "").replace(/^#/, "")}`;
        row.appendChild(chunkIdButton(String(c.id || "")));
        row.appendChild(el("div", "paper-chunk-text", String(c.text || "")));
        box.appendChild(row);
      }
    }
  }
  return box;
}

function chunksCard() {
  const key = `${paperProject}:${paperIdNum}`;
  const st = paperChunks && paperChunks.key === key ? paperChunks : null;
  const det = document.createElement("details");
  det.className = "dash-card paper-card paper-chunks";
  const count = st && st.state === "ok" ? `（${st.items.length}）` : "";
  det.appendChild(el("summary", "dash-card-title", `チャンク${count}`));
  if (st) {
    det.open = st.open;
    det.appendChild(chunksBody(st));
  }
  // 閉じた状態で始め、初めて開いたときに読む（本文が長く重いため）
  det.addEventListener("toggle", () => {
    if (paperChunks && paperChunks.key === key) paperChunks.open = det.open;
    else if (det.open) void loadPaperChunks();
  });
  return det;
}

function refRow(r, i) {
  const wrap = document.createElement("div");
  wrap.className = "ref-row";
  const local = r.local_paper_id != null && r.local_paper_id !== "";

  const btn = document.createElement("button");
  btn.className = "ref-main";
  const lines = document.createElement("div");
  lines.className = "ref-lines";
  lines.appendChild(el("div", "ref-line1", `[${i + 1}] ${r.title || "(無題)"}`));
  const sub = [r.authors, r.year, r.journal]
    .map((x) => String(x || "").trim())
    .filter(Boolean)
    .join(" · ");
  if (sub) lines.appendChild(el("div", "ref-sub", sub));
  btn.appendChild(lines);

  if (local) {
    // Library にある引用先へ移る
    btn.appendChild(el("span", "ref-chev", "›"));
    btn.addEventListener("click", () => {
      location.hash = paperHash(paperProject, r.local_paper_id, false);
    });
  } else {
    // 無い行はその場で開く（DOI のリンクを出す）
    btn.addEventListener("click", () => {
      if (refsOpen.has(i)) refsOpen.delete(i);
      else refsOpen.add(i);
      renderPaper();
    });
  }
  wrap.appendChild(btn);

  if (!local && refsOpen.has(i)) {
    const det = document.createElement("div");
    det.className = "ref-detail";
    if (r.doi) {
      const a = el("a", "paper-doi", "DOI を開く");
      a.href = doiUrl(r.doi);
      a.target = "_blank";
      a.rel = "noopener";
      det.appendChild(a);
    } else {
      det.textContent = "この論文は Library にありません";
    }
    wrap.appendChild(det);
  }
  return wrap;
}

// --- 本文（#/paper/<name>/<id>/text・07 §4.5・18 §3.5） ---

async function openPaperText(params) {
  const hash = location.hash; // 開いたときの住所（読み込み中に移動したら捨てる・07 §4.5）
  // 戻る先（その論文の情報）を覚える（深いリンクで直接開いた場合も動くように）
  paperProject = await projectForAddress(params.name); // 住所に名前があれば切り替える（18 §3.5）
  if (location.hash !== hash) return;
  paperIdNum = parseInt(params.id, 10);
  paperTarget = null; // 本文はいつも論文情報の画面（広い画面の欄ではない）
  const box = document.getElementById("paper-text");
  if (!box) return;
  box.innerHTML = "";
  if (!paperProject) {
    box.appendChild(el("p", "lib-empty", NO_PROJECT_MSG)); // 置き場が1つも無い（18 §1.1）
    return;
  }
  box.appendChild(el("p", "lib-empty", "読み込み中…"));
  let data;
  try {
    const res = await apiFetch(`${papersApiBase(paperProject)}/${encodeURIComponent(paperIdNum)}/text`);
    const body = await res.json().catch(() => ({}));
    if (!res.ok) throw new Error(body.detail || "読み込めませんでした");
    data = body;
  } catch (e) {
    if (location.hash !== hash) return;
    box.innerHTML = "";
    box.appendChild(el("p", "lib-empty", (e && e.message) || "読み込めませんでした"));
    window.SaiShell.fail();
    return;
  }
  if (location.hash !== hash) return; // 既に別の画面へ移っている
  box.innerHTML = "";
  if (data.text) box.innerHTML = renderMarkdown(data.text);
  else box.appendChild(el("p", "lib-empty", "本文がありません"));
  window.SaiShell.ok();
}

// --- 画面の登録（ブラウザでのみ・07 §4.2） ---

// 論文情報の def は interval を動かす（音声作成中のときだけ 10 秒・07 §4.2）
// 住所は新旧どちらも読む（18 §3.5）。name が null なら選んでいるプロジェクトで開く。
const paperRoute = {
  match(hash) {
    const p = parsePaperHash(hash);
    return p && !p.text ? { id: p.id, name: p.name } : null;
  },
  view: "view-paper",
  tab: "research",
  hideTabbar: false,
  interval: 0,
  async open(params) {
    await openPaper(params);
  },
  async poll() {
    await pollPaper();
    await papersPollExtras(); // Download が走っているときだけ 3 秒ごと（U6）
  },
};

// Dashboard（数字カードと出来事・Project のカード）
const dashboardRoute = {
  match(hash) {
    return hash === "#/papers/dashboard" ? {} : null;
  },
  view: "view-papers-dashboard",
  tab: "research",
  hideTabbar: false,
  interval: 0, // 定期更新はしない（↻ で読み直す）
  async open() {
    await openPapersDashboard();
  },
  async poll() {
    await papersPollExtras();
  },
};

// Library 一覧・階層
const libraryRoute = {
  match(hash) {
    return parseLibraryHash(hash);
  },
  view: "view-papers-library",
  tab: "research",
  hideTabbar: false,
  interval: 0, // 定期更新はしない（↻ で読み直す・07 §4.3）
  async open(params) {
    await openLibrary({ paper: params && params.paper, log: params && params.log, find: params && params.find });
  },
  async poll() {
    await papersPollExtras();
  },
};

// 旧 Discover の住所（ブックマーク用）。Library に統合したので移すだけ
const discoverRoute = {
  match(hash) {
    return hash === "#/papers/discover" ? {} : null;
  },
  view: "view-papers-library",
  tab: "research",
  hideTabbar: false,
  interval: 0,
  async open() {
    location.replace("#/papers/library");
  },
};

// 過去の検索結果（新旧どちらの住所でも・08 §4.2・18 §3.5）
const logRoute = {
  match(hash) {
    const p = parseLogHash(hash);
    return p && p.position == null ? { id: p.id, name: p.name } : null;
  },
  view: "view-papers-log",
  tab: "research",
  hideTabbar: false,
  interval: 0,
  async open(params) {
    await openLog(params);
  },
  async poll() {
    await papersPollExtras();
  },
};

// U03: Library 内の検索結果（#/papers/find/<name>/<log_id>・狭い画面用）
const findRoute = {
  match(hash) {
    const p = parseFindHash(hash);
    return p ? { id: p.id, name: p.name } : null;
  },
  view: "view-papers-find",
  tab: "research",
  hideTabbar: false,
  interval: 0,
  async open(params) {
    await lfindOpen(params);
  },
  async poll() {
    await papersPollExtras();
  },
};

// 未ダウンロードの論文情報（新旧どちらの住所でも・08 §4.4・18 §3.5）
const remoteRoute = {
  match(hash) {
    const p = parseLogHash(hash);
    return p && p.position != null
      ? { id: p.id, name: p.name, position: p.position }
      : null;
  },
  view: "view-paper-remote",
  tab: "research",
  hideTabbar: false,
  interval: 0,
  async open(params) {
    await openRemote(params);
  },
  async poll() {
    await papersPollExtras();
  },
};

// 本文（新旧どちらの住所でも・18 §3.5。論文情報より先に判定される必要は無い）
const paperTextRoute = {
  match(hash) {
    const p = parsePaperHash(hash);
    return p && p.text ? { id: p.id, name: p.name } : null;
  },
  view: "view-paper-text",
  tab: "research",
  hideTabbar: false,
  interval: 0,
  async open(params) {
    await openPaperText(params);
  },
  async poll() {
    await papersPollExtras();
  },
};

if (typeof window !== "undefined") {
  // 09・10 が関数を登録すると，論文情報を描くたびに (detail, slotElement) で呼ぶ（07 §4.4-3）
  window.SaiPaperHooks = window.SaiPaperHooks || [];

  // paperimport.js が「Download が走っているあいだだけ 3 秒ごとに読み直す」ため、
  // Papers の各画面の def を出しておきます（def.interval だけを変える）。
  window.SaiPapersRoutes = {
    dashboard: dashboardRoute,
    library: libraryRoute,
    discover: discoverRoute,
    log: logRoute,
    find: findRoute,
    remote: remoteRoute,
    paper: paperRoute,
    text: paperTextRoute,
  };

  (window.SaiRoutes = window.SaiRoutes || []).push(
    dashboardRoute,
    libraryRoute,
    discoverRoute,
    logRoute,
    findRoute,
    remoteRoute,
    paperTextRoute,
    paperRoute
  );


  // 再読み込みボタン（↻）（一覧のプロジェクトも取り直す・18 §1.1）
  const reload = document.getElementById("papers-reload");
  if (reload) reload.addEventListener("click", () => { void openLibrary({ force: true }); });

  // 上部のプロジェクト名（押すと切り替えの一覧が下から出る・18 §1.1）
  const libChip = document.getElementById("library-project");
  if (libChip) libChip.addEventListener("click", () => { void openProjectSheet(); });
  for (const id of ["dashboard-project", "radio-project", "sci-project"]) {
    const chip = document.getElementById(id);
    if (chip) chip.addEventListener("click", () => { void openProjectSheet(); });
  }
  const dashReload = document.getElementById("papers-dashboard-reload");
  if (dashReload) dashReload.addEventListener("click", () => { void openPapersDashboard(); });

  // 絞り込み欄（画面の中だけで絞る・サーバーに聞かない・07 §4.3）
  const filter = document.getElementById("library-filter");
  if (filter) {
    filter.addEventListener("input", (e) => {
      libFilter = (e.target && e.target.value) || "";
      if (libPapers !== null) renderLibrary();
    });
  }

  // 戻る: 論文情報 → 最後に見ていた一覧，本文 → その論文（07 §4.2）
  const back = document.getElementById("paper-back");
  if (back) back.addEventListener("click", () => { location.hash = loadPapersList(); });
  const textBack = document.getElementById("paper-text-back");
  if (textBack) {
    textBack.addEventListener("click", () => {
      location.hash = paperIdNum != null
        ? paperHash(paperProject, paperIdNum, false)
        : "#/papers/library";
    });
  }

  // --- Discover の部品（08 §4.1）。U03: 内部のときは lfindRun を呼ぶ ---
  // 検索ボタンと検索欄の Enter
  const go = document.getElementById("discover-go");
  if (go) go.addEventListener("click", () => {
    if (libFindKind === "paper") return;
    if (libFindKind === "internal") void lfindRun();
    else void runDiscoverSearch();
  });
  const dInput = document.getElementById("discover-input");
  if (dInput) {
    dInput.addEventListener("keydown", (e) => {
      if (e.key === "Enter") {
        e.preventDefault();
        if (libFindKind === "paper") return;
        if (libFindKind === "internal") void lfindRun();
        else void runDiscoverSearch();
      }
    });
    // 検索欄の文字は画面を移っても消さない
    dInput.addEventListener("input", (e) => {
      discoverQuery = (e.target && e.target.value) || "";
    });
  }
  // 条件の開閉
  const condToggle = document.getElementById("discover-cond-toggle");
  if (condToggle) {
    condToggle.addEventListener("click", () => {
      discoverCondOpen = !discoverCondOpen;
      syncCondUI();
    });
  }
  // 検索先の札（押すと選択・解除。最後の 1 個は外せない・08 §4.1）
  const dbsBox = document.getElementById("discover-dbs");
  if (dbsBox) {
    dbsBox.addEventListener("click", (e) => {
      const target = e.target && e.target.closest ? e.target.closest("button[data-db]") : null;
      if (!target) return;
      const db = target.dataset.db;
      const cur = (discoverDbs || []).slice();
      const at = cur.indexOf(db);
      if (at >= 0) {
        if (cur.length <= 1) return;
        cur.splice(at, 1);
      } else {
        cur.push(db);
      }
      discoverDbs = cur;
      saveDiscoverDbs(cur);
      syncCondUI();
    });
  }
  // 年の範囲（空欄は送らない・08 §4.1）
  const yearFrom = document.getElementById("discover-year-from");
  if (yearFrom) {
    yearFrom.addEventListener("input", (e) => {
      discoverYears.from = (e.target && e.target.value) || "";
    });
  }
  const yearTo = document.getElementById("discover-year-to");
  if (yearTo) {
    yearTo.addEventListener("input", (e) => {
      discoverYears.to = (e.target && e.target.value) || "";
    });
  }
  // 件数（10 / 20 / 50・既定 20・08 §4.1）
  const limitBox = document.getElementById("discover-limit");
  if (limitBox) {
    limitBox.querySelectorAll("button").forEach((b) => {
      b.addEventListener("click", () => {
        const n = Number(b.dataset.limit);
        if (n === 10 || n === 20 || n === 50) discoverLimit = n;
        syncCondUI();
      });
    });
  }
  // U03: 外部 DB／内部 DB／Paper 一覧の切り替え
  const kindBox = document.getElementById("library-kind");
  if (kindBox) {
    kindBox.querySelectorAll("button").forEach((b) => {
      b.addEventListener("click", () => {
        const next = libFindKindOf(b.dataset.kind);
        if (next === libFindKind) return;
        libFindKind = next;
        saveLibFindKind(next);
        syncCondUI();
        renderLibrary();
      });
    });
  }
  // U03: 内部検索の条件（Mode・Scope・Count）
  const findModeBox = document.getElementById("find-mode");
  if (findModeBox) {
    findModeBox.querySelectorAll("button").forEach((b) => {
      b.addEventListener("click", () => {
        const v = b.dataset.mode || "";
        if (v !== "" && v !== "word" && v !== "vector" && v !== "hybrid") return;
        libFindMode = v;
        syncCondUI();
      });
    });
  }
  const findScopeBox = document.getElementById("find-scope");
  if (findScopeBox) {
    findScopeBox.querySelectorAll("button").forEach((b) => {
      b.addEventListener("click", () => {
        const v = b.dataset.scope || "";
        if (v !== "" && v !== "papers" && v !== "notes" && v !== "figures" && v !== "all") return;
        libFindScope = v;
        syncCondUI();
      });
    });
  }
  const findCountBox = document.getElementById("find-count");
  if (findCountBox) {
    findCountBox.querySelectorAll("button").forEach((b) => {
      b.addEventListener("click", () => {
        const n = Number(b.dataset.count);
        if (n === 10 || n === 20 || n === 50) libFindCount = n;
        syncCondUI();
      });
    });
  }
  const findPapersBtn = document.getElementById("find-papers-btn");
  if (findPapersBtn) findPapersBtn.addEventListener("click", () => { lfindOpenPaperSheet(); });
  const findNode = document.getElementById("find-node");
  if (findNode) {
    findNode.addEventListener("change", (e) => {
      libFindNode = (e.target && e.target.value) || "";
      syncCondUI();
    });
  }
  const findSubtree = document.getElementById("find-subtree");
  if (findSubtree) {
    findSubtree.addEventListener("change", (e) => {
      libFindSubtree = !!(e.target && e.target.checked);
      syncCondUI();
    });
  }
  const findEach = document.getElementById("find-each");
  if (findEach) {
    findEach.addEventListener("input", (e) => {
      libFindEach = (e.target && e.target.value) || "";
    });
  }
  // U03: 内部検索の結果の戻る → Library
  const findBack = document.getElementById("find-back");
  if (findBack) findBack.addEventListener("click", () => { location.hash = "#/papers/library"; });
  // 過去の検索結果の戻る → Library（08 §4.2）
  const logBack = document.getElementById("log-back");
  if (logBack) logBack.addEventListener("click", () => { location.hash = "#/papers/library"; });
  // 未ダウンロードの論文情報の戻る → その検索結果（08 §4.4）
  const remoteBack = document.getElementById("remote-back");
  if (remoteBack) {
    remoteBack.addEventListener("click", () => {
      location.hash = remoteView
        ? logHash(remoteView.name, remoteView.logId, null)
        : "#/papers/library";
    });
  }
}

// --- node からの読み込み（tests/test_papers.js・07 §4.7・18 §3.5・§3.6） ---
if (typeof module !== "undefined" && module.exports) {
  module.exports = {
    authorShort,
    filterPapers,
    paperHash,
    parsePaperHash,
    logHash,
    parseLogHash,
    parseLibraryHash,
    findHash,
    parseFindHash,
    libraryFindHash,
    lfindBody,
    lfindSummary,
    migrateTreeFolds,
    migrateDiscoverLast,
    activityHash,
    papersDbLabel,
    chunkGroups,
    chunksBody,
    libFindKindOf,
    papersFailedLines,
    papersAllFailed,
    paperCardText,
    RESEARCH_SEG,
    pdMergeActivity,
  };
}
