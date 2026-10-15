# 実装計画書 U06: Web（apps/web）の名前を agent-link に変える

## 目的
Web のパッケージ・命令・環境変数・置き場・画面の名前を、共有契約 C2 の対応表どおりに agent-link に変える。`agent-link-web serve` で起き、`agent-link` の CLI を呼ぶ状態にする。

## 背景
- agent-link は sai-tree を複製した開発用の道具である。開発するアプリ（sai-tree）と名前が混ざらないように、外から見える名前を全部変える（要件定義書 `docs/plan/requirements.md`）。
- 古いデータ（`~/.sai`）は移さない。古い名前を読む互換の処理は作らない。
- Web は sai-link の CLI の JSON だけを読む（DB を開かない）。CLI の名前と、CLI と共有する置き場（`<home>/web/runs`・`<home>/web/token`・プロジェクトの `.agent-link/engine/` の `daemon.pid`・`daemon.status`）は、U05 と同じ物を使う。

## 前提（着手時点のコードの状態）
- ガイド 1（U01〜U04）が終わっている。Web は単独モードの sai-link の窓口だけになっている。画面の版は `sw.js` の `CACHE = "sai-web-v132"`・`index.html` の `?v=132`。
- 名前を決めている主な所（行番号は複製の元の値。ずれていたら名前で探す）:
  - `apps/web/src/sai_web/registry.py:37-40` の `_DB_REL`・`_PID_REL`・`_STATUS_REL`（`Path(".sai") / "sai-engine" / …`）、`:134-143` `sai_home()`（`SAI_HOME` → `~/.sai`）。
  - `apps/web/src/sai_web/config.py:24-25`（`DATA_DIR = registry.sai_home() / "web"`・`DB_PATH = DATA_DIR / "sai_web.db"`）、`:48`（`sai_link_bin = os.getenv("SAI_LINK_BIN", "sai-link")`）。
  - `apps/web/src/sai_web/webcli.py:66`（`SAI_WEB_URL`）、`api_token()`（`SAI_WEB_TOKEN`・`<home>/web/token`）。
  - `apps/web/bin/sai-web:7`（`-m sai_web.webcli`）、`apps/web/pyproject.toml:2`（`name = "sai-web"`）。
  - `apps/web/src/sai_web/static/index.html:6`（`<title>sai</title>`）、`manifest.json:2-3`、`sw.js:125`（`CACHE`）・`:199-202`（`swIsOldCache`）。
  - `apps/web/tests/test_migrate.js:63-65`（古い倉庫の判定の検査）。
  - `apps/web/hooks/cc_question.py`・`cc_register.py`（Web の住所の環境変数 `SAI_WEB_URL` など）。
- 環境変数（ガイド 1 のあとに残る物）: `SAI_PROJECT_ROOT`・`SAI_PARENT_RUN_ID`・`SAI_LINK_BIN`・`SAI_HOME`・`SAI_WEB_TOKEN`・`SAI_WEB_NO_WATCH`・`SAI_WEB_PORT`・`SAI_WEB_URL`・`SAI_WEB_RUNS_DIR`・`SAI_WEB_OPENROUTER_ACCOUNTS`。`SAI_ORIGIN` を CLI に渡している所（`control.py:252` など）が残っていたら消す（sai-link は読まない）。
- 同じガイドの U05（Rust）と並列で動く。U05 は `crates/**` だけを書く。

## 変更内容

### 1. パッケージと命令
- `git mv apps/web/src/sai_web apps/web/src/agent_link_web`。コードとテストの `sai_web` の import を `agent_link_web` に変える。
- `git mv apps/web/bin/sai-web apps/web/bin/agent-link-web`。中身を `-m agent_link_web.webcli` にし、頭の注記を直す。
- `pyproject.toml` の `name` を `agent-link-web` にする。`uv lock` で `uv.lock` を作り直す（依存は変えない）。
- uvicorn の起動の文字列（`sai_web.main:app`）を `agent_link_web.main:app` にする（`webcli.py` の `serve` など）。
- `FastAPI(title="sai-web")` を `FastAPI(title="agent-link-web")` にする。
- `webcli.py` の argparse の `prog` と使い方の文の `sai-web` を `agent-link-web` にする。

