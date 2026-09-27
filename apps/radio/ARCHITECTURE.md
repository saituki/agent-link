# ARCHITECTURE

開発者向けの技術メモ．設計の背景は `docs/20260917-1136-implementation-plan.md`（初期計画）と `docs/20260917-1910-aivisspeech-whisper-migration.md`（AivisSpeechへの移行と実機テスト記録），および `docs/20260922-0950-independence-and-upload-ui-plan.md`（open-notebook独立化計画）も参照．

## 全体構成

```
[Mac]
  ├─ AivisSpeech Engine (lib/AivisSpeech-Engine, port 10101)
  │    └─ POST /audio_query → POST /synthesis … 文章 → 日本語音声
  ├─ sai-radio 本体サーバー (src/sai_radio/main.py, uvicorn, port 8201)
  │    ├─ PDF→Markdown抽出 (docling CLI呼び出し)
  │    ├─ 日本語ラジオ原稿要約 (LLM直接呼び出し via esperanto)
  │    ├─ ポッドキャスト生成 (podcast-creatorライブラリ)
  │    ├─ 音声を AAC(m4a, 64kbps) に圧縮して保存
  │    ├─ Gemini Live API 用の使い捨てトークン発行 (live.py)
  │    ├─ 解説モード音声生成 (AivisSpeech Engine)
  │    └─ 再生位置 → 論文箇所の逆引き
  └─ sai-radio TTS プロキシ (src/sai_radio/tts_proxy.py, uvicorn, port 8202)
       └─ podcast-creator ⇔ AivisSpeech Engine の橋渡し (podcastモード用)

[Google]
  └─ Gemini Live API … 質問の聞き取り・回答文・回答音声をすべて担当
       └─ iPhone のブラウザから WebSocket で直接接続

[Rust TUI]
  └─ sai-paper (別リポジトリ, port 8201への HTTP POST)
       └─ POST /api/papers/from-extract … `r` キーで抽出済みMarkdownを送って音声化を依頼

[sai-phone (FastAPI, port 8100)]
  └─ Papers タブ → Radio 画面
       ├─ /api/radio/* を sai-radio の /api/* へ中継 (127.0.0.1:8201)
       ├─ 「音声を作る」も POST /api/papers/from-extract を使う (sai-paper の `r` キーと同じ窓口)
       ├─ 音声の事前ダウンロード (オフライン再生) は sai-phone 側が担当
       └─ 質問時は Mac からトークンと論文本文をもらい Google へ直結 (Live API)

Mac・sai-paper は Tailscale VPN で相互に接続する．
質問の音声は iPhone ⇔ Google 間で直接やり取りされ，Mac を経由しない．
(以前使っていた Windows PC の音声認識サーバーは削除済み)
```

**重要**: sai-radio は常時起動する必要があります（決定事項#11）．sai-paper の `r` キーを使うには sai-radio サーバーが起動していることが前提です．
起動コマンドは次のこと（`127.0.0.1` 固定．sai-radio に画面は無く，sai-phone からのみ中継で使われる）:

```bash
uv run uvicorn sai_radio.main:app --app-dir src --host 127.0.0.1 --port 8201
```

## ディレクトリ構成

