# sai-sci（研究方針の木 / Research-tree）

**読み手**: 利用者と、`sai-sci/` の md を直す開発者。

初めて使う人は、手を動かしながら全部の機能を回る [`TUTORIAL.md`](TUTORIAL.md) から始めてください。

`sai-sci` は、**研究の方向づけの段階（実験の前にある「何をやるか」を決める段階）を、研究方針の木（Research-tree）で扱う仕組み**です。
木を紙に描いたり表に整理したりするのではなく、木の正を 1 か所（`papr.db`）に置き、絵と文章はその正からの書き出しにします。

- 木は、論文の置き場があるプロジェクトごとに 1 つあります。
- Web（`apps/phone`）・TUI・CLI・MCP・pi の拡張が、同じ木を読み書きします。
- エージェントは木に提案を出します。自動で入るのは「知見・問い・結び付け・種」で、木の形が変わる操作は利用者が仕分けます。

## 1. sai-sci とは

`sai-sci` は、**研究の方向づけの段階（実験の前）を、研究方針の木（Research-tree）で扱う仕組み**です。

- **木の正は `papr.db`** です。項目・候補・問い・知見・結び付き・決定の記録・提案・種は、いずれも `sci_` で始まる表に入っています（migration `0046_sci_tree.sql`）。
- **`state.md` と `tree.md` は書き出し**です。手で直しても次の書き出しで消えます。
- **木は型が決めるときの骨組みの形になります。** 研究の型（5 つの型）ごとに決まる「項目の順序」と「関門（通るための条件の並び）」があります。
- **エージェントは提案を出すだけです。**木に直接書き込む処理は入れません。

用語の読み方:

| 用語 | 何 |
|---|---|
| 種の文 | 研究の型ごとの 1 文のひな形。決まった項目の値が入り、まだ決まっていない所は［？］で出る |
| 関門 | 型ごとに決めた通過条件の並び。最初に通っていない関門が「今いる所」 |
| 項目 / 候補 | 項目は研究の穴（例: 小目的）。選ぶ・並べる項目には「候補」（発作部位の予測など）が付く |
| 空欄 | 項目がまだ決まっていない所。空欄には「種類」（未着手・調査中・情報不足・判断待ち・対立）が付く |
| 印 | 決まっている項目でも、根拠が薄い・上流が変わっているものに付く印（根拠不足・要再確認） |
| 負債 | 先に片付けたいもの（仕分けていない提案・反映していない論文・選別していない検索・書いていない振り返り） |
| 知見 | 論文などから分かった 1〜3 文の事実と出典。支持・反証などの向きを持つ |
| 提案 | エージェントが出した変更の申し出。「自動で入るもの」と「利用者が仕分けるもの」がある |
| 仕分け | 提案を採用するか却下するかを決めること |
| 墓標 | 外した論文の記録。もう一度検索に出ないように残す |
| 読む札 | 利用者が 1 本の論文を読んで知見を書くための札（`runner: user` のカード） |

---

## 2. 3 つの考え

### 2.1 骨組みと候補の木

研究の穴と、その穴の候補を木で表す。型が決まると、一番上の項目の並び（骨組み）と、その下に候補を置くかが決まる。
木には親子（`parent`）のほかに、依存（`depends`）・由来（`derived`）・同型（`analogous`）の線も引ける。

### 2.2 知見を 1 級の物として扱う

「読んだ論文のメモ」ではなく、**出典と向きを持った知見（F31 など）**が木の 1 級の要素です。
どの項目を支持したのか、どの候補を反証したのかが、木の上でたどれる。

### 2.3 変更は危険度で分ける

| 危険度 | それは | 例 |
|---|---|---|
| 自動 | 出した提案はすぐ木に入る。取り消せるので、迷ったら出してよい | 知見・問い・結び付き・種 |
| 人 | 提案は仕分け箱で待つ。利用者が採用すると反映される | 候補・項目の追加・決定・線・対象外にする |

---

## 3. フォルダの中身

```
sai-sci/
├── types/      # 5 つの型（problem・phenomenon・method・data・improve）
├── guides/     # 項目ガイド（上の項目 9 本・子の項目 16 本）
├── template/   # sci-core.md（エージェントの共通の決まり）と exp-core.md・exp-run.md（実験ループの決まり）
├── mind.md     # 土壌のひな形
├── tools/      # 実験ループの道具: 台帳 exp_summary.py・実行先への分配器 exp_exec.py とそのテスト（`sci init` が <root>/.sai/tools/ へ配る）
├── exec/       # 実行先の見本: local（この PC・有効）・ssh（Linux の PC・無効の見本）・dok（さくらの高火力 DOK・課金あり・無効の見本）・_example（雛形）。`sci init` が <root>/.sai/exec/ へ配る
└── blocks/     # 18 本のブロック
```

