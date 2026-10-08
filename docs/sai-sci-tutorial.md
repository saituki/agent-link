# research-tree（sai-sci）を手で確かめるチュートリアル

**読み手**: プログラムに慣れていない利用者。
**かかる時間**: 1 時間ほど（エージェントを動かす章を入れると、さらに 30 分ほど）。
**このあとできるようになること**: research-tree の機能をひと通り命令で動かし、出力の意味が分かる。承認（「許可する」）の流れが分かる。

このチュートリアルでは練習用のプロジェクトを 1 つ作り、次の流れを最初から最後まで体験します。

```
① 準備 → ② 練習用プロジェクトを作る → ③ 木の見方
→ ④ Q を読む → ⑤ A を足す → ⑥ F を付ける → ⑦ ★ を付ける
→ ⑧ SPROUT を作る → ⑨ 承認（許可する）を体験する → ⑩ 画面で確かめる
→ ⑪ 命令の一覧 → ⑫ 片付け
```

練習用のプロジェクトは本物の研究とは別の場所（`~/research/sci-practice`）に作ります。**練習で打つ命令は、すべてこの練習用のフォルダの中で行います。** 何を打っても本物の研究には影響しません。

> **出力の番号について**: ここに書いた出力の ID（`A8`・`F10`・`S11` など）は、練習のときに出る番号に読み替えてください。番号は作った順に付くので、手元の出力と一致しないことがあります。出力の形（キーの名前・終了コード）は同じです。

---

## 0. 先に覚えておく言葉

用語は [`sai-sci.md`](sai-sci.md) の用語表を見てください。今は全部を覚えなくて構いません。

| 言葉 | 意味 |
|---|---|
| Q | 問い。研究の穴を表す |
| A | Q への答えの候補（旧名 O） |
| F | 根拠。A の下に置く。`support`・`deny`・`issue` の立場を持つ |
| SEED | 研究の初期の入力。top_Q の親 |
| S（SPROUT） | 実験の計画。A の ID を PATH として集める |
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

RESEARCH_TYPE は 5 つあります（`sai-sci types` で一覧が出ます）。練習では `problem` を使います。

| `--type` の値 | 名前 | どんな研究か |
|---|---|---|
| `problem` | PROBLEM_DRIVEN（課題駆動） | 現場の「困った」から始まる |
| `phenomenon` | PHENOMENON_DRIVEN（現象駆動） | 起きている現象の「なぜ」を問う |
| `method` | METHOD_DRIVEN（手法駆動） | 使える手法が先にあり、使い道を探す |
| `data` | DATA_DRIVEN（データ駆動） | 手元のデータで何が言えるかを問う |
| `improve` | IMPROVE_DRIVEN（改良駆動） | 今ある手法をよくする |

### 2.2 何が起きたか

終わると、たくさんの行が出ます。最後の方は次のようになります。

```
✅ 登録簿に登録しました: sci-practice
✅ 研究の assets を 85 件置きました（既にあるものはそのまま）
✅ mind.md（土壌のひな形）を作りました
✅ 研究プロジェクトを初期化しました: …/sci-practice/.sai/paper
✅ research-tree を作りました（RESEARCH_TYPE: PROBLEM_DRIVEN・Q 7）
```

次の 3 つが起きています。

1. フォルダの中に `.sai/` という隠しフォルダができ、論文の置き場（`.sai/paper/papr.db`）と research-tree（SEED と、その下に並ぶ top_Q）が作られた。
2. エージェント（cc と pi）がこのプロジェクトで使う設定（`.mcp.json`・`.claude/`・`.pi/`）が置かれた。
3. プロジェクトが登録簿（`~/.sai/registry.json`）に `sci-practice` という名前で載った。**これで Web の画面にこのプロジェクトが出ます。**

