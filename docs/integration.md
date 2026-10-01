# sai-moon の Web 層で変わったこと

作成日: 2026-09-27
**正はコード**（`apps/phone/src/sai_phone/` と `crates/`・`apps/radio` の実装）。計画と違う所はこの文書に書いてある。
ほか `crates/`・`apps/radio` 側を直すときの注意は [`for-sai-developers.md`](./for-sai-developers.md) を読むこと。

旧 sai-phone の `docs/integration.md` を土台に、sai-moon の値（パス・バイナリ名・ポート・置き場）に置き換えたもの。
API 契約そのもの（JSON の項目・終了コード）は変わっていないので、表の構造は流用できる。

---

## 1. 使い心地（何が変わったか）

| 前（4 リポジトリ） | 後（sai-moon） |
|---|---|
| Rust コアが `~/workdir/Sai/sai-link` と `~/workdir/Sai/sai-paper` の 2 つ | 1 つの Cargo ワークスペース `crates/`（8 クレート） |
| CLI が `sai-link`・`sai-paper` | `sai-moon-link`・`sai-moon-paper`（PATH 上で衝突しない） |
| Web が `~/workdir/Sai/sai-phone`（8100 番） | `apps/phone`（**8200 番**） |
| 音声が `~/workdir/Sai/sai-radio`（8000 番） | `apps/radio`（**8201 番**。TTS プロキシは **8202**） |
| 利用者全体の置き場が `~/.sai/` | **`~/.sai-moon/`**（`SAI_SAI_HOME` で上書き可） |
| 4 つの `git status` を見比べる | 1 つの `git status`（旧 4 つは読み取り専用） |
| ホーム画面から入るアイコンが複数（sai-phone・sai-radio） | アイコンは sai-moon ひとつ |

- **`apps/phone` の画面と API は変えない。** タブも 5 つのまま（ホーム / プロジェクト / 会話 / 論文 / 使用量）、住所（`#/home`・`#/projects`・`#/papers/library` など）も API も旧 sai-phone と同じ。ブラウザに保存済みの PWA は、ポートを 8200 番に合わせて起動し直せばそのまま使える（`sw.js` の `CACHE` が `sai-phone-v30`）。
  - ここに U06 で「実験」タブを足して 6 つになった（ホーム / プロジェクト / 会話 / 論文 / 実験 / 使用量）。MLflow へ読み取りだけで中継する `/api/mlflow/*`（§5）を見る画面。旧 sai-phone には無い画面。
- **変わるのは「中身の置き場」と「呼ぶバイナリ名」と「ポート」だけ。** §5〜§7 の表をこの 3 つだけ読み替えること。

---

## 2. 全体のつながり

```
iPhone（Safari・ホーム画面のアイコン 1 つ）
   │  HTTPS  tailscale serve :4443
   ▼
127.0.0.1:8200  apps/phone（FastAPI・窓口だけ）
   ├─ 会話   : ~/.sai-moon/phone/runs/<project>/<run_id>.json を読む   ← sai-moon-link が書き出す
   ├─ Project: 登録簿 `~/.sai-moon/registry.json` を直接読む ／ `sai-moon-link phone …` / `sai-moon-link daemon …` を呼ぶ
   ├─ Papers : `sai-moon-paper phone …`（cwd = そのプロジェクトの root・`SAI_PAPER_ROOT` も同じ）
   ├─ Radio  : /api/radio/* を 127.0.0.1:8201 の apps/radio /api/* へ中継
   └─ MLflow : /api/mlflow/* を 127.0.0.1:5001 の MLflow へ読み取りだけで中継

音声化の依頼:  apps/phone ──(背景で数分)──▶ `sai-moon-paper phone radio <id>`
                                          └─(HTTP)──▶ apps/radio POST /api/papers/from-extract

論文の Download:  apps/phone ──(背景)──▶ `sai-moon-paper phone download …`（PDF を取って登録）
                                          └─(通知)──▶「保存しました」→ 続けて `phone after-download <id>`
```

| タブ / 画面 | 使うアプリのどこ |
|---|---|
| 会話（日付・実行・メッセージ・統計） | sai-moon-link が書く書き出し JSON（`~/.sai-moon/phone/runs/`） |
| プロジェクト（一覧・Dashboard・List・todo 実行・作成フォーム・常駐起動停止・task / review の保存） | `sai-moon-link phone summary` / `phone dashboard` / `daemon <start\|stop>` と、`phone export` / `phone tree` / `phone run` / `phone md`（保存は `--write`） / `phone waggle` / `phone card options` / `card inputs` / `card create` / `card run`。一覧の `name` / `root` / `kind` / `root_exists` / `db_exists` / `daemon_alive` は登録簿（`apps/phone/src/sai_phone/registry.py`）から作る |
| 論文 → Dashboard（数字カード・出来事） | `sai-moon-paper phone dashboard`（中で apps/radio の `/api/logs?limit=` を読み、そのプロジェクトの音声の分だけ混ぜる） |
| 論文 → Discover（検索・過去の検索） | `sai-moon-paper phone search` / `phone log` / `phone providers` / `phone tree` |
| 論文 → Library（一覧・階層・論文情報・本文・PDF・要旨訳） | `sai-moon-paper phone library` / `phone tree` / `phone paper` / `phone translate` / `content paper` |
| 論文 → Radio（一覧・再生・文字起こし・質問・オフライン保存） | apps/radio の `/api/papers` `/api/papers/{id}` `/audio` `/listen` `/favorite` `/logs` `/ask` `/live-session` `/question-log` |
| 論文情報のノート保存・図 | `sai-moon-paper phone note <id>`（本文は標準入力）・`phone paper` の `figures` |
| 論文情報の「🎙 音声を作る」 | apps/phone が背景で `sai-moon-paper phone radio <id>` → apps/radio |
| プロジェクト → Workflow（Workflow 盤面・Kanban の 4 レーン） | `sai-moon-link phone workflow`（TUI と同じ配置の計算を通す） |
| プロジェクト → research-tree（実行の図・root / seed / branch の 3 層） | `sai-moon-link phone rtree`（node / edge の JSON。項目の説明は `docs/for-sai-developers.md` §2.2.2） |
| プロジェクト → 設定（block / template / settings の読み書き） | `sai-moon-link phone def list / show / write / delete`（`scope` でそのプロジェクトと `~/.sai-moon/` の両方） |
| 論文 → Import（BibTeX の読み込み・一覧・削除） | `sai-moon-paper phone import list / add / remove`（本文は標準入力） |
| 論文 → Download（PDF の取得と Library への登録） | apps/phone が背景で `sai-moon-paper phone download` → 続けて `phone after-download` |
| 論文 → Project（現況・考え・状況のまとめ・要対応） | `sai-moon-paper phone project` / `phone mind-append`（本文は標準入力）/ `phone attention-resolve` / `attention --json` |
| ホーム（質問・承認・プロジェクト・todo・論文の要対応） | 上の住所を 1 本の住所（`GET /api/home`）にまとめる。`ThreadPoolExecutor` で並べる |
| ホームの全プロジェクトの合計（今週 / 今月 / 全期間） | 各プロジェクトの `sai-moon-link phone dashboard --period` を並べて足し合わせる（`GET /api/home/overview`）。画面側で期間を変えたときに読み直すため別の住所にしている |

