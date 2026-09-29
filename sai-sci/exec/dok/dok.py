"""高火力 DOK (さくらインターネット) の API を操作する CLI。sai-ssh から取り込んだ。

実験ループからは同じフォルダの adapter が `ssh-run` を切り離して呼ぶ（README.md の先頭を読む）。
単体でも使える。実行は `python3 dok.py <サブコマンド>`（このフォルダで）。

    python3 dok.py auth | prices | registry | run | status | cancel | list
    python3 dok.py ssh-key | ssh-run

DOK は従量課金 (実行秒数) なので、`run` は「起動 → 監視 → 必ず停止」までが
一続きの処理として書かれている。Ctrl-C でも SIGTERM でも、終了時にタスクを
止める。秘密 (アクセストークン・シークレット・パスワード) は
絶対に出力しない。値を表示するのは無い。名前だけを表示する。

`ssh-run` も同じ作りで、公開の PyTorch image を SSH 有効で起動し、
`bash -s` で手元のシェルスクリプトを流す。止めるのは `run` と同じ `finally`。
"""

from __future__ import annotations

import argparse
import base64
import json
import os
import re
import select
import signal
import subprocess
import sys
import time
from datetime import datetime
from pathlib import Path
from typing import Mapping, Sequence
from urllib import error, request

BASE_URL = "https://secure.sakura.ad.jp/cloud/zone/{zone}/api/managed-container/1.0"

DEFAULT_ZONE = "is1a"
DEFAULT_IMAGE = "test-saito.sakuracr.jp/dok-test:0.1"
DEFAULT_PLAN = "v100-32gb"
DEFAULT_REGISTRY_USER = "saito"
DEFAULT_MLFLOW_URL = "https://macbook-pro.tail3ab809.ts.net"
DEFAULT_EXPERIMENT = "dok"
DEFAULT_LIMIT_SEC = 900
DEFAULT_POLL_SEC = 5
MIN_LIMIT_SEC = 600

# ssh-run の既定。image は公開のもの (Docker Hub。レジストリ認証が要らない)。
DEFAULT_SSH_IMAGE = "pytorch/pytorch:2.8.0-cuda12.6-cudnn9-runtime"
DEFAULT_KEY_PATH = "~/.ssh/dok_key"
DEFAULT_KEY_NAME = "sai-ssh-dok"
# コンテナは `sleep` で保持し、そのあいだ ssh でスクリプトを流す。
DEFAULT_HOLD_SEC = 780
DEFAULT_SSH_TIMEOUT_SEC = 1500  # image の pull は課金対象外だが、数分かかることがある
# 起動直後は ssh がまだ受け付けないことがある。何回試すか。
SSH_CONNECT_RETRIES = 6
SSH_RETRY_INTERVAL_SEC = 5
SSH_TIMEOUT_CODE = -9
_DONE_RE = re.compile(rb"__DOK_DONE_(\d+)__")
_ANSI_RE = re.compile(rb"\x1b\[\?2004[hl]|\x1b\][^\x07]*\x07|\x1b\[K")
# ssh が「接続できなかった」ことを返す終了コード (255)。
SSH_CONNECT_FAILED = 255
# `~/.ssh/known_hosts` は汚さない。専用のファイルにだけ書く。
DEFAULT_KNOWN_HOSTS = "~/.ssh/known_hosts_dok"

# 監視が `--limit-sec` を超えても、実行時間は DOK 側で 900 秒で止まる。
# その 120 秒後を「手元で打ち切る」ときの境目にする。
TIMEOUT_MARGIN_SEC = 120
# cancel を出してから、終了状態になるまでに何秒待つか。
CANCEL_GRACE_SEC = 60

TERMINAL_STATUSES = frozenset({"error", "done", "aborted", "canceled"})

# 使う秘密の名前。大文字表記は表示用、小文字表記は load_secrets の戻り値のキー。
API_ACCESS_TOKEN = "SAKURA_API_ACCESS_TOKEN"
API_SECRET_KEY = "SAKURA_API_SECRET_KEY"
S3_ACCESS_KEY = "SAKURA_S3_ACCESS_KEY"
S3_SECRET_KEY = "SAKURA_S3_SECRET_KEY"
MLFLOW_BASIC_USER = "MLFLOW_BASIC_USER"
MLFLOW_BASIC_PASSWORD = "MLFLOW_BASIC_PASSWORD"
# レジストリのパスワードは環境変数だけから読む (.env からは読まない)。
REGISTRY_PASSWORD = "DOK_REGISTRY_PASSWORD"

# DOK API 自身の認証に使う名前 → 表示用の大文字表記。
API_SECRET_NAMES = {API_ACCESS_TOKEN: "sakura_api_access_token", API_SECRET_KEY: "sakura_api_secret_key"}


class SshWaitFailed(Exception):
    """タスクが ssh を使える状態にならずに終わったこと。値は持たない。"""


class MissingSecrets(Exception):
    """足りない秘密の名前だけを持つ。値は持たない。"""

    def __init__(self, names: Sequence[str]) -> None:
        self.names = list(names)
        super().__init__("足りない環境変数: " + ", ".join(self.names))


# ---------------------------------------------------------------- 通信


def _monotonic() -> float:
    """テストで差し替えられるよう、時刻取得を 1 か所にまとめる。"""
    return time.monotonic()


def _request(
    method: str,
    path: str,
    body: dict | None,
    auth: tuple[str, str],
    zone: str,
    timeout: int = 30,
) -> dict | None:
    """DOK API を 1 回呼ぶ。HTTP エラーは RuntimeError。

    例外文に Authorization ヘッダの値 (つまり秘密) は決して含めない。
    """
    url = BASE_URL.format(zone=zone) + path
    headers = {"Accept": "application/json"}
    data = None
    if body is not None:
        data = json.dumps(body).encode("utf-8")
        headers["Content-Type"] = "application/json"
    if auth:
        raw = f"{auth[0]}:{auth[1]}".encode("utf-8")
        headers["Authorization"] = "Basic " + base64.b64encode(raw).decode("ascii")
    req = request.Request(url, data=data, headers=headers, method=method)
    try:
        with request.urlopen(req, timeout=timeout) as resp:  # noqa: S310 - https 固定の API
            payload = resp.read()
    except error.HTTPError as exc:
        try:
            detail = exc.read().decode("utf-8", "replace")[:300]
        except Exception:  # noqa: BLE001 - 読めなくてもLocationは出す
            detail = ""
        raise RuntimeError(
            f"DOK API {method} {path} が HTTP {exc.code} を返した: {detail}"
        ) from None
    except error.URLError as exc:
        raise RuntimeError(f"DOK API {method} {path} に接続できない: {exc.reason}") from None
    if not payload:
        return None
    return json.loads(payload.decode("utf-8"))


