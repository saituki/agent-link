# sai-sci（research-tree）

**読み手**: 利用者と、`sai-sci/` の md を直す開発者。

初めて使う人は、手を動かしながら機能を回る [`sai-sci-tutorial.md`](sai-sci-tutorial.md) から始めてください。

`sai-sci` は、**研究の方向づけの段階（実験の前にある「何をやるか」を決める段階）を、問い（Q）・答え（A）・根拠（F）・実験（E）・seed（D）・SPROUT（S）の research-tree で扱う仕組み**です。
research-tree の正を 1 か所（`papr.db`）に置き、絵と文章（`state.md`・`tree.md`）はその正からの書き出しにします。

- research-tree は、論文の置き場があるプロジェクトごとに 1 つあります。
- Web（`apps/web`）・CLI・cc の質問フックが、同じ research-tree を読み書きします。
- エージェントは `sai-sci op` で木を変えます。許可が要る op は、利用者が質問で「許可する」を選ぶまで入りません（5.4 節・5.10 節）。

## 1. sai-sci とは

- **research-tree の正は `papr.db`** です。Q・A・F・E・S・D・SEED の記録と承認の記録は `rt_` で始まる表の中にあります。
- **`state.md` と `tree.md` は書き出し**です。手で直しても次の書き出しで消えます。
- **research-tree は seed・root・sprout の 3 つの役でできています。** seed（D）は 1 つの研究の制約（複数持てる）、root は分野の可能性の幅（軸の Q・選択肢の A・論文の事実の F。seed に縛られない）、sprout（S）は 1 つの seed と、軸ごとに 1 本の root の path を組にした 1 研究＝研究計画です。
- **root の骨組みは、どの RESEARCH_TYPE でも同じ 6 つの軸の Q です。** RESEARCH_TYPE（5 本）の違いは、seed のどの項目が最初から「決定」になりやすいかだけです。
- **エージェントは `sai-sci op` で木を変えます。** 危険度は 3 つに分かれます（5.4 節）。

## 用語（専門用語は英単語のまま使います）

| 用語 | 意味 |
|---|---|
| research-tree | seed（D）・SEED・ROOT・SPROUT・BRANCH の層の木と、それを描いた図の総称 |
| layer | 層（seed・SEED・ROOT・SPROUT・BRANCH） |
| seed（D） | 1 つの研究の制約。親なし・複数。9 項目の本文を持つ（9 見出しの形は 2.1 節）。Node ID は `D<n>`（例 `D1`）。Web では SEED の札の横に並ぶ |
| SEED | root の根（RESEARCH_TYPE と mind.md）。Node ID は `SEED`（DB の行は持たない）。seed（D）とは別の物 |
| ROOT | SEED から下へ分かれる Q・A・F の総称。分野の可能性の幅で、全部の seed が同じ root を使う |
| 軸の Q | top_Q の 6 つ（task・target・data・method・output・metric）。「〜には、どんな種類があるか」の形 |
| SPROUT | 1 つの seed（D）と、軸ごとの path の組＝1 研究（ID は `S<n>`。親は D）。研究計画を body に書く |
| 研究計画 | S の body。`## 研究計画`（軸ごとの選び）・`## 成功基準`・`## 矛盾`・`## 先行研究との違い` |
| BRANCH | SPROUT か E の下に分かれる E（実験の実行） |
| Node | research-tree の 1 つの要素（Q・A・F・E・S・D） |
| Q / A / F / E / S / D | 問い / 選択肢 / 根拠 / 実験 / SPROUT / seed |
| 種類の頭文字 | ID の頭文字が種類。形は `^[QAFESD][1-9][0-9]{0,8}$`（例 `Q12`・`A15`・`D1`） |
| stance | F の立場。`support`・`deny`・`issue` のどれか |
| top_Q | SEED の直下の Q（親が NULL）。軸の Q の 6 つ |
| parent_Q / child_Q | parent_Q は親にあたる Q、child_Q は子にあたる Q |
| bottom_Q | 生きた child_Q を持たない Q（JSON は `bottom_q`） |
| PATH | 軸の Q（top_Q）から A に至る道。終点の A で表す。S は軸ごとに 1 本選ぶ |
| ★ | 利用者が付ける目印。計算には使わない |
| 範囲（range） | SPROUT が数える Q・A と F の数（2.3 節） |
| OP | research-tree を変える操作 1 つ（`add_q` など）。`sai-sci op` と `sai-sci apply` で行う |
| 危険度 | OP の 3 つの区分。`Free`・`Approval`・`UserOnly`（5.4 節） |
| 承認 | 質問で「許可する」を選んでもらうこと。許可が要る op を 1 バッチ分通す |
| 見た番号の台帳 | run ごとに agent が CLI で見た論文の番号 `#x.y.z` の記録。`add_f` の `cites` と報告の引用の照合に使う。MLflow・実験の台帳（`exp_summary.py`）とは別の物（5.11 節） |
| CARD / READ_CARD | agent への依頼 / 利用者が論文を読んで F を書く CARD |
| Next Action | 次にすべき CARD の候補 |
| REPORT | node agent・manage agent が終わりに保存する報告書 |
| GUIDE | 軸の Q のガイド（6 本）。A に何を並べるか・F に何を抜くかを決める |
| STUDY_SEED | 対象が S のとき `# context` に出る節。その S の seed（D の本文） |
| 検証 agent | `sci-verify`。target_node と子の関係を評価する |
| LabAgent | sci_type を持つ sai-sci のブロックで動く agent |
| node agent / manage agent / support agent | sci_type の 3 分類 |
| context | 実行の直前に agent の最初のメッセージへ入る `# context` |
| 消す用語 | 旧模型の用語は使わない。旧い語と今の物の対応は 8.1 節 |

