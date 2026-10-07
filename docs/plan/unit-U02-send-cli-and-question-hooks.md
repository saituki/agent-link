# 実装計画書 U02: 実行への送信・中断の CLI、headless での質問、statusline と承認のフックの削除

## 目的
1. `sai-link run send <run_id>` と `sai-link run interrupt <run_id>` を足し、Web がメインの実行（チャット）に続きの文を送ったり、止めたりできるようにする。
2. headless で動く cc・pi が質問（cc の `AskUserQuestion`・pi の `ask_user`）をしたとき、Web の質問カードで答えると、エージェントがその答えを受け取って続けられるようにする。
3. フックと拡張が、Web の場所（`SAI_WEB_URL`）と内部用の合言葉（`SAI_WEB_INTERNAL_TOKEN`）を使えるようにする（コンテナから Web を呼ぶため）。
4. headless では使えない cc の statusLine（`sai-link cc-statusline`）と承認（PermissionRequest）のフックを消す。

## 背景
- sai-tree は sai-moon を複製した Linux の多人数サーバ版で、エージェントはすべて headless（U01 の擬似 pane）で動く。
- 今の Web の送信は herdr を直接呼んでいる（`apps/web/src/sai_web/main.py:489-533` → `herdr.prompt_agent`）。擬似 pane に送るには CLI の口が要る（Web は CLI だけを呼ぶ約束。`ARCHITECTURE.md` §2.1）。
- 今の cc の質問は、フック `apps/web/hooks/cc_question.py` が Web に登録だけして終わり、答えは Web が herdr で端末の画面にキーを送って入れている（`apps/web/src/sai_web/questions.py:483-538` `_send_answers_cc`）。headless には端末の画面が無い。cc の PreToolUse のフックは「ツールの実行を拒否し、その理由の文」をモデルに返せるので、フックが答えを待ち、答えを理由の文に入れて返す。
- pi の質問の拡張は、画面が無いと何もせずに失敗を返す（`apps/web/src/sai_web/extensions/ask_user.ts:353-354`）。答えを Web から取る `pollAnswer`（`ask_user.ts:300-316`）は既にある。
- statusLine は対話の画面でしか呼ばれず、headless では費用は U01 が `result` 行から書く。5 時間枠の表示は削る（要件）。承認は `--permission-mode acceptEdits --allowedTools …` で起動する headless では来ない（調査 E3 §5.2）。

## 前提（着手時点のコードの状態）
- U01 が終わっている: エージェントは擬似 pane（id は `hl-` で始まる）で動く。`HerdrClient` の `send_text`・`send_keys`・`get_pane_status` は擬似 pane に効く。ターンのプロセスには環境変数 `HERDR_PANE_ID=<擬似 pane の id>` と `SAI_AGENT_TRANSPORT=headless` が付く。
- 子タスクの送信・中断は既にある（`crates/sai-link-engine/src/agent_api.rs:1555-1650` の `interrupt` / `send`）。実行（run）単位の送信の命令は無い。
- メインの実行の pane id は、完了まで DB の `herdr_pane_id` に入らず、run の session dir の `herdr_pane_id` ファイルにある（`crates/sai-link-engine/src/phone_export.rs:588` の `PANE_ID_FILE` と、`:591` の非公開の関数 `pane_id_from_session_dir`）。
- `sai-link run` の命令は `crates/sai-link-cli/src/cmd_run.rs:12` `run` と `crates/sai-link-cli/src/cli.rs` の `RunCmd` にある。Web の「コマンド」画面に出すかどうかは `crates/sai-link-cli/src/cmd_schema.rs:179` `web_default`。
- cc のフックの登録は `crates/sai-link-engine/src/cc_command.rs:358` `cc_hooks_json`（`sai-link init` が `.claude/settings.json` に書く）。statusLine は `cc_command.rs:344-347`（`cc_settings_json`）。`PermissionRequest` のフックは `cc_command.rs:374-377`。
- statusLine の CLI は `crates/sai-link-cli/src/cli.rs:141-147` `Cmd::CcStatusline`・`:1993` `handle_cc_statusline`・`main.rs:99`、本体は `phone_export.rs:897` `handle_cc_statusline`・`:907` `handle_cc_statusline_in`（5 時間枠 `:925-935`）。`cc_rate_limits_dir`（`:881`）・`cc_rate_limits_path`（`:886`）。テストは `phone_export.rs:1148`・`:1177`・`:1212`・`:1369-1372`、`cc_command.rs:757-758`。
- `cc_question.py` と `cc_register.py` は Web の場所を `http://127.0.0.1:8200` と決め打ちしている（`apps/web/hooks/cc_question.py:43`）。

