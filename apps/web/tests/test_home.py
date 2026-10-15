"""ホームの集計（`GET /api/home`・U3 §変更内容 3）と合計のダッシュボード
（`GET /api/home/overview`・U02 §1）の小さなテスト（assert だけ）。

確かめること:
- 質問・承認の `project` / `run_id`（session_id を含む実行を探す）
- `title` / `summary` の取り方（1 番目の質問・command → file_path → tool_input の JSON）と 200 文字で切る
- `projects` の review（run_id と label の並び）と todo の数え方（`phone tree` のフォルダ）・`todo_runs`
- `papers` の項目が無いこと
- 1 プロジェクトの失敗が `errors` に入り、ほかのプロジェクトは出ること
- 10 秒の覚え置き（2 回目は集めない）
- 合計（`build_overview`）は各プロジェクトのダッシュボードを足し合わせること（U02 §1）

実行:
    cd <作業用コピー>
    PYTHONPATH=src .venv/bin/python tests/test_home.py
"""

from __future__ import annotations

import json

from fastapi import HTTPException

from agent_link_web import home
from agent_link_web.main import api_home_overview, app

# --- 偽の値（登録簿・tree・runs・store） ---

PROJECTS = [
    {
        "name": "agent-link-web",
        "root": "/tmp/agent-link-web",
        "root_exists": True,
        "db_exists": True,
        "daemon_alive": True,
        "running": 1,
        "last_executed_at": "2026-09-26 10:12:03",
    },
    {
        "name": "agent-link",
        "root": "/tmp/agent-link",
        "root_exists": True,
        "db_exists": True,
        "daemon_alive": False,
        "running": 0,
        "last_executed_at": "2026-09-25 09:00:00",
    },
    # DB が無い・フォルダが無いプロジェクトは.projects に出さない
    {"name": "fresh", "root": "/tmp/fresh", "root_exists": True, "db_exists": False},
    {"name": "gone", "root": "", "root_exists": False, "db_exists": True},
]

# `agent-link phone tree` の返答（トップの todo / running / review / done フォルダ）
TREE = {
    "items": [
        {
            "type": "folder",
            "id": "todo",
            "label": "todo",
            "children": [
                {"type": "run", "run_id": 56, "label": "ID56/kantoku", "lane": "todo"},
                {"type": "run", "run_id": 57, "label": "ID57/👷 worker", "lane": "todo"},
            ],
        },
        {
            "type": "folder",
            "id": "running",
            "label": "running",
            "children": [
                {"type": "run", "run_id": 55, "label": "ID55/📋 plan", "lane": "running"}
            ],
        },
        {
            "type": "folder",
            "id": "review",
            "label": "review",
            "children": [
                {"type": "run", "run_id": 54, "label": "ID54/📋 plan", "lane": "review"}
            ],
        },
        {"type": "folder", "id": "done", "label": "done", "children": []},
    ]
}

