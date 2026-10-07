# sai-sci（research-tree）

**読み手**: 利用者と、`sai-sci/` の md を直す開発者。

初めて使う人は、手を動かしながら全部の機能を回る [`sai-sci-tutorial.md`](sai-sci-tutorial.md) から始めてください。

`sai-sci` は、**研究の方向づけの段階（実験の前にある「何をやるか」を決める段階）を、Q・O・F・E の research-tree で扱う仕組み**です。
research-tree の正を 1 か所（`papr.db`）に置き、絵と文章（`state.md`・`tree.md`）はその正からの書き出しにします。

- research-tree は、論文の置き場があるプロジェクトごとに 1 つあります。
- Web（`apps/web`）・CLI・pi の拡張が、同じ research-tree を読み書きします。
- エージェントは research-tree に PROPOSAL を出します。自動で入る物と、利用者が TRIAGE する物があります。

## 1. sai-sci とは

- **research-tree の正は `papr.db`** です。Q・O・F・E・PROPOSAL・SEED などは `rt_` で始まる表の中にあります。
- **`state.md` と `tree.md` は書き出し**です。手で直しても次の書き出しで消えます。
- **research-tree の骨組みは RESEARCH_TYPE が決めます。** RESEARCH_TYPE（5 本）は、SEED の下に並ぶ top_Q から下へ伸びる Q の階層の雛形です。
- **エージェントは PROPOSAL を出すだけです。** research-tree に直接書き込む処理は入れません。

## 用語（専門用語は英単語のまま使います）

| 用語 | 意味 |
|---|---|
| research-tree | SEED・ROOT・SPROUT・BRANCH の 4 layer の木と、それを描いた図の総称 |
| layer | 4 つの層（SEED・ROOT・SPROUT・BRANCH） |
| SEED | 研究の初期の入力（RESEARCH_TYPE と mind.md）。Node ID は `SEED` |
| ROOT | SEED から下へ分かれる Q・O・F の総称 |
| SPROUT | SEED から上へ分かれる node（ID は `S<n>`）。PATH を集めて実験の計画を立てる |
| BRANCH | SPROUT から上へ分かれる E（実験の実行） |
| Node | research-tree の 1 つの要素（Q・O・F・E・S） |
| Q / O / F / E / S | 問い / 答えの候補 / 根拠 / 実験 / SPROUT |
| Answer / Issue | O の 2 種（答え / 未解決の課題） |
| STAR（★） | 利用者が Answer の O に付ける印 |
| STAR_PATH | top_Q から ★ の O を通ってたどれる Node の道（JSON は `on_path`） |
| top_Q | SEED の直下の Q |
| parent_Q / child_Q | parent_Q は親にあたる Q、child_Q は子にあたる Q |
| bottom_Q | child_Q を持たない Q（JSON は `bottom_q`） |
| PATH | SEED から top_Q を通って O に至る道。終点の O で表す |
| PROPOSAL | agent が出す research-tree の変更の案（ID は `P<n>`） |
| OP | research-tree を変える操作 1 つ（`add_q` など） |
| AUTO_OP / TRIAGE_OP / USER_OP | すぐ入る OP / 利用者が TRIAGE してから入る OP / 利用者だけの OP |
| TRIAGE | PROPOSAL を accept・reject・revert すること |
| PENDING | TRIAGE を待っている PROPOSAL |
| CARD / READ_CARD | agent への依頼 / 利用者が論文を読んで F を足す CARD |
| REPORT | agent が終わりに保存する報告書 |
| GUIDE | Q の項目ガイド |
| Objectivity / Logicality | O の評価の 2 軸（出典の根拠 / 筋が通っているか） |
| Next Action | 次にすべき CARD などの候補 |
| RESEARCH_TYPE | 研究の始まり方の型（PROBLEM_DRIVEN など） |
| LabAgent | sci_type を持つ sai-sci のブロックで動く agent |
| node agent / manage agent / support agent | sci_type の 3 分類 |
| context | 実行の直前に agent の最初のメッセージへ入る `# context` |
| 消す用語 | 旧模型の用語は使わない。`ui.js` などに残っていた物 |

---

## 2. 3 つの考え

### 2.1 4 layer の research-tree（SEED・ROOT・SPROUT・BRANCH）

