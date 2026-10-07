---
block: explore
run: 408
description: 作業報告書 WEB-MULTIUSER: Linux 多人数 Web サーバ化のための現状調査（認証の外・登録簿・SAI_HOME/HOME 依存・API キー・macOS 依存・papr.db・radio/MLflow 区別）
---

# 作業報告書 WEB-MULTIUSER: Linux 多人数 Web サーバ化のための現状調査

対象: `~/workdir/Sai/sai-moon`（作業時点の HEAD）。コードは変更していない。
読み方は「事実」＝コードで確認したこと（`path:line` 付き）、「推測」＝明記したうえでの見込み、「要確認」＝決められなかった点。
行番号は 1 始まり。`~` は実行ユーザーのホーム。断りの無い `path:line` はリポジトリ相対。

---

## Q1. 今の認証（`SAI_WEB_TOKEN`・`require_token`・`require_local`・`web/token`）と、認証の外にある API

### 仕組み（事実）

- 合言葉の設定値: `Settings.api_token = os.getenv("SAI_WEB_TOKEN") or None` — `apps/web/src/sai_web/config.py:34`。未設定なら `None`（＝認証なし・開発用）。
- `require_token` — `apps/web/src/sai_web/main.py:59-72`。`HTTPBearer(auto_error=False)` の Bearer ヘッダの値か、クエリ `?token=` が `settings.api_token` と一致すれば通す。`api_token` が空なら無条件で通す（`main.py:68-69`）。クエリを受ける理由は iPhone でヘッダを付けられない場面のため（`main.py:64-67`）。
- `require_local` — `apps/web/src/sai_web/main.py:75-79`。`request.client.host` が `127.0.0.1` か `::1` のときだけ通す。X-Forwarded-For は見ていない。
- `web/token` の書き込み: `write_token_file()` — `apps/web/src/sai_web/main.py:82-108`。起動時（`@app.on_event("startup")` の `main.py:118`）に `<sai_home>/web/token` へ 0600 で書く。`api_token` が無いときは既存ファイルを削除する（`main.py:94-97`）。書けなくてもサーバは止めない。
- 読み手: `apps/web/src/sai_web/webcli.py:67-80`（`api_token()` は `SAI_WEB_TOKEN` を先に、無ければ `<sai_home>/web/token` を読む）。テストは `apps/web/tests/test_token_file.py`。
- 注: `main.py:85` と `webcli.py:12` のコメントが読む相手として挙げる `sai-sci/tools/phone_api.py` は、このリポジトリには存在しない（`find` で 0 件。参照はコメントと `.sai/outputs/*` のみ）。現存の読み手は `webcli.py`。

### `require_token` が付いていない API（＝認証の外。事実）

`apps/web/src/sai_web/main.py` のルートを全部数えた結果、`Depends(require_token)` が無いのは次のとおり。

| ルート | 場所 | 守り |
|---|---|---|
| `GET /api/health` | `main.py:156` | なし（誰でも） |
| `GET /api/questions/{qid}` | `main.py:340` | **なし**（`require_local` も無い） |
| `POST /api/register` | `main.py:256` | `require_local` のみ（`main.py:258`） |
| `POST /api/questions` | `main.py:278` | `require_local` のみ（`main.py:280`） |
| `POST /api/questions/{qid}/resolved` | `main.py:303` | `require_local` のみ（`main.py:307`） |
| `POST /api/approvals` | `main.py:387` | `require_local` のみ（`main.py:389`） |
| 静的ファイル `app.mount("/")` | `main.py:1453` | **なし**（`index.html`・`app.js`・`style.css`・`sw.js` 等が誰でも取れる） |
| `/docs`・`/openapi.json`（FastAPI 既定） | `main.py:54` | **なし**（`docs_url`/`openapi_url` を無効化していない。推測: 既定で公開） |