### 2. 環境変数・置き場・CLI の名前
- `\bSAI_` で始まる名前を全部 `AGENT_LINK_` に変える（Python・JS・テスト・`.env.example`）。例外は `SAI_LINK_BIN` → `AGENT_LINK_BIN` だけ。JS の定数 `SAI_WIDE_PX`・`SAI_TERMS` も `AGENT_LINK_WIDE_PX`・`AGENT_LINK_TERMS` になる（`test_globals.js` の一覧も合わせる）。
- `registry.py`: プロジェクトのフォルダを `Path(".agent-link") / "engine" / …`、DB を `agent-link.db` にする。`sai_home()` は `AGENT_LINK_HOME` → `~/.agent-link` を返す（関数名は変えない）。
- `config.py`: `DB_PATH = DATA_DIR / "agent_link_web.db"`、`sai_link_bin` の既定を `"agent-link"`、環境変数を `AGENT_LINK_BIN` にする。
- Web のログのファイル名（`sai-web.log`）を作っている所があれば `agent-link-web.log` にする。
- `commands.py` の `_CLI_LABELS` と `sessions.py` の `_CLI_NAMES`・`_CLI_SUBWORDS` の `sai-link`・`sai-web` を `agent-link`・`agent-link-web` にする。
- 画面の文・エラーの文・hint・`saiTerm` の説明文に出る `sai-link`・`sai-web`・`.sai/` を新しい名前にする。アプリの名前としての「sai」も `agent-link` にする。
- `hooks/cc_question.py`・`cc_register.py` の環境変数と注記を新しい名前にする。

### 3. 画面の名前と版
- `index.html` の `<title>` と `manifest.json` の `name`・`short_name` を `agent-link` にする。
- `sw.js` の `CACHE` を `"agent-link-web-v133"` に、`index.html` の `?v=` を全部 `133` にする。版の履歴の注記に 1 行足す。
- `swIsOldCache(name, current)` は、`sai-phone-` で始まる名前、`sai-web-v` で始まる名前、`agent-link-web-v` で始まり `current` と違う名前を、古いと判定する。Mac の 8200 番は同じ住所のまま sai-moon から agent-link に替わるので、iPhone に残った sai-moon の倉庫もこれで消える。
  ```js
  function swIsOldCache(name, current) {
    if (name.startsWith("sai-phone-") || name.startsWith("sai-web-v")) return true;
    return name.startsWith("agent-link-web-v") && name !== current;
  }
  ```
- `test_migrate.js` の検査を合わせる。`swIsOldCache("sai-web-v132", "agent-link-web-v133") === true`、`swIsOldCache("agent-link-web-v132", "agent-link-web-v133") === true`、`swIsOldCache("agent-link-web-v133", "agent-link-web-v133") === false`。版の数字の検査も `agent-link-web-v<数>`・`?v=<数>` にする。

### 4. 残りを探す
変えたあとに次を実行し、出た行をすべて直す。直さない行は、理由を完了報告に書く。
```bash
git grep -nIE 'sai-link|sai-web|SAI_|\.sai\b|sai-engine|sai_web|sai-paper|sai-sci' -- apps/web
```
許す行は、`sw.js` の `swIsOldCache` と `test_migrate.js` の古い倉庫の名前（`sai-web-v`・`sai-phone-`）だけである。

## 共有契約（他の作業と共有する取り決め。worker は変えない）
### C2: 名前の対応表（U05・U06・U07 で同じ物を使う）

