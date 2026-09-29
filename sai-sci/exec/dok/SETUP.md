> **sai-moon に取り込んだ文書**（sai-ssh の `src/dok/SETUP.md`）。さくらのクラウド（A）・Tailscale（B）の設定は変わらない。
> C 以降の Mac の手順は、`sai-ssh` のフォルダを前提に書いてあった。今は `python3 dok.py …` を `.sai/exec/dok/`（または `sai-sci/exec/dok/`）で打ち、MLflow は `apps/mlflow-server/` で動かす。
> **MLflow を Tailscale Funnel で公開インターネットに出す手順が含まれる**（守りは Caddy の Basic 認証だけ）。sai-moon の README は「funnel を使わない」方針なので、テストの間だけ開けて、開いたまま放置しないこと。

# 1 から始める手順書: さくら 高火力 DOK ＋ 手元 Mac の mlflow

インストールしたばかりの Mac と、さくらのクラウドのアカウントだけがある状態から、
**「DOK のコンテナで深層学習を動かし、成果物はオブジェクトストレージへ、指標は手元 Mac の mlflow へ送る」**
ところまでを再現する手順書である。

- 書いてあるコマンドは、2026-09-29 に Apple Silicon の Mac（macOS 26 相当、Docker Desktop 29、Tailscale 1.102）で実際に動かしたものである。
- 画面操作（さくらのコントロールパネル、Tailscale 管理画面）は、公式マニュアルの記述に基づく。画面の文言が変わっている場合は、**（要確認）** のとおり、画面に合わせて読み替える。
- パスワード・鍵・トークンの実値は、この文書に書かない。`<...>` の所は自分の値に置き換える。

## 0. 全体像

```
[Mac]                                            [さくら]
 mlflow(:5001) ← Caddy(:8443, Basic 認証)          コンテナレジストリ  <名>.sakuracr.jp
       ▲              ▲                            オブジェクトストレージ  <バケット>
       │  Tailscale Funnel (https :10000)               ▲          ▲
       │              │                                 │成果物     │イメージ
       └──── インターネット ◀──── 高火力 DOK のコンテナ ─┘          │
              指標(train_loss など)      (GPU: V100)  ◀── DOK API ── Mac の python3 dok.py
```

| 誰が | 何をするか |
|---|---|
| Mac の `docker compose` | mlflow ＋ postgres ＋ rustfs(S3) ＋ Caddy を動かす（`mlflow-server/`） |
| Mac の `tailscale funnel` | Caddy を、テストの間だけインターネットへ出す |
| Mac の `python3 dok.py` | DOK の API でタスクを起動し、終了時に必ず止める（`dok.py`） |
| DOK のコンテナ | GPU で学習し、指標を Funnel 経由で Mac の mlflow へ、成果物をさくら S3 へ送る |

この構成の要点（実際にハマった所）:

- mlflow は `--no-serve-artifacts`（直接方式）。**成果物は mlflow を通らず、コンテナが S3 へ直接書く。** だからコンテナに S3 の鍵が要る。
- DOK のコンテナは tailnet の外にいる。Mac の mlflow へは **Funnel（公開）** で届く。守りは Caddy の Basic 認証だけ。**Funnel はテストの間だけ開ける。**
- 課金は DOK の実行秒数（V100 は 0.016 円/秒。最低課金 1 秒）。image の取得時間と待ち時間は対象外（公式）。

### 費用の目安と守り

- DOK: 1 回の試験で数円以下。タスクには必ず `execution_time_limit_sec`（最小 600 秒）を付け、CLI は終了時に必ず cancel する。900 秒で打ち切られても約 14 円。
- オブジェクトストレージ: バケットごとの月額基本料金（作成した時点から課金。公式）。
- コンテナレジストリ: 料金はサービスサイトで確認する（**要確認**）。

---

## A. さくらのクラウドの設定

コントロールパネル: <https://secure.sakura.ad.jp/cloud/>

### A1. アカウントとプロジェクト

