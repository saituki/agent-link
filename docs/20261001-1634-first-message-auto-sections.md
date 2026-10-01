# 修正計画書：最初のメッセージと message.md の見出しの構造を直す

- 作成日: 2026-10-01（同日に 2 回改訂：1 回目 `# 範囲` の廃止案、2 回目 `# context` の見出しを sai-link が付ける案）
- 対象: `~/workdir/Sai/sai-moon`（`crates/sai-link-engine`・`crates/sai-paper-core`・`crates/sai-link-tui`・`sai-sci`・`crates/sai-paper-cli`・`apps/phone`）
- 状態: 計画（未実装）。「11. 決めてほしいこと」を決めてから着手する。
- 関連: `docs/20261001-1806-research-tree-glossary-english.md`（用語を英語の大文字に統一する計画。`sci-core.md` と `context.rs` が重なるので、**本書を先に実施する**）

---

## 1. 要約

起動時に agent へ渡る文章と、実行後に保存される `message.md` で、**見出しの深さが揃っていない**。

- `sci context` の出力は、`今回の行動` だけが `#`、他の節（研究の現在地・土壌・対象の節点など）は `##` で、すべて `今回の行動` の子に見える。
- task_text に埋め込む waggle 文書は、包みの見出しだけが付き、中身の `#` は元のまま。`# task_text` と同じ深さになる。
- `message.md` の区切り（`## user (n)`・`## assistant (n)`）は、本文の `#` より浅い。
- block.md と template.md の `{{input.task_text}}` と `{{context}}` は、最初のメッセージのどの節にあるかを示す案内に置き換わるだけで、実質的な役割がない。

そこで次のとおりにする。

1. **最初のメッセージを `# context`、`# task_text` の順に、sai-link が必ず自動で作る。** 見出しも sai-link が付ける。本文の `{{input.task_text}}`・`{{context}}` は書かなくてよくする。
2. **context の出力の節は、すべて `##` にする**（`今回の行動` も `##`）。`# context` の下に、同じ深さの兄弟として並ぶ。
3. **waggle 文書は、包みを `###`、中身を 3 段下げ、frontmatter を外して埋め込む。** コードブロックの中は下げない。
4. **`message.md` は、`user (1)`（最初のメッセージ。task_text と同じ内容）の区切りをなくす。** 区切りは `#`。assistant の本文は 1 段下げる。2 通目以降の利用者の発話は、番号を 1 から振り直す。

---

## 2. 決定事項（利用者との確認済み）

| 項目 | 決定 |
|---|---|
| 最初のメッセージの順序 | context → task_text |
| 見出しの付け方 | sai-link が `# context`・`# task_text` を付ける。context の中身は `##` から始める |
| waggle 文書の frontmatter | **外す** |
| `message.md` の `user (1)` | 不要（task_text と同じ内容のため）。区切りごとなくす |
| `message.md` の区切り | `#` 1 つ |
| 計画書の取り込み | 本書と用語統一の計画書に反映する |

---

## 3. 現状（調べて分かったこと）

### 3.1 起動時の入力の分け方

| 部分 | 中身 | 渡し方 |
|---|---|---|
| 固定部分（system prompt） | template と block.md の本文。`{{input.<id>}}` と `{{context}}` は案内文に置き換わる | run フォルダの `system-prompt.md`。`--append-system-prompt`（pi）・`--append-system-prompt-file`（cc） |
| 最初のメッセージ | 本文に現れた `{{input.<id>}}` の順に `# <id>` の節。`context: true` なら最後に `# 範囲` の節 | 起動後の 1 通目 |

実装は `crates/sai-link-engine/src/placeholder.rs` の `split_fixed_and_variable` と、`orchestrator.rs` の `pre_run::context_text` を呼ぶ所。

### 3.2 `{{input.task_text}}` と `{{context}}`

