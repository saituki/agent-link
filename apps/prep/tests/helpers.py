"""テストの共通の下ごしらえ。

`src` を `sys.path` に入れて `sai_prep` を import できるようにする。各テストは
先頭で `import helpers` する。
"""

from __future__ import annotations

import socket
import sys
from pathlib import Path

SRC = Path(__file__).resolve().parents[1] / "src"
if str(SRC) not in sys.path:
    sys.path.insert(0, str(SRC))


def free_port() -> int:
    """空いているポートを 1 つ返す。"""
    sock = socket.socket()
    sock.bind(("127.0.0.1", 0))
    port = sock.getsockname()[1]
    sock.close()
    return port
