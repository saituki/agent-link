# 概要

プロジェクト名: `sai-moon`
プロジェクトルート: `~/workdir/Sai/sai-moon`
目的: 旧 4 リポジトリ（`sai-link`・`sai-paper`・`sai-phone`・`sai-radio`）を 1 つのリポジトリにまとめた、Mac 上の coding agent と論文の作業を一元扱う道具。
Rust の実行エンジン（`sai-link`・`sai-paper`・`sai-sci`）と、Web の窓口（`apps/web`）と、音声の裏方サーバー（`apps/radio`）の 3 プロセス構成。

考え方と仕組みは `ARCHITECTURE.md` にある。作業の前に §1（3 つの考え）と §2（大原則）を読むこと。

## ルール

- 日本語で説明すること．
- 日本語でコメントアウトを書くこと．
- コメントと画面の文字は自然な日本語で書く。中国語・韓国語の語や、意味をなさない英字の断片を混ぜない。
- 画面の案内文・説明・エラーは日本語にする。専門用語は英単語にそろえる（Next Action・History・PATH・SPROUT・Create SPROUT・Add PATH・Delete PATH・TRIAGE・Create CARD・Library・Import など）。`data-tab`・住所・API の名前などの識別子も英語のまま。専門用語の一覧は [`CONCEPT.md`](./CONCEPT.md)。

## フォルダ

| フォルダ | 中身 | 編集 |
|---|---|---|
| `crates/` | Rust のワークスペース。7 クレート（`sai-link-engine`・`sai-link-view`・`sai-link-cli`・`sai-paper-core`・`sai-paper-cli`・`sai-sci-core`・`sai-sci-cli`） | 編集可能 |
| `waggle/` | 独立した `[workspace]` を持つ vendor クレート群。名前とバージョンを変えないよう Cargo の `exclude` に入れてある | 原則編集しない |
| `apps/web/` | Web の窓口（FastAPI + 素の JavaScript）。`apps/web/src/sai_web/` にサーバと画面、`bin/sai-web` に CLI、`hooks/` に cc のフック、`launchd/` に plist のひな形 | 編集可能 |
| `apps/radio/` | 解説音声の裏方サーバー（FastAPI）。`apps/radio/src/sai_radio/` に本体、`tests/` に 3 本のテスト。`README.md` と `ARCHITECTURE.md` はこのフォルダにある | 編集可能 |
| `apps/mlflow-server/` | 実験の記録を溜める MLflow サーバの compose（postgres・minio・mlflow・caddy）と `templates/mlflow-agent.md`。Docker で動かす。`.env` は無い（鍵が入るので追跡しない。`.env.example` が見本） | 編集可能（`docker-compose.yml` の `name: mlflow-server` は消さない。消すと volume が別になり記録を失う） |
| `sai-sci/` | research-tree の型・項目ガイド・sci-core・mind.md のひな形・32 本のブロック（professor・professor-backward・operator・kaizen・paper-debater・paper-reviewer と 26 本の `sci-*`）・研究向けの共通 template（`sai-agent`・`sai-task`・`sai-cli`）・逆向きの共通の決まり（`bw-core`）・`tools/`（実験ループの台帳と、実行先への分配器、参照コードの道具 `refcode.py`・ModelDB のカタログ `modeldb.py`、データ分析の検査と記録 `data_result.py`・K-Dense の EDA と生体信号の CLI の写し（`eda/`・`biosignal/`））・`exec/`（実行先の見本）。`sai-sci` の実行ファイルに埋め込まれる | 編集可能（直したら release を作り直す） |
| `docs/` | 開発者向けの文書（`integration.md`・`for-sai-developers.md`）と、利用者が 1 回だけ行う切り替えの手順書（`switch-over.md`） | 編集可能 |
| `.github/workflows/` | CI（`ci.yml`）。Rust・Web・Radio・sai-sci の道具のテストを回す。下の「テストと確認」と同じ命令にそろえる | 編集可能 |
| `.sai/` | このプロジェクト自身の中のデータ（`sai-engine.db`・`blocks/`・`template/`・`settings.json`・`outputs/`）。`.gitignore` 済み | 書き込み注意 |

**リポジトリ直下に `tests/` は無い。** テストはそれぞれのクレート・アプリの中に置かれている。

