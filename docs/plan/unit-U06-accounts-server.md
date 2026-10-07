# 実装計画書 U06: アカウントとログイン（Web のサーバ側・多人数モード・利用者ごとの置き場・個人のトークン）

## 目的
環境変数 `SAI_WEB_USERS_ROOT` を付けて `sai-web` を起こすと「多人数モード」になり、**大学のメールアドレスとパスワード**でログインした人だけが API を使えるようにする。ログインした人ごとに、プロジェクト・実行・会話・Usage が分かれる。sai-prep（各自の Mac のアプリ）のために、各自が「個人のトークン」を作り、`Authorization: Bearer` で API を呼べるようにする。`SAI_WEB_USERS_ROOT` が無ければ、今の「単独モード」（合言葉。開発とテスト用）のまま何も変わらない。

## 背景
- sai-tree は sai-moon を複製した Linux の多人数サーバ版である。ログインの ID は大学のメールアドレス、フォルダ名とコンテナ名には管理者が決める短い名前を使う（要件）。
- 今の入口の守りは合言葉 1 つ（`apps/web/src/sai_web/main.py:59-72` `require_token`）で、利用者の区別が無い。
- 置き場は `registry.sai_home()`（`apps/web/src/sai_web/registry.py:41`）1 つで決まり、登録簿（`registry.py:95` `load_registry`）・実行の書き出し（`config.py:54-56` の `runs_dir`、`runs.py:126`）・pi の記録（`usage.py:40-42`・`:196`）がそこから決まる。**これらを「ログインした人の置き場」から引くように変えれば、上に乗る処理はほぼそのまま利用者ごとに分かれる。**
- エージェントのコンテナ（ガイド 4）には、そのプロジェクトのフォルダと**プロジェクト専用の HOME** だけを見せる（同じ人の別のプロジェクトも見えなくするため。要件）。そのため、実行の書き出し（engine が `$SAI_HOME/web/runs/<name>/` に書く。`crates/sai-link-engine/src/phone_export.rs:632-640`）と cc・pi の記録は、プロジェクトごとの HOME の中にできる。Web はそれらを利用者のプロジェクトごとに集めて読む。置き場の形は共有契約 C4 で決める。

## 前提（着手時点のコードの状態）
- ガイド 1・2 と U05 が終わっている（Web に herdr・承認・遠隔ブラウザは無い）。
- `require_token` は約 100 か所の住所に `Depends(require_token)` で付いている。付いていない住所: `/api/health`・内部用（`require_local`。`POST /api/register`・`POST /api/questions`・`POST /api/questions/{qid}/resolved`）・`GET /api/questions/{qid}`・静的ファイル・FastAPI の `/docs`。
- Web 自身の DB は `store.py`（`<SAI_HOME>/web/sai_web.db`、`config.py:22-26`）。
- サーバ全体で共有している覚え（キャッシュ）: `home.py:60` `_cache`・`home.py:62` `_overview_cache`、`papers.py:53` `_projects_cache`、`usage.py:45-47` `_pi_cache`・`_summary_cache`、`control.py:41` `_last_export`・`control.py:47` `_summary_cache`。
- `sai-web` の CLI は `apps/web/src/sai_web/webcli.py`（`serve` は `:485` 付近）。

## 変更内容

### 1. 多人数モードと「今の利用者」 — `apps/web/src/sai_web/registry.py`
共有契約 C5 の関数を足す。
```python
from contextvars import ContextVar
current_user: ContextVar[dict | None] = ContextVar("sai_current_user", default=None)
# 値は {"name": "saito", "email": "saito@…", "is_admin": False}

def users_root() -> Path | None:      # SAI_WEB_USERS_ROOT（空でない絶対パス）。無ければ None
def multi_user() -> bool:             # users_root() is not None
def user_dir(name=None) -> Path:      # <users_root>/<name or 今の利用者>
def user_home(name=None) -> Path:     # 多人数: user_dir()/"home"（その人のひな形の HOME）。単独: Path.home()
def project_home(project: str) -> Path:   # 多人数: user_dir()/"homes"/project。単独: Path.home()
def sai_home() -> Path:
    # 多人数で今の利用者がいる: user_home() / ".sai"
    # それ以外: 今のまま（SAI_HOME → ~/.sai）
def within_user(path) -> bool:        # 多人数: resolve() が user_dir() の中。単独: 常に真
```
- 多人数モードで「今の利用者」がいないときの `sai_home()` は、サーバ自身の置き場（今のまま）。Web の DB・通知の鍵はこちらに置く（`config.py:22` の `DATA_DIR` は import 時に決まるので変わらない）。

