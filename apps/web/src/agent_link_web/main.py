"""agent-link-web 本体サーバー (Mac 側)．

- agent-link が書き出した実行の一覧（日付ごと・1日ごと・1実行）
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
import time

from fastapi import Body, Depends, FastAPI, HTTPException, Query, Request
from fastapi.exceptions import RequestValidationError
from fastapi.responses import FileResponse, JSONResponse
from fastapi.staticfiles import StaticFiles
from pydantic import BaseModel

from agent_link_web import (
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
from agent_link_web.config import settings
from agent_link_web.watch import watch_loop
from pathlib import Path
import asyncio

logger = logging.getLogger("agent_link_web")

app = FastAPI(title="agent-link-web")


@app.exception_handler(RequestValidationError)
async def validation_error_handler(request: Request, exc: RequestValidationError):
    """検査の誤り（422）の detail を、配列ではなく英語 1 文にする（docs/integration.md の形）。"""
    first = exc.errors()[0]
    where = ".".join(str(p) for p in first["loc"])
    return JSONResponse(
        status_code=422, content={"detail": f"Invalid request: {where}: {first['msg']}"}
    )

def require_internal(request: Request) -> None:
    """フック・拡張からの内部の要求の守り。Mac 自身（127.0.0.1 / ::1）からのみ。"""
    client = request.client.host if request.client else ""
    if client not in ("127.0.0.1", "::1"):
        raise HTTPException(status_code=403, detail="Internal use only.")


@app.on_event("startup")
def _startup() -> None:
    store.init_db()
    notify.ensure_keys()
    # 確かめ用のサーバは見張りを止められる（AGENT_LINK_WEB_NO_WATCH=1）。
    # 見張りは headless の `state.json` を見て iPhone へ通知を送るので，動いているサーバと二重にならないように。
    if registry.config_var("AGENT_LINK_WEB_NO_WATCH") == "1":
        logger.warning("見張りを止めて起動しました（AGENT_LINK_WEB_NO_WATCH=1）")
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
def serve_manifest() -> FileResponse:
    # PWA の設定ファイル．iPhone が読み込めるよう個別に返す．
    return FileResponse(_static_dir / "manifest.json")


# --- agent-link の実行 ---
# 旧 /api/sessions* は画面から使わなくなったため，この3本に置き換えた。


def _runs_view() -> tuple[list[dict], set[str]]:
    """一覧系エンドポイント共通の読み込み（実行ファイルと未回答質問）．"""
    return runs.load_runs(), runs.open_question_session_ids()


def _find_run(project_name: str, run_id: int) -> dict:
    """<project_name>/<run_id> の実行を探す．無ければ 404．"""
    for run in runs.load_runs():
        if run.get("project_name") == project_name and run.get("run_id") == run_id:
            return run
    raise HTTPException(status_code=404, detail="Run not found.")


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
def api_days(q: str | None = None) -> dict:
    """日付ごとのまとめ（新しい日が先）．

    q を付けたときは検索結果（日付をまたいだ実行の一覧）も返す．
    """
    run_list, qids = _runs_view()
    body: dict = {"days": runs.build_days(run_list, qids)}
    if q and q.strip():
        body["results"] = runs.search_runs(run_list, q, qids)
    return body


@app.get("/api/days/{date}")
def api_day(date: str) -> dict:
    """その日の実行一覧（プロジェクトごと，サブタスクは親の直下）．"""
    run_list, qids = _runs_view()
    return runs.build_day(run_list, date, qids)


@app.get("/api/runs/{project_name}/{run_id}")
def api_run(project_name: str, run_id: int) -> dict:
    """1実行分（stats も含む．04 の統計画面が使う）．"""
    run = _find_run(project_name, run_id)
    _, qids = _runs_view()
    return runs.run_to_api(run, qids)


@app.get("/api/runs/{project_name}/{run_id}/items")
def api_run_items(
    project_name: str,
    run_id: int,
    after: str | None = None,
) -> dict:
    """会話の項目（旧 GET /api/sessions/{id} と同じ {items, next_cursor} 形）．

    読むファイルは Run の transcript_path。
    headless の実行では，生成中の文を `state.json`から足す。
    返す形は `{items, next_cursor, live, partial, error}`。
    """
    run = _find_run(project_name, run_id)
    agent = "claude"
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
        raise HTTPException(status_code=400, detail="agent must be claude or pi.")
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
        raise HTTPException(status_code=400, detail="agent must be claude or pi.")
    q = questions.register_question(
        req.agent, req.session_id, req.pane_id, req.tool_use_id, req.questions
    )
    # 登録ついでにセッション対応も覚える（pane_id と transcript は分かる範囲で）
    store.save_registration(
        req.agent, req.session_id, "", "", req.pane_id
    )
    return {"ok": True, "id": q["id"]}


@app.get("/api/questions/pending")
def api_pending_questions() -> dict:
    return {"questions": store.list_open_questions()}


class QuestionResolvedRequest(BaseModel):
    """pi の拡張から「PC で答えた」の知らせ．空でよい．"""


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
def api_push_key() -> dict:
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
def api_push_subscribe(sub: PushSubscription) -> dict:
    info = {"endpoint": sub.endpoint, "keys": sub.keys}
    store.save_subscription(sub.endpoint, info)
    # 登録できたらその場で 1 通送る（届くかどうかを iPhone ですぐ確かめられるように）
    result = notify.send_to(info, "agent-link","Notifications are set up.", "/#/")
    return {"ok": True, "test_sent": result == "ok"}


@app.get("/api/questions/{qid}")
def api_get_question(qid: str, _: None = Depends(require_internal)) -> dict:
    # 拡張機能からのポーリング用．内部専用（Mac 自身だけ）。
    # iPhone 側は /api/questions?session_id=… を使う．
    q = store.get_question(qid)
    if q is None:
        raise HTTPException(status_code=404, detail="Question not found.")
    return q


@app.get("/api/questions")
def api_session_questions(
    session_id: str
) -> dict:
    # 質問の登録（POST /api/questions）は必ず登録情報も一緒に保存するので，
    # 登録情報の pane_id だけで対応づける（全 jsonl 走査はやめた）
    reg = store.get_registration(session_id)
    if reg is None:
        raise HTTPException(status_code=404, detail="Session not found.")
    questions.expire_stale_questions()
    questions.refresh_question_state(
        session_id, reg.get("pane_id") or None, reg.get("agent")
    )
    # open / answered / resolved_elsewhere を画面に出す:
    # 回答済みは「回答済み: …」の1行，PC で答えた分は「PC で回答済み」の1行に
    # 変わるため，消さずに返す．expired は返さない．
    return {"questions": store.list_questions_for_session(session_id)}


class AnswerItem(BaseModel):
    """1問分の回答．選択肢の番号か自由文のどちらか．

    choice_indices は複数選択のときの番号の配列．
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
    qid: str, req: AnswerRequest
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
) -> dict:
    """テキスト送信（`agent-link run send <run_id> --json`）．

    本文は標準入力で渡す。失敗の読み替えは `cli.run_json` に任せる
    （CLI の 4 → 409・3 → 404）。
    """
    text = (req.text or "").strip()
    if not text:
        raise HTTPException(status_code=400, detail="The message is empty.")
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
) -> dict:
    """実行中の run を中断する（`agent-link run interrupt <run_id> --json`）．

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
    qid: str, req: WithdrawRequest | None = None
) -> dict:
    """未回答の質問を取り下げる（質問カードの「質問を止める」）。

    open でなければ 409。文は 8000 文字まで。
    """
    text = (req.text or "") if req else ""
    if len(text) > 8000:
        raise HTTPException(status_code=400, detail="The message must be at most 8000 characters.")
    if store.get_question(qid) is None:
        raise HTTPException(status_code=404, detail="Question not found.")
    if not questions.withdraw_question(qid, text):
        raise HTTPException(status_code=409, detail="This question is already closed.")
    return {"ok": True}


@app.post("/api/runs/{project_name}/{run_id}/finalize")
def api_run_finalize(
    project_name: str, run_id: int
) -> dict:
    """メッセージ画面の ■ 終了（`agent-link run finalize <run_id>`）。

    完了の処理そのもの（DB 保存・成果物のまとめ・pane を閉じる）は agent-link の
    デーモンが行う。ここは要求を出すだけ。
    """
    _find_run(project_name, run_id)  # 無い run は 404（CLI を呼ばない）
    return control.cli_finalize(project_name, run_id)


# --- Project タブ ---
# 外部 CLI（agent-link）を呼ぶため，これらは `def` で書く
# （FastAPI が別スレッドで動かし，CLI を待つ間サーバー全体を止めない）。


class DaemonRequest(BaseModel):
    """常駐の操作．action は start / stop．"""

    action: str


@app.get("/api/projects")
def api_projects() -> dict:
    """登録簿のプロジェクト一覧（最後に動いた順）．"""
    projects = control.list_projects()
    return {"projects": projects}


@app.get("/api/projects/{name}/dashboard")
def api_project_dashboard(
    name: str, period: str = "week"
) -> dict:
    """カード型 Dashboard（period は week / month / all・既定 week）．"""
    return control.dashboard(name, period)


@app.post("/api/projects/{name}/daemon")
def api_project_daemon(
    name: str, req: DaemonRequest
) -> dict:
    """常駐の起動・停止．"""
    return control.daemon(name, req.action)


# --- List 画面（`agent-link …` 経由） ---
# 外から来る値（run_id / view / kind / token）は control 側で確かめる。
# CLI の呼び出しは `def` で書く（外部 CLI を待つ間サーバー全体を止めない）。固有の
# 見出し（tree / runs / …）なので，他の `/api/projects/{name}/…` と住所が衝突しない。
# 旧 `GET …/todo` と `POST …/run`（todo 一覧とカード実行）は、
# 新形式 todo（tree の todo フォルダと `POST …/runs/{run_id}/start`）に置き換えた。


@app.get("/api/projects/{name}/tree")
def api_project_tree(name: str) -> dict:
    """引き出しツリー（`run tree`）。"""
    return control.cli_tree(name)


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
) -> dict:
    """文書（task / message / waggle）の検索（`doc search`・会話検索の画面の文書の検索）。

    `q` は空白区切りの AND。`category` はカンマ区切り（空なら全部）。`since` / `until` は
    `YYYY-MM-DD`、`block` は完全一致、`from_id` / `to_id` は run_id の範囲。
    """
    return control.cli_search(name, q, category, since, until, block, from_id, to_id, limit)


@app.get("/api/projects/{name}/runs/{run_id:int}/view/{view}")
def api_project_run_view(
    name: str, run_id: int, view: str
) -> dict:
    """run の中身（Log / Stats / History）。"""
    return control.cli_run_view(name, run_id, view)


@app.get("/api/projects/{name}/runs/{run_id:int}/history/calls/{call_id}")
def api_project_run_history_call(
    name: str, run_id: int, call_id: str
) -> dict:
    """History の 1 呼び出しの全文（`agent-link run history <id> --call <call_id>`）。"""
    return control.cli_run_call(name, run_id, call_id)


@app.get("/api/projects/{name}/runs/{run_id:int}/md/{kind}")
def api_project_run_md(
    name: str, run_id: int, kind: str
) -> dict:
    """run の md（task / message / report / system・無いときは 404）。"""
    return control.cli_md(name, run_id, kind)


@app.put("/api/projects/{name}/runs/{run_id:int}/md/{kind}")
def api_project_run_md_save(
    name: str,
    run_id: int,
    kind: str,
    payload: dict = Body(...),
) -> dict:
    """task（未実行の todo カードだけ）の保存（body は `{"text": …}`）。"""
    return control.cli_md_save(name, run_id, kind, payload.get("text"))


@app.get("/api/projects/{name}/runs/{run_id:int}/waggle")
def api_project_run_waggle(
    name: str, run_id: int
) -> dict:
    """run が保存した成果物の一覧。"""
    return control.cli_waggle(name, run_id)


@app.get("/api/projects/{name}/runs/{run_id:int}/waggle/{waggle_token}")
def api_project_run_waggle_item(
    name: str, run_id: int, waggle_token: str
) -> dict:
    """成果物 1 件の本文．

    住所の名前は他の `{token}` と同じ形（URL のセグメント 1 つ）だが，
    変数名は `waggle_token`（`token` という名前のクエリと混ざらないように）。
    """
    return control.cli_waggle(name, run_id, waggle_token)


# --- task.md 作成フォーム（`agent-link card …` 経由） ---
# 外から来る値（block / 送る JSON の中身）は control 側で CLI を呼ぶ前に確かめる。固有の
# 見出し（card）なので，他の `/api/projects/{name}/…` と住所が衝突しない。


@app.get("/api/projects/{name}/card/options")
def api_project_card_options(
    name: str
) -> dict:
    """作成フォームの選択肢（block / model / thinking）。"""
    return control.cli_card_options(name)


@app.post("/api/projects/{name}/card")
def api_project_card(
    name: str, payload: dict = Body(...)
) -> dict:
    """カード（task.md）の作成（`card create`・作成後は todo に配置）。"""
    return control.cli_card_create(name, payload)


@app.post("/api/projects/{name}/runs/{run_id:int}/start")
def api_project_run_start(
    name: str, run_id: int
) -> dict:
    """新形式 todo の ▶ 実行（`card run <run_id>`）。"""
    return control.cli_card_start(name, run_id)


@app.post("/api/projects/{name}/runs/{run_id:int}/advance")
def api_project_run_advance(
    name: str, run_id: int
) -> dict:
    """review → Workflow（日付フォルダ）の移動（`run advance <run_id>`）。"""
    return control.cli_advance(name, run_id)


@app.delete("/api/projects/{name}/runs/{run_id:int}")
def api_project_run_delete(
    name: str, run_id: int
) -> dict:
    """List の項目を消す（`run rm <run_id>`・子の実行と run フォルダも消える）。"""
    return control.cli_rm(name, run_id)


@app.post("/api/projects/{name}/runs/{run_id:int}/schedule")
def api_project_run_schedule(
    name: str, run_id: int, payload: dict = Body(...)
) -> dict:
    """todo の実行予定時刻（`{"at": "YYYY-MM-DD HH:MM"}`・空で取り消し）。"""
    at = payload.get("at") if isinstance(payload, dict) else None
    if at is not None and not isinstance(at, str):
        raise HTTPException(status_code=400, detail="The time format is invalid.")
    return control.cli_card_schedule(name, run_id, at or "")


@app.get("/api/projects/{name}/card/docs")
def api_project_card_docs(
    name: str
) -> dict:
    """最近の文書（task_text に `waggle:<tok>:path:<path>` を貼る候補）。"""
    return control.cli_card_docs(name)


# --- 会話の検索 ---
# `agent-link session search`（エージェントの会話の履歴）を Web から使う。
# 検査は control.cli_session_search が CLI を呼ぶ前に全部行う。


@app.get("/api/projects/{name}/sessions/search")
def api_project_session_search(
    name: str,
    word: str | None = Query(default=None),
    id: str | None = Query(default=None),
    block: str | None = Query(default=None),
    date: str | None = Query(default=None),
    status: str | None = Query(default=None),
    type: str | None = Query(default=None),
) -> dict:
    """会話の検索（先頭 200 件・`{"items", "truncated"}`）。"""
    return control.cli_session_search(
        name,
        word=word,
        run_id=id,
        block=block,
        on_date=date,
        status=status,
        sess_type=type,
    )


# --- ホーム ---
# 答えを待っている質問・確認を待っている承認・動いているプロジェクト・論文の要対応を
# 1 本の住所で集める（画面から全部を別々に呼ぶと回数が多くなるため）。
# 集める本体は `home.py`（外部 CLI を待つので `def` で書く）。 


# --- Workflow と定義の編集 ---
# Workflow の盤面と block / template / settings の編集を Web から行う。
# 外から来る値（kind / scope / 名前 / 本文）は control 側で CLI を呼ぶ前に確かめる。


@app.get("/api/projects/{name}/workflow")
def api_project_workflow(name: str, subtasks: bool = Query(default=False)) -> dict:
    """Workflow 盤面（`agent-link workflow`・`?subtasks=1` で子の実行も含める）。"""
    return control.cli_workflow(name, subtasks)


@app.get("/api/projects/{name}/defs")
def api_project_defs(name: str) -> dict:
    """定義の一覧（`def list`）。"""
    return control.cli_def_list(name)


@app.get("/api/projects/{name}/defs/{kind}/{scope}/{def_name}")
def api_project_def_show(
    name: str, kind: str, scope: str, def_name: str
) -> dict:
    """定義 1 件の中身（`def show`）。

    住所の `{def_name}` は変数名も `def_name` にする。
    """
    return control.cli_def_show(name, kind, scope, def_name)


@app.put("/api/projects/{name}/defs/{kind}/{scope}/{def_name}")
def api_project_def_write(
    name: str,
    kind: str,
    scope: str,
    def_name: str,
    payload: dict = Body(...),
) -> dict:
    """定義の保存（body は `{"text": …, "create": false}`・`create` が真なら `--create`）。"""
    return control.cli_def_write(
        name, kind, scope, def_name, payload.get("text"), bool(payload.get("create"))
    )


@app.delete("/api/projects/{name}/defs/{kind}/{scope}/{def_name}")
def api_project_def_delete(
    name: str, kind: str, scope: str, def_name: str
) -> dict:
    """定義の削除（`def delete`・settings は 400「settings は消せません」）。"""
    return control.cli_def_delete(name, kind, scope, def_name)


@app.get("/api/home")
def api_home() -> dict:
    """ホームの集計（質問・承認・プロジェクト・論文の要対応）。"""
    return home.build_home()


@app.get("/api/home/overview")
def api_home_overview(period: str = "week") -> dict:
    """全プロジェクトの合計のダッシュボード（`period` は week / month / all）。

    `/api/home` とは別の住所にしてある。`/api/home` は 15 秒ごとに読み直すので、
    期間を変えて読み直すものと混ぜない（画面の「今週 / 今月 / 全期間」の切り替え）。
    CLI（各プロジェクトの `project dashboard`）を待つので `def` で書く。
    """
    return home.build_overview(period)


# --- Usage（Claude Code の上限と費用） ---


@app.get("/api/usage")
def api_usage() -> dict:
    """Claude Code の使用量（上限と費用）。"""
    return {"cc": usage.cc_card(), "saved_at": int(time.time())}


# --- PWA（静的ファイル）の配信 ---
# API の後に置く．
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
    """agent-link-web を枠（iframe）に入れさせない（要件定義書）。

    ts のオリジンの JS が agent-link-web を枠に入れて、利用者に押させる悪用を防ぐ。
    既に付いている値は上書きしない。
    """
    response = await call_next(request)
    response.headers.setdefault("X-Frame-Options", "DENY")
    response.headers.setdefault("Content-Security-Policy", "frame-ancestors 'none'")
    return response
