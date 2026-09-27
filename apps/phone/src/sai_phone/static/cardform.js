// sai-phone の task.md 作成フォーム（計画 15 §1.5）の本体。
//
// List 画面右上の ＋ で下から出る。TUI のウィザード（sai-link-tui の card_wizard.rs）と
// 同じ順に 1 画面ずつ進み、スキップ・戻る・番号の飛び方も同じ規則にする:
//   1 引き継ぐ review → 2 runner → 3 ブロック → 4 入力ファイル（4.1, 4.2 …）
//   → 5 provider（pi のみ）→ 6 モデル → 7 thinking → 8 ラウンド
//   → 9 task_text（🎤 音声入力）→ 10 確認 → 作成
//   user / gl は 3 から 8 へ、cc は 6（入力が無ければ）、入力が無ければ 5 へ飛ぶ。
// 選択肢は `GET …/card/options`、手順 4 は `GET …/card/inputs?block=<id>`、
// 作成は `POST …/card`（値の検査はサーバー側でも行う・計画 15 §3.3）。
// 書きかけはプロジェクトごとに localStorage へ残し、作成に成功したら消す。
// 進み方（DOM を使わない関数）は末尾で node に渡す（tests/test_cardform.js）。
"use strict";

// --- 定数 ---

// 手順の地図（番号は 10 固定。スキップや自動で飛んだ画面は番号が飛ぶ・計画 15 §1.5）
const CF_STEPS = [
  { id: "done", no: 1, title: "引き継ぐ review" },
  { id: "runner", no: 2, title: "runner" },
  { id: "block", no: 3, title: "ブロック" },
  { id: "inputs", no: 4, title: "入力ファイル" },
  { id: "provider", no: 5, title: "provider" },
  { id: "model", no: 6, title: "モデル" },
  { id: "thinking", no: 7, title: "thinking" },
  { id: "rounds", no: 8, title: "ラウンド" },
  { id: "task_text", no: 9, title: "task_text" },
  { id: "confirm", no: 10, title: "確認" },
];
// 10 画面それぞれの見出しの下に出す説明（端末に慣れていない人向け・U5 §3）。
// 画面の流れ・スキップ・戻る・保存の動きは変えない（見出しの下に文言を足すだけ）。
const CF_STEP_HINTS = {
  done: "前に出した仕事（確認済みのもの）を、このカードに引き継ぎたいときに選びます。",
  runner: "このカードをどの AI に渡すかです。cc や pi など、名前で選びます。",
  block: "どんな順番で、どのファイルを見て、どう直せばよいかを書いた指示書です。",
  inputs: "指示書の中で波括弧で囲んだ場所に渡す文書（waggle トークン）を選びます。",
  provider: "モデルの入手元です。ここでどの入手元にするかが決まります。",
  model: "使うモデルの名前です。動かすエージェントによって出し方が違います。",
  thinking: "考えの深さです。難しい仕事で深めにすると時間がかかりますが、精度が上がります。",
  rounds: "同じ仕事を引き継ぎながら何回回すかです。回数を増やすほど長く続けます。",
  task_text: "依頼文です。エージェントはこの文章に従って動きます。",
  confirm: "作った依頼文（task.md）を最後に読みます。「保存」でファイルができます。",
};
// 書きかけの置き場（プロジェクトごとに分ける・計画 15 §1.5）
const CF_DRAFT_KEY = "sai_phone_card_draft";
// user / gl は手順 4〜7 が対象外（TUI の advance_from_block と同じ）
const CF_NO_INPUT_RUNNERS = ["user", "gl"];

// --- 進み方（DOM を使わない・tests/test_cardform.js） ---

