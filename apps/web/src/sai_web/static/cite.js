"use strict";
// sai-web の引用一覧の画面（#/paper/<name>/<id>/cites・実装計画 U5 §4）。
//
// sai-paper-cite（Mac の TUI）の引用一覧を iPhone と PC へ移したもの。
// ここでできること:
//   - 題名・著者・年で絞り込む（TUI の filtered_indices と同じ）
//   - Library にある引用には緑的点と「Library の論文情報を開く」
//   - 行Details の「本文での引用」（CITED IN）に、その引用を引用している文を出す
//   - 引用の抜き出し（docling を使うので数十秒〜数分かかる・仕事として進む）
//   - 引用のダウンロード・論文のサイトを開く・手動で取り込む（paperimport.js）
//
// 中身はすべて /api/projects/<name>/papers/… 経由（sai-web 自身は
// sai-paper の DB を開かない・00 §2）。
// 一番外側の名前は ct / CT_ で始める（C-F6・test_globals.js が重なりを見る）。
// papers.js と paperimport.js より**後**に読み込む（それらの関数を呼ぶため）。
// DOM を使わない関数は末尾で node に渡して tests/test_cite.js で確かめる。

// --- 定数 ---

// 抜き出しのあいだだけ、この間隔で状態を読み直す
const CT_POLL_MS = 2000;
// 引用文の見せる行数（多いときは 100 文字で切る・CLI 側でも 300 字まで）
const CT_SENTENCE_MAX = 300;
// 参考になったリンクを探すときのサイト（cite TUI の do_open_pdf と同じ）
const CT_SCHOLAR = "https://scholar.google.com/scholar?q=";

// --- 画面の状態 ---

let ctData = null;      // GET …/citations の応答（null＝まだ読んでいない）
let ctError = "";       // 読めなかったときの文
let ctName = "";        // 選んでいるプロジェクト名
let ctPaperId = null;   // 見ている論文の id
let ctFilterText = "";  // 絞り込みの言葉
let ctSelected = null;  // 広い画面で右の欄に出している ref_no
let ctListBox = null;   // 一覧を入れる箱（絞り込みのときだけ描き直す）
let ctExtractBusy = false; // 抜き出しの依頼を送っているあいだ
// 走っている抜き出しがあれば画面を 2 秒ごとに読み直す（下の画面登録で切り替える）
let ctRouteDef = null;
// 広い画面の右の欄（画面を開いたときだけ作る）
let ctSideBox = null;

// --- DOM を使わない小さな関数（node のテストからも使う） ---

/**
 * 引用一覧の住所を読む（`#/paper/<name>/<id>/cites`）。
 * 見つからなければ null。
 *
 * なぜ cite.js の中で持つのか: 同じ 3 段の住所は `parsePaperHash` には
 * 「3 段目は text だけ」と書いてある通りなので、`cites` を読んでも
 * papers.js の論文情報・本文のどちらにも当たらない（route の並び順是靠いない）。
 * そのため cite.js は最後に読み込む読み順のままでよい。
 */
function ctParseHash(hash) {
  const m = /^#\/paper\/([^/?#]+)\/(\d+)\/cites$/.exec(String(hash || ""));
  if (!m) return null;
  return { name: decodeURIComponent(m[1]), id: m[2] };
}

/** 引用一覧の住所を書く。 */
function ctHash(name, id) {
  return `#/paper/${encodeURIComponent(name)}/${encodeURIComponent(id)}/cites`;
}

/** 引用が Library にあるか（`local_paper_id` が 0 でないこと）。 */
function ctInLibrary(ref) {
  const id = (ref || {}).local_paper_id;
  return id != null && id !== "" && Number(id) !== 0;
}

