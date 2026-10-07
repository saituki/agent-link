---
block: explore
run: 407
description: 作業報告書 U01: cc/pi の起動・監視・終了の経路と herdr 依存を path:line 付きで調査
---

# 作業報告書 U01: cc / pi の起動・監視・終了と herdr 依存

対象: sai-moon（`/Users/touta/workdir/Sai/sai-moon`）。**コードは読むだけで変更していない。**
事実には `path:line`（1 始まり）を付ける。確かめられていない推論は「（推測）」、未確定は「（要確認）」と書く。

## 0. 全体像（3 プロセスと受け渡し）

- 実行エンジンは Rust の `sai-link` CLI と常駐 `sai-link daemon`、窓口は `apps/web`（FastAPI）。
- **エージェント（cc / pi）は herdr の pane の中で、シェル経由で起動される。** pane 分割・コマンド投入・キー送信・状態取得・画面読み・pane 削除はすべて `herdr` CLI を子プロセスで呼んで行う（`crates/sai-link-engine/src/herdr.rs`）。
- CLI と daemon の受け渡しは `.sai/sai-engine/notifications/` 配下の JSON ファイル。`pending/` に積み、処理して `processed/` へ move、完了は `results/` に書く（`crates/sai-link-engine/src/task_manager.rs:39` 付近の `notify_base_dir`、`crates/sai-link-engine/src/daemon.rs:509`）。

---

## 問い 1. cc / pi プロセス起動までの経路とコマンドライン組み立て

### 1-a. `sai-link card run <run_id>`（人の実行 → デーモン → メインブロック）

事実（経路）:
1. `crates/sai-link-cli/src/cmd_card.rs:35` — `CardCmd::Run` を `PhoneCmd::Card { PhoneCardCmd::Run }` に移す。
2. `crates/sai-link-cli/src/phone.rs:237` → `crates/sai-link-cli/src/phone.rs:1331` `card_run(run_id)`。todo のカードを選び（`phone.rs:1334` の `TODO_OWNER`）、`crates/sai-link-cli/src/phone.rs:1349` で `kanban_run::run_card_request` を呼ぶ。
3. `crates/sai-link-cli/src/kanban_run.rs:90` `run_card_request` — カードをパースし、block 定義を解決（`kanban_run.rs:127` 付近）、`reserved_run_id` を frontmatter の `run_id` から取る（`kanban_run.rs:137`）。**人が起こすカード（executer: agent）はここで弾く**（`kanban_run.rs:106` 付近）。最後に `crates/sai-link-cli/src/kanban_run.rs:200` で `sai_link_engine::daemon::write_block_run_request_full(...)`。
4. `crates/sai-link-engine/src/daemon.rs:1322` `write_block_run_request_full` — `block_run_request` JSON を `.sai/sai-engine/notifications/pending/` に書く（`daemon.rs:1329`〜`1352`）。
5. デーモンのループ `crates/sai-link-engine/src/daemon.rs:361` `loop_forever` が `daemon.rs:509` `process_block_run_request` で拾い、`daemon.rs:708` `start_main_block` を呼ぶ。
6. `daemon.rs:759`〜`768` で別スレッドを起こし、`crates/sai-link-engine/src/orchestrator.rs:1959` `execute_block_pipeline_full` を実行する。

事実（コマンドライン組み立て）:
- 組み立ての中心は `crates/sai-link-engine/src/orchestrator.rs:653` `prepare_pipeline`。**この時点では herdr を一切触らない**（`orchestrator.rs:648` のコメント）。
- セッションディレクトリは run フォルダの `session/`（`orchestrator.rs:694`〜`733`）。固定部分（template＋本文）は run フォルダの `system-prompt.md` に書き（`orchestrator.rs:955` `write_system_prompt_file`）、最初の文には入力値と範囲だけを残す（`orchestrator.rs:757` 付近）。
- runner 分岐（`orchestrator.rs:761`〜`816`）:
  - **pi**: `orchestrator.rs:763` `PiCommandBuilder::new(&session_dir_str)`。`with_config`（`orchestrator.rs:765`）、外部 MCP 設定（`orchestrator.rs:770` `resolve_mcp_config` → `with_mcp_config`）、固定部分（`orchestrator.rs:776` `with_system_prompt_file`）を積んで `build()`（`crates/sai-link-engine/src/pi_command.rs:487`）。
  - **cc**: `orchestrator.rs:787` `check_project_files` で `.claude/settings.json` / `.mcp.json` のあるディレクトリを決め、`orchestrator.rs:793` `generate_uuid_v4()` で `agent_session_id` を作り、`orchestrator.rs:797` `CcCommandBuilder::new(&session_id)`。cwd がプロジェクトルート外なら `with_config_dir`（`orchestrator.rs:801`）、固定部分を `with_system_prompt_file`（`orchestrator.rs:804`）。`build()` は `crates/sai-link-engine/src/cc_command.rs:168`。
  - **gl**（sai-gemini-tui）: `orchestrator.rs:816` で `("gl".to_string(), Vec::new())`。**タスク書き込みなし・プロンプト送信なし**（`crates/sai-link-engine/src/runner.rs:80` の `is_no_task_write`）。
