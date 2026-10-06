# 修正計画書：research-tree の専門用語を英語の大文字（UPPER_SNAKE）に統一する

- 作成日: 2026-10-01
- 対象: `~/workdir/Sai/sai-moon`（`sai-sci/`・`crates/sai-paper-core`・`crates/sai-paper-cli`・`crates/sai-paper-tui`・`crates/sai-link-widgets`・`apps/phone`・`pi-litbase`）
- 状態: 計画（未実装）。「10. 決めてほしいこと」を決めてから着手する。
- 関連: `docs/20261001-1634-first-message-auto-sections.md`（最初のメッセージと `message.md` の見出しの構造。`sci context` の見出しを両方で触るので、順序に注意する。9 章）

---

## 1. 要約

research-tree のワークフローの専門用語が、`項目`・`候補`・`範囲`・`決定` といった**一般名詞**で書かれている。
同じ語が、専門語としても一般の意味でも使われ、人も agent も文脈から推測するしかない。たとえば `項目` は、ITEM のほかに「評価項目」（60 件）や「抜き出す項目」にも使われている。

そこで、**専門用語をすべて英語の大文字（UPPER_SNAKE）に統一**する。一般名詞としての日本語は、専門語と字面が重ならない語に言い換える。

- 英語名は、**すでにコードにある英語の識別子**（`item`・`option`・`in_progress` など）を大文字にしたもの。新しい語は作らない。
- 画面（Research-tree の Web 画面・TUI・CLI の表示）も、同じ英語名に揃える。
- 利用者や agent が書く**内容**（知見の本文・問い・mind.md・報告の文章）は、日本語のまま。統一するのは**用語**だけ。
- 内部の識別子（op 名・DB の列・API・JSON のキー・ID の接頭辞 `N` `Q` `F` `L` `P` `S`）は変えない。

---

## 2. 決定事項（利用者との確認済み）

| 項目 | 決定 |
|---|---|
| `AGENTS.md` の「画面に出す言葉は日本語にする」 | この変更では適用しない（`sai-moon` 自体の開発ルールのため） |
| 変更の範囲 | agent が読む文章（A 層）と、Research-tree の画面（B 層のうち画面）。内部の識別子（C 層）は変えない |
| 関門の名前 | 専門用語に含める（英語の大文字にする） |
| 表記 | UPPER_SNAKE。**常に大文字** |

---

## 3. 用語集（確定案）

### 3.1 節点の種類と状態

| 日本語 | 英語 | 備考 |
|---|---|---|
| 節点 | NODE | ITEM と OPTION の総称 |
| 項目 | ITEM | 決める穴 |
| 候補 | OPTION | ITEM の下の答えの案 |
| 項目の型 | ITEM_TYPE | CHOOSE（選ぶ）・LIST（並べる）・WRITE（書く）・VERIFY（確かめる） |
| 研究の型 | RESEARCH_TYPE | PROBLEM_DRIVEN・DATA_DRIVEN・METHOD_DRIVEN・PHENOMENON_DRIVEN・IMPROVE_DRIVEN（型の id `problem` `data` `method` `phenomenon` `improve` は変えない） |
| 固定 | FIXED | |
| 未決定 | OPEN | ITEM の状態 |
| 仮決定 | TENTATIVE | 同上 |
| 決定 | DECIDED | 同上 |
| 対象外 | EXCLUDED | 同上 |
| 候補（状態） | CANDIDATE | OPTION の状態 |
| 選択 | SELECTED | 同上 |
| 却下 | REJECTED | 同上 |
| 休眠 | DORMANT | 同上 |

### 3.2 空欄・印・負債

| 日本語 | 英語 |
|---|---|
| 空欄 | BLANK |
| 未着手 | UNTOUCHED |
| 調査中 | IN_PROGRESS |
| 情報不足 | INSUFFICIENT |
| 判断待ち | AWAITING_DECISION |
| 対立 | CONFLICT |
| 印 | MARK |
| 根拠不足 | WEAK_EVIDENCE |
| 要再確認 | RECHECK |
| 負債 | DEBT |
| 仕分けていない提案 | PENDING_PROPOSALS |
| 読んだが反映していない論文 | UNREFLECTED_PAPERS |
| 選別していない検索 | UNSCREENED_SETS |
| 書いていない振り返り | EMPTY_REVIEWS |

### 3.3 問い・知見・結び付き・提案など

