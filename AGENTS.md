# 概要

プロジェクト名: `agent-link`
プロジェクトルート: `~/workdir/Sai/agent-link`
目的: アプリ（主に sai-tree）を開発するための道具。Mac で 1 人が使う（単独モード）。実行エンジン（run・カード・文書・子タスク・daemon）と Web（単独モード）だけを持つ。sai-tree（`~/workdir/Sai/sai-tree`）から複製した。

構成は 2 つ。Rust の実行エンジン（実行ファイル `agent-link`）と、Web の窓口（`apps/web`・実行ファイル `agent-link-web`）。

作業の前に `ARCHITECTURE.md` の §1（考え方）と §2（大原則）を読むこと。

## フォルダ

| フォルダ | 中身 |
|---|---|
| `crates/` | Rust のクレート 3 つ。`agent-link-engine`（実行エンジン。DB・常駐・パース・遠隔の子）・`agent-link-view`（CLI が使う表示用の組み立て。描画は持たない）・`agent-link-cli`（実行ファイル `agent-link`） |
| `apps/web/` | Web の窓口（FastAPI と素の JavaScript）。パッケージは `src/agent_link_web/` |
| `waggle/` | vendor クレート（原則編集しない） |
| `scripts/` | 入れ方・点検・消し方の台本 `agent-link-setup`（`install`・`doctor`・`uninstall`） |
| `docs/` | 設計・契約・決定・手順書。`docs/plan/` は計画書（**終わった計画は読むだけ。進行中の計画は書いてよい**） |

## 読む文書

| 文書 | 中身 |
|---|---|
| `README.md` | 入れ方（台本）・最初の一歩・ポート・環境変数と `config` の表・文書の一覧 |
| `docs/design.md` | 現在の設計（不変条件・層と境界・守る良い所・やらないこと） |
| `docs/for-developers.md` | **契約の正**（CLI の終了コード・標準出力と標準エラー・`--json`・`schema --json`・プロジェクトの決め方・`AGENT_LINK_HOME` の規則・Web が読む JSON の項目・`registry.json` と `settings.json` の形） |
| `docs/integration.md` | **契約の正**（Web の HTTP の住所と画面の住所の一覧） |
| `docs/decisions.md` | 過去の決定（日付・理由・状態）。決定を変えるときは古い行を「やめた」にして新しい行を足す |
| `ARCHITECTURE.md` | 層・置き場（`AGENT_LINK_HOME`）・ファイル構成・機能を足す手順 |
| `CONCEPT.md` | 概念と用語・画面に出す専門用語の一覧 |
| `docs/remote.md` | 遠隔の機械で子の claude を動かす手順書（利用者が行う） |
| `docs/switch-over.md` | sai-moon と agent-link を並べて使う手順書（利用者が行う） |
| `docs/separation.md` | sai-tree と agent-link を同じ Mac で分けて使う片付けの手順書（利用者が行う。点検・決めること・片付け・戻し方） |
| `crates/agent-link-engine/assets/template/ext-contract.md` | 拡張（extension）の契約の正（宣言・置き場・CLI の作法・Web の住所・画面の約束）。拡張の作者向け |

`agent-link` などは PATH に無いことがある。`README.md` の「入れ方」の PATH の足し方を見ること。

## 作業の約束

- **`apps/web` は DB（`agent-link.db`）を開かない。** CLI（`agent-link`）の JSON・書き出し・登録簿だけを使う。足りない処理はコアに足して CLI から出す（`ARCHITECTURE.md` §2.4）。
- **CLI を直したら** 終了コード・`--json`・標準出力と標準エラーの約束を守る（全文は `docs/for-developers.md`）。標準エラーの最後の行と `--json` の人向けの文は英語で書く（clap の誤りの頭は `Invalid arguments: `）。`apps/web` が読む JSON の項目を変えたら、`docs/for-developers.md` と `apps/web` を一緒に直す。
- **`AGENT_LINK_HOME` の規則は 2 か所にある**（`crates/agent-link-engine/src/project.rs` の `resolve_sai_home`・`apps/web/src/agent_link_web/registry.py` の `sai_home()`）。そろえて直す（違いは `HOME` が無いときだけ。Rust は `None`、Python は `Path.home()`）。設定は `$AGENT_LINK_HOME/settings.json` の `config` にも書ける（環境変数 → `config` → 既定値。`AGENT_LINK_HOME` は書けない。表は `README.md`）。
- **依存を増やさない。** サーバは FastAPI と標準ライブラリ、画面は素の JavaScript。npm などで新しいパッケージを入れない。
- iPhone（幅 390px）と PC（幅 1280px・900px 以上は 2 列）の両方で使える画面にする。外から来る値は CLI を呼ぶ前に `apps/web` が検査する（通らなければ 400）。ファイルのパスは iPhone から受け取らない。
- 編集の注意:
  - `waggle/` は vendor クレート。原則編集しない。
  - `.agent-link/` はこのプロジェクト自身のデータ。書き込みに注意する。sai-moon が書いた古い記録のフォルダは、agent-link は読まない。

### 画面のコード

