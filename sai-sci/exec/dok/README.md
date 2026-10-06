> **sai-moon に取り込んだ文書**（sai-ssh の `src/dok/README.md`）。実験ループからは `adapter` が `ssh-run` を使う（下の「実験ループからの使い方」）。
> 以下の `python3 dok.py …` は、この `.sai/exec/dok/` で打つ。テスト用の image・スクリプト（`Dockerfile`・`pytorch_ssh_test.sh`）は取り込んでいない。
> 本文に旧 sai-ssh のフォルダ名・`src/…` が残る所は、読み替えること。

# DOK（さくら 高火力DOK）テストの経路と実行手順

この書は、Mac の手元 mlflow へ DOK コンテナから記録するための経路と、実行・停止の手順をまとめたものである。
コマンドは原則リポジトリのルートで実行する（`python3 dok.py` は `PYTHONPATH=src` が要るため）。

**出荷状態の Mac から 1 から設定する手順は [`SETUP.md`](SETUP.md)。** この文書は経路と実行の詳細。

## 1. これは何か

DOK（さくら オブジェクトストレージのコンテナ実行サービス）で GPU ジョブを走らせ、
メトリクスを mlflow へ記録し、成果物は S3 へ直接書く。その全体図。

```
[Mac の手元]  mlflow (127.0.0.1:5001)  ← Caddy (:8443, Basic 認証)
     ▲                                        ▲
     │ https 443                             │ 平文 HTTP
     │ TLS は Funnel が終端                    │
[インターネット]                          [Mac]
     ▲
     │
[DOK コンテナ]  ── 成果物（result.json ほか）──▶  さくら オブジェクトストレージ
     │                                             bucket: hps-data-2026
     │                                             prefix: saito260927/dok-test/<task_id>/
     └── イメージは さくらコンテナレジストリ test-saito.sakuracr.jp から取得
```

記録の流れ:

1. テストのコンテナイメージを `test-saito.sakuracr.jp/dok-test:0.1` として push する。
2. DOK にタスクを登録する。コンテナは内部ネットワークから
   `https://macbook-pro.tail3ab809.ts.net`（＝ Tailscale Funnel の https 443）に mlflow だけを書き込む。
   それ以外は mlflow を通さず、結果を S3 へ直接書く。
3. 出力は標準出力に 1 項目 1 行の JSON、終わりに `{"summary": {...}}`。
   同じ内容を S3 の `saito260927/dok-test/<task_id>/result.json` にも保存する。
4. `run` は終了時に必ずタスクを cancel する（課金を止めるため）。

**重要**: Funnel は mlflow を**公開インターネット**に出す。守りは Caddy の Basic 認証だけ。
そのため Funnel は**テストの間だけ開ける**運用にし、開いたまま放置しない。

## 2. 事前準備（1 回だけ）

### 2.1 Tailscale に Funnel を許可する

Tailscale 管理画面で、この Mac に `funnel` の nodeAttrs を付与する。
確認:

```bash
tailscale status --json | python3 -c 'import json,sys;print(json.load(sys.stdin)["Self"]["Capabilities"])'
```

`"funnel"` を含む項目が出ればよい。

- この Mac には、2026-09-29 にユーザーが付与した（`Self.Capabilities` に `funnel` が出る）。付与前は `Self.Capabilities` に `funnel` 系が無く、Funnel を開けなかった。
  付与はこの手順書だけでは完結せず、ユーザーが Tailscale 管理画面で ACL を変える必要がある。
  未付与のまま `tailscale funnel` を実行するとエラーになる。

### 2.2 DOK の利用規約に同意する

DOK の管理画面で利用規約に同意する。API `POST /auth/agree/` は CLI には無いので画面からしかできない。
同意済みの確認:

```bash
python3 dok.py auth
```

出力の `tos_agreed_at` が `null` でなければ同意済み。

### 2.3 レジストリの認証情報を用意する

