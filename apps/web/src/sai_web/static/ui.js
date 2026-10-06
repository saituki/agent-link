"use strict";
// sai-web の共通部品（実装計画 U4・共有契約 C-F3）。
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

// --- 古い保存キーの移し替え（U17） ---
// 端末に残る旧名の保存キー（sai_phone_*）を新名（sai_web_*）に写し替える。
// 合言葉と下書きを失わないよう、画面がキーを読むより前にここで 1 度だけ移す。
// 新しいキーが既にあれば上書きしない。写したら古いキーは消す。
// localStorage と sessionStorage は別物なので、呼び出し側が両方を渡す。
function saiMigrateStorage(storage) {
  if (!storage) return;
  const oldPrefix = "sai_phone_";
  const newPrefix = "sai_web_";
  const oldKeys = [];
  for (let i = 0; i < storage.length; i += 1) {
    const key = storage.key(i);
    if (key && key.startsWith(oldPrefix)) oldKeys.push(key);
  }
  for (const key of oldKeys) {
    const value = storage.getItem(key);
    const next = newPrefix + key.slice(oldPrefix.length);
    if (storage.getItem(next) === null) storage.setItem(next, value);
    storage.removeItem(key);
  }
}

// ブラウザでだけ実行する（node からの require では window が無い）。
if (typeof window !== "undefined" && window.localStorage) {
  saiMigrateStorage(window.localStorage);
  saiMigrateStorage(window.sessionStorage);
}

// --- 画面が広いと思う境目（900px 以上） ---
const SAI_WIDE_PX = 900;

/**
 * 用語の一覧（C-F5 の key 全部）。
 * label は札に出す既定の表示名、text はその語の説明（1〜3 文・日本語）。
 */
