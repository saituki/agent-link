#!/usr/bin/env python3
"""sai-phone（Web の窓口）の HTTP API を呼ぶ道具。

professor や operator は、この 1 本を呼ぶだけで Web と同じ操作ができる。
画面（`apps/phone` の JavaScript）と同じ HTTP API を使うので、
どちらで答えても同じ質問・承認として扱われる。
中身の処理は `apps/phone` と Rust のコアにある。このファイルは HTTP を呼ぶだけ。

呼び方（標準ライブラリだけ。Python 3.11 以上）:
    python3 phone_api.py questions [--run <run_id>] [--project <名前>]
    python3 phone_api.py question <qid>
    python3 phone_api.py answer <qid> (--choice <番号> | --choices <番号,番号> | --text <文>) …
    python3 phone_api.py approvals [--run <run_id>]
    python3 phone_api.py approve <aid> <allow|always|deny> [--reason <文>]
    python3 phone_api.py send <project> <run_id> <文>
    python3 phone_api.py mlflow experiments
    python3 phone_api.py mlflow runs <experiment_id> [--filter <式>] [--max <n>]
    python3 phone_api.py mlflow run <mlflow_run_id>
    python3 phone_api.py mlflow metric <mlflow_run_id> <key> [--max <n>]

    例: answer q123 --choice 0 --text "30 epoch で止める"
        （`--choice` `--choices` `--text` は問いの順に 1 つずつ並べる）

置き場:
    研究プロジェクトの `.sai/tools/phone_api.py`（配るのは別の作業）。
    呼ぶときは `python3 "$SAI_PROJECT_ROOT/.sai/tools/phone_api.py" …`。

接続先と合言葉:
    - 接続先は環境変数 `SAI_PHONE_URL`（既定 `http://127.0.0.1:8200`）。
    - 合言葉は環境変数 `SAI_PHONE_TOKEN`、無ければ `<sai_home>/phone/token`
      （`SAI_SAI_HOME` か `~/.sai-moon`。`apps/phone` が起動時に書く）。
      どちらも無ければ合言葉なしで呼ぶ（開発用のサーバ）。
    - 合言葉は URL に付けず、ヘッダ（`Authorization: Bearer`）で渡す。
      **合言葉の値は出力にもエラーにも出さない。**

出力と終了コード:
    - 標準出力に JSON を 1 つ（日本語はそのまま。字下げ 2）。
    - 0 = 呼び出しが終わった、1 = サーバがエラーを返した・引数が違う、
      2 = sai-phone につながなかった。
    - 1 回の呼び出しは 30 秒まで待つ。
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
# 承認の答え（サーバが受け取る値）
DECISIONS = ("allow", "always", "deny")


# ---------------------------------------------------------------------------
# 接続先と合言葉
# ---------------------------------------------------------------------------


def base_url() -> str:
    """sai-phone さんの置き場（環境変数で差し替えられる）。"""
    return (os.getenv("SAI_PHONE_URL") or "http://127.0.0.1:8200").rstrip("/")


def sai_home() -> str:
    """`~/.sai-moon` の置き場（`SAI_SAI_HOME` で上書きできる）。"""
    return os.getenv("SAI_SAI_HOME") or os.path.join(os.path.expanduser("~"), ".sai-moon")


def api_token() -> str | None:
    """合言葉（無ければ None）。

    環境変数を先に、次に `apps/phone` が書いたファイルを見る。
    **返り値をそのまま画面やログに出さないこと。**
    """
    token = os.getenv("SAI_PHONE_TOKEN")
    if token:
        return token
    path = os.path.join(sai_home(), "phone", "token")
    try:
        with open(path, encoding="utf-8") as fh:
            return fh.read().strip() or None
    except OSError:
        # ファイルが無いのは開発用のサーバ（認証なし）
        return None


# ---------------------------------------------------------------------------
# HTTP
# ---------------------------------------------------------------------------


class PhoneError(Exception):
    """サーバがエラーを返した・要求：中身が足りない（終了コード 1）。"""


class PhoneUnreachable(Exception):
    """sai-phone につながなかった（終了コード 2）。"""


class UsageError(Exception):
    """引数が違う（終了コード 2）。"""


def request_json(method: str, path: str, payload: dict | None = None) -> object:
    """1 つの API を呼び、JSON を返す。

    合言葉はヘッダで渡す（URL に入れない）。
    下の 2 つの例外以外は投げない（画面に返る JSON は 1 つだけにするため）。
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
            raise PhoneError(
                "合言葉にヘッダで送れない文字があります（英数字だけを使ってください）"
            ) from None
        headers["Authorization"] = f"Bearer {token}"
    req = urllib.request.Request(url, data=data, headers=headers, method=method)
    try:
        with urllib.request.urlopen(req, timeout=TIMEOUT_SEC) as res:  # noqa: S310（宛先は設定値）
            body = res.read().decode("utf-8")
    except urllib.error.HTTPError as exc:
        raise PhoneError(f"{exc.code} {detail_of(exc)}") from None
    except (urllib.error.URLError, TimeoutError, OSError):
        raise PhoneUnreachable(
            f"sai-phone（{base_url()}）につながりません。Web の窓口が動いているか確かめてください"
        ) from None
    if not body.strip():
        return {}
    try:
        return json.loads(body)
    except json.JSONDecodeError:
        raise PhoneError(f"200 {body.strip()}") from None


