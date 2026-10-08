// sai-web のログイン・パスワード変更・アカウント・管理の画面（実装計画 U07）。
//
// 多人数モード（研究室の Linux サーバ）で使う 4 つの画面:
//   ログイン       #/login     大学のメールアドレスとパスワード（POST /api/auth/login）
//   パスワード変更 #/password  初回ログインのときと「パスワードを変える」から
//   アカウント     #/account   自分の情報と sai-prep のトークン・Usage（U05）
//   管理           #/admin     利用者の一覧と作成・変更（管理者だけ）
//
// 単独モード（Mac で 1 人が使うとき）は /api/auth/me が {"mode":"single"} を返すので，
// この画面は使わない（合言葉の prompt は app.js に今までどおり残る）。
//
// 画面の作り方の約束（AGENTS.md「画面のコード」）:
//   - 登録は window.SaiRoutes（app.js に画面ごとの分岐を足さない）
//   - ui.js の部品（saiApi・saiEl・saiToast・saiConfirm・saiSheet・saiEmpty）を使う
//   - 一番外側の名前には頭 ac / AC を付ける（tests/test_globals.js が重なりを見る）
//
// 区画（U11 が足す）:
//   #/account は acSections， #/admin は acAdminSections の関数を順に呼んで描く。
//   1 つ 1 つは (root: HTMLElement, me: {name,email,is_admin}) => void。
//   U11 は配列に関数を足すだけで「API キー」「プロジェクトとコンテナ」の区画を足せる。
"use strict";

// --- 検査（DOM を使わない。tests/test_account.js が読む） ---

/** 短い名前（管理者が決める。フォルダ名とコンテナ名に使う）: 英小文字で始まり 2〜16 文字。 */
function acValidName(v) {
  return typeof v === "string" && /^[a-z][a-z0-9]{1,15}$/.test(v);
}

/** 大学のメールアドレス（@ と . があり，空白を含まない）。 */
function acValidEmail(v) {
  return typeof v === "string" && /^[^@\s]+@[^@\s]+\.[^@\s]+$/.test(v);
}

/** パスワードは 8〜128 文字。 */
function acValidPassword(v) {
  return typeof v === "string" && v.length >= 8 && v.length <= 128;
}

// --- ログインの状態（window.SaiAuth） ---
//
// app.js の起動の処理から acLoadMe() を 1 回呼ぶ。401 を受けたときに
// 「多人数モードか」を判断するのに使う（app.js の apiFetch）。

const AC_LOGIN_FROM_KEY = "sai_web_login_from";

/**
 * GET /api/auth/me を 1 回読み，結果を window.SaiAuth に置く。
 * 単独モードは {"mode":"single"}，多人数モードは {mode:"multi", name, email, is_admin, …}。
 * 多人数モードで未ログインのときは 401 なので multi とする。
 * つながらない・返しが無い（古いサーバ）ときは単独モードとして扱う。
 */
async function acLoadMe() {
  let data = null;
  try {
    const res = await fetch("/api/auth/me", { headers: { Accept: "application/json" } });
    if (res.ok) {
      data = await res.json();
    } else if (res.status === 401) {
      data = { mode: "multi" };
    }
  } catch {
    // つながらないときは単独モードのまま（今までの動きを壊さない）
  }
  if (!data || typeof data.mode !== "string") data = { mode: "single" };
  window.SaiAuth = data;
  return data;
}

/** 401 のとき，今の住所を覚えてログイン画面へ移る（app.js の apiFetch から呼ぶ）。 */
function acRememberThenLogin() {
  try {
    const h = location.hash;
    if (h && h !== "#/login" && h !== "#/password") sessionStorage.setItem(AC_LOGIN_FROM_KEY, h);
  } catch { /* 覚えられなくてもよい */ }
  if (location.hash !== "#/login") location.hash = "#/login";
}

/** ログインの前に覚えた住所を取り出す（1 回で消す）。無ければ空文字。 */
function acTakeLoginFrom() {
  try {
    const v = sessionStorage.getItem(AC_LOGIN_FROM_KEY);
    sessionStorage.removeItem(AC_LOGIN_FROM_KEY);
    return v || "";
  } catch {
    return "";
  }
}

// --- 小さな部品（DOM を作る。ブラウザでのみ呼ばれる） ---

/** 見出しの帯（他の画面と同じ .appbar）。 */
function acAppbar(title) {
  const bar = saiEl("header", "appbar");
  bar.appendChild(saiEl("h1", null, title));
  return bar;
}

/** 入力欄 1 つ。type は "text" / "email" / "password" など。 */
function acInput(type, name, autocomplete) {
  const input = document.createElement("input");
  input.type = type;
  input.className = "ac-input";
  if (name) input.name = name;
  if (autocomplete) input.autocomplete = autocomplete;
  return input;
}

