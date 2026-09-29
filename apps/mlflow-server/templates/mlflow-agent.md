# mlflow: 実験を記録する

## 1. 使う場面

- 深層学習・シミュレーションの実験はすべて mlflow に記録する。
- 記憶は引き継がれない。**前回の実験の数値は必ず mlflow から読み直して**から次のパラメータを決める。
- 接続情報は `env` で渡されている（追跡 URI、S3 のエンドポイント、AWS の認証情報、リージョン）。export し直さない。
- ライブラリは入っていないので `uv run --no-project --python 3.14 --with mlflow==3.16.0 python` で動かす（サーバの版に合わせる）。

## 2. 最初の 1 回だけ実行する（experiment を作る）

- 既にある名前かどうかで分岐する。**`set_experiment` で新規を作らない**（保存先が `--default-artifact-root` になり壊れる）。
- 共有契約の experiment 名は `local`、保存先は `s3://mlflow-local/`。

```bash
uv run --no-project --python 3.14 --with mlflow==3.16.0 python - <<'PY'
from mlflow.tracking import MlflowClient

client = MlflowClient()          # MLFLOW_TRACKING_URI を読む
name = "local"
exp = client.get_experiment_by_name(name)
if exp is None:
    exp_id = client.create_experiment(name, artifact_location="s3://mlflow-local/")
else:
    exp_id = exp.experiment_id
print("experiment_id =", exp_id)
PY
```

- 作り終えたら `mlflow-server/AGENT-SETUP.md` の experiment の表に名前を書き足しておく。

## 3. run を記録する

- 使い捨ての `train.py` などに数行書く。**run を始めたらすぐ `sai.project` と `sai.run_id` の 2 つのタグを付ける**（途中で落ちても、どの実行の記録かが分かる）。
- パラメータは `log_param`、数値は `log_metric`、成果物は `log_artifact`。実測値を `log_param` に入れない。
- 必ず `try/finally` で `mlflow.end_run()`。これを入れないと run が `FINISHED` にならない。

```bash
uv run --no-project --python 3.14 --with mlflow==3.16.0 --with boto3 python - <<'PY'
import os
import mlflow
from mlflow.tracking import MlflowClient

os.environ.setdefault("MLFLOW_S3_ENDPOINT_URL", "http://127.0.0.1:9000")
os.environ.setdefault("AWS_DEFAULT_REGION", "us-east-1")

client = MlflowClient()
exp = client.get_experiment_by_name("local")
exp_id = exp.experiment_id if exp else client.create_experiment("local", artifact_location="s3://mlflow-local/")

project = os.environ.get("SAI_PROJECT_ROOT", "").rstrip("/").split("/")[-1]
run_id = os.environ.get("SAI_PARENT_RUN_ID", "")

run = mlflow.start_run(experiment_id=exp_id, run_name="agent-check")
try:
    if project:
        mlflow.set_tag("sai.project", project)
    if run_id:
        mlflow.set_tag("sai.run_id", run_id)
    mlflow.log_param("lr", 0.001)
    mlflow.log_metric("loss", 0.42, step=0)
    with open("/tmp/agent-check.txt", "w") as f:
        f.write("hello\n")
    mlflow.log_artifact("/tmp/agent-check.txt")
    print(mlflow.active_run().info.run_id, dict(mlflow.active_run().data.tags))
finally:
    mlflow.end_run()
PY
```

- `sai.project` は `SAI_PROJECT_ROOT` の末尾のフォルダ名、`sai.run_id` は `SAI_PARENT_RUN_ID` で取る。**空ならそのタグは付けない**（agent の外で試しても壊れないように）。
- `set_terminated` を呼ぶ必要はない。`end_run()` が終了時に `FINISHED` を書く。

## 4. 前回の実験を読む

- この experiment の run を新しい順に取る。**「どれが一番よかったか」を自分で裁いて、次のパラメータを決める。**
- 指標ごとの最大・最小、絞り込み、タグごとのまとめは下の 1 つのコードで全部できる。

```bash
uv run --no-project --python 3.14 --with mlflow==3.16.0 python - <<'PY'
import mlflow
from mlflow.tracking import MlflowClient

client = MlflowClient()
exp = client.get_experiment_by_name("local")
# client.search_runs は Run オブジェクトのリストを返す。
# mlflow.search_runs（上位）は pandas の DataFrame を返すので、並べ替えたいときは注意。
runs = client.search_runs([exp.experiment_id], order_by=["attributes.start_time DESC"], max_results=20)
for r in runs:
    print(r.info.run_id[:8], r.data.tags.get("mlflow.runName", "-"),
          dict(r.data.params), dict(r.data.metrics),
          "tags:", {k: v for k, v in r.data.tags.items() if k.startswith("sai")})

# 指標ごとに最大・最小を求める
if runs:
    best = max(runs, key=lambda r: r.data.metrics.get("acc", 0.0))
    print("best acc:", best.data.metrics.get("acc"), best.info.run_id[:8])

# 絞り込み。値は必ず引用符で囲む。
# 使える比較子は = != LIKE ILIKE IS NULL IS NOT NULL だけ（> や < はサーバに拒否される）。
hits = client.search_runs([exp.experiment_id], filter_string="params.lr = '0.001' AND tags.`sai.run_id` = '999'")
print("hits:", [r.info.run_id[:8] for r in hits])

# 数値の大小で絞り込みたいときは、サーバに投げずに Python 側で行う
good = [r for r in client.search_runs([exp.experiment_id], max_results=1000)
        if r.data.params.get("lr") and float(r.data.params["lr"]) > 0.0005]
print("lr > 0.0005:", [r.info.run_id[:8] for r in good])

# タグの値ごとにまとめる（どの sai run がどの mlflow run を作ったか）
by_run = {}
for r in runs:
    by_run.setdefault(r.data.tags.get("sai.run_id", "-"), []).append(r.info.run_id[:8])
print("by sai.run_id:", by_run)
PY
```

## 5. やってはいけないこと

- `MLFLOW_TRACKING_URI` を `sqlite://` に変えない。`export` し直さない（渡された `env` が正）。
- `aws` / `boto3` で直接 MinIO へ成果物を書かない。成果物は `log_artifact` だけ。
- 実測値を `log_param` に入れない（パラメータと指標を混ぜない）。
- `mlflow.set_experiment()` で experiment を作らない（保存先が壊れる）。
- `end_run()` を省かない（run が `FINISHED` にならない）。
- パスワードやアクセスキーをスクリプト・テンプレ・報告書に書かない（`.env` / `env` から読むだけ）。

## 6. 動かないとき

- `curl -s "$MLFLOW_TRACKING_URI/version"` が 401 → Basic 認証が要る（LAN の PC だけ）。
- `Connection refused` → Mac の mlflow コンテナがStopped。`docker compose -f mlflow-server/docker-compose.yml ps`（**5000 番ではなく 5001 番**を使う。5000 は macOS の AirPlay）。
- `Experiment 'local' does not exist` → §2 のコードで作成する（`local` が既にあるなら増えない）。
- 成果物が読めない → MinIO のバケット名と `artifact_location` が食い違っていないか、`MLFLOW_S3_ENDPOINT_URL` が `http://127.0.0.1:9000` かを見る。
