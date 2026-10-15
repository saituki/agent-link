# agent-link の概念と用語

この文書は、agent-link の専門的な概念と用語を 1 か所で説明する。用語の意味を調べるときは、まず §5 の索引から節を引く。

- **読み方**: 各節は短い説明と、「用語・意味・どこにあるか」の表でできている。「どこにあるか」は、画面の住所・CLI の命令・コードの場所（ファイル名と関数名・型名。行番号は書かない）のどれか。
- **言葉の方針**: 専門用語は英単語のまま使う（Next Action・History・PATH・Dashboard など）。案内文と説明は日本語で書く。`data-tab`・住所・API の名前などの識別子も英語のまま。
- **正はコード**: この文書とコードが食い違ったら、コードに合わせて直す。確かめきれていない文には（要確認）を付ける。
- **ほかの文書**: 使い方と入れ方は [`README.md`](./README.md)、層の分け方と守る約束は [`ARCHITECTURE.md`](./ARCHITECTURE.md)、このリポジトリで作業する人への指示は [`AGENTS.md`](./AGENTS.md)、API と画面の住所は [`docs/integration.md`](./docs/integration.md)、CLI の約束と Web が読む JSON は [`docs/for-developers.md`](./docs/for-developers.md)、sai-moon からの切り替えは [`docs/switch-over.md`](./docs/switch-over.md)。

---

## 1. 全体の見取り図

agent-link は、2 つの部品（実行エンジン・Web の窓口）でできている。人は CLI と Web を使い、エージェントは CLI を Bash で呼ぶ。画面は Web だけ。

```
 人: ブラウザ（iPhone / PC）              人・エージェント: CLI
        │ HTTPS                                      │
        ▼                                            │
   agent-link-web（apps/web・8200）                  │
        │  agent-link の CLI（--json）だけを使う      │
        └──────────────────────┬─────────────────────┘
                               ▼
                         agent-link（実行エンジン）
                    （agent-link.db・.agent-link/outputs）
```

| 用語 | 意味 | どこにあるか |
|---|---|---|
| **agent-link** | アプリ全体の名前と、実行エンジンの実行ファイルの名前。アプリ（主に sai-tree）を開発する道具。 | リポジトリ全体（`ARCHITECTURE.md` §1.1） |
| **実行エンジン** | 基盤。coding agent の実行・カード・常駐・文書を扱う。Web を知らない。 | `crates/agent-link-engine`・`agent-link-view`・`agent-link-cli`。実行ファイルは `agent-link` |
| **Web の窓口** | 画面と、質問・通知・使用量だけを持ち、DB を開かない。 | `apps/web`、CLI は `apps/web/bin/agent-link-web`、待ち受けは 127.0.0.1:8200 |
| **waggle** | 文書に短いトークンを付けて受け渡す仕組み。vendor したクレート群。 | `waggle/`（`waggle-core`・`waggle-store`・`waggle-store-sqlite`）。使う側は `crates/agent-link-engine/src/waggle_integration.rs` |
| **`agent-link.db`** | 実行エンジンの DB。run・card・子タスクなどの記録が入る。Web は開かない。 | `<root>/.agent-link/engine/agent-link.db` |
| **プロジェクト（`<root>/.agent-link/`）** | 作業の単位となるフォルダ。中の `.agent-link/` に DB・block・template・settings・成果物（`outputs/`）を置く。 | `agent-link init` が作る。`AGENT_LINK_PROJECT_ROOT` か、今のフォルダから上へたどって `.agent-link/` を持つ最初のフォルダ |
| **`AGENT_LINK_HOME`** | 利用者全体の置き場。環境変数 `AGENT_LINK_HOME`（空でない値）が優先、無ければ `~/.agent-link`。登録簿・共通の block / template / settings・`web/` が入る。プロジェクトの `.agent-link/` とは別。 | `agent_link_engine::project::sai_home()`・`registry.sai_home()`（Python）。**2 つの規則をそろえる** |
| **登録簿（registry）** | 全プロジェクトの一覧。`agent-link init` が書き、Web のプロジェクト一覧が読む。 | `$AGENT_LINK_HOME/registry.json`、読む側は `apps/web/src/agent_link_web/registry.py` |

---

## 2. 実行と文書（実行エンジン）

実行エンジンは、エージェントを動かし（run）、仕事の依頼を札にして（card）、成果を文書として残し（doc）、文書を waggle トークンで受け渡す。

