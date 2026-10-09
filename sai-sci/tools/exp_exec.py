#!/usr/bin/env python3
"""実験のスクリプトを「実行先（target）」に投げる分配器。

エージェントは実行先の中身を知らない。この 1 本を呼ぶだけで、Mac 本体・LAN の PC・
クラウドのどれにも同じ形で投げられる。実行先は名前を持つフォルダで、増減はフォルダの
出し入れだけで済む（このファイルを直さない）。

呼び方（標準ライブラリだけ。Python 3.11 以上）:
    uv run --no-project --python ">=3.11" python "$SAI_PROJECT_ROOT/.sai/tools/exp_exec.py" \
        run <target> [--name 名前] -- <command...>     # 1 行目に job_id
        status <job_id> [--json]                       # state を出す
        started <job_id> [--timeout 600] [--poll 15] [--json]  # 最初の epoch の行が出るまで待つ
        logs <job_id> [-n N]                           # ログの末尾
        cancel <job_id>                                # 中止
        check <target>                                 # 接続と前提の確認
        list [--json]                                  # 実行先の一覧

実行先のフォルダ（`<project>/.sai/exec/<target>/`。無ければ `<sai_home>/exec/<target>/`）:
    target.toml   description・kind・enable・max_jobs と、[env]（実行先ごとの環境変数）
    adapter       実行ファイル 1 本。第 1 引数が動詞（run / status / logs / cancel / check）

adapter の約束:
    - 環境変数 EXP_JOB_ID（分配器が作る job_id）・EXP_JOB_NAME・EXP_TARGET・EXP_TARGET_DIR・EXP_MAX_JOBS が入る。
      target.toml の [env] も入る。cwd は呼んだ人の cwd（スクリプトのある所）。
    - `run <command...>`   コマンドを投げて、すぐ終了コード 0 で戻る（終わるのを待たない）。
    - `status`             標準出力に JSON 1 つ（{"state": "..."}）。state は STATES のどれか。
    - `logs [-n N]`        ログの末尾を標準出力へ。
    - `cancel`             中止する。
    - `check`              接続と前提を確かめる。使えれば終了コード 0、理由は標準エラーへ。
    - 同時実行数は adapter が守る（分配器は数えない。上限は EXP_MAX_JOBS）。

job_id と実行先の対応は `<sai_home>/exec/jobs/<job_id>.json` に残す（status で実行先を引くため）。
終了コード: 0 = 成功、1 = 失敗（実行先が見つからない・adapter が失敗など）、2 = 引数の誤り。
"""

from __future__ import annotations

import argparse
import http.client
import json
import os
import re
import secrets
import subprocess
import sys
import time
import tomllib
import urllib.request
from datetime import datetime, timezone
from pathlib import Path

# adapter が返してよい state（sai-ssh と同じ言葉。台帳・報告書の書き方を変えないため）
STATES = ("queued", "running", "done", "failed", "cancelled", "lost")
# 実行先の名前（先頭は英数字。`_example` のような下線始まりは実行先として扱わない）
NAME_RE = re.compile(r"^[A-Za-z0-9][A-Za-z0-9_-]{0,63}$")
JOB_RE = re.compile(r"^[A-Za-z0-9][A-Za-z0-9_-]{0,63}$")
# adapter の 1 回の呼び出しの上限（秒）。run は転送を含むので長め
TIMEOUT = {"run": 600, "status": 60, "logs": 60, "cancel": 60, "check": 120}


class ExecError(Exception):
    """利用者に見せる失敗（メッセージだけ出して終了コード 1）。"""


def sai_home() -> Path:
    """利用者全体の置き場（`SAI_HOME` があればそれ、無ければ `~/.sai`）。"""
    value = os.environ.get("SAI_HOME")
    return Path(value) if value else Path.home() / ".sai"


def project_root() -> Path:
    value = os.environ.get("SAI_PROJECT_ROOT")
    return Path(value) if value else Path.cwd()


def target_roots() -> list[Path]:
    """実行先を探す場所（先に見つかったほうを使う）。"""
    return [project_root() / ".sai" / "exec", sai_home() / "exec"]


