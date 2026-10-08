# sai-sci（research-tree）

**読み手**: 利用者と、`sai-sci/` の md を直す開発者。

初めて使う人は、手を動かしながら機能を回る [`sai-sci-tutorial.md`](sai-sci-tutorial.md) から始めてください。

`sai-sci` は、**研究の方向づけの段階（実験の前にある「何をやるか」を決める段階）を、問い（Q）・答え（A）・根拠（F）・実験（E）・SPROUT（S）の research-tree で扱う仕組み**です。
research-tree の正を 1 か所（`papr.db`）に置き、絵と文章（`state.md`・`tree.md`）はその正からの書き出しにします。

- research-tree は、論文の置き場があるプロジェクトごとに 1 つあります。
- Web（`apps/web`）・CLI・cc と pi の質問フックが、同じ research-tree を読み書きします。
- エージェントは `sai-sci op` で木を変えます。許可が要る op は、利用者が質問で「許可する」を選ぶまで入りません（5.4 節・5.10 節）。

## 1. sai-sci とは

- **research-tree の正は `papr.db`** です。Q・A・F・E・S・SEED の記録と承認の記録は `rt_` で始まる表の中にあります。
- **`state.md` と `tree.md` は書き出し**です。手で直しても次の書き出しで消えます。
- **research-tree の骨組みは RESEARCH_TYPE が決めます。** RESEARCH_TYPE（5 本）は、SEED の下に並ぶ top_Q から下へ伸びる Q の階層の雛形です。
- **エージェントは `sai-sci op` で木を変えます。** 危険度は 3 つに分かれます（5.4 節）。

## 用語（専門用語は英単語のまま使います）

| 用語 | 意味 |
|---|---|
| research-tree | SEED・ROOT・SPROUT・BRANCH の 4 layer の木と、それを描いた図の総称 |
| layer | 4 つの層（SEED・ROOT・SPROUT・BRANCH） |
| SEED | 研究の初期の入力（RESEARCH_TYPE と mind.md）。Node ID は `SEED`（DB の行は持たない） |
| ROOT | SEED から下へ分かれる Q・A・F の総称 |
| SPROUT | SEED から上へ分かれる node（ID は `S<n>`）。PATH を集めて実験の計画を立てる |
| BRANCH | SPROUT か E の下に分かれる E（実験の実行） |
| Node | research-tree の 1 つの要素（Q・A・F・E・S） |
| Q / A / F / E / S | 問い / 答えの候補 / 根拠 / 実験 / SPROUT |
| 種類の頭文字 | ID の頭文字が種類。形は `^[QAFES][1-9][0-9]{0,8}$`（例 `Q12`・`A15`） |
| stance | F の立場。`support`・`deny`・`issue` のどれか |
| top_Q | SEED の直下の Q（親が NULL） |
| parent_Q / child_Q | parent_Q は親にあたる Q、child_Q は子にあたる Q |
| bottom_Q | 生きた child_Q を持たない Q（JSON は `bottom_q`） |
| PATH | SEED から top_Q を通って A に至る道。終点の A で表す |
| ★ | 利用者が付ける目印。計算には使わない |
| 範囲（range） | SPROUT が数える Q・A と F の数（2.3 節） |
| OP | research-tree を変える操作 1 つ（`add_q` など）。`sai-sci op` と `sai-sci apply` で行う |
| 危険度 | OP の 3 つの区分。`Free`・`Approval`・`UserOnly`（5.4 節） |
| 承認 | 質問で「許可する」を選んでもらうこと。許可が要る op を 1 バッチ分通す |
| CARD / READ_CARD | agent への依頼 / 利用者が論文を読んで F を書く CARD |
| Next Action | 次にすべき CARD の候補 |
| REPORT | node agent・manage agent が終わりに保存する報告書 |
| GUIDE | Q の項目ガイド |
| 検証 agent | `sci-verify`。target_node と子の関係を評価する |
| LabAgent | sci_type を持つ sai-sci のブロックで動く agent |
| node agent / manage agent / support agent | sci_type の 3 分類 |
| context | 実行の直前に agent の最初のメッセージへ入る `# context` |
| 消す用語 | 旧模型の用語は使わない。旧い語と今の物の対応は 8.1 節 |

---

## 2. 木の考え

### 2.1 4 layer（SEED・ROOT・SPROUT・BRANCH）

研究の穴を Q で表し、答えの候補を A、根拠を F、確かめの実験を E で表します。
SEED は研究の初期の入力（RESEARCH_TYPE と `mind.md`）で、その下に top_Q が並びます（ROOT）。SPROUT（S）は PATH（A の ID）を 1〜20 個集めた node で、同じ A は 2 回書けません。E は S か E の下に付きます。

```
E（BRANCH）── S か E の下。結果は MLflow に結ぶ（E の下に F は置かない）
 │
S（SPROUT）── paths に A の ID を 1〜20 個。同じ A は 2 回書けない
 │
SEED ──────── RESEARCH_TYPE と mind.md。Node ID は SEED
 │
Q（top_Q）── SEED の直下（親が NULL）
 ├ Q（child_Q）── Q の下は child_Q だけ、または A だけ
 └ A（答えの候補）── Q の下。A の下に F と Q を置ける
     └ F（根拠）── A の下。stance は support・deny・issue のどれか
```

### 2.2 親子の決まり

`rt/ops.rs` の `execute` だけが守ります（DB は守りません）。

| 子 | 置ける親 |
|---|---|
| top_Q（親 NULL） | `SEED` |
| Q | 生きた A の子を持たない Q、または A |
| A | 生きた Q の子を持たない Q |
| F | A だけ |
| E | S か E |
| S | 親なし |

- Q の下は「A だけ」か「child_Q だけ」で、混ぜません。A の下は F と Q を混ぜてよい。
- 実験の結果は、E の `mlflow` で MLflow に結ぶだけです。E の下に F は置かず、実験の結論も木に足しません。