- `GET /api/questions/{qid}` は「トークン不要の内部専用」とコメントされているが、`require_local` を呼んでいない（`main.py:340-345` に `request: Request` も無い）。質問の本文（ツール入力）を返す。`qid` は `uuid4().hex[:12]`（`apps/web/src/sai_web/store.py:128`）なので総当たりは現実的でない（推測）。
- 静的文件の mount の直前コメントは「認証は require_token で同じく守る」（`main.py:1450`）だが、`StaticFiles` に認証は掛かっていない（事実）。`/manifest.json` だけは個別に `require_token` 付き（`main.py:166-168`）。
- `_revalidate_static` ミドルウェア（`main.py:1456-1466`）は Cache-Control を付けるだけで、認証は行わない。

### 多人数 Linux サーバでの注意（推測）

- `require_local` はピア IP しか見ない。リバースプロキシ（nginx 等）を同一ホストで前段に置くと、全リクエストの `request.client.host` が `127.0.0.1` になり、`require_local` が全リクエストを通す。多人数サーバでは同一ホストの他ユーザーも `127.0.0.1` になるため、「Mac 自身から」の前提が崩れる（推測）。
- 認証の外にある `POST /api/register`・`POST /api/questions`・`POST /api/approvals`・`POST /api/questions/{qid}/resolved`・`GET /api/questions/{qid}` は、この前提の上でだけ安全である（推測）。

---

## Q2. プロジェクトの作成と登録（registry.json、`sai-link init`・`project`、Web からの作成、root の位置）

### registry.json の形式（事実）

- 置き場: `<SAI_HOME>/registry.json`（`SAI_HOME` が無ければ `~/.sai/registry.json`）— `apps/web/src/sai_web/registry.py:38-48`、`crates/sai-link-engine/src/phone_export.rs:286`。
- 形: JSON の**配列**。1 要素は次の項目（`phone_export.rs:264-276`）:
  ```json
  { "name": "…", "root": "/abs/path", "kind": "link", "color": null,
    "created_at": "RFC3339", "last_opened": "RFC3339" }
  ```
- 書き込みは `register_project()`（`phone_export.rs:237-281`）のみ。同じ `root` が既にあれば `AlreadyRegistered`（何もしない）、同じ `name` が別 `root` で使われていればエラー（名前が一意でないと Web が名前で引けないため。`phone_export.rs:258-263`）。
- 読み込みは `apps/web/src/sai_web/registry.py:95-133`（`load_registry`）。`kind == "link"` だけ返し、`last_opened` の降順。各項目に `root_exists`・`db_exists`（`<root>/.sai/sai-engine/sai-engine.db`）・`daemon_alive`（`<root>/.sai/sai-engine/daemon.pid`）を足す（`registry.py:110-130`）。

### CLI の命令（事実）

- `sai-link init [--name NAME] [--update]` — `crates/sai-link-cli/src/cli.rs:78-88`（定義）、`cli.rs:2054-2127`（`handle_init`）。
  - 作る物: `<root>/.sai/` 配下（`blocks/`・`template/`・`outputs/*`・`sai-engine/notifications/*`・`settings.json`）と、cc/pi の設定 `.mcp.json`・`.claude/settings.json`・`.pi/settings.json`・`.serena/project.yml`（`crates/sai-link-engine/src/init.rs:1-50`、`cli.rs:2062`）。**root フォルダ自体は作らない**（既にある前提。`init.rs` の作成物は `<root>/.sai/…` 等）。
  - 登録名: `--name`、無ければ root のフォルダ名（`cli.rs:2095-2100`）。
  - 登録: `register_project`（`cli.rs:2102-2117`）。
- `sai-link project` は `summary` と `dashboard` の 2 つだけ（`crates/sai-link-cli/src/cmd_project.rs:11-21`、`cli.rs:627-642`）。**`project list` / `create` / `remove` は無い**。
- 登録簿から外す命令は無い（`register_project` が唯一の書き込み。`phone_export.rs:237` 以外に `registry.json` を書くコードは無い）。手で編集する手順が `sai-sci/TUTORIAL.md:477-483` にある。

