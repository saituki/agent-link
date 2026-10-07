# 実装計画書 U03: Web のサーバから herdr を外し、headless の実行と会話できるようにする

## 目的
`apps/web` のサーバ側から herdr の呼び出しをすべて消し、headless の実行（U01 の擬似 pane）について次を返せるようにする。
- 実行の状態（`working` / `idle`）と、生成中の文（逐次表示用）と、直前のターンの失敗の理由。
- 続きの文の送信と、中断（U02 の CLI を呼ぶ）。
- 質問カードへの回答（端末へのキー送信をせず、記録するだけ。フックと拡張が取りに来る）。
- プッシュ通知（「質問している」「長い作業が終わった」）を、herdr ではなく `state.json` と質問の登録から出す。

承認（PermissionRequest）の仕組みは丸ごと消す。

## 背景
- sai-tree は sai-moon を複製した Linux の多人数サーバ版で、エージェントはすべて headless で動く。herdr は無い。
- headless の実行の状態は、engine が `<project_root>/.sai/sai-engine/headless/<id>/state.json` に書く（共有契約 C1）。実行の書き出しの `herdr_pane_id` が `hl-` で始まる。
- 会話の本文（確定したメッセージ・道具の呼び出し）は、今と同じく `transcript_path` の jsonl から読む（cc は `claude -p` でも `~/.claude/projects/` に記録を書く。pi は `--session-dir` の中に書く）。生成中の文だけを `state.json` の `partial` から足す。
- Web 側の herdr の呼び出しの全一覧・承認と質問の今の仕組み・消すテストは、調査 E3（`docs/plan/research/E3-mac-only-removal.md` の §5）にある。主な所:
  - `herdr.py`（全部）・`config.py:43-51`（`herdr_bin`）・`watch.py:77-78`・`runs.py:244-316`（`open_question_session_ids`・`pane_for_run`・`live_status`）・`main.py:33`・`:206,214`・`:490-531`（`/send`）・`:116-124`（`watch_loop` の起動）・`:372-440`（承認の API）・`approvals.py`（全部）・`questions.py` の画面読みとキー送信（`refresh_question_state` `:107`・`_question_open` `:376`・`_send_answers_cc` `:521` ほか）・`store.py` の承認の関数（`:271-380`）・`home.py:132-`（`_home_approvals`）と `"approvals"` キー（`:275`）・`webcli.py:366-380,569-579`（`approvals` / `approve`）。
- `watch.notify_heading` は承認と通知の両方が使う（`watch.py:45-50`）。

## 前提（着手時点のコードの状態）
- U01・U02 が終わっている。
  - U02: `sai-link run send <run_id> [--text]` と `sai-link run interrupt <run_id>`（C2）。cc の質問のフックは答えを Web に取りに来る。`POST /api/approvals` はもう呼ばれない。
- 会話の項目の住所: `main.py:217-245` `api_run_items`（`{items, next_cursor}` を返す）。
- sai-link の CLI を呼ぶ共通の口: `control._phone_json(args, root=…, stdin_text=…)`（`control.py:252` 付近）。終了コードの読み替えは `cli.py` の 1 か所。
- 質問の回答: `questions.answer_question`（`questions.py:219-249`）が `agent` で分け、cc は `_send_answers_cc`、pi は記録だけ（`questions.py:349-353`）。

## 変更内容

### 1. herdr と承認を消す
- 消すファイル: `apps/web/src/sai_web/herdr.py`・`approvals.py`・`apps/web/tests/test_herdr_submit.py`。
- `config.py:43-51` の `herdr_bin` を消す。`main.py:33` の import を消す。
- `main.py:372-440` の承認の API 3 本を消す。`store.py` の承認の関数（`:271-380`）を消す（表 `approvals` は `CREATE TABLE IF NOT EXISTS` のまま残してよい。既存の DB を壊さないため）。`home.py` の `_home_approvals` と `"approvals"` キーを消す。`webcli.py` の `approvals` / `approve` を消す。
- `watch.notify_heading`（`watch.py:45-50`）を `notify.py` に移し、呼び出し元を直す。
- テストのうち、承認と herdr の画面の偽装に頼る物を消す（一覧は調査 E3 §5.2・§5.5。`test_questions.py` の承認の 4 本と画面読みの 6 本、`test_home.py`・`test_webcli.py`・`test_notify.py` の承認の部分、`test_runs.py:47-48,183-206`）。pi の質問のテストは残す。消したテストの名前は作業報告書に書く。