---

## 2. 木の考え

### 2.1 層（seed・SEED・ROOT・SPROUT・BRANCH）

research-tree は、seed・root・sprout の 3 つの役に分けてできています。

- **root** は分野の可能性の幅です。seed に縛られません。SEED の直下の Q（軸の Q＝top_Q）が軸ごとに 1 つずつ並びます。Q は「〜の種類は何があるか」の形、A は選択肢 1 つ、F は論文の事実です。
- **seed（D）** は 1 つの研究の制約です。親はなく、研究ごとに複数持てます。
- **sprout（S）** は 1 研究＝研究計画です。1 つの seed（D）と、軸ごとに選んだ root の path（A）の組でできます。1 つの D から複数の S を作れます。
- 1 研究＝1 つの S＝1 つの D ＋ 軸ごとに 1 本の path の組です。先行研究を論文から復元した研究と自分の研究は、木の上で区別しません（どちらも D と S）。自分の研究の新しさは、先行研究の S と違う seed・path を通ることで示します。
- root の Q・A に「この研究では」「本研究では」を書きません。研究の決定は seed（D）と sprout（S）に書きます。

```
E（BRANCH）── S か E の下。実験の 1 回
 │
S（SPROUT）── 1 研究＝研究計画。親は D。paths（軸ごとの A）を 1 つ以上
 │
D（seed）──── 1 研究の制約（9 項目）。親なし。複数。今の SEED の札の横に並ぶ
SEED ──────── root の根（RESEARCH_TYPE と mind.md）。Node ID は SEED・DB の行なし
 │
Q（軸の Q＝top_Q）── 「〜の種類は何があるか」。task・target・data・method・output・metric
 └ A（選択肢）── 出典を書かない（op が断る）
     ├ F（論文の事実。stance は support・deny・issue）
     └ child_Q（その A の下の種類・条件）…
```

seed の本文は次の 9 見出しで、この順に書きます。軸の 6 項目（解く問題・対象・データ・手法・出力・評価指標）は、見出しの次の行が `決定: <値>` か `空欄` のどちらかです。

```markdown
## 背景
<自由な文>
## 解く問題（task）
決定: 発作部位の特定
## 対象（target）
決定: 側頭葉てんかん患者
## データ（data）
決定: 術前の EEG・ECoG
## 手法（method）
空欄
## 出力（output）
空欄
## 評価指標（metric）
空欄
## 成功基準
<指標の値と向き。未定なら「空欄」>
## 制約
<期間・計算資源・倫理・データの入手など>
```

S の body（研究計画）は `lab-sprout` が書き、`sci-exp-sprout`・`sci-restore` が読みます。形は `## 研究計画`（軸ごとに `- <key>: 決定「…」→ A3` か `- <key>: 空欄 → A15`）・`## 成功基準`・`## 矛盾`・`## 先行研究との違い` です。

### 2.2 親子の決まり

`rt/ops.rs` の `execute` だけが守ります（DB は守りません）。

| 子 | 置ける親 |
|---|---|
| top_Q（親 NULL） | `SEED` |
| Q | 生きた A の子を持たない Q、または A |
| A | 生きた Q の子を持たない Q |
| F | A だけ |
| E | S か E |
| S | D（seed） |
| D | 親なし |

- Q の下は「A だけ」か「child_Q だけ」で、混ぜません。A の下は F と Q を混ぜてよい。
- 実験の結果は、E の `mlflow` で MLflow に結ぶだけです。E の下に F は置かず、実験の結論も木に足しません。
- A に出典（`#番号`）は書けません。出典は A の下の F（`add_f` の `cites`）に書きます。
- D の下に生きた S があると、その D は消せません。先に S を消します。

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
├── types/      # RESEARCH_TYPE 5 本（problem・phenomenon・method・data・improve）。どれも同じ 6 軸
├── guides/     # ガイド 6 ファイル（軸の Q の文・Child Q・Extract・Pitfall）
├── template/   # sci-core・sci-find・sci-match・bw-core・exp-core・exp-run・exp-ref・sim-core・data-core・stat-core・sai-agent・sci-task・sci-cli・debate-participant と、分野の観点 domain-general・domain-medical・domain-signal-ml
├── mind.md     # プロジェクトの覚え書き（mind.md）のひな形
├── tools/      # 実験ループの道具（台帳・分配器・参照コード・ModelDB・データ分析・EDA・生体信号）。`sai-sci init` が <root>/.sai/tools/ へ配る
├── exec/       # 実行先の見本: local・ssh・dok・_example。`sai-sci init` が <root>/.sai/exec/ へ配る
└── blocks/     # 役 4 本・lab-* 8 本・検証（sci-verify）・sci-* の作業・参加者（paper-debater・paper-reviewer）
```

### 3.1 RESEARCH_TYPE（5 本）

`types/` の 5 本。**どの型も `## Q` は同じ平らな 6 行（軸）です。** 型の違いは、seed のどの項目が最初から「決定」になりやすいかで、説明の段落と `## SEED` の節（engine は読まない。`sci-start` と利用者が読む）に書きます。`## Q` の `+` は `-` と同じに読みます（古い型を読めるように残している）。frontmatter は `id`・`name`。

