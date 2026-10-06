# sai アーキテクチャ（開発者向け）

**対象読者:** sai を直す人と、sai-link・sai-paper・sai-sci（Rust）か `apps/radio` に Web 向けの機能を足す人。
使い方は [`README.md`](./README.md)、API と住所の一覧は [`docs/integration.md`](./docs/integration.md)、CLI の約束と Web が読む JSON の項目は [`docs/for-sai-developers.md`](./docs/for-sai-developers.md)、言葉の意味は [`docs/glossary.md`](./docs/glossary.md) を正とする。
ここには、全体の考え方・層の分け方・守る約束・置き場を書く。**正はコード**である。この文書とコードが食い違ったら、コードに合わせて直す。

---

## 1. 3 つの考え

### 1.1 部品 — 3 つの部品と Web の窓口

アプリの名前は `sai`。次の 3 つの部品と、Web の窓口でできている。

| 部品 | クレート・フォルダ | 実行ファイル | 持つもの |
|---|---|---|---|
| **sai-link**（基盤） | `crates/sai-link-engine`・`crates/sai-link-view`・`crates/sai-link-cli` | `sai-link` | 実行の DB（`sai-engine.db`）・常駐（daemon）・ブロック定義の解析と検査・カード・文書（waggle トークン）・Workflow の配置の計算・実行の書き出し |
| **sai-paper**（研究の基盤） | `crates/sai-paper-core`・`crates/sai-paper-cli` | `sai-paper` | 論文の DB（`papr.db`）・検索と取得・BibTeX・Download の後の登録・翻訳・図の抽出・Library の階層・検索セット・音声化の依頼 |
| **sai-sci**（研究の仕組み） | `crates/sai-sci-core`・`crates/sai-sci-cli`・`sai-sci/`（md の部品） | `sai-sci` | 研究方針の木（`papr.db` の `rt_*` 表）・研究の記録・エージェントの報告・研究用の block と template（LabAgent） |
| **sai-web**（Web の窓口） | `apps/web` | `apps/web/bin/sai-web` | 画面（FastAPI + 素の JavaScript）。質問・承認・通知・使用量・MLflow と Radio の中継 |
| （周辺） | `apps/radio` | — | 解説音声の生成と配信（HTTP API・8201 番） |
| （周辺） | `apps/mlflow-server` | — | 実験の記録（MLflow）の compose。Docker で動かす |

- sai-link と sai-paper は、それぞれ**単独で**使える。sai-sci は両方の CLI を使うアプリで、単独では動かない。
- 部品の関係の細部（誰が誰を知っているか）は §2.1 の境界で決める。

### 1.2 人は CLI と Web、エージェントは CLI

- 人は CLI と Web を使い、エージェントは CLI を使う。**画面は Web だけ**（端末の画面は持たない）。
- エージェントは MCP の道具を持たない。`sai-link`・`sai-paper`・`sai-sci`・`sai-web` を Bash で呼ぶ。
  外部の MCP（serena・playwright・donsetch・headroom）は `sai-link init` が `.mcp.json` に書く。
- **入口の中にしか無い処理を作らない。** Web に要る処理が CLI に無ければ、コアのクレートに足して CLI の命令に出す。
  Web のために処理を `apps/web` に写すことはしない（写すと処理が 2 か所になり、片方だけ直る不具合が起きるため）。

### 1.3 Web と CLI の同等

- **CLI でできることは Web でもでき、Web でできることは CLI でもできる。**
- CLI → Web: 3 つの CLI は `schema --json` で命令の一覧を出す。Web の「コマンド」画面（`#/project/<名前>/commands`）は、これを読んで `web: true` の命令を入力欄つきで出す。
  CLI に命令を足せば、Web の画面を足さなくても「コマンド」画面から使える。
- Web → CLI: Web の HTTP API は、すべて `sai-web api <METHOD> <path>` から呼べる。Web にしか無い機能（質問・承認・送信・使用量・MLflow）にも専用の命令がある（`sai-web questions`・`answer`・`approvals`・`approve`・`send`・`usage`・`mlflow …`）。
- Web の専用の画面は、**一覧・図・編集のように専用の見た目が要るときだけ**足す。

