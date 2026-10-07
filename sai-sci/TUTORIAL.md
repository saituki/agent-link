# research-tree（sai-sci）を手で確かめるチュートリアル

**読み手**: プログラムに慣れていない利用者。
**かかる時間**: 1〜2 時間（エージェントを動かす章を入れると、さらに 30 分ほど）。
**このあとできるようになること**: research-tree の機能をひと通り自分で動かし、画面のどこに何が出るかが分かる。

このチュートリアルでは練習用のプロジェクトを 1 つ作り、次の流れを最初から最後まで体験します。

```
① 準備 → ② 練習用プロジェクトを作る → ③ 画面の見取り図
→ ④ Q を分ける → ⑤ O を書く → ⑥ 論文を集める（検索と選別）
→ ⑦ READ_CARD で F を付ける → ⑧ ★ を付ける → ⑨ Logicality を確かめる
→ ⑩ Issue と実験（E）・エージェントに頼む → ⑪ SPROUT を作る → ⑫ TRIAGE
→ ⑬ Dashboard と PENDING → ⑭ 命令（CLI）→ ⑮ 片付け
```

練習用のプロジェクトは本物の研究とは別の場所（`~/research/sci-practice`）に作ります。**練習で打つ命令は、すべてこの練習用のフォルダの中で行います。** 何を押しても本物の研究には影響しません。安心して試してください。

---

## 0. 先に覚えておく言葉

画面によく出る言葉です。今は全部を覚えなくて構いません。分からなくなったらここに戻ってください。
用語は [`README.md`](README.md) の用語表を見てください。

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

### 1.2 Web の画面（apps/web）を起動する

ターミナルの窓を**新しく 1 つ**開きます（`⌘ + N`）。この窓は Web の画面を動かし続けるための窓なので、作業中は閉じないでください。

```bash
cd ~/workdir/Sai/sai-moon
PATH=$HOME/workdir/Sai/sai-moon/target/release:$PATH \
  apps/web/.venv/bin/uvicorn sai_web.main:app \
  --host 127.0.0.1 --port 8200 --app-dir apps/web/src
```

`Uvicorn running on http://127.0.0.1:8200` と出たら起動できています。
ブラウザ（Safari や Chrome）で **http://localhost:8200/** を開いてください。

> 止めるときは、この窓で `control + C` を押します。

画面の左（iPhone では下）に、次の 6 つのタブがあります。

```
ホーム / プロジェクト / 会話 / 論文 / 研究 / 使用量
```

このチュートリアルで主に使うのは **論文** タブです。エージェントを動かす章（⑩）では **プロジェクト** と **会話** も使います。

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

終わると、たくさんの行が出ます。次の 3 つが起きています。

1. フォルダの中に `.sai/` という隠しフォルダができ、論文の置き場（`.sai/paper/papr.db`）と research-tree（SEED と、その下に並ぶ top_Q）が作られた。
2. エージェント（cc と pi）がこのプロジェクトで使う設定（`.mcp.json`・`.claude/`・`.pi/`）が置かれた。
3. プロジェクトが登録簿（`~/.sai/registry.json`）に `sci-practice` という名前で載った。**これで Web の画面にこのプロジェクトが出ます。**

最後の方に `✅ 登録簿に登録しました: sci-practice` と出ていれば成功です。

> 隠しフォルダは Finder では見えません。見たいときは Finder で `⌘ + shift + .`（ピリオド）を押します。

### 2.3 画面で確かめる

ブラウザで **論文** タブを押します。上の方の 📁 の札が `sci-practice` になっていなければ、📁 を押して `sci-practice` を選んでください。

論文タブの中には、さらに次のタブがあります。

```
Dashboard / Library / Import / Radio
```

research-tree は **研究** タブにあります。**研究方針** を押すと、research-tree の画面（`#/research/tree`）になります。

> research-tree がまだ無いプロジェクトでは「research-tree がまだありません」と出ます。**始める** を押して RESEARCH_TYPE を選ぶと、`sai-sci init` と同じことが行われます。古い形の research-tree だけがあるプロジェクトでは「古い形の research-tree です」と出ます。その場合は新しい research-tree を作り直します（`sai-sci/README.md` の 4.2 節）。

---

## 3. 画面の見取り図

PC（横幅の広い画面）では次のように並びます。