- `{{input.task_text}}` は、本文に書かないと `# task_text` の節が作られない（依頼文が届かない）。
- `{{context}}` は、書かなくても `# 範囲` の節が最後に足される。
- `{{input.…}}` は、`.sai/blocks`・`sai-sci`・`crates/sai-paper-cli/assets` で合計 74 件、**すべて `task_text`**。`block_config.rs:110` に「入力は `task_text` の 1 つに統一し、旧 `inputs:` は廃止した」とある。

### 3.3 `sci context` の出力の見出し

```md
# 今回の行動（sai-sci より）
## 研究の現在地
## 土壌（mind.md から）
## 対象の節点
## 上流の道（なぜこの節点を扱うか）
## 仮置き（この議論では問い直さない）
## 範囲外（この回では議論しない）
## 項目ガイド: …
## 既に調べたこと
## 出せる提案
## 対象論文
```

`今回の行動` だけが `#` で、残りはすべて `##`。`今回の行動` が、他の節をすべて含んで見える。

失敗時（`pre_run.rs` の `context_text`）は、見出しのない 1 行を返す。例：「`.sai/settings.json` に context がありません。範囲を作れませんでした」。

### 3.4 waggle 文書の埋め込み

- `placeholder.rs` の `wrap_token_content` が、次の形で包む。
  ```md
  <!-- waggle:token:<token> block:<block> -->

  ## 出力トークン: <token> (<block>)

  <中身>

  <!-- /waggle:token:<token> -->
  ```
- 中身は加工されない。元の文書の frontmatter（`---` で囲まれた部分）と `#` 見出しが、そのまま入る。
- sci のカードの task.md は、`## 受け取る文書` の下に `- waggle:<token>` の行を並べる（`card.rs`）。`phone card create` の `doc_tokens` は、`# task_text` の先頭に `waggle:<token>` の行を貼る（`phone.rs`）。

### 3.5 message.md

- 作る所は `orchestrator.rs` の `render_message_md` の 1 か所だけ。
- 区切りは `## user (n)`・`## assistant (n)`。
- 区切りの見出しを読む側（Web など）は、見つからなかった。見出しを期待しているのは、テスト 2 か所だけ（`crates/sai-link-tui/src/app/input.rs:1716–1721`、`orchestrator.rs:6111`）。

### 3.6 今の最初のメッセージの形

```md
# task_text
…依頼文…
## 受け取る文書
<!-- waggle:token … -->
## 出力トークン: <token> (<block>)
---
frontmatter
---
# <文書の見出し>       ← `# task_text` と同じ深さ
…

# 範囲

# 今回の行動（sai-sci より）   ← `# 範囲` が空に見える
## 研究の現在地               ← `今回の行動` の子に見える
```

---

## 4. 問題

1. **見出しの入れ子が崩れる。** 3.6 のとおり。agent も人も、どの節がどの節に含まれるかを読み違える。
2. **context の構造が、1 つの文書の中で揃っていない。** `今回の行動` だけ `#`。
3. **waggle の中身が、frontmatter ごと本文に混ざる。** task.md を埋め込むと、`runner: cc` などが本文に入る。
4. **`message.md` の区切りが本文より浅い。** 最初のメッセージは、task_text と同じ内容が二重に出る。
5. **書き忘れで依頼文が消える。** `{{input.task_text}}` を書かないと届かない。
6. **書く意味の薄い記法が 74 か所に残っている。**

---

## 5. 目標と範囲外

### 5.1 目標

- 最初のメッセージを、`# context` と `# task_text` の 2 つの最上位の節にする。順序は固定。
- 見出しの入れ子を、`message.md` まで含めて正しくする。
- 既存のブロック（古い写しを含む）が、変更なしでも壊れず、依頼文や範囲が二重にならない。
- 新しく書くブロックと template では、`{{input.task_text}}` と `{{context}}` を書かなくてよい。

### 5.2 範囲外

- `{{subject}}`・`{{description}}` の整理。
- `sci context` が出す節の構成の変更（見出しの**深さ**だけ直す。節の追加・削除はしない）。
- 用語の英語化（`docs/20261001-1806-research-tree-glossary-english.md` の仕事）。
- ブロックの front-matter（`templates:`・`sci_scope`・`sci_ops` など）の変更。
- 旧 4 リポジトリは読み取り専用なので触らない。