### 1.4 層の図

```
┌────────────────────────── 入口 ──────────────────────────┐
│ 人: ブラウザ（iPhone / PC）          人・エージェント: CLI  │
│      │ HTTPS（Tailscale serve :4443）       │               │
│      ▼                                     │               │
│ sai-web  127.0.0.1:8200 FastAPI            │               │
│      │ 3 つの CLI（--json）と HTTP API だけを使う          │
└──────┼─────────────────────────────────────┼───────────────┘
       ▼                                     ▼
 sai-link ◀─────CLI──────── sai-sci ────ライブラリ────▶ sai-paper
 sai-engine.db                (研究の木)                 papr.db
 .sai/outputs                                            ▲
                                  apps/radio (:8201) ◀── HTTP
```

- **コア**（sai-link・sai-paper・sai-sci）は DB と処理を持つ。
- **公開された約束**（3 つの CLI の `--json` と終了コード・`schema --json`・`SAI_HOME/web/runs/*.json` の書き出し・登録簿 `registry.json`・`apps/radio` の HTTP API）が、コアと Web の境目である。Web はここだけを使う。
  約束の中身は [`docs/for-sai-developers.md`](./docs/for-sai-developers.md) に書いてあり、コア側を直す人はそれを守る。
- **研究方針の木（sai-sci）**: 定義の md は `sai-sci/`、処理は `crates/sai-sci-core`、入口は `sai-sci` の CLI と Web の Research タブ。
  木は Q（問い）・O（答えの候補）・F（知見）・E（実験）の Node でできている。木に向けた 1 回の実行が Action（task.md のカード）で、
  professor が研究を回し、`sci-*` の block が研究作業を行う。詳しくは [`sai-sci/README.md`](./sai-sci/README.md)。
- **ブロックの起動時の文は 2 つ**（固定部分と最初の文）。固定部分（template と block.md の本文）は run フォルダの `system-prompt.md` に書き、
  pi は `--append-system-prompt <ファイル>`、cc は `--append-system-prompt-file <ファイル>` で渡す。最初の文は sai-link が自動で作る
  `# context`（`context: true` のとき）と `# task_text`（依頼文）の節だけ。作り方は `docs/for-sai-developers.md` の「起動時の文の分け方」。

---

## 2. 大原則

### 2.1 境界

| 部品 | 知ってよい物 | 知らない物 |
|---|---|---|
| **sai-link** | 自分の DB（`sai-engine.db`）・`.sai/outputs`・登録簿 | sai-paper・sai-sci（呼ばない・読まない） |
| **sai-paper** | 自分の DB（`papr.db`）・`.sai/paper/`・`apps/radio` の HTTP API | sai-link のデータ（`sai-engine.db`・`.sai/outputs`）は**読まない**。sai-link と sai-sci は**呼ばない** |
| **sai-sci** | sai-paper を**ライブラリ**（`sai-paper-core`）で、sai-link を **CLI** で使う | sai-link の DB を開かない |
| **sai-web** | 3 つの CLI と、`apps/radio`・MLflow の HTTP API、登録簿と書き出しのファイル | `sai-engine.db`・`papr.db`・`sai_radio.db` を**開かない** |

- 他の部品の DB の形が変わったときに、黙って壊れるのを防ぐための境界である。
- sai-sci が sai-link の情報（run の一覧・カードの作成）を使うときは、`sai-link run list --json`・`sai-link card create` を呼ぶ。入出力の形は `docs/for-sai-developers.md` の「sai-sci が読む sai-link の出力」。

### 2.2 Cargo の依存の向き

| 向き | 可否 |
|---|---|
| `sai-sci-*` → `sai-paper-core` | よい |
| `sai-sci-*` → `sai-link-*` | 実行ファイルの本体は CLI で呼ぶ（クレートの依存にしない）。テストが block を読むための `dev-dependencies` だけ許す |
| `sai-paper-*` → `sai-sci-*`・`sai-link-*` | **いけない** |
| `sai-link-*` → `sai-paper-*`・`sai-sci-*` | **いけない** |
| `sai-link-cli` → `sai-link-engine`・`sai-link-view` | よい（同じ部品の中） |

