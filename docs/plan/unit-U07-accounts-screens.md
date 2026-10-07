# 実装計画書 U07: ログイン・パスワード変更・アカウント・管理の画面

## 目的
多人数モードの Web に、ログイン画面（大学のメールアドレスとパスワード）・パスワード変更画面・アカウント画面（個人のトークンの作成と削除を含む）・管理画面（利用者の一覧と作成・変更）を足す。単独モード（開発とテスト用）の画面は変わらない。

## 背景
- sai-tree は sai-moon を複製した Linux の多人数サーバ版である。ログインの ID は大学のメールアドレス、フォルダ名とコンテナ名には管理者が決める短い名前を使う（要件）。
- サーバ側の住所は U06 が作る（下の共有契約 C5' に全文を写す）。
- 個人のトークンは、各自の Mac の sai-prep がサーバを呼ぶのに使う（ガイド 5）。
- 今の画面は、401 を受けると `prompt("合言葉を入力してください")` を出す（`apps/web/src/sai_web/static/app.js:32-47` `apiFetch`）。多人数モードでは合言葉は無いので、ログイン画面へ移す。
- 画面の作り方の約束（AGENTS.md「画面のコード」）: 新しい画面は `window.SaiRoutes` に登録し、`app.js` に画面ごとの分岐を足さない。`ui.js` の部品（`saiApi`・`saiToast`・`saiConfirm`・`saiPrompt`・`saiSheet`・`saiEmpty`）を使う。一番外側の名前には画面ごとの頭を付ける（`apps/web/tests/test_globals.js` が重なりを検査する）。

## 前提（着手時点のコードの状態）
- U05 が終わっている。U06 は並列で進む（この作業の作業用コピーには U06 が無い。サーバでの確認（AC3）は U06 とのマージの後に kantoku が行う）。
- 画面の登録の例: `apps/web/src/sai_web/static/usage.js` の末尾の `window.SaiRoutes` への登録（`match`・`view`・`tab`・`hideTabbar`・`interval`・`open`・`poll`）。
- 画面の枠は `apps/web/src/sai_web/static/index.html` の `<section class="view">`（例 `<section id="view-home" class="view" hidden><div id="home-root"></div></section>`）。
- Home の見出しは `apps/web/src/sai_web/static/home.js` の `hmRender` の `appbar`。
- 版: `sw.js` の `const CACHE = "sai-web-v…"`・`ASSETS`、`index.html` の `?v=…`。

## 変更内容

### 1. 新しい画面ファイル `apps/web/src/sai_web/static/account.js`（頭は `ac`）
登録する住所:

| 住所 | 画面 | 中身 |
|---|---|---|
| `#/login` | `view-login`（タブバーを隠す） | メールアドレス（`type="email"`）・パスワード・「ログイン」。`POST /api/auth/login`。成功したら、`must_change_password` なら `#/password`、それ以外は `sessionStorage` に覚えた元の住所（無ければ `#/home`）へ |
| `#/password` | `view-password`（タブバーを隠す） | 今のパスワード・新しいパスワード・確認。8 文字未満・確認と不一致は送る前に画面で止める。`POST /api/auth/password`。成功したら `#/home` |
| `#/account` | `view-account` | 区画を順に並べる（`acSections` の配列）。この作業で作る区画: ①「アカウント」（メールアドレス・短い名前・役割・「パスワードを変える」（`#/password`）・「ログアウト」（`POST /api/auth/logout` → `#/login`）・管理者なら「管理」（`#/admin`）への行） ②「sai-prep のトークン」（一覧（ラベル・作った日・最後に使った日）・「トークンを作る」（`saiSheet` でラベルを入れる → `POST /api/me/tokens` → 返った値を 1 回だけ大きく見せ、「コピー」ボタンと「この値は二度と表示されません」の注意を出す）・行ごとの「消す」（`saiConfirm` → `DELETE /api/me/tokens/{id}`）） |
| `#/admin` | `view-admin` | 区画を順に並べる（`acAdminSections`）。この作業で作る区画:「利用者」の一覧（短い名前・メール・管理者・無効・初回変更待ち）。「利用者を作る」（`saiSheet` に短い名前・メールアドレス・初期パスワード・管理者の印）→ `POST /api/admin/users`。行ごとに「パスワードを再発行」「メールを変える」「管理者にする / 外す」「無効にする / 戻す」→ `PATCH /api/admin/users/{name}`（無効にする前に `saiConfirm`） |

- **U11 がこの 2 つの画面に区画を足す**（「API キーと Claude のトークン」・「プロジェクトとコンテナ」）。区画は「`(root, me) => void` の関数」とし、配列 `acSections`・`acAdminSections` に並べて順に呼ぶ。
- 名前・メール・パスワードの検査（`^[a-z][a-z0-9]{1,15}$`・`^[^@\s]+@[^@\s]+\.[^@\s]+$`・8〜128 文字）は DOM を使わない関数（`acValidName`・`acValidEmail`・`acValidPassword`）にし、テストする。
- 画面の文字はすべて日本語。エラーはサーバの `detail` をそのまま `saiToast` で出す。

