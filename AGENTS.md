# 概要

プロジェクト名: `sai-moon`
プロジェクトルート: `~/workdir/Sai/sai-moon`
作業ブランチ: `feat/sai-moon-v1`
目的: 旧 4 リポジトリ（`sai-link`・`sai-paper`・`sai-phone`・`sai-radio`）を 1 つのリポジトリにまとめた、Mac 上の coding agent と論文の作業を一元扱う道具。
Rust の実行エンジン（sai-link と sai-paper）と、Web の窓口（sai-phone）と、音声の裏方サーバー（sai-radio）の 3 プロセス構成。

考え方と仕組みは `ARCHITECTURE.md` にある。作業の前に §1（3 つの考え）と §2（大原則）を読むこと。

## ルール

- 日本語で説明すること．
- 日本語でコメントアウトを書くこと．
- コメントと画面の文字は自然な日本語で書く。中国語・韓国語の語や、意味をなさない英字の断片を混ぜない。
- 画面に出す言葉は日本語にする（`data-tab`・住所・API の名前などの識別子は英語のまま）。

## フォルダ

| フォルダ | 中身 | 編集 |
|---|---|---|
| `crates/` | Rust のワークスペース。8 クレート（`sai-link-engine`・`sai-link-cli`・`sai-link-tui`・`sai-link-widgets`・`sai-paper-core`・`sai-paper-cli`・`sai-paper-tui`・`sai-paper-cite`） | 編集可能 |
| `waggle/` | 独立した `[workspace]` を持つ vendor クレート群。名前とバージョン を変えないないよう Cargo の `exclude` に入れてある | 原則編集しない |
| `pi-litbase/` | pi の拡張（TypeScript）。Cargo の外 | 編集可能 |
| `apps/phone/` | Web の窓口（FastAPI + 素の JavaScript）。`apps/phone/src/sai_phone/` にサーバと画面、`hooks/` に cc のフック、`launchd/` に plist | 編集可能 |
| `apps/radio/` | 解説音声の裏方サーバー（FastAPI）。`apps/radio/src/sai_radio/` に本体、`tests/` に 3 本のテスト。`README.md` と `ARCHITECTURE.md` はこのフォルダにある | 編集可能 |
| `apps/mlflow-server/` | 実験の記録を溜める MLflow サーバの compose（postgres・minio・mlflow・caddy）と `templates/mlflow-agent.md`。Docker で動かす。`.env` は無い（鍵が入るので追跡しない。`.env.example` が見本） | 編集可能（`docker-compose.yml` の `name: mlflow-server` は消さない。消すと volume が別になり記録を失う） |
| `sai-sci/` | 研究方針の木（Research-tree）の型・項目ガイド・sci-core・mind.md のひな形・20 本のブロック（professor・operator と 18 本の `sci-*`）・研究向けの共通 template（`sai-agent`・`sai-task`・`sai-cli`）・`tools/`（実験ループの台帳と、実行先への分配器、`phone_api.py`）・`exec/`（実行先の見本）。`sai-moon-paper` に埋め込まれる | 編集可能（直したら release を作り直す） |
| `docs/` | 開発者向けの文書（`integration.md`・`for-sai-developers.md`） | 編集可能 |
| `.sai/` | このプロジェクト自身の中のデータ（`sai-engine.db`・`blocks/`・`template/`・`settings.json`・`outputs/`）。`.gitignore` 済み | 書き込み注意 |

**リポジトリ直下に `tests/` は無い。** テストはそれぞれのクレート・アプリの中に置かれている。

| 種類 | 場所 | 動かし方 |
|---|---|---|
| Rust の結合テスト | `crates/*/tests/*.rs` | `cargo test --workspace` |
| Web の画面テスト | `apps/phone/tests/test_*.js` | `node` |
| Web のサーバテスト | `apps/phone/tests/test_*.py` | `PYTHONPATH=apps/phone/src apps/phone/.venv/bin/python` |
| Radio のテスト | `apps/radio/tests/test_*.py`（3 本） | `cd apps/radio && PYTHONPATH=src .venv/bin/python tests/<file>.py` |

## バイナリ名

