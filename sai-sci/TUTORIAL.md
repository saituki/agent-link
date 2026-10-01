# 研究方針の木（sai-sci）を手で確かめるチュートリアル

**読み手**: プログラムに慣れていない利用者。
**かかる時間**: 1〜2 時間（エージェントを動かす章を入れると、さらに 30 分ほど）。
**このあとできるようになること**: 研究方針の木の機能をひと通り自分で動かし、画面のどこに何が出るかが分かる。

このチュートリアルでは練習用のプロジェクトを 1 つ作り、次の流れを最初から最後まで体験します。

```
① 準備 → ② 練習用プロジェクトを作る → ③ 画面の見取り図
→ ④ 論文を集める → ⑤ 論文を ITEM（項目）に結ぶ → ⑥ READ_CARD（読む札）で読んで FINDING（知見）を書く
→ ⑦ OPTION（候補）と FINDING を足す → ⑧ 決める → ⑨ 決めたことを戻す
→ ⑩ QUESTION（問い）・子の ITEM・文・線 → ⑪ エージェントに頼む → ⑫ TRIAGE（仕分け）
→ ⑬ Dashboard と DEBT（負債） → ⑭ ROOT_GRAPH（根の図）と TABLE（表） → ⑮ TOMBSTONE（墓標） → ⑯ TUI
→ ⑰ 同じことをコマンドで → ⑱ 片付け
```

練習用のプロジェクトは本物の研究とは別の場所に作ります。何を押しても本物の研究には影響しません。安心して試してください。

---

## 0. 先に覚えておく言葉

画面によく出る言葉です。今は全部を覚えなくて構いません。分からなくなったらここに戻ってください。
画面の専門語は英語の大文字です（例 ITEM・OPTION）。日本語の名前は、この表で確かめられます。

| 言葉（英語） | 日本語 | 意味 | 例 |
|---|---|---|---|
| ― | 研究方針の木 | 「何を研究するか」を決めるための木。ITEM・OPTION・FINDING がつながっている | ― |
| ITEM | 項目 | 研究で決めなければならない穴 | PURPOSE_SUBGOAL・TARGET・DATA_TYPE |
| OPTION | 候補 | ITEM に入りうる答えの案 | PURPOSE_SUBGOAL の OPTION「発作の予知」 |
| FINDING | 知見 | 論文などから分かった 1〜3 文の事実と、その CITE | 「数分先の発作予測で AUC 0.9 前後の報告がある（#1 abstract）」 |
| CITE | 出典 | FINDING がどこに書いてあったか。`#論文の番号 場所` の形で書く | `#1 abstract`、`#37 §3.2` |
| POLARITY | 向き | FINDING が OPTION を支えるか、否定するか | SUPPORT・REFUTE・NEUTRAL・NONE（支持・反証・中立・該当なし） |
| SEED_SENTENCE | 種の文 | 研究を 1 文で言うためのひな形。決まった ITEM の値が入る | 「［？TARGET］の［？DATA_TYPE］で発作の予知を行う。…」 |
| GATE | 関門 | 研究が進んだかを測る関所。上から順に通っていく | 「GATE 3/7 DATA_ACCESS_DECIDED」 |
| DEBT | 負債 | 後回しにしている片付け物の数 | 「PENDING_PROPOSALS 2」 |
| READ_CARD | 読む札 | 自分が論文を 1 本読むための作業票 | ― |
| CARD | カード | エージェント（pi や cc）に仕事を頼む作業票 | ― |
| PROPOSAL | 提案 | エージェントが出した「木をこう変えたい」という申し出 | ― |
| TRIAGE | 仕分け | PROPOSAL を採用するか却下するかを決めること | ― |
| TOMBSTONE | 墓標 | 「この論文は外した」という記録。同じ論文が検索に出なくなる | ― |
| ― | ターミナル | 文字で命令を打つアプリ（Mac の「ターミナル.app」） | ― |

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
which sai-moon-paper sai-moon-link sai-moon-paper-tui
```

3 行とも `/Users/<あなたの名前>/.local/bin/…` のように出れば準備はできています。
`not found` と出たら、開発者に「`~/.local/bin` に sai-moon の実行ファイルへのリンクを置いてほしい」と頼んでください。

### 1.2 Web の画面（apps/phone）を起動する

ターミナルの窓を**新しく 1 つ**開きます（`⌘ + N`）。この窓は Web の画面を動かし続けるための窓なので、作業中は閉じないでください。

```bash
cd ~/workdir/Sai/sai-moon
PATH=$HOME/workdir/Sai/sai-moon/target/release:$PATH \
  apps/phone/.venv/bin/uvicorn sai_phone.main:app \
  --host 127.0.0.1 --port 8200 --app-dir apps/phone/src
```

`Uvicorn running on http://127.0.0.1:8200` と出たら起動できています。
ブラウザ（Safari や Chrome）で **http://localhost:8200/** を開いてください。

> 止めるときは、この窓で `control + C` を押します。

画面の左（iPhone では下）に、次の 5 つのタブがあります。

```
ホーム / プロジェクト / 会話 / 論文 / 使用量
```

このチュートリアルで主に使うのは **論文** タブです。エージェントを動かす章（⑪）では **プロジェクト** と **会話** も使います。

---

## 2. 練習用のプロジェクトを作る