### 2.3 ★ と SPROUT の範囲

- **★** は利用者が付ける目印です。計算には使いません。
- **SPROUT の範囲**（`range`）は、`paths` の各 A について、その A・A の祖先の Q と A・A の子の生きた F を集めて数えます（重ならないように数える）。出すのは Q・A の数と、F の stance ごとの数です。A の子の Q 以下は入れません。

### 2.4 Action と Next Action

- **Action** は、Node 1 つに向けた 1 回の実行（CARD）です。task.md を持ち、目的を 1 行で書きます。
- **足す子の型**（`sci_child`。CLI は `sai-sci card --child <Q|A|F>`）は、対象の種類で決まります。Q → `Q` か `A`、A → `Q` か `F`。それ以外の組は終了コード 2 です。
- **Next Action** は、次にやるとよい Action の候補です。上の帯に先頭 3 件が並びます。`kind` は全部 `card` です。規則の順位（小さいほど先）は次のとおりです。

| 順 | why | 対象 | block | 目的 | CARD の追加 |
|---|---|---|---|---|---|
| 1 | `NO_CHILD` | 生きた子の無い Q | `lab-grow` | 「<Q の文>」の子（A か child_Q）を作る | なし |
| 2 | `NO_FINDING` | 生きた F の子の無い A | `lab-search` | 「<A の文>」の F を探す | `child: F` |
| 3 | `UNVERIFIED` | 生きた子が 1 つ以上ある Q・A で、`sci-verify` がその Node を対象にしたことが無い、または、その Node の直の子の一番新しい `created_at` が、その Node を対象にした一番新しい `sci-verify` の run の時刻より後 | `sci-verify` | 「<文>」と子の関係を確かめる | なし |

---

## 3. フォルダの中身

```
sai-sci/
├── types/      # RESEARCH_TYPE 5 本（problem・phenomenon・method・data・improve）
├── guides/     # ガイド 25 ファイル（Q の文・Child Q・Extract・Pitfall）
├── template/   # sci-core・sci-find・sci-match・bw-core・exp-core・exp-run・exp-ref・sim-core・data-core・stat-core・sai-agent・sci-task・sci-cli・debate-participant と、分野の観点 domain-general・domain-medical・domain-signal-ml
├── mind.md     # 研究の土台（mind.md）のひな形
├── tools/      # 実験ループの道具（台帳・分配器・参照コード・ModelDB・データ分析・EDA・生体信号）。`sai-sci init` が <root>/.sai/tools/ へ配る
├── exec/       # 実行先の見本: local・ssh・dok・_example。`sai-sci init` が <root>/.sai/exec/ へ配る
└── blocks/     # 役 4 本・lab-* 6 本・検証（sci-verify）・sci-* の作業・参加者（paper-debater・paper-reviewer）
```

### 3.1 RESEARCH_TYPE（5 本）

`types/` の 5 本。型は SEED の下の Q の階層の雛形です。本文の説明と `## Q` の節に階層（`-` が親子、`+` が並び）を持ちます。frontmatter は `id`・`name`。

置き場の順（先に見つかった物を使う）:

1. バイナリに埋め込んだ版
2. `<sai_home>/sci/types/`（利用者全体の置き場。`sai-sci type-save` で書く）
3. `<root>/.sai/sci/types/`（プロジェクトの写し）

| id | 名前 | どれを選ぶか |
|---|---|---|
| `problem` | PROBLEM_DRIVEN（課題駆動） | 現場の課題が先にある。決まっているのは大目的だけ |
| `phenomenon` | PHENOMENON_DRIVEN（現象駆動） | 必ず起きている現象が先にあり、「なぜ」を問う |
| `method` | METHOD_DRIVEN（手法駆動） | 手元の手法が先にあり、「何に使うか」を決める |
| `data` | DATA_DRIVEN（データ駆動） | 手元のデータが先にあり、「何が言えるか」を問う |
| `improve` | IMPROVE_DRIVEN（改良駆動） | すでに動いている手法をよくする |

一覧は `sai-sci types`。よく分からないときは `problem` から始めます。

### 3.2 ガイド（25 ファイル）

`guides/` の 25 ファイル。Q 1 つごとに、次の節を持ちます。

- frontmatter の `key`・`name`
- `## Q` — Q の文
- `## Child Q` — child_Q の案
- `## Extract` — F として抜き出す項目
- `## Pitfall` — つまずきやすい所

上の Q 9 本と child_Q 16 本があります。

| key | 名前 | 親 |
|---|---|---|
| `background` | BACKGROUND | ― |
| `background.goal` | BACKGROUND_GOAL | `background` |
| `background.problem` | BACKGROUND_PROBLEM | `background` |
| `target` | TARGET | ― |
| `target.subgroup` | TARGET_SUBGROUP | `target` |
| `data` | DATA | ― |
| `data.type` | DATA_TYPE | `data` |
| `data.size` | DATA_SIZE | `data` |
| `data.label` | DATA_LABEL | `data` |
| `data.access` | DATA_ACCESS | `data` |
| `purpose` | PURPOSE | ― |
| `purpose.subgoal` | PURPOSE_SUBGOAL | `purpose` |
| `purpose.output` | PURPOSE_OUTPUT | `purpose` |
| `purpose.metric` | PURPOSE_METRIC | `purpose` |
| `existing` | EXISTING | ― |
| `issue` | ISSUE | ― |
| `issue.limit` | ISSUE_LIMIT | `issue` |
| `issue.root` | ISSUE_ROOT | `issue` |
| `method` | METHOD | ― |
| `method.analog` | METHOD_ANALOG | `method` |
| `method.adapt` | METHOD_ADAPT | `method` |
| `novelty` | NOVELTY | ― |
| `novelty.closest` | NOVELTY_CLOSEST | `novelty` |
| `novelty.diff` | NOVELTY_DIFF | `novelty` |
| `phenomenon` | PHENOMENON | ― |

