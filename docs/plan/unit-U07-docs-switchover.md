# 実装計画書 U07: 文書・CI を agent-link に合わせ、切り替えの手順書を書く

## 目的
agent-link のリポジトリの文書（`AGENTS.md`・`README.md`・`ARCHITECTURE.md`・`CONCEPT.md`・`docs/`）と CI を、ガイド 1・2 のあとのコードに合わせて書き直す。利用者が sai-moon から agent-link に切り替えるための手順書 `docs/switch-over.md` を新しく書く。

## 背景
- agent-link は sai-tree を複製し、sai-link と Web だけを残して、名前を agent-link に変えた開発用の道具である（要件定義書 `docs/plan/requirements.md`）。文書は sai-tree のまま（論文・Research・多人数・サーバの説明）残っている。
- 今の Mac の開発の道具は sai-moon（`~/workdir/Sai/sai-moon`）である。PATH の `sai-link`、8200 番の Web、daemon、8201 番の Radio は、どれも sai-moon から動いている。切り替えは利用者が手順書を見て行う。agent や worker は切り替えを実行しない。
- 利用者と決めたこと: sai-moon は止めて、フォルダとデータは残す。sai-tree プロジェクトの今の記録（`.sai/`）は移さず、agent-link は新しい `.agent-link/` から始める。Linux のテスト利用者は tailnet に招き、ACL で Linux の機械だけを許す。

## 前提（着手時点のコードの状態）
- U05・U06 が終わっている。Rust は `crates/agent-link-*`、実行ファイルは `agent-link`、Web は `apps/web/src/agent_link_web` と `apps/web/bin/agent-link-web`、環境変数は `AGENT_LINK_*`、プロジェクトのフォルダは `.agent-link/`、置き場は `~/.agent-link` である。
- `apps/web` の設定は `apps/web/.env` から読む（`config.py` の `load_dotenv`）。
- `CLAUDE.md` の中身は `@AGENTS.md` の 1 行だけ。
- 調べて分かっている Mac の今の状態（2026-10-15）:
  - sai-moon の daemon が 2 つ（`~/workdir/Sai/sai-moon/target/release/sai-link daemon _run`）、Web（`uvicorn sai_web.main:app --port 8200`、cwd は `~/workdir/Sai/sai-moon/apps/web`）、Radio（8201 番）が動いている。
  - `tailscale serve` は `:4443 → 127.0.0.1:8100`、`:4444 → 127.0.0.1:8200`、`:4445 → 127.0.0.1:8299` を tailnet に出している。8200 番の Web には合言葉（`SAI_WEB_TOKEN`）が設定されていない。
  - sai-moon の置き場 `~/.sai/registry.json` には、sai-moon・sai-tree・west・test-moon-* などが登録されている。

## 変更内容

### 1. `AGENTS.md`（`CLAUDE.md` はそのまま）
sai-tree の `AGENTS.md` の形（概要・フォルダ・読む文書・作業の約束・テストと確認・実行しないこと）を残し、中身を agent-link に合わせて書き直す。
- 概要: agent-link は、アプリ（主に sai-tree）を開発するための道具。Mac で 1 人が使う。sai-link（run・カード・文書・子タスク・daemon）と Web（単独モード）だけを持つ。sai-tree から複製した。
- フォルダ: `crates/`（3 crate）・`apps/web/`・`waggle/`・`docs/`。
- 作業の約束: `apps/web` は DB を開かない。CLI の約束（終了コード・`--json`）。`AGENT_LINK_HOME` の規則は 2 か所（`crates/agent-link-engine/src/project.rs` の `resolve_sai_home`・`apps/web/src/agent_link_web/registry.py` の `sai_home()`）にあり、そろえて直す。依存を増やさない。画面のコードの約束（`window.SaiRoutes`・`ui.js`・`sw.js` と `?v=` の版）。
- テストと確認: 下の命令と、実際に数えたテストの本数。Web の確認用のサーバは 8297 番で、一時フォルダの置き場（`AGENT_LINK_HOME`）で起こす。`env -u` の注記は消す（名前が分かれたので要らない）。
  ```bash
  perl -e 'alarm 1800; exec @ARGV' cargo test --workspace
  for f in apps/web/tests/test_*.js; do node "$f" || exit 1; done
  for f in apps/web/tests/test_*.py; do PYTHONPATH=apps/web/src apps/web/.venv/bin/python "$f" || exit 1; done
  ```
- 実行しないこと: `agent-link daemon start`、本物のプロジェクトでの `agent-link init`、`~/.agent-link`・`~/.sai`・シェルの設定の変更、sai-moon・sai-tree のリポジトリを直すこと（読み取り専用）、動いている 8200 番の Web を止めること。

