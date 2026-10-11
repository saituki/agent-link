"use strict";
// 波形のレーン・イベントの重ね描き・既定のキー（共有契約 C10）。マウスの操作は mouse.js。
// 純関数（DOM を使わない計算）は末尾で node のテストに渡す。

const tsGutterW = 120; // 左の欄（label・縦軸の目盛り）の幅（CSS px）
const tsAxisH = 24; // 時間軸の高さ（#ts-axis の上の部分）
const tsNavH = 14; // 記録全体の帯（#ts-axis の下の部分。高さ 8px と余白）
const tsLaneMinH = 24;
const tsHitPx = 4; // ラインの当たり判定の幅（CSS px）
const tsDefaultColor = "#64748b";
let tsGain = 1; // 全体の倍率（tsGainSteps の段）
let tsGainDs = null; // 倍率を読んだデータセット（保存の鍵）
let tsFetchTimer = null;
let tsFetchKey = "";
let tsFetchCtl = null;
let tsFetchSeq = 0;
let tsWave = null; // {ds, rec, data, scales, reqAt}
let tsCss = { w: 0, h: 0 };
let tsCol = null; // 配色（CSS の変数を tsTheme が読む）

// ---- 純関数（DOM を使わない。node のテストが読む） ----

// 表示中の変数ごとのレーン。高さは 高さ ÷ 変数の数（最小 minH）。時間軸は含まない
function tsLaneLayout(nvars, height, minH) {
  if (nvars <= 0) return { laneH: 0, rows: [], total: 0 };
  const laneH = Math.max(minH, Math.floor(height / nvars));
  const rows = [];
  for (let i = 0; i < nvars; i++) rows.push({ y0: i * laneH, y1: (i + 1) * laneH });
  return { laneH, rows, total: nvars * laneH };
}

// 1% 点と 99% 点で上下を決める。全部 null なら [-1, 1]
function tsAutoRange(values) {
  const v = values.filter((x) => x !== null && x !== undefined && Number.isFinite(x)).sort((a, b) => a - b);
  if (!v.length) return [-1, 1];
  const q = (p) => v[Math.min(v.length - 1, Math.floor(p * (v.length - 1)))];
  let lo = q(0.01);
  let hi = q(0.99);
  if (hi - lo < 1e-12) {
    lo -= 1;
    hi += 1;
  }
  return [lo, hi];
}

// M4 の列を点の列にする。列 i の last と列 i+1 の first を線でつなぐ。null の列は飛ばす
// toX は列の番号を受け、toY は値を受ける。move が true の点から線を新しく始める
function tsM4Path(cols, toX, toY) {
  const out = [];
  let started = false;
  for (let i = 0; i < cols.first.length; i++) {
    const f = cols.first[i];
    if (f === null || f === undefined) {
      started = false;
      continue;
    }
    const pick = (a) => (a === null || a === undefined ? f : a);
    const x = toX(i);
    out.push({ x, y: toY(f), move: !started });
    started = true;
    out.push({ x, y: toY(pick(cols.min[i])), move: false });
    out.push({ x, y: toY(pick(cols.max[i])), move: false });
    out.push({ x, y: toY(pick(cols.last[i])), move: false });
  }
  return out;
}

// raw（1 サンプルずつ）の折れ線。null で線を切る
function tsRawPath(y, start, dt, toX, toY) {
  const out = [];
  let started = false;
  for (let k = 0; k < y.length; k++) {
    const v = y[k];
    if (v === null || v === undefined) {
      started = false;
      continue;
    }
    out.push({ x: toX(start + k * dt), y: toY(v), move: !started });
    started = true;
  }
  return out;
}

// クリックの位置のイベント。ラインは許容幅 tolSec の中で一番近いもの、無ければエリアの中で一番狭いもの
function tsHitEvent(events, t, laneVar, tolSec) {
  const onLane = (ev) => !ev.vars || ev.vars.length === 0 || ev.vars.includes(laneVar);
  let best = null;
  let bestD = Infinity;
  for (const ev of events) {
    if (ev.type !== "line" || !onLane(ev)) continue;
    const d = Math.abs(ev.t - t);
    if (d <= tolSec && d < bestD) {
      best = ev;
      bestD = d;
    }
  }
  if (best) return best;
  let bestSpan = Infinity;
  for (const ev of events) {
    if (ev.type !== "area" || !onLane(ev)) continue;
    if (t >= ev.t0 && t <= ev.t1 && ev.t1 - ev.t0 < bestSpan) {
      best = ev;
      bestSpan = ev.t1 - ev.t0;
    }
  }
  return best;
}

