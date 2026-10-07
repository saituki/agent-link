---
block: explore
run: 446
description: 調査報告書 E3: 遠隔ブラウザ・launchd・キーチェーン・cc statusline・Web 側の herdr を削る範囲
---

# 調査報告 E3: Mac 専用の機能を削る範囲（apps/web 側と周辺）

対象: `~/workdir/Sai/sai-moon`（書き換えていない）。行番号は 1 始まり。
事実はコードで確かめたもの。推測は「（推測）」、未確定は「（要確認）」を付けた。

## 0. 結論（先に読む）

| 項目 | 削れるか | 一緒に直す所 | 壊れる他の機能 |
|---|---|---|---|
| (1) 遠隔ブラウザ | そのまま削れる | `papers.py` の 1 関数・`papers.js` の 2 ボタン・index.html・sw.js・文書 | なし（papers の `translate-pdf` のテストが `test_rbrowser.py` に同居しているので先に移す） |
| (2) launchd | ひな形 1 本と文書だけ | README・ARCHITECTURE・AGENTS・docs/switch-over.md・integration.md の言い回し | なし。コードは launchd を呼ばない |
| (3) キーチェーン | 削れる。`paths.rs` の約 130 行とテスト | `sai-paper-core` だけ | なし。Linux ではもともと動かない（`target_os` で `None`） |
| (4) cc の 5 時間枠と cc-cost | 5 時間枠は削れる。cc-cost は代わりが要る | Rust の engine（`phone_export.rs`・`cc_command.rs`・`cli.rs`）・`usage.py`・`usage.js` | cc の run の費用表示（`stats.cost_source == "cc"`）が概算に落ちる |
| (5) Web 側の herdr | 全部削れるが、**画面と API の動作が変わる**（下記） | `herdr.py`・`approvals.py`・`questions.py` の cc 経路・`watch.py`・`runs.py`・`main.py`・`store.py`・画面・フック・テスト | 実行中の run への文の送信（`/send`）・質問に答える cc 経路・状態のランプ・通知（working → blocked） |

headless の cc では承認が来ないので、承認の画面ごと消せる（§5.2）。質問は **pi の経路だけが herdr 無しで動く**（§5.3）。cc の質問は headless では新しい経路が要る。

---

## 1. 遠隔ブラウザ（Mac の Chrome を Web から操作）

### 1.1 定義の場所
- サーバ: `apps/web/src/sai_web/rbrowser.py`（493 行）。Chrome を `--remote-debugging-pipe` で起動（`:254` `_start`）、画面は JPEG（`:413` `screenshot`）、入力は `:421` `send_input`、DOI 検査は `:96` `check_doi`、ダウンロード一覧 `:456` `list_downloads`、ファイル名の検査 `:482` `download_path`。既定の Chrome は Mac の `.app`（`:48` `DEFAULT_CHROME`）。
- 画面: `static/rbrowser.js`（428 行・`window.SaiRoutes` に登録 `:408`・住所 `#/papers/browser` `:11`）、`static/rbrowser.css`（25 行）。
- API（`main.py:1024-1073`）: `POST …/papers/browser/open`・`GET …/browser/state`・`GET …/browser/screen`・`POST …/browser/input`・`POST …/browser/import`（202）・`DELETE …/papers/browser`。
- 取り込み: `papers.py:1495` `request_browser_import`（`rbrowser.download_path` を呼ぶ `:1509`）。

### 1.2 呼び出し元（全部）
| 場所 | 内容 |
|---|---|
| `main.py:40` | `rbrowser` の import |
| `main.py:131` | `_shutdown` で `rbrowser.close()` |
| `main.py:1033,1039,1046,1057,1072` | 上の 6 本の API |
| `papers.py:37` | `from sai_web import cli, control, notify, rbrowser` |
| `papers.py:1509` | `rbrowser.download_path` |
| `rbrowser.py:484` | 逆向きに `papers` を関数の中で import（循環を避けるため） |
| `static/papers.js:2705-2716` | Download の案内に「Mac のブラウザで開く」ボタンと説明文。`typeof rbOpenHash === "function"` で守られているので rbrowser.js が無くても壊れない |
| `static/papers.js:2898-2904` | 論文の詳細（PDF が無いとき）の「Mac のブラウザで開く」ボタン。同じく `typeof` で守られている |
| `static/index.html:26` | `<link … /rbrowser.css?v=89>` |
| `static/index.html:511-512` | `<section id="view-papers-browser">`（中は `#rb-root`） |
| `static/index.html:662` | `<script src="/rbrowser.js?v=89">` |
| `static/sw.js:56`（コメント）・`:91`（`ASSETS` の `"/rbrowser.js"`・`"/rbrowser.css"`） | 配る物の一覧。`CACHE = "sai-web-v89"`（`:90`）と index.html の `?v=89` は一緒に上げる決まり（AGENTS.md「画面のコード」） |
| `static/paperimport.js` | rbrowser の API は呼ばない。ただし取り込み対象 `{log_id, position}` / `{import_id}` / `{cite_paper_id, ref_no}` / `{paper_id}` の組み立て（`:217-272`）を rbrowser.js が同じ形で持つ（`rbrowser.js:15` `RB_NUM_KEYS`）。paperimport.js 側は消さない |
| Rust・他のテスト | なし（grep で確かめた） |