/** 「見出し + 入力欄」の 1 組（label の中に入れて押しやすくする）。 */
function acField(labelText, input) {
  const wrap = saiEl("label", "ac-field");
  wrap.appendChild(saiEl("span", "ac-label", labelText));
  wrap.appendChild(input);
  return wrap;
}

/** 日時の表示（数値は UNIX 秒。読めない値はそのまま出す）。 */
function acDate(v) {
  if (!v) return "";
  const d = new Date(typeof v === "number" ? v * 1000 : v);
  if (isNaN(d)) return String(v);
  return d.toLocaleString("ja-JP", {
    year: "numeric",
    month: "numeric",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  });
}

/** 値を大きく見せて 1 回だけコピーさせる板（トークンと再発行のパスワードで使う）。 */
function acShowSecret(value, title, warn) {
  const text = String(value || "");
  const body = saiEl("div", "ac-token-once");
  body.appendChild(saiEl("p", "ac-token-warn", warn));
  const code = saiEl("code", "ac-token-value", text);
  body.appendChild(code);
  const copy = saiEl("button", "sai-btn primary", "コピー");
  copy.type = "button";
  copy.addEventListener("click", async () => {
    try {
      await navigator.clipboard.writeText(text);
      saiToast("コピーしました", "ok");
    } catch {
      acSelectText(code);
      saiToast("コピーできませんでした。値を選んでコピーしてください。", "error");
    }
  });
  body.appendChild(copy);
  saiSheet({ title: title || "値", body });
}

/** 文字を範囲選択する（クリップボードが使えないときの逃げ道）。 */
function acSelectText(el) {
  try {
    const range = document.createRange();
    range.selectNodeContents(el);
    const sel = window.getSelection();
    sel.removeAllRanges();
    sel.addRange(range);
  } catch { /* 選べなくてもよい */ }
}

// --- ログイン画面（#/login） ---

function acLoginOpen() {
  const root = document.getElementById("login-root");
  if (!root) return;
  root.innerHTML = "";
  root.appendChild(acAppbar("ログイン"));
  const body = saiEl("div", "ac-body");
  const card = saiEl("div", "sai-card ac-card ac-narrow");
  const form = saiEl("form", "ac-form");
  const email = acInput("email", "email", "username");
  const password = acInput("password", "password", "current-password");
  form.appendChild(acField("大学のメールアドレス", email));
  form.appendChild(acField("パスワード", password));
  const err = saiEl("p", "ac-error");
  form.appendChild(err);
  const submit = saiEl("button", "sai-btn primary", "ログイン");
  submit.type = "submit";
  form.appendChild(submit);
  form.addEventListener("submit", async (e) => {
    e.preventDefault();
    err.textContent = "";
    const mail = email.value.trim();
    if (!acValidEmail(mail)) {
      err.textContent = "メールアドレスの形が正しくありません。";
      return;
    }
    if (!password.value) {
      err.textContent = "パスワードを入れてください。";
      return;
    }
    submit.disabled = true;
    try {
      const data = await saiApi("/api/auth/login", {
        method: "POST",
        body: { email: mail, password: password.value },
      });
      window.SaiAuth = { mode: "multi", ...(data || {}) };
      const from = acTakeLoginFrom();
      if (data && data.must_change_password) location.hash = "#/password";
      else location.hash = from || "#/home";
    } catch (e2) {
      err.textContent = e2 && e2.message ? e2.message : "ログインできませんでした";
    } finally {
      submit.disabled = false;
    }
  });
  card.appendChild(form);
  body.appendChild(card);
  root.appendChild(body);
  email.focus();
}

// --- パスワード変更画面（#/password） ---

function acPasswordOpen() {
  const root = document.getElementById("password-root");
  if (!root) return;
  root.innerHTML = "";
  root.appendChild(acAppbar("パスワードを変える"));
  const body = saiEl("div", "ac-body");
  const card = saiEl("div", "sai-card ac-card ac-narrow");
  const form = saiEl("form", "ac-form");
  const current = acInput("password", "current", "current-password");
  const next = acInput("password", "new", "new-password");
  const confirm = acInput("password", "confirm", "new-password");
  form.appendChild(acField("今のパスワード", current));
  form.appendChild(acField("新しいパスワード（8 文字以上）", next));
  form.appendChild(acField("新しいパスワード（確認）", confirm));
  const err = saiEl("p", "ac-error");
  form.appendChild(err);
  const submit = saiEl("button", "sai-btn primary", "変える");
  submit.type = "submit";
  form.appendChild(submit);
  form.addEventListener("submit", async (e) => {
    e.preventDefault();
    err.textContent = "";
    if (!current.value) {
      err.textContent = "今のパスワードを入れてください。";
      return;
    }
    if (!acValidPassword(next.value)) {
      err.textContent = "新しいパスワードは 8 文字以上 128 文字以下です。";
      return;
    }
    if (next.value !== confirm.value) {
      err.textContent = "確認のパスワードが一致しません。";
      return;
    }
    submit.disabled = true;
    try {
      await saiApi("/api/auth/password", {
        method: "POST",
        body: { current: current.value, new: next.value },
      });
      await acLoadMe();
      saiToast("パスワードを変えました", "ok");
      location.hash = "#/home";
    } catch (e2) {
      err.textContent = e2 && e2.message ? e2.message : "変えられませんでした";
    } finally {
      submit.disabled = false;
    }
  });
  card.appendChild(form);
  body.appendChild(card);
  root.appendChild(body);
  current.focus();
}

