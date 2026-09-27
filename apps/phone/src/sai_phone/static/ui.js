"use strict";
// sai-phone の共通部品（実装計画 U4・共有契約 C-F3）。
//
// 端末（ターミナル）に慣れていない人が iPhone と PC のブラウザだけで
// sai-link / sai-paper を使えるように、画面からすぐ使う部品を 1 か所にまとめたもの。
// ここで定義するものは全部「一番外側の sai / SAI_ で始まる名前」（C-F6）。
// 外部ライブラリは使わない（docs/integration.md §3）。
//
// 用意するもの:
//   saiApi        … JSON を取る。つながらないときは日本語の Error を投げる
//   saiToast      … 画面の下（タブバーの上）に 3 秒だけ出す短い知らせ
//   saiConfirm    … 「そうですか？」の小窓（window.confirm は使わない）
//   saiPrompt     … 1 行の入力の小窓
//   saiSheet      … 下から出る板（狭い画面）/ 右の欄（広い画面）
//   saiEmpty      … 空の画面の案内
//   saiTerm       … 用語の札。押すとその語の説明が出る
//   saiIsWide     … 画面が広い（900px 以上）かどうか
//   saiSplitShow  … .sai-split の中の見せる側を切り替える（狭い画面だけ）
//
// 重ね板（sai-overlay）は document.body の最後に 1 つだけ置き、全体で使い回す。
// 呼び出し側は何もしなくてよい（Esc とマスクを押したときの処理をここで受ける）。
//
// 画面ファイル（app.js より前）から読み込む。node からの読み込みは tests/test_ui.js
// （DOM を使わない部分＝用語の表と「広いか」の境目だけ）。

// --- 画面が広いと思う境目（900px 以上） ---
const SAI_WIDE_PX = 900;

/**
 * 用語の一覧（C-F5 の key 全部）。
 * label は札に出す既定の表示名、text はその語の説明（1〜3 文・日本語）。
 */
const SAI_TERMS = {
  run: {
    label: "実行",
    text:
      "エージェントが 1 つのカードを 1 回動かした記録です。" +
      "［会話］のタブにはプロジェクト名・#番号・実行者の順に並びます。" +
      "「#54 の実行」のように番号で表します。",
  },
  todo: {
    label: "未実行",
    text: "まだ実行していないカードの枚数です。プロジェクトの画面から実行できます。",
  },
  running: {
    label: "実行中",
    text: "いま動かしているカードの枚数です。動いているあいだは［会話］のタブにも出ます。",
  },
  review: {
    label: "確認待ち",
    text:
      "エージェントが仕事を終えたあと、人に確認を待つことです。" +
      "ここに来ているものは読んで「これでよい」を押すと次に進みます。",
  },
  done: {
    label: "確認済み",
    text: "人が読んで確認したぶんです。あとから中身を書き換えることはしません。",
  },
  workflow: {
    label: "Workflow",
    text: "プロジェクトごとにカードが今どうなっているかを見る盤面です。［プロジェクト］の画面の「Workflow」で開きます。",
  },
  kanban: {
    label: "Kanban",
    text: "カードを「doing → done → review」のように段で分けた一覧です。Workflow で横並びに出ます。",
  },
  block: {
    label: "ブロック",
    text: "1 つのカードが・どのファイルを見て・どう直せばよいかを書いた指示書です。",
  },
  template: {
    label: "テンプレート",
    text: "新しいカードを作るときの出発点になる書き方です。プロジェクトごとに直せます。",
  },
  settings: {
    label: "設定ファイル",
    text: "エージェントの動くしかた（どのモデルを使うか・何個同時に動かすかなど）を書いたファイルです。",
  },
  runner: {
    label: "動かすエージェント",
    text: "カードを動かす AI です。cc（Claude Code）や pi など、名前で選びます。",
  },
  round: {
    label: "ラウンド",
    text: "プロジェクトごとに 1 周まわす仕事です。何周したかが「第 N ラウンド」になります。",
  },
  global_round: {
    label: "グローバルラウンド",
    text: "すべてのプロジェクトを 1 周まわす仕事です。プロジェクトごとのラウンドを束ねる上位の仕事です。",
  },
  waggle: {
    label: "文書のトークン",
    text:
      "エージェントが保存した文書の合言葉です。次のカードの依頼文に書くと、その文書の本文がそのまま渡されます。" +
      "（例: AXANSMpo）",
  },
  daemon: {
    label: "常駐",
    text:
      "プロジェクトごとに Mac で動き続ける係です。止まっているとカードを実行できません。" +
      "プロジェクトの画面（Dashboard）から起動できます。",
  },
  card: {
    label: "カード",
    text: "依頼文（task.md）を 1 枚のファイルに書いたものです。仕事 1 つを 1 枚に書いています。",
  },
  task_text: {
    label: "依頼文",
    text: "カードを実行するときの依頼文です。エージェントはこれを読んで動きます。",
  },
  phase: {
    label: "フェーズ",
    text: "1 ラウンドのなかを「作る」と「確かめる」の 2 つに区切る区切りです。",
  },
  bibtex: {
    label: "BibTeX",
    text: "論文の著者・題名・発行年などを書いた文字の書き方です。論文をまとめて読み込むときに使います。",
  },
  attention: {
    label: "要対応",
    text:
      "論文のリーダーが、まだ人が見ていないものを出した状態です。" +
      "「書いていない review」「ユーザーへの質問」「出典の要確認」など。",
  },
  state: {
    label: "現況 state.md",
    text: "プロジェクトがいまどうなっているかをエージェントが継続的に書いているメモ（state.md）です。",
  },
  mind: {
    label: "考え mind.md",
    text: "これまでの経緯と、次にやりたいことをエージェントが継続的に書いているメモ（mind.md）です。",
  },
};

