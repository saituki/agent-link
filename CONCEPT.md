# agent-link の概念と用語

この文書は、agent-link の専門的な概念と用語を 1 か所で説明する。用語の意味を調べるときは、まず §5 の索引から節を引く。

- **読み方**: 各節は短い説明と、「用語・意味・どこにあるか」の表でできている。「どこにあるか」は、画面の住所・CLI の命令・コードの場所（ファイル名と関数名・型名。行番号は書かない）のどれか。
- **言葉の方針**: 専門用語は英単語のまま使う（Next Action・History・PATH・Dashboard など）。この文書の案内文と説明は日本語で書く。画面の文言と `agent-link` の標準エラー・`--json` の人向けの文は英語（音声の言語は日本語）。`data-tab`・住所・API の名前などの識別子も英語のまま。
- **正はコード**: この文書とコードが食い違ったら、コードに合わせて直す。確かめきれていない文には（要確認）を付ける。
- **ほかの文書**: 使い方と入れ方は [`README.md`](./README.md)、変わらない物（不変条件）と層は [`docs/design.md`](./docs/design.md)、層・置き場・ファイル構成は [`ARCHITECTURE.md`](./ARCHITECTURE.md)、過去の決定は [`docs/decisions.md`](./docs/decisions.md)、このリポジトリで作業する人への指示は [`AGENTS.md`](./AGENTS.md)、Web の HTTP と画面の住所は [`docs/integration.md`](./docs/integration.md)、CLI の約束と Web が読む JSON は [`docs/for-developers.md`](./docs/for-developers.md)、遠隔の機械で子の claude を動かす手順は [`docs/remote.md`](./docs/remote.md)、sai-moon と並べて使う手順は [`docs/switch-over.md`](./docs/switch-over.md)。

---

## 1. 全体の見取り図

agent-link は、2 つの部品（実行エンジン・Web の窓口）でできている。人は CLI と Web を使い、エージェントは CLI を Bash で呼ぶ。画面は Web だけ。

```
 人: ブラウザ（iPhone / PC）              人・エージェント: CLI
        │ HTTPS                                      │
        ▼                                            │
   agent-link-web（apps/web・8400）                  │
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
| **Web の窓口** | 画面と、質問・通知・使用量だけを持ち、DB を開かない。 | `apps/web`、CLI は `apps/web/bin/agent-link-web`、待ち受けは 127.0.0.1:8400 |
| **waggle** | 文書に短いトークンを付けて受け渡す仕組み。vendor したクレート群。 | `waggle/`（`waggle-core`・`waggle-store`・`waggle-store-sqlite`）。使う側は `crates/agent-link-engine/src/waggle_integration.rs` |
| **`agent-link.db`** | 実行エンジンの DB。run・card・子タスクなどの記録が入る。Web は開かない。 | `<root>/.agent-link/engine/agent-link.db` |
| **プロジェクト（`<root>/.agent-link/`）** | 作業の単位となるフォルダ。中の `.agent-link/` に DB・block・template・settings・成果物（`outputs/`）を置く。 | `agent-link init` が作る。`AGENT_LINK_PROJECT_ROOT` か、今のフォルダから上へたどって `.agent-link/` を持つ最初のフォルダ |
| **`AGENT_LINK_HOME`** | 利用者全体の置き場。環境変数 `AGENT_LINK_HOME`（空でない値）が優先、無ければ `~/.agent-link`。登録簿・共通の block / template / settings・`web/` が入る。プロジェクトの `.agent-link/` とは別。 | `agent_link_engine::project::sai_home()`・`registry.sai_home()`（Python）。**2 つの規則をそろえる**（違いは `HOME` が無いときだけ。Rust は `None`、Python は `Path.home()`） |
| **拡張（extension）** | 基盤に後から足す機能。フォルダ 1 つ（`extension.json`）に、任意の言語の CLI・block と template・サイドバーのタブ 1 つ（`ui/`）・任意の service（外部サービスへの読み取りの中継）をまとめたもの。基盤は拡張の名前を知らない。有効かどうかはプロジェクトごとに決まる（`ext enable`）。 | 置き場は `AGENT_LINK_EXTENSIONS_DIR`（既定 `$AGENT_LINK_HOME/extensions`）。命令は `agent-link ext`（`docs/for-developers.md` §2.2）。宣言と作法の正は `crates/agent-link-engine/assets/template/ext-contract.md` |
| **登録簿（registry）** | 全プロジェクトの一覧。`agent-link init` が書き、Web のプロジェクト一覧が読む。 | `$AGENT_LINK_HOME/registry.json`、読む側は `apps/web/src/agent_link_web/registry.py` |

---

## 2. 実行と文書（実行エンジン）

実行エンジンは、エージェントを動かし（run）、仕事の依頼を札にして（card）、成果を文書として残し（doc）、文書を waggle トークンで受け渡す。

run の一生（フォルダは状態に合わせて移る）:

```
card（todo）    …… .agent-link/outputs/todo/<run_id>/      task.md だけがある。まだ動いていない
   │ agent-link card run
