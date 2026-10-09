"""sai-prep のローカルの Web 画面と API。

標準ライブラリの `http.server` で `127.0.0.1` だけで待ち受ける。画面からの要求には
起動のたびに作る合言葉（`secrets.token_urlsafe(16)`）を `index.html` に埋めて付けさせ、
無い要求は 403 にする（ほかのサイトのページが localhost を叩けないように）。
"""

from __future__ import annotations

import base64
import binascii
import hmac
import io
import json
import re
import secrets
import tempfile
import urllib.parse
import zipfile
import zlib
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from typing import Callable

from . import __version__, config as config_mod
from .client import Client, ClientError
from .pipeline import Pipeline, PipelineError

NONCE_HEADER = "X-Sai-Prep-Nonce"
STATIC_DIR = Path(__file__).with_name("static")
INDEX_PATH = STATIC_DIR / "index.html"
MAX_BIBTEX_BYTES = 64 * 1024
# データファイル（Data upload）。1 件の上限は sai-web と同じ 5GB。名前の規則も sai-web と同じ。
DATA_MAX_BYTES = 5 * 1024**3
DATA_CHUNK_BYTES = 1024 * 1024
DATA_NAME_RE = re.compile(r"[A-Za-z0-9][A-Za-z0-9_.-]{0,127}")


class PrepHTTPServer(ThreadingHTTPServer):
    """画面と API を出すサーバ。"""

    daemon_threads = True
    allow_reuse_address = True

    def __init__(
        self,
        address: tuple[str, int],
        pipeline: Pipeline,
        config_provider: Callable[[], config_mod.Config],
        client_factory: Callable[[str, str], Client] = Client,
    ):
        super().__init__(address, _Handler)
        self.pipeline = pipeline
        self.config_provider = config_provider
        self.client_factory = client_factory
        self.nonce = secrets.token_urlsafe(16)


def _upload_dir() -> Path:
    """データの一時置き場。送り終えたら（成否どちらでも）中のファイルを消す。"""
    return Path.home() / "Library" / "Caches" / "sai-prep" / "uploads"