置き場の順（先に見つかった物を使う）: 

1. バイナリに埋め込んだ版
2. `<sai_home>/sci/types/`（利用者全体の置き場。`sai-sci type-save` で書く）
3. `<root>/.sai/sci/types/`（プロジェクトの写し）

| id | 名前 | seed で決定になりやすい項目 | 空欄で始まりやすい項目 |
|---|---|---|---|
| `problem` | PROBLEM_DRIVEN（課題駆動） | 解く問題（task）・背景 | 手法・出力・評価指標 |
| `phenomenon` | PHENOMENON_DRIVEN（現象駆動） | 対象（target）・背景 | データ・手法・評価指標 |
| `method` | METHOD_DRIVEN（手法駆動） | 手法（method） | 解く問題・対象・評価指標 |
| `data` | DATA_DRIVEN（データ駆動） | データ（data） | 解く問題・手法・出力 |
| `improve` | IMPROVE_DRIVEN（改良駆動） | 手法（method。改善の元）・評価指標（metric） | 改善の方向は手法の A の下で探す |

一覧は `sai-sci types`。よく分からないときは `problem` から始めます。

### 3.2 ガイド（6 ファイル）

`guides/` の 6 ファイル。root の軸の Q 1 つごとに 1 本あり、次の節を持ちます。

- frontmatter の `key`・`name`
- `## Q` — 軸の Q の文
- `## Child Q` — A の下に足す child_Q の案（どれも「〜の種類は何があるか」の形）
- `## Extract` — F として抜き出す項目（どの軸も「適用の条件」と「限界・失敗する条件」を含む）
- `## Pitfall` — つまずきやすい所（「この研究では」と書かない・A に出典を書かない・A 1 つは選択肢 1 つ、は 6 本共通）

| key | 名前 | 軸の Q の文 | seed の見出し |
|---|---|---|---|
| `task` | TASK | この分野で解かれている問題（タスク）には、どんな種類があるか。 | `## 解く問題（task）` |
| `target` | TARGET | 研究の対象には、どんな種類があるか。 | `## 対象（target）` |
| `data` | DATA | 使われているデータには、どんな種類があるか。 | `## データ（data）` |
| `method` | METHOD | 使われている手法には、どんな種類があるか。 | `## 手法（method）` |
| `output` | OUTPUT | 研究が出す出力の形には、どんな種類があるか。 | `## 出力（output）` |
| `metric` | METRIC | 使われている評価指標には、どんな種類があるか。 | `## 評価指標（metric）` |

古い木の `guide_key`（`background` など）は、プロジェクトの `.sai/sci/guides/` に古い写しが残っているので、そのまま読めます。

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

- `sai-sci card` に `--executer` は無く、作る CARD は人が起こす CARD です（`executer` は `human`）。Professor は、自分の子として起こす CARD を `sai-link task create` の `card` で作り、`sai-link run wait` で待ちます。古い CARD の `executer: agent` は読むだけで、`sai-link card run` は断ります。
- 子からの質問は親の `sai-link task wait` に `stalled`（`reason: "question"`）で届き、`sai-web question` / `answer` で答える。
- 完了は、子は `sai-link task report`、root（professor）は質問ツールで「完了してよいか」を確かめて `sai-link run finalize` に `report`。
- 研究日誌は `.sai/paper/professor-log.md`。operator の学びは `~/.sai/operator-notes.md`（`SAI_HOME` があればその中）。

#### 3.3.2 LabAgent（研究の進め方）と検証

root を育てる LabAgent と、S を作る `lab-sprout`、検証 agent があります。

