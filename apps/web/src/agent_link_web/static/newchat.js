"use strict";
// Chats の一覧から「新しいチャット」を始める（実装計画 U04 §5）。
//
// プロジェクト・block・runner・model・thinking・最初の文を選び、
// カード（task.md）を作って実行し、その会話の画面へ移る。
// サーバに新しい処理は足さず、既にある 2 本の API を続けて呼ぶ:
//   POST /api/projects/{name}/card          → {run_id, …}（cfBuildPayload の形）
//   POST /api/projects/{name}/runs/{id}/start
// 選択肢は GET /api/projects と GET /api/projects/{name}/card/options から取る。
//
// 起動の書き出しは常駐の周期（約 5 秒）だけ遅れるので、app.js が
// sessionStorage の印（NC_RUN_KEY）を見て 60 秒まで「起動を待っています…」を出す。
//
// DOM を使わない関数（ncValidate / ncModelChoices）は末尾で node に渡す
// （tests/test_newchat.js）。一番外側の名前は nc / NC_ で始める。

// 前回選んだプロジェクト・runner を覚えておく場所
const NC_PROJECT_KEY = "agent_link_web_newchat_project";
const NC_RUNNER_KEY = "agent_link_web_newchat_runner";
// 起動待ちの見分け（app.js の NEWCHAT_RUN_KEY と同じ値）
const NC_RUN_KEY = "agent_link_web_newchat_run";

/** localStorage から読む（読めなければ fallback）。 */
function ncStored(key, fallback) {
  try {
    const v = localStorage.getItem(key);
    return v == null || v === "" ? fallback : v;
  } catch {
    return fallback;
  }
}

/** localStorage に書く（書けなくても動く）。 */
function ncStore(key, value) {
  try { localStorage.setItem(key, String(value)); } catch { /* 保存できなくても動く */ }
}

/**
 * runner に合わせた model の選択肢（U04 §5 の 4）。
 *
 * - cc（runner が空なら cc 扱い）: `cc_models`
 * - user / gl: モデルは無い
 *
 * @param {object|null|undefined} options `card/options` の応答
 * @param {string} runner
 * @returns {string[]}
 */
function ncModelChoices(options, runner) {
  const o = options || {};
  if (runner === "cc" || runner === "") return (o.cc_models || []).map(String);
  return [];
}

/**
 * 入力の検査（U04 §5）。プロジェクト・block・最初の文が空でないこと。
 *
 * @param {object|null|undefined} state
 * @returns {{ok: boolean, error?: string}}
 */
function ncValidate(state) {
  const s = state || {};
  if (!s.project) return { ok: false, error: "プロジェクトを選んでください" };
  if (!s.block) return { ok: false, error: "block を選んでください" };
  if (!String(s.task_text || "").trim()) {
    return { ok: false, error: "最初の文を書いてください" };
  }
  return { ok: true };
}

/** 見出し付きの入力 1 つ。 */
function ncField(labelText, control) {
  const wrap = saiEl("label", "nc-field");
  wrap.appendChild(saiEl("span", "nc-label", labelText));
  wrap.appendChild(control);
  return wrap;
}

/** 選択肢の一覧（{value, label} の配列）。emptyLabel があれば空の選択肢を先頭に置く。 */
function ncSelect(values, value, onChange, emptyLabel) {
  const sel = document.createElement("select");
  sel.className = "nc-select";
  if (emptyLabel != null) {
    const opt = document.createElement("option");
    opt.value = "";
    opt.textContent = emptyLabel;
    sel.appendChild(opt);
  }
  for (const v of values) {
    const opt = document.createElement("option");
    opt.value = String(v.value);
    opt.textContent = String(v.label != null ? v.label : v.value);
    sel.appendChild(opt);
  }
  sel.value = value == null ? "" : String(value);
  sel.addEventListener("change", () => onChange(sel.value));
  return sel;
}

// 住所の名前の形（project・block）。合わない値は空として扱う
const NC_NAME_RE = /^[A-Za-z0-9][A-Za-z0-9_.-]{0,63}$/;

/**
 * 住所 `#/newchat?project=&block=` から {project, block} を取る（G3-C3）。
 * 違う住所なら null。形の合わない値は "" にする（ncOpen が既定に戻す）。
 *
 * @param {string} hash
 * @returns {{project: string, block: string} | null}
 */
