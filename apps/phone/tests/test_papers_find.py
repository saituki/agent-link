"""内部検索（find）の引数づくりと検査の小さなテスト（assert だけ・U02）。

実行:
    cd <作業用コピー>
    env -u SAI_LINK_BIN PYTHONPATH=apps/phone/src apps/phone/.venv/bin/python apps/phone/tests/test_papers_find.py
"""

from __future__ import annotations

from fastapi import HTTPException

from sai_phone import papers
from sai_phone.papers import FindRequest, build_find_args


def expect_400(req: FindRequest) -> None:
    """こんな依頼は 400 で弾かれるはず（理由の文も付くこと）。"""
    try:
        build_find_args(req)
    except HTTPException as e:
        assert e.status_code == 400, f"400 のはずが {e.status_code}: {e.detail}"
        assert str(e.detail).strip(), "400 のときは日本語で理由を書く"
        return
    raise AssertionError(f"400 になるはず: {req}")


def test_normal() -> None:
    """正常な依頼の引数（U02 完了条件 AC1 の例）。"""
    req = FindRequest(
        query="epilepsy EEG",
        mode="hybrid",
        top_k=10,
        scope="all",
        papers=[3, 7],
        node="Q12",
        subtree=True,
        each=3,
    )
    args = build_find_args(req)
    assert args == [
        "find", "--json",
        "--mode", "hybrid",
        "--top-k", "10",
        "--scope", "all",
        "--paper", "3", "--paper", "7",
        "--node", "Q12",
        "--subtree",
        "--each", "3",
        "--", "epilepsy EEG",
    ], args


def test_defaults() -> None:
    """指定しない項目は引数に出ない。`-` で始まる語も最後に `--` の後ろへ入る。"""
    args = build_find_args(FindRequest(query="  attention  "))
    assert args == ["find", "--json", "--", "attention"], args

    args2 = build_find_args(FindRequest(query="-x attention"))
    assert args2[-2:] == ["--", "-x attention"], args2

    # node の前後の空白は取る
    args3 = build_find_args(FindRequest(query="x", node="  Q12  "))
    assert args3[args3.index("--node") + 1] == "Q12", args3

    # papers の重複は許す（CLI 側で除く）
    args4 = build_find_args(FindRequest(query="x", papers=[3, 3]))
    assert args4.count("--paper") == 2, args4


def test_400() -> None:
    """400 になる例（U02 完了条件 AC2）。"""
    expect_400(FindRequest(query="  "))  # 空の検索語
    expect_400(FindRequest(query="x" * 201))  # 200 文字を超える
    expect_400(FindRequest(query="x", mode="x"))  # 知らない取り方
    expect_400(FindRequest(query="x", scope="x"))  # 知らない範囲
    expect_400(FindRequest(query="x", top_k=0))  # 件数が 0
    expect_400(FindRequest(query="x", top_k=101))  # 件数が 100 超
    expect_400(FindRequest(query="x", papers=[0]))  # 0 の番号
    expect_400(FindRequest(query="x", papers=[-1]))  # 負の番号
    expect_400(FindRequest(query="x", papers=list(range(1, 22))))  # 21 個
    expect_400(FindRequest(query="x", node="Q0"))  # 0 番の Node
    expect_400(FindRequest(query="x", node="X1"))  # 知らない種類
    expect_400(FindRequest(query="x", node="q1"))  # 小文字
    expect_400(FindRequest(query="x", subtree=True))  # node 無しで subtree
    expect_400(FindRequest(query="x", each=0))  # 論文ごとの件数が 0
    expect_400(FindRequest(query="x", each=21))  # 論文ごとの件数が 20 超
    expect_400(FindRequest(query="x", each=3))  # node も papers も無しで each


def test_find_binary() -> None:
    """node を付けた検索は sai-sci、付けない検索は sai-moon-paper に渡す（U14）。"""
    from types import SimpleNamespace

    seen: list[list[str]] = []
    orig_run, orig_settings = papers.cli.run_json, papers.settings
    papers.cli.run_json = lambda args, **kw: seen.append(list(args)) or {"log": None, "hits": []}
    papers.settings = SimpleNamespace(sai_paper_bin="sai-paper", sai_sci_bin="sai-sci")
    try:
        project = {"name": "p", "root": "/tmp/p"}
        papers.find(project, FindRequest(query="x", node="Q12"))
        papers.find(project, FindRequest(query="x"))
    finally:
        papers.cli.run_json, papers.settings = orig_run, orig_settings
    assert seen[0][0] == "sai-sci" and seen[1][0] == "sai-paper", seen
    assert seen[0][1:] == ["find", "--json", "--node", "Q12", "--", "x"], seen[0]
    assert seen[1][1:] == ["find", "--json", "--", "x"], seen[1]


def main() -> None:
    test_normal()
    test_defaults()
    test_400()
    test_find_binary()
    print("test_papers_find: すべて通りました")


main()
