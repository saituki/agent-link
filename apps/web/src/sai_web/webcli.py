"""sai-web: Web の窓口（`apps/web`）の CLI。

Web にしか無い機能（質問・承認・送信・使用量・MLflow）と、Web のすべての
HTTP API を、コマンド 1 本から使えるようにする。中身の処理は `apps/web` と
Rust のコアにあり、このファイルは HTTP を呼ぶだけ（`sai-sci/tools/phone_api.py`
と同じ動き。ただし失敗の扱いは「sai-web の約束」に合わせる）。

呼び方:
    apps/web/bin/sai-web projects
    apps/web/bin/sai-web home
    apps/web/bin/sai-web questions [--run <run_id>] [--project <名前>]
    apps/web/bin/sai-web question <qid>
    apps/web/bin/sai-web answer <qid> (--choice <番号> | --choices <番号,番号> | --text <文>) …
    apps/web/bin/sai-web approvals [--run <run_id>]
    apps/web/bin/sai-web approve <aid> <allow|always|deny> [--reason <文>]
    apps/web/bin/sai-web send <project> <run_id> <文>
    apps/web/bin/sai-web usage
    apps/web/bin/sai-web mlflow experiments
    apps/web/bin/sai-web mlflow runs <experiment_id> [--filter <式>] [--max <n>]
    apps/web/bin/sai-web mlflow run <mlflow_run_id>
    apps/web/bin/sai-web mlflow metric <mlflow_run_id> <key> [--max <n>]
    apps/web/bin/sai-web api <METHOD> <path> [--data <JSON> | --data -]
    apps/web/bin/sai-web serve [--host <H>] [--port <P>]

接続先と合言葉（この作業の間は今の名前を使う。最後のガイドで `SAI_WEB_*` に変える）:
    - 接続先: `SAI_WEB_URL`（既定 `http://127.0.0.1:8200`）
    - 合言葉: `SAI_WEB_TOKEN`、無ければ `<sai_home>/web/token`
    - 待ち受けの番号: `SAI_WEB_PORT`（既定 8200）
    - 合言葉は `Authorization: Bearer` のヘッダで送る。URL には付けない。
      **合言葉の値は出力・エラー・ログのどこにも出さない。**

終了コード:
    0 = 成功、2 = 入力の誤り、3 = 見つからない、4 = 状態の衝突、
    1 = その他の失敗（つながらない・扱えない応答など）。
"""

from __future__ import annotations

import argparse
import json
import os
import sys
import urllib.error
import urllib.parse
import urllib.request

from sai_web import registry

# 1 回の呼び出しで待つ時間の上限（秒）
TIMEOUT_SEC = 30
# 承認の答え（サーバが受け取る値）
DECISIONS = ("allow", "always", "deny")
# `api` で受ける HTTP の方法（大文字小文字は問わない）
METHODS = ("GET", "POST", "PUT", "DELETE", "PATCH")


# ---------------------------------------------------------------------------
# 接続先と合言葉
# ---------------------------------------------------------------------------


def base_url() -> str:
    """Web の窓口の置き場（環境変数で差し替えられる）。"""
    return (os.getenv("SAI_WEB_URL") or "http://127.0.0.1:8200").rstrip("/")


def api_token() -> str | None:
    """合言葉（無ければ None）。

    環境変数を先に、次に `apps/web` が起動時に書いたファイルを見る。
    **返り値をそのまま画面やログに出さないこと。**
    """
    token = os.getenv("SAI_WEB_TOKEN")
    if token:
        return token
    path = registry.sai_home() / "web" / "token"
    try:
        return path.read_text(encoding="utf-8").strip() or None
    except OSError:
        # ファイルが無いのは開発用のサーバ（認証なし）
        return None


# ---------------------------------------------------------------------------
# 失敗（終了コードを運ぶ）
# ---------------------------------------------------------------------------


class CliError(Exception):
    """失敗の親。`code` が終了コードになる。"""

    code = 1


class UsageError(CliError):
    """引数の誤り（終了コード 2）。"""

    code = 2


class RawText:
    """`api` で `text/*` の応答をそのまま出すための入れ物。"""

    def __init__(self, text: str) -> None:
        self.text = text


class _NoOutput:
    """出力しない（`serve` が uvicorn に替わったとき）。"""


NO_OUTPUT = _NoOutput()


def _exit_code_for_status(status: int) -> int:
    """HTTP の状態コードを、約束の終了コードに読み替える。"""
    if status == 404:
        return 3  # 見つからない
    if status == 409:
        return 4  # 状態の衝突
    if status in (400, 422):
        return 2  # 入力の誤り
    return 1  # 401・403・5xx など


