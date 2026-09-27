"use strict";
// md 整形表示（static/markdown.js・計画 13 §5）の小さなテスト（assert だけ）。
// 実行:
//     node tests/test_markdown.js

const assert = require("assert");
const path = require("path");

const { renderMarkdown, mdHighlight, MD_LANG_KIND, MD_CODE_LONG_LINES, MD_CODE_PEEK_LINES } = require(path.join(
  __dirname,
  "..",
  "src",
  "sai_phone",
  "static",
  "markdown.js"
));

// 見出し 6 段階と，見出しの節が <section> で包まれること（たたみのため）
function testHeadings() {
  const html = renderMarkdown("# 大\n\n## 中\n\n### 小\n\n#### 4\n\n##### 5\n\n###### 6");
  for (const lv of [1, 2, 3, 4, 5, 6]) {
    assert.ok(html.includes(`<h${lv} class="md-h md-lv${lv}"`), `h${lv} が出ないこと`);
    assert.ok(html.includes(`</h${lv}>`), `h${lv} の閉じる札`);
  }
  assert.ok(html.includes('data-md-fold'), "見出しはタップできる印を持つ");
  // 6 段階の見出しは 6 つの節（入れ子）になり，全部閉じている
  assert.strictEqual((html.match(/<section class="md-sec">/g) || []).length, 6);
  assert.strictEqual((html.match(/<\/section>/g) || []).length, 6);
  // 中身は直前の見出しの節の中に入る
  const html2 = renderMarkdown("# 見出\n\n中身");
  const h = html2.indexOf("</h1>");
  assert.ok(html2.indexOf("中身", h) > h, "中身が見出しのあと");
  assert.ok(html2.indexOf("</section>") > html2.indexOf("中身"), "中身が節の中");

  // 行末の ### は落とす
  assert.ok(renderMarkdown("## タイトル ##").includes(">タイトル<"));
}

// 段落・太字 / 斜体・インラインコード・段落の改行
function testInlineAndParagraph() {
  const html = renderMarkdown("**太** と *斜体* と `x < y`\n二行目");
  assert.ok(html.includes("<strong>太</strong>"));
  assert.ok(html.includes("<em>斜体</em>"));
  assert.ok(html.includes("<code>x &lt; y</code>"));
  assert.ok(html.includes("<br>二行目"), "同じ段落の中の行は <br> でつなぐ");
  assert.ok(html.includes('<p class="md-p">'), "段落の枠");

  // 空の入力でも落ちない
  assert.strictEqual(renderMarkdown(""), "");
  assert.strictEqual(renderMarkdown(null), "");
  assert.strictEqual(renderMarkdown(undefined), "");
}

// 入れ子・番号付き・チェックボックスの箇条書き
function testLists() {
  const nested = renderMarkdown("- a\n  - b\n    - c");
  assert.ok(nested.includes('<ul class="md-ul">'));
  assert.strictEqual((nested.match(/<ul /g) || []).length, 3, "入れ子で3つ");
  // 入れ子は item の中に入る（li の中にさらに ul）
  assert.ok(/<li class="md-li">a<ul /.test(nested), "a の item の中に入れ子");

  const ordered = renderMarkdown("1. い\n2. う");
  assert.ok(ordered.includes('<ol class="md-ol">'));
  assert.ok(ordered.includes("<li"), "番号付き item");
  assert.ok(!ordered.includes("<ul"), "番号付きは ul にしない");

  const task = renderMarkdown("- [x] 済んだ\n- [ ] 未");
  assert.ok(task.includes('type="checkbox"'), "チェックボックス");
  assert.ok(task.includes("checked"), "x は付いた状態");
  assert.strictEqual((task.match(/type="checkbox"/g) || []).length, 2);
}

// 表（はみ出す分だけ横スクロールする枠付き）
function testTable() {
  const html = renderMarkdown("| 名前 | 値 |\n| :-- | --: |\n| a | 1 |\n| b | 2 |");
  assert.ok(html.includes('<div class="md-tbl-wrap">'), "横スクロールの枠");
  assert.ok(html.includes('<table class="md-table">'));
  assert.ok(html.includes("<thead>"));
  assert.ok(html.includes("<th"), "見出しセル");
  assert.strictEqual((html.match(/<td/g) || []).length, 4, "データセル 2×2");
  assert.ok(html.includes('style="text-align:left"'), "左揃えの指定");
  assert.ok(html.includes('style="text-align:right"'), "右揃えの指定");
  // 表のセルの中もエスケープされる
  const esc = renderMarkdown("| x |\n| --- |\n| <b>1</b> |");
  assert.ok(esc.includes("&lt;b&gt;1&lt;/b&gt;"));
}

