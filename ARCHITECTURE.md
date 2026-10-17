# agent-link アーキテクチャ（開発者向け）

**対象読者:** agent-link を直す人と、Rust のコア（`agent-link`）に Web 向けの機能を足す人。
使い方は [`README.md`](./README.md)、API と住所の一覧は [`docs/integration.md`](./docs/integration.md)、CLI の約束と Web が読む JSON の項目は [`docs/for-developers.md`](./docs/for-developers.md)、概念と用語の意味は [`CONCEPT.md`](./CONCEPT.md) を正とする。
ここには、全体の考え方・層の分け方・守る約束・置き場を書く。**正はコード**である。この文書とコードが食い違ったら、コードに合わせて直す。

由来（sai-tree から複製した経緯）は [`README.md`](./README.md) の冒頭に書いた。要件定義書は `docs/plan/requirements.md`。

---

## 1. 3 つの考え

### 1.1 部品 — 実行エンジンと Web の窓口

アプリの名前は `agent-link`。次の 2 つの部品でできている。

| 部品 | クレート・フォルダ | 実行ファイル | 持つもの |
|---|---|---|---|
| **実行エンジン**（基盤） | `crates/agent-link-engine`・`crates/agent-link-view`・`crates/agent-link-cli` | `agent-link` | 実行の DB（`agent-link.db`）・常駐（daemon）・ブロック定義の解析と検査・カード・文書（waggle トークン）・Workflow の配置の計算・実行の書き出し |
| **Web の窓口** | `apps/web` | `apps/web/bin/agent-link-web` | 画面（FastAPI + 素の JavaScript）。質問・通知・Claude の使用量 |

- `waggle/` は文書のトークンを扱う vendor クレート群で、原則編集しない。
- 部品の関係の細部（誰が誰を知っているか）は §2.1 の境界で決める。

### 1.2 人は CLI と Web、エージェントは CLI

- 人は CLI と Web を使い、エージェントは CLI を使う。**画面は Web だけ**（端末の画面は持たない）。
- エージェントは MCP の道具を持たない。`agent-link`・`agent-link-web` を Bash で呼ぶ。
  外部の MCP（serena・playwright・donsetch・headroom）は `agent-link init` が `.mcp.json` に書く。
- **入口の中にしか無い処理を作らない。** Web に要る処理が CLI に無ければ、コアのクレートに足して CLI の命令に出す。
  Web のために処理を `apps/web` に写すことはしない（写すと処理が 2 か所になり、片方だけ直る不具合が起きるため）。

### 1.3 Web と CLI の同等

- **Web の HTTP API は `agent-link-web api <METHOD> <path>` で呼べる。CLI の命令は `agent-link schema --json` に出る。** 人もエージェントも、画面を開かずに同じことをコマンドで行える。
- Web にしか無い機能（質問・送信・Claude の使用量）にも専用の命令がある（`agent-link-web questions`・`answer`・`send`・`usage`）。
- Web の専用の画面は、**一覧・図・編集のように専用の見た目が要るときだけ**足す。

### 1.4 層の図

開発（Mac・単独モード）:

```
┌──────────────────── 入口 ────────────────────┐
│ 人: ブラウザ（iPhone / PC）       人・エージェント: CLI │
│      │ HTTPS（iPhone だけ Tailscale serve）  │         │
│      ▼                                      │         │
│ agent-link-web  127.0.0.1:8400 FastAPI      │         │
│      │ agent-link の CLI（--json）だけを使う            │
└──────┼──────────────────────────────────────┼─────────┘
       ▼                                      ▼
   agent-link（実行エンジン）
   .agent-link/engine/agent-link.db
   .agent-link/outputs
```

- **コア**（agent-link）は DB と処理を持つ。
- **公開された約束**（`agent-link` の `--json` と終了コード・`schema --json`・`AGENT_LINK_HOME/web/runs/*.json` の書き出し・登録簿 `registry.json`）が、コアと Web の境目である。Web はここだけを使う。
  約束の中身は [`docs/for-developers.md`](./docs/for-developers.md) に書いてあり、コア側を直す人はそれを守る。