- コマンド先頭への前置（`orchestrator.rs:824`〜`926`）:
  - `.sai/settings.json` と `~/.sai/settings.json` の `env` を `KEY='値' ` として先頭に付ける（`orchestrator.rs:827` の `agent_env_pairs` → `pi_command.rs:273`）。
  - `SAI_PROJECT_ROOT` / `SAI_PARENT_RUN_ID` / `SAI_BLOCK_NAME`（`orchestrator.rs:861`）、子なら `SAI_IS_SUBTASK=1 SAI_LINK_BIN=…`（`orchestrator.rs:838` 付近）。
  - `PATH=<sai-link のあるフォルダ>:$PATH`（`orchestrator.rs:891`）— 子やフックが `sai-link` を名前だけで呼べるように。
  - メインの pi / cc だけ、終了後に続けて `sai-link run finalize <run_id>` を走らせる `exit_command` を作る（`orchestrator.rs:905`〜`927`）。
- cc の引数（`crates/sai-link-engine/src/cc_command.rs:168`〜`215`）: `--session-id <uuid>`（または `--resume`）、`--model`、`--effort`、`--dangerously-skip-permissions`（既定。`SAI_CC_SKIP_PERMISSIONS=0` で `--permission-mode acceptEdits`）、必要時のみ `--mcp-config` / `--settings`、固定部分は **`--append-system-prompt-file <path>`**（`cc_command.rs:211` 付近）。
- pi の引数（`crates/sai-link-engine/src/pi_command.rs:487`〜`537`、詳細は `pi_command.rs:542` `agent_args_for_config`）: **`--session-dir <dir>` が必須の先頭**、`--model`、`--thinking`、`--provider`（モデルから自動検出。`pi_command.rs:677` `resolve_provider_for_model`）、`--tools` / `--no-tools`、`--mcp-config`、固定部分は **`--append-system-prompt <path>`**（cc とフラグ名が違う）。`--extension` は付けない（`pi_command.rs:604` 付近）。

事実（pane 作成 → 起動）:
- `crates/sai-link-engine/src/orchestrator.rs:2909` `execute_pipeline_background`。`orchestrator.rs:2935`〜`2980` で pane を用意する（`pane_id` 指定ありは既存 pane を `get_pane_status`、`base_pane_id` は `split_pane_from`、`tab_id` は `split_pane_in_tab`、無指定は `split_pane`）。
- pane ラベル `sai-<block>-<run_id>` を付け（`orchestrator.rs:2985`、`rename_pane`）、起動前に 3 秒待つ（`orchestrator.rs:3003`）。
- 起動コマンドは `cd '<cwd>' && <pi_command>` の形にし（`orchestrator.rs:3008`〜`3014`）、`;` で `exit_command` を繋ぎ（`orchestrator.rs:3018`）、`orchestrator.rs:3022` `herdr.run_in_pane` で pane に投入する。以降 pi 初期化 5 秒待ち（`orchestrator.rs:3029`）、idle/done を最大 30 秒待ち（`orchestrator.rs:3033`）、プロンプト送信（`orchestrator.rs:3065` `submit_prompt_verified`）。

### 1-b. `sai-link task create` → `sai-link task execute`（サブタスク）