PATH 上で旧 `sai-link` / `sai-paper` と衝突しないよう、`[[bin]]` の名前を `sai-moon-*` にリネーム済み。**ライブラリ crate 名（`sai_link_engine` / `sai_link_widgets` / `sai_paper_core`）は変えていない。**

| クレート | 実行ファイル | 何 |
|---|---|---|
| `crates/sai-link-cli` | `sai-moon-link` | CLI。`daemon`・`phone`・`task`・`block`・`usage` など |
| `crates/sai-link-tui` | `sai-moon-link-tui` | sai-link の TUI。**`--help` に非対応**（起動するとすぐ TUI になる） |
| `crates/sai-paper-cli` | `sai-moon-paper` | 論文の CLI。`phone`・`content` など |
| `crates/sai-paper-tui` | `sai-moon-paper-tui` | 論文の TUI |
| `crates/sai-paper-cite` | `sai-moon-paper-cite` | 引用リストの TUI |

`sai-moon-link` は PATH に無い。PATH を足す方法は `README.md` の「起動」を見ること。

## 読む文書

| 文書 | 中身 |
|---|---|
| `ARCHITECTURE.md` | 全体の考え方・層の分け方・大原則・置き場・機能を足す手順 |
| `README.md` | 3 プロセスの起動手順・ポート表・環境変数表 |
| `docs/integration.md` | `apps/phone` の API と画面の住所の一覧・統合の注意点 |
| `docs/for-sai-developers.md` | `crates/` 側・`apps/radio` 側が守る約束（`apps/phone` が読む JSON の項目） |
| `apps/radio/README.md`・`apps/radio/ARCHITECTURE.md` | Radio だけ跌破ったときはこれ（ポートは 8201 / 8202 に直してある） |

## 作業の約束

### 設計

- **中の処理は Rust のコア、窓口は `apps/phone` だけ。** `apps/phone` は他の DB（`sai-engine.db` / `papr.db` / `sai_radio.db`）を開かない。`sai-moon-link phone …` / `sai-moon-paper phone …` の JSON・書き出し・登録簿・`apps/radio` の HTTP API だけを使う。
- **足りない処理はコアに足す。** Web に要る処理が TUI の中にしか無いときは、`sai-link-widgets` か `sai-paper-core` へ移し、TUI も同じ関数を使うようにしてから phone CLI で出す。`apps/phone` や CLI に処理を写さない。
- **外から来る値は、CLI を呼ぶ前に `apps/phone` が検査する。** 通らない値では CLI を呼ばない（400）。ファイルのパスは iPhone から受け取らない。
- **依存を増やさない。** サーバは FastAPI と標準ライブラリ、画面は素の JavaScript。npm などで新しいパッケージを入れない。
- iPhone（幅 390px）と PC（幅 1280px・900px 以上は 2 列）の両方で使える画面にする。

### 置き場

- **利用者全体の置き場は `~/.sai-moon/`。** 環境変数 `SAI_SAI_HOME` があればそれが優先、無ければ `~/.sai-moon`。
  Rust 側は `sai_link_engine::project::sai_home()`（`crates/sai-link-engine/src/project.rs:79`）と `sai_paper_core::paths::sai_home()`（`crates/sai-paper-core/src/paths.rs:172`）、`apps/phone` 側は `registry.sai_home()`（`apps/phone/src/sai_phone/registry.py:41`）。**3 つの値を必ずそろえること。**
  この中に `registry.json`・`phone/runs`・`cc-cost`・共通の `blocks/`・`template/`・`settings.json`・`paper.env` が入る。
- **プロジェクトの中の `<root>/.sai/` は変わらない。** `sai-engine.db`・`blocks/`・`outputs/` はここ。
- `apps/phone/src/sai_phone/usage.py` の `cc_limits_path()` は `<sai_home>/phone/cc-rate-limits.json` を読む（書くのは `sai-moon-link cc-statusline`）。旧 `~/.sai/phone/` は読まない。

### 画面のコード

