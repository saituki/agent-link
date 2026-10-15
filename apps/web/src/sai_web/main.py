"""sai-web 本体サーバー (Mac 側)．

- sai-link が書き出した実行の一覧（日付ごと・1日ごと・1実行）
- 実行の会話（最初から最後まで）と続き（差分）
- iPhone 用の画面（PWA）の配信
- テキスト送信・質問の登録と回答

会話の続きの届け方はポーリング（2秒おきに ?after= で取りに来る）．
WebSocket や SSE は使わない．
理由: 4G で切れたときの再接続処理が増え，
「切れている間の取りこぼし」対策と二重に作ることになるため．
ポーリングなら，後から取りに行くだけで取りこぼしが起きない．
体感が遅い場合は間隔を詰めるか SSE に変える．
"""

from __future__ import annotations

import logging
import os
import time

from fastapi import Body, Depends, FastAPI, HTTPException, Query, Request
from fastapi.responses import FileResponse
from fastapi.security import HTTPAuthorizationCredentials, HTTPBearer
from fastapi.staticfiles import StaticFiles
from pydantic import BaseModel

from sai_web import (
    commands,
    control,
    home,
    notify,
    questions,
    registry,
    runs,
    sessions,
    store,
    usage,
)
from sai_web.config import settings
from sai_web.watch import watch_loop
from pathlib import Path
import asyncio

logger = logging.getLogger("sai_web")

app = FastAPI(title="sai-web")

_bearer = HTTPBearer(auto_error=False)


def require_token(
    creds: HTTPAuthorizationCredentials | None = Depends(_bearer),
    token: str | None = Query(default=None),
) -> None:
    """入口の守り。`SAI_WEB_TOKEN` と照合する（未設定なら認証なし）。

    なぜクエリ（?token=）も受け付けるか: iPhone 側でヘッダを付けられない
    場面（直接URLを開く等）でも開けるようにするため．
    """
    if not settings.api_token:
        return
    if (creds is not None and creds.credentials == settings.api_token) or token == settings.api_token:
        return
    raise HTTPException(status_code=401, detail="認証エラー")


def require_internal(request: Request) -> None:
    """フック・拡張からの内部の要求の守り。Mac 自身（127.0.0.1 / ::1）からのみ。"""
    client = request.client.host if request.client else ""
    if client not in ("127.0.0.1", "::1"):
        raise HTTPException(status_code=403, detail="内部専用です")


def write_token_file() -> None:
    """合言葉（`SAI_WEB_TOKEN`）を `<sai_home>/web/token` に書く（0600）．

    エージェント側の道具 `phone_api.py` はこのファイルから合言葉を読む．
    設定ファイル（`settings.json` の `env`）ではなくファイルにするのは，
    `env` は起動コマンドの文字になって pane や記録に残るため（要件 D13）．
    合言葉が無いときはファイルを消す．書けなくてもサーバは止めず，
    警告だけログに出す（合言葉の値そのものは決して出さない）．
    """
    path = registry.sai_home() / "web" / "token"
    try:
        if not settings.api_token:
            if path.exists():
                path.unlink()
            return
        path.parent.mkdir(parents=True, exist_ok=True)
        # 既に作られたファイルの桁が 0644 でも 0600 へ戻すため chmod もする
        fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
        try:
            os.write(fd, settings.api_token.encode("utf-8"))
        finally:
            os.close(fd)
        os.chmod(path, 0o600)
    except OSError as exc:
        logger.warning("合言葉のファイル (%s) を書けません: %s", path, exc)


@app.on_event("startup")
def _startup() -> None:
    store.init_db()
    notify.ensure_keys()
    if not settings.api_token:
        # トークン無しは開発用．外に公開する前に必ず設定すること．
        logger.warning("SAI_WEB_TOKEN 未設定：認証なしで動作中")
    # 合言葉をファイルで渡す（`phone_api.py` が読む）。
    write_token_file()
    # 確かめ用のサーバは見張りを止められる（SAI_WEB_NO_WATCH=1）。
    # 見張りは headless の `state.json` を見て iPhone へ通知を送るので，動いているサーバと二重にならないように。
    if os.getenv("SAI_WEB_NO_WATCH") == "1":
        logger.warning("見張りを止めて起動しました（SAI_WEB_NO_WATCH=1）")
        return
    # 状態の監視ループを起動する（通知用．失敗してもサーバーは止めない）
    asyncio.get_event_loop().create_task(watch_loop())


