"""sai-phone 本体サーバー (Mac 側)．

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
import time

from fastapi import Body, Depends, FastAPI, HTTPException, Query, Request
from fastapi.responses import FileResponse
from fastapi.security import HTTPAuthorizationCredentials, HTTPBearer
from fastapi.staticfiles import StaticFiles
from pydantic import BaseModel
from starlette.concurrency import run_in_threadpool

from sai_phone import (
    approvals,
    control,
    herdr,
    home,
    notify,
    papers,
    questions,
    radio,
    runs,
    sessions,
    store,
    usage,
)
from sai_phone.config import settings
from sai_phone.watch import watch_loop
from pathlib import Path
import asyncio

logger = logging.getLogger("sai_phone")

app = FastAPI(title="sai-phone")

_bearer = HTTPBearer(auto_error=False)


def require_token(
    creds: HTTPAuthorizationCredentials | None = Depends(_bearer),
    token: str | None = Query(default=None),
) -> None:
    """簡易トークン認証．SAI_PHONE_TOKEN が未設定なら認証なし（開発用）．

    なぜクエリ（?token=）も受け付けるか: iPhone 側でヘッダを付けられない
    場面（直接URLを開く等）でも開けるようにするため．
    """
    if not settings.api_token:
        return
    if (creds is not None and creds.credentials == settings.api_token) or token == settings.api_token:
        return
    raise HTTPException(status_code=401, detail="認証エラー")


def require_local(request: Request) -> None:
    """フック・拡張からの内部登録は Mac 自身からのみ受ける．"""
    client = request.client.host if request.client else ""
    if client not in ("127.0.0.1", "::1"):
        raise HTTPException(status_code=403, detail="内部専用です")


@app.on_event("startup")
def _startup() -> None:
    store.init_db()
    notify.ensure_keys()
    if not settings.api_token:
        # トークン無しは開発用．外に公開する前に必ず設定すること．
        logger.warning("SAI_PHONE_TOKEN 未設定：認証なしで動作中")
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
    return runs.build_day(run_list, date, qids, herdr.list_agents(), herdr.list_panes())


@app.get("/api/runs/{project_name}/{run_id}")
def api_run(project_name: str, run_id: int, _: None = Depends(require_token)) -> dict:
    """1実行分（stats も含む．04 の統計画面が使う）．"""
    run = _find_run(project_name, run_id)
    _, qids = _runs_view()
    return runs.run_to_api(run, qids, herdr.list_agents(), herdr.list_panes())


@app.get("/api/runs/{project_name}/{run_id}/items")
def api_run_items(
    project_name: str,
    run_id: int,
    after: str | None = None,
    _: None = Depends(require_token),
) -> dict:
    """会話の項目（旧 GET /api/sessions/{id} と同じ {items, next_cursor} 形）．

    読むファイルは Run の transcript_path（00 §3.7）。
    """
    run = _find_run(project_name, run_id)
    agent = {"cc": "claude", "pi": "pi"}.get(run.get("runner") or "")
    transcript = run.get("transcript_path")
    if agent is None:
        # 未対応の runner
        raise HTTPException(status_code=404, detail="会話の記録が見つかりません")
    if not transcript:
        # 記録の場所がまだ書き出されていない（起動直後など）。
        # 404 にすると画面が「オフライン」になり質問カードも読まなくなるので空で返す
        return {"items": [], "next_cursor": ""}
    path = Path(transcript)
    if not path.exists():
        # 置き場所は分かっているがまだ書かれていない（起動直後など）
        return {"items": [], "next_cursor": ""}
    items, next_cursor = sessions.read_conversation(agent, path, after=after)
    return {"items": items, "next_cursor": next_cursor}


class RegisterRequest(BaseModel):
    """エージェント自身からの登録（どのセッションがどの pane か）．"""

    agent: str  # 'claude' か 'pi'
    session_id: str
    transcript_path: str
    cwd: str
    pane_id: str


@app.post("/api/register")
def api_register(req: RegisterRequest, request: Request) -> dict:
    # 内部登録は Mac 自身からのみ．トークンは付けなくてよい（フックを軽くするため）．
    require_local(request)
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
    # 内部登録は Mac 自身からのみ
    require_local(request)
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
    # 内部専用（Mac 自身からのみ）。トークンは要らない（拡張を軽くするため）
    require_local(request)
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
    result = notify.send_to(info, "sai-phone", "通知を受け取る設定ができました", "/#/")
    return {"ok": True, "test_sent": result == "ok"}


@app.get("/api/questions/{qid}")
def api_get_question(qid: str) -> dict:
    # 拡張機能からのポーリング用．トークン不要の内部専用（Mac 自身からのみ）．
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


# --- cc のコマンド承認（05 §3.3） ---


class ApprovalRegisterRequest(BaseModel):
    """PermissionRequest フックからの承認登録．

    フックは stdin の JSON を丸ごと送ってくるが，保存するのはこの項目だけ
    （cwd / transcript_path / permission_mode 等は落とす）．
    """

    session_id: str
    tool_name: str
    tool_input: dict = {}
    permission_suggestions: list = []
    pane_id: str = ""
    tool_use_id: str | None = None


@app.post("/api/approvals")
def api_register_approval(req: ApprovalRegisterRequest, request: Request) -> dict:
    # 内部登録は Mac 自身からのみ（00・05 §3.3）
    require_local(request)
    # 質問（AskUserQuestion）は別のカード（質問カード）で出すため承認にはしない
    if req.tool_name == "AskUserQuestion":
        return {"ok": True, "skipped": True}
    if not req.session_id or not req.tool_name:
        raise HTTPException(status_code=400, detail="session_id と tool_name が必要です")
    pane_id = req.pane_id
    if not pane_id:
        # HERDR_PANE_ID が無いときの保険（セッション登録から拾う）
        reg = store.get_registration(req.session_id)
        pane_id = (reg or {}).get("pane_id") or ""
    a = approvals.register(
        session_id=req.session_id,
        pane_id=pane_id,
        tool_use_id=req.tool_use_id or "",
        tool_name=req.tool_name,
        tool_input=req.tool_input,
        permission_suggestions=req.permission_suggestions,
    )
    if store.get_registration(req.session_id) is None:
        # SessionStart フックの登録が無いときだけ作る（既存の登録は消さない）
        store.save_registration("claude", req.session_id, "", "", pane_id)
    return {"ok": True, "id": a["id"]}


@app.get("/api/approvals")
def api_session_approvals(
    session_id: str, _: None = Depends(require_token)
) -> dict:
    reg = store.get_registration(session_id)
    pane_id = (reg or {}).get("pane_id") or None
    # PC で先に答えた分は「PC で回答済み」に（質問と同じ判定）
    if pane_id:
        approvals.refresh(session_id, pane_id)
    return {"approvals": store.list_approvals_for_session(session_id)}


class ApprovalAnswerRequest(BaseModel):
    """承認への回答．decision は allow（許可） / always（今後も許可） / deny（拒否）．"""

    decision: str
    reason: str = ""


@app.post("/api/approvals/{aid}/answer")
def api_answer_approval(
    aid: str, req: ApprovalAnswerRequest, _: None = Depends(require_token)
) -> dict:
    ok, reason = approvals.answer(aid, req.decision, req.reason)
    if not ok:
        raise HTTPException(status_code=400, detail=reason)
    return {"ok": True}


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
    """テキスト送信（旧 /api/sessions/{id}/send と同じ動き）．

    送り先ペインは 00 §3.7 で探す（herdr_pane_id → ペイン名）。
    """
    text = (req.text or "").strip()
    if not text:
        raise HTTPException(status_code=400, detail="文章が空です")
    run = _find_run(project_name, run_id)
    pane = runs.pane_for_run(run, herdr.list_agents(), herdr.list_panes())
    if pane is None or not pane.get("pane_id"):
        raise HTTPException(status_code=400, detail="この実行は終了しています")
    if pane.get("agent_status") == "blocked":
        raise HTTPException(
            status_code=400,
            detail="質問への回答は選択肢ボタンから送ってください",
        )
    # 送り先：要る時に追えるように 1 行だけ残す（pane の取り違えの切入点・計画書 A の U06）
    logger.info(
        "send: project=%s run=%s pane_id=%s label=%s",
        project_name,
        run_id,
        pane.get("pane_id"),
        pane.get("label"),
    )
    # pane には herdr の agent 一覧の要素が入る（agent に "claude" / "pi"）．
    # slash コマンドは claude のときだけ確定キーが変わる（herdr._submit_keys）．
    ok, reason = herdr.prompt_agent(pane.get("pane_id"), text, pane.get("agent"))
    if not ok:
        if reason == "blocked":
            raise HTTPException(
                status_code=400,
                detail="質問への回答は選択肢ボタンから送ってください",
            )
        if "繋がり" in reason:
            raise HTTPException(status_code=503, detail=reason)
        raise HTTPException(status_code=400, detail=reason or "送信できなかった")
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
# 外部 CLI（sai-link / sai-paper）を呼ぶため，これらは `def` で書く
# （FastAPI が別スレッドで動かし，CLI を待つ間サーバー全体を止めない・06 §3.4）。


class DaemonRequest(BaseModel):
    """常駐の操作．action は start / stop．"""

    action: str


@app.get("/api/projects")
def api_projects(_: None = Depends(require_token)) -> dict:
    """登録簿のプロジェクト一覧（最後に動いた順）．"""
    return {"projects": control.list_projects()}


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


@app.get("/api/projects/{name}/runs/{run_id:int}/view/{view}")
def api_project_run_view(
    name: str, run_id: int, view: str, _: None = Depends(require_token)
) -> dict:
    """run の中身（Log / Stats / History・計画 12 §3.2）。"""
    return control.phone_run_view(name, run_id, view)


@app.get("/api/projects/{name}/runs/{run_id:int}/md/{kind}")
def api_project_run_md(
    name: str, run_id: int, kind: str, _: None = Depends(require_token)
) -> dict:
    """run の md（task / review / message・無いときは 404・計画 12 §3.2）。"""
    return control.phone_md(name, run_id, kind)


@app.put("/api/projects/{name}/runs/{run_id:int}/md/{kind}")
def api_project_run_md_save(
    name: str,
    run_id: int,
    kind: str,
    payload: dict = Body(...),
    _: None = Depends(require_token),
) -> dict:
    """task（未実行の todo カードだけ）/ review の保存（body は `{"text": …}`）。"""
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


@app.get("/api/projects/{name}/card/inputs")
def api_project_card_inputs(
    name: str, block: str = Query(""), _: None = Depends(require_token)
) -> dict:
    """手順 4 の入力欄と候補トークン（`?block=<id>`・計画 15 §1.5）．

    無い・空でも 422 ではなく control の 400（日本語）で返すため既定は空文字。
    """
    return control.phone_card_inputs(name, block)


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


# --- Papers タブ（計画 07・Discover は 08・プロジェクト付きの住所は 17 §3.4） ---
# 「論文の置き場があるプロジェクト」（17 §3.1）の名前付き住所。古い /api/papers/... は消した（17 §3.4）。
# 住所から root は 1 度だけ引く（papers.resolve_project・プロジェクト一覧を何度も呼ばない・17 §3.2）。
# 固定パス（library / tree / providers / search / logs）を /api/projects/{name}/papers/{paper_id:int} より先に
# 登録する（FastAPI は登録順に照合するため・00 §6）。外部 CLI を呼ぶので `def` で書く（06 §3.4）。


@app.get("/api/paper-projects")
def api_paper_projects(_: None = Depends(require_token)) -> list[dict]:
    """論文の置き場があるプロジェクトの一覧（最後に動いた順・17 §3.4）。"""
    return papers.paper_projects_view()


@app.get("/api/projects/{name}/papers/library")
def api_papers_library(name: str, _: None = Depends(require_token)) -> dict:
    """Library 一覧（sai-paper `phone library`）。"""
    return papers.library(papers.resolve_project(name))


@app.get("/api/projects/{name}/papers/tree")
def api_papers_tree(name: str, _: None = Depends(require_token)) -> dict:
    """Library 階層（sai-paper `phone tree`・検索ごとのまとまり）。"""
    return papers.tree(papers.resolve_project(name))


@app.get("/api/projects/{name}/papers/dashboard")
def api_papers_dashboard(name: str, _: None = Depends(require_token)) -> dict:
    """Dashboard（sai-paper `phone dashboard`・数字カードと出来事の一覧）。"""
    return papers.dashboard(papers.resolve_project(name))


# --- Discover（計画 08 §3） ---
# どれも /api/projects/{name}/papers/{paper_id:int} より先に登録する（providers / search / logs は
# {id} と住所が競うため・00 §6 の注意書き）。検索は外部サービスへ行くため待ちは長め（120 秒）。


@app.get("/api/projects/{name}/papers/providers")
def api_papers_providers(name: str, _: None = Depends(require_token)) -> dict:
    """検索先の一覧と既定（sai-paper `phone providers`）。"""
    return papers.providers(papers.resolve_project(name))


@app.post("/api/projects/{name}/papers/search")
def api_papers_search(
    name: str, req: papers.SearchRequest, _: None = Depends(require_token)
) -> dict:
    """検索（sai-paper `phone search`・検索履歴にも残る・08 §3）。"""
    return papers.search(papers.resolve_project(name), req)


@app.get("/api/projects/{name}/papers/logs/{log_id:int}")
def api_papers_search_log(
    name: str, log_id: int, _: None = Depends(require_token)
) -> dict:
    """過去の検索結果（sai-paper `phone log <log_id>`）。"""
    return papers.search_log(papers.resolve_project(name), log_id)


# --- Import・Download・Project（U3 §変更内容 2・共有契約 C-A2） ---
# sai-paper-tui の BibTeX の取り込み・PDF の Download・Project の画面を Web からも使う。
# 固定の住所（import / downloads / project / attention）は
# `/api/projects/{name}/papers/{paper_id:int}` より先に登録する（FastAPI は登録順に照合するため・00 §6）。
# 外から来る値（.bib の名前・本文・Download の依頼・メモ・key）は papers 側で
# CLI を呼ぶ前に確かめる。


@app.get("/api/projects/{name}/papers/import")
def api_papers_import_list(name: str, _: None = Depends(require_token)) -> dict:
    """.bib の一覧と中の論文（sai-paper `phone import list`）。"""
    return papers.import_list(papers.resolve_project(name))


@app.post("/api/projects/{name}/papers/import")
def api_papers_import_add(
    name: str, payload: dict = Body(...), _: None = Depends(require_token)
) -> dict:
    """.bib の保存（body は `{"name": "refs.bib" | "", "text": "@article{…}"}`・本文は標準入力）。"""
    return papers.import_add(
        papers.resolve_project(name), payload.get("name"), payload.get("text")
    )


@app.delete("/api/projects/{name}/papers/import/{file_name}")
def api_papers_import_remove(
    name: str, file_name: str, _: None = Depends(require_token)
) -> dict:
    """.bib の削除（sai-paper `phone import remove`）。"""
    return papers.import_remove(papers.resolve_project(name), file_name)


@app.post("/api/projects/{name}/papers/download", status_code=202)
async def api_papers_download(
    name: str, payload: dict = Body(...), _: None = Depends(require_token)
) -> dict:
    """論文の PDF を取る（背景で `phone download` → `phone after-download` を走らせる）。

    body は `{"log_id": 3, "position": 2, "title": "…"}` か `{"import_id": "DOI:…", "title": "…"}`。
    """
    return await papers.request_download(papers.resolve_project(name), payload)


@app.get("/api/projects/{name}/papers/downloads")
def api_papers_downloads(name: str, _: None = Depends(require_token)) -> dict:
    """走っている／終わった論文のダウンロード（新しい順）。"""
    return papers.list_downloads(papers.resolve_project(name))


# --- 手動で取り込んだ PDF の取り込み（U05 §3） ---
# 固定の住所（manual/candidates・manual/import・manual/upload）は
# `/api/projects/{name}/papers/{paper_id:int}` より先に登録する（登録順に照合するため・00 §6）。
# アップロードは本文そのもの（application/pdf）で受ける。FastAPI の UploadFile は
# python-multipart を要するため使わない（依存を増やさない大原則）。


@app.get("/api/projects/{name}/papers/manual/candidates")
def api_papers_manual_candidates(name: str, _: None = Depends(require_token)) -> dict:
    """ダウンロードフォルダの最近の PDF 一覧（`phone manual-candidates --minutes 30`）。"""
    return papers.manual_candidates(papers.resolve_project(name))


@app.post("/api/projects/{name}/papers/manual/import", status_code=202)
async def api_papers_manual_import(
    name: str, payload: dict = Body(...), _: None = Depends(require_token)
) -> dict:
    """ダウンロードフォルダの PDF を取り込む（202・background で `phone manual-import`）。

    body は「対象のどれか 1 つ」＋ `{"file_name": "paper.pdf"}`。
    """
    body = dict(payload or {})
    file_name = body.pop("file_name", None)
    return await papers.request_manual_import(papers.resolve_project(name), body, file_name)


@app.post("/api/projects/{name}/papers/manual/upload", status_code=202)
async def api_papers_manual_upload(
    name: str, request: Request, _: None = Depends(require_token)
) -> dict:
    """この端末から送った PDF を取り込む（202・本文は PDF そのもの）。

    対象はクエリで渡す（例 `?log_id=3&position=2` / `?paper_id=12` / `?cite_paper_id=12&ref_no=3`）。
    Content-Type が違えば 415・大きすぎる／Content-Length が無ければ 413・
    先頭が `%PDF-` でなければ 400（検査は CLI を呼ぶ前に papers 側で行う）。
    """
    project = papers.resolve_project(name)
    target = {k: v for k, v in request.query_params.items()}
    # 対象は本文を読む前に確かめる（外から来る値は先に検査するため・00 §2）
    papers._manual_target(target)
    body = await papers.read_pdf_upload(request, target)
    return await papers.request_manual_import(project, target, body=body)


@app.get("/api/projects/{name}/papers/project")
def api_papers_project(name: str, _: None = Depends(require_token)) -> dict:
    """現況・考え・状況のまとめ・要対応（sai-paper `phone project`）。"""
    return papers.project_view(papers.resolve_project(name))


@app.post("/api/projects/{name}/papers/project/memo")
def api_papers_project_memo(
    name: str, payload: dict = Body(...), _: None = Depends(require_token)
) -> dict:
    """mind.md のメモを 1 行足す（body は `{"text": "…"}`・`phone mind-append`）。"""
    return papers.mind_append(papers.resolve_project(name), payload.get("text"))


@app.post("/api/projects/{name}/papers/attention/resolve")
def api_papers_attention_resolve(
    name: str, payload: dict = Body(...), _: None = Depends(require_token)
) -> dict:
    """要対応の 1 件を「対応済み」にする（body は `{"key": "…"}`・`phone attention-resolve`）。"""
    return papers.attention_resolve(papers.resolve_project(name), payload.get("key"))


@app.get("/api/projects/{name}/papers/{paper_id:int}")
def api_paper_detail(
    name: str, paper_id: int, _: None = Depends(require_token)
) -> dict:
    """論文情報（題名・要旨・ノート・引用文献・ファイルの置き場）。"""
    return papers.detail(papers.resolve_project(name), paper_id)


@app.put("/api/projects/{name}/papers/{paper_id:int}/note")
def api_paper_note(
    name: str, paper_id: int, payload: dict = Body(...), _: None = Depends(require_token)
) -> dict:
    """ノート（note.md）の保存（body は `{"text": …}`・無ければ Mac の n と同じ場所に作る）。"""
    return papers.save_note(papers.resolve_project(name), paper_id, payload.get("text"))


@app.get("/api/projects/{name}/papers/{paper_id:int}/figures/{fig_no:int}")
def api_paper_figure(
    name: str, paper_id: int, fig_no: int, _: None = Depends(require_token)
) -> FileResponse:
    """図の画像（<img> はヘッダを付けられないので合言葉は ?token= で来る）。"""
    return FileResponse(papers.figure_path(papers.resolve_project(name), paper_id, fig_no))


@app.post("/api/projects/{name}/papers/{paper_id:int}/translate")
def api_paper_translate(
    name: str, paper_id: int, _: None = Depends(require_token)
) -> dict:
    """要旨の日本語訳を作る（数十秒かかることがある・07 §4.4）。

    動いているあいだは「進み具合の一枚」に `kind="translate"` の仕事として出す（U05 §1）。
    """
    return papers.translate_with_job(papers.resolve_project(name), paper_id)


@app.get("/api/projects/{name}/papers/{paper_id:int}/citations")
def api_paper_citations(
    name: str, paper_id: int, _: None = Depends(require_token)
) -> dict:
    """引用一覧（読み取りだけ・抜き出す前の論文にも空の形で返る・U05 §2）。"""
    return papers.citations(papers.resolve_project(name), paper_id)


@app.post("/api/projects/{name}/papers/{paper_id:int}/citations/extract", status_code=202)
async def api_paper_citations_extract(
    name: str, paper_id: int, _: None = Depends(require_token)
) -> dict:
    """引用を PDF から抜き出す（202・background で `phone citations-extract`）。"""
    return await papers.request_cite_extract(papers.resolve_project(name), paper_id)



@app.post("/api/projects/{name}/papers/{paper_id:int}/radio", status_code=202)
async def api_paper_radio(
    name: str, paper_id: int, _: None = Depends(require_token)
) -> dict:
    """音声化の依頼（背景で `sai-paper phone radio` を走らせる・計画 10 §3）。"""
    return await papers.request_radio(papers.resolve_project(name), paper_id)


@app.get("/api/projects/{name}/papers/{paper_id:int}/text")
def api_paper_text(
    name: str, paper_id: int, _: None = Depends(require_token)
) -> dict:
    """本文（Markdown）。md_path は返さない（07 §3.2）。"""
    return papers.text(papers.resolve_project(name), paper_id)


@app.get("/api/projects/{name}/papers/{paper_id:int}/pdf")
def api_paper_pdf(
    name: str, paper_id: int, lang: str | None = None, _: None = Depends(require_token)
) -> FileResponse:
    """PDF（`lang=ja` で日本語訳）。Safari でその場で見せるため inline で返す（07 §3.3）。"""
    path = papers.pdf_path(papers.resolve_project(name), paper_id, lang)
    suffix = "-ja" if lang == "ja" else ""
    return FileResponse(
        path,
        media_type="application/pdf",
        filename=f"paper-{paper_id}{suffix}.pdf",
        content_disposition_type="inline",
    )


# --- Canvas（U04 §C-D） ---
# sai-paper-tui の Canvas の画面（U06 / U07）が使う住所。sai-paper の
# `phone canvas …`（U03）を中継し，外から来る値は必ず papers 側で検査する。
# 住所から root は 1 度だけ引く（papers.resolve_project・17 §3.2）。
# `{paper_id:int}` の前に登録しても邪魔はしない（`canvas` は int に変換されないため・U04 §前提）。


@app.get("/api/projects/{name}/papers/canvas")
def api_canvas_list(name: str, _: None = Depends(require_token)) -> dict:
    """Canvas 一覧（sai-paper `phone canvas list`）。"""
    return papers.canvas_list(papers.resolve_project(name))


@app.post("/api/projects/{name}/papers/canvas")
def api_canvas_new(
    name: str, payload: dict = Body(...), _: None = Depends(require_token)
) -> dict:
    """Canvas を新しく作る（body は `{"name": "…"}`・U04 §C-D）。"""
    return papers.canvas_new(papers.resolve_project(name), payload.get("name"))


@app.get("/api/projects/{name}/papers/canvas/{canvas}")
def api_canvas_show(name: str, canvas: str, _: None = Depends(require_token)) -> dict:
    """Canvas の中身（ノードの一覧と配置・U04 §C-D）。"""
    return papers.canvas_show(papers.resolve_project(name), canvas)


@app.put("/api/projects/{name}/papers/canvas/{canvas}/layout")
def api_canvas_layout(
    name: str, canvas: str, payload: dict = Body(...), _: None = Depends(require_token)
) -> dict:
    """位置を保存する（body は `{"nodes": [{"id","x","y","width"?,"height"?}]}`・U04 §C-D）。"""
    return papers.canvas_layout(papers.resolve_project(name), canvas, payload)


@app.post("/api/projects/{name}/papers/canvas/{canvas}/toggle")
def api_canvas_toggle(
    name: str, canvas: str, payload: dict = Body(...), _: None = Depends(require_token)
) -> dict:
    """札を出す・隠す（body は `{"node_id": "…", "near": "…" または null}`・U04 §C-D）。"""
    return papers.canvas_toggle(
        papers.resolve_project(name), canvas, payload.get("node_id"), payload.get("near")
    )


@app.get("/api/projects/{name}/papers/canvas/{canvas}/nodes/{node_id}/missing")
def api_canvas_missing(
    name: str, canvas: str, node_id: str, _: None = Depends(require_token)
) -> dict:
    """Canvas に無いつながりの一覧（U04 §C-D）。"""
    return papers.canvas_missing(papers.resolve_project(name), canvas, node_id)


@app.get("/api/projects/{name}/papers/canvas/{canvas}/nodes/{node_id}/body")
def api_canvas_body(
    name: str, canvas: str, node_id: str, _: None = Depends(require_token)
) -> dict:
    """ノードの本文（Markdown・U04 §C-D）。"""
    return papers.canvas_body(papers.resolve_project(name), canvas, node_id)


# --- Radio 中継（計画 09 §3） ---
# sai-radio（127.0.0.1:8000）への GET / POST の中継。合言葉（require_token）で守る。
# 中継の待ち時間でサーバー全体を止めないようスレッドプールで呼ぶ（09 §3.3）。
# 静的ファイルの app.mount("/") より先に登録すること（09 §3.3）。


@app.api_route("/api/radio/{path:path}", methods=["GET", "POST"])
async def api_radio(path: str, request: Request, _: None = Depends(require_token)):
    body = await request.body()
    return await run_in_threadpool(
        radio.forward,
        request.method,
        path,
        list(request.query_params.multi_items()),
        body,
        request.headers.get("content-type"),
        request.headers.get("range"),
    )


# --- Usage タブ（計画 42 §3.3） ---
# 外部 API（OpenRouter / DeepSeek）へ行くため `def` で書く（06 §3.4）。
# 外から来る provider / account は設定にある名前だけ受け付け，それ以外は 404。


@app.get("/api/usage")
def api_usage(refresh: int = 0, _: None = Depends(require_token)) -> dict:
    """Usage タブのカード一覧（cc / OpenRouter 各アカウント / DeepSeek）．

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
