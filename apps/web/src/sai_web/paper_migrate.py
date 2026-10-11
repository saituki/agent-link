"""論文の移植の受け口（実装計画 M02・共有契約 C-M1・C-M2・C-M3）。

2 つの入口:
- `POST …/papers/migrate`: sai-prep が Mac で作った「移植の束」（zip）を受け、
  `<root>/.sai/paper/inbox/<uuid>/` へ展開して
  `sai-paper library import-migration --dir <フォルダ> --json` で取り込む。
- `POST …/papers/migrate-from`: 同じ利用者の別の sai-tree プロジェクトから、サーバの中で移す。
  移植元のコンテナで `sai-paper library export --ids … --out <outbox>` → Web（ホスト）が
  移植先の `inbox/<uuid>` へ移す → 移植先のコンテナで `import-migration`。
  移植先のコンテナからは移植元のファイルが見えないので、書き出しと移動は Web が行う。

zip の読み取り・展開の部品（`bundles.py`）は前処理の束のものをそのまま使う。
仕事（job）は `papers._download_jobs` に `kind="migrate"` で入れ、進み具合の帯に出す。
"""

from __future__ import annotations

import asyncio
import json
import logging
import shutil
import uuid
import zipfile
from pathlib import Path

from fastapi import HTTPException
from starlette.concurrency import run_in_threadpool

from sai_web import bundles, notify, papers

logger = logging.getLogger("sai_web")

# 移植の束の形（C-M1）
MIGRATE_FORMAT = "sai-paper-migration"
MIGRATE_VERSION = 1
MIGRATE_MAX_IDS = 1000
# 段（0 から数える。_import_job・_from_job の step と合わせる）
MIGRATE_STEPS_UPLOAD = ["展開", "取り込み"]
MIGRATE_STEPS_FROM = ["書き出し", "移す", "取り込み"]
# 書き出しと取り込みの待ち時間（秒）
EXPORT_TIMEOUT = 600.0
IMPORT_TIMEOUT = 1800.0
# 書き出し先（root からの相対。移植元のコンテナと同じパスに見える）
OUTBOX_REL = Path(".sai") / "paper" / "outbox"
# SQLite ファイルの先頭の記号（papr.db の検査）
SQLITE_MAGIC = b"SQLite format 3\x00"
# 通知を押したときに開く先
LIBRARY_URL = "/#/papers/library"


# --- zip の検査（CLI を呼ぶ前に必ず通す） ---


def _valid_ids(ids) -> bool:
    """papers の id の一覧か（1〜1000 個の正の整数・bool は除く）。"""
    return (
        isinstance(ids, list)
        and 1 <= len(ids) <= MIGRATE_MAX_IDS
        and all(not isinstance(i, bool) and isinstance(i, int) and i > 0 for i in ids)
    )


