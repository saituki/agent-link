# 実装計画書 U05: Rust（sai-link）の名前を agent-link に変える

## 目的
Rust の crate・実行ファイル・環境変数・フォルダ・DB・起動時の文に出る名前を、共有契約 C2 の対応表どおりに agent-link に変える。`agent-link init` で `.agent-link/` ができ、`cargo test --workspace` が通る状態にする。

## 背景
- agent-link は sai-tree を複製した開発用の道具である。開発するアプリ（sai-tree）と同じ名前（`sai-link`・`SAI_PROJECT_ROOT`・`.sai/`）を使っているので、道具の環境変数がアプリのテストに混ざる事故が起きた（要件定義書 `docs/plan/requirements.md`）。名前を全部分ければ、混ざらなくなる。
- 古いデータ（`.sai/`・`~/.sai`）は移さない。新しい名前の場所から始めるので、古い名前を読む互換の処理は作らない。
- 作業中のこのリポジトリは sai-moon のプロジェクトで、sai-moon が `SAI_PROJECT_ROOT` などを入れて worker を動かす。名前を変えたあとのコードは `AGENT_LINK_*` しか読まないので、テストに `env -u SAI_…` は要らなくなる。

## 前提（着手時点のコードの状態）
- ガイド 1（U01〜U04）が終わっている。Rust は `crates/sai-link-engine`・`sai-link-cli`・`sai-link-view` の 3 crate だけで、研究向けの機能（`target_node`・`seen_ids`・`context`・`job_status`・`layer`）と Customer の block は消してある。`cargo test --workspace` は通る。
- 名前を決めている主な所（行番号は複製の元の値。ずれていたら関数名で探す）:
  - `crates/sai-link-engine/src/project.rs:101` `sai_path(rel)`（プロジェクトの `.sai/` の下のパスを作る）、`:116`・`:124-126`・`:131`（DB のパスの候補）、`:146` `sai_home()`、`:163` `resolve_sai_home`（`SAI_HOME` → `$HOME/.sai`）。
  - `crates/sai-link-engine/src/init.rs:61-74` `INIT_DIRS`（`sai-engine` を含む）。
  - `crates/sai-link-engine/src/waggle_integration.rs:296`・`:313`（sharer `"sai-engine"`）、`:376`・`:534`（`.sai/sai-engine/waggle-blobs`）。
  - `crates/sai-link-engine/src/cc_command.rs:348`（`sai_link_exe()` の既定 `sai-link`）、`:363-364`（許可）、`:393-419`（フック）、`:448`（`SAI_WEB_HOOKS_DIR`）、`:471-476`（`SAI_FINALIZE_COMMAND_MD`）。
  - `crates/sai-link-engine/src/assets.rs`（`BASE_BLOCKS`・`BASE_TEMPLATES` の `include_str!`）。
  - `crates/sai-link-cli/src/cli.rs:50`（`#[command(name = "sai-link")]`）、`crates/sai-link-cli/Cargo.toml` の `[[bin]] name = "sai-link"`。
  - `crates/sai-link-engine/src/session.rs:2720` `SAI_CLI_NAMES`、`:2913` `is_sai_cli`。
  - DB のパスを直に書いている所: `cli.rs:1185`・`:2001`・`:2152`・`:2276-2277`、`search.rs:92`・`:222`、`left_tree.rs:587`。
  - `phone_export.rs:627`（書き出しの置き場 `<home>/web/runs`）。
- 環境変数（ガイド 1 のあとに残る物）: `SAI_PROJECT_ROOT`・`SAI_PARENT_RUN_ID`・`SAI_BLOCK_NAME`・`SAI_IS_SUBTASK`・`SAI_LINK_BIN`・`SAI_HOME`・`SAI_WRITE_FILENAME`・`SAI_WRITE_DESCRIPTION`・`SAI_CLAUDE_BIN`・`SAI_WEB_HOOKS_DIR`・`SAI_AGENT_TRANSPORT`・`SAI_IDLE_TIMEOUT_MS`・`SAI_LONG_RUN_ALERT_MS`、それとテスト専用の `SAI_ENV_TEST_LOCK`・`SAI_TEST_*` など。`cli.rs:2298` の `SAI_MAKASERU` は、古い名前を環境から外すだけの行なので消す。
- 同じガイドの U06（Web）と並列で動く。U06 は `apps/web/**` だけを書く。

## 変更内容