### 3.3 ブロック

`blocks/` の本は、`sai-sci init` がプロジェクトの `.sai/blocks/` に配ります。ブロックの frontmatter の `sci_type`（node・manage・support）と `sci_ops`（agent が出せる op の一覧）を持ちます。`sci_ops` を書いていない block は、`Free` の op 全部を出せます。

#### 3.3.1 役

研究の外から research-tree を見る役です。

```
professor / professor-backward（root の run。researcher を直接起こす）
  |-- researcher（sci・lab の block。research-tree の Node に向けて働く）
  `-- operator（professor の子。sai-tree の操作・監視・情報の抽出だけ）
```

| id | 役 | 共通の決まり | 研究の判断 |
|---|---|---|---|
| `professor` | 研究の現在地を読み、次の研究作業を researcher に任せ、報告を評価して次の CARD を作る | `sai-agent` / `sci-task` / `sci-cli` | する。root の run になる。研究作業は自分でしない |
| `professor-backward` | 既存の論文から始める逆向きの舵取り（5.8 節） | 上の 3 本と `bw-core` | する。root の run になる |
| `operator` | sai-tree の故障の対処・監視・情報の抽出・Web と同じ操作 | `sai-agent` / `sci-cli` | しない。professor の子 |
| `kaizen` | professor / professor-backward の実行履歴から問題を洗い出し、改善計画書を作る。コードも research-tree も直さない | `sai-agent` / `sci-task` / `sci-cli` | しない |

- CARD の `executer` は起動者（`human` / `agent`。古い値 `professor` は `agent` として読む）。`--executer agent` の CARD は人間が Web から起こせず、professor が `sai-link task create` の `card` で自分の子として起こす。
- 子からの質問は親の `sai-link task wait` に `stalled`（`reason: "question"`）で届き、`sai-web question` / `answer` で答える。
- 完了は、子は `sai-link task report`、root（professor）は質問ツールで「完了してよいか」を確かめて `sai-link run finalize` に `report`。
- 研究日誌は `.sai/paper/professor-log.md`。operator の学びは `~/.sai/operator-notes.md`（`SAI_HOME` があればその中）。

#### 3.3.2 LabAgent（研究の進め方の 6 場面）と検証

研究の場面ごとに、次の 6 本の LabAgent と検証 agent があります。

| # | 場面 | ブロック | sci_type | sci_ops |
|---|---|---|---|---|
| #1 | 型指定 ＋ 論文あり | `lab-read` | node | `add_a, add_f, add_q` |
| #2 | 型指定 ＋ 検索（段 1 で検索し候補を報告 → 利用者が取り込む → 段 2 は `lab-read` の CARD を作る） | `lab-search` | node | `add_a, add_f, add_q` |
| #3 | 型未指定 ＋ 論文あり | `lab-grow` | node | `add_a, add_f, add_q, insert_q` |
| #4 | 型未指定 ＋ 検索 | `lab-survey` | node | `add_a, add_f, add_q` |
| #5 | 論文だけ（木を探して置き場所を決め、`lab-read` の CARD を作る） | `lab-place` | manage | `add_a, add_f, add_q` |
| #6 | 全体の進み具合の確認（数と穴を報告し、次の CARD を作る） | `lab-progress` | manage | なし（op を出さない） |
| 検証 | target_node と子の関係を評価し、評価の報告・op・次の CARD を作る（固定） | `sci-verify` | node | `set_stance, add_f, remove, move_a, insert_q, add_q` |

- READ_CARD（`runner: user`）の既定の block は `lab-read` です。
- `sci-verify` の参加者は `paper-debater`・`paper-reviewer` を子として使えます。

#### 3.3.3 作業のブロック

| Action | ブロック | sci_type |
|---|---|---|
| 実験 | `sci-exp-sprout`（manage）・`sci-exp-branch`（support）・`sci-exp-review`（node）・`sci-result-compare`（manage・逆向き） | 上の欄 |
| 実験の support | `sci-exp-scout`（参照実装を探す）・`sci-sim-model`（論文 1 本のモデル定義書）・`sci-sim-compose`（つなぎのモデル定義書） | support |
| データ分析 | `sci-data-explore`（node）・`sci-data-test`（node） | node |
| その他 | `sci-start`・`sci-restore`（逆向き）・`sci-typify`（manage）・`sci-digest`（manage・op を出さない）・`sci-coach`（node）・`sci-cite-check`（node・逆向き） | 上の欄 |
| support（呼ばれる側） | `sci-analogy`・`sci-extract`・`sci-lookup`・`paper-debater`・`paper-reviewer` | support |

- support agent は、ほかのブロックが呼ぶ agent。単独の Action としては動かしません。
- `sci-exp-branch` だけは、research-tree の範囲も共通の決まり（`sci-core`）も受け取りません（実験ループ。5.7 節）。
- 逆向きのブロック（`sci-restore`・`sci-cite-check`・`sci-result-compare` と `professor-backward`）は 5.8 節のためのもの。
- 旧 block（`sci-read`・`sci-search`・`sci-ideate`・`sci-split`・`sci-review`・`sci-debate`・`sci-compare`）は消えました。`sci-*` の旧版は使いません（8.2 節）。

### 3.4 template と mind.md

- **`template/sci-core.md`** — エージェントに共通で渡す決まり。context の読み方、`sai-sci op` の書き方、使える op と危険度、根拠の扱い、迎合しない姿勢。
- **`template/sci-find.md`** — F を探すブロックの共通の決まり。
- **`template/sci-match.md`** — 論文と Q・A を照らし合わせるときの共通の決まり。
- **`template/bw-core.md`** — 逆向きの運用の共通の決まり（5.8 節）。
- **`template/exp-core.md`** — 実験ループの共通の決まり。役割、文書の渡し方、実験仕様書、MLflow の記録の決まり、台帳、判定の規則、実行場所の規則、停止の条件。
- **`template/exp-run.md`** — 実験のスクリプトを「実行先」に投げる手順（分配器 `exp_exec.py` の呼び方）。
- **`template/exp-ref.md`** — 参照コードと出典の決まり。置き場は `<sai_home>/refcode/`。
- **`template/sim-core.md`** — 神経シミュレーションの共通の決まり。
- **`template/data-core.md`** — データ分析の共通の決まり（データの安全・データ表・EDA・前処理・MLflow・作業フォルダ・再現性）。
- **`template/stat-core.md`** — 統計解析の決まり（解析計画・検定の選び方・前提・多重比較・効果量・報告）。
- **`template/sai-agent.md`** — 研究プロジェクトのエージェントに共通で渡す決まり。文書だけでの引き継ぎ・waggle トークン・`sai-link doc write` / `sai-link doc read`・完了のしかた。
- **`template/sci-task.md`** — 子タスクの扱い（professor が使う）。
- **`template/sci-cli.md`** — 研究プロジェクトでの命令（professor と operator が使う）。
- **`tools/refcode.py`** — 参照コードの道具（`links`・`gh-search`・`fetch`・`params`・`check`）。
- **`tools/modeldb.py`** — ModelDB のカタログ（`sync`・`search`・`show`・`status`）。
- **`tools/data_result.py`** — データ分析の検査と記録。`results.json` を読み書きし、REPORT の骨を作る。
- **`tools/eda/`**・**`tools/biosignal/`** — K-Dense からの写し（MIT）。
- **`mind.md`**（`sai-sci/mind.md`）— 研究の土台のひな形。`ORIGIN・RESEARCH_TYPE・GIVEN・CONSTRAINTS・VALUE_AXES・INTERESTS・NOTES` の 7 節。**利用者が書くファイルで、エージェントは直接書き換えません。** `GIVEN` は RESEARCH_TYPE で最初から与えられている物（SEED に入る物）、`VALUE_AXES` は A を比べるときの軸になる。

---

## 4. 始め方

### 4.1 新しいプロジェクト

Web の Research タブの research-tree で「始める」を押して型を選ぶか、次を打ちます。

```bash
sai-sci init --type <型>
```

`init` は、`sai-sci/mind.md` のひな形を `mind.md` が無いときだけ書きます。

### 4.2 今のプロジェクト

```bash
sai-sci init --type <型>
```

型の一覧は `sai-sci types`。

古い形の research-tree（旧い版の `sci_*` の表だけがある research-tree）のプロジェクトでは、画面と `sai-sci status` が「作り直し」を促します。作り直しは新しい `sai-sci init` です。古い表は消さず、新しい research-tree はそれを読みません。

### 4.3 入る物

| 入るもの | 置き場 |
|---|---|
| ブロック | `<root>/.sai/blocks/` |
| template（と各ブロックが使う `search-internal` などのテンプレ、`domain`） | `<root>/.sai/template/` |
| 台帳 `exp_summary.py` と分配器 `exp_exec.py`・`refcode.py`・`modeldb.py`・`data_result.py`・写しの `eda/`・`biosignal/`（無いときだけ書く） | `<root>/.sai/tools/` |
| 実行先の見本 `local`・`_example` など（無いときだけ書く） | `<root>/.sai/exec/` |
| `.sai/settings.json` の `context`（実行直前に範囲を作る命令。無いときだけ書く） | `<root>/.sai/settings.json` |
| `mind.md`（無いときだけ書く） | `<root>/.sai/paper/mind.md` |
| 型とガイドの写し | `<root>/.sai/sci/types/` と `<root>/.sai/sci/guides/` |
| 取得した参照コードと ModelDB のカタログ | `<sai_home>/refcode/`（`SAI_HOME` が無ければ `~/.sai`） |
| research-tree | `papr.db` の `rt_*` の表 |
| 書き出し（`state.md` と `tree.md`） | `<root>/.sai/paper/` |

**手で足す設定はありません。** `init` だけで sci の設定が終わります。cc と pi の質問フックの登録は 5.10 節の手順で行います（`sai-link` が cc の登録を書くこともあります）。

---

## 5. 使い方の流れ

### 5.1 木を育てる流れ

```
Q を分ける ─▶ A を書く ─▶ F を集める ─▶ 検証（sci-verify）─▶ E（実験）
 （lab-grow）  （lab-grow）   （lab-search・lab-read）
