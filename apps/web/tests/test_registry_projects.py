"""プロジェクト一覧・Dashboard・常駐の操作（実装計画 C C1・C2）の小さなテスト。

確かめること:
- `registry.load_registry` は `~/.sai/registry.json` から `kind == "link"` だけを引き，
  `root_exists` / `db_exists` / `daemon_alive` を足す（`SAI_HOME` で差し替える）
- `control.list_projects` は項目名と並び順を今までどおりにし，
  `sai-link phone summary` は root と DB が両方あるプロジェクトだけで呼ぶ
- 最小集計は 5 秒だけ覚える／失敗しても一覧は止めない
- `control.dashboard` は `sai-link phone dashboard --period <p>` を root で呼ぶ
- `control.daemon` は `sai-link daemon <start|stop>` を root で呼ぶ

実行:
    cd <作業用コピー>
    PYTHONPATH=src .venv/bin/python tests/test_registry_projects.py
"""

from __future__ import annotations

import json
import os
import subprocess
import tempfile
import time
from pathlib import Path

from fastapi import HTTPException

from sai_web import control, registry
from sai_web.config import settings


def expect(code: int, fn) -> HTTPException:
    """fn は指定した状態番号の HTTPException で失敗し，日本語の理由が付くこと。"""
    try:
        fn()
    except HTTPException as e:
        assert e.status_code == code, f"{code} のはずが {e.status_code}: {e.detail}"
        assert str(e.detail).strip(), f"{code} のときは日本語で理由を書く"
        return e
    raise AssertionError(f"{code} になるはず")


class FakeCli:
    """`control.run_cli` の置き換え係。呼ばれた引数を全部覚えておく。"""

    def __init__(self) -> None:
        self.calls: list[dict] = []
        self.returncode = 0
        self.stdout = "{}"
        self.stderr = ""
        # プロジェクトごとに標準出力を差し替える（応答挂钩）
        self.by_cwd: dict[str, str] = {}

    def __call__(self, args, *, app, cwd=None, env=None, timeout=20.0, input_text=None):
        self.calls.append(
            {"args": list(args), "app": app, "cwd": cwd, "env": env, "timeout": timeout}
        )
        out = self.stdout
        if cwd and cwd in self.by_cwd:
            out = self.by_cwd[cwd]
        return subprocess.CompletedProcess(list(args), self.returncode, out, self.stderr)

    def link_calls(self) -> list[list[str]]:
        """sai-link を呼んだ argv を全部取り出す（U09 で接頭辞の `phone` が消えた）。"""
        return [c["args"] for c in self.calls]


def make_registry(home: Path, base: Path) -> None:
    """一時フォルダに DB と PID ファイルを用意し、登録簿を書く。"""
    # alpha: root と DB があり、常駐も生きている（daemon.pid は自分の PID）
    alpha = base / "alpha"
    (alpha / ".sai" / "sai-engine").mkdir(parents=True)
    (alpha / ".sai" / "sai-engine" / "sai-engine.db").write_text("")
    (alpha / ".sai" / "sai-engine" / "daemon.pid").write_text(f"{os.getpid()}\n")
    # beta: root と DB がある（PID ファイルが無い = 常駐は止まっている）
    beta = base / "beta"
    (beta / ".sai" / "sai-engine").mkdir(parents=True)
    (beta / ".sai" / "sai-engine" / "sai-engine.db").write_text("")
    # gamma: root はあるが DB が無い
    (base / "gamma").mkdir(parents=True)
    # delta: root 自体が無い

    entries = [
        _entry("alpha", alpha, "2026-09-20T00:00:00+00:00"),
        _entry("beta", beta, "2026-09-25T00:00:00+00:00"),
        _entry("gamma", base / "gamma", "2026-09-21T00:00:00+00:00"),
        _entry("delta", base / "delta", "2026-09-19T00:00:00+00:00"),
        # link 以外（paper）は対象外
        {**_entry("paperish", beta, "2026-09-26T00:00:00+00:00"), "kind": "paper"},
    ]
    home.mkdir(parents=True, exist_ok=True)
    (home / "registry.json").write_text(json.dumps(entries, ensure_ascii=False))


