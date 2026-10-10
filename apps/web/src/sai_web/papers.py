"""sai-paper の呼び出し（Papers タブ・計画 07・複数プロジェクト対応は 17）。

第 1 弾の Web 機能（U3）で足したもの:
- Import（BibTeX の読み込み）: `import list / add / remove`
- Library に入れる（PDF 無しの登録・U13）: `paper keep`
- Project（state.md / mind.md / 要対応。研究の命令は sai-sci が行う・U14）:
  `project show` / `project mind` / `attention resolve` / `attention list`

sai-web は他アプリの DB を直接開かない（00 §2）ため，ここはすべて CLI に頼る。
呼ぶ先は「論文の置き場があるプロジェクト」（計画 17 §3.1）で決めるもので，
cwd と `SAI_PROJECT_ROOT` を **そのプロジェクトの root** にし，`SAI_ORIGIN=phone` を渡す
（sai-paper の `.env` の API キーを読ませるため・00 §8・17 §3.2・共有契約「Web が CLI を呼ぶ約束」）。
iPhone からはパスを受け取らず，プロジェクト名から root を引く（17 §3.2）。

使う CLI（00 §7.2）:
- `library list` / `library tree` / `paper show <id>`（読み取り・02）
- `paper translate <id>`（要旨の日本語訳・03）
- `content paper <id> --json`（本文・既存）
- `providers` / `search --json` / `search-history <id>`（Discover・02・03・計画 08）
"""

from __future__ import annotations

import asyncio
import datetime
import json
import logging
import os
import re
import time
from pathlib import Path

from fastapi import HTTPException
from pydantic import BaseModel
from starlette.concurrency import run_in_threadpool

from sai_web import cli, control, notify, registry
from sai_web.config import settings

logger = logging.getLogger("sai_web")


# --- 論文の置き場があるプロジェクト（計画 17 §3.1・§3.2） ---

# プロジェクト名の検査（画面の住所と音声の名前に使うため，合わない名前は対象外・17 §3.1）
_PROJECT_NAME_RE = re.compile(r"^[A-Za-z0-9_.-]{1,64}$")
# 論文の置き場のしるし（`sai-paper init` でできる。ファイルがあるかどうか見るだけで
# DB は開かない・00 §2・17 §3.1）
_PAPER_DB_REL = Path(".sai") / "paper" / "papr.db"
# 一覧を覚えておく秒数（プロジェクト一覧を呼びすぎないため・17 §3.2）
_PROJECTS_TTL = 30.0
# 覚えておいた一覧（利用者の名前 → (時刻, 一覧)）。中身は root の絶対パスも含む内部用
_projects_cache: dict[str, tuple[float, list[dict]]] = {}


def list_paper_projects(refresh: bool = False) -> list[dict]:
    """論文の置き場があるプロジェクト（内部用・root を含む・17 §3.1）。

    - プロジェクト一覧（control.list_projects）を 1 度だけ引き，
      30 秒だけ覚えておく（17 §3.2）。
    - `root_exists` が真で，かつ `<root>/.sai/paper/papr.db` がファイルのものだけ。
    - 名前が `^[A-Za-z0-9_.-]{1,64}$` に合わない物は対象外（ログに 1 行）。
    - 並びは登録簿のまま（最後に動いた順・06 §3）。
    """
    global _projects_cache
    now = time.monotonic()
    key = registry.user_key()
    hit = _projects_cache.get(key)
    if not refresh and hit is not None and now - hit[0] < _PROJECTS_TTL:
        return hit[1]
    out: list[dict] = []
    for project in control.list_projects():
        name = project.get("name") or ""
        root = project.get("root") or ""
        if not project.get("root_exists") or not root:
            continue
        if not _PROJECT_NAME_RE.match(name):
            # 音声の名前（17 §3.3）と画面の住所（§3.5）に使うため対象外にする
            logger.warning(f"論文の置き場の対象外（名前が規則に合わない）: {name!r}")
            continue
        if not (Path(root) / _PAPER_DB_REL).is_file():
            continue
        out.append(
            {
                "name": name,
                "root": root,
                # 音声の名前 `<フォルダ名>-<番号>` に使う root の末尾のフォルダ名（17 §3.3）
                "root_name": Path(root).name,
                "last_executed_at": project.get("last_executed_at"),
            }
        )
    _projects_cache[key] = (now, out)
    return out


def paper_projects_view() -> list[dict]:
    """`GET /api/paper-projects` に返す形（root の絶対パスは出さない・17 §3.4）。"""
    return [
        {
            "name": p["name"],
            "root_name": p["root_name"],
            "last_executed_at": p["last_executed_at"],
        }
        for p in list_paper_projects()
    ]


def resolve_project(name: str) -> dict:
    """プロジェクト名から root を引く（iPhone からはパスを受け取らない・17 §3.2）。

    1 回の API 呼び出しで 1 度だけ呼ぶ（呼びすぎるとプロジェクト一覧を
    何度も引くため・17 §3.2）。一覧に無ければ 404（名前の規則に合わない物も
    一覧には載らないので同じ扱い）。
    """
    for project in list_paper_projects():
        if project["name"] == name:
            return project
    raise HTTPException(
        status_code=404, detail="論文の置き場があるプロジェクトが見つかりません"
    )


def _paper_cli(project: dict, args: list[str], timeout: float = 30.0):
    """sai-paper の CLI を呼ぶ（cwd = root，`SAI_PROJECT_ROOT` = root，`SAI_ORIGIN` = phone）。

    環境変数は `SAI_PROJECT_ROOT` と `SAI_ORIGIN`（プロジェクトは `SAI_PROJECT_ROOT` と
    `.sai/` の探索で決める）。
    """
    root = str(project["root"])
    return cli.run_json(
        [settings.sai_paper_bin, *args],
        app="sai-paper",
        cwd=root,
        env={**os.environ, "SAI_PROJECT_ROOT": root, "SAI_ORIGIN": "phone"},
        timeout=timeout,
    )


def _sci_cli(project: dict, args: list[str], timeout: float = 30.0, input_text: str | None = None):
    """sai-sci の CLI を呼ぶ（研究の命令。cwd・env の約束は `_paper_cli` と同じ）。

    `input_text` を渡すと標準入力に書く（`project mind` の全文・U04）。
    """
    root = str(project["root"])
    return cli.run_json(
        [settings.sai_sci_bin, *args],
        app="sai-sci",
        cwd=root,
        env={**os.environ, "SAI_PROJECT_ROOT": root, "SAI_ORIGIN": "phone"},
        timeout=timeout,
        input_text=input_text,
    )


def library(project: dict) -> dict:
    """Library 一覧（`sai-paper library list`）。"""
    return _paper_cli(project, ["library", "list"])


def tree(project: dict) -> dict:
    """Library 階層（`sai-paper library tree`）。"""
    return _paper_cli(project, ["library", "tree"])


def detail(project: dict, paper_id: int) -> dict:
    """論文情報（`sai-paper paper show <id>`）。見つからなければ 400 で失敗。"""
    return _paper_cli(project, ["paper", "show", str(paper_id)])


def translate(project: dict, paper_id: int) -> dict:
    """要旨の日本語訳を作る（`paper translate <id>`）。数十秒かかりうるため待ちは長め。"""
    return _paper_cli(project, ["paper", "translate", str(paper_id)], timeout=90.0)


