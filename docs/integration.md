# sai-web の API と画面の住所

**正はコード**（`apps/web/src/sai_web/` と `crates/`・`apps/radio` の実装）。この文書とコードが食い違ったら、コードに合わせて直す。
`crates/`・`apps/radio` 側を直すときの注意は [`for-sai-developers.md`](./for-sai-developers.md)、全体の考え方は [`../ARCHITECTURE.md`](../ARCHITECTURE.md)、言葉の意味は [`../CONCEPT.md`](../CONCEPT.md)。

この文書の API の数は、`main.py` の `@app.get`・`post`・`put`・`delete`・`api_route` を数えた **110 本**（着手時点）。

---

## 1. 全体のつながり

```
iPhone / PC のブラウザ
   │  HTTPS  tailscale serve :4443（iPhone だけ）
   ▼
127.0.0.1:8200  sai-web（FastAPI・窓口だけ）
   ├─ Chats   : $SAI_HOME/web/runs/<project>/<run_id>.json を読む   ← `sai-link run export` が書き出す
   ├─ Projects: 登録簿 $SAI_HOME/registry.json を直接読む ／ `sai-link …` を呼ぶ
   ├─ Papers  : `sai-paper …`（cwd = そのプロジェクトの root・`SAI_PROJECT_ROOT` も同じ）
   ├─ Research: `sai-sci …`（同上）
   ├─ コマンド : 3 つの CLI の `schema --json` を読み、`web: true` の命令を実行する
   ├─ Radio   : /api/radio/* を 127.0.0.1:8201 の apps/radio の /api/* へ中継
   └─ MLflow  : /api/mlflow/* を 127.0.0.1:5001 の MLflow へ読み取りだけで中継

音声化の依頼:    sai-web ──(背景で数分)──▶ `sai-paper paper radio <id>`
                                          └─(HTTP)──▶ apps/radio POST /api/papers/from-extract
Library に入れる: sai-web ──▶ `sai-paper paper keep …`（PDF 無しで登録・すぐ返る）
前処理の束の取込: sai-prep ──(zip を POST)──▶ sai-web（検査・展開）──(背景)──▶ `sai-paper paper import-bundle --dir <フォルダ>`
                                          └─(通知)──▶「保存しました」
論文の移植:      sai-prep ──(移植の束の zip を POST)──▶ sai-web（検査・展開）──(背景)──▶ `sai-paper library import-migration --dir <フォルダ>`
                 別プロジェクト（同じ利用者）──(sai-web が仲立ち)──▶ 移植元で `sai-paper library export --ids …` ──(移す)──▶ 移植先で `import-migration`
                                          └─(通知)──▶「論文を移しました」
```

| タブ / 画面 | 使う物 |
|---|---|
| Chats（日付・実行・メッセージ・統計） | `sai-link` が書く書き出し JSON（`$SAI_HOME/web/runs/`）と会話の記録（.jsonl） |
| Projects（一覧・Dashboard・List・カードの作成と実行・常駐・task / review の保存） | `sai-link project summary` / `project dashboard` / `daemon start\|stop` / `run tree` / `run log\|stats\|history` / `run md` / `doc list\|read\|search` / `card options\|docs\|create\|run\|schedule` / `run advance\|finalize\|rm`。一覧の `name` / `root` / `kind` / `root_exists` / `db_exists` / `daemon_alive` は登録簿（`registry.py`）から作る |
| Projects → Workflow | `sai-link workflow`（Workflow の配置は sai-link が計算する） |
| Projects → 設定（block / template / settings） | `sai-link def list\|show\|write\|delete`（`scope` で、そのプロジェクトと `$SAI_HOME` の両方） |
| Projects → コマンド | `GET /api/projects/{name}/commands`（3 つの CLI の `schema --json`）と `POST …/commands/run` |
| Projects → セッション検索 | `sai-link session search` |
| Papers → Dashboard | `sai-paper dashboard`（中で `apps/radio` の `/api/logs?limit=` を読み、そのプロジェクトの音声の分だけ混ぜる） |
| Papers → Library・論文情報 | `sai-paper library list\|tree` / `paper show\|note\|translate\|translate-pdf\|citations\|citations-extract\|radio` / `content paper` / `search` / `search-history` / `find` / `find-history` / `providers` |
| Papers → Import（BibTeX） | `sai-paper import list\|add\|remove` |
| Papers → Library に入れる | `sai-paper paper keep (--log <id> --position <p> \| --import-id <id> \| --cite <id> --ref <n>)` |
| Papers → 前処理の束（sai-prep） | `sai-paper paper import-bundle --dir <フォルダ>` / `sai-paper library pending` |
| Papers → Radio | `apps/radio` の `/api/papers` `/api/papers/{id}` `/audio` `/listen` `/favorite` `/logs` `/ask` `/live-session` `/question-log` |
| Research → research-tree・READ_CARD | `sai-sci status\|types\|init\|node\|apply\|card\|read`、READ_CARD の開始と終了は `sai-link card start\|finish` |
| Research → Experiments | `apps/mlflow-server` の MLflow（`/api/mlflow/*`・読み取りだけ） |
| Home（質問・プロジェクト・todo・論文の要対応） | 上の住所を 1 本の住所（`GET /api/home`）にまとめる。`ThreadPoolExecutor` で並べる。要対応は `sai-sci attention list --json` |
| Home の全プロジェクトの合計（今週 / 今月 / 全期間） | 各プロジェクトの `sai-link project dashboard --period` を並べて足し合わせる（`GET /api/home/overview`） |
| Usage | `usage.py`（OpenRouter・DeepSeek。古い pi の記録も読む）。CLI は呼ばない |

---

## 2. 大原則

1. **sai-web は窓口だけ。** 処理とデータの保管は各部品がする。
   - 他の部品の DB（`papr.db` / `sai-engine.db` / `sai_radio.db`）を**直接開かない**。DB の形を変えたときに sai-web が壊れ、原因が追いにくくなるため。
     （例外は 1 つだけ: Chats は `sai-link` が書き出す `$SAI_HOME/web/runs/*.json` を読む。これは「公開された書き出し」であり DB ではない。）
   - 機能は **3 つの CLI（`--json`）か HTTP API** だけを使う。
2. **既にある物を使う。** 新しい依存ライブラリは入れない（FastAPI・標準ライブラリ・素の JS のみ）。
3. **画面は素の JavaScript**（フレームワーク・ビルド無し）。グラフは SVG を手で描く。
4. **合言葉（`SAI_WEB_TOKEN`）で入口を守る。** エラーは画面に日本語でそのまま出す。
5. **外から来る値は CLI を呼ぶ前に検査する。** 通らない値では CLI を 1 度も呼ばない（400）。ファイルのパスは受け取らない。

---

## 3. 置き場（`SAI_HOME`）

| 置き場 | 場所 |
|---|---|
| 登録簿 | `$SAI_HOME/registry.json` |
| 書き出し（Chats が読む） | `$SAI_HOME/web/runs/` |
| 共通の block・template・settings | `$SAI_HOME/blocks/`・`template/`・`settings.json` |
| 論文の共通の API キー | `$SAI_HOME/paper.env` |
| 合言葉・質問の保管・通知の鍵・ログ | `$SAI_HOME/web/token`・`sai_web.db`・`vapid.json`・`sai-web.log` |
| **プロジェクト内の `.sai/`** | `<root>/.sai/`（`sai-engine.db`・`blocks/`・`template/`・`settings.json`・`outputs/`・`paper/`）。`SAI_HOME` とは別 |

- `SAI_HOME` は空でない値があればそれ、無ければ `~/.sai`。3 か所（sai-link・sai-paper・`registry.sai_home()`）で同じ規則を使う。

### 3.1 多人数モードの置き場（共有契約 C4）

環境変数 `SAI_WEB_USERS_ROOT`（空でない絶対パス。例 `/srv/sai/users`）を付けて `sai-web` を起こすと**多人数モード**になる。無ければ今までの**単独モード**（合言葉。開発とテスト用）で何も変わらない。

以下 `<U>` は `SAI_WEB_USERS_ROOT`、`<user>` は管理者が決める短い名前（`^[a-z][a-z0-9]{1,15}$`・ハイフンは許さない）。

| 置き場 | 場所 | 中身 |
|---|---|---|
| その人のひな形の HOME | `<U>/<user>/home/` | `.sai/`（Web が引くその人の `SAI_HOME`）など。コンテナには見せない |
| その人の Claude のログイン（共有契約 C-MOUNT） | `<U>/<user>/claude/`（0700） | cc のログイン（`.credentials.json`・会話の記録など）。その人の全プロジェクトのコンテナの `$HOME/.claude` に bind mount する。`$HOME/.claude.json` は共有しない |
| プロジェクトの root | `<U>/<user>/projects/<user>-<project>/` | 登録簿の名前もフォルダ名と同じ `<user>-<project>` |
| プロジェクトのコンテナの HOME | `<U>/<user>/homes/<user>-<project>/` | `.claude`（共有のログインの mount の下に隠れる。作るときに会話の記録だけ `<U>/<user>/claude/` へ写す）・`.claude.json`・`.sai`。コンテナの中の `SAI_HOME` はこの中の `.sai`。実行の書き出しは `<この HOME>/.sai/web/runs/<user>-<project>/` |
| その人の鍵 | `<U>/<user>/secrets.env` | API キーと Claude の長命トークン（0600）。コンテナには `--env-file` で渡し、ファイルそのものは見せない |

