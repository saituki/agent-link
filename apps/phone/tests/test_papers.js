"use strict";
// Papers タブの DOM を使わない関数の小さなテスト（assert だけ）・07 §5・18 §6。
// 実行:
//     node tests/test_papers.js

const assert = require("assert");
const path = require("path");

const {
  authorShort,
  filterPapers,
  paperHash,
  parsePaperHash,
  logHash,
  parseLogHash,
  parseLibraryHash,
  migrateTreeFolds,
  migrateDiscoverLast,
  activityHash,
  papersDbLabel,
  papersFailedLines,
  papersAllFailed,
  papersAttentionHash,
  papersDebtCount,
  paperCardText,
} = require(path.join(
  __dirname,
  "..",
  "src",
  "sai_phone",
  "static",
  "papers.js"
));

function testAuthorShort() {
  // 07 §4.7 の表示例（最初の 1 人＋「ほか」）
  assert.strictEqual(authorShort("A, B, C"), "A ほか");
  assert.strictEqual(authorShort("A"), "A");
  assert.strictEqual(authorShort(""), "");
  // 余分な空白があっても同じ動き
  assert.strictEqual(authorShort("  A ,  B  "), "A ほか");
  // 無いとき（壊れたデータでも画面が崩れない）
  assert.strictEqual(authorShort(null), "");
  assert.strictEqual(authorShort(undefined), "");
}

function testFilterPapers() {
  const papers = [
    {
      id: 1,
      title: "Squeeze the Soaked Sponge: Efficient Finetuning",
      authors: "Jing Liang, Hongyao Tang",
    },
    { id: 2, title: "Attention Is All You Need", authors: "Ashish Vaswani, Noam Shazeer" },
    { id: 3, title: "Retrieval Augmented Generation", authors: "Jane Wang" },
    { id: 4, title: "SpongeNet: Soaking Up Data", authors: "Li Wang" },
  ];
  const ids = (q) => filterPapers(papers, q).map((p) => p.id);

  // 空の言葉は全部（新しい配列が返る）
  assert.deepStrictEqual(ids(""), [1, 2, 3, 4]);
  assert.deepStrictEqual(ids("   "), [1, 2, 3, 4]);

  // 全語が含まれる論文だけ（07 §4.7: "sponge wang"）
  assert.deepStrictEqual(ids("sponge wang"), [4]);

  // 大文字小文字は無視
  assert.deepStrictEqual(ids("SPONGE WANG"), [4]);
  assert.deepStrictEqual(ids("attention"), [2]);

  // 1 語でも題名・著者の両方を見る（著者にだけある言葉でも拾う）
  assert.deepStrictEqual(ids("vaswani"), [2]);
  assert.deepStrictEqual(ids("wang"), [3, 4]);

  // 一致なしは空
  assert.deepStrictEqual(ids("存在しない言葉"), []);

  // 余分な空白は区切りとして扱う（1 語目と 2 語目はどちらも paper 1 にしか無い）
  assert.deepStrictEqual(ids("  sponge   efficient "), [1]);

  // 入力が壊れていても動く
  assert.deepStrictEqual(filterPapers(null, "x"), []);
}