```
┌───────────────────────────────────────────────────────────┐
│ 研究方針 | Dashboard | Discover | Library | Radio          │ ← 論文タブの中のタブ
├───────────────────────────────────────────────────────────┤
│ SEED   ［？target］の［？data.type］で…                      │ ← A SEED
│ 進み具合 2 / 25                                              │
│ Next Action  ① … [Create CARD]  ② …  ③ …                    │ ← B Next Action
│ PROPOSAL 0 | ⚠ 注意 0                                       │ ← C PROPOSAL・注意
├───────────────────────────────────────────────────────────┤
│ [一覧 | 図]                                                  │
│ ┌────────┐ ┌─────────────────┐ ┌──────────────────────────┐ │
│ │Q1      │ │ research-tree   │ │ Q12 Q  Objectivity ■■□ 2/3│ │
│ │ Q2 …   │ │ (Node を押すと   │ │ Logicality UNCHECKED      │ │ ← E 板
│ │  O3 ★  │ │  右に板が開く)   │ │ 操作 / Next Action / F …  │ │
│ └────────┘ └─────────────────┘ └──────────────────────────┘ │
│  D 左の一覧（Q・O・F・E の Node）                         │
└───────────────────────────────────────────────────────────┘
```

iPhone（横幅の狭い画面）では A〜D が縦に並び、Node を押すと E の板が下からせり上がる小窓（sheet）で開きます。✕ で閉じます。PC では板は右に出て（約 420px）、✕ で閉じます。

| 場所 | 何が出るか | いつ変わるか |
|---|---|---|
| A SEED | 研究の初期の入力（RESEARCH_TYPE と `mind.md`）。決まっていない所は［？…］（橙色の点線）で出る。押すと SEED の欄が開き、「SPROUT を作る」釦で PATH（O）を選んで SPROUT を作る | `mind.md` を直すと変わる |
| 進み具合 | 答えが出た Q の数 / 全部 | ★ を付ける・Q を足すと変わる |
| B Next Action | 今やるとよい Action の先頭 3 件。「Create CARD」で CARD のフォームが開く | 何かを変えるたび |
| C PROPOSAL・注意 | PROPOSAL の数。押すと Node を選んだ右の板の PENDING の節が出る。⚠ は research-tree の決まりに合わない所 | Action が終わる・PROPOSAL が来ると変わる |
| D 左の一覧 | Q・O・F・E の Node。O には ★ と Answer / Issue の札が付く | 何かを変えるたび |
| research-tree | 下に ROOT（top_Q）、次に SEED の欄、上に SPROUT、さらに上に BRANCH（E）の図。「F を出す」で F の札を出し入れできる。STAR_PATH は `on_path` の印 | 何かを変えるたび |
| E 板 | 押した Node の中身と操作の釦 | 何かを変えるたび |

### 板の中身

一覧で Q を押すと、右に板が出ます。住所（ブラウザの上の欄）は `…/#/research/tree/Q12` に変わります。

| 欄 | 中身 |
|---|---|
| 見出し | ID・種類・文。Q には Objectivity（`■■□ 2/3`。マスは目安まで）と Logicality の札。O には ★ と Answer / Issue の札 |
| 親までの道 | 親をたどる札。押すとその Node へ移る |
| 操作 | その Node でできる操作の釦と「Create CARD」（下の表） |
| Objectivity | 目安（target）の本数。0〜99 の整数か空（空はガイドの値）。「Objectivity の目安を変える」で直す |
| Logicality | 値・確かめた日・note・根拠の F |
| Next Action | この Node で今やるとよいこと |
| PENDING | この Node への PROPOSAL のうち、まだ TRIAGE していないもの |
| Action の履歴 | この Node に向けた CARD の実行（Action）。「run を開く」 |
| 決定の記録 | ★ を付けた・消したなどの記録。消えずに残る |

板の操作の釦（Node の種類で出る物が変わる）:

| Node | 操作 |
|---|---|
| Q | Q を足す・O を足す・文を直す・目安を変える・消す |
| O | ★ を付ける / 外す・Answer / Issue を変える・F を足す・E を足す・Q を足す・写す・文を直す・消す |
| F | 文を直す・消す |
| E | F を足す・E を足す・文を直す・消す |

> 画面の文字の横にある灰色の点線付きの言葉（SEED・PROPOSAL など）を押すと、その言葉の説明が出ます。

