"""experiment を作る（2 回走らせても増えない）。compose の init サービスから呼ぶ。

標準出力には表だけを出す。エラーは標準出力に出さず、標準エラーに 1 行だけ出す。
"""

from __future__ import annotations

import os
import sys

# mlflow の案内メッセージを無効にする（import 前に設定する）。
os.environ.setdefault("MLFLOW_DISABLE_AGENT_HINT", "1")

import boto3  # noqa: E402
from botocore.exceptions import ClientError  # noqa: E402
from mlflow.tracking import MlflowClient  # noqa: E402

# `local` は MinIO 側、`dok` はさくら オブジェクトストレージ側。
# `dok` のバケット（hps-data-2026）は既にあるので ensure_bucket() は触らない。
# experiment を作るのは必ず create_experiment(name, artifact_location=...) で行う。
EXPERIMENTS = [
    ("local", "s3://mlflow-local/"),
    ("dok", "s3://hps-data-2026/saito260927/mlflow/"),
]

# MLFLOW 側の S3 設定と同じ MinIO のバケット。
BUCKET_NAME = "mlflow-local"


def ensure_bucket() -> None:
    """MinIO にバケットを作る。既にあれば何もしない。"""
    s3 = boto3.client(
        "s3",
        endpoint_url=os.environ["MLFLOW_S3_ENDPOINT_URL"],
        aws_access_key_id=os.environ["AWS_ACCESS_KEY_ID"],
        aws_secret_access_key=os.environ["AWS_SECRET_ACCESS_KEY"],
        region_name=os.environ.get("AWS_DEFAULT_REGION", "us-east-1"),
    )
    try:
        s3.create_bucket(Bucket=BUCKET_NAME)
    except ClientError as exc:
        code = str(exc.response.get("Error", {}).get("Code", ""))
        if code not in ("BucketAlreadyExists", "BucketAlreadyOwnedByYou"):
            raise


def main() -> int:
    try:
        ensure_bucket()
        client = MlflowClient(tracking_uri=os.environ["MLFLOW_TRACKING_URI"])
        for name, location in EXPERIMENTS:
            exp = client.get_experiment_by_name(name)
            if exp is None:
                # create_experiment は experiment_id（str）を返すので、作り直して読み直す。
                client.create_experiment(name, artifact_location=location)
                exp = client.get_experiment_by_name(name)
            if exp is None:
                raise RuntimeError(f"experiment {name} を作成できませんでした")
            print(f"{exp.name}\t{exp.artifact_location}")
        return 0
    except Exception as exc:  # noqa: BLE001 - 1 行にして出すため
        print(f"experiment の作成に失敗しました: {exc}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
