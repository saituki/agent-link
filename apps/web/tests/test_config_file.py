"""`registry.config_var`（環境変数 → `$AGENT_LINK_HOME/settings.json` の `config` → 既定値）を確かめる。

Rust の `settings::config_var_in` のテストと同じ 4 つの場合と、`webcli.base_url()` がファイルのポートを使うこと。

実行:
    cd <作業用コピー>
    PYTHONPATH=apps/web/src apps/web/.venv/bin/python apps/web/tests/test_config_file.py
"""

from __future__ import annotations

import os
import tempfile
from pathlib import Path

home = Path(tempfile.mkdtemp(prefix="agent-link-config-"))
os.environ["AGENT_LINK_HOME"] = str(home)
for k in ("X", "AGENT_LINK_WEB_URL", "AGENT_LINK_WEB_PORT"):
    os.environ.pop(k, None)

from agent_link_web import registry, webcli  # noqa: E402


def write(body: str) -> None:
    (home / "settings.json").write_text(body, encoding="utf-8")


def main() -> None:
    # 環境変数が勝つ
    write('{"config":{"X":"file"}}')
    os.environ["X"] = "env"
    assert registry.config_var("X") == "env"
    # 空白だけの環境変数はファイルに落ちる
    os.environ["X"] = "  "
    assert registry.config_var("X") == "file"
    del os.environ["X"]
    # 整数は文字列になる
    write('{"config":{"AGENT_LINK_WEB_PORT":8401}}')
    assert registry.config_var("AGENT_LINK_WEB_PORT") == "8401"
    # 壊れたファイル・config 無し・型違い・キー無しは default
    write("{ broken json")
    assert registry.config_var("X") is None
    assert registry.config_var("X", "d") == "d"
    write('{"wip":3}')
    assert registry.config_var("X") is None
    write('{"config":{"X":true}}')
    assert registry.config_var("X") is None
    (home / "settings.json").unlink()
    assert registry.config_var("X", "d") == "d"
    # base_url がファイルのポートを使い、環境変数が勝つ
    write('{"config":{"AGENT_LINK_WEB_PORT":8401}}')
    assert webcli.base_url() == "http://127.0.0.1:8401"
    os.environ["AGENT_LINK_WEB_PORT"] = "8297"
    assert webcli.base_url() == "http://127.0.0.1:8297"
    print("ok")


if __name__ == "__main__":
    main()