- 新しい画面は `window.SaiRoutes` に登録する。`app.js` に画面ごとの分岐を足さない。
- 画面の文言は英語にする（音声の言語は日本語）。Web の `detail`（エラーの文）も英語。文書とエージェント向けの文は日本語のまま。
- `ui.js` の部品（`saiApi`・`saiToast`・`saiConfirm`・`saiSheet`・`saiEmpty`・`saiTerm` など）と、`markdown.js` の `renderMarkdown` を使う。同じ物を作り直さない。
- 一番外側の関数・変数には画面ごとの頭（`sai`・`hm`・`wf`・`df` など）を付ける。`apps/web/tests/test_globals.js` で重なりを検査している。
- 画面ファイルを変えて配るときは、`sw.js` の `CACHE` と `index.html` の `?v=` をそろえて上げる。`apps/web/tests/test_migrate.js` が版の数字（`agent-link-web-v<数>`・`?v=<数>`）を固定しているので、それも合わせる。ファイルを足したら `ASSETS` にも足す。

### テストと確認

テストの命令は次のとおりで、CI（`.github/workflows/ci.yml`）も同じ命令を回す。命令を直したら両方直す。

```bash
cd ~/workdir/Sai/agent-link

# Rust（864 件。--no-fail-fast は最初の失敗で止めず、全部の本数を数えるため）
perl -e 'alarm 1800; exec @ARGV' cargo test --workspace --no-fail-fast

# Web（JS 24 本）
for f in apps/web/tests/test_*.js; do node "$f" || exit 1; done

# Web（Python 23 本）
for f in apps/web/tests/test_*.py; do PYTHONPATH=apps/web/src apps/web/.venv/bin/python "$f" || exit 1; done
```

| 対象 | 本数 | CI のジョブ |
|---|---|---|
| Rust（`cargo test --workspace --no-fail-fast`） | 864 件（ほかに ignored 3 件） | Rust（ubuntu） |
| Web の JS | 24 本 | Web（ubuntu） |
| Web の Python | 23 本 | Web（ubuntu） |

- 環境変数の名前は `AGENT_LINK_*` だけである。sai-moon が入れる環境変数は名前が違うので混ざらず、`env -u` で外す必要は無い。
- CI の Linux（ubuntu-latest）で同じ命令が通る。リポジトリ直下に `tests/` は無い。テストはクレート・アプリの中にある。
- Mac で確かめるときも、上の命令を上から順に実行する。`apps/web` の仮想環境は `(cd apps/web && uv sync)` で作る（`pyproject.toml` と `uv.lock` を使う）。
- **Web を確かめるサーバは、一時フォルダの置き場（`AGENT_LINK_HOME`）で起こす。** 確認用は 8297 番で起こし、終わったら必ず止める。
  ```bash
  T=$(mktemp -d)
  AGENT_LINK_WEB_NO_WATCH=1 AGENT_LINK_HOME="$T" perl -e 'alarm 180; exec @ARGV' apps/web/bin/agent-link-web serve --port 8297 &
  # … curl http://127.0.0.1:8297/ などで確かめる …
  kill %1; rm -rf "$T"
  ```
- **動いている Web（Mac の 8400 番）は、確認のために止めたり再起動したりしない。** 確認は 8297 番で行う。
- サーバ・ブラウザ・ネットワークを使う命令には、必ず時間の上限を付ける（例 `perl -e 'alarm 180; exec @ARGV' <命令>`）。背景で起動したサーバとブラウザは、作業を終える前に止める。
- `cargo build --release` は本体フォルダで行い、`target/release/agent-link` を必ず作り直す。`apps/web` は release の実行ファイルを叩く。

### 実行しないこと

- 本物のプロジェクトでの `agent-link init`（`--update` を含む）。ただし、利用者が頼んだときに限り、agent-link 自身のルート（`~/workdir/Sai/agent-link`）で `agent-link init --update` を実行してよい（`.agent-link/cc-settings.json` のフックを release のバイナリに合わせるため。run を始めるたびにも合わせ直す）。sai-tree などほかのプロジェクトでは実行しない。ただし例外が 1 つある: extensions リポジトリ（`~/workdir/Sai/agent-link-extensions`）は、agent-link のプロジェクトとして `agent-link init` してよい（2026-10-18 利用者の決定。拡張はそこで作る）。確かめるときは一時フォルダをプロジェクトにし（`AGENT_LINK_PROJECT_ROOT`）、`AGENT_LINK_HOME` にも別の一時フォルダを入れ、最後に消す。
- 利用者の環境（`~/.agent-link`・シェルの設定・`~/.local/bin`）を変える操作。文書に書くだけにする。`scripts/agent-link-setup install`・`uninstall` を利用者の環境で実行しない（`doctor` は読むだけなので実行してよい）。確かめるときは、一時フォルダを `HOME`・`AGENT_LINK_HOME`・`--bin-dir` に向けて行い、最後に消す。
- `agent-link remote setup`（Mac の `~/.ssh/authorized_keys` と遠隔の機械を書き換える）。実機での確認は利用者が行う（`docs/remote.md`）。
- 本物の `.agent-link/blocks/*.md`・`~/.agent-link/` の中身を、確かめのために変えること。書き込みの確認は一時フォルダで行い、最後に消す。
- sai-moon（`~/workdir/Sai/sai-moon`）と sai-tree（`~/workdir/Sai/sai-tree`）のリポジトリを直すこと。読み取り専用。
- 動いている 8400 番の Web を止めること。
- `pkill -f` のような広い止め方をしない。止めるのは自分で起こした PID だけ。
