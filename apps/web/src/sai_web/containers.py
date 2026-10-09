"""プロジェクトの作成・コンテナの操作・API キーと Claude のトークン（実装計画 U09）。

多人数モードのサーバで、ログインした人が Web から次をできるようにする。

- 研究プロジェクトを作る（フォルダ・プロジェクト専用の HOME・コンテナ・`sai-link init`・
  `sai-paper init`・常駐の起動まで）。
- 自分のプロジェクトのコンテナを起動・停止し、状態を見る。
- 自分の API キーと Claude の長命トークン（`CLAUDE_CODE_OAUTH_TOKEN`）を保存する。
  保存すると、実行中でない自分のコンテナが新しい値で作り直される。

決め事:
- Docker は `docker` の CLI を `subprocess` で呼ぶ（Python の docker の SDK は足さない。
  依存を増やさない約束）。実行ファイルは `SAI_DOCKER_BIN`（既定 `docker`）。
- 置き場は共有契約 C4。`<U>` = `SAI_WEB_USERS_ROOT`。
  `<U>/<user>/projects/<user>-<project>/`（root）と
  `<U>/<user>/homes/<user>-<project>/`（コンテナの HOME）の 2 つだけを見せる。
- コンテナの形（名前・ラベル・イメージ・引数）は共有契約 C7・C8。
- キーとトークンは `<U>/<user>/secrets.env`（0600）に置き、`--env-file` で渡す。
  値は API でも画面でも返さない（返すのは設定済みかと末尾 4 文字だけ）。
- Claude のログイン（U10）は、利用者ごとのフォルダ `<U>/<user>/claude/` を、その人の全コンテナの
  `$HOME/.claude` に mount して共有する（共有契約 C-MOUNT）。`$HOME/.claude.json` は共有しない。
"""

from __future__ import annotations

import hashlib
import json
import logging
import os
import re
import secrets
import shutil
import subprocess
import tempfile
from pathlib import Path

from fastapi import HTTPException

from sai_web import control, registry, store

logger = logging.getLogger("sai_web")

# プロジェクトの短い名前（共有契約 C4）
_PROJECT_NAME_RE = re.compile(r"^[a-z0-9][a-z0-9-]{0,29}$")
# 管理者が操作するコンテナの名前（`sai-<利用者>-<プロジェクト>`。共有契約 C7）
_CONTAINER_NAME_RE = re.compile(r"^sai-[a-z][a-z0-9]{1,15}-[a-z0-9][a-z0-9-]{0,29}$")
# キーの値に許す文字（印字できる ASCII で空白と改行を含まない・400 文字まで）
_KEY_VALUE_RE = re.compile(r"^[\x21-\x7e]{1,400}$")

# 扱う名前はこの 5 つだけ（順番は画面の並び）
KEY_NAMES = (
    "CLAUDE_CODE_OAUTH_TOKEN",
    "OPENROUTER_API_KEY",
    "SEMANTICSCHOLAR_API_KEY",
    "OPENALEX_API_KEY",
    "NCBI_API_KEY",
)

# コンテナを作るときの既定値（共有契約 C8）
_DEFAULT_MEMORY = "8g"
_DEFAULT_CPUS = "2"
_DEFAULT_WEB_URL = "http://host.docker.internal:8200"
_DEFAULT_IMAGE = "sai-tree:latest"
# 会話の記録を共有のフォルダへ写し終えた印（HOME の `.claude` の中に作る）
_CLAUDE_COPIED_MARK = ".copied-to-shared"


# --- docker の呼び出し ---


def _last_reason(proc: subprocess.CompletedProcess) -> str:
    """標準エラーの最後の空でない行（失敗の理由）。無ければ空文字。"""
    for line in reversed((proc.stderr or "").splitlines()):
        text = line.strip()
        if text:
            return text
    return ""


def _looks_disconnected(stderr: str) -> bool:
    """docker のデーモンにつながらない失敗か（見つからない・動いていない）。"""
    text = (stderr or "").lower()
    hints = (
        "cannot connect to the docker daemon",
        "is the docker daemon running",
        "error during connect",
        "docker daemon is not running",
        "connection refused",
        "no such file or directory",
    )
    return any(hint in text for hint in hints)


def docker_bin() -> str:
    """`docker` の実行ファイル（`SAI_DOCKER_BIN`・既定 `docker`）。"""
    return (os.getenv("SAI_DOCKER_BIN") or "docker").strip() or "docker"