### 2.1 フォルダを作って初期化する

もう 1 つターミナルの窓を開き（こちらは命令を打つ窓です）、次を打ちます。

```bash
mkdir -p ~/research/sci-practice
cd ~/research/sci-practice
sai-moon-paper init --sci --type problem
```

- `mkdir -p` はフォルダを作る命令です。
- `cd` はそのフォルダへ移る命令です。
- `sai-moon-paper init --sci --type problem` は、ここを「研究方針の木を使う研究プロジェクト」にする命令です。`--type problem` は RESEARCH_TYPE（研究の型）を PROBLEM_DRIVEN（課題駆動）にするという意味です。

研究の型は 5 つあります。練習では `problem` を使います。

| `--type` の値 | 名前 | どんな研究か |
|---|---|---|
| `problem` | PROBLEM_DRIVEN（課題駆動） | 現場の「困った」から始まる |
| `phenomenon` | PHENOMENON_DRIVEN（現象駆動） | 起きている現象の「なぜ」を問う |
| `method` | METHOD_DRIVEN（手法駆動） | 使える手法が先にあり、使い道を探す |
| `data` | DATA_DRIVEN（データ駆動） | 手元のデータで何が言えるかを問う |
| `improve` | IMPROVE_DRIVEN（改良駆動） | 今ある手法をよくする |

### 2.2 何が起きたか

終わると、たくさんの行が出ます。次の 3 つが起きています。

1. フォルダの中に `.sai/` という隠しフォルダができ、論文の置き場（`.sai/paper/papr.db`）と研究方針の木が作られた。
2. エージェント（cc と pi）がこのプロジェクトで使う設定（`.mcp.json`・`.claude/`・`.pi/`）が置かれた。
3. プロジェクトが登録簿（`~/.sai-moon/registry.json`）に `sci-practice` という名前で載った。**これで Web の画面にこのプロジェクトが出ます。**

最後の方に `✅ 登録簿に登録しました: sci-practice` と出ていれば成功です。

> 隠しフォルダは Finder では見えません。見たいときは Finder で `⌘ + shift + .`（ピリオド）を押します。

### 2.3 画面で確かめる

ブラウザで **論文** タブを押します。上の方の 📁 の札が `sci-practice` になっていなければ、📁 を押して `sci-practice` を選んでください。

論文タブの中には、さらに次のタブがあります。

```
研究方針 / Dashboard / Discover / Library / Radio
```

**研究方針** を押すと、研究方針の木の画面になります（研究方針の木を使うプロジェクトでは、最初からこの画面が開きます）。

---

## 3. 研究方針の画面の見取り図

PC（横幅の広い画面）では次のように並びます。

```
┌───────────────────────────────────────────────────────────┐
│ 研究方針 | Dashboard | Discover | Library | Radio          │ ← 論文タブの中のタブ
├───────────────────────────────────────────────────────────┤
│ SEED_SENTENCE ［？TARGET］の［？DATA_TYPE］で［？PURPOSE_SUBGOAL］を行う。… │ ← A SEED_SENTENCE
│ GATE    GATE 1/7　BACKGROUND_GOAL_DECIDED                   │ ← B GATE
│ DEBT    PENDING_PROPOSALS 0 | UNREFLECTED_PAPERS 0 …        │ ← C DEBT
├───────────────────────────────────────────────────────────┤
│ [LIST][ROOT_GRAPH][TABLE][RUN_GRAPH]                        │ ← D 表示の切り替え
│ ┌──────────────┐ ┌──────────────────────────────────────┐ │
│ │○ N1 BACKGROUND│ │ ○ OPEN N12 PURPOSE_SUBGOAL           │ │
│ │  ○ N2 BACKGROUND_GOAL │ [決める][対象外][OPTION を足す]… │ │ ← F 板
│ │  …           │ │ 次の一手 / QUESTION / OPTION /        │ │
│ │○ N11 PURPOSE  │ │ FINDING / LINK / 文献の流れ /         │ │
│ │  ○ N12 PURPOSE_SUBGOAL │ TRIAGE 待ち / 行動の履歴 /      │ │
│ │              │ │ 決定の記録                            │ │
│ └──────────────┘ └──────────────────────────────────────┘ │
│   E ITEM の一覧                                             │
└───────────────────────────────────────────────────────────┘
```

iPhone（横幅の狭い画面）では A〜E が縦に並び、一覧の ITEM を押すと F の板が下からせり上がる小窓で開きます。✕ で閉じます。

| 場所 | 何が出るか | いつ変わるか |
|---|---|---|
| A SEED_SENTENCE | 研究を 1 文で言ったもの。決まっていない所は **［？ITEM の名前］**（橙色の点線。例 ［？TARGET］）で出る | ITEM を「決める」と、その値が入る |
| B GATE | 今いる GATE（例 GATE 1/7　BACKGROUND_GOAL_DECIDED）。押すと 7 つの GATE の一覧（通った・まだ）と通る条件が出る。全部通ると「GATE 7/7　SEED にできる」になる | 条件の ITEM が決まると進む |
| C DEBT | 後回しの片付け物の数。0 のものは薄く出る。PENDING_PROPOSALS を押すと TRIAGE の画面へ | PROPOSAL が来る・論文を読むと増減する |
| D 切り替え | LIST・ROOT_GRAPH・TABLE・RUN_GRAPH | ― |
| E 一覧 | ITEM と OPTION の並び。印（下の表）と BLANK の種類の札が付く | 何かを変えるたび |
| F 板 | 選んだ ITEM の中身と操作のボタン | 何かを変えるたび |