| 日本語 | 英語 | 備考 |
|---|---|---|
| 問い | QUESTION | ID は `Q` |
| 回答待ち / 回答済み / 取り下げ | OPEN / ANSWERED / WITHDRAWN | 問いの状態。OPEN は ITEM の OPEN と同じ綴りだが、分類（QUESTION の状態）で区別する |
| 知見 | FINDING | ID は `F` |
| 向き | POLARITY | SUPPORT・REFUTE・NEUTRAL・NONE |
| 出典 | CITE | `cites` に合わせる |
| 結び付き | LINK | ID は `L` |
| 論文の役割 | ROLE | EXEMPLAR・COMPARE・ISSUE_EVIDENCE・METHOD_SOURCE・DATA_SOURCE・BACKGROUND・COUNTER |
| 読み具合 | READ_STATE | UNREAD・READ・REFLECTED・IRRELEVANT |
| 読む深さ | DEPTH | ABSTRACT・FIGURES・FULL |
| 提案 | PROPOSAL | ID は `P` |
| 仕分け | TRIAGE | ACCEPT・REJECT・REVERT |
| 自動で入る提案 | AUTO_OP | |
| 利用者が仕分ける提案 | REVIEW_OP | |
| 墓標 | TOMBSTONE | |
| 種 | SEED | ID は `S` |
| 種の文 | SEED_SENTENCE | |
| 関門 | GATE | 名前は 3.6 |
| 決定の記録 | DECISION_LOG | |
| 行動の履歴 | ACTION_LOG | |
| 文献の流れ | LITERATURE_FLOW | |

### 3.4 範囲と文脈（`sci context` の見出し）

| 日本語 | 英語 |
|---|---|
| 範囲 | SCOPE |
| 節点 / 道 / 部分木 / 兄弟 / 全体の要約 / 目隠し | NODE / PATH / SUBTREE / SIBLINGS / SUMMARY / BLIND |
| 範囲外 | OUT_OF_SCOPE |
| 今回の行動 | CURRENT_ACTION |
| 研究の現在地 | PROGRESS |
| 土壌 | SOIL |
| 対象の節点 | TARGET_NODE |
| 上流の道 | UPSTREAM_PATH |
| 仮置き | ASSUMPTION |
| 項目ガイド | ITEM_GUIDE |
| 既に調べたこと | SEARCH_HISTORY |
| 出せる提案 | ALLOWED_OPS |
| 対象論文 | TARGET_PAPERS |
| 次の一手 | NEXT_ACTION |
| 読み方（目的先行 / 目的後付け） | MODE（PURPOSE_FIRST / BLIND_FIRST） |

### 3.5 カード・画面の区分

| 日本語 | 英語 | 備考 |
|---|---|---|
| カード | CARD | `sci card` で作る task.md。sai-link 一般の「カード」と区別する |
| 読む札 | READ_CARD | `--runner user` のカード |
| 検索セット | SEARCH_SET | |
| 一覧 / 根の図 / 表 / 実行の図 | LIST / ROOT_GRAPH / TABLE / RUN_GRAPH | 表示の切り替え |

### 3.6 関門の名前

関門の名前は型ごとに `sai-sci/types/*.md` の `## 関門` に日本語で書かれている（例 `problem` 型: 出発点・小目的・データの入手・既存手法の網羅・課題の確認・改善の方向・新規性）。
**関門の名前は、判定の条件（`decided(background.goal)` など）から機械的に付ける。** 形は `<対象の key>_<条件>` を UPPER_SNAKE にする。

| 例（`problem` 型） | 条件 | 英語名（案） |
|---|---|---|
| 出発点 | `decided(background.goal)` | BACKGROUND_GOAL_DECIDED |
| 小目的 | `decided(purpose.subgoal)` | PURPOSE_SUBGOAL_DECIDED |
| 既存手法の網羅 | `saturated(existing)` | EXISTING_SATURATED |
| 課題の確認 | `level(issue.limit, 2)` | ISSUE_LIMIT_LEVEL_2 |
| 新規性 | `count(novelty.closest, 3) かつ decided(novelty.diff)` | NOVELTY_CLOSEST_COUNT_3_AND_NOVELTY_DIFF_DECIDED |

- 長くなるものは、関門の定義に英語名を直接書けるようにする（`## 関門` の各行に `<名前>: <条件>` の名前を英語にする）。上の自動の名前は既定にすぎない。実際の名前は、実装のときに型ごとに決める（7 章 段階 3）。
- 項目名（`name:`）と、`mind.md` の見出しの扱いは「10. 決めてほしいこと」。