### 2. headless の状態を読む — `apps/web/src/sai_web/runs.py`
- `def headless_state(run: dict) -> dict | None`:
  - `run["herdr_pane_id"]` が `hl-` で始まらなければ None。
  - パス `Path(run["project_root"]) / ".sai/sai-engine/headless" / <id> / "state.json"`。`<id>` は `^hl-[0-9a-f]{12}$` でなければ None。
  - 読めない・JSON でない・`state` が `working` / `idle` / `stopped` 以外なら None。
  - 返す: `{"state", "partial", "error", "turn"}`（`partial`・`error` は文字列に直し、`partial` は末尾 20000 文字まで）。
- `pane_for_run` を消し、`live_status(run)`（`runs.py:304`）を `headless_state` から作る: `working` → `"working"`、`idle` → `"idle"`、`stopped`・None → None。質問が open の実行（`store.list_open_questions` にその session がある）は `"blocked"`。**`run["status"]` が `running` でなくても返す**（メインの実行は `doc write` の後も review のまま会話を続けられるため）。
- `run_to_api`・`build_day` の引数の `agents`・`panes` を消す。`open_question_session_ids`（`runs.py:244`）は画面読みをやめ、`store` の open の質問から作る。
- `run_to_api`（`runs.py:322`）に `"headless": bool`（`herdr_pane_id` が `hl-` で始まるか）を足す。
- `sessions.py:41` `_herdr_status_to_api` は名前だけ残してよい（使われなくなれば消す）。

### 3. 会話の項目に生成中の文を足す — `main.py:217-245` `api_run_items`
- 返す形を `{items, next_cursor, live, partial, error}` にする（`live`・`partial`・`error` は headless の実行のときだけ値が入り、それ以外は `null`・`""`・`""`）。今の画面は余分な項目を無視するので壊れない。
- `transcript_path` がまだ無い・ファイルが無いときも、`live`・`partial` は返す（起動直後に逐次表示を始められるように）。

### 4. 送信と中断
- `api_send_text`（`main.py:490-531`）: herdr を使わずに `control.run_send(project, run_id, text)` を呼ぶ。中身は `sai-link run send <run_id> --json`（本文は標準入力）。失敗の読み替えは `cli.py` のまま（CLI の 4 → 409、3 → 404）。
- 新しい住所 `POST /api/runs/{project_name}/{run_id}/interrupt`（`Depends(require_token)`）: `_find_run` で実行を確かめ（無ければ 404。CLI を呼ばない）、`control.run_interrupt(project, run_id)` → `sai-link run interrupt <run_id> --json` の結果をそのまま返す。
- `control.py` に `run_send(name, run_id, text)` と `run_interrupt(name, run_id)` を足す（`find_project` で root を引き、`_phone_json` で呼ぶ）。
- `webcli.py` の `send`（`:383-`）は同じ住所を呼ぶので、そのまま動くことを確かめる。

### 5. 質問 — `apps/web/src/sai_web/questions.py`
- `answer_question`（`:219`）: `agent` に関係なく「記録するだけ」（`store.mark_answered`）にする。cc のフック（U02）と pi の拡張が `GET /api/questions/{qid}` で取りに来る。
- `refresh_question_state`（`:116`）: 画面を読まない。open の質問は open のまま返す（pi の `resolved` の通知と期限切れ `expire_stale_questions` だけで状態が変わる）。cc の質問にも pi と同じ期限（`_PI_EXPIRE_SEC`）を当てない（cc のフックは 24 時間待つので、期限は 24 時間にする）。
- `_send_answers_cc` 以下のキー送信・画面読みの関数（約 250 行）を消す。
- `store.py` の `expire_open_for_pane`（`:241`）は使われなくなれば消す。`pane_id` の列は残す（空文字でも動く）。

### 6. 通知 — `apps/web/src/sai_web/watch.py`
- herdr の一覧を読む代わりに、5 秒ごとに「今の実行の書き出し（`runs.load_runs`）のうち `headless` の物」の `headless_state` を読む。
  - `working` が 3 分以上続いたあと `idle` になったら「作業が終わりました」を通知する（今の `notify_status_change` の文と同じ）。
  - 質問の通知は、`questions.register_question` で登録したときに出す（今そうしていなければ足す）。
- 起動（`main.py:116-124`）と `SAI_WEB_NO_WATCH=1` の扱いは今のまま。
- `test_notify.py` を新しい形に直す（偽の `state.json` で working → idle を作る）。

### 7. 文書
- `docs/integration.md`: `GET /api/runs/{p}/{id}/items` の新しい項目、`POST /api/runs/{p}/{id}/interrupt` を足す。承認の住所と herdr の記述を消す（`:103,278,280-283,301,398`）。
- `docs/for-sai-developers.md`: C1 の `state.json` を「Web が読む書き出し」として足す。`herdr_pane_label` の約束（`:130`）を消す。

## 共有契約（他の作業と共有する取り決め。worker は変えない）

