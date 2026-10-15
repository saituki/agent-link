# 概要

プロジェクト名: `sai-tree`
プロジェクトルート: `~/workdir/Sai/sai-tree`
目的: 研究室の多人数で使う coding agent と論文の作業の道具。Linux のサーバで動く多人数モード（sai-tree 本体）と、各自の Mac で動く論文の前処理アプリ（sai-prep）からなる。sai-moon（`~/workdir/Sai/sai-moon`）の複製で、sai-moon は読み取り専用（下の「実行しないこと」）。

サーバの構成は 3 プロセス: Rust の実行エンジン（`sai-link`・`sai-paper`・`sai-sci`）と、Web の窓口（`apps/web`）と、音声の裏方サーバー（`apps/radio`）。プロジェクトの作業は、Linux のサーバの上で 1 プロジェクト 1 コンテナに分けて動く（`docs/server.md`）。時系列の道具 sai-ts（`apps/ts`）は各プロジェクトのコンテナの中で動き、sai-web が別のオリジンの入口（`127.0.0.1` の別の待ち受け・`SAI_WEB_TS_PORT`）で中継する。

作業の前に `ARCHITECTURE.md` の §1（3 つの考え）と §2（大原則）を読むこと。

## フォルダ

| フォルダ | 中身 |
|---|---|
| `crates/` | Rust のクレート（`sai-link-engine`・`sai-link-cli`・`sai-link-view`・`sai-paper-core`・`sai-paper-cli`・`sai-sci-core`・`sai-sci-cli`） |
| `apps/web/` | Web の窓口（FastAPI と素の JavaScript）。`server/` に配備（`Dockerfile`・`sai-web.service`・`deploy.sh`・`sai-in-container`） |
| `apps/prep/` | sai-prep。各自の Mac で論文 PDF を前処理して束を送るアプリ（標準ライブラリだけ） |
| `apps/radio/` | 音声の裏方サーバー（Radio）。`README.md`・`ARCHITECTURE.md` を読む |
| `apps/ts/` | sai-ts。時系列を見て判断し、その判断を agent に渡す道具。`README.md` を読む |
| `apps/mlflow-server/` | MLflow（実験の記録）の docker compose |
| `sai-sci/` | research-tree の部品（`blocks/`・`template/`・`tools/`・`exec/`）。`sai-sci` に埋め込まれる |
| `waggle/` | vendor クレート（原則編集しない） |
| `docs/` | 利用者向けの文書。`docs/plan/` は計画書（読むだけ。書かない） |

## 読む文書

| 文書 | 中身 |
|---|---|
| `ARCHITECTURE.md` | 考え方・境界・依存の向き・CLI の約束・置き場（`SAI_HOME`）・ファイル構成・機能を足す手順 |
| `CONCEPT.md` | 概念と用語・画面に出す専門用語の一覧 |
| `README.md` | 開発（Mac・単独モード）の起動手順・ポート表・環境変数表・sai-prep の入れ方 |
| `docs/server.md` | サーバ（Linux・多人数モード）の入れ方・配備（`deploy.sh`）・バックアップ・時系列（sai-ts・§17） |
| `docs/integration.md` | `apps/web` の API と画面の住所の一覧 |
| `docs/for-sai-developers.md` | CLI と `apps/radio` が守る約束（`apps/web` が読む JSON の項目） |
| `docs/sai-sci.md`・`docs/sai-sci-tutorial.md` | research-tree の使い方・手を動かす練習 |
| `apps/prep/README.md` | sai-prep の使い方（Mac） |
| `apps/radio/README.md`・`apps/radio/ARCHITECTURE.md` | Radio だけ触るとき |
| `apps/ts/README.md` | sai-ts（時系列）だけ触るとき |

`sai-link` などは PATH に無い。`README.md` の「入れ方」の PATH の足し方を見ること。

## 作業の約束

