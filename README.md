# sai-moon

**Mac 上の coding agent（sai-link）と論文の作業（sai-paper）を、ブラウザと音声で使うためのアプリ**です。
旧 4 リポジトリ（`sai-link`・`sai-paper`・`sai-phone`・`sai-radio`）を 1 つにまとめたもので、1 つのリポジトリ・3 つのプロセスで動きます。

| プロセス | 何 | ポート |
|---|---|---|
| Rust の実行ファイル（`sai-moon-link`・`sai-moon-paper`） | 常駐・エージェントの実行・論文の検索と保存 | （待受けなし。必要に応じて CLI / TUI で起動する） |
| **Web の窓口**（`apps/phone`・FastAPI） | iPhone と PC のブラウザから使う画面 | `127.0.0.1:8200` |
| **Radio の裏方サーバー**（`apps/radio`・FastAPI） | 論文の解説音声の生成と配信 | `127.0.0.1:8201`（TTS プロキシは `8202`） |

## 考え方

- **中身は 1 つ、入口は 3 つ。** データと処理を持つのは Rust のクレート（`sai-engine.db`・`papr.db`・DB・CLI・常駐）だけです。
  TUI 2 つ（`sai-moon-link-tui`・`sai-moon-paper-tui`）と Web（`apps/phone`）が同じ中身を使う入口です。
  Web と TUI のどちらで操作しても結果は同じで、どちらから作った物も両方に出ます。
- **Web を先に考える。** 端末（ターミナル）に慣れていない人でも、主な操作を Web だけで終えられるようにします。
- **iPhone と PC の両方で使う。** 幅 900px 未満（iPhone）は下にタブが並ぶ 1 列、900px 以上（PC）は左に縦のタブが並び、一覧と中身が 2 列で並びます。
- **言葉で迷わせない。** 画面の文字は日本語にし、sai 独自の言葉（run・review・waggle など）には押すと説明が出る札を付けています。
- **置き場は `~/.sai-moon/`。** 利用者全体で共有する物（登録簿・共通ブロック・実行の書き出し・`paper.env`）はここに入る。プロジェクトの中の `.sai/` は従来どおり。

仕組みの詳しい説明は [`ARCHITECTURE.md`](./ARCHITECTURE.md) にあります。

## 画面は 5 つのタブ

| タブ | 何ができるか |
|---|---|
| **ホーム** | 答えを待っている質問・承認、確認を待っている review、動いている実行、論文の要対応を 1 画面で見る |
| **プロジェクト** | プロジェクト（Rust コアが管理するもの）を見る・動かす（Dashboard・Workflow・List・設定） |
| **会話** | coding agent の会話を読み、文字や声で指示を送る。質問と承認に答える |
| **論文** | 論文を探す・取り込む・読む・聴く（Dashboard・Discover・Library・Canvas・Radio） |
| **使用量** | LLM の料金と制限（Claude Code・OpenRouter・DeepSeek） |

- **ホーム** — いま決めることを 1 枚に。質問と承認、確認を待っている review、動いている実行、止まっている常駐、論文の要対応が並ぶ。15 秒ごとに読み直す。
- **プロジェクト** — 一覧から選ぶと **Dashboard | Workflow | List | 設定** に切り替わる。
  - **Dashboard**: 実行数・成功率・トークン量・時間の大きな数字、日ごとのグラフ、モデル別・ブロック別の内訳、直近の実行。常駐の起動・停止。
  - **Workflow**: sai-link-tui と**同じ並び**の図（配置は Rust のコアが計算する）。札（実行）・線・Kanban の 4 レーン。ドラッグで動かせ、拡大縮小できる。
  - **List**: TUI の左の一覧と同じツリーから実行を選び、**Log / Stats / History / task / review / message / waggle** の 7 つで見る。task と review はその場で書き換えて保存できる。右上の **＋** で task.md を作れる。
  - **設定**: そのプロジェクトの `.sai/` と、全体の `~/.sai-moon/` にある block / template / settings を読み書きする。壊れた定義は保存しない。
- **会話** — 日付 → 実行 → 会話。下の入力欄から文字を送れ、🎤 で音声入力もできる。質問と cc のコマンド承認には **PC と iPhone の両方に同時に出て、先に答えた方が採用される**。
- **論文** — 上部の **Dashboard / Discover / Library / Canvas / Radio**。Radio は続きから聴く・15 秒送り戻し・速度・オフライン保存と、文字か声での質問ができる。
- **使用量** — Claude Code（Team）の 5 時間枠、OpenRouter・DeepSeek の残高と使用料。

