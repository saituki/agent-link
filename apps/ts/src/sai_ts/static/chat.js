// チャット（U05）。sidebar の「チャット」タブ。会話はサーバが sai-link の会話記録から分けて返す（C11）。
// 返事は 2 秒ごとの読み直しで待つ。本文は文字のまま出す（HTML として解釈しない）。

const tsChatPollMs = 2000;
const tsChatWaitMs = 15 * 60 * 1000;
const tsChatMaxChars = 20000;

// 文字とコードの囲み（```）に分ける。閉じていない囲みは終わりまでをコードにする（GFM と同じ）
function tsSplitCode(text) {
  const out = [];
  let plain = [];
  let code = null;
  const flushPlain = () => {
    const s = plain.join("\n");
    if (s.trim() !== "") out.push({ kind: "text", lang: "", text: s });
    plain = [];
  };
  for (const line of String(text).split("\n")) {
    if (code === null) {
      if (line.trimStart().startsWith("```")) {
        flushPlain();
        code = { lang: line.trimStart().slice(3).trim(), lines: [] };
      } else {
        plain.push(line);
      }
    } else if (line.trim() === "```") {
      out.push({ kind: "code", lang: code.lang, text: code.lines.join("\n") });
      code = null;
    } else {
      code.lines.push(line);
    }
  }
  if (code !== null) out.push({ kind: "code", lang: code.lang, text: code.lines.join("\n") });
  else flushPlain();
  return out;
}

// 画面の選択とイベントの選択の 1 行要約。送る本文には足さない
function tsSelectionSummary(state) {
  const parts = [];
  const sel = state.selection;
  if (sel) {
    const vars = sel.vars && sel.vars.length ? ` ${sel.vars.join(", ")}` : "";
    parts.push(`選択: ${state.record} ${Number(sel.t0).toFixed(1)}–${Number(sel.t1).toFixed(1)} 秒${vars}`);
  } else {
    parts.push("選択: なし");
  }
  const n = (state.selectedEvents || []).length;
  if (n > 0) parts.push(`イベント ${n} 件`);
  return parts.join(" / ");
}

// 前の会話に無い分（増えた分）。順番は保つ
function tsChatDiff(oldMsgs, newMsgs) {
  return newMsgs.slice(oldMsgs.length);
}

const tsChat = {
  ds: null,
  started: null, // null（まだ不明）| true | false（まだ始まっていない）
  status: "",
  msgs: [],
  pending: [], // 送ったばかりで、まだサーバに出ていない文
  baseline: 0, // 送った時点の assistant の数
  waitSince: 0, // 考え中の開始時刻（0 は待っていない）
  inFlight: false,
  logSig: "",
  els: null,
  timer: null,
};

function tsChatStyle() {
  if (document.getElementById("ts-style-chat.js")) return;
  const st = document.createElement("style");
  st.id = "ts-style-chat.js";
  st.textContent = `
.ts-chat { display: flex; flex-direction: column; gap: 6px; height: 100%; }
.ts-chat-log { flex: 1; overflow: auto; display: flex; flex-direction: column; gap: 6px; max-height: 60vh; }
.ts-msg { max-width: 85%; padding: 6px 8px; border-radius: 8px; background: #eee; }
.ts-msg.ts-user { align-self: flex-end; background: #dbe9ff; }
.ts-msg.ts-assistant { align-self: flex-start; }
.ts-msg.ts-pending { opacity: 0.6; }
.ts-msg .ts-t { white-space: pre-wrap; word-break: break-word; }
.ts-msg pre.ts-code { margin: 4px 0; padding: 6px; background: #f4f4f4; border: 1px solid #ccc; overflow: auto; font-family: ui-monospace, Menlo, monospace; font-size: 12px; white-space: pre; }
.ts-chat-note { color: #555; font-size: 13px; }
.ts-chat-form textarea { width: 100%; min-height: 56px; box-sizing: border-box; }
.ts-chat-form .ts-sum { font-size: 12px; color: #555; margin-bottom: 2px; }
`;
  document.head.appendChild(st);
}

function tsChatEnc() {
  return encodeURIComponent(TS.state.dataset || "");
}

function tsChatAssistants(msgs) {
  return msgs.filter((m) => m.role === "assistant").length;
}

// 会話を読み直す。まだ始まっていない（409・404）は started = false
async function tsChatLoad() {
  if (tsChat.ds !== TS.state.dataset) {
    tsChat.ds = TS.state.dataset;
    tsChat.started = null;
    tsChat.msgs = [];
    tsChat.pending = [];
    tsChat.waitSince = 0;
    tsChat.logSig = "";
  }
  try {
    const res = await TS.api(`/api/datasets/${tsChatEnc()}/chat/messages`);
    const before = tsChat.msgs;
    const added = tsChatDiff(before, res.messages || []);
    // 増えたら pending は出ている（サーバが受け取った）ので消す
    if (added.length > 0) tsChat.pending = [];
    tsChat.msgs = res.messages || [];
    tsChat.status = res.status || "";
    tsChat.started = true;
    if (tsChat.waitSince && tsChatAssistants(tsChat.msgs) > tsChat.baseline) tsChat.waitSince = 0;
  } catch (e) {
    if (e.status === 409 || e.status === 404) {
      tsChat.started = false;
      tsChat.msgs = [];
      tsChat.status = "";
    } else if (tsChat.els) {
      tsChat.els.note.textContent = `読めません: ${e.message}`;
    }
  }
  if (tsChat.waitSince && Date.now() - tsChat.waitSince > tsChatWaitMs) {
    tsChat.waitSince = 0;
    if (tsChat.els) tsChat.els.note.textContent = "返事がまだありません";
  }
}

