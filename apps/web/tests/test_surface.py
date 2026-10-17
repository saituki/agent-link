"""Web の住所の一覧（見直しの基準）を固定する小さなテスト。

機能を変えない見直しの基準。住所を意図して足す・消すときだけ、EXPECTED を直す。

確かめること:
- `app.routes` から作った `"<METHOD> <path>"` の一覧（辞書順）が EXPECTED と一致する
  （`HEAD` は除く。`methods` を持たない物は `"MOUNT <path>"`）

実行:
    cd <作業用コピー>
    PYTHONPATH=apps/web/src apps/web/.venv/bin/python apps/web/tests/test_surface.py
"""

from __future__ import annotations

import os
import tempfile

# config.py は import のときに $AGENT_LINK_HOME/web を作るので、先に一時フォルダにする
os.environ["AGENT_LINK_HOME"] = tempfile.mkdtemp(prefix="agent-link-surface-")
os.environ["AGENT_LINK_WEB_NO_WATCH"] = "1"

from agent_link_web.main import app  # noqa: E402

EXPECTED: list[str] = [
    "DELETE /api/projects/{name}/defs/{kind}/{scope}/{def_name}",
    "DELETE /api/projects/{name}/runs/{run_id:int}",
    "GET /api/days",
    "GET /api/days/{date}",
    "GET /api/health",
    "GET /api/home",
    "GET /api/home/overview",
    "GET /api/projects",
    "GET /api/projects/{name}/card/docs",
    "GET /api/projects/{name}/card/options",
    "GET /api/projects/{name}/dashboard",
    "GET /api/projects/{name}/defs",
    "GET /api/projects/{name}/defs/{kind}/{scope}/{def_name}",
    "GET /api/projects/{name}/runs/{run_id:int}/history/calls/{call_id}",
    "GET /api/projects/{name}/runs/{run_id:int}/md/{kind}",
    "GET /api/projects/{name}/runs/{run_id:int}/view/{view}",
    "GET /api/projects/{name}/runs/{run_id:int}/waggle",
    "GET /api/projects/{name}/runs/{run_id:int}/waggle/{waggle_token}",
    "GET /api/projects/{name}/search",
    "GET /api/projects/{name}/sessions/search",
    "GET /api/projects/{name}/tree",
    "GET /api/projects/{name}/workflow",
    "GET /api/push/key",
    "GET /api/questions",
    "GET /api/questions/pending",
    "GET /api/questions/{qid}",
    "GET /api/runs/{project_name}/{run_id}",
    "GET /api/runs/{project_name}/{run_id}/items",
    "GET /api/usage",
    "GET /docs",
    "GET /docs/oauth2-redirect",
    "GET /manifest.json",
    "GET /openapi.json",
    "GET /redoc",
    "MOUNT ",
    "POST /api/projects/{name}/card",
    "POST /api/projects/{name}/daemon",
    "POST /api/projects/{name}/runs/{run_id:int}/advance",
    "POST /api/projects/{name}/runs/{run_id:int}/schedule",
    "POST /api/projects/{name}/runs/{run_id:int}/start",
    "POST /api/push/subscribe",
    "POST /api/questions",
    "POST /api/questions/{qid}/answer",
    "POST /api/questions/{qid}/resolved",
    "POST /api/questions/{qid}/withdraw",
    "POST /api/register",
    "POST /api/runs/{project_name}/{run_id}/finalize",
    "POST /api/runs/{project_name}/{run_id}/interrupt",
    "POST /api/runs/{project_name}/{run_id}/send",
    "PUT /api/projects/{name}/defs/{kind}/{scope}/{def_name}",
    "PUT /api/projects/{name}/runs/{run_id:int}/md/{kind}",
]


def surface() -> list[str]:
    lines = []
    for r in app.routes:
        methods = getattr(r, "methods", None)
        if methods:
            lines += [f"{m} {r.path}" for m in methods if m != "HEAD"]
        else:
            lines.append(f"MOUNT {r.path}")
    return sorted(lines)


def main() -> None:
    actual = surface()
    assert actual == EXPECTED, "住所の一覧が変わった:\n" + "\n".join(
        f'    "{x}",' for x in actual
    )
    print("ok: test_surface")


if __name__ == "__main__":
    main()