def _fetch_text(url: str, timeout: int = 30) -> str:
    """署名付き URL の本文を取りに行く。"""

    req = request.Request(url, method="GET")
    with request.urlopen(req, timeout=timeout) as resp:  # noqa: S310 - DOK が返した URL
        return resp.read().decode("utf-8", "replace")


# ---------------------------------------------------------------- 秘密


def _parse_env_text(text: str) -> dict[str, str]:
    """`KEY=VALUE` を読む。`#` 行と空行は無視、値の前後の引用符は外す。"""
    parsed: dict[str, str] = {}
    for raw in text.splitlines():
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        if line.startswith("export "):
            line = line[len("export ") :].strip()
        if "=" not in line:
            continue
        key, _, value = line.partition("=")
        key = key.strip().lower()
        value = value.strip()
        if len(value) >= 2 and value[0] == value[-1] and value[0] in ("'", '"'):
            value = value[1:-1]
        if key:
            parsed[key] = value
    return parsed


def load_secrets(env_files: Sequence[str], environ: Mapping[str, str]) -> dict[str, str]:
    """env ファイルと環境変数から秘密を読む。返り値のキーはすべて小文字。

    環境変数を `--env-file` より優先する。キー名の大文字小文字は区別しない
    (.env に `SAKURA_API_SECRET_kEY` のような綴りゆれがあるため)。
    値はそのまま入る。呼び出した側が絶対に出力しないこと。
    """
    merged: dict[str, str] = {}
    for path in env_files:
        try:
            text = Path(path).read_text(encoding="utf-8")
        except OSError:
            continue
        merged.update(_parse_env_text(text))
    for key, value in environ.items():
        if value is None:
            continue
        merged[key.strip().lower()] = value.strip()
    return merged


def _require(secrets: Mapping[str, str], names: Sequence[str]) -> list[str]:
    """足りない名前を列挙する (値は覗かない)。"""
    return [name for name in names if not secrets.get(name.lower())]


_UUID_RE = re.compile(r"^[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}$")


def _api_auth(secrets: Mapping[str, str]) -> tuple[str, str]:
    missing = _require(secrets, list(API_SECRET_NAMES))
    if missing:
        raise MissingSecrets(missing)
    token, secret = (secrets[API_SECRET_NAMES[name]] for name in API_SECRET_NAMES)
    # アクセストークンは UUID 形式。.env で 2 つが逆に書かれていたら入れ替える。
    if not _UUID_RE.match(token) and _UUID_RE.match(secret):
        print("警告: SAKURA_API_ACCESS_TOKEN と SAKURA_API_SECRET_KEY が逆のようなので入れ替えて使う", file=sys.stderr)
        token, secret = secret, token
    return token, secret


def build_environment(
    secrets: Mapping[str, str], mlflow_url: str, experiment: str
) -> dict[str, str]:
    """コンテナに渡す environment を組み立てる (共有契約の表のとおり)。

    キーの名前と固定値は変更しない。`SAKURA_` で始まるキーは使わない
    (DOK の予約なので、渡すと衝突する)。
    """
    needed = [S3_ACCESS_KEY, S3_SECRET_KEY, MLFLOW_BASIC_USER, MLFLOW_BASIC_PASSWORD]
    missing = _require(secrets, needed)
    if missing:
        raise MissingSecrets(missing)
    return {
        "AWS_ACCESS_KEY_ID": secrets[S3_ACCESS_KEY.lower()],
        "AWS_SECRET_ACCESS_KEY": secrets[S3_SECRET_KEY.lower()],
        "AWS_DEFAULT_REGION": "us-east-1",
        "MLFLOW_S3_ENDPOINT_URL": "https://s3.isk01.sakurastorage.jp",
        "DOK_BUCKET": "hps-data-2026",
        "DOK_PREFIX": "saito260927",
        "MLFLOW_TRACKING_URI": mlflow_url,
        "MLFLOW_TRACKING_USERNAME": secrets[MLFLOW_BASIC_USER.lower()],
        "MLFLOW_TRACKING_PASSWORD": secrets[MLFLOW_BASIC_PASSWORD.lower()],
        "MLFLOW_EXPERIMENT_NAME": experiment,
        "DOK_REQUIRE_GPU": "1",
    }


# ---------------------------------------------------------------- run


def _get_task(task_id: str, auth: tuple[str, str], zone: str) -> dict:
    data = _request("GET", f"/tasks/{task_id}/", None, auth, zone)
    return data or {}


def _cancel_task(task_id: str, auth: tuple[str, str], zone: str) -> bool:
    """停止する。403 (既に終了) は無視。他は表示して 1 度だけ再試行する。

    戻り値: 停止できた、または既に終了していたなら True。
    """
    for attempt in (1, 2):
        try:
            _request("POST", f"/tasks/{task_id}/cancel/", None, auth, zone)
            print(f"cancel しました (task_id={task_id})")
            return True
        except RuntimeError as exc:
            if "403" in str(exc):
                print(f"既に終了しています (task_id={task_id})")
                return True
            print(f"cancel に失敗しました ({attempt} 回目): {exc}", file=sys.stderr)
    print(f"cancel できませんでした (task_id={task_id})", file=sys.stderr)
    return False


def _print_output(task_id: str, auth: tuple[str, str], zone: str) -> None:
    try:
        data = _request("GET", f"/tasks/{task_id}/download/output/", None, auth, zone) or {}
        url = data.get("url")
        if not url:
            print("出力を取得できなかった")
            return
        print(_fetch_text(url))
    except (RuntimeError, error.URLError, OSError) as exc:
        print(f"出力を取得できなかった: {exc}")