running         …… .agent-link/outputs/running/<run_id>/   cc が pane の中で動く
   │ 文書を保存する（agent-link doc write）と `file_written` の通知が残り、常駐が完了を検出する
review          …… .agent-link/outputs/review/<run_id>/    利用者の確認待ち
   │ agent-link run advance（review だけ進められる）
<yymmdd>        …… .agent-link/outputs/<yymmdd>/<run_id>/  Workflow に移した run（日付フォルダ）
```

| 用語 | 意味 | どこにあるか |
|---|---|---|
| **run（実行）** | エージェントが 1 回動いた記録。`run_id`（数）で呼ぶ。成果物は run フォルダに置く。 | `agent-link run list\|show\|tree\|log\|stats\|history\|md\|finalize\|advance\|send\|interrupt\|wait\|rm\|export`、`agent-link.db` |
| **`run_id`** | run の番号。run を指す引数を省くと、`agent-link` は環境変数 `AGENT_LINK_PARENT_RUN_ID`（`agent-link` が起動時に渡す「自分の run」）を使う。 | `agent-link` の引数 `RUN_ID`、`cmd_run.rs` |
| **run フォルダ** | run の成果物（`task.md`・`message.md`・`system-prompt.md`・`session/`・文書）を集めたフォルダ。状態に合わせて `todo`・`running`・`review`、Workflow へ送ると日付フォルダ `<yymmdd>` に移る。 | `.agent-link/outputs/<状態か日付>/<run_id>/`。`crates/agent-link-engine/src/run_dir.rs` の `owner_for`・`find_run_dir` |
| **日付フォルダ（date folder）** | Workflow に移した run が入るフォルダ `outputs/<yymmdd>/`。日付は一番上の親 run の実行開始日（ローカル時刻）。子の run は親の日付フォルダに入る。`run tree` では「todo / running / review / 日付 / 親 run / 子 run」の入れ子になる（子は run の `children`）。 | `run_dir.rs` の `run_date`・`root_run_date`・`is_date_owner`、`agent-link run tree --json` |
| **card（カード）** | まだ実行していない run。todo にある `task.md`（frontmatter と依頼文）。 | `agent-link card create\|schedule\|run\|options\|docs`、`crates/agent-link-engine/src/kanban.rs` |
| **`task.md`** | card と run の依頼書。frontmatter と本文の `# task_text` でできる。固定のキーは `creation_date`・`execution_date`・`run_id`・`block`・`thinking`・`model`・`description`（と、block の入力の `inputs`）、メタのキーは `source_doc`・`scheduled_at`・`executer`。古い `runner`・`provider`・`tag`・`round`・`global_round`・`after_job` などは読み飛ばし、書き出さない。 | run フォルダの直下。読み書きは `kanban.rs`（`FIXED_FM_KEYS`・`META_FM_KEYS`・`LEGACY_FM_KEYS`） |
| **`# task_text`** | `task.md` の依頼文の節。エージェントが読んで動く。`waggle:<トークン>` と書くと、実行時にその位置へ文書の本文が埋め込まれる（1 段だけ）。 | `task.md` の本文。展開は `crates/agent-link-engine/src/placeholder.rs` |
| **doc（文書）** | waggle トークンの付いた文書。run フォルダに保存し、他の run が読む。 | `agent-link doc write\|read\|list\|search` |
| **waggle トークン** | 文書を指す短い名前（例 `AXANSMpo`）。文書は必ずトークンで渡し、パスは渡さない。参照は `waggle:<トークン>:path:.agent-link/outputs/*/<run_id>/<ファイル名>` の形（正はトークン）。 | `waggle/`、`waggle_integration.rs`。sharer の名前は `agent-link` |
| **block** | エージェントの役の定義。frontmatter に `model`・`thinking`（既定値。親の指定が優先）・`templates`・`worktree`・`enable` などを書き、本文が役の指示になる。 | `.agent-link/blocks/<id>.md`・`$AGENT_LINK_HOME/blocks/`。既定の定義は `crates/agent-link-engine/assets/blocks/`。型は `block_config.rs` |
| **template** | block の本文に差し込む共通の決まり。block の `templates:` に名前を並べる。名前は `link-base`・`link-cli`・`link-task`・`link-user`・`link-report` など。 | `.agent-link/template/<名前>.md`・`$AGENT_LINK_HOME/template/`。既定は `crates/agent-link-engine/assets/template/` |
| **def（定義）** | block・template・settings の総称と、それを読み書きする命令。 | `agent-link def list\|show\|write\|delete\|preview`、画面は Projects の Settings（設定） |
| **settings** | プロジェクトの設定（`default_model`・`default_thinking_level`・`wip`・`max_concurrent`・`pre_run`・`write_check`・`env`）。`$AGENT_LINK_HOME/settings.json` が、プロジェクトの `.agent-link/settings.json` を**キー単位で上書き**する。 | `.agent-link/settings.json`・`$AGENT_LINK_HOME/settings.json` |
| **cc（Claude Code）** | card と子タスクを動かす唯一の実行者。実行者を選ぶ項目（`runner`）は無い。起動は 2 つの経路（Card で待たずに起こす・`task create` で待つ）。 | `agent-link card run`・`agent-link task create`、`crates/agent-link-engine/src/cc_command.rs` |
| **`executer`** | card を**起こす人**の区別（メタのキー）。`human`（既定。`card run` と常駐の予約実行で起こす）か `agent`（エージェントが子の run として起こす。人が起こす道は断る）。古い `professor` は `agent` として読む。 | `kanban.rs` の `parse_executer`・`executer()`・`is_agent_executer()` |
| **常駐（daemon）** | プロジェクトごとの背景のプロセス。予約した card・ジョブ待ちの card の実行と、状態の見張り・完了の検出をする。手で起こす（launchd は使わない）。 | `agent-link daemon start\|stop\|status\|reload\|recover`、`crates/agent-link-engine/src/daemon.rs` |
| **review** | run の状態のひとつ。完了して利用者の確認を待っている。`agent-link run advance` で Workflow（日付フォルダ）へ進める。 | `.agent-link/outputs/review/`、`run_dir.rs` の `REVIEW_OWNER` |
| **Workflow** | Projects の画面のひとつ。run を札と線で描いた図（日付の区切り線つき）と、Kanban。札は `model`・`thinking` を持つ。子の run（`task create` で作った run）は、画面の `Subtasks` の切り替え（`?subtasks=1`）で出す。配置は `agent-link` が計算する。 | `#/project/<名前>/workflow`、`agent-link workflow [--subtasks]`、`workflow.js` |
| **Kanban** | card と run を 3 つのレーン（todo・running・review）に分けた見方。 | `kanban.rs` の `KanbanLane`・`classify_lane` |
| **擬似 pane（headless）** | headless の run の区画。id は `hl-` で始まる（`hl-` + 12 桁の 16 進）。状態は `state.json` に書かれ、Web は一覧の `live` の判定に使う。herdr の窓は使わない（`herdr_pane_id` のキー名だけ残る）。 | `crates/agent-link-engine/src/headless.rs`、`state.json` |
| **子タスク** | 親の run が作る子の run。親は `task create` で依頼し、子は最後に `task report` で報告し、親が `task finalize` で確定する。 | `agent-link task create\|execute\|wait\|list\|show\|capture\|send\|interrupt\|delete\|finalize\|requeue\|report\|question` |
| **worktree（作業用コピー）** | block の `worktree:` が決める、run の作業フォルダの分け方。`none`（分けない）・`isolation`（git の worktree とブランチ `<block 名>-<run_id>` を作る）・ブランチ名そのまま。 | `orchestrator.rs` の `resolve_worktree_branch` |
| **finalize** | 完了を確定すること。root の run は `agent-link run finalize [RUN_ID] --report <文>`、子タスクは親が `agent-link task finalize <タスク ID>`。子タスクの run フォルダは、確定するまで `running` のまま。 | `cli.rs` の `RunCmd::Finalize`・`TaskCmd::Finalize`、`run_dir.rs` の `subtask_owner` |
| **`host`（遠隔の子）** | `task create` の入力の `host`。子の claude を、Mac の `~/.ssh/config` の Host 名の機械（例 `wsl-server`）で動かす。状態・DB・文書は Mac のまま。子の画面は Mac の pane の中の `ssh -t <host> 'claude …'`。 | `agent-link task create`（JSON の `host`・`cwd`）、`crates/agent-link-engine/src/remote.rs`、手順は `docs/remote.md` |
| **`remote setup`** | 遠隔の機械に中継と鍵を置き、Mac の `~/.ssh/authorized_keys` に門番の行を足して、疎通を確かめる一度だけの準備。 | `agent-link remote setup <host>`、`crates/agent-link-cli/src/cmd_remote.rs`。実行は利用者が行う |
| **`remote check`** | 遠隔の機械の疎通を 4 項目（Mac から遠隔・claude・中継・遠隔から Mac）で確かめる。 | `agent-link remote check <host>` |
| **門番（`remote-gate`）** | 遠隔の機械から来た命令を Mac で受ける入口。`authorized_keys` の強制コマンド（`command="… agent-link remote-gate",restrict`）で起こり、通す命令だけを Mac の `agent-link` で実行する。内部用（`--help` に出ない）。 | `agent-link remote-gate`、`remote.rs` の `gate_plan` |
| **中継（relay）** | 遠隔の機械に置く `agent-link` の代わりのスクリプト。遠隔の claude のフックと `agent-link` の命令を、ssh で Mac の門番に送る。 | 遠隔の `~/.agent-link-remote/bin/agent-link`、本文は `remote.rs` の `shim_script` |
| **`/agent-link-finalize`** | cc の命令。root の run を、エージェントに送らずに完了にする。`init` が `.claude/commands/agent-link-finalize.md` に書く。 | `agent-link cc-prompt-hook`、`crates/agent-link-engine/src/cc_command.rs` |

