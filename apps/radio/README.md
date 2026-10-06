# sai-radio

論文をランニング中などに音声で聞け，聞きながら音声で質問もできるアプリです．
画面は **sai-web**（Papers タブ → Radio）から使います．sai-radio 自体は画面の無い裏方サーバーです．

## これはどんなアプリ？

1. パソコンに論文のPDFを渡すと，日本語のラジオ原稿風の要約ができ，それを音声で読み上げてくれます．
2. sai-web（iPhone のホーム画面のアイコン）の Papers タブ → Radio から，その音声を再生できます．
3. 聞いている途中で「ここどういう意味？」と声で質問すると，その場所の内容をふまえた回答が音声で返ってきます．

論文の登録方法は3通りあります:
- **論文管理アプリ「sai-paper」から**: `sai-paper paper radio <論文番号>` を実行するだけ（sai-paper を使っている場合）
- **sai-web から**: Radio 画面で「音声を作る」を押す
- **コマンドラインから**: `python -m sai_radio.ingest`（後述）

`sai-paper paper radio`と sai-web の「音声を作る」は，どちらも同じ音声化の窓口
（`POST /api/papers/from-extract`）を使います．

## 使っている場所（2台 + Google 構成）

| 場所 | 役割 |
|---|---|
| **iPhone** | 論文を選んで再生する．画面は **sai-web**（Papers タブ → Radio）で，sai-radio へは中継でつながる |
| **Mac** | 論文の登録・要約・音声生成を行う本体サーバー．質問用の「合鍵（トークン）」の発行も担当 |
| **Google** | 質問の声を聞き取り，回答の文章と音声を作る（Gemini Live API） |

Mac と iPhone は同じ **Tailscale**（VPN）ネットワークに参加しているので，外出先からでもつながります．
質問の音声は iPhone から Google に直接送られます．Mac を経由しないので速く，Mac の負担もかかりません．

音声を作る部分（テキスト→声）は，Macで動く **AivisSpeech Engine** というアプリが担当します．
要約や質疑応答の文章を作る部分（LLM）は，OpenRouterなどのAPIを直接呼び出します．

以前はこれらの処理を「open-notebook」という別アプリに任せていましたが，現在は使っていません．
sai-radio単体で完結します．

## 初期設定（初回だけ）

使い始める前に，以下を1回だけ設定します．

### 1. `.env`ファイル

プロジェクトルートに`.env`ファイルを作ります．書き方の見本が `.env.example` にあるので，
コピーして使ってください:
```bash
cp .env.example .env
```
その後，最低限これだけ書きます:
```
LLM_MODEL=openai/gpt-4o-mini
```
（要約・質問応答・ポッドキャストの台本作りに使うLLMのモデル名．`<プロバイダ>/<モデル>`の形式．
上の例は仮の値なので，好みや予算に応じて変えてください）

さらに，LLMプロバイダのAPIキーを**環境変数として**設定します（`.env`でも，シェルの`export`でも
どちらでも構いません）．OpenRouterを使う場合:
```
OPENROUTER_API_KEY=sk-xxxxxxxx
```
声質（AivisSpeechの話者ID）を解説モードで使う場合は，`.env`に`AIVISSPEECH_SPEAKER=<数字>`も
書きます．IDはAivisSpeech Engineを起動した状態で`http://127.0.0.1:10101/speakers`を
ブラウザで開くと一覧で確認できます．

sai-radioに簡単なアクセス制限をかけたい場合は`SAI_RADIO_TOKEN=<好きな文字列>`も設定できます
（未設定なら誰でもアクセスできる開発用の状態のままです）．

質問機能を使うには，Google のAPIキーも `.env` に書きます（Google AI Studio で発行できます）:
```
GEMINI_API_KEY=AIzaSy...（自分で取得したキー）
```
詳しい項目の一覧は `.env.example` を参照してください．

### 2. ポッドキャストモードの設定ファイル

`config/episode_config.json`と`config/speakers_config.json`の2ファイルが既に用意されています
（中身は仮の値が入った状態）．ポッドキャストモード（2人が対話する形式）を使う前に，
自分の環境に合わせて書き換えてください:

- `config/episode_config.json`の`outline_model`/`transcript_model`: `.env`の`LLM_MODEL`と
  同じ値，または別のモデルに変更できます
- `config/speakers_config.json`の`voice_id`(2箇所): AivisSpeechの話者IDに書き換えます
  （調べ方は上の「1. `.env`ファイル」を参照．ホスト役・ゲスト役で別の声にすることを推奨します）

解説モードだけを使う場合は，この2ファイルの設定は不要です．

### 3. sai-paperと連携する場合

論文管理アプリ「sai-paper」の `paper radio` で登録したい場合は，sai-paper側の設定も必要です:
- sai-paper側の`.env`に`SAI_RADIO_TOKEN`（sai-radio側と同じ値，未設定なら空でよい）を設定
- sai-paper側の`config.toml`の`[podcast]`セクションで，sai-radioのアドレス
  （既定は`http://127.0.0.1:8201`）を確認・設定

## 使い方（日常の流れ）

### 論文を1本登録する

**sai-paperから**(論文管理アプリ「sai-paper」を使っている場合): `sai-paper paper radio <論文番号>` を実行するだけです．
数分待つと，日本語のポッドキャスト風音声（2人が対話する形式）ができます．進捗は`shift+L`の
画面で確認できます．

**sai-webから**: Papers タブ → Radio 画面の「音声を作る」を押すと，バックグラウンドで
音声化が始まり，終わると一覧に並びます．

