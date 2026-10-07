"use strict";
// sai-web の Papers の Import（#/papers/import・実装計画 U6）。
//
// ここに 2 つがある:
//   1. Import の画面（#view-papers-import・枠は U4 が index.html に用意）
//      BibTeX（.bib）をファイルで送るか貼り付けて読み込み、一覧から論文を選んで
//      PDF を Download できる。
//   2. Download の進み具合の見張り（Import の行・検索結果の論文情報・
//      Library の帯・広い画面の右の欄で使う）
//
// 画面は素の JavaScript。外部ライブラリは使わない。
// papers.js が先に読まれるので、そこの関数（papersApiBase / paperHash /
// selectedPaperProject / ensurePaperProjects / openProjectSheet など）は
// そのまま呼べる。一番外側の名前は pim / PIM_ で始める（C-F6）。
// 通信は ui.js の saiApi、知らせは saiToast / saiConfirm / saiSheet / saiEmpty。
// DOM を使わない関数は末尾で node に渡して tests/test_paperimport.js で確かめる。

// --- 定数 ---

// Download が進んでいるあいだだけ、この間隔で状態を読み直す
const PIM_POLL_MS = 3000;
// 進み具合のログは見せる行数（残りは「残りを見る」で全部出す）
const PIM_PROGRESS_LINES = 5;
// .bib の名前の規則（サーバの papers.py の _BIB_NAME_RE と同じ）
const PIM_BIB_NAME_RE = /^[A-Za-z0-9._-]{1,80}\.bib$/;
// 現況・考えの md がこの行数を超えるときは、はじめはたたんで「全部見る」にする
const PIM_MD_LONG_LINES = 40;
// たたんだときに見せる行数
const PIM_MD_PEEK_LINES = 8;
// 送る PDF の大きさの上限（100MB・サーバの papers.py と同じ）
const PIM_UPLOAD_MAX = 100 * 1024 * 1024;
// 仕事の種類 → 画面に出す表示名（U05 §6。知らない種類は空文字）
const PIM_JOB_KINDS = {
  download: "Download",
  manual: "Manual import",
  "cite-extract": "Citations",
  radio: "Audio",
  translate: "Abstract translation",
};
// 要対応の種類 → [札の文章, 札の色, 1 行の説明]
const PIM_ATTENTION_KINDS = {
  empty_review: ["review", "muted", "Review not written"],
  question: ["Question", "warn", "Question for the user"],
  state_update: ["State", "warn", "Proposed State update"],
  recommend_card: ["Recommend", "ok", "Recommendation card"],
  report_issue: ["Source", "danger", "Source needs checking"],
  // U19: research-tree から出る要対応
  sci_decision: ["NO_STAR", "warn", "Q has no star"],
  sci_triage: ["PROPOSAL", "warn", "PROPOSAL awaiting TRIAGE"],
};

// --- 画面の状態 ---

let pimData = null;        // GET …/papers/import の応答（null＝まだ読んでいない）
let pimError = "";         // 一覧を読めなかったときの文
let pimLoading = false;    // 一覧を読み込み中
let pimSaveError = "";     // 読み込み・削除で弾かれたときの理由（赤い 1 行）
let pimBusy = false;       // 読み込み・削除のあいだ（ボタンを止める）
let pimFilterText = "";    // 一覧の絞り込みの言葉（画面を移っても残す）
let pimPasteText = "";     // 貼り付ける（BibTeX の中身）
let pimPasteName = "";     // 貼り付けるときの名前（空ならサーバーが日付で名付ける）
let pimListBox = null;     // 論文の一覧を入れる箱（絞り込みのときだけ作り直す）
let pimJobs = [];          // GET …/papers/downloads の jobs（新しい順）
let pimJobsName = "";      // pimJobs はどのプロジェクトの分か
let pimJobsBusy = false;   // 状態を読み込み中（重ねて読まない）
let pimRetry = new Map();  // 仕事のキー → 「もう一度」で送り直す値
const pimJobNodes = new Map(); // 仕事のキー → その表示を入れる箱
const pimJobDois = new Map(); // 仕事のキー → その論文の DOI（サイトを開くリンクを出すため）

// --- 共通部品 ui.js（C-F3）の準備 ---

let pimUiPromise = null;   // ui.js を読み込む待ち（1 回だけ）

/**
 * 共通部品（ui.js）が読み込まれていれば何もしない。
 * index.html に `/ui.js` の読み込みが無いときだけ、ここから読み込む
 * （共有契約 C-F2 は U4 の担当。index.html に読み込みが無いときの逃げ道で、
 * 共通部品の上書き（読み直し）はしない）。
 */
function pimEnsureUi() {
  if (typeof saiApi === "function") return Promise.resolve();
  if (pimUiPromise) return pimUiPromise;
  pimUiPromise = new Promise((resolve) => {
    if (typeof document === "undefined" || typeof window === "undefined") {
      resolve();
      return;
    }
    const tag = document.createElement("script");
    tag.src = "/ui.js?v=30";
    tag.async = false;
    tag.addEventListener("load", () => resolve());
    tag.addEventListener("error", () => resolve());
    document.head.appendChild(tag);
  });
  return pimUiPromise;
}

// --- DOM を使わない小さな関数（node のテストからも使う） ---

/** .bib の名前として正しいか（サーバと同じ規則・400 になるものを先に弾く）。 */
function pimValidName(name) {
  return typeof name === "string" && PIM_BIB_NAME_RE.test(name);
}

/**
 * 選んだファイルの名前（規則に合うならその名前・合わなければ空）。
 * 空のときはサーバが `phone-<日時>.bib` を作るので、壊れた名前を送らない。
 */
function pimFileNameFor(fileName) {
  return pimValidName(fileName) ? fileName : "";
}

/** 著者（配列でも文字列でも）を「先頭の 3 人」で 1 行にする。4 人以上は「ほか」を付ける。 */
function pimAuthors(list) {
  let names;
  if (Array.isArray(list)) names = list.map((x) => String(x == null ? "" : x).trim());
  else names = String(list == null ? "" : list).split(",").map((x) => x.trim());
  names = names.filter(Boolean);
  if (names.length === 0) return "";
  const head = names.slice(0, 3).join("、");
  return names.length > 3 ? `${head} ほか` : head;
}

/** 年・雑誌・被引用の 1 行（無ければ空）。 */
function pimMetaLine(entry) {
  const e = entry || {};
  const parts = [];
  if (e.year != null && String(e.year).trim()) parts.push(String(e.year).trim());
  if (e.journal != null && String(e.journal).trim()) parts.push(String(e.journal).trim());
  if (e.citation_count != null && String(e.citation_count) !== "") {
    parts.push(`被引用 ${e.citation_count}`);
  }
  return parts.join(" · ");
}