/** 新しい書きかけ（すべてスキップした状態から始める）。 */
function cfNewDraft() {
  return {
    step: "done",
    inputIdx: 0,
    doneIds: [],        // 引き継ぐ run_id（複数可）
    runner: "",         // "" = スキップ（TUI と同じく pi 扱い）
    block: "",
    inputs: {},         // 入力 id → [waggle トークン（接頭辞なし）]
    provider: "",
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
 * 今いる手順の番号と題名（表示は TUI と同じ「Step n/10」・入力は 4.1, 4.2 …）。
 * 番号は 10 固定で、スキップした分は飛ぶ（user / gl なら 3 → 8）。
 */
function cfStepInfo(state) {
  const entry = cfStepEntry(state.step);
  const label =
    entry.id === "inputs"
      ? `Step 4.${Number(state.inputIdx || 0) + 1}/10`
      : `Step ${entry.no}/10`;
  return { id: entry.id, no: entry.no, title: entry.title, label };
}

/** 候補トークンの合計数（必須の判定で使う・0 件なら選択肢が無い扱い）。 */
function cfTokenCount(field) {
  let n = 0;
  for (const run of (field && field.runs) || []) {
    n += ((run && run.tokens) || []).length;
  }
  return n;
}

/** user / gl か、ブロック未選択か、定義が無いか → 手順 4 は無い（TUI と同じ分岐）。 */
function cfHasInputStep(state, fields) {
  if (CF_NO_INPUT_RUNNERS.indexOf(String(state.runner || "")) >= 0) return false;
  if (!state.block) return false;
  return Array.isArray(fields) && fields.length > 0;
}

/** ブロック確定後の行き先（番号の飛び方は TUI の advance_from_block と同じ）。 */
function cfAfterBlock(state, fields) {
  state.inputIdx = 0;
  state.inputs = {}; // ブロックが変われば前の選択は捨てる（TUI の prepare_block_inputs と同じ）
  const runner = String(state.runner || "");
  if (runner === "user" || runner === "gl") {
    // gl は起動時のタスク書き込みなし・user は人がやる作業なので 4〜7 は丸ごと省く
    state.provider = "";
    state.model = "";
    return "rounds";
  }
  if (runner === "cc") {
    state.provider = ""; // cc には provider が無い
    return cfHasInputStep(state, fields) ? "inputs" : "model";
  }
  return cfHasInputStep(state, fields) ? "inputs" : "provider";
}

/** 手順 4 を通ったあとの行き先（runner 別・TUI の after_inputs と同じ）。 */
function cfAfterInputs(state) {
  const runner = String(state.runner || "");
  if (runner === "user" || runner === "gl") return "rounds";
  if (runner === "cc") return "model";
  return "provider";
}

/** 必須の入力で止まる理由（止めないなら null）。 */
function cfRequiredError(state, fields) {
  if (state.step !== "inputs") return null;
  const field = (fields || [])[Number(state.inputIdx || 0)];
  if (!field || !field.required) return null;
  const chosen = (state.inputs || {})[field.id] || [];
  if (chosen.length > 0) return null;
  // 候補 0 件は選択肢が無いため許容（TUI と同じ）
  if (cfTokenCount(field) === 0) return null;
  return `${field.id}: 1 個以上選択してください（必須）`;
}

/** 次へ進めるか。進めれば null、ダメなら理由（その画面に止まる）。 */
function cfNext(state, fields) {
  const blocked = cfRequiredError(state, fields);
  if (blocked) return blocked;
  switch (state.step) {
    case "done":
      state.step = "runner";
      return null;
    case "runner":
      state.step = "block";
      return null;
    case "block":
      state.step = cfAfterBlock(state, fields);
      return null;
    case "inputs":
      if (Number(state.inputIdx || 0) + 1 < (fields || []).length) {
        state.inputIdx = Number(state.inputIdx || 0) + 1;
        return null;
      }
      state.step = cfAfterInputs(state);
      return null;
    case "provider":
      state.step = "model";
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
  return ["runner", "block", "inputs", "provider", "model", "thinking", "rounds", "task_text"].includes(
    state.step
  );
}

/** スキップ（その項目は sai-link の既定値に任せる）。止めれば理由を返す。 */
function cfSkip(state, fields) {
  if (!cfCanSkip(state)) return "スキップできません";
  switch (state.step) {
    case "runner":
      state.runner = "";
      state.step = "block";
      return null;
    case "block":
      state.block = "";
      state.step = cfAfterBlock(state, []);
      return null;
    case "inputs": {
      const field = (fields || [])[Number(state.inputIdx || 0)];
      if (field && field.required) return `${field.id}: 必須のためスキップできません`;
      if (field) delete state.inputs[field.id];
      if (Number(state.inputIdx || 0) + 1 < (fields || []).length) {
        state.inputIdx = Number(state.inputIdx || 0) + 1;
        return null;
      }
      state.step = cfAfterInputs(state);
      return null;
    }
    case "provider":
      state.provider = "";
      state.step = "model";
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
      // TUI はここで作成するが、本フォームは 9 task_text → 10 確認 があるので次へ進む
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

/** 1 つ前へ戻る。引き継ぎ画面なら "close"（フォームを閉じる合図）。 */
function cfBack(state) {
  switch (state.step) {
    case "done":
      return "close";
    case "runner":
      state.step = "done";
      return null;
    case "block":
      state.step = "runner";
      return null;
    case "inputs":
      if (Number(state.inputIdx || 0) > 0) {
        state.inputIdx = Number(state.inputIdx || 0) - 1;
        return null;
      }
      state.step = "block";
      return null;
    case "provider":
      state.step = "block";
      return null;
    case "model":
      // 戻り先は runner 別（cc → ブロック・それ以外 → provider・TUI と同じ）
      state.step = state.runner === "cc" ? "block" : "provider";
      return null;
    case "thinking":
      state.step = "model";
      return null;
    case "rounds":
      // user / gl → ブロック、pi / cc → thinking（TUI と同じ）
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
 * トークンの選択を 1 つ切り替える（手順 4）。
 * 戻り値は理由（変えられなければその文言、変えられたら null）。
 */
function cfToggleToken(state, fields, fieldId, token) {
  const field = (fields || []).find((f) => f && f.id === fieldId);
  if (!field) return "入力が見つかりません";
  if (!state.inputs || typeof state.inputs !== "object") state.inputs = {};
  const current = state.inputs[fieldId] || [];
  const at = current.indexOf(token);
  if (at >= 0) {
    current.splice(at, 1);
    return null;
  }
  if (field.multiple === false) {
    state.inputs[fieldId] = [token]; // 単一選択は常に 1 個に絞る（TUI と同じ）
    return null;
  }
  const max = Number(field.max_tokens);
  if (field.max_tokens != null && Number.isFinite(max) && current.length >= max) {
    return `max ${max} tokens までです（選択中: ${current.length}）`;
  }
  state.inputs[fieldId] = current.concat([token]);
  return null;
}

/** `POST …/card` に送る JSON（項目名は計画 12 §3.2 の形のまま）。 */
function cfBuildPayload(state) {
  const inputs = {};
  for (const key of Object.keys(state.inputs || {})) {
    const tokens = (state.inputs || {})[key] || [];
    if (Array.isArray(tokens) && tokens.length > 0) {
      // 送る値は `waggle:` 形式のカンマ区切り（sai-link の validate_card_spec と同じ）
      inputs[key] = tokens.map((t) => `waggle:${t}`).join(", ");
    }
  }
  const rounds = state.rounds || {};
  return {
    done_ids: (state.doneIds || []).map(Number).filter((n) => Number.isFinite(n)),
    runner: state.runner || "",
    block: state.block || "",
    inputs,
    provider: state.provider || "",
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
    if (!CF_STEPS.some((s) => s.id === obj.step)) return null; // 手順が無い書きかけは無し扱い
    // 形が崩れた項目は新しい値でそろえる
    return Object.assign(cfNewDraft(), obj, {
      rounds: Object.assign(cfNewDraft().rounds, obj.rounds || {}),
      inputs: obj.inputs && typeof obj.inputs === "object" ? obj.inputs : {},
      doneIds: Array.isArray(obj.doneIds) ? obj.doneIds : [],
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
let cfFields = [];     // 選んだブロックの `GET …/card/inputs` の fields
let cfError = "";      // 手順の下に出す 1 行
let cfWarn = "";       // options の warnings（settings.json の注意など）
let cfBusy = false;    // 送信中

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

/** 手順 4 の定取り（ブロックが決まったときだけ呼ぶ・失敗したら手順 4 は無し）。 */
async function cfLoadFields(block) {
  const data = await cfFetchJson(
    `/api/projects/${encodeURIComponent(cfName)}/card/inputs?block=${encodeURIComponent(block)}`
  );
  return Array.isArray(data.fields) ? data.fields : [];
}

/** ＋ を押した入口（書きかけがあれば続きから・計画 15 §1.5）。 */
async function cfOpen() {
  const parsed = typeof parseListHash === "function" ? parseListHash(location.hash) : null;
  if (parsed && parsed.name && parsed.name !== cfName) {
    // プロジェクトが変われば選択肢と手順 4 の材料は取り直す（別プロジェクトの選択肢を出さないため）
    cfName = parsed.name;
    cfOptions = null;
    cfFields = [];
    cfState = null;
    cfError = "";
  }
  if (!cfName) return;

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

  // 続きから开くときも手順 4 の材料は取りに行く（取れなければその画面で止まる）
  if (cfState.block && cfState.step) {
    try {
      cfFields = await cfLoadFields(cfState.block);
    } catch (e) {
      cfFields = [];
      cfError = (e && e.message) || "入力の候補を読み込めませんでした";
    }
  } else {
    cfFields = [];
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

  if (info.id === "done") {
    const done = (cfOptions && cfOptions.done) || [];
    if (done.length === 0) {
      body.appendChild(cfNote("引き継げる実行（done）がありません"));
    } else {
      const box = document.createElement("div");
      box.className = "cf-choices";
      for (const row of done) {
        const on = (state.doneIds || []).includes(Number(row.run_id));
        const b = document.createElement("button");
        b.className = "cf-choice" + (on ? " active" : "");
        b.textContent = `${row.label || `ID${row.run_id}`}${row.description ? ` — ${row.description}` : ""}`;
        b.addEventListener("click", () => {
          const id = Number(row.run_id);
          const ids = (state.doneIds || []).slice();
          const at = ids.indexOf(id);
          if (at >= 0) ids.splice(at, 1);
          else ids.push(id);
          state.doneIds = ids;
          cfSaveDraft(cfName, state);
          cfRenderBody();
        });
        box.appendChild(b);
      }
      body.appendChild(box);
      body.appendChild(
        cfNote("選んだ実行の review.md が本文に付きます（複数可・選ばなくても先へ進めます）")
      );
    }
    return;
  }

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
    body.appendChild(cfNote("スキップすると pi 扱い（sai-link の既定どおり）"));
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
    body.appendChild(cfNote("選ぶと次に手順 4（そのブロックの入力）が出ます"));
    return;
  }

  if (info.id === "inputs") {
    const field = cfFields[Number(state.inputIdx || 0)];
    if (!field) {
      body.appendChild(cfNote("入力の定義がありません（戻るかスキップしてください）"));
      return;
    }
    const head = el("div", "cf-field-head");
    head.appendChild(el("span", "cf-field-label", String(field.label || field.id)));
    if (field.required) head.appendChild(el("span", "cf-required", "必須"));
    body.appendChild(head);
    const runs = Array.isArray(field.runs) ? field.runs : [];
    if (runs.length === 0) body.appendChild(cfNote("選べるトークンがありません"));
    for (const run of runs) {
      body.appendChild(el("div", "cf-run-label", String(run.label || `ID${run.run_id}`)));
      const box = document.createElement("div");
      box.className = "cf-choices";
      for (const t of run.tokens || []) {
        const on = ((state.inputs || {})[field.id] || []).includes(String(t.token));
        const b = document.createElement("button");
        b.className = "cf-choice" + (on ? " active" : "");
        b.textContent = String(t.label || t.token);
        b.addEventListener("click", () => {
          const err = cfToggleToken(state, cfFields, field.id, String(t.token));
          cfSetError(err || "");
          cfSaveDraft(cfName, state);
          cfRenderBody();
        });
        box.appendChild(b);
      }
      body.appendChild(box);
    }
    return;
  }

  if (info.id === "provider") {
    const providers = Object.keys((cfOptions && cfOptions.providers) || {});
    body.appendChild(
      cfChoiceList(
        providers.map((p) => ({ value: p, label: p })),
        state.provider,
        (v) => {
          state.provider = v;
          cfSaveDraft(cfName, state);
          cfRenderBody();
        }
      )
    );
    body.appendChild(cfNote("スキップすると provider の自動解決に任せます"));
    return;
  }

  if (info.id === "model") {
    let models = [];
    if (state.runner === "cc") {
      models = (cfOptions && cfOptions.cc_models) || [];
    } else if (state.provider) {
      models = ((cfOptions && cfOptions.providers) || {})[state.provider] || [];
    } else {
      // provider をスキップしたときは全部の provider のモデルを並べる
      const all = (cfOptions && cfOptions.providers) || {};
      models = Array.from(new Set(Object.values(all).flat()));
    }
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

  // 10 確認: 選んだ内容の一覧と「作成」
  const payload = cfBuildPayload(state);
  const rows = [
    ["引き継ぐ review", (payload.done_ids || []).map((id) => `ID${id}`).join(", ")],
    ["runner", payload.runner || "（スキップ）"],
    ["ブロック", payload.block || "（スキップ）"],
    ...Object.keys(payload.inputs).map((k) => [k, payload.inputs[k]]),
    ["provider", payload.provider || "（スキップ）"],
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
    back.textContent = info.id === "done" ? "閉じる" : "‹ 戻る";
  }
  const skip = document.getElementById("cf-skip");
  if (skip) {
    skip.hidden = !cfCanSkip(cfState);
    skip.disabled = cfBusy;
  }
  const next = document.getElementById("cf-next");
  if (next) {
    next.hidden = false;
    next.textContent = info.id === "confirm" ? "作成" : "次へ";
    next.disabled = cfBusy;
  }
  const restart = document.getElementById("cf-restart");
  if (restart) restart.hidden = false;
}

/** 次へ（手順 3 のときは先に手順 4 の定義を取りに行く）。 */
async function cfGoNext() {
  if (!cfState || cfBusy) return;
  if (cfState.step === "confirm") {
    await cfCreate();
    return;
  }
  if (cfState.step === "block" && cfState.block) {
    try {
      cfFields = await cfLoadFields(cfState.block);
    } catch (e) {
      // 取れないと手順 4（必須入力）を書けないので，その場で理由を出して止まる
      cfSetError((e && e.message) || "入力の候補を読み込めませんでした");
      return;
    }
  }
  const err = cfNext(cfState, cfFields);
  cfError = err || "";
  if (!err) cfSaveDraft(cfName, cfState);
  cfRender();
}

function cfGoSkip() {
  if (!cfState || cfBusy) return;
  const err = cfSkip(cfState, cfFields);
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

/** 10 確認 → 作成（成功したら書きかけを消し，ツリーの todo で task 表示）。 */
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
  // List 画面右上の ＋（index.html では hidden のまま置き場だけある・14 §1.5）
  const add = document.getElementById("list-add");
  if (add) {
    add.hidden = false;
    add.addEventListener("click", () => void cfOpen());
  }
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
      cfFields = [];
      cfError = "";
      cfRender();
    });
  }
  // List を離れたときはフォームを閉じる（隠れた場所に残さない）
  window.addEventListener("hashchange", () => {
    if (cfState && typeof parseListHash === "function" && !parseListHash(location.hash)) {
      cfClose();
    }
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
    cfTokenCount,
    cfHasInputStep,
    cfAfterBlock,
    cfAfterInputs,
    cfRequiredError,
    cfNext,
    cfCanSkip,
    cfSkip,
    cfBack,
    cfToggleToken,
    cfBuildPayload,
    cfDraftKey,
    cfLoadDraft,
    cfSaveDraft,
    cfClearDraft,
  };
}
