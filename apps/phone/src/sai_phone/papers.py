"""sai-paper の呼び出し（Papers タブ・計画 07・複数プロジェクト対応は 17）。

第 1 弾の Web 機能（U3）で足したもの:
- Import（BibTeX の読み込み）: `phone import list / add / remove`
- Download（PDF の取得と Library への登録）: `phone download` → `phone after-download`（背景で走らせる）
- Project（state.md / mind.md / 要対応）: `phone project` / `phone mind-append` / `phone attention-resolve`

sai-phone は他アプリの DB を直接開かない（00 §2）ため，ここはすべて CLI に頼る。
呼ぶ先は「論文の置き場があるプロジェクト」（計画 17 §3.1）で決めるもので，
cwd と `SAI_PAPER_ROOT` を **そのプロジェクトの root** にする
（sai-paper の `.env` の API キーを読ませるため・00 §8・17 §3.2）。
iPhone からはパスを受け取らず，プロジェクト名から root を引く（17 §3.2）。

使う CLI（00 §7.2）:
- `phone library` / `phone tree` / `phone paper <id>`（読み取り・02）
- `phone translate <id>`（要旨の日本語訳・03）
- `content paper <id> --json`（本文・既存）
- `phone providers` / `phone search` / `phone log <id>`（Discover・02・03・計画 08）
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

from sai_phone import cli, control, notify
from sai_phone.config import settings

logger = logging.getLogger("sai_phone")


# --- 論文の置き場があるプロジェクト（計画 17 §3.1・§3.2） ---

# プロジェクト名の検査（画面の住所と音声の名前に使うため，合わない名前は対象外・17 §3.1）
_PROJECT_NAME_RE = re.compile(r"^[A-Za-z0-9_.-]{1,64}$")
# 論文の置き場のしるし（`sai-paper init` でできる。ファイルがあるかどうか見るだけで
# DB は開かない・00 §2・17 §3.1）
_PAPER_DB_REL = Path(".sai") / "paper" / "papr.db"
# 一覧を覚えておく秒数（プロジェクト一覧を呼びすぎないため・17 §3.2）
_PROJECTS_TTL = 30.0
# 覚えておいた一覧（(時刻, 一覧)）。中身は root の絶対パスも含む内部用
_projects_cache: tuple[float, list[dict]] | None = None


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
    if (
        not refresh
        and _projects_cache is not None
        and now - _projects_cache[0] < _PROJECTS_TTL
    ):
        return _projects_cache[1]
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
    _projects_cache = (now, out)
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
    """sai-paper の CLI を呼ぶ（cwd と SAI_PAPER_ROOT をそのプロジェクトの root にする・17 §3.2）。"""
    root = str(project["root"])
    return cli.run_json(
        [settings.sai_paper_bin, *args],
        app="sai-paper",
        cwd=root,
        env={**os.environ, "SAI_PAPER_ROOT": root},
        timeout=timeout,
    )


def library(project: dict) -> dict:
    """Library 一覧（`phone library`）。"""
    return _paper_cli(project, ["phone", "library"])


def tree(project: dict) -> dict:
    """Library 階層（`phone tree`）。"""
    return _paper_cli(project, ["phone", "tree"])


def detail(project: dict, paper_id: int) -> dict:
    """論文情報（`phone paper <id>`）。見つからなければ 400 で失敗。"""
    return _paper_cli(project, ["phone", "paper", str(paper_id)])


def translate(project: dict, paper_id: int) -> dict:
    """要旨の日本語訳を作る（`phone translate <id>`）。数十秒かかりうるため待ちは長め。"""
    return _paper_cli(project, ["phone", "translate", str(paper_id)], timeout=90.0)


def text(project: dict, paper_id: int) -> dict:
    """本文（既存の `content paper <id> --json`）。md_path は画面に返さない（07 §3.2）。"""
    data = _paper_cli(project, ["content", "paper", str(paper_id), "--json"])
    if not isinstance(data, dict):
        raise HTTPException(status_code=502, detail="sai-paper の応答を読めませんでした")
    return {"id": data.get("id", paper_id), "text": data.get("text") or ""}


def dashboard(project: dict) -> dict:
    """Dashboard（`phone dashboard`・数字カードと Radio の出来事を混ぜた出来事の一覧）。"""
    return _paper_cli(project, ["phone", "dashboard"])


# ノートの大きさの上限（100KB・task.md と同じ）
NOTE_MAX = 100 * 1024


def save_note(project: dict, paper_id: int, body) -> dict:
    """ノート（note.md）を保存する（`phone note <id>`・本文は標準入力）。"""
    if not isinstance(body, str) or len(body.encode("utf-8")) > NOTE_MAX:
        raise HTTPException(status_code=400, detail="ノートは 100KB までの文字にしてください")
    root = str(project["root"])
    return cli.run_json(
        [settings.sai_paper_bin, "phone", "note", str(paper_id)],
        app="sai-paper",
        cwd=root,
        env={**os.environ, "SAI_PAPER_ROOT": root},
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
    """検索先の一覧と既定（`phone providers`・08 §3）。"""
    return _paper_cli(project, ["phone", "providers"])


def search_log(project: dict, log_id: int) -> dict:
    """過去の検索結果（`phone log <log_id>`・08 §3）。"""
    return _paper_cli(project, ["phone", "log", str(log_id)])


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
    """`phone search` の引数を作る（検査もここで行う・08 §3）。

    検索語は最後に `--` を置いて渡す（語が `-` で始めると clap が
    オプションと間違えるため・08 §2）。
    """
    query = validate_search(req, available)
    args = ["phone", "search", "--limit", str(req.limit)]
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


# --- 音声化の依頼（計画 10 §3・プロジェクト付きは 17 §3.3・§3.4） ---

# 今動いている音声化（(プロジェクト名, 論文 id) → タスク）。サーバーを再起動すると完了通知は失われる
# （状態そのものは sai-paper の paper_podcasts に残るため，画面はそちらを見る・10 §3）。
# 別プロジェクトに同じ番号の論文があっても「作成中」と誤判定しないよう名前と番号の組で持つ（17 §3.4）。
_radio_jobs: dict[tuple[str, int], asyncio.Task] = {}
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

    sai-phone 側で組み立てた名前と食い違うことがあるため，返ってきたものが正。
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
                "phone",
                "radio",
                str(paper_id),
                cwd=root,
                env={**os.environ, "SAI_PAPER_ROOT": root},
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
            notify.broadcast("音声を作れませんでした", f"{title}: {_radio_error(stdout)}", fail_url)
    except Exception as e:  # サーバーは止めない（10 §3）
        logger.exception(f"音声化の背景実行で失敗: {e}")
    finally:
        _radio_jobs.pop((name, paper_id), None)


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


# --- Canvas（sai-paper phone canvas の中継・U04） ---
# sai-paper-tui の Canvas の画面（U06 / U07）を iPhone へ移すため，
# sai-paper の `sai-paper phone canvas …`（U03）を中継する（U04 §目的）。
# 画面からは直接 CLI を呼ばず，必ずこの中の検査を通す
# （外から来る値で CLI を呼ぶため・U04 §決定事項）。

# Canvas の名前の長さ（1〜128 文字・U04 C-D）
CANVAS_NAME_MAX = 128
# ノードの id（U04 C-D）
_NODE_ID_RE = re.compile(r"^[A-Za-z0-9_.:-]{1,128}$")
# 位置の上限（±10,000,000・U04 C-D）
LAYOUT_COORD_MAX = 10_000_000
# 大きさの範囲（1〜100,000・U04 C-D）
LAYOUT_SIZE_MIN = 1
LAYOUT_SIZE_MAX = 100_000
# 一度に送れるノードの数（1〜5000・U04 C-D）
LAYOUT_NODES_MIN = 1
LAYOUT_NODES_MAX = 5000
# `show` / `missing` の待ち時間（note.md を読むため長め・U04 C-D）
CANVAS_SLOW_TIMEOUT = 60.0


def validate_canvas_name(name) -> str:
    """Canvas の名前を検査して返す（違反は 400・U04 C-D）。

    - 1〜128 文字の文字列
    - `/` `\\` と制御文字（U+0000〜U+001F・U+007F）を含まない
    - `.` / `..` でない
    """
    ok = (
        isinstance(name, str)
        and 1 <= len(name) <= CANVAS_NAME_MAX
        and name not in (".", "..")
        and "/" not in name
        and "\\" not in name
        and not any(ord(c) < 0x20 or ord(c) == 0x7F for c in name)
    )
    if not ok:
        raise HTTPException(status_code=400, detail="Canvas の名前が正しくありません")
    return name


def validate_node_id(node_id) -> str:
    """ノードの id を検査して返す（`^[A-Za-z0-9_.:-]{1,128}$`・U04 C-D）。"""
    if not isinstance(node_id, str) or not _NODE_ID_RE.match(node_id):
        raise HTTPException(status_code=400, detail="ノードの id が正しくありません")
    return node_id


def _validate_layout_coord(value) -> int:
    """位置の値（整数・bool 不可・±10,000,000 以内）。違反は 400。"""
    if not isinstance(value, int) or isinstance(value, bool):
        raise HTTPException(status_code=400, detail="位置の指定が正しくありません")
    if abs(value) > LAYOUT_COORD_MAX:
        raise HTTPException(status_code=400, detail="位置の指定が正しくありません")
    return value


def _validate_layout_size(value) -> int:
    """大きさの値（1〜100,000 の整数）。違反は 400。"""
    if not isinstance(value, int) or isinstance(value, bool):
        raise HTTPException(status_code=400, detail="位置の指定が正しくありません")
    if not (LAYOUT_SIZE_MIN <= value <= LAYOUT_SIZE_MAX):
        raise HTTPException(status_code=400, detail="位置の指定が正しくありません")
    return value


def validate_layout(payload) -> dict:
    """位置の指定を検査して `{"nodes": [...]}` の形に整えて返す（違反は 400・U04 C-D）。

    - `nodes` は 1〜5000 件の配列
    - `x` / `y` は整数（bool 不可）で絶対値が 10,000,000 以下
    - `width` / `height` があれば 1〜100,000 の整数
    - 他の項目は落とす（CLI が見るのは位置だけ・C-C）
    """
    def bad() -> HTTPException:
        return HTTPException(status_code=400, detail="位置の指定が正しくありません")

    if not isinstance(payload, dict):
        raise bad()
    nodes = payload.get("nodes")
    if not isinstance(nodes, list):
        raise bad()
    if not (LAYOUT_NODES_MIN <= len(nodes) <= LAYOUT_NODES_MAX):
        raise bad()
    out: list[dict] = []
    for node in nodes:
        if not isinstance(node, dict):
            raise bad()
        item = {
            "id": validate_node_id(node.get("id")),
            "x": _validate_layout_coord(node.get("x")),
            "y": _validate_layout_coord(node.get("y")),
        }
        for key in ("width", "height"):
            if node.get(key) is not None:
                item[key] = _validate_layout_size(node.get(key))
        out.append(item)
    return {"nodes": out}


def canvas_list(project: dict) -> dict:
    """Canvas 一覧（`phone canvas list`）。"""
    return _paper_cli(project, ["phone", "canvas", "list"])


def canvas_new(project: dict, name: str) -> dict:
    """Canvas を新しく作る（`phone canvas new -- <name>`）。"""
    return _paper_cli(project, ["phone", "canvas", "new", "--", validate_canvas_name(name)])


def canvas_show(project: dict, canvas: str) -> dict:
    """Canvas の中身（`phone canvas show -- <canvas>`・note.md も読むため待ちは 60 秒）。"""
    return _paper_cli(
        project,
        ["phone", "canvas", "show", "--", validate_canvas_name(canvas)],
        timeout=CANVAS_SLOW_TIMEOUT,
    )


def canvas_layout(project: dict, canvas: str, payload) -> dict:
    """位置を保存する（`phone canvas layout -- <canvas>`・本文は標準入力）。

    `save_note` と同じく `cli.run_json` を直接呼ぶ（cwd と `SAI_PAPER_ROOT` も同じ）。
    """
    data = validate_layout(payload)
    root = str(project["root"])
    return cli.run_json(
        [
            settings.sai_paper_bin,
            "phone",
            "canvas",
            "layout",
            "--",
            validate_canvas_name(canvas),
        ],
        app="sai-paper",
        cwd=root,
        env={**os.environ, "SAI_PAPER_ROOT": root},
        input_text=json.dumps(data),
    )


def canvas_toggle(project: dict, canvas: str, node_id: str, near: str | None) -> dict:
    """札を出す・隠す（`phone canvas toggle [--near <id>] -- <canvas> <node_id>`）。

    `near` が無ければ `--near` を付けない（`--` の前・U03 の決まり）。
    """
    name = validate_canvas_name(canvas)
    node = validate_node_id(node_id)
    args = ["phone", "canvas", "toggle"]
    if near is not None:
        args += ["--near", validate_node_id(near)]
    args += ["--", name, node]
    return _paper_cli(project, args)


def canvas_missing(project: dict, canvas: str, node_id: str) -> dict:
    """Canvas に無いつながりの一覧（`phone canvas missing -- <canvas> <node_id>`・待ちは 60 秒）。"""
    return _paper_cli(
        project,
        [
            "phone",
            "canvas",
            "missing",
            "--",
            validate_canvas_name(canvas),
            validate_node_id(node_id),
        ],
        timeout=CANVAS_SLOW_TIMEOUT,
    )


def canvas_body(project: dict, canvas: str, node_id: str) -> dict:
    """ノードの本文（`phone canvas body -- <canvas> <node_id>`）。"""
    return _paper_cli(
        project,
        [
            "phone",
            "canvas",
            "body",
            "--",
            validate_canvas_name(canvas),
            validate_node_id(node_id),
        ],
    )


# --- Import（BibTeX の読み込み）と Download・Project（U3 §変更内容 2） ---
# 外から来る値（.bib の名前・本文・Download の依頼・メモ・key）は
# すべて CLI を呼ぶ前にここで確かめる（共有契約 C-A2）。

# .bib の名前の規則（英数字・.・-・_ で .bib で終わる 80 文字まで）
_BIB_NAME_RE = re.compile(r"^[A-Za-z0-9._-]{1,80}\.bib$")
# .bib の中身の大きさの上限（1MB）
BIB_TEXT_MAX = 1024 * 1024
# メモ 1 行の長さの上限（mind-append は 1 行だけ書く）
MEMO_MAX = 2000
# 要対応の key の長さの上限
ATTENTION_KEY_MAX = 300
# 題名（通知の文に使うだけ）の長さの上限
DOWNLOAD_TITLE_MAX = 200
# Download（PDF を取りに行きて登録するまで）の待ち時間
DOWNLOAD_TIMEOUT = 300.0
# 登録後の自動処理（図の抽出・PDF の翻訳）の待ち時間
AFTER_DOWNLOAD_TIMEOUT = 2000.0
# 終わった Download の仕事を一覧に残す秒数（30 分）
_DOWNLOAD_KEEP = 1800.0
# Download の仕事の段（0 = PDF取得, 1 = 図抽出, 2 = 翻訳, 3 = 完了）。
# 段の名前は画面（paperimport.js）と共有する。
DOWNLOAD_STEPS = ["PDF取得", "図抽出", "翻訳"]
# 仕事のログに残す行数の上限（古いものから捨てる）
DOWNLOAD_MESSAGE_KEEP = 50

# 今の Download の仕事（(プロジェクト名, 仕事のキー) → 仕事の dict）。
# 状態は sai-phone のメモリだけに持つ（音声化と同じ作り・再起動しても論文は sai-paper に残る）。
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
    """.bib の一覧と中の論文（`phone import list`・U3 §変更内容 2）。"""
    return _paper_cli(project, ["phone", "import", "list"])


def import_add(project: dict, name, text) -> dict:
    """.bib の保存（`phone import add --name <name>`・本文は標準入力）。

    名前に空文字が来たときは `phone-<YYYYMMDD-HHMMSS>.bib` を作る。
    `save_note` と同じく `cli.run_json` を直接呼ぶ（cwd と `SAI_PAPER_ROOT` も同じ）。
    """
    if name is None:
        name = ""
    if not isinstance(name, str):
        raise HTTPException(status_code=400, detail="ファイル名は文字列で送ってください")
    file_name = _default_bib_name() if not name.strip() else _validate_bib_name(name)
    body = _validate_bib_text(text)
    root = str(project["root"])
    return cli.run_json(
        [settings.sai_paper_bin, "phone", "import", "add", "--name", file_name],
        app="sai-paper",
        cwd=root,
        env={**os.environ, "SAI_PAPER_ROOT": root},
        input_text=body,
    )


def import_remove(project: dict, file_name) -> dict:
    """.bib の削除（`phone import remove --name <name>`）。"""
    name = _validate_bib_name(file_name)
    return _paper_cli(project, ["phone", "import", "remove", "--name", name])


# --- Download（PDF を取って Library に登録する・U3 §変更内容 2） ---


def _download_payload(payload) -> tuple[str, list[str], str]:
    """Download の依頼を確かめて `(仕事のキー, CLI の引数, 題名)` を返す（違反は 400）。

    - 検索履歴の 1 件: `{"log_id": 3, "position": 2}` → キー `log:3:2`
    - BibTeX の 1 件: `{"import_id": "DOI:…"}` → キー `import:DOI:…`
    - 両方・どちらも無しは 400。`title` は通知の文に使うだけなので任意。
    """
    if not isinstance(payload, dict):
        raise HTTPException(status_code=400, detail="送る値が正しくありません")
    log_id = payload.get("log_id")
    position = payload.get("position")
    import_id = payload.get("import_id")
    has_log = log_id is not None or position is not None
    has_import = import_id is not None
    if has_log and has_import:
        raise HTTPException(
            status_code=400, detail="log_id と import_id はどちらか一方を指定してください"
        )
    if not has_log and not has_import:
        raise HTTPException(
            status_code=400, detail="log_id か import_id のどちらか一方を指定してください"
        )
    if has_log:
        for label, value in (("log_id", log_id), ("position", position)):
            if not isinstance(value, int) or isinstance(value, bool) or value <= 0:
                raise HTTPException(
                    status_code=400, detail=f"{label} は 1 以上の整数にしてください"
                )
        key = f"log:{log_id}:{position}"
        args = ["--log", str(log_id), "--position", str(position)]
    else:
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
    title = payload.get("title")
    if title is None:
        title = "論文"
    elif not isinstance(title, str):
        raise HTTPException(status_code=400, detail="title は文字列で送ってください")
    else:
        title = title[:DOWNLOAD_TITLE_MAX].strip() or "論文"
    return key, args, title


def _download_json(stdout: bytes | None) -> dict:
    """`phone download` の応答を読む（読めなければ空の dict）。"""
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
    if status == "done":
        job["step"] = len(DOWNLOAD_STEPS)
        job["percent"] = 100
    job["_done_at"] = time.monotonic()


def apply_progress_line(job: dict, line: str) -> bool:
    """`sai-paper` の標準エラーの 1 行を仕事の進み具合へ反映する（`sai-progress …` だけ True）。

    受け付ける 2 つの形:
    - `sai-progress step=<n>`: 段を進める（% は段から作る。0/33/66/100）
    - `sai-progress msg=<文>`: ログを 1 行足す（最大 50 行）

    それ以外の行は進捗ではなく `False`（呼び出し側は通常のログとして出す）。
    """
    text = str(line or "").strip()
    if text.startswith("sai-progress step="):
        raw = text[len("sai-progress step=") :].strip()
        try:
            step = int(raw)
        except ValueError:
            return False
        # 段の番号は 0..len(DOWNLOAD_STEPS) に収める（越えた数字は端の段にする）。
        step = max(0, min(step, len(DOWNLOAD_STEPS)))
        job["step"] = step
        job["percent"] = step * 100 // len(DOWNLOAD_STEPS)
        return True
    if text.startswith("sai-progress msg="):
        message = text[len("sai-progress msg=") :]
        messages = list(job.get("messages") or [])
        messages.append(message)
        job["messages"] = messages[-DOWNLOAD_MESSAGE_KEEP:]
        return True
    return False


async def request_download(project: dict, payload) -> dict:
    """Download の依頼（`POST …/papers/download`・U3 §変更内容 2）。

    ブラウザにはすぐ返し（202）、実際の取得と登録は `_download_job` を背景で走らせる
    （音声化と同じ作り・同じ論文を 2 回同時に取らない）。
    """
    key, args, title = _download_payload(payload)
    name = project["name"]
    job = _download_jobs.get((name, key))
    if job is not None and job.get("status") in ("downloading", "processing"):
        raise HTTPException(status_code=409, detail="この論文はダウンロード中です")
    job = {
        "key": key,
        "title": title,
        "status": "downloading",
        "paper_id": None,
        "error": None,
        "started_at": _now_iso(),
        "finished_at": None,
        # 進み具合（段の定義は DOWNLOAD_STEPS・% は段から作る）
        "steps": list(DOWNLOAD_STEPS),
        "step": 0,
        "percent": 0,
        "messages": [],
    }
    _download_jobs[(name, key)] = job
    job["_task"] = asyncio.create_task(_download_job(project, key, args, title, job))
    return {"key": key, "status": "downloading"}


async def _run_paper_command(
    root: str, args: list[str], timeout: float
) -> tuple[int, bytes, str]:
    """sai-paper の phone サブコマンドを背景で走らせて `(終了コード, stdout, 失敗理由)` を返す。

    失敗理由は「起動できない」「時間切れ」の 2 つだけ。それ以外は空文字で、
    応答の中身は呼び出し側が見る（`_radio_job` と同じ作り）。
    """
    try:
        proc = await asyncio.create_subprocess_exec(
            settings.sai_paper_bin,
            *args,
            cwd=root,
            env={**os.environ, "SAI_PAPER_ROOT": root},
            stdout=asyncio.subprocess.PIPE,
            stderr=asyncio.subprocess.PIPE,
        )
    except (FileNotFoundError, OSError) as e:
        logger.warning(f"sai-paper を起動できません: {e}")
        return -1, b"", "sai-paper が見つかりません"
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
    return proc.returncode or 0, stdout, ""


async def _run_paper_command_lines(
    root: str, args: list[str], timeout: float, on_line
) -> tuple[int, bytes, str]:
    """`_run_paper_command` と同じ作りだが、標準エラーを 1 行ずつ `on_line` に渡す。

    `phone after-download` は図の抽出と PDF の翻訳で数分かかることがあるため、
    待つあいだの標準エラー（`sai-progress step=…` / `sai-progress msg=…`）を
    途中で受け取る。標準出力は終わってから読む。
    返り値と時間切れの扱いは `_run_paper_command` と同じ。
    """
    try:
        proc = await asyncio.create_subprocess_exec(
            settings.sai_paper_bin,
            *args,
            cwd=root,
            env={**os.environ, "SAI_PAPER_ROOT": root},
            stdout=asyncio.subprocess.PIPE,
            stderr=asyncio.subprocess.PIPE,
        )
    except (FileNotFoundError, OSError) as e:
        logger.warning(f"sai-paper を起動できません: {e}")
        return -1, b"", "sai-paper が見つかりません"

    async def run() -> tuple[int, bytes]:
        while True:
            raw = await proc.stderr.readline()
            if not raw:
                break
            on_line(raw.decode("utf-8", "replace").rstrip("\r\n"))
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
    return code, stdout, ""


async def _download_job(
    project: dict, key: str, args: list[str], title: str, job: dict
) -> None:
    """背景で論文の PDF を取得し、Library に登録する（U3 §変更内容 2）。

    1. `sai-paper phone download …`（300 秒まで）。`downloaded` なら `processing` にして
       「保存しました」と通知し、続けて `phone after-download`（図の抽出・PDF 翻訳・
       2000 秒まで）を走らせる。終わったら `done`（後処理が失敗しても `done` にして
       `error` に理由を入れる。論文自体は Library に入っているため）。
    2. `exists`（既に Library に有る）なら `done` にして通知しない。
    3. 失敗は `failed` にして通知する。
    4. 例外は全部つかまえてログに出す（サーバーは止めない）。
    """
    name = project["name"]
    root = str(project["root"])
    fail_url = "/#/papers/library"

    def fail(error: str) -> None:
        _finish_download(job, "failed", error=error)
        notify.broadcast("論文を保存できませんでした", f"{title}: {error}", fail_url)

    try:
        code, stdout, reason = await _run_paper_command(
            root, ["phone", "download", *args], DOWNLOAD_TIMEOUT
        )
        if reason:
            fail(reason)
            return
        if code != 0:
            fail(_radio_error(stdout))
            return
        data = _download_json(stdout)
        paper_id = data.get("paper_id")
        status = data.get("status")
        if status == "exists":
            job["paper_id"] = paper_id if isinstance(paper_id, int) else None
            _finish_download(job, "done")
            return
        if (
            status != "downloaded"
            or not isinstance(paper_id, int)
            or isinstance(paper_id, bool)
        ):
            fail("sai-paper の応答を読めませんでした")
            return
        # 登録できたので、まず「保存した」と知らせる（その間が processing）
        job["paper_id"] = paper_id
        job["status"] = "processing"
        # PDF は取れたので「図の抽出」の段から
        job["step"] = 1
        job["percent"] = 1 * 100 // len(DOWNLOAD_STEPS)
        notify.broadcast("📄 論文を保存しました", title, f"/#/paper/{name}/{paper_id}")

        def on_line(line: str) -> None:
            """`sai-paper` の標準エラーの 1 行を受け取る（進捗以外はログに）。"""
            if apply_progress_line(job, line):
                return
            text = str(line or "").strip()
            if text:
                logger.info(f"論文の後処理: {text[:200]}")

        # 続けて自動処理（図の抽出と PDF の翻訳）。失敗しても論文は入っているので done にする。
        code2, stdout2, reason2 = await _run_paper_command_lines(
            root,
            ["phone", "after-download", str(paper_id)],
            AFTER_DOWNLOAD_TIMEOUT,
            on_line,
        )
        if reason2:
            _finish_download(job, "done", error=reason2)
        elif code2 != 0:
            _finish_download(job, "done", error=_radio_error(stdout2))
        else:
            _finish_download(job, "done")
    except Exception as e:  # サーバーは止めない（音声化と同じ扱い）
        logger.exception(f"論文のダウンロードで失敗: {e}")
        if job.get("status") not in ("done", "failed"):
            fail("原因不明のエラー")


def list_downloads(project: dict) -> dict:
    """そのプロジェクトの Download の仕事（新しい順・U3 §変更内容 2）。

    終わった仕事も 30 分は出す（画面が後から見るため）。30 分を過ぎたものは消す。
    """
    name = project["name"]
    now = time.monotonic()
    out: list[dict] = []
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


# --- Project（state.md / mind.md / 要対応・U3 §変更内容 2） ---


def project_view(project: dict) -> dict:
    """現況・考え・状況のまとめ・要対応（`phone project`・U3 §変更内容 2）。"""
    return _paper_cli(project, ["phone", "project"], timeout=30.0)


def mind_append(project: dict, text) -> dict:
    """mind.md のメモを 1 行足す（`phone mind-append`・本文は標準入力）。"""
    if (
        not isinstance(text, str)
        or not (1 <= len(text) <= MEMO_MAX)
        or any(c in text for c in "\r\n")
    ):
        raise HTTPException(
            status_code=400, detail=f"メモは改行なしで 1〜{MEMO_MAX} 文字にしてください"
        )
    root = str(project["root"])
    return cli.run_json(
        [settings.sai_paper_bin, "phone", "mind-append"],
        app="sai-paper",
        cwd=root,
        env={**os.environ, "SAI_PAPER_ROOT": root},
        input_text=text,
    )


def attention_resolve(project: dict, key) -> dict:
    """要対応の 1 件を「対応済み」にする（`phone attention-resolve <key>`）。"""
    if (
        not isinstance(key, str)
        or not (1 <= len(key) <= ATTENTION_KEY_MAX)
        or any(c in key for c in "\r\n")
    ):
        raise HTTPException(
            status_code=400,
            detail=f"key は改行のない 1〜{ATTENTION_KEY_MAX} 文字にしてください",
        )
    return _paper_cli(project, ["phone", "attention-resolve", key])


def attention_list(project: dict) -> list:
    """要対応の一覧（`attention --json`・ホームの集計に使う）。"""
    data = _paper_cli(project, ["attention", "--json"], timeout=15.0)
    return data if isinstance(data, list) else []
