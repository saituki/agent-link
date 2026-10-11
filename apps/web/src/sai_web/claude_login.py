"""利用者の Claude のログイン（実装計画 U10・共有契約 C-LOGIN）。

利用者ごとに 1 つ作り、その人の全コンテナの `$HOME/.claude` で共有する（置き場は
`containers.ensure_claude_dir`）。Web は端末の画面を持たないので、補助のコンテナの中で
`claude auth login` を pty で動かし、URL とコードを中継する。

- `start(user)`: 補助のコンテナを起こし、ログインの URL を返す。
- `submit(user, code)`: 貼られたコードを端末へ送り、`Login successful.` を待つ。
- `status(user)`: `claude auth status` の結果（ログイン中か・認証の方法）。
- `cancel(user)`: 進行中のログインを止める。

決め事:
- 標準ライブラリだけ（`pty`・`subprocess`）。docker は `containers.docker_bin()` を使う。
- 1 人に 1 つまで。10 分で切れたら補助のコンテナを止める。
- コードと token の値は、ログ・画面・返り値に書かない（失敗の理由に出す行からは伏せる）。
"""

from __future__ import annotations

import codecs
import fcntl
import json
import logging
import os
import pty
import re
import struct
import subprocess
import termios
import threading
import time

from fastapi import HTTPException

from sai_web import containers

logger = logging.getLogger("sai_web")

# 補助のコンテナの中の HOME（共有のフォルダを `<home>/.claude` に mount する）
_LOGIN_HOME = "/home/login"
_SESSION_TTL = 600.0  # 秒。切れたら補助のコンテナを止める
_URL_WAIT = 30.0  # 秒。URL が出るまで待つ
_RESULT_WAIT = 60.0  # 秒。コードの結果を待つ
_STATUS_TIMEOUT = 30.0  # 秒
_OUTPUT_KEEP = 65536  # 文字。読んだ出力はこの分だけ残す
_CODE_RE = re.compile(r"^[\x21-\x7e]{1,500}$")
# 端末の制御文字（CSI と OSC。OSC 8 の hyperlink を含む）
_ANSI_RE = re.compile(r"\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)|\x1b\[[0-9;?]*[A-Za-z]")
# 次の "https://" の手前で切る（OSC 8 で同じ URL が続けて出ても 1 つにする）
_URL_RE = re.compile(r"https://(?:(?!https://)\S)+?/oauth/(?:(?!https://)\S)+")
_SUCCESS = "Login successful."
_FAILURES = ("Login interrupted", "timed out", "error", "Invalid code")


def _container_name(user: str) -> str:
    return f"sai-login-{user}"


def _clean(raw: str) -> str:
    """端末の制御文字と CR を除く。"""
    return _ANSI_RE.sub("", raw).replace("\r", "")


def _last_line(text: str, code: str = "") -> str:
    """出力の最後の空でない行（失敗の理由）。貼ったコードは伏せる。"""
    lines = [line.strip() for line in text.splitlines() if line.strip()]
    line = lines[-1] if lines else ""
    if code:
        line = line.replace(code, "（コード）")
    return line[:200]


def _find_url(text: str) -> str | None:
    """ログインの URL。改行の後ろまで読めたものだけ返す（途中で切れた URL を返さない）。"""
    for match in _URL_RE.finditer(text):
        if "claude" in match.group(0) and match.end() < len(text):
            return match.group(0)
    return None


def _docker_rm(name: str) -> None:
    """補助のコンテナを消す（無ければ何もしない）。"""
    try:
        subprocess.run(
            [containers.docker_bin(), "rm", "-f", name],
            capture_output=True,
            text=True,
            timeout=20.0,
        )
    except (OSError, subprocess.TimeoutExpired):
        logger.warning("補助のコンテナを消せません: %s", name)


