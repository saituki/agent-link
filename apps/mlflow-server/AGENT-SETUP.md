# AGENT-SETUP: agent に mlflow を使わせる

このファイルは、**手元の Mac / LAN の PC 上で動く agent（pi・cc）に mlflow を使わせる**ための手順書である。
リモートの実行先（sakura など）から書くジョブ向けではないので、ポートや接続先は手元のものである。

- 指示文の中身: `templates/mlflow-agent.md`（このフォルダ）
- サーバの構成と運用: `README.md`（このフォルダ）

## 1. 置き場

agent の指示文（テンプレート）は `~/.sai-moon/template/` に置く。sai-moon の共通テンプレの置き場である。

```bash
mkdir -p ~/.sai-moon/template
cp src/templates/mlflow-agent.md ~/.sai-moon/template/mlflow-agent.md
ls -l ~/.sai-moon/template/mlflow-agent.md
```

## 2. 接続情報を agent に渡す

接続情報は **平文で書かず**、設定の `env` に書く。値は `mlflow-server/.env` と同じものを使う（キーの値はこの文書に載せない）。

対象プロジェクトの `.sai/settings.json`（または `~/.sai-moon/settings.json`）の `env` に 5 つのキーを足す。

| キ | Mac での値 | どこから取るか |
|---|---|---|
| `MLFLOW_TRACKING_URI` | `http://127.0.0.1:5001` | `mlflow-server/.env` |
| `MLFLOW_S3_ENDPOINT_URL` | `http://127.0.0.1:9000` | `mlflow-server/.env` |
| `AWS_ACCESS_KEY_ID` | MinIO のアクセスキー | `mlflow-server/.env` |
| `AWS_SECRET_ACCESS_KEY` | MinIO のシークレットキー | `mlflow-server/.env` |
| `AWS_DEFAULT_REGION` | `us-east-1` | `mlflow-server/.env` |

`5000` 番は macOS の AirPlay レシーバーが使っているため、**Mac 側では必ず 5001** を使う。

```json
{
  "env": {
    "MLFLOW_TRACKING_URI": "http://127.0.0.1:5001",
    "MLFLOW_S3_ENDPOINT_URL": "http://127.0.0.1:9000",
    "AWS_ACCESS_KEY_ID": "<mlflow-server/.env の値>",
    "AWS_SECRET_ACCESS_KEY": "<mlflow-server/.env の値>",
    "AWS_DEFAULT_REGION": "us-east-1"
  }
}
```

これを足したら sai-moon の daemon を再起動する。**再起動するまで agent の起動コマンドには付かない。**

## 3. ブロック定義に載せる

```yaml
templates: sai-link,sai-docs,mlflow-agent
```

これでそのブロックのエージェントの指示文の先頭に `mlflow-agent` が入る。

## 4. 手元で確かめる（この 4 つを順番に実行する）

前提: 上の 1〜3 が終わっていて、`env` が現在のシェルにも入っていない場合は先に読み込む。

### 1) サーバがここにあるか

```bash
curl -s "$MLFLOW_TRACKING_URI/version"
```

期待: `3.16.0` のような版が返る。空なら `docker compose -f mlflow-server/docker-compose.yml ps` でコンテナを見る。

### 2) 記録できる experiment があるか / 無いなら作る

```bash
uv run --no-project --python 3.14 --with mlflow==3.16.0 python - <<'PY'
from mlflow.tracking import MlflowClient
c = MlflowClient()
for e in c.search_experiments(max_results=100):
    print(e.experiment_id, e.name, e.artifact_location)
PY
```

期待: `1 local s3://mlflow-local/` が 1 行出る。`local` が無いときは `templates/mlflow-agent.md`（このフォルダ） §2 のコードで作成する（`get_experiment_by_name` が `None` のときだけ作るので、**何度走らせても experiment は増えない**）。

### 3) 1 run 記録して読み返す

