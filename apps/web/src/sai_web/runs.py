"""sai-link の書き出しファイル（~/.sai/web/runs/<project>/<run_id>.json）の
読み込みと，画面ごとの集計．

方針（00 §3 を正とする）:
- sai-web はこのフォルダを読むだけ。sai-link の DB には触らない。
- mtime が変わったファイルだけ読み直す（数百件でも毎回全部読まない）。
- 壊れた JSON・schema_version が 1 以外のファイルは飛ばしてログに出す。
- 「どの日の実行か」は started_at を日本時間に直した日付で決める（00 §3.6）。
"""

from __future__ import annotations

import json
import logging
from collections import Counter, defaultdict
from datetime import datetime, timedelta, timezone
from pathlib import Path

from sai_web.config import settings

logger = logging.getLogger("sai_web")

# 00 §3.6: 日付は日本時間で区切る（UTC の日のまま区切ると朝9時前の実行が前日になる）
_JST = timezone(timedelta(hours=9))

# 一覧に出してよい実行状態（todo / pending は未実行なので載せない・00 §3.2）
_VISIBLE_STATUSES = ("running", "completed", "failed", "cancelled")

# 実行中の失敗・中止の数え方（画面上は failed にまとめて出す）
_FAILED_STATUSES = ("failed", "cancelled")

# runner の表示順（決め事の例は cc → pi）
_RUNNER_ORDER = ("cc", "pi", "gl")

# ファイルの読み込み結果（絶対パス → (mtime_ns, Run or None)）。
# None は「読めなかった／対象外」。壊れたファイルを毎回読み直さないため入れておく。
_file_cache: dict[str, tuple[int, dict | None]] = {}
# transcript 先頭行のセッション ID（絶対パス → (mtime_ns, frozenset)）
_transcript_ids_cache: dict[str, tuple[int, frozenset[str]]] = {}


# --- 時刻とキーの小さな道具 ---


def _parse_time(value: object) -> datetime | None:
    """ISO 8601 の文字列を datetime にする（取りきれない形は None）．"""
    if not isinstance(value, str) or not value:
        return None
    text = value.strip()
    if text.endswith("Z"):
        text = text[:-1] + "+00:00"
    try:
        dt = datetime.fromisoformat(text)
    except ValueError:
        return None
    if dt.tzinfo is None:
        # DB 由来の無タイムゾーン文字列は UTC とみなす（00 §3.2 は UTC 指定）
        dt = dt.replace(tzinfo=timezone.utc)
    return dt


def run_key(run: dict) -> str:
    """sai-web 内での実行の識別子（00 §3.7）: <project_name>/<run_id>．"""
    return f"{run.get('project_name', '')}/{run.get('run_id', '')}"


def run_date(run: dict) -> str | None:
    """その実行が「どの日」か（日本時間・YYYY-MM-DD）．00 §3.6．"""
    dt = _parse_time(run.get("started_at")) or _parse_time(run.get("updated_at"))
    if dt is None:
        return None
    return dt.astimezone(_JST).strftime("%Y-%m-%d")


def run_time(run: dict) -> str:
    """画面に出す開始時刻（日本時間の HH:MM）．"""
    dt = _parse_time(run.get("started_at"))
    if dt is None:
        return ""
    return dt.astimezone(_JST).strftime("%H:%M")


def _start_dt(run: dict) -> datetime:
    """並べ替え用。取れない実行は一番古い扱いにする．"""
    return _parse_time(run.get("started_at")) or datetime.min.replace(tzinfo=timezone.utc)


# --- ファイルの読み込み ---


def _read_run(path: Path) -> dict | None:
    """1ファイル分を読む。対象外なら None（理由はログ）．"""
    try:
        raw = json.loads(path.read_text(encoding="utf-8"))
    except OSError as e:
        logger.warning(f"実行ファイルを読めません: {path}: {e}")
        return None
    except json.JSONDecodeError as e:
        # 01 は一時ファイル→rename で書くので起きないはずだが，念のため飛ばす
        logger.warning(f"実行ファイルが JSON として読めません（飛ばします）: {path}: {e}")
        return None
    if not isinstance(raw, dict):
        logger.warning(f"実行ファイルが JSON オブジェクトではありません（飛ばします）: {path}")
        return None
    if raw.get("schema_version") != 1:
        logger.warning(f"未対応の schema_version（飛ばします）: {path}")
        return None
    if not isinstance(raw.get("run_id"), int) or not isinstance(raw.get("project_name"), str):
        logger.warning(f"run_id / project_name が不正（飛ばします）: {path}")
        return None
    # user 実行と未実行は一覧に出さない（00 §3.2 / 00 §5）
    if raw.get("runner") == "user":
        return None
    if raw.get("status") not in _VISIBLE_STATUSES:
        return None
    return raw


