# 実装計画書 U02: Web のサーバ（Python）を単独モードの sai-link の窓口だけにする

## 目的
`apps/web` のサーバ（FastAPI）から、論文・Research・Radio・MLflow・sai-ts・sai-prep・Customer サポート・多人数モードを消す。単独モード（合言葉 `SAI_WEB_TOKEN`）で sai-link の画面だけを出すサーバにする。Python のテストが通る状態にする。

## 背景
- agent-link は Mac で 1 人が使う開発用の道具である（要件定義書 `docs/plan/requirements.md`）。
- 単独と多人数の切り替えの合図は 1 つだけで、環境変数 `SAI_WEB_USERS_ROOT` である（`apps/web/src/sai_web/registry.py:57-74` の `users_root()`・`multi_user()`）。多人数の分岐を消し、単独の側だけを残す。
- 名前（`sai_web`・`sai-web`・`SAI_*`）はこの作業では変えない。後の U06 が変える。

## 前提（着手時点のコードの状態）
- リポジトリは `~/workdir/Sai/agent-link`（sai-tree の main を clone した物）。行番号は複製の元（sai-tree の 4736175）の値。ずれていたら関数名・route の文字列で探す。
- 同じガイドの U01（Rust）・U03（Web の画面）・U04（ほかのアプリ・CI）と並列で動く。U01 が CLI の JSON から消すのは `after_job`・`target_node`・`layer`・台帳の数・Customer フォルダだけである（共有契約 C1）。

## 変更内容

### 1. ファイルを丸ごと消す（`apps/web/src/sai_web/`）
`auth.py`・`containers.py`・`claude_login.py`・`ts_gate.py`・`papers.py`・`sci.py`・`radio.py`・`mlflow_proxy.py`・`bundles.py`・`datafiles.py`・`paper_migrate.py`

`apps/web/server/` を丸ごと消す（`Dockerfile`・`deploy.sh`・`dev.sh`・`sai-in-container`・`sai-web.service`・`test_sai_in_container.sh`）。

`apps/web/hooks/cc_question.py`・`cc_register.py` は**残す**。ただし `SAI_WEB_INTERNAL_TOKEN` の Authorization を足す所（`cc_register.py:30-32`・`cc_question.py:53-55`）は消す。単独モードでは localhost からの呼び出しを `require_internal` が通す。

### 2. `main.py`
- `app` を作る所（`:63-68`）: 単独の `FastAPI(title="sai-web")` だけにする。
- ミドルウェア `_multi_user_auth`（`:107-220`）を消す。`require_token`（`:86-94`）は単独の合言葉の照合だけを残す。
- `require_internal` の多人数の枝（`:262-265`）を消す。localhost の判定（`:224-268` の残り）は hooks が使うので残す。
- `_startup`（`:300-320`）の多人数の分岐（`:304-307`・`:312-313`）を消す。`write_token_file`（`:271-297`）は残す。
- route を消す・直す（下の表）。消す route が使っていた補助の関数も消す。

| route | すること |
|---|---|
| `/api/auth/*`（login・logout・password・me）、`/api/me/*`（tokens・keys・claude-login）、`/api/admin/*` | 消す |
| `GET /api/days` | `customer` の件数（`:771-778`）だけ消す。`q` の検索は残す |
| `GET /api/customer`・`POST /api/projects/{name}/customer` | 消す |
| `GET /api/runs/{project}/{run_id}/items` | `within_user` の枝（`:830-832`）だけ消す |
| `GET /api/questions/mine` | 消す（コンテナの中の CLI 専用だった） |
| `POST /api/questions/{qid}/answer` | `internal_project` の検査（`:1021-1027`）だけ消す |
| `POST /api/feedback`（と `:1041-1090`・`:1113-1115` の補助） | 消す |
| `POST /api/projects` | 消す（コンテナを作る route） |
| `GET /api/projects` | `container_state` の枝（`:1234-1244`）だけ消す |
| `GET/POST /api/projects/{name}/container*`・`POST /api/projects/{name}/ts/open` | 消す |
| `GET /api/home` | `papers` の項目を消す（`home.py` の変更と合わせる） |
| `GET /api/paper-projects`・`/api/projects/{name}/papers/*`（sci を含む 46 本）・`…/data/raw`（2 本）・`GET /api/prep/download` | 消す |
| `/api/radio/{path}`・`/api/mlflow/*`（8 本） | 消す |
| 上以外（health・manifest・runs・register・questions・push・send・interrupt・withdraw・finalize・dashboard・daemon・tree・search・history・md・waggle・card・schedule・sessions・advance・delete・commands・workflow・defs・home/overview・usage・static） | 残す |