// 住所の読み書き（新旧・計画 18 §3.5・§6）
function testPaperHash() {
  // 書き方: 新しい形（名前あり）と古い形（名前なし＝選んでいるプロジェクトで開く）
  assert.strictEqual(paperHash("sai-paper", 30), "#/paper/sai-paper/30");
  assert.strictEqual(paperHash("sai-paper", 30, true), "#/paper/sai-paper/30/text");
  assert.strictEqual(paperHash(null, 30), "#/paper/30");
  assert.strictEqual(paperHash(null, 30, true), "#/paper/30/text");

  // 読み方: 古い形（ホーム画面の履歴や古い通知から来る）
  assert.deepStrictEqual(parsePaperHash("#/paper/30"), { name: null, id: "30", text: false });
  assert.deepStrictEqual(parsePaperHash("#/paper/30/text"), { name: null, id: "30", text: true });
  // 読み方: 新しい形
  assert.deepStrictEqual(parsePaperHash("#/paper/sai-paper/30"), {
    name: "sai-paper",
    id: "30",
    text: false,
  });
  assert.deepStrictEqual(parsePaperHash("#/paper/my-proj-2/5/text"), {
    name: "my-proj-2",
    id: "5",
    text: true,
  });

  // 通さない形（1段が数字でない・部品が足りない・Papers の一覧の住所）
  assert.strictEqual(parsePaperHash("#/paper/abc"), null);
  assert.strictEqual(parsePaperHash("#/paper/sai-paper"), null); // 名前だけは旧形と解釈できない
  assert.strictEqual(parsePaperHash("#/paper/sai-paper/text"), null);
  assert.strictEqual(parsePaperHash("#/paper"), null);
  assert.strictEqual(parsePaperHash("#/papers/library"), null);
  assert.strictEqual(parsePaperHash("#/"), null);
  assert.strictEqual(parsePaperHash(null), null);

  // 書いて読んで同じ
  assert.deepStrictEqual(parsePaperHash(paperHash("x_y.z", 7, true)), {
    name: "x_y.z",
    id: "7",
    text: true,
  });
}

function testLogHash() {
  // 書き方: 新旧
  assert.strictEqual(logHash(null, 12), "#/papers/log/12");
  assert.strictEqual(logHash(null, 12, 3), "#/papers/log/12/3");
  assert.strictEqual(logHash("sai-paper", 12), "#/papers/log/sai-paper/12");
  assert.strictEqual(logHash("sai-paper", 12, 3), "#/papers/log/sai-paper/12/3");

  // 読み方: 古い形（1段，数字2段は旧 <log_id>/<position> とみなす）
  assert.deepStrictEqual(parseLogHash("#/papers/log/12"), {
    name: null,
    id: "12",
    position: null,
  });
  assert.deepStrictEqual(parseLogHash("#/papers/log/12/3"), {
    name: null,
    id: "12",
    position: "3",
  });
  // 読み方: 新しい形（1段目が数字でない2段，3段は常に新）
  assert.deepStrictEqual(parseLogHash("#/papers/log/sai-paper/12"), {
    name: "sai-paper",
    id: "12",
    position: null,
  });
  assert.deepStrictEqual(parseLogHash("#/papers/log/sai-paper/12/3"), {
    name: "sai-paper",
    id: "12",
    position: "3",
  });
  assert.deepStrictEqual(parseLogHash("#/papers/log/12/3/4"), {
    name: "12",
    id: "3",
    position: "4",
  });

  // 通さない形（1段が数字でない・番号でない部品）
  assert.strictEqual(parseLogHash("#/papers/log/abc"), null);
  assert.strictEqual(parseLogHash("#/papers/log/sai-paper/x"), null);
  assert.strictEqual(parseLogHash("#/papers/log/sai-paper/12/x"), null);
  assert.strictEqual(parseLogHash("#/papers/discover"), null);

  // 書いて読んで同じ
  assert.deepStrictEqual(parseLogHash(logHash("a-1.b", 9, 2)), {
    name: "a-1.b",
    id: "9",
    position: "2",
  });
}

// 設定の読み替え（古い形は sai-paper プロジェクトの分として読む・計画 18 §3.6・§6）
function testSettingsMigration() {
  // 階層の開閉: 古い形（配列）→ プロジェクトごとの形
  assert.deepStrictEqual(migrateTreeFolds(["query:12", "import:2026-09-01"]), {
    "sai-paper": ["query:12", "import:2026-09-01"],
  });
  // 古い形に紛れた文字列以外は落とす
  assert.deepStrictEqual(migrateTreeFolds(["query:1", 5, null]), { "sai-paper": ["query:1"] });
  // 新しい形はそのまま
  const folds = { "sai-paper": ["query:1"], other: [] };
  assert.deepStrictEqual(migrateTreeFolds(folds), folds);
  // 壊れた値は空
  assert.deepStrictEqual(migrateTreeFolds(null), {});
  assert.deepStrictEqual(migrateTreeFolds("x"), {});
  assert.deepStrictEqual(migrateTreeFolds(3), {});

  // Discover の最後の検索: 古い形（数字／数字の文字列）→ プロジェクトごとの形
  assert.deepStrictEqual(migrateDiscoverLast(12), { "sai-paper": 12 });
  assert.deepStrictEqual(migrateDiscoverLast("12"), { "sai-paper": 12 });
  // 新しい形はそのまま
  assert.deepStrictEqual(migrateDiscoverLast({ other: 3 }), { other: 3 });
  // 壊れた値は空（配列は使わない形）
  assert.deepStrictEqual(migrateDiscoverLast(null), {});
  assert.deepStrictEqual(migrateDiscoverLast("abc"), {});
  assert.deepStrictEqual(migrateDiscoverLast([1, 2]), {});
}