# `agent-link phone dashboard --period week` の返答（2 プロジェクト分）
DASHBOARDS = {
    "agent-link-web": {
        "name": "agent-link-web",
        "period": "week",
        "since": "2026-09-21",
        "until": "2026-09-27",
        "daemon_alive": True,
        "summary": {
            "executions": 4,
            "completed": 3,
            "failed": 1,
            "running": 0,
            "success_rate": 0.75,
            "duration_ms": 1000,
            "tokens_input": 10,
            "tokens_output": 5,
            "tokens_cache": 0,
            "tokens_total": 15,
            "cost_usd": 0.5,
        },
        "daily": [
            {"date": "2026-09-21", "executions": 1, "tokens": 5},
            {"date": "2026-09-22", "executions": 3, "tokens": 10},
        ],
        "recent": [
            {
                "run_id": 55,
                "parent_run_id": None,
                "block": "worker",
                "status": "completed",
                "started_at": "2026-09-27 10:28:21",
                "description": "U02 ホーム",
                "linkable": True,
                "project": "agent-link-web",
            },
            # プロジェクト名が無い行（登録簿の名前を入れる）と書き出しが無い行
            {
                "run_id": 54,
                "parent_run_id": 50,
                "block": "plan",
                "status": "failed",
                "started_at": "2026-09-26 09:00:00",
                "description": "",
                "linkable": False,
            },
        ],
    },
    "agent-link": {
        "name": "agent-link",
        "period": "week",
        # 1 プロジェクトだけ一時的に昔までさかのぼっている（since〜until のまとめ方）
        "since": "2026-09-20",
        "until": "2026-09-27",
        "daemon_alive": False,
        "summary": {
            "executions": 2,
            "completed": 1,
            "failed": 0,
            "running": 1,
            "success_rate": 0.5,
            "duration_ms": 2000,
            "tokens_input": 1,
            "tokens_output": 1,
            "tokens_cache": 0,
            "tokens_total": 2,
            "cost_usd": 1.25,
        },
        "daily": [
            {"date": "2026-09-22", "executions": 1, "tokens": 7},
            {"date": "2026-09-27", "executions": 1, "tokens": 9},
        ],
        "recent": [
            # 同じ時刻（run_id の大きい方が先）
            {
                "run_id": 9,
                "parent_run_id": None,
                "block": "kantoku",
                "status": "running",
                "started_at": "2026-09-27 10:28:21",
                "description": "kantoku",
                "linkable": True,
            }
        ],
    },
}

RUNS = [
    {
        "project_name": "agent-link-web",
        "run_id": 55,
        "agent_session_id": "sess-55",
        "transcript_path": "",
    },
    {
        "project_name": "agent-link",
        "run_id": 20,
        "agent_session_id": "sess-20",
        "transcript_path": "",
    },
]

QUESTIONS = [
    {
        "id": "q-1",
        "agent": "cc",
        "session_id": "sess-55",
        "created_at": 100.0,
        "questions": [{"question": "どちらにしますか？"}, {"question": "あとで"}],
    },
    # 実行の無いセッション・質問が空
    {"id": "q-2", "agent": "pi", "session_id": "unknown", "created_at": 90.0, "questions": []},
]


class Fake:
    """`control` / `runs` / `store` の呼び出し記録と、差し替える対象。"""

    def __init__(self) -> None:
        self.tree_calls: list[str] = []
        self.dashboard_calls: list[tuple[str, str]] = []
        self.run_calls: int = 0
        self.fail_tree: set[str] = set()
        self.fail_dashboard: set[str] = set()


def setup(fake: Fake) -> None:
    """home が使う関数だけを偽物に差し替える（他は本物）。"""
    # 呼び出しの記録はテストごとに初期化する
    fake.tree_calls.clear()
    fake.dashboard_calls.clear()
    fake.run_calls = 0
    fake.fail_tree = set()
    fake.fail_dashboard = set()

    def list_projects():
        return [dict(p) for p in PROJECTS]

    def phone_tree(name: str) -> dict:
        fake.tree_calls.append(name)
        if name in fake.fail_tree:
            raise HTTPException(status_code=502, detail="agent-link が失敗しました")
        return json.loads(json.dumps(TREE))

    def load_runs():
        fake.run_calls += 1
        return [dict(r) for r in RUNS]

    def run_session_ids(run: dict) -> set[str]:
        return {run["agent_session_id"]}

    def dashboard(name: str, period: str) -> dict:
        fake.dashboard_calls.append((name, period))
        if name in fake.fail_dashboard:
            raise HTTPException(status_code=502, detail="dashboard が失敗しました")
        return json.loads(json.dumps(DASHBOARDS[name]))

    home.control.list_projects = list_projects
    home.control.phone_tree = phone_tree
    home.control.dashboard = dashboard
    home.runs.load_runs = load_runs
    home.runs.run_session_ids = run_session_ids
    home.store.list_open_questions = lambda: [dict(q) for q in QUESTIONS]
    home.forget()


