"use strict";
// 指標の時系列のグラフ（実装計画 U07・共有契約 C-03 / C-07）。
//
// run の詳細と「run の比較」の両方で使う。グラフは素の SVG を手描きする（外部ライブラリは
// 入れない）。目盛りは ui.js の saiNiceScale、目盛りに書く文字の規則も saiAxisValue に
// そろえる。色は CSS 変数（--series-* / --chart-grid / --chart-axis）。
//
// 上の純関数（mlfSeriesFromMetrics / mlfChartScale / mlfPathD / mlfChartNum / mlfHasPoints）は
// DOM を使わない。末尾で node に渡す（tests/test_mlfchart.js）。描くのは mlfLineChart。
//
// 間引きは MLflow のサーバに任せる。get-history-bulk-interval は最初と最後を残して均等に
// 間引くので、画面では間引かない。
//
// 一番外側の名前の頭は mlf（mlflow.js と同じ頭。他の画面では使っていない）。

// --- 定数 -------------------------------------------------------------

// グラフの横幅の既定（親の幅が読めないとき）
const MLF_CHART_W = 320;
// グラフの高さ
const MLF_CHART_H = 160;
// 系列の色は --series-1〜6 の 6 色
const MLF_CHART_SERIES = 6;
// 0 と比べるための、第二に小さい数
const MLF_CHART_EPS = 1e-9;

// ============================================================================
// 純関数（DOM を使わない。ここだけを tests/test_mlfchart.js で確かめる）
// ============================================================================

/**
 * MLflow の指標の点（`[{key, value, timestamp, step, run_id?}]`）を run ごとに分ける。
 *   xKind が "step" のとき x は step、"time" のときその run の最初の
 *   timestamp からの秒数。
 * NaN と ±Infinity の値は捨てる。x の昇順に並べる。
 * 返り値は `[{runId, points: [{x, y, step, t}]}]`（run_id が無い物は空文字）。
 */
function mlfSeriesFromMetrics(metrics, xKind) {
  const byRun = new Map();
  const time = xKind === "time";
  for (const one of Array.isArray(metrics) ? metrics : []) {
    if (!one) continue;
    if (one.value == null || one.value === "") continue; // 値の無い点は捨てる
    const y = Number(one.value);
    if (!Number.isFinite(y)) continue; // NaN と ±Infinity は捨てる
    const step = Number(one.step);
    const t = Number(one.timestamp);
    const runId = one.run_id == null ? "" : String(one.run_id);
    if (!byRun.has(runId)) byRun.set(runId, []);
    byRun.get(runId).push({ x: 0, y, step: Number.isFinite(step) ? step : null, t: Number.isFinite(t) ? t : null });
  }
  const out = [];
  for (const [runId, points] of byRun) {
    // 経過時間の 0 は、その run で最初に記録した点
    let first = Infinity;
    if (time) {
      for (const p of points) if (p.t != null && p.t < first) first = p.t;
    }
    for (const p of points) {
      p.x = !time ? (p.step == null ? 0 : p.step) : (p.t == null || !Number.isFinite(first) ? 0 : (p.t - first) / 1000);
    }
    points.sort((a, b) => a.x - b.x);
    out.push({ runId, points });
  }
  return out;
}

/**
 * series を画素に写す目盛りを作る。
 *   series: `[{runId, points: [{x, y}]}]`（mlfSeriesFromMetrics の返り値）
 *   width / height: SVG の viewBox の大きさ
 * 点が 1 つだけのときや全部同じ値のとき、NaN を出さない。
 */
