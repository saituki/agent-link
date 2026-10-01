# sai-sci（研究方針の木 / Research-tree）

**読み手**: 利用者と、`sai-sci/` の md を直す開発者。

初めて使う人は、手を動かしながら全部の機能を回る [`TUTORIAL.md`](TUTORIAL.md) から始めてください。

`sai-sci` は、**研究の方向づけの段階（実験の前にある「何をやるか」を決める段階）を、研究方針の木（Research-tree）で扱う仕組み**です。
木を紙に描いたり表に整理したりするのではなく、木の正を 1 か所（`papr.db`）に置き、絵と文章はその正からの書き出しにします。

- 木は、論文の置き場があるプロジェクトごとに 1 つあります。
- Web（`apps/phone`）・TUI・CLI・MCP・pi の拡張が、同じ木を読み書きします。
- エージェントは木に PROPOSAL を出します。自動で入るのは FINDING・QUESTION・LINK・SEED で、木の形が変わる操作は利用者が TRIAGE します。

## 1. sai-sci とは

`sai-sci` は、**研究の方向づけの段階（実験の前）を、研究方針の木（Research-tree）で扱う仕組み**です。

- **木の正は `papr.db`** です。ITEM・OPTION・QUESTION・FINDING・LINK・DECISION_LOG・PROPOSAL・SEED は、いずれも `sci_` で始まる表に入っています（migration `0046_sci_tree.sql`）。
- **`state.md` と `tree.md` は書き出し**です。手で直しても次の書き出しで消えます。
- **木は型が決めるときの骨組みの形になります。** RESEARCH_TYPE（5 つの型）ごとに決まる ITEM_ORDER（項目の順序）と GATE（通るための条件の並び）があります。
- **エージェントは PROPOSAL を出すだけです。**木に直接書き込む処理は入れません。

用語の読み方:

画面の専門語は英語の大文字（UPPER_SNAKE）です。次の表で日本語の名前と対応を確かめられます。

| 用語（英語） | 日本語 | 何 |
|---|---|---|
| SEED_SENTENCE | 種の文 | 研究の型ごとの 1 文のひな形。決まった ITEM の値が入り、まだ決まっていない所は［？］で出る |
| GATE | 関門 | 型ごとに決めた通過条件の並び。最初に通っていない GATE が「今いる所」。名前は `BACKGROUND_GOAL_DECIDED`・`DATA_ACCESS_DECIDED`・`NOVELTY_CLOSEST_AND_DIFF` など（条件から決まる名前） |
| NODE | 節点 | ITEM と OPTION の総称 |
| ITEM / OPTION | 項目 / 候補 | ITEM は研究の穴（例: 小目的）。選ぶ・並べる ITEM には OPTION（発作部位の予測など）が付く |
| ITEM_TYPE | 項目の型 | CHOOSE（選ぶ）・LIST（並べる）・WRITE（書く）・VERIFY（確かめる） |
| RESEARCH_TYPE | 研究の型 | PROBLEM_DRIVEN・DATA_DRIVEN・METHOD_DRIVEN・PHENOMENON_DRIVEN・IMPROVE_DRIVEN。型の id（`problem` `data` など）は変わらない |
| BLANK | 空欄 | ITEM がまだ決まっていない所。BLANK には種類（UNTOUCHED・IN_PROGRESS・INSUFFICIENT・AWAITING_DECISION・CONFLICT。未着手・調査中・情報不足・判断待ち・対立）が付く |
| MARK | 印 | 決まっている ITEM でも、根拠が薄い・上流が変わっているものに付く印。種類は WEAK_EVIDENCE（根拠不足）・RECHECK（要再確認） |
| DEBT | 負債 | 先に片付けたいもの。種類は PENDING_PROPOSALS（仕分けていない提案）・UNREFLECTED_PAPERS（反映していない論文）・UNSCREENED_SETS（選別していない検索）・EMPTY_REVIEWS（書いていない振り返り） |
| FINDING | 知見 | 論文などから分かった 1〜3 文の事実と CITE（出典）。POLARITY（向き。SUPPORT・REFUTE・NEUTRAL・NONE）を持つ |
| PROPOSAL | 提案 | エージェントが出した変更の申し出。AUTO_OP（自動で入るもの）と REVIEW_OP（利用者が仕分けるもの）がある |
| TRIAGE | 仕分け | PROPOSAL を採用（ACCEPT）するか却下（REJECT）するか、採用済みを取り消す（REVERT）かを決めること |
| TOMBSTONE | 墓標 | 外した論文の記録。もう一度検索に出ないように残す |
| SCOPE | 範囲 | 実行に切り出す木の範囲。NODE・PATH・SUBTREE・SIBLINGS・SUMMARY・BLIND |
| CARD | カード | `sci card` で作る実行の依頼（task.md） |
| READ_CARD | 読む札 | 利用者が 1 本の論文を読んで FINDING を書くための CARD（`runner: user`） |

用語の正は `docs/20261001-1806-research-tree-glossary-english.md` の 3 章。名前の付け方は 4 章。

---

## 2. 3 つの考え

### 2.1 骨組みと OPTION の木

研究の穴（ITEM）と、その穴の OPTION を木で表す。型が決まると、一番上の ITEM の並び（骨組み）と、その下に OPTION を置くかが決まる。
木には親子（`parent`）のほかに、DEPENDS（依存）・DERIVED（由来）・ANALOGOUS（同型）の線も引ける。

