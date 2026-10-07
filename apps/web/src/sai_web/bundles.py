"""前処理の束（zip）の受け口（実装計画 U13 §1・共有契約 C12・C13）。

sai-prep が個人のトークンで送る「前処理の束」（zip）を受け、**Web が**検査して
`<プロジェクトの root>/.sai/paper/inbox/<uuid>/` へ展開し、プロジェクトのコンテナの
中の `sai-paper paper import-bundle --dir <フォルダ> --json` で取り込む。

なぜ Web が zip を扱うか: Rust に zip の依存が無い（依存を増やさない）。外から来る値は
CLI を呼ぶ前に Web が検査する（AGENTS.md）。展開先はホストとコンテナで同じパスに見え、
コンテナの中の CLI がそのまま読める（C4）。

仕事（job）は `papers._download_jobs` に `kind="bundle"` で入れ、既存の
`GET /api/projects/{name}/papers/downloads` に出す（画面の進み具合の帯がそのまま使える）。
"""

from __future__ import annotations

import asyncio
import io
import json
import logging
import re
import shutil
import stat
import uuid
import zipfile
from pathlib import Path

from fastapi import HTTPException
from starlette.concurrency import run_in_threadpool

from sai_web import notify, papers

logger = logging.getLogger("sai_web")

# 本文の種類（違えば 415）と大きさの上限（300MB・C14）
BUNDLE_CONTENT_TYPE = "application/zip"
BUNDLE_MAX_BYTES = 300 * 1024 * 1024
# zip の検査の上限（項目数・展開後の合計・manifest.json の大きさ）
BUNDLE_MAX_ENTRIES = 5000
BUNDLE_MAX_UNCOMPRESSED = 1024 * 1024 * 1024
MANIFEST_MAX_BYTES = 1024 * 1024
# manifest.json の target の bibtex の上限（64KB・C12）
BIBTEX_MAX_BYTES = 64 * 1024
# 取り込みの待ち時間（docling はもう走らないが、大きな PDF の書き込みを見込む・U13 §1）
BUNDLE_TIMEOUT = 600.0
# manifest.json の形（C12）
BUNDLE_FORMAT = "sai-prep-bundle"
BUNDLE_VERSION = 1
# PDF の先頭の記号
PDF_MAGIC = b"%PDF-"
# 展開先（root からの相対。ホストとコンテナで同じパスに見える・C4）
INBOX_REL = Path(".sai") / "paper" / "inbox"
# 題名の上限（通知の文に使うだけ・papers.DOWNLOAD_TITLE_MAX と同じ値を使う）
_TITLE_MAX = papers.DOWNLOAD_TITLE_MAX


# --- 本文の読み取り（Content-Type・大きさ・先頭） ---


async def read_bundle_upload(request) -> bytes:
    """束の本文（zip そのもの）を読む（U13 §1）。

    - `Content-Type` が `application/zip` でなければ 415
    - `Content-Length` が無いか上限（300MB）を超えれば 413
    - 読みながら上限を超えたら 413（そこで止める）

    zip の中身の検査は `extract_bundle` で行う（CLI はまだ呼ばない）。
    """
    content_type = (request.headers.get("content-type") or "").split(";")[0].strip().lower()
    if content_type != BUNDLE_CONTENT_TYPE:
        raise HTTPException(
            status_code=415,
            detail="本文は前処理の束（zip）だけを送ってください（application/zip にしてください）",
        )
    raw_length = request.headers.get("content-length")
    if raw_length is None or not raw_length.strip().isdigit():
        raise HTTPException(
            status_code=413, detail="前処理の束は 300MB までにしてください"
        )
    if int(raw_length.strip()) > BUNDLE_MAX_BYTES:
        raise HTTPException(
            status_code=413, detail="前処理の束は 300MB までにしてください"
        )
    chunks: list[bytes] = []
    total = 0
    async for chunk in request.stream():
        total += len(chunk)
        if total > BUNDLE_MAX_BYTES:
            raise HTTPException(
                status_code=413, detail="前処理の束は 300MB までにしてください"
            )
        chunks.append(chunk)
    return b"".join(chunks)


# --- zip の検査（CLI を呼ぶ前に必ず通す） ---


