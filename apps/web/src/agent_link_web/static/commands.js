// Web の「コマンド」画面（U10）。
//
// CLI（link）の `schema --json` から入力欄を作り、命令を実行する。
// 住所は `#/project/<name>/commands`（Project の上部の切り替えの 1 つ）。
//
// - 一覧（左）: CLI の選択・絞り込み・命令の一覧（path の 1 語目でまとめる）。
// - 中身（右）: 選んだ命令の入力欄と「実行」。結果は下に出す。
// - 入力欄の種類は schema の引数から決める（cmdFieldType）。
// - 送る中身は cmdBuildRequest が組む。どちらも DOM を使わない純関数で、
//   tests/test_commands.js が確かめる。
"use strict";

// --- 純関数（DOM を使わない） ---

/**
 * 引数 1 つから入力欄の種類を決める。
 * 返すのは { kind }（kind は check / select / number / lines / text のいずれか）。
 */
function cmdFieldType(arg) {
  const a = arg || {};
  if (a.multiple) return { kind: "lines" };
  if (a.kind === "flag") return { kind: "check" };
  if (a.type === "enum") return { kind: "select", values: Array.isArray(a.values) ? a.values : [] };
  if (a.type === "int" || a.type === "float") return { kind: "number" };
  return { kind: "text" };
}

/** 入力欄の 1 つの値を、送る型にする。数として読めなければ Error。 */
function cmdScalar(arg, text) {
  const name = (arg && arg.name) || "";
  const s = String(text == null ? "" : text).trim();
  if (arg && arg.type === "int") {
    if (!/^-?\d+$/.test(s)) throw new Error(`${name} は整数で入力してください`);
    return parseInt(s, 10);
  }
  if (arg && arg.type === "float") {
    const n = Number(s);
    if (!Number.isFinite(n)) throw new Error(`${name} は数で入力してください`);
    return n;
  }
  return s;
}

/**
 * 入力欄の値から `POST …/commands/run` に送る中身を作る。
 * values は引数名 → 入力欄の値（チェックは真偽、複数行は文字列）。
 * stdinText は命令の stdin が json / text のときの標準入力。
 */
function cmdBuildRequest(cliKey, command, values, stdinText) {
  const args = {};
  const vals = values || {};
  for (const arg of (command && command.args) || []) {
    if (!arg || !arg.web) continue;
    const name = arg.name;
    const raw = vals[name];
    if (arg.kind === "flag") {
      args[name] = !!raw;
      continue;
    }
    if (arg.multiple) {
      const lines = String(raw == null ? "" : raw)
        .split("\n")
        .map((line) => line.trim())
        .filter((line) => line !== "");
      if (lines.length) args[name] = lines.map((line) => cmdScalar(arg, line));
      continue;
    }
    const rawText = String(raw == null ? "" : raw);
    if (rawText.trim() === "") continue;
    args[name] = cmdScalar(arg, rawText);
  }
  const out = { cli: cliKey, path: ((command && command.path) || []).slice(), args };
  const stdin = stdinText == null ? "" : String(stdinText);
  if (stdin !== "") out.stdin = stdin;
  return out;
}

/** 命令の並びを path の 1 語目でまとめる（表示用）。 */
function cmdGroup(commands) {
  const groups = [];
  const byKey = new Map();
  for (const c of commands || []) {
    const path = c.path || [];
    const key = path[0] || "";
    let g = byKey.get(key);
    if (!g) {
      g = { key, items: [] };
      byKey.set(key, g);
      groups.push(g);
    }
    g.items.push(c);
  }
  return groups;
}

// --- 画面（ブラウザだけ） ---

let cmdName = "";
let cmdClis = null;      // `GET …/commands` の clis
let cmdCliKey = "";      // 選んでいる CLI
let cmdCommand = null;   // 選んでいる命令の schema
let cmdResult = "";      // 結果の欄に出す文字
let cmdFilterText = "";  // 絞り込みの語