### 2.2 FINDING を 1 級の物として扱う

「読んだ論文のメモ」ではなく、**CITE（出典）と POLARITY（向き）を持った FINDING（F31 など）**が木の 1 級の要素です。
どの ITEM を支持したのか、どの OPTION を反証したのかが、木の上でたどれる。

### 2.3 変更は危険度で分ける

| 危険度 | それは | 例 |
|---|---|---|
| 自動（AUTO_OP） | 出した PROPOSAL はすぐ木に入る。取り消せるので、迷ったら出してよい | FINDING・QUESTION・LINK・SEED |
| 人（REVIEW_OP） | PROPOSAL は TRIAGE の待ち箱で待つ。利用者が ACCEPT すると反映される | OPTION・ITEM の追加・DECIDED にする・線・EXCLUDED にする |

---

## 3. フォルダの中身

```
sai-sci/
├── types/      # 5 つの型（problem・phenomenon・method・data・improve）
├── guides/     # ITEM_GUIDE（項目ガイド。上の ITEM 9 本・子の ITEM 16 本）
├── template/   # sci-core.md（エージェントの共通の決まり）・exp-core.md・exp-run.md（実験ループの決まり）・sai-agent.md・sai-task.md・sai-cli.md（professor と operator の共通の決まり）・bw-core.md（逆向きの共通の決まり）
├── mind.md     # SOIL のひな形
├── tools/      # 実験ループの道具: 台帳 exp_summary.py・実行先への分配器 exp_exec.py・Web と同じ操作の phone_api.py とそのテスト（`sci init` が <root>/.sai/tools/ へ配る）
├── exec/       # 実行先の見本: local（この PC・有効）・ssh（Linux の PC・無効の見本）・dok（さくらの高火力 DOK・課金あり・無効の見本）・_example（雛形）。`sci init` が <root>/.sai/exec/ へ配る
└── blocks/     # 25 本のブロック（professor・professor-backward・operator を含む）
```

### 3.1 5 つの型

`types/` の 5 本。各型は ITEM_ORDER（項目の順序）・SEED_SENTENCE（種の文）・GATE（関門）を持つ。`given` は「最初から与えられているもの」。

| id | 名前 | given | どれを選ぶか |
|---|---|---|---|
| `problem` | PROBLEM_DRIVEN（課題駆動） | `background` | 現場で今のやり方では形にならない問題が残っており、「どうにかしたい」という課題から研究が始まる。決まっているのは大目的だけ |
| `phenomenon` | PHENOMENON_DRIVEN（現象駆動） | `phenomenon` | 必ず起きている現象が先にあり、「なぜそうなるのか」を問う。観察できる事実が手法より先にある |
| `method` | METHOD_DRIVEN（手法駆動） | `method` | 手元で新しい手法が使える状態が先にあり、「それを何に使うか」を決める。手法の中身は既に決まっている |
| `data` | DATA_DRIVEN（データ駆動） | `data` | 手元のデータセットが先にあり、「このデータで何が言えるか」を問う。集めたときの目的とは限らない |
| `improve` | IMPROVE_DRIVEN（改良駆動） | `existing` | すでに動いている手法があり、「これをよくする」ことを目標にする。何を直すかはおおむね決まっている |

### 3.2 ITEM_GUIDE（上の ITEM 9 本）

`guides/` の直下に無印で置かれる。一番上の ITEM。名前は key の `.` を `_` にして大文字にしたもの（旧い日本語名を括弧に残す）。型は ITEM_TYPE（書く=WRITE・選ぶ=CHOOSE・並べる=LIST・確かめる=VERIFY）。

| key | 名前 | 型 |
|---|---|---|
| `background` | BACKGROUND（背景） | WRITE |
| `target` | TARGET（対象） | CHOOSE |
| `data` | DATA（データ） | CHOOSE・VERIFY |
| `purpose` | PURPOSE（目的） | CHOOSE |
| `existing` | EXISTING（既存手法） | LIST |
| `issue` | ISSUE（課題） | VERIFY |
| `method` | METHOD（手法） | LIST・CHOOSE |
| `novelty` | NOVELTY（新規性） | WRITE・VERIFY |
| `phenomenon` | PHENOMENON（現象） | WRITE・VERIFY |

### 3.3 ITEM_GUIDE（子の ITEM 16 本）

| key | 名前 | 型 | 親 |
|---|---|---|---|
| `background.goal` | BACKGROUND_GOAL（大目的） | WRITE | `background` |
| `background.problem` | BACKGROUND_PROBLEM（問題の背景） | WRITE・VERIFY | `background` |
| `target.subgroup` | TARGET_SUBGROUP（対象の細分） | CHOOSE | `target` |
| `data.type` | DATA_TYPE（データの型） | CHOOSE | `data` |
| `data.size` | DATA_SIZE（データの量） | WRITE・VERIFY | `data` |
| `data.label` | DATA_LABEL（ラベル） | CHOOSE | `data` |
| `data.access` | DATA_ACCESS（入手） | CHOOSE・VERIFY | `data` |
| `purpose.subgoal` | PURPOSE_SUBGOAL（小目的） | CHOOSE | `purpose` |
| `purpose.output` | PURPOSE_OUTPUT（出力の型） | CHOOSE | `purpose` |
| `purpose.metric` | PURPOSE_METRIC（評価指標と成功基準） | WRITE | `purpose` |
| `issue.limit` | ISSUE_LIMIT（既存手法の課題） | LIST・VERIFY | `issue` |
| `issue.root` | ISSUE_ROOT（根本の原因） | WRITE・VERIFY | `issue` |
| `method.analog` | METHOD_ANALOG（同型の問題） | LIST | `method` |
| `method.adapt` | METHOD_ADAPT（取り入れ方） | WRITE | `method` |
| `novelty.closest` | NOVELTY_CLOSEST（最も近い研究） | LIST | `novelty` |
| `novelty.diff` | NOVELTY_DIFF（差分） | WRITE・VERIFY | `novelty` |

