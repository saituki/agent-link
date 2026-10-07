# 実装計画書 U04: Chats の画面（逐次表示・中断・新しいチャット）と、承認・herdr の画面の削除

## 目的
Chats の会話画面（`#/run/<project>/<run_id>`）を、headless の実行と続けて会話できる画面にする。
- エージェントの生成中の文を、1 秒ごとに少しずつ表示する。
- 入力欄から続きを送れる。「中断」で止められる。
- Chats の一覧から「新しいチャット」を始められる（プロジェクト・block・runner・model・最初の文を選んで、カード（task.md）を作って実行し、その会話へ移る）。

あわせて、承認（PermissionRequest）のカードと、herdr を前提にした画面の部分を消す。

## 背景
- sai-tree は sai-moon を複製した Linux の多人数サーバ版で、エージェントはすべて headless で動く。1 つのチャットは task.md で起動した 1 つの run との会話で、runner・provider・model は task.md で決まる。
- サーバの API は U03 が作る（下の共有契約 C6 に全文を写す）。承認の住所と `GET /api/home` の `approvals` は U03 が消した。
- 今の会話画面は 2 秒ごとに `pollRun`（`apps/web/src/sai_web/static/app.js:710-713`）で実行と会話の差分を読み、`renderItem`（`app.js:1348-1389`）で `user` / `agent` / `tool` / `event` を描く。agent の本文は `renderMarkdown`（`app.js:1377`）。
- カードの作成と実行の API は既にある: `POST /api/projects/{name}/card`（本文は `cfBuildPayload` の形。`cardform.js:260-274`）と `POST /api/projects/{name}/runs/{run_id}/start`（`docs/integration.md:156`）。選択肢は `GET /api/projects/{name}/card/options`。
- カードを実行してから、実行の書き出し（`GET /api/runs/{p}/{id}` が引けるようになる）まで、daemon の書き出しの周期（約 5 秒）だけ遅れる（`crates/sai-link-engine/src/daemon.rs:1055` `export_phone_runs`）。
- 画面側の承認と herdr の部分の一覧は、調査 E3（`docs/plan/research/E3-mac-only-removal.md` §5.1・§5.2）にある: `app.js:327-334`（`STATUS_LAMP` と `run.live` のランプ）・`app.js:545`（`run.live === "blocked"`）・`app.js:722-764`・`:992-1150`（承認の読み込みとカード `approvalCard`・`submitApproval`）・`home.js:90,123-128,399`（待ちの一覧の承認）・`style.css:902-906`・`linkview.js:748` のコメント。

## 前提（着手時点のコードの状態）
- U03 が終わっている（C6）。
- 会話画面の枠: `apps/web/src/sai_web/static/index.html:61-82`（`#run-lamp`・`#finalize-button`・`#stats-button`・`#chat-list`・`#chat-input`・`#send-button`）。
- 更新の間隔: `app.js:105-111` `baseInterval`（実行の画面は 2000ms）。
- 送信: `app.js:1656-1690`（`#send-button` の click）と、通信が切れたときの送り直し `flushPendingSends`（`app.js:1629-1652`）。
- Chats の一覧（`#/`）の見出し: `index.html:31-37`。
- 状態の札: `runLamp`（`app.js:333-338`）・`runStatusText`（`app.js:339-347`）。

## 変更内容