```
src/
├── sai_radio/              # Mac で動かす本体サーバー
│   ├── main.py             # APIサーバー本体 (FastAPI)
│   ├── extract.py          # PDF→Markdown抽出 (docling CLI呼び出し)
│   ├── llm.py              # 要約・質問応答 (LLM直接呼び出し via esperanto)
│   ├── live.py             # Gemini Live API 用の使い捨てトークン発行
│   ├── podcast.py          # ポッドキャスト生成 (podcast-creatorライブラリ)
│   ├── voice.py            # AivisSpeech Engine 呼び出し
│   ├── sections.py         # 要約テキストをセクション単位に分割
│   ├── store.py            # 論文・音声対応表・ログの保存 (SQLite)
│   ├── ingest.py           # 論文登録CLI (python -m sai_radio.ingest)
│   ├── config.py           # 環境変数(.envも含む)から設定値を読む
│   ├── tts_proxy.py        # podcast-creator用 openai_compatible TTSプロキシ (AivisSpeechへの橋渡し)
│   └── tts_proxy.py        # podcast-creator用 openai_compatible TTSプロキシ (AivisSpeechへの橋渡し)
└── data/                   # 生成した音声・SQLite DB (.gitignore対象)

scripts/
└── migrate_audio_to_m4a.py # 既存の WAV 音声を m4a に変換する移行スクリプト (変換済み)

config/                    # podcast-creator用の設定JSONファイル
├── speakers_config.json    # 話者設定 (ホスト・ゲストの声質)
└── episode_config.json     # エピソード設定 (LLMモデル・セグメント数)

lib/
└── AivisSpeech-Engine/    # 読み取り専用．音声合成エンジン本体 (別途起動)
└── open-notebook/         # 読み取り専用．参照用 (現在未使用)
```

以前あった `src/sai_radio/static/` (PWAの画面) も削除済み (11-cleanup §3 A)．
画面は sai-phone (Papers タブ → Radio) が持ち，sai-radio は API だけの裏方サーバーになった．

以前の `src/tsukasa-server/` (Windows PC 用の音声認識サーバー) は削除済み．
質問機能は Gemini Live API に移行したため不要になった．
(削除前の状態は git のコミット `deef9cf` に残してあるので，必要なら戻せる)

## なぜ AivisSpeech Engine に切り替えたか

当初は `lib/Tsukasa-Speech` を `src/tsukasa-server/tsukasa/` にコピーし，音声合成(TTS)もそこで行う設計だった．しかし依存ライブラリ (`torch<2.6` など) のバージョン制約が古く，新しい環境では構築できなくなった．

**AivisSpeech Engine**（`lib/AivisSpeech-Engine`）は VOICEVOX ENGINE 互換のHTTP APIサーバーで，単独のアプリとして別途起動しておくだけで使える．そのため `src/tsukasa-server` からTTS機能を削除し，`voice.py` が直接AivisSpeech EngineのAPIを叩く形に変更した．

Whisperは元々Tsukasa-Speechとは無関係な独立ライブラリのため，この変更の影響を受けない．

## なぜ Gemini Live API に移行したか

以前は質問の声を文字に起こす処理を Windows PC 上の Whisper サーバー
(`src/tsukasa-server`) に任せていた．しかしこの方式だと Windows PC の
電源が入っていないと質問できず，外出先での利用が成立しなかった．

そこで質問の「聞き取り・回答文づくり・回答の声づくり」をすべて
Gemini Live API に集約し，iPhone から Google に直接つなぐ方式に変えた．
Mac は最初に「使い捨ての合鍵(トークン)」と「今聞いている場所の論文本文」を
渡すだけで，音声そのものは Mac を経由しない．Windows PC は不要になったため削除した．

質問1回の流れ:

```
1. iPhone → Mac      「質問する．トークンをください」(再生位置を添えて)
   Mac → iPhone      トークン + 該当セクションの論文本文 + モデル名
                     ※ここは JSON のみ．1〜2 KB．4G でも一瞬で終わる

2. iPhone ←→ Google  WebSocket 直結．音声を送り，音声が返る
                     Mac は経由しない
                     同じ接続のまま何ターンでも続けられる

3. iPhone → Mac      質問文と回答文をログとして記録 (後追いでよい)
```

## Gemini Live API の接続方式

すべて実接続で検証済み．推測で変えないこと．

### エフェメラルトークン (使い捨ての合鍵) によるブラウザ直結

iPhone のブラウザから Google に直接 WebSocket 接続でき，API キーを sai-phone 側に置く必要がない．

**トークン発行** (Mac 側で実行する):