Docker が `test-saito.sakuracr.jp` にログイン済みであること。
パスワードの実値は文書に書かない。環境変数 `DOK_REGISTRY_PASSWORD` でだけ渡す（4. の手順 3 参照）。

### 2.4 `dok` experiment を作る

```bash
cd mlflow-server && docker compose run --rm init
```

2 回走らせても増えない（`get_experiment_by_name` が `None` のときだけ作る）。
`dok` の `artifact_location` は `s3://hps-data-2026/saito260927/mlflow/`。

## 3. Funnel の開閉

既存の `serve`（`https://macbook-pro.tail3ab809.ts.net:4443` → `127.0.0.1:8100`、
`:4444` → `127.0.0.1:8200`）とは**ポートが違う**ので壊さない。
Funnel は https **443** → `http://127.0.0.1:8443`（Mac の Caddy）。

### 3.1 開く

```bash
tailscale funnel --https=443 --bg 8443
```

> **443 が使用中のとき**: 別の `tailscale funnel`（フォアグラウンド実行）が 443 を持っていると、
> `foreground listener already exists for port 443` で開けない。その場合は 443 の代わりに
> **10000**（Funnel で使える 443 / 8443 / 10000 の 1 つ）を使う。開く: `tailscale funnel --bg --https=10000 8443`、
> 閉じる: `tailscale funnel --https=10000 off`、`run` には `--mlflow-url https://macbook-pro.tail3ab809.ts.net:10000` を渡す。
> 2026-09-29 の実機テストは 10000 で成功した（開閉とも確認済み）。


- `--https=443` は「外から見える https ポート」を 443 にする指定。
  これを付けないと `--bg 8443` は https **8443** を公開してしまう。
- `--bg` はバックグラウンドで開く（閉じずにシェルに戻る）。

### 3.2 状態を見る

```bash
tailscale funnel status
```

`https://macbook-pro.tail3ab809.ts.net:443 (tailscale funnel)` と
`|-- / proxy http://127.0.0.1:8443` が出ていれば開いている。

### 3.3 閉じる

```bash
tailscale funnel --https=443 off
```

- **（要確認）** `tailscale funnel --help` の USAGE は
  `tailscale funnel <target>` / `funnel status` / `funnel reset` しか列挙しておらず、
  `off` を target として受け付けるかは help からは確認できなかった（v1.102.4）。
  実際の用法は `https://tailscale.com/kb/1242/tailscale-serve` に従う。閉じる前に
  `tailscale funnel status` で 443 が消えたことを確認する。

### 3.4 やってはいけないこと

```bash
tailscale funnel reset    # ← 既存 Funnel も全部消える
tailscale serve reset     # ← 既存の serve 4443 / 4444 も消える
```

443 だけを落としたいときは 3.3 の `off` を使う。

### 3.5 外側から確かめる

認証なしで通っては**いけない**。

```bash
# → 401 が返る（そうでないなら閉める）
curl -s -o /dev/null -w '%{http_code}\n' https://macbook-pro.tail3ab809.ts.net/version

# → {"version": "3.16.0"} 形式の JSON が返る
curl -s -u "mlflow:<平文>" https://macbook-pro.tail3ab809.ts.net/version
```

**（要確認）** `-u` のユーザ名は Caddy の `MLFLOW_BASIC_USER`、
パスワードは平文の `MLFLOW_BASIC_PASSWORD`（Caddy は `MLFLOW_BASIC_PASSWORD_HASH` のハッシュを受け取るが、
認証する側は平文）。ここでは `mlflow` と仮置きしている。実値は `.env` にあるが、この文書には書かない。

**Basic 認証のパスワードは十分に長くすること。** 公開中の mlflow を守るのは Basic 認証だけである。

## 4. 実行手順

この順で、**リポジトリのルート**で実行する。