def _entry(name: str, root: Path, last_opened: str) -> dict:
    return {
        "name": name,
        "root": str(root),
        "kind": "link",
        "color": None,
        "created_at": "2026-09-01T00:00:00+00:00",
        "last_opened": last_opened,
    }


def test_load_registry(base: Path) -> None:
    """`kind == "link"` だけを引き、root / DB / 常駐の生死と並びを確かめる。"""
    rows = registry.load_registry()
    assert [r["name"] for r in rows] == ["beta", "gamma", "alpha", "delta"], rows
    by_name = {r["name"]: r for r in rows}
    # alpha: 3 つとも真
    assert by_name["alpha"]["root_exists"] is True
    assert by_name["alpha"]["db_exists"] is True
    assert by_name["alpha"]["daemon_alive"] is True, by_name["alpha"]
    # beta: root と DB はあるが PID ファイルが無い
    assert by_name["beta"]["db_exists"] is True
    assert by_name["beta"]["daemon_alive"] is False, by_name["beta"]
    # gamma: root はあるが DB が無い
    assert by_name["gamma"]["root_exists"] is True
    assert by_name["gamma"]["db_exists"] is False
    # delta: root が無い
    assert by_name["delta"]["root_exists"] is False
    assert by_name["delta"]["db_exists"] is False
    assert by_name["delta"]["daemon_alive"] is False
    # 並びの鍵になる last_opened も持つ
    assert by_name["beta"]["last_opened"] == "2026-09-25T00:00:00+00:00", by_name["beta"]
    # paper は出ない
    assert "paperish" not in by_name, by_name


def test_load_registry_edges(tmp: Path) -> None:
    """ファイルが無い / 壊れている / 形で違う場合は空のリスト（ログだけ）。"""
    assert registry.load_registry(tmp / "nothing.json") == [], "無いファイルは空"
    broken = tmp / "broken.json"
    broken.write_text("{ 壊れている")
    assert registry.load_registry(broken) == [], "壊れた JSON は空"
    shape = tmp / "shape.json"
    shape.write_text('{"projects": []}')
    assert registry.load_registry(shape) == [], "配列でない JSON は空"


def test_daemon_alive_dead_pid(base: Path) -> None:
    """PID ファイルに死んでいる PID が入っていれば False。"""
    root = base / "alpha"
    pid_file = root / ".sai" / "sai-engine" / "daemon.pid"
    pid_file.write_text("999999\n")
    assert registry.daemon_alive(str(root)) is False
    pid_file.write_text("これは PID ではない")
    assert registry.daemon_alive(str(root)) is False
    assert registry.process_alive(0) is False
    # 以後のテストのためにもとの状態（常駐が生きている状態）へ戻す
    pid_file.write_text(f"{os.getpid()}\n")


def test_list_projects(base: Path) -> None:
    """項目名と並び順を今までどおりにし、summary は DB のあるプロジェクトだけ。"""
    cli = FakeCli()
    cli.by_cwd = {
        str(base / "alpha"): json.dumps({"running": 2, "last_executed_at": "2026-09-20 01:00:00"}),
        str(base / "beta"): json.dumps({"running": 0, "last_executed_at": "2026-09-25 03:00:00"}),
    }
    original = control.run_cli
    control.run_cli = cli
    control._summary_cache.clear()
    try:
        rows = control.list_projects()
    finally:
        control.run_cli = original
        control._summary_cache.clear()

    # 返す項目の名前と並び順（last_executed_at があるもの→新しい順、無いものは後ろ）
    assert [set(r) for r in rows] == [
        {"name", "root", "kind", "root_exists", "db_exists", "daemon_alive", "running", "last_executed_at"}
    ] * 4, rows
    assert [r["name"] for r in rows] == ["beta", "alpha", "gamma", "delta"], rows
    assert rows[0]["running"] == 0, rows[0]
    assert rows[0]["last_executed_at"] == "2026-09-25 03:00:00", rows[0]
    assert rows[1]["running"] == 2 and rows[1]["daemon_alive"] is True, rows[1]
    assert rows[2]["last_executed_at"] is None and rows[2]["db_exists"] is False, rows[2]
    assert rows[3]["root_exists"] is False, rows[3]

    # summary を呼ぶのは alpha と beta だけ（DB と root が両方ある物）
    assert cli.link_calls() == [
        [settings.sai_link_bin, "project", "summary"],
        [settings.sai_link_bin, "project", "summary"],
    ], cli.link_calls()
    for call in cli.calls:
        assert call["cwd"] in (str(base / "alpha"), str(base / "beta")), call
        assert call["env"]["SAI_PROJECT_ROOT"] == call["cwd"], call
        assert call["env"]["SAI_ORIGIN"] == "phone", call


