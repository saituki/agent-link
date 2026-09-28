#!/usr/bin/env node
// 配色の土台を確かめる。style.css の :root（明るい画面）と
// 暗い画面の :root にある design token を読み、WCAG の相対輝度で
// コントラスト比を計算して、下の表どおりの下限を満たすかを調べる。
//
//   node apps/phone/tests/test_contrast.js
//
// 検査する token は必ず不透明な #rrggbb にする（rgba などは数えない）。
// U02〜U04 は下の TEXT / GRAPHICAL に組を足す。
// グラフの系列や Workflow の線のように新しい役を作ったら、ここにも足して
// 対比が足りないまま入らないようにする。

"use strict";

const fs = require("fs");
const path = require("path");
const assert = require("assert");

const STATIC = path.join(__dirname, "..", "src", "sai_phone", "static");
const CSS_PATH = path.join(STATIC, "style.css");

// --- 対比を測る相手の地色（明るい画面と暗い画面の同じ 3 つ） ---
const BGS = ["--bg", "--bg-soft", "--st-card"];

/** 検査する組を並べる。TEXT が 4.5:1、GRAPHICAL が 3:1 を求める。 */
const PAIRS = [];

/** 文字の役（小さい文字でも読めること。4.5:1）。 */
function TEXT(names) {
  for (const n of names) for (const bg of BGS) PAIRS.push([n, bg, 4.5, 1]);
}
/** 線・枠・図形の役（3:1）。 */
function GRAPHICAL(names) {
  for (const n of names) for (const bg of BGS) PAIRS.push([n, bg, 3, 1]);
}
/** 薄くした線や枠（Workflow の選ばれてない線など）。濃さは 0〜1 で指定する。 */
function DIMMED(names, alpha) {
  for (const n of names) for (const bg of BGS) PAIRS.push([n, bg, 3, alpha]);
}

/**
 * 枠の薄い塗り（color-mix で地色に tints の色を alpha だけ混ぜた色）の上に文字を置くとき。
 * fg は文字、tints は枠の色、alpha は塗りの濃さ、min は下限。
 */
function TINTED(fg, tints, alpha, min) {
  for (const t of tints) for (const base of BGS) {
    PAIRS.push([fg, { tint: t, alpha, over: base }, min, 1]);
  }
}

// 文字
TEXT(["--text", "--text-soft", "--accent", "--accent-mcp", "--ok", "--warn", "--danger", "--run"]);
// 線と枠
GRAPHICAL(["--line-strong", "--sel", "--focus"]);
// 札のタグ 6 色
GRAPHICAL(["--tag-brst", "--tag-plan", "--tag-code", "--tag-comp", "--tag-rfrs", "--tag-other"]);
// Workflow の線の種類
GRAPHICAL([
  "--edge-delegation", "--edge-task-output", "--edge-waggle",
  "--edge-commit", "--edge-phase", "--edge-other",
]);
// Canvas の線 5 色・札 3 色・枠 6 色
GRAPHICAL([
  "--cv-edge-citation", "--cv-edge-uses", "--cv-edge-derived", "--cv-edge-mentions", "--cv-edge-link",
  "--cv-card-note", "--cv-card-report", "--cv-card-paper",
  "--cv-group-1", "--cv-group-2", "--cv-group-3", "--cv-group-4", "--cv-group-5", "--cv-group-6",
]);
// グラフの系列
GRAPHICAL(["--series-1", "--series-2", "--series-3", "--series-4", "--series-5", "--series-6"]);
// 研究方針の木の状態と線（U19）
GRAPHICAL([
  "--sci-decided", "--sci-tentative", "--sci-open", "--sci-excluded", "--sci-candidate",
  "--sci-rejected", "--sci-dormant", "--sci-support", "--sci-refute",
  "--sci-edge-parent", "--sci-edge-depends", "--sci-edge-derived", "--sci-edge-analogous",
]);
// グラフの文字（縦の目盛り）と、棒の底の線（0 の日を表す短い棒にも使う）
TEXT(["--chart-axis"]);
GRAPHICAL(["--line-strong"]);

// Workflow 盤面（U02）。wfEdgeOpacity（workflow.js）の WF_EDGE_DIM と同じ濃さで、
// 薄くしても線として 3:1 を保つことを検査する。
DIMMED([
  "--edge-delegation", "--edge-task-output", "--edge-waggle",
  "--edge-commit", "--edge-phase", "--edge-other",
], 0.8);