/** 絞り込みの言葉（空白区切り・大文字小文字は無視）。 */
function pimWords(query) {
  return String(query || "")
    .toLowerCase()
    .split(/\s+/)
    .filter(Boolean);
}

/** 題名・著者にすべての言葉が含まれる論文だけ残す。 */
function pimFilter(entries, query) {
  const words = pimWords(query);
  const list = Array.isArray(entries) ? entries : [];
  if (words.length === 0) return list.slice();
  return list.filter((e) => {
    const x = e || {};
    const hay = `${x.title || ""} ${Array.isArray(x.authors) ? x.authors.join(" ") : x.authors || ""}`.toLowerCase();
    return words.every((w) => hay.includes(w));
  });
}

/** Import の論文の仕事のキー（サーバの `import:<import_id>` と同じ形）。 */
function pimImportKey(importId) {
  return `import:${importId}`;
}

/** 検索結果の論文の仕事のキー（サーバの `log:<log_id>:<position>` と同じ形）。 */
function pimLogKey(logId, position) {
  return `log:${logId}:${position}`;
}

/** 仕事のキーからその仕事を探す（無ければ null）。 */
function pimJobFor(jobs, key) {
  if (!key) return null;
  const list = Array.isArray(jobs) ? jobs : [];
  return list.find((j) => j && j.key === key) || null;
}

/** まだ終わっていない仕事（downloading / processing）。 */
function pimActiveJobs(jobs) {
  const list = Array.isArray(jobs) ? jobs : [];
  return list.filter((j) => j && (j.status === "downloading" || j.status === "processing"));
}

/** 状態の 1 行の文。 */
function pimJobLabel(status) {
  if (status === "downloading") return "ダウンロード中…";
  if (status === "processing") return "保存しました。図の取り出しと翻訳をしています";
  if (status === "done") return "保存しました";
  if (status === "failed") return "保存できませんでした";
  return "";
}

/** 状態の類（帯の色・CSS の class に使う）。 */
function pimJobTone(status) {
  if (status === "downloading" || status === "processing") return "busy";
  if (status === "done") return "ok";
  if (status === "failed") return "danger";
  return "";
}

/** 失敗の理由が「PDF の場所が見つからない」かどうか（論文のサイトを開く案内を出す）。 */
function pimJobErrorNotFound(error) {
  return String(error || "").includes("PDF の場所が見つかりません");
}

/** 要対応の種類 → 1 行の説明（知らない種類は「要対応」）。 */
function pimAttentionKindLabel(kind) {
  const k = PIM_ATTENTION_KINDS[kind];
  return k ? k[2] : "Attention";
}

/** 仕事の種類 → 表示名（知らない種類は空文字・U05 §6）。 */
function pimJobKindLabel(kind) {
  return PIM_JOB_KINDS[kind] || "";
}

// --- 手動で取り込むときの対象（U05 §5） ---
// 対象は 4 通り。キーの形はサーバの `_manual_target` と必ずそろえる。
//   {log_id, position} / {import_id} / {cite_paper_id, ref_no} / {paper_id}

/** 対象からアップロードのクエリ（`?a=1&b=2`）を作る。対象が無ければ空文字。 */
function pimManualQuery(target, title) {
  const t = target || {};
  const parts = [];
  const add = (key, value) => {
    if (value == null || value === "") return;
    parts.push(`${encodeURIComponent(key)}=${encodeURIComponent(value)}`);
  };
  if (t.log_id != null) {
    add("log_id", t.log_id);
    add("position", t.position);
  } else if (t.import_id != null) {
    add("import_id", t.import_id);
  } else if (t.cite_paper_id != null) {
    add("cite_paper_id", t.cite_paper_id);
    add("ref_no", t.ref_no);
  } else if (t.paper_id != null) {
    add("paper_id", t.paper_id);
  } else {
    return "";
  }
  // 題名は画面に出すためだけなので、無ければ付けない（サーバは「論文」にする）
  if (title) add("title", title);
  return parts.length > 0 ? `?${parts.join("&")}` : "";
}

/** 対象から仕事のキー（`manual:log:3:2`・サーバと同じ形）を作る。 */
function pimManualKey(target) {
  const q = pimManualQuery(target);
  if (!q) return "";
  const params = new URLSearchParams(q.slice(1));
  let key = "";
  if (params.has("log_id")) key = `log:${params.get("log_id")}:${params.get("position")}`;
  else if (params.has("import_id")) key = `import:${params.get("import_id")}`;
  else if (params.has("cite_paper_id")) {
    key = `cite:${params.get("cite_paper_id")}:${params.get("ref_no")}`;
  } else if (params.has("paper_id")) key = `paper:${params.get("paper_id")}`;
  if (!key) return "";
  return `manual:${key}`;
}

/**
 * 仕事のキーから取り込み先（対象）を読み戻す（無ければ null）。
 * ダウンロードが失敗したとき「手動で取り込む」を出すのに使う（U05 §5）。
 */
function pimTargetFromKey(key) {
  // 手動取り込みの鍵は `manual:` が付くので外す（中身のキーの形に戻す）
  const text = String(key || "").replace(/^manual:/, "");
  const log = /^log:(\d+):(\d+)$/.exec(text);
  if (log) return { log_id: Number(log[1]), position: Number(log[2]) };
  const cite = /^cite:(\d+):(\d+)$/.exec(text);
  if (cite) return { cite_paper_id: Number(cite[1]), ref_no: Number(cite[2]) };
  const imp = /^import:(.+)$/.exec(text);
  if (imp) return { import_id: imp[1] };
  const paper = /^paper:(\d+)$/.exec(text);
  if (paper) return { paper_id: Number(paper[1]) };
  return null;
}

/**
 * 送る前の検査（`{ok, error}` を返す・U05 §5）。
 * 大きさ 100MB までと、種類（application/pdf か名前が .pdf で終わること）を見る。
 */
function pimCheckUpload(file) {
  if (!file) return { ok: false, error: "ファイルを選んでください" };
  const size = Number(file.size);
  if (!Number.isFinite(size) || size <= 0) {
    return { ok: false, error: "ファイルが空です" };
  }
  if (size > PIM_UPLOAD_MAX) {
    return { ok: false, error: "100MB までの PDF を選んでください" };
  }
  const type = String(file.type || "").toLowerCase();
  const name = String(file.name || "");
  if (type !== "application/pdf" && !name.toLowerCase().endsWith(".pdf")) {
    return { ok: false, error: "PDF を選んでください" };
  }
  return { ok: true, error: "" };
}