def translate_with_job(project: dict, paper_id: int) -> dict:
    """要旨の日本語訳を作り、動いているあいだだけ仕事の形にして画面に出せるようにする（U05 §1）。

    `phone translate` は同期の呼び出しなので、呼ぶあいだ `_download_jobs` に
    `kind="translate"` の仕事を入れ、終わったら `done` / `failed` にする
    （30 分で消えるのは Download と同じ）。% は分からないので `percent` は None
    （画面は動く帯で出す）。
    """
    name = project["name"]
    key = f"paper:{paper_id}"
    job = _new_job("translate", key, f"#{paper_id} の要旨の訳", "processing", [], None)
    job["paper_id"] = paper_id
    _download_jobs[(name, key)] = job
    try:
        out = translate(project, paper_id)
    except HTTPException as e:
        _finish_download(job, "failed", error=str(e.detail))
        raise
    except Exception:
        # 画面にも出したいので 30 分は残す（Download の失敗と同じ扱い）
        _finish_download(job, "failed", error="原因不明のエラー")
        raise
    _finish_download(job, "done")
    return out



def text(project: dict, paper_id: int) -> dict:
    """本文（既存の `content paper <id> --json`）。md_path は画面に返さない（07 §3.2）。"""
    data = _paper_cli(project, ["content", "paper", str(paper_id), "--json"])
    if not isinstance(data, dict):
        raise HTTPException(status_code=502, detail="sai-paper の応答を読めませんでした")
    return {"id": data.get("id", paper_id), "text": data.get("text") or ""}


def chunks(project: dict, paper_id: int) -> dict:
    """論文の全チャンク（`content chunks <id> --json`）。"""
    data = _paper_cli(project, ["content", "chunks", str(paper_id), "--json"], timeout=60.0)
    if not isinstance(data, dict) or not isinstance(data.get("chunks"), list):
        raise HTTPException(status_code=502, detail="sai-paper の応答を読めませんでした")
    return {
        "paper_id": data.get("paper_id", paper_id),
        "title": data.get("title") or "",
        "chunks": data["chunks"],
    }


def dashboard(project: dict) -> dict:
    """Dashboard（`dashboard`・数字カードと Radio の出来事を混ぜた出来事の一覧）。"""
    return _paper_cli(project, ["dashboard"])


# ノートの大きさの上限（100KB・task.md と同じ）
NOTE_MAX = 100 * 1024


def save_note(project: dict, paper_id: int, body) -> dict:
    """ノート（note.md）を保存する（`paper note <id>`・本文は標準入力）。"""
    if not isinstance(body, str) or len(body.encode("utf-8")) > NOTE_MAX:
        raise HTTPException(status_code=400, detail="ノートは 100KB までの文字にしてください")
    root = str(project["root"])
    return cli.run_json(
        [settings.sai_paper_bin, "paper", "note", str(paper_id)],
        app="sai-paper",
        cwd=root,
        env={**os.environ, "SAI_PROJECT_ROOT": root, "SAI_ORIGIN": "phone"},
        input_text=body,
    )


def figure_path(project: dict, paper_id: int, fig_no: int) -> Path:
    """図の画像の置き場（sai-paper が返した `figures.items[].image` だけを使う）。

    PDF と同じく，そのプロジェクトの root の外を指していたり消えていたりしたら 404。
    """
    info = detail(project, paper_id) or {}
    figs = info.get("figures") if isinstance(info.get("figures"), dict) else {}
    items = figs.get("items") if isinstance(figs.get("items"), list) else []
    hit = next(
        (f for f in items if isinstance(f, dict) and f.get("fig_no") == fig_no), None
    )
    if not hit or not hit.get("image"):
        raise HTTPException(status_code=404, detail="図がありません")
    path = Path(str(hit["image"]))
    if not path.resolve().is_relative_to(Path(project["root"]).resolve()) or not path.is_file():
        raise HTTPException(status_code=404, detail="図がありません")
    return path


# --- Discover（計画 08 §3） ---

# 検索語の長さ（前後の空白を取って 1〜200 文字・08 §3）
SEARCH_QUERY_MAX = 200
# 年の範囲（1900〜2100・08 §3）
SEARCH_YEAR_MIN = 1900
SEARCH_YEAR_MAX = 2100
# 件数の範囲（1〜100・08 §3）
SEARCH_LIMIT_MIN = 1
SEARCH_LIMIT_MAX = 100


class SearchRequest(BaseModel):
    """検索の依頼（`POST …/papers/search` の body・08 §3）。

    外から来る値なので，中身の検査は `validate_search` で必ず行う。
    """

    query: str
    dbs: list[str] | None = None  # 省略したら sai-paper の既定
    year_from: int | None = None
    year_to: int | None = None
    limit: int = 20


def providers(project: dict) -> dict:
    """検索先の一覧と既定（`providers`・08 §3）。"""
    return _paper_cli(project, ["providers"])


def search_history(project: dict, log_id: int) -> dict:
    """過去の検索結果（`search-history <log_id>`・08 §3）。"""
    return _paper_cli(project, ["search-history", str(log_id)])


def validate_search(req: SearchRequest, available: list[str]) -> str:
    """検索の依頼を検査し，前後の空白を取った検索語を返す（違反は 400・08 §3）。"""
    query = str(req.query or "").strip()
    if not (1 <= len(query) <= SEARCH_QUERY_MAX):
        raise HTTPException(
            status_code=400,
            detail=f"検索語は前後の空白を取って1〜{SEARCH_QUERY_MAX}文字で入れてください",
        )
    for db in req.dbs or []:
        if db not in available:
            raise HTTPException(status_code=400, detail=f"検索先が分かりません: {db}")
    for label, value in (("年の始め", req.year_from), ("年の終わり", req.year_to)):
        if value is not None and not (SEARCH_YEAR_MIN <= value <= SEARCH_YEAR_MAX):
            raise HTTPException(
                status_code=400,
                detail=f"{label}は{SEARCH_YEAR_MIN}〜{SEARCH_YEAR_MAX}で入れてください",
            )
    if (
        req.year_from is not None
        and req.year_to is not None
        and req.year_from > req.year_to
    ):
        raise HTTPException(status_code=400, detail="年の範囲が逆です（から ≦ まで）")
    if not (SEARCH_LIMIT_MIN <= req.limit <= SEARCH_LIMIT_MAX):
        raise HTTPException(
            status_code=400,
            detail=f"件数は{SEARCH_LIMIT_MIN}〜{SEARCH_LIMIT_MAX}で入れてください",
        )
    return query


def build_search_args(req: SearchRequest, available: list[str]) -> list[str]:
    """`search --json` の引数を作る（検査もここで行う・08 §3）。

    検索語は最後に `--` を置いて渡す（語が `-` で始めると clap が
    オプションと間違えるため・08 §2）。`--json` が無いと人向けの文が出るので必ず付ける。
    """
    query = validate_search(req, available)
    args = ["search", "--json", "--limit", str(req.limit)]
    for db in req.dbs or []:
        args += ["--db", db]
    if req.year_from is not None:
        args += ["--year-from", str(req.year_from)]
    if req.year_to is not None:
        args += ["--year-to", str(req.year_to)]
    args += ["--", query]
    return args


