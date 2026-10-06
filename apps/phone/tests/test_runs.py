"""runs.py の日付まとめ処理の小さなテスト（assert だけ）・02 §5．

実行:
    PYTHONPATH=src .venv/bin/python tests/test_runs.py
    （または PYTHONPATH=src python3 -m pytest tests/test_runs.py）
"""

from __future__ import annotations

import json
import sys
import tempfile
from pathlib import Path

# リポジトリ直下から実行しなくても import できるようにしておく
sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "src"))

from sai_phone import runs  # noqa: E402


def make_run(
    run_id: int,
    *,
    started_at: str,
    status: str = "completed",
    runner: str = "cc",
    parent_run_id: int | None = None,
    stats: dict | None = None,
    project_name: str = "sai-phone",
    **extra,
) -> dict:
    """00 §3.2 の形のダミー実行を作る．"""
    run = {
        "schema_version": 1,
        "project_name": project_name,
        "project_root": "/Users/alice/work/sai-phone",
        "run_id": run_id,
        "parent_run_id": parent_run_id,
        "block_name": "sai2",
        "description": f"タスク{run_id}",
        "runner": runner,
        "status": status,
        "model": None,
        "started_at": started_at,
        "finished_at": None,
        "duration_ms": None,
        "herdr_pane_label": f"sai-sai2-{run_id}",
        "herdr_pane_id": None,
        "agent_session_id": None,
        "transcript_path": None,
        "task_md_path": None,
        "updated_at": started_at,
        "stats": stats,
    }
    run.update(extra)
    return run


def test_jst_date_boundary() -> None:
    """UTC 2026-09-22T23:30:00Z の実行は日本時間の 2026-09-23 に入る（00 §3.6）．"""
    run = make_run(1, started_at="2026-09-22T23:30:00Z")
    assert runs.run_date(run) == "2026-09-23"
    assert runs.run_time(run) == "08:30"

    days = runs.build_days([run])
    assert len(days) == 1
    assert days[0]["date"] == "2026-09-23"
    assert days[0]["weekday"] == "Wed"  # 2026-09-23 は水曜日

    # 日本時間で前の日の実行は別の日に入る
    prev = make_run(2, started_at="2026-09-22T15:00:00Z")  # JST 2026-09-23 00:00 ちょうど
    assert runs.run_date(prev) == "2026-09-23"
    prev2 = make_run(3, started_at="2026-09-22T14:59:00Z")  # JST 2026-09-22 23:59
    assert runs.run_date(prev2) == "2026-09-22"


def test_done_counts_and_cost() -> None:
    """done の runner 別件数・running・failed・コスト合計（02 §1 の1番）．"""
    day = [
        make_run(
            1,
            started_at="2026-09-23T01:00:00Z",
            status="completed",
            runner="cc",
            stats={"cost_usd": {"total": 1.25}, "cost_is_estimate": False},
        ),
        make_run(
            2,
            started_at="2026-09-23T02:00:00Z",
            status="completed",
            runner="cc",
            stats={"cost_usd": {"total": 0.25}, "cost_is_estimate": True},
        ),
        make_run(3, started_at="2026-09-23T03:00:00Z", status="completed", runner="pi"),
        make_run(4, started_at="2026-09-23T04:00:00Z", status="running", runner="cc"),
        make_run(5, started_at="2026-09-23T05:00:00Z", status="failed", runner="pi"),
    ]
    days = runs.build_days(day)
    assert len(days) == 1
    d = days[0]
    assert d["done"] == {"cc": 2, "pi": 1}
    assert d["running"] == 1
    assert d["failed"] == 1
    assert abs(d["cost_usd"] - 1.5) < 1e-9
    # 概算の実行が1つでも混ざれば ≈ を付ける（02 §1 の1番）
    assert d["cost_is_estimate"] is True

    # 0 の runner は done に入らない
    assert "gl" not in d["done"]

    # もう一日（実行中だけ・stats は null）は running のみが出る
    other = runs.build_days([make_run(9, started_at="2026-09-24T00:00:00Z", status="running")])
    assert other[0]["done"] == {}
    assert other[0]["running"] == 1
    assert other[0]["cost_usd"] == 0.0
    assert other[0]["cost_is_estimate"] is False
    # 新しい日が先
    assert days[0]["date"] == "2026-09-23"
    assert other[0]["date"] == "2026-09-24"


def test_subtask_under_parent() -> None:
    """サブタスクは親の直下に字下げして並ぶ（02 §1 の2番）．"""
    parent = make_run(10, started_at="2026-09-23T00:00:00Z")
    other = make_run(12, started_at="2026-09-23T01:00:00Z")
    # サブタスクは親より後（時刻だけなら先頭）に始まっても親の直下に来る
    child = make_run(11, started_at="2026-09-23T02:00:00Z", parent_run_id=10)

    ordered = runs.order_runs([child, other, parent])
    ids = [r["run_id"] for r in ordered]
    assert ids.index(11) == ids.index(10) + 1  # 親の直下
    assert ordered[ids.index(11)]["indent"] is True
    assert ordered[ids.index(11)]["show_parent"] is False
    # 根（親が居ない行）同士は新しい順
    assert ids.index(12) < ids.index(10)

    # 親がその日に居ないサブタスクは普通に並べて ↳ #親ID を添える
    day = runs.build_day([child], "2026-09-23")
    rows = day["projects"][0]["runs"]
    assert len(rows) == 1
    assert rows[0]["run_id"] == 11
    assert rows[0]["indent"] is False
    assert rows[0]["show_parent"] is True
    assert rows[0]["parent_run_id"] == 10


