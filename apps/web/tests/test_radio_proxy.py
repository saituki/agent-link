"""Radio 中継の path 検査と送り先 URL の組み立ての小さなテスト（assert だけ・計画 09 §5）。

sai-radio への通信（radio.forward）は呼ばず，`build_target(path, query)` だけを確かめる。

実行:
    cd <作業用コピー>
    PYTHONPATH=src .venv/bin/python tests/test_radio_proxy.py
"""

from __future__ import annotations

from fastapi import HTTPException

from sai_web.config import settings
from sai_web.radio import build_target


def expect_error(path: str, query: list[tuple[str, str]], status: int) -> None:
    """path は status（400 / 403）で弾かれるはず（理由の日本語も付くこと）。"""
    try:
        build_target(path, query)
    except HTTPException as e:
        assert e.status_code == status, f"{status} のはずが {e.status_code}: {e.detail}"
        assert str(e.detail).strip(), f"{status} のときは日本語で理由を書く"
        return
    raise AssertionError(f"{status} になるはず: {path!r}")


def test_normal() -> None:
    """普通の path は radio_url の /api/ 以下に組み上がる（09 §5）。"""
    url = build_target("papers", [])
    assert url == f"{settings.radio_url}/api/papers", url

    url2 = build_target("papers/49/audio", [("paper_id", "49")])
    assert url2 == f"{settings.radio_url}/api/papers/49/audio?paper_id=49", url2

    # ask（質問）のような path も同じ組み立て方
    url3 = build_target("papers/49/ask", [])
    assert url3 == f"{settings.radio_url}/api/papers/49/ask", url3


def test_token_removed() -> None:
    """合言葉 token は sai-web 自分の物なので送り先に残らない（09 §5）。"""
    url = build_target("papers", [("token", "sai-web-secret"), ("paper_id", "49")])
    assert "token=" not in url, url
    assert "sai-web-secret" not in url, url
    assert "paper_id=49" in url, url

    # token が先頭・後ろどちらに来ても消える
    url2 = build_target("stats", [("token", "x")])
    assert url2 == f"{settings.radio_url}/api/stats", url2


def test_400() -> None:
    """`..` や英数字以外は 400（09 §5）。"""
    expect_error("../etc/passwd", [], 400)      # .. を含む
    expect_error("papers/../../secret", [], 400)  # 中間の ..
    expect_error("papers/49 aud", [], 400)        # スペース
    expect_error("papers/49?x=1", [], 400)        # 記号が勝手な物
    expect_error("", [], 400)                     # 空の path


def test_403() -> None:
    """音声作りは sai-web から使わせない（09 §5）。"""
    expect_error("papers/upload", [], 403)
    expect_error("papers/from-extract", [], 403)
    # 同じ始まりの下の path も弾く（勝手に登録し直させないため）
    expect_error("papers/upload/extra", [], 403)
    # 似ているけれど別の path は通る（一覧は中継する）
    url = build_target("papers", [])
    assert url.endswith("/api/papers"), url


def main() -> None:
    test_normal()
    test_token_removed()
    test_400()
    test_403()
    print("test_radio_proxy: すべて通りました")


main()