def _server_version() -> str:
    """動いているサーバーのコードの版（git の commit・取れなければ "?"）。"""
    import subprocess

    try:
        out = subprocess.run(
            ["git", "rev-parse", "--short", "HEAD"],
            cwd=Path(__file__).resolve().parent,
            capture_output=True,
            text=True,
            timeout=5,
        )
        return out.stdout.strip() or "?"
    except (OSError, subprocess.SubprocessError):
        return "?"


# 起動したときの版と時刻（iPhone の版の確認画面に出す。Python を直したら再起動が要るかを見分けるため）
_SERVER_VERSION = _server_version()
_SERVER_STARTED = time.strftime("%m/%d %H:%M")


@app.get("/api/health")
def api_health() -> dict:
    return {"ok": True, "server": _SERVER_VERSION, "started_at": _SERVER_STARTED}


# --- PWA（静的ファイル）の配信 ---
# 会話データ（/api/…）より後に登録する．先に登録すると API が静的扱いになるため．
_static_dir = Path(__file__).resolve().parent / "static"


@app.get("/manifest.json", include_in_schema=False)
def serve_manifest(_: None = Depends(require_token)) -> FileResponse:
    # PWA の設定ファイル．認証付きでも iPhone が読み込めるよう個別に返す．
    return FileResponse(_static_dir / "manifest.json")


# --- sai-link の実行（02 §2.2） ---
# 旧 /api/sessions* は画面から使わなくなったため，この3本に置き換えた（02 §2.3）。


def _runs_view() -> tuple[list[dict], set[str]]:
    """一覧系エンドポイント共通の読み込み（実行ファイルと未回答質問）．"""
    return runs.load_runs(), runs.open_question_session_ids()


def _find_run(project_name: str, run_id: int) -> dict:
    """<project_name>/<run_id> の実行を探す（00 §3.7）．無ければ 404．"""
    for run in runs.load_runs():
        if run.get("project_name") == project_name and run.get("run_id") == run_id:
            return run
    raise HTTPException(status_code=404, detail="実行が見つかりません")


def _withdraw_open_questions(project_name: str, run_id: int, text: str) -> list[str]:
    """その run の未回答の質問を取り下げ，取り下げた質問の ID を返す（送った文は `text`）。

    run の書き出しが無い・未回答が無いときは何もせず空を返す。
    """
    for run in runs.load_runs():
        if run.get("project_name") != project_name or run.get("run_id") != run_id:
            continue
        sids = runs.run_session_ids(run)
        open_ids = [q["id"] for q in store.list_open_questions() if q.get("session_id") in sids]
        return [qid for qid in open_ids if questions.withdraw_question(qid, text)]
    return []


@app.get("/api/days")
def api_days(q: str | None = None, _: None = Depends(require_token)) -> dict:
    """日付ごとのまとめ（新しい日が先）．

    q を付けたときは検索結果（日付をまたいだ実行の一覧）も返す（02 §3.2）．
    """
    run_list, qids = _runs_view()
    body: dict = {"days": runs.build_days(run_list, qids)}
    if q and q.strip():
        body["results"] = runs.search_runs(run_list, q, qids)
    return body


@app.get("/api/days/{date}")
def api_day(date: str, _: None = Depends(require_token)) -> dict:
    """その日の実行一覧（プロジェクトごと，サブタスクは親の直下）．"""
    run_list, qids = _runs_view()
    return runs.build_day(run_list, date, qids)


@app.get("/api/runs/{project_name}/{run_id}")
def api_run(project_name: str, run_id: int, _: None = Depends(require_token)) -> dict:
    """1実行分（stats も含む．04 の統計画面が使う）．"""
    run = _find_run(project_name, run_id)
    _, qids = _runs_view()
    return runs.run_to_api(run, qids)


