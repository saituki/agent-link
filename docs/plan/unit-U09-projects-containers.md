# 実装計画書 U09: プロジェクトの作成・コンテナの操作・API キーと Claude のトークン（Web のサーバ側）

## 目的
多人数モードで、ログインした人が Web から次をできるようにする。
- 研究プロジェクトを作る（フォルダ・プロジェクト専用の HOME・コンテナ・`sai-link init`・`sai-paper init`・常駐の起動まで）。
- 自分のプロジェクトのコンテナを起動・停止し、状態を見る。
- 自分の API キー（OpenRouter・Semantic Scholar・OpenAlex・NCBI）と Claude の長命トークン（`CLAUDE_CODE_OAUTH_TOKEN`）を保存する。保存すると、自分のコンテナが新しい値で作り直される。

管理者は、全員のプロジェクトのコンテナの一覧を見て、起動・停止できる。

## 背景
- sai-tree は sai-moon を複製した Linux の多人数サーバ版である。1 つのプロジェクトに rootless Docker のコンテナを 1 つ割り当てる。コンテナには、そのプロジェクトの root と専用の HOME だけを見せる（要件）。コンテナの形は U08 の共有契約 C7、置き場は U06 の C4 で決まっている。CLI は U08 の中継（`sai-in-container`）でコンテナの中で動く。
- Claude は、各自が自分の Mac で `claude setup-token` を実行して発行した長命トークンを画面に貼り、Web が保存して、その人の全部のコンテナへ環境変数 `CLAUDE_CODE_OAUTH_TOKEN` で渡す（利用者の決定。規約上は灰色で、使う人の責任。調査 R3 §Q7）。長命トークンは更新（refresh）が無いので、複数のコンテナで同時に使ってもログイン情報のファイルの書き換えがぶつからない。
- Web からプロジェクトを作る手段は今は無い（`init` は Web の「コマンド」画面の対象外。`crates/sai-link-cli/src/cmd_schema.rs:188-194`）。
- キーとトークンはコンテナの環境変数として渡す（`--env-file`）。環境変数は作るときに決まるので、値を変えたらコンテナを作り直す。daemon がその環境を引き継ぎ、エージェント（cc・pi）と `sai-paper` に届く（`crates/sai-paper-core/src/config.rs:379`・`:146` の環境変数名）。
- Docker は `docker` の CLI を `subprocess` で呼ぶ（Python の docker の SDK は足さない。依存を増やさない約束）。

## 前提（着手時点のコードの状態）
- U06（多人数モード・`registry.current_user`・`registry.user_dir`・`registry.project_home`・`registry.within_user`・`auth.require_admin`）が終わっている。U08（イメージ・中継・C7）は並列で進む。この作業の作業用コピーには U08 が無いので、実物の Docker での確認（AC5）は U08 とのマージの後に kantoku が行う。worker は偽の docker のテストまでを行う。
- 常駐の生死は `registry.daemon_alive`（`apps/web/src/sai_web/registry.py:76-91`）が pid ファイルの PID をホストで確かめている。コンテナの中の PID はホストから見えないので、多人数モードでは使えない。daemon は `.sai/sai-engine/daemon.status` を約 5 秒ごとに書く（`crates/sai-link-engine/src/daemon.rs:1003-1008` `write_status_file`、間引きは `daemon.rs:57-63`）。
- プロジェクトの実行中の件数は `control._project_summary`（`control.py:73`。`sai-link project summary`）で取れる。

## 変更内容