1. さくらインターネットの会員 ID とさくらのクラウドの利用登録を済ませる。支払い方法（クレジットカードなど）を登録する。DOK は後払いで、法人ならプリペイドも選べる（公式 FAQ）。
2. 個別ユーザで操作する場合は、そのユーザに「オブジェクトストレージ」「コンテナレジストリ」「APIキー作成」の権限を付ける（ユーザ・プロジェクト機能）。会員 ID の本人なら不要。

### A2. API キーを作る（DOK の API 用）

1. コントロールパネル左のメニューから「APIキー」を選ぶ。
2. 「APIキーの作成」で、種類は「リソース操作用」、名前（例 `dok-cli`）、アクセスレベルを入力する。DOK は IaaS 以外のサービスなので、**「サービスへのアクセス権」にチェックを入れる**（要確認: 必要なアクセスレベルの最小値は確認していない。401 が返るときは、ここを疑う）。
3. 作成すると、**アクセストークン**と**アクセストークンシークレット**が表示される。**シークレットはこの画面でしか見られない。** すぐ控える（忘れたら再発行）。

> **取り違えに注意（実際に起きた）**: アクセストークンは UUID 形式（36 文字）、シークレットは 64 文字の文字列である。
> `.env` に書くとき `SAKURA_API_ACCESS_TOKEN` に UUID の方を入れる。
> 逆に書いても CLI が UUID 形式で見分けて入れ替え、警告を出す（`dok.py` の `_api_auth`）が、正しく書くこと。

### A3. 高火力 DOK を使えるようにする

1. コントロールパネルのメニューから「高火力 DOK」を開く。
2. **利用規約に同意する**（同意するまで API は他の操作を受け付けない）。API の `POST /auth/agree/` は法的な同意なので、この CLI には実装していない。画面から本人が行う（画面の文言は**要確認**）。
3. 確認は、Mac の手順（C10）の `python3 dok.py auth` で `tos_agreed_at` に日時が出ること。

DOK のゾーンは石狩第一（`is1a`）。同時に動かせるタスクは 1 プロジェクト 1 つ。

### A4. オブジェクトストレージ（データと成果物の置き場）

1. コントロールパネルの「オブジェクトストレージ」を開く。
2. 左メニュー「サイト」から、**石狩第1サイト**（エンドポイント `s3.isk01.sakurastorage.jp`、リージョン名 `jp-north-1`）を選び、利用を開始する。
3. 左メニュー「バケット」→「バケットの追加」でバケットを作る。この環境では `hps-data-2026`。
   - 名前は、小文字英字・数字・ダッシュの 3〜63 文字。**オブジェクトストレージ全体で一意**なので、取れないときは別名にする（別名にしたら、下の「自分の値に変えるとき」に従う）。
   - 作成した時点から月額の基本料金が発生する。
4. 左メニュー「パーミッション」→「パーミッションの追加」で、そのバケットに **READ/WRITE** を与えたアクセスキーを発行する。**シークレットアクセスキーはこのとき一度しか表示されない。** 控える。
   - サイト利用開始時に出るキーではなく、このパーミッションのキーを使うことが推奨されている（公式）。
   - 1 パーミッションにつきアクセスキーは 1 個まで。
5. 「フォルダ」（prefix）は作らなくてよい。この環境では `saito260927/` を prefix にしている（オブジェクトのキーに含めるだけで、自動で作られる）。

> S3 クライアントの設定は、`endpoint_url=https://s3.isk01.sakurastorage.jp`、署名は `s3v4`。
> リージョンは、公式の検証値は `jp-north-1` だが、この環境では **`us-east-1` を指定して動いた**（boto3、`AWS_DEFAULT_REGION=us-east-1`）。

### A5. コンテナレジストリ（自作 image の置き場）

公開の image（Docker Hub の `pytorch/pytorch` など）だけを使うなら、**この節は不要**。DOK 用の軽量テスト image（（Dockerfile は取り込んでいない。公開 image を使う `ssh-run` だけを使う））を使うときに作る。

