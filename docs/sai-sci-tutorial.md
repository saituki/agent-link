# research-tree（sai-sci）を手で確かめるチュートリアル

**読み手**: プログラムに慣れていない利用者。
**かかる時間**: 1 時間ほど（エージェントを動かす章を入れると、さらに 30 分ほど）。
**このあとできるようになること**: research-tree の機能をひと通り命令で動かし、出力の意味が分かる。承認（「許可する」）の流れが分かる。

このチュートリアルでは練習用のプロジェクトを 1 つ作り、次の流れを最初から最後まで体験します。

```
① 準備 → ② 練習用プロジェクトを作る → ③ 木の見方
→ ④ seed を作る → ⑤ A を足す → ⑥ F を付ける → ⑦ ★ を付ける
→ ⑧ SPROUT を作る → ⑨ 承認（許可する）を体験する → ⑩ 画面で確かめる
→ ⑪ 命令の一覧 → ⑫ 片付け
```

練習用のプロジェクトは本物の研究とは別の場所（`~/research/sci-practice`）に作ります。**練習で打つ命令は、すべてこの練習用のフォルダの中で行います。** 何を打っても本物の研究には影響しません。

> **出力の番号について**: ここに書いた出力の ID（`D7`・`A8`・`F10`・`S11` など）は、練習のときに出る番号に読み替えてください。番号は A・F・E・S・D をまとめて作った順に付くので、手元の出力と一致しないことがあります。この文書の通りに上から順に打てば、同じ番号になります。出力の形（キーの名前・終了コード）は同じです。

---

## 0. 先に覚えておく言葉

用語は [`sai-sci.md`](sai-sci.md) の用語表を見てください。今は全部を覚えなくて構いません。

| 言葉 | 意味 |
|---|---|
| 問い（SQ） | seed の問い。研究を実験の段階に進めるために確定させなければならない疑問点。root には問いを置かない |
| A | 選択肢。1 つの A は選択肢 1 つ。出典は書けない。6 つの軸（task・target・data・method・output・metric）は親の無い A で、その下に選択肢の A を並べる |
| F | 根拠。A の下に置く。`support`・`deny`・`issue` の立場を持つ |
| root | 分野の可能性の幅（軸の A・A・F）。研究の制約に縛られない |
| seed（D） | 1 つの研究の問いの一覧（雛形の 7 問。問いへの回答は固定）。複数持てる。ID は `D7` など |
| 木の全体 | `--node` を省いた対象。木の根の Node は無い（軸の A が最上段に並ぶ） |
| S（SPROUT） | 1 研究＝研究計画。1 つの seed の未回答の問いに回答していく。全部答えると完成し、実験を足せる |
| ★ | 利用者が付ける目印。計算には使わない |
| 承認 | 許可が要る操作を agent が行うための、利用者の「許可する」 |

### 命令（コマンド）の打ち方

このチュートリアルでは、次のような灰色の箱に命令が書いてあります。

```bash
echo こんにちは
```

1. ターミナルを開きます（Finder →「アプリケーション」→「ユーティリティ」→「ターミナル」）。
2. 箱の中の文字をコピーして、ターミナルに貼り付けます。
3. `return` キーを押します。

`#` から後ろは説明です。打たなくても構いません（貼り付けても害はありません）。

---

## 1. 準備

### 1.1 命令が使えるか確かめる

ターミナルで次を打ちます。

```bash
which sai-sci sai-paper sai-link
```

3 行とも `/Users/<あなたの名前>/.local/bin/…` のように出れば準備はできています。
`not found` と出たら、開発者に「`~/.local/bin` に sai の実行ファイルへのリンクを置いてほしい」と頼んでください。

### 1.2 Web の画面（apps/web）を起動する（画面を使う章のため）

ターミナルの窓を**新しく 1 つ**開きます（`⌘ + N`）。この窓は Web の画面を動かし続けるための窓なので、画面を使う間は閉じないでください。

```bash
cd ~/workdir/Sai/sai-tree
PATH=$HOME/workdir/Sai/sai-tree/target/release:$PATH \
  apps/web/.venv/bin/uvicorn sai_web.main:app \
  --host 127.0.0.1 --port 8200 --app-dir apps/web/src
```

`Uvicorn running on http://127.0.0.1:8200` と出たら起動できています。
ブラウザで **http://localhost:8200/** を開いてください。止めるときは、この窓で `control + C` を押します。

---

## 2. 練習用のプロジェクトを作る

### 2.1 フォルダを作って初期化する

もう 1 つターミナルの窓を開き（こちらは命令を打つ窓です）、次を打ちます。

```bash
mkdir -p ~/research/sci-practice
cd ~/research/sci-practice
sai-sci init --type problem
```

- `mkdir -p` はフォルダを作る命令です。
- `cd` はそのフォルダへ移る命令です。
- `sai-sci init --type problem` は、ここを「research-tree を使う研究プロジェクト」にする命令です。`--type problem` は RESEARCH_TYPE を PROBLEM_DRIVEN（課題駆動）にするという意味です。

RESEARCH_TYPE は 5 つあります（`sai-sci types` で一覧が出ます）。練習では `problem` を使います。**どの型でも、作られる軸は同じ 6 つです。** 軸は、親の無い A です。 型の違いは、雛形の問いのうち、どれが最初から seed で回答されているか、どれが未回答で始まるかだけです。