// コードの枠（```lang … ```）と中の文字のエスケープ
function testCodeFrame() {
  const html = renderMarkdown("```js\nconst a = '<b>';\n```");
  assert.ok(html.includes('<pre class="md-pre"'), "コードの枠");
  assert.ok(html.includes('data-lang="js"'), "言語の印");
  assert.ok(html.includes("&lt;b&gt;"), "コードの中の < は文字");
  assert.ok(!html.includes("<b>"), "コードの中が HTML にならない");

  // 閉じる ``` が無くても最後までコードとして扱う
  const open = renderMarkdown("```\na\nb");
  assert.ok(open.includes("<pre"));
  assert.ok(open.includes("a\nb"));
}

// 先頭の frontmatter は「設定」の表としてたたんで出る
function testFrontmatter() {
  const html = renderMarkdown("---\ntitle: 実装報告\nround: 3\n---\n\n# 本文");
  // 中身の前には .md-doc の入れ物（C-F4・道具の行はここの中でだけ働く）
  assert.ok(html.startsWith('<div class="md-doc">'), "md-doc で包まれること");
  assert.ok(html.endsWith("</div>"), "md-doc が閉じていること");
  assert.ok(html.indexOf("<details class=\"md-front\">") < html.indexOf("# 本文".replace("# ", "")), "本文の前に出る");
  assert.ok(html.includes("<summary>設定</summary>"), "「設定」という見出し");
  assert.ok(html.includes("<table class=\"md-front-table\">"));
  assert.ok(html.includes("<th>title</th>"));
  assert.ok(html.includes("<td>実装報告</td>"));
  assert.ok(html.includes("<th>round</th>"));
  // 閉じる --- が無いとき frontmatter にしない
  assert.ok(!renderMarkdown("---\ntitle: x").includes("md-front"));
}

// 文書中の HTML は実行せず文字として出す
function testHtmlIsText() {
  const html = renderMarkdown("<script>alert(1)</script>\n\n<img src=x onerror=alert(1)>");
  assert.ok(!html.includes("<script>"), "script タグが出ないこと");
  assert.ok(!html.includes("<img"), "img タグが出ないこと");
  assert.ok(html.includes("&lt;script&gt;"), "文字として出る");
  // 見出しや太字の中でも同じ
  const inHeading = renderMarkdown("# <b>x</b>");
  assert.ok(inHeading.includes("&lt;b&gt;"));
}

// リンクは http/https だけ有効（危ない接続辞書は文字のまま）
function testLinks() {
  const html = renderMarkdown(
    "[怪しい](javascript:alert(1))\n\n[無効](JaVaScRiPt:alert(1))\n\n[データ](data:text/html,x)\n\n[安全](https://example.com/a?b=1&c=2)"
  );
  assert.ok(!html.includes('href="javascript'), "javascript: はリンクにしない");
  assert.ok(!html.includes('href="JaVaScRiPt'), "大文字でも同じ");
  assert.ok(!html.includes('href="data:'), "data: もリンクにしない");
  assert.ok(html.includes('href="https://example.com/a?b=1&amp;c=2"'), "http(s) はリンクになる");
  assert.ok(html.includes('rel="noopener noreferrer"'), "別画面で開く設定");
  assert.ok(html.includes("[怪しい]"), "無効なリンクは文字のまま残る");
}

// 引用と区切り線
function testQuoteAndRule() {
  const html = renderMarkdown("> 引用の一行\n> 二行目\n\n---");
  assert.ok(html.includes('<blockquote class="md-quote">'));
  assert.ok(html.includes("引用の一行"));
  assert.ok(html.indexOf("引用の一行") < html.indexOf("</blockquote>"));
  assert.ok(html.includes("<hr>"), "区切り線");
}

// --- U4 で足したもののテスト ---