@app.get("/api/runs/{project_name}/{run_id}/items")
def api_run_items(
    project_name: str,
    run_id: int,
    after: str | None = None,
    _: None = Depends(require_token),
) -> dict:
    """会話の項目（旧 GET /api/sessions/{id} と同じ {items, next_cursor} 形）．

    読むファイルは Run の transcript_path（00 §3.7）。
    headless の実行では，生成中の文を `state.json`（共有契約 C1）から足す。
    返す形は `{items, next_cursor, live, partial, error}`（共有契約 C6）。
    """
    run = _find_run(project_name, run_id)
    agent = {"cc": "claude", "pi": "pi"}.get(run.get("runner") or "")
    if agent is None:
        # 未対応の runner
        raise HTTPException(status_code=404, detail="会話の記録が見つかりません")
    _, qids = _runs_view()
    live = runs.live_status(run, qids)
    st = runs.headless_state(run)
    partial = st["partial"] if st else ""
    error = st["error"] if st else ""
    transcript = run.get("transcript_path")
    path = Path(transcript) if transcript else None
    if path is None or not path.exists():
        # 記録の場所がまだ書き出されていない・まだ書かれていない（起動直後など）。
        # 404 にすると画面が「オフライン」になり質問カードも読まなくなるので空で返す。
        # 生成中の文（partial）はこのときも返す（逐次表示をすぐ始められるように）。
        return {"items": [], "next_cursor": "", "live": live, "partial": partial, "error": error}
    items, next_cursor = sessions.read_conversation(agent, path, after=after)
    return {
        "items": items,
        "next_cursor": next_cursor,
        "live": live,
        "partial": partial,
        "error": error,
    }


class RegisterRequest(BaseModel):
    """エージェント自身からの登録（どのセッションがどの pane か）．"""

    agent: str  # 'claude' か 'pi'
    session_id: str
    transcript_path: str
    cwd: str
    pane_id: str


@app.post("/api/register")
def api_register(req: RegisterRequest, request: Request) -> dict:
    # 内部登録は内部専用（Mac 自身だけ）．
    require_internal(request)
    if req.agent not in ("claude", "pi"):
        raise HTTPException(status_code=400, detail="agent は claude か pi を指定してください")
    store.save_registration(
        req.agent, req.session_id, req.transcript_path, req.cwd, req.pane_id
    )
    return {"ok": True}


class QuestionRegisterRequest(BaseModel):
    """フック・拡張からの質問登録．"""

    agent: str
    session_id: str
    pane_id: str
    tool_use_id: str
    questions: list[dict]


@app.post("/api/questions")
def api_register_question(req: QuestionRegisterRequest, request: Request) -> dict:
    # 内部登録は内部専用（Mac 自身だけ）
    require_internal(request)
    if req.agent not in ("claude", "pi"):
        raise HTTPException(status_code=400, detail="agent は claude か pi を指定してください")
    q = questions.register_question(
        req.agent, req.session_id, req.pane_id, req.tool_use_id, req.questions
    )
    # 登録ついでにセッション対応も覚える（pane_id と transcript は分かる範囲で）
    store.save_registration(
        req.agent, req.session_id, "", "", req.pane_id
    )
    return {"ok": True, "id": q["id"]}


@app.get("/api/questions/pending")
def api_pending_questions(_: None = Depends(require_token)) -> dict:
    return {"questions": store.list_open_questions()}


class QuestionResolvedRequest(BaseModel):
    """pi の拡張から「PC で答えた」の知らせ（05 §3.1）．空でよい．"""


@app.post("/api/questions/{qid}/resolved")
def api_question_resolved(
    qid: str, req: QuestionResolvedRequest, request: Request
) -> dict:
    # 内部専用（Mac 自身だけ）。
    require_internal(request)
    if not questions.resolve_on_terminal(qid):
        # 既に回答済み・解決済みでもエラーにはしない（二重通知対策）
        return {"ok": True, "changed": False}
    return {"ok": True, "changed": True}


@app.get("/api/push/key")
def api_push_key(_: None = Depends(require_token)) -> dict:
    """iPhone に配る公開鍵．"""
    key = notify.get_public_key()
    if key is None:
        key = notify.ensure_keys()
    return {"public_key": key}


class PushSubscription(BaseModel):
    """iPhone の通知登録情報（ブラウザが作る物そのまま）．"""

    endpoint: str
    keys: dict


@app.post("/api/push/subscribe")
def api_push_subscribe(sub: PushSubscription, _: None = Depends(require_token)) -> dict:
    info = {"endpoint": sub.endpoint, "keys": sub.keys}
    store.save_subscription(sub.endpoint, info)
    # 登録できたらその場で 1 通送る（届くかどうかを iPhone ですぐ確かめられるように）
    result = notify.send_to(info, "sai", "通知を受け取る設定ができました", "/#/")
    return {"ok": True, "test_sent": result == "ok"}


