// sai-web の task.md 作成フォーム（計画 15 §1.5）の本体。
//
// List 画面右上の ＋ で下から出る。TUI のウィザード（sai-link-tui の card_wizard.rs）と
// 同じ順に 1 画面ずつ進み、スキップ・戻る・番号の飛び方も同じ規則にする:
//   1 runner → 2 ブロック → 3 モデル → 4 thinking
//   → 5 ラウンド → 6 task_text（🎤 音声入力・最近の文書を貼れる）→ 7 確認 → 作成
//   user / gl は 2 から 5 へ飛ぶ（runner が空なら cc 扱い）。
// 選択肢は `GET …/card/options`、作成は `POST …/card`
// （値の検査はサーバー側でも行う・計画 15 §3.3）。
// 入力ファイルの手順は無い（入力は task_text に統一した・U09）。
// 書きかけはプロジェクトごとに localStorage へ残し、作成に成功したら消す。
// 進み方（DOM を使わない関数）は末尾で node に渡す（tests/test_cardform.js）。
"use strict";

// --- 定数 ---

// 手順の地図（番号は手順の数で決まる。スキップや自動で飛んだ画面は番号が飛ぶ・計画 15 §1.5）
const CF_STEPS = [
  { id: "runner", no: 1, title: "runner" },
  { id: "block", no: 2, title: "ブロック" },
  { id: "model", no: 3, title: "モデル" },
  { id: "thinking", no: 4, title: "thinking" },
  { id: "rounds", no: 5, title: "ラウンド" },
  { id: "task_text", no: 6, title: "task_text" },
  { id: "confirm", no: 7, title: "確認" },
];
// 7 画面それぞれの見出しの下に出す説明（端末に慣れていない人向け・U5 §3）。
// 画面の流れ・スキップ・戻る・保存の動きは変えない（見出しの下に文言を足すだけ）。
const CF_STEP_HINTS = {
  runner: "このカードをどの AI に渡すかです。cc など、名前で選びます。",
  block: "どんな順番で、どのファイルを見て、どう直せばよいかを書いた指示書です。",
  model: "使うモデルの名前です。動かすエージェントによって出し方が違います。",
  thinking: "考えの深さです。難しい仕事で深めにすると時間がかかりますが、精度が上がります。",
  rounds: "同じ仕事を引き継ぎながら何回回すかです。回数を増やすほど長く続けます。",
  task_text: "依頼文です。エージェントはこの文章に従って動きます。",
  confirm: "作った依頼文（task.md）を最後に読みます。「保存」でファイルができます。",
};
// 書きかけの置き場（プロジェクトごとに分ける・計画 15 §1.5）
const CF_DRAFT_KEY = "sai_web_card_draft";
// user / gl はモデルと thinking が対象外（TUI の advance_from_block と同じ）
const CF_NO_INPUT_RUNNERS = ["user", "gl"];

// --- 進み方（DOM を使わない・tests/test_cardform.js） ---

/** 新しい書きかけ（すべてスキップした状態から始める）。 */
function cfNewDraft() {
  return {
    step: "runner",
    runner: "",         // "" = スキップ（cc 扱い）
    block: "",
    model: "",
    thinking: "",
    rounds: { global_round: "", round: "", tag: "" },
    task_text: "",
  };
}

/** 手順 id から地図の 1 行を引く。 */
function cfStepEntry(id) {
  return CF_STEPS.find((s) => s.id === id) || CF_STEPS[0];
}

/** 今の手順の見出しの下に出す説明（端末に慣れていない人向け・U5 §3）。 */
function cfStepHint(id) {
  return Object.prototype.hasOwnProperty.call(CF_STEP_HINTS, id) ? CF_STEP_HINTS[id] : "";
}

/**
 * 今いる手順の番号と題名（表示は「Step n/7」）。
 * 番号は手順の数で決まり、スキップした分は飛ぶ（user / gl なら 2 → 5）。
 */
function cfStepInfo(state) {
  const entry = cfStepEntry(state.step);
  return { id: entry.id, no: entry.no, title: entry.title, label: `Step ${entry.no}/${CF_STEPS.length}` };
}

