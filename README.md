# sai

**Mac 上の coding agent と論文の作業を、ブラウザ（iPhone と PC）とコマンドで使うためのアプリ**です。

`sai` は 3 つの部品と Web の窓口でできています。

| 部品 | 何 | 実行ファイル |
|---|---|---|
| **sai-link** | 基盤。coding agent の実行・カード（実行の予約）・常駐・文書（waggle トークン） | `sai-link` |
| **sai-paper** | 研究の基盤。論文の検索・取得・翻訳・音声化・Library・検索セット（CLI の `sai-paper set`。エージェントの選別に使う） | `sai-paper` |
| **sai-sci** | 研究の仕組み。research-tree・研究の記録・エージェントの報告。sai-link と sai-paper の CLI を使うアプリ | `sai-sci` |
| **sai-web** | Web の窓口（FastAPI + 素の JavaScript）。上の 3 つの CLI と Radio の HTTP API だけを使う | `apps/web/bin/sai-web` |

```
人 ──── ブラウザ ───▶ sai-web (:8200) ──┐
 │                                       ├─▶ sai-link ─▶ sai-engine.db
 └──── CLI ──────────────────────────────┤   sai-paper ─▶ papr.db
エージェント ──── CLI（Bash）─────────────┘   sai-sci   ─▶ sai-link・sai-paper を呼ぶ
                                  sai-web ─▶ apps/radio (:8201)・MLflow (:5001)
```

- 人は **CLI と Web** を使い、エージェントは **CLI** を使います（エージェントに MCP の道具は渡しません。Bash で CLI を呼びます）。
- 画面は Web だけです。CLI でできることは Web でもでき、Web でできることは CLI（`sai-web`）でもできます。
- sai-link と sai-paper は、それぞれ**単独で**使えます。sai-sci は両方の上に載る研究用のアプリです。

仕組みの詳しい説明は [`ARCHITECTURE.md`](./ARCHITECTURE.md)、概念と用語は [`CONCEPT.md`](./CONCEPT.md) にあります。

## 必要な物

