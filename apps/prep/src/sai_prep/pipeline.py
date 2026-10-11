"""docling・mlx-lm・pdf2zh を順に動かし、C12 の「前処理の束」（zip）を作ってサーバへ送る。

仕事は 1 つずつ、列（1 本のスレッド）で処理する。docling と mlx のサーバは重ねない
（16GB の Mac でメモリを取り合わないため）。列が空になったら mlx のサーバを止める。
"""

from __future__ import annotations

import hashlib
import json
import os
import queue
import re
import shutil
import subprocess
import threading
import time
import uuid
import zipfile
from dataclasses import dataclass, field
from datetime import datetime, timezone
from pathlib import Path
from typing import Callable

from . import __version__, config as config_mod, migrate
from .client import Client, ClientError

# 時間の上限（秒）。
DOCLING_TIMEOUT = 30 * 60
PDF2ZH_TIMEOUT = 60 * 60
MLX_START_TIMEOUT = 30 * 60
JOB_POLL_TIMEOUT = 15 * 60
JOB_POLL_INTERVAL = 3.0
# 移植（DB Migration）の取り込みの待ち時間の上限。束は論文 1 本でも大きいことがある。
MIGRATE_POLL_TIMEOUT = 60 * 60

# 前処理の束の名前（共有契約 C12）。
BUNDLE_FORMAT = "sai-prep-bundle"
BUNDLE_VERSION = 1

SETUP_HINT = "`sai-prep setup` を実行してください。"


class PipelineError(Exception):
    """前処理の途中で失敗した。画面にそのまま出す理由。"""


@dataclass
class Job:
    """前処理の仕事 1 つ。"""

    id: str
    project: str
    kind: str  # "paper"（行への drop）・"bibtex"（BibTeX と PDF）・"migrate"（DB Migration）
    target: dict
    filename: str
    status: str = "queued"  # queued / running / done / failed
    stage: str = "待機"
    message: str = ""
    progress: int = 0
    created_at: str = field(default_factory=lambda: datetime.now(timezone.utc).isoformat())
    result: dict | None = None
    error: str | None = None
    # PDF の本文。画面には出さない（大きいため）。
    pdf: bytes = field(default=b"", repr=False, compare=False)
    # 渡された翻訳。None は config.translate に従う。{} は翻訳しない。
    # {"mono": …, "dual": …} は pdf2zh を動かさずに使う。
    translations: dict | None = field(default=None, repr=False, compare=False)

    def to_public(self) -> dict:
        return {
            "id": self.id,
            "project": self.project,
            "kind": self.kind,
            "target": self.target,
            "filename": self.filename,
            "status": self.status,
            "stage": self.stage,
            "message": self.message,
            "progress": self.progress,
            "created_at": self.created_at,
            "result": self.result,
            "error": self.error,
        }


def find_tool(name: str, extra_dirs: list[str] | None = None) -> str | None:
    """道具の実行ファイルを探す。追加のフォルダ、`PATH`、`~/.local/bin` の順に見る。"""
    for directory in [Path(d) for d in (extra_dirs or [])]:
        candidate = directory / name
        if candidate.is_file() and os.access(candidate, os.X_OK):
            return str(candidate)
    found = shutil.which(name)
    if found:
        return found
    local = Path.home() / ".local" / "bin" / name
    if local.is_file() and os.access(local, os.X_OK):
        return str(local)
    return None


def tool_version(binary: str) -> str:
    """道具の版を 1 行で返す。取れなければ空文字。

    pdf2zh は `--version` の 1 行目に「not in git repo」を出すので、版らしい行
    （数字を含む行）を優先する。
    """
    for args in (["--version"], ["version"]):
        try:
            done = subprocess.run(
                [binary, *args], capture_output=True, text=True, timeout=30
            )
        except (OSError, subprocess.TimeoutExpired):
            continue
        text = ((done.stdout or "") + "\n" + (done.stderr or "")).strip()
        lines = [line.strip() for line in text.splitlines() if line.strip()]
        if not lines:
            continue
        for line in lines:
            if re.search(r"\d+\.\d+", line):
                return line
        return lines[0]
    return ""


def _tail(text: str, limit: int = 500) -> str:
    text = (text or "").strip()
    if len(text) <= limit:
        return text
    return text[-limit:]