/** ブロック確定後の行き先（番号の飛び方は TUI の advance_from_block と同じ）。 */
function cfAfterBlock(state) {
  const runner = String(state.runner || "");
  if (runner === "user" || runner === "gl") {
    // gl は起動時のタスク書き込みなし・user は人がやる作業なので model〜thinking は丸ごと省く
    state.model = "";
    return "rounds";
  }
  return "model"; // cc（runner が空なら cc 扱い）
}

/** 次へ進めるか。進めれば null、ダメなら理由（その画面に止まる）。 */
function cfNext(state) {
  switch (state.step) {
    case "runner":
      state.step = "block";
      return null;
    case "block":
      state.step = cfAfterBlock(state);
      return null;
    case "model":
      state.step = "thinking";
      return null;
    case "thinking":
      state.step = "rounds";
      return null;
    case "rounds":
      state.step = "task_text";
      return null;
    case "task_text":
      state.step = "confirm";
      return null;
    default:
      // 確認画面は「作成」ボタンで送る（次へは無い）
      return "これ以上進めません";
  }
}

/** スキップできる手順か（引き継ぎと確認は無し・TUI と同じ）。 */
function cfCanSkip(state) {
  return ["runner", "block", "model", "thinking", "rounds", "task_text"].includes(state.step);
}

/** スキップ（その項目は sai-link の既定値に任せる）。止めれば理由を返す。 */
function cfSkip(state) {
  if (!cfCanSkip(state)) return "スキップできません";
  switch (state.step) {
    case "runner":
      state.runner = "";
      state.step = "block";
      return null;
    case "block":
      state.block = "";
      state.step = cfAfterBlock(state);
      return null;
    case "model":
      state.model = "";
      state.step = "thinking";
      return null;
    case "thinking":
      state.thinking = "";
      state.step = "rounds";
      return null;
    case "rounds":
      // TUI はここで作成するが、本フォームは task_text → 確認 があるので次へ進む
      state.step = "task_text";
      return null;
    case "task_text":
      state.task_text = "";
      state.step = "confirm";
      return null;
    default:
      return "スキップできません";
  }
}

/** 1 つ前へ戻る。最初の画面（runner）なら "close"（フォームを閉じる合図）。 */
function cfBack(state) {
  switch (state.step) {
    case "runner":
      return "close";
    case "block":
      state.step = "runner";
      return null;
    case "model":
      state.step = "block";
      return null;
    case "thinking":
      state.step = "model";
      return null;
    case "rounds":
      // user / gl → ブロック、cc → thinking（TUI と同じ）
      state.step = CF_NO_INPUT_RUNNERS.includes(String(state.runner || ""))
        ? "block"
        : "thinking";
      return null;
    case "task_text":
      state.step = "rounds";
      return null;
    case "confirm":
      state.step = "task_text";
      return null;
    default:
      return "close";
  }
}

/**
 * task_text の参照行（1 行）を出し入れする。`on` が true なら無いときだけ末尾に足し、
 * false なら同じ行を消す。戻り値は新しい task_text。
 */
function cfToggleRef(text, ref, on) {
  const lines = String(text || "").split("\n");
  const has = lines.some((l) => l.trim() === ref);
  if (on) {
    if (has || !ref) return String(text || "");
    const base = String(text || "").replace(/\s+$/, "");
    return base ? `${base}\n${ref}` : ref;
  }
  return lines.filter((l) => l.trim() !== ref).join("\n");
}

/**
 * task_text の末尾に文を足す（`opts.taskText`・U10）。
 * 空でなければ空行を 1 つはさむ。同じ文が既にあれば足さない。書きかけは消さない。
 */
function cfAppendTaskText(text, add) {
  const base = String(text || "");
  const line = String(add == null ? "" : add);
  if (!line.trim()) return base;
  if (base.split("\n").some((l) => l.trim() === line.trim())) return base;
  const trimmed = base.replace(/\s+$/, "");
  return trimmed ? `${trimmed}\n\n${line}` : line;
}