def image_name() -> str:
    """コンテナのイメージ（`SAI_IMAGE`・既定 `sai-tree:latest`）。"""
    return (os.getenv("SAI_IMAGE") or "").strip() or _DEFAULT_IMAGE


def _docker(args: list[str], timeout: float = 60.0) -> subprocess.CompletedProcess:
    """`docker` の CLI を呼ぶ（シェルは使わない）。

    - 実行ファイルは `docker_bin()`。
    - 終了コード 0 以外は `HTTPException(502, <標準エラーの最後の行>)`。
    - 見つからない・デーモンにつながらないときは
      `HTTPException(503, "docker が動いていません")`。
    """
    binary = docker_bin()
    try:
        proc = subprocess.run(
            [binary, *args], capture_output=True, text=True, timeout=timeout
        )
    except FileNotFoundError:
        raise HTTPException(status_code=503, detail="docker が動いていません")
    except subprocess.TimeoutExpired:
        raise HTTPException(status_code=503, detail="docker が動いていません")
    except OSError as exc:
        logger.warning("docker を呼べません: %s", exc)
        raise HTTPException(status_code=503, detail="docker が動いていません")
    if proc.returncode != 0:
        if _looks_disconnected(proc.stderr or ""):
            raise HTTPException(status_code=503, detail="docker が動いていません")
        raise HTTPException(status_code=502, detail=_last_reason(proc) or "docker が失敗しました")
    return proc


# --- コンテナの引数（共有契約 C8） ---


def container_name(project: str) -> str:
    """プロジェクト名からコンテナの名前を作る（`sai-<root のフォルダ名>`）。"""
    return f"sai-{project}"


def run_args(user: str, project_root: Path, project_home: Path, token: str) -> list[str]:
    """`docker run` の引数を作る純関数（共有契約 C8）。

    先頭は `run`（実行ファイル `docker` は `_docker` が付ける）。
    """
    root = Path(project_root)
    home = Path(project_home)
    name = root.name
    memory = (os.getenv("SAI_CONTAINER_MEMORY") or "").strip() or _DEFAULT_MEMORY
    cpus = (os.getenv("SAI_CONTAINER_CPUS") or "").strip() or _DEFAULT_CPUS
    web_url = (os.getenv("SAI_WEB_URL_FOR_CONTAINERS") or "").strip() or _DEFAULT_WEB_URL
    image = image_name()
    secrets_path = registry.user_dir(user) / "secrets.env"
    shared_claude = registry.user_dir(user) / "claude"
    return [
        "run",
        "-d",
        "--name",
        container_name(name),
        "--init",
        "--restart",
        "unless-stopped",
        "--memory",
        memory,
        "--memory-swap",
        memory,
        "--cpus",
        cpus,
        "--pids-limit",
        "1024",
        "--security-opt",
        "no-new-privileges",
        "--cap-drop",
        "ALL",
        "--add-host",
        "host.docker.internal:host-gateway",
        "-v",
        f"{root}:{root}",
        "-v",
        f"{home}:{home}",
        "-v",
        f"{shared_claude}:{home / '.claude'}",
        "-w",
        str(root),
        "-e",
        f"HOME={home}",
        "-e",
        f"SAI_PROJECT_ROOT={root}",
        "-e",
        f"SAI_WEB_URL={web_url}",
        "-e",
        f"SAI_WEB_INTERNAL_TOKEN={token}",
        "--env-file",
        str(secrets_path),
        "--label",
        f"sai.owner={user}",
        "--label",
        f"sai.project={name}",
        image,
    ]


def _new_internal_token() -> str:
    """コンテナと Web の内部の合言葉を作る（`sai_` で始まらない値）。"""
    token = secrets.token_urlsafe(32)
    while token.startswith("sai_"):
        token = secrets.token_urlsafe(32)
    return token


def ensure_claude_dir(user: str) -> Path:
    """利用者の Claude のログインの置き場 `<user_dir>/claude/`（0700）を、無ければ作る。"""
    path = registry.user_dir(user) / "claude"
    path.mkdir(parents=True, exist_ok=True)
    try:
        os.chmod(path, 0o700)
    except OSError:
        pass
    return path