| # | 場面 | ブロック | sci_type | sci_ops |
|---|---|---|---|---|
| #1 | 型指定 ＋ 論文あり | `lab-read` | node | `add_a, add_f, add_q` |
| #2 | 型指定 ＋ 検索（段 1 で検索し候補を報告 → 利用者が取り込む → 段 2 は `lab-read` の CARD を作る） | `lab-search` | node | `add_a, add_f, add_q` |
| #3 | 型未指定 ＋ 論文あり | `lab-grow` | node | `add_a, add_f, add_q, insert_q` |
| #4 | 型未指定 ＋ 検索 | `lab-survey` | node | `add_a, add_f, add_q` |
| #5 | 論文だけ（木を探して置き場所を決め、`lab-read` の CARD を作る） | `lab-place` | manage | `add_a, add_q` |
| #6 | 全体の進み具合の確認（数と穴を報告し、次の CARD を作る） | `lab-progress` | manage | なし（op を出さない） |
| #7 | review・survey 論文から root の骨組みを作る（`--node SEED` か軸の Q） | `lab-map` | manage | `add_q, add_a, add_f` |
| #8 | seed と path から研究計画（S）を作る（`--node D<n>`）。seed との矛盾と、最も近い先行研究の S との違いを報告する | `lab-sprout` | manage | `add_sprout` |
| 検証 | target_node と子の関係を評価し、評価の報告・op・次の CARD を作る（固定） | `sci-verify` | node | `set_stance, add_f, remove, move_a, insert_q, add_q` |

- READ_CARD（`runner: user`）の既定の block は `lab-read` です。
- `sci-verify` の参加者は `paper-debater`・`paper-reviewer` を子として使えます。
- `lab-sprout` は、seed の「決定」の項目と path が矛盾しないか、「空欄」の項目を path で埋めたかを軸ごとに確かめ、`add_sprout`（`seed` 付き）で S を作る前に、矛盾の表と先行研究の S との違いを報告します。新しさは agent の文で示します（画面での可視化は無い）。

#### 3.3.3 作業のブロック

| Action | ブロック | sci_type |
|---|---|---|
| 実験 | `sci-exp-sprout`（manage。既にある S から始める）・`sci-exp-branch`（support）・`sci-exp-review`（node）・`sci-result-compare`（manage・逆向き） | 上の欄 |
| 実験の support | `sci-exp-scout`（参照実装を探す。op を出さない）・`sci-sim-model`（論文 1 本のモデル定義書）・`sci-sim-compose`（つなぎのモデル定義書） | support |
| データ分析 | `sci-data-explore`（node）・`sci-data-test`（node） | node |
| その他 | `sci-start`（聞き取りから seed を作る）・`sci-restore`（逆向き）・`sci-typify`（manage）・`sci-digest`（manage・op を出さない）・`sci-coach`（node）・`sci-cite-check`（node・逆向き） | 上の欄 |
| support（呼ばれる側） | `sci-analogy`・`sci-extract`・`sci-lookup`・`paper-debater`・`paper-reviewer` | support |

- support agent は、ほかのブロックが呼ぶ agent。単独の Action としては動かしません。
- `sci-exp-branch` だけは、research-tree の範囲も共通の決まり（`sci-core`）も受け取りません（実験ループ。5.7 節）。
- 実験・データの block（`sci-exp-*`・`sci-data-*`・`sci-result-compare`）は、結論（F）を木に足しません。結果は MLflow と REPORT に残し、木には E の作成と `set_mlflow` だけを出します。`sci-exp-scout` は op を出さず、確かめた事実を参照コード調査書に書きます。
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
- **`mind.md`**（`sai-sci/mind.md`）— プロジェクトの覚え書きのひな形。`ORIGIN`（きっかけ）・`INTERESTS`（興味があること・無いこと）・`NOTES`（メモ）の 3 節。**利用者が書くファイルで、エージェントは直接書き換えません。** 研究ごとの制約（使うデータ・対象・成功基準・期間など）は mind.md ではなく seed（D）に書きます。既にあるプロジェクトの mind.md は書き換えません（`init` は無いときだけ書く）。

---

## 4. 始め方

### 4.1 新しいプロジェクト

Web の Research タブの research-tree で「始める」を押して型を選ぶか、次を打ちます。

```bash
sai-sci init --type <型>
```

`init` は、どの型でも同じ 6 つの軸の Q（top_Q）を作ります。`sai-sci/mind.md` のひな形（プロジェクトの覚え書き）を `mind.md` が無いときだけ書きます。

そのあと、研究ごとの制約を seed（D）に書きます（5.1 節）。

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

**手で足す設定はありません。** `init` だけで sci の設定が終わります。cc の質問フックの登録は 5.10 節の手順で行います（`sai-link` が cc の登録を書くこともあります）。

---

## 5. 使い方の流れ

### 5.1 木を育てる流れ

```
前向き:  init（6 軸の top_Q）→ seed（D）を作る（Web の Create SEED か sci-start）
         → root を育てる（lab-map・lab-grow・lab-search・lab-read・sci-verify）
         → lab-sprout で S を作る（seed との矛盾・先行研究との違いを報告）
         → sci-exp-sprout で実験（S の下に E）
逆向き:  sci-restore で論文の D・A・F・S を復元 → 引用の確かめ → 論文の S の下で再現 → 直した S → 比べる
```

画面（`#/research/tree`）での操作:

1. **seed を作る** — SEED の板か SPROUT の作成パネルの **Create SEED** で、題と本文（9 見出し）を書く。聞き取りながら作りたいときは、`sci-start` の CARD を動かす（`sci-start` は Q を作らず、seed だけを作る）。
2. **root を育てる** — 軸の Q の下に A（選択肢）を足す。review 論文があれば `lab-map` で骨組みを作り、Next Action の「Create CARD」から `lab-grow` を動かす。A の下に child_Q を作って種類を細かく分ける。板の「Q を足す」・「A を足す」で自分で足してもよい。A に出典（`#番号`）は書けない。
3. **F を付ける** — Library で論文を探し、READ_CARD（`#/research/read/<run_id>`）で 1 本ずつ読んで F を書く。エージェントが出した F は、`add_f` として自動で research-tree に入る。
4. **検証** — 子を持つ Q・A の Next Action に `sci-verify` が出る。評価の報告と op・次の CARD が作られる。
5. **S（研究計画）を作る** — `lab-sprout`（`--node D<n>`）か、Create SPROUT で seed と path を選ぶ。1 つの seed から複数の S を作れる。`lab-sprout` は、seed の「決定」と path の矛盾と、最も近い先行研究の S との違いを報告してから `add_sprout` を出す。
6. **E（実験）** — `sci-exp-sprout`（`--node S<n>`）が、既にある S から実験を始める（5.7 節）。実験の結論は木に足さない（E の `mlflow` で MLflow に結ぶだけ）。

### 5.2 Web の research-tree の画面

Research タブの最初の画面が research-tree（`#/research/tree`）。

- 上の帯: **Next Action**（先頭 3 件。`[NO_CHILD] Q24 lab-grow「…」` の形で、右に Create CARD）・折りたたみの **History**（全 Node の履歴を古い順に。`sai-sci status` の `history[]`）・注意。
- 左: Q・A・F・E・S（SPROUT）の一覧。F の行には stance の印（`S` support・`D` deny・`I` issue）が出る。**Filter** の欄に `*` だけを打つと、★ を付けた Node（とその祖先）だけに絞れる（★ は目印なので、どの種類の Node にも付けられる）。
- 真ん中: **DAG**（research-tree の図。下に ROOT（軸の Q）、次に SEED の欄（その横に seed（D）の札）、上に SPROUT、さらに上に BRANCH（E））。SEED の欄を押すと RESEARCH_TYPE と `mind.md` の節と、seed（D）の一覧が出る。Node を押すと `#/research/tree/Q12` のように住所が変わり、詳細が右の **SB**（サイドバー）に出る。
- SB: 見出し・親までの道・操作の釦・Next Action・F・History など。F の stance は SB から変えられる（`set_stance`）。SPROUT の SB には範囲の数（`範囲 Q 2・A 1・F S 1 / D 1 / I 0`）、E の SB には MLflow の参照（`mlflow`）が出て、押すと Experiments の画面（`#/research/experiments`）へ移る。何も選んでいないときは出ない。PC 幅（900px 以上）は右（約 420px・✕ で閉じる）、iPhone 幅は下から出る sheet。
- **Create SEED**: SEED の SB か SPROUT の作成パネルの釦を押すと、題と本文を書く欄が出る。作った seed は `D1` の札になり、SEED の SB の「SEED の一覧」に並ぶ。
- **Create SPROUT**: SEED の SB か上の帯の釦を押すと、History と DAG の間に作成パネルが出る。先に **SEED（D）を選ぶ**（必須。seed が無ければ Create SEED を案内する）。A の SB の **Add PATH** / **Delete PATH** で PATH（終点の A）を選ぶ。PATH は 1〜20 個で、同じ A は 2 回選べない。
- READ_CARD: `#/research/read/<run_id>`。

### 5.3 sai-link がブロックを動かすとき

`sci-*` と `lab-*` のブロックは `context: true` です。sai-link は実行の直前に `.sai/settings.json` の `context` の命令を実行し、標準出力を、最初のメッセージの `# context` の節に入れます。依頼文は同じメッセージの `# task_text` の節に入ります。

TREE の範囲はブロックの sci_type で決まります。`node` は対象の Node とその子孫、`manage` は SEED から対象までの祖先と対象とその子孫（対象が `SEED` なら research-tree の全体）、`support` には出しません。範囲を作れなかったときは 1 行の注意が `# context` の下に入ります（実行は止まりません）。

対象が S のときは、`## TREE` の直後に `## STUDY_SEED`（その S の親 D の ID・題・本文）が入ります。研究の制約なので、agent は必ず読みます。

task.md の frontmatter `sci_child` があれば、`# context` の対象の節に `- 足す子の型: A` の行を出します（空なら行を出さない）。

**決まり:** `.sai/settings.json` の `context` に別の命令を指定するときは、その出力も `##` から始めてください。

### 5.4 agent は op を出す（危険度と承認）

`sai-sci op` の標準入力は `{"ops": [ {"op": "add_a", "q": "Q3", "text": "…"}, … ]}`（1〜20 件）です。全部を 1 つのトランザクションで行い、1 件でも弾かれたら何も残しません。