### 1.3 一緒に消すテスト
- `apps/web/tests/test_rbrowser.js`（70 行）: `rbrowser.js` を `require` する。
- `apps/web/tests/test_rbrowser.py`（277 行）: `rbrowser.py` の検査・偽 Chrome（`test_fake_chrome` `:207`）・`request_browser_import`（`test_import` `:107`）・ルートの登録順（`test_routes` `:251`）。
- **注意**: 同じファイルに `test_translate_pdf`（`:143`）があり、これは遠隔ブラウザと無関係（`papers.request_translate_pdf`）。`test_routes` の最後の 1 行（`:258` `{paper_id:int}/translate-pdf` が登録されている）も同じ。消す前に別ファイル（例 `test_papers_*.py`）へ移さないと、PDF 翻訳のテストが無くなる。

### 1.4 文書
- `ARCHITECTURE.md:186`（`rbrowser.py` の行）・`:234`（`rbrowser.js` を含む行）。
- `README.md:33`（「macOS … Chrome の遠隔操作」）・`:173-174`（`SAI_WEB_CHROME`・`SAI_WEB_BROWSER_HEADED`）。
- `docs/integration.md:216-220`（登録順の注意に `browser/*`、Chrome の説明）・`:389-390`（環境変数 2 つ）。
- `docs/switch-over.md:177,186`（`browser-profile`・`browser-downloads` の言及。`~/.sai/web` の移行の例）。
- `AGENTS.md`・`docs/for-sai-developers.md` に言及なし。

### 1.5 消すと壊れるもの
- なし。Papers の「手動で取り込む」（`pimOpenManual`・`POST …/papers/manual/*`）は別経路で残る。
- 置き場 `$SAI_HOME/web/browser-profile/`・`browser-downloads/` は使われなくなる（コードに他の参照なし）。

---

## 2. launchd

### 2.1 定義の場所
- `apps/web/launchd/com.sai.web.plist.template` の 1 ファイルだけ（ラベル `com.sai.web`・`sai-web serve`・`RunAtLoad`・`KeepAlive`・ログ `__SAI_HOME__/web/sai-web.log`）。

### 2.2 呼び出し元
- コード（Python・Rust・JS）は launchd を呼ばない。grep で `launchd`・`launchctl` が出るのは次の**文書とコメントだけ**。
  - `README.md:64-70`（plist のコピーと `launchctl load/unload`）・`:86`（「launchd で立ち上げているなら不要」）
  - `ARCHITECTURE.md:154`（`web/sai-web.log`「launchd で動かしたとき」）・`:290`（`launchctl load` を実行しない）・`:314`（フォルダ図の `launchd/`）
  - `AGENTS.md:23`（`launchd/` の行）・`:62`・`:138`（`launchctl load / unload` を実行しない）・`:143`（`~/Library/LaunchAgents/`）
  - `docs/switch-over.md:15,108-117,276-303,390-402,488-492`（段 5 の plist の入れ替えほか）
  - `docs/integration.md:401,409`・`docs/for-sai-developers.md:745,765`（PATH が短い話）
  - `apps/web/src/sai_web/config.py:44`（コメント「launchd や nohup で PATH が短いと herdr が見つからず」）
- CLI（`sai-web`）に launchd の命令は無い。`webcli.py` の `serve` は普通に uvicorn を起こすだけ（要確認: Linux では systemd の unit が別に要る）。

### 2.3 テスト
- なし。

### 2.4 消すと壊れるもの
- なし。サーバ版では systemd 等に置き換える（文書の追記が要る・今回の範囲外）。

---

## 3. paper.env のキーチェーン記法

### 3.1 定義の場所（`crates/sai-paper-core/src/paths.rs`）
- `:108` `load_project_dotenv`: `:114-117` で `paper.env` に対し `load_keychain_env(&base)` を呼んでから `dotenvy::from_path(base)`。
- `:128` `load_keychain_env`（行を見つけて環境変数に入れる）・`:153` `struct KeychainLine`・`:162` `keychain_lines`（`export NAME=$(security find-generic-password -s … -a … -w)` を取り出す）・`:215` `shell_words`（引用符を外す）・`:248` `keychain_value`（`/usr/bin/security` を直接呼ぶ `:252`。`!cfg!(target_os = "macos")` なら `None`）。
- テスト: `:657-695` `mod keychain_tests`（2 本）。