function mlfChartScale(series, width, height) {
  const W = Math.max(80, Number(width) || MLF_CHART_W);
  const H = Math.max(80, Number(height) || MLF_CHART_H);
  const pad = { l: 46, r: 12, t: 10, b: 22 };
  const innerW = Math.max(1, W - pad.l - pad.r);
  const innerH = Math.max(1, H - pad.t - pad.b);

  let xMaxData = 0;
  let yMinData = 0;
  let yMaxData = 0;
  let count = 0;
  for (const one of Array.isArray(series) ? series : []) {
    if (!one || !Array.isArray(one.points)) continue;
    for (const p of one.points) {
      if (!p) continue;
      const x = Number(p.x);
      const y = Number(p.y);
      if (!Number.isFinite(x) || !Number.isFinite(y)) continue;
      count += 1;
      if (x > xMaxData) xMaxData = x;
      if (y > yMaxData) yMaxData = y;
      if (y < yMinData) yMinData = y;
    }
  }
  if (count === 0) {
    xMaxData = 1;
    yMaxData = 1;
  }
  // x は 0 起点（step も経過時間も 0 起点）
  // y は 0 を通す。負の値があれば下にも伸ばす
  const yScale = saiNiceScale(Math.max(yMaxData, -yMinData, 0), 3, 0);
  const yMax = yScale.max;
  const yMin = yMinData < 0 ? -yScale.max : 0;

  // 目盛りの本数は幅で決める（390px でも文字が重ならないように減らす）
  const xCount = W < 360 ? 3 : W < 720 ? 4 : 6;
  // x も切りりのよい数に切る（6.333 のような数字を出さない）
  const xScale = saiNiceScale(xMaxData > 0 ? xMaxData : 1, xCount, 0);
  const xMax = xScale.max;
  const xTicks = xScale.values.map((v) => Number(v.toFixed(6)));
  const yTicks = [];
  if (yMin < 0) {
    for (const v of yScale.values) yTicks.push(-v);
    for (const v of yScale.values) yTicks.push(v);
  } else {
    for (const v of yScale.values) yTicks.push(v);
  }
  yTicks.sort((a, b) => a - b);

  const xOf = (v) => {
    const n = Number(v);
    if (!Number.isFinite(n)) return pad.l;
    return pad.l + Math.max(0, Math.min(1, n / xMax)) * innerW;
  };
  const yOf = (v) => {
    const n = Number(v);
    if (!Number.isFinite(n)) return pad.t + innerH;
    const span = yMax - yMin;
    if (!Number.isFinite(span) || Math.abs(span) < MLF_CHART_EPS) return pad.t + innerH / 2;
    return pad.t + (1 - Math.max(0, Math.min(1, (n - yMin) / span))) * innerH;
  };
  return {
    width: W,
    height: H,
    pad,
    inner: { w: innerW, h: innerH },
    xMin: 0,
    xMax,
    yMin,
    yMax,
    xTicks,
    yTicks,
    xOf,
    yOf,
  };
}

/** 折れ線の `d` 属性の文字列（点が 1 つでも壊れない）。 */
function mlfPathD(points, scale) {
  if (!scale || typeof scale.xOf !== "function") return "";
  const parts = [];
  for (const p of Array.isArray(points) ? points : []) {
    if (!p) continue;
    const x = Number(p.x);
    const y = Number(p.y);
    if (!Number.isFinite(x) || !Number.isFinite(y)) continue; // 壊れた点は飛ばす
    parts.push(`${parts.length === 0 ? "M" : "L"} ${scale.xOf(x).toFixed(1)} ${scale.yOf(y).toFixed(1)}`);
  }
  return parts.join(" ");
}

/**
 * 指標の値と目盛りを画面に出す文字列にする。
 * 4 桁を超えると K / M にするのは saiAxisValue と同じ。
 */
function mlfChartNum(v) {
  const n = Number(v);
  if (!Number.isFinite(n)) return "0";
  const a = Math.abs(n);
  if (a >= 1e6) return `${mlfTrim1(n / 1e6)}M`;
  if (a >= 1e4) return `${mlfTrim1(n / 1e3)}K`;
  if (a > 0 && a < 1e-3) return n.toExponential(2);
  if (Number.isInteger(n)) return n.toLocaleString("ja-JP");
  return String(Number(n.toPrecision(4)));
}

/** 小数1位まで。整数なら .0 を付けない（1.5M / 2M）。 */
function mlfTrim1(x) {
  const r = Math.round(Number(x) * 10) / 10;
  return Number.isInteger(r) ? String(r) : r.toFixed(1);
}

/** 点が 2 つ以上ある series があるか（「最後の値だけ」なら false）。 */
function mlfHasPoints(series) {
  for (const one of Array.isArray(series) ? series : []) {
    if (one && Array.isArray(one.points) && one.points.length >= 2) return true;
  }
  return false;
}

// ============================================================================
// 描く（DOM を使う）
// ============================================================================

/** 系列の色（--series-1〜6 を順に使う）。 */
function mlfChartColor(i) {
  const n = Math.max(0, Math.floor(Number(i) || 0)) % MLF_CHART_SERIES;
  return `var(--series-${n + 1})`;
}

