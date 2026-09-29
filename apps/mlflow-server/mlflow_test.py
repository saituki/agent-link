import mlflow
import time
import tempfile
from pathlib import Path

# ===== 接続設定 =====
# Caddy 経由（Basic 認証つき）。名前解決は要らない。
# MAC の IP は `ipconfig getifaddr en0` で確認して書き換える。
mlflow.set_tracking_uri("http://192.168.100.12:8443")

# 直指定（切り分け用。同一 docker network 内の場合）
# mlflow.set_tracking_uri("http://mlflow:5000")

print("Tracking URI:", mlflow.get_tracking_uri())

# ===== Experiment を指定（なければ作成される）=====
experiment_name = "connection_test_experiment"
mlflow.set_experiment(experiment_name)

# ===== Run 開始 =====
with mlflow.start_run(run_name="connection_test_run"):

    # ---- パラメータ ----
    mlflow.log_param("model", "dummy_model")
    mlflow.log_param("seed", 42)

    # ---- メトリクス ----
    mlflow.log_metric("accuracy", 0.85)
    mlflow.log_metric("loss", 0.35)

    # ---- 時系列メトリクス（よくある）----
    for step in range(3):
        mlflow.log_metric("loss_step", 0.5 / (step + 1), step=step)
        time.sleep(0.1)

    # ---- アーティファクト ----
    with tempfile.TemporaryDirectory() as tmpdir:
        p = Path(tmpdir) / "test.txt"
        p.write_text("MLflow connection test OK\n")
        mlflow.log_artifact(str(p))

    print("MLflow logging test completed.")

print("Done.")