---

## 3. 大原則（4 つ）とその理由

1. **`apps/phone` は窓口だけ。** 処理とデータの保管は各アプリがする。
   - 他アプリの DB（`papr.db` / `sai-engine.db` / `sai_radio.db`）を**直接開かない**。
     理由: DB の形を変えたときに `apps/phone` が壊れ、原因が追いにくくなるため。
     （例外は 1 つだけ: 会話タブは sai-moon-link が書き出す `~/.sai-moon/phone/runs/*.json` を読む。これは「公開された書き出し」であり DB ではない。）
   - 機能は **そのアプリの CLI（JSON 出力）か HTTP API** だけを使う。
2. **既にある物を使う。** 新しい依存ライブラリは入れない（FastAPI・標準ライブラリ・素の JS のみ）。入れない理由: iPhone でも Mac でも動く単純さを保つため。
3. **画面は素の JavaScript**（フレームワーク・ビルド無し）。グラフは SVG を手で描く。
4. **合言葉（`SAI_PHONE_TOKEN`）で入口を守る。** エラーは画面に日本語でそのまま出す。

---

## 4. 置き場の変更（`~/.sai` → `~/.sai-moon`）

| 置き場 | 旧 | 新 |
|---|---|---|
| 登録簿 | `~/.sai/registry.json` | **`~/.sai-moon/registry.json`** |
| 書き出し（会話タブが読む） | `~/.sai/phone/runs/` | **`~/.sai-moon/phone/runs/`** |
| 共通の block | `~/.sai/blocks/` | **`~/.sai-moon/blocks/`** |
| 共通の template | `~/.sai/template/` | **`~/.sai-moon/template/`** |
| 共通の settings | `~/.sai/settings.json` | **`~/.sai-moon/settings.json`** |
| 論文の共通の API キー | `~/.sai/paper.env` | **`~/.sai-moon/paper.env`** |
| cc の使用量の記録 | `~/.sai/cc-cost/` | **`~/.sai-moon/cc-cost/`** |
| **プロジェクト内の `.sai/`** | `<root>/.sai/` | **変わらない**（`sai-engine.db`・`blocks/`・`template/`・`settings.json`・`outputs/`） |

- 環境変数 `SAI_SAI_HOME` があればそれが優先。無ければ `~/.sai-moon`。
  Rust 側は `sai_link_engine::project::sai_home()` と `sai_paper_core::paths::sai_home()`、`apps/phone` 側は `registry.sai_home()`。
- `apps/phone/src/sai_phone/usage.py` の `cc_limits_path()` は `<sai_home>/phone/cc-rate-limits.json` を読む。
  書くのは `sai-moon-link cc-statusline`（`phone_export.rs` の `cc_rate_limits_path()`）。旧 `~/.sai/phone/` は読まない。

---

## 5. `apps/phone` の API 一覧

すべて合言葉（`require_token`）で守る。エラーは `{"detail": "<日本語>"}`。

### プロジェクト（一覧・Dashboard・常駐）

| メソッド・パス | 中身 |
|---|---|
| `GET /api/projects` | プロジェクト一覧（`{"projects": [...]}`） |
| `GET /api/projects/{name}/dashboard?period=week\|month\|all` | Dashboard（既定 week） |
| `POST /api/projects/{name}/daemon` `{"action":"start"\|"stop"}` | 常駐の起動・停止 |

### ホーム

| メソッド・パス | 中身 |
|---|---|
| `GET /api/home` | 質問・承認・プロジェクト（review の run と todo の行）・論文の要対応・`errors` |
| `GET /api/home/overview?period=week\|month\|all` | 全プロジェクトをまたいだ合計（`period` 省略時は `week`・それ以外は 400） |

- 質問・承認は `store.list_open_questions` / `store.list_open_approvals` から作る。
  `project` / `run_id` は `runs.load_runs` の中で `run_session_ids` に `session_id` を含む実行を探して付ける（無ければ `null`）。
- プロジェクトの `review` / `todo` は `sai-moon-link phone tree` の `review` / `todo` フォルダを数える。論文の要対応は `sai-moon-paper attention --json` の件数と先頭 3 件。
- プロジェクトの `todo` は件数、`todo_runs` は同じフォルダの行（`[{run_id, label}]`・ツリーの順）。画面は `todo_runs` の行を押せるようにする。todo の行には `card`（`creation_date`・`block`・`runner`・`model`・`description`）が付く（`phone tree` の run ノードの追加項目。画面は「作成日 ID block runner model」と説明の 2 行で出す）。実行予約した todo の行には `scheduled_at`（`YYYY-MM-DD HH:MM`・`phone tree` の run ノードの追加項目）が付く。画面はこれがある行だけを「予約した todo」に並べる。`review` はホームには出さない（API には残す）。
- 各プロジェクトは `ThreadPoolExecutor(max_workers=4)` で並べる。1 つが失敗しても全体は止めず `errors` に「<名前>: <理由>」を足す。
- 結果は 10 秒だけ覚える（`home.forget()` で消す）。

