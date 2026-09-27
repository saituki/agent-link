"""論文を登録する処理．CLI・Web UIアップロード・sai-paper連携の3経路から呼ばれる共通ロジック．

使い方（CLI）:
    python -m sai_radio.ingest 論文.pdf
    python -m sai_radio.ingest 論文.pdf --mode podcast --title "論文タイトル"
"""

from __future__ import annotations

import argparse
import asyncio
from pathlib import Path

from sai_radio import extract, llm, voice
from sai_radio.podcast import build_podcast
from sai_radio.sections import split_into_sections
from sai_radio.store import init_db, save_paper


async def ingest_from_markdown(
    markdown: str,
    mode: str,
    title: str,
    paper_id: str | None = None,
    request_settings: dict | None = None,
    audio_dir: str | None = None,
):
    """抽出済みMarkdownから登録する(sai-paper連携・PDFアップロード共通の後半処理)．

    request_settings / audio_dir は podcast モードだけで使う(build_podcast を参照)．
    """
    paper_id = paper_id or voice.new_paper_id()
    # 重い同期処理(LLM呼び出し・音声合成)は asyncio.to_thread で別スレッドに移す.
    # async 関数の中で直接呼ぶと，その間 FastAPI の受付(Event Loop)が止まり，
    # サーバー全体(再生中の音声配信や一覧表示)が固まってしまうため.
    script = await asyncio.to_thread(llm.summarize_to_radio_script, markdown)

    if mode == "narration":
        sections = split_into_sections(script)
        audio_path, manifest = await asyncio.to_thread(voice.build_narration, sections, paper_id)
    elif mode == "podcast":
        audio_path, manifest = await build_podcast(script, paper_id, request_settings, audio_dir)
    else:
        raise ValueError(f"不明な mode です: {mode}")

    init_db()
    return save_paper(paper_id, title, mode, audio_path, manifest)


async def ingest_from_pdf(pdf_path: Path, mode: str, title: str | None = None):
    """生PDFから登録する(CLI・Web UIアップロード用)．"""
    title = title or pdf_path.stem
    # docling の抽出は最大300秒かかる同期処理なので，同上の理由で別スレッドへ移す.
    markdown = await asyncio.to_thread(extract.extract_markdown, pdf_path)
    return await ingest_from_markdown(markdown, mode, title)


def main() -> None:
    parser = argparse.ArgumentParser(description="論文PDFを登録して音声を作る")
    parser.add_argument("pdf", type=Path, help="論文PDFのパス")
    parser.add_argument("--mode", choices=["narration", "podcast"], default="narration")
    parser.add_argument("--title", default=None, help="論文タイトル (省略時はファイル名)")
    args = parser.parse_args()
    paper = asyncio.run(ingest_from_pdf(args.pdf, args.mode, args.title))
    print(f"完了: paper_id={paper.id}  audio={paper.audio_path}  セクション数={len(paper.sections)}")


if __name__ == "__main__":
    main()
