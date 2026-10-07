# 実装ガイド 3/5: Mac 専用の機能の削除と、アカウント・ログイン

## ゴール
Mac 専用の機能（遠隔ブラウザ・launchd・キーチェーン・cc の 5 時間枠）が sai-tree から消える。`SAI_WEB_USERS_ROOT` を付けて sai-web を起こすと多人数モードになり、管理者が作ったアカウントに**大学のメールアドレス**でログインし、初回にパスワードを変え、自分のプロジェクト・実行・会話だけが見える。各自がアカウント画面で sai-prep 用の個人のトークンを作れる。管理者は管理画面で利用者（短い名前・メールアドレス）を作る・変える。`SAI_WEB_USERS_ROOT` が無ければ今の単独モード（開発とテスト用）のまま。

## 実装計画書の一覧
| ID | 件名 | 文書 | 書くファイル | 依存 | 並列 |
|---|---|---|---|---|---|
| U05 | Mac 専用の機能を削る | `docs/plan/unit-U05-remove-mac-only.md`（waggle:rpwWZ4n8） | `rbrowser.py`・`static/rbrowser.*`（消す）・`main.py`・`papers.py`（遠隔ブラウザの部分）・`usage.py`・`static/papers.js`（2 か所）・`usage.js`・`index.html`・`sw.js`・テスト・`apps/web/launchd/`（消す）・`crates/sai-paper-core/src/paths.rs`・`docs/integration.md` | ガイド 2 | - |
| U06 | アカウントとログイン（サーバ） | `docs/plan/unit-U06-accounts-server.md`（waggle:ptDGT8xi） | `apps/web/src/sai_web/` の `registry.py`・`auth.py`（新規）・`store.py`・`main.py`・`runs.py`・`usage.py`・`home.py`・`papers.py`・`control.py`（覚えだけ）・`webcli.py`・`config.py`・テスト 2 本・`docs/integration.md` | U05 | 可（U07 と） |
| U07 | ログイン・アカウント・管理の画面 | `docs/plan/unit-U07-accounts-screens.md`（waggle:Wv4VphqX） | `static/account.js`（新規）・`app.js`（`apiFetch` と起動の 1 行）・`home.js`（ボタン）・`index.html`・`sw.js`・`style.css`・`test_account.js` | U05 | 可（U06 と） |

## 実行順序
```
U05 ─▶ U06 ∥ U07
```
- U05 を先にする（`main.py`・`papers.py`・`usage.py`・`index.html`・`sw.js` を U06・U07 も書くため）。
- U06 ∥ U07（書くファイルが重ならない）。U07 は U06 の住所（C5'。U07 の計画書に全文）だけに頼る。
- U07 の AC3（実際のサーバで画面を確かめる）は、U07 の作業用コピーに U06 が無いので、**両方をマージした後に kantoku が行う**。U07 の worker には、node のテストと単独モードの画面（AC4）までを行うよう追加指示で伝える。

## 所有権の表
| ファイル | U05 | U06 | U07 |
|---|---|---|---|
| `apps/web/src/sai_web/*.py` | 書く（遠隔ブラウザ・5 時間枠） | 書く ※U05 の後 | 読む |
| `apps/web/src/sai_web/static/*` | 書く（遠隔ブラウザ・5 時間枠） | 読む | 書く ※U05 の後 |
| `crates/sai-paper-core/src/paths.rs`・`apps/web/launchd/` | 書く | - | - |
| `docs/integration.md` | 書く | 書く ※U05 の後 | - |

## 共有契約
- C4 多人数モードの置き場（U06 の計画書に全文。ガイド 4・5 も使う）。**プロジェクトごとの HOME（`<U>/<user>/homes/<user>-<project>/`）を持ち、コンテナにはプロジェクトの root と HOME だけを見せる。**
- C5 今の利用者（`registry.current_user` ほか。U06 の計画書）。個人のトークン（`Bearer sai_…`）でも入れる。
- C5' アカウントの住所（U06 が作り U07 が使う。U07 の計画書に全文）。
- 画面の区画（`acSections`・`acAdminSections`。U07 が作り、ガイド 4 の U11 が使う）。

## 全体の完了条件
| 条件 | 担当 ID | 確認方法 |
|---|---|---|
| Mac 専用の機能が残らない | U05 | U05 の AC1 |
| 単独モードが変わらない | U05・U06・U07 | Web のテスト（JS・Python）がすべて通る。単独モードの画面 |
| メールアドレスでのログイン・初回変更・管理者の作成・個人のトークンが動く | U06・U07 | U06 の AC2、統合後の U07 の AC3 |
| 他人のプロジェクト・実行・会話が見えない | U06 | U06 の AC3 |

## 統合と検証
- U05 → U06 → U07 の順にマージし、毎回 Rust（U05 は `paths.rs` を書く）と Web のテストをすべて回す。
- 統合後、kantoku が 8299 番に多人数モードのサーバ（`SAI_HOME`・`SAI_WEB_USERS_ROOT` は一時フォルダ、`SAI_WEB_COOKIE_SECURE=0`、`SAI_WEB_NO_WATCH=1`）を起こし、U07 の AC3 をブラウザで行う。alice と bob を作り、互いのプロジェクトが見えないことも確かめる（一時フォルダに C4 の形でプロジェクトと実行の書き出しを置く）。サーバは止め、一時フォルダを消す。本番の 8200 番は止めない。

## 決定事項と理由 / 採用しなかった案
- ログインの ID は大学のメールアドレス、フォルダ名とコンテナ名は管理者が決める短い名前（利用者の決定）。
- アカウントは管理者が作る。招待コード・Tailscale の身元ヘッダは採らない。
- パスワードは `hashlib.scrypt`、Cookie は HttpOnly・SameSite=Lax、CSRF は Origin の照合。依存を増やさない。
- 利用者の区別は `ContextVar` と置き場の関数の差し替え。ミドルウェアで入れる（同期の依存関数の `ContextVar` は住所の関数に届かないため）。
- 覚え（キャッシュ）はすべて利用者の名前をキーに含める。
- 多人数モードの登録簿はフォルダを数える（プロジェクトは Web だけが作る）。
- Mac 専用の機能を削る（利用者の決定）。

## リスクと対処
- ミドルウェアの `ContextVar` が同期の住所に届かない: U06 の AC4 で確かめる。届かなければ、純粋な ASGI のミドルウェアにする（kantoku が決めてよい）。
- 既存のテストが `settings.runs_dir` を直に差し替えている: `SAI_WEB_RUNS_DIR` を優先する形を保てば通る。通らないときは、テストの差し替え方を直してよい（理由を報告に書く）。

## 参照文書
- 要件定義書: `docs/plan/requirements.md`（waggle:y7EoLAgV）
- 調査 X3（認証・置き場・Mac 依存）: `docs/plan/research/X3-multiuser.md`
- 調査 E3（Mac 専用の機能の削り方）: `docs/plan/research/E3-mac-only-removal.md`
