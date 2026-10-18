# agent-link-web の API と画面の住所

**この文書は、Web の HTTP の住所と画面の住所の契約の正である。** 一覧は `apps/web/tests/test_surface.py` の一覧と一致させる。この文書とコードが食い違ったら、コードに合わせて直す。
CLI と Web の間の契約（終了コード・JSON の項目・終了コードから HTTP への読み替え・`AGENT_LINK_HOME` の規則）は [`for-developers.md`](./for-developers.md)、環境変数の表は [`../README.md`](../README.md)、全体の考え方は [`../ARCHITECTURE.md`](../ARCHITECTURE.md)、言葉の意味は [`../CONCEPT.md`](../CONCEPT.md)。

この文書の API の数は、`main.py` の `@app.get`・`post`・`put`・`delete`・`api_route` を数えた **53 本**（`/manifest.json` と拡張の 5 本を含む。2026-10-18 時点）。これに FastAPI の既定の `GET /openapi.json`・`GET /docs`・`GET /docs/oauth2-redirect`・`GET /redoc` と、静的ファイルの `app.mount("/")` が加わる（`apps/web/tests/test_surface.py` の一覧と同じ）。

---

## 1. 全体のつながり

```
iPhone / PC のブラウザ
   │  HTTPS  tailscale serve :4446（iPhone だけ。自分 1 人の tailnet のときだけ）
   ▼
127.0.0.1:8400  agent-link-web（FastAPI・窓口だけ）
   ├─ Chats   : $AGENT_LINK_HOME/web/runs/<project>/<run_id>.json を読む   ← `agent-link run export` が書き出す
   └─ Projects: 登録簿 $AGENT_LINK_HOME/registry.json を直接読む ／ `agent-link …` を呼ぶ
```

| タブ / 画面 | 使う物 |
|---|---|
| Chats（日付・実行・メッセージ・統計） | `agent-link` が書く書き出し JSON（`$AGENT_LINK_HOME/web/runs/`）と会話の記録（.jsonl） |
| Projects（一覧・Dashboard・List・カードの作成と実行・常駐・task / review の保存） | `agent-link project summary` / `project dashboard` / `daemon start\|stop` / `run tree` / `run log\|stats\|history` / `run md` / `doc list\|read\|search` / `card options\|docs\|create\|run\|schedule` / `run advance\|finalize\|rm`。一覧の `name` / `root` / `kind` / `root_exists` / `db_exists` / `daemon_alive` は登録簿（`registry.py`）から作る |
| Projects → Workflow | `agent-link workflow`（Workflow の配置は `agent-link` が計算する） |
| Projects → 設定（block / template / settings） | `agent-link def list\|show\|write\|delete`（`scope` で、そのプロジェクトと `$AGENT_LINK_HOME` の両方） |
| Projects → セッション検索 | `agent-link session search` |
| Home（質問・プロジェクト・todo） | 上の住所を 1 本の住所（`GET /api/home`）にまとめる。`ThreadPoolExecutor` で並べる |
| Home の全プロジェクトの合計（今週 / 今月 / 全期間） | 各プロジェクトの `agent-link project dashboard --period` を並べて足し合わせる（`GET /api/home/overview`） |
| Home の Claude のカード | `usage.py`（cc の枠と費用。`GET /api/usage`）。cc の枠の値は `agent-link cc-statusline` が書いた `cc-rate-limits.json` を読む。CLI は呼ばない |

---

## 2. 大原則

1. **Web は窓口だけ。** 処理とデータの保管は実行エンジンがする。
   - 実行エンジンの DB（`agent-link.db`）を**直接開かない**。DB の形を変えたときに Web が壊れ、原因が追いにくくなるため。
     （例外は 1 つだけ: Chats は `agent-link` が書き出す `$AGENT_LINK_HOME/web/runs/*.json` を読む。これは「公開された書き出し」であり DB ではない。）
   - 機能は **CLI（`--json`）か HTTP API** だけを使う。