run の一生（フォルダは状態に合わせて移る）:

```
card（todo）    …… .agent-link/outputs/todo/<run_id>/      task.md だけがある。まだ動いていない
   │ agent-link card run
running         …… .agent-link/outputs/running/<run_id>/   runner が pane の中で動く
   │ 文書を保存する（agent-link doc write）と `file_written` の通知が残り、常駐が完了を検出する
review          …… .agent-link/outputs/review/<run_id>/    利用者の確認待ち
   │ agent-link run advance（次の状態へ進める。review → done → Workflow）
done            …… .agent-link/outputs/done/<run_id>/      確認済み（Workflow へ移すと R<global_round>/ になる）
```

| 用語 | 意味 | どこにあるか |
|---|---|---|
| **run（実行）** | エージェントが 1 回動いた記録。`run_id`（数）で呼ぶ。成果物は run フォルダに置く。 | `agent-link run list\|show\|tree\|log\|stats\|history\|md\|finalize\|advance\|send\|interrupt\|wait\|rm\|export`、`agent-link.db` |
| **`run_id`** | run の番号。run を指す引数を省くと、`agent-link` は環境変数 `AGENT_LINK_PARENT_RUN_ID`（`agent-link` が起動時に渡す「自分の run」）を使う。 | `agent-link` の引数 `RUN_ID`、`cmd_run.rs` |
| **run フォルダ** | run の成果物（`task.md`・`message.md`・`system-prompt.md`・`session/`・文書）を集めたフォルダ。状態に合わせて `todo`・`running`・`review`・`done`、Workflow へ送ると `R<global_round>` に移る。 | `.agent-link/outputs/<状態>/<run_id>/`。`crates/agent-link-engine/src/run_dir.rs` の `owner_for`・`find_run_dir` |
| **card（カード）** | まだ実行していない run。todo にある `task.md`（frontmatter と依頼文）。 | `agent-link card create\|schedule\|run\|start\|finish\|options\|docs`、`crates/agent-link-engine/src/kanban.rs` |
| **`task.md`** | card と run の依頼書。frontmatter（`runner`・`block`・`model`・`thinking`・`provider`・`executer`・`scheduled_at`・`after_job`・`source_doc` など）と本文の `# task_text` でできる。 | run フォルダの直下。読み書きは `kanban.rs`（`FIXED_FM_KEYS`・`META_FM_KEYS`） |
| **`# task_text`** | `task.md` の依頼文の節。エージェントが読んで動く。`waggle:<トークン>` と書くと、実行時にその位置へ文書の本文が埋め込まれる（1 段だけ）。 | `task.md` の本文。展開は `crates/agent-link-engine/src/placeholder.rs` |
| **doc（文書）** | waggle トークンの付いた文書。run フォルダに保存し、他の run が読む。 | `agent-link doc write\|read\|list\|search` |
| **waggle トークン** | 文書を指す短い名前（例 `AXANSMpo`）。文書は必ずトークンで渡し、パスは渡さない。参照は `waggle:<トークン>:path:.agent-link/outputs/*/<run_id>/<ファイル名>` の形（正はトークン）。 | `waggle/`、`waggle_integration.rs`。sharer の名前は `agent-link` |
| **block** | エージェントの役の定義。frontmatter に `model`・`thinking`（既定値。親の指定が優先）・`templates`・`worktree`・`enable` などを書き、本文が役の指示になる。 | `.agent-link/blocks/<id>.md`・`$AGENT_LINK_HOME/blocks/`。既定の定義は `crates/agent-link-engine/assets/blocks/`。型は `block_config.rs` |
| **template** | block の本文に差し込む共通の決まり。block の `templates:` に名前を並べる。名前は `link-base`・`link-cli`・`link-task`・`link-user`・`link-report` など。 | `.agent-link/template/<名前>.md`・`$AGENT_LINK_HOME/template/`。既定は `crates/agent-link-engine/assets/template/` |
| **def（定義）** | block・template・settings の総称と、それを読み書きする命令。 | `agent-link def list\|show\|write\|delete\|preview`、画面は Projects の Settings（設定） |
| **settings** | プロジェクトの設定（`context`・`job_status` など）。`$AGENT_LINK_HOME/settings.json` が、プロジェクトの `.agent-link/settings.json` を**キー単位で上書き**する。 | `.agent-link/settings.json`・`$AGENT_LINK_HOME/settings.json` |
| **runner** | card を動かす実行者。cc（Claude Code。既定）・`gl`（gemini の端末アプリ）・`user`（利用者が自分で行う札）。pi は廃止し、古い値は cc として読む。起動は 2 つの経路（Card で待たずに起こす・`task create` で待つ）。 | `crates/agent-link-engine/src/runner.rs` の `RunnerKind`、`agent-link card start\|finish` |
| **`executer`**（廃止） | card を**起こす人**の区別。古い Card の `executer` 項目は読むだけ（`agent`・`professor` は `agent` として読む）。`runner` とは別の項目。 | `kanban.rs` の `parse_executer`・`executer()` |
| **常駐（daemon）** | プロジェクトごとの背景のプロセス。予約した card・ジョブ待ちの card の実行と、状態の見張り・完了の検出をする。手で起こす（launchd は使わない）。 | `agent-link daemon start\|stop\|status\|reload\|recover`、`crates/agent-link-engine/src/daemon.rs` |
| **`job_status`** | `.agent-link/settings.json` の項目。実験などのジョブの状態を確かめる命令（文字の配列。`{job}` がジョブの ID に置き換わる）。出力の `state` が `done`・`failed`・`cancelled`・`lost` になればジョブは終わり。 | `daemon.rs` の `job_status_command` |
| **review** | run の状態のひとつ。完了して利用者の確認を待っている。 | `.agent-link/outputs/review/`、`run_dir.rs` の `REVIEW_OWNER` |
| **Workflow** | Projects の画面のひとつ。run を札と線で描いた図と、Kanban。配置は `agent-link` が計算する。 | `#/project/<名前>/workflow`、`agent-link workflow`、`workflow.js` |
| **Kanban** | card と run を 4 つのレーン（todo・running・review・done）に分けた見方。 | `kanban.rs` の `KanbanLane`・`classify_lane` |
| **擬似 pane（headless）** | headless の run の区画。id は `hl-` で始まる（`hl-` + 12 桁の 16 進）。状態は `state.json` に書かれ、Web は一覧の `live` の判定に使う。herdr の窓は使わない（`herdr_pane_id` のキー名だけ残る）。 | `crates/agent-link-engine/src/headless.rs`、`state.json` |
| **子タスク** | 親の run が作る子の run。親は `task create` で依頼し、子は最後に `task report` で報告し、親が `task finalize` で確定する。 | `agent-link task create\|execute\|wait\|list\|show\|capture\|send\|interrupt\|delete\|finalize\|requeue\|report\|question` |
| **worktree（作業用コピー）** | block の `worktree:` が決める、run の作業フォルダの分け方。`none`（分けない）・`isolation`（git の worktree とブランチ `<block 名>-<run_id>` を作る）・ブランチ名そのまま。 | `orchestrator.rs` の `resolve_worktree_branch` |
| **finalize** | 完了を確定すること。root の run は `agent-link run finalize [RUN_ID] --report <文>`、子タスクは親が `agent-link task finalize <タスク ID>`。子タスクの run フォルダは、確定するまで `running` のまま。 | `cli.rs` の `RunCmd::Finalize`・`TaskCmd::Finalize`、`run_dir.rs` の `subtask_owner` |
| **`context`（settings）** | block の `context: true` のとき、実行の直前に `.agent-link/settings.json` の `context` の命令を実行し、標準出力を最初の文の `# context` の節に入れる。 | `crates/agent-link-engine/src/pre_run.rs` |
| **`/agent-link-finalize`** | cc の命令。root の run を、エージェントに送らずに完了にする。`init` が `.claude/commands/agent-link-finalize.md` に書く。 | `agent-link cc-prompt-hook`、`crates/agent-link-engine/src/cc_command.rs` |