---

## 6. 設計

### 6.1 最初のメッセージの組み立て

| 順 | 節 | 足す条件 | 中身 |
|---|---|---|---|
| 1 | `# context` | `context: true` で、context の出力が空でないとき | context の出力。**見出しは付け直さない**（context の側が `##` から始める） |
| 2 | `# task_text` | 依頼文が空でないとき | 依頼文（waggle の埋め込みを含む） |

- 節と節の間は空行 1 行。
- context が空で依頼文も空なら、今までどおり定型文を使う（固定部分が空でないとき）。
- **失敗時の 1 行も、`# context` の下にそのまま入る。** 失敗時のために見出しを付ける特別な処理は要らない。

### 6.2 context の出力の見出し（`context.rs`）

- `# 今回の行動（sai-sci より）` を `## 今回の行動（sai-sci より）` にする。他の節は、すでに `##`。
- これで、`今回の行動` を含むすべての節が、`# context` の下の兄弟になる。
- 標準出力に出す形（`sai-moon-paper sci context --run <id>` を人が打ったとき）は、`##` から始まる。最上位の見出しはない。それでよいとする。
- 木がないとき（`NO_TREE`）・節点の指定がないとき（`NO_NODE`）の文も、先頭の見出しを `##` にする。
- **決まり：** `.sai/settings.json` の `context` に別の命令を指定する場合、その出力も `##` から始める。この決まりを `sai-sci/README.md` の §5.2 に書く。

### 6.3 waggle 文書の埋め込み

`# task_text` の下に入る waggle 文書は、次の形にする。

```md
# task_text
…依頼文…
## 受け取る文書
<!-- waggle:token:<token> block:<block> -->
### 出力トークン: <token> (<block>)
#### <文書の元の `#` 見出し>
##### <元の `##` 見出し>
<!-- /waggle:token:<token> -->
```

- 包みの見出しを `###` にする（`## 受け取る文書` の子）。
- 中身の見出しを **3 段下げる**（元の `#` が `####`、`##` が `#####`、`###` 以降は 6 段で止める）。
- **中身の frontmatter（先頭の `---` で囲まれた部分）を外す。**
- **コードブロック（```` ``` ```` で囲まれた部分）の中の行は、下げない。** bash の `# コメント` や Markdown の例が壊れるため。
- マーカーのコメント（`<!-- waggle:token … -->`）は、今までどおり。
- `## 受け取る文書` の下以外（`phone card create` の `doc_tokens` が先頭に貼る行など）に入る waggle 文書も、同じ包み（`###`）にする。**親の見出しのない位置では、見出しが 1 段飛ぶ**（`# task_text` の直下に `###`）。これは許容する（11 章）。

### 6.4 message.md

```md
---
block_name: plan
run_id: 129
---

# context
## 今回の行動（sai-sci より）
…
# task_text
…
## 受け取る文書
### 出力トークン: <token> (<block>)
#### …

# assistant (1)
## <本文の元の `#` 見出しを 1 段下げたもの>

# user (1)
…
# assistant (2)
…
```

- **最初の利用者の発話（`user (1)`）は、区切りごとなくす。** 内容は `# context`、`# task_text` と同じなので、その節が、そのまま最上位に並ぶ。
- 区切りは `#`（`# assistant (n)`、`# user (n)`）。
- 2 通目以降の利用者の発話は、**番号を 1 から振り直す**（元の `user (2)` が `user (1)`）。assistant は、これまでどおり 1 から。
- assistant の本文の見出しは、**1 段下げる**（元の `#` が `##`）。コードブロックの中は下げない。
- 2 通目以降の利用者の発話の本文の見出しも、1 段下げる。

### 6.5 見出しを下げる共通の関数

次の 3 か所が同じ規則を使うので、**共通の関数を 1 つ**にする（`sai-link-engine` の `placeholder.rs`、または近くの小さな関数）。

