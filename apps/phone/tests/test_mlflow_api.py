"""MLflow の読み取り中継の小さなテスト（assert だけ・U03）。

確かめること:
- 検査関数は正しい値を通し、違う値を 400 と日本語の理由で断る（AC1）
- `project` を付けた runs/search は送る filter に `tags."sai.project"` を足す（AC2）
- 偽の MLflow（`http.server`）に対して本文がそのまま返り、
  400 は 400 と本文がそのまま、止まった住所は 503、成果物は 5MB を超えたら 413（AC3）

本物の MLflow（5001 番）と本物の CLI は呼ばない。偽のものを立てるだけ。

実行:
    cd <作業用コピー>
    PYTHONPATH=src <venv>/bin/python tests/test_mlflow_api.py
"""

from __future__ import annotations

import json
import os
import sys
import tempfile
import threading
from dataclasses import replace
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from urllib.parse import urlparse

# リポジトリ直下から実行しなくても import できるようにしておく
sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "src"))

from fastapi import HTTPException  # noqa: E402

from sai_phone import control, mlflow_proxy  # noqa: E402
from sai_phone.config import settings  # noqa: E402

# 32 桁の 16 進（正しい run_id）
RUN_ID = "0123456789abcdef0123456789abcdef"
# 6MB の本文（大きすぎる成果物の確認に使う）
BIG_SIZE = 6 * 1024 * 1024


def expect_error(fn, status: int = 400) -> HTTPException:
    """fn は指定した状態番号の HTTPException で失敗し、日本語の理由が付くこと。"""
    try:
        fn()
    except HTTPException as e:
        assert e.status_code == status, f"{status} のはずが {e.status_code}: {e.detail}"
        assert str(e.detail).strip(), f"{status} のときは日本語で理由を書く"
        return e
    raise AssertionError(f"{status} になるはず")


# --- AC1: 検査関数 --------------------------------------------------


def test_accepts() -> None:
    """正しい値はそのまま返す。"""
    assert mlflow_proxy.check_experiment_id("1") == "1"
    assert mlflow_proxy.check_run_id(RUN_ID) == RUN_ID
    assert mlflow_proxy.check_metric_key("loss") == "loss"
    assert mlflow_proxy.check_artifact_path("") == ""
    assert mlflow_proxy.check_artifact_path("model/model.pkl") == "model/model.pkl"
    assert mlflow_proxy.check_max_results(None, 100) == 100
    assert mlflow_proxy.check_max_results(1, 100) == 1
    assert mlflow_proxy.check_max_results(100, 100) == 100


def test_experiment_id_400() -> None:
    """実験 ID は数字の 1〜19 桁（12a や空は 400）。"""
    expect_error(lambda: mlflow_proxy.check_experiment_id("12a"))
    expect_error(lambda: mlflow_proxy.check_experiment_id(""))
    expect_error(lambda: mlflow_proxy.check_experiment_id("1" * 20))
    expect_error(lambda: mlflow_proxy.check_experiment_id("-1"))
    expect_error(lambda: mlflow_proxy.check_experiment_id(1))


def test_run_id_400() -> None:
    """run ID は 16 進の 32 桁（大文字や 31 桁は 400）。"""
    expect_error(lambda: mlflow_proxy.check_run_id(RUN_ID.upper()))
    expect_error(lambda: mlflow_proxy.check_run_id(RUN_ID[:-1]))
    expect_error(lambda: mlflow_proxy.check_run_id(RUN_ID + "0"))
    expect_error(lambda: mlflow_proxy.check_run_id(""))
    expect_error(lambda: mlflow_proxy.check_run_id("0123456789abcdef0123456789abcdeg"))


def test_metric_key_400() -> None:
    """指標の名前は 1〜250 字・制御文字なし。"""
    expect_error(lambda: mlflow_proxy.check_metric_key(""))
    expect_error(lambda: mlflow_proxy.check_metric_key("a" * 251))
    expect_error(lambda: mlflow_proxy.check_metric_key("a\nb"))
    expect_error(lambda: mlflow_proxy.check_metric_key("a\x00b"))