---

## 3. Web の画面

画面は `apps/web/src/agent_link_web/static/` にある。新しい画面は `window.SaiRoutes` に登録し、`app.js` に画面ごとの分岐を足さない。幅 900px（`ui.js` の `AGENT_LINK_WIDE_PX`）を境に、狭い画面は下のタブの 1 列、広い画面は左の縦のタブと 2 列になる。

| 用語 | 意味 | どこにあるか |
|---|---|---|
| **Home** | 最初に出る画面。全プロジェクトの合計・Questions（質問）・todo・最近のアクティビティ・動いているもの・止まっている常駐を並べる。 | `#/home`、`home.js`、`GET /api/home` |
| **Projects** | プロジェクトの一覧と、その中の Dashboard・Workflow・List・Settings（設定）・コマンド・会話検索（セッション検索）。 | `#/projects`、`#/project/<名前>/…`（`project.js`・`linkview.js`） |
| **Chats** | coding agent の会話を読み、指示を送り、質問に答える。 | `data-tab="agent"`、`app.js` |
| **Account** | Usage（LLM の料金と制限）を出す画面。 | `#/account`、`account.js`・`usage.js` |
| **Usage** | LLM の料金と制限（Claude Code・OpenRouter・DeepSeek）。Account の中の区画。住所 `#/usage` は `#/account` へ送られ、内訳は `#/usage/<provider>/<account>`。 | `usage.js` |
| **Dashboard** | 期間ごとの実行数・成功率・トークン量・時間の数字とグラフ。Projects にある。 | `#/project/<名前>` |
| **List** | Projects の画面のひとつ。run のツリーから 1 つ選び、Log・Stats・History・task・message・report・system・waggle の表示で見る。 | `#/project/<名前>/list[/<run_id>/<表示>]`、`linkview.js` |
| **Settings（設定）** | Projects の画面のひとつ（画面の表示は `Settings`）。block・template・settings を読み書きする。 | `#/project/<名前>/defs`、`defs.js`、`linkview.js` の `LV_MODES` |
| **コマンド** | Projects の画面のひとつ（画面の表示は `コマンド`）。`schema --json` から、`web: true` の命令を入力欄つきで出して実行する。 | `#/project/<名前>/commands`、`commands.js` |
| **会話検索（セッション検索）** | Projects の画面のひとつ（画面の表示は `会話検索`）。会話の履歴を検索する。 | `#/project/<名前>/sessions`、`sessionsearch.js`、`agent-link session search` |
| **先着の答え** | 質問を PC と iPhone の両方に同時に出し、先に答えた方を採用する決まり。 | `apps/web/src/agent_link_web/questions.py` |
| **合言葉** | Web の入口を守る値（`AGENT_LINK_WEB_TOKEN`）。起動すると `$AGENT_LINK_HOME/web/token` に 0600 で書かれ、エージェントの道具が読む。 | `config.py`・`main.py` の `require_token`・`write_token_file` |
| **`SaiRoutes`** | 画面を登録する場所。各画面の JS が「住所の照合と描画」を `window.SaiRoutes` に足す。 | `app.js` |
| **`saiTerm`（用語の札）** | 画面の用語に付ける、押すと説明が出る札。説明は `AGENT_LINK_TERMS` に書く。 | `ui.js` |
| **画面の版** | 画面ファイルを変えて配るとき、`sw.js` の `CACHE` と `index.html` の `?v=` をそろえて上げる。上げないと iPhone に古い画面が残る。 | `sw.js`・`index.html` |
| **`data-tab`** | 下のタブ（左の縦のタブ）の識別子。`home`・`project`・`agent`・`account`。 | `index.html` |
| **住所（`#/…`）** | 画面の URL の `#` 以降。 | `app.js` と各画面の JS |