研究の穴を Q で表し、答えの候補を O、根拠を F、確かめの実験を E で表す。
SEED は研究の初期の入力（RESEARCH_TYPE と `mind.md`）で、その下に top_Q が並ぶ（ROOT）。SEED の下の Q は child_Q に分け、O は Answer か Issue に分ける。
Answer の O の中から利用者が ★ を付けた物が STAR_PATH になる（STAR_PATH は `on_path` の印）。SPROUT は、SEED から下る PATH（終点の O）を 1 つ以上集めた node で、Issue の O を 1 つ以上含む。BRANCH（E）は SPROUT か E の下に付く。

```
E（BRANCH）── SPROUT か E の下。E の下に F（結果）か次の E
 │
S（SPROUT）── PATH（終点の O）を 1 つ以上。Issue の O を 1 つ以上
 │
SEED ──────── RESEARCH_TYPE と mind.md。Node ID は SEED
 │
Q（top_Q）── SEED の直下
 └ O（答えの候補）── ★ を付けた O が Answer（STAR_PATH）
     ├ F（根拠）
     └ O の下に child_Q を足せる
```

### 2.2 評価は Objectivity と Logicality

Q ごとに、2 つの目で進み具合を見る。

- **Objectivity** — F が目安（target）の本数そろっているか。数字を含む F には出典（論文の `cites` か、エージェントの REPORT のトークン `report`）が要る。
- **Logicality** — Q の答えの理路が通っているかを確かめた結果。矛盾（`CONTRADICTION`）が見つかれば、指摘の文と F を 2 つ以上添えて記録する。

### 2.3 Action = task.md、Next Action

- **Action** は task.md を持つ 1 回の実行（CARD）。research-tree の Node 1 つに向け、目的を 1 行で書く。
- **Next Action** は、Action の候補を画面の上の帯が 3 件まで並べる。押すと「Create CARD」で CARD のフォームが開く。

---

## 3. フォルダの中身

```
sai-sci/
├── types/      # RESEARCH_TYPE 5 本（problem・phenomenon・method・data・improve）
├── guides/     # ガイド 25 ファイル（Q の文・Child Q・Extract・Pitfall・目安）
├── template/   # sci-core・sci-find・sci-match・bw-core・exp-core・exp-run・exp-ref・sim-core・data-core・stat-core・sai-agent・sci-task・sci-cli・debate-participant と、分野の観点 domain-general・domain-medical・domain-signal-ml の 17 本
├── mind.md     # 研究の土台（mind.md）のひな形
├── tools/      # 実験ループの道具: 台帳 exp_summary.py・実行先への分配器 exp_exec.py・参照コードを探す/取る/読む/出典を確かめる道具 refcode.py・ModelDB のカタログ modeldb.py・データ分析の検査と記録 data_result.py と写し（K-Dense の EDA `eda/` と生体信号の CLI `biosignal/`）と、そのテスト（`sai-sci init` が <root>/.sai/tools/ へ配る）
├── exec/       # 実行先の見本: local（この PC・有効）・ssh（Linux の PC・無効の見本）・dok（さくらの高火力 DOK・課金あり・無効の見本）・_example（雛形）。`sai-sci init` が <root>/.sai/exec/ へ配る
└── blocks/     # 32 本のブロック（役 4 本 + sci 26 本 + 議論の 2 本）
```

### 3.1 RESEARCH_TYPE（5 本）

`types/` の 5 本。型は SEED の下の Q の階層の雛形である。本文の説明と `## Q` の節に階層（`-` が親子、`+` が並び）を持つ。frontmatter は `id`・`name`。

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

一覧は `sai-sci types`。型の本文に、最初から与えられていることが多い物と、序盤の進め方が書いてある。よく分からないときは `problem` から始める。

### 3.2 ガイド（25 ファイル）

`guides/` の 25 ファイル。Q 1 つごとに、次の節を持つ。

- frontmatter の `key`・`name`・`target`（Objectivity の目安の本数）
- `## Q` — Q の文
- `## Child Q` — child_Q の案
- `## Extract` — F として抜き出す項目
- `## Pitfall` — つまずきやすい所

上の Q 9 本と child_Q 16 本がある。

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

### 3.3 32 本のブロック

`blocks/` の 32 本。`sai-sci init` が、これらを `sai-sci init --type <型>` や `sai-sci init --update` でプロジェクトの `.sai/blocks/` に配る。

- 役 4 本（`professor`・`professor-backward`・`operator`・`kaizen`）は、research-tree の外から research-tree を見る役（3.3.1 節）。
- sci の 26 本は、research-tree の中で動く researcher。Action の種類で分ける。