- **ブロックの起動時の文は 2 つ**（固定部分と最初の文）。固定部分（template と block.md の本文）は run フォルダの `system-prompt.md` に書き、
  cc は `--append-system-prompt-file <ファイル>` で渡す。最初の文は agent-link が自動で作る
  `# context`（`context: true` のとき）と `# task_text`（依頼文）の節だけ。作り方は `docs/for-developers.md` の「起動時の文の分け方」。

---

## 2. 大原則

### 2.1 境界

| 部品 | 知ってよい物 | 知らない物 |
|---|---|---|
| **実行エンジン**（`agent-link`） | 自分の DB（`agent-link.db`）・`.agent-link/outputs`・登録簿 | Web |
| **Web の窓口**（`agent-link-web`） | `agent-link` の CLI と、登録簿と書き出しのファイル | `agent-link.db` を**開かない** |

- 他の部品の DB の形が変わったときに、黙って壊れるのを防ぐための境界である。

### 2.2 Cargo の依存の向き

| 向き | 可否 |
|---|---|
| `agent-link-cli` → `agent-link-engine`・`agent-link-view` | よい（同じ部品の中） |
| `agent-link-view` → `agent-link-engine` | よい |
| `agent-link-engine` → `agent-link-cli`・`agent-link-view` | **いけない** |

### 2.3 CLI の約束

`agent-link` は次の約束を守る（全文は `docs/for-developers.md`）。

1. **終了コード**: 0 成功、2 入力の誤り（clap の使い方の誤りも 2）、3 見つからない、4 状態の衝突、1 その他の失敗。
2. **失敗**: 標準出力には何も書かない。標準エラーの最後の行に、英語で理由を 1 行書く。clap の誤りは最後の行が `Invalid arguments: <clap の文の 1 行目>`。`--json` の返り値のうち人が読む文（`message` など）も英語。
3. **成功**: `--json` があれば標準出力に JSON を 1 つだけ書く。Web が付けるフラグは CLI が必ず受け付ける。
4. **進み具合**: 時間のかかる命令は、標準エラーに `sai-progress step=<数>`・`sai-progress msg=<文>` を出してよい。
5. **プロジェクトの決め方**: `AGENT_LINK_PROJECT_ROOT`（空でない値）→ cwd から上へたどり `.agent-link/` を持つ最初のフォルダ（ホームとそれより上は数えない）。見つからなければ 2。`init` は「`AGENT_LINK_PROJECT_ROOT` → cwd」だけ。`schema` はプロジェクトを要らない。
6. **`schema --json`**: 命令の一覧（`bin`・`version`・`commands[]`（`path`・`about`・`stdin`・`json`・`web`・`args[]`））を出す。`web` の印は出力に残っているが、Web の画面は読まない。

### 2.4 そのほかの原則

1. **Web は窓口だけ。** 処理とデータの保管はコアがする。
   - Web 自身が持つのは、質問と通知の購読だけ（`$AGENT_LINK_HOME/web/agent_link_web.db`・`store.py`）。
