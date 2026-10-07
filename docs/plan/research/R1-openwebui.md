---
block: web-research
run: 439
description: 調査報告書 R1: Open WebUI を headless エージェントの会話画面にする方法
---

# ウェブ調査報告 R1: Open WebUI を headless エージェントの会話画面にする方法

## 前提（確かめた版）

- 調査日: 2026-10-07。
- 対象: open-webui/open-webui 本体。最新リリース **v0.11.4**（2026-09-21 公開）。`package.json` の `version` が `0.11.4` であることを main ブランチの浅い clone で確認（https://github.com/open-webui/open-webui 、リリース一覧 https://github.com/open-webui/open-webui/releases ）。
- 公式ドキュメント: https://docs.openwebui.com （2026-10 時点の内容）。
- 行番号付きの引用は main ブランチ（v0.11.4 時点）のソースに対するものである。

## 問いと答え

### Q1: 外部バックエンドをつなぐ方法の比較

#### (a) OpenAI 互換 API を自作サーバが出す方式（「接続」で登録）

- 管理者画面 Admin Settings → Connections → Manage OpenAI API Connections に自作サーバの URL（例 `http://agent-server:9000/v1`）と API キーを登録する。「OpenAI API 互換」なら何でも接続できる（Discussion #9022、https://github.com/open-webui/open-webui/discussions/9022 ）。`/v1/models` でモデル一覧を返し、`/v1/chat/completions` で SSE ストリームを返せばよい。
- **chat_id は渡る。** 環境変数 `ENABLE_FORWARD_USER_INFO_HEADERS=true` を付けると、chat completion 要求に次のヘッダが付く（`backend/open_webui/routers/openai.py:179-182`。`metadata.get('chat_id')` があるとき `X-OpenWebUI-Chat-Id` も付く）:
  - `X-OpenWebUI-User-Name` / `X-OpenWebUI-User-Id` / `X-OpenWebUI-User-Email` / `X-OpenWebUI-User-Role`（`backend/open_webui/env.py:993-996`、名前は `FORWARD_USER_INFO_HEADER_*` で変えられる）
  - `X-OpenWebUI-Chat-Id`（`backend/open_webui/env.py:999`）
  - さらに `FORWARD_USER_INFO_HEADER_JWT_SECRET` を設定すると、分割ヘッダの代わりに署名付き JWT（既定 300 秒有効）が 1 つ飛ぶ（`backend/open_webui/env.py:1001-1010`）。
- **Message-Id は chat completion には付かない。** `X-OpenWebUI-Message-Id` が付くのは外部ツール（OpenAPI/MCP ツールサーバ）への呼び出しのときだけ（`backend/open_webui/utils/middleware.py` の外部ツール呼び出し経路、公式 Events ドキュメント https://docs.openwebui.com/features/extensibility/plugin/development/events/ ）。
- **chat_id が要らないときでも渡せる:** 接続ごとの custom headers に `{{CHAT_ID}}`・`{{MESSAGE_ID}}`・`{{USER_MESSAGE_ID}}`・`{{USER_ID}}`・`{{USER_EMAIL}}`・`{{USER_GROUPS}}` などのプレースホルダを書ける。これは `ENABLE_FORWARD_USER_INFO_HEADERS` 無しでも chat completion に適用される（`backend/open_webui/utils/headers.py` の `parse_custom_headers`、`routers/openai.py:215-216`）。`{{MESSAGE_ID}}` は completions 経路でも metadata から取れるので、custom header 経由ならメッセージ ID も自作サーバに渡せる。
- ストリーム: SSE（`data: {...}` チャンク、`delta.content`）で普通に流せる。Open WebUI は OpenAI 互換の SSE をそのまま解釈する。
- 限界: OpenAI プロトコルには「status イベント」がないため、ツール実行の途中経過を専用 UI に出すことはできない。後述の `<details>` ブロックか `reasoning_content` を使う。

#### (b) Pipe（Functions）で自作サーバを呼ぶ方式