```bash
# 1) イメージ（U01。テストの中身を変更したときだけ）
docker buildx build --platform linux/amd64 -t test-saito.sakuracr.jp/dok-test:0.1 --push src/dok

# 2) Funnel を開く（3.1）
tailscale funnel --https=443 --bg 8443

# 3) レジストリ認証を DOK に登録（初回のみ。パスワードは環境変数でだけ渡す）
DOK_REGISTRY_PASSWORD='<パスワード>' python3 dok.py registry

# 4) 実行（終了時に必ず cancel する）
python3 dok.py run

# 5) 結果を S3 から読み返す（result.json）

# 6) Funnel を閉じる（3.3）
tailscale funnel --https=443 off
```

補足:

- 2 と 6 は必ず対にする。4 の途中で異常終了しても 6 は実行する。
- 3 は 1 を push し直した場合にもう一度実行する（認証情報の有効期限と、
  push し直した digest の再登録のため）。
- 4 は既定で `--mlflow-url https://macbook-pro.tail3ab809.ts.net`（Funnel の URL）。
  `--plan` / `--limit-sec` / `--poll-sec` / `--name` を付けて調整できる。

## 5. 停止と課金

- `run` は `finally` で**必ず** cancel する。Ctrl-C しても `finally` を抜けるので cancel される。
- 異常時にタスクが残っている場合の手動手順:

```bash
python3 dok.py list
# 出力の task_id を使って
python3 dok.py cancel <task_id>
```

- DOK の管理画面でも「実行中」が 0 件であることを確認する。
- 課金は**実行秒数**。V100 は最低 1 秒。イメージの取得と待ち時間は課金対象ではない。
- 同時実行は 1 タスクだけ。単価は `python3 dok.py prices`。

## 6. 見方

### 6.1 標準出力

1 項目 1 行の JSON が出る。

| キー | 意味 |
|---|---|
| `step` | ステップ名（`pull` / `upload` / `train` / `report` など） |
| `ok` | そのステップが成功したか |
| `sec` | そのステップの所要秒数 |
| `detail` | 補足。エラー時は理由 |

最後に `{"summary": {...}}` が 1 行出る。`summary` に全体の所要秒数と
`train_loss` の最終値が入る。

### 6.2 mlflow の画面

Funnel を閉じずに画面を見るなら `https://macbook-pro.tail3ab809.ts.net` へ。
`dok` experiment に次の run が見える。

- run 名: `dok-<task_id の先頭 8 桁>`
- `train_loss` に 20 点出る（途中の学習曲線）

### 6.3 S3

`bucket: hps-data-2026` / `prefix: saito260927/dok-test/<task_id>/result.json` に
コンテナが書き込んだ結果がある。実値の取得は 4. の手順 5。

## 7. つまずいたとき

| 症状 | 対処 |
|---|---|
| `tailscale funnel` が権限エラー | この Mac に `funnel` の nodeAttrs が無い。ユーザーが管理画面で ACL を変える（2.1）。 |
| 外から `200` が返る / `401` にならない | Basic 認証が効いていない。`MLFLOW_BASIC_USER` / `MLFLOW_BASIC_PASSWORD_HASH` が Caddy に入ってるか確認。通ってしまうなら**すぐ Funnel を閉じる**。 |
| `NoCredentialsError` | S3 の鍵が DOK コンテナに渡っていない。`aws_access_key_id` / `aws_secret_access_key` を含む。 |
| `SignatureDoesNotMatch` | リージョンと endpoint の不一致。S3 は `region_name="us-east-1"`、`endpoint_url="https://s3.isk01.sakurastorage.jp"`。異なるリージョンで署名するとこのエラー。 |
| `Experiment 'dok' does not exist` | 2.4 の `docker compose run --rm init` を実行する。 |
| タスクが `waiting` のまま | 既に別タスクが動いている（同時実行は 1 タスク）。`list` で確認し、必要なら `cancel` する。 |
| `result.json` が無い | コンテナが S3 まで届いていない。registry 認証（4. 手順 3）をやり直す。 |
| run 名が見つからない | mlflow の experiment 名は `dok`、run 名は `dok-<task_id の先頭 8 桁>`。S3 側の `task_id` と一致するか確認する。 |

## 8. SSH で流す（公開 PyTorch image）