### 2.3 3 つの CLI の約束

`sai-link`・`sai-paper`・`sai-sci` は同じ約束を守る（全文は `docs/for-sai-developers.md`）。

1. **終了コード**: 0 成功、2 入力の誤り（clap の使い方の誤りも 2）、3 見つからない、4 状態の衝突、1 その他の失敗。
2. **失敗**: 標準出力には何も書かない。標準エラーの最後の行に、日本語で理由を 1 行書く。clap の誤りは最後の行が `引数の誤り: <clap の文の 1 行目>`。
3. **成功**: `--json` があれば標準出力に JSON を 1 つだけ書く。
4. **進み具合**: 時間のかかる命令は、標準エラーに `sai-progress step=<数>`・`sai-progress msg=<文>` を出してよい。
5. **プロジェクトの決め方**: `SAI_PROJECT_ROOT`（空でない値）→ cwd から上へたどり `.sai/` を持つ最初のフォルダ（ホームとそれより上は数えない）。見つからなければ 2。`init` は「`SAI_PROJECT_ROOT` → cwd」だけ。`schema` はプロジェクトを要らない。
6. **記録の出どころ**: `SAI_ORIGIN`（無ければ `cli`。Web が呼ぶときは `phone`）。
7. **`schema --json`**: 命令の一覧（`bin`・`version`・`commands[]`（`path`・`about`・`stdin`・`json`・`web`・`args[]`））を出す。

### 2.4 そのほかの原則

1. **sai-web は窓口だけ。** 処理とデータの保管はコアがする。
   - sai-web 自身が持つのは、質問・承認・pane の対応表・通知の購読だけ（`$SAI_HOME/web/sai_web.db`・`store.py`）。
   - Download と音声化の進み具合は sai-web のメモリにだけ持つ（再起動で消えてよい。結果はコアに残る）。
2. **足りない機能はコアに足す。** Web に要る処理がコアに無ければ、コアのクレートへ足して CLI の命令に出す。`apps/web` で計算し直さない。
3. **外から来る値は、CLI を呼ぶ前に sai-web が検査する。** 名前の形・大きさ・種類の一覧を sai-web が先に確かめる。通らない値では CLI を 1 度も呼ばない（400）。ファイルのパスは iPhone から受け取らない。置き場はコアが名前から決める。
4. **依存を増やさない。** サーバは FastAPI と標準ライブラリ、画面は素の JavaScript（フレームワーク・ビルド無し）。グラフと Workflow の線は SVG を手で描く。md の色分けも自前の簡易版で行う。
5. **合言葉（`SAI_WEB_TOKEN`）で入口を守る。** 127.0.0.1 でだけ待ち受け、外へは Tailscale serve で tailnet の中にだけ出す。
6. **エラーは日本語でそのまま画面に出す。** 何が起きたか、次に何をすればよいかが分かる文にする。
7. **iPhone と PC の両方を前提にする。** 幅 900px（`ui.js` の `SAI_WIDE_PX`）を境に、狭い画面は下のタブの 1 列、広い画面は左の縦のタブと 2 列にする。
8. **言葉で迷わせない。** 表示は日本語にする（`data-tab` や住所などの識別子は英語のまま）。sai 独自の言葉は `saiTerm` の札で説明する。空の画面には次の一手を出す（`saiEmpty`）。
9. **答えは先着。** 質問と承認は PC と iPhone の両方に同時に出し、先に答えた方を採用する。

---

## 3. 置き場（`SAI_HOME`）

利用者全体で共有する置き場は **`SAI_HOME`**。空でない値があればそれ、無ければ **`~/.sai`**。

### 3.1 決める場所

| 場所 | 規則 |
|---|---|
| sai-link（`sai_link_engine::project::sai_home()`・`crates/sai-link-engine/src/project.rs:146`） | `SAI_HOME` → `~/.sai` → `HOME` も無ければ `None` |
| sai-paper（`sai_paper_core::paths::sai_home()`・`crates/sai-paper-core/src/paths.rs:281`） | 同じ規則の小さなコピー。**値は必ず sai-link と一致させること** |
| sai-sci | 自前の規則を持たず、`sai_paper_core::paths::sai_home()` を使う |
| sai-web（`registry.sai_home()`） | 同じ規則のコピー（Python） |