class _Session:
    """補助のコンテナ 1 つ（`docker run -t` の端末と、読み取りの係）。"""

    def __init__(self, user: str, proc: subprocess.Popen, master: int) -> None:
        self.user = user
        self.name = _container_name(user)
        self.proc = proc
        self.master = master
        self.lock = threading.Lock()
        self.buf = ""
        self.timer: threading.Timer | None = None
        self._closed = False
        self._decoder = codecs.getincrementaldecoder("utf-8")(errors="replace")
        self._reader = threading.Thread(target=self._read, daemon=True)
        self._reader.start()

    def _read(self) -> None:
        while True:
            try:
                data = os.read(self.master, 4096)
            except OSError:
                break
            if not data:
                break
            text = self._decoder.decode(data)
            with self.lock:
                self.buf = (self.buf + text)[-_OUTPUT_KEEP:]

    def wait_output(self) -> None:
        """端末が閉じた後は、読み残しを待つ（終わりの文字を取りこぼさないため）。"""
        self._reader.join(timeout=2.0)

    def text(self) -> str:
        """今までの出力（制御文字を除いたもの）。"""
        with self.lock:
            raw = self.buf
        return _clean(raw)

    def clear(self) -> None:
        with self.lock:
            self.buf = ""

    def write(self, text: str) -> None:
        try:
            os.write(self.master, text.encode("utf-8"))
        except OSError:
            raise HTTPException(status_code=502, detail="ログインの中継が切れました")

    def close(self) -> None:
        """補助のコンテナを止めて、端末を閉じる（2 回目以降は何もしない）。"""
        if self._closed:
            return
        self._closed = True
        if self.timer is not None:
            self.timer.cancel()
        if self.proc.poll() is None:
            self.proc.terminate()
            try:
                self.proc.wait(timeout=5.0)
            except subprocess.TimeoutExpired:
                self.proc.kill()
                self.proc.wait()
            # docker の CLI を止めても、コンテナは残るので消す
            _docker_rm(self.name)
        self._reader.join(timeout=5.0)
        try:
            os.close(self.master)
        except OSError:
            pass


_lock = threading.Lock()
_sessions: dict[str, _Session] = {}


def _take(user: str, expected: _Session | None = None) -> _Session | None:
    """利用者の進行中のログインを外す。`expected` が違えば何もしない（期限切れの係の用）。"""
    with _lock:
        session = _sessions.get(user)
        if session is None or (expected is not None and session is not expected):
            return None
        del _sessions[user]
        return session


def _expire(user: str, session: _Session) -> None:
    taken = _take(user, session)
    if taken is not None:
        taken.close()


def start(user: str) -> dict:
    """ログインを始める。ログインの URL を返す（30 秒まで待つ）。"""
    claude_dir = containers.ensure_claude_dir(user)
    cancel(user)
    # 前の回の補助のコンテナが残っていたら消す（名前が重ならないように）
    _docker_rm(_container_name(user))
    args = [
        containers.docker_bin(),
        "run",
        "--rm",
        "-i",
        "-t",
        "--name",
        _container_name(user),
        "-v",
        f"{claude_dir}:{_LOGIN_HOME}/.claude",
        "-e",
        f"HOME={_LOGIN_HOME}",
        "-e",
        "COLUMNS=1000",
        containers.image_name(),
        "claude",
        "auth",
        "login",
    ]
    master, slave = pty.openpty()
    try:
        # 端末を広くして、URL が折り返されないようにする
        fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 50, 1000, 0, 0))
        proc = subprocess.Popen(args, stdin=slave, stdout=slave, stderr=slave, close_fds=True)
    except (OSError, ValueError):
        os.close(master)
        os.close(slave)
        raise HTTPException(status_code=503, detail="docker が動いていません")
    os.close(slave)

    session = _Session(user, proc, master)
    deadline = time.monotonic() + _URL_WAIT
    url = None
    while time.monotonic() < deadline:
        url = _find_url(session.text())
        if url or proc.poll() is not None:
            break
        time.sleep(0.2)
    if url is None and proc.poll() is not None:
        session.wait_output()
        url = _find_url(session.text())
    if url is None:
        reason = _last_line(session.text())
        session.close()
        raise HTTPException(
            status_code=502,
            detail=f"ログインの URL を取れませんでした: {reason}" if reason else "ログインの URL を取れませんでした",
        )

    with _lock:
        previous = _sessions.get(user)
        _sessions[user] = session
    if previous is not None:
        previous.close()
    session.timer = threading.Timer(_SESSION_TTL, _expire, args=(user, session))
    session.timer.daemon = True
    session.timer.start()
    return {"url": url}


