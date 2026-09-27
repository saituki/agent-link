"""PDFをMarkdownに変換する．docling CLIをサブプロセスとして呼ぶだけ．

sai-paper(別プロジェクト)の crates/sai-paper-core/src/citation/docling.rs と
同じ発想(docling <pdf> --to md --output <dir> を起動して結果を読む)．
"""

from __future__ import annotations

import subprocess
import tempfile
from pathlib import Path

from sai_radio.config import settings


class ExtractError(RuntimeError):
    pass


def extract_markdown(pdf_path: Path) -> str:
    """PDFをdoclingでMarkdown化して，その全文を返す．"""
    with tempfile.TemporaryDirectory() as tmp:
        tmp_dir = Path(tmp)
        try:
            # docling convert <pdf> --to md --output <dir>
            subprocess.run(
                [settings.docling_bin, "convert", str(pdf_path), "--to", "md", "--output", str(tmp_dir)],
                check=True,
                capture_output=True,
                timeout=settings.docling_timeout_sec,
            )
        except subprocess.CalledProcessError as e:
            raise ExtractError(
                f"docling によるPDF抽出に失敗しました: {e.stderr.decode(errors='ignore')}"
            ) from e
        except subprocess.TimeoutExpired as e:
            raise ExtractError(f"docling がタイムアウトしました({settings.docling_timeout_sec}秒)") from e

        # docling convert の出力ファイル名は「入力PDFのstem.md」の規約
        # フォールバック: tmp_dir配下を rglob で探す (バージョンによって出力パスが異なる可能性があるため)
        md_files = list(tmp_dir.rglob("*.md"))
        if not md_files:
            raise ExtractError(f"docling の出力ファイルが見つかりません: {tmp_dir}")

        # 最初に見つかったMarkdownファイルを使う
        md_path = md_files[0]
        return md_path.read_text(encoding="utf-8")