---

## 3. Web の画面

画面は `apps/web/src/agent_link_web/static/` にある。新しい画面は `window.SaiRoutes` に登録し、`app.js` に画面ごとの分岐を足さない。幅 900px（`ui.js` の `AGENT_LINK_WIDE_PX`）を境に、狭い画面は下のタブの 1 列、広い画面は左の縦のタブと 2 列になる。

| 用語 | 意味 | どこにあるか |
|---|---|---|
| **Home** | 最初に出る画面。全プロジェクトの合計・Questions（質問）・todo・最近のアクティビティ・動いているもの・止まっている常駐・Claude のカード（cc の 5 時間枠・7 日枠と費用）を並べる。 | `#/home`、`home.js`、`GET /api/home`・`GET /api/usage`（`{"cc", "saved_at"}`） |
| **Projects** | プロジェクトの一覧と、その中の Dashboard・Workflow・List・Settings（設定）・会話検索（セッション検索）。 | `#/projects`、`#/project/<名前>/…`（`project.js`・`linkview.js`） |
| **Chats** | coding agent の会話を読み、指示を送り、質問に答える。 | `data-tab="agent"`、`app.js` |
| **Dashboard** | 期間ごとの実行数・成功率・トークン量・時間の数字とグラフ。Projects にある。 | `#/project/<名前>` |
| **List** | Projects の画面のひとつ。run のツリーから 1 つ選び、Log・Stats・History・task・message・report・system・waggle の表示で見る。 | `#/project/<名前>/list[/<run_id>/<表示>]`、`linkview.js` |
| **Settings（設定）** | Projects の画面のひとつ（画面の表示は `Settings`）。block・template・settings を読み書きする。 | `#/project/<名前>/defs`、`defs.js`、`linkview.js` の `LV_MODES` |
| **会話検索（セッション検索）** | Projects の画面のひとつ。会話の履歴を検索する。 | `#/project/<名前>/sessions`、`sessionsearch.js`、`agent-link session search` |
| **先着の答え** | 質問を PC と iPhone の両方に同時に出し、先に答えた方を採用する決まり。 | `apps/web/src/agent_link_web/questions.py` |
| **`SaiRoutes`** | 画面を登録する場所。各画面の JS が「住所の照合と描画」を `window.SaiRoutes` に足す。 | `app.js` |
| **`saiTerm`（用語の札）** | 画面の用語に付ける、押すと説明が出る札。説明は `AGENT_LINK_TERMS` に書く。 | `ui.js` |
| **画面の版** | 画面ファイルを変えて配るとき、`sw.js` の `CACHE` と `index.html` の `?v=` をそろえて上げる。上げないと iPhone に古い画面が残る。 | `sw.js`・`index.html` |
| **`data-tab`** | 下のタブ（左の縦のタブ）の識別子。`home`・`project`・`agent`。 | `index.html` |
| **住所（`#/…`）** | 画面の URL の `#` 以降。 | `app.js` と各画面の JS |