### ITEM と OPTION の印

| 印 | 意味 |
|---|---|
| ○ OPEN | まだ決めていない ITEM（未決定） |
| ◐ TENTATIVE | 仮に決めた ITEM（仮決定） |
| ● DECIDED | 決めた ITEM（決定） |
| × EXCLUDED | 今回の研究では扱わない ITEM（対象外） |
| ◇ CANDIDATE | OPTION（まだ選ばれていない。候補） |
| ◆ SELECTED | 選ばれた OPTION（選択） |
| ✕ REJECTED | 外された OPTION（却下） |
| ◌ DORMANT | 今は使わないが残しておく OPTION（休眠） |

### BLANK の種類（まだ決まっていない ITEM に付く橙色の札）

| 札 | 意味 |
|---|---|
| UNTOUCHED | まだ何もしていない（未着手） |
| IN_PROGRESS | READ_CARD や CARD が動いている（調査中） |
| INSUFFICIENT | FINDING や OPTION が足りない（情報不足） |
| AWAITING_DECISION | 材料はそろった。あなたが決める番（判断待ち） |
| CONFLICT | SUPPORT と REFUTE がぶつかっている（対立） |

決まった ITEM にも、MARK（印）として灰色の札が付くことがあります。

| 札 | 意味 |
|---|---|
| WEAK_EVIDENCE | 根拠の FINDING が無いまま決めた（根拠不足） |
| RECHECK | 上流の ITEM が後から変わったので、見直した方がよい（要再確認） |

### 板（F）のボタン

一覧で ITEM（例: **N12 PURPOSE_SUBGOAL**）を押すと、右に板が出ます。住所（ブラウザの上の欄）は `…/#/papers/tree/N12` に変わります。

| ボタン | すること | この文書の章 |
|---|---|---|
| 決める | OPTION を選んで ITEM を決める（理由は必須） | ⑧ |
| 再開 | 決めた ITEM を OPEN に戻す | ⑨ |
| 対象外 | 今回は扱わない ITEM（EXCLUDED）にする | ⑨ |
| OPTION を足す | OPTION を 1 つ足す（CHOOSE・LIST の ITEM だけに出る） | ⑦ |
| 子の ITEM を足す | この ITEM の下に小さな ITEM を足す | ⑩ |
| QUESTION を足す | この ITEM について考えたい QUESTION を足す | ⑩ |
| FINDING を足す | 分かったことを CITE 付きで足す | ⑦ |
| 結ぶ | 論文・報告・ノート・SEARCH_SET・実行をこの ITEM に LINK する | ⑤ |
| 文を直す | ITEM の題と本文を書き直す | ⑩ |
| 線を引く | ほかの ITEM との関係（DEPENDS・DERIVED・ANALOGOUS）の線を引く | ⑩ |

板の下の欄は次のとおりです。

| 欄 | 中身 |
|---|---|
| ITEM_GUIDE | ▶ を押すと開く。この ITEM で何を考えるか・つまずきやすい所・CONDITION（埋まった条件） |
| 次の一手 | 今やるとよいこと（NEXT_ACTION）。「カードを作る」でエージェントに頼める。「自分で行う」の札は自分でやること。一番下に「READ_CARD を作る」 |
| QUESTION | 考える QUESTION。「答える」「取り下げる」 |
| OPTION | OPTION の表（状態・支持の数・反証の数・論文の数）と、選ぶ・REJECTED・DORMANT・CANDIDATE に戻す のボタン |
| FINDING | この ITEM の FINDING と CITE。「外す」 |
| LINK | 結んだ論文など。論文には READ_STATE と「外す（TOMBSTONE）」 |
| 文献の流れ | 検索 → 選別 → 読む → 反映 の数（LITERATURE_FLOW） |
| TRIAGE 待ち | この ITEM への PROPOSAL のうち、まだ TRIAGE していないもの |
| 行動の履歴 | この ITEM に向けた READ_CARD と CARD の実行（ACTION_LOG）。「実行を開く」 |
| 決定の記録 | 決めた・戻した・却下した記録（DECISION_LOG）。消えずに残る |

> 画面の文字の横にある灰色の点線付きの言葉（SEED_SENTENCE・GATE など）を押すと、その言葉の説明が出ます。

---

## 4. 論文を集める（Discover）

木に結ぶ論文を 1 本、Library に入れます。

1. **論文** タブ → **Discover** を押します。
2. 検索の欄に `seizure prediction EEG deep learning` と入れて **検索** を押します。
   - 「条件 ▸」を押すと、検索先（s2・pubmed・arxiv など）・年・件数を変えられます。
3. 結果の一覧から、読めそうな論文を 1 つ押します。
4. **⬇ ダウンロード** を押します。
   - 上の **進み具合** に「ダウンロード中 1 件」と出ます。1 分ほどかかります。
   - 誰でも読める（オープンアクセスの）論文だけが PDF まで入ります。
5. **Library** を押すと、入った論文が並んでいます。

**論文の番号の調べ方**: Library で論文を押すと、住所が `…/#/paper/sci-practice/1` のようになります。**最後の数字（この例では 1）がその論文の番号**です。以下では、この論文を「#1」と呼びます。