| `--type` の値 | 名前 | どんな研究か |
|---|---|---|
| `problem` | PROBLEM_DRIVEN（課題駆動） | 現場の「困った」から始まる（解く問題が決まっている） |
| `phenomenon` | PHENOMENON_DRIVEN（現象駆動） | 起きている現象の「なぜ」を問う（対象が決まっている） |
| `method` | METHOD_DRIVEN（手法駆動） | 使える手法が先にあり、使い道を探す（手法が決まっている） |
| `data` | DATA_DRIVEN（データ駆動） | 手元のデータで何が言えるかを問う（データが決まっている） |
| `improve` | IMPROVE_DRIVEN（改良駆動） | 今ある手法をよくする（元の手法と評価指標が決まっている） |

### 2.2 何が起きたか

終わると、たくさんの行が出ます。最後の方は次のようになります。

```
✅ 登録簿に登録しました: sci-practice
✅ 研究の assets を 89 件置きました（既にあるものはそのまま）
✅ 分野の観点の見本を 3 件置きました（既にあるものはそのまま）
✅ 分野の観点: general（.sai/template/domain.md）
✅ mind.md（土壌のひな形）を作りました
✅ テンプレートを 6 件置きました（既にあるものはそのまま）
✅ 研究プロジェクトを初期化しました: …/sci-practice/.sai/paper
✅ research-tree を作りました（RESEARCH_TYPE: PROBLEM_DRIVEN・軸 6）
```

次の 3 つが起きています。

1. フォルダの中に `.sai/` という隠しフォルダができ、論文の置き場（`.sai/paper/papr.db`）と research-tree（その下に並ぶ 6 つの軸の A。親の無い A）が作られた。
2. エージェント（cc）がこのプロジェクトで使う設定（`.mcp.json`・`.claude/`）が置かれた。
3. プロジェクトが登録簿（`~/.sai/registry.json`）に `sci-practice` という名前で載った。**これで Web の画面にこのプロジェクトが出ます。**

> 隠しフォルダは Finder では見えません。見たいときは Finder で `⌘ + shift + .`（ピリオド）を押します。
> `.mcp.json`・`.claude/` の行には「git commit してください」と出ることがあります。練習では無視して構いません。

---

## 3. 木の見方

### 3.1 RESEARCH_TYPE と様子

```bash
sai-sci types
```

```
problem PROBLEM_DRIVEN
phenomenon PHENOMENON_DRIVEN
method METHOD_DRIVEN
data DATA_DRIVEN
improve IMPROVE_DRIVEN
```

```bash
sai-sci status --brief
```

`--brief` は数と次にやることだけを JSON で出します。先頭だけ見ると次のとおりです（`…` は省略）。

```
{"counts":{"a":0,"axis":6,"e":0,"f_deny":0,"f_issue":0,"f_support":0,"s":0},"next":[{"block":"lab-grow","expect":"子を 1 つ以上","kind":"card","node":"A1","option":null,"purpose":"「解く問題」の子の A を作る","why":"NO_CHILD"},…],"running":[],"type":"problem","warnings":[…]}
```

- `counts` は生きた Node の数です。`axis` は軸の A、`a` は軸でない A、`f_support`・`f_deny`・`f_issue` は F の立場ごと、`e` は E、`s` は SPROUT の数です。
- `next` は Next Action です。`why` が理由（`NO_CHILD`・`NO_FINDING`・`UNVERIFIED`）で、`block` が頼む仕事の種類です。
- `warnings` は、research-tree の決まりに合わない所です。出ていても練習には差し支えありません。

### 3.2 Node を 1 つ読む

```bash
sai-sci node A4
```

A4 の文・親までの道・GUIDE・Next Action が人が読む形で出ます。

```
A4 手法

PATH: A4

GUIDE METHOD: 分野で使われている手法の種類を並べる軸の A である。root の一部で、seed に縛られない。…

Next Action:
  1. A4 lab-grow 「手法」の子の A を作る（NO_CHILD）
```

JSON で読むときは `--json` を付けます（`sai-sci node A4 --json`）。

- 作られた 6 つの A（`A1`〜`A6`）は、**軸の A** です。順に task（解く問題）・target（対象）・data（データ）・method（手法）・output（出力）・metric（評価指標）で、どの型でも同じです。軸の A の文は名詞で、`手法` のように書きます。
- **選択肢の A は、軸の A の下に並べます。** 「この研究では」のような、この研究だけの決まりは A には書きません。それは次の章の seed の回答に書きます。
- **A の下にも A を足せます。** 細かい種類は、A の下の A として足します（`rel` は省くと `subtype`）。`rel` の決まりは [`sai-sci.md`](sai-sci.md) の 2.1 節です。

---

## 4. seed を作る（利用者の操作）

root（軸の A）は分野の可能性の幅で、この研究の制約は入りません。**この研究の決まりは seed（D）に書きます。** seed は 1 つの研究の問いの一覧で、研究ごとに 1 つ作ります（複数持てます）。

seed には、雛形の 7 問が最初から入ります。問いには、利用者が決まっていることを**回答**します。回答は、テキストか root の A（選択肢）のどちらか一方です。