function cmdParseHash(hash) {
  const m = String(hash || "").match(/^#\/project\/([^/]+)\/commands\/?$/);
  return m ? { name: decodeURIComponent(m[1]) } : null;
}

function cmdEl(tag, cls, text) {
  const node = document.createElement(tag);
  if (cls) node.className = cls;
  if (text != null) node.textContent = String(text);
  return node;
}

function cmdCommands() {
  const cli = (cmdClis || []).find((c) => c.key === cmdCliKey);
  return (cli && cli.commands) || [];
}

async function cmdOpen(params) {
  cmdName = params.name;
  // 見出しと切り替え
  const title = document.getElementById("cmd-title");
  if (title) title.textContent = cmdName;
  if (typeof fillLinkSeg === "function") fillLinkSeg("cmd-mode-seg", "commands", cmdName);
  const side = document.getElementById("cmd-side");
  const main = document.getElementById("cmd-main");
  if (side) side.textContent = "読み込み中…";
  if (main) main.textContent = "";
  try {
    const data = await saiApi(`/api/projects/${encodeURIComponent(cmdName)}/commands`);
    cmdClis = (data && data.clis) || [];
  } catch (e) {
    cmdClis = [];
    if (side) side.textContent = "";
    if (side) side.appendChild(cmdEl("p", "lib-empty", (e && e.message) || "読み込めませんでした"));
    return;
  }
  if (!cmdClis.length) {
    if (side) {
      side.textContent = "";
      side.appendChild(cmdEl("p", "lib-empty", "使える CLI がありません"));
    }
    return;
  }
  if (!cmdCliKey || !cmdClis.some((c) => c.key === cmdCliKey)) {
    cmdCliKey = cmdClis[0].key;
  }
  cmdCommand = null;
  cmdResult = "";
  cmdRenderSide();
  cmdRenderMain();
}

function cmdRenderSide() {
  const side = document.getElementById("cmd-side");
  if (!side) return;
  side.innerHTML = "";

  // CLI の選択
  const cliRow = cmdEl("div", "dash-seg small cmd-cli-seg");
  for (const cli of cmdClis) {
    const b = cmdEl("button", cli.key === cmdCliKey ? "active" : "", cli.key);
    b.type = "button";
    b.addEventListener("click", () => {
      cmdCliKey = cli.key;
      cmdCommand = null;
      cmdResult = "";
      cmdRenderSide();
      cmdRenderMain();
    });
    cliRow.appendChild(b);
  }
  side.appendChild(cliRow);

  // 絞り込み
  const filter = document.createElement("input");
  filter.type = "search";
  filter.className = "cmd-filter";
  filter.placeholder = "命令を絞り込む";
  filter.value = cmdFilterText;
  filter.addEventListener("input", () => {
    cmdFilterText = filter.value;
    cmdRenderList();
  });
  side.appendChild(filter);

  const list = cmdEl("div", "cmd-list");
  list.id = "cmd-list";
  side.appendChild(list);
  cmdRenderList();
}

function cmdRenderList() {
  const list = document.getElementById("cmd-list");
  if (!list) return;
  list.innerHTML = "";
  const q = cmdFilterText.trim().toLowerCase();
  const commands = cmdCommands().filter((c) => {
    if (!q) return true;
    const hay = `${(c.path || []).join(" ")} ${c.about || ""}`.toLowerCase();
    return hay.includes(q);
  });
  if (!commands.length) {
    list.appendChild(cmdEl("p", "lib-empty", "命令がありません"));
    return;
  }
  for (const group of cmdGroup(commands)) {
    list.appendChild(cmdEl("div", "cmd-group", group.key));
    for (const c of group.items) {
      const label = (c.path || []).slice(1).join(" ") || group.key;
      const b = cmdEl("button", "cmd-item" + (cmdCommand === c ? " active" : ""), label);
      b.type = "button";
      b.title = c.about || "";
      b.addEventListener("click", () => {
        cmdCommand = c;
        cmdResult = "";
        cmdRenderSide();
        cmdRenderMain();
      });
      list.appendChild(b);
    }
  }
}

function cmdRenderMain() {
  const main = document.getElementById("cmd-main");
  if (!main) return;
  main.innerHTML = "";
  if (!cmdCommand) {
    main.appendChild(cmdEl("p", "lib-empty", "左から命令を選んでください"));
    return;
  }
  main.appendChild(cmdEl("h2", "cmd-title", (cmdCommand.path || []).join(" ")));
  if (cmdCommand.about) main.appendChild(cmdEl("p", "cmd-about", cmdCommand.about));

  const fields = cmdEl("div", "cmd-fields");
  for (const arg of cmdCommand.args || []) {
    if (!arg.web) continue;
    fields.appendChild(cmdField(arg));
  }
  // 標準入力（json / text のときだけ）。bytes の命令は一覧に出さない。
  const stdinKind = cmdCommand.stdin || "none";
  if (stdinKind === "json" || stdinKind === "text") {
    const wrap = cmdEl("div", "cmd-field");
    const label = cmdEl("label", "cmd-label", `stdin（${stdinKind}）`);
    const area = document.createElement("textarea");
    area.className = "cmd-input";
    area.rows = 5;
    area.id = "cmd-stdin";
    label.htmlFor = "cmd-stdin";
    wrap.append(label, area);
    fields.appendChild(wrap);
  }
  main.appendChild(fields);

  const row = cmdEl("div", "cmd-run-row");
  const run = cmdEl("button", "sai-btn primary", "実行");
  run.type = "button";
  run.addEventListener("click", () => { void cmdRun(); });
  row.appendChild(run);
  main.appendChild(row);

  const result = cmdEl("pre", "cmd-result");
  if (cmdResult) result.textContent = cmdResult;
  main.appendChild(result);
}

function cmdField(arg) {
  const wrap = cmdEl("div", "cmd-field");
  const spec = cmdFieldType(arg);
  const id = `cmd-arg-${arg.name}`;
  const label = cmdEl("label", "cmd-label");
  label.htmlFor = id;
  label.textContent = `${arg.name}${arg.required ? " *" : ""}`;
  let input;
  if (spec.kind === "check") {
    input = document.createElement("input");
    input.type = "checkbox";
  } else if (spec.kind === "select") {
    input = document.createElement("select");
    if (!arg.required) {
      const opt = document.createElement("option");
      opt.value = "";
      opt.textContent = "（未指定）";
      input.appendChild(opt);
    }
    for (const value of spec.values) {
      const opt = document.createElement("option");
      opt.value = String(value);
      opt.textContent = String(value);
      input.appendChild(opt);
    }
  } else if (spec.kind === "number") {
    input = document.createElement("input");
    input.type = "number";
    input.step = arg.type === "int" ? "1" : "any";
  } else if (spec.kind === "lines") {
    input = document.createElement("textarea");
    input.rows = 3;
    input.placeholder = "1 行 1 値";
  } else {
    input = document.createElement("input");
    input.type = "text";
  }
  input.className = "cmd-input";
  input.id = id;
  input.dataset.cmdArg = arg.name;
  wrap.append(label, input);
  if (arg.help) wrap.appendChild(cmdEl("p", "cmd-help", arg.help));
  return wrap;
}

/** 入力欄から値を集める。 */
function cmdReadValues() {
  const values = {};
  const main = document.getElementById("cmd-main");
  if (!main) return values;
  for (const node of main.querySelectorAll("[data-cmd-arg]")) {
    const name = node.dataset.cmdArg;
    values[name] = node.type === "checkbox" ? node.checked : node.value;
  }
  return values;
}

async function cmdRun() {
  if (!cmdCommand) return;
  const stdinNode = document.getElementById("cmd-stdin");
  const stdinText = stdinNode ? stdinNode.value : "";
  let payload;
  try {
    payload = cmdBuildRequest(cmdCliKey, cmdCommand, cmdReadValues(), stdinText);
  } catch (e) {
    saiToast((e && e.message) || "入力を確かめてください", "error");
    return;
  }
  const result = document.querySelector("#cmd-main .cmd-result");
  if (result) result.textContent = "実行中…";
  try {
    const out = await saiApi(
      `/api/projects/${encodeURIComponent(cmdName)}/commands/run`,
      { method: "POST", body: payload }
    );
    cmdResult = out && out.json != null
      ? JSON.stringify(out.json, null, 2)
      : ((out && out.text) || "(出力なし)");
    if (result) result.textContent = cmdResult;
  } catch (e) {
    cmdResult = (e && e.message) || "実行できませんでした";
    if (result) result.textContent = cmdResult;
    saiToast(cmdResult, "error");
  }
}

// --- 画面の登録（ブラウザだけ） ---

if (typeof window !== "undefined") {
  (window.SaiRoutes = window.SaiRoutes || []).push({
    match(hash) {
      return cmdParseHash(hash);
    },
    view: "view-commands",
    tab: "project",
    hideTabbar: false,
    interval: 0,
    async open(params) {
      await cmdOpen(params);
    },
    async poll() {},
  });

  const back = document.getElementById("cmd-back");
  if (back) back.addEventListener("click", () => { location.hash = "#/projects"; });
}

// --- node からの読み込み（tests/test_commands.js） ---
if (typeof module !== "undefined" && module.exports) {
  module.exports = {
    cmdFieldType,
    cmdScalar,
    cmdBuildRequest,
    cmdGroup,
    cmdParseHash,
  };
}