- waggle 文書の中身（3 段）
- assistant の本文（1 段）
- 2 通目以降の利用者の発話（1 段）

規則：

1. 行頭の `#` が連続して 1〜6 個あり、その次が空白である行を、見出しとする。
2. 見出しの `#` の数に、下げる段数を足す。6 を超えたら 6 で止める。
3. ```` ``` ````（または `~~~`）で囲まれたコードブロックの中の行は、見出しとみなさない。
4. HTML コメントの中の行は、触らない。

### 6.6 固定部分（system prompt）に足す案内文

固定部分の先頭に、次の趣旨の案内を 1 か所だけ自動で足す。

> 範囲（研究方針の木から切り出した文脈）は、最初のメッセージの「# context」の節にあります。依頼は「# task_text」の節にあります。

- `context: true` でないブロックには、`# context` に触れる 1 文を入れない。
- 文言は実装時に決める。

### 6.7 古い記法の扱い（互換）

本文に `{{input.task_text}}` や `{{context}}` が残っているときは、次のとおりにする。

- 固定部分では、今までどおり案内文に置き換える。
  - `{{input.task_text}}` → `（最初のメッセージの「# task_text」の節にある）`
  - `{{context}}` → `（最初のメッセージの「# context」の節にある）`
- **6.6 の自動の案内文は、重複するので足さない。**
- 最初のメッセージの節は、本文に書いてあっても、`# context` と `# task_text` の 1 つずつだけ。
- 本文に `{{input.task_text}}` がなくても、`# task_text` の節は足す（今との違い）。

### 6.8 `block preview`

今は context を実行せず `{{context}}` のまま見せ、`context_text` は「実行番号が無い」の 1 行を返す。変更後は、次のとおりにする。

- 固定部分の自動の案内文を表示する。
- `# context` の節を表示する。中身は、「実行番号が無いため、範囲を作れませんでした」の 1 行。

---

## 7. 変更する場所

### 7.1 コード

| 場所 | 変更 |
|---|---|
| `crates/sai-link-engine/src/placeholder.rs` | `split_fixed_and_variable`：`# context`・`# task_text` を、この順で必ず足す。固定部分に案内文を足す。古い記法との重複を避ける。`wrap_token_content`：包みを `###` にし、frontmatter を外し、中身を 3 段下げる。見出しを下げる共通の関数を足す。ドキュメントコメントと例を直す |
| `crates/sai-link-engine/src/orchestrator.rs` | `render_message_md`：`user (1)` の区切りをなくす。区切りを `#` にする。番号を振り直す。本文の見出しを下げる。呼び出し側の確認。テスト（約 4762 行・6111 行・6169 行）の期待値を直す |
| `crates/sai-link-engine/src/pre_run.rs` | `context_text` のコメントを直す（出力は `##` から始まる）。関連テストを確認する |
| `crates/sai-paper-core/src/sci/context.rs` | `今回の行動`（`NO_TREE`・`NO_NODE` を含む）の見出しを `##` にする。テストの文言を直す |
| `crates/sai-link-tui/src/app/input.rs`（1716〜1721 行） | `## user (1)` などを期待するテストを、新しい形に直す |
| `crates/sai-link-engine/src/block_config.rs` | コメント（110 行）の文言を直す |
| `crates/sai-link-engine/src/mcp_server.rs` | `task_text` の説明文（262 行）から `{{input.task_text}}` の言及を外す |
| `crates/sai-link-cli/src/cli.rs` | `block preview` などの表示の確認・直し |
| `apps/phone/src/sai_phone/static/defs.js`（115 行）と `apps/phone/tests/test_defs.js`（96・100 行） | 新しいブロックのひな形に `# 依頼 / {{input.task_text}}` を入れている。入れないひな形に直す。直したら `sw.js` の `CACHE` と `index.html` の `?v=` を揃えて上げる |

### 7.2 テスト