規則のコピーが 3 つある（sai-link・sai-paper・sai-web）。**どれか 1 つだけ直さないこと。**

### 3.2 `SAI_HOME` に入るもの

| 物 | 何 |
|---|---|
| `registry.json` | プロジェクトの登録簿 |
| `blocks/` | 全プロジェクト共通の block |
| `template/` | 全プロジェクト共通の template |
| `settings.json` | 全プロジェクト共通の settings（プロジェクトの `.sai/settings.json` を**キー単位で上書き**する） |
| `paper.env` | 全プロジェクト共通の API キー（プロジェクトの `.env` より優先順位が低い） |
| `sci/types/<型>.md` | 利用者が保存した研究の型（`sai-sci type-save`） |
| `web/runs/<project_name>/<run_id>.json` | sai-link が書き出す実行の記録。会話の画面が読む |
| `web/cc-rate-limits.json` | Claude Code の制限。書くのは `sai-link cc-statusline`、読むのは Web の使用量 |
| `web/token` | Web の合言葉（0600）。エージェントの道具が読む |
| `web/sai_web.db` | 質問・承認・pane の対応表・通知の購読 |
| `web/vapid.json` | プッシュ通知の鍵 |
| `web/sai-web.log` | Web のログ（launchd で動かしたとき） |

### 3.3 `SAI_HOME` に入らないもの

- **プロジェクトの中の `<root>/.sai/`** は別。`sai-engine.db`・`blocks/`・`template/`・`settings.json`・`outputs/`・`paper/`（sai-paper の置き場）がここにある。
- `~/.pi/agent/models.json`・`~/.claude/projects`・`~/Downloads` などは元の置き場のまま。

### 3.4 切り替え

旧構成（`~/.sai-moon` など）からの移し方と切り替えの順番は [`docs/switch-over.md`](./docs/switch-over.md) にある。

---

## 4. Web の中身（`apps/web`）

### 4.1 サーバ（`apps/web/src/sai_web/`）

| ファイル | 受け持ち |
|---|---|
| `main.py` | FastAPI の住所の登録・合言葉（`require_token`）・Mac 自身からだけ受ける内部 API（`require_local`） |
| `cli.py` | 3 つの CLI を呼ぶ共通の受け口（§4.2）。終了コードを HTTP の状態に直す 1 か所 |
| `control.py` | sai-link の呼び出し（プロジェクト・List・Workflow・定義の編集・カード・文書・会話の検索・常駐）。cwd と `SAI_PROJECT_ROOT` を必ずプロジェクトの root にする |
| `papers.py` | sai-paper の呼び出し（Library・検索・Import・Download・音声化の依頼）と、sai-sci の要対応・プロジェクトの現況 |
| `sci.py` | sai-sci の呼び出し（研究方針の木・提案の仕分け・読む札）と検索セット（sai-paper の `set …`） |
| `commands.py` | 「コマンド」画面。3 つの CLI の `schema --json` を読んで命令の一覧を作り、入力欄から命令を実行する |
| `home.py` | ホームの集計（`GET /api/home`）。プロジェクトごとに並べて呼び、失敗は `errors` に入れる。10 秒覚える |
| `registry.py` | 登録簿 `registry.json` の読み込み（プロジェクトの一覧）と `sai_home()` |
| `runs.py` / `sessions.py` | sai-link の書き出し（`web/runs/`）と会話の記録（.jsonl）の読み取り |
| `questions.py` / `approvals.py` / `store.py` | 質問と cc のコマンド承認の保管と回答（先着） |
| `herdr.py` / `watch.py` / `notify.py` | herdr の呼び出し・2 秒ごとの状態の監視・プッシュ通知 |
| `radio.py` | `apps/radio`（127.0.0.1:8201）への中継 |
| `mlflow_proxy.py` | MLflow（127.0.0.1:5001）への読み取りだけの中継 |
| `rbrowser.py` | 遠隔ブラウザ（Mac の Chrome）の操作 |
| `usage.py` | 使用量（Claude Code・OpenRouter・DeepSeek・pi の記録） |
| `webcli.py` | `sai-web` の命令（`serve`・`projects`・`home`・`questions`・`answer`・`approvals`・`approve`・`send`・`usage`・`mlflow …`・`api`） |
| `extensions/ask_user.ts` | pi に Web 連携付きの質問ツールを足す拡張（既定の接続先は `SAI_WEB_URL`） |
| `hooks/`（`apps/web/hooks/`） | cc のフック（質問・承認・セッションの登録）。何も出力せず、必ず終了コード 0 で終わる |

