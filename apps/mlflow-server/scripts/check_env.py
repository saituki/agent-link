""".env の必須変数が空のまま残っていないか調べる。docker compose run --rm checkenv を呼ぶ。"""

from __future__ import annotations

import os
import sys

REQUIRED = [
    "MLFLOW_VERSION",
    "POSTGRES_USER",
    "POSTGRES_PASSWORD",
    "POSTGRES_DB",
    "MINIO_ROOT_USER",
    "MINIO_ROOT_PASSWORD",
    "MLFLOW_BASIC_USER",
    "MLFLOW_BASIC_PASSWORD",
    "MLFLOW_BASIC_PASSWORD_HASH",
]


def main() -> int:
    empty = [k for k in REQUIRED if not os.environ.get(k, "").strip()]
    if empty:
        print(
            ".env の次の変数が空です: " + ", ".join(empty) + "\n（README §3 を埋めて値を入れること）",
            file=sys.stderr,
        )
        return 1
    print("OK: 必須変数がすべて埋まっています。")
    return 0


if __name__ == "__main__":
    sys.exit(main())