class HttpFailure(CliError):
    """サーバがエラーを返した（状態コードに応じた終了コード）。"""

    def __init__(self, status: int, detail: str) -> None:
        super().__init__(detail)
        self.status = status
        self.code = _exit_code_for_status(status)


class UnreachableError(CliError):
    """Web の窓口につながらなかった（終了コード 1）。"""

    code = 1


def detail_of_body(body: str, status: int) -> str:
    """サーバが返した本文から理由を取り出す。

    本物の Web は FastAPI の形 `{"detail": …}` を返す。`detail` が一覧
    （入力の検査の誤り・422）なら、各項目の `msg` を「・」でつなぐ。
    """
    fallback = body.strip() or f"HTTP {status}"
    try:
        data = json.loads(body)
    except json.JSONDecodeError:
        return fallback
    if isinstance(data, dict):
        detail = data.get("detail")
        if isinstance(detail, str) and detail:
            return detail
        if isinstance(detail, list):
            msgs = [
                str(item.get("msg"))
                for item in detail
                if isinstance(item, dict) and item.get("msg")
            ]
            if msgs:
                return "・".join(msgs)
    return fallback


# ---------------------------------------------------------------------------
# HTTP
# ---------------------------------------------------------------------------


def request_raw(method: str, path: str, payload: object | None = None):
    """1 つの API を呼び、`(状態コード, 本文, Content-Type)` を返す。

    合言葉はヘッダで渡す（URL に入れない）。失敗は `HttpFailure` か
    `UnreachableError` で伝える。
    """
    url = base_url() + path
    data = None
    headers = {"Accept": "application/json"}
    if payload is not None:
        data = json.dumps(payload, ensure_ascii=False).encode("utf-8")
        headers["Content-Type"] = "application/json"
    token = api_token()
    if token:
        # HTTP のヘッダの値は latin-1 でしか送れない。送れない文字があれば、
        # 記録を残さず、理由だけ伝える（合言葉の値そのものは出さない）
        try:
            token.encode("latin-1")
        except UnicodeEncodeError:
            raise CliError(
                "合言葉にヘッダで送れない文字があります（英数字だけを使ってください）"
            ) from None
        headers["Authorization"] = f"Bearer {token}"
    req = urllib.request.Request(url, data=data, headers=headers, method=method)
    try:
        with urllib.request.urlopen(req, timeout=TIMEOUT_SEC) as res:  # noqa: S310（宛先は設定値）
            return (
                res.status,
                res.read(),
                res.headers.get("Content-Type") or "",
            )
    except urllib.error.HTTPError as exc:
        body = exc.read().decode("utf-8", errors="replace")
        raise HttpFailure(exc.code, detail_of_body(body, exc.code)) from None
    except (urllib.error.URLError, TimeoutError, OSError):
        raise UnreachableError(
            f"Web の窓口（{base_url()}）につながりません。動いているか確かめてください"
        ) from None


def request_json(method: str, path: str, payload: object | None = None) -> object:
    """1 つの API を呼び、JSON を返す（本文が空なら `{}`）。"""
    status, body, _ctype = request_raw(method, path, payload)
    text = body.decode("utf-8", errors="replace")
    if not text.strip():
        return {}
    try:
        return json.loads(text)
    except json.JSONDecodeError:
        raise CliError(
            f"HTTP {status} 応答が JSON ではありません: {text.strip()[:200]}"
        ) from None


# ---------------------------------------------------------------------------
# 質問・承認
# ---------------------------------------------------------------------------


def home() -> dict:
    """ホームの集計（答えを待っている質問と、承認待ちの一覧）。"""
    data = request_json("GET", "/api/home")
    return data if isinstance(data, dict) else {}


def pick_run(items: object, run_id: str | None, project: str | None) -> list:
    """一覧から `--run` と `--project` で狭める（無ければ全部）。"""
    rows = items if isinstance(items, list) else []
    out = []
    for row in rows:
        if not isinstance(row, dict):
            continue
        if run_id is not None and str(row.get("run_id") or "") != str(run_id):
            continue
        if project and (row.get("project") or "") != project:
            continue
        out.append(row)
    return out


def cmd_projects(_args: argparse.Namespace) -> object:
    """登録簿のプロジェクト一覧。"""
    return request_json("GET", "/api/projects")


def cmd_home(_args: argparse.Namespace) -> object:
    """ホームの集計。"""
    return request_json("GET", "/api/home")