def test_summary_cache(base: Path) -> None:
    """同じプロジェクトは 5 秒以内に summary を呼び直さない。"""
    cli = FakeCli()
    cli.by_cwd = {
        str(base / "alpha"): json.dumps({"running": 1, "last_executed_at": "2026-09-20 01:00:00"}),
        str(base / "beta"): json.dumps({"running": 0, "last_executed_at": "2026-09-25 03:00:00"}),
    }
    original = control.run_cli
    control.run_cli = cli
    control._summary_cache.clear()
    try:
        control.list_projects()
        assert len(cli.calls) == 2, cli.calls
        control.list_projects()
        assert len(cli.calls) == 2, f"5 秒以内にsummary を呼び直した: {cli.calls}"
        # 5 秒を過ぎた想定で引き直す
        control._summary_cache = {
            k: (time.monotonic() - 6.0, v[1]) for k, v in control._summary_cache.items()
        }
        control.list_projects()
        assert len(cli.calls) == 4, cli.calls
    finally:
        control.run_cli = original
        control._summary_cache.clear()


def test_summary_failure(base: Path) -> None:
    """summary が失敗しても一覧は止めない（running=0 / last_executed_at=None）。"""
    cli = FakeCli()
    cli.returncode = 1
    cli.stderr = "sai-link が失敗しました"
    original = control.run_cli
    control.run_cli = cli
    control._summary_cache.clear()
    try:
        rows = control.list_projects()
    finally:
        control.run_cli = original
        control._summary_cache.clear()
    assert len(rows) == 4, rows
    assert all(r["running"] == 0 for r in rows), rows
    assert all(r["last_executed_at"] is None for r in rows), rows


def _stub_project(project: dict):
    """find_project の偽物（登録簿の行をそのまま返す）。"""

    def find(name: str) -> dict:
        if name == project["name"]:
            return project
        raise HTTPException(status_code=404, detail="プロジェクトが見つかりません")

    return find


def test_dashboard(base: Path) -> None:
    """`sai-link phone dashboard --period <p>` を root で呼ぶ。recent にも足す。"""
    root = str(base / "alpha")
    project = {
        "name": "alpha",
        "root": root,
        "kind": "link",
        "root_exists": True,
        "db_exists": True,
        "daemon_alive": True,
        "running": 0,
        "last_executed_at": None,
    }
    body = {
        "name": "alpha",
        "period": "week",
        "since": "2026-09-21",
        "until": "2026-09-26",
        "daemon_alive": True,
        "summary": {"executions": 2},
        "daily": [],
        "by_model": [],
        "by_block": [],
        "recent": [
            {"run_id": 7, "parent_run_id": None, "block": "a", "status": "completed"},
            {"run_id": 8, "parent_run_id": None, "block": "b", "status": "completed"},
        ],
    }
    cli = FakeCli()
    cli.stdout = json.dumps(body)
    original_run_cli, original_find = control.run_cli, control.find_project
    original_load = control.runs.load_runs
    control.run_cli = cli
    control.find_project = _stub_project(project)
    control.runs.load_runs = lambda: [
        {"project_root": root, "run_id": 8, "description": "説明8", "project_name": "別名"}
    ]
    try:
        data = control.dashboard("alpha", "week")
    finally:
        control.run_cli = original_run_cli
        control.find_project = original_find
        control.runs.load_runs = original_load
        control._last_export.clear()

    dash = [c for c in cli.calls if "dashboard" in c["args"]]
    assert len(dash) == 1, cli.calls
    assert dash[0]["args"] == [
        settings.sai_link_bin,
        "project",
        "dashboard",
        "--period",
        "week",
    ], dash[0]["args"]
    assert dash[0]["cwd"] == root, dash[0]
    assert dash[0]["env"]["SAI_PROJECT_ROOT"] == root, dash[0]
    # recent には description / linkable / project が足される
    assert data["recent"][0] == {
        "run_id": 7,
        "parent_run_id": None,
        "block": "a",
        "status": "completed",
        "description": "",
        "linkable": False,
    }, data["recent"][0]
    assert data["recent"][1]["description"] == "説明8", data["recent"][1]
    assert data["recent"][1]["linkable"] is True
    assert data["recent"][1]["project"] == "別名", data["recent"][1]

    # period の検査は前と同じ（400）
    expect(400, lambda: control.dashboard("alpha", "year"))