def test_artifact_path_400() -> None:
    """成果物の場所は 0〜500 字・先頭の / なし・.. の区切りなし。"""
    expect_error(lambda: mlflow_proxy.check_artifact_path("../x"))
    expect_error(lambda: mlflow_proxy.check_artifact_path("model/../../x"))
    expect_error(lambda: mlflow_proxy.check_artifact_path("/etc"))
    expect_error(lambda: mlflow_proxy.check_artifact_path("/etc/passwd"))
    expect_error(lambda: mlflow_proxy.check_artifact_path("a\x00b"))
    expect_error(lambda: mlflow_proxy.check_artifact_path("a" * 501))
    # 途中が `..x` のような物は通す（区切りが `..` の物だけを止める）
    assert mlflow_proxy.check_artifact_path("model/..x/a") == "model/..x/a"


def test_max_results_400() -> None:
    """取る個数は 1〜top。0 と top+1 は 400。"""
    expect_error(lambda: mlflow_proxy.check_max_results(0, 100))
    expect_error(lambda: mlflow_proxy.check_max_results(101, 100))
    expect_error(lambda: mlflow_proxy.check_max_results(-1, 100))
    expect_error(lambda: mlflow_proxy.check_max_results(2501, 2500))
    expect_error(lambda: mlflow_proxy.check_max_results("10", 100))
    expect_error(lambda: mlflow_proxy.check_max_results(True, 100))


# --- AC2: project の絞り込み ----------------------------------------


def use_registry(tmp: str) -> None:
    """一時フォルダの登録簿を，使う（`SAI_SAI_HOME` で差し替える）。"""
    home = Path(tmp)
    (home / "demo-root").mkdir(parents=True, exist_ok=True)
    rows = [
        {
            "kind": "link",
            "name": "demo",
            "root": str(home / "demo-root"),
            "last_opened": "2026-09-29 10:00:00",
        }
    ]
    (home / "registry.json").write_text(json.dumps(rows), encoding="utf-8")
    os.environ["SAI_SAI_HOME"] = str(home)
    control._summary_cache.clear()


def test_project_filter() -> None:
    """`project` が来たら `tags."sai.project"` を filter の前に足す（U03 §決定事項）。"""
    with tempfile.TemporaryDirectory() as tmp:
        use_registry(tmp)
        sent: list[dict] = []
        original = mlflow_proxy._call

        def fake_call(method, api_path, query=None, body=None, timeout=30.0):
            sent.append(
                {"method": method, "api_path": api_path, "body": body, "query": query}
            )
            return json.dumps({"runs": [], "next_page_token": None}).encode("utf-8")

        mlflow_proxy._call = fake_call  # type: ignore
        try:
            # 元の絞り込みが無いときは前半だけ
            mlflow_proxy.runs_search({"experiment_ids": ["1"], "project": "demo"})
            assert sent[-1]["body"]["filter"] == 'tags."sai.project" = "demo-root"', sent[-1]
            # 元の絞り込みがあれば AND でつなぐ
            mlflow_proxy.runs_search(
                {"experiment_ids": ["1"], "filter": 'params."a" = "1"', "project": "demo"}
            )
            assert sent[-1]["body"]["filter"] == (
                'tags."sai.project" = "demo-root" AND params."a" = "1"'
            ), sent[-1]
            # project が無ければ足さない
            mlflow_proxy.runs_search({"experiment_ids": ["1"], "filter": 'params."a" = "1"'})
            assert sent[-1]["body"]["filter"] == 'params."a" = "1"', sent[-1]
            # 始めからある条件もそのまま送る
            mlflow_proxy.runs_search({"experiment_ids": ["1"]})
            assert sent[-1]["body"]["filter"] == "", sent[-1]
            # run_view_type は固定
            assert sent[-1]["body"]["run_view_type"] == "ACTIVE_ONLY", sent[-1]
            # max_results の既定は 100
            assert sent[-1]["body"]["max_results"] == 100, sent[-1]
            assert sent[-1]["api_path"] == "/api/2.0/mlflow/runs/search", sent[-1]
        finally:
            mlflow_proxy._call = original  # type: ignore
        os.environ.pop("SAI_SAI_HOME", None)


def test_project_unknown_404() -> None:
    """登録簿に無いプロジェクトは 404（MLflow は呼ばない）。"""
    with tempfile.TemporaryDirectory() as tmp:
        use_registry(tmp)
        sent: list[dict] = []
        original = mlflow_proxy._call

        def fake_call(method, api_path, query=None, body=None, timeout=30.0):
            sent.append(
                {"method": method, "api_path": api_path, "body": body, "query": query}
            )
            return b'{"runs": []}'

        mlflow_proxy._call = fake_call  # type: ignore
        try:
            expect_error(
                lambda: mlflow_proxy.runs_search(
                    {"experiment_ids": ["1"], "project": "not-there"}
                ),
                status=404,
            )
            assert sent == [], "登録簿に無い時は MLflow を呼ばない"
        finally:
            mlflow_proxy._call = original  # type: ignore
        os.environ.pop("SAI_SAI_HOME", None)