| 問い（`SQ<n>`） | 軸 |
|---|---|
| 解く問題は何か | `task` |
| 対象は誰・何か | `target` |
| どのデータを使うか | `data` |
| どの手法を使うか | `method` |
| 何を出力するか | `output` |
| どの指標で評価するか | `metric` |
| 成功基準は何か（指標の値と向き） | 無し |

- 回答を seed に付けると **固定**（初期制約）になります。その seed の全部の sprout に効きます。
- 回答が無い問いは **未回答** のままです。未回答の問いに回答していくと、実験を始められる S（SPROUT）になります（8 章）。
- 形の一覧は [`sai-sci.md`](sai-sci.md) の 2.1.1 節です。

### 4.1 seed を作る

本文（body）は、背景と制約の自由な文です。本文に改行が入るので、`<<'EOF'` で JSON を渡します（`echo` は使わない）。

```bash
sai-sci apply --json <<'EOF'
{"op":"add_seed","title":"自分の研究","body":"## 背景\nてんかん発作の予知を臨床で使いたい\n## 制約\n半年・GPU 1 枚"}
EOF
```

```
{"op":"add_seed","result":"D7"}
```

`result` の `D7` が seed の ID です（頭文字 `D`）。問いを見ます。`node` の `--json` の `node.questions` に、雛形の 7 問が出ます。

```bash
sai-sci node D7 --json | python3 -c 'import json,sys; [print(q["id"], q["text"], q["axis"]) for q in json.load(sys.stdin)["node"]["questions"]]'
```

```
SQ1 解く問題は何か task
SQ2 対象は誰・何か target
SQ3 どのデータを使うか data
SQ4 どの手法を使うか method
SQ5 何を出力するか output
SQ6 どの指標で評価するか metric
SQ7 成功基準は何か（指標の値と向き） None
```

`None` は、成功基準の問いに軸が無いことを表します（JSON の `null`）。

### 4.2 決まっていることを回答する

決まっていることは、問いに回答します。この練習では、解く問題・対象・データを決めました。

```bash
sai-sci apply --json <<'EOF'
{"op":"add_answer","to":"D7","q":"SQ1","text":"発作の予知"}
EOF
sai-sci apply --json <<'EOF'
{"op":"add_answer","to":"D7","q":"SQ2","text":"側頭葉てんかん患者"}
EOF
sai-sci apply --json <<'EOF'
{"op":"add_answer","to":"D7","q":"SQ3","text":"術前の EEG"}
EOF
```

```
{"op":"add_answer","result":"SA1"}
{"op":"add_answer","result":"SA2"}
{"op":"add_answer","result":"SA3"}
```

- `to` は seed の ID（`D7`）、`q` は問いの ID（`SQ1`）です。`text` の代わりに `a`（root の A の ID。例 `"a":"A8"`）でも回答できます。どちらか一方だけを書きます。
- 手法・出力・評価指標・成功基準（`SQ4`〜`SQ7`）は、未回答のままにしました。この未回答は、root から選択肢を選んで回答していきます（8 章）。
- seed は親を持ちません。S の親として並びます。`sai-sci status --json` の `nodes` に種類 `D` で出ます。
- 初めての人は、聞き取りながら seed を作る `sci-start`（CARD）を使えます。`sci-start` は軸の A を作らず、seed だけを作ります。

---

## 5. A を足す（利用者の操作）

軸の A（A4）の下に、選択肢の A を足します。手法の軸（A4）に「こういう手法がある」を並べます。利用者は `sai-sci apply` で 1 件ずつ足せます。標準入力は op 1 件の JSON です。

```bash
echo '{"op":"add_a","parent":"A4","text":"CNN で発作の前兆を分類する手法がある"}' | sai-sci apply --json
```

```
{"op":"add_a","result":"A8"}
```

`result` が新しい Node の ID です（ここでは `A8`）。以後の手順では、この番号を使います。

- `apply` は利用者の命令なので、承認は要りません。
- 同じ文の A を同じ親に 2 つ足すと、「A4 に同じ文の A があります」と断られます（終了コード 2）。
- 1 つの A は選択肢 1 つです。複数の選択肢を 1 つの A に混ぜないでください。

### 5.1 A に出典は書けない

A の文と本文に出典（`#37` のような論文の番号）は書けません。出典は、A の下の F に書きます。

```bash
echo '{"op":"add_a","parent":"A4","text":"精度は #37 による"}' | sai-sci apply --json
```

```
A の文と本文に出典（#番号）は書けません。出典は add_f の cites に書き、F として足してください
```

（終了コードは 2 です。何も書かれません。）

### 5.2 A が多すぎるときは組み直す（TOO_FLAT・move_node）

直下の A が 7 件を超えると、Next Action に `TOO_FLAT`（block `lab-review`）が出ます。木は拒まれません。組み直すときは、まとまりの親になる A を足し、兄弟の A を `move_node` でその下へ移します。練習では、「学習で重みを決める手法」の A（`A9`）を足して、A8 を移します。

```bash
echo '{"op":"add_a","parent":"A4","text":"学習で重みを決める手法"}' | sai-sci apply --json
```

```
{"op":"add_a","result":"A9"}
```

```bash
echo '{"op":"move_node","node":"A8","to":"A9"}' | sai-sci apply --json
```

```
{"op":"move_node","result":"A8"}
```