画面と API の全部の住所は [`docs/integration.md`](./docs/integration.md) にあります。

## 起動

3 つとも `~/workdir/Sai/sai-moon` で動かす。**`sai-moon-link` は PATH に無い**ので、PATH を足すか `SAI_LINK_BIN` に絶対パスを渡す。

```bash
# 0) 実行ファイルを作る（最初だけ。release を作る。apps/phone は release を叩く）
cd ~/workdir/Sai/sai-moon
cargo build --release
```

### 1. 常駐（そのプロジェクトでタスクを動かすときだけ）

```bash
cd ~/workdir/Sai/sai-moon
./target/release/sai-moon-link daemon start
# 止めるとき
./target/release/sai-moon-link daemon stop
```

常駐はプロジェクトごとに 1 つ。`apps/phone` の Dashboard からも起動・停止できる。

### 2. Web の窓口（`apps/phone`）

```bash
cd ~/workdir/Sai/sai-moon
PATH=$HOME/workdir/Sai/sai-moon/target/release:$PATH \
  apps/phone/.venv/bin/uvicorn sai_phone.main:app \
  --host 127.0.0.1 --port 8200 --app-dir apps/phone/src
```

ブラウザで `http://localhost:8200/` を開く。

- `PATH` を足さないと `sai-moon-link` が見つからず、Project タブが 503 になる。`SAI_PAPER_BIN` は既定で絶対パスなので、Papers タブは PATH 無しでも動く。
- 仮想環境は `uv sync` 済み（`apps/phone/.venv`）。入れ直すときは `cd apps/phone && uv sync`。

### 3. Radio の裏方サーバー（`apps/radio`）

```bash
cd ~/workdir/Sai/sai-moon
apps/radio/.venv/bin/uvicorn sai_radio.main:app \
  --host 127.0.0.1 --port 8201 --app-dir apps/radio/src
```

画面は無い。`apps/phone` の `/api/radio/*` がここへ中継する。`<id>` の音声を作るときだけ AivisSpeech Engine（既定 `http://127.0.0.1:10101`）が必要。

### 3 つのポート

`apps/phone` と `apps/radio` が使うポートは次のとおり。

| ポート | 誰 | 備考 |
|---|---|---|
| `8200` | `apps/phone`（Web） | `SAI_PHONE_PORT` で変更可 |
| `8201` | `apps/radio`（裏方サーバー） | `SAI_RADIO_URL` で変更可 |
| `8202` | `apps/radio` の TTS プロキシ | 使うときだけ別プロセスで起動 |
| `10101` | AivisSpeech Engine | 外部サービス。`AIVISSPEECH_BASE_URL` |

**旧構成の 8100 番（旧 sai-phone）と 8000 番（旧 sai-radio）は別プロセス。** sai-moon の確認では、この 2 つの古いポートと必ず区別すること。古い方を止めたり再起動したりしない。

### Mac のログイン時に自動で立ち上げる

`apps/phone` だけ plist がある（`apps/phone/launchd/com.sai-moon.plist`、label `com.sai-moon`）。

```bash
cp apps/phone/launchd/com.sai-moon.plist ~/Library/LaunchAgents/
launchctl load ~/Library/LaunchAgents/com.sai-moon.plist
# 止めるとき
launchctl unload ~/Library/LaunchAgents/com.sai-moon.plist
```

plist は `PYTHONPATH`（`apps/phone/src`）と `PATH`（`target/release`・`target/debug`・`/usr/local/bin:…`）を渡す。
`PATH` が無いと `sai-moon-link` が見つからず 503 になる。

## 設定（環境変数）

### `apps/phone`

`.env` に書く（`apps/phone/.env.example` が見本）。

