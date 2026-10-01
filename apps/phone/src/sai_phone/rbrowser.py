"""リモートブラウザ（Mac の Chrome を iPhone から操作する・U01）。

外出先では電子ジャーナルを開けないので、sai-moon が動く Mac（学内の回線）の Chrome を
遠隔で操作して PDF を保存する。

- Chrome は全体で 1 つだけ（プロジェクトごとではない）。専用プロファイルの headless Chrome。
- Chrome DevTools Protocol を `--remote-debugging-pipe` で話す（標準ライブラリだけ。
  websocket は使わない）。子の fd 3 から読み fd 4 へ書き、メッセージは JSON + `\\0`。
- 画面は `Page.captureScreenshot` の JPEG を iPhone が取りに来る。タップ・スクロール・
  文字は検査してから `Input.*` に変える。
- 最後の操作から `IDLE_SECONDS` 経つと Chrome を終える。
- ダウンロードの完了はディレクトリを見て判定する（CDP のイベントは使わない）。
"""

from __future__ import annotations

import asyncio
import concurrent.futures
import datetime
import fcntl
import json
import logging
import math
import os
import re
import subprocess
import threading
import time
import urllib.parse
from pathlib import Path

from fastapi import HTTPException

from sai_phone import registry

logger = logging.getLogger("sai_phone")

# 画面の大きさ（CSS px）。iPhone に出す JPEG もこの大きさ
VIEWPORT = (600, 1000)
# 最後の操作からこの秒数が経つと Chrome を終える（確認は 30 秒ごと）
IDLE_SECONDS = 600
# 画面の JPEG の質
JPEG_QUALITY = 60
# ダウンロード先のファイルを残す日数（open_page のたびに古い物を消す）
DOWNLOAD_KEEP_DAYS = 7
# Mac の Chrome の既定の場所
DEFAULT_CHROME = "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"
# "HeadlessChrome" を含めない通常の UA（出版社に弾かれにくくする）
USER_AGENT = (
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 "
    "(KHTML, like Gecko) Chrome/154.0.0.0 Safari/537.36"
)
IDLE_CHECK_SECONDS = 30
DOI_RE = re.compile(r"10\.\d{4,9}/[^\s]{1,200}")
TEXT_MAX = 200
SCROLL_MAX = 3000
# キー名 → (code, windowsVirtualKeyCode)
KEYS = {
    "Enter": ("Enter", 13),
    "Backspace": ("Backspace", 8),
    "Tab": ("Tab", 9),
    "Escape": ("Escape", 27),
}

_proc: subprocess.Popen | None = None
_rfd = -1  # Chrome からの読み端
_wfd = -1  # Chrome への書き端
_pending: dict[int, concurrent.futures.Future] = {}
_next_id = 0
_lock = threading.Lock()  # _pending / _next_id / 書き込み
_start_lock: asyncio.Lock | None = None
_session = ""
_target_id = ""
_last_op = 0.0
_idle_task: asyncio.Task | None = None


# --- 置き場 ---


def profile_dir() -> Path:
    return registry.sai_home() / "phone" / "browser-profile"


def download_dir() -> Path:
    return registry.sai_home() / "phone" / "browser-downloads"


# --- 入力の検査（Chrome を触る前に行う） ---


def _bad(detail: str) -> HTTPException:
    return HTTPException(status_code=400, detail=detail)


def check_doi(doi) -> str:
    if not isinstance(doi, str) or not DOI_RE.fullmatch(doi):
        raise _bad("DOI が正しくありません")
    return doi


def _is_num(v) -> bool:
    return isinstance(v, (int, float)) and not isinstance(v, bool)


def check_input(payload) -> dict:
    """操作の body を確かめて返す（違反は 400）。形は 7 通りだけ。"""
    if not isinstance(payload, dict):
        raise _bad("操作が正しくありません")
    kind = payload.get("type")
    keys = set(payload) - {"type"}
    if kind == "tap" and keys == {"x", "y"}:
        x, y = payload["x"], payload["y"]
        if _is_num(x) and _is_num(y) and 0 <= x <= 1 and 0 <= y <= 1 and not (
            math.isnan(x) or math.isnan(y)
        ):
            return payload
    elif kind == "scroll" and keys == {"dy"}:
        dy = payload["dy"]
        if isinstance(dy, int) and not isinstance(dy, bool) and -SCROLL_MAX <= dy <= SCROLL_MAX:
            return payload
    elif kind == "text" and keys == {"text"}:
        text = payload["text"]
        if (
            isinstance(text, str)
            and 1 <= len(text) <= TEXT_MAX
            and not any(ord(c) < 32 or ord(c) == 127 for c in text)
        ):
            return payload
    elif kind == "key" and keys == {"key"}:
        if isinstance(payload["key"], str) and payload["key"] in KEYS:
            return payload
    elif kind in ("back", "reload") and not keys:
        return payload
    raise _bad("操作が正しくありません")