def cmd_usage(_args: argparse.Namespace) -> object:
    """使用量（cc のコスト・上限）。"""
    return request_json("GET", "/api/usage")


def cmd_questions(args: argparse.Namespace) -> object:
    """答えを待っている質問の一覧。"""
    data = home()
    return pick_run(data.get("questions"), args.run, args.project)


def cmd_question(args: argparse.Namespace) -> object:
    """質問 1 件の全文（問いと選択肢を番号付きで返す）。"""
    data = request_json("GET", f"/api/questions/{urllib.parse.quote(args.qid)}")
    if not isinstance(data, dict):
        raise CliError("質問の返り方が違います")
    raw = data.get("questions")
    questions = []
    for i, item in enumerate(raw if isinstance(raw, list) else []):
        if not isinstance(item, dict):
            continue
        options = []
        for j, opt in enumerate(
            item.get("options") if isinstance(item.get("options"), list) else []
        ):
            label = opt.get("label") if isinstance(opt, dict) else opt
            options.append(
                {
                    "index": j,
                    "label": str(label or ""),
                    "description": str(
                        (opt.get("description") or "") if isinstance(opt, dict) else ""
                    ),
                }
            )
        questions.append(
            {
                "index": i,
                "question": str(item.get("question") or ""),
                "header": str(item.get("header") or ""),
                "multiSelect": bool(item.get("multiSelect")),
                # `--choice` / `--choices` に使う番号は、この options の index
                "options": options,
            }
        )
    return {"id": data.get("id", args.qid), "state": data.get("state"), "questions": questions}


def parse_answer_items(rest: list[str]) -> list[dict]:
    """`--choice` / `--choices` / `--text` を、書いた順に 1 つずつ答えへ変える。

    3 つの指定は 1 組として扱う（混ぜると数が合わなくなるため）。
    `--json` は他の命令と共通の飾りなので、ここでは読み飛ばす。
    引数が違うときは `UsageError`（終了コード 2）で終わる。
    """
    items: list[dict] = []
    i = 0
    while i < len(rest):
        flag = rest[i]
        if flag == "--json":
            i += 1
            continue
        if flag not in ("--choice", "--choices", "--text"):
            raise UsageError(
                f"答えの指定が違います: {flag}（--choice / --choices / --text を使う）"
            )
        if i + 1 >= len(rest):
            raise UsageError(f"{flag} には値が必要です")
        value = rest[i + 1]
        if flag == "--text":
            items.append({"text": value})
        else:
            try:
                numbers = [int(x) for x in value.split(",") if x.strip() != ""]
            except ValueError:
                raise UsageError(f"{flag} には番号を書きます: {value}") from None
            if not numbers:
                raise UsageError(f"{flag} には番号を書きます: {value}")
            if flag == "--choice":
                items.append({"choice_index": numbers[0]})
            else:
                items.append({"choice_indices": numbers})
        i += 2
    return items


def cmd_answer(args: argparse.Namespace) -> object:
    """質問に答える（問いの順に 1 つずつ並べる）。"""
    items = parse_answer_items(args.rest)
    if not items:
        raise UsageError("答えがありません（--choice / --choices / --text を 1 つ以上書く）")
    # 問いの数と並べた数が合わなければ送らない（サーバが中途半端に記録するのを防ぐ）
    full = cmd_question(argparse.Namespace(qid=args.qid))
    questions = full["questions"] if isinstance(full, dict) else []
    if len(items) != len(questions):
        raise UsageError(
            f"問いは {len(questions)} 個ですが、答えは {len(items)} 個です。"
            " 問いの順に 1 つずつ並べてください"
        )
    return request_json(
        "POST", f"/api/questions/{urllib.parse.quote(args.qid)}/answer", {"answers": items}
    )


def cmd_approvals(args: argparse.Namespace) -> object:
    """承認待ちの一覧。"""
    data = home()
    return pick_run(data.get("approvals"), args.run, None)


def cmd_approve(args: argparse.Namespace) -> object:
    """承認に答える。"""
    if args.decision not in DECISIONS:
        raise UsageError(f"決定は {' / '.join(DECISIONS)} のどれかです: {args.decision}")
    return request_json(
        "POST",
        f"/api/approvals/{urllib.parse.quote(args.aid)}/answer",
        {"decision": args.decision, "reason": args.reason or ""},
    )


def cmd_send(args: argparse.Namespace) -> object:
    """実行中の root の run に文を送る。"""
    try:
        run_id = int(args.run_id)
    except ValueError:
        raise UsageError(f"run_id は番号で書いてください: {args.run_id}") from None
    path = f"/api/runs/{urllib.parse.quote(args.project)}/{run_id}/send"
    return request_json("POST", path, {"text": args.text})