/** 絞り込み（cite TUI の filtered_indices と同じ：大文字小文字を無視した部分一致）。 */
function ctFilter(refs, query) {
  const list = Array.isArray(refs) ? refs : [];
  const f = String(query || "").trim().toLowerCase();
  if (!f) return list.slice();
  return list.filter((r) => {
    const x = r || {};
    const title = String(x.title || "").toLowerCase();
    const author = String(x.first_author || "").toLowerCase();
    const year = x.year == null ? "" : String(x.year);
    const raw = String(x.raw_text || "").toLowerCase();
    return title.includes(f) || author.includes(f) || year.includes(f) || raw.includes(f);
  });
}

/** 引用文の 1 行（[番号] 本文）。300 字で切る。 */
function ctSentenceText(sentence) {
  const s = sentence || {};
  const text = String(s.text == null ? "" : s.text).trim();
  const body = text.length > CT_SENTENCE_MAX ? `${text.slice(0, CT_SENTENCE_MAX)}…` : text;
  return `[${s.sent_no}] ${body}`;
}

/** 引用 1 件の 2 行目（著者・年・掲載誌・本文での引用回数）。 */
function ctMetaLine(ref) {
  const r = ref || {};
  const parts = [];
  if (r.first_author) parts.push(String(r.first_author));
  if (r.year != null && String(r.year).trim()) parts.push(String(r.year));
  if (r.journal) parts.push(String(r.journal));
  const n = Array.isArray(r.sentences) ? r.sentences.length : 0;
  parts.push(n > 0 ? `本文で ${n} 回` : "本文では引用なし");
  return parts.join(" · ");
}

/** 開くリンク（`url` があればそれ、無ければ題名の Google Scholar 検索）。 */
function ctSiteUrl(ref) {
  const r = ref || {};
  const url = String(r.url || "").trim();
  if (url) return url;
  if (r.doi) {
    const doi = String(r.doi).trim();
    return /^https?:\/\//i.test(doi) ? doi : `https://doi.org/${doi}`;
  }
  const title = String(r.title || "").trim();
  return title ? `${CT_SCHOLAR}${encodeURIComponent(title)}` : "";
}

/** 抜き出しが動く仕事（`cite:<id>`）を jobs から見つけ、なければ null。 */
function ctExtractJob(jobs, paperId) {
  const list = Array.isArray(jobs) ? jobs : [];
  return list.find((j) => j && j.key === `cite:${paperId}`) || null;
}

/** その論文の引用が抜かれているか（一覧を出せるか）。 */
function ctExtracted(data) {
  return !!(data && data.extracted);
}

// --- 画面を描くための小さな部品 ---

/** 要素を 1 つ作る（papers.js の el と同じ形）。 */
function ctEl(tag, cls, text) {
  const node = document.createElement(tag);
  if (cls) node.className = cls;
  if (text != null) node.textContent = text;
  return node;
}

/** 今のプロジェクト名（無ければ空・papers.js の関数）。 */
function ctProjectName() {
  if (typeof selectedPaperProject === "function") return selectedPaperProject();
  return null;
}

/** Papers の API の住所（papers.js の関数）。 */
function ctApiBase(name) {
  return papersApiBase(name);
}

/** 共通部品（ui.js）。無ければ読み込む（paperimport.js と同じ作り）。 */
function ctEnsureUi() {
  if (typeof pimEnsureUi === "function") return pimEnsureUi();
  return Promise.resolve();
}

/** 共通部品の要素を作る（無ければ要素の自作に落とす）。 */
function ctUiEl(tag, cls, text) {
  if (typeof saiEl === "function") return saiEl(tag, cls, text);
  return ctEl(tag, cls, text);
}

/** 空の案内（無ければ要素の自作に落とす）。 */
function ctEmpty(opts) {
  if (typeof saiEmpty === "function") return saiEmpty(opts);
  const box = ctEl("div", "sai-empty");
  if (opts && opts.title) box.appendChild(ctEl("p", "sai-empty-title", opts.title));
  if (opts && opts.body) box.appendChild(ctEl("p", "sai-empty-body", opts.body));
  return box;
}