- **`apps/web` は DB（`sai-engine.db`・`papr.db`・`sai_radio.db`）を開かない。** 3 つの CLI の JSON・書き出し・登録簿と、`apps/radio`・MLflow の HTTP API だけを使う。足りない処理はコアに足して CLI から出す（`ARCHITECTURE.md` §2.4）。
- **CLI を直したら** 終了コード・`--json`・標準出力と標準エラーの約束を守る（`ARCHITECTURE.md` §2.3）。`apps/web` が読む JSON の項目を変えたら、`docs/for-sai-developers.md` と `apps/web` を一緒に直す。
- **`SAI_HOME` の規則は 3 か所にある**（`crates/sai-link-engine/src/project.rs` の `sai_home()`・`crates/sai-paper-core/src/paths.rs` の `sai_home()`・`apps/web/src/sai_web/registry.py` の `sai_home()`）。必ず 3 つそろえて直す。
- **依存を増やさない。** サーバは FastAPI と標準ライブラリ、画面は素の JavaScript。npm などで新しいパッケージを入れない。sai-prep も標準ライブラリだけ（docling・pdf2zh・mlx-lm は別の `uv tool` の環境に入れて CLI で呼ぶ）。
- iPhone（幅 390px）と PC（幅 1280px・900px 以上は 2 列）の両方で使える画面にする。外から来る値は CLI を呼ぶ前に `apps/web` が検査する（通らなければ 400）。ファイルのパスは iPhone から受け取らない。
- **多人数モードの権限。** ログインは大学のメールアドレスと Cookie（`SAI_WEB_COOKIE_SECURE`）で行う。Claude のトークンは各自の個人のトークンで、他人と共有しない。コンテナから Web を呼ぶ内部の合言葉（`SAI_WEB_INTERNAL_TOKEN`）は、利用者の画面に出さない。
- 編集の注意:
  - `waggle/` は vendor クレート。原則編集しない。
  - `sai-sci/` は `sai-sci` の実行ファイルに埋め込まれる。直したら release を作り直す。
  - `apps/mlflow-server/docker-compose.yml` の `name: mlflow-server` は消さない（消すと volume が別になり記録を失う）。
  - `.sai/` はこのプロジェクト自身のデータ。書き込みに注意する。

### 画面のコード

- 新しい画面は `window.SaiRoutes` に登録する。`app.js` に画面ごとの分岐を足さない。
- `ui.js` の部品（`saiApi`・`saiToast`・`saiConfirm`・`saiSheet`・`saiEmpty`・`saiTerm` など）と、`markdown.js` の `renderMarkdown` を使う。同じ物を作り直さない。
- 一番外側の関数・変数には画面ごとの頭（`sai`・`hm`・`wf`・`df`・`pim` など）を付ける。`apps/web/tests/test_globals.js` で重なりを検査している。
- 画面ファイルを変えて配るときは、`sw.js` の `CACHE` と `index.html` の `?v=` をそろえて上げる。`apps/web/tests/test_migrate.js` が版の数字（`sai-web-v<数>`・`?v=<数>`）を固定しているので、それも合わせる。ファイルを足したら `ASSETS` にも足す。

### テストと確認

テストの命令は次のとおりで、CI（`.github/workflows/ci.yml`）も同じ命令を回す。命令を直したら両方直す。

```bash
cd ~/workdir/Sai/sai-tree

# Rust（環境変数が混ざっていると History テスト 1 件が落ちる。必ず env -u で外す）
env -u SAI_PROJECT_ROOT -u SAI_PARENT_RUN_ID -u SAI_BLOCK_NAME -u SAI_TARGET_NODE \
  perl -e 'alarm 1800; exec @ARGV' cargo test --workspace

# Web（JS 36 本）
for f in apps/web/tests/test_*.js; do node "$f" || exit 1; done

# Web（Python 42 本。`SAI_LINK_BIN` などが設定されていると 5 本が落ちるので、そのときは env -u SAI_LINK_BIN -u SAI_PAPER_BIN -u SAI_SCI_BIN で外す）
for f in apps/web/tests/test_*.py; do PYTHONPATH=apps/web/src apps/web/.venv/bin/python "$f" || exit 1; done

# Web（コンテナの中継。偽の docker で確かめる。実物の Docker は使わない）
sh apps/web/server/test_sai_in_container.sh

# Radio（3 本）
(cd apps/radio && for f in tests/test_*.py; do PYTHONPATH=src .venv/bin/python "$f" || exit 1; done)

# sai-ts（Python 19 本・JS 12 本。`apps/ts` の uv sync が要る）
(cd apps/ts && for f in tests/test_*.py; do PYTHONPATH=src .venv/bin/python "$f" || exit 1; done)
for f in apps/ts/tests/test_*.js; do node "$f" || exit 1; done

# sai-prep（5 本。標準ライブラリだけ）
(cd apps/prep && python3 -m unittest discover -s tests)

# 実験ループの道具（sai-sci/tools。Python 3.11 以上。`~/.python-version` が未導入の版を指すと uv が失敗するので --python を付ける）
(cd sai-sci/tools && for t in test_exp_exec test_exec_ssh test_exec_dok test_dok test_refcode test_modeldb test_data_result test_data_eda test_onto_build; do python3.13 -W ignore -m unittest $t || exit 1; done)
(cd sai-sci/tools && uv run --no-project --python ">=3.11" --with mlflow python -m unittest test_exp_summary)

# 配備の形（sh の文法だけ。反映はしない）
sh -n apps/web/server/deploy.sh
```

