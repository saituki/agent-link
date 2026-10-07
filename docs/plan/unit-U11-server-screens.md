# 実装計画書 U11: プロジェクトとコンテナ・API キーと Claude のトークンの画面

## 目的
多人数モードの画面に、次を足す。
- Projects: 「新しいプロジェクト」と、各プロジェクトのコンテナの状態・起動・停止。
- アカウント（`#/account`）: 「API キーと Claude のトークン」の区画（設定済みかどうか・末尾 4 文字・保存・消す・規約の注意）。
- 管理（`#/admin`）: 全員のコンテナの一覧と起動・停止。

## 背景
- sai-tree は sai-moon を複製した Linux の多人数サーバ版である。
- サーバの API は U09（C9）が作った（下に全文を写す）。画面の区画の足し方は U07 が決めた（`acSections`・`acAdminSections` の配列に関数を足す）。
- 単独モード（開発とテスト用）の画面は変えない。モードは `window.SaiAuth.mode`（U07。`GET /api/auth/me`）で分ける。
- Claude は、各自が自分の Mac で `claude setup-token` を実行して発行した長命トークンを貼る（利用者の決定）。Anthropic の規約は資格情報の保存・仲介を禁じているので、灰色であることを画面に出す。

## 前提（着手時点のコードの状態）
- U07・U09・U10 が終わっている。
- Projects の一覧: `apps/web/src/sai_web/static/project.js:197` `renderProjects`・`:236` `projectRow`・`:277` `loadProjects`。
- アカウントと管理の画面: `apps/web/src/sai_web/static/account.js`（U07。`acSections`・`acAdminSections`）。

## 変更内容

### 1. Projects — `project.js`
- 多人数モードのとき、一覧の上に「新しいプロジェクト」のボタン。押すと `saiPrompt` で短い名前を聞き（`^[a-z0-9][a-z0-9-]{0,29}$` を送る前に検査。DOM を使わない関数 `prjValidShortName` にしてテストする）、`POST /api/projects {"name"}`。待つ間は「作成中…（1 分ほどかかります）」と出し、ボタンを押せなくする。成功したら一覧を読み直す。画面に出すプロジェクトの名前は、登録簿の名前（`<短い名前>-<プロジェクト>`）のままでよい。
- 多人数モードのとき、各行（`projectRow`）に `container_state` の札（動いている / 止まっている / 無い）と、「起動」/「停止」のボタン（`POST /api/projects/{name}/container/{start|stop}`。停止の前に `saiConfirm`「止めると、動いているエージェントも止まります」）。

### 2. アカウント — `account.js` の `acSections` に区画を足す（多人数モードだけ）
**API キーと Claude のトークン**
- 5 行: 「Claude（`claude setup-token` のトークン）」（`CLAUDE_CODE_OAUTH_TOKEN`）・「OpenRouter」（`OPENROUTER_API_KEY`）・「Semantic Scholar」（`SEMANTICSCHOLAR_API_KEY`）・「OpenAlex」（`OPENALEX_API_KEY`）・「NCBI（PubMed）」（`NCBI_API_KEY`）。
- 各行に「設定済み（…末尾 4 文字）」か「未設定」、入力欄（`type="password"`・`autocomplete="off"`）、「保存」と「消す」。
- 保存・消すは `PUT /api/me/keys` に変えた 1 つだけを送る。結果の `recreated` があれば「<名前> を新しい値で起動し直しました」、`pending` があれば「<名前> は実行中のため、終わってから起動し直すと反映されます」を `saiToast` で出す。
- Claude の行の下に、次の説明と注意をそのまま出す:
  > 自分の Mac の端末で `claude setup-token` を実行し、研究室の Team プランに大学のメールアドレスでログインして発行したトークンを貼ってください。トークンはあなたのプロジェクトのコンテナにだけ渡り、他の人とは共有されません。
  > 注意: Anthropic の規約は、アプリが Claude の資格情報を保存・仲介することを禁じています。この機能はその点で灰色です。使うかどうかはあなたの判断で決めてください。トークンは他の人に渡さないでください。