1. コントロールパネル左「グローバル」の「コンテナレジストリ」→「追加」。
2. 「コンテナレジストリ名」を入力する（小文字英字・数字・ハイフン。英字で始まる。**作成後は変更できない**。他のユーザが使っている名前は使えない）。この環境では `test-saito`。接続先は `test-saito.sakuracr.jp`。
3. 作成後、レジストリの「ユーザ」タブで「追加」。ユーザ名（この環境では `saito`）とパスワードを決め、権限は **Push & Pull** 以上にする。
4. 公開設定は「非公開」のまま（2026-06-25 以降に作ったレジストリは、初期値が非公開）。

---

## B. Tailscale の設定（Mac ⇔ インターネットの入口）

### B1. アカウントを作り、tailnet に Mac を入れる

1. <https://login.tailscale.com/> でアカウント（tailnet）を作る。
2. Mac に Tailscale アプリを入れて（C3）、ログインする。Mac の名前が管理画面の「Machines」に出る。

### B2. MagicDNS と HTTPS 証明書を有効にする

Funnel には、MagicDNS ＋ HTTPS 証明書が要る（公式）。

1. 管理画面の「DNS」ページを開く。
2. MagicDNS が有効であることを確認する。
3. 「HTTPS Certificates」を有効にする。

この Mac の DNS 名を控える。**後で `--mlflow-url` に使う。**

```bash
tailscale status --json | python3 -c 'import json,sys; print(json.load(sys.stdin)["Self"]["DNSName"].rstrip("."))'
# 例: macbook-pro.tail3ab809.ts.net
```

### B3. Funnel を許可する（ACL）

管理画面の「Access controls」を開き、Funnel の節で「Add Funnel to policy」を押す。または、ポリシーファイルに次を足す。

```json
"nodeAttrs": [
  { "target": ["autogroup:member"], "attr": ["funnel"] }
]
```

Owner / Admin / Network admin の権限が要る。確認:

```bash
tailscale status --json | python3 -c 'import json,sys; c=json.load(sys.stdin)["Self"].get("Capabilities") or []; print([x for x in c if "funnel" in x.lower()] or "funnel 権限なし")'
```

`funnel` を含む項目が出れば良い（Funnel が使えるポートは **443 / 8443 / 10000** だけ）。

---

## C. Mac の設定（出荷状態から）

### C1. Xcode コマンドラインツールと Homebrew

```bash
xcode-select --install          # git と ssh / ssh-keygen が入る。ダイアログに従う
/bin/bash -c "$(curl -fsSL https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh)"
# 最後に表示される「PATH を通すコマンド」（Apple Silicon なら /opt/homebrew/bin/brew shellenv）を実行する
brew --version
```

### C2. Docker Desktop

```bash
brew install --cask docker
open -a Docker                  # 初回は許諾の画面が出る。クジラのアイコンが安定するまで待つ
docker version
docker compose version
docker buildx version
```

`docker version` の `Server` が出れば良い。Compose と buildx は Docker Desktop に入っている。

### C3. Tailscale アプリ ＋ CLI

```bash
brew install --cask tailscale   # または App Store / 公式サイトの版
open -a Tailscale               # メニューバーのアイコンからログイン（B1）
```

このドキュメントのコマンドは `tailscale` を使う。CLI の道が無いときは、次のラッパーを作る（この Mac ではこの形）。

```bash
sudo tee /usr/local/bin/tailscale >/dev/null <<'EOF'
#!/bin/sh
/Applications/Tailscale.app/Contents/MacOS/Tailscale "$@"
EOF
sudo chmod 755 /usr/local/bin/tailscale
tailscale version
tailscale status
```

（アプリの版によっては、設定に CLI を入れる項目がある。どちらでもよい。）

### C4. Python

CLI（`dok.py`）は標準ライブラリだけで動く。`src/pyproject.toml` は Python 3.10 以上を指定している。macOS 付属の python3 が古い場合に備えて入れる。

```bash
brew install python@3.12        # または: brew install uv
python3 --version               # 3.10 以上であること
```

`uv` を使う場合は、S3 の確認スクリプトで `uv run --no-project --python 3.14 --with boto3 python ...` の形が便利（Python が無ければ自動で取得される）。