| 場所 | 内容 |
|---|---|
| `crates/sai-link-engine/tests/placeholder_tests.rs` | 新しい挙動：本文に記法がなくても節が足される／古い記法でも二重にならない／順序が context → task_text／`context: true` でないブロックは `# context` が足されない／依頼文が空なら `# task_text` は足されない／見出しを下げる関数（フェンス内を下げない・6 段で止まる・frontmatter を外す） |
| `crates/sai-link-engine/tests/orchestrator_test.rs` | 期待値の更新。`message.md` の新しい形（`user (1)` なし・番号の振り直し） |
| `crates/sai-link-engine/tests/waggle_integration_test.rs` | 影響の確認 |
| `crates/sai-paper-core/src/sci/context.rs` の単体テスト | 見出しの深さ |
| `apps/phone/tests/test_defs.js` | ひな形の期待値の更新 |

### 7.3 ブロックと template（段階 2 で行う）

| 場所 | 数（調査時点） | 変更 |
|---|---|---|
| `sai-sci/blocks/*.md` | 26 | `## 依頼`（または同等の見出し）と `{{input.task_text}}` の行を削除 |
| `crates/sai-paper-cli/assets/blocks/*.md` | 17 | 同上 |
| `sai-sci/template/sci-core.md` | 1 | `## 今回の範囲` の `{{context}}` を削除。節の説明は、「最初のメッセージの `# context` の `## 今回の行動` から始まる節」を指す言い方に直す |
| `.sai/blocks/*.md`（この repo の写し） | 28 | 配布物の写し。`sai-moon-paper init --assets-only` で更新される。**手では直さない** |
| `~/.sai-moon/` の写し | ― | 同上 |

文書（`docs/explain_all.md`・`docs/for-sai-developers.md`・`docs/20260929-1700-system-map.md`・`sai-sci/README.md` の §5.2）で、`{{input.task_text}}`・`{{context}}`・`# 範囲` を説明している箇所も、段階 2 で直す。

---

## 8. 実施の順序

### 段階 1：構造を直す（互換あり）

1. 見出しを下げる共通の関数を作り、単体テストを書く（6.5）。
2. `split_fixed_and_variable` を直す（6.1・6.6・6.7）。
3. `wrap_token_content` を直す（6.3）。
4. `context.rs` の見出しを `##` にする（6.2）。
5. `render_message_md` を直す（6.4）。
6. `block preview` の表示を直す（6.8）。
7. この段階では、ブロックと template の本文は触らない。

**完了の条件：** 既存のブロックを変えずに、次の 3 点が成り立つ。
- 最初のメッセージが `# context`、`# task_text` の順になり、見出しの入れ子が正しい。
- 古い記法が本文に残っていても、依頼文や範囲が二重にならない。
- `message.md` に `user (1)` がなく、区切りが `#` になる。

### 段階 2：記法を消す

1. `sai-sci/blocks`・`crates/sai-paper-cli/assets/blocks`・`sci-core.md`・`defs.js` のひな形から、記法と見出しを消す。
2. 文書の説明を直す。
3. `cargo build` を本体フォルダで行い、`target/release/sai-moon-paper` を作り直す（`AGENTS.md` の約束）。
4. `sai-sci/` を直したので、release を作り直す。

**完了の条件：** `sai-sci` と assets のブロックに `{{input.task_text}}` が 0 件。テストがすべて通る。

### 段階 3：古い記法を廃止するか決める（任意）

すべての配布済みの写しが更新されたあとで、古い記法を読み捨てるか、警告にするか決める。**段階 2 までで止めてもよい。**

---

## 9. 確認の方法

`AGENTS.md` の手順に従う。

```bash
cd ~/workdir/Sai/sai-moon

# Rust（環境変数が混ざっていると History テスト 1 件が落ちる。必ず env -u で外す）
env -u SAI_PROJECT_ROOT -u SAI_PARENT_RUN_ID -u SAI_BLOCK_NAME \
  perl -e 'alarm 1800; exec @ARGV' cargo test --workspace

# Web
for f in apps/phone/tests/test_*.js; do node "$f" || exit 1; done
```

