#!/usr/bin/env python3
"""参照コードの道具。実験の計画要用に、既存のコードを探して中身の数値に出典を付ける。

Library（論文の本文の塊とメモ）にある参照実装のリンクを抜き、リポジトリを浅く取得して、
取得したコードから数値の定数と式の行を抜き出し、`train.py` の数値に出典の注記が付いているかを確かめる。

呼び方（標準ライブラリだけ。Python 3.11 以上）:
    uv run --no-project --python ">=3.11" python "$SAI_PROJECT_ROOT/.sai/tools/refcode.py" <命令> …

命令:
    links [--paper N]… [--json]            # Library から参照実装の在処を引く
    gh-search <検索語> [--language L] [--limit N] [--json]   # GitHub を検索する
    fetch <URL | modeldb:<id>> [--ref REF] [--json]         # リポジトリを浅く取得する
    params <パス> [--ext .h,.cpp,…] [--json]                # コードから数値の定数を抜く
    eqs <パス> [--ext .mod,.ode,…] [--json]                 # コードから式の行を出典つきで抜く
    check <train.py> [--json]                               # 数値に出典の注記があるか確かめる

置き場（利用者全体で共有する）:
    <sai_home>/refcode/github/<owner>/<repo>@<sha12>/    # GitHub から取ったコード
    <sai_home>/refcode/modeldb/<id>@<sha12>/              # ModelDB の GitHub ミラー
    <sai_home>/refcode/modeldb/<id>@v<ver_number>/        # ModelDB の zip
    取得したフォルダには必ず `.refcode.json` がある（`url`・`kind`・`id`・`sha`・`ref`・`fetched_at`・`license`）。

`sai_home()` は環境変数 `SAI_HOME`（無ければ `~/.sai`）。`project_root()` は環境変数
`SAI_PROJECT_ROOT`（無ければ作業中のフォルダ）。`links` は `sai-paper find`（実行ファイルは
環境変数 `SAI_PAPER_BIN`、無ければ PATH の `sai-paper`）を呼ぶだけで、`papr.db` を直接は開かない。

外部の命令は `git` と `sai-paper` だけ。取得は `git clone --depth 1` で行い、GitHub の API は
検索（`gh-search`）と ModelDB の版の確認にだけ使う。認証なしの API の上限（検索は 10 回/分）を
避けるため、取得では API を使わない。

`--json` のとき標準出力は JSON 1 つだけ。終了コードは 0 = 成功、1 = 失敗（理由は標準エラー）、
2 = 引数の誤り。
"""

from __future__ import annotations

import argparse
import ast
import json
import os
import re
import secrets
import shutil
import subprocess
import sys
import tokenize
import urllib.error
import urllib.parse
import urllib.request
import zipfile
from datetime import datetime, timezone
from pathlib import Path

# 数値の文字列（C 系・Python 系・mod・ode で使う共通形）
NUM = r"[-+]?(?:\d+\.?\d*|\.\d+)(?:[eE][-+]?\d+)?"
# 取得しない対象（URL を記録するだけにする）
NO_FETCH = ("Zenodo", "OSF", "gitlab", "bitbucket")

# links で Library に引く検索語（同じ見立ての語だけ。全文検索の式）
LINKS_QUERY = "github OR gitlab OR bitbucket OR modeldb OR senselab OR zenodo"

# 取得の待ち時間（秒）
TIMEOUT_LINKS = 120
TIMEOUT_HTTP = 30
TIMEOUT_API = 60
TIMEOUT_CLONE = 300
TIMEOUT_MODELDB = 300
# zip の取り込みの上限（バイト）
MAX_ZIP_BYTES = 200 * 1024 * 1024

# params が読む拡張子（既定）
DEFAULT_EXTS = (".h", ".hpp", ".c", ".cc", ".cpp", ".py", ".m", ".jl", ".hoc", ".mod", ".ode")
C_EXTS = (".h", ".hpp", ".c", ".cc", ".cpp")
SCRIPT_EXTS = (".py", ".jl", ".m", ".hoc")
# params が飛ばすフォルダと、読む前に捨てる大きさ
SKIP_DIRS = (".git", "node_modules", "__pycache__")
MAX_FILE_BYTES = 1024 * 1024

# check が見ない「雛形の名前」（sai-sci/template/exp-core.md の定数）
TEMPLATE_NAMES = frozenset(
    {
        "PROJECT", "SCI_NODE", "STUDY", "SPROUT_RUN_ID", "MODEL", "ACTION",
        "PARENT_BRANCH", "METRIC", "DIRECTION", "EPOCHS", "DEVICE", "EXPERIMENT",
        "ARTIFACT_LOCATION", "BRANCH_RUN_ID", "TRIAL_TIMEOUT_S", "EXP_TAGS", "RUN_TAGS",
    }
)
# check が見ない関数
TEMPLATE_FUNCS = frozenset({"pick_device", "main"})
# 自明な数値（これらは出典が要らない）
TRIVIAL_NUMS = frozenset({0.0, 1.0, 2.0, 0.5})


class RefcodeError(Exception):
    """利用者に見せる失敗（メッセージだけ出して終了コード 1）。"""


class ArgError(Exception):
    """引数の誤り（メッセージだけ出して終了コード 2）。"""


class RateLimited(Exception):
    """GitHub の検索の上限に当たった（`X-RateLimit-Reset` の UNIX 時刻。分からなければ None）。"""

    def __init__(self, reset: int | None = None) -> None:
        super().__init__("GitHub の検索の上限に当たった")
        self.reset = reset


def sai_home() -> Path:
    """利用者全体の置き場（`SAI_HOME` があればそれ、無ければ `~/.sai`）。"""
    value = os.environ.get("SAI_HOME")
    return Path(value) if value else Path.home() / ".sai"


def project_root() -> Path:
    value = os.environ.get("SAI_PROJECT_ROOT")
    return Path(value) if value else Path.cwd()


def refcode_root() -> Path:
    """取得した参照コードを置くフォルダ。"""
    return sai_home() / "refcode"


def paper_bin() -> str:
    """`sai-paper` の実行ファイル（`SAI_PAPER_BIN` が優先）。"""
    return os.environ.get("SAI_PAPER_BIN") or "sai-paper"


def emit(data, as_json: bool, table) -> None:
    """`--json` のときは JSON 1 つ、無ければ表を出す。"""
    if as_json:
        print(json.dumps(data, ensure_ascii=False))
    else:
        table(data)


# --------------------------------------------------------------------------
# links: Library から参照実装の在処を引く
# --------------------------------------------------------------------------

