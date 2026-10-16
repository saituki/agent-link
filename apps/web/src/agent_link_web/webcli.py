"""agent-link-web: Web の窓口（`apps/web`）の CLI。

Web にしか無い機能（質問・送信・使用量）と、Web のすべての
HTTP API を、コマンド 1 本から使えるようにする。中身の処理は `apps/web` と
Rust のコアにあり、このファイルは HTTP を呼ぶだけ。失敗の扱いは「agent-link-web の約束」に合わせる。

呼び方:
    apps/web/bin/agent-link-web projects
    apps/web/bin/agent-link-web home
    apps/web/bin/agent-link-web questions [--run <run_id>] [--project <名前>]
    apps/web/bin/agent-link-web question <qid>
    apps/web/bin/agent-link-web answer <qid> (--choice <番号> | --choices <番号,番号> | --text <文>) …
    apps/web/bin/agent-link-web send <project> <run_id> <文>
    apps/web/bin/agent-link-web usage
    apps/web/bin/agent-link-web api <METHOD> <path> [--data <JSON> | --data -]
    apps/web/bin/agent-link-web serve [--host <H>] [--port <P>]

接続先:
    - 接続先: `AGENT_LINK_WEB_URL` → `http://127.0.0.1:${AGENT_LINK_WEB_PORT:-8400}`
    - 待ち受けの番号: `AGENT_LINK_WEB_PORT`（既定 8400）

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

# 1 回の呼び出しで待つ時間の上限（秒）
TIMEOUT_SEC = 30
# `api` で受ける HTTP の方法（大文字小文字は問わない）
METHODS = ("GET", "POST", "PUT", "DELETE", "PATCH")


# ---------------------------------------------------------------------------
# 接続先
# ---------------------------------------------------------------------------


def base_url() -> str:
    """Web の窓口の置き場（環境変数で差し替えられる）。"""
    url = os.getenv("AGENT_LINK_WEB_URL")
    if url:
        return url.rstrip("/")
    return f"http://127.0.0.1:{os.getenv('AGENT_LINK_WEB_PORT') or '8400'}"


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
                return "; ".join(msgs)
    return fallback


# ---------------------------------------------------------------------------
# HTTP
# ---------------------------------------------------------------------------


def request_raw(method: str, path: str, payload: object | None = None):
    """1 つの API を呼び、`(状態コード, 本文, Content-Type)` を返す。

    失敗は `HttpFailure` か `UnreachableError` で伝える。
    """
    url = base_url() + path
    data = None
    headers = {"Accept": "application/json"}
    if payload is not None:
        data = json.dumps(payload, ensure_ascii=False).encode("utf-8")
        headers["Content-Type"] = "application/json"
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
            f"Cannot reach the Web server ({base_url()}). Check that it is running."
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
            f"HTTP {status}: the response is not JSON: {text.strip()[:200]}"
        ) from None


# ---------------------------------------------------------------------------
# 質問
# ---------------------------------------------------------------------------


def home() -> dict:
    """ホームの集計（答えを待っている質問と、プロジェクトの一覧）。"""
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
    """使用量（Usage のカード・Claude の枠と費用・OpenRouter・DeepSeek）。"""
    return request_json("GET", "/api/usage")


def cmd_questions(args: argparse.Namespace) -> object:
    """答えを待っている質問の一覧（`home()` の `questions`）。"""
    return pick_run(home().get("questions"), args.run, args.project)


def cmd_question(args: argparse.Namespace) -> object:
    """質問 1 件の全文（問いと選択肢を番号付きで返す）。"""
    data = request_json("GET", f"/api/questions/{urllib.parse.quote(args.qid)}")
    if not isinstance(data, dict):
        raise CliError("Unexpected question response.")
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
    return {
        "id": data.get("id", args.qid),
        "state": data.get("state"),
        "questions": questions,
        "answer": data.get("answer"),
        "answered_by": data.get("answered_by"),
    }


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
                f"Invalid answer option: {flag}. Use --choice, --choices or --text."
            )
        if i + 1 >= len(rest):
            raise UsageError(f"{flag} requires a value.")
        value = rest[i + 1]
        if flag == "--text":
            items.append({"text": value})
        else:
            try:
                numbers = [int(x) for x in value.split(",") if x.strip() != ""]
            except ValueError:
                raise UsageError(f"{flag} requires a number: {value}") from None
            if not numbers:
                raise UsageError(f"{flag} requires a number: {value}")
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
        raise UsageError("No answer given. Pass at least one of --choice, --choices, --text.")
    # 問いの数と並べた数が合わなければ送らない（サーバが中途半端に記録するのを防ぐ）
    full = cmd_question(argparse.Namespace(qid=args.qid))
    questions = full["questions"] if isinstance(full, dict) else []
    if len(items) != len(questions):
        raise UsageError(
            f"The question has {len(questions)} items but {len(items)} answers were given."
            " Give one answer per item, in order."
        )
    return request_json(
        "POST", f"/api/questions/{urllib.parse.quote(args.qid)}/answer", {"answers": items}
    )


def cmd_send(args: argparse.Namespace) -> object:
    """実行中の root の run に文を送る。"""
    try:
        run_id = int(args.run_id)
    except ValueError:
        raise UsageError(f"run_id must be a number: {args.run_id}") from None
    path = f"/api/runs/{urllib.parse.quote(args.project)}/{run_id}/send"
    return request_json("POST", path, {"text": args.text})


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
        raise UsageError("--data must be JSON.") from None


def cmd_api(args: argparse.Namespace) -> object:
    """Web のすべての HTTP API をそのまま呼ぶ。"""
    method = args.method.upper()
    if method not in METHODS:
        raise UsageError(f"METHOD must be one of {', '.join(METHODS)}: {args.method}")
    path = args.path
    # 別の宛先へ送らないため、`/` で始まる path だけを受ける
    if not path.startswith("/") or "://" in path:
        raise UsageError(f"path must start with /: {path}")
    # 空白・制御文字は URL に入れられない（http.client が InvalidURL で落ちる）
    if any(c.isspace() or not c.isprintable() for c in path):
        raise UsageError(f"path must not contain whitespace or control characters: {path!r}")
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
    raise CliError(f"The CLI cannot handle this API response ({ctype or 'unknown'}).")


# ---------------------------------------------------------------------------
# serve（サーバを起動する）
# ---------------------------------------------------------------------------


def cmd_serve(args: argparse.Namespace) -> object:
    """uvicorn に替わって Web の窓口のサーバを起動する。"""
    port = args.port
    if port is None:
        try:
            port = int(os.getenv("AGENT_LINK_WEB_PORT", "8400"))
        except ValueError:
            raise UsageError(
                f"AGENT_LINK_WEB_PORT is not a number: {os.getenv('AGENT_LINK_WEB_PORT')}"
            ) from None
    os.execv(
        sys.executable,
        [
            sys.executable,
            "-m",
            "uvicorn",
            "agent_link_web.main:app",
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
        help="Output is always JSON (this flag changes nothing).",
    )


def build_parser() -> argparse.ArgumentParser:
    """命令の形。"""
    parser = argparse.ArgumentParser(
        prog="agent-link-web",
        description="CLI for the Web app (apps/web)",
    )
    _add_json(parser)
    sub = parser.add_subparsers(dest="command", required=True)

    p = sub.add_parser("serve", help="Start the Web server")
    p.add_argument("--host", default="127.0.0.1", help="Host to listen on (default 127.0.0.1)")
    p.add_argument("--port", type=int, default=None, help="Port to listen on (default AGENT_LINK_WEB_PORT)")
    _add_json(p)
    p.set_defaults(func=cmd_serve)

    p = sub.add_parser("projects", help="List projects")
    _add_json(p)
    p.set_defaults(func=cmd_projects)

    p = sub.add_parser("home", help="Home summary")
    _add_json(p)
    p.set_defaults(func=cmd_home)

    p = sub.add_parser("questions", help="List questions waiting for an answer")
    p.add_argument("--run", help="Only questions of this run (run_id)")
    p.add_argument("--project", help="Only questions of this project")
    _add_json(p)
    p.set_defaults(func=cmd_questions)

    p = sub.add_parser("question", help="Full text of one question (options are numbered)")
    p.add_argument("qid")
    _add_json(p)
    p.set_defaults(func=cmd_question)

    p = sub.add_parser("answer", help="Answer a question (one answer per item, in order)")
    p.add_argument("qid")
    p.add_argument(
        "rest",
        nargs=argparse.REMAINDER,
        help="Give --choice <number> / --choices <number,number> / --text <text> in question order",
    )
    _add_json(p)
    p.set_defaults(func=cmd_answer)

    p = sub.add_parser("send", help="Send a message to a running root run")
    p.add_argument("project")
    p.add_argument("run_id")
    p.add_argument("text")
    _add_json(p)
    p.set_defaults(func=cmd_send)

    p = sub.add_parser("usage", help="Usage (Claude Code limits and cost)")
    _add_json(p)
    p.set_defaults(func=cmd_usage)

    p = sub.add_parser("api", help="Call any Web HTTP API as is")
    p.add_argument("method", help="GET / POST / PUT / DELETE / PATCH")
    p.add_argument("path", help="Path starting with /")
    p.add_argument("--data", help="JSON body (`-` reads standard input)")
    _add_json(p)
    p.set_defaults(func=cmd_api)

    return parser


def main(argv: list[str] | None = None) -> int:
    """入口。終了コードを返す。"""
    args = build_parser().parse_args(argv)
    try:
        data = args.func(args)
    except UsageError as exc:
        print(f"Invalid arguments: {exc}", file=sys.stderr)
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