// --- アカウント画面（#/account） ---
//
// 区画は acSections の配列。U11 がここに関数を足す（「API キーと Claude のトークン」など）。

/** ①「アカウント」: 自分の情報・パスワード変更・ログアウト・管理への入口。 */
function acSectionAccount(root, me) {
  if (!acMultiUser()) return; // 単独モードにはアカウントが無い（U05）
  const card = saiEl("div", "sai-card ac-card");
  card.appendChild(saiEl("h2", "ac-title", "アカウント"));
  const rows = [
    ["メールアドレス", me.email || ""],
    ["短い名前", me.name || ""],
    ["役割", me.is_admin ? "管理者" : "一般"],
  ];
  for (const [label, value] of rows) {
    const row = saiEl("div", "ac-row");
    row.appendChild(saiEl("span", "ac-row-label", label));
    row.appendChild(saiEl("span", "ac-row-value", value));
    card.appendChild(row);
  }
  const actions = saiEl("div", "ac-actions");
  const pw = saiEl("button", "sai-btn", "パスワードを変える");
  pw.type = "button";
  pw.addEventListener("click", () => { location.hash = "#/password"; });
  actions.appendChild(pw);
  const out = saiEl("button", "sai-btn ghost", "ログアウト");
  out.type = "button";
  out.addEventListener("click", () => void acLogout());
  actions.appendChild(out);
  if (me.is_admin) {
    const admin = saiEl("button", "sai-btn", "管理");
    admin.type = "button";
    admin.addEventListener("click", () => { location.hash = "#/admin"; });
    actions.appendChild(admin);
  }
  card.appendChild(actions);
  root.appendChild(card);
}

/** ログアウト（POST /api/auth/logout → #/login）。失敗してもログイン画面へ移す。 */
async function acLogout() {
  try {
    await saiApi("/api/auth/logout", { method: "POST" });
  } catch { /* 失敗してもログイン画面へ */ }
  window.SaiAuth = { mode: "multi" };
  location.hash = "#/login";
}

/** ②「sai-prep のトークン」: 一覧・作成・削除。 */
function acSectionTokens(root) {
  if (!acMultiUser()) return; // トークンは多人数モードのログインに付く（U05）
  const card = saiEl("div", "sai-card ac-card");
  const head = saiEl("div", "ac-card-head");
  head.appendChild(saiEl("h2", "ac-title", "sai-prep のトークン"));
  const add = saiEl("button", "sai-btn small", "トークンを作る");
  add.type = "button";
  head.appendChild(add);
  card.appendChild(head);
  card.appendChild(
    saiEl(
      "p",
      "ac-note",
      "各自の Mac の sai-prep がこのサーバを呼ぶときに使います。値は作ったときだけ表示され、あとからは見られません。"
    )
  );
  const list = saiEl("div", "ac-token-list");
  card.appendChild(list);
  root.appendChild(card);
  add.addEventListener("click", () => acTokenCreate(list));
  void acTokenLoad(list);
}

/** トークンの一覧を読む（GET /api/me/tokens）。 */
async function acTokenLoad(list) {
  list.innerHTML = "";
  list.appendChild(saiEl("p", "ac-loading", "読み込み中…"));
  try {
    const data = await saiApi("/api/me/tokens");
    const tokens = (data && data.tokens) || [];
    list.innerHTML = "";
    if (tokens.length === 0) {
      list.appendChild(saiEl("p", "ac-empty", "トークンはまだありません。"));
      return;
    }
    for (const t of tokens) list.appendChild(acTokenRow(t, list));
  } catch (e) {
    list.innerHTML = "";
    saiToast(e && e.message ? e.message : "トークンを読み込めませんでした", "error");
  }
}