### 1. 新しいモジュール `apps/web/src/sai_web/containers.py`
- `docker` の呼び出しは 1 つの関数 `_docker(args: list[str], timeout=60) -> subprocess.CompletedProcess` に集める（実行ファイルは環境変数 `SAI_DOCKER_BIN`、既定 `docker`。テストは偽の docker を差す）。失敗は `HTTPException(502, "<標準エラーの最後の行>")`、見つからない・つながらないときは 503「docker が動いていません」。
- `run_args(user: str, project_root: Path, project_home: Path, token: str) -> list[str]` — C8 の引数を作る純関数（テストで全文を照合する）。
- `create_container(user, name)` — `docker run` を呼ぶ。内部用の合言葉（`token`）は U10 が使う。ここでは `secrets.token_urlsafe(32)` を作り、SHA-256 を `store.save_project_token(sha, owner, project)` で保存する（**この作業で表と関数を作る**: `project_tokens(token_sha256 TEXT PRIMARY KEY, owner TEXT NOT NULL, project TEXT NOT NULL, created_at REAL NOT NULL)`。同じ project の古い行は消す。`find_project_token(sha) -> {"owner","project"} | None` も作る）。
- `create_project(user, short)`:
  1. `short` を C4 の規則で検査（違えば 400「名前は英小文字・数字・ハイフンで 30 文字までにしてください」）。
  2. name = `<user>-<short>`、root = `<user_dir>/projects/<name>`、home = `<user_dir>/homes/<name>`。どちらか既にあれば 409「同じ名前のプロジェクトがあります」。
  3. root と home を作る（0700）。`<user_dir>/secrets.env` が無ければ空で作る（0600）。home にひな形を写す: 環境変数 `SAI_WEB_SKEL_DIR`（例 `/srv/sai/skel`）があればその中身、続けて `<user_dir>/home/.pi/agent/models.json` があればそれを、home に**無いファイルだけ**写す（`shutil.copytree(…, dirs_exist_ok=True)` は上書きするので、ファイルごとに無いときだけ写す）。
  4. `create_container(user, name)`。
  5. 中継と同じ形で `docker exec -i -w <root> -e SAI_PROJECT_ROOT=<root> <ctr>` を使い、`sai-link init --name <name>`・`sai-paper init`・`sai-link daemon start` を順に呼ぶ。どこかで失敗したら、コンテナを消し（`docker rm -f`）、root と home を消し、502 で理由を返す。
  6. 返す: `{"name": "<name>", "container": "sai-<name>"}`。
- `state(name) -> "running" | "exited" | "missing"` — `docker inspect -f '{{.State.Status}}'`。
- `start(name)` / `stop(name)` — `docker start` / `docker stop -t 20`。`start` でコンテナが無い（`missing`）ときは `create_container` で作り直す（イメージを入れ替えたあとに使う。`docs/server.md` の運用）。
- `list_all() -> list[dict]` — `docker ps -a --filter label=sai.owner --format '{{json .}}'` を読み、`{"container","owner","project","state","status"}` にする（ラベルから owner と project を取る）。
- `recreate_for_user(user) -> {"recreated": [...], "pending": [...]}` — その人のコンテナ（`label=sai.owner=<user>`）ごとに、`control._project_summary` の `running` が 0 なら `docker rm -f` → `create_container`（主プロセスが `.sai` を見て常駐を起こす。C7）。1 以上なら作り直さず `pending` に入れる。

### 2. API キーと Claude のトークン — `apps/web/src/sai_web/containers.py`
- 扱う名前は 5 つだけ（`KEY_NAMES`）: `CLAUDE_CODE_OAUTH_TOKEN`・`OPENROUTER_API_KEY`・`SEMANTICSCHOLAR_API_KEY`・`OPENALEX_API_KEY`・`NCBI_API_KEY`。
- `read_keys(user) -> dict[str, str]` — `secrets.env` の `KEY=値` の行を読む（`#` の行と空行は飛ばす）。
- `write_keys(user, changes: dict[str, str])` — 名前が `KEY_NAMES` に無ければ 400。値の検査: 空（＝消す）か、印字できる ASCII で空白と改行を含まず 400 文字以下。違えば 400。一時ファイル（0600）→ rename で書く。
- 値は API でも画面でも返さない。返すのは `{"name","set": bool,"last4"}` だけ。

### 3. 住所 — `apps/web/src/sai_web/main.py`（どれも多人数モードだけ。単独モードは 404）