`/api/home/overview`（U02 §1）は次を返す。

```json
{
  "period": "week", "since": "2026-09-21", "until": "2026-09-27", "projects": 2,
  "summary": {"executions": 12, "completed": 10, "failed": 1, "running": 1,
              "success_rate": 0.83, "duration_ms": 3600000,
              "tokens_input": 1, "tokens_output": 2, "tokens_cache": 3, "tokens_total": 1234567,
              "cost_usd": 3.21},
  "daily": [{"date": "2026-09-21", "executions": 3, "tokens": 120000}],
  "recent": [{"project": "sai-moon", "run_id": 42, "parent_run_id": 1, "block": "worker",
              "status": "completed", "started_at": "2026-09-27 10:28:21",
              "description": "…", "linkable": true}],
  "errors": []
}
```

- 対象は `/api/home` と同じプロジェクト（`root_exists` と `db_exists` が真）で、`sai-moon-link phone dashboard --period <期間>` を並べる。`linkable` が偽の `recent` は会話画面が開けないので List 側の画面へ。
- `summary` は各プロジェクトの同じ名前の数を足し、`success_rate` は足したあとの `completed / executions`（0 件なら `0.0`）で求め直す。`daily` は日付ごとに足す。`recent` は `started_at` の新しい順（同じなら `run_id` の大きい順）で最大 20 件。
- 期間も 10 秒だけ覚える（期間ごと）。`forget()` は両方消す。

### Workflow と設定（`sai-moon-link phone workflow` / `phone def` 経由）

| メソッド・パス | 中身 |
|---|---|
| `GET /api/projects/{name}/workflow` | Workflow 盤面（`phone workflow`・札・線・separators・Kanban の 4 レーン） |
| `GET /api/projects/{name}/defs` | 定義の一覧（`phone def list`） |
| `GET /api/projects/{name}/defs/{kind}/{scope}/{def_name}` | 定義 1 件の中身（`phone def show`） |
| `PUT /api/projects/{name}/defs/{kind}/{scope}/{def_name}` | 定義の保存（body `{"text": …, "create": false}`・`phone def write`・本文は標準入力。`create` が真なら `--create`） |
| `DELETE /api/projects/{name}/defs/{kind}/{scope}/{def_name}` | 定義の削除（`phone def delete`・settings は 400「settings は消せません」） |

- `kind` = `block` / `template` / `settings`、`scope` = `project` / `global`。`def_name` は block / template が `^[A-Za-z0-9][A-Za-z0-9_-]{0,63}$`、settings が `settings` だけ。違えば 400。
- `scope=global` の置き場は **`~/.sai-moon/`**（`phone_def.rs:132` が `sai_home()` を使う）。
- 定義の編集は **DB が無いプロジェクトでもできる**（root が無いときだけ 404「プロジェクトのフォルダが見つかりません」）。`phone workflow` は実行の記録が要るので 404。
- 終了コードの分け方: 0 = JSON をそのまま（読めなければ 502）/ 2 = 400（検査の理由を stderr の先頭 300 文字）/ 3 = 404（stderr）/ 1 で stderr が「同じ名前の定義があります」= 409 / それ以外 = 502（stderr の先頭 200 文字）。
- 本文は 256KB まで、`create` は真偽値。検査は CLI を呼ぶ**前**に行い、通らない値は CLI を 1 度も呼ばない。
- 住所の `{def_name}` は FastAPI 上の変数名も同じ（`token` は合言葉の `?token=` と衝突するため使わない）。

### List（`sai-moon-link phone …` 経由）

| メソッド・パス | 中身 |
|---|---|
| `GET /api/projects/{name}/tree` | 引き出しツリー（`phone tree`・TUI の左サイドバーと同じ）。各 run ノードは `executer`（`human` / `professor`）を持つ。`professor` の todo には画面が ▶ 実行を出さない |
| `GET /api/projects/{name}/search?q=…` | 文書（task / message / waggle）の本文検索（`phone search`・Dashboard の検索カード）。`q`（空白区切りで AND・200 文字まで）に加え、`category`（カンマ区切り・空で全部）・`since` / `until`（`YYYY-MM-DD`・実行日）・`block`（完全一致）・`from_id` / `to_id`（run_id の範囲）・`limit`（1〜100・既定 50）。通らない値は CLI を呼ばずに 400。返り値は `{items: [{run_id, category, file, token?, block, date, snippet}], truncated}` |
| `GET /api/projects/{name}/rtree` | 3 層の実行の図（`phone rtree`・U03。`{"nodes": […], "edges": […]}` をそのまま返す。実行の記録が要るので 404 の決まりは `tree` と同じ） |
| `GET /api/projects/{name}/runs/{run_id:int}/view/{log\|stats\|history}` | run の中身（`phone run`） |
| `GET /api/projects/{name}/runs/{run_id:int}/md/{task\|message\|report\|system}` | md 本文（`phone md`・「まだありません」は 404）。`report` は run の報告（`finalize-report.md`・子の `task_report` と root の `sai_finalize report` が書く）。`system` は固定部分（`system-prompt.md`・起動時に `--append-system-prompt` で渡す本文。U6） |
| `PUT /api/projects/{name}/runs/{run_id:int}/md/{task}` | md の保存（body `{"text"}`・`phone md --write`・task は未実行の todo カードだけで、それ以外は 404「未実行のカードが見つかりません」。message と report は保存できない） |
| `GET /api/projects/{name}/runs/{run_id:int}/waggle` | 成果物の一覧（`phone waggle`） |
| `GET /api/projects/{name}/runs/{run_id:int}/waggle/{token}` | 成果物 1 件の本文（`phone waggle --token`） |
| `POST /api/projects/{name}/runs/{run_id:int}/start` | 新形式 todo の ▶ 実行（`phone card run`・未実行カードが無ければ 404。`executer: professor` のカードは 502「このカードは professor が起こします（executer: professor）。人が起こすときは、カードの executer を human に直してください」） |
| `POST /api/projects/{name}/runs/{run_id:int}/advance` | review → done → Workflow の移動（`phone advance`・その run が無ければ 404「実行が見つかりません」、review / done 以外は 502） |
| `POST /api/projects/{name}/runs/{run_id:int}/schedule` | todo の実行予定時刻（`{"at": "YYYY-MM-DD HH:MM"}`・空で取り消し・`phone card schedule`）。時刻になると常駐が `phone card run` と同じ流れで実行する |
| `DELETE /api/projects/{name}/runs/{run_id:int}` | List の項目を消す（`phone rm`・子の実行と run フォルダも消える。実行中は 502） |
| `GET /api/projects/{name}/card/docs` | 最近の文書（`phone card docs`・task_text に貼る `ref` 付き） |
| `GET /api/projects/{name}/card/options` | 作成フォームの選択肢（`phone card options`） |
| `GET /api/projects/{name}/card/inputs?block=<id>` | 手順 4 の入力欄と候補（`phone card inputs`） |
| `POST /api/projects/{name}/card` | カード（task.md）の作成（`phone card create`・stdin に JSON。任意の `executer`（`human` / `professor`）も渡せる） |