### 2. 401 と 403 の扱い — `apps/web/src/sai_web/static/app.js:32-47` `apiFetch`
- 起動時に 1 回 `GET /api/auth/me` を読み、`mode` を `window.SaiAuth = {mode, name, email, is_admin}` に置く（`account.js` の関数 `acLoadMe` を `app.js` の起動の処理から 1 行で呼ぶ。`app.js` に画面の分岐は書かない）。
- `apiFetch` が 401 を受けたとき: `window.SaiAuth?.mode === "multi"` なら、今の住所を `sessionStorage` に覚えて `location.hash = "#/login"` にする（`prompt` は出さない）。単独モードは今のまま。
- 403 で `detail` が「パスワードを変えてください」なら `#/password` へ移す。
- `#/login` と `#/password` を開いている間は、他の画面のポーリングを止める（今の `app.js` が他の画面の `poll` を呼ばないことを確かめる。呼ぶなら、`account.js` の `match` が最優先になるよう登録の順に注意する）。

### 3. 入口のボタン — `apps/web/src/sai_web/static/home.js` の `hmRender` の `appbar`
- 多人数モード（`window.SaiAuth?.mode === "multi"`）のときだけ、`appbar` の右に「アカウント」のボタン（`#/account` へ）を出す。

### 4. 枠と版
- `index.html`: `view-login`・`view-password`・`view-account`・`view-admin` の 4 つの `<section class="view" hidden>` と `<script src="/account.js?v=…">` を足す（`app.js` より前に読む）。
- `sw.js`: `CACHE` を 1 つ上げ、`ASSETS` に `/account.js` を足す。`index.html` の `?v=` を同じ値にそろえる。
- 390px と 1280px の両方で崩れないこと。入力欄は `autocomplete="username"`（メール欄）・`current-password`・`new-password` を付ける。

## 共有契約（他の作業と共有する取り決め。worker は変えない）

**C5' アカウントの住所**（U06 が作る。全文）
- `GET /api/auth/me` → 単独 `{"mode":"single"}`／多人数 `{"mode":"multi","name","email","is_admin","must_change_password"}`／未ログイン 401。
- `POST /api/auth/login` `{"email","password"}` → 200 `{"name","email","is_admin","must_change_password"}`＋Cookie／401「メールアドレスかパスワードが違います」。
- `POST /api/auth/logout` → `{"ok":true}`。
- `POST /api/auth/password` `{"current","new"}` → `{"ok":true}`／400。
- `GET /api/me/tokens` → `{"tokens":[{"id","label","created_at","last_used_at"}]}`。
- `POST /api/me/tokens` `{"label"}` → `{"id","label","token":"sai_…"}`（値はこの 1 回だけ）。
- `DELETE /api/me/tokens/{id}` → `{"ok":true}`／404。
- `GET /api/admin/users` → `{"users":[{"name","email","is_admin","disabled","must_change_password","created_at"}]}`。
- `POST /api/admin/users` `{"name","email","password","is_admin"}` → 作成（同名・同メール 409）。
- `PATCH /api/admin/users/{name}` `{"email"?, "is_admin"?, "disabled"?, "password"?}`。
- 多人数モードの API は、未ログインで 401「ログインしてください」、初回のパスワード変更前は 403「パスワードを変えてください」。

**画面の区画**（U11 が使う）
- `#/account` と `#/admin` は、区画を描く関数の配列（`acSections`・`acAdminSections`）を順に呼んで描く。区画の関数は `(root: HTMLElement, me: {name,email,is_admin}) => void`。U11 は配列に関数を足すだけで区画を足せる。

## ファイルの所有権
- 書いてよい: `apps/web/src/sai_web/static/account.js`（新規）・`app.js`（`apiFetch` と起動の 1 行だけ）・`home.js`（`appbar` のボタンだけ）・`index.html`・`sw.js`・`style.css`（必要な分だけ足す）・`apps/web/tests/test_account.js`（新規）
- 読むだけ: `apps/web/src/sai_web/static/ui.js`・`usage.js`
- 触らない: `apps/web/src/sai_web/*.py`（U06 が書く）

## 完了条件
- [ ] AC1: `apps/web/tests/test_account.js`: `acValidName`・`acValidEmail`・`acValidPassword` の境界（2 文字・16 文字・17 文字・ハイフン・大文字、`a@b.c`・`@b.c`・空白入り、7 文字・8 文字・129 文字）。
- [ ] AC2: `test_globals.js` を含む JS のテストがすべて通る。
- [ ] AC3（U06 とのマージ後に kantoku が行う）: 8299 番に多人数モードのサーバ（`SAI_WEB_COOKIE_SECURE=0`）を起こし、ブラウザ（playwright）で: 管理者でメールアドレスで login → 初回は `#/password` に移される → 変えると Home に「アカウント」が出る → `#/admin` で一般の利用者を作る → `#/account` で sai-prep のトークンを作ると値が 1 回だけ出る → ログアウト → その人で login → `#/admin` を直接開いても一覧が出ない（403 のトースト）。390px と 1280px の画面写真を報告に付ける。
- [ ] AC4: 単独モードの 8299 番で、今までどおり Home が開き、「アカウント」のボタンが出ない。

## 検証
```bash
cd ~/workdir/Sai/sai-tree
for f in apps/web/tests/test_*.js; do node "$f" || exit 1; done
```
サーバは AGENTS.md の手順で一時フォルダの置き場を使って起こし、終わったら止めて消す。

## 決定事項と理由
- ログインは Web の画面（Cookie）で行い、合言葉の `prompt` は単独モードだけに残す — 理由: 多人数モードに合言葉は無い（U06）。
- 区画の配列にしておく — 理由: U11 が同じ画面に API キー・Claude のトークン・コンテナを足す。後の作業が同じファイルを書くので、足し方を決めておく。

## 要確認
- なし。