def submit(user: str, code: object) -> dict:
    """貼られたコードを端末へ送り、ログインの結果を待つ（60 秒まで）。成功なら {"ok": True}。"""
    if not isinstance(code, str) or not _CODE_RE.match(code):
        raise HTTPException(
            status_code=400,
            detail="コードは空白を含まない印字できる文字で、500 文字までにしてください",
        )
    with _lock:
        session = _sessions.get(user)
    if session is None:
        raise HTTPException(status_code=409, detail="先にログインを始めてください")

    session.clear()
    session.write(code + "\r")
    deadline = time.monotonic() + _RESULT_WAIT
    while True:
        exited = session.proc.poll() is not None
        if exited:
            session.wait_output()
        text = session.text()
        if _SUCCESS in text:
            _finish(user, session)
            return {"ok": True}
        lowered = text.lower()
        for marker in _FAILURES:
            if marker.lower() in lowered:
                reason = _failure_line(text, marker, code)
                _finish(user, session)
                raise HTTPException(status_code=502, detail=f"ログインに失敗しました: {reason}")
        if exited:
            reason = _last_line(text, code)
            _finish(user, session)
            raise HTTPException(status_code=502, detail=f"ログインに失敗しました: {reason}")
        if time.monotonic() >= deadline:
            _finish(user, session)
            raise HTTPException(status_code=502, detail="ログインが時間内に終わりませんでした")
        time.sleep(0.2)


def _finish(user: str, session: _Session) -> None:
    """この回のログインを終える（登録を外して、補助のコンテナを止める）。"""
    _take(user, session)
    session.close()


def _failure_line(text: str, marker: str, code: str) -> str:
    """失敗の文字を含む行（貼ったコードは伏せる）。"""
    for line in text.splitlines():
        if marker.lower() in line.lower():
            return line.strip().replace(code, "（コード）")[:200]
    return _last_line(text, code)


def status(user: str) -> dict:
    """`claude auth status` の結果。`logged_in` と `auth_method` だけを返す（中身は返さない）。"""
    claude_dir = containers.ensure_claude_dir(user)
    args = [
        containers.docker_bin(),
        "run",
        "--rm",
        "-v",
        f"{claude_dir}:{_LOGIN_HOME}/.claude",
        "-e",
        f"HOME={_LOGIN_HOME}",
        containers.image_name(),
        "claude",
        "auth",
        "status",
    ]
    try:
        proc = subprocess.run(
            args,
            capture_output=True,
            text=True,
            stdin=subprocess.DEVNULL,
            timeout=_STATUS_TIMEOUT,
        )
    except FileNotFoundError:
        raise HTTPException(status_code=503, detail="docker が動いていません")
    except subprocess.TimeoutExpired:
        raise HTTPException(status_code=503, detail="ログインの状態を確かめられませんでした（時間切れ）")
    except OSError as exc:
        logger.warning("docker を呼べません: %s", exc)
        raise HTTPException(status_code=503, detail="docker が動いていません")
    if proc.returncode != 0 and containers._looks_disconnected(proc.stderr or ""):
        raise HTTPException(status_code=503, detail="docker が動いていません")

    method = ""
    try:
        data = json.loads(proc.stdout or "")
    except json.JSONDecodeError:
        data = None
    if isinstance(data, dict):
        method = str(data.get("authMethod") or "")
    return {"logged_in": proc.returncode == 0, "auth_method": method}


def cancel(user: str) -> None:
    """進行中のログインを止める（無ければ何もしない）。"""
    session = _take(user)
    if session is not None:
        session.close()
