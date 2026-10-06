# sai-moon：iPhone 接続と音声作成の修正記録

2026-09-27 に行った作業の記録。対象プロジェクトは `/Users/touta/research/test-moon-0927`（以下 PROJECTROOT）。

---

## 1. 何が問題だったか（3 件）

### 1.1 iPhone から接続する方法が文書に書かれていなかった

Web 画面（`127.0.0.1:8200`）は Mac のブラウザからは開けたが、iPhone から開く手順が確定していなかった。

### 1.2 合言葉を入力しても接続できない

`SAI_PHONE_TOKEN` を設定して iPhone から開くと、`合言葉を入力してください` の入力欄が繰り返し出現し、入力しても API が 401 を返し続けた。

**原因**（`apps/phone/src/sai_phone/static/app.js:33-45` の実装から）:

- ページを開くと API が十数個同時に飛ぶ（`/api/home` `/api/projects` `/api/days` など）。
- ブラウザが合言葉を覚えていないと、その全部が 401 を返す。
- 401 を受けるたびに `apiFetch` が `prompt("合言葉を入力してください")` を呼ぶ（`app.js:37`）。つまり入力欄が十数個出てくる。
- iOS Safari は `prompt` を同時に 1 つしか表示できない。残りは空文字で返る。
- `if (input)` が false のため再送しない（`app.js:39`）。
- 結果として、同じ API が 200 と 401 を混在して返す（`/tmp/sai-moon-8200.log` で実際に観測した）。

ブラウザが合言葉を覚えていないのは、プライベートブラウズ（無痕モード）で開いており、localStorage の内容がタブを閉じたときに破棄されるため。

**対処**: 合言葉そのものを無効化（`SAI_PHONE_TOKEN=`）。401 が出なくなるので、プライベートブラウズでも動く。

### 1.3 音声作成ができない

`POST /api/papers/from-extract` が 500（`Internal Server Error`）を返した。

**原因**: `apps/radio/.env` の `AIVISSPEECH_SPEAKER=0` が、今の AivisSpeech Engine に存在しないスタイル ID だった。

```
$ curl http://127.0.0.1:10101/is_initialized_speaker?speaker=0
{"detail":"スタイル 0 は存在しません。"}   # 404
```

`voice.py:50` の `synth_resp.raise_for_status()` が 404 で例外を投げ、それがそのまま 500 になっていた。
AivisSpeech Engine **1.1.0-dev** は初期状態でスタイル 0 を持たない。ID は 10 億規模のランダム値。

**対処**: 存在する ID（`888753760`）に変更。

---

## 2. 今回の修正内容（ファイル別）

### 2.1 `sai-moon/apps/phone/.env`

```
SAI_PHONE_TOKEN=          ← 値を入れていたものを空に戻した
```

合言葉なし（= tailnet 内に到達できる端末なら誰でも開ける）。起動ログに `SAI_PHONE_TOKEN 未設定：認証なしで動作中` と出るのは正常。

### 2.2 `sai-moon/apps/radio/.env`

```
AIVISSPEECH_SPEAKER=888753760     ← 0 から変更（まお / ノーマル）
```

コメントに「2026-09-27 実測、Engine 1.1.0-dev にはスタイル 0 が存在しない」旨と、他の ID 一覧を追記した。

### 2.3 `PROJECTROOT/.sai/paper/settings.json`

論文の音声を旧 sai-radio から sai-moon へ切り替えた。

| 項目 | 変更前 | 変更後 |
|---|---|---|
| `radio.server_url` | `http://127.0.0.1:8000` | `http://127.0.0.1:8201` |
| `radio.voice.base_url` | `http://127.0.0.1:8600` | `http://127.0.0.1:8202` |

`radio.voice.speakers` の 2 つの `voice_id` は変更なし。どちらも実在する ID だった。

- ホスト = `1310138977`（阿井田 茂 / Calm）
- ゲスト = `888753760`（まお / ノーマル）

### 2.4 `PROJECTROOT/.claude/settings.json`

cc（claude code）のフック 3 本をプロジェクト側に書き足した。既存の `UserPromptSubmit`（`sai-moon-link cc-prompt-hook`）はそのまま。

```
PreToolUse          AskUserQuestion → apps/phone/hooks/cc_question.py
SessionStart                          → apps/phone/hooks/cc_register.py
PermissionRequest                     → apps/phone/hooks/cc_permission.py
```

