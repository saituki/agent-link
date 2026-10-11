"use strict";
// マウスの操作（U04）。#ts-overlay と #ts-axis の押す・動かす・ホイールを受ける。
// モード（TS.state.mode）で左ボタンの役目が変わる。純関数（tsMouseDraft）は末尾で node のテストに渡す。

const tsPanStep = 0.1; // ホイールで 1 回に動く表示幅の割合
const tsDragPx = 3; // これより動いたらドラッグ扱い
const tsMouseCursors = { create: "crosshair", move: "grab", zoom: "zoom-in" };

let tsDrag = null; // {mode: "press"・"pan"・"nav", ...}
let tsSpace = false; // スペースを押したまま（パン）

// ---- 純関数（DOM を使わない。node のテストが読む） ----

// y の位置にあるレーン（無ければ null）
function tsMouseLane(frame, y) {
  return frame.lanes.find((l) => y >= l.y0 && y < l.y1) || null;
}

// 作成モードで、ドラッグかクリックの結果からイベントの下書きを作る。作れないときは {error}
// def: カテゴリ（G1-C2）。start・end: {x, y}。frame: tsLanesFrame()。moved: ドラッグしたか
function tsMouseDraft(def, start, end, frame, moved) {
  if (!def) return { error: "イベント定義を選んでください" };
  const type = def.type || (moved ? "area" : "line");
  if (type === "line" && moved) return { error: "この定義はクリックで作ります" };
  if (type === "area" && !moved) return { error: "この定義はドラッグで作ります" };
  const lane = tsMouseLane(frame, start.y);
  if (def.ch === "single" && !lane) return { error: "レーンの中で始めてください" };
  if (type === "line") {
    const vars = def.ch === "all" ? [] : lane ? [lane.var] : [];
    return { type, t: frame.toT(start.x), vars };
  }
  const a = Math.min(frame.toT(start.x), frame.toT(end.x));
  const b = Math.max(frame.toT(start.x), frame.toT(end.x));
  const t0 = Math.max(frame.t0, a);
  const t1 = Math.min(frame.t1, b);
  if (!(t1 > t0)) return { error: "範囲が小さすぎます" };
  let vars;
  if (def.ch === "single") vars = [lane.var];
  else if (def.ch === "all") vars = [];
  else vars = tsSelectionFromDrag(start.x, end.x, start.y, end.y, frame).vars;
  return { type, t0, t1, vars };
}

// ---- ここから画面（ブラウザだけで動く） ----

// マウスの位置（#ts-overlay の中の座標。#ts-axis も左の端は同じ）
function tsMousePoint(e) {
  const r = document.getElementById("ts-overlay").getBoundingClientRect();
  return { x: e.clientX - r.left, y: e.clientY - r.top };
}

// 記録全体の帯（#ts-axis の下の部分）の上の座標。y は #ts-axis の中の座標
function tsAxisPoint(e) {
  const ax = document.getElementById("ts-axis").getBoundingClientRect();
  return { x: tsMousePoint(e).x, y: e.clientY - ax.top };
}

// 作成に使う定義（無ければ null）
function tsMouseDef() {
  const st = window.TS.state;
  return (st.schema.categories || []).find((c) => c.name === st.eventDef) || null;
}

function tsMouseCursor() {
  const ov = document.getElementById("ts-overlay");
  if (!ov) return;
  const grabbing = tsDrag && tsDrag.mode === "pan";
  ov.style.cursor = grabbing ? "grabbing" : tsMouseCursors[window.TS.state.mode] || "default";
}

// 押したあいだの範囲の見た目（作成は帯、拡大は縦の全体に薄い帯）
function tsMouseLayer(ctx, frame) {
  const d = tsDrag;
  if (!d || d.mode !== "press" || !d.moved) return;
  const acc = tsCol.accent;
  const mode = window.TS.state.mode;
  if (mode === "zoom") {
    ctx.fillStyle = tsRgba(acc, 0.12);
    ctx.fillRect(Math.min(d.x0, d.x), 0, Math.abs(d.x - d.x0), frame.height);
    return;
  }
  if (mode !== "create") return;
  const draft = tsMouseDraft(tsMouseDef(), { x: d.x0, y: d.y0 }, { x: d.x, y: d.y }, frame, true);
  if (draft.error || draft.type !== "area") return;
  const vars = draft.vars.length ? draft.vars : frame.lanes.map((l) => l.var);
  tsPaintRange(ctx, frame, draft.t0, draft.t1, vars, tsRgba(acc, 0.12), acc);
}