### Web からプロジェクトを作れるか（事実: 作れない）

- `GET /api/projects` は一覧のみ（`apps/web/src/sai_web/main.py:559-563`、`control.list_projects` — `control.py:102-131`）。作成のルートは無い。
- Web の「コマンド」画面は CLI の `schema --json` を読み、`web: true` の命令だけ実行する（`apps/web/src/sai_web/commands.py:105-118`, `296-302`）。`init` は `web` 対象外に明示されている（`crates/sai-link-cli/src/cmd_schema.rs:179-196` の `LOWER_LEVEL` に `"init"`）。したがって **Web からはプロジェクトを作れない**。
- Web の一覧は既存プロジェクトの root を絶対パスで持つ（`registry.py:110-130`）。Papers タブは `<root>/.sai/paper/papr.db` があるプロジェクトだけを対象にする（`papers.py:58-95`）。

### プロジェクトの root はどこに置かれるか（事実）

- 位置は決まっていない。`SAI_PROJECT_ROOT`（空でなければそれをそのまま）か、カレントディレクトリ（`crates/sai-link-engine/src/project.rs:49-58`、`cli.rs:2060-2062`）。
- すなわち人間が任意のフォルダを作り、そこで `sai-link init` を実行する。`init` は root を作らない。`SAI_PROJECT_ROOT` があるのにフォルダが無ければ `project_root_checked` はエラーにする（`project.rs:59-73`）。
- Web/CLI は登録簿の `root` を絶対パスとして扱う（`registry.py:110`、`control.py:102-131`）。

---

## Q3. `SAI_HOME` と `~/.sai` の中身・HOME に依存するもの

### `SAI_HOME` の解決（事実）

`SAI_HOME`（空でない値）→ `HOME/.sai`。両方の綴りが一致していること（AGENTS.md の約束）。
- `crates/sai-link-engine/src/project.rs:146-166`（`sai_home`）
- `crates/sai-paper-core/src/paths.rs:281-305`（`sai_home`、同規則）
- `apps/web/src/sai_web/registry.py:41-43`（`sai_home`）
- `sai-sci/tools/exp_exec.py:63-66`、`sai-sci/tools/refcode.py:110-113`、`sai-sci/tools/modeldb.py:102-104`

### `~/.sai` に置かれるもの（事実）

`README.md:152-160` の表と、コードの実際の参照先:

| 物 | 何 | 根拠 |
|---|---|---|
| `registry.json` | プロジェクト登録簿 | `registry.py:38-48`、`phone_export.rs:286` |
| `blocks/` | 全プロジェクト共通の block 定義 | `crates/sai-link-engine/src/common.rs:72`、`orchestrator.rs:2885` |
| `template/` | 共通 template | `crates/sai-link-engine/src/parser.rs:168` |
| `settings.json` | 共通 settings（プロジェクトの `.sai/settings.json` を優先マージ） | `crates/sai-link-engine/src/pi_command.rs:122-156`（プロジェクト→グローバル優先、`151` が global） |
| `paper.env` | 論文の API キー（全プロジェクト共通） | `crates/sai-paper-core/src/paths.rs:114` |
| `cc-cost/` | cc の費用ファイル | `crates/sai-link-engine/src/phone_export.rs:874` |
| `web/cc-rate-limits.json` | cc の 5 時間枠 | `phone_export.rs:877-885`、`apps/web/src/sai_web/usage.py:36-37` |
| `web/runs/` | 実行の書き出し | `apps/web/src/sai_web/config.py:54-56` |
| `web/token` | 合言葉（0600） | `main.py:93`、`webcli.py:76` |
| `web/sai_web.db` | Web の DB（質問・承認・登録・購読） | `config.py:26`、`store.py` |
| `web/vapid.json` | 通知の鍵 | `apps/web/src/sai_web/notify.py:23` |
| `web/sai-web.log` | ログ（launchd の plist 指定） | `apps/web/launchd/com.sai.web.plist.template` |
| `web/browser-profile`・`web/browser-downloads` | 遠隔ブラウザの Chrome プロファイル・DL | `apps/web/src/sai_web/rbrowser.py:82,86` |
| `sci/types/` | 研究方針の型（全体共通） | `crates/sai-sci-core/src/rt/defs.rs:524-526` |
| `refcode/` | 参照コードの取得物 | `sai-sci/tools/refcode.py:123` |
| `exec/<target>/`・`exec/jobs/` | 実行先の設定と仕事の対応 | `sai-sci/tools/exp_exec.py:76,133` |