// ドラッグの範囲を時間と変数にする。レーンに触れなければ中心が一番近いレーン
function tsSelectionFromDrag(x0, x1, y0, y1, frame) {
  const a = Math.min(frame.toT(x0), frame.toT(x1));
  const b = Math.max(frame.toT(x0), frame.toT(x1));
  const t0 = Math.max(frame.t0, a);
  const t1 = Math.min(frame.t1, b);
  const ya = Math.min(y0, y1);
  const yb = Math.max(y0, y1);
  let vars = frame.lanes.filter((l) => l.y1 > ya && l.y0 < yb).map((l) => l.var);
  if (!vars.length && frame.lanes.length) {
    const c = (ya + yb) / 2;
    let near = frame.lanes[0];
    for (const l of frame.lanes) {
      if (Math.abs((l.y0 + l.y1) / 2 - c) < Math.abs((near.y0 + near.y1) / 2 - c)) near = l;
    }
    vars = [near.var];
  }
  return { t0, t1, vars };
}

// #rrggbb に透明度を付ける。形が違えば灰色
function tsRgba(hex, a) {
  const m = /^#([0-9a-fA-F]{6})$/.exec(hex || "");
  if (!m) return `rgba(100,116,139,${a})`;
  const n = parseInt(m[1], 16);
  return `rgba(${(n >> 16) & 255},${(n >> 8) & 255},${n & 255},${a})`;
}

// 縦軸の値の書き方。有効数字 3 桁。大きすぎる・小さすぎる値は指数表記
function tsFmtVal(v) {
  if (v === 0) return "0";
  const a = Math.abs(v);
  if (a >= 1e4 || a < 1e-3) return v.toExponential(2).replace(/\.?0+e/, "e");
  return String(+v.toPrecision(3));
}

// レーンの上端と下端の値の範囲で、目盛りを maxTicks 個前後
function tsValueTicks(lane, maxTicks) {
  const lo = lane.toV(lane.y1 - 2);
  const hi = lane.toV(lane.y0 + 2);
  return tsNiceTicks(Math.min(lo, hi), Math.max(lo, hi), maxTicks);
}

// 倍率の段（全体と ch ごとで共通）
const tsGainSteps = [0.1, 0.2, 0.5, 1, 2, 5, 10];

// 段として読める値（有限で正の数）。読めなければ 1
function tsGainNum(v) {
  return typeof v === "number" && Number.isFinite(v) && v > 0 ? v : 1;
}

// 倍率を段で 1 つ動かす（dir が正で上げ、負で下げる）。段に無い値は一番近い段から動く。端で止まる
function tsGainStep(cur, dir) {
  const c = tsGainNum(cur);
  let best = 0;
  tsGainSteps.forEach((s, i) => {
    if (Math.abs(Math.log(c / s)) < Math.abs(Math.log(c / tsGainSteps[best]))) best = i;
  });
  const j = Math.min(tsGainSteps.length - 1, Math.max(0, best + (dir > 0 ? 1 : -1)));
  return tsGainSteps[j];
}

// 保存の文字（{"all": 段}）を読む。読めない値は 1 として扱う。古い保存の "ch" は読み捨てる
function tsGainParse(text) {
  const out = { all: 1 };
  let o = null;
  try {
    o = JSON.parse(text);
  } catch {
    return out;
  }
  if (!o || typeof o !== "object") return out;
  out.all = tsGainOf(o.all);
  return out;
}

// 段の値に直す（段に無ければ 1）
function tsGainOf(v) {
  return tsGainSteps.includes(v) ? v : 1;
}

// ---- ここから画面（ブラウザだけで動く） ----

function tsPlotPx() {
  const lanes = document.getElementById("ts-lanes");
  return Math.max(1, Math.min(8192, Math.round((lanes ? lanes.clientWidth : 0) - tsGutterW)));
}

function tsFmtSec(t) {
  return `${+t.toPrecision(8)} s`;
}

function tsCatColor(schema, name) {
  const c = (schema.categories || []).find((x) => x.name === name);
  return c && c.color ? c.color : tsDefaultColor;
}