```

画面（`#/research/tree`）での操作:

1. **Q を分ける** — Next Action の「Create CARD」から `lab-grow` を動かす。または板の「Q を足す」で自分で足す。
2. **A を書く** — Q の板の「A を足す」、または `lab-grow` の CARD。
3. **F を付ける** — Library で論文を探し、READ_CARD（`#/research/read/<run_id>`）で 1 本ずつ読んで F を書く。エージェントが出した F は、`add_f` として自動で research-tree に入る。
4. **検証** — 子を持つ Q・A の Next Action に `sci-verify` が出る。評価の報告と op・次の CARD が作られる。
5. **E（実験）** — 実験は 5.7 節。実験の結論は木に足さない（E の `mlflow` で MLflow に結ぶだけ）。

### 5.2 Web の research-tree の画面

Research タブの最初の画面が research-tree（`#/research/tree`）。

- 上の帯: **Next Action**（先頭 3 件。`[NO_CHILD] Q24 lab-grow「…」` の形で、右に Create CARD）・折りたたみの **History**（全 Node の履歴を古い順に。`sai-sci status` の `history[]`）・注意。
- 左: Q・A・F・E・S（SPROUT）の一覧。
- 真ん中: **DAG**（research-tree の図。下に ROOT（top_Q）、次に SEED の欄、上に SPROUT、さらに上に BRANCH（E））。SEED の欄を押すと RESEARCH_TYPE と `mind.md` の節が出る。Node を押すと `#/research/tree/Q12` のように住所が変わり、詳細が右の **SB**（サイドバー）に出る。
- SB: 見出し・親までの道・操作の釦・Next Action・F・History など。何も選んでいないときは出ない。PC 幅（900px 以上）は右（約 420px・✕ で閉じる）、iPhone 幅は下から出る sheet。
- **Create SPROUT**: SEED の SB か上の帯の釦を押すと、History と DAG の間に作成パネルが出る。A の SB の **Add PATH** / **Delete PATH** で PATH（終点の A）を選ぶ。PATH は 1〜20 個で、同じ A は 2 回選べない。
- READ_CARD: `#/research/read/<run_id>`。