---

## 4. Q を分ける

SEED の下に並ぶ top_Q を child_Q に分けます。

1. 一覧で top_Q（例 `Q1`）を押します。
2. 板の **Next Action** に「Q を分ける」があれば、横の **Create CARD** を押します。
3. 小窓の欄は次のとおりです。

| 欄 | 中身 |
|---|---|
| ブロック | 頼む仕事の種類。Q を分けるときは `sci-split`（最初の Q だけは `sci-start`） |
| 目的 | 必須（300 字まで）。エージェントが何を達成するか |
| 期待する成果 | 任意（300 字まで） |
| 実行するもの | pi か cc。自分で読むときは「自分で読む（READ_CARD）」 |
| 範囲 | エージェントに見せる research-tree の範囲。空ならブロックの既定 |
| 仮置き | `Q9=値` の形（5 個まで）。仮に決めたことにして考えてもらう |

4. **作る** を押すと「run N の CARD を作りました」と出ます。**run N を開く** でプロジェクトの画面が開きます。エージェントを動かすのは ⑩ です。

エージェントを動かさずに、自分で足すこともできます。

1. Q の板の **Q を足す** を押します（親は Q か O）。
2. 文（必須・300 字まで）を書いて送ります。画面に「Q を足しました」と出ます。

**画面のどこが変わるか**: 一覧に child_Q が増え、進み具合の全部の数が変わります。エージェントが出した Q の案は PROPOSAL の PENDING に入ります（⑫）。

---

## 5. O を書く

Q への答えの案（O）を足します。

1. 一覧で `PURPOSE_SUBGOAL` の Q を押します（番号はプロジェクトによって違います。題を探してください）。
2. 板の **O を足す** を押します。
3. 文（必須・120 字まで）を書きます。本文は任意（2000 字まで）です。種類は **Answer**（答えの案）か **Issue**（答えが無い・足りない）を選びます。
4. 送ると「O を足しました」と出て、一覧の Q の下に O が付きます。
5. O をもう 1 つ足します（比べるため）。

エージェントに頼むときは、Q の Next Action の「O を増やす」から `sci-ideate` の CARD を作ります。種類はあとから「Answer / Issue を変える」で変えられます。

---

## 6. 論文を集める（検索と選別）

### 6.1 Discover で検索して Library に入れる

1. **論文** タブ → **Discover** を押します。
2. 検索の欄に `seizure prediction EEG deep learning` と入れて **検索** を押します。
   - 「条件 ▸」を押すと、検索先・年・件数を変えられます。
3. 結果の一覧から、読めそうな論文を 1 つ押し、**⬇ ダウンロード** を押します。1 分ほどかかります。
4. **Library** を押すと、入った論文が並んでいます。

**論文の番号の調べ方**: Library で論文を押すと、住所が `…/#/paper/sci-practice/1` のようになります。**最後の数字（この例では 1）がその論文の番号**です。以下では、この論文を「#1」と呼びます。

### 6.2 検索の履歴と選別

検索は Library の **外部 DB** の履歴に残ります。候補は、最初は pending（未選別）です。

- keep / drop の選別は、エージェント（`sci-screen`）が `sai-paper set mark` で行います。人が画面で付ける必要はありません。
- 人が確かめたいときは、Mac で `sai-paper set show <ID>` を実行します。`<ID>` は `sai-paper set list` で調べます。
- Library の **Paper** を押すと、入った論文が番号の順に並びます。

---

## 7. READ_CARD で F を付ける

論文を 1 本読んで、F（知見）を書きます。

### 7.1 READ_CARD を作る

1. 一覧で O（例 `O3`）を押します。
2. 板の **Create CARD** を押し、実行するものに **自分で読む（READ_CARD）** を選びます。
3. 論文は**ちょうど 1 本**入れます（番号を打つ。例 `1`）。深さは abstract・figures・full のどれかです。
4. **作る** を押します。READ_CARD の対象は Q か O にします。

### 7.2 READ_CARD の画面

READ_CARD の画面（`#/research/read/<run_id>`）が開きます。