### 1. 生成中の文の表示 — `app.js`
- `loadChatMore`（`pollRun` から呼ばれる。`/items` を読む所）で、応答の `partial`・`live`・`error` を受け取る。
- `partial` が空でなければ、`#chat-list` の末尾に 1 つだけ「生成中」の agent の吹き出し（`id="chat-partial"`、`class` に `partial` を足し、点滅する印を付ける）を置き、毎回中身を `renderMarkdown(partial)` で置き換える。`partial` が空になったら消す。確定した `agent` の項目が来たら、生成中の吹き出しはその下に来ないよう、毎回末尾へ付け直す。
- 下端追従（`stickToBottom`・`app.js:601`）は、生成中の吹き出しが伸びたときも効くようにする。
- `error` が空でなければ、会話の末尾に `event` の赤い帯（`renderEvent` の error の形）で 1 回だけ出す（同じ文は 2 回出さない）。
- 生成中の文の扱い（いつ出す・消す・置き換える）は DOM を使わない関数（`chatPartialPlan(prevPartial, nextPartial)` → `"none" | "show" | "update" | "hide"`）に分け、`apps/web/src/sai_web/static/chatorder.js` に置いてテストする（`chatorder.js` は会話の中に行を置く位置の DOM を使わない関数の置き場）。

### 2. 間隔 — `app.js:105-111` `baseInterval`
- 実行の画面で、直前の `live` が `working` なら 1000ms、それ以外は今の 2000ms。`live` が変わったら `onOk()` と同じく `rearm()` で間隔を変える。

### 3. 中断 — `index.html:69-71` と `app.js`
- 見出しの `#finalize-button` の左に `#interrupt-button`（`aria-label="中断"`、表示は `⏸`）を足す。`live` が `working` のときだけ出す。
- 押すと `POST /api/runs/{p}/{id}/interrupt`。結果の `status` が `interrupted` なら `saiToast("中断しました")`、`still_working` なら `saiToast("まだ止まっていません。もう一度押すか、■ で終了してください")`。

### 4. 入力欄
- `live` が null（終わった）のときだけ入力欄を使えなくし、「この会話は終了しています」と出す。`status` が `completed` でも `live` があれば送れる（C6）。
- 送信の失敗で 409 のときは、サーバの `detail` をそのまま `saiToast` で出す（質問待ちなど）。

### 5. 新しいチャット — 新しいファイル `apps/web/src/sai_web/static/newchat.js`（頭は `nc`）
- Chats の一覧（`index.html:31-37` の見出し）に「＋」のボタン `#newchat-button`（`aria-label="新しいチャット"`）を足し、押すと `saiSheet` で次の入力を出す。
  1. プロジェクト（`GET /api/projects` の名前。前回選んだ物を `localStorage` に覚えて既定にする）
  2. block（`GET /api/projects/{name}/card/options` の `blocks`。既定は `main`）
  3. runner（options の `runners`。既定は前回の値、無ければ `cc`）
  4. model（options の `cc_models` または `providers` から。runner に合わせて出し分ける。空なら block の既定）
  5. thinking（options の `thinking`。空なら block の既定）
  6. 最初の文（複数行）
- 「始める」で:
  1. `POST /api/projects/{name}/card` に `cfBuildPayload({runner, block, model, thinking, task_text: 最初の文})` の形を送る（`cfBuildPayload` は `cardform.js` の関数を呼ぶ。写さない）。戻り値の `run_id` を取る。
  2. `POST /api/projects/{name}/runs/{run_id}/start`。
  3. `location.hash = "#/run/<name>/<run_id>"`。
- 会話画面を開いたとき、実行がまだ書き出されていない（`GET /api/runs/{p}/{id}` が 404）なら、60 秒までは「起動を待っています…」と出して 2 秒ごとに読み直す（新しいチャットから来たときだけ。`sessionStorage` に `run_id` を覚えておいて見分ける）。60 秒を過ぎたら今の 404 の表示に戻す。
- 入力の検査（プロジェクト・block・最初の文が空でない）は DOM を使わない関数 `ncValidate(state)` にし、テストする。

### 6. 承認と herdr の画面を消す
- `app.js` の承認の読み込みとカード（`:722-764` の承認の部分・`:992-1150` の `approvalCard`・`submitApproval` など）、`home.js` の承認の分岐（`:90,123-128,399`）、`style.css:902-906` を消す。
- `app.js:327-334` の `STATUS_LAMP` は C6 の `live`（`working` / `idle` / `blocked` / null）で動くので残す。herdr を指す言葉（コメント・画面の文字）を消す（`linkview.js:748` のコメントなど）。
- 承認を前提にした JS のテスト（`test_home.js:18,102-108,137,357-358` の承認の部分）を直す。