# GitHub の URL（tree の枝まで取る）
GH_RE = re.compile(
    r"(?:https://)?(?:www\.)?github\.com/([\w.-]+)/([\w.-]+)"
    r"(?:/tree/([\w.-]+)(?:/([\w./-]*))?)?"
)
# repo の直後に並んだ語（`.../NM Cortex SR.` → `NM_Cortex_SR` と `NM-Cortex-SR` の 2 案）
GH_CAND_RE = re.compile(r"\s+((?:[A-Z0-9][\w-]*)(?:\s+[A-Z0-9][\w-]*){0,2})(?=[.,)]|$)")
# ModelDB の 3 つの形
MDB1_RE = re.compile(r"modeldb\.(?:yale\.edu|science|sci\.yale\.edu)/(?:showmodel(?:\.cshtml|\.asp)?\?model=)?(\d+)")
MDB2_RE = re.compile(r"senselab\.med\.yale\.edu/ModelDB/showmodel(?:\.cshtml|\.asp)?\?model=(\d+)")
MDB3_RE = re.compile(r"modeldb[^.]{0,60}?accession\s*(?:no\.?|number|#)?\s*:?\s*(\d+)", re.IGNORECASE)
# Zenodo の 2 つの形
ZEN1_RE = re.compile(r"(?:doi\.org/)?10\.5281/zenodo\.(\d+)")
ZEN2_RE = re.compile(r"zenodo\.org/records?/(\d+)")
# その他の存放所
OTHER_RES = (
    ("gitlab", re.compile(r"gitlab\.com/([\w.-]+)/([\w.-]+)")),
    ("bitbucket", re.compile(r"bitbucket\.org/([\w.-]+)/([\w.-]+)")),
)


def clean_repo(name: str) -> str:
    """repo の末尾の `.` と `.git` を外す。"""
    name = name.rstrip(".")
    if name.endswith(".git"):
        name = name[:-4]
    return name.rstrip(".")


def fix_text(text: str) -> str:
    r"""PDF の抽出で崩れたリンクを、直せる範囲で直す。

    1. `\_` を `_` にする。
    2. `h?ttps?: // ` を `https://` にする（`ttps://` の欠けた h も補う）。
    3. `github.com/` の直後と `github.com/<owner>/` の直後の空白を消す。
    """
    text = text.replace("\\_", "_")
    text = re.sub(r"h?ttps?:\s*//\s*", "https://", text)
    text = re.sub(r"github\.com/([\w.-]+)/\s+", r"github.com/\1/", text)
    text = re.sub(r"github\.com/\s+", "github.com/", text)
    return text


def link(kind: str, url: str, raw: str, **extra) -> dict:
    """C3 の形のリンク 1 件を作る（無い項目は null / 空配列）。"""
    row = {
        "kind": kind,
        "url": url,
        "raw": raw,
        "model_id": extra.get("model_id"),
        "ref": extra.get("ref"),
        "subpath": extra.get("subpath"),
        "candidates": extra.get("candidates") or [],
    }
    return row


def extract_links(text: str) -> list[dict]:
    """直した文字列からリンクを抜く。同じ URL は 1 件だけ。"""
    fixed = fix_text(text)
    rows: list[dict] = []
    seen: set[str] = set()

    def push(row: dict) -> None:
        if row["url"] not in seen:
            seen.add(row["url"])
            rows.append(row)

    for m in GH_RE.finditer(fixed):
        owner, repo, ref, subpath = m.group(1), clean_repo(m.group(2)), m.group(3), m.group(4)
        if not repo:
            continue
        url = f"https://github.com/{owner}/{repo}"
        # 崩れた repo 名（空白で切られた）のときは、並んだ語から 2 案を出す
        candidates = []
        cm = GH_CAND_RE.match(fixed[m.end():])
        if cm:
            words = cm.group(1).split()
            candidates = [
                f"https://github.com/{owner}/{repo}_{'_'.join(words)}",
                f"https://github.com/{owner}/{repo}-{'-'.join(words)}",
            ]
        push(
            link(
                "github",
                url,
                m.group(0),
                ref=ref,
                subpath=(subpath.rstrip("/") if subpath else None),
                candidates=candidates,
            )
        )

    # ModelDB（3 つの形を全部見て、同じ ID は 1 件にまとめる）
    mdb_ids: list[str] = []
    for pattern in (MDB1_RE, MDB2_RE, MDB3_RE):
        mdb_ids.extend(pattern.findall(fixed))
    for mid in dict.fromkeys(mdb_ids):
        push(link("modeldb", f"https://modeldb.science/{mid}", fixed, model_id=int(mid)))

    for pattern in (ZEN1_RE, ZEN2_RE):
        for num in pattern.findall(fixed):
            push(link("zenodo", f"https://doi.org/10.5281/zenodo.{num}", fixed))

    for name, pattern in OTHER_RES:
        for a, b in pattern.findall(fixed):
            push(link("other", f"https://{name}.com/{a}/{clean_repo(b)}", fixed))
    return rows


def run_paper(args: list[str], timeout: int) -> str:
    """`sai-paper` を実行して標準出力を返す。"""
    try:
        proc = subprocess.run(
            [paper_bin(), *args],
            cwd=str(project_root()),
            capture_output=True,
            text=True,
            encoding="utf-8",
            errors="replace",
            timeout=timeout,
        )
    except FileNotFoundError as e:
        raise RefcodeError(f"{paper_bin()} が見つかりません（`SAI_PAPER_BIN` で実行ファイルを指定する）") from e
    except subprocess.TimeoutExpired as e:
        raise RefcodeError(f"{paper_bin()} が {timeout} 秒で終わりませんでした") from e
    if proc.returncode != 0:
        last = (proc.stderr or proc.stdout or "").strip().splitlines()
        raise RefcodeError(f"{paper_bin()} が失敗しました: {last[-1] if last else proc.returncode}")
    return proc.stdout


def find_chunks(papers: list[int]) -> list[dict]:
    """`sai-paper find` で、参照実装のリンクがありそうな塊を引く。"""
    out = run_paper(
        [
            "find", LINKS_QUERY,
            "--fts-expr", "--mode", "word", "--scope", "all",
            "--top-k", "1000", "--per-paper", "0", "--json",
        ],
        TIMEOUT_LINKS,
    )
    try:
        data = json.loads(out)
    except json.JSONDecodeError as e:
        raise RefcodeError(f"`find` の出力が JSON ではありません: {out.strip()[:120]!r}") from e
    if not isinstance(data, dict) or "hits" not in data:
        raise RefcodeError("`find` の出力に hits がありません")
    chunks = data["hits"]
    if not isinstance(chunks, list):
        raise RefcodeError("`find` の hits が配列ではありません")
    if papers:
        chunks = [c for c in chunks if c.get("paper_id") in papers]
    return chunks