class _Handler(BaseHTTPRequestHandler):
    server_version = f"sai-prep/{__version__}"
    protocol_version = "HTTP/1.1"

    # -- 道具 --------------------------------------------------------------

    def log_message(self, format: str, *args) -> None:  # noqa: A002
        # 画面の要求で端末を埋めない。
        pass

    @property
    def prep(self) -> PrepHTTPServer:
        return self.server  # type: ignore[return-value]

    def _read_body(self) -> bytes:
        length = self.headers.get("Content-Length")
        if not length:
            return b""
        try:
            size = int(length)
        except ValueError:
            return b""
        if size <= 0:
            return b""
        return self.rfile.read(size)

    def _read_json(self) -> dict:
        body = self._read_body()
        if not body:
            return {}
        try:
            data = json.loads(body.decode("utf-8"))
        except (UnicodeDecodeError, json.JSONDecodeError):
            return {}
        return data if isinstance(data, dict) else {}

    def _send(self, status: int, body: bytes, content_type: str) -> None:
        self.send_response(status)
        self.send_header("Content-Type", content_type)
        self.send_header("Content-Length", str(len(body)))
        self.send_header("Cache-Control", "no-store")
        self.end_headers()
        if self.command != "HEAD":
            self.wfile.write(body)

    def _send_json(self, status: int, payload: object) -> None:
        body = json.dumps(payload, ensure_ascii=False).encode("utf-8")
        self._send(status, body, "application/json; charset=utf-8")

    def _error(self, status: int, message: str) -> None:
        self._send_json(status, {"error": message})

    def _nonce_ok(self) -> bool:
        given = self.headers.get(NONCE_HEADER, "")
        return bool(given) and hmac.compare_digest(given, self.prep.nonce)

    def _require_nonce(self) -> bool:
        if self._nonce_ok():
            return True
        self._error(403, "合言葉がありません。画面を開き直してください。")
        return False

    # -- 経路 --------------------------------------------------------------

    def do_GET(self) -> None:  # noqa: N802
        parsed = urllib.parse.urlparse(self.path)
        path = parsed.path
        query = urllib.parse.parse_qs(parsed.query)
        if path in ("/", "/index.html"):
            self._serve_index()
            return
        if path == "/local/config":
            if not self._require_nonce():
                return
            self._send_json(200, self.prep.config_provider().to_dict())
            return
        if path == "/local/projects":
            if not self._require_nonce():
                return
            self._projects()
            return
        if path == "/local/pending":
            if not self._require_nonce():
                return
            self._pending(query.get("project", [""])[0])
            return
        if path == "/local/jobs":
            if not self._require_nonce():
                return
            jobs = [job.to_public() for job in self.prep.pipeline.list_jobs()]
            self._send_json(200, {"jobs": jobs})
            return
        if path == "/local/data":
            if not self._require_nonce():
                return
            self._data_list(query.get("project", [""])[0])
            return
        self._error(404, "その住所はありません。")

    def do_PUT(self) -> None:  # noqa: N802
        path = urllib.parse.urlparse(self.path).path
        if path == "/local/config":
            if not self._require_nonce():
                return
            self._put_config()
            return
        self._error(404, "その住所はありません。")

    def do_POST(self) -> None:  # noqa: N802
        parsed = urllib.parse.urlparse(self.path)
        path = parsed.path
        query = urllib.parse.parse_qs(parsed.query)
        if path == "/local/check":
            if not self._require_nonce():
                return
            self._check()
            return
        if path == "/local/jobs":
            if not self._require_nonce():
                return
            self._post_job(query)
            return
        if path == "/local/data":
            if not self._require_nonce():
                return
            self._post_data(query)
            return
        if path.startswith("/local/jobs/") and path.endswith("/retry"):
            if not self._require_nonce():
                return
            job = self.prep.pipeline.retry(path[len("/local/jobs/"):-len("/retry")])
            if job is None:
                self._error(404, "やり直せる失敗の仕事がありません。")
                return
            self._send_json(202, job.to_public())
            return
        self._error(404, "その住所はありません。")

    # -- 画面 --------------------------------------------------------------

    def _serve_index(self) -> None:
        try:
            html = INDEX_PATH.read_text(encoding="utf-8")
        except OSError:
            self._error(500, "画面のファイルが読めません。")
            return
        html = html.replace("__SAI_PREP_NONCE__", self.prep.nonce)
        html = html.replace("__SAI_PREP_VERSION__", __version__)
        self._send(200, html.encode("utf-8"), "text/html; charset=utf-8")

    # -- API ---------------------------------------------------------------

    def _put_config(self) -> None:
        data = self._read_json()
        config = self.prep.config_provider()
        if "server_url" in data:
            url = str(data["server_url"]).strip()
            if url and not config_mod.allowed_server_url(url):
                self._error(
                    400,
                    "サーバの住所は https:// か http://127.0.0.1 / http://localhost だけを使えます。",
                )
                return
            config.server_url = config_mod.normalize_server_url(url)
        if "token" in data:
            config.token = str(data["token"]).strip()
        if "project" in data:
            config.project = str(data["project"]).strip()
        if "translate" in data:
            config.translate = bool(data["translate"])
        if "mlx_model" in data and str(data["mlx_model"]).strip():
            config.mlx_model = str(data["mlx_model"]).strip()
        if "mlx_port" in data:
            try:
                config.mlx_port = int(data["mlx_port"])
            except (TypeError, ValueError):
                self._error(400, "mlx_port は整数で入れてください。")
                return
        if "docling_device" in data and str(data["docling_device"]).strip():
            config.docling_device = str(data["docling_device"]).strip()
        config_mod.save(config)
        self._send_json(200, config.to_dict())

    def _check(self) -> None:
        data = self._read_json()
        config = self.prep.config_provider()
        url = config_mod.normalize_server_url(str(data.get("server_url", config.server_url)))
        token = str(data.get("token", config.token)).strip()
        if not url:
            self._error(400, "サーバの住所を入れてください。")
            return
        if not config_mod.allowed_server_url(url):
            self._error(
                400,
                "サーバの住所は https:// か http://127.0.0.1 / http://localhost だけを使えます。",
            )
            return
        try:
            me = self.prep.client_factory(url, token).me()
        except ClientError as error:
            self._error(400, str(error))
            return
        self._send_json(200, {"ok": True, "name": me.get("name", ""), "email": me.get("email", "")})

    def _projects(self) -> None:
        config = self.prep.config_provider()
        try:
            projects = self.prep.client_factory(config.server_url, config.token).projects()
        except ClientError as error:
            self._error(400, str(error))
            return
        self._send_json(200, {"projects": projects})

    def _pending(self, project: str) -> None:
        if not project:
            self._error(400, "project を指定してください。")
            return
        config = self.prep.config_provider()
        try:
            result = self.prep.client_factory(config.server_url, config.token).pending(project)
        except ClientError as error:
            self._error(400, str(error))
            return
        self._send_json(200, result)

    def _data_list(self, project: str) -> None:
        if not project:
            self._error(400, "project を指定してください。")
            return
        config = self.prep.config_provider()
        try:
            files = self.prep.client_factory(config.server_url, config.token).data_files(project)
        except ClientError as error:
            self._error(400, str(error))
            return
        self._send_json(200, {"files": files})

    def _refuse(self, status: int, message: str) -> None:
        """本文を読まずに断る。残りの本文が次の要求に混ざらないよう、この接続は閉じる。"""
        self.close_connection = True
        self._error(status, message)

    def _post_data(self, query: dict[str, list[str]]) -> None:
        """データ 1 件を受けて sai-web へ送る（U01）。

        本文は一時ファイルへ書いてから送る（`rfile` を直接渡すと、keep-alive の接続は
        EOF にならず送信が終わらないため）。送り終えたら一時ファイルは消す。
        """
        project = (query.get("project", [""])[0] or "").strip()
        filename = (query.get("filename", [""])[0] or "").strip()
        if not project:
            self._refuse(400, "project を指定してください。")
            return
        if not DATA_NAME_RE.fullmatch(filename):
            self._refuse(
                400,
                "ファイル名は英数字と . _ - だけで、先頭は英数字にしてください（128 字まで）。",
            )
            return
        length = (self.headers.get("Content-Length") or "").strip()
        if not length.isdigit():
            self._refuse(400, "Content-Length を付けてください。")
            return
        size = int(length)
        if size > DATA_MAX_BYTES:
            self._refuse(413, "データは 1 件 5GB までです。")
            return
        status, payload = self._forward_data(project, filename, size)
        self._send_json(status, payload)

    def _forward_data(self, project: str, filename: str, size: int) -> tuple[int, dict]:
        """本文を一時ファイルへ書いて sai-web へ送る。一時ファイルは返る前に消す（応答より先）。"""
        config = self.prep.config_provider()
        upload_dir = _upload_dir()
        upload_dir.mkdir(parents=True, exist_ok=True)
        with tempfile.NamedTemporaryFile(dir=upload_dir, prefix="data-", suffix=".part") as tmp:
            remaining = size
            while remaining > 0:
                chunk = self.rfile.read(min(DATA_CHUNK_BYTES, remaining))
                if not chunk:
                    break
                tmp.write(chunk)
                remaining -= len(chunk)
            if remaining:
                self.close_connection = True
                return 400, {"error": "本文が途中で切れました。"}
            tmp.flush()
            tmp.seek(0)
            try:
                result = self.prep.client_factory(config.server_url, config.token).post_data(
                    project, filename, tmp, size
                )
            except ClientError as error:
                # 同じ名前（409）と大きさ（413）は、理由を画面に出せるようにそのまま返す
                status = error.status if error.status in (409, 413) else 400
                return status, {"error": str(error)}
        return 201, result

    def _post_job(self, query: dict[str, list[str]]) -> None:
        content_type = self.headers.get("Content-Type", "").split(";")[0].strip().lower()
        if content_type not in ("application/pdf", "application/zip"):
            self._error(
                415,
                "PDF か zip を送ってください（Content-Type: application/pdf か application/zip）。",
            )
            return
        project = (query.get("project", [""])[0] or "").strip()
        if not project:
            self._error(400, "project を指定してください。")
            return
        body = self._read_body()
        translations = None
        if content_type == "application/zip":
            try:
                pdf_bytes, translations = _unpack_zip(body)
            except ValueError as error:
                self._error(400, str(error))
                return
        else:
            pdf_bytes = body
            if not pdf_bytes.startswith(b"%PDF-"):
                self._error(400, "PDF ではないファイルです（先頭が %PDF- ではありません）。")
                return
        paper_id_raw = query.get("paper_id", [""])[0].strip()
        bibtex_raw = self.headers.get("X-Sai-Bibtex", "").strip()
        if paper_id_raw:
            try:
                paper_id = int(paper_id_raw)
            except ValueError:
                self._error(400, "paper_id は整数で指定してください。")
                return
            filename = (query.get("filename", [""])[0] or "paper.pdf").strip()
            job = self.prep.pipeline.submit_paper(
                project, paper_id, pdf_bytes, filename, translations=translations
            )
        elif bibtex_raw:
            try:
                bibtex = base64.b64decode(bibtex_raw, validate=True).decode("utf-8")
            except (binascii.Error, UnicodeDecodeError):
                self._error(400, "BibTeX を読めません（base64 で送ってください）。")
                return
            if not bibtex.strip():
                self._error(400, "BibTeX が空です。")
                return
            if len(bibtex.encode("utf-8")) > MAX_BIBTEX_BYTES:
                self._error(400, "BibTeX が大きすぎます（64KB まで）。")
                return
            filename = (query.get("filename", [""])[0] or "paper.pdf").strip()
            job = self.prep.pipeline.submit_bibtex(
                project, bibtex, pdf_bytes, filename, translations=translations
            )
        else:
            self._error(400, "paper_id か BibTeX のどちらかが必要です。")
            return
        self._send_json(202, job.to_public())