> 隠しフォルダは Finder では見えません。見たいときは Finder で `⌘ + shift + .`（ピリオド）を押します。
> `.mcp.json`・`.claude/`・`.pi/` の行には「git commit してください」と出ることがあります。練習では無視して構いません。

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
{"counts":{"a":0,"e":0,"f_deny":0,"f_issue":0,"f_support":0,"q":7,"s":0},"next":[{"block":"lab-grow","expect":"子を 1 つ以上","kind":"card","node":"Q2","option":null,"purpose":"「この研究が目指している大きな目的は何か。」の子（A か child_Q）を作る","why":"NO_CHILD"},…],"running":[],"type":"problem","warnings":[…]}
```

- `counts` は生きた Node の数です。`q` は Q、`a` は A、`f_support`・`f_deny`・`f_issue` は F の立場ごと、`e` は E、`s` は SPROUT の数です。
- `next` は Next Action です。`why` が理由（`NO_CHILD`・`NO_FINDING`・`UNVERIFIED`）で、`block` が頼む仕事の種類です。
- `warnings` は、research-tree の決まりに合わない所です。出ていても練習には差し支えありません。

### 3.2 Node を 1 つ読む

```bash
sai-sci node Q1
```

Q1 の文・親までの道・children（子）・Next Action が人が読む形で出ます。

```
Q1 この研究は、どの現場の、誰の、どんな不便さをなくしたいのか。
…
children:
    - Q2 この研究が目指している大きな目的は何か。
    - Q3 その問題があると言える根拠は何か。