def detail_of(exc: urllib.error.HTTPError) -> str:
    """サーバが返した本文から理由を取り出す（`{"detail": "…"}` のはず）。"""
    try:
        body = exc.read().decode("utf-8")
    except (OSError, UnicodeDecodeError):
        return exc.reason if isinstance(exc.reason, str) else "サーバがエラーを返しました"
    try:
        data = json.loads(body)
    except json.JSONDecodeError:
        return body.strip() or "サーバがエラーを返しました"
    if isinstance(data, dict):
        detail = data.get("detail")
        if isinstance(detail, str) and detail:
            return detail
    return body.strip() or "サーバがエラーを返しました"


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


def cmd_questions(args: argparse.Namespace) -> object:
    """答えを待っている質問の一覧。"""
    data = home()
    return pick_run(data.get("questions"), args.run, args.project)


def cmd_question(args: argparse.Namespace) -> object:
    """質問 1 件の全文（問いと選択肢を番号付きで返す）。"""
    data = request_json("GET", f"/api/questions/{urllib.parse.quote(args.qid)}")
    if not isinstance(data, dict):
        raise PhoneError("200 質問の返り方が違います")
    raw = data.get("questions")
    questions = []
    for i, item in enumerate(raw if isinstance(raw, list) else []):
        if not isinstance(item, dict):
            continue
        options = []
        for j, opt in enumerate(item.get("options") if isinstance(item.get("options"), list) else []):
            label = opt.get("label") if isinstance(opt, dict) else opt
            options.append({"index": j, "label": str(label or ""), "description": str(
                (opt.get("description") or "") if isinstance(opt, dict) else ""
            )})
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
    引数が違うときは `UsageError`（終了コード 2）で終わる。
    """
    items: list[dict] = []
    i = 0
    while i < len(rest):
        flag = rest[i]
        if flag not in ("--choice", "--choices", "--text"):
            raise UsageError(f"答えの指定が違います: {flag}（--choice / --choices / --text を使う）")
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
        raise PhoneError(
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
    path = f"/api/runs/{urllib.parse.quote(args.project)}/{int(args.run_id)}/send"
    return request_json("POST", path, {"text": args.text})


# ---------------------------------------------------------------------------
# MLflow（読み取りだけ）
# ---------------------------------------------------------------------------


def cmd_mlflow(args: argparse.Namespace) -> object:
    if args.what == "experiments":
        return request_json("GET", "/api/mlflow/experiments")
    if args.what == "runs":
        payload: dict = {"experiment_id": args.experiment_id}
        if args.filter:
            payload["filter"] = args.filter
        if args.max is not None:
            payload["max_results"] = int(args.max)
        return request_json("POST", "/api/mlflow/runs/search", payload)
    if args.what == "run":
        return request_json("GET", f"/api/mlflow/runs/{urllib.parse.quote(args.run_id)}")
    if args.what == "metric":
        query = urllib.parse.urlencode(
            {
                "run_id": args.run_id,
                "metric_key": args.metric_key,
                **({"max_results": int(args.max)} if args.max is not None else {}),
            }
        )
        return request_json("GET", f"/api/mlflow/metrics/history?{query}")
    raise UsageError(f"mlflow の命令が違います: {args.what}")


# ---------------------------------------------------------------------------
# 引数と入口
# ---------------------------------------------------------------------------


def build_parser() -> argparse.ArgumentParser:
    """命令の形（共有契約 C8）。professor と operator の手順はこの形。"""
    parser = argparse.ArgumentParser(
        prog="phone_api.py",
        description="sai-phone（Web の窓口）の HTTP API を呼ぶ道具",
    )
    sub = parser.add_subparsers(dest="command", required=True)

    p = sub.add_parser("questions", help="答えを待っている質問の一覧")
    p.add_argument("--run", help="この run の質問だけ（run_id）")
    p.add_argument("--project", help="このプロジェクトの質問だけ")
    p.set_defaults(func=cmd_questions)

    p = sub.add_parser("question", help="質問 1 件の全文（選択肢に番号が付く）")
    p.add_argument("qid")
    p.set_defaults(func=cmd_question)

    p = sub.add_parser("answer", help="質問に答える（問いの順に 1 つずつ並べる）")
    p.add_argument("qid")
    p.add_argument(
        "rest",
        nargs=argparse.REMAINDER,
        help="--choice <番号> / --choices <番号,番号> / --text <文> を問いの順に並べる",
    )
    p.set_defaults(func=cmd_answer)

    p = sub.add_parser("approvals", help="承認待ちの一覧")
    p.add_argument("--run", help="この run の承認だけ（run_id）")
    p.set_defaults(func=cmd_approvals)

    p = sub.add_parser("approve", help="承認に答える")
    p.add_argument("aid")
    p.add_argument("decision", help="allow / always / deny")
    p.add_argument("--reason", help="理由（書かなくてもよい）")
    p.set_defaults(func=cmd_approve)

    p = sub.add_parser("send", help="実行中の root の run に文を送る")
    p.add_argument("project")
    p.add_argument("run_id")
    p.add_argument("text")
    p.set_defaults(func=cmd_send)

    p = sub.add_parser("mlflow", help="MLflow の読み取り（中継 API 経由）")
    ms = p.add_subparsers(dest="what", required=True)

    q = ms.add_parser("experiments", help="実験の一覧")
    q.set_defaults(func=cmd_mlflow)

    q = ms.add_parser("runs", help="実験の中の実行を検索")
    q.add_argument("experiment_id")
    q.add_argument("--filter", help="MLflow の検索式")
    q.add_argument("--max", help="取り出す上限")
    q.set_defaults(func=cmd_mlflow)

    q = ms.add_parser("run", help="実行 1 件")
    q.add_argument("run_id")
    q.set_defaults(func=cmd_mlflow)

    q = ms.add_parser("metric", help="指標 1 本の履歴")
    q.add_argument("run_id")
    q.add_argument("metric_key")
    q.add_argument("--max", help="取り出す上限")
    q.set_defaults(func=cmd_mlflow)

    return parser


def main(argv: list[str] | None = None) -> int:
    args = build_parser().parse_args(argv)
    try:
        data = args.func(args)
    except PhoneUnreachable as exc:
        print(json.dumps({"error": str(exc)}, ensure_ascii=False, indent=2))
        return 2
    except PhoneError as exc:
        print(json.dumps({"error": str(exc)}, ensure_ascii=False, indent=2))
        return 1
    except UsageError as exc:
        # 引数の誤りは理由だけを標準エラーへ（JSON じゃなくてもよい）
        print(f"引数が違います: {exc}", file=sys.stderr)
        return 2
    print(json.dumps(data, ensure_ascii=False, indent=2))
    return 0


if __name__ == "__main__":
    sys.exit(main())