**なぜプロジェクトに置くのか**: 旧手順（`sai-base` 4.12）は `~/.claude/settings.json` を上書きして試し、終わったら戻すという往復だった。プロジェクトに置くことで、プロジェクトごとに付け外しができる。

**注意**: cc はグローバル設定とプロジェクト設定を合算する。`~/.claude/settings.json` の旧フック（8100 向け）はまだ残っているので、質問・承認は 8200 と 8100 の両方に通知が飛ぶ。

### 2.5 Tailscale serve（設定ファイルではなく Mac の状態）

```bash
tailscale serve --bg --https 4444 http://127.0.0.1:8200
```

`--set-path /sai-moon` は使ってはいけない（理由は 4.1）。

---

## 3. 設定と起動の方法

### 3.1 前提

- AivisSpeech Engine が `127.0.0.1:10101` で動いていること

  ```bash
  curl -s -o /dev/null -w "%{http_code}\n" http://127.0.0.1:10101/speakers   # 200 なら OK
  ```

- `OPENROUTER_API_KEY` と `GEMINI_API_KEY` が `~/.zshrc` に入っていること（8201 は LLM 台本生成と質問機能に必要）
- 3 サーバーはすべて手動起動。Mac を再起動すると落ちる（launchd 登録はしていない）

### 3.2 8200（Web の窓口）— 必要

```bash
cd ~/workdir/Sai/sai-moon
PATH=$HOME/workdir/Sai/sai-moon/target/release:$PATH \
  apps/phone/.venv/bin/uvicorn sai_phone.main:app \
  --app-dir apps/phone/src --host 127.0.0.1 --port 8200
```

- `PATH` を足さないと `sai-moon-link` が見つからず Project タブが 503 になる（`sai-moon-link` は PATH に無い）
- 起動ログに `SAI_PHONE_TOKEN 未設定：認証なしで動作中` と出るのは正常

### 3.3 8201（Radio の裏方サーバー）— 論文の音声に必要

```bash
cd ~/workdir/Sai/sai-moon
apps/radio/.venv/bin/uvicorn sai_radio.main:app \
  --app-dir apps/radio/src --host 127.0.0.1 --port 8201
```

### 3.4 8202（TTS 変換プロキシ）— 音声合成に必要

```bash
cd ~/workdir/Sai/sai-moon/apps/radio
.venv/bin/uvicorn sai_radio.tts_proxy:app \
  --app-dir src --host 127.0.0.1 --port 8202
```

**作業フォルダが `apps/radio` で、相対パス `.venv/bin/uvicorn` と `--app-dir src` を使う**点が 8201 と違う。ここで間違うと `Could not import module "sai_radio"` になる。

### 3.5 まとめて起動する

```bash
cd ~/workdir/Sai/sai-moon
nohup apps/radio/.venv/bin/uvicorn sai_radio.main:app \
  --app-dir apps/radio/src --host 127.0.0.1 --port 8201 \
  > /tmp/sai-moon-8201.log 2>&1 &

cd ~/workdir/Sai/sai-moon/apps/radio
nohup .venv/bin/uvicorn sai_radio.tts_proxy:app \
  --app-dir src --host 127.0.0.1 --port 8202 \
  > /tmp/sai-moon-8202.log 2>&1 &

cd ~/workdir/Sai/sai-moon
PATH=$HOME/workdir/Sai/sai-moon/target/release:$PATH \
  nohup apps/phone/.venv/bin/uvicorn sai_phone.main:app \
  --app-dir apps/phone/src --host 127.0.0.1 --port 8200 \
  > /tmp/sai-moon-8200.log 2>&1 &
```

**注意**: エージェントのシェルから `nohup` で起動すると、エージェント自身の `SAI_PARENT_RUN_ID` / `SAI_BLOCK_NAME` / `SAI_PROJECT_ROOT` を子プロセスに持ち込む。`config.py:17` の `load_dotenv(..., override=False)` により、環境変数が `.env` より優先される。実際の作業は必ず自分のターミナルから行う。

### 3.6 起動の確認

