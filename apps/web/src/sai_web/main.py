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
from fastapi.responses import FileResponse, JSONResponse, Response
from fastapi.security import HTTPAuthorizationCredentials, HTTPBearer
from fastapi.staticfiles import StaticFiles
from pydantic import BaseModel
from starlette.concurrency import run_in_threadpool

from sai_web import (
    auth,
    commands,
    control,
    home,
    mlflow_proxy,
    notify,
    papers,
    questions,
    radio,
    registry,
    runs,
    sci,
    sessions,
    store,
    usage,
)
from sai_web.config import settings
from sai_web.watch import watch_loop
from pathlib import Path
import asyncio

logger = logging.getLogger("sai_web")

# 多人数モードでは API の説明（/docs・/openapi.json）を出さない（共有契約 C5）。
# 単独モードは今までどおり出す（開発とテスト用）。
if registry.multi_user():
    app = FastAPI(title="sai-web", docs_url=None, redoc_url=None, openapi_url=None)
else:
    app = FastAPI(title="sai-web")

_bearer = HTTPBearer(auto_error=False)


def require_token(
    creds: HTTPAuthorizationCredentials | None = Depends(_bearer),
    token: str | None = Query(default=None),
) -> None:
    """入口の守り。

    - 多人数モードでは、ミドルウェアが入れた今の利用者（`registry.current_user`）が
      いれば通す。合言葉（Bearer の合言葉・`?token=`）は受けない。
    - 単独モードは今までどおり `SAI_WEB_TOKEN` と照合する（未設定なら認証なし）。

    なぜクエリ（?token=）も受け付けるか: iPhone 側でヘッダを付けられない
    場面（直接URLを開く等）でも開けるようにするため．
    """
    if registry.multi_user():
        if registry.current_user.get() is None:
            raise HTTPException(status_code=401, detail="ログインしてください")
        return
    if not settings.api_token:
        return
    if (creds is not None and creds.credentials == settings.api_token) or token == settings.api_token:
        return
    raise HTTPException(status_code=401, detail="認証エラー")


@app.middleware("http")
async def _multi_user_auth(request: Request, call_next):
    """多人数モードの入口（共有契約 C5）。

    1. `Authorization: Bearer sai_…` があれば個人のトークンとして利用者を引く。
    2. 無ければ Cookie `sai_session` から利用者を引く。
    3. 変更系で Cookie で入った要求は `Origin` と `Host` を照合する（CSRF 対策）。
    4. 初回のパスワード変更前は `/api/auth/` 以外の `/api/` を止める。

    依存関数ではなくミドルウェアにするのは、同期の住所の関数が別のスレッドで
    動いても `ContextVar` が届くようにするため（`call_next` の前に set する）。
    """
    if not registry.multi_user():
        return await call_next(request)

    user: dict | None = None
    via_api_token = False

    # 1. 個人のトークン（`sai_…`）
    authz = request.headers.get("authorization") or ""
    if authz.lower().startswith("bearer "):
        value = authz[len("bearer "):].strip()
        if value.startswith(auth.API_TOKEN_PREFIX):
            row = store.find_api_token(auth.sha256_hex(value))
            if row is not None:
                candidate = store.get_user(str(row.get("user_name") or ""))
                if candidate is not None and not candidate.get("disabled"):
                    user = candidate
                    via_api_token = True

    # 2. Cookie
    if user is None:
        cookie = request.cookies.get(auth.SESSION_COOKIE)
        if cookie:
            candidate = store.get_session_user(auth.sha256_hex(cookie))
            if candidate is not None and not candidate.get("disabled"):
                user = candidate

    if user is None:
        registry.current_user.set(None)
    else:
        registry.current_user.set(
            {
                "name": user.get("name") or "",
                "email": user.get("email") or "",
                "is_admin": bool(user.get("is_admin")),
            }
        )
        request.state.via_api_token = via_api_token

    # 3. CSRF: 変更系で Cookie で入った要求だけ見る（個人のトークンは見ない）
    if (
        user is not None
        and not via_api_token
        and request.method in ("POST", "PUT", "PATCH", "DELETE")
    ):
        origin = request.headers.get("origin")
        if origin:
            from urllib.parse import urlparse

            origin_host = urlparse(origin).netloc
            host = request.headers.get("host") or ""
            if origin_host and origin_host != host:
                return JSONResponse(
                    {"detail": "別のサイトからの要求は受けません"}, status_code=403
                )

    # 4. 初回のパスワード変更前は /api/auth/ 以外の /api/ を止める
    path = request.url.path
    if (
        user is not None
        and user.get("must_change_password")
        and path.startswith("/api/")
        and not path.startswith("/api/auth/")
    ):
        return JSONResponse(
            {"detail": "パスワードを変えてください"}, status_code=403
        )

    return await call_next(request)