2. **足りない機能はコアに足す。** Web に要る処理がコアに無ければ、コアのクレートへ足して CLI の命令に出す。`apps/web` で計算し直さない。
3. **外から来る値は、CLI を呼ぶ前に Web が検査する。** 名前の形・大きさ・種類の一覧を Web が先に確かめる。通らない値では CLI を 1 度も呼ばない（400）。ファイルのパスは iPhone から受け取らない。置き場はコアが名前から決める。
4. **依存を増やさない。** サーバは FastAPI と標準ライブラリ、画面は素の JavaScript（フレームワーク・ビルド無し）。グラフと Workflow の線は SVG を手で描く。md の色分けも自前の簡易版で行う。
5. **入口を守る。** 合言葉は無い。127.0.0.1 でだけ待ち受け、フックの住所（`POST /api/register` など）は `require_internal` で Mac 自身（127.0.0.1 / ::1）だけに絞る。Tailscale serve は、tailnet が自分だけのときだけ使う（他の人が届くと全機能を使われる。`docs/switch-over.md` の 6）。
6. **遠隔の機械からは門番だけが入る。** 遠隔の機械（`task create` の `host`）からは、強制コマンドの鍵（`~/.ssh/authorized_keys` の `command="… agent-link remote-gate",restrict …`）で `agent-link remote-gate` だけが入る。通す命令は `crates/agent-link-engine/src/remote.rs` の `gate_plan` が決める（フックと `doc`・`task report` だけ）。手順は `docs/remote.md`。
7. **エラーは英語でそのまま画面に出す。** 何が起きたか、次に何をすればよいかが分かる文にする。CLI の標準エラーの最後の行と Web の `detail` は英語で書く（文書とエージェント向けの文は日本語のまま）。
8. **iPhone と PC の両方を前提にする。** 幅 900px（`ui.js` の `AGENT_LINK_WIDE_PX`）を境に、狭い画面は下のタブの 1 列、広い画面は左の縦のタブと 2 列にする。
9. **言葉で迷わせない。** 画面の文言は英語にする（音声の言語は日本語。`data-tab` や住所などの識別子も英語）。独自の言葉は `saiTerm` の札で説明する。空の画面には Next Action を出す（`saiEmpty`）。
10. **答えは先着。** 質問は PC と iPhone の両方に同時に出し、先に答えた方を採用する。

---

## 3. 置き場（`AGENT_LINK_HOME`）

利用者全体で共有する置き場は **`AGENT_LINK_HOME`**。空でない値があればそれ、無ければ **`~/.agent-link`**。
古い置き場（sai-moon の物）は読まない。古いデータを読みに行く互換の処理は無い。

### 3.1 決める場所

| 場所 | 規則 |
|---|---|
| 実行エンジン（`agent_link_engine::project::sai_home()`・本体は同じファイルの `resolve_sai_home`・`crates/agent-link-engine/src/project.rs`） | `AGENT_LINK_HOME` → `~/.agent-link` → `HOME` も無ければ `None` |
| Web（`registry.sai_home()`・`apps/web/src/agent_link_web/registry.py`） | 同じ規則のコピー（Python） |

規則のコピーが 2 つある。**どちらか 1 つだけ直さないこと。**

### 3.2 `AGENT_LINK_HOME` に入るもの

| 物 | 何 |
|---|---|
| `registry.json` | プロジェクトの登録簿 |
| `blocks/` | 全プロジェクト共通の block |
| `template/` | 全プロジェクト共通の template |
| `settings.json` | 全プロジェクト共通の settings（プロジェクトの `.agent-link/settings.json` を**キー単位で上書き**する） |
| `web/runs/<project_name>/<run_id>.json` | 実行エンジンが書き出す実行の記録。会話の画面が読む |
| `web/agent_link_web.db` | 質問・通知の購読 |
| `web/vapid.json` | プッシュ通知の鍵 |
| `web/agent-link-web.log` | Web のログ（README の `agent-link-web-restart` で起こしたときの出し先。`agent-link-web serve` を直接起こすと標準出力・標準エラーに出る） |

### 3.3 `AGENT_LINK_HOME` に入らないもの

- **プロジェクトの中の `<root>/.agent-link/`** は別。`engine/agent-link.db`（同じフォルダに `daemon.pid`・`daemon.status`・`daemon.log`・`notifications/`・`waggle-blobs/`）・`blocks/`・`template/`・`settings.json`・`outputs/` がここにある。`outputs/` の中は `todo/`・`running/`・`review/` と、Workflow に移した run の日付フォルダ `<yymmdd>/`（日付は一番上の親 run の実行開始日・ローカル時刻。子の run は親の日付フォルダに入る）。
- `~/.claude/projects` などは元の置き場のまま。