### 3.4 26 本のブロック

`blocks/` の 26 本。`sai-moon-paper` はこれらを `--assets-only` の `init` や `sci init` でプロジェクトの `.sai/blocks/` に配る。
`sci_scope` は「このブロックの行動に切り出す範囲」、`sci_ops` は「このブロックで PROPOSAL に出せる op」。

- 表の 23 本は木の中で動く researcher（4 つの層 seed / root / sprout / branch）。うち 4 本（`sci-restore`・`sci-cite-check`・`sci-tree-review`・`sci-result-compare`）は逆向きの運用（5.7 節）のためのもの。
- 残りの 3 本（`professor`・`professor-backward`・`operator`）は木の中で動かず、木の外から木を見る役。下の 3.4.1 に書く。

#### 3.4.1 3 役の関係

```
professor / professor-backward（root の run。researcher を直接起こす）
  |-- researcher（sci-* の 23 本。木の 4 つの層（seed / root / sprout / branch）で働く）
  `-- operator（professor の子。sai-moon の操作・監視・情報の抽出だけ）
```

| id | 名前 | 役 | 共通の決まり | 研究の判断 |
|---|---|---|---|---|
| `professor` | 🎓 professor | 研究の現在地を読み、次の研究作業を researcher に任せ、報告を評価して研究方針（木の TRIAGE）をする | `sai-agent` / `sai-task` / `sai-cli` | する。root の run になる。研究作業は自分でしない |
| `professor-backward` | 🔙 professor-backward | 既存の論文から始める逆向きの舵取り。抽出 → 評価 → 再現 → 更新 → 比較の順に researcher を起こす（5.7 節） | `sai-agent` / `sai-task` / `sai-cli` / `bw-core` | する。root の run になる。研究作業は自分でしない |
| `operator` | 🛠 operator | sai-moon の故障の対処・監視・情報の抽出・Web と同じ操作 | `sai-agent` / `sai-cli` | しない。professor の子。子の `task_id` で professor の子を扱える |
| `sci-*` | （23 本） | 研究作業そのもの（読む・比べる・OPTION を出す・考える・書く・探す・実験する） | `sci-core`（+ `literature-hard-rules` / `domain` / `exp-core`） | しない。カード（`--executer professor` で起こすもの）に書かれた作業だけ |

- カードの `executer` は起動者（`human` / `professor`）。`--executer professor` のカードは人間が Web から起こせず、professor が `task_create` の `card` で自分の子として起こす。
- 子からの質問は親の `task_output` に `stalled`（`reason: "question"`）で届き、`phone_api.py question` / `answer` で答える。Web で人間が先に答えた場合は 400 で断られる。
- 完了は、子は `task_report`、root（professor）は質問ツールで「完了してよいか」を確かめて `sai_finalize` に `report`。報告は run フォルダの `finalize-report.md`（`sai-moon-link phone md --kind report <run_id>` で読む）。
- 研究日誌は `.sai/paper/professor-log.md`。operator の学びは `~/.sai-moon/operator-notes.md`（`SAI_SAI_HOME` があればその中）。
- 3 役が使う共通の決まりは `template/sai-agent.md`・`sai-task.md`・`sai-cli.md` の 3 本（3.5 節）。sai-moon 自身の `.sai/template/sai-base.md` などを元に、研究プロジェクト向けに書き直したもの。

| id | 名前 | 類 | SCOPE | 出せる op（ALLOWED_OPS） | 呼べる子ブロック |
|---|---|---|---|---|---|
| `sci-start` | 🌱 研究の入口 | outline | summary | add_finding, add_question, answer_question, link, add_option, add_child, edit_text, decide | sci-search, sci-lookup |
| `sci-outline` | 🧭 全体の見直し | outline | summary | add_question, link, add_option, reopen, exclude, add_edge | sci-lookup |
| `sci-digest` | 🗒 まとめ | outline | summary | add_question | ― |
| `sci-seed` | 🌰 種づくり | outline | summary | add_seed, add_question, add_finding | sci-lookup |
| `sci-read` | 📖 1 本を読む | detail | path | add_finding, add_question, answer_question, link, add_option, exclude | sci-lookup |
| `sci-compare` | ⚖ 比べる | detail | path | add_finding, link, add_option, decide, option_state | sci-lookup, sci-extract |
| `sci-probe` | 🔍 確かめる | detail | path | add_finding, add_question, link | sci-search, sci-lookup |
| `sci-draft` | ✍ 文を書く | detail | path | edit_text, add_finding, add_question | sci-lookup |
| `sci-ideate` | 💡 OPTION を増やす | detail | path | add_option, add_child, add_question, add_finding, link | sci-analogy, sci-lookup |
| `sci-debate` | 🗣 議論 | detail | path | add_finding, add_question, link | paper-debater, paper-reviewer |
| `sci-coach` | 🎓 理解の確認 | detail | node | add_question, answer_question | ― |
| `sci-search` | 🔎 外部を探す | support | node | link, add_finding | ― |
| `sci-screen` | 🧹 選別 | support | node | link, exclude, add_question | ― |
| `sci-extract` | 📋 抜き出す | support | node | add_finding, link | ― |
| `sci-lookup` | 📚 Library で答える | support | node | add_finding, link | ― |
| `sci-analogy` | 🔁 同型を探す | support | node | add_option, add_finding, link, add_question | sci-search |
| `sci-exp-sprout` | 🌱 exp-sprout | sprout | path | add_finding, add_question, answer_question, link | ― |
| `sci-exp-branch` | 🌿 exp-branch | branch | ― | ― | ― |
| `sci-exp-review` | 🔍 exp-review | branch | path | add_finding, add_question, answer_question, link | ― |
| `sci-restore` | 🔙 論文から復元 | backward | summary | add_finding, add_question, link, add_option, add_child, edit_text, decide, add_seed | sci-lookup |
| `sci-cite-check` | 🔗 引用を確かめる | backward | path | add_finding, add_question, link | sci-lookup |
| `sci-tree-review` | 🩺 復元した木の審査 | backward | summary | add_finding, add_question, link, reopen, add_edge | sci-lookup |
| `sci-result-compare` | 📊 結果を比べる | backward | summary | add_finding, add_question, link | sci-lookup |

- 先の 16 本は全部 `context: true`（実行の直前に研究方針の木から範囲を作る）・`model: inherit`・`worktree: none`。
- 3 つの類で `timeout` が違う。outline と detail は 5400 秒、support は 3600 秒。
- `sci-debate` の参加者は `paper-debater` と `paper-reviewer`（普通の `paper-*` のブロック）。
- 後ろの 3 本は**実験ループ**（5.6 節）。`sci-exp-branch` だけは木の範囲も共通の決まり（`sci-core`）も受け取らない。
- 最後の 4 本（類が backward）は逆向きの運用（5.7 節）のためのもの。4 本とも `sci-core` と `bw-core` を受け、`context: true`・`layer: root`・`timeout: 5400`。
- `layer` は 4 つ。`seed`（`sci-start`・`sci-seed`）・`root`（上の 14 本と逆向きの 4 本）・`sprout`（`sci-exp-sprout`）・`branch`（`sci-exp-branch`・`sci-exp-review`）。図では上から branch / sprout / seed / root。書かないブロックは図に出さない。

### 3.5 `template/` と `mind.md`

- **`template/sci-core.md`** — エージェントに共通で渡す決まり。最初のメッセージの「# context」に入る SCOPE の読み方、確かめの地点、PROPOSAL の書き方、使える 14 種類の op、根拠の扱い、迎合しない姿勢。
- **`template/exp-core.md`** — 実験ループ（seed と branch）の共通の決まり。役割、文書の渡し方、実験仕様書、MLflow の記録の決まり（C-04）、台帳、判定の規則、依頼文の形、実行場所の規則、停止の条件。
- **`template/exp-run.md`** — 実験のスクリプトを「実行先」に投げる手順（分配器 `exp_exec.py` の呼び方）。`exp-core` の「実行場所の規則」と対で読む。
- **`template/sai-agent.md`** — 研究プロジェクトのエージェントに共通で渡す決まり（professor と operator が使う）。文書だけでの引き継ぎ・waggle トークン・`sai_write` / `sai_read`・書き方の原則・質問のしかた・合言葉と鍵の扱い・完了のしかた。
- **`template/sai-task.md`** — 子タスクの扱い（professor が使う）。流れ・`task_output` の status ごとの対処・`task_manage` の表・予約済みのカードから子を起こす形・子の質問への対処・報告を受け取ったあと。
- **`template/bw-core.md`** — 逆向きの運用の共通の決まり（逆向きの 4 本と `professor-backward` が使う）。5 つの段、基盤論文の扱い、論文の節と木の ITEM の対応、主張と根拠の分け方、主張と引用の対応表、引用の判定の語、記載結果表、SEED の札（再現）、CRITERIA（評価基準。E1〜E10）、COMPARISON_CRITERIA（比較項目。K1〜K8）。
- **`template/sai-cli.md`** — 研究プロジェクトでの命令（professor と operator が使う）。実行を調べる命令・研究方針の木の命令・カードを作る命令・`phone_api.py`・実行先・使わない命令。
- **`tools/phone_api.py`** — sai-phone（Web の窓口）の HTTP API を呼ぶ道具。質問・承認・root の run への送信・MLflow の読み取り。Web と同じ操作がコマンドラインからできる。
- **`mind.md`**（`sai-sci/mind.md`）— SOIL（土壌）のひな形。`ORIGIN・RESEARCH_TYPE・GIVEN・CONSTRAINTS・VALUE_AXES・INTERESTS・NOTES` の 7 節（旧 始まり・研究の型・与えられていること・制約・価値の軸・興味がある/無い・メモ）。旧い日本語の見出しも読みます。**利用者が書くファイルで、エージェントは直接書き換えません。** VALUE_AXES は CHOOSE の ITEM を比べる軸として `sci node` に出す。

---

## 4. 始め方

### 4.1 新しいプロジェクト

```bash
sai-moon-paper init --sci --type <型>
```

`init` 自身を終えたあとに `sci init --type <型>` と同じことをします。`mind.md` が無ければ、`sai-sci/mind.md` の SOIL のひな形を書きます（普通の `project_files.rs:13` のひな形の代わり）。

配り終えたら、最初の `professor` のカードを作って人が起こす。professor が現在の地を読み、次の一手を選んで researcher を起こす。

```bash
sai-moon-paper sci card --node <N1> --block <最初のブロックの id> \
  --purpose "<研究の構想>" --executer human --origin-run <run_id> --json