### 2. プロジェクトと実行の一覧を利用者ごとにする
- `registry.load_registry`（`registry.py:95`）: 多人数モードでは `registry.json` を読まず、`<user_dir>/projects/` の直下のフォルダのうち、名前が `<今の利用者>-` で始まり、`.sai/` を持つ物を、`{"name": <フォルダ名>, "root": <絶対パス>, "kind": "link", …}` として返す（`last_opened` は `.sai/` の更新時刻）。`root_exists`・`db_exists`・`daemon_alive` の足し方は今のまま（`daemon_alive` は U09 が直す）。
- `runs.py:126`（実行の書き出しの読み込み）: 置き場を 1 つの `settings.runs_dir` ではなく、関数 `runs_dirs() -> list[Path]` から引く。
  - `SAI_WEB_RUNS_DIR` が設定されていれば `[その値]`（今のテストの差し替え方を保つ）。
  - 単独モード: `[registry.sai_home() / "web" / "runs"]`。
  - 多人数モード: 今の利用者のプロジェクトごとに `project_home(name) / ".sai/web/runs"`。
- `runs.load_runs`（`runs.py:119`）: 多人数モードでは、書き出しの `project_root` が `load_registry` の結果に無い実行を捨てる。
- `main.py:217-245` `api_run_items`: `transcript_path` が `within_user` でなければ 404「会話の記録が見つかりません」。U03 の `runs.headless_state` も `within_user` でなければ None。
- `usage.py`: pi の記録（`:196` の `settings.pi_sessions_dir`）は、多人数モードでは利用者のプロジェクトごとの `project_home(name) / ".pi/agent/sessions"` を全部読む。`PI_MODELS_PATH`・`PI_AUTH_PATH`（`:40-42`）は `user_home() / ".pi/agent/…"` を毎回引く関数にする。単独モードでは今と同じパスになる（`PI_SESSIONS_DIR` の環境変数があればそれを優先する）。
- 覚え（キャッシュ）を利用者ごとにする: 前提に挙げた 7 つすべてのキーの頭に「利用者の名前（単独モードは空文字）」を足す（1 つしか持たない物は、利用者の名前をキーにする dict に変える）。テストが覚えを消すための関数（`home.forget()` など）も合わせて直す。

### 3. 利用者・ログイン・個人のトークンの記録 — `apps/web/src/sai_web/store.py`
`_SCHEMA` に 3 つの表を足す（`CREATE TABLE IF NOT EXISTS`）。
```sql
CREATE TABLE IF NOT EXISTS users (
    name TEXT PRIMARY KEY,           -- 短い名前 ^[a-z][a-z0-9]{1,15}$
    email TEXT NOT NULL UNIQUE,      -- 小文字にして保存する。ログインの ID
    password_hash TEXT NOT NULL,     -- "scrypt$<n>$<r>$<p>$<salt b64>$<hash b64>"
    is_admin INTEGER NOT NULL DEFAULT 0,
    must_change_password INTEGER NOT NULL DEFAULT 1,
    disabled INTEGER NOT NULL DEFAULT 0,
    created_at REAL NOT NULL
);
CREATE TABLE IF NOT EXISTS web_sessions (
    token_sha256 TEXT PRIMARY KEY,   -- Cookie の値そのものは保存しない
    user_name TEXT NOT NULL,
    created_at REAL NOT NULL,
    expires_at REAL NOT NULL
);
CREATE TABLE IF NOT EXISTS api_tokens (
    token_sha256 TEXT PRIMARY KEY,   -- 個人のトークン。値そのものは保存しない
    user_name TEXT NOT NULL,
    label TEXT NOT NULL,             -- 例 "sai-prep (MacBook)"
    created_at REAL NOT NULL,
    last_used_at REAL
);
```
関数: `create_user`・`get_user`・`get_user_by_email`・`list_users`・`update_user`（`email`・`is_admin`・`disabled`・`password_hash`・`must_change_password`）・`create_session`・`get_session_user`（期限切れは消して None）・`delete_session`・`delete_sessions_of`・`create_api_token`・`find_api_token`（`last_used_at` を更新する）・`list_api_tokens`・`delete_api_token`。