def search(project: dict, req: SearchRequest) -> dict:
    """検索（`phone search`・検索履歴にも残る・08 §3）。

    外部サービスへ問い合わせるため数秒〜数十秒かかる（待ちは 120 秒）。
    """
    data = providers(project)
    available = data.get("available") if isinstance(data, dict) else None
    if not isinstance(available, list):
        raise HTTPException(status_code=502, detail="sai-paper の検索先を読めませんでした")
    args = build_search_args(req, [str(x) for x in available])
    return _paper_cli(project, args, timeout=120.0)


# --- 内部検索（Library 内の検索・U02） ---
# 外部検索（`phone search`）と同じ作り。検査はここで行い（大原則）、
# CLI 側の検査だけに頼らない。既定値は sai-paper 側（`rag.*` の設定）に任せ、
# 省略した項目は引数に出さない。

# 内部検索の取り方（省略で既定）
FIND_MODES = ("word", "vector", "hybrid")
# 内部検索の範囲（省略で既定）
FIND_SCOPES = ("papers", "notes", "figures", "all")
# 内部検索の件数（1〜100）
FIND_TOP_K_MIN = 1
FIND_TOP_K_MAX = 100
# 内部検索の論文番号（1〜999999999・20 個まで）
FIND_PAPER_MIN = 1
FIND_PAPER_MAX = 999999999
FIND_PAPERS_MAX = 20
# 内部検索の論文ごとの件数（1〜20）
FIND_EACH_MIN = 1
FIND_EACH_MAX = 20
# research-tree の Node の形（sci.py と同じ値。sci を import せず持つのは
# 循環 import を避けるため・U02 計画書の前提）
_FIND_NODE_RE = re.compile(r"^[QAFESD][1-9][0-9]{0,8}$")


class FindRequest(BaseModel):
    """内部検索の依頼（`POST …/papers/find` の body・U02）。

    外から来る値なので，中身の検査は `validate_find` で必ず行う。
    """

    query: str
    mode: str | None = None  # word / vector / hybrid（省略で既定）
    top_k: int | None = None  # 1〜100（省略で既定）
    scope: str | None = None  # papers / notes / figures / all（省略で既定）
    papers: list[int] | None = None  # Library の論文番号（20 個まで）
    node: str | None = None  # research-tree の Node（Q12・A5・F3・E2）
    subtree: bool = False  # node の子孫も含める
    each: int | None = None  # 範囲の論文ごとに上位 m 件（1〜20）


def validate_find(req: FindRequest) -> str:
    """内部検索の依頼を検査し，前後の空白を取った検索語を返す（違反は 400・U02）。"""
    query = str(req.query or "").strip()
    if not (1 <= len(query) <= SEARCH_QUERY_MAX):
        raise HTTPException(
            status_code=400,
            detail=f"検索語は前後の空白を取って1〜{SEARCH_QUERY_MAX}文字で入れてください",
        )
    if req.mode is not None and req.mode not in FIND_MODES:
        raise HTTPException(
            status_code=400,
            detail="検索の取り方は word・vector・hybrid のどれかにしてください",
        )
    if req.scope is not None and req.scope not in FIND_SCOPES:
        raise HTTPException(
            status_code=400,
            detail="検索の範囲は papers・notes・figures・all のどれかにしてください",
        )
    if req.top_k is not None and (
        isinstance(req.top_k, bool) or not (FIND_TOP_K_MIN <= req.top_k <= FIND_TOP_K_MAX)
    ):
        raise HTTPException(
            status_code=400,
            detail=f"件数は{FIND_TOP_K_MIN}〜{FIND_TOP_K_MAX}で入れてください",
        )
    if req.papers is not None:
        if len(req.papers) > FIND_PAPERS_MAX:
            raise HTTPException(
                status_code=400,
                detail=f"論文は{FIND_PAPERS_MAX}個までにしてください",
            )
        for paper_id in req.papers:
            if (
                isinstance(paper_id, bool)
                or not isinstance(paper_id, int)
                or not (FIND_PAPER_MIN <= paper_id <= FIND_PAPER_MAX)
            ):
                raise HTTPException(
                    status_code=400,
                    detail=f"論文の番号は{FIND_PAPER_MIN}〜{FIND_PAPER_MAX}の整数にしてください",
                )
    node = None
    if req.node is not None:
        if not isinstance(req.node, str) or not _FIND_NODE_RE.match(req.node.strip()):
            raise HTTPException(status_code=400, detail="Node の ID の形が正しくありません")
        node = req.node.strip()
    if req.subtree and node is None:
        raise HTTPException(
            status_code=400,
            detail="子孫も含める（subtree）ときは Node を指定してください",
        )
    if req.each is not None:
        if isinstance(req.each, bool) or not (FIND_EACH_MIN <= req.each <= FIND_EACH_MAX):
            raise HTTPException(
                status_code=400,
                detail=f"論文ごとの件数は{FIND_EACH_MIN}〜{FIND_EACH_MAX}で入れてください",
            )
        if node is None and not (req.papers or []):
            raise HTTPException(
                status_code=400,
                detail="論文ごとの件数（each）を指定するときは Node か論文を指定してください",
            )
    return query


def build_find_args(req: FindRequest) -> list[str]:
    """`phone find` の引数を作る（検査もここで行う・U02）。

    検索語は最後に `--` を置いて渡す（語が `-` で始めると clap が
    オプションと間違えるため・`build_search_args` と同じ）。
    """
    query = validate_find(req)
    args = ["find", "--json"]
    if req.mode is not None:
        args += ["--mode", req.mode]
    if req.top_k is not None:
        args += ["--top-k", str(req.top_k)]
    if req.scope is not None:
        args += ["--scope", req.scope]
    for paper_id in req.papers or []:
        args += ["--paper", str(paper_id)]
    if req.node is not None:
        args += ["--node", req.node.strip()]
    if req.subtree:
        args += ["--subtree"]
    if req.each is not None:
        args += ["--each", str(req.each)]
    args += ["--", query]
    return args


def find(project: dict, req: FindRequest) -> dict:
    """内部検索（`phone find`・検索履歴にも残る・U02）。

    意味検索は埋め込みの計算で数十秒かかりうるため待ちは 120 秒。
    `node` を付けた検索は範囲の決定と記録を sai-sci が行う（U14）。
    """
    if req.node is not None:
        return _sci_cli(project, build_find_args(req), timeout=120.0)
    return _paper_cli(project, build_find_args(req), timeout=120.0)


def find_history(project: dict, log_id: int) -> dict:
    """過去の内部検索の結果（`find-history <log_id>`・U02）。"""
    return _paper_cli(project, ["find-history", str(log_id)])


# --- 音声化の依頼（計画 10 §3・プロジェクト付きは 17 §3.3・§3.4） ---

# 今動いている音声化（(プロジェクト名, 論文 id) → タスク）。サーバーを再起動すると完了通知は失われる
# （状態そのものは sai-paper の paper_podcasts に残るため，画面はそちらを見る・10 §3）。
# 別プロジェクトに同じ番号の論文があっても「作成中」と誤判定しないよう名前と番号の組で持つ（17 §3.4）。
_radio_jobs: dict[tuple[str, int], asyncio.Task] = {}
# 走っている音声化の附带情報（(プロジェクト名, 論文 id) → 題名と始めた時刻）。
# 画面の一覧（`list_downloads`・U05 §1）に出すためのものだから、ここでは使わない。
_radio_meta: dict[tuple[str, int], dict] = {}
# 音声化サブプロセスの待ち時間（sai-paper 側の 1800 秒より少し長く・10 §3）
RADIO_JOB_TIMEOUT = 2000.0