// --- 重ね板（saiConfirm / saiPrompt / saiSheet で使い回す 1 枚） ---

/** 重ね板を作る（2 回目以降は同じ物を使う）。 */
function saiOverlay() {
  let ov = document.getElementById("sai-overlay");
  if (ov) return ov;
  ov = document.createElement("div");
  ov.id = "sai-overlay";
  ov.className = "sai-overlay";
  ov.hidden = true;
  // マスク（押すと閉じる）と中身を入れる箱。これしか作らない。
  ov.innerHTML = '<div class="sai-mask" data-sai-close></div><div class="sai-panel"></div>';
  document.body.appendChild(ov);
  return ov;
}

/** 重ね板を開ける。中の箱を返して、中身を呼び出し側に入れさせる。 */
function saiOverlayOpen(panelClass) {
  const ov = saiOverlay();
  const panel = ov.querySelector(".sai-panel");
  panel.className = panelClass ? `sai-panel ${panelClass}` : "sai-panel";
  panel.innerHTML = "";
  ov.hidden = false;
  return panel;
}

/** 重ね板を閉じる（中身は空にする）。 */
function saiOverlayShut() {
  const ov = document.getElementById("sai-overlay");
  if (!ov) return;
  ov.hidden = true;
  const panel = ov.querySelector(".sai-panel");
  if (panel) {
    panel.className = "sai-panel";
    panel.innerHTML = "";
  }
}

// いま開いている重ね板の「閉じる方法」（閉じるときの値を渡す）
let saiOverlayCloser = null;

/** 重ね板を閉じる。value は閉じるときの値（confirm は true/false、prompt は string|null）。 */
function saiOverlayEnd(value) {
  const fn = saiOverlayCloser;
  saiOverlayCloser = null;
  saiOverlayShut();
  if (typeof fn === "function") fn(value);
}

// マスクと Esc を受ける（document に 1 回だけ付ける）
if (typeof document !== "undefined" && typeof window !== "undefined" && !window.__saiUiOverlay) {
  window.__saiUiOverlay = true;
  document.addEventListener("click", (e) => {
    const t = e.target;
    if (!t || typeof t.closest !== "function") return;
    if (t.closest("[data-sai-close]")) saiOverlayEnd(null);
  });
  document.addEventListener("keydown", (e) => {
    if (e.key !== "Escape") return;
    const ov = document.getElementById("sai-overlay");
    if (ov && !ov.hidden) saiOverlayEnd(null);
  });
}

// --- 小さい DOM を作る手 ---

/** 要素を 1 つ作る（class は空白で分けた文字列、text は textContent）。 */
function saiEl(tag, cls, text) {
  const el = document.createElement(tag);
  if (cls) el.className = cls;
  if (text != null) el.textContent = text;
  return el;
}

// --- 通信 ---

/**
 * 通信して JSON を返す。
 *   - HTTP の失敗は Error（message = サーバーの detail の日本語、.status = 状態番号）
 *   - つながらない（fetch の TypeError）ときは SaiShell.fail() を呼び、Error("Mac につながりません")
 *   - 応答が来たら SaiShell.ok() を呼ぶ
 * options は fetch と同じ（body が物なら JSON にして Content-Type を付ける）。
 */