### 4.2 CLI を呼ぶ約束

- 呼び方: シェルを使わず、引数の配列で渡す。env は `{**os.environ, "SAI_PROJECT_ROOT": <root>, "SAI_ORIGIN": "phone"}`、cwd は root。
  プロジェクトの root は登録簿（`registry.json`）から名前で引く。iPhone からパスは受け取らない。
- 実行ファイルの場所は `SAI_LINK_BIN`・`SAI_PAPER_BIN`・`SAI_SCI_BIN`（既定はそれぞれ `sai-link`・`sai-paper`・`sai-sci`）。
- 検索語や文のように `-` で始まりうる値は、`--` のあとに置いて渡す。
- 終了コードから HTTP の状態への読み替えは `cli.py` の 1 か所:

| CLI の結果 | sai-web の返し方 |
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

### 4.3 「コマンド」画面と `sai-web api`

- `GET /api/projects/{name}/commands` は、3 つの CLI の `schema --json` を 60 秒だけ覚えて、`web: true` の命令と引数だけを返す。`stdin` が `bytes` の命令は出さない（iPhone からファイルを送れないため）。`sai-sci` が無い環境では sai-sci の命令だけが外れる。
- `POST /api/projects/{name}/commands/run`（body は `{"cli": "link|paper|sci", "path": ["card", "run"], "args": {…}, "stdin": "…"}`）は、値を **CLI を呼ぶ前に** schema に照らして検査し（通らなければ 400）、argv を `[bin, *path, <--json>, オプション, フラグ, --, 位置引数]` の順に組んで実行する。失敗の読み替えは §4.2 と同じ。
- `sai-web api <METHOD> <path> [--data <JSON>|--data -]` は、Web のすべての HTTP API をそのまま呼ぶ。合言葉は `Authorization: Bearer` で送り、値は出力・ログに出さない。終了コードは HTTP の状態から 400・422 → 2、404 → 3、409 → 4、そのほか → 1 に直す。

### 4.4 画面（`apps/web/src/sai_web/static/`）

- **枠**: `index.html` が全画面の `<section class="view">` とタブを持ち、JS を決まった順に読む。タブは Home・Projects・Chats・Papers・Research・Usage（`data-tab` は `home`・`project`・`agent`・`papers`・`research`・`usage`）。
- **画面の登録**: 各画面の JS が `window.SaiRoutes` に「住所の照合・描画」を登録し、`app.js` が照合して切り替える。**`app.js` に画面ごとの分岐を足さない。**
- **共通部品**（`ui.js`）: `saiApi`（通信と日本語のエラー）・`saiToast`・`saiConfirm`・`saiPrompt`・`saiSheet`（下から出る板）・`saiEmpty`（空の画面の案内）・`saiTerm`（用語の札。用語は `SAI_TERMS`）・`saiIsWide` / `saiSplitShow`（広い画面の 2 列）。新しい画面はこれを使い、同じ物を作り直さない。
- **md の表示**（`markdown.js`）: `renderMarkdown(text, {toolbar: true})`。コードの色分けは `mdHighlight`。
- **名前の重なり**: 画面ファイルは `<script>` で順に読むので、一番外側の名前が重なると後の物で上書きされる。画面ごとの頭（`sai` 共通・`hm` ホーム・`wf` Workflow・`df` 設定・`pim` Import など）を付け、`apps/web/tests/test_globals.js` で検査する。
- **版**: `sw.js` の `CACHE` と `index.html` の `?v=` をそろえて上げる。上げないと iPhone に古い画面が残る。画面ファイルを足したら `ASSETS` にも足す。