---

## 4. 書き方の決まり

1. 専門語は、英語の大文字のまま、助詞をつなげて書く。例：「ITEM の状態が DECIDED になる」「OPTION を CANDIDATE から REJECTED にする」。
2. **同じ綴りが複数の分類にあるときは、分類を前に付ける。** 対象は `OPEN`（ITEM と QUESTION）、`BACKGROUND`（ROLE と ITEM の名前）、`NODE`（SCOPE と NODE の総称）。書き方は `ITEM:OPEN`・`QUESTION:OPEN`・`ROLE:BACKGROUND` のように `分類:値`。曖昧さがないときは省いてよい。
3. 一般名詞としての日本語は、専門語と字面が重ならない語にする。
   | 日本語の語 | 衝突する専門語 | 言い換え |
   |---|---|---|
   | 評価項目（E1〜E10） | ITEM | CRITERIA（評価基準） |
   | 比較項目 | ITEM | COMPARISON_CRITERIA |
   | 抜き出す項目 | ITEM | EXTRACT_FIELDS |
   | 候補の手法・候補の論文 | OPTION | 案、または HIT（検索結果） |
   | 範囲（一般の意味） | SCOPE | 文脈に合わせて「対象」「幅」など |
4. 報告（report）でも、専門語は英語の大文字で書く。内容の文章は日本語のまま。
5. 見出しは英語の大文字にする（例 `## ITEM_GUIDE: 名前`）。見出しの中の名前（節点の題名など）は変えない。
6. ID（`N12`・`Q7`・`F31`・`L3`・`P55`・`S1`）と op 名（`add_finding` など）は、そのまま。

---

## 5. 変更する場所

調査時点の「該当語を含むファイル数」を目安にする（`項目`・`候補`・`知見`・`問い`・`提案`・`仕分け`・`関門`・`負債`・`空欄`・`墓標`・`土壌`・`仮置き`・`種の文`・`読む札`・`結び付き`・`範囲` のいずれかを含む）。

### 5.1 A 層：agent が読む文章

| 場所 | ファイル数 | 変更 |
|---|---|---|
| `sai-sci/blocks/*.md` | 25 | 本文の専門語を置き換える |
| `sai-sci/template/*.md`（`sci-core`・`bw-core`・`exp-core`・`exp-run`・`sai-agent`・`sai-task`・`sai-cli`） | 6 | 同上 |
| `sai-sci/guides/*.md` | 25 | 見出し（`## 問い`・`## 埋まった条件`・`## 抜き出す項目`・`## 落とし穴`・`## 次の一手`・`## 子の項目`）と本文。見出しを読む側（`defs.rs`）は、新旧どちらの見出しも読めるようにする（6 章） |
| `sai-sci/types/*.md` | 5 | `## 項目の順序`・`## 種の文`・`## 関門` の見出しと、関門の名前 |
| `sai-sci/mind.md` | 1 | ひな形の見出しと注釈（10 章の決定による） |
| `crates/sai-paper-cli/assets/blocks/*.md`（`paper-*`） | 17 のうち research-tree を扱うもの | 該当する語だけ。`paper-*` は research-tree を使わないので、用語の衝突が少ない。必要な箇所だけ直す |
| `crates/sai-paper-cli/assets/template/*.md` | 一部 | 同上 |
| `crates/sai-paper-core/src/sci/context.rs` ほか | 8 | `sci context` の見出しと本文（`CURRENT_ACTION` など）。`export.rs` の `state.md` と `tree.md` の見出し・ラベルも同じ表で置き換える |

### 5.2 B 層：Research-tree の画面

| 場所 | 変更 |
|---|---|
| `apps/phone/src/sai_phone/static/scitree.js`（約 86 件） | 板・一覧・帯のラベル |
| `…/sciedit.js`（約 99 件） | カード作成・操作のフォーム、メッセージ |
| `…/scigraph.js`（約 49 件） | 根の図・表・凡例 |
| `…/sciread.js`（約 43 件） | 読む札の画面 |
| `…/ui.js`（約 18 件）・`workflow.js`・`papers.js`・`paperimport.js`・`cardform.js`・`index.html` | 用語の札、ラベル |
| `apps/phone/src/sai_phone/*.py`（38 ファイル） | 画面に出る文言（エラーメッセージを含む）。API のキーは変えない |
| `apps/phone/tests/test_*.js` / `test_*.py`（24 ファイル） | 期待値を更新する |
| `crates/sai-paper-cli/src/sci.rs`（約 109 件）・`main.rs`・`phone_sci.rs`・`mcp.rs`・`expand.rs` ほか | CLI の表示と `--help` の文 |
| `crates/sai-paper-tui/src/`（`tree_view.rs`・`main.rs`・`ui.rs`・`state.rs`） | TUI のラベル |
| `crates/sai-link-widgets/src/`（`block_log.rs`・`phone_view.rs` ほか 12 ファイル） | 実行の図・ログの表示に出る語（research-tree 関連のみ） |
| `pi-litbase/` | 拡張が出す文言（4 ファイル） |

