# 実装計画書 U10: コンテナからの内部の要求・質問の持ち主・Usage の鍵を利用者ごとにする

## 目的
1. コンテナの中のフック・拡張から Web への内部の要求（セッションの登録・質問の登録と答えの取得・解決の知らせ）を、そのプロジェクトの合言葉で受け、どの利用者の物かを記録する。
2. 質問・登録を、多人数モードでは持ち主の利用者にだけ見せる。
3. Usage の OpenRouter・DeepSeek の鍵を、Web のプロセスの環境変数ではなく、今の利用者の `secrets.env` から読む。

## 背景
- sai-tree は sai-moon を複製した Linux の多人数サーバ版である。
- 今の内部の住所は「127.0.0.1 から来たか」だけで守っている（`apps/web/src/sai_web/main.py:75-79` `require_local`）。多人数のサーバでは、どのコンテナからも Web に届くので、これでは誰の質問か分からない。`GET /api/questions/{qid}` は守りが無い（調査 X3 Q1）。
- コンテナには `SAI_WEB_INTERNAL_TOKEN`（プロジェクトごとの合言葉）が付き（C8）、Web の DB の `project_tokens` に SHA-256 と持ち主がある（U09 が表と `save_project_token`・`find_project_token` を作った）。フックと拡張はその値を `Authorization: Bearer` で送る（U02 の C3）。
- 質問・登録の表（`apps/web/src/sai_web/store.py`）には持ち主の列が無い。Home の集計は全部の質問を並べる（`apps/web/src/sai_web/home.py` の `_home_questions` → `store.list_open_questions`）。
- U06 のミドルウェアは、個人のトークン（`Bearer sai_…`）と Cookie で `registry.current_user` を入れる。この作業は、そこに「プロジェクトの合言葉 → 持ち主」を足す。

## 前提（着手時点のコードの状態）
- U06（ミドルウェアで `registry.current_user` を入れる）・U08（イメージ）・U09（`project_tokens`・`containers.read_keys`・C8）が終わっている。
- 内部の住所: `POST /api/register`・`POST /api/questions`・`POST /api/questions/{qid}/resolved`・`GET /api/questions/{qid}`（承認の住所は U03 が消した）。
- Usage の OpenRouter の鍵: `apps/web/src/sai_web/usage.py:74-77` `_key_for`（Web のプロセスの環境変数）。DeepSeek: `usage.py:161`。

## 変更内容

### 1. 内部の合言葉 — `main.py` のミドルウェア（U06 が作った物に足す）
- 多人数モードで、要求に `Authorization: Bearer <値>` があり、値が `sai_` で始まらず、`store.find_project_token(sha256(値))` が見つかれば、`registry.current_user.set({"name": <owner>, "email": "", "is_admin": False})` と `request.state.internal_project = <project>` を入れる（Cookie より先に見る）。見つからなければ何もしない（→ 内部の住所は 403 になる）。
- `require_local`（`main.py:75`）を `require_internal(request)` に変える:
  - 単独モード: 今のまま（127.0.0.1 / ::1 だけ）。
  - 多人数モード: `request.state.internal_project` が無ければ 403「内部専用です」。
- 内部の 3 本に加え、`GET /api/questions/{qid}` にも `require_internal` を付ける。多人数モードでは、その質問の `owner` が今の利用者でなければ 404。
- 内部の合言葉で入った要求は、内部の 4 本以外の住所では 403「内部専用の合言葉では使えません」にする（コンテナの中のエージェントが、持ち主の Web の API を勝手に使えないように）。

### 2. 持ち主の列 — `apps/web/src/sai_web/store.py`
- `registrations`・`questions` に `owner TEXT NOT NULL DEFAULT ''` を足す（`init_db` で `PRAGMA table_info` を見て、無ければ `ALTER TABLE … ADD COLUMN`。既存の DB を壊さない）。
- 保存する関数（`save_registration`・`save_question`）は、`registry.current_user` があればその名前を `owner` に入れる（単独モードは空文字のまま）。
- 読む関数（`list_open_questions`・`list_questions_for_session`・`get_question`・`get_registration` など、読む物すべて）: 多人数モードでは `owner = 今の利用者` で絞る。単独モードは今のまま。絞り込みは store の中で行い、呼び出し側（`home.py`・`questions.py`・`main.py`・`watch.py`）は変えない。
- `watch.py`（U03 が作った見張り）と通知は、利用者がいない（バックグラウンドの）処理である。多人数モードでは、見張りが利用者ごとに `registry.current_user` を入れてから読むようにする（`store.list_users` を回す）。通知の購読（`subscriptions`）にも `owner` を足し、その人の購読にだけ送る。