def cmd_links(args) -> int:
    chunks = find_chunks(args.paper or [])
    links_out: list[dict] = []
    mentions: list[dict] = []
    seen: set[tuple] = set()
    for chunk in chunks:
        text = str(chunk.get("text") or "")
        rows = extract_links(text)
        for row in rows:
            key = (chunk.get("paper_id"), row["url"])
            if key in seen:
                continue
            seen.add(key)
            links_out.append(
                {
                    "paper_id": chunk.get("paper_id"),
                    "title": chunk.get("title"),
                    "cite": chunk.get("cite"),
                    "source_kind": chunk.get("source_kind"),
                    **row,
                }
            )
        if not rows:
            mentions.append(
                {
                    "paper_id": chunk.get("paper_id"),
                    "title": chunk.get("title"),
                    "cite": chunk.get("cite"),
                    "text": text,
                }
            )
    data = {"links": links_out, "mentions": mentions}

    def table(d: dict) -> None:
        for row in d["links"]:
            print(f"#{row['paper_id']}\t{row['kind']}\t{row['url']}\t{row['cite'] or ''}")
            for cand in row["candidates"]:
                print(f"  候補\t{cand}")
        for row in d["mentions"]:
            print(f"#{row['paper_id']}\t言及\t{row['cite'] or ''}\t{row['text'][:100]}")

    emit(data, args.json, table)
    return 0


# --------------------------------------------------------------------------
# gh-search: GitHub を検索する
# --------------------------------------------------------------------------


def http_get(url: str, headers: dict, timeout: int = TIMEOUT_HTTP) -> bytes:
    """HTTP の GET はこの 1 か所で呼ぶ（テストはこの関数を差し替える）。

    403 か 429（検索の上限）なら `RateLimited` を上げる。
    """
    request = urllib.request.Request(url, headers=headers)
    try:
        with urllib.request.urlopen(request, timeout=timeout) as resp:
            return resp.read()
    except urllib.error.HTTPError as e:
        if e.code in (403, 429):
            reset = None
            try:
                raw = e.headers.get("X-RateLimit-Reset")
                reset = int(raw) if raw else None
            except (AttributeError, TypeError, ValueError):
                reset = None
            raise RateLimited(reset) from e
        raise RefcodeError(f"GitHub の検索に失敗しました（HTTP {e.code}）") from e
    except urllib.error.URLError as e:
        raise RefcodeError(f"GitHub に接続できません: {e.reason}") from e


def gh_headers() -> dict:
    """GitHub の API に送るヘッダ。`GITHUB_TOKEN` があるときだけ認証を付ける。"""
    headers = {"Accept": "application/vnd.github+json", "User-Agent": "sai-refcode/1"}
    token = os.environ.get("GITHUB_TOKEN")
    if token:
        headers["Authorization"] = f"Bearer {token}"
    return headers


def cmd_gh_search(args) -> int:
    limit = max(1, min(args.limit, 50))
    query = args.query
    if args.language:
        query = f"{query} language:{args.language}"
    url = (
        "https://api.github.com/search/repositories?q="
        + urllib.parse.quote(query)
        + f"&sort=stars&order=desc&per_page={limit}"
    )
    try:
        raw = http_get(url, gh_headers(), TIMEOUT_HTTP)
    except RateLimited as e:
        if e.reset:
            when = datetime.fromtimestamp(e.reset, tz=timezone.utc).strftime("%Y-%m-%d %H:%M:%S UTC")
        else:
            when = "時刻不明 "
        print(
            f"エラー: GitHub の検索の上限に当たった。<{when}> 以降にやり直すか、GITHUB_TOKEN を設定する",
            file=sys.stderr,
        )
        return 1
    try:
        data = json.loads(raw)
    except json.JSONDecodeError as e:
        raise RefcodeError(f"GitHub の出力が JSON ではありません: {raw[:120]!r}") from e

    items = [
        {
            "full_name": item.get("full_name"),
            "url": item.get("html_url"),
            "description": item.get("description"),
            "stars": item.get("stargazers_count"),
            "language": item.get("language"),
            "pushed_at": item.get("pushed_at"),
            "license": (item.get("license") or {}).get("spdx_id"),
        }
        for item in (data.get("items") or [])
    ][:limit]

    def table(rows: list[dict]) -> None:
        for row in rows:
            print(f"{row['full_name']}\t{row['stars']}\t{row['language'] or '-'}\t{row['url']}")

    emit(items, args.json, table)
    return 0


# --------------------------------------------------------------------------
# fetch: リポジトリを浅く取得して commit を控える
# --------------------------------------------------------------------------


def run_git(args: list[str], timeout: int, cwd: Path | None = None) -> subprocess.CompletedProcess:
    """`git` の呼び出しはこの 1 か所。取れなかったものは空の標準出力で返る。"""
    return subprocess.run(
        ["git", *args],
        cwd=str(cwd) if cwd else None,
        capture_output=True,
        text=True,
        encoding="utf-8",
        errors="replace",
        timeout=timeout,
    )


def parse_github(text: str) -> dict | None:
    """GitHub の URL を `owner`・`repo`・`ref` に直す。無ければ None。"""
    fixed = fix_text(text)
    m = GH_RE.search(fixed)
    if not m:
        return None
    repo = clean_repo(m.group(2))
    if not repo:
        return None
    return {
        "kind": "github",
        "owner": m.group(1),
        "repo": repo,
        "url": f"https://github.com/{m.group(1)}/{repo}",
        "ref": m.group(3) or "",
    }


def parse_modeldb(text: str) -> dict | None:
    """ModelDB の指定（`https://modeldb.science/<id>` か `modeldb:<id>`）を ID にする。無ければ None。"""
    value = text.strip()
    if value.lower().startswith("modeldb:"):
        num = value.split(":", 1)[1].strip()
        return {"kind": "modeldb", "model_id": num} if num else None
    fixed = fix_text(value)
    for pattern in (MDB1_RE, MDB2_RE):
        m = pattern.search(fixed)
        if m:
            return {"kind": "modeldb", "model_id": m.group(1)}
    return None


def resolve_target(text: str) -> dict:
    """入口で受け付ける入力を決める（URL の検査はここだけで行う）。"""
    value = text.strip()
    if not value:
        raise ArgError("URL か modeldb:<id> を書いてください")
    if value.lower().startswith("modeldb:"):
        found = parse_modeldb(value)
        if not found or not found["model_id"].isdigit():
            raise ArgError(f"ModelDB の ID が正しくありません: {value!r}（modeldb:<数字>）")
        return found
    gh = parse_github(value)
    if gh:
        return gh
    mdb = parse_modeldb(value)
    if mdb:
        return mdb
    raise ArgError(
        f"取得しません: {value!r}（GitHub の URL か modeldb:<id> を書く。{'・'.join(NO_FETCH)} は取得せず、URL を記録するだけにする）"
    )


def remote_sha(url: str, ref: str) -> str:
    """`git ls-remote` で sha を取る。ref が無ければ HEAD。"""
    wanted = [ref] if ref else ["HEAD"]
    if ref:
        wanted += [f"refs/heads/{ref}", f"refs/tags/{ref}"]
    for want in wanted:
        proc = run_git(["ls-remote", url, want], TIMEOUT_CLONE)
        for line in (proc.stdout or "").splitlines():
            parts = line.split()
            if len(parts) >= 2 and parts[0].isalnum():
                return parts[0]
    raise RefcodeError(f"リポジトリか ref が見つかりません: {url}" + (f"（ref={ref}）" if ref else ""))