/** トークン 1 行（名前・作った日・最後に使った日・消す）。 */
function acTokenRow(t, list) {
  const row = saiEl("div", "ac-token-row");
  const info = saiEl("div", "ac-token-info");
  info.appendChild(saiEl("div", "ac-token-label", t.label || "(名前なし)"));
  const dates = [];
  if (t.created_at) dates.push(`作成 ${acDate(t.created_at)}`);
  dates.push(t.last_used_at ? `最終使用 ${acDate(t.last_used_at)}` : "未使用");
  info.appendChild(saiEl("div", "ac-token-date", dates.join(" · ")));
  row.appendChild(info);
  const del = saiEl("button", "sai-btn small ghost", "消す");
  del.type = "button";
  del.addEventListener("click", async () => {
    const ok = await saiConfirm({
      title: "トークンを消しますか？",
      body: `「${t.label || "(名前なし)"}」を消すと、sai-prep からこのサーバへつながらなくなります。`,
      ok: "消す",
      danger: true,
    });
    if (!ok) return;
    try {
      await saiApi(`/api/me/tokens/${encodeURIComponent(t.id)}`, { method: "DELETE" });
      saiToast("トークンを消しました", "ok");
      await acTokenLoad(list);
    } catch (e) {
      saiToast(e && e.message ? e.message : "消せませんでした", "error");
    }
  });
  row.appendChild(del);
  return row;
}

/** トークンを作る（saiSheet で名前を入れる → POST /api/me/tokens → 値を 1 回だけ見せる）。 */
function acTokenCreate(list) {
  const body = saiEl("div", "ac-form");
  const label = acInput("text", "label", "off");
  label.placeholder = "例: 研究室の Mac";
  body.appendChild(acField("名前（あとで見分けるための文字）", label));
  const err = saiEl("p", "ac-error");
  body.appendChild(err);
  const create = saiEl("button", "sai-btn primary", "作る");
  create.type = "button";
  const actions = saiEl("div", "ac-actions");
  actions.appendChild(create);
  body.appendChild(actions);
  const sheet = saiSheet({ title: "トークンを作る", body });
  create.addEventListener("click", async () => {
    err.textContent = "";
    const name = label.value.trim();
    if (!name) {
      err.textContent = "名前を入れてください。";
      return;
    }
    create.disabled = true;
    try {
      const data = await saiApi("/api/me/tokens", { method: "POST", body: { label: name } });
      sheet.close();
      acShowSecret(
        data.token,
        data.label ? `トークン「${data.label}」` : "トークン",
        "この値は二度と表示されません。いまコピーして sai-prep に貼り付けてください。"
      );
      await acTokenLoad(list);
    } catch (e) {
      err.textContent = e && e.message ? e.message : "作れませんでした";
    } finally {
      create.disabled = false;
    }
  });
  label.focus();
}

// --- アカウント: API キーと Claude のトークン（U11・共有契約 C9） ---

/** 多人数モードか（U07 の `window.SaiAuth.mode`）。単独モードではこの区画を出さない。 */
function acMultiUser() {
  return !!(
    typeof window !== "undefined" &&
    window.SaiAuth &&
    window.SaiAuth.mode === "multi"
  );
}

// 扱う名前と画面の見出し（順番はサーバの KEY_NAMES と同じ・共有契約 C9）。
const AC_KEY_LABELS = [
  ["CLAUDE_CODE_OAUTH_TOKEN", "Claude（`claude setup-token` のトークン）"],
  ["OPENROUTER_API_KEY", "OpenRouter"],
  ["SEMANTICSCHOLAR_API_KEY", "Semantic Scholar"],
  ["OPENALEX_API_KEY", "OpenAlex"],
  ["NCBI_API_KEY", "NCBI（PubMed）"],
];

/** 「API キーと Claude のトークン」の区画（多人数モードだけ）。 */
async function acSectionKeys(root) {
  if (!acMultiUser()) return;
  const card = saiEl("div", "sai-card ac-card");
  card.appendChild(saiEl("h2", "ac-title", "API キーと Claude のトークン"));
  card.appendChild(
    saiEl(
      "p",
      "ac-note",
      "値はこのサーバにだけ保存され、あなたのプロジェクトのコンテナにだけ渡ります。保存し直すと、実行中でないコンテナが新しい値で作り直されます。"
    )
  );
  const list = saiEl("div", "ac-key-list");
  card.appendChild(list);
  root.appendChild(card);
  await acKeysLoad(list);
}

/** 鍵の状態を読む（GET /api/me/keys）。 */
async function acKeysLoad(list) {
  list.innerHTML = "";
  list.appendChild(saiEl("p", "ac-loading", "読み込み中…"));
  let keys;
  try {
    const data = await saiApi("/api/me/keys");
    keys = (data && data.keys) || [];
  } catch (e) {
    list.innerHTML = "";
    saiToast(e && e.message ? e.message : "鍵を読み込めませんでした", "error");
    return;
  }
  const byName = new Map(keys.map((k) => [k && k.name, k]));
  list.innerHTML = "";
  for (const [name, label] of AC_KEY_LABELS) {
    const k = byName.get(name) || { name, set: false, last4: "" };
    list.appendChild(acKeyRow(name, label, k, () => acKeysLoad(list)));
    // Claude の行の下に説明と注意を出す
    if (name === "CLAUDE_CODE_OAUTH_TOKEN") list.appendChild(acClaudeNote());
  }
}