class Pipeline:
    """仕事の列を 1 本のスレッドで回す。"""

    def __init__(
        self,
        config_provider: Callable[[], config_mod.Config],
        client_factory: Callable[[str, str], Client] = Client,
        work_root: Path | str | None = None,
        extra_tool_dirs: list[str] | None = None,
        poll_interval: float = JOB_POLL_INTERVAL,
    ):
        self._config_provider = config_provider
        self._client_factory = client_factory
        self._work_root = Path(work_root) if work_root else config_mod.jobs_dir()
        self._extra_tool_dirs = list(extra_tool_dirs or [])
        self._poll_interval = float(poll_interval)
        self._queue: "queue.Queue[str]" = queue.Queue()
        self._jobs: dict[str, Job] = {}
        self._lock = threading.Lock()
        self._mlx_lock = threading.Lock()
        self._mlx_proc: subprocess.Popen | None = None
        self._mlx_port: int | None = None
        self._worker = threading.Thread(
            target=self._run, daemon=True, name="sai-prep-worker"
        )
        self._worker.start()

    # -- 仕事の受け付け ----------------------------------------------------

    def submit_paper(
        self,
        project: str,
        paper_id: int,
        pdf_bytes: bytes,
        filename: str,
        translations: dict | None = None,
    ) -> Job:
        """「PDF 待ち」の行に drop された PDF の仕事。"""
        target = {"paper_id": int(paper_id)}
        return self._submit(project, "paper", target, pdf_bytes, filename, translations)

    def submit_bibtex(
        self,
        project: str,
        bibtex: str,
        pdf_bytes: bytes,
        filename: str,
        translations: dict | None = None,
    ) -> Job:
        """BibTeX と PDF を一緒に drop した仕事。"""
        target = {"bibtex": bibtex}
        return self._submit(project, "bibtex", target, pdf_bytes, filename, translations)

    def submit_migration(self, project: str, target: dict, filename: str) -> Job:
        """DB Migration の仕事。target は `{"source":"moon","path":…,"ids":[id]}` か `{"source":"tree","project":…,"ids":[…]}`。"""
        return self._submit(project, "migrate", target, b"", filename)

    def _submit(
        self,
        project: str,
        kind: str,
        target: dict,
        pdf_bytes: bytes,
        filename: str,
        translations: dict | None = None,
    ) -> Job:
        job = Job(
            id=uuid.uuid4().hex,
            project=project,
            kind=kind,
            target=target,
            filename=filename or "paper.pdf",
            pdf=pdf_bytes,
            translations=translations,
        )
        with self._lock:
            self._jobs[job.id] = job
        self._queue.put(job.id)
        return job

    def retry(self, job_id: str) -> Job | None:
        """失敗した仕事を、同じ PDF でもう一度列に積む（古い失敗の行は捨てる）。"""
        with self._lock:
            old = self._jobs.get(job_id)
            if old is None or old.status != "failed":
                return None
            del self._jobs[job_id]
        return self._submit(
            old.project, old.kind, old.target, old.pdf, old.filename, old.translations
        )

    def get(self, job_id: str) -> Job | None:
        with self._lock:
            return self._jobs.get(job_id)

    def list_jobs(self) -> list[Job]:
        with self._lock:
            return list(self._jobs.values())

    def wait(self, job_id: str, timeout: float = 300.0) -> Job:
        """仕事が終わるまで待つ（テストと、必要なら外から使う）。"""
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            job = self.get(job_id)
            if job is not None and job.status in ("done", "failed"):
                return job
            time.sleep(0.05)
        raise TimeoutError(f"仕事 {job_id} が時間内に終わりません")

    @property
    def mlx_running(self) -> bool:
        with self._mlx_lock:
            return self._mlx_proc is not None and self._mlx_proc.poll() is None

    def close(self) -> None:
        """列を止めて mlx のサーバを片付ける。"""
        self._queue.put("")
        self._stop_mlx()

    # -- 仕事を回す --------------------------------------------------------

    def _run(self) -> None:
        while True:
            job_id = self._queue.get()
            if not job_id:
                return
            job = self.get(job_id)
            if job is None:
                continue
            try:
                if job.kind == "migrate":
                    self._process_migration(job)
                else:
                    self._process(job)
            except PipelineError as error:
                job.status = "failed"
                job.error = str(error)
                job.stage = "失敗"
            except Exception as error:  # 予期しない失敗も理由を残す
                job.status = "failed"
                job.error = f"予期しない失敗: {error}"
                job.stage = "失敗"
            finally:
                if self._queue.empty():
                    self._stop_mlx()

    def _set(self, job: Job, stage: str, progress: int, message: str = "") -> None:
        job.status = "running"
        job.stage = stage
        job.progress = progress
        job.message = message

    def _process(self, job: Job) -> None:
        config = self._config_provider()
        workdir = self._work_root / job.id
        workdir.mkdir(parents=True, exist_ok=True)
        try:
            self._set(job, "PDF を置く", 5)
            pdf_path = workdir / "paper.pdf"
            pdf_path.write_bytes(job.pdf)
            pdf_sha256 = self._prepare_pdf(pdf_path)

            docling_bin = self._require_tool("docling")
            docling_version = tool_version(docling_bin)

            self._set(job, "docling で変換中", 20)
            self._run_docling(docling_bin, workdir, config.docling_device)
            normalize_docling_output(workdir)
            self._check_files(workdir / "docling" / "paper.md", workdir / "docling" / "paper.json", stage="docling")

            # service が None なら build_bundle が mlx:<モデル> を書く。
            translated, pdf2zh_version, service = False, "", None
            if job.translations is None:
                translated = bool(config.translate)
                if translated:
                    self._set(job, "翻訳中", 45, "翻訳のモデルを準備中")
                    pdf2zh_bin = self._require_tool("pdf2zh")
                    pdf2zh_version = tool_version(pdf2zh_bin)
                    port = self._ensure_mlx(config)
                    self._set(job, "翻訳中", 55)
                    self._run_pdf2zh(pdf2zh_bin, workdir, config.mlx_model, port)
                    self._check_files(
                        workdir / "translated" / "paper-mono.pdf",
                        workdir / "translated" / "paper-dual.pdf",
                        stage="翻訳",
                    )
            elif job.translations:
                # agent が送った翻訳。pdf2zh は動かさない。
                self._set(job, "翻訳を置く", 55)
                (workdir / "translated").mkdir(parents=True, exist_ok=True)
                (workdir / "translated" / "paper-mono.pdf").write_bytes(job.translations["mono"])
                (workdir / "translated" / "paper-dual.pdf").write_bytes(job.translations["dual"])
                translated, service = True, "provided"

            self._set(job, "束を作成中", 75)
            bundle_path = build_bundle(
                workdir,
                pdf_bytes=job.pdf,
                target=job.target,
                pdf_sha256=pdf_sha256,
                docling_version=docling_version,
                pdf2zh_version=pdf2zh_version,
                translated=translated,
                mlx_model=config.mlx_model,
                service=service,
            )

            self._set(job, "送信中", 85)
            self._send(job, config, bundle_path)

            self._set(job, "完了", 100)
            job.status = "done"
            shutil.rmtree(workdir, ignore_errors=True)
        except PipelineError:
            raise
        except Exception as error:
            raise PipelineError(f"予期しない失敗: {error}") from error

    def _process_migration(self, job: Job) -> None:
        """DB Migration: moon は束を作って送り、tree はサーバの中で移すよう頼む。"""
        config = self._config_provider()
        client = self._client_for(config)
        target = job.target
        workdir = self._work_root / job.id
        try:
            if target.get("source") == "moon":
                workdir.mkdir(parents=True, exist_ok=True)
                self._set(job, "束を作成中", 10)
                package = workdir / "migration.zip"
                try:
                    migrate.build_package(Path(target["path"]), target["ids"], package)
                except migrate.MigrateError as error:
                    raise PipelineError(str(error)) from error
                self._set(job, "送信中", 40)
                blob = package.read_bytes()
                try:
                    response = client.post_migrate(job.project, blob)
                except ClientError as error:
                    raise PipelineError(f"束を送れません: {error}") from error
            else:
                self._set(job, "移植を依頼中", 40)
                try:
                    response = client.post_migrate_from(job.project, target["project"], target["ids"])
                except ClientError as error:
                    raise PipelineError(f"移植を頼めません: {error}") from error
            key = response.get("key")
            if not key:
                raise PipelineError("移植を頼みましたが、サーバが job の鍵を返しませんでした")
            self._set(job, "取り込みを待っています", 60)
            state = _wait_server_job(client, job.project, key, MIGRATE_POLL_TIMEOUT, self._poll_interval)
            job.result = state.get("result")
            self._set(job, "完了", 100)
            job.status = "done"
            shutil.rmtree(workdir, ignore_errors=True)
        except PipelineError:
            raise
        except Exception as error:
            raise PipelineError(f"予期しない失敗: {error}") from error

    # -- 各段 --------------------------------------------------------------

    def _prepare_pdf(self, pdf_path: Path) -> str:
        data = pdf_path.read_bytes()
        if not data.startswith(b"%PDF-"):
            raise PipelineError("PDF ではないファイルです（先頭が %PDF- ではありません）")
        return hashlib.sha256(data).hexdigest()

    def _require_tool(self, name: str) -> str:
        found = find_tool(name, self._extra_tool_dirs)
        if not found:
            raise PipelineError(f"{name} が見つかりません。{SETUP_HINT}")
        return found

    def _run_docling(self, binary: str, workdir: Path, device: str) -> None:
        args = [
            binary,
            "convert",
            "paper.pdf",
            "--to",
            "md",
            "--to",
            "json",
            "--image-export-mode",
            "referenced",
            "--output",
            "docling",
            "--device",
            device,
        ]
        self._run_command(args, workdir, timeout=DOCLING_TIMEOUT, stage="docling")

    def _run_pdf2zh(self, binary: str, workdir: Path, model: str, port: int) -> None:
        prompt_path = workdir / "prompt.txt"
        # 言語は ISO コードではなく名前で書く（`_immersive-translate` 版のテンプレートが
        # 名前を前提にしているため。調査 R2）。
        prompt_path.write_text(
            "<<<source>>>English<<<target>>>Japanese<<<text>>>${text}\n", encoding="utf-8"
        )
        # pdf2zh は出力フォルダを自分で作らないので、先に作っておく。
        (workdir / "translated").mkdir(parents=True, exist_ok=True)
        env = os.environ.copy()
        env["OPENAILIKED_BASE_URL"] = f"http://127.0.0.1:{port}/v1"
        env["OPENAILIKED_API_KEY"] = "EMPTY"
        env["OPENAILIKED_MODEL"] = model
        args = [
            binary,
            "paper.pdf",
            "-o",
            "translated",
            "-li",
            "en",
            "-lo",
            "ja",
            "--service",
            "openailiked",
            "--prompt",
            "prompt.txt",
        ]
        self._run_command(args, workdir, env=env, timeout=PDF2ZH_TIMEOUT, stage="pdf2zh")

    def _run_command(
        self,
        args: list[str],
        cwd: Path,
        *,
        env: dict | None = None,
        timeout: int,
        stage: str,
    ) -> None:
        try:
            done = subprocess.run(
                args,
                cwd=str(cwd),
                env=env,
                capture_output=True,
                text=True,
                timeout=timeout,
            )
        except subprocess.TimeoutExpired as error:
            raise PipelineError(f"{stage} が時間の上限（{timeout // 60} 分）を超えました") from error
        except OSError as error:
            raise PipelineError(f"{stage} を実行できません: {error}") from error
        if done.returncode != 0:
            reason = _tail(done.stderr) or _tail(done.stdout) or f"終了コード {done.returncode}"
            raise PipelineError(f"{stage} が失敗しました: {reason}")

    def _check_files(self, *paths: Path, stage: str) -> None:
        for path in paths:
            if not path.is_file():
                raise PipelineError(f"{stage} の出力が見つかりません: {path.name}")

    def _ensure_mlx(self, config: config_mod.Config) -> int:
        with self._mlx_lock:
            if self._mlx_proc is not None and self._mlx_proc.poll() is None:
                return int(self._mlx_port)
            binary = self._require_tool("mlx_lm.server")
            port = int(config.mlx_port)
            proc = subprocess.Popen(
                [
                    binary,
                    "--model",
                    config.mlx_model,
                    "--host",
                    "127.0.0.1",
                    "--port",
                    str(port),
                ],
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
            )
            if not _wait_for_mlx(port, MLX_START_TIMEOUT):
                if proc.poll() is None:
                    proc.terminate()
                raise PipelineError(
                    "翻訳のモデルを準備できませんでした（mlx のサーバが応答しません）"
                )
            self._mlx_proc = proc
            self._mlx_port = port
            return port

    def _stop_mlx(self) -> None:
        with self._mlx_lock:
            proc = self._mlx_proc
            self._mlx_proc = None
            self._mlx_port = None
        if proc is not None and proc.poll() is None:
            proc.terminate()
            try:
                proc.wait(timeout=10)
            except subprocess.TimeoutExpired:
                proc.kill()

    # -- 送信 --------------------------------------------------------------

    def _client_for(self, config: config_mod.Config) -> Client:
        if not config.server_url:
            raise PipelineError("サーバの住所が設定されていません（設定を入れてください）")
        if not config.token:
            raise PipelineError("個人のトークンが設定されていません（設定を入れてください）")
        return self._client_factory(config.server_url, config.token)

    def _send(self, job: Job, config: config_mod.Config, bundle_path: Path) -> None:
        client = self._client_for(config)
        try:
            response = client.post_bundle(job.project, bundle_path.read_bytes())
        except ClientError as error:
            raise PipelineError(f"束を送れません: {error}") from error
        key = response.get("key")
        if not key:
            raise PipelineError("束を送りましたが、サーバが job の鍵を返しませんでした")
        job.result = _wait_server_job(client, job.project, key, JOB_POLL_TIMEOUT, self._poll_interval)


