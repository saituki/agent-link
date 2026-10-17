"use strict";
// agent-link-web の共通部品。
//
// 端末（ターミナル）に慣れていない人が iPhone と PC のブラウザだけで
// agent-link を使えるように、画面からすぐ使う部品を 1 か所にまとめたもの。
// ここで定義するものは全部「一番外側の sai / AGENT_LINK_ で始まる名前」。
// 外部ライブラリは使わない（docs/integration.md §2）。
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

// --- 古い保存キーの移し替え ---
// 端末に残る旧名の保存キー（sai_phone_*）を新名（agent_link_web_*）に写し替える。
// 合言葉と下書きを失わないよう、画面がキーを読むより前にここで 1 度だけ移す。
// 新しいキーが既にあれば上書きしない。写したら古いキーは消す。
// localStorage と sessionStorage は別物なので、呼び出し側が両方を渡す。
function saiMigrateStorage(storage) {
  if (!storage) return;
  const oldPrefix = "sai_phone_";
  const newPrefix = "agent_link_web_";
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
const AGENT_LINK_WIDE_PX = 900;

/**
 * 用語の一覧（key 全部）。
 * label は札に出す既定の表示名、text はその語の説明（1〜3 文・日本語）。
 */
const AGENT_LINK_TERMS = {
  run: {
    label: "Run",
    text:
      "A record of an agent running one card once. " +
      "In the Chats tab, runs are listed as project name, #number, then runner. " +
      "A run is written by its number, like \"run #54\".",
  },
  todo: {
    label: "todo",
    text: "The number of cards that have not been run yet. You can run them from the project screen.",
  },
  running: {
    label: "running",
    text: "The number of cards being run right now. While they run, they also appear in the Chats tab.",
  },
  review: {
    label: "review",
    text:
      "The state of waiting for a person to check after an agent finishes its work. " +
      "Read what is here and approve it to move on.",
  },
  workflow: {
    label: "Workflow",
    text: "A board that shows the current state of each project's cards. Open it with \"Workflow\" on the Projects screen.",
  },
  kanban: {
    label: "Kanban",
    text: "A list that splits cards into columns such as \"doing → done → review\". Workflow shows the columns side by side.",
  },
  block: {
    label: "Block",
    text: "Instructions for one card: which files to look at and how to change them.",
  },
  template: {
    label: "Template",
    text: "A starting form for writing a new card. You can edit it for each project.",
  },
  settings: {
    label: "Settings file",
    text: "A file that sets how agents run (which model to use, how many to run at once, and so on).",
  },
  waggle: {
    label: "Waggle token",
    text:
      "A passcode for a document an agent saved. Write it in the task text of the next card, and the document body is passed along as is. " +
      "(Example: AXANSMpo)",
  },
  daemon: {
    label: "Daemon",
    text:
      "A process that keeps running on the Mac for each project. While it is stopped, cards cannot run. " +
      "Start it from the project screen (Dashboard).",
  },
  card: {
    label: "Card",
    text: "One file that holds a task text (task.md). One card holds one piece of work.",
  },
  task_text: {
    label: "Task text",
    text: "The request used when a card runs. The agent reads it and does the work.",
  },
  bibtex: {
    label: "BibTeX",
    text: "A text format that holds a paper's authors, title, year, and so on. Use it to import many papers at once.",
  },
  attention: {
    label: "Attention",
    text:
      "The state where the paper reader shows items nobody has looked at yet: " +
      "reviews not written, questions to the user, sources to confirm, and so on.",
  },
  state: {
    label: "State (state.md)",
    text: "A note (state.md) where an agent keeps writing the current state of the project.",
  },
  mind: {
    label: "Mind (mind.md)",
    text: "A note (mind.md) where the user writes the base of the research (initial ideas and constraints). Agents do not rewrite it directly.",
  },
  // --- research-tree（今の用語に作り直した） ---
  sci_research_tree: {
    label: "research-tree",
    text: "A tree made of the layers ROOT, seed (D), SPROUT, and BRANCH, and the diagram that draws it.",
  },
  sci_seed: {
    label: "seed (D)",
    text: "The constraints of one study. It has a title and six axes: task, target, data, method, output, and metric. Its ID starts with D.",
  },
  sci_root: {
    label: "ROOT",
    text: "A collective name for the A and F nodes that branch downward from an axis A.",
  },
  sci_sprout: {
    label: "SPROUT",
    text: "Sprout (S): a pair of a seed and one path for each axis. Its ID starts with S, and it is the basis for planning experiments.",
  },
  sci_branch: {
    label: "BRANCH",
    text: "An E that branches upward from a SPROUT. It stands for running an experiment.",
  },
  sci_path: {
    label: "PATH",
    text: "The route of A nodes chosen for each axis of a SPROUT. It is written as the A nodes listed in the SPROUT's paths.",
  },
  sci_node: {
    label: "Node",
    text: "One element of the research-tree. It is one of A, F, E, S, or D.",
  },
  sci_top_q: {
    label: "Axis",
    text: "The A nodes at the very top of the tree (axis A). There are six: task, target, data, method, output, and metric. Other A nodes go under them.",
  },
  sci_rel: {
    label: "Subtype / condition",
    text: "The relation of a child A. A subtype is a kind of its parent (a split by kind). A condition is an attribute or condition of its parent, not a split by kind. A condition can only go under a STAR A or a seed answer A.",
  },
  sci_star: {
    label: "STAR (★)",
    text: "A mark the user puts on an A. STAR_PATH is a route that can be traced from an axis through STAR A nodes.",
  },
  sci_proposal: {
    label: "PROPOSAL",
    text: "A proposed change to the research-tree made by an agent. Its ID starts with P.",
  },
  sci_triage: {
    label: "TRIAGE",
    text: "Deciding a PROPOSAL as accept, reject, or revert.",
  },
  sci_card: {
    label: "CARD / READ_CARD",
    text: "A request to an agent is a CARD. A CARD where the user reads a paper and adds an F is a READ_CARD.",
  },
  sci_guide: {
    label: "GUIDE",
    text: "A guide to what to put under an axis A. It describes what to extract and what to watch out for.",
  },
  sci_research_type: {
    label: "RESEARCH_TYPE",
    text: "A classification of how a study starts (PROBLEM_DRIVEN, PHENOMENON_DRIVEN, and so on).",
  },
  sci_lab_agent: {
    label: "LabAgent",
    text: "An agent that runs on a block with sci_type. It is one of three kinds: node agent, manage agent, and support agent.",
  },
};

// 軸の A（research-tree の木の一番上。親の無い A）か。research-tree の画面はこの 1 か所で見分ける。
function saiIsAxis(node) {
  return !!node && node.kind === "A" && node.parent == null;
}

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
 *   - つながらない（fetch の TypeError）ときは SaiShell.fail() を呼び、Error("Cannot reach the Mac.")
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
    throw new Error("Cannot reach the Mac.");
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
          detail = text; // not JSON: show as is
        }
      }
    } catch { /* 読めなくても下の既定の文でよい */ }
    const err = new Error(detail || `Could not load (${res.status}).`);
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
  return Number.isFinite(w) && w >= AGENT_LINK_WIDE_PX;
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
  return Object.prototype.hasOwnProperty.call(AGENT_LINK_TERMS, key) ? AGENT_LINK_TERMS[key].text : "";
}