1. **読んでいる** の状態にして読み始めます（「読み始めました」と出ます）。抄録・図・節を読みます。
2. 「F を足す」に、分かったことを 1〜3 文で書きます（必須）。出典は `#1 abstract` や `#37 §3.2` の形で書きます。付ける先は O か E を選びます。
3. **数字を含む F には出典が要ります。** 出典が無いと「数字を含む F には出典が要ります」と断られます。
4. 足すと「F を足しました」と出て、「この札で入れた F」に並びます。
5. 読み終えたら **終わり** を押します。実行は review へ移ります。F が 0 本でも止まりません。

**画面のどこが変わるか**: O の板の F の欄に F が付き、Q の Objectivity の本数が増えます（`■□□ 1/2` → `■■□ 2/2`）。

---

## 8. ★ を付ける

Answer の O のうち、研究の道として採る物に ★ を付けます。**★ を付けられるのは利用者だけです**（エージェントは付けられません）。

1. O の板の **★ を付ける** を押し、理由を書きます（任意・500 字まで）。
2. 「★ を付けました」と出ます。O の見出しに ★ が付き、research-tree の「SEED の道」にその O が入ります。
3. 外すときは **★ を外す** を押します。

比べるときは、Q の Next Action の「O を比べる」から `sci-compare` の CARD を作ると手伝ってくれます。`mind.md` の `VALUE_AXES` に書いた軸が、比べるときの軸になります。

**画面のどこが変わるか**: 上の SEED の文に、★ の O が入ります。進み具合も増えます。

---

## 9. Logicality を確かめる

Q の答えの理路が通っているかを確かめて、記録します。

1. Q の Next Action の「Logicality を確かめる」から Create CARD を押し、`sci-debate`（賛否を出し合う）か `sci-review` を選びます。
2. 自分で記録するときは、Q の板の **Logicality を記録** を押します。値は `OK` か `CONTRADICTION` です。
3. `CONTRADICTION` のときは、指摘の文（必須・1000 字まで）と、矛盾する F を 2 つ以上選びます。足りないと「CONTRADICTION には F を 2 つ以上選んでください」と断られます。
4. 記録すると、Q の見出しの Logicality の札が `OK`（緑）か `CONTRADICTION`（橙）に変わります。まだ確かめていない Q は `UNCHECKED`（灰色）です。

---

## 10. Issue と実験（E）

★ を付けられる Answer が無いとき、その O を **Issue** にして、実験（E）で確かめます。

1. O の板の **Answer / Issue を変える** で Issue にします。
2. **E を足す** で実験を足します（文は 120 字まで。MLflow の名前は任意）。親は Issue の O か、ほかの E です。
3. 実験をエージェントに回してもらうときは、`sci-exp-sprout`（CARD の対象は `SEED`）で SPROUT を作ります（`sai-sci/README.md` の 5.7 節）。**実行先によっては課金されます。練習では実験を回さず、足すだけにしてください。**

### 10.1 エージェントに頼む（Action）

エージェント（pi または cc）に research-tree の仕事を頼みます。エージェントは research-tree を**直接は書き換えません**。PROPOSAL を出すだけです。

| PROPOSAL の種類 | 扱い |
|---|---|
| O・F・E・Logicality の記録・SEED | **自動で research-tree に入る**（あとで取り消せる） |
| child_Q・O の写し・消す | **PENDING**になり、あなたが採用すると入る |
| ★・Answer / Issue の変更・文の直し・目安 | エージェントは出せない（あなただけ） |

1. **プロジェクト** タブ → `sci-practice` → **Dashboard** を見ます。常駐（デーモン）が止まっていたら **起動** を押します。
   - 常駐は、CARD を実際に動かす裏方のプログラムです。プロジェクトごとに 1 つです。
2. **List** の `todo` の列にある、作った CARD を押し、**▶ 実行** を押します。
3. CARD は `todo` → `running` → `review` と列を移ります。
   - **cc を初めてこのフォルダで動かすとき**は、エージェントの窓に「このフォルダを信頼しますか」と出ます。「Yes, proceed」を選んで `return` を押してください。
   - エージェントが途中で質問してきたら、**ホーム** の質問の札か、エージェントの窓で答えます。
   - 様子は **会話** タブで見られます。止めたいときは **■ 終了** を押します。

> 常駐を起動せずに体験だけしたいときは、⑭の「エージェントの PROPOSAL をまねる」の命令で PROPOSAL を手で出せます。

---

## 11. SPROUT を作る