- 管理者が Open WebUI 内に Python の `Pipe` クラス（Function）を登録すると、それが 1 つの「モデル」として画面に出る。`pipe()` の中で自作サーバへ httpx などで接続し、返答を yield すればストリーミングになる（公式 Pipe ドキュメント https://docs.openwebui.com/features/extensibility/plugin/functions/pipe/ ）。
- 情報の引き渡しが最も豊富。`pipe()` のシグネチャに名前を書くだけで次が注入される（Pipe ドキュメント「Special Arguments」。ソースでは `backend/open_webui/utils/middleware.py:2543-2555` の `extra_params`）:
  - `__chat_id__` / `__message_id__` / `__metadata__`（`chat_id`・`message_id`・`session_id` を含む dict）→ **会話 ID は確実に取れる**
  - `__user__`（利用者の dict。`valves` 込み）
  - `__event_emitter__` / `__event_call__`（下の Q3・Q4 の全イベントが使える）
  - `__task__`（タイトル生成などの裏タスクかどうか）
- 欠点: コードが Open WebUI のプロセス・DB の中に置かれる（管理者が Function として貼る）。自作サーバが FastAPI で独立しているなら、Pipe は「薄い通訳」にして本丸は自作サーバに置く形になる。Function の登録・更新は管理 API（`/api/v1/functions/*`）からも可能（要確認: 今回落としていない。管理画面から貼る前提なら問題ない）。

#### (c) その他

- **Pipelines**（別プロダクト open-webui/pipelines）: pipe を Open WebUI の外の独立プロセスで動かす、OpenAI 互換の前段プロキシ。公式ドキュメントに残っている（https://docs.openwebui.com/features/extensibility/pipelines/ ）。ただし (b) とほぼ同じことが (a)+(b) の組合せでもできるので、新規導入で必須ではない。
- **MCP**: v0.6.31 から MCP（Streamable HTTP）サーバをネイティブに「外部ツール」として登録できる（https://docs.openwebui.com/features/extensibility/mcp/ ）。ただしこれは「ツール」であり、エージェント本体（会話の相手）を置き換えるものではない。公式の注意書きに「Open WebUI の UI event システムはネイティブ Python ツールでのみ使える」とある一方、Events ドキュメントでは外部ツールにも REST でのイベント発火を認めている（下の Q3）。旧来の mcpo プロキシ（https://docs.openwebui.com/features/extensibility/plugin/tools/openapi-servers/mcp/ ）は、MCP を OpenAPI に変換するもので、これもツール用途。
- **OpenAPI Tool サーバ**: 同じく「ツール」用途。自作エージェントをツールとして呼ばせる形で、会話の主役にはならない。
- **Direct Connections**（https://docs.openwebui.com/features/chat-conversations/direct-connections/ ）: 利用者のブラウザが API プロバイダに直接つなぐ機能。利用者ごとに API キーを要求するもので、今回の「サーバ側エージェント」用途には合わない。

### Q2: 会話の続きの扱い（全履歴は毎回送るのか・セッション対応）

- **はい、全履歴が毎回来る。** OpenAI プロトコルではフロントエンドが保持する `messages` 全体を送る。さらに保存済み chat では、Open WebUI サーバが DB からメッセージを再構築して `form_data['messages']` を作り直す（`backend/open_webui/utils/middleware.py` の `MESSAGE_REPLAY_KEYS = ('id','role','content','output','files',...)` と直後の再構築処理）。つまり自作サーバは毎ターン「これまでの全部」を受け取る。
- **セッション対応は chat_id を鍵にするのが正解。** `messages` の内容を鍵にすると履歴編集や分岐で壊れる。`X-OpenWebUI-Chat-Id`（(a) 方式、名前は `FORWARD_SESSION_INFO_HEADER_CHAT_ID` で変更可）か、Pipe の `__chat_id__` / custom headers の `{{CHAT_ID}}` を取り、自作サーバ側で「chat_id → エージェントセッション（`--resume` 用のセッション ID）」の対応表を持つ。
- 1 通目から chat_id は付く。UI から送る通常の chat completion には必ず `metadata.chat_id` が入る。**temporary chat（履歴を残さない一時チャット）では空になる**（`backend/open_webui/utils/middleware.py:2394` で `metadata['chat_id'] = ''`）。temporary chat を禁止する設定と併用するのが安全。
- 注意: 同じ chat 内で利用者がメッセージを編集すると分岐（ツリー）が生まれ、chat_id は同じまま履歴が変わる。`{{USER_MESSAGE_ID}}` も渡しておいて、自作サーバ側で「初見の user message id か」を見ると再生成・分岐を検出できる（推測: この設計の確かめは実装時に必要）。

### Q3: 途中経過・ツール実行・「考え中」の表示