def _unpack_zip(body: bytes) -> tuple[bytes, dict]:
    """agent が送る zip（C-U01）を読む。`paper.pdf` と、あれば翻訳の 2 つを返す。

    翻訳は `{"mono": …, "dual": …}`、無ければ `{}`。誤りは理由を入れた ValueError。
    """
    # 中身の展開（read）で壊れた deflate も 400 にする（zlib.error は read で出る）
    try:
        with zipfile.ZipFile(io.BytesIO(body)) as archive:
            names = set(archive.namelist())
            if "paper.pdf" not in names:
                raise ValueError("zip に paper.pdf がありません。")
            pdf_bytes = archive.read("paper.pdf")
            has_mono = "translated/paper-mono.pdf" in names
            has_dual = "translated/paper-dual.pdf" in names
            if has_mono != has_dual:
                raise ValueError("翻訳は mono と dual の両方を入れてください（片方だけはだめです）。")
            translations = {}
            if has_mono:
                translations = {
                    "mono": archive.read("translated/paper-mono.pdf"),
                    "dual": archive.read("translated/paper-dual.pdf"),
                }
    except (zipfile.BadZipFile, zlib.error) as error:
        raise ValueError("zip として読めません。") from error
    for data in [pdf_bytes, *translations.values()]:
        if not data.startswith(b"%PDF-"):
            raise ValueError("zip の中の PDF ではないファイルがあります（先頭が %PDF- ではありません）。")
    return pdf_bytes, translations


def create_server(
    config_provider: Callable[[], config_mod.Config],
    pipeline: Pipeline,
    host: str = "127.0.0.1",
    port: int = 8765,
    client_factory: Callable[[str, str], Client] = Client,
) -> PrepHTTPServer:
    """サーバを作る（`serve_forever` は呼び出し側）。"""
    return PrepHTTPServer((host, port), pipeline, config_provider, client_factory)


def serve(
    config_provider: Callable[[], config_mod.Config],
    pipeline: Pipeline,
    host: str = "127.0.0.1",
    port: int = 8765,
    client_factory: Callable[[str, str], Client] = Client,
) -> PrepHTTPServer:
    """サーバを作って待ち受ける（`serve_forever`）。"""
    server = create_server(config_provider, pipeline, host, port, client_factory)
    try:
        server.serve_forever()
    finally:
        server.server_close()
    return server