| Action | ブロック | support |
|---|---|---|
| Q を分ける | `sci-start`・`sci-split`・`sci-restore`（逆向き） | ― |
| O を書く | `sci-ideate` | `sci-analogy` |
| F を探す | `sci-search`・`sci-screen`・`sci-read`・`sci-cite-check`（逆向き） | `sci-lookup`・`sci-extract`（`sci-search` も呼ばれれば support） |
| Logicality を確かめる | `sci-debate`・`sci-review` | ― |
| ★ を選ぶ | なし（利用者だけ）。手伝いに `sci-compare` | ― |
| 実験 | `sci-exp-sprout`・`sci-exp-branch`・`sci-exp-review`・`sci-result-compare`（逆向き） | `sci-exp-scout`（参照実装を探す）・`sci-sim-model`（論文 1 本のモデル定義書）・`sci-sim-compose`（つなぎのモデル定義書） |
| データ分析 | `sci-data-explore`・`sci-data-test` | ― |
| その他 | `sci-digest`・`sci-coach`・`sci-typify` | ― |

- support agent は、ほかのブロックが呼ぶ agent。単独の Action としては動かさない。
- `sci-exp-branch` だけは、research-tree の範囲も共通の決まり（`sci-core`）も受け取らない（実験ループ。5.5 節）。
- 逆向きの 4 本（`sci-restore`・`sci-cite-check`・`sci-result-compare` と `professor-backward`）は 5.6 節のためのもの。

#### 3.3.1 役の関係

```
professor / professor-backward（root の run。researcher を直接起こす）
  |-- researcher（sci の 26 本。research-tree の Node に向けて働く）
  `-- operator（professor の子。sai-moon の操作・監視・情報の抽出だけ）