def radio_id_for(project: dict, paper_id: int) -> str:
    """音声の名前 `<root のフォルダ名>-<論文番号>`（例: `sai-paper-30`・17 §3.3）。

    決め方は sai-paper 側と同じにする（sai-paper はフォルダ名だけを知ればよい・17 §3.3）。
    """
    return f'{project["root_name"]}-{paper_id}'


def _radio_error(stdout: bytes | None) -> str:
    """失敗した CLI の標準出力から `error` を取り出す（読めなければ「原因不明のエラー」・10 §3）。"""
    try:
        data = json.loads((stdout or b"").decode("utf-8", "replace") or "")
        if isinstance(data, dict) and data.get("error"):
            return str(data["error"])
    except (json.JSONDecodeError, TypeError, UnicodeDecodeError):
        pass
    return "原因不明のエラー"


def _radio_id_from_output(stdout: bytes | None) -> str | None:
    """`phone radio` の応答の `radio_id` を読む（無ければ None・17 §3.3）。

    sai-web 側で組み立てた名前と食い違うことがあるため，返ってきたものが正。
    """
    try:
        data = json.loads((stdout or b"").decode("utf-8", "replace") or "")
    except (json.JSONDecodeError, TypeError, UnicodeDecodeError):
        return None
    value = data.get("radio_id") if isinstance(data, dict) else None
    return str(value) if value else None


async def request_radio(project: dict, paper_id: int) -> dict:
    """音声化の依頼（`POST …/papers/{id}/radio`・10 §3）。

    ブラウザにはすぐ返し（202），実際の音声化は `_radio_job` を背景で走らせる
    （`sai-paper phone radio` は数分かかるため・10 §3）。
    返り値に `radio_id` を足す（再生画面への導線・17 §3.4）。
    """
    key = (project["name"], paper_id)
    info = await run_in_threadpool(detail, project, paper_id)  # 論文が無ければ 400 で失敗
    podcast = info.get("podcast") if isinstance(info.get("podcast"), dict) else {}
    job = _radio_jobs.get(key)
    if podcast.get("status") == "generating" or (job is not None and not job.done()):
        raise HTTPException(status_code=409, detail="この論文の音声は作成中です")
    files = info.get("files") if isinstance(info.get("files"), dict) else {}
    if not (files or {}).get("extract"):
        raise HTTPException(
            status_code=400,
            detail="本文がまだ取り出されていないため作れません（Mac の sai-paper で取り出してください）",
        )
    paper = info.get("paper") if isinstance(info.get("paper"), dict) else {}
    title = str((paper or {}).get("title") or "(無題)")
    # 画面の一覧に出すため、題名と始めた時刻を覚えておく（U05 §1）
    _radio_meta[key] = {"title": title, "started_at": _now_iso()}
    _radio_jobs[key] = asyncio.create_task(_radio_job(project, paper_id, title))
    return {"status": "generating", "radio_id": radio_id_for(project, paper_id)}


async def _radio_job(project: dict, paper_id: int, title: str) -> None:
    """背景で `sai-paper phone radio <id>` を走らせ，終わったら通知する（10 §3・17 §3.4）。

    - 通知の飛び先: 成功は `/#/radio/<radio_id>`，失敗は `/#/paper/<name>/<id>`（17 §3.4）。
    - 例外は全部つかまえてログに出す（サーバーは止めない）。
    """
    name = project["name"]
    root = str(project["root"])
    # 失敗の飛び先は「そのプロジェクトのその論文」の論文情報（17 §3.4）
    fail_url = f"/#/paper/{name}/{paper_id}"
    try:
        try:
            proc = await asyncio.create_subprocess_exec(
                settings.sai_paper_bin,
                "paper",
                "radio",
                str(paper_id),
                cwd=root,
                env={**os.environ, "SAI_PROJECT_ROOT": root, "SAI_ORIGIN": "phone"},
                stdout=asyncio.subprocess.PIPE,
                stderr=asyncio.subprocess.PIPE,
            )
        except (FileNotFoundError, OSError) as e:
            # PATH に無い・設定の場所が間違っている（10 §3）
            logger.warning(f"sai-paper を起動できません: {e}")
            notify.broadcast(
                "音声を作れませんでした", f"{title}: sai-paper が見つかりません", fail_url
            )
            return
        try:
            stdout, stderr = await asyncio.wait_for(proc.communicate(), timeout=RADIO_JOB_TIMEOUT)
        except asyncio.TimeoutError:
            # 時間切れなら kill（sai-paper 側の 1800 秒より少し長く待った・10 §3）
            proc.kill()
            try:
                stdout, _ = await proc.communicate()
            except Exception:
                stdout = b""
            notify.broadcast(
                "音声を作れませんでした", f"{title}: 時間内に終わりませんでした", fail_url
            )
            return
        if stderr:
            logger.warning(f"sai-paper phone radio の標準エラー: {stderr.decode('utf-8', 'replace')[:300]}")
        if proc.returncode == 0:
            # radio_id は sai-paper が返したものが正。無ければフォルダ名から組み立てる
            # （sai-paper がまだ radio_id を返さない場合の保険・17 §3.3）
            radio_id = _radio_id_from_output(stdout) or radio_id_for(project, paper_id)
            notify.broadcast("🎧 音声ができました", title, f"/#/radio/{radio_id}")
        else:
            # 理由は標準エラーの最後の行 → 標準出力の {"error"} → 「原因不明のエラー」（U09）
            reason = _last_stderr_reason(stderr) or _radio_error(stdout)
            notify.broadcast("音声を作れませんでした", f"{title}: {reason}", fail_url)
    except Exception as e:  # サーバーは止めない（10 §3）
        logger.exception(f"音声化の背景実行で失敗: {e}")
    finally:
        _radio_jobs.pop((name, paper_id), None)
        _radio_meta.pop((name, paper_id), None)


def pdf_path(project: dict, paper_id: int, lang: str | None) -> Path:
    """PDF の置き場（`lang == "ja"` なら日本語訳 PDF）。

    利用者から来たパスは使わず，sai-paper が返した `files` の値だけを使う（07 §3.2）。
    返ったパスが **そのプロジェクトの root の中** にあることを確かめてから返す
    （プロジェクトが増えたので念のため・17 §3.4）。無ければ・外なら 404。
    """
    files = (detail(project, paper_id) or {}).get("files") or {}
    if not isinstance(files, dict):
        files = {}
    value = files.get("pdf_ja") if lang == "ja" else files.get("pdf")
    if not value:
        raise HTTPException(status_code=404, detail="PDF がありません")
    path = Path(str(value))
    # root の外を指していないか確かめる（17 §3.4）
    if not path.resolve().is_relative_to(Path(project["root"]).resolve()):
        raise HTTPException(status_code=404, detail="PDF がありません")
    if not path.is_file():
        # sai-paper が返したパスなのに消えていた場合も「無し」と同じ扱いにする
        raise HTTPException(status_code=404, detail="PDF がありません")
    return path


# --- Import（BibTeX の読み込み）と Library に入れる・Project（U3 §変更内容 2・U13 §2） ---
# 外から来る値（.bib の名前・本文・keep の依頼・メモ・key）は
# すべて CLI を呼ぶ前にここで確かめる（共有契約 C-A2）。