### 3.2 呼び出し元
- `load_project_dotenv` を呼ぶのは `crates/sai-paper-cli/src/main.rs:840` と `crates/sai-sci-cli/src/main.rs:255` の 2 か所。`load_keychain_env` / `keychain_*` / `shell_words` は `paths.rs` の中だけ（grep で確認）。
- `crates/sai-paper-cli/src/main.rs:1109` のメッセージ「`.env か ~/.sai-moon/paper.env に置いてください`」は旧パスの文言（古い。キーチェーンとは無関係）。

### 3.3 消し方
- `paths.rs:115` の 1 行と `:120-150`・`:151-245` の関数・構造体、`:247-266` の `keychain_value`、`:656-696` のテストを消す。`dotenvy::from_read_override` を使うのはこの関数だけ（他に使われていなければ import も整理。要確認）。
- `paper.env` 自体（`~/.sai/paper.env` の通常の `KEY=value`）は残す。`ARCHITECTURE.md:147`・`README.md:158`・`docs/integration.md:74`・`docs/for-sai-developers.md:244`・`AGENTS.md:89` の記述は変えなくてよい。
- 文書に「キーチェーン」の説明は無い（grep で `paths.rs` 以外に出ない。`sai-sci/exec/dok/SETUP.md:320` は docker login の話で別件）。

### 3.4 壊れるもの
- なし。Mac の利用者が `.zshrc` と同じ書き方を `paper.env` に書いていると、キーが入らなくなる（S2 が 429 になる・`paths.rs:122-125` のコメントの症状）。サーバ版は Linux 前提なので影響しない。

---

## 4. cc の statusline（5 時間枠）と cc-cost

### 4.1 定義の場所
| 物 | 場所 |
|---|---|
| CLI（隠し命令） | `crates/sai-link-cli/src/cli.rs:141-147` `Cmd::CcStatusline`、`:1993` `handle_cc_statusline`、`main.rs:99`（dispatch）・`main.rs:10`（先頭のコメント） |
| 本体 | `crates/sai-link-engine/src/phone_export.rs:897` `handle_cc_statusline`、`:907` `handle_cc_statusline_in`（5 時間枠の書き出し `:925-935`・費用の書き出し `:938-956`） |
| 出力先 | `:873` `cc_cost_dir`（`~/.sai/cc-cost`）・`:881` `cc_rate_limits_dir`・`:886` `cc_rate_limits_path`（`<SAI_HOME>/web/cc-rate-limits.json`） |
| 読み取り（費用） | `phone_export.rs:371` `read_cc_cost`（`~/.sai/cc-cost/<session_id>.json` の `total_cost_usd`） |
| `.claude/settings.json` に statusLine を書く | `crates/sai-link-engine/src/cc_command.rs:344-347`（`cc_settings_json` の `statusLine`）。`sai-link init`（`init.rs:196`）が書く |
| 補助 | `phone_export.rs:608` `unix_now_f64`・`:618` `write_json_atomic`（この 2 つは statusline 専用。他に使われていないか要確認） |
| Web 側（5 時間枠） | `apps/web/src/sai_web/usage.py:35` `cc_limits_path`・`:99` `cc_limits`・`:434`（`summary()` の `"cc": cc_limits()`）・冒頭の説明 `:5` |
| 画面 | `static/usage.js:137-168` `usageCcCard`（「5時間枠」の棒）・`:275`（`usageData.cc`）・補助の `:36`（`usageSavedAgo`）・`:54`（`usageResetText` の「回復済み（次に cc が動くと更新）」） |

### 4.2 呼び出し元と、5 時間枠・cc-cost の用途
- **5 時間枠**: 読むのは `usage.py:99` の `cc_limits` だけ → Usage 画面の「Claude Code（Team）」カード 1 枚。他に使う所は無い。**削れる**。`summary()` の `cc` キーと `usageCcCard` と `test_usage.py:269-` を消す。
- **cc-cost**（`total_cost_usd`）: 読むのは `phone_export.rs:478` の 1 か所。cc の run の `cost_source` と `cost_usd.total` を決める:
  - `cost_source == "cc"` かつ `cost_is_estimate == false`: `cc-cost/<session_id>.json` があるとき（cc 自身の金額）。
  - なければ `cost_source == "estimate"`: transcript の jsonl を `collect_metrics_from_file_cc` で集計してモデルの公式単価から概算（`usage_metrics.rs:277` `estimate_cc_cost`）。
  - この値が出る先（`apps/web` が JSON で読む）: `runs.py:410-429`（日ごとの合計 `cost_usd`・`cost_is_estimate`）→ `app.js:400`（日の一覧に「約 $1.23」）、`stats.js:251-256`（コストの大きな数字と「概算 / cc 集計 / pi 集計」の注記）、`stats.js:402-405`（`cost_source === "cc"` のとき「内訳は概算」）。`docs/for-sai-developers.md:128` が `stats.cost_usd.total`・`cost_is_estimate` を約束している。
  - `sai-sci/blocks/kaizen.md:69` が「コスト: `~/.sai/cc-cost`・`sai-link usage`（使えるときだけ）」と案内している。