// --- 画面の上の帯 ---

function ctHeader() {
  const bar = ctEl("header", "appbar");
  const back = document.createElement("button");
  back.type = "button";
  back.textContent = "‹";
  back.setAttribute("aria-label", "論文情報に戻る");
  back.addEventListener("click", () => {
    location.hash = `#/paper/${encodeURIComponent(ctName)}/${encodeURIComponent(ctPaperId)}`;
  });
  bar.appendChild(back);
  const title = (ctData && ctData.paper && ctData.paper.title) || "";
  const h1 = ctEl("h1", null, "引用一覧");
  h1.title = title;
  bar.appendChild(h1);
  const reload = document.createElement("button");
  reload.type = "button";
  reload.textContent = "↻";
  reload.setAttribute("aria-label", "読み直す");
  reload.addEventListener("click", () => { void ctOpenCites({ name: ctName, id: ctPaperId }); });
  bar.appendChild(reload);
  return bar;
}

// --- 一覧 ---

/** 絞り込みの欄（画面を移っても残す）。 */
function ctFilterBox() {
  const box = ctEl("div", "ct-filter-box");
  const input = document.createElement("input");
  input.type = "search";
  input.className = "sai-input ct-filter";
  input.placeholder = "題名・著者・年で絞り込み";
  input.value = ctFilterText;
  input.setAttribute("aria-label", "引用を絞り込む");
  input.addEventListener("input", () => {
    ctFilterText = input.value || "";
    if (ctListBox) ctFillList(ctListBox);
  });
  box.appendChild(input);
  const total = Array.isArray(ctData && ctData.refs) ? ctData.refs.length : 0;
  const shown = ctFilter(ctData && ctData.refs, ctFilterText).length;
  box.appendChild(ctEl("p", "ct-count", `${shown} 件`));
  return box;
}

/** 一覧の行を描く（絞り込みの箱を作り直さない）。 */
function ctFillList(box) {
  const refs = Array.isArray(ctData && ctData.refs) ? ctData.refs : [];
  const rows = ctFilter(refs, ctFilterText);
  box.innerHTML = "";
  if (rows.length === 0) {
    box.appendChild(ctEl("p", "lib-empty", "一致する引用がありません"));
    return;
  }
  for (const ref of rows) box.appendChild(ctRow(ref));
}

/** 引用の 1 行（[番号] 題名 / 著者・年・本文での引用回数）。 */
function ctRow(ref) {
  const r = ref || {};
  const local = ctInLibrary(r);
  const row = ctEl("div", `ct-row${local ? " in-library" : ""}`);
  const btn = document.createElement("button");
  btn.type = "button";
  btn.className = "ct-row-main";
  const line1 = ctEl("div", "ct-line1");
  line1.appendChild(ctEl("span", "ct-no", `[${r.ref_no}]`));
  if (local) {
    const dot = ctEl("span", "ct-dot");
    dot.title = "Library にあります";
    line1.appendChild(dot);
  }
  line1.appendChild(ctEl("span", "ct-title", r.title || "(無題)"));
  btn.appendChild(line1);
  btn.appendChild(ctEl("div", "ct-meta", ctMetaLine(r)));
  btn.addEventListener("click", () => ctSelect(r));
  row.appendChild(btn);
  return row;
}

// --- 詳細（狭い画面は一枚の下・広い画面は右の欄） ---

/** 行を選ぶ（狭い画面なら一枚を開く・広い画面なら右の欄に出す）。 */
function ctSelect(ref) {
  const r = ref || {};
  ctSelected = r.ref_no;
  if (typeof saiIsWide === "function" && saiIsWide() && ctSideBox) {
    ctFillSide(r);
    ctHighlight();
    return;
  }
  const body = ctEl("div", "ct-detail");
  ctFillDetail(body, r);
  if (typeof saiSheet === "function") saiSheet({ title: "引用の詳細", body });
}