def test_experiments_project() -> None:
    """実験の一覧は `project` で experiment のタグを絞る。無い名前は 404 で MLflow を呼ばない。"""
    with tempfile.TemporaryDirectory() as tmp:
        use_registry(tmp)
        sent: list[dict] = []
        original = mlflow_proxy._call

        def fake_call(method, api_path, query=None, body=None, timeout=30.0):
            sent.append({"api_path": api_path, "body": body})
            return b'{"experiments": []}'

        mlflow_proxy._call = fake_call  # type: ignore
        try:
            mlflow_proxy.experiments("demo")
            assert sent[-1]["api_path"] == "/api/2.0/mlflow/experiments/search", sent[-1]
            assert sent[-1]["body"]["filter"] == 'tags."sai.project" = "demo-root"', sent[-1]
            assert sent[-1]["body"]["order_by"] == ["name ASC"], sent[-1]
            mlflow_proxy.experiments()
            assert "filter" not in sent[-1]["body"], sent[-1]
            n = len(sent)
            expect_error(lambda: mlflow_proxy.experiments("not-there"), status=404)
            assert len(sent) == n, "登録簿に無い時は MLflow を呼ばない"
        finally:
            mlflow_proxy._call = original  # type: ignore
        os.environ.pop("SAI_SAI_HOME", None)


def test_runs_search_400() -> None:
    """本文の型と各項目を検査し、通らない値は MLflow を呼ばない。"""
    sent: list[dict] = []
    original = mlflow_proxy._call

    def fake_call(method, api_path, query=None, body=None, timeout=30.0):
        sent.append({"method": method, "api_path": api_path, "body": body, "query": query})
        return b'{"runs": []}'

    mlflow_proxy._call = fake_call  # type: ignore
    try:
        expect_error(lambda: mlflow_proxy.runs_search("x"))
        expect_error(lambda: mlflow_proxy.runs_search({}))
        expect_error(lambda: mlflow_proxy.runs_search({"experiment_ids": []}))
        expect_error(lambda: mlflow_proxy.runs_search({"experiment_ids": ["12a"]}))
        expect_error(
            lambda: mlflow_proxy.runs_search({"experiment_ids": [str(i) for i in range(51)]})
        )
        expect_error(
            lambda: mlflow_proxy.runs_search({"experiment_ids": ["1"], "max_results": 101})
        )
        expect_error(
            lambda: mlflow_proxy.runs_search({"experiment_ids": ["1"], "max_results": 0})
        )
        expect_error(
            lambda: mlflow_proxy.runs_search({"experiment_ids": ["1"], "order_by": ["a"] * 4})
        )
        expect_error(
            lambda: mlflow_proxy.runs_search(
                {"experiment_ids": ["1"], "order_by": ["a" * 201]}
            )
        )
        expect_error(
            lambda: mlflow_proxy.runs_search(
                {"experiment_ids": ["1"], "filter": "x" * 2001}
            )
        )
        expect_error(
            lambda: mlflow_proxy.runs_search(
                {"experiment_ids": ["1"], "page_token": "x" * 4001}
            )
        )
        expect_error(
            lambda: mlflow_proxy.runs_search({"experiment_ids": ["1"], "project": ""})
        )
        assert sent == [], "検査に通らない値は MLflow を 1 度も呼ばない"
        # 正しいものは通る
        mlflow_proxy.runs_search({"experiment_ids": ["1"]})
        assert len(sent) == 1, sent
    finally:
        mlflow_proxy._call = original  # type: ignore


# --- AC3: 偽の MLflow ------------------------------------------------

RUNS_GET_JSON = {"run": {"info": {"run_id": RUN_ID, "status": "FINISHED"}}}