### 5.3 sai-link がブロックを動かすとき

`sci-*` と `lab-*` のブロックは `context: true` です。sai-link は実行の直前に `.sai/settings.json` の `context` の命令を実行し、標準出力を、最初のメッセージの `# context` の節に入れます。依頼文は同じメッセージの `# task_text` の節に入ります。

TREE の範囲はブロックの sci_type で決まります。`node` は対象の Node とその子孫、`manage` は SEED から対象までの祖先と対象とその子孫（対象が `SEED` なら research-tree の全体）、`support` には出しません。範囲を作れなかったときは 1 行の注意が `# context` の下に入ります（実行は止まりません）。

task.md の frontmatter `sci_child` があれば、`# context` の対象の節に `- 足す子の型: A` の行を出します（空なら行を出さない）。

**決まり:** `.sai/settings.json` の `context` に別の命令を指定するときは、その出力も `##` から始めてください。

### 5.4 agent は op を出す（危険度と承認）

`sai-sci op` の標準入力は `{"ops": [ {"op": "add_a", "q": "Q3", "text": "…"}, … ]}`（1〜20 件）です。全部を 1 つのトランザクションで行い、1 件でも弾かれたら何も残しません。

op の危険度は 3 つに分かれます。

- **`Free`**（agent がそのまま出せる）: `add_a`・`add_f`・`add_e`・`add_sprout`・`set_stance`・`set_mlflow`
- **`Approval`**（agent は承認が要る）: `add_q`・`insert_q`・`move_a`・`copy`・`remove`
- **`UserOnly`**（利用者だけ）: `star`・`unstar`・`edit`

| op | 危険度 | 規則 |
|---|---|---|
| `add_q` | Approval | 親は C1 の表のとおり。`guide` は GUIDE に無ければ拒否 |
| `insert_q` | Approval | `q` に生きた A が 1 つ以上要る。新しい Q を `q` の子にし、`q` の生きた A を全部その下へ移す（`ord` の順を保つ） |
| `move_a` | Approval | `from` ≠ `to`。`from` に生きた A が 1 つ以上要る。`to` は生きた child_Q を持たない。`to` が `from` の A の子孫なら拒否（輪になる） |
| `copy` | Approval | `to` は生きた child_Q を持たない。A と、その生きた F（stance ごと）を写す。A の下の Q と E は写さない |
| `remove` | Approval | Node を下の Node ごと消す |
| `add_a` | Free | `q` は生きた child_Q を持たない。同じ Q の生きた A と同じ文なら拒否。作ったあと GUIDE の `+` の子 Q を作る |
| `add_f` | Free | 3 文まで。数字を含む文は `cites` か `report` が要る。`cites` の形と存在は確かめる |
| `add_e` | Free | 親は S か E |
| `add_sprout` | Free | 同じ A を 2 回書けない |
| `set_stance` | Free | 値が同じなら何もしない |
| `set_mlflow` | Free | E の `mlflow` を変える（空文字は外す） |
| `star` / `unstar` | UserOnly | 値が変わるときだけ書く |
| `edit` | UserOnly | 字数の上限は Q 300・A/E/S 120・F 1200。F は 3 文まで。`body` は A・E・S だけ |

- **block の `sci_ops`** に書いた op だけを agent が出せます。書いていない block は `Free` の op 全部です。
- **利用者の `sai-sci apply`** は、どの op も承認なしで行えます。rt_log の `actor` は `user` になり、`approved_by`・`qid`・`question` は空です。
- 承認が要る op を出すとき、agent は次の流れで許可を取ります（5.10 節）。質問で「許可する」→ 回答が `sai-sci approval record` で記録される → `sai-sci op` が許可を 1 件取って全部を書き、その許可を消費する。許可が無いと終了コード 4 で、何も書かれません。

#### LabAgent の 3 分類

| sci_type | 動き方 | TREE の範囲 | 終わり方 |
|---|---|---|---|
| node | 対象の Node を指定されて動く | 対象＋子孫 | REPORT を `sai-link doc write` で 1 つ保存する |
| manage | research-tree の全体を見て動く | 祖先＋対象＋子孫（対象が `SEED` なら全体） | REPORT を `sai-link doc write` で 1 つ保存する |
| support | node agent・manage agent に呼ばれ、1 つの道具の使い方に特化した作業をする | TREE なし | REPORT を保存しない。呼んだ agent に `sai-link task report` で返す |

正常に終わったかどうかは、REPORT があるかで見られます（support agent は REPORT を持ちません）。

### 5.5 READ_CARD

`sai-sci card --runner user --papers <番号> --depth <abstract|figures|full>` は READ_CARD を作ります。`apps/web` の READ_CARD の画面で始めて、F を書き、終えます。F が 1 つも無いまま終えても止まりません。

### 5.6 research-tree の直し方

research-tree と `tree.md` は、Web か `sai-sci` でだけ直します。`state.md` を手で直さないでください（次の書き出しで消えます）。

### 5.7 実験（E）

`sci-exp-sprout`・`sci-exp-branch`・`sci-exp-review` の 3 本で実験を回します。3 本は互いの終わりを待ちません。sprout は最初の branch を起こして終わり、branch はジョブ待ちの review の CARD を作って終わり、review はジョブが終わると daemon に起こされます。