---

## 5. 論文を ITEM に結ぶ

論文を PURPOSE_SUBGOAL の ITEM に LINK します。

1. **研究方針** → 一覧で **N12 PURPOSE_SUBGOAL** を押します（番号はプロジェクトによって少し違うことがあります。題が PURPOSE_SUBGOAL の ITEM を探してください）。
2. 板の **結ぶ** を押します。右（iPhone では下）に小窓が出ます。
3. 次のように入れます。

| 欄 | 入れるもの |
|---|---|
| 相手 | **論文** を押す（ほかに 報告・ノート・SEARCH_SET・実行 がある） |
| 番号 | `1`（④で調べた番号） |
| ROLE | **EXEMPLAR**（ほかに COMPARE・ISSUE_EVIDENCE・METHOD_SOURCE・DATA_SOURCE・BACKGROUND・COUNTER。論文のときは必ず選ぶ） |
| READ_STATE | **UNREAD** のまま |
| 理由 | 空でもよい（書くと記録に残る） |

4. **送る** を押します。

**画面のどこが変わるか**

- 下に「結び付けました」と出ます。
- 板の **LINK** に `L1 論文 EXEMPLAR UNREAD <論文の題>` が出ます。
- 一覧の N12 の行に「論文 1」の札が付きます。

---

## 6. READ_CARD で論文を読み、FINDING を書く

READ_CARD（読む札）は、自分が論文を 1 本読むときの作業票です。読んで分かったことが、そのまま木の FINDING になります。

### 6.1 READ_CARD を作る

1. N12 の板の **次の一手** の欄の一番下にある、青い **READ_CARD を作る** を押します。
2. 小窓で次を選びます。

| 欄 | 中身 |
|---|---|
| 論文 | 結んである論文から選ぶ（#1） |
| 役割 | 代表例（結んだときの役割が入っている） |
| 深さ | ABSTRACT（要旨だけ）・FIGURES（図と表まで）・FULL（全部） |
| 目的 | 空なら「PURPOSE_SUBGOAL のために #1 を読む」になる |

3. **作る** を押すと「READ_CARD を作りました」と出て、READ_CARD の画面（住所 `#/papers/read/1`）に移ります。

> LINK している論文が 1 本も無いと、「先に論文を LINK してください」と出ます。⑤に戻ってください。

### 6.2 READ_CARD の画面

```
┌ この READ_CARD ─────────────────────────┐
│ <論文の題>  2025  （まだ読んでいない）     │ ← 題を押すと論文が開く
│ N12 PURPOSE_SUBGOAL ・役割 EXEMPLAR ・深さ FULL │
│ PURPOSE_SUBGOAL のために #1 を読む        │
│ 研究方針へ戻る                            │
└─────────────────────────────────────────┘
[読み始める]
┌ QUESTION ───────────────────────────────┐
│ その研究が 1 回の研究で何を達成したかは？  │ ← 役割ごとの QUESTION
│ 文 [                     ]               │
│ 場所 [ §3.2（空なら #1） ]                │
│ POLARITY [SUPPORT ▼]    [FINDING を足す]  │
└─────────────────────────────────────────┘
┌ 入れた FINDING ─┐   ┌ 終える ─┐
```

1. **読み始める** を押します。札が「読んでいる」に変わります。
2. 論文を読みます（題を押すと論文が開きます）。
3. QUESTION に答える形で、分かったことを書きます。
   - 文: `新生児の脳波から数分先の発作を予測することを到達点としている。`
   - 場所: `abstract`（空なら出典は `#1` だけになる）
   - POLARITY: SUPPORT
4. **FINDING を足す** を押します。**入れた FINDING** に `F1 SUPPORT …` が出ます。
5. **終える** を押します。
   - FINDING が 1 つも無いまま終えようとすると、「反映しない理由」を聞かれます（1 語でも構いません）。
   - 終えると、研究方針の板に戻ります。

**画面のどこが変わるか**（N12 の板）

| 欄 | 変わり方 |
|---|---|
| FINDING | `F1 SUPPORT … 出典: #1 abstract` が出る |
| LINK | L1 の READ_STATE が **REFLECTED** になり、`L2 実行 1` が増える |
| 文献の流れ | 選別 1・読む 1 になる |
| 行動の履歴 | `#1 sci-read review …`（「実行を開く」で実行の中身へ） |
| 一覧 | 読んでいる間、N12 の札が **IN_PROGRESS** になる |

> UNREFLECTED_PAPERS（読んだが反映していない論文。DEBT の 1 つ）は、FINDING を書かずに読み終えた論文の数です。

---

## 7. OPTION と FINDING を足す

### 7.1 OPTION を足す

1. N12 の板で **OPTION を足す** を押します。
2. 題に `発作の予知` と入れて **送る** を押します（本文は無くてもよい）。
3. もう一度 **OPTION を足す** を押して、`発作部位の推定` を足します。

**画面のどこが変わるか**

- 一覧の N12 のすぐ下に `◇ N25 発作の予知` と `◇ N26 発作部位の推定` が並びます（番号は環境によって違います）。
- 板の **OPTION** の欄に表が出ます。列は OPTION・状態・支持・反証・論文 です。

### 7.2 OPTION に FINDING を足す