def _unit_price(plan: str, auth: tuple[str, str], zone: str) -> float | None:
    today = datetime.now()
    path = f"/unit_prices/?year={today.year}&month={today.month}&day={today.day}"
    try:
        data = _request("GET", path, None, auth, zone) or {}
    except RuntimeError as exc:
        print(f"単価を取得できなかった: {exc}", file=sys.stderr)
        return None
    for row in data.get("results", []):
        if row.get("plan") == plan:
            try:
                return float(row.get("price"))
            except (TypeError, ValueError):
                return None
    return None


def _print_cost(task: Mapping[str, object], plan: str, auth: tuple[str, str], zone: str) -> None:
    seconds = 0.0
    for container in task.get("containers") or []:
        try:
            seconds = float(container.get("execution_seconds") or 0)
        except (TypeError, ValueError):
            seconds = 0.0
    print(f"execution_seconds={seconds:g}")
    price = _unit_price(plan, auth, zone)
    if price is None:
        print("概算の金額は取得できなかった")
        return
    print(f"概算={seconds * price:.2f} 円 ({plan} {price:g} 円/秒)")


def _hostname_of(image: str) -> str:
    return image.split("/", 1)[0]


def _error_headline(exc: BaseException) -> str:
    """例外文から応答本文 (`: ` の後ろ) を落とした見出しを返す。"""
    return str(exc).split(": ", 1)[0]


def _find_registry(
    image: str, username: str | None, auth: tuple[str, str], zone: str
) -> str | None:
    """username が None なら hostname の一致だけで返す (run の場合)。"""
    data = _request("GET", "/registries/?page=1&page_size=100", None, auth, zone) or {}
    hostname = _hostname_of(image)
    for row in data.get("results", []):
        if row.get("hostname") != hostname:
            continue
        if username is not None and row.get("username") != username:
            continue
        return row.get("id")
    return None


def run_task(args: argparse.Namespace, secrets: Mapping[str, str]) -> int:
    """タスクを起動し、終了まで監視し、終了一定要る前に必ず止める。

    戻り値: 0 = done かつ exit_code 0、1 = それ以外、2 = 起動しなかった。
    """
    auth = _api_auth(secrets)
    zone = args.zone
    if args.limit_sec < MIN_LIMIT_SEC:
        print(f"--limit-sec は {MIN_LIMIT_SEC} 以上でなければならない", file=sys.stderr)
        return 2
    if args.poll_sec < 0:
        print("--poll-sec は 0 以上でなければならない", file=sys.stderr)
        return 2

    # 1) 実行中のタスクがあれば起動しない (多重起動と課金の暴走の防止)。
    running = _request("GET", "/tasks/?status=running&status=waiting", None, auth, zone) or {}
    active = running.get("results") or []
    if active:
        print("既に実行中のタスクがある。起動しない。")
        for row in active:
            print(f"  task_id={row.get('id')} status={row.get('status')} name={row.get('name')}")
        return 3

    # 2) レジストリの認証 (hostname 一致即可)。
    registry_id = _find_registry(args.image, None, auth, zone)
    if not registry_id:
        print(
            f"レジストリ認証が無い。先に `python3 dok.py registry` を実行せよ "
            f"(hostname={_hostname_of(args.image)}, user={DEFAULT_REGISTRY_USER})",
            file=sys.stderr,
        )
        return 2

    # 3) 環境変数。値は見せない。名前だけ見せる。
    environment = build_environment(secrets, args.mlflow_url, args.experiment)
    print("送る環境変数の名前: " + " ".join(sorted(environment)))

    # 4) 起動。ここから先は必ず停止する。
    body = {
        "name": args.name,
        "containers": [
            {
                "image": args.image,
                "registry": registry_id,
                "environment": environment,
                "plan": args.plan,
            }
        ],
        "tags": ["sai-ssh", "dok-test"],
        "execution_time_limit_sec": args.limit_sec,
    }
    created = _request("POST", "/tasks/", body, auth, zone) or {}
    task_id = created.get("id")
    if not task_id:
        print("タスクの起動に失敗した: id が返らなかった", file=sys.stderr)
        return 1
    print(f"task_id={task_id} status={created.get('status')}")

    state: dict[str, object] = {"task": {}}
    task: dict = {}
    try:
        task = _poll_task(task_id, auth, zone, args)
        state["task"] = task
    except TimeoutError as exc:
        # 打ち切ったので終了コード 1。下の finally で必ず止める。
        print(str(exc), file=sys.stderr)
    finally:
        # 正常終了・KeyboardInterrupt・SIGTERM・TimeoutError のどれでもここを通る。
        task = _stop_and_report(task_id, auth, zone, args, state)
        state["task"] = task

    status = str(task.get("status") or "")
    exit_code = _exit_code_of(task)
    print(f"status={status} exit_code={exit_code}")
    if status == "done" and exit_code == 0:
        return 0
    return 1


def _poll_task(
    task_id: str, auth: tuple[str, str], zone: str, args: argparse.Namespace
) -> dict:
    """終了状態になるまで `--poll-sec` ごとに監視する。"""
    started = _monotonic()
    last: str | None = None
    while True:
        task = _get_task(task_id, auth, zone)
        status = str(task.get("status") or "")
        if status != last:
            print(f"status={status} elapsed={_monotonic() - started:.0f}sec")
            last = status
        if status in TERMINAL_STATUSES:
            return task
        if _monotonic() - started > args.limit_sec + TIMEOUT_MARGIN_SEC:
            raise TimeoutError(
                f"task_id={task_id} が {args.limit_sec + TIMEOUT_MARGIN_SEC} 秒で終了しなかった"
            )
        time.sleep(args.poll_sec)