// --- 進み具合（段・%・経過時間・ログ・U3 §5） ---

/** 経過時間（`0:12` / `1:02:03`。読めない時刻なら空文字）。 */
function pimElapsedLabel(startIso, endMs) {
  const start = Date.parse(String(startIso || ""));
  if (!Number.isFinite(start)) return "";
  const now = Number.isFinite(Number(endMs)) ? Number(endMs) : Date.now();
  let sec = Math.max(0, Math.round((now - start) / 1000));
  const hours = Math.floor(sec / 3600);
  sec -= hours * 3600;
  const min = Math.floor(sec / 60);
  sec -= min * 60;
  const pad = (n) => String(n).padStart(2, "0");
  return hours > 0 ? `${hours}:${pad(min)}:${pad(sec)}` : `${min}:${pad(sec)}`;
}

/**
 * 1 件の仕事の表示に使う値（`{percent, stepLabel, elapsed, lines, allLines}`）。
 * 画面はここだけを見て描く（DOM を使わないので node のテストからも確かめられる）。
 */
function pimProgressView(job, nowMs) {
  const j = job || {};
  const steps = Array.isArray(j.steps) ? j.steps : [];
  // `percent` が null の仕事は「% の分からない仕事」（画面は動く帯で出す）
  const rawPercent = j.percent;
  const unknown = rawPercent == null;
  let percent = Number(rawPercent);
  if (!Number.isFinite(percent)) percent = 0;
  percent = Math.max(0, Math.min(100, Math.round(percent)));
  const step = Number.isFinite(Number(j.step)) ? Number(j.step) : 0;
  let stepLabel = String(steps[step] || "");
  if (!stepLabel) {
    stepLabel = percent >= 100 ? "完了" : j.status === "downloading" ? "PDF取得" : "";
  }
  const allLines = (Array.isArray(j.messages) ? j.messages : []).map((x) => String(x));
  const end = j.finished_at ? Date.parse(j.finished_at) : Number(nowMs);
  return {
    percent,
    unknown,
    stepLabel,
    elapsed: pimElapsedLabel(j.started_at, end),
    lines: allLines.slice(-PIM_PROGRESS_LINES),
    allLines,
  };
}

/** 長い md かどうかと、たたんだときの少し（{long, lines, peek}）。 */
function pimLongText(text) {
  const lines = String(text == null ? "" : text).split("\n");
  return {
    long: lines.length > PIM_MD_LONG_LINES,
    lines: lines.length,
    peek: lines.slice(0, PIM_MD_PEEK_LINES).join("\n"),
  };
}

// --- 小さな部品 ---

/** 要素を 1 つ作る（el が無いところ用）。 */
function pimEl(tag, cls, text) {
  const node = document.createElement(tag);
  if (cls) node.className = cls;
  if (text != null) node.textContent = text;
  return node;
}

/** 今のプロジェクト名（無ければ空・ papers.js の関数）。 */
function pimProjectName() {
  if (typeof selectedPaperProject === "function") return selectedPaperProject();
  return null;
}

/** Papers の API の住所（papers.js の関数）。 */
function pimApiBase(name) {
  return papersApiBase(name);
}

/** 論文情報の住所（papers.js の関数）。 */
function pimPaperHash(name, id) {
  return paperHash(name, id, false);
}

// --- Import の画面（#/papers/import） ---

/** 画面の上の帯（← Library・題・今のプロジェクト）。 */
function pimHeader() {
  const bar = pimEl("header", "appbar");
  const back = document.createElement("button");
  back.type = "button";
  back.textContent = "‹";
  back.setAttribute("aria-label", "Library に戻る");
  back.addEventListener("click", () => { location.hash = "#/papers/library"; });
  bar.appendChild(back);
  bar.appendChild(pimEl("h1", null, "BibTeX の読み込み"));
  // 今のプロジェクト（押すと papers.js の切り替え一覧が出る）
  const chip = document.createElement("button");
  chip.type = "button";
  chip.id = "import-project";
  chip.className = "papers-proj";
  chip.textContent = `📁 ${pimProjectName() || "—"}`;
  chip.setAttribute("aria-label", "プロジェクトを切り替える");
  chip.addEventListener("click", () => { void openProjectSheet(); });
  bar.appendChild(chip);
  return bar;
}

/** 説明の 2 文。 */
function pimIntro() {
  const box = pimEl("div", "pim-intro");
  const p = document.createElement("p");
  p.appendChild(document.createTextNode("論文サイトの「Cite」などで手に入れた "));
  p.appendChild(saiTerm("bibtex", "BibTeX"));
  p.appendChild(
    document.createTextNode(
      " を読み込むと、下の一覧に出ます。一覧から論文を選ぶと、PDF を取りに行って Library に保存します。"
    )
  );
  box.appendChild(p);
  return box;
}

/** 読み込みの箱（ファイルを選ぶ・貼り付ける・読み込む）。 */
function pimLoadBox() {
  const card = pimEl("section", "sai-card pim-load");

  // 1. ファイルを選ぶ（隠れた input を押す）
  const fileLine = pimEl("div", "pim-load-line");
  const pick = document.createElement("button");
  pick.type = "button";
  pick.className = "sai-btn";
  pick.textContent = "ファイルを選ぶ";
  const input = document.createElement("input");
  input.type = "file";
  input.accept = ".bib,text/plain,application/x-bibtex";
  input.className = "pim-file";
  input.addEventListener("change", () => {
    const file = input.files && input.files[0];
    if (!file) return;
    const reader = new FileReader();
    reader.onload = () => {
      void pimAddBib(pimFileNameFor(file.name), String(reader.result || ""));
    };
    reader.onerror = () => {
      pimSaveError = "ファイルを読めませんでした。もう一度選んでください";
      pimRender();
    };
    reader.readAsText(file);
  });
  pick.addEventListener("click", () => input.click());
  fileLine.appendChild(pick);
  fileLine.appendChild(input);
  fileLine.appendChild(pimEl("span", "pim-note", "iPhone なら「ファイル」から .bib を選べます"));
  card.appendChild(fileLine);

  // 2. 貼り付ける
  const pasteTitle = pimEl("p", "pim-sub", "貼り付ける");
  card.appendChild(pasteTitle);
  const ta = document.createElement("textarea");
  ta.className = "sai-input pim-textarea";
  ta.rows = 5;
  ta.placeholder = "@article{…} の BibTeX を貼り付けます";
  ta.value = pimPasteText;
  ta.addEventListener("input", () => { pimPasteText = ta.value; });
  card.appendChild(ta);
  const nameLine = pimEl("div", "pim-load-line");
  const name = document.createElement("input");
  name.type = "text";
  name.className = "sai-input pim-name";
  name.placeholder = "refs.bib（空でもよい）";
  name.value = pimPasteName;
  name.addEventListener("input", () => { pimPasteName = name.value; });
  const add = document.createElement("button");
  add.type = "button";
  add.className = "sai-btn primary";
  add.textContent = "読み込む";
  add.disabled = pimBusy;
  add.addEventListener("click", () => {
    void pimAddBib(name.value.trim(), ta.value);
  });
  nameLine.appendChild(name);
  nameLine.appendChild(add);
  card.appendChild(nameLine);

  if (pimSaveError) card.appendChild(pimEl("p", "pim-error", pimSaveError));
  return card;
}