## 変更内容

### 1. `sai-link run send` / `sai-link run interrupt`（engine と CLI）
- `crates/sai-link-engine/src/agent_api.rs` に 2 つの関数を足す。
  - `pub fn run_send(run_id: i64, text: &str) -> Result<Value, String>`
    1. 実行を DB から読む（無ければ「実行 <id> が見つかりません」。CLI は終了コード 3）。
    2. pane id = `exec.herdr_pane_id` → 無ければ `phone_export::pane_id_from_session_dir(&exec)`。どちらも無ければ「実行 <id> には送り先がありません」（終了コード 3）。
    3. `HerdrClient::get_pane_status` が失敗する（擬似 pane が `stopped`）なら「実行 <id> は終了しています」（終了コード 4）。`agent_status` が `blocked` なら「質問への回答は質問カードから送ってください」（終了コード 4）。
    4. `orchestrator::submit_prompt_verified(&herdr, &pane_id, text, RunnerKind::parse(&exec.runner).submit_key(), None)`。
    5. 戻り値: `{"run_id": 12, "pane_id": "hl-…", "status": "sent"}`。
  - `pub fn run_interrupt(run_id: i64) -> Result<Value, String>` — 同じ手順で pane を探し、`send_keys(pane, "esc")` を送り、`agent_api.rs:1569-1580` と同じ形で `working` でなくなるまで最大 5 秒待つ。戻り値: `{"run_id": 12, "status": "interrupted" | "still_working", "agent_status": "idle"}`。
  - 終了コードを分けるため、今の `agent_result` / `CmdError` の約束（`crates/sai-link-cli/src/cmd_common.rs`）に合わせ、見つからない・状態の衝突を区別できる形で返す（既存の命令が 3・4 をどう返しているかに倣う）。
- `crates/sai-link-cli/src/cli.rs` の `RunCmd` に足す:
  ```
  sai-link run send <RUN_ID> [--text <文>] [--json]     # --text が無ければ標準入力の全文
  sai-link run interrupt <RUN_ID> [--json]
  ```
  `send` の本文が空なら終了コード 2「送る文が空です」。
- `cmd_run.rs:12` の `match` に 2 つを足す。`web_default` は変えない（どちらも Web の「コマンド」画面に出てよい）。
- `phone_export.rs` の `pane_id_from_session_dir` を `pub(crate)` にする。

### 2. cc の質問フック — `apps/web/hooks/cc_question.py`
- Web の場所: `SAI_WEB_URL`、無ければ `http://127.0.0.1:${SAI_WEB_PORT:-8200}`。フックは標準ライブラリだけで書く。
- 内部用の合言葉: 環境変数 `SAI_WEB_INTERNAL_TOKEN` があれば、すべての要求に `Authorization: Bearer <値>` を付ける。
- 動き（sai-tree では常にこの動き。herdr の経路は無い）:
  1. 今と同じ本文で `POST {url}/api/questions` に登録する。応答 `{"ok": true, "id": "<qid>"}` から id を取る。取れなければ何も出さずに終了コード 0（cc はそのままツールを動かし、headless では選択画面が出ないので失敗が返る）。
  2. 1 秒ごとに `GET {url}/api/questions/{qid}` を読む。答えの取り出し方は `ask_user.ts` の `answersFromState` と同じにする（`state` が `answered` のとき `answer` の中の `answers`）。`state` が `open` 以外で答えが無ければ打ち切る。最長 24 時間。
  3. 答えが取れたら、標準出力に次の JSON を 1 つ書いて終了コード 0:
     ```json
     {"hookSpecificOutput": {"hookEventName": "PreToolUse",
       "permissionDecision": "deny",
       "permissionDecisionReason": "利用者が Web で答えました。\n<質問 1>: <答え 1>\n<質問 2>: <答え 2>"}}
     ```
     打ち切ったときは `permissionDecisionReason` を「利用者は答えませんでした。」にする。
