# agent-link

**アプリ（主に sai-tree）を開発するための道具**です。Mac で 1 人が使います。coding agent の実行・カード（実行の予約）・文書（waggle トークン）を、ブラウザ（iPhone と PC）とコマンドで扱います。
由来: sai-tree から、sai-link（run・カード・文書・子タスク・daemon）と Web の単独モードだけを取り出し、外から見える名前を agent-link に変えた物です。

2 つの部品でできています（遠隔の機械は、子の claude を動かすときだけ足す任意の相手です）。

| 部品 | 何 | 実行ファイル |
|---|---|---|
| **実行エンジン** | 基盤。coding agent の実行・カード・文書・子タスク・常駐（daemon） | `agent-link` |
| **Web の窓口** | FastAPI と素の JavaScript。`agent-link` の CLI だけを使う（DB は開かない） | `apps/web/bin/agent-link-web` |
| 遠隔の機械（任意） | 子タスクの claude だけを ssh 先で動かす。`agent-link` 本体・DB・文書・Web は Mac に置いたまま | 中継 `~/.agent-link-remote/bin/agent-link`（`agent-link remote setup` が置く）と、Mac 側の門番 `agent-link remote-gate` |

```
人 ──── ブラウザ ───▶ （iPhone だけ Tailscale serve）─▶ agent-link-web (127.0.0.1:8400)
 │                                          │ CLI（--json）
 └──── CLI ───────────────────────────────▶ agent-link ──▶ daemon・headless の実行
エージェント ──── CLI（Bash）──────────────▶ agent-link        │ ssh -t（task create の host があるとき）
                                                                 ▼
                                          遠隔の機械の claude（子）──中継──ssh──▶ Mac の remote-gate
```

- 人は **CLI と Web** を使い、エージェントは **CLI** を使います（エージェントに MCP の道具は渡しません。Bash で CLI を呼びます）。
- 画面は Web だけです。Web の HTTP API は `agent-link-web api` で呼べ、CLI の命令は `agent-link schema --json` に出ます。
- Mac 専用の常駐（launchd）・承認・遠隔ブラウザは持ちません。daemon と Web は手で起こします。

仕組みの詳しい説明は [`ARCHITECTURE.md`](./ARCHITECTURE.md)、概念と用語は [`CONCEPT.md`](./CONCEPT.md) にあります。sai-moon と並べて使うときは [`docs/switch-over.md`](./docs/switch-over.md) を上から順に実行します。

## 必要な物