def require_local(request: Request) -> None:
    """フック・拡張からの内部登録は Mac 自身からのみ受ける．"""
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
    from sai_web import registry

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
    if registry.multi_user():
        # 多人数モードでは合言葉を使わない（ログインは Cookie・sai-prep は個人のトークン）。
        # 合言葉のファイルは書かない。
        pass
    elif not settings.api_token:
        # トークン無しは開発用．外に公開する前に必ず設定すること．
        logger.warning("SAI_WEB_TOKEN 未設定：認証なしで動作中")
    # 合言葉をファイルで渡す（`phone_api.py` が読む）。多人数モードでは書かない。
    if not registry.multi_user():
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


# --- アカウントとログイン（U06・共有契約 C5'） ---
# ここは require_token を付けない（未ログインでも呼べる必要があるため）。
# 多人数モードでないときは 404 を返す。


class LoginRequest(BaseModel):
    email: str
    password: str


@app.post("/api/auth/login")
def api_auth_login(req: LoginRequest, response: Response) -> dict:
    """メールアドレスとパスワードでログインする（Cookie を配る）。"""
    if not registry.multi_user():
        raise HTTPException(status_code=404, detail="多人数モードではありません")
    email = req.email.strip().lower()
    user = store.get_user_by_email(email)
    if (
        user is None
        or user.get("disabled")
        or not auth.verify_password(req.password, str(user.get("password_hash") or ""))
    ):
        # どちらが違うかは言わない
        raise HTTPException(
            status_code=401, detail="メールアドレスかパスワードが違います"
        )
    token = auth.new_session_token()
    store.create_session(
        auth.sha256_hex(token),
        str(user["name"]),
        time.time() + auth.SESSION_DAYS * 86400,
    )
    auth.set_session_cookie(response, token)
    return auth.user_public(user)


@app.post("/api/auth/logout")
def api_auth_logout(request: Request, response: Response) -> dict:
    """ログアウトする（セッションを消して Cookie を消す）。"""
    if not registry.multi_user():
        raise HTTPException(status_code=404, detail="多人数モードではありません")
    cookie = request.cookies.get(auth.SESSION_COOKIE)
    if cookie:
        store.delete_session(auth.sha256_hex(cookie))
    auth.clear_session_cookie(response)
    return {"ok": True}


class PasswordRequest(BaseModel):
    current: str
    new: str


@app.post("/api/auth/password")
def api_auth_password(req: PasswordRequest, response: Response) -> dict:
    """今のパスワードを確かめて変える。他のセッションは消す。"""
    if not registry.multi_user():
        raise HTTPException(status_code=404, detail="多人数モードではありません")
    current = registry.current_user.get()
    if not isinstance(current, dict) or not current.get("name"):
        raise HTTPException(status_code=401, detail="ログインしてください")
    name = str(current["name"])
    row = store.get_user(name)
    if row is None or not auth.verify_password(
        req.current, str(row.get("password_hash") or "")
    ):
        raise HTTPException(status_code=400, detail="今のパスワードが違います")
    try:
        auth.check_password(req.new)
    except ValueError as e:
        raise HTTPException(status_code=400, detail=str(e))
    store.update_user(
        name,
        password_hash=auth.hash_password(req.new),
        must_change_password=False,
    )
    # 他のセッションを消し、この端末には新しいセッションを配る
    store.delete_sessions_of(name)
    token = auth.new_session_token()
    store.create_session(
        auth.sha256_hex(token), name, time.time() + auth.SESSION_DAYS * 86400
    )
    auth.set_session_cookie(response, token)
    return {"ok": True}