```
sai-sci card --node SEED --block sci-exp-sprout   SPROUT を作る（対象は SEED）
        │
        ▼
 sprout ── PATH を選んで SPROUT を作り、実験仕様書と最初の branch を
        │  1〜2 本起こして終わる
        │  （sai-sci card --block sci-exp-branch → sai-link card run）
        ▼
 branch ── スクリプト 1 本を書いて実行先に投げる。始まりを確かめたら、
        │  ジョブ待ちの review の CARD を作って終わる（sai-link card wait --job）
        ▼  …ジョブが終わると daemon が起こす
 review ── 台帳で判定する。tune・debug → 同じ experiment の次の branch、
        │  model・ablation → 新しい experiment の次の branch、stop → 完了報告書
        └─▶ 次の branch …   （stop で終わる。起動元が professor なら起こし直す）
```

- **MLflow の形**: experiment は `<project>/ID<sprout>/<model>`、run は `ID<branch>-p<NN>`。epoch ごとに `train_loss`・`val_loss` を記録する。
- **木への書き込み**: 実験の結果は E の `mlflow` で MLflow に結ぶだけです（`set_mlflow`）。E の下に F は置かず、結論も木に足しません。
- **止め方**: 判定が `stop` のときだけ。`max_branches` が branch の総数の上限になる。
- **最初の sprout の起動**は、`sai-sci card --node SEED` で作る。`--input` は依頼文の末尾に載せる文書の waggle トークン（繰り返し可）。

```bash
sai-sci card --node SEED --block sci-exp-sprout \
  --purpose "<解きたい Q の A を含む SPROUT を作る>" \
  --input <参照コード調査書や復元書のトークン> --json
sai-link card run <返ってきた run_id>
```

- 詳しい決まりは `template/exp-core.md`。

**実行先**（スクリプトを動かす PC）は環境変数 `EXP_TARGET` で選ぶ。空なら `local`（この PC）。実行先は `<root>/.sai/exec/<名前>/` のフォルダで、利用者が個別に足せる（フォルダを置くだけ。増減で Rust や画面を直さない）。さくらの高火力 DOK は `dok/`（`README.md`・`SETUP.md` を先に読む。課金される）。記録は実行先から中央の MLflow に直接入る。

### 5.8 逆向き（既存の論文から始める）

前向き（5.1〜5.7）は研究の構想から research-tree を育てます。**逆向き**は、既存の論文（基盤論文 1〜5 本）を出発点にして、論文に書いてある研究方針を research-tree に復元し、その根拠を確かめ、論文の手法を再現し、新しい論文で直し、比べます。決まりは `template/bw-core.md` にまとめてあります。

| 段 | すること | ブロック |
|---|---|---|
| 1. 抽出 | 序論を上の Q に、手法と実験の設定を下の Q に割り当てる。結果の数値を記載結果表に、主張と引用の組を対応表にする | `sci-restore` |
| 2. 評価 | 主張ごとに引用文献を取り寄せて、根拠になっているかを判定する。そのうえで、復元した research-tree を検証する | `sci-cite-check` → `sci-verify` |
| 3. 再現 | PATH を選んで再現の SPROUT を作り、論文の手法を動かす | `sci-exp-sprout` |
| 4. 更新 | 基盤論文より新しい論文を探して A を直し、更新の SPROUT を作って動かす | 前向きのブロック |
| 5. 比較 | 論文の記載値・再現・更新を比べる | `sci-result-compare` |

- **始め方**: 基盤論文を Library に入れ（Web の論文の画面）、research-tree を作ってから（`sai-sci init --update --type <型>`）、`professor-backward` の CARD を人が起こします。依頼文に `基盤論文: #12 #15` の形で番号を書きます。
- **主張と根拠を分けます。** 基盤論文の文は「主張」として F にし、A は Q の下に置きます。
- **再現は論文に合わせにいきません。** 論文に無い設定だけを仮置きとして置き、論文に書いてある値と手法は変えません。
- 研究日誌は `.sai/paper/professor-backward-log.md` です。

### 5.9 データ分析（手元のデータ）

実験（5.7 節）がスクリプトを回すのに対し、データ分析は**手元にあるデータ**を読みます。2 本のブロックで道筋を分けます。

```
sai-sci card --node <N> --block sci-data-explore
        │
        ▼
 explore ── データを受け取り、データ表・EDA・前処理（欠損・重複・外れ値・分布）を
        │  作り、変換の台帳に「何をしたか」を残す。検定はしない。
        │  REPORT の「## Next Action」に sci-data-test の CARD を作る命令を書く
        ▼
 professor ── その REPORT を評価し、命令どおりに test の CARD を作って起こす
        │
        ▼
 test ── 解析計画書を先に保存して固定し、そのあとは計画どおりに検定する。
        │  効果量と検出力・前提・多重比較を確かめ、図と results.json を作り、
        │  research-tree に E と F を足す
```

- **最初の explore の CARD の作り方**:

```bash
sai-sci card --node <N> --block sci-data-explore \
  --purpose "<データのパスと調べたいこと>" --json
sai-link card run <返ってきた run_id>
```

- **test の CARD は explore の REPORT から作ります。** REPORT の `## Next Action` に書かれた命令をそのまま使います。professor はその REPORT を評価してから起こします。
- **置き場**: 作業は `analysis/<study>/` に置きます。元のデータは書き換えません。
- **解析はこの PC で動かします。** 30 分を超えるもの・データの量が大きいものは実験の流れ（5.7 節）へ回します。
- 詳しい決まりは `template/data-core.md` と `template/stat-core.md`。

### 5.10 質問と承認

許可が要る op（5.4 節）は、利用者の「許可する」が 1 件そろってから入ります。質問と承認の流れは次のとおりです。

