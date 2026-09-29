FROM python:3.12-slim

# MLflow のバージョン。compose の build.args から渡す（.env の MLFLOW_VERSION）。
ARG MLFLOW_VERSION=3.16.0

RUN pip install --no-cache-dir \
    "mlflow==${MLFLOW_VERSION}" \
    # backend store は PostgreSQL
    psycopg2-binary \
    # artifact store は S3（boto3 だけで足りる。s3fs は要らない）
    boto3

EXPOSE 5000

# 直接方式（--no-serve-artifacts）。
# 成果物の保存先は experiment ごとの artifact_location で決まるので、
# ここでは固定しない。接続者全員が自分の S3 endpoint を持つ。
CMD ["mlflow", "server", \
     "--host", "0.0.0.0", \
     "--port", "5000", \
     "--no-serve-artifacts", \
     "--allowed-hosts", "*", \
     "--cors-allowed-origins", "*"]