@app.get("/api/auth/me")
def api_auth_me() -> dict:
    """今のモードと利用者（単独モードは `{"mode":"single"}`）。"""
    if not registry.multi_user():
        return {"mode": "single"}
    current = registry.current_user.get()
    if not isinstance(current, dict) or not current.get("name"):
        raise HTTPException(status_code=401, detail="ログインしてください")
    row = store.get_user(str(current["name"]))
    if row is None:
        raise HTTPException(status_code=401, detail="ログインしてください")
    return {
        "mode": "multi",
        "name": row.get("name") or "",
        "email": row.get("email") or "",
        "is_admin": bool(row.get("is_admin")),
        "must_change_password": bool(row.get("must_change_password")),
    }


# --- 個人のトークン（sai-prep が使う・U06） ---
# 値は作った 1 回だけ返す。DB には SHA-256 だけを置く。


def _require_multi() -> None:
    """多人数モードでなければ 404。"""
    if not registry.multi_user():
        raise HTTPException(status_code=404, detail="多人数モードではありません")


def _current_name() -> str:
    """今の利用者の名前。いなければ 401。"""
    current = registry.current_user.get()
    if not isinstance(current, dict) or not current.get("name"):
        raise HTTPException(status_code=401, detail="ログインしてください")
    return str(current["name"])


@app.get("/api/me/tokens")
def api_me_tokens(_: None = Depends(require_token)) -> dict:
    """自分の個人のトークンの一覧（値は返さない）。"""
    _require_multi()
    tokens = [
        {
            "id": str(r.get("token_sha256") or "")[:12],
            "label": r.get("label") or "",
            "created_at": r.get("created_at"),
            "last_used_at": r.get("last_used_at"),
        }
        for r in store.list_api_tokens(_current_name())
    ]
    return {"tokens": tokens}


class TokenRequest(BaseModel):
    label: str


@app.post("/api/me/tokens")
def api_me_tokens_create(req: TokenRequest, _: None = Depends(require_token)) -> dict:
    """個人のトークンを作る（値はこの 1 回だけ返す）。"""
    _require_multi()
    name = _current_name()
    label = (req.label or "").strip()
    if not (1 <= len(label) <= 60):
        raise HTTPException(status_code=400, detail="ラベルは 1〜60 文字にしてください")
    token = auth.new_api_token()
    digest = auth.sha256_hex(token)
    store.create_api_token(digest, name, label)
    return {"id": digest[:12], "label": label, "token": token}


@app.delete("/api/me/tokens/{token_id}")
def api_me_tokens_delete(token_id: str, _: None = Depends(require_token)) -> dict:
    """自分の個人のトークンを消す（他人の物は消せない）。"""
    _require_multi()
    if not store.delete_api_token(token_id, _current_name()):
        raise HTTPException(status_code=404, detail="トークンが見つかりません")
    return {"ok": True}


# --- 管理者（U06） ---


def _admin_user_view(user: dict) -> dict:
    """管理画面に返す 1 人分。"""
    return {
        "name": user.get("name") or "",
        "email": user.get("email") or "",
        "is_admin": bool(user.get("is_admin")),
        "disabled": bool(user.get("disabled")),
        "must_change_password": bool(user.get("must_change_password")),
        "created_at": user.get("created_at"),
    }


@app.get("/api/admin/users")
def api_admin_users(
    _: None = Depends(require_token), __: None = Depends(auth.require_admin)
) -> dict:
    """利用者の一覧（管理者だけ）。"""
    return {"users": [_admin_user_view(u) for u in store.list_users()]}


class CreateUserRequest(BaseModel):
    name: str
    email: str
    password: str
    is_admin: bool = False


@app.post("/api/admin/users")
def api_admin_create(
    req: CreateUserRequest,
    _: None = Depends(require_token),
    __: None = Depends(auth.require_admin),
) -> dict:
    """利用者を作る（同じ名前・同じメールは 409）。"""
    try:
        name = auth.check_name(req.name)
        email = auth.check_email(req.email)
        auth.check_password(req.password)
    except ValueError as e:
        raise HTTPException(status_code=400, detail=str(e))
    if store.get_user(name) is not None or store.get_user_by_email(email) is not None:
        raise HTTPException(
            status_code=409, detail="同じ名前かメールアドレスが既にあります"
        )
    auth.create_user_dirs(name)
    user = store.create_user(
        name,
        email,
        auth.hash_password(req.password),
        is_admin=req.is_admin,
        must_change_password=True,
    )
    return _admin_user_view(user)