- 外から来る値は `apps/phone` 自身が CLI を呼ぶ**前**に検査する（`run_id` は正の整数，
  `view` / `kind` は一覧の値だけ，`token` は英数字と `-_.:`・長さ 128 まで）→ 400。
- 作成フォームも同じ（`block` は英数字と `-_.:`、`task_text` は 100KB まで、`runner` / `thinking` は一覧の値、
  入力の値は `waggle:` 形式、`global_round` / `round` は数字、`tag` は 1 行）→ 400。
- CLI の失敗は 502（stderr の先頭 200 文字）。404 は 2 種類で、md が無いとき（終了コード 3・stderr「まだありません」）と
  未実行カードが無いとき（終了コード 3・stderr「未実行のカードが見つかりません」）。文言は呼び出し側が渡す。
- 呼び出しは cwd = プロジェクトの root，かつ `SAI_PROJECT_ROOT` = 同じ root（デーモン起動時の環境変数に引きずられない）。
- 住所の `{token}` は FastAPI 上の変数名だけ `{waggle_token}`（合言葉の `?token=` と名前が衝突するため・URL の形は同じ）。

### 論文（`sai-moon-paper phone …` 経由）

| メソッド・パス | 中身 |
|---|---|
| `GET /api/paper-projects` | 論文の置き場があるプロジェクトの一覧 `[{name, root_name, last_executed_at}]` |
| `GET /api/projects/{name}/papers/library` | Library 一覧 |
| `GET /api/projects/{name}/papers/tree` | Library 階層 |
| `GET /api/projects/{name}/papers/providers` | 検索先の一覧と既定 |
| `POST /api/projects/{name}/papers/search` | 検索（履歴にも残る） |
| `GET /api/projects/{name}/papers/logs/{log_id}` | 過去の検索結果 |
| `GET /api/projects/{name}/papers/{paper_id:int}` | 論文情報 |
| `POST /api/projects/{name}/papers/{paper_id:int}/translate` | 要旨の日本語訳 |
| `POST /api/projects/{name}/papers/{paper_id:int}/translate-pdf` | PDF の日本語訳を始める（202 `{"key": "pdf:<id>", "status": "processing"}`・翻訳中は 409・仕事は `downloads` に `kind="translate"`・`percent=null` で出る・`phone translate-pdf`） |
| `POST /api/projects/{name}/papers/browser/open` | Mac の Chrome で DOI のページを開く（body `{"doi": "10.…"}`・不正は 400・Chrome が無ければ 503） |
| `GET /api/projects/{name}/papers/browser/state` | `{open, url, title, width: 600, height: 1000, downloads: [{name, size, done, modified_at}]}`（`downloads` は新しい順） |
| `GET /api/projects/{name}/papers/browser/screen` | 画面の JPEG（600×1000・閉じていれば 409） |
| `POST /api/projects/{name}/papers/browser/input` | 操作（`{"type": "tap", "x": 0〜1, "y": 0〜1}` / `{"type": "scroll", "dy": -3000〜3000}` / `{"type": "text", "text": 1〜200 字の 1 行}` / `{"type": "key", "key": "Enter"\|"Backspace"\|"Tab"\|"Escape"}` / `{"type": "back"}` / `{"type": "reload"}`）→ `{"ok": true}`。形の違反は 400・閉じていれば 409 |
| `POST /api/projects/{name}/papers/browser/import` | 保存した PDF の取り込み（202・body は `manual/import` と同じ対象のどれか 1 つ＋ `file_name`・404 / 409 / 413。進み具合は `downloads` の `kind="manual"`） |
| `DELETE /api/projects/{name}/papers/browser` | Chrome を終える（`{"open": false}`） |
| `GET /api/projects/{name}/papers/{paper_id:int}/text` | 本文（Markdown） |
| `GET /api/projects/{name}/papers/{paper_id:int}/pdf?lang=ja` | PDF（`lang=ja` で日本語訳・パスがそのプロジェクトの root 外なら 404） |
| `GET /api/projects/{name}/papers/dashboard` | Dashboard（数字カードと出来事・`phone dashboard`） |
| `PUT /api/projects/{name}/papers/{paper_id:int}/note` | ノートの保存（body `{"text"}`・100KB まで） |
| `GET /api/projects/{name}/papers/{paper_id:int}/figures/{fig_no:int}` | 図の画像（`<img>` 用に合言葉は `?token=`・root 外なら 404） |
| `POST /api/projects/{name}/papers/{paper_id:int}/radio` | 音声化の依頼（背景で実行・202・返り値に `radio_id`） |
| `GET /api/projects/{name}/papers/import` | .bib の一覧と中の論文（`phone import list`） |
| `POST /api/projects/{name}/papers/import` | .bib の保存（body `{"name": "refs.bib" \| "", "text": "@article{…}"}`・本文は標準入力。名前が空なら `phone-<YYYYMMDD-HHMMSS>.bib`） |
| `DELETE /api/projects/{name}/papers/import/{file_name}` | .bib の削除（`phone import remove`） |
| `POST /api/projects/{name}/papers/download` | 論文の PDF を取る（202・background で `phone download` → `phone after-download`）。body は「`{"log_id": 3, "position": 2, "title": "…"}`」「`{"import_id": "DOI:…", "title": "…"}`」「`{"cite_paper_id": 12, "ref_no": 3, "title": "…"}`（引用の 1 件）」のどれか 1 つ |
| `GET /api/projects/{name}/papers/downloads` | 走っている／終わった仕事（`{"jobs": [{key, kind, title, status, paper_id, error, started_at, finished_at, steps, step, percent, messages}]}`・新しい順）。`kind` は `download` / `manual` / `cite-extract` / `radio` / `translate` の 5 つ。`percent` が `null` の仕事は「% の分からない仕事」（画面は動く帯で出す） |
| `GET /api/projects/{name}/papers/manual/candidates` | ダウンロードフォルダの最近の PDF 一覧（`phone manual-candidates --minutes 30`） |
| `POST /api/projects/{name}/papers/manual/import` | 手動で取り込む（202・background で `phone manual-import … --file …` → `phone after-download`）。body は「対象のどれか 1 つ」＋ `{"file_name": "paper.pdf"}`。ファイル名は 1〜255 字・`.pdf` で終わる物だけ（パスは受けない） |
| `POST /api/projects/{name}/papers/manual/upload` | この端末から送った PDF の取り込み（202）。対象はクエリで渡す（`?log_id=3&position=2` / `?import_id=…` / `?cite_paper_id=12&ref_no=3` / `?paper_id=12`）。本文は PDF そのもの（`application/pdf`）。`Content-Type` が違えば 415・`Content-Length` が無いか 104857600 バイト（100MB）を超えれば 413・先頭が `%PDF-` でなければ 400 |
| `GET /api/projects/{name}/papers/{paper_id:int}/citations` | 引用一覧（読み取りだけ・抜き出す前の論文にも空の形で返る・`phone citations`） |
| `POST /api/projects/{name}/papers/{paper_id:int}/citations/extract` | 引用を PDF から抜き出す（202・background で `phone citations-extract`。docling を使うので数十秒〜数分かかる）。走っているあいだは `cite:<paper_id>` の仕事として進捗に出る |
| `GET /api/projects/{name}/papers/project` | 現況・考え・状況のまとめ・要対応（`phone project`） |
| `POST /api/projects/{name}/papers/project/memo` | mind.md のメモを 1 行足す（body `{"text": "…"}`・`phone mind-append`・本文は標準入力） |
| `POST /api/projects/{name}/papers/attention/resolve` | 要対応の 1 件を対応済みにする（body `{"key": "…"}`・`phone attention-resolve`） |
| `GET /api/projects/{name}/papers/sci` | 研究方針の木全体（`phone sci status`） |
| `POST /api/projects/{name}/papers/sci/init` | 研究方針の木を始める（body `{"type": "problem"}`・型は 5 つだけ。`phone sci init --type <型>`） |
| `GET /api/projects/{name}/papers/sci/nodes/{node}` | NODE 1 つの中身（`phone sci node <N>`。`{node}` は `N12` の形） |
| `POST /api/projects/{name}/papers/sci/apply` | 利用者の操作を 1 つ反映する（body は op の JSON。`phone sci apply` の標準入力へ渡す） |
| `GET /api/projects/{name}/papers/sci/proposals` | TRIAGE 待ちの PROPOSAL の一覧（`phone sci proposals`） |
| `POST /api/projects/{name}/papers/sci/proposals/{pid}` | PROPOSAL を TRIAGE する（body `{"action": "accept\|reject\|revert", "note": "…", "edit": {…}}`。`phone sci triage <P> --action … [--note …]`・`edit` は標準入力） |
| `POST /api/projects/{name}/papers/sci/cards` | NODE のための実行（CARD）を作る（body はカードの JSON。`phone sci card` の標準入力へ渡す） |
| `GET /api/projects/{name}/papers/sci/reads/{run_id}` | READ_CARD の中身（`phone sci read <run_id>`） |
| `POST /api/projects/{name}/papers/sci/reads/{run_id}/start` | READ_CARD を始める（`sai-moon-link phone user start <run_id>`） |
| `POST /api/projects/{name}/papers/sci/reads/{run_id}/finish` | READ_CARD を終えて review へ移す（body は `{"no_reflect": "…"}`・任意。`phone sci finish-read <run_id> [--no-reflect …]` の後に `sai-moon-link phone user finish <run_id>`） |