/** .bib を 1 つ読み込む（名前が空ならサーバーが日付で名付ける）。 */
async function pimAddBib(name, text) {
  await pimEnsureUi();
  const project = pimProjectName();
  if (!project) return;
  if (pimBusy) return;
  if (!String(text || "").trim()) {
    pimSaveError = "BibTeX の中身が空です";
    pimRender();
    return;
  }
  pimBusy = true;
  pimSaveError = "";
  pimRender();
  try {
    const res = await saiApi(`${pimApiBase(project)}/import`, {
      method: "POST",
      body: { name: name || "", text: String(text) },
    });
    const count = Number(res && res.count) || 0;
    // 貼り付けた本文は消しておく（同じものを二重で送らないように）
    if (!name) {
      pimPasteText = "";
      pimPasteName = "";
    }
    saiToast(`${count} 件を読み込みました`, "ok");
  } catch (e) {
    // 400（同じ名前・壊れた BibTeX など）はそのまま日本語で出す
    pimSaveError = (e && e.message) || "読み込めませんでした";
  } finally {
    pimBusy = false;
  }
  await pimLoadList(project);
  pimRender();
}

/** .bib の一覧と中の論文を読む。 */
async function pimLoadList(name) {
  if (!name) return;
  pimLoading = true;
  try {
    const res = await saiApi(`${pimApiBase(name)}/import`);
    if (pimProjectName() !== name || location.hash !== "#/papers/import") return;
    pimData = res && typeof res === "object" ? res : {};
    pimError = "";
  } catch (e) {
    if (pimProjectName() !== name) return;
    pimData = null;
    pimError = (e && e.message) || "読み込めませんでした";
  } finally {
    pimLoading = false;
  }
}

/** .bib を 1 つ消す（削除の前に確認を出す）。 */
async function pimRemoveBib(fileName) {
  await pimEnsureUi();
  const project = pimProjectName();
  if (!project || pimBusy) return;
  const ok = await saiConfirm({
    title: "この .bib を消しますか",
    body: `${fileName} を消します。中の論文は Library にすでに入っているものは残ります。`,
    ok: "消す",
    danger: true,
  });
  if (!ok) return;
  pimBusy = true;
  pimSaveError = "";
  pimRender();
  try {
    await saiApi(`${pimApiBase(project)}/import/${encodeURIComponent(fileName)}`, {
      method: "DELETE",
    });
    saiToast(`${fileName} を消しました`, "ok");
  } catch (e) {
    pimSaveError = (e && e.message) || "消せませんでした";
  } finally {
    pimBusy = false;
  }
  await pimLoadList(project);
  pimRender();
}

/** 読み込んだ .bib の一覧。 */
function pimFilesCard() {
  const files = Array.isArray(pimData && pimData.files) ? pimData.files : [];
  const card = pimEl("section", "sai-card");
  card.appendChild(pimEl("h2", "pim-title", `読み込んだファイル（${files.length}）`));
  if (files.length === 0) {
    card.appendChild(pimEl("p", "lib-empty", "まだ .bib がありません"));
    return card;
  }
  const list = pimEl("div", "pim-files");
  for (const f of files) {
    const row = pimEl("div", "pim-file-row");
    const main = document.createElement("button");
    main.type = "button";
    main.className = "pim-file-main";
    main.appendChild(pimEl("span", "pim-file-name", String(f.name || "")));
    main.appendChild(pimEl("span", "pim-file-count", `${Number(f.count) || 0} 件`));
    if (f.error) main.appendChild(pimEl("span", "pim-file-error", String(f.error)));
    main.addEventListener("click", () => { void pimRemoveBib(String(f.name || "")); });
    const del = document.createElement("button");
    del.type = "button";
    del.className = "sai-btn small danger";
    del.textContent = "🗑";
    del.setAttribute("aria-label", `${f.name || ""} を消す`);
    del.disabled = pimBusy;
    del.addEventListener("click", () => { void pimRemoveBib(String(f.name || "")); });
    row.appendChild(main);
    row.appendChild(del);
    list.appendChild(row);
  }
  card.appendChild(list);
  card.appendChild(pimEl("p", "pim-note", "🗑 を押すとその .bib を消します"));
  return card;
}

/** 論文の一覧（絞り込みの欄と 1 件ごとの行）。 */
function pimEntriesCard() {
  const entries = Array.isArray(pimData && pimData.entries) ? pimData.entries : [];
  const card = pimEl("section", "sai-card");
  card.appendChild(pimEl("h2", "pim-title", `論文の一覧（${entries.length}）`));
  const filter = document.createElement("input");
  filter.type = "search";
  filter.className = "sai-input pim-filter";
  filter.placeholder = "題名・著者で絞り込み";
  filter.value = pimFilterText;
  filter.addEventListener("input", () => {
    pimFilterText = filter.value || "";
    if (pimListBox) pimFillEntries(pimListBox);
  });
  card.appendChild(filter);
  if (entries.length === 0) {
    card.appendChild(
      saiEmpty({
        icon: "📄",
        title: "まだ BibTeX がありません",
        body: "上の「ファイルを選ぶ」か「貼り付ける」から読み込んでください。",
      })
    );
    return card;
  }
  const list = pimEl("div", "pim-entries");
  card.appendChild(list);
  pimFillEntries(list);
  return card;
}

