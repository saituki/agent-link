// md の整形表示（実装計画 13 §1.4・§5）の共通部品。
//
// - 見出し 6 段階・段落・太字 / 斜体・`コード`・リンク（http/https のみ有効）
// - 箇条書き（入れ子・番号付き・チェックボックス）・引用・区切り線・表・コードの枠
//   （表とコードの枠は style.css ではみ出す分だけ横にスクロールさせる）
// - 先頭の frontmatter（--- で囲んだ設定部分）は小さな「設定」表としてたたんで出す
// - 見出しをタップすると、その見出しの節（中身）をたたむ / 開く（状態は保存しない）
// - 文字は全部 HTML エスケープしてから組み立てる（文書中の生の HTML は文字として出す）
// - 外部ライブラリは入れない
//
// 使い方: renderMarkdown(text[, {toolbar: true}]) が返す HTML を innerHTML に入れる。
// 呼び出し元: app.js（Agent の吹き出し）・papers.js（ノートと論文本文）・radio.js（質問の吹き出し）。
// index.html では app.js より前に読む。node からの読み込みは tests/test_markdown.js。
//
// U4 で足した物:
//   - コードの色分け（mdHighlight・TUI と同じ base16-ocean.dark の色）
//   - 道具の行（全部たたむ / 全部開く / 目次）… renderMarkdown(text, {toolbar: true})
//   - 長いコードの枠（21 行以上）は 12 行だけ出して「全部見る」で開く
//   - 一番外側に .md-doc を付ける（道具の行は 1 つの文書の中だけに効く）
"use strict";

// --- 文字のエスケープ（この部品から出る文字は必ずここを通す） ---

