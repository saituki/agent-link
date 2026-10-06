// sai-web の Papers タブ — Radio（実装計画 09・プロジェクト対応は 18）の本体。
//
// 画面は2つ:
//   Radio 一覧   #/papers/radio（続きを聴く・一覧）
//   再生画面     #/radio/<radio_id>（再生・文字起こし・オフライン保存・質問チャット〔文字・声〕）
// 中身はすべて /api/radio/… 経由（sai-web が sai-radio へ中継する・09 §3）。
// Radio 一覧は上部で選んだプロジェクトの音声だけを出す（音声の名前
// `<root のフォルダ名>-<論文番号>` のフォルダ名で見分ける。どのプロジェクトか
// 分からない古い音声は出さない）。「続きを聴く」も同じ。聴いた記録は
// Papers → Dashboard の出来事に出る（sai-paper `phone dashboard`）。
// 再生は index.html の1つの <audio id="radio-audio"> だけを使う
// （画面を移っても止めないため・09 §4.3）。
// 通信は app.js の apiFetch（合言葉を自動で付ける），要素の作りは stats.js の
// el() を使う（00 §9）。DOM を使わない小さな関数は末尾で node に渡す
// （tests/test_radio.js）。移植元は sai-radio の static/app.js・sw.js（読むだけ）。
"use strict";

// --- 定数 ---

// 続きの位置の保存場所（09 §4.3）
const RADIO_LAST_KEY = "sai_web_radio_last";
// オフライン保存の倉庫（合言葉は含めない・09 §4.5）
const RADIO_AUDIO_CACHE = "sai-web-radio-audio";
// 再生速度（押すたびに 1 → 1.25 → 1.5 → 2 → 1・09 §4.5）
const RADIO_SPEEDS = [1, 1.25, 1.5, 2];
// 再生エラーから自動復帰する再試行回数（移植元と同じ3回・09 §4.3）
const RADIO_MAX_RETRY = 3;
// 論文情報の戻る先「最後に見ていた一覧」の鍵（papers.js の PAPERS_LIST_KEY と同じ物）
const RADIO_PAPERS_LIST_KEY = "sai_web_papers_list";
// 再生画面の「文字起こし | 質問」の切り替えの保存場所（10 §4.1）
const RADIO_PANE_KEY = "sai_web_radio_pane";
// 質問を10秒ごとに読み直す間隔（音声化の作成中表示・10 §4.5）
const RADIO_SLOT_POLL_MS = 10000;

// --- DOM を使わない小さな関数（tests/test_radio.js・09 §4.11） ---

/** 秒を見やすい形にする（1 時間未満は "12:34"，1 時間以上は "1:02:03"）。 */
function formatSec(sec) {
  const s = Math.max(0, Math.floor(Number(sec) || 0));
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const r = s % 60;
  if (h > 0) return `${h}:${String(m).padStart(2, "0")}:${String(r).padStart(2, "0")}`;
  return `${m}:${String(r).padStart(2, "0")}`;
}

/**
 * その位置（秒）の章を返す（質問の下の「03:12 手法 について」に使う・10 §4.6）。
 * 境界は `start <= sec < end`。`end` が無い（null / undefined）の章は最後まで。
 * どの章にも当たらない（範囲外・壊れたデータ）ときは null。
 */
function sectionAt(sections, sec) {
  if (!Array.isArray(sections) || sections.length === 0) return null;
  if (sec == null || sec === "") return null; // 位置が無いときは無理に引かない
  const t = Number(sec);
  if (!Number.isFinite(t)) return null;
  for (const s of sections) {
    if (!s) continue;
    const start = Number(s.start);
    if (!Number.isFinite(start)) continue;
    const rawEnd = s.end;
    const end = rawEnd == null || rawEnd === "" ? Infinity : Number(rawEnd);
    if (Number.isFinite(end) && end <= start) continue; // 壊れた境目は無視する
    if (t >= start && t < end) return s;
  }
  return null;
}

/**
 * sai-radio の論文 ID が住所に入れてよいか（英数字と _ - . だけ・10 §4.3・18 §3.3）。
 * sai-radio の ID は `4ee6474932af` のような16進の英数字と，`sai-paper-30` のような
 * 音声の名前（フォルダ名に `.` が含まれることがある・18 §3.3）の両方がある。
 * 英数字を1つ以上含まないもの（`..` や `.`）と Path の混入（`../x`・`a/b`）は通さない。
 */
function isRadioPaperId(id) {
  if (id == null) return false;
  return /^(?=.*[A-Za-z0-9])[A-Za-z0-9_.-]+$/.test(String(id));
}

/**
 * 音声の名前を「フォルダ名」と「論文番号」に分ける（18 §3.3）。
 * 形は `<root のフォルダ名>-<論文番号>`（例 `sai-paper-30`・`my-proj-2-5`）。
 * フォルダ名に `-` が入っていても，最後の `-<数字>` が論文番号なので一意に分かれる。
 * 分けられないもの（番号だけ・16進の ID・空）は null。DOM は使わない（tests/test_radio.js）。
 */
function splitRadioName(radioId) {
  const m = /^(.+)-(\d+)$/.exec(radioId == null ? "" : String(radioId));
  if (!m) return null;
  return { root_name: m[1], paper_id: m[2] };
}

/**
 * 音声がどのプロジェクトの論文かを引く（18 §1.2）。
 * 名前のフォルダ名が一致するプロジェクト（一覧の `root_name`）を返す。
 * 分けられない・一致しない音声は null（名札なし・「論文情報」ボタンなし・18 §1.2）。
 * DOM は使わない（tests/test_radio.js）。
 */
function radioProjectOf(projects, radioId) {
  const parts = splitRadioName(radioId);
  if (!parts || !Array.isArray(projects)) return null;
  return projects.find((p) => p && p.root_name === parts.root_name) || null;
}

/**
 * 音声がそのプロジェクト（root のフォルダ名）の論文のものか。
 * 分けられない古い音声（16進の ID・番号だけ）はどのプロジェクトにも入れない。
 * DOM は使わない（tests/test_radio.js）。
 */
function radioInProject(radioId, rootName) {
  const parts = splitRadioName(radioId);
  return !!(parts && rootName && parts.root_name === rootName);
}

/** 今 Papers の上部で選んでいるプロジェクトの root のフォルダ名（無ければ null）。 */
function selectedRootName() {
  const proj = paperProjectByName(selectedPaperProject());
  return (proj && proj.root_name) || null;
}

// --- 音声の住所（09 §4.3） ---

/** 倉庫（キャッシュ）に使う住所。合言葉は付けない（トークンを変えても同じ保存に当てるため）。 */
function audioCacheKey(id) {
  return `/api/radio/papers/${encodeURIComponent(id)}/audio`;
}

/** 再生用の住所。<audio> は Authorization ヘッダを付けられないので ?token= で渡す。 */
function audioUrl(id) {
  const base = audioCacheKey(id);
  // apiToken は app.js の変数（関数の中でしか呼ばないので読み込み順に関係しない）
  if (typeof apiToken !== "undefined" && apiToken) {
    return `${base}?token=${encodeURIComponent(apiToken)}`;
  }
  return base;
}

// --- 画面の状態（一覧・09 §4.4） ---

let radioList = null;      // GET /api/radio/papers の応答（null＝まだ読んでいない）
let radioError = "";       // 最後に一覧を読めなかったときの detail
let radioFilter = "";      // 題名の絞り込み（画面の中だけ）
let radioSort = "new";     // "new"（新着順） | "fav"（お気に入り優先）
const downloadedIds = new Set(); // ダウンロード済みの論文 id（表示の印用）

// --- 再生の状態（1 つの audio を画面をまたいで使い回す・09 §4.3） ---

let currentPaper = null;    // {id, title}（null なら再生バーは出さない）
let playerPaperId = null;   // 再生画面が開いている id（「論文情報 ›」用）
let currentSections = [];   // 文字起こし（sections）
let radioRetry = 0;         // 再生エラーの再試行回数
let radioWantPlay = false;  // エラーから戻すときに再生を再開するか
let listenedIds = new Set(); // listen を送った論文（最初の再生で1回だけ・09 §4.3）
let lastSavedAt = 0;        // 位置の保存の間引き（移植元と同じ・09 §4.3）
let lastSavedPos = -1;
let speedIndex = 0;         // 今の再生速度（RADIO_SPEEDS の番号）
let seekDragging = false;   // つまみを動かしている間は時刻だけ変える（09 §4.5）
let pendingStart = null;    // 「続きから再生」で移動したときの開始位置（09 §4.5）

// --- 質問チャットの状態（計画 10 §4.2） ---

let radioChat = [];           // 質問と回答の組 [{t, position_sec, question, answer, error}]
let radioChatError = "";      // 過去の質問を読めなかったときの detail
let radioChatLoadedFor = null; // 読み込んだ論文 id（同じ論文の開き直しでは読み直さない）
let radioAsking = false;      // 文字の質問を送っている間 true（二重送信防止・10 §4.3）
let chatSeq = 0;              // 吹き出しの識別子（openIds 用の連番・10 §4.2）

/** 画面の <audio> を取り出す（常にこの1つだけを使う・09 §4.3）。 */
function radioAudio() {
  return document.getElementById("radio-audio");
}

// --- 続きの位置（localStorage・09 §4.3） ---