STAR_PATH がそろったら、実験の計画（SPROUT）を作ります。

1. 研究方針の画面で SEED の欄を押します。
2. **SPROUT を作る** を押します。
3. 解きたい Issue の O と、前提にする Answer の O を選びます（複数選べます）。Issue の O を 1 つ以上含めてください。
4. 選んだ PATH（終点の O）が `Q2 › Q12 › O15 Issue` の形で出るので、「この組み合わせでよいか」を確かめます。
5. 作ると、SPROUT（`S<n>`）ができ、`sci-exp-sprout` の CARD の入口になります。実験の 1 回の実行（BRANCH の E）は、SPROUT の下に付きます。

---

## 12. TRIAGE

エージェントが出した PROPOSAL のうち、TRIAGE が要る物を採用・却下します。

1. 上の帯の PROPOSAL の札を押すか、Node を選んだ右の板の PENDING の節を開きます。
2. **Node ごと** と **run ごと** を切り替えられます。
3. PROPOSAL 1 つずつに、次の釦があります。

| 釦 | すること |
|---|---|
| 採用 | そのまま research-tree に入れる |
| 直して採用 | 中身（文など）を直してから入れる |
| 却下 | 入れない |
| 取り消し | 自動で入った物を元に戻す |

4. まとまりごとに **まとめて採用** と **まとめて却下** もあります。1 件の失敗で、ほかの件は止まりません。
5. 全部 TRIAGE すると「PENDING の PROPOSAL はありません」と出ます。

**画面のどこが変わるか**: 採用した Q が一覧の親の下に並びます。PROPOSAL の数が 0 に戻ります。

---

## 13. Dashboard と PENDING

### 13.1 PENDING と Next Action

エージェントが出した PROPOSAL のうち、TRIAGE（採用・却下・取り消し）を待っているものを **PENDING** と呼びます。PENDING は、Node を選んだ右の板の **PENDING の節**に出ます。上の帯の PROPOSAL の数からもたどれます。

- 節では、その Node への PENDING の PROPOSAL を 1 つずつ採用・直して採用・却下・取り消しできます。
- 全部 TRIAGE すると節は空になります。
- Next Action には、その Node で次にやるとよいことが並びます。上の帯にも先頭 3 件が出ます。

### 13.2 Dashboard

**論文** タブ → **Dashboard** を押します。

| 欄 | 中身 |
|---|---|
| 要対応 | あなたの判断が要る物。押すと研究方針のその場所が開く |
| 現況 | research-tree から書き出した `state.md` |
| 考え | `mind.md`（あなたが書く研究の土台。ORIGIN・RESEARCH_TYPE・GIVEN・CONSTRAINTS・VALUE_AXES など） |
| メモ | 1 行のメモを足せる |

---

## 14. 同じことを命令（CLI）で行う

画面でできることは、すべて命令でもできます。エージェントもこの命令を使っています。
**練習用のフォルダで行ってください。**

```bash
cd ~/research/sci-practice
```

| したいこと | 命令 |
|---|---|
| 使える RESEARCH_TYPE を見る | `sai-sci types` |
| research-tree の様子を見る | `sai-sci status`（数だけなら `--brief`） |
| Node の中身を見る | `sai-sci node Q2` |
| エージェントに見せる範囲を見る | `sai-sci context --run 2`（⑩ の CARD の番号） |
| PENDING の PROPOSAL を見る | `sai-sci proposals` |
| READ_CARD の中身を見る | `sai-sci read 3`（READ_CARD の run 番号） |
| 書き出しを作り直す | `sai-sci export` |
| 検索セットを見る | `sai-paper set list` |

research-tree を変える命令は、1 行の JSON を `sai-sci apply` に渡します。例として O を 1 つ足します（`Q2` は自分の research-tree にある Q の番号に直します）。

```bash
echo '{"op":"add_o","q":"Q2","text":"発作の予知","kind":"answer"}' | sai-sci apply
```

`{"op":"add_o","result":"O1"}` のような JSON が出たら成功です。ブラウザの研究方針の画面で再読み込みすると、Q2 の下に O が増えています。

### エージェントの PROPOSAL をまねる

常駐を動かさずに TRIAGE を体験したいときは、次の命令でエージェントの PROPOSAL をまねられます（`--run 2` は ⑩ で作った CARD の番号）。