| 種類 | 場所 | 動かし方 |
|---|---|---|
| Rust の結合テスト | `crates/*/tests/*.rs` | `cargo test --workspace` |
| Web の画面テスト | `apps/web/tests/test_*.js` | `node` |
| Web のサーバテスト | `apps/web/tests/test_*.py` | `PYTHONPATH=apps/web/src apps/web/.venv/bin/python` |
| Radio のテスト | `apps/radio/tests/test_*.py`（3 本） | `cd apps/radio && PYTHONPATH=src .venv/bin/python tests/<file>.py` |
| 実験ループの道具 | `sai-sci/tools/test_*.py` | `cd sai-sci/tools && python -m unittest <名前>`（下の「テストと確認」） |

## 実行ファイル

| クレート・フォルダ | 実行ファイル | 何 |
|---|---|---|
| `crates/sai-link-cli` | `sai-link` | 実行エンジンの CLI。`daemon`・`run`・`card`・`doc`・`def`・`project`・`session`・`workflow`・`usage`・`init`・`schema` など |
| `crates/sai-paper-cli` | `sai-paper` | 論文の CLI。`search`・`find`・`library`・`paper`・`import`・`set`・`index`・`graph`・`content`・`init`・`schema` など |
| `crates/sai-sci-cli` | `sai-sci` | 研究の CLI。`init`・`status`・`node`・`context`・`propose`・`triage`・`card`・`report`・`brief`・`schema` など |
| `apps/web/bin/sai-web` | `sai-web` | Web の CLI（`projects`・`questions`・`answer`・`approvals`・`send`・`usage`・`mlflow`・`api`・`serve`）。仮想環境（`apps/web/.venv`）の Python で動く |

**ライブラリ crate 名（`sai_link_engine` / `sai_link_view` / `sai_paper_core` / `sai_sci_core`）と実行ファイル名は別。** TUI は無い。

`sai-link` などは PATH に無い。PATH を足す方法と、古い名前・置き場からの移行は `docs/switch-over.md`（段 3）を見ること。

## 読む文書

| 文書 | 中身 |
|---|---|
| `ARCHITECTURE.md` | 全体の考え方・層の分け方・大原則・置き場・機能を足す手順 |
| `CONCEPT.md` | 概念と用語（部品・run・card・doc・research-tree・画面の名前ほか）。画面に出す専門用語の一覧 |
| `README.md` | 3 プロセスの起動手順・ポート表・環境変数表 |
| `docs/integration.md` | `apps/web` の API と画面の住所の一覧・統合の注意点 |
| `docs/for-sai-developers.md` | `crates/` 側・`apps/radio` 側が守る約束（`apps/web` が読む JSON の項目） |
| `docs/switch-over.md` | 利用者が 1 回だけ行う切り替えの手順書（実行ファイル・置き場・launchd・各プロジェクトの `init --update`） |
| `sai-sci/README.md` | research-tree の使い方と置き場 |
| `apps/radio/README.md`・`apps/radio/ARCHITECTURE.md` | Radio だけ触るときはこれ（ポートは 8201 / 8202 に直してある） |

## 作業の約束

### 設計

- **中の処理は Rust のコア、窓口は `apps/web` だけ。** `apps/web` は他の DB（`sai-engine.db` / `papr.db` / `sai_radio.db`）を開かない。呼ぶのは **3 つの CLI（`sai-link`・`sai-paper`・`sai-sci`）の JSON・書き出し・登録簿と、`apps/radio`・MLflow の HTTP API だけ**。
- **3 つの CLI は同じ約束を守る。**
  - 終了コード: `0` 成功、`2` 入力の誤り、`3` 見つからない、`4` 状態の衝突、`1` それ以外の失敗。
  - 失敗のとき標準出力は空にし、理由は標準エラーに書く。
  - 機械が読む出力は `--json`。命令の一覧は `schema --json`（Web の「コマンド」画面が読む）。
  - Web が付けるフラグは CLI が受け付ける（例: `sai-sci apply --json`。`docs/for-sai-developers.md` の約束）。
- **依存の向きを守る。**
  - `sai-link` は `sai-paper` と `sai-sci` を知らない。
  - `sai-paper` は `sai-link` のデータを読まず、`sai-link` と `sai-sci` を呼ばない。
  - `sai-sci` は `sai-paper` をライブラリで、`sai-link` を CLI で使う。