```

| id | 役 | 共通の決まり | 研究の判断 |
|---|---|---|---|
| `professor` | 研究の現在地を読み、次の研究作業を researcher に任せ、報告を評価して TRIAGE をする | `sai-agent` / `sci-task` / `sci-cli` | する。root の run になる。研究作業は自分でしない |
| `professor-backward` | 既存の論文から始める逆向きの舵取り（5.6 節） | 上の 3 本と `bw-core` | する。root の run になる |
| `operator` | sai-moon の故障の対処・監視・情報の抽出・Web と同じ操作 | `sai-agent` / `sci-cli` | しない。professor の子 |
| `kaizen` | professor / professor-backward の実行履歴から問題を洗い出し、改善計画書を作る。コードも research-tree も直さない | `sai-agent` / `sci-task` / `sci-cli` | しない |
| `sci-*` | 研究作業そのもの（読む・比べる・O を出す・考える・書く・探す・実験する） | `sci-core`（+ `sci-find`・`sci-match`・`exp-core`） | しない。CARD に書かれた作業だけ |

- CARD の `executer` は起動者（`human` / `agent`。古い値 `professor` は `agent` として読む）。`--executer agent` の CARD は人間が Web から起こせず、professor が `sai-link task create` の `card` で自分の子として起こす。
- 子からの質問は親の `sai-link task wait` に `stalled`（`reason: "question"`）で届き、`sai-web question` / `answer` で答える。Web で人間が先に答えた場合は 400 で断られる。
- 完了は、子は `sai-link task report`、root（professor）は質問ツールで「完了してよいか」を確かめて `sai-link run finalize` に `report`。
- 研究日誌は `.sai/paper/professor-log.md`。operator の学びは `~/.sai/operator-notes.md`（`SAI_HOME` があればその中）。

### 3.4 template（17 本）と mind.md

- **`template/sci-core.md`** — エージェントに共通で渡す決まり。context の読み方、PROPOSAL の書き方、使える op、根拠の扱い、迎合しない姿勢。
- **`template/sci-find.md`** — F を探すブロックの共通の決まり。
- **`template/sci-match.md`** — 論文と Q・O を照らし合わせるときの共通の決まり。
- **`template/bw-core.md`** — 逆向きの運用の共通の決まり（5.6 節）。
- **`template/exp-core.md`** — 実験ループの共通の決まり。役割、文書の渡し方、実験仕様書、MLflow の記録の決まり、台帳、判定の規則、実行場所の規則、停止の条件。
- **`template/exp-run.md`** — 実験のスクリプトを「実行先」に投げる手順（分配器 `exp_exec.py` の呼び方）。
- **`template/exp-ref.md`** — 参照コードと出典の決まり。参照実装をどう探すか（Library → GitHub → ModelDB）、値にどう `# src:` で出典を付けるか、どこで止めるか。取得は `refcode.py` で行い、置き場は `<sai_home>/refcode/`。
- **`template/sim-core.md`** — 神経シミュレーションの共通の決まり。モデル定義書の書式、式・定数を `refcode.py eqs` / `params` で抜く順序、単位と数値計算の決まり、再現の目標の決め方、改良と組み合わせの決まり。`sci-sim-model` と `sci-sim-compose` が読む。
- **`template/data-core.md`** — データ分析の共通の決まり。データの安全（元のデータを壊さない）・データ表・EDA・前処理・時系列の EDA・MLflow への記録・作業フォルダ・再現性。`sci-data-explore` と `sci-data-test` の 2 本が読む。
- **`template/stat-core.md`** — 統計解析の決まり。解析計画の写法・検定の選び方・前提の確かめ・多重比較・効果量と検出力・回帰・時系列・機械学習・ベイズ・図・`results.json`・報告。`sci-data-test` が読む。
- **`template/sai-agent.md`** — 研究プロジェクトのエージェントに共通で渡す決まり。文書だけでの引き継ぎ・waggle トークン・`sai-link doc write` / `sai-link doc read`・書き方の原則・完了のしかた。
- **`template/sci-task.md`** — 子タスクの扱い（professor が使う）。
- **`template/sci-cli.md`** — 研究プロジェクトでの命令（professor と operator が使う）。
- **`tools/refcode.py`** — 参照コードの道具。Library からリンクを引く（`links`）・GitHub を検索する（`gh-search`）・リポジトリを浅く取る（`fetch`）・コードから数値の定数を抜く（`params`）・`train.py` の数値に出典の注記があるか確かめる（`check`）。
- **`tools/modeldb.py`** — ModelDB のカタログ。カタログを手元の jsonl に写す（`sync`）・カタログだけを読む（`search`・`show`・`status`）。カタログは `<sai_home>/refcode/modeldb/catalog.jsonl` に置く。
- **`tools/data_result.py`** — データ分析の検査と記録。データ表・前処理の結果・検定の結果を `results.json` として読み書きし、REPORT の骨を作る。
- **`tools/eda/`** — データ表・画像・時系列・構造化データの EDA の CLI（K-Dense からの写し。MIT。`LICENSE` と `README.md` つき）。
- **`tools/biosignal/`** — 心電（HRV）・皮膚電反応・合成信号生成・解析の epochs の計画の CLI（K-Dense からの写し。MIT）。
- **`mind.md`**（`sai-sci/mind.md`）— 研究の土台のひな形。`ORIGIN・RESEARCH_TYPE・GIVEN・CONSTRAINTS・VALUE_AXES・INTERESTS・NOTES` の 7 節。**利用者が書くファイルで、エージェントは直接書き換えません。** `GIVEN` は RESEARCH_TYPE で最初から与えられている物（SEED に入る物）、`VALUE_AXES` は O を比べるときの軸（`sci-compare`）になる。

---

## 4. 始め方

### 4.1 新しいプロジェクト

Web の Research タブの research-tree で「始める」を押して型を選ぶか、次を打つ。

```bash
sai-sci init --type <型>
```

`init` を終えたあとに `sai-sci init --type <型>` と同じことをします。`mind.md` が無ければ、`sai-sci/mind.md` のひな形を書きます。

### 4.2 今のプロジェクト

```bash
sai-sci init --type <型>
```

型の一覧は `sai-sci types`。

古い形の research-tree（旧い版の `sci_*` の表だけがある research-tree）のプロジェクトでは、画面と `sai-sci status` が「作り直し」を促します。作り直しは新しい `sai-sci init` です。古い表は消さず、新しい research-tree はそれを読みません。

### 4.3 入る物

