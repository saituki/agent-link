"use strict";
// 質問カードの読み上げ・音声回答（U01）。DOM を使わない関数と，画面の部品だけを書く。
// サーバーは変えず，端末内の音声機能（speechSynthesis / SpeechRecognition）だけを使う。
// 一番外側の名前は va で始める（他の画面ファイルと重ならないようにするため）。

// --- DOM を使わない部分（tests/test_voiceask.js が使う） ---

// 「(Recommended)」「（推奨）」などの末尾の印を取り去って，「、おすすめ」と読み直す
function vaReadLabel(label) {
  const s = String(label == null ? "" : label).trim();
  const m = s.match(/^(.*?)[\s、,]*[（(]\s*(?:Recommended|recommended|推奨)\s*[）)]\s*$/);
  return m ? `${m[1]}、おすすめ` : s;
}

// 1 問ぶんを読み上げる文にする。index は 0 始まり，total は問の数
function vaSpeechText(one, index, total) {
  const q = one && typeof one === "object" ? one : {};
  const parts = [];
  const n = Number(total) || 0;
  if (n >= 2) parts.push(`${n}問中${Number(index) + 1}問目。`);
  const header = String(q.header || "").trim();
  const question = String(q.question || "").trim();
  if (header) parts.push(`${header}。`);
  if (question) parts.push(`${question}。`);
  if (!header && !question) parts.push(`質問${Number(index) + 1}。`);
  const opts = Array.isArray(q.options) ? q.options : [];
  opts.forEach((opt, i) => {
    const o = typeof opt === "string" ? { label: opt } : (opt || {});
    parts.push(`${i + 1}番、${vaReadLabel(o.label)}。`);
    const desc = String(o.description || "").trim();
    if (desc && desc !== String(o.label || "").trim()) parts.push(`${desc}。`);
  });
  if (q.multiSelect === true) parts.push("いくつでも選べます。");
  parts.push("番号か、選択肢の名前で答えてください。");
  return parts.join("");
}

// 見比べ用の文字列にする（全角半角・空白・句読点を無視して比べられるように）
function vaNormalize(s) {
  return String(s == null ? "" : s)
    .normalize("NFKC")
    .toLowerCase()
    .replace(/[\s\u3000、。，．,.!?！？「」『』()（）・ー－−-]/g, "");
}

// 数字の言い方の表（長いものから順に試す）。値は 1 始まり。
const VA_NUMBER_WORDS = [
  ["ひとつめ", 1], ["ふたつめ", 2], ["みっつめ", 3], ["よっつめ", 4], ["いつつめ", 5],
  ["むっつめ", 6], ["ななつめ", 7], ["やっつめ", 8], ["ここのつめ", 9],
  ["ひとつ", 1], ["ふたつ", 2], ["みっつ", 3], ["よっつ", 4], ["いつつ", 5],
  ["むっつ", 6], ["ななつ", 7], ["やっつ", 8], ["ここのつ", 9],
  ["きゅう", 9], ["いち", 1], ["よん", 4], ["さん", 3], ["しち", 8], ["ろく", 7],
  ["なな", 7], ["はち", 8], ["に", 2], ["し", 5], ["ご", 6], ["く", 10],
  ["一", 1], ["二", 2], ["三", 3], ["四", 4], ["五", 5], ["六", 6],
  ["七", 7], ["八", 8], ["九", 9], ["十", 10],
];
// 後ろに付いたときだけ数字とみなす語（番・ばん・つ目・つめ・個目・こめ）
const VA_COUNTER_RE = /^(?:番|ばん|つ目|つめ|個目|こめ)/;

// 発話から選んだ選択肢の番号（0 始まり）を集める。範囲外は無視する。
function vaFindNumbers(spoken, max) {
  const raw = String(spoken == null ? "" : spoken);
  const norm = vaNormalize(raw);
  const out = [];
  let i = 0;
  while (i < raw.length) {
    // 書きたい数字を探す（算用数字・漢数字・かなの数え方）
    const digits = /^\d{1,2}/.exec(raw.slice(i));
    let value = 0;
    let token = "";
    if (digits) {
      value = parseInt(digits[0], 10);
      token = digits[0];
    } else {
      for (const [w, v] of VA_NUMBER_WORDS) {
        if (raw.startsWith(w, i)) { value = v; token = w; break; }
      }
    }
    if (!value) { i += 1; continue; }
    // 「3 番」のように後ろに言葉があるときか，発話全体が数字のときだけ数える
    const isNumber = VA_COUNTER_RE.test(raw.slice(i + token.length));
    const isWhole = norm !== "" && norm === vaNormalize(token);
    if ((isNumber || isWhole) && value >= 1 && value <= max) out.push(value - 1);
    i += Math.max(token.length, 1);
  }
  return out;
}