def check_migration_zip(zf: zipfile.ZipFile) -> dict:
    """移植の束の中身を検査し、manifest を返す（違反は 400・CLI は呼ばない）。

    項目の数・展開後の合計・名前・シンボリックリンクは前処理の束と同じ。
    manifest の形（format・version・papers）と `papr.db` の先頭を見る。
    """
    infos = zf.infolist()
    if len(infos) > bundles.BUNDLE_MAX_ENTRIES:
        raise HTTPException(
            status_code=400,
            detail=f"前処理の束の項目が多すぎます（{bundles.BUNDLE_MAX_ENTRIES} まで）",
        )
    total = 0
    for info in infos:
        if bundles._bad_zip_name(info.filename):
            raise HTTPException(
                status_code=400, detail=f"前処理の束に危ない名前があります: {info.filename}"
            )
        if bundles._is_symlink(info):
            raise HTTPException(
                status_code=400, detail=f"前処理の束にシンボリックリンクがあります: {info.filename}"
            )
        if not info.is_dir():
            total += int(info.file_size)
            if total > bundles.BUNDLE_MAX_UNCOMPRESSED:
                raise HTTPException(
                    status_code=400, detail="前処理の束を展開すると大きすぎます（1GB まで）"
                )
    names = {info.filename for info in infos}
    if "manifest.json" not in names:
        raise HTTPException(status_code=400, detail="前処理の束に manifest.json がありません")
    try:
        with zf.open("manifest.json") as handle:
            raw = handle.read(bundles.MANIFEST_MAX_BYTES + 1)
    except (KeyError, zipfile.BadZipFile, OSError, RuntimeError, ValueError):
        raise HTTPException(status_code=400, detail="manifest.json を読めませんでした")
    if len(raw) > bundles.MANIFEST_MAX_BYTES:
        raise HTTPException(status_code=400, detail="manifest.json が大きすぎます")
    try:
        manifest = json.loads(raw.decode("utf-8"))
    except (ValueError, UnicodeDecodeError):
        raise HTTPException(status_code=400, detail="manifest.json の JSON を読めませんでした")
    if not isinstance(manifest, dict):
        raise HTTPException(status_code=400, detail="manifest.json の形が違います")
    if manifest.get("format") != MIGRATE_FORMAT or manifest.get("version") != MIGRATE_VERSION:
        raise HTTPException(
            status_code=400, detail="移植の束の形が違います（format / version）"
        )
    if not _valid_ids(manifest.get("papers")):
        raise HTTPException(
            status_code=400,
            detail="manifest.json の papers は 1〜1000 個の正の整数の配列にしてください",
        )
    if "papr.db" not in names:
        raise HTTPException(status_code=400, detail="移植の束に papr.db がありません")
    try:
        with zf.open("papr.db") as handle:
            head = handle.read(len(SQLITE_MAGIC))
    except (KeyError, zipfile.BadZipFile, OSError, RuntimeError, ValueError):
        raise HTTPException(status_code=400, detail="papr.db を読めませんでした")
    if head != SQLITE_MAGIC:
        raise HTTPException(status_code=400, detail="papr.db の先頭が SQLite ではありません")
    return manifest


def _extract(zf: zipfile.ZipFile, folder: Path) -> None:
    """検査済みの zip を `folder` へ展開する（失敗したら `folder` を消して 400）。"""
    try:
        folder.mkdir(parents=True, exist_ok=True)
        bundles._safe_extract(zf, folder)
    except HTTPException:
        shutil.rmtree(folder, ignore_errors=True)
        raise
    except OSError as e:
        shutil.rmtree(folder, ignore_errors=True)
        logger.warning(f"移植の束を展開できません: {e}")
        raise HTTPException(status_code=400, detail="移植の束を展開できませんでした")


# --- 依頼と背景の実行 ---


def _fail(job: dict, title: str, error: str) -> None:
    """仕事を失敗で終え、通知を出す。"""
    papers._finish_download(job, "failed", error=error)
    notify.broadcast("論文を移せませんでした", f"{title}: {error}", LIBRARY_URL)


async def request_upload(project: dict, body: bytes) -> dict:
    """移植の束の依頼（`POST …/papers/migrate`・C-M3）。

    検査して展開し（違えば 400 / 413 / 415）、仕事を作って 202 ですぐ返す。
    取り込みは `_import_job` を背景で走らせる。
    """
    zf = bundles._open_zip(body)
    with zf:
        manifest = check_migration_zip(zf)
        folder = Path(project["root"]) / bundles.INBOX_REL / uuid.uuid4().hex
        _extract(zf, folder)
    title = f"論文の移植（{len(manifest['papers'])} 件）"
    key = f"migrate:{uuid.uuid4().hex}"
    job = papers._new_job("migrate", key, title, "processing", MIGRATE_STEPS_UPLOAD, 0)
    papers._download_jobs[(project["name"], key)] = job
    job["_task"] = asyncio.create_task(_import_job(project, folder, job, title))
    return {"key": key, "status": "processing"}