---

## 4. CLI の約束

`agent-link` は次の約束を守る。全文は [`docs/for-developers.md`](./docs/for-developers.md)（§1）と [`ARCHITECTURE.md`](./ARCHITECTURE.md)（§2.3）。ここには 1 行の意味だけを書く。

| 用語 | 意味 | どこにあるか |
|---|---|---|
| **終了コード** | 0 成功・2 入力の誤り・3 見つからない・4 状態の衝突・1 その他の失敗。失敗のとき標準出力は空で、理由は標準エラーの最後の行。 | `docs/for-developers.md` §1 |
| **`--json`** | 成功したとき、標準出力に JSON を 1 つだけ書く指定。 | 各命令 |
| **`schema --json`** | 命令の一覧（`bin`・`version`・`commands[]`）を出す命令。プロジェクトを要らない。Web の「コマンド」画面が読む。 | `agent-link schema --json` |
| **`sai-progress`** | 時間のかかる命令が標準エラーに出す進み具合の行。`sai-progress step=<数>`・`sai-progress msg=<文>`。 | 標準エラー |
| **`AGENT_LINK_PROJECT_ROOT`** | 対象のプロジェクトの root。空なら、今のフォルダから上へたどって `.agent-link/` を持つ最初のフォルダ。 | 環境変数 |
| **`AGENT_LINK_PARENT_RUN_ID`** | エージェントが動いている run の番号。`agent-link` が起動時に渡し、`run_id` を省いた命令が「自分の run」として読む。 | 環境変数（`pre_run.rs` が渡す） |