事実:
1. `sai-link task create`: `crates/sai-link-cli/src/cmd_task.rs:27` → `crates/sai-link-engine/src/agent_api.rs:628` `task_create`。`write_notification("task_created", …)`（`agent_api.rs:122`、`agent_api.rs:899` 付近）で pending に書く。承認は廃止され、常に `approval.status="approved"`（`agent_api.rs:901`）。
2. デーモンが `crates/sai-link-engine/src/task_manager.rs:2051` で `process_task_created`（`task_manager.rs:511`）を呼び、DB に `approved` で保存。
3. `sai-link task execute`: `cmd_task.rs:42` → `agent_api.rs:956` `task_execute`。`task_execute_request` を pending に書く（`agent_api.rs:1000` 付近）。
4. デーモンが `task_manager.rs:2070` 付近で `process_batch_execute`（`task_manager.rs:666`）または `process_task_execute`（`task_manager.rs:627`）を呼ぶ。`process_task_execute` は `try_promote`（`task_manager.rs:1326`）で AgentPool（`crates/sai-link-engine/src/agent_pool.rs`）に投入する。
5. デーモンのワーカー 4 本（`daemon.rs:39` MAX_WORKERS、`daemon.rs:388`）が `task_manager.rs:1524` `execute_subtask` を実行。
6. `execute_subtask` は `task_manager.rs:1574` で `resolve_pane_placement`（`task_manager.rs:775` / 本体 `:785`）を呼び、**タブ取得/作成 → レイアウト取得 → split までをここで行い、作成済み pane_id を得る**（`task_manager.rs:793` `create_tab`、`:832` `snapshot_layout`、`:924` 付近 `split_pane_from`）。
7. `task_manager.rs:1593` で `crates/sai-link-engine/src/orchestrator.rs:2596` `run_subtask` → さらに `execute_block_pipeline_full`（`orchestrator.rs:2726` 付近）へ。`pane_id` は `Some` で渡されるため、`execute_pipeline_background` は分割せず既存 pane を使う（`orchestrator.rs:2942`）。

runner ごとの差（まとめ）:
| | 実行ファイル | セッション指定 | 固定部分フラグ | 確定キー | 起動時のタスク書き込み |
|---|---|---|---|---|---|
| pi | `pi`（`runner.rs:61`） | `--session-dir` | `--append-system-prompt` | `Alt+Enter`（`runner.rs:75`） | あり |
| cc | `cc`（`runner.rs:62`。Claude Code の shim 前提） | `--session-id`/`--resume` | `--append-system-prompt-file` | `Alt+Enter` | あり |
| gl | `gl`（`runner.rs:63`） | なし（空セッション） | なし | `Enter` | なし（`runner.rs:89`） |

---

## 問い 2. herdr 依存箇所の全一覧と stalled 判定

### 2-a. herdr で何をしているか（Rust 側 API）

`crates/sai-link-engine/src/herdr.rs` の `HerdrClient`（`herdr.rs:420` 付近）:
- pane 作成: `split_pane`（`herdr.rs:672`）、`split_pane_from`（`herdr.rs:941`）、`split_pane_in_tab`（`herdr.rs:889`）、`create_tab`（`herdr.rs:793`）。すべて `herdr pane split …` / `herdr tab create --no-focus`。
- コマンド実行: `run_in_pane`（`herdr.rs:703`）= `herdr pane run <pane> <command>`。
- 文字・キー送信: `send_text`（`herdr.rs:710`）= `pane send-text`、`send_keys`（`herdr.rs:728`）= `pane send-keys`。
- 状態・一覧: `get_pane_status`（`herdr.rs:749`）= `pane get`、`list_panes`（`herdr.rs:764`）= `pane list`、`workspace_list`（`herdr.rs:817`）、`list_panes_in`（`herdr.rs:823`）、`find_workspace_for_project`（`herdr.rs:838`）、`snapshot_layout`（`herdr.rs:981`）、`is_herdr_available`（`herdr.rs:509`）。
- 画面読み（capture）: `read_pane_output`（`herdr.rs:1064`）= `pane read`（`apps/web` 側が主な利用者）。
- 終了（kill）: `close_pane`（`herdr.rs:1041`）、`close_tab`（`herdr.rs:1011`）。
- ラベル: `rename_pane`（`herdr.rs:1113`）、`clear_pane_label`（`herdr.rs:1134`）。
- 呼び出しはすべて `std::process::Command` で `herdr` を spawn（`herdr.rs:533` `run_command`）。**`env` は明示設定していない**（後述 Q6）。タイムアウトは `SAI_HERDR_TIMEOUT_MS`（既定 30000、`herdr.rs:434`）。

`PaneInfo.agent_status`（`herdr.rs:87`）が "idle" / "working" / "done" / "blocked" 等。

### 2-b. Rust 側の herdr 呼び出し箇所（`src`、テスト除く）

- `crates/sai-link-engine/src/orchestrator.rs`
  - `:1810` `close_execution_pane` → `close_pane`
  - `:2919` `HerdrClient::new`（`execute_pipeline_background` の先頭）
  - `:2944` `get_pane_status`、`:2953` `split_pane_from`、`:2963` `split_pane_in_tab`、`:2971` `split_pane`、`:2985` `rename_pane`、`:3022` `run_in_pane`
  - `:3482` / `:3596` `get_pane_status`（完了待ちループ）
  - `:3743` / `:3799` `get_pane_status`（`send_turn_to_pane` / 待機）
  - `:3986` `send_keys`、`:3996` `get_pane_status`（`submit_prompt_verified`）
  - `:4061` / `:4102` / `:4133` `get_pane_status`（各種待機）
