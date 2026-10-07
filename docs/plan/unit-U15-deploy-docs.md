# 実装計画書 U15: 配備（deploy.sh）・CI・文書の書き直し

## 目的
1. Mac で開発・テストした sai-tree を、Linux のサーバへ 1 本の命令（`deploy.sh`）で反映できるようにする。
2. CI（`.github/workflows/ci.yml`）を、本番と同じ Linux で回すジョブと、sai-prep のテストを含む形にする。
3. sai-tree の文書（`README.md`・`ARCHITECTURE.md`・`AGENTS.md`・`docs/`）を、headless・多人数モード・コンテナ・sai-prep の仕組みに合わせて書き直す。sai-moon から引き継いだ Mac 専用の記述を消す。

## 背景
- sai-tree は sai-moon を複製した Linux の多人数サーバ版である。ガイド 1〜5 の U01〜U14 で、herdr の削除・headless・Chats・アカウント・コンテナ・前処理の束・sai-prep ができている。
- リポジトリには外部の git の remote が無い（sai-moon も無かった）。配備は、サーバの上の bare リポジトリへ Tailscale の ssh で push し、サーバで deploy の手順を走らせる形にする。
- 今の CI は `macos-latest` で、Rust・Web・Radio・sai-sci の道具の 4 つのジョブを回す（`.github/workflows/ci.yml`）。AGENTS.md の「テストと確認」と同じ命令にそろえる約束がある。
- 文書の中の herdr・承認・launchd・遠隔ブラウザ・cc-statusline・キーチェーンの記述の場所は、調査 E3（`docs/plan/research/E3-mac-only-removal.md` の §1.4・§2.2・§4.2・§5.2・§5.6）にある。`sai-sci/blocks/professor.md:155`・`professor-backward.md:173`・`operator.md:37`・`kaizen.md:68-69`・`sai-sci/template/sci-cli.md:67` に `sai-web approvals` / `approve` と `~/.sai/cc-cost` の案内がある。

## 前提（着手時点のコードの状態）
- U12・U13・U14 が終わっている（ガイド 5 の最後の作業）。
- `docs/server.md` は U08 が書いた（入れ方の 14 項目）。

## 変更内容

### 1. 配備 — `apps/web/server/deploy.sh`（新規。POSIX sh）と `docs/server.md` の「更新のしかた」
- Mac 側の使い方（`docs/server.md` に書く）:
  1. 初回だけ: サーバで `sai` として `git init --bare /srv/sai/repo.git`。Mac で `git remote add server ssh://sai@<マシン名>/srv/sai/repo.git`（Tailscale の ssh）。サーバの作業用のリポジトリ `/srv/sai/app` は `git clone /srv/sai/repo.git /srv/sai/app`。
  2. 毎回: Mac でテストを全部通す → `git push server main` → `ssh sai@<マシン名> /srv/sai/app/apps/web/server/deploy.sh`。
- `deploy.sh` がすること（`set -eu`。各段で何をしているかを表示する）:
  1. `cd /srv/sai/app && git pull --ff-only`。
  2. `docker build -f apps/web/server/Dockerfile -t sai-tree:latest .`（上限 60 分）。
  3. `(cd apps/web && uv sync)`。
  4. `(cd apps/prep && uv build)`（sai-prep の入れ物を `apps/prep/dist/` に作る。U13 の `GET /api/prep/download` が配る）。
  5. `sudo systemctl restart sai-web`（`sai` に、この 1 つだけを許す sudoers の行を `docs/server.md` に書く）。
  6. 「動いていないコンテナを新しいイメージで作り直すには、Projects の『起動』を押すか、管理画面で止めてから起動する」と表示する。**実行中のエージェントがいるコンテナには触らない**（daemon を止めると、動いていた headless の実行は失敗の扱いになるため。要件の範囲外）。
- 配備の前に `/srv/sai/users` のバックアップを取る手順（`tar` か `rsync` の 1 行）を `docs/server.md` に書く。