### 3.1 5 つの型

`types/` の 5 本。各型は「項目の順序」「種の文」「関門」を持つ。`given` は「最初から与えられているもの」。

| id | 名前 | given | どれを選ぶか |
|---|---|---|---|
| `problem` | 課題駆動 | `background` | 現場で今のやり方では形にならない問題が残っており、「どうにかしたい」という課題から研究が始まる。決まっているのは大目的だけ |
| `phenomenon` | 現象駆動 | `phenomenon` | 必ず起きている現象が先にあり、「なぜそうなるのか」を問う。観察できる事実が手法より先にある |
| `method` | 手法駆動 | `method` | 手元で新しい手法が使える状態が先にあり、「それを何に使うか」を決める。手法の中身は既に決まっている |
| `data` | データ駆動 | `data` | 手元のデータセットが先にあり、「このデータで何が言えるか」を問う。集めたときの目的とは限らない |
| `improve` | 改良駆動 | `existing` | すでに動いている手法があり、「これをよくする」ことを目標にする。何を直すかはおおむね決まっている |

### 3.2 項目ガイド（上の項目 9 本）

`guides/` の直下に無印で置かれる。一番上の項目。

| key | 名前 | 型 |
|---|---|---|
| `background` | 背景 | 書く |
| `target` | 対象 | 選ぶ |
| `data` | データ | 選ぶ・確かめる |
| `purpose` | 目的 | 選ぶ |
| `existing` | 既存手法 | 並べる |
| `issue` | 課題 | 確かめる |
| `method` | 手法 | 並べる・選ぶ |
| `novelty` | 新規性 | 書く・確かめる |
| `phenomenon` | 現象 | 書く・確かめる |

### 3.3 項目ガイド（子の項目 16 本）

| key | 名前 | 型 | 親 |
|---|---|---|---|
| `background.goal` | 大目的 | 書く | `background` |
| `background.problem` | 問題の背景 | 書く・確かめる | `background` |
| `target.subgroup` | 対象の細分 | 選ぶ | `target` |
| `data.type` | データの型 | 選ぶ | `data` |
| `data.size` | データの量 | 書く・確かめる | `data` |
| `data.label` | ラベル | 選ぶ | `data` |
| `data.access` | 入手 | 選ぶ・確かめる | `data` |
| `purpose.subgoal` | 小目的 | 選ぶ | `purpose` |
| `purpose.output` | 出力の型 | 選ぶ | `purpose` |
| `purpose.metric` | 評価指標と成功基準 | 書く | `purpose` |
| `issue.limit` | 既存手法の課題 | 並べる・確かめる | `issue` |
| `issue.root` | 根本の原因 | 書く・確かめる | `issue` |
| `method.analog` | 同型の問題 | 並べる | `method` |
| `method.adapt` | 取り入れ方 | 書く | `method` |
| `novelty.closest` | 最も近い研究 | 並べる | `novelty` |
| `novelty.diff` | 差分 | 書く・確かめる | `novelty` |

### 3.4 18 本のブロック

`blocks/` の 18 本。`sai-moon-paper` はこれらを `--assets-only` の `init` や `sci init` でプロジェクトの `.sai/blocks/` に配る。
`sci_scope` は「このブロックの行動に切り出す範囲」、`sci_ops` は「このブロックで提案できる op」。

