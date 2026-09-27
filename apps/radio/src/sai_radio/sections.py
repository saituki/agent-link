"""要約テキストを「見出し単位」のセクションに分割する処理．

見出しは Markdown の `#`〜`######` 行として扱う．見出しが1つも
無い要約は，全体をまとめて1セクション扱いにする．
"""

from __future__ import annotations

import re
from dataclasses import dataclass

_HEADING_RE = re.compile(r"^#{1,6}\s+(.+?)\s*$")


@dataclass
class TextSection:
    title: str
    body: str  # 読み上げる本文 (見出し行は含まない)


def split_into_sections(summary_text: str, fallback_title: str = "全体") -> list[TextSection]:
    lines = summary_text.splitlines()
    sections: list[TextSection] = []
    current_title: str | None = None
    current_body: list[str] = []

    def flush() -> None:
        body = "\n".join(current_body).strip()
        if body:
            sections.append(TextSection(title=current_title or fallback_title, body=body))

    for line in lines:
        m = _HEADING_RE.match(line)
        if m:
            flush()
            current_title = m.group(1)
            current_body = []
        else:
            current_body.append(line)
    flush()

    if not sections:
        body = summary_text.strip()
        if body:
            sections.append(TextSection(title=fallback_title, body=body))

    return sections