```
POST https://generativelanguage.googleapis.com/v1alpha/auth_tokens?key=<GEMINI_API_KEY>
Content-Type: application/json

{"uses": 1,
 "expireTime": "<ISO8601 Z形式>",
 "newSessionExpireTime": "<ISO8601 Z形式>"}
```

レスポンス: `{"name": "auth_tokens/<64桁のhex>"}`

**注意**: REST の `v1alpha/auth_tokens` は `liveConnectConstraints` フィールドを受け付けない
(`400 Unknown name "liveConnectConstraints" at 'auth_token': Cannot find field.`)．
制約なしのトークンで発行すること．`uses: 1` と短い `newSessionExpireTime` で実質的な安全性を確保する．

**WebSocket 接続** (iPhone のブラウザから実行する):

```
wss://generativelanguage.googleapis.com/ws/google.ai.generativelanguage.v1alpha.GenerativeService.BidiGenerateContentConstrained?access_token=<URLエンコードしたトークン名>
```

**ここが最大の落とし穴**: メソッド名が `BidiGenerateContent` ではなく
**`BidiGenerateContentConstrained`** である．通常の `BidiGenerateContent` にエフェメラルトークンを渡すと
`1008 policy violation: Method doesn't allow unregistered callers` で切断される．
API バージョンも **`v1alpha` 固定** (`v1beta` は不可)．

トークン名には `/` が含まれるので，`encodeURIComponent()` で URL エンコードすること．

**検証結果の一覧** (どれが通ってどれが落ちるか):

| 認証方法 | メソッド | 結果 |
|---|---|---|
| `?access_token=` | `BidiGenerateContentConstrained` | **成功** (これを採用する) |
| `Authorization: Token <name>` ヘッダ | `BidiGenerateContentConstrained` | 成功 (ただしブラウザは WebSocket にヘッダを付けられないので使えない) |
| `?key=` | `BidiGenerateContentConstrained` | `1007 Missing or malformed auth token in request` |
| `Sec-WebSocket-Protocol` サブプロトコル | `BidiGenerateContentConstrained` | `1007 Missing or malformed auth token in request` |
| `?access_token=` | `BidiGenerateContent` | `1008 policy violation` |
| `?key=<トークン>` | `BidiGenerateContent` | `1007 API key not valid` |

ブラウザの `new WebSocket(url)` はカスタムヘッダを設定できないため，
`access_token` クエリパラメータ方式が唯一の選択肢である．

### Live API のメッセージ形式

**setup メッセージ** (接続直後に1回だけ送る):

```json
{"setup": {
  "model": "models/gemini-2.5-flash-native-audio-preview-12-2025",
  "generationConfig": {"responseModalities": ["AUDIO"]},
  "systemInstruction": {"parts": [{"text": "<指示文 + 論文セクション本文>"}]},
  "realtimeInputConfig": {"automaticActivityDetection": {"disabled": true}},
  "inputAudioTranscription": {},
  "outputAudioTranscription": {}
}}
```

サーバーは `{"setupComplete": {}}` を返す (実測 0.13 秒)．

- `responseModalities` に `["TEXT"]` を指定すると native-audio モデルは拒否する:
  `1007 The requested combination of response modalities (TEXT) is not supported by the model.`
  音声出力専用である．
- `realtimeInputConfig.automaticActivityDetection.disabled: true` にすると，
  サーバー側の自動発話区間検出を止め，クライアントが明示的に発話の開始・終了を伝える方式
  (プッシュトゥトーク) になる．本アプリはボタン押下で録音を開始・終了するので，この方式が合う．
- `inputAudioTranscription` / `outputAudioTranscription` を空オブジェクトで有効にすると，
  質問の聞き取り結果と回答の字幕がテキストで届く．ログ保存と画面表示に使う．

**音声の送信**:

```json
{"realtimeInput": {"activityStart": {}}}
{"realtimeInput": {"audio": {"mimeType": "audio/pcm;rate=16000", "data": "<base64>"}}}
   ... 100ms 分ずつ繰り返す ...
{"realtimeInput": {"activityEnd": {}}}
```