- `cc_register.py` にも、同じ Web の場所の規則と `Authorization` の付け方を入れる。動きは変えない。

### 3. フックと設定 — `crates/sai-link-engine/src/cc_command.rs`
- `cc_hooks_json`（`:358`）: `PreToolUse`（`AskUserQuestion`）のフックに `"timeout": 86400` を足す（秒。cc の既定の待ち時間で切られないように）。`PermissionRequest` のフック（`:374-377`）を消す。
- `cc_settings_json`（`:344-347`）: `statusLine` を消す。
- `hooks_dir_in` などが `cc_permission.py` を数えている所（`:400` 付近）を直す。テスト（`:676-713`・`:757-758` など）を直す。
- `apps/web/hooks/cc_permission.py` を消す。

### 4. statusLine の CLI を消す
- `crates/sai-link-cli/src/cli.rs:141-147` `Cmd::CcStatusline`・`:1993` `handle_cc_statusline`・`main.rs:99` の分岐・`main.rs:10` のコメントを消す。
- `phone_export.rs` の `handle_cc_statusline`・`handle_cc_statusline_in`・`cc_rate_limits_dir`・`cc_rate_limits_path` と、そのテスト（`:1148`・`:1177`・`:1212`・`:1369-1372`）を消す。U01 が切り出した cc-cost の書き方の関数と `cc_cost_dir`・`read_cc_cost` は残す。`unix_now_f64`・`write_json_atomic` は、ほかに使う所が無ければ消す。

### 5. pi の質問 — `apps/web/src/sai_web/extensions/ask_user.ts`
- `SAI_WEB_INTERNAL_TOKEN` があれば、`fetch` のすべての要求に `Authorization: Bearer <値>` を付ける（`registerToServer`・`tellResolved`・`answeredOnServer`・`pollAnswer`・`/api/register`）。
- `handleAskUserPhone`（`ask_user.ts:341`）: 画面が無いとき（`!ctx?.hasUI || !ctx?.ui || ctx.mode === "headless"`）、または `process.env.SAI_AGENT_TRANSPORT === "headless"` のとき、今の「UI is unavailable」で返す代わりに次のようにする。
  1. `registerToServer` で登録する。失敗したら今の「UI is unavailable in this mode.」を返す。
  2. `pollAnswer(serverId)` で答えを待つ（画面の `ui.select` は呼ばない）。
  3. 答えがあれば `result(answeredText(answers), { answers })`、無ければ `result("User cancelled.", { answers: null })`。
- `pollAnswer` の最大回数（`POLL_MAX_COUNT`、1 秒ごと 600 回）は、headless のときだけ 86400 回にする。
- `setBlocked`（`:292-298`。herdr への合図）を消す。

### 6. 文書 — `docs/for-sai-developers.md`
- C2・C3 を書き足す。statusLine・cc-rate-limits・承認のフックの記述を消す。

## 共有契約（他の作業と共有する取り決め。worker は変えない）

**C1 headless の書き出し**（U01 が書く。この作業は読むだけ）
- 擬似 pane の id は `hl-` で始まる。ターンのプロセスには `HERDR_PANE_ID=<id>` と `SAI_AGENT_TRANSPORT=headless` が付く。

**C2 送信・中断の CLI**（U03 が呼ぶ）
- `sai-link run send <run_id> [--text <文>] --json`（`--text` が無ければ標準入力）→ `{"run_id": <int>, "pane_id": "<id>", "status": "sent"}`。空の文は 2、実行や送り先が無いときは 3、終了している・質問待ちのときは 4。
- `sai-link run interrupt <run_id> --json` → `{"run_id": <int>, "status": "interrupted" | "still_working", "agent_status": "<状態>"}`。見つからないときは 3、終了しているときは 4。

**C3 フック・拡張と Web の約束**（U03・U10 が Web 側を作る）
- Web の場所: `SAI_WEB_URL` → `http://127.0.0.1:${SAI_WEB_PORT:-8200}`。
- 合言葉: `SAI_WEB_INTERNAL_TOKEN` があれば `Authorization: Bearer <値>` を付ける（無ければ付けない。単独モードは今のまま 127.0.0.1 からの要求として受ける）。
- 使う住所: `POST /api/register`・`POST /api/questions`（応答 `{"ok":true,"id":"<qid>"}`）・`GET /api/questions/{qid}`・`POST /api/questions/{qid}/resolved`。`POST /api/approvals` は使わない（U03 が消す）。