| id | 名前 | 類 | 範囲 | 提案できる op | 呼べる子ブロック |
|---|---|---|---|---|---|
| `sci-start` | 🌱 研究の入口 | outline | summary | add_finding, add_question, answer_question, link, add_option, add_child, edit_text, decide | sci-search, sci-lookup |
| `sci-outline` | 🧭 全体の見直し | outline | summary | add_question, link, add_option, reopen, exclude, add_edge | sci-lookup |
| `sci-digest` | 🗒 まとめ | outline | summary | add_question | ― |
| `sci-seed` | 🌰 種づくり | outline | summary | add_seed, add_question, add_finding | sci-lookup |
| `sci-read` | 📖 1 本を読む | detail | path | add_finding, add_question, answer_question, link, add_option, exclude | sci-lookup |
| `sci-compare` | ⚖ 比べる | detail | path | add_finding, link, add_option, decide, option_state | sci-lookup, sci-extract |
| `sci-probe` | 🔍 確かめる | detail | path | add_finding, add_question, link | sci-search, sci-lookup |
| `sci-draft` | ✍ 文を書く | detail | path | edit_text, add_finding, add_question | sci-lookup |
| `sci-ideate` | 💡 候補を増やす | detail | path | add_option, add_child, add_question, add_finding, link | sci-analogy, sci-lookup |
| `sci-debate` | 🗣 議論 | detail | path | add_finding, add_question, link | paper-debater, paper-reviewer |
| `sci-coach` | 🎓 理解の確認 | detail | node | add_question, answer_question | ― |
| `sci-search` | 🔎 外部を探す | support | node | link, add_finding | ― |
| `sci-screen` | 🧹 選別 | support | node | link, exclude, add_question | ― |
| `sci-extract` | 📋 抜き出す | support | node | add_finding, link | ― |
| `sci-lookup` | 📚 Library で答える | support | node | add_finding, link | ― |
| `sci-analogy` | 🔁 同型を探す | support | node | add_option, add_finding, link, add_question | sci-search |
| `sci-exp-seed` | 🌱 exp-seed | seed | path | add_finding, add_question, answer_question, link | sci-exp-branch |
| `sci-exp-branch` | 🌿 exp-branch | branch | ― | ― | ― |

- 先の 16 本は全部 `context: true`（実行の直前に研究方針の木から範囲を作る）・`model: inherit`・`worktree: none`。
- 3 つの類で `timeout` が違う。outline と detail は 5400 秒、support は 3600 秒。
- `sci-debate` の参加者は `paper-debater` と `paper-reviewer`（普通の `paper-*` のブロック）。
- 後ろの 2 本は**実験ループ**（5.6 節）。`sci-exp-branch` だけは木の範囲も共通の決まり（`sci-core`）も受け取らない。判定をしないから、範囲も要らない。
- `layer` は C-01 の層で、`root`（上の 16 本）・`seed`（`sci-exp-seed`）・`branch`（`sci-exp-branch`）の 3 種類。書かないブロックは図に出さない。

### 3.5 `template/` と `mind.md`

- **`template/sci-core.md`** — エージェントに共通で渡す決まり。範囲を出す `{{context}}`、確かめの地点、提案の書き方、使える 14 種類の op、根拠の扱い、迎合しない姿勢。
- **`template/exp-core.md`** — 実験ループ（seed と branch）の共通の決まり。役割、文書の渡し方、実験仕様書、MLflow の記録の決まり（C-04）、台帳、判定の規則、依頼文の形、実行場所の規則、停止の条件。
- **`template/exp-run.md`** — 実験のスクリプトを「実行先」に投げる手順（分配器 `exp_exec.py` の呼び方）。`exp-core` の「実行場所の規則」と対で読む。
- **`mind.md`**（`sai-sci/mind.md`）— 土壌のひな形。「始まり・研究の型・与えられていること・制約・価値の軸・興味がある/無い・メモ」の 7 節。**利用者が書くファイルで、エージェントは直接書き換えません。** 「価値の軸」は「選ぶ項目」を比べる軸として `sci node` に出す。

---

## 4. 始め方

### 4.1 新しいプロジェクト

```bash
sai-moon-paper init --sci --type <型>
```

`init` 自身を終えたあとに `sci init --type <型>` と同じことをします。`mind.md` が無ければ、`sai-sci/mind.md` の土壌のひな形を書きます（普通の `project_files.rs:13` のひな形の代わり）。

### 4.2 今のプロジェクト

```bash
sai-moon-paper sci init --type <型>
```

既に始めているときは終了コード 1 で「既に始めています（型: 課題駆動）」が出ます。型を変えるには `--change-type` を付けます。

### 4.3 入る物

`sci init`（と `init --sci`）は次を配ります。