### 2. CI — `.github/workflows/ci.yml`
- Rust・Web のジョブを `ubuntu-latest` にする（本番と同じ Linux）。sai-tree は Mac の herdr・Chrome を使わないので、Mac で回す理由が無い。Radio・sai-sci の道具も `ubuntu-latest` にしてよい（落ちる物があれば `macos-latest` のまま残し、理由をコメントに書く）。
- ジョブ「sai-prep」を足す: `actions/setup-python`（3.12）→ `cd apps/prep && python -m unittest discover -s tests`。
- Web のジョブに `sh apps/web/server/test_sai_in_container.sh` を足す。
- 冒頭のコメント（「利用者の環境（Mac 上の道具）に合わせて macOS で動かす」）を直す。
- 外部の remote が無いので、CI は GitHub に置いたときに動く。置くかどうかは利用者が決める（文書にそう書く）。

### 3. 文書
- `AGENTS.md`: 概要を sai-tree（Linux の多人数サーバ版。sai-moon の複製）に書き直す。フォルダの表に `apps/web/server/`・`apps/prep/`・`docs/plan/` を足し、`launchd` を消す。テストの表と「テストと確認」に sai-prep のテストと `test_sai_in_container.sh` を足し、JS・Python のテストの本数を数え直す。herdr・launchd・`~/Library/LaunchAgents/`・本番の 8100 / 8000 番の記述を、sai-tree に合う形（本番は Linux の 8200 番。確かめは Mac の 8299 番）に直す。旧 4 リポジトリの記述を「sai-moon（`~/workdir/Sai/sai-moon`）は読み取り専用。sai-tree はその複製」に替える。
- `README.md`: 3 プロセスの起動手順を、「開発（Mac・単独モード）」と「サーバ（Linux・多人数モード。`docs/server.md`）」の 2 つに分ける。必要な物から herdr・Chrome を消す。環境変数の表に `SAI_WEB_USERS_ROOT`・`SAI_WEB_COOKIE_SECURE`・`SAI_IMAGE`・`SAI_DOCKER_BIN`・`SAI_CONTAINER_MEMORY`・`SAI_CONTAINER_CPUS`・`SAI_WEB_SKEL_DIR`・`SAI_WEB_URL_FOR_CONTAINERS`・`SAI_CLAUDE_BIN`・`SAI_PI_BIN`・`SAI_WEB_INTERNAL_TOKEN`・`SAI_PREP_DIST` を足し、`HERDR_BIN`・`SAI_WEB_CHROME`・`SAI_WEB_BROWSER_HEADED` を消す。sai-prep の入れ方（Web から入れ物を落として `uv tool install`、`sai-prep setup`）の節を足す。
- `ARCHITECTURE.md`:
  - §1 の層の図に、多人数モード（ブラウザ → Tailscale serve → sai-web → 中継 → プロジェクトのコンテナ（daemon・headless のエージェント））と、各自の Mac の sai-prep（→ 束 → sai-web → コンテナの `sai-paper paper import-bundle`）を足す。
  - §2 の合言葉の項に、多人数モードのログイン（メールアドレス・Cookie）・個人のトークン・内部の合言葉を足す。
  - §3 に、多人数モードの置き場（C4）を足す。
  - §4 の表に `auth.py`・`containers.py`・`bundles.py`・`account.js`・`newchat.js` を足し、`herdr.py`・`approvals.py`・`rbrowser.py` を消す。engine の headless（`headless.rs`・擬似 pane・`state.json`）を、herdr の説明と入れ替える。
  - §8 に範囲外（要件定義書の一覧）を足す。§9 の経緯に「sai-moon から複製して sai-tree を作った」を 1 行足す。