### 4. 新しいモジュール `apps/web/src/sai_web/auth.py`
- パスワード: `hash_password(pw) -> str` と `verify_password(pw, stored) -> bool`。`hashlib.scrypt(pw.encode(), salt=os.urandom(16), n=2**14, r=8, p=1, maxmem=2**26, dklen=64)`、比較は `hmac.compare_digest`。パスワードは 8 文字以上 128 文字以下。
- 名前の検査: `^[a-z][a-z0-9]{1,15}$`（ハイフンを許さない。プロジェクトのフォルダ名 `<名前>-<プロジェクト>` が一意になるため。C4）。メールアドレスの検査: `^[^@\s]+@[^@\s]+\.[^@\s]+$`、254 文字以下、小文字にして扱う。
- セッション: `secrets.token_urlsafe(32)` を Cookie `sai_session` に入れ、DB には SHA-256 だけを置く。期限 30 日。Cookie は `HttpOnly`・`SameSite=Lax`・`Path=/`。`Secure` は環境変数 `SAI_WEB_COOKIE_SECURE`（既定 `1`。`0` で外す。手元の http の確認用）。
- 個人のトークン: `"sai_" + secrets.token_urlsafe(32)`。作ったときに 1 回だけ値を返し、DB には SHA-256 だけを置く。
- `require_admin()`（依存関数）: 今の利用者が管理者でなければ 403「管理者だけが使えます」。

### 5. 入口の守り — `apps/web/src/sai_web/main.py`
- **ミドルウェア**（`@app.middleware("http")`）を 1 つ足し、多人数モードのときだけ次を行う。依存関数ではなくミドルウェアにするのは、同期の依存関数で `ContextVar` を入れても、別のスレッドで動く同期の住所の関数に届かないため。
  1. `Authorization: Bearer sai_…` があり `store.find_api_token(sha256)` で見つかれば、その利用者を `registry.current_user.set({...})` する（`request.state.via_api_token = True`）。
  2. それが無ければ、Cookie `sai_session` から利用者を引き、`registry.current_user.set({...})`。無効・期限切れ・`disabled` の利用者は「いない」扱い。
  3. 変更系（`POST`・`PUT`・`PATCH`・`DELETE`）で、Cookie で入った要求に `Origin` ヘッダがあり、その host が要求の `Host` と違えば 403「別のサイトからの要求は受けません」（CSRF の対策。個人のトークンで入った要求は見ない）。
  4. 利用者が `must_change_password` のとき、`/api/auth/` で始まらない `/api/` の住所は 403「パスワードを変えてください」。
- `require_token`（`main.py:59`）: 多人数モードでは、`registry.current_user.get()` が None なら 401「ログインしてください」。合言葉（Bearer の合言葉・`?token=`）は多人数モードでは受けない。単独モードは今のまま。`Depends(require_token)` の付いた住所は書き換えない。
- 新しい住所（どれも `require_token` を付けない。多人数モードでないときは 404）:

| 住所 | body | 返り値 |
|---|---|---|
| `POST /api/auth/login` | `{"email","password"}` | 200 `{"name","email","is_admin","must_change_password"}` と Cookie。違えば 401「メールアドレスかパスワードが違います」（どちらが違うかは言わない） |
| `POST /api/auth/logout` | なし | `{"ok":true}`。Cookie を消す |
| `POST /api/auth/password` | `{"current","new"}` | `{"ok":true}`。`must_change_password` を 0 にし、他のセッションを消す。今のパスワードが違えば 400 |

- `GET /api/auth/me`（`require_token` を付けない。両方のモードで動く）: 単独モード `{"mode":"single"}`、多人数モード `{"mode":"multi","name","email","is_admin","must_change_password"}`、未ログインは 401。
- 個人のトークンの住所（`Depends(require_token)`。多人数モードだけ）:

| 住所 | body | すること |
|---|---|---|
| `GET /api/me/tokens` | — | `{"tokens":[{"id": <sha256 の先頭 12 文字>,"label","created_at","last_used_at"}]}`（値は返さない） |
| `POST /api/me/tokens` | `{"label"}` | 作る。`{"id","label","token":"sai_…"}`（値を返すのはこの 1 回だけ）。label は 1〜60 文字 |
| `DELETE /api/me/tokens/{id}` | — | 自分のトークンだけ消せる。無ければ 404 |

