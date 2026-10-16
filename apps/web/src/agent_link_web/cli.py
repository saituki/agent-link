"""外部 CLI を呼ぶ共通の受け口（06 §3.2・07 以降も使う）．

決め事（共有契約「Web が CLI を呼ぶ約束」）:
- シェルは使わない（args の配列で渡す。`shell=True` は使わない）。
- 成功は終了コード 0・`--json` なら標準出力に JSON 1 個。
- 失敗は標準出力に何も書かず，標準エラーの最後の空でない行に日本語で理由 1 行。
  終了コードは 2（入力の誤り）・3（見つからない）・4（状態の衝突）・1（その他）。

終了コードの HTTP への読み替え（この 1 か所に集める）:

| CLI の終了コード | HTTP の状態 | `detail` |
|---|---|---|
| 0 で標準出力が JSON | （成功） | — |
| 0 で標準出力が JSON でない | 502 | 「<app> の応答を読めませんでした」 |
| 2 | 400 | 標準エラーの最後の空でない行（`sai-progress` を除く） |
| 3 | 404 | 同上 |
| 4 | 409 | 同上 |
| 1 とそのほか | 502 | 同上（無ければ「<app> が失敗しました」） |
| 起動できない | 503 | 今のまま |
| 時間切れ | 504 | 今のまま |

"""

from __future__ import annotations

import json
import logging
import subprocess

from fastapi import HTTPException

logger = logging.getLogger("agent_link_web")

# 進捗の目印（この行は失敗の理由にしない・CLI 共通の約束 4）
_PROGRESS_PREFIX = "sai-progress"


def _log_stderr(app: str, proc: subprocess.CompletedProcess) -> None:
    """標準エラーの先頭 300 文字をログに出す（どの失敗でも呼ぶ）．"""
    err = (proc.stderr or "").strip()
    if err:
        logger.warning(f"{app} の標準エラー: {err[:300]}")


def last_reason(proc: subprocess.CompletedProcess) -> str:
    """標準エラーの最後の空でない行（`sai-progress` で始まる行を除く）。

    失敗の理由として使う（無ければ空文字）。
    """
    for line in reversed((proc.stderr or "").splitlines()):
        text = line.strip()
        if text and not text.startswith(_PROGRESS_PREFIX):
            return text
    return ""


def raise_for_failure(app: str, proc: subprocess.CompletedProcess):
    """終了コード 0 以外を，共有契約の HTTP の状態に読み替えて `HTTPException` を出す。

    `run_json`（JSON を返す呼び出し）と `result_json`，文字出力を扱う `daemon` が
    同じ読み替えを使うための入口。理由は終了コードと標準エラーだけで決める。
    """
    _log_stderr(app, proc)
    detail = last_reason(proc) or f"{app} failed."
    status = {2: 400, 3: 404, 4: 409}.get(proc.returncode, 502)
    raise HTTPException(status_code=status, detail=detail)


def result_json(proc: subprocess.CompletedProcess, *, app: str):
    """終了コードを見て，成功なら標準出力の JSON を返し，失敗は読み替えて失敗させる。

    `run_json` と同じ読み替えを，呼び出し側が `run_cli` を直接呼んだ場合にも使えるようにする。
    """
    if proc.returncode != 0:
        raise_for_failure(app, proc)
    try:
        return json.loads(proc.stdout or "")
    except (json.JSONDecodeError, TypeError):
        _log_stderr(app, proc)
        raise HTTPException(status_code=502, detail=f"Could not read the response from {app}.")


def run_cli(
    args: list[str],
    *,
    app: str,
    cwd: str | None = None,
    env: dict | None = None,
    timeout: float = 20.0,
    input_text: str | None = None,
) -> subprocess.CompletedProcess:
    """外部 CLI を呼ぶ（シェルは使わない）．起動できない・時間切れは HTTPException．

    終了コードは見ない（呼び出し側が必要なら自分で確かめる）。
    JSON でない CLI（文字出力の ones）もこれで呼ぶ。
    `input_text` は標準入力へ渡す文字列（`agent-link card create` の
    stdin JSON・計画 15 §3.3）。無ければ標準入力は閉じたまま。
    """
    try:
        return subprocess.run(
            args,
            capture_output=True,
            text=True,
            cwd=cwd,
            env=env,
            timeout=timeout,
            input=input_text,
        )
    except FileNotFoundError:
        # PATH に無い・設定の場所が間違っている
        raise HTTPException(
            status_code=503,
            detail=f"{app} not found. Check the configured path.",
        )
    except subprocess.TimeoutExpired:
        raise HTTPException(
            status_code=504,
            detail=f"{app} did not respond in time.",
        )


def run_json(
    args: list[str],
    *,
    app: str,
    cwd: str | None = None,
    env: dict | None = None,
    timeout: float = 20.0,
    input_text: str | None = None,
    runner=None,
):
    """共有契約の CLI を呼び，JSON を返す（成功・失敗の読み替えは `result_json`）．

    - 終了コード 0: 標準出力の JSON をそのまま返す。
    - それ以外: `result_json` の読み替え（http の状態 + 標準エラーの最後の行）。
    - JSON として読めない（終了コード 0 のとき）: 502。
    - `input_text` は標準入力へ渡す（標準入力に渡す本文）。
    - `runner` を渡すと，その関数で CLI を呼ぶ（テストが `control.run_cli` を
      差し替えても効くようにするため。既定はこのモジュールの `run_cli`）。
    """
    call = runner or run_cli
    proc = call(
        args, app=app, cwd=cwd, env=env, timeout=timeout, input_text=input_text
    )
    return result_json(proc, app=app)