| ファイル | 画面 |
|---|---|
| `app.js` | 切り替えの本体・Chats（日付・実行・メッセージ）・タブ |
| `home.js` | Home |
| `project.js` / `workflow.js` / `linkview.js` / `cardform.js` / `stats.js` / `defs.js` | Projects（一覧・Dashboard・Workflow・List・カードの作成・統計・設定） |
| `commands.js` / `sessionsearch.js` / `dashsearch.js` | 「コマンド」画面・セッション検索・Dashboard の文書検索 |
| `papers.js` / `paperimport.js` / `cite.js` / `rbrowser.js` | Papers（Dashboard・Library・Import・検索・引用・遠隔ブラウザ） |
| `radio.js` / `pcm-worklet.js` / `voiceask.js` | Radio・声の質問・質問の読み上げと音声回答 |
| `scitree.js` / `sciedit.js` / `sciread.js` / `scigraph.js` / `sciset.js` | Research の木（一覧と板・操作と仕分け・読む札・方針の図・検索セットの選別） |
| `mlflow.js` / `mlfchart.js` | Research の Experiments（MLflow の記録） |
| `canvaslayout.js` | 配置・全体表示・拡大の純関数（方針の図と Workflow が使う） |
| `usage.js` | Usage |
| `chatorder.js` | 会話の中に行やカードを置く位置（DOM を使わない関数） |

### 4.5 Research タブの住所

| 住所 | 画面 |
|---|---|
| `#/research/tree` / `#/research/tree/<ID>` | 研究方針の木（Q・O・F・E。`<ID>` は `Q12` の形） |
| `#/research/triage` | 仕分け（提案の採用・却下・取り消し） |
| `#/research/read/<run_id>` | 読む札（1 本の論文を読んで F を書く） |
| `#/research/experiments` ほか（`/e/<ID\|all>`・`/run/<ID>`・`/compare/<ID>,…`） | 実験（MLflow の記録） |

古い住所 `#/papers/tree`・`#/papers/triage`・`#/papers/read/<run_id>`・`#/mlflow…`・`#/papers/canvas`・`#/canvas/…` は、`scitree.js` の登録が `location.replace` で Research の住所へ送る。

### 4.6 処理の流れの型

- **すぐ返る操作**（一覧・中身・保存）: 画面 → `saiApi` → sai-web の住所 → 値の検査 → CLI（`--json`）→ 画面。
- **時間がかかる操作**（Download・音声化・引用の抜き出し・PDF の翻訳）: sai-web が 202 を返し、背景で CLI を走らせる。画面は仕事の一覧の住所を見張り、終わったらプッシュ通知を出す。
  Download は 2 段で、`sai-paper paper download`（PDF を取って登録。ここで「保存しました」を通知）→ `sai-paper paper after-download`（図の抽出・PDF の翻訳。標準エラーの `sai-progress` を読んで進み具合を出す）。
- **エージェントからの質問**: cc のフックと pi の拡張が `POST /api/questions`（Mac 自身からだけ）に登録する → 画面に出て通知 → PC と iPhone の先に答えた方を採用する。

---

## 5. 機能を足すときの手順

1. **処理はどこにあるか。** コア（`sai-link-engine`・`sai-link-view`・`sai-paper-core`・`sai-sci-core`）にあればそれを使う。無ければコアに足す。CLI や `apps/web` に処理を写さない。
2. **CLI の命令に出す。** 該当する CLI（`sai-link`・`sai-paper`・`sai-sci`）に命令を足す。成功は標準出力に JSON 1 個、失敗は決まった終了コードと日本語の理由 1 行（§2.3）。
   命令は `schema --json` に**自動で**出る。Web に出してよい命令には `web` の印を付ける。ここまでで「コマンド」画面から使える。