def make_fake_mlflow() -> tuple[ThreadingHTTPServer, dict]:
    """偽の MLflow（`http.server`）を立てる。記録した要求は `seen` に入る。"""
    seen: dict = {"paths": [], "bodies": []}

    class Handler(BaseHTTPRequestHandler):
        def log_message(self, *args) -> None:  # 出力しない
            pass

        def _send(self, code: int, body: bytes, content_type: str) -> None:
            self.send_response(code)
            self.send_header("Content-Type", content_type)
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)

        def _json(self, code: int, value: dict) -> None:
            self._send(code, json.dumps(value).encode("utf-8"), "application/json")

        def do_GET(self) -> None:  # noqa: N802
            parsed = urlparse(self.path)
            seen["paths"].append(self.path)
            query = dict(p.split("=", 1) for p in parsed.query.split("&") if "=" in p)
            if parsed.path == "/api/2.0/mlflow/runs/get":
                self._json(200, RUNS_GET_JSON)
            elif parsed.path == "/api/2.0/mlflow/experiments/get":
                self._json(200, {"experiment": {"experiment_id": query.get("experiment_id")}})
            elif parsed.path == "/api/2.0/mlflow/metrics/get-history":
                self._json(200, {"metrics": [{"metric_key": query.get("metric_key")}]})
            elif parsed.path == "/api/2.0/mlflow/metrics/get-history-bulk-interval":
                self._json(200, {"metrics": []})
            elif parsed.path == "/api/2.0/mlflow/artifacts/list":
                self._json(
                    200,
                    {"root_uri": "file:///tmp/mlruns", "files": [{"path": "a.txt", "is_dir": False, "file_size": 3}]},
                )
            elif parsed.path == "/get-artifact":
                if query.get("path") == "big.bin":
                    self._send(200, b"x" * BIG_SIZE, "application/octet-stream")
                elif query.get("path") == "broken.txt":
                    self._json(400, {"error_code": "INVALID_PARAMETER_VALUE", "message": " boom"})
                else:
                    self._send(200, b"hello", "text/plain")
            else:
                self._json(404, {"error_code": "ENDPOINT_NOT_FOUND", "message": parsed.path})

        def do_POST(self) -> None:  # noqa: N802
            parsed = urlparse(self.path)
            seen["paths"].append(self.path)
            length = int(self.headers.get("Content-Length") or 0)
            raw = self.rfile.read(length) if length else b"{}"
            try:
                body = json.loads(raw or b"{}")
            except json.JSONDecodeError:
                body = {}
            seen["bodies"].append(body)
            if parsed.path == "/api/2.0/mlflow/experiments/search":
                self._json(200, {"experiments": [{"experiment_id": "1", "name": "demo"}]})
            elif parsed.path == "/api/2.0/mlflow/runs/search":
                # 壊れた式を入れたときは MLflow が 400 を返す
                if "boom" in (body.get("filter") or ""):
                    self._json(
                        400,
                        {
                            "error_code": "INVALID_PARAMETER_VALUE",
                            "message": "Invalid clause(s) in filter string: boom",
                        },
                    )
                else:
                    self._json(200, {"runs": [{"info": {"run_id": RUN_ID}}], "next_page_token": None})
            else:
                self._json(404, {"error_code": "ENDPOINT_NOT_FOUND", "message": parsed.path})

    server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    seen["server"] = server
    return server, seen


def with_mlflow_url(url: str):
    """`mlflow_proxy` だけ MLflow の先を差し替える（他は触らない）。"""
    return replace(settings, mlflow_url=url)


def test_fake_mlflow() -> None:
    """偽の MLflow への返り値はそのまま（AC3）。"""
    server, seen = make_fake_mlflow()
    original = mlflow_proxy.settings
    mlflow_proxy.settings = with_mlflow_url(f"http://127.0.0.1:{server.server_port}")
    try:
        # 実験の一覧
        got = mlflow_proxy.experiments()
        assert got["experiments"][0]["name"] == "demo", got
        # 実行 1 件（その JSON がそのまま返る）
        got = mlflow_proxy.run(RUN_ID)
        assert got == RUNS_GET_JSON, got
        # run_id はそのまま送られる
        assert seen["paths"][-1] == f"/api/2.0/mlflow/runs/get?run_id={RUN_ID}", seen["paths"][-1]
        # 指標
        got = mlflow_proxy.metric_history(RUN_ID, "loss")
        assert got["metrics"][0]["metric_key"] == "loss", got
        got = mlflow_proxy.metric_bulk([RUN_ID], "loss", 100)
        assert got == {"metrics": []}, got
        # 成果物の一覧
        got = mlflow_proxy.artifacts(RUN_ID)
        assert got["files"][0]["path"] == "a.txt", got
        # 検索
        got = mlflow_proxy.runs_search({"experiment_ids": ["1"]})
        assert got["runs"][0]["info"]["run_id"] == RUN_ID, got
        # 検索は POST で filter を本文に載せる
        assert seen["paths"][-1] == "/api/2.0/mlflow/runs/search", seen["paths"][-1]
        assert seen["bodies"][-1]["filter"] == "", seen["bodies"][-1]
    finally:
        mlflow_proxy.settings = original
        server.shutdown()
        server.server_close()