- 管理者の住所（`Depends(require_token)` と `Depends(require_admin)`）:

| 住所 | body | すること |
|---|---|---|
| `GET /api/admin/users` | — | `{"users":[{"name","email","is_admin","disabled","must_change_password","created_at"}]}` |
| `POST /api/admin/users` | `{"name","email","password","is_admin"}` | 作る（同じ名前・同じメールは 409）。C4 のフォルダ（`home/.sai`・`projects`・`homes`）を作る（0700） |
| `PATCH /api/admin/users/{name}` | `{"email"?, "is_admin"?, "disabled"?, "password"?}` | 変える。`password` を入れたら `must_change_password=1` にし、その人のセッションを消す。自分自身の `is_admin=false`・`disabled=true` は 400 |

- 多人数モードでは、`FastAPI(title="sai-web", docs_url=None, redoc_url=None, openapi_url=None)` にする（`main.py:54`。`multi_user()` で分ける）。
- 起動（`main.py:112` `_startup`）: 多人数モードでは、合言葉のファイル書き（`write_token_file`）を行わない。

### 6. 最初の管理者を作る CLI — `apps/web/src/sai_web/webcli.py`
- `sai-web user add <name> --email <addr> [--admin]`: パスワードを標準入力の 1 行目から読み（空なら `getpass` で聞く）、`store.create_user` で直接書く（Web が止まっていても使える。Web 自身の DB なので境界の約束に反しない）。§5 の `POST /api/admin/users` と同じフォルダも作る。
- `sai-web user list --json`: 名前・メール・管理者・無効を出す。
- 名前やメールの誤りは終了コード 2、同じ名前・メールは 4。

## 共有契約（他の作業と共有する取り決め。worker は変えない）

**C4 多人数モードの置き場**（U08・U09・U10・U13 も同じ形を使う）
- 多人数モードの合図: 環境変数 `SAI_WEB_USERS_ROOT`（絶対パス。例 `/srv/sai/users`）。以下 `<U>` と書く。
- `<U>/<user>/home/` … その人のひな形の HOME（Web だけが使う。`.sai/`（Web が引くその人の `SAI_HOME`）と `.pi/agent/models.json` など）。コンテナには見せない。
- `<U>/<user>/projects/<user>-<project>/` … プロジェクトの root。登録簿の名前もフォルダ名と同じ `<user>-<project>`。
- `<U>/<user>/homes/<user>-<project>/` … そのプロジェクトのコンテナの HOME（`.claude`・`.claude.json`・`.pi`・`.sai`）。コンテナの中の `SAI_HOME` は `<この HOME>/.sai`。実行の書き出しは `<この HOME>/.sai/web/runs/<user>-<project>/`。
- `<U>/<user>/secrets.env` … その人の API キーと Claude のトークン（0600。U09 が書く）。コンテナには `--env-file` で渡し、ファイルそのものは見せない。
- 利用者の短い名前 `^[a-z][a-z0-9]{1,15}$`、プロジェクトの短い名前 `^[a-z0-9][a-z0-9-]{0,29}$`。
- コンテナに見せる（bind する）のは、そのプロジェクトの root と HOME の 2 つだけ（同じパスに）。

**C5 今の利用者**（U07・U09・U10・U13 が使う）
- `registry.current_user: ContextVar[dict | None]`（値 `{"name": str, "email": str, "is_admin": bool}`）、`registry.users_root()`・`multi_user()`・`user_dir(name=None)`・`user_home(name=None)`・`project_home(project)`・`sai_home()`・`within_user(path)`。
- 多人数モードの認証はミドルウェアで `current_user` を入れる。`require_token` は「入っているか」を見るだけ。U10 は同じミドルウェアに「コンテナの合言葉 → 利用者」を足す。
- 個人のトークン（`Authorization: Bearer sai_…`）でも Cookie と同じ住所を使える（sai-prep が使う）。
- `auth.require_admin` は管理者だけを通す依存関数。

**C5' アカウントの住所**（U07 の画面が使う）: §5 の 3 つの表と `GET /api/auth/me` が全文。