- `result` は移した A です。A8 の子（F や子の A）は、ID を保ったまま一緒に移ります。`move_node` は承認が要りません。
- 同じ親へもう一度移すと「A8 は既に A9 の下にあります」と断られます（終了コード 2）。
- 上下を逆にして A9 を A8 の下へ移すと、「A8 は A9 の下にあるので移せません」と断られます（終了コード 2）。

---

## 6. F を付ける（agent の操作）

agent は、`sai-sci op` で木を変えます。標準入力は `{"ops": [...]}` の形で、1〜20 件の op を入れられます。**全部が通ったときだけ書かれ、1 件でも弾かれると何も書かれません。**

```bash
echo '{"ops":[{"op":"add_f","parent":"A8","text":"前兆の分類精度は高い","stance":"support"}]}' | sai-sci op
```

```
{"approval":null,"results":[{"op":"add_f","result":"F10"}]}
```

- `approval` は、承認を使ったときだけ値が入ります（ここでは `null`）。
- `stance` は F の立場です（`support`・`deny`・`issue`）。

### 6.1 数字を含む F には出典が要る

数字を含む F は、出典（`cites`）か REPORT のトークン（`report`）が要ります。出典が無いと断られます。

```bash
echo '{"ops":[{"op":"add_f","parent":"A8","text":"精度は 95 %","stance":"deny"}]}' | sai-sci op
```

```
1 件目（add_f）: 数字を含む F には出典（cites）か REPORT のトークン（report）を付けてください
```

（終了コードは 2 です。`echo $?` で確かめられます。）

---

## 7. ★ を付ける（利用者だけ）

★ は利用者だけが付けられる目印です。agent は ★ を付けられません（`star` は利用者だけが行う op で、agent の op には無い）。

```bash
echo '{"op":"star","node":"A8"}' | sai-sci apply --json
```

```
{"op":"star","result":"A8"}
```

- ★ は計算に使いません。目印として画面に出ます。
- 外すときは `{"op":"unstar","node":"A8"}` を `apply` に渡します。

---

## 8. SPROUT を作る

SPROUT（S）は、**1 つの seed（D）の未回答の問いに回答していく研究計画**です。答える問いの全部に回答が付くと**完成**し、実験（E）を足せます。`seed` が必須です。回答は `answers` に 0〜40 件まで付けられます。seed（D）で回答済みの問いには、S から答えられません。

ここで使う ID は、この章までの手順で決まります。5 章で足した A は `A8`（手法の選択肢）、4 章の seed は `D7` です。S の番号は、その後の手順で決まります。

### 8.1 seed を付けないと断られる

```bash
sai-sci apply --json <<'EOF'
{"op":"add_sprout","title":"予知の SPROUT"}
EOF
```

```
seed が必要です
```

（終了コードは 2 です。）

### 8.2 paths は廃止した

以前の書き方（`paths`）は断られます。回答（`answers`）を使います。

```bash
sai-sci apply --json <<'EOF'
{"op":"add_sprout","seed":"D7","title":"予知の SPROUT","paths":["A8"]}
EOF
```

```
paths は廃止しました。answers を使ってください（例: {"q": "SQ3", "a": "A12"}）
```

### 8.3 回答を付けて S を作る

未回答の問い `SQ4`（どの手法を使うか）に、5 章の A（`A8`）で回答して S を作ります。

```bash
sai-sci apply --json <<'EOF'
{"op":"add_sprout","seed":"D7","title":"予知の SPROUT","answers":[{"q":"SQ4","a":"A8"}]}
EOF
```

```
{"op":"add_sprout","range":{"a":2,"f_deny":0,"f_issue":0,"f_support":1},"result":"S11"}
```

- `result` の `S11` が SPROUT の ID です。`range` は SPROUT の範囲です。回答の A の祖先の A（軸の A は数えない）と、A の子の F を数えます（`a` は A の数、`f_*` は F の立場ごとの数）。今の例では、A8 と祖先の A9、A8 の子の F10 を数えます（軸の A4 は数えない）。
- `answers` の 1 件は `{"q":"SQ<n>","a":"A<n>"}`（path の回答）か `{"q":"SQ<n>","text":"…"}`（テキストの回答）です。
- 1 つの seed から、別の回答をした S を何本でも作れます（8.6 で作ります）。

### 8.4 完成を見る

S の中身を見ます。seed の回答は「seed（固定）」、S の回答は「新規性（sprout の回答）」、まだ答えていない問いは「未回答」に出ます。

```bash
sai-sci node S11
```

```
S11 予知の SPROUT
## seed（固定）  D7 自分の研究
- SQ1 解く問題は何か: 「発作の予知」
- SQ2 対象は誰・何か: 「側頭葉てんかん患者」
- SQ3 どのデータを使うか: 「術前の EEG」
## 新規性（sprout の回答）
- SQ4 どの手法を使うか: A8 CNN で発作の前兆を分類する手法がある（A4 › A9 › A8）
## 未回答
- SQ5 何を出力するか
- SQ6 どの指標で評価するか
- SQ7 成功基準は何か（指標の値と向き）

PATH: D7 > S11

DECISIONS:
  <日時> edit user add_answer SQ4 A8
…
```

完成しているかは、`--json` の `complete` と `missing` で見ます（この 2 つは画面の text には出ません）。