/** 一覧の中身を_words で絞って描き直す（絞り込み欄は作り直さない）。 */
function pimFillEntries(box) {
  const entries = Array.isArray(pimData && pimData.entries) ? pimData.entries : [];
  const rows = pimFilter(entries, pimFilterText);
  box.innerHTML = "";
  if (rows.length === 0) {
    box.appendChild(pimEl("p", "lib-empty", "一致する論文がありません"));
    return;
  }
  for (const entry of rows) box.appendChild(pimEntryRow(entry));
}

/** 1 論文の行（Library にあれば論文情報、無ければ詳しい中間の板）。 */
function pimEntryRow(entry) {
  const e = entry || {};
  const local = e.local_paper_id != null && e.local_paper_id !== "";
  const row = pimEl("div", "pim-entry");

  const main = document.createElement("button");
  main.type = "button";
  main.className = "pim-entry-main";
  const line1 = pimEl("div", "pim-entry-line1");
  line1.appendChild(pimEl("span", "pim-entry-title", e.title || "(無題)"));
  main.appendChild(line1);
  const authors = pimAuthors(e.authors);
  const meta = [authors, String(e.year || ""), String(e.journal || "")].filter(Boolean).join(" · ");
  if (meta) main.appendChild(pimEl("div", "pim-entry-meta", meta));
  main.addEventListener("click", () => {
    if (local) location.hash = pimPaperHash(pimProjectName(), e.local_paper_id);
    else pimOpenEntrySheet(e);
  });
  row.appendChild(main);

  if (local) {
    // Library にある論文は、そのまま論文情報を開ける
    const chip = document.createElement("button");
    chip.type = "button";
    chip.className = "sai-chip ok";
    chip.textContent = "Library にあります";
    chip.addEventListener("click", () => {
      location.hash = pimPaperHash(pimProjectName(), e.local_paper_id);
    });
    row.appendChild(chip);
  }
  // Download の進み具合（Import の行の下の枠）
  row.appendChild(pimJobSlot(pimImportKey(e.id), e.doi));
  return row;
}

/** 1 論文の詳しい中身（下から出る板）。押した场地 で Download を始める。 */
function pimOpenEntrySheet(entry) {
  const e = entry || {};
  const body = pimEl("div", "pim-sheet");
  body.appendChild(pimEl("h3", "pim-sheet-title", e.title || "(無題)"));
  const authors = pimAuthors(e.authors);
  if (authors) body.appendChild(pimEl("p", "pim-sheet-line", `著者: ${authors}`));
  const meta = pimMetaLine(e);
  if (meta) body.appendChild(pimEl("p", "pim-sheet-line", meta));
  if (e.doi) {
    const a = document.createElement("a");
    a.href = doiUrl(e.doi);
    a.target = "_blank";
    a.rel = "noopener";
    a.textContent = `論文のサイトを開く（DOI: ${e.doi}）`;
    a.className = "pim-sheet-link";
    body.appendChild(a);
  }
  if (e.abstract) {
    body.appendChild(pimEl("p", "pim-sheet-sub", "要旨"));
    body.appendChild(pimEl("div", "pim-sheet-abstract", String(e.abstract)));
  }

  const go = document.createElement("button");
  go.type = "button";
  go.className = "sai-btn primary";
  go.textContent = "⬇ Download";
  go.addEventListener("click", () => {
    sheet.close();
    void pimStartDownload(
      pimProjectName(),
      { import_id: e.id, title: e.title || "Papers" },
      pimImportKey(e.id)
    );
  });
  body.appendChild(go);
  body.appendChild(pimEl("p", "pim-note", "数分かかることがあります。終わったら通知でお知らせします"));

  const sheet = saiSheet({ title: "論文のダウンロード", body });
  return sheet;
}

/** 画面を描く（#/papers/import）。 */
function pimRender() {
  const root = document.getElementById("import-root");
  if (!root) return;
  root.innerHTML = "";
  pimJobNodes.clear(); // 前の画面で使った箱はもう無い
  root.appendChild(pimHeader());
  // Papers の小さなタブの枠（U14。中身は papers.js の fillSegment）
  const seg = pimEl("div", "papers-seg-row");
  seg.id = "import-seg";
  root.appendChild(seg);
  if (typeof fillSegment === "function") fillSegment("import-seg", "import");
  const body = pimEl("div", "pim-body");
  root.appendChild(body);

  const name = pimProjectName();
  if (!name) {
    body.appendChild(
      saiEmpty({
        icon: "📁",
        title: "論文の置き場のあるプロジェクトがありません",
        body: "Mac で sai-paper init を実行してから、もう一度読み込んでください。",
      })
    );
    return;
  }
  if (pimData === null) {
    body.appendChild(pimEl("p", "lib-empty", pimError || "読み込み中…"));
    return;
  }
  body.appendChild(pimIntro());
  body.appendChild(pimLoadBox());
  body.appendChild(pimFilesCard());
  body.appendChild(pimEntriesCard());
  // 壊れた .bib の理由（一番下の赤い行）
  const errors = Array.isArray(pimData.errors) ? pimData.errors.filter((x) => x) : [];
  if (errors.length > 0) {
    const box = pimEl("div", "pim-errors");
    box.appendChild(pimEl("p", "pim-errors-title", `読み込めなかった .bib（${errors.length}）`));
    for (const e of errors) box.appendChild(pimEl("p", "pim-error", String(e)));
    body.appendChild(box);
  }
}

/** Import 画面を開く。 */
async function pimOpenImport() {
  await pimEnsureUi();
  // どのプロジェクトの Library なのかを決める（papers.js の関数）
  if (typeof ensurePaperProjects === "function") await ensurePaperProjects();
  if (location.hash !== "#/papers/import") return;
  const name = pimProjectName();
  if (!name) {
    pimData = null;
    pimError = "";
    pimRender();
    return;
  }
  if (pimData === null) pimRender(); // まず「読み込み中…」
  pimLoading = true;
  await pimLoadList(name);
  // 走っている Download があれば表示する（Import を開いたときだけ 1 回読む）
  if (pimJobsName !== name) pimJobs = [];
  await pimRefreshJobs(name);
  pimRender();
}

// --- Download（依頼・状態の表示・見張り） ---

/** 走っている仕事のあいだだけ 3 秒ごとに読み直すよう、各画面の更新間隔を切り替える。 */
function pimSetPolling(active) {
  const routes = (typeof window !== "undefined" && window.SaiPapersRoutes) || null;
  if (!routes) return;
  for (const def of Object.values(routes)) {
    if (def && typeof def === "object") def.interval = active ? PIM_POLL_MS : 0;
  }
}

/** 走っている仕事の配列（papers.js の帯と Download の表示が使う）。 */
function pimJobsList() {
  return pimJobs;
}