def _stop_and_report(
    task_id: str,
    auth: tuple[str, str],
    zone: str,
    args: argparse.Namespace,
    state: dict[str, object],
) -> dict:
    """必ずタスクを止めて、出力と実行秒数と概算の金額を出す。"""
    task = state.get("task") or {}
    try:
        current = _get_task(task_id, auth, zone)
        if current:
            task = current
    except RuntimeError as exc:
        print(f"状態を確認できなかった: {exc}", file=sys.stderr)

    if str(task.get("status") or "") not in TERMINAL_STATUSES:
        canceled = _cancel_task(task_id, auth, zone)
        deadline = _monotonic() + CANCEL_GRACE_SEC
        while True:
            try:
                task = _get_task(task_id, auth, zone)
            except RuntimeError as exc:
                print(f"状態を確認できなかった: {exc}", file=sys.stderr)
                canceled = False
                break
            if str(task.get("status") or "") in TERMINAL_STATUSES:
                break
            if _monotonic() > deadline:
                print(f"{CANCEL_GRACE_SEC} 秒待っても終了を確認できなかった", file=sys.stderr)
                canceled = False
                break
            time.sleep(args.poll_sec)
        # 呼び出し側 (ssh-run) が「止められたか」を判断できるように残す。
        state["cancel_ok"] = canceled
    else:
        state["cancel_ok"] = True

    _print_output(task_id, auth, zone)
    _print_cost(task, args.plan, auth, zone)
    return task


def _exit_code_of(task: Mapping[str, object]) -> int | None:
    for container in task.get("containers") or []:
        value = container.get("exit_code")
        if value is not None:
            try:
                return int(value)
            except (TypeError, ValueError):
                return None
    return None


# ---------------------------------------------------------------- ssh


def _pub_path(key_path: Path) -> Path:
    return key_path.with_name(key_path.name + ".pub")


def _read_pub_key(key_path: Path) -> str:
    """公開鍵ファイルの本文 (前後の空白を除いた 1 行) を読む。

    秘密鍵は読まない。読めるのは `<key>.pub` だけ。
    """
    try:
        text = _pub_path(key_path).read_text(encoding="utf-8")
    except OSError as exc:
        raise RuntimeError(f"公開鍵を読めなかった: {_pub_path(key_path)}") from exc
    lines = [line.strip() for line in text.splitlines() if line.strip()]
    if not lines:
        raise RuntimeError(f"公開鍵が空だった: {_pub_path(key_path)}")
    return lines[0]


def _pub_key_id(pub_line: str) -> str:
    """公開鍵を比較しやすい形に揃える (先頭の `type base64` の 2 語)。"""
    return " ".join(pub_line.split()[:2])


def _ensure_key_pair(key_path: Path, name: str) -> None:
    """秘密鍵が無ければ ssh-keygen で作る。あれば触らない。

    パスフレーズなしで作る (自動実行のため)。上書きはしない。
    """
    if key_path.exists():
        print(f"秘密鍵は既にあるので作らない: {key_path}")
        return
    if _pub_path(key_path).exists():
        print(
            f"公開鍵だけが残っている。秘密鍵が無いので作れない: {key_path}",
            file=sys.stderr,
        )
        raise RuntimeError(f"秘密鍵が無い: {key_path}")
    key_path.parent.mkdir(parents=True, exist_ok=True)
    print(f"秘密鍵をパスフレーズなしで作る: {key_path} (コメント: {name})")
    try:
        proc = subprocess.run(
            ["ssh-keygen", "-t", "ed25519", "-N", "", "-f", str(key_path), "-C", name],
            capture_output=True,
            text=True,
            timeout=30,
        )
    except (OSError, subprocess.SubprocessError) as exc:
        raise RuntimeError(f"ssh-keygen を実行できなかった: {exc}") from exc
    if proc.returncode != 0 or not key_path.exists():
        # stderr には鍵的文件への言及が混ざるので出さない。
        raise RuntimeError(f"ssh-keygen が失敗した (終了コード {proc.returncode})")
    # 秘密鍵のパーミッションを 600 にしておく (ssh の要求)。
    try:
        key_path.chmod(0o600)
    except OSError:
        pass


def _key_fingerprint(key_path: Path) -> str:
    """公開鍵の指紋 (SHA256) を 1 行で返す。秘密鍵には触れない。"""
    try:
        proc = subprocess.run(
            ["ssh-keygen", "-lf", str(_pub_path(key_path))],
            capture_output=True,
            text=True,
            timeout=30,
        )
    except (OSError, subprocess.SubprocessError) as exc:
        print(f"指紋を計算できなかった: {exc}", file=sys.stderr)
        return "(不明)"
    if proc.returncode != 0:
        return "(不明)"
    return proc.stdout.strip().splitlines()[0] if proc.stdout.strip() else "(不明)"


def _list_ssh_keys(auth: tuple[str, str], zone: str) -> list[dict]:
    data = _request("GET", "/ssh/keys/?page=1&page_size=100", None, auth, zone) or {}
    return [dict(row) for row in (data.get("results") or [])]


def _find_ssh_key(
    keys: list[dict], pub_line: str, active_only: bool = True
) -> dict | None:
    """同じ公開鍵のエントリを探す。`is_active` でフィルタできる。"""
    wanted = _pub_key_id(pub_line)
    for row in keys:
        if active_only and row.get("is_active") is False:
            continue
        if _pub_key_id(str(row.get("pub_key") or "")) == wanted:
            return row
    return None


def cmd_ssh_key(args: argparse.Namespace, secrets: Mapping[str, str]) -> int:
    """DOK 用の SSH 鍵を作り、公開鍵を DOK に登録する (冪等)。

    秘密鍵の内容は読まない・表示しない。表示するのは鍵のパス・公開鍵の指紋・
    DOK 側の id だけ。
    """
    auth = _api_auth(secrets)
    key_path = Path(args.key).expanduser()
    print("DOK 用の SSH 鍵 (パスフレーズなし。自動実行のため)")
    try:
        _ensure_key_pair(key_path, args.name)
        pub_line = _read_pub_key(key_path)
    except RuntimeError as exc:
        # 秘密鍵の生成失敗の stderr には鍵の記述が混ざるので、exc だけ出す。
        print(str(exc), file=sys.stderr)
        return 1
    print(f"鍵のパス: {key_path}")
    print(f"指紋: {_key_fingerprint(key_path)}")

    existing = _find_ssh_key(_list_ssh_keys(auth, args.zone), pub_line, active_only=False)
    if existing:
        print(f"ssh_key_id={existing.get('id')} (既に登録済み。登録し直さない)")
        return 0

    created = (
        _request(
            "POST",
            "/ssh/keys/",
            {"name": args.name, "pub_key": pub_line, "is_active": True},
            auth,
            args.zone,
        )
        or {}
    )
    if not created.get("id"):
        print("SSH 公開鍵の登録に失敗した: id が返らなかった", file=sys.stderr)
        return 1
    print(f"ssh_key_id={created['id']} (登録した)")
    return 0


