"""解析用のデータの受け口（U01・共有契約）。

sai-prep が個人のトークンで送るデータファイル（種類は問わない）を、
プロジェクトの `<root>/data/raw/<filename>` に置く。置くだけで、形式の変換はしない
（コンテナの中で agent かスクリプトが行う）。

なぜ束の受け口（bundles）を使わないか: 束は本文を全部メモリに読む（上限 300MB）。
データは GB 級になりうるので、ここではディスクへ流し書きする（上限 5GB）。
同じ名前があれば上書きせず 409（解析のもとデータを黙って失わないため）。
"""

from __future__ import annotations

import os
import re
from datetime import datetime, timezone
from pathlib import Path

from fastapi import HTTPException, Request
from starlette.concurrency import run_in_threadpool

from sai_web import control

# 1 件の大きさの上限（5GB）。テストでは差し替える。
DATA_MAX_BYTES = 5 * 1024**3
# 置き場（root からの相対。ホストとコンテナで同じパスに見える・C4）
RAW_REL = Path("data") / "raw"
# ファイル名の規則（パスの区切り・`..`・隠しファイルを通さない）。sai-prep と同じ式。
_NAME_RE = re.compile(r"[A-Za-z0-9][A-Za-z0-9_.-]{0,127}")


def _check_filename(filename: str) -> None:
    """名前の規則に合わなければ 400。`fullmatch` で改行付きの名前も通さない。"""
    if not _NAME_RE.fullmatch(filename):
        raise HTTPException(
            status_code=400,
            detail="ファイル名は英数字と . _ - だけで、先頭は英数字にしてください（128 字まで）",
        )


def _declared_length(request: Request) -> int:
    """Content-Length を読む。無い・数でない・上限超えは 413。"""
    raw = (request.headers.get("content-length") or "").strip()
    if not raw.isdigit() or int(raw) > DATA_MAX_BYTES:
        raise HTTPException(
            status_code=413, detail="データは 1 件 5GB までです（Content-Length が必要です）"
        )
    return int(raw)


def _project_root(name: str) -> Path:
    """プロジェクトの root を引く。無い・root が無ければ 404（同期の処理なので別スレッドで）。"""
    project = control.find_project(name)
    if not project.get("root_exists"):
        raise HTTPException(status_code=404, detail="プロジェクトの root が見つかりません")
    return Path(project["root"])


async def store_raw(name: str, filename: str, request: Request) -> dict:
    """本文（ファイルそのもの）を `<root>/data/raw/<filename>` に置く（201・U01）。

    - 名前が規則に合わなければ 400、大きさ（Content-Length）が無い・上限超えは 413
    - 無いプロジェクトは 404、同じ名前があれば 409（上書きしない）
    - 本文は `.<filename>.part` へ流し書きし、全部届いたら `os.replace` で名前を付ける。
      途中で切れたり上限を超えたりしたら `.part` を消す（残さない）。
    """
    _check_filename(filename)
    length = _declared_length(request)
    root = await run_in_threadpool(_project_root, name)
    folder = root / RAW_REL
    target = folder / filename
    if target.exists():
        raise HTTPException(status_code=409, detail="同じ名前のファイルがあります（上書きしません）")
    part = folder / f".{filename}.part"
    total = 0
    try:
        await run_in_threadpool(folder.mkdir, parents=True, exist_ok=True)
        with part.open("wb") as handle:
            async for chunk in request.stream():
                total += len(chunk)
                if total > DATA_MAX_BYTES:
                    raise HTTPException(status_code=413, detail="データは 1 件 5GB までです")
                handle.write(chunk)
        if total != length:
            raise HTTPException(status_code=400, detail="本文が途中で切れました（大きさが合いません）")
        if target.exists():
            raise HTTPException(status_code=409, detail="同じ名前のファイルがあります（上書きしません）")
        os.replace(part, target)
    finally:
        # 成功なら part は無い。失敗（途中で切れた・上限・409）なら残さない。
        part.unlink(missing_ok=True)
    return {"name": filename, "size": total, "path": f"data/raw/{filename}"}


def list_raw(name: str) -> dict:
    """`<root>/data/raw/` の一覧（名前の昇順）。`.part` と隠しファイルは出さない。

    フォルダが無ければ `{"files": []}`。無いプロジェクトは 404。
    """
    folder = _project_root(name) / RAW_REL
    files = []
    if folder.is_dir():
        for path in sorted(folder.iterdir(), key=lambda p: p.name):
            if path.name.startswith(".") or not path.is_file():
                continue
            stat = path.stat()
            files.append(
                {
                    "name": path.name,
                    "size": stat.st_size,
                    "mtime": datetime.fromtimestamp(stat.st_mtime, timezone.utc).isoformat(timespec="seconds"),
                }
            )
    return {"files": files}