// 却下・休眠の候補の薄い色（U19。U21 が候補の表に描く）。
// 候補の行は「却下 ✕」「休眠 ◌」の記号と文字も出すので、色は補助として使う。
// 不透明度 0.45 では 3:1 に届かない（実測 2.3:1）ので、両方の画面で 3:1 を保つ
// 最小の濃さ 0.7 で検査する。U21 は 0.7 以上で描くか、記号と文字に頼ること。
DIMMED(["--sci-rejected", "--sci-dormant"], 0.7);

// 地色をまたいだ組。[[手前, 後ろ, 下限, 濃さ], …] を足す。
PAIRS.push(
  ["--warn-on-bg", "--warn-bg", 4.5, 1],   // 質問待ちの行・接続切れの帯
  ["--note-on-bg", "--note-bg", 4.5, 1],   // Canvas の本文ブロック
  ["--note-line", "--note-bg", 3, 1],      // その枠
  ["--chart-axis", "--bg", 4.5, 1],        // 目盛りの文字
  ["--chart-axis", "--bg-soft", 4.5, 1],
  ["--on-accent", "--accent", 4.5, 1],     // 塗りつぶしたボタンの上の文字
  ["--on-accent", "--ok", 4.5, 1],
  ["--on-accent", "--warn", 4.5, 1],
  ["--on-accent", "--danger", 4.5, 1]
);

// --- Workflow 盤面（U02）---
//
// 上の TEXT / GRAPHICAL / DIMMED は --bg・--bg-soft・--st-card の 3 つの地色と自動で組を作る。
// ここには Workflow 固有の「どの地色のの上に何が乗る」を明示的に書いておく。
// 札の地色は --bg、盤面（workflow.css の .wf-stage）は --bg-soft。
PAIRS.push(
  // 札の地色（--bg）の上の文字。題は --text、補助の文字は --text-soft
  ["--text", "--bg", 4.5, 1],
  ["--text-soft", "--bg", 4.5, 1],
  // 薄くした札（.wf-dim）の地色は盤面と同じ（--bg-soft）になる。その上の文字
  ["--text", "--bg-soft", 4.5, 1],
  ["--text-soft", "--bg-soft", 4.5, 1],
  // 盤面（--bg-soft）の上の線。選ばれてない線を WF_EDGE_DIM まで薄くしても 3:1
  ["--edge-delegation", "--bg-soft", 3, 0.8],
  ["--edge-task-output", "--bg-soft", 3, 0.8],
  ["--edge-waggle", "--bg-soft", 3, 0.8],
  ["--edge-commit", "--bg-soft", 3, 0.8],
  ["--edge-phase", "--bg-soft", 3, 0.8],
  ["--edge-other", "--bg-soft", 3, 0.8],
  // 枠。round の区切り線と札の枠は --line-strong、選んだ札の枠は --sel
  ["--line-strong", "--bg-soft", 3, 1],
  ["--line-strong", "--bg", 3, 1],
  ["--sel", "--bg", 3, 1]
);

// --- Canvas 盤面（U03）---
//
// 上の TEXT / GRAPHICAL / DIMMED は --bg・--bg-soft・--st-card の 3 つの地色と自動で組を作る。
// ここには Canvas 固有の「どの地色の上に何が乗る」を明示的に書いておく。
// 盤面（canvas.css の .cv-stage）は --bg-soft、札（.cv-card）は --bg、枠は盤面の上。
const CV_EDGES = [
  "--cv-edge-citation", "--cv-edge-uses", "--cv-edge-derived", "--cv-edge-mentions", "--cv-edge-link",
];
const CV_GROUPS = [
  "--cv-group-1", "--cv-group-2", "--cv-group-3", "--cv-group-4", "--cv-group-5", "--cv-group-6",
];
const CV_CARDS = ["--cv-card-note", "--cv-card-report", "--cv-card-paper"];
PAIRS.push(
  // 札の地色（--bg）の上の文字。題は --text、補助の文字は --text-soft
  ["--text", "--bg", 4.5, 1],
  ["--text-soft", "--bg", 4.5, 1],
  // 薄くした札（.cv-dim）の地色は盤面と同じ（--bg-soft）になる。その上の文字
  ["--text", "--bg-soft", 4.5, 1],
  ["--text-soft", "--bg-soft", 4.5, 1],
  // 札の枠（--line-strong）と、選んだ札・選んだ枠（--sel）
  ["--line-strong", "--bg-soft", 3, 1],
  ["--line-strong", "--bg", 3, 1],
  ["--sel", "--bg", 3, 1],
  ["--sel", "--bg-soft", 3, 1]
);
// 線。盤面（--bg-soft）の上で、選ばれてない線を CV_EDGE_DIM（canvas.js）まで薄くしても 3:1
for (const e of CV_EDGES) {
  PAIRS.push([e, "--bg-soft", 3, 1]);
  PAIRS.push([e, "--bg-soft", 3, 0.8]);
}
// 枠の見出しは枠の外（盤面の上）にあるので、文字として 4.5 を求める
for (const g of CV_GROUPS) PAIRS.push([g, "--bg-soft", 4.5, 1]);
// 札の左の帯は図形（3:1）だが、種類の文字（.cv-kind）も同じ色を使うので 4.5 も見る
for (const c of CV_CARDS) {
  PAIRS.push([c, "--bg", 3, 1]);
  PAIRS.push([c, "--bg", 4.5, 1]);
}
// 枠の塗り（cvAlpha が color-mix で書く薄い色）の上に置く文字。--text なら 4.5 を保つ
TINTED("--text", CV_GROUPS, 0.12, 4.5);