| 対象 | 本数 | CI のジョブ | 備考 |
|---|---|---|---|
| Rust（`cargo test --workspace`） | - | Rust（ubuntu） | |
| Web の JS | 36 | Web（ubuntu） | |
| Web の Python | 42 | Web（ubuntu） | |
| コンテナの中継 | 1（sh） | Web（ubuntu） | |
| sai-ts の Python | 19 | sai-ts（ubuntu） | `apps/ts` の uv sync が要る |
| sai-ts の JS | 12 | sai-ts（ubuntu） | |
| Radio | 3 | Radio（ubuntu） | |
| sai-prep | 5 | sai-prep（ubuntu・Python 3.12） | |
| sai-sci の道具 | 10 | sai-sci の道具（ubuntu・Python 3.13） | `test_exp_summary` は mlflow が要る |

CI の Linux（ubuntu-latest）は本番と同じ OS である。リポジトリ直下に `tests/` は無い。テストはクレート・アプリの中にある。
Mac で確かめるときも、上の命令を上から順に実行する。

- **Web を確かめるサーバは、一時フォルダの置き場で起こす。** 確認用は 8299 番で起こし、終わったら必ず止める。
  ```bash
  T=$(mktemp -d)
  SAI_WEB_NO_WATCH=1 SAI_HOME="$T" SAI_WEB_TOKEN= perl -e 'alarm 180; exec @ARGV' apps/web/bin/sai-web serve --port 8299 &
  # … curl http://127.0.0.1:8299/ などで確かめる …
  kill %1; rm -rf "$T"
  ```
- **動いている Web（本番は Linux の 8200 番）と、Mac で別に動かしている Web は、確認のために止めたり再起動したりしない。** 確認は 8299 番など別の番号で行う。
- `apps/web`・`apps/radio` の仮想環境は `uv sync` で作る。`apps/web` は `pyproject.toml` と `uv.lock` を使う。
- サーバ・ブラウザ・ネットワークを使う命令には、必ず時間の上限を付ける（例 `perl -e 'alarm 180; exec @ARGV' <命令>`）。背景で起動したサーバとブラウザは、作業を終える前に止める。
- `cargo build --release` は本体フォルダで行い、`target/release/sai-link`・`sai-paper`・`sai-sci` を必ず作り直す。`apps/web` は release の実行ファイルを叩く。
- 配備（`apps/web/server/deploy.sh`）は Linux のサーバの上で走らせる。Mac からは `sh -n` で文法だけ確かめる。

### 実行しないこと

- `sai-link daemon start`・cc フックの登録。文書を書くだけで済ませる。
- `apps/mlflow-server` の `docker compose up / down / restart`（本物の MLflow を止める）。`docker compose config` のような読み取りだけにする。
- さくらの DOK（`sai-sci/exec/dok`）の `run` / `ssh-run`（課金される）。確認は偽の `dok.py` を使うテストで行う。
- `apps/web/server/deploy.sh` の本物の実行・`sudo systemctl`・本物のサーバの `/srv/sai/` の変更。確かめは文法（`sh -n`）と、偽の命令を使う試験で行う。
- `sai-link init`・`sai-sci init`（`--update` を含む）など、プロジェクトを書き換える命令を本物のプロジェクトで実行すること。確かめるときは一時フォルダをプロジェクトにし、`SAI_HOME` にも別の一時フォルダを入れ、最後に消す。
- 本物の `.sai/blocks/*.md`・`~/.sai/`・`papr.db` の中身を、確かめのために変えること。書き込みの確認は確認用の物（例: block `guide-check`）か一時フォルダで行い、最後に消す。
- 利用者の環境（`~/.sai*`・シェルの設定・`~/.local/bin`・サーバの `/srv/sai/`・`/etc/sudoers.d/`）を変える操作。文書に書くだけにする。
- sai-moon（`~/workdir/Sai/sai-moon`）と、その元の 4 リポジトリ（`~/workdir/Sai/sai-link`・`sai-paper`・`sai-phone`・`sai-radio`）を直すこと。読み取り専用。sai-tree は sai-moon の複製で、sai-moon にあった Mac の常駐（launchd）・herdr・承認の機能は持たない（`docs/plan/` の E3 と U05）。