def fetch_git(url: str, dest_parent: Path, name: str, ref: str = "") -> str:
    """浅く clone して `dest_parent/name` に置き、HEAD の sha を返す。

    URL の検査はしない（入口の `resolve_target` が行う）。テストは `file://` の URL を渡せる。
    """
    dest_parent.mkdir(parents=True, exist_ok=True)
    dest = dest_parent / name
    args = ["clone", "--depth", "1", "--no-tags"]
    if ref:
        args += ["--branch", ref]
    args += [url, str(dest)]
    try:
        proc = run_git(args, TIMEOUT_CLONE)
    except subprocess.TimeoutExpired as e:
        shutil_rmtree(dest)
        raise RefcodeError(f"{url} の取得が {TIMEOUT_CLONE} 秒で終わりませんでした") from e
    if proc.returncode != 0 or not dest.is_dir():
        shutil_rmtree(dest)
        last = (proc.stderr or proc.stdout or "").strip().splitlines()
        raise RefcodeError(f"{url} を取得できません: {last[-1] if last else proc.returncode}")
    head = run_git(["rev-parse", "HEAD"], 30, cwd=dest)
    # 中身だけでよいので、取得の記録（`.git`）は残さない
    shutil_rmtree(dest / ".git")
    sha = (head.stdout or "").strip().splitlines()[:1]
    return sha[0] if sha else ""


def shutil_rmtree(path: Path) -> None:
    """フォルダを消す（無ければ何もしない）。"""
    shutil.rmtree(path, ignore_errors=True)


def read_license(folder: Path) -> str | None:
    """直下の `LICENSE*`・`COPYING*`・`LICENCE*` の 1 行目を返す。無ければ None。"""
    try:
        names = sorted(p.name for p in folder.iterdir() if p.is_file())
    except OSError:
        return None
    for name in names:
        if not name.upper().startswith(("LICENSE", "LICENCE", "COPYING")):
            continue
        try:
            for line in (folder / name).read_text(encoding="utf-8", errors="replace").splitlines():
                if line.strip():
                    return line.strip()[:120]
        except OSError:
            continue
    return None


def write_meta(dest: Path, record: dict) -> None:
    """取得したフォルダに `.refcode.json` を書く。"""
    (dest / ".refcode.json").write_text(json.dumps(record, ensure_ascii=False, indent=2), encoding="utf-8")


def count_files(dest: Path) -> int:
    """`.refcode.json` を除いたファイルの数を数える（フォルダも 1 件と数える）。"""
    total = 0
    for path in dest.rglob("*"):
        if path.name == ".refcode.json" or ".git" in path.parts:
            continue
        total += 1
    return total


def move_into_place(tmp_dir: Path, src: Path, dest: Path) -> None:
    """取得したフォルダを一時フォルダから外へ移す。"""
    dest.parent.mkdir(parents=True, exist_ok=True)
    try:
        os.replace(src, dest)
    except OSError as e:
        shutil_rmtree(tmp_dir)
        raise RefcodeError(f"{dest} に置けません: {e}") from e
    shutil_rmtree(tmp_dir)


def fetch_repo(url: str, parent: Path, name: str, kind: str, ident: str, ref: str = "") -> dict:
    """リポジトリを浅く取得して `parent/name@sha12` に置く。`fetch` と同じ形の辞書を返す。

    `url` の検査はしない（入口の `resolve_target` が行う）。テストは `file://` の URL を渡せる。
    """
    sha = remote_sha(url, ref)
    dest = parent / f"{name}@{sha[:12]}"
    if dest.is_dir():
        return done_record(url, kind, ident, sha, ref, dest, cached=True)
    parent.mkdir(parents=True, exist_ok=True)
    tmp = parent / f".tmp-{secrets.token_hex(4)}"
    try:
        head = fetch_git(url, tmp, "repo", ref)
    except RefcodeError:
        shutil_rmtree(tmp)
        raise
    # clone した HEAD の sha が 1. と違えば、その sha を使う
    if head and head != sha:
        sha = head
        dest = parent / f"{name}@{sha[:12]}"
        if dest.is_dir():
            shutil_rmtree(tmp)
            return done_record(url, kind, ident, sha, ref, dest, cached=True)
    move_into_place(tmp, tmp / "repo", dest)
    return done_record(url, kind, ident, sha, ref, dest, cached=False)


def fetch_github_repo(owner: str, repo: str, url: str, ref: str = "") -> dict:
    """GitHub のリポジトリを浅く取得する。"""
    return fetch_repo(url, refcode_root() / "github" / owner, repo, "github", f"{owner}/{repo}", ref)


def fetch_modeldb_github(model_id: str, ref: str = "") -> dict:
    """ModelDB の GitHub ミラーを取得する（置き先は `modeldb/`）。"""
    url = f"https://github.com/ModelDBRepository/{model_id}"
    return fetch_repo(url, refcode_root() / "modeldb", model_id, "modeldb", f"modeldb/{model_id}", ref)


def done_record(url: str, kind: str, ident: str, sha: str | None, ref: str, dest: Path, cached: bool) -> dict:
    """取得したフォルダに `.refcode.json` を書いて、C3 の形の辞書を返す。"""
    license_text = read_license(dest)
    write_meta(
        dest,
        {
            "url": url,
            "kind": kind,
            "id": ident,
            "sha": sha,
            "ref": ref or None,
            "fetched_at": datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
            "license": license_text,
        },
    )
    return {
        "url": url,
        "kind": kind,
        "id": ident,
        "sha": sha,
        "ref": ref or None,
        "path": str(dest),
        "cached": cached,
        "license": license_text,
        "files": count_files(dest),
    }


def http_download(url: str, dest: Path, timeout: int, limit: int = MAX_ZIP_BYTES) -> None:
    """大きなファイルを取り込む（`url` の通りに見て、上限を超えたら止める）。"""
    request = urllib.request.Request(url, headers={"User-Agent": "sai-refcode/1"})
    dest.parent.mkdir(parents=True, exist_ok=True)
    try:
        with urllib.request.urlopen(request, timeout=timeout) as resp, dest.open("wb") as out:
            total = 0
            while True:
                chunk = resp.read(1024 * 256)
                if not chunk:
                    break
                total += len(chunk)
                if total > limit:
                    out.close()
                    dest.unlink(missing_ok=True)
                    raise RefcodeError(f"{url} が上限（{limit} バイト）を越えました")
                out.write(chunk)
    except urllib.error.URLError as e:
        dest.unlink(missing_ok=True)
        raise RefcodeError(f"{url} を取れません: {e.reason}") from e


def safe_extract(zf: zipfile.ZipFile, dest: Path) -> None:
    """zip を展開する。`..` や絶対パスの名前は拒む。"""
    dest.mkdir(parents=True, exist_ok=True)
    for info in zf.infolist():
        name = info.filename
        if name.startswith("/") or ".." in Path(name).parts or Path(name).is_absolute():
            raise RefcodeError(f"zip の中の値が正しくありません: {name!r}")
        target = (dest / name).resolve()
        if not str(target).startswith(str(dest.resolve())):
            raise RefcodeError(f"zip の中の値が正しくありません: {name!r}")
    zf.extractall(dest)