/**
 * 開くプロジェクト名を決める（`opts.project`・U10）。
 * `opts.project` があればそれを使い，`parseListHash` に頼らない。
 * 無ければ今の List の住所の名前，それも無ければ今の名前のまま。
 */
function cfTargetName(opts, hash, current) {
  const fromOpts = opts && opts.project ? String(opts.project) : "";
  if (fromOpts) return fromOpts;
  const parsed =
    typeof parseListHash === "function" ? parseListHash(hash) : null;
  return (parsed && parsed.name) || current || "";
}

/**
 * フォームを閉じるか（U10）。
 * 開いたときが List の住所なら，List の住所でなくなったら閉じる（今までどおり）。
 * そうでなければ（論文の詳細などから開いたとき），開いたときの住所から変わったら閉じる。
 */
function cfShouldClose(openedHash, nowHash) {
  const opened = String(openedHash || "");
  const now = String(nowHash || "");
  if (!opened) return false;
  const parse = typeof parseListHash === "function" ? parseListHash : null;
  if (parse && parse(opened)) return !parse(now);
  return now !== opened;
}

/** `POST …/card` に送る JSON（項目名は計画 12 §3.2 の形のまま）。 */
function cfBuildPayload(state) {
  const rounds = state.rounds || {};
  return {
    runner: state.runner || "",
    block: state.block || "",
    model: state.model || "",
    thinking: state.thinking || "",
    global_round: rounds.global_round || "",
    round: rounds.round || "",
    tag: rounds.tag || "",
    task_text: state.task_text || "",
  };
}

// --- 書きかけの保存（プロジェクトごと・プライベートブラウズでも動くように失敗に備える） ---

function cfDraftKey(name) {
  return `${CF_DRAFT_KEY}:${name}`;
}

/** 書きかけを 1 つ引く。無ければ null。 */
function cfLoadDraft(name) {
  if (!name) return null;
  try {
    const raw = localStorage.getItem(cfDraftKey(name));
    const obj = raw ? JSON.parse(raw) : null;
    if (!obj || typeof obj !== "object") return null;
    // 手順が無い書きかけは無し扱い（U09 より前の `inputs` の手順もここで外れる）
    if (!CF_STEPS.some((s) => s.id === obj.step)) return null;
    // 形が崩れた項目は新しい値でそろえる
    return Object.assign(cfNewDraft(), obj, {
      rounds: Object.assign(cfNewDraft().rounds, obj.rounds || {}),
    });
  } catch {
    return null;
  }
}

function cfSaveDraft(name, state) {
  if (!name || !state) return;
  try {
    localStorage.setItem(cfDraftKey(name), JSON.stringify(state));
  } catch { /* 保存できなくても動く */ }
}

/** 作成に成功したら消す（計画 15 §1.5）。 */
function cfClearDraft(name) {
  if (!name) return;
  try {
    localStorage.removeItem(cfDraftKey(name));
  } catch { /* 保存できなくても動く */ }
}

// ===========================================================================
// ここから下はブラウザだけで動く部分（node のテストでは読まれない）
// ===========================================================================

// --- 画面の状態 ---

let cfName = "";       // 開いているプロジェクト名
let cfState = null;    // 今いる手順（null = フォームは閉じている）
let cfOptions = null;  // `GET …/card/options` の応答
let cfError = "";      // 手順の下に出す 1 行
let cfWarn = "";       // options の warnings（settings.json の注意など）
let cfBusy = false;    // 送信中
let cfDocs = null;     // 最近の文書（`GET …/card/docs` の items・null＝未読）
let cfOpenedHash = ""; // フォームを開いたときの住所（閉じ方を決める・U10）

function cfSheet() {
  return document.getElementById("cardform-wrap");
}

function cfSetError(text) {
  cfError = String(text || "");
  const box = document.getElementById("cf-msg");
  if (!box) return;
  box.textContent = cfError;
  box.hidden = !cfError;
}