// 今の記録・表示中のカテゴリのイベント
function tsVisibleEvents() {
  const st = window.TS.state;
  return st.events.filter(
    (ev) => (!ev.record || ev.record === st.record) && !st.hiddenCategories.has(ev.category),
  );
}

function tsCurrentWave() {
  const st = window.TS.state;
  return tsWave && tsWave.ds === st.dataset && tsWave.rec === st.record ? tsWave : null;
}

// 変数ごとの上下の中心と幅（1% 点・99% 点）
function tsScales(data) {
  const out = {};
  Object.keys(data.vars).forEach((name) => {
    const v = data.vars[name];
    const vals = v.mode === "m4" ? v.min.concat(v.max) : v.y;
    const [lo, hi] = tsAutoRange(vals);
    out[name] = { mid: (lo + hi) / 2, half: (hi - lo) / 2 || 1 };
  });
  return out;
}

// 全体の倍率の保存は データセットごと（記録をまたいで同じ倍率）
function tsGainSave() {
  if (!tsGainDs) return;
  try {
    localStorage.setItem("ts_gain:" + tsGainDs, JSON.stringify({ all: tsGain }));
  } catch {
    // 保存できなくても画面は動く
  }
}

// データセットが変わったら、そのデータセットの保存を読む
function tsGainSync() {
  const ds = window.TS.state.dataset;
  if (!ds || ds === tsGainDs) return;
  tsGainDs = ds;
  let text = null;
  try {
    text = localStorage.getItem("ts_gain:" + ds);
  } catch {
    text = null;
  }
  tsGain = tsGainParse(text).all;
  window.TS.emit("gain");
}

function tsGainSetAll(v) {
  tsGain = tsGainOf(v);
  tsGainSave();
  window.TS.emit("gain");
  tsDrawAll();
}

// 配色は CSS の変数から読む（canvas も同じ色にする）。起動時と resize のときに読み直す
function tsTheme() {
  const cs = getComputedStyle(document.documentElement);
  const v = (name) => cs.getPropertyValue(name).trim();
  tsCol = {
    bg: v("--ts-bg"),
    panel: v("--ts-panel"),
    border: v("--ts-border"),
    text: v("--ts-text"),
    muted: v("--ts-muted"),
    accent: v("--ts-accent"),
    grid: v("--ts-grid"),
    wave: v("--ts-wave"),
  };
}

// 時間の目盛り（秒）。幅に合わせて数を決める
function tsTimeTicks(frame) {
  return tsNiceTicks(frame.t0, frame.t1, Math.max(2, Math.floor(frame.plotW / 100)));
}

function tsLanesFrame() {
  const st = window.TS.state;
  const lanesEl = document.getElementById("ts-lanes");
  const w = lanesEl ? lanesEl.clientWidth : 0;
  const h = lanesEl ? lanesEl.clientHeight : 0;
  const vars = st.vars || [];
  const lay = tsLaneLayout(vars.length, h, tsLaneMinH);
  const plotW = Math.max(1, w - tsGutterW);
  const span = st.t1 - st.t0 || 1;
  const wave = tsCurrentWave();
  const chOf = new Map((st.channels || []).map((c) => [c.name, c]));
  const lanes = vars.map((name, i) => {
    const row = lay.rows[i];
    const sc = wave && wave.scales[name] ? wave.scales[name] : { mid: 0, half: 1 };
    const ch = chOf.get(name) || { gain: 1 };
    const mid = (row.y0 + row.y1) / 2;
    const amp = (row.y1 - row.y0) / 2 - 4;
    const k = tsGain * tsGainNum(ch.gain) * amp; // 全体の倍率 × ch の倍率 × 半分の高さ
    return {
      var: name,
      kind: "wave",
      y0: row.y0,
      y1: row.y1,
      ch,
      toY: (v) => mid - ((v - sc.mid) / sc.half) * k,
      toV: (y) => sc.mid + ((mid - y) / k) * sc.half,
    };
  });
  return {
    t0: st.t0,
    t1: st.t1,
    width: w,
    height: lay.total,
    plotW,
    axis: { y0: 0, h: tsAxisH }, // #ts-axis の中の座標
    nav: { y0: tsAxisH, h: tsNavH },
    lanes,
    toX: (t) => tsGutterW + ((t - st.t0) / span) * plotW,
    toT: (x) => st.t0 + ((x - tsGutterW) / plotW) * span,
  };
}