def _wait_server_job(
    client: Client, project: str, key: str, timeout: float, interval: float
) -> dict:
    """サーバの仕事が done になるまで待つ。done の答えを返す。failed・時間切れは PipelineError。"""
    deadline = time.monotonic() + timeout
    while True:
        try:
            state = client.job(project, key)
        except ClientError as error:
            raise PipelineError(f"取り込みの進み具合を読めません: {error}") from error
        status = state.get("status")
        if status == "done":
            return state
        if status == "failed":
            raise PipelineError(state.get("error") or "サーバの取り込みに失敗しました")
        if time.monotonic() > deadline:
            raise PipelineError(f"サーバの取り込みの待ち時間が上限（{timeout // 60} 分）を超えました")
        time.sleep(interval)


def _wait_for_mlx(port: int, timeout: float) -> bool:
    """mlx のサーバの `GET /v1/models` が 200 になるまで待つ。"""
    import urllib.error
    import urllib.request

    deadline = time.monotonic() + timeout
    url = f"http://127.0.0.1:{port}/v1/models"
    while time.monotonic() < deadline:
        try:
            with urllib.request.urlopen(url, timeout=5) as response:
                if response.status == 200:
                    return True
        except (urllib.error.URLError, OSError):
            pass
        time.sleep(1.0)
    return False