# --- 確かめること ---


def test_shape(fake: Fake) -> None:
    """全体の形（質問・プロジェクト・errors の 3 つ。`papers` は無い）。"""
    setup(fake)
    out = home.build_home()
    assert set(out) == {"questions", "projects", "errors"}, set(out)
    assert out["errors"] == [], out["errors"]


def test_questions(fake: Fake) -> None:
    """質問の題と、session_id を含む実行の対応づけ。"""
    setup(fake)
    out = home.build_home()
    qs = out["questions"]
    assert len(qs) == 2, qs
    first = qs[0]
    assert first["id"] == "q-1", first
    assert first["agent"] == "cc", first
    assert first["title"] == "どちらにしますか？", first
    assert first["created_at"] == 100.0, first
    assert first["project"] == "agent-link-web", first
    assert first["run_id"] == 55, first
    # 実行が無い質問は null（無ければ null にする）
    second = qs[1]
    assert second["title"] == "質問", second
    assert second["project"] is None, second
    assert second["run_id"] is None, second


def test_title_truncated(fake: Fake) -> None:
    """題は 200 文字で切る。"""
    setup(fake)
    long_q = {"id": "q-x", "agent": "cc", "session_id": "sess-55", "questions": [{"question": "あ" * 500}]}
    home.store.list_open_questions = lambda: [long_q]
    home.forget()
    out = home.build_home()
    assert len(out["questions"][0]["title"]) == 200, len(out["questions"][0]["title"])


def test_projects(fake: Fake) -> None:
    """プロジェクトは root と DB が両方ある物だけ。review と todo の数え方も確かめる。"""
    setup(fake)
    out = home.build_home()
    rows = out["projects"]
    assert [r["name"] for r in rows] == ["agent-link-web", "agent-link"], rows
    assert fake.tree_calls == ["agent-link-web", "agent-link"], fake.tree_calls
    first = rows[0]
    assert first["daemon_alive"] is True, first
    assert first["running"] == 1, first
    assert first["last_executed_at"] == "2026-09-26 10:12:03", first
    assert first["review"] == [{"run_id": 54, "label": "ID54/📋 plan"}], first
    assert first["todo"] == 2, first  # todo フォルダの run は 2 件
    # todo の行そのものも同じ内容で返す（画面の todo カードが押せるようにするため・U02 §2）
    assert first["todo_runs"] == [
        {"run_id": 56, "label": "ID56/kantoku"},
        {"run_id": 57, "label": "ID57/👷 worker"},
    ], first
    assert rows[1]["daemon_alive"] is False, rows[1]
    assert rows[1]["running"] == 0, rows[1]


def test_folder_runs_passes_card() -> None:
    """todo の行に `card`（task.md の項目）があるときだけ、文字にして返す。"""
    items = [
        {
            "type": "folder",
            "label": "todo",
            "children": [
                {"type": "run", "run_id": 9, "label": "ID9/plan", "card": {"creation_date": "2026/09/30 08:25:12", "block": "plan", "runner": "cc", "model": "opus", "description": "議論"}},
                {"type": "run", "run_id": 10, "label": "ID10/sub"},
            ],
        }
    ]
    rows = home._folder_runs(items, "todo")
    assert rows[0]["card"] == {"creation_date": "2026/09/30 08:25:12", "block": "plan", "runner": "cc", "model": "opus", "description": "議論"}, rows
    assert "card" not in rows[1], rows


def test_folder_runs_passes_scheduled_at() -> None:
    """todo の行に `scheduled_at` があるときだけ、そのまま返す（予約したもの用）。"""
    items = [
        {
            "type": "folder",
            "label": "todo",
            "children": [
                {"type": "run", "run_id": 1, "label": "a", "scheduled_at": "2026-10-01 09:00"},
                {"type": "run", "run_id": 2, "label": "b"},
            ],
        }
    ]
    rows = home._folder_runs(items, "todo")
    assert rows == [
        {"run_id": 1, "label": "a", "scheduled_at": "2026-10-01 09:00"},
        {"run_id": 2, "label": "b"},
    ], rows