```bash
echo '{"proposals":[{"op":"add_q","parent":"Q2","text":"新生児と成人で発作の見え方はどう違うか"}]}' \
  | sai-sci propose --run 2 --block sci-split
```

- child_Q（`add_q`）は PENDING（pending）になります。`sai-sci proposals` で ID（`P1` など）を見て、`sai-sci triage P1 --accept` で採用します（却下は `--reject`）。
- ブロックが出せない PROPOSAL は断られます。

### 書き出されるファイル

| ファイル | 中身 | 手で直すか |
|---|---|---|
| `.sai/paper/state.md` | 現況（Dashboard の「現況」） | 直さない（次の書き出しで上書きされる） |
| `.sai/paper/tree.md` | research-tree の全体を文章にしたもの | 直さない |
| `.sai/paper/mind.md` | 研究の土台（Dashboard の「考え」） | **あなたが書く**。エージェントは書き換えない |

---

## 15. 片付け

練習が終わったら片付けます。**消す前に、消すものの名前が `sci-practice` であることを必ず確かめてください。**

1. 常駐を起動していたら、**プロジェクト** タブ → `sci-practice` → Dashboard の **停止** を押します。
2. Web の画面を動かしている窓で `control + C` を押します。
3. 登録簿から `sci-practice` を外します。次の命令は、登録簿の写し（`registry.json.bak`）を取ってから、`sci-practice` の行だけを外します。

```bash
cp ~/.sai/registry.json ~/.sai/registry.json.bak
python3 -c '
import json, pathlib
p = pathlib.Path.home() / ".sai" / "registry.json"
rows = json.loads(p.read_text())
p.write_text(json.dumps([r for r in rows if r.get("name") != "sci-practice"], ensure_ascii=False, indent=2))
print("外しました")'
```

4. 練習用のフォルダを消します（ゴミ箱を通らずに消えます。名前を確かめてから打ってください）。

```bash
rm -rf ~/research/sci-practice
```

---

## 困ったとき

| 起きたこと | 直し方 |
|---|---|
| Web にプロジェクトが出ない | 📁 を押して選び直す。無ければ `~/research/sci-practice` で `sai-sci init --type problem` をもう一度打つ（何度打っても壊れない） |
| 画面が古いまま | ブラウザで再読み込み（`⌘ + R`） |
| 「research-tree がまだありません」 | **始める** を押して型を選ぶ |
| 「古い形の research-tree です」 | 新しい research-tree を作り直す（`sai-sci/README.md` の 4.2 節） |
| READ_CARD で「F を付ける O または E がありません」 | ⑤で O を足してから戻る |
| F を送ると断られる | 数字を書いたなら出典（`#1 abstract` の形）を入れる |
| CONTRADICTION を記録できない | 指摘の文と、F を 2 つ以上入れる |
| CARD が `todo` から動かない | プロジェクトの Dashboard で常駐が「稼働中」か確かめる。止まっていれば **起動** |
| cc が止まったまま | エージェントの窓に「このフォルダを信頼しますか」が出ていないか確かめる |
| Project タブが 503 | Web を起動するとき、1.2 の `PATH=…` の行を付け忘れている |

---

## 確かめの一覧（全部に印が付けば完了）

- [ ] 練習用プロジェクトを作り、Web の研究方針の画面が開いた（②）
- [ ] 上の帯（SEED・Next Action・PROPOSAL）・一覧・research-tree・板の場所が分かった（③）
- [ ] Q を足した（④）
- [ ] O を 2 つ足した（⑤）
- [ ] Discover で論文を 1 本入れ、番号を調べた。Library の **Paper** で番号を確かめた（⑥）
- [ ] READ_CARD を作り、F を足し、終えた。Objectivity の本数が増えた（⑦）
- [ ] 数字だけで出典の無い F が断られた（⑦）
- [ ] O に ★ を付け、SEED の文に入った（⑧）
- [ ] Logicality を記録した（⑨）
- [ ] O を Issue にして、E を足した（⑩）
- [ ] SPROUT を作った（⑪）
- [ ] PROPOSAL を採用・直して採用・却下で TRIAGE した（⑫）
- [ ] PENDING の節と、Dashboard の要対応・現況・考えを見た（⑬）
- [ ] 命令で O を足し、画面に出た（⑭）
- [ ] 片付けた（⑮）