def test_load_runs_filters() -> None:
    """`.` 始まり・壊れた JSON・schema 違い・user 実行は読み飛ばす（02 §2.1）．"""
    with tempfile.TemporaryDirectory() as tmp:
        base = Path(tmp)
        proj = base / "sai-phone"
        proj.mkdir()
        ok = make_run(1, started_at="2026-09-23T00:00:00Z")
        (proj / "1.json").write_text(json.dumps(ok), encoding="utf-8")
        # 01 の一時ファイル（先頭ドット）は無視する
        (proj / ".2.json.tmp").write_text(json.dumps(ok), encoding="utf-8")
        # schema_version が 1 以外は飛ばす
        v2 = dict(ok, run_id=3, schema_version=2)
        (proj / "3.json").write_text(json.dumps(v2), encoding="utf-8")
        # 壊れた JSON は飛ばす
        (proj / "4.json").write_text("{ broken", encoding="utf-8")
        # user の実行は一覧に出さない（00 §3.2）
        user = dict(ok, run_id=5, runner="user")
        (proj / "5.json").write_text(json.dumps(user), encoding="utf-8")
        # 未実行（pending）は出さない
        pending = dict(ok, run_id=6, status="pending")
        (proj / "6.json").write_text(json.dumps(pending), encoding="utf-8")
        # `.` 始まりのフォルダも無視
        (base / ".trash").mkdir()
        (base / ".trash" / "7.json").write_text(json.dumps(ok), encoding="utf-8")

        got = runs.load_runs(base)
        assert [r["run_id"] for r in got] == [1]

        # ファイルを消したらいったん読まれなくなる（キャッシュに引きずられない）
        (proj / "1.json").unlink()
        assert runs.load_runs(base) == []


def test_pane_only_for_running_run() -> None:
    """終わった実行・pane_id 不一致では，同じ名前の別プロジェクトのペインに当てない．"""
    panes = [{"pane_id": "w1:p1", "label": "sai-main-7"}]
    running = {"status": "running", "herdr_pane_label": "sai-main-7"}
    assert runs.pane_for_run(running, [], panes) == panes[0]
    assert runs.pane_for_run({**running, "status": "completed"}, [], panes) is None
    assert runs.pane_for_run({**running, "herdr_pane_id": "w9:p9"}, [], panes) is None


def test_pane_id_with_wrong_label() -> None:
    """ID が同じでもラベルの違うペインには送らない（計画書 A の U06）．

    herdr のペイン ID は番号で，ペインが消えると再利用される。
    番号が同じでラベルが違う＝別のプロジェクトの agent。
    """
    panes = [{"pane_id": "w1:p8", "label": "sai-main-84"}]
    run = {
        "status": "running",
        "herdr_pane_id": "w1:p8",
        "herdr_pane_label": "sai-main-11",
    }
    assert runs.pane_for_run(run, [], panes) is None
    # agent list の要素は label が無いが，panes 側のラベルで判断する
    agents = [{"pane_id": "w1:p8", "agent": "claude"}]
    assert runs.pane_for_run(run, agents, panes) is None
    # ラベルが一致すれば見つかる（panes の要素が返る）
    ok = {**run, "herdr_pane_label": "sai-main-84"}
    assert runs.pane_for_run(ok, [], panes) == panes[0]
    # ペインにラベルが無ければ今までどおり ID の一致だけで決める
    unlabelled = [{"pane_id": "w1:p8"}]
    assert runs.pane_for_run(run, [], unlabelled) == unlabelled[0]
    assert runs.pane_for_run(ok, agents, unlabelled) == agents[0]


def test_cursor_resets_on_file_change() -> None:
    """記録ファイルが変わったら，前のファイルの読み位置を使わず最初から読む．"""
    from sai_phone import sessions

    with tempfile.TemporaryDirectory() as tmp:
        path = Path(tmp) / "real.jsonl"
        path.write_text("x" * 100)
        assert sessions._decode_cursor("real.jsonl:40", path) == 40
        assert sessions._decode_cursor("main.jsonl:40", path) == 0


def main() -> None:
    test_jst_date_boundary()
    test_done_counts_and_cost()
    test_subtask_under_parent()
    test_load_runs_filters()
    test_pane_only_for_running_run()
    test_pane_id_with_wrong_label()
    test_cursor_resets_on_file_change()
    print("test_runs: すべて通りました")


if __name__ == "__main__":
    main()