class PatchUserRequest(BaseModel):
    email: str | None = None
    is_admin: bool | None = None
    disabled: bool | None = None
    password: str | None = None


@app.patch("/api/admin/users/{user_name}")
def api_admin_patch(
    user_name: str,
    req: PatchUserRequest,
    _: None = Depends(require_token),
    __: None = Depends(auth.require_admin),
) -> dict:
    """利用者を変える（自分自身の管理者外し・無効化は 400）。"""
    target = store.get_user(user_name)
    if target is None:
        raise HTTPException(status_code=404, detail="利用者が見つかりません")
    me = registry.current_user.get()
    me_name = me.get("name") if isinstance(me, dict) else ""
    fields: dict = {}
    if req.email is not None:
        try:
            email = auth.check_email(req.email)
        except ValueError as e:
            raise HTTPException(status_code=400, detail=str(e))
        other = store.get_user_by_email(email)
        if other is not None and other.get("name") != user_name:
            raise HTTPException(
                status_code=409, detail="同じメールアドレスが既にあります"
            )
        fields["email"] = email
    if req.is_admin is not None:
        if user_name == me_name and not req.is_admin:
            raise HTTPException(status_code=400, detail="自分自身の管理者を外せません")
        fields["is_admin"] = req.is_admin
    if req.disabled is not None:
        if user_name == me_name and req.disabled:
            raise HTTPException(status_code=400, detail="自分自身を無効にできません")
        fields["disabled"] = req.disabled
    if req.password is not None:
        try:
            auth.check_password(req.password)
        except ValueError as e:
            raise HTTPException(status_code=400, detail=str(e))
        fields["password_hash"] = auth.hash_password(req.password)
        fields["must_change_password"] = True
    store.update_user(user_name, **fields)
    if "password_hash" in fields or fields.get("disabled") is True:
        # パスワードを変えた・無効にしたときは、その人のセッションを消す
        store.delete_sessions_of(user_name)
    return {"ok": True}


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
    # 多人数モードでは、今の利用者の置き場の外を指す会話の記録は読まない
    if transcript and registry.multi_user() and not registry.within_user(transcript):
        raise HTTPException(status_code=404, detail="会話の記録が見つかりません")
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
    result = notify.send_to(info, "sai", "通知を受け取る設定ができました", "/#/")
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
    return control.run_send(project_name, run_id, text)