def fetch_modeldb_zip(model_id: str) -> dict:
    """ModelDB の zip を取り込む（GitHub のミラーが無いとき）。"""
    url = f"https://modeldb.science/api/v1/models/{model_id}"
    try:
        info = json.loads(http_get(url, gh_headers(), TIMEOUT_API))
    except RateLimited:
        raise
    except (RefcodeError, json.JSONDecodeError) as e:
        raise RefcodeError(f"ModelDB {model_id} の版が分かりません: {e}") from e
    version = str(info.get("ver_number") or "unknown")
    parent = refcode_root() / "modeldb"
    dest = parent / f"{model_id}@v{version}"
    ident = f"modeldb/{model_id}"
    source = f"https://modeldb.science/download/{model_id}"
    if dest.is_dir():
        return done_record(source, "modeldb", ident, None, f"v{version}", dest, cached=True)
    tmp = parent / f".tmp-{secrets.token_hex(4)}"
    tmp.mkdir(parents=True, exist_ok=True)
    archive = tmp / "model.zip"
    http_download(source, archive, TIMEOUT_MODELDB)
    with zipfile.ZipFile(archive) as zf:
        safe_extract(zf, tmp / "x")
    archive.unlink(missing_ok=True)
    move_into_place(tmp, tmp / "x", dest)
    return done_record(source, "modeldb", ident, None, f"v{version}", dest, cached=False)


def cmd_fetch(args) -> int:
    target = resolve_target(args.url)
    if target["kind"] == "github":
        ref = args.ref or target.get("ref") or ""
        data = fetch_github_repo(target["owner"], target["repo"], target["url"], ref)
    else:
        model_id = target["model_id"]
        try:
            data = fetch_modeldb_github(model_id, args.ref or "")
        except RefcodeError:
            # GitHub のミラーが無ければ zip を使う
            data = fetch_modeldb_zip(model_id)

    def table(d: dict) -> None:
        mark = "（既にあった）" if d["cached"] else ""
        print(f"{d['id']}\t{d['sha'] or d['ref'] or '-'}\t{d['files']} ファイル\t{d['path']}{mark}")

    emit(data, args.json, table)
    return 0


# --------------------------------------------------------------------------
# params: コードから数値の定数を抜き出す
# --------------------------------------------------------------------------

C_ASSIGN_RE = re.compile(
    r"^\s*(?:(?:static|const|constexpr|inline)\s+)*"
    r"(?:double|float|int|long|unsigned|auto)?\s*"
    r"(?P<name>[A-Za-z_]\w*)\s*=\s*(?P<value>" + NUM + r")\s*[;,]"
)
SCRIPT_ASSIGN_RE = re.compile(
    r"^\s*(?P<name>[A-Za-z_]\w*)\s*=\s*(?P<value>" + NUM + r")\s*(?:[;#%]|//|$)"
)
SCRIPT_DICT_RE = re.compile(r"['\"](?P<name>\w+)['\"]\s*:\s*(?P<value>" + NUM + r")")
MOD_ASSIGN_RE = re.compile(
    r"^\s*(?P<name>\w+)\s*=\s*(?P<value>" + NUM + r")\s*(?:\((?P<unit>[^)]*)\))?"
)
ODE_PAIR_RE = re.compile(r"(?P<name>[A-Za-z_]\w*)\s*=\s*(?P<value>" + NUM + r")")
ODE_HEADS = ("par", "param", "p", "number", "n")
SCRIPT_COMMENTS = ("#", "%", "//")


def strip_c_comment(rest: str) -> str | None:
    """行の残りから `//` か `/* */` の注記を取る。"""
    line = rest.find("//")
    block = rest.find("/*")
    if block != -1 and (line == -1 or block < line):
        end = rest.find("*/", block)
        text = rest[block + 2 : end if end != -1 else None]
        return text.strip() or None
    if line != -1:
        return rest[line + 2 :].strip() or None
    return None


def strip_script_comment(text: str) -> str | None:
    """行から `#`・`%`・`//` の後ろの注記を取る（記号は残さない）。"""
    cut = len(text)
    mark_len = 0
    for mark in SCRIPT_COMMENTS:
        pos = text.find(mark)
        if pos != -1 and pos < cut:
            cut = pos
            mark_len = len(mark)
    return text[cut + mark_len :].strip() or None


def strip_colon_comment(text: str) -> str | None:
    """行から `:` の後ろの注記を取る（`.mod` の書き方）。"""
    pos = text.find(":")
    return text[pos + 1 :].strip() if pos != -1 else None


def read_text(path: Path) -> str | None:
    """ファイルを読む。読めなければ None。"""
    try:
        return path.read_text(encoding="utf-8", errors="replace")
    except OSError:
        return None


def hits_in_c(text: str, file_label: str, src_of) -> list[dict]:
    rows = []
    for n, line in enumerate(text.splitlines(), 1):
        m = C_ASSIGN_RE.match(line)
        if not m:
            continue
        comment = strip_c_comment(line[m.end():])
        rows.append(make_hit(m.group("name"), m.group("value"), None, comment, file_label, n, src_of))
    return rows


def hits_in_script(text: str, file_label: str, src_of, ext: str) -> list[dict]:
    rows = []
    for n, line in enumerate(text.splitlines(), 1):
        comment = strip_script_comment(line)
        m = SCRIPT_ASSIGN_RE.match(line)
        if m:
            rows.append(make_hit(m.group("name"), m.group("value"), None, comment, file_label, n, src_of))
        if ext == ".py":
            # 辞書の値も同じ行から抜く
            for dm in SCRIPT_DICT_RE.finditer(line):
                rows.append(
                    make_hit(dm.group("name"), dm.group("value"), None, comment, file_label, n, src_of)
                )
    return rows


def hits_in_mod(text: str, file_label: str, src_of) -> list[dict]:
    rows = []
    inside = False
    depth = 0
    for n, line in enumerate(text.splitlines(), 1):
        stripped = line.strip()
        if not inside:
            if re.match(r"^PARAMETER\s*\{", stripped):
                inside, depth = True, 1
            continue
        depth += line.count("{") - line.count("}")
        m = MOD_ASSIGN_RE.match(line)
        if m:
            unit = (m.group("unit") or "").strip() or None
            rows.append(
                make_hit(m.group("name"), m.group("value"), unit, strip_colon_comment(line), file_label, n, src_of)
            )
        if depth <= 0:
            inside = False
    return rows


def hits_in_ode(text: str, file_label: str, src_of) -> list[dict]:
    rows = []
    for n, line in enumerate(text.splitlines(), 1):
        stripped = line.strip()
        head = stripped.split(None, 1)[0] if stripped else ""
        if head not in ODE_HEADS:
            continue
        rest = stripped[len(head) :]
        comment = strip_colon_comment(rest)
        for m in ODE_PAIR_RE.finditer(rest):
            rows.append(make_hit(m.group("name"), m.group("value"), None, comment, file_label, n, src_of))
    return rows