function loadLastPosition() {
  try {
    const raw = localStorage.getItem(RADIO_LAST_KEY);
    const obj = raw ? JSON.parse(raw) : null;
    return obj && obj.paper_id != null ? obj : null;
  } catch {
    return null; // プライベートブラウズでも動くように
  }
}

/** 再生位置を端末に保存する。毎回書くと重いので 5 秒以上空いた・3 秒以上動いたときだけ（09 §4.3）。 */
function saveLastPosition(paper, pos) {
  if (!paper) return;
  const now = Date.now();
  if (now - lastSavedAt < 5000 && Math.abs(pos - lastSavedPos) < 3) return;
  lastSavedAt = now;
  lastSavedPos = pos;
  try {
    localStorage.setItem(RADIO_LAST_KEY, JSON.stringify({
      paper_id: paper.id,
      title: paper.title,
      position_sec: pos,
    }));
  } catch {
    // 保存に失敗しても再生には影響しないので無視する
  }
}

// --- 小さな再生バー（#mini-player・09 §4.6） ---

/** 今の画面では再生バーは出るかを決めて，中身と表示を揃える。 */
function updateMiniPlayer() {
  const mini = document.getElementById("mini-player");
  if (!mini) return;
  // 出す条件: 再生中（または一時停止中）の論文があり，今の画面が再生画面ではない
  const onPlayer = /^#\/radio\/[^/]+$/.test(location.hash);
  const show = !!(currentPaper && !onPlayer);
  mini.hidden = !show;
  if (show) {
    const titleEl = document.getElementById("mini-title");
    if (titleEl) titleEl.textContent = (currentPaper && currentPaper.title) || "無題の論文";
  }
  syncPlayButtons();
  // 重ね部の高さと body.mini-visible を作り直す（app.js・05 §5）
  if (typeof updateBottomStack === "function") updateBottomStack();
}

/** 再生 / 一時停止の記号を実際の状態に合わせる（再生画面と再生バーの両方）。 */
function syncPlayButtons() {
  const audio = radioAudio();
  const paused = !audio || audio.paused;
  const play = document.getElementById("radio-play");
  if (play) {
    play.textContent = paused ? "▶" : "❚❚";
    play.setAttribute("aria-label", paused ? "Play" : "Pause");
  }
  const miniToggle = document.getElementById("mini-toggle");
  if (miniToggle) miniToggle.textContent = paused ? "▶" : "❚❚";
}

function setRadioStatus(text) {
  const status = document.getElementById("radio-status");
  if (status) status.textContent = text || "";
}

/** 15 秒ぶんだけ移動する（前後どちらも端まで）。 */
function skipBy(delta) {
  const audio = radioAudio();
  if (!audio) return;
  const max = Number.isFinite(audio.duration) ? audio.duration : Infinity;
  const next = Math.min(max, Math.max(0, (audio.currentTime || 0) + delta));
  try { audio.currentTime = next; } catch { /* まだ読めていなければ無視 */ }
}

// --- 時刻の表示（つまみと 12:34 / 23:10・09 §4.5） ---

function updateTimeUI() {
  const audio = radioAudio();
  if (!audio) return;
  const dur = Number.isFinite(audio.duration) ? audio.duration : 0;
  const seek = document.getElementById("radio-seek");
  if (seek) {
    seek.max = String(dur);
    // 動かしている間は時刻だけ変えて，離したときの change で currentTime を変える
    if (!seekDragging) seek.value = String(audio.currentTime || 0);
  }
  const durEl = document.getElementById("radio-dur");
  if (durEl) durEl.textContent = formatSec(dur);
  const posEl = document.getElementById("radio-pos");
  if (posEl) posEl.textContent = formatSec(audio.currentTime || 0);
}

// --- 通信エラーからの復帰（移植元と同じ3回まで・09 §4.3） ---

function onAudioError() {
  if (!currentPaper) return;
  const audio = radioAudio();
  if (radioRetry >= RADIO_MAX_RETRY) {
    setRadioStatus("再生できませんでした。通信状況を確認するか，ダウンロードしてから再生してください。");
    return;
  }
  radioRetry += 1;
  const pos = audio ? audio.currentTime : 0;
  setRadioStatus(`読み込みを再試行しています... (${radioRetry}/${RADIO_MAX_RETRY})`);
  setTimeout(() => {
    const a = radioAudio();
    if (!a) return;
    a.load();
    if (pos > 0) { try { a.currentTime = pos; } catch { /* 読めるようになってから */ } }
    // 停止中にエラーが起きたら自動再生しない（利用者の操作なしの再生を避ける）
    if (radioWantPlay) a.play().catch(() => {});
  }, 2000);
}

// --- 文字起こし（移植元の loadSections / highlightSection と同じ動き・09 §4.5） ---

function renderSections() {
  const box = document.getElementById("radio-sections");
  if (!box) return;
  box.innerHTML = "";
  if (!currentSections.length) {
    box.appendChild(el("p", "lib-empty", "文字起こしがありません"));
    return;
  }
  currentSections.forEach((s, i) => {
    const btn = document.createElement("button");
    btn.className = "radio-section";
    btn.dataset.index = String(i);
    btn.textContent = `${formatSec(s.start)}　${(s && s.title) || ""}`;
    btn.addEventListener("click", () => {
      const audio = radioAudio();
      if (!audio) return;
      try { audio.currentTime = Number(s.start) || 0; } catch { /* 読めてから動く */ }
      audio.play().catch(() => {}); // 押すのは利用者なので再生してよい
    });
    box.appendChild(btn);
  });
}

/** 今聞いているセクションだけ色を付ける（09 §4.5）。 */
function highlightSection(pos) {
  const box = document.getElementById("radio-sections");
  if (!box) return;
  const buttons = box.querySelectorAll(".radio-section");
  if (buttons.length === 0) return;
  let active = 0;
  currentSections.forEach((s, i) => {
    const end = s && s.end != null ? Number(s.end) : Infinity;
    if (Number(pos) >= Number(s.start) && Number(pos) < end) active = i;
  });
  buttons.forEach((b) => {
    b.classList.toggle("active", Number(b.dataset.index) === active);
  });
}

// --- オフライン保存（移植元の setupDownloadButton と同じ動き・09 §4.5） ---

/** その音声が倉庫（sai-web-radio-audio）にあるか。 */
async function isAudioDownloaded(id) {
  try {
    const cache = await caches.open(RADIO_AUDIO_CACHE);
    const res = await cache.match(audioCacheKey(id), { ignoreSearch: true });
    return !!res;
  } catch {
    return false; // 倉庫が使えない環境では「無し」扱い
  }
}

/** ダウンロードボタンを作り直す（保存済みなら「ダウンロード済み」・09 §4.5）。 */
async function setupDownloadButton() {
  const button = document.getElementById("radio-download");
  const id = currentPaper && currentPaper.id;
  if (!button || id == null) return;
  button.disabled = false;
  const updateLabel = async () => {
    const saved = await isAudioDownloaded(id);
    if (saved) downloadedIds.add(String(id));
    if (currentPaper && String(currentPaper.id) === String(id)) {
      button.textContent = saved ? "ダウンロード済み" : "この論文をダウンロード";
    }
  };
  await updateLabel();
  button.onclick = async () => {
    button.disabled = true;
    try {
      // 進み具合を出すため，全部を受け取ってから倉庫に保存する（11MB でも数十秒かかる）
      const res = await apiFetch(audioUrl(id));
      if (!res.ok) throw new Error(await res.text());
      const total = Number(res.headers.get("Content-Length") || 0);
      const reader = res.body.getReader();
      const chunks = [];
      let received = 0;
      for (;;) {
        const { done, value } = await reader.read();
        if (done) break;
        chunks.push(value);
        received += value.length;
        setRadioStatus(total
          ? `ダウンロード中... ${(received / 1024 / 1024).toFixed(1)} / ${(total / 1024 / 1024).toFixed(1)} MB`
          : `ダウンロード中... ${(received / 1024 / 1024).toFixed(1)} MB`);
      }
      // 倉庫にしまう住所は合言葉なしに統一する（audioCacheKey・09 §4.5）
      const cache = await caches.open(RADIO_AUDIO_CACHE);
      await cache.put(
        audioCacheKey(id),
        new Response(new Blob(chunks), {
          headers: { "Content-Type": res.headers.get("Content-Type") || "audio/mp4" },
        })
      );
      downloadedIds.add(String(id));
      setRadioStatus("ダウンロードしました。オフラインで再生できます。");
      await updateLabel();
    } catch (err) {
      setRadioStatus(`ダウンロードに失敗しました: ${(err && err.message) || err}`);
    } finally {
      button.disabled = false;
    }
  };
}

// --- ロック画面・コントロールセンターの操作（Media Session・09 §4.7） ---

function setupMediaSession() {
  if (typeof navigator === "undefined" || !("mediaSession" in navigator)) return;
  const audio = radioAudio();
  const set = (action, fn) => {
    try { navigator.mediaSession.setActionHandler(action, fn); } catch { /* 使えない操作は無視 */ }
  };
  set("play", () => { if (audio) audio.play().catch(() => {}); });
  set("pause", () => { if (audio) audio.pause(); });
  set("seekbackward", () => skipBy(-15));
  set("seekforward", () => skipBy(15));
  set("seekto", (details) => {
    if (!audio || !details || !Number.isFinite(details.seekTime)) return;
    try { audio.currentTime = details.seekTime; } catch { /* 読めてから動く */ }
  });
}

