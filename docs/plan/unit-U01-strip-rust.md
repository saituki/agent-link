# 実装計画書 U01: Rust から sai-paper・sai-sci・研究向けの部分・Customer の block を消す

## 目的
agent-link の Rust を sai-link の 3 crate だけにする。sai-link の中にある研究向けの機能（研究の対象 Node・番号の台帳・範囲・ジョブの待ち・層）と、Customer サポートの block を消す。`cargo test --workspace` が通る状態にする。

## 背景
- agent-link は sai-tree を複製した、アプリ開発専用の道具である。論文・research-tree・Customer は使わない（要件定義書 `docs/plan/requirements.md`）。
- sai-link は sai-paper・sai-sci に Rust の依存を持たない。`crates/sai-link-*/Cargo.toml` に `sai-paper-*`・`sai-sci-*` は無く、`use sai_paper`・`use sai_sci` も無い。なので crate を消しても、sai-link の build は壊れない。壊れるのは、研究向けの機能を使うコードとテストだけである。
- 名前（`sai-link` → `agent-link` など）はこの作業では変えない。後の U05 が変える。

## 前提（着手時点のコードの状態）
- リポジトリは `~/workdir/Sai/agent-link`（sai-tree の main を clone した物）。行番号は複製の元（sai-tree の 4736175）の値。ずれていたら、関数名で探す。
- ルートの `Cargo.toml` の `members` は 7 つ（`crates/sai-link-engine`・`sai-link-cli`・`sai-link-view`・`sai-paper-core`・`sai-sci-core`・`sai-sci-cli`・`sai-paper-cli`）。`[workspace.dependencies]` に `sai-paper-core`・`sai-sci-core` の path がある。
- 同じガイドの U02（Web の Python）・U03（Web の画面）・U04（ほかのアプリ・CI）と並列で動く。書くファイルは重ならない。

## 変更内容

### 1. crate とフォルダを消す
- `git rm -r crates/sai-paper-core crates/sai-paper-cli crates/sai-sci-core crates/sai-sci-cli sai-sci`
- ルートの `Cargo.toml`: `members` を `crates/sai-link-engine`・`crates/sai-link-cli`・`crates/sai-link-view` の 3 つにする。`[workspace.dependencies]` の `sai-paper-core`・`sai-sci-core` の行を消す。`exclude = ["waggle"]` は残す。
- `Cargo.lock` は `cargo build` が書き直した物を commit する。

### 2. 研究の対象 Node（`target_node`・`SAI_TARGET_NODE`）を消す
- `crates/sai-link-engine/src/target_node.rs` を丸ごと消し、`lib.rs:62` の `pub mod target_node;` を消す。
- `orchestrator.rs`: `PipelineConfig.target_node`（`:114`）、`:155`、`:832-840`（`SAI_TARGET_NODE=` を前に付ける所）、`:2061`（`target_node::resolve` の呼び出し）、テストの `:4451`・`:4509-4519` を消す。
- `kanban.rs:56-58`: `META_FM_KEYS` から `"target_node"` を外し、doc の行も消す。配列の長さを直す。
- `settings.rs:65-72`: 予約キーから `SAI_TARGET_NODE` を外す。
- `crates/sai-link-cli/src/cli.rs:1382` と `crates/sai-link-engine/tests/orchestrator_test.rs:79,213,269` の `target_node: None` を消す。
- 変更後: 子 task の task.md の frontmatter に `target_node:` が出ない。子の起動の命令に `SAI_TARGET_NODE=` が付かない。

### 3. 番号の台帳（`seen_ids`）を消す
- `crates/sai-link-engine/src/seen_ids.rs` を丸ごと消し、`lib.rs:84` を消す。
- 呼び出し元: `orchestrator.rs:689`（最初の文を記録）、`crates/sai-link-cli/src/cmd_doc.rs:46-49`（doc write で記録）、`session.rs:2945-2952`（`ids_in`）、`crates/sai-link-view/src/block_history.rs:256`（`read_all`）、テスト `orchestrator.rs:4872` を消す。
- `block_history.rs` の `cli_summary(events, ledger)`（`:275-290`）から `ledger` の引数を外す。出力の JSON から台帳の数の項目も外す。Web の画面がこの項目を読んでいたら、U03 の担当に任せず、ここで消してよい項目かを `apps/web` を grep して確かめる。読む所があれば、項目を消さずに常に 0 を返す。