/** 鍵 1 行（見出し・設定済みか・入力欄・保存・消す）。 */
function acKeyRow(name, label, k, reload) {
  const row = saiEl("div", "ac-key-row");
  const head = saiEl("div", "ac-key-head");
  head.appendChild(saiEl("span", "ac-key-label", label));
  head.appendChild(
    saiEl(
      "span",
      "ac-key-status" + (k.set ? " on" : ""),
      k.set ? `設定済み（…${k.last4}）` : "未設定"
    )
  );
  row.appendChild(head);

  const input = acInput("password", name, "off");
  input.placeholder = k.set ? "新しい値に変える" : "値を貼る";
  const actions = saiEl("div", "ac-key-actions");
  const save = saiEl("button", "sai-btn small", "保存");
  save.type = "button";
  save.addEventListener("click", () => void acKeySave(name, input, reload));
  const del = saiEl("button", "sai-btn small ghost", "消す");
  del.type = "button";
  del.disabled = !k.set;
  del.addEventListener("click", () => void acKeyDelete(name, input, reload));
  actions.append(save, del);
  row.append(input, actions);
  return row;
}

/** 鍵を保存する（PUT /api/me/keys・変えた 1 つだけ送る）。 */
async function acKeyWrite(changes, input, reload) {
  try {
    const data = await saiApi("/api/me/keys", { method: "PUT", body: changes });
    input.value = "";
    saiToast("保存しました", "ok");
    acKeyReport(data);
    await reload();
  } catch (e) {
    saiToast(e && e.message ? e.message : "保存できませんでした", "error");
  }
}

/** 「保存」: 入力欄の値をその名前で保存する。 */
async function acKeySave(name, input, reload) {
  const value = input.value;
  if (!value) {
    saiToast("値を入れてください", "error");
    return;
  }
  await acKeyWrite({ [name]: value }, input, reload);
}

/** 「消す」: 空文字を送ってその名前を消す。 */
async function acKeyDelete(name, input, reload) {
  await acKeyWrite({ [name]: "" }, input, reload);
}

/** 保存の結果（作り直した / あとで反映される）をトーストで知らせる。 */
function acKeyReport(data) {
  const d = data || {};
  for (const project of d.recreated || []) {
    saiToast(`${project} を新しい値で起動し直しました`, "ok");
  }
  for (const project of d.pending || []) {
    saiToast(
      `${project} は実行中のため、終わってから起動し直すと反映されます`,
      "info"
    );
  }
}

/** Claude の行の下に出す説明と注意（Anthropic の規約について）。 */
function acClaudeNote() {
  const box = saiEl("div", "ac-claude-note");
  box.appendChild(
    saiEl(
      "p",
      "ac-note",
      "自分の Mac の端末で `claude setup-token` を実行し、研究室の Team プランに大学のメールアドレスでログインして発行したトークンを貼ってください。トークンはあなたのプロジェクトのコンテナにだけ渡り、他の人とは共有されません。"
    )
  );
  box.appendChild(
    saiEl(
      "p",
      "ac-note ac-warn",
      "注意: Anthropic の規約は、アプリが Claude の資格情報を保存・仲介することを禁じています。この機能はその点で灰色です。使うかどうかはあなたの判断で決めてください。トークンは他の人に渡さないでください。"
    )
  );
  return box;
}

/** #/account の区画（順に呼ぶ。U11 が足す。Usage は acAccountOpen の最後に足す）。 */
const acSections = [acSectionAccount, acSectionTokens, acSectionKeys];

function acAccountOpen() {
  const root = document.getElementById("account-root");
  if (!root) return;
  root.innerHTML = "";
  root.appendChild(acAppbar("Account"));
  const me = window.SaiAuth || {};
  const body = saiEl("div", "ac-body");
  for (const fn of acSections) {
    try {
      fn(body, me);
    } catch { /* 1 つの区画が失敗しても他は出す */ }
  }
  root.appendChild(body);
  // Usage の区画（U05）。usageOpen が #usage-list を探すので、body を root に付けてから描く
  if (typeof usageMountInto === "function") {
    try {
      usageMountInto(body);
    } catch { /* 使用量が出せなくても他の区画は出す */ }
  }
}

// --- 管理画面（#/admin） ---
//
// 区画は acAdminSections の配列（U11 が「プロジェクトとコンテナ」などを足す）。