function testActivityHash() {
  // Radio の再生・質問は再生画面へ，論文のある出来事は論文情報へ，それ以外は押せない
  assert.strictEqual(
    activityHash({ kind: "radio_play", radio_id: "sai-paper-30", paper_id: 30 }, "sai-paper"),
    "#/radio/sai-paper-30"
  );
  assert.strictEqual(
    activityHash({ kind: "radio_question", radio_id: "sai-paper-30" }, "sai-paper"),
    "#/radio/sai-paper-30"
  );
  assert.strictEqual(
    activityHash({ kind: "download", paper_id: 7 }, "sai-paper"),
    paperHash("sai-paper", 7, false)
  );
  assert.strictEqual(activityHash({ kind: "discover", paper_id: null }, "sai-paper"), null);
  assert.strictEqual(activityHash({ kind: "notes", paper_id: 7 }, null), null);
  assert.strictEqual(activityHash(null, "sai-paper"), null);
}

function testFailedLines() {
  // 429（混雑）→ 再検索の案内
  const busy = papersFailedLines([
    { db: "s2", reason: "search error: invalid S2 response: S2 status 429 (rate limited)" },
  ]);
  assert.strictEqual(busy.length, 1, busy);
  assert.ok(busy[0].startsWith("s2 は混雑していて結果を取れませんでした"), busy[0]);
  assert.ok(busy[0].includes("もう一度検索"), busy[0]);
  // 403（キー不行）→ paper.env の案内
  const key = papersFailedLines([{ db: "s2", reason: "search error: 403 Forbidden" }]);
  assert.ok(key[0].includes("API キーが使えませんでした"), key[0]);
  assert.ok(key[0].includes("paper.env"), key[0]);
  // それ以外 → 理由の先頭 120 字
  const other = papersFailedLines([{ db: "openalex", reason: "network error: 接続できません" }]);
  assert.strictEqual(other[0], "openalex で検索できませんでした: network error: 接続できません");
  // 理由は先頭 120 字まで（文全体はその前置き 19 字 + 120）
  const long = papersFailedLines([{ db: "arxiv", reason: "x".repeat(300) }]);
  assert.ok(long[0].length <= 19 + 120, String(long[0].length));
  // 壊れた値でも止まらない
  assert.deepStrictEqual(papersFailedLines([]), []);
  assert.deepStrictEqual(papersFailedLines(null), []);
  assert.strictEqual(papersFailedLines([null, { db: "", reason: "失敗" }]).length, 1);
  // 失敗が 2 つなら 2 行
  const two = papersFailedLines([
    { db: "s2", reason: "429" },
    { db: "pubmed", reason: "timeout" },
  ]);
  assert.strictEqual(two.length, 2, two);
}

function testAllFailed() {
  // 選んだ検索先すべてが失敗したときだけ true（「見つかりません」を出さない）
  const failed = [{ db: "s2", reason: "429" }];
  assert.strictEqual(papersAllFailed(failed, "s2"), true);
  assert.strictEqual(papersAllFailed(failed, "s2,openalex"), false);
  // 2 つとも失敗なら true
  assert.strictEqual(
    papersAllFailed([...failed, { db: "openalex", reason: "403" }], "s2,openalex"),
    true
  );
  // 検索先が分からないときは決めない（通常の「見つかりません」を出す）
  assert.strictEqual(papersAllFailed(failed, ""), false);
  assert.strictEqual(papersAllFailed(failed, undefined), false);
  // 失敗が無ければ false
  assert.strictEqual(papersAllFailed([], "s2"), false);
  assert.strictEqual(papersAllFailed(null, "s2"), false);
}