### 4. 範囲（`context: true`・`# context`）を消す
- `crates/sai-link-engine/src/pre_run.rs` の `context_text`・`run_capture` と、そのテスト（`:623-703`、`sci-impl` を使う物）を消す。
- `block_config.rs:209-213` の `context` の項目を消す。呼び出し元の `orchestrator.rs:675-680`、`cli.rs:1399` を消す。
- `placeholder.rs:151・206・889` の「範囲（research-tree から切り出した文脈）」の説明を消す。`{{context}}` の置き換えが残っていたら、それも消す。
- `pre_run.rs` の `run_if_configured`・`run_with`（`pre_run` の汎用の仕組み）と `run_with_stdin`（`agent_api.rs:413` の `write_check` が使う）は**残す**。冒頭の doc の研究の例（`sai-paper brief --write`、`:6-7`）だけ消す。

### 5. ジョブの待ち（`job_status`・`after_job`・`card wait`）を消す
- `pre_run.rs` の `job_status_template`・`job_status_template_at`・`JOB_STATUS_MISSING`（`:368` 付近）を消す。
- `daemon.rs:185-215`（`job_status_command`・`read_job_state`、カードを起こす処理）と、そのテスト `:1558-1630` を消す。
- `kanban.rs:54-58` の `after_job` を `META_FM_KEYS` から外す。ただし、古いカードに `after_job:` が残っていても読めるようにする。同じファイルの「廃止したキー」の一覧に `after_job`・`target_node` を足す。
- `crates/sai-link-cli/src/phone.rs`: `todo_after_job`・`after_job_of`（`:682-694`）、一覧の `after_job`（`:792-794`）、`card_wait`（`:1082-1114`）、`:1282`、テスト `:2343-2353` を消す。`cli.rs` の `PhoneCardCmd::Wait` と、それを呼ぶ `phone.rs:236` を消す。
- 変更後: `sai-link phone card wait` は無くなる（clap の誤りで終了コード 2）。`sai-link run wait`（別の命令）は残す。

### 6. block の `layer`（research-tree の層）を消す
- `block_config.rs:192-194` の `layer`、`validation.rs:38-42`・`:151-160`・テスト `:524-540` を消す。
- DB: `db/schema.rs:256-257` の `block_metadata.layer` の列を足す移行を消す。`db/models.rs:86-87・132`、`db/store.rs` の `layer` の読み書き（`:460・478・509・530・548・569・589・611・639・658`）を消す。agent-link は新しい DB から始めるので、古い DB の列は気にしなくてよい（要件定義書の決定）。
- `doc_write_notify.rs:426`、`tests/orchestrator_test.rs:59,251`、`tests/validation_tests.rs:27` の `layer` を消す。`tests/layer_tests.rs` を丸ごと消す。
- `crates/sai-link-view/src/workflow/` の `layered_layout`（図の配置）は別物なので**残す**。

### 7. 命令名と cc の許可から `sai-paper`・`sai-sci` を外す
- `session.rs:2720` の `SAI_CLI_NAMES` を `["sai-link", "sai-web"]` にする。`session.rs:2913` の `is_sai_cli` も同じにする。テスト `session.rs:3801-3935` の `sai-sci op`・`sai-paper content read` の例は、`sai-link doc read` と `sai-link task create` の例に置き換える。
- `block_history.rs:384-472` のテストも同じように置き換える。
- `cc_command.rs:363-366`: `Bash(sai-paper *)`・`Bash(sai-sci *)` を消す。テスト `:803-806` を合わせる。
- `orchestrator.rs:823-880` の PATH の前置きと注記から `sai-paper`・`sai-sci` を消す。`SAI_LINK_BIN` は残す（子が sai-link を呼ぶため）。注記の「sai-sci が使う」は消す。
- `orchestrator.rs:2638`・`:2747` の注記の「sci card」「research-tree」を消す。
- `orchestrator.rs:5968-6018` のテストを読み、research-tree の範囲（`sci_node`）を前提にしていたら消す。前提にしていなければ、`sci_node` を汎用の meta の名前（例 `note`）に変える。
- `crates/sai-link-cli/tests/cli_test.rs:519-535` の `sci_node: Q12` を `note: n1` にする。
- `settings.rs:44-49`・`:527`・`:576-625` と `orchestrator.rs:4587-4650` の MLflow の例（`MLFLOW_TRACKING_URI`）は、汎用の例 `EXAMPLE_URL` に変える。
- `task_manager.rs:2494・2537` の `sai-code-research` は `sai-code` にする。
- `phone.rs:2035-2039` の検索の例 `mlflow 実験` は `daemon 再起動` にする。
- `tests/assets_cli_only.rs:20-25` の禁止語の一覧は、そのまま残してよい。

### 8. Customer サポートの block を消す
- `git rm crates/sai-link-engine/assets/blocks/user-opinion.md user-question.md user-report.md`
- `assets.rs:28-30` の 3 行を消す。block の数を数えるテストがあれば、13 → 10 に直す。
- `crates/sai-link-view/src/left_tree.rs:32-34` の `CUSTOMER_BLOCKS`・`CUSTOMER_FOLDER_ID`、`:129-140`・`:197-199` の Customer フォルダの処理、テスト `:664-716` を消す。

