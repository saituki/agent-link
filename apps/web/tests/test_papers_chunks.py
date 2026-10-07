"""論文のチャンク一覧 API（`GET …/papers/{id}/chunks`）のテスト（assert だけ）。

確かめること:
- `content chunks <id> --json` の引数（`--json` 付き）
- 返す JSON の項目（paper_id・title・chunks の各項目）
- CLI が終了コード 3（本文なし）なら 404、形が違えば 502
- 住所が `/text` の隣に登録されている

実行:
    cd <作業用コピー>
    PYTHONPATH=src ~/workdir/Sai/sai-moon/apps/web/.venv/bin/python tests/test_papers_chunks.py
"""

from __future__ import annotations

import json
import subprocess
from types import SimpleNamespace

from fastapi import HTTPException

from sai_web import papers
from sai_web.main import app

PROJECT = {"name": "sai-paper", "root": "/root/sai-paper", "root_name": "sai-paper"}
CHUNK = {"id": "#57.3.2", "section": "#57.3", "heading": "3 Method", "text": "本文", "n_sentences": 4}


def fake_cli(returncode: int, stdout: str = "", stderr: str = ""):
    """`cli.run_cli` の代わり。呼ばれた引数を calls に残す。"""
    calls: list[list[str]] = []

    def run(args, **kwargs):
        calls.append(args)
        return subprocess.CompletedProcess(args, returncode, stdout, stderr)

    return run, calls


def failure_status(fn) -> int:
    try:
        fn()
    except HTTPException as e:
        assert str(e.detail).strip(), "理由を書く"
        return e.status_code
    raise AssertionError("失敗するはず")


def main() -> None:
    papers.settings = SimpleNamespace(sai_paper_bin="sai-paper")
    original = papers.cli.run_cli
    try:
        ok = {"paper_id": 57, "title": "題名", "chunks": [CHUNK]}
        papers.cli.run_cli, calls = fake_cli(0, json.dumps(ok))
        assert papers.chunks(PROJECT, 57) == ok
        assert calls[0][1:] == ["content", "chunks", "57", "--json"], calls
        assert set(papers.chunks(PROJECT, 57)["chunks"][0]) == set(CHUNK)

        papers.cli.run_cli, _ = fake_cli(3, "", "論文 57 の本文がまだ取り出されていません")
        assert failure_status(lambda: papers.chunks(PROJECT, 57)) == 404

        papers.cli.run_cli, _ = fake_cli(0, json.dumps({"paper_id": 57}))
        assert failure_status(lambda: papers.chunks(PROJECT, 57)) == 502
    finally:
        papers.cli.run_cli = original

    paths = [r.path for r in app.routes]
    assert "/api/projects/{name}/papers/{paper_id:int}/chunks" in paths, "住所が無い"
    print("test_papers_chunks: すべて通りました")


main()