- テスト（Rust）: `phone_export.rs:1148`（`cc_statusline_writes_cost_file`）・`:1177`（`cc_statusline_swallows_bad_input`）・`:1212`（`cc_statusline_writes_rate_limits_without_cost`）・`:1369-1372`（`cc_rate_limits_dir/path`）、`cc_command.rs:757-758`（`statusLine` の assert）。`crates/sai-link-cli/tests/cli_test.rs` に statusline の言及は無い。
- 文書: `AGENTS.md:89-92`・`ARCHITECTURE.md:150`・`README.md:159`・`docs/integration.md:50,76`・`docs/switch-over.md:177,186`・`sai-sci/blocks/kaizen.md:69`・`sai-link-engine/src/init.rs:20`（コメント）・`orchestrator.rs:788`・`cc_command.rs:16,327` のコメント。

### 4.3 headless の `claude -p` で代わりになるか（判断材料）
- 今は cc を **対話の pane** で動かし、費用は statusLine の標準入力の JSON（`cost.total_cost_usd`）から取っている（`phone_export.rs:938-946`）。headless では statusLine は呼ばれない（推測）ので、`cc-cost/` は書かれず、**今のままだと全 cc の run が概算に落ちる**。
- `claude -p --output-format stream-json` の最後の `result` 行には `total_cost_usd`・`usage`（`input_tokens`・`output_tokens`・`cache_read_input_tokens`・`cache_creation_input_tokens`）・`session_id`・`duration_ms`・`num_turns` が入る（Claude Code の仕様についての知識。**この repo のコードでは確かめていない**。実機で `result` 行を 1 回確かめること（要確認））。repo 内に stream-json を読むコードは無い（`grep 'stream-json\|total_cost_usd'` は `phone_export.rs` と `store.rs:245`（集計用の別物）だけ）。
- 代わりになる見込み: `total_cost_usd` は statusLine の `cost.total_cost_usd` と同じ意味の累計（推測）なので、engine が子プロセスの標準出力から `result` 行を取り、`cc-cost/<session_id>.json` と同じ形（`{session_id, total_cost_usd, updated_at}`）で書けば、`read_cc_cost`（`phone_export.rs:371`）以降は変えずに済む。`usage` は `usage_metrics.rs` の `CacheUsageTotals` に直接入れられる（transcript の jsonl が headless でも `~/.claude/projects/` に残るなら、今の集計もそのまま動く。要確認）。
- 5 時間枠（`rate_limits`）は stream-json の `result` 行には入らない（推測）。5 時間枠の表示は削る方針で矛盾しない。
- 判断: (a) cc-cost の書き手を `cc-statusline` から「engine が `result` 行から書く」に替える、(b) 書き手をやめて常に transcript の概算にする（`read_cc_cost` と `cost_source: "cc"` を消す）。(b) は `apps/web` の `stats.js:255,402` の分岐が 1 つ死ぬだけで済む。(a) なら費用が正確。どちらでも `apps/web` の JSON の項目は変わらない。

---

## 5. Web 側の herdr