def test_errors(fake: Fake) -> None:
    """1 プロジェクトの失敗は errors に入り、ほかのプロジェクトは出る。"""
    setup(fake)
    fake.fail_tree = {"agent-link-web"}
    out = home.build_home()
    names = [r["name"] for r in out["projects"]]
    assert names == ["agent-link"], names
    assert out["errors"] == ["agent-link-web: agent-link が失敗しました"], out["errors"]


def test_cache(fake: Fake) -> None:
    """10 秒だけ覚える（2 回目は集めない）。"""
    setup(fake)
    first = home.build_home()
    tree_calls = len(fake.tree_calls)
    assert tree_calls == 2, tree_calls
    second = home.build_home()
    assert second is first, "覚え置いてある間は集めない"
    assert len(fake.tree_calls) == tree_calls, fake.tree_calls
    # 10 秒経ったら集め直す
    for k in list(home._cache):
        home._cache[k] = (home._cache[k][0] - 11.0, home._cache[k][1])
    third = home.build_home()
    assert third is not first, "10 秒経ったら集め直す"
    assert len(fake.tree_calls) == tree_calls + 2, fake.tree_calls


def test_route() -> None:
    """`GET /api/home` と `GET /api/home/overview` が登録されている。"""
    got = set()
    for route in app.routes:
        for method in getattr(route, "methods", []) or []:
            got.add((method, getattr(route, "path", "")))
    assert ("GET", "/api/home") in got, sorted(got)
    assert ("GET", "/api/home/overview") in got, sorted(got)


# --- 合計のダッシュボード（U02 §1） ---


def test_overview_sum(fake: Fake) -> None:
    """2 プロジェクトのダッシュボードを足し合わせる。"""
    setup(fake)
    out = home.build_overview("week")
    assert set(out) == {
        "period",
        "since",
        "until",
        "projects",
        "summary",
        "daily",
        "recent",
        "errors",
    }, set(out)
    assert out["period"] == "week", out
    assert out["errors"] == [], out["errors"]
    # DB が無い・フォルダが無いプロジェクトは呼ばない
    assert sorted(fake.dashboard_calls) == [("agent-link", "week"), ("agent-link-web", "week")], fake.dashboard_calls
    assert out["projects"] == 2, out
    s = out["summary"]
    assert s["executions"] == 6, s
    assert s["completed"] == 4, s
    assert s["failed"] == 1, s
    assert s["running"] == 1, s
    assert s["duration_ms"] == 3000, s
    assert s["tokens_total"] == 17, s
    assert s["cost_usd"] == 1.75, s
    # 成功率は足したあとに求め直す
    assert s["success_rate"] == round(4 / 6, 4), s
    # since〜until は全プロジェクトをまたいだ最早と最遅
    assert out["since"] == "2026-09-20", out
    assert out["until"] == "2026-09-27", out


def test_overview_daily(fake: Fake) -> None:
    """日ごとは日付ごとに足して古い順に並べる。"""
    setup(fake)
    out = home.build_overview("week")
    assert out["daily"] == [
        {"date": "2026-09-21", "executions": 1, "tokens": 5},
        {"date": "2026-09-22", "executions": 4, "tokens": 17},
        {"date": "2026-09-27", "executions": 1, "tokens": 9},
    ], out["daily"]