async function cfFetchJson(path) {
  const res = await apiFetch(path);
  const body = await res.json().catch(() => ({}));
  if (!res.ok) throw new Error(body.detail || "読み込めませんでした");
  return body;
}

/**
 * ＋ を押した入口（書きかけがあれば続きから・計画 15 §1.5）。
 * `opts.ref` があれば task_text にその参照行（`waggle:<tok>:path:<path>`）を足して開く（List の waggle の ＋）。
 */
async function cfOpen(opts) {
  const o = opts || {};
  const ref = String(o.ref || "");
  const taskText = String(o.taskText || "");
  const project = cfTargetName(o, location.hash, cfName);
  if (project && project !== cfName) {
    // プロジェクトが変われば選択肢は取り直す（別プロジェクトの選択肢を出さないため）
    cfName = project;
    cfOptions = null;
    cfState = null;
    cfError = "";
  }
  if (!cfName) return;
  // 開いたときの住所を覚える（閉じ方を決める・U10）
  cfOpenedHash = location.hash;

  // 選択肢を取りに行く間も画面は開いたままにする（下から出る形のまま）
  const sheet = cfSheet();
  if (sheet) sheet.hidden = false;
  const body = document.getElementById("cf-body");
  if (body) body.innerHTML = "";
  if (body && !cfOptions) body.appendChild(cfNote("読み込み中…"));

  if (!cfOptions) {
    try {
      cfOptions = await cfFetchJson(
        `/api/projects/${encodeURIComponent(cfName)}/card/options`
      );
      window.SaiShell.ok();
    } catch (e) {
      window.SaiShell.fail();
      cfRenderFatal((e && e.message) || "選択肢を読み込めませんでした");
      return;
    }
  }
  cfWarn = ((cfOptions && cfOptions.warnings) || []).join(" / ");

  const draft = cfLoadDraft(cfName);
  cfState = draft || cfNewDraft();
  cfError = "";
  cfDocs = null; // 最近の文書は開くたびに取り直す
  if (ref) {
    cfState.task_text = cfToggleRef(cfState.task_text, ref, true);
    cfSaveDraft(cfName, cfState);
  }
  if (taskText) {
    // 論文の詳細などから開いたときの 1 行を足す（書きかけは消さない・U10）
    cfState.task_text = cfAppendTaskText(cfState.task_text, taskText);
    cfSaveDraft(cfName, cfState);
  }

  cfRender();
}

/** フォームを閉じる（書きかけは残す）。 */
function cfClose() {
  if (cfState) cfSaveDraft(cfName, cfState);
  cfState = null;
  const sheet = cfSheet();
  if (sheet) sheet.hidden = true;
}

/** 選択肢が取れなかったときの画面（手順は進めない・閉じるだけ）。 */
function cfRenderFatal(message) {
  const sheet = cfSheet();
  if (sheet) sheet.hidden = false;
  const body = document.getElementById("cf-body");
  if (body) body.innerHTML = "";
  const step = document.getElementById("cf-step");
  if (step) step.textContent = "";
  const title = document.getElementById("cf-name");
  if (title) title.textContent = "作成フォーム";
  cfSetError(message);
  // 進めない・戻れない状態なので，下の 3 つは隠して × だけ残す
  for (const id of ["cf-back", "cf-skip", "cf-next", "cf-restart"]) {
    const b = document.getElementById(id);
    if (b) b.hidden = true;
  }
}

/** 1 行（エラーと知らせ）を出す。 */
function cfNote(text) {
  return el("p", "cf-note", String(text || ""));
}

/** 選ぶ行（手順 2・3・5・6・7 などで使い回す）。押した値はアクティブになる。 */
function cfChoiceList(items, activeValue, onPick) {
  const box = document.createElement("div");
  box.className = "cf-choices";
  for (const item of items) {
    const b = document.createElement("button");
    b.className = "cf-choice" + (item.value === activeValue ? " active" : "");
    b.textContent = item.label;
    b.addEventListener("click", () => onPick(item.value));
    box.appendChild(b);
  }
  return box;
}