function ncParsePreset(hash) {
  const h = String(hash || "");
  if (h !== "#/newchat" && !h.startsWith("#/newchat?")) return null;
  const q = new URLSearchParams(h.slice("#/newchat".length).replace(/^\?/, ""));
  const pick = (key) => {
    const v = q.get(key) || "";
    return NC_NAME_RE.test(v) ? v : "";
  };
  return { project: pick("project"), block: pick("block") };
}

/**
 * 「＋」や住所（#/newchat）から新しいチャットのシートを開く。
 *
 * @param {{project?: string, block?: string}} [preset] 住所で頼まれた project と block（任意）
 */
async function ncOpen(preset) {
  const want = preset || {};
  let projects = [];
  try {
    const data = await saiApi("/api/projects");
    projects = (data.projects || []).map((p) => p && p.name).filter(Boolean);
  } catch (e) {
    saiToast(e.message || "プロジェクトを読めませんでした", "error");
    return;
  }
  if (projects.length === 0) {
    saiToast("プロジェクトがありません", "error");
    return;
  }

  // project は住所の物（一覧にあれば）→ 前回の値 → 先頭の順
  const project = [want.project, ncStored(NC_PROJECT_KEY, "")].find((p) => p && projects.includes(p)) || projects[0];
  const state = {
    project,
    block: "main",
    wantBlock: want.block || "", // 住所で頼まれた block（loadOptions で 1 回だけ当てる）
    notice: "",
    runner: ncStored(NC_RUNNER_KEY, "cc"),
    provider: "",
    model: "",
    thinking: "",
    task_text: "",
    options: null,
    touched: false,
  };

  const body = saiEl("div", "nc-body");
  const sheet = saiSheet({ title: "新しいチャット", body });

  // 今の state で入力欄を作り直す（runner を変えたときに model を出し分けるため）。
  function render() {
    body.innerHTML = "";
    const options = state.options || {};

    // 1. プロジェクト（前回選んだ物を既定にする）
    body.appendChild(
      ncField(
        "プロジェクト",
        ncSelect(
          projects.map((p) => ({ value: p, label: p })),
          state.project,
          (v) => {
            state.project = v;
            state.touched = true;
            ncStore(NC_PROJECT_KEY, v);
            void loadOptions();
          }
        )
      )
    );

    // 2. block（既定は main）
    const blocks = (options.blocks || []).map((b) => ({
      value: String(b.id),
      label: String(b.label || b.id),
    }));
    body.appendChild(
      ncField(
        "block",
        ncSelect(blocks, state.block, (v) => {
          state.block = v;
          state.touched = true;
          updateStart();
        })
      )
    );
    // 住所で頼まれた block が無かったときの注意（main で開いている）
    if (state.notice) body.appendChild(saiEl("div", "nc-error", state.notice));

    // 3. runner（既定は前回の値、無ければ cc）
    body.appendChild(
      ncField(
        "runner",
        ncSelect(
          (options.runners || []).map((r) => ({ value: r, label: r })),
          state.runner,
          (v) => {
            state.runner = v;
            state.touched = true;
            state.model = "";
            ncStore(NC_RUNNER_KEY, v);
            render();
          }
        )
      )
    );

    // 4. model（runner に合わせて出し分ける。空なら block の既定）
    const models = ncModelChoices(options, state.runner);
    body.appendChild(
      ncField(
        "model",
        ncSelect(
          models.map((m) => ({ value: m, label: m })),
          state.model,
          (v) => {
            state.model = v;
            state.touched = true;
            updateStart();
          },
          "（block の既定）"
        )
      )
    );

    // 5. thinking（空なら block の既定）
    body.appendChild(
      ncField(
        "thinking",
        ncSelect(
          (options.thinking || []).map((t) => ({ value: t, label: t })),
          state.thinking,
          (v) => {
            state.thinking = v;
            state.touched = true;
            updateStart();
          },
          "（block の既定）"
        )
      )
    );

    // 6. 最初の文（複数行）
    const ta = document.createElement("textarea");
    ta.className = "nc-text";
    ta.rows = 4;
    ta.placeholder = "最初の文";
    ta.value = state.task_text;
    ta.addEventListener("input", () => {
      state.task_text = ta.value;
      state.touched = true;
      updateStart();
    });
    body.appendChild(ncField("最初の文", ta));

    const err = saiEl("div", "nc-error", "");
    err.hidden = true;
    body.appendChild(err);

    const start = saiEl("button", "sai-btn primary nc-start", "始める");
    start.type = "button";
    start.addEventListener("click", () => void startChat(start, err));
    body.appendChild(start);

    // 空のときは押せない（AC3）。理由は触ったあとだけ出す。
    function updateStart() {
      const v = ncValidate(state);
      start.disabled = !v.ok;
      err.textContent = v.ok ? "" : v.error || "";
      err.hidden = v.ok || !state.touched;
    }
    updateStart();
  }

  // プロジェクトの選択肢を読み直して描き直す。
  async function loadOptions() {
    body.innerHTML = "";
    body.appendChild(saiEl("div", "nc-loading", "読み込み中…"));
    try {
      state.options = (await saiApi(
        `/api/projects/${encodeURIComponent(state.project)}/card/options`
      )) || {};
    } catch (e) {
      saiToast(e.message || "選択肢を読めませんでした", "error");
      state.options = {};
    }
    // 住所で頼まれた block は 1 回だけ当てる（無ければ注意を出して既定へ）
    const ids = (state.options.blocks || []).map((b) => String(b.id));
    state.notice = "";
    if (state.wantBlock) {
      if (ids.includes(state.wantBlock)) state.block = state.wantBlock;
      else state.notice = `block ${state.wantBlock} がありません（sai-ts init を確かめてください）`;
      state.wantBlock = "";
    }
    // block の既定（main があれば main、無ければ先頭）
    if (!ids.includes(state.block)) state.block = ids.includes("main") ? "main" : (ids[0] || "");
    state.model = "";
    render();
  }

  // カードを作る → 実行する → その会話へ移る。
  async function startChat(start, err) {
    const v = ncValidate(state);
    if (!v.ok) {
      state.touched = true;
      err.textContent = v.error || "";
      err.hidden = false;
      start.disabled = true;
      return;
    }
    start.disabled = true;
    try {
      const payload = typeof cfBuildPayload === "function"
        ? cfBuildPayload(state)
        : {
            runner: state.runner, block: state.block, provider: "", model: state.model,
            thinking: state.thinking, task_text: state.task_text,
          };
      // start は round・global_round が空だと断るので、新しいチャットは 1 回目として始める
      payload.round = payload.round || "1";
      payload.global_round = payload.global_round || "1";
      const card = await saiApi(`/api/projects/${encodeURIComponent(state.project)}/card`, {
        method: "POST",
        body: payload,
      });
      const runId = card && card.run_id;
      if (runId == null) throw new Error("run_id を読めませんでした");
      await saiApi(
        `/api/projects/${encodeURIComponent(state.project)}/runs/${encodeURIComponent(runId)}/start`,
        { method: "POST" }
      );
      // 起動待ちの見分け（app.js が 60 秒まで「起動を待っています…」を出す）
      try {
        sessionStorage.setItem(NC_RUN_KEY, JSON.stringify({ run_id: runId, at: Date.now() }));
      } catch { /* 保存できなくても動く */ }
      sheet.close();
      location.hash = `#/run/${encodeURIComponent(state.project)}/${encodeURIComponent(runId)}`;
    } catch (e) {
      start.disabled = false;
      saiToast(e.message || "始められませんでした", "error");
    }
  }

  await loadOptions();
}

// Chats の一覧の「＋」に結び付ける（index.html の #newchat-button）。
if (typeof document !== "undefined") {
  const btn = document.getElementById("newchat-button");
  if (btn) btn.addEventListener("click", () => void ncOpen());
}

// 住所 #/newchat?project=&block=（sai-ts の「Chats で開く」の行き先・G3-C3）。
// 一覧（#/ の days）を出してから、その project と block を選んだ欄を開く。
// 住所は #/ に戻す（replaceState は hashchange を起こさないので route を直に呼ぶ）。
if (typeof window !== "undefined") {
  (window.SaiRoutes = window.SaiRoutes || []).push({
    match(hash) {
      return ncParsePreset(hash);
    },
    view: "view-days",
    tab: "agent",
    hideTabbar: false,
    interval: 0,
    async open(params) {
      history.replaceState(null, "", "#/");
      await route();
      await ncOpen(params);
    },
    async poll() {},
  });
}

// --- node からの読み込み（tests/test_newchat.js） ---
if (typeof module !== "undefined" && module.exports) {
  module.exports = { ncValidate, ncModelChoices, ncParsePreset, NC_RUN_KEY };
}