# --- CDP（pipe） ---


def _reader(rfd: int) -> None:
    """Chrome の出力を `\\0` 区切りに切り、id 付きの応答を待ち手へ渡す（イベントは捨てる）。"""
    buf = b""
    while True:
        try:
            chunk = os.read(rfd, 65536)
        except OSError:
            chunk = b""
        if not chunk:
            break
        buf += chunk
        while b"\0" in buf:
            raw, buf = buf.split(b"\0", 1)
            try:
                msg = json.loads(raw.decode("utf-8", "replace"))
            except ValueError:
                continue
            if not isinstance(msg, dict) or "id" not in msg:
                continue
            with _lock:
                fut = _pending.pop(msg["id"], None)
            if fut is not None and not fut.done():
                fut.set_result(msg)
    with _lock:  # Chrome が終わった: 待っている呼び出しはすべて失敗にする
        futs = list(_pending.values())
        _pending.clear()
    for fut in futs:
        if not fut.done():
            fut.set_exception(HTTPException(status_code=502, detail="Chrome が終了しました"))


async def call(method: str, params: dict | None = None, session_id: str | None = None,
               timeout: float = 15):
    """CDP を 1 つ呼んで `result` を返す（応答の error は 502）。"""
    global _next_id
    fut: concurrent.futures.Future = concurrent.futures.Future()
    msg = {"method": method, "params": params or {}}
    if session_id:
        msg["sessionId"] = session_id
    with _lock:
        _next_id += 1
        msg["id"] = mid = _next_id
        _pending[mid] = fut
        try:
            os.write(_wfd, json.dumps(msg).encode("utf-8") + b"\0")
        except OSError:
            _pending.pop(mid, None)
            raise HTTPException(status_code=502, detail="Chrome に送れません")
    try:
        resp = await asyncio.wait_for(asyncio.wrap_future(fut), timeout)
    except asyncio.TimeoutError:
        with _lock:
            _pending.pop(mid, None)
        raise HTTPException(status_code=502, detail="Chrome の応答がありません")
    if "error" in resp:
        err = resp["error"]
        text = err.get("message") if isinstance(err, dict) else str(err)
        raise HTTPException(status_code=502, detail=f"Chrome がエラーを返しました: {text}")
    return resp.get("result") or {}


async def _page(method: str, params: dict | None = None, timeout: float = 15):
    return await call(method, params, _session, timeout)


# --- 起動と終了 ---


def _alive() -> bool:
    return _proc is not None and _proc.poll() is None


def _chrome_path() -> str:
    path = os.environ.get("SAI_PHONE_CHROME") or DEFAULT_CHROME
    if not os.path.isfile(path):
        raise HTTPException(status_code=503, detail="Chrome が見つかりません")
    return path


def _write_preferences(profile: Path, dl: Path) -> None:
    """PDF を Chrome の中で開かず、必ずダウンロードに回す設定を書く。"""
    path = profile / "Default" / "Preferences"
    path.parent.mkdir(parents=True, exist_ok=True)
    try:
        prefs = json.loads(path.read_text("utf-8"))
        if not isinstance(prefs, dict):
            prefs = {}
    except (OSError, ValueError):
        prefs = {}
    prefs.setdefault("plugins", {})["always_open_pdf_externally"] = True
    download = prefs.setdefault("download", {})
    download["prompt_for_download"] = False
    download["default_directory"] = str(dl)
    path.write_text(json.dumps(prefs), "utf-8")


def _cleanup_downloads() -> None:
    limit = time.time() - DOWNLOAD_KEEP_DAYS * 86400
    for p in download_dir().iterdir():
        try:
            if p.is_file() and p.stat().st_mtime < limit:
                p.unlink()
        except OSError:
            pass