/** 今の手順の中身を作る。 */
function cfRenderBody() {
  const body = document.getElementById("cf-body");
  if (!body) return;
  body.innerHTML = "";
  const state = cfState;
  const info = cfStepInfo(state);
  // 見出しの下に 1 文の説明を足す（画面の流れは変えない）
  const hint = cfStepHint(info.id);
  if (hint) body.appendChild(cfNote(hint));

  if (info.id === "runner") {
    const runners = (cfOptions && cfOptions.runners) || [];
    body.appendChild(
      cfChoiceList(
        runners.map((r) => ({ value: r, label: r })),
        state.runner,
        (v) => {
          state.runner = v;
          cfSaveDraft(cfName, state);
          cfRenderBody();
        }
      )
    );
    body.appendChild(cfNote("スキップすると cc 扱い（sai-link の既定どおり）"));
    return;
  }

  if (info.id === "block") {
    const blocks = (cfOptions && cfOptions.blocks) || [];
    body.appendChild(
      cfChoiceList(
        blocks.map((b) => ({ value: String(b.id), label: String(b.label || b.id) })),
        state.block,
        (v) => {
          state.block = v;
          cfSaveDraft(cfName, state);
          cfRenderBody();
        }
      )
    );
    body.appendChild(cfNote("指示書を選びます（スキップすると sai-link の既定に任せます）"));
    return;
  }

  if (info.id === "model") {
    const models = (cfOptions && cfOptions.cc_models) || [];
    body.appendChild(
      cfChoiceList(
        models.map((m) => ({ value: String(m), label: String(m) })),
        state.model,
        (v) => {
          state.model = v;
          cfSaveDraft(cfName, state);
          cfRenderBody();
        }
      )
    );
    if (models.length === 0) body.appendChild(cfNote("選べるモデルがありません"));
    return;
  }

  if (info.id === "thinking") {
    const levels = (cfOptions && cfOptions.thinking) || [];
    body.appendChild(
      cfChoiceList(
        levels.map((t) => ({ value: t, label: t })),
        state.thinking,
        (v) => {
          state.thinking = v;
          cfSaveDraft(cfName, state);
          cfRenderBody();
        }
      )
    );
    body.appendChild(cfNote("スキップすると settings.json / ブロックの既定に任せます"));
    return;
  }

  if (info.id === "rounds") {
    for (const [key, label, ph] of [
      ["global_round", "global_round", "例: 5（空で引き継ぎ）"],
      ["round", "round", "例: 2（空で引き継ぎ）"],
      ["tag", "tag", "例: fix（空でなし）"],
    ]) {
      const row = el("label", "cf-input-row");
      row.appendChild(el("span", "cf-input-label", label));
      const input = document.createElement("input");
      input.type = "text";
      input.value = (state.rounds && state.rounds[key]) || "";
      input.placeholder = ph;
      input.addEventListener("input", () => {
        state.rounds[key] = input.value;
        cfSaveDraft(cfName, state);
      });
      row.appendChild(input);
      body.appendChild(row);
    }
    return;
  }

  if (info.id === "task_text") {
    body.appendChild(
      cfNote("何をしてほしいかを書きます（🎤 で音声入力もできます・空でスキップ）")
    );
    const ta = document.createElement("textarea");
    ta.className = "cf-textarea";
    ta.rows = 8;
    ta.value = state.task_text || "";
    ta.placeholder = "task_text を入力";
    ta.addEventListener("input", () => {
      state.task_text = ta.value;
      cfSaveDraft(cfName, state);
    });
    body.appendChild(ta);
    body.appendChild(cfDocsBox(state, ta));
    // 🎤 は Agent タブの音声入力をそのまま使う（新しく作らない・計画 15 §1.5）
    const mic = document.createElement("button");
    mic.className = "cf-mic";
    mic.id = "cf-mic";
    mic.setAttribute("aria-label", "音声入力");
    mic.textContent = "🎤";
    mic.addEventListener("click", () => {
      ta.focus();
    });
    body.appendChild(mic);
    if (typeof window !== "undefined" && typeof window.attachVoiceInput === "function") {
      window.attachVoiceInput(mic, ta, () => {
        state.task_text = ta.value;
        cfSaveDraft(cfName, state);
      });
    } else {
      mic.hidden = true;
    }
    return;
  }

  // 7 確認: 選んだ内容の一覧と「作成」
  const payload = cfBuildPayload(state);
  const rows = [
    ["runner", payload.runner || "（スキップ）"],
    ["ブロック", payload.block || "（スキップ）"],
    ["モデル", payload.model || "（スキップ）"],
    ["thinking", payload.thinking || "（スキップ）"],
    ["global_round", payload.global_round || "（自動）"],
    ["round", payload.round || "（自動）"],
    ["tag", payload.tag || "（なし）"],
    ["task_text", (payload.task_text || "").slice(0, 200) || "（空）"],
  ];
  for (const [label, value] of rows) {
    const row = el("div", "lv-kv");
    row.appendChild(el("span", "lv-kv-label", label));
    row.appendChild(el("span", "lv-kv-value", value));
    body.appendChild(row);
  }
}