### 2. `README.md`・`ARCHITECTURE.md`・`CONCEPT.md`
- 論文・Research・Radio・MLflow・sai-ts・sai-prep・多人数モード・サーバ・Customer の節を消す。
- 残る節の名前を C2 の対応表どおりに変える。
- `README.md` の入れ方: `cargo build --release`、`(cd apps/web && uv sync)`、PATH（`export PATH="$HOME/workdir/Sai/agent-link/target/release:$HOME/workdir/Sai/agent-link/apps/web/bin:$PATH"`）、`agent-link init`、`agent-link daemon start`、`apps/web/.env` に `AGENT_LINK_WEB_TOKEN`、`agent-link-web serve`。ポート表は 8200 番（Web）だけ。環境変数の表は U05・U06 のあとに実際に残った `AGENT_LINK_*` を書く（`git grep -ohE 'AGENT_LINK_[A-Z_]+' -- crates apps/web | sort -u` で数える。テスト専用の物は除く）。
- `ARCHITECTURE.md`: 部品は sai-link と Web の 2 つ。層の図は「開発（Mac・単独モード）」だけにする。境界と依存の向きの表は、sai-paper・sai-sci の行を消す。
- `CONCEPT.md`: research-tree・論文の用語を消す。
- 由来の説明（「sai-tree から複製した」）は 1 か所に書く。

### 3. `docs/`
- `docs/integration.md`: 残る API と画面の住所だけにする（U02・U03 で消した route と画面を消す）。
- `docs/for-sai-developers.md` を `git mv` で `docs/for-developers.md` にし、中身を agent-link の CLI の約束（Web が読む JSON の項目）だけにする。sai-sci・sai-paper・Radio の節を消す。この文書を参照している所（`AGENTS.md`・`README.md`・`ARCHITECTURE.md`・コードの注記）を全部直す。コードの注記を直す必要があれば、直すのは注記の文字列だけにする。

### 4. CI（`.github/workflows/ci.yml`）
- `rust` のジョブの `env -u SAI_PROJECT_ROOT …` を外し、`perl -e 'alarm 1800; exec @ARGV' cargo test --workspace` だけにする。
- `web` のジョブの命令を、新しいパッケージの名前で動くことを確かめる（`PYTHONPATH=apps/web/src` はそのまま）。
- 頭の注記を agent-link に合わせる。

### 5. `.gitignore`
- `.agent-link/` の行を足す。`.sai/` の行は**残す**（作業中と、切り替えるまでは sai-moon がこのリポジトリの `.sai/` に書く）。
- `init` が作る物の行（`.claude/commands/sai-finalize.md` などがあれば）を、新しい名前（`agent-link-finalize.md`）にする。

### 6. 切り替えの手順書 `docs/switch-over.md`（新しく書く）
利用者が上から順に実行する手順書。命令はそのまま貼れる形で書き、各段に「確かめ方」と「うまくいかないとき」を付ける。中身は次のとおり。

1. **前提**: agent-link の main に全部入っている。`cargo build --release` と `(cd apps/web && uv sync)` が済んでいる。
2. **動いている run が無いことを確かめる**（sai-moon の CLI で）: 各プロジェクトで `sai-link run tree` を実行し、running が空であることを見る。sai-tree と agent-link の両方で確かめる。
3. **sai-moon を止める**:
   - daemon: 各プロジェクトで `sai-link daemon stop`（sai-moon の CLI）。
   - Web: `lsof -nP -iTCP:8200 -sTCP:LISTEN` で PID を見て `kill <PID>`。
   - Radio: `lsof -nP -iTCP:8201 -sTCP:LISTEN` で PID を見て `kill <PID>`。
   - 確かめ方: `ps aux | grep sai-moon` に何も出ない。フォルダ `~/workdir/Sai/sai-moon` と `~/.sai` は消さない。
4. **PATH**: `~/.zshrc` の `…/sai-moon/target/release` を消し、`$HOME/workdir/Sai/agent-link/target/release:$HOME/workdir/Sai/agent-link/apps/web/bin` を足す。新しいシェルで `command -v agent-link agent-link-web` を確かめる。`command -v sai-link` が sai-moon を指していないことも確かめる。
5. **プロジェクトを agent-link にする**（sai-tree と agent-link の両方）:
   - `cd ~/workdir/Sai/sai-tree && agent-link init`。`.agent-link/` ができる。`.claude/settings.json` と `.mcp.json` は書き直され、前の中身は `.bak` に残る。古い `.sai/` は残る。
   - sai-tree の `.gitignore` に `.agent-link/` の 1 行を足して commit する（sai-tree への変更はこの 1 行だけ）。
   - 前の `.sai/blocks/` に自分で直した block があれば、`diff -r .sai/blocks .agent-link/blocks` で見て、必要な物を `.agent-link/blocks/` に写す。ただし、中の `sai-link` などの名前は新しい名前に直す。
   - `cd ~/workdir/Sai/agent-link && agent-link init`。
   - それぞれで `agent-link daemon start` を実行する。
   - 確かめ方: `agent-link run tree` が空の一覧を返す。