/** 広い画面で选中されている行に印を付ける。 */
function ctHighlight() {
  const root = document.getElementById("cites-root");
  if (!root) return;
  for (const node of root.querySelectorAll(".ct-row")) {
    const no = node.querySelector(".ct-no");
    if (no && ctSelected != null && no.textContent === `[${ctSelected}]`) {
      node.classList.add("selected");
    } else {
      node.classList.remove("selected");
    }
  }
}

/**
 * 一覧のコンテナの横に「選んだ引用の詳細」の欄を作る（広い画面だけ）。
 * 一覧（listBox）を左の欄に入れ、右の欄（HTMLElement）を返す。
 * 狭い画面では null（そのときは一枚の下から出す）。
 */
function ctSidePane(listBox) {
  if (typeof saiIsWide !== "function" || !saiIsWide()) return null;
  if (ctSideBox && ctSideBox.isConnected) return ctSideBox;
  if (!listBox || !listBox.parentNode) return null;
  const wrap = ctEl("div", "sai-split cites-split");
  const list = ctEl("div", "sai-split-list");
  const side = ctEl("div", "sai-split-main ct-side");
  wrap.appendChild(list);
  wrap.appendChild(side);
  listBox.parentNode.insertBefore(wrap, listBox);
  list.appendChild(listBox); // 一覧は左の欄の中に入れる（住所は変えない）
  ctSideBox = side;
  return side;
}

function ctFillSide(ref) {
  if (!ctSideBox) return;
  ctSideBox.innerHTML = "";
  if (!ref) {
    ctSideBox.appendChild(
      ctEmpty({
        icon: "🔍",
        title: "引用を選んでください",
        body: "左の一覧から選ぶと、ここに詳細が出ます。",
      })
    );
    return;
  }
  const body = ctEl("div", "ct-detail");
  ctFillDetail(body, ref);
  ctSideBox.appendChild(body);
}