入力音声の形式: **PCM signed 16-bit little-endian / 16000 Hz / モノラル**．100 ms 分は 3200 バイト．

**音声の受信**:

サーバーからは `serverContent.modelTurn.parts[].inlineData` に base64 の音声が届く．
`mimeType` は `audio/pcm;rate=24000`．すなわち **PCM signed 16-bit little-endian / 24000 Hz / モノラル**．
入力と出力でサンプリングレートが違う点に注意．

- `serverContent.inputTranscription.text` … 質問の聞き取り結果 (断片的に届くので連結する)
- `serverContent.outputTranscription.text` … 回答の字幕 (同上)
- `serverContent.turnComplete: true` … そのターンの応答が終わった合図

**次のターン**: 同じ WebSocket 接続のまま `activityStart` から繰り返すだけでよい．setup も論文本文も送り直さない．

**採用するモデル**: `models/gemini-2.5-flash-native-audio-preview-12-2025`
(native-audio 系は音声入力→音声出力が通ることを確認済み．`-latest` は中身が差し替わるため日付固定版を使う)．

### ブラウザ側の実装上の注意 (app.js)

- **Live API の応答は WebSocket のバイナリフレームで届く**．
  そのため `binaryType` を `arraybuffer` にし，`TextDecoder` で文字列に直してから JSON として読んでいる．
  この2つを忘れると応答が読めない．
- マイク入力は `AudioWorklet` (pcm-worklet.js) で PCM に変換して送る．`MediaRecorder` は使わない
  (Live API は生の PCM を要求するため)．
- 回答音声は `<audio>` タグでは鳴らせない (PCM のため)．`AudioContext` で順番に連結して再生する．

## 音声の圧縮とオフライン再生

以前は非圧縮 WAV (44100 Hz / 16 bit / 1 ch = 705.6 kbps) をそのまま配信していた．
24分の音声で 128.5 MB になり，iPhone の 4G 回線では数秒で止まってしまった．

そこで次の2つを入れた:

- **音声は AAC(m4a, 64kbps) で保存する**．ファイルサイズが約 1/11 になる
  (128.5 MB → 約 11.6 MB)．`voice.py` が WAV 連結後に ffmpeg で変換する．
  `-movflags +faststart` は再生開始を早めるための必須オプション
  (ファイルの先頭に再生に必要な情報を置く)．
  配信側は `.m4a` を `audio/mp4` として返す．シークのための Range リクエストには
  `FileResponse` が 206 を自前で返している (Starlette の標準機能)．
- **オフライン再生は sai-phone 側の機能になった** (sai-radio に画面は無いため)．
  sai-phone の Radio 画面が音声をキャッシュし，通信なしで再生できるようにする．

## モジュールの役割

### `sai_radio/config.py`

すべての設定値を環境変数から読む．主なもの:

| 環境変数 | 既定値 | 用途 |
|---|---|---|
| `LLM_PROVIDER` | `openrouter` | LLM API プロバイダ (esperanto経由) |
| `LLM_MODEL` | `""` (要設定) | LLM モデル名 (形式: `<プロバイダ>/<モデル>`, 例: `openai/gpt-4o-mini`) |
| `DOCLING_BIN` | `docling` | docling CLI のパス |
| `DOCLING_TIMEOUT_SEC` | `300` | PDF抽出のタイムアウト秒数 |
| `PODCAST_EPISODE_CONFIG` | `config/episode_config.json` | podcast-creator用エピソード設定ファイルパス |
| `PODCAST_SPEAKERS_CONFIG` | `config/speakers_config.json` | podcast-creator用話者設定ファイルパス |
| `PODCAST_PROFILE_NAME` | `sai_radio` | podcast-creator内で使用するプロファイル名 |
| `AUDIO_BITRATE` | `64k` | 解説モード音声を m4a に圧縮するときのビットレート |
| `GEMINI_API_KEY` | 未設定 (要設定) | Gemini API キー．質問機能 (Live API のトークン発行) に必須 |
| `GEMINI_LIVE_MODEL` | `models/gemini-2.5-flash-native-audio-preview-12-2025` | Live API で使うモデル (日付固定版) |
| `GEMINI_TOKEN_TTL_SEC` | `1800` | 使い捨てトークンの有効期限 (秒)．既定30分 |
| `AIVISSPEECH_BASE_URL` | `http://127.0.0.1:10101` | AivisSpeech Engine |
| `AIVISSPEECH_SPEAKER` | `0` (要設定) | 解説モード用の話者スタイルID．`GET /speakers` で確認する |
| `SAI_RADIO_TOKEN` | 未設定 | 簡易トークン認証．未設定なら認証なし(開発用) |