async def request_from(project: dict, payload) -> dict:
    """同じ利用者の別プロジェクトからの移植の依頼（`POST …/papers/migrate-from`・C-M3）。

    `source` は `papers.resolve_project` で引ける物だけ（無ければ 404）。
    重複する id は 1 つにまとめる。仕事を作って 202 ですぐ返す。
    """
    if not isinstance(payload, dict):
        raise HTTPException(status_code=400, detail="本文は {\"source\": …, \"ids\": […]} の形にしてください")
    source = payload.get("source")
    if not isinstance(source, str) or not source.strip():
        raise HTTPException(status_code=400, detail="移植元（source）を入れてください")
    if source == project["name"]:
        raise HTTPException(status_code=400, detail="移植元と移植先が同じです")
    ids = payload.get("ids")
    if not _valid_ids(ids):
        raise HTTPException(
            status_code=400, detail="ids は 1〜1000 個の正の整数にしてください"
        )
    source_project = papers.resolve_project(source)
    ids = list(dict.fromkeys(ids))
    title = f"論文の移植（{len(ids)} 件）"
    key = f"migrate:{uuid.uuid4().hex}"
    job = papers._new_job("migrate", key, title, "processing", MIGRATE_STEPS_FROM, 0)
    papers._download_jobs[(project["name"], key)] = job
    job["_task"] = asyncio.create_task(
        _from_job(project, source_project, ids, job, title)
    )
    return {"key": key, "status": "processing"}


async def _from_job(
    project: dict, source_project: dict, ids: list[int], job: dict, title: str
) -> None:
    """移植元で書き出し → Web が移す → 移植先で取り込む（背景・C-M2）。

    - 終わったら（成否どちらでも）書き出し先と移した先の両方を消す。
    - 例外は全部つかまえてログに出す（サーバーは止めない）。
    """
    uid = uuid.uuid4().hex
    out = Path(source_project["root"]) / OUTBOX_REL / uid
    dest = Path(project["root"]) / bundles.INBOX_REL / uid
    try:
        out.parent.mkdir(parents=True, exist_ok=True)
        code, stdout, reason = await papers._run_paper_command(
            str(source_project["root"]),
            ["library", "export", "--ids", ",".join(str(i) for i in ids), "--out", str(out), "--json"],
            EXPORT_TIMEOUT,
        )
        if reason:
            _fail(job, title, reason)
            return
        if code != 0:
            _fail(job, title, papers._radio_error(stdout))
            return
        job["step"] = 1
        dest.parent.mkdir(parents=True, exist_ok=True)
        # 別のファイルシステムでも動くように、写してから消す（shutil.move の作り）
        await run_in_threadpool(shutil.move, str(out), str(dest))
        job["step"] = 2
        await _import(project, dest, job, title)
    except Exception as e:  # サーバーは止めない（前処理の束と同じ扱い）
        logger.exception(f"論文の移植で失敗: {e}")
        if job.get("status") not in ("done", "failed"):
            _fail(job, title, "原因不明のエラー")
    finally:
        shutil.rmtree(out, ignore_errors=True)
        shutil.rmtree(dest, ignore_errors=True)


async def _import_job(project: dict, folder: Path, job: dict, title: str) -> None:
    """展開済みの移植の束を `_import` で取り込む（背景）。終わったらフォルダを消す。"""
    try:
        job["step"] = 1
        await _import(project, folder, job, title)
    except Exception as e:  # サーバーは止めない
        logger.exception(f"論文の移植で失敗: {e}")
        if job.get("status") not in ("done", "failed"):
            _fail(job, title, "原因不明のエラー")
    finally:
        shutil.rmtree(folder, ignore_errors=True)


async def _import(project: dict, folder: Path, job: dict, title: str) -> None:
    """`sai-paper library import-migration --dir <フォルダ> --json` を走らせて仕事を終える。"""
    code, stdout, reason = await papers._run_paper_command(
        str(project["root"]),
        ["library", "import-migration", "--dir", str(folder), "--json"],
        IMPORT_TIMEOUT,
    )
    if reason:
        _fail(job, title, reason)
        return
    if code != 0:
        _fail(job, title, papers._radio_error(stdout))
        return
    data = papers._download_json(stdout)
    imported = data.get("imported")
    skipped = data.get("skipped")
    failed = data.get("failed")
    if not all(isinstance(part, list) for part in (imported, skipped, failed)):
        _fail(job, title, "sai-paper の応答を読めませんでした")
        return
    job["result"] = data
    papers._finish_download(job, "done")
    notify.broadcast(
        "📚 論文を移しました",
        f"{title}: 移した {len(imported)} 件・飛ばした {len(skipped)} 件・失敗 {len(failed)} 件",
        LIBRARY_URL,
    )