```bash
sai-sci node S11 --json | python3 -c 'import json,sys; n=json.load(sys.stdin)["node"]; print(n["complete"], n["missing"])'
```

```
False ['SQ5', 'SQ6', 'SQ7']
```

未完成の S には、実験（E）を足せません。試すと断られます。

```bash
sai-sci apply --json <<'EOF'
{"op":"add_e","parent":"S11","text":"試験 1"}
EOF
```

```
S11 は未完成です（未回答: SQ5, SQ6, SQ7）。全部の問いに回答してから実験を足してください
```

### 8.5 残りの問いに回答して完成させる

残りの 3 問に回答します。`op` は 1 回で複数を出せます（1〜20 件。1 件でも弾かれると何も書かれません）。

```bash
sai-sci op <<'EOF'
{"ops":[
  {"op":"add_answer","to":"S11","q":"SQ5","text":"発作起始の位置"},
  {"op":"add_answer","to":"S11","q":"SQ6","text":"AUC"},
  {"op":"add_answer","to":"S11","q":"SQ7","text":"AUC 0.8 以上"}
]}
EOF
```

```
{"approval":null,"results":[{"op":"add_answer","result":"SA5"},{"op":"add_answer","result":"SA6"},{"op":"add_answer","result":"SA7"}]}
```

```bash
sai-sci node S11 --json | python3 -c 'import json,sys; n=json.load(sys.stdin)["node"]; print(n["complete"], n["missing"])'
```

```
True []
```

完成したので、S11 は凍結されます。完成した時点の問いの一覧を保ちます。後から D7 に問いを足しても、S11 には効きません。

- 同じ問いに、seed（D）で回答済みの問い（`SQ1`〜`SQ3`）を S から答えると断られます（`SQ1 は seed で回答済みです`）。
- 実験の回し方は [`sai-sci.md`](sai-sci.md) の 5.7 節です。`sci-exp-sprout`（`--node S<n>`）が完成した S から実験を始めます。練習では実験を回さず、SPROUT を作るまでにしてください（実行先によっては課金されます）。

### 8.6 別の回答で S を作って比べる

同じ seed から、別の手法（テキストの回答）で S を作ります。

```bash
sai-sci apply --json <<'EOF'
{"op":"add_sprout","seed":"D7","title":"別の SPROUT","answers":[{"q":"SQ4","text":"発作の前兆を Transformer で予測する手法"}]}
EOF
```

```
{"op":"add_sprout","range":{"a":0,"f_deny":0,"f_issue":0,"f_support":0},"result":"S12"}
```

2 つの S を並べて比べるには、`compare` を使います（2〜10 個）。

```bash
sai-sci compare S11 S12
```

```
| S | task | target | data | method* | output* | metric* |
|---|---|---|---|---|---|---|
| S11 予知の SPROUT | 発作の予知 [固定] | 側頭葉てんかん患者 [固定] | 術前の EEG [固定] | A8 CNN で発作の前兆を分類する手法がある [新規] | 発作起始の位置 [新規] | AUC [新規] |
| S12 別の SPROUT | 発作の予知 [固定] | 側頭葉てんかん患者 [固定] | 術前の EEG [固定] | 発作の前兆を Transformer で予測する手法 [新規] | （空欄） | （空欄） |
```

- 行が S、列が軸の付いた問い（6 軸）の表です。欄には、その問いの回答を書きます。`[固定]` は seed の回答、`[新規]` は sprout の回答です。`（空欄）` は未回答です。
- 列名の `*` は、S どうしで回答が違う列です。`--json` も使えます。
- Web では、`#/research/compare?s=S11,S12` の画面で同じ表が出ます。

### 8.7 seed は、S がある間は消せない

S の親になっている seed（D）は、消せません。先に S を消します。

```bash
sai-sci apply --json <<'EOF'
{"op":"remove","node":"D7"}
EOF
```

```
D7 の下に SPROUT（S）があるので消せません。先に S を消してください
```

- 実際の研究では、S は `lab-sprout`（`--node D<n>`）の agent が作ります。`lab-sprout` は、seed の回答と矛盾しないか、先行研究の S と何が違うかを報告してから、未回答の問いに回答を書きます。root の骨組みを review 論文から作るのが `lab-map` です。

---

## 9. 承認（「許可する」）を体験する

agent が seed の問いを足す（`add_seed_q`）ときは、利用者の承認が要ります。承認が要る op は `copy`・`remove`・`add_seed_q`・`edit_seed_q`・`remove_seed_q`・`remove_answer` です（[`sai-sci.md`](sai-sci.md) の 5.4 節）。承認の流れを体験します。ここでは、聞き取りの block `sci-start` の run で、D7 に問いを 1 つ足します。

### 9.1 承認が無いと入らない

agent の実行（run）の番号は `--run` で渡すので、まず run を作ります。

```bash
sai-sci card --purpose "D7 の問いを聞き取って足す" --block sci-start --json
```

```
{"path":"…/sci-practice/.sai/outputs/todo/1/task.md","run_id":1}
```

- `run_id`（ここでは `1`）は、このあとの手順で使います。
- `--block sci-start` は、この CARD を動かす仕事の種類です。木の全体を対象にする block なので、`--node` は付けません。CARD を実際に動かすのは、利用者が画面（**プロジェクト** タブ → Dashboard）から **▶ 実行** を押すか、`sai-link card run <run_id>` を打ちます。練習では実行せず、承認の体験だけをします。