1. 一覧で `N25 発作の予知` を押します。OPTION の板が出ます（札は `◇ CANDIDATE`）。
   - OPTION の板には「決める」「再開」「対象外」はありません。OPTION の状態は、親の ITEM（N12）の **OPTION** の表で変えます。
2. **FINDING を足す** を押して、次のように入れます。

| 欄 | 入れるもの |
|---|---|
| 文 | `数分先の発作予測で AUC 0.9 前後の報告がある。`（1〜3 文） |
| CITE | `#1 abstract`（「1 行足す」で CITE を増やせる） |
| POLARITY | SUPPORT |
| 根拠の段 | 任意。① 著者自身の限界 ② 別の研究が指摘 ③ 比較実験で示す ④ 自分で確かめた |
| QUESTION | 任意。この QUESTION への答えなら選ぶ |
| 論文 | 任意。結んでいる論文から選ぶ |

3. **送る** を押します。
4. 同じようにして、`N26 発作部位の推定` に **REFUTE** の FINDING を足します（例: `部位の推定は電極の数が少ないと難しいとされる。` CITE `#1 discussion`）。

**試してみよう（わざと失敗する）**: 文に数字（例 `0.85`）を書いて CITE を空にすると、断られます。**数字を書くときは必ず CITE が要る**決まりです。

**画面のどこが変わるか**: N12 の板の **OPTION** の表で、N25 の「支持」が 1、N26 の「反証」が 1 になります。N12 の **FINDING** の欄にも、OPTION の FINDING が OPTION の番号付きで出ます。

---

## 8. 決める

### 8.1 小目的を決める

1. 一覧で **N12 PURPOSE_SUBGOAL** を押し、板の **決める** を押します。
2. 小窓の欄は次のとおりです。

| 欄 | 入れるもの |
|---|---|
| 選ぶ OPTION | **N25 発作の予知** |
| TENTATIVE にする | 今は付けない（付けると ◐ TENTATIVE になる） |
| 根拠の FINDING | F1 と F2 に印を付ける |
| 外す OPTION と理由 | N26 に印を付けて、理由に `電極の数が足りない` |
| 反証を探した | 印を付ける（反対の意見も探したか、という確認） |
| 理由 | `予知の支持の FINDING が 2 つあり、部位の推定には反証がある`（必須） |

3. **送る** を押します。

**画面のどこが変わるか**

| 場所 | 変わり方 |
|---|---|
| A SEED_SENTENCE | `［？PURPOSE_SUBGOAL］` が **発作の予知** に変わる |
| E 一覧 | N12 が `● DECIDED`、N25 が `◆ SELECTED`、N26 が `✕ REJECTED` |
| F 板の決定の記録 | `DECIDE N25 を選んだ <理由> 根拠: F1、F2 却下: N26 user <日時>` |
| B GATE | PURPOSE_SUBGOAL_DECIDED が「通った」になる（まだ BACKGROUND_GOAL_DECIDED が残っているので、札は GATE 1/7 のまま） |

**試してみよう（わざと失敗する）**

- OPTION がある ITEM で、OPTION を選ばずに送る →「OPTION を選んでください」
- 理由を空で送る →「理由を入力してください」

### 8.2 GATE を進める（BACKGROUND_GOAL を決める）

GATE の 1 番目 BACKGROUND_GOAL_DECIDED は、**N2 BACKGROUND_GOAL** が決まると通ります。

1. 一覧で **N2 BACKGROUND_GOAL** を押します。
2. **文を直す** を押し、本文に `新生児集中治療室で、発作の見逃しを減らす。` と書いて **送る**。
3. **決める** を押し、理由に `指導教員と合意した` と書いて **送る**（BACKGROUND_GOAL は WRITE の ITEM なので、OPTION はありません）。

**画面のどこが変わるか**

- B GATE の札が **GATE 3/7　DATA_ACCESS_DECIDED** に進みます。札を押すと、BACKGROUND_GOAL_DECIDED と PURPOSE_SUBGOAL_DECIDED が **通った** になっています。
- N2 の行に **WEAK_EVIDENCE** の札が付きます（根拠の FINDING を付けずに決めたため）。
- N12 に **RECHECK** が付くことがあります（上流の N2 が後から変わったため）。次の一手に「上流の決定が変わったので PURPOSE_SUBGOAL を見直す（自分で行う）」と出ます。

---

## 9. 決めたことを戻す

決めたことは消えません。戻した記録も **決定の記録**（DECISION_LOG）に積み上がります。

### 9.1 選んだ OPTION を REJECTED にすると、ITEM は OPEN に戻る

1. N12 の板の **OPTION** の表で、N25 の行の **REJECTED** を押します。
2. 状態は REJECTED のまま、理由に `練習のために一度外す` と書いて **送る**。

**画面のどこが変わるか**

- N12 が `○ OPEN` に戻り、SEED_SENTENCE も `［？PURPOSE_SUBGOAL］` に戻ります。
- 決定の記録に `REJECT_OPTION`（対象: N25）と `REOPEN` の 2 行が増えます。

元に戻すには、N25 の行の **CANDIDATE に戻す** を押し（理由を書いて送る）、もう一度 **決める** を行います。