### 3. 一部を消すファイル
- `registry.py`: 多人数の物を全部消す。`_USERS_ROOT_ENV`・`current_user`・`users_root`・`multi_user`・`user_dir`・`user_home`・`project_home`・`host_transcript`・`within_user`・`user_key`・`_load_user_projects`、`sai_home()`・`daemon_alive()`・`load_registry()` の多人数の枝（`:47-158`・`:161-165`・`:215-220`・`:232-272`・`:285-286`）。残すのは `sai_home()`（単独: `SAI_HOME` → `~/.sai`）・`registry_path`・`engine_db_path`・`daemon_pid_path`・`process_alive`・`daemon_alive`・登録簿を読む処理・`kind == "link"` の絞り込み。
  - `user_key()` を呼んでいる所（`home.py:261`・`:421`、`control.py:86`・`:166`）は、引数を空文字に置き換えるか、その引数ごと消す。
  - `host_transcript()` を呼んでいる所（`runs.py:258`・`:270`）は、渡された値をそのまま使う。
- `config.py`: `sai_paper_bin`（`:51`）・`sai_sci_bin`（`:54`）・`radio_url`・`radio_token`（`:58-60`）・`mlflow_*`（`:62-65`）・`ts_url`（`:67`）を消す。`sai_link_bin`・`CLAUDE_SESSIONS_DIR`・`DATA_DIR`・`DB_PATH` は残す。
- `store.py`: 表 `users`・`web_sessions`・`api_tokens`・`project_tokens`（`:61-91`）と、それを使う関数（`:467-690`）を消す。`_current_owner`・`_owner_filter`・`_ensure_owner_column`（`:97-129`）を消し、呼び出し元は「持ち主で絞らない」動きにする（今の単独モードと同じ結果）。`registrations`・`questions`・`subscriptions`・`approvals` の表は残す。`owner` の列は表の定義に残してよい。
- `webcli.py`: `user` の命令（`cmd_user`・`_read_new_password`・parser `:717-733`）、`feedback`（`:399-417`・`:712-715`）、`mlflow`（`cmd_mlflow` `:499-526`・`:736-746`）、sai-ts の待ち受け（`:576-614` の `_route_by_port`・`_DropTsAccess`・`_serve_with_ts`、`:598-599`・`:608-609`・`:628-630`）を消す。`SAI_WEB_INTERNAL_TOKEN` を読む所（`api_token()` の `:72-76`、コンテナ向けの質問の読み方 `:278-286`）は消し、合言葉は `SAI_WEB_TOKEN` と置き場の `web/token` だけにする。質問は常に `home()` の `questions` から読む。残す命令: `projects`・`home`・`questions`・`question`・`answer`・`send`・`usage`・`api`・`serve`。
- `commands.py`: `_CLI_LABELS`（`:44`）と `cli_table`（`:54-55`）から paper・sci を消す。コマンド画面は sai-link だけを出す。
- `control.py`: `customer_start` 一式（`:713-790`、`_CUSTOMER_TITLES`・`_customer_line`）、`phone_user_start`・`phone_user_finish`（`:786-808`）、`:572` の `runs.CUSTOMER_BLOCKS` の除外を消す。`DEV_BLOCKS`（`:561-563`）の注記の「sai-sci の block は出す」を消す。
- `runs.py`: Customer（`CUSTOMER_KINDS`・`CUSTOMER_BLOCKS` `:34-36`、`is_customer` `:94-96`、`build_days` の除外 `:435-440`、`build_day` の除外 `:493-494`、`build_customer` `:540-`）と、多人数の枝（`runs_dirs` `:141-147`、`load_runs` `:194`、`within_user` `:323`）を消す。
- `home.py`: 論文（`:27` の import、`_home_paper` `:180-197`、`paper_rows` と `"paper"` のジョブ `:211-220`、`papers_out` `:243-244`、`"papers"` のキー `:249`）を消す。
- `usage.py`: 多人数の枝（`:53-54`・`:69-70`・`:110-116`・`:466-467`、`_project_names` `:34-44` が多人数専用ならそれも）を消す。cc・OpenRouter・DeepSeek・pi の集計は残す。
- `watch.py`: 多人数の分岐（`:58-69`）を消し、単独の見回りだけにする。
- `sessions.py`: `_CLI_NAMES`（`:124`）と `_CLI_SUBWORDS`（`:127-135`）から `sai-paper`・`sai-sci`・`sai-web mlflow` を消す。

### 4. テスト（`apps/web/tests/test_*.py`）
- 消す: `test_auth.py`・`test_bundles.py`・`test_claude_login.py`・`test_containers.py`・`test_containers_ts.py`・`test_datafiles.py`・`test_mlflow_api.py`・`test_multi_user_paths.py`・`test_paper_migrate.py`・`test_radio_job.py`・`test_radio_proxy.py`・`test_sci_api.py`・`test_ts_gate.py`・`test_ts_open.py`・`test_papers_*.py`（8 本）。
- 直す（papers・sci・customer・多人数・mlflow・feedback・ts の検査を消す）: `test_cli_contract.py`・`test_commands_api.py`・`test_home.py`・`test_runs.py`・`test_webcli.py`・`test_usage.py`・`test_registry_projects.py`・`test_token_file.py`・`test_internal_auth.py`・`test_headless_runs.py`。
- 残す: そのほか。
- テストを消すたびに、残すテストが消したモジュールを import していないか確かめる。

