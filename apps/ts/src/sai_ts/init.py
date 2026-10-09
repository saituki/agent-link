"""sai-ts init（C12）。ts/ を作り、block ts-agent を sai-link の定義に入れる。"""

from __future__ import annotations

from pathlib import Path

from . import linkcli, paths

BLOCK = Path(__file__).resolve().parents[2] / "blocks" / "ts-agent.md"
WRITE = ["def", "write", "--kind", "block", "--scope", "project", "--name", "ts-agent", "--json"]


def init(update: bool) -> dict:
    root = paths.ts_root()
    for d in (root, paths.datasets_dir(), paths.ext_dir(), paths.scripts_dir()):
        d.mkdir(parents=True, exist_ok=True)

    text = BLOCK.read_text(encoding="utf-8")
    try:
        linkcli.run([*WRITE, "--create"], stdin=text)
        block = "created"
    except linkcli.LinkError as e:
        if e.status != 409:  # 409 は「既にあります」（sai-link の終了コード 4）
            raise
        if not update:
            return {"ts_root": str(root), "block": "exists"}
        linkcli.run(WRITE, stdin=text)
        block = "updated"
    return {"ts_root": str(root), "block": block}