```bash
for p in 8200 8201 8202; do
  echo -n "$p: "
  lsof -ti :$p -sTCP:LISTEN >/dev/null 2>&1 && echo "LISTEN 中" || echo "停止"
done

# 8201 が準備できたか
curl -s http://127.0.0.1:8201/api/papers        # [] が返れば準備完了

# 8202 の音声合成が動くか
curl -s -X POST http://127.0.0.1:8202/audio/speech \
  -H "Content-Type: application/json" \
  -d '{"model":"aivisspeech","voice":"888753760","input":"テスト"}' \
  -o /dev/null -w "%{http_code}\n"               # 200 が返れば OK

# 8200 から 8201 への中継が繋がっているか
curl -s -o /dev/null -w "%{http_code}\n" http://127.0.0.1:8200/api/radio/papers
```

### 3.7 iPhone から開く URL

```
https://macbook-pro.tail3ab809.ts.net:4444/
```

`https://<Mac 名>.<tailnet>.ts.net:4444/` の形。Mac 名と tailnet ドメインは `tailscale status` で確認できる。

- 屋外（4G）でも同じ URL で開ける。Tailscale アプリが ON なら、外の Wi-Fi を経由しない
- 合言葉が無いのでプライベートブラウズでも動く
- Safari で開いて「ホーム画面に追加」すると、独立したアプリのように使える

### 3.8 Tailscale の状態確認

```bash
tailscale serve status
```

期待する出力:

```
https://<Mac名>.<tailnet>.ts.net:4443 (tailnet only)
|-- / proxy http://127.0.0.1:8100          ← 旧 sai-phone（触っていない）

https://<Mac名>.<tailnet>.ts.net:4444 (tailnet only)
|-- / proxy http://127.0.0.1:8200          ← sai-moon
```

`funnel` が含まれていないこと。`tailnet only` と表示されていれば正しい（インターネット全体への公開はしていない）。

### 3.9 よく使う声の ID

`curl -s http://127.0.0.1:10101/speakers` で取得できる。2026-09-27 時点の値:

| 話者 | ノーマル | そのほか |
|---|---|---|
| まお | 888753760 | ふつー 888753761 / あまあま 888753762 / おちつき 888753763 / からかい 888753764 / せつなめ 888753765 |
| コハク | 1878365376 | あまあま 1878365377 / せつなめ 1878365378 / ねむたい 1878365379 |
| 阿井田 茂 | 1310138976 | Calm 1310138977 / Far 1310138978 / Heavy 1310138979 / Mid 1310138980 / Shout 1310138981 / Surprise 1310138982 |

**AivisSpeech Engine を再インストールすると ID が変わる。** 音声が使えなくなったら、まず `curl -s http://127.0.0.1:10101/speakers` で ID を取り直す。

---

## 4. ハマった点

### 4.1 `--set-path /sai-moon` は使えない

`README.md:156` の手順（`tailscale serve --bg --https 4443 --set-path /sai-moon http://127.0.0.1:8200`）は、そのままでは動かない。

実測した挙動:

| リクエスト | 結果 |
|---|---|
| `/sai-moon/` （HTML） | 200 |
| `/sai-moon/api/projects` | 200（API も通る） |
| `/style.css` `/app.js` `/sw.js` | 404 |

`index.html:13` 以降が `href="/style.css"` とルート直書きで、prefix の下に無いため。ブラウザは 404 の CSS を読み、画面は白になる。API は通るので「通信はできているのに画面が真っ白」という症状になる。

**対処**: prefix を付けず、ルート公開にする（3.7 のコマンド）。

**注記**: `README.md:153` に「sai-moon ではまだ試していません（要確認）」とあったが、答えは「そのままでは動かない」。README のこの節はまだ修正していない。

### 4.2 `address already in use`

既に起動済みのプロセスがポートを掴んでいるときに起きる。

```bash
lsof -ti :8200 -sTCP:LISTEN     # PID を調べる
kill <PID>
```

2026-09-27 19:15 に実際に遭遇した。`nohup` で起動したプロセスが当時の PID を保持していたため。

### 4.3 作業フォルダの取り違え

- 8200 と 8201 はリポジトリのルート（`~/workdir/Sai/sai-moon`）で起動する
- 8202 は `apps/radio` で起動する

この違いを取り違えると `Could not import module "sai_radio"` になる。

---

## 5. 残っている課題

### 5.1 論文 2 の音声は旧スタックで作られたまま

