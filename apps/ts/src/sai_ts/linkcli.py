"""sai-link を呼ぶ共通の関数（C11）。シェルは使わず、引数の列で呼ぶ。"""

from __future__ import annotations

import json
import os
import subprocess

from . import paths

# sai-link の終了コード → HTTP の状態（2 入力の誤り・3 見つからない・4 状態の衝突）
_STATUS = {2: 400, 3: 404, 4: 409}


class LinkError(Exception):
    def __init__(self, status: int, detail: str):
        super().__init__(detail)
        self.status = status
        self.detail = detail


def run(args: list[str], stdin: str | None = None, timeout: float = 20) -> dict:
    root = paths.project_root()
    cmd = [os.environ.get("SAI_LINK_BIN", "sai-link"), *args]
    env = {**os.environ, "SAI_PROJECT_ROOT": str(root)}
    try:
        # stdin を必ず渡す（None だと親の標準入力を読んで止まることがあるため空にする）
        p = subprocess.run(cmd, input=stdin if stdin is not None else "", capture_output=True,
                           text=True, timeout=timeout, env=env, cwd=root)
    except FileNotFoundError:
        raise LinkError(503, "sai-link が見つかりません")
    except subprocess.TimeoutExpired:
        raise LinkError(504, "sai-link の時間切れです")
    if p.returncode != 0:
        lines = [ln for ln in p.stderr.splitlines() if ln.strip()]
        detail = lines[-1] if lines else f"sai-link が終了コード {p.returncode} で失敗しました"
        raise LinkError(_STATUS.get(p.returncode, 502), detail)
    try:
        return json.loads(p.stdout)
    except ValueError:
        raise LinkError(502, "sai-link の出力が JSON ではありません")