/** 詳細の中身（題名・著者・年・掲載誌・DOI / 本文での引用 / 原文 / ボタン）。 */
function ctFillDetail(body, ref) {
  const r = ref || {};
  body.appendChild(ctEl("h3", "ct-detail-title", r.title || "(無題)"));
  const meta = [r.first_author, r.year, r.journal]
    .map((x) => (x == null ? "" : String(x).trim()))
    .filter(Boolean)
    .join(" · ");
  if (meta) body.appendChild(ctEl("p", "ct-detail-meta", meta));
  if (r.doi) {
    const a = ctEl("a", "ct-detail-doi", `DOI: ${r.doi}`);
    a.href = typeof doiUrl === "function" ? doiUrl(r.doi) : `https://doi.org/${r.doi}`;
    a.target = "_blank";
    a.rel = "noopener";
    body.appendChild(a);
  }

  // 本文での引用（CITED IN）
  const sentences = Array.isArray(r.sentences) ? r.sentences : [];
  body.appendChild(
    ctEl("p", "ct-detail-sub", `本文での引用（${sentences.length}）`)
  );
  if (sentences.length === 0) {
    body.appendChild(ctEl("p", "lib-empty", "本文での引用は見つかりませんでした"));
  } else {
    const box = ctEl("div", "ct-sentences");
    for (const s of sentences) {
      box.appendChild(ctEl("p", "ct-sentence", ctSentenceText(s)));
    }
    body.appendChild(box);
  }

  // 原文の引用文（小さい字でたたむ）
  if (r.raw_text) {
    const details = document.createElement("details");
    details.className = "ct-raw";
    const summary = document.createElement("summary");
    summary.textContent = "引用の原文";
    details.appendChild(summary);
    details.appendChild(ctEl("p", "ct-raw-text", String(r.raw_text)));
    body.appendChild(details);
  }

  // ボタン
  const actions = ctEl("div", "ct-actions");
  if (ctInLibrary(r)) {
    const open = ctUiEl("button", "sai-btn primary", "Library の論文情報を開く");
    open.type = "button";
    open.addEventListener("click", () => {
      location.hash = `#/paper/${encodeURIComponent(ctName)}/${encodeURIComponent(r.local_paper_id)}`;
    });
    actions.appendChild(open);
  } else if (r.can_download) {
    const dl = ctUiEl("button", "sai-btn primary", "＋ Library に入れる");
    dl.type = "button";
    dl.addEventListener("click", () => {
      if (typeof pimStartDownload !== "function") return;
      void pimStartDownload(
        ctName,
        { cite_paper_id: ctPaperId, ref_no: r.ref_no, title: r.title || "Papers" },
        `cite:${ctPaperId}:${r.ref_no}`
      );
    });
    actions.appendChild(dl);
  }
  const site = ctSiteUrl(r);
  if (site) {
    const a = ctUiEl("a", "sai-btn", "論文のサイトを開く");
    a.href = site;
    a.target = "_blank";
    a.rel = "noopener";
    actions.appendChild(a);
  }
  // Library にある引用には出さない（取り込むと既存の論文の PDF を置き換えてしまうため）
  if (r.can_import && !ctInLibrary(r)) {
    const manual = ctUiEl("button", "sai-btn", "手動で取り込む");
    manual.type = "button";
    manual.addEventListener("click", () => {
      if (typeof pimOpenManual !== "function") return;
      void pimOpenManual({
        name: ctName,
        target: { cite_paper_id: ctPaperId, ref_no: r.ref_no },
        title: r.title || "Papers",
        siteUrl: site,
      });
    });
    actions.appendChild(manual);
  }
  body.appendChild(actions);

  // 取りに行っている／失敗したcite があれば今の pimJobBlock と同じ表示を出す
  if (typeof pimJobBlock === "function" && typeof pimJobsList === "function") {
    const job = pimJobFor(pimJobsList(), `cite:${ctPaperId}:${r.ref_no}`);
    if (job) body.appendChild(pimJobBlock(job, r.doi || ""));
  }
}

// --- 抜き出しが動くときと、引用が無いときの案内 ---

/** 引用の抜き出しの進み具合（動いているあいだだけ出す）。 */
function ctExtractProgress(job) {
  const box = ctEl("div", "ct-extract");
  const line = ctEl("div", "ct-extract-line");
  const steps = Array.isArray(job.steps) ? job.steps : [];
  const step = Number(job.step) || 0;
  line.appendChild(ctEl("span", "ct-extract-step", steps[step] || "引用を抜けています"));
  const percent = Number(job.percent);
  const bar = ctEl("div", "pim-progress-bar");
  if (Number.isFinite(percent)) {
    const fill = ctEl("div", "pim-progress-fill");
    fill.style.width = `${Math.max(0, Math.min(100, percent))}%`;
    bar.appendChild(fill);
  } else {
    bar.classList.add("pim-progress-bar-working");
  }
  box.appendChild(line);
  box.appendChild(bar);
  if (job.error) box.appendChild(ctEl("p", "pim-progress-error", String(job.error)));
  return box;
}

/** `citations/extract` を依頼する。 */
async function ctRequestExtract() {
  if (ctExtractBusy || ctPaperId == null) return;
  ctExtractBusy = true;
  ctError = "";
  ctRender();
  try {
    await saiApi(`${ctApiBase(ctName)}/${encodeURIComponent(ctPaperId)}/citations/extract`, {
      method: "POST",
    });
    if (typeof pimRefreshJobs === "function") await pimRefreshJobs(ctName);
  } catch (e) {
    ctError = (e && e.message) || "引用を抜けませんでした";
    if (typeof window !== "undefined" && window.SaiShell && window.SaiShell.fail) {
      window.SaiShell.fail();
    }
  } finally {
    ctExtractBusy = false;
  }
  ctRender();
}