/**
 * 最近の文書（running / review / done / Workflow の実行の成果物）を並べる。
 * 押すと task_text に `waggle:<tok>:path:<path>` の行を足す（もう一度押すと消す・複数可）。
 */
function cfDocsBox(state, ta) {
  const box = el("div", "cf-docs");
  box.appendChild(el("div", "cf-run-label", "最近の文書（押すと task_text に貼ります・複数可）"));
  const list = el("div", "cf-choices");
  box.appendChild(list);
  const draw = () => {
    list.innerHTML = "";
    if (!Array.isArray(cfDocs)) {
      list.appendChild(cfNote("読み込み中…"));
      return;
    }
    if (cfDocs.length === 0) {
      list.appendChild(cfNote("文書がありません"));
      return;
    }
    for (const d of cfDocs) {
      const ref = String(d.ref || "");
      const on = String(state.task_text || "").split("\n").some((l) => l.trim() === ref);
      const b = document.createElement("button");
      b.className = "cf-choice" + (on ? " active" : "");
      b.textContent = `${d.token}/${d.label}${d.description ? ` — ${d.description}` : ""}`;
      b.addEventListener("click", () => {
        state.task_text = cfToggleRef(state.task_text, ref, !on);
        ta.value = state.task_text;
        cfSaveDraft(cfName, state);
        draw();
      });
      list.appendChild(b);
    }
  };
  draw();
  if (!Array.isArray(cfDocs)) {
    cfFetchJson(`/api/projects/${encodeURIComponent(cfName)}/card/docs`)
      .then((data) => { cfDocs = Array.isArray(data.items) ? data.items : []; })
      .catch(() => { cfDocs = []; })
      .finally(draw);
  }
  // 手で書き換えたときも選択の印を合わせる
  ta.addEventListener("input", draw);
  return box;
}

/** 見出し・中身・ボタンを全部描き直す。 */
function cfRender() {
  if (!cfState) return;
  const info = cfStepInfo(cfState);
  const step = document.getElementById("cf-step");
  if (step) step.textContent = info.label;
  const title = document.getElementById("cf-name");
  if (title) title.textContent = info.title;
  const warn = document.getElementById("cf-warn");
  if (warn) {
    warn.textContent = cfWarn;
    warn.hidden = !cfWarn;
  }
  cfRenderBody();
  cfSetError(cfError);

  const back = document.getElementById("cf-back");
  if (back) {
    back.hidden = false; // 読み込み失敗（cfRenderFatal）から戻すため必ず出す
    back.disabled = cfBusy;
    back.textContent = info.id === "runner" ? "Close" : "‹ Back";
  }
  const skip = document.getElementById("cf-skip");
  if (skip) {
    skip.hidden = !cfCanSkip(cfState);
    skip.disabled = cfBusy;
  }
  const next = document.getElementById("cf-next");
  if (next) {
    next.hidden = false;
    next.textContent = info.id === "confirm" ? "作成" : "Next";
    next.disabled = cfBusy;
  }
  const restart = document.getElementById("cf-restart");
  if (restart) restart.hidden = false;
}