2. **既にある物を使う。** 新しい依存ライブラリは入れない（FastAPI・標準ライブラリ・素の JS のみ）。
3. **画面は素の JavaScript**（フレームワーク・ビルド無し）。グラフは SVG を手で描く。
4. **合言葉は無い。** 入口は次の 3 つで守る。エラーは英語（`{"detail": "<英語の 1 文>"}`）で画面にそのまま出す。
   - **Host の検査**（`main.py` の `_host_guard`・全部の住所）: 許可するホストは `127.0.0.1`・`localhost`・`::1`・末尾 `.ts.net`、それに `AGENT_LINK_WEB_URL` のホスト名（空なら無し）。ポートは外し、大文字小文字は区別しない。許可外は 403 `{"detail": "Host not allowed."}`（DNS rebinding を止める）。
   - **Origin の検査**（同じ関数）: GET・HEAD・OPTIONS 以外で `Origin` が付いていて、そのホストが許可外（`null` を含む）なら 403 `{"detail": "Cross-origin request refused."}`。`Origin` が無い要求（フック・`agent-link-web api`・curl）は通す。独自ヘッダは必須にしない。
   - **内部用の住所の `require_internal`**（Mac 自身だけ）。
   - **`serve --host`**（`webcli.py`）: 待ち受けの住所が `127.0.0.1`・`localhost`・`::1` 以外なら、標準エラーに `Refusing to listen on <host>: the API has no password. Pass --allow-remote to do it anyway.` と出して終了コード 2。`--allow-remote` を付けたときだけ通す。
5. **外から来る値は CLI を呼ぶ前に検査する。** 通らない値では CLI を 1 度も呼ばない（400）。ファイルのパスは受け取らない。

---

## 3. 置き場（`AGENT_LINK_HOME`）

| 置き場 | 場所 |
|---|---|
| 登録簿 | `$AGENT_LINK_HOME/registry.json` |
| 書き出し（Chats が読む） | `$AGENT_LINK_HOME/web/runs/` |
| 共通の block・template・settings | `$AGENT_LINK_HOME/blocks/`・`template/`・`settings.json` |
| 質問の保管・通知の鍵・ログ | `$AGENT_LINK_HOME/web/agent_link_web.db`・`vapid.json`・`agent-link-web.log`（README の `agent-link-web-restart` で起こしたときのログの出し先） |
| Claude（cc）の費用と枠 | `$AGENT_LINK_HOME/cc-cost/<session_id>.json`・`cc-rate-limits.json`（どちらも `agent-link cc-statusline` が書き、`GET /api/usage` が読む） |
| **プロジェクト内の `.agent-link/`** | `<root>/.agent-link/`（`engine/agent-link.db`・`blocks/`・`template/`・`settings.json`・`outputs/`）。`AGENT_LINK_HOME` とは別 |

- `AGENT_LINK_HOME` の規則（Rust と Python の 2 か所）は [`for-developers.md`](./for-developers.md) の「`AGENT_LINK_HOME`（利用者全体の置き場）の規則」。`registry.json` の形は同 §3.8。

---

## 4. API 一覧