/** 横軸の値の日本語（step か経過時間か）。 */
function mlfXLabel(v, xKind) {
  if (xKind === "time") return `${mlfChartNum(v)} 秒`;
  return mlfChartNum(v);
}

/** 1 本の線・1 つの点を 1 行で読むための文。 */
function mlfPointOneText(series, p, xKind) {
  const name = (series && series.label) || "";
  const head = name ? `${name} ・ ` : "";
  const at = xKind === "time" ? ` 経過 ${mlfSecondsText(p.x)}` : ` step ${mlfChartNum(p.x)}`;
  return `${head}${mlfChartNum(p.y)} ・${at}`;
}

/** 秒数を「1 分 30 秒」のような日本語にする。 */
function mlfSecondsText(sec) {
  const n = Number(sec);
  if (!Number.isFinite(n) || n < 0) return "—";
  const s = Math.round(n);
  if (s < 60) return `${s} 秒`;
  const min = Math.floor(s / 60);
  const rest = s % 60;
  if (min < 60) return rest ? `${min} 分 ${rest} 秒` : `${min} 分`;
  const hour = Math.floor(min / 60);
  const restMin = min % 60;
  return restMin ? `${hour} 時間 ${restMin} 分` : `${hour} 時間`;
}

/** グラフの横幅（親の幅。読めなければ既定）。 */
function mlfChartWidth(container) {
  const w = (container && container.clientWidth) || 0;
  return Math.max(240, Math.round(w) || MLF_CHART_W);
}

/** 読み上げ用の 1 文（最大の点を必ず入れる）。 */
function mlfChartAria(series, xKind, title) {
  const xs = xKind === "time" ? "経過時間" : "step";
  let max = null;
  for (const one of series) {
    for (const p of one.points || []) {
      if (p && (!max || p.y > max.y)) max = p;
    }
  }
  const head = title ? `${title}の` : "";
  const total = series.reduce((sum, one) => sum + ((one && one.points) || []).length, 0);
  const tail = max ? ` 最大の値は ${mlfChartNum(max.y)}（${xs} ${mlfXLabel(max.x, xKind)}）。` : "";
  return `${head}${xs}のグラフ。${series.length} 本の線、点の総数は ${total}。${tail}`;
}

/**
 * 指標の折れ線グラフを container に描く。
 *   o.series: `[{runId, label, points: [{x, y, step, t}]}]`
 *   o.xKind:  "step" / "time"
 *   o.title:  指標の名前（読み上げに使う）
 * 点を押すと（タップしても）、その点の step・経過時間・値を 1 行で出す。
 */