function tsLaneAt(frame, y) {
  return frame.lanes.find((l) => y >= l.y0 && y < l.y1) || null;
}

function tsWavePoints(wave, data, frame, lane) {
  if (data.mode === "m4") {
    const n = data.first.length;
    const colX = (i) => frame.toX(wave.data.t0 + ((i + 0.5) * (wave.data.t1 - wave.data.t0)) / n);
    return tsM4Path({ first: data.first, min: data.min, max: data.max, last: data.last }, colX, lane.toY);
  }
  return tsRawPath(data.y, data.start, data.dt, frame.toX, lane.toY);
}

// canvas の実寸と CSS の大きさをそろえる（寸法が変わったときだけ実寸を変える）
function tsSizeCanvas(c, w, h, dpr) {
  if (c.width !== Math.round(w * dpr) || c.height !== Math.round(h * dpr)) {
    c.width = Math.round(w * dpr);
    c.height = Math.round(h * dpr);
  }
  c.style.width = w + "px";
  c.style.height = h + "px";
}

function tsDrawAll() {
  const lanesEl = document.getElementById("ts-lanes");
  const main = document.getElementById("ts-main");
  if (!lanesEl || !main) return;
  if (!tsCol) tsTheme();
  const frame = tsLanesFrame();
  const w = lanesEl.clientWidth;
  const h = Math.max(lanesEl.clientHeight, frame.height);
  tsCss = { w, h };
  const dpr = window.devicePixelRatio || 1;
  tsSizeCanvas(document.getElementById("ts-canvas"), w, h, dpr);
  tsSizeCanvas(document.getElementById("ts-overlay"), w, h, dpr);
  const ctx = document.getElementById("ts-canvas").getContext("2d");
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  ctx.clearRect(0, 0, w, h);
  tsPaintLanes(ctx, frame, w, h);
  // 時間軸と記録全体の帯は下に固定（#ts-axis）
  const axisW = main.clientWidth;
  const axisH = tsAxisH + tsNavH;
  const ac = document.getElementById("ts-axis");
  tsSizeCanvas(ac, axisW, axisH, dpr);
  const actx = ac.getContext("2d");
  actx.setTransform(dpr, 0, 0, dpr, 0, 0);
  actx.clearRect(0, 0, axisW, axisH);
  tsPaintAxis(actx, frame, axisW, axisH);
  tsDrawOverlay();
}

function tsPaintLanes(ctx, frame, w, h) {
  const c = tsCol;
  const wave = tsCurrentWave();
  ctx.fillStyle = c.bg;
  ctx.fillRect(0, 0, w, h);
  ctx.fillStyle = c.panel;
  ctx.fillRect(0, 0, tsGutterW, h);
  // 時間の目盛りに合わせた縦の薄い線
  ctx.fillStyle = c.grid;
  ctx.globalAlpha = 0.35;
  tsTimeTicks(frame).forEach((t) => ctx.fillRect(Math.round(frame.toX(t)), 0, 1, frame.height));
  ctx.globalAlpha = 1;
  frame.lanes.forEach((lane) => {
    const laneH = lane.y1 - lane.y0;
    const color = lane.ch.drawColor || c.wave;
    ctx.fillStyle = c.border;
    ctx.fillRect(0, lane.y1 - 1, w, 1);
    ctx.fillStyle = color;
    ctx.fillRect(0, lane.y0, 4, laneH);
    tsPaintLaneAxis(ctx, lane, w, c);
    tsPaintLaneLabel(ctx, lane, c);
    const data = wave && wave.data.vars[lane.var];
    if (!data) return;
    ctx.save();
    ctx.beginPath();
    ctx.rect(tsGutterW, lane.y0, frame.plotW, laneH);
    ctx.clip();
    ctx.strokeStyle = color;
    ctx.lineWidth = 1;
    ctx.beginPath();
    tsWavePoints(wave, data, frame, lane).forEach((p) => (p.move ? ctx.moveTo(p.x, p.y) : ctx.lineTo(p.x, p.y)));
    ctx.stroke();
    ctx.restore();
  });
  ctx.fillStyle = c.border;
  ctx.fillRect(tsGutterW - 1, 0, 1, frame.height);
}