| 物 | 用途 |
|---|---|
| macOS | 動作の前提（常駐・launchd・Chrome の遠隔操作） |
| Rust（stable）と `cargo` | `sai-link`・`sai-paper`・`sai-sci` のビルド |
| [`uv`](https://docs.astral.sh/uv/) | `apps/web`・`apps/radio` の Python の仮想環境 |
| Node.js | 画面のテストを流すときだけ（実行には要らない） |
| herdr | エージェントを動かす端末の窓（pane）の管理。`herdr` が PATH に要る |
| エージェント（`claude`・`pi`） | カードの実行者（runner）。使う方だけ入れる |
| Tailscale | iPhone からつなぐときだけ |
| Docker | 実験の記録（MLflow）を使うときだけ |

## 入れ方

```bash
git clone <このリポジトリ> ~/workdir/Sai/sai-moon && cd ~/workdir/Sai/sai-moon

# 1) 実行ファイルを作る（release。Web は release を呼ぶ）
cargo build --release

# 2) Web の窓口の Python 環境
(cd apps/web && uv sync)

# 3) PATH に足す（~/.zshrc などに書く）
export PATH="$HOME/workdir/Sai/sai-moon/target/release:$HOME/workdir/Sai/sai-moon/apps/web/bin:$PATH"
```

これで `sai-link`・`sai-paper`・`sai-sci`・`sai-web` が使えます。確かめ方:

```bash
sai-link --help && sai-paper --help && sai-sci --help && sai-web --help
sai-link schema --json | head -c 200    # 3 つの CLI は命令の一覧を JSON で出す
```

ログイン時に Web を自動で立ち上げたいときは、`apps/web/launchd/` のひな形を使います。

```bash
cp apps/web/launchd/*.plist ~/Library/LaunchAgents/
launchctl load ~/Library/LaunchAgents/<コピーした plist のファイル名>
# 止めるとき
launchctl unload ~/Library/LaunchAgents/<コピーした plist のファイル名>
```

ひな形の `PATH` に `target/release` と `apps/web/bin` が入っていないと、Web が CLI を見つけられず 503 になります。plist の `PATH` を直して入れ直してください。

launchd を使わずに、シェルから直接起動することもできます。

```bash
cd apps/web && nohup .venv/bin/python -m uvicorn sai_web.main:app --host 127.0.0.1 --port 8200 --app-dir src
```

**コードを直したあと**: Python のコードは Web の再起動が要ります。静的ファイル（`apps/web/src/sai_web/static/`）は再起動が要らず、`sw.js` の `CACHE` と `index.html` の `?v=` を上げると反映されます。`cargo build --release` で `sai-link`・`sai-paper`・`sai-sci` が入れ替わり、Web は次の呼び出しから新しい版を使います。

## 最初の一歩

```bash
# 1) 新しいフォルダでプロジェクトを始める（.sai/ と登録簿への登録ができる）
mkdir ~/work/my-project && cd ~/work/my-project
sai-link init
#    研究のプロジェクトなら、代わりに研究の型を選んで始める
#    （型: problem / phenomenon / method / data / improve。sai-link と sai-paper の初期化も含む）
#    sai-sci init --type problem

# 2) Web を起動する
sai-web serve            # 既定は 127.0.0.1:8200（launchd で立ち上げているなら不要）

# 3) ブラウザで http://localhost:8200/ を開く
# 4) Projects タブでプロジェクトを選び、右上の ＋ からカード（実行の予約）を作って、▶ で動かす
```

- カードを動かすには常駐が要ります（下の「起動とポート」）。
- CLI だけでも同じことができます。`sai-link card create`（標準入力に JSON）→ `sai-link card run <run_id>`。命令の一覧は `sai-link schema --json`、Web の「コマンド」画面（`#/project/<名前>/commands`）でも見られます。

## 起動とポート

| ポート | 誰 | 備考 |
|---|---|---|
| `8200` | `sai-web`（Web） | `SAI_WEB_PORT` で変更可 |
| `8201` | `apps/radio`（解説音声の裏方サーバー） | `SAI_RADIO_URL` で変更可 |
| `8202` | `apps/radio` の TTS プロキシ | 使うときだけ別プロセスで起動 |
| `5001` | MLflow（`apps/mlflow-server`・127.0.0.1 のみ） | Research タブの Experiments が読む。`SAI_MLFLOW_URL` |
| `8443` | MLflow の Caddy（LAN 向け・Basic 認証） | LAN の別 PC から記録を書くとき |
| `9000` | MLflow の成果物置き場（minio） | 成果物を書く側が使う |
| `10101` | AivisSpeech Engine | 外部サービス。`AIVISSPEECH_BASE_URL` |

**常駐**（そのプロジェクトでカードを動かすときだけ。プロジェクトごとに 1 つ）:

```bash
cd ~/work/my-project
sai-link daemon start
sai-link daemon status
sai-link daemon stop
```

Web の Projects → Dashboard からも起動・停止できます。

**Radio の裏方サーバー**（論文の解説音声を使うときだけ）:

```bash
cd ~/workdir/Sai/sai-moon
apps/radio/.venv/bin/uvicorn sai_radio.main:app --host 127.0.0.1 --port 8201 --app-dir apps/radio/src
```

画面は無く、Web の Papers → Radio が `/api/radio/*` で中継します。音声を作るときだけ AivisSpeech Engine が要ります。詳しくは [`apps/radio/README.md`](./apps/radio/README.md)。

**MLflow**（実験の記録）は `apps/mlflow-server/` の Docker compose です。起動の手順は [`apps/mlflow-server/README.md`](./apps/mlflow-server/README.md) を読んでください。

## 画面（Web のタブ）

| タブ | 何ができるか |
|---|---|
| **Home** | 答えを待っている質問・承認、確認を待っている run、動いている実行、論文の要対応を 1 画面で見る |
| **Projects** | プロジェクトの Dashboard・Workflow・List・設定（block・template・settings）・コマンド・セッション検索 |
| **Chats** | coding agent の会話を読み、文字や声で指示を送る。質問と承認に答える |
| **Papers** | Dashboard・Library・Import（BibTeX）・Radio（解説音声）。詳しくは下の「Papers」 |
| **Research** | research-tree（`#/research/tree`・`#/research/tree/<ID>`）・READ_CARD（`#/research/read/<run_id>`）・Experiments（`#/research/experiments`）。詳しくは下の「Research」 |
| **Usage** | LLM の料金と制限（Claude Code・OpenRouter・DeepSeek） |

- 質問と cc のコマンド承認は、**PC と iPhone の両方に同時に出て、先に答えた方が採用されます**。
- iPhone（幅 390px）と PC（幅 1280px）の両方で使えます。900px 以上は 2 列になります。
- 画面と API の全部の住所は [`docs/integration.md`](./docs/integration.md) にあります。

### Papers

上の切り替えは Dashboard / Library / Import / Radio の 4 つです。

- **Dashboard**: 数字のカード・State・Mind・Recent activity が出ます。
- **Library**: 上の切り替えで `外部 DB`（外部の論文 DB を検索する。履歴は `sai-paper set` と同じ表に残る）・`内部 DB`（取り込み済みの論文を検索する）・`Paper`（全論文を番号 `#1` の昇順に並べる）を選びます。`Paper` の行を押すと論文詳細が開き、要旨・ノート・図・引用文献・チャンク一覧（本文を `#論文.章.段落` ごとに並べたもの）を読めます。
- **Import**: BibTeX から論文を取り込みます。
- **Radio**: 論文の解説音声を作って聞きます。
- 検索セット専用の画面はありません。`sai-paper set …`（CLI）と API `GET /api/projects/<名前>/papers/sets` は残り、`sci-screen` ブロックのエージェントが keep / drop を付けます。

### Research

research-tree の画面（`#/research/tree`）は、次の並びです。

- **上の帯**: **Next Action**（`[NO_CHILD_Q] Q24 sci-split「…」` の形で次にやることを出す。**Create CARD** を押すとカードを作る）と、折りたたみの **History**（全 Node の履歴を古い順に出す）。
- **中央**: **DAG**（4 layer の図）。左に Node の一覧、右に選んだ Node の **SB**（サイドバー。Node を選ぶまで出ない）。
- **Create SPROUT**（SEED の SB と上の帯の釦）を押すと、History と DAG の間に作成パネルが開きます。作成中も DAG・一覧・SB を操作でき、O の SB の **Add PATH** / **Delete PATH** で PATH の O を選びます。
- TRIAGE（エージェントの PROPOSAL を採るか決める操作）は、Node の SB の PENDING の欄で行います。TRIAGE 専用の住所はありません（古い `#/papers/triage` は research-tree へ送られます）。
- **READ_CARD**（`#/research/read/<run_id>`）: 人が論文を読み、O または E に F を付けて、`note.md` を書いて終える画面です。
- **Experiments**（`#/research/experiments`）: MLflow の実験を見ます。右上の project 選択は research-tree と共有です。

## 設定（環境変数）

### 利用者全体の置き場

| 変数 | 既定値 | 用途 |
|---|---|---|
| `SAI_HOME` | `~/.sai` | 利用者全体の置き場。空でない値があればそれが優先 |

`SAI_HOME` の中身:

| 物 | 何 |
|---|---|
| `registry.json` | プロジェクトの登録簿（`sai-link init` が書く） |
| `blocks/`・`template/`・`settings.json` | 全プロジェクト共通の block・template・settings |
| `paper.env` | 論文の検索・翻訳に使う API キー（全プロジェクト共通） |
| `web/` | Web の置き場: `runs/`（実行の書き出し）・`cc-rate-limits.json`・`token`（合言葉）・`sai_web.db`・`vapid.json`・`sai-web.log` |

プロジェクトの中の `<root>/.sai/`（`sai-engine.db`・`blocks/`・`template/`・`settings.json`・`outputs/`）はこれとは別です。

### sai-web（`apps/web`）

`apps/web/.env` に書きます（`apps/web/.env.example` が見本）。

| 変数 | 既定値 | 用途 |
|---|---|---|
| `SAI_WEB_URL` | `http://127.0.0.1:8200` | `sai-web` の命令とエージェントの道具がつなぐ先 |
| `SAI_WEB_TOKEN` | （空＝認証なし・開発用） | 合言葉。iPhone 側と合わせる |
| `SAI_WEB_PORT` | `8200` | 待ち受けポート |
| `SAI_WEB_RUNS_DIR` | `$SAI_HOME/web/runs` | 会話の画面が読む、実行の書き出し |
| `SAI_WEB_CHROME` | `/Applications/Google Chrome.app/Contents/MacOS/Google Chrome` | 遠隔ブラウザ（Papers）が起動する Chrome |
| `SAI_WEB_BROWSER_HEADED` | （空＝headless） | `1` で画面ありの Chrome（出版社が headless を弾くとき） |
| `SAI_WEB_OPENROUTER_ACCOUNTS` | `main=OPENROUTER_API_KEY` | 使用量の OpenRouter の「名前=キーの入った環境変数名」（カンマ区切り） |
| `SAI_WEB_NO_WATCH` | （空） | `1` で状態の見張りを止める（確かめ用のサーバ） |
| `SAI_WEB_HOOK_LOG` | （空） | cc の承認フックが受けた JSON を書き出すファイル（観察用） |
| `SAI_WEB_HOOKS_DIR` | `apps/web/hooks` | `sai-link init` が cc のフックを書くときに探す置き場 |
| `SAI_LINK_BIN` | `sai-link` | Web が呼ぶ sai-link。PATH に無ければ絶対パス |
| `SAI_PAPER_BIN` | `sai-paper` | Web が呼ぶ sai-paper |
| `SAI_SCI_BIN` | `sai-sci` | Web が呼ぶ sai-sci（無ければ Research の命令が出ない） |
| `SAI_RADIO_URL` | `http://127.0.0.1:8201` | Radio の中継先 |
| `SAI_RADIO_TOKEN` | （空） | `apps/radio` に合言葉があるときだけ同じ値 |
| `SAI_MLFLOW_URL` | `http://127.0.0.1:5001` | MLflow の中継先（読み取りだけ） |
| `SAI_MLFLOW_USER` / `SAI_MLFLOW_PASSWORD` | （空） | MLflow の Basic 認証。空なら付けない |
| `CLAUDE_SESSIONS_DIR` / `PI_SESSIONS_DIR` / `HERDR_BIN` | 各既定 | 会話の記録の置き場・herdr |

### 3 つの CLI

| 変数 | 既定値 | 用途 |
|---|---|---|
| `SAI_PROJECT_ROOT` | （空） | 対象のプロジェクトの root。空なら今のフォルダから上へたどって `.sai/` を持つ最初のフォルダ |
| `SAI_ORIGIN` | `cli` | 記録の出どころ。Web が呼ぶときは `phone` |

### `apps/radio`

`apps/radio/.env` に書きます（見本は `apps/radio/.env.example`）。項目は `LLM_PROVIDER`・`LLM_MODEL`・`OPENROUTER_API_KEY`・`GEMINI_API_KEY`・`GEMINI_LIVE_MODEL`・`AIVISSPEECH_BASE_URL`・`AIVISSPEECH_SPEAKER`・`DOCLING_BIN`・`DOCLING_TIMEOUT_SEC`・`AUDIO_BITRATE`・`GEMINI_TOKEN_TTL_SEC`・`SAI_RADIO_TOKEN`。全部の項目は [`apps/radio/README.md`](./apps/radio/README.md) を見てください。

## iPhone からつなぐ（Tailscale）

通知とマイクには HTTPS が要ります。Tailscale の serve で、`sai-web` だけを tailnet の中に公開します。

```bash
tailscale serve --bg --https 4443 --set-path /sai http://127.0.0.1:8200
tailscale serve status
```

- iPhone からは `https://<Mac名>.ts.net:4443/sai/` を開きます。初回は `?token=…` を付けると、合言葉がブラウザに残ります。
- 公開の手順は旧構成の手順にならったもので、この版ではまだ試していません（要確認）。
- `sai-web` は必ず `--host 127.0.0.1` で動かします（`sai-web serve` の既定です。`0.0.0.0` にすると Tailscale の外からも届きます）。
- `apps/radio` は公開しません（音声は Web の中継からだけ使います）。

## エージェントとつなぐ

エージェントは **CLI を Bash で呼びます**（`sai-link`・`sai-paper`・`sai-sci`・`sai-web`）。MCP の `sai` はありません。
次の設定は **`sai-link init` がプロジェクトに書く**ので、手で登録しません。利用者全体の `~/.claude/` や `~/.pi/` には入れません（init したプロジェクトのエージェントだけが対象です）。

| 書かれる物 | 中身 |
|---|---|
| `.claude/settings.json` | CLI の許可（`Bash(sai-link *)`・`Bash(sai-paper *)`・`Bash(sai-sci *)`・`Bash(sai-web *)`）・cc のフック（下）・不要な道具の禁止 |
| `.claude/commands/` | `/sai-finalize`（root の run を完了にする） |
| `.mcp.json` | 外部の MCP: `serena`・`playwright`・`donsetch`・`headroom`（cc も pi も同じ物を読む） |
| `.pi/settings.json` | pi の質問の拡張 `ask_user.ts`（拡張が見つかったときだけ） |
| `.claude/skills/`・`.pi/skills/` | design skill（`frontend-design`・`ui-ux-rules`） |
| `.serena/project.yml` | serena の言語サーバ（プロジェクトのファイルの拡張子から選ぶ） |
| `.sai/` | `sai-engine`・`blocks/`・`template/`・`settings.json`・`outputs/` |

前に init したプロジェクトは `sai-link init --update` で、配った block・template を新しい版にできます（変わる物は `.bak` に退避されます）。

### cc のフック

`.claude/settings.json` の `hooks` に、次が入ります。

- `UserPromptSubmit`: `sai-link cc-prompt-hook`（入力が `/sai-finalize` のとき、エージェントに送らずに run を完了にする）。
- `apps/web/hooks/` に 3 本（質問 `cc_question.py`・セッションの登録 `cc_register.py`・承認 `cc_permission.py`）が揃っているときだけ、`PreToolUse`（`AskUserQuestion`）・`SessionStart`・`PermissionRequest` の 3 つも入ります。置き場は `SAI_WEB_HOOKS_DIR` で変えられます。

3 本のフックは何も出力せず、必ず終了コード 0 で終わります（出力すると PC の承認画面が出なくなるため）。Web が止まっていても cc は普通に動きます。

### pi の質問の拡張（`ask_user`）

pi が人に質問するとき、Web に質問を送る拡張です（`apps/web/src/sai_web/extensions/ask_user.ts`）。質問は PC と iPhone の両方に出て、先に答えた方が採用されます。

- pi は、信頼していないフォルダの `.pi/` を読みません。init のあと、そのフォルダで初めて pi を起動したときに 1 回だけ信頼を承認します。
- 同じ拡張を `~/.pi/agent/extensions/` にも置くと、名前が重なって pi が起動しません。重なったら片方を消すか、名前を `.old` に変えます。
- `herdr agent list` に pi が出ないときは `herdr integration install pi` を実行します。

### 外部の MCP

- **playwright**: `@playwright/mcp@0.0.82`（WebKit・iPhone 15 の画面・画面なし）。使うプロジェクトで最初に 1 回だけ `npx -y @playwright/mcp@0.0.82 install-browser webkit` を実行します（init 自身は通信しません）。版は固定です（`@latest` にすると WebKit の版とずれます）。
- **serena**: コードの読み書き。`.serena/project.yml` は init が書きます。
- **donsetch**: Web の取得・検索・クロール。
- **headroom**: 長い出力の圧縮と取り出し。

### design skill

`frontend-design`（配色・文字組み・余白。Apache-2.0）と `ui-ux-rules`（コントラスト・文字とタッチの大きさ・フォーカス・ダークモード・グラフ。MIT）の 2 つです。cc は `/frontend-design` のように呼べ、pi は `.pi/skills/` のフォルダを信頼したあとに `/skill:frontend-design` で呼べます。入れるのは規則の文書だけで、スクリプトは入りません。ライセンスは各 skill のフォルダにあります。

### 研究（sai-sci）・実験の記録・実行先

research-tree（Q・O・F・E）・professor・LabAgent（研究用の block と template）・実験の記録（MLflow への接続情報）・実験の実行先（どの PC で動かすか）は、[`docs/sai-sci.md`](./docs/sai-sci.md) にまとめてあります。手を動かして覚えるなら [`docs/sai-sci-tutorial.md`](./docs/sai-sci-tutorial.md) から始めてください。

- 始め方は `sai-sci init --type <型>` の 1 行です（`sai-link`・`sai-paper` の初期化と、研究の部品の配布を含みます）。
- agent に渡す環境変数（MLflow の接続情報など）は `.sai/settings.json` か `$SAI_HOME/settings.json` の `env` に書きます。書き方は [`docs/for-sai-developers.md`](./docs/for-sai-developers.md) の「`.sai/settings.json` の `env`」にあります。

## 安全性

| 項目 | 確かめ方 |
|---|---|
| 合言葉が設定されている | `apps/web/.env` に `SAI_WEB_TOKEN`。未設定なら起動時に警告が出る |
| 合言葉のファイル | 起動すると `$SAI_HOME/web/token` に 0600 でできる。エージェントの道具が読む。**中身を記録や画面に出さない** |
| 127.0.0.1 だけで待ち受けている | `lsof -i :8200` |
| 内部用の API が外から呼べない | `POST /api/register`・`/api/questions`・`/api/questions/{qid}/resolved`・`/api/approvals` は Mac 自身（127.0.0.1）からだけ受ける |
| 他の DB を開いていない | プロジェクト・論文・研究はすべて 3 つの CLI か中継 API を通す |
| 読み書きするファイルを iPhone から指定できない | run の md・定義・.bib の置き場は CLI が名前から決める。名前は Web が先に形を検査する |
| 壊れた定義を保存しない | 定義の保存前に sai-link が検査し、理由を出す |

## テスト（開発者向け）

テストの命令は [`AGENTS.md`](./AGENTS.md) の「テストと確認」にあります（Rust は `cargo test --workspace`、Web の画面は `apps/web/tests/test_*.js`、サーバは `apps/web/tests/test_*.py`、Radio は `apps/radio/tests/`、sai-sci の道具は `sai-sci/tools/`）。

画面の確認は 8200 / 8201 / 8202 番で行います。**旧構成の 8100 番・8000 番は、確認のために止めたり再起動したりしません。**

## 文書

- [`ARCHITECTURE.md`](./ARCHITECTURE.md) — 全体の考え方・境界・依存の向き・CLI の約束・置き場・ファイル構成（開発者向け）。
- [`AGENTS.md`](./AGENTS.md) — このリポジトリで作業するエージェントへの指示とテストの命令。
- [`CONCEPT.md`](./CONCEPT.md) — 概念と用語（sai の部品・run・card・doc・research-tree・画面の名前ほか）。
- [`docs/integration.md`](./docs/integration.md) — Web の API と画面の住所の一覧、各 API が呼ぶ CLI の命令、統合の注意点。
- [`docs/for-sai-developers.md`](./docs/for-sai-developers.md) — 3 つの CLI の約束と、Web が読む JSON の項目（CLI を直す人が守ること）。
- [`docs/sai-sci.md`](./docs/sai-sci.md) — research-tree・研究の型・ガイド・ブロック・実験・Web の画面の説明書。
- [`apps/radio/README.md`](./apps/radio/README.md)・[`apps/radio/ARCHITECTURE.md`](./apps/radio/ARCHITECTURE.md) — Radio の詳細。
- [`apps/mlflow-server/README.md`](./apps/mlflow-server/README.md) — MLflow サーバの起動と運用。