---

## 4. CLI の約束

`agent-link` は次の約束を守る。**全文は [`docs/for-developers.md`](./docs/for-developers.md)。** ここには 1 行の意味だけを書く。

| 用語 | 意味 | どこにあるか |
|---|---|---|
| **終了コード** | 0 成功・2 入力の誤り・3 見つからない・4 状態の衝突・1 その他の失敗。失敗のとき標準出力は空で、理由は標準エラーの最後の行。 | `docs/for-developers.md` §1 |
| **`--json`** | 成功したとき、標準出力に JSON を 1 つだけ書く指定。 | 各命令 |
| **`schema --json`** | 命令の一覧（`bin`・`version`・`commands[]`）を出す命令。プロジェクトを要らない。 | `agent-link schema --json` |
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
- **block** — エージェントの役の定義 — §2
- **cc（Claude Code）** — card と子タスクを動かす唯一の実行者 — §2
- **card（カード）** — まだ実行していない run — §2
- **Chats** — coding agent の会話を読み、指示を送り、質問に答える — §3
- **Dashboard** — 期間ごとの実行数・成功率・トークン量・時間の数字とグラフ — §3
- **`data-tab`** — 下のタブ（左の縦のタブ）の識別子 — §3
- **def（定義）** — block・template・settings の総称と、それを読み書きする命令 — §2
- **doc（文書）** — waggle トークンの付いた文書 — §2
- **`executer`** — card を起こす人（`human` / `agent`） — §2
- **`ext enable`** — 拡張をプロジェクトで有効にする命令 — §1
- **`ext-contract`** — 拡張の作者向けの契約の正（template の名前） — §1
- **extension（拡張）** — 基盤に後から足す機能 — §1
- **`extension-plan`** — 拡張を作るための block — §1
- **`extension.json`** — 拡張の宣言（名前・CLI・画面・service） — §1
- **finalize** — 完了を確定すること — §2
- **Home** — 最初に出る画面 — §3
- **`host`（遠隔の子）** — 子の claude を ssh 先の機械で動かす入力 — §2
- **`--json`** — 成功したとき、標準出力に JSON を 1 つだけ書く指定 — §4
- **Kanban** — card と run を 3 レーンに分けた見方 — §2
- **List** — Projects の画面のひとつ — §3
- **Projects** — プロジェクトの一覧と、その中の画面 — §3
- **`remote check`** — 遠隔の機械の疎通を確かめる命令 — §2
- **`remote setup`** — 遠隔の機械の一度だけの準備 — §2
- **review** — 完了して利用者の確認を待っている状態 — §2
- **run（実行）** — エージェントが 1 回動いた記録 — §2
- **`SaiRoutes`** — 画面を登録する場所 — §3
- **`saiTerm`（用語の札）** — 押すと説明が出る札 — §3
- **`schema --json`** — 命令の一覧を出す命令 — §4
- **service** — 拡張の外部サービスへの読み取りの中継（宛先は `config`） — §1
- **Settings（設定）** — block・template・settings を読み書きする画面 — §3
- **`task.md`** — card と run の依頼書 — §2
- **`# task_text`** — `task.md` の依頼文の節 — §2
- **template** — block の本文に差し込む共通の決まり — §2
- **waggle トークン** — 文書を指す短い名前 — §2
- **Workflow** — run を札と線で描いた図と Kanban — §2

### 日本語

- **画面の版** — `sw.js` の `CACHE` と `index.html` の `?v=` — §3
- **拡張（extension）** — 基盤に後から足す機能 — §1
- **日付フォルダ（date folder）** — Workflow に移した run が入る `outputs/<yymmdd>/` — §2
- **会話検索（セッション検索）** — 会話の履歴を検索する画面 — §3
- **子タスク** — 親の run が作る子の run — §2
- **常駐（daemon）** — プロジェクトごとの背景のプロセス — §2
- **終了コード** — 0・2・3・4・1 の意味 — §4
- **擬似 pane（headless）** — headless の run の区画 — §2
- **先着の答え** — 先に答えた方を採用する決まり — §3
- **中継（relay）** — 遠隔の機械から Mac の門番へ命令を送るスクリプト — §2
- **登録簿（registry）** — 全プロジェクトの一覧 — §1
- **門番（`remote-gate`）** — 遠隔の機械から来た命令を Mac で受ける入口 — §2
- **プロジェクト** — 作業の単位となるフォルダ — §1
- **住所（`#/…`）** — 画面の URL の `#` 以降 — §3
