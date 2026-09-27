"""音声生成の間にサーバーが止まらないことを確認するテスト（壊れたら失敗する）．

考え方:
    音声生成(ingest_from_markdown)の間に，別スレッドで 0.05 秒ごとに数を数える
    ticker が動いているかを見る．もし重い処理が別スレッドに飛んでいなければ，
    音声生成のあいだ ticker は数えられないので，数が少なくて失敗する．

実行:
    PYTHONPATH=src python tests/test_ingest_nonblocking.py
"""

from __future__ import annotations

import asyncio
import sys
import time
from pathlib import Path

# src/ を読み込めるようにする（作業用コピーから実行するため）
ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / "src"))

from sai_radio import ingest  # noqa: E402


def _slow_summarize(markdown: str) -> str:
    """LLM の代わり: 0.3 秒待ってから文字列を返す偽物．"""
    time.sleep(0.3)
    return "## はじめに\nこれはテスト用の原稿です。"


def _fake_build_narration(sections, paper_id):
    """音声合成の代わり: 何も作らずにダミーを返す偽物．"""
    return f"audio/{paper_id}.m4a", []


def _fake_save_paper(paper_id, title, mode, audio_path, manifest):
    """DB 保存の代わりの偽物（ファイルも DB も作らない）．"""
    return type("Paper", (), {"id": paper_id, "audio_path": audio_path, "sections": manifest})()


async def _ticker(ingest_task):
    """0.05 秒ごとに数を数えるだけの関数（サーバーが止まっていない証拠）．

    音声生成(ingest_task)が終わったら数えるのをやめて，
    「音声生成の最中にだけ，何回数えたか」を正確に測る．
    """
    n = 0
    while not ingest_task.done():
        n += 1
        await asyncio.sleep(0.05)
    return n


async def _main() -> int:
    # 重い処理・ファイルや DB を触る処理をすべて軽い偽物に差し替える．
    ingest.llm.summarize_to_radio_script = _slow_summarize
    ingest.split_into_sections = lambda script: ["dummy"]
    ingest.voice.build_narration = _fake_build_narration
    ingest.init_db = lambda: None
    ingest.save_paper = _fake_save_paper

    # 音声生成とカウンタを同時に走らせる．
    ingest_task = asyncio.ensure_future(
        ingest.ingest_from_markdown("md", "narration", "t", paper_id="1")
    )
    _, counted = await asyncio.gather(ingest_task, _ticker(ingest_task))

    print(f"音声生成のあいだに数えた回数: {counted}")
    assert counted >= 4, (
        f"音声生成の最中にサーバーが止まっている（数えた回数 {counted} < 4）"
    )
    print("OK: 音声生成のあいだもサーバーは動き続けている")
    return 0


if __name__ == "__main__":
    sys.exit(asyncio.run(_main()))