承認が無い状態で `add_seed_q` を出すと、次のように断られます。

```bash
echo '{"ops":[{"op":"add_seed_q","seed":"D7","text":"手法は患者ごとに調整が要るか","axis":"method"}]}' | sai-sci op --run 1 --block sci-start
```

```
許可がありません: 先に質問ツールで「許可する」を選んでもらってください（この run に、まだ使っていない「許可する」の記録がありません。質問の答えの記録に失敗した可能性もあります。同じ質問をもう一度出してください）
```

（終了コードは 4 です。何も書かれていません。）

### 9.2 「許可する」を記録する

agent が質問ツールで「許可する」を選んでもらうと、質問フックがこの記録を自動で行います。練習では、同じ記録を手で打ちます。

```bash
sai-sci approval record --qid demo1 --question "次の op を許可しますか: add_seed_q（D7 に「手法は患者ごとに調整が要るか」）" --answer "許可する" --answered-by user --run 1 --json
```

```
{"approval":{"consumable":true,"id":1,"qid":"demo1","run_id":1}}
```

- `--answered-by` は答えた人の名前です。画面で答えたときは、ログインした人の名前が入ります。
- `consumable: true` は、回答が「許可する」だったという意味です。

### 9.3 許可を使って問いを足す

もう一度同じ `add_seed_q` を出すと、今度は入ります。許可は 1 件だけ使えて、使うと消えます。

```bash
echo '{"ops":[{"op":"add_seed_q","seed":"D7","text":"手法は患者ごとに調整が要るか","axis":"method"}]}' | sai-sci op --run 1 --block sci-start
```

```
{"approval":{"answered_by":"user","qid":"demo1"},"results":[{"op":"add_seed_q","result":"SQ8"}]}
```

- `approval` に、使った許可の質問の ID と、答えた人が出ます。rt_log の記録にも同じ内容が残ります。
- もう一度同じ命令を打つと、許可が消えているので「許可がありません」と出ます（終了コード 4）。
- 同じ `--qid` で `approval record` をもう一度打つと、「この質問の承認は記録済みです: demo1」と出ます（終了コード 4）。
- 命令の `--block` の `sci_ops` に無い op は、許可があっても「このブロックでは add_seed_q を出せません」と断られます（終了コード 2）。

> 利用者が `sai-sci apply` で行う操作には、承認は要りません。承認が要るのは agent の `sai-sci op` の許可が要る op だけです。

---

## 10. 画面で確かめる

Web の画面（1.2 節の窓を起動したまま）で、research-tree を確かめます。

1. ブラウザで **研究** タブを押し、先頭の **research-tree** を押します（`#/research/tree`）。
2. 上の帯に **Next Action**（先頭 3 件）が出ます。ここに、`sai-sci status --brief` の `next` と同じ内容が出ます。
3. 左の一覧で Node を押すと、右に詳細が出ます。`A8` を押すと、★ と F（`F10`）の札が見えます。
4. 真ん中の図（DAG）で、seed（D）の札 `D7` と、その上に SPROUT（`S11`・`S12`）が出ます。
5. 木の全体の板で「seed（D）の一覧」を開き、`D7` を押すと seed の本文が読めます。
6. 板の **Create seed（D）** を押すと、題と本文を書く欄が出ます。ここから画面で seed を作れます（練習ではやらなくて構いません）。
7. **Create SPROUT** を押すと、作成パネルに **seed（D）を選ぶ**（必須）欄が出ます。seed を選び、`A` の板の **Add PATH** で PATH を選んでから作ります。seed が 1 つも無いときは、先に Create seed（D）を促されます。
8. 命令で足した A・F は、画面を読み直すと出ます。

> 画面の表示は、命令の結果と同じ research-tree を読んでいます。画面で見えない物は、命令の出力も確かめてください。

---

## 11. 同じことを命令（CLI）で行う

画面でできることは、すべて命令でもできます。エージェントもこの命令を使っています。**練習用のフォルダで行ってください。**

```bash
cd ~/research/sci-practice
```

| したいこと | 命令 |
|---|---|
| 使える RESEARCH_TYPE を見る | `sai-sci types` |
| research-tree の数と次にやることを見る | `sai-sci status --brief` |
| research-tree の全体を JSON で見る | `sai-sci status --json` |
| Node の中身を見る | `sai-sci node A8 --json`（seed は `sai-sci node D7`） |
| エージェントに見せる範囲を見る | `sai-sci context --run 1` |
| 書き出しを作り直す | `sai-sci export` |
| 命令の一覧を見る | `sai-sci schema --json` |

### 11.1 文脈を見る

`sai-sci context --run 1` は、その run の agent に渡す research-tree の範囲と、使える op の一覧です。ここは `sci-start` の run（木の全体）の例です。見出しは次のとおりです（一部）。