op の危険度は 3 つに分かれます。

- **`Free`**（agent がそのまま出せる）: `add_a`・`add_f`・`add_e`・`add_seed`・`add_sprout`・`set_stance`・`set_mlflow`
- **`Approval`**（agent は承認が要る）: `add_q`・`insert_q`・`move_a`・`copy`・`remove`
- **`UserOnly`**（利用者だけ）: `star`・`unstar`・`edit`

| op | 危険度 | 規則 |
|---|---|---|
| `add_q` | Approval | 親は C1 の表のとおり。`guide` は GUIDE に無ければ拒否 |
| `insert_q` | Approval | `q` に生きた A が 1 つ以上要る。新しい Q を `q` の子にし、`q` の生きた A を全部その下へ移す（`ord` の順を保つ） |
| `move_a` | Approval | `from` ≠ `to`。`from` に生きた A が 1 つ以上要る。`to` は生きた child_Q を持たない。`to` が `from` の A の子孫なら拒否（輪になる） |
| `copy` | Approval | `to` は生きた child_Q を持たない。A と、その生きた F（stance ごと）を写す。A の下の Q と E は写さない |
| `remove` | Approval | Node を下の Node ごと消す。生きた S を持つ D は消せない（`… の下に SPROUT（S）があるので消せません。先に S を消してください`） |
| `add_a` | Free | `q` は生きた child_Q を持たない。同じ Q の生きた A と同じ文なら拒否。文と本文に出典（`#番号`）があれば拒否（`A の文と本文に出典（#番号）は書けません。出典は add_f の cites に書き、F として足してください`）。型の Q は増えない |
| `add_f` | Free | 3 文まで。数字を含む文は `cites` か `report` が要る。`cites` の形と存在は確かめる |
| `add_e` | Free | 親は S か E。`dataset`（任意・0〜200 字）を付けられる（sai-ts のスナップショットの参照。文字の形は検査しない） |
| `add_seed` | Free | `title`（1〜120 字）・`body`（0〜2000 字）。結果は `D<n>`。許可は要らない（agent は出す前に質問ツールで内容を確かめる） |
| `add_sprout` | Free | `seed`（D の ID）が必須（`seed には SEED（D）の ID を書いてください: …`）。同じ A を 2 回書けない |
| `set_stance` | Free | 値が同じなら何もしない |
| `set_mlflow` | Free | E の `mlflow` を変える（空文字は外す） |
| `set_dataset` | Free | E の `dataset` を変える（空文字は外す）。`node`（E の ID）と `dataset` が要る。値が変わったときだけ `rt_log` に残る |
| `star` / `unstar` | UserOnly | 値が変わるときだけ書く |
| `edit` | UserOnly | 字数の上限は Q 300・A/E/S/D 120・F 1200。F は 3 文まで。`body` は A・E・S・D だけ。D の文の直しは利用者だけ |

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
sai-sci card --node S<n> --block sci-exp-sprout   既にある S から実験を始める（対象は S）
        │
        ▼
 sprout ── S の研究計画と seed（STUDY_SEED）を読み、実験仕様書と最初の branch を
        │  1〜2 本起こして終わる（S は作らない。指標と成功基準は seed から読む）
        │  （sai-sci card --block sci-exp-branch → sai-link card run）
        ▼
 branch ── スクリプト 1 本を書いて実行先に投げる。始まりを確かめたら、
        │  ジョブ待ちの review の CARD を作って終わる（sai-link card wait --job）
        ▼  …ジョブが終わると daemon が起こす
 review ── 台帳で判定する。tune・debug → 同じ experiment の次の branch、
        │  model・ablation → 新しい experiment の次の branch、stop → 完了報告書
        └─▶ 次の branch …   （stop で終わる。起動元が professor なら、professor が `sai-link run wait` で見張って結果を受け取る）
```

- **MLflow の形**: experiment は `<project>/ID<sprout>/<model>`、run は `ID<branch>-p<NN>`。epoch ごとに `train_loss`・`val_loss` を記録する。
- **木への書き込み**: 実験の結果は E の `mlflow` で MLflow に結ぶだけです（`set_mlflow`）。E の下に F は置かず、結論も木に足しません。
- **データセット**: E の `dataset` は、sai-ts のスナップショットの参照（`ts:<name>@<sha12>`）です。sprout は S の上にある E の値を次の E の `dataset` に引き継ぎ、review は親の E の値を引き継ぎます（`set_dataset` で後から結ぶこともできます）。`MLflow` では experiment を作るときに `sai.dataset` タグとして残ります（既にある experiment には付きません）。branch は依頼書の「データセット」の行から、スナップショットのフォルダの `.f32` を読みます。スナップショットは workdir の外にあるので、実行先には送られません（遠隔の実行先で使うには、置き場を起動者に聞きます）。
  - 結び方の入口は sai-ts です（画面の「データセット」タブか `sai-ts snapshot link`）。sai-ts が `sai-sci apply` を呼んで、S なら新しい E、E なら `set_dataset` を出します。
- **止め方**: 判定が `stop` のときだけ。`max_branches` が branch の総数の上限になる。
- **最初の sprout の起動**は、`sai-sci card --node S<n>` で作る。S は先に `lab-sprout` か Create SPROUT で作っておく。`--input` は依頼文の末尾に載せる文書の waggle トークン（繰り返し可）。

```bash
sai-sci card --node S<n> --block sci-exp-sprout \
  --purpose "<この S の実験を始める>" \
  --input <参照コード調査書や復元書のトークン> --json