// --- グラフ（U04）---
//
// グラフは素の SVG を手描きする（style.css の .dash-* と .lv-*）。
// 系列の色は棒とドーナツの線と凡例の四角に、地色はカードの
// --bg・--bg-soft・--st-card（統計画面のカード）の 3 つに使われる。
// 上の GRAPHICAL はその 3 つの地色と自動で組を作るので、ここでは
// 「1 つのグラフの中で隣り合う系列が色相で見分けられるか」だけ足す。
PAIRS.push(
  // ドーナツの凡例は .st-card（統計）と --bg-soft（Dashboard）の上に並ぶ
  ["--chart-axis", "--st-card", 4.5, 1],
  // 0 の日を表す短い棒（.dash-bar.zero）は線と同じ扱い
  ["--line-strong", "--bg-soft", 3, 1]
);

// 隣り合う系列の色相差（度）。下の HUE_PAIRS で 1 組ずつ比べる。
const HUE_MIN = 30;
// 色相の比較をしない色（彩度が低いので色相では区別できないため）。
// 灰色は凡例の文字と値とで見分ける（色だけに頼らない）。
const HUE_MIN_SAT = 0.35;
const HUE_PAIRS = [
  ["--series-1", "--series-2"],
  ["--series-2", "--series-3"],
  ["--series-3", "--series-4"],
  ["--series-4", "--series-5"],
  ["--series-5", "--series-6"],
];
// 明るい画面と暗い画面の両方に定義されていることを確かめる
// （片側にしか無いと、どちらかの画面で読めなくなる）。
const REQUIRED = [
  // 盤面・札・文字・枠
  "--bg", "--bg-soft", "--text", "--text-soft", "--line", "--line-strong", "--sel",
  // 札の帯と線
  "--tag-brst", "--tag-plan", "--tag-code", "--tag-comp", "--tag-rfrs", "--tag-other",
  "--edge-delegation", "--edge-task-output", "--edge-waggle", "--edge-commit",
  "--edge-phase", "--edge-other",
  // 状態
  "--ok", "--warn", "--danger", "--accent",
  // 論文の Canvas が必ず使うもの（付箋・線 5 色・札 3 色・枠 6 色）
  "--note-bg", "--note-on-bg", "--note-line",
  ...CV_EDGES, ...CV_CARDS, ...CV_GROUPS,
  // グラフ（U04）
  "--series-1", "--series-2", "--series-3", "--series-4", "--series-5", "--series-6",
  "--chart-axis", "--chart-grid", "--st-card",
  // 研究方針の木（U19）
  "--sci-decided", "--sci-tentative", "--sci-open", "--sci-excluded", "--sci-candidate",
  "--sci-rejected", "--sci-dormant", "--sci-support", "--sci-refute",
  "--sci-edge-parent", "--sci-edge-depends", "--sci-edge-derived", "--sci-edge-analogous",
];

// --- CSS を読む ---

