# 実装計画書 U01: エージェントを herdr 無しで動かす headless の輸送路

## 目的
sai-tree の `sai-link daemon` が、cc・pi を herdr の pane ではなく子プロセス（`claude -p` の stream-json・`pi --mode json`）で動かすようにする。herdr は呼ばない。カードの実行・子タスク（create / execute / wait / send / interrupt / capture / finalize）は、今と同じ命令のまま動く。子タスクも pane を作らず、裏のプロセスとして動く。

## 背景
- sai-tree は sai-moon（`~/workdir/Sai/sai-moon`）を複製した、Linux の多人数サーバ版である。エージェントはコンテナの中で動き、herdr も端末の画面も無い。利用者は端末を見ず、Web の Chats から会話する。
- 今はエージェントを herdr の pane に起動し、文字とキーを送って操作している（`crates/sai-link-engine/src/orchestrator.rs:2909` `execute_pipeline_background`）。runner の抽象は `RunnerKind` の enum だけで、pane の抽象は無い（`crates/sai-link-engine/src/runner.rs:20`、`crates/sai-link-engine/src/herdr.rs:424` `HerdrClient`）。
- 完了の判定（`sai-link task report` → `finalize_request`、`sai-link doc write` の検出）は herdr に依存していない（`orchestrator.rs:3281` `execute_completion_with_doc_write` の `CompletionEvent` の分岐）。herdr に依存しているのは「pane を作る・コマンドを入れる・文字とキーを送る・状態（`agent_status`）を読む・画面を読む・閉じる」だけである。
- そこで **pane の代わりになる「擬似 pane」** を作り、`HerdrClient` の操作をすべて擬似 pane へ向ける。呼び出し側（orchestrator・task_manager・agent_api。herdr を呼ぶ場所は 30 か所以上。調査 X1）は、pane を作る 2 か所を除いて変えない。
- 調査 X1（起動経路と herdr 依存の全一覧）は `docs/plan/research/X1-agent-lifecycle.md` にある。

## 前提（着手時点のコードの状態）
- sai-tree は sai-moon の commit `0c4903d` の複製である。行番号はその時点の物。ずれていたら関数名で探すこと。
- pane を作る場所は 2 か所: `orchestrator.rs:2935-2980`（`execute_pipeline_background` の Step 6）と `task_manager.rs:785` `resolve_pane_placement_inner`（子タスク。タブ作成・レイアウト取得・分割をここで行い、`orchestrator.rs:2942` が `pane_id` Some で受ける）。
- 起動コマンドは `orchestrator.rs:3008-3020` で `cd '<cwd>' && <pi_command> ; <exit_command>` の形に組まれ、`herdr.run_in_pane`（`herdr.rs:703`）で入れる。`<pi_command>` の頭には `settings.json` の `env`・`SAI_PROJECT_ROOT` などが `KEY='値' ` の形で付く（`orchestrator.rs:827-901`）。
- 最初の依頼文は `submit_prompt_verified`（`orchestrator.rs:3956`）が `send_text` を 1 回、確定キー（`send_keys`）を最大 3 回送り、`get_pane_status` が `working` になるのを待つ。
- 子への追加の依頼は `send_turn_to_pane`（`orchestrator.rs:3730`）と `agent_api.rs:1610` 付近（`task send` の `SendRoute::DirectToPane`）が `submit_prompt_verified` を使う。中断は `agent_api.rs:1565` が `send_keys(pane, "esc")`、画面は `agent_api.rs:1764` `task_capture` が `read_pane_output`、閉じるのは `orchestrator.rs:1806` `close_execution_pane` が `close_pane`。
- cc のコマンドは `cc_command.rs:168` `CcCommandBuilder::build`（`format!("cc {}", …)`）、pi のコマンドは `pi_command.rs:487` `PiCommandBuilder::build`（`format!("pi {}", …)`）が作る。cc は既定で `--dangerously-skip-permissions`（`cc_command.rs:188-195`・`:231-236` `cc_skip_permissions`）。
- `orchestrator.rs:905-927` が「エージェントが終わったら `sai-link run finalize <id>` を走らせる」`exit_command` を作る。
- cc の費用は、今は statusLine のフック（`sai-link cc-statusline`）が `~/.sai/cc-cost/<session_id>.json` に書き、`phone_export.rs:371` `read_cc_cost` が読む。headless では statusLine が呼ばれない（調査 E3 §4）。

