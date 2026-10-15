# 実装ガイド 1/2: agent-link から sai-link と Web 以外を消す（U01〜U04）

## ゴール
`~/workdir/Sai/agent-link`（sai-tree の複製）から、sai-paper・sai-sci・Radio・MLflow・sai-ts・sai-prep・Customer サポート・多人数モードを消す。sai-link と Web（単独モード）だけが残り、Rust・Web の JS・Web の Python のテストが通る。名前（`sai-link`・`SAI_*`・`.sai/`）はまだ変えない（ガイド 2 で変える）。

完了したときに利用者が確かめられること:
- `ls crates` が `sai-link-cli sai-link-engine sai-link-view`、`ls apps` が `web` だけ。
- 一時フォルダの置き場で Web を起こすと、下のタブが Home・Projects・Chats・Account の 4 つ。

## 作業の場所と渡し方
- このガイドと計画書は、agent-link のリポジトリの `docs/plan/` にある（waggle のトークンはプロジェクトをまたげないので、パスで渡す）。worker には、計画書のパスを `task_text` に書いて渡す。例: 「`docs/plan/unit-U01-strip-rust.md` を読み、それに従って実装せよ」。
- agent-link は sai-moon のプロジェクトとして動いている。worker の作業用コピー（worktree）は agent-link のリポジトリにできる。
- **worker の環境変数**: sai-moon が `SAI_PROJECT_ROOT`・`SAI_PARENT_RUN_ID` などを入れている。このガイドの間はコードがまだ `SAI_*` を読むので、テストは計画書の検証どおりに `env -u` を付けて回す。`init` は一時フォルダでだけ行う。各 worker の `task_text` に、この 2 点を書く。
- worker の runner は `cc` にする。

## 実装計画書の一覧
| ID | 件名 | 文書 | 書くファイル | 依存 | 並列 |
|---|---|---|---|---|---|
| U01 | Rust から sai-paper・sai-sci・研究向けの部分・Customer の block を消す | `docs/plan/unit-U01-strip-rust.md` | `Cargo.toml`・`Cargo.lock`・`crates/**`・`sai-sci/` | - | 可 |
| U02 | Web のサーバ（Python）を単独モードの sai-link の窓口だけにする | `docs/plan/unit-U02-strip-web-server.md` | `apps/web/src/sai_web/*.py`・`apps/web/tests/test_*.py`・`apps/web/server/`・`apps/web/hooks/*.py`・`apps/web/.env.example` | - | 可 |
| U03 | Web の画面（JS・CSS・HTML）を sai-link の画面だけにする | `docs/plan/unit-U03-strip-web-screens.md` | `apps/web/src/sai_web/static/**`・`apps/web/tests/test_*.js` | - | 可 |
| U04 | ほかのアプリ・研究の文書・CI のジョブを消す | `docs/plan/unit-U04-strip-apps-ci.md` | `apps/{radio,ts,prep,mlflow-server}/`・`docs/server.md`・`docs/sai-sci*.md`・`.github/workflows/ci.yml`・`.dockerignore`・`.gitignore` | - | 可 |

## 実行順序
4 つとも同時に始めてよい。書くファイルが重ならない（下の所有権の表）。

マージの順: U04 → U01 → U02 → U03。消すだけの U04 を先に入れる。U02 と U03 は API の住所と JSON の形（共有契約 C3）で結ばれているので、2 つを入れたあとに Web をまとめて確かめる。

## 所有権の表
| ファイル | U01 | U02 | U03 | U04 |
|---|---|---|---|---|
| `Cargo.toml`・`Cargo.lock`・`crates/**`・`sai-sci/` | 書く | | | |
| `apps/web/src/sai_web/*.py`・`apps/web/hooks/`・`apps/web/server/`・`apps/web/.env.example`・`apps/web/tests/test_*.py` | | 書く | | |
| `apps/web/src/sai_web/static/**`・`apps/web/tests/test_*.js` | | | 書く | |
| `apps/radio`・`apps/ts`・`apps/prep`・`apps/mlflow-server`・`docs/server.md`・`docs/sai-sci*.md`・`.github/**`・`.dockerignore`・`.gitignore` | | | | 書く |
| `docs/plan/**`・`AGENTS.md`・`README.md`・`ARCHITECTURE.md`・`CONCEPT.md`・`docs/integration.md`・`docs/for-sai-developers.md` | | | | |

最後の行は、このガイドでは誰も書かない（ガイド 2 の U07 が書き直す）。

## 共有契約
各計画書に全文を写してある。
- C1: CLI（`sai-link`）の JSON から消してよい項目は、`after_job`・`target_node`・`layer`・台帳の数・Customer フォルダ（id `-1004`）だけ。このガイドでは名前を変えない。（U01・U02・U03・U04）
- C3: Web の残す API の住所と JSON の形は変えない。消えるのは、`GET /api/home` の `papers`、`GET /api/days` の `customer`、`GET /api/projects` の `container_state`・`container`、`GET /api/auth/me`（route ごと）だけ。（U02・U03）
- C4: 画面の版は `sai-web-v132`・`?v=132`。（U03）
- C5: `.gitignore` の `.sai/` の行は残す。（U04）

