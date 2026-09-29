"""MLflow の読み取り中継（実験タブ・U03）。

iPhone から実験の記録を見られるようにするため、Mac の MLflow（既定
`http://127.0.0.1:5001`）を **読み取りだけ** で中継する。sai-phone は他の
DB を開かない（ARCHITECTURE.md §2）ので、MLflow とは HTTP でしか話さない。

- 書き込み用の住所は作らない（要件の範囲外。`/api/mlflow/{path:path}` で
  全部を通す案も採らない）。
- 通信手段は標準ライブラリの `urllib.request` だけ（依存を増やさない大原則）。
- 外から来る値は **MLflow を呼ぶ前に** ここで検査する（通らない値は 400）。
- プロジェクトの絞り込み（`tags."sai.project"`）はサーバが組み立てる。値は
  登録簿の root のフォルダ名で、画面は root を知らない（U03 §決定事項）。
"""

from __future__ import annotations

import base64
import json
import mimetypes
import re
import urllib.error
import urllib.parse
import urllib.request
from pathlib import Path

from fastapi import HTTPException
from fastapi.responses import Response

from sai_phone.config import settings

# 実験 ID（MLflow は数字だけ・U03 §変更内容 2）
_EXPERIMENT_ID_RE = re.compile(r"^\d{1,19}$")
# run ID（MLflow は 16 進の 32 桁・U03 §変更内容 2）
_RUN_ID_RE = re.compile(r"^[0-9a-f]{32}$")

# 長さの上限（外から来る値なので、無限に受け取らない）
METRIC_KEY_MAX = 250
ARTIFACT_PATH_MAX = 500
FILTER_MAX = 2000
ORDER_BY_MAX = 200
PAGE_TOKEN_MAX = 4000
# まとめて受け取る要素の個数の上限
EXPERIMENT_IDS_MAX = 50
ORDER_BY_COUNT_MAX = 3
RUN_IDS_MAX = 100
# 成果物の大きさの上限（iPhone の回線とメモリ・U03 §決定事項）
ARTIFACT_MAX = 5 * 1024 * 1024

# 時間切れ（秒）。MLflow が止まっているときのために上限を置いておく
DEFAULT_TIMEOUT = 30.0


class MlflowError(Exception):
    """MLflow がエラーを返したとき（本文と状態コードをそのまま持ち回る）。

    返り値を JSON の dict にすると本文の形が変わってしまうため、MLflow の
    本文と状態コードをそのまま iPhone へ返すために使う。`main.py` が例外
    ハンドラで `Response` に変えて返す。
    """

    def __init__(self, status_code: int, body: bytes) -> None:
        super().__init__(f"MLflow が {status_code} を返しました")
        self.status_code = status_code
        self.body = body


def _bad(detail: str) -> HTTPException:
    """400 を返す。理由は日本語で書く（利用者が読む文言）。"""
    return HTTPException(status_code=400, detail=detail)


def _has_control_char(text: str) -> bool:
    """制御文字（改行やタブも含む）が入ってはいけない。"""
    return any(ord(c) < 0x20 or ord(c) == 0x7F for c in text)


# --- 外から来る値の検査 ---------------------------------------------


def check_experiment_id(value: str) -> str:
    """実験 ID は数字 1〜19 桁（違えば 400）。"""
    if not isinstance(value, str) or not _EXPERIMENT_ID_RE.match(value):
        raise _bad("実験の番号は数字で送ってください")
    return value


def check_run_id(value: str) -> str:
    """run ID は 16 進の 32 桁（違えば 400）。"""
    if not isinstance(value, str) or not _RUN_ID_RE.match(value):
        raise _bad("実行の番号は 16 進の 32 桁にしてください")
    return value


def check_metric_key(value: str) -> str:
    """指標の名前は 1〜250 字（制御文字なし）。"""
    if (
        not isinstance(value, str)
        or not 1 <= len(value) <= METRIC_KEY_MAX
        or _has_control_char(value)
    ):
        raise _bad("指標の名前は 250 文字までの 1 行にしてください")
    return value


def check_artifact_path(value: str) -> str:
    """成果物の場所は 0〜500 字（先頭の `/` なし・`..` の区切りなし）。"""
    if not isinstance(value, str) or len(value) > ARTIFACT_PATH_MAX:
        raise _bad("成果物の場所が長すぎます（500 文字まで）")
    if _has_control_char(value):
        raise _bad("成果物の場所に制御文字は使えません")
    if value.startswith("/"):
        raise _bad("成果物の場所は / で始まらないでください")
    # 区切りが `..` のものは外へ出る可能性があるので止める
    parts = [p for p in value.split("/") if p != ""]
    if any(p == ".." for p in parts):
        raise _bad("成果物の場所に .. は使えません")
    return value


