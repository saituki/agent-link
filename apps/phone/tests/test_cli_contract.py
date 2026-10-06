"""Web が CLI を呼ぶ約束（共有契約・U09）の読み替えの試験。

確かめること:
- `cli.run_json` の読み替え（終了コード 0 / 2 / 3 / 4 / 1 / その他）
  - 0 で JSON → その JSON
  - 0 で JSON でない → 502
  - 2 → 400・3 → 404・4 → 409・1 とそのほか → 502
  - 理由は標準エラーの最後の空でない行（`sai-progress` の行は飛ばす）
  - 標準出力の `{"error": …}` は理由に使わない（終了コードで決める）
- 起動できない → 503・時間切れ → 504（`run_cli`）
- 呼び出しの env（`_phone_env` / sai-paper の `_paper_cli`）
  - `SAI_PROJECT_ROOT` と `SAI_ORIGIN=phone` があり，`SAI_PAPER_ROOT` が無い

実行:
    cd <作業用コピー>
    env -u SAI_LINK_BIN PYTHONPATH=apps/phone/src apps/phone/.venv/bin/python apps/phone/tests/test_cli_contract.py
"""

from __future__ import annotations

import subprocess

from fastapi import HTTPException

from sai_phone import cli, control, papers


def expect(code: int, fn) -> HTTPException:
    try:
        fn()
    except HTTPException as e:
        assert e.status_code == code, f"{code} のはずが {e.status_code}: {e.detail}"
        assert str(e.detail).strip(), f"{code} のときは日本語で理由を書く"
        return e
    raise AssertionError(f"{code} になるはず")


def proc(code: int, stdout: str = "", stderr: str = "") -> subprocess.CompletedProcess:
    return subprocess.CompletedProcess(["x"], code, stdout, stderr)


def call(code: int, stdout: str = "", stderr: str = ""):
    """`run_json` を偽の runner で呼ぶ。"""
    return cli.run_json(
        ["x"], app="sai-link", runner=lambda *a, **k: proc(code, stdout, stderr)
    )


def test_zero_json() -> None:
    assert call(0, '{"a": 1}') == {"a": 1}
    # 0 で JSON でない → 502
    e = expect(502, lambda: call(0, "not json"))
    assert e.detail == "sai-link の応答を読めませんでした", e.detail
    # 空の標準出力も同じ
    expect(502, lambda: call(0, ""))


def test_exit_codes() -> None:
    # 2 → 400
    e = expect(400, lambda: call(2, "", "引数の誤り: bad"))
    assert e.detail == "引数の誤り: bad", e.detail
    # 3 → 404
    e = expect(404, lambda: call(3, "", "実行が見つかりません"))
    assert e.detail == "実行が見つかりません", e.detail
    # 4 → 409
    e = expect(409, lambda: call(4, "", "実行中です"))
    assert e.detail == "実行中です", e.detail
    # 1 → 502
    e = expect(502, lambda: call(1, "", "DB を開けません"))
    assert e.detail == "DB を開けません", e.detail
    # その他 → 502
    e = expect(502, lambda: call(7, "", "その他"))
    assert e.detail == "その他", e.detail
    # stderr が空なら既定の文
    e = expect(502, lambda: call(1, "", ""))
    assert e.detail == "sai-link が失敗しました", e.detail


def test_reason_is_last_line() -> None:
    """理由は標準エラーの最後の空でない行（`sai-progress` は飛ばす）。"""
    e = expect(502, lambda: call(1, "", "sai-progress step=1\n本当の理由\n\n"))
    assert e.detail == "本当の理由", e.detail
    # 進捗しか無ければ既定の文
    e = expect(502, lambda: call(1, "", "sai-progress step=1\nsai-progress msg=途中\n"))
    assert e.detail == "sai-link が失敗しました", e.detail


def test_stdout_error_is_not_special() -> None:
    """標準出力の `{"error"}` は理由に使わない（終了コードで決める・U14 で形 A を消した）。"""
    e = expect(502, lambda: call(1, '{"error": "stdout の理由"}'))
    assert e.detail == "sai-link が失敗しました", e.detail
    # 2 なら標準エラーの理由で 400
    e = expect(400, lambda: call(2, '{"error": "stdout の理由"}', "引数の誤り"))
    assert e.detail == "引数の誤り", e.detail
    # 0 で `{"error"}` でも成功として JSON を返す
    assert call(0, '{"error": "これは値"}') == {"error": "これは値"}


def test_run_cli_503_504() -> None:
    # 起動できない → 503
    e = expect(503, lambda: cli.run_cli(["/no/such/bin-xyz"], app="sai-link"))
    assert "見つかりません" in str(e.detail), e.detail
    # 時間切れ → 504
    e = expect(
        504, lambda: cli.run_cli(["/bin/sleep", "5"], app="sai-link", timeout=0.05)
    )
    assert "時間内" in str(e.detail), e.detail


def test_phone_env() -> None:
    env = control._phone_env("/tmp/root")
    assert env["SAI_PROJECT_ROOT"] == "/tmp/root", env.get("SAI_PROJECT_ROOT")
    assert env["SAI_ORIGIN"] == "phone", env.get("SAI_ORIGIN")
    assert "SAI_PAPER_ROOT" not in env, env.get("SAI_PAPER_ROOT")


def test_paper_env() -> None:
    """sai-paper の呼び出しも `SAI_PROJECT_ROOT` と `SAI_ORIGIN` で，`SAI_PAPER_ROOT` を渡さない。"""
    seen: list[dict] = []
    orig = papers.cli.run_json
    papers.cli.run_json = lambda args, **kw: seen.append(dict(kw, args=list(args))) or {}
    try:
        papers.library({"name": "p", "root": "/tmp/p-root"})
    finally:
        papers.cli.run_json = orig
    assert seen, "呼ばれていない"
    env = seen[-1]["env"]
    assert env["SAI_PROJECT_ROOT"] == "/tmp/p-root", env
    assert env["SAI_ORIGIN"] == "phone", env
    assert "SAI_PAPER_ROOT" not in env, env
    assert seen[-1]["args"][1:] == ["library", "list"], seen[-1]["args"]


def main() -> None:
    test_zero_json()
    test_exit_codes()
    test_reason_is_last_line()
    test_stdout_error_is_not_special()
    test_run_cli_503_504()
    test_phone_env()
    test_paper_env()
    print("test_cli_contract: すべて通りました")


main()