| 種類 | 今 | 新しい名前 |
|---|---|---|
| 実行ファイル（Rust） | `sai-link` | `agent-link` |
| Web の命令 | `sai-web`（`apps/web/bin/sai-web`） | `agent-link-web`（`apps/web/bin/agent-link-web`） |
| crate のフォルダと名前 | `crates/sai-link-engine`・`sai-link-cli`・`sai-link-view`（Rust では `sai_link_engine` など） | `crates/agent-link-engine`・`agent-link-cli`・`agent-link-view`（`agent_link_engine` など） |
| Python のパッケージ | `apps/web/src/sai_web`（pyproject の `name = "sai-web"`） | `apps/web/src/agent_link_web`（`name = "agent-link-web"`）。起動は `agent_link_web.main:app`・`-m agent_link_web.webcli` |
| 環境変数 | `SAI_<名前>` | `AGENT_LINK_<名前>`。例外は 1 つだけ: `SAI_LINK_BIN` → `AGENT_LINK_BIN` |
| `SAI_` で始まる定数（Rust の `SAI_CLI_NAMES`・`SAI_FINALIZE_COMMAND_MD`、JS の `SAI_WIDE_PX`・`SAI_TERMS` など） | `SAI_<名前>` | `AGENT_LINK_<名前>`（環境変数と同じ規則） |
| プロジェクトのフォルダ | `.sai/` | `.agent-link/` |
| 実行の DB の置き場 | `.sai/sai-engine/sai-engine.db`（同じフォルダに `daemon.pid`・`daemon.status`・`daemon.log`・`notifications/`・`waggle-blobs/`） | `.agent-link/engine/agent-link.db`（同じフォルダに同じ名前の物） |
| 置き場（home） | `~/.sai`（`SAI_HOME`） | `~/.agent-link`（`AGENT_LINK_HOME`） |
| Web の DB とログ | `<home>/web/sai_web.db`・`<home>/web/sai-web.log` | `<home>/web/agent_link_web.db`・`<home>/web/agent-link-web.log` |
| 書き出し・合言葉のファイル | `<home>/web/runs/<project>/*.json`・`<home>/web/token` | 同じ相対パス（`<home>` だけが変わる） |
| template のファイル | `sai-base.md`・`sai-cli.md`・`sai-task.md`・`sai-user.md`・`sai-report.md` | `link-base.md`・`link-cli.md`・`link-task.md`・`link-user.md`・`link-report.md`（block の `templates:` の値も同じに変える） |
| cc の命令ファイル | `.claude/commands/sai-finalize.md`（`/sai-finalize`） | `.claude/commands/agent-link-finalize.md`（`/agent-link-finalize`） |
| cc の許可 | `Bash(sai-link *)`・`Bash(sai-web *)` | `Bash(agent-link *)`・`Bash(agent-link-web *)` |
| waggle の sharer の名前 | `"sai-engine"` | `"agent-link"` |
| 文書の参照の形 | `waggle:<tok>:path:.sai/outputs/*/<run_id>/<file>` | `waggle:<tok>:path:.agent-link/outputs/*/<run_id>/<file>` |
| 画面の名前 | `<title>sai</title>`・`manifest.json` の `name`・`short_name` が `sai` | `agent-link` |
| 画面の版 | `sw.js` の `CACHE = "sai-web-v132"`・`?v=132` | `CACHE = "agent-link-web-v133"`・`?v=133`。`swIsOldCache` は `sai-phone-*`・`sai-web-v*`・今と違う `agent-link-web-v*` を古いと判定する |

変えない物:
- 中だけで使う名前（Rust の関数 `sai_home()`・`sai_path()`、JS の `saiApi`・`saiToast`・`window.SaiRoutes`・画面ごとの頭 `sai`・`hm`・`wf` など、CSS のクラス）。外から見えないので変えない。
- cc のフックの命令の名前（`cc-prompt-hook`・`cc-state-hook`・`cc-permission-hook`・`cc-statusline`）。
- `waggle/`（vendor の crate）。sai の名前を含まない。
- `apps/web` のフォルダの名前と、`apps/web/hooks/` の場所。Rust は `env!("CARGO_MANIFEST_DIR")` からの相対パス `../../apps/web/hooks` で見つける（`cc_command.rs:447-453`）。crate のフォルダの名前が変わっても、このパスはそのまま使える。
- 古いデータの読み替えはしない。`.sai/` や `~/.sai` を読みに行く互換の処理は作らない（要件定義書の決定: 新しく始める）。