function mlfLineChart(container, o) {
  const opts = o || {};
  if (!container) return;
  container.innerHTML = "";
  const series = (Array.isArray(opts.series) ? opts.series : []).filter(
    (one) => one && Array.isArray(one.points) && one.points.length > 0
  );
  const xKind = opts.xKind === "time" ? "time" : "step";
  const title = String(opts.title || "");
  if (series.length === 0) {
    container.appendChild(saiEl("p", "mlf-note tiny", "この指標の記録がありません。"));
    return;
  }

  const scale = mlfChartScale(series, mlfChartWidth(container), MLF_CHART_H);
  const wrap = saiEl("div", "mlf-chart");
  const svg = saiSvg("svg", {
    viewBox: `0 0 ${scale.width} ${scale.height}`,
    class: "mlf-chart-svg",
    role: "img",
    "aria-label": mlfChartAria(series, xKind, title),
  });

  // 縦の目盛りと格子線（0 のところだけ濃い線にする）
  for (const t of scale.yTicks) {
    const y = scale.yOf(t).toFixed(2);
    svg.appendChild(
      saiSvg("line", {
        x1: String(scale.pad.l),
        y1: y,
        x2: String(scale.width - scale.pad.r),
        y2: y,
        class: Math.abs(t) < MLF_CHART_EPS ? "mlf-chart-base" : "mlf-chart-grid",
      })
    );
    const label = saiSvg("text", {
      x: String(scale.pad.l - 5),
      y,
      dy: "4",
      class: "mlf-chart-tick",
      "text-anchor": "end",
    });
    label.textContent = mlfChartNum(t);
    svg.appendChild(label);
  }

  // 横の目盛り
  const lastX = scale.xTicks[scale.xTicks.length - 1];
  for (const t of scale.xTicks) {
    const text = saiSvg("text", {
      x: scale.xOf(t).toFixed(1),
      y: String(scale.height - 6),
      class: "mlf-chart-tick",
      "text-anchor": Math.abs(t - lastX) < MLF_CHART_EPS ? "end" : "middle",
    });
    text.textContent = mlfXLabel(t, xKind);
    svg.appendChild(text);
  }

  // 折れ線
  series.forEach((one, i) => {
    const list = one.points
      .map((p) => `${scale.xOf(p.x).toFixed(1)},${scale.yOf(p.y).toFixed(1)}`)
      .join(" ");
    if (!list) return;
    svg.appendChild(
      saiSvg("polyline", {
        points: list,
        fill: "none",
        stroke: mlfChartColor(i),
        "stroke-width": "2",
        "stroke-linejoin": "round",
        "stroke-linecap": "round",
      })
    );
  });

  // 点を押した位置の縦線と点（押すまでは隠す）
  const mark = saiSvg("g", { class: "mlf-chart-mark", opacity: "0" });
  const vline = saiSvg("line", {
    y1: String(scale.pad.t),
    y2: String(scale.height - scale.pad.b),
    class: "mlf-chart-cursor",
  });
  const dots = series.map((one, i) => saiSvg("circle", { r: "4", fill: mlfChartColor(i) }));
  mark.appendChild(vline);
  for (const dot of dots) mark.appendChild(dot);
  svg.appendChild(mark);

  // 点を押した位置を探す面（透明の長方形。点 1 つ 1 つごとに四角を置くより軽い）
  const hit = saiSvg("rect", {
    x: String(scale.pad.l - 4),
    y: String(scale.pad.t),
    width: String(scale.inner.w + 8),
    height: String(scale.inner.h),
    fill: "transparent",
    class: "mlf-chart-hit",
  });
  svg.appendChild(hit);

  const read = saiEl("p", "mlf-chart-read", "点を押すと値が出ます");
  wrap.append(svg, read);

  // 凡例（run の名前と色）
  const legend = saiEl("div", "mlf-chart-legend");
  series.forEach((one, i) => {
    const item = saiEl("span", "mlf-chart-leg");
    const swatch = saiEl("span", "mlf-chart-swatch");
    swatch.style.background = mlfChartColor(i);
    item.append(swatch, saiEl("span", "", one.label || one.runId || "—"));
    legend.appendChild(item);
  });
  wrap.appendChild(legend);
  container.appendChild(wrap);

  const show = (clientX) => {
    const box = svg.getBoundingClientRect();
    if (!box || !box.width) return;
    const viewX = ((clientX - box.left) / box.width) * scale.width;
    let best = null;
    series.forEach((one, si) => {
      for (let pi = 0; pi < one.points.length; pi += 1) {
        const d = Math.abs(scale.xOf(one.points[pi].x) - viewX);
        if (!best || d < best.d) best = { d, si, pi };
      }
    });
    if (!best) return;
    const picked = series[best.si].points[best.pi];
    vline.setAttribute("x1", scale.xOf(picked.x).toFixed(1));
    vline.setAttribute("x2", scale.xOf(picked.x).toFixed(1));
    dots.forEach((dot, i) => {
      if (i !== best.si) {
        dot.setAttribute("opacity", "0");
        return;
      }
      dot.setAttribute("cx", scale.xOf(picked.x).toFixed(1));
      dot.setAttribute("cy", scale.yOf(picked.y).toFixed(1));
      dot.setAttribute("opacity", "1");
    });
    mark.setAttribute("opacity", "1");
    read.textContent = mlfPointOneText(series[best.si], picked, xKind);
  };
  hit.addEventListener("pointerdown", (ev) => {
    ev.preventDefault();
    show(ev.clientX);
  });
  hit.addEventListener("mousemove", (ev) => {
    if (ev.buttons) show(ev.clientX);
  });
}

// --- node からの読み込み（tests/test_mlfchart.js） ---
if (typeof module !== "undefined" && module.exports) {
  module.exports = {
    MLF_CHART_W,
    MLF_CHART_H,
    MLF_CHART_SERIES,
    mlfSeriesFromMetrics,
    mlfChartScale,
    mlfPathD,
    mlfChartNum,
    mlfHasPoints,
    mlfChartColor,
    mlfXLabel,
    mlfPointOneText,
    mlfSecondsText,
    mlfLineChart,
  };
}