### 7. 枠と版
- `index.html`: `#interrupt-button`・`#newchat-button`・`<script src="/newchat.js?v=…">` を足す。
- `sw.js`: `CACHE` を 1 つ上げ、`ASSETS` に `/newchat.js` を足す。`index.html` の `?v=` をそろえる。
- 390px と 1280px で崩れないこと。

## 共有契約（他の作業と共有する取り決め。worker は変えない）

**C6 チャットの API**（U03 が作る。全文）
- `GET /api/runs/{p}/{id}/items?after=<cursor>` → `{"items":[…今の形…],"next_cursor":"…","live":"working"|"idle"|"blocked"|null,"partial":"…","error":"…"}`。
- `GET /api/runs/{p}/{id}` と一覧の実行に `"headless": true|false` と `"live"`。`live` は、`status` が `running` でなくても値が入ることがある。
- `POST /api/runs/{p}/{id}/send` `{"text"}` → `{"ok":true}`。
- `POST /api/runs/{p}/{id}/interrupt` → `{"run_id","status":"interrupted"|"still_working","agent_status"}`。
- 承認の住所（`/api/approvals*`）は無い。`GET /api/home` に `approvals` は無い。

## ファイルの所有権
- 書いてよい: `apps/web/src/sai_web/static/app.js`・`chatorder.js`・`newchat.js`（新規）・`home.js`（承認の分岐だけ）・`linkview.js`（コメントだけ）・`index.html`・`sw.js`・`style.css`・`apps/web/tests/test_chatorder.js`（既存に足す）・`test_newchat.js`（新規）・`test_home.js`
- 読むだけ: `apps/web/src/sai_web/static/cardform.js`・`ui.js`・`markdown.js`
- 触らない: `apps/web/src/sai_web/*.py`（U03 が書く）

## 完了条件
- [ ] AC1: `chatPartialPlan` と `ncValidate` の node のテストが通る。`test_globals.js` を含む JS のテストがすべて通る。
- [ ] AC2: 8299 番のサーバ（`SAI_HOME` は一時フォルダ）に、偽の実行の書き出しと `state.json` を置いて、ブラウザ（playwright）で: `partial` を書き換えると吹き出しの中身が 1〜2 秒で変わる。`state` を `working` にすると ⏸ が出て、`idle` にすると消える。390px と 1280px の画面写真を作業報告書に付ける。U03 がまだマージされていない作業用コピーでは、偽の応答を返す `saiApi` の差し替えで行ってよい（その旨を報告に書く）。
- [ ] AC3: 同じサーバで、「＋」から新しいチャットのシートが開き、空のときは「始める」が押せない。カードの作成と開始の住所が順に呼ばれることを、偽の CLI の記録かサーバのログで確かめる。
- [ ] AC4: Home の待ちの一覧と会話画面に、承認のカードが出ない（`approvals` が無い応答で JS が落ちない）。

## 検証
```bash
cd ~/workdir/Sai/sai-tree
for f in apps/web/tests/test_*.js; do node "$f" || exit 1; done
```
サーバは AGENTS.md の手順で一時フォルダの置き場を使って起こし、終わったら止めて消す。

## 決定事項と理由
- 新しい画面を作らず、今の会話画面に生成中の吹き出しと中断を足す — 理由: 会話の描画（折りたたみ・質問カード・日付の区切り・統計）を保つため。Open WebUI などの既存の画面は、task.md から自分で動き続ける run を見る用途に合わないので使わない（利用者の決定）。
- 新しいチャットは「カードを作る → 実行」の既存の 2 本の API を画面から続けて呼ぶ — 理由: サーバに新しい処理を足さずに済む。runner・model・thinking はカード（task.md）に書かれ、そこで決まる。

## 要確認
- なし。
