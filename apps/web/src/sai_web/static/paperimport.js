"use strict";
// sai-web の Download の見張り（実装計画 U6・U06）。
//
// Import の画面は無い（論文の取り込みは各自の Mac の sai-prep で行う・U06）。ここにあるのは
// Download の進み具合の見張りだけ（論文情報・引用の画面・Library の帯・Dashboard で使う）。
// 古い #/papers/import の住所は Library へ送る（下の登録）。
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
// 現況・考えの md がこの行数を超えるときは、はじめはたたんで「全部見る」にする
const PIM_MD_LONG_LINES = 40;
// たたんだときに見せる行数
const PIM_MD_PEEK_LINES = 8;
// --- 画面の状態 ---

let pimJobs = [];          // GET …/papers/downloads の jobs（新しい順）
let pimJobsName = "";      // pimJobs はどのプロジェクトの分か
let pimJobsBusy = false;   // 状態を読み込み中（重ねて読まない）

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

/** 状態の 1 行の文（仕事の種類で「後処理」の文を変える）。 */
function pimJobLabel(status, kind) {
  if (status === "downloading") return "取り込み中…";
  if (status === "processing") {
    if (kind === "bundle") return "前処理の束を取り込んでいます";
    return "保存しました。図の取り出しと翻訳をしています";
  }
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

// --- Library に入れる（U13 §2） ---

/** keep の結果（`added` / `exists`）→ 画面に知らせる文。 */
function pimKeepMessage(status) {
  if (status === "exists") return "もう Library にあります";
  return "Library に入れました（PDF は sai-prep から送ってください）";
}

// --- 進み具合（段・%・経過時間・ログ・U3 §5） ---

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

/** Library に入れる（`POST …/papers/keep`・すぐ終わる・U13 §2）。 */
async function pimStartKeep(projectName, payload, key) {
  await pimEnsureUi();
  const project = projectName || pimProjectName();
  if (!project) return null;
  try {
    const res = await saiApi(`${pimApiBase(project)}/keep`, { method: "POST", body: payload });
    const status = res && res.status;
    saiToast(pimKeepMessage(status), "ok");
    // 一覧を「Library にあります」に更新する
    if (typeof loadLibrary === "function" && location.hash === "#/papers/library") void loadLibrary();
    return (res && res.paper_id) || key || null;
  } catch (e) {
    saiToast((e && e.message) || "Library に入れられませんでした", "error");
    return null;
  }
}

/** 古い名前（cite.js が呼ぶ）。中身は Library に入れるに変わった（U13 §2）。 */
function pimStartDownload(projectName, payload, key) {
  return pimStartKeep(projectName, payload, key);
}

/** 1 仕事の表示（論文情報・引用の画面で使う）。 */
function pimJobBlock(job, doi) {
  const wrap = pimEl("div", `pim-job-body ${pimJobTone(job.status)}`);
  const line = pimEl("div", "pim-job-line");
  if (job.status === "downloading") {
    const spin = pimEl("span", "papers-spin");
    line.appendChild(spin);
  }
  line.appendChild(pimEl("span", "pim-job-text", pimJobLabel(job.status, job.kind)));
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

/** 検索結果の 1 論文から Library に入れる（renderRemote から使う）。 */
function pimStartLogKeep(projectName, logId, position, title) {
  return pimStartKeep(
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
    if (typeof papersRenderDownloadBand === "function") papersRenderDownloadBand();
    if (typeof papersRenderJobBoxes === "function") papersRenderJobBoxes();
    // 保存できた論文が Library に増えたので取り直す
    if (fresh.some((j) => j.status === "done" && j.paper_id != null)) {
      if (typeof loadLibrary === "function" && location.hash === "#/papers/library") void loadLibrary();
    }
    // 走っているものが無くなったら 3 秒ごと更新をやめる
    pimSetPolling(pimActiveJobs(pimJobs).length > 0);
  } catch {
    // つながらないときは黙る（次の 3 秒で読み直す）
  } finally {
    pimJobsBusy = false;
  }
}
// --- 画面の登録（ブラウザでのみ） ---

// 古い #/papers/import の住所は Library へ送る（Import の画面は U06 で無くした）
if (typeof window !== "undefined") {
  (window.SaiRoutes = window.SaiRoutes || []).push({
    match(hash) {
      return hash === "#/papers/import" ? {} : null;
    },
    view: "view-papers-library",
    tab: "research",
    hideTabbar: false,
    interval: 0,
    open() {
      location.replace("#/papers/library");
    },
  });
}

// --- node からの読み込み（tests/test_paperimport.js。DOM を使わない部分だけ） ---
if (typeof module !== "undefined" && module.exports) {
  module.exports = {
    PIM_POLL_MS,
    PIM_MD_LONG_LINES,
    PIM_MD_PEEK_LINES,
    pimLogKey,
    pimJobFor,
    pimActiveJobs,
    pimJobLabel,
    pimJobTone,
    pimJobErrorNotFound,
    pimKeepMessage,
    pimLongText,
    // ブラウザの画面からも使う（papers.js の帯・Download の表示）
    pimJobsList,
    pimJobBlock,
  };
}