def create_container(user: str, name: str) -> None:
    """プロジェクトのコンテナを `docker run` で作る（内部の合言葉も作り直す）。

    合言葉は `store.project_tokens` に SHA-256 だけを保存する。同じプロジェクトの
    古い行は `save_project_token` が消す。
    共有のログインの置き場を作り、HOME に既にある会話の記録を一度だけ写してから起こす。
    """
    token = _new_internal_token()
    store.save_project_token(hashlib.sha256(token.encode("utf-8")).hexdigest(), user, name)
    base = registry.user_dir(user)
    root = base / "projects" / name
    home = base / "homes" / name
    shared = ensure_claude_dir(user)
    _adopt_claude_history(home, shared)
    _docker(run_args(user, root, home, token), timeout=120.0)


# --- コンテナの状態と操作 ---


def _inspect_state(container: str) -> str:
    """コンテナの状態（`docker inspect -f '{{.State.Status}}'`）。

    コンテナが無い（inspect が 502 で失敗する）ときは `missing`。
    docker のデーモンにつながらない（503）ときはそのまま失敗させる。
    """
    try:
        proc = _docker(["inspect", "-f", "{{.State.Status}}", container], timeout=20.0)
    except HTTPException as exc:
        if exc.status_code == 502:
            return "missing"
        raise
    value = (proc.stdout or "").strip()
    if value == "running":
        return "running"
    if value in ("exited", "created", "paused", "dead", "restarting"):
        return "exited"
    return "missing"


def state(name: str) -> str:
    """プロジェクトのコンテナの状態（`running` / `exited` / `missing`）。"""
    return _inspect_state(container_name(name))


def _start_container(container: str) -> dict:
    """コンテナを起動する。無ければ作り直す（イメージを入れ替えたあとに使う）。"""
    if _inspect_state(container) == "missing":
        project = container[len("sai-") :]
        user = project.split("-", 1)[0]
        create_container(user, project)
    else:
        _docker(["start", container], timeout=60.0)
    return {"container": container, "state": _inspect_state(container)}


def _stop_container(container: str) -> dict:
    """コンテナを止める（20 秒待つ）。無ければ 404。"""
    if _inspect_state(container) == "missing":
        raise HTTPException(status_code=404, detail="コンテナが見つかりません")
    _docker(["stop", "-t", "20", container], timeout=60.0)
    return {"container": container, "state": _inspect_state(container)}


def start(name: str) -> dict:
    """プロジェクトのコンテナを起動する。"""
    return _start_container(container_name(name))


def stop(name: str) -> dict:
    """プロジェクトのコンテナを止める。"""
    return _stop_container(container_name(name))


def start_container(container: str) -> dict:
    """管理者用: コンテナの名前で起動する。"""
    return _start_container(container)


def stop_container(container: str) -> dict:
    """管理者用: コンテナの名前で止める。"""
    return _stop_container(container)


def check_container_name(container: str) -> str:
    """管理者が指定するコンテナの名前を検査する（違えば 400）。"""
    value = (container or "").strip()
    if not _CONTAINER_NAME_RE.match(value):
        raise HTTPException(status_code=400, detail="コンテナの名前が正しくありません")
    return value


# --- コンテナの一覧 ---


def _parse_labels(text: str) -> dict[str, str]:
    """`docker ps` の Labels 列（`k=v,k=v`）を辞書にする。"""
    labels: dict[str, str] = {}
    for item in (text or "").split(","):
        if "=" not in item:
            continue
        key, _, value = item.partition("=")
        key = key.strip()
        if key:
            labels[key] = value
    return labels


def _list_containers(owner: str | None = None) -> list[dict]:
    """`docker ps -a` の出力を読む（`sai.owner` のラベルで絞る）。"""
    args = ["ps", "-a", "--filter", "label=sai.owner"]
    if owner:
        args = ["ps", "-a", "--filter", f"label=sai.owner={owner}"]
    args += ["--format", "{{json .}}"]
    proc = _docker(args, timeout=30.0)
    rows: list[dict] = []
    for line in (proc.stdout or "").splitlines():
        line = line.strip()
        if not line:
            continue
        try:
            row = json.loads(line)
        except json.JSONDecodeError:
            continue
        if not isinstance(row, dict):
            continue
        labels = _parse_labels(str(row.get("Labels") or ""))
        rows.append(
            {
                "container": str(row.get("Names") or ""),
                "owner": labels.get("sai.owner") or "",
                "project": labels.get("sai.project") or "",
                "state": str(row.get("State") or ""),
                "status": str(row.get("Status") or ""),
            }
        )
    return rows


def list_all() -> list[dict]:
    """`sai.owner` のラベルがある全部のコンテナ（管理者の一覧）。"""
    return _list_containers()