// 画面の中の時刻に当たるイベント（ダブルクリックと、クリックの選びで使う）
function tsHitAt(x, y, tol) {
  const frame = tsLanesFrame();
  const lane = tsMouseLane(frame, y);
  return tsHitEvent(tsVisibleEvents(), frame.toT(x), lane ? lane.var : null, tol);
}

// 記録全体の帯を押した位置へ移動する（表示幅は保つ）
function tsNavJump(x) {
  const st = window.TS.state;
  const dur = tsDuration(st.meta);
  if (!(dur > 0)) return;
  const frame = tsLanesFrame();
  const t = Math.min(dur, Math.max(0, ((x - tsGutterW) / frame.plotW) * dur));
  const span = st.t1 - st.t0;
  const r = tsClampRange(t - span / 2, t + span / 2, dur, span);
  window.TS.setView({ t0: r.t0, t1: r.t1 });
}

// 幅を保ったまま、押した位置から動いた分だけずらす
function tsMousePan(d, x) {
  const st = window.TS.state;
  const dt = -((x - d.x0) / tsLanesFrame().plotW) * (d.t1 - d.t0);
  const r = tsPan(d.t0, d.t1, dt, tsDuration(st.meta));
  window.TS.setView({ t0: r.t0, t1: r.t1 });
}

function tsOnDown(e) {
  const st = window.TS.state;
  if (!st.meta) return;
  const p = tsMousePoint(e);
  if (e.button === 1 || (e.button === 0 && tsSpace)) {
    e.preventDefault();
    tsDrag = { mode: "pan", x0: p.x, t0: st.t0, t1: st.t1 };
    tsMouseCursor();
    return;
  }
  if (e.button !== 0 || p.x < tsGutterW) return;
  e.preventDefault();
  if (st.mode === "create") {
    const def = tsMouseDef();
    if (!def) {
      window.TS.ui.toast("イベント定義を選んでください（ヘッダかイベントのタブ）");
      return;
    }
    if (def.ch === "single" && !tsMouseLane(tsLanesFrame(), p.y)) return; // レーンの外では始めない
  }
  tsDrag = { mode: "press", x0: p.x, y0: p.y, x: p.x, y: p.y, moved: false, t0: st.t0, t1: st.t1 };
}

function tsOnAxisDown(e) {
  const st = window.TS.state;
  if (!st.meta || e.button !== 0) return;
  const p = tsAxisPoint(e);
  if (p.x < tsGutterW) return;
  const frame = tsLanesFrame();
  if (p.y < frame.nav.y0) return; // 時間の目盛りの所は何もしない（記録全体の帯だけ）
  e.preventDefault();
  tsDrag = { mode: "nav" };
  tsNavJump(p.x);
}

function tsOnMove(e) {
  const d = tsDrag;
  if (!d) return;
  if (d.mode === "nav") {
    tsNavJump(tsAxisPoint(e).x);
    return;
  }
  const p = tsMousePoint(e);
  if (d.mode === "pan") {
    tsMousePan(d, p.x);
    return;
  }
  d.x = p.x;
  d.y = p.y;
  if (!d.moved && Math.hypot(p.x - d.x0, p.y - d.y0) >= tsDragPx) d.moved = true;
  if (d.moved && window.TS.state.mode === "move") tsMousePan(d, p.x);
  else if (d.moved) tsDrawOverlay();
}

function tsOnUp(e) {
  const d = tsDrag;
  if (!d) return;
  tsDrag = null;
  tsMouseCursor();
  if (d.mode === "press") tsMouseRelease(d, e);
  tsDrawOverlay();
}