# .bib の名前の規則（英数字・.・-・_ で .bib で終わる 80 文字まで）
_BIB_NAME_RE = re.compile(r"^[A-Za-z0-9._-]{1,80}\.bib$")
# .bib の中身の大きさの上限（1MB）
BIB_TEXT_MAX = 1024 * 1024
# mind.md の全文の長さの上限（文字・U04）
MIND_MAX = 200000
# 要対応の key の長さの上限
ATTENTION_KEY_MAX = 300
# 題名（通知の文に使うだけ）の長さの上限
DOWNLOAD_TITLE_MAX = 200
# 終わった仕事を一覧に残す秒数（30 分）
_DOWNLOAD_KEEP = 1800.0
# 前処理の束の取り込みの段（0 = 展開, 1 = 取り込み, 2 = 完了）。
# 段の名前は画面（paperimport.js）と共有する。
BUNDLE_STEPS = ["展開", "取り込み"]
# `sai-progress` の段を持たない仕事の既定の段（引用の抜き出しは自前の段を使う）。
DOWNLOAD_STEPS = ["PDF取得", "図抽出", "翻訳"]
# 仕事のログに残す行数の上限（古いものから捨てる）
DOWNLOAD_MESSAGE_KEEP = 50

# 今走っている仕事（(プロジェクト名, 仕事のキー) → 仕事の dict）。
# 束の取り込み（bundle）・引用の抜き出し・要旨の訳・PDF の訳・音声化を入れる。
# 状態は sai-web のメモリだけに持つ（音声化と同じ作り・再起動しても論文は sai-paper に残る）。
_download_jobs: dict[tuple[str, str], dict] = {}


def _now_iso() -> str:
    """画面に出す時刻（ローカル時・秒まで）。"""
    return datetime.datetime.now().astimezone().isoformat(timespec="seconds")


def _validate_bib_name(name) -> str:
    """.bib の名前を確かめる（違反は 400・U3 §変更内容 2）。"""
    if not isinstance(name, str) or not _BIB_NAME_RE.match(name):
        raise HTTPException(
            status_code=400,
            detail="ファイル名は英数字・.・-・_ で .bib で終わる 80 文字までにしてください",
        )
    return name


def _default_bib_name() -> str:
    """名前が空のときの .bib の名前（`phone-<YYYYMMDD-HHMMSS>.bib`・U3 §変更内容 2）。"""
    return f"phone-{datetime.datetime.now().strftime('%Y%m%d-%H%M%S')}.bib"


def _validate_bib_text(text) -> str:
    """BibTeX の中身を確かめる（文字列・空白以外あり・1MB まで）。"""
    if not isinstance(text, str) or not text.strip():
        raise HTTPException(status_code=400, detail="BibTeX の中身を入れてください")
    if len(text.encode("utf-8")) > BIB_TEXT_MAX:
        raise HTTPException(status_code=400, detail="BibTeX は 1MB までにしてください")
    return text


def import_list(project: dict) -> dict:
    """.bib の一覧と中の論文（`import list`・U3 §変更内容 2）。"""
    return _paper_cli(project, ["import", "list"])


def import_add(project: dict, name, text) -> dict:
    """.bib の保存（`import add --name <name>`・本文は標準入力）。

    名前に空文字が来たときは `phone-<YYYYMMDD-HHMMSS>.bib` を作る。
    `save_note` と同じく `cli.run_json` を直接呼ぶ（cwd と env も同じ）。
    """
    if name is None:
        name = ""
    if not isinstance(name, str):
        raise HTTPException(status_code=400, detail="ファイル名は文字列で送ってください")
    file_name = _default_bib_name() if not name.strip() else _validate_bib_name(name)
    body = _validate_bib_text(text)
    root = str(project["root"])
    return cli.run_json(
        [settings.sai_paper_bin, "import", "add", "--name", file_name],
        app="sai-paper",
        cwd=root,
        env={**os.environ, "SAI_PROJECT_ROOT": root, "SAI_ORIGIN": "phone"},
        input_text=body,
    )


def import_remove(project: dict, file_name) -> dict:
    """.bib の削除（`import remove --name <name>`）。"""
    name = _validate_bib_name(file_name)
    return _paper_cli(project, ["import", "remove", "--name", name])


# --- Library に入れる（PDF 無しの登録・U13 §2） ---


def _as_positive_int(value, label: str, allow_text: bool = False) -> int:
    """外から来る「1 以上の整数」を int にして返す（違反は 400）。

    JSON の body では数値だけ、クエリの入力では数字の文字列が届くため、
    クエリから引く関数は `allow_text` で数字の文字列も受ける。
    """
    if isinstance(value, bool):
        raise HTTPException(
            status_code=400, detail=f"{label} は 1 以上の整数にしてください"
        )
    if isinstance(value, int):
        number = value
    elif allow_text and isinstance(value, str) and re.fullmatch(r"\d{1,9}", value.strip()):
        number = int(value.strip())
    else:
        raise HTTPException(
            status_code=400, detail=f"{label} は 1 以上の整数にしてください"
        )
    if number <= 0:
        raise HTTPException(
            status_code=400, detail=f"{label} は 1 以上の整数にしてください"
        )
    return number


def _job_title(payload) -> str:
    """通知の文に使う題名（無ければ「論文」・長すぎる値は切る）。"""
    title = payload.get("title")
    if title is None:
        return "論文"
    if not isinstance(title, str):
        raise HTTPException(status_code=400, detail="title は文字列で送ってください")
    return title[:DOWNLOAD_TITLE_MAX].strip() or "論文"


def _cite_target(payload, allow_text: bool = False) -> tuple[str, list[str]] | None:
    """引用の指定（`{"cite_paper_id": 12, "ref_no": 3}`）があれば `(キー, 引数)` を返す。

    どちらも無ければ None、片方だけ・0 以下・文字列以外なら 400。
    `keep`（U13 §2）と引用の指定（U05 §2）のどちらでも使う。
    """
    paper_id = payload.get("cite_paper_id")
    ref_no = payload.get("ref_no")
    if paper_id is None and ref_no is None:
        return None
    pid = _as_positive_int(paper_id, "cite_paper_id", allow_text)
    ref = _as_positive_int(ref_no, "ref_no", allow_text)
    return f"cite:{pid}:{ref}", ["--cite", str(pid), "--ref", str(ref)]


def _download_payload(payload) -> tuple[str, list[str], str]:
    """Library に入れる対象を確かめて `(対象のキー, CLI の引数, 題名)` を返す（違反は 400）。

    `sai-paper paper keep`（U13 §2）と、かつての Download が共有する検査。
    - 検索履歴の 1 件: `{"log_id": 3, "position": 2}` → キー `log:3:2`
    - BibTeX の 1 件: `{"import_id": "DOI:…"}` → キー `import:DOI:…`
    - 引用の 1 件: `{"cite_paper_id": 12, "ref_no": 3}` → キー `cite:12:3`（U05 §2）
    - どれかちょうど 1 つ。どれも無し・2 つ以上なら 400。
    - `title` は通知の文に使うだけなので任意。
    """
    if not isinstance(payload, dict):
        raise HTTPException(status_code=400, detail="送る値が正しくありません")
    log_id = payload.get("log_id")
    position = payload.get("position")
    import_id = payload.get("import_id")
    has_log = log_id is not None or position is not None
    has_import = import_id is not None
    has_cite = payload.get("cite_paper_id") is not None or payload.get("ref_no") is not None
    if sum((has_log, has_import, has_cite)) != 1:
        raise HTTPException(
            status_code=400,
            detail="log_id・import_id・引用（cite_paper_id と ref_no）のどれか 1 つを指定してください",
        )
    if has_log:
        log_id = _as_positive_int(log_id, "log_id")
        position = _as_positive_int(position, "position")
        key = f"log:{log_id}:{position}"
        args = ["--log", str(log_id), "--position", str(position)]
    elif has_import:
        if (
            not isinstance(import_id, str)
            or not (1 <= len(import_id) <= ATTENTION_KEY_MAX)
            or any(c in import_id for c in "\r\n")
        ):
            raise HTTPException(
                status_code=400,
                detail=f"import_id は 1〜{ATTENTION_KEY_MAX} 文字の 1 行にしてください",
            )
        key = f"import:{import_id}"
        args = ["--import-id", import_id]
    else:
        cite = _cite_target(payload)
        assert cite is not None  # has_cite が真のときだけここに来る
        key, args = cite
    return key, args, _job_title(payload)