### 5.1 herdr を呼ぶ・前提にする場所（全部）
| 場所 | 内容 |
|---|---|
| `herdr.py`（278 行） | `herdr` コマンドの呼び出し: `list_agents`（`:42`）・`list_panes`（`:63`）・`pane_gone`（`:83`）・`get_agent`（`:96`）・`prompt_agent`（`:136`・送信と確定キー）・`send_keys`（`:213`）・`read_pane`（`:229`）・`send_text`（`:263`） |
| `config.py:43-51` | `herdr_bin`（`HERDR_BIN`・`HERDR_BIN_PATH`・`~/.local/bin/herdr`） |
| `approvals.py` | `herdr.pane_gone/read_pane/get_agent/send_keys/send_text`（`:144,148,154,162,220,241,250,254`） |
| `questions.py` | `herdr.pane_gone/read_pane/get_agent/send_keys/send_text`（`:96,142,147,158,299,318,321,370,416,429,447,454,472,477,527`） |
| `watch.py` | `herdr.list_agents/list_panes`（`:77-78`）で 2 秒おきに状態を見て通知（working → blocked、3 分以上の作業のあと idle/done） |
| `runs.py:262-316` | `pane_for_run`（`herdr_pane_id` / `herdr_pane_label` で pane を探す）・`live_status`（`working/blocked/idle` を返す）。`:244-253` `open_question_session_ids` は `questions.refresh_question_state`（画面読み）を呼ぶ |
| `runs.py:326,343,441,463` | `run_to_api(…, agents, panes)`・`"live"`・`build_day(…, agents, panes)` |
| `sessions.py:41` | `_herdr_status_to_api`（`runs.live_status` が使う）。`sessions.py:9` に「pane read は使わない」とあるが関数名に herdr が残る |
| `main.py:33` | `herdr` の import |
| `main.py:206,214` | `/api/days/{date}`・`/api/runs/{project}/{run_id}` が `herdr.list_agents()/list_panes()` を渡す |
| `main.py:490-531` | `POST /api/runs/{project}/{run_id}/send`: `runs.pane_for_run` → `herdr.prompt_agent`。blocked 中は「質問への回答は選択肢ボタンから」と断る |
| `main.py:116-124` | 起動時に `watch_loop` を始める（`SAI_WEB_NO_WATCH=1` で止まる） |
| `main.py:372-440` | 承認の API 3 本（§5.2） |
| `main.py:281-340` | 質問の API: `POST /api/questions`（登録）・`POST /api/questions/{qid}/resolved`・`GET /api/questions…`・`POST /api/questions/{qid}/answer` |
| `store.py` | `registrations`（`pane_id` 列 `:25`）・`questions`（`:32`）・`approvals`（`:44-57`）の 3 表に `pane_id` NOT NULL。`expire_open_for_pane`（`:241`）は pane が消えたカードを片付ける |
| `hooks/cc_question.py` | `os.environ.get("HERDR_PANE_ID")` を `pane_id` として送る。送り先は **`http://127.0.0.1:8200` 固定**（`SAI_WEB_URL` を読まない・他の 2 本は読む） |
| `hooks/cc_register.py` | `HERDR_PANE_ID`（3 行で言及）を送る |
| `hooks/cc_permission.py` | 同上（`payload["pane_id"]`） |
| `extensions/ask_user.ts` | `:292-298` `setBlocked`（`herdr:blocked` イベント）・`:360` `paneId`（`HERDR_PANE_ID`）・`:519-520` `session_start` で `pane` が空なら `/api/register` を呼ばない |
| 画面 | `app.js:327-334`（`STATUS_LAMP` と `run.live` のランプ）・`app.js:545`（`run.live === "blocked"`）・`app.js:1625-1680`（`/send` を呼ぶ送信欄と送信待ち）。`linkview.js:748` のコメント（「pane が生きているため」）だけ言及 |
| Rust 側の約束 | `docs/for-sai-developers.md:130`（`herdr_pane_id` / `herdr_pane_label` を `apps/web` が読む）。書き出しは `phone_export.rs:57-59,561-578`（調査済みの範囲） |

### 5.2 承認（PermissionRequest）の今の仕組み
1. cc が承認画面を出す直前に `PermissionRequest` フック `apps/web/hooks/cc_permission.py` が呼ばれる。フックは**何も出力せず終了コード 0** で終わる（出力すると PC の承認画面が出なくなるため。`cc_permission.py` の冒頭 docstring）。stdin の JSON に `pane_id = $HERDR_PANE_ID` を足して `POST /api/approvals`（既定 `127.0.0.1:8200`、`SAI_WEB_URL` / `SAI_WEB_PORT` で変更）へ送る。
2. サーバ（`main.py:387` `api_register_approval`）は Mac 自身からだけ受け（`require_local`）、`approvals.register`（`approvals.py:90`）が `store.save_approval` で保存し、その run があれば「承認を求めています」をプッシュ通知する（`_notify_if_run` `:111`）。
3. 画面は `GET /api/approvals?session_id=`（`main.py:416`）を引く。そのたびに `approvals.refresh`（`approvals.py:134`）が **herdr で画面を読み**（`read_pane` 60 行）、「Do you want to proceed?」（`_APPROVAL_MARK` `:49`）がまだ出ているかで、PC で先に答えられたかを判定する。pane が消えていれば期限切れにする。
4. iPhone で答えると `POST /api/approvals/{aid}/answer`（`main.py:435`）→ `approvals.answer`（`approvals.py:184`）。画面を読み直して同じ承認か確かめ（`_matches_screen`）、`herdr agent send-keys <pane> 1|2|<No の番号>` で**番号キー**を送る。画面が閉じるのを最大 6 回（0.4 秒おき）見て確かめる。拒否に理由があれば `herdr pane send-text` で入れて `alt+enter` で確定する。
5. 画面側: `app.js:722-764`（読み込みとカード）・`:992-1150`（`approvalCard`・`submitApproval`）、`home.js:90,123-128,399`（待ちの一覧に「承認をしてほしいです」）、`style.css:902-906`。`sai-web approvals` / `approve`（`webcli.py:366-380,569-579`）が同じ API を呼ぶ。
6. cc の承認画面の文言（`Do you want to proceed?`・`1. Yes`・`4. No`）は実機の観察（cc 2.1.280）に固定されている（`approvals.py` の冒頭）。