```

JSON で読むときは `--json` を付けます（`sai-sci node Q1 --json`）。

- **child_Q を持つ Q には、A を足せません。** Q の下は「A だけ」か「child_Q だけ」で、混ぜません。Q1 は child_Q（Q2・Q3）を持つので、A は足せません。
- A を足すときは、子の無い Q（例 `Q2`）を選びます。

---

## 4. A を足す（利用者の操作）

Q2 の下に A を足します。利用者は `sai-sci apply` で 1 件ずつ足せます。標準入力は op 1 件の JSON です。

```bash
echo '{"op":"add_a","q":"Q2","text":"発作の予知"}' | sai-sci apply --json
```

```
{"op":"add_a","result":"A8"}
```

`result` が新しい Node の ID です（ここでは `A8`）。以後の手順では、この番号を使います。

- `apply` は利用者の命令なので、承認は要りません。
- 同じ文の A を同じ Q に 2 つ足すと、「Q2 に同じ文の A があります」と断られます（終了コード 2）。

---

## 5. F を付ける（agent の操作）

agent は、`sai-sci op` で木を変えます。標準入力は `{"ops": [...]}` の形で、1〜20 件の op を入れられます。**全部が通ったときだけ書かれ、1 件でも弾かれると何も書かれません。**

```bash
echo '{"ops":[{"op":"add_f","parent":"A8","text":"予知の精度は高い","stance":"support"}]}' | sai-sci op
```

```
{"approval":null,"results":[{"op":"add_f","result":"F10"}]}
```

- `approval` は、承認を使ったときだけ値が入ります（ここでは `null`）。
- `stance` は F の立場です（`support`・`deny`・`issue`）。

### 5.1 数字を含む F には出典が要る

数字を含む F は、出典（`cites`）か REPORT のトークン（`report`）が要ります。出典が無いと断られます。

```bash
echo '{"ops":[{"op":"add_f","parent":"A8","text":"精度は 95 %","stance":"deny"}]}' | sai-sci op
```

```
1 件目（add_f）: 数字を含む F には出典（cites）か REPORT のトークン（report）を付けてください
```

（終了コードは 2 です。`echo $?` で確かめられます。）

---

## 6. ★ を付ける（利用者だけ）

★ は利用者だけが付けられる目印です。agent は ★ を付けられません（`star` は承認も要らず、agent の op にも無い）。

```bash
echo '{"op":"star","node":"A8"}' | sai-sci apply --json
```

```
{"op":"star","result":"A8"}
```

- ★ は計算に使いません。目印として画面に出ます。
- 外すときは `{"op":"unstar","node":"A8"}` を `apply` に渡します。

---

## 7. SPROUT を作る

SPROUT（S）は、実験の計画の入口です。PATH として A の ID を 1〜20 個集め、同じ A は 2 回選べません。

```bash
echo '{"ops":[{"op":"add_sprout","title":"予知の SPROUT","paths":["A8"]}]}' | sai-sci op
```

```
{"approval":null,"results":[{"op":"add_sprout","range":{"a":1,"f_deny":0,"f_issue":0,"f_support":1,"q":2},"result":"S11"}]}
```

- `range` は SPROUT の範囲です。PATH の A とその祖先の Q、A の子の F を数えます（`q` は Q の数、`a` は A の数、`f_*` は F の立場ごとの数）。
- 実験の回し方は [`sai-sci.md`](sai-sci.md) の 5.7 節です。練習では実験を回さず、SPROUT を作るまでにしてください（実行先によっては課金されます）。

---

## 8. 承認（「許可する」）を体験する

agent が Q を足すときは、利用者の承認が要ります（`add_q`・`insert_q`・`move_a`・`copy`・`remove`）。承認の流れを体験します。

### 8.1 承認が無いと入らない

承認が無いまま `add_q` を出すと、断られます。agent の実行（run）の番号は `--run` で渡すので、まず run を作ります。

```bash
sai-sci card --node Q5 --child A --purpose "Q5 の子の A を作る" --block lab-grow --json
```

```
{"executer":"human","path":"…/sci-practice/.sai/outputs/todo/1/task.md","run_id":1}
```

- `run_id`（ここでは `1`）は、このあとの手順で使います。
- `--block lab-grow` は、この CARD を動かす仕事の種類です（5.2 節）。CARD を実際に動かすのは、利用者が画面（**プロジェクト** タブ → Dashboard）から **▶ 実行** を押すか、`sai-link card run <run_id>` を打ちます。練習では実行せず、承認の体験だけをします。

承認が無い状態で Q を足すと、次のように断られます。

```bash
echo '{"ops":[{"op":"add_q","parent":"A8","text":"新生児と成人で違うか"}]}' | sai-sci op --run 1 --block lab-grow
```

```
許可がありません: 先に質問ツールで「許可する」を選んでもらってください
```

（終了コードは 4 です。何も書かれていません。）

### 8.2 「許可する」を記録する

agent が質問ツールで「許可する」を選んでもらうと、質問フックがこの記録を自動で行います。練習では、同じ記録を手で打ちます。

```bash
sai-sci approval record --qid demo1 --question "次の op を許可しますか: add_q（A8 の下に「新生児と成人で違うか」）" --answer "許可する" --answered-by user --run 1 --json
```

```
{"approval":{"consumable":true,"id":1,"qid":"demo1","run_id":1}}
```

- `--answered-by` は答えた人の名前です。画面で答えたときは、ログインした人の名前が入ります。
- `consumable: true` は、回答が「許可する」だったという意味です。

### 8.3 許可を使って Q を足す

もう一度同じ `add_q` を出すと、今度は入ります。許可は 1 件だけ使えて、使うと消えます。

```bash
echo '{"ops":[{"op":"add_q","parent":"A8","text":"新生児と成人で違うか"}]}' | sai-sci op --run 1 --block lab-grow
```

```
{"approval":{"answered_by":"user","qid":"demo1"},"results":[{"op":"add_q","result":"Q9"}]}
```

- `approval` に、使った許可の質問の ID と、答えた人が出ます。rt_log の記録にも同じ内容が残ります。
- もう一度同じ命令を打つと、許可が消えているので「許可がありません」と出ます（終了コード 4）。

> 利用者が `sai-sci apply` で行う操作には、承認は要りません。承認が要るのは agent の `sai-sci op` の許可が要る op だけです。

---

## 9. 画面で確かめる

Web の画面（1.2 節の窓を起動したまま）で、research-tree を確かめます。

1. ブラウザで **研究** タブを押し、先頭の **research-tree** を押します（`#/research/tree`）。
2. 上の帯に **Next Action**（先頭 3 件）が出ます。ここに、`sai-sci status --brief` の `next` と同じ内容が出ます。
3. 左の一覧で Node を押すと、右に詳細が出ます。`A8` を押すと、★ と F（`F10`・`F12`）の札が見えます。
4. 真ん中の図（DAG）で、下に Q、上に SPROUT（`S11`）が出ます。
5. 命令で足した A・F・Q は、画面を読み直すと出ます。

> 画面の表示は、命令の結果と同じ research-tree を読んでいます。画面で見えない物は、命令の出力も確かめてください。