def make_hit(name: str, value: str, unit: str | None, comment: str | None, file_label: str, line: int, src_of) -> dict:
    """params の出力 1 行（C3 の形）。"""
    try:
        number = float(value)
    except ValueError:
        number = None
    return {
        "name": name,
        "value": value,
        "number": number,
        "unit": unit,
        "comment": comment,
        "file": file_label,
        "line": line,
        "src": src_of(line),
    }


def find_origin(start: Path) -> tuple[str, Path] | None:
    """ファイルの親をたどって `.refcode.json` を探す。無ければ None。"""
    for folder in [start, *start.parents]:
        meta = folder / ".refcode.json"
        if meta.is_file():
            try:
                return json.loads(meta.read_text(encoding="utf-8")), folder
            except (OSError, json.JSONDecodeError):
                return None
    return None


def src_maker(path: Path, root: Path):
    """`src`（`id@sha12:path:line` か `file:line`）を作る関数を返す。"""
    origin = find_origin(path.parent)
    try:
        label = str(path.relative_to(root))
    except ValueError:
        label = str(path)
    if not origin:
        return lambda line: f"{label}:{line}"
    meta, folder = origin
    ident = str(meta.get("id") or "")
    sha = meta.get("sha")
    version = str(sha)[:12] if sha else str(meta.get("ref") or "")
    try:
        inner = str(path.relative_to(folder))
    except ValueError:
        inner = path.name
    return lambda line: f"{ident}@{version}:{inner}:{line}"


def walk_files(path: Path, exts: tuple[str, ...]) -> list[Path]:
    """ファイルかフォルダを、読む対象のファイルの列にする。"""
    if path.is_file():
        return [path] if path.suffix in exts else []
    rows = []
    for root, dirs, files in os.walk(path):
        dirs[:] = [d for d in dirs if d not in SKIP_DIRS]
        for name in sorted(files):
            child = Path(root) / name
            if child.suffix not in exts:
                continue
            try:
                if child.stat().st_size > MAX_FILE_BYTES:
                    continue
            except OSError:
                continue
            rows.append(child)
    return rows


def scan_file(path: Path, root: Path) -> list[dict]:
    """1 個のファイルから数値の定数を抜く。"""
    text = read_text(path)
    if text is None:
        return []
    src_of = src_maker(path, root)
    label = src_of(0).rsplit(":", 1)[0] if not find_origin(path.parent) else src_of(0).split(":", 2)[-1]
    label = file_label(path, root)
    ext = path.suffix
    if ext in C_EXTS:
        return hits_in_c(text, label, src_of)
    if ext in SCRIPT_EXTS:
        return hits_in_script(text, label, src_of, ext)
    if ext == ".mod":
        return hits_in_mod(text, label, src_of)
    if ext == ".ode":
        return hits_in_ode(text, label, src_of)
    return []


def file_label(path: Path, root: Path) -> str:
    """`params` の `file`（`path` からの相対）。"""
    try:
        return str(path.relative_to(root))
    except ValueError:
        return path.name


def cmd_params(args) -> int:
    path = Path(args.path)
    if not path.exists():
        raise RefcodeError(f"パスが見つかりません: {path}")
    exts = tuple(
        e if e.startswith(".") else f".{e}" for e in (x.strip() for x in args.ext.split(",")) if e
    ) or DEFAULT_EXTS
    root = path if path.is_dir() else path.parent
    rows: list[dict] = []
    for target in walk_files(path, exts):
        rows.extend(scan_file(target, root))
    rows.sort(key=lambda r: (r["file"], r["line"], r["name"]))

    def table(items: list[dict]) -> None:
        for row in items:
            tail = f"  // {row['comment']}" if row["comment"] else ""
            unit = f" ({row['unit']})" if row["unit"] else ""
            print(f"{row['src']}\t{row['name']} = {row['value']}{unit}{tail}")

    emit(rows, args.json, table)
    return 0


# --------------------------------------------------------------------------
# eqs: コードから式の行を抜き出す
# --------------------------------------------------------------------------

# `.mod` の「式が書けるブロック」（見出しの最初の語。`{` で中が変わる）
MOD_BLOCKS = frozenset(
    {
        "DERIVATIVE", "BREAKPOINT", "KINETIC", "PROCEDURE",
        "FUNCTION", "INITIAL", "NET_RECEIVE", "LINEAR", "NONLINEAR",
    }
)
# `.ode` の「代入以外」と分かる行頭の語
ODE_SKIP_HEADS = frozenset(
    {
        "par", "param", "p", "number", "n", "init", "i", "@", "done",
        "#", "table", "wiener", "global", "bdry", "set",
    }
)
# `d<名前>/dt = …`（Brian2 の式の文字列・Julia・注釈の中の式）
DXDT_RE = re.compile(r"\bd([A-Za-z_]\w*)\s*/\s*dt\b[^=\n]*=")
# 代入（`=`・`+=`・`-=`）。比較（`==`・`<=`・`>=`・`!=`）は数えない
ASSIGN_OP_RE = re.compile(r"(?<![=!<>])[-+]?=(?!=)")
# 更新式に必ず出る `dt` の掛け算
EULER_DT_RE = re.compile(r"\bdt\b\s*\*|\*\s*\(?\s*\bdt\b")
# コメントだけの行の先頭（この行は式として出さない）
COMMENT_HEADS = ("#", "%", "//", "/*", "*")
# 1 行から出す text の上限（文字）
MAX_EQ_TEXT = 300


def is_comment_only(text: str) -> bool:
    """注記だけの行かどうか。"""
    return text.startswith(COMMENT_HEADS)


def make_eq(file_label: str, line: int, block: str | None, kind: str, text: str, src_of) -> dict:
    """eqs の出力 1 件（C1 の形）。"""
    body = text.strip()
    if len(body) > MAX_EQ_TEXT:
        body = body[:MAX_EQ_TEXT]
    return {
        "file": file_label,
        "line": line,
        "block": block,
        "kind": kind,
        "text": body,
        "src": src_of(line),
    }


def hits_in_mod_eqs(text: str, file_label: str, src_of) -> list[dict]:
    """`.mod` から、式が書けるブロックの中の文の行を抜く。

    中は `{` と `}` の数で追うので、ブロックの中に `if (…) { … }` が入れ子でも区別できる。
    """
    rows: list[dict] = []
    block: str | None = None
    depth = 0
    for n, line in enumerate(text.splitlines(), 1):
        stripped = line.strip()
        if block is None:
            head = stripped.split(None, 1)[0] if stripped else ""
            if head in MOD_BLOCKS and "{" in stripped:
                block = stripped.replace("{", "", 1).strip()
                depth = stripped.count("{") - stripped.count("}")
                if depth <= 0:
                    block = None
            continue
        depth += stripped.count("{") - stripped.count("}")
        if not stripped or stripped in ("{", "}") or stripped.startswith(":") or is_comment_only(stripped):
            if depth <= 0:
                block = None
            continue
        rows.append(make_eq(file_label, n, block, "mod", stripped, src_of))
        if depth <= 0:
            block = None
    return rows


