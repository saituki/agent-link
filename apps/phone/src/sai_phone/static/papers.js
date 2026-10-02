// sai-phone の Papers タブ（実装計画 07）の本体。
//
// 画面は7つ（Radio は radio.js が持つ・09）:
//   Dashboard     #/papers/dashboard（数字カードと出来事の一覧。Radio の出来事も混ざる）
//   Library 一覧   #/papers/library
//   Discover      #/papers/discover（検索と最近の検索・計画 08 §4.1）
//   過去の検索結果  #/papers/log/<name>/<log_id>（08 §4.2・18 §3.5）
//   未DL論文の情報 #/papers/log/<name>/<log_id>/<position>（08 §4.4・18 §3.5）
//   論文情報       #/paper/<name>/<id>（07 §4.4・18 §3.5）
//   本文           #/paper/<name>/<id>/text（07 §4.5・18 §3.5）
// <name> は「論文の置き場があるプロジェクト」の名前。住所に名前がある画面を開いたら
// 「選んでいるプロジェクト」をその名前に切り替え，名前の無い古い住所（ホーム画面の
// 履歴や古い通知から来る）は選んでいるプロジェクトの同じ画面として開く（18 §3.5）。
// Papers の上部でプロジェクトを切り替え，選んだプロジェクトは覚えておく（18 §1.1・§3.6）。
// Radio（#/papers/radio・#/radio/<radio_id>）の中身は radio.js（09）にある。
// 中身はすべて /api/projects/<name>/papers/… 経由（18 §3.4）で，sai-phone 自身は
// sai-paper の DB を開かない（00 §2）。
// 通信は app.js の apiFetch（合言葉を自動で付ける），Markdown の表示は
// markdown.js の renderMarkdown，数字や要素の作りは stats.js の関数を使い回す（00 §9）。
// DOM を使わない小さな関数は末尾で node に渡す（tests/test_papers.js）。
"use strict";

// --- 定数 ---

// Papers の上部の切り替え（07 §4.1）。押すと該当の # へ移る。
// U19: 先頭に研究方針（#/papers/tree）を足した。論文タブの最初の画面。
const PAPERS_SEG = [
  ["tree", "Research tree", "#/papers/tree"],
  ["dashboard", "Dashboard", "#/papers/dashboard"],
  ["discover", "Discover", "#/papers/discover"],
  ["library", "Library", "#/papers/library"],
  ["radio", "Radio", "#/papers/radio"],
];
// 一覧 / 階層 の選択を覚える（localStorage・07 §4.3）
const LIB_MODE_KEY = "sai_phone_library_mode";
// 階層の開閉を覚える（sessionStorage・07 §4.3）
const TREE_FOLDS_KEY = "sai_phone_tree_folds";
// 「最後に見ていた一覧」＝論文情報の戻る先（sessionStorage・07 §4.2）
const PAPERS_LIST_KEY = "sai_phone_papers_list";
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
const DISCOVER_DBS_KEY = "sai_phone_discover_dbs";
// Discover: 最後に見た結果（sessionStorage・08 §4.1）
const DISCOVER_LAST_KEY = "sai_phone_discover_last";
// Discover: 「最近の検索」に出す件数（08 §4.1）
const DISCOVER_RECENT_N = 10;
// 選んでいるプロジェクトを覚える（localStorage・18 §3.6）
const PAPERS_PROJECT_KEY = "sai_phone_papers_project";
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
};
// 論文の置き場があるプロジェクトが1つもないときの案内文（18 §1.1）
const NO_PROJECT_MSG =
  "論文の置き場があるプロジェクトがありません（Mac で sai-paper init を実行してください）";
// 広い画面（900px 以上）で Library と Discover の一覧の横に置く「中身」の欄の id
const PAPER_SIDE_IDS = { library: "library-side", discover: "discover-side" };
// 要対応の種類 → [札の文, 札の色]（paperimport.js の PIM_ATTENTION_KINDS と同じ種類）
const ATTENTION_CHIPS = {
  empty_review: ["review", "muted"],
  question: ["Question", "warn"],
  state_update: ["State", "warn"],
  recommend_card: ["Recommend", "ok"],
  report_issue: ["Source", "danger"],
  new_set_items: ["New", "muted"],
  // U19: 研究方針の木から出る要対応
  sci_decision: ["NO_STAR", "warn"],
  sci_triage: ["PROPOSAL", "warn"],
};
// 広い画面の「中身」に出しているもの（"" は何も出していない）
let sideKind = "";       // "paper" | "remote" | ""
let sidePaperId = null;  // 論文の id（sideKind = "paper"）
let sideRemote = null;   // {logId, name, position}（sideKind = "remote"）
// Library・Discover の上に出す「ダウンロード中」の帯（作ったものを覚えておく）
let downloadBand = null;
// Download の表示を入れる箱（画面の中に置く・key で探せるように覚えておく）
const jobBoxes = [];

// --- ブラウザの保存場所（プライベートブラウズでも動くように必ず失敗に備える） ---

function libMode() {
  try {
    return localStorage.getItem(LIB_MODE_KEY) === "tree" ? "tree" : "list";
  } catch {
    return "list";
  }
}