def _download_json(stdout: bytes | None) -> dict:
    """CLI の JSON の応答を読む（読めなければ空の dict）。"""
    try:
        data = json.loads((stdout or b"").decode("utf-8", "replace") or "")
    except (json.JSONDecodeError, TypeError, UnicodeDecodeError):
        return {}
    return data if isinstance(data, dict) else {}


def _finish_download(job: dict, status: str, error: str | None = None) -> None:
    """仕事を終わらせる（状態・理由・終わった時刻を入れる）。"""
    job["status"] = status
    job["error"] = error
    job["finished_at"] = _now_iso()
    # 終わったので 100%。失敗のときはそのときのまま残す（どこまで来たかが分かるように）。
    # 段を持たない仕事（要旨の訳など・U05 §1）は % を出さないので触らない。
    if status == "done" and job.get("steps"):
        job["step"] = len(job["steps"])
        job["percent"] = 100
    job["_done_at"] = time.monotonic()


def _new_job(kind: str, key: str, title: str, status: str, steps: list[str], percent) -> dict:
    """仕事の dict を作る（一覧に出す形はすべて同じにする・U05 §1）。

    `kind` は `download` / `manual` / `cite-extract` / `radio` / `translate` / `index` / `bundle` / `migrate`。
    `percent` が None の仕事は「% の分からない仕事」（画面は動く帯で出す）。
    """
    return {
        "key": key,
        "kind": kind,
        "title": title,
        "status": status,
        "paper_id": None,
        "error": None,
        "started_at": _now_iso(),
        "finished_at": None,
        "steps": list(steps),
        "step": 0,
        "percent": percent,
        "messages": [],
    }


def apply_progress_line(job: dict, line: str) -> bool:
    """`sai-paper` の標準エラーの 1 行を仕事の進み具合へ反映する（`sai-progress …` だけ True）。

    受け付ける 2 つの形:
    - `sai-progress step=<n>`: 段を進める（% は段から作る。0/33/66/100）
    - `sai-progress msg=<文>`: ログを 1 行足す（最大 50 行）

    それ以外の行は進捗ではなく `False`（呼び出し側は通常のログとして出す）。
    """
    steps = job.get("steps") or DOWNLOAD_STEPS
    text = str(line or "").strip()
    if text.startswith("sai-progress step="):
        raw = text[len("sai-progress step=") :].strip()
        try:
            step = int(raw)
        except ValueError:
            return False
        # 段の番号は 0..len(steps) に収める（越えた数字は端の段にする）。
        step = max(0, min(step, len(steps)))
        job["step"] = step
        job["percent"] = step * 100 // len(steps)
        return True
    if text.startswith("sai-progress msg="):
        message = text[len("sai-progress msg=") :]
        messages = list(job.get("messages") or [])
        messages.append(message)
        job["messages"] = messages[-DOWNLOAD_MESSAGE_KEEP:]
        return True
    return False


def keep(project: dict, payload) -> dict:
    """PDF 無しで Library に入れる（`POST …/papers/keep`・U13 §2）。

    body は検索履歴・BibTeX・引用のどれか 1 つ（`_download_payload` と同じ検査）。
    `sai-paper paper keep … --json` はすぐ終わるので、202 にせず 200 で結果
    （`{"paper_id", "title", "status": "added"|"exists"}`）をそのまま返す。
    """
    _key, args, _title = _download_payload(payload)
    return _paper_cli(project, ["paper", "keep", *args, "--json"])


async def _write_stdin(proc, payload: bytes) -> None:
    """標準入力へ本文を書き込んで閉じる（`--stdin` の取り込み用・U05 §3）。

    パイプが壊れていてもその場では止めない。取り込みの失敗は終了コードと
    標準出力の `error` で分かるため（呼び出し側が見る）。
    """
    if proc.stdin is None:
        return
    try:
        proc.stdin.write(payload)
        await proc.stdin.drain()
        proc.stdin.close()
        await proc.stdin.wait_closed()
    except (BrokenPipeError, ConnectionResetError, OSError) as e:
        logger.warning(f"標準入力へ書けません: {e}")


def _last_stderr_reason(stderr: bytes | None) -> str:
    """標準エラーの最後の空でない行（`sai-progress` で始まる行を除く）。

    背景の仕事の失敗の理由（無ければ空文字・共有契約「Web が CLI を呼ぶ約束」）。
    """
    if not stderr:
        return ""
    for line in reversed(stderr.decode("utf-8", "replace").splitlines()):
        text = line.strip()
        if text and not text.startswith("sai-progress"):
            return text
    return ""


async def _run_paper_command(
    root: str, args: list[str], timeout: float, input_bytes: bytes | None = None
) -> tuple[int, bytes, str]:
    """sai-paper の新しい命令を背景で走らせて `(終了コード, stdout, 失敗理由)` を返す。

    `reason` は起動できない・時間切れのときの文，または終了コードが 0 でないときの
    標準エラーの最後の空でない行（`sai-progress` を除く）。成功なら空文字。
    呼び出し側は reason → `_radio_error(stdout)` → 「原因不明のエラー」の順に使う。
    `input_bytes` を渡すと標準入力に本文を書く（`paper import-pdf --stdin`・U05 §3）。
    """
    try:
        proc = await asyncio.create_subprocess_exec(
            settings.sai_paper_bin,
            *args,
            cwd=root,
            env={**os.environ, "SAI_PROJECT_ROOT": root, "SAI_ORIGIN": "phone"},
            stdin=asyncio.subprocess.PIPE if input_bytes is not None else None,
            stdout=asyncio.subprocess.PIPE,
            stderr=asyncio.subprocess.PIPE,
        )
    except (FileNotFoundError, OSError) as e:
        logger.warning(f"sai-paper を起動できません: {e}")
        return -1, b"", "sai-paper が見つかりません"
    if input_bytes is not None:
        await _write_stdin(proc, input_bytes)
    try:
        stdout, stderr = await asyncio.wait_for(proc.communicate(), timeout=timeout)
    except asyncio.TimeoutError:
        proc.kill()
        try:
            await proc.communicate()
        except Exception:
            pass
        return -2, b"", "時間内に終わりませんでした"
    if stderr:
        logger.warning(
            f"sai-paper {' '.join(args)} の標準エラー: "
            f"{stderr.decode('utf-8', 'replace')[:300]}"
        )
    code = proc.returncode or 0
    return code, stdout, ("" if code == 0 else _last_stderr_reason(stderr))