/** 引用がまだ抜けていないとき（PDF が無い・arXiv ID や DOI が無いを含む）。 */
function ctNotExtractedBox() {
  const box = ctEl("section", "sai-card ct-empty-card");
  const data = ctData || {};
  if (data.key == null) {
    box.appendChild(
      ctEmpty({
        icon: "🔍",
        title: "arXiv ID か DOI が無いので抜き出せません",
        body: "論文情報の DOI から取り込み直すと、引用を抜けます。",
      })
    );
    return box;
  }
  if (!data.has_pdf) {
    box.appendChild(
      ctEmpty({
        icon: "📄",
        title: "PDF がありません",
        body: "先に PDF を取り込んでください。取り込めれば引用を抜けます。",
      })
    );
    const b = ctUiEl("button", "sai-btn primary", "PDF を手動で取り込む");
    b.type = "button";
    b.addEventListener("click", () => {
      if (typeof pimOpenManual !== "function") return;
      void pimOpenManual({
        name: ctName,
        target: { paper_id: ctPaperId },
        title: (data.paper && data.paper.title) || "Papers",
      });
    });
    box.appendChild(b);
    return box;
  }
  box.appendChild(
    ctEmpty({
      icon: "🔍",
      title: "まだ引用を抜き出していません",
      body: "PDF の本文から引用のリストと、その引用している文を抜きます。数十秒から数分かかることがあります。",
    })
  );
  const b = ctUiEl("button", "sai-btn primary", ctExtractBusy ? "抜き出し中です…" : "引用を抜き出す");
  b.type = "button";
  b.disabled = ctExtractBusy;
  b.addEventListener("click", () => { void ctRequestExtract(); });
  box.appendChild(b);
  return box;
}

// --- 画面を描く ---

/** 画面を描く（`#cites-root`）。 */
function ctRender() {
  const root = document.getElementById("cites-root");
  if (!root) return;
  root.innerHTML = "";
  root.appendChild(ctHeader());
  const host = ctEl("main", "cites-body");
  root.appendChild(host);

  if (!ctName) {
    host.appendChild(
      ctEmpty({
        icon: "📁",
        title: "論文の置き場のあるプロジェクトがありません",
        body: "Mac で sai-paper init を実行してから、もう一度読み込んでください。",
      })
    );
    return;
  }
  if (ctData === null) {
    host.appendChild(ctEl("p", "lib-empty", ctError || "読み込み中…"));
    return;
  }

  // 抜き出しのあいだは一覧の代わりに進み具合を出す
  const jobs = typeof pimJobsList === "function" ? pimJobsList() : [];
  const job = ctExtractJob(jobs, ctPaperId);
  ctSetPolling(!!(job && job.status === "processing"));
  if (job && job.status === "processing") {
    host.appendChild(ctExtractProgress(job));
    if (!ctExtracted(ctData)) return;
  }
  if (job && job.status === "failed" && !ctExtracted(ctData)) {
    host.appendChild(ctExtractProgress(job));
    return;
  }

  if (!ctExtracted(ctData)) {
    host.appendChild(ctNotExtractedBox());
    return;
  }

  const filter = ctFilterBox();
  const list = ctEl("div", "ct-list");
  ctListBox = list;
  host.appendChild(filter);
  // 一覧を箱に入れてから右の欄を作る（広い画面だけ。ctSidePane は親のある要素が要る）
  host.appendChild(list);
  const side = ctSidePane(list);
  ctFillList(list);
  if (side) {
    const refs = Array.isArray(ctData.refs) ? ctData.refs : [];
    // 広い画面では最初の一件を右の欄に出しておく（選ぶまでは案内を出さない）
    const shown = refs.find((r) => r && Number(r.ref_no) === Number(ctSelected)) || refs[0] || null;
    if (shown && shown.ref_no != null) ctSelected = shown.ref_no;
    ctFillSide(shown);
    ctHighlight();
  }
  if (!ctRefsEmpty(ctData)) {
    host.appendChild(ctEl("p", "pim-note", "行を押すと詳細が出ます（引用している文も見られます）"));
  }
}