**コマンドラインから**(Macで直接PDFファイルを持っている場合):
```bash
python -m sai_radio.ingest 論文.pdf
```
数分待つと，日本語の要約音声ができます．解説モード（1人が順番に読む）とポッドキャストモード
（2人が対話する）を選べます．
```bash
python -m sai_radio.ingest 論文.pdf --mode podcast --title "論文のタイトル"
```

### iPhoneで聞く

iPhone では **sai-web** の Papers タブ → Radio を開きます（sai-web の起動方法と画面の
説明は sai-web の README を参照）．ここに音声の一覧が並び，選ぶと再生画面に入ります．
sai-radio は Mac 上の `127.0.0.1:8201` で動いており，sai-web からのみ中継で使われます．

### 質問する

再生画面の下に，チャットのような質問欄があります．

- **文字で質問する場合**: 入力欄に文字を打って「送信」ボタンを押します．少し待つと，回答が
  吹き出しで表示されます．
- **声で質問する場合**: マイクのボタンを押す → マイクの使用を許可 → 話す → もう一度ボタンを押す，
  で送信されます．少し待つと，その場所の内容をふまえた回答が音声で流れ，字幕も吹き出しに残ります．

どちらの方法で質問しても，やり取りは同じチャット欄に履歴として残ります．
再生画面を開き直しても，これまでの質問と回答は消えずに読み返せます．

仕組み（初心者向け，声で質問した場合）: 質問の声は iPhone から Google の AI（Gemini Live API）に
直接送られ，聞き取り・回答文づくり・回答の声づくりまで全部 Google 側で行われます．
Mac は最初に「使い捨ての合鍵（トークン）」と「今聞いている場所の論文本文」を渡すだけで，
APIキーそのものを iPhone に置く必要はありません．
（文字で質問した場合は，Mac 側のLLMが論文の該当箇所を読んで文字で回答します）

### そのほかの機能（画面は sai-web 側にある）

- 「続きを聴く」・お気に入り（★）・15秒スキップ・再生速度の変更（1x/1.25x/1.5x/2x）・
  文字起こしのタップジャンプは，すべて sai-web の Radio 画面で使えます．
- **音声をダウンロードしてオフラインで聞く（おすすめ）**: 外出先の電波が不安定でも止まらずに
  聞けるよう，Wi-Fi につないでいるうちに音声を iPhone に保存できます
  （Radio 画面の「この論文をダウンロード」）．
  ダウンロード済みの論文は，機内モードでもそのまま再生できます．

## 起動しておく必要があるもの

日常使う前に，以下がすべて起動している必要があります．

| 起動する場所 | 何を起動するか |
|---|---|
| Mac | AivisSpeech Engine |
| Mac | sai-radio本体サーバー（**常時起動が必要**．`sai-paper paper radio` もこれを前提にしています） |
| Mac | sai-web（ポート8200）．iPhoneの画面はこいつ経由で開きます |
| Mac（ポッドキャストモードを使う場合） | sai-radio TTSプロキシ（`sai_radio.tts_proxy`） |

それぞれの起動コマンドや詳しい設定項目は `ARCHITECTURE.md` を参照してください．

## つまずきやすいポイント

- **質問（録音）はHTTPS接続でないと動きません．** iPhone からは `tailscale serve`（4443番）で HTTPS 化された sai-web から画面を開くので，そのままで動きます．（Mac上の `localhost` は例外的にそのまま動きます）
- 質問しても回答が返ってこない場合は，まず`.env`の`GEMINI_API_KEY`が設定されているかを確認してください．次に，スマホの電波（Google につながるか）を確認してください．
- **ポッドキャストモードを使うには，`config/episode_config.json`と`config/speakers_config.json`に，使うLLMモデル名とAivisSpeechの話者IDを設定しておく必要があります．** 詳しくは`ARCHITECTURE.md`を参照してください．
- **`sai-paper paper radio` が失敗する場合**は，まずsai-radio本体サーバーが起動しているか確認してください．次に，その論文の`extract.md`（PDF本文の抽出結果）がsai-paper側で既に作られているか確認してください（無い場合は先に`sai-paper paper extract` で抽出する必要があります）．
- **サーバーはポート8201・`127.0.0.1`で起動してください．** `ARCHITECTURE.md`の想定どおり，
  本体サーバーは Mac の中だけ（sai-web からのみ使われます）受け付ける設定です．

  ```bash
  uv run uvicorn sai_radio.main:app --app-dir src --host 127.0.0.1 --port 8201
  ```

  **もしポート8201につながらない場合**: 他のプロセスがポート8201を先に使っていることが
  あります．`lsof -i :8201`で確認してください．なお，使っていない
  `open-notebook`のDockerコンテナ`opennotebook-surrealdb-1`は旧ポートの8000を使うため，
  sai-moon（8201）とは衝突しません（この注意は旧構成 8000番時代の記録です）．

- **Pythonのファイルを直したら，サーバーを再起動してください．** Pythonのプログラムは起動したときにメモリへ読み込まれるため，ファイルを書き換えても再起動するまで反映されません．開発中は起動コマンドの末尾に `--reload` を付けておくと自動で反映されます．
- **質問がうまくいかないときは，画面に出るメッセージで原因を切り分けられます．**

  | 表示 | 意味 | 確認すること |
  |---|---|---|
  | `Google に接続できませんでした(通信が届いていない可能性)` | iPhoneからGoogleまで届いていない | 電波状況，Tailscaleの設定 |
  | `接続はできましたが設定が拒否されました` | 届いているがGoogleが断っている | `.env`の`GEMINI_API_KEY` |
  | `Method Not Allowed` | サーバーに窓口がない | サーバーの再起動忘れ（上記） |