- **エージェントは sai の機能を Bash で CLI として呼ぶ**（`sai-link doc write` など）。MCP の `sai` は無い。
- **足りない処理はコアに足す。** Web に要る処理が CLI の中にしか無いときは、`sai-link-view` か `sai-paper-core`・`sai-sci-core` に移し、CLI から出してから `apps/web` が呼ぶ。`apps/web` や CLI に処理を写さない。
- **外から来る値は、CLI を呼ぶ前に `apps/web` が検査する。** 通らない値では CLI を呼ばない（400）。ファイルのパスは iPhone から受け取らない。
- **依存を増やさない。** サーバは FastAPI と標準ライブラリ、画面は素の JavaScript。npm などで新しいパッケージを入れない。
- iPhone（幅 390px）と PC（幅 1280px・900px 以上は 2 列）の両方で使える画面にする。

### 置き場

- **利用者全体の置き場は `~/.sai/`。** 環境変数 `SAI_HOME`（空でない値）があればそれが優先、無ければ `~/.sai`。
  Rust 側は `sai_link_engine::project::sai_home()`（`crates/sai-link-engine/src/project.rs`）と `sai_paper_core::paths::sai_home()`（`crates/sai-paper-core/src/paths.rs`）、`apps/web` 側は `registry.sai_home()`（`apps/web/src/sai_web/registry.py`）。**3 つの値を必ずそろえること。**
  この中に `registry.json`・`cc-cost`・共通の `blocks/`・`template/`・`settings.json`・`paper.env`・`web/` が入る。
  `web/` の中身は `runs/`・`cc-rate-limits.json`・`token`・`sai_web.db`・`vapid.json`・`sai-web.log`。
- **プロジェクトの中の `<root>/.sai/` は変わらない。** `sai-engine.db`・`blocks/`・`outputs/` はここ。
- `apps/web/src/sai_web/usage.py` の `cc_limits_path()` は `<SAI_HOME>/web/cc-rate-limits.json` を読む（書くのは `sai-link` の `cc-statusline`）。
- Web の環境変数は `SAI_WEB_*`（`SAI_WEB_URL`・`SAI_WEB_TOKEN`・`SAI_WEB_PORT`・`SAI_WEB_RUNS_DIR` など）。呼ぶ CLI の場所は `SAI_LINK_BIN`・`SAI_PAPER_BIN`・`SAI_SCI_BIN`。表は `README.md`。

### 画面のコード

- 新しい画面は `window.SaiRoutes` に登録する。`app.js` に画面ごとの分岐を足さない。
- `ui.js` の部品（`saiApi`・`saiToast`・`saiConfirm`・`saiSheet`・`saiEmpty`・`saiTerm` など）と、`markdown.js` の `renderMarkdown` を使う。同じ物を作り直さない。
- 一番外側の関数・変数には画面ごとの頭（`sai`・`hm`・`wf`・`df`・`pim` など）を付ける。`apps/web/tests/test_globals.js` で重なりを検査している。
- 画面ファイルを変えて配るときは、`sw.js` の `CACHE` と `index.html` の `?v=` をそろえて上げる。`apps/web/tests/test_migrate.js` が版の数字（`sai-web-v<数>`・`?v=<数>`）を固定しているので、それも合わせる。ファイルを足したら `ASSETS` にも足す。

### テストと確認

```bash
cd ~/workdir/Sai/sai-moon

# Rust（環境変数が混ざっていると History テスト 1 件が落ちる。必ず env -u で外す）
env -u SAI_PROJECT_ROOT -u SAI_PARENT_RUN_ID -u SAI_BLOCK_NAME \
  perl -e 'alarm 1800; exec @ARGV' cargo test --workspace

# Web（JS 35 本）
for f in apps/web/tests/test_*.js; do node "$f" || exit 1; done

# Web（Python 28 本。`SAI_LINK_BIN` などが設定されていると 5 本が落ちるので、そのときは env -u SAI_LINK_BIN -u SAI_PAPER_BIN -u SAI_SCI_BIN で外す）
for f in apps/web/tests/test_*.py; do PYTHONPATH=apps/web/src apps/web/.venv/bin/python "$f" || exit 1; done

# Radio（3 本）
(cd apps/radio && for f in tests/test_*.py; do PYTHONPATH=src .venv/bin/python "$f" || exit 1; done)

# 実験ループの道具（sai-sci/tools。Python 3.11 以上。`~/.python-version` が未導入の版を指すと uv が失敗するので --python を付ける）
(cd sai-sci/tools && for t in test_exp_exec test_exec_ssh test_exec_dok test_dok test_refcode test_modeldb test_data_result test_data_eda; do python3.13 -W ignore -m unittest $t || exit 1; done)
(cd sai-sci/tools && uv run --no-project --python ">=3.11" --with mlflow python -m unittest test_exp_summary)
```