function updateMediaSession() {
  if (typeof navigator === "undefined" || !("mediaSession" in navigator)) return;
  if (typeof MediaMetadata === "undefined") return;
  try {
    navigator.mediaSession.metadata = currentPaper
      ? new MediaMetadata({
          title: currentPaper.title || "無題の論文",
          artist: "sai-radio",
        })
      : null;
    const a = radioAudio();
    if (a && !a.paused) navigator.mediaSession.playbackState = "playing";
  } catch { /* 使えない環境では何もしない */ }
}

// --- Radio 一覧（#/papers/radio・09 §4.4） ---

/** 論文情報の戻る先として今の住所を覚える（07 §4.2 と同じ役割）。 */
function saveRadioBackHash() {
  try {
    sessionStorage.setItem(RADIO_PAPERS_LIST_KEY, "#/papers/radio");
  } catch { /* 保存できなくても動く */ }
}

/** 再生画面の戻る先（覚えていなければ Radio 一覧・09 §4.5）。 */
function radioBackHash() {
  try {
    const v = sessionStorage.getItem(RADIO_PAPERS_LIST_KEY);
    if (v) return v;
  } catch { /* 読めなければ既定 */ }
  return "#/papers/radio";
}

/** 一覧に出す行を今の絞り込み・並び順で取り出す（09 §4.4）。 */
function sortedRadioPapers() {
  const q = radioFilter.trim().toLowerCase();
  const rootName = selectedRootName();
  // 選んでいるプロジェクトの音声だけ
  let rows = (radioList || []).filter((p) => radioInProject(p && p.id, rootName));
  if (q) {
    rows = rows.filter((p) => String((p && p.title) || "").toLowerCase().includes(q));
  }
  rows.sort((a, b) => {
    if (radioSort === "fav") {
      const d = ((b && b.favorite) ? 1 : 0) - ((a && a.favorite) ? 1 : 0);
      if (d) return d;
    }
    return (Number(b && b.created_at) || 0) - (Number(a && a.created_at) || 0);
  });
  return rows;
}

/** 登録日（UNIX 秒）を "9/22" の形に。 */
function createdLabel(epochSec) {
  const d = new Date(Number(epochSec) * 1000);
  if (isNaN(d)) return "";
  return `${d.getMonth() + 1}/${d.getDate()}`;
}

/** お気に入りの星を切り替える（行は開かない・09 §4.4）。 */
async function toggleFavorite(p, favBtn) {
  if (!p || p.id == null) return;
  favBtn.disabled = true;
  try {
    const res = await apiFetch(`/api/radio/papers/${encodeURIComponent(p.id)}/favorite`, {
      method: "POST",
    });
    if (!res.ok) return;
    const data = await res.json().catch(() => ({}));
    if (typeof data.favorite === "boolean") p.favorite = data.favorite;
    if (location.hash === "#/papers/radio") renderRadioList();
  } catch {
    // 押し直せるよう状態は変えない
  } finally {
    favBtn.disabled = false;
  }
}

/** 一覧の1行（題名 2 行まで・下に 長さ と日付・右端に ★・09 §4.4）。 */
function radioRow(p) {
  const wrap = el("div", "radio-row");
  const btn = document.createElement("button");
  btn.className = "paper-row radio-main";
  btn.appendChild(el("div", "paper-title", (p && p.title) || "(無題)"));
  const meta = [];
  if (p && p.duration != null) meta.push(`長さ ${formatSec(p.duration)}`);
  if (p && p.created_at != null) meta.push(createdLabel(p.created_at));
  if (downloadedIds.has(String(p && p.id))) meta.push("⬇ 保存済み");
  if (meta.length) btn.appendChild(el("div", "paper-meta", meta.join(" · ")));
  btn.addEventListener("click", () => {
    location.hash = `#/radio/${encodeURIComponent(p.id)}`;
  });
  const fav = document.createElement("button");
  fav.className = "radio-fav" + ((p && p.favorite) ? " on" : "");
  fav.textContent = (p && p.favorite) ? "★" : "☆";
  fav.setAttribute("aria-label", "お気に入り");
  fav.addEventListener("click", (e) => {
    e.stopPropagation(); // 星は行を開かない（09 §4.4）
    void toggleFavorite(p, fav);
  });
  wrap.append(btn, fav);
  return wrap;
}

/** 「続きを聴く」カード（選んでいるプロジェクトの音声のときだけ・09 §4.4）。 */
function renderRadioHome(box) {
  const saved = loadLastPosition();
  if (!saved || !radioInProject(saved.paper_id, selectedRootName())) return;
  const card = el("section", "dash-card radio-resume");
  card.appendChild(el("h3", "dash-card-title", "続きを聴く"));
  card.appendChild(el("div", "radio-resume-title", saved.title || "無題の論文"));
  card.appendChild(el("div", "radio-resume-pos", `前回の位置 ${formatSec(saved.position_sec)}`));
  const btn = el("button", "radio-resume-btn", "続きから再生");
  btn.addEventListener("click", () => {
    // その位置から再生するため，開く前に開始位置を覚えておく（09 §4.4）
    pendingStart = Number(saved.position_sec) || 0;
    location.hash = `#/radio/${encodeURIComponent(saved.paper_id)}`;
  });
  card.appendChild(btn);
  box.appendChild(card);
}

function renderRadioList() {
  const home = document.getElementById("radio-home");
  const listBox = document.getElementById("radio-list");
  if (!home || !listBox) return;

  // 道具行の状態（入力中は絞り込み欄を上書きしない・09 §4.4）
  const filterInput = document.getElementById("radio-filter");
  if (filterInput && document.activeElement !== filterInput) filterInput.value = radioFilter;
  const sortBox = document.getElementById("radio-sort");
  if (sortBox) {
    sortBox.querySelectorAll("button").forEach((b) => {
      b.classList.toggle("active", b.dataset.sort === radioSort);
    });
  }

  home.innerHTML = "";
  listBox.innerHTML = "";
  if (radioList === null) {
    // 失敗の detail をそのまま出す（00 §9）
    home.appendChild(el("p", "lib-empty", radioError || "読み込み中…"));
    return;
  }
  if (!selectedPaperProject()) {
    home.appendChild(el("p", "lib-empty", NO_PROJECT_MSG));
    return;
  }
  renderRadioHome(home);

  const rows = sortedRadioPapers();
  if (rows.length === 0) {
    listBox.appendChild(el(
      "p",
      "lib-empty",
      radioFilter.trim()
        ? "一致する論文がありません"
        : "このプロジェクトの音声はまだありません。Library の論文から作れます。"
    ));
    return;
  }
  for (const p of rows) listBox.appendChild(radioRow(p));
}

/** 保存済みの印を取りに行く（非同期・全部来てから1回だけ描き直す・09 §4.4）。 */
async function markDownloaded() {
  if (!radioList) return;
  let changed = false;
  for (const p of radioList) {
    if (!p || p.id == null) continue;
    if (downloadedIds.has(String(p.id))) continue;
    if (await isAudioDownloaded(p.id)) {
      downloadedIds.add(String(p.id));
      changed = true;
    }
  }
  if (changed && location.hash === "#/papers/radio") renderRadioList();
}

async function openRadioList() {
  saveRadioBackHash(); // 論文情報の戻る先にする（07 §4.2 と同じ・09 §4.2）
  fillSegment("radio-seg", "radio"); // 上部の Dashboard / Discover / Library / Radio（07 §4.1）
  await ensurePaperProjects(); // どのプロジェクトの音声を出すか決める
  if (location.hash !== "#/papers/radio") return; // 既に別の画面へ移っている
  updateMiniPlayer(); // 移動直後に表示を切り替える（通信を待たないため）
  if (radioList === null) renderRadioList(); // 「読み込み中…」を先に出す

  const hash = location.hash;
  let listOk = false;
  try {
    const res = await apiFetch("/api/radio/papers");
    const body = await res.json().catch(() => ({}));
    if (!res.ok) throw new Error((body && body.detail) || "読み込めませんでした");
    if (location.hash !== hash) return; // 既に別の画面へ移っている
    radioList = Array.isArray(body) ? body : [];
    radioError = "";
    listOk = true;
  } catch (e) {
    if (location.hash !== hash) return;
    radioError = (e && e.message) || "読み込めませんでした";
    renderRadioList();
  }

  renderRadioList();
  if (listOk) window.SaiShell.ok();
  else window.SaiShell.fail();
  if (listOk) void markDownloaded();
}

// --- 再生画面（#/radio/<id>・09 §4.5） ---

/** 読み込めなかったときの表示（見出しの下の1行に出す）。 */
function playerError(text) {
  const titleEl = document.getElementById("radio-title");
  if (titleEl) titleEl.textContent = "Play";
  const msg = document.getElementById("radio-player-msg");
  if (msg) {
    msg.textContent = text;
    msg.hidden = false;
  }
}