def hits_in_ode_eqs(text: str, file_label: str, src_of) -> list[dict]:
    """`.ode` から、式か更新の行を抜く（`par`・`init` などは出さない）。"""
    rows: list[dict] = []
    for n, line in enumerate(text.splitlines(), 1):
        stripped = line.strip()
        if not stripped or is_comment_only(stripped):
            continue
        head = stripped.split(None, 1)[0] if stripped else ""
        if head in ODE_SKIP_HEADS:
            continue
        if "=" not in stripped:
            continue
        rows.append(make_eq(file_label, n, None, "ode", stripped, src_of))
    return rows


def hits_in_code_eqs(text: str, file_label: str, src_of) -> list[dict]:
    """C 系とスクリプトから、`d<x>/dt` と `dt` の入った更新式の行を抜く。

    関数の範囲までは追わない。取りこぼしより拾いすぎを選ぶ（読むのは LLM なので余分な行は読み飛ばせる）。
    """
    rows: list[dict] = []
    for n, line in enumerate(text.splitlines(), 1):
        stripped = line.strip()
        if not stripped or is_comment_only(stripped):
            continue
        if DXDT_RE.search(stripped):
            kind = "dxdt"
        elif ASSIGN_OP_RE.search(stripped) and EULER_DT_RE.search(stripped):
            kind = "euler"
        else:
            continue
        rows.append(make_eq(file_label, n, None, kind, stripped, src_of))
    return rows


def scan_eqs_file(path: Path, root: Path) -> list[dict]:
    """1 個のファイルから式の行を抜く。"""
    text = read_text(path)
    if text is None:
        return []
    src_of = src_maker(path, root)
    label = file_label(path, root)
    ext = path.suffix
    if ext == ".mod":
        return hits_in_mod_eqs(text, label, src_of)
    if ext == ".ode":
        return hits_in_ode_eqs(text, label, src_of)
    if ext in C_EXTS or ext in SCRIPT_EXTS:
        return hits_in_code_eqs(text, label, src_of)
    return []


def cmd_eqs(args) -> int:
    path = Path(args.path)
    if not path.exists():
        raise RefcodeError(f"パスが見つかりません: {path}")
    exts = tuple(
        e if e.startswith(".") else f".{e}" for e in (x.strip() for x in args.ext.split(",")) if e
    ) or DEFAULT_EXTS
    root = path if path.is_dir() else path.parent
    rows: list[dict] = []
    for target in walk_files(path, exts):
        rows.extend(scan_eqs_file(target, root))
    rows.sort(key=lambda r: (r["file"], r["line"]))

    def table(items: list[dict]) -> None:
        for row in items:
            print(f"{row['src']}\t[{row['block'] or row['kind']}] {row['text']}")

    emit(rows, args.json, table)
    return 0


# --------------------------------------------------------------------------
# check: train.py の数値に出典の注記があるか確かめる
# --------------------------------------------------------------------------

REF_SRC_RE = re.compile(
    r"^([\w.-]+)/([\w.-]+)@([0-9a-f]{7,40}|v\d+):([^\s:]+):(\d+(?:-\d+)?)$"
)
CHUNK_SRC_RE = re.compile(r"^#\d+(?:\.\d+)*$")
F_SRC_RE = re.compile(r"^F\d+$")
SPEC_SRC = "仕様書"
GUESS_SRC = "目安"
SOURCE_KINDS = ("ref", "chunk", "f", "spec", "guess")


def numbers_in(node: ast.AST) -> list[tuple[int, float]]:
    """その文のなかに現れる数を（行番号, 値）で返す。負の数は 1 つとして数える。

    bool と文字列は数えない（`-65` は UnaryOp として 1 つだけ出す）。
    """
    operands = {
        id(child.operand)
        for child in ast.walk(node)
        if isinstance(child, ast.UnaryOp) and isinstance(child.op, ast.USub)
    }
    rows: list[tuple[int, float]] = []
    for child in ast.walk(node):
        line = getattr(child, "lineno", None)
        if isinstance(child, ast.Constant) and isinstance(child.value, (int, float)) and not isinstance(child.value, bool):
            if id(child) not in operands:
                rows.append((line, float(child.value)))
        elif (
            isinstance(child, ast.UnaryOp)
            and isinstance(child.op, ast.USub)
            and isinstance(child.operand, ast.Constant)
            and isinstance(child.operand.value, (int, float))
            and not isinstance(child.operand.value, bool)
        ):
            rows.append((line, -float(child.operand.value)))
    return rows


def is_trivial(value: float) -> bool:
    """自明な数値かどうか（0・1・2・0.5 は出典が要らない）。"""
    return abs(value) in TRIVIAL_NUMS


def nontrivial_lines(node: ast.AST) -> set[int]:
    """その文のなかに自明でない数が現れる行の集合。"""
    return {line for line, value in numbers_in(node) if not is_trivial(value)}


def has_number(node: ast.AST) -> bool:
    """そのなかに自明でない数があるか。"""
    return any(not is_trivial(value) for _, value in numbers_in(node))


def comment_sources(text: str) -> dict[int, list[str]]:
    """`# src:` の注記を、行ごとに中身の出典へ割る。"""
    rows: dict[int, list[str]] = {}
    try:
        tokens = tokenize.generate_tokens(iter(text.splitlines(True)).__next__)
        for token in tokens:
            if token.type != tokenize.COMMENT:
                continue
            pos = token.string.find("src:")
            if pos == -1:
                continue
            for part in token.string[pos + 4 :].split(";"):
                value = part.strip()
                if value:
                    rows.setdefault(token.start[0], []).append(value)
    except (tokenize.TokenError, IndentationError):
        for n, line in enumerate(text.splitlines(), 1):
            pos = line.find("src:")
            if pos == -1:
                continue
            for part in line[pos + 4 :].split(";"):
                value = part.split("#", 1)[0].strip()
                if value:
                    rows.setdefault(n, []).append(value)
    return rows


def classify_source(source: str) -> str | None:
    """出典が C4 の 5 つの形のどれかならその名前を返す。違えば None。"""
    if REF_SRC_RE.match(source):
        return "ref"
    if CHUNK_SRC_RE.match(source):
        return "chunk"
    if F_SRC_RE.match(source):
        return "f"
    if source == SPEC_SRC:
        return "spec"
    if source == GUESS_SRC:
        return "guess"
    return None


def has_memory_word(source: str) -> bool:
    """出典が「記憶」か `memory` を含んでいるか。"""
    low = source.lower()
    return "記憶" in source or "memory" in low