合言葉は無い。内部用の 4 本（`POST /api/register`・`POST /api/questions`・`POST /api/questions/{qid}/resolved`・`GET /api/questions/{qid}`）は `require_internal` で Mac 自身（127.0.0.1 / ::1）だけ受け、ほかから来ると 403「Internal use only.」。エラーは `{"detail": "<英語>"}`。
「呼ぶ CLI」の列は、cwd = プロジェクトの root、`AGENT_LINK_PROJECT_ROOT` = 同じ root で呼ぶ命令。終了コードから HTTP の状態への読み替えは [§7](#7-cli-の失敗の読み替え) のリンク先の 1 か所。

### 4.1 プロジェクト（一覧・Dashboard・常駐）

| メソッド・パス | 中身 | 呼ぶ CLI |
|---|---|---|
| `GET /api/projects` | プロジェクト一覧（`{"projects": [...]}`）。`running` と `last_executed_at` は 5 秒だけ覚える | `agent-link project summary` |
| `GET /api/projects/{name}/dashboard?period=week\|month\|all` | Dashboard（既定 week）。開くとき、同じプロジェクトの書き出しを 60 秒に 1 回まで `agent-link run export` で更新する（書き出しは常駐の daemon も同じ周期で行う：`crates/agent-link-engine/src/daemon.rs` の `export_runs`） | `agent-link project dashboard --period <期間>`（前に `agent-link run export`） |
| `POST /api/projects/{name}/daemon` `{"action":"start"\|"stop"}` | 常駐の起動・停止 | `agent-link daemon start\|stop`（文字出力） |

### 4.2 ホーム

| メソッド・パス | 中身 | 呼ぶ CLI |
|---|---|---|
| `GET /api/home` | 質問・プロジェクト（review の run と todo の行）・`errors` | `agent-link run tree` |
| `GET /api/home/overview?period=week\|month\|all` | 全プロジェクトをまたいだ合計（`period` 省略時は `week`・それ以外は 400） | `agent-link project dashboard --period <期間>` |

- 質問は `store.list_open_questions` から作る。`project` / `run_id` は `runs.load_runs` の中で `run_session_ids` に `session_id` を含む実行を探して付ける（無ければ `null`）。
- プロジェクトの `review` / `todo` は `run tree` の `review` / `todo` フォルダを数える。
- `todo` は件数、`todo_runs` は同じフォルダの行（`[{run_id, label}]`・ツリーの順）。todo の行には `card`（`creation_date`・`block`・`model`・`description`）が付き、実行予約した行には `scheduled_at`（`YYYY-MM-DD HH:MM`）が付く。`review` はホームには出さない（API には残す）。
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
  "recent": [{"project": "agent-link", "run_id": 42, "parent_run_id": 1, "block": "worker",
              "status": "completed", "started_at": "2026-09-27 10:28:21",
              "description": "…", "linkable": true}],
  "errors": []
}
```

- 対象は `/api/home` と同じプロジェクト（`root_exists` と `db_exists` が真）。`linkable` が偽の `recent` は会話画面が開けないので List 側の画面へ。
- `summary` は各プロジェクトの同じ名前の数を足し、`success_rate` は足したあとの `completed / executions`（0 件なら `0.0`）で求め直す。`daily` は日付ごとに足す。`recent` は `started_at` の新しい順（同じなら `run_id` の大きい順）で最大 20 件。期間も 10 秒だけ覚える。

### 4.3 Workflow と設定（`agent-link workflow` / `def` 経由）

| メソッド・パス | 中身 | 呼ぶ CLI |
|---|---|---|
| `GET /api/projects/{name}/workflow[?subtasks=1]` | Workflow 盤面（札・線・日付の区切り線・Kanban の 3 レーン）。`?subtasks=1` で子の run（`task create` で作った run）も載せる | `agent-link workflow [--subtasks]` |
| `GET /api/projects/{name}/defs` | 定義の一覧 | `agent-link def list` |
| `GET /api/projects/{name}/defs/{kind}/{scope}/{def_name}` | 定義 1 件の中身 | `agent-link def show --kind <kind> --scope <scope> --name <名前>` |
| `PUT /api/projects/{name}/defs/{kind}/{scope}/{def_name}` | 定義の保存（body `{"text": …, "create": false}`。本文は標準入力。`create` が真なら `--create`） | `agent-link def write --kind … --scope … --name … [--create]` |
| `DELETE /api/projects/{name}/defs/{kind}/{scope}/{def_name}` | 定義の削除（settings は 400「settings cannot be deleted.」） | `agent-link def delete --kind … --scope … --name …` |

- `kind` = `block` / `template` / `settings`、`scope` = `project` / `global`。`def_name` は block / template が `^[A-Za-z0-9][A-Za-z0-9_-]{0,63}$`、settings が `settings` だけ。違えば 400。
- `scope=global` の置き場は `$AGENT_LINK_HOME`。
- 定義の編集は **DB が無いプロジェクトでもできる**（root が無いときだけ 404「Project folder not found.」）。`workflow` は実行の記録が要るので 404「This project has no run records yet.」。
- 本文は 256KB まで。検査は CLI を呼ぶ**前**に行う。`write --create` で同じ名前があるときは、CLI が終了コード 4 で返し、Web は 409「A definition with the same name already exists.」にする。
- 住所の `{def_name}` は FastAPI 上の変数名も同じ。

### 4.4 List（`agent-link` 経由）

| メソッド・パス | 中身 | 呼ぶ CLI |
|---|---|---|
| `GET /api/projects/{name}/tree` | 引き出しツリー（todo / running / review / 日付 / 親 run / 子 run の入れ子。run は `children` を持てる。`lane` は `todo`・`running`・`review`・`workflow`）。各 run ノードは `executer`（`human` / `agent`）を持つ。`agent` の todo には画面が ▶ 実行を出さない | `agent-link run tree` |
| `GET /api/projects/{name}/search?q=…` | 文書（task / message / waggle）の本文検索。`q`（空白区切りで AND・200 文字まで）に加え `category`（カンマ区切り・空で全部）・`since` / `until`（`YYYY-MM-DD`）・`block`（完全一致）・`from_id` / `to_id`（run_id の範囲）・`limit`（1〜100・既定 50）。返り値は `{items: [{run_id, category, file, token?, block, date, snippet}], truncated}` | `agent-link doc search --word … --category … --limit …` |
| `GET /api/projects/{name}/sessions/search` | 会話（セッション履歴）の検索。クエリは `word`・`id`・`block`・`date`（`YYYY/MM/DD` か `YYYY/MM/DD-YYYY/MM/DD`）・`status`・`type`。先頭 200 件を `{"items", "truncated"}` で返す | `agent-link session search --id … --block … --date … --status … --type … -w … --json` |
| `GET /api/projects/{name}/runs/{run_id:int}/view/{log\|stats\|history}` | run の中身 | `agent-link run log\|stats\|history <run_id>` |
| `GET /api/projects/{name}/runs/{run_id:int}/history/calls/{call_id}` | History の 1 呼び出しの全文（`input`・`result`（20000 字まで）・`truncated`）。`call_id` は `^[A-Za-z0-9_-]{1,80}$`（通らなければ 400）。見つからなければ 404 | `agent-link run history <run_id> --call <call_id> --json` |
| `GET /api/projects/{name}/runs/{run_id:int}/md/{task\|message\|report\|system}` | md 本文（「まだありません」は 404）。`report` は run の報告（`finalize-report.md`）、`system` は固定部分（`system-prompt.md`） | `agent-link run md <run_id> --kind <種類>` |
| `PUT /api/projects/{name}/runs/{run_id:int}/md/{task}` | md の保存（body `{"text"}`。task は未実行の todo カードだけで、それ以外は 404） | `agent-link run md <run_id> --kind task --write`（標準入力） |
| `GET /api/projects/{name}/runs/{run_id:int}/waggle` | 成果物の一覧 | `agent-link doc list --run <run_id> --json` |
| `GET /api/projects/{name}/runs/{run_id:int}/waggle/{token}` | 成果物 1 件の本文 | `agent-link doc read <トークン> --json` |
| `POST /api/projects/{name}/runs/{run_id:int}/start` | 新形式 todo の ▶ 実行（未実行カードが無ければ 404。`executer: agent` のカードと、常駐が止まっているときは 400「This card is started by an agent as a subtask run (executer: agent). …」「The daemon is stopped. Start it first.」） | `agent-link card run <run_id>` |
| `POST /api/projects/{name}/runs/{run_id:int}/advance` | review → Workflow（日付フォルダ）への移動（その run が無ければ 404、review 以外は 400「Only a run in review can be moved.」） | `agent-link run advance <run_id>` |
| `POST /api/projects/{name}/runs/{run_id:int}/schedule` | todo の実行予定時刻（`{"at": "YYYY-MM-DD HH:MM"}`・空で取り消し）。時刻になると常駐が `card run` と同じ流れで実行する | `agent-link card schedule <run_id> [--at "…"]` |
| `DELETE /api/projects/{name}/runs/{run_id:int}` | List の項目を消す（子の実行と run フォルダも消える。実行中は 409） | `agent-link run rm <run_id> --yes` |
| `GET /api/projects/{name}/card/docs` | 最近の文書（task_text に貼る `ref` 付き） | `agent-link card docs` |
| `GET /api/projects/{name}/card/options` | 作成フォームの選択肢 | `agent-link card options` |
| `POST /api/projects/{name}/card` | カード（task.md）の作成。body は `block`・`model`・`thinking`・`description`・`task_text`（と `done_ids`。`done_ids` は受け取るが CLI は無視する。done はやめた）。Web は `executer` を渡さない（CLI の `card create` は `human` / `agent` を受ける） | `agent-link card create`（標準入力に JSON） |

- 外から来る値は Web 自身が CLI を呼ぶ**前**に検査する（`run_id` は正の整数、`view` / `kind` は一覧の値だけ、`token` は英数字と `-_.:`・長さ 128 まで）→ 400。
- 型の誤り（整数でない `run_id`・`limit` など）は FastAPI が 422 で返す。`detail` は `Invalid request: <場所>: <英語の 1 文>`（`main.py` の `validation_error_handler`）。
- 作成フォームも同じ（`block` は英数字と `-_.:`、`task_text` は 100KB まで、`thinking` は一覧の値（`none`・`low`・`medium`・`high`・`xhigh`・`max`）、`model` は一覧の値（`opus`・`sonnet`・`haiku`・空）、`description` は 200 文字までの 1 行、`done_ids` は 1 以上の整数の配列）→ 400。廃止した `runner`・`provider`・`tag`・`round`・`global_round` は、送られても検査せず、CLI にも渡さない（断らない）。
- CLI の失敗は §7 の読み替え。404 は 2 種類で、md が無いとき（終了コード 3・stderr「Not available yet.」）と未実行カードが無いとき（終了コード 3）。
- 呼び出しは cwd = プロジェクトの root、かつ `AGENT_LINK_PROJECT_ROOT` = 同じ root（常駐を起動した環境変数に引きずられない）。
- 住所の `{token}` は FastAPI 上の変数名だけ `{waggle_token}`（URL の形は同じ）。

### 4.5 会話・質問・通知・Claude の使用量

| メソッド・パス | 中身 |
|---|---|
| `GET /api/health` | 生存確認。`{"ok": true, "server": <起動時の git の短い commit。読めなければ "?">, "started_at": "MM/DD HH:MM"}`（`main.py` の `api_health`。`scripts/agent-link-setup doctor` が `ok` を見る） |
| `GET /api/days` / `GET /api/days/{date}` | 会話の日付一覧・その日の実行 |
| `GET /api/runs/{project}/{run_id}` / `…/items` | 実行 1 件・会話の中身。`…/items` は `{items, next_cursor, live, partial, error}` を返す（`live` は `working` / `idle` / `blocked` / `null`、`partial` は生成中の文・`error` は直前のターンの失敗の理由。headless の実行は `state.json` から読む） |
| `POST /api/runs/{project}/{run_id}/send` | 実行中の run に文を送る（`agent-link run send <run_id> --json`・本文は標準入力）。質問が開いているときは tmux に送らず、その質問を取り下げて文を答えの代わりに渡す（応答 `{"run_id", "status": "withdrawn", "question_ids": […]}`）。終了・質問待ちは 409、無ければ 404。CLI が `"status": "unconfirmed"`（文は送ったが working にならなかった）を返したときは、そのまま返す（画面は入力欄を消して注意を出す） |
| `POST /api/runs/{project}/{run_id}/interrupt` | 実行中の run を中断する（`agent-link run interrupt <run_id> --json`）。質問が開いているときは esc を送らず、質問を文なしで取り下げる（応答 `{"run_id", "status": "interrupted", "agent_status": "blocked"}`）。無い実行は CLI を呼ばず 404 |
| `POST /api/runs/{project}/{run_id}/finalize` | メッセージ画面の ■ 終了（その run が無ければ 404、実行中でない・子ブロック・常駐停止は 502）。呼ぶ CLI は `agent-link run finalize <run_id>` |
| `POST /api/questions/{qid}/withdraw` | 未回答の質問を取り下げる（本文 `{"text"?}`・8000 文字まで。質問カードの「質問を止める」は文なし）。応答 `{"ok": true}`。締め切られた質問は 409、無ければ 404 |
| `POST /api/register` `/api/questions` | 登録（**内部専用**。Mac 自身（127.0.0.1 / ::1）だけ。cc のフックが呼ぶ） |
| `POST /api/questions/{qid}/resolved` | 質問が端末側で答えられたことの通知（**内部専用**） |
| `GET /api/questions/{qid}` | 質問 1 件（**内部専用**。cc のフックが答えを取りに来る）。JSON は `answered_by`（答えた人。端末で答えたときは `terminal`）を持つ。`state` は `open`・`answered`・`resolved_elsewhere`・`expired`・`withdrawn`（取り下げ。`answer` は `{"text": "<送った文。空もある>"}`） |
| `GET /api/questions/pending` / `GET /api/questions?session_id=` | 答え待ちの質問 |
| `POST /api/questions/{qid}/answer` | 答える（記録するだけ。cc のフックが `GET /api/questions/{qid}` で取りに来る）。本文は `{"answers": [{choice_index \| choice_indices \| text}, …]}`。番号は **0 から**数え、範囲外（単一選択に 2 つ以上も）は 400（質問は `open` のまま） |
| `GET /api/push/key` / `POST /api/push/subscribe` | プッシュ通知の鍵と購読。購読は `endpoint` が `https://` で 2048 字以内、ホストが `fcm.googleapis.com`・`*.push.apple.com`・`*.push.services.mozilla.com`・`*.notify.windows.com` のどれかで、`keys` に文字列の `p256dh` と `auth` が要る（通らなければ 400 `Invalid push subscription.`）。購読は 20 件まで（同じ endpoint の上書きは数えない。超えると 400 `Too many push subscriptions.`）。検査は保存の前に行う |
| `GET /api/usage` | Claude（cc）の使用量。`{"cc": {"limits": <5 時間枠・7 日枠または null>, "cost": {"today", "month", "sessions"}}, "saved_at": <epoch 秒>}` |
| `GET /manifest.json` | PWA の設定（`name`・`short_name` は `agent-link`） |