### C5. リポジトリを Mac に入れる

このリポジトリ（`sai-ssh`）には、**git のリモートが設定されていない**（2026-09-29 時点）。新しい Mac へは、次のどれかで移す。

```bash
# 例: 旧 Mac で bundle を作り、新しい Mac にコピーする
git -C ~/workdir/Sai/sai-ssh bundle create /tmp/sai-ssh.bundle --all
# 新しい Mac:
mkdir -p ~/workdir/Sai && git clone /path/to/sai-ssh.bundle ~/workdir/Sai/sai-ssh
```

または、プライベートな GitHub リポジトリを作って push / clone する。**`.env` は git に入れない**（`.gitignore` 済み）。

以降、コマンドは、特に書いていなければ次の場所で実行する。

```bash
cd ~/workdir/Sai/sai-ssh
git switch feat/mlflow-agent-loop   # DOK の CLI が入っているブランチ（マージ後は main など）
```

### C6. 秘密の設定ファイルを 2 つ作る

**どちらも git に入れない。** 権限を絞る。

#### (1) mlflow のサーバ用 `mlflow-server/.env`

```bash
cd ~/workdir/Sai/sai-moon/apps/mlflow-server
cp .env.example .env
chmod 600 .env

# Caddy の Basic 認証のハッシュを作る（入力する平文がパスワード。長く推測されにくいものにする）
docker run --rm -it caddy:2 caddy hash-password
```

`.env` を開き、次を埋める（`.env.example` の他のキーはそのままでよい。`MLFLOW_VERSION` は `3.16.0`）。

| キー | 値 |
|---|---|
| `MLFLOW_VERSION` | `3.16.0`（Dockerfile とクライアントの版をそろえる） |
| `POSTGRES_USER` / `POSTGRES_PASSWORD` / `POSTGRES_DB` | 好きな値（DB 名は例 `mlflow`） |
| `MINIO_ROOT_USER` / `MINIO_ROOT_PASSWORD` | 好きな値（Mac 内の S3 互換ストレージ用） |
| `MLFLOW_BASIC_USER` | Basic 認証のユーザ名（例 `mlflow`） |
| `MLFLOW_BASIC_PASSWORD` | 上で入力した**平文** |
| `MLFLOW_BASIC_PASSWORD_HASH` | `caddy hash-password` の出力（`$2a$…` で始まる 1 行） |

**Funnel でインターネットに出るので、Basic 認証のパスワードは長い乱数にする**（パスワードマネージャで作る）。

#### (2) DOK の CLI 用 `.env`（リポジトリのルート）

```bash
cd ~/workdir/Sai/sai-ssh
touch .env && chmod 600 .env
```

次の 4 つを書く（`KEY=値` の形。引用符は付けても付けなくてもよい。キー名の大文字小文字は区別されない）。

| キー | 値の出どころ |
|---|---|
| `SAKURA_API_ACCESS_TOKEN` | A2 の**アクセストークン**（UUID 形式） |
| `SAKURA_API_SECRET_KEY` | A2 の**アクセストークンシークレット**（64 文字） |
| `SAKURA_S3_ACCESS_KEY` | A4 で発行したアクセスキー ID |
| `SAKURA_S3_SECRET_KEY` | A4 のシークレットアクセスキー |

CLI は `./.env` と `./mlflow-server/.env` の両方を読む（環境変数が優先）。Basic 認証（`MLFLOW_BASIC_USER` / `MLFLOW_BASIC_PASSWORD`）は (1) から読まれる。

### C7. mlflow を起動する

```bash
cd ~/workdir/Sai/sai-moon/apps/mlflow-server

docker compose run --rm checkenv                      # 必須キーが埋まっているか
docker network inspect mlflow-net-test >/dev/null 2>&1 || docker network create mlflow-net-test
docker compose up -d --build                          # 初回は image の取得とビルドに数分かかる
docker compose ps                                     # postgres / minio / mlflow / caddy が Up
```

確認:

```bash
curl -s http://127.0.0.1:5001/version                 # {"version": "3.16.0"} のような JSON
source .env
curl -s -o /dev/null -w '%{http_code}\n' http://127.0.0.1:8443/version                     # 401（認証なし）
curl -s -u "$MLFLOW_BASIC_USER:$MLFLOW_BASIC_PASSWORD" http://127.0.0.1:8443/version       # 200 の JSON
```

- 5000 番は macOS の AirPlay レシーバーが使うので、Mac 側は **5001** を使う（compose の設定済み）。
- MinIO の代わりに `rustfs/rustfs` を使っている（`minio/minio` が配布終了したため）。ブラウザのコンソールは使えない。

### C8. experiment を作る

```bash
cd ~/workdir/Sai/sai-moon/apps/mlflow-server
docker compose run --rm init        # 2 回走らせても増えない
docker compose run --rm smoke       # JSON が 1 行出れば、Mac 内の記録は動いている
```

`init` の出力に、次の 2 行が出る。

```
local   s3://mlflow-local/
dok     s3://hps-data-2026/saito260927/mlflow/
```

`dok` の保存先は、**A4 のバケットと prefix に合わせる**（`mlflow-server/scripts/init_experiments.py` の `EXPERIMENTS`）。

experiment は必ず `MlflowClient.create_experiment(name, artifact_location=...)` で作る。`mlflow.set_experiment` は保存先が `--default-artifact-root` になってしまう。

### C9. DOK 用のテスト image を build して push する（自作 image を使うときだけ）

公開の PyTorch image だけを使う（D2）なら、この節は飛ばしてよい。

Mac は arm64 だが、**DOK のホストは `linux/amd64`**。付け忘れると DOK で動かない。

```bash
docker login test-saito.sakuracr.jp            # A5 のユーザ名とパスワード
cd ~/workdir/Sai/sai-ssh
docker buildx build --platform linux/amd64 \
  -t test-saito.sakuracr.jp/dok-test:0.1 --push src/dok
docker buildx imagetools inspect test-saito.sakuracr.jp/dok-test:0.1     # Platform: linux/amd64
```

`docker login` は macOS の Keychain（Docker Desktop）に保存される。

### C10. DOK の API に繋がるか確かめる

```bash
cd ~/workdir/Sai/sai-ssh
export PYTHONPATH=src
python3 dok.py auth        # tos_agreed_at に日時、account_code が出る
python3 dok.py prices      # プランごとの単価。v100-32gb は 0.016 円/秒
python3 dok.py list        # 実行中のタスクは無い
```

- `HTTP 401` → A2 のキーを見直す（アクセストークンとシークレットの取り違え、アクセス権）。
- `tos_agreed_at` が空 → A3 の利用規約に同意する。
- `警告: SAKURA_API_ACCESS_TOKEN と SAKURA_API_SECRET_KEY が逆…` → `.env` の 2 行を入れ替える。

### C11. レジストリ認証を DOK に登録する（自作 image を使うときだけ）

```bash
cd ~/workdir/Sai/sai-ssh && export PYTHONPATH=src
DOK_REGISTRY_PASSWORD='<A5 のパスワード>' python3 dok.py registry
# registry_id=... (登録した)。2 回目以降は「既に登録済み」
```

パスワードは環境変数でだけ渡す（ファイルには書かない）。ユーザ名を変えたときは `--registry-user`。

### C12. ssh 用の鍵を作って DOK に登録する（ssh で入るときだけ）

```bash
cd ~/workdir/Sai/sai-ssh && export PYTHONPATH=src
python3 dok.py ssh-key
# ~/.ssh/dok_key（パスフレーズなしの ed25519）を作り、公開鍵を DOK に登録する。指紋と ssh_key_id が出る。
```

- **タスクを作る前に登録する**（後で登録した鍵は、そのタスクでは使えない）。
- パスフレーズなしで作るのは、自動実行のため。不要になったら DOK の公開鍵管理から削除し、`~/.ssh/dok_key*` も消す（F を参照）。

---

## D. 実行

Funnel を開く前に、**443 が別の `tailscale funnel` に使われていないか**確認する（フォアグラウンドの `tailscale funnel 3000` などが動いていると、443 は開けない）。