def assigned_names(node: ast.AST) -> list[str]:
    """代入文が持つ名前。"""
    targets = node.targets if isinstance(node, ast.Assign) else [node.target]
    names = []
    for target in targets:
        if isinstance(target, ast.Name):
            names.append(target.id)
        elif isinstance(target, (ast.Tuple, ast.List)):
            names.extend(e.id for e in target.elts if isinstance(e, ast.Name))
    return names


def comment_only_lines(text: str) -> set[int]:
    """コメントだけの行の集合（コードの後ろに付いた注記は行に数えない）。

    文の「すぐ上の行の注記」を数えるのは、この行のときだけにする。
    """
    return {n for n, line in enumerate(text.splitlines(), 1) if line.strip().startswith("#")}


def command_check(args) -> int:
    path = Path(args.path)
    if not path.is_file():
        raise RefcodeError(f"ファイルが見つかりません: {path}")
    text = read_text(path)
    if text is None:
        raise RefcodeError(f"{path} を読みません")
    try:
        tree = ast.parse(text)
    except SyntaxError as e:
        raise RefcodeError(f"{path} が Python として読めません: {e}") from e

    sources = comment_sources(text)
    alone = comment_only_lines(text)
    errors: list[dict] = []
    guesses: list[dict] = []
    counts = {kind: 0 for kind in SOURCE_KINDS}

    # R3: 出典そのものの形と「記憶」を見る
    for line in sorted(sources):
        for source in sources[line]:
            if has_memory_word(source):
                errors.append({"line": line, "name": "src", "reason": f"「記憶」を使った出典は使えない: {source}"})
                continue
            kind = classify_source(source)
            if kind is None:
                errors.append({"line": line, "name": "src", "reason": f"出典の形が分かりません: {source}"})
                continue
            counts[kind] += 1
            if kind == "guess":
                guesses.append({"line": line, "name": name_at_line(tree, line), "src": source})

    # R1: 最上位の代入（自明でない数値を含む行には # src: が要る）
    for node in tree.body:
        if not isinstance(node, (ast.Assign, ast.AnnAssign)):
            continue
        names = assigned_names(node)
        if names and all(name in TEMPLATE_NAMES for name in names):
            continue
        label = names[0] if names else "<代入>"
        lines = sorted(nontrivial_lines(node))
        first = node.lineno
        for line in lines:
            # 文の 1 行目だけは、すぐ上の行がコメントだけの行なら、その注記でもよい
            above = line - 1 in alone and bool(sources.get(line - 1))
            ok = bool(sources.get(line)) or (line == first and above)
            if not ok:
                errors.append({"line": line, "name": label, "reason": "数値に出典の注記が無い（`# src:` を付ける）"})

    # R2: 最上位の関数（pick_device と main を除く）
    for node in tree.body:
        if not isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef)):
            continue
        if node.name in TEMPLATE_FUNCS:
            continue
        if not has_number(node):
            continue
        span = set(range(node.lineno, (node.end_lineno or node.lineno) + 1))
        if any(line in span for line in sources):
            continue
        errors.append(
            {
                "line": node.lineno,
                "name": node.name,
                "reason": "関数の中の数値に出典の注記が無い（`# src:` を 1 つ以上付ける）",
            }
        )

    data = {"ok": not errors, "errors": errors, "guesses": guesses, "counts": counts}

    def table(d: dict) -> None:
        for row in d["errors"]:
            print(f"{row['line']}: {row['name']}: {row['reason']}")
        c = d["counts"]
        print(
            f"出典: ref {c['ref']} / chunk {c['chunk']} / F {c['f']} / 仕様書 {c['spec']} / 目安 {c['guess']}"
        )

    emit(data, args.json, table)
    return 1 if errors else 0


def name_at_line(tree: ast.AST, line: int) -> str:
    """その行にある代入の名前（無ければ空文字）。"""
    for node in ast.walk(tree):
        if isinstance(node, (ast.Assign, ast.AnnAssign)):
            span = range(node.lineno, (node.end_lineno or node.lineno) + 1)
            if line in span:
                names = assigned_names(node)
                if names:
                    return names[0]
    return ""


# --------------------------------------------------------------------------
# 入口
# --------------------------------------------------------------------------


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        description="参照コードを探して取り、数値に出典を付ける道具",
        epilog="命令の一覧と置き場は、このファイルの冒頭の docstring を見ること",
    )
    sub = parser.add_subparsers(dest="verb", required=True)

    links = sub.add_parser("links", help="Library から参照実装の在処を引く")
    links.add_argument("--paper", type=int, action="append", help="この論文の ID に絞る（繰り返し可）")
    links.add_argument("--json", action="store_true", help="JSON 1 つだけ出す")
    links.set_defaults(func=cmd_links)

    gh = sub.add_parser("gh-search", help="GitHub を検索する")
    gh.add_argument("query", help="検索語")
    gh.add_argument("--language", default="", help="言語で絞る（例: python）")
    gh.add_argument("--limit", type=int, default=10, help="取り出す数（既定 10・最大 50）")
    gh.add_argument("--json", action="store_true", help="JSON 1 つだけ出す")
    gh.set_defaults(func=cmd_gh_search)

    fetch = sub.add_parser("fetch", help="リポジトリを浅く取得して commit を控える")
    fetch.add_argument("url", help="GitHub の URL か modeldb:<id>")
    fetch.add_argument("--ref", default="", help="見る枝や版（無ければ URL の tree か既定の枝）")
    fetch.add_argument("--json", action="store_true", help="JSON 1 つだけ出す")
    fetch.set_defaults(func=cmd_fetch)

    params = sub.add_parser("params", help="コードから数値の定数を抜き出す")
    params.add_argument("path", help="ファイルかフォルダ")
    params.add_argument("--ext", default="", help="読む拡張子（例: .h,.cpp。既定は数種類）")
    params.add_argument("--json", action="store_true", help="JSON 1 つだけ出す")
    params.set_defaults(func=cmd_params)

    eqs = sub.add_parser(
        "eqs",
        help="コードから式の行を出典つきで抜き出す",
        description="参照コードから微分方程式と更新式の行を、src つきで抜き出す",
    )
    eqs.add_argument("path", help="ファイルかフォルダ")
    eqs.add_argument("--ext", default="", help="読む拡張子（例: .mod,.ode。既定は数種類）")
    eqs.add_argument("--json", action="store_true", help="JSON 1 つだけ出す")
    eqs.set_defaults(func=cmd_eqs)

    check = sub.add_parser("check", help="train.py の数値に出典の注記があるか確かめる")
    check.add_argument("path", help="train.py のパス")
    check.add_argument("--json", action="store_true", help="JSON 1 つだけ出す")
    check.set_defaults(func=command_check)
    return parser


def main(argv: list[str] | None = None) -> int:
    args = build_parser().parse_args(sys.argv[1:] if argv is None else argv)
    try:
        return args.func(args)
    except ArgError as e:
        print(f"引数の誤り: {e}", file=sys.stderr)
        return 2
    except RefcodeError as e:
        print(f"エラー: {e}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())