async function openRadioPlayer(params) {
  const id = String(params && params.id != null ? params.id : "");
  const hash = location.hash;
  // 「論文情報 ›」の判定にプロジェクト一覧が必要（18 §1.2）。読めなくても再生は続行
  await ensurePaperProjects();
  if (location.hash !== hash) return; // 既に別の画面へ移っている
  const startSec = pendingStart;
  pendingStart = null; // その場で片付ける（失敗しても次のを開いたときに出ないように）

  updateMiniPlayer(); // 移動直後に表示を切り替える（通信を待たないため）
  playerPaperId = id;
  const msg = document.getElementById("radio-player-msg");
  if (msg) { msg.hidden = true; msg.textContent = ""; }
  const link = document.getElementById("radio-paper-link");
  // 音声の名前が分けられて，そのプロジェクトが分かるときだけ出す（18 §1.2・§3.3）
  if (link) link.hidden = !radioProjectOf(getPaperProjects(), id);
  setRadioStatus("");
  // 切り替えと操作カードの sticky 位置は通信を待たずに先に合わせる（10 §4.1）
  renderRadioPane();
  syncRadioControlsTop();

  const samePaper = !!(currentPaper && String(currentPaper.id) === id);
  if (!samePaper) {
    // 別の論文を開いたときは声の質問の接続も閉じる（移植元と同じ・10 §4.4）
    closeLive();
    radioChat = [];
    radioChatError = "";
    radioChatLoadedFor = null;
    renderRadioChat();
    // 別の論文を開いたときだけ表示と音声を捨てる（同じ論文なら今の再生を続ける・09 §4.3）
    currentSections = [];
    renderSections();
    const titleEl = document.getElementById("radio-title");
    if (titleEl) titleEl.textContent = "読み込み中…";
    updateTimeUI();
    syncPlayButtons();
  }

  // 題名と文字起こし（sections）を読む
  let data = null;
  let notFound = false;
  try {
    const res = await apiFetch(`/api/radio/papers/${encodeURIComponent(id)}`);
    if (res.status === 404) {
      notFound = true;
    } else {
      const body = await res.json().catch(() => ({}));
      if (!res.ok) throw new Error((body && body.detail) || "読み込めませんでした");
      data = body;
    }
  } catch (e) {
    if (location.hash !== hash) return;
    playerError((e && e.message) || "読み込めませんでした");
    window.SaiShell.fail();
    return;
  }
  if (location.hash !== hash) return; // 既に別の画面へ移っている
  if (notFound || !data) {
    playerError("音声が見つかりません"); // 09 §4.5
    window.SaiShell.ok();
    return;
  }

  const title = data.title || "無題の論文";
  const titleEl = document.getElementById("radio-title");
  if (titleEl) titleEl.textContent = title;

  const audio = radioAudio();
  currentPaper = { id, title };
  if (!samePaper) {
    radioRetry = 0;
    radioWantPlay = false;
    speedIndex = 0;
    if (audio) {
      audio.playbackRate = RADIO_SPEEDS[0];
      audio.src = audioUrl(id); // 差し替えるのはこのときだけ（09 §4.3）
    }
    const speedBtn = document.getElementById("radio-speed");
    if (speedBtn) speedBtn.textContent = `${RADIO_SPEEDS[0]}x`;
    updateTimeUI();
  }
  updateMediaSession();

  currentSections = Array.isArray(data.sections) ? data.sections : [];
  renderSections();
  // 過去の質問と回答を読む（開き直しても残っているように・10 §4.2）
  if (radioChatLoadedFor !== String(id)) void loadRadioChat(id);
  void setupDownloadButton();
  window.SaiShell.ok();
  updateMiniPlayer(); // 再生画面なので再生バーは隠れる

  // 「続きから再生」から来たときだけ，その位置へ移して再生を試みる（09 §4.5）
  if (startSec != null && startSec > 0 && audio) {
    try { audio.currentTime = startSec; } catch { /* 読めてから動く */ }
    audio.play().catch(() => {}); // 利用者がボタンを押してきたので試みる
  }
}

// --- 質問チャット（文字起こし | 質問・計画 10 §4.1〜4.3） ---

/** 切り替えの選択（"transcript" | "ask"）は端末ごとに覚える（10 §4.1）。 */
function radioPane() {
  try {
    if (sessionStorage.getItem(RADIO_PANE_KEY) === "ask") return "ask";
  } catch { /* 読めなければ既定 */ }
  return "transcript";
}

function setRadioPane(pane) {
  try {
    sessionStorage.setItem(RADIO_PANE_KEY, pane === "ask" ? "ask" : "transcript");
  } catch { /* 保存できなくても動く */ }
  renderRadioPane();
}

/** 質問の件数を切り替えのボタンに入れる（「質問（3）」・10 §4.1）。 */
function updatePaneCount() {
  const btn = document.querySelector("#radio-pane-seg button[data-pane='ask']");
  if (btn) btn.textContent = radioChat.length ? `質問（${radioChat.length}）` : "質問";
}

/** 文字起こし / 質問の切り替えと表示の中身を今の状態に合わせる（10 §4.1）。 */
function renderRadioPane() {
  const pane = radioPane();
  const seg = document.getElementById("radio-pane-seg");
  if (seg) {
    seg.querySelectorAll("button").forEach((b) => {
      b.classList.toggle("active", b.dataset.pane === pane);
    });
  }
  updatePaneCount();
  const sections = document.getElementById("radio-sections");
  if (sections) sections.hidden = pane !== "transcript";
  const chat = document.getElementById("radio-chat");
  if (chat) chat.hidden = pane !== "ask";
  const bar = document.getElementById("radio-inputbar");
  if (bar) bar.hidden = pane !== "ask";
  const status = document.getElementById("radio-ask-status");
  if (status) status.hidden = pane !== "ask";
  if (pane === "ask") scrollRadioChat();
}

/** 新しい吹き出しを足したあと，一番下へスクロールする（質問のときだけ・10 §4.2）。 */
function scrollRadioChat() {
  if (radioPane() !== "ask") return;
  window.scrollTo(0, document.body.scrollHeight);
}

/** 吹き出しの時刻（UNIX 秒）を ISO 文字列に（app.js の timeLabel / dateSepLabel が受ける形・10 §4.2）。 */
function chatIso(t) {
  const ms = Number(t);
  return new Date(Number.isFinite(ms) ? ms * 1000 : Date.now()).toISOString();
}

/** 章の位置から表示用の1行を作る（「03:12 手法 について」・10 §4.2）。 */
function positionLabel(sec) {
  const n = Number(sec);
  const t = Number.isFinite(n) ? n : 0;
  const section = sectionAt(currentSections, t);
  if (section && section.title) return `${formatSec(t)} ${section.title} について`;
  return `${formatSec(t)} の位置`;
}

/**
 * 吹き出し1つ（Agent の .fold-item をそのまま使い，最初から開いた状態・10 §4.2）。
 * kind は "user"（右寄せの青い吹き出し）か "agent"（左の吹き出し）。
 * opts: {error} 失敗の文（赤字） / {pending} 「考え中…」の表示。
 */
function chatFold(kind, t, text, opts) {
  const o = opts || {};
  const iso = chatIso(t);
  const fold = foldItem(`radio-chat-${(chatSeq += 1)}`, iso);
  fold.wrap.classList.add("open"); // 質問と回答は短いので最初から開いた状態（押すと閉じられる・10 §4.2）
  const body = document.createElement("div");
  if (kind === "user") {
    fold.wrap.classList.add("kind-user");
    fold.who.textContent = "あなた";
    body.className = "fold-body bubble-user";
    body.textContent = text || "";
  } else {
    fold.wrap.classList.add("kind-agent");
    fold.who.textContent = "sai-radio";
    body.className = "fold-body bubble-agent";
    if (o.error) {
      body.classList.add("radio-ask-error");
      body.textContent = o.error;
    } else if (o.pending) {
      body.textContent = "考え中…";
    } else {
      body.innerHTML = renderMarkdown(text || "");
    }
  }
  fold.wrap.appendChild(body);
  return fold.wrap;
}

/** 質問と回答を古い順に並べ直して描く（日付が変わった所に区切り線・10 §4.2）。 */
function renderRadioChat() {
  updatePaneCount();
  const box = document.getElementById("radio-chat");
  if (!box) return;
  box.innerHTML = "";
  if (radioChatError) {
    box.appendChild(el("p", "lib-empty", radioChatError)); // detail をそのまま出す（00 §9）
    return;
  }
  if (radioChat.length === 0) {
    box.appendChild(el("p", "lib-empty", "まだ質問はありません"));
    return;
  }
  let lastKey = "";
  for (const pair of radioChat) {
    const iso = chatIso(pair.t);
    const key = dateKey(iso);
    if (key && key !== lastKey) {
      const sep = document.createElement("div");
      sep.className = "date-sep";
      sep.textContent = dateSepLabel(iso); // ── 9月23日(火) ──（app.js と同じ・10 §4.2）
      box.appendChild(sep);
      lastKey = key;
    }
    const q = chatFold("user", pair.t, pair.question);
    q.appendChild(el("div", "radio-q-pos", positionLabel(pair.position_sec))); // 位置の章（10 §4.2）
    box.appendChild(q);
    if (pair.answer == null && pair.error) {
      box.appendChild(chatFold("agent", pair.t, null, { error: pair.error }));
    } else if (pair.answer == null) {
      box.appendChild(chatFold("agent", pair.t, null, { pending: true }));
    } else {
      box.appendChild(chatFold("agent", pair.t, pair.answer));
    }
  }
}