# ---------------------------------------------------------------------------
# MLflow（読み取りだけ）
# ---------------------------------------------------------------------------


def _int_arg(value: str | None, name: str) -> int | None:
    """`--max` などの番号を整数にする（違えば `UsageError`）。"""
    if value is None:
        return None
    try:
        return int(value)
    except ValueError:
        raise UsageError(f"{name} は番号で書いてください: {value}") from None


def cmd_mlflow(args: argparse.Namespace) -> object:
    """MLflow の読み取り（中継 API 経由）。"""
    if args.what == "experiments":
        return request_json("GET", "/api/mlflow/experiments")
    if args.what == "runs":
        payload: dict = {"experiment_ids": [args.experiment_id]}
        if args.filter:
            payload["filter"] = args.filter
        max_results = _int_arg(args.max, "--max")
        if max_results is not None:
            payload["max_results"] = max_results
        return request_json("POST", "/api/mlflow/runs/search", payload)
    if args.what == "run":
        return request_json("GET", f"/api/mlflow/runs/{urllib.parse.quote(args.run_id)}")
    if args.what == "metric":
        query = urllib.parse.urlencode(
            {
                "run_id": args.run_id,
                "metric_key": args.metric_key,
                **(
                    {"max_results": _int_arg(args.max, "--max")}
                    if args.max is not None
                    else {}
                ),
            }
        )
        return request_json("GET", f"/api/mlflow/metrics/history?{query}")
    raise UsageError(f"mlflow の命令が違います: {args.what}")


# ---------------------------------------------------------------------------
# api（すべての HTTP API をそのまま呼ぶ）
# ---------------------------------------------------------------------------


def read_data(value: str | None) -> object | None:
    """`--data` の JSON を読む（`-` なら標準入力）。読めなければ `UsageError`。"""
    if value is None:
        return None
    raw = sys.stdin.read() if value == "-" else value
    try:
        return json.loads(raw)
    except json.JSONDecodeError:
        raise UsageError("--data は JSON で書いてください") from None


def cmd_api(args: argparse.Namespace) -> object:
    """Web のすべての HTTP API をそのまま呼ぶ。"""
    method = args.method.upper()
    if method not in METHODS:
        raise UsageError(f"METHOD は {' / '.join(METHODS)} のどれかです: {args.method}")
    path = args.path
    # 合言葉を付けて別の宛先へ送らないため、`/` で始まる path だけを受ける
    if not path.startswith("/") or "://" in path:
        raise UsageError(f"path は / で始まる形で書いてください: {path}")
    payload = read_data(args.data)
    status, body, ctype = request_raw(method, path, payload)
    text = body.decode("utf-8", errors="replace")
    if not text.strip():
        return {}
    try:
        return json.loads(text)
    except json.JSONDecodeError:
        pass
    if ctype.split(";")[0].strip().lower().startswith("text/"):
        return RawText(text)
    raise CliError(f"この API の応答（{ctype or '不明'}）は CLI では扱えません")


# ---------------------------------------------------------------------------
# serve（サーバを起動する）
# ---------------------------------------------------------------------------


def cmd_serve(args: argparse.Namespace) -> object:
    """uvicorn に替わって Web の窓口のサーバを起動する。"""
    port = args.port
    if port is None:
        try:
            port = int(os.getenv("SAI_WEB_PORT", "8200"))
        except ValueError:
            raise UsageError(
                f"SAI_WEB_PORT が番号ではありません: {os.getenv('SAI_WEB_PORT')}"
            ) from None
    os.execv(
        sys.executable,
        [
            sys.executable,
            "-m",
            "uvicorn",
            "sai_web.main:app",
            "--host",
            args.host,
            "--port",
            str(port),
        ],
    )
    return NO_OUTPUT


# ---------------------------------------------------------------------------
# 引数と入口
# ---------------------------------------------------------------------------


def _add_json(parser: argparse.ArgumentParser) -> None:
    """どの命令でも `--json` を受ける（出力はいつも JSON。付けても変わらない）。"""
    parser.add_argument(
        "--json",
        action="store_true",
        help="出力はいつも JSON（付けても変わらない）",
    )