/** Download を依頼する（202 ならその仕事の箱を作り、409 は知らせて止める）。 */
async function pimStartDownload(projectName, payload, key) {
  await pimEnsureUi();
  const project = projectName || pimProjectName();
  if (!project) return null;
  try {
    const res = await saiApi(`${pimApiBase(project)}/download`, { method: "POST", body: payload });
    const jobKey = (res && res.key) || key;
    // 失敗したときに「もう一度」で同じ値を送れるように覚えておく
    pimRetry.set(jobKey, payload);
    pimUpsertJob({
      key: jobKey,
      title: (payload && payload.title) || "Papers",
      status: "downloading",
      paper_id: null,
      error: null,
    });
    pimSetPolling(true);
    pimUpdateJobNodes();
    if (typeof papersRenderDownloadBand === "function") papersRenderDownloadBand();
    return jobKey;
  } catch (e) {
    const status = e && e.status;
    if (status === 409) saiToast((e && e.message) || "この論文はダウンロード中です", "error");
    else saiToast((e && e.message) || "ダウンロードの依頼はできませんでした", "error");
    return null;
  }
}

/** 仕事の配列へ 1 件を入れる（同じキーは上書き）。 */
function pimUpsertJob(job) {
  if (!job || !job.key) return;
  const rest = pimJobs.filter((j) => j && j.key !== job.key);
  pimJobs = [job, ...rest];
}

/** 仕事の表示を入れる箱を作る（キーが変わっても描き直せるように覚えておく）。 */
function pimJobSlot(key, doi) {
  const box = pimEl("div", "pim-job");
  if (doi) pimJobDois.set(key, doi);
  pimJobNodes.set(key, box);
  pimFillJobBox(box, key);
  return box;
}

/** 1 つの箱に、その仕事の状態を描く。 */
function pimFillJobBox(box, key) {
  const job = pimJobFor(pimJobs, key);
  box.innerHTML = "";
  box.hidden = !job;
  if (!job) return;
  box.appendChild(pimJobBlock(job, pimJobDois.get(key)));
}

/** すべての仕事の箱を描き直す（状態が変わったときに 1 回だけ呼ぶ）。 */
function pimUpdateJobNodes() {
  for (const [key, box] of pimJobNodes) pimFillJobBox(box, key);
}

/** 1 仕事の表示（Import の行・論文情報の下の板・広い画面の欄で使う）。 */
function pimJobBlock(job, doi) {
  const wrap = pimEl("div", `pim-job-body ${pimJobTone(job.status)}`);
  const line = pimEl("div", "pim-job-line");
  if (job.status === "downloading") {
    const spin = pimEl("span", "papers-spin");
    line.appendChild(spin);
  }
  line.appendChild(pimEl("span", "pim-job-text", pimJobLabel(job.status)));
  wrap.appendChild(line);

  // 登録できたら論文情報を開ける（processing / done）
  if ((job.status === "processing" || job.status === "done") && job.paper_id != null) {
    const open = document.createElement("button");
    open.type = "button";
    open.className = "sai-btn small";
    open.textContent = "論文情報を開く";
    open.addEventListener("click", () => {
      location.hash = pimPaperHash(pimProjectName() || pimJobsName, job.paper_id);
    });
    wrap.appendChild(open);
  }
  if (job.error) {
    // done の error は「後の処理」の失敗。failed は保存できなかったこと。
    const text =
      job.status === "done" ? `後の処理で失敗: ${job.error}` : String(job.error);
    wrap.appendChild(pimEl("p", "pim-job-error", text));
  }
  if (job.status === "failed") {
    const again = document.createElement("button");
    again.type = "button";
    again.className = "sai-btn small";
    again.textContent = "もう一度";
    again.addEventListener("click", () => {
      const payload = pimRetry.get(job.key);
      if (!payload) return;
      pimRetry.delete(job.key);
      pimUpsertJob({ key: job.key, title: job.title, status: "downloading", paper_id: null, error: null });
      pimUpdateJobNodes();
      void pimStartDownload(pimProjectName() || pimJobsName, payload, job.key);
    });
    wrap.appendChild(again);
    // 取り込めなかったものは、ブラウザや保存した PDF から手で入れられる（U05 §5）
    const target = pimTargetFromKey(job.key);
    if (target) {
      const manual = document.createElement("button");
      manual.type = "button";
      manual.className = "sai-btn small";
      manual.textContent = "手動で取り込む";
      manual.addEventListener("click", () => {
        void pimOpenManual({
          name: pimProjectName() || pimJobsName,
          target,
          title: job.title || "Papers",
        });
      });
      wrap.appendChild(manual);
    }
    // 論文のサイトを開く（PDF の場所が見つからないとき）
    const linkDoi = doi || job.doi;
    if (pimJobErrorNotFound(job.error) && linkDoi) {
      const a = pimEl("a", "pim-job-link", "論文のサイトを開く（PDF を手で取る）");
      a.href = doiUrl(linkDoi);
      a.target = "_blank";
      a.rel = "noopener";
      wrap.appendChild(a);
    }
  }
  return wrap;
}

/** 検索結果の 1 論文から Download を依頼する（renderRemote から使う）。 */
function pimStartLogDownload(projectName, logId, position, title) {
  return pimStartDownload(
    projectName,
    { log_id: logId, position, title },
    pimLogKey(logId, position)
  );
}

/** 状態を読み直す（Download が走っているあいだだけ 3 秒ごと）。 */
async function pimPollJobs() {
  if (!pimActiveJobs(pimJobs).length) return; // 走っているものが無ければ読まない
  const name = pimProjectName() || pimJobsName;
  if (!name) return;
  await pimRefreshJobs(name);
}

/** GET …/papers/downloads を読む（どの画面からでも 1 本で済む）。 */
async function pimRefreshJobs(name) {
  await pimEnsureUi();
  const project = name || pimProjectName();
  if (!project || pimJobsBusy) return;
  pimJobsBusy = true;
  const beforeKeys = new Set(
    pimJobs.filter((j) => j && (j.status === "done" || j.status === "failed")).map((j) => j.key)
  );
  try {
    const res = await saiApi(`${pimApiBase(project)}/downloads`);
    if (pimProjectName() && pimProjectName() !== project) return;
    const jobs = res && Array.isArray(res.jobs) ? res.jobs : [];
    pimJobs = jobs;
    pimJobsName = project;
    const fresh = jobs.filter((j) => j && (j.status === "done" || j.status === "failed") && !beforeKeys.has(j.key));
    pimUpdateJobNodes();
    if (typeof papersRenderDownloadBand === "function") papersRenderDownloadBand();
    if (typeof papersRenderJobBoxes === "function") papersRenderJobBoxes();
    // 保存できた論文が Library に増えたので取り直す
    if (fresh.some((j) => j.status === "done" && j.paper_id != null)) {
      if (typeof loadLibrary === "function" && location.hash === "#/papers/library") void loadLibrary();
      if (typeof pimRender === "function" && location.hash === "#/papers/import") pimRender();
    }
    // 走っているものが無くなったら 3 秒ごと更新をやめる
    pimSetPolling(pimActiveJobs(pimJobs).length > 0);
  } catch {
    // つながらないときは黙る（次の 3 秒で読み直す）
  } finally {
    pimJobsBusy = false;
  }
}