```bash
ps aux | grep "[t]ailscale funnel"
```

この文書では、443 の代わりに **10000 番**を使う（Funnel で使えるポートは 443 / 8443 / 10000）。

以降のコマンドの `<DNS 名>` は、B2 で控えた `macbook-pro.tail3ab809.ts.net` のような値に置き換える。

### D0. Funnel を開く（テストの間だけ）

```bash
tailscale funnel --bg --https=10000 8443       # https :10000 → Caddy (127.0.0.1:8443)
tailscale funnel status
curl -s -o /dev/null -w '%{http_code}\n' https://<DNS 名>:10000/version    # 401 が出ること（認証なしでは通らない）
```

`401` でなく `200` が返ったら、Caddy の認証が効いていない。すぐに閉じる（下）。

閉じる（テストが終わったら、必ず）:

```bash
tailscale funnel --https=10000 off
tailscale funnel status                        # 10000 が消えていること
```

**やってはいけない**: `tailscale funnel reset` / `tailscale serve reset`。他の serve / funnel 設定も全部消える。

### D1. 自作 image をバッチ実行する（`run`）

`dok_test.py` を 1 回起動して、GPU・S3・mlflow の 8 項目を試す。

```bash
cd ~/workdir/Sai/sai-ssh && export PYTHONPATH=src
python3 dok.py run --mlflow-url https://<DNS 名>:10000
```

期待する出力:

```
task_id=... status=waiting
status=running elapsed=...sec
status=done elapsed=...sec
{"step": "gpu", "ok": true, ..."Tesla V100-PCIE-32GB, 32768 MiB"}
{"step": "s3_roundtrip", "ok": true, ... "sha256 match"}
{"step": "mlflow_log", "ok": true, ... "20 metrics"}
{"summary": {"ok": 8, "failed": 0, ...}}
status=done exit_code=0
```

`--mlflow-url` は**必ず自分の値を渡す**（CLI の既定値は、この環境の DNS 名が書かれている）。

### D2. 公開 PyTorch image に ssh で入って実行する（`ssh-run`）

Docker Hub の `pytorch/pytorch` を、SSH ON で起動する。コンテナに入って `pip install mlflow-skinny`、Python スクリプトの作成、GPU での学習、指標の送信までを、`src/dok/pytorch_ssh_test.sh` が行う。

```bash
cd ~/workdir/Sai/sai-ssh && export PYTHONPATH=src
python3 dok.py ssh-run \
  --script src/dok/pytorch_ssh_test.sh \
  --mlflow-url https://<DNS 名>:10000
```

期待する流れ:

```
status=running ssh=準備完了 container=image 取得中 elapsed=6sec      ← image の取得中は課金されない
status=running ssh=準備完了 container=起動 elapsed=...
ssh の接続先: <task_id>@<...>.container.sakurausercontent.com:30022
root@<task_id>:/workspace# ...                                        ← コンテナ内のシェル
{"step":"pip","ok":true,"sec":15}
gpu=Tesla V100-PCIE-32GB torch=2.8.0+cu126
{"run_id": "...", "experiment": "dok", "final_loss": 0.016..., "points": 20, "gpu": "Tesla V100-PCIE-32GB"}
__DOK_DONE_0__
cancel しました (task_id=...)
status=canceled ssh_exit_code=0
```

- **image の取得に 10 秒〜10 分近くかかる**（PyTorch は約 3.75GB）。`container=image 取得中` の間は待つ（`--ssh-timeout-sec` 既定 1500）。この間は課金されない。
- `execution_seconds` に課金される。`--limit-sec`（既定 900、最小 600）が上限。
- ssh セッションを閉じてもタスクは止まらない。**CLI が最後に必ず cancel する**（`Ctrl-C` でも `SIGTERM` でも）。

### D3. 届いたことを確かめる

**mlflow（Mac 側）**