def build_parser() -> argparse.ArgumentParser:
    """命令の形。"""
    parser = argparse.ArgumentParser(
        prog="sai-web",
        description="Web の窓口（apps/web）の CLI",
    )
    _add_json(parser)
    sub = parser.add_subparsers(dest="command", required=True)

    p = sub.add_parser("serve", help="Web の窓口のサーバを起動する")
    p.add_argument("--host", default="127.0.0.1", help="待ち受ける住所（既定 127.0.0.1）")
    p.add_argument("--port", type=int, default=None, help="待ち受ける番号（既定 SAI_WEB_PORT）")
    _add_json(p)
    p.set_defaults(func=cmd_serve)

    p = sub.add_parser("projects", help="プロジェクトの一覧")
    _add_json(p)
    p.set_defaults(func=cmd_projects)

    p = sub.add_parser("home", help="ホームの集計")
    _add_json(p)
    p.set_defaults(func=cmd_home)

    p = sub.add_parser("questions", help="答えを待っている質問の一覧")
    p.add_argument("--run", help="この run の質問だけ（run_id）")
    p.add_argument("--project", help="このプロジェクトの質問だけ")
    _add_json(p)
    p.set_defaults(func=cmd_questions)

    p = sub.add_parser("question", help="質問 1 件の全文（選択肢に番号が付く）")
    p.add_argument("qid")
    _add_json(p)
    p.set_defaults(func=cmd_question)

    p = sub.add_parser("answer", help="質問に答える（問いの順に 1 つずつ並べる）")
    p.add_argument("qid")
    p.add_argument(
        "rest",
        nargs=argparse.REMAINDER,
        help="--choice <番号> / --choices <番号,番号> / --text <文> を問いの順に並べる",
    )
    _add_json(p)
    p.set_defaults(func=cmd_answer)

    p = sub.add_parser("approvals", help="承認待ちの一覧")
    p.add_argument("--run", help="この run の承認だけ（run_id）")
    _add_json(p)
    p.set_defaults(func=cmd_approvals)

    p = sub.add_parser("approve", help="承認に答える")
    p.add_argument("aid")
    p.add_argument("decision", help="allow / always / deny")
    p.add_argument("--reason", help="理由（書かなくてもよい）")
    _add_json(p)
    p.set_defaults(func=cmd_approve)

    p = sub.add_parser("send", help="実行中の root の run に文を送る")
    p.add_argument("project")
    p.add_argument("run_id")
    p.add_argument("text")
    _add_json(p)
    p.set_defaults(func=cmd_send)

    p = sub.add_parser("usage", help="使用量（cc のコスト・上限）")
    _add_json(p)
    p.set_defaults(func=cmd_usage)

    p = sub.add_parser("mlflow", help="MLflow の読み取り（中継 API 経由）")
    _add_json(p)
    ms = p.add_subparsers(dest="what", required=True)

    q = ms.add_parser("experiments", help="実験の一覧")
    _add_json(q)
    q.set_defaults(func=cmd_mlflow)

    q = ms.add_parser("runs", help="実験の中の実行を検索")
    q.add_argument("experiment_id")
    q.add_argument("--filter", help="MLflow の検索式")
    q.add_argument("--max", help="取り出す上限")
    _add_json(q)
    q.set_defaults(func=cmd_mlflow)

    q = ms.add_parser("run", help="実行 1 件")
    q.add_argument("run_id")
    _add_json(q)
    q.set_defaults(func=cmd_mlflow)

    q = ms.add_parser("metric", help="指標 1 本の履歴")
    q.add_argument("run_id")
    q.add_argument("metric_key")
    q.add_argument("--max", help="取り出す上限")
    _add_json(q)
    q.set_defaults(func=cmd_mlflow)

    p = sub.add_parser("api", help="Web のすべての HTTP API をそのまま呼ぶ")
    p.add_argument("method", help="GET / POST / PUT / DELETE / PATCH")
    p.add_argument("path", help="/ で始まる住所")
    p.add_argument("--data", help="本文の JSON（`-` なら標準入力）")
    _add_json(p)
    p.set_defaults(func=cmd_api)

    return parser


def main(argv: list[str] | None = None) -> int:
    """入口。終了コードを返す。"""
    args = build_parser().parse_args(argv)
    try:
        data = args.func(args)
    except UsageError as exc:
        print(f"引数が違います: {exc}", file=sys.stderr)
        return 2
    except CliError as exc:
        # 失敗の理由は標準エラーの最後の 1 行に書く（標準出力には何も書かない）
        print(str(exc), file=sys.stderr)
        return exc.code
    if data is NO_OUTPUT:
        return 0
    if isinstance(data, RawText):
        sys.stdout.write(data.text)
        if not data.text.endswith("\n"):
            sys.stdout.write("\n")
        return 0
    print(json.dumps(data, ensure_ascii=False, indent=2))
    return 0


if __name__ == "__main__":
    sys.exit(main())