def test_fake_mlflow_400() -> None:
    """MLflow の 400 は状態コードも本文もそのまま（AC3）。"""
    server, _seen = make_fake_mlflow()
    original = mlflow_proxy.settings
    mlflow_proxy.settings = with_mlflow_url(f"http://127.0.0.1:{server.server_port}")
    try:
        try:
            mlflow_proxy.runs_search({"experiment_ids": ["1"], "filter": 'tags."x" = "boom"'})
        except mlflow_proxy.MlflowError as e:
            assert e.status_code == 400, e.status_code
            assert b"Invalid clause(s) in filter string" in e.body, e.body
        else:
            raise AssertionError("400 になるはず")
        # main.py の例外ハンドラが本文と状態コードをそのまま返す
        from sai_phone import main as phone_main

        import asyncio

        response = asyncio.run(
            phone_main._mlflow_error(
                None, mlflow_proxy.MlflowError(400, b'{"message": "boom"}')
            )
        )
        assert response.status_code == 400, response.status_code
        assert response.body == b'{"message": "boom"}', response.body
    finally:
        mlflow_proxy.settings = original
        server.shutdown()
        server.server_close()


def test_stopped_mlflow_503() -> None:
    """止まった MLflow には 503（AC3）。"""
    original = mlflow_proxy.settings
    # 誰も聞いていないポート（荷が listen していないのでつながらない）
    mlflow_proxy.settings = with_mlflow_url("http://127.0.0.1:9")
    try:
        expect_error(lambda: mlflow_proxy.experiments(), status=503)
        expect_error(lambda: mlflow_proxy.run(RUN_ID), status=503)
    finally:
        mlflow_proxy.settings = original


def test_artifact_size_and_type() -> None:
    """成果物は 5MB を超えると 413・小さいのは本文と Content-Type が返る（AC3）。"""
    server, _seen = make_fake_mlflow()
    original = mlflow_proxy.settings
    mlflow_proxy.settings = with_mlflow_url(f"http://127.0.0.1:{server.server_port}")
    try:
        # 小さい本文（.txt は text/plain）
        response = mlflow_proxy.artifact(RUN_ID, "a.txt")
        assert response.body == b"hello", response.body
        assert response.media_type == "text/plain", response.media_type
        # 6MB は 413
        expect_error(lambda: mlflow_proxy.artifact(RUN_ID, "big.bin"), status=413)
        # 形の違う run_id と場所は 400（偽のサーバには届かない）
        expect_error(lambda: mlflow_proxy.artifact("nope", "a.txt"))
        expect_error(lambda: mlflow_proxy.artifact(RUN_ID, "/etc/passwd"))
        expect_error(lambda: mlflow_proxy.artifact(RUN_ID, ""))
    finally:
        mlflow_proxy.settings = original
        server.shutdown()
        server.server_close()


# --- MLflow の中継の住所 -----------------------------------------------


def test_mlflow_routes_registered() -> None:
    """MLflow の中継の住所が登録されている。"""
    from sai_phone import main as phone_main

    routes = {r.path for r in phone_main.app.routes if hasattr(r, "path")}
    for path in (
        "/api/mlflow/experiments",
        "/api/mlflow/experiments/{experiment_id}",
        "/api/mlflow/runs/search",
        "/api/mlflow/runs/{run_id}",
        "/api/mlflow/metrics/history",
        "/api/mlflow/metrics/bulk",
        "/api/mlflow/artifacts",
        "/api/mlflow/artifact",
    ):
        assert path in routes, f"住所が無い: {path}"


class _FakeProc:
    def __init__(self, returncode: int, stdout: str, stderr: str) -> None:
        self.returncode = returncode
        self.stdout = stdout
        self.stderr = stderr


def main() -> None:
    test_accepts()
    test_experiment_id_400()
    test_run_id_400()
    test_metric_key_400()
    test_artifact_path_400()
    test_max_results_400()
    test_project_filter()
    test_project_unknown_404()
    test_experiments_project()
    test_runs_search_400()
    test_fake_mlflow()
    test_fake_mlflow_400()
    test_stopped_mlflow_503()
    test_artifact_size_and_type()
    test_mlflow_routes_registered()
    print("test_mlflow_api: すべて通りました")


main()