| 入るもの | 置き場 |
|---|---|
| 18 本の `sci-*` ブロック | `<root>/.sai/blocks/` |
| `sci-core.md`・`exp-core.md`・`exp-run.md`（と 18 本が使う `search-internal` などのテンプレ、`domain`） | `<root>/.sai/template/` |
| 台帳 `exp_summary.py`（判定の材料を取る道具）と分配器 `exp_exec.py`（無いときだけ書く） | `<root>/.sai/tools/` |
| 実行先の見本 `local`・`_example`（無いときだけ書く） | `<root>/.sai/exec/` |
| `.sai/settings.json` の `context`（実行直前に範囲を作る命令。無いときだけ書く） | `<root>/.sai/settings.json` |
| `mind.md`（土壌のひな形。無いときだけ書く） | `<root>/.sai/paper/mind.md` |
| 型と項目ガイドの写し | `<root>/.sai/sci/types/` と `<root>/.sai/sci/guides/` |
| 木 | `papr.db` の `sci_*` の表 |
| `state.md` を 1 度だけ写して残す（`state.pre-sci.md`。`state.md` がひな形のままなら写さない） | `<root>/.sai/paper/` |
| 書き出し（`state.md` と `tree.md`） | `<root>/.sai/paper/` |

**手で足す設定はありません。** `init` だけで sci の設定が終わります。

### 4.4 5 つの型のどれを選ぶか

- **課題駆動** — 大目的（誰がどの場面で何に困っているか）だけが決まっている。対象もデータも手法も決まっていない。ここが典型。
- **現象駆動** — 必ず起きている現象（誰も疑っていない）が先にある。使う手法より先に観察できる事実がある。
- **手法駆動** — 手元で新しい手法（自己教師ありの分解など）がある。何に使うかが決まっていない。
- **データ駆動** — 手元のデータセット（病院信号など）が先にある。集めたときの目的とは限らない。
- **改良駆動** — すでに動いている手法がある。どこを直すかが本命。

型の説明は型の md（`sai-sci/types/<id>.md`）の本文と `given` に書いてあります。よくわからないときは `problem` から始めて、型を変えるには `sci init --type <型> --change-type` を打ってください。

---

## 5. 使い方の流れ

### 5.1 Web の「研究方針」の画面

論文タブの最初の画面が「研究方針」です（`#/papers/tree`）。

- 木を見て、節点を選ぶ（`#/papers/tree/N12`）と、その節点の問い・候補・知見・結び付き・決定の記録・行動が「板」に出ます。
- 板から行動（カード）を作ります。作ったカードが次に動くものです。
- 「仕分け」（`#/papers/triage`）で、エージェントの提案を採用・却下（採用済みのものは取り消し）します。
- 「読む札」（`#/papers/read/<run_id>`）は、利用者が 1 本の論文を読んで知見を書く画面です。

### 5.2 sai-link がブロックを動かすとき

`sci-*` のブロックは `context: true` です。sai-link は実行の直前に `.sai/settings.json` の `context` を実行し、標準出力を本文の `{{context}}` に入れます。`{{context}}` が無い本文では先頭に入ります。

**`{{context}}` を持つのは `sci-core.md` だけ**です。ブロックの本文には書きません（二重になります）。sci-core は範囲を作る 17 本（`sci-exp-branch` を除く）の `templates` に入るので、どのブロックを動かすときにも範囲が入ります。

この範囲は「対象の節点・上流の道・範囲外の項目・仮置き（その回で問い直さないと決めた値）」です。範囲を作れなかったときは 1 行の注意が `{{context}}` に入ります（実行は止まりません）。`block preview` は context を実行せず、`{{context}}` のまま見せます。

### 5.3 エージェントは提案だけを出す

- 自動で入るもの（知見・問い・問いへの回答・結び付き・種）は、出した提案がすぐ木に入り、自動で反映された印が付きます。
- 「仕分け」の画面を利用者が決めるもの（候補や項目の追加・決定・線・対象外にする）は、採用するまで木に入りません。
- どちらの場合も、利用者は 1 押しで取り消せます。

### 5.4 利用者が読む札

`sci card --runner user --paper <番号> --role <役割>` は「読む札」を作ります。`apps/phone` からは「読む」の画面で始めて、知見を書き、終えます（`sai-moon-link phone user start` → 知見を書く → `sai-moon-paper phone sci finish-read` → `sai-moon-link phone user finish`）。知見が 1 つも無いまま終えようとすると、理由（`--no-reflect`）を求められます。理由は論文の結び付きと節点と run の結び付きに残り、その論文は「反映済み」になります。

### 5.5 TUI の「研究方針」のページ

`sai-moon-paper-tui` の「研究方針」のページは**読むだけ**です。木と `tree.md` を見て、何も書き換えません。直すのは Web か `sai-moon-paper sci` だけです。

### 5.6 実験（seed と branch）

`sci-exp-seed` と `sci-exp-branch` の 2 本で、実験を**世代**で回します。1 世代は並列に走る branch 1 つ以上とひとかたまりで、`sci-exp-seed` が自分で次の世代を起動します。人が「続きを」と頼まなくても最後まで回ります。