- `docs/integration.md`: API の表が U03・U05〜U13 で足した・消した住所と一致しているか確かめ、直す。新しい画面の住所（`#/login`・`#/password`・`#/account`・`#/admin`）を足す。
- `docs/for-sai-developers.md`: herdr・承認・statusLine の記述を消し、C1（`state.json`）・C2（`run send` / `run interrupt`）・C3（フックと内部の合言葉）・C12（束）・C13 を足す（U02・U03 が足した物と重複しないように）。
- `CONCEPT.md`（概念と用語・画面に出す専門用語の一覧。旧 `docs/glossary.md` を統合した物）: 「多人数モード」「単独モード」「headless」「擬似 pane」「内部の合言葉」「個人のトークン」「前処理の束」「sai-prep」「PDF 待ち」を足し、herdr・承認の語を消すか「sai-tree では使わない」と書く。
- `docs/sai-sci.md`・`docs/sai-sci-tutorial.md`（research-tree の使い方と練習）の中の Mac・herdr・launchd を前提にした手順を、sai-tree の形（Web とコンテナ）に直す。`docs/` に sai-moon の Mac の運用だけの文書が残っていれば消す（消す前に中身を見て、sai-tree に要る記述があれば `docs/server.md` に移す。消した物を作業報告書に書く）。
- `sai-sci/blocks/professor.md:155`・`professor-backward.md:173`・`operator.md:37`・`kaizen.md:68-69`・`sai-sci/template/sci-cli.md:67` の `sai-web approvals` / `approve` と `~/.sai/cc-cost` の案内を、sai-tree の形（承認は無い。費用は `sai-link usage` と Web の Usage）に直す。

## 共有契約（他の作業と共有する取り決め。worker は変えない）
- なし（文書と配備だけ）。文書に書く API・置き場・束の形は、U01〜U14 の計画書の共有契約 C1〜C14 と一致させる（`docs/plan/` にある）。

## ファイルの所有権
- 書いてよい: `apps/web/server/deploy.sh`（新規）・`docs/server.md`（「更新のしかた」と「バックアップ」の節を足す）・`.github/workflows/ci.yml`・`AGENTS.md`・`README.md`・`ARCHITECTURE.md`・`docs/`（`docs/plan/` を除く）・`sai-sci/blocks/`・`sai-sci/template/`（上に挙げた箇所だけ）
- 触らない: コード（`crates/`・`apps/web/src/`・`apps/prep/src/`）・`docs/plan/`

## 完了条件
- [ ] AC1: `sh -n apps/web/server/deploy.sh` が通る。`docs/server.md` に初回の bare リポジトリの用意・毎回の手順・sudoers の 1 行・バックアップがある。
- [ ] AC2: `.github/workflows/ci.yml` の命令と AGENTS.md の「テストと確認」が一致する（sai-prep と `test_sai_in_container.sh` を含む）。AGENTS.md の命令を Mac で上から順に実行して、すべて通る。
- [ ] AC3: `grep -rn "herdr\|launchctl\|rbrowser\|cc-statusline\|approvals" AGENTS.md README.md ARCHITECTURE.md docs/ sai-sci/blocks sai-sci/template --include=*.md` の結果が、「sai-tree では使わない」と書いた箇所と経緯の記述だけ（`docs/plan/` は除く）。
- [ ] AC4: 文書の API の表と、`apps/web/src/sai_web/main.py` の住所が一致する（作業報告書に、照らし合わせた方法を書く）。

## 検証
```bash
cd ~/workdir/Sai/sai-tree
sh -n apps/web/server/deploy.sh
# AGENTS.md の「テストと確認」をすべて実行する
```

## 決定事項と理由
- 配備は bare リポジトリへの push と ssh の 1 命令 — 理由: 外部の remote が無くても動く。GitHub の self-hosted runner などの自動化は、GitHub に置くと決めてから足す（ponytail: 手動の 1 命令で足りる間は自動化しない）。
- CI を Linux にする — 理由: 本番が Linux で、sai-tree は Mac の道具を使わない。
- 実行中のコンテナは配備で作り直さない — 理由: daemon を止めると動いていた headless の実行が失敗する（要件の範囲外にした引き継ぎ）。

## 要確認
- GitHub（private）に置くかどうか（置かなければ CI は動かない）。この計画では決めず、文書に選択肢として書く。
