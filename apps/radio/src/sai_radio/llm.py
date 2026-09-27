"""要約・質問応答用のLLM呼び出し．esperanto経由でOpenRouter等を直接叩く．

open-notebookのChat/Transformation機能の代替．
"""

from __future__ import annotations

from esperanto import AIFactory

from sai_radio.config import settings

_RADIO_SCRIPT_PROMPT = (
    "あなたはランニング中のリスナー向けに英語論文を紹介するラジオ原稿の作家です。\n"
    "与えられた論文本文をもとに、日本語のラジオ原稿を書いてください。\n"
    "- 見出しは Markdown の `## 見出し` 形式で、話の区切り(背景・手法・実験・結論など)ごとに付けてください。\n"
    "- 各見出しの下は、実際に読み上げる自然な話し言葉の日本語にしてください。\n"
    "- 専門用語は簡単な言葉で言い換えるか、一言説明を添えてください。"
)


class LLMError(RuntimeError):
    pass


def _chat_model():
    if not settings.llm_model:
        raise LLMError("LLM_MODEL が設定されていません(.env を確認してください)")
    return AIFactory.create_language(settings.llm_provider, settings.llm_model).to_langchain()


def summarize_to_radio_script(markdown_text: str) -> str:
    """PDFから抽出したMarkdown全文を，日本語のラジオ原稿風要約にする．"""
    model = _chat_model()
    prompt = f"{_RADIO_SCRIPT_PROMPT}\n\n---\n論文本文:\n{markdown_text}"
    result = model.invoke(prompt)
    return result.content


def answer(section_text: str, question: str) -> str:
    """論文の該当箇所+質問文から回答を作る．旧 notebook.ask() と同じプロンプト構成．"""
    model = _chat_model()
    message = (
        f"次は論文の該当箇所です。\n---\n{section_text}\n---\n"
        f"この内容を踏まえて、次の質問に日本語で簡潔に答えてください。\n質問: {question}"
    )
    result = model.invoke(message)
    return result.content