/** 引用が 1 件も無いとき用の補助（空なら案内を出す）。 */
function ctRefsEmpty(data) {
  const refs = Array.isArray(data && data.refs) ? data.refs : [];
  return refs.length === 0;
}

/** 走っている抜き出しがあれば 2 秒ごとの読み直しに切り替える（0 なら止める）。 */
function ctSetPolling(active) {
  if (!ctRouteDef) return;
  ctRouteDef.interval = active ? CT_POLL_MS : 0;
}

// --- 画面を開く・状態を読み直す ---

/** 引用一覧を読む（202 で抜き出しを始めた直後は仕事だけを読む）。 */
async function ctLoad(force) {
  if (!ctName || ctPaperId == null) return;
  const hash = location.hash;
  try {
    const data = await saiApi(`${ctApiBase(ctName)}/${encodeURIComponent(ctPaperId)}/citations`);
    if (location.hash !== hash) return;
    ctData = data && typeof data === "object" ? data : {};
    ctError = "";
  } catch (e) {
    if (location.hash !== hash) return;
    ctData = null;
    ctError = (e && e.message) || "読み込めませんでした";
  }
  if (typeof pimRefreshJobs === "function") await pimRefreshJobs(ctName);
  ctRender();
}

/** 画面を開く（住所の読み・引用の読み・仕事の読み）。 */
async function ctOpenCites(params) {
  await ctEnsureUi();
  const p = params || {};
  const hash = location.hash;
  // 住所に名前があれば「選んでいるプロジェクト」を切り替える（papers.js の関数）
  const name = typeof projectForAddress === "function"
    ? await projectForAddress(p.name)
    : (p.name || ctProjectName());
  if (location.hash !== hash) return;
  ctName = name || "";
  ctPaperId = p.id != null ? p.id : null;
  ctSelected = null;
  ctSideBox = null;
  ctListBox = null;
  ctFilterText = "";
  ctData = null;
  ctError = "";
  ctRender();
  await ctLoad();
}

/** 定期更新（抜き出しが走っているあいだだけ 2 秒ごと）。 */
async function ctPoll() {
  const jobs = typeof pimJobsList === "function" ? pimJobsList() : [];
  const job = ctExtractJob(jobs, ctPaperId);
  if (job && (job.status === "processing")) {
    await ctLoad();
    return;
  }
  // 抜き出しが終わった直後（引用出境に増えた）を取り直す
  if (ctData === null) {
    await ctLoad();
    return;
  }
  if (job && job.status === "done" && !ctExtracted(ctData)) {
    await ctLoad();
  }
}

// --- 画面の登録（citation.js は index.html で paperimport.js の後に読む） ---

if (typeof window !== "undefined") {
  ctRouteDef = {
    match(hash) {
      return ctParseHash(hash);
    },
    view: "view-paper-cites",
    tab: "research",
    hideTabbar: false,
    // 0 のあいだは更新しない。抜き出しのあいだだけ 2 秒になる（ctSetPolling）
    interval: 0,
    async open(params) {
      await ctOpenCites(params);
    },
    async poll() {
      await ctPoll();
    },
  };
  (window.SaiRoutes = window.SaiRoutes || []).push(ctRouteDef);
}

// --- node からの読み込み（tests/test_cite.js。DOM を使わない部分だけ） ---
if (typeof module !== "undefined" && module.exports) {
  module.exports = {
    CT_POLL_MS,
    CT_SENTENCE_MAX,
    CT_SCHOLAR,
    ctParseHash,
    ctHash,
    ctInLibrary,
    ctFilter,
    ctSentenceText,
    ctMetaLine,
    ctSiteUrl,
    ctExtractJob,
    ctExtracted,
  };
}