| 入るもの | 置き場 |
|---|---|
| 32 本のブロック | `<root>/.sai/blocks/` |
| template 17 本（と各ブロックが使う `search-internal` などのテンプレ、`domain`） | `<root>/.sai/template/` |
| 台帳 `exp_summary.py` と分配器 `exp_exec.py`・`refcode.py`・`modeldb.py`・`data_result.py`・写しの `eda/`・`biosignal/`（無いときだけ書く） | `<root>/.sai/tools/` |
| 実行先の見本 `local`・`_example` など（無いときだけ書く） | `<root>/.sai/exec/` |
| `.sai/settings.json` の `context`（実行直前に範囲を作る命令。無いときだけ書く） | `<root>/.sai/settings.json` |
| `mind.md`（無いときだけ書く） | `<root>/.sai/paper/mind.md` |
| 型とガイドの写し | `<root>/.sai/sci/types/` と `<root>/.sai/sci/guides/` |
| 取得した参照コードと ModelDB のカタログ | `<sai_home>/refcode/`（`SAI_HOME` が無ければ `~/.sai`） |
| research-tree | `papr.db` の `rt_*` の表 |
| 書き出し（`state.md` と `tree.md`） | `<root>/.sai/paper/` |

**手で足す設定はありません。** `init` だけで sci の設定が終わります。

---

## 5. 使い方の流れ

### 5.1 1 本道: Q → O → F → ★ →（答えが無ければ）Issue と E

```
Q を分ける ─▶ O を書く ─▶ F を付ける ─▶ ★ を付ける ─▶ （答えが無ければ Issue）─▶ E
```

画面（`#/research/tree`）での操作:

1. **Q を分ける** — Next Action の「Create CARD」から `sci-start`・`sci-split` を動かす。または板の「Q を足す」で自分で足す。
2. **O を書く** — Q の板の「O を足す」、または `sci-ideate` の CARD。Answer か Issue かは O ごとに変えられる（「Answer / Issue を変える」）。
3. **F を付ける** — Library で論文を探し（検索セットの keep / drop はエージェントの `sci-screen` が `sai-paper set` で行う。Web に検索セットの画面は無い）、READ_CARD（`#/research/read/<run_id>`）で 1 本ずつ読んで F を書く。エージェントが出した F は自動で research-tree に入る。取り消したいときは Node を選んだ右の板の PENDING の節で revert で戻す。
4. **★ を付ける** — O の板の「★ を付ける」。★ を付けられるのは利用者だけ。比べるときは `sci-compare` が手伝う。
5. **答えが無ければ Issue** — O を Issue にし、E（実験）を足す。実験の回し方は 5.5 節。

### 5.2 Web の research-tree の画面

Research タブの最初の画面が research-tree（`#/research/tree`）。

- 上の帯: **Next Action**（先頭 3 件。`[NO_CHILD_Q] Q24 sci-split「…」` の形で、右に Create CARD）・折りたたみの **History**（全 Node の履歴を古い順に。`sai-sci status` の `history[]`）・注意。
- 左: Q・O・F・E・S（SPROUT）の一覧。
- 真ん中: **DAG**（research-tree の図。下に ROOT（top_Q）、次に SEED の欄、上に SPROUT、さらに上に BRANCH（E））。SEED の欄を押すと RESEARCH_TYPE と `mind.md` の節が出る。Node を押すと `#/research/tree/Q12` のように住所が変わり、詳細が右の **SB**（サイドバー）に出る。SPROUT を押すと、その SPROUT に関係しない札と線が薄くなる。
- SB: 見出し（Q は Objectivity と Logicality の札付き）・親までの道・PENDING の節（TRIAGE を行う）・操作の釦・Next Action・F・History など。何も選んでいないときは出ない。PC 幅（900px 以上）は右（約 420px・✕ で閉じる）、iPhone 幅は下から出る sheet。
- **Create SPROUT**: SEED の SB か上の帯の釦を押すと、History と DAG の間に作成パネルが出る。作成中も DAG・一覧・SB を操作でき、O の SB の **Add PATH** / **Delete PATH** で PATH（終点の O）を選ぶ。PATH は 1〜20 個で、Issue の O を 1 つ以上含める。
- TRIAGE: Node を選んだ右の SB の PENDING の節で行う。
- READ_CARD: `#/research/read/<run_id>`。
- 検索セット: Web に画面は無い。keep / drop の選別は `sci-screen` ブロックが `sai-paper set` で行う。

### 5.3 sai-link がブロックを動かすとき

`sci-*` のブロックは `context: true` です。sai-link は実行の直前に `.sai/settings.json` の `context` の命令を実行し、標準出力を、最初のメッセージの `# context` の節に入れます。依頼文は同じメッセージの `# task_text` の節に入ります。