### 4.6 拡張（`agent-link ext` 経由・U06 と U07）

| メソッド・パス | 中身 | 呼ぶ CLI |
|---|---|---|
| `GET /api/ext` | 拡張の一覧（`{"extensions": [{"name", "title", "icon", "ui", "service", "ready", "web", "enabled_in"}]}`）。10 秒覚える | `agent-link ext list --json` |
| `GET /ext/{name}/{path}` | 拡張の `ui/` の中のファイル（画面の JS・CSS・アイコン）。`ui` を持つ拡張だけ。`..`・空の区切り・先頭の `/` や `\` を含む道、`ui/` の外へ出る道は 404 | —（ファイルを返す） |
| `POST /api/projects/{name}/ext/{ext_name}/cli` | 拡張の CLI を呼ぶ（body `{"args": [...]}`。そのプロジェクトで有効で、`args[0]` が `cli.web` にあること。通らなければ 400・403・404） | `agent-link ext run <ext_name> -- <args…>` |
| `GET /api/ext/{ext_name}/svc/{path}` / `POST /api/ext/{ext_name}/svc/{path}` | 拡張の service へ中継する（宛先は `service.url_env` の値）。POST は `service.post` にある道だけ（無ければ 403）。宛先が未設定なら 404 | —（HTTP で中継） |
| `GET /` / `GET /index.html` | 画面の HTML。拡張の目印（`<!-- ext:styles -->`・`<!-- ext:tabs -->`・`<!-- ext:scripts -->`）を、画面に出す拡張の物に差し替える（拡張が 0 個なら目印を消すだけ） | — |

- 拡張の作者向けの約束（`AgentLinkExt.register`・`ctx`・住所）の正は [`crates/agent-link-engine/assets/template/ext-contract.md`](../crates/agent-link-engine/assets/template/ext-contract.md)。この文書には住所だけを書く。
- 画面の JS は `static/ext.js`（`AgentLinkExt.register` で `window.SaiRoutes` に登録する）。拡張のタブは `data-tab="ext-<name>"`・`data-first="#/ext/<name>"`。

> **登録順の決まり**: FastAPI は登録した順に照合する。固定の住所は、変数の住所（`{run_id:int}` など）より**前**に登録する。静的ファイルの `app.mount("/")` は最後に置く。

---

## 5. `agent-link-web` の命令（CLI から Web を使う）

Web のすべての HTTP API を、コマンド 1 本から使える（接続先 `AGENT_LINK_WEB_URL`。合言葉は無い）。

| 命令 | 何 |
|---|---|
| `agent-link-web serve [--host <H>] [--port <P>]` | Web のサーバを起動する（既定 `127.0.0.1`・`AGENT_LINK_WEB_PORT`） |
| `agent-link-web projects` / `home` | プロジェクトの一覧・ホームの集計 |
| `agent-link-web questions [--run <id>] [--project <名前>]` / `question <qid>` / `answer <qid> (--choice … \| --choices … \| --text …)` | 質問の一覧・全文・答え |
| `agent-link-web send <project> <run_id> <文>` | 実行中の run に文を送る。質問が開いていれば質問を取り下げて文を渡す |
| `agent-link-web usage` | Claude の使用量（`GET /api/usage`） |
| `agent-link-web api <METHOD> <path> [--data <JSON>\|--data -]` | 上の API 一覧のどれでも呼ぶ |

終了コードは `agent-link` と同じ（0 成功・2 入力の誤り・3 見つからない・4 状態の衝突・1 その他）。HTTP の状態からは 400・422 → 2、404 → 3、409 → 4、そのほか → 1 に直す。

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
| `#/project/<name>/sessions` | セッション検索（会話の履歴の検索） | Projects | 表示 |
| `#/` | Chats 日付一覧 | Chats | 表示 |
| `#/day/<YYYY-MM-DD>` | Chats 実行一覧 | Chats | 表示 |
| `#/newchat?project=<名前>&block=<block>` | 新しいチャットのシート（`project`・`block` は省略できる） | Chats | 表示 |
| `#/run/<project>/<run_id>` | メッセージ | Chats | 隠す |
| `#/run/<project>/<run_id>/stats` | 統計 | Chats | 隠す |
| `#/ext/<name>` | 拡張の画面。最後に選んだプロジェクト（無ければ `enabled_in` の先頭）の `#/ext/<name>/<project>` へ置き換える。無ければ案内 | 拡張のタブ（`ext-<name>`） | 表示 |
| `#/ext/<name>/<project>[/<path>]` | 拡張の画面。上にプロジェクトを選ぶ欄（`enabled_in` だけ）、その下に拡張の `mount`。`<path>` は拡張が読む | 拡張のタブ（`ext-<name>`） | 表示 |