| 住所 | 守り | すること |
|---|---|---|
| `POST /api/projects` `{"name": "<短い名前>"}` | `require_token` | `create_project(今の利用者, name)` |
| `GET /api/projects/{name}/container` | `require_token` | `find_project(name)` で今の利用者の物か確かめ（無ければ 404）、`{"container","state"}` |
| `POST /api/projects/{name}/container/{action}` | `require_token` | `action` は `start` / `stop`（他は 400）。同上の確認ののち実行 |
| `GET /api/me/keys` | `require_token` | `{"keys":[{"name","set","last4"}]}` |
| `PUT /api/me/keys` `{"CLAUDE_CODE_OAUTH_TOKEN"?: str, …}` | `require_token` | `write_keys` → `recreate_for_user` の結果を返す: `{"ok":true,"recreated":[…],"pending":[…]}` |
| `GET /api/admin/containers` | `require_token`・`require_admin` | `{"containers": list_all()}` |
| `POST /api/admin/containers/{container}/{action}` | `require_token`・`require_admin` | `container` は `^sai-[a-z][a-z0-9]{1,15}-[a-z0-9][a-z0-9-]{0,29}$`（違えば 400）。`start` / `stop` |

- 固定の住所（`/api/projects`）は、変数の住所より先に登録する（ARCHITECTURE §5 の 3）。
- `GET /api/projects`（一覧）の各行に、多人数モードでは `container_state` を足す（`control.list_projects` の結果に `containers.state` を足す。`main.py` 側で足し、`control.py` は変えない）。

### 4. 常駐の生死 — `apps/web/src/sai_web/registry.py:76` `daemon_alive`
- 多人数モードでは、`<root>/.sai/sai-engine/daemon.status` の更新時刻が 30 秒以内なら生きているとみなす。単独モードは今のまま。

## 共有契約（他の作業と共有する取り決め。worker は変えない）

**C4 多人数モードの置き場**（U06。全文）
- `<U>` = `SAI_WEB_USERS_ROOT`。`<U>/<user>/home/`（ひな形の HOME。Web だけが使う）・`<U>/<user>/projects/<user>-<project>/`（root）・`<U>/<user>/homes/<user>-<project>/`（コンテナの HOME。`SAI_HOME`＝`<HOME>/.sai`）・`<U>/<user>/secrets.env`（0600。`--env-file` で渡し、ファイルは見せない）。利用者の短い名前 `^[a-z][a-z0-9]{1,15}$`、プロジェクトの短い名前 `^[a-z0-9][a-z0-9-]{0,29}$`。コンテナに見せるのは root と HOME の 2 つだけ。

**C7 コンテナと中継**（U08。全文）
- 名前 `sai-<root のフォルダ名>`、ラベル `sai.owner`・`sai.project`、イメージ `SAI_IMAGE`（既定 `sai-tree:latest`）、root で動かす（`--user` を付けない）、`-w` は root、主プロセスは `.sai` があれば常駐を起こして眠る。中継は `SAI_PROJECT_ROOT`・`SAI_ORIGIN` だけを渡す。

**C8 `docker run` の引数**（この作業が作る。U10 が内部用の合言葉を使う）
```
docker run -d --name sai-<user>-<project> --init --restart unless-stopped
  --memory <SAI_CONTAINER_MEMORY 既定 8g> --memory-swap <同じ値>
  --cpus <SAI_CONTAINER_CPUS 既定 2> --pids-limit 1024
  --security-opt no-new-privileges --cap-drop ALL
  --add-host host.docker.internal:host-gateway
  -v <root>:<root> -v <home>:<home>
  -w <root>
  -e HOME=<home> -e SAI_PROJECT_ROOT=<root>
  -e SAI_WEB_URL=<SAI_WEB_URL_FOR_CONTAINERS 既定 http://host.docker.internal:8200>
  -e SAI_WEB_INTERNAL_TOKEN=<合言葉>
  --env-file <U>/<user>/secrets.env
  --label sai.owner=<user> --label sai.project=<user>-<project>
  <SAI_IMAGE>
```

**C9 プロジェクト・コンテナ・キーの API**（U11 の画面が使う。§3 の表が全文）

## ファイルの所有権
- 書いてよい: `apps/web/src/sai_web/containers.py`（新規）・`main.py`（§3 の住所と一覧の 1 項目）・`registry.py`（`daemon_alive` だけ）・`store.py`（`project_tokens` の表と 2 つの関数だけ）・`apps/web/tests/test_containers.py`（新規）・`docs/integration.md`（§3 の住所を足す）
- 読むだけ: `apps/web/src/sai_web/control.py`・`auth.py`・`apps/web/server/`
- 触らない: `apps/web/src/sai_web/static/`（U11）・`apps/web/hooks/`・`crates/`