```
## PROJECT
- 型: PROBLEM_DRIVEN
- 対象: 木の全体 ← TARGET

## TREE
- A1 解く問題 [A 0・深さ 1]
- A2 対象 [A 0・深さ 1]
- A3 データ [A 0・深さ 1]
- A4 手法 [A 1・深さ 1]
  - A9 学習で重みを決める手法 [A 1・F 1]
    - A8 ★ CNN で発作の前兆を分類する手法がある [F 1]
      - F10 [support] 前兆の分類精度は高い
- A5 出力 [A 0・深さ 1]
- A6 評価指標 [A 0・深さ 1]
- D7 自分の研究
  - S11 予知の SPROUT [固定: task=「発作の予知」, target=「側頭葉てんかん患者」, data=「術前の EEG」 / 新規: method=A8, output=「発作起始の位置」, metric=「AUC」, SQ7=「AUC 0.8 以上」] [範囲: A 2・F 1（S 1 / D 0 / I 0）]
  - S12 別の SPROUT [固定: task=「発作の予知」, target=「側頭葉てんかん患者」, data=「術前の EEG」 / 新規: method=「発作の前兆を Transformer で予測する手法」 / 未回答: 4] [範囲: A 0・F 0（S 0 / D 0 / I 0）]

## ALLOWED_OPS
- add_seed: seed（D）を足す（研究の背景・制約の文）。問いは雛形の 7 問が最初から入る（消せる）。
  必須: title（1〜120 字）
  任意: body（0〜2000 字）
- add_seed_q（承認が要る）: seed（D）に問いを足す。未完成の SPROUT にだけ効く（完成した SPROUT は凍結）。
  必須: seed（seed（D）の ID）, text（1〜300 字）
  任意: axis（task / target / data / method / output / metric / ）
- add_answer: 問いに回答する。to が seed（D）なら固定の回答、SPROUT（S）なら新規の回答。a（A の ID）か text（文）のどちらか一方。
  必須: to（Node の ID）, q（問いの ID（SQ<n>））
  任意: a（Node の ID）, text（1〜300 字）
出し方: sai-sci op（標準入力に {"ops": [...]}、1 回は 20 件まで）。1 件でも違反があれば何も入りません。
```

- `## TREE` の `[A 1・深さ 1]` は、直下の生きた A の数と、軸の A からの段です。`## PROJECT` は木の全体のときだけ出ます。
- `## ALLOWED_OPS` は、この run の block が出せる op です（`sci_ops` の中の op）。

対象が **S（SPROUT）** の run では、`## TREE` の直後に **`## STUDY_SEED`**（その S の seed の本文）が出ます。`sci-exp-sprout` の CARD を作って確かめます（S11 は 8 章で作りました）。

```bash
sai-sci card --node S11 --block sci-exp-sprout --purpose "S11 の実験を始める" --json
sai-sci context --run 2
```

```
## TREE
- D7 自分の研究
  - S11 予知の SPROUT [固定: task=「発作の予知」, target=「側頭葉てんかん患者」, data=「術前の EEG」 / 新規: method=A8, output=「発作起始の位置」, metric=「AUC」, SQ7=「AUC 0.8 以上」] [範囲: A 2・F 1（S 1 / D 0 / I 0）] ← TARGET
  [新規] PATH: A4 手法 > A9 学習で重みを決める手法 > A8 CNN で発作の前兆を分類する手法がある

## STUDY_SEED
D7 自分の研究
## 背景
てんかん発作の予知を臨床で使いたい
## 制約
半年・GPU 1 枚

| 問い | 軸 | seed の回答 | この S の回答 |
|---|---|---|---|
| SQ1 解く問題は何か | task | 発作の予知 |  |
| SQ2 対象は誰・何か | target | 側頭葉てんかん患者 |  |
| SQ3 どのデータを使うか | data | 術前の EEG |  |
| SQ4 どの手法を使うか | method |  | A8 CNN で発作の前兆を分類する手法がある |
| SQ5 何を出力するか | output |  | 発作起始の位置 |
| SQ6 どの指標で評価するか | metric |  | AUC |
| SQ7 成功基準は何か（指標の値と向き） | - |  | AUC 0.8 以上 |

## ALLOWED_OPS
- add_e: SPROUT か E の下に E（実験）を足す。SPROUT は完成しているときだけ。
  必須: parent（A の ID）, text（1〜120 字）
  任意: body（0〜2000 字）, mlflow（0〜200 字）, dataset（0〜200 字）
- set_mlflow: E の MLflow の結び付けを変える（空文字で外す）。
  必須: node（Node の ID）, mlflow（0〜200 字）
- set_dataset: E のデータセットの参照（sai-ts の ts:<name>@<sha12> など）を変える（空文字で外す）。
  必須: node（Node の ID）, dataset（0〜200 字）
出し方: sai-sci op（標準入力に {"ops": [...]}、1 回は 20 件まで）。1 件でも違反があれば何も入りません。
```

- `## STUDY_SEED` は研究の制約です。agent は必ず読み、実験の指標と成功基準もここから読みます。
- `## TARGET` の `足す子の型` は、CARD を作るときに `--child` で決めた子の型です。軸の A の下は `A` だけです。

### 11.2 書き出されるファイル

| ファイル | 中身 | 手で直すか |
|---|---|---|
| `.sai/paper/state.md` | 現況（段階・今の作業・Next Action） | 直さない（次の書き出しで上書きされる） |
| `.sai/paper/tree.md` | research-tree の全体を文章にしたもの | 直さない |
| `.sai/paper/mind.md` | プロジェクトの覚え書き（ORIGIN・INTERESTS・NOTES。あなたが書く） | **あなたが書く**。エージェントは書き換えない。研究ごとの制約は mind.md ではなく seed（D）に書く |

