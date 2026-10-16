# CLI の約束（Web と実行エンジンの間）

対象: **`crates/`（`agent-link`）を直す人**。
目的: 直した結果として **agent-link-web（Web の窓口・8400 番）を壊さないこと**。
全体の話は [`integration.md`](./integration.md)（API と画面の住所）と [`../ARCHITECTURE.md`](../ARCHITECTURE.md)、言葉の意味は [`../CONCEPT.md`](../CONCEPT.md)。
この文書の JSON の項目は、Web のコード（`apps/web/src/agent_link_web/` と `static/`）が実際に読む物と、2026-10 に `target/release/` の `agent-link` を一時フォルダのプロジェクトで実行して確かめた物。**正はコード**。

---

## 0. 最初に読む節の早見表

| 直すもの | その節 | 最低限守ること |
|---|---|---|
| **CLI（`agent-link`）** | [§1](#1-cli-の約束) | 終了コード・失敗の出し方・`--json`・`schema --json` |
| **Web が呼ぶ命令** | [§2](#2-web-が呼ぶ命令と-web-が読む-json-の項目) | 命令の名前と、Web が読む JSON の項目名。`run export` の書き出しの形 |
| **エージェントとの約束**（settings・カードの frontmatter・報告・質問・フック） | [§3](#3-web-とエージェントの間の約束) | 項目の名前と置き場 |
| **項目を変える** | [§4](#4-項目を変えるときの決まり) | Web が読む項目を変えたら、この文書と Web を一緒に直す |
| **連携が壊れた** | [§5](#5-連携が壊れたときの見分け方) | 画面の文言から原因を引く |

---

## 1. CLI の約束

`agent-link` は、次の約束を守る。Web は終了コードと標準エラーの最後の行だけで失敗を扱う。

| # | 約束 |
|---|---|
| 1 | **終了コード**: 0 成功、2 入力の誤り（clap の使い方の誤りも 2）、3 見つからない、4 状態の衝突、1 その他の失敗 |
| 2 | **失敗したとき**: 標準出力には何も書かない。標準エラーの最後の行に、英語で理由を 1 行書く。clap の誤りは最後の行に `Invalid arguments: <clap の文の 1 行目>`。`--json` の返り値の人向けの文（`message` など）も英語 |
| 3 | **成功したとき**: `--json` があれば標準出力に JSON を 1 つだけ書く（前後に余計な文字を出さない） |
| 4 | **時間のかかる命令**は、標準エラーに `sai-progress step=<数>`・`sai-progress msg=<文>` を出してよい（1 行 = 1 つ。段落内の改行は空白にする） |
| 5 | **プロジェクトの決め方**: `AGENT_LINK_PROJECT_ROOT`（空でない値）→ cwd から上へたどり `.agent-link/` を持つ最初のフォルダ（ホームとそれより上は数えない）。見つからなければ 2。`AGENT_LINK_PROJECT_ROOT` が `.agent-link/` の無いフォルダを指すときも 2（「… has no .agent-link/: <path>. Run "agent-link init".」。何も作らない）。`init` は「`AGENT_LINK_PROJECT_ROOT` → cwd」だけ（`.agent-link/` が無くてよい）。`schema` はプロジェクトを要らない |
| 6 | **`schema --json`** で命令の一覧を出す（下） |

標準出力に `println!` やデバッグ表示を 1 行でも足すと、Web が **502 になる**（`apps/web/src/agent_link_web/cli.py`）。標準エラーには何を出してもよい（先頭 300 文字が Web のログに残る）。

実測の例（一時フォルダのプロジェクト）:

```
$ agent-link run md 999 --kind task      → 終了コード 3・標準エラーの最後の行「Not available yet.」
$ agent-link run list --bogus            → 終了コード 2・「Invalid arguments: unexpected argument '--bogus' found」
$ （プロジェクトの外で）agent-link run list --json → 終了コード 2・「Project not found. Run "agent-link init".」
$ agent-link run advance 1               → 終了コード 2・「Only a run in review can be moved.」（1 が review でないとき）
$ agent-link run rm 999 --yes            → 終了コード 3・「Run 999 not found.」（実行中の run は 4）
$ agent-link task list                   → 終了コード 2（run の外。`AGENT_LINK_PARENT_RUN_ID` が無い）
$ agent-link init --name 'bad/name'      → 終了コード 2・「Invalid project name: …」（何も作らない）
```

### `schema --json` の形

```json
{
  "bin": "agent-link", "version": "0.1.0",
  "commands": [
    {"path": ["card", "create"], "about": "カードを作成する（標準入力に JSON）",
     "stdin": "json", "json": true, "web": true,
     "args": [
       {"name": "period", "kind": "option", "type": "enum", "values": ["week","month","all"],
        "required": true, "multiple": false, "default": null, "help": "…", "web": true}
     ]}
  ]
}
```

| 項目 | 型 | 意味 |
|---|---|---|
| `bin` / `version` | 文字列 | 実行ファイルの名前と版 |
| `commands[].path` | 文字列の配列 | 命令の語（`["run", "list"]`）。サブコマンドの無い命令は 1 語 |
| `commands[].about` | 文字列 | 1 行の説明（clap の説明。命令の説明の文は今は日本語） |
| `commands[].stdin` | `"none"` / `"text"` / `"json"` / `"bytes"` | 標準入力の種類 |
| `commands[].json` | 真偽値 | `--json` を受けるか |
| `commands[].web` | 真偽値 | 出力に残っている印。Web の画面は読まない |
| `args[].name` | 文字列 | 引数の ID（clap の ID。オプションの綴りは `--` + 名前だが、`_` は `-` と書く物がある） |
| `args[].kind` | `"option"` / `"flag"` / `"positional"` | 引数の種類 |
| `args[].type` | `"string"` / `"int"` / `"float"` / `"enum"` / `"path"` | 値の型（enum は `values` に選択肢） |
| `args[].required` / `multiple` / `default` / `help` / `web` | 真偽値・真偽値・値・文字列・真偽値 | 必須・複数・既定・説明・Web に出してよいか |

- `schema --json` は命令を足すと自動で出る。プロジェクトを要らない。

### Web が CLI を呼ぶ約束

- env は `{**os.environ, "AGENT_LINK_PROJECT_ROOT": <root>}`、cwd は root。シェルは使わない（`shell=True` 禁止・引数は配列）。
- 終了コードから HTTP の状態へ: 2 → 400、3 → 404、4 → 409、1 とそのほか → 502。0 で JSON でない → 502。起動できない → 503、時間切れ → 504。`detail` は標準エラーの空でない最後の行（`sai-progress` の行を除く。英語）。
- 時間切れは既定 20 秒。個別に、daemon・workflow 30 秒、検索 60 秒（`control.py` の定数）。
- 実行ファイルの場所は `AGENT_LINK_BIN`。**バイナリ名（`agent-link`）と、Web が呼ぶ命令の名前を変えてはいけない**（変えると 503 / 400 になる）。
- `agent-link` は `AGENT_LINK_PROJECT_ROOT` を最優先で DB 解決に使う。Web は呼び出しのたびに必ず root を渡す。

---

## 2. Web が呼ぶ命令と、Web が読む JSON の項目

### 2.1 呼ばれ方の一覧

cwd は **そのプロジェクトの root**。

| 呼ぶ側 | Web が呼ぶ命令（呼ぶ側のモジュール） | 時間切れ |
|---|---|---|
| `control.py` | `run export`（60 秒に 1 回）・`project summary`・`project dashboard --period`・`run tree`・`run log\|stats\|history <id>`・`run md <id> --kind …`（保存は `--write`）・`run finalize\|advance <id>`・`run rm <id> --yes`・`run send\|interrupt <id>`・`doc list --run <id>`・`doc read <token>`・`doc search …`・`card options\|docs\|create\|run\|schedule`・`workflow [--subtasks]`・`def list\|show\|write\|delete`・`session search …`・`daemon start\|stop` | 10 秒（export）・20 秒（既定）・30 秒（daemon・workflow）・60 秒（検索） |

- どの呼び出しも**シェルを使わず**、引数は配列で渡される。
- 検索語や文のように `-` で始まりうる値は、最後に `--` を置いて渡される。
- **`target/release/` の実行ファイルを作り直す**（`cargo build --release`）。直しても作り直さないと Web は古いまま。

### 2.2 `agent-link` が出す JSON

#### `run export`（書き出し。Chats が読む）

`$AGENT_LINK_HOME/web/runs/<project_name>/<run_id>.json`（一時ファイル → rename で書く）。1 ファイル分の項目（`runs.py` の `_read_run`）。**足し算は安全、減らすのは危険。**

| 項目 | 型 | 何に使う | 無い・型が違うとき |
|---|---|---|---|
| `schema_version` | 数値（**1 固定**） | 受け入れ判定 | `1` 以外は**そのファイルを丸ごと捨てる** |
| `run_id` | 数値 | 実行の ID・URL `#/run/<project>/<run_id>` | 無ければ捨てる |
| `project_name` | 文字列 | 一覧の並び・URL | 無ければ捨てる |
| `runner` | 文字列（常に `cc`。項目は残してある） | 記録の読み分け | 無ければ `cc` と読む |
| `status` | 文字列 | 一覧に出す判定 | `running / completed / failed / cancelled` のみ表示。**それ以外（todo・pending）は出さない** |
| `started_at` / `updated_at` | 文字列（タイムゾーン無し） | どの日の実行か（**UTC とみて日本時間に変換**）・並び順 | 無ければ日付なしで一覧から落ちる |
| `finished_at` / `duration_ms` | 文字列 / 数値 | 実行ごとの表示 | 空欄でも動く |
| `description` / `block_name` | 文字列 | 一覧の行・検索 | 空文字扱い |
| `parent_run_id` | 数値 \| null | サブタスクを親の直下に字下げ | 通常の位置に出る |
| `model` | 文字列 \| null | 統計画面 | ― |
| `stats` | オブジェクト \| null | 日ごとの費用（`stats.cost_usd.total`・`cost_is_estimate`） | 実行中は `null`（想定内） |
| `agent_session_id` / `transcript_path` | 文字列 \| null | 質問カード・会話の中身 | 会話が読めない |
| `herdr_pane_id` | 文字列 \| null | headless の擬似 pane の id（`hl-` で始まる）。キー名は herdr の時代の名残で、中身は擬似 pane の id | 一覧の `live` と `headless` が付かない |

`herdr_pane_label` は Web では読まない（headless では pane を名前で探さないため）。

#### headless の `state.json`（Web が読む）

headless の実行の状態は実行エンジンが `<project_root>/.agent-link/engine/headless/<id>/state.json` に書く。
Web が読んでよいのはこのファイルだけ（`runs.py` の `headless_state`）。`<id>` は `hl-` + 12 桁の 16 進で，実行の書き出しの `herdr_pane_id` に入る。

| 項目 | 型 | 何に使う | 無い・型が違うとき |
|---|---|---|---|
| `state` | 文字列（`starting`（起動中） / `working` / `idle` / `blocked`（質問の答え待ち） / `stopped`） | 会話の `live` ランプ・通知 | `starting` / `working` / `idle` / `blocked` / `stopped` 以外は読まない |
| `partial` | 文字列 | 生成中の文の逐次表示（末尾 20000 文字まで） | 空文字 |
| `error` | 文字列 | 直前のターンの失敗の理由 | 空文字 |
| `turn` | 数値 | （Web では未使用） | ― |
| `pane_id` / `runner` / `updated_at` | 文字列 | （Web では未使用） | ― |

#### `project summary` → オブジェクト

| 項目 | 型 | 使うところ | 無い・違うとき |
|---|---|---|---|
| `running` | 数値 | 「実行中 N」 | 0 |
| `last_executed_at` | 文字列 \| null | 「5分前」と並び順 | 最後尾・時刻なし |

#### `project dashboard --period week|month|all` → オブジェクト

| 階層 | 項目 | 型 | 使途 | 無いとき |
|---|---|---|---|---|
| 直下 | `name` / `since` / `until` | 文字列 | 見出しと期間 | ― |
| 直下 | `period` | 文字列 | グラフ横軸のラベルの出し分け | 今週扱い |
| 直下 | `daemon_alive` | 真偽値 | ランプ・ボタン | 停止中 |
| 直下 | `daily` | 配列 | 日ごとの棒グラフ | グラフ空 |
| `daily[]` | `date` / `executions` / `tokens` | 文字列 / 数値 / 数値 | 棒とタップ中の 1 行 | 0 の棒 |
| 直下 | `by_model` | 配列（`model` / `tokens`） | ドーナツ | 「データがありません」 |
| 直下 | `by_block` | 配列（`block` / `tokens` / `executions`） | 横棒（上位 8） | 「データがありません」 |
| 直下 | `recent` | 配列 | 直近の実行 | 「この期間の実行はありません」 |
| `recent[]` | `run_id` / `parent_run_id` / `block` / `status` / `started_at` / `duration_ms` | 数値 / 数値 \| null / 文字列 / 文字列 / 文字列 / 数値 | 行の形と `#/run/...` の URL | ― |
| `summary` | `executions` / `completed` / `failed` / `running` / `success_rate` / `tokens_total` / `tokens_input` / `tokens_output` / `tokens_cache` / `duration_ms` / `cost_usd` | 数値 | 大きな数字と補足行 | 0 |

Web が `recent[]` に**追加で足す**項目: `description`（文字列）・`linkable`（真偽値・メッセージ画面が開けるか）・`project`（文字列）。実行エンジンの側では不要。

#### `run tree` → `{"items": [...]}`（左サイドバーと同じツリー）

`items[]` は depth 0 のフォルダで、並びは `todo`・`running`・`review`（空でも出す）と、Workflow に移した run の日付フォルダ（`yymmdd`・古い順）。run は `children` に子 run を持てる（同じフォルダの中に親がいるときだけ。孫も同じ。親が別のフォルダにいる run は、そのフォルダの直下に出る）。同じ run は 1 か所にしか出ない。

```json
{"items": [
  {"type": "folder", "id": "todo", "label": "todo", "depth": 0, "children": [
    {"type": "run", "run_id": 41, "label": "ID41/worker", "lane": "todo", "status": "todo",
     "executer": "human", "scheduled_at": "2026-10-17 09:00",
     "card": {"creation_date": "2026/10/16 11:22:14", "block": "worker", "model": "", "description": "…"}}]},
  {"type": "folder", "id": "running", "label": "running", "depth": 0, "children": []},
  {"type": "folder", "id": "review", "label": "review", "depth": 0, "children": [
    {"type": "run", "run_id": 40, "label": "ID40/kantoku", "lane": "review", "status": "completed",
     "executer": "human",
     "children": [{"type": "run", "run_id": 42, "label": "ID42/worker", "lane": "review",
                   "status": "completed", "executer": "human"}]}]},
  {"type": "folder", "id": "261015", "label": "261015", "depth": 0, "children": [
    {"type": "run", "run_id": 38, "label": "ID38/kantoku", "lane": "workflow", "status": "completed",
     "executer": "human", "children": [ … ]}]}
]}
```

| 項目 | 型 | 使うところ |
|---|---|---|
| `items[].type` / `id` / `label` / `depth` / `children[]` | `"folder"` / 文字列 / 文字列 / 数値 / 配列 | フォルダの行（`id` と `label` は `todo`・`running`・`review`・日付 `yymmdd`） |
| run ノード: `type` / `run_id` / `label` / `status` | `"run"` / 数値 / 文字列 / 文字列 | 行 |
| run ノード: `lane` | `"todo"` / `"running"` / `"review"` / `"workflow"` | 色・Workflow との対応（日付フォルダの run は `workflow`） |
| run ノード: `children[]` | 配列（子があるときだけ。run_id の昇順） | 子 run（`task create` で作った run）。何段でも入れ子になる |
| run ノード: `executer` | `"human"` / `"agent"` | カードの起動者（既定 `human`。`agent` の todo には画面が ▶ 実行を出さない） |
| todo の run ノード: `card` | `{creation_date, block, model, description}` | Dashboard とホームの todo の 2 行表示 |
| todo の run ノード: `scheduled_at` | `YYYY-MM-DD HH:MM`（予約があるときだけ） | ホームの「予約した todo」 |

旧形式の todo カード（`run_id` の無い物）は出さない。ホーム（`home.py`）は top レベルの `review`・`todo` フォルダの中の run ノードを読む。

#### `run log|stats|history <run_id>`

オブジェクトをそのまま画面に出す。`run log` の `connections[]`（作られた順。`direction` `from`/`to`・`kind` `task_text`/`task_create`/`task_output`・`run_id`・`block_name`・`description`・`waggle_token`・`file_path`）を Summary の Connection が読む。`block_name` / `exec_id` / `created_at` / `finished_at` / `input_fields[]` / `model` なども見出しに使う。

#### `run history <run_id> [--json]` と `run history <run_id> --call <CALL_ID> --json`（History）

`run history <run_id> --json` は次の項目を持つ（画面は `static/linkview.js` の `lvRenderHistory`）。

- `title` / `subtitle`（文字列）・`type_tokens`（`user` / `agent` / `toolCall` / `toolResult` / `thinking` の近似トークン数）。
- `events[]`（時刻順）: `time`・`type`（上の 5 種）・`name`・`tokens` は必ず持つ。`call_id`・`command`・`preview`（結果の本文の先頭 200 字）・`is_error`・`ids`・`result_tokens` は、その行にあるときだけ出す（無いときは JSON に出さない）。toolCall と、それに対応する toolResult は 1 行にまとめる。対になる toolCall が無い結果だけ `toolResult` の行になる。
- `cli`: `calls[]`（`command`・`count`・`errors`・`help`。多い順）・`failures[]`（`class`・`command`・`count`）。
  - `calls[].count` と `errors` は `--help`・`-h` の呼び出しを除く。その回数は `help` に数える（`count` には入らない・失敗にも数えない）。
  - 命令名は `command` の語の位置（先頭・`&&`・`|`・改行・`(`・`$(`・`` ` `` の後。`timeout`・`env`・`exec`・`xargs`・`perl -e` の後も含む）の `agent-link`・`agent-link-web` から読む。heredoc の本文と `#` の注釈は読まない。
  - `failures.class` は `input`（終了コード 2）・`not_found`（3）・`conflict`（4）・`other`（読めない・それ以外）。`failures.command` は命令名。
  - `cli` が無いのは古い CLI の JSON。
- cc の History は、その run の transcript（`phone_export::resolve_transcript`）を読む。全体で一番新しい cc セッションは使わない。

`--call <CALL_ID> --json` は、その呼び出しの全文 `{"call_id","name","command","input","result","is_error","truncated"}` を返す。`result` は 20000 字までで、超えたら切って `truncated: true`。`--json` を付けないと本文だけの文字を出すので、Web は必ず付ける。見つからなければ終了コード 3。Web の住所は `GET /api/projects/{name}/runs/{run_id:int}/history/calls/{call_id}`（`call_id` は `^[A-Za-z0-9_-]{1,80}$`。通らなければ 400）。

#### `run list [--limit N] --json` → `{"runs": [...]}`

既定 20 件（`--limit 0` で全部）。`runs[]` の項目は `run_id`・`block`・`description`・`status`・`lane`（`todo`・`running`・`review`・`workflow`）・`parent_run_id`（数値 \| null）・`created_at`（UTC）・`started_at`・`finished_at`（null あり）・`dir`（run フォルダのパス）。`round` 系の項目は無い。Web は呼ばない。

#### `run show <run_id> [--json]`

`--json` は 1 つの JSON を出す（文字の表示と同じ項目。英小文字の snake_case）。`run_id`・`block_name`・`block_file`（ブロックの項目。ブロックが無ければ null）・`status`（そのまま）・`started_at`・`finished_at`・`duration_ms`・`exit_code`・`pane_id`・`error_message`・`git_commit_before`・`git_commit_after`・`session_stats`（セッション統計の JSON の object。無ければ null）・`document_tokens`（出力の文書トークン）・`input_document_tokens`（入力の waggle トークン）・`output_content`（空・無しは null）。見つからなければ終了コード 3。`--json` なしは文字の表示。

#### `run md <run_id> --kind task|message|report|system [--write]` → `{"kind", "name", "text"}`

- 無ければ **終了コード 3・stderr「Not available yet.」**。`--write` は `task` だけ（未実行の todo カードだけ。無ければ 3）。本文は標準入力。
- `report` は run の報告（`finalize-report.md`）、`system` は固定部分（`system-prompt.md`）。

#### `run finalize` / `run advance` / `run rm`

- `run finalize <run_id>` → `{"run_id", "status": "requested", "message"}`（`message` は英語）。その run が無ければ 3「Run <id> not found.」、実行中でない（「Run <id> is not running (status: …).」）・子ブロック・常駐停止は 1。
- `run advance <run_id>` → `{"run_id", "from", "to"}`。`from` は `review`、`to` は日付フォルダの名前（`yymmdd`。日付は一番上の親 run の実行開始日・ローカル時刻）。review でない run は **終了コード 2**「Only a run in review can be moved.」。無ければ 3「Run not found.」。review にある子（孫も）は同じ日付フォルダへ移り、running の子は動かさない。
- `run rm <run_id> --yes` → `{"run_id", "executions": 消した数}`。実行中・待機中は **4**「A running run cannot be deleted. Stop it first.」（Web は 409）、無ければ 3「Run <id> not found.」。

#### `run send <run_id>` / `run interrupt <run_id>`

| 命令 | 返り値 | 終了コード |
|---|---|---|
| `run send <run_id> [--text <文>] [--json]` | `{"run_id", "pane_id", "status": "sent" \| "unconfirmed", "message"?}` | 空の文 2・実行や送り先が無い 3・終了している/質問待ち 4 |
| `run interrupt <run_id> [--json]` | `{"run_id", "status": "interrupted" \| "still_working", "agent_status"}` | 無い 3・終了している 4 |

`unconfirmed` は、文と確定キーを送ったが状態が `working` にならなかった（届いたか分からない）ことを表す。失敗にすると利用者が送り直して二重に届くので、終了コード 0 で返し、`message`（英語 1 文）を付ける。`task send`（作業中の子への直接送信）も同じで、`{"action": "send", "task_id", "status": "unconfirmed", "message"}` を返す。最初の文（run の起動時）と完了した子へのターン送信は、今までどおり失敗にする。

- `--text` を省くと標準入力の全文を送る（Chats の「続きを送る」が使う）。
- 送り先は headless の擬似 pane（`hl-` で始まる id）。`draft.txt`・`inbox/`・`interrupt` のファイルで監督スレッドへ渡す。
- `state.json` が `stopped`（終了している）、または `blocked`（質問待ち）のときは 4。

#### `doc list --run <id> --json` / `doc read <token> --json` / `doc search …`

| 命令 | 返り値 | Web が読む項目 |
|---|---|---|
| `doc list --run <id> --json` | `{"count", "items": [...]}` | `items[].token`・`label`・`description`・`status`・`ref`（task_text に貼れる `waggle:<token>:path:…`） |
| `doc read <token> --json` | `{"token", "label", "text"}` | `token`・`label`・`text` |
| `doc search --word … [--category task,message,waggle] [--since …] [--until …] [--block …] [--from-id N] [--to-id N] [--limit N]` | `{"items": [...], "truncated": 真偽値}` | `items[].run_id`・`category`・`file`・`token`（waggle のとき）・`block`・`date`・`snippet`。新しい run から `--limit` 件で打ち切り。message.md は run フォルダにあるものだけ |

#### `card options` / `card docs` / `card create` / `card run|schedule`

| 命令 | 返り値 | Web が読む項目 |
|---|---|---|
| `card options` | `{"blocks", "cc_models", "thinking", "warnings"}` | `blocks[]`（`id`・`label`）・`cc_models`（`opus`・`sonnet`・`haiku`）・`thinking`（`none`・`low`・`medium`・`high`・`xhigh`・`max` の並び）・`warnings` |
| `card docs [--limit N]` | `{"items": [{token, label, description, status, ref}]}` | 同左（task_text に貼る候補） |
| `card create`（**標準入力に JSON**・作成後は todo に配置） | `{"run_id", "path", "executer"}` | `run_id`（次の画面の行き先） |
| `card run <run_id>` | JSON | 未実行カードが無ければ **終了コード 3・stderr「No unstarted card found.」**。`executer: agent` のカードは **終了コード 2**・stderr「This card is started by an agent as a subtask run (executer: agent). To start it yourself, change the card's executer to human.」。常駐が止まっているときも 2「The daemon is stopped. Start it first.」 |
| `card schedule <run_id> [--at "YYYY-MM-DD HH:MM"]` | `{"run_id", "scheduled_at"}` | `--at` を省くと予定を消す。常駐が 5 秒ごとに見回って実行する |

`card start`・`card finish`（`runner: user` の札）と `card wait`（`after_job`）は無い。

`card create` の標準入力の JSON（Web が作る形）:

| 項目 | 型 | 内容 |
|---|---|---|
| `block` | 文字列 | **必須**。存在する block の ID（`^[A-Za-z0-9][A-Za-z0-9_:.-]*$`）。空・無い ID は**終了コード 2** |
| `model` / `thinking` | 文字列 | 空でスキップ。`model` は `opus / sonnet / haiku`（CLI は `claude-*` も受ける）、`thinking` は `none / low / medium / high / xhigh / max`。ほかは**終了コード 2**（Web は 400） |
| `description` | 文字列 | 一覧に出す 1 行（1 行目だけ・200 字まで） |
| `task_text` | 文字列（100KB まで） | 依頼文。`waggle:<トークン>` で文書を埋め込める |
| `executer` | 文字列 | `human`（既定）/ `agent`（旧 `professor` も `agent`）。知らない値は**終了コード 2**。Web の作成フォームは渡さない |
| `doc_tokens` | 配列 | 貼る文書のトークン（`task_text` の先頭に `waggle:<トークン>` の行として貼られる）。無いトークンは**終了コード 3** |
| `meta` | オブジェクト | frontmatter に足す任意の項目（キーは `^[a-z][a-z0-9_]*$`。`run_id`・`block` などの標準のキーと、廃止した `runner`・`provider`・`tag`・`round`・`global_round` は書けない＝終了コード 2） |

廃止した `runner`・`provider`・`tag`・`global_round`・`round`・`done_ids` が入っていても、エラーにせず無視する。

#### `workflow [--subtasks]` → オブジェクト

実行の記録（DB）が要る。実行が 0 件でも、空の配列で成功する。DB が開けないときは終了コード 4。配置（座標）は `agent-link` が計算する（Web で計算し直さない）。`--subtasks` を付けないと親の札だけ、付けると `task create` で作った子の札も載せる（Web は `?subtasks=1`）。

| 項目 | 型 | 内容 |
|---|---|---|
| `width` / `height` | 数値 | 盤面全体の大きさ（文字マス） |
| `nodes[]` | 配列 | 実行 1 件 = 札 1 枚 |
| `edges[]` | 配列 | 札と札の線 |
| `separators[]` | 配列 | 日付の区切り線 |
| `lanes` | オブジェクト | Kanban の 3 レーン |

`nodes[]` の項目:

| 項目 | 型 | 内容 |
|---|---|---|
| `id` / `run_id` | 数値 | 実行の ID（同じ値） |
| `name` | 文字列 | block の名前 |
| `status` | 文字列 | 最後の状態 |
| `created_at` / `description` | 文字列 | 作成日時・説明 |
| `cost` | 文字列 \| null | 費用の表示 |
| `orphaned` | 真偽値 | 親が見つからない子か |
| `model` / `thinking` | 文字列 \| null | 札に出す（無ければ null） |
| `parent_run_id` / `batch_id` | 数値 \| null / 文字列 \| null | 子の親と、`task create` の batch |
| `x` / `y` / `w` / `h` | 数値 | 位置と大きさ（文字マス） |

`edges[]` の項目:

| 項目 | 型 | 内容 |
|---|---|---|
| `from` / `to` | 数値 | 札の `id`（実行の ID） |
| `kind` | `"delegation"` / `"waggle_token"` | 親 → 子の依頼 / waggle トークンの受け渡し |
| `side` | 真偽値 | 左右の辺につなぐか（偽なら上下の辺） |

`separators[]` の項目: `y`（数値・文字マス）と `label`（文字列・日付 `yymmdd`）。

`lanes` は `{"todo": [...], "running": [...], "review": [...]}`（空でも 3 つとも出す）。札の項目は `run_id`・`title`・`date`・`description`・`executer`。`nodes[]` に `phase`・`review`・`tag`・`runner`・`global_round`・`round` は無い（review の記入判定も無い）。

#### `def list|show|write|delete`

| 命令 | 返り値 | 注意 |
|---|---|---|
| `def list [--parent <名前>]` | `{"global_root", "project_root", "items": [...]}` | `items[]` は `kind`・`name`・`scope`・`title`・`description`・`exists`・`shadowed`・`used_by`・`templates`・`error` |
| `def show --kind block\|template\|settings --scope project\|global --name <名前>` | `{"kind", "scope", "name", "path", "exists", "text", "used_by", "warnings", "error"}` | `--scope` を省くと解決済みの定義 |
| `def write --kind … --scope … --name … [--create]`（本文は標準入力） | JSON | 壊れた定義は保存せず**終了コード 2**で理由を出す。`--create` で同じ名前があるときも今は 2「同じ名前の定義があります」（Web が 409 に読み替える） |
| `def delete --kind … --scope … --name …` | JSON | settings は消せない |

`scope=global` の置き場は `$AGENT_LINK_HOME`。定義の編集は DB が無くてもできる。

#### `session search … --json`

`--id <run_id または N-M>`・`--block`・`--date`（`YYYY/MM/DD` か `YYYY/MM/DD-YYYY/MM/DD`）・`--status`・`--type`・`-w <語>`（`-R`・`-r` は無い）。返り値は配列、または `{"items": [...]}`（Web は両方を受け、先頭 200 件に切って `truncated` を付ける）。

#### `init [--name <名前>]`

- `--name` を明示したときだけ名前を検査する: `^[A-Za-z0-9][A-Za-z0-9_.-]{0,63}$`（Web の新しいチャットの `NC_NAME_RE` と同じ）。通らなければ**終了コード 2**・「Invalid project name: <名前>. Use 1-64 letters, digits, "_", "." or "-", starting with a letter or digit.」で、何も作らない。`--name` を省いたとき（フォルダ名が既定名）は検査しない。
- 登録簿への登録に失敗したとき（名前の重複など）は**終了コード 1**。`.agent-link/` などの初期化は済んだ状態になる（重複は登録簿を読まないと分からないため）。
- 書く物は `.agent-link/`（`cc-settings.json` を含む）・`.mcp.json`・`.claude/commands/`・`.claude/skills/`・`.serena/project.yml`。プロジェクトの `.claude/settings.json` は書かない。

#### `daemon start|stop`

**文字出力**（JSON ではない）。Web は終了コードだけで成否を見る。

#### `task create`（標準入力に子の配列の JSON）→ `{"batch_id", "round", "supersedes_batch_id", "status", "tasks"}`（この `round` は子の batch の版で、廃止した run の round とは別）

検査に通らない子があれば、標準出力は空・**終了コード 2**。標準エラーに子ごとの理由を 1 行ずつ、最後の行に「Input validation of task_create failed. …」を出す。run の外（`AGENT_LINK_PARENT_RUN_ID` が無い）で呼ぶと終了コード 2。

子ごとの項目（既存の項目に足すもの）:

| 項目 | 型 | 内容 |
|---|---|---|
| `model` | 文字列（省略可） | `opus` / `sonnet` / `haiku` / `inherit` / `claude…`。それ以外は断る |
| `thinking` | 文字列（省略可） | `none` / `low` / `medium` / `high` / `xhigh` / `max`。それ以外は断る |
| `runner` | （無視） | 廃止した項目。書いてあっても値が何でもエラーにせず無視する。出力の JSON にも無い |

- 優先順位は「要素 > Card の task.md > block の frontmatter > settings.json」。省いた項目は、次の順で決まる。
- `tasks[]` の子ごとの形は `{"client_id", "status": "approved", "task_id": int | null}`。`task_id` が null のときは `task list` の `server_id` で確かめる（`task_id` が無いことは、子が作られていないことを意味しない）。

#### `run wait <run_id>… [--timeout-ms N] [--json]`

自分の子でない run（`card run` で起こした run）を待つ。子は `task wait` で待つ。

- 1 回の待ちは最大 90 秒。`--timeout-ms 0` なら 1 回だけ調べて返る。`run_id` は 1〜50 件。
- `status` は次のどれか。質問は見ない。

| `status` | 追加の項目 | 意味 |
|---|---|---|
| `completed` | `run_id`・`block`・`pending` | 完了した |
| `failed` | `run_id`・`block`・`error`・`pending` | 失敗した。`error` に理由が入る |
| `stalled` | `run_id`・`block`・`reason`（`idle`）・`message`・`pending` | pane が idle のまま止まった。`message` は Web の質問を見るように書く |
| `in_progress` | `pending`（全件）・`message` | 90 秒以内に結果の出る run が無かった |

- 返した run は、次の呼び出しで `pending` だけを渡す。実行エンジンは返した run を覚えない。
- 存在しない run_id は終了コード 3（標準エラーの最後の行に「Run <id> not found.」）。

`failed` の `error` の文言:

- 401: `Claude authentication failed (401). Check the Claude login or token: …`
- pane が消えた: `The agent exited before finalize (the pane is gone).`

---

## 3. Web とエージェントの間の約束

エージェントは CLI を Bash で呼ぶ。Web が読む・書く物のうち、エージェント側の約束を書く。

### 3.1 `.agent-link/settings.json` の項目（agent に渡す環境変数ほか）

settings は 2 か所で書ける。**後者（`$AGENT_LINK_HOME`）が前者（プロジェクト）をキー単位で上書きする。**

1. `<project>/.agent-link/settings.json`
2. `$AGENT_LINK_HOME/settings.json`（**こちらが優先**）

| 項目 | 型 | 何に使う |
|---|---|---|
| `default_model` / `default_thinking_level` | 文字列 \| null | block の frontmatter に無いときの既定 |
| `wip` / `max_concurrent` | 数値 \| null | タブごとの pane 上限（既定 8）・同時実行数（既定 16） |
| `pre_run` | 文字列の配列 \| null | 起動直前の準備（シェルは通さない） |
| `write_check` | 文字列の配列 \| null | `agent-link doc write` が保存の前に本文を標準入力で通す命令。`{run_id}`・`{block}`・`{filename}` を置き換える（シェルは通さない）。環境変数 `AGENT_LINK_WRITE_FILENAME`・`AGENT_LINK_WRITE_DESCRIPTION`・`AGENT_LINK_PARENT_RUN_ID`・`AGENT_LINK_BLOCK_NAME`。終了コード 0: 標準出力が空でなければそれを保存、空なら元の本文を保存。1: 標準エラーを返して保存しない。それ以外（起動失敗・30 秒の時間切れ・2 以上）: 元の本文を保存し警告を返す |
| `env` | オブジェクト（KEY → 値） | **agent の起動コマンドの先頭に付ける環境変数** |

読み込みは `crates/agent-link-engine/src/settings.rs` の `load_settings()`。

#### `env` の決まり

- キーは `^[A-Za-z_][A-Za-z0-9_]*$`。値は 1 行（`\n` / `\r` を含む値は破棄）・長さ 1〜2000。
- **予約キー** `AGENT_LINK_PROJECT_ROOT` / `AGENT_LINK_PARENT_RUN_ID` / `AGENT_LINK_BLOCK_NAME` / `AGENT_LINK_IS_SUBTASK` は書いても破棄される（`agent-link` 自身が必ず正しい値を入れるため。`settings.rs` の `RESERVED_ENV_KEYS`）。
- 通らない項目は**エラーにせず、警告を 1 行出して破棄する**。
- 並びは `AGENT_LINK_*` → `env` → エージェントの実行ファイル。値は shell のシングルクォート 1 つで囲むので、`export` や連結の文字列を書く必要はない。
- 実装は `settings::agent_env_pairs()` と `orchestrator::prepare_pipeline` の 1 か所。`prepare_pipeline` 以外に agent を起動する経路を増やさないこと。
- `env` には**平文の鍵が入りうる**ので `.gitignore` にする（利用者が決める。`agent-link` は `.gitignore` を書き換えない）。秘密の値は `env` で配らない（起動コマンドの文字になって pane と記録に残るため）。

### 3.2 カードの frontmatter

`task.md`（カード）の frontmatter のキー。

| 種類 | キー |
|---|---|
| 固定のキー | `creation_date`・`execution_date`・`run_id`・`block`・`thinking`・`model`・`description`（と、block の入力の `inputs`） |
| メタのキー（**入力の欄ではなく、起動の道を決める情報**・入力検査の対象外） | `source_doc`・`scheduled_at`・`executer` |
| 廃止したキー（読み飛ばし、書き出さない） | `runner`・`provider`・`tag`・`round`・`global_round`・`after_job`・`target_node`・`origin_run_id`・`origin_block` |

| メタのキー | 値 | 意味 |
|---|---|---|
| `executer` | `human`（既定）/ `agent`（旧 `professor` も `agent` と読む） | 起動者。`agent` は、エージェントが自分の子の run として起こすカード。**人が起こす道（`card run`・常駐の予約実行）では動かさない**（起こすと root の run になり、報告が起こす側に届かないため） |
| `scheduled_at` | `YYYY-MM-DD HH:MM` | 実行予定時刻。`card schedule` が書く。常駐が 5 秒ごとに見回る |
| `source_doc` | 文字列 | カードの元になった文書 |

- `executer` は `agent_link_engine::kanban::CardFrontmatter::executer()` / `is_agent_executer()` が読む。未記載・空・知らない値は `human` 扱い。`run tree` の各 run ノードと `workflow` の `lanes` の各札が `"executer"` を持つ。
- 固定のキーとメタのキーは `kanban.rs` の `FIXED_FM_KEYS`・`META_FM_KEYS`、廃止したキーは `LEGACY_FM_KEYS`。

### 3.3 起動時の文の分け方

ブロックの起動時に送る文は **2 つ**に分かれる。作り方は `orchestrator::prepare_pipeline`（`crates/agent-link-engine/src/orchestrator.rs`）、分け目の処理は `placeholder::split_fixed_and_variable`（`crates/agent-link-engine/src/placeholder.rs`）。

| | 中身 | 置き場 | 渡し方 |
|---|---|---|---|
| 固定部分（system prompt） | template と block.md の本文 | run フォルダ直下の `system-prompt.md` | cc は `--append-system-prompt-file <絶対パス>` |
| 最初の文 | 自動で作られる `# context`（`context: true` のとき）と `# task_text` の節だけ | メモリ | headless の transport で 1 通送る |

- 節の順は `# context` → `# task_text` → 本文に `{{input.<id>}}` として現れた順。節の間は空行 1 行。固定部分の**先頭**に案内文（`依頼は、最初のメッセージの「# task_text」の節にあります。` など）を足す。
- 古い記法（`{{input.<id>}}`・`{{context}}`）は互換でまだ読めるが、書く必要は無い。
- `system-prompt.md` は `agent-link run md <run_id> --kind system` で読める（`--write` は `task` だけ）。
- cc の `--system-prompt-snapshot` は既定 on。`system-prompt.md` を直して再開しても cc は古い固定部分を使うことがある（固定部分を直して動かしたいときは `off` を付けるか、セッションを作り直す）。

### 3.4 報告の置き場

報告は **run フォルダの `finalize-report.md`** に 1 つだけ置く。

| 誰が書く | どう書く |
|---|---|
| 子の run | `agent-link task report`（本文は標準入力。`--run-id` を省くと自分の run） |
| root の run | `agent-link run finalize --report <文>` / `/agent-link-finalize <文>`（cc のフック） |

- `--report` を付けない `run finalize` は**保存済みの報告に一切触らない**。
- Web の run の画面は `run md --kind report` で読む（帯は「報告」）。親の `agent-link task wait` は完了の `report` にこの本文を返す。
- 文書の保存は `agent-link doc write --filename <名前> [--description <文>]`（本文は標準入力。`write_check` を通る）。

### 3.5 質問の通知

```bash
# 子の run の中から呼ぶ（親の run から呼ぶとエラー）
"$AGENT_LINK_BIN" task question --qid <質問ID> --text <見出し> [--run-id <N>]
```

- `AGENT_LINK_PARENT_RUN_ID` の実行を引き、親が無ければエラー「task-question is for subtask runs only.」。親の `agent-link task wait` は `{"task_id", "status": "stalled", "reason": "question", "question_id": "q1", "message": "Question q1: …"}` を返す。`question_id` は質問の通知にだけ付く。`text` は 1 行に畳んで 500 文字で切る。
- 質問の全文は Web の DB（`$AGENT_LINK_HOME/web/agent_link_web.db`）にある。コアは中身を持たない。答えるのは `agent-link-web answer`。

### 3.6 質問と送信

- 質問が開いている run に `agent-link-web send` すると、tmux には送らず質問を取り下げる（質問の状態は `withdrawn`・送った文は `answer.text`）。フックは、その文を次の指示として deny の理由文に入れる。
- Web に合言葉は無い。`agent-link-web` と Web のフックは `AGENT_LINK_WEB_URL`（既定 `http://127.0.0.1:8400`）を、何も付けずに HTTP で呼ぶ。内部用の住所は `require_internal` で Mac 自身（127.0.0.1 / ::1）だけ受ける。

### 3.7 フック・拡張と Web の約束

cc の質問フック（`apps/web/hooks/cc_question.py`・`cc_register.py`）は、Web の窓口を次で呼ぶ。

- **Web の場所**: 環境変数 `AGENT_LINK_WEB_URL` → `http://127.0.0.1:${AGENT_LINK_WEB_PORT:-8400}`。
- **使う住所**（Mac 自身（127.0.0.1）からの要求として受ける）:

| 住所 | 何に使う |
|---|---|
| `POST /api/register` | セッションと pane の対応づけ（`cc_register.py`） |
| `POST /api/questions` | 質問の登録。応答は `{"ok": true, "id": "<qid>"}` |
| `GET /api/questions/{qid}` | 答えのポーリング（1 秒ごと）。JSON の `answered_by`（答えた人。端末で答えたときは `terminal`） |
| `POST /api/questions/{qid}/resolved` | PC で答えたことの通知（PC の画面が呼ぶ） |

- cc は、tmux の対話モードで動く。tmux の session（名前は `hl-…`）の環境変数は `HERDR_PANE_ID=<pane_id>`・`AGENT_LINK_AGENT_TRANSPORT=tmux`・`AGENT_LINK_PROJECT_ROOT=<root>`・`AGENT_LINK_WEB_URL`（フックの送り先）と、`AGENT_LINK_HOME` が設定されているときはその値（`tmux.rs` の `-e`）。
- **cc の設定**は `.agent-link/cc-settings.json`（`init` が書き、run を始めるたびに今の版に合わせる）。cc は `--settings <そのファイル>` で受ける（許可・フック・statusLine）。プロジェクトの `.claude/settings.json` は読むだけで書かない（sai-moon の物と取り合わないため）。フックは `AGENT_LINK_PARENT_RUN_ID` のある cc（agent-link の run の中）でだけ動き、無ければ何もせず終了コード 0 で素通りする。許可する外部 MCP は `.mcp.json` の名前ごとの `mcp__<名前>__*`（`mcp__*` ではない）。
- cc の質問フックは `PreToolUse`（`AskUserQuestion`）で呼ばれ、答えを待つ（最長 24 時間）。答えが届いたら `permissionDecision: "allow"` と `updatedInput`（`questions` は cc の物をそのまま、`answers` は `{質問文: 答え}`）を返し、cc はこれを AskUserQuestion の結果としてモデルへ渡す。複数選択は label を `", "` でつなぐ。`AGENT_LINK_CC_QUESTION_MODE=deny` のときは、答えがあっても `permissionDecision: "deny"` と `permissionDecisionReason`（「利用者が Web で答えました。」と答え）で返す（`updatedInput` が cc で効かなかったときに戻すため）。答えが来ないまま打ち切ったときは `deny` と「利用者は答えませんでした。」。cc の hooks の `timeout` は 86400 秒。
- 承認は `agent-link cc-permission-hook`（`PermissionRequest`）が常に `allow` を返す。状態は `agent-link cc-state-hook <working|idle|blocked|tool>` と `agent-link cc-prompt-hook`（`UserPromptSubmit`）が `state.json` に書く。`PreToolUse`（全ツール）は `cc-state-hook tool`（入力の `tool_name` が `AskUserQuestion` なら `blocked`、ほかは `working`）、`PostToolUse` は `working`、`Stop`・`SessionStart` は `idle`。質問が答え無しで終わったとき（取り下げ・打ち切り）は `cc_question.py` が `working` に戻す。`blocked` は質問の答え待ちで、engine は取り消さない。
- cc の費用は cc の statusLine（`agent-link cc-statusline`）が `<AGENT_LINK_HOME>/cc-cost/<session_id>.json` に書く。
- cc の 5 時間枠・7 日枠は同じ `agent-link cc-statusline` が `<AGENT_LINK_HOME>/cc-rate-limits.json` に書く（`{"five_hour":{"used_percentage","resets_at"},"seven_day":{…},"session_id","updated_at"}`。欠けた窓は省く）。`GET /api/usage` は `{"cc": {"limits": <枠または null>, "cost": {"today","month","sessions"}}, "saved_at": <epoch 秒>}`（Home の Claude のカードが読む）。

---

## 4. 項目を変えるときの決まり

**Web が読む項目を変えたら、この文書と Web（`apps/web`）を一緒に直す。** 項目を足すだけなら Web は壊れない。

### 4.1 壊れる変更・壊れない変更

| 変更 | 結果 |
|---|---|
| JSON の**項目の追加** | **壊れない**（読まないだけ） |
| 項目の**削除・名前変更**（`daemon_alive` → `alive` など） | **壊れる**（ランプ・札・ボタンが消える／行が押せない） |
| **型の変更**（数値を文字列に、真偽値を 0/1 に） | **壊れる** |
| 配列をオブジェクトに包む（`[...]` → `{projects: [...]}`） | **壊れる**（502「応答を読めませんでした」） |
| 「無いとき `null`」を `0` や空文字にする | **壊れる**（`0` が「ある」とみなされる） |
| **命令名・サブコマンド名の変更**（`run tree` → `run list-tree`） | **壊れる**（Web は 400 / 502 を返す） |
| `--period` の値を減らす／外す | **壊れる**（400 または 502） |
| 標準出力への余計な 1 行（デバッグ表示） | **壊れる**（502） |
| 失敗時の終了コードと標準エラーの最後の行の形を守る範囲の文言変更 | **壊れない**（文言は画面にそのまま出るだけ） |
| 終了コードの意味を変える（3 → 1 など） | **壊れる**（404 が 502 になる。「Not available yet.」「No unstarted card found.」も 3 を前提にしている） |
| 書き出し `run export` の `schema_version` を 2 にする | **壊れる**（全ファイルが無視され Chats が空になる） |
| `run_id` / `project_name` の型変更・`status` の語の変更・フォルダの形の変更 | **壊れる** |
| `card create` の標準入力を読まない | **壊れる** |
| `AGENT_LINK_PROJECT_ROOT` を見ない形にする | **壊れる**（全プロジェクトが同じ DB を開く） |
| バイナリ名の変更 | **壊れる**（503） |
| 置き場の規則（`AGENT_LINK_HOME`）を 1 か所だけ変える | **壊れる**（2 か所そろえる。[ARCHITECTURE.md](../ARCHITECTURE.md) の §3） |

### 4.2 直したあとに流す確認（読み取りだけ）

一時フォルダにプロジェクトを作り、`AGENT_LINK_HOME` と `AGENT_LINK_PROJECT_ROOT` をその中へ向けて行う。本物の `.agent-link/`・`$AGENT_LINK_HOME` を確かめのために変えない。

```bash
cd ~/workdir/Sai/agent-link
perl -e 'alarm 1800; exec @ARGV' cargo build --release      # 必ず release。Web は release を呼ぶ

T=$(mktemp -d); export AGENT_LINK_HOME=$T/home AGENT_LINK_PROJECT_ROOT=$T/proj; mkdir -p $AGENT_LINK_HOME $AGENT_LINK_PROJECT_ROOT
B=$PWD/target/release
(cd $AGENT_LINK_PROJECT_ROOT && git init -q && $B/agent-link init --name check >/dev/null)

# 1) schema が出るか
$B/agent-link schema --json | python3 -c "import sys,json;d=json.load(sys.stdin);print(d['bin'],len(d['commands']))"

# 2) 形（読み取りだけ）
cd $AGENT_LINK_PROJECT_ROOT
$B/agent-link project dashboard --period week | python3 -c "
import sys, json
d = json.load(sys.stdin)
for k in ('name','period','since','until','daemon_alive','daily','by_model','by_block','recent','summary'): assert k in d, k
print('dashboard OK')"
$B/agent-link run tree | head -c 200; echo
$B/agent-link card options | head -c 200; echo

# 3) 失敗の形（終了コード 3・標準出力が空・標準エラーの最後の行が英語）
$B/agent-link run md 999 --kind task >/dev/null; echo "exit=$?"

rm -rf $T; unset AGENT_LINK_HOME AGENT_LINK_PROJECT_ROOT
```

**流さないこと**: `daemon start`（常駐が起動する）・`card run`（エージェントが動く）。

### 4.3 変更したらすること

1. `cargo build --release`（Web は release の実行ファイルを叩く）。**PATH 上の名前 `agent-link` は変えない。**
2. 既存の書き出しを作り直す: `agent-link run export`（プロジェクトの root で）。常駐が動いていると自動で書き出される。
3. Web 側を直す場所: 項目の変更 → `apps/web/src/agent_link_web/` の `control.py` と、画面の `static/*.js`。値の検査の追加 → 同じモジュール。
4. 動作確認は 8297 番など別の番号で。**動いている 8400 番の Web は触らない。**
5. この文書の表と `docs/integration.md` の該当表を更新する。

---

## 5. 連携が壊れたときの見分け方

### 5.1 画面に出る文言 → 原因

| 画面の文言（`detail` の中身） | 状態 | 原因と直し方 |
|---|---|---|
| `agent-link not found. Check the configured path.` | 503 | CLI の場所。`AGENT_LINK_BIN` と PATH を確認。**バイナリ名を変えても同じ文が出る** |
| `Could not read the response from agent-link.` | 502 | 終了コード 0 なのに JSON でない（**標準出力への余計な表示**を疑う） |
| CLI の標準エラーの最後の行（例 `Not available yet.`） | 404 / 400 / 409 / 502 | 終了コードが 3 / 2 / 4 / 1。文言は CLI のまま |
| `period must be one of week, month, all.` `action must be start or stop.` `run_id must be an integer of 1 or more.` `block is invalid.` など | 400 | **Web 自身の検査**。CLI を呼ぶ前に弾いている。相手側を疑わない |
| `Not available yet.` | 404 | その run に md が無い（`run md` の終了コード 3） |
| `A running run cannot be deleted. Stop it first.` | 409 | 実行中・待機中の run を消そうとした（`run rm` の終了コード 4。以前は 502 だった） |
| `Choice <n> is out of range for question <N> (0 to <最大>).` `Question <N> takes one choice.` | 400 | 回答の番号が範囲外（**選択肢の番号は 0 から**）・単一選択に 2 つ以上。質問は `open` のまま |
| `Invalid request: <場所>: <理由>` | 422 | FastAPI の型の検査（本文の形が違う）。`detail` は**文字列**（既定の配列ではない） |
| `No unstarted card found.` | 404 | その run の未実行カードが無い（`card run` の終了コード 3） |
| `This card is started by an agent as a subtask run (executer: agent). …` | 400 | `executer: agent` のカードは人が起こす道では動かさない |
| `The daemon is stopped. Start it first.` | 400 | 常駐が止まっている（`card run`） |
| `Project not found.` / `This project has no run records yet.` | 404 | 登録簿に無い／`db_exists` が偽 |
| `Internal use only.` | 403 | 内部用の住所を Mac 自身（127.0.0.1 / ::1）以外から呼んだ |
| `agent-link did not respond in time.` | 504 | 時間切れ |
| 画面の `Cannot reach the Mac.` | 通信失敗 | 8400 自体に届かない（Web・tailscale・ネットワーク） |
| プロジェクト一覧が **空** | ― | `AGENT_LINK_HOME` が正しいか、`$AGENT_LINK_HOME/registry.json` にそのプロジェクトがあるか確かめる |

### 5.2 ログの見方

| どこを見る | 中身 |
|---|---|
| **Web**: `agent-link-web-restart`（README）で起こしたときは `$AGENT_LINK_HOME/web/agent-link-web.log`、直接起こしたときは起動したターミナル | `<CLI 名> の標準エラー: …`（先頭 300 文字）。**CLI の失敗原因はまずここ** |
| **`agent-link`** | 単発の CLI なので、Web のログに流れた標準エラーが手がかり。直接試すときは手元で `…; echo $?` |

### 5.3 直す順番

1. 画面の `detail` から **503 / 502 / 400 / 404 / 409 のどれか**を決める。
2. 503 → 場所（PATH・設定・常駐）。502 → **出力の混入**か終了コード 1。400 → Web の検査か CLI の終了コード 2。404 → データの有無。
3. §4.2 の確認を**読み取りだけで**流す。
4. 直したら §4.3 のビルド・作り直し・文書更新を行う。