- 新しい画面は `window.SaiRoutes` に登録する。`app.js` に画面ごとの分岐を足さない。
- `ui.js` の部品（`saiApi`・`saiToast`・`saiConfirm`・`saiSheet`・`saiEmpty`・`saiTerm` など）と、`markdown.js` の `renderMarkdown` を使う。同じ物を作り直さない。
- 一番外側の関数・変数には画面ごとの頭（`sai`・`hm`・`wf`・`df`・`pim` など）を付ける。`apps/phone/tests/test_globals.js` で重なりを検査している。
- 画面ファイルを変えて配るときは、`sw.js` の `CACHE` と `index.html` の `?v=` をそろえて上げる。ファイルを足したら `ASSETS` にも足す。

### テストと確認

```bash
cd ~/workdir/Sai/sai-moon

# Rust（環境変数が混ざっていると History テスト 1 件が落ちる。必ず env -u で外す）
env -u SAI_PROJECT_ROOT -u SAI_PARENT_RUN_ID -u SAI_BLOCK_NAME \
  perl -e 'alarm 1800; exec @ARGV' cargo test --workspace

# Web（JS 24 本）
for f in apps/phone/tests/test_*.js; do node "$f" || exit 1; done

# Web（Python 18 本）
for f in apps/phone/tests/test_*.py; do PYTHONPATH=apps/phone/src apps/phone/.venv/bin/python "$f" || exit 1; done

# Radio（3 本）
(cd apps/radio && for f in tests/test_*.py; do PYTHONPATH=src .venv/bin/python "$f" || exit 1; done)

# 実験ループの道具（sai-sci/tools。Python 3.11 以上。`~/.python-version` が未導入の版を指すと uv が失敗するので --python を付ける）
(cd sai-sci/tools && for t in test_exp_exec test_exec_ssh test_exec_dok test_dok; do python3.13 -W ignore -m unittest $t || exit 1; done)
(cd sai-sci/tools && uv run --no-project --python ">=3.11" --with mlflow python -m unittest test_exp_summary)
```

- **本番の旧 sai-phone（8100 番）と旧 sai-radio（8000 番）は、確認のために止めたり再起動したりしない。** sai-moon の確認は 8200 / 8201 / 8202 番で行う。
- `apps/phone` と `apps/radio` の仮想環境は `uv sync` 済み（`apps/phone/.venv`・`apps/radio/.venv`）。`apps/phone` は `pyproject.toml` と `uv.lock`、`requirements.txt` は無い。
- サーバ・ブラウザ・ネットワークを使う命令には、必ず時間の上限を付ける（例 `perl -e 'alarm 180; exec @ARGV' <命令>`）。上限の無い命令を `| tail` などにつないで待たない。
- 背景で起動したサーバとブラウザは、作業を終える前に止める。
- **`sai-moon-link daemon start`・`launchctl load`・cc フックや pi 拡張の登録は実行しない。** 文書を書くだけで済ませる。
- **`apps/mlflow-server` の `docker compose up / down / restart` は、確認のために実行しない。** 動いている本物の MLflow（実験の記録）を止める。`docker compose config` のような読み取りだけにする。
- **さくらの DOK（`sai-sci/exec/dok`）の `run` / `ssh-run` は実行しない。** 課金される。確認は偽の `dok.py` を使うテストで行う。
- `sai-moon-link init` など、プロジェクトを書き換える sai-link の命令は実行しない。
- 書き込みを確かめるときは、確認用の物（例: block `guide-check`・確認用の .bib）か一時フォルダを使い、最後に消す。本物の `.sai/blocks/*.md`・`~/.sai-moon/`・`papr.db` の中身を確かめのために変えない。

### 他のフォルダを直すとき

- 約束は `docs/for-sai-developers.md` に従う。`apps/phone` が読む JSON の項目を変えたら、その文書と `apps/phone` を一緒に直す。
- 旧 4 リポジトリ（`~/workdir/Sai/sai-link`・`sai-paper`・`sai-phone`・`sai-radio`）は**読み取り専用**。sai-moon がコピー元。ここを直さない。
  着手前から未 commit の変更が残っているので、触る前に `git -C ~/workdir/Sai/<名前> status --short` を控えておき、終わりに同じであることを確かめる。
- `cargo build` は本体フォルダで行い、`target/release/sai-moon-paper` を必ず作り直す。`apps/phone` は release の実行ファイルを叩く（`SAI_PAPER_BIN` の既定）。