// コードの色分け（mdHighlight・言語ごと）
function testHighlight() {
  // js: キーワード・文字列・数・行コメント・複数行注釈・関数名
  const js = mdHighlight("const a = 1; // 注\nf('<b>');", "js");
  assert.ok(js.includes('<span class="tok-kw">const</span>'), "キーワード");
  assert.ok(js.includes('<span class="tok-num">1</span>'), "数");
  assert.ok(js.includes('<span class="tok-com">// 注</span>'), "行コメント");
  assert.ok(js.includes('<span class="tok-fn">f</span>'), "関数名");
  assert.ok(js.includes("&lt;b&gt;"), "文字列の中の < は文字");
  assert.ok(!js.includes("<b>"), "生のタグは出さない");
  // 複数行注釈は行をまたいでも色が付く
  const cm = mdHighlight("/* 1\n   2 */\nlet x", "javascript");
  assert.ok(cm.includes('<span class="tok-com">/* 1\n   2 */</span>'), "複数行注釈");
  assert.ok(cm.includes('<span class="tok-kw">let</span>'), "注釈のあとキーワード");

  // py: キーワード・文字列（3 引用符）・数・# の注釈
  const py = mdHighlight("def f():\n    return 42  # 答\ns = '''a'''", "python");
  assert.ok(py.includes('<span class="tok-kw">def</span>'), "py のキーワード");
  assert.ok(py.includes('<span class="tok-num">42</span>'), "py の数");
  assert.ok(py.includes('<span class="tok-com"># 答</span>'), "py の注釈");
  // 3 引用符の文字列（' は & #39; に退避される）
  assert.ok(py.includes(`<span class="tok-str">${"&#39;&#39;&#39;"}a${"&#39;&#39;&#39;"}</span>`), "py の 3 引用符");

  // sh: 行頭の語・--オプション・文字列・# の注釈
  const sh = mdHighlight("git push --force origin main  # 送る", "bash");
  assert.ok(sh.includes('<span class="tok-fn">git</span>'), "sh のコマンド名");
  assert.ok(sh.includes('<span class="tok-key">--force</span>'), "sh のオプション");
  assert.ok(sh.includes('<span class="tok-com"># 送る</span>'), "sh の注釈");
  // 行の先頭の語だけ tok-fn（origin main は普通の文字）
  const sh2 = mdHighlight("git push origin", "sh");
  assert.ok(sh2.includes('<span class="tok-fn">git</span>'));
  assert.ok(sh2.includes("origin"), "引数の文字が残る");

  // json: キー・文字列・数・true / false / null
  const json = mdHighlight('{"a": "b", "n": 1, "ok": true}', "json");
  assert.ok(json.includes('<span class="tok-key">&quot;a&quot;</span>'), "json のキー");
  assert.ok(json.includes('<span class="tok-str">&quot;b&quot;</span>'), "json の文字列");
  assert.ok(json.includes('<span class="tok-num">1</span>'), "json の数");
  assert.ok(json.includes('<span class="tok-kw">true</span>'), "json の true");

  // yaml / toml: キー・注釈
  const yaml = mdHighlight("name: sai  # 名", "yaml");
  assert.ok(yaml.includes('<span class="tok-key">name</span>'), "yaml のキー");
  assert.ok(yaml.includes('<span class="tok-com"># 名</span>'), "yaml の注釈");
  const toml = mdHighlight('key = "v"', "toml");
  assert.ok(toml.includes('<span class="tok-key">key</span>'), "toml のキー");

  // diff / patch: + の行・- の行・@@ の行
  const diff = mdHighlight("@@ -1 +1 @@\n-old\n+new\n same", "diff");
  assert.ok(diff.includes('<span class="tok-key">@@ -1 +1 @@</span>'), "diff の @@ 行");
  assert.ok(diff.includes('<span class="tok-del">-old</span>'), "diff の削除行");
  assert.ok(diff.includes('<span class="tok-add">+new</span>'), "diff の追加行");

  // 知らない言語・言語なしはエスケープだけ
  assert.strictEqual(mdHighlight("<b>x</b>", "brainfuck"), "&lt;b&gt;x&lt;/b&gt;");
  assert.strictEqual(mdHighlight("<b>x</b>", ""), "&lt;b&gt;x&lt;/b&gt;");
  assert.strictEqual(mdHighlight("<b>x</b>"), "&lt;b&gt;x&lt;/b&gt;");
  assert.strictEqual(mdHighlight(null, "js"), "");

  // 定義した言語と本文の言語表がずれていない
  for (const lang of ["js", "py", "sh", "json", "yaml", "diff"]) {
    assert.ok(MD_LANG_KIND[lang], `${lang} の色分けの種類があること`);
  }
}