def _ssh_endpoint(task: Mapping[str, object]) -> tuple[str, int] | None:
    """`containers[0].ssh` の (host_name, port)。未準備なら None。"""
    for container in task.get("containers") or []:
        ssh = container.get("ssh") or {}
        host = ssh.get("host_name")
        port = ssh.get("port")
        if not host or not port:
            continue
        try:
            return str(host), int(port)
        except (TypeError, ValueError):
            continue
    return None


def _wait_for_ssh(
    task_id: str, auth: tuple[str, str], zone: str, args: argparse.Namespace
) -> tuple[str, int]:
    """タスクが running になり、`ssh` の host_name と port が入るまで待つ。"""
    started = _monotonic()
    last: tuple[str, bool] | None = None
    while True:
        task = _get_task(task_id, auth, zone)
        status = str(task.get("status") or "")
        endpoint = _ssh_endpoint(task)
        # start_at が入るまでは image の pull 中で、ssh は開いてもシェルが無い (課金対象外)。
        containers = task.get("containers") or [{}]
        started_flag = bool(containers[0].get("start_at"))
        line = (status, bool(endpoint), started_flag)
        if line != last:
            print(
                f"status={status} ssh={'準備完了' if endpoint else '待ち'} "
                f"container={'起動' if started_flag else 'image 取得中'} "
                f"elapsed={_monotonic() - started:.0f}sec"
            )
            last = line
        if status in TERMINAL_STATUSES:
            raise SshWaitFailed(f"タスクが ssh を使える前に終わった (status={status})")
        if status == "running" and endpoint and started_flag:
            return endpoint
        if _monotonic() - started > args.ssh_timeout_sec:
            raise TimeoutError(
                f"task_id={task_id} が {args.ssh_timeout_sec} 秒で ssh の接続先 (host_name/port) を公開しなかった"
            )
        time.sleep(args.poll_sec)


def build_ssh_command(
    key_path: Path,
    port: int,
    host: str,
    task_id: str,
    known_hosts: Path,
    extra_options: Sequence[str] = (),
) -> list[str]:
    """DOK の対話シェルに入る ssh のコマンド (リモートコマンドは付けない)。

    実機で PTY が必要かどうか (要確認) は `extra_options` と
    `SSH_EXTRA_OPTIONS` の 1 か所だけ直せば済むようにしておく。
    `~/.ssh/known_hosts` は汚さない。
    """
    return [
        "ssh",
        "-i",
        str(key_path),
        "-p",
        str(port),
        "-o",
        "IdentitiesOnly=yes",
        "-o",
        "StrictHostKeyChecking=accept-new",
        "-o",
        f"UserKnownHostsFile={known_hosts}",
        "-o",
        "ServerAliveInterval=15",
        "-o",
        "ServerAliveCountMax=4",
        "-o",
        "BatchMode=yes",
        *SSH_EXTRA_OPTIONS,
        *extra_options,
        f"{task_id}@{host}",
    ]


# 実機 (2026-09-29) では PTY なしだと、ssh は終了コード 0 で終わるがスクリプトが
# 実行されなかった。そのため `-tt` で PTY を強制する。
SSH_EXTRA_OPTIONS: tuple[str, ...] = ("-tt",)