// 発話を回答の形に変える。戻り値は state[qi] と同じ形（空のときは null）。
function vaMatchAnswer(spoken, options, multi) {
  const text = String(spoken == null ? "" : spoken);
  if (!text.trim()) return null;
  const opts = Array.isArray(options) ? options : [];
  const labels = opts.map((o) =>
    vaNormalize(vaReadLabel(typeof o === "string" ? o : ((o || {}).label || "")))
  );
  // 1. 番号を探す
  const byNumber = vaFindNumbers(text, opts.length);
  // 2. 選択肢の名前を探す（どちらの 文章にも 2 文字以上あれば一致とみなす）
  const norm = vaNormalize(text);
  const hits = new Set();
  labels.forEach((lab, i) => {
    if (!lab) return;
    if (lab.length >= 2 && norm.includes(lab)) hits.add(i);
    else if (norm.length >= 2 && lab.includes(norm)) hits.add(i);
  });
  // 3. まとめる
  if (multi) {
    const all = new Set(byNumber);
    hits.forEach((i) => all.add(i));
    const picked = [...all].sort((a, b) => a - b);
    return picked.length ? { choice_indices: picked } : { text: text.trim() };
  }
  if (byNumber.length) return { choice_index: byNumber[0] };
  if (hits.size === 1) return { choice_index: [...hits][0] };
  return { text: text.trim() };
}

// --- 画面の部品（ブラウザだけで使う） ---

/**
 * 質問カードの上に「🔊 読み上げ」「🎤」の 1 行を足し，読み上げ → 聞き取り → 次の問、を順に流す。
 * 答えは state[qi] と同じ形で apply に渡すだけ（送信はしない。誤認識でも勝手に送らないため）。
 * ctl = { apply(qi, answer), isAnswered(qi) }
 * 使えない環境では何も足さず null を返す（エラーで画面を止めない）。
 */