## 変更内容

### 1. 新しいモジュール `crates/sai-link-engine/src/headless.rs`
`lib.rs` に `pub mod headless;` を足す。中身は次のとおり。

- `pub fn is_headless_pane(id: &str) -> bool` — `id.starts_with("hl-")`。
- `pub fn pane_dir(id: &str) -> PathBuf` — `crate::project::sai_path(".sai/sai-engine/headless").join(id)`。
- `pub fn new_pane(cwd: Option<&str>) -> Result<PaneInfo, HerdrError>` — id を `hl-` + 12 桁の 16 進（`cc_command::generate_uuid_v4` の先頭 12 文字でよい）で作り、`pane_dir` を作って `cwd.txt` に cwd を書き、`state.json` を `idle` で書いて `PaneInfo`（`workspace_id`・`tab_id` は `"headless"`、`agent_status` は `"idle"`）を返す。
- 擬似 pane の操作（`HerdrClient` から呼ぶ。下の §2）:

| 関数 | すること |
|---|---|
| `run(id, command)` | `command.txt` に起動コマンドを書き、**監督スレッド**（下）を `std::thread::spawn` で起こす |
| `send_text(id, text)` | `draft.txt` に上書きで書く |
| `send_keys(id, keys)` | `keys` が `esc` / `Escape` / `ctrl+c` なら `interrupt` という空のファイルを作る。それ以外（`Alt+Enter`・`Enter` など確定キー）は、`draft.txt` があれば `inbox/<unix ミリ秒>-<連番>.txt` へ rename する。無ければ何もしない（`submit_prompt_verified` の再送で同じ文が 2 回入らないため） |
| `status(id)` | `state.json` を読み `PaneInfo` を返す。`agent_status` は `state` の値（`working` / `idle`）。`stopped` と、フォルダが無いときは `Err(HerdrError::…)`（pane が消えた扱い。`orchestrator.rs:3530` 付近の連続エラー判定にそのまま乗る） |
| `read_output(id, lines)` | `stream.jsonl` の末尾 `lines` 行を、1 行 300 文字までに切って返す |
| `close(id)` | `stop` という空のファイルを作る |

- **監督スレッド**（1 つの擬似 pane に 1 本。200ms ごとに回る）:
  1. `stop` があれば、動いているターンのプロセスを止め（下の「止め方」）、`state` を `stopped` にして終わる。
  2. `interrupt` があれば消し、動いているターンのプロセスを止める。
  3. ターンのプロセスが動いていなければ、`inbox/` の一番古いファイルを取り出して**ターンを始める**。
  4. ターンのプロセスの stdout を 1 行ずつ `stream.jsonl` に追記し、生成中の文（`partial`）を更新する。stderr は `stderr.log` に追記する。
  5. プロセスが終わったらターンを閉じる。終了コードが 0 でなければ `state.json` の `error` に `stderr.log` の最後の空でない行を入れる。
- **ターンの始め方**: `sh -c "<コマンド>"` を `std::process::Command` で起こす。stdin にはそのメッセージの本文を書いて閉じる。stdout・stderr はパイプにする。`process_group(0)` を付ける（`std::os::unix::process::CommandExt`）。環境変数 `HERDR_PANE_ID=<擬似 pane の id>` と `SAI_AGENT_TRANSPORT=headless` を足す（質問のフックと拡張が pane を名乗れるように。U02 が使う）。`stream.jsonl` には、ターンの前に次の 1 行を書く。
  ```json
  {"type":"sai_turn","turn":1,"ts":"2026-10-07T12:00:00+09:00","text":"<メッセージ>"}
  ```
- **2 回目以降のターンのコマンド**: `command.txt` の文を次のように書き換える（関数 `resume_command(cmd: &str, runner: RunnerKind) -> String` にし、単体テストを付ける）。
  - cc: ` --session-id ` を ` --resume ` に置き換える（同じ session id で続きになる）。
  - pi: 末尾に ` --continue` を足す（`--session-dir` の中の直近のセッションを続ける）。