// レーンの縦軸（値の目盛り）。低いレーンは中央の 1 本だけ（値は書かない）
function tsPaintLaneAxis(ctx, lane, w, c) {
  const laneH = lane.y1 - lane.y0;
  const small = laneH < 40;
  const pts = small
    ? [{ v: null, y: (lane.y0 + lane.y1) / 2 }]
    : tsValueTicks(lane, Math.max(2, Math.floor(laneH / 28))).map((v) => ({ v, y: lane.toY(v) }));
  ctx.font = "11px sans-serif";
  ctx.textAlign = "right";
  ctx.textBaseline = "middle";
  pts.forEach((p) => {
    if (p.y < lane.y0 || p.y > lane.y1) return;
    const y = Math.round(p.y);
    ctx.globalAlpha = 0.35;
    ctx.fillStyle = c.grid;
    ctx.fillRect(tsGutterW, y, w - tsGutterW, 1);
    ctx.globalAlpha = 1;
    ctx.fillStyle = c.muted;
    ctx.fillRect(tsGutterW, y, 4, 1);
    if (p.v !== null) ctx.fillText(tsFmtVal(p.v), tsGutterW - 4, p.y);
  });
  ctx.textAlign = "left";
}

// レーンの label（倍率が 1 でなければ ×2 のように付ける）と単位
function tsPaintLaneLabel(ctx, lane, c) {
  const laneH = lane.y1 - lane.y0;
  const mid = (lane.y0 + lane.y1) / 2;
  const two = laneH >= 40; // 単位の行を出せる高さ
  const gain = tsGainNum(lane.ch.gain);
  const label = (lane.ch.label || lane.var) + (gain === 1 ? "" : ` ×${gain}`);
  ctx.font = "13px sans-serif";
  ctx.fillStyle = c.text;
  ctx.textBaseline = "middle";
  ctx.fillText(label, 10, two ? mid - 7 : mid, tsGutterW - 60);
  if (two && lane.ch.unit) {
    ctx.font = "11px sans-serif";
    ctx.fillStyle = c.muted;
    ctx.fillText(lane.ch.unit, 10, mid + 8, tsGutterW - 60);
  }
}

// 時間軸（上の 24px）と記録全体の帯（下の 14px）。#ts-axis に描く
function tsPaintAxis(ctx, frame, w, h) {
  const c = tsCol;
  const st = window.TS.state;
  ctx.fillStyle = c.panel;
  ctx.fillRect(0, 0, w, h);
  ctx.fillStyle = c.border;
  ctx.fillRect(0, 0, w, 1);
  ctx.fillRect(tsGutterW - 1, 0, 1, h);
  ctx.font = "13px sans-serif";
  ctx.textBaseline = "top";
  ctx.fillStyle = c.muted;
  tsTimeTicks(frame).forEach((t) => {
    const x = frame.toX(t);
    if (x < tsGutterW - 1 || x > w + 1) return;
    ctx.fillRect(Math.round(x), frame.axis.y0, 1, 6);
    ctx.fillText(tsFmtSec(t), x + 3, frame.axis.y0 + 6);
  });
  // 記録全体の中の今の範囲（押すと移動）
  const dur = tsDuration(st.meta);
  if (dur > 0) {
    const y = frame.nav.y0 + 3;
    const x0 = tsGutterW + (st.t0 / dur) * frame.plotW;
    const x1 = tsGutterW + (st.t1 / dur) * frame.plotW;
    ctx.fillStyle = c.grid;
    ctx.fillRect(tsGutterW, y, frame.plotW, 8);
    ctx.fillStyle = tsRgba(c.accent, 0.45);
    ctx.fillRect(x0, y, Math.max(2, x1 - x0), 8);
  }
}

function tsDrawOverlay() {
  const ov = document.getElementById("ts-overlay");
  if (!ov || !tsCss.w) return;
  const frame = tsLanesFrame();
  const dpr = window.devicePixelRatio || 1;
  const ctx = ov.getContext("2d");
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  ctx.clearRect(0, 0, tsCss.w, tsCss.h);
  tsPaintEvents(ctx, frame);
  const st = window.TS.state;
  const acc = tsCol.accent;
  if (st.selection) tsPaintRange(ctx, frame, st.selection.t0, st.selection.t1, st.selection.vars, tsRgba(acc, 0.2), acc);
  tsRunLayers(ctx, frame);
}