### HOME に依存するもの（`~/.sai` 以外。事実）

| パス | 用途 | 根拠 |
|---|---|---|
| `~/.claude/projects` | cc のセッション JSONL | `apps/web/src/sai_web/config.py:37`、`crates/sai-link-engine/src/session.rs:1852,2236` |
| `~/.pi/agent/sessions` | pi のセッション記録 | `config.py:41` |
| `~/.pi/agent/models.json` | pi のモデル一覧 | `crates/sai-link-engine/src/common.rs:168-183`、`apps/web/src/sai_web/usage.py:41` |
| `~/.pi/agent/auth.json` | pi の認証 | `usage.py:42` |
| `~/.local/bin/herdr`（`shutil.which("herdr")`） | herdr の実行ファイル | `config.py:46-50` |
| `~/Downloads` | 論文の取り込み元（`sai-paper init` が `import_folders`/`download_path` に入れる） | `crates/sai-paper-core/src/manual_import.rs:88-101`、`crates/sai-paper-core/src/config.rs:840-841` |
| `~/.config/user-dirs.dirs` | `XDG_DOWNLOAD_DIR` の解決 | `manual_import.rs:96,114-` |
| `~/.sai-gemini/sessions` | gl（sai-gemini）のセッション | `session.rs:2104` |
| `HOME`（`~` 展開） | サブタスクの cwd 指定 | `crates/sai-link-engine/src/task_cwd.rs:20` |
| `HOME`（表示） | `~/…` の表示 | `crates/sai-paper-cli/src/phone_cite.rs:239` |

- `papr.db` は HOME に依存しない。プロジェクト内 `<root>/.sai/paper/papr.db`（Q6 参照）。
- `SAI_PROJECT_ROOT` は対象プロジェクトの root。無ければ cwd から上へ `.sai/` を探す（ホームは数えない）— `project.rs:59-110`、`paths.rs:29-66`。

### ユーザーごとに分ける必要がありそうなもの（推測を含む）

以下は現状すべて「プロセスが動いている 1 ユーザー」前提で共有されている。多人数 Linux では分離を検討する必要がある（推測）。

- `~/.sai/` 全体（とくに `registry.json`・`web/token`・`web/sai_web.db`・`web/vapid.json`・`cc-cost/`・`web/cc-rate-limits.json`・`blocks/`・`template/`・`settings.json`・`paper.env`）。
- `~/.claude`・`~/.pi`・`~/.sai-gemini`（エージェントの記録と認証）。
- `~/Downloads`・`~/.config/user-dirs.dirs`。
- 遠隔ブラウザの Chrome プロファイル（`<SAI_HOME>/web/browser-profile`。コメントにも「Chrome は全体で 1 つだけ」`rbrowser.py:6`）。
- プロジェクトの root と `<root>/.sai/`（同一 root を複数ユーザーが共有する前提か要確認。`registry.json` は 1 利用者に 1 つ）。
- `apps/radio` の DB・音声（リポジトリ内の固定パス。ユーザー/プロジェクト分離なし。Q7）。
- MLflow（単一サーバ・単一 Basic 認証ユーザー。Q7）。