## ファイルの所有権
- 書いてよい: `apps/web/src/sai_web/registry.py`・`auth.py`（新規）・`store.py`・`main.py`・`runs.py`・`usage.py`・`home.py`・`papers.py`・`control.py`（どちらも覚えの部分だけ）・`webcli.py`・`config.py`・`apps/web/tests/test_auth.py`（新規）・`test_multi_user_paths.py`（新規）・`docs/integration.md`
- 読むだけ: `apps/web/src/sai_web/sessions.py`・`questions.py`
- 触らない: `apps/web/src/sai_web/static/`（U07 が書く）・`apps/web/hooks/`・`crates/`

## 完了条件
- [ ] AC1: 単独モード（`SAI_WEB_USERS_ROOT` 無し）で、既存の Web のテスト（JS・Python）がすべて通る。`GET /api/auth/me` が `{"mode":"single"}`。
- [ ] AC2: `apps/web/tests/test_auth.py`（`fastapi.testclient` を使い、`SAI_HOME`・`SAI_WEB_USERS_ROOT` を一時フォルダにする）: `sai-web user add` 相当で管理者を作る → メールアドレスで login → Cookie で `GET /api/projects` が 200、Cookie 無しは 401。メールは大文字が混ざっても通る。初回は `/api/projects` が 403「パスワードを変えてください」、`/api/auth/password` の後は 200。間違ったパスワードは 401。別の Origin からの Cookie の POST は 403。一般の人の `/api/admin/users` は 403。無効にした人の Cookie は 401。個人のトークンを作り、その値の Bearer で `GET /api/projects` が 200、消したあとは 401。DB にパスワード・Cookie・トークンの値がそのまま入っていない。
- [ ] AC3: `apps/web/tests/test_multi_user_paths.py`: 利用者 alice と bob の `projects/` と `homes/<name>/.sai/web/runs/` に別々のプロジェクトと実行を置くと、alice の `GET /api/projects`・`GET /api/days` に bob の物が出ない。`GET /api/runs/<bobのプロジェクト>/<id>` は alice には 404。alice のプロジェクトの書き出しに、`transcript_path` が bob の HOME を指す物を置いても、`/items` は 404。
- [ ] AC4: 同期の住所の関数の中で `registry.current_user.get()` が取れることを、AC2 のテストの中で確かめる（ミドルウェアの値が届くこと）。
- [ ] AC5: `docs/integration.md` に新しい住所（§5 の表）と多人数モードの説明・C4 を足す。

## 検証
```bash
cd ~/workdir/Sai/sai-tree
for f in apps/web/tests/test_*.js; do node "$f" || exit 1; done
for f in apps/web/tests/test_*.py; do PYTHONPATH=apps/web/src apps/web/.venv/bin/python "$f" || exit 1; done
```
手元の確認は AGENTS.md の手順で 8299 番に起こす（`SAI_HOME`・`SAI_WEB_USERS_ROOT` は一時フォルダ、`SAI_WEB_COOKIE_SECURE=0`）。終わったら止めて消す。

## 決定事項と理由
- ログインの ID は大学のメールアドレス、フォルダ名には管理者が決める短い名前 — 理由: 利用者の決定。メールアドレスはフォルダ名に向かない（`@`・`.`）。
- アカウントは管理者が作り、初回にパスワードを変えさせる — 理由: 研究室の数人に配るだけなので、招待コードや自己登録は要らない。
- パスワードは標準ライブラリの `hashlib.scrypt` — 理由: 依存を増やさない（AGENTS.md）。
- 利用者の区別は `ContextVar` と置き場の関数の差し替えで行う — 理由: 置き場を引く関数がまとまっているので、上の処理（Projects・Chats・Papers・Research・Home）を書き換えずに分けられる。/ 採用しなかった案: すべての関数に利用者の引数を足す（差分が大きく、渡し忘れが情報の漏れになる）。
- 多人数モードの登録簿はフォルダを数える — 理由: プロジェクトは Web だけが作る（U09）。コンテナの中のエージェントが登録簿を書き換えても、Web の一覧は変わらない。
- 個人のトークンを足す — 理由: sai-prep は Cookie を持たない。値は 1 回だけ見せ、DB には SHA-256 だけを置く。
- CSRF は `SameSite=Lax` と `Origin` の照合で防ぐ — 理由: 画面は同じ住所からしか呼ばない。

## 要確認
- なし（ログインの試行回数の制限は付けない。tailnet の中だけで使うため。必要になったら足す）。
