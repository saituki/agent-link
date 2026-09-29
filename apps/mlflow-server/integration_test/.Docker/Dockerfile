FROM python:3.12-slim

ARG MLFLOW_VERSION=3.16.0

# 作業ディレクトリ
WORKDIR /workspace

# 基本ツール（最低限）
RUN apt-get update && \
    apt-get install -y --no-install-recommends \
        curl \
        git \
    && rm -rf /var/lib/apt/lists/*

# Python ライブラリ
RUN pip install --no-cache-dir \
    "mlflow==${MLFLOW_VERSION}" \
    numpy \
    pandas \
    scikit-learn \
    python-dotenv

# 対話実行前提
CMD ["bash"]
