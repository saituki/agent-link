// 会話の中に「行」や「カード」を置く位置を決める，DOM を使わない小さな関数（確認点 CP3 の直し）。
//
// - 会話の行は時刻順に並べる（app.js の appendItems が使う）
// - 回答済みの質問カードは「質問された時刻」の位置に置く
//   （created_at よ後の最初の会話行の直前。無ければ末尾）
// - 生成中の文（partial）をどう出し入れするかを決める（app.js の applyPartial が使う）
//
// index.html で app.js より先に読み込む。node からの読み込みは tests/test_chatorder.js。
"use strict";

/**
 * 時刻の値をミリ秒にそろえる。
 *
 * - 数字: UNIX 秒（質問カードの created_at）
 * - 文字列: ISO 8601（会話の行の time）
 * - それ以外・読めないもの: null
 *
 * @param {string|number|null|undefined} value
 * @returns {number|null} ミリ秒、読めなければ null
 */
function timeMs(value) {
  if (typeof value === "number" && Number.isFinite(value)) {
    // 1e11 ms ≒ 1973年。より小さい数字は UNIX 秒とみなす
    return value < 1e11 ? value * 1000 : value;
  }
  if (typeof value === "string" && value.trim() !== "") {
    const t = Date.parse(value);
    return Number.isNaN(t) ? null : t;
  }
  return null;
}

/**
 * t より後の最初の行の番号（その行の「前」に置く位置を返す）。
 *
 * - t より後の行が1つも無ければ times.length（＝一番下／末尾）
 * - t が読めないときも末尾（今の並びを崩さないため）
 * - 時刻の読めない行は判断できないので数えず，先の行を見る
 *
 * @param {Array<string|number|null>} times 行の時刻（古い順）
 * @param {string|number|null} t 基準の時刻（UNIX 秒 or ISO 文字列）
 * @returns {number} 挿す位置（0 〜 times.length）
 */
function firstIndexAfter(times, t) {
  const list = Array.isArray(times) ? times : [];
  const base = timeMs(t);
  if (base == null) return list.length;
  for (let i = 0; i < list.length; i += 1) {
    const m = timeMs(list[i]);
    if (m == null) continue; // 時刻の読めない行は飛ばす
    if (m > base) return i;
  }
  return list.length;
}

/**
 * 生成中の文（partial）をどう扱うかを決める。
 *
 * app.js は毎回の読み込みで返る partial を渡し，返った合図どおりに
 * 吹き出しを出す・中身を置き換える・消す。DOM には触らない。
 *
 * - 空 → 空: "none"（何もしない）
 * - 空 → 文: "show"（吹き出しを出す）
 * - 文 → 同じ文: "none"（描き直さない）
 * - 文 → 別の文: "update"（中身を置き換える）
 * - 文 → 空: "hide"（吹き出しを消す）
 *
 * 空白だけの文は「空」とみなす（見えない吹き出しを出さない）。
 *
 * @param {string|null|undefined} prevPartial 直前に出していた生成中の文
 * @param {string|null|undefined} nextPartial 今回の読み込みで返った生成中の文
 * @returns {"none"|"show"|"update"|"hide"}
 */
function chatPartialPlan(prevPartial, nextPartial) {
  const prev = typeof prevPartial === "string" ? prevPartial : "";
  const next = typeof nextPartial === "string" ? nextPartial : "";
  const prevEmpty = prev.trim() === "";
  const nextEmpty = next.trim() === "";
  if (prevEmpty && nextEmpty) return "none";
  if (prevEmpty) return "show";
  if (nextEmpty) return "hide";
  return prev === next ? "none" : "update";
}

// --- node からの読み込み（tests/test_chatorder.js） ---
if (typeof module !== "undefined" && module.exports) {
  module.exports = { timeMs, firstIndexAfter, chatPartialPlan };
}