CI（`.github/workflows/ci.yml`）は同じ命令を 4 つのジョブ（Rust・Web・Radio・sai-sci の道具）で回す。命令を直したら両方直す。

- **Web を確かめるサーバは、一時フォルダの置き場で起こす。** 本番の 8200 番の Web は止めない。確認用は別の番号（8299）で起こし、終わったら必ず止める。
  ```bash
  T=$(mktemp -d)
  SAI_WEB_NO_WATCH=1 SAI_HOME="$T" SAI_WEB_TOKEN= perl -e 'alarm 180; exec @ARGV' apps/web/bin/sai-web serve --port 8299 &
  # … curl http://127.0.0.1:8299/ などで確かめる …
  kill %1; rm -rf "$T"
  ```
- **本番の旧 sai-phone（8100 番）と旧 sai-radio（8000 番）、いま使っている Web（8200 番）は、確認のために止めたり再起動したりしない。** 確認は 8201 / 8202 / 8299 番など別の番号で行う。
- `apps/web` と `apps/radio` の仮想環境は `uv sync` で作る（`apps/web/.venv`・`apps/radio/.venv`）。`apps/web` は `pyproject.toml` と `uv.lock`、`requirements.txt` は無い。
- サーバ・ブラウザ・ネットワークを使う命令には、必ず時間の上限を付ける（例 `perl -e 'alarm 180; exec @ARGV' <命令>`）。上限の無い命令を `| tail` などにつないで待たない。
- 背景で起動したサーバとブラウザは、作業を終える前に止める。
- **`sai-link daemon start`・`launchctl load` / `unload`・cc フックや pi 拡張の登録は実行しない。** 文書を書くだけで済ませる。
- **`apps/mlflow-server` の `docker compose up / down / restart` は、確認のために実行しない。** 動いている本物の MLflow（実験の記録）を止める。`docker compose config` のような読み取りだけにする。
- **さくらの DOK（`sai-sci/exec/dok`）の `run` / `ssh-run` は実行しない。** 課金される。確認は偽の `dok.py` を使うテストで行う。
- **`sai-link init`・`sai-sci init`（`--update` を含む）など、プロジェクトを書き換える命令は、本物のプロジェクトでは実行しない。** 確かめるときは一時フォルダをプロジェクトにし、`SAI_HOME` にも別の一時フォルダを入れて行い、最後に消す。
- 書き込みを確かめるときは、確認用の物（例: block `guide-check`・確認用の .bib）か一時フォルダを使い、最後に消す。本物の `.sai/blocks/*.md`・`~/.sai/`・`papr.db` の中身を確かめのために変えない。
- 利用者の環境（`~/.sai*`・`~/Library/LaunchAgents/`・シェルの設定・`~/.local/bin`）を変える操作は、手順書（`docs/switch-over.md`）に書くだけで、エージェントは実行しない。

### 他のフォルダを直すとき

- 約束は `docs/for-sai-developers.md` に従う。`apps/web` が読む JSON の項目を変えたら、その文書と `apps/web` を一緒に直す。
- 旧 4 リポジトリ（`~/workdir/Sai/sai-link`・`sai-paper`・`sai-phone`・`sai-radio`）は**読み取り専用**。sai-moon がコピー元。ここを直さない。
  着手前から未 commit の変更が残っているので、触る前に `git -C ~/workdir/Sai/<名前> status --short` を控えておき、終わりに同じであることを確かめる。
- `cargo build --release` は本体フォルダで行い、`target/release/sai-link`・`sai-paper`・`sai-sci` を必ず作り直す。`apps/web` は release の実行ファイルを叩く（`SAI_LINK_BIN`・`SAI_PAPER_BIN`・`SAI_SCI_BIN` の既定は PATH 上の名前）。