def _high_fd(fd: int) -> int:
    """fd 3・4 と取り合わないよう、10 以上の番号へ写す。"""
    new = fcntl.fcntl(fd, fcntl.F_DUPFD, 10)
    os.close(fd)
    return new


async def _start() -> None:
    global _proc, _rfd, _wfd, _session, _target_id
    chrome = _chrome_path()
    profile, dl = profile_dir(), download_dir()
    profile.mkdir(parents=True, exist_ok=True)
    dl.mkdir(parents=True, exist_ok=True)
    _write_preferences(profile, dl)
    args = [chrome, "--remote-debugging-pipe", f"--user-data-dir={profile}"]
    if os.environ.get("SAI_PHONE_BROWSER_HEADED") != "1":
        args.append("--headless=new")
    args += [
        "--no-first-run",
        "--no-default-browser-check",
        "--disable-features=TranslateUI",
        f"--user-agent={USER_AGENT}",
        "about:blank",
    ]
    to_child_r, to_child_w = (_high_fd(f) for f in os.pipe())
    from_child_r, from_child_w = (_high_fd(f) for f in os.pipe())

    def child_setup() -> None:
        os.dup2(to_child_r, 3)
        os.dup2(from_child_w, 4)

    try:
        proc = subprocess.Popen(
            args,
            stdin=subprocess.DEVNULL,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            preexec_fn=child_setup,
            close_fds=False,  # dup2 した 3・4 を残す（他の fd は CLOEXEC で閉じる）
        )
    except OSError as e:
        for f in (to_child_r, to_child_w, from_child_r, from_child_w):
            os.close(f)
        raise HTTPException(status_code=503, detail=f"Chrome を起動できません: {e}")
    os.close(to_child_r)
    os.close(from_child_w)
    _proc, _rfd, _wfd = proc, from_child_r, to_child_w
    threading.Thread(target=_reader, args=(from_child_r,), daemon=True).start()
    try:
        targets = (await call("Target.getTargets")).get("targetInfos") or []
        page = next((t for t in targets if t.get("type") == "page"), None)
        if page is None:
            made = await call("Target.createTarget", {"url": "about:blank"})
            _target_id = made.get("targetId", "")
        else:
            _target_id = page.get("targetId", "")
        attached = await call("Target.attachToTarget", {"targetId": _target_id, "flatten": True})
        _session = attached.get("sessionId", "")
        w, h = VIEWPORT
        await _page(
            "Emulation.setDeviceMetricsOverride",
            {"width": w, "height": h, "deviceScaleFactor": 1, "mobile": False},
        )
        await call("Browser.setDownloadBehavior", {"behavior": "allow", "downloadPath": str(dl)})
    except BaseException:
        await close()
        raise


async def _ensure() -> None:
    """Chrome が動いていなければ起動する（動いていれば何もしない）。"""
    global _start_lock
    if _start_lock is None:
        _start_lock = asyncio.Lock()
    async with _start_lock:
        if _alive():
            return
        await _reset()
        await _start()


def _stop_proc(proc: subprocess.Popen) -> None:
    for step in (proc.terminate, proc.kill):
        try:
            proc.wait(timeout=5)
            return
        except subprocess.TimeoutExpired:
            step()
    proc.wait()


async def _reset() -> None:
    """動いていない Chrome の後始末（fd と状態を戻す）。"""
    global _proc, _rfd, _wfd, _session, _target_id
    for fd in (_rfd, _wfd):
        if fd >= 0:
            try:
                os.close(fd)
            except OSError:
                pass
    _proc, _rfd, _wfd, _session, _target_id = None, -1, -1, "", ""


async def close() -> None:
    """Chrome を終える（ダウンロード先のファイルは消さない）。"""
    global _idle_task
    task, _idle_task = _idle_task, None
    if task is not None and task is not asyncio.current_task():
        task.cancel()
    proc = _proc
    if proc is not None and proc.poll() is None:
        try:
            await call("Browser.close", timeout=3)
        except HTTPException:
            pass
        await asyncio.get_running_loop().run_in_executor(None, _stop_proc, proc)
    await _reset()


async def _idle_loop() -> None:
    while True:
        await asyncio.sleep(IDLE_CHECK_SECONDS)
        if not _alive():
            return
        if time.monotonic() - _last_op > IDLE_SECONDS:
            await close()
            return


def _touch() -> None:
    global _last_op
    _last_op = time.monotonic()


# --- 公開の関数 ---