`templates/mlflow-agent.md`（このフォルダ） §3 のコードをそのまま実行する。run 名は `agent-check`。

期待: run_id とタグの辞書が 1 行出る。`sai.project` と `sai.run_id` がタグに入っている（この 2 つは `SAI_PROJECT_ROOT` / `SAI_PARENT_RUN_ID` があるときだけ付くので、無い環境では現れない）。

### 4) 成果物が MinIO に並んでいるか

ブラウザコンソールは無いので `boto3` でバケットの中を見る。

```bash
uv run --no-project --python 3.14 --with boto3 python - <<'PY'
import os
import boto3
from botocore.config import Config

s3 = boto3.client("s3", endpoint_url=os.environ["MLFLOW_S3_ENDPOINT_URL"],
                  config=Config(signature_version="s3v4"))
print("buckets:", [b["Name"] for b in s3.list_buckets()["Buckets"]])
r = s3.list_objects_v2(Bucket="mlflow-local", MaxKeys=20)
for o in r.get("Contents", []):
    print(o["Key"], o["Size"])
PY
```

期待: `buckets: ['mlflow-local']` と、`<run_id>/artifacts/agent-check.txt` のようなキーが 1 つ以上出る。
出ないときは 3 の `log_artifact` が `boto3` なしで動いていない（`--with boto3` が入っているか）、または MinIO のバケット名と `artifact_location` が食い違っている。

## 5. 記録した run のタグを確認する

3 で記録した run に `sai.project` / `sai.run_id` が入っていることの確認（`search_runs` で見、`mlflow.kind` などの特別タグが無い run も全部見る）。

```bash
uv run --no-project --python 3.14 --with mlflow==3.16.0 python - <<'PY'
from mlflow.tracking import MlflowClient
c = MlflowClient()
e = c.get_experiment_by_name("local")
for r in c.search_runs([e.experiment_id], order_by=["attributes.start_time DESC"], max_results=10):
    sai = {k: v for k, v in r.data.tags.items() if k.startswith("sai")}
    print(r.info.run_id[:8], r.info.status, r.data.tags.get("mlflow.runName", "-"), sai)
PY
```

期待: `agent-check` の行に `{'sai.project': ..., 'sai.run_id': ...}` が出て、`info.status` が `FINISHED` になっている（`mlflow.end_run()` を呼んでいる証拠）。
`sai.project` / `sai.run_id` が出ない場合は、`SAI_PROJECT_ROOT` / `SAI_PARENT_RUN_ID` がそのシェルに入っていない。

## 6. 実験の表（作るたびに 1 行足す）

| experiment 名 | `artifact_location` | 使う人 |
|---|---|---|
| `local` | `s3://mlflow-local/` | Mac / LAN の PC 上の agent |
| `dok` | `s3://hps-data-2026/saito260927/mlflow/` | さくら 高火力DOK のジョブ（フェーズ2・U04） |

新しい experiment は **必ず** `MlflowClient().create_experiment(name, artifact_location=...)` で作る。
`mlflow.set_experiment(name)` は保存先が `--default-artifact-root` になるため使わない。

## 7. 使えないとき

| 症状 | 対処 |
|---|---|
| `curl` が空 / `Connection refused` | `docker compose -f mlflow-server/docker-compose.yml ps`。 stopped なら `up -d` |
| 5000 に繋がらない | 5000 は AirPlay。`5001` を使う |
| `NoCredentialsError` / `Unable to locate credentials` | `env` の 5 つのキーがそのシェルに入っていない |
| `Experiment 'local' does not exist` | §4 の 2 のコードで作成する |
| `~/.sai-moon/template/` に置いても反映されない | daemon を再起動する。frontmatter の `templates:` に `mlflow-agent` があるか見る |
| `uv run` が `No interpreter found` とエラーになる | `~/.python-version` が未インストールの版を指している。コマンドの `--python 3.14` のところを変える |