| OPTION の表のボタン | すること |
|---|---|
| 選ぶ | その OPTION を SELECTED にする |
| REJECTED | 外す（SELECTED だったら、ITEM は OPEN に戻る） |
| DORMANT | 今は使わないが残す |
| CANDIDATE に戻す | ただの CANDIDATE に戻す |

### 9.2 再開と対象外

- **再開**: 決めた ITEM を OPEN に戻します。SELECTED だった OPTION も CANDIDATE に戻ります。
- **対象外**: 今回の研究で扱わない ITEM にします（`× EXCLUDED`）。**再開** で戻せます。

---

## 10. QUESTION・子の ITEM・文・線

### 10.1 QUESTION

ITEM には、最初から QUESTION がいくつか付いています（例: N12 の `Q34 1 回の研究で何を達成するか`）。

- **答える**: 答えを書いて送ると、札が ANSWERED になる。
- **取り下げる**: 要らない QUESTION を下げる。札は WITHDRAWN になる。
- **QUESTION を足す**（板の上のボタン）: 自分で QUESTION を足す。

### 10.2 子の ITEM を足す

**子の ITEM を足す** で、題（例 `確かめの範囲`）と ITEM_TYPE（CHOOSE・LIST・WRITE・VERIFY、複数可）を選んで送ると、その ITEM の下に新しい ITEM ができます。

### 10.3 文を直す

**文を直す** で、ITEM の題と本文を書き直せます。本文は SEED_SENTENCE に入る値にもなります（WRITE の ITEM のとき）。

### 10.4 線を引く

**線を引く** で、ほかの ITEM との関係を描きます。

| 線の種類 | 意味 |
|---|---|
| DEPENDS（依存） | この ITEM は、あの ITEM が決まらないと決められない |
| DERIVED（由来） | この ITEM は、あの ITEM から出てきた |
| ANALOGOUS（同型） | 同じ形の問題 |

輪になる線（A が B に依存し、B が A に依存する）は断られます。引いた線は ⑭ の **ROOT_GRAPH** で見えます。

---

## 11. エージェントに頼む（CARD）

ここからは、エージェント（pi または cc）に木の仕事を頼みます。エージェントは木を**直接は書き換えません**。PROPOSAL を出すだけです。

| PROPOSAL の種類 | 扱い |
|---|---|
| FINDING・QUESTION・LINK・SEED | **自動で木に入る**（AUTO_OP。あとで外せる） |
| OPTION・ITEM の追加・決定・線・対象外 | **TRIAGE 待ち**（REVIEW_OP）になり、あなたが採用すると入る |

### 11.1 CARD を作る

1. 一覧で **N4 TARGET** を押します。
2. **次の一手** の「対象の候補を出す」の横の **カードを作る** を押します。
3. 小窓の欄は次のとおりです。

| 欄 | 中身 |
|---|---|
| ブロック | 頼む仕事の種類（16 本）。ここでは 💡 候補を増やす（`sci-ideate`）が入っている |
| 目的 | 必須。エージェントが何を達成するか |
| 期待する成果 | 任意 |
| QUESTION | 任意。この QUESTION に答えてもらう |
| 実行するもの | pi か cc |
| SCOPE | エージェントに見せる木の範囲。空ならブロックの既定（NODE・PATH・SUBTREE・SIBLINGS・SUMMARY・BLIND） |
| MODE | 1 本を読むときだけ使う（PURPOSE_FIRST・BLIND_FIRST。目的先行・目的後付け） |
| ASSUMPTION | `N12=発作部位` の形で、仮に決めたことにして考えてもらう（5 個まで） |

4. **作る** を押すと「CARD を作りました」と出ます。**run 2 を開く** を押すと、プロジェクトの画面（`#/project/sci-practice/list/2/task`）でそのカードが開きます。

主なブロック（全部で 16 本）:

| ブロック | すること |
|---|---|
| 🌱 研究の入口（sci-start） | 最初に木全体を埋め始める |
| 💡 候補を増やす（sci-ideate） | 候補を出す |
| ⚖ 比べる（sci-compare） | 候補を比べて推す |
| 🔍 確かめる（sci-probe） | FINDING を確かめる |
| 🔎 外部を探す（sci-search） | 外の論文を探す |
| 📚 Library で答える（sci-lookup） | 手元の論文から答える |
| ✍ 文を書く（sci-draft） | ITEM の文を書く |
| 🗣 議論（sci-debate） | 反対の立場から議論する |

### 11.2 実行する

1. **プロジェクト** タブ → `sci-practice` → **Dashboard** を見ます。常駐（デーモン）が止まっていたら **起動** を押します。
   - 常駐は、カードを実際に動かす裏方のプログラムです。プロジェクトごとに 1 つです。
2. **List** の `todo` の列にある `ID2/sci-ideate` を押し、**▶ 実行** を押します。
3. エージェントが動き始めます。カードは `todo` → `running` → `review` と列を移ります。
   - **cc を初めてこのフォルダで動かすとき**は、エージェントの窓に「このフォルダを信頼しますか」と出ます。「Yes, proceed」を選んで `return` を押してください。
   - エージェントが途中で確かめの質問をしてきたら、**ホーム** の質問の札か、エージェントの窓で答えます。
   - 様子は **会話** タブで見られます。止めたいときは **■ 終了** を押します。
4. 終わると、カードは `review`（確認待ち）に入ります。

**画面のどこが変わるか**