def find_target(name: str) -> Path:
    if not NAME_RE.match(name):
        raise ExecError(f"実行先の名前が正しくありません: {name!r}（英数字と - _ だけ）")
    for root in target_roots():
        folder = root / name
        if (folder / "adapter").is_file():
            return folder
    raise ExecError(f"実行先が見つかりません: {name}（`list` で一覧を見る）")


def load_config(folder: Path) -> dict:
    path = folder / "target.toml"
    if not path.is_file():
        return {}
    try:
        return tomllib.loads(path.read_text(encoding="utf-8"))
    except (tomllib.TOMLDecodeError, OSError) as e:
        raise ExecError(f"{path} を読めません: {e}") from e


def target_env(config: dict) -> dict:
    """target.toml の [env] を足した環境変数（adapter を呼ぶときと check で同じ）。"""
    env = dict(os.environ)
    for key, value in (config.get("env") or {}).items():
        env[str(key)] = str(value)
    return env


def call_adapter(
    folder: Path, verb: str, args: list[str], job_id: str, name: str = ""
) -> subprocess.CompletedProcess:
    """adapter を呼ぶ。target.toml の [env] と EXP_* を足す。"""
    config = load_config(folder)
    env = target_env(config)
    if config.get("max_jobs") is not None:
        env["EXP_MAX_JOBS"] = str(config["max_jobs"])
    env.update(
        EXP_JOB_ID=job_id,
        EXP_JOB_NAME=name,
        EXP_TARGET=folder.name,
        EXP_TARGET_DIR=str(folder),
    )
    adapter = folder / "adapter"
    if not os.access(adapter, os.X_OK):
        raise ExecError(f"{adapter} に実行の権限がありません（chmod +x）")
    try:
        return subprocess.run(
            [str(adapter), verb, *args],
            env=env,
            capture_output=True,
            text=True,
            encoding="utf-8",
            errors="replace",  # adapter が UTF-8 でない出力をしても止まらない
            timeout=TIMEOUT[verb],
        )
    except subprocess.TimeoutExpired as e:
        raise ExecError(f"{folder.name} の {verb} が {TIMEOUT[verb]} 秒で終わりませんでした") from e


def jobs_dir() -> Path:
    return sai_home() / "exec" / "jobs"


def save_job(record: dict) -> None:
    jobs_dir().mkdir(parents=True, exist_ok=True)
    path = jobs_dir() / f"{record['job_id']}.json"
    tmp = path.with_suffix(".tmp")
    tmp.write_text(json.dumps(record, ensure_ascii=False, indent=2), encoding="utf-8")
    tmp.replace(path)


def load_job(job_id: str) -> dict:
    if not JOB_RE.match(job_id):
        raise ExecError(f"job_id が正しくありません: {job_id!r}")
    path = jobs_dir() / f"{job_id}.json"
    if not path.is_file():
        raise ExecError(f"job が見つかりません: {job_id}")
    return json.loads(path.read_text(encoding="utf-8"))


def fail_text(proc: subprocess.CompletedProcess) -> str:
    text = (proc.stderr or proc.stdout or "").strip().splitlines()
    return text[-1] if text else f"終了コード {proc.returncode}"


def cmd_run(args) -> int:
    folder = find_target(args.target)
    config = load_config(folder)
    if config.get("enable", True) is False:
        raise ExecError(f"実行先 {args.target} は無効です（target.toml の enable）")
    if not args.command:
        print("`--` の後に実行するコマンドを書いてください", file=sys.stderr)
        return 2
    job_id = "j" + datetime.now(timezone.utc).strftime("%y%m%d-%H%M%S") + "-" + secrets.token_hex(2)
    proc = call_adapter(folder, "run", args.command, job_id, args.name or "")
    if proc.returncode != 0:
        raise ExecError(f"投入に失敗しました（{args.target}）: {fail_text(proc)}")
    save_job(
        {
            "job_id": job_id,
            "target": args.target,
            "name": args.name or "",
            "command": args.command,
            "cwd": os.getcwd(),
            "submitted_at": datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
        }
    )
    print(job_id)
    return 0


def job_folder(job: dict) -> Path:
    return find_target(job["target"])