/** 「利用者」: 一覧と作成、行ごとの変更。 */
function acSectionUsers(root, me) {
  const card = saiEl("div", "sai-card ac-card");
  const head = saiEl("div", "ac-card-head");
  head.appendChild(saiEl("h2", "ac-title", "利用者"));
  const add = saiEl("button", "sai-btn small", "利用者を作る");
  add.type = "button";
  head.appendChild(add);
  card.appendChild(head);
  const list = saiEl("div", "ac-user-list");
  card.appendChild(list);
  root.appendChild(card);
  add.addEventListener("click", () => acUserCreate(list, me));
  void acUsersLoad(list, me);
}

/** 利用者の一覧を読む（GET /api/admin/users）。読めないときは一覧を出さずトーストだけ出す。 */
async function acUsersLoad(list, me) {
  list.innerHTML = "";
  list.appendChild(saiEl("p", "ac-loading", "読み込み中…"));
  try {
    const data = await saiApi("/api/admin/users");
    const users = (data && data.users) || [];
    list.innerHTML = "";
    if (users.length === 0) {
      list.appendChild(saiEl("p", "ac-empty", "利用者がいません。"));
      return;
    }
    for (const u of users) list.appendChild(acUserRow(u, me, () => acUsersLoad(list, me)));
  } catch (e) {
    list.innerHTML = "";
    saiToast(e && e.message ? e.message : "利用者を読み込めませんでした", "error");
  }
}

/** 小さな操作ボタン 1 つ。 */
function acUserButton(label, onClick) {
  const b = saiEl("button", "sai-btn small", label);
  b.type = "button";
  b.addEventListener("click", onClick);
  return b;
}

/** 利用者 1 行（名前・メール・印・操作）。 */
function acUserRow(u, me, reload) {
  const row = saiEl("div", "ac-user-row");
  const info = saiEl("div", "ac-user-info");
  info.appendChild(saiEl("div", "ac-user-name", u.name || ""));
  info.appendChild(saiEl("div", "ac-user-email", u.email || ""));
  const chips = saiEl("div", "ac-user-chips");
  chips.appendChild(saiEl("span", "sai-chip" + (u.is_admin ? " ok" : " muted"), u.is_admin ? "管理者" : "一般"));
  if (u.disabled) chips.appendChild(saiEl("span", "sai-chip danger", "無効"));
  if (u.must_change_password) chips.appendChild(saiEl("span", "sai-chip warn", "初回変更待ち"));
  info.appendChild(chips);
  row.appendChild(info);
  const acts = saiEl("div", "ac-user-actions");
  acts.appendChild(acUserButton("パスワードを再発行", () => void acUserReissue(u, reload)));
  acts.appendChild(acUserButton("メールを変える", () => void acUserEmail(u, reload)));
  acts.appendChild(
    acUserButton(u.is_admin ? "管理者を外す" : "管理者にする", () =>
      void acUserPatch(u, { is_admin: !u.is_admin }, reload)
    )
  );
  acts.appendChild(acUserButton(u.disabled ? "戻す" : "無効にする", () => void acUserDisable(u, reload)));
  row.appendChild(acts);
  return row;
}

/** PATCH /api/admin/users/{name} を 1 回送って一覧を読み直す。 */
async function acUserPatch(u, patch, reload) {
  try {
    await saiApi(`/api/admin/users/${encodeURIComponent(u.name)}`, { method: "PATCH", body: patch });
    saiToast("変えました", "ok");
    await reload();
  } catch (e) {
    saiToast(e && e.message ? e.message : "変えられませんでした", "error");
  }
}

/** パスワードを再発行する（新しい値を 1 回だけ見せる）。 */
async function acUserReissue(u, reload) {
  const ok = await saiConfirm({
    title: "パスワードを再発行しますか？",
    body: `${u.name} さんのパスワードを新しくします。新しい値はこのあと 1 回だけ表示されます。`,
    ok: "再発行",
  });
  if (!ok) return;
  const password = acRandomPassword();
  try {
    await saiApi(`/api/admin/users/${encodeURIComponent(u.name)}`, {
      method: "PATCH",
      body: { password },
    });
    acShowSecret(
      password,
      `${u.name} の新しいパスワード`,
      "このパスワードは二度と表示されません。本人に伝えてください。"
    );
    await reload();
  } catch (e) {
    saiToast(e && e.message ? e.message : "再発行できませんでした", "error");
  }
}

/** メールアドレスを変える。 */
async function acUserEmail(u, reload) {
  const next = await saiPrompt({
    title: "メールを変える",
    body: "大学のメールアドレス",
    value: u.email || "",
    placeholder: "name@university.ac.jp",
    ok: "変える",
  });
  if (next == null) return;
  const email = next.trim();
  if (!acValidEmail(email)) {
    saiToast("メールアドレスの形が正しくありません。", "error");
    return;
  }
  await acUserPatch(u, { email }, reload);
}