def _bad_zip_name(name: str) -> bool:
    """zip の項目の名前として危ないか（絶対パス・`..`・`\\`・NUL・ドライブ文字）。"""
    if not name or "\0" in name or "\\" in name:
        return True
    if name.startswith("/") or re.match(r"^[A-Za-z]:", name):
        return True
    return any(part == ".." for part in name.split("/"))


def _is_symlink(info: zipfile.ZipInfo) -> bool:
    """zip の項目がシンボリックリンクか（外部属性の上位 16 bit を見る）。"""
    return stat.S_ISLNK(info.external_attr >> 16)


def _open_zip(body: bytes) -> zipfile.ZipFile:
    """本文を zip として開く（壊れていれば 400）。"""
    try:
        return zipfile.ZipFile(io.BytesIO(body))
    except (zipfile.BadZipFile, OSError, ValueError):
        raise HTTPException(
            status_code=400, detail="前処理の束（zip）として読めませんでした"
        )


def _check_zip(zf: zipfile.ZipFile) -> tuple[dict, set[str]]:
    """zip の中身を検査し、`(manifest, 名前の集合)` を返す（違反は 400）。

    項目の数・展開後の合計・名前・シンボリックリンク・manifest.json の形・
    PDF の先頭を見る。ここで違えば CLI を 1 度も呼ばない。
    """
    infos = zf.infolist()
    if len(infos) > BUNDLE_MAX_ENTRIES:
        raise HTTPException(
            status_code=400,
            detail=f"前処理の束の項目が多すぎます（{BUNDLE_MAX_ENTRIES} まで）",
        )
    total = 0
    for info in infos:
        if _bad_zip_name(info.filename):
            raise HTTPException(
                status_code=400, detail=f"前処理の束に危ない名前があります: {info.filename}"
            )
        if _is_symlink(info):
            raise HTTPException(
                status_code=400, detail=f"前処理の束にシンボリックリンクがあります: {info.filename}"
            )
        if not info.is_dir():
            total += int(info.file_size)
            if total > BUNDLE_MAX_UNCOMPRESSED:
                raise HTTPException(
                    status_code=400, detail="前処理の束を展開すると大きすぎます（1GB まで）"
                )
    names = {info.filename for info in infos}
    if "manifest.json" not in names:
        raise HTTPException(status_code=400, detail="前処理の束に manifest.json がありません")
    try:
        with zf.open("manifest.json") as handle:
            raw = handle.read(MANIFEST_MAX_BYTES + 1)
    except (KeyError, zipfile.BadZipFile, OSError, RuntimeError, ValueError):
        raise HTTPException(status_code=400, detail="manifest.json を読めませんでした")
    if len(raw) > MANIFEST_MAX_BYTES:
        raise HTTPException(status_code=400, detail="manifest.json が大きすぎます")
    try:
        manifest = json.loads(raw.decode("utf-8"))
    except (ValueError, UnicodeDecodeError):
        raise HTTPException(status_code=400, detail="manifest.json の JSON を読めませんでした")
    if not isinstance(manifest, dict):
        raise HTTPException(status_code=400, detail="manifest.json の形が違います")
    if manifest.get("format") != BUNDLE_FORMAT or manifest.get("version") != BUNDLE_VERSION:
        raise HTTPException(
            status_code=400, detail="前処理の束の形が違います（format / version）"
        )
    _check_target(manifest.get("target"))
    pdf = manifest.get("pdf")
    if not isinstance(pdf, str) or pdf not in names:
        raise HTTPException(
            status_code=400, detail="manifest.json の pdf が前処理の束の中にありません"
        )
    try:
        with zf.open(pdf) as handle:
            head = handle.read(len(PDF_MAGIC))
    except (KeyError, zipfile.BadZipFile, OSError, RuntimeError, ValueError):
        raise HTTPException(status_code=400, detail="PDF を読めませんでした")
    if head != PDF_MAGIC:
        raise HTTPException(status_code=400, detail="PDF の先頭が %PDF- ではありません")
    docling = manifest.get("docling")
    if not isinstance(docling, dict):
        raise HTTPException(status_code=400, detail="manifest.json の docling がありません")
    for key in ("md", "json"):
        path = docling.get(key)
        if not isinstance(path, str) or path not in names:
            raise HTTPException(
                status_code=400,
                detail=f"manifest.json の docling.{key} が前処理の束の中にありません",
            )
    translation = manifest.get("translation")
    if translation is not None:
        if not isinstance(translation, dict):
            raise HTTPException(
                status_code=400, detail="manifest.json の translation の形が違います"
            )
        for key in ("mono", "dual"):
            path = translation.get(key)
            if path is not None and (not isinstance(path, str) or path not in names):
                raise HTTPException(
                    status_code=400,
                    detail=f"manifest.json の translation.{key} が前処理の束の中にありません",
                )
    return manifest, names