残してよい古い名前（U07 の最後の grep で許す所）:
- `sw.js` の `swIsOldCache` と `apps/web/tests/test_migrate.js` の、古い倉庫の名前（`sai-web-v`・`sai-phone-`）。
- `.gitignore` の `.sai/` の行（作業中は sai-moon がこのリポジトリの `.sai/` に書くため）。
- `docs/switch-over.md`（sai-moon の命令と古い置き場を説明する）と `docs/plan/**`。
- `README.md`・`ARCHITECTURE.md` の「sai-tree から複製した」という由来の説明。

## ファイルの所有権
- 書いてよい: `apps/web/**`
- 読むだけ: `crates/**`（CLI の名前と JSON の確認）
- 触らない: `crates/**`・`Cargo.*`・`docs/**`・`.github/**`・`AGENTS.md`・`README.md`・`ARCHITECTURE.md`・`CONCEPT.md`・`CLAUDE.md`・`.gitignore`

## 完了条件
- [ ] AC1: `apps/web/src/agent_link_web/` と `apps/web/bin/agent-link-web` がある。`sai_web`・`bin/sai-web` が無い。
- [ ] AC2: Web の JS と Python のテストが、`env -u` を付けずに通る（下の検証）。
- [ ] AC3: 変更内容 4 の grep の結果が、許した行だけになる。
- [ ] AC4: 一時フォルダの置き場（`AGENT_LINK_HOME`）で `agent-link-web serve --port 8297` を起こすと、`GET /api/health` が 200 を返す。`<置き場>/web/agent_link_web.db` ができる。`GET /` の HTML の `<title>` が `agent-link`。

## 検証
サーバは 8297 番で起こし、必ず止める（8299 番は使用中）。

```bash
cd ~/workdir/Sai/agent-link   # worktree の中ならその場所
(cd apps/web && uv sync)
for f in apps/web/tests/test_*.js; do node "$f" || exit 1; done
for f in apps/web/tests/test_*.py; do PYTHONPATH=apps/web/src apps/web/.venv/bin/python "$f" || exit 1; done
git grep -nIE 'sai-link|sai-web|SAI_|\.sai\b|sai-engine|sai_web|sai-paper|sai-sci' -- apps/web

T=$(mktemp -d)
AGENT_LINK_WEB_NO_WATCH=1 AGENT_LINK_HOME="$T" AGENT_LINK_WEB_TOKEN= \
  perl -e 'alarm 120; exec @ARGV' apps/web/bin/agent-link-web serve --port 8297 &
sleep 3
curl -s -o /dev/null -w "%{http_code}\n" http://127.0.0.1:8297/api/health
curl -s http://127.0.0.1:8297/ | grep -o "<title>[^<]*</title>"
ls "$T/web"
kill %1; rm -rf "$T"
```
期待: テストが通る。grep は許した行だけ。`200`。`<title>agent-link</title>`。`agent_link_web.db` がある。

## 決定事項と理由
- JS の中だけの名前（`saiApi`・`window.SaiRoutes`・画面ごとの頭 `sai`）は変えない — 理由: 外から見えない（要件定義書の決定）。`SAI_` で始まる定数は、規則を 1 つにするために変える。
- 画面の版は 133 から続ける — 理由: 同じ住所（Mac の 8200 番）で sai-moon の画面から替わる。数字を戻すと、どちらが新しいか分かりにくい。
- `swIsOldCache` は sai-moon の倉庫（`sai-web-v*`）も消す — 理由: iPhone に古い画面が残らないようにする。

## 要確認
- なし