- 研究方針の N4 の板の **行動の履歴** に `#2 sci-ideate` が出ます。
- 自動で入る PROPOSAL（QUESTION など）は、もう N4 の **QUESTION** などに入っています。
- TRIAGE の要る PROPOSAL は、C DEBT の **PENDING_PROPOSALS** の数と、N4 の板の **TRIAGE 待ち** に出ます。一覧の N4 の行には「TRIAGE 2」の札が付きます。

> 常駐を起動せずに体験だけしたいときは、⑰の「エージェントの PROPOSAL をまねる」の命令で、PROPOSAL を手で出せます。

---

## 12. TRIAGE する

1. C DEBT の **PENDING_PROPOSALS** を押します（または **論文** タブの **Dashboard** の要対応から）。住所は `#/papers/triage` になります。
2. 上で **NODE ごと** と **run ごと** を切り替えられます。
3. PROPOSAL 1 つずつに、次のボタンがあります。

| ボタン | すること |
|---|---|
| 採用 | そのまま木に入れる |
| 直して採用 | 中身（題など）を直してから入れる |
| 却下 | 入れない |

4. まとまりごとに **まとめて採用** と **まとめて却下** もあります。
5. 全部 TRIAGE すると「TRIAGE 待ちの PROPOSAL はありません」と出ます。

**画面のどこが変わるか**

- 採用した OPTION が、一覧の N4 のすぐ下に `◇` で並びます。
- DEBT の PENDING_PROPOSALS が 0 に戻ります。

---

## 13. Dashboard と DEBT

**論文** タブ → **Dashboard** を押します。

| 欄 | 中身 |
|---|---|
| 要対応 | AWAITING_DECISION の ITEM と TRIAGE の PROPOSAL。押すと研究方針のその場所が開く |
| 現況 | 木から書き出した `state.md`（目標の 1 文・今の GATE・今の作業・次の一手） |
| 考え | `mind.md`（あなたが書く SOIL（土壌）。ORIGIN・RESEARCH_TYPE・GIVEN・CONSTRAINTS・VALUE_AXES など） |
| メモ | 1 行のメモを足せる |

- **AWAITING_DECISION** は、FINDING や OPTION がそろい、あなたが決める番になった ITEM です。
- `mind.md` の VALUE_AXES に書いたことは、**TABLE**（⑭）の上にも出て、OPTION を比べるときの軸になります。

---

## 14. ROOT_GRAPH と TABLE

研究方針の画面の D で切り替えます。

- **LIST**（一覧）: ここまで使ってきた並び。
- **ROOT_GRAPH**（根の図）: ITEM と OPTION を、左から右へ線でつないだ図です。青い線が親子、灰色の線がほかの関係です。上の **＋** **−** で拡大・縮小、**⤢** で全体を表示、**OPTION を隠す** で OPTION を消せます。図の箱を押すと、その ITEM の板が開きます。
- **TABLE**（表）: 選んだ ITEM の OPTION を、列（状態・支持・反証・論文・FINDING）で比べる表です。ITEM を選んでいないときは、下に出る一覧から ITEM を選びます。別の ITEM を見るときは **LIST** に戻って選び直します。
- **RUN_GRAPH**（実行の図）: 実行を ROOT・SEED・BRANCH の 3 層で並べた図です。

---

## 15. TOMBSTONE（論文を外す）

関係のない論文を、二度と検索に出ないように外します。

1. 板の **LINK** で、論文の行の **外す（TOMBSTONE）** を押します。
2. 理由（必須）を書いて **外す** を押します。

**画面のどこが変わるか**: その論文は LINK から消え、以後の検索結果に出なくなります。外した記録は TOMBSTONE として残ります（⑰の `sci tombstones` で一覧できます）。

論文の READ_STATE ボタンでは、UNREAD・READ・REFLECTED・IRRELEVANT を手で変えられます。

---

## 16. TUI（ターミナルの画面）で見る

ターミナルで次を打ちます。

```bash
cd ~/research/sci-practice
sai-moon-paper-tui
```

1. 左の欄を矢印キーで動かして、一番下の **研究方針** を選び、`return`（または →）で中に入ります。
2. SEED_SENTENCE・GATE・DEBT・ITEM の一覧と、選んだ ITEM の中身が出ます。
3. 使うキー:

| キー | すること |
|---|---|
| ↑ ↓（または k j） | ITEM を選ぶ |
| r | 読み直す（Web で変えたことを映す） |
| o | `tree.md`（木を文章にしたもの）を開く |
| q | 終わる |

TUI の研究方針は**見るだけ**です。変えるときは Web か、次の章の命令を使います。
Web で何かを変えてから TUI で `r` を押し、同じ中身になることを確かめてください。

---

## 17. 同じことを命令（CLI）で行う

画面でできることは、すべて命令でもできます。エージェントもこの命令を使っています。
練習用のフォルダで行ってください。

```bash
cd ~/research/sci-practice
```

| したいこと | 命令 |
|---|---|
| 木の様子を見る | `sai-moon-paper sci status` |
| ITEM の中身を見る | `sai-moon-paper sci node N12` |
| エージェントに見せる SCOPE を見る | `sai-moon-paper sci context --run 2`（⑪ のカードの番号） |
| TRIAGE 待ちの PROPOSAL を見る | `sai-moon-paper sci proposals` |
| TOMBSTONE を見る | `sai-moon-paper sci tombstones` |
| 書き出しを作り直す | `sai-moon-paper sci export` |