sai-link card run <返ってきた run_id>
```

- 詳しい決まりは `template/exp-core.md`。

**実行先**（スクリプトを動かす PC）は環境変数 `EXP_TARGET` で選ぶ。空なら `local`（この PC）。実行先は `<root>/.sai/exec/<名前>/` のフォルダで、利用者が個別に足せる（フォルダを置くだけ。増減で Rust や画面を直さない）。さくらの高火力 DOK は `dok/`（`README.md`・`SETUP.md` を先に読む。課金される）。記録は実行先から中央の MLflow に直接入る。

### 5.8 逆向き（既存の論文から始める）

前向き（5.1〜5.7）は研究の構想から research-tree を育てます。**逆向き**は、既存の論文（基盤論文 1〜5 本）を出発点にして、論文に書いてある研究方針を research-tree に復元し、その根拠を確かめ、論文の手法を再現し、新しい論文で直し、比べます。決まりは `template/bw-core.md` にまとめてあります。

| 段 | すること | ブロック |
|---|---|---|
| 1. 抽出 | 論文から seed（D。論文の研究の制約）・root に足りない A と F・論文の S（paths）を作る。結果の数値を記載結果表に、主張と引用の組を対応表にする | `sci-restore`（対象は `SEED`。D → A → F → S の順） |
| 2. 評価 | 主張ごとに引用文献を取り寄せて、根拠になっているかを判定する。そのうえで、復元した research-tree を検証する | `sci-cite-check` → `sci-verify` |
| 3. 再現 | 論文の S（既にある S）から、論文の手法を動かす | `sci-exp-sprout`（`--node S<n>`） |
| 4. 更新 | 基盤論文より新しい論文を探して A を直し、直した S（`lab-sprout`）を動かす | 前向きのブロック |
| 5. 比較 | 論文の記載値・再現・更新を比べる | `sci-result-compare` |

- **始め方**: 基盤論文を Library に入れ（Web の論文の画面）、research-tree を作ってから（`sai-sci init --update --type <型>`）、`professor-backward` の CARD を人が起こします。依頼文に `基盤論文: #12 #15` の形で番号を書きます。
- **復元した研究と自分の研究は、木の上で区別しません。** 論文の研究も D と S で表し、論文の事実は root の A の下の F に書きます（F の文は論文を主語にし、「本研究では」と書かない）。
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
   - 端末で答えたときは `--answered-by terminal`。
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

- **命令の場所**: 承認の記録の命令は、環境変数 `SAI_SCI_BIN` で場所を変えられます（無ければ `sai-sci`）。
- **確かめ方**: 一時プロジェクトで（5.4 節の op が必要な）質問に答えたあと、`sai-sci op` の `add_q` が通ること。通らないときは、`sai-sci status --json` の `warnings` と、`rt_approvals` の行を見ます。

### 5.11 見た番号の台帳（見ていない番号を引用しない）

agent が読んでいない論文の番号（`#37.5.2` など）を引用する作り話を止めるための仕組みです。MLflow や実験の台帳（`exp_summary.py`）とは関係がありません。

- **置き場**: `<root>/.sai/seen_ids/<run_id>`（拡張子なし。1 行に番号 1 つ。追記だけ）。自分の run の ID は環境変数 `SAI_PARENT_RUN_ID`。
- **書く人**: `sai-paper`・`sai-sci` は標準出力に出した `#x.y.z` を書きます。`sai-link` は run の開始時に最初の文（依頼文・埋め込んだ文書・`# context`）の番号を書き、`doc read` で読んだ本文の番号も足します。
- **照合する所**: `sai-sci op` の `add_f` の `cites`、`sai-sci report prepare` が拾う引用。台帳に無い番号があれば、何も書かず（`op` は終了コード 2、`report prepare` は終了コード 1 で保存しない）`台帳に無い番号です: #37.5.2（この run で CLI が出した番号だけを使えます）` の 1 行。節・塊（`#37.5`・`#37.5.2`）は完全一致、論文（`#37`）は `#37` か `#37.*` が台帳に 1 つでもあれば通ります。`sai-sci apply`（利用者の操作）は照合しません。
- **台帳が無い run**: `SAI_PARENT_RUN_ID` があるのに台帳のファイルが無い run は、何も見ていない run です。空の台帳として照合するので、番号を含む `cites` は止まります。照合しないのは、`SAI_PARENT_RUN_ID` が無いとき（手で打つとき）と、台帳が読めないとき（権限など）だけです。古い版の `sai-sci`・`sai-paper` は台帳に書かず、新しい run でも止まるので、3 つの実行ファイルを同じ版に作り直します。
- **集計と改善**: 使われた番号・使われなかった番号の数、結果のトークン数、失敗の分類は、`sai-link run history <run_id> --json` の `cli`（Web の run の History）に出ます。`kaizen` がそれを読んで改善計画書に書き、block・template の直しは別の作業で行います（kaizen 自身は block を直しません）。