@app.get("/api/questions/{qid}")
def api_get_question(qid: str, _: None = Depends(require_internal)) -> dict:
    # 拡張機能からのポーリング用．内部専用（Mac 自身だけ）。
    # iPhone 側は /api/questions?session_id=… を使う．
    q = store.get_question(qid)
    if q is None:
        raise HTTPException(status_code=404, detail="質問が見つかりません")
    return q


@app.get("/api/questions")
def api_session_questions(
    session_id: str, _: None = Depends(require_token)
) -> dict:
    # 質問の登録（POST /api/questions）は必ず登録情報も一緒に保存するので，
    # 登録情報の pane_id だけで対応づける（全 jsonl 走査はやめた・02 §2.3）
    reg = store.get_registration(session_id)
    if reg is None:
        raise HTTPException(status_code=404, detail="セッションが見つかりません")
    questions.expire_stale_questions()
    questions.refresh_question_state(
        session_id, reg.get("pane_id") or None, reg.get("agent")
    )
    # open / answered / resolved_elsewhere を画面に出す（05 §1 の2番）:
    # 回答済みは「回答済み: …」の1行，PC で答えた分は「PC で回答済み」の1行に
    # 変わるため，消さずに返す．expired は返さない．
    return {"questions": store.list_questions_for_session(session_id)}


class AnswerItem(BaseModel):
    """1問分の回答．選択肢の番号か自由文のどちらか．

    choice_indices は複数選択のときの番号の配列（05 §1 の2番）．
    """

    choice_index: int | None = None
    choice_indices: list[int] | None = None
    text: str | None = None


class AnswerRequest(BaseModel):
    """質問への回答．

    全問分（answers）を受け取るのが新しい形．
    旧形式（choice_index / text を直接持つ，1問分のみ）も後方互換で受け付ける．
    """

    answers: list[AnswerItem] | None = None
    # 旧形式（1問だけの回答）．answers が無い場合はこちらを使う．
    choice_index: int | None = None
    text: str | None = None


@app.post("/api/questions/{qid}/answer")
def api_answer_question(
    qid: str, req: AnswerRequest, _: None = Depends(require_token)
) -> dict:
    if req.answers is not None:
        answers = [a.model_dump() for a in req.answers]
    else:
        # 旧形式：1問分の回答を配列に包み直す
        answers = [{"choice_index": req.choice_index, "text": req.text}]
    ok, reason = questions.answer_question(qid, answers)
    if not ok:
        raise HTTPException(status_code=400, detail=reason)
    return {"ok": True}


class SendRequest(BaseModel):
    """テキスト送信．"""

    text: str


@app.post("/api/runs/{project_name}/{run_id}/send")
def api_send_text(
    project_name: str,
    run_id: int,
    req: SendRequest,
    _: None = Depends(require_token),
) -> dict:
    """テキスト送信（`sai-link run send <run_id> --json`・共有契約 C2）．

    本文は標準入力で渡す。失敗の読み替えは `cli.run_json` に任せる
    （CLI の 4 → 409・3 → 404）。
    """
    text = (req.text or "").strip()
    if not text:
        raise HTTPException(status_code=400, detail="文章が空です")
    # 質問が開いているときは tmux に送らず，質問を取り下げて文を答えの代わりに渡す
    # （フックが待っている間は cc の入力欄の文がエージェントに届かないため）
    withdrawn = _withdraw_open_questions(project_name, run_id, text)
    if withdrawn:
        return {"run_id": run_id, "status": "withdrawn", "question_ids": withdrawn}
    return control.run_send(project_name, run_id, text)


@app.post("/api/runs/{project_name}/{run_id}/interrupt")
def api_interrupt(
    project_name: str,
    run_id: int,
    _: None = Depends(require_token),
) -> dict:
    """実行中の run を中断する（`sai-link run interrupt <run_id> --json`・C2）．

    無い実行では CLI を呼ばずに 404 を返す。
    質問が開いているときは esc を送らず，質問を文なしで取り下げる（esc はフックごと止めて
    質問が開いたまま残るため）。
    """
    _find_run(project_name, run_id)  # 無い run は 404（CLI を呼ばない）
    if _withdraw_open_questions(project_name, run_id, ""):
        return {"run_id": run_id, "status": "interrupted", "agent_status": "blocked"}
    return control.run_interrupt(project_name, run_id)