- List のサイドバーの一番上の「実行［＋］［↻］」の行で、**＋** は task.md 作成フォーム、**↻** はツリーの読み直し。書きかけは `localStorage`（プロジェクトごと）。
- Projects の 4 画面（Dashboard / Workflow / List / 設定）の見出しは同じ形（`‹`・プロジェクト名・`Dashboard|Workflow|List|設定`）。`‹` は 4 画面ともプロジェクト一覧（`#/projects`）へ戻る。
- 画面ファイル: `index.html`（枠）＋各画面の JS ＋ `markdown.js` ＋ `app.js`。**タブごとのファイルは `app.js` より前**に読み込む。新しい画面は `window.SaiRoutes` に登録する（`app.js` の分岐は増やさない）。

---

## 7. CLI の失敗の読み替え

終了コードから HTTP の状態への読み替え（0 → 200・2 → 400・3 → 404・4 → 409・1 とそのほか → 502・起動できない → 503・時間切れ → 504）と `detail` の規則は、[`for-developers.md`](./for-developers.md) の「Web が CLI を呼ぶ約束」に全文がある。

---

## 8. 設定（環境変数）

環境変数の表（`config` に書ける名前を含む）は [`../README.md`](../README.md) の「設定（環境変数と `config`）」にある（PATH の注意は §9）。