@app.post("/api/runs/{project_name}/{run_id}/interrupt")
def api_interrupt(
    project_name: str,
    run_id: int,
    _: None = Depends(require_token),
) -> dict:
    """実行中の run を中断する（`sai-link run interrupt <run_id> --json`・C2）．

    無い実行では CLI を呼ばずに 404 を返す。
    """
    _find_run(project_name, run_id)  # 無い run は 404（CLI を呼ばない）
    return control.run_interrupt(project_name, run_id)


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
    """文書（task / message / waggle）の検索（`phone search`・Dashboard の検索）。

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
# 3 つの CLI（link / paper / sci）の schema から入力欄を作り，命令を実行する。
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


@app.post("/api/projects/{name}/papers/find")
def api_papers_find(name: str, req: papers.FindRequest, _: None = Depends(require_token)) -> dict:
    """内部検索（sai-paper `phone find`・履歴にも残る）。"""
    return papers.find(papers.resolve_project(name), req)


@app.get("/api/projects/{name}/papers/find-logs/{log_id:int}")
def api_papers_find_log(
    name: str, log_id: int, _: None = Depends(require_token)
) -> dict:
    """過去の内部検索の結果（sai-paper `phone find-log <log_id>`）。"""
    return papers.find_log(papers.resolve_project(name), log_id)


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


@app.post("/api/projects/{name}/papers/{paper_id:int}/translate-pdf", status_code=202)
async def api_paper_translate_pdf(
    name: str, paper_id: int, _: None = Depends(require_token)
) -> dict:
    """PDF の日本語訳を始める（202・背景で `phone translate-pdf`）。"""
    return await papers.request_translate_pdf(papers.resolve_project(name), paper_id)


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


@app.get("/api/projects/{name}/papers/{paper_id:int}/chunks")
def api_paper_chunks(
    name: str, paper_id: int, _: None = Depends(require_token)
) -> dict:
    """論文のチャンク一覧（`#論文.章.段落`）。本文が無ければ 404。"""
    return papers.chunks(papers.resolve_project(name), paper_id)


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


# --- research-tree（sai-sci・U18 C-13） ---
# 検査は sci.py が CLI を呼ぶ前に行う（外から来る値で CLI を呼ぶため・U18 決定）。
# READ_CARD の「始め」「終え」だけは sai-link の `phone user …`（C-12）を sci.py 越しに呼ぶ。


@app.get("/api/projects/{name}/papers/sci/status")
def api_sci_status(name: str, _: None = Depends(require_token)) -> dict:
    """research-tree 全体（`phone sci status`）。"""
    return sci.status(papers.resolve_project(name))


@app.get("/api/projects/{name}/papers/sci/types")
def api_sci_types(name: str, _: None = Depends(require_token)) -> dict:
    """使える RESEARCH_TYPE（`phone sci types`）。"""
    return sci.types(papers.resolve_project(name))


@app.post("/api/projects/{name}/papers/sci/init")
def api_sci_init(
    name: str, payload: dict = Body(...), _: None = Depends(require_token)
) -> dict:
    """始める（body は `{"type": "problem"}`）。"""
    return sci.init(papers.resolve_project(name), payload)


@app.get("/api/projects/{name}/papers/sci/nodes/{node_id}")
def api_sci_node(
    name: str, node_id: str, _: None = Depends(require_token)
) -> dict:
    """Node の中身（`phone sci node <ID>`）。"""
    return sci.node(papers.resolve_project(name), node_id)


@app.post("/api/projects/{name}/papers/sci/apply")
def api_sci_apply(
    name: str, payload: dict = Body(...), _: None = Depends(require_token)
) -> dict:
    """人の操作を反映する（body は C-OP の op の JSON）。"""
    return sci.apply(papers.resolve_project(name), payload)


@app.post("/api/projects/{name}/papers/sci/proposals/{pid}")
def api_sci_triage(
    name: str, pid: str, payload: dict = Body(...), _: None = Depends(require_token)
) -> dict:
    """TRIAGE する（body は `{"action","edit"}`）。"""
    return sci.triage(papers.resolve_project(name), pid, payload)


@app.post("/api/projects/{name}/papers/sci/cards")
def api_sci_card(
    name: str, payload: dict = Body(...), _: None = Depends(require_token)
) -> dict:
    """READ_CARD を作る（body はカードの JSON）。"""
    return sci.create_card(papers.resolve_project(name), payload)


@app.get("/api/projects/{name}/papers/sci/reads/{run_id}")
def api_sci_read(
    name: str, run_id: str, _: None = Depends(require_token)
) -> dict:
    """READ_CARD の中身（`phone sci read <run_id>`）。"""
    return sci.read(papers.resolve_project(name), run_id)


@app.post("/api/projects/{name}/papers/sci/reads/{run_id}/start")
def api_sci_read_start(
    name: str, run_id: str, _: None = Depends(require_token)
) -> dict:
    """READ_CARD を始める（`sai-link phone user start`）。"""
    return sci.read_start(papers.resolve_project(name), run_id)


@app.post("/api/projects/{name}/papers/sci/reads/{run_id}/finish")
def api_sci_read_finish(
    name: str, run_id: str, _: None = Depends(require_token)
) -> dict:
    """READ_CARD を終えて review へ移す（F が 0 でも止めない）。"""
    return sci.read_finish(papers.resolve_project(name), run_id)


# --- 検索セット（`phone set …`） ---


@app.get("/api/projects/{name}/papers/sets")
def api_set_list(name: str, _: None = Depends(require_token)) -> dict:
    """検索セットの一覧（`phone set list`）。"""
    return sci.set_list(papers.resolve_project(name))


@app.get("/api/projects/{name}/papers/sets/{set_id}")
def api_set_show(name: str, set_id: str, _: None = Depends(require_token)) -> dict:
    """検索セット 1 つと候補（`phone set show`）。"""
    return sci.set_show(papers.resolve_project(name), set_id)


@app.post("/api/projects/{name}/papers/sets/{set_id}/mark")
def api_set_mark(
    name: str, set_id: str, payload: dict = Body(...), _: None = Depends(require_token)
) -> dict:
    """候補に keep / drop を付ける（body は `{"positions","decision","reason"?}`）。"""
    return sci.set_mark(papers.resolve_project(name), set_id, payload)


@app.post("/api/projects/{name}/papers/sets/{set_id}/drop-pending")
def api_set_drop_pending(
    name: str, set_id: str, payload: dict = Body(default={}), _: None = Depends(require_token)
) -> dict:
    """未判定の候補をまとめて drop にする（body は `{"reason"?}`）。"""
    return sci.set_drop_pending(papers.resolve_project(name), set_id, payload)


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


# --- MLflow 中継（U03） ---
# Mac の MLflow（`SAI_MLFLOW_URL`・既定 127.0.0.1:5001）への読み取りだけの中継。
# 合言葉（require_token）で守り、中継の待ち時間でサーバー全体を止めないよう
# スレッドプールで呼ぶ。MLflow がエラーを返したときは本文と状態コードを
# そのまま返すので、MlflowError を Response に変える。


@app.exception_handler(mlflow_proxy.MlflowError)
async def _mlflow_error(request: Request, exc: mlflow_proxy.MlflowError) -> Response:
    """MLflow の本文と状態コードをそのまま返す（理由の文言を変えない）。"""
    return Response(content=exc.body, status_code=exc.status_code)


@app.get("/api/mlflow/experiments")
async def api_mlflow_experiments(
    project: str | None = None, _: None = Depends(require_token)
) -> dict:
    """実験の一覧（`POST experiments/search`）。`project` で experiment のタグ `sai.project` を絞る。"""
    return await run_in_threadpool(mlflow_proxy.experiments, project or None)


@app.get("/api/mlflow/experiments/{experiment_id}")
async def api_mlflow_experiment(
    experiment_id: str, _: None = Depends(require_token)
) -> dict:
    """実験 1 件（`GET experiments/get`）。"""
    return await run_in_threadpool(mlflow_proxy.experiment, experiment_id)


@app.post("/api/mlflow/runs/search")
async def api_mlflow_runs_search(
    payload: dict = Body(...), _: None = Depends(require_token)
) -> dict:
    """実行の検索（`POST runs/search`・本文は C-03 の形）。"""
    return await run_in_threadpool(mlflow_proxy.runs_search, payload)


@app.get("/api/mlflow/runs/{run_id}")
async def api_mlflow_run(run_id: str, _: None = Depends(require_token)) -> dict:
    """実行 1 件（`GET runs/get`）。"""
    return await run_in_threadpool(mlflow_proxy.run, run_id)


@app.get("/api/mlflow/metrics/history")
async def api_mlflow_metric_history(
    run_id: str,
    metric_key: str,
    max_results: int | None = Query(default=None),
    _: None = Depends(require_token),
) -> dict:
    """指標 1 本の履歴（`GET metrics/get-history`）。"""
    return await run_in_threadpool(
        mlflow_proxy.metric_history, run_id, metric_key, max_results
    )


@app.get("/api/mlflow/metrics/bulk")
async def api_mlflow_metric_bulk(
    run_ids: list[str] = Query(default=[]),
    metric_key: str = "",
    max_results: int | None = Query(default=None),
    _: None = Depends(require_token),
) -> dict:
    """指標 1 本の履歴をまとめて取る（`GET metrics/get-history-bulk-interval`）。"""
    return await run_in_threadpool(
        mlflow_proxy.metric_bulk, run_ids, metric_key, max_results
    )


@app.get("/api/mlflow/artifacts")
async def api_mlflow_artifacts(
    run_id: str, path: str = "", _: None = Depends(require_token)
) -> dict:
    """成果物の一覧（`GET artifacts/list`）。"""
    return await run_in_threadpool(mlflow_proxy.artifacts, run_id, path or None)


@app.get("/api/mlflow/artifact")
async def api_mlflow_artifact(
    run_id: str, path: str, _: None = Depends(require_token)
) -> Response:
    """成果物 1 件の本文（`GET /get-artifact`・5MB を超えたら 413）。"""
    return await run_in_threadpool(mlflow_proxy.artifact, run_id, path)


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