// 道具の行（全部たたむ・全部開く・目次）
function testToolbar() {
  // 見出し 2 つ以上 → 道具の行が付く
  const two = renderMarkdown("# A\n\n本文\n\n## B\n\n本文", { toolbar: true });
  assert.ok(two.includes('class="md-toolbar"'), "道具の行が付く");
  assert.ok(two.includes("全部たたむ"), "全部たたむ");
  assert.ok(two.includes("全部開く"), "全部開く");
  assert.ok(two.includes("目次"), "目次");
  // 目次の行は見出しの数だけ
  assert.strictEqual((two.match(/data-md-goto=/g) || []).length, 2, "見出し 2 つ");
  // 見出し 1 つでは付かない
  const one = renderMarkdown("# A\n\n本文", { toolbar: true });
  assert.ok(!one.includes("md-toolbar"), "見出し 1 つでは付けない");
  // 見出し 0 つでも付けない
  assert.ok(!renderMarkdown("本文だけ", { toolbar: true }).includes("md-toolbar"));
  // toolbar を指定しなければ道具の行は出さない（今までと同じ画面を壊さない）
  assert.ok(!renderMarkdown("# A\n\n## B").includes("md-toolbar"), "指定なしなら付けない");
}

// 長いコードの枠（21 行以上だけ「全部見る」）
function testLongCode() {
  const lines = (n) => Array.from({ length: n }, (_, i) => `line ${i + 1}`).join("\n");
  const long = renderMarkdown("```js\n" + lines(MD_CODE_LONG_LINES) + "\n```");
  assert.ok(long.includes("md-code-folded"), "長いコードははじめ畳んで出る");
  assert.ok(
    long.includes(`全部見る（${MD_CODE_LONG_LINES} 行）`),
    `「全部見る（${MD_CODE_LONG_LINES} 行）」が付く`
  );
  assert.ok(long.includes(`data-md-code-peek="${MD_CODE_PEEK_LINES}"`), "はじめ何行ぶんかを持つ");
  assert.ok(long.includes(`data-md-lines="${MD_CODE_LONG_LINES}"`), "全部何行かを持つ");

  // 1 行少ないと「全部見る」は付かない
  const short = renderMarkdown("```js\n" + lines(MD_CODE_LONG_LINES - 1) + "\n```");
  assert.ok(!short.includes("md-code-folded"), "20 行は畳まない");
  assert.ok(!short.includes("全部見る"), "20 行に「全部見る」は付かない");
  assert.ok(short.includes('<pre class="md-pre"'), "20 行は今までと同じ枠");
}

// 今までの出力（md-doc を除いて）が壊れていないこと
function testNoToolbarSameAsBefore() {
  const md = "# 見出し\n\n本文 **太** と `x`\n\n- a\n- b\n\n> 引用\n\n| a | b |\n| --- | --- |\n| 1 | 2 |";
  const html = renderMarkdown(md);
  assert.ok(html.startsWith('<div class="md-doc">'), "md-doc は入る");
  assert.ok(html.endsWith("</div>"));
  for (const frag of [
    '<section class="md-sec">',
    'data-md-fold',
    "<strong>太</strong>",
    "<code>x</code>",
    '<ul class="md-ul">',
    '<blockquote class="md-quote">',
    '<div class="md-tbl-wrap">',
  ]) {
    assert.ok(html.includes(frag), `${frag} が残る`);
  }
  // md-doc の中身だけで数える（入れ物のタグだけ増える）
  const inner = html.slice('<div class="md-doc">'.length, -"</div>".length);
  assert.strictEqual(
    inner,
    renderMarkdown(md).slice('<div class="md-doc">'.length, -"</div>".length),
    "中身は変わらない"
  );
}

function main() {
  testHeadings();
  testInlineAndParagraph();
  testLists();
  testTable();
  testCodeFrame();
  testFrontmatter();
  testHtmlIsText();
  testLinks();
  testQuoteAndRule();
  testHighlight();
  testToolbar();
  testLongCode();
  testNoToolbarSameAsBefore();
  console.log("test_markdown: すべて通りました");
}

main();