1. agent は、承認を取る質問を 1 回 1 問で出します。先頭の選択肢は `許可する`、2 つ目は `許可しない` です。質問文には、これから行う op の一覧（op 名と対象の ID・文）を書きます（template の決まり）。
2. 利用者が答えます。
3. 回答を受け取った質問フックが、環境変数 `SAI_PARENT_RUN_ID` のある run でだけ、次を 10 秒の上限で実行します（失敗しても無視し、答えは今までどおり agent に返します）。

   ```
   <SAI_SCI_BIN か sai-sci> approval record --qid <qid> --question <質問文> --answer <回答> --answered-by <answered_by>
   ```

   - 質問が複数あるときは、質問文・回答をそれぞれ改行でつないだ 1 本の文字列にします。
   - pi で端末から答えたときは `--answered-by terminal`。
4. `sai-sci approval record` は `rt_approvals` に 1 行足します。同じ `qid` が既にあれば終了コード 4。
5. agent が `sai-sci op` を出すと、許可を 1 件取って全部の op を書き、その許可を消費します。許可が要る op の rt_log の行に `approved_by`・`qid`・`question` が入ります。
- agent は `sai-sci approval record` を自分で呼びません。

**登録の手順**（登録は手で行います。この文書を読んで打つだけで、`sai-link` は cc の登録を自動で書くこともあります）。

- **cc**: プロジェクトの `.claude/settings.json` の `hooks.PreToolUse` に、`AskUserQuestion` の matcher で `cc_question.py` を置きます。`sai-link` が init で書く形と同じです。手で書くときの例:

  ```json
  {
    "hooks": {
      "PreToolUse": [
        {
          "matcher": "AskUserQuestion",
          "hooks": [
            {
              "type": "command",
              "command": "'/絶対パス/sai-tree/apps/web/hooks/cc_question.py'",
              "timeout": 86400
            }
          ]
        }
      ]
    }
  }
  ```

  `timeout` は 86400（24 時間）。フックが Web の答えを待つので、cc の既定の待ち時間で切られないようにします。

- **pi**: プロジェクトの `.pi/settings.json` の `extensions` に、`ask_user.ts` の絶対パスを入れます。pi は global と project の `extensions` を両方読むので、global の拡張は消えません。

  ```json
  {
    "extensions": ["/絶対パス/sai-tree/apps/web/src/sai_web/extensions/ask_user.ts"]
  }
  ```

- **命令の場所**: 承認の記録の命令は、環境変数 `SAI_SCI_BIN` で場所を変えられます（無ければ `sai-sci`）。
- **確かめ方**: 一時プロジェクトで（5.4 節の op が必要な）質問に答えたあと、`sai-sci op` の `add_q` が通ること。通らないときは、`sai-sci status --json` の `warnings` と、`rt_approvals` の行を見ます。

---

## 6. 命令の一覧

### 6.1 `sai-sci …`

| 命令 | 何をするか |
|---|---|
| `sai-sci init --type <型> [--json]` | 研究プロジェクトの配る物を配り、SEED の下に top_Q を作る |
| `sai-sci types [--json]` | 使える RESEARCH_TYPE を一覧する |
| `sai-sci type-save` | RESEARCH_TYPE を `<sai_home>/sci/types/` に保存する（標準入力に型の Markdown） |
| `sai-sci status [--json] [--brief]` | 現況（counts・SEED・research-tree・Next Action）。`--brief` は `{"type","counts","next","running","warnings"}` を JSON で出す |
| `sai-sci node <Q12\|A5\|F3\|E2\|S2> [--json]` | Node 1 つの詳細 |
| `sai-sci context --run <run_id> [--tree]` | 今の実行の `# context` を標準出力へ出す |
| `sai-sci op [--run <run_id>] [--block <id>]` | agent が op を出す（標準入力は `{"ops":[…]}`）。許可が要る op は承認を使う（5.4・5.10 節） |
| `sai-sci approval record --qid <qid> --question <文> --answer <文> --answered-by <名前> [--run <id>] [--json]` | 質問の回答を承認として記録する。質問フックが呼ぶ（agent は呼ばない） |
| `sai-sci apply` | 利用者の操作を 1 つ反映する（標準入力に 1 件の JSON。承認なし） |
| `sai-sci card --node <Q12> --purpose <文> [--block <id>] [--child <Q\|A\|F>] [--expect <文>] [--mode <purpose_first\|blind_first>] [--papers <番号,…>] [--runner pi\|cc\|user] [--depth abstract\|figures\|full] [--origin-run <run_id>] [--input <トークン>]… [--executer human\|agent] [--json]` | Node に向けた Action（CARD）を作る。`--runner user` は READ_CARD。`--child` は足す子の型 |
| `sai-sci read <run_id> [--json]` | READ_CARD の中身 |
| `sai-sci export` | research-tree を `state.md` と `tree.md` に書き出す |
| `sai-sci schema [--json]` | 命令の一覧（`stdin`・`json`・`web` の印つき） |

書き込む命令（`init`・`apply`・`op`・`card`）は、終わる前に `export` を実行します。Web の窓口は `sai-sci {status,init,node,apply,types,card,read}` を呼びます。`op` と `approval record` は Web の「コマンド」画面に出しません（agent と質問フックだけが呼ぶ）。

論文の節と塊を読む命令（`sai-paper content …`）:

| 命令 | 何をするか |
|---|---|
| `content outline <番号> [--json]` | 論文の節の一覧（節の ID `#37.5`・文の数・要約） |
| `content read <ID>… [--max-chars N] [--json]` | 節・塊を読む（`37`・`37.5`・`37.5.2`・`37.5.2-4`） |
| `content build [<番号>…] [--all] [--force] [--summarize]` | 節と塊の表を作る（作り直す・要約を付ける） |

---

## 7. 置き場