`AGENT_LINK_HOME`（置き場）は §1。`AGENT_LINK_WEB_*`・`AGENT_LINK_BIN` などの表は [`README.md`](./README.md)。

---

## 5. 用語の索引

本文の表の左の列にある用語を、「用語 — 一言 — 節」の形で並べる。アルファベット（A〜Z）が先、日本語は五十音の順。詳しい意味と「どこにあるか」は、節の表を見る。

### A〜Z

- **`agent-link`** — アプリ全体と実行ファイルの名前 — §1
- **`agent-link.db`** — 実行エンジンの DB — §1
- **`AGENT_LINK_HOME`** — 利用者全体の置き場（既定 `~/.agent-link`） — §1
- **`AGENT_LINK_PARENT_RUN_ID`** — 自分の run の番号 — §4
- **`AGENT_LINK_PROJECT_ROOT`** — 対象のプロジェクトの root — §4
- **Account** — Usage を出す画面 — §3
- **block** — エージェントの役の定義 — §2
- **card（カード）** — まだ実行していない run — §2
- **Chats** — coding agent の会話を読み、指示を送り、質問に答える — §3
- **`context`（settings）** — block の `context: true` のとき、実行の直前に走る settings の命令 — §2
- **Dashboard** — 期間ごとの実行数・成功率・トークン量・時間の数字とグラフ — §3
- **`data-tab`** — 下のタブ（左の縦のタブ）の識別子 — §3
- **def（定義）** — block・template・settings の総称と、それを読み書きする命令 — §2
- **doc（文書）** — waggle トークンの付いた文書 — §2
- **`executer`** — card を起こす人（廃止。古い Card は読むだけ） — §2
- **finalize** — 完了を確定すること — §2
- **Home** — 最初に出る画面 — §3
- **`job_status`** — ジョブの状態を確かめる settings の命令 — §2
- **`--json`** — 成功したとき、標準出力に JSON を 1 つだけ書く指定 — §4
- **Kanban** — card と run を 4 レーンに分けた見方 — §2
- **List** — Projects の画面のひとつ — §3
- **Projects** — プロジェクトの一覧と、その中の画面 — §3
- **review** — 完了して利用者の確認を待っている状態 — §2
- **run（実行）** — エージェントが 1 回動いた記録 — §2
- **runner** — card を動かす実行者（cc・gl・user） — §2
- **`SaiRoutes`** — 画面を登録する場所 — §3
- **`saiTerm`（用語の札）** — 押すと説明が出る札 — §3
- **`schema --json`** — 命令の一覧を出す命令 — §4
- **Settings（設定）** — block・template・settings を読み書きする画面 — §3
- **`sai-progress`** — 進み具合の行 — §4
- **`task.md`** — card と run の依頼書 — §2
- **`# task_text`** — `task.md` の依頼文の節 — §2
- **template** — block の本文に差し込む共通の決まり — §2
- **Usage** — LLM の料金と制限 — §3
- **waggle トークン** — 文書を指す短い名前 — §2
- **Workflow** — run を札と線で描いた図と Kanban — §2

### 日本語

- **画面の版** — `sw.js` の `CACHE` と `index.html` の `?v=` — §3
- **会話検索（セッション検索）** — 会話の履歴を検索する画面 — §3
- **合言葉** — Web の入口を守る値 — §3
- **コマンド** — `web: true` の命令を入力欄つきで実行する画面 — §3
- **子タスク** — 親の run が作る子の run — §2
- **常駐（daemon）** — プロジェクトごとの背景のプロセス — §2
- **終了コード** — 0・2・3・4・1 の意味 — §4
- **擬似 pane（headless）** — headless の run の区画 — §2
- **先着の答え** — 先に答えた方を採用する決まり — §3
- **登録簿（registry）** — 全プロジェクトの一覧 — §1
- **プロジェクト** — 作業の単位となるフォルダ — §1
- **住所（`#/…`）** — 画面の URL の `#` 以降 — §3