async def _run_paper_command_lines(
    root: str,
    args: list[str],
    timeout: float,
    on_line,
    input_bytes: bytes | None = None,
) -> tuple[int, bytes, str]:
    """`_run_paper_command` と同じ作りだが、標準エラーを 1 行ずつ `on_line` に渡す。

    `paper citations-extract` は本文の抜き出しで数分かかることがあるため、
    待つあいだの標準エラー（`sai-progress step=…` / `sai-progress msg=…`）を
    途中で受け取る。標準出力は終わってから読む。
    返り値と時間切れの扱いは `_run_paper_command` と同じ（失敗の理由は
    最後の `sai-progress` 以外の行）。
    """
    try:
        proc = await asyncio.create_subprocess_exec(
            settings.sai_paper_bin,
            *args,
            cwd=root,
            env={**os.environ, "SAI_PROJECT_ROOT": root, "SAI_ORIGIN": "phone"},
            stdin=asyncio.subprocess.PIPE if input_bytes is not None else None,
            stdout=asyncio.subprocess.PIPE,
            stderr=asyncio.subprocess.PIPE,
        )
    except (FileNotFoundError, OSError) as e:
        logger.warning(f"sai-paper を起動できません: {e}")
        return -1, b"", "sai-paper が見つかりません"
    if input_bytes is not None:
        await _write_stdin(proc, input_bytes)

    last_line = ""

    async def run() -> tuple[int, bytes]:
        nonlocal last_line
        while True:
            raw = await proc.stderr.readline()
            if not raw:
                break
            line = raw.decode("utf-8", "replace").rstrip("\r\n")
            on_line(line)
            text = line.strip()
            if text and not text.startswith("sai-progress"):
                last_line = text
        stdout = await proc.stdout.read()
        await proc.wait()
        return proc.returncode or 0, stdout

    try:
        code, stdout = await asyncio.wait_for(run(), timeout=timeout)
    except asyncio.TimeoutError:
        proc.kill()
        try:
            await asyncio.wait_for(proc.wait(), timeout=5.0)
        except Exception:
            pass
        return -2, b"", "時間内に終わりませんでした"
    return code, stdout, ("" if code == 0 else last_line)


def _radio_jobs_for(project: dict) -> list[dict]:
    """そのプロジェクトで走っている音声化を仕事の形にして返す（U05 §1）。

    画面は「進み具合の一枚」だけで見るため `%` は出さない（`percent` は None）。
    題名と始めた時刻は `request_radio` が `_radio_meta` に覚えている。
    """
    name = project["name"]
    out: list[dict] = []
    for (pname, paper_id), task in list(_radio_jobs.items()):
        if pname != name or task.done():
            continue
        meta = _radio_meta.get((pname, paper_id)) or {}
        out.append(
            {
                "key": f"radio:{paper_id}",
                "kind": "radio",
                "title": str(meta.get("title") or "音声化"),
                "status": "processing",
                "paper_id": paper_id,
                "error": None,
                "started_at": meta.get("started_at"),
                "finished_at": None,
                "steps": [],
                "step": 0,
                "percent": None,
                "messages": [],
            }
        )
    return out


def list_downloads(project: dict) -> dict:
    """そのプロジェクトの「仕事」一覧（新しい順・U3 §変更内容 2・U05 §1 で種類ごとに）。

    束の取り込み（bundle）・引用の抜き出し・要旨の訳・PDF の訳は `_download_jobs` に入れている。
    音声化だけは `_radio_jobs` にあるので、`kind="radio"` として同じ並びに入れる
    （画面の「進み具合の一枚」は 1 つの `jobs` だけを見る）。
    終わった仕事も 30 分は出す（画面が後から見るため）。30 分を過ぎたものは消す。
    """
    name = project["name"]
    now = time.monotonic()
    out: list[dict] = _radio_jobs_for(project)
    expired: list[tuple[str, str]] = []
    for (pname, key), job in list(_download_jobs.items()):
        if pname != name:
            continue
        done_at = job.get("_done_at")
        if done_at is not None and now - done_at > _DOWNLOAD_KEEP:
            expired.append((pname, key))
            continue
        out.append(
            {
                key_name: job.get(key_name)
                for key_name in (
                    "key",
                    "kind",
                    "title",
                    "status",
                    "paper_id",
                    "error",
                    "started_at",
                    "finished_at",
                    "steps",
                    "step",
                    "percent",
                    "messages",
                )
            }
        )
    for stale in expired:
        _download_jobs.pop(stale, None)
    out.sort(key=lambda j: j.get("started_at") or "", reverse=True)
    return {"jobs": out}


# --- 引用一覧（`phone citations` / `phone citations-extract`・U05 §2） ---
# 引用の抜き出しは数十秒〜数分かかるため、Download と同じく背景の仕事として持つ。
# 段は 0 = PDF確認（サーバーが出す）/ 1 = 引用抽出（CLI）/ 2 = 完了（サーバー）。

# 引用の抜き出しの段（番号 2 は終わったことを表すので一覧の steps には出さない）
CITE_EXTRACT_STEPS = ["PDF確認", "引用抽出"]
# 引用の抜き出しの待ち時間（docling を使うので長め）
CITE_EXTRACT_TIMEOUT = 900.0


def citations(project: dict, paper_id: int) -> dict:
    """引用一覧（`paper citations <id>`・読み取りだけ）。"""
    return _paper_cli(project, ["paper", "citations", str(paper_id)])


async def request_cite_extract(project: dict, paper_id: int) -> dict:
    """引用の抜き出しの依頼（`POST …/papers/{id}/citations/extract`・U05 §2）。

    202 ですぐ返し、実際の抜き出しは `_cite_extract_job` を背景で走らせる。
    同じ論文が走っているときは 409（2 回同時に抜かない）。
    """
    key = f"cite:{paper_id}"
    name = project["name"]
    job = _download_jobs.get((name, key))
    if job is not None and job.get("status") == "processing":
        raise HTTPException(status_code=409, detail="この論文の引用は抜き出し中です")
    job = _new_job("cite-extract", key, f"#{paper_id} の引用", "processing", CITE_EXTRACT_STEPS, 0)
    job["paper_id"] = paper_id
    _download_jobs[(name, key)] = job
    job["_task"] = asyncio.create_task(_cite_extract_job(project, paper_id, job))
    return {"key": key, "status": "processing"}