| 物 | 置き場 |
|---|---|
| research-tree（Q・A・F・E・S・SEED・検索の記録・承認の記録・rt_log） | `papr.db`（`rt_` で始まる表: `rt_meta`・`rt_nodes`・`rt_log`・`rt_approvals`・`rt_seeds`・`rt_searches`） |
| 書き出し（`state.md` と `tree.md`） | `<root>/.sai/paper/state.md` と `<root>/.sai/paper/tree.md` |
| RESEARCH_TYPE（利用者全体） | `<sai_home>/sci/types/`（`SAI_HOME` があればその中） |
| RESEARCH_TYPE とガイドの写し（プロジェクト） | `<root>/.sai/sci/types/` と `<root>/.sai/sci/guides/` |
| professor の研究日誌 | `<root>/.sai/paper/professor-log.md` |
| professor-backward の研究日誌 | `<root>/.sai/paper/professor-backward-log.md` |
| operator の学び | `~/.sai/operator-notes.md`（`SAI_HOME` があればその中。無ければ作らない） |

- **`state.md` を手で直さない。** 次の書き出しで消えます。直すのは Web か `sai-sci` だけです。
- **写しは埋め込みより先に読まれます。** 型は「埋め込み → `<sai_home>/sci/types/` → `<root>/.sai/sci/types/`」の順に探し、後ろの物が先の物を上書きします。

---

## 8. md を直すとき（開発者向け）

- **型・ガイド・ブロックは `sai-sci` に埋め込まれています**（`include_str!`）。`sai-sci/` の md を直したら `cargo build --release` で作り直さないと反映されません（`apps/web` が叩くのは release の実行ファイル）。
- **形の決まり**: 型は `id`・`name` の frontmatter と `## Q` の節、ガイドは `key`・`name` の frontmatter と `## Q`・`## Child Q`・`## Extract`・`## Pitfall` の節が要ります。崩れていれば 1 行の warning として `status` の `warnings` に入ります。
- **block の形**: frontmatter の `sci_type`（node・manage・support）と `sci_ops`（op の一覧）。`sci_ops` を書かない block は `Free` の op 全部を出せます。`lab-progress` は op を出さない（3.3.2 節）。ただし `sci_ops` を書かない block は `Free` の op 全部を出せるため、「op を出さない」を frontmatter でどう書くかは要確認。
- **確かめ**:
  ```bash
  cd ~/workdir/Sai/sai-tree
  env -u SAI_PROJECT_ROOT -u SAI_PARENT_RUN_ID -u SAI_BLOCK_NAME \
    perl -e 'alarm 1800; exec @ARGV' cargo test -p sai-sci-cli
  ```
- 利用者が直した `.sai/sci/` の写しを優先して読むので、配布の版を上げたいときは 8.2 節の手順を使います。

### 8.1 古い区分と新しい物の対応

古い版（旧 research-tree）の言葉と、今の物の対応です。**この表の中だけ古い語を書きます。**

| 古い区分 | 新しい物 |
|---|---|
| ITEM | Q |
| OPTION（旧 O） | A |
| FINDING | F |
| sprout / branch（旧 4 層の下の 2 層） | E の親子 |
| GATE・BLANK・MARK・TENTATIVE | 廃止（評価の軸は持たない） |
| LINK | F の出典と、Action の記録 |
| READ_CARD | READ_CARD（そのまま） |
| TOMBSTONE | なし（論文を外す記録は持たない） |
| ROOT_GRAPH / RUN_GRAPH | research-tree（下に伸びる図 1 つ）。実行の図は research-tree の画面から無くした |
| SOIL（mind） | mind.md（研究の土台）。見出し（`## ORIGIN` など）は変わらない |

型・ガイドの節の見出しは、新しい md に新しい見出し（`## Q`・`## Child Q`・`## Extract`・`## Pitfall`）だけを書きます。

### 8.2 配布済みの写しの更新

研究プロジェクトの `<root>/.sai/blocks/`・`<root>/.sai/template/`・`<root>/.sai/sci/` は、配ったときの写しです。**自動では新しくなりません**（手で直した写しを壊さないため）。新しい版にするには、利用者が次を打ちます。

1. 写しを手で直していないか確かめる（`git -C <root> status` など）。直していれば控える。
2. 古い写しを別の名前に移す。

   ```bash
   mv <root>/.sai/blocks <root>/.sai/blocks.old
   mv <root>/.sai/template <root>/.sai/template.old
   mv <root>/.sai/sci <root>/.sai/sci.old
   ```

3. 新しい写しを配る。

   ```bash
   sai-sci init --update --type <型>
   ```

4. 控えた手の直しを新しい写しに移し、古い写し（`*.old`）を消す。

- `sai-sci init --update` は、プロジェクトの `.sai/blocks/` に旧 block（`sci-read`・`sci-search`・`sci-ideate`・`sci-split`・`sci-review`・`sci-debate`・`sci-compare` の 7 つ）があれば退避してから配ります。退避の置き場は既存の退避の仕組みと同じです。
- 古い形の research-tree（旧 `sci_*` の表）は消えません。新しい research-tree は `sai-sci init` で作り直します（4.2 節）。
- 利用者全体の置き場（`~/.sai/` の `blocks/`・`template/`・`sci/types/`）に同じ名前の物があるときも、同じように移してから配ります。
- この手順は文書に書くだけで、開発の作業では実行しません。

---

## 9. Canvas から移った人へ

- **Canvas の画面と `sai-paper canvas` はありません。** 代わりに `#/research/tree`（research-tree）と `sai-sci` を使います。
- **今までの `.sai/paper/canvas/*.canvas` は消していません。** JSON Canvas 1.0 の形なので、他の道具（Obsidian など）で開けます。
- **`paper-map` は report だけを出すようになりました。** Canvas を作る手順は外れています。
- **古い paper-map の写し**（`<root>/.sai/blocks/paper-map.md`）には、無い命令が書いてあることがあります。8.2 節の手順で新しい写しにしてください。