`.env`ファイルはプロジェクトルート(`./​.env`)から `python-dotenv` で自動読み込みされる(`config.py` 冒頭の `load_dotenv()`)．`export` は不要で，`.env` に書けばそのまま反映される．書き方の見本は `.env.example` を参照．

LLMの認証情報（`OPENROUTER_API_KEY` など）は環境変数として別途設定が必要です（`config.py` では参照せず，`esperanto` ライブラリが自動で読み込みます）．

### `sai_radio/extract.py`

PDFをMarkdown形式に変換するだけのシンプルなモジュール．`docling` CLI をサブプロセスで起動します．

- `extract_markdown(pdf_path)`: PDFファイルをdocling経由でMarkdown化し，その全文を返す

docling は `~/.local/bin/docling` にインストール済みで，Python パッケージとしてではなく CLI コマンドとして呼び出します（`podcast-creator` の依存関係として torch 等の重い ML スタックが不要なため）．

### `sai_radio/llm.py`

要約・質問応答を LLM に直接委譲するモジュール．`esperanto` ライブラリ経由で OpenRouter 等のプロバイダの LLM を呼びます．

- `summarize_to_radio_script(markdown_text)`: PDFから抽出したMarkdown全文を日本語のラジオ原稿風に要約
- `answer(section_text, question)`: 論文の該当箇所＋質問文から回答を生成 (sai-phone の文字での質問 `POST /api/papers/{paper_id}/ask` から呼ばれる)

LLMのプロンプトは元の `notebook.py` から一字一句移植したものです（意思決定含む）．

### `sai_radio/live.py`

Gemini Live API 用の使い捨てトークン (エフェメラルトークン) を発行するモジュール．

- `issue_ephemeral_token() -> str`: `POST /v1alpha/auth_tokens` を叩き，トークン名 (`auth_tokens/...` 形式) を返す．接続方式の落とし穴は「Gemini Live API の接続方式」の節を参照

### `sai_radio/podcast.py`

`podcast-creator` ライブラリを直接呼び出し，ポッドキャスト音声を生成するモジュール．

- `build_podcast(full_text, paper_id)`: 与えられたテキストから podcast-creator でポッドキャスト音声を生成

内部では podcast-creator の設定ファイル（`config/episode_config.json` / `config/speakers_config.json`）を読み込み，LLM と TTS の呼び出しを podcast-creator の内部グラフに委譲します．

### `sai_radio/voice.py`

AivisSpeech Engine を呼び出す薄いラッパー．

- `synthesize(text, speaker=None) -> bytes`: AivisSpeech Engineの2段階API（`POST /audio_query` でクエリ作成 → `POST /synthesis` で音声化）を呼ぶ．`speaker` パラメータは style_id（省略時は `settings.aivisspeech_speaker`）
- `build_narration(sections, paper_id)`: セクションごとに`synthesize()`を呼び，WAVを連結しながら各セクションの開始・終了秒数を記録する（逆引き対応表の元になる）．連結後に ffmpeg で m4a (AAC) に圧縮する
- `new_paper_id()`: 新規論文のIDを生成する

`synthesize()` が `speaker` 引数を取れるのは，podcast.py の `tts_proxy.py` 呼び出しで話者ごとに異なるIDを渡すため（podcast-creator は複数話者を使い分ける）．