def _check_target(target) -> None:
    """manifest.json の target を検査する（`paper_id` か `bibtex` のどちらか 1 つ・C12）。"""
    if not isinstance(target, dict):
        raise HTTPException(status_code=400, detail="manifest.json の target がありません")
    has_id = "paper_id" in target
    has_bib = "bibtex" in target
    if has_id == has_bib:
        raise HTTPException(
            status_code=400, detail="target は paper_id か bibtex のどちらか 1 つにしてください"
        )
    if has_id:
        paper_id = target.get("paper_id")
        if isinstance(paper_id, bool) or not isinstance(paper_id, int) or paper_id <= 0:
            raise HTTPException(
                status_code=400, detail="target の paper_id は 1 以上の整数にしてください"
            )
        return
    bibtex = target.get("bibtex")
    if not isinstance(bibtex, str) or not bibtex.strip():
        raise HTTPException(status_code=400, detail="target の bibtex を入れてください")
    if len(bibtex.encode("utf-8")) > BIBTEX_MAX_BYTES:
        raise HTTPException(
            status_code=400, detail="target の bibtex は 64KB までにしてください"
        )


def _safe_extract(zf: zipfile.ZipFile, dest: Path) -> None:
    """zip を `dest` の下だけに展開する（名前は `_check_zip` で検査済み）。"""
    base = dest.resolve()
    written = 0
    for info in zf.infolist():
        target = dest / info.filename
        if not target.resolve().is_relative_to(base):
            raise HTTPException(
                status_code=400, detail=f"前処理の束に危ない名前があります: {info.filename}"
            )
        if info.is_dir():
            target.mkdir(parents=True, exist_ok=True)
            continue
        target.parent.mkdir(parents=True, exist_ok=True)
        with zf.open(info) as src, open(target, "wb") as out:
            while True:
                chunk = src.read(1024 * 1024)
                if not chunk:
                    break
                written += len(chunk)
                if written > BUNDLE_MAX_UNCOMPRESSED:
                    raise HTTPException(
                        status_code=400, detail="前処理の束を展開すると大きすぎます（1GB まで）"
                    )
                out.write(chunk)


def extract_bundle(project: dict, body: bytes) -> tuple[Path, dict]:
    """検査して `<root>/.sai/paper/inbox/<uuid>/` へ展開し、`(フォルダ, manifest)` を返す。

    違反は 400（CLI を呼ばない）。展開に失敗したら途中のフォルダを消す。
    """
    zf = _open_zip(body)
    with zf:
        manifest, _names = _check_zip(zf)
        folder = Path(project["root"]) / INBOX_REL / uuid.uuid4().hex
        try:
            folder.mkdir(parents=True, exist_ok=True)
            _safe_extract(zf, folder)
        except HTTPException:
            shutil.rmtree(folder, ignore_errors=True)
            raise
        except OSError as e:
            shutil.rmtree(folder, ignore_errors=True)
            logger.warning(f"前処理の束を展開できません: {e}")
            raise HTTPException(
                status_code=400, detail="前処理の束を展開できませんでした"
            )
    return folder, manifest


# --- 題名（通知の文に使うだけ） ---


def _bibtex_title(bibtex: str) -> str:
    """BibTeX 1 件から題名をざっくり取る（取れなければ空文字）。"""
    match = re.search(r"title\s*=\s*[{\"](.+?)[}\"]\s*,?\s*\n", bibtex, re.IGNORECASE | re.DOTALL)
    if match is None:
        match = re.search(r"title\s*=\s*[{\"](.+?)[}\"]", bibtex, re.IGNORECASE | re.DOTALL)
    if match is None:
        return ""
    return " ".join(match.group(1).split())


def _manifest_title(project: dict, manifest: dict) -> str:
    """bibtex の題名を取る（無ければ空文字）。"""
    target = manifest.get("target") or {}
    if isinstance(target.get("bibtex"), str):
        return _bibtex_title(target["bibtex"])
    return ""