async def open_page(doi) -> dict:
    """DOI のページを開く（Chrome が無ければ起動する）。"""
    global _idle_task
    doi = check_doi(doi)
    await _ensure()
    _touch()
    if _idle_task is None or _idle_task.done():
        _idle_task = asyncio.create_task(_idle_loop())
    _cleanup_downloads()
    url = "https://doi.org/" + urllib.parse.quote(doi, safe="/()<>:;,.-_~")
    await _page("Page.navigate", {"url": url})
    return await state()


async def state() -> dict:
    w, h = VIEWPORT
    out = {"open": False, "url": "", "title": "", "width": w, "height": h,
           "downloads": list_downloads()}
    if _alive():
        info = (await call("Target.getTargetInfo", {"targetId": _target_id})).get("targetInfo") or {}
        out.update(open=True, url=str(info.get("url", "")), title=str(info.get("title", "")))
    return out


def _require_open() -> None:
    if not _alive():
        raise HTTPException(status_code=409, detail="ブラウザは開いていません")


async def screenshot() -> bytes:
    import base64

    _require_open()
    res = await _page("Page.captureScreenshot", {"format": "jpeg", "quality": JPEG_QUALITY})
    return base64.b64decode(res.get("data", ""))


async def send_input(payload) -> None:
    """タップ・スクロール・文字・キー・戻る・再読込を Chrome へ送る（先に検査する）。"""
    p = check_input(payload)
    _require_open()
    _touch()
    kind = p["type"]
    w, h = VIEWPORT
    if kind == "tap":
        x, y = p["x"] * w, p["y"] * h
        await _page("Input.dispatchMouseEvent", {"type": "mouseMoved", "x": x, "y": y})
        for t in ("mousePressed", "mouseReleased"):
            await _page("Input.dispatchMouseEvent",
                        {"type": t, "x": x, "y": y, "button": "left", "clickCount": 1})
    elif kind == "scroll":
        await _page("Input.dispatchMouseEvent", {"type": "mouseWheel", "x": w // 2, "y": h // 2,
                                                 "deltaX": 0, "deltaY": p["dy"]})
    elif kind == "text":
        await _page("Input.insertText", {"text": p["text"]})
    elif kind == "key":
        code, vk = KEYS[p["key"]]
        ev = {"key": p["key"], "code": code, "windowsVirtualKeyCode": vk}
        if p["key"] == "Enter":
            ev["text"] = "\r"
        await _page("Input.dispatchKeyEvent", {"type": "keyDown", **ev})
        await _page("Input.dispatchKeyEvent", {"type": "keyUp", **ev})
    elif kind == "back":
        hist = await _page("Page.getNavigationHistory")
        idx = hist.get("currentIndex", 0) - 1
        entries = hist.get("entries") or []
        if 0 <= idx < len(entries):
            await _page("Page.navigateToHistoryEntry", {"entryId": entries[idx]["id"]})
    else:
        await _page("Page.reload")


def list_downloads() -> list[dict]:
    """ダウンロード先の直下のファイル（新しい順）。`.crdownload` は done=false で元の名前に。"""
    found: dict[str, dict] = {}
    try:
        files = list(download_dir().iterdir())
    except OSError:
        return []
    for p in files:
        if p.name.startswith(".") or not p.is_file():
            continue
        try:
            st = p.stat()
        except OSError:
            continue
        partial = p.name.endswith(".crdownload")
        name = p.name[: -len(".crdownload")] if partial else p.name
        if not name:
            continue
        mtime = datetime.datetime.fromtimestamp(st.st_mtime, datetime.timezone.utc)
        item = {"name": name, "size": st.st_size, "done": not partial,
                "modified_at": mtime.isoformat()}
        if partial or name not in found:  # 途中の物があれば途中として出す
            found[name] = item
    return sorted(found.values(), key=lambda d: d["modified_at"], reverse=True)


def download_path(name) -> Path:
    """ダウンロード済みの PDF のパスを返す（パスは受け取らず名前だけ。404 / 409）。"""
    from sai_phone import papers

    papers._validate_pdf_name(name)
    base = download_dir()
    path = base / name
    if (base / (name + ".crdownload")).exists():
        raise HTTPException(status_code=409, detail="ダウンロード中です")
    if not path.is_file() or path.resolve().parent != base.resolve():
        raise HTTPException(status_code=404, detail="PDF が見つかりません")
    return path