```
sai card --block sci-exp-seed            種づくり（sci-seed）が終わった節点
        │
        ▼
┌──▶ 世代 1 の seed ─────────────────────────────────────────┐
│    ・実験仕様書 exp-spec-<study>.md を作る（最初の 1 回だけ） │
│    ・スクリプト <workdir>/<script>/g1.py を書く               │
│    ・branch を 1〜max_branches 個、並列に task_create する   │
└──┬──────────────────────────────────────────────────────────┘
   │  task_execute → task_output で全部待つ → finalize
   ▼
┌──▶ 世代 2 の seed ── 台帳 exp_summary.py を取る ────────────┐
│    ・判定の規則 R1〜R7 を上から当てて判定書 review-…-g2.md   │
│    ・stop なら完了報告書 exp-final-<study>.md を書いて終える  │
│    ・そうでなければ世代 3 のスクリプトを書いてまた branch      │
└──┴──────────────────────────────────────────────────────────┘
   │  次の seed のカードを作って起動する（sai card → phone card run）
   └─▶ 世代 3 …（`last.round >= max_generations` で stop）
```

**最初の seed の起動**は、`sci card` に種の札のトークンを渡して作る。`--input` は依頼文の末尾に載せる文書の waggle トークン（繰り返し可）。

```bash
sai-moon-paper sci card --node <N> --block sci-exp-seed \
  --purpose "<study> 世代 1 の計画" \
  --input <種の札のトークン> --json
sai-moon-link phone card run <返ってきた run_id>
```

**止め方**は 2 つ。どちらも利用者が決める。

- **自然に止まる**: 判定が `stop` になるときだけ。`max_generations`（既定 8 世代）に達したときと、成功基準に `ablation` まで済んで達したとき。
- **利用者に訊いて止める**: 同じ世代で branch の記録が 1 つも無いことが 2 代続けて続いたとき、または判定書が 2 回続けて作られなかったとき。実験を勝手に進めず、MLflow への接続などを訊いて止まる。

**実行先**（スクリプトを動かす PC）は環境変数 `EXP_TARGET` で選ぶ。空なら `local`（この PC）。実行先は `<root>/.sai/exec/<名前>/` のフォルダで、Mac 本体・LAN の PC・クラウドなどを利用者が個別に足せる（フォルダを置くだけ。増減で Rust や画面を直さない）。エージェントは分配器 `exp_exec.py`（`run` / `status` / `logs` / `cancel` / `check` / `list`）だけを呼び、実行先の中身を知らない。LAN の Linux の PC やクラウドの VM は、`ssh/` を PC の名前で写して host を書けば足せる（`cp -r .sai/exec/ssh .sai/exec/lab-linux`。コードを rsync で送り、状態は ssh で読む）。さくらの高火力 DOK は `dok/`（`README.md`・`SETUP.md` を先に読む。課金される）。Windows など別の種類は `_example/` を写して adapter を書く。約束は `exp_exec.py` の冒頭にある。記録は実行先から中央の MLflow に直接入る。台帳コマンド `exp_summary.py` と分配器は `sai-moon-paper sci init` が `<root>/.sai/tools/` に置く（環境変数は要らない）。

---

## 6. 命令の一覧

### 6.1 `sai-moon-paper sci …`

| 命令 | 何をするか |
|---|---|
| `sci init --type <型> [--change-type]` | 研究プロジェクトの部品を配り、木を作り、書き出す。既に始めているときは終了コード 1 |
| `sci status [--json]` | 現況（種の文・関門・負債・項目・次の一手）。`--json` が無いと人が読む文 |
| `sci node <N> [--json]` | 節点 1 つの中身（問い・候補・知見・結び付き・決定の記録・行動・候補の表の軸） |
| `sci context --run <run_id> [--scope <範囲>]` | 今の実行の範囲（研究方針の木から切り出した入力） |
| `sci propose [--run <run_id>] [--block <id>]` | 木への変更を提案する（標準入力に C-07 の JSON） |
| `sci apply` | 利用者の操作を 1 つ反映する（標準入力に C-07 の JSON） |
| `sci proposals [--json]` | 仕分け待ちの一覧 |
| `sci triage <P> (--accept [--edit <JSON>] \| --reject \| --revert) [--note <文>]` | 提案を仕分ける |
| `sci card --node <N> --block <id> --purpose <文> [--question <Q>] [--expect <文>] [--scope <範囲>] [--assume <N=値>]… [--mode <型>] [--paper <番号>]… [--runner pi\|cc\|user] [--role <役割>] [--depth <深さ>] [--origin-run <run_id>] [--input <トークン>]… [--json]` | 節点のための実行（カード）を作る。`--runner user` は読む札。`--input` は依頼文の末尾に載せる文書の waggle トークン（繰り返し可） |
| `sci read <run_id> [--json]` | 読む札の中身 |
| `sci finish-read <run_id> [--no-reflect <理由>]` | 読む札を終える（反映を `reflected` にする） |
| `sci export` | 木を `state.md` と `tree.md` に書き出す |
| `sci tombstones [--json]` | 墓標（外した論文）の一覧 |