# --- プロジェクトの作成 ---


def _ensure_secrets_file(user_dir: Path) -> Path:
    """`<user_dir>/secrets.env` を（無ければ）空で作る（0600）。"""
    path = user_dir / "secrets.env"
    if not path.exists():
        fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
        os.close(fd)
    try:
        os.chmod(path, 0o600)
    except OSError:
        pass
    return path


def _copy_missing(src: Path, dst: Path, skip: frozenset[str] = frozenset()) -> None:
    """`src` の中身を `dst` へ、`dst` に無いファイルだけ写す（上書きしない）。

    `skip` は（どの階層でも）名前の一致するファイルを写さない。
    """
    for root, _dirs, files in os.walk(src):
        rel = Path(root).relative_to(src)
        target_dir = dst / rel
        target_dir.mkdir(parents=True, exist_ok=True)
        for fname in files:
            target = target_dir / fname
            if fname in skip or target.exists():
                continue
            try:
                shutil.copy2(Path(root) / fname, target)
            except OSError as exc:
                logger.warning("ひな形を写せません: %s: %s", target, exc)


def _adopt_claude_history(home: Path, shared: Path) -> None:
    """HOME の `.claude` の会話の記録を共有のフォルダへ一度だけ写す（元は消さない）。

    `.credentials.json` は写さない（古い短命の token で、共有のログインを上書きしないため）。
    写し終えたら `.claude/.copied-to-shared` を作り、2 回目からは何もしない。
    """
    src = home / ".claude"
    mark = src / _CLAUDE_COPIED_MARK
    if src.is_symlink() or not src.is_dir() or mark.exists() or not any(src.iterdir()):
        return
    _copy_missing(src, shared, skip=frozenset({".credentials.json"}))
    try:
        mark.touch()
    except OSError as exc:
        logger.warning("写し終えの印を付けられません: %s: %s", mark, exc)


def _copy_skeleton(home: Path) -> None:
    """ひな形を HOME に写す（`SAI_WEB_SKEL_DIR`）。HOME に既にあるファイルは上書きしない。"""
    skel = (os.getenv("SAI_WEB_SKEL_DIR") or "").strip()
    if skel:
        src = Path(skel)
        if src.is_dir():
            _copy_missing(src, home)


def _remove_dirs(root: Path, home: Path) -> None:
    """失敗したプロジェクトのフォルダを消す。"""
    for path in (root, home):
        shutil.rmtree(path, ignore_errors=True)


def create_project(user: str, short: str) -> dict:
    """プロジェクトを作る（フォルダ → コンテナ → init 3 本）。

    どこかで失敗したら、コンテナを消し（`docker rm -f`）、root と home を消して、
    理由を返す（502）。名前の誤りは 400、同じ名前は 409。
    """
    short = (short or "").strip()
    if not _PROJECT_NAME_RE.match(short):
        raise HTTPException(
            status_code=400,
            detail="名前は英小文字・数字・ハイフンで 30 文字までにしてください",
        )
    name = f"{user}-{short}"
    base = registry.user_dir(user)
    root = base / "projects" / name
    home = base / "homes" / name
    if root.exists() or home.exists():
        raise HTTPException(status_code=409, detail="同じ名前のプロジェクトがあります")

    root.mkdir(parents=True, exist_ok=True)
    home.mkdir(parents=True, exist_ok=True)
    for path in (root, home):
        try:
            os.chmod(path, 0o700)
        except OSError:
            pass
    _ensure_secrets_file(base)
    _copy_skeleton(home)

    container = container_name(name)
    try:
        create_container(user, name)
        for command in (
            ["sai-link", "init", "--name", name],
            ["sai-paper", "init"],
            ["sai-link", "daemon", "start"],
        ):
            _docker(
                ["exec", "-i", "-w", str(root), "-e", f"SAI_PROJECT_ROOT={root}", container, *command],
                timeout=300.0,
            )
    except HTTPException:
        try:
            _docker(["rm", "-f", container], timeout=60.0)
        except HTTPException as exc:
            logger.warning("失敗したコンテナを消せません: %s: %s", container, exc.detail)
        _remove_dirs(root, home)
        raise
    return {"name": name, "container": container}


# --- API キーと Claude のトークン ---


def _secrets_path(user: str) -> Path:
    return registry.user_dir(user) / "secrets.env"