def load_runs(runs_dir: Path | None = None) -> list[dict]:
    """runs_dir 直下の <project_name>/*.json を読む．

    - `.` で始まるフォルダ・ファイルは無視（01 の一時ファイル対策）。
    - mtime が変わったファイルだけ読み直す。
    """
    global _file_cache
    base = Path(runs_dir) if runs_dir is not None else settings.runs_dir
    found: dict[str, tuple[int, dict | None]] = {}
    try:
        project_dirs = [p for p in base.iterdir() if not p.name.startswith(".")]
    except OSError:
        # 置き場所が無くてもサーバーは動かす（01 がまだ書き出していない場合）
        _file_cache = {}
        return []
    for proj_dir in project_dirs:
        if not proj_dir.is_dir():
            continue
        try:
            children = list(proj_dir.iterdir())
        except OSError:
            continue
        for path in children:
            if path.name.startswith(".") or path.suffix != ".json":
                continue
            key = str(path)
            try:
                mtime = path.stat().st_mtime_ns
            except OSError:
                continue
            old = _file_cache.get(key)
            if old is not None and old[0] == mtime:
                found[key] = old  # 変わっていないので読み直さない
            else:
                # 新規・更新時だけ読む（壊れていたらログもこの時だけ出る）
                found[key] = (mtime, _read_run(path))
    _file_cache = found
    return [run for _, run in found.values() if run is not None]


# --- 実行と質問の対応づけ（00 §3.7） ---


def _transcript_session_ids(path: Path) -> frozenset[str]:
    """transcript 先頭数行からセッション ID を拾う（00 §3.7）．

    cc: 先頭行の sessionId。pi: 先頭行 type:session の id。
    mtime が変わっていない間は前回の結果を使う。
    """
    try:
        mtime = path.stat().st_mtime_ns
    except OSError:
        return frozenset()
    key = str(path)
    cached = _transcript_ids_cache.get(key)
    if cached is not None and cached[0] == mtime:
        return cached[1]
    ids: set[str] = set()
    try:
        with open(path, encoding="utf-8") as f:
            for _ in range(3):
                line = f.readline()
                if not line:
                    break
                line = line.strip()
                if not line:
                    continue
                try:
                    row = json.loads(line)
                except json.JSONDecodeError:
                    continue
                if not isinstance(row, dict):
                    continue
                sid = row.get("sessionId")
                if isinstance(sid, str) and sid:
                    ids.add(sid)
                if row.get("type") == "session" and isinstance(row.get("id"), str):
                    ids.add(row["id"])
    except OSError:
        return frozenset()
    result = frozenset(ids)
    _transcript_ids_cache[key] = (mtime, result)
    return result


def run_session_ids(run: dict) -> set[str]:
    """その実行に関係するセッション ID の集合（質問との対応づけに使う）．

    - cc: 質問の session_id ＝ agent_session_id（00 §3.7）
    - pi: 質問の session_id ＝ transcript 先頭行の id（00 §3.7）
    両方入れておけば片方しか見えない質問にも当たる。
    """
    ids: set[str] = set()
    agent_sid = run.get("agent_session_id")
    if isinstance(agent_sid, str) and agent_sid:
        ids.add(agent_sid)
    transcript = run.get("transcript_path")
    if isinstance(transcript, str) and transcript:
        ids |= set(_transcript_session_ids(Path(transcript)))
    return ids


def question_session_id(run: dict) -> str | None:
    """画面から /api/questions?session_id=… に渡す ID（00 §3.7）．"""
    if run.get("runner") == "cc":
        sid = run.get("agent_session_id")
        if isinstance(sid, str) and sid:
            return sid
    transcript = run.get("transcript_path")
    if isinstance(transcript, str) and transcript:
        ids = sorted(_transcript_session_ids(Path(transcript)))
        if ids:
            return ids[0]
    sid = run.get("agent_session_id")
    if isinstance(sid, str) and sid:
        return sid
    return None