| 変数 | 既定値 | 用途 |
|---|---|---|
| `SAI_PHONE_TOKEN` | （空＝認証なし・開発用） | 合言葉。iPhone 側と合わせる |
| `SAI_PHONE_PORT` | `8200` | 待ち受けポート |
| `SAI_LINK_BIN` | `sai-moon-link` | Project タブ・List 画面が呼ぶ CLI。**PATH に無いので PATH を足すか絶対パスを書く** |
| `SAI_PAPER_BIN` | `~/workdir/Sai/sai-moon/target/release/sai-moon-paper` | Papers タブが呼ぶ CLI（絶対パス） |
| `SAI_SAI_HOME` | （空＝`~/.sai-moon`） | 登録簿 `registry.json` の置き場。空なら `$HOME/.sai-moon/registry.json` を読む |
| `SAI_RADIO_URL` | `http://127.0.0.1:8201` | Radio の中継先 |
| `SAI_RADIO_TOKEN` | （空） | `apps/radio` に合言葉があるときだけ同じ値 |
| `SAI_PHONE_RUNS_DIR` | `~/.sai-moon/phone/runs` | 会話の画面が読む、書き出し |
| `SAI_PHONE_OPENROUTER_ACCOUNTS` | `main=OPENROUTER_API_KEY` | 使用量の OpenRouter の「名前=キーの入った環境変数名」（カンマ区切り） |
| `CLAUDE_SESSIONS_DIR` / `PI_SESSIONS_DIR` / `HERDR_BIN` | 各既定 | 会話の記録の置き場・herdr |

### Rust 側（`crates/`）

| 変数 | 既定値 | 用途 |
|---|---|---|
| `SAI_SAI_HOME` | （空＝`~/.sai-moon`） | 利用者全体の置き場。`registry.json`・`phone/runs`・`cc-cost`・共通の `blocks`・`template`・`settings.json`・`paper.env` がここ |
| `SAI_PROJECT_ROOT` | （空） | sai-link が DB 解決に使う最優先の root。`apps/phone` は呼び出し時に必ずプロジェクトの root を明示で渡す |
| `SAI_PAPER_ROOT` | （空） | `sai-moon-paper` が使う root。`apps/phone` は選んだプロジェクトの root を渡す |

### `apps/radio`

`.env` に書く（`apps/radio/.env.example` が見本）。`LLM_PROVIDER`・`LLM_MODEL`・`OPENROUTER_API_KEY`・`GEMINI_API_KEY`・`GEMINI_LIVE_MODEL`・`AIVISSPEECH_BASE_URL`・`AIVISSPEECH_SPEAKER`・`DOCLING_BIN`・`DOCLING_TIMEOUT_SEC`・`AUDIO_BITRATE`・`GEMINI_TOKEN_TTL_SEC`・`SAI_RADIO_TOKEN`。全部の項目は [`apps/radio/README.md`](./apps/radio/README.md) を参照。

## iPhone からつなぐ（Tailscale）

通知とマイクには HTTPS が要ります。Tailscale の serve で、`apps/phone` だけを tailnet の中に公開します。

旧 sai-phone の `--set-path /sai-phone` にならった手順です。sai-moon ではまだ試していません（要確認）。

```bash
tailscale serve --bg --https 4443 --set-path /sai-moon http://127.0.0.1:8200
tailscale serve status        # funnel が含まれていないことを確かめる
```

- iPhone からは `https://<Mac名>.ts.net:4443/sai-moon/` を開きます。初回は `?token=…` を付けると、合言葉がブラウザに残ります。
- `apps/phone` は必ず `--host 127.0.0.1` で動かします（`0.0.0.0` にすると Tailscale の外からも届きます）。
- `tailscale funnel` は使いません（インターネット全体に公開されます）。
- `apps/radio` は公開しません（音声は `apps/phone` の中継からだけ使います）。
- 旧構成の `http://127.0.0.1:8100` に繋がった場合は新しい plist / PATH を読み直してください（再起動が必要）。

## エージェントとつなぐ

以下は**手順を書くだけ**で、このリポジトリにスクリプトとしては登録しません。実際に切り替えるときは、自分で決めて登録してください。

### pi の質問ツール（ask_user_phone）

```bash
cp apps/phone/src/sai_phone/extensions/ask_user_phone.ts ~/.pi/agent/extensions/
# pi を起動中なら /reload。道具の一覧に ask_user_phone が出れば完了
herdr integration install pi    # herdr agent list に pi が出ないとき
```

既定の `SAI_PHONE_URL` は `http://127.0.0.1:8200`。

### cc のフック