---

## Q4. API キーを読む場所の全一覧と環境変数名

### Rust（`crates/`）

`sai-paper-core` の設定（`config.rs`）は「キー本体ではなく、キーが入った環境変数の名前」を持つ。

| 環境変数名 | 用途 | 根拠 |
|---|---|---|
| `SEMANTICSCHOLAR_API_KEY`（既定） | Semantic Scholar（discover.s2_api_key_env） | `crates/sai-paper-core/src/config.rs:134,146,866-875` |
| `S2_API_KEY`・`SCHOLAR_API_KEY` | 上の代替（探索順は `SEMANTICSCHOLAR_API_KEY, S2_API_KEY, SCHOLAR_API_KEY`。設定値が `SCHOLAR_API_KEY` のときだけ順が変わる） | `config.rs:644-649,880-882` |
| `OPENALEX_API_KEY`（既定） | OpenAlex（discover.openalex_api_key_env） | `config.rs:136,147,884-887` |
| `OPENROUTER_API_KEY`（既定） | 要約・evidence（`config.rs:364,379`）、RAG 埋め込み（`config.rs:399,529`） | 同左 |
| `translate.pdf.api_key_env`（既定は空） | PDF 翻訳。空なら vLLM 用のダミー `EMPTY` | `config.rs:192,204`、`crates/sai-paper-core/src/translation/pdf2zh.rs:100-110` |
| `translate.abstract.api_key_env`（既定は空。`service=="openailiked"` のときだけ警告対象） | 要旨翻訳 | `config.rs:904`、`crates/sai-paper-core/src/translation/mod.rs:230-256` |
| `SAI_RADIO_TOKEN`（既定） | 音声化の依頼先（radio.token_env） | `config.rs:234,246` |
| `NCBI_API_KEY` | PubMed | `crates/sai-paper-core/src/api/mod.rs:252`、`crates/sai-paper-core/src/api/pubmed.rs:4,581` |
| `NCBI_API_KEY` | 引用チェック | `crates/sai-sci-core/src/citation_check.rs:72` |

- 環境変数を探す本体: `config.rs:652-660`（`first_env_value`）。値が空白だけなら無い扱い。
- 設定が使う変数で値が無いものを列挙する `missing_env_vars()` — `config.rs:890-910`（`translate.pdf.api_key_env`・`rag.embed_api_key_env`・`discover.openalex_api_key_env`・`translate.abstract.api_key_env`・`radio.token_env`）。
- `paper.env` のキーチェーン記法（`export NAME=$(security find-generic-password …)`）は Rust が `/usr/bin/security` を直接呼んで展開する — `crates/sai-paper-core/src/paths.rs:114-150,248-263`（macOS のみ。Q5）。

### Python（`apps/web`・`apps/radio`）

| 環境変数名 | 用途 | 根拠 |
|---|---|---|
| `OPENROUTER_API_KEY`（既定） | Usage タブの OpenRouter アカウント。`SAI_WEB_OPENROUTER_ACCOUNTS`（`名前=環境変数名` のカンマ区切り）で複数に分けられる | `apps/web/src/sai_web/usage.py:54-71` |
| `DEEPSEEK_API_KEY` | Usage タブの DeepSeek 残高 | `usage.py:161` |
| `SAI_RADIO_TOKEN` | radio への中継の Bearer | `apps/web/src/sai_web/config.py:70`、`radio.py:105-106` |
| `SAI_MLFLOW_USER` / `SAI_MLFLOW_PASSWORD` | MLflow の Basic 認証 | `config.py:74-75`、`mlflow_proxy.py:161-163` |
| `GEMINI_API_KEY` | radio の質問機能（Live API のトークン発行） | `apps/radio/src/sai_radio/config.py:56`、`apps/radio/src/sai_radio/live.py:53` |
| `OPENROUTER_API_KEY` | radio の LLM（**コードでは読まない**。`esperanto` ライブラリが自動で読む） | `apps/radio/README.md:58,274,383`、`apps/radio/ARCHITECTURE.md:274,383` |