- 古い `/api/papers/...` は無い。`{name}` は 3.1 の一覧の名前で，無ければ 404。
- `import` / `downloads` / `manual/candidates` / `manual/import` / `manual/upload` / `project` / `attention` は `/api/projects/{name}/papers/{paper_id:int}` より**先に**登録する（登録順に照合するため）。
- `papers/browser/…`（`open` / `state` / `screen` / `input` / `import` と `DELETE papers/browser`）も `{paper_id:int}` より**先に**登録する。
- リモートブラウザの Chrome は全体で 1 つ・最後の操作から 10 分で終了・専用プロファイル `~/.sai-moon/phone/browser-profile/`・ダウンロード先 `~/.sai-moon/phone/browser-downloads/`（7 日で消える）。DOI だけを受け、任意の URL は受けない。環境変数は `SAI_PHONE_CHROME`（Chrome の実行ファイル）と `SAI_PHONE_BROWSER_HEADED`（`1` で画面あり。出版社が headless を弾くとき）。
- 外から来る値は papers 側で CLI を呼ぶ**前**に検査する（.bib の名前は `^[A-Za-z0-9._-]{1,80}\.bib$`・本文は 1MB まで・Download は対象 3 つのうち 1 つ・手動取り込みも対象 4 つのうち 1 つ・メモは改行なしの 1〜2000 文字・key は 1〜300 文字の 1 行）→ 400。同じ論文が進行中は 409「この論文はダウンロード中です」。
- アップロードは `FastAPI` の `UploadFile` を使わず、**本文そのもの**（`application/pdf`）を受ける（`UploadFile` には `python-multipart` が要り、依存を増やさない大原則に反するため）。
- `papers/sci` の 10 本は `sci.py` が検査してから CLI を呼ぶ。NODE・PROPOSAL・QUESTION・FINDING・LINK の ID は `^[NQFLPS][1-9][0-9]{0,8}$`、run_id は 1〜999999999 の整数、型は 5 つだけ、`action` は 3 つ、`note` は 1000 字まで。通らない値は 400 で、CLI を 1 度も呼ばない。**ファイルのパスは受け取らない。**
- 仕事（`GET …/papers/downloads`）の状態は `apps/phone` のメモリだけに持つ。`downloading` = PDF を取って登録中、`processing` = 登録済みで後処理中、`done` = 終わり（`error` があれば後処理の失敗）、`failed` = 保存できなかった。終わった仕事は 30 分で一覧から消える。
- 実行ファイルは **`target/release/sai-moon-paper`**（PATH に無く、既定が絶対パス）。`apps/phone` は release しか叩かない。