3. **Web の専用の画面が要るときだけ足す。** `apps/web` の `control.py`・`papers.py`・`sci.py` に関数を書き、値の検査を CLI を呼ぶ前に置く。`main.py` に登録する（固定の住所は `{paper_id:int}` のような変数の住所より先に登録する）。
   テストは偽の CLI（`FakeCli` など）で、検査で CLI が呼ばれないことまで確かめる。
4. **画面を足す。** 新しい JS に `window.SaiRoutes` で登録し、`index.html` に枠と読み込みを足し、`sw.js` の版と `ASSETS` を上げる。`ui.js` の部品を使い、390px と 1280px の両方で確かめる。
5. **文書を直す。** `README.md`（使い方）・この文書（仕組みが変わったとき）・`docs/integration.md`（API と住所）・`docs/for-sai-developers.md`（Web が読む JSON の項目）・`docs/glossary.md`（言葉が増えたとき）。
   Web が読む項目を変えたら、`docs/for-sai-developers.md` と `apps/web` を**一緒に**直す。

---

## 6. テストと確認

テストの命令は [`AGENTS.md`](./AGENTS.md) の「テストと確認」を正とする。種類と場所だけ書く。

| 種類 | 場所 |
|---|---|
| Rust の結合テスト | `crates/*/tests/*.rs`（`cargo test --workspace`） |
| Web の画面テスト | `apps/web/tests/test_*.js`（`node`） |
| Web のサーバテスト | `apps/web/tests/test_*.py`（素の `python`・`PYTHONPATH=apps/web/src`） |
| Radio のテスト | `apps/radio/tests/test_*.py` |
| 実験ループの道具 | `sai-sci/tools/test_*.py`（`unittest`） |

- JS のテストは DOM を使わない関数を確かめる（画面の関数は DOM を使わない部分に分けて書く）。
- 画面の確認は **8200 / 8201 / 8202 番**で行う。**旧構成の 8100 番と 8000 番は、確認のために再起動しない。**
- サーバ・ブラウザ・ネットワークを使う確認の命令には時間の上限を付ける（例 `perl -e 'alarm 180; exec @ARGV' <命令>`）。背景で起動したサーバは最後に止める。
- 書き込みを確かめるときは、一時フォルダを作り、`SAI_HOME` と `SAI_PROJECT_ROOT` をその中に向けて行い、最後に消す。本物の `.sai/`・`$SAI_HOME`・`papr.db` を確かめのために変えない。
- `sai-link daemon start`・`launchctl load`・MLflow の `docker compose up / down / restart` は確認のために実行しない。

---

## 7. ファイル構成

```
sai-moon/
├── crates/                   # Rust のワークスペース（7 クレート）
│   ├── sai-link-engine/      # lib  実行エンジン（DB・常駐・パース・オーケストレーター）
│   ├── sai-link-view/        # lib  Web と CLI が使う組み立て処理（描画は持たない）
│   ├── sai-link-cli/         # bin  sai-link
│   ├── sai-paper-core/       # lib  検索/取得/解析/保存
│   ├── sai-paper-cli/        # bin  sai-paper
│   ├── sai-sci-core/         # lib  研究方針の木・研究の記録・報告の検査
│   └── sai-sci-cli/          # bin  sai-sci
├── waggle/                   # 独立 [workspace] の vendor クレート群（exclude 済み）
├── apps/
│   ├── web/                  # Web の窓口（8200）
│   │   ├── bin/sai-web       # sai-web の CLI（apps/web/.venv の Python で動く）
│   │   ├── src/sai_web/      # サーバ（§4.1）
│   │   │   ├── static/       # 画面（§4.4）
│   │   │   └── extensions/   # pi の拡張（ask_user.ts）
│   │   ├── hooks/            # cc のフック
│   │   ├── launchd/          # launchd のひな形
│   │   └── tests/            # test_*.js（node）と test_*.py（素の python）
│   ├── radio/                # Radio の裏方サーバー（8201・TTS プロキシは 8202）
│   │   ├── src/sai_radio/
│   │   ├── config/           # podcast-creator 用の設定 JSON
│   │   └── tests/
│   └── mlflow-server/        # MLflow の compose（postgres・minio・mlflow・caddy。Docker で動かす）
├── sai-sci/                  # 研究の仕組みの md の部品（`sai-sci` に埋め込まれる。直したら release を作り直す）
│   ├── types/                # 研究の型（Q の階層の雛形）
│   ├── guides/               # ガイド（Q の文・Child Q・Extract・Pitfall・目安）
│   ├── template/             # 研究向けの共通 template（`sai-agent`・`sai-task`・`sai-cli` ほか）
│   ├── blocks/               # block（professor・professor-backward・operator・kaizen と `sci-*`）
│   ├── tools/                # 実験ループの台帳・実行先への分配器・参照コードの道具・データ分析の道具とテスト
│   ├── exec/                 # 実行先の見本（local・ssh・dok・_example）
│   └── mind.md               # mind.md（研究の土台）のひな形
├── docs/                     # 開発者向けの文書
│   ├── integration.md        # Web の API と画面の住所・各 API が呼ぶ CLI の命令
│   ├── for-sai-developers.md # CLI の約束と Web が読む JSON の項目
│   ├── glossary.md           # 用語集
│   ├── switch-over.md        # 旧構成から切り替える手順書
│   └── archive/              # 過去の記録
├── .sai/                     # このプロジェクト自身の中のデータ（.gitignore 済み）
├── AGENTS.md                 # このリポジトリで作業するエージェントへの指示
├── CLAUDE.md                 # @AGENTS.md の 1 行
├── README.md                 # 概要・入れ方・最初の一歩
└── ARCHITECTURE.md           # この文書
```