```bash
RID=<D2 の run_id>
curl -s "http://127.0.0.1:5001/api/2.0/mlflow/runs/get?run_id=$RID" | python3 -m json.tool | head -40   # status: FINISHED
curl -s "http://127.0.0.1:5001/api/2.0/mlflow/metrics/get-history?run_id=$RID&metric_key=train_loss" \
  | python3 -c 'import sys,json; m=json.load(sys.stdin)["metrics"]; print(len(m),"points", m[0]["value"], "->", m[-1]["value"])'
```

ブラウザなら <http://127.0.0.1:5001> を開き、experiment `dok` の run を見る。

**成果物（さくら S3）**

mlflow の API は成果物の一覧を返さない（直接方式のため）。S3 を直接見る。

```bash
cd ~/workdir/Sai/sai-ssh
uv run --no-project --python 3.14 --with boto3 python - <<'PY'
import sys, boto3
sys.path.insert(0, "src")
from sai_ssh import dok
s = dok.load_secrets(["./.env"], {})
c = boto3.client("s3", endpoint_url="https://s3.isk01.sakurastorage.jp", region_name="us-east-1",
                 aws_access_key_id=s["sakura_s3_access_key"], aws_secret_access_key=s["sakura_s3_secret_key"])
for o in c.list_objects_v2(Bucket="hps-data-2026", Prefix="saito260927/", MaxKeys=50).get("Contents", []):
    print(o["Key"], o["Size"])
PY
```

`saito260927/mlflow/<run_id>/artifacts/train.py` などが並ぶ。

### D4. テスト後に、動いている DOK コンテナがないことを確かめる（必須）

```bash
cd ~/workdir/Sai/sai-ssh && export PYTHONPATH=src
python3 dok.py list          # 「実行中のタスクは無い」
tailscale funnel status | grep -c 10000    # 0（D0 で閉じた）
```

まだ動いているタスクがあれば、`python3 dok.py cancel <task_id>`。コントロールパネルの「高火力 DOK」でも、「実行中」が 0 件であることを目で確かめる。

---

## E. つまずいたとき（実際に出たものを含む）

| 症状 | 原因と対処 |
|---|---|
| DOK API が `HTTP 401` | API キーの取り違え（A2 の注意）、アクセス権、キーの失効。`python3 dok.py auth` で確かめる |
| `tailscale funnel` が `foreground listener already exists for port 443` | 別の端末で `tailscale funnel <ポート>` がフォアグラウンドで動いている。**そのプロセスを止めるか、`--https=10000` を使う。** 止める前に、何を公開しているか確かめる |
| `tailscale funnel` が権限エラー | B3 の ACL（`funnel` の nodeAttrs）が未設定 |
| Funnel 経由の `/version` が `200`（認証なしで通る） | Caddy の認証が効いていない。すぐに `tailscale funnel --https=10000 off`。`docker compose logs caddy` を見る |
| Funnel 経由が `503` / 接続できない | Funnel の HTTPS 証明書が未発行（B2）。数分待つ。`curl` は Mac 自身からでなく別の回線で試す方が確実 |
| `ssh-run` で ssh が `Connection refused` | `running` になった直後は ssh がまだ受け付けない。CLI が自動で待つ |
| ssh が繋がるのに何も出ず、すぐ終わる（終了コード 0） | コンテナがまだ起動していない（`container=image 取得中`）。`start_at` が入るまで待つ。CLI は待つ作りになっている |
| ssh がスクリプトを実行せず閉じる | DOK の ssh は、標準入力が閉じるとセッションも閉じ、リモートコマンドも効かない。CLI は対話シェルに入って base64 でスクリプトを送る方式にしてある（`_drive_shell`） |
| コンテナ内で `NoCredentialsError` | `environment` が渡っていない（`SAKURA_` 始まりの名前は予約されているので使えない。CLI は `AWS_*` に写している） |
| `mlflow_log` が `Experiment 'dok' does not exist` | C8 の `init` を実行していない |
| `mlflow_log` が 401 | `mlflow-server/.env` の `MLFLOW_BASIC_PASSWORD` とハッシュが別の平文から作られている |
| `SignatureDoesNotMatch` | S3 のリージョンとエンドポイント。`endpoint=https://s3.isk01.sakurastorage.jp`、`region=us-east-1`（この環境で動いた値） |
| タスクが `waiting` のまま進まない | 別のタスクが動いている（同時 1 つまで）。`list` で確かめる |
| `Bucket ... does not exist` | バケット名の綴り。A4 のバケットにパーミッション（READ/WRITE）が付いているか |
| コンテナが `linux/arm64` で動かない | `--platform linux/amd64` を付けずに build した（C9） |
| `docker compose` で `external network mlflow-net-test not found` | `docker network create mlflow-net-test` |
| Mac 側 5000 番に繋がらない | AirPlay が使っている。5001 を使う |