| 物 | 用途 |
|---|---|
| macOS | 動作環境 |
| Rust（stable）と `cargo` | `agent-link` のビルド |
| [`uv`](https://docs.astral.sh/uv/) | `apps/web` の Python の仮想環境 |
| tmux | cc（Claude Code）を動かす（`brew install tmux`） |
| ssh | 遠隔の機械で子の claude を動かすときだけ（`docs/remote.md`） |
| Node.js | 画面のテストを流すときだけ（実行には要らない） |
| エージェント（`claude`） | カードの実行者（cc だけ） |
| Tailscale | iPhone からつなぐときだけ（自分 1 人の tailnet のときだけ。下の注意を読む） |

## 入れ方

台本 `scripts/agent-link-setup` を使います。入れる・点検する・消すの 3 つの命令だけで、シェルの設定（`~/.zshrc` など）は書き換えません。

```bash
git clone <このリポジトリ> ~/workdir/Sai/agent-link && cd ~/workdir/Sai/agent-link
scripts/agent-link-setup install        # cargo build --release・uv sync・~/.local/bin にリンク
# 表示された export PATH=… の行を ~/.zshrc に足す（台本はシェルの設定を書かない）
scripts/agent-link-setup doctor         # ok / warn / NG を 1 行ずつ。NG があれば終了コード 1
```

- `install [--bin-dir DIR]`: `cargo build --release` と `(cd apps/web && uv sync)` を実行し、`DIR`（既定 `$HOME/.local/bin`）に `agent-link`・`agent-link-web` のリンクを 2 本置きます。`DIR` が PATH に無ければ、足す 1 行を表示します。`DIR` に普通のファイルの `agent-link` があるときは、上書きせずに止まります。
- `doctor`: 次の項目を `ok|warn|NG <項目> <詳しく>` の 1 行ずつで出します。`NG` が 1 つでもあれば終了コード 1。`warn` は動くが直したほうがよい物です。
  `tmux`・`claude`・`uv`・`cargo`（`cargo` だけ `warn`）・`agent-link`・`agent-link-release`（PATH の `agent-link` がこのリポジトリの release か）・`release-fresh`（ソースより release が新しいか）・`agent-link-web`・`web-venv`・`hooks`・`registry`・`projects`・`web`（Web が答えるか。止まっていても `warn` だけ）。
- 新しいシェルで、`command -v agent-link agent-link-web` と `agent-link schema --json | head -c 200`（命令の一覧を JSON で出す）でも確かめられます。

**台本を使わないとき**（手作業）:

```bash
cargo build --release                    # 1) 実行ファイルを作る（Web は release を呼ぶ）
(cd apps/web && uv sync)                 # 2) Web の窓口の Python 環境
# 3) PATH に足す（~/.zshrc などに書く）
export PATH="$HOME/workdir/Sai/agent-link/target/release:$HOME/workdir/Sai/agent-link/apps/web/bin:$PATH"
```

### 消し方

```bash
scripts/agent-link-setup uninstall [--bin-dir DIR] [--purge-home]
```

- **消す物**: `DIR`（既定 `$HOME/.local/bin`）の `agent-link`・`agent-link-web` のリンクのうち、このリポジトリを指す物だけ。`--purge-home` を付けたときは `$AGENT_LINK_HOME`（既定 `~/.agent-link`）も消します（`$AGENT_LINK_HOME` が空・`/`・`HOME` のときは何も消さずに止まります）。
- **消さない物（一覧を出すだけ）**: 登録簿にあるプロジェクトの中（`.agent-link/`・`.mcp.json`・`.claude/commands/agent-link-finalize.md`・`.claude/skills/frontend-design`・`.claude/skills/ui-ux-rules`・`.serena/project.yml`）、`~/.claude.json`、`~/.ssh/authorized_keys`（`remote setup` が足した `remote-gate` の行。`docs/remote.md` の「鍵を外す」）、遠隔の機械の `~/.agent-link-remote/`。
- **止めない物**: 動いている Web と daemon は止めません。止めるための命令（PID 指定の `kill`・`agent-link daemon stop`）を表示します。
- シェルの設定の PATH の行と `agent-link-web-restart` は、利用者が自分で消します。

Web を起こし直す関数も `~/.zshrc` に書いておくと楽です。PATH に `agent-link` が無いシェルから起こすと、Web が `agent-link not found` で繋がらなくなるので、PATH（上の 3）と一緒に書きます。

```bash
agent-link-web-restart() {
  kill $(lsof -tiTCP:8400 -sTCP:LISTEN) 2>/dev/null
  mkdir -p ~/.agent-link/web
  (cd ~/workdir/Sai/agent-link/apps/web && nohup bin/agent-link-web serve --port 8400 >~/.agent-link/web/agent-link-web.log 2>&1 &)
}
```

**コードを直したあと**: Python のコードは Web の再起動（`agent-link-web-restart`）が要ります。静的ファイル（`apps/web/src/agent_link_web/static/`）は再起動が要らず、`sw.js` の `CACHE` と `index.html` の `?v=` を上げると反映されます。`cargo build --release` で `agent-link` が入れ替わり、Web は次の呼び出しから新しい版を使います。

## 最初の一歩

```bash
# 1) 新しいフォルダでプロジェクトを始める（.agent-link/ と登録簿への登録ができる）
mkdir ~/work/my-project && cd ~/work/my-project
git init
agent-link init

# 2) このプロジェクトの常駐（daemon）を起こす（カードを動かすときだけ要る）
agent-link daemon start

# 3) Web を起こす
cd ~/workdir/Sai/agent-link/apps/web
agent-link-web serve        # 既定は 127.0.0.1:8400

# 4) ブラウザで http://localhost:8400/ を開く（合言葉は無い）
# 5) Projects タブでプロジェクトを選び、右上の ＋ からカード（実行の予約）を作って、▶ で動かす
```

- 背景で動かし続けるときは、`cd apps/web && nohup .venv/bin/python -m uvicorn agent_link_web.main:app --host 127.0.0.1 --port 8400 --app-dir src > ~/.agent-link/web/uvicorn-8400.log 2>&1 &` を使います（`~/.agent-link/web/` は Web が最初の起動で作ります）。
- CLI だけでも同じことができます。`agent-link card create`（標準入力に JSON）→ `agent-link card run <run_id>`。命令の一覧は `agent-link schema --json` で見られます。Web の HTTP API は `agent-link-web api <METHOD> <path>` で呼べます。
- `init` は、プロジェクトに `.agent-link/`（cc の設定 `.agent-link/cc-settings.json` を含む）・`.mcp.json`・`.claude/commands/`・`.claude/skills/`・`.serena/project.yml` を書きます（既にある設定は `.bak` に退避します）。

## 起動とポート

| ポート | 誰 | 備考 |
|---|---|---|
| `8400` | `agent-link-web`（Web） | `AGENT_LINK_WEB_PORT` で変更可。確認用は 8297 番など別の番号を使う |

**常駐**（そのプロジェクトでカードを動かすときだけ。プロジェクトごとに 1 つ）:

```bash
cd ~/work/my-project
agent-link daemon start
agent-link daemon status
agent-link daemon stop
```

Web の Projects → Dashboard からも起動・停止できます。

## 画面（Web のタブ）

| タブ | 何ができるか |
|---|---|
| **Home** | 答えを待っている質問、todo、動いている実行、全プロジェクトの合計、Claude の使用量（5 時間枠・7 日枠と費用のカード）を 1 画面で見る |
| **Projects** | プロジェクトの Dashboard・Workflow・List・設定（block・template・settings）・セッション検索 |
| **Chats** | coding agent の会話を読み、文字や声で指示を送る。新しいチャットを始める。質問に答える |

- 質問は、PC と iPhone の両方に出て、先に答えた方が採用されます。
- iPhone（幅 390px）と PC（幅 1280px）の両方で使えます。900px 以上は 2 列になります。
- 画面の文言と、`agent-link` の標準エラーおよび `--json` の人向けの文は英語です（音声の言語は日本語）。この README など文書は日本語です。
- run の流れは todo → running → review →（`agent-link run advance`）→ Workflow です。Workflow に移した run は `.agent-link/outputs/<yymmdd>/<run_id>/` に入ります（日付は一番上の親 run の実行開始日）。
- 画面と API の全部の住所は [`docs/integration.md`](./docs/integration.md) にあります。

## 設定（環境変数と `config`）

**環境変数の表はこの README だけにあります**（ほかの文書はここへのリンクです）。表の「`config` に書ける」の列に ○ がある名前は、環境変数の代わりに `$AGENT_LINK_HOME/settings.json` の `config` にも書けます。

- 優先は、環境変数（空白だけは無いのと同じ）→ `config` → 既定値。書く場所は `$AGENT_LINK_HOME/settings.json` だけで、プロジェクトの `.agent-link/settings.json` の `config` は読みません。`AGENT_LINK_HOME` 自身は `config` に書けません。
- 値は文字列か整数です。詳しい規則は [`docs/for-developers.md`](./docs/for-developers.md) の「`config`」にあります。

```json
{ "config": { "AGENT_LINK_WEB_PORT": 8401, "AGENT_LINK_CLAUDE_BIN": "/opt/homebrew/bin/claude" } }
```

### 利用者全体の置き場

| 変数 | 既定値 | 用途 | `config` に書ける |
|---|---|---|---|
| `AGENT_LINK_HOME` | `~/.agent-link` | 利用者全体の置き場。空でない値があればそれが優先（規則の全文は [`docs/for-developers.md`](./docs/for-developers.md)） | |

`AGENT_LINK_HOME` の中身:

| 物 | 何 |
|---|---|
| `registry.json` | プロジェクトの登録簿（`agent-link init` が書く。形は `docs/for-developers.md` の §3.8） |
| `blocks/`・`template/`・`settings.json` | 全プロジェクト共通の block・template・settings（`settings.json` の `config` は上の設定） |
| `web/` | Web の置き場: `runs/`（実行の書き出し）・`agent_link_web.db`・`vapid.json`・`agent-link-web.log`（上の `agent-link-web-restart` で起こしたときのログの出し先） |
| `cc-cost/`・`cc-rate-limits.json` | cc の費用（`cc-cost/<session_id>.json`）と 5 時間枠・7 日枠。cc の statusLine（`agent-link cc-statusline`）が書き、Home の Claude のカードが読む |

プロジェクトの中の `<root>/.agent-link/`（`engine/agent-link.db`・`blocks/`・`template/`・`settings.json`・`outputs/`）はこれとは別です。古い置き場（sai-moon の物）は読みません。

### Web（`apps/web`）

`apps/web/.env` に書きます（`apps/web/.env.example` が見本）。

| 変数 | 既定値 | 用途 | `config` に書ける |
|---|---|---|---|
| `AGENT_LINK_WEB_PORT` | `8400` | 待ち受けポート | ○ |
| `AGENT_LINK_WEB_URL` | `http://127.0.0.1:8400` | cc のフックと `agent-link-web` の CLI がつなぐ先（MCP は無い） | ○ |
| `AGENT_LINK_WEB_RUNS_DIR` | `$AGENT_LINK_HOME/web/runs` | 会話の画面が読む、実行の書き出し | ○ |
| `AGENT_LINK_WEB_NO_WATCH` | （空） | `1` で状態の見張りを止める（確かめ用のサーバ） | ○ |
| `AGENT_LINK_WEB_HOOKS_DIR` | `apps/web/hooks` | `agent-link init` が cc のフックを書くときに探す置き場 | ○ |
| `AGENT_LINK_BIN` | `agent-link` | Web が呼ぶ `agent-link`。PATH に無ければ絶対パス（子タスクには `agent-link` が自分のパスを入れる） | ○ |
| `CLAUDE_SESSIONS_DIR` | `~/.claude/projects` | cc の会話の記録の置き場 | ○ |

### CLI（`agent-link`）

| 変数 | 既定値 | 用途 | `config` に書ける |
|---|---|---|---|
| `AGENT_LINK_PROJECT_ROOT` | （空） | 対象のプロジェクトの root。空なら今のフォルダから上へたどって `.agent-link/` を持つ最初のフォルダ | |
| `AGENT_LINK_PARENT_RUN_ID` | （agent-link が起動時に渡す） | エージェントが動いている run の番号。`run_id` を省いた命令が「自分の run」として読む | |
| `AGENT_LINK_BLOCK_NAME` | （agent-link が起動時に渡す） | 動いている block の名前 | |
| `AGENT_LINK_IS_SUBTASK` | （agent-link が起動時に渡す） | 子タスクとして動いているとき `1` | |
| `AGENT_LINK_CLAUDE_BIN` | `claude` | cc の実行ファイル | ○ |
| `AGENT_LINK_IDLE_TIMEOUT_MS` | `60000` | 子タスクが止まったとみなす時間（ミリ秒） | ○ |
| `AGENT_LINK_LONG_RUN_ALERT_MS` | 20 分 | 長時間アラートの間隔（ミリ秒） | ○ |
| `AGENT_LINK_CC_QUESTION_MODE` | （空） | `deny` で、cc の質問フックが答えを拒否の形で返す | |
| `AGENT_LINK_SSH_BIN` | `ssh` | 試験用。Mac 側の ssh（`remote setup`・遠隔の子の起動）を差し替える | ○ |

`AGENT_LINK_PARENT_RUN_ID`・`AGENT_LINK_BLOCK_NAME`・`AGENT_LINK_IS_SUBTASK` のほか、`doc write` の検査（`write_check`）に渡す `AGENT_LINK_WRITE_FILENAME`・`AGENT_LINK_WRITE_DESCRIPTION`、tmux の session に入れる `AGENT_LINK_AGENT_TRANSPORT`（と、daemon の値を渡す `AGENT_LINK_WEB_URL`・`AGENT_LINK_HOME`）、遠隔の子の起動行に入れる `AGENT_LINK_HOST`（その子が動く ssh の Host 名）は、agent-link が自分で入れる値です。利用者は設定しません。

遠隔の機械（`wsl-server` など）で子の claude を動かす手順は [`docs/remote.md`](./docs/remote.md) にあります。

## iPhone からつなぐ（Tailscale）

通知とマイクには HTTPS が要ります。Tailscale の serve で、`agent-link-web` だけを tailnet の中に公開します。

```bash
tailscale serve --bg --https 4446 http://127.0.0.1:8400
tailscale serve status
```

- iPhone からは `https://<Mac名>.<tailnet>.ts.net:4446/` を開きます（4444 番は sai-moon の Web が使います）。
- `agent-link-web` は必ず `--host 127.0.0.1` で動かします（`agent-link-web serve` の既定です。`0.0.0.0` にすると Tailscale の外からも届きます）。
- **tailnet が自分 1 人のときだけ使います。** 合言葉は無いので、tailnet に届く人は誰でも全機能（エージェントの起動を含む）を使えます。他の人を招くときは、先に ACL で届く機械を絞るか、serve を止めます（`docs/switch-over.md` の 6）。

## エージェントとつなぐ

エージェントは **CLI を Bash で呼びます**（`agent-link`・`agent-link-web`）。MCP の `agent-link` はありません。
次の設定は **`agent-link init` がプロジェクトに書く**ので、手で登録しません。利用者全体の `~/.claude/` には入れません（init したプロジェクトのエージェントだけが対象です）。

| 書かれる物 | 中身 |
|---|---|
| `.agent-link/cc-settings.json` | CLI の許可（`Bash(agent-link *)`・`Bash(agent-link-web *)`）・cc のフック（下）・statusLine・不要な道具の禁止。run の cc に `--settings` で渡す（run を始めるたびに今の版に合わせる） |
| `.claude/commands/` | `/agent-link-finalize`（root の run を完了にする） |
| `.mcp.json` | 外部の MCP: `serena`・`playwright`・`donsetch`・`headroom` |
| `.claude/skills/` | design skill（`frontend-design`・`ui-ux-rules`） |
| `.serena/project.yml` | serena の言語サーバ（プロジェクトのファイルの拡張子から選ぶ） |
| `.agent-link/` | `engine/`・`blocks/`・`template/`・`settings.json`・`outputs/` |

前に init したプロジェクトは `agent-link init --update` で、配った block・template を新しい版にできます（変わる物は `.bak` に退避されます）。

### cc のフック

`.agent-link/cc-settings.json` の `hooks` に、次が入ります。プロジェクトの `.claude/settings.json` は書きません（sai-moon など、同じプロジェクトを使う別のアプリの物だからです）。

- `UserPromptSubmit`: `agent-link cc-prompt-hook`（入力が `/agent-link-finalize` のとき、エージェントに送らずに run を完了にする）。
- `apps/web/hooks/` に 2 本（質問 `cc_question.py`・セッションの登録 `cc_register.py`）が揃っているときだけ、`PreToolUse`（`AskUserQuestion`）と `SessionStart` のフックも入ります（状態を書く `PreToolUse`・`PostToolUse` のフックは、置き場の有無にかかわらず入ります）。置き場は `AGENT_LINK_WEB_HOOKS_DIR` で変えられます。

フックは agent-link の run の中（`AGENT_LINK_PARENT_RUN_ID` がある）でだけ動きます。Web が止まっていても cc は普通に動きます（終了コード 0）。フックの送り先は `AGENT_LINK_WEB_URL` で、agent-link が tmux の session に入れます（daemon の `AGENT_LINK_WEB_URL` → `http://127.0.0.1:${AGENT_LINK_WEB_PORT:-8400}`）。cc のコマンドの承認（PermissionRequest）は扱いません（headless の実行では来ない）。

### 外部の MCP

- **playwright**: `@playwright/mcp@0.0.82`（WebKit・iPhone 15 の画面・画面なし）。使うプロジェクトで最初に 1 回だけ `npx -y @playwright/mcp@0.0.82 install-browser webkit` を実行します（init 自身は通信しません）。版は固定です（`@latest` にすると WebKit の版とずれます）。
- **serena**: コードの読み書き。`.serena/project.yml` は init が書きます。
- **donsetch**: Web の取得・検索・クロール。
- **headroom**: 長い出力の圧縮と取り出し。

### design skill

`frontend-design`（配色・文字組み・余白。Apache-2.0）と `ui-ux-rules`（コントラスト・文字とタッチの大きさ・フォーカス・ダークモード・グラフ。MIT）の 2 つです。cc は `/frontend-design` のように呼びます。入れるのは規則の文書だけで、スクリプトは入りません。ライセンスは各 skill のフォルダにあります。

agent に渡す環境変数は `.agent-link/settings.json` か `$AGENT_LINK_HOME/settings.json` の `env` に書きます。書き方は [`docs/for-developers.md`](./docs/for-developers.md) の「`.agent-link/settings.json` の項目」にあります。

## sai-moon と並べて使う

agent-link と sai-moon は、同じ Mac・同じプロジェクトで並べて使えます。名前が重ならないようにしてあります。

| 物 | sai-moon | agent-link |
|---|---|---|
| 実行ファイル | `sai-link`・`sai-web` | `agent-link`・`agent-link-web` |
| 環境変数 | `SAI_*` | `AGENT_LINK_*` |
| 置き場 | `~/.sai`・プロジェクトの `.sai/` | `~/.agent-link`・プロジェクトの `.agent-link/` |
| Web | 8200 番（Tailscale 4444） | 8400 番（Tailscale 4446） |
| cc の設定 | プロジェクトの `.claude/settings.json` | `.agent-link/cc-settings.json`（`--settings` で渡す） |
| 完了のコマンド | `/sai-finalize` | `/agent-link-finalize` |

- cc は `--settings` のフックとプロジェクトのフックを両方動かします。どちらのフックも、相手の run の中では何もしません（agent-link は `AGENT_LINK_PARENT_RUN_ID` が無ければ、sai-moon はあれば素通り）。
- 前の agent-link（2026-10-16 より前）は、run のたびにプロジェクトの `.claude/settings.json` を agent-link 版で上書きしていました。sai-moon の run の質問が Web に届かないプロジェクトでは、そのプロジェクトで `sai-link init --update` を実行して sai-moon 版に戻します。

## 安全性

| 項目 | 確かめ方 |
|---|---|
| 合言葉は無い | 入口の守りは「127.0.0.1 だけで待ち受ける」ことと、Tailscale serve を自分だけの tailnet に限ることだけ |
| 待ち受けの住所 | `lsof -nP -iTCP:8400 -sTCP:LISTEN` で 127.0.0.1 だけと確かめる |
| 内部用の API が外から呼べない | `POST /api/register`・`POST /api/questions`・`POST /api/questions/{qid}/resolved`・`GET /api/questions/{qid}` は、`require_internal` で Mac 自身（127.0.0.1 / ::1）からだけ受ける |
| 他の DB を開いていない | プロジェクトの記録はすべて `agent-link` の CLI を通す |
| 読み書きするファイルを iPhone から指定できない | run の md・定義の置き場は CLI が名前から決める。名前は Web が先に形を検査する |
| 壊れた定義を保存しない | 定義の保存前に `agent-link` が検査し、理由を出す |
| 遠隔の機械から Mac に入れる物を絞っている | `agent-link remote setup` は Mac の `~/.ssh/authorized_keys` に強制コマンドの鍵（`command="… remote-gate",restrict <公開鍵>`）を 1 行足す。門番は決めた命令と環境変数だけを通す（`docs/remote.md`・`docs/for-developers.md` の `remote-gate`）。使わないなら `remote setup` を実行しない |

## テスト（開発者向け）

テストの命令は [`AGENTS.md`](./AGENTS.md) の「テストと確認」にあります（Rust は `cargo test --workspace --no-fail-fast`、Web の画面は `apps/web/tests/test_*.js`、サーバは `apps/web/tests/test_*.py`）。CI（`.github/workflows/ci.yml`）は Linux（ubuntu-latest）で同じ命令を回します。

画面の確認は 8297 番など別の番号で行います。**Mac で動いている Web（8400 番）は、確認のために止めたり再起動したりしません。**

## 文書

| 文書 | 役割 |
|---|---|
| [`README.md`](./README.md)（この文書） | 入れ方（台本）・最初の一歩・ポート・環境変数と `config` の表・文書の一覧 |
| [`docs/design.md`](./docs/design.md) | 現在の設計（不変条件・層と境界・守る良い所・やらないこと） |
| [`docs/for-developers.md`](./docs/for-developers.md) | **契約の正 1**。終了コード・標準出力と標準エラー・`--json`・`schema --json` の形・プロジェクトの決め方・`AGENT_LINK_HOME` の規則・終了コードから HTTP への読み替え・Web が読む JSON の項目・`registry.json` と `settings.json` の形 |
| [`docs/integration.md`](./docs/integration.md) | **契約の正 2**。Web の HTTP の住所と画面の住所の一覧 |
| [`docs/decisions.md`](./docs/decisions.md) | 過去の決定（日付・理由・状態） |
| [`ARCHITECTURE.md`](./ARCHITECTURE.md) | 層・置き場・ファイル構成・機能を足す手順 |
| [`CONCEPT.md`](./CONCEPT.md) | 概念と用語（run・card・doc・block・Workflow・画面の名前ほか） |
| [`AGENTS.md`](./AGENTS.md) | このリポジトリで作業するエージェントへの約束とテストの命令 |
| [`docs/remote.md`](./docs/remote.md) | 遠隔の機械で子の claude を動かす手順書 |
| [`docs/switch-over.md`](./docs/switch-over.md) | sai-moon と並べて使う手順書 |