```

`--executer human`（既定）なら Web からそのカードを実行できる。`--executer professor` のカード（研究作業）は、professor が `task_create` の `card` で自分の子として起こす。

### 4.2 今のプロジェクト

```bash
sai-moon-paper sci init --type <型>
```

既に始めているときは終了コード 1 で「既に始めています（型: 課題駆動）」が出ます。型を変えるには `--change-type` を付けます。

### 4.3 入る物

`sci init`（と `init --sci`）は次を配ります。

| 入るもの | 置き場 |
|---|---|
| 22 本の `sci-*` ブロックと `professor.md`・`professor-backward.md`・`operator.md` | `<root>/.sai/blocks/` |
| `sci-core.md`・`exp-core.md`・`exp-run.md`・`sai-agent.md`・`sai-task.md`・`sai-cli.md`・`bw-core.md`（と各ブロックが使う `search-internal` などのテンプレ、`domain`） | `<root>/.sai/template/` |
| 台帳 `exp_summary.py`（判定の材料を取る道具）と分配器 `exp_exec.py`（無いときだけ書く） | `<root>/.sai/tools/` |
| `phone_api.py`（Web と同じ操作をする道具。無いときだけ書く） | `<root>/.sai/tools/` |
| 実行先の見本 `local`・`_example`（無いときだけ書く） | `<root>/.sai/exec/` |
| `.sai/settings.json` の `context`（実行直前に範囲を作る命令。無いときだけ書く） | `<root>/.sai/settings.json` |
| `mind.md`（SOIL のひな形。無いときだけ書く） | `<root>/.sai/paper/mind.md` |
| 型と ITEM_GUIDE の写し | `<root>/.sai/sci/types/` と `<root>/.sai/sci/guides/` |
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

- 木を見て、NODE を選ぶ（`#/papers/tree/N12`）と、その NODE の QUESTION・OPTION・FINDING・LINK・DECISION_LOG・ACTION_LOG が「板」に出ます。
- 板から行動（カード）を作ります。作ったカードが次に動くものです。
- TRIAGE（`#/papers/triage`）で、エージェントの PROPOSAL を ACCEPT・REJECT（採用済みのものは REVERT）します。
- READ_CARD（`#/papers/read/<run_id>`）は、利用者が 1 本の論文を読んで FINDING を書く画面です。