PROJECTROOT の論文 2（*Fast simulations of highly-connected spiking cortical models using GPUs*）の音声ファイル `.sai/paper/papers/0002_.../radio.m4a`（3.9MB）は、旧 sai-radio（8000）で作られたもの。

- sai-moon 側（8201）の DB には `test-moon-0927-2` の記録が無い
- 8201 側で `GET /api/papers/test-moon-0927-2` は 404 になる
- Web 画面の音声再生は 8201 を中継する（`apps/phone/src/sai_phone/main.py:972` の `/api/radio/{path}`）ため、今この論文の音声は再生できない

**治し方**: 作り直す。

```bash
cd PROJECTROOT
SAI_PAPER_ROOT=$PWD \
  ~/workdir/Sai/sai-moon/target/release/sai-moon-paper phone radio 2
```

または Web 画面（論文 → Radio）から作成。作成には数分かかり、`radio.m4a` は上書きされ、LLM 費用が発生する。

### 5.2 旧スタック（8000 / 8600）が動いている

`~/workdir/Sai/sai-radio` の 8000（radio）と 8600（tts_proxy）が今も起動している。停止していない。使わないなら:

```bash
kill $(lsof -ti :8000) $(lsof -ti :8600)
```

### 5.3 3 サーバーは手動起動

`sai-moon/apps/phone/launchd/com.sai-moon.plist` には launchd 用の plist があるが、launchd には load していない。Mac を再起動すると 8200 / 8201 / 8202 すべて落ちる。自動起動化はしていない（利用者判断）。

### 5.4 合言葉の入力欄が同時に出る問題（コード側）

token を有効に戻すなら `app.js` の改修が要る。入力欄を `apiFetch` の中に 1 つだけ持ち、同時に 401 が来ても 1 個だけ表示し、入力されたら待っている全リクエストを新しい合言葉で再送する方式（単一のプロミスを共有）。現在は token を無効にして回避している。

### 5.5 cc フックが二重に登録されている

`~/.claude/settings.json` の旧フック（8100 向け）が残っているため、cc で質問・承認を出すと 8200 と 8100 の両方に通知が飛ぶ。プロジェクト側（8200 向け）を足したのは 2026-09-27。古い方を消すか、切り替える時期を決めておく。

### 5.6 pi 側は変更していない

`~/.pi/agent/extensions/ask_user_phone.ts` は既に sai-moon 版で 8200 を指している（`SAI_PHONE_URL = "http://127.0.0.1:8200"`。`sai-moon/apps/phone/src/sai_phone/extensions/ask_user_phone.ts` と同一）。

プロジェクトに置くなら `<PROJECTROOT>/.pi/settings.json` に:

```json
{ "extensions": ["/Users/touta/workdir/Sai/sai-moon/apps/phone/src/sai_phone/extensions/ask_user_phone.ts"] }
```

ただし 2 つの制約がある（どちらも実測）。

1. **グローバル側を消さないと起動に失敗する。** 同名ツールが 2 つあると
   `Tool "ask_user_phone" conflicts with <project 側>` で起動しない
2. **`.pi` は trust の承認が要る。** `pi -p --approve` を付けないとプロジェクトリソースを読まない（実測、道具が「無し」になる）。sai-link は pi を pane で自動起動するが `pi_command.rs` は `--approve` を渡していない（`crates/sai-link-engine/src/pi_command.rs:361-395`）。初回は pane 内で「Trust project?」が出て止まる。`/trust` で `~/.pi/agent/trust.json` に記録すれば 2 回目以降は出ない

---

## 6. 検証記録（2026-09-27 実施）

- 8200 の各 API が 200（`/api/projects` `/api/projects/…/tree` `/api/projects/…/dashboard` `/api/home` `/api/radio/papers` `/api/questions/pending`）。503 なし
- 8202 に `voice=888753760` と `voice=1310138977` の両方で 200
- 8201 に `POST /api/papers/from-extract`（mode=podcast）を投げ、201 を受け取った。113 秒で `radio.m4a` 2.3MB が出力された
- 生成した音声が `GET /api/papers/{id}/audio`（8201 経由）と `GET /api/radio/papers/{id}/audio`（8200 経由の両方で 200、サイズも同じ
- テストで作った論文は DB から削除済み（8201 の一覧は `[]` に戻っている）
- iPhone（100.126.129.108）から 4444 番経由で 8200 に到達できる（`tailscale ping iphone-se-gen-3` で応答あり）