/** 失敗のとき detail（日本語の文）を取り出す（10 §4.3）。 */
async function resDetail(res, fallback) {
  try {
    const body = await res.json();
    if (body && typeof body.detail === "string" && body.detail) return body.detail;
  } catch { /* JSON でなければ下の既定文を使う */ }
  return fallback;
}

/** 開いたとき，過去の質問と回答を古い順に読む（`GET /api/radio/logs`・10 §4.2）。 */
async function loadRadioChat(id) {
  radioChatLoadedFor = String(id);
  radioChat = [];
  radioChatError = "";
  renderRadioChat();
  const hash = location.hash;
  try {
    const res = await apiFetch(`/api/radio/logs?paper_id=${encodeURIComponent(id)}`);
    const body = await res.json().catch(() => null);
    if (!res.ok) throw new Error((body && typeof body.detail === "string" && body.detail) || "読み込めませんでした");
    const events = Array.isArray(body) ? body : [];
    const pairs = events
      .filter((e) => e && e.kind === "question" && e.payload)
      .sort((a, b) => Number(a.created_at || 0) - Number(b.created_at || 0))
      .map((e) => ({
        t: Number(e.created_at) || Math.floor(Date.now() / 1000),
        position_sec: e.payload.position_sec,
        question: String(e.payload.question || ""),
        answer: String(e.payload.answer || ""),
        error: null,
      }))
      .filter((p) => p.question || p.answer);
    if (location.hash !== hash || radioChatLoadedFor !== String(id)) return; // 既に別の画面・別の論文
    radioChat = pairs;
    radioChatError = "";
    window.SaiShell.ok();
  } catch (e) {
    if (location.hash !== hash || radioChatLoadedFor !== String(id)) return;
    radioChatError = (e && e.message) || "読み込めませんでした";
    window.SaiShell.fail();
  }
  renderRadioChat();
  scrollRadioChat();
}

/** 質問と回答を1組足して一番下へスクロールする（声の質問の答えにも使う・10 §4.2）。 */
function pushRadioPair(question, answer, positionSec) {
  radioChat.push({
    t: Math.floor(Date.now() / 1000),
    position_sec: positionSec,
    question: String(question || ""),
    answer: String(answer || ""),
    error: null,
  });
  renderRadioChat();
  scrollRadioChat();
}

// --- 文字で質問（➤・計画 10 §4.3） ---

async function sendRadioAsk() {
  if (radioAsking) return;
  const ta = document.getElementById("radio-input");
  const sendBtn = document.getElementById("radio-send");
  const question = ((ta && ta.value) || "").trim();
  const id = playerPaperId;
  // 空なら何もしない。ID の判定は sai-radio の16進 ID を通す（10 §4.3・直し）
  if (!question || !isRadioPaperId(id)) return;
  const audio = radioAudio();
  const positionSec = audio ? Number(audio.currentTime) || 0 : 0;
  radioAsking = true;
  if (sendBtn) sendBtn.disabled = true; // 送信中はボタンを無効にする（10 §4.3）
  // 送信した瞬間に入力欄を空にし高さも戻す（送れたように見えるようにして，
  // 待ち時間の間に何度も押して同じ質問を重ね送りしないため・CP3）
  if (ta) {
    ta.value = "";
    growRadioInput();
  }

  // 自分の吹き出しをすぐ足し，回答の場所に「考え中…」を出す（10 §4.3）
  const pair = {
    t: Math.floor(Date.now() / 1000),
    position_sec: positionSec,
    question,
    answer: null,
    error: null,
  };
  radioChat.push(pair);
  renderRadioChat();
  scrollRadioChat();
  try {
    const res = await apiFetch(`/api/radio/papers/${encodeURIComponent(id)}/ask`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ question, position_sec: positionSec }),
    });
    if (!res.ok) throw new Error(await resDetail(res, "質問できませんでした"));
    const data = await res.json().catch(() => ({}));
    pair.answer = (data && data.answer) || "";
    window.SaiShell.ok();
  } catch (e) {
    pair.error = (e && e.message) || "質問できませんでした"; // detail を赤字で（10 §4.3）
    // 失敗したときだけ，入力欄がまだ空のときに限り質問文を戻す
    // （空でなければ打ちかけの文字があるので触らず，打ち直し不要にする・CP3）
    if (ta && !ta.value.trim()) {
      ta.value = question;
      growRadioInput();
    }
    window.SaiShell.fail();
  } finally {
    radioAsking = false;
    if (sendBtn) sendBtn.disabled = false;
    renderRadioChat();
    scrollRadioChat();
  }
}

// --- 声で質問（Gemini Live API 直結・移植元 sai-radio の質問の部分を移す・10 §4.4） ---
//
// 仕組み（移植元と同じ）:
// 1. 🎤 を押す → ポッドキャストを一時停止し，Mac から「合鍵（トークン）＋論文本文」をもらう
// 2. iPhone から Google に直接 WebSocket でつなぎ，論文本文を1回だけ送る（setup）
// 3. マイクの音を生の PCM に変換して 100ms ずつ送る（もう一度押すと送信終了）
// 4. 回答の音声を受け取って順番に再生し，字幕も吹き出しに残す
// 5. 接続は開いたままにする。続けて質問するときは 3 から繰り返すだけ

let liveSocket = null;      // Google との WebSocket（使い回す）
let livePaperId = null;     // 今つながっている接続がどの論文用か
let liveState = "idle";     // idle / connecting / recording / waiting / answering
let liveMicStream = null;   // マイクの音の流れ
let liveInputCtx = null;    // 録音用の AudioContext
let liveOutputCtx = null;   // 再生用の AudioContext
let liveWorkletNode = null; // PCM 変換部品
let livePositionSec = 0;    // 質問したときの再生位置
let liveQuestionText = "";  // 聞き取り結果（断片が届くのでつなげる）
let liveAnswerText = "";    // 回答の字幕（同上）
let liveAnswerTimer = null; // 回答終了後にポッドキャストを再開するための予約
let liveIdleTimer = null;   // 3分質問がなければ接続を閉じる予約
let liveNextStartTime = 0;  // 次の回答音声を鳴らし始める時刻（途切れなくつなぐため）
let liveSession = null;     // Mac からもらった接続情報（トークン等）

/** 質問中の状態を入力欄のすぐ上の1行に出す（10 §4.4）。 */
function setAskStatus(text) {
  const status = document.getElementById("radio-ask-status");
  if (status) status.textContent = text || "";
}

/** 🎤 の見た目を状態に合わせる（録音中は赤い丸・10 §4.4）。 */
function updateAskButton() {
  const button = document.getElementById("radio-mic");
  if (!button) return;
  button.classList.toggle("recording", liveState === "recording");
  button.disabled = liveState === "connecting" || liveState === "waiting";
}

/**
 * Int16（-32768〜32767）の音の列を base64 に変換する（移植元から移す・10 §4.4）。
 * 注意: ここに渡すのは必ず Int16Array にすること。
 * Live API は「1サンプル2バイトの整数（リトルエンディアン）」しか受け付けない。
 */
function int16ToBase64(int16) {
  const bytes = new Uint8Array(int16.buffer, int16.byteOffset, int16.byteLength);
  let bin = "";
  for (let i = 0; i < bytes.length; i++) bin += String.fromCharCode(bytes[i]);
  return btoa(bin);
}

/** base64 を Float32（-1.0〜1.0）に戻す（回答の音声・移植元から移す・10 §4.4）。 */
function base64ToFloat32(base64) {
  const bin = atob(base64);
  const int16 = new Int16Array(bin.length / 2);
  for (let i = 0; i < int16.length; i++) {
    int16[i] = bin.charCodeAt(i * 2) | (bin.charCodeAt(i * 2 + 1) << 8);
  }
  const floats = new Float32Array(int16.length);
  for (let i = 0; i < int16.length; i++) floats[i] = int16[i] / 0x8000;
  return floats;
}

/**
 * Google からの受信データを JSON に直す（移植元から移す・10 §4.4）。
 * Live API は応答を WebSocket の「バイナリフレーム」で送ってくるため，
 * 文字列ならそのまま，バイナリなら TextDecoder で文字列に直してから JSON.parse する。
 */
function parseLiveJson(data) {
  try {
    if (typeof data === "string") return JSON.parse(data);
    const text = new TextDecoder().decode(data);
    return JSON.parse(text);
  } catch (err) {
    // 握りつぶさず警告を出す（届いているのに捨てている状態を見逃さないため・移植元から）
    let typeName = typeof data;
    if (data instanceof ArrayBuffer) typeName = "ArrayBuffer";
    else if (typeof Blob !== "undefined" && data instanceof Blob) typeName = "Blob";
    let preview = "";
    try {
      if (typeof data === "string") preview = data.slice(0, 120);
      else if (data instanceof ArrayBuffer) preview = new TextDecoder().decode(data.slice(0, 120));
      else preview = String(data).slice(0, 120);
    } catch {
      preview = "(先頭の表示に失敗)";
    }
    console.warn(`Live API の受信データを JSON にできませんでした(型=${typeName} 先頭=${preview})`, err);
    throw err;
  }
}

/** Mac から合鍵（トークン）と論文本文をもらう（通信先は 09 の中継・10 §4.4）。 */
async function fetchLiveSession() {
  const res = await apiFetch(`/api/radio/papers/${encodeURIComponent(livePaperId)}/live-session`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ position_sec: livePositionSec }),
  });
  if (!res.ok) throw new Error(await resDetail(res, "合鍵をもらえませんでした"));
  return res.json();
}