def test_dashboard_no_db(base: Path) -> None:
    """DB が無いプロジェクトの Dashboard は 404（CLI を呼ばない）。"""
    project = {
        "name": "gamma",
        "root": str(base / "gamma"),
        "kind": "link",
        "root_exists": True,
        "db_exists": False,
        "daemon_alive": False,
        "running": 0,
        "last_executed_at": None,
    }
    cli = FakeCli()
    original_find = control.find_project
    control.find_project = _stub_project(project)
    try:
        expect(404, lambda: control.dashboard("gamma", "week"))
    finally:
        control.find_project = original_find
    assert cli.calls == [], cli.calls


def test_daemon(base: Path) -> None:
    """常駐の起動・停止は `sai-link daemon <action>` を root で呼ぶ。"""
    root = str(base / "alpha")
    project = {
        "name": "alpha",
        "root": root,
        "kind": "link",
        "root_exists": True,
        "db_exists": True,
        "daemon_alive": False,
        "running": 0,
        "last_executed_at": None,
    }
    cli = FakeCli()
    cli.stdout = "🛑 sai デーモンを停止しました (PID: 123)\n"
    original_run_cli, original_find = control.run_cli, control.find_project
    control.run_cli = cli
    control.find_project = _stub_project(project)
    try:
        res = control.daemon("alpha", "stop")
        assert res == {"ok": True, "message": "🛑 sai デーモンを停止しました (PID: 123)"}, res
        assert cli.calls[-1]["args"] == [settings.sai_link_bin, "daemon", "stop"], cli.calls[-1]
        assert cli.calls[-1]["cwd"] == root, cli.calls[-1]
        assert cli.calls[-1]["env"]["SAI_PROJECT_ROOT"] == root, cli.calls[-1]

        # 出力が空なら既定の文言
        cli.stdout = ""
        res = control.daemon("alpha", "start")
        assert res == {"ok": True, "message": "起動を要求しました"}, res
        assert cli.calls[-1]["args"] == [settings.sai_link_bin, "daemon", "start"], cli.calls[-1]

        # 失敗は 502 に stderr の最後の行（共有契約の読み替え）
        cli.returncode = 1
        cli.stderr = "実行中の作業があるため停止しませんでした"
        e = expect(502, lambda: control.daemon("alpha", "stop"))
        assert "実行中の作業" in str(e.detail), e.detail

        # action の検査と、未知のプロジェクト
        expect(400, lambda: control.daemon("alpha", "reload"))
        expect(404, lambda: control.daemon("unknown", "start"))
    finally:
        control.run_cli = original_run_cli
        control.find_project = original_find


def test_find_project_404(base: Path) -> None:
    """一覧に無い名前は 404。"""
    cli = FakeCli()
    original = control.run_cli
    control.run_cli = cli
    control._summary_cache.clear()
    try:
        expect(404, lambda: control.find_project("unknown"))
    finally:
        control.run_cli = original
        control._summary_cache.clear()


def main() -> None:
    with tempfile.TemporaryDirectory() as tmp:
        root_tmp = Path(tmp)
        base = root_tmp / "projects"
        home = root_tmp / "sai-home"
        base.mkdir(parents=True)
        make_registry(home, base)

        old_env = os.environ.get("SAI_HOME")
        os.environ["SAI_HOME"] = str(home)
        try:
            test_load_registry(base)
            test_load_registry_edges(root_tmp)
            test_daemon_alive_dead_pid(base)
            test_list_projects(base)
            test_summary_cache(base)
            test_summary_failure(base)
            test_dashboard(base)
            test_dashboard_no_db(base)
            test_daemon(base)
            test_find_project_404(base)
        finally:
            if old_env is None:
                os.environ.pop("SAI_HOME", None)
            else:
                os.environ["SAI_HOME"] = old_env
    print("test_registry_projects: すべて通りました")


main()