def cmd_status(args) -> int:
    job = load_job(args.job_id)
    proc = call_adapter(job_folder(job), "status", [], job["job_id"], job.get("name", ""))
    if proc.returncode != 0:
        raise ExecError(f"状態を取れません（{job['target']}）: {fail_text(proc)}")
    try:
        data = json.loads(proc.stdout)
        state = data["state"]
    except (json.JSONDecodeError, KeyError, TypeError) as e:
        raise ExecError(f"adapter の status が JSON ではありません: {proc.stdout.strip()[:120]!r}") from e
    if state not in STATES:
        raise ExecError(f"adapter が知らない state を返しました: {state!r}（{'/'.join(STATES)}）")
    data.update(job_id=job["job_id"], target=job["target"])
    print(json.dumps(data, ensure_ascii=False) if args.json else state)
    return 0


PROGRESS_RE = re.compile(r"^\[epoch \d+/\d+\]")
DEVICE_RE = re.compile(r"^device=(\S+)")
END_STATES = ("done", "failed", "cancelled", "lost")


def adapter_state(job: dict) -> str:
    proc = call_adapter(job_folder(job), "status", [], job["job_id"], job.get("name", ""))
    if proc.returncode != 0:
        raise ExecError(f"状態を取れません（{job['target']}）: {fail_text(proc)}")
    try:
        state = json.loads(proc.stdout)["state"]
    except (json.JSONDecodeError, KeyError, TypeError) as e:
        raise ExecError(f"adapter の status が JSON ではありません: {proc.stdout.strip()[:120]!r}") from e
    if state not in STATES:
        raise ExecError(f"adapter が知らない state を返しました: {state!r}（{'/'.join(STATES)}）")
    return state


def cmd_started(args) -> int:
    """ジョブの「始まり」（最初の epoch の行）が出るか、終わるか、時間切れになるまで待つ。"""
    job = load_job(args.job_id)
    t0 = time.monotonic()
    while True:
        state = adapter_state(job)
        proc = call_adapter(job_folder(job), "logs", ["-n", "200"], job["job_id"], job.get("name", ""))
        if proc.returncode != 0:
            raise ExecError(f"logs に失敗しました（{job['target']}）: {fail_text(proc)}")
        lines = proc.stdout.splitlines()
        device = next((m.group(1) for l in lines if (m := DEVICE_RE.match(l))), None)
        line = next((l for l in lines if PROGRESS_RE.match(l)), None)
        waited = time.monotonic() - t0
        if line is not None or state in END_STATES or waited >= args.timeout:
            break
        time.sleep(args.poll)
    started = line is not None
    result = {
        "job_id": job["job_id"],
        "state": state,
        "started": started,
        "device": device,
        "line": line,
        "waited_s": int(waited),
        "log_tail": "" if started else "\n".join(lines[-20:]),
    }
    print(json.dumps(result, ensure_ascii=False) if args.json else f"{state} {str(started).lower()}")
    return 0


def passthrough(job: dict, verb: str, extra: list[str]) -> int:
    proc = call_adapter(job_folder(job), verb, extra, job["job_id"], job.get("name", ""))
    sys.stdout.write(proc.stdout)
    if proc.returncode != 0:
        raise ExecError(f"{verb} に失敗しました（{job['target']}）: {fail_text(proc)}")
    return 0


def cmd_logs(args) -> int:
    return passthrough(load_job(args.job_id), "logs", ["-n", str(args.n)])


def cmd_cancel(args) -> int:
    return passthrough(load_job(args.job_id), "cancel", [])


def in_container(env: dict) -> bool:
    """Docker の中か（`/.dockerenv`、または sai-web が渡す内部の合言葉があるか）。値は見せない。"""
    return Path("/.dockerenv").exists() or bool(env.get("SAI_WEB_INTERNAL_TOKEN"))