---

## 6. 命令の一覧

### 6.1 `sai-sci …`

| 命令 | 何をするか |
|---|---|
| `sai-sci init --type <型> [--json]` | 研究プロジェクトの配る物を配り、SEED の下に 6 つの軸の Q（top_Q）を作る（型に依らず同じ） |
| `sai-sci types [--json]` | 使える RESEARCH_TYPE を一覧する |
| `sai-sci type-save` | RESEARCH_TYPE を `<sai_home>/sci/types/` に保存する（標準入力に型の Markdown） |
| `sai-sci status [--json] [--brief]` | 現況（counts・SEED・research-tree・Next Action）。`--brief` は `{"type","counts","next","running","warnings"}` を JSON で出す |
| `sai-sci node <Q12\|A5\|F3\|E2\|S2\|D1> [--json]` | Node 1 つの詳細 |
| `sai-sci context --run <run_id> [--tree]` | 今の実行の `# context` を標準出力へ出す |
| `sai-sci op [--run <run_id>] [--block <id>]` | agent が op を出す（標準入力は `{"ops":[…]}`）。許可が要る op は承認を使う（5.4・5.10 節） |
| `sai-sci approval record --qid <qid> --question <文> --answer <文> --answered-by <名前> [--run <id>] [--json]` | 質問の回答を承認として記録する。質問フックが呼ぶ（agent は呼ばない） |
| `sai-sci apply` | 利用者の操作を 1 つ反映する（標準入力に 1 件の JSON。承認なし） |
| `sai-sci card --node <Q12> --purpose <文> [--block <id>] [--child <Q\|A\|F>] [--expect <文>] [--mode <purpose_first\|blind_first>] [--papers <番号,…>] [--runner cc\|user] [--model opus\|sonnet\|haiku\|inherit] [--thinking none\|low\|medium\|high\|xhigh\|max] [--depth abstract\|figures\|full] [--origin-run <run_id>] [--input <トークン>]… [--json]` | Node に向けた Action（CARD）を作る。`--runner` は既定 `cc`、`--runner user` は READ_CARD。`--model`・`--thinking` は省くと settings.json の既定。`--child` は足す子の型 |
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
| research-tree（Q・A・F・E・S・D・SEED・検索の記録・承認の記録・rt_log） | `papr.db`（`rt_` で始まる表: `rt_meta`・`rt_nodes`・`rt_log`・`rt_approvals`・`rt_seeds`・`rt_searches`） |
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
- **形の決まり**: 型は `id`・`name` の frontmatter と `## Q` の節（`## SEED` の節は読まない）、ガイドは `key`・`name` の frontmatter と `## Q`・`## Child Q`・`## Extract`・`## Pitfall` の節が要ります。崩れていれば 1 行の warning として `status` の `warnings` に入ります。
- **block の形**: frontmatter の `sci_type`（node・manage・support）と `sci_ops`（op の一覧）。`sci_ops` を書かない block は `Free` の op 全部を出せます。`lab-progress` は `sci_ops` を書かず、本文で「op を出さない」と決めています（3.3.2 節）。
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
| SOIL（mind） | mind.md（プロジェクトの覚え書き。`ORIGIN`・`INTERESTS`・`NOTES`） |

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
- **既にある木（型の Q が「この研究は〜」の形）は移しません。** `init --update` でブロックとテンプレートを新しい物に置き換えると、古い木に新しいブロック（seed・軸の運用）が働きます。古い木の `guide_key` は `.sai/sci/guides/` の古い写しで読めます。移行の道具は無いので、新しい運用を使うときは新しいプロジェクトで `sai-sci init` します。
- 利用者全体の置き場（`~/.sai/` の `blocks/`・`template/`・`sci/types/`）に同じ名前の物があるときも、同じように移してから配ります。
- この手順は文書に書くだけで、開発の作業では実行しません。

---

## 9. Canvas から移った人へ

- **Canvas の画面と `sai-paper canvas` はありません。** 代わりに `#/research/tree`（research-tree）と `sai-sci` を使います。
- **今までの `.sai/paper/canvas/*.canvas` は消していません。** JSON Canvas 1.0 の形なので、他の道具（Obsidian など）で開けます。
- **`paper-map` は report だけを出すようになりました。** Canvas を作る手順は外れています。
- **古い paper-map の写し**（`<root>/.sai/blocks/paper-map.md`）には、無い命令が書いてあることがあります。8.2 節の手順で新しい写しにしてください。