## 完了条件
- [ ] AC1: `apps/web/tests/test_containers.py`（偽の docker を `SAI_DOCKER_BIN` で差す。呼ばれた引数を記録する）: `run_args` が C8 の全文と一致し、bind が root と home の 2 つだけである。`create_project` が docker run → init 3 本の順に呼ぶ。途中の失敗で `docker rm -f` とフォルダの削除が走る。名前の誤りは docker を呼ばずに 400。同じ名前は 409。ひな形が上書きせずに写る。
- [ ] AC2: 同じテストで、`PUT /api/me/keys` が `secrets.env` を 0600 で書き、`GET` が値を返さず `last4` だけを返す。改行入りの値・`KEY_NAMES` に無い名前は 400。実行中のプロジェクト（偽の CLI で `running: 1`）は `pending` に入り、作り直されない。
- [ ] AC3: 他人のプロジェクト名で `/container` を叩くと 404（docker を呼ばない）。一般の利用者の `/api/admin/containers` は 403。
- [ ] AC4: 多人数モードの `daemon_alive` が `daemon.status` の更新時刻で決まる（古いファイルで False、新しいファイルで True）。
- [ ] AC5（U08 とのマージ後に kantoku が行う）: Mac の Docker で U08 の `sai-tree:check` を使い、`SAI_WEB_USERS_ROOT` をホーム直下の一時フォルダにした 8299 番の多人数モードのサーバで、ログインした利用者が `POST /api/projects {"name":"x"}` → コンテナが動き、`GET /api/projects` に `daemon_alive: true` で出る。同じ利用者の 2 つ目のプロジェクト `y` を作り、`x` のコンテナの中から `y` の root と HOME が見えないことを確かめる。`PUT /api/me/keys` で `OPENROUTER_API_KEY` を入れると、コンテナの中で `printenv OPENROUTER_API_KEY` が値を持つ。終わったらコンテナ・イメージ・一時フォルダを消す。
- [ ] AC6: 既存の Web のテストがすべて通る。

## 検証
```bash
cd ~/workdir/Sai/sai-tree
for f in apps/web/tests/test_*.py; do PYTHONPATH=apps/web/src apps/web/.venv/bin/python "$f" || exit 1; done
for f in apps/web/tests/test_*.js; do node "$f" || exit 1; done
```

## 決定事項と理由
- キーとトークンはコンテナの `--env-file` で渡し、変えたら作り直す — 理由: daemon と CLI の両方に同じ値が届き、engine と sai-paper を変えずに済む。実行中のコンテナは作り直さない（作業を壊さない）。/ 採用しなかった案: キーを暗号化して DB に置く（`cryptography` の依存が増える。サーバは管理者が管理する機械で、ファイルは 0600）。
- Claude は長命トークンを預かる — 理由: 利用者の決定。1 回の発行で全部のプロジェクトに効き、ログイン情報のファイルの更新の衝突（anthropics/claude-code#56339）も起きない。規約上の灰色は画面（U11）と `docs/server.md` で知らせる。Web の端末でのログインの中継は作らない。
- コンテナに見せるのは root と専用の HOME だけ — 理由: 要件（エージェントから他のプロジェクトのデータを物理的に見えなくする）。同じ人の別のプロジェクトも見えない。
- プロジェクトの削除は作らない — 理由: 要件の範囲外。取り返しがつかないため、手順書（`docs/server.md`）に任せる。
- docker の SDK を使わず CLI を呼ぶ — 理由: 依存を増やさない。

## 要確認
- `--cap-drop ALL` で、cc・pi・git・uv が普段の作業に困らないか（AC5 で `sai-link daemon start` まで動けばよい。困る物があれば、足した capability と理由を書く）。
- 1 つの `CLAUDE_CODE_OAUTH_TOKEN` を複数のコンテナで同時に使って問題が無いか（公式の文書に明記が無い。調査 R3）。問題が出たら作業報告書に書く。