### 5.2 sai-link がブロックを動かすとき

`sci-*` のブロックは `context: true` です。sai-link は実行の直前に `.sai/settings.json` の `context` の命令を実行し、標準出力を、最初のメッセージの `# context` の節に入れます（`# context` の見出しは sai-link が付けます）。依頼文は同じメッセージの `# task_text` の節に入ります。

ブロックの本文・template に `{{context}}` を書く必要は無い（古い写しに残っていても互換で読めます。二重にもなりません）。sci-core は範囲を作る 21 本（`sci-exp-branch` を除く `sci-*`）の `templates` に入るので、どのブロックを動かすときにも範囲が入ります。

この範囲は「TARGET_NODE・UPSTREAM_PATH・OUT_OF_SCOPE の ITEM・ASSUMPTION（その回で問い直さないと決めた値）」です。範囲を作れなかったときは 1 行の注意が `# context` の下に入ります（実行は止まりません）。`block preview` は context を実行せず、`# context` の下に「（実行番号が無いため、範囲を作れませんでした）」の 1 行を見せます。

**決まり:** `.sai/settings.json` の `context` に別の命令を指定するときは、その出力も `##` から始めてください（`# context` の下に並びます）。

### 5.3 エージェントは PROPOSAL だけを出す

- AUTO_OP（FINDING・QUESTION・QUESTION への回答・LINK・SEED）は、出した PROPOSAL がすぐ木に入り、自動で反映された印が付きます。
- REVIEW_OP（TRIAGE の画面で利用者が決めるもの。OPTION や ITEM の追加・DECIDED にする・線・EXCLUDED にする）は、採用するまで木に入りません。
- どちらの場合も、利用者は 1 押しで取り消せます。

### 5.4 利用者が読む READ_CARD

`sci card --runner user --paper <番号> --role <役割>` は READ_CARD を作ります。`apps/phone` からは READ_CARD の画面で始めて、FINDING を書き、終えます（`sai-moon-link phone user start` → FINDING を書く → `sai-moon-paper phone sci finish-read` → `sai-moon-link phone user finish`）。FINDING が 1 つも無いまま終えようとすると、理由（`--no-reflect`）を求められます。理由は論文の LINK と NODE と run の結び付きに残り、その論文は「反映済み」になります。

### 5.5 TUI の「研究方針」のページ