---

## F. 片付け

| やること | コマンド・操作 |
|---|---|
| Funnel を閉じる | `tailscale funnel --https=10000 off` |
| DOK のタスクを止める | `python3 dok.py list` → `cancel <id>` |
| ssh 鍵を消す | DOK の公開鍵管理から削除し、`rm ~/.ssh/dok_key ~/.ssh/dok_key.pub ~/.ssh/known_hosts_dok` |
| テストで作った S3 のオブジェクトを消す | `saito260927/dok-test/`、`saito260927/mlflow/<run_id>/` 以下（コントロールパネルか S3 API） |
| mlflow を止める | `cd mlflow-server && docker compose stop`（データは残る）。`docker compose down -v` は**全データが消える** |
| バケット・レジストリ | 月額課金が続く。不要なら削除する（バケットは中身を空にしてから） |
| API キー | 使わないなら、コントロールパネルの「APIキー」から削除 |

---

## G. 自分の値に変えるとき

この環境の固定値（バケット `hps-data-2026`、prefix `saito260927`、レジストリ `test-saito`、ユーザ `saito`、Mac の DNS 名 `macbook-pro.tail3ab809.ts.net`）は、次の場所に書かれている。別の名前で作った場合は、ここを直す。

| 何 | 場所 |
|---|---|
| バケット名・prefix・S3 エンドポイント・リージョン（コンテナに渡す環境変数） | `dok.py` の `build_environment`（`DOK_BUCKET` `DOK_PREFIX` `MLFLOW_S3_ENDPOINT_URL` `AWS_DEFAULT_REGION`） |
| 自作 image 名、レジストリのユーザ名、mlflow の URL の既定値 | `dok.py` の冒頭の `DEFAULT_IMAGE` `DEFAULT_REGISTRY_USER` `DEFAULT_MLFLOW_URL`（`--image` `--registry-user` `--mlflow-url` でも上書きできる） |
| `dok` experiment の `artifact_location` | `mlflow-server/scripts/init_experiments.py` の `EXPERIMENTS`（変えたら `init` を実行。既に作った experiment は `artifact_location` を変えられないので、別名で作る） |
| プラン（GPU の種類） | `--plan v100-32gb`（既定）。`h100-80gb` は最低課金 60 秒で単価が高い |
| ゾーン | `--zone is1a`（石狩第一）。DOK の提供ゾーンはマニュアルで確認する |

---

## H. 参考（公式）

- 高火力 DOK: <https://manual.sakura.ad.jp/cloud/manual-koukaryoku-container.html>（SSH: `koukaryoku-container/use-ssh.html`、仕様: `specification.html`、料金: `pricing.html`）
- DOK API: <https://manual.sakura.ad.jp/koukaryoku-dok-api/spec.html>
- API キー: <https://manual.sakura.ad.jp/cloud/api/apikey.html>
- オブジェクトストレージ: <https://manual.sakura.ad.jp/cloud/objectstorage/about.html>、<https://manual.sakura.ad.jp/cloud/objectstorage/basic.html>
- コンテナレジストリ: <https://manual.sakura.ad.jp/cloud/appliance/container-registry/index.html>
- Tailscale Funnel: <https://tailscale.com/kb/1223/funnel>
- このリポジトリの実装: `dok.py`、`src/dok/README.md`（経路の詳細）、`mlflow-server/README.md`（mlflow サーバ）