def read_keys(user: str) -> dict[str, str]:
    """`secrets.env` の `KEY=値` の行を読む（`#` の行と空行は飛ばす）。"""
    keys: dict[str, str] = {}
    try:
        text = _secrets_path(user).read_text(encoding="utf-8")
    except FileNotFoundError:
        return {}
    except OSError as exc:
        logger.warning("鍵のファイルを読めません: %s", exc)
        return {}
    for line in text.splitlines():
        line = line.strip()
        if not line or line.startswith("#") or "=" not in line:
            continue
        key, _, value = line.partition("=")
        key = key.strip()
        if key:
            keys[key] = value
    return keys


def _write_env_file(path: Path, keys: dict[str, str]) -> None:
    """`KEY=値` の行を一時ファイル（0600）に書いて rename する。"""
    order = {name: i for i, name in enumerate(KEY_NAMES)}
    lines = [
        f"{key}={keys[key]}"
        for key in sorted(keys, key=lambda n: order.get(n, len(order)))
        if keys.get(key)
    ]
    text = "".join(line + "\n" for line in lines)
    path.parent.mkdir(parents=True, exist_ok=True)
    fd, tmp = tempfile.mkstemp(dir=str(path.parent), prefix=".secrets-", suffix=".tmp")
    try:
        os.write(fd, text.encode("utf-8"))
        os.close(fd)
        os.chmod(tmp, 0o600)
        os.replace(tmp, path)
    except OSError as exc:
        try:
            os.close(fd)
        except OSError:
            pass
        try:
            os.unlink(tmp)
        except OSError:
            pass
        logger.warning("鍵のファイルを書けません: %s", exc)
        raise HTTPException(status_code=502, detail="鍵のファイルを書けませんでした")
    try:
        os.chmod(path, 0o600)
    except OSError:
        pass


def write_keys(user: str, changes: dict) -> dict[str, str]:
    """鍵を保存する。空文字の値は消す。名前か値が違えば 400。

    値は「印字できる ASCII で空白と改行を含まず 400 文字以下」。
    """
    if not isinstance(changes, dict):
        raise HTTPException(status_code=400, detail="保存する値を読み取れませんでした")
    current = read_keys(user)
    for name, value in changes.items():
        if name not in KEY_NAMES:
            raise HTTPException(status_code=400, detail=f"知らない名前です: {name}")
        if not isinstance(value, str):
            raise HTTPException(status_code=400, detail="値は文字列で指定してください")
        if value == "":
            current.pop(name, None)
            continue
        if not _KEY_VALUE_RE.match(value):
            raise HTTPException(
                status_code=400,
                detail="値は空白や改行を含まない 400 文字までの半角文字にしてください",
            )
        current[name] = value
    _write_env_file(_secrets_path(user), current)
    return current


def describe_keys(user: str) -> list[dict]:
    """画面に返す鍵の状態（値は返さない・末尾 4 文字だけ）。"""
    keys = read_keys(user)
    out: list[dict] = []
    for name in KEY_NAMES:
        value = keys.get(name) or ""
        out.append(
            {"name": name, "set": bool(value), "last4": value[-4:] if value else ""}
        )
    return out


# --- 鍵を変えたときの作り直し ---


def _running_count(user: str, project: str) -> int:
    """そのプロジェクトの実行中の件数（`control._project_summary`）。失敗は 0。"""
    root = registry.user_dir(user) / "projects" / project
    try:
        summary = control._project_summary(project, str(root))
    except HTTPException:
        return 0
    running = summary.get("running")
    if not isinstance(running, int) or isinstance(running, bool) or running < 0:
        return 0
    return running


def recreate_for_user(user: str) -> dict:
    """その人のコンテナを新しい鍵で作り直す（実行中でない物だけ）。

    - 実行中（`running` が 1 以上）のプロジェクトは作り直さず `pending` に入れる
      （作業を壊さない）。
    - それ以外は `docker rm -f` → `create_container`（主プロセスが `.sai` を見て
      常駐を起こす。共有契約 C7）。
    """
    recreated: list[str] = []
    pending: list[str] = []
    for row in _list_containers(owner=user):
        project = str(row.get("project") or "")
        if not project:
            continue
        if _running_count(user, project) > 0:
            pending.append(project)
            continue
        container = str(row.get("container") or container_name(project))
        try:
            _docker(["rm", "-f", container], timeout=60.0)
        except HTTPException as exc:
            logger.warning("古いコンテナを消せません: %s: %s", container, exc.detail)
        create_container(user, project)
        recreated.append(project)
    return {"recreated": recreated, "pending": pending}