async function saiApi(path, options) {
  const opts = { ...(options || {}) };
  const body = opts.body;
  const isForm = typeof FormData !== "undefined" && body instanceof FormData;
  if (body != null && typeof body !== "string" && !isForm) {
    opts.body = JSON.stringify(body);
    opts.headers = { ...(opts.headers || {}), "Content-Type": "application/json" };
  }
  let res;
  try {
    res = await apiFetch(path, opts);
  } catch {
    // fetch 自体 通らない ＝ Mac につながっていない
    if (window.SaiShell && typeof window.SaiShell.fail === "function") window.SaiShell.fail();
    throw new Error("Mac につながりません");
  }
  if (!res.ok) {
    let detail = "";
    try {
      const text = await res.text();
      if (text) {
        try {
          const j = JSON.parse(text);
          detail = j.detail || j.message || text;
        } catch {
          detail = text; // JSON でなければそのまま日本語として見せる
        }
      }
    } catch { /* 読めなくても下の既定の文でよい */ }
    const err = new Error(detail || `読み込めませんでした（${res.status}）`);
    err.status = res.status;
    throw err;
  }
  if (window.SaiShell && typeof window.SaiShell.ok === "function") window.SaiShell.ok();
  if (res.status === 204) return null;
  return await res.json();
}

// --- 画面が広いか ---

/** 幅（px）が「広い」と見る境目。node のテストからも読めるよう関数にしておく。 */
function saiWideAt(width) {
  const w = Number(width);
  return Number.isFinite(w) && w >= SAI_WIDE_PX;
}

/** 画面の幅が 900px 以上か。 */
function saiIsWide() {
  return saiWideAt(typeof window === "undefined" ? 0 : window.innerWidth);
}

// --- 知らせ（3 秒だけ下に出る） ---

/** 画面の下（タブバーの上）に 3 秒出す。kind は "info" / "ok" / "error"。 */
function saiToast(text, kind) {
  if (text == null || text === "") return;
  let box = document.getElementById("sai-toasts");
  if (!box) {
    box = document.createElement("div");
    box.id = "sai-toasts";
    box.className = "sai-toasts";
    box.setAttribute("aria-live", "polite");
    document.body.appendChild(box);
  }
  box.appendChild(saiEl("div", `sai-toast ${kind || "info"}`, String(text)));
  const nodes = box.querySelectorAll(".sai-toast");
  const last = nodes[nodes.length - 1];
  setTimeout(() => last.remove(), 3000);
}

// --- 確認の小窓 ---

/**
 * 確認の小窓。{title, body, ok = "OK", cancel = "やめる", danger = false}
 * → Promise<boolean>。window.confirm は使わない。
 */
function saiConfirm(opts) {
  const o = opts || {};
  return new Promise((resolve) => {
    const panel = saiOverlayOpen(o.danger ? "sai-panel-danger" : "");
    saiOverlayCloser = (v) => resolve(v === true);
    if (o.title) panel.appendChild(saiEl("h2", "sai-panel-title", o.title));
    if (o.body) panel.appendChild(saiEl("p", "sai-panel-body", o.body));
    const row = saiEl("div", "sai-panel-row");
    const cancel = saiEl("button", "sai-btn ghost", o.cancel || "やめる");
    cancel.type = "button";
    cancel.addEventListener("click", () => saiOverlayEnd(false));
    const ok = saiEl("button", `sai-btn ${o.danger ? "danger" : "primary"}`, o.ok || "OK");
    ok.type = "button";
    ok.addEventListener("click", () => saiOverlayEnd(true));
    row.appendChild(cancel);
    row.appendChild(ok);
    panel.appendChild(row);
    ok.focus();
  });
}

// --- 入力の小窓 ---

/**
 * 1 行の入力の小窓。{title, body, value = "", placeholder = "", ok = "OK"}
 * → Promise<string | null>（やめたら null）。
 */
function saiPrompt(opts) {
  const o = opts || {};
  return new Promise((resolve) => {
    const panel = saiOverlayOpen("");
    saiOverlayCloser = (v) => resolve(typeof v === "string" ? v : null);
    if (o.title) panel.appendChild(saiEl("h2", "sai-panel-title", o.title));
    if (o.body) panel.appendChild(saiEl("p", "sai-panel-body", o.body));
    const input = document.createElement("input");
    input.type = "text";
    input.className = "sai-input";
    input.value = o.value || "";
    if (o.placeholder) input.placeholder = o.placeholder;
    // Enter で確定（重ね板の Esc とは別）
    input.addEventListener("keydown", (e) => {
      if (e.key === "Enter") {
        e.preventDefault();
        saiOverlayEnd(input.value);
      }
    });
    panel.appendChild(input);
    const row = saiEl("div", "sai-panel-row");
    const cancel = saiEl("button", "sai-btn ghost", "やめる");
    cancel.type = "button";
    cancel.addEventListener("click", () => saiOverlayEnd(null));
    const ok = saiEl("button", "sai-btn primary", o.ok || "OK");
    ok.type = "button";
    ok.addEventListener("click", () => saiOverlayEnd(input.value));
    row.appendChild(cancel);
    row.appendChild(ok);
    panel.appendChild(row);
    input.focus();
    input.select();
  });
}

// --- 空の画面 ---

/**
 * 空の画面の案内。{icon, title, body, action: {label, onClick} | null} → HTMLElement。
 */