TREE の範囲はブロックの sci_type で決まります。`node` は対象の Node とその子孫、`manage` は SEED から対象までの祖先と対象とその子孫（対象が `SEED` なら research-tree の全体）、`support` には出しません。範囲を作れなかったときは 1 行の注意が `# context` の下に入ります（実行は止まりません）。

**決まり:** `.sai/settings.json` の `context` に別の命令を指定するときは、その出力も `##` から始めてください。

### 5.4 エージェントは PROPOSAL だけを出す

op（操作）は危険度で 3 つに分かれます。

- **自動で入る物**（`add_o`・`add_f`・`add_e`・`add_sprout`・`record_logic`）は、出した PROPOSAL がすぐ research-tree に入り、自動で反映された印が付く。
- **利用者が TRIAGE する物**（`add_q`・`copy`・`remove`）は、TRIAGE（accept）するまで research-tree に入らない。TRIAGE は Node を選んだ右の板の PENDING の節で行う。
- **利用者だけの物**（`star`・`unstar`・`set_kind`・`edit`・`set_target`）は、エージェントは出せない。
- どちらも、利用者は 1 押しで取り消せる（revert）。

#### LabAgent の 3 分類

research-tree で動くエージェント（LabAgent）は、ブロックの frontmatter の `sci_type` で 3 つに分かれます。それぞれ TREE（`# context` に切り出す research-tree の範囲）と終わり方が違います。

| sci_type | 動き方 | TREE の範囲 | 終わり方 |
|---|---|---|---|
| node | 対象の Node を指定されて動く | 対象＋子孫 | REPORT を `sai-link doc write` で 1 つ保存する |
| manage | research-tree の全体を見て動く | 祖先＋対象＋子孫（対象が `SEED` なら全体） | REPORT を `sai-link doc write` で 1 つ保存する |
| support | node agent・manage agent に呼ばれ、1 つの道具の使い方に特化した作業をする | TREE なし | REPORT を保存しない。呼んだ agent に `sai-link task report` で返す |

ブロックごとの分類:

| sci_type | ブロック |
|---|---|
| node | sci-split・sci-ideate・sci-search・sci-screen・sci-read・sci-compare・sci-debate・sci-coach・sci-cite-check・sci-data-explore・sci-data-test・sci-exp-review |
| manage | sci-start・sci-review・sci-digest・sci-restore・sci-typify・sci-result-compare・sci-exp-sprout |
| support | sci-analogy・sci-extract・sci-lookup・sci-exp-scout・sci-sim-model・sci-sim-compose・sci-exp-branch・paper-debater・paper-reviewer |
| なし | professor・professor-backward・kaizen・operator |

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
- **止め方**: 判定が `stop` のときだけ。`max_branches` が branch の総数の上限になる。
- **最初の sprout の起動**は、`sai-sci card --node SEED` で作る。`--input` は依頼文の末尾に載せる文書の waggle トークン（繰り返し可。`mind.md` や参照コード調査書、逆向きなら復元書）。

```bash
sai-sci card --node SEED --block sci-exp-sprout \
  --purpose "<解きたい Issue の O を含む SPROUT を作る>" \
  --input <参照コード調査書や復元書のトークン> --json
sai-link card run <返ってきた run_id>
```

- 詳しい決まりは `template/exp-core.md`。

**実行先**（スクリプトを動かす PC）は環境変数 `EXP_TARGET` で選ぶ。空なら `local`（この PC）。実行先は `<root>/.sai/exec/<名前>/` のフォルダで、Mac 本体・LAN の PC・クラウドなどを利用者が個別に足せる（フォルダを置くだけ。増減で Rust や画面を直さない）。エージェントは分配器 `exp_exec.py`（`run` / `status` / `logs` / `cancel` / `check` / `list`）だけを呼び、実行先の中身を知らない。LAN の Linux の PC やクラウドの VM は、`ssh/` を PC の名前で写して host を書けば足せる（`cp -r .sai/exec/ssh .sai/exec/lab-linux`）。さくらの高火力 DOK は `dok/`（`README.md`・`SETUP.md` を先に読む。課金される）。Windows など別の種類は `_example/` を写して adapter を書く。記録は実行先から中央の MLflow に直接入る。台帳コマンド `exp_summary.py` と分配器は `sai-sci init` が `<root>/.sai/tools/` に置く。

### 5.8 逆向き（既存の論文から始める）