`ssh-run` は、**公開の** PyTorch image を SSH 有効で起動し、手元の
`src/dok/pytorch_ssh_test.sh` を `ssh … 'bash -s'` に標準入力で流す。
`run` と同じく `finally` で必ず cancel する（Ctrl-C でも SIGTERM でも）。

特点:

- image は公開なので**レジストリ認証が要らない**（`registry` を API に送らない）。
  そのため 4. の手順 1（buildx）と 3（`registry`）は不要。
- 鍵は専用の `~/.ssh/dok_key`（パスフレーズなし・既存の鍵とは別）。
- `~/.ssh/known_hosts` は汚さない。`~/.ssh/known_hosts_dok` にだけ書く。
- 流入させるスクリプトの中身を `train.py` は mlflow の artifact として送る。

### 8.1 実行手順

```bash
# 1) Funnel を開く（3.1。10000 番で開く。443 は別のプロセスが使っている）

# 2) SSH 鍵を作って公開鍵を DOK に登録する（初回のみ。冪等）
python3 dok.py ssh-key

# 3) 実行（終了時に必ず cancel する）
python3 dok.py ssh-run \
  --script src/dok/pytorch_ssh_test.sh \
  --mlflow-url https://macbook-pro.tail3ab809.ts.net:10000 \
  --limit-sec 900 --hold-sec 780

# 4) 起動したタスクが全部終わっていることを確かめる
python3 dok.py list   # 「実行中のタスクは無い」

# 5) Funnel を閉じる（3.3）
```

`ssh-key` の表示は「鍵のパス」「公開鍵の指紋」「DOK 側の `ssh_key_id`」だけ。
秘密鍵の内容は読まない・表示しない。同じ公開鍵が登録済みなら `POST` しない。

### 8.2 実行時に流れること

`src/dok/pytorch_ssh_test.sh`（コンテナ内）が順に次を行う。

1. 必須の環境変数 7 個を名前だけで確認する（無いものは `exit 3`）。
2. `nvidia-smi --query-gpu=name,memory.total --format=csv,noheader`。
3. `pip install --no-cache-dir -q mlflow-skinny==3.16.0 boto3`。
4. `/tmp/work/train.py` を作る（heredoc。シェルの変数展開はしない）。
5. `python /tmp/work/train.py`: 乱数回帰を V100 上で 200 step 学び、
   `train_loss` を 20 点、`final_loss` と `gpu_mem_mb` を 1 点ずつ記録し、
   `train.py` と `metrics.json` を artifact として送る。
   最後に 1 行の JSON（`run_id` / `final_loss` / `points` / `gpu`）を出す。
   学習が進んでいなければ `exit 6`、experiment が無ければ `exit 5`、
   GPU が無ければ `exit 4`。

run 名は `dok-ssh-<SAKURA_TASK_ID の先頭 8 桁>`。experiment は `dok`（2.4 で作成済み）。
experiment は**作らない**（作ると `artifact_location` が `--default-artifact-root` になるため）。

### 8.3 終了コード

| コード | 意味 |
|---|---|
| 0 | スクリプトが成功し、タスクも止まった |
| 1 | スクリプト失敗 / ssh 失敗 / 中断 |
| 2 | 前確認で起動しなかった（スクリプト・鍵が無い、`--limit-sec` < 600、`--hold-sec` ≥ `--limit-sec`、公開鍵が DOK に無い） |
| 3 | 既に実行中のタスクがある（`run` と同じ） |
| 4 | タスクを止められなかった（**課金が続きうる**。`cancel <task_id>` を手で実行する） |

### 8.4 つまずいたとき（SSH）