### 9. 起動時の文から研究の説明を消す
- `crates/sai-link-engine/assets/template/sai-base.md:13-14`: 道具の表の `sai-paper`・`sai-sci` の行を消す。`sai-web` の行が論文・研究の命令を説明していたら、その部分も消す。
- `assets/blocks/explore.md:29`: `crates/sai-paper-cli/src/` を入口にする指示を消す。
- `assets/blocks/bugfix.md:89`: 旧 sai-phone・sai-radio の注意を消す。
- 消したあと、`grep -rnE "sai-paper|sai-sci|research-tree|論文|Radio|MLflow" crates/` で残りを探し、研究・Customer の説明なら消す。コードの意味に関わる所が残っていたら、完了報告の「問題点」に書く。

### 10. 使われていないファイル
- `crates/sai-link-engine/src/sai-engine/block02_rust-best-practices.md` はどこからも読まれていない。このファイルは消さずに残し、完了報告に「残した」と書く。

## 共有契約（他の作業と共有する取り決め。worker は変えない）
- C1（ガイド 1 の約束）: 残す CLI（`sai-link`）の JSON の項目は、次の物だけを消してよい。`after_job`・`target_node`・`layer`・台帳の数・Customer フォルダ（id `-1004`）。ほかの項目の名前と形は変えない。Web（U02・U03）はこれらの項目を読まない前提で直す。
- C1: この時点では名前を変えない（`sai-link`・`SAI_*`・`.sai/` のまま）。

## ファイルの所有権
- 書いてよい: `Cargo.toml`・`Cargo.lock`・`crates/**`、消すフォルダ `sai-sci/`
- 読むだけ: `apps/web/**`（項目を読む所の確認だけ）
- 触らない: `apps/**`・`docs/**`・`.github/**`・`AGENTS.md`・`README.md`・`ARCHITECTURE.md`・`CONCEPT.md`

## 完了条件
- [ ] AC1: `ls crates` が `sai-link-cli sai-link-engine sai-link-view` だけになる。`sai-sci/` が無い。
- [ ] AC2: `cargo build --workspace` と `cargo test --workspace`（下の検証の命令）が通る。
- [ ] AC3: `grep -rnE "target_node|seen_ids|SAI_TARGET_NODE|after_job|job_status|CUSTOMER_BLOCKS|user-report" crates/` の結果が、廃止キーの一覧の行（`kanban.rs`）だけになる。
- [ ] AC4: `grep -rnE "sai-paper|sai-sci" crates/` の結果が、禁止語のテスト（`assets_cli_only.rs`）だけになる。
- [ ] AC5: 一時フォルダで `target/debug/sai-link init` を実行すると、`.sai/blocks/` に `user-*.md` ができず、`.claude/settings.json` の `permissions.allow` が `Bash(sai-link *)`・`Bash(sai-web *)` の 2 つになる。

## 検証
本物のプロジェクトで `init` しない。環境変数は必ず外す（道具の sai-moon が `SAI_*` を入れている）。

```bash
cd ~/workdir/Sai/agent-link   # worktree の中ならその場所
env -u SAI_PROJECT_ROOT -u SAI_PARENT_RUN_ID -u SAI_BLOCK_NAME -u SAI_TARGET_NODE \
  perl -e 'alarm 1800; exec @ARGV' cargo test --workspace

T=$(mktemp -d); H=$(mktemp -d)
(cd "$T" && git init -q && env -u SAI_PROJECT_ROOT -u SAI_PARENT_RUN_ID SAI_HOME="$H" \
  "$OLDPWD/target/debug/sai-link" init)
ls "$T/.sai/blocks"; python3 -c "import json;print(json.load(open('$T/.claude/settings.json'))['permissions']['allow'])"
rm -rf "$T" "$H"
```
期待: テストがすべて通る。block の一覧に `user-*` が無い。allow が `['Bash(sai-link *)', 'Bash(sai-web *)']` になる。

## 決定事項と理由
- 研究向けの機能は、残して隠すのではなく消す — 理由: agent-link では使わない。残すと、改名（U05）の対象と読む量が増えるだけ。/ 採らなかった案: 設定で切る。
- `pre_run`（汎用）と `write_check` は残す — 理由: 研究専用ではない。
- 古い DB の `layer` の列は放っておく — 理由: agent-link は新しい DB から始める（要件定義書）。

## 要確認
- `block_history.rs` の台帳の数を Web が読むか。読むなら、項目を残して 0 を返す（変更内容 3）。
- `block02_rust-best-practices.md` の扱いは利用者に聞く。それまでは残す。