---

## 4. Web の中身（`apps/web`）

### 4.1 サーバ（`apps/web/src/agent_link_web/`）

| ファイル | 受け持ち |
|---|---|
| `main.py` | FastAPI の住所の登録・内部用の住所を Mac 自身（127.0.0.1 / ::1）だけに絞る守り（`require_internal`） |
| `cli.py` | `agent-link` を呼ぶ共通の受け口（§4.2）。終了コードを HTTP の状態に直す 1 か所 |
| `control.py` | `agent-link` の呼び出し（プロジェクト・List・Workflow・定義の編集・カード・文書・会話の検索・常駐）。cwd と `AGENT_LINK_PROJECT_ROOT` を必ずプロジェクトの root にする |
| `home.py` | ホームの集計（`GET /api/home`）。プロジェクトごとに並べて呼び、失敗は `errors` に入れる。10 秒覚える |
| `registry.py` | 登録簿 `registry.json` の読み込み（プロジェクトの一覧）と `sai_home()` |
| `runs.py` / `sessions.py` | 実行エンジンの書き出し（`web/runs/`）と会話の記録（.jsonl）の読み取り |
| `questions.py` / `store.py` | 質問の保管と回答（先着）・通知の購読 |
| `watch.py` / `notify.py` | 2 秒ごとの状態の監視（headless の state.json を読む）・プッシュ通知 |
| `usage.py` | Claude の使用量（cc の 5 時間枠・7 日枠と費用。`GET /api/usage` が `{"cc", "saved_at"}` を返す） |
| `config.py` | 設定（環境変数と `apps/web/.env`） |
| `webcli.py` | `agent-link-web` の命令（`serve`・`projects`・`home`・`questions`・`question`・`answer`・`send`・`usage`・`api`）。HTTP の状態を終了コードに直す |
| `hooks/`（`apps/web/hooks/`） | cc のフック（質問・セッションの登録）。何も出力せず、必ず終了コード 0 で終わる |

### 4.2 CLI を呼ぶ約束

- 呼び方: シェルを使わず、引数の配列で渡す。env は `{**os.environ, "AGENT_LINK_PROJECT_ROOT": <root>}`、cwd は root。
  プロジェクトの root は登録簿（`registry.json`）から名前で引く。iPhone からパスは受け取らない。
- 実行ファイルの場所は `AGENT_LINK_BIN`（既定は `agent-link`）。
- 検索語や文のように `-` で始まりうる値は、`--` のあとに置いて渡す。
- 終了コードから HTTP の状態への読み替えは `cli.py` の 1 か所:

| CLI の結果 | Web の返し方 |
|---|---|
| 終了コード 0 で標準出力が JSON | 200（JSON をそのまま返す） |
| 終了コード 0 で標準出力が JSON でない | 502「…の応答を読めませんでした」 |
| 終了コード 2（入力の誤り） | 400 |
| 終了コード 3（見つからない） | 404 |
| 終了コード 4（状態の衝突） | 409 |
| 終了コード 1 とそのほか | 502 |
| CLI が起動できない | 503（設定の場所を確かめる文） |
| 時間切れ | 504 |

`detail` は標準エラーの空でない最後の行（`sai-progress` で始まる行は除く）。無ければ「<CLI 名> が失敗しました」。

### 4.3 `agent-link-web api`

- `agent-link-web api <METHOD> <path> [--data <JSON>|--data -]` は、Web のすべての HTTP API をそのまま呼ぶ。接続先は `AGENT_LINK_WEB_URL`（既定 `http://127.0.0.1:8400`）。終了コードは HTTP の状態から 400・422 → 2、404 → 3、409 → 4、そのほか → 1 に直す。
- CLI の命令の一覧は `agent-link schema --json`（プロジェクトを要らない）。

