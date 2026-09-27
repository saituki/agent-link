"""外部 CLI を呼ぶ共通の受け口（06 §3.2・07 以降も使う）．

決め事（00 §7）:
- シェルは使わない（args の配列で渡す。`shell=True` は使わない）。
- 成功は終了コード 0・標準出力に JSON 1 個。
- 失敗は終了コード 1・標準出力に `{"error": "<日本語>"}`。

失敗は fastapi の HTTPException に変換して画面に渡す:
- 起動できない           → 503（設定の場所を確認してもらう文言）
- 時間切れ               → 504
- CLI の失敗（error JSON）→ 400（その文言をそのまま出す）
- 応答を読めない          → 502
どの失敗も標準エラーの先頭 300 文字をログに出す（原因の追跡のため）。
"""

from __future__ import annotations

import json
import logging
import subprocess

from fastapi import HTTPException

logger = logging.getLogger("sai_phone")


def _log_stderr(app: str, proc: subprocess.CompletedProcess) -> None:
    """標準エラーの先頭 300 文字をログに出す（どの失敗でも呼ぶ）．"""
    err = (proc.stderr or "").strip()
    if err:
        logger.warning(f"{app} の標準エラー: {err[:300]}")


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
    `input_text` は標準入力へ渡す文字列（`sai-link phone card create` の
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
            detail=f"{app} が見つかりません（設定の場所を確認してください）",
        )
    except subprocess.TimeoutExpired:
        raise HTTPException(
            status_code=504,
            detail=f"{app} が時間内に応答しませんでした",
        )


def run_json(
    args: list[str],
    *,
    app: str,
    cwd: str | None = None,
    env: dict | None = None,
    timeout: float = 20.0,
    input_text: str | None = None,
):
    """00 §7 の約束の CLI を呼び、JSON を返す．

    - 終了コード 0: 標準出力の JSON をそのまま返す。
    - 終了コード 0 以外: 標準出力の `{"error": …}` を 400 で画面に出す。
    - JSON として読めない: 502。
    - `input_text` は標準入力へ渡す（sai-paper `phone note` の本文）。
    """
    proc = run_cli(args, app=app, cwd=cwd, env=env, timeout=timeout, input_text=input_text)
    try:
        data = json.loads(proc.stdout or "")
    except (json.JSONDecodeError, TypeError):
        # 失敗の形（error JSON）でも読めない場合もここ（00 §7）
        _log_stderr(app, proc)
        raise HTTPException(status_code=502, detail=f"{app} の応答を読めませんでした")
    if proc.returncode == 0:
        return data
    _log_stderr(app, proc)
    # 失敗は {"error": "<日本語のメッセージ>"}（00 §7 共通）
    if isinstance(data, dict) and data.get("error"):
        raise HTTPException(status_code=400, detail=str(data["error"]))
    raise HTTPException(status_code=400, detail=f"{app} が失敗しました")
