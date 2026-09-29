import os
import tempfile
import mlflow
from mlflow.tracking import MlflowClient

from dotenv import load_dotenv
load_dotenv()
# 環境変数の確認
print(os.getenv("MLFLOW_TRACKING_USERNAME"))
print(os.getenv("MLFLOW_TRACKING_PASSWORD"))

# mlflow server（Docker 内）
mlflow.set_tracking_uri(os.getenv("MLFLOW_TRACKING_URI", "http://192.168.100.12:8443"))
client = MlflowClient()
# Experiment 作成 or 取得
experiment_name = "Debug5"
experiment = client.get_experiment_by_name(experiment_name)
if experiment is None:
    experiment_id = client.create_experiment(experiment_name)
    print(f"Created experiment: {experiment_id}")
else:
    experiment_id = experiment.experiment_id
    print(f"Using existing experiment: {experiment_id}")
# Run 作成・記録
with mlflow.start_run(experiment_id=experiment_id):
    mlflow.log_param("lr", 0.01)
    mlflow.log_metric("loss", 0.123)
    # 一時ファイル作成
    with tempfile.TemporaryDirectory() as tmpdir:
        artifact_path = os.path.join(tmpdir, "test_artifact.txt")
        with open(artifact_path, "w") as f:
            f.write("This is a test artifact for MinIO storage.\n")
        # アーチファクト保存
        mlflow.log_artifact(artifact_path)
    run_id = mlflow.active_run().info.run_id
    print(f"Run created: {run_id}")

# # Run を backend store から取得できるか確認
# run = client.get_run(run_id)
# print("Run fetched from backend store")
# print("params:", run.data.params)
# print("metrics:", run.data.metrics)


# アーチファクトファイルがどこにあるかを確認 
# -> mlflow-atrifacts:/1 ：mlflowのアーチファクトにある，
# （minioからは見れない．コンテナが消えたら消える）
# from mlflow.tracking import MlflowClient
# import mlflow

# mlflow.set_tracking_uri(os.getenv("MLFLOW_TRACKING_URI", "http://mlflow:5000"))
# client = MlflowClient()

# exp = client.get_experiment_by_name("Debug3")
# print(exp.artifact_location)