# 実装計画書 U03: Web の画面（JS・CSS・HTML）を sai-link の画面だけにする

## 目的
`apps/web/src/sai_web/static/` から、論文・Research・Radio・MLflow・Customer・多人数モード（ログイン・パスワード・管理・コンテナ・sai-ts）の画面を消す。下のタブは Home・Projects・Chats・Account の 4 つにする。JS のテストが通る状態にする。

## 背景
- agent-link は Mac で 1 人が使う開発用の道具である（要件定義書 `docs/plan/requirements.md`）。
- 画面は素の JavaScript で、画面ファイルを `index.html` の `<script>` と `sw.js` の `ASSETS` に並べている。`sw.js:192` の `addAll(ASSETS)` は、1 つでもファイルが欠けると install に失敗する。なので、ファイルを消したら `ASSETS` からも必ず消す。
- 名前（`sai`・`sai-web-v`）はこの作業では変えない。後の U06 が変える。

## 前提（着手時点のコードの状態）
- リポジトリは `~/workdir/Sai/agent-link`（sai-tree の main を clone した物）。行番号は複製の元（sai-tree の 4736175）の値。ずれていたら、関数名・id で探す。
- 画面の版は `sw.js:125` の `CACHE = "sai-web-v131"` と、`index.html` の全部の `?v=131` である。`apps/web/tests/test_migrate.js` がこの数字を固定している。
- 同じガイドの U02（サーバ）と並列で動く。サーバ側は次の物を消す（共有契約 C3）。`/api/auth/*`・`/api/me/*`・`/api/admin/*`・`/api/customer`・`/api/feedback`・論文・sci・radio・mlflow・prep・コンテナ・`ts/open` の route。`GET /api/home` の `papers`、`GET /api/days` の `customer`、`GET /api/projects` の `container_state`。

## 変更内容

### 1. ファイルを消す（`static/`）
`customer.js`・`papers.js`・`paperimport.js`・`cite.js`・`papers.css`・`radio.js`・`pcm-worklet.js`・`scitree.js`・`sciedit.js`・`sciread.js`・`scigraph.js`・`scitree.css`・`sciedit.css`・`scigraph.css`・`mlflow.js`・`mlfchart.js`・`mlflow.css`

`voiceask.js`（質問の読み上げと音声の答え）は**残す**。端末の `speechSynthesis`・`SpeechRecognition` だけを使い、Radio を使っていない。`app.js:1899-1945` の `attachVoiceInput`（Chats の音声入力）も残す。

### 2. `index.html`
- `<link>`（`:13-24`）から `papers`・`scitree`・`sciedit`・`scigraph`・`mlflow` の 5 本を消す。
- 画面の要素を消す: `#view-customer`（`:44-50`）、`#view-papers-*`・`#view-paper*`・`#view-sci-*`・`#view-radio-player`・`#radio-inputbar`（`:195-497`）、`#view-login`・`#view-password`・`#view-admin`（`:511-520` の中）、`#view-paper-cites`（`:530`）、`#view-mlflow`（`:533`）、`<audio id="radio-audio">`（`:536`）。
- `#tabbar`（`:569-609`）から Research（`:599-607`）を消す。残すのは Home・Projects・Chats・Account。
- `<script>`（`:621-677`）から、消したファイルの行を消す。
- 残す: `#view-usage-detail`・`#view-home`・`#view-account`・`#view-workflow`・`#view-defs` など。

### 3. `app.js`
- `:40-41` の 401 のとき `SaiAuth.mode === "multi"` の枝を消す。401 は単独モードの合言葉の入力に進む（今の単独の動き）。
- `TAB_FIRST_URL`（`:223-228`）から `research: "#/papers/dashboard"`（`:227`）を消す。`tabbarHidden`（`:269-278`）とタブのクリック（`:312-321`）の `research` の分も消す。
- Customer の行（`daysCustomer` `:380`・`:392`・`customerDayRow` `:426-442`・`:464`）を消す。

### 4. `account.js`
- Account 画面（`#/account`、`:1103-1110`）と、その中の Usage の区画（`acSections` `:1125`）は**残す**。`usage.js:567-572` が `view-account` を使う。
- 消す: `#/login`・`#/password`・`#/admin` の route（`:1072-1119` の中）、`acLoginOpen`・`acPasswordOpen`・`acAdminOpen`・`acAdminSections`、鍵・Claude ログイン・トークンの区画、`window.SaiAuth` の定義（`:66`・`:201`・`:318`）。
- 他のファイルで `SaiAuth` を読んでいる所（`grep -rn SaiAuth static/`）は、単独モードとして動くように直す（その分岐を消す）。