/** 無効にする / 戻す。無効にする前には確認する。 */
async function acUserDisable(u, reload) {
  if (!u.disabled) {
    const ok = await saiConfirm({
      title: "利用者を無効にしますか？",
      body: `${u.name} さんはログインできなくなります。あとから戻せます。`,
      ok: "無効にする",
      danger: true,
    });
    if (!ok) return;
  }
  await acUserPatch(u, { disabled: !u.disabled }, reload);
}

/** 利用者を作る（saiSheet で名前・メール・初期パスワード・管理者の印を入れる）。 */
function acUserCreate(list, me) {
  const body = saiEl("div", "ac-form");
  const name = acInput("text", "name", "off");
  const email = acInput("email", "email", "off");
  const password = acInput("password", "password", "new-password");
  body.appendChild(acField("短い名前（フォルダ名とコンテナ名に使う）", name));
  body.appendChild(acField("大学のメールアドレス", email));
  body.appendChild(acField("初期パスワード（8 文字以上）", password));
  const check = saiEl("label", "ac-check");
  const admin = document.createElement("input");
  admin.type = "checkbox";
  check.appendChild(admin);
  check.appendChild(document.createTextNode("管理者にする"));
  body.appendChild(check);
  const err = saiEl("p", "ac-error");
  body.appendChild(err);
  const create = saiEl("button", "sai-btn primary", "作る");
  create.type = "button";
  const actions = saiEl("div", "ac-actions");
  actions.appendChild(create);
  body.appendChild(actions);
  const sheet = saiSheet({ title: "利用者を作る", body });
  create.addEventListener("click", async () => {
    err.textContent = "";
    const n = name.value.trim();
    const mail = email.value.trim();
    if (!acValidName(n)) {
      err.textContent = "短い名前は英小文字で始まり、英小文字と数字で 2〜16 文字です。";
      return;
    }
    if (!acValidEmail(mail)) {
      err.textContent = "メールアドレスの形が正しくありません。";
      return;
    }
    if (!acValidPassword(password.value)) {
      err.textContent = "パスワードは 8 文字以上 128 文字以下です。";
      return;
    }
    create.disabled = true;
    try {
      await saiApi("/api/admin/users", {
        method: "POST",
        body: { name: n, email: mail, password: password.value, is_admin: admin.checked },
      });
      sheet.close();
      saiToast("利用者を作りました", "ok");
      await acUsersLoad(list, me);
    } catch (e) {
      err.textContent = e && e.message ? e.message : "作れませんでした";
    } finally {
      create.disabled = false;
    }
  });
  name.focus();
}

/** 再発行に使う仮のパスワードを作る（紛らわしい文字を外した英数字 14 文字）。 */
function acRandomPassword() {
  const chars = "abcdefghijkmnpqrstuvwxyzABCDEFGHJKLMNPQRSTUVWXYZ23456789";
  const buf = new Uint32Array(14);
  if (typeof crypto !== "undefined" && crypto.getRandomValues) crypto.getRandomValues(buf);
  let out = "";
  for (let i = 0; i < 14; i += 1) {
    const r = buf[i] || Math.floor(Math.random() * 1e9);
    out += chars[r % chars.length];
  }
  return out;
}

// --- 管理: プロジェクトとコンテナ（U11・共有契約 C9） ---

/** 「プロジェクトとコンテナ」の区画（多人数モードだけ・管理者だけが見る画面）。 */
async function acSectionContainers(root) {
  if (!acMultiUser()) return;
  const card = saiEl("div", "sai-card ac-card");
  card.appendChild(saiEl("h2", "ac-title", "プロジェクトとコンテナ"));
  card.appendChild(
    saiEl(
      "p",
      "ac-note",
      "全員のプロジェクトのコンテナです。起動と停止だけができ、中身（会話・論文）は見られません。"
    )
  );
  const list = saiEl("div", "ac-container-list");
  card.appendChild(list);
  root.appendChild(card);
  await acContainersLoad(list);
}

/** 全員のコンテナの一覧を読む（GET /api/admin/containers）。 */
async function acContainersLoad(list) {
  list.innerHTML = "";
  list.appendChild(saiEl("p", "ac-loading", "読み込み中…"));
  let rows;
  try {
    const data = await saiApi("/api/admin/containers");
    rows = (data && data.containers) || [];
  } catch (e) {
    list.innerHTML = "";
    saiToast(e && e.message ? e.message : "コンテナを読み込めませんでした", "error");
    return;
  }
  list.innerHTML = "";
  if (rows.length === 0) {
    list.appendChild(saiEl("p", "ac-empty", "コンテナはありません。"));
    return;
  }
  const wrap = saiEl("div", "ac-table-wrap");
  const table = document.createElement("table");
  table.className = "ac-table";
  const thead = document.createElement("thead");
  const hr = document.createElement("tr");
  for (const label of ["持ち主", "プロジェクト", "状態", "Docker の状態", ""]) {
    hr.appendChild(saiEl("th", null, label));
  }
  thead.appendChild(hr);
  table.appendChild(thead);
  const tbody = document.createElement("tbody");
  for (const c of rows) tbody.appendChild(acContainerRow(c, () => acContainersLoad(list)));
  table.appendChild(tbody);
  wrap.appendChild(table);
  list.appendChild(wrap);
}