| 症状 | 対処 |
|---|---|
| `ssh-key` は成功するのに `ssh-run` が「公開鍵が DOK に登録されていない」 | 登録が `is_active=false` になっている。`ssh-key` は冪等なので既存の id を表示して終わる。管理画面で有効にする、または `DELETE /ssh/keys/{id}/` してから登録し直す。 |
| `ssh の接続先` が表示されない | `--ssh-timeout-sec` を増やす。`running` になると `containers[0].ssh` に `host_name` と `port` が入る（それまで待つ）。 |
| 接続直後に `255` になる | 起動直後は ssh がまだ受け付けないことがある。CLI は既定で 5 秒おきに 6 回まで試す。それでも失敗したら上の表の `--ssh-timeout-sec` を確認する。 |
| スクリプトが 1 バイトも流れない | `ssh … 'bash -s'`（PTY なし）が通らない可能性がある。`dok.py` の `SSH_EXTRA_OPTIONS` に `-tt` を足す。実機で確認して報告する。 |
| 環境変数が足りない（`exit 3`） | SSH セッションのシェルに DOK の `environment` が入らない場合の症状。`bash -s` ではなく `bash -lc '…'` 形式に変更が必要かもしれない。 |

## 実機で分かった ssh-run の条件（2026-09-29）

- `status=running` でも、`containers[0].start_at` が入るまでは image の pull 中で、ssh のポートは開いてもシェルが無い。課金対象外で、PyTorch image では 10 秒〜8 分以上かかった。`ssh-run` は `start_at` を待つ（`--ssh-timeout-sec` 既定 1500）。
- DOK の ssh は標準入力が閉じるとセッションも閉じる。リモートコマンド（`bash -s`）も効かない。そのため `-tt` で対話シェルに入り、stdin を開いたまま、スクリプトを base64 で送って `bash` で実行し、目印 `__DOK_DONE_<n>__` で終わりを知る（`_drive_shell`）。
- コンテナの `environment` は ssh のシェルにも入る。
- 結果: V100 で `pytorch_ssh_test.sh` が成功（pip install 15 秒、20 点の `train_loss` を Funnel 経由で手元 mlflow に送信）。


---

## 実験ループからの使い方（sai-moon）

実験ループ（`sci-exp-branch`）は、分配器 `exp_exec.py` 経由でこの実行先を使う。`dok.py` を直接打つ必要はない。

1. `SETUP.md` の A・B（さくらのクラウドと Tailscale）を済ませ、`python3 dok.py ssh-key` で鍵を作って登録する。
2. 秘密（`SAKURA_API_ACCESS_TOKEN`・`SAKURA_API_SECRET_KEY`・`SAKURA_S3_ACCESS_KEY`・`SAKURA_S3_SECRET_KEY`・`MLFLOW_BASIC_USER`・`MLFLOW_BASIC_PASSWORD`）を、`~/.sai/dok.env`（権限 600）か `settings.json` の `env` に書く。`target.toml` の `env_file` に前者のパスを書く。
3. `target.toml` の `[env] MLFLOW_TRACKING_URI` に、コンテナから届く MLflow の URL を書く。`enable = true` にする。
4. 確かめる（課金されない）: 分配器の `check dok`。認証と鍵だけを見る。
5. 使う: `EXP_TARGET=dok`。branch が `run dok -- <コマンド>` を呼ぶと、cwd のコードが tar.gz で送られ、`setup` のあとにコマンドが動く。

**課金の安全**
- DOK は同時に 1 タスクだけ。動いている間は 2 本目の `run` を断る。
- 中止（`cancel`）は運転役へ SIGTERM を送る。`dok.py` が受けて DOK のタスクを止める。
- 運転役が異常に終わったとき（`dok.py` が終了コード 4 = タスクを止められなかった）は、状態が `failed` になり、ログに警告が残る。**そのときは `python3 dok.py list` と `cancel <task_id>` で手で確かめる。**
- ログは `~/.sai/exec/dok/<job_id>/log`。

**実験のコマンドの注意**
- コンテナの image には `uv` が無い。`target.toml` の `setup` で入れる（既定は `pip install -q uv`）。
- 送るのは cwd のコードだけ（`.git`・`.venv`・`__pycache__`・`node_modules`・`.sai`・`mlruns` は除く。既定 5 MB まで）。データは、コンテナから取れる場所（S3 など）に置く。