書き込む命令（`apply`・`op`・`card`・`init`）は、終わる前に `state.md` と `tree.md` を書き出します。

---

## 12. 片付け

練習が終わったら片付けます。**消す前に、消すものの名前が `sci-practice` であることを必ず確かめてください。**

1. Web の画面を動かしている窓で `control + C` を押します。
2. 登録簿から `sci-practice` を外します。次の命令は、登録簿の写し（`registry.json.bak`）を取ってから、`sci-practice` の行だけを外します。

```bash
cp ~/.sai/registry.json ~/.sai/registry.json.bak
python3 -c '
import json, pathlib
p = pathlib.Path.home() / ".sai" / "registry.json"
rows = json.loads(p.read_text())
p.write_text(json.dumps([r for r in rows if r.get("name") != "sci-practice"], ensure_ascii=False, indent=2))
print("外しました")'
```

3. 練習用のフォルダを消します（ゴミ箱を通らずに消えます。名前を確かめてから打ってください）。

```bash
rm -rf ~/research/sci-practice
```

---

## 困ったとき

| 起きたこと | 直し方 |
|---|---|
| Web にプロジェクトが出ない | 📁 を押して選び直す。無ければ `~/research/sci-practice` で `sai-sci init --type problem` をもう一度打つ（何度打っても壊れない） |
| 画面が古いまま | ブラウザで再読み込み（`⌘ + R`） |
| 「A の文と本文に出典（#番号）は書けません」 | 出典を A から外し、A の下に F として足す（5.1・6 章） |
| 「seed が必要です」 | `add_sprout` に `"seed":"D7"` のように seed の ID を付ける（8 章）。seed が無ければ先に作る（4 章） |
| 「paths は廃止しました。answers を使ってください」 | `paths` を書かず、`"answers":[{"q":"SQ4","a":"A8"}]` の形で回答を付ける（8.2 節） |
| 「S11 は未完成です（未回答: SQ5, SQ6, SQ7）」 | 残りの問いに回答してから、実験を足す（8.4・8.5 節） |
| 「SQ1 は seed で回答済みです」 | seed（D）で回答した問いには、S から答えない（8.5 節） |
| 「D7 の下に SPROUT（S）があるので消せません」 | 先に S を消す（8 章） |
| 「数字を含む F には出典（cites）か REPORT のトークン（report）を付けてください」 | 文を直すか、出典を付ける（6.1 節） |
| 「許可がありません: 先に質問ツールで「許可する」を選んでもらってください」（終了コード 4） | 承認を先に取る（9.2 節）。承認は 1 回の op に 1 件だけ使える |
| 「run_id がありません（--run か SAI_PARENT_RUN_ID）」（終了コード 4） | `--run <run_id>` を付ける（9.1 節） |
| 「この質問の承認は記録済みです」（終了コード 4） | 同じ `--qid` は 2 回書けない。新しい `--qid` で記録する |
| 「ブロック定義がありません: lab-grow」 | 開発者に、block の入った版かを確かめてもらう |
| 「古い形の research-tree です。`sai-sci init --type <型>` で作り直してください」 | 新しい research-tree を作る。古い木の移行（移行 0062）は新しい版が `papr.db` を開いたときに自動で行われる（[`sai-sci.md`](sai-sci.md) の 4.2 節・6.2 節） |
| 「このブロックでは add_seed_q を出せません」（終了コード 2） | その op は今の run の block の `sci_ops` に無い。`sci-start` の run で出す（9 章） |

---

## 確かめの一覧（全部に印が付けば完了）

- [ ] 練習用プロジェクトを作り、`status --brief` の `counts.axis` が 6 と出た（2・3）
- [ ] `sai-sci node A4` で、軸の A の文が名詞の `手法` だと分かった（3.2）
- [ ] `apply` で seed を作り、`{"op":"add_seed","result":"D…"}` が出て、問い（`SQ1`〜`SQ7`）が見えた。決まっていることを `add_answer` で回答した（4）
- [ ] `apply` で A を足し、`{"op":"add_a","result":"A…"}` が出た。出典（`#37`）を書いた A が断られた（5・5.1）
- [ ] `add_a` の下に A を足し、`move_node` で A8 を移した。同じ親への移動は断られた（5.2）
- [ ] `op` で F を足し、数字の F が出典なしで断られた（6・6.1）
- [ ] `apply` で ★ を付けた（7）
- [ ] seed の無い SPROUT と `paths` が断られ、`answers` 付きで SPROUT を作って `range` が出た。未完成の S には `add_e` が断られ、残りの問いに回答して完成させた。同じ seed から 2 つ目の S を作れた。S のある seed は消せなかった（8）
- [ ] 承認なしの `add_seed_q` が「許可がありません」で断られた（9.1）
- [ ] `approval record` で「許可する」を記録し、`add_seed_q` が入った（SQ8）。同じ命令は断られた（9.2・9.3）
- [ ] 画面の Next Action・Node の札・図・seed（D）の一覧・Create SPROUT の seed の選択で、命令の結果が見えた（10）
- [ ] `context --run 1` を打ち、S を対象にした `context --run 2` に `## STUDY_SEED` が出た。`export` を打った（11）
- [ ] 片付けた（12）