### 1. crate とフォルダの名前
- `git mv crates/sai-link-engine crates/agent-link-engine`（`sai-link-cli`・`sai-link-view` も同じ）。
- 各 `Cargo.toml` の `name`、path の依存、`[[bin]] name = "agent-link"`。ルートの `Cargo.toml` の `members`。
- Rust のコードの `sai_link_engine::`・`sai_link_view::` を `agent_link_engine::`・`agent_link_view::` に変える（`use` と `extern` を含む）。
- `cli.rs:50` を `#[command(name = "agent-link")]` にする。`schema --json` の `bin` は `agent-link` になる。

### 2. フォルダ・DB・置き場
- `project.rs`: プロジェクトのフォルダの名前を 1 つの定数にまとめる（`pub const PROJECT_DIR: &str = ".agent-link";`）。`sai_path()` と DB のパスの候補（`:116`・`:124-126`・`:131`）はこの定数を使う。DB は `.agent-link/engine/agent-link.db` にする。`:124-126` の古い場所の候補（`../.sai/…` など）は、同じ形のまま `.agent-link` に変える。
- 置き場: `resolve_sai_home` は `AGENT_LINK_HOME` → `$HOME/.agent-link` にする。
- DB のパスを直に書いている所（前提の一覧）は、`project.rs` の関数か定数を使う形に直す。直せない所（テストの中など）は文字列を `.agent-link`・`engine`・`agent-link.db` にする。
- `init.rs:61-74` の `INIT_DIRS` の `sai-engine` を `engine` にする。
- `waggle_integration.rs` の sharer を `"agent-link"`、blobs の置き場を `.agent-link/engine/waggle-blobs` にする（`project.rs` の関数を使う）。
- `cli.rs:2276-2277` の daemon の PID・ログのパスも同じにする。
- 変更後の例:
  ```
  agent-link init        →  .agent-link/{blocks,template,outputs/{todo,running,review,done},engine/notifications/{pending,processed,results}}
  agent-link doc write …  →  "ref": "waggle:AbCdEfGh:path:.agent-link/outputs/*/12/report.md"
  ```

### 3. 環境変数と `SAI_` の定数
- `\bSAI_` で始まる名前を全部 `AGENT_LINK_` に変える。例外は `SAI_LINK_BIN` → `AGENT_LINK_BIN` だけ。
- 子に渡す所も同じに変える: `orchestrator.rs:821-880`（`AGENT_LINK_IS_SUBTASK=1`・`AGENT_LINK_BIN` など）、`tmux.rs:58`・`:73`、`pre_run.rs:111`・`:163`、`agent_api.rs:407-410`、`cli.rs:2295`。
- `settings.rs:65-72` の予約キーも新しい名前にする。
- `cli.rs:2298` の `SAI_MAKASERU` の行を消す。

### 4. 命令名と cc の設定
- `cc_command.rs`: `sai_link_exe()` の既定を `agent-link` にする。許可を `Bash(agent-link *)`・`Bash(agent-link-web *)` にする。finalize の命令ファイルを `.claude/commands/agent-link-finalize.md` にし、中身の `sai-link run finalize` を `agent-link run finalize` にする（`init.rs:195` も）。`AGENT_LINK_WEB_HOOKS_DIR` を読む。
- `session.rs:2720` を `AGENT_LINK_CLI_NAMES = ["agent-link", "agent-link-web"]`、`:2913` の `is_sai_cli` も同じにする。命令名の読み取りのテストの例も `agent-link …` にする。
- 案内・hint・エラーの文（`agent_api.rs`・`orchestrator.rs`・`daemon.rs`・`cli.rs`・`settings.rs` など）の `sai-link` を `agent-link`、`sai-web` を `agent-link-web`、エンジンの名前としての `sai-engine` を `agent-link` に変える（例: `タスク 217 の操作（finalize）を sai-engine に要求しました。` → `… agent-link に要求しました。`）。

### 5. 起動時の文（`crates/agent-link-engine/assets/`）
- template のファイルの名前を C2 のとおりに変える（`git mv`）。`assets.rs` の `include_str!` と名前、block の `templates:` の値も変える。
- template と block の本文の `sai-link` → `agent-link`、`sai-web` → `agent-link-web`、`SAI_` → `AGENT_LINK_`、`.sai/` → `.agent-link/`、`sai-finalize` → `agent-link-finalize` に変える。`waggle:<トークン>:path:.sai/outputs/…` の参照の形も変える。
- 本文の中で「sai」というアプリの名前を指す言葉（例「sai-link 上で動くエージェント」「sai の機能」）は agent-link に変える。`sai-tree`（開発するアプリの名前）は変えない。