- 多人数モードでは、今の利用者（ログインした人）ごとに `registry.sai_home()`・`registry.user_home()`・`registry.project_home()` が変わり、プロジェクトの一覧・実行の書き出し・古い pi の記録・Usage が分かれる（共有契約 C5）。
- 今の利用者がいないとき（未ログイン）の `registry.sai_home()` はサーバ自身の `SAI_HOME`。Web 自身の DB・通知の鍵はここに置く。
- 多人数モードの登録簿はフォルダを数える（`<U>/<user>/projects/` の直下の `<user>-` で始まり `.sai/` を持つフォルダ）。`registry.json` は読まない。

---

## 4. API 一覧

すべて合言葉（`require_token`）で守る（内部用の 6 本を除く）。エラーは `{"detail": "<日本語>"}`。
「呼ぶ CLI」の列は、cwd = プロジェクトの root、`SAI_PROJECT_ROOT` = 同じ root、`SAI_ORIGIN=phone` で呼ぶ命令。終了コードから HTTP の状態への読み替えは [§7](#7-cli-の失敗の読み替え) の 1 か所。

### 4.1 プロジェクト（一覧・Dashboard・常駐）

| メソッド・パス | 中身 | 呼ぶ CLI |
|---|---|---|
| `GET /api/projects` | プロジェクト一覧（`{"projects": [...]}`）。`running` と `last_executed_at` は 5 秒だけ覚える。多人数モードでは各行に `container_state`（`running` / `exited` / `missing`）が付く | `sai-link project summary`。あわせて 60 秒に 1 回 `sai-link run export`（書き出しの更新） |
| `POST /api/projects` `{"name":"<短い名前>"}` | プロジェクトを作る（多人数モードだけ）。`{"name":"<user>-<名前>","container":"sai-<user>-<名前>"}`。名前の誤り 400「名前は英小文字・数字・ハイフンで 30 文字までにしてください」・同じ名前 409「同じ名前のプロジェクトがあります」 | コンテナの中で `sai-link init --name <名前>`・`sai-paper init`・`sai-link daemon start` |
| `GET /api/projects/{name}/dashboard?period=week\|month\|all` | Dashboard（既定 week） | `sai-link project dashboard --period <期間>` |
| `POST /api/projects/{name}/daemon` `{"action":"start"\|"stop"}` | 常駐の起動・停止 | `sai-link daemon start\|stop`（文字出力） |
| `GET /api/projects/{name}/container` | 自分のプロジェクトのコンテナの状態 `{"container","state"}`（多人数モードだけ）。他人の物は 404 | `docker inspect -f '{{.State.Status}}'` |
| `POST /api/projects/{name}/ts/open` | 自分のプロジェクトの時系列（sai-ts）を開く入場券つきの URL `{"url": "<SAI_WEB_TS_URL>/p/<名前>/__gate/enter?ticket=<券>"}`（多人数モードだけ・他人の物は 404）。券を出す前に `containers.ts_ensure` で sai-ts を起こす（起動の失敗はそのまま返す）。`SAI_WEB_TS_URL` が空なら 503 | `docker` の CLI（`containers.ts_ensure`）。券は `ts_gate.issue_ticket`（4.11） |
| `POST /api/projects/{name}/container/{start\|stop}` | 自分のプロジェクトのコンテナを起動・停止（多人数モードだけ）。未知の action は 400 | `docker start` / `docker stop -t 20`。`start` でコンテナが無いときは作り直す |

- コンテナの操作は `docker` の CLI を `subprocess` で呼ぶ（`apps/web/src/sai_web/containers.py`。実行ファイルは `SAI_DOCKER_BIN`、既定 `docker`）。Python の docker の SDK は使わない（依存を増やさない）。
- プロジェクトを作るときは、`<U>/<user>/projects/<user>-<名前>/`（root）と `<U>/<user>/homes/<user>-<名前>/`（HOME）を作り、`<U>/<user>/secrets.env`（0600）を用意し、`SAI_WEB_SKEL_DIR` の中身を（無いファイルだけ）写してから、コンテナを `docker run` で作り、init 3 本を `docker exec` で呼ぶ。途中で失敗したらコンテナとフォルダを消して 502 を返す。
- コンテナの形は共有契約 C7・C8（`run_args`）。`SAI_CONTAINER_MEMORY`（既定 `8g`）・`SAI_CONTAINER_CPUS`（既定 `2`）・`SAI_IMAGE`（既定 `sai-tree:latest`）・`SAI_WEB_URL_FOR_CONTAINERS`（既定 `http://host.docker.internal:8200`）で変えられる。
- docker のデーモンにつながらない・実行ファイルが無いときは 503「docker が動いていません」、その他の失敗は 502（標準エラーの最後の行）。
- 多人数モードの常駐の生死（`registry.daemon_alive`）は、コンテナの中の PID がホストから見えないため、常駐が約 5 秒ごとに書く `<root>/.sai/sai-engine/daemon.status` の更新時刻が 30 秒以内かで見る（単独モードは今までどおり `daemon.pid`）。

### 4.2 ホーム

| メソッド・パス | 中身 | 呼ぶ CLI |
|---|---|---|
| `GET /api/home` | 質問・プロジェクト（review の run と todo の行）・論文の要対応・`errors` | `sai-link run tree`・`sai-sci attention list --json` |
| `GET /api/home/overview?period=week\|month\|all` | 全プロジェクトをまたいだ合計（`period` 省略時は `week`・それ以外は 400） | `sai-link project dashboard --period <期間>` |

- 質問は `store.list_open_questions` から作る。`project` / `run_id` は `runs.load_runs` の中で `run_session_ids` に `session_id` を含む実行を探して付ける（無ければ `null`）。
- プロジェクトの `review` / `todo` は `run tree` の `review` / `todo` フォルダを数える。論文の要対応は `attention list --json` の件数と先頭 3 件。
- `todo` は件数、`todo_runs` は同じフォルダの行（`[{run_id, label}]`・ツリーの順）。todo の行には `card`（`creation_date`・`block`・`runner`・`model`・`description`）が付き、実行予約した行には `scheduled_at`（`YYYY-MM-DD HH:MM`）が付く。`review` はホームには出さない（API には残す）。
- 各プロジェクトは `ThreadPoolExecutor(max_workers=4)` で並べる。1 つが失敗しても全体は止めず `errors` に「<名前>: <理由>」を足す。結果は 10 秒だけ覚える（`home.forget()` で消す）。

`/api/home/overview` は次を返す。

```json
{
  "period": "week", "since": "2026-09-21", "until": "2026-09-27", "projects": 2,
  "summary": {"executions": 12, "completed": 10, "failed": 1, "running": 1,
              "success_rate": 0.83, "duration_ms": 3600000,
              "tokens_input": 1, "tokens_output": 2, "tokens_cache": 3, "tokens_total": 1234567,
              "cost_usd": 3.21},
  "daily": [{"date": "2026-09-21", "executions": 3, "tokens": 120000}],
  "recent": [{"project": "sai-tree", "run_id": 42, "parent_run_id": 1, "block": "worker",
              "status": "completed", "started_at": "2026-09-27 10:28:21",
              "description": "…", "linkable": true}],
  "errors": []
}
```

- 対象は `/api/home` と同じプロジェクト（`root_exists` と `db_exists` が真）。`linkable` が偽の `recent` は会話画面が開けないので List 側の画面へ。
- `summary` は各プロジェクトの同じ名前の数を足し、`success_rate` は足したあとの `completed / executions`（0 件なら `0.0`）で求め直す。`daily` は日付ごとに足す。`recent` は `started_at` の新しい順（同じなら `run_id` の大きい順）で最大 20 件。期間も 10 秒だけ覚える。

### 4.3 Workflow と設定（`sai-link workflow` / `def` 経由）

| メソッド・パス | 中身 | 呼ぶ CLI |
|---|---|---|
| `GET /api/projects/{name}/workflow` | Workflow 盤面（札・線・separators・Kanban の 4 レーン） | `sai-link workflow` |
| `GET /api/projects/{name}/defs` | 定義の一覧 | `sai-link def list` |
| `GET /api/projects/{name}/defs/{kind}/{scope}/{def_name}` | 定義 1 件の中身 | `sai-link def show --kind <kind> --scope <scope> --name <名前>` |
| `PUT /api/projects/{name}/defs/{kind}/{scope}/{def_name}` | 定義の保存（body `{"text": …, "create": false}`。本文は標準入力。`create` が真なら `--create`） | `sai-link def write --kind … --scope … --name … [--create]` |
| `DELETE /api/projects/{name}/defs/{kind}/{scope}/{def_name}` | 定義の削除（settings は 400「settings は消せません」） | `sai-link def delete --kind … --scope … --name …` |

- `kind` = `block` / `template` / `settings`、`scope` = `project` / `global`。`def_name` は block / template が `^[A-Za-z0-9][A-Za-z0-9_-]{0,63}$`、settings が `settings` だけ。違えば 400。
- `scope=global` の置き場は `$SAI_HOME`。
- 定義の編集は **DB が無いプロジェクトでもできる**（root が無いときだけ 404「プロジェクトのフォルダが見つかりません」）。`workflow` は実行の記録が要るので 404。
- 本文は 256KB まで。検査は CLI を呼ぶ**前**に行う。`write --create` で同じ名前があるときは 409「同じ名前の定義があります」にする（CLI は終了コード 2 で返すため、`control.py` が読み替える）。
- 住所の `{def_name}` は FastAPI 上の変数名も同じ（`token` は合言葉の `?token=` と衝突するため使わない）。

### 4.4 List（`sai-link` 経由）

| メソッド・パス | 中身 | 呼ぶ CLI |
|---|---|---|
| `GET /api/projects/{name}/tree` | 引き出しツリー。各 run ノードは `executer`（`human` / `agent`）を持つ。`agent` の todo には画面が ▶ 実行を出さない | `sai-link run tree` |
| `GET /api/projects/{name}/search?q=…` | 文書（task / message / waggle）の本文検索。`q`（空白区切りで AND・200 文字まで）に加え `category`（カンマ区切り・空で全部）・`since` / `until`（`YYYY-MM-DD`）・`block`（完全一致）・`from_id` / `to_id`（run_id の範囲）・`limit`（1〜100・既定 50）。返り値は `{items: [{run_id, category, file, token?, block, date, snippet}], truncated}` | `sai-link doc search --word … --category … --limit …` |
| `GET /api/projects/{name}/sessions/search` | 会話（セッション履歴）の検索。クエリは `word`・`global_round`・`round`・`id`・`block`・`date`・`status`・`type`。先頭 200 件を `{"items", "truncated"}` で返す | `sai-link session search -R … -r … --id … --block … --date … --status … --type … -w … --json` |
| `GET /api/projects/{name}/runs/{run_id:int}/view/{log\|stats\|history}` | run の中身 | `sai-link run log\|stats\|history <run_id>` |
| `GET /api/projects/{name}/runs/{run_id:int}/history/calls/{call_id}` | History の 1 呼び出しの全文（`input`・`result`（20000 字まで）・`truncated`）。`call_id` は `^[A-Za-z0-9_-]{1,80}$`（通らなければ 400）。見つからなければ 404 | `sai-link run history <run_id> --call <call_id> --json` |
| `GET /api/projects/{name}/runs/{run_id:int}/md/{task\|message\|report\|system}` | md 本文（「まだありません」は 404）。`report` は run の報告（`finalize-report.md`）、`system` は固定部分（`system-prompt.md`） | `sai-link run md <run_id> --kind <種類>` |
| `PUT /api/projects/{name}/runs/{run_id:int}/md/{task}` | md の保存（body `{"text"}`。task は未実行の todo カードだけで、それ以外は 404「未実行のカードが見つかりません」） | `sai-link run md <run_id> --kind task --write`（標準入力） |
| `GET /api/projects/{name}/runs/{run_id:int}/waggle` | 成果物の一覧 | `sai-link doc list --run <run_id> --json` |
| `GET /api/projects/{name}/runs/{run_id:int}/waggle/{token}` | 成果物 1 件の本文 | `sai-link doc read <トークン> --json` |
| `POST /api/projects/{name}/runs/{run_id:int}/start` | 新形式 todo の ▶ 実行（未実行カードが無ければ 404。`executer: agent` のカードは 502「このカードはエージェントが子の run として起こします（executer: agent）。…」） | `sai-link card run <run_id>` |
| `POST /api/projects/{name}/runs/{run_id:int}/advance` | review → done → Workflow の移動（その run が無ければ 404、review / done 以外は 400「review / done の run だけ移動できます」） | `sai-link run advance <run_id>` |
| `POST /api/projects/{name}/runs/{run_id:int}/schedule` | todo の実行予定時刻（`{"at": "YYYY-MM-DD HH:MM"}`・空で取り消し）。時刻になると常駐が `card run` と同じ流れで実行する | `sai-link card schedule <run_id> [--at "…"]` |
| `DELETE /api/projects/{name}/runs/{run_id:int}` | List の項目を消す（子の実行と run フォルダも消える。実行中は 502） | `sai-link run rm <run_id> --yes` |
| `GET /api/projects/{name}/card/docs` | 最近の文書（task_text に貼る `ref` 付き） | `sai-link card docs` |
| `GET /api/projects/{name}/card/options` | 作成フォームの選択肢 | `sai-link card options` |
| `POST /api/projects/{name}/card` | カード（task.md）の作成（任意の `executer`（`human` / `agent`。旧 `professor` も `agent` と読む）も渡せる） | `sai-link card create`（標準入力に JSON） |
| `GET /api/projects/{name}/commands` | Web に出せる命令の一覧（`web: true` の命令と引数だけ） | 3 つの CLI の `schema --json` |
| `POST /api/projects/{name}/commands/run` | 命令を実行する（body `{"cli": "link\|paper\|sci", "path": [...], "args": {...}, "stdin": "…"}`。戻りは `{"ok": true, "json": …, "text": …}`） | `schema` が許した命令 |

- 外から来る値は sai-web 自身が CLI を呼ぶ**前**に検査する（`run_id` は正の整数、`view` / `kind` は一覧の値だけ、`token` は英数字と `-_.:`・長さ 128 まで）→ 400。
- 作成フォームも同じ（`block` は英数字と `-_.:`、`task_text` は 100KB まで、`runner` は `user` / `cc` / `gl`（`pi` は廃止のため 400）、`thinking` は一覧の値、入力の値は `waggle:` 形式、`global_round` / `round` は数字、`tag` は 1 行）→ 400。
- 「コマンド」画面の実行も、`cli` は 3 つのうち 1 つ、`path` は schema にある命令、`args` は `web: true` の引数だけ、型・必須・値の一覧・1 行 2000 文字・複数値 100 個まで、標準入力は 1MB まで（`stdin` が `json` の命令は JSON として読めること、`bytes` の命令は拒否）を、CLI を呼ぶ前に確かめる。時間の上限は 120 秒、標準出力は 2MB まで（超えたら 502）。
- CLI の失敗は §7 の読み替え。404 は 2 種類で、md が無いとき（終了コード 3・stderr「まだありません」）と未実行カードが無いとき（終了コード 3・stderr「未実行のカードが見つかりません」）。
- 呼び出しは cwd = プロジェクトの root、かつ `SAI_PROJECT_ROOT` = 同じ root（常駐を起動した環境変数に引きずられない）。
- 住所の `{token}` は FastAPI 上の変数名だけ `{waggle_token}`（合言葉の `?token=` と名前が衝突するため。URL の形は同じ）。

### 4.5 論文（`sai-paper` 経由）

| メソッド・パス | 中身 | 呼ぶ CLI |
|---|---|---|
| `GET /api/paper-projects` | 論文の置き場（`.sai/paper/`）があるプロジェクトの一覧 `[{name, root_name, last_executed_at}]` | 登録簿とファイルの有無だけ（CLI は呼ばない） |
| `GET /api/projects/{name}/papers/library` | Library 一覧 | `sai-paper library list` |
| `GET /api/projects/{name}/papers/tree` | Library 階層（`find_groups`（内部検索の履歴の組）も返る） | `sai-paper library tree` |
| `GET /api/projects/{name}/papers/providers` | 検索先の一覧と既定 | `sai-paper providers` |
| `POST /api/projects/{name}/papers/search` | 検索（履歴にも残る） | `sai-paper search --json --limit … [--db …] [--year-from …] [--year-to …] -- <語>` |
| `GET /api/projects/{name}/papers/search-history/{log_id}` | 過去の検索結果 | `sai-paper search-history <log_id>` |
| `POST /api/projects/{name}/papers/find` | 内部検索（Library 内の検索・履歴にも残る）。`node` を付けると範囲の決定を sai-sci が行う（記録は `SAI_TARGET_NODE` があるときだけ） | `sai-paper find --json … -- <語>`（`node` があれば `sai-sci find --json --node <ID> …`） |
| `GET /api/projects/{name}/papers/find-history/{log_id}` | 過去の内部検索の結果 | `sai-paper find-history <log_id>` |
| `GET /api/projects/{name}/papers/{paper_id:int}` | 論文情報 | `sai-paper paper show <id>` |
| `POST /api/projects/{name}/papers/{paper_id:int}/translate` | 要旨の日本語訳 | `sai-paper paper translate <id>` |
| `POST /api/projects/{name}/papers/{paper_id:int}/translate-pdf` | PDF の日本語訳を始める（202 `{"key": "pdf:<id>", "status": "processing"}`・翻訳中は 409。仕事は `downloads` に `kind="translate"`・`percent=null` で出る） | `sai-paper paper translate-pdf <id> --json` |
| `POST /api/projects/{name}/papers/{paper_id:int}/index` | 論文 1 本の Index（chunk 作成と埋め込み）を始める（202 `{"key": "index:<id>", "status": "processing"}`・作成中は 409。終わりは通知と `downloads` の `kind="index"` で分かる） | `sai-paper index paper <id> --json` |
| `GET /api/projects/{name}/papers/{paper_id:int}/text` | 本文（Markdown） | `sai-paper content paper <id> --json` |
| `GET /api/projects/{name}/papers/{paper_id:int}/chunks` | 論文の全チャンク（`#論文.章.段落` の段落）。字数の上限なし・章番号、段落番号の昇順。論文が無い・本文がまだ取り出されていないときは 404。返す JSON: `{"paper_id": 57, "title": "題名", "chunks": [{"id": "#57.3.2", "section": "#57.3", "heading": "3 Method", "text": "本文", "n_sentences": 4}]}` | `sai-paper content chunks <id> --json` |
| `GET /api/projects/{name}/papers/{paper_id:int}/pdf?lang=ja` | PDF（`lang=ja` で日本語訳・パスがそのプロジェクトの root 外なら 404） | `sai-paper paper show <id>` の `files` のパスを読む |
| `GET /api/projects/{name}/papers/dashboard` | Dashboard（数字カードと出来事） | `sai-paper dashboard` |
| `PUT /api/projects/{name}/papers/{paper_id:int}/note` | ノートの保存（body `{"text"}`・100KB まで） | `sai-paper paper note <id>`（標準入力） |
| `GET /api/projects/{name}/papers/{paper_id:int}/figures/{fig_no:int}` | 図の画像（`<img>` 用に合言葉は `?token=`・root 外なら 404） | `sai-paper paper show <id>` の `figures.items[].image` を読む |
| `POST /api/projects/{name}/papers/{paper_id:int}/radio` | 音声化の依頼（背景で実行・202・返り値に `radio_id`） | `sai-paper paper radio <id>`（`apps/radio` を呼ぶ） |
| `GET /api/projects/{name}/papers/import` | .bib の一覧と中の論文 | `sai-paper import list` |
| `POST /api/projects/{name}/papers/import` | .bib の保存（body `{"name": "refs.bib" \| "", "text": "@article{…}"}`・本文は標準入力。名前が空なら `phone-<YYYYMMDD-HHMMSS>.bib`） | `sai-paper import add --name <名前>`（標準入力） |
| `DELETE /api/projects/{name}/papers/import/{file_name}` | .bib の削除 | `sai-paper import remove --name <名前>` |
| `POST /api/projects/{name}/papers/keep` | PDF 無しで Library に入れる（200）。body は「`{"log_id": 3, "position": 2, "title": "…"}`」「`{"import_id": "DOI:…", "title": "…"}`」「`{"cite_paper_id": 12, "ref_no": 3, "title": "…"}`」のどれか 1 つ。返す JSON: `{"paper_id", "title", "status": "added"\|"exists"}` | `sai-paper paper keep (--log <id> --position <p> \| --import-id <id> \| --cite <id> --ref <n>) --json` |
| `POST /api/projects/{name}/papers/bundle` | sai-prep の「前処理の束」（zip）を受け取る（202・背景）。本文は zip そのもの（`Content-Type: application/zip`・300MB まで）。Web が検査・展開してから `import-bundle` を走らせる。種類が違えば 415・大きすぎ／Content-Length 無しは 413・中身が違えば 400（CLI を呼ばない）。返す JSON: `{"key": "bundle:<uuid>", "status": "processing"}` | `sai-paper paper import-bundle --dir <展開したフォルダ> --json` |
| `POST /api/projects/{name}/papers/migrate` | sai-prep の「移植の束」（zip）を受け取る（202・背景）。本文は zip そのもの（`Content-Type: application/zip`・300MB まで）。Web が検査・展開してから `import-migration` を走らせる。種類が違えば 415・大きすぎ／Content-Length 無しは 413・中身が違えば 400（CLI を呼ばない）。返す JSON: `{"key": "migrate:<uuid>", "status": "processing"}` | `sai-paper library import-migration --dir <展開したフォルダ> --json` |
| `POST /api/projects/{name}/papers/migrate-from` | 同じ利用者の別のプロジェクトから論文を移す（202・背景）。body `{"source": "<移植元のプロジェクト名>", "ids": [5, 6, 12]}`（ids は 1〜1000 個の正の整数・重複は 1 つにまとめる）。移植元は `papers/resolve_project` で引ける自分のプロジェクトだけ（引けなければ 404）。Web が移植元で書き出し（outbox）→ 移植先の inbox へ移す → 移植先で取り込む。返す JSON: `{"key": "migrate:<uuid>", "status": "processing"}`。source が空・移植先と同じ・ids が不正なら 400 | 移植元で `sai-paper library export --ids <ids> --out <outbox> --json`、移植先で `sai-paper library import-migration --dir <inbox> --json` |
| `GET /api/projects/{name}/papers/pending` | PDF 待ちの論文の一覧（sai-prep が使う） | `sai-paper library pending --json` |
| `POST /api/projects/{name}/data/raw?filename=<名前>` | 解析用のデータを 1 件置く（sai-prep の Data upload が使う・201）。本文はファイルそのもの（種類は問わない・`Content-Length` が必要）。`<root>/data/raw/<名前>` に置く。返す JSON: `{"name", "size", "path"}`。名前は `^[A-Za-z0-9][A-Za-z0-9_.-]{0,127}$`（違えば 400）。同じ名前は 409（上書きしない）。1 件 5GB まで（超えれば 413）。無いプロジェクト・root が無いプロジェクトは 404。ディスクへ流し書きする（途中で切れたら `.part` を消す） | （sai-web が直接書く。CLI は呼ばない） |
| `GET /api/projects/{name}/data/raw` | `data/raw/` の一覧（名前の昇順）。返す JSON: `{"files": [{"name", "size", "mtime"}]}`（`mtime` は ISO 8601・UTC）。隠しファイルと `.part` は出さない。置き場が無ければ空の一覧 | （sai-web が直接読む） |
| `GET /api/projects/{name}/papers/jobs/{key}` | 仕事 1 件（sai-prep が進み具合を見る）。無ければ 404。返す JSON: `{"key", "status", "title", "paper_id"?, "figures"?, "translated"?, "error"?, "result"?}`（`result` は移植の仕事が `done` のときだけ・`import-migration` の戻り） | （sai-web のメモリ） |
| `GET /api/projects/{name}/papers/downloads` | 走っている／終わった仕事（`{"jobs": [{key, kind, title, status, paper_id, error, started_at, finished_at, steps, step, percent, messages}]}`・新しい順）。`kind` は `bundle` / `migrate` / `cite-extract` / `radio` / `translate` の 5 つ。`percent` が `null` の仕事は「% の分からない仕事」（画面は動く帯で出す） | （sai-web のメモリ） |
| `GET /api/projects/{name}/papers/{paper_id:int}/citations` | 引用一覧（読み取りだけ・抜き出す前の論文にも空の形で返る） | `sai-paper paper citations <id>` |
| `POST /api/projects/{name}/papers/{paper_id:int}/citations/extract` | 引用を PDF から抜き出す（202・背景。docling を使うので数十秒〜数分）。進捗は `cite:<paper_id>` の仕事 | `sai-paper paper citations-extract <id>` |
| `GET /api/projects/{name}/papers/project` | 現況・考え・状況のまとめ・要対応 | `sai-sci project show --json` |
| `PUT /api/projects/{name}/papers/project/mind` | mind.md の全文を保存する（body `{"text": "…"}`・1〜200000 字・空白だけ・NUL は 400）。戻り `{"ok": true, "bytes": <n>}` | `sai-sci project mind --json`（本文は標準入力） |
| `POST /api/projects/{name}/papers/attention/resolve` | 要対応の 1 件を対応済みにする（body `{"key": "…"}`） | `sai-sci attention resolve <key> --json` |
| `GET /api/prep/download` | sai-prep の入れ物（一番新しい `sai_prep-*.whl`）を配る。無ければ 404「sai-prep の入れ物がまだ作られていません（管理者が deploy.sh を実行してください）」 | （ファイルを返すだけ・`SAI_PREP_DIST` があればそこを見る） |

- 古い `/api/papers/...` は無い。`{name}` は `GET /api/paper-projects` の名前で、無ければ 404。
- `import` / `downloads` / `keep` / `bundle` / `migrate` / `migrate-from` / `pending` / `jobs/{key}` / `project` / `project/mind` / `attention` / `find` / `find-history` / `library` / `tree` / `providers` / `search` / `search-history/{log_id}` は `/api/projects/{name}/papers/{paper_id:int}` より**先に**登録する（登録順に照合するため）。
- 外から来る値は papers 側で CLI を呼ぶ**前**に検査する（.bib の名前は `^[A-Za-z0-9._-]{1,80}\.bib$`・本文は 1MB まで・keep は対象 3 つのうち 1 つ）→ 400。同じ論文が進行中は 409。
- 前処理の束（zip）の検査は Web が行う（Rust に zip の依存が無い）。項目の数 ≤ 5000・展開後の合計 ≤ 1GB・絶対パス / `..` / `\` / NUL の名前無し・シンボリックリンク無し・`manifest.json` の形（下の C12）・`paper.pdf` の先頭が `%PDF-`。違えば CLI を 1 度も呼ばず 400。展開先は `<root>/.sai/paper/inbox/<uuid>/`（ホストとコンテナで同じパスに見える・C4）。
- 仕事の状態は sai-web のメモリだけに持つ。`processing` = 取り込み中（束）／後処理中、`done` = 終わり（`error` があれば後処理の失敗）、`failed` = 保存できなかった。終わった仕事は 30 分で一覧から消える。

**C12 前処理の束（zip）の形**（sai-prep が作り、Web が検査して展開する）:
- zip の中身: `manifest.json`・`paper.pdf`・`docling/paper.md`・`docling/paper.json`・`docling/paper_artifacts/…`・（翻訳したときだけ）`translated/paper-mono.pdf`・`translated/paper-dual.pdf`。
- `manifest.json`: `{"format":"sai-prep-bundle","version":1,"target":{"paper_id":<int>}|{"bibtex":"<1 件>"},"pdf":"paper.pdf","pdf_sha256":"<hex>","docling":{"md":"docling/paper.md","json":"docling/paper.json","dir":"docling"},"translation":{"mono":"…","dual":"…","service":"…"}|null,"tools":{"sai_prep","docling","pdf2zh"},"created_at":"RFC3339"}`。
- `import-bundle` の戻り: `{"paper_id","title","figures","sections","refs","translated"}`。
- `import-migration` の戻り（移植の仕事の `result` に入る・`GET …/papers/jobs/{key}` で読む）: `{"imported":[{"source_id","paper_id","title","mode":"new"|"filled"}],"skipped":[{"source_id","paper_id","title","reason"}],"failed":[{"source_id","title","error"}]}`。論文ごとの失敗は `failed` に入り、仕事は `done` のまま。
- 移植の束（C-M1）: zip の中身は `manifest.json`（`{"format":"sai-paper-migration","version":1,"papers":[<移植元の papers.id>, …]}`）・`papr.db`（論文の表だけの SQLite・先頭が `SQLite format 3\x00`）・`papers/<移植元のフォルダ名>/…`。

### 4.6 研究（`sai-sci` 経由・Research タブ）

| メソッド・パス | 中身 | 呼ぶ CLI |
|---|---|---|
| `GET /api/projects/{name}/papers/sci/status` | research-tree 全体 | `sai-sci status --json` |
| `GET /api/projects/{name}/papers/sci/types` | 使える RESEARCH_TYPE | `sai-sci types --json` |
| `POST /api/projects/{name}/papers/sci/init` | research-tree を始める（body `{"type": "problem"}`・型は 5 つだけ） | `sai-sci init --type <型> --json` |
| `GET /api/projects/{name}/papers/sci/nodes/{node_id}` | Node 1 つの中身（`{node_id}` は `Q12`・`A5`・`F3`・`E2`・`S2` の形） | `sai-sci node <ID> --json` |
| `POST /api/projects/{name}/papers/sci/apply` | 利用者の操作を 1 つ反映する（body は 14 種の op の JSON。READ_CARD の画面が足す `add_f` だけは任意の `run` を付け、その run の F として残す） | `sai-sci apply --json`（標準入力） |
| `POST /api/projects/{name}/papers/sci/cards` | Node のための CARD を作る（body は C-CARD の JSON。`child`（足す子の型 `Q`・`A`・`F`）は任意。`runner`（`cc` / `user`・既定 `cc`）・`model`・`thinking` は任意。`--executer` は無い） | `sai-sci card --from-json --json`（標準入力） |
| `GET /api/projects/{name}/papers/sci/reads/{run_id}` | READ_CARD の中身 | `sai-sci read <run_id> --json` |
| `POST /api/projects/{name}/papers/sci/reads/{run_id}/start` | READ_CARD を始める | `sai-link card start <run_id>` |
| `POST /api/projects/{name}/papers/sci/reads/{run_id}/finish` | READ_CARD を終えて review へ移す（始まっていなければ先に始める。F が 0 でも止めない。戻りは `{"run_id","status":"review","findings":n}`） | `sai-sci read <run_id> --json` → `sai-link card finish <run_id>` |

- API の住所は `papers/sci/…` のまま。画面の住所だけが Research（§6）に移った。
- `papers/sci` の 10 本は `sci.py` が検査してから CLI を呼ぶ。Node の ID は `^[QAFES][1-9][0-9]{0,8}$`、run_id は 1〜999999999 の整数、型は 5 つだけ、op は 14 種（項目と字数は [`sai-sci.md`](sai-sci.md) の 5.4 節）。通らない値は 400 で、CLI を 1 度も呼ばない。**ファイルのパスは受け取らない。**

### 4.7 Radio 中継

| メソッド・パス | 中身 |
|---|---|
| `GET/POST /api/radio/{path}` | `apps/radio` の `/api/{path}` へそのまま中継（`papers/upload` と `papers/from-extract` は 403） |

- 中継先は `SAI_RADIO_URL`（既定 `http://127.0.0.1:8201`）。合言葉 `SAI_RADIO_TOKEN` は `apps/radio` 側と同じ値を設定する。
- 中継で **パスの文字は `A-Za-z0-9_./-` のみ**（それ以外・`..` は 400）。

### 4.8 MLflow 中継（Research → Experiments）

| メソッド・パス | 中身 | MLflow への送り先 |
|---|---|---|
| `GET /api/mlflow/experiments?project=<name>` | 実験の一覧。`project` を付けると experiment のタグ `sai.project` で絞る（登録簿に無い名前は 404・MLflow は呼ばない） | `POST experiments/search`（`max_results` は 100・`order_by` は `name ASC`・`filter` は `tags."sai.project" = "<root のフォルダ名>"`） |
| `GET /api/mlflow/experiments/{experiment_id}` | 実験 1 件 | `GET experiments/get` |
| `POST /api/mlflow/runs/search` | 実行の検索 | `POST runs/search`（`run_view_type` は `ACTIVE_ONLY` 固定） |
| `GET /api/mlflow/runs/{run_id}` | 実行 1 件 | `GET runs/get` |
| `GET /api/mlflow/metrics/history` | 指標 1 本の履歴 | `GET metrics/get-history` |
| `GET /api/mlflow/metrics/bulk` | 指標 1 本の履歴をまとめて取る | `GET metrics/get-history-bulk-interval`（`run_ids` を繰り返しで送る） |
| `GET /api/mlflow/artifacts` | 成果物の一覧 | `GET artifacts/list` |
| `GET /api/mlflow/artifact` | 成果物 1 件の本文（5MB を超えたら 413） | `GET /get-artifact`（`run_uuid` と `path`） |

- 中継先は `SAI_MLFLOW_URL`（既定 `http://127.0.0.1:5001`）。名前は `SAI_MLFLOW_USER`・`SAI_MLFLOW_PASSWORD`（空なら Basic 認証を付けない）。
- **読み取りだけ**。書き込む住所は無い。
- 外から来る値は MLflow を呼ぶ**前**に検査する（実験 ID は数字の 1〜19 桁・run ID は 16 進の 32 桁・指標の名前は 250 字まで・成果物の場所は 500 字までで `..` の区切りと先頭の `/` は不可）→ 400。まとめて取る個数は `runs/search` が 100、`metrics/history` が 25000、`metrics/bulk` が 2500 まで。
- `POST /api/mlflow/runs/search` の本文は `{"experiment_ids": ["1"], "filter": "", "order_by": ["attributes.start_time DESC"], "max_results": 100, "page_token": "", "project": "sai-tree"}`。`project` があると**サーバが** `tags."sai.project" = "<登録簿の root のフォルダ名>"` を `filter` の前に `AND` で足す。登録簿に無い名前は 404。
- MLflow がエラーを返したら**本文と状態コードをそのまま**返す。つながらないときは 503「Mac の MLflow が応答しません」。成果物の `Content-Type` は拡張子から決める。5MB を超えたら 413「大きすぎるので表示しません（5MB まで）」。

### 4.9 会話・質問・通知・使用量

| メソッド・パス | 中身 |
|---|---|
| `GET /api/health` | 生存確認 |
| `GET /api/days` / `GET /api/days/{date}` | 会話の日付一覧・その日の実行 |
| `GET /api/runs/{project}/{run_id}` / `…/items` | 実行 1 件・会話の中身。`…/items` は `{items, next_cursor, live, partial, error}` を返す（`live` は `working` / `idle` / `blocked` / `null`、`partial` は生成中の文・`error` は直前のターンの失敗の理由。headless の実行は `state.json`（共有契約 C1）から読む・C6） |
| `POST /api/runs/{project}/{run_id}/send` | 実行中の run に文を送る（`sai-link run send <run_id> --json`・本文は標準入力）。終了・質問待ちは 409、無ければ 404 |
| `POST /api/runs/{project}/{run_id}/interrupt` | 実行中の run を中断する（`sai-link run interrupt <run_id> --json`）。無い実行は CLI を呼ばず 404 |
| `POST /api/runs/{project}/{run_id}/finalize` | メッセージ画面の ■ 終了（その run が無ければ 404、実行中でない・子ブロック・常駐停止は 502）。呼ぶ CLI は `sai-link run finalize <run_id>` |
| `POST /api/register` `/api/questions` | 登録（**内部専用**。単独モードは Mac 自身、多人数モードは `Authorization: Bearer <SAI_WEB_INTERNAL_TOKEN>`。cc のフックが呼ぶ） |
| `POST /api/questions/{qid}/resolved` | 質問が端末側で答えられたことの通知（**内部専用**） |
| `GET /api/questions/pending` / `GET /api/questions?session_id=` / `GET /api/questions/{qid}` | 答え待ちの質問（`{qid}` は**内部専用**。多人数モードでは持ち主の物だけ）。`{qid}` の JSON は `answered_by`（答えた人。端末で答えたときは `terminal`）を持つ |
| `GET /api/questions/mine` | **内部専用**。自分のプロジェクトの答え待ちだけを `{"questions": [{"id","agent","title","created_at","project","run_id"}, …]}` で返す（`/api/questions/pending` と同じ形を、プロジェクトで絞ったもの）。多人数モードでは合言葉のプロジェクトの分だけ。コンテナの agent の `sai-web questions` が使う |
| `POST /api/questions/{qid}/answer` | 答える（記録するだけ。cc のフックが `GET /api/questions/{qid}` で取りに来る）。**内部の合言葉**（コンテナの agent の `sai-web answer`）でも使えるが、その質問が合言葉のプロジェクトの物でなければ 403「このプロジェクトの質問ではありません」（分からないときも 403）。内部の合言葉で答えた記録は `answered_by` が `agent` になる。本文は今と同じ（`{"answers": [{choice_index \| choice_indices \| text}, …]}`） |
| `GET /api/push/key` / `POST /api/push/subscribe` | プッシュ通知の鍵と購読 |
| `GET /api/usage` / `GET /api/usage/{provider}/{account}/activity` / `…/logs` | 使用量（OpenRouter・DeepSeek。古い pi の記録も読む） |
| `GET /manifest.json` | PWA の設定 |

> **登録順の決まり**: FastAPI は登録した順に照合する。`/api/projects/{name}/papers/library` ほかの固定の住所は、**`/api/projects/{name}/papers/{paper_id:int}` より前**に登録しないと ID として誤認される（`main.py` のコメント参照）。静的ファイルの `app.mount("/")` は最後に置く。

### 4.10 アカウントとログイン（U06・多人数モード）

多人数モード（`SAI_WEB_USERS_ROOT` あり）では、ログインした人だけが `/api/…` を使える。守りはミドルウェア（`main.py` の `_multi_user_auth`）で、Cookie `sai_session` か個人のトークン `Authorization: Bearer sai_…` から今の利用者を `registry.current_user` に入れる。合言葉（`SAI_WEB_TOKEN`・`?token=`）は多人数モードでは受けない。未ログインは 401「ログインしてください」、初回のパスワード変更前は `/api/auth/` 以外の `/api/` が 403「パスワードを変えてください」。変更系（POST・PUT・PATCH・DELETE）で Cookie で入った要求は `Origin` と `Host` を照合し、違えば 403「別のサイトからの要求は受けません」。

| メソッド・パス | 中身 |
|---|---|
| `GET /api/auth/me` | 単独モード `{"mode":"single"}`／多人数 `{"mode":"multi","name","email","is_admin","must_change_password"}`／未ログイン 401 |
| `POST /api/auth/login` | body `{"email","password"}` → 200 `{"name","email","is_admin","must_change_password"}` と Cookie。違えば 401「メールアドレスかパスワードが違います」 |
| `POST /api/auth/logout` | `{"ok":true}`。Cookie を消す |
| `POST /api/auth/password` | body `{"current","new"}` → `{"ok":true}`。`must_change_password` を 0 にし、他のセッションを消す。今のパスワードが違えば 400 |
| `GET /api/me/tokens` | `{"tokens":[{"id","label","created_at","last_used_at"}]}`（値は返さない） |
| `POST /api/me/tokens` | body `{"label"}` → `{"id","label","token":"sai_…"}`（値はこの 1 回だけ）。label は 1〜60 文字 |
| `DELETE /api/me/tokens/{id}` | 自分のトークンだけ消せる。無ければ 404 |
| `GET /api/admin/users` | `{"users":[{"name","email","is_admin","disabled","must_change_password","created_at"}]}`（管理者だけ） |
| `POST /api/admin/users` | body `{"name","email","password","is_admin"}` → 作成（同名・同メール 409）。C4 のフォルダを作る |
| `PATCH /api/admin/users/{name}` | body `{"email"?,"is_admin"?,"disabled"?,"password"?}`。`password` を入れたら `must_change_password=1` にし、その人のセッションを消す。自分自身の `is_admin=false`・`disabled=true` は 400 |
| `GET /api/me/keys` | `{"keys":[{"name","set","last4"}]}`（値は返さない）。名前は `CLAUDE_CODE_OAUTH_TOKEN`・`OPENROUTER_API_KEY`・`SEMANTICSCHOLAR_API_KEY`・`OPENALEX_API_KEY`・`NCBI_API_KEY` の 5 つ |
| `PUT /api/me/keys` | body `{"<名前>": "<値>"}`（空文字は消す）→ `{"ok":true,"recreated":[…],"pending":[…]}`。値は印字できる ASCII で空白と改行を含まず 400 文字まで。知らない名前・改行入りの値は 400 |
| `GET /api/me/claude-login` | Claude のログインの状態（多人数モードだけ）→ `{"logged_in": bool, "auth_method": str, "token_set": bool}`。`token_set` は `CLAUDE_CODE_OAUTH_TOKEN` が設定済みか（値は返さない）。中身（メールなど）は返さない。`claude auth status` の終了コードと `authMethod` を見る |
| `POST /api/me/claude-login/start` | ログインを始める（多人数モードだけ）→ `{"url": str}`。補助のコンテナで `claude auth login` を pty で動かし、URL が出るまで最大 30 秒待つ。URL が出なければ 502「ログインの URL を取れませんでした: <最後の行>」。進行中の登録は 1 人に 1 つで、10 分で切れる。`docker run -t` の補助のコンテナ `sai-login-<user>` を使う |
| `POST /api/me/claude-login/code` | body `{"code": str}`（印字できる ASCII・空白なし・500 文字まで。違えば 400）。ログインを始めていなければ 409。端末へ送り `Login successful.` を最大 60 秒待つ。失敗は 502（理由の行。貼ったコードは伏せる）。成功したら `CLAUDE_CODE_OAUTH_TOKEN` を `secrets.env` から消し（環境変数の token がファイルのログインより優先されるため）、コンテナを作り直す → `{"ok": true, "recreated": […], "pending": […]}`（`pending` は実行中で作り直せなかった物。終わってから起こし直すと反映される） |
| `DELETE /api/me/claude-login` | 進行中のログインをやめる（補助のコンテナを止める）→ `{"ok": true}` |
| `GET /api/admin/containers` | `{"containers":[{"container","owner","project","state","status"}]}`（管理者だけ） |
| `POST /api/admin/containers/{container}/{start\|stop}` | コンテナを起動・停止（管理者だけ）。名前は `^sai-[a-z][a-z0-9]{1,15}-[a-z0-9][a-z0-9-]{0,29}$`、誤りは 400 |

- 鍵と Claude の長命トークンは `<U>/<user>/secrets.env`（0600）に `KEY=値` の行で保存し、コンテナには `--env-file` で渡す。値を変えると、その人の実行中でないコンテナを `docker rm -f` → `docker run` で作り直す（実行中のプロジェクトは `pending` に入れ、作業を壊さない）。
- プロジェクトの内部の合言葉（`SAI_WEB_INTERNAL_TOKEN`）はコンテナを作るときに `secrets.token_urlsafe(32)` で作り、SHA-256 だけを DB（`project_tokens`）に置く（同じプロジェクトの古い行は消す）。値は `sai_` で始まらない（個人のトークンと区別する）。
- 内部の合言葉を `Authorization: Bearer` で送ると、ミドルウェアが `project_tokens` を引いて持ち主を `registry.current_user` に入れ、`request.state.internal_project` にプロジェクト名を入れる（Cookie より先に見る）。内部の合言葉で使えるのは**内部の 6 本**（`POST /api/register`・`POST /api/questions`・`POST /api/questions/{qid}/resolved`・`GET /api/questions/{qid}`・`GET /api/questions/mine`・`POST /api/questions/{qid}/answer`）だけで、それ以外の住所は 403「内部専用の合言葉では使えません」。`/api/home`・`/api/questions/pending` は持ち主の全プロジェクトを返すので、内部の合言葉には開けない。`/api/questions/{qid}/answer` と `/api/questions/mine` は、合言葉のプロジェクトの質問に限る。
- 質問・登録・通知の購読は `owner` を持ち、多人数モードでは今の利用者の物だけを読む（`GET /api/questions/{qid}` は持ち主でなければ 404）。保存する関数は `registry.current_user` の名前を `owner` に入れる。見張り（`watch.py`）は利用者ごとに `registry.current_user` を入れてから見回る。単独モードは今までどおり（`owner` は空文字で、絞り込みをしない）。古い DB（`owner` の列が無い）は `init_db` が `ALTER TABLE` で列を足す。
- Usage の OpenRouter・DeepSeek の鍵は、多人数モードでは今の利用者の `secrets.env`（`containers.read_keys`）から読む（無ければ「キーが未設定です」）。単独モードは今までどおり環境変数から読む。

- 多人数モードでは `FastAPI(docs_url=None, redoc_url=None, openapi_url=None)` にする（`/docs`・`/openapi.json` を出さない）。起動時に合言葉のファイル（`<sai_home>/web/token`）は書かない。
- パスワードは `hashlib.scrypt`（`auth.hash_password`・`verify_password`）で保存し、Cookie とトークンは SHA-256 だけを DB（`web_sessions`・`api_tokens`）に置く。値そのものは保存しない。

### 4.11 sai-ts の入口（別のオリジン・`ts_gate.py`）

時系列（sai-ts）の画面は sai-web と**別のオリジン**（同じホスト名の別ポート）で出す。sai-web の Cookie と API は届かない。実装は `apps/web/src/sai_web/ts_gate.py`（sai-web と同じプロセスで、`127.0.0.1` の別の待ち受け。`SAI_WEB_TS_PORT`）。中継先はコンテナの中の sai-ts（`-p 127.0.0.1::8210`・C3）。

| 住所 | 中身 | 合言葉 |
|---|---|---|
| `GET /p/{project}/__gate/enter?ticket=<券>` | 券（60 秒・1 回だけ）を合言葉に替え、`303` で `/p/{project}/#token=<合言葉>` へ。券が無効・切れていれば 403 | 券 |
| `GET /p/{project}` | `307` で `/p/{project}/` へ | なし |
| `GET /p/{project}/<画面ファイル>`（`/p/{project}/` は `index.html`） | 画面の静的ファイル（ホストの `apps/ts/src/sai_ts/static`。パスの外は 404） | なし |
| `… /p/{project}/api/{rest}` | コンテナの sai-ts の `/api/{rest}` へ中継 | 利用者の合言葉（`Authorization: Bearer`。Cookie と元の `Authorization` は送らない） |
| `… /p/{project}/ext/{rest}` | コンテナの sai-ts の `/ext/{rest}` へ中継 | 同上 |
| それ以外 | 404 | |

- 合言葉は断片（`#`）に置くので、サーバに届かず、ログにも残らない。画面は `sessionStorage`（キー `ts_token`）に保存し、URL からは消す。
- 中継は利用者の合言葉を確かめてから、コンテナの sai-ts へ内部の合言葉（`SAI_TS_TOKEN`。起動のたびに作る）を付けて送る。利用者の合言葉は上流に送らない。
- 上流が応えない・合言葉を拒んだときは覚えを捨てて（`containers.ts_forget`）502「時系列のサーバにつながりません」。
- 券と合言葉はメモリに SHA-256 だけを置く。sai-web を起動し直すと合言葉は無効になる（開き直す）。

---

## 5. `sai-web` の命令（CLI から Web を使う）

Web のすべての HTTP API を、コマンド 1 本から使える（接続先 `SAI_WEB_URL`・合言葉 `SAI_WEB_TOKEN`、無ければコンテナの中の `SAI_WEB_INTERNAL_TOKEN`、それも無ければ `$SAI_HOME/web/token`。コンテナの中では `questions` は自分のプロジェクトの分だけ）。合言葉は `Authorization: Bearer` で送り、値は出力に出さない）。

| 命令 | 何 |
|---|---|
| `sai-web serve [--host <H>] [--port <P>]` | Web のサーバを起動する（既定 `127.0.0.1`・`SAI_WEB_PORT`） |
| `sai-web projects` / `home` | プロジェクトの一覧・ホームの集計 |
| `sai-web questions [--run <id>] [--project <名前>]` / `question <qid>` / `answer <qid> (--choice … \| --choices … \| --text …)` | 質問の一覧・全文・答え |
| `sai-web send <project> <run_id> <文>` | 実行中の run に文を送る |
| `sai-web usage` | 使用量 |
| `sai-web mlflow experiments\|runs <experiment_id>\|run <id>\|metric <id> <key>` | MLflow の読み取り（中継 API 経由） |
| `sai-web api <METHOD> <path> [--data <JSON>\|--data -]` | 上の API 一覧のどれでも呼ぶ |
| `sai-web user add <名前> --email <メール> [--admin]` | 利用者を作る（多人数モード。パスワードは標準入力の 1 行目、空なら `getpass` で聞く。同じ名前・メールは終了コード 4） |
| `sai-web user list --json` | 利用者の一覧（名前・メール・管理者・無効・初回変更待ち） |

終了コードは 3 つの CLI と同じ（0 成功・2 入力の誤り・3 見つからない・4 状態の衝突・1 その他）。HTTP の状態からの直し方は ARCHITECTURE.md の §4.3。

---

## 6. 画面と URL（`#` ルート）

| URL | 画面 | タブ | タブバー |
|---|---|---|---|
| `#/home` | Home（アプリを開いたときの入口） | Home | 表示 |
| `#/projects` | プロジェクト一覧 | Projects | 表示 |
| `#/project/<name>` | Dashboard | Projects | 表示 |
| `#/project/<name>/workflow` | Workflow 盤面（Kanban の帯つき） | Projects | 表示 |
| `#/project/<name>/list` | List（run 未選択） | Projects | 表示 |
| `#/project/<name>/list/<run_id>/<view>` | List（`view` は log / stats / history / task / message / report / waggle。report の帯は「報告」と出る） | Projects | 表示 |
| `#/project/<name>/defs` | 設定の一覧（block / template / settings） | Projects | 表示 |
| `#/project/<name>/defs/<kind>/<scope>/<name>` | 設定の編集（`kind` = block / template / settings、`scope` = project / global） | Projects | 表示 |
| `#/project/<name>/commands` | コマンド（3 つの CLI の命令を入力欄から実行する） | Projects | 表示 |
| `#/project/<name>/sessions` | セッション検索（会話の履歴の検索） | Projects | 表示 |
| `#/` | Chats 日付一覧 | Chats | 表示 |
| `#/day/<YYYY-MM-DD>` | Chats 実行一覧 | Chats | 表示 |
| `#/run/<project>/<run_id>` | メッセージ | Chats | 隠す |
| `#/run/<project>/<run_id>/stats` | 統計 | Chats | 隠す |
| `#/papers/dashboard` | Dashboard | Papers | 表示 |
| `#/papers/library` | Library | Papers | 表示 |
| `#/papers/import` | Import（BibTeX の読み込みと一覧） | Papers | 表示 |
| `#/papers/radio` | Radio 一覧 | Papers | 表示 |
| `#/papers/discover` | 旧 Discover（Library へ移すだけ） | Papers | 表示 |
| `#/paper/<name>/<id>` | 論文情報 | Papers | 表示 |
| `#/paper/<name>/<id>/text` | 本文 | Papers | 表示 |
| `#/paper/<name>/<id>/cites` | 引用一覧（絞り込み・詳細・引用の抜き出し・手動取り込み） | Papers | 表示 |
| `#/papers/log/<name>/<log_id>` / `…/<position>` | 過去の検索結果・未ダウンロード論文の情報 | Papers | 表示 |
| `#/papers/find/<name>/<log_id>` | 過去の内部検索の結果 | Papers | 表示 |
| `#/papers/library/find/<log_id>` | 過去の内部検索の結果（今選んでいるプロジェクト） | Papers | 表示 |
| `#/radio/<radio_id>` | 再生画面（質問チャット付き） | Papers | 隠す |
| `#/research/tree` / `#/research/tree/<ID>` | research-tree（`<ID>` は Q・A・F・E・S の Node） | Research | 表示 |
| `#/research/read/<run_id>` | READ_CARD（1 本の論文を読んで F を書く） | Research | 表示 |
| `#/research/experiments` | 実験（MLflow）の一覧 | Research | 表示 |
| `#/research/experiments/e/<experiment_id\|all>` | run の表（`?sai_run=<branch>`・`?sprout=<n>`・`?study=<s>`・`?status=<S>` を付けられる。run は sprout → branch → run にまとめる） | Research | 表示 |
| `#/research/experiments/run/<run_id>` | run の詳細（設定・指標・タグ・成果物） | Research | 表示 |
| `#/research/experiments/compare/<run_id>,<run_id>,…` | run の比較（2〜20 件） | Research | 表示 |
| `#/usage` | 使用量（`#/usage/<provider>/<account>` で内訳） | Usage | 表示 |
| `#/login` | ログイン（多人数モードだけ。大学のメールアドレスとパスワード） | なし | 隠す |
| `#/password` | パスワードの変更（初回ログインのときと、「パスワードを変える」から。多人数モードだけ） | なし | 隠す |
| `#/account` | アカウント（自分の情報と sai-prep のトークン。多人数モードだけ） | なし | 隠す |
| `#/admin` | 管理（利用者の一覧と作成・変更。管理者だけ。多人数モードだけ） | なし | 隠す |

- **古い住所は Research へ送る**（`scitree.js` の登録が `location.replace` で送り直す）:
  `#/papers/tree[/<ID>]` → `#/research/tree[/<ID>]`、`#/papers/read/<run_id>` → `#/research/read/<run_id>`、`#/mlflow…` → `#/research/experiments…`、`#/papers/canvas`・`#/canvas/…` → `#/research/tree`。
- 古い名前の無い住所 `#/paper/<id>` / `#/paper/<id>/text` / `#/papers/log/<log_id>` / `#/papers/log/<log_id>/<position>` は、今選んでいるプロジェクトの同じ画面として開く。
- Papers の上部の切り替えは Dashboard・Library・Import・Radio。Research の上部の切り替えは research-tree・Experiments。
- research-tree の画面は、左に一覧（Q・A・F・E・S の Node）、真ん中に research-tree の図、PC 幅（900px 以上）は右の板（約 420px・✕ で閉じる）、iPhone 幅は下から sheet。板には、その Node の操作の釦と Next Action がある。許可が要る操作は質問ツールの「許可する」で行い、画面で採否を決める操作は無い。
- 住所に名前がある画面を開いたら、Papers タブの「選んでいるプロジェクト」もその名前に切り替わる。選んだプロジェクトは `localStorage` に覚える。Experiments の「選んでいるプロジェクト」は別に覚える（「すべてのプロジェクト」は `*`。`POST /api/mlflow/runs/search` には `project` を送らない）。
- List のサイドバーの一番上の「実行［＋］［↻］」の行で、**＋** は task.md 作成フォーム、**↻** はツリーの読み直し。書きかけは `localStorage`（プロジェクトごと）。
- Projects の 4 画面（Dashboard / Workflow / List / 設定）の見出しは同じ形（`‹`・プロジェクト名・`Dashboard|Workflow|List|設定`）。`‹` は 4 画面ともプロジェクト一覧（`#/projects`）へ戻る。
- 画面ファイル: `index.html`（枠）＋各画面の JS ＋ `markdown.js` ＋ `app.js`。**タブごとのファイルは `app.js` より前**に読み込む。新しい画面は `window.SaiRoutes` に登録する（`app.js` の分岐は増やさない）。

---

## 7. CLI の失敗の読み替え

3 つの CLI の失敗は、`cli.py` の 1 か所で HTTP の状態に直す（ARCHITECTURE.md の §4.2 と同じ）。

| CLI の結果 | HTTP |
|---|---|
| 終了コード 0 で標準出力が JSON | 200 |
| 終了コード 0 で標準出力が JSON でない | 502「…の応答を読めませんでした」 |
| 終了コード 2 | 400 |
| 終了コード 3 | 404 |
| 終了コード 4 | 409 |
| 終了コード 1 とそのほか | 502 |
| CLI が起動できない | 503「…が見つかりません（設定の場所を確認してください）」 |
| 時間切れ | 504「…が時間内に応答しませんでした」 |

`detail` は標準エラーの空でない最後の行（`sai-progress` の行を除く）。時間の上限は既定 20 秒（検索・内部検索 120 秒、訳 90 秒、音声化 2000 秒、「コマンド」画面の実行 120 秒など個別）。

---

## 8. 設定（環境変数）

| 変数 | 既定値 | 用途 |
|---|---|---|
| `SAI_WEB_PORT` | `8200` | 待ち受けポート |
| `SAI_WEB_TOKEN` | （空＝認証なし・開発用） | 合言葉（単独モードだけ） |
| `SAI_WEB_USERS_ROOT` | （空＝単独モード） | 多人数モードの置き場（絶対パス。C4） |
| `SAI_WEB_COOKIE_SECURE` | `1` | `0` で Cookie の `Secure` を外す（手元の http の確認用） |
| `SAI_WEB_URL` | `http://127.0.0.1:8200` | `sai-web` の命令とエージェントの道具がつなぐ先 |
| `SAI_WEB_RUNS_DIR` | `$SAI_HOME/web/runs` | Chats が読む、書き出しフォルダ（多人数モードでは利用者のプロジェクトごとの `<project_home>/.sai/web/runs` を全部読む） |
| `SAI_WEB_OPENROUTER_ACCOUNTS` | `main=OPENROUTER_API_KEY` | 使用量の OpenRouter の「名前=キーの入った環境変数名」 |
| `SAI_WEB_NO_WATCH` | （空） | `1` で状態の見張りを止める |
| `SAI_WEB_HOOKS_DIR` | `apps/web/hooks` | `sai-link init` が cc のフックを書くときに探す置き場 |
| `SAI_LINK_BIN` / `SAI_PAPER_BIN` / `SAI_SCI_BIN` | `sai-link` / `sai-paper` / `sai-sci` | Web が呼ぶ CLI。PATH に無ければ絶対パス |
| `SAI_RADIO_URL` / `SAI_RADIO_TOKEN` | `http://127.0.0.1:8201` / （空） | Radio の中継先と合言葉 |
| `SAI_WEB_TS_PORT` | （空＝ts 入口を起こさない） | sai-ts の入口が `127.0.0.1` で待つ番号（4.11）。多人数モード（`SAI_WEB_USERS_ROOT`）でだけ使える。単独モードで設定すると `sai-web serve` は終了コード 2。サーバは `8210`、Mac の確認環境（`dev.sh`）は `8298` |
| `SAI_WEB_TS_URL` | （空＝「時系列」は 503） | 利用者のブラウザから見た ts 入口の住所（末尾の `/` なし）。サーバは `https://<機械>.<tailnet>.ts.net:8443`、`dev.sh` は `http://127.0.0.1:8298` |
| `SAI_MLFLOW_URL` / `SAI_MLFLOW_USER` / `SAI_MLFLOW_PASSWORD` | `http://127.0.0.1:5001` / （空） / （空） | MLflow の中継先と Basic 認証 |
| `CLAUDE_SESSIONS_DIR` / `PI_SESSIONS_DIR` | 各既定 | 会話の記録置き場 |
| `SAI_HOME` | `~/.sai` | 利用者全体の置き場（Rust 側も同じ） |

Rust 側にも同じ `SAI_HOME`・`SAI_PROJECT_ROOT`・`SAI_ORIGIN` が効く。**PATH が無いと CLI が見つからず 503 になる。**

---

## 9. 注意点

| 注意点 | 内容 |
|---|---|
| **CLI は PATH に要る** | `sai-link`・`sai-paper`・`sai-sci` は `target/release/` にできる。PATH に足すか、`SAI_LINK_BIN` などに絶対パスを書く。忘れると Projects が全部 503 |
| **release ビルド** | Web が呼ぶのは release の実行ファイル。`crates/` を直したら `cargo build --release` で作り直さないと古いまま |
| **置き場は `SAI_HOME`** | 登録簿・書き出し・共通定義・`paper.env` はここ。`SAI_HOME` を一時フォルダに差し替えれば、テストを隔離できる |
| **`SAI_PROJECT_ROOT`** | sai-link は `SAI_PROJECT_ROOT` を最優先で DB 解決に使う。Web は呼び出しのたびに必ずプロジェクトの root を渡す（`control._phone_env`・`papers._paper_cli`）。これを消すと、起動元の環境によっては**全プロジェクトが同じ DB を開いてしまう** |
| **Radio の論文 ID は英数字** | ID は「論文 ID（`<root のフォルダ名>-<番号>`）」と「`apps/radio` が作った音声（12 桁の 16 進・`demo001` 等）」の両方がある。数字だけと当て込むと壊れる。判定は `radio.js` の `isRadioPaperId` |
| **長すぎる論文は音声化できない** | `apps/radio` が論文全文を LLM に 1 回で渡す（`llm.py`）ため、約 13 万トークンを超える論文は文脈上限で失敗する（`apps/radio` 側の制限） |
| **FastAPI の登録順** | 固定パスを `/api/projects/{name}/papers/{paper_id:int}` より前に登録しないと 404 / 400 になる。順番を変えたら curl で一覧を確かめる |
| **`sw.js` の `CACHE`** | 版を上げないと iPhone に古い画面が残る。画面ファイルを足したら `ASSETS` にも足す。音声の倉庫（`sw.js` の `RADIO_CACHE`）は別物なので消さない |
| **常駐・実行を動かす操作** | 常駐の起動停止・カードの実行・音声化の依頼は、実際にエージェントや生成を動かす。確認は `--help` と不正値で行う |
| **旧構成の 8100 番・8000 番は触らない** | 旧構成の Web と Radio が別のプロセスで動いていることがある。確認で再起動しない |