- **(a) OpenAI 互換経路で使えるもの**:
  - `<details>` ブロック: 本文に `<details>...</details>` を含めると、フロントエンドがこれを独立した折りたたみブロックとして抽出・描画する（`src/lib/components/chat/Messages/ContentRenderer.svelte:20-60`、`ResponseMessage.svelte:370-377`）。ツール実行の経過をこの中に追記してストリームすれば、折りたたまれた作業ログとして見せられる。
  - reasoning: SSE の `delta.reasoning_content` / `delta.reasoning` / `delta.thinking` を Open WebUI が reasoning として処理する（`backend/open_webui/utils/middleware.py:3748`）。`<think>...</think>` タグも認識する（`middleware.py:243`）。これが「考え中」の表示になる。
  - status イベントは OpenAI プロトコルには無いので使えない。
- **(b) Pipe 経路で使えるもの**（公式 Events ドキュメント https://docs.openwebui.com/features/extensibility/plugin/development/events/ ）:
  - `status`: `{description, done, hidden}`。`done:false` の間は UI に shimmer アニメが出る。**必ず最後に `done:true` を送る**（送らないと処理中の見た目のまま残る）。`statusHistory` として DB に永続化される。
  - `chat:message:delta` / `message`、`source`/`citation`、`files`、`embeds`（Rich UI、`data.replace: true` で差し替え可能）、`notification`。
  - 注意: Pipe では本文は `yield` / `return` で返し、イベントは補助に使う。本文をイベントのみで流すと、完了時のフロントエンドの全履歴保存が上書きして空になることがある（Events ドキュメントに明記）。
- **外部ツール（OpenAPI/MCP）からもイベントを発火できる**: `ENABLE_FORWARD_USER_INFO_HEADERS=true` で付与される `X-OpenWebUI-Chat-Id` / `X-OpenWebUI-Message-Id` を使って `POST /api/v1/chats/{chat_id}/messages/{message_id}/event` を呼ぶ。認証には Open WebUI の API キーが必要で、管理者（サービスアカウント）キー 1 つで全ユーザの chat に書ける（Events ドキュメント「External Tool Events」）。ただし `input` / `confirmation` など双方向イベントは外部からは不可。

### Q4: エージェントからの質問に利用者が答える方法

- **選択肢つきの問いかけは、ネイティブ Python 関数（Pipe/Tool/Filter）からなら可能。** `__event_call__` で:
  - `{"type": "input", "data": {"title": ..., "message": ..., ...}}` — テキスト入力ダイアログ。パスワード型もある。
  - 選択肢は `"input": {"type": "select", "options": ["a", "b"]}` を data に入れると選択式になる（Events ドキュメント「Select Input」。キャンセルすると `false` が返る）。
  - `{"type": "confirmation", ...}` — OK/キャンセル。
  - 双方向は WebSocket が要るため、**タブを閉じると `{"error": "Client session disconnected."}` が返る**。タイムアウトは環境変数 `WEBSOCKET_EVENT_CALLER_TIMEOUT`（秒）で制限できる（未設定なら永久待ち）。
- **(a) OpenAI 互換 API のバックエンドからは不可能。** 双方向イベントの仕組みは Open WebUI 内部のものなので、外部 HTTP バックエンドには経路がない。この場合は「エージェントが本文で質問を書き、利用者が次のメッセージで返答する」形になる。選択肢は Markdown の箇条書きで提示し、「番号で返してください」とするのが現実的（推測: 自作サーバ側で pi の ask_user / Claude Code の選択肢をテキスト化する実装が必要）。
- 中間案として、自作サーバを「OpenAPI ツールサーバ」としても登録しておき、`ask_user` 相当を通常の返信で実現する構成も考えられるが、質問待ちで generator を止める挙動との整合は要確認。

### Q5: 停止ボタンを押したとき、バックエンドに何が届くか

- **特別な「停止メッセージ」は届かない。HTTP 接続が切れるだけ。**
- UI の停止は `POST /api/tasks/chat/{chat_id}/stop`（`backend/open_webui/main.py:2160`）とブラウザ側の fetch 中断。サーバ側ではストリーミング処理が `asyncio.CancelledError` を受け、`response.body_iterator.aclose()` を呼んで**上流（自作サーバ）への接続を解放する**（`backend/open_webui/utils/middleware.py:6591-6637`。コメントに "release the upstream connection" とある）。部分出力は Open WebUI 側の DB に保存される。
- したがって、エージェント（`claude -p ...` のプロセス）を本当に止めるには、自作サーバが HTTP クライアント切断（Starlette/Uvicorn の `http.disconnect`）を検知して子プロセスに割り込みを送る実装が必要（推測: 実装時の確認事項。切断検知の確実性は ASGI サーバの挙動次第）。

