"""拡張（extension）の住所: 一覧・画面の配信・index.html への差し込み・CLI と service の中継。

- 拡張の一覧は `agent-link ext list --json` から得る（Web は extension.json を読み直さない）。10 秒覚える。
- 画面に出す拡張は `visible`。タブ・style・script は `render_index` で `index.html` に差し込む。
- 拡張の CLI は `agent-link ext run <name> -- <args…>` を呼ぶ（`run_cli`）。
- service は設定の宛先へ中継する（`relay`）。リダイレクトは追わない。

検査（値の形の誤り 400・有効でない / web に無い 403・無い 404）は CLI を呼ぶ前に行う。
"""

from __future__ import annotations

import base64
import html
import http.client
import logging
import re
import time
import urllib.error
import urllib.request
from pathlib import Path

from fastapi import HTTPException
from fastapi.responses import Response

from agent_link_web import control, registry
from agent_link_web.cli import run_json
from agent_link_web.config import settings

logger = logging.getLogger("agent_link_web")

# 拡張の一覧は 10 秒だけ覚える（`/` を開くたびに CLI を起こさないため）
_LIST_TTL = 10.0
# キーは空文字 1 つ → (時刻, 一覧)。テストから消すため clear_cache() がある
_cache: dict[str, tuple[float, list[dict]]] = {}

_NAME_RE = re.compile(r"^[a-z][a-z0-9-]{0,31}$")
# `ext run` の時間の上限は CLI 側が 120 秒。Web はそれより長く待つ
_CLI_TIMEOUT = 130
_SVC_TIMEOUT = 15
_SVC_MAX_RESPONSE = 5 * 1024 * 1024
_SVC_MAX_BODY = 1024 * 1024

# 目印の行（static/index.html）。行ごと置き換える（目印の無い版には何もしない）
_MARK_RE = re.compile(r"^([ \t]*)<!-- ext:(styles|tabs|scripts) -->[ \t]*\n", re.MULTILINE)


class _NoRedirect(urllib.request.HTTPRedirectHandler):
    """リダイレクトを追わない（3xx はそのまま HTTPError になり 502）。設定した宛先の外へ要求を出さないため。"""

    def redirect_request(self, req, fp, code, msg, headers, newurl):
        return None


_opener = urllib.request.build_opener(_NoRedirect)


def clear_cache() -> None:
    """覚えた一覧を消す（テスト用）。"""
    _cache.clear()


def list_extensions() -> list[dict]:
    """`agent-link ext list --json` の拡張の一覧（10 秒覚える）。失敗したら空（base の画面は出す）。"""
    now = time.monotonic()
    hit = _cache.get("")
    if hit and now - hit[0] < _LIST_TTL:
        return hit[1]
    try:
        items = run_json(
            [settings.agent_link_bin, "ext", "list", "--json"],
            app="agent-link",
            runner=control.run_cli,
        )["extensions"]
    except Exception as e:  # 拡張が壊れても base の画面は出す
        logger.warning("拡張の一覧を読めませんでした: %s", e)
        items = []
    _cache[""] = (now, items)
    return items


def _find(name: str) -> dict | None:
    return next((ext for ext in list_extensions() if ext.get("name") == name), None)


def _ui_title(ext: dict) -> str:
    return (ext.get("ui") or {}).get("title") or ext["title"]


def _esc(value) -> str:
    return html.escape(str(value), quote=True)


def visible(ext: dict) -> bool:
    """画面に出す拡張か（画面があり・どこかのプロジェクトで有効で・宛先があれば ready）。"""
    if not ext.get("ui") or not ext.get("enabled_in"):
        return False
    service = ext.get("service")
    return service is None or bool(service.get("ready"))


def api_list() -> dict:
    """`GET /api/ext`。入っている拡張の一覧（画面・CLI・service の有無と状態）。"""
    out = []
    for ext in list_extensions():
        service = ext.get("service")
        out.append(
            {
                "name": ext["name"],
                "title": _ui_title(ext),
                "icon": (ext.get("ui") or {}).get("icon"),
                "ui": bool(ext.get("ui")),
                "service": service is not None,
                "ready": service is None or bool(service.get("ready")),
                "web": (ext.get("cli") or {}).get("web", []),
                "enabled_in": ext.get("enabled_in", []),
            }
        )
    return {"extensions": out}


def _asset_url(ext: dict, file: str) -> str:
    # 版（version）を付けるので、拡張を入れ替えたら新しい物を取らせる
    return f"/ext/{ext['name']}/{file}?v={ext.get('version') or '0'}"


def _tab_button(ext: dict) -> str:
    name, title = _esc(ext["name"]), _ui_title(ext)
    icon = (ext.get("ui") or {}).get("icon")
    if icon:
        mark = f'<img src="{_esc("/ext/" + ext["name"] + "/" + icon)}" alt="" aria-hidden="true">'
    else:
        mark = f'<span class="ext-tab-letter" aria-hidden="true">{_esc(title[:1])}</span>'
    return (
        f'<button data-tab="ext-{name}" data-first="#/ext/{name}" '
        f'aria-label="{_esc(title)}">{mark}<span>{_esc(title)}</span></button>'
    )