class WithdrawRequest(BaseModel):
    """質問の取り下げ（text は送った文・省略すると空）。"""

    text: str | None = None


@app.post("/api/questions/{qid}/withdraw")
def api_withdraw_question(
    qid: str, req: WithdrawRequest | None = None, _: None = Depends(require_token)
) -> dict:
    """未回答の質問を取り下げる（質問カードの「質問を止める」・U03）。

    open でなければ 409。文は 8000 文字まで。
    """
    text = (req.text or "") if req else ""
    if len(text) > 8000:
        raise HTTPException(status_code=400, detail="文は 8000 文字までです")
    if store.get_question(qid) is None:
        raise HTTPException(status_code=404, detail="質問が見つかりません")
    if not questions.withdraw_question(qid, text):
        raise HTTPException(status_code=409, detail="この質問はもう締め切られています")
    return {"ok": True}


@app.post("/api/runs/{project_name}/{run_id}/finalize")
def api_run_finalize(
    project_name: str, run_id: int, _: None = Depends(require_token)
) -> dict:
    """メッセージ画面の ■ 終了（`sai-link phone finalize <run_id>`・計画書 A の D2）。

    完了の処理そのもの（DB 保存・成果物のまとめ・pane を閉じる）は sai-link の
    デーモンが行う。ここは要求を出すだけ。
    """
    _find_run(project_name, run_id)  # 無い run は 404（CLI を呼ばない）
    return control.phone_finalize(project_name, run_id)


# --- Project タブ（計画 06） ---
# 外部 CLI（sai-link）を呼ぶため，これらは `def` で書く
# （FastAPI が別スレッドで動かし，CLI を待つ間サーバー全体を止めない・06 §3.4）。


class DaemonRequest(BaseModel):
    """常駐の操作．action は start / stop．"""

    action: str


@app.get("/api/projects")
def api_projects(_: None = Depends(require_token)) -> dict:
    """登録簿のプロジェクト一覧（最後に動いた順）．"""
    projects = control.list_projects()
    return {"projects": projects}


@app.get("/api/projects/{name}/dashboard")
def api_project_dashboard(
    name: str, period: str = "week", _: None = Depends(require_token)
) -> dict:
    """カード型 Dashboard（period は week / month / all・既定 week）．"""
    return control.dashboard(name, period)


@app.post("/api/projects/{name}/daemon")
def api_project_daemon(
    name: str, req: DaemonRequest, _: None = Depends(require_token)
) -> dict:
    """常駐の起動・停止．"""
    return control.daemon(name, req.action)


# --- List 画面（計画 14 §3.3・`sai-link phone …` 経由） ---
# 外から来る値（run_id / view / kind / token）は control 側で確かめる（計画 14 §3.3）。
# CLI の呼び出しは `def` で書く（外部 CLI を待つ間サーバー全体を止めない・06 §3.4）。固有の
# 見出し（tree / runs / …）なので，他の `/api/projects/{name}/…` と住所が衝突しない。
# 旧 `GET …/todo` と `POST …/run`（todo 一覧とカード実行）は、
# 新形式 todo（tree の todo フォルダと `POST …/runs/{run_id}/start`）に置き換えた（計画 15）。


@app.get("/api/projects/{name}/tree")
def api_project_tree(name: str, _: None = Depends(require_token)) -> dict:
    """引き出しツリー（`phone tree`・計画 12 §3.2）。"""
    return control.phone_tree(name)


@app.get("/api/projects/{name}/search")
def api_project_search(
    name: str,
    q: str = "",
    category: str = "",
    since: str | None = None,
    until: str | None = None,
    block: str | None = None,
    from_id: int | None = None,
    to_id: int | None = None,
    limit: int = 50,
    _: None = Depends(require_token),
) -> dict:
    """文書（task / message / waggle）の検索（`phone search`・会話検索の画面の文書の検索）。

    `q` は空白区切りの AND。`category` はカンマ区切り（空なら全部）。`since` / `until` は
    `YYYY-MM-DD`、`block` は完全一致、`from_id` / `to_id` は run_id の範囲。
    """
    return control.phone_search(name, q, category, since, until, block, from_id, to_id, limit)