- `crates/sai-link-engine/src/task_manager.rs`
  - `:451` / `:468` / `:1764` `HerdrClient::new`（`:1764` は `execute_task_turn` 用）
  - `:793` `create_tab`、`:832` `snapshot_layout`、`:924` 付近 `split_pane_from`（`resolve_pane_placement_inner`）
  - `:1003` `get_pane_status`（親 workspace 解決）、`:1058` `close_tab`（`maybe_close_parent_tab`）
- `crates/sai-link-engine/src/agent_api.rs`
  - `:1565` `send_keys("esc")` + `:1567` 付近 `get_pane_status`（`task_manage` interrupt）
  - `:1614` `get_pane_status`（send のルーティング）、`:1637` `submit_prompt_verified`（同一 pane 送信）
  - `:1770` `HerdrClient::new`（`task_capture` の画面読み）
- `crates/sai-link-engine/src/herdr.rs` — 本体。

### 2-c. apps/web 側の herdr 依存（すべて `herdr` CLI を subprocess で呼ぶ）

`apps/web/src/sai_web/herdr.py`:
- `:52` `list_agents` = `herdr agent list`、`:65` `list_panes` = `herdr pane list`、`:82` `pane_gone`、`:90` `get_agent` = `herdr agent get`、`:120` `prompt_agent`（claude は `pane send-text` + 確定キー、他は `herdr agent prompt`）、`:204` `send_keys` = `herdr agent send-keys`、`:216` `read_pane` = `herdr pane read --format text`、`:259` `send_text` = `herdr pane send-text`。
- 実行ファイル: `apps/web/src/sai_web/config.py:44`〜`50`（`HERDR_BIN` / `HERDR_BIN_PATH` / `which("herdr")` / `~/.local/bin/herdr`）。

呼び出し元:
- `apps/web/src/sai_web/main.py:206` / `:214`（実行一覧に live 状態を付ける）、`:504` / `:522`（run へのプロンプト送信）。
- `apps/web/src/sai_web/runs.py:290` 付近（pane と run の対応）、`apps/web/src/sai_web/runs.py:304` 付近 `live_status`。
- `apps/web/src/sai_web/approvals.py:144` `pane_gone` / `:148` `read_pane` / `:154` `get_agent` / `:220` `get_agent` / `:241` `send_keys` / `:250` `send_text` / `:254` `send_keys`（承認画面）。
- `apps/web/src/sai_web/questions.py:96` / `:142` / `:147` / `:158` / `:299` / `:318` / `:321` / `:370` `read_pane`・`get_agent`・`pane_gone`、`:416` / `:429` / `:447` / `:454` / `:472` / `:477` / `:527` `send_keys`・`send_text`（質問画面）。
- `apps/web/src/sai_web/watch.py:77` / `:78`（ポーリングして live を配る）。

**apps/web は pane を作らない・kill しない。** pane 作成/削除は Rust 側（daemon と CLI）だけが行う。

### 2-d. `task wait` の stalled 判定の根拠

事実: 判定は 2 段に分かれる。
1. **検出（書く側）** は orchestrator の完了待ちループ。`crates/sai-link-engine/src/orchestrator.rs:3144` `StallDetector` / `:3149` `observe`。250ms ごとに `herdr pane get` の `info.agent_status`（`orchestrator.rs:3482`）を渡し、
   - `working` が `SAI_LONG_RUN_ALERT_MS`（既定 20 分、`orchestrator.rs:3225` `long_run_alert_ms`）続く → `LongRunning`（`observe` の `orchestrator.rs:3158` 付近）
   - `idle` が `SAI_IDLE_TIMEOUT_MS`（既定 60 秒、`orchestrator.rs:3324`）続く → `Idle`
   - `blocked` / `error` が連続 2 回 → `Blocked` / `Error`
   を返したら `task_manager::write_stalled_notification`（`crates/sai-link-engine/src/task_manager.rs:2300`、実体 `:2240`）で `results/<時刻>-task_<id>.json` に `type:"task_stalled"` を書く（`task_manager.rs:2301` 付近、古い stalled は消す `task_manager.rs:2249` 付近）。
   - timeout 超過は別途 `timeout` 理由で同じ通知を書く（`orchestrator.rs:3355`〜`3365`）。
   - 子が `sai-link task question` したときは `reason:"question"`（`crates/sai-link-engine/src/agent_api.rs:1401` `task_question` → `agent_api.rs:1426` `write_stalled_notification`）。