---

## 9. 注意点

| 注意点 | 内容 |
|---|---|
| **CLI は PATH に要る** | `agent-link` は `target/release/` にできる。PATH に足すか、`AGENT_LINK_BIN` に絶対パスを書く。忘れると Projects が全部 503 |
| **release ビルド** | Web が呼ぶのは release の実行ファイル。`crates/` を直したら `cargo build --release` で作り直さないと古いまま |
| **置き場は `AGENT_LINK_HOME`** | 登録簿・書き出し・共通定義はここ。`AGENT_LINK_HOME` を一時フォルダに差し替えれば、テストを隔離できる |
| **`AGENT_LINK_PROJECT_ROOT`** | `agent-link` は `AGENT_LINK_PROJECT_ROOT` を最優先で DB 解決に使う。Web は呼び出しのたびに必ずプロジェクトの root を渡す（`control.py`）。これを消すと、起動元の環境によっては**全プロジェクトが同じ DB を開いてしまう** |
| **FastAPI の登録順** | 固定パスを変数のパスより前に登録しないと 404 / 400 になる。順番を変えたら curl で一覧を確かめる |
| **`sw.js` の `CACHE`** | 版を上げないと iPhone に古い画面が残る。画面ファイルを足したら `ASSETS` にも足す |
| **常駐・実行を動かす操作** | 常駐の起動停止・カードの実行は、実際にエージェントを動かす。確認は `--help` と不正値で行う |
| **8400 番の Web は触らない** | 動いている Web を確認のために止めない。確認は 8297 番など別の番号で行う |