@app.get("/api/projects/{name}/runs/{run_id:int}/view/{view}")
def api_project_run_view(
    name: str, run_id: int, view: str, _: None = Depends(require_token)
) -> dict:
    """run の中身（Log / Stats / History・計画 12 §3.2）。"""
    return control.phone_run_view(name, run_id, view)


@app.get("/api/projects/{name}/runs/{run_id:int}/history/calls/{call_id}")
def api_project_run_history_call(
    name: str, run_id: int, call_id: str, _: None = Depends(require_token)
) -> dict:
    """History の 1 呼び出しの全文（`sai-link run history <id> --call <call_id>`・C11）。"""
    return control.phone_run_call(name, run_id, call_id)


@app.get("/api/projects/{name}/runs/{run_id:int}/md/{kind}")
def api_project_run_md(
    name: str, run_id: int, kind: str, _: None = Depends(require_token)
) -> dict:
    """run の md（task / message / report / system・無いときは 404・計画 12 §3.2）。"""
    return control.phone_md(name, run_id, kind)


@app.put("/api/projects/{name}/runs/{run_id:int}/md/{kind}")
def api_project_run_md_save(
    name: str,
    run_id: int,
    kind: str,
    payload: dict = Body(...),
    _: None = Depends(require_token),
) -> dict:
    """task（未実行の todo カードだけ）の保存（body は `{"text": …}`）。"""
    return control.phone_md_save(name, run_id, kind, payload.get("text"))


@app.get("/api/projects/{name}/runs/{run_id:int}/waggle")
def api_project_run_waggle(
    name: str, run_id: int, _: None = Depends(require_token)
) -> dict:
    """run が保存した成果物の一覧（計画 12 §3.2）。"""
    return control.phone_waggle(name, run_id)


@app.get("/api/projects/{name}/runs/{run_id:int}/waggle/{waggle_token}")
def api_project_run_waggle_item(
    name: str, run_id: int, waggle_token: str, _: None = Depends(require_token)
) -> dict:
    """成果物 1 件の本文（計画 12 §3.2）．

    住所の名前は計画 12 §3.3 の `{token}` と同じ形（URL のセグメント 1 つ）だが，
    変数名は `waggle_token`。`token` は合言葉の `?token=`（require_token の Query）と
    衝突し，FastAPI が path パラメータと解釈できなくなるため。
    """
    return control.phone_waggle(name, run_id, waggle_token)


# --- task.md 作成フォーム（計画 15 §1.5・§3.3・`sai-link phone card …` 経由） ---
# 外から来る値（block / 送る JSON の中身）は control 側で CLI を呼ぶ前に確かめる。固有の
# 見出し（card）なので，他の `/api/projects/{name}/…` と住所が衝突しない。


@app.get("/api/projects/{name}/card/options")
def api_project_card_options(
    name: str, _: None = Depends(require_token)
) -> dict:
    """作成フォームの選択肢（done / runner / block / provider / model / thinking）。"""
    return control.phone_card_options(name)


@app.post("/api/projects/{name}/card")
def api_project_card(
    name: str, payload: dict = Body(...), _: None = Depends(require_token)
) -> dict:
    """カード（task.md）の作成（`phone card create`・作成後は todo に配置）。"""
    return control.phone_card_create(name, payload)


@app.post("/api/projects/{name}/runs/{run_id:int}/start")
def api_project_run_start(
    name: str, run_id: int, _: None = Depends(require_token)
) -> dict:
    """新形式 todo の ▶ 実行（`phone card run <run_id>`・計画 15）。"""
    return control.phone_card_start(name, run_id)


@app.post("/api/projects/{name}/runs/{run_id:int}/advance")
def api_project_run_advance(
    name: str, run_id: int, _: None = Depends(require_token)
) -> dict:
    """review → done → Workflow の移動（`phone advance <run_id>`・計画書 A の U10）。

    どちらへ移すか（done か R<gr> か）は sai-link が見て決める。
    """
    return control.phone_advance(name, run_id)


@app.delete("/api/projects/{name}/runs/{run_id:int}")
def api_project_run_delete(
    name: str, run_id: int, _: None = Depends(require_token)
) -> dict:
    """List の項目を消す（`phone rm <run_id>`・子の実行と run フォルダも消える）。"""
    return control.phone_rm(name, run_id)


