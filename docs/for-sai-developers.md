# CLI の約束（Web と各部品の間）

対象: **`crates/`（sai-link・sai-paper・sai-sci）か `apps/radio` のどれかを直す人**。
目的: 直した結果として **sai-web（Web の窓口・8200 番）を壊さないこと**。
全体の話は [`integration.md`](./integration.md)（API と画面の住所）と [`../ARCHITECTURE.md`](../ARCHITECTURE.md)、言葉の意味は [`../CONCEPT.md`](../CONCEPT.md)。
この文書の JSON の項目は、Web のコード（`apps/web/src/sai_web/` と `static/`）が実際に読む物と、2026-10 に `target/release/` の CLI を一時フォルダのプロジェクトで実行して確かめた物。**正はコード**。

---

## 0. 最初に読む節の早見表

| 直すもの | その節 | 最低限守ること |
|---|---|---|
| **3 つの CLI のどれか** | [§1](#1-3-つの-cli-の約束) | 終了コード・失敗の出し方・`--json`・`schema --json` |
| **sai-link**（`crates/sai-link-*`） | [§2.2](#22-sai-link-が出す-json) | 命令の名前と、Web が読む JSON の項目名。`run export` の書き出しの形 |
| **sai-paper**（`crates/sai-paper-*`） | [§2.3](#23-sai-paper-が出す-json) | 命令の名前と JSON の項目名。`target/release/sai-paper` に作り直す手順 |
| **sai-sci**（`crates/sai-sci-*`） | [§2.4](#24-sai-sci-が出す-json) | research-tree の JSON の項目名。`card --from-json` の入力 |
| **sai-sci が sai-link を呼ぶところ** | [§3](#3-sai-sci-が読む-sai-link-の出力) | `run list --json`・`card create` の入出力 |
| **前処理の束（sai-prep・Library の PDF 待ち）** | [§2.6](#26-前処理の束c12-と-library-の追加pdf-待ちc13) | 束の形（C12）と `import-bundle`・`keep`・`library pending` の JSON（C13） |
| **apps/radio** | [§5](#5-appsradio) | `/api/papers` 以下のパスと JSON、論文 ID は**英数字**、Range（206）応答 |
| **項目を変える** | [§6](#6-項目を変えるときの決まり) | Web が読む項目を変えたら、この文書と Web を一緒に直す |

---

## 1. 3 つの CLI の約束

`sai-link`・`sai-paper`・`sai-sci` は、次の約束を同じに守る。Web は終了コードと標準エラーの最後の行だけで失敗を扱う。

| # | 約束 |
|---|---|
| 1 | **終了コード**: 0 成功、2 入力の誤り（clap の使い方の誤りも 2）、3 見つからない、4 状態の衝突、1 その他の失敗 |
| 2 | **失敗したとき**: 標準出力には何も書かない。標準エラーの最後の行に、日本語で理由を 1 行書く。clap の誤りは最後の行に `引数の誤り: <clap の文の 1 行目>` |
| 3 | **成功したとき**: `--json` があれば標準出力に JSON を 1 つだけ書く（前後に余計な文字を出さない） |
| 4 | **時間のかかる命令**は、標準エラーに `sai-progress step=<数>`・`sai-progress msg=<文>` を出してよい（1 行 = 1 つ。段落内の改行は空白にする） |
| 5 | **プロジェクトの決め方**: `SAI_PROJECT_ROOT`（空でない値）→ cwd から上へたどり `.sai/` を持つ最初のフォルダ（ホームとそれより上は数えない）。見つからなければ 2。`init` は「`SAI_PROJECT_ROOT` → cwd」だけ。`schema` はプロジェクトを要らない |
| 6 | **記録の出どころ**: `SAI_ORIGIN`（無ければ `cli`）。Web が呼ぶときは `phone` |
| 7 | **`schema --json`** で命令の一覧を出す（下） |

標準出力に `println!` やデバッグ表示を 1 行でも足すと、Web が **502 になる**（`apps/web/src/sai_web/cli.py`）。標準エラーには何を出してもよい（先頭 300 文字が Web のログに残る）。

実測の例（一時フォルダのプロジェクト）:

```
$ sai-link run md 999 --kind task      → 終了コード 3・標準エラーの最後の行「まだありません」
$ sai-paper paper show 999999          → 終了コード 3・「論文が見つかりません」
$ sai-sci node Q999 --json             → 終了コード 3・「Q999 はありません」
$ sai-link run list --bogus            → 終了コード 2・「引数の誤り: unexpected argument '--bogus' found」
$ （プロジェクトの外で）sai-link run list --json → 終了コード 2・「プロジェクトが見つかりません（sai-link init を実行してください）」
```

### `schema --json` の形

```json
{
  "bin": "sai-link", "version": "0.1.0",
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
| `commands[].about` | 文字列 | 1 行の説明（Web の「コマンド」画面に出る） |
| `commands[].stdin` | `"none"` / `"text"` / `"json"` / `"bytes"` | 標準入力の種類。`bytes` の命令は Web に出さない |
| `commands[].json` | 真偽値 | `--json` を受けるか |
| `commands[].web` | 真偽値 | Web の「コマンド」画面に出してよいか |
| `args[].name` | 文字列 | 引数の ID（clap の ID。オプションの綴りは `--` + 名前だが、`_` は `-` と書く物がある。例: `year_from` は `--year-from`） |
| `args[].kind` | `"option"` / `"flag"` / `"positional"` | 引数の種類 |
| `args[].type` | `"string"` / `"int"` / `"float"` / `"enum"` / `"path"` | 値の型（enum は `values` に選択肢） |
| `args[].required` / `multiple` / `default` / `help` / `web` | 真偽値・真偽値・値・文字列・真偽値 | 必須・複数・既定・説明・Web に出してよいか |

- `schema --json` は **3 つの CLI すべてが持つ**。命令を足すと自動で出る。Web に出してはいけない命令（ファイルの送信・常駐の内部命令など）は `web: false` のままにする。
- Web の「コマンド」画面は argv を `[bin, *path, <--json>, オプション, フラグ, --, 位置引数]` の順に組む。位置引数は `--` のあとに置く（`-` で始まる値を守るため）。

### Web が CLI を呼ぶ約束

- env は `{**os.environ, "SAI_PROJECT_ROOT": <root>, "SAI_ORIGIN": "phone"}`、cwd は root。シェルは使わない（`shell=True` 禁止・引数は配列）。
- 終了コードから HTTP の状態へ: 2 → 400、3 → 404、4 → 409、1 とそのほか → 502。0 で JSON でない → 502。起動できない → 503、時間切れ → 504。`detail` は標準エラーの空でない最後の行（`sai-progress` の行を除く）。
- 時間切れは既定 20 秒。個別に、検索・内部検索 120 秒、訳 90 秒、PDF の訳・音声化・Download の後処理は長め（`papers.py` の定数）、「コマンド」画面の実行は 120 秒。
- 実行ファイルの場所は `SAI_LINK_BIN`・`SAI_PAPER_BIN`・`SAI_SCI_BIN`。**バイナリ名（`sai-link`・`sai-paper`・`sai-sci`）と、Web が呼ぶ命令の名前を変えてはいけない**（変えると 503 / 400 になる）。
- `sai-link` は `SAI_PROJECT_ROOT` を最優先で DB 解決に使う。Web は呼び出しのたびに必ず root を渡す。

---

## 2. Web が呼ぶ命令と、Web が読む JSON の項目

### 2.1 呼ばれ方の一覧

cwd は **そのプロジェクトの root**（論文は `papers.resolve_project(name)` が引く一覧の `root`）。

| 部品 | Web が呼ぶ命令（呼ぶ側のモジュール） | 時間切れ |
|---|---|---|
| sai-link | `run export`（`control.export_runs`・60 秒に 1 回）・`project summary`・`project dashboard --period`・`run tree`・`run log\|stats\|history <id>`・`run md <id> --kind …`（保存は `--write`）・`run finalize\|advance <id>`・`run rm <id> --yes`・`doc list --run <id>`・`doc read <token>`・`doc search …`・`card options\|docs\|create\|run\|start\|finish\|schedule`・`workflow`・`def list\|show\|write\|delete`・`session search …`・`daemon start\|stop`（すべて `control.py`） | 10 秒（export）・20 秒（既定）・30 秒（daemon・workflow）・60 秒（検索） |
| sai-paper | `library list\|tree`・`paper show\|note\|translate\|translate-pdf\|citations\|citations-extract\|download\|after-download\|candidates\|import-pdf\|radio`・`content paper`・`search`・`search-history`・`find`・`find-history`・`providers`・`dashboard`・`import list\|add\|remove`（`papers.py`・`sci.py`） | 30 秒（既定）・訳 90・検索 120・Download 300・後処理 2000・音声化 2000 |
| sai-sci | `status`・`types`・`init --type`・`node`・`apply`・`card --from-json`・`read`・`find --node`（範囲だけ）・`project show\|memo`・`attention list\|resolve`（`sci.py`・`papers.py`） | 15〜30 秒（init は長め） |
| 3 つの CLI | `schema --json`（`commands.py`・60 秒だけ覚える。取得は 20 秒まで） | 20 秒 |
| apps/radio | HTTP（中継 `/api/radio/…` → `127.0.0.1:8201/api/…`） | 質問（`…/ask`）180 秒・他 30 秒 |

- どの呼び出しも**シェルを使わず**、引数は配列で渡される。
- 検索語や文のように `-` で始まりうる値は、最後に `--` を置いて渡される。
- **`target/release/` の実行ファイルを作り直す**（`cargo build --release`）。直しても作り直さないと Web は古いまま。

### 2.2 sai-link が出す JSON

#### `run export`（書き出し。Chats が読む）

`$SAI_HOME/web/runs/<project_name>/<run_id>.json`（一時ファイル → rename で書く）。1 ファイル分の項目（`runs.py` の `_read_run`）。**足し算は安全、減らすのは危険。**

| 項目 | 型 | 何に使う | 無い・型が違うとき |
|---|---|---|---|
| `schema_version` | 数値（**1 固定**） | 受け入れ判定 | `1` 以外は**そのファイルを丸ごと捨てる** |
| `run_id` | 数値 | 実行の ID・URL `#/run/<project>/<run_id>` | 無ければ捨てる |
| `project_name` | 文字列 | 一覧の並び・URL | 無ければ捨てる |
| `runner` | 文字列（`cc` …。`pi` は廃止。古い記録は `cc` と読む） | 記録の読み分け | `"user"` は**一覧に出さない** |
| `status` | 文字列 | 一覧に出す判定 | `running / completed / failed / cancelled` のみ表示。**それ以外（todo・pending）は出さない** |
| `started_at` / `updated_at` | 文字列（タイムゾーン無し） | どの日の実行か（**UTC とみて日本時間に変換**）・並び順 | 無ければ日付なしで一覧から落ちる |
| `finished_at` / `duration_ms` | 文字列 / 数値 | 実行ごとの表示 | 空欄でも動く |
| `description` / `block_name` | 文字列 | 一覧の行・検索 | 空文字扱い |
| `parent_run_id` | 数値 \| null | サブタスクを親の直下に字下げ | 通常の位置に出る |
| `model` | 文字列 \| null | 統計画面 | ― |
| `stats` | オブジェクト \| null | 日ごとの費用（`stats.cost_usd.total`・`cost_is_estimate`） | 実行中は `null`（想定内） |
| `agent_session_id` / `transcript_path` | 文字列 \| null | 質問カード・会話の中身 | 会話が読めない |
| `herdr_pane_id` | 文字列 \| null | headless の擬似 pane の id（`hl-` で始まる・C1）。キー名は herdr の時代の名残で、中身は擬似 pane の id | 一覧の `live` と `headless` が付かない |

`herdr_pane_label` は Web では読まない（headless では pane を名前で探さないため・U03）。

#### headless の `state.json`（Web が読む・共有契約 C1）

headless の実行の状態は engine が `<project_root>/.sai/sai-engine/headless/<id>/state.json` に書く。
Web が読んでよいのはこのファイルだけ（`runs.py` の `headless_state`）。`<id>` は `hl-` + 12 桁の 16 進で，実行の書き出しの `herdr_pane_id` に入る。

| 項目 | 型 | 何に使う | 無い・型が違うとき |
|---|---|---|---|
| `state` | 文字列（`starting`（起動中） / `working` / `idle` / `stopped`） | 会話の `live` ランプ・通知 | `starting` / `working` / `idle` / `stopped` 以外は読まない |
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

Web が `recent[]` に**追加で足す**項目: `description`（文字列）・`linkable`（真偽値・メッセージ画面が開けるか）・`project`（文字列）。sai-link の側では不要。

#### `run tree` → `{"items": [...]}`（左サイドバーと同じツリー）

| 項目 | 型 | 使うところ |
|---|---|---|
| `items[].type` | `"folder"` / `"run"` | 行の種類 |
| `items[].id` / `label` / `depth` | 文字列 / 文字列 / 数値 | フォルダ（`todo`・`running`・`review`・`done`・`R<n>` …）の行 |
| `items[].children[]` | 配列 | フォルダの中身。run は `type: "run"` のノード |
| run ノード: `run_id` / `label` / `lane` / `status` | 数値 / 文字列 / 文字列 / 文字列 | 行・色・Workflow との対応 |
| run ノード: `executer` | `"human"` / `"agent"` | カードの起動者（既定 `human`。`agent` の todo には画面が ▶ 実行を出さない） |
| todo の run ノード: `card` | `{creation_date, block, runner, model, description}` | Dashboard とホームの todo の 2 行表示 |
| todo の run ノード: `scheduled_at` | `YYYY-MM-DD HH:MM`（予約があるときだけ） | ホームの「予約した todo」 |
| todo の run ノード: `after_job` | 文字列（`card wait` したときだけ） | ジョブ待ちの印 |

ホーム（`home.py`）は top レベルの `review`・`todo` フォルダの中の run ノードを読む。

#### `run log|stats|history <run_id>`

オブジェクトをそのまま画面に出す。`run log` の `connections[]`（作られた順。`direction` `from`/`to`・`kind` `task_text`/`task_create`/`task_output`・`run_id`・`global_round`・`round`・`block_name`・`description`・`waggle_token`・`file_path`）を Summary の Connection が読む。`block_name` / `exec_id` / `created_at` / `finished_at` / `input_fields[]` / `model` なども見出しに使う。

#### `run history <run_id> [--json]` と `run history <run_id> --call <CALL_ID> --json`（History・共有契約 C11）

`run history <run_id> --json` は次の項目を持つ（画面は `static/linkview.js` の `lvRenderHistory`）。

- `title` / `subtitle`（文字列）・`type_tokens`（`user` / `agent` / `toolCall` / `toolResult` / `thinking` の近似トークン数）。
- `events[]`（時刻順）: `time`・`type`（上の 5 種）・`name`・`tokens` は必ず持つ。`call_id`・`command`・`preview`（結果の本文の先頭 200 字）・`is_error`・`ids`（呼び出しの入力に出た `#x.y.z`）・`result_tokens` は、その行にあるときだけ出す（無いときは JSON に出さない）。toolCall と、それに対応する toolResult は 1 行にまとめる。対になる toolCall が無い結果だけ `toolResult` の行になる。
- `cli`（C11）: `calls[]`（`command`・`count`・`errors`・`help`。多い順）・`failures[]`（`class`・`command`・`count`）・`ids`（`seen` は台帳の異なる番号・`used` は台帳にあって呼び出しの入力に出た番号・`unused` は台帳にあって使わなかった番号。`used + unused == seen`）。
  - `calls[].count` と `errors` は `--help`・`-h` の呼び出しを除く。その回数は `help` に数える（`count` には入らない・失敗にも数えない）。
  - 命令名は `command` の語の位置（先頭・`&&`・`|`・改行・`(`・`$(`・`` ` `` の後。`timeout`・`env`・`exec`・`xargs`・`perl -e` の後も含む）の `sai-*` から読む。heredoc の本文と `#` の注釈は読まない。
  - `failures.class` は `ledger`（結果に `台帳に無い番号です` を含む）・`input`（終了コード 2）・`not_found`（3）・`conflict`（4）・`other`（読めない・それ以外）。`failures.command` は命令名。命令が読めない（`(不明)`）のは、入力に `sai-link`・`sai-sci`・`sai-paper`・`sai-web` の語がある Bash の呼び出しの失敗（`grep sai-paper` のような無関係な呼び出しも入る）。`Edit` など他の道具の失敗は数えない。
  - `cli` が無いのは古い CLI の JSON。
- 項目の意味の変更（U04）: `ids.used` は、以前は台帳に無い番号も含めていたが、台帳にある番号だけを数えるように変えた。`cli.calls[].help` と `cli.failures[].command` は足した項目。
- cc の History は、その run の transcript（`phone_export::resolve_transcript`）を読む。全体で一番新しい cc セッションは使わない。

`--call <CALL_ID> --json` は、その呼び出しの全文 `{"call_id","name","command","input","result","is_error","truncated"}` を返す。`result` は 20000 字までで、超えたら切って `truncated: true`。`--json` を付けないと本文だけの文字を出すので、Web は必ず付ける。見つからなければ終了コード 3。Web の住所は `GET /api/projects/{name}/runs/{run_id:int}/history/calls/{call_id}`（`call_id` は `^[A-Za-z0-9_-]{1,80}$`。通らなければ 400）。

#### `run show <run_id> [--json]`（U04）

`--json` は 1 つの JSON を出す（文字の表示と同じ項目。英小文字の snake_case）。`run_id`・`block_name`・`block_file`・`block_tag`・`block_round`（ブロックの項目。ブロックが無ければ null）・`status`（そのまま。文字の表示は日本語）・`started_at`・`finished_at`・`duration_ms`・`exit_code`・`pane_id`・`error_message`・`git_commit_before`・`git_commit_after`・`session_stats`（セッション統計の JSON の object。無ければ null）・`document_tokens`（出力の文書トークン。文字の表示の「文書トークン」）・`input_document_tokens`（入力の waggle トークン）・`output_content`（空・無しは null）。見つからなければ終了コード 3。`--json` なしは文字の表示（見出しは「文書トークン:」）。

#### `run md <run_id> --kind task|message|report|system [--write]` → `{"kind", "name", "text"}`

- 無ければ **終了コード 3・stderr「まだありません」**。`--write` は `task` だけ（未実行の todo カードだけ。無ければ 3「未実行のカードが見つかりません」）。本文は標準入力。
- `report` は run の報告（`finalize-report.md`）、`system` は固定部分（`system-prompt.md`）。

#### `run finalize` / `run advance` / `run rm`

- `run finalize <run_id>` → `{"run_id", "status": "requested", "message"}`。その run が無ければ 3、実行中でない・子ブロック・常駐停止は 1。
- `run advance <run_id>` → `{"run_id", "from", "to"}`。review / done 以外は **終了コード 2**「review / done の run だけ移動できます」。無ければ 3。
- `run rm <run_id> --yes` → `{"run_id", "executions": 消した数}`。実行中は 1、無ければ 3。

#### `run send <run_id>` / `run interrupt <run_id>`

| 命令 | 返り値 | 終了コード |
|---|---|---|
| `run send <run_id> [--text <文>] [--json]` | `{"run_id", "pane_id", "status": "sent"}` | 空の文 2・実行や送り先が無い 3・終了している/質問待ち 4 |
| `run interrupt <run_id> [--json]` | `{"run_id", "status": "interrupted" \| "still_working", "agent_status"}` | 無い 3・終了している 4 |

- `--text` を省くと標準入力の全文を送る（Chats の「続きを送る」が使う）。
- 送り先は headless の擬似 pane（`hl-` で始まる id）。`draft.txt`・`inbox/`・`interrupt` のファイルで監督スレッドへ渡す。
- `state.json` が `stopped`（終了している）、または `blocked`（質問待ち）のときは 4。

#### `doc list --run <id> --json` / `doc read <token> --json` / `doc search …`

| 命令 | 返り値 | Web が読む項目 |
|---|---|---|
| `doc list --run <id> --json` | `{"count", "items": [...]}` | `items[].token`・`label`・`description`・`status`・`ref`（task_text に貼れる `waggle:<token>:path:…`） |
| `doc read <token> --json` | `{"token", "label", "text"}` | `token`・`label`・`text` |
| `doc search --word … [--category task,message,waggle] [--since …] [--until …] [--block …] [--from-id N] [--to-id N] [--limit N]` | `{"items": [...], "truncated": 真偽値}` | `items[].run_id`・`category`・`file`・`token`（waggle のとき）・`block`・`date`・`snippet`。新しい run から `--limit` 件で打ち切り。message.md は run フォルダにあるものだけ |

#### `card options` / `card docs` / `card create` / `card run|start|finish|schedule|wait`

| 命令 | 返り値 | Web が読む項目 |
|---|---|---|
| `card options` | `{"blocks", "cc_models", "providers", "runners", "thinking", "warnings"}` | `blocks[]`（`id`・`label`）・`runners`（`user`・`cc`）・`thinking`・`providers`（キーが provider、値がモデルの配列）・`cc_models` |
| `card docs [--limit N]` | `{"items": [{token, label, description, status, ref}]}` | 同左（task_text に貼る候補） |
| `card create`（**標準入力に JSON**・作成後は todo に配置） | `{"run_id", "path", "executer"}` | `run_id`（次の画面の行き先） |
| `card run <run_id>` | JSON | 未実行カードが無ければ **終了コード 3・stderr「未実行のカードが見つかりません」**。`executer: agent` のカードは **終了コード 1**・stderr「このカードはエージェントが子の run として起こします（executer: agent）。人が起こすときは、カードの executer を human に直してください」 |
| `card start <run_id>` / `card finish <run_id>` | `{"run_id", "status": "running" \| "review"}` | `runner: user` の READ_CARD を running にする / review へ移す。未実行カードが無ければ 3、`runner: user` で無ければ 2「runner: user のカードではありません」（`finish` は実行中の利用者の札で無ければ 2「実行中の利用者の札ではありません」） |
| `card schedule <run_id> [--at "YYYY-MM-DD HH:MM"]` | `{"run_id", "scheduled_at"}` | `--at` を省くと予定を消す。常駐が 5 秒ごとに見回って実行する |
| `card wait <run_id> [--job <job_id>]` | `{"run_id", "after_job"}` | `--job` を省くと解除。形は `^[A-Za-z0-9][A-Za-z0-9_-]{0,63}$`。詳しくは [§4.2](#42-カードの-frontmatter) |

`card create` の標準入力の JSON（Web が作る形）:

| 項目 | 型 | 内容 |
|---|---|---|
| `runner` | 文字列 | `user` / `cc` / `gl`（空でスキップ。`pi` は廃止で `cc` と読む） |
| `block` | 文字列 | block の ID（`^[A-Za-z0-9][A-Za-z0-9_:.-]*$`） |
| `provider` / `model` / `thinking` | 文字列 | 空でスキップ。`thinking` は `none / low / medium / high / xhigh / max` |
| `global_round` / `round` / `tag` / `description` | 文字列 | ラウンドは数字、`tag` は 1 行 |
| `task_text` | 文字列（100KB まで） | 依頼文。`waggle:<トークン>` で文書を埋め込める |
| `executer` | 文字列 | `human`（既定）/ `agent`（旧 `professor` も `agent`）。知らない値は**終了コード 2** |
| `doc_tokens` / `done_ids` | 配列 | 貼る文書のトークン・完了済みの run（Web の作成フォームが渡す） |
| `meta` | オブジェクト | frontmatter に足す任意の項目（sai-sci が使う。§3） |

#### `workflow` → オブジェクト

`nodes`・`edges`・`lanes`（`todo`・`running`・`review`・`done`）・`separators`・`width`・`height`。配置は sai-link が計算する（Web で計算し直さない）。各札に `executer` が付く。実行の記録（DB）が要るので、無いと 404。

#### `def list|show|write|delete`

| 命令 | 返り値 | 注意 |
|---|---|---|
| `def list [--parent <名前>]` | `{"global_root", "project_root", "items": [...]}` | `items[]` は `kind`・`name`・`scope`・`title`・`description`・`exists`・`shadowed`・`used_by`・`templates`・`error` |
| `def show --kind block\|template\|settings --scope project\|global --name <名前>` | `{"kind", "scope", "name", "path", "exists", "text", "used_by", "warnings", "error"}` | `--scope` を省くと解決済みの定義 |
| `def write --kind … --scope … --name … [--create]`（本文は標準入力） | JSON | 壊れた定義は保存せず**終了コード 2**で理由を出す。`--create` で同じ名前があるときも今は 2「同じ名前の定義があります」（Web が 409 に読み替える） |
| `def delete --kind … --scope … --name …` | JSON | settings は消せない |

`scope=global` の置き場は `$SAI_HOME`。定義の編集は DB が無くてもできる。

#### `session search … --json`

`-R <round>`（global_round）・`-r <round>`・`--id <run_id>`・`--block`・`--date`・`--status`・`--type`・`-w <語>`。返り値は配列、または `{"items": [...]}`（Web は両方を受け、先頭 200 件に切って `truncated` を付ける）。

#### `daemon start|stop`

**文字出力**（JSON ではない）。Web は終了コードだけで成否を見る。

#### `task create`（標準入力に子の配列の JSON）→ `{"batch_id", "round", "supersedes_batch_id", "status", "tasks"}`（C-TASK・C-TASK-OUT）

子ごとの項目（既存の項目に足すもの）:

| 項目 | 型 | 内容 |
|---|---|---|
| `model` | 文字列（省略可） | `opus` / `sonnet` / `haiku` / `inherit` / `claude…`。それ以外は断る |
| `thinking` | 文字列（省略可） | `none` / `low` / `medium` / `high` / `xhigh` / `max`。それ以外は断る |
| `runner` | 文字列（省略可） | 省略か `cc`。`pi` は廃止のため断る（終了コード 2）。`cc` 以外も断る（終了コード 2） |

- 優先順位は「要素 > Card の task.md > block の frontmatter > settings.json」。省いた項目は、次の順で決まる。
- `tasks[]` の子ごとの形は `{"client_id", "status": "approved", "task_id": int | null}`。`task_id` が null のときは `sai-link task list` の `server_id` で確かめる（`task_id` が無いことは、子が作られていないことを意味しない）。

#### `run wait <run_id>… [--timeout-ms N] [--json]`（C-RUNWAIT・C-FAILED）

自分の子でない run（`card run` で起こした run）を待つ。子は `task wait` で待つ。

- 1 回の待ちは最大 90 秒。`--timeout-ms 0` なら 1 回だけ調べて返る。`run_id` は 1〜50 件。
- `status` は次のどれか。質問は見ない。

| `status` | 追加の項目 | 意味 |
|---|---|---|
| `completed` | `run_id`・`block`・`pending` | 完了した |
| `failed` | `run_id`・`block`・`error`・`pending` | 失敗した。`error` に理由が入る |
| `stalled` | `run_id`・`block`・`reason`（`idle`）・`message`・`pending` | pane が idle のまま止まった。`message` は sai-web の質問を見るように書く |
| `in_progress` | `pending`（全件）・`message` | 90 秒以内に結果の出る run が無かった |

- 返した run は、次の呼び出しで `pending` だけを渡す。engine は返した run を覚えない。
- 存在しない run_id は終了コード 3（標準エラーの最後の行に「実行 <id> が見つかりません」）。

`failed` の `error` の文言（C-FAILED）:

- 401: `Claude の認証に失敗しました（401）。Account 画面で Claude のログイン（または token）を確かめてください: <行>`
- pane が消えた: `エージェントが finalize の前に終了しました（pane が消えました）`

### 2.3 sai-paper が出す JSON

cwd と `SAI_PROJECT_ROOT` は「論文の置き場（`.sai/paper/`）があるプロジェクト」の root。共通 API キーは `$SAI_HOME/paper.env`（プロジェクトの `.env` より優先順位が低い）。

#### `library list` → `{"papers": [...]}`

| 項目 | 型 | 使うところ | 無いとき |
|---|---|---|---|
| `id` | 数値 | 行を押したときの `#/paper/<name>/<id>` | その行が開けない |
| `title` | 文字列 | 1 行目 | 空行 |
| `authors` | 文字列（`"A, B"`） | 2 行目（最初の 1 人＋「ほか」） | 著者なし表示 |
| `year` / `journal` | 文字列 \| null | 2 行目 | 部分を省略 |
| `reading_status` | 文字列 | 札（`unread/queued/reading/read`） | 札なし |
| `podcast` | 文字列 \| null | 札（`ready` →「🎧 音声あり」／`generating` →「音声作成中」） | 札なし |
| `created_at` / `has_pdf` / `has_extract` / `is_favorite` | ― | **画面では未使用** | ― |

#### `library tree` → `{"groups": [...], "find_groups": [...]}`

| 項目 | 型 | 使うところ | 無いとき |
|---|---|---|---|
| `groups[].kind` | `"query"` / `"import"` | 見出しの形（`query` は「検索結果 ›」も出る） | そのまとまりが出ない |
| `groups[].log_id` | 数値 \| null | 「検索結果 ›」の行き先 `#/papers/log/<name>/<log_id>` | リンクなし |
| `groups[].date` / `query` | 文字列 \| null | 見出しの文言 | ― |
| `groups[].hit_count` | 数値 | 「N 件」 | 0 件扱い |
| `groups[].paper_ids` | 数値の配列 | 中の論文を引く | そのまとまりが空 |
| `find_groups[]` | `log_id`・`date`・`query`・`hit_count`・`paper_ids`（Library にある論文だけ。ヒット順・重複なし）・`actor`（`{"kind"}` のみ） | 内部検索の履歴の組（新しい順・1 つの履歴につき 1 組） | ― |

#### `paper show <id>` → `{"paper", "files", "note", "refs", "podcast"}`

| 階層 | 項目 | 型 | 使うところ | 無いとき |
|---|---|---|---|---|
| `paper` | `id` `title` `authors` `year` `journal` `doi` `citation_count` `abstract` `abstract_ja` `reading_status` | 各種 | 題名・著者・年・掲載誌・被引用・DOI リンク・要旨の切替 | ― |
| `files` | `pdf` / `pdf_ja` / `extract` | 絶対パス \| null | 「PDF」「日本語 PDF」「本文」ボタン。**パスは画面に出さず、開けるかだけ見る**（Web が配信する `/pdf` はこのパスを読む。プロジェクトの root の外なら 404） | そのボタンが出ない |
| `figures` | `items[].image` | 絶対パス | 図の画像（`/figures/<n>`。root の外なら 404） | 図が出ない |
| 直下 | `note` | 文字列 \| null | ノートの本文 | 「ノートはありません」 |
| `refs[]` | `title` `authors` `year` `journal` `doi` `local_paper_id` | 各種 | 引用文献の行。`local_paper_id` があれば Library へ移動 | ― |
| `podcast` | `status` | `"ready"/"generating"/"failed"` \| null | 音声ボタンの出し分け（`generating` は 10 秒ごとに読み直す） | なし扱い |
| `podcast` | `error` | 文字列 \| null | 「前回は失敗しました: …」 | ― |
| ― | 論文が無いとき | | **終了コード 3・「論文が見つかりません」** | ― |

#### `search-history <id>` / `search` / `expand` → `{"log", "items"}`

| 階層 | 項目 | 型 | 使うところ | 無いとき |
|---|---|---|---|---|
| `log` | `id` | 数値 | 画面を移っても結果を取り直すための鍵 | 検索直後の復元ができない |
| `log` | `query` / `date` / `source_dbs` | 文字列 | 見出しの下の小さい行 | ― |
| `log` | `executer` | `"human"` \| `"agent"` | 検索を実行した人。`SAI_ORIGIN=phone` は `human`、それ以外で `SAI_PARENT_RUN_ID` が数なら `agent` | 出さない |
| `log` | `actor` | `{"kind":"human"}` \| `{"kind":"agent","run_id":数値\|null,"block":文字列\|null}` | 「実行者」（agent は `SAI_PARENT_RUN_ID` を書いた検索） | 出さない |
| `log` | `node` | `{"id":"A5"}` \| null | 見出しの下の Node へのリンク（`#/research/tree/A5`） | リンクを出さない |
| `items[]` | `position` | 数値 | 行を押したときの `#/papers/log/<name>/<log_id>/<position>` | 開けない |
| `items[]` | `title` `authors`（**配列**）`year` `journal` `doi` `abstract` `citation_count` | 各種 | 行と詳細の表示 | ― |
| `items[]` | `local_paper_id` | 数値 \| null | 「✓ Library」の札と行き先 | 未ダウンロード論文の情報画面 |
| `failed[]`（**`search` だけ**） | `db` / `reason` | 文字列 | 検索の失敗の帯。`reason` の 429 は「混雑」、403 は「キーが使えない」 | 帯を出さない |

- `search --json --limit N [--db <ID>]… [--year-from Y] [--year-to Y] -- <語>`。失敗した DB は標準エラーに `<db>: <理由>` の 1 行と、標準出力の `failed` へ出す。検索は履歴に残る。
- `find --json [--mode word|vector|hybrid] [--top-k N] [--scope papers|notes|figures|all] [--paper <番号>]… [--each m] -- <語>` は内部検索（要約なし）。新しい履歴を作り、`find-history <id>` と同じ形で返す（`log.executer` も付く）。`sai-sci find --node <ID> [--subtree]` は検索の範囲を絞るだけで、記録を Node に結ぶのは環境変数 `SAI_TARGET_NODE`。`sai-sci search` に `--node` は無い。
- `expand --json` は `search` と同じ `{"log","items"}`（`results` つき）を返す。
- `find-history <id>` の `log.actor`・`log.node`・`log.mode`・`log.scope`（指定が無ければ空文字）・`log.top_k`（無ければ `null`）・`hits[].authors`（配列）。存在しない id は終了コード 3「検索履歴が見つかりません」。

#### `providers` → `{"default": [...], "available": [...]}`

両方**文字列の配列**。`available` が来ないと検索を始められない（502「検索先を読めませんでした」）。`default` は初期選択。Web は `available` で `--db` を検査する（増減は `providers` と同時に変える）。

#### `content paper <id> --json` → `{"id", "md_path", "text"}`

画面は `id` と `text` のみ。

#### `paper translate <id>` → `{"id", "abstract_ja"}` / `paper translate-pdf <id> --json`

`translate` は画面が `abstract_ja` のみ読む。`translate-pdf` は数分かかる（Web は 202 を返し背景で待つ）。

#### `paper citations <id>` → `{"paper", "key", "has_pdf", "extracted", "refs"}`

抜き出し済みでも未済みでも同じ形（読み取りだけ）。

| 階層 | 項目 | 型 | 使うところ |
|---|---|---|---|
| `paper` | `id` / `title` | 数値 / 文字列 | 見出しと戻る先 |
| 直下 | `key` | 文字列 \| null | 引用の抜き出しが要るかを見る（`ArXiv:…` / `DOI:…` / 40hex） |
| 直下 | `has_pdf` / `extracted` | 真偽値 | 「先に PDF を取り込んでください」・引用の行と引用文の出し分け |
| `refs[]` | `ref_no` | 数値 | 行番号と `--ref` に渡す値 |
| `refs[]` | `title` / `first_author` / `year` / `journal` | 文字列 \| null / 文字列 \| null / 数値 \| null / 文字列 \| null | 行の表示 |
| `refs[]` | `doi` / `s2_paper_id` / `raw_text` / `url` | 文字列 \| null | 詳細表示・開くボタン |
| `refs[]` | `local_paper_id` | 数値 \| null | 「✓ Library」の行き先 |
| `refs[]` | `can_download` / `can_import` | 真偽値 | 「取りに行く」「手動取り込み」ボタン |
| `refs[].sentences[]` | `sent_no` / `text` | 数値 / 文字列 | その引用を引用している文（`text` は 300 字まで） |

#### `paper citations-extract <id>` → `{"paper_id", "refs", "sentences", "docling_error"}`

標準エラーに `sai-progress step=1` を 1 行出す（段は `0 = PDF確認`（Web）→ `1 = 引用抽出` → `2 = 完了`（Web））。`docling` が無い・失敗したときも終了コード 0 で、`docling_error` に理由が入る。arXiv ID か DOI が無い・PDF が無いときは終了コード 1。

#### `paper download` / `paper after-download <id>`

- `paper download (--log <id> --position <p> | --import-id <id> | --cite <paper_id> --ref <ref_no>)`（**数分かかる**）→ `status` が `exists`（既に Library にある）か `downloaded`（`paper_id` を持つ）。
- `paper after-download <paper_id>`（**数分かかる**）は標準エラーに 1 行 = 1 文で次を出す。Web が 1 行ずつ読む。別の形（`warning: …` など）は進捗ではなく Web のログに出る。標準出力の JSON は変えない。
  - `sai-progress step=<n>` — 段。`0 = PDF取得` / `1 = 図抽出` / `2 = 翻訳` / `3 = 完了`。0 と 3 は Web が決める（`after-download` は 1 と 2 を知らせる）。
  - `sai-progress msg=<文>` — ログの 1 行。

#### `paper candidates --minutes N` → `{"dir", "minutes", "items"}`

ダウンロードフォルダ（`XDG_DOWNLOAD_DIR` → `~/.config/user-dirs.dirs` → `~/Downloads` → `~/ダウンロード`）の最近の PDF。

| 階層 | 項目 | 型 | 使うところ |
|---|---|---|---|
| 直下 | `dir` | 文字列 \| null | 見出しのフォルダ名（ホームを `~` に置き換える） |
| 直下 | `minutes` | 数値 | 表示の期間（Web は 30 を渡す） |
| `items[]` | `name` | 文字列 | 選ぶときのラベル。**フルパスは返さない** |
| `items[]` | `size` / `modified_at` | 数値 / 文字列 | 大きさ・更新時刻 |

#### `paper import-pdf <対象> (--file <名前> [--minutes N] | --stdin)` → `{"paper_id", "title", "status", "pdf_path"}`

ブラウザで保存した PDF（または Web のアップロード）を Library に入れる。`paper_id`・`title`・`status`（`"imported"`）を Web が読む。`pdf_path` は**画面に出さない**。

- 対象は 1 つだけ: `--log <id> --position <p>` / `--import-id <id>` / `--cite <paper_id> --ref <ref_no>` / `--paper-id <id>`。
- 中身は 1 つだけ: `--file <名前>`（**ファイル名だけ**。`/`・`\`・`..` は失敗。ダウンロードフォルダの最近 `--minutes` 分の PDF に同じ名前が要る。**移動される**）か `--stdin`（**100MB まで**）。
- 図の抽出と PDF の翻訳は**しない**。Web が続けて `paper after-download <paper_id>` を呼ぶ。
- `--cite` の取り込み先が既に Library にあるときは、`--file` の元のファイルを動かさず終了コード 1（`この論文は既に Library にあります（ID <n>）`）。PDF を付け替えたいときは `--paper-id` を使う。

#### `paper radio <id>`（**数分・終わるまで戻らない**）

`apps/radio` の `POST /api/papers/from-extract` を呼んで音声化を依頼する。Web は背景で 2000 秒まで待ち、終わったら通知する。返り値の `radio_id` を使う。

#### `paper note <id>`（標準入力 = 本文）→ JSON

100KB まで。

#### `dashboard` → `{"cards", "activity"}`

数字カード（`cards`: `papers`・`notes`・`tags`・`db_bytes`・`pdf_bytes`）と、Radio の出来事を混ぜた `activity[]`。research-tree の CARD の実行は `kind: "action"` の行として入る（`todo` のものは除く）。`kind`・`label`・`occurred_at` は他の行と同じで、`action` の行は次を足して持つ:

| 項目 | 型 | 使途 |
|---|---|---|
| `run_id` | 数値 | CARD の run（`#/research/read/<run_id>` などの行き先） |
| `node` | 文字列 \| null | CARD が付いた Node の ID（`Q12` など） |
| `block` | 文字列 | CARD の block 名（`lab-search` など） |
| `owner` | 文字列 | `todo` 以外の状態（`running`・`review` など） |

#### `import list|add|remove`

- `import list` → `{"dir", "files", "entries", "errors"}`（.bib の一覧と中の論文）。
- `import add --name <名前>`（標準入力 = BibTeX・1MB まで）。名前は `^[A-Za-z0-9._-]{1,80}\.bib$`。
- `import remove --name <名前>`。


### 2.4 sai-sci が出す JSON

Web の `sci.py` が検査してから呼ぶ。Node は Q（問い）・A（答え）・F（根拠）・E（実験）・S（SPROUT）・D（seed。研究ごとの制約）の 6 種。ID は `^[QAFESD][1-9][0-9]{0,8}$`（例 `Q12`・`A5`・`F3`・`E2`・`S2`・`D1`）。根の `SEED` は廃止した（`node SEED` は終了コード 2 と案内の文）。軸の Q は親の無い Q（`sai-sci init` が作る。`add_q` の `parent` は Q か A だけ）。
`status` と `node` の JSON は `sai-sci-core` の `Status` / `NodeDetail` を `serde_json::to_value` したもので、CLI は項目を足したり名前を変えたりしない。書き込む命令（`init`・`apply`・`op`・`card`）は、終わる前に `state.md` と `tree.md` を書き出す。Web は書き込みのあと画面を読み直す。

#### `status --json` → research-tree 全体

| 階層 | 項目 | 型 | 使うところ | 無いとき |
|---|---|---|---|---|
| 直下 | `initialized` | 真偽値 | 始めていないときの案内 | 常に research-tree を表示してしまう |
| 直下 | `legacy` | 真偽値 | 古い形の research-tree だけがあるときの「作り直し」の案内 | 古い research-tree に気づけない |
| 直下 | `type` | `{"id","name"}` \| null | 見出しの型の名前 | 型名が空欄 |
| 直下 | `counts` | `{"q","a","f_support","f_deny","f_issue","e","s"}`（生きた Node の総数。F は stance ごと） | 「Q 3・A 2・F 4（S 1 / D 2 / I 1）・E 0・S 1」の行 | 0 に見える |
| 直下 | `nodes[]` | 配列（深さ優先） | research-tree そのもの。research-tree の図もここから描く（図だけを返す命令は無い） | research-tree が描かれない |
| 直下 | `project` | `{"type","mind":[{"heading","body"}]}` | 木の型と mind.md（RESEARCH_TYPE と mind.md）。D の seed とは別 | 型が出ない |
| 直下 | `next[]` | NextAction の配列 | 「Next Action」の行と CARD 作成の種 | 出ない |
| 直下 | `decisions[]` | `{"node","node_text","action","detail","actor","at","approved_by","qid","question"}` | 決定の記録（新しい順 30 件。画面は `history[]` を使う） | ― |
| 直下 | `history[]` | `{"node","node_text","action","at","by","run_id","block","global_round","round","approved_by","qid","question"}` | 全 Node の History（`Add *` を除き古い順）。Research 画面の「History」 | History が出ない |
| 直下 | `steps` | `{"total","list[]","nodes[]","removed[]"}`（下の「steps」） | 時系列。step を送って任意の時点の木を描き直す（Web の画面は U02） | 空（`total` 0。始めていないとき） |
| 直下 | `running[]` | `{"run_id","node","block","owner"}` | 動いている実行の行。木の全体を対象にした run は `node` が `null` | ― |
| 直下 | `warnings[]` | 文字列 | そのまま見せる注意 | 注意が隠される |
| ― | 始めていないとき | | `type`・`project` は `null`、配列は空、数は 0 | ― |

`status --brief`（いつも JSON）は `{"type","counts","next","running","warnings"}` だけを返す。`next[]` の各行は `kind`・`node`・`option`・`block`・`purpose`・`expect`・`why`。

**steps**（`status.steps`。始めているときだけ作る。作る所は `crates/sai-sci-core/src/rt/steps.rs`）

- 成功した op 1 回が 1 step。`n` は 1 から振る。`list[]`（1 番目が step 1）は `{"n","at","op","by","node"}` で、`at` は RFC3339 UTC。`op` は `add_q`・`add_a`・`add_f`・`add_e`・`add_sprout`・`add_seed`・`insert_q`・`copy`・`move_a`・`remove`。`node` は作られた・動いた・消えた Node（消去は根の Node）。`by` は作った人・動かした人・消した人。
- `insert_q` は Q の作成と A の移動が 1 step。`copy` は A の作成が `copy`、F は 1 Node ごとに `add_f`。
- `nodes[]`（消えた Node も。ID の数の昇順）は `{"id","born","gone","parents":[{"step","parent"}]}`。`gone` は消えた step（生きていれば null）。`parents` の先頭は `{step: born, parent: 生まれたときの親}`、その後は付け替えのあった step の昇順。
- step `k` に Node が見えるのは `born <= k` かつ（`gone` が null または `gone > k`）のとき。その step の親は、`parents` のうち `step <= k` の最後の `parent`。
- `removed[]` は消えた Node の `NodeJson`（`depth` は 0、`bottom_q` は false、`counts` は 0、`range` は null）。
- `insert_q`・`move_a` の `rt_log.detail` は、末尾に移した A の ID を `（A5, A6）` で残す（この変更から）。
- 限界（コードの `ponytail:` と同じ）:
  - 同じ秒の作成・`move_a`・消去は、作成 → `move_a` → 消去の順に決め打ち（時刻は秒単位）。
  - 消去は `removed_at` が同じ値の部分木を 1 step にまとめる。同じ秒に 2 回消すと 1 step になる。
  - この変更より前の `move_a`・`insert_q` は移した A の ID が無い。移動先にもとからある A と取り違えうる。ID の小さい順に件数だけ取る（推測）。
  - 本文・★・stance・MLflow の変更（`edit`・`star`・`set_stance`・`set_mlflow`）は step に数えず、再生もしない。
  - `add_a`・`add_f`・`add_e`・`add_seed`・`add_sprout` は `rt_log` に書かない。作成は `rt_nodes` の行から作る。

**NodeRow**（`status.nodes[]` と `node.node` / `children[]` / `subtree[]` の 1 件）

| 項目 | 型 | 使うところ | 無いとき |
|---|---|---|---|
| `id` / `kind` | 文字列（`Q12`）/ `"Q"` `"A"` `"F"` `"E"` `"S"` `"D"` | 行の種類の出し分け、行き先 `#/research/tree/<id>` | Node が選べない |
| `parent` / `depth` / `ord` | 文字列 \| null / 数値 / 数値 | 並びと字下げ（top_Q と D は `parent` が `null`、深さ 0。S は親が D の ID（`"D1"`）。親の無い S は古い形で `null`） | 並びが壊れる |
| `text` / `body` | 文字列 | 行のラベル / 本文（D の `text` は題（1〜120 字）、`body` は seed の本文（0〜2000 字）。本文は見出し付きの markdown。9 項目の見出しは `apps/web/src/sai_web/static/sciedit.js` の `seSeedBody` が作る） | ― |
| `guide` | 文字列 \| null（Q だけ） | Q のガイドの key | ガイドの説明が出ない |
| `stance` | 文字列 \| null（F だけ） | F の立場（`support`・`deny`・`issue`） | ― |
| `starred` | 真偽値 | ★ の目印（計算には使わない） | ★ が出ない |
| `range` | `{"q","a","f_support","f_deny","f_issue"}` \| null（S だけ） | SPROUT の範囲（Q・A の数と F の stance ごとの数） | 範囲が出ない |
| `paths[]` | 文字列の配列（S だけ） | SPROUT が選んだ PATH（A の ID） | ― |
| `bottom_q` | 真偽値 | bottom_Q の印 | ― |
| `counts` | `{"q","a","f","e"}` | 直の子の数 | 数が空 |
| `cites[]` / `paper` | 文字列の配列 / 数値 \| null | F の引用と論文 | ― |
| `run` / `mlflow` | 数値 \| null / 文字列 \| null | E の実行と実験の記録 | ― |
| `created_by` / `created_at` | 文字列 | 作った人（`user` / `run:<N>`）と時刻 | ― |

**NextAction**（`status.next[]` / `node.next[]` の 1 件）: `kind`・`node`・`option`・`block`・`purpose`・`expect`・`why`・`card`（そのまま `card --from-json` に渡せる C-CARD の JSON \| null）。

#### `node <ID> --json` → Node 1 つ

| 階層 | 項目 | 型 | 使うところ |
|---|---|---|---|
| 直下 | `node` | NodeRow | 板の見出しと本文 |
| 直下 | `path[]` | `{"id","kind","text"}`（SEED から順） | パンくず |
| 直下 | `children[]` / `subtree[]` | NodeRow の配列 | 直の子 / 子孫すべて |
| 直下 | `guide` | `{"key","name","explain","question","child_candidates","extract","pitfalls"}` \| null | Q の説明と目安 |
| 直下 | `decisions[]` | status の `decisions[]` と同じ | この Node の決定の記録 |
| 直下 | `actions[]` | `{"run_id","block","owner","node","global_round","purpose","occurred_at"}` | この Node のための実行の行（子孫の run も入る。Web の側で `node` で絞る） |
| 直下 | `creator` | `{"by","run_id","block","owner","global_round","node","purpose","at"}` | この Node を作った人とその run（`by` が `user` のときは `run_id` が null） |
| 直下 | `history[]` | `{"action","at","by","run_id","block","global_round","round","approved_by","qid","question"}` | この Node の履歴（先頭は必ず `init`。`at` は RFC3339 UTC） |
| 直下 | `next[]` | NextAction の配列 | この Node の Next Action |
| 直下 | `searches[]` | `{"id","run","block","tool","query","hits","log_id","created_at"}` | この Node に結んだ検索の履歴（`log_id` は Library の検索結果へのリンク先） |
| 直下 | `sources[]` | `{"cite","paper","title","section","passage"}` | `node.cites` ごとの出典（`passage` は先頭 200 字） |

#### `read <run_id> --json` → READ_CARD

`run_id` / `owner`（まだ `todo` なら「始める」を出す）・`node`（`{"id","kind","text"}`）・`purpose` / `depth`・`paper`（`{"id","title","year"}`）・`questions[]` / `extract[]`・`findings[]`（`{"id","parent","text","cites"}`）・`note`。行き先は `#/research/read/<run_id>`。

#### `card --from-json --json`（標準入力 = C-CARD）→ `{"run_id","path","executer"}`

Web が作る要求の JSON。Web が先に検査し、知らない項目は落とす。

| 項目 | 型 | 内容 |
|---|---|---|
| `node` | 文字列（`Q12` の形。Q・A・F・E・S・D。空は木の全体。`SEED` は終了コード 2） | どの Node のためのカードか。node の型の block には**必須**（木の全体は manage と support の block だけ） |
| `block` | 文字列（`^(lab\|sci)-[a-z-]{2,30}$`） | 動かす block（`lab-search`・`sci-verify` など）。**必須**（runner が `user` で空なら `lab-read`） |
| `purpose` | 文字列（1〜300 字） | 依頼文。**必須** |
| `expect` | 文字列（300 字まで） | 依頼文の「期待する成果」 |
| `mode` | `purpose_first` / `blind_first` | READ_CARD の型 |
| `papers` | 整数の配列（20 個まで。`user` はちょうど 1 本） | 対象論文 |
| `runner` | `cc` / `user`（既定 `cc`） | 実行主体 |
| `model` / `thinking` | 文字列 | 省くと settings.json の既定。`model` は `opus` / `sonnet` / `haiku` / `inherit`、`thinking` は `none` / `low` / `medium` / `high` / `xhigh` / `max`（C-SCICARD） |
| `depth` | `abstract` / `figures` / `full`（`user` のときだけ） | 読む深さ |
| `origin_run` | 数値 | 推薦した実行 |
| `child` | 文字列（`Q`・`A`・`F`。任意） | 足す子の型。対象が Q なら `Q` か `A`、A なら `Q` か `F`。それ以外の組は終了コード 2（CLI は `--child`） |

`input` は Web からは指定しない（CLI の `--input` だけ）。`executer` は Web からも CLI からも指定しない（C-SCICARD）。**ファイルのパスは受け取らない。**

#### 見た番号の台帳の照合（C10・`sai-sci op` の `add_f` と `report prepare`）

- `op` の `add_f` の `cites`（`apply` は照合しない）と、`report prepare` が拾う引用は、run の台帳（`<root>/.sai/seen_ids/<run_id>`）と照らす。台帳に無い番号を引いていれば、その op・報告は何も書かない（`op` は終了コード 2、`report prepare` は終了コード 1 で、文書を保存しない）。`op` は agent が動く run の中（`SAI_PARENT_RUN_ID` がある）でだけ照らす。
- 標準エラーに `台帳に無い番号です: #37.5.2（この run で CLI が出した番号だけを使えます）` の行が出る。`op` ではこれが最後の行（テストで確かめている）。この語は History の `ledger` の分類に使うので変えない。
- 台帳は MLflow・実験の台帳（`exp_summary.py`）とは別の物で、論文の番号 `#x.y.z` だけを持つ。
- run の ID（`SAI_PARENT_RUN_ID`）があるのに台帳のファイルが無いときは、その run は何も見ていないので、空の台帳として照合する（読める `cites` があれば上の文で終了コード 2。`cites` が無ければ通る）。run の ID が無いとき（手で打つ）は照合しない。台帳が読めないとき（権限など）だけ、標準エラーに `台帳を読めませんでした（検査を飛ばします）` を出して通す。

#### そのほかの命令

- `types --json` → `{"types":[{"id","name"}]}`。
- `init --type <型> --json` → オブジェクトで、`tree`（`{"type","created","warnings","backup","assets"}`）を持つ。Web は `tree` だけを返し、`tree` が無ければ 502。`created` は作った Q の数、`assets` は配る物の文字列の配列。新しい research-tree が既にあると終了コード 4「既に始めています（型: <名前>）」で失敗する。
- `apply --json`（標準入力 = op 1 件の JSON）→ `{"op","result"}`。`add_sprout` だけ `range` を持つ。利用者の op は、どの op も承認なしで行える。
  - `add_seed`（項目 `title` 必須・1〜120 字、`body` 任意・0〜2000 字）→ `result` は `D<n>`。許可は要らない。
  - `add_sprout` の項目 `seed` は必須で、生きた D の ID（親になる。1 つの D から複数の S を作れる）。`paths`（A の ID 1〜20 個）と `title` は従来どおり。
  - `add_q` の項目 `parent` は生きた Q か A の ID だけ。`SEED`・空は終了コード 2（「軸の Q は sai-sci init が作ります」）。
- `op [--run <id>] [--block <id>]`（標準入力 = `{"ops":[…]}`・1〜20 件。agent が使う。Web からは呼ばない）→ `{"results":[{"op","result"（`range` は `add_sprout` のとき）}…],"approval":null（承認を使ったときは `{"qid","answered_by"}`）}`。1 件でも弾かれたら何も書かず、標準エラーの最後の行に `<n> 件目（<op>）: <理由>`、終了コード 2。許可が要る op を含むのに許可が無いと終了コード 4。`--run` を省くと `SAI_PARENT_RUN_ID`、`--block` を省くと `SAI_BLOCK_NAME`。
- `approval record --qid <qid> --question <文> --answer <文> --answered-by <名前> [--run <id>] --json`（質問フックが呼ぶ。agent は呼ばない。Web からは呼ばない）→ `{"approval":{"id","run_id","qid","consumable"}}`。`consumable` は回答を空白で切った値が `許可する` のとき true。同じ `qid` が既にあれば終了コード 4。`--run` も `SAI_PARENT_RUN_ID` も無ければ終了コード 2。
- `find --json [--node <ID> [--subtree]] -- <語>` — `sai-paper find` と同じ形。`--node` は範囲だけ。`SAI_TARGET_NODE` があれば `rt_searches` にも 1 行残す。`search` は `SAI_TARGET_NODE` があれば記録を結ぶ（`--node` は無い）。
- `project show --json` → `{"mind","state","brief","attention"}`。`project mind --json`（本文は標準入力・1〜200000 字）→ `{"ok": true, "bytes": <n>}`。mind.md の全文を置き換える。
- `index paper <id> --json` → `{"paper_id","index","chunks_created","embedded_count","chunks","chunks_with_embeddings"}`。論文 1 本の chunk と埋め込みを作る（`chunks` は 0 のとき本文が無い）。無い論文は終了コード 3。
- `attention list --json` → 配列（要対応の `key`・文）。`attention resolve <key> --json` → 対応済みにする。

#### `status --brief [--limit N]`（C-NEXT）

- `--limit` の既定は 10。`0` は全部。
- `next` の並び: ★の A の NO_FINDING → Q の NO_CHILD → ★の無い A の NO_FINDING → UNVERIFIED。

#### `card` の `--model`・`--thinking`・`--runner`（C-SCICARD）

- `--runner` は `cc` / `user`（既定 `cc`）。`user` なら READ_CARD を作る。
- `--model` は `opus` / `sonnet` / `haiku` / `inherit`。`--thinking` は `none` / `low` / `medium` / `high` / `xhigh` / `max`。省くと settings.json の既定。
- `--executer` は無い。作る Card は人が起こす Card。

#### `apply` の actor（C-ACTOR）

- 記録の actor は、`SAI_PARENT_RUN_ID` があれば `run:<id>`（Professor の代行）、無ければ `user`。

#### `init --update`（C-ASSETS）

- blocks・template・tools・exec を源の版にそろえる。
- 違う物は `<名前>.bak` に退避する（同じ名前の `.bak` が既にあれば `.bak.<YYYYmmdd-HHMMSS>`）。
- block の frontmatter の `model`・`thinking` も源の値になる。

#### `exp_exec.py check <target>`（C-CHECK）

- local の target では、MLflow の `/health` と、成果物の置き場が `s3://` のときは s3 の鍵（`AWS_ACCESS_KEY_ID`）を確かめる。
- 確かめるだけで、ジョブは動かさない。

### 2.5 `apps/radio` を呼ぶ側

`sai-paper paper radio` は `POST /api/papers/from-extract` を、Web は `/api/radio/*` の中継を使う。項目は [§5](#5-appsradio)。

---

### 2.6 前処理の束（C12）と Library の追加・PDF 待ち（C13）

sai-prep が作った束（C12）は、Web が zip を展開して検査し、展開したフォルダを CLI に渡す（Rust の依存に zip は無い）。CLI の入り口は次の 3 つ。

| 命令 | 出力の JSON | 失敗 |
|---|---|---|
| `sai-paper paper import-bundle --dir <展開したフォルダの絶対パス> --json` | `{"paper_id","title","figures","sections","refs","translated"}` | 誤り 2・対象なし 3・PDF あり 4 |
| `sai-paper library export --ids <id,id,…> --out <無いフォルダの絶対パス> --json` | `{"dir","papers":[移植元の id],"missing":[移植元に無かった id]}` | 誤り 2（ids が空・`--out` が既にある）・1 つも無い 3（`--out` は作らない） |
| `sai-paper library import-migration --dir <束のフォルダの絶対パス> --json` | `{"imported":[{"source_id","paper_id","title","mode":"new" \| "filled"}],"skipped":[{"source_id","paper_id","title","reason"}],"failed":[{"source_id","title","error"}]}` | 誤り 2（`manifest.json`・`papr.db` が無い・形が違う）。論文ごとの失敗は `failed` に入れて終了コード 0 |
| `sai-paper paper keep (--log <id> --position <n> \| --import-id <id> \| --cite <paper_id> --ref <n>) --json` | `{"paper_id","title","status":"added" \| "exists"}` | ― |
| `sai-paper library pending --json` | `{"papers":[{"id","title","year","authors","doi","arxiv_id","s2_paper_id","created_at"}]}` | ― |

- 束の中身（zip の直下からの相対パス）: `manifest.json`・`paper.pdf`・`docling/paper.md`・`docling/paper.json`・`docling/paper_artifacts/…`・（翻訳したときだけ）`translated/paper-mono.pdf`・`translated/paper-dual.pdf`。
- `manifest.json` の形: `{"format": "sai-prep-bundle", "version": 1, "target": {"paper_id": 12} か {"bibtex": "<BibTeX 1 件>"}, "pdf": "paper.pdf", "pdf_sha256": "<64 桁の 16 進>", "docling": {"md", "json", "dir"}, "translation": {"mono", "dual", "service"} か null, "tools": {"sai_prep", "docling", "pdf2zh"}, "created_at"}`。
- 展開の検査（Web・`bundles.py`）: 項目 5000 以下・展開後 1GB 以下（束そのものは 300MB 以下）・絶対パス・`..`・`\`・NUL・シンボリックリンクは拒む。`pdf_sha256` が PDF と一致すること。
- `paper download`・`paper get-pdf` は終了コード 4（サーバは PDF を落とさない）。`import-bundle` は Web から 600 秒の上限で呼ばれる（`BUNDLE_TIMEOUT`）。

#### extract.md の画像と本文の読み方（U06）

- 取り込み（`import-bundle`）と Rust で docling を呼ぶ経路は、`extract.md` に base64 の画像（`data:image/…;base64,…`）も画像だけの行も残さない（`chunking::strip_images`）。文中の通常の画像参照（`![x](figures/…)`）は残す。
- 既にある論文は `sai-paper paper strip-images [<id>…] [--all] --json` で直す。`--all` と ID は一緒に使わない（両方・どちらも無いときは誤り 2）。変わった論文だけ書き直し、PDF より新しい更新時刻を保つ。元には戻せない（docling から作り直せば画像入りに戻る）。2 回目は `before` と `after` が同じになる。
  - 出力: `{"papers":[{"id","before","after"}],"total_before","total_after"}`。`extract.md` の無い論文は `papers` に入れない。論文の行が無ければ終了コード 3。
- `sai-paper content read <ID>… --json`（節・塊の本文）の項目: `{"refs","max_chars","truncated","next","passages":[{"id","section","heading","text","n_sentences"}],"empty","empty_refs"}`。
  - 節はあるが塊が 0 件の節（図や表だけの節）は、終了コード 0 で `empty_refs` に `#<論文>.<節>` を入れる（`empty` は読んだ塊が 1 つも無く、指定がすべて空の節のとき `true`）。テキスト出力は「#43.1 は本文が空です（図や表だけの節の可能性）」。
  - 論文の行が無いときは終了コード 3「論文が見つかりません: id=N」。論文はあるが `extract.md` が無いときは終了コード 1 のまま（作り方の案内）。

---

## 3. sai-sci が読む sai-link の出力

sai-sci は sai-link の DB と実行フォルダの並び（`.sai/outputs/`）を**直に開かない**。次の 2 つの命令だけを CLI で呼ぶ（呼び出しは `crates/sai-sci-core/src/link.rs`。実行ファイルは `SAI_LINK_BIN` → 自分の隣 → PATH）。

### `sai-link run list --json --limit 0` → `{"runs": [...]}`

`--limit 0` で全件。`runs[]` の項目（`run_id` が無い項目があると sai-sci は失敗する。値が無い項目は `null`・空）:

| 項目 | 型 | 内容 |
|---|---|---|
| `run_id` | 数値 | 実行 ID（**必須**） |
| `block` / `tag` / `description` | 文字列 | block 名・タグ・1 行の説明 |
| `status` | 文字列 | `todo`・`pending`・`running`・`completed` … |
| `lane` | 文字列 | 実行フォルダの親の名前（`todo`・`running`・`review`・`done`・`R<n>`）。次の `global_round` は `R<n>` の最大 + 1 |
| `parent_run_id` | 数値 \| null | 親の実行 ID |
| `global_round` / `round` | 文字または数 \| null | ラウンド（sai-sci は文字にして読む） |
| `created_at` / `started_at` / `finished_at` | 文字列 \| null | 日時 |
| `dir` | 文字列 \| null | 実行フォルダの絶対パス（空は `None` 扱い） |

### `sai-link card create`（標準入力に JSON）→ `{"run_id", "path", "executer"}`

sai-sci が渡す項目: `runner`（`cc` / `user`）・`block`・`global_round`・`round`・`tag`・`description`・`task_text`・`meta`（frontmatter に足す任意の文字列の組）。`model`・`thinking` は空でなければ渡す（C-SCICARD）。`executer` は渡さない。
返り値の `run_id` と `path`（task.md の絶対パス）を読む。無ければ失敗。成功したら sai-sci は覚えていた run の一覧を捨てる。

---

## 4. sai-web とエージェントの間の約束

エージェントは CLI を Bash で呼ぶ。Web が読む・書く物のうち、エージェント側の約束を書く。

### 4.1 `.sai/settings.json` の項目（agent に渡す環境変数ほか）

settings は 2 か所で書ける。**後者（`$SAI_HOME`）が前者（プロジェクト）をキー単位で上書きする。**

1. `<project>/.sai/settings.json`
2. `$SAI_HOME/settings.json`（**こちらが優先**）

| 項目 | 型 | 何に使う |
|---|---|---|
| `default_provider` / `default_model` / `default_thinking_level` | 文字列 \| null | block の frontmatter に無いときの既定 |
| `wip` / `max_concurrent` | 数値 \| null | タブごとの pane 上限・同時実行数 |
| `extensions` | 文字列の配列 | 起動時に読み込む拡張 |
| `pre_run` / `context` | 文字列の配列 \| null | 起動直前の準備・作業範囲の準備（シェルは通さない） |
| `write_check` | 文字列の配列 \| null | `sai-link doc write` が保存の前に本文を標準入力で通す命令。`{run_id}`・`{block}`・`{filename}` を置き換える（シェルは通さない）。環境変数 `SAI_WRITE_FILENAME`・`SAI_WRITE_DESCRIPTION`・`SAI_PARENT_RUN_ID`・`SAI_BLOCK_NAME`。終了コード 0: 標準出力が空でなければそれを保存、空なら元の本文を保存。1: 標準エラーを返して保存しない。それ以外（起動失敗・30 秒の時間切れ・2 以上）: 元の本文を保存し警告を返す。研究のプロジェクトでは `sai-sci init` が `sai-sci report prepare …` を書く |
| `job_status` | 文字列の配列 \| null | ジョブの状態を確かめる命令（先頭が実行ファイル。`{job}` をジョブの ID に置き換える）。常駐と `card wait` が読む。出力の `state` が `done` / `failed` / `cancelled` / `lost` なら終わり。無いと `card wait` は終了コード 2。`sai-sci init` が書く |
| `env` | オブジェクト（KEY → 値） | **agent の起動コマンドの先頭に付ける環境変数** |

読み込みは `crates/sai-link-engine/src/pi_command.rs` の `load_settings()`。

#### `env` の決まり

- キーは `^[A-Za-z_][A-Za-z0-9_]*$`。値は 1 行（`\n` / `\r` を含む値は破棄）・長さ 1〜2000。
- **予約キー** `SAI_PROJECT_ROOT` / `SAI_PARENT_RUN_ID` / `SAI_BLOCK_NAME` / `SAI_IS_SUBTASK` / `SAI_TARGET_NODE` は書いても破棄される（sai-link 自身が必ず正しい値を入れるため）。
- `SAI_TARGET_NODE`: Node を対象に動く Agent に sai-link が起動時に前置する。決め方は task.md の `sci_node`（空・無しと古い `SEED` は対象の Node なし）→ task.md の `target_node` → 親の run を同じ規則でたどる。親から受け継いだときは子の task.md に `target_node` を書く（`target_node.rs`）。
- 通らない項目は**エラーにせず、警告を 1 行出して破棄する**。
- 並びは `SAI_*` → `env` → エージェントの実行ファイル。値は shell のシングルクォート 1 つで囲むので、`export` や連結の文字列を書く必要はない。
- 実装は `pi_command::agent_env_pairs()` と `orchestrator::prepare_pipeline` の 1 か所。`prepare_pipeline` 以外に agent を起動する経路を増やさないこと。
- `env` には**平文の鍵・合言葉が入りうる**ので `.gitignore` にする（利用者が決める。sai は `.gitignore` を書き換えない）。合言葉は `env` で配らない（起動コマンドの文字になって pane と記録に残るため）。

```json
"env": {
  "MLFLOW_TRACKING_URI": "http://127.0.0.1:5001",
  "MLFLOW_S3_ENDPOINT_URL": "http://127.0.0.1:9000",
  "AWS_ACCESS_KEY_ID": "…",
  "AWS_SECRET_ACCESS_KEY": "…",
  "AWS_DEFAULT_REGION": "us-east-1"
}
```

### 4.2 カードの frontmatter

`task.md`（カード）の frontmatter のメタ情報。**入力の欄ではなく、起動の道を決める情報**（入力検査の対象外）。

| キー | 値 | 意味 |
|---|---|---|
| `executer` | `human`（既定）/ `agent`（旧 `professor` も `agent` と読む） | 起動者。`agent` は、エージェント（professor など）が自分の子の run として起こすカード。**人が起こす道（`card run`・常駐の予約実行）では動かさない**（起こすと root の run になり、報告が起こす側に届かないため） |
| `scheduled_at` | `YYYY-MM-DD HH:MM` | 実行予定時刻。`card schedule` が書く。常駐が 5 秒ごとに見回る |
| `after_job` | `<job_id>`（`^[A-Za-z0-9][A-Za-z0-9_-]{0,63}$`） | ジョブの終わりを待つ。`card wait --job` が書く |

- `executer` は `sai_link_engine::kanban::CardFrontmatter::executer()` / `is_agent_executer()` が読む。未記載・空・知らない値は `human` 扱い。`run tree` の各 run ノードと `workflow` の各札が `"executer"` を持つ。
- `sai-sci card` に `--executer` は無い（C-SCICARD）。作る Card は人が起こす Card で、`executer` は `human`。古い Card の `executer: agent` は読むだけで、`card run` は断る（終了コード 1）。
- `after_job`: 常駐は約 60 秒ごとに、カード 1 枚ごとの別の糸で `job_status` の命令を呼ぶ（90 秒で打ち切り）。`state` が `done` / `failed` / `cancelled` / `lost` なら `after_job` を消してそのカードを起こす。`queued` / `running` や、読めなかったときは、カードを変えず次の見回りでやり直す。環境変数 `SAI_EXP_STATUS_CMD` があれば状態を読む命令を差し替える。`agent` のカードは断られるので、起こされてもログに残るだけ。

### 4.3 起動時の文の分け方

ブロックの起動時に送る文は **2 つ**に分かれる。作り方は `orchestrator::prepare_pipeline`（`crates/sai-link-engine/src/orchestrator.rs`）、分け目の処理は `placeholder::split_fixed_and_variable`（`crates/sai-link-engine/src/placeholder.rs`）。

| | 中身 | 置き場 | 渡し方 |
|---|---|---|---|
| 固定部分（system prompt） | template と block.md の本文 | run フォルダ直下の `system-prompt.md` | cc は `--append-system-prompt-file <絶対パス>` |
| 最初の文 | 自動で作られる `# context`（`context: true` のとき）と `# task_text` の節だけ | メモリ | headless の transport で 1 通送る |

- 節の順は `# context` → `# task_text` → 本文に `{{input.<id>}}` として現れた順。節の間は空行 1 行。固定部分の**先頭**に案内文（`依頼は、最初のメッセージの「# task_text」の節にあります。` など）を足す。
- 古い記法（`{{input.<id>}}`・`{{context}}`）は互換でまだ読めるが、書く必要は無い。
- `system-prompt.md` は `sai-link run md <run_id> --kind system` で読める（`--write` は `task` だけ）。
- cc の `--system-prompt-snapshot` は既定 on。`system-prompt.md` を直して再開しても cc は古い固定部分を使うことがある（固定部分を直して動かしたいときは `off` を付けるか、セッションを作り直す）。

### 4.4 報告の置き場

報告は **run フォルダの `finalize-report.md`** に 1 つだけ置く。

| 誰が書く | どう書く |
|---|---|
| 子の run | `sai-link task report`（本文は標準入力。`--run-id` を省くと自分の run） |
| root の run | `sai-link run finalize --report <文>` / `/sai-finalize <文>`（cc のフック） |

- `--report` を付けない `run finalize` は**保存済みの報告に一切触らない**。
- Web の run の画面は `run md --kind report` で読む（帯は「報告」）。親の `sai-link task wait` は完了の `report` にこの本文を返す。
- 文書の保存は `sai-link doc write --filename <名前> [--description <文>]`（本文は標準入力。`write_check` を通る）。

### 4.5 質問の通知

```bash
# 子の run の中から呼ぶ（親の run から呼ぶとエラー）
"$SAI_LINK_BIN" task question --qid <質問ID> --text <見出し> [--run-id <N>]
```

- `SAI_PARENT_RUN_ID` の実行を引き、親が無ければエラー「task-question は子の run 専用です」。親の `sai-link task wait` は `{"task_id", "status": "stalled", "reason": "question", "question_id": "q1", "message": "質問 q1: …"}` を返す。`question_id` は質問の通知にだけ付く。`text` は 1 行に畳んで 500 文字で切る。
- 質問の全文は Web の DB（`$SAI_HOME/web/sai_web.db`）にある。コアは中身を持たない。答えるのは `sai-web answer`。

### 4.6 合言葉の置き場

Web は起動時に **`$SAI_HOME/web/token` に合言葉（`SAI_WEB_TOKEN`）を 0600 で書く**。合言葉が無いときはそのファイルを消す。

- 読むのは `sai-web` の命令とエージェントの道具（環境変数 `SAI_WEB_TOKEN` を先に、次に `SAI_WEB_INTERNAL_TOKEN`、無ければこのファイル）。
- コンテナの中では `SAI_WEB_TOKEN` は無く、`SAI_WEB_INTERNAL_TOKEN` だけがある。このとき `sai-web questions` は `GET /api/questions/mine` を呼び、**自分のプロジェクトの答え待ちだけ**を返す。`sai-web answer` は、自分のプロジェクトの質問にだけ答えられる（ほかは 403）。答えの記録は `agent`。
- コンテナの `sai-web` は `/usr/local/bin/sai-web` の sh で、`PYTHONPATH=/opt/sai-tree/apps/web/src` の `sai_web.webcli` を `python3` で動かす（fastapi は要らない）。
- 合言葉はヘッダ（`Authorization: Bearer`）で渡す。URL の `?token=` にはしない（Web は両方を受け付けるが、画面や記録に残るため）。
- HTTP のヘッダの値は latin-1 でしか送れないため、合言葉には英数字だけを使う。
- **合言葉の値そのものを、ログ・画面・エージェントの作業記録（session ファイル・run フォルダの成果物）に出さない。**

### 4.7 フック・拡張と Web の約束

cc の質問フック（`apps/web/hooks/cc_question.py`・`cc_register.py`）は、Web の窓口を次で呼ぶ。

- **Web の場所**: 環境変数 `SAI_WEB_URL` → `http://127.0.0.1:${SAI_WEB_PORT:-8200}`。
- **合言葉**: 環境変数 `SAI_WEB_INTERNAL_TOKEN` があれば、すべての要求に `Authorization: Bearer <値>` を付ける。無ければ付けない（単独モードは今のまま 127.0.0.1 からの要求として受ける）。
- **使う住所**:

| 住所 | 何に使う |
|---|---|
| `POST /api/register` | セッションと pane の対応づけ（`cc_register.py`） |
| `POST /api/questions` | 質問の登録。応答は `{"ok": true, "id": "<qid>"}` |
| `GET /api/questions/{qid}` | 答えのポーリング（1 秒ごと）。JSON の `answered_by`（答えた人。端末で答えたときは `terminal`） |
| `POST /api/questions/{qid}/resolved` | PC で答えたことの通知（PC の画面が呼ぶ） |

- cc は、コンテナの中の tmux の対話モードで動く。tmux の session（名前は `hl-…`）の環境変数は `HERDR_PANE_ID=<pane_id>`・`SAI_AGENT_TRANSPORT=tmux`・`SAI_PROJECT_ROOT=<root>`。
- cc の質問フックは `PreToolUse`（`AskUserQuestion`）で呼ばれ、答えを待つ（最長 24 時間）。答えが届いたら `permissionDecision: "allow"` と `updatedInput`（`questions` は cc の物をそのまま、`answers` は `{質問文: 答え}`）を返し、cc はこれを AskUserQuestion の結果としてモデルへ渡す。複数選択は label を `", "` でつなぐ。`SAI_CC_QUESTION_MODE=deny` のときは、答えがあっても `permissionDecision: "deny"` と `permissionDecisionReason`（「利用者が Web で答えました。」と答え）で返す（`updatedInput` が cc で効かなかったときに戻すため）。答えが来ないまま打ち切ったときは `deny` と「利用者は答えませんでした。」。cc の hooks の `timeout` は 86400 秒。
- 承認は `sai-link cc-permission-hook`（`PermissionRequest`）が常に `allow` を返す。状態は `sai-link cc-state-hook <working|idle>`（`Stop`・`SessionStart`）と `sai-link cc-prompt-hook`（`UserPromptSubmit`）が `state.json` に書く。
- cc の費用は cc の statusLine（`sai-link cc-statusline`）が `<SAI_HOME>/cc-cost/<session_id>.json` に書く。
- cc の 5 時間枠・7 日枠は同じ `sai-link cc-statusline` が `<SAI_HOME>/cc-rate-limits.json` に書く（`{"five_hour":{"used_percentage","resets_at"},"seven_day":{…},"session_id","updated_at"}`。欠けた窓は省く）。`/api/usage` の `cc` は `{"limits": <枠または null>, "cost": {"today","month","sessions"}}`。

---

## 5. `apps/radio`

Web は **HTTP の中継しかしない**（`apps/radio` のコードや DB は開かない。合言葉は `Authorization: Bearer` でだけ付ける）。

| 呼ばれる API | 実装場所 | 画面 |
|---|---|---|
| `GET /api/papers` | `apps/radio/src/sai_radio/main.py` `api_list_papers` | Radio 一覧 |
| `GET /api/papers/{id}` | `api_get_paper` | 再生画面（題名・文字起こし `sections`） |
| `GET /api/papers/{id}/audio`（**Range に対応**） | `api_get_audio`（`FileResponse`） | 再生・ダウンロード |
| `POST /api/papers/{id}/listen` | `api_listen` | 最初の再生で 1 回 |
| `POST /api/papers/{id}/favorite` | `api_toggle_favorite` | ★ |
| `GET /api/stats` | `api_stats` | カレンダー |
| `GET /api/logs?paper_id=` | `api_logs` | 質問の履歴 |
| `POST /api/papers/{id}/ask` | `api_ask` | 文字の質問（中継の時間切れ 180 秒） |
| `POST /api/papers/{id}/live-session` | `api_live_session` | 声の質問の合鍵（iPhone は Google へ直結） |
| `POST /api/papers/{id}/question-log` | `api_question_log` | 声の質問の記録 |
| `POST /api/papers/from-extract` | `api_from_extract` | **Web からは中継禁止（403）**。`sai-paper paper radio` が直接呼ぶ |
| `POST /api/papers/upload` | ― | **中継禁止（403）・削除済み**（`tests/test_main_routes.py` が確かめている） |

- 中継先は `SAI_RADIO_URL`（既定 `http://127.0.0.1:8201`）。合言葉 `SAI_RADIO_TOKEN` は Web 側にも同じ値を設定する。
- 中継で **パスの文字は `A-Za-z0-9_./-` のみ**（それ以外・`..` は 400）。
- **画面（`static/`）と PDF アップロードは削除済み。** 残っているのは API だけ。TTS プロキシ（`tts_proxy.py`）は 8202 番の別プロセスで、使わなければ起動しなくてよい。

### Web が読んでいる JSON の項目（`static/radio.js`）

| API | 項目 | 型 | 使うところ | 無い・違うとき |
|---|---|---|---|---|
| `GET /api/papers` | `id` | **英数字の文字列** | 行の URL `#/radio/<id>`・ダウンロード判定 | 行が開けない |
| 同 | `title` | 文字列 | 1 行目・絞り込み | 「(無題)」 |
| 同 | `created_at` | 数値（UNIX 秒） | 「9/22」表示と新着順 | ― |
| 同 | `favorite` | 真偽値 | ★ | ― |
| 同 | `duration` | 数値 \| null | 「長さ 23:10」 | 長さの表示なし |
| 同 | `mode` | ― | **未使用** | ― |
| `GET /api/papers/{id}` | `title` / `sections` | 文字列 / 配列 | 題名・文字起こし（`sections[]` は `start` `end` `title`） | 「無題の論文」・文字起こしなし |
| `POST …/favorite` | `favorite` | 真偽値 | 星の反映 | 反映されない（開き直すと直る） |
| `GET /api/stats` | `listened_papers` | 数値 | 「これまでに聴いた論文 N 本」 | 0 |
| 同 | `by_day` | 配列（`date` 文字列・`count` 数値） | カレンダー | 空 |
| `GET /api/logs?paper_id=` | `created_at`（UNIX 秒） / `kind` / `payload.position_sec` / `payload.question` / `payload.answer` | 各種 | 質問の吹き出し | 行が出ない（`kind=="question"` のみ表示） |
| `POST …/ask` | `answer` | 文字列 | 回答の吹き出し | 「考え中…」が残る |
| `POST …/live-session` | `token` / `ws_url` / `system_instruction` / `model` | 文字列 | Google への直結 | 声の質問が失敗 |
| 見つからないとき | `{"detail": "論文が見つかりません"}`（**FastAPI 形**・`{"error"}` ではない） | | 再生画面は「音声が見つかりません」 | ― |

---

## 6. 項目を変えるときの決まり

**Web が読む項目を変えたら、この文書と Web（`apps/web`）を一緒に直す。** 項目を足すだけなら Web は壊れない。

### 6.1 壊れる変更・壊れない変更

| 変更 | 結果 |
|---|---|
| JSON の**項目の追加** | **壊れない**（読まないだけ） |
| 項目の**削除・名前変更**（`daemon_alive` → `alive`、`reading_status` → `status` など） | **壊れる**（ランプ・札・ボタンが消える／行が押せない） |
| **型の変更**（数値を文字列に、真偽値を 0/1 に。`authors` の配列と文字列の入れ替え） | **壊れる** |
| 配列をオブジェクトに包む（`[...]` → `{projects: [...]}`） | **壊れる**（502「応答を読めませんでした」） |
| 「無いとき `null`」を `0` や空文字にする（`local_paper_id` を `0` に） | **壊れる**（`0` が「ある」とみなされる） |
| `podcast` をオブジェクトに包む | **壊れる**（Library と論文情報の両方を揃えて直す） |
| **命令名・サブコマンド名の変更**（`run tree` → `run list-tree`） | **壊れる**（Web は 400 / 502 を返す。「コマンド」画面は `schema` から自動で追従する） |
| `--period` の値を減らす／外す | **壊れる**（400 または 502） |
| 標準出力への余計な 1 行（デバッグ表示） | **壊れる**（502） |
| 失敗時の終了コードと標準エラーの最後の行の形を守る範囲の文言変更 | **壊れない**（文言は画面にそのまま出るだけ） |
| 終了コードの意味を変える（3 → 1 など） | **壊れる**（404 が 502 になる。「まだありません」「未実行のカードが見つかりません」も 3 を前提にしている） |
| 書き出し `run export` の `schema_version` を 2 にする | **壊れる**（全ファイルが無視され Chats が空になる） |
| `run_id` / `project_name` の型変更・`status` の語の変更・フォルダの形の変更 | **壊れる** |
| 研究方針の JSON の項目の名前・値の名前（`stance`・`range`・`counts`・`next` の `why` と `kind` など）を変える | **壊れる**（Research の画面が名前で読んでいる） |
| `card --from-json`・`apply`・`op` の標準入力を読まない | **壊れる** |
| `search` の `--db` の ID の増減 | `providers` の `available` と**同時に**変える |
| `SAI_PROJECT_ROOT` を見ない形にする | **壊れる**（全プロジェクトが同じ DB を開く） |
| バイナリ名の変更 | **壊れる**（503） |
| 置き場の規則（`SAI_HOME`）を 1 か所だけ変える | **壊れる**（3 か所そろえる。[ARCHITECTURE.md](../ARCHITECTURE.md) の §3） |

### 6.2 直したあとに流す確認（読み取りだけ）

一時フォルダにプロジェクトを作り、`SAI_HOME` と `SAI_PROJECT_ROOT` をその中へ向けて行う。本物の `.sai/`・`$SAI_HOME`・`papr.db` を確かめのために変えない。

```bash
cd ~/workdir/Sai/sai-tree
perl -e 'alarm 1800; exec @ARGV' cargo build --release      # 必ず release。Web は release を呼ぶ

T=$(mktemp -d); export SAI_HOME=$T/home SAI_PROJECT_ROOT=$T/proj; mkdir -p $SAI_HOME $SAI_PROJECT_ROOT
B=$PWD/target/release
(cd $SAI_PROJECT_ROOT && $B/sai-sci init --type problem --json >/dev/null)   # sai-link・sai-paper の初期化も含む

# 1) schema が 3 つとも出るか
for b in sai-link sai-paper sai-sci; do $B/$b schema --json | python3 -c "import sys,json;d=json.load(sys.stdin);print(d['bin'],len(d['commands']))"; done

# 2) 形（読み取りだけ）
cd $SAI_PROJECT_ROOT
$B/sai-link project dashboard --period week | python3 -c "
import sys, json
d = json.load(sys.stdin)
for k in ('name','period','since','until','daemon_alive','daily','by_model','by_block','recent','summary'): assert k in d, k
print('dashboard OK')"
$B/sai-link run tree | head -c 200; echo
$B/sai-link card options | head -c 200; echo
$B/sai-paper library list; $B/sai-paper providers
$B/sai-sci status --json | head -c 200; echo

# 3) 失敗の形（終了コード 3・標準出力が空・標準エラーの最後の行が日本語）
$B/sai-paper paper show 999999 >/dev/null; echo "exit=$?"

rm -rf $T; unset SAI_HOME SAI_PROJECT_ROOT
```

**流さないこと**: `daemon start`（常駐が起動する）・`card run`・`paper radio`（数分の生成）・`search`（検索履歴が増える）・`paper translate`（DB に訳を書く）・DOK や MLflow の起動。

### 6.3 変更したらすること

1. `cargo build --release`（Web は release の実行ファイルを叩く）。**PATH 上の名前 `sai-link`・`sai-paper`・`sai-sci` は変えない。**
2. 既存の書き出しを作り直す: `sai-link run export`（プロジェクトの root で）。常駐が動いていると自動で書き出される。
3. Web 側を直す場所: 項目の変更 → `apps/web/src/sai_web/` の `control.py`（sai-link）・`papers.py`（sai-paper）・`sci.py`（sai-sci）と、画面の `static/*.js`。値の検査の追加 → 同じモジュール。
4. 動作確認は 8200 番で（`apps/radio` は 8201 を再起動せず、別の番号で）。**旧構成の 8100 番・8000 番は触らない。**
5. この文書の表と `docs/integration.md` の該当表を更新する。

---

## 7. 連携が壊れたときの見分け方

### 7.1 画面に出る文言 → 原因

| 画面の文言（`detail` の中身） | 状態 | 原因と直し方 |
|---|---|---|
| `…が見つかりません（設定の場所を確認してください）` | 503 | CLI の場所。`SAI_LINK_BIN` / `SAI_PAPER_BIN` / `SAI_SCI_BIN` と PATH を確認（コンテナの中では中継 `/opt/sai/bin/…`）。**バイナリ名を変えても同じ文が出る** |
| `Mac の sai-radio が応答しません` | 503 | `apps/radio`（8201）が止まっている／`SAI_RADIO_URL` 間違い（文言は今の Web のまま） |
| `…の応答を読めませんでした` | 502 | 終了コード 0 なのに JSON でない（**標準出力への余計な表示**を疑う） |
| CLI の標準エラーの最後の行（例 `論文が見つかりません`） | 404 / 400 / 409 / 502 | 終了コードが 3 / 2 / 4 / 1。文言は CLI のまま |
| `period は week / month / all のいずれかにしてください` `action は start / stop を指定してください` `検索先が分かりません: …` `run_id は 1 以上の整数にしてください` `block の指定が正しくありません` など | 400 | **Web 自身の検査**。CLI を呼ぶ前に弾いている。相手側を疑わない |
| `まだありません` | 404 | その run に md が無い（`run md` の終了コード 3） |
| `未実行のカードが見つかりません` | 404 | その run の未実行カードが無い（`card run` / `card start` の終了コード 3） |
| `このカードはエージェントが子の run として起こします（executer: agent）。…` | 502 | `executer: agent` のカードは人が起こす道では動かさない |
| `プロジェクトが見つかりません` / `このプロジェクトにはまだ実行の記録がありません` | 404 | 登録簿に無い／`db_exists` が偽 |
| `PDF がありません` / `音声が見つかりません` | 404 | ファイル・音声が無い |
| `この操作は sai-web からは使えません` | 403 | 中継先で `papers/upload` / `papers/from-extract` を呼ぼうとした（仕様） |
| `認証エラー` | 401 | 合言葉が合わない（`SAI_WEB_TOKEN` / `SAI_RADIO_TOKEN`） |
| `…が時間内に応答しませんでした` | 504 | 時間切れ |
| 画面の帯 `接続できていません` | 通信失敗 | 8200 自体に届かない（サーバー・tailscale・ネットワーク） |
| プロジェクト一覧が **空** | ― | `SAI_HOME` が正しいか、`$SAI_HOME/registry.json` にそのプロジェクトがあるか確かめる |

### 7.2 ログの見方

| どこを見る | 中身 |
|---|---|
| **Web**: `$SAI_HOME/web/sai-web.log`／起動したターミナル | `<CLI 名> の標準エラー: …`（先頭 300 文字）。**CLI の失敗原因はまずここ** |
| **apps/radio** | 起動しているターミナル。音声化の失敗は Web のログの `sai-paper` の標準エラーと、`apps/radio` 自身の例外・LLM のエラー |
| **3 つの CLI** | 単発の CLI なので、Web のログに流れた標準エラーが手がかり。直接試すときは手元で `…; echo $?` |
| 音声化の状態 | sai-paper の DB の `paper_podcasts`（`generating` / `ready` / `failed` + `error`）。論文情報画面にも「前回は失敗しました: …」が出る |

### 7.3 直す順番

1. 画面の `detail` から **503 / 502 / 400 / 404 / 409 のどれか**を決める。
2. 503 → 場所（PATH・設定・常駐）。502 → **出力の混入**か終了コード 1。400 → Web の検査か CLI の終了コード 2。404 → データの有無。
3. §6.2 の確認を**読み取りだけで**流す。
4. 直したら §6.3 のビルド・作り直し・文書更新を行う。