def render_index(text: str) -> str:
    """`static/index.html` の目印の行を、画面に出す拡張の物に置き換える（無ければ目印の行を消すだけ）。"""
    shown = [ext for ext in list_extensions() if visible(ext)]
    fragments = {
        "styles": [
            f'<link rel="stylesheet" href="{_esc(_asset_url(ext, style))}" />'
            for ext in shown
            for style in (ext["ui"].get("styles") or [])
        ],
        "tabs": [_tab_button(ext) for ext in shown],
        "scripts": [
            f'<script src="{_esc(_asset_url(ext, script))}"></script>'
            for ext in shown
            for script in (ext["ui"].get("scripts") or [])
        ],
    }

    def replace(match: re.Match) -> str:
        indent, kind = match.group(1), match.group(2)
        return "".join(f"{indent}{line}\n" for line in fragments[kind])

    return _MARK_RE.sub(replace, text)


def _safe_rel(path: str) -> bool:
    """`..`・空の区切り・先頭の `/`・`\\` を含まない相対の道か。"""
    return "\\" not in path and all(part not in ("", "..") for part in path.split("/"))


def ui_file(name: str, path: str) -> Path:
    """`/ext/<name>/<path>` の実体のパス。`ui/` の実体の外（symlink の先を含む）なら 404。"""
    ext = _find(name)
    if not ext or not ext.get("ui") or not _NAME_RE.match(name) or not _safe_rel(path):
        raise HTTPException(status_code=404, detail="Not found.")
    ui_dir = (Path(ext["path"]) / "ui").resolve()
    try:
        target = (ui_dir / path).resolve(strict=True)
    except (OSError, ValueError):  # 無い・NUL 文字を含む
        target = None
    if target is None or not target.is_file() or not target.is_relative_to(ui_dir):
        raise HTTPException(status_code=404, detail="Not found.")
    return target


def _check_args(payload: dict) -> list[str]:
    args = payload.get("args")
    if not isinstance(args, list) or not 1 <= len(args) <= 32:
        raise HTTPException(status_code=400, detail="args must be a list of 1 to 32 strings.")
    for arg in args:
        if (
            not isinstance(arg, str)
            or not 1 <= len(arg) <= 1000
            or any(c in arg for c in "\x00\r\n")
        ):
            raise HTTPException(
                status_code=400,
                detail="Each arg must be a string of 1 to 1000 characters without line breaks.",
            )
    return args


def run_cli(project: str, ext_name: str, payload: dict) -> dict:
    """拡張の CLI（`ext run <e> -- <args…>`）を、検査を通した物だけ呼ぶ。"""
    proj = control.find_project(project)  # 登録簿に無ければ 404
    if not proj.get("root_exists"):
        raise HTTPException(status_code=404, detail="Project folder not found.")
    if not _NAME_RE.match(ext_name):
        raise HTTPException(status_code=400, detail="Invalid extension name.")
    ext = _find(ext_name)
    if not ext or project not in ext.get("enabled_in", []):
        raise HTTPException(status_code=403, detail="Extension is not enabled in this project.")
    args = _check_args(payload)
    if args[0] not in (ext.get("cli") or {}).get("web", []):
        raise HTTPException(status_code=403, detail="This command is not allowed from the web.")
    return control._cli_json(
        ["ext", "run", ext_name, "--", *args], root=proj["root"], timeout=_CLI_TIMEOUT
    )


def relay(ext_name: str, path: str, method: str, query: str, body: bytes | None) -> Response:
    """拡張の service の宛先へ中継する（GET はどの道でも、POST は `service.post` の道だけ）。"""
    ext = _find(ext_name)
    service = (ext or {}).get("service")
    base = registry.config_var(service["url_env"]) if service else None
    if not base or not base.startswith(("http://", "https://")):
        raise HTTPException(status_code=404, detail="Extension service is not configured.")
    if not _safe_rel(path):
        raise HTTPException(status_code=400, detail="Invalid path.")
    headers = {}
    if method == "POST":
        if path not in (service.get("post") or []):
            raise HTTPException(status_code=403, detail="This path is not allowed for POST.")
        if body is not None and len(body) > _SVC_MAX_BODY:
            raise HTTPException(status_code=413, detail="Request body is too large.")
        headers["Content-Type"] = "application/json"
    if service.get("user_env") and service.get("password_env"):
        user = registry.config_var(service["user_env"])
        password = registry.config_var(service["password_env"])
        if user and password:
            token = base64.b64encode(f"{user}:{password}".encode()).decode()
            headers["Authorization"] = f"Basic {token}"

    url = f"{base.rstrip('/')}/{path}" + (f"?{query}" if query else "")
    req = urllib.request.Request(
        url, data=body if method == "POST" else None, headers=headers, method=method
    )
    try:
        with _opener.open(req, timeout=_SVC_TIMEOUT) as resp:
            data = resp.read(_SVC_MAX_RESPONSE + 1)
            status = resp.status
            ctype = resp.headers.get("Content-Type", "application/octet-stream")
    except urllib.error.HTTPError as e:  # 3xx（リダイレクトは追わない）と 4xx・5xx
        raise HTTPException(status_code=502, detail=f"Extension service returned {e.code}.")
    except (urllib.error.URLError, http.client.HTTPException, OSError, ValueError):
        raise HTTPException(status_code=502, detail="Could not reach the extension service.")
    if len(data) > _SVC_MAX_RESPONSE:
        raise HTTPException(status_code=502, detail="Extension service response is too large.")
    return Response(content=data, status_code=status, headers={"Content-Type": ctype})