@app.post("/api/projects/{name}/runs/{run_id:int}/schedule")
def api_project_run_schedule(
    name: str, run_id: int, payload: dict = Body(...), _: None = Depends(require_token)
) -> dict:
    """todo の実行予定時刻（`{"at": "YYYY-MM-DD HH:MM"}`・空で取り消し）。"""
    at = payload.get("at") if isinstance(payload, dict) else None
    if at is not None and not isinstance(at, str):
        raise HTTPException(status_code=400, detail="時刻の形が正しくありません")
    return control.phone_card_schedule(name, run_id, at or "")


@app.get("/api/projects/{name}/card/docs")
def api_project_card_docs(
    name: str, _: None = Depends(require_token)
) -> dict:
    """最近の文書（task_text に `waggle:<tok>:path:<path>` を貼る候補）。"""
    return control.phone_card_docs(name)


# --- Web の「コマンド」画面（U10） ---
# sai-link の schema から入力欄を作り，命令を実行する。
# 外から来る値は CLI を呼ぶ前に commands 側で確かめる（通らなければ 400）。
# `bytes` の命令（ファイルを送る物）は Web には出さず，実行も断る。


@app.get("/api/projects/{name}/commands")
def api_project_commands(name: str, _: None = Depends(require_token)) -> dict:
    """Web に出せる命令の一覧（`web: true` だけ・sci が無ければ外れる）。"""
    return commands.list_commands(name)


@app.post("/api/projects/{name}/commands/run")
def api_project_command_run(
    name: str, payload: dict = Body(...), _: None = Depends(require_token)
) -> dict:
    """命令を実行する（共有契約「コマンドの実行の口」）。"""
    return commands.run_command(name, payload)


# --- 会話の検索（U10） ---
# `sai-link session search`（エージェントの会話の履歴）を Web から使う。
# 検査は control.phone_session_search が CLI を呼ぶ前に全部行う。


@app.get("/api/projects/{name}/sessions/search")
def api_project_session_search(
    name: str,
    word: str | None = Query(default=None),
    global_round: str | None = Query(default=None),
    round: str | None = Query(default=None),
    id: str | None = Query(default=None),
    block: str | None = Query(default=None),
    date: str | None = Query(default=None),
    status: str | None = Query(default=None),
    type: str | None = Query(default=None),
    _: None = Depends(require_token),
) -> dict:
    """会話の検索（先頭 200 件・`{"items", "truncated"}`）。"""
    return control.phone_session_search(
        name,
        word=word,
        global_round=global_round,
        round=round,
        run_id=id,
        block=block,
        on_date=date,
        status=status,
        sess_type=type,
    )


# --- ホーム（U3 §変更内容 3） ---
# 答えを待っている質問・確認を待っている承認・動いているプロジェクト・論文の要対応を
# 1 本の住所で集める（画面から全部を別々に呼ぶと回数が多くなるため）。
# 集める本体は `home.py`（外部 CLI を待つので `def` で書く・06 §3.4）。 


# --- Workflow と定義の編集（U3 §変更内容 1・共有契約 C-A1） ---
# sai-link-tui の Workflow 盤面と block / template / settings の編集を Web から行う。
# 外から来る値（kind / scope / 名前 / 本文）は control 側で CLI を呼ぶ前に確かめる。


@app.get("/api/projects/{name}/workflow")
def api_project_workflow(name: str, _: None = Depends(require_token)) -> dict:
    """Workflow 盤面（`sai-link phone workflow`）。"""
    return control.phone_workflow(name)


@app.get("/api/projects/{name}/defs")
def api_project_defs(name: str, _: None = Depends(require_token)) -> dict:
    """定義の一覧（`phone def list`・U3 C-A1）。"""
    return control.phone_def_list(name)


@app.get("/api/projects/{name}/defs/{kind}/{scope}/{def_name}")
def api_project_def_show(
    name: str, kind: str, scope: str, def_name: str, _: None = Depends(require_token)
) -> dict:
    """定義 1 件の中身（`phone def show`）。

    住所の `{def_name}` は変数名も `def_name` にする（`token` は合言葉の
    `?token=` と衝突し、FastAPI が path パラメータと解釈できなくなるため・main.py のコメント）。
    """
    return control.phone_def_show(name, kind, scope, def_name)