書き込む命令（`init` / `propose` / `apply` / `triage` / `card` / `finish-read`）は、終わる前に `export` を実行します。

### 6.2 MCP の 4 つの道具（`sai-moon-paper mcp`）

| 道具 | 何をするか |
|---|---|
| `sci_context` | 今の実行の範囲を返す |
| `sci_status` | 研究方針の木の全体（種の文・関門・項目・負債・次の一手） |
| `sci_node` | 節点 1 つの中身 |
| `sci_propose` | 木への変更を提案する |

### 6.3 pi の拡張の 4 つの道具（`pi-litbase`）

| 道具 | 何をするか |
|---|---|
| `sai_paper_sci_context` | 今の実行の範囲を返す |
| `sai_paper_sci_status` | 研究方針の木の全体 |
| `sai_paper_sci_node` | 節点 1 つの中身 |
| `sai_paper_sci_propose` | 木への変更を提案する |

---

## 7. 置き場

| 物 | 置き場 |
|---|---|
| 木（項目・候補・問い・知見・結び付き・決定の記録・提案・種・検索の記録・墓標） | `papr.db`（migration `0046_sci_tree.sql`） |
| 書き出し（`state.md` と `tree.md`） | `<root>/.sai/paper/state.md` と `<root>/.sai/paper/tree.md` |
| 型と項目ガイドの写し | `<root>/.sai/sci/types/` と `<root>/.sai/sci/guides/` |

- **`state.md` を手で直さない。** 次の書き出しで消えます。直すのは Web か `sai-moon-paper sci` だけです。
- **写しは埋め込みより先に読まれます。** `<root>/.sai/sci/` に同じ相対パスの md があれば、それが使われます（無いときはバイナリに埋め込んだ版）。

---

## 8. md を直すとき（開発者向け）

- **型・項目ガイド・ブロックは `sai-moon-paper` に埋め込まれています**（`include_str!`）。`sai-sci/` の md を直したら `cargo build --release` で作り直さないと反映されません（`apps/phone` が叩くのは release の実行ファイル）。
- **形の決まり**: 型は `id` と「項目の順序」の節、項目ガイドは `key` と `parent` と `min_findings` の frontmatter が要ります。条文が崩れていれば 1 行の warning として `status` の `warnings` に入ります。
- **確かめ**:
  ```bash
  cd ~/workdir/Sai/sai-moon
  env -u SAI_PROJECT_ROOT -u SAI_PARENT_RUN_ID -u SAI_BLOCK_NAME \
    perl -e 'alarm 1800; exec @ARGV' cargo test -p sai-paper-core
  ```
  埋め込みの全ファイルが解析できること、各ブロックの op と範囲が読めること、を確かめます。
- **利用者のプロジェクトの `.sai/sci/` の写しは作り直しても変わりません。** 利用者が直した内容はそのまま残ります。
- 利用者が直した `.sai/sci/` の写しを優先して読むので、**配布の版を上げたいときは `.sai/sci/` の写しを消してから** `init` を打ち直します。

---

## 9. Canvas から移った人へ

- **Canvas の画面と `sai-moon-paper canvas` はありません。** 代わりに `#/papers/tree`（研究方針）と `sai-moon-paper sci` を使います。
- **今までの `.sai/paper/canvas/*.canvas` は消していません。** JSON Canvas 1.0 の形なので、他の道具（Obsidian など）で開けます。
- **`paper-map` は report だけを出すようになりました。** Canvas を作る手順は外れています。
- **古い paper-map の写し**（`<root>/.sai/blocks/paper-map.md`）には、無い命令（Canvas を作るなど）が書いてあることがあります。**その写しを消してから** `sai-moon-paper init --assets-only` を打つと、新しい物が入ります（無いファイルだけを書きます）。
