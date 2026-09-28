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

// Workflow 盤面（U02）。wfEdgeOpacity（workflow.js）の WF_EDGE_DIM と同じ濃さで、
// 薄くしても線として 3:1 を保つことを検査する。
DIMMED([
  "--edge-delegation", "--edge-task-output", "--edge-waggle",
  "--edge-commit", "--edge-phase", "--edge-other",
], 0.8);

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

// --- Workflow 盤面が必ず使う token ---
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
      const b = resolve(bg, vars);
      if (!isHex(f)) {
        bad.push(`${theme}: ${fg} が不透明な 6 桁の hex でない（${f == null ? "なし" : f}）`);
        continue;
      }
      if (!isHex(b)) {
        bad.push(`${theme}: ${bg} が不透明な 6 桁の hex でない（${b == null ? "なし" : b}）`);
        continue;
      }
      // 薄くする指定があるときは地色に混ぜてから比べる
      const shown = alpha == null || alpha >= 1 ? f : blend(f, b, alpha);
      const r = ratio(shown, b);
      if (r + 1e-9 < min) {
        bad.push(`${theme}: ${fg}(${shown}) / ${bg}(${b}) = ${r.toFixed(2)} が下限 ${min} 未満`);
      }
      checked += 1;
    }
  }

  // Workflow 盤面が使う token が両方の画面にあること
  for (const [theme, vars] of [["明るい画面", light], ["暗い画面", dark]]) {
    for (const name of REQUIRED) {
      if (!isHex(resolve(name, vars))) {
        bad.push(`${theme}: Workflow が使う ${name} が両方の画面に定義されていない`);
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