const SAI_TERMS = {
  run: {
    label: "Run",
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
    label: "Attention",
    text:
      "論文のリーダーが、まだ人が見ていないものを出した状態です。" +
      "「書いていない review」「ユーザーへの質問」「出典の要確認」など。",
  },
  state: {
    label: "State (state.md)",
    text: "プロジェクトがいまどうなっているかをエージェントが継続的に書いているメモ（state.md）です。",
  },
  mind: {
    label: "Mind (mind.md)",
    text: "これまでの経緯と、次にやりたいことをエージェントが継続的に書いているメモ（mind.md）です。",
  },
  // --- 研究方針の木（U19・共有契約 C-14） ---
  sci_seed: {
    label: "SEED_SENTENCE（種の文）",
    text: "研究の型ごとの 1 文のひな形です。決まった ITEM の値が入り、まだ決まっていない所は［？］で出ます。",
  },
  sci_gate: {
    label: "GATE（関門）",
    text: "研究の型ごとに決めた通過条件の並びです。最初に通っていない GATE が、今いる所です。",
  },
  sci_blank: {
    label: "BLANK の種類（空欄の種類）",
    text: "まだ決まっていない ITEM が、なぜ空いているかの分類です（UNTOUCHED・IN_PROGRESS・INSUFFICIENT・AWAITING_DECISION・CONFLICT）。",
  },
  sci_debt: {
    label: "DEBT（負債）",
    text: "先に片付けたいものです（PENDING_PROPOSALS・UNREFLECTED_PAPERS・UNSCREENED_SETS・EMPTY_REVIEWS）。",
  },
  sci_finding: {
    label: "FINDING（知見）",
    text: "論文などから分かった 1〜3 文の事実と、その出典です。SUPPORT・REFUTE などの POLARITY（向き）を持ちます。",
  },
  sci_assume: {
    label: "ASSUMPTION（仮置き）",
    text: "その議論では問い直さない前提として置く値です。",
  },
  sci_scope: {
    label: "SCOPE（範囲）",
    text: "エージェントに渡す、研究方針の木から切り出した部分です。",
  },
  sci_triage: {
    label: "TRIAGE（仕分け）",
    text: "エージェントの PROPOSAL を採用するか却下するかを決めることです（ACCEPT・REJECT・REVERT）。",
  },
  sci_tombstone: {
    label: "TOMBSTONE（墓標）",
    text: "外した論文の記録です。もう一度検索に出ないように残します。",
  },
  sci_type: {
    label: "RESEARCH_TYPE（研究の型）",
    text: "研究の始まり方の分類です（PROBLEM_DRIVEN・PHENOMENON_DRIVEN・METHOD_DRIVEN・DATA_DRIVEN・IMPROVE_DRIVEN）。ITEM_ORDER と GATE が変わります。",
  },
  sci_item: {
    label: "ITEM（項目）",
    text: "研究方針の木で「決める穴」です。状態は OPEN・TENTATIVE・DECIDED・EXCLUDED のどれかです。",
  },
  sci_option: {
    label: "OPTION（候補）",
    text: "ITEM の下に置く答えの案です。状態は CANDIDATE・SELECTED・REJECTED・DORMANT のどれかです。",
  },
  sci_mark: {
    label: "MARK（印）",
    text: "ITEM に付く注意の印です（WEAK_EVIDENCE・RECHECK）。",
  },
  sci_proposal: {
    label: "PROPOSAL（提案）",
    text: "エージェントが木に出す変更の案です。AUTO_OP は自動で入り、REVIEW_OP は利用者が TRIAGE で仕分けます。",
  },
  sci_card: {
    label: "CARD・READ_CARD（カード・読む札）",
    text: "研究方針の木から作るエージェントへの依頼です。READ_CARD は利用者自身が論文を読んで FINDING を足す CARD です。",
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

/** SVG の要素を 1 つ作る（属性は「名前 → 値」の順に並べる）。 */
function saiSvg(tag, attrs) {
  const node = document.createElementNS("http://www.w3.org/2000/svg", tag);
  for (const [k, v] of Object.entries(attrs || {})) node.setAttribute(k, v);
  return node;
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
 * 確認の小窓。{title, body, ok = "OK", cancel = "Cancel", danger = false}
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
    const cancel = saiEl("button", "sai-btn ghost", o.cancel || "Cancel");
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
    const cancel = saiEl("button", "sai-btn ghost", "Cancel");
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

// --- todo の 2 行表示（Dashboard と Home で共通） ---

/**
 * todo 1 件の 2 行表示: {head, desc}。
 * head は `<作成日> ID<run_id> <block> <runner> <model>`（空の項目は詰める）、desc は説明。
 * `card`（task.md の項目）が無いときは head を label にする。
 */
function saiTodoLines(run) {
  const r = run || {};
  const c = r.card && typeof r.card === "object" ? r.card : null;
  if (!c) return { head: String(r.label || `ID${r.run_id}`), desc: "" };
  const head = [c.creation_date, `ID${r.run_id}`, c.block, c.runner, c.model]
    .map((v) => String(v == null ? "" : v).trim())
    .filter(Boolean)
    .join(" ");
  return { head, desc: String(c.description || "").trim() };
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
    const ok = saiEl("button", "sai-btn primary", "Close");
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
  x.setAttribute("aria-label", "Close");
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

// --- グラフの目盛りと値（U04） ---
//
// グラフはどの画面でも素の SVG を手描きする（外部ライブラリは入れない）。
// 「目盛りをどの刻みで置くか」「その数字をどう書くか」「日ごとの棒の形」は
// ここに 1 つだけ置き、Project（project.js）と Usage（usage.js）はそれを呼ぶ。
// 色は CSS 変数（--series-* / --chart-grid / --chart-axis）で決める（style.css）。

/** 小数1位まで。ただし整数なら .0 を付けない（1.5M / 2M）。 */
function saiTrim1(x) {
  const r = Math.round(Number(x) * 10) / 10;
  return Number.isInteger(r) ? String(r) : r.toFixed(1);
}

/**
 * 切りりのよい目盛り（0 と、途中の数本と、最大の数本）。
 * 例: 12.34 を 3 本に切ると 0 / 5 / 10 / 15（棒は 15 の高さまで描く）。
 * minStep は 1 本の最低の刻み（金額は 0.01 未満にしない）。
 * 返り値: { max（上端の値）, step（刻み）, values（下から上への目盛り）}
 */
function saiNiceScale(max, ticks, minStep) {
  const count = Math.max(2, Math.floor(Number(ticks)) || 3);
  const floorStep = Math.max(0, Number(minStep) || 0);
  const m = Number(max);
  if (!Number.isFinite(m) || m <= 0) {
    return { max: floorStep > 0 ? floorStep : 1, step: floorStep > 0 ? floorStep : 1, values: [0, floorStep > 0 ? floorStep : 1] };
  }
  const rough = m / count;
  const mag = Math.pow(10, Math.floor(Math.log10(rough)));
  const norm = rough / mag;
  const mult = norm <= 1 ? 1 : norm <= 2 ? 2 : norm <= 2.5 ? 2.5 : norm <= 5 ? 5 : 10;
  let step = mult * mag;
  if (step < floorStep) step = floorStep;
  const top = Math.ceil(m / step - 1e-9) * step;
  const values = [];
  for (let i = 0; i * step <= top + step * 1e-6; i += 1) values.push(i * step);
  return { max: top, step, values };
}

/** 目盛りや値に書く文字（単位つき）。unit は money / tokens / percent / それ以外。 */
function saiAxisValue(value, unit) {
  const v = Number(value);
  if (!Number.isFinite(v)) return "0";
  if (unit === "money") {
    if (Math.abs(v) >= 1000) return `$${Math.round(v).toLocaleString("ja-JP")}`;
    return `$${v.toFixed(2)}`;
  }
  if (unit === "tokens") {
    const a = Math.abs(v);
    if (a >= 1e9) return `${saiTrim1(v / 1e9)}B`;
    if (a >= 1e6) return `${saiTrim1(v / 1e6)}M`;
    if (a >= 1e3) return `${saiTrim1(v / 1e3)}K`;
    return String(Math.round(v));
  }
  if (unit === "percent") return `${Math.round(v)}%`;
  // 回数は 4 桁を超えると目盛りには長すぎるので K / M にする
  const a = Math.abs(v);
  if (a >= 1e6) return `${saiTrim1(v / 1e6)}M`;
  if (a >= 1e4) return `${saiTrim1(v / 1e3)}K`;
  return Math.round(v).toLocaleString("ja-JP");
}

/** "2026-09-20" から「9/20」を作る（読めなければ空文字）。 */
function saiDayLabel(date) {
  const s = String(date || "");
  if (s.length < 10) return "";
  return `${Number(s.slice(5, 7))}/${Number(s.slice(8, 10))}`;
}

/** 画面が「動きを減らす」と言っているか（グラフを描きながら動かすかの分岐）。 */
function saiReducedMotion() {
  return (
    typeof window !== "undefined" &&
    typeof window.matchMedia === "function" &&
    window.matchMedia("(prefers-reduced-motion: reduce)").matches
  );
}

/**
 * 日ごとの棒グラフ（縦の目盛り 0 / 中間 / 最大＋格子線＋横のラベル）。
 * Project の Dashboard と Usage の Activity が同じ形で使う。
 *
 * opts:
 *   rows    一覧（空のときは軸だけの枠を出さず、そのまま戻す）
 *   valueOf (row) => 数値
 *   labelOf (row, i, n) => 横のラベル（空文字ならその位置のラベルは出さない）
 *   unit    money / tokens / percent / それ以外
 *   what    グラフの説明（aria-label の先頭）
 *   onPick  (row) => void（棒を押したとき）
 */
function saiDailyBars(opts) {
  const o = opts || {};
  const rows = Array.isArray(o.rows) ? o.rows : [];
  const unit = o.unit || "count";
  const svg = saiSvg("svg", {
    viewBox: "0 0 320 150",
    class: "dash-chart",
    role: "img",
    "aria-label": `${o.what || "日ごとのグラフ"}。データがありません`,
  });
  // データが無いときは「空の軸だけの枠」を出さない（空の表示の規則）
  if (rows.length === 0) return svg;

  const valueOf =
    typeof o.valueOf === "function" ? o.valueOf : (row) => Number(row && row.value) || 0;
  const values = rows.map(valueOf);
  const dataMax = Math.max(0, ...values);
  const total = values.reduce((a, v) => a + (Number.isFinite(v) ? v : 0), 0);
  // 金額は 1 cents 未満に切らない（目盛りが全部 $0.00 になるのを防ぐ）
  const scale = saiNiceScale(dataMax, 3, unit === "money" ? 0.01 : 0);

  const W = 320;
  const L = 40; // 左は目盛りの文字のぶん
  const R = 6;
  const T = 10;
  const B = 22;
  const baseY = 128; // 棒の底（150 から下の余白 22 を引く）
  const n = rows.length;
  const slot = (W - L - R) / n;
  const yOf = (v) =>
    baseY - (Math.max(0, Math.min(v, scale.max)) / scale.max) * (baseY - T);

  // 縦の目盛りと格子線（0 のところだけは:--chart-axis のsolid な線）
  for (const t of scale.values) {
    const y = yOf(t).toFixed(2);
    svg.appendChild(
      saiSvg("line", {
        x1: String(L),
        y1: y,
        x2: String(W - R),
        y2: y,
        class: t === 0 ? "dash-base" : "dash-gridline",
      })
    );
    const label = saiSvg("text", {
      x: String(L - 5),
      y,
      dy: "4",
      class: "dash-axis dash-yaxis",
      "text-anchor": "end",
    });
    label.textContent = saiAxisValue(t, unit);
    svg.appendChild(label);
  }

  // 棒
  const barW = Math.max(2, Math.min(slot * 0.62, 26));
  rows.forEach((row, i) => {
    const v = values[i];
    const h = v > 0 ? Math.max(3, (v / scale.max) * (baseY - T)) : 3;
    const rect = saiSvg("rect", {
      x: (L + i * slot + (slot - barW) / 2).toFixed(2),
      y: (baseY - h).toFixed(2),
      width: barW.toFixed(2),
      height: h.toFixed(2),
      rx: "3",
      class: v > 0 ? "dash-bar" : "dash-bar zero",
    });
    if (typeof o.onPick === "function") {
      rect.setAttribute("role", "button");
      rect.setAttribute("tabindex", "0");
      rect.setAttribute("aria-label", `${saiDayLabel(row && row.date)} ${saiAxisValue(v, unit)}`);
      const pick = () => o.onPick(row);
      rect.addEventListener("click", pick);
      rect.addEventListener("keydown", (e) => {
        if (e.key === "Enter" || e.key === " ") {
          e.preventDefault();
          pick();
        }
      });
    }
    svg.appendChild(rect);
  });

  // 横のラベル。画面が狭いときは 30 の間隔、広いときは 46 の間隔までで間引く。
  const minGap = saiIsWide() ? 46 : 30;
  const picks = [];
  let lastX = -Infinity;
  rows.forEach((row, i) => {
    const text =
      typeof o.labelOf === "function"
        ? String(o.labelOf(row, i, n) || "")
        : saiDayLabel(row && row.date);
    if (!text) return;
    const x = L + i * slot + slot / 2;
    const isEnd = i === 0 || i === n - 1;
    if (x - lastX < (isEnd ? minGap * 0.6 : minGap)) return;
    lastX = x;
    picks.push({ i, x, text });
  });
  // 最後（何日までか）は必ず出す。潰れていたら直前のラベルを潰す
  if (n > 1 && picks[picks.length - 1].i !== n - 1) {
    const last = rows[n - 1];
    const text =
      typeof o.labelOf === "function"
        ? String(o.labelOf(last, n - 1, n) || "")
        : saiDayLabel(last && last.date);
    if (text) {
      const x = L + (n - 1) * slot + slot / 2;
      while (picks.length > 1 && x - picks[picks.length - 1].x < minGap * 0.6) {
        picks.pop();
      }
      picks.push({ i: n - 1, x, text });
    }
  }
  for (const p of picks) {
    const t = saiSvg("text", {
      x: (p.i === n - 1 ? W - R : p.x).toFixed(2),
      y: String(baseY + 16),
      class: "dash-axis",
      // 最後のラベルは右端で切れないように右寄せにする
      "text-anchor": p.i === n - 1 ? "end" : "middle",
    });
    t.textContent = p.text;
    svg.appendChild(t);
  }

  // 要点を 1 文で（画面読み上げ用に、最大の所と合計を教える）
  let maxAt = "";
  if (dataMax > 0) {
    const i = values.indexOf(dataMax);
    const day = saiDayLabel(rows[i] && rows[i].date);
    maxAt = `。最大 ${saiAxisValue(dataMax, unit)}${day ? `（${day}）` : ""}`;
  }
  svg.setAttribute(
    "aria-label",
    `${o.what || "日ごとのグラフ"}。${n} 日分${maxAt}。合計 ${saiAxisValue(total, unit)}`
  );
  return svg;
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
    saiTodoLines,
    saiTerm,
    saiIsWide,
    saiSheet,
    saiSplitShow,
    saiWideAt,
    saiTermText,
    saiTermLabel,
    saiTermList,
    saiMigrateStorage,
    // グラフの目盛りと値（U04。DOM を触らないものだけ）
    saiNiceScale,
    saiAxisValue,
    saiDayLabel,
    saiWideAt,
  };
}