### `sai-sci/tools`・`sai-sci/exec`

| 環境変数名 | 用途 | 根拠 |
|---|---|---|
| `GITHUB_TOKEN` | GitHub 検索の認証（任意） | `sai-sci/tools/refcode.py:373-375` |
| `MLFLOW_TRACKING_URI` | MLflow の接続先（キーではない） | `sai-sci/tools/data_result.py:552`、`sai-sci/tools/exp_summary.py:357` |
| `EXP_ARTIFACT_LOCATION` | MLflow 成果物の置き場 | `data_result.py:579`、`sai-sci/template/exp-core.md:80,187` |
| `MLFLOW_TRACKING_USERNAME` / `MLFLOW_TRACKING_PASSWORD` | MLflow の認証（実行先へ渡す） | `sai-sci/exec/dok/dok.py:240-241` |
| `SAKURA_API_ACCESS_TOKEN` / `SAKURA_API_SECRET_KEY` | さくら DOK の API 認証 | `sai-sci/exec/dok/dok.py:74-84,208-217` |
| `SAKURA_S3_ACCESS_KEY` / `SAKURA_S3_SECRET_KEY` | さくら S3（`AWS_ACCESS_KEY_ID`/`AWS_SECRET_ACCESS_KEY` へ写す） | `dok.py:76-77,233-235` |
| `MLFLOW_BASIC_USER` / `MLFLOW_BASIC_PASSWORD` | MLflow Basic 認証（実行先へ渡す） | `dok.py:78-79,240-241` |
| `DOK_REGISTRY_PASSWORD` | レジストリのパスワード（環境変数のみ。`.env` からは読まない） | `dok.py:80,84` |
| `.env`・`mlflow-server/.env` | 上の秘密を読むファイル（既定） | `dok.py:179-198`（`load_secrets`）、`1058-1059`（`default_env_files`） |
| `target.toml` の `[env]` | 実行先ごとの環境変数 | `sai-sci/tools/exp_exec.py:102-121` |

- 補足（事実）: `apps/web/.env.example` と `apps/radio/.env.example` にキー名の見本がある。`apps/web/src/sai_web/config.py:16-17` が `apps/web/.env` を、`apps/radio/src/sai_radio/config.py:16-17` が `apps/radio/.env` を `load_dotenv` する。

---

## Q5. macOS 依存の箇所の全一覧（Linux で動かない・意味をなさないもの）

### macOS でしか動かないもの（事実）

| 箇所 | 何 | 根拠 |
|---|---|---|
| `/usr/bin/security` の呼び出し | `paper.env` のキーチェーン記法を展開 | `crates/sai-paper-core/src/paths.rs:248-263`。`cfg!(target_os = "macos")` でないと即 `None`（`paths.rs:249`） |
| `pdf_viewer` の既定値 | macOS は `"open"`、Windows は `"cmd /C start msedge \"\""`、その他は `"xdg-open"` | `crates/sai-paper-core/src/config.rs:827-833` |
| Chrome の絶対パス | `DEFAULT_CHROME = "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"` | `apps/web/src/sai_web/rbrowser.py:47`（`SAI_WEB_CHROME` で差し替え可。`rbrowser.py:213-214`） |
| Chrome の UA | `Mozilla/5.0 (Macintosh; …)` 固定 | `rbrowser.py:49-51`（見た目のみ。動作には影響しないと推測） |
| launchd | `apps/web/launchd/com.sai.web.plist.template`（`launchctl load`・`~/Library/LaunchAgents`）。PATH に `/opt/homebrew/bin` を決め打ち | 同ファイル。AGENTS.md は「`launchctl load`/`unload` は実行しない」 |

### macOS の前提が文中に残っているもの（コードではない。事実）