/** HTML の文字として出したいときに使う（生の HTML は文字化して無効にする）。 */
function mdEscape(s) {
  return String(s == null ? "" : s)
    .replace(/\u0000/g, "\uFFFD") // 後段の退避（プレースホルダ）と混ざらないように
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;")
    .replace(/'/g, "&#39;");
}

// waggle の参照。前は英数字・_ 以外、トークンは英数字 8 文字、`:path:` 以降は任意
// （パスの最後の文字が `.` のときは文末の点なので含めない）。
const RE_WAGGLE_REF =
  /(^|[^A-Za-z0-9_])waggle:([A-Za-z0-9]{8})(?![A-Za-z0-9])(?::path:((?:[A-Za-z0-9_.\/*+@%~-]*[A-Za-z0-9_\/*+@%~-])?))?/g;

/** 1 行（または1セル）の中の md 記号を整える。エスケープは先に行う。 */
function mdInline(raw) {
  const text = mdEscape(raw);
  // 1) `コード` を先に退避する（他の記号をかけないように）
  const codes = [];
  let out = text.replace(/`([^`]+)`/g, (m, code) => {
    codes.push(`<code>${code}</code>`); // code はもうエスケープ済み
    return `\u0000${codes.length - 1}\u0000`;
  });
  // 1.5) waggle の参照（`waggle:<トークン>` と `waggle:<トークン>:path:.sai/outputs/*/<run_id>/<ファイル>`）。
  //      押せるリンクにする（行き先は linkview.js が住所にする）。パスの `*` が斜体にならないよう退避する。
  const waggles = [];
  out = out.replace(RE_WAGGLE_REF, (m, pre, token, path) => {
    const run = /(?:^|\/)outputs\/[^/]+\/(\d+)\//.exec(path || "");
    waggles.push(
      `<a href="#" class="md-waggle" data-waggle-token="${token}"${run ? ` data-waggle-run="${run[1]}"` : ""}>${m.slice(pre.length)}</a>`
    );
    return `${pre}\u0000W${waggles.length - 1}\u0000`;
  });
  // 2) リンク。http: / https: だけ有効にし、危ない接続辞書は文字のまま残す。
  //    URL は退避して、太字 / 斜体が属性の中を壊さないようにする。
  const urls = [];
  out = out.replace(/\[([^\]]*)\]\(\s*([^)\s]+)\s*\)/g, (m, label, url) => {
    if (!/^https?:\/\//i.test(url)) return m; // javascript: などはリンクにしない
    urls.push(url); // エスケープ済み（属性に入れてよい形）
    return `<a href="\u0000U${urls.length - 1}\u0000" target="_blank" rel="noopener noreferrer">${label}</a>`;
  });
  // 3) 太字 → 斜体（** の方が先。残った * 1 個ぶんは斜体）
  out = out
    .replace(/\*\*([^*]+)\*\*/g, "<strong>$1</strong>")
    .replace(/\*([^*]+)\*/g, "<em>$1</em>");
  // 4) 退避した URL とコードを戻す（URL を先に戻す。コード番号と取り違えないため）
  out = out.replace(/\u0000W(\d+)\u0000/g, (m, k) => waggles[Number(k)] || "");
  out = out.replace(/\u0000U(\d+)\u0000/g, (m, k) => urls[Number(k)] || "");
  return out.replace(/\u0000(\d+)\u0000/g, (m, k) => codes[Number(k)] || "");
}

// --- frontmatter（先頭の --- で囲んだ設定部分） ---

/** 先頭の frontmatter と本文に分ける（無ければ front は null）。 */
function splitFrontmatter(text) {
  if (!/^---\n/.test(text)) return { front: null, body: text };
  const lines = text.split("\n");
  let end = -1;
  for (let i = 1; i < lines.length; i++) {
    if (/^(---|\.\.\.)\s*$/.test(lines[i])) { end = i; break; }
  }
  if (end < 0) return { front: null, body: text }; // 閉じる --- が無いので設定扱いしない
  return { front: lines.slice(1, end), body: lines.slice(end + 1).join("\n") };
}

/** frontmatter を小さな「設定」表にして，たたんだ状態で出す（タップで開く）。 */
function renderFrontmatter(lines) {
  const rows = [];
  for (const line of lines) {
    if (line.trim() === "") continue;
    const m = /^([^:]+):\s*(.*)$/.exec(line);
    const key = m ? m[1].trim() : line.trim();
    const val = m ? m[2] : "";
    rows.push(`<tr><th>${mdEscape(key)}</th><td>${mdInline(val)}</td></tr>`);
  }
  if (rows.length === 0) return "";
  return `<details class="md-front"><summary>設定</summary><table class="md-front-table"><tbody>${rows.join("")}</tbody></table></details>`;
}

// --- 箇条書き（入れ子はインデントの深さで見る） ---

const RE_LIST = /^(\s*)([-*+]|\d+[.)])\s+(.*)$/;

/** 行頭の空白幅（タブは 2 ぶんとして数える）。 */
function indentWidth(s) {
  let w = 0;
  for (const ch of String(s)) {
    if (ch === " ") w += 1;
    else if (ch === "\t") w += 2;
    else break;
  }
  return w;
}

function isOrderedMarker(marker) {
  return /^\d/.test(marker);
}

/** リストの本体（入れ子の行・空行・続ける行）を start から全部集める。 */
function collectListLines(lines, start) {
  const first = RE_LIST.exec(lines[start]);
  const baseIndent = indentWidth(first[1]);
  const ordered = isOrderedMarker(first[2]);
  const out = [];
  let i = start;
  while (i < lines.length) {
    const line = lines[i];
    if (line.trim() === "") {
      // 空行のあとがリストの続きなら取り込む（item の区切りとして使う）
      let j = i;
      while (j < lines.length && lines[j].trim() === "") j++;
      if (j >= lines.length) break;
      const nm = RE_LIST.exec(lines[j]);
      const ni = nm ? indentWidth(nm[1]) : indentWidth(lines[j].match(/^\s*/)[0]);
      const keep = nm ? ni >= baseIndent && isOrderedMarker(nm[2]) === ordered : ni > baseIndent;
      if (!keep) break;
      out.push("");
      i++;
      continue;
    }
    const m = RE_LIST.exec(line);
    if (m) {
      const ni = indentWidth(m[1]);
      if (ni < baseIndent) break;                        // 外側のレベル → ここまで
      if (ni === baseIndent && isOrderedMarker(m[2]) !== ordered) break; // 種類が変わったら別リスト
      out.push(line);
      i++;
      continue;
    }
    if (indentWidth(line) > baseIndent) { out.push(line); i++; continue; } // 続きの行
    break;
  }
  return { lines: out, next: i };
}

/** 集めた行（リスト 1 ぶん）を <ul>/<ol> の HTML に組み立てる。 */
function buildList(lines) {
  const firstLine = lines.find((l) => RE_LIST.test(l));
  if (!firstLine) return "";
  const first = RE_LIST.exec(firstLine);
  const baseIndent = indentWidth(first[1]);
  const ordered = isOrderedMarker(first[2]);
  const items = [];
  let i = 0;
  while (i < lines.length) {
    const line = lines[i];
    if (line.trim() === "") { i++; continue; }
    const m = RE_LIST.exec(line);
    if (!m || indentWidth(m[1]) !== baseIndent || isOrderedMarker(m[2]) !== ordered) break;
    // item の中身（入れ子のリスト・続ける行）を全部集める
    const body = [];
    i++;
    while (i < lines.length) {
      const l = lines[i];
      if (l.trim() === "") {
        let j = i;
        while (j < lines.length && lines[j].trim() === "") j++;
        if (j >= lines.length) break;
        const nm = RE_LIST.exec(lines[j]);
        const ni = nm ? indentWidth(nm[1]) : indentWidth(lines[j].match(/^\s*/)[0]);
        if (!(ni > baseIndent)) break; // 空白のあともっと深い所だけ item の中身
        body.push("");
        i++;
        continue;
      }
      const nm = RE_LIST.exec(l);
      if (nm && indentWidth(nm[1]) <= baseIndent) break;  // 同じか外側のレベル → 次の item
      if (!nm && indentWidth(l) <= baseIndent) break;     // item の外の文章
      body.push(l);
      i++;
    }
    items.push(renderListItem(m[3], body));
  }
  const tag = ordered ? "ol" : "ul";
  const cls = ordered ? "md-ol" : "md-ul";
  return `<${tag} class="${cls}">${items.join("")}</${tag}>`;
}

/** item 1 行の本文（続きの行・入れ子のリスト）を順に組み立てる。 */
function renderBodySegments(body) {
  let html = "";
  let i = 0;
  while (i < body.length) {
    const line = body[i];
    if (line.trim() === "") { i++; continue; }
    if (RE_LIST.test(line)) {
      const r = collectListLines(body, i);
      html += buildList(r.lines);
      i = r.next;
      continue;
    }
    // 続きの文章（行頭のインデントは落とし，行と行の間は <br> でつなぐ）
    const para = [];
    while (i < body.length && body[i].trim() !== "" && !RE_LIST.test(body[i])) {
      para.push(body[i].trim());
      i++;
    }
    html += `<p class="md-p">${para.map(mdInline).join("<br>")}</p>`;
  }
  return html;
}

/** 箇条書きの item（チェックボックスにも対応）。 */
function renderListItem(headText, body) {
  let head = headText;
  let inner = "";
  const cls = ["md-li"];
  const cb = /^\[([ xX])\]\s*(.*)$/.exec(headText);
  if (cb) {
    // チェックボックス（読むだけなので disabled）
    cls.push("md-task");
    inner += `<input type="checkbox" disabled${cb[1] !== " " ? " checked" : ""}> `;
    head = cb[2];
  }
  inner += mdInline(head);
  inner += renderBodySegments(body);
  return `<li class="${cls.join(" ")}">${inner}</li>`;
}

// --- ブロック（見出し・表・引用・コードの枠・段落） ---

const RE_HEADING = /^(#{1,6})\s+(.*?)\s*$/;
const RE_FENCE = /^\s*(```|~~~)\s*([A-Za-z0-9_+-]*)\s*$/;
const RE_FENCE_END = /^\s*(```|~~~)\s*$/;
const RE_HR = /^\s*(?:(?:-\s*){3,}|(?:\*\s*){3,}|(?:_\s*){3,})$/;
const RE_QUOTE = /^\s*>/;

/** 表の区切り行（| --- | :--: |）かどうか。 */
function isTableSepLine(line) {
  const cells = splitTableRow(line);
  return cells.length > 0 && cells.every((c) => /^:?-+:?$/.test(c));
}

/** 表の 1 行をセルの配列にする（\| は区切りにしない）。 */
function splitTableRow(line) {
  const t = line.trim().replace(/\\\|/g, "\u0001");
  let s = t;
  if (s.startsWith("|")) s = s.slice(1);
  if (s.endsWith("|")) s = s.slice(0, -1);
  return s.split("|").map((c) => c.trim().replace(/\u0001/g, "|"));
}

/** 表（見出し行＋区切り行＋データ行）。はみ出た分は .md-tbl-wrap が横スクロールする。 */
function renderTable(lines, start) {
  const head = splitTableRow(lines[start]);
  const specs = splitTableRow(lines[start + 1]);
  // 左右中央の指定（:--- / ---: / :---:）を style にする
  const alignOf = (c) =>
    c.startsWith(":") && c.endsWith(":") && c.length > 1
      ? "center"
      : c.endsWith(":")
        ? "right"
        : c.startsWith(":")
          ? "left"
          : "";
  const aligned = specs.map(alignOf);
  const attr = (k) => (aligned[k] ? ` style="text-align:${aligned[k]}"` : "");
  let i = start + 2;
  const rows = [];
  while (i < lines.length && lines[i].trim() !== "" && lines[i].includes("|")) {
    const l = lines[i];
    if (RE_HEADING.test(l) || RE_LIST.test(l) || RE_HR.test(l) || RE_QUOTE.test(l) || RE_FENCE.test(l)) break;
    rows.push(splitTableRow(l));
    i++;
  }
  const th = head.map((c, k) => `<th${attr(k)}>${mdInline(c)}</th>`).join("");
  const trs = rows
    .map(
      (r) =>
        `<tr>${head.map((_, k) => `<td${attr(k)}>${mdInline(r[k] == null ? "" : r[k])}</td>`).join("")}</tr>`
    )
    .join("");
  return {
    html: `<div class="md-tbl-wrap"><table class="md-table"><thead><tr>${th}</tr></thead><tbody>${trs}</tbody></table></div>`,
    next: i,
  };
}

/** ブロックの先頭かどうか（段落をここで打ち切る）。 */
function isBlockStart(line) {
  return (
    RE_HEADING.test(line) || RE_FENCE.test(line) || RE_HR.test(line) ||
    RE_LIST.test(line) || RE_QUOTE.test(line)
  );
}

/** 行の配列をブロック（見出しと HTML 断片）の並びにする。 */
function parseBlocks(lines) {
  const out = [];
  let i = 0;
  while (i < lines.length) {
    const line = lines[i];
    if (line.trim() === "") { i++; continue; }

    // コードの枠（```lang … ```）
    const mf = RE_FENCE.exec(line);
    if (mf) {
      const fence = mf[1];
      const lang = mf[2];
      const body = [];
      i++;
      while (i < lines.length && !RE_FENCE_END.test(lines[i])) { body.push(lines[i]); i++; }
      if (i < lines.length) i++; // 閉じる ``` の行
      out.push({ type: "html", html: renderCodeBlock(body, lang) });
      continue;
    }

    // 見出し（# 〜 ######）
    const mh = RE_HEADING.exec(line);
    if (mh) {
      out.push({
        type: "heading",
        level: mh[1].length,
        text: mh[2].replace(/\s+#+\s*$/, ""), // 行末の ### は落とす
      });
      i++;
      continue;
    }

    // 区切り線（--- / *** / ___）
    if (RE_HR.test(line)) { out.push({ type: "html", html: "<hr>" }); i++; continue; }

    // 引用（> …。中身はもう一度ここを通して整形する）
    if (RE_QUOTE.test(line)) {
      const inner = [];
      while (
        i < lines.length &&
        (RE_QUOTE.test(lines[i]) ||
          (lines[i].trim() === "" && i + 1 < lines.length && RE_QUOTE.test(lines[i + 1])))
      ) {
        inner.push(lines[i].replace(/^\s*>\s?/, ""));
        i++;
      }
      out.push({ type: "html", html: `<blockquote class="md-quote">${renderBody(inner)}</blockquote>` });
      continue;
    }

    // 表（見出し行と区切り行の両方に | がある）
    if (
      line.includes("|") && !RE_LIST.test(line) && i + 1 < lines.length &&
      lines[i + 1].includes("|") && isTableSepLine(lines[i + 1])
    ) {
      const t = renderTable(lines, i);
      out.push({ type: "html", html: t.html });
      i = t.next;
      continue;
    }

    // 箇条書き
    if (RE_LIST.test(line)) {
      const r = collectListLines(lines, i);
      out.push({ type: "html", html: buildList(r.lines) });
      i = r.next;
      continue;
    }

    // 段落（空行か次のブロックまで。行と行の間は <br>）
    const para = [];
    while (i < lines.length) {
      const l = lines[i];
      if (l.trim() === "" || isBlockStart(l)) break;
      if (l.includes("|") && i + 1 < lines.length && lines[i + 1].includes("|") && isTableSepLine(lines[i + 1])) break; // 表の見出し行
      para.push(l);
      i++;
    }
    if (para.length === 0) { i++; continue; } // 進まないための保険
    out.push({ type: "html", html: `<p class="md-p">${para.map(mdInline).join("<br>")}</p>` });
  }
  return out;
}

/** 見出しの節を <section> で包む（見出しのタップで中身をたためるように）。 */
function wrapSections(blocks) {
  let html = "";
  const stack = []; // 開いている節のレベル
  for (const b of blocks) {
    if (b.type === "heading") {
      while (stack.length > 0 && stack[stack.length - 1] >= b.level) {
        html += "</section>";
        stack.pop();
      }
      const tag = `h${b.level}`;
      html +=
        `<section class="md-sec">` +
        `<${tag} class="md-h md-lv${b.level}" data-md-fold role="button" tabindex="0">${mdInline(b.text)}</${tag}>`;
      stack.push(b.level);
    } else {
      html += b.html;
    }
  }
  while (stack.length > 0) { html += "</section>"; stack.pop(); }
  return html;
}

/** 本文（frontmatter も見た目も無い行の配列）を HTML にする。 */
function renderBody(lines) {
  return wrapSections(parseBlocks(lines));
}

// --- コードの色分け（mdHighlight・U4） ---
//
// TUI（sai-link-tui の base16-ocean.dark）と同じ色を、暗い地の上に置く。
// 完全な文法解析はしない（正規表現の簡易版・崩れても読める程度でよい）。
// 行をまたぐもの（C 式の複数行注釈・Python の 3 引用符の文字列）は状態として持ち回す。

// 色分けしない言語をまとめる（この中以外はエスケープだけ）
const MD_LANG_KIND = {
  // C 系の言語（キーワード・文字列・数・行コメントと複数行注釈）
  js: "clike", javascript: "clike", ts: "clike", typescript: "clike",
  rust: "clike", rs: "clike", go: "clike", java: "clike", c: "clike",
  cpp: "clike", css: "clike",
  // Python（キーワード・文字列・数・# の注釈）
  py: "py", python: "py",
  // シェル（行頭の語・--オプション・文字列・# の注釈）
  sh: "sh", bash: "sh", zsh: "sh", shell: "sh", console: "sh",
  // JSON（キー・文字列・数・true / false / null）
  json: "json",
  // YAML / TOML（キー・文字列・数・# の注釈）
  yaml: "yaml", yml: "yaml", toml: "yaml",
  // diff（+ の行 / - の行 / @@ の行）
  diff: "diff", patch: "diff",
};

const MD_KEYWORDS_CLIKE = new Set((
  "abstract as async await break case catch class const constructor continue crate declare default delete do " +
  "else enum export extends extern false final finally fn for from func function go goto if impl implements " +
  "import in instanceof interface let loop match mod move mut new null of override package priv private " +
  "protected pub public readonly return self sizeof static struct super switch this throw trait true try " +
  "type typeof undefined unsafe use using var void where while with yield"
).split(" "));

const MD_KEYWORDS_PY = new Set((
  "and as assert async await break class continue def del elif else except False finally for from global if " +
  "import in is lambda None nonlocal not or pass raise return self True try while with yield"
).split(" "));

// --- 色分けの素になる小道具（文字は必ず mdEscape を通す） ---
function mdTok(cls, text) {
  return cls ? `<span class="${cls}">${mdEscape(text)}</span>` : mdEscape(text);
}

// 識別子・数の読み取り（先頭からできるだけ長く取る）
const MD_IDENT = /^[A-Za-z_$][A-Za-z0-9_$]*/;
const MD_NUM = /^(?:0[xXbBoO][0-9a-fA-F_]+|[0-9][0-9_]*(?:\.[0-9_]+)?(?:[eE][+-]?[0-9]+)?)/;

/** C 系を色分けする（行コメント・複数行注釈・文字列・数・キーワード・関数名）。 */
function mdScanClike(src) {
  let out = "";
  let i = 0;
  const n = src.length;
  while (i < n) {
    const ch = src[i];
    // // から行末
    if (ch === "/" && src[i + 1] === "/") {
      let e = src.indexOf("\n", i);
      if (e < 0) e = n;
      out += mdTok("tok-com", src.slice(i, e));
      i = e;
      continue;
    }
    // 複数行注釈の開始
    if (ch === "/" && src[i + 1] === "*") {
      const e = src.indexOf("*/", i + 2);
      const end = e < 0 ? n : e + 2;
      out += mdTok("tok-com", src.slice(i, end));
      i = end;
      continue;
    }
    // 文字列（\ で次の文字を飛ばす）
    if (ch === '"' || ch === "'" || ch === "`") {
      let j = i + 1;
      while (j < n && src[j] !== ch && src[j] !== "\n") {
        if (src[j] === "\\") j++;
        j++;
      }
      const end = j < n ? j + 1 : n;
      out += mdTok("tok-str", src.slice(i, end));
      i = end;
      continue;
    }
    if (/[0-9]/.test(ch)) {
      const m = MD_NUM.exec(src.slice(i));
      if (m) {
        out += mdTok("tok-num", m[0]);
        i += m[0].length;
        continue;
      }
    }
    if (/[A-Za-z_$]/.test(ch)) {
      const m = MD_IDENT.exec(src.slice(i));
      const w = m[0];
      let j = i + w.length;
      while (src[j] === " " || src[j] === "\t") j++;
      if (MD_KEYWORDS_CLIKE.has(w)) out += mdTok("tok-kw", w);
      else if (src[j] === "(") out += mdTok("tok-fn", w);
      else out += mdTok("", w);
      i += w.length;
      continue;
    }
    out += mdTok("", ch);
    i++;
  }
  return out;
}

/** Python を色分けする（''' / \"\"\" の文字列は行をまたげる）。 */
function mdScanPy(src) {
  let out = "";
  let i = 0;
  const n = src.length;
  while (i < n) {
    const ch = src[i];
    if (ch === "#") {
      let e = src.indexOf("\n", i);
      if (e < 0) e = n;
      out += mdTok("tok-com", src.slice(i, e));
      i = e;
      continue;
    }
    if (ch === '"' || ch === "'") {
      const triple = src.startsWith(ch.repeat(3), i);
      const close = triple ? ch.repeat(3) : ch;
      let j = i + close.length;
      while (j < n && !src.startsWith(close, j)) {
        if (!triple && src[j] === "\\") j++;
        if (!triple && src[j] === "\n") break; // 1 行の文字列は行をまたがない
        j++;
      }
      const end = Math.min(j + close.length, n);
      out += mdTok("tok-str", src.slice(i, end));
      i = end;
      continue;
    }
    if (/[0-9]/.test(ch)) {
      const m = MD_NUM.exec(src.slice(i));
      if (m) {
        out += mdTok("tok-num", m[0]);
        i += m[0].length;
        continue;
      }
    }
    if (/[A-Za-z_]/.test(ch)) {
      const m = MD_IDENT.exec(src.slice(i));
      const w = m[0];
      let j = i + w.length;
      while (src[j] === " " || src[j] === "\t") j++;
      if (MD_KEYWORDS_PY.has(w)) out += mdTok("tok-kw", w);
      else if (src[j] === "(") out += mdTok("tok-fn", w);
      else out += mdTok("", w);
      i += w.length;
      continue;
    }
    out += mdTok("", ch);
    i++;
  }
  return out;
}

/** シェルを色分けする（行頭の語・--オプション・文字列・# の注釈）。 */
function mdScanSh(src) {
  return src
    .split("\n")
    .map((line) => {
      let out = "";
      let i = 0;
      const n = line.length;
      let head = true; // 行頭の語（コマンド名）をまだ出していない
      while (i < n) {
        const ch = line[i];
        if (ch === "#") {
          out += mdTok("tok-com", line.slice(i));
          break;
        }
        if (ch === "|" || ch === "&") {
          // パイプのあとはまた「行頭の語」
          out += mdTok("", ch);
          i++;
          head = true;
          continue;
        }
        if (ch === '"' || ch === "'") {
          let j = i + 1;
          while (j < n && line[j] !== ch) {
            if (line[j] === "\\") j++;
            j++;
          }
          const end = j < n ? j + 1 : n;
          out += mdTok("tok-str", line.slice(i, end));
          i = end;
          head = false;
          continue;
        }
        if (ch === "-" && line[i + 1] === "-") {
          const m = /^--?[A-Za-z][\w-]*/.exec(line.slice(i));
          if (m) {
            out += mdTok("tok-key", m[0]);
            i += m[0].length;
            head = false;
            continue;
          }
        }
        if (head && /[^\s=]/.test(ch)) {
          const m = /^[^\s=|&]+/.exec(line.slice(i));
          out += mdTok("tok-fn", m[0]);
          i += m[0].length;
          head = false;
          continue;
        }
        out += mdTok("", ch);
        i++;
      }
      return out;
    })
    .join("\n");
}

/** JSON を色分けする（\"…\": のキーは .tok-key・true/false/null は .tok-kw）。 */
function mdScanJson(src) {
  let out = "";
  let i = 0;
  const n = src.length;
  while (i < n) {
    const ch = src[i];
    if (ch === '"') {
      let j = i + 1;
      while (j < n && src[j] !== '"') {
        if (src[j] === "\\") j++;
        j++;
      }
      const end = Math.min(j + 1, n);
      let k = end;
      while (k < n && (src[k] === " " || src[k] === "\t")) k++;
      out += mdTok(src[k] === ":" ? "tok-key" : "tok-str", src.slice(i, end));
      i = end;
      continue;
    }
    const word = /^(?:true|false|null)\b/.exec(src.slice(i));
    if (word) {
      out += mdTok("tok-kw", word[0]);
      i += word[0].length;
      continue;
    }
    if (/[0-9]/.test(ch)) {
      const m = MD_NUM.exec(src.slice(i));
      if (m) {
        out += mdTok("tok-num", m[0]);
        i += m[0].length;
        continue;
      }
    }
    out += mdTok("", ch);
    i++;
  }
  return out;
}

/** YAML / TOML を色分けする（行頭のキーが .tok-key）。 */
function mdScanYaml(src) {
  return src
    .split("\n")
    .map((line) => {
      // 行頭の key: または key = を先に取る
      const m = /^(\s*(?:-\s+)?)([A-Za-z0-9_.\-"' ]+?)(\s*[:=])/.exec(line);
      if (m) {
        const rest = line.slice(m[0].length);
        const lead = mdTok("", m[1]) + mdTok("tok-key", m[2]) + mdTok("", m[3]);
        return lead + mdScanYamlLine(rest);
      }
      return mdScanYamlLine(line);
    })
    .join("\n");
}

// YAML / TOML の 1 行（key: を抜いたあと）
function mdScanYamlLine(line) {
  let out = "";
  let i = 0;
  const n = line.length;
  while (i < n) {
    const ch = line[i];
    if (ch === "#") {
      out += mdTok("tok-com", line.slice(i));
      break;
    }
    if (ch === '"' || ch === "'") {
      let j = i + 1;
      while (j < n && line[j] !== ch) j++;
      const end = Math.min(j + 1, n);
      out += mdTok("tok-str", line.slice(i, end));
      i = end;
      continue;
    }
    if (/[0-9]/.test(ch)) {
      const m = MD_NUM.exec(line.slice(i));
      if (m) {
        out += mdTok("tok-num", m[0]);
        i += m[0].length;
        continue;
      }
    }
    const word = /^(?:true|false|null|yes|no)\b/.exec(line.slice(i));
    if (word) {
      out += mdTok("tok-kw", word[0]);
      i += word[0].length;
      continue;
    }
    out += mdTok("", ch);
    i++;
  }
  return out;
}

/** diff を色分けする（+ の行・- の行・@@ の行）。 */
function mdScanDiff(src) {
  return src
    .split("\n")
    .map((line) => {
      if (/^@@/.test(line)) return mdTok("tok-key", line);
      if (/^\+\+\+|^---/.test(line)) return mdTok("tok-key", line); // ファイル名の行
      if (line.startsWith("+")) return mdTok("tok-add", line);
      if (line.startsWith("-")) return mdTok("tok-del", line);
      return mdTok("", line);
    })
    .join("\n");
}

/**
 * コードに色を付ける（純関数・HTML を返す・中身は必ず mdEscape を通す）。
 * 知らない言語・言語なしなら色を付けずにエスケープだけする。
 */
function mdHighlight(code, lang) {
  const src = String(code == null ? "" : code);
  const key = String(lang == null ? "" : lang).trim().toLowerCase();
  const kind = Object.prototype.hasOwnProperty.call(MD_LANG_KIND, key) ? MD_LANG_KIND[key] : "";
  if (!kind) return mdEscape(src);
  if (kind === "clike") return mdScanClike(src);
  if (kind === "py") return mdScanPy(src);
  if (kind === "sh") return mdScanSh(src);
  if (kind === "json") return mdScanJson(src);
  if (kind === "yaml") return mdScanYaml(src);
  if (kind === "diff") return mdScanDiff(src);
  return mdEscape(src);
}

// --- 長いコードの枠（U4） ---
// 21 行以上のときははじめ 12 行だけ出して「全部見る」で開く（U4 §2）
const MD_CODE_LONG_LINES = 21;
const MD_CODE_PEEK_LINES = 12;

/** コードの枠（長いものは「全部見る」のボタンを付ける）。 */
function renderCodeBlock(bodyLines, lang) {
  const code = bodyLines.join("\n");
  const dataLang = lang ? ` data-lang="${mdEscape(lang)}"` : "";
  const inner = mdHighlight(code, lang);
  const pre = `<pre class="md-pre"${dataLang}><code>${inner}</code></pre>`;
  if (bodyLines.length < MD_CODE_LONG_LINES) return pre;
  const btn =
    `<button type="button" class="md-code-more" data-md-code-toggle ` +
    `aria-expanded="false">全部見る（${bodyLines.length} 行）</button>`;
  return (
    `<div class="md-code md-code-folded" data-md-code-peek="${MD_CODE_PEEK_LINES}" ` +
    `data-md-lines="${bodyLines.length}">` +
    pre +
    btn +
    `</div>`
  );
}

// --- 道具の行（全部たたむ / 全部開く / 目次） ---
/** 見出しの行だけを取り出す（道具の行の目次と、本文の節の判定に使う）。 */
function mdHeadingList(blocks) {
  return blocks.filter((b) => b.type === "heading");
}

/** 道具の行と見出しの目次（見出しが 2 つ以上のときだけ付ける）。 */
function renderToolbar(blocks) {
  const heads = mdHeadingList(blocks);
  if (heads.length < 2) return "";
  const rows = heads
    .map(
      (h, i) =>
        `<button type="button" class="md-toc-row md-lv${h.level}" data-md-goto="${i}">` +
        `${mdInline(h.text)}</button>`
    )
    .join("");
  return (
    `<div class="md-toolbar">` +
    `<button type="button" class="sai-btn small ghost" data-md-all="fold">全部たたむ</button>` +
    `<button type="button" class="sai-btn small ghost" data-md-all="open">全部開く</button>` +
    `<button type="button" class="sai-btn small ghost" data-md-toc aria-expanded="false">目次</button>` +
    `<div class="md-toc" hidden>${rows}</div>` +
    `</div>`
  );
}

// --- 入口 ---

/**
 * md の文字を整形表示用の HTML にする。
 * 文字は全部エスケープしてから組み立てる（文書中の HTML は文字として出る）。
 * opts.toolbar が true なら、見出しが 2 つ以上のときだけ先頭に道具の行を足す。
 */
function renderMarkdown(src, opts) {
  const o = opts || {};
  const text = String(src == null ? "" : src)
    .replace(/\r\n?/g, "\n")
    .replace(/\u0000/g, "\uFFFD");
  const parts = splitFrontmatter(text);
  const front = parts.front ? renderFrontmatter(parts.front) : "";
  const blocks = parseBlocks(parts.body.split("\n"));
  const body = wrapSections(blocks);
  const toolbar = o.toolbar ? renderToolbar(blocks) : "";
  const inner = front + toolbar + body;
  if (!inner) return ""; // 中身が無いときは空（今までと同じ）
  return `<div class="md-doc">${inner}</div>`;
}

// --- 見出しのたたみ / 開き・道具の行（document に 1 回だけ付け，状態は保存しない） ---
if (typeof document !== "undefined" && typeof window !== "undefined" && !window.__saiMdFold) {
  window.__saiMdFold = true;
  const toggle = (node) => {
    const sec = node.parentElement;
    if (sec && sec.classList.contains("md-sec")) sec.classList.toggle("md-folded");
  };
  // 道具の行は「一番近い .md-doc の中だけ」に効かせる（1 つの文書に閉じる）
  const docOf = (node) => node.closest(".md-doc");
  document.addEventListener("click", (e) => {
    const t = e.target;
    if (!t || typeof t.closest !== "function") return;

    // 1) 全部たたむ / 全部開く
    const all = t.closest("[data-md-all]");
    if (all) {
      const doc = docOf(all);
      if (!doc) return;
      const fold = all.getAttribute("data-md-all") === "fold";
      doc.querySelectorAll(".md-sec").forEach((sec) => sec.classList.toggle("md-folded", fold));
      return;
    }

    // 2) 目次の出し入れ
    const tocBtn = t.closest("[data-md-toc]");
    if (tocBtn) {
      const doc = docOf(tocBtn);
      if (!doc) return;
      const box = tocBtn.parentElement ? tocBtn.parentElement.querySelector(".md-toc") : null;
      if (!box) return;
      const open = box.hidden;
      box.hidden = !open;
      tocBtn.setAttribute("aria-expanded", open ? "true" : "false");
      return;
    }

    // 3) 目次の 1 行（その見出しの祖先を開いてからそこへ移動）
    const row = t.closest("[data-md-goto]");
    if (row) {
      const doc = docOf(row);
      if (!doc) return;
      const heads = doc.querySelectorAll(".md-h[data-md-fold]");
      const target = heads[Number(row.getAttribute("data-md-goto"))];
      if (!target) return;
      let n = target.parentElement;
      while (n && n !== doc) {
        if (n.classList && n.classList.contains("md-sec")) n.classList.remove("md-folded");
        n = n.parentElement;
      }
      if (target.scrollIntoView) target.scrollIntoView({ block: "start" });
      return;
    }

    // 4) 長いコードの「全部見る / たたむ」
    const more = t.closest("[data-md-code-toggle]");
    if (more) {
      const box = more.closest(".md-code");
      if (!box) return;
      const open = box.classList.toggle("md-code-open");
      box.classList.toggle("md-code-folded", !open);
      more.setAttribute("aria-expanded", open ? "true" : "false");
      more.textContent = open ? "たたむ" : `全部見る（${box.getAttribute("data-md-lines") || ""} 行）`;
      return;
    }

    if (t.closest("a")) return; // リンクは開くため見出しはたたまない
    const h = t.closest("[data-md-fold]");
    if (h) toggle(h);
  });
  document.addEventListener("keydown", (e) => {
    if (e.key !== "Enter" && e.key !== " ") return;
    const t = e.target;
    if (!t || typeof t.closest !== "function") return;
    const h = t.closest("[data-md-fold]");
    if (h) { e.preventDefault(); toggle(h); }
  });
}

// --- node からの読み込み（tests/test_markdown.js） ---
if (typeof module !== "undefined" && module.exports) {
  module.exports = { renderMarkdown, mdHighlight, MD_LANG_KIND, MD_CODE_LONG_LINES, MD_CODE_PEEK_LINES };
}