function tsPaintRange(ctx, frame, t0, t1, vars, fill, stroke) {
  const x0 = frame.toX(t0);
  const x1 = frame.toX(t1);
  frame.lanes
    .filter((l) => vars.includes(l.var))
    .forEach((l) => {
      ctx.fillStyle = fill;
      ctx.fillRect(x0, l.y0, x1 - x0, l.y1 - l.y0);
      ctx.strokeStyle = stroke;
      ctx.lineWidth = 1.5;
      ctx.strokeRect(x0 + 0.5, l.y0 + 0.5, x1 - x0 - 1, l.y1 - l.y0 - 1);
    });
}

function tsPaintEvents(ctx, frame) {
  const st = window.TS.state;
  ctx.font = "13px sans-serif";
  ctx.textBaseline = "top";
  tsVisibleEvents().forEach((ev) => {
    const color = tsCatColor(st.schema, ev.category);
    const sel = st.selectedEvents.includes(ev.id);
    const lanes = frame.lanes.filter((l) => !ev.vars || ev.vars.length === 0 || ev.vars.includes(l.var));
    if (!lanes.length) return;
    ctx.fillStyle = color;
    if (ev.type === "area") {
      if (ev.t1 < frame.t0 || ev.t0 > frame.t1) return;
      const x0 = frame.toX(ev.t0);
      const x1 = frame.toX(ev.t1);
      lanes.forEach((l) => {
        ctx.fillStyle = tsRgba(color, 0.22);
        ctx.fillRect(x0, l.y0 + 1, x1 - x0, l.y1 - l.y0 - 2);
        ctx.strokeStyle = color;
        ctx.lineWidth = sel ? 2 : 1;
        ctx.strokeRect(x0 + 0.5, l.y0 + 1.5, x1 - x0 - 1, l.y1 - l.y0 - 3);
        if (sel) {
          // 選んだエリアは外側にアクセントの輪を付ける
          ctx.strokeStyle = tsCol.accent;
          ctx.lineWidth = 2;
          ctx.strokeRect(x0 - 2, l.y0 + 0.5, x1 - x0 + 4, l.y1 - l.y0 - 1);
        }
      });
      ctx.fillStyle = color;
      ctx.fillText(ev.category || "", x0 + 3, lanes[0].y0 + 2);
    } else {
      if (ev.t < frame.t0 || ev.t > frame.t1) return;
      const x = frame.toX(ev.t);
      ctx.strokeStyle = color;
      ctx.lineWidth = sel ? 3 : 2;
      lanes.forEach((l) => {
        ctx.beginPath();
        ctx.moveTo(x, l.y0);
        ctx.lineTo(x, l.y1);
        ctx.stroke();
      });
      ctx.fillStyle = color;
      ctx.fillText(ev.category || "", x + 4, lanes[0].y0 + 2);
    }
  });
}

function tsTolSec() {
  const frame = tsLanesFrame();
  return (tsHitPx * (frame.t1 - frame.t0)) / frame.plotW;
}

function tsSelectEvents(ids) {
  window.TS.setView({ selectedEvents: ids });
  window.TS.emit("select", ids);
}

// 取り直しは 80ms 待ってから。前の要求は取り消す
function tsLanesScheduleFetch() {
  const st = window.TS.state;
  const key = [st.dataset, st.record, st.t0, st.t1, (st.vars || []).join(","), tsPlotPx()].join("|");
  if (key === tsFetchKey) return;
  tsFetchKey = key;
  clearTimeout(tsFetchTimer);
  if (tsFetchCtl) tsFetchCtl.abort();
  if (!st.meta || !(st.vars || []).length) {
    tsWave = null;
    tsDrawAll();
    return;
  }
  tsFetchTimer = setTimeout(tsLanesFetch, 80);
}

async function tsLanesFetch() {
  const st = window.TS.state;
  if (!st.dataset || !st.record || !st.meta || !st.vars.length) return;
  if (tsFetchCtl) tsFetchCtl.abort();
  const ctl = new AbortController();
  tsFetchCtl = ctl;
  const seq = ++tsFetchSeq;
  const ds = st.dataset;
  const rec = st.record;
  const url =
    `/api/datasets/${tsEnc(ds)}/records/${tsEnc(rec)}/m4?vars=${st.vars.map(tsEnc).join(",")}` +
    `&t0=${st.t0}&t1=${st.t1}&px=${tsPlotPx()}`;
  const reqAt = performance.now();
  try {
    const data = await window.TS.api(url, { signal: ctl.signal });
    if (seq !== tsFetchSeq) return;
    tsWave = { ds, rec, data, scales: tsScales(data), reqAt };
    tsDrawAll();
    console.info(`sai-ts 波形: 要求から描き終わりまで ${Math.round(performance.now() - reqAt)} ms`);
  } catch (e) {
    if (e.name === "AbortError") return;
    window.TS.ui.toast("波形を取れません: " + e.message);
  }
}