def _drive_shell(command: Sequence[str], script: bytes, timeout: float) -> int:
    """PTY のシェルにスクリプトを流し、終わりの目印 (`__DOK_DONE_<n>__`) を待つ。

    DOK の ssh は標準入力が閉じるとセッションも閉じる (実機 2026-09-29)。だから stdin を
    開いたままにして、スクリプトを base64 で送って `bash` で実行し、目印で終わりを知る。
    戻り値: スクリプトの終了コード。繋がらなければ 255、時間切れは SSH_TIMEOUT_CODE。
    """
    b64 = base64.b64encode(script).decode()
    # tty の 1 行は 4095 文字までなので、1000 文字ずつ送る。
    lines = ["stty -echo", "PS1=''", "rm -f /tmp/dok_run.b64"]
    lines += [f"printf %s '{b64[i:i + 1000]}' >> /tmp/dok_run.b64" for i in range(0, len(b64), 1000)]
    lines.append("base64 -d /tmp/dok_run.b64 > /tmp/dok_run.sh; bash /tmp/dok_run.sh; echo __DOK_DONE_$?__")
    proc = subprocess.Popen(command, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
    assert proc.stdin is not None and proc.stdout is not None
    os.set_blocking(proc.stdout.fileno(), False)
    deadline = time.time() + timeout
    seen = b""
    result: int | None = None
    try:
        time.sleep(2)  # シェルの立ち上がりを待つ (入力は tty に溜まるので厳密でなくてよい)
        proc.stdin.write(("\n".join(lines) + "\n").encode())
        proc.stdin.flush()
        while result is None and time.time() < deadline:
            ready, _, _ = select.select([proc.stdout], [], [], 0.5)
            if ready:
                chunk = proc.stdout.read(65536)
                if chunk:
                    seen = (seen + chunk)[-8192:]
                    sys.stdout.write(_ANSI_RE.sub(b"", chunk).decode(errors="replace"))
                    sys.stdout.flush()
                    match = _DONE_RE.search(seen)
                    if match:
                        result = int(match.group(1))
                elif chunk == b"" and proc.poll() is not None:
                    break
            if proc.poll() is not None and not ready:
                break
        if result is None:
            if time.time() >= deadline:
                return SSH_TIMEOUT_CODE
            return 255 if proc.returncode == 255 else 1
        try:
            proc.stdin.write(b"exit\n")
            proc.stdin.flush()
        except OSError:
            pass
        return result
    finally:
        try:
            proc.wait(timeout=3)
        except subprocess.TimeoutExpired:
            proc.kill()
            proc.wait()


def _run_ssh(
    args: argparse.Namespace, task_id: str, host: str, port: int, script: Path
) -> int:
    """ssh でスクリプトを流す。標準出力・標準エラーはそのまま流す。

    戻り値: ssh の終了コード (0 なら成功)。
    """
    key_path = Path(args.key).expanduser()
    known_hosts = Path(args.known_hosts).expanduser()
    known_hosts.parent.mkdir(parents=True, exist_ok=True)
    command = build_ssh_command(key_path, port, host, task_id, known_hosts)
    # コンテナは `sleep <hold-sec>` で止まるので、それより長く待たない。
    timeout = float(max(1, min(args.hold_sec, args.limit_sec) - 15))
    print(f"ssh でスクリプトを流す: {script} (cmd: {' '.join(command)})")
    for attempt in range(1, SSH_CONNECT_RETRIES + 1):
        if attempt > 1:
            print(f"接続に失敗したので {SSH_RETRY_INTERVAL_SEC} 秒待って試す ({attempt} 回目)", file=sys.stderr)
            time.sleep(SSH_RETRY_INTERVAL_SEC)
        try:
            code = _drive_shell(command, script.read_bytes(), timeout)
            if code == SSH_TIMEOUT_CODE:
                print(f"ssh が {timeout:g} 秒で終わらなかった (script={script})", file=sys.stderr)
                return 1
            proc = argparse.Namespace(returncode=code)
        except OSError as exc:
            print(f"ssh を実行できなかった: {exc}", file=sys.stderr)
            return 1
        if proc.returncode == 0:
            return 0
        if proc.returncode != SSH_CONNECT_FAILED:
            print(f"スクリプトの実行に失敗した (ssh の終了コード {proc.returncode})", file=sys.stderr)
            return proc.returncode
        print(f"接続できなかった (ssh の終了コード 255): {attempt} 回目", file=sys.stderr)
    return SSH_CONNECT_FAILED


def ssh_run(args: argparse.Namespace, secrets: Mapping[str, str]) -> int:
    """公開 image を SSH 有効で起動し、ssh でスクリプトを流し、必ず止める。

    戻り値: 0 = スクリプトが成功し、タスクも止まった、1 = それ以外、
    2 = 起動しなかった、3 = 実行中のタスクがある、4 = タスクを止められなかった。
    """
    auth = _api_auth(secrets)
    zone = args.zone

    # 1) 前確認。足りないならタスクを作らない。
    script = Path(args.script)
    if not script.is_file():
        print(f"--script が読めない: {script}", file=sys.stderr)
        return 2
    key_path = Path(args.key).expanduser()
    if not key_path.is_file():
        print(f"秘密鍵が無い: {key_path} (先に `python3 dok.py ssh-key` を実行せよ)", file=sys.stderr)
        return 2
    if args.limit_sec < MIN_LIMIT_SEC:
        print(f"--limit-sec は {MIN_LIMIT_SEC} 以上でなければならない", file=sys.stderr)
        return 2
    if args.hold_sec >= args.limit_sec:
        print("--hold-sec は --limit-sec より小さくしなければならない", file=sys.stderr)
        return 2
    if args.poll_sec < 0:
        print("--poll-sec は 0 以上でなければならない", file=sys.stderr)
        return 2

    # 2) 実行中のタスクがあれば起動しない (既存の run と同じ)。
    running = _request("GET", "/tasks/?status=running&status=waiting", None, auth, zone) or {}
    active = running.get("results") or []
    if active:
        print("既に実行中のタスクがある。起動しない。")
        for row in active:
            print(f"  task_id={row.get('id')} status={row.get('status')} name={row.get('name')}")
        return 3

    # 3) 公開鍵が DOK に「有効」として無いなら起動しない
    #    (タスク作成後に登録しても、そのタスクでは使えない)。
    try:
        pub_line = _read_pub_key(key_path)
    except RuntimeError as exc:
        print(f"{exc} (先に `python3 dok.py ssh-key` を実行せよ)", file=sys.stderr)
        return 2
    if _find_ssh_key(_list_ssh_keys(auth, zone), pub_line, active_only=True) is None:
        print(
            f"公開鍵が DOK に登録されていない。先に `python3 dok.py ssh-key` を実行せよ "
            f"(指紋: {_key_fingerprint(key_path)})",
            file=sys.stderr,
        )
        return 2

    # 4) 環境変数。値は見せない。名前だけ見せる。
    environment = build_environment(secrets, args.mlflow_url, args.experiment)
    print("送る環境変数の名前: " + " ".join(sorted(environment)))

    # 5) 起動。公開 image なのでレジストリ認証は不要 (registry を付けない)。
    body = {
        "name": args.name,
        "containers": [
            {
                "image": args.image,
                "plan": args.plan,
                "entrypoint": ["/bin/bash"],
                "command": ["-c", f"sleep {args.hold_sec}"],
                "environment": environment,
                "ssh": {"shell": "/bin/bash"},
            }
        ],
        "tags": ["sai-ssh", "dok-ssh"],
        "execution_time_limit_sec": args.limit_sec,
    }
    created = _request("POST", "/tasks/", body, auth, zone) or {}
    task_id = created.get("id")
    if not task_id:
        print("タスクの起動に失敗した: id が返らなかった", file=sys.stderr)
        return 1
    print(f"task_id={task_id} status={created.get('status')}")

    # ここから先は必ず停止する。
    state: dict[str, object] = {"task": {}, "cancel_ok": False}
    ssh_code = 1
    try:
        try:
            host, port = _wait_for_ssh(str(task_id), auth, zone, args)
            print(f"ssh の接続先: {task_id}@{host}:{port}")
            ssh_code = _run_ssh(args, str(task_id), host, port, script)
        except SshWaitFailed as exc:
            print(str(exc), file=sys.stderr)
    finally:
        task = _stop_and_report(str(task_id), auth, zone, args, state)
        state["task"] = task

    status = str(task.get("status") or "")
    print(f"status={status} ssh_exit_code={ssh_code}")
    if not state.get("cancel_ok"):
        print(
            f"警告: タスクを止められなかった。課金が続く可能性がある。"
            f"`python3 dok.py cancel {task_id}` を手で実行して確かめよ。",
            file=sys.stderr,
        )
        return 4
    return 0 if ssh_code == 0 else 1


# ---------------------------------------------------------------- サブコマンド


def cmd_auth(args: argparse.Namespace, secrets: Mapping[str, str]) -> int:
    auth = _api_auth(secrets)
    data = _request("GET", "/auth/", None, auth, args.zone) or {}
    account = data.get("account") or {}
    agreed = account.get("tos_agreed_at") or data.get("tos_agreed_at")
    code = account.get("account_code") or data.get("account_code")
    print(f"tos_agreed_at={agreed if agreed else '(未同意)'}")
    print(f"account_code={code if code else '(なし)'}")
    if not agreed:
        print("利用規約に同意していない。他の操作はできない。")
        print("同意は CLI では行わない: Web コンソールから本人が行うこと。")
    return 0


def cmd_prices(args: argparse.Namespace, secrets: Mapping[str, str]) -> int:
    auth = _api_auth(secrets)
    today = datetime.now()
    path = f"/unit_prices/?year={today.year}&month={today.month}&day={today.day}"
    data = _request("GET", path, None, auth, args.zone) or {}
    for row in data.get("results", []):
        print(f"{row.get('plan')}\t{row.get('price')} 円/秒")
    return 0


def cmd_registry(args: argparse.Namespace, secrets: Mapping[str, str]) -> int:
    auth = _api_auth(secrets)
    registry_id = _find_registry(args.image, args.registry_user, auth, args.zone)
    if registry_id:
        print(f"registry_id={registry_id} (既に登録済み)")
        return 0
    environ = args.environ
    password = None
    for key, value in environ.items():
        if key.strip().lower() == REGISTRY_PASSWORD.lower():
            password = value
            break
    if not password:
        print(f"環境変数 {REGISTRY_PASSWORD} が無い", file=sys.stderr)
        return 2
    try:
        created = (
            _request(
                "POST",
                "/registries/",
                {
                    "hostname": _hostname_of(args.image),
                    "username": args.registry_user,
                    "password": password,
                },
                auth,
                args.zone,
            )
            or {}
        )
    except RuntimeError as exc:
        # 応答本文にパスワードが混ざっている可能性があるので、本文は出さない。
        print(f"レジストリ認証の登録に失敗した: {_error_headline(exc)}", file=sys.stderr)
        return 1
    if not created.get("id"):
        print("レジストリ認証の登録に失敗した", file=sys.stderr)
        return 1
    print(f"registry_id={created['id']} (登録した)")
    return 0


def cmd_status(args: argparse.Namespace, secrets: Mapping[str, str]) -> int:
    auth = _api_auth(secrets)
    task = _get_task(args.task_id, auth, args.zone)
    print(f"task_id={task.get('id')} status={task.get('status')}")
    if task.get("error_message"):
        print(f"error_message={task.get('error_message')}")
    for container in task.get("containers") or []:
        print(
            f"  exit_code={container.get('exit_code')} "
            f"execution_seconds={container.get('execution_seconds')} "
            f"start_at={container.get('start_at')} stop_at={container.get('stop_at')}"
        )
    return 0


def cmd_cancel(args: argparse.Namespace, secrets: Mapping[str, str]) -> int:
    auth = _api_auth(secrets)
    _cancel_task(args.task_id, auth, args.zone)
    return 0


def cmd_list(args: argparse.Namespace, secrets: Mapping[str, str]) -> int:
    auth = _api_auth(secrets)
    data = _request("GET", "/tasks/?status=running&status=waiting", None, auth, args.zone) or {}
    rows = data.get("results") or []
    if not rows:
        print("実行中のタスクは無い")
        return 0
    for row in rows:
        print(f"task_id={row.get('id')} status={row.get('status')} name={row.get('name')}")
    return 0


def cmd_run(args: argparse.Namespace, secrets: Mapping[str, str]) -> int:
    return _with_sigterm(lambda: run_task(args, secrets))


def _with_sigterm(func):
    """SIGTERM で KeyboardInterrupt を起こし、func の finally を必ず通す。"""
    previous = _install_sigterm_handler()
    try:
        return func()
    finally:
        if previous is not None:
            try:
                signal.signal(signal.SIGTERM, previous)
            except (ValueError, OSError):
                pass


def cmd_ssh_run(args: argparse.Namespace, secrets: Mapping[str, str]) -> int:
    return _with_sigterm(lambda: ssh_run(args, secrets))


def _install_sigterm_handler():
    """SIGTERM でも KeyboardInterrupt を起こして、run の finally を必ず通す。"""

    def handler(signum, frame):  # noqa: ARG001
        raise KeyboardInterrupt

    try:
        return signal.signal(signal.SIGTERM, handler)
    except (ValueError, OSError):
        # main スレッド以外 (テストなど) では差し替えられない。
        return None


# ---------------------------------------------------------------- CLI


def default_env_files() -> list[str]:
    return [str(p) for p in (Path(".env"), Path("mlflow-server/.env")) if p.is_file()]


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        prog="python3 dok.py",
        description="高火力 DOK のタスクを「認証登録 → 起動 → 監視 → 必ず停止」まで操作する。",
    )
    parser.add_argument("--zone", default=DEFAULT_ZONE, help=f"DOK のゾーン (既定 {DEFAULT_ZONE})")
    parser.add_argument(
        "--env-file",
        action="append",
        default=None,
        help="読む .env のパス。複数回指定可。既定は ./.env と ./mlflow-server/.env の存在するもの。",
    )
    sub = parser.add_subparsers(dest="command")

    def add(name: str, help_text: str):
        """共通オプションはサブコマンドの後ろにも書けるようにする。"""
        p = sub.add_parser(name, help=help_text)
        p.add_argument("--zone", default=argparse.SUPPRESS, help=argparse.SUPPRESS)
        p.add_argument(
            "--env-file", action="append", default=argparse.SUPPRESS, help=argparse.SUPPRESS
        )
        return p

    add("auth", "認証情報を確認する (利用規約の同意はしない)")
    add("prices", "今日の単価を表示する (円/秒)")
    add("list", "実行中・待ちのタスクを表示する")

    p_registry = add("registry", "レジストリ認証を冪等に登録する")
    p_registry.add_argument("--image", default=DEFAULT_IMAGE, help="イメージ名")
    p_registry.add_argument(
        "--registry-user",
        default=DEFAULT_REGISTRY_USER,
        help=f"レジストリのユーザー名 (既定 {DEFAULT_REGISTRY_USER})",
    )

    p_run = add("run", "タスクを起動し、終了まで監視し、必ず止める")
    p_run.add_argument("--image", default=DEFAULT_IMAGE, help="イメージ名")
    p_run.add_argument("--plan", default=DEFAULT_PLAN, help=f"プラン (既定 {DEFAULT_PLAN})")
    p_run.add_argument(
        "--limit-sec",
        type=int,
        default=DEFAULT_LIMIT_SEC,
        help=f"実行時間の上限 (既定 {DEFAULT_LIMIT_SEC}、{MIN_LIMIT_SEC} 未満は不可)",
    )
    p_run.add_argument("--mlflow-url", default=DEFAULT_MLFLOW_URL, help="mlflow の URL")
    p_run.add_argument("--experiment", default=DEFAULT_EXPERIMENT, help="mlflow の実験名")
    p_run.add_argument("--poll-sec", type=float, default=DEFAULT_POLL_SEC, help="監視の間隔 (秒)")
    p_run.add_argument("--name", default=None, help="タスク名 (既定 dok-test-<日時>)")

    p_sshkey = add("ssh-key", "DOK 用の SSH 鍵を作り、公開鍵を DOK に登録する (冪等)")
    p_sshkey.add_argument(
        "--key", default=DEFAULT_KEY_PATH, help=f"秘密鍵のパス (既定 {DEFAULT_KEY_PATH})"
    )
    p_sshkey.add_argument(
        "--name",
        default=DEFAULT_KEY_NAME,
        help=f"DOK 側の鍵の名前 (既定 {DEFAULT_KEY_NAME})",
    )

    p_sshrun = add("ssh-run", "公開 image を SSH 有効で起動し、ssh でスクリプトを流す")
    p_sshrun.add_argument("--script", default=None, help="ssh の `bash -s` に流すシェルスクリプト")
    p_sshrun.add_argument(
        "--image", default=DEFAULT_SSH_IMAGE, help=f"公開イメージ (既定 {DEFAULT_SSH_IMAGE})"
    )
    p_sshrun.add_argument("--plan", default=DEFAULT_PLAN, help=f"プラン (既定 {DEFAULT_PLAN})")
    p_sshrun.add_argument(
        "--limit-sec",
        type=int,
        default=DEFAULT_LIMIT_SEC,
        help=f"実行時間の上限 (既定 {DEFAULT_LIMIT_SEC}、{MIN_LIMIT_SEC} 未満は不可)",
    )
    p_sshrun.add_argument(
        "--hold-sec",
        type=int,
        default=DEFAULT_HOLD_SEC,
        help=f"コンテナを保持する秒数 (既定 {DEFAULT_HOLD_SEC}、--limit-sec より小さいこと)",
    )
    p_sshrun.add_argument(
        "--key", default=DEFAULT_KEY_PATH, help=f"秘密鍵のパス (既定 {DEFAULT_KEY_PATH})"
    )
    p_sshrun.add_argument(
        "--known-hosts",
        default=DEFAULT_KNOWN_HOSTS,
        help=f"known_hosts のパス (`~/.ssh/known_hosts` は汚さない) (既定 {DEFAULT_KNOWN_HOSTS})",
    )
    p_sshrun.add_argument("--mlflow-url", default=DEFAULT_MLFLOW_URL, help="mlflow の URL")
    p_sshrun.add_argument("--experiment", default=DEFAULT_EXPERIMENT, help="mlflow の実験名")
    p_sshrun.add_argument("--poll-sec", type=float, default=DEFAULT_POLL_SEC, help="監視の間隔 (秒)")
    p_sshrun.add_argument(
        "--ssh-timeout-sec",
        type=int,
        default=DEFAULT_SSH_TIMEOUT_SEC,
        help=f"ssh が使える状態になるまでの待ち (既定 {DEFAULT_SSH_TIMEOUT_SEC})",
    )
    p_sshrun.add_argument("--name", default=None, help="タスク名 (既定 dok-ssh-<日時>)")

    p_status = add("status", "タスクの状態を表示する")
    p_status.add_argument("task_id")

    p_cancel = add("cancel", "タスクを止める")
    p_cancel.add_argument("task_id")

    return parser


