"""Discover の検索の引数づくりと検査の小さなテスト（assert だけ・計画 08 §5）。

実行:
    cd <作業用コピー>
    PYTHONPATH=src .venv/bin/python tests/test_papers_search.py
"""

from __future__ import annotations

from fastapi import HTTPException

from sai_web import papers
from sai_web.papers import SearchRequest, build_search_args

AVAILABLE = ["s2", "pubmed", "biorxiv", "medrxiv", "openalex", "arxiv"]


def expect_400(req: SearchRequest, available: list[str] | None = None) -> None:
    """こんな依頼は 400 で弾かれるはず（理由の文も付くこと）。"""
    try:
        build_search_args(req, available if available is not None else AVAILABLE)
    except HTTPException as e:
        assert e.status_code == 400, f"400 のはずが {e.status_code}: {e.detail}"
        assert str(e.detail).strip(), "400 のときは日本語で理由を書く"
        return
    raise AssertionError(f"400 になるはず: {req}")


def test_normal() -> None:
    """正常な依頼の引数（08 §5 の正常の例）。"""
    req = SearchRequest(query="-x attention", dbs=["s2"], year_from=2020, limit=20)
    args = build_search_args(req, AVAILABLE)
    assert args[:3] == ["search", "--json", "--limit"], args
    # - で始まる検索語でもオプションと間違えないよう最後は ["--", 語]（08 §2）
    assert args[-2:] == ["--", "-x attention"], args
    assert args[args.index("--db") + 1] == "s2", args
    assert args[args.index("--year-from") + 1] == "2020", args
    assert args[args.index("--limit") + 1] == "20", args
    assert "--year-to" not in args, args  # 指定しなければ出さない


def test_defaults() -> None:
    """既定（dbs / 年 なし）は --db も --year-* も出さず，空白は取る。"""
    args = build_search_args(SearchRequest(query="  attention  "), AVAILABLE)
    assert "--db" not in args, args  # 省略時は sai-paper の既定を使う（08 §3）
    assert "--year-from" not in args and "--year-to" not in args, args
    assert args[-2:] == ["--", "attention"], args
    assert args[args.index("--limit") + 1] == "20", args  # 既定は 20

    # year_to だけ指定しても出る
    args2 = build_search_args(
        SearchRequest(query="x", year_to=2026, limit=50), AVAILABLE
    )
    assert args2[args2.index("--year-to") + 1] == "2026", args2
    assert "--year-from" not in args2, args2
    assert args2[args2.index("--limit") + 1] == "50", args2


def test_400() -> None:
    """400 になる例（08 §5）。"""
    expect_400(SearchRequest(query="  "))                       # 空の検索語
    expect_400(SearchRequest(query="x" * 201))                  # 200 文字を超える
    expect_400(SearchRequest(query="x", dbs=["evil"]))          # 知らない検索先
    expect_400(SearchRequest(query="x", year_from=2025, year_to=2020))  # 年が逆
    expect_400(SearchRequest(query="x", limit=0))               # 件数が 0
    expect_400(SearchRequest(query="x", limit=101))             # 件数が 100 超
    expect_400(SearchRequest(query="x", year_from=1899))        # 年の下限外
    expect_400(SearchRequest(query="x", year_to=2101))          # 年の上限外
    # available の外はどれでも弾く（順不同でも）
    expect_400(SearchRequest(query="x", dbs=["s2", "evil"]), ["evil"])


def test_search_history_find() -> None:
    """全検索履歴の絞り込み（U01）: 語の検査と、CLI の引数（偽の `_paper_cli` で確かめる）。"""
    calls: list[list[str]] = []

    def fake(project, args, timeout=30.0):
        calls.append(list(args))
        return {"words": "x", "groups": [], "truncated": False}

    papers._paper_cli = fake  # 偽物（CLI は呼ばない）
    project = {"name": "p", "root": "/root/p", "root_name": "p"}
    papers.search_history_find(project, "  large model  ")
    assert calls[-1] == ["search-history-find", "--", "large model"], calls[-1]
    papers.search_history_find(project, "-x")
    assert calls[-1] == ["search-history-find", "--", "-x"], calls[-1]

    n = len(calls)
    for bad in ("", "   ", "x" * 201, "a\nb", "a\rb", None, 12):
        try:
            papers.search_history_find(project, bad)
        except HTTPException as e:
            assert e.status_code == 400, e.status_code
            assert str(e.detail).strip(), "400 のときは日本語で理由を書く"
        else:
            raise AssertionError(f"400 になるはず: {bad!r}")
    assert len(calls) == n, "検査で弾いたら CLI は呼ばない"


def main() -> None:
    test_normal()
    test_defaults()
    test_400()
    test_search_history_find()
    print("test_papers_search: すべて通りました")


main()