### `sai_radio/tts_proxy.py`

podcast-creator のポッドキャスト生成（`podcast.py` 経由）は，TTS を **openai_compatible** プロバイダ経由でしか呼べません（`POST /audio/speech`，リクエストは `{model, voice, input, response_format}`，レスポンスはMP3バイト列固定）．一方AivisSpeech EngineはVOICEVOX互換の2段階API（`/audio_query`→`/synthesis`）でWAVしか返します．このモジュールは両者を仲介する変換サーバーです：

```
podcast-creator (openai_compatible TTS経由) → POST /audio/speech
  → tts_proxy.py: voice フィールドを AivisSpeech の話者スタイルID(int)として解釈
  → voice.synthesize(text, speaker=...) で WAV 生成
  → ffmpeg (subprocess) で WAV → MP3 に変換 (podcast-creator が拡張子.mp3固定でファイル保存するため，本物のMP3が必須)
  → MP3バイト列を返す
```

起動: `uv run uvicorn sai_radio.tts_proxy:app --port 8202`（sai-radio本体サーバーとは別プロセス）．`GET /models` と `GET /audio/voices` は実装していません（呼び出し元の `esperanto` ライブラリ側にフォールバックがあり，なくても動きます）．

### `sai_radio/store.py`

SQLite1ファイルで完結する永続化層．テーブルは`papers`（論文情報＋対応表のJSON＋お気に入りフラグ）と`events`（聴取・質問ログ）の2つだけ．

- `find_section(paper_id, position_sec)`: 対応表を線形探索し，再生位置が含まれるセクションを返す．これが「再生位置からの逆引き」の実体
- `set_favorite(paper_id, favorite)`: お気に入り(`favorite`列)のON/OFFを切り替える
- `stats_by_day(days=84)`: 直近`days`日分の`kind="listen"`イベントを日付ごとに数える．聞いていない日も0件で返すので，ホーム画面のカレンダーヒートマップにそのまま並べられる
- `favorite`列は`init_db()`内で`ALTER TABLE ... ADD COLUMN`を`try/except`で流すことで，
  既存のDBファイルにも非破壊で追加できるようにしている

### `sai_radio/main.py`

FastAPIアプリ本体．論文の一覧・詳細・音声配信・質問応答のAPI，新規論文の登録（sai-paper連携・CLI）のAPIを1つのアプリにまとめています．画面（静的ファイル）と PDF アップロード（`POST /api/papers/upload`）は削除済みで，`GET /` は 404 を返します（11-cleanup §3 A）．sai-phone から中継で使われる裏方サーバーです．

主要なAPI：
- `GET /api/papers`: 論文一覧（`favorite`フラグ・再生時間を含む）
- `GET /api/papers/{paper_id}`: 論文詳細（セクション対応表`sections`を含む．文字起こしタップジャンプに使用）
- `POST /api/papers/{paper_id}/favorite`: お気に入りのON/OFFを反転する
- `POST /api/papers/{paper_id}/ask`: **テキストで質問する**（音声を使わない経路）．
  再生位置から該当セクションを逆引きし，`llm.answer()`（従来から存在したが未使用だった関数）で
  回答テキストを生成して返す．同期呼び出しなので数秒待たせる
- `GET /api/stats`: ホーム画面用の集計．登録数・聴いた論文数・直近84日分の日別再生回数
  （`store.stats_by_day()`）を返す
- `POST /api/papers/from-extract`: 抽出済みMarkdownを受信してポッドキャスト生成（podcast専用）．sai-paperの`r`キーとsai-phoneの「音声を作る」の両方がこの窓口を使う
- `POST /api/papers/{paper_id}/live-session`: **声で**質問する用．再生位置から該当セクションを逆引きし，Live API 用のトークン＋論文本文を返す
- `POST /api/papers/{paper_id}/question-log`: 声で質問した内容(質問・回答のテキスト)をログに記録するだけのエンドポイント