- **止め方**: プロセスグループに SIGTERM（`libc` が無ければ `kill -TERM -<pgid>` を `Command` で呼ぶ）。2 秒たっても終わらなければ SIGKILL。
- **生成中の文（partial）** — stdout の 1 行ごとに JSON を読み、次の規則で更新する（関数 `update_partial(partial: &mut String, line: &serde_json::Value, runner)` にし、単体テストを付ける）。
  - cc: `type == "stream_event"` かつ `event.delta.type == "text_delta"` なら `event.delta.text` を足す。`type == "assistant"` または `type == "result"` なら空にする。
  - pi: `type == "message_update"` で `assistantMessageEvent.type == "text_delta"` なら `assistantMessageEvent.delta` を足す（キー名は pi の `docs/json.md` で確かめる）。`type == "message_end"` または `type == "agent_end"` なら空にする。
- **cc の費用**: cc の stdout に `type == "result"` の行が来たら、その `session_id` と `total_cost_usd` を、今の `cc-statusline` と同じ形（`{"session_id","total_cost_usd","updated_at"}`）で `phone_export::cc_cost_dir()`（`phone_export.rs:873`）の `<session_id>.json` に書く。書き方は `phone_export.rs:938-956` を関数に切り出して呼ぶ。`read_cc_cost`（`phone_export.rs:371`）以降は変えない。
- **`state.json`**（共有契約 C1。書くのは監督スレッドだけ。変化したとき、ただし `partial` の更新は 200ms に 1 回まで）を一時ファイル → rename で書く。

### 2. `HerdrClient` を擬似 pane だけにする — `crates/sai-link-engine/src/herdr.rs`
- `run_in_pane`（`:703`）・`send_text`（`:710`）・`send_keys`（`:728`）・`get_pane_status`（`:749`）・`read_pane_output`（`:1064`）・`close_pane`（`:1041`）は、`is_headless_pane(pane_id)` なら `crate::headless::…` を呼ぶ。そうでない id は `Err(HerdrError::…)`「herdr は使えません（sai-tree は headless だけです）」を返し、**`herdr` の実行ファイルを呼ばない**。
- `rename_pane`（`:1113`）・`clear_pane_label`（`:1134`）は何もせず `Ok`。
- pane を作る・タブ・レイアウトの関数（`split_pane`・`split_pane_from`・`split_pane_in_tab`・`create_tab`・`close_tab`・`snapshot_layout`・`list_panes`・`workspace_list` など）は、呼ばれたら `Err`（同じ文）を返す。§3 で呼ばれなくなる。
- `is_herdr_available`（`:509`）は常に `false`。
- herdr の CLI を呼ぶ `run_command`（`:533`）と、それだけが使う補助は消す。`HerdrClient` という名前は変えない（ponytail: 名前の付け替えは差分が大きいので後回し。`// ponytail: 名前は herdr のままの擬似 pane の窓口` とコメントを付ける）。
- herdr の CLI の出力を読む単体テスト（`herdr.rs` の `#[cfg(test)]`）は、消した関数の分だけ消す。

### 3. pane を作る 2 か所
- `orchestrator.rs:2935-2980`: `prepared.pane_id` が Some のときは今のまま（擬似 pane も `get_pane_status` で取れる）。None のときは `crate::headless::new_pane(cwd_str.as_deref())` を使い、herdr の分割を呼ばない。
- `task_manager.rs:785` `resolve_pane_placement_inner`: 先頭で herdr を呼ばずに `Ok(PlacementResult::Placed("headless".into(), new_pane(cwd)?.pane_id, <今の既定の方向>))` を返す。タブの作成・レイアウト・WIP の上限（herdr の列の数）は使わない（同時実行の上限は `AgentPool` のまま）。`maybe_close_parent_tab`（`task_manager.rs:1058` 付近）は何もしない。
- `orchestrator.rs` の「シェル初期化待機 3 秒」（Step 7・`:3003`）と「pi 初期化待機 5 秒」（Step 9・`:3029`）を消す。

### 4. 起動コマンド
- `orchestrator.rs:3017-3020`: `exit_command` を**付けない**（ターンが終わるたびにプロセスが終わるので、付けると 1 ターンで完了してしまう）。`exit_command` を作るコード（`:905-927`）も消す。
- `cc_command.rs:168` `build`:
  - 実行ファイルを `cc` ではなく、環境変数 `SAI_CLAUDE_BIN`（無ければ `claude`）にする（Linux の `cc` は C コンパイラなので shim を置かない）。`runner.rs:62` の `bin()` も `claude` にする。
  - 先頭に `-p --output-format stream-json --verbose --include-partial-messages` を足す。
  - **権限**: `--dangerously-skip-permissions` をやめ、`--permission-mode acceptEdits --allowedTools <一覧>` にする。一覧は定数 `CC_ALLOWED_TOOLS`（`Bash`・`Edit`・`Write`・`MultiEdit`・`Read`・`Glob`・`Grep`・`LS`・`WebFetch`・`WebSearch`・`NotebookEdit`・`Task`・`TodoWrite`・`AskUserQuestion`、MCP の道具 `mcp__*`）。`cc_skip_permissions`（`:231`）と `SAI_CC_SKIP_PERMISSIONS` は消す。理由: コンテナは root（rootless Docker のコンテナの root ＝ホストの利用者）で動き、Claude Code は root で `--dangerously-skip-permissions` を拒否する（調査 R3 §Q2）。
  - 他の引数（`--session-id`・`--model`・`--effort`・`--append-system-prompt-file` など）は今のまま。