// 左ボタンを離したとき。モードごとに選ぶ・範囲を見せる・作る
function tsMouseRelease(d, e) {
  const st = window.TS.state;
  const mode = st.mode;
  if (mode === "move") {
    if (d.moved) return; // パンは動いたときに済んでいる
    const hit = tsHitAt(d.x0, d.y0, tsTolSec());
    if (hit) {
      tsSelectEvents([hit.id]);
    } else {
      window.TS.setView({ selectedEvents: [], selection: null });
      window.TS.emit("select", []);
    }
    return;
  }
  if (mode === "zoom") {
    if (!d.moved) return;
    const frame = tsLanesFrame();
    const a = Math.min(frame.toT(d.x0), frame.toT(d.x));
    const b = Math.max(frame.toT(d.x0), frame.toT(d.x));
    const r = tsClampRange(a, b, tsDuration(st.meta), tsMinSamples / st.meta.srate);
    window.TS.setView({ t0: r.t0, t1: r.t1 });
    return;
  }
  // 作成: エリアはクリックで選び、ドラッグで範囲を作る。ポイントはクリックで作る
  const def = tsMouseDef();
  if (!def) return;
  const type = def.type || (d.moved ? "area" : "line");
  if (type === "area" && !d.moved) {
    const hit = tsHitAt(d.x0, d.y0, tsTolSec());
    if (hit) tsSelectEvents([hit.id]);
    return;
  }
  if (type === "line" && d.moved) return;
  const draft = tsMouseDraft(def, { x: d.x0, y: d.y0 }, { x: d.x, y: d.y }, tsLanesFrame(), d.moved);
  if (draft.error) {
    window.TS.ui.toast(draft.error);
    return;
  }
  window.TS.createEvent(draft, def, { x: e.clientX, y: e.clientY });
}

function tsOnDbl(e) {
  const p = tsMousePoint(e);
  if (p.x < tsGutterW) return;
  const hit = tsHitAt(p.x, p.y, tsTolSec());
  if (!hit) return;
  tsSelectEvents([hit.id]);
  window.TS.editEvent(hit.id, { anchor: { x: e.clientX, y: e.clientY } });
}

// 縦の回転は横の移動。Ctrl・⌘ を押すと（トラックパッドのピンチを含む）マウスの位置を中心に拡大縮小
function tsOnWheel(e) {
  e.preventDefault();
  const st = window.TS.state;
  if (!st.meta) return;
  const dur = tsDuration(st.meta);
  const minSpan = tsMinSamples / st.meta.srate;
  if (e.ctrlKey || e.metaKey) {
    const tc = tsLanesFrame().toT(tsMousePoint(e).x);
    const r = tsZoomAt(st.t0, st.t1, tc, Math.exp(e.deltaY * 0.0015), dur, minSpan);
    window.TS.setView({ t0: r.t0, t1: r.t1 });
    return;
  }
  const d = Math.abs(e.deltaX) > Math.abs(e.deltaY) ? e.deltaX : e.deltaY;
  if (!d) return;
  const r = tsPan(st.t0, st.t1, Math.sign(d) * tsPanStep * (st.t1 - st.t0), dur);
  window.TS.setView({ t0: r.t0, t1: r.t1 });
}

// 小窓の外の左クリックは、小窓を閉じるだけ（他の操作はしない）。押す前に止める
function tsMouseOutside(e) {
  const pop = document.getElementById("ts-timepop");
  if (!pop || e.button !== 0 || pop.contains(e.target)) return;
  e.stopPropagation();
  tsCloseTimePopup();
}

function tsMouseInit() {
  const ov = document.getElementById("ts-overlay");
  const ax = document.getElementById("ts-axis");
  ov.addEventListener("mousedown", tsOnDown);
  ov.addEventListener("dblclick", tsOnDbl);
  ax.addEventListener("mousedown", tsOnAxisDown);
  ov.addEventListener("wheel", tsOnWheel, { passive: false });
  ax.addEventListener("wheel", tsOnWheel, { passive: false });
  window.addEventListener("mousedown", tsMouseOutside, true);
  window.addEventListener("mousemove", tsOnMove);
  window.addEventListener("mouseup", tsOnUp);
  window.addEventListener("keydown", (e) => {
    if (e.key === " " && !tsIsEditing(e.target)) {
      tsSpace = true;
      e.preventDefault();
    }
  });
  window.addEventListener("keyup", (e) => {
    if (e.key === " ") tsSpace = false;
  });
  window.addEventListener("blur", () => {
    tsSpace = false;
    tsDrag = null;
    tsMouseCursor();
  });
  window.TS.ui.addLayer({ id: "drag", draw: tsMouseLayer });
  window.TS.on("mode", tsMouseCursor);
  tsMouseCursor();
}

// app.js の tsBoot は触らない。読み込みのあと、画面の要素が揃ったときに付ける
if (typeof window !== "undefined") {
  window.addEventListener("DOMContentLoaded", () => tsMouseInit());
}

if (typeof module !== "undefined") {
  module.exports = { tsMouseDraft };
}