def open_question_session_ids() -> set[str]:
    """未回答の質問が紐づくセッション ID の集合．

    端末側で先に答えられていた分はここで解決済みにしてから集計する
    （旧 /api/sessions と同じ判定・questions.refresh_question_state を流用）。
    """
    from sai_web import questions, store

    # 期限切れの pi 質問を先に片付ける（05 §3.1）
    questions.expire_stale_questions()
    for q in store.list_open_questions():
        pane_id = q.get("pane_id") or None
        if pane_id:
            questions.refresh_question_state(
                q["session_id"], pane_id, q.get("agent")
            )
    return {q["session_id"] for q in store.list_open_questions()}


def has_question(run: dict, question_session_ids) -> bool:
    """その実行に未回答の質問があるか（00 §3.7 の対応で判定）．"""
    return bool(run_session_ids(run) & set(question_session_ids))


# --- herdr のペイン対応づけ（00 §3.7） ---


def pane_for_run(run: dict, agents: list[dict], panes: list[dict]) -> dict | None:
    """実行に対応づく herdr のペインを探す．見つからなければ None。

    - herdr_pane_id があればそれを最優先（00 §3.7）。
    - なければペイン名（label）が herdr_pane_label と一致するもの。
    動作中エージェント（agents）を先に，全ペイン（panes）を後に見る。

    終わった実行はペインが消えているので探さない。ペイン名はプロジェクト間で
    重なる（sai-<ブロック名>-<実行番号>）ため，探すと別の agent に当たる。
    herdr_pane_id が分かっているときも，名前で探し直さない（同じ理由）。

    さらに herdr のペイン ID はワークスペース内の番号（w1Z:p8）で，ペインが
    消えたあとに同じ番号が再び使われる（計画書 A の U06）。そのため
    herdr_pane_id が一致しても，ペインにラベルが付いていて run の
    herdr_pane_label と違うときは別のプロジェクトのものと見て送らない。
    ラベルが付いていないときは今までどおり ID の一致だけで決める。
    """
    if run.get("status") != "running":
        return None
    pane_id = run.get("herdr_pane_id")
    label = run.get("herdr_pane_label")
    if pane_id:
        found = [a for src in (agents, panes) for a in src if a.get("pane_id") == pane_id]
        if not found:
            return None
        # ラベル付きの要素があればそれを使う（agent list には付かないので panes 側・herdr.list_panes）
        labeled = [a for a in found if a.get("label")]
        hit = labeled[0] if labeled else found[0]
        if labeled and label and hit.get("label") != label:
            return None
        return hit
    if label:
        for src in (agents, panes):
            for a in src:
                if a.get("label") == label:
                    return a
    return None


def live_status(run: dict, agents: list[dict], panes: list[dict]) -> str | None:
    """実行中のときだけ herdr の状態（working / blocked / idle）を返す．

    それ以外・ペインが見つからないときは None（02 §2.2 Run の live）。
    """
    from sai_web.sessions import _herdr_status_to_api

    if run.get("status") != "running":
        return None
    pane = pane_for_run(run, agents, panes)
    if pane is None:
        return None
    return _herdr_status_to_api(pane.get("agent_status"))


# --- 画面別の形にする ---


def run_to_api(
    run: dict,
    question_session_ids=frozenset(),
    agents: list[dict] | None = None,
    panes: list[dict] | None = None,
) -> dict:
    """画面に渡す Run（02 §2.2 の形）．"""
    return {
        "key": run_key(run),
        "project_name": run.get("project_name") or "",
        "run_id": run["run_id"],
        "parent_run_id": run.get("parent_run_id"),
        "block_name": run.get("block_name") or "",
        "description": run.get("description") or "",
        "runner": run.get("runner") or "",
        "status": run.get("status") or "",
        "model": run.get("model"),
        "started_at": run.get("started_at"),
        "finished_at": run.get("finished_at"),
        "duration_ms": run.get("duration_ms"),
        "date": run_date(run),
        "live": live_status(run, agents or [], panes or []),
        "has_question": has_question(run, question_session_ids),
        "question_session_id": question_session_id(run),
        "stats": run.get("stats"),
    }


