# 概要

プロジェクト名: `sai-moon`
プロジェクトルート: `~/workdir/Sai/sai-moon`
目的: Mac 上の coding agent と論文の作業を一元扱う道具。Rust の実行エンジン（`sai-link`・`sai-paper`・`sai-sci`）と、Web の窓口（`apps/web`）と、音声の裏方サーバー（`apps/radio`）の 3 プロセス構成。

作業の前に `ARCHITECTURE.md` の §1（3 つの考え）と §2（大原則）を読むこと。

## ルール

- 日本語で説明すること．
- 日本語でコメントアウトを書くこと．
- コメントと画面の文字は自然な日本語で書く。中国語・韓国語の語や、意味をなさない英字の断片を混ぜない。
- 画面の案内文・説明・エラーは日本語にする。専門用語は英単語にそろえる（Next Action・History・PATH・SPROUT・Create SPROUT・Add PATH・Delete PATH・TRIAGE・Create CARD・Library・Import など）。`data-tab`・住所・API の名前などの識別子も英語のまま。専門用語の一覧は [`CONCEPT.md`](./CONCEPT.md)。

## 読む文書

| 文書 | 中身 |
|---|---|
| `ARCHITECTURE.md` | 考え方・境界・依存の向き・CLI の約束・置き場（`SAI_HOME`）・ファイル構成・機能を足す手順 |
| `CONCEPT.md` | 概念と用語・画面に出す専門用語の一覧 |
| `README.md` | 起動手順・ポート表・環境変数表 |
| `docs/integration.md` | `apps/web` の API と画面の住所の一覧 |
| `docs/for-sai-developers.md` | CLI と `apps/radio` が守る約束（`apps/web` が読む JSON の項目） |
| `sai-sci/README.md` | research-tree の使い方 |
| `apps/radio/README.md`・`apps/radio/ARCHITECTURE.md` | Radio だけ触るとき |

`sai-link` などは PATH に無い。`README.md` の「入れ方」の PATH の足し方を見ること。

## 作業の約束

- **`apps/web` は DB（`sai-engine.db`・`papr.db`・`sai_radio.db`）を開かない。** 3 つの CLI の JSON・書き出し・登録簿と、`apps/radio`・MLflow の HTTP API だけを使う。足りない処理はコアに足して CLI から出す（`ARCHITECTURE.md` §2.4）。
- **CLI を直したら** 終了コード・`--json`・標準出力と標準エラーの約束を守る（`ARCHITECTURE.md` §2.3）。`apps/web` が読む JSON の項目を変えたら、`docs/for-sai-developers.md` と `apps/web` を一緒に直す。
- **`SAI_HOME` の規則は 3 か所にある**（`crates/sai-link-engine/src/project.rs` の `sai_home()`・`crates/sai-paper-core/src/paths.rs` の `sai_home()`・`apps/web/src/sai_web/registry.py` の `sai_home()`）。必ず 3 つそろえて直す。
- **依存を増やさない。** サーバは FastAPI と標準ライブラリ、画面は素の JavaScript。npm などで新しいパッケージを入れない。
- iPhone（幅 390px）と PC（幅 1280px・900px 以上は 2 列）の両方で使える画面にする。外から来る値は CLI を呼ぶ前に `apps/web` が検査する（通らなければ 400）。ファイルのパスは iPhone から受け取らない。
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

CI（`.github/workflows/ci.yml`）は同じ命令を 4 つのジョブ（Rust・Web・Radio・sai-sci の道具）で回す。命令を直したら両方直す。リポジトリ直下に `tests/` は無い。テストはクレート・アプリの中にある。

- **Web を確かめるサーバは、一時フォルダの置き場で起こす。** 確認用は 8299 番で起こし、終わったら必ず止める。
  ```bash
  T=$(mktemp -d)
  SAI_WEB_NO_WATCH=1 SAI_HOME="$T" SAI_WEB_TOKEN= perl -e 'alarm 180; exec @ARGV' apps/web/bin/sai-web serve --port 8299 &
  # … curl http://127.0.0.1:8299/ などで確かめる …
  kill %1; rm -rf "$T"
  ```
- **いま使っている Web（8200 番）と旧 sai-phone（8100 番）・旧 sai-radio（8000 番）は、確認のために止めたり再起動したりしない。** 確認は 8201 / 8202 / 8299 番など別の番号で行う。
- `apps/web` と `apps/radio` の仮想環境は `uv sync` で作る。`apps/web` は `pyproject.toml` と `uv.lock` を使う。
- サーバ・ブラウザ・ネットワークを使う命令には、必ず時間の上限を付ける（例 `perl -e 'alarm 180; exec @ARGV' <命令>`）。背景で起動したサーバとブラウザは、作業を終える前に止める。
- `cargo build --release` は本体フォルダで行い、`target/release/sai-link`・`sai-paper`・`sai-sci` を必ず作り直す。`apps/web` は release の実行ファイルを叩く。

### 実行しないこと

- `sai-link daemon start`・`launchctl load` / `unload`・cc フックや pi 拡張の登録。文書を書くだけで済ませる。
- `apps/mlflow-server` の `docker compose up / down / restart`（本物の MLflow を止める）。`docker compose config` のような読み取りだけにする。
- さくらの DOK（`sai-sci/exec/dok`）の `run` / `ssh-run`（課金される）。確認は偽の `dok.py` を使うテストで行う。
- `sai-link init`・`sai-sci init`（`--update` を含む）など、プロジェクトを書き換える命令を本物のプロジェクトで実行すること。確かめるときは一時フォルダをプロジェクトにし、`SAI_HOME` にも別の一時フォルダを入れ、最後に消す。
- 本物の `.sai/blocks/*.md`・`~/.sai/`・`papr.db` の中身を、確かめのために変えること。書き込みの確認は確認用の物（例: block `guide-check`）か一時フォルダで行い、最後に消す。
- 利用者の環境（`~/.sai*`・`~/Library/LaunchAgents/`・シェルの設定・`~/.local/bin`）を変える操作。文書に書くだけにする。
- 旧 4 リポジトリ（`~/workdir/Sai/sai-link`・`sai-paper`・`sai-phone`・`sai-radio`）を直すこと。読み取り専用で、sai-moon のコピー元。着手前から未 commit の変更があるので、触る前に `git -C ~/workdir/Sai/<名前> status --short` を控え、終わりに同じであることを確かめる。