**C1 headless の書き出し**（U01 が書く。全文）
- 擬似 pane の id: `hl-` + 12 桁の 16 進。実行の書き出しの `herdr_pane_id` に入る。
- `<project_root>/.sai/sai-engine/headless/<id>/state.json`:
  `{"pane_id","runner":"cc|pi","state":"working|idle|stopped","turn":<int>,"partial":"<生成中の文>","error":"<理由 1 行>","updated_at":"RFC3339"}`。Web が読んでよいのはこのファイルだけ。

**C2 送信・中断の CLI**（U02 が作る。全文）
- `sai-link run send <run_id> [--text <文>] --json`（`--text` が無ければ標準入力）→ `{"run_id","pane_id","status":"sent"}`。空 2、無い 3、終了・質問待ち 4。
- `sai-link run interrupt <run_id> --json` → `{"run_id","status":"interrupted"|"still_working","agent_status"}`。無い 3、終了 4。

**C6 チャットの API**（U04 が使う）
- `GET /api/runs/{p}/{id}/items?after=<cursor>` → `{"items":[…今の形…],"next_cursor":"…","live":"working"|"idle"|"blocked"|null,"partial":"…","error":"…"}`。
- `GET /api/runs/{p}/{id}` と一覧の実行に `"headless": true|false` と `"live"`。`live` は、`status` が `running` でなくても `state.json` が `stopped` でなければ値が入る。
- `POST /api/runs/{p}/{id}/send` `{"text"}` → `{"ok":true}`。
- `POST /api/runs/{p}/{id}/interrupt` → C2 の `run interrupt` の JSON。
- 承認の住所（`/api/approvals*`）は無い。`GET /api/home` に `approvals` は無い。

## ファイルの所有権
- 書いてよい: `apps/web/src/sai_web/` の `herdr.py`（消す）・`approvals.py`（消す）・`runs.py`・`main.py`・`control.py`（2 つの関数だけ）・`questions.py`・`store.py`（承認の関数を消す・`expire_open_for_pane`）・`home.py`・`watch.py`・`notify.py`・`webcli.py`・`config.py`・`sessions.py`・`apps/web/tests/` の Python のテスト（直す・消す・`test_headless_runs.py` を新規）・`docs/integration.md`・`docs/for-sai-developers.md`
- 読むだけ: `apps/web/src/sai_web/registry.py`・`cli.py`
- 触らない: `apps/web/src/sai_web/static/`・`apps/web/tests/*.js`（U04 が書く）・`crates/`

## 完了条件
- [ ] AC1: `apps/web/src/sai_web/` に `herdr` の文字が残らない（`grep -rn herdr apps/web/src/sai_web --include=*.py` が `herdr_pane_id` の読み出しだけ）。
- [ ] AC2: `apps/web/tests/test_headless_runs.py`（一時フォルダに実行の書き出しと `state.json` を置く）: `headless_state` が `state.json` を読む。`stopped` は `live` が None。`status` が `completed` でも `idle` の headless 実行は `live == "idle"`。`/items` が `partial` を返す。`transcript_path` が無くても `live` と `partial` を返す。
- [ ] AC3: 偽の CLI（`FakeCli`。既存のテストの書き方に倣う）で、`/send` が `sai-link run send <id> --json` を本文を標準入力にして呼ぶ。`/interrupt` は無い実行で CLI を呼ばずに 404。
- [ ] AC4: cc の質問に答えると、キーを送らずに `answered` になり、`GET /api/questions/{qid}` が答えを返す。
- [ ] AC5: 通知のテスト: 偽の `state.json` が 3 分以上 `working` のあと `idle` になると、通知が 1 回出る。
- [ ] AC6: Web の Python のテストがすべて通る（JS のテストは U04 の範囲。この作業で JS のテストが落ちるなら、落ちる理由（API の形の変更）を作業報告書に書く）。

## 検証
```bash
cd ~/workdir/Sai/sai-tree
for f in apps/web/tests/test_*.py; do PYTHONPATH=apps/web/src apps/web/.venv/bin/python "$f" || exit 1; done
```

## 決定事項と理由
- 確定した会話は今の jsonl から、生成中の文だけを `state.json` から読む — 理由: 会話の項目への変換（`sessions.py`）を作り直さずに済む。engine が生成中の文をまとめておくので、Web は stream を読み解かない。
- 配信はポーリングのまま — 理由: SSE・WebSocket を足さない（`main.py:8-15` の方針）。画面が 1 秒ごとに `/items` を読めば、逐次表示になる（U04）。
- 承認を丸ごと消す — 理由: headless の cc は許可の一覧で動き、承認が来ない。承認の答えは端末へのキー送信でしか届かない作りで、端末が無い（調査 E3 §5.2）。
- 通知は `state.json` を見張る形にする — 理由: 今の通知の文と間隔を変えずに、herdr の代わりの元を読むだけで済む。

## 要確認
- なし。