2. **読む側** が `sai-link task wait`。`crates/sai-link-engine/src/agent_api.rs:1213` `task_output_ids` が 500ms ごとに `results/` を舐め（`:1252` 付近）、`agent_api.rs:1026` `find_result_file` で `-task_<id>.json` を探す。**完了（task_stalled 以外）を優先**し、無ければファイル名が一番新しい stalled を返す（`agent_api.rs:1047`〜`1059`）。`agent_api.rs:1072` `build_task_output_result` の `:1134` 分岐が `status:"stalled"` に `reason`（idle/blocked/error/long_running/timeout/question）を付けて返す。
- 上限: 1 回の待ちは 90 秒（`agent_api.rs:1177` `TASK_OUTPUT_MAX_WAIT_MS`）。

補足（事実）: デーモン連携時の完了待ちは herdr の状態ではなく **自分の run_id の sai-write 検出イベント**を正とする（`orchestrator.rs:3345` 付近のコメント、`daemon.rs:437`〜`460`）。herdr の `agent_status` は、(a) pane 消滅の検出（連続 3 回エラーで「消えた」とみなし出力抽出して完了扱い、`orchestrator.rs:3530` 付近）と、(b) サブブロックの stalled 検知にだけ使う。

---

## 問い 3. 完了・失敗の検出と run の状態遷移

### 3-a. 完了・失敗の検出方法

- **サブブロックの正常完了**: 子が `sai-link task report`。`crates/sai-link-cli/src/cmd_task.rs:124` → `crates/sai-link-engine/src/agent_api.rs:1345` `task_report`。本文を run フォルダの `finalize-report.md` に書き（`agent_api.rs:504` `REPORT_FILE`、`agent_api.rs:517` `write_report_file`）、`crates/sai-link-engine/src/daemon.rs:1362` `write_finalize_request` で `finalize_request` を pending に書く。
- デーモン `crates/sai-link-engine/src/daemon.rs:816` `process_finalize_request` が対応する実行の監視チャネルへ `CompletionEvent::FinalizeRequested` を送る（`daemon.rs:860` 付近）。orchestrator の完了待ちループがこれを受けて `break`（`crates/sai-link-engine/src/orchestrator.rs:3419`）、`execute_block_pipeline_full` の成功パス（`orchestrator.rs:2085`〜`2110`）が `store.update_execution_status(run_id,"completed")` する。
- **メインブロックの正常完了**: (1) sai-write（`sai-link doc write`）を daemon が検出して `CompletionEvent::SaiWriteDetected` を送る（`daemon.rs:437`〜`460`、`orchestrator.rs:3349` 付近で受信）、(2) 子と同じ `finalize_request` 経由、(3) エージェントが終了すると pane のシェルが続けて `sai-link run finalize <run_id>` を実行（`orchestrator.rs:905`〜`927` で組み立て、`orchestrator.rs:3018` で `;` 連結）。`sai-link run finalize` は `crates/sai-link-cli/src/cmd_run.rs:53` → `crates/sai-link-engine/src/agent_api.rs:535` `finalize` → `write_finalize_request`。
- **Ctrl+D 2 回**（TUI）: `crates/sai-link-engine/src/orchestrator.rs:1836` `request_main_block_finalize`（`finalize_request` 通知＋pane close＋DB completed+finalized）。
- **失敗**: `execute_block_pipeline_full` の Err パス（`orchestrator.rs:2200` 付近）が `store.update_execution_error(run_id, …, 1)`。サブタスクは `crates/sai-link-engine/src/task_manager.rs:1914` `fail_task`（status `failed` + `task_failed` 通知）。
- **pane 消滅**（ユーザーが pane を閉じた等）: 完了待ちループで `get_pane_status` が連続 3 回エラーになると、その時点の出力を抽出して正常完了扱い（`orchestrator.rs:3541`、ケース 4）。
- **セッション jsonl は完了判定には使わない**。使うのは完了後の出力・統計抽出（`orchestrator.rs:1254` `extract_outputs_for_pipeline`、cc は `orchestrator.rs:1243` `claude_session_path`、pi は run の session dir）。

### 3-b. run の状態（todo/running/review/done）を誰が書くか