画面のファイルを変えて配るときは、`AGENTS.md` の手順（`sw.js` の `CACHE` と `index.html` の `?v=` を揃えて上げる。`ASSETS` の確認）に従う。

### 5.3 C 層：変更しないもの

op 名・CLI のコマンド名とオプション名・DB の表名と列名・JSON のキー・MCP の道具名・ID の接頭辞・状態や種類の**保存される値**（`open`・`in_progress` など）。

---

## 6. 互換と移行

| 問題 | 対策 |
|---|---|
| 項目ガイド・型の md を、利用者が直している | **見出しは新旧どちらも読めるようにする**（`defs.rs` の読み取りに別名を持たせる）。段階 4 で旧名の扱いを決める |
| 配布済みの写し（`.sai/blocks/`・`.sai/template/`・`~/.sai-moon/`）が旧用語のまま残る | 配布物は `sai-moon-paper init --assets-only` などで更新される。**古い写しは自動では消えない**（sai-sci README §9 の paper-map の例）。更新の手順を文書に書く |
| 木に保存された節点の題名 | `create_item_from_guide` は、作成時のガイドの `name:` を題名として保存する。**既存の木の題名は変わらない**。表示が `display_name`（ガイドの名前 → 題 → key）の順を取るので、ガイドを直せば表示は追従するはずだが、**確認が要る**（11 章） |
| `mind.md` の見出し | `mind(価値の軸)` と `{mind:価値の軸}` が、見出しの文字列で読み取る。利用者の `mind.md` を壊さないため、**新旧の見出しを両方受け付ける**（10 章の決定による） |
| 画面の文言を見るテスト | 期待値を、段階ごとに更新する |
| 日本語だけを知っている利用者 | 画面の用語表（`TUTORIAL.md` の「先に覚えておく言葉」）に、英語名を足す。用語集（本書 3 章）への参照を置く |

---

## 7. 実施の順序

### 段階 1：用語集を文書に固定する
1. 本書 3 章を `sai-sci/README.md` の「用語の読み方」と `TUTORIAL.md` の用語表に反映する（日本語名と英語名を併記）。
2. 3.6 の関門の名前を、型ごとに決める。

### 段階 2：A 層（agent が読む文章）
1. `sci-core.md`・`bw-core.md` など template から置き換える。
2. 項目ガイドと型の見出し・本文を置き換える。`defs.rs` に別名の読み取りを足す。
3. ブロックの本文を置き換える。
4. `sci context`（`context.rs`）と `state.md`・`tree.md` の見出しを置き換える。テストを直す。
5. `sai-sci/` を直したので、release を作り直す（`AGENTS.md`）。

### 段階 3：B 層（Research-tree の画面）
1. `scitree.js`・`sciedit.js`・`scigraph.js`・`sciread.js`・`ui.js` と、そのテストを直す。
2. `apps/phone` の Python の文言と、そのテストを直す。
3. CLI・TUI・MCP・pi-litbase の表示を直す。
4. `sw.js` の `CACHE` と `index.html` の `?v=` を揃えて上げる。
5. `cargo build` を本体フォルダで行い、`target/release/sai-moon-paper` を作り直す。

### 段階 4：旧用語の扱い
- 旧い見出しと旧い用語の読み取りを、残すか、警告にするか、廃止するかを決める。**段階 3 まで終えて止めてもよい。**

---

## 8. 確認の方法

```bash
cd ~/workdir/Sai/sai-moon

# Rust（環境変数が混ざっていると History テスト 1 件が落ちる。必ず env -u で外す）
env -u SAI_PROJECT_ROOT -u SAI_PARENT_RUN_ID -u SAI_BLOCK_NAME \
  perl -e 'alarm 1800; exec @ARGV' cargo test --workspace

# Web
for f in apps/phone/tests/test_*.js; do node "$f" || exit 1; done
for f in apps/phone/tests/test_*.py; do PYTHONPATH=apps/phone/src apps/phone/.venv/bin/python "$f" || exit 1; done
```