声の質問(`live-session`)と文字の質問(`ask`)は，どちらも同じ`events`テーブルに`kind="question"`で
記録され，sai-phone 側の再生画面ではこの2つを区別なく同じチャット履歴として表示する．

質問の流れ (`POST /api/papers/{paper_id}/live-session` + Live API 直結):

```
再生位置
  → find_section() で該当セクション本文を取得
  → live.issue_ephemeral_token() で使い捨てトークンを発行
  → トークン + 論文本文 + 接続先URL を iPhone に返す
  → iPhone から Google に直接 WebSocket 接続 (音声の送受信は Mac を経由しない)
  → 終わったら質問文・回答文を /question-log に記録
```

## 既知の制約・落とし穴

- **sai-radio は常時起動が必要**: sai-paper の `r` キーを使うには sai-radio サーバーが `http://127.0.0.1:8201` で起動していることが前提です（決定事項#11）．起動していない場合は sai-paper 側でエラーになります
- **ローカルのポート衝突に注意**: 開発機で他プロセスが `8201` / `8202` / `10101` などのポートを掴んでいることがある．起動前に `lsof -i :<ポート>` で確認するか，`.env` で `AIVISSPEECH_BASE_URL` 等を変更してください
  - **実例(2026-09-22，旧構成の記録)**: 使っていない`open-notebook`のDockerコンテナ`opennotebook-surrealdb-1`
    (`surrealdb/surrealdb:v2`)が`127.0.0.1:8000`を先に握っていたため，sai-radioが起動していても
    ポート8000に繋がらなかった．`docker stop opennotebook-surrealdb-1`で解消する．
    sai-moon では本体のポートを 8201 に変更したため，この Docker コンテナとの衝突は起きない（古い 8000 番の記録）
    このコンテナは再起動ポリシーが`always`のため，Docker再起動やMac再起動のたびに自動で復活し，
    再びポートを奪う可能性がある点に注意
- **LLM_MODEL の設定が必須**: `.env` に `LLM_MODEL` を設定してください（形式: `<プロバイダ>/<モデル>`，例: `openai/gpt-4o-mini`）．未設定のままだと LLM 初期化時に `ConfigurationError` が発生します
- **LLM API キーが環境変数として必須**: OpenRouter の場合は `OPENROUTER_API_KEY` を環境変数として設定してください．`config.py` では参照しません（`esperanto` ライブラリが自動で読み込みます）
- **GEMINI_API_KEY の設定が必須 (質問機能)**: `.env` に `GEMINI_API_KEY` を設定してください．未設定のままだと質問開始時に日本語のエラーが出ます
- **podcast-creator の設定JSONが必須**: `config/episode_config.json` / `config/speakers_config.json` が存在し，有効な JSON 形式である必要があります．ポッドキャスト生成開始時にバリデーションされ，エラーがあると生成が失敗します
- **AivisSpeech Engine の話者スタイルID**: `config/speakers_config.json` の `voice_id` フィールドは数字として解釈可能な文字列である必要があります（AivisSpeechへ `int` で渡される）．`GET http://127.0.0.1:10101/speakers` で確認できる ID を使用してください
- **質問機能はHTTPS必須**: ブラウザのマイクAPIは `https://` または `localhost` でのみ動きます．iPhoneからは `tailscale serve`（4443番）でHTTPS化された sai-phone の画面を使うので，sai-radio 本体は `127.0.0.1` のままで足りる
- **Docling CLI のパス**: `docling` CLI は `~/.local/bin/docling` にインストール済みと仮定しています．別の場所にある場合は `.env` に `DOCLING_BIN=/path/to/docling` を設定してください
- **ポッドキャスト生成には複数のLLM呼び出しが発生**: outline 生成 1 回 + transcript 生成 1 回＋最初の日本語要約用 LLM が 1 回，合計 3 回のLLM API 呼び出しが発生します．無料枠のモデルを使う場合はレート制限に注意してください