/** Google に WebSocket でつなぎ，論文本文を1回だけ送る（setup）・10 §4.4。 */
async function connectLive() {
  livePaperId = currentPaper ? currentPaper.id : null;
  liveSession = await fetchLiveSession();
  const url = `${liveSession.ws_url}?access_token=${encodeURIComponent(liveSession.token)}`;
  liveSocket = new WebSocket(url);
  // 受信は同期的に扱える ArrayBuffer で受け取る（移植元と同じ・10 §4.4）
  liveSocket.binaryType = "arraybuffer";

  await new Promise((resolve, reject) => {
    let finished = false; // 二重に結果を出さないための印
    let opened = false;   // onopen を見たかどうかの印
    const timer = setTimeout(() => {
      if (finished) return;
      finished = true;
      if (!opened) {
        reject(new Error("接続がタイムアウトしました。Google に接続できませんでした（通信が届いていない可能性）"));
      } else {
        reject(new Error("接続がタイムアウトしました。接続はできましたが設定が拒否されました（setupComplete が届きませんでした）"));
      }
    }, 15000);
    liveSocket.onopen = () => {
      opened = true;
      // 接続直後に1回だけ送る設定。responseModalities は AUDIO 固定（移植元から・10 §4.4）
      liveSocket.send(JSON.stringify({
        setup: {
          model: liveSession.model,
          generationConfig: { responseModalities: ["AUDIO"] },
          systemInstruction: { parts: [{ text: liveSession.system_instruction }] },
          realtimeInputConfig: { automaticActivityDetection: { disabled: true } },
          inputAudioTranscription: {},
          outputAudioTranscription: {},
        },
      }));
    };
    liveSocket.onmessage = (e) => {
      let msg;
      try {
        msg = parseLiveJson(e.data);
      } catch {
        return;
      }
      if (msg.setupComplete) {
        if (finished) return;
        finished = true;
        clearTimeout(timer);
        resolve();
      }
    };
    // エラーだけでは原因が分からないので切断の理由を待つ（移植元から・10 §4.4）
    liveSocket.onerror = () => { /* onclose の code / reason を見る */ };
    liveSocket.onclose = (e) => {
      onLiveClose(e);
      if (finished) return;
      finished = true;
      clearTimeout(timer);
      const code = e.code;
      const reason = e.reason || "";
      if (!opened) {
        reject(new Error(`接続が閉じられました（code=${code} ${reason}）。Google に接続できませんでした（通信が届いていない可能性）`));
      } else {
        reject(new Error(`接続が閉じられました（code=${code} ${reason}）。接続はできましたが設定が拒否されました`));
      }
    };
  });

  // つながった後の受信処理（回答音声・字幕・終了合図）を登録する（移植元から・10 §4.4）
  liveSocket.onmessage = (e) => onLiveMessage(e);
  liveSocket.onclose = (e) => onLiveClose(e);
}

/** Google から届いたメッセージをさばく（移植元から移す・10 §4.4）。 */
function onLiveMessage(e) {
  let msg;
  try {
    msg = parseLiveJson(e.data);
  } catch {
    return;
  }
  const content = msg.serverContent;
  if (!content) return;
  // 回答の音声（PCM 24000Hz）が届いたら順番に再生する
  const parts = (content.modelTurn && content.modelTurn.parts) || [];
  for (const part of parts) {
    if (part.inlineData && part.inlineData.data) {
      liveState = "answering";
      updateAskButton();
      setAskStatus("回答中…");
      playAnswerChunk(part.inlineData.data);
    }
  }
  // 聞き取り結果と字幕は断片的に届くので，ターン終了時にまとめて吹き出しにする
  if (content.inputTranscription && content.inputTranscription.text) {
    liveQuestionText += content.inputTranscription.text;
  }
  if (content.outputTranscription && content.outputTranscription.text) {
    liveAnswerText += content.outputTranscription.text;
  }
  // このターンの応答が終わった合図 → ログを送り，ポッドキャスト再開を予約する
  if (content.turnComplete) finishLiveTurn();
}

/** 接続が切れたときの処理（移植元から移す・10 §4.4）。 */
function onLiveClose(e) {
  // 自分から閉じた場合（code 1000）は何もしない
  if (e && e.code && e.code !== 1000 && liveState !== "idle") {
    setAskStatus(`接続が切れました（code=${e.code} ${e.reason || ""}）。もう一度質問すると接続し直します。`);
  }
  liveSocket = null;
  livePaperId = null;
  if (liveState !== "idle" && liveState !== "recording") liveState = "idle";
  updateAskButton();
}

/** 回答音声の1かけら（base64・PCM 24000Hz）を途切れなく再生する（移植元から・10 §4.4）。 */
function playAnswerChunk(base64) {
  if (!liveOutputCtx) liveOutputCtx = new AudioContext();
  if (liveOutputCtx.state === "suspended") liveOutputCtx.resume();
  const floats = base64ToFloat32(base64);
  const buffer = liveOutputCtx.createBuffer(1, floats.length, 24000);
  buffer.getChannelData(0).set(floats);
  const source = liveOutputCtx.createBufferSource();
  source.buffer = buffer;
  source.connect(liveOutputCtx.destination);
  const now = liveOutputCtx.currentTime;
  // 前のかけらがまだ鳴っている場合はその終わりに続ける
  const startAt = Math.max(now, liveNextStartTime);
  source.start(startAt);
  liveNextStartTime = startAt + buffer.duration;
  clearTimeout(liveAnswerTimer);
  // 最後のかけらが鳴り終わったらポッドキャストを再開する（少し余裕を見て +0.5 秒後）
  const waitMs = Math.max(0, (liveNextStartTime - liveOutputCtx.currentTime) * 1000) + 500;
  liveAnswerTimer = setTimeout(() => {
    const a = radioAudio(); // ポッドキャストは画面共通の1つだけ（09 §4.3）
    if (a) a.play().catch(() => {});
    setAskStatus("");
    if (liveState === "answering") {
      liveState = "idle";
      updateAskButton();
    }
  }, waitMs);
}

/** 1ターンの応答が終わったら Mac にログを送り，吹き出しに追記する（移植元から・10 §4.4）。 */
async function finishLiveTurn() {
  try {
    await apiFetch(`/api/radio/papers/${encodeURIComponent(livePaperId || "")}/question-log`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({
        position_sec: livePositionSec,
        question: liveQuestionText,
        answer: liveAnswerText,
      }),
    });
  } catch {
    // ログ送信に失敗しても回答自体は届いているので無視する（移植元から）
  }
  pushRadioPair(liveQuestionText, liveAnswerText, livePositionSec); // 聞き取り結果と字幕を吹き出しに（10 §4.4）
  // 3分質問がなければ接続を閉じる（Live API のセッションには時間上限があるため・10 §4.4）
  clearTimeout(liveIdleTimer);
  liveIdleTimer = setTimeout(() => closeLive(), 3 * 60 * 1000);
}

/** マイクを開始し，PCM を 100ms ずつ Google に送る（移植元から・10 §4.4）。 */
async function startLiveRecording() {
  const stream = await navigator.mediaDevices.getUserMedia({ audio: true });
  liveMicStream = stream;
  // 録音用は 16000Hz で作る。iPhone が無視した場合は部品側で揃える
  liveInputCtx = new AudioContext({ sampleRate: 16000 });
  if (liveInputCtx.state === "suspended") await liveInputCtx.resume();
  await liveInputCtx.audioWorklet.addModule("/pcm-worklet.js");
  const source = liveInputCtx.createMediaStreamSource(stream);
  liveWorkletNode = new AudioWorkletNode(liveInputCtx, "pcm-capture");
  liveWorkletNode.port.postMessage({ targetRate: 16000, chunkSize: 1600 });
  liveWorkletNode.port.onmessage = (e) => {
    // 100ms 分（1600サンプル）の PCM を base64 にして送る（部品からは Int16 で来る）
    if (liveSocket && liveSocket.readyState === WebSocket.OPEN) {
      const int16 = e.data.pcm16 instanceof Int16Array ? e.data.pcm16 : new Int16Array(e.data.pcm16);
      liveSocket.send(JSON.stringify({
        realtimeInput: {
          audio: { mimeType: "audio/pcm;rate=16000", data: int16ToBase64(int16) },
        },
      }));
    }
  };
  source.connect(liveWorkletNode);
  // 発話の開始を明示的に伝える（自動検出は setup で無効化している）
  liveSocket.send(JSON.stringify({ realtimeInput: { activityStart: {} } }));
}

/** マイクを止め，発話の終了を伝える（移植元から・10 §4.4）。 */
function stopLiveRecording() {
  try {
    if (liveSocket && liveSocket.readyState === WebSocket.OPEN) {
      liveSocket.send(JSON.stringify({ realtimeInput: { activityEnd: {} } }));
    }
  } catch {
    // 送信に失敗しても録音停止は続ける
  }
  if (liveWorkletNode) {
    liveWorkletNode.disconnect();
    liveWorkletNode = null;
  }
  if (liveInputCtx) {
    liveInputCtx.close().catch(() => {});
    liveInputCtx = null;
  }
  if (liveMicStream) {
    liveMicStream.getTracks().forEach((t) => t.stop());
    liveMicStream = null;
  }
}