- 置き換えの取りこぼしは、旧用語の一覧（本書 3 章の日本語欄）を `grep` して、残りを人が確認する。**一括置換はしない。**（`項目` は 507 件、`候補` は 401 件あり、一般の意味と衝突するため）
- 配る `sci` ブロックが engine の parser で読めることを見るテスト（コミット `ca95f63`）を通す。
- 実際の起動の確認は、確認用の run の `system-prompt.md` と最初のメッセージ、`block preview` を読む。
- `sai-moon-link daemon start`・`sai-moon-link init`・cc フックや pi 拡張の登録は実行しない（`AGENTS.md`）。
- 画面の確認は、`8200` 番で行う。本番の旧 sai-phone（8100 番）は止めない。

---

## 9. 他の計画との関係

`docs/20261001-1634-first-message-auto-sections.md`（最初のメッセージと `message.md` の見出しの構造を直す計画）も、`sci context` の出力と `sci-core.md` を触る。次の順序を勧める。

1. **先に、見出しの構造の計画を実施する。** 次の変更が済む。
   - 最初のメッセージが、sai-link の付ける `# context`、`# task_text` の順になる。
   - `sci context` の節が、すべて `##` になる（`今回の行動` も `##`）。
   - `{{input.task_text}}` と `{{context}}` が、本文から消える。
2. **そのあとで、本書の用語統一を行う。** 見出しの名前の変更が、1 回で済む。

本書の用語集との関係：
- `# context` と `# task_text` は、sai-link の入力名（`task_text` は `{{input.task_text}}` の id）であり、research-tree の専門語ではない。**小文字のまま変えない。**
- `## 今回の行動`・`## 研究の現在地` などの `##` の見出しは、本書 3.4 の英語名（`## CURRENT_ACTION`・`## PROGRESS` など）に置き換える。深さは、見出しの構造の計画で決まった `##` のまま。

逆に先にやると、`sci-core.md` と `context.rs` を 2 回触ることになる。

---

## 10. 決めてほしいこと

| 項目 | 案 | 推奨 |
|---|---|---|
| 項目名（`guides/*.md` の `name:`。背景・大目的・対象など 25 個） | A: key から UPPER_SNAKE を作る（`BACKGROUND_GOAL`）／B: 日本語のまま（研究内容の名前なので専門語に含めない） | **A**。「専門用語に含める」の決定と揃う。ただし画面の項目名は、研究内容を表す日本語から、英語の識別子に変わる |
| `mind.md` の見出し（始まり・研究の型・与えられていること・制約・価値の軸・興味がある / 興味が無い・メモ） | A: 英語に変える（新旧どちらも読む）／B: 日本語のまま（利用者が書くファイルなので内容に近い） | **A**（新旧どちらも読む）。ひな形は英語にするが、既存の `mind.md` は壊さない |
| 同じ綴りの分類の付け方（4 章の 2） | `分類:値`／別の綴りにする（例 ROLE の BACKGROUND を `ROLE_BACKGROUND` に） | `分類:値`（必要なときだけ） |
| 報告（report）の見出し | 英語の大文字にするか、日本語の見出しのままにするか。報告の書式はブロックごとに決まっている | 英語の大文字（専門語を含む見出しだけ） |

---

## 11. 確認していないこと

- 既存の木で、節点の題名の表示が `display_name`（ガイドの名前 → 題 → key）に従うか。従うなら、ガイドの `name:` を直すだけで、既存の木の表示も変わる。従わないなら、題名を直す移行処理が要る。
- `crates/sai-link-widgets/src/` の 12 ファイルのうち、research-tree に関わる語がどれだけあるか。
- `pi-litbase/` の 4 ファイルの、用語の使われ方。
- 項目ガイドの見出しを読む `defs.rs` が、別名を足しやすい作りか。
- 関門の名前（3.6）の英語名の自動生成が、すべての型で十分に読める長さになるか。長い名前（`NOVELTY_CLOSEST_COUNT_3_AND_NOVELTY_DIFF_DECIDED`）になる関門が、画面の幅（390px）に収まるか。
- 該当語のファイル数は、`grep` の数え上げで、一般の意味の用例も含む。実際に置き換える数は、これより少ない。