事実（run フォルダ owner の対応は `crates/sai-link-engine/src/run_dir.rs:48` `owner_for`、`:14`〜`:20` の定数）:
- **todo**: カード作成時。`crates/sai-link-cli/src/phone.rs:1127` `card_create` が `run_dir::ensure_run_dir(TODO_OWNER, …)` する。
- **running**: 実行レコード作成時。`crates/sai-link-engine/src/orchestrator.rs:1493` `prepare_execution_record_full` が `store.update_execution_status(run_id,"running")`（予約 run は `orchestrator.rs:1543`、子の予約は `:1568`、新規は `:1600`）。フォルダの todo→running move は `execute_block_pipeline_full` の `orchestrator.rs:2035`〜`2039`（`reserved_run_id` があるとき）。
- **review**: メインブロックの正常完了時。`orchestrator.rs:2181`〜`2184` が `REVIEW_OWNER` へ move（サブタスクは pane が生きている間は running のまま。`run_dir.rs:74` `subtask_owner`）。
- **done / R<global_round>**: 人が `sai-link phone advance` / `user finish` で動かす。`crates/sai-link-cli/src/phone.rs:1424`〜`1445`、実体は `crates/sai-link-engine/src/run_dir.rs:337` `advance_review_to_done` / `:363` `advance_done_to_workflow`。
- DB のタスク状態: `task_manager.rs:1859` `complete_task`（`completed`）、`:1914` `fail_task`（`failed`）、`process_task_manage` の finalize で `finalized`（`task_manager.rs:1127`）、requeue で `approved`（`task_manager.rs:1252`）。

---

## 問い 4. 会話記録の読み取りと `SAI_HOME/web/runs/*.json` の書き手

### 4-a. 会話記録（jsonl）をどこで読むか

- **cc**: `~/.claude/projects/<cwd-slug>/<session_id>.jsonl`。`crates/sai-link-engine/src/session.rs:1851` `claude_projects_dir`、`:1859` `claude_project_slug`、`:1869` `claude_session_file(cwd, session_id)`、`:1883` `claude_session_file_any_cwd`（cwd 不明時）。読むのは `session.rs:1893` `visit_claude_jsonl`、集計は `extract_claude_final_output` / `extract_claude_full_transcript` / `extract_claude_session_stats`（`orchestrator.rs:1254` の分岐で使用）。
- **pi**: run フォルダの session dir 配下の jsonl。`crates/sai-link-engine/src/session.rs:497` `find_main_session_file`、走査 `session.rs:1818` `visit_jsonl_files`、抽出は `extract_full_transcript` / `extract_session_stats`。
- **gl**: `~/.sai-gemini/sessions/`（`orchestrator.rs:1280` 付近のコメント。sai-link 側は空を返す）。
- **apps/web 側**は runs の `transcript_path` 1 本だけ読む。`apps/web/src/sai_web/sessions.py:1`〜`15`（方針）、`:read_conversation`（`sessions.py:497` 付近）が行単位・バイトカーソルで cc/pi 両対応。`herdr pane read` は会話には使わない（`sessions.py:9`）。

### 4-b. `SAI_HOME/web/runs/<project>/<run_id>.json` を誰がいつ書くか

事実:
- 書き手は **Rust 側だけ**。`crates/sai-link-engine/src/phone_export.rs` の `Exporter`（`:717` `Exporter`、`:756` `export`）と `write_run_json_in`（`:645`）。置き場は `sai_home()/web/runs`（`phone_export.rs:633` `export_root`、`:638` `export_dir`）。一時ファイル `.<run_id>.json.tmp` → rename（`phone_export.rs:645`〜`661`、`:617` `write_json_atomic`）。
- daemon が定期的に呼ぶ: `crates/sai-link-engine/src/daemon.rs:1055` `export_phone_runs`。起動時に全件（`daemon.rs:1234` 付近 `export_phone_runs(true)`）、以後 `tick % STATUS_WRITE_EVERY`（=10、`daemon.rs:63`）ごと、約 5 秒間隔（`POLL_INTERVAL`=500ms ×10、`daemon.rs:57`）。`status` / `finished_at` / `session_stats_json` が変わった実行だけ書く（`daemon.rs:1085`〜`1105`）。終了状態になった実行は次周期でもう一度書く（stats/transcript の遅れ対策、`daemon.rs:1085` 付近）。
- 手動の `sai-link phone export [<run_id>]` も同じ関数を使う（`crates/sai-link-cli/src/cli.rs:1975` `export_one` / `:1981` `export_all`）。
- `transcript_path` の解決は `phone_export.rs:303` `resolve_transcript`（cc は `agent_session_id` ＋ cwd/worktree、pi は `session_dir`）。
- **apps/web は書かない**。`apps/web/src/sai_web/runs.py:1`〜`5`（読むだけ）、`apps/web/src/sai_web/config.py:54` `runs_dir`（既定 `<SAI_HOME>/web/runs`、`SAI_WEB_RUNS_DIR` で変更可）。