### Q6: 利用者と権限（model ごとの見える範囲・利用者情報）

- アカウント: 管理画面から作る、サインアップを許す、または**トラストヘッダ SSO**。環境変数 `WEBUI_AUTH_TRUSTED_EMAIL_HEADER`（このヘッダの値をメールアドレスとして自動ログイン）と `WEBUI_AUTH_TRUSTED_NAME_HEADER` がある（`backend/open_webui/env.py:802-803`）。既存の認証付きリバースプロキシの背後に置く場合に使う。
- **model の見える範囲は API から設定できる。** Workspace model（プリセット）に access 制御を付け、private / グループ / 全員を切り替えられる。API は `POST /api/v1/models/create`・`/model/update`・`/model/access/update`（`backend/open_webui/routers/models.py:285, 932, 1035`。v0.11.x では `ModelForm.access_grants` が制御フィールド、`backend/open_webui/models/models.py:169-176`）。グループの作成・ユーザ追加・削除も API がある: `POST /api/v1/groups/create`・`/id/{id}/users/add`・`/id/{id}/users/remove`（`backend/open_webui/routers/groups.py:60, 222, 262`）。
- **「利用者ごとに見える model（＝プロジェクト）を分ける」構成**: 自作サーバの各 model を base にした workspace model をプロジェクトごと（または人ごと）に作り、`access_grants` で公開範囲を絞る。プライベートな項目は所有者と共有された人にしか見えない（公式 Roles ドキュメント https://docs.openwebui.com/features/authentication-access/rbac/roles/ ）。
- 自作サーバへの利用者通知: `X-OpenWebUI-User-Id/Email/Name/Role`（`ENABLE_FORWARD_USER_INFO_HEADERS`）か、custom headers の `{{USER_ID}}`・`{{USER_EMAIL}}`・`{{USER_GROUPS}}`。JWT 方式（`FORWARD_USER_INFO_HEADER_JWT_SECRET`）なら署名検証で偽装を防げる。平文ヘッダは「情報の付与」であって本人確認ではないので、**自作サーバは Open WebUI からしか到達できない内網/Tailnet に置く**こと（Events ドキュメントにも、このヘッダに鍵は含まれないと明記）。

### Q7: ファイル添付（PDF など）を自作サーバへそのまま渡せるか

- **既定では RAG で処理される。** 添付ファイルは抽出・チャンク化され、検索結果がコンテキストとしてプロンプトに注入される。生のファイルは OpenAI 互換バックエンドには送られない。送出直前に `messages` から `files` フィールドは取り除かれ、画像だけが `image_url` パートに変換されて残る（`backend/open_webui/utils/middleware.py:2455-2480` 付近）。
- **全文をそのまま入れる設定はある**: 管理者設定の `rag.bypass_embedding_and_retrieval`（埋め込み・検索を省いて全文を注入。`backend/open_webui/retrieval/utils.py:1386, 1506`）と環境変数 `RAG_FULL_CONTEXT`（`backend/open_webui/config.py:979`）。ただしこれでも渡るのは**抽出済みテキスト**であり、PDF のバイト列そのものではない。
- エージェントに原物（PDF ファイル）を渡したいなら、(1) 自作サーバが別経路でファイルを受ける、(2) Open WebUI の files API / knowledge に上げた実ファイルのパスや URL を model の system prompt 経由で示す、などの設計が必要（推測: 実装時に要設計）。

### Q8: 動かし方・資源・Tailscale・ライセンス