// --- 手動で取り込む一枚（U05 §5） ---
// 2 つの選び方を 1 枚にまとめる:
//   1. Mac のダウンロードフォルダにある最近の PDF（名前をcli 側に検証させる）
//   2. この端末（iPhone でも PC でも）から PDF の本文をそのまま送る

/**
 * 手動取り込みの一枚を開く。
 *   {name, target, title, siteUrl}
 *   - `target` は {log_id, position} / {import_id} / {cite_paper_id, ref_no} / {paper_id}
 *   - `siteUrl` は論文のサイト（無ければ「論文のサイトを開く」は出さない）
 */
async function pimOpenManual(opts) {
  await pimEnsureUi();
  const o = opts || {};
  const name = o.name || pimProjectName();
  const target = o.target || {};
  const query = pimManualQuery(target, o.title || "");
  if (!name || !query) {
    saiToast("取り込む対象が分かりません", "error");
    return null;
  }
  const body = document.createElement("div");
  body.className = "pim-manual";
  const sheet = saiSheet({ title: "PDF を手動で取り込む", body });

  // 一枚の中の状態（描き直すたびに読む）
  const state = {
    candidates: null,     // GET manual/candidates の応答（null = 読めていない）
    candidatesError: "",  // 候補を読めなかったときの理由
    chosen: "",           // ダウンロードフォルダから選んだ PDF の名前
    file: null,           // この端末から選ぶたファイル
    fileError: "",        // 選んだファイルが通らなかったときの理由
    busy: false,          // 取り込みの依頼を送っているあいだ
    error: "",            // 依頼の失敗（赤字。一枚は閉じない）
  };

  const draw = () => {
    body.innerHTML = "";
    if (o.title) body.appendChild(pimEl("h3", "pim-sheet-title", o.title));

    // 1. 論文のサイトで PDF を保存する
    const step1 = pimEl("div", "pim-manual-step");
    step1.appendChild(pimEl("p", "pim-manual-step-label", "1. 論文のサイトで PDF を保存します"));
    if (o.siteUrl) {
      const link = document.createElement("a");
      link.href = o.siteUrl;
      link.target = "_blank";
      link.rel = "noopener";
      link.className = "sai-btn";
      link.textContent = "論文のサイトを開く";
      step1.appendChild(link);
    } else {
      step1.appendChild(pimEl("p", "pim-note", "論文のサイトの URL が分かりませんでした"));
    }
    body.appendChild(step1);

    // 2. 保存した PDF を選ぶ（下の 2 つのうちのどちらか一方）
    const step2 = pimEl("div", "pim-manual-step");
    step2.appendChild(pimEl("p", "pim-manual-step-label", "2. 保存した PDF を選びます"));
    body.appendChild(step2);
    body.appendChild(pimCandidatesBox(state, draw, load));
    body.appendChild(pimUploadBox(state, draw, submit));

    if (state.error) body.appendChild(pimEl("p", "pim-error", state.error));
    if (state.busy) {
      body.appendChild(pimEl("p", "pim-note", "取り込みを始めています…"));
    } else if (!state.chosen && !state.file) {
      body.appendChild(pimEl("p", "pim-note", "候補から選ぶか、ファイルを選んでください"));
    }
  };

  // 候補を一度だけ読む
  async function load() {
    state.candidatesError = "";
    try {
      const res = await saiApi(`${pimApiBase(name)}/manual/candidates`);
      state.candidates = res && typeof res === "object" ? res : {};
    } catch (e) {
      state.candidates = null;
      state.candidatesError = (e && e.message) || "候補を読めませんでした";
    }
    draw();
  }

  /** 取り込みの依頼を送る（ファイル名か本文のどちらか一方）。 */
  async function submit() {
    if (state.busy) return;
    state.busy = true;
    state.error = "";
    draw();
    try {
      if (state.file) {
        // 本文をそのまま送る（UploadFile には python-multipart が要るので使わない）
        const url = `${pimApiBase(name)}/manual/upload${query}`;
        const res = await apiFetch(url, {
          method: "POST",
          headers: { "Content-Type": "application/pdf" },
          body: state.file,
        });
        const json = await res.json().catch(() => ({}));
        if (!res.ok) throw new Error(json.detail || "PDF を送れませんでした");
        if (window.SaiShell && typeof window.SaiShell.ok === "function") window.SaiShell.ok();
      } else {
        await saiApi(`${pimApiBase(name)}/manual/import`, {
          method: "POST",
          body: { ...(target || {}), title: o.title || "Papers", file_name: state.chosen },
        });
      }
      sheet.close();
      saiToast("取り込みを始めました", "ok");
      await pimRefreshJobs(name);
      pimSetPolling(true);
    } catch (e) {
      state.busy = false;
      state.error = (e && e.message) || "取り込みの依頼はできませんでした";
      if (window.SaiShell && typeof window.SaiShell.fail === "function" &&
          /つなが/.test(String(state.error))) {
        window.SaiShell.fail();
      }
    }
    draw();
  }

  await draw();
  await load();
  return sheet;
}