async def _cite_extract_job(project: dict, paper_id: int, job: dict) -> None:
    """背景で引用を PDF から抜き出す（U05 §2）。

    - `phone citations-extract <id>`（900 秒まで）を `_run_paper_command_lines` で走らせる
      （`sai-progress step=1` が来るので段が進む）。
    - 終わったら `done`（`step=2` / 100%）。`docling_error` があれば `messages` に足す
      （引用は取れたが本文からの引用が取れなかったため）。
    - 失敗は `failed`。どちらも `notify.broadcast` で知らせる（Download と同じ作り）。
    """
    name = project["name"]
    root = str(project["root"])
    title = str(job.get("title") or "論文")
    fail_url = f"/#/paper/{name}/{paper_id}/cites"

    def on_line(line: str) -> None:
        """標準エラーの 1 行（`sai-progress` 以外はログへ）。"""
        if apply_progress_line(job, line):
            return
        text = str(line or "").strip()
        if text:
            logger.info(f"引用の抜き出し: {text[:200]}")

    try:
        code, stdout, reason = await _run_paper_command_lines(
            root, ["paper", "citations-extract", str(paper_id)], CITE_EXTRACT_TIMEOUT, on_line
        )
        if reason:
            _finish_download(job, "failed", error=reason)
            notify.broadcast("引用を抜けませんでした", f"{title}: {reason}", fail_url)
            return
        if code != 0:
            error = _radio_error(stdout)
            _finish_download(job, "failed", error=error)
            notify.broadcast("引用を抜けませんでした", f"{title}: {error}", fail_url)
            return
        data = _download_json(stdout)
        docling_error = data.get("docling_error")
        messages = list(job.get("messages") or [])
        if docling_error:
            messages.append(
                f"docling で本文を読めなかったので、S2 の引用だけにしました: {docling_error}"
            )
        job["messages"] = messages[-DOWNLOAD_MESSAGE_KEEP:]
        _finish_download(job, "done")
        notify.broadcast(
            "引用を抜けました",
            f"{title}: 引用 {int(data.get('refs') or 0)} 件",
            fail_url,
        )
    except Exception as e:  # サーバーは止めない（Download と同じ扱い）
        logger.exception(f"引用の抜き出しで失敗: {e}")
        if job.get("status") not in ("done", "failed"):
            _finish_download(job, "failed", error="原因不明のエラー")
            notify.broadcast("引用を抜けませんでした", f"{title}: 原因不明のエラー", fail_url)


# PDF の日本語訳の待ち時間
PDF_TRANSLATE_TIMEOUT = 1800.0


async def request_translate_pdf(project: dict, paper_id: int) -> dict:
    """PDF の日本語訳を始める（`POST …/papers/{id}/translate-pdf`・202 ですぐ返す）。

    仕事キーは `pdf:{id}`（要旨の訳 `paper:{id}` と別。同時に出ても上書きしない）。
    """
    name = project["name"]
    key = f"pdf:{paper_id}"
    job = _download_jobs.get((name, key))
    if job is not None and job.get("status") in ("downloading", "processing"):
        raise HTTPException(status_code=409, detail="この論文の PDF は翻訳中です")
    title = f"#{paper_id} の PDF の訳"
    job = _new_job("translate", key, title, "processing", [], None)
    job["paper_id"] = paper_id
    _download_jobs[(name, key)] = job
    job["_task"] = asyncio.create_task(_translate_pdf_job(project, paper_id, title, job))
    return {"key": key, "status": "processing"}


async def _translate_pdf_job(project: dict, paper_id: int, title: str, job: dict) -> None:
    """背景で `phone translate-pdf <id>` を走らせ、結果を通知する。"""
    url = f"/#/paper/{project['name']}/{paper_id}"

    def fail(error: str) -> None:
        _finish_download(job, "failed", error=error)
        notify.broadcast("PDF を翻訳できませんでした", f"{title}: {error}", url)

    try:
        code, stdout, reason = await _run_paper_command(
            str(project["root"]), ["paper", "translate-pdf", str(paper_id), "--json"], PDF_TRANSLATE_TIMEOUT
        )
        if reason:
            fail(reason)
        elif code != 0:
            fail(_radio_error(stdout))
        else:
            _finish_download(job, "done")
            notify.broadcast("📄 PDF の日本語訳ができました", title, url)
    except Exception as e:  # サーバーは止めない
        logger.exception(f"PDF の翻訳で失敗: {e}")
        if job.get("status") not in ("done", "failed"):
            fail("原因不明のエラー")


PAPER_INDEX_TIMEOUT = 1800.0


async def request_index(project: dict, paper_id: int) -> dict:
    """論文 1 本の Index を始める（`POST …/papers/{id}/index`・202 ですぐ返す・U04）。

    仕事キーは `index:{id}`。埋め込みは外の API を呼ぶので、背景で `sai-paper index paper` を走らせる。
    """
    name = project["name"]
    key = f"index:{paper_id}"
    job = _download_jobs.get((name, key))
    if job is not None and job.get("status") in ("downloading", "processing"):
        raise HTTPException(status_code=409, detail="この論文の Index は作成中です")
    title = f"#{paper_id} の Index"
    job = _new_job("index", key, title, "processing", [], None)
    job["paper_id"] = paper_id
    _download_jobs[(name, key)] = job
    job["_task"] = asyncio.create_task(_index_job(project, paper_id, title, job))
    return {"key": key, "status": "processing"}


async def _index_job(project: dict, paper_id: int, title: str, job: dict) -> None:
    """背景で `sai-paper index paper <id> --json` を走らせ、結果を通知する。"""
    url = f"/#/paper/{project['name']}/{paper_id}"

    def fail(error: str) -> None:
        _finish_download(job, "failed", error=error)
        notify.broadcast("Index を作れませんでした", f"{title}: {error}", url)

    try:
        code, stdout, reason = await _run_paper_command(
            str(project["root"]), ["index", "paper", str(paper_id), "--json"], PAPER_INDEX_TIMEOUT
        )
        if reason:
            fail(reason)
        elif code != 0:
            fail(_radio_error(stdout))
        else:
            data = json.loads(stdout)
            chunks = data.get("chunks", 0)
            embedded = data.get("chunks_with_embeddings", 0)
            if chunks == 0:
                # 本文（抽出したテキスト）が無いと chunk は 1 つも作れない
                fail("本文が無いので Index を作れませんでした")
            else:
                _finish_download(job, "done")
                notify.broadcast(
                    "🔎 Index ができました", f"{title}: chunk {chunks}・埋め込み {embedded}", url
                )
    except Exception as e:  # サーバーは止めない
        logger.exception(f"Index の作成で失敗: {e}")
        if job.get("status") not in ("done", "failed"):
            fail("原因不明のエラー")


# --- Project（state.md / mind.md / 要対応・U3 §変更内容 2） ---


def project_view(project: dict) -> dict:
    """現況・考え・状況のまとめ・要対応（`sai-sci project show --json`・U14）。

    研究の命令は sai-sci が行う（U14）。
    """
    return _sci_cli(project, ["project", "show", "--json"], timeout=30.0)


def mind_save(project: dict, text) -> dict:
    """mind.md の全文を保存する（`sai-sci project mind --json`・本文は標準入力・U04）。

    本文は文字列・空白以外を 1 文字以上含む・200000 字以下・NUL を含まない。
    """
    if (
        not isinstance(text, str)
        or not text.strip()
        or len(text) > MIND_MAX
        or "\0" in text
    ):
        raise HTTPException(
            status_code=400,
            detail=f"Mind は 1〜{MIND_MAX} 文字で書いてください",
        )
    return _sci_cli(project, ["project", "mind", "--json"], timeout=30.0, input_text=text)


def attention_resolve(project: dict, key) -> dict:
    """要対応の 1 件を「対応済み」にする（`sai-sci attention resolve <key> --json`・U14）。"""
    if (
        not isinstance(key, str)
        or not (1 <= len(key) <= ATTENTION_KEY_MAX)
        or any(c in key for c in "\r\n")
    ):
        raise HTTPException(
            status_code=400,
            detail=f"key は改行のない 1〜{ATTENTION_KEY_MAX} 文字にしてください",
        )
    return _sci_cli(project, ["attention", "resolve", key, "--json"])


def attention_list(project: dict) -> list:
    """要対応の一覧（`sai-sci attention list --json`・ホームの集計に使う・U14）。"""
    data = _sci_cli(project, ["attention", "list", "--json"], timeout=15.0)
    return data if isinstance(data, list) else []