`sai-moon-paper-tui` の「研究方針」のページは**読むだけ**です。木と `tree.md` を見て、何も書き換えません。直すのは Web か `sai-moon-paper sci` だけです。

### 5.6 実験（sprout・branch・review）

`sci-exp-sprout`・`sci-exp-branch`・`sci-exp-review` の 3 本で実験を回します。3 本は互いの終わりを待ちません。sprout は最初の branch を起こして終わり、branch はジョブ待ちの review のカードを作って終わり、review はジョブが終わると daemon に起こされます。

```
sci card --block sci-exp-sprout            SEED の札ができた NODE
        │
        ▼
 sprout ── 実験仕様書を作り、最初の branch を 1〜2 本起こして終わる
        │  （sci card --block sci-exp-branch → phone card run）
        ▼
 branch ── スクリプト 1 本を書いて実行先に投げる。始まりを確かめたら、
        │  ジョブ待ちの review のカードを作って終わる（phone card wait --job）
        ▼  …ジョブが終わると daemon が起こす
 review ── 台帳で判定する。tune・debug → 同じ experiment の次の branch、
        │  model・ablation → 新しい experiment の次の branch、stop → 完了報告書
        └─▶ 次の branch …   （stop で終わる。起動元が professor なら起こし直す）
```

- **MLflow の形**: experiment は `<project>/ID<sprout>/<model>`、run は `ID<branch>-p<NN>`。epoch ごとに `train_loss`・`val_loss` を記録する。
- **止め方**: 判定が `stop` のときだけ。`max_branches` が branch の総数の上限になる。
- **最初の sprout の起動**は、`sci card` に SEED の札のトークンを渡して作る。`--input` は依頼文の末尾に載せる文書の waggle トークン（繰り返し可）。

```bash
sai-moon-paper sci card --node <N> --block sci-exp-sprout \
  --purpose "<study> の計画" \
  --input <SEED の札のトークン> --json
sai-moon-link phone card run <返ってきた run_id>
```

- 詳しい決まりは `template/exp-core.md`。

**実行先**（スクリプトを動かす PC）は環境変数 `EXP_TARGET` で選ぶ。空なら `local`（この PC）。実行先は `<root>/.sai/exec/<名前>/` のフォルダで、Mac 本体・LAN の PC・クラウドなどを利用者が個別に足せる（フォルダを置くだけ。増減で Rust や画面を直さない）。エージェントは分配器 `exp_exec.py`（`run` / `status` / `logs` / `cancel` / `check` / `list`）だけを呼び、実行先の中身を知らない。LAN の Linux の PC やクラウドの VM は、`ssh/` を PC の名前で写して host を書けば足せる（`cp -r .sai/exec/ssh .sai/exec/lab-linux`。コードを rsync で送り、状態は ssh で読む）。さくらの高火力 DOK は `dok/`（`README.md`・`SETUP.md` を先に読む。課金される）。Windows など別の種類は `_example/` を写して adapter を書く。約束は `exp_exec.py` の冒頭にある。記録は実行先から中央の MLflow に直接入る。台帳コマンド `exp_summary.py` と分配器は `sai-moon-paper sci init` が `<root>/.sai/tools/` に置く（環境変数は要らない）。

### 5.7 逆向き（既存の論文から始める）

前向き（5.1〜5.6）は研究の構想から木を育てます。**逆向き**は、既存の論文（基盤論文 1〜5 本）を出発点にして、論文に書いてある研究方針を木に復元し、その根拠を確かめ、論文の手法を再現し、新しい論文で直し、比べます。木・PROPOSAL・CARD・実験ループは前向きと同じで、運用の順番だけが違います。決まりは `template/bw-core.md` にまとめてあります。

| 段 | すること | ブロック |
|---|---|---|
| 1. 抽出 | 序論を root の ITEM に、手法と実験の設定を seed の ITEM に割り当てる。結果の数値を記載結果表に、主張と引用の組を対応表にする。論文の手法を動かす SEED の札（再現）を書く | `sci-restore` |
| 2. 評価 | 主張ごとに引用文献を取り寄せて、根拠になっているかを判定する。そのうえで、復元した木を CRITERIA（E1〜E10）で審査する | `sci-cite-check` → `sci-tree-review` |
| 3. 再現 | SEED の札（再現）で論文の手法を動かす | `sci-exp-sprout` |
| 4. 更新 | 基盤論文より新しい論文を探して ITEM を直し、新しい SEED を作って動かす | 前向きのブロック（`sci-search` から `sci-exp-sprout` まで） |
| 5. 比較 | 論文の記載値・再現・更新を COMPARISON_CRITERIA（K1〜K8）で比べる | `sci-result-compare` |

- **始め方**: 基盤論文を Library に入れ（Web の論文の画面）、木を作ってから（`sai-moon-paper init --assets-only --sci --type <型>`）、`professor-backward` のカードを人が起こします。依頼文に `基盤論文: #12 #15` の形で番号を書きます。
- **主張と根拠を分けます。** 基盤論文の文は「主張」として FINDING にし（POLARITY は NEUTRAL）、ITEM は **TENTATIVE** で入ります。引用文献で支えられ、審査で「良」になった ITEM だけを professor-backward が DECIDED に上げます。前向きの GATE は DECIDED で通るので、評価が済むまで GATE は通りません。
- **再現は論文に合わせにいきません。** 論文に無い設定だけを ASSUMPTION として置き、論文に書いてある値と手法は変えません。判定が `new_script`（別の手法を試す）になったら、再現はそこで止めて比較の材料にします。
- **Human に聞くこと**: 再現の許容幅（論文の記載値 ± どれだけなら再現できたとするか）、実行先（課金の有無）、同じ型の論文が 2 本以上あるときにどれを再現するか、段を飛ばすか。
- 研究日誌は `.sai/paper/professor-backward-log.md` です。