/** ダウンロードフォルダの候補（読み直しの ↻ ボタンつき）。 */
function pimCandidatesBox(state, draw, load) {
  const box = pimEl("section", "pim-manual-box");
  const head = pimEl("div", "pim-manual-head");
  const res = state.candidates || {};
  const minutes = Number(res.minutes) || 30;
  head.appendChild(
    pimEl("p", "pim-manual-head-title", `Mac のダウンロードフォルダ（直近 ${minutes} 分）`)
  );
  const reload = document.createElement("button");
  reload.type = "button";
  reload.className = "sai-btn small";
  reload.textContent = "↻";
  reload.setAttribute("aria-label", "候補を読み直す");
  reload.addEventListener("click", () => { void load(); });
  head.appendChild(reload);
  box.appendChild(head);

  if (state.candidates === null) {
    box.appendChild(pimEl("p", "pim-note", state.candidatesError || "候補を読み込み中…"));
    return box;
  }
  const items = Array.isArray(res.items) ? res.items : [];
  if (items.length === 0) {
    box.appendChild(
      pimEl(
        "p",
        "pim-note",
        `〜${res.dir || "~/Downloads"} に直近 ${minutes} 分の PDF がありません。保存してから ↻ を押してください`
      )
    );
    return box;
  }
  const list = pimEl("div", "pim-manual-list");
  for (const item of items) {
    const name = String((item || {}).name || "");
    if (!name) continue;
    const row = pimEl("label", "pim-manual-row");
    const radio = document.createElement("input");
    radio.type = "radio";
    radio.name = "pim-manual-candidate";
    radio.checked = state.chosen === name;
    radio.addEventListener("change", () => {
      state.chosen = name;
      state.file = null; // 候補とファイルは片方だけ選べる
      state.fileError = "";
      draw();
    });
    row.appendChild(radio);
    row.appendChild(pimEl("span", "pim-manual-name", name));
    row.appendChild(pimEl("span", "pim-manual-size", pimFileSize(item.size)));
    row.appendChild(pimEl("span", "pim-manual-time", pimAgoLabel(item.modified_at)));
    list.appendChild(row);
  }
  box.appendChild(list);
  return box;
}

/** この端末から PDF を送る欄（input type=file・accept は application/pdf）。 */
function pimUploadBox(state, draw, submit) {
  const box = pimEl("section", "pim-manual-box");
  box.appendChild(pimEl("p", "pim-manual-head-title", "または、この端末から PDF を送る"));
  const line = pimEl("div", "pim-manual-line");
  const input = document.createElement("input");
  input.type = "file";
  input.accept = "application/pdf,.pdf";
  input.className = "pim-manual-file";
  input.addEventListener("change", () => {
    const file = input.files && input.files[0];
    if (!file) return;
    const check = pimCheckUpload(file);
    if (!check.ok) {
      state.file = null;
      state.fileError = check.error;
    } else {
      state.file = file;
      state.fileError = "";
      state.chosen = ""; // 候補とファイルは片方だけ選べる
    }
    draw();
  });
  line.appendChild(input);
  const go = document.createElement("button");
  go.type = "button";
  go.className = "sai-btn primary";
  go.textContent = state.busy ? "取り込み中…" : "取り込む";
  go.disabled = state.busy || (!state.chosen && !state.file);
  go.addEventListener("click", () => { void submit(); });
  line.appendChild(go);
  box.appendChild(line);
  if (state.fileError) box.appendChild(pimEl("p", "pim-error", state.fileError));
  box.appendChild(pimEl("p", "pim-note", "100MB までの PDF なら iPhone からも送れます"));
  return box;
}

/** 大きさ（`1.2MB` などに丸める）。読めなければ空文字。 */
function pimFileSize(bytes) {
  const n = Number(bytes);
  if (!Number.isFinite(n) || n <= 0) return "";
  if (n < 1024) return `${n}B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)}KB`;
  return `${(n / 1024 / 1024).toFixed(1)}MB`;
}

/** 更新時刻から「3分前」などに（読めなければ空文字）。 */
function pimAgoLabel(iso) {
  const at = Date.parse(String(iso || ""));
  if (!Number.isFinite(at)) return "";
  const sec = Math.max(0, Math.round((Date.now() - at) / 1000));
  if (sec < 90) return `${sec}秒前`;
  const min = Math.round(sec / 60);
  if (min < 60) return `${min}分前`;
  const hour = Math.round(min / 60);
  if (hour < 24) return `${hour}時間前`;
  return `${Math.round(hour / 24)}日前`;
}

// --- 進み具合の一枚（どのページからでも開く・U3 §5） ---

/**
 * 仕事の題名を押したときの行き先（U05 §6）。
 * - 引用の抜き出しは、その論文の引用一覧
 * - 音声化は再生画面（音声の名前が分からなければ論文情報）
 * - それ以外は論文情報
 */
function pimJobTarget(job, name) {
  const j = job || {};
  if (j.paper_id == null || !name) return "";
  const paper = `#/paper/${encodeURIComponent(name)}/${encodeURIComponent(j.paper_id)}`;
  if (j.kind === "cite-extract") return `${paper}/cites`;
  if (j.kind === "radio") {
    if (typeof paperProjectByName === "function") {
      const proj = paperProjectByName(name);
      if (proj && proj.root_name) {
        return `#/radio/${encodeURIComponent(`${proj.root_name}-${j.paper_id}`)}`;
      }
    }
  }
  return paper;
}

// --- 画面の登録（ブラウザでのみ） ---

if (typeof window !== "undefined") {
  (window.SaiRoutes = window.SaiRoutes || []).push({
    match(hash) {
      return hash === "#/papers/import" ? {} : null;
    },
    view: "view-papers-import",
    tab: "papers",
    hideTabbar: false,
    // 0 のあいだは更新しない。Download が走っているあいだだけ 3 秒になる（pimSetPolling）
    interval: 0,
    async open() {
      await pimOpenImport();
    },
    async poll() {
      await pimPollJobs();
    },
  });
}

// --- node からの読み込み（tests/test_paperimport.js。DOM を使わない部分だけ） ---
if (typeof module !== "undefined" && module.exports) {
  module.exports = {
    PIM_POLL_MS,
    PIM_BIB_NAME_RE,
    PIM_MD_LONG_LINES,
    PIM_MD_PEEK_LINES,
    PIM_ATTENTION_KINDS,
    PIM_UPLOAD_MAX,
    PIM_JOB_KINDS,
    pimValidName,
    pimFileNameFor,
    pimAuthors,
    pimMetaLine,
    pimWords,
    pimFilter,
    pimImportKey,
    pimLogKey,
    pimJobFor,
    pimActiveJobs,
    pimJobLabel,
    pimJobTone,
    pimJobErrorNotFound,
    pimAttentionKindLabel,
    pimJobKindLabel,
    pimManualQuery,
    pimManualKey,
    pimTargetFromKey,
    pimCheckUpload,
    pimFileSize,
    pimAgoLabel,
    pimJobTarget,
    pimLongText,
    // ブラウザの画面からも使う（papers.js の帯・Download の表示）
    pimJobsList,
    pimJobBlock,
    pimJobSlot,
    pimProgressView,
    pimElapsedLabel,
  };
}