function saveLibMode(mode) {
  try {
    localStorage.setItem(LIB_MODE_KEY, mode === "tree" ? "tree" : "list");
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

let libPapers = null;   // null は「まだ読んでいない／読み込み中」
let libGroups = null;   // 階層（tree を読んだときだけ）
let libFilter = "";     // 絞り込みの言葉（画面の中だけ）
let libError = "";      // 最後に失敗したときの文（表示が空のときだけ出す）

// --- 画面の状態（論文情報） ---

let paperIdNum = null;  // 開いている論文の id
let paperData = null;   // GET /api/projects/<name>/papers/<id> の応答
let paperError = "";    // 失敗の detail
let abstractLang = "";  // 要旨の言語（"ja" | "en"）
let abstractMsg = "";   // 訳の作成に失敗したときの detail
let translating = false;// 訳の作成中
let refsAll = false;    // 引用文献を全部出しているか
let refsOpen = new Set(); // 引用文献を開いた行（index）・その場で開く用
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
let memoDraft = "";        // 考え（mind.md）に足す 1 行のメモ
let memoSaving = false;    // メモを足している途中
let memoMsg = "";          // メモに失敗したときの理由
let memoFocused = false;   // メモの入力欄に focus がある（描き直し後も戻す）
const attentionBusy = new Set(); // 「対応済みにする」を押している途中

// --- 画面の状態（Discover / 過去の検索結果・08 §4） ---
let discoverProviders = null;   // {default, available}（null＝まだ読んでいない）
let discoverDbs = null;         // 選んでいる検索先（null＝読み込み前）
let discoverYears = { from: "", to: "" }; // 年の範囲（空欄は送らない・08 §4.1）
let discoverLimit = 20;         // 件数（10 / 20 / 50・既定 20）
let discoverCondOpen = false;   // 条件を開いているか
let discoverQuery = "";         // 検索欄の今の文字（画面を移っても消さない）
let discoverMode = "recent";    // "recent" | "results" | "loading" | "error"
let discoverResult = null;      // {logId, query}（mode = "results" の中身）
let discoverError = "";         // 失敗の detail（mode = "error" に出す）
let discoverRecent = null;      // 最近の検索（null＝未読込）
let discoverRecentError = "";   // 最近の検索を読めなかったときの文
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
  memoDraft = "";
  memoMsg = "";
  attentionBusy.clear();
  sideKind = "";
  sidePaperId = null;
  sideRemote = null;
  jobBoxes.length = 0;
  libPapers = null;
  libGroups = null;
  libError = "";
  discoverProviders = null;
  discoverRecent = null;
  discoverRecentError = "";
  discoverResult = null;
  discoverError = "";
  discoverMode = "recent";
  logCache.clear();
  logView = null;
  remoteView = null;
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
  // import-project は Import 画面（paperimport.js）が作るので、ここでも名前を揃える
  for (const id of ["dashboard-project", "library-project", "discover-project", "radio-project", "import-project", "sci-project"]) {
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
  // U19: 研究方針の木（節点つきでも同じ画面を作り直す）
  if (/^#\/papers\/tree(?:\/N[1-9][0-9]{0,8})?$/.test(location.hash) && typeof stOpen === "function") void stOpen(stParseHash(location.hash));
  else if (location.hash === "#/papers/discover") void openDiscover();
  else if (location.hash === "#/papers/dashboard") void openPapersDashboard();
  else if (location.hash === "#/papers/radio") void openRadioList(); // radio.js
  else if (location.hash === "#/papers/import" && typeof pimOpenImport === "function") void pimOpenImport(); // paperimport.js（U6）
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
 * Radio の再生・質問は再生画面，論文のある出来事は論文情報，それ以外は null（押せない）。
 */
function activityHash(item, projectName) {
  if (!item) return null;
  const radioKinds = item.kind === "radio_play" || item.kind === "radio_question";
  if (radioKinds && item.radio_id) return `#/radio/${encodeURIComponent(item.radio_id)}`;
  if (item.paper_id != null && projectName) return paperHash(projectName, item.paper_id, false);
  return null;
}

async function openPapersDashboard() {
  savePapersList("#/papers/dashboard"); // 論文情報の戻る先（07 §4.2）
  fillSegment("dashboard-seg", "dashboard");
  await ensurePaperProjects();
  if (location.hash !== "#/papers/dashboard") return;
  const name = selectedProject;
  if (!name) {
    dashboardData = null;
    dashboardError = "";
    renderPapersDashboard();
    return;
  }
  if (dashboardData === null) renderPapersDashboard(); // 「読み込み中…」を先に出す
  // Project（要対応・現況・考え）は別々に読む。数字カードと並べて出す
  void loadDashboardProject(name);
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

function renderPapersDashboard() {
  const box = document.getElementById("papers-dashboard-body");
  if (!box) return;
  box.innerHTML = "";
  if (!selectedProject) {
    box.appendChild(el("p", "lib-empty", paperProjectsError || NO_PROJECT_MSG));
    return;
  }
  // 0. Project（要対応・現況・考え・状況のまとめ）。読み込みに失敗しても下の数字と出来事は出す
  if (projectData !== null || projectError) box.appendChild(projectCards());
  if (!dashboardData) {
    box.appendChild(el("p", "lib-empty", dashboardError || "読み込み中…"));
    pdFocusMemo();
    return;
  }
  // 1. 数字カード（Mac の Dashboard の 5 つ + DEBT）
  const c = dashboardData.cards || {};
  const card = el("section", "dash-card");
  const grid = el("div", "dash-grid pd-grid");
  for (const [value, label] of [
    [String(Number(c.papers) || 0), "Papers"],
    [String(Number(c.tags) || 0), "Tags"],
    [String(Number(c.notes) || 0), "Notes"],
    [bytesLabel(c.pdf_bytes), "PDF"],
    [bytesLabel(c.db_bytes), "DB"],
    [String(papersDebtCount(dashboardData.activity)), "DEBT"],
  ]) {
    const cell = el("div", "dash-cell");
    cell.appendChild(el("div", "dash-big", value));
    cell.appendChild(el("div", "dash-big-label", label));
    grid.appendChild(cell);
  }
  card.appendChild(grid);
  box.appendChild(card);

  // 2. 出来事（新しい順・種類ごとに色分け）
  const list = el("section", "dash-card");
  list.appendChild(el("h3", "dash-card-title", "Recent activity"));
  const items = Array.isArray(dashboardData.activity) ? dashboardData.activity : [];
  if (items.length === 0) list.appendChild(el("p", "lib-empty", "No activity yet"));
  for (const item of items) list.appendChild(activityRow(item));
  if (dashboardError) list.appendChild(el("p", "paper-msg", dashboardError));
  box.appendChild(list);
  pdFocusMemo(); // メモの入力欄便携入力中なら描き直し後も focus を戻す
}

// --- Project（要対応・現況・考え・状況のまとめ・U6・sai-paper の phone project） ---

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

/** Project の 3 枚のカードと状況のまとめ（読み込み中は「読み込み中…」）。 */
function projectCards() {
  const wrap = el("div", "pd-project");
  if (projectData === null) {
    wrap.appendChild(el("p", "lib-empty", projectError || "Loading…"));
    return wrap;
  }
  wrap.appendChild(attentionCard());
  wrap.appendChild(stateCard());
  wrap.appendChild(mindCard());
  wrap.appendChild(briefCard());
  return wrap;
}

/** カード 1 枚の入れ物（見出しの横には用語の札を置ける）。 */
function projectCard(title, term, termLabel) {
  const card = el("section", "dash-card pd-card");
  const head = el("div", "dash-card-head");
  head.appendChild(el("h3", "dash-card-title", title));
  if (term) {
    const badge = paperTerm(term, termLabel);
    if (badge) head.appendChild(badge);
  }
  card.appendChild(head);
  return card;
}

/** 要対応（まだ人が見ていないもの）。「対応済みにする」で消せる。 */
function attentionCard() {
  const items = Array.isArray(projectData.attention) ? projectData.attention : [];
  const card = projectCard(`Attention (${items.length})`, "attention", "Attention");
  if (items.length === 0) {
    card.appendChild(el("p", "lib-empty", "Nothing needs attention"));
    return card;
  }
  const list = el("div", "pd-att-list");
  for (const item of items) list.appendChild(attentionRow(item));
  card.appendChild(list);
  return card;
}

/** 要対応の行き先（研究方針の木・仕分けの画面の住所）。開けないときは空文字。
 *
 * `sci_decision` のキーはコアが `sci_decision:Q<番号>` の形で作る
 * （crates/sai-paper-core/src/attention.rs）。形が合えば研究方針の画面を開く。
 */
function papersAttentionHash(item) {
  const it = item || {};
  if (it.kind === "sci_triage") return "#/papers/triage";
  if (it.kind !== "sci_decision") return "";
  // キーは `sci_decision:<Q の ID>`。その Q を選んだ研究方針の画面を開く
  const m = /^sci_decision:([QOFE][1-9][0-9]{0,8})$/.exec(String(it.key || ""));
  return m ? `#/papers/tree/${m[1]}` : "";
}

/** 要対応の 1 行（種類の札・題・対応済みにする・実行があれば開く）。 */
function attentionRow(item) {
  const it = item || {};
  const kind = ATTENTION_CHIPS[it.kind] || ["Attention", "muted"];
  const row = el("div", "pd-att");

  const main = document.createElement("button");
  main.type = "button";
  main.className = "pd-att-main";
  main.appendChild(el("span", `sai-chip ${kind[1]}`, kind[0]));
  main.appendChild(el("span", "pd-att-title", it.title || "(untitled item)"));
  // U19: 研究方針の木から出る要対応は、節点つきで研究方針（または仕分け）へ移る
  const sciHash = papersAttentionHash(it);
  if (sciHash) {
    main.addEventListener("click", () => { location.hash = sciHash; });
  } else if (it.run_id != null) {
    // 論文のプロジェクトは sai-link のプロジェクトでもある（その実行の review を開く）
    main.addEventListener("click", () => {
      location.hash =
        `#/project/${encodeURIComponent(selectedProject)}/list/${encodeURIComponent(it.run_id)}/review`;
    });
  } else {
    main.disabled = true;
  }
  row.appendChild(main);

  const done = document.createElement("button");
  done.type = "button";
  done.className = "sai-btn small";
  done.textContent = "Mark resolved";
  done.disabled = attentionBusy.has(it.key);
  done.addEventListener("click", () => { void resolveAttention(it); });
  row.appendChild(done);
  return row;
}

/** 要対応を 1 件、対応済みにする。 */
async function resolveAttention(item) {
  const name = selectedProject;
  if (!name || !item || !item.key || attentionBusy.has(item.key)) return;
  const ok = await paperConfirm({
    title: "Mark as resolved?",
    body: `Mark ${item.title || "this item"} as resolved.`,
    ok: "Mark resolved",
  });
  if (!ok) return;
  attentionBusy.add(item.key);
  renderPapersDashboard();
  try {
    await paperApi(`${papersApiBase(name)}/attention/resolve`, {
      method: "POST",
      body: { key: item.key },
    });
    if (projectData && Array.isArray(projectData.attention)) {
      projectData.attention = projectData.attention.filter((a) => a && a.key !== item.key);
    }
    if (typeof saiToast === "function") saiToast("Marked as resolved", "ok");
    void loadDashboardProject(name, true); // 現況・考え・要対応を読み直す
  } catch (e) {
    if (typeof saiToast === "function") {
      saiToast((e && e.message) || "Could not mark as resolved", "error");
    }
  } finally {
    attentionBusy.delete(item.key);
    renderPapersDashboard();
  }
}

/** 現況（state.md）。読むだけ（sai-paper の決まりでスマホからは書き換えない）。 */
function stateCard() {
  const card = projectCard("State", "state", "State");
  card.appendChild(projectMd(projectData.state, "No state yet. It is written as the agent advances the research"));
  return card;
}

/** 考え（mind.md）。下に 1 行のメモを足せる。 */
function mindCard() {
  const card = projectCard("Mind", "mind", "Mind");
  card.appendChild(projectMd(projectData.mind, "No mind yet. Add a memo below and it appears here"));

  const line = el("div", "pd-memo-line");
  const input = document.createElement("input");
  input.type = "text";
  input.className = "sai-input pd-memo";
  input.placeholder = "考えを 1 行足す";
  input.value = memoDraft;
  input.maxLength = 2000;
  input.addEventListener("input", () => { memoDraft = input.value; });
  input.addEventListener("focus", () => { memoFocused = true; });
  input.addEventListener("blur", () => { memoFocused = false; });
  input.addEventListener("keydown", (e) => {
    if (e.key === "Enter") {
      e.preventDefault();
      void addMemo();
    }
  });
  const mic = document.createElement("button");
  mic.type = "button";
  mic.className = "sai-btn small";
  mic.textContent = "🎤";
  mic.setAttribute("aria-label", "音声で入力");
  if (typeof window.attachVoiceInput !== "function") mic.hidden = true;
  else window.attachVoiceInput(mic, input);
  const add = document.createElement("button");
  add.type = "button";
  add.className = "sai-btn primary small";
  add.textContent = memoSaving ? "足しています…" : "足す";
  add.disabled = memoSaving;
  add.addEventListener("click", () => { void addMemo(); });
  line.appendChild(input);
  line.appendChild(mic);
  line.appendChild(add);
  card.appendChild(line);
  card.appendChild(el("p", "pim-note", "Enter でも足せます。1 行だけ（上書きはしません）"));
  if (memoMsg) card.appendChild(el("p", "pim-error", memoMsg));
  return card;
}

/** 考えに 1 行のメモを足す。 */
async function addMemo() {
  const name = selectedProject;
  const text = String(memoDraft || "").trim();
  if (!name || memoSaving) return;
  if (!text) {
    memoMsg = "メモの文字を入れてください";
    renderPapersDashboard();
    return;
  }
  memoSaving = true;
  memoMsg = "";
  renderPapersDashboard();
  try {
    await paperApi(`${papersApiBase(name)}/project/memo`, { method: "POST", body: { text } });
    memoDraft = "";
    if (typeof saiToast === "function") saiToast("考えに足しました", "ok");
    void loadDashboardProject(name, true);
  } catch (e) {
    memoMsg = (e && e.message) || "足せませんでした";
  } finally {
    memoSaving = false;
    renderPapersDashboard();
  }
}

/** 状況のまとめ（エージェントに渡す文）。たたんで出す。 */
function briefCard() {
  const brief = String(projectData.brief || "");
  const det = document.createElement("details");
  det.className = "pd-brief";
  det.appendChild(el("summary", null, "Status summary (text for the agent)"));
  if (!brief.trim()) {
    det.appendChild(el("p", "lib-empty", "No status summary yet"));
  } else {
    const box = el("div", "pd-md");
    box.innerHTML = renderMarkdown(brief, { toolbar: true });
    det.appendChild(box);
  }
  return det;
}

/** md を表示する（長いものははじめはたたんで「全部見る」）。 */
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

/** 描き直したあともメモの入力欄に focus を戻す（入力中の文字を消さないため）。 */
function pdFocusMemo() {
  if (!memoFocused) return;
  const input = document.querySelector("#papers-dashboard-body .pd-memo");
  if (!input) return;
  input.focus();
  const at = input.value.length;
  try { input.setSelectionRange(at, at); } catch { /* 数値の欄だけなら無視 */ }
}

/** Recent Activity の Action の行のうち DEBT のある行の数。 */
function papersDebtCount(items) {
  return (Array.isArray(items) ? items : []).filter((i) => i && i.kind === "action" && i.debt).length;
}

function activityRow(item) {
  const [label, cls] = ACTIVITY_KINDS[item && item.kind] || [String((item && item.kind) || ""), "ak-other"];
  const row = document.createElement("button");
  row.className = "pd-act";
  row.appendChild(el("span", `pd-kind ${cls}`, label));
  const debt = item && item.kind === "action" && item.debt;
  row.appendChild(el("span", "pd-label", (debt ? "⚠ " : "") + ((item && item.label) || "")));
  if (debt && Array.isArray(item.debt_messages)) row.title = item.debt_messages.join("\n");
  row.appendChild(el("span", "pd-time", activityTime(item && item.occurred_at)));
  const hash = activityHash(item, selectedProject);
  if (hash) row.addEventListener("click", () => { location.hash = hash; });
  else row.disabled = true;
  return row;
}

// --- 共通: 上部の切り替え（Discover / Library / Radio・07 §4.1） ---

/**
 * 論文タブの「進み具合」ボタンを今の数で描き直す
 * （paperimport.js が状態を読み直したあとに呼ばれる）。
 */
function papersRenderProgressButton() {
  if (typeof pimProgressButton !== "function") return;
  const old = document.querySelector(".papers-seg-bar .pim-progress-tab");
  if (!old || !old.parentNode) return;
  old.parentNode.replaceChild(pimProgressButton(), old);
}

function fillSegment(containerId, active) {
  const box = document.getElementById(containerId);
  if (!box) return;
  box.innerHTML = "";
  // 切り替え帯と右端の「進み具合」を行に並べる（帯の外側）
  const row = document.createElement("div");
  row.className = "papers-seg-bar";
  const seg = document.createElement("div");
  seg.className = "dash-seg papers-seg"; // 06 の期間切り替えと同じ見た目
  for (const [key, label, hash] of PAPERS_SEG) {
    const b = document.createElement("button");
    b.textContent = label;
    if (key === active) b.className = "active";
    b.addEventListener("click", () => { location.hash = hash; });
    seg.appendChild(b);
  }
  row.appendChild(seg);
  // 走っている Download があれば数をつける（paperimport.js が作る）
  if (typeof pimProgressButton === "function") row.appendChild(pimProgressButton());
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
    wrap.appendChild(main);
  }
  return main;
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
  // 音声や図の置き場はここ（論文情報の変数）
  paperProject = projectName;
  paperIdNum = paperId;
  main.innerHTML = "";
  main.appendChild(el("p", "lib-empty", "読み込み中…"));
  void loadPaper(paperId, main);
}

/** 広い画面なら右の欄に検索の結果（未ダウンロードの論文の情報）を出す。 */
async function showRemoteInSide(main, projectName, logId, position) {
  if (!main) {
    location.hash = logHash(projectName, logId, position);
    return;
  }
  sideKind = "remote";
  sidePaperId = null;
  sideRemote = { logId, name: projectName, position };
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
      lines.push(`${name} の API キーが使えませんでした（~/.sai-moon/paper.env を確かめてください）。`);
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

// --- Download の表示（paperimport.js のを見張り・U6） ---

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

/** Library の画面の上に「ダウンロード中 N 件」の細い帯を出す（Download が走るときだけ）。 */
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
  downloadBand.textContent = "";
  const btn = document.createElement("button");
  btn.type = "button";
  btn.className = "sai-btn small";
  btn.textContent = `ダウンロード中 ${jobs.length} 件（進み具合を開く）`;
  btn.addEventListener("click", () => { location.hash = "#/papers/import"; });
  downloadBand.appendChild(btn);
}

/** 3 秒ごとの更新で Download の進み具合も見る（paperimport.js があれば）。 */
async function papersPollExtras() {
  if (typeof pimPollJobs === "function") await pimPollJobs();
}

// --- Library（#/papers/library・07 §4.3） ---

async function openLibrary(opts) {
  // 論文情報の戻る先として今の住所を覚える（07 §4.2）
  savePapersList("#/papers/library");
  fillSegment("library-seg", "library");
  updateModeButtons();
  papersImportButton(); // 道具行の「＋ BibTeX を読み込む」
  const input = document.getElementById("library-filter");
  if (input) input.value = libFilter;
  // どのプロジェクトの Library を出すか決める（↻ は一覧も取り直す・18 §1.1）
  await ensurePaperProjects(opts && opts.force);
  if (location.hash !== "#/papers/library") return; // 既に別の画面へ移っている
  if (!selectedProject) {
    // 置き場があるプロジェクトが1つも無い（案内文・18 §1.1）
    libPapers = null;
    libGroups = null;
    libError = "";
    renderLibrary();
    return;
  }
  // まだ何も無いときは「読み込み中…」を先に出す（07 §4.2）
  if (libPapers === null) renderLibrary();
  // 走っている Download があれば上の帯を出す（Import を開いたときの一度きりの読み出し）
  if (typeof pimRefreshJobs === "function") void pimRefreshJobs(selectedProject);
  await loadLibrary();
}

/** Library の道具行に「＋ BibTeX を読み込む」を足す（1 回だけ）。 */
function papersImportButton() {
  if (document.getElementById("library-import")) return;
  const tools = document.querySelector("#view-papers-library .lib-tools");
  if (!tools) return;
  const btn = document.createElement("button");
  btn.id = "library-import";
  btn.type = "button";
  btn.className = "sai-btn small papers-import-btn";
  btn.textContent = "＋ BibTeX を読み込む";
  btn.addEventListener("click", () => { location.hash = "#/papers/import"; });
  tools.appendChild(btn);
}

async function loadLibrary() {
  const hash = location.hash;
  const name = selectedProject;
  if (!name) return;
  const withTree = libMode() === "tree";
  try {
    const res = await apiFetch(`${papersApiBase(name)}/library`);
    const body = await res.json().catch(() => ({}));
    if (!res.ok) throw new Error(body.detail || "読み込めませんでした");
    let groups = null;
    if (withTree) {
      // 階層表示のときは tree も読む（一覧表示では要らないため後から）
      const res2 = await apiFetch(`${papersApiBase(name)}/tree`);
      const body2 = await res2.json().catch(() => ({}));
      if (!res2.ok) throw new Error(body2.detail || "階層を読み込めませんでした");
      groups = Array.isArray(body2.groups) ? body2.groups : [];
    }
    // 既に別の画面へ移った，またはその間にプロジェクトが切り替わったら捨てる（18 §1.1）
    if (location.hash !== hash || selectedProject !== name) return;
    libPapers = Array.isArray(body.papers) ? body.papers : [];
    libGroups = groups;
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
  if (libMode() === "tree") {
    if (libGroups) renderTree(box);
    else box.appendChild(el("p", "lib-empty", libError || "読み込み中…"));
  } else {
    renderList(box);
  }
  // 広い画面は左の一覧・右に選んだ論文の中身を出す（狭い画面は作らない）
  const side = paperSidePane("library-list", PAPER_SIDE_IDS.library);
  if (side) paperSideRender(side);
}

function renderList(box) {
  const rows = filterPapers(libPapers, libFilter);
  if (rows.length === 0) {
    if (libPapers.length === 0) {
      // 論文が 1 本も無いときは何をするかを出す
      box.appendChild(
        papersEmpty(
          "📚",
          "まだ論文がありません",
          "Discover で探すか、BibTeX を読み込んでください。",
          [
            { label: "論文を探す（Discover）", onClick: () => { location.hash = "#/papers/discover"; } },
            { label: "BibTeX を読み込む", onClick: () => { location.hash = "#/papers/import"; } },
          ]
        )
      );
      return;
    }
    box.appendChild(
      papersEmpty("🔍", "一致する論文がありません", "上の絞り込みの欄の言葉を短くすると出てきます。", null)
    );
    return;
  }
  for (const p of rows) box.appendChild(paperRow(p, false));
}

/** 一覧の行（階層の中は small で小さく・題名 1 行＋年）。 */
function paperRow(p, small) {
  const btn = document.createElement("button");
  btn.className = small ? "paper-row small" : "paper-row";
  btn.appendChild(el("div", "paper-title", p.title || "(無題)"));

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

/** まとまりのキー（開閉を覚えるため・07 §4.3）。 */
function treeKey(g) {
  return g.kind === "query" ? `query:${g.log_id}` : `import:${g.date}`;
}

function renderTree(box) {
  const words = queryWords(libFilter);
  const folds = loadFolds(selectedProject);
  const byId = new Map((libPapers || []).map((p) => [p.id, p]));
  let shown = 0;

  for (const g of libGroups || []) {
    // 見出しの言葉が合う場合は中身を絞らない（全部出す・07 §4.3）
    const heading = g.kind === "query"
      ? `${g.date || ""} ${g.query || ""}`
      : `${g.date || ""} Import`;
    const headingHit = includesAll(heading, words);
    const all = (g.paper_ids || []).map((id) => byId.get(id)).filter(Boolean);
    const rows = headingHit
      ? all
      : all.filter((p) => includesAll(`${p.title || ""} ${p.authors || ""}`, words));
    // 合う論文が 0 件のまとまりは出さない（見出しの合致時を除く・07 §4.3）
    if (rows.length === 0 && !headingHit) continue;

    const key = treeKey(g);
    const open = folds.includes(key);
    // 見出しの行は「開閉の部分」と右端の「検索結果 ›」に分ける（08 §4.5）
    const head = document.createElement("div");
    head.className = "lib-group-head";
    const toggle = document.createElement("button");
    toggle.className = "lib-g-main";
    toggle.appendChild(el("span", "lib-g-arrow", open ? "▼" : "▶"));
    toggle.appendChild(el("span", "lib-g-date", String(g.date || "")));
    toggle.appendChild(
      el("span", "lib-g-query", g.kind === "query" ? String(g.query || "(検索)") : "Import")
    );
    toggle.appendChild(el("span", "lib-g-count", `(${rows.length})`));
    toggle.addEventListener("click", () => {
      const cur = loadFolds(selectedProject);
      const next = cur.includes(key) ? cur.filter((x) => x !== key) : [...cur, key];
      saveFolds(selectedProject, next);
      renderLibrary();
    });
    head.appendChild(toggle);
    if (g.kind === "query" && g.log_id != null) {
      // 開閉とは別に反応する小さなリンク（押しても開閉しない・08 §4.5）
      const link = document.createElement("button");
      link.className = "lib-g-log";
      link.textContent = "検索結果 ›";
      link.addEventListener("click", (e) => {
        e.stopPropagation();
        location.hash = logHash(selectedProject, g.log_id, null);
      });
      head.appendChild(link);
    }
    box.appendChild(head);
    shown += 1;
    if (!open) continue;
    for (const p of rows) box.appendChild(paperRow(p, true));
  }

  if (shown === 0) {
    box.appendChild(
      papersEmpty(
        "📚",
        libPapers.length === 0 ? "まだ論文がありません" : "一致する論文がありません",
        libPapers.length === 0
          ? "Discover で探すか、BibTeX を読み込んでください。"
          : "上の絞り込みの欄の言葉を短くすると出てきます。",
        libPapers.length === 0
          ? [{ label: "BibTeX を読み込む", onClick: () => { location.hash = "#/papers/import"; } }]
          : null
      )
    );
  }
}

/** 一覧 / 階層 の選択（localStorage に覚える・07 §4.3）。 */
function updateModeButtons() {
  const box = document.getElementById("library-mode");
  if (!box) return;
  const now = libMode();
  box.querySelectorAll("button").forEach((b) => {
    b.classList.toggle("active", b.dataset.mode === now);
  });
}

function onModeClick(mode) {
  if (mode !== "list" && mode !== "tree") return;
  if (libMode() === mode) return;
  saveLibMode(mode);
  updateModeButtons();
  // 階層をまだ読んだことが無ければ，読むところから（一覧の読むのも兼ねる）
  if (mode === "tree" && libGroups === null) {
    void loadLibrary();
    return;
  }
  renderLibrary();
}

// --- Radio は radio.js（計画 09）が持つ ---
// （かつての準備中画面 openSoon は 09 で radio.js に置き換わった）

// --- Discover（#/papers/discover・08 §4.1） ---

/** 条件の札と要約を今の値に合わせる（08 §4.1）。 */
function syncCondUI() {
  const panel = document.getElementById("discover-cond");
  if (panel) panel.hidden = !discoverCondOpen;
  const toggle = document.getElementById("discover-cond-toggle");
  if (toggle) {
    toggle.textContent = discoverCondOpen ? "Filters ▾" : "Filters ▸";
    toggle.setAttribute("aria-expanded", discoverCondOpen ? "true" : "false");
  }
  const summary = document.getElementById("discover-cond-summary");
  if (summary && discoverProviders) summary.textContent = (discoverDbs || []).join(" · ");
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

/** 最近の検索（Library 階層の query まとめの上から 10 件・08 §4.1）。 */
async function loadRecent() {
  if (discoverRecent !== null) return;
  const name = selectedProject;
  if (!name) return;
  try {
    const res = await apiFetch(`${papersApiBase(name)}/tree`);
    const body = await res.json().catch(() => ({}));
    if (!res.ok) throw new Error(body.detail || "読み込めませんでした");
    if (selectedProject !== name) return; // その間に切り替わった（18 §1.1）
    const groups = Array.isArray(body.groups) ? body.groups : [];
    discoverRecent = groups.filter((g) => g.kind === "query").slice(0, DISCOVER_RECENT_N);
    discoverRecentError = "";
    window.SaiShell.ok();
  } catch (e) {
    if (selectedProject !== name) return; // その間に切り替わった（18 §1.1）
    discoverRecent = [];
    discoverRecentError = (e && e.message) || "読み込めませんでした";
    window.SaiShell.fail();
  }
  if (discoverMode === "recent") renderDiscover();
}

/** log の記憶の鍵（log_id はプロジェクトごとに違うため名前と組にする・18 §3.6）。 */
function logCacheKey(projectName, logId) {
  return `${projectName || ""}/${logId}`;
}

/** log_id の結果を取る（入っているなら取り直さない・08 §4.4）。失敗は throw。 */
async function getLog(projectName, logId) {
  const key = logCacheKey(projectName, logId);
  if (logCache.has(key)) return logCache.get(key);
  const res = await apiFetch(`${papersApiBase(projectName)}/logs/${encodeURIComponent(logId)}`);
  const body = await res.json().catch(() => ({}));
  if (!res.ok) throw new Error(body.detail || "読み込めませんでした");
  if (!body || !body.log) throw new Error("sai-paper の応答を読めませんでした");
  logCache.set(key, body);
  return body;
}

async function openDiscover() {
  // 論文情報の戻る先として今の住所を覚える（07 §4.2）
  savePapersList("#/papers/discover");
  fillSegment("discover-seg", "discover");
  const input = document.getElementById("discover-input");
  if (input) input.value = discoverQuery;
  syncCondUI();
  // どのプロジェクトの検索か決める（18 §1.1）
  await ensurePaperProjects();
  if (location.hash !== "#/papers/discover") return; // 既に別の画面へ移っている
  const name = selectedProject;
  if (name) {
    // 前に見た結果を出しておく（検索し直さない・08 §4.1）
    if (discoverMode === "recent" && !discoverResult) {
      const last = loadDiscoverLast(name);
      if (last != null) {
        try {
          const data = await getLog(name, last);
          if (location.hash !== "#/papers/discover" || selectedProject !== name) return;
          discoverResult = { logId: last, name, query: (data.log && data.log.query) || "" };
          discoverMode = "results";
        } catch {
          saveDiscoverLast(name, null); // 消えた結果の覚え書きも消す
        }
      }
    } else if (discoverMode === "results" && discoverResult
      && !logCache.has(logCacheKey(discoverResult.name, discoverResult.logId))) {
      try {
        await getLog(discoverResult.name, discoverResult.logId);
      } catch {
        discoverMode = "recent";
        discoverResult = null;
      }
    }
  }
  renderDiscover();
  if (name) {
    void loadProviders();
    void loadRecent();
  }
}

/** 今の条件で送る検索先（読んでいる available にあるものだけ・08 §4.1）。 */
function selectedDbs() {
  if (!discoverDbs || discoverDbs.length === 0) return null;
  if (!discoverProviders) return discoverDbs.slice();
  const picked = discoverDbs.filter((d) => discoverProviders.available.includes(d));
  return picked.length > 0 ? picked : null;
}

async function runDiscoverSearch() {
  const input = document.getElementById("discover-input");
  const raw = (input && input.value) || "";
  discoverQuery = raw;
  const query = raw.trim();
  const name = selectedProject;
  if (!name) return; // 置き場が無いときは案内文が出ているだけ（18 §1.1）
  if (!query) {
    // 空欄で検索 → 結果を消して「最近の検索」に戻る（08 §4.1）
    saveDiscoverLast(name, null);
    discoverResult = null;
    discoverError = "";
    discoverMode = "recent";
    renderDiscover();
    if (discoverRecent === null) void loadRecent();
    return;
  }
  discoverMode = "loading";
  discoverError = "";
  renderDiscover(); // この中で検索ボタンを押せなくなる（08 §4.1）

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
    discoverError = "接続できていません";
    renderDiscover();
    window.SaiShell.fail();
    return;
  }
  const data = await res.json().catch(() => ({}));
  if (selectedProject !== name) return; // その間に切り替わった（古いプロジェクトの結果を出さない・18 §1.1）
  if (!res.ok) {
    // 失敗の detail を結果の場所に出す（08 §4.1）
    discoverMode = "error";
    discoverError = data.detail || "検索できませんでした";
    renderDiscover();
    return;
  }
  if (!data || !data.log) {
    discoverMode = "error";
    discoverError = "sai-paper の応答を読めませんでした";
    renderDiscover();
    return;
  }
  logCache.set(logCacheKey(name, data.log.id), data);
  saveDiscoverLast(name, data.log.id);
  discoverResult = { logId: data.log.id, name, query: data.log.query || query };
  discoverMode = "results";
  discoverRecent = null; // 履歴に新しい検索が増えたので次に取り直す（08 §4.1）
  renderDiscover();
  window.SaiShell.ok();
}

function renderDiscover() {
  const box = document.getElementById("discover-body");
  if (!box) return;
  const goBtn = document.getElementById("discover-go");
  if (goBtn) goBtn.disabled = discoverMode === "loading";
  // 広い画面は左に結果・右に選んだ論文の中身を出す（U6）
  const side = paperSidePane("discover-body", PAPER_SIDE_IDS.discover);
  if (discoverMode !== "results") {
    sideKind = "";
    sidePaperId = null;
    sideRemote = null;
  }
  if (side) paperSideRender(side);
  box.innerHTML = "";
  if (paperProjects !== null && !selectedProject) {
    // 置き場があるプロジェクトが1つも無い（案内文・18 §1.1）
    box.appendChild(el("p", "lib-empty", NO_PROJECT_MSG));
    return;
  }
  if (discoverMode === "loading") {
    const wrap = document.createElement("div");
    wrap.className = "papers-loading";
    wrap.appendChild(el("span", "papers-spin"));
    wrap.appendChild(el("span", null, "Mac の sai-paper で検索中…"));
    box.appendChild(wrap);
    return;
  }
  if (discoverMode === "error") {
    box.appendChild(el("p", "lib-empty", discoverError || "検索できませんでした"));
    return;
  }
  if (discoverMode === "results" && discoverResult) {
    const data = logCache.get(logCacheKey(discoverResult.name, discoverResult.logId)) || { items: [] };
    const items = Array.isArray(data.items) ? data.items : [];
    const failedBox = papersFailedBox(data);
    if (failedBox) box.appendChild(failedBox);
    box.appendChild(
      el("h2", "papers-heading", `“${discoverResult.query}” の結果 ${items.length} 件`)
    );
    if (items.length === 0) {
      // 全部の検索先が失敗したときは「見つかりません」と言わない（理由を出して終える）
      if (!papersAllFailed(data.failed, data.log && data.log.source_dbs)) {
        box.appendChild(
          papersEmpty(
            "🔍",
            "見つかりませんでした",
            "別の言葉で探すか、下の「条件」で検索先や年を変えてみてください。",
            null
          )
        );
      }
      return;
    }
    for (const item of items) box.appendChild(resultRow(item, discoverResult.name, discoverResult.logId));
    return;
  }
  // 最近の検索（08 §4.1）
  box.appendChild(el("h2", "papers-heading", "最近の検索"));
  if (discoverRecent === null) {
    box.appendChild(el("p", "lib-empty", "読み込み中…"));
    return;
  }
  if (discoverRecentError) {
    box.appendChild(el("p", "lib-empty", discoverRecentError));
    return;
  }
  if (discoverRecent.length === 0) {
    box.appendChild(
      papersEmpty(
        "🔍",
        "まだ検索がありません",
        "上の欄に論文の題名やキーワードを入れて「検索」を押してください。",
        null
      )
    );
    return;
  }
  for (const g of discoverRecent) box.appendChild(discoverRecentRow(g));
}

/** 結果の 1 行（Discover と過去の検索結果で共通・08 §4.3）。 */
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
  btn.addEventListener("click", () => {
    // 広い画面は右の欄に出し、狭い画面は今まで通り論文情報の画面へ（U6）
    const side = paperSidePane("discover-body", PAPER_SIDE_IDS.discover);
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

/**
 * 最近の検索の 1 行（検索語・件数・日付・08 §4.1）。
 * 名前は project.js の recentRow（直近の実行）と重ねない（同じ名前だと後から読む papers.js が上書きする）。
 */
function discoverRecentRow(g) {
  const btn = document.createElement("button");
  btn.className = "paper-row";
  const line1 = document.createElement("div");
  line1.className = "paper-line1";
  line1.appendChild(el("span", "paper-title", g.query || "(検索)"));
  if (g.hit_count != null) line1.appendChild(el("span", "paper-count", `${g.hit_count} 件`));
  btn.appendChild(line1);
  if (g.date) btn.appendChild(el("div", "paper-meta", String(g.date)));
  btn.addEventListener("click", () => {
    location.hash = logHash(selectedProject, g.log_id, null);
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
    sub.textContent = [log.date, log.source_dbs, `${items.length} 件`]
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

  // 3. Download（Library なら開くボタン・無ければ「⬇ ダウンロード」・U6）
  const guide = document.createElement("section");
  guide.className = "dash-card";
  const local = item.local_paper_id != null && item.local_paper_id !== "";
  const key = typeof pimLogKey === "function"
    ? pimLogKey(remoteView.logId, remoteView.position)
    : `log:${remoteView.logId}:${remoteView.position}`;
  const job = typeof pimJobFor === "function"
    ? pimJobFor(typeof pimJobsList === "function" ? pimJobsList() : [], key)
    : null;
  if (local) {
    guide.appendChild(
      paperBtn("Library の論文情報を開く", () => {
        location.hash = paperHash(remoteView.name, item.local_paper_id, false);
      })
    );
    guide.appendChild(el("p", "paper-msg", "この論文は Library に入っています。"));
  } else if (job && (job.status === "downloading" || job.status === "processing")) {
    // 取りに行っている最中は「もう一度」を出さない
    guide.appendChild(el("p", "paper-msg", "PDF を取りに行っています。終わったら通知でお知らせします。"));
    guide.appendChild(jobBox(key));
  } else {
    const actions = el("div", "paper-actions");
    const dl = paperBtn("⬇ Download", () => {
      void pimStartLogDownload(remoteView.name, remoteView.logId, remoteView.position, item.title || "Papers");
    }, "primary");
    actions.appendChild(dl);
    // 取りに行けないときは自分で保存した PDF を入れられるようにする（U05 §5）
    if (typeof pimOpenManual === "function") {
      actions.appendChild(paperBtn("手動で取り込む", () => {
        void pimOpenManual({
          name: remoteView.name,
          target: { log_id: remoteView.logId, position: remoteView.position },
          title: item.title || "Papers",
          siteUrl: item.doi ? doiUrl(item.doi) : "",
        });
      }));
    }
    // 学内ネットワークの Mac のブラウザで開く（rbrowser.js が無くても壊れない）
    if (item.doi && typeof rbOpenHash === "function") {
      actions.appendChild(paperBtn("Mac のブラウザで開く", () => {
        rbOpenHash(remoteView.name, item.doi, { log_id: remoteView.logId, position: remoteView.position });
      }));
    }
    guide.appendChild(actions);
    if (item.doi && typeof rbOpenHash === "function") {
      guide.appendChild(
        el("p", "paper-msg", "学内ネットワークの Mac のブラウザで論文のページを開きます。PDF を保存して Import すると Library に入ります")
      );
    }
    guide.appendChild(
      el("p", "paper-msg", "PDF を取りに行って Library に保存します。数分かかることがあります。")
    );
    // 前の失敗があれば理由と「論文のサイトを開く」を出す
    if (job && job.status === "failed") {
      guide.appendChild(jobBox(key));
      if (typeof pimJobErrorNotFound === "function" && pimJobErrorNotFound(job.error) && item.doi) {
        const a = el("a", "paper-doi", "論文のサイトを開く");
        a.href = doiUrl(item.doi);
        a.target = "_blank";
        a.rel = "noopener";
        guide.appendChild(a);
      }
    }
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
  // PDF が無い論文は、保存した PDF を手で取り込めるようにする（U05 §5）
  if (!files.pdf && typeof pimOpenManual === "function") {
    actions.appendChild(
      paperBtn("PDF を手動で取り込む", () => {
        void pimOpenManual({
          name: paperProject,
          target: { paper_id: paperIdNum },
          title: p.title || "Papers",
          siteUrl: p.doi ? doiUrl(p.doi) : "",
        });
      })
    );
  }
  if (!files.pdf && p.doi && typeof rbOpenHash === "function") {
    actions.appendChild(
      paperBtn("Mac のブラウザで開く", () => {
        rbOpenHash(paperProject, p.doi, { paper_id: paperIdNum });
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
  hints.push("ノート: この論文の覚え書きです。書くとここに残ります");
  for (const text of hints) body.appendChild(el("p", "paper-hint", text));

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
  tab: "papers",
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
  tab: "papers",
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
    return hash === "#/papers/library" ? {} : null;
  },
  view: "view-papers-library",
  tab: "papers",
  hideTabbar: false,
  interval: 0, // 定期更新はしない（↻ で読み直す・07 §4.3）
  async open() {
    await openLibrary();
  },
  async poll() {
    await papersPollExtras();
  },
};

// Discover（検索と最近の検索・08 §4.1）
const discoverRoute = {
  match(hash) {
    return hash === "#/papers/discover" ? {} : null;
  },
  view: "view-papers-discover",
  tab: "papers",
  hideTabbar: false,
  interval: 0, // 定期更新はしない（検索はボタンで行う・08 §4.1）
  async open() {
    await openDiscover();
  },
  async poll() {
    await papersPollExtras();
  },
};

// 過去の検索結果（新旧どちらの住所でも・08 §4.2・18 §3.5）
const logRoute = {
  match(hash) {
    const p = parseLogHash(hash);
    return p && p.position == null ? { id: p.id, name: p.name } : null;
  },
  view: "view-papers-log",
  tab: "papers",
  hideTabbar: false,
  interval: 0,
  async open(params) {
    await openLog(params);
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
  tab: "papers",
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
  tab: "papers",
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
    remote: remoteRoute,
    paper: paperRoute,
    text: paperTextRoute,
  };

  (window.SaiRoutes = window.SaiRoutes || []).push(
    dashboardRoute,
    libraryRoute,
    discoverRoute,
    logRoute,
    remoteRoute,
    paperTextRoute,
    paperRoute
  );

  // Library の道具行に「＋ BibTeX を読み込む」を足す（1 回だけ）
  papersImportButton();

  // 再読み込みボタン（↻）（一覧のプロジェクトも取り直す・18 §1.1）
  const reload = document.getElementById("papers-reload");
  if (reload) reload.addEventListener("click", () => { void openLibrary({ force: true }); });

  // 上部のプロジェクト名（押すと切り替えの一覧が下から出る・18 §1.1）
  const libChip = document.getElementById("library-project");
  if (libChip) libChip.addEventListener("click", () => { void openProjectSheet(); });
  for (const id of ["dashboard-project", "discover-project", "radio-project", "sci-project"]) {
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

  // 一覧 / 階層
  const modeBox = document.getElementById("library-mode");
  if (modeBox) {
    modeBox.querySelectorAll("button").forEach((b) => {
      b.addEventListener("click", () => onModeClick(b.dataset.mode));
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

  // --- Discover の部品（08 §4.1） ---
  // 検索ボタンと検索欄の Enter
  const go = document.getElementById("discover-go");
  if (go) go.addEventListener("click", () => { void runDiscoverSearch(); });
  const dInput = document.getElementById("discover-input");
  if (dInput) {
    dInput.addEventListener("keydown", (e) => {
      if (e.key === "Enter") {
        e.preventDefault();
        void runDiscoverSearch();
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
  // 過去の検索結果の戻る → Discover（08 §4.2）
  const logBack = document.getElementById("log-back");
  if (logBack) logBack.addEventListener("click", () => { location.hash = "#/papers/discover"; });
  // 未ダウンロードの論文情報の戻る → その検索結果（08 §4.4）
  const remoteBack = document.getElementById("remote-back");
  if (remoteBack) {
    remoteBack.addEventListener("click", () => {
      location.hash = remoteView
        ? logHash(remoteView.name, remoteView.logId, null)
        : "#/papers/discover";
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
    migrateTreeFolds,
    migrateDiscoverLast,
    activityHash,
    papersDbLabel,
    papersFailedLines,
    papersAllFailed,
    papersAttentionHash,
    papersDebtCount,
  };
}