function testDbLabel() {
  assert.strictEqual(papersDbLabel("s2"), "s2");
  assert.strictEqual(papersDbLabel(" openalex "), "openalex");
  assert.strictEqual(papersDbLabel(""), "Source");
  assert.strictEqual(papersDbLabel(null), "Source");
}

function testAttentionHash() {
  // `sci_decision` のキーはコアが `sci_decision:<Q の ID>` の形で作る
  assert.strictEqual(papersAttentionHash({ kind: "sci_decision", key: "sci_decision:Q12" }), "#/papers/tree/Q12");
  assert.strictEqual(papersAttentionHash({ kind: "sci_triage", key: "sci_triage:P55" }), "#/papers/triage");
  assert.strictEqual(papersAttentionHash({ kind: "sci_decision", key: "sci_decision:N12:0" }), "");
  assert.strictEqual(papersAttentionHash({ kind: "sci_decision", key: "sci_decision:12" }), "");
  assert.strictEqual(papersAttentionHash({ kind: "sci_decision", key: "" }), "");
  assert.strictEqual(papersAttentionHash({ kind: "question", key: "N12" }), "");
  assert.strictEqual(papersAttentionHash({}), "");
  assert.strictEqual(papersAttentionHash(null), "");
}

function testDebtCount() {
  assert.strictEqual(papersDebtCount([{ kind: "action", debt: true }, { kind: "action", debt: false }, { kind: "notes", debt: true }]), 1);
  assert.strictEqual(papersDebtCount(null), 0);
  assert.strictEqual(activityHash({ kind: "action", paper_id: null }, "sai-paper"), null);
}

function testPaperCardText() {
  // 題名・年・DOI はある物だけ書く（計画 U10 の例）
  assert.strictEqual(
    paperCardText({ title: "Attention Is All You Need", year: 2017, doi: "10.1000/xyz" }, 12),
    "対象の論文: #12「Attention Is All You Need」（2017・DOI 10.1000/xyz）"
  );
  assert.strictEqual(paperCardText({ title: "A" }, 3), "対象の論文: #3「A」");
  assert.strictEqual(paperCardText({ doi: "10.1" }, 4), "対象の論文: #4「(無題)」（DOI 10.1）");
  assert.strictEqual(paperCardText({}, 5), "対象の論文: #5「(無題)」");
  // 実行ファイルの名前は書かない（名前は最後のガイドで変わるため）
  assert.ok(!paperCardText({ title: "A" }, 1).includes("sai-moon"));
}

function main() {
  testDebtCount();
  testActivityHash();
  testAuthorShort();
  testFilterPapers();
  testPaperHash();
  testLogHash();
  testSettingsMigration();
  testFailedLines();
  testAllFailed();
  testDbLabel();
  testAttentionHash();
  testPaperCardText();
  console.log("test_papers: すべて通りました");
}

main();

// Library の住所（論文を右の欄に開く形も読める。U03 で find を追加）
assert.deepStrictEqual(parseLibraryHash("#/papers/library"), { paper: null, log: null, find: null });
assert.deepStrictEqual(parseLibraryHash("#/papers/library/57"), { paper: 57, log: null, find: null });
assert.deepStrictEqual(parseLibraryHash("#/papers/library/log/12"), { paper: null, log: 12, find: null });
assert.deepStrictEqual(parseLibraryHash("#/papers/library/find/12"), { paper: null, log: null, find: 12 });
assert.strictEqual(parseLibraryHash("#/papers/library/log/0"), null);
assert.strictEqual(parseLibraryHash("#/papers/library/find/0"), null);
assert.strictEqual(parseLibraryHash("#/papers/library/0"), null);
assert.strictEqual(parseLibraryHash("#/papers/discover"), null);