- `pi_command.rs:487` `build`:
  - 実行ファイルを環境変数 `SAI_PI_BIN`（無ければ `pi`）にする。
  - `--mode json` と、プロジェクトの `.pi/settings.json`（質問の拡張）を信頼して読ませる `--approve` を足す。
- 依頼文は stdin で渡す（§1 のターンの始め方）。`-p`（cc）と `--mode json`（pi）が stdin の本文を依頼として読むことを、実物で確かめる（要確認）。

### 5. gl
- `RunnerKind::Gl` は使えなくする。`prepare_pipeline`（`orchestrator.rs:653`）で gl なら `OrchestratorError` で「gl は sai-tree では使えません」と失敗させる。`card options` の runner の一覧（`sai-link card options`）から gl を外す。

## 共有契約（他の作業と共有する取り決め。worker は変えない）

**C1 headless の書き出し**（U02・U03 が読む。U01 が書く）
- 擬似 pane の id: `hl-` + 12 桁の 16 進。実行の書き出し（`$SAI_HOME/web/runs/<project>/<run_id>.json`）の `herdr_pane_id` にこの id が入る（今の項目名のまま）。
- 置き場: `<project_root>/.sai/sai-engine/headless/<id>/`。Web が読んでよいのは `state.json` だけ。他のファイルは engine の中だけで使う。
- `state.json` の形:
  ```json
  {"pane_id":"hl-0123456789ab","runner":"cc","state":"working","turn":2,
   "partial":"生成中の文","error":"","updated_at":"2026-10-07T12:00:00+09:00"}
  ```
  `state` は `working`（ターンのプロセスが動いている）・`idle`（待っている）・`stopped`（閉じた）のどれか。`runner` は `cc` か `pi`。`partial` は生成中の文（無ければ空）。`error` は直前のターンが失敗したときの理由 1 行（無ければ空）。
- 質問のフックへ渡す環境変数: ターンのプロセスに `HERDR_PANE_ID=<id>` と `SAI_AGENT_TRANSPORT=headless`。

## ファイルの所有権
- 書いてよい: `crates/sai-link-engine/src/headless.rs`（新規）・`lib.rs`・`herdr.rs`・`orchestrator.rs`・`task_manager.rs`・`cc_command.rs`（`build` と権限の部分だけ）・`pi_command.rs`・`runner.rs`・`phone_export.rs`（cc-cost の書き方の切り出しだけ）・`crates/sai-link-engine/tests/headless_*.rs`（新規）・`crates/sai-link-cli/src/` のうち `card options` の runner の一覧
- 読むだけ: `crates/sai-link-engine/src/agent_api.rs`・`daemon.rs`
- 触らない: `cc_command.rs` の `cc_hooks_json`・`cc_settings_json`（U02 が書く）・`apps/`