/** 接続を閉じる。別の論文を開いたとき・3分経ったときに呼ぶ（移植元から・10 §4.4）。 */
function closeLive(onlyIfPaperId) {
  if (onlyIfPaperId && livePaperId !== onlyIfPaperId) return;
  if (liveState === "recording") stopLiveRecording();
  clearTimeout(liveIdleTimer);
  clearTimeout(liveAnswerTimer);
  if (liveSocket) {
    try {
      liveSocket.close();
    } catch {
      // 閉じるときのエラーは無視する
    }
    liveSocket = null;
  }
  livePaperId = null;
  liveState = "idle";
  updateAskButton();
}

/** 🎤 の1押しが状態（idle → connecting → recording → waiting → answering）を進める（10 §4.4）。 */
async function toggleAsk() {
  // 録音中にもう一度押したら送信する
  if (liveState === "recording") {
    stopLiveRecording();
    liveState = "waiting";
    updateAskButton();
    setAskStatus("考え中…");
    return;
  }
  if (liveState === "connecting" || liveState === "waiting" || liveState === "answering") return;
  if (!currentPaper) return;

  const audio = radioAudio();
  livePositionSec = audio ? Number(audio.currentTime) || 0 : 0;
  if (audio) audio.pause(); // 質問中は音声を一時停止する（10 §1）

  // 新しい質問の準備。聞き取り結果と字幕を空にする
  liveQuestionText = "";
  liveAnswerText = "";
  liveNextStartTime = 0;
  clearTimeout(liveAnswerTimer);
  clearTimeout(liveIdleTimer);

  try {
    // まだつながっていない・別の論文の接続なら，トークンをもらって接続する
    if (!liveSocket || liveSocket.readyState !== WebSocket.OPEN || livePaperId !== currentPaper.id) {
      liveState = "connecting";
      updateAskButton();
      setAskStatus("接続中…");
      await connectLive();
    }
    await startLiveRecording();
    liveState = "recording";
    updateAskButton();
    setAskStatus("話してください（もう一度押すと送信）");
  } catch (err) {
    liveState = "idle";
    updateAskButton();
    setAskStatus(`エラー: ${(err && err.message) || err}`);
  }
}

// --- 音声化の依頼（論文情報の #paper-audio-slot・計画 10 §4.5） ---

const radioPendingIds = new Set(); // この画面で依頼した（まだ作成中と分かる）論文
const radioSlotErrors = new Map(); // 直前の依頼で失敗した文（id → detail・10 §4.5）
let radioSlotTimer = null;         // 10秒ごとの読み直し（画面を離れたら止める・10 §4.5）

function stopRadioSlotPoll() {
  if (radioSlotTimer) {
    clearInterval(radioSlotTimer);
    radioSlotTimer = null;
  }
}

/** 10秒ごとに論文情報を読み直し，`ready` / `failed` になったら描き直す（10 §4.5）。 */
function startRadioSlotPoll(projectName, id, slot) {
  stopRadioSlotPoll();
  radioSlotTimer = setInterval(async () => {
    // そのプロジェクトの論文情報から離れたら止める（10 §4.5・18 §3.5）
    if (location.hash !== paperHash(projectName, id, false)) {
      stopRadioSlotPoll();
      return;
    }
    try {
      const res = await apiFetch(`${papersApiBase(projectName)}/${encodeURIComponent(id)}`);
      if (!res.ok) return; // 次の10秒にもう一度試す
      const data = await res.json().catch(() => null);
      const status = data && data.podcast && data.podcast.status;
      if (!status || status === "generating") return;
      // ready / failed に変わった
      radioPendingIds.delete(String(id));
      radioSlotErrors.delete(String(id));
      stopRadioSlotPoll();
      if (document.getElementById("paper-audio-slot") === slot) renderPaperAudioSlot(data, slot);
    } catch {
      // 通信に失敗しても次の10秒でもう一度試す
    }
  }, RADIO_SLOT_POLL_MS);
}

/** 「🎙 音声を作る」を押したあと（確認 → 202 → 作成中…・10 §4.5）。 */
async function requestRadioAudio(projectName, id, detail, slot) {
  const yes = confirm("この論文の音声を作りますか？\n数分かかります。できたら通知でお知らせします。");
  if (!yes) return;
  radioSlotErrors.delete(String(id));
  try {
    const res = await apiFetch(`${papersApiBase(projectName)}/${encodeURIComponent(id)}/radio`, { method: "POST" });
    if (res.status === 202) {
      radioPendingIds.add(String(id));
      const next = { ...detail, podcast: { ...(detail.podcast || {}), status: "generating" } };
      renderPaperAudioSlot(next, slot);
      window.SaiShell.ok();
      return;
    }
    // 409・400 は detail をそのまま出す（10 §4.5）
    radioSlotErrors.set(String(id), await resDetail(res, "音声を作れませんでした"));
    renderPaperAudioSlot(detail, slot);
    window.SaiShell.fail();
  } catch (e) {
    radioSlotErrors.set(String(id), (e && e.message) || "音声を作れませんでした");
    renderPaperAudioSlot(detail, slot);
    window.SaiShell.fail();
  }
}

/** 論文情報の音声の置き場（状態ごとの表示は 10 §4.5 の表のとおり）。 */
function renderPaperAudioSlot(detail, slot) {
  stopRadioSlotPoll(); // 再描画のたびに読み直しを作り直す
  if (!detail || !slot) return;
  const id = detail.paper && detail.paper.id;
  const projectName = currentPaperProject(); // 開いている論文のプロジェクト（18 §3.5）
  const files = detail.files || {};
  const podcast = detail.podcast || {};
  const status = podcast.status || null;
  const pending = radioPendingIds.has(String(id));
  const wrap = el("div", "paper-actions");
  const notes = [];
  const err = radioSlotErrors.get(String(id));
  if (err) notes.push({ cls: "radio-slot-error", text: err });

  if (status === "ready") {
    // 再生画面は音声の名前（radio_id）で開く（09 のまま・18 §3.4）
    const btn = el("button", "paper-btn", "▶ 音声を聴く");
    btn.addEventListener("click", () => {
      const proj = paperProjectByName(projectName);
      const radioId = podcast.radio_id || (proj ? `${proj.root_name}-${id}` : id);
      location.hash = `#/radio/${encodeURIComponent(radioId)}`;
    });
    wrap.appendChild(btn);
  } else if (status === "generating" || pending) {
    const busy = el("span", "radio-slot-busy");
    busy.appendChild(el("span", "papers-spin")); // 回るしるし（08 のものを使い回す）
    busy.appendChild(el("span", null, "音声を作成中…（数分かかります。できたら通知します）"));
    wrap.appendChild(busy);
    if (typeof Notification === "undefined" || Notification.permission !== "granted") {
      notes.push({ cls: "radio-slot-note", text: "通知を受け取るには Agent タブの 🔔 を押してください" });
    }
    startRadioSlotPoll(projectName, id, slot);
  } else if (!files.extract) {
    const btn = el("button", "paper-btn", "🎙 音声を作る");
    btn.disabled = true; // 本文が無いので作れない（10 §4.5）
    wrap.appendChild(btn);
    notes.push({ cls: "radio-slot-note", text: "本文がまだ取り出されていないため作れません" });
  } else {
    const btn = el("button", "paper-btn", "🎙 音声を作る");
    btn.addEventListener("click", () => void requestRadioAudio(projectName, id, detail, slot));
    wrap.appendChild(btn);
    if (status === "failed") {
      notes.push({ cls: "radio-slot-note", text: `前回は失敗しました: ${podcast.error || "原因不明のエラー"}` });
    }
  }

  slot.innerHTML = "";
  slot.appendChild(wrap);
  for (const note of notes) slot.appendChild(el("div", note.cls, note.text));
}

/** 操作カードを appbar の直下に残すため，appbar の高さを CSS 変数に入れる（10 §4.1）。 */
function syncRadioControlsTop() {
  const bar = document.querySelector("#view-radio-player .appbar");
  if (bar) document.documentElement.style.setProperty("--appbar-h", `${bar.offsetHeight}px`);
}

/** 質問の入力欄を中身の高さに合わせる（app.js の autoGrow と同じやり方・10 §4.1）。 */
function growRadioInput() {
  const ta = document.getElementById("radio-input");
  if (!ta) return;
  ta.style.height = "auto";
  const max = parseFloat(getComputedStyle(ta).maxHeight);
  ta.style.height = `${Math.min(ta.scrollHeight, Number.isFinite(max) ? max : 120)}px`;
}

// --- 初期化（ブラウザでのみ・09 §4） ---