---

## 6. 命令の一覧

### 6.1 `sai-moon-paper sci …`

| 命令 | 何をするか |
|---|---|
| `sci init --type <型> [--change-type]` | 研究プロジェクトの部品を配り、木を作り、書き出す。既に始めているときは終了コード 1 |
| `sci status [--json]` | 現況（SEED_SENTENCE・GATE・DEBT・ITEM・NEXT_ACTION）。`--json` が無いと人が読む文 |
| `sci node <N> [--json]` | NODE 1 つの中身（QUESTION・OPTION・FINDING・LINK・DECISION_LOG・ACTION_LOG・OPTION の表の軸） |
| `sci context --run <run_id> [--scope <SCOPE>]` | 今の実行の SCOPE（研究方針の木から切り出した入力） |
| `sci propose [--run <run_id>] [--block <id>]` | 木への変更を PROPOSAL として出す（標準入力に C-07 の JSON） |
| `sci apply` | 利用者の操作を 1 つ反映する（標準入力に C-07 の JSON） |
| `sci proposals [--json]` | TRIAGE 待ちの PROPOSAL の一覧 |
| `sci triage <P> (--accept [--edit <JSON>] \| --reject \| --revert) [--note <文>]` | PROPOSAL を TRIAGE する |
| `sci card --node <N> --block <id> --purpose <文> [--question <Q>] [--expect <文>] [--scope <範囲>] [--assume <N=値>]… [--mode <型>] [--paper <番号>]… [--runner pi\|cc\|user] [--role <役割>] [--depth <深さ>] [--origin-run <run_id>] [--input <トークン>]… [--json]` | NODE のための実行（CARD）を作る。`--runner user` は READ_CARD。`--input` は依頼文の末尾に載せる文書の waggle トークン（繰り返し可） |
| `sci read <run_id> [--json]` | READ_CARD の中身 |
| `sci finish-read <run_id> [--no-reflect <理由>]` | READ_CARD を終える（反映を `reflected` にする） |
| `sci export` | 木を `state.md` と `tree.md` に書き出す |
| `sci retitle [--dry-run]` | 旧い日本語のガイド名のままの ITEM の題名を、今のガイドの名前へ移す。先に `--dry-run` で一覧を見る |
| `sci tombstones [--json]` | TOMBSTONE（外した論文）の一覧 |

書き込む命令（`init` / `propose` / `apply` / `triage` / `card` / `finish-read`）は、終わる前に `export` を実行します。

論文の節と塊を読む命令（`sai-moon-paper content …`）:

| 命令 | 何をするか |
|---|---|
| `content outline <番号> [--json]` | 論文の節の一覧（節の ID `#37.5`・文の数・要約）。MCP の `paper_outline` と同じ |
| `content read <ID>… [--max-chars N] [--json]` | 節・塊を読む（`37`・`37.5`・`37.5.2`・`37.5.2-4`）。MCP の `paper_read` と同じ |
| `content build [<番号>…] [--all] [--force] [--summarize]` | 節と塊の表を作る（作り直す・要約を付ける） |

### 6.2 MCP の道具（`sai-moon-paper mcp`）

| 道具 | 何をするか |
|---|---|
| `sci_context` | 今の実行の SCOPE を返す |
| `sci_status` | 研究方針の木の全体（SEED_SENTENCE・GATE・ITEM・DEBT・NEXT_ACTION） |
| `sci_node` | NODE 1 つの中身 |
| `sci_propose` | 木への変更を PROPOSAL として出す |
| `paper_outline` | 論文の節の一覧（節の ID・文の数・要約） |
| `paper_read` | 節・塊の本文を読む（出典は塊の ID `[#37.5.2]`） |

### 6.3 pi の拡張の 4 つの道具（`pi-litbase`）

| 道具 | 何をするか |
|---|---|
| `sai_paper_sci_context` | 今の実行の SCOPE を返す |
| `sai_paper_sci_status` | 研究方針の木の全体 |
| `sai_paper_sci_node` | NODE 1 つの中身 |
| `sai_paper_sci_propose` | 木への変更を PROPOSAL として出す |

---

## 7. 置き場

| 物 | 置き場 |
|---|---|
| 木（ITEM・OPTION・QUESTION・FINDING・LINK・DECISION_LOG・PROPOSAL・SEED・検索の記録・TOMBSTONE） | `papr.db`（migration `0046_sci_tree.sql`） |
| 書き出し（`state.md` と `tree.md`） | `<root>/.sai/paper/state.md` と `<root>/.sai/paper/tree.md` |
| 型と ITEM_GUIDE の写し | `<root>/.sai/sci/types/` と `<root>/.sai/sci/guides/` |
| professor の研究日誌 | `<root>/.sai/paper/professor-log.md` |
| professor-backward の研究日誌 | `<root>/.sai/paper/professor-backward-log.md` |
| operator の学び | `~/.sai-moon/operator-notes.md`（`SAI_SAI_HOME` があればその中。無ければ作らない） |