## ファイルの所有権
- 書いてよい: `crates/sai-link-engine/src/agent_api.rs`・`cc_command.rs`（`cc_hooks_json`・`cc_settings_json` とそのテスト）・`phone_export.rs`（statusline の部分と `pane_id_from_session_dir` の公開）・`crates/sai-link-cli/src/cli.rs`・`cmd_run.rs`・`main.rs`・`crates/sai-link-cli/tests/`（新しいテスト）・`apps/web/hooks/cc_question.py`・`cc_register.py`・`cc_permission.py`（消す）・`apps/web/src/sai_web/extensions/ask_user.ts`・`apps/web/tests/test_cc_question_hook.py`（新規）・`apps/web/tests/test_askuser.js`・`docs/for-sai-developers.md`
- 読むだけ: `crates/sai-link-engine/src/headless.rs`・`herdr.rs`・`orchestrator.rs`
- 触らない: `apps/web/src/sai_web/*.py`（U03 以降が書く）・`apps/web/src/sai_web/static/`

## 完了条件
- [ ] AC1: `sai-link run send` と `run interrupt` の結合テスト（一時フォルダのプロジェクト・偽のエージェント）: 動いている実行に `run send` すると次のターンが走る（`stream.jsonl` に `sai_turn` の行が増える）。`run interrupt` で長く眠る偽のエージェントが止まる。無い実行は終了コード 3。
- [ ] AC2: `sai-link schema --json` に `run send` と `run interrupt` が `web: true` で出る。`cc-statusline` が出ない。
- [ ] AC3: `apps/web/tests/test_cc_question_hook.py`: 標準ライブラリの `http.server` で偽の Web を立て、`cc_question.py` を走らせると、登録 → 答え待ち → 答えが入った `permissionDecisionReason` の JSON を出す。`SAI_WEB_INTERNAL_TOKEN` を付けると `Authorization` ヘッダが届く。`SAI_WEB_URL` の値に送る。
- [ ] AC4: `cc_hooks_json` の `AskUserQuestion` のフックに `"timeout": 86400` が入り、`PermissionRequest` が無い。`cc_settings_json` に `statusLine` が無い（Rust の単体テスト）。
- [ ] AC5: 実物の確認（費用の小さい model で 1 回）: headless の cc に「AskUserQuestion で好きな色を聞け」と頼み、偽の Web（AC3 と同じ）で答えると、cc がその答えを使って返事をする。pi も同様に `ask_user` で確かめる。結果を作業報告書に書く。
- [ ] AC6: 既存のテストがすべて通る（Rust・Web の JS と Python。消したテストは作業報告書に名前を書く）。

## 検証
```bash
cd ~/workdir/Sai/sai-tree
env -u SAI_PROJECT_ROOT -u SAI_PARENT_RUN_ID -u SAI_BLOCK_NAME \
  perl -e 'alarm 1800; exec @ARGV' cargo test --workspace
for f in apps/web/tests/test_*.js; do node "$f" || exit 1; done
for f in apps/web/tests/test_*.py; do PYTHONPATH=apps/web/src apps/web/.venv/bin/python "$f" || exit 1; done
```

## 決定事項と理由
- cc の質問は「フックが答えを待ち、拒否の理由の文で答えを返す」 — 理由: headless の `claude -p` には端末の選択画面が無い。フックは拒否の理由をモデルに返せる（公式の PreToolUse の出力）。/ 採用しなかった案: `--permission-prompt-tool` の MCP を作る（MCP サーバを 1 つ増やすことになる）、`updatedInput` で答えを入れる（動くかが文書から確かめられない）。
- 送信の口は実行（run）単位の CLI にする — 理由: Web は CLI だけを呼ぶ約束。
- statusLine と承認を消す — 理由: headless では呼ばれない。費用は U01 が `result` 行から書き、5 時間枠の表示は要件で削る。

## 要確認
- `claude -p` で `AskUserQuestion` が道具として出るか。出ない（モデルが質問できない）場合は、作業報告書に書き、`docs/for-sai-developers.md` に「headless の cc では質問できない」と書く（その場合も他の完了条件は満たす）。