def check_local(folder: Path, env: dict) -> None:
    """local の実行先で、MLflow と成果物の置き場を確かめる。実験の途中で BLOCKED になる前に止める。"""
    uri = env.get("MLFLOW_TRACKING_URI", "")
    if uri:
        try:
            with urllib.request.urlopen(uri.rstrip("/") + "/health", timeout=5):
                pass
        except (OSError, ValueError, http.client.HTTPException) as e:
            where = "コンテナの中では 127.0.0.1 はコンテナ自身です。" if in_container(env) else ""
            raise ExecError(
                f"MLflow（{uri}）に届きません: {getattr(e, 'reason', e)}。{where}"
                f"{folder / 'target.toml'} の [env] の MLFLOW_TRACKING_URI を直してください"
            ) from e
    location = env.get("EXP_ARTIFACT_LOCATION", "s3://mlflow-local/")
    if location.startswith("s3://") and not env.get("AWS_ACCESS_KEY_ID"):
        raise ExecError(
            f"成果物の置き場 {location} には s3 の鍵（AWS_ACCESS_KEY_ID など）が要ります。"
            "鍵を target.toml の [env] に足すか、EXP_ARTIFACT_LOCATION を file:// にしてください"
        )


def cmd_check(args) -> int:
    folder = find_target(args.target)
    proc = call_adapter(folder, "check", [], "-", "")
    sys.stdout.write(proc.stdout)
    if proc.returncode != 0:
        raise ExecError(f"{args.target} は使えません: {fail_text(proc)}")
    config = load_config(folder)
    if config.get("kind") == "local":
        check_local(folder, target_env(config))
    print(f"{args.target}: 使えます")
    return 0


def list_targets() -> list[dict]:
    rows, seen = [], set()
    for root in target_roots():
        if not root.is_dir():
            continue
        for folder in sorted(root.iterdir()):
            name = folder.name
            if name in seen or not NAME_RE.match(name) or not (folder / "adapter").is_file():
                continue
            seen.add(name)
            try:
                config = load_config(folder)
            except ExecError as e:
                config = {"description": str(e)}
            rows.append(
                {
                    "name": name,
                    "kind": config.get("kind", ""),
                    "enable": config.get("enable", True) is not False,
                    "max_jobs": config.get("max_jobs"),
                    "description": config.get("description", ""),
                    "path": str(folder),
                }
            )
    return rows


def cmd_list(args) -> int:
    rows = list_targets()
    if args.json:
        print(json.dumps({"targets": rows}, ensure_ascii=False))
        return 0
    if not rows:
        print("実行先がありません（`sai-paper sci init` が local を置きます）")
    for row in rows:
        mark = "有効" if row["enable"] else "無効"
        print(f"{row['name']}\t{mark}\t{row['kind']}\t{row['description']}")
    return 0


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description="実験のスクリプトを実行先に投げる")
    sub = parser.add_subparsers(dest="verb", required=True)

    run = sub.add_parser("run", help="スクリプトを投入する（1 行目に job_id）")
    run.add_argument("target")
    run.add_argument("--name", default="")
    run.set_defaults(func=cmd_run)

    status = sub.add_parser("status", help="ジョブの状態")
    status.add_argument("job_id")
    status.add_argument("--json", action="store_true")
    status.set_defaults(func=cmd_status)

    started = sub.add_parser("started", help="ジョブが始まった（最初の epoch の行が出た）かを確かめる")
    started.add_argument("job_id")
    started.add_argument("--timeout", type=float, default=600)
    started.add_argument("--poll", type=float, default=15)
    started.add_argument("--json", action="store_true")
    started.set_defaults(func=cmd_started)

    logs = sub.add_parser("logs", help="ログの末尾")
    logs.add_argument("job_id")
    logs.add_argument("-n", type=int, default=50)
    logs.set_defaults(func=cmd_logs)

    cancel = sub.add_parser("cancel", help="ジョブを中止する")
    cancel.add_argument("job_id")
    cancel.set_defaults(func=cmd_cancel)

    check = sub.add_parser("check", help="実行先の接続と前提を確かめる")
    check.add_argument("target")
    check.set_defaults(func=cmd_check)

    lst = sub.add_parser("list", help="実行先の一覧")
    lst.add_argument("--json", action="store_true")
    lst.set_defaults(func=cmd_list)
    return parser


def main(argv: list[str] | None = None) -> int:
    argv = list(sys.argv[1:] if argv is None else argv)
    # `--` より後ろは実行するコマンドそのもの。argparse に読ませず先に切り分ける
    command: list[str] = []
    if "--" in argv:
        cut = argv.index("--")
        argv, command = argv[:cut], argv[cut + 1 :]
    args = build_parser().parse_args(argv)
    args.command = command
    try:
        return args.func(args)
    except ExecError as e:
        print(f"エラー: {e}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