@app.put("/api/projects/{name}/defs/{kind}/{scope}/{def_name}")
def api_project_def_write(
    name: str,
    kind: str,
    scope: str,
    def_name: str,
    payload: dict = Body(...),
    _: None = Depends(require_token),
) -> dict:
    """定義の保存（body は `{"text": …, "create": false}`・`create` が真なら `--create`）。"""
    return control.phone_def_write(
        name, kind, scope, def_name, payload.get("text"), bool(payload.get("create"))
    )


@app.delete("/api/projects/{name}/defs/{kind}/{scope}/{def_name}")
def api_project_def_delete(
    name: str, kind: str, scope: str, def_name: str, _: None = Depends(require_token)
) -> dict:
    """定義の削除（`phone def delete`・settings は 400「settings は消せません」）。"""
    return control.phone_def_delete(name, kind, scope, def_name)


@app.get("/api/home")
def api_home(_: None = Depends(require_token)) -> dict:
    """ホームの集計（質問・承認・プロジェクト・論文の要対応・U3 C-A3）。"""
    return home.build_home()


@app.get("/api/home/overview")
def api_home_overview(period: str = "week", _: None = Depends(require_token)) -> dict:
    """全プロジェクトの合計のダッシュボード（U02 §1・`period` は week / month / all）。

    `/api/home` とは別の住所にしてある。`/api/home` は 15 秒ごとに読み直すので、
    期間を変えて読み直すものと混ぜない（画面の「今週 / 今月 / 全期間」の切り替え）。
    CLI（各プロジェクトの `phone dashboard`）を待つので `def` で書く。
    """
    return home.build_overview(period)


# --- Usage タブ（計画 42 §3.3） ---
# 外部 API（OpenRouter / DeepSeek）へ行くため `def` で書く（06 §3.4）。
# 外から来る provider / account は設定にある名前だけ受け付け，それ以外は 404。


@app.get("/api/usage")
def api_usage(refresh: int = 0, _: None = Depends(require_token)) -> dict:
    """Usage タブのカード一覧（cc / OpenRouter 各アカウント / DeepSeek / 設定に無い provider）．

    `?refresh=1` で外部 API を取り直す（↻ ボタン・42 §3.5）。
    """
    return usage.summary(refresh=bool(refresh))


@app.get("/api/usage/{provider}/{account}/activity")
def api_usage_activity(
    provider: str, account: str, _: None = Depends(require_token)
) -> dict:
    """日ごと（30 日）とモデル別の集計（provider は openrouter / deepseek・42 §3.3）．"""
    if not usage.known_target(provider, account):
        raise HTTPException(status_code=404, detail="そのアカウントはありません")
    return usage.activity(provider, account)


@app.get("/api/usage/{provider}/{account}/logs")
def api_usage_logs(
    provider: str,
    account: str,
    offset: int = Query(0, ge=0),
    limit: int = Query(50, ge=1, le=200),
    _: None = Depends(require_token),
) -> dict:
    """1 回ずつの一覧（新しい順・50 件ずつ・42 §3.3）．"""
    if not usage.known_target(provider, account):
        raise HTTPException(status_code=404, detail="そのアカウントはありません")
    return usage.logs(provider, account, offset=offset, limit=limit)


# --- PWA（静的ファイル）の配信 ---
# API の後に置く．認証は require_token で同じく守る．
# 会話データ（/api/…）はキャッシュしない（sw.js 側で通すだけ）．
app.mount("/", StaticFiles(directory=_static_dir, html=True), name="static")


@app.middleware("http")
async def _revalidate_static(request: Request, call_next):
    """画面ファイルは毎回「新しいか」を確かめさせる（Cache-Control: no-cache）．

    指定が無いと iPhone は最終更新からの経過時間で勝手に「まだ新しい」とみなし，
    古い app.js / sw.js を使い続ける（直した画面が届かない）．
    中身が同じなら 304 が返るだけなので通信はほとんど増えない．
    """
    response = await call_next(request)
    if not request.url.path.startswith("/api/"):
        response.headers.setdefault("Cache-Control", "no-cache")
    return response


@app.middleware("http")
async def _frame_guard(request: Request, call_next):
    """sai-web を枠（iframe）に入れさせない（要件定義書 §5）。

    ts のオリジンの JS が sai-web を枠に入れて、利用者に押させる悪用を防ぐ。
    既に付いている値は上書きしない。
    """
    response = await call_next(request)
    response.headers.setdefault("X-Frame-Options", "DENY")
    response.headers.setdefault("Content-Security-Policy", "frame-ancestors 'none'")
    return response