/** 次へ（確認画面なら作成）。 */
async function cfGoNext() {
  if (!cfState || cfBusy) return;
  if (cfState.step === "confirm") {
    await cfCreate();
    return;
  }
  const err = cfNext(cfState);
  cfError = err || "";
  if (!err) cfSaveDraft(cfName, cfState);
  cfRender();
}

function cfGoSkip() {
  if (!cfState || cfBusy) return;
  const err = cfSkip(cfState);
  cfError = err || "";
  if (!err) cfSaveDraft(cfName, cfState);
  cfRender();
}

function cfGoBack() {
  if (!cfState || cfBusy) return;
  const res = cfBack(cfState);
  if (res === "close") {
    cfClose();
    return;
  }
  cfError = "";
  cfSaveDraft(cfName, cfState);
  cfRender();
}

/** 9 確認 → 作成（成功したら書きかけを消し，ツリーの todo で task 表示）。 */
async function cfCreate() {
  if (!cfState || cfBusy) return;
  cfBusy = true;
  cfSetError("");
  cfRender();
  try {
    const res = await apiFetch(
      `/api/projects/${encodeURIComponent(cfName)}/card`,
      {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(cfBuildPayload(cfState)),
      }
    );
    const out = await res.json().catch(() => ({}));
    if (!res.ok) throw new Error(out.detail || "作成できませんでした");
    window.SaiShell.ok();
    cfClearDraft(cfName);
    cfState = null;
    const sheet = cfSheet();
    if (sheet) sheet.hidden = true;
    // 作成するとツリーの todo に出て，選ばれた状態で task 表示になる（計画 15 §1.5）
    if (out.run_id != null && typeof listHash === "function") {
      location.hash = listHash(cfName, Number(out.run_id), "task");
    }
  } catch (e) {
    window.SaiShell.fail();
    cfSetError((e && e.message) || "作成できませんでした");
  } finally {
    cfBusy = false;
    if (cfState) cfRender();
  }
}

// --- 画面の登録（ブラウザでのみ・計画 15 §1.5） ---

if (typeof window !== "undefined") {
  // 開くのは List のサイドバーの見出しの行の ＋（lvRenderDrawer が呼ぶ・U01 §4）
  const close = document.getElementById("cf-close");
  if (close) close.addEventListener("click", () => cfClose());
  const mask = document.getElementById("cf-mask");
  if (mask) mask.addEventListener("click", () => cfClose());
  const back = document.getElementById("cf-back");
  if (back) back.addEventListener("click", () => cfGoBack());
  const skip = document.getElementById("cf-skip");
  if (skip) skip.addEventListener("click", () => cfGoSkip());
  const next = document.getElementById("cf-next");
  if (next) next.addEventListener("click", () => void cfGoNext());
  const restart = document.getElementById("cf-restart");
  if (restart) {
    restart.addEventListener("click", () => {
      cfClearDraft(cfName);
      cfState = cfNewDraft();
      cfError = "";
      cfRender();
    });
  }
  // 開いたときの住所から離れたらフォームを閉じる（隠れた場所に残さない・U10）
  window.addEventListener("hashchange", () => {
    if (cfState && cfShouldClose(cfOpenedHash, location.hash)) cfClose();
  });
}

// --- node からの読み込み（tests/test_cardform.js・計画 15 §7） ---
if (typeof module !== "undefined" && module.exports) {
  module.exports = {
    CF_STEPS,
    CF_STEP_HINTS,
    cfStepHint,
    cfNewDraft,
    cfStepInfo,
    cfAfterBlock,
    cfNext,
    cfCanSkip,
    cfSkip,
    cfBack,
    cfBuildPayload,
    cfToggleRef,
    cfAppendTaskText,
    cfTargetName,
    cfShouldClose,
    cfDraftKey,
    cfLoadDraft,
    cfSaveDraft,
    cfClearDraft,
  };
}