**headless で画面ごと消せるかの判断材料**
- 事実: engine は cc を **既定で `--dangerously-skip-permissions`** で起動する（`crates/sai-link-engine/src/cc_command.rs:188-195`、既定の判定 `:231-236` `cc_skip_permissions`。`SAI_CC_SKIP_PERMISSIONS=0` のときだけ `--permission-mode acceptEdits`）。コメントに「無人実行のため既定で全許可」とある。つまり**今でも、sai-link が起こした cc の run では承認はほぼ出ない**。承認が出るのは `SAI_CC_SKIP_PERMISSIONS=0` にしたときと、利用者が手で起こした cc だけ。
- 事実: headless に herdr の pane は無い。承認の答えは「画面を読んでキーを送る」ことでしか届かない（フックは答えを返せない設計。`cc_permission.py` の docstring）ので、pane が無ければ `approvals.answer` は動かない（`approvals.py:196-198` で `pane_id` が空なら「端末に繋がりませんでした」）。
- 判断（推測）: `claude -p --dangerously-skip-permissions` では PermissionRequest は発生しない。したがって `approvals.py`・`hooks/cc_permission.py`・`store.py` の `approvals` 表と関数（`:271-380`）・`main.py:372-440`・`home.py:132-`（`_home_approvals`）と `"approvals"` キー（`:275`）・`home.js` の approval 分岐・`app.js` の承認カード・`webcli.py` の `approvals` / `approve`・`style.css:902-906`・`cc_command.rs:374-377` の `PermissionRequest` フック（と `hooks_dir_in` の 3 本のうち `cc_permission.py`・`cc_command.rs:400`・テスト `:676-713`）は**画面ごと消せる**。
- 要確認: `--dangerously-skip-permissions` でも確認が出る操作（保護されたパスなど）が cc の版によって残らないか。残るなら headless では自動で拒否される（推測）ので、承認の仕組みは要らないが、run が失敗する形になる。実機で 1 回確かめるとよい。
- 消すときに触る他の文書・ブロック: `sai-sci/blocks/professor.md:155`・`professor-backward.md:173`・`operator.md:37`・`kaizen.md:68`・`sai-sci/template/sci-cli.md:67`（`sai-web approvals` / `approve` の案内）、`docs/integration.md:103,280-283,301`、`README.md:235,272`、`ARCHITECTURE.md:40,111,152,182,188`、`AGENTS.md:48`。
- 消すテスト: `test_questions.py` のうち承認の 4 本（`test_approval_state_transitions` `:289`・`test_approval_resolved_elsewhere_and_mismatch` `:349`・`test_approval_hook_is_silent` `:390`・`test_gone_pane_expires_cards` の承認の部分 `:444-460`。同じファイルの質問のテストは別。ファイルの import `approvals`（`:22`）と `HOOK_PATH`（`:27`）も直す）。`test_home.py:208,297,332-363,621`・`test_home.js:18,102-108,137,357-358`・`test_webcli.py:108,147,219` の承認の部分。`test_notify.py:106-114`（`approvals._notify_if_run`）。