### 5. 多人数・論文の分岐を消す（残すファイルの中）
- `project.js:206`・`:266`・`:286`・`:309-314`・`:406`・`:430`: コンテナの札・作成のバー・`ts/open`・`container/start|stop`。`prjMultiUser()` とそれを使う分岐を全部消す。
- `home.js:30`（`HM_PAPERS_KEY`）、`:309-330`、`:638-645`（論文の要対応）、`:323` の `sci_` の判定を消す。
- `style.css`: `papers-*`・`paper-*`・`lib-*`・`sci-*`・`radio-*`・`mlf-*`・`customer-*`・`login-*`・`admin-*` のクラスの規則を消す（`:1338`・`:1593` 以降など）。ただし `.papers-seg-row`（`style.css:1343`）は `linkview.js:1870` が使うので**残す**。消す前に、そのクラスを残すファイルが使っていないか `grep` で確かめる。
- `commands.js`・`linkview.js`・`newchat.js`・`usage.js` に、消した API（`/api/projects/{name}/papers`・`/api/mlflow`・`/api/radio`・`/api/customer`・`/api/feedback`・`/api/auth`）の呼び出しが残っていないか grep し、あれば消す。

### 6. `sw.js`
- `ASSETS`（`:126`）から、消したファイルを全部消す: `/papers.css`・`/papers.js`・`/radio.js`・`/paperimport.js`・`/cite.js`・`/scitree.js`・`/sciedit.js`・`/sciread.js`・`/scigraph.js`・`/scitree.css`・`/sciedit.css`・`/scigraph.css`・`/mlflow.js`・`/mlfchart.js`・`/mlflow.css`・`/customer.js`・`/pcm-worklet.js`。
- `CACHE` を `"sai-web-v132"` に上げ、`index.html` の `?v=` を全部 `132` にそろえる。`sw.js` の版の履歴の注記に 1 行足す（「agent-link: 論文・Research・多人数の画面を消した」）。

### 7. テスト（`apps/web/tests/test_*.js`）
- 消す: `test_cite.js`・`test_customer.js`・`test_mlfchart.js`・`test_mlflow.js`・`test_paperimport.js`・`test_papers.js`・`test_papers_find.js`・`test_radio.js`・`test_sciedit.js`・`test_scigraph.js`・`test_sciread.js`・`test_scitree.js`
- 直す: `test_account.js`（ログイン・管理の検査を消す）・`test_commands.js`・`test_home.js`（論文の検査を消す）・`test_project.js`（コンテナの検査を消す）・`test_migrate.js`（版を 132 に、`ASSETS` と実物のファイルが一致することの検査を合わせる）・`test_globals.js`（消したファイルを一覧から外す）・`test_contrast.js`（消した CSS を外す）。
- 残す: `test_voiceask.js` を含むそのほか。

## 共有契約（他の作業と共有する取り決め。worker は変えない）
- C1（ガイド 1 の約束）: この時点では名前を変えない（`sai`・`sai-web-v`・`SAI_WIDE_PX`・`SAI_TERMS` のまま）。
- C3（U02 と U03 の境目）: 残す API の住所と JSON の形は変わらない。消えるのは次の物だけ。`GET /api/home` の `papers`、`GET /api/days` の `customer`、`GET /api/projects` の `container_state`・`container`、`GET /api/auth/me`（route ごと）。画面はこれらを読まない。
- C4: 画面の版は `sai-web-v132`・`?v=132` にする（U06 が名前を変えるときに、この数字から上げる）。

## ファイルの所有権
- 書いてよい: `apps/web/src/sai_web/static/**`・`apps/web/tests/test_*.js`
- 読むだけ: `apps/web/src/sai_web/*.py`（API の形の確認）
- 触らない: `apps/web/src/sai_web/*.py`・`apps/web/tests/test_*.py`・`crates/**`・`.github/**`・`docs/**`

## 完了条件
- [ ] AC1: 変更内容 1 のファイルが無い。`sw.js` の `ASSETS` の全部のファイルが `static/` にある（`test_migrate.js` で確かめる）。
- [ ] AC2: 残す JS のテストがすべて通る（`for f in apps/web/tests/test_*.js; do node "$f" || exit 1; done`）。
- [ ] AC3: `grep -rnE "papers|scitree|sciedit|radio|mlflow|customer|SaiAuth|prjMultiUser|/api/auth|/api/admin" apps/web/src/sai_web/static/` の結果が、`.papers-seg-row` とその使い手だけになる。
- [ ] AC4: U02 と合わせた後、一時フォルダの置き場で 8297 番のサーバを起こして開くと、下のタブが Home・Projects・Chats・Account の 4 つになる。Home・Projects・Chats・Account（Usage）の画面が開き、ブラウザの console に error が出ない。幅 390px と 1280px の両方で確かめる。

## 検証
```bash
cd ~/workdir/Sai/agent-link   # worktree の中ならその場所
for f in apps/web/tests/test_*.js; do node "$f" || exit 1; done
```
AC4 は U02 と合わせた後に kantoku が確かめる（ガイドの「統合と検証」）。worker は、U02 が終わっていればその場で確かめてよい。

## 決定事項と理由
- `voiceask.js` と `attachVoiceInput` は残す — 理由: 質問の読み上げと Chats の音声入力に使う。Radio を使っていない。
- Account 画面は残し、Usage だけを載せる — 理由: Usage の画面が `view-account` を使う（`usage.js:567-572`）。
- `.papers-seg-row` は名前のまま残す — 理由: Workflow（`linkview.js:1870`）が使う。名前を変えると差分が増えるだけである。

## 要確認
- `style.css` の消す範囲は、クラス名で grep して決める。使い手が残るクラスは消さない。