---

## 8. 入れていないもの・やめたもの

| 物 | 状況 |
|---|---|
| **端末の画面（TUI）** | やめた。画面は Web だけ。TUI にしか無かった処理は、先にコア（`sai-link-view`・`sai-paper-core`）へ移してある |
| **MCP の `sai`**（エージェントの道具） | やめた。エージェントは CLI を Bash で呼ぶ。外部の MCP（serena・playwright・donsetch・headroom）は残る |
| **`phone …` の命令**（Web 専用の JSON の出口） | やめた。Web は普通の命令を `--json` で呼ぶ（`docs/for-sai-developers.md`） |
| **`lib/`**（参考リポジトリ置き場） | 作っていない |
| **global の cc フック・pi 拡張** | `~/.claude/settings.json` と `~/.pi/agent/extensions/` には**入れない**。`sai-link init` がプロジェクトの `.claude/settings.json`・`.pi/settings.json` に書く |
| **単一ディスパッチャ CLI** | 「1 つの `sai` コマンドですべてをまとめる」案は不採用。`sai-link`・`sai-paper`・`sai-sci`・`sai-web` の 4 本に分けたまま。必要になれば後からまとめる入口を足せる |
| **Rust のプリセット領域** | 旧 sai-paper からそのまま写した物は、sai 用の見直しをしていない |

## 9. 経緯

- 2026-09-27: 旧 4 リポジトリ（`sai-link`・`sai-paper`・`sai-phone`・`sai-radio`）を 1 つにまとめた（バイナリ名は `sai-moon-*`、置き場は `~/.sai-moon`、Web は `apps/phone`）。
- 2026-10: 名前と境界を整理した。アプリの名前を `sai` にし、sai-link・sai-paper・sai-sci・sai-web の 4 つに分けた。
  TUI・MCP・`phone` 命令をやめ、3 つの CLI に共通の約束（終了コード・`--json`・`schema`）を置いた。Web は `apps/web`、置き場は `SAI_HOME`（`~/.sai`）、実行ファイルは `sai-link`・`sai-paper`・`sai-sci`・`sai-web` になった。
  Web に Research タブ（研究方針の木と実験）と「コマンド」画面を足した。旧構成からの切り替えは [`docs/switch-over.md`](./docs/switch-over.md)。

## 10. 次に回すもの

- sai-link: 会話の再開・Model の画面。
- sai-paper: Tags・Authors・DOI / arXiv ID での取り込み・論文を block に渡すカード。
- 統合: iPhone からの Tailscale 公開の手順の実機確認。