- 実際の起動の確認は、`block preview` と、確認用の run の `system-prompt.md`・`message.md` を読む。
- `sai-moon-link daemon start`・`sai-moon-link init`・cc フックや pi 拡張の登録は実行しない（`AGENTS.md`）。確認用の run が必要なら、利用者に頼む。
- 本物の `.sai/blocks/*.md` と `~/.sai-moon/` の中身は、確認のために変えない。

---

## 10. リスクと戻し方

| リスク | 対策 |
|---|---|
| 古い写しの本文で、依頼文や範囲が二重に入る | 6.7 の規則で節を 1 つにする。単体テストで確かめる |
| 固定部分の案内文が重複する | 古い記法が本文にあるときは、自動の案内文を足さない |
| 見出しを下げる処理が、コードブロックの中の `#` まで書き換える | フェンスを数える（6.5）。テストで、bash の `# コメント` と ```` ```md ```` の例を確かめる |
| 6 段を超える見出しの扱い | 6 段で止める。深い見出しは潰れるが、内容は失われない |
| waggle 文書の frontmatter を外すと、`runner` や `run_id` などの情報が agent に届かない | 必要な情報は、task.md の本文と context に入っている。frontmatter だけにある情報で agent が必要とするものがないか、段階 1 の最初に確認する（12 章） |
| `message.md` の形が変わり、過去の `message.md` と見た目が違う | 過去のファイルは書き換えない。新しい run から新しい形になる |
| context を別の命令に指定した利用者の出力が `#` から始まる | 6.2 の決まりを文書に書く。出力が `#` から始まっても、`# context` の下に深さの違う節が入るだけで、動作は止まらない |
| テストの期待値の直し漏れ | 段階ごとに `cargo test --workspace` を通す |

戻し方：段階 1 は、`placeholder.rs`・`orchestrator.rs`・`context.rs` の変更を戻すだけでよい。段階 2 は、ブロックと template の削除なので、git で戻せる。

---

## 11. 決めてほしいこと

| 項目 | 案 | 推奨 |
|---|---|---|
| 古い記法の扱い | A: 当面受け付ける（互換あり）／B: すぐ廃止する | **A で始め、段階 3 で B を判断する。** 利用者のプロジェクトの `.sai/blocks/` に古い写しが残るため |
| 固定部分の案内文の位置 | 先頭／末尾 | 先頭（読む順番で最初に分かる） |
| 親の見出し（`## 受け取る文書`）のない位置にある waggle 文書 | A: `###` の包みのまま（見出しが 1 段飛ぶ）／B: 位置に応じて包みの深さを変える | A。実装が単純で、`phone card create` の `doc_tokens` の例だけが対象 |
| 2 通目以降の利用者の発話の番号 | 1 から振り直す（`user (1)`）／元のまま（`user (2)`） | 1 から振り直す（assistant と揃う） |

---

## 12. 確認していないこと

- `block_body` に template の本文が含まれるかどうか（`orchestrator.rs` の組み立て）は、コメントからの推測で、コードを追っていない。段階 1 の最初に確かめる。
- `crates/sai-link-cli/src/cli.rs` の `block preview` の実装は読んでいない。
- `apps/phone` の `defs.js` が作るひな形の正確な用途（新規ブロックの作成か、表示か）は読んでいない。
- 実際の `system-prompt.md`・`message.md`・最初のメッセージの全文は、確かめていない。
- Web の message 画面が、`message.md` の見出し（区切り）をどう解析・表示しているかは、確認していない。コードの検索では、見出しを読む側が見つからなかった。
- 最初の利用者の発話（`user (1)`）が、本当に task_text と同じ内容かどうか（context を含むため、同じではない可能性がある）。計画では、`# context` と `# task_text` が最上位の節として並ぶので、内容は残る。
- waggle 文書の frontmatter にだけある情報で、agent が必要とするものがないか。
- `sci context` の `NO_TREE`・`NO_NODE` の文を、テストが見出しの文言で確かめているか。