木を変える命令は、1 行の JSON を `sai-moon-paper sci apply` に渡します。例として OPTION を 1 つ足します。

```bash
echo '{"op":"add_option","node":"N12","title":"発作の検出"}' | sai-moon-paper sci apply
```

`{"op":"add_option","result":"N29"}` のように出たら成功です。ブラウザの研究方針の画面で ↻ を押すと、N12 の下に OPTION が増えています。

### エージェントの PROPOSAL をまねる

常駐を動かさずに TRIAGE を体験したいときは、次の命令でエージェントの PROPOSAL をまねられます（`--run 2` は ⑪ で作ったカードの番号）。

```bash
echo '{"proposals":[{"op":"add_option","node":"N4","title":"新生児集中治療室の脳波","reason":"代表例の論文の対象"},{"op":"add_question","node":"N4","text":"新生児と成人で発作の見え方はどう違うか","reason":"対象を絞るため"}]}' \
  | sai-moon-paper sci propose --run 2 --block sci-ideate
```

- QUESTION（`add_question`）は `applied`（自動で入った）、OPTION（`add_option`）は `pending`（TRIAGE 待ち）になります。
- ブロックが出せない PROPOSAL（例: `sci-ideate` からの `decide`）は、「このブロックは decide を提案できません」と断られます。

### 書き出されるファイル

| ファイル | 中身 | 手で直すか |
|---|---|---|
| `.sai/paper/state.md` | 現況（Dashboard の「現況」） | 直さない（次の書き出しで上書きされる） |
| `.sai/paper/tree.md` | 木の全体を文章にしたもの（TUI の `o`） | 直さない |
| `.sai/paper/mind.md` | SOIL（Dashboard の「考え」） | **あなたが書く**。エージェントは書き換えない |

---

## 18. 片付け

練習が終わったら片付けます。**消す前に、消すものの名前が `sci-practice` であることを必ず確かめてください。**

1. 常駐を起動していたら、**プロジェクト** タブ → `sci-practice` → Dashboard の **停止** を押します。
2. Web の画面を動かしている窓で `control + C` を押します。
3. 登録簿から `sci-practice` を外します。次の命令は、登録簿の写し（`registry.json.bak`）を取ってから、`sci-practice` の行だけを外します。

```bash
cp ~/.sai-moon/registry.json ~/.sai-moon/registry.json.bak
python3 -c '
import json, pathlib
p = pathlib.Path.home() / ".sai-moon" / "registry.json"
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
| Web にプロジェクトが出ない | 📁 を押して選び直す。無ければ `~/research/sci-practice` で `sai-moon-paper init --sci --type problem` をもう一度打つ（何度打っても壊れない） |
| 画面が古いまま | ↻ を押す。それでも古ければブラウザで再読み込み（`⌘ + R`） |
| 「READ_CARD を作る」で「先に論文を LINK してください」 | ⑤で論文を結ぶ |
| FINDING を送ると断られる | 数字を書いたなら CITE（`#1 abstract` の形）を入れる |
| 決めると断られる | OPTION を選ぶ・理由を書く |
| カードが `todo` から動かない | プロジェクトの Dashboard で常駐が「稼働中」か確かめる。止まっていれば **起動** |
| cc が止まったまま | エージェントの窓に「このフォルダを信頼しますか」が出ていないか確かめる |
| Project タブが 503 | Web を起動するとき、1.2 の `PATH=…` の行を付け忘れている |

---

## 確かめの一覧（全部に印が付けば完了）

- [ ] 練習用プロジェクトを作り、Web の研究方針の画面が開いた（②）
- [ ] SEED_SENTENCE・GATE・DEBT の場所が分かり、GATE の札を押して 7 つの GATE を見た（③）
- [ ] Discover で論文を 1 本入れ、番号を調べた（④）
- [ ] 論文を ITEM に結び、LINK に L1 が出た（⑤）
- [ ] READ_CARD を作り、読み始め、FINDING を足し、終えた。L1 が REFLECTED になった（⑥）
- [ ] OPTION を 2 つ足し、それぞれに SUPPORT と REFUTE の FINDING を足した（⑦）
- [ ] 数字だけで CITE の無い FINDING が断られた（⑦）
- [ ] PURPOSE_SUBGOAL を決め、SEED_SENTENCE に値が入った（⑧）
- [ ] BACKGROUND_GOAL を決め、GATE が 3/7 に進んだ（⑧）
- [ ] 選んだ OPTION を REJECTED にして、ITEM が OPEN に戻った。決定の記録に残った（⑨）
- [ ] QUESTION に答えた・取り下げた。子の ITEM・文を直す・線を引く を試した（⑩）
- [ ] CARD を作り、run を開いた（⑪）
- [ ] PROPOSAL を 採用・直して採用・却下 で TRIAGE した（⑫）
- [ ] Dashboard の要対応・現況・考え を見た（⑬）
- [ ] ROOT_GRAPH と TABLE を見た（⑭）
- [ ] 論文を TOMBSTONE にした（⑮）
- [ ] TUI の研究方針で、Web と同じ中身を見た（⑯）
- [ ] 命令で OPTION を足し、画面に出た（⑰）
- [ ] 片付けた（⑱）