def check_max_results(value, top: int) -> int:
    """まとめて取る個数は 1〜top の整数（既定は top）。"""
    if value is None:
        return top
    if isinstance(value, bool) or not isinstance(value, int) or not 1 <= value <= top:
        raise _bad(f"取る個数は 1 から {top} までの整数にしてください")
    return value


def check_text(value, limit: int, label: str) -> str:
    """長さの範囲だけ確かめる補助（無ければ空文字列）。"""
    if value is None:
        return ""
    if not isinstance(value, str) or len(value) > limit or _has_control_char(value):
        raise _bad(f"{label}は {limit} 文字までの 1 行にしてください")
    return value


# --- MLflow へ送る ----------------------------------------------------


def _call(
    method: str,
    api_path: str,
    query: list[tuple[str, str]] | None = None,
    body: dict | None = None,
    timeout: float = DEFAULT_TIMEOUT,
    max_bytes: int | None = None,
) -> bytes:
    """MLflow の REST API を 1 回呼び、本文のバイト列を返す（U03 §変更内容 2）。

    - 送り先は `settings.mlflow_url` + `api_path`。
    - `settings.mlflow_user` が空でなければ Basic 認証を付ける。
    - MLflow のエラーは本文と状態コードをそのまま `MlflowError` で持ち回る。
    - つながらない・応答しないときは 503。
    """
    url = f"{settings.mlflow_url}{api_path}"
    qs = urllib.parse.urlencode(query or [])
    if qs:
        url = f"{url}?{qs}"
    headers: dict[str, str] = {"Accept": "application/json"}
    if settings.mlflow_user:
        raw = f"{settings.mlflow_user}:{settings.mlflow_password}".encode("utf-8")
        headers["Authorization"] = "Basic " + base64.b64encode(raw).decode("ascii")
    data = None
    if body is not None:
        data = json.dumps(body).encode("utf-8")
        headers["Content-Type"] = "application/json"
    req = urllib.request.Request(url, data=data, headers=headers, method=method)
    try:
        with urllib.request.urlopen(req, timeout=timeout) as resp:
            # max_bytes があれば、その 1 バイト先までしか読まない（大きすぎる本文でメモリを食わない）
            return resp.read() if max_bytes is None else resp.read(max_bytes + 1)
    except urllib.error.HTTPError as e:
        # MLflow の {"error_code": …, "message": …} をそのまま返す（400 の理由もそのまま）
        raise MlflowError(e.code, e.read()) from e
    except (urllib.error.URLError, OSError) as e:
        raise HTTPException(
            status_code=503, detail="Mac の MLflow が応答しません"
        ) from e


def _call_json(
    method: str,
    api_path: str,
    query: list[tuple[str, str]] | None = None,
    body: dict | None = None,
) -> dict:
    """`_call` の返り値を JSON として読む（読めなければ 502）。"""
    raw = _call(method, api_path, query=query, body=body)
    try:
        parsed = json.loads(raw)
    except (json.JSONDecodeError, TypeError, ValueError) as e:
        raise HTTPException(status_code=502, detail="MLflow の応答を読めませんでした") from e
    return parsed if isinstance(parsed, dict) else {"items": parsed}


def _escape_filter_value(value: str) -> str:
    """引用符で囲んだ filter の値として使う（中身の `"` と `\\` を逃がす）。"""
    return value.replace("\\", "\\\\").replace('"', '\\"')


def _project_filter(name: str) -> str:
    """`tags."sai.project" = "<root のフォルダ名>"` の 1 節を作る（U03 §決定事項）。

    登録簿に無いプロジェクトは 404（`control.find_project`）。
    """
    from sai_phone import control  # 循環を避けるため関数の中で読み込む

    project = control.find_project(name)
    folder = Path(project.get("root") or "").name
    return f'tags."sai.project" = "{_escape_filter_value(folder)}"'


# --- 公開する関数 ----------------------------------------------------


def experiments() -> dict:
    """実験の一覧（`POST experiments/search`・最大 100 個・名前順）。"""
    return _call_json(
        "POST",
        "/api/2.0/mlflow/experiments/search",
        body={"max_results": 100, "order_by": ["name ASC"]},
    )


def experiment(experiment_id: str) -> dict:
    """実験 1 件（`GET experiments/get`）。"""
    return _call_json(
        "GET",
        "/api/2.0/mlflow/experiments/get",
        query=[("experiment_id", check_experiment_id(experiment_id))],
    )