### 5.3 質問（AskUserQuestion・pi の ask_user）の今の経路
**cc（AskUserQuestion）**
1. cc が `AskUserQuestion` を呼ぶ直前に `PreToolUse`（matcher `AskUserQuestion`）フック `hooks/cc_question.py` が呼ばれ、`{agent: "claude", session_id, pane_id: $HERDR_PANE_ID, tool_use_id, questions}` を `POST http://127.0.0.1:8200/api/questions` に送る（出力なし・終了コード 0。フックは答えられない）。
2. `main.py:~282` `api_register_question`（Mac 自身だけ）→ `questions.register_question` → `store.save_question`。あわせて `store.save_registration`（session ↔ pane の対応）。
3. 画面は `GET /api/questions?session_id=`（`app.js:728-745`、`main.py:~340`）。そのたびに `questions.refresh_question_state`（`questions.py:107`）が `herdr pane read` で「Enter to select」（`_question_open` `:376`）が出ているか読み、出ていなければ 2 回続けて見えなかったときだけ「PC で回答済み」にする（`_MISS_LIMIT` `:48`）。pane が消えていたら期限切れにする。`runs.open_question_session_ids`（`runs.py:244`）も同じ関数を呼ぶ（日の一覧の「質問あり」の印のため）。
4. iPhone で答えると `POST /api/questions/{qid}/answer`（`main.py:~468`）→ `questions.answer_question`（`:183`）→ `_send_answers_cc`（`:521`）。**herdr でキーを送る**: 単一選択は `down` を n 回 + `enter`、複数選択は各選択肢で `enter`、Submit まで `down`、自由入力は `pane send-text`、2 問以上はレビュー画面で `enter`。送るごとに画面を読んで期待どおりか確かめる（最大 10 回 × 0.35 秒）。
**pi（ask_user）**
1. pi の拡張 `extensions/ask_user.ts` が `POST {SAI_WEB_URL}/api/questions`（`:160`）へ `{agent: "pi", pane_id: $HERDR_PANE_ID, …}` を送る。
2. iPhone で答えると `answer_question` は **herdr を使わず**答えを保存するだけ（`questions.py:~270-275` `agent == "pi"` の分岐）。拡張が 1 秒おきに `GET /api/questions/{qid}`（`:304`）で取りに来る。PC で答えたら `POST …/resolved` で知らせる（`:183`）。open のまま 10 分で期限切れ（`_PI_EXPIRE_SEC` `:25`・`expire_stale_questions`）。
3. herdr に触れるのは `setBlocked`（`herdr:blocked` イベント。herdr に「質問待ち」と知らせる合図）と、`pane_id` を送る部分だけ。

**headless にしたときの判断材料**
- pi: 経路はサーバだけで完結するので herdr を消しても質問に答えられる。ただし (a) `ask_user.ts:353` が `ctx.mode === "headless"` のとき `UI is unavailable in this mode.` のエラーを返す。pi を headless（print/RPC モード）で動かすなら、この分岐を外すか、サーバ経由の経路に切り替える必要がある（要確認: pi の headless モードで `ctx.hasUI` が何になるか）。(b) `:519-520` は `HERDR_PANE_ID` が空だと `/api/register` を送らない → herdr 無しでは登録されない。`/api/questions` 側が `save_registration` をするので質問自体は動く（`main.py` の `api_register_question`）が、`pane_id` なしの登録に変えるなら条件を直す。
- cc: `AskUserQuestion` は `CC_DENIED_TOOLS` に入っておらず残してある（`cc_command.rs:727` のテスト名と assert `"AskUserQuestion"` が deny に無い）。`claude -p` では対話の選択画面を出せない（推測）。サーバ版では (1) `AskUserQuestion` を deny に入れて、pi と同じ「サーバに質問を出して答えを待つ」ツール（MCP か slash）を足す、(2) stream-json の入力モードで `tool_result` を返す、のどちらかが要る（要設計）。**今の herdr のキー送信の経路（`_send_answers_cc` 以下・約 250 行）は headless では使えず、消す**。`cc_question.py` の `PreToolUse` フックも、フックは答えを返せない設計（docstring）なので、(1) にするなら不要になる。
- 残る共通部分（そのまま使える）: `store.py` の `questions` 表・`answer_question` の pi 分岐・`POST /api/questions/{qid}/answer`・画面の質問カード（`app.js:851-1130`）・`home.js` の待ちの一覧・`sai-web questions` / `answer`・プッシュ通知（`notify.py`）。

### 5.4 消すと壊れる他の機能（herdr 全体）
- **実行中の run への文の送信**（`POST /api/runs/{p}/{id}/send`・`app.js:1625-1680` の送信欄と送信待ち・`sai-web send`・`webcli.py:383-`）。headless の子プロセスの標準入力へ送る形に作り直さないと使えない（`claude -p` は 1 回で終わるので、送るなら stream-json 入力か `--resume` で次の run を起こす形になる。要設計）。`docs/integration.md:278`・`docs/for-sai-developers.md:130`・`sai-sci/blocks/operator.md`/`professor.md` の「人が起こした root の run への送信」の案内が影響を受ける。
- **実行の状態のランプ**（`run.live`・`app.js:327-334,545`・`runs.py:304-316`）。`status` だけで足りるなら `live` を消せる（`docs/for-sai-developers.md` の JSON の約束には `live` が無い。Web 内部の項目。要確認: `docs/integration.md` にあれば直す）。
- **通知**（`watch.py`）。「質問している」「作業が終わった」の 2 つのプッシュ通知は pane の `agent_status` に依存する。headless では run の `status` の変化（`running → completed`）か質問の登録（`questions.register_question` 時）を契機にする形へ移す。`notify_heading` は `approvals.py` と `watch.py` の共有（`watch.py:45-50`）なので、承認を消すなら `watch.py` の他の関数を消す前に `notify.py` か `runs.py` へ移すこと。`test_notify.py:75-104` が `watch.notify_heading`・`notify_status_change`・`_working_since` を使う。
- **質問カードの「PC で回答済み」判定**（画面読み）。headless では判定元が無くなるので `resolved_elsewhere` は pi の `resolved` 通知だけになる。
- **`store.py` の `pane_id` 列**（3 表に NOT NULL）。列は残して `""` を入れ続けても動く（SQLite の既存の表を作り直さずに済む）。消すなら移行が要る。
- Rust 側で `herdr_pane_id` / `herdr_pane_label` を書き出す所（`phone_export.rs:57-59,561-578`）は調査済みの範囲。`apps/web` が読む JSON の項目なので、`runs.py:284-285` を消してから消すこと（先に Web を直す）。