### 3. Usage の鍵 — `apps/web/src/sai_web/usage.py`
- `_key_for`（`:74`）: 多人数モードでは、今の利用者の `secrets.env`（`containers.read_keys`）から読む。`DEEPSEEK_API_KEY`（`:161`）も同じ（`secrets.env` に無ければ無し）。単独モードは今のまま。

### 4. 文書
- `docs/integration.md`: 内部の住所の守り（合言葉）・持ち主の絞り込み・内部の合言葉で使えない住所を書く。

## 共有契約（他の作業と共有する取り決め。worker は変えない）

**C3 フック・拡張と Web の約束**（U02。全文）
- Web の場所: `SAI_WEB_URL` → `http://127.0.0.1:${SAI_WEB_PORT:-8200}`。合言葉: `SAI_WEB_INTERNAL_TOKEN` があれば `Authorization: Bearer <値>`。住所は `POST /api/register`・`POST /api/questions` → `{"ok":true,"id"}`・`GET /api/questions/{qid}`・`POST /api/questions/{qid}/resolved`。

**C5 今の利用者**（U06。この作業で足す物）
- 内部の合言葉で入った要求: `registry.current_user` は持ち主、`request.state.internal_project` はプロジェクトの名前。
- 個人のトークンは `sai_` で始まる。内部の合言葉は `sai_` で始まらない（`secrets.token_urlsafe(32)`。もし `sai_` で始まる値が出たら作り直す — U09 の `create_container` に 1 行足す）。

## ファイルの所有権
- 書いてよい: `apps/web/src/sai_web/main.py`（ミドルウェア・`require_internal`）・`store.py`（持ち主の列と絞り込み）・`watch.py`・`notify.py`（利用者ごとの見張りと購読）・`usage.py`（`_key_for` と DeepSeek）・`containers.py`（合言葉の作り直しの 1 行）・`apps/web/tests/test_internal_auth.py`（新規）・`docs/integration.md`
- 読むだけ: `apps/web/src/sai_web/registry.py`・`auth.py`・`apps/web/hooks/`
- 触らない: `apps/web/src/sai_web/static/`（U11）・`crates/`

## 完了条件
- [ ] AC1: `apps/web/tests/test_internal_auth.py`（多人数モード・一時フォルダ）: alice のプロジェクトの合言葉で `POST /api/questions` → `owner` が alice。合言葉無しは 403。bob の Cookie で `GET /api/questions?session_id=<aliceの>`・`/api/home` に alice の質問が出ない。bob のプロジェクトの合言葉で alice の質問の `GET /api/questions/{qid}` は 404。alice のプロジェクトの合言葉で `GET /api/projects` は 403。
- [ ] AC2: 単独モードで、今の質問のテスト（`test_questions.py` ほか）が変えずに通る。既存の DB（`owner` の列が無い）を開いても動く（テストで古い形の DB を作って確かめる）。
- [ ] AC3: 通知: alice の質問の通知は alice の購読にだけ送られる（偽の送り先で確かめる）。
- [ ] AC4: Usage の OpenRouter が `secrets.env` のキーを使う（偽の HTTP で、送られた鍵の値を確かめる）。
- [ ] AC5: 既存の Web のテストがすべて通る。

## 検証
```bash
cd ~/workdir/Sai/sai-tree
for f in apps/web/tests/test_*.py; do PYTHONPATH=apps/web/src apps/web/.venv/bin/python "$f" || exit 1; done
for f in apps/web/tests/test_*.js; do node "$f" || exit 1; done
```

## 決定事項と理由
- 内部の要求はプロジェクトごとの合言葉で受ける — 理由: どのコンテナからの要求かが分かり、持ち主を記録できる。127.0.0.1 の判定は多人数のサーバでは意味をなさない。
- 内部の合言葉では内部の住所しか使えない — 理由: コンテナの中のエージェントは合言葉を環境変数で読める。持ち主の API を使えると、エージェントがプロジェクトの外（持ち主の別のプロジェクト）に触れる。
- 持ち主の絞り込みは store の中で行う — 理由: 読む場所は多い（Home・Chats・質問・通知）。1 か所で絞れば漏れが出にくい。

## 要確認
- なし。