### 6. テスト
- テストの中の古い名前も同じ規則で変える。
- `tests/assets_cli_only.rs` の禁止語の一覧（`sai-moon-link` など）は変えない。
- 環境変数を立てるテスト（`AGENT_LINK_ENV_TEST_LOCK` など）は名前を変えるだけで、中身は変えない。

### 7. 残りを探す
変えたあとに次を実行し、出た行をすべて直す。直さない行は、理由を完了報告に書く。
```bash
git grep -nIE 'sai-link|sai-web|SAI_|\.sai\b|sai-engine|sai_web|sai-paper|sai-sci' -- crates Cargo.toml
```
中だけで使う名前（`sai_home()`・`sai_path()` などの関数名）はこの grep に当たらないので、変えなくてよい。

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
- 書いてよい: `crates/**`・`Cargo.toml`・`Cargo.lock`
- 読むだけ: `apps/web/**`（Web が読む JSON の項目と環境変数の確認）
- 触らない: `apps/**`・`docs/**`・`.github/**`・`AGENTS.md`・`README.md`・`ARCHITECTURE.md`・`CONCEPT.md`・`CLAUDE.md`・`.gitignore`

## 完了条件
- [ ] AC1: `ls crates` が `agent-link-cli agent-link-engine agent-link-view` になる。`cargo build --workspace` で `target/debug/agent-link` ができる。
- [ ] AC2: `cargo test --workspace` が、`env -u` を付けずに通る（sai-moon の `SAI_*` が入っていても通る）。
- [ ] AC3: 変更内容 7 の grep の結果が空になる。
- [ ] AC4: 一時フォルダで `agent-link init` を実行すると、`.agent-link/engine/` ができ、`.sai/` はできない。`.claude/settings.json` の allow が `Bash(agent-link *)`・`Bash(agent-link-web *)`、フックの命令が `agent-link` の実行ファイルを指す。`.claude/commands/agent-link-finalize.md` ができる。
- [ ] AC5: 同じ一時フォルダで `agent-link card create` → `agent-link run tree --json` が動き、`.agent-link/engine/agent-link.db` ができる。`agent-link schema --json` の `bin` が `agent-link`。

## 検証
本物のプロジェクトでは `init` しない。置き場も一時フォルダにする。

```bash
cd ~/workdir/Sai/agent-link   # worktree の中ならその場所
perl -e 'alarm 1800; exec @ARGV' cargo test --workspace
git grep -nIE 'sai-link|sai-web|SAI_|\.sai\b|sai-engine|sai_web|sai-paper|sai-sci' -- crates Cargo.toml || echo CLEAN

B="$PWD/target/debug/agent-link"; T=$(mktemp -d); H=$(mktemp -d)
cd "$T" && git init -q
AGENT_LINK_HOME="$H" AGENT_LINK_PROJECT_ROOT="$T" "$B" init
ls -a "$T" "$T/.agent-link" "$T/.agent-link/engine"
python3 -c "import json;d=json.load(open('.claude/settings.json'));print(d['permissions']['allow'])"
ls .claude/commands
echo '{"block":"sub","task_text":"t"}' | AGENT_LINK_HOME="$H" AGENT_LINK_PROJECT_ROOT="$T" "$B" card create --json
AGENT_LINK_HOME="$H" AGENT_LINK_PROJECT_ROOT="$T" "$B" schema --json | python3 -c "import json,sys;print(json.load(sys.stdin)['bin'])"
ls "$T/.agent-link/engine"
cd - >/dev/null; rm -rf "$T" "$H"
```
期待: テストが通る。`CLEAN`。`.sai` が無く `.agent-link` がある。allow が `['Bash(agent-link *)', 'Bash(agent-link-web *)']`。`agent-link-finalize.md` がある。カードができる。`agent-link`。`agent-link.db` がある。

## 決定事項と理由
- 中だけで使う関数名（`sai_home()` など）は変えない — 理由: 外から見えず、変えると差分が増えるだけ（要件定義書の決定）。
- 古い名前を読む互換の処理を作らない — 理由: 新しく始める（要件定義書の決定）。
- プロジェクトのフォルダの名前を定数 1 つにまとめる — 理由: 今は `.sai` が数十か所に散らばっている。次に変えるときに 1 か所で済む。

## 要確認
- なし