前向き（5.1〜5.7）は研究の構想から research-tree を育てます。**逆向き**は、既存の論文（基盤論文 1〜5 本）を出発点にして、論文に書いてある研究方針を research-tree に復元し、その根拠を確かめ、論文の手法を再現し、新しい論文で直し、比べます。決まりは `template/bw-core.md` にまとめてあります。

| 段 | すること | ブロック |
|---|---|---|
| 1. 抽出 | 序論を上の Q に、手法と実験の設定を下の Q に割り当てる。結果の数値を記載結果表に、主張と引用の組を対応表にする | `sci-restore` |
| 2. 評価 | 主張ごとに引用文献を取り寄せて、根拠になっているかを判定する。そのうえで、復元した research-tree を審査する | `sci-cite-check` → `sci-review` |
| 3. 再現 | PATH を選んで再現の SPROUT を作り、論文の手法を動かす | `sci-exp-sprout` |
| 4. 更新 | 基盤論文より新しい論文を探して O を直し、更新の SPROUT を作って動かす | 前向きのブロック |
| 5. 比較 | 論文の記載値・再現・更新を比べる | `sci-result-compare` |

- **始め方**: 基盤論文を Library に入れ（Web の論文の画面）、research-tree を作ってから（`sai-sci init --update --type <型>`）、`professor-backward` の CARD を人が起こします。依頼文に `基盤論文: #12 #15` の形で番号を書きます。
- **主張と根拠を分けます。** 基盤論文の文は「主張」として F にし、O は Issue の側に置きます。引用文献で支えられ、審査で良になった O だけを professor-backward が ★ の候補にします（★ を付けるのは利用者です）。
- **再現は論文に合わせにいきません。** 論文に無い設定だけを仮置きとして置き、論文に書いてある値と手法は変えません。判定が `new_script`（別の手法を試す）になったら、再現はそこで止めて比較の材料にします。
- **Human に聞くこと**: 再現の許容幅、実行先（課金の有無）、同じ型の論文が 2 本以上あるときにどれを再現するか、段を飛ばすか。
- 研究日誌は `.sai/paper/professor-backward-log.md` です。

### 5.9 データ分析（手元のデータ）

実験（5.7 節）がスクリプトを回すのに対し、データ分析は**手元にあるデータ**（データ表・CSV・画像・時系列・生体信号）を読みます。2 本のブロックで道筋を分けます。

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

- **最初の explore の CARD の作り方**: データの path と、調べたいことを `--purpose` に書きます。

```bash
sai-sci card --node <N> --block sci-data-explore \
  --purpose "<データのパスと調べたいこと>" --json
sai-link card run <返ってきた run_id>
```

- **test の CARD は explore の REPORT から作ります。** `sci-data-explore` 自身は test を起こしません。REPORT（run フォルダの `finalize-report.md`）の `## Next Action` に書かれた命令をそのまま使います。professor はその REPORT を評価してから起こします（5.7 節の sprout → branch と同じ考え方）。
- **置き場**: 作業は `analysis/<study>/` に置きます。`<study>` は `<Node の ID>-ID<explore の run_id>` の形です（5.7 節と同じ名前）。元のデータは書き換えません（前処理の出力は `analysis/<study>/` の中にだけ作ります）。
- **MLflow の名前**: experiment は `<project>/ID<explore の run_id>/data`、run は explore が `ID<run_id>-eda`、test が `ID<run_id>-test`。タグは `sai.project`・`sai.run_id`・`exp.study`・`sci.node`・`exp.kind`。
- **解析はこの PC で動かします。** 30 分を超えるもの・データの量が大きいものはデータ分析ではなく実験の流れ（5.7 節）へ回します。
- 詳しい決まりは `template/data-core.md`（データの安全・データ表・EDA・前処理・MLflow・research-tree・再現性）と `template/stat-core.md`（解析計画・検定の選び方・前提・多重比較・効果量と検出力・回帰・時系列・機械学習・ベイズ・図・`results.json`・報告）。
- 使う道具は `tools/data_result.py` と、写しの `tools/eda/`（データ表・画像・時系列の EDA）と `tools/biosignal/`（心電・皮膚電反応）です。写しは K-Dense（MIT。commit `1549884`）からのもので、`.sai/tools/eda/`・`.sai/tools/biosignal/` にそのまま配られます（`$SAI_PROJECT_ROOT/.sai/tools/eda/<名前>` で呼びます）。

---

## 6. 命令の一覧

### 6.1 `sai-sci …`