### 3. 管理 — `account.js` の `acAdminSections` に区画を足す
- 「プロジェクトとコンテナ」: `GET /api/admin/containers` の表（持ち主・プロジェクト・状態・Docker の状態の文）。行ごとに「起動」/「停止」（`POST /api/admin/containers/{container}/{action}`。停止の前に `saiConfirm`）。中身（会話・論文）へのリンクは出さない。

### 4. 版
- `sw.js` の `CACHE` を 1 つ上げ、`index.html` の `?v=` をそろえる。
- 390px と 1280px で崩れないこと。

## 共有契約（他の作業と共有する取り決め。worker は変えない）

**C9 プロジェクト・コンテナ・キーの API**（U09。全文）
- `POST /api/projects {"name"}` → `{"name":"<user>-<name>","container":"sai-<user>-<name>"}`／400／409。
- `GET /api/projects` の各行に `container_state`（`running` / `exited` / `missing`）。
- `GET /api/projects/{name}/container` → `{"container","state"}`。`POST /api/projects/{name}/container/{start|stop}`。
- `GET /api/me/keys` → `{"keys":[{"name","set","last4"}]}`。`PUT /api/me/keys {"<名前>": "<値>|"}` → `{"ok":true,"recreated":[…],"pending":[…]}`。名前は `CLAUDE_CODE_OAUTH_TOKEN`・`OPENROUTER_API_KEY`・`SEMANTICSCHOLAR_API_KEY`・`OPENALEX_API_KEY`・`NCBI_API_KEY`。
- `GET /api/admin/containers` → `{"containers":[{"container","owner","project","state","status"}]}`。`POST /api/admin/containers/{container}/{start|stop}`。

**画面の区画**（U07。全文）
- `#/account` と `#/admin` は、区画を描く関数の配列（`acSections`・`acAdminSections`）を順に呼んで描く。区画の関数は `(root: HTMLElement, me: {name,email,is_admin}) => void`。

## ファイルの所有権
- 書いてよい: `apps/web/src/sai_web/static/project.js`・`account.js`（区画を足すだけ）・`index.html`・`sw.js`・`style.css`・`apps/web/tests/test_project.js`（無ければ新規）
- 読むだけ: `apps/web/src/sai_web/static/ui.js`
- 触らない: `apps/web/src/sai_web/*.py`

## 完了条件
- [ ] AC1: `prjValidShortName` の node のテスト（`x`・`a-b`・30 文字・31 文字・先頭のハイフン・大文字）。`test_globals.js` を含む JS のテストがすべて通る。
- [ ] AC2: 8299 番の多人数モードのサーバ（偽の docker を `SAI_DOCKER_BIN` で差してよい）で、ブラウザ（playwright）で: Projects に「新しいプロジェクト」が出て、作ると一覧に増える。アカウントの区画で Claude のトークンを保存すると「設定済み（…末尾 4 文字）」になり、入力欄に値が残らない。注意書きが出る。管理の区画にコンテナの一覧が出る。390px と 1280px の画面写真を作業報告書に付ける。
- [ ] AC3: 単独モードの 8299 番で、Projects・アカウントのボタンの見た目が今と変わらない。

## 検証
```bash
cd ~/workdir/Sai/sai-tree
for f in apps/web/tests/test_*.js; do node "$f" || exit 1; done
```
サーバは AGENTS.md の手順で一時フォルダの置き場を使って起こし、終わったら止めて消す。

## 決定事項と理由
- Claude のトークンを貼る形にする — 理由: 利用者の決定。Web の端末でのログインの中継は作らない。規約上の灰色は画面に出す。
- 管理者には中身へのリンクを出さない — 理由: 要件（管理者はコンテナの起動・停止だけで、中身は見ない）。

## 要確認
- なし。