---

## 問い 5. headless（`claude -p --output-format stream-json` / pi print/rpc）に差し替える場合

事実:
- **runner の抽象化は「種類の enum」だけ。** `crates/sai-link-engine/src/runner.rs:20` `RunnerKind`（Pi / Cc / Gl）が実行ファイル名 `bin()`（`:60`）、確定キー `submit_key()`（`:74`）、`is_no_task_write()`（`:89`）を返す。**trait は無い**（`crates/sai-link-engine/src/` に `pub trait` は 0 個。`grep` で確認）。
- **pane の抽象化も無い。** `HerdrClient`（`crates/sai-link-engine/src/herdr.rs:420`）が具体型のまま、`orchestrator.rs:2919`、`task_manager.rs:451`、`agent_api.rs:1565` で直接 `HerdrClient::new()` される。差し替え点（trait 化すべきメソッド）は `run_in_pane` / `send_text` / `send_keys` / `get_pane_status` / `split_pane*` / `create_tab` / `close_pane` / `snapshot_layout` / `read_pane_output`。
- 起動〜送信〜完了待ちは `crates/sai-link-engine/src/orchestrator.rs:2909` `execute_pipeline_background` に 1 本化されている。headless では「pane 分割 → run_in_pane → send_text+send_keys → agent_status ポーリング」の代わりに「`claude -p` / `pi --print` を spawn して stdout（stream-json）を読む」に置き換える必要がある。置き換え範囲は `:2935`〜`:3022`（pane 用意と起動）、`:3033`〜`:3080`（プロンプト送信）、`:3482`・`:3596`（状態ポーリング）、`:3530`（pane 消滅判定）。
- 一方で **再利用できる層**:
  - コマンド組み立て `prepare_pipeline`（`orchestrator.rs:653`）は herdr 非依存。headless でも引数生成（cc: `cc_command.rs:168`、pi: `pi_command.rs:487`）は使える。
  - 完了判定は herdr 非依存。完了待ちは sai-write 通知（`daemon.rs:437`）と `CompletionEvent`、`running_executions`（`daemon.rs` の `running_executions`）で回っている。`FinalizeRequested` 分岐（`orchestrator.rs:3425` 付近）は headless でもそのまま使える（推測ではなく、コード上 herdr を参照していないことを確認）。
  - 出力・統計抽出 `extract_outputs_for_pipeline`（`orchestrator.rs:1254`）は runner 分岐済みで、cc/pi の jsonl を読むだけ。headless の stream-json を読む分岐を足す形になる。
  - 同時実行制御 `AgentPool`（`crates/sai-link-engine/src/agent_pool.rs`）と DB/通知は herdr 非依存。
- `apps/web` 側は live 状態・プロンプト送信・質問/承認の回答を herdr に依存している（`apps/web/src/sai_web/herdr.py`、`questions.py:416` 以降、`approvals.py:241` 以降）。headless にするなら、進捗・質問・承認・追加指示の伝達チャネルも herdr 以外（例: run の jsonl を tail／DB／ファイル）に替える必要がある。
- 結論（事実＋推測）: **現状に抽象化は無い。** 差し替えが必要な層は (1) pane 輸送（`HerdrClient` の trait 化または headless 実装の追加）、(2) `execute_pipeline_background` の起動・送信・状態ポーリング、(3) apps/web の herdr 依存 3 機能（live 状態・プロンプト・質問/承認）。(1)(2) は engine 内で閉じるが、(3) は Web と engine の契約（`docs/integration.md` の JSON）にかかわる（推測）。

---

## 問い 6. 認証情報・API キーの渡り方