### Radio 中継

| メソッド・パス | 中身 |
|---|---|
| `GET/POST /api/radio/{path}` | apps/radio の `/api/{path}` へそのまま中継（`papers/upload` と `papers/from-extract` は 403） |

- 中継先は `SAI_RADIO_URL`（既定 **`http://127.0.0.1:8201`**）。合言葉 `SAI_RADIO_TOKEN` は `apps/phone` 側にも同じ値を設定する。
- 中継で **パスの文字は `A-Za-z0-9_./-` のみ**（それ以外・`..` は 400）。

### MLflow 中継（実験タブ・U03）

| メソッド・パス | 中身 | MLflow への送り先 |
|---|---|---|
| `GET /api/mlflow/experiments` | 実験の一覧 | `POST experiments/search`（`max_results` は 100・`order_by` は `name ASC`） |
| `GET /api/mlflow/experiments/{experiment_id}` | 実験 1 件 | `GET experiments/get` |
| `POST /api/mlflow/runs/search` | 実行の検索 | `POST runs/search`（`run_view_type` は `ACTIVE_ONLY` 固定） |
| `GET /api/mlflow/runs/{run_id}` | 実行 1 件 | `GET runs/get` |
| `GET /api/mlflow/metrics/history` | 指標 1 本の履歴 | `GET metrics/get-history` |
| `GET /api/mlflow/metrics/bulk` | 指標 1 本の履歴をまとめて取る | `GET metrics/get-history-bulk-interval`（`run_ids` を繰り返しで送る） |
| `GET /api/mlflow/artifacts` | 成果物の一覧 | `GET artifacts/list` |
| `GET /api/mlflow/artifact` | 成果物 1 件の本文（5MB を超えたら 413） | `GET /get-artifact`（`run_uuid` と `path`） |

- 中継先は `SAI_MLFLOW_URL`（既定 **`http://127.0.0.1:5001`**）。名前は `SAI_MLFLOW_USER`・`SAI_MLFLOW_PASSWORD`（空なら Basic 認証を付けない）。
- **読み取りだけ**。書き込む住所は無い（`/api/mlflow/{path:path}` で全部通す作りは採らない）。
- 外から来る値は MLflow を呼ぶ**前**に検査する（実験 ID は数字の 1〜19 桁・run ID は 16 進の 32 桁・指標の名前は 250 字まで・成果物の場所は 500 字までで `..` の区切りと先頭の `/` は不可）→ 400。まとめて取る個数は `runs/search` が 100、`metrics/history` が 25000、`metrics/bulk` が 2500 まで。
- `POST /api/mlflow/runs/search` の本文は `{"experiment_ids": ["1"], "filter": "", "order_by": ["attributes.start_time DESC"], "max_results": 100, "page_token": "", "project": "sai-moon"}`。`project` があると**サーバが** `tags."sai.project" = "<登録簿の root のフォルダ名>"` を `filter` の前に `AND` で足す（利用者は `filter` を書かなくてよいし、画面も root を知らない）。登録簿に無い名前は 404。
- MLflow がエラーを返したら**本文と状態コードをそのまま**返す（例：`Invalid clause(s) in filter string` の 400）。つながらないときは 503「Mac の MLflow が応答しません」。
- 成果物の `Content-Type` は拡張子から決める（`mimetypes`。分からなければ `application/octet-stream`）。5MB を超えたら 413「大きすぎるので表示しません（5MB まで）」。

### 会話・質問・通知（統合前から同じ）

`GET /api/health` / `GET /api/days` / `GET /api/days/{date}` /
`GET /api/runs/{project}/{run_id}` / `…/items` / `POST …/send` /
`POST /api/runs/{project}/{run_id}/finalize`（メッセージ画面の ■ 終了・`phone finalize`・その run が無ければ 404、実行中でない・子ブロック・常駐停止は 502）/
`POST /api/register` `/api/questions` `/api/approvals`（内部専用・Mac 自身のみ）/
`GET /api/questions/pending` `/api/questions?session_id=` `/api/approvals` /
`POST /api/questions/{qid}/answer` `/api/approvals/{aid}/answer` /
`GET /api/push/key` `POST /api/push/subscribe`。

> **登録順の決まり**: FastAPI は登録した順に照合する。`/api/projects/{name}/papers/library`・`tree`・`providers`・`search`・`logs/{log_id}` は **`/api/projects/{name}/papers/{paper_id:int}` より前**に登録しないと ID として誤認される（`main.py` のコメント参照）。

---

## 6. 画面と URL（`#` ルート）