function saiEmpty(opts) {
  const o = opts || {};
  const box = document.createElement("div");
  box.className = "sai-empty";
  if (o.icon) box.appendChild(saiEl("div", "sai-empty-icon", o.icon));
  if (o.title) box.appendChild(saiEl("p", "sai-empty-title", o.title));
  if (o.body) box.appendChild(saiEl("p", "sai-empty-body", o.body));
  if (o.action && o.action.label) {
    const btn = saiEl("button", "sai-btn primary", o.action.label);
    btn.type = "button";
    if (typeof o.action.onClick === "function") btn.addEventListener("click", o.action.onClick);
    box.appendChild(btn);
  }
  return box;
}

// --- 用語 ---

/** 用語 key の説明文（無ければ空）。 */
function saiTermText(key) {
  return Object.prototype.hasOwnProperty.call(SAI_TERMS, key) ? SAI_TERMS[key].text : "";
}

/** 用語 key の既定の表示名（無ければ空）。 */
function saiTermLabel(key) {
  const t = SAI_TERMS[key];
  return t && t.label ? t.label : "";
}

/** 用語一覧（[{key, label, text}]）。テストと説明の画面で使う。 */
function saiTermList() {
  return Object.keys(SAI_TERMS).map((key) => ({
    key,
    label: saiTermLabel(key),
    text: saiTermText(key),
  }));
}

/**
 * 用語の札。key は C-F5 の用語、label を省くと既定の表示名。
 * 押すと説明の小窓が出る。→ HTMLElement（span）
 */
function saiTerm(key, label) {
  const span = document.createElement("span");
  span.className = "sai-term";
  span.setAttribute("role", "button");
  span.tabIndex = 0;
  const title = label || saiTermLabel(key) || key;
  span.textContent = title;
  const open = () => {
    const panel = saiOverlayOpen("");
    saiOverlayCloser = () => {};
    panel.appendChild(saiEl("h2", "sai-panel-title", title));
    panel.appendChild(saiEl("p", "sai-panel-body", saiTermText(key) || "この語の説明はまだありません。"));
    const row = saiEl("div", "sai-panel-row");
    const ok = saiEl("button", "sai-btn primary", "閉じる");
    ok.type = "button";
    ok.addEventListener("click", () => saiOverlayEnd(null));
    row.appendChild(ok);
    panel.appendChild(row);
    ok.focus();
  };
  span.addEventListener("click", open);
  span.addEventListener("keydown", (e) => {
    if (e.key === "Enter" || e.key === " ") {
      e.preventDefault();
      open();
    }
  });
  return span;
}

// --- 板（狭い画面は下から / 広い画面は右から） ---

/**
 * 下から出る板（狭い画面）/ 右の欄（広い画面）。
 * {title, body: HTMLElement, onClose} → {close(), el}。
 */
function saiSheet(opts) {
  const o = opts || {};
  const panel = saiOverlayOpen("sai-panel-sheet");
  let closed = false;
  const close = () => {
    if (closed) return;
    closed = true;
    const fn = saiOverlayCloser;
    saiOverlayCloser = null;
    saiOverlayShut();
    if (typeof fn === "function") fn(null);
    if (typeof o.onClose === "function") o.onClose();
  };
  saiOverlayCloser = close;
  const head = saiEl("div", "sai-sheet-head");
  head.appendChild(saiEl("h2", "sai-sheet-title", o.title || ""));
  const x = saiEl("button", "sai-sheet-close", "✕");
  x.type = "button";
  x.setAttribute("aria-label", "閉じる");
  x.addEventListener("click", close);
  head.appendChild(x);
  panel.appendChild(head);
  if (o.body) panel.appendChild(o.body);
  return { close, el: panel };
}

/**
 * .sai-split の中の見せる側を切り替える（狭い画面だけ効く）。
 * which は "list" / "main"。
 */
function saiSplitShow(container, which) {
  if (!container || !container.classList) return;
  container.classList.toggle("sai-split--main", which === "main");
  container.classList.toggle("sai-split--list", which === "list");
}

// --- node からの読み込み（tests/test_ui.js） ---
// DOM を使わない部分（用語の表・saiWideAt）は値を返す。
// C-F3 の関数も「存在するか」を確かめられるよう形だけ出してある
//（これらを呼んでもブラウザの外では document を触らないので require してよい）。
if (typeof module !== "undefined" && module.exports) {
  module.exports = {
    SAI_WIDE_PX,
    SAI_TERMS,
    saiApi,
    saiToast,
    saiConfirm,
    saiPrompt,
    saiEmpty,
    saiTerm,
    saiIsWide,
    saiSheet,
    saiSplitShow,
    saiWideAt,
    saiTermText,
    saiTermLabel,
    saiTermList,
  };
}