if (typeof window !== "undefined" && window.document) {
  // 07 の論文情報が呼ぶ入れ物をまだ用意していなければ作る（papers.js が先に作る・07 §4.4-3）
  window.SaiPaperHooks = window.SaiPaperHooks || [];

  // 論文情報の音声の置き場（▶ 音声を聴く / 🎙 音声を作る / 作成中…・10 §4.5）
  window.SaiPaperHooks.push(renderPaperAudioSlot);

  // 画面の登録（09 §4.2）
  (window.SaiRoutes = window.SaiRoutes || []).push(
    {
      match(hash) {
        return hash === "#/papers/radio" ? {} : null;
      },
      view: "view-papers-radio",
      tab: "papers",
      hideTabbar: false,
      interval: 0, // 定期更新はしない（読むのは開いたときだけ・09 §4.2）
      async open() {
        await openRadioList();
      },
      async poll() {},
    },
    {
      match(hash) {
        const m = hash.match(/^#\/radio\/([^/]+)$/);
        return m ? { id: decodeURIComponent(m[1]) } : null;
      },
      view: "view-radio-player",
      tab: "papers",
      hideTabbar: true, // 10 で入力欄を置くためタブバーを隠す（09 §4.2）
      interval: 0,
      async open(params) {
        await openRadioPlayer(params);
      },
      async poll() {},
    }
  );

  // 画面が切り替わったら再生バーの出し入れを直す（app.js が dispatch する・09 §4.1）
  window.addEventListener("sai-route", updateMiniPlayer);

  // --- 小さな再生バーの中身（09 §4.6） ---
  const mini = document.getElementById("mini-player");
  if (mini) {
    mini.innerHTML = "";
    const miniTitle = el("button", "mini-title");
    miniTitle.id = "mini-title";
    miniTitle.setAttribute("aria-label", "再生画面を開く");
    const miniToggle = el("button", "mini-btn mini-toggle", "▶");
    miniToggle.id = "mini-toggle";
    miniToggle.setAttribute("aria-label", "再生 / 一時停止");
    const miniFwd = el("button", "mini-btn", "15⏩");
    miniFwd.id = "mini-fwd";
    miniFwd.setAttribute("aria-label", "15 秒進む");
    const miniClose = el("button", "mini-btn", "✕");
    miniClose.id = "mini-close";
    miniClose.setAttribute("aria-label", "再生バーを閉じる");
    mini.append(miniTitle, miniToggle, miniFwd, miniClose);

    // 題名を押すと再生画面に戻る（09 §4.6）
    miniTitle.addEventListener("click", () => {
      if (currentPaper) location.hash = `#/radio/${encodeURIComponent(currentPaper.id)}`;
    });
    miniToggle.addEventListener("click", () => {
      const audio = radioAudio();
      if (!audio) return;
      if (audio.paused) audio.play().catch(() => {});
      else audio.pause();
    });
    miniFwd.addEventListener("click", () => skipBy(15));
    // ✕ は一時停止して今の論文を忘れ，バーを隠す（09 §4.6）
    miniClose.addEventListener("click", () => {
      const audio = radioAudio();
      if (audio) {
        audio.pause();
        audio.removeAttribute("src");
        audio.load();
      }
      currentPaper = null;
      currentSections = [];
      updateMediaSession();
      updateMiniPlayer();
    });
  }

  // --- 再生画面の部品（09 §4.5） ---
  const backBtn = document.getElementById("radio-back");
  if (backBtn) backBtn.addEventListener("click", () => { location.hash = radioBackHash(); });
  const paperLink = document.getElementById("radio-paper-link");
  if (paperLink) {
    paperLink.addEventListener("click", () => {
      // 音声の名前からプロジェクトを引き，その論文情報へ（18 §1.2・§3.3・§3.5）
      const proj = radioProjectOf(getPaperProjects(), playerPaperId);
      const parts = splitRadioName(playerPaperId);
      if (!proj || !parts) return;
      // 戻ったときの Library も同じプロジェクトになるよう，選んでいるのも切り替える（18 §1.2）
      setSelectedProject(proj.name);
      location.hash = paperHash(proj.name, parts.paper_id, false);
    });
  }
  const playBtn = document.getElementById("radio-play");
  if (playBtn) {
    playBtn.addEventListener("click", () => {
      const audio = radioAudio();
      if (!audio) return;
      if (audio.paused) audio.play().catch(() => {});
      else audio.pause();
    });
  }
  const rewBtn = document.getElementById("radio-rew");
  if (rewBtn) rewBtn.addEventListener("click", () => skipBy(-15));
  const fwdBtn = document.getElementById("radio-fwd");
  if (fwdBtn) fwdBtn.addEventListener("click", () => skipBy(15));
  const speedBtn = document.getElementById("radio-speed");
  if (speedBtn) {
    speedBtn.addEventListener("click", () => {
      // 押すたびに 1 → 1.25 → 1.5 → 2 → 1（09 §4.5）
      speedIndex = (speedIndex + 1) % RADIO_SPEEDS.length;
      const rate = RADIO_SPEEDS[speedIndex];
      const audio = radioAudio();
      if (audio) audio.playbackRate = rate;
      speedBtn.textContent = `${rate}x`;
    });
  }
  // つまみ: 動かしている間は時刻だけ変え，離したら currentTime を変える（09 §4.5）
  const seek = document.getElementById("radio-seek");
  if (seek) {
    seek.addEventListener("input", () => {
      seekDragging = true;
      const posEl = document.getElementById("radio-pos");
      if (posEl) posEl.textContent = formatSec(Number(seek.value));
    });
    seek.addEventListener("change", () => {
      const audio = radioAudio();
      const v = Number(seek.value);
      if (audio && Number.isFinite(v)) {
        try { audio.currentTime = v; } catch { /* 読めてから動く */ }
      }
      seekDragging = false;
      updateTimeUI();
    });
  }

  // --- 道具行（絞り込みと並び順・09 §4.4） ---
  const filterInput = document.getElementById("radio-filter");
  if (filterInput) {
    filterInput.addEventListener("input", (e) => {
      radioFilter = (e.target && e.target.value) || "";
      if (radioList !== null) renderRadioList();
    });
  }
  const sortBox = document.getElementById("radio-sort");
  if (sortBox) {
    sortBox.querySelectorAll("button").forEach((b) => {
      b.addEventListener("click", () => {
        if (b.dataset.sort !== "new" && b.dataset.sort !== "fav") return;
        radioSort = b.dataset.sort;
        renderRadioList();
      });
    });
  }

  // --- 文字起こし | 質問（N）の切り替え（10 §4.1） ---
  const paneSeg = document.getElementById("radio-pane-seg");
  if (paneSeg) {
    paneSeg.innerHTML = "";
    const seg = document.createElement("div");
    seg.className = "dash-seg"; // 06 の期間切り替えと同じ見た目（10 §4.1）
    for (const [key, label] of [["transcript", "文字起こし"], ["ask", "質問"]]) {
      const b = document.createElement("button");
      b.dataset.pane = key;
      b.textContent = label;
      b.addEventListener("click", () => setRadioPane(key));
      seg.appendChild(b);
    }
    paneSeg.appendChild(seg);
  }
  renderRadioPane();

  // --- 質問の入力欄（🎤・文字欄・➤・10 §4.1・4.3・4.4） ---
  const askSend = document.getElementById("radio-send");
  if (askSend) askSend.addEventListener("click", () => void sendRadioAsk());
  const askInput = document.getElementById("radio-input");
  if (askInput) askInput.addEventListener("input", growRadioInput);
  const askMic = document.getElementById("radio-mic");
  if (askMic) askMic.addEventListener("click", () => void toggleAsk());
  syncRadioControlsTop();
  window.addEventListener("resize", syncRadioControlsTop);

  // --- <audio> の出来事（画面をまたいで1つだけ・09 §4.3） ---
  setupMediaSession();
  const audio = radioAudio();
  if (audio) {
    audio.addEventListener("play", () => {
      radioWantPlay = true;
      radioRetry = 0;
      // listen は論文ごとに最初の再生で 1 回だけ送る（09 §4.3）
      if (currentPaper && !listenedIds.has(String(currentPaper.id))) {
        listenedIds.add(String(currentPaper.id));
        void apiFetch(`/api/radio/papers/${encodeURIComponent(currentPaper.id)}/listen`, {
          method: "POST",
        }).catch(() => {});
      }
      try {
        if (typeof navigator !== "undefined" && navigator.mediaSession) {
          navigator.mediaSession.playbackState = "playing";
        }
      } catch { /* 使えない環境では無視 */ }
      syncPlayButtons();
      updateMiniPlayer();
    });
    audio.addEventListener("pause", () => {
      radioWantPlay = false;
      saveLastPosition(currentPaper, audio.currentTime);
      try {
        if (typeof navigator !== "undefined" && navigator.mediaSession) {
          navigator.mediaSession.playbackState = "paused";
        }
      } catch { /* 使えない環境では無視 */ }
      syncPlayButtons();
      updateMiniPlayer();
    });
    // 位置が変わるたびに文字起こしの色と「続きを聴く」の保存を直す（移植元と同じ・09 §4.3）
    audio.addEventListener("timeupdate", () => {
      highlightSection(audio.currentTime);
      saveLastPosition(currentPaper, audio.currentTime);
      updateTimeUI();
    });
    audio.addEventListener("loadedmetadata", updateTimeUI);
    audio.addEventListener("durationchange", updateTimeUI);
    audio.addEventListener("playing", () => setRadioStatus(""));
    audio.addEventListener("waiting", () => setRadioStatus("読み込み中..."));
    audio.addEventListener("stalled", () => setRadioStatus("読み込み中..."));
    audio.addEventListener("error", onAudioError);
    syncPlayButtons();
  }
  updateMiniPlayer();
}

// --- node からの読み込み（tests/test_radio.js・09 §4.11・10 §4.6・18 §3.3） ---
if (typeof module !== "undefined" && module.exports) {
  module.exports = {
    formatSec,
    sectionAt,
    isRadioPaperId,
    splitRadioName,
    radioProjectOf,
    radioInProject,
  };
}