## 完了条件
- [ ] AC1: `cargo test --workspace` がすべて通る（herdr の CLI を前提にしたテストは消すか、擬似 pane に直す。消したテストは作業報告書に名前を書く）。
- [ ] AC2: `resume_command` と `update_partial` の単体テストが通る（cc・pi の両方。上の規則の例を 1 つずつ）。cc の `result` 行から cc-cost のファイルが書かれる単体テストが通る。
- [ ] AC3: 結合テスト `crates/sai-link-engine/tests/headless_turns.rs`: 偽のエージェント（stdin を読み、`{"type":"stream_event",…}` を 2 行、`{"type":"assistant",…}`、`{"type":"result","session_id":"s1","total_cost_usd":0.01,…}` を出して終わる sh スクリプト）を `SAI_CLAUDE_BIN` で差し、擬似 pane に `run` → `send_text` → `send_keys("Alt+Enter")` すると、`state.json` が `working` → `idle` になり、`stream.jsonl` に `sai_turn` の行とエージェントの行が入る。2 回目の送信では `command` に `--resume` が入る。`send_keys("esc")` で長く眠る偽のエージェントが止まる。`close` で `stopped` になる。
- [ ] AC4: 一時フォルダのプロジェクトで daemon を動かし、偽のエージェント（最初のターンで `sai-link task report` を呼ぶ）を使って、`sai-link task create` → `execute` → `wait` が `completed` を返す。`sai-link task send` → 次のターンが走る。`sai-link task capture` が `stream.jsonl` の末尾を返す。`sai-link task finalize` で `state.json` が `stopped` になる。この間 `herdr` は 1 度も呼ばれない（PATH の先頭に、呼ばれたら印のファイルを作る偽の `herdr` を置いて確かめる）。
- [ ] AC5: 実物の確認（費用の小さい model で 1 回だけ）: `claude -p` と `pi --mode json` が stdin の依頼を読み、stream-json / JSON Lines を出すこと、`claude` が `--permission-mode acceptEdits --allowedTools …` で `Bash` と `Write` を確認なしで使えることを確かめ、結果（使ったコマンドと出力の先頭）を作業報告書に書く。

## 検証
```bash
cd ~/workdir/Sai/sai-tree
env -u SAI_PROJECT_ROOT -u SAI_PARENT_RUN_ID -u SAI_BLOCK_NAME \
  perl -e 'alarm 1800; exec @ARGV' cargo test --workspace
```
AC4 は一時フォルダで行う（`SAI_HOME` も一時フォルダにする。`sai-link init` は一時フォルダでだけ実行する。daemon は最後に必ず止める）。

## 決定事項と理由
- 擬似 pane を `HerdrClient` の中で受ける — 理由: herdr を呼ぶ場所が 30 か所以上あり（orchestrator・task_manager・agent_api）、窓口 1 つを差し替えれば呼び出し側を変えずに済む。/ 採用しなかった案: runner の trait を作って全部の呼び出しを書き換える（差分が大きい）、herdr の経路を残して切り替える（sai-tree では herdr を使わないので要らない）。
- ターンごとにプロセスを起こす（`claude -p` を毎回起動し、2 回目からは `--resume`） — 理由: stdin で多ターンを流す方式（`--input-format stream-json`）は説明が薄く（anthropics/claude-code#24594）、中断の制御メッセージも公開の文書が無い。ターンごとなら、中断はプロセスを止めるだけで済み、cc と pi で同じ作りにできる。/ 採用しなかった案: 常駐プロセスの stdin に流す・Agent SDK を使う（規約上、Agent SDK はサブスクのログインを使えない）。
- 送受は `draft.txt`・`inbox/`・`interrupt`・`stop` のファイルで行う — 理由: `sai-link task send` などの CLI のプロセスと、監督スレッドがいる daemon のプロセスは別で、今の通知もファイルで受け渡している。
- cc は許可の一覧で起動する — 理由: 利用者の決定。コンテナは root で動き、`--dangerously-skip-permissions` は root で拒否される。公式文書に無い `IS_SANDBOX=1` には頼らない。
- cc の費用は stream-json の `result` 行から書く — 理由: headless では statusLine が呼ばれない。同じ形で書けば読む側を変えずに済む（調査 E3 §4.3）。

## 要確認
- `claude -p` が stdin の本文を依頼として読むこと（位置引数なし）。読まない場合は、本文をファイルに書き、`claude -p "$(cat <file>)"` の形にする。どちらにしたかを報告に書く。
- `pi --mode json` が stdin の本文を読むか。読まなければ位置引数で渡す（`sh -c` に渡すときは一重引用符でくくる。`herdr::shell_escape_single_quotes` を使う）。`--continue` と `--approve` の綴り、`message_update` の中の差分の項目名も pi の `docs/json.md`・`--help` で確かめる。
- `--allowedTools` が `mcp__*` の形を受けるか。受けなければ、`.mcp.json` に書かれた MCP の名前ごとに `mcp__<名前>` を足す。
- `result` 行の項目名（`total_cost_usd`・`session_id`）を実物で確かめる。
- daemon が止まると監督スレッドも消える。そのとき動いていた headless の実行は、次に daemon が起きたときの幽霊の処理（`daemon recover`）に任せる（今回は引き継がない）。