| URL | 画面 | タブ | タブバー |
|---|---|---|---|
| `#/` | 会話 日付一覧 | 会話 | 表示 |
| `#/day/<YYYY-MM-DD>` | 会話 実行一覧 | 会話 | 表示 |
| `#/run/<project>/<run_id>` | メッセージ | 会話 | 隠す |
| `#/run/<project>/<run_id>/stats` | 統計 | 会話 | 隠す |
| `#/projects` | プロジェクト一覧 | プロジェクト | 表示 |
| `#/project/<name>` | Dashboard | プロジェクト | 表示 |
| `#/project/<name>/list` | List（run 未選択） | プロジェクト | 表示 |
| `#/project/<name>/list/<run_id>/<view>` | List（`view` は log / stats / history / task / message / report / waggle。report の帯は「報告」と出る） | プロジェクト | 表示 |
| `#/home` | ホーム（アプリを開いたときの入口） | ホーム | 表示 |
| `#/project/<name>/workflow` | Workflow 盤面（Kanban の帯つき） | プロジェクト | 表示 |
| `#/project/<name>/defs` | 設定の一覧（block / template / settings） | プロジェクト | 表示 |
| `#/project/<name>/defs/<kind>/<scope>/<name>` | 設定の編集（`kind` = block / template / settings、`scope` = project / global） | プロジェクト | 表示 |
| `#/papers/import` | Import（BibTeX の読み込みと一覧） | 論文 | 表示 |
| `#/papers/tree` | 研究方針（論文タブの最初の画面） | 論文 | 表示 |
| `#/papers/tree/<N>` | 研究方針（NODE 1 つを選んだ状態） | 論文 | 表示 |
| `#/papers/triage` | TRIAGE（PROPOSAL の採用・却下・取り消し） | 論文 | 表示 |
| `#/papers/read/<run_id>` | READ_CARD（1 本の論文を読んで FINDING を書く） | 論文 | 表示 |
| `#/papers/dashboard` | Dashboard | 論文 | 表示 |
| `#/papers/discover` | Discover | 論文 | 表示 |
| `#/papers/library` | Library | 論文 | 表示 |
| `#/papers/radio` | Radio 一覧 | 論文 | 表示 |
| `#/paper/<name>/<id>` | 論文情報 | 論文 | 表示 |
| `#/paper/<name>/<id>/text` | 本文 | 論文 | 表示 |
| `#/paper/<name>/<id>/cites` | 引用一覧（絞り込み・詳細・引用の抜き出し・手動取り込み） | 論文 | 表示 |
| `#/papers/log/<name>/<log_id>` | 過去の検索結果 | 論文 | 表示 |
| `#/papers/log/<name>/<log_id>/<position>` | 未ダウンロード論文の情報 | 論文 | 表示 |
| `#/radio/<radio_id>` | 再生画面（質問チャット付き） | 論文 | 隠す |
| `#/mlflow` | 実験（MLflow）の一覧 | 実験 | 表示 |
| `#/mlflow/e/<experiment_id\|all>` | run の表（`?sai_run=<n>`・`?seed=<n>`・`?study=<s>`・`?status=<S>` を付けられる） | 実験 | 表示 |
| `#/mlflow/run/<run_id>` | run の詳細（設定・指標・タグ・成果物） | 実験 | 表示 |
| `#/mlflow/compare/<run_id>,<run_id>,…` | run の比較（2〜20 件。表は差分だけに絞れ、指標は重ねたグラフで見比べる） | 実験 | 表示 |

- 古い住所も開く: `#/paper/<id>` / `#/paper/<id>/text` / `#/papers/log/<log_id>` / `#/papers/log/<log_id>/<position>`（名前の無い形）は，今選んでいるプロジェクトの同じ画面として開く。
- 古い住所 `#/papers/canvas` と `#/canvas/…` は研究方針の画面（`#/papers/tree`）へ送る（消した画面の `scitree.js` の登録が `location.replace` で送り直す）。
- 住所に名前がある画面を開いたら，論文タブの「選んでいるプロジェクト」もその名前に切り替わる。選んだプロジェクトは `sai_phone_papers_project`（localStorage）に覚える。
  - 実験タブの「選んでいるプロジェクト」は別の `sai_phone_mlf_project`（localStorage）に覚える（論文タブと別々に持つ）。「すべてのプロジェクト」は `*` として覚え、`POST /api/mlflow/runs/search` には `project` を送らない。
- **List のサイドバーの一番上（todo フォルダの上）の「実行［＋］［↻］」の行**で，**＋** は **task.md 作成フォーム**（↻ はツリーの読み直し），**↻** でツリーを読み直す（どちらも住所は付けない一枚・`cardform.js` / `linkview.js`）。手順は 10 画面、書きかけは localStorage（プロジェクトごと）。
- Project の 4 画面（Dashboard / Workflow / List / 設定）の見出しは，同じ形（`‹`・プロジェクト名・`Dashboard|Workflow|List|設定`）にそろえてある（共通のクラス `.proj-bar`・`style.css`）。`‹` は 4 画面ともプロジェクト一覧（`#/projects`）へ戻る。上向きの固定は `html, body` の `overflow-x: clip` による（`hidden` にすると `body` がスクロールの箱になって `.appbar` と `.inputbar` の `position: sticky` が効かない）。
- 画面ファイル: `index.html`（枠）＋ `stats.js` / `project.js` / `linkview.js` / `cardform.js` / `papers.js` / `radio.js` / `chatorder.js` / `cite.js` / `scitree.js` / `sciedit.js` / `sciread.js` / `scigraph.js` / `mlflow.js` / `mlfchart.js`（画面ごと）＋ `markdown.js` ＋ `app.js`。
  **タブごとのファイルは `app.js` より前**に読み込む。新しい画面は `window.SaiRoutes` に登録する（`app.js` の分岐は増やさない）。

---

## 7. 設定（環境変数・`apps/phone/src/sai_phone/config.py`）