function vaAttach(card, qs, ctl) {
  // カードは appendChild される前に渡される（この時点では card.isConnected は false）ので見ない
  if (!card) return null;
  if (typeof document === "undefined") return null;
  if (typeof window === "undefined") return null;
  if (!window.speechSynthesis || !window.SpeechSynthesisUtterance) return null;
  const questions = Array.isArray(qs) ? qs : [];
  const control = ctl && typeof ctl === "object" ? ctl : {};
  const SR = window.SpeechRecognition || window.webkitSpeechRecognition || null;

  const bar = document.createElement("div");
  bar.className = "va-bar";
  const speakBtn = document.createElement("button");
  speakBtn.type = "button";
  speakBtn.className = "va-speak";
  speakBtn.textContent = "🔊 読み上げ";
  const micBtn = document.createElement("button");
  micBtn.type = "button";
  micBtn.className = "va-listen";
  micBtn.textContent = "🎤";
  micBtn.hidden = true;
  const status = document.createElement("span");
  status.className = "va-status";
  bar.append(speakBtn, micBtn, status);
  const first = card.querySelector(".q-block");
  if (first) card.insertBefore(bar, first);
  else card.appendChild(bar);

  // 今の状態（-1 は動いていない）
  const st = { qi: -1, busy: false, rec: null };
  const setStatus = (s) => { status.textContent = s || ""; };
  const mark = (on) => {
    bar.classList.toggle("recording", !!on);
    micBtn.classList.toggle("recording", !!on);
  };

  const stopAll = () => {
    st.busy = false;
    if (st.rec) {
      try { st.rec.abort(); } catch { /* 止まらなくても卡片は抜けたので無視 */ }
      st.rec = null;
    }
    try { window.speechSynthesis.cancel(); } catch { /* 同じく黙って無視 */ }
    mark(false);
  };

  const nextUnanswered = (from) => {
    for (let i = from == null ? 0 : from; i < questions.length; i++) {
      if (typeof control.isAnswered === "function" && control.isAnswered(i)) continue;
      return i;
    }
    return -1;
  };

  const utterance = (text, onend) => {
    const u = new window.SpeechSynthesisUtterance(text);
    u.lang = "ja-JP";
    try {
      const voices = window.speechSynthesis.getVoices ? window.speechSynthesis.getVoices() : [];
      const v = (voices || []).find((x) => x && String(x.lang || "").toLowerCase().indexOf("ja") === 0);
      if (v) u.voice = v;
    } catch { /* 声の一覧が読めなければ既定ののまま */ }
    if (typeof onend === "function") u.onend = onend;
    return u;
  };

  // 問を 1 つ読み上げて，終わったら聞き取りへ進む
  const speak = (qi) => {
    if (!card.isConnected) { stopAll(); return; }
    st.qi = qi;
    setStatus("読み上げ中…");
    const u = utterance(vaSpeechText(questions[qi], qi, questions.length), () => {
      if (!card.isConnected) { stopAll(); return; }
      listenOnce(qi);
    });
    u.onerror = () => {
      if (card.isConnected) setStatus("読み上げできませんでした");
      st.busy = false;
    };
    // 1 回目はボタンを押した操作の中で呼ぶ（iOS はそうしないと音を出さない）
    try {
      window.speechSynthesis.speak(u);
    } catch {
      st.busy = false;
      listenOnce(qi); // 話せなかったときは聞き取りへ進む
    }
  };

  // 今の問を聞き取る。返り値は「🎤 に戻したか」
  const listenOnce = (qi) => {
    if (!card.isConnected) { stopAll(); return; }
    st.qi = qi;
    if (!SR) {
      // 聞き取りが使えない環境では読み上げだけ
      st.busy = false;
      setStatus("読み上げました（この端末では聞き取りを使えません）");
      return;
    }
    const rec = new SR();
    st.rec = rec;
    rec.lang = "ja-JP";
    rec.interimResults = false;
    rec.continuous = false;
    rec.onresult = (e) => {
      let text = "";
      const results = (e && e.results) || [];
      for (const r of results) text += r[0].transcript;
      st.rec = null;
      mark(false);
      const answer = vaMatchAnswer(
        text,
        (questions[qi] && questions[qi].options) || [],
        !!(questions[qi] && questions[qi].multiSelect === true)
      );
      if (answer && typeof control.apply === "function") control.apply(qi, answer);
      setStatus(`聞き取り: ${text}`);
      const next = nextUnanswered(qi + 1);
      if (next >= 0) speak(next);
      else {
        st.busy = false;
        micBtn.hidden = true;
        setStatus("送信ボタンを押してください");
        const u = utterance("送信ボタンを押してください", () => {});
        try { window.speechSynthesis.speak(u); } catch { /* 読めなくても状態欄は出ている */ }
      }
    };
    rec.onerror = (e) => {
      st.rec = null;
      mark(false);
      const err = (e && e.error) || "";
      if (err === "no-speech" || err === "aborted") {
        st.busy = false;
        micBtn.hidden = false;
        setStatus("聞き取れませんでした");
        return;
      }
      // 権限が無い／音声サービスが止まっているときは 🎤 を出しておく
      st.busy = false;
      micBtn.hidden = false;
      setStatus("🎤 を押して答えてください");
    };
    setStatus("聞き取り中…");
    try {
      rec.start();
      mark(true);
    } catch {
      st.rec = null;
      st.busy = false;
      mark(false);
      micBtn.hidden = false;
      setStatus("🎤 を押して答えてください");
    }
  };

  speakBtn.addEventListener("click", () => {
    if (st.busy) { stopAll(); setStatus(""); return; }
    const qi = nextUnanswered(0);
    if (qi < 0) { setStatus("全て回答済みです"); return; }
    st.busy = true;
    speak(qi); // この押した操作の中で最初の speak が呼ばれる
  });

  micBtn.addEventListener("click", () => {
    if (st.busy) { stopAll(); return; }
    const qi = st.qi;
    if (qi < 0) return;
    st.busy = true;
    listenOnce(qi); // この押した操作の中で start() が呼ばれる
  });

  return { bar, speakBtn, micBtn, status, stop: stopAll };
}

// --- node からの読み込み（tests/test_voiceask.js） ---
if (typeof module !== "undefined" && module.exports) {
  module.exports = {
    vaSpeechText,
    vaMatchAnswer,
    vaNormalize,
  };
}