- Docker イメージ: `ghcr.io/open-webui/open-webui:main`（README のインストール手順、https://github.com/open-webui/open-webui#readme ）。データはコンテナの `/app/backend/data` に置かれるので、ボリュームとしてマウントする（README の docker run 手順に「データのマウントが不可欠」と明記）。v0.11.4 で、ローカルモデル関連を除いた slim イメージ（従来比約 89% 小）が出た（リリースノート、https://github.com/open-webui/open-webui/releases ・whatsnew.fyi の抄録）。
- 資源: FastAPI + SvelteKit の単一コンテナ。外部 LLM だけ相手にするならメモリ 1〜2GB 程度で足りる目安（推測。埋め込みモデルをローカルで動かすと増える）。
- Tailscale: tailnet 内の HTTP で直接運用する場合、ブラウザの Secure Cookie や PWA/Service Worker の要件で不具合が出る可能性がある（推測: `tailscale serve` で HTTPS 化して使うのが無難。Open WebUI 側の `WEBUI_URL` にも実 URL を設定する）。公式ドキュメントの HTTPS 要求の記載位置は今回確認できなかった（要確認）。
- ライセンス: **BSD-3-Clause + ブランド条項**。ブランド表示（名称・ロゴ等）の変更・除去は、(i) 30 日間で 50 人以下の利用、(ii) 書面許可、(iii) エンタープライズライセンス、のいずれかの下でしか認められない（`LICENSE` 第 4 条、https://github.com/open-webui/open-webui/blob/main/LICENSE ）。研究室の数人では (i) に当てはまるので自前画面に見せたくなっても名前を消す余地はあるが、そのまま使うのが確実。別途、アップストリームへの送信時に `HTTP-Referer` / `X-Title` を付与するコメントがコードにある（`routers/openai.py:173-178`、OpenRouter 向けの固有処理）。

### Q9: Open WebUI 以外（LibreChat など）

LibreChat も `librechat.yaml` で OpenAI 互換の custom endpoints を設定でき、マルチユーザ・権限も備える（https://www.librechat.ai/docs/quick_start/custom_endpoints ）。ただし今回の用途で決定的に有利な点は見当たらない。Open WebUI には (1) バックエンドから UI へ status 等を push できる公開 REST エンドポイント（`/api/v1/chats/{id}/messages/{id}/event`）、(2) `chat_id` / ユーザ情報ヘッダの転送、(3) model 単位の access_grants API がそろっており、headless エージェントの画面として必要な接点が公式ドキュメント化されている。LibreChat 側で (1) に相当する公開 API があるかは今回確認できなかった（要確認）。他に LobeChat・AnythingLLM などがあるが、いずれも「外部エージェントを会話の主体にする」ことへの明示的な対応は見つからなかった。

## 推奨の接続方式

**推奨: (a) OpenAI 互換 API + `ENABLE_FORWARD_USER_INFO_HEADERS=true` を主経路にし、会話対応は `X-OpenWebUI-Chat-Id` で行う。**

理由:

1. 自作サーバ（FastAPI）が OpenAI 互換の 2 エンドポイント（`/v1/models`・`/v1/chat/completions` の SSE）を出すだけでよく、Open WebUI 側にコードを置かない。Pipe 方式だと Open WebUI の DB に Python を登録・更新し続ける運用が発生する。
2. chat_id・ユーザ情報・（custom headers なら）message_id が要求ヘッダとして確実に届き、`claude -p --resume` / `pi --mode json` のセッション対応（chat_id → セッション ID の対応表）が作れる。履歴は毎回全量で届くが、headless エージェント側は自セッションを持つので、messages は「使わず無視」してよい。
3. 途中経過は `<details>` ブロック（折りたたみの作業ログ）と `reasoning_content`（考え中）で、OpenAI プロトコルのまま見せられる。
4. 質問の選択肢ボタンはこの方式では出せないが、Pipe 方式でも「外部 HTTP バックエンドが止まって答えを待つ」構成は作りにくく（event_call は Open WebUI 内の Python からしか呼べない）、実用上は本文での質問・返答で足りる（推測）。どうしても選択肢ボタンが必要になった段階で、Pipe の薄い通訳（自作サーバへ転送しつつ `__event_call__` で select を出す）へ段階的に足せる。

補足の設計注意: temporary chat の禁止、`messages` 無視の明示、停止時のクライアント切断検知による子プロセス停止、`rag.bypass_embedding_and_retrieval`（または添付を別経路で渡す設計）の検討。

## 分からなかったこと

- Functions（Pipe）の管理 API（`/api/v1/functions/*`）での自動登録の可否。main ブランチの `backend/open_webui/routers/functions.py` で確認できる（要確認: 今回は clone を消した後に確認したくなった）。
- `{{MESSAGE_ID}}` custom header が models 一覧取得（`GET /v1/models`）でも適用されるか。completions では適用されることをコードで確認済みだが、models 経路（`routers/openai.py:107`）はユーザ情報ヘッダのみを確認。models は静的応答で問題ないはず（推測）。
- Tailscale・HTTPS についての公式ドキュメントの現行の記載位置。
- LibreChat に status push の公開 API があるか。