### 5.5 一緒に消すテスト（herdr）
- `test_herdr_submit.py`（全部。`herdr.prompt_agent` / `_submit_keys` のテスト）。
- `test_questions.py` のうち herdr の画面を偽装する物（27 か所に `herdr` が出る）: `test_claude_question_resolved_elsewhere` `:80`・`test_one_miss_does_not_resolve_but_two_do` `:136`・`test_review_screen_is_not_resolved_elsewhere` `:166`・`test_concurrent_answers_run_only_once` `:179`・`test_poll_does_not_resolve_while_sending` `:462`・`test_wrapped_question_text_is_found_on_screen` `:503`、承認の 4 本（§5.2）。**残す**: `test_pi_question_expires_after_10_minutes` `:227`・`test_answer_requires_all_questions` `:216`・`test_pi_question_stays_open_when_pane_not_blocked` `:61`（herdr を偽装しているが、pi が open のままであることの検査。pane の判定を消すなら中身を直す）。
- `test_runs.py:47-48,183-206`（`pane_for_run` の検査・`herdr_pane_*` を持つ見本）。`pane_for_run` と `live_status` を消すとき一緒に消す。
- `test_notify.py`（`watch` のテスト）。
- `test_askuser.js`（pi の拡張の小さな関数のテスト。`HERDR_PANE_ID` は見ていない。headless 対応で直すなら対象）。

### 5.6 文書（herdr）
- `README.md:37`（必要な物に herdr）・`:186`（`HERDR_BIN`）・`:245`（「`herdr agent list` に pi が出ないとき」）。
- `ARCHITECTURE.md:111,152`（pane の対応表）・`:183`（`herdr.py` / `watch.py` / `notify.py`）。
- `docs/integration.md:278,398`・`docs/for-sai-developers.md:130,548,581`（`herdr_pane_*`・「herdr のテキスト送信」）。
- `AGENTS.md` に herdr の記述は無い。

---

## 6. 順番の提案（依存の都合）
1. §1 遠隔ブラウザ（独立。テストの `translate-pdf` を先に移す）。
2. §2 launchd・§3 キーチェーン（独立。文書とひな形のみ／`paths.rs` のみ）。
3. §4 5 時間枠 → 次に cc-cost の書き手（`cc-statusline`）を `result` 行へ替える（§4.3）。`apps/web` の JSON は変わらない。
4. §5 は、(a) `watch.notify_heading` を共有の場所へ移す → (b) 承認（§5.2）を丸ごと消す → (c) 通知の契機を run の状態へ移して `watch.py` を消す → (d) `/send` と `live` を headless 用に作り直すか消す → (e) cc の質問の新経路を決めてから `questions.py` の画面読み・キー送信と `herdr.py` を消す。
5. 画面ファイルを変えたら `sw.js` の `CACHE`（`sw.js:90`）と `index.html` の `?v=`（89）を一緒に上げる（AGENTS.md）。`apps/web/tests/test_globals.js` は `static/*.js` を全部読む（`:15`）ので、ファイルを消すと検査の対象から自然に外れる。CI（`.github/workflows/ci.yml:43,45`）は `test_*.js` / `test_*.py` を全部回すので、消したテストの分は CI の記述を変えなくてよい（AGENTS.md の「JS 36 本」「Python 27 本」の数だけ直す）。

## 分からなかったこと
- `claude -p --output-format stream-json` の `result` 行の項目と、headless で transcript の jsonl が `~/.claude/projects/` に残るか（repo のコードでは確かめられない。実機で 1 回見ること）。
- `--dangerously-skip-permissions` でも承認が残る操作があるか（cc の版による）。
- pi の headless モードでの `ctx.hasUI` の値（`ask_user.ts:353`）。
- `phone_export.rs` の `unix_now_f64`・`write_json_atomic` が statusline 以外でも使われるか（呼び出し元の grep が `phone_export.rs` の内側だけ未確認）。
- Linux の常駐の仕方（systemd の unit）。今のコードに無いので文書の追記が要る（今回の範囲外）。