/** コンテナ 1 行（持ち主・プロジェクト・状態・Docker の状態・起動 / 停止）。 */
function acContainerRow(c, reload) {
  const tr = document.createElement("tr");
  tr.appendChild(saiEl("td", null, c.owner || ""));
  tr.appendChild(saiEl("td", null, c.project || ""));
  tr.appendChild(saiEl("td", null, acContainerStateLabel(c.state)));
  tr.appendChild(saiEl("td", null, c.status || ""));
  const td = document.createElement("td");
  const running = c.state === "running";
  const btn = saiEl("button", "sai-btn small", running ? "停止" : "起動");
  btn.type = "button";
  btn.addEventListener("click", () => void acContainerToggle(c, running, reload));
  td.appendChild(btn);
  tr.appendChild(td);
  return tr;
}

/** Docker の状態の文を日本語にする。 */
function acContainerStateLabel(state) {
  return (
    { running: "動いている", exited: "止まっている", missing: "無い" }[state] ||
    String(state || "")
  );
}

/** コンテナを起動・停止する（停止の前に確認する）。 */
async function acContainerToggle(c, running, reload) {
  if (running) {
    const ok = await saiConfirm({
      title: `${c.container} を止めますか？`,
      body: "止めると、動いているエージェントも止まります。",
      ok: "停止",
      danger: true,
    });
    if (!ok) return;
  }
  try {
    await saiApi(
      `/api/admin/containers/${encodeURIComponent(c.container)}/${running ? "stop" : "start"}`,
      { method: "POST" }
    );
    saiToast(running ? "コンテナを止めました" : "コンテナを起動しました", "ok");
  } catch (e) {
    saiToast(e && e.message ? e.message : "操作できませんでした", "error");
  }
  await reload();
}

/** #/admin の区画（順に呼ぶ。U11 が足す）。 */
const acAdminSections = [acSectionUsers, acSectionContainers];

function acAdminOpen() {
  const root = document.getElementById("admin-root");
  if (!root) return;
  root.innerHTML = "";
  root.appendChild(acAppbar("管理"));
  const me = window.SaiAuth || {};
  const body = saiEl("div", "ac-body");
  for (const fn of acAdminSections) {
    try {
      fn(body, me);
    } catch { /* 1 つの区画が失敗しても他は出す */ }
  }
  root.appendChild(body);
}

// --- 画面の登録（ブラウザでのみ） ---
//
// #/login と #/password はタブバーを隠す（hideTabbar）。tab は "auth" にして，
// タブの「最後に開いた住所」を本当のタブに覚えさせない（ログイン後に戻らないように）。
// interval は 0 にして，開いている間ほかの画面のポーリングを動かさない。

if (typeof window !== "undefined") {
  (window.SaiRoutes = window.SaiRoutes || []).push(
    {
      match(hash) {
        return hash === "#/login" ? {} : null;
      },
      view: "view-login",
      tab: "auth",
      hideTabbar: true,
      interval: 0,
      open() {
        acLoginOpen();
      },
    },
    {
      match(hash) {
        return hash === "#/password" ? {} : null;
      },
      view: "view-password",
      tab: "auth",
      hideTabbar: true,
      interval: 0,
      open() {
        acPasswordOpen();
      },
    },
    {
      match(hash) {
        return hash === "#/account" ? {} : null;
      },
      view: "view-account",
      tab: "account",
      hideTabbar: false,
      interval: 60000, // Usage を 60 秒ごとに読み直す（usage.js の一覧と同じ間隔・42 §3.5）
      open() {
        acAccountOpen();
      },
      poll() {
        if (typeof usageLoad === "function") return usageLoad();
      },
    },
    {
      match(hash) {
        return hash === "#/admin" ? {} : null;
      },
      view: "view-admin",
      tab: "account",
      hideTabbar: false,
      interval: 0,
      open() {
        acAdminOpen();
      },
    }
  );

  // U11 が区画を足せるように window からも触れるようにする
  window.acSections = acSections;
  window.acAdminSections = acAdminSections;
}

// --- node からの読み込み（tests/test_account.js） ---
// DOM を使わない検査だけを出す。
if (typeof module !== "undefined" && module.exports) {
  module.exports = {
    acValidName,
    acValidEmail,
    acValidPassword,
  };
}