| 変数 | 既定値 | 用途 |
|---|---|---|
| `SAI_PHONE_PORT` | `8200` | 待ち受けポート |
| `SAI_PHONE_CHROME` | `/Applications/Google Chrome.app/Contents/MacOS/Google Chrome` | リモートブラウザが起動する Chrome |
| `SAI_PHONE_BROWSER_HEADED` | （空＝headless） | `1` で画面ありの Chrome（出版社が headless を弾くとき） |
| `SAI_PHONE_TOKEN` | （空＝認証なし・開発用） | 合言葉 |
| `SAI_LINK_BIN` | `sai-moon-link` | プロジェクトタブ・List 画面が呼ぶ CLI。**PATH 上に無い**ので PATH を足すか絶対パスを書く |
| `SAI_PAPER_BIN` | `…/sai-moon/target/release/sai-moon-paper` | 論文タブが呼ぶ CLI（PATH に無いため絶対パス） |
| `SAI_SAI_HOME` | （空＝`$HOME/.sai-moon`） | 登録簿の置き場。空なら `~/.sai-moon/registry.json` を読む |
| `SAI_RADIO_URL` | `http://127.0.0.1:8201` | Radio の中継先 |
| `SAI_RADIO_TOKEN` | （空） | apps/radio 側に合言葉があるときだけ同じ値 |
| `SAI_MLFLOW_URL` | `http://127.0.0.1:5001` | MLflow の中継先（実験タブ・読み取りだけ） |
| `SAI_MLFLOW_USER` / `SAI_MLFLOW_PASSWORD` | （空） | MLflow の Basic 認証。空なら付けない（127.0.0.1 の MLflow は認証しない） |
| `SAI_PHONE_RUNS_DIR` | `~/.sai-moon/phone/runs` | 会話タブが読む、書き出しフォルダ |
| `CLAUDE_SESSIONS_DIR` / `PI_SESSIONS_DIR` / `HERDR_BIN` | 各既定 | 会話の記録置き場・herdr（統合前から） |

Rust 側にも同じ `SAI_SAI_HOME`（`~/.sai-moon`）・`SAI_PROJECT_ROOT`・`SAI_PAPER_ROOT` が効く。
launchd（`apps/phone/launchd/com.sai-moon.plist`）では `PYTHONPATH`（`apps/phone/src`）と
`PATH`（`sai-moon/target/release:sai-moon/target/debug:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin`）を渡している。
**PATH がないと `sai-moon-link` が見つからず 503 になる。**

---

## 8. 統合後に見つかった注意点

| 注意点 | 内容 |
|---|---|
| **`sai-moon-link` は PATH に無い** | 既定値は `sai-moon-link` という名前だけで、PATH 上には無い。起動時は `PATH=$HOME/workdir/Sai/sai-moon/target/release:$PATH` を足すか、`SAI_LINK_BIN` に絶対パス（`target/release/sai-moon-link`）を書く。忘れるとプロジェクトタブが全部 503 |
| **`sai-moon-paper` は release ビルド** | `SAI_PAPER_BIN` の場所の実行ファイルを使う。`crates/sai-paper-*` を直したら `cargo build --release` で作り直さないと古いまま |
| **置き場が `~/.sai-moon`** | 登録簿・書き出し・共通定義・`paper.env` はここ。`~/.sai` ではない。`SAI_SAI_HOME` で一時フォルダに差し替えられる（テストはこれで隔離する） |
| **`~/.sai/phone/runs` の過去分は自動で移らない** | `sai-moon-link phone export` を流すと `~/.sai-moon/phone/runs` に新しく出る。古い方はそのまま残る |
| **使用量の `cc-rate-limits.json` だけは `~/.sai`** | `usage.py:34`。利用者の statusline が書くファイルなので `~/.sai` のまま |
| **`sai-moon-link-tui` は `--help` に出ない** | 起動するとすぐ TUI に入る。`--help` を試さないこと（`sai-moon-link tui` 経由でも同じ） |
| **`sai-moon-link phone` の `SAI_PROJECT_ROOT`** | sai-moon-link は `SAI_PROJECT_ROOT`（デーモン起動時に設定された値）を最優先で DB 解決に使う。List の API は cwd = root に加え `control._phone_env` で必ず root を渡している。これを消すと，起動元の環境によっては**全プロジェクトが同じ DB を開いてしまう**（ツリーが全部同じになる） |
| **Radio の論文 ID は英数字** | ID は「論文 ID（`<root のフォルダ名>-<番号>`・例 `sai-moon-30`）」と「apps/radio が作った音声（12 桁の 16 進・`demo001` 等）」の両方がある。数字だけと当て込むと壊れる。判定は `radio.js` の `isRadioPaperId` |
| **長すぎる論文は音声化できない** | apps/radio が論文全文をそのまま LLM に 1 回で渡す（`llm.py`）ため、約 13 万トークンを超える論文は文脈上限で失敗する（**apps/radio 側の制限**）。`apps/phone` / `sai-moon-paper` 側に分割処理はない |
| **FastAPI の登録順** | 固定パス（`library` `tree` `providers` `search` `logs/{log_id}` `manual/candidates` `manual/import` `manual/upload`）を `/api/projects/{name}/papers/{paper_id:int}` より前に登録しないと 404 / 400 になる。順番を変えたら curl で一覧を確かめる |
| **`sw.js` の `CACHE`** | 版（今は `sai-phone-v31`）を上げないと iPhone に古い画面が残る。画面ファイルを足したら `ASSETS` にも足す。音声の倉庫 `sai-phone-radio-audio` は別物なので消さない |
| **launchd の PATH** | plist に `PATH` が無いとプロジェクトタブ全部が 503（「`sai-moon-link` が見つかりません」）になる |
| **常駐・実行を動かす操作** | 常駐の起動停止・todo 実行・音声化の依頼は実際にエージェントや生成を動かす。確認は `--help` と不正値で行う |
| **旧構成の 8100 番・8000 番は触らない** | 旧 sai-phone と旧 sai-radio は別のプロセスで動いている。sai-moon の確認で再起動しないこと |
| **旧 4 リポジトリは読み取り専用** | `~/workdir/Sai/sai-link`・`sai-paper`・`sai-phone`・`sai-radio` に書き込みしない。着手前から未 commit の変更があるので、触る前に `git status --short` を控えておき、終わりに同じであることを確かめる |