| 命令 | 何をするか |
|---|---|
| `sai-sci init --type <型> [--json]` | 研究プロジェクトの配る物を配り、SEED の下に top_Q を作る |
| `sai-sci types [--json]` | 使える RESEARCH_TYPE を一覧する |
| `sai-sci type-save` | RESEARCH_TYPE を `<sai_home>/sci/types/` に保存する（標準入力に型の Markdown。JSON で返す） |
| `sai-sci status [--json] [--brief]` | 現況（進み具合・SEED・research-tree・Next Action）。`--brief` は数だけを JSON で出す |
| `sai-sci node <Q12\|O5\|F3\|E2\|S2> [--json]` | Node 1 つの詳細 |
| `sai-sci context --run <run_id> [--tree]` | 今の実行の `# context` を標準出力へ出す |
| `sai-sci propose [--run <run_id>] [--block <id>]` | PROPOSAL を出す（標準入力に `{"proposals":[…]}`） |
| `sai-sci apply` | 利用者の操作を 1 つ反映する（標準入力に 1 件の JSON） |
| `sai-sci proposals [--json]` | PENDING の PROPOSAL の一覧 |
| `sai-sci triage <P56> (--accept [--edit <JSON>] \| --reject \| --revert)` | PROPOSAL を TRIAGE する |
| `sai-sci card --node <Q12> --purpose <文> [--block <id>] [--expect <文>] [--mode <purpose_first\|blind_first>] [--papers <番号,…>] [--runner pi\|cc\|user] [--depth abstract\|figures\|full] [--origin-run <run_id>] [--input <トークン>]… [--executer human\|agent] [--json]` | Node に向けた Action（CARD）を作る。`--runner user` は READ_CARD |
| `sai-sci read <run_id> [--json]` | READ_CARD の中身 |
| `sai-sci export` | research-tree を `state.md` と `tree.md` に書き出す |

書き込む命令は、終わる前に `export` を実行します。Web の窓口は `sai-sci {status,init,node,apply,proposals,triage,card,read,types}` を呼びます。検索セットは `sai-paper set {list,show,create,mark,merge,delete,rerun}`。

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
| research-tree（Q・O・F・E・PROPOSAL・SEED・検索の記録） | `papr.db`（`rt_` で始まる表） |
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
- **形の決まり**: 型は `id`・`name` の frontmatter と `## Q` の節、ガイドは `key`・`name`・`target` の frontmatter と `## Q`・`## Child Q`・`## Extract`・`## Pitfall` の節が要ります。崩れていれば 1 行の warning として `status` の `warnings` に入ります。
- **確かめ**:
  ```bash
  cd ~/workdir/Sai/sai-moon
  env -u SAI_PROJECT_ROOT -u SAI_PARENT_RUN_ID -u SAI_BLOCK_NAME \
    perl -e 'alarm 1800; exec @ARGV' cargo test -p sai-sci-cli
  ```
- 利用者が直した `.sai/sci/` の写しを優先して読むので、配布の版を上げたいときは 8.2 節の手順を使います。

### 8.1 古い区分と新しい物の対応

古い版（旧 research-tree）の言葉と、今の物の対応です。**この表の中だけ古い語を書きます。**

| 古い区分 | 新しい物 |
|---|---|
| ITEM | Q |
| OPTION | O |
| FINDING | F |
| sprout / branch（旧 4 層の下の 2 層） | E の親子 |
| GATE・BLANK・MARK・TENTATIVE | Objectivity と Logicality |
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

- 古い形の research-tree（旧 `sci_*` の表）は消えません。新しい research-tree は `sai-sci init` で作り直します（4.2 節）。
- 利用者全体の置き場（`~/.sai/` の `blocks/`・`template/`・`sci/types/`）に同じ名前の物があるときも、同じように移してから配ります。
- この手順は文書に書くだけで、開発の作業では実行しません。

---

## 9. Canvas から移った人へ

- **Canvas の画面と `sai-paper canvas` はありません。** 代わりに `#/research/tree`（research-tree）と `sai-sci` を使います。
- **今までの `.sai/paper/canvas/*.canvas` は消していません。** JSON Canvas 1.0 の形なので、他の道具（Obsidian など）で開けます。
- **`paper-map` は report だけを出すようになりました。** Canvas を作る手順は外れています。
- **古い paper-map の写し**（`<root>/.sai/blocks/paper-map.md`）には、無い命令が書いてあることがあります。8.2 節の手順で新しい写しにしてください。