def order_runs(runs: list[dict]) -> list[dict]:
    """1つの日付・1プロジェクト分の並べ替え（02 §1 の2番）．

    - 開始時刻の新しい順。
    - サブタスク（parent_run_id あり）は同じ範囲に親が居れば親の直下に
      字下げして並べる（indent=True）。
    - 親がその日に居なければ普通の位置に並べ， `↳ #親ID` を添える
      （show_parent=True）。
    返り値は浅いコピー（キャッシュ済み Run を壊さないため）。
    """
    by_id = {r["run_id"]: r for r in runs}
    children: dict[int, list[dict]] = defaultdict(list)
    child_ids: set[int] = set()
    for r in runs:
        pid = r.get("parent_run_id")
        if pid is not None and pid in by_id:
            children[pid].append(r)
            child_ids.add(r["run_id"])

    out: list[dict] = []

    def emit(run: dict, indent: bool, show_parent: bool) -> None:
        out.append({**run, "indent": indent, "show_parent": show_parent})
        kids = sorted(children.get(run["run_id"], []), key=_start_dt, reverse=True)
        for kid in kids:
            emit(kid, True, False)

    for run in sorted(runs, key=_start_dt, reverse=True):
        if run["run_id"] in child_ids:
            continue  # 親の直下で出される
        # 親がこの範囲に居ないサブタスクは普通に並べ，親 ID を添える
        emit(run, False, run.get("parent_run_id") is not None)
    return out


def build_days(runs: list[dict], question_session_ids=frozenset()) -> list[dict]:
    """日付ごとのまとめ（02 §2.2 /api/days）．新しい日が先．"""
    by_date: dict[str, list[dict]] = defaultdict(list)
    for r in runs:
        d = run_date(r)
        if d:
            by_date[d].append(r)

    days: list[dict] = []
    for date, rs in by_date.items():
        done_counter = Counter(
            r.get("runner") for r in rs if r.get("status") == "completed"
        )
        # runner 0 の件は出さない（表示順は cc → pi → その他）
        done: dict[str, int] = {}
        for runner in _RUNNER_ORDER:
            if done_counter.get(runner):
                done[runner] = done_counter.pop(runner)
        for runner, n in sorted(done_counter.items()):
            if n:
                done[runner] = n

        cost = 0.0
        cost_seen = False
        cost_estimate = False
        for r in rs:
            stats = r.get("stats")
            if not isinstance(stats, dict):
                continue  # 実行中は stats が null（00 §3.4）
            total = (stats.get("cost_usd") or {}).get("total")
            if isinstance(total, (int, float)):
                cost += float(total)
                cost_seen = True
                if stats.get("cost_is_estimate") is True:
                    cost_estimate = True

        days.append(
            {
                "date": date,
                "weekday": datetime.strptime(date, "%Y-%m-%d").strftime("%a"),
                "done": done,
                "running": sum(1 for r in rs if r.get("status") == "running"),
                "failed": sum(1 for r in rs if r.get("status") in _FAILED_STATUSES),
                "cost_usd": round(cost, 2) if cost_seen else 0.0,
                "cost_is_estimate": cost_estimate,
                "has_question": any(
                    has_question(r, question_session_ids) for r in rs
                ),
            }
        )
    days.sort(key=lambda d: d["date"], reverse=True)
    return days


def build_day(
    runs: list[dict], date: str, question_session_ids=frozenset(),
    agents: list[dict] | None = None, panes: list[dict] | None = None,
) -> dict:
    """1日の実行一覧（02 §2.2 /api/days/{date}）．"""
    day_runs = [r for r in runs if run_date(r) == date]
    grouped: dict[str, list[dict]] = defaultdict(list)
    for r in day_runs:
        grouped[r.get("project_name") or ""].append(r)

    # プロジェクトはその日の最新実行が新しい順
    project_order = sorted(
        grouped.keys(),
        key=lambda name: max(_start_dt(r) for r in grouped[name]),
        reverse=True,
    )
    projects = []
    for name in project_order:
        ordered = order_runs(grouped[name])
        projects.append(
            {
                "project_name": name,
                "runs": [
                    {
                        **run_to_api(r, question_session_ids, agents, panes),
                        "indent": r.get("indent", False),
                        "show_parent": r.get("show_parent", False),
                    }
                    for r in ordered
                ],
            }
        )
    return {"date": date, "projects": projects}


def search_runs(runs: list[dict], keyword: str, question_session_ids=frozenset()) -> list[dict]:
    """日付をまたいで説明・ブロック名・プロジェクト名に一致した実行（02 §3.2）．"""
    kw = (keyword or "").strip()
    if not kw:
        return []
    hits = [
        r
        for r in runs
        if kw in (r.get("description") or "")
        or kw in (r.get("block_name") or "")
        or kw in (r.get("project_name") or "")
    ]
    hits.sort(key=_start_dt, reverse=True)
    return [run_to_api(r, question_session_ids) for r in hits]