## 全体の完了条件
| 条件 | 担当 ID | 確認方法 |
|---|---|---|
| crate は sai-link の 3 つだけ、`apps/` は `web` だけ | U01・U04 | `ls crates apps` |
| Rust のテストが通る | U01 | 下の「統合と検証」1 |
| Web の JS・Python のテストが通る | U02・U03 | 下の「統合と検証」1 |
| 研究・多人数・Customer の画面と API が無い | U02・U03 | 下の「統合と検証」2 |
| CI のジョブが `rust` と `web` だけ | U04 | `ruby -ryaml -e 'p YAML.load_file(".github/workflows/ci.yml")["jobs"].keys'` |

## 統合と検証
全部をマージしたあとに、agent-link の main で kantoku が行う。

1. テスト:
   ```bash
   cd ~/workdir/Sai/agent-link
   env -u SAI_PROJECT_ROOT -u SAI_PARENT_RUN_ID -u SAI_BLOCK_NAME -u SAI_TARGET_NODE \
     perl -e 'alarm 1800; exec @ARGV' cargo test --workspace
   (cd apps/web && uv sync)
   for f in apps/web/tests/test_*.js; do node "$f" || exit 1; done
   for f in apps/web/tests/test_*.py; do
     env -u SAI_LINK_BIN -u SAI_PAPER_BIN -u SAI_SCI_BIN -u SAI_PROJECT_ROOT -u SAI_PARENT_RUN_ID \
       PYTHONPATH=apps/web/src apps/web/.venv/bin/python "$f" || exit 1
   done
   ```
2. 画面（8297 番。8299 番は使用中。必ず止める）:
   ```bash
   cargo build --workspace
   B="$PWD/target/debug/sai-link"
   T=$(mktemp -d); P=$(mktemp -d)/check; mkdir -p "$P"; (cd "$P" && git init -q)
   (cd "$P" && env -u SAI_PROJECT_ROOT -u SAI_PARENT_RUN_ID SAI_HOME="$T" "$B" init --name check)
   env -u SAI_PROJECT_ROOT -u SAI_PARENT_RUN_ID SAI_WEB_NO_WATCH=1 SAI_HOME="$T" SAI_WEB_TOKEN= SAI_LINK_BIN="$B" \
     perl -e 'alarm 300; exec @ARGV' apps/web/bin/sai-web serve --port 8297 &
   ```
   playwright で `http://127.0.0.1:8297/` を幅 390px と 1280px で開く。確かめること: 下のタブが Home・Projects・Chats・Account の 4 つ。Home・Projects（`check`）・Chats・Account の Usage が開く。console に error が無い。最後に `kill %1; rm -rf "$T" "$(dirname "$P")"`。
3. 作業用コピー（worktree）とブランチを片付ける。

## 決定事項と理由 / 採用しなかった案
- 元のコードは sai-tree（sai-moon ではない）。sai-link が新しく、Mac 専用の機能もすでに無い（利用者の選択）。
- 研究向けの機能は、隠すのではなく消す。sai-tree の固定版をそのまま使う案は、利用者が採らなかった。
- 文書（`AGENTS.md` など）は、このガイドでは直さない。ガイド 2 で名前を変えるときに、まとめて書き直す。
- Mac 専用の機能（launchd・承認・遠隔ブラウザ・5 時間枠・pi）は戻さない（利用者の選択）。

## リスクと対処
- **worker が本物のプロジェクトで `init` する**: 前に一度起きた。`task_text` に「`init` は `mktemp -d` の一時フォルダでだけ、環境変数を `env -u` で外して行う」と書く。
- **消した物を残す側が使っていた**: 計画書の「直す」の一覧から漏れていたら、worker が直してよい。所有権の外のファイルなら、kantoku が担当の worker に直させる。
- **`style.css` の消しすぎ**: 統合の画面の確認で見つける。崩れていたら U03 の worker に直させる。
- **kantoku が自分で決めてよい範囲**: テストの直し方、計画書の行番号のずれ、所有権の中での小さな追加の削除。
- **利用者に聞く範囲**: 残すと決めた機能（Chats・Workflow・Usage・質問・音声入力など）が、消さないと動かないと分かったとき。`block02_rust-best-practices.md` の扱い。

## 参照文書
- 要件定義書: `docs/plan/requirements.md`（sai-tree プロジェクトの run 289 にも同じ物を保存してある）
- 調査報告書 X1・X3: sai-tree プロジェクトの waggle:MizF9uqb・waggle:m88AApdz（このプロジェクトからは読めない。要点は計画書に写してある）
