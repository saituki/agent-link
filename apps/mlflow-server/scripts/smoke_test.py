"""experiment `local` に 1 run 作って param / metric / artifact を記録し、読み返す。

compose の smoke サービスから呼ぶ。`docker compose run --rm smoke`
標準出力は結果の JSON 1 行だけ。
"""

from __future__ import annotations

import json
import os
import sys
import tempfile
from pathlib import Path

# mlflow の案内メッセージを無効にする（import 前に設定する）。
os.environ.setdefault("MLFLOW_DISABLE_AGENT_HINT", "1")
# MlflowClient.log_artifact / set_terminated が標準出力に URL を出すのを止める。
# これがないと JSON の前に 2 行混ざる。
os.environ.setdefault("MLFLOW_SUPPRESS_PRINTING_URL_TO_STDOUT", "true")

from mlflow.tracking import MlflowClient  # noqa: E402

EXPERIMENT_NAME = "local"
ARTIFACT_NAME = "hello.txt"
ARTIFACT_BODY = "hello mlflow\n"  # 11 バイト


def main() -> int:
    try:
        client = MlflowClient(tracking_uri=os.environ["MLFLOW_TRACKING_URI"])

        exp = client.get_experiment_by_name(EXPERIMENT_NAME)
        if exp is None:
            print(
                f"experiment `{EXPERIMENT_NAME}` がありません。"
                "先に `docker compose run --rm init` を実行してください。",
                file=sys.stderr,
            )
            return 1

        run = client.create_run(
            exp.experiment_id,
            run_name="smoke",
            # 共有契約 C2: 特別扱いする run には sai.kind だけを付ける。
            tags={"sai.kind": "smoke"},
        )
        run_id = run.info.run_id

        client.log_param(run_id, "lr", 0.01)
        client.log_metric(run_id, "loss", 0.5, 0, 1)

        with tempfile.TemporaryDirectory() as tmpdir:
            path = Path(tmpdir) / ARTIFACT_NAME
            path.write_text(ARTIFACT_BODY)
            client.log_artifact(run_id, str(path))

        fetched = client.get_run(run_id)
        artifacts = client.list_artifacts(run_id)
        names = [a.path for a in artifacts]

        client.set_terminated(run_id, "FINISHED")

        if fetched.data.params.get("lr") != "0.01":
            raise AssertionError(f"param が読み戻せません: {fetched.data.params}")
        if fetched.data.metrics.get("loss") != 0.5:
            raise AssertionError(f"metric が読み戻せません: {fetched.data.metrics}")
        if ARTIFACT_NAME not in names:
            raise AssertionError(f"成果物が読み戻せません: {names}")

        print(
            json.dumps(
                {
                    "run_id": run_id,
                    "artifact": ARTIFACT_NAME,
                    "param": "lr=0.01",
                    "metric": "loss=0.5",
                },
                ensure_ascii=False,
            )
        )
        return 0
    except Exception as exc:  # noqa: BLE001 - 1 行にして出すため
        print(f"smoke テストに失敗しました: {exc}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