### 4.4 画面（`apps/web/src/agent_link_web/static/`）

- **枠**: `index.html` が全画面の `<section class="view">` とタブを持ち、JS を決まった順に読む。タブは Home・Projects・Chats（`data-tab` は `home`・`project`・`agent`）。Claude の使用量は Home のカード。
- **画面の登録**: 各画面の JS が `window.SaiRoutes` に「住所の照合・描画」を登録し、`app.js` が照合して切り替える。**`app.js` に画面ごとの分岐を足さない。**
- **共通部品**（`ui.js`）: `saiApi`（通信とエラーの表示）・`saiToast`・`saiConfirm`・`saiPrompt`・`saiSheet`（下から出る板）・`saiEmpty`（空の画面の案内）・`saiTerm`（用語の札。用語は `AGENT_LINK_TERMS`）・`saiIsWide` / `saiSplitShow`（広い画面の 2 列）。新しい画面はこれを使い、同じ物を作り直さない。
- **md の表示**（`markdown.js`）: `renderMarkdown(text, {toolbar: true})`。コードの色分けは `mdHighlight`。
- **名前の重なり**: 画面ファイルは `<script>` で順に読むので、一番外側の名前が重なると後の物で上書きされる。画面ごとの頭（`sai` 共通・`hm` ホーム・`wf` Workflow・`df` 設定など）を付け、`apps/web/tests/test_globals.js` で検査する。
- **版**: `sw.js` の `CACHE` と `index.html` の `?v=` をそろえて上げる。上げないと iPhone に古い画面が残る。`apps/web/tests/test_migrate.js` が版の数字を固定しているので、これも一緒に上げる。画面ファイルを足したら `ASSETS` にも足し、消したら `ASSETS` からも外す。
- **再起動**: 静的ファイルは Web の再起動が要らない。`apps/web/src/agent_link_web/*.py` を直したら Web の再起動が要る。

| ファイル | 画面 |
|---|---|
| `app.js` | 切り替えの本体・Chats（日付・実行・メッセージ）・タブ |
| `home.js` | Home（質問・todo・合計・最近・Claude のカード） |
| `project.js` / `workflow.js` / `linkview.js` / `cardform.js` / `stats.js` / `defs.js` | Projects（一覧・Dashboard・Workflow・List・カードの作成・統計・設定） |
| `sessionsearch.js` | セッション検索（会話の検索と、文書（task / message / waggle）の本文検索を同じ画面で） |
| `newchat.js` | Chats の新しいチャット（生成中の文・中断） |
| `voiceask.js` | 質問の読み上げと音声回答 |
| `canvaslayout.js` | 配置・全体表示・拡大の純関数（Workflow が使う） |
| `chatorder.js` | 会話の中に行やカードを置く位置（DOM を使わない関数） |

### 4.5 処理の流れの型

- **すぐ返る操作**（一覧・中身・保存）: 画面 → `saiApi` → Web の住所 → 値の検査 → CLI（`--json`）→ 画面。
- **エージェントからの質問**: cc のフックが `POST /api/questions`（Mac 自身から）に登録する → 画面に出て通知 → PC と iPhone の先に答えた方を採用する。

---

## 5. 機能を足すときの手順

1. **処理はどこにあるか。** コア（`agent-link-engine`・`agent-link-view`）にあればそれを使う。無ければコアに足す。CLI や `apps/web` に処理を写さない。
2. **CLI の命令に出す。** `agent-link` に命令を足す。成功は標準出力に JSON 1 個、失敗は決まった終了コードと英語の理由 1 行（§2.3）。
   命令は `schema --json` に**自動で**出る。ここまでで、CLI から使える。
3. **Web の専用の画面が要るときだけ足す。** `apps/web` の `control.py` に関数を書き、値の検査を CLI を呼ぶ前に置く。`main.py` に登録する。
   テストは偽の CLI で、検査で CLI が呼ばれないことまで確かめる。