async function tsKeyDelete() {
  const st = window.TS.state;
  const ids = st.selectedEvents.slice();
  if (!ids.length || !st.dataset) return;
  if (!window.confirm(`${ids.length} 件のイベントを消しますか？`)) return;
  for (const id of ids) {
    await window.TS.api(`/api/datasets/${tsEnc(st.dataset)}/events/${tsEnc(id)}`, { method: "DELETE" }).catch((e) =>
      window.TS.ui.toast("消せません: " + e.message),
    );
  }
  window.TS.setView({ selectedEvents: [] });
  await window.TS.reloadEvents();
}

function tsLanesInstallKeys() {
  const ui = window.TS.ui;
  const zoom = (factor) => {
    const st = window.TS.state;
    if (!st.meta) return;
    const mid = (st.t0 + st.t1) / 2;
    const r = tsZoomAt(st.t0, st.t1, mid, factor, tsDuration(st.meta), (tsMinSamples / st.meta.srate) || 0);
    window.TS.setView({ t0: r.t0, t1: r.t1 });
  };
  const pan = (k) => {
    const st = window.TS.state;
    if (!st.meta) return;
    const r = tsPan(st.t0, st.t1, k * (st.t1 - st.t0), tsDuration(st.meta));
    window.TS.setView({ t0: r.t0, t1: r.t1 });
  };
  ui.addKey({ key: "ArrowLeft", desc: "左へ移動（Shift で 1 画面）", handler: (e) => pan(e.shiftKey ? -1 : -0.5) });
  ui.addKey({ key: "ArrowRight", desc: "右へ移動（Shift で 1 画面）", handler: (e) => pan(e.shiftKey ? 1 : 0.5) });
  ui.addKey({ key: "+", desc: "拡大", handler: () => zoom(0.8) });
  ui.addKey({ key: "=", desc: "拡大", handler: () => zoom(0.8) });
  ui.addKey({ key: "-", desc: "縮小", handler: () => zoom(1.25) });
  ui.addKey({
    key: "0",
    desc: "記録全体",
    handler: () => {
      const st = window.TS.state;
      if (st.meta) window.TS.setView({ t0: 0, t1: tsDuration(st.meta) });
    },
  });
  ui.addKey({ key: "ArrowUp", desc: "全体の倍率を 1 段上げる", handler: () => tsGainSetAll(tsGainStep(tsGain, 1)) });
  ui.addKey({ key: "ArrowDown", desc: "全体の倍率を 1 段下げる", handler: () => tsGainSetAll(tsGainStep(tsGain, -1)) });
  ui.addKey({
    key: "Escape",
    desc: "選択を外す",
    handler: () => window.TS.setView({ selection: null, selectedEvents: [] }),
  });
  ui.addKey({ key: "Delete", desc: "選んだイベントを消す", handler: tsKeyDelete });
  ui.addKey({ key: "Backspace", desc: "選んだイベントを消す", handler: tsKeyDelete });
}

function tsLanesInit() {
  // マウスの操作は mouse.js が受ける
  tsTheme();
  // 欄の大きさは app.js の ResizeObserver が知らせる（window の resize には頼らない）
  window.TS.on("resize", () => {
    tsTheme();
    tsDrawAll();
    tsLanesScheduleFetch();
  });
  window.TS.on("channels", tsDrawAll);
  window.TS.on("state", tsGainSync);
  window.TS.on("state", tsLanesScheduleFetch);
  tsLanesInstallKeys();
}

if (typeof module !== "undefined") {
  module.exports = {
    tsLaneLayout,
    tsAutoRange,
    tsM4Path,
    tsRawPath,
    tsHitEvent,
    tsSelectionFromDrag,
    tsRgba,
    tsFmtVal,
    tsValueTicks,
    tsGainSteps,
    tsGainStep,
    tsGainParse,
  };
}