### 5. そのほか
- `apps/web/pyproject.toml`・`uv.lock`: 依存は変えない（`fastapi`・`uvicorn`・`python-dotenv`・`pywebpush` は残す機能が使う）。
- `apps/web/.env.example` から、消した機能の環境変数（`SAI_WEB_USERS_ROOT`・`SAI_RADIO_*`・`SAI_MLFLOW_*`・`SAI_PAPER_BIN`・`SAI_SCI_BIN`・`SAI_WEB_TS_*`・`SAI_WEB_SHARED_DIR` など）を消す。

## 共有契約（他の作業と共有する取り決め。worker は変えない）
- C1（ガイド 1 の約束）: 残す CLI（`sai-link`）の JSON の項目は、次の物だけが消える。`after_job`・`target_node`・`layer`・台帳の数・Customer フォルダ（id `-1004`）。Web はこれらの項目を読まない。
- C1: この時点では名前を変えない（`sai_web`・`sai-web`・`SAI_*`・`.sai/` のまま）。
- C3（U02 と U03 の境目）: 残す API の住所と、返す JSON の形は変えない。例外は消す項目だけである。`GET /api/home` の `papers`、`GET /api/days` の `customer`、`GET /api/projects` の各項目の `container_state`・`container` の情報。画面（U03）はこれらの項目を読まないように直す。
- C3: `GET /api/auth/me` は消す。画面（U03）は `window.SaiAuth` を使わず、単独モードの前提で動く。

## ファイルの所有権
- 書いてよい: `apps/web/src/sai_web/*.py`・`apps/web/tests/test_*.py`・`apps/web/server/`（消す）・`apps/web/.env.example`・`apps/web/hooks/*.py`
- 読むだけ: `apps/web/src/sai_web/static/**`（U03 の担当）
- 触らない: `apps/web/src/sai_web/static/**`・`apps/web/tests/test_*.js`・`crates/**`・`.github/**`・`docs/**`

## 完了条件
- [ ] AC1: 変更内容 1 のファイルと `apps/web/server/` が無い。
- [ ] AC2: `PYTHONPATH=apps/web/src apps/web/.venv/bin/python -c "import sai_web.main"` が通る。
- [ ] AC3: 残す Python のテストがすべて通る（下の検証の命令）。
- [ ] AC4: `grep -rnE "USERS_ROOT|multi_user|containers|papers|sci\.|radio|mlflow|ts_gate|customer|feedback" apps/web/src/sai_web/*.py` の結果に、消した機能のコードが残っていない（変数名の一部が当たっただけの行は除く。残った行は完了報告に書く）。
- [ ] AC5: 一時フォルダを置き場にしてサーバを 8297 番で起こすと、`GET /api/health` が 200、`GET /api/home` が 200 で `papers` の項目が無い。`GET /api/auth/me` は 404 になる。

## 検証
環境変数は必ず外す（道具の sai-moon が `SAI_*` を入れている）。サーバは 8297 番で起こし、必ず止める（8299 番は使用中）。

```bash
cd ~/workdir/Sai/agent-link   # worktree の中ならその場所
(cd apps/web && uv sync)
for f in apps/web/tests/test_*.py; do
  env -u SAI_LINK_BIN -u SAI_PAPER_BIN -u SAI_SCI_BIN -u SAI_PROJECT_ROOT -u SAI_PARENT_RUN_ID \
    PYTHONPATH=apps/web/src apps/web/.venv/bin/python "$f" || exit 1
done

T=$(mktemp -d)
env -u SAI_PROJECT_ROOT -u SAI_PARENT_RUN_ID SAI_WEB_NO_WATCH=1 SAI_HOME="$T" SAI_WEB_TOKEN= \
  perl -e 'alarm 120; exec @ARGV' apps/web/bin/sai-web serve --port 8297 &
sleep 3
curl -s -o /dev/null -w "%{http_code}\n" http://127.0.0.1:8297/api/health
curl -s http://127.0.0.1:8297/api/home | python3 -c "import json,sys;d=json.load(sys.stdin);print('papers' in d)"
curl -s -o /dev/null -w "%{http_code}\n" http://127.0.0.1:8297/api/auth/me
kill %1; rm -rf "$T"
```
期待: テストがすべて通る。`200`、`False`、`404` が出る。

## 決定事項と理由
- 多人数モードは分岐ごと消し、単独の側だけを残す — 理由: agent-link は Mac の 1 人用（利用者の選択）。/ 採らなかった案: 合図を残して使わない。
- `store.py` の `owner` の列は表に残してよい — 理由: 単独モードでは読まないので害が無い。新しい DB から始めるので、移行の手間も無い。
- `approvals` の表は残す — 理由: 参照元が見当たらないが、消す判断の根拠が無い。消すかどうかは別の作業にする。

## 要確認
- `approvals` の表を使っている所は見つかっていない。残したまま完了報告に書く。