- **`state.md` を手で直さない。** 次の書き出しで消えます。直すのは Web か `sai-moon-paper sci` だけです。
- **写しは埋め込みより先に読まれます。** `<root>/.sai/sci/` に同じ相対パスの md があれば、それが使われます（無いときはバイナリに埋め込んだ版）。

---

## 8. md を直すとき（開発者向け）

- **型・ITEM_GUIDE・ブロックは `sai-moon-paper` に埋め込まれています**（`include_str!`）。`sai-sci/` の md を直したら `cargo build --release` で作り直さないと反映されません（`apps/phone` が叩くのは release の実行ファイル）。
- **形の決まり**: 型は `id` と ITEM_ORDER の節、ITEM_GUIDE は `key` と `parent` と `min_findings` の frontmatter が要ります。条文が崩れていれば 1 行の warning として `status` の `warnings` に入ります。
- **確かめ**:
  ```bash
  cd ~/workdir/Sai/sai-moon
  env -u SAI_PROJECT_ROOT -u SAI_PARENT_RUN_ID -u SAI_BLOCK_NAME \
    perl -e 'alarm 1800; exec @ARGV' cargo test -p sai-paper-core
  ```
  埋め込みの全ファイルが解析できること、各ブロックの op と範囲が読めること、を確かめます。
- **利用者のプロジェクトの `.sai/sci/` の写しは作り直しても変わりません。** 利用者が直した内容はそのまま残ります。
- 利用者が直した `.sai/sci/` の写しを優先して読むので、**配布の版を上げたいときは `.sai/sci/` の写しを消してから** `init` を打ち直します。

### 8.1 型・ガイドの節の見出し（新旧の対応）

節の見出しは英語名になりました。**旧い日本語の見出しも読めます**（新しい md には新しい見出しだけを書く）。節の中の機械が読む鍵（`- <役割>: …`・frontmatter・条件式）は変わりません。

| 旧い見出し | 新しい見出し | 置き場 |
|---|---|---|
| `## 項目の順序` | `## ITEM_ORDER` | 型 |
| `## 種の文` | `## SEED_SENTENCE` | 型 |
| `## 関門` | `## GATE` | 型 |
| `## 問い` | `## QUESTION` | ガイド |
| `## 埋まった条件` | `## CONDITION` | ガイド |
| `## 抜き出す項目` | `## EXTRACT_FIELDS` | ガイド |
| `## 落とし穴` | `## PITFALL` | ガイド |
| `## 次の一手` | `## NEXT_ACTION` | ガイド |
| `## 子の項目` | `## CHILD_ITEM` | ガイド |

GATE の名前も英語名になります（例: `出発点` → `BACKGROUND_GOAL_DECIDED`）。条件は変わりません。名前は表示にしか使われないので、旧い日本語名の写しもそのまま動きます。

mind.md の見出しも同じです（旧い見出しも読む）。

| 旧い見出し | 新しい見出し |
|---|---|
| `# 土壌（mind）` | `# SOIL（mind）` |
| `## 始まり` | `## ORIGIN` |
| `## 研究の型` | `## RESEARCH_TYPE` |
| `## 与えられていること` | `## GIVEN` |
| `## 制約` | `## CONSTRAINTS` |
| `## 価値の軸` | `## VALUE_AXES` |
| `## 興味がある / 興味が無い` | `## INTERESTS` |
| `## メモ` | `## NOTES` |

### 8.2 配布済みの写しの更新

配布物の写しは**自動では更新されません**。新しい版（英語名の見出し・名前）を使うには、古い写しを消してから `init` を打ち直します。

1. 古い写しを消す: `<root>/.sai/sci/`・`<root>/.sai/blocks/`・`<root>/.sai/template/`、利用者全体の `~/.sai-moon/`（`SAI_SAI_HOME` があればその中）の `blocks/`・`template/`。
2. 打ち直す: `sai-moon-paper sci init`（研究プロジェクト）または `sai-moon-paper init --assets-only`。無いファイルだけが書かれます。
   `sci-exp-seed` は `sci-exp-sprout` に改名した。古い `.sai/blocks/sci-exp-seed.md` は、消してから打ち直すと無くなる。
3. 手で直した写しは、消す前に控えておく（消すと直した内容も無くなります）。
4. 既存の木の題名を新しい名前へ移すには `sai-moon-paper sci retitle`（先に `--dry-run` で一覧を見る）。

この手順は文書に書くだけで、開発の作業では実行しません。

---

## 9. Canvas から移った人へ

- **Canvas の画面と `sai-moon-paper canvas` はありません。** 代わりに `#/papers/tree`（研究方針）と `sai-moon-paper sci` を使います。
- **今までの `.sai/paper/canvas/*.canvas` は消していません。** JSON Canvas 1.0 の形なので、他の道具（Obsidian など）で開けます。
- **`paper-map` は report だけを出すようになりました。** Canvas を作る手順は外れています。
- **古い paper-map の写し**（`<root>/.sai/blocks/paper-map.md`）には、無い命令（Canvas を作るなど）が書いてあることがあります。**その写しを消してから** `sai-moon-paper init --assets-only` を打つと、新しい物が入ります（無いファイルだけを書きます）。