def main(
    argv: Sequence[str] | None = None,
    environ: Mapping[str, str] | None = None,
    env_files: Sequence[str] | None = None,
) -> int:
    parser = build_parser()
    args = parser.parse_args(list(argv) if argv is not None else None)
    environ = os.environ if environ is None else environ
    args.environ = environ
    files = list(env_files) if env_files is not None else (args.env_file or default_env_files())
    secrets = load_secrets(files, environ)

    if args.command is None:
        parser.print_help()
        return 2

    handlers = {
        "auth": cmd_auth,
        "prices": cmd_prices,
        "registry": cmd_registry,
        "run": cmd_run,
        "ssh-key": cmd_ssh_key,
        "ssh-run": cmd_ssh_run,
        "status": cmd_status,
        "cancel": cmd_cancel,
        "list": cmd_list,
    }
    if args.command == "run" and args.name is None:
        args.name = "dok-test-" + datetime.now().strftime("%Y%m%d-%H%M%S")
    if args.command == "ssh-run" and args.name is None:
        args.name = "dok-ssh-" + datetime.now().strftime("%Y%m%d-%H%M%S")

    try:
        return handlers[args.command](args, secrets)
    except MissingSecrets as exc:
        print(str(exc), file=sys.stderr)
        return 2
    except TimeoutError as exc:
        print(str(exc), file=sys.stderr)
        return 1
    except KeyboardInterrupt:
        print("中断した (タスクは停止済みの予定)", file=sys.stderr)
        return 130
    except RuntimeError as exc:
        print(str(exc), file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