`apps/phone/hooks/` に 3 本ある（質問 `cc_question.py`・セッションの登録 `cc_register.py`・承認 `cc_permission.py`）。
**`sai-moon-link init` がプロジェクトの `.claude/settings.json` に書く**ので、手で登録しなくてよい。
利用者全体の `~/.claude/settings.json` には入れない（init したプロジェクトの cc だけが Web の窓口に質問を送る）。

- 書く場所は `SAI_PHONE_HOOKS_DIR` があればそれ、無ければ sai-moon-link をビルドしたリポジトリの `apps/phone/hooks`。3 本が揃っていなければフックは書かない。
- 前に init したプロジェクトは、`init` をやり直すか（前の設定は `.bak` に退避される）、`.claude/settings.json` の `hooks` に次の 3 つを足す。

```json
"PreToolUse": [ { "matcher": "AskUserQuestion",
  "hooks": [ { "type": "command", "command": "'/Users/touta/workdir/Sai/sai-moon/apps/phone/hooks/cc_question.py'" } ] } ],
"SessionStart": [ { "hooks": [ { "type": "command", "command": "'/Users/touta/workdir/Sai/sai-moon/apps/phone/hooks/cc_register.py'" } ] } ],
"PermissionRequest": [ { "matcher": "",
  "hooks": [ { "type": "command", "command": "'/Users/touta/workdir/Sai/sai-moon/apps/phone/hooks/cc_permission.py'" } ] } ]
```

フックは何も出力せず、必ず終了コード 0 で終わります（出力すると PC の承認画面が出なくなるため）。`apps/phone` が止まっていても cc は普通に動きます。

## 安全性

| 項目 | 確かめ方 |
|---|---|
| 合言葉が設定されている | `.env` に `SAI_PHONE_TOKEN`。未設定なら起動時に警告が出る |
| 127.0.0.1 だけで待ち受けている | `lsof -i :8200` |
| funnel を使っていない・公開は 4443 番だけ | `tailscale serve status` |
| 内部用の API が外から呼べない | `POST /api/register`・`/api/questions`・`/api/questions/{qid}/resolved`・`/api/approvals` は Mac 自身（127.0.0.1）からだけ受ける（`main.py` の `require_local`） |
| 他の DB を開いていない | プロジェクト・論文はすべて CLI か中継 API を通す |
| 読み書きするファイルを iPhone から指定できない | run の md・定義・.bib の置き場は Rust のコアが名前から決める。名前は `apps/phone` が先に形を検査する |
| 壊れた定義を保存しない | 定義の保存前に Rust 側が検査し、理由を出す |

## テスト（開発者向け）

```bash
cd ~/workdir/Sai/sai-moon

# Rust。SAI_PROJECT_ROOT などが混ざっていると History テスト 1 件が落ちる。必ず env -u で外す。
env -u SAI_PROJECT_ROOT -u SAI_PARENT_RUN_ID -u SAI_BLOCK_NAME \
  perl -e 'alarm 1800; exec @ARGV' cargo test --workspace

# Web（JS 19 本）
for f in apps/phone/tests/test_*.js; do node "$f" || exit 1; done

# Web（Python 17 本。PYTHONPATH が必要）
(cd apps/phone && export PYTHONPATH=src && for f in tests/test_*.py; do .venv/bin/python "$f" || exit 1; done)

# Radio（3 本）
(cd apps/radio && for f in tests/test_*.py; do PYTHONPATH=src .venv/bin/python "$f" || exit 1; done)

plutil -lint apps/phone/launchd/com.sai-moon.plist
```

画面の確認は 8200 / 8201 / 8202 番で行います。**旧の 8100 番・8000 番は再起動しない。**

## 文書

- [`ARCHITECTURE.md`](./ARCHITECTURE.md) — 全体の仕組み・層の分け方・守る約束・置き場（開発者向け）。
- [`AGENTS.md`](./AGENTS.md) — このリポジトリで作業するエージェントへの指示。
- [`docs/integration.md`](./docs/integration.md) — `apps/phone` の API と画面の住所の一覧、統合の注意点。
- [`docs/for-sai-developers.md`](./docs/for-sai-developers.md) — `crates/` と `apps/radio` を直す人が守ること（`apps/phone` を壊さないために）。
- [`apps/radio/README.md`](./apps/radio/README.md)・[`apps/radio/ARCHITECTURE.md`](./apps/radio/ARCHITECTURE.md) — Radio だけ 詳しく知りたときの詳細。