事実:
- **エージェント起動コマンドに明示注入されるのは `settings.json` の `env` だけ。** `crates/sai-link-engine/src/pi_command.rs:273` `agent_env_pairs` が `.sai/settings.json` と `~/.sai/settings.json` の `env` を読み（`:146` `load_settings_quiet`、`:151` グローバル優先）、`orchestrator.rs:827`〜`835` が `KEY='値' ` としてコマンド先頭に並べる。予約キー `SAI_PROJECT_ROOT` / `SAI_PARENT_RUN_ID` / `SAI_BLOCK_NAME` / `SAI_IS_SUBTASK` は破棄（`pi_command.rs:115` `RESERVED_ENV_KEYS`、`:245`）。
- それ以外の環境変数は**明示されない**。`HerdrClient::run_command`（`herdr.rs:533`）は `Command::new(herdr).args(...)` で `env` を設定しない。したがって pane のシェルが受け取る環境は **herdr サーバの環境の継承**である（推測: herdr を起動したシェル／launchd の環境。`OPENROUTER_API_KEY` 等はここに入っていれば cc/pi に届く）。SAI 側は「利用者の PATH に頼らない」ために `PATH=<sai-link のあるフォルダ>:$PATH` と `SAI_*` だけを前置する（`orchestrator.rs:838`〜`901`）。
- **pi の `~/.pi/agent/models.json`**: `crates/sai-link-engine/src/common.rs:170` `pi_models_json_path`（`PI_CONFIG_DIR` 優先、既定 `~/.pi/agent/models.json`）。`crates/sai-link-engine/src/pi_command.rs:710` `load_models_json` を `resolve_provider_for_model`（`pi_command.rs:677`）が読み、**モデル ID から `--provider` を決めるためだけ**に使う。キーそのものは pi 本体が自前の環境／設定から読む（推測。sai-link はキーを読まない）。
- **cc**: 認証情報（`~/.claude` の資格情報）は sai-link からは注入しない。`sai-link init` が作る `.mcp.json` / `.claude/settings.json` を cwd 外起動時だけ `--mcp-config` / `--settings` で渡す（`cc_command.rs:201`〜`211`）。statusLine から `sai-link cc-statusline` を呼び、`git` 等のフックは cc の設定側（`cc_command.rs:329` `cc_settings_json`）。cc の費用は `~/.sai/cc-cost/<session_id>.json`（`crates/sai-link-cli/src/cli.rs` の `handle_cc_statusline`、`claude statusline` 由来）。
- **`paper.env` / プロジェクト `.env`**: 読むのは **sai-paper CLI 自身**。`crates/sai-paper-core/src/paths.rs:108` `load_project_dotenv` がプロジェクト root の `.env` → `~/.sai/paper.env` を `dotenvy` で読む（`paths.rs:114`）。`crates/sai-paper-cli/src/main.rs:843` が起動時に呼ぶ。キーチェーン行の解決は `paths.rs:128` `load_keychain_env`。**エージェントの pane には注入されない**（sai-paper の検索・翻訳のプロセスだけが読む）。既定の env 名は `crates/sai-paper-core/src/config.rs:379`（`OPENROUTER_API_KEY`）、`:146`（`SEMANTICSCHOLAR_API_KEY`）。
- **`OPENROUTER_API_KEY` のもう 1 つの利用者**: `apps/web`。`apps/web/src/sai_web/usage.py:54`〜`76` `_openrouter_accounts` / `_key_for` が `os.getenv(env_var)`（既定 `OPENROUTER_API_KEY`、複数は `SAI_WEB_OPENROUTER_ACCOUNTS`）で残高・使用量を読む。これは Web サーバの環境であり、エージェントへ渡す経路ではない。
- `apps/web` が CLI を呼ぶときは `{**os.environ, SAI_PROJECT_ROOT, SAI_ORIGIN:"phone"}` を渡す（例 `apps/web/src/sai_web/papers.py:136`、`apps/web/src/sai_web/control.py:236` `_phone_env`）。なので Web プロセスの環境（`.env` 読み込み込み、`apps/web/src/sai_web/config.py:20` `load_dotenv`）が sai-paper に届く。

要点（事実）: sai-link がエージェントへ明示的に渡す秘密は `settings.json` の `env` のみ。`OPENROUTER_API_KEY`・`paper.env`・`.env` は「herdr を起動した環境」または「各 CLI が自分で読む」経由で入る。エージェントの pane に `.env` / `paper.env` を直接注入するコードは無い（`grep` で engine に `dotenv` 呼び出しは 0 件）。

---

## 分からなかったこと / 要確認

- herdr サーバが実際にどの環境で起動されているか（launchd か、人のシェルか）。コードからは分からない。エージェントに API キーが届くかはここに依存する（要確認）。`docs/switch-over.md` に launchd の記載があるので、そこを見ると確かめられる。
- cc の `--append-system-prompt-file` が run フォルダの `system-prompt.md` を指すのは確認したが、そのファイルが worktree 実行時に `.worktrees/<branch>` 側のパスになるかは未確認（`write_system_prompt_file` は run フォルダ基準なのでプロジェクト root 基準。`orchestrator.rs:955`）。worktree の `cd` 後でも絶対パスなので実害は無いと推測。
- headless の pi 側の正確な起動モード名（`--print` / rpc）はこのリポジトリ内にコードが無く、pi 本体の仕様に依存する（要確認）。