def runs_search(payload: dict) -> dict:
    """実行の検索（`POST runs/search`）。

    - 外から来る値を全部ここで検査してから MLflow を呼ぶ。
    - `project` があれば `tags."sai.project"` の条件を先に足す。
    - `run_view_type` は `ACTIVE_ONLY` 固定（iPhone には削除した実行を出さない）。
    """
    if not isinstance(payload, dict):
        raise _bad("本文は JSON の物（{…}）で送ってください")
    ids = payload.get("experiment_ids")
    if not isinstance(ids, list) or not 1 <= len(ids) <= EXPERIMENT_IDS_MAX:
        raise _bad("実験の番号は 1 から 50 個までの配列で送ってください")
    checked_ids = [check_experiment_id(str(i)) for i in ids]
    user_filter = check_text(payload.get("filter"), FILTER_MAX, "絞り込みの式")
    order_by = payload.get("order_by")
    if order_by is None:
        order_by = []
    if not isinstance(order_by, list) or len(order_by) > ORDER_BY_COUNT_MAX:
        raise _bad("並びの指定は 3 個までの配列にしてください")
    checked_order = [
        check_text(item, ORDER_BY_MAX, "並びの指定") for item in order_by
    ]
    checked_order = [item for item in checked_order if item]
    max_results = check_max_results(payload.get("max_results"), 100)
    page_token = check_text(payload.get("page_token"), PAGE_TOKEN_MAX, "続きの印")

    filter_expr = user_filter
    project = payload.get("project")
    if project is not None:
        if not isinstance(project, str) or not project.strip():
            raise _bad("プロジェクト名を空では送れません")
        clause = _project_filter(project)
        filter_expr = clause if not user_filter else f"{clause} AND ({user_filter})"

    body = {
        "experiment_ids": checked_ids,
        "filter": filter_expr,
        "run_view_type": "ACTIVE_ONLY",
        "order_by": checked_order,
        "max_results": max_results,
        "page_token": page_token,
    }
    return _call_json("POST", "/api/2.0/mlflow/runs/search", body=body)


def run(run_id: str) -> dict:
    """実行 1 件（`GET runs/get`）。"""
    return _call_json(
        "GET", "/api/2.0/mlflow/runs/get", query=[("run_id", check_run_id(run_id))]
    )


def metric_history(run_id: str, metric_key: str, max_results=None) -> dict:
    """指標 1 本の履歴（`GET metrics/get-history`）。"""
    return _call_json(
        "GET",
        "/api/2.0/mlflow/metrics/get-history",
        query=[
            ("run_id", check_run_id(run_id)),
            ("metric_key", check_metric_key(metric_key)),
            ("max_results", str(check_max_results(max_results, 25000))),
        ],
    )


def metric_bulk(run_ids: list[str], metric_key: str, max_results=None) -> dict:
    """指標 1 本の履歴をまとめて取る（`GET metrics/get-history-bulk-interval`）。

    MLflow の run 数は 100 件まで、1 run の点は 2500 点まで。
    実行の番号は `run_ids` という名前で繰り返し送る（MLflow の決まり）。
    """
    if not isinstance(run_ids, list) or not 1 <= len(run_ids) <= RUN_IDS_MAX:
        raise _bad("実行の番号は 1 から 100 個までで送ってください")
    query = [("run_ids", check_run_id(r)) for r in run_ids]
    query.append(("metric_key", check_metric_key(metric_key)))
    query.append(("max_results", str(check_max_results(max_results, 2500))))
    return _call_json("GET", "/api/2.0/mlflow/metrics/get-history-bulk-interval", query=query)


def artifacts(run_id: str, path: str | None = None) -> dict:
    """成果物の一覧（`GET artifacts/list`）。"""
    query = [("run_id", check_run_id(run_id))]
    target = "" if path is None else check_artifact_path(path)
    if target:
        query.append(("path", target))
    return _call_json("GET", "/api/2.0/mlflow/artifacts/list", query=query)


def artifact(run_id: str, path: str) -> Response:
    """成果物 1 件の本文（`GET /get-artifact`）。

    - 5MB を超えたら 413（iPhone の回線とメモリのため・U03 §決定事項）。
    - `Content-Type` は拡張子から決める（分からなければ `application/octet-stream`）。
    - 5MB + 1 バイトまでしか読まないので、巨大な本文でメモリを食わない。
    """
    checked_run = check_run_id(run_id)
    checked_path = check_artifact_path(path)
    if not checked_path:
        raise _bad("成果物の場所を送ってください")
    raw = _call(
        "GET",
        "/get-artifact",
        query=[("run_uuid", checked_run), ("path", checked_path)],
        timeout=60.0,
        max_bytes=ARTIFACT_MAX,
    )
    if len(raw) > ARTIFACT_MAX:
        raise HTTPException(
            status_code=413, detail="大きすぎるので表示しません（5MB まで）"
        )
    guessed, _encoding = mimetypes.guess_type(checked_path)
    return Response(
        content=raw,
        media_type=guessed or "application/octet-stream",
    )