/** 用語 key の既定の表示名（無ければ空）。 */
function saiTermLabel(key) {
  const t = AGENT_LINK_TERMS[key];
  return t && t.label ? t.label : "";
}

/** 用語一覧（[{key, label, text}]）。テストと説明の画面で使う。 */
function saiTermList() {
  return Object.keys(AGENT_LINK_TERMS).map((key) => ({
    key,
    label: saiTermLabel(key),
    text: saiTermText(key),
  }));
}

/**
 * 用語の札。key は用語の一覧の key、label を省くと既定の表示名。
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
    panel.appendChild(saiEl("p", "sai-panel-body", saiTermText(key) || "No description for this term yet."));
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

// --- グラフの目盛りと値 ---
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
    if (Math.abs(v) >= 1000) return `$${Math.round(v).toLocaleString("en-US")}`;
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
  return Math.round(v).toLocaleString("en-US");
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
    "aria-label": `${o.what || "Daily chart"}. No data.`,
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
    maxAt = `. Max ${saiAxisValue(dataMax, unit)}${day ? ` (${day})` : ""}`;
  }
  svg.setAttribute(
    "aria-label",
    `${o.what || "Daily chart"}. ${n} ${n === 1 ? "day" : "days"}${maxAt}. Total ${saiAxisValue(total, unit)}`
  );
  return svg;
}

// --- node からの読み込み（tests/test_ui.js） ---
// DOM を使わない部分（用語の表・saiWideAt）は値を返す。
// 共通部品の関数も「存在するか」を確かめられるよう形だけ出してある
//（これらを呼んでもブラウザの外では document を触らないので require してよい）。
if (typeof module !== "undefined" && module.exports) {
  module.exports = {
    AGENT_LINK_WIDE_PX,
    AGENT_LINK_TERMS,
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
    saiIsAxis,
    // グラフの目盛りと値（U04。DOM を触らないものだけ）
    saiNiceScale,
    saiAxisValue,
    saiDayLabel,
    saiWideAt,
  };
}