6. **Web を起こす（合言葉付き）**:
   - `openssl rand -hex 16` で合言葉を作り、`apps/web/.env` に `AGENT_LINK_WEB_TOKEN=<値>` と書く（`.env` は git に入れない）。
   - `cd ~/workdir/Sai/agent-link/apps/web && nohup .venv/bin/python -m uvicorn agent_link_web.main:app --host 127.0.0.1 --port 8200 --app-dir src > ~/.agent-link/web/uvicorn-8200.log 2>&1 &`（README の起動の命令と同じ物にする）。
   - `tailscale serve` の `:4444 → 8200` はそのまま使える。使っていない `:4443 → 8100` は `tailscale serve --https=4443 off` で閉じる。
   - iPhone と PC で `https://<Mac の名前>.<tailnet>.ts.net:4444/` を開き、合言葉を入れる。通知を使うなら、置き場が変わって VAPID の鍵が新しくなるので、iPhone で通知を登録し直す。
7. **Linux のテスト利用者を tailnet に招く前に（ACL）**:
   - Tailscale の既定の ACL は「全員が全部の機械に届く」なので、そのまま招くと、テスト利用者が Mac の `:4444`（agent-link の Web）と `:4445` に届く。招く前に ACL を変える。
   - Linux の機械に tag（例 `tag:sai-server`）を付け、テスト利用者を group にまとめ、group には Linux の 443 番と 8443 番だけを許す。例:
     ```json
     {
       "groups":    { "group:testers": ["tester1@example.com"] },
       "tagOwners": { "tag:sai-server": ["autogroup:admin"] },
       "acls": [
         { "action": "accept", "src": ["autogroup:admin"], "dst": ["*:*"] },
         { "action": "accept", "src": ["group:testers"],  "dst": ["tag:sai-server:443,8443"] }
       ]
     }
     ```
   - 確かめ方: テスト利用者の端末から Mac の `:4444` に届かず、Linux の `https://<Linux の名前>.<tailnet>.ts.net/` に届く。
   - （要確認）tag の付け方・利用者の上限は Tailscale の今の文書で確かめる。
8. **戻し方**: agent-link の daemon と Web を止める。PATH を sai-moon に戻す。sai-tree の `.claude/settings.json.bak` を `.claude/settings.json` に戻す。sai-moon の daemon と Web を前と同じ命令で起こす。古い `.sai/` と `~/.sai` は手を付けていないので、そのまま使える。

### 7. 残りを探す
最後に次を実行し、許した行（C2 の「残してよい古い名前」）以外が出ないようにする。
```bash
git grep -nIE 'sai-link|sai-web|SAI_|\.sai\b|sai-engine|sai_web|sai-paper|sai-sci|for-sai-developers' -- . ':!docs/plan' ':!docs/switch-over.md'
```

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
- 書いてよい: `AGENTS.md`・`README.md`・`ARCHITECTURE.md`・`CONCEPT.md`・`docs/**`（`docs/plan/**` を除く）・`.github/workflows/ci.yml`・`.gitignore`。コードの注記の中の文書の名前（`for-sai-developers.md` → `for-developers.md`）だけは、`crates/**`・`apps/web/**` でも直してよい
- 読むだけ: `crates/**`・`apps/web/**`（文書に書く事実の確認）
- 触らない: `docs/plan/**`・`CLAUDE.md`・コードの動き

## 完了条件
- [ ] AC1: `docs/switch-over.md` に変更内容 6 の 1〜8 があり、命令はそのまま貼れる形になっている。
- [ ] AC2: `AGENTS.md` のテストの命令をそのまま実行すると全部通り、書いた本数と実際の本数が合う。
- [ ] AC3: `README.md` の入れ方を一時フォルダで試すと動く。`AGENT_LINK_HOME` と `AGENT_LINK_PROJECT_ROOT` を一時フォルダにして、`agent-link init` から `agent-link-web serve --port 8297` まで行う。
- [ ] AC4: 変更内容 7 の grep の結果が、許した行だけになる。
- [ ] AC5: `ci.yml` が YAML として読める（`ruby -ryaml -e 'p YAML.load_file(".github/workflows/ci.yml")["jobs"].keys'` が `["rust", "web"]`）。
- [ ] AC6: `git check-ignore -q .sai && git check-ignore -q .agent-link` が成功する。

## 検証
```bash
cd ~/workdir/Sai/agent-link   # worktree の中ならその場所
git grep -nIE 'sai-link|sai-web|SAI_|\.sai\b|sai-engine|sai_web|sai-paper|sai-sci|for-sai-developers' -- . ':!docs/plan' ':!docs/switch-over.md'
ruby -ryaml -e 'p YAML.load_file(".github/workflows/ci.yml")["jobs"].keys'
git check-ignore -q .sai && git check-ignore -q .agent-link && echo ignored
```
AGENTS.md のテストの命令も上から順に実行する。

## 決定事項と理由
- 切り替えは手順書だけにし、agent は実行しない — 理由: sai-moon を止める・PATH を変える・Tailscale の ACL を変えるのは、利用者の環境を変える操作である（要件定義書の範囲外）。
- 手順書は ACL の例と「招く前に変える」を目立たせる — 理由: 今の Mac の 8200 番の Web は合言葉が無いまま tailnet に出ていて、既定の ACL のまま招くと、テスト利用者が Mac でエージェントを起動できてしまう。
- `docs/for-sai-developers.md` を `docs/for-developers.md` にする — 理由: 名前に sai が入っている。

## 要確認
- Tailscale の tag の付け方と利用者の上限（手順書に「要確認」と書く）。