/** コメントを取り除いた CSS。 */
function stripComments(css) {
  return css.replace(/\/\*[\s\S]*?\*\//g, " ");
}

/** pos 以降の最初の { に対応する {} の中身と、その始めと終りの位置。 */
function blockAt(css, pos) {
  const open = css.indexOf("{", pos);
  if (open < 0) return null;
  let depth = 0;
  for (let i = open; i < css.length; i += 1) {
    if (css[i] === "{") depth += 1;
    else if (css[i] === "}") {
      depth -= 1;
      if (depth === 0) return { body: css.slice(open + 1, i), start: open, end: i };
    }
  }
  return null;
}

/** 1 つの :root の中身から「名前 → 値」を読む。後のものが上書きする。 */
function readVars(body, into) {
  const re = /(--[A-Za-z0-9-]+)\s*:\s*([^;]+);/g;
  let m;
  while ((m = re.exec(body)) !== null) into.set(m[1], m[2].trim());
  return into;
}

/**
 * style.css から 2 つの表を返す。{ light, dark }。
 * 暗い画面の :root が dark、それ以外の :root が light。
 * 同じセレクタが複数あれば順に読み、後のものを上書きする（CSS と同じ）。
 */
function readThemes(css) {
  const src = stripComments(css);
  const light = new Map();
  const dark = new Map();

  // 暗い画面の :root がある範囲（[開始, 終了] の inclusive）を集める
  const darkRanges = [];
  let at = 0;
  while (true) {
    const i = src.indexOf("@media (prefers-color-scheme: dark)", at);
    if (i < 0) break;
    const blk = blockAt(src, i);
    if (!blk) break;
    darkRanges.push([i, blk.end]);
    at = blk.end;
  }
  const inDark = (pos) => darkRanges.some(([a, b]) => pos > a && pos < b);

  at = 0;
  while (true) {
    const i = src.indexOf(":root", at);
    if (i < 0) break;
    const blk = blockAt(src, i);
    if (!blk) break;
    at = blk.end;
    readVars(blk.body, inDark(blk.start) ? dark : light);
  }
  return { light, dark };
}

// --- 色の計算 ---

/** 変数の値を hex にする。"var(--x)" なら先に 1 段だけたどる。 */
function resolve(name, vars) {
  const raw = vars.get(name);
  if (raw == null) return null;
  const m = /^var\((--[A-Za-z0-9-]+)\)$/.exec(raw);
  if (m) return vars.get(m[1]) || null;
  return raw;
}

/** sRGB の 8bit 値を線形の輝度にする。 */
function linear(v) {
  const c = v / 255;
  return c <= 0.03928 ? c / 12.92 : Math.pow((c + 0.055) / 1.055, 2.4);
}

/** "#rrggbb" の相対輝度（0〜1）。それ以外の文字列は null。 */
function luminance(hex) {
  if (typeof hex !== "string") return null;
  const m = /^#([0-9a-fA-F]{6})$/.exec(hex.trim());
  if (!m) return null;
  const n = parseInt(m[1], 16);
  return 0.2126 * linear((n >> 16) & 255) + 0.7152 * linear((n >> 8) & 255) + 0.0722 * linear(n & 255);
}

/** コントラスト比（1〜21）。読めない値があれば null。 */
function ratio(a, b) {
  const la = luminance(a);
  const lb = luminance(b);
  if (la == null || lb == null) return null;
  const hi = Math.max(la, lb);
  const lo = Math.min(la, lb);
  return (hi + 0.05) / (lo + 0.05);
}

const isHex = (v) => typeof v === "string" && /^#[0-9a-fA-F]{6}$/.test(v.trim());

/** "#rrggbb" を {r,g,b} にする。 */
function rgbOf(hex) {
  const n = parseInt(hex.slice(1), 16);
  return { r: (n >> 16) & 255, g: (n >> 8) & 255, b: n & 255 };
}

/** 彩度（0 は灰色、1 ははっきりした色）。HLS での s。 */
function saturation(hex) {
  const { r, g, b } = rgbOf(hex);
  const max = Math.max(r, g, b);
  const min = Math.min(r, g, b);
  if (max === 0) return 0;
  return (max - min) / max;
}

/** 色相（0〜360 度）。灰色のときは 0。 */
function hue(hex) {
  const { r, g, b } = rgbOf(hex);
  const max = Math.max(r, g, b);
  const min = Math.min(r, g, b);
  const d = max - min;
  if (d === 0) return 0;
  let h;
  if (max === r) h = ((g - b) / d) % 6;
  else if (max === g) h = (b - r) / d + 2;
  else h = (r - g) / d + 4;
  return ((h * 60) % 360 + 360) % 360;
}

/** 2 つの色の色相の差（0〜180 度）。 */
function hueGap(a, b) {
  const d = Math.abs(hue(a) - hue(b));
  return d > 180 ? 360 - d : d;
}

/** 2 つの hex を a ずつ混ぜて 1 色にする（薄くした線と地色の実効色）。 */
function blend(fg, bg, alpha) {
  const a = Math.max(0, Math.min(1, Number(alpha)));
  if (a >= 1) return fg;
  if (a <= 0) return bg;
  const f = fg.slice(1);
  const b = bg.slice(1);
  let out = "#";
  for (let i = 0; i < 6; i += 2) {
    const fv = parseInt(f.substr(i, 2), 16);
    const bv = parseInt(b.substr(i, 2), 16);
    out += Math.round(fv * a + bv * (1 - a)).toString(16).padStart(2, "0");
  }
  return out;
}

/**
 * 枠の塗り（color-mix で tint を alpha だけ地色に混ぜた色）を、実際に的地色に混ぜた色にする。
 * CSS の color-mix(in srgb, tint …%, transparent) は手前の色を地色に透かすので、
 * ここでは地色に tint を混ぜるのと同じ計算になる。
 */
function mixed(spec, vars) {
  const tint = resolve(spec.tint, vars);
  const over = resolve(spec.over, vars);
  if (!isHex(tint) || !isHex(over)) return null;
  return blend(tint, over, spec.alpha);
}

// --- 検査 ---

function main() {
  const css = fs.readFileSync(CSS_PATH, "utf8");
  const { light, dark } = readThemes(css);

  assert.ok(light.size > 0, "明るい画面の :root が読めませんでした");
  assert.ok(dark.size > 0, "暗い画面の :root が読めませんでした（@media (prefers-color-scheme: dark) の中）");

  const bad = [];
  let checked = 0;
  for (const [theme, vars] of [["明るい画面", light], ["暗い画面", dark]]) {
    for (const [fg, bg, min, alpha] of PAIRS) {
      const f = resolve(fg, vars);
      // 後ろの色は token 名（--bg など）か、枠の塗りを計算した色（{tint, alpha, over}）
      const isMix = bg !== null && typeof bg === "object";
      const b = isMix ? mixed(bg, vars) : resolve(bg, vars);
      // 失敗の文で使う色名（枠の塗りのときは混ぜる色を書き出す）
      const bgName = isMix ? `${bg.tint} ${Math.round(bg.alpha * 100)}% の塗り` : bg;
      if (!isHex(f)) {
        bad.push(`${theme}: ${fg} が不透明な 6 桁の hex でない（${f == null ? "なし" : f}）`);
        continue;
      }
      if (!isHex(b)) {
        bad.push(`${theme}: ${bgName} が不透明な 6 桁の hex でない（${b == null ? "なし" : b}）`);
        continue;
      }
      // 薄くする指定があるときは地色に混ぜてから比べる
      const shown = alpha == null || alpha >= 1 ? f : blend(f, b, alpha);
      const r = ratio(shown, b);
      if (r + 1e-9 < min) {
        bad.push(`${theme}: ${fg}(${shown}) / ${bgName}(${b}) = ${r.toFixed(2)} が下限 ${min} 未満`);
      }
      checked += 1;
    }
  }

  // グラフの隣り合う系列が色相で見分けられること（U04）
  for (const [theme, vars] of [["明るい画面", light], ["暗い画面", dark]]) {
    for (const [a, b] of HUE_PAIRS) {
      const ca = resolve(a, vars);
      const cb = resolve(b, vars);
      if (!isHex(ca) || !isHex(cb)) continue; // 定義の無さは上で報告している
      if (saturation(ca) < HUE_MIN_SAT || saturation(cb) < HUE_MIN_SAT) continue;
      const gap = hueGap(ca, cb);
      checked += 1;
      if (gap < HUE_MIN) {
        bad.push(`${theme}: ${a}(${ca}) と ${b}(${cb}) の色相差が ${gap.toFixed(0)} 度で ${HUE_MIN} 度未満`);
      }
    }
  }

  // Workflow・Canvas 盤面が使う token が両方の画面にあること
  for (const [theme, vars] of [["明るい画面", light], ["暗い画面", dark]]) {
    for (const name of REQUIRED) {
      if (!isHex(resolve(name, vars))) {
        bad.push(`${theme}: 盤面が使う ${name} が両方の画面に定義されていない`);
      }
      checked += 1;
    }
  }

  if (bad.length) {
    for (const line of bad) console.error("  NG " + line);
    console.error(`コントラストの検査に ${bad.length} 件 失敗しました（${checked} 組を測定）`);
    process.exit(1);
  }
  console.log(`コントラストの検査に合格しました（${checked} 組・明るい画面と暗い画面の両方）`);
}

main();