4. **画面を足す。** 新しい JS に `window.SaiRoutes` で登録し、`index.html` に枠と読み込みを足し、`sw.js` の版と `ASSETS` を上げる。`ui.js` の部品を使い、390px と 1280px の両方で確かめる。画面の文言は英語にする。
5. **文書を直す。** `README.md`（使い方）・この文書（仕組みが変わったとき）・`docs/integration.md`（API と住所）・`docs/for-developers.md`（Web が読む JSON の項目）・`CONCEPT.md`（概念と用語が増えたとき）。
   Web が読む項目を変えたら、`docs/for-developers.md` と `apps/web` を**一緒に**直す。

---

## 6. テストと確認

テストの命令は [`AGENTS.md`](./AGENTS.md) の「テストと確認」を正とする。種類と場所だけ書く。

| 種類 | 場所 |
|---|---|
| Rust の結合テスト | `crates/*/tests/*.rs`（`cargo test --workspace`） |
| Web の画面テスト | `apps/web/tests/test_*.js`（`node`） |
| Web のサーバテスト | `apps/web/tests/test_*.py`（素の `python`・`PYTHONPATH=apps/web/src`） |

- JS のテストは DOM を使わない関数を確かめる（画面の関数は DOM を使わない部分に分けて書く）。
- 画面の確認は **8297 番など別の番号**で行う。**Mac で動いている Web（8400 番）は、確認のために止めたり再起動したりしない。**
- サーバ・ブラウザ・ネットワークを使う確認の命令には時間の上限を付ける（例 `perl -e 'alarm 180; exec @ARGV' <命令>`）。背景で起動したサーバは最後に止める。
- 書き込みを確かめるときは、一時フォルダを作り、`AGENT_LINK_HOME` と `AGENT_LINK_PROJECT_ROOT` をその中に向けて行い、最後に消す。本物の `.agent-link/`・`$AGENT_LINK_HOME` を確かめのために変えない。
- `agent-link daemon start` の本物の実行は、確認のために実行しない。

---

## 7. ファイル構成

```
agent-link/
├── crates/                   # Rust のワークスペース（3 クレート）
│   ├── agent-link-engine/    # lib  実行エンジン（DB・常駐・パース・オーケストレーター）
│   ├── agent-link-view/      # lib  Web と CLI が使う組み立て処理（描画は持たない）
│   └── agent-link-cli/       # bin  agent-link
├── waggle/                   # 独立 [workspace] の vendor クレート群（exclude 済み）
├── apps/
│   └── web/                  # Web の窓口（8400）
│       ├── bin/agent-link-web  # Web の CLI（apps/web/.venv の Python で動く）
│       ├── src/agent_link_web/ # サーバ（§4.1）
│       │   └── static/       # 画面（§4.4）
│       ├── hooks/            # cc のフック（cc_question.py・cc_register.py）
│       └── tests/            # test_*.js（node）と test_*.py（素の python）
├── docs/                     # 開発者向けの文書
│   ├── integration.md        # Web の API と画面の住所・各 API が呼ぶ CLI の命令
│   ├── for-developers.md     # CLI の約束と Web が読む JSON の項目
│   ├── switch-over.md        # sai-moon と並べて使う手順書
│   ├── remote.md             # 遠隔の機械（wsl-server など）で子の claude を動かす手順書
│   └── plan/                 # 実装計画書。読むだけ
├── .agent-link/              # このプロジェクト自身の中のデータ（.gitignore 済み）
├── CONCEPT.md                # 概念と用語の定義
├── AGENTS.md                 # このリポジトリで作業するエージェントへの指示
├── CLAUDE.md                 # @AGENTS.md の 1 行
├── README.md                 # 概要・入れ方・最初の一歩
└── ARCHITECTURE.md           # この文書
```

---

## 8. 入れていないもの・やめたもの