async def _title_for(project: dict, manifest: dict) -> str:
    """仕事と通知に出す題名（bibtex の題名か、paper_id の論文の題名・U13 §1）。"""
    target = manifest.get("target") or {}
    title = _manifest_title(project, manifest)
    if not title and isinstance(target.get("paper_id"), int):
        try:
            info = await run_in_threadpool(papers.detail, project, target["paper_id"])
            paper = info.get("paper") if isinstance(info, dict) else {}
            title = str((paper or {}).get("title") or "")
        except Exception:  # 題名は飾りなので、取れなくても取り込みは続ける
            title = ""
    return (title or "前処理の束")[:_TITLE_MAX]


# --- 依頼と背景の実行 ---


async def request_bundle(project: dict, body: bytes) -> dict:
    """束の依頼（`POST …/papers/bundle`・U13 §1）。

    検査して展開し（違えば 400 / 413 / 415）、`kind="bundle"` の仕事を作って
    202 ですぐ返す。取り込みは `_bundle_job` を背景で走らせる。
    """
    folder, manifest = extract_bundle(project, body)
    title = await _title_for(project, manifest)
    key = f"bundle:{uuid.uuid4().hex}"
    job = papers._new_job("bundle", key, title, "processing", papers.BUNDLE_STEPS, 0)
    papers._download_jobs[(project["name"], key)] = job
    job["_task"] = asyncio.create_task(_bundle_job(project, folder, key, job, title))
    return {"key": key, "status": "processing"}


async def _bundle_job(
    project: dict, folder: Path, key: str, job: dict, title: str
) -> None:
    """背景で `sai-paper paper import-bundle --dir <フォルダ> --json` を走らせる（U13 §1）。

    - 終わったら（成否どちらでも）展開したフォルダを消す。
    - 成功は `done`（結果の `paper_id`・`figures`・`translated` を入れる）、
      失敗は `failed`（CLI の標準エラーの最後の行）。
    - 例外は全部つかまえてログに出す（サーバーは止めない）。
    """
    name = project["name"]
    root = str(project["root"])
    fail_url = "/#/papers/library"

    def fail(error: str) -> None:
        papers._finish_download(job, "failed", error=error)
        notify.broadcast("前処理の束を取り込めませんでした", f"{title}: {error}", fail_url)

    try:
        code, stdout, reason = await papers._run_paper_command(
            root,
            ["paper", "import-bundle", "--dir", str(folder), "--json"],
            BUNDLE_TIMEOUT,
        )
        if reason:
            fail(reason)
            return
        if code != 0:
            fail(papers._radio_error(stdout))
            return
        data = papers._download_json(stdout)
        paper_id = data.get("paper_id")
        if isinstance(paper_id, bool) or not isinstance(paper_id, int):
            fail("sai-paper の応答を読めませんでした")
            return
        job["paper_id"] = paper_id
        job["figures"] = data.get("figures")
        job["translated"] = data.get("translated")
        papers._finish_download(job, "done")
        notify.broadcast("📄 論文を保存しました", title, f"/#/paper/{name}/{paper_id}")
    except Exception as e:  # サーバーは止めない（Download と同じ扱い）
        logger.exception(f"前処理の束の取り込みで失敗: {e}")
        if job.get("status") not in ("done", "failed"):
            fail("原因不明のエラー")
    finally:
        shutil.rmtree(folder, ignore_errors=True)


# --- sai-prep が使う読み取り（C13・C14） ---


def pending(project: dict) -> dict:
    """PDF 待ちの論文の一覧（`sai-paper library pending --json`・C13）。

    sai-prep が「どの論文の PDF を送ればよいか」を知るために使う。
    """
    return papers._paper_cli(project, ["library", "pending", "--json"])


def job_view(project: dict, key: str) -> dict:
    """仕事 1 件（`GET …/papers/jobs/{key}`・U13 §1・C14）。無ければ 404。"""
    job = papers._download_jobs.get((project["name"], key))
    if job is None:
        raise HTTPException(status_code=404, detail="その仕事はありません")
    out = {
        "key": job.get("key"),
        "status": job.get("status"),
        "title": job.get("title"),
    }
    for name in ("paper_id", "figures", "translated", "error"):
        value = job.get(name)
        if value is not None:
            out[name] = value
    return out