def build_bundle(
    workdir: Path,
    *,
    pdf_bytes: bytes,
    target: dict,
    pdf_sha256: str,
    docling_version: str,
    pdf2zh_version: str,
    translated: bool,
    mlx_model: str,
    service: str | None = None,
) -> Path:
    """C12 の形の zip を作り、そのパスを返す。

    `paper.pdf` はファイルからではなく渡された本文から書く。pdf2zh は入力 PDF が
    システムの一時フォルダの中にあると、それを消してしまう（pdf2zh 1.9.11 の
    `high_level.py`）ため、束が入力 PDF の残存に依存しないようにしている。
    """
    manifest = {
        "format": BUNDLE_FORMAT,
        "version": BUNDLE_VERSION,
        "target": target,
        "pdf": "paper.pdf",
        "pdf_sha256": pdf_sha256,
        "docling": {
            "md": "docling/paper.md",
            "json": "docling/paper.json",
            "dir": "docling",
        },
        "translation": None,
        "tools": {
            "sai_prep": __version__,
            "docling": docling_version,
            "pdf2zh": pdf2zh_version,
        },
        "created_at": datetime.now().astimezone().isoformat(),
    }
    if translated:
        manifest["translation"] = {
            "mono": "translated/paper-mono.pdf",
            "dual": "translated/paper-dual.pdf",
            "service": service if service is not None else f"mlx:{mlx_model}",
        }
    (workdir / "manifest.json").write_text(
        json.dumps(manifest, ensure_ascii=False, indent=2) + "\n", encoding="utf-8"
    )

    bundle_path = workdir / "bundle.zip"
    with zipfile.ZipFile(bundle_path, "w", zipfile.ZIP_DEFLATED) as archive:
        archive.write(workdir / "manifest.json", "manifest.json")
        archive.writestr("paper.pdf", pdf_bytes)
        _add_tree(archive, workdir / "docling", "docling")
        if translated:
            _add_tree(archive, workdir / "translated", "translated")
    return bundle_path


def normalize_docling_output(workdir: Path) -> None:
    """docling の出力を C12 の形（`docling/paper_artifacts/…`）にそろえる。

    docling 2.123.1 は `--output docling` のとき図を `docling/docling/paper_artifacts/…`
    に書く（JSON の参照は `docling/paper_artifacts/…` のまま）。C12 は 1 段なので、
    余分な段を上へ動かす。2.134.0 のように最初から 1 段のときは何もしない。
    """
    nested = workdir / "docling" / "docling"
    if not nested.is_dir():
        return
    parent = workdir / "docling"
    for child in list(nested.iterdir()):
        target = parent / child.name
        if target.exists():
            continue
        shutil.move(str(child), str(target))
    try:
        nested.rmdir()
    except OSError:
        pass


def _add_tree(archive: zipfile.ZipFile, root: Path, prefix: str) -> None:
    if not root.exists():
        return
    for path in sorted(root.rglob("*")):
        if path.is_file():
            archive.write(path, f"{prefix}/{path.relative_to(root).as_posix()}")