def test_overview_recent(fake: Fake) -> None:
    """最近の実行は新しい順（同じ時刻なら run_id の大きい順）・プロジェクト名を埋める。"""
    setup(fake)
    out = home.build_overview("week")
    got = [(r["project"], r["run_id"], r["block"], r["status"], r["linkable"])
           for r in out["recent"]]
    assert got == [
        ("agent-link-web", 55, "worker", "completed", True),
        ("agent-link", 9, "kantoku", "running", True),
        ("agent-link-web", 54, "plan", "failed", False),
    ], out["recent"]
    assert out["recent"][0]["description"] == "U02 ホーム", out["recent"][0]
    assert out["recent"][0]["started_at"] == "2026-09-27 10:28:21", out["recent"][0]


def test_overview_recent_limit(fake: Fake) -> None:
    """最近の実行は 20 件まで。"""
    setup(fake)
    many = dict(DASHBOARDS["agent-link-web"])
    many["recent"] = [
        {
            "run_id": i,
            "parent_run_id": None,
            "block": "worker",
            "status": "completed",
            "started_at": f"2026-09-27 12:{i:02d}:00",
            "description": f"R{i}",
            "linkable": True,
        }
        for i in range(25)
    ]
    home.control.dashboard = lambda name, period: json.loads(json.dumps(many if name == "agent-link-web" else DASHBOARDS[name]))
    home.forget()
    out = home.build_overview("week")
    assert len(out["recent"]) == 20, len(out["recent"])
    assert out["recent"][0]["run_id"] == 24, out["recent"][0]
    assert out["recent"][-1]["run_id"] == 5, out["recent"][-1]


def test_overview_errors(fake: Fake) -> None:
    """1 プロジェクトが失敗しても errors に入り、ほかのプロジェクトは返る。"""
    setup(fake)
    fake.fail_dashboard = {"agent-link-web"}
    out = home.build_overview("week")
    assert out["errors"] == ["agent-link-web: dashboard が失敗しました"], out["errors"]
    assert out["projects"] == 1, out
    assert out["summary"]["executions"] == 2, out["summary"]
    assert all(r["project"] == "agent-link" for r in out["recent"]), out["recent"]


def test_overview_bad_period(fake: Fake) -> None:
    """period が違えば 400（CLI を 1 度も呼ばない）。"""
    setup(fake)
    try:
        home.build_overview("bad")
    except HTTPException as e:
        assert e.status_code == 400, e.status_code
        assert e.detail == "period は week / month / all のいずれかにしてください", e.detail
    else:
        raise AssertionError("period=bad で 400 が返らなかった")
    # 住所の関数越しでも同じ（/?period=bad は 400 になる）
    try:
        api_home_overview(period="bad")
    except HTTPException as e:
        assert e.status_code == 400, e.status_code
    else:
        raise AssertionError("period=bad で 400 が返らなかった")
    assert fake.dashboard_calls == [], fake.dashboard_calls


def test_overview_cache(fake: Fake) -> None:
    """合計も期間ごとに 10 秒だけ覚える。"""
    setup(fake)
    first = home.build_overview("week")
    assert len(fake.dashboard_calls) == 2, fake.dashboard_calls
    second = home.build_overview("week")
    assert second is first, "覚え置いてある間は集めない"
    assert len(fake.dashboard_calls) == 2, fake.dashboard_calls
    # 期間を変えても別の結果を覚える
    home.build_overview("month")
    assert len(fake.dashboard_calls) == 4, fake.dashboard_calls
    # forget() で消す
    home.forget()
    home.build_overview("week")
    assert len(fake.dashboard_calls) == 6, fake.dashboard_calls


def main() -> None:
    fake = Fake()
    test_shape(fake)
    test_questions(fake)
    test_title_truncated(fake)
    test_projects(fake)
    test_folder_runs_passes_card()
    test_folder_runs_passes_scheduled_at()
    test_errors(fake)
    test_cache(fake)
    test_route()
    test_overview_sum(fake)
    test_overview_daily(fake)
    test_overview_recent(fake)
    test_overview_recent_limit(fake)
    test_overview_errors(fake)
    test_overview_bad_period(fake)
    test_overview_cache(fake)
    print("test_home: すべて通りました")


main()