| 物 | 状況 |
|---|---|
| **端末の画面（TUI）** | やめた。画面は Web だけ。TUI にしか無かった処理は、先にコア（`agent-link-view`）へ移してある |
| **MCP の `agent-link`**（エージェントの道具） | やめた。エージェントは CLI を Bash で呼ぶ。外部の MCP（serena・playwright・donsetch・headroom）は残る |
| **global の cc フック** | `~/.claude/settings.json` にもプロジェクトの `.claude/settings.json` にも**入れない**。`.agent-link/cc-settings.json` に書き、run の cc に `--settings` で渡す（sai-moon と同じプロジェクトで取り合わないため） |
| **Mac の常駐（launchd）・herdr・cc のコマンド承認・遠隔ブラウザ・5 時間枠・pi** | 持たない。daemon と Web は手で起こす。headless の実行（擬似 pane）で動く。engine の `herdr.rs` は擬似 pane の型などの名残 |
| **論文・研究（Paper・Research）・Radio・MLflow・sai-ts・sai-prep** | 持たない（sai-tree の機能） |
| **多人数モード・サーバ（ログイン・コンテナ）・Customer サポート** | 持たない（sai-tree の機能） |
| **runner の選択（`runner: user`・`agent-link card start\|finish`・gl・pi）・provider・tag** | やめた。runner は cc だけ。block 定義の `tag`・`provider`・`round` と settings.json の `default_provider` も使わない。古い `runner`・`provider`・`tag` の項目は読み飛ばし、書き出さない |
| **round・global_round** | やめた。Workflow に移した run は `<yymmdd>/`（日付フォルダ）に入る。`run list`・`run log`・`session search`（`-R`・`-r`）の round 系の項目も無い |
| **done（状態）** | やめた。流れは todo → running → review →（`agent-link run advance`）→ Workflow。Kanban は 3 レーン（`todo`・`running`・`review`） |
| **Account タブ・Usage の詳細** | やめた。Claude の使用量は Home のカード（`GET /api/usage` は `{"cc", "saved_at"}`） |
| **OpenRouter・DeepSeek・pi の使用量と環境変数** | やめた（`AGENT_LINK_WEB_OPENROUTER_ACCOUNTS`・`PI_SESSIONS_DIR` も無い） |
| **合言葉**（`AGENT_LINK_WEB_TOKEN`・`$AGENT_LINK_HOME/web/token`） | やめた。入口の守りは 127.0.0.1 の待ち受けと `require_internal` だけ |
| **「コマンド」画面とその API**（`/api/projects/{name}/commands[/run]`） | やめた。`agent-link schema --json` は残る |
| **review の記入判定（reflection）** | やめた。Workflow の札と Kanban に「未記入」の印は無い |
| **古い記録の読み替え** | しない。古い記録のフォルダや古い置き場を読みに行く処理は無い（agent-link は新しい `.agent-link/` から始める） |

範囲外:

- sai-tree プロジェクトの古い記録（run 1〜289）を agent-link に移すこと。
- agent-link を Linux に置くこと。
- sai-tree で直した実行エンジンを agent-link に移す仕組み（手で移す）。

---

## 9. 経緯

- sai-tree（研究室の Linux 多人数サーバ）を複製して、agent-link を作った（要件定義書 `docs/plan/requirements.md`）。sai-moon・sai-tree は読み取り専用で、直さない。
- 作業は U01〜U07 の計画書（`docs/plan/unit-U*.md`）の順に進めた。U01〜U04 が論文・研究・多人数モード・周辺アプリの削除、U05・U06 が名前の変更（Rust・Web）、U07 が文書・CI と切り替えの手順書。
- そのあと簡素化を行った（`docs/plan/` の簡素化の計画書）。cc だけにする・合言葉と Account をなくす・done と round をなくして日付フォルダにする・Workflow の子の表示の切り替え・画面と CLI の文を英語にする、を行い、文書を合わせた。