// 本文を 1 つの吹き出しにする。文字は textContent に入れる
function tsChatBubble(role, text, pending) {
  const div = document.createElement("div");
  div.className = `ts-msg ts-${role}`;
  if (pending) div.classList.add("ts-pending");
  for (const part of tsSplitCode(text)) {
    if (part.kind === "code") {
      const pre = document.createElement("pre");
      pre.className = "ts-code";
      pre.textContent = part.text;
      div.appendChild(pre);
    } else {
      const t = document.createElement("div");
      t.className = "ts-t";
      t.textContent = part.text;
      div.appendChild(t);
    }
  }
  return div;
}

function tsChatDraw() {
  const e = tsChat.els;
  if (!e || !e.root.isConnected) return;
  e.sum.textContent = tsSelectionSummary(TS.state);

  const sig = JSON.stringify([tsChat.started, tsChat.status, tsChat.msgs, tsChat.pending]);
  if (sig !== tsChat.logSig) {
    tsChat.logSig = sig;
    e.log.replaceChildren();
    for (const m of tsChat.msgs) e.log.appendChild(tsChatBubble(m.role, m.text, false));
    for (const t of tsChat.pending) e.log.appendChild(tsChatBubble("user", t, true));
    e.log.scrollTop = e.log.scrollHeight;
  }

  const finished = tsChat.started === true && tsChat.status !== "running";
  e.start.hidden = !(tsChat.started === false || finished);
  e.start.textContent = tsChat.started === false ? "チャットを始める" : "チャットは終わっています。始め直す";

  if (tsChat.waitSince) e.note.textContent = "考え中…";
  else if (tsChat.started === false) e.note.textContent = "まだ始まっていません";
  else if (e.note.textContent === "考え中…" || e.note.textContent === "まだ始まっていません") e.note.textContent = "";
}

async function tsChatStart() {
  try {
    await TS.api(`/api/datasets/${tsChatEnc()}/chat/start`, { method: "POST" });
  } catch (e) {
    TS.ui.toast(`始められません: ${e.message}`);
    return;
  }
  await tsChatLoad();
  tsChatDraw();
}

async function tsChatSend() {
  const ta = tsChat.els.ta;
  const text = ta.value;
  if (text.trim() === "") return;
  if (text.length > tsChatMaxChars) {
    TS.ui.toast(`${tsChatMaxChars} 字までです`);
    return;
  }
  ta.value = "";
  try {
    await TS.api(`/api/datasets/${tsChatEnc()}/chat/send`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ text }),
    });
  } catch (e) {
    // 送れなかったら文字を戻す（入力中の文字を失わない）
    if (ta.value === "") ta.value = text;
    TS.ui.toast(`送れません: ${e.message}`);
    return;
  }
  tsChat.pending.push(text);
  tsChat.baseline = tsChatAssistants(tsChat.msgs);
  tsChat.waitSince = Date.now();
  tsChat.els.note.textContent = "";
  tsChatDraw();
}

function tsChatRender(el) {
  tsChatStyle();
  el.replaceChildren();
  const root = document.createElement("div");
  root.className = "ts-chat";

  const log = document.createElement("div");
  log.className = "ts-chat-log";
  const note = document.createElement("div");
  note.className = "ts-chat-note";
  const start = document.createElement("button");
  start.type = "button";
  start.hidden = true;
  start.addEventListener("click", tsChatStart);

  const form = document.createElement("div");
  form.className = "ts-chat-form";
  const sum = document.createElement("div");
  sum.className = "ts-sum";
  const ta = document.createElement("textarea");
  ta.placeholder = "agent への文（Ctrl+Enter か ⌘+Enter で送る）";
  ta.addEventListener("keydown", (e) => {
    if (e.key === "Enter" && (e.ctrlKey || e.metaKey)) {
      e.preventDefault();
      tsChatSend();
    }
  });
  const send = document.createElement("button");
  send.type = "button";
  send.textContent = "送る";
  send.addEventListener("click", tsChatSend);
  form.append(sum, ta, send);

  root.append(start, log, note, form);
  el.appendChild(root);
  tsChat.els = { root, log, note, start, sum, ta };
  tsChat.logSig = "";
  tsChatDraw();

  tsChatPollNow();
  clearInterval(tsChat.timer);
  tsChat.timer = setInterval(() => {
    if (!root.isConnected) {
      clearInterval(tsChat.timer);
      return;
    }
    tsChatPollNow();
  }, tsChatPollMs);
}

async function tsChatPollNow() {
  if (tsChat.inFlight) return;
  tsChat.inFlight = true;
  try {
    await tsChatLoad();
  } finally {
    tsChat.inFlight = false;
  }
  tsChatDraw();
}

function tsChatBoot() {
  TS.ui.addSidebarTab({ id: "chat", title: "チャット", render: tsChatRender });
  TS.on("state", () => tsChatDraw());
  TS.on("select", () => tsChatDraw());
}

if (typeof window !== "undefined" && window.TS) {
  window.TS.on("boot", tsChatBoot);
}

if (typeof module !== "undefined") {
  module.exports = { tsSplitCode, tsSelectionSummary, tsChatDiff, tsChatMaxChars };
}