---

## 10. 同じことを命令（CLI）で行う

画面でできることは、すべて命令でもできます。エージェントもこの命令を使っています。**練習用のフォルダで行ってください。**

```bash
cd ~/research/sci-practice
```

| したいこと | 命令 |
|---|---|
| 使える RESEARCH_TYPE を見る | `sai-sci types` |
| research-tree の数と次にやることを見る | `sai-sci status --brief` |
| research-tree の全体を JSON で見る | `sai-sci status --json` |
| Node の中身を見る | `sai-sci node A8 --json` |
| エージェントに見せる範囲を見る | `sai-sci context --run 1` |
| 書き出しを作り直す | `sai-sci export` |
| 命令の一覧を見る | `sai-sci schema --json` |

### 10.1 文脈を見る

`sai-sci context --run 1` は、その run の agent に渡す research-tree の範囲と、使える op の一覧です。見出しは次のとおりです（一部）。

```
## TREE
- Q5 対象を、どの範囲で分けて見るのか（分けないなら分けないと言う）。 ← TARGET

## TARGET
- 足す子の型: A

## GUIDE
…

## ALLOWED_OPS
- add_q（承認が要る）: SEED・Q・A の下に child_Q を足す。SEED の下なら top_Q になる。
…
```

- `## TARGET` の `足す子の型` は、CARD を作るときの `--child` の値です（Q の下は `Q` か `A`）。
- `## ALLOWED_OPS` は、この run の block が出せる op です（`sci_ops` の中の op）。

### 10.2 書き出されるファイル

| ファイル | 中身 | 手で直すか |
|---|---|---|
| `.sai/paper/state.md` | 現況（段階・今の作業・Next Action） | 直さない（次の書き出しで上書きされる） |
| `.sai/paper/tree.md` | research-tree の全体を文章にしたもの | 直さない |
| `.sai/paper/mind.md` | 研究の土台（あなたが書く） | **あなたが書く**。エージェントは書き換えない |

書き込む命令（`apply`・`op`・`card`・`init`）は、終わる前に `state.md` と `tree.md` を書き出します。

---

## 11. 片付け

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
| 「Q1 には child_Q があるので、A を足せません」 | 子の無い Q（例 `Q2`）を選ぶ（3.2 節） |
| 「数字を含む F には出典（cites）か REPORT のトークン（report）を付けてください」 | 文を直すか、出典を付ける（5.1 節） |
| 「許可がありません: 先に質問ツールで「許可する」を選んでもらってください」（終了コード 4） | 承認を先に取る（8.2 節）。承認は 1 回の op に 1 件だけ使える |
| 「run_id がありません（--run か SAI_PARENT_RUN_ID）」（終了コード 4） | `--run <run_id>` を付ける（8.1 節） |
| 「この質問の承認は記録済みです」（終了コード 4） | 同じ `--qid` は 2 回書けない。新しい `--qid` で記録する |
| 「ブロック定義がありません: lab-grow」 | 開発者に、block の入った版かを確かめてもらう |
| 命令が「古い形の research-tree」と言う | 新しい research-tree を作り直す（[`sai-sci.md`](sai-sci.md) の 4.2 節） |

---

## 確かめの一覧（全部に印が付けば完了）

- [ ] 練習用プロジェクトを作り、`status --brief` の `counts.q` が 7 と出た（2・3）
- [ ] `sai-sci node Q1` で、Q1 の子が Q2・Q3 だと分かった（3.2）
- [ ] `apply` で A を足し、`{"op":"add_a","result":"A…"}` が出た（4）
- [ ] `op` で F を足し、数字の F が出典なしで断られた（5・5.1）
- [ ] `apply` で ★ を付けた（6）
- [ ] SPROUT を作り、`range` が出た（7）
- [ ] 承認なしの `add_q` が「許可がありません」で断られた（8.1）
- [ ] `approval record` で「許可する」を記録し、`add_q` が入った。同じ命令は断られた（8.2・8.3）
- [ ] 画面の Next Action・Node の札・図で、命令の結果が見えた（9）
- [ ] `context --run 1` と `export` を打った（10）
- [ ] 片付けた（11）