- ポート 5000 は macOS の AirPlay が使うので MLflow は 5001 — `apps/mlflow-server/README.md:72,331`、`apps/mlflow-server/AGENT-SETUP.md:33,143`、`apps/mlflow-server/templates/mlflow-agent.md:129`、`apps/mlflow-server/docker-compose.yml`（`127.0.0.1:5001:5000` のコメント）。
- 手順書の `open -a Docker`・`open -a Tailscale` — `sai-sci/exec/dok/SETUP.md:158,170`（文書のみ）。
- Safari 向けの説明（音声の `Range` ヘッダ中継）— `apps/web/src/sai_web/radio.py:8`、`apps/radio/src/sai_radio/main.py:26-28`（機能自体は OS 非依存）。
- `~/Downloads` の決め打ち — `crates/sai-paper-core/src/manual_import.rs:101`（Linux でも通るが、locale によっては `XDG_DOWNLOAD_DIR` を見る）。

### 探したが**コードには無かった**もの（事実: リポジトリ全体 grep で 0 件）

- `osascript`・`pbcopy`/`pbpaste`・`say`・`afplay`・`notifyutil`・`terminal-notifier` — いずれもコードには無い。通知は `pywebpush`（`apps/web/pyproject.toml`、`apps/web/src/sai_web/notify.py`）、音声合成は AivisSpeech Engine の HTTP（`apps/radio/src/sai_radio/config.py:66-68`、既定 `http://127.0.0.1:10101`）と Gemini Live。
- `platform.system()` / `sys.platform` による分岐は無い。`cfg!(target_os=…)` は `config.rs:827,829`・`paths.rs:249` の 3 か所だけ。

### Linux で動かない・用意が要るもの（推測）

- `herdr`（`config.py:46-50`、`shutil.which("herdr")` → `~/.local/bin/herdr`）。herdr 本体はこのリポジトリの外の道具（`apps/web/src/sai_web/herdr.py:1-25`）。Linux に無ければ質問への回答・実行の継続が動かない（推測）。
- 遠隔ブラウザは Chrome の場所を `SAI_WEB_CHROME` で Linux のパスに変える必要がある（推測）。
- AivisSpeech Engine を Linux で別途起動する必要（推測）。
- macOS キーチェーン記法の `paper.env` 行は Linux では展開されない（`paths.rs:249` で `None`）。Linux では `paper.env` に値を直書きする必要がある（推測）。

---

## Q6. `papr.db` は利用者全体で 1 つか、プロジェクトごとか（Library の共有のされ方）

- **プロジェクトごとに 1 つ**（事実）。置き場は `<root>/.sai/paper/papr.db`。
  - `crates/sai-paper-core/src/paths.rs:76-95`（`PaperRoots` の説明と `paper_roots()`。`data_dir = <root>/.sai/paper`）
  - `apps/web/src/sai_web/papers.py:47-49`（`_PAPER_DB_REL = .sai/paper/papr.db`）
  - `crates/sai-paper-core/src/downloads.rs:260`（`data_dir.join("papr.db")`）
- Library（`papers` 表など）も研究方針の木（`sci_*` 表）も**この 1 つの DB の中**にある。`sai-sci/README.md:8,16,249,440` が正は `papr.db` と述べている。
- 利用者全体で共有する `papr.db` は**無い**（`SAI_HOME` 配下に `papr.db` を作るコードは無い。grep で該当なし）。
- Web の Papers タブは、登録簿のうち `<root>/.sai/paper/papr.db` が存在するプロジェクトだけを候補にする（`papers.py:58-95`）。
- したがって、**Library の共有単位はプロジェクト（root）**。同じ root を複数ユーザーが開けば同じ Library を共有し、別 root なら別 Library（事実からの帰結）。プロジェクト root 自体の置き場に決まりは無い（Q2）。

---

## Q7. `apps/radio`・MLflow がユーザーやプロジェクトをどう区別しているか

### `apps/radio`（事実: 区別しない）

- DB は `apps/radio/src/data/sai_radio.db`、音声は `apps/radio/src/data/audio/`（リポジトリ内の固定パス）— `apps/radio/src/sai_radio/config.py:19-21`。
- 認証は単一の `SAI_RADIO_TOKEN`（`config.py:72`、`main.py:35-49`）。ユーザーの概念は無い。
- プロジェクトの概念も無い。区別は `paper_id` の**名前の付け方**だけ: `<プロジェクト root のフォルダ名>-<論文番号>`（`apps/web/src/sai_web/papers.py:508-513` の `radio_id_for`）。`apps/radio` 側のソースに `project` という項目は無い（`apps/radio/src/sai_radio/*.py` を grep して 0 件）。
- Web は radio への中継で、利用者の `?token=` を radio に送らず、`SAI_RADIO_TOKEN` を Bearer で付ける（`apps/web/src/sai_web/radio.py:56-57,105-106`）。
- 帰結（推測）: 別プロジェクトで同じフォルダ名（例 `west`）を使うと同じ `radio_id` になり、音声が衝突し得る。

### MLflow（事実: 単一サーバ・単一ユーザー。プロジェクトはタグで区別）

- 単一の docker compose（`apps/mlflow-server/docker-compose.yml`、`name: mlflow-server`）。volume は `db_store`・`minio_store`・`caddy_data` の 3 つで、利用者ごとの volume 分けは無い。
- 認証は Caddy の Basic 認証 1 組だけ（`MLFLOW_BASIC_USER` / `MLFLOW_BASIC_PASSWORD_HASH`）— `apps/mlflow-server/caddy/Caddyfile`、`.env.example`。ユーザーの概念は無い。
- プロジェクトの区別は experiment/run のタグ `sai.project` = **プロジェクト root のフォルダ名**。
  - 記録側: `sai-sci/tools/data_result.py:570,586`（`--project` は必須）、`sai-sci/template/exp-core.md:100`（「プロジェクトのフォルダ名」）、`sai-sci/README.md:397`。
  - 表示側: `apps/web/src/sai_web/mlflow_proxy.py:202-211`（`_project_filter` が `tags."sai.project" = "<root のフォルダ名>"` を組み立てる）、`mlflow_proxy.py:217-225`、`apps/web/src/sai_web/main.py:1340-1346`。
- Web は読み取りだけを中継する（`mlflow_proxy.py:1-17`）。`SAI_MLFLOW_USER`/`SAI_MLFLOW_PASSWORD` が空なら認証ヘッダを付けない（`mlflow_proxy.py:161-163`）。
- 帰結（推測）: 別プロジェクトで同じフォルダ名を使うと、`sai.project` タグが同じになり実験が混ざる。

---

## 分からなかったこと・要確認

- `sai-sci/tools/phone_api.py`（`main.py:85`・`webcli.py:12` がコメントで参照）がこのリポジトリに存在しない。合言葉のファイルを読む別の配布物があるのか、古い名前なのか不明。要確認。
- 多人数運用で「誰のどのプロジェクトか」を表す識別子が今のデータ構造に無い（`registry.json` は 1 利用者 1 ファイル、radio/MLflow はフォルダ名依存）。分離単位（Linux ユーザーごとに `SAI_HOME` を分けるのか、1 ユーザー内で複数テナントを分けるのか）が決まらないと、radio/MLflow の衝突回避の設計が決められない。要確認。
- リバースプロキシを前段に置く前提かどうかで、`require_local`（`main.py:75-79`）の扱いが変わる。前提の有無はコードからは決められない。要確認。
- `apps/radio` の AivisSpeech Engine・herdr・Chrome を Linux でどう用意するか（どのディストリ・どのパッケージ）はこのリポジトリの外の話で、コードからは決められない。要確認。

