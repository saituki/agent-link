"""sai-radio 本体サーバー (Mac 側)．sai-phone から中継で使われる裏方サーバー．

- 登録済み論文の一覧・音声配信
- 質問(録音)を受け取り，回答音声を返す
- ログ(いつ・どの論文を聞いたか／質問したか)の確認
- 新規論文の登録(sai-paper連携・CLI)

画面は持たない(画面は sai-phone の Papers タブ → Radio)．
"""

from __future__ import annotations

import mimetypes
from pathlib import Path

from fastapi import Depends, FastAPI, HTTPException, Query
from pydantic import BaseModel
from fastapi.responses import FileResponse
from fastapi.security import HTTPAuthorizationCredentials, HTTPBearer

from sai_radio import ingest, live, llm
from sai_radio.config import AUDIO_DIR, settings
from sai_radio.store import add_event, find_section, get_paper, init_db, list_events, list_papers, set_favorite, stats_by_day

# .m4a は環境によって audio/mp4a-latm などに解決されることがある．
# iPhone の Safari で確実に再生できるよう audio/mp4 に統一する．
# (実測: このMacのPythonでは audio/mp4a-latm と判定されたため追加)
mimetypes.add_type("audio/mp4", ".m4a")

app = FastAPI(title="sai-radio")

_bearer = HTTPBearer(auto_error=False)


def require_token(
    creds: HTTPAuthorizationCredentials | None = Depends(_bearer),
    token: str | None = Query(default=None),
) -> None:
    """簡易トークン認証．SAI_RADIO_TOKEN が未設定なら認証なし(開発用)．

    なぜクエリ(?token=)も受け付けるか: 音声再生の <audio> タグは
    Authorization ヘッダを付けられない．将来トークンを設定しても
    音声だけ401エラーにならないようにするため．
    """
    if not settings.api_token:
        return
    if (creds is not None and creds.credentials == settings.api_token) or token == settings.api_token:
        return
    raise HTTPException(status_code=401, detail="認証エラー")


@app.on_event("startup")
def _startup() -> None:
    init_db()


@app.get("/api/papers")
def api_list_papers(_: None = Depends(require_token)) -> list[dict]:
    return [
        {
            "id": p.id,
            "title": p.title,
            "mode": p.mode,
            "created_at": p.created_at,
            "favorite": p.favorite,
            "duration": p.sections[-1].end if p.sections and p.sections[-1].end else None,
        }
        for p in list_papers()
    ]


@app.get("/api/papers/{paper_id}")
def api_get_paper(paper_id: str, _: None = Depends(require_token)) -> dict:
    paper = get_paper(paper_id)
    if paper is None:
        raise HTTPException(status_code=404, detail="論文が見つかりません")
    return {
        "id": paper.id,
        "title": paper.title,
        "mode": paper.mode,
        "created_at": paper.created_at,
        "favorite": paper.favorite,
        "sections": [
            {"start": s.start, "end": s.end, "title": s.title} for s in paper.sections
        ],
    }


@app.get("/api/papers/{paper_id}/audio")
def api_get_audio(paper_id: str, _: None = Depends(require_token)) -> FileResponse:
    paper = get_paper(paper_id)
    if paper is None:
        raise HTTPException(status_code=404, detail="論文が見つかりません")
    # sai-paper の論文フォルダに書いた音声は絶対パス，古い音声は AUDIO_DIR からの相対パス．
    audio_path = Path(paper.audio_path)
    if not audio_path.is_absolute():
        audio_path = AUDIO_DIR / audio_path
    if not audio_path.exists():
        raise HTTPException(status_code=404, detail="音声ファイルが見つかりません")
    media_type = mimetypes.guess_type(audio_path.name)[0] or "application/octet-stream"
    return FileResponse(audio_path, media_type=media_type)


@app.post("/api/papers/{paper_id}/listen")
def api_listen(paper_id: str, _: None = Depends(require_token)) -> dict:
    if get_paper(paper_id) is None:
        raise HTTPException(status_code=404, detail="論文が見つかりません")
    add_event(paper_id, "listen", {})
    return {"ok": True}


# 記録を一度に返す件数の上限（sai-paper の Dashboard が全プロジェクト分をまとめて読むため）
LOGS_LIMIT_MAX = 5000


@app.get("/api/logs")
def api_logs(
    paper_id: str | None = None, limit: int = 200, _: None = Depends(require_token)
) -> list[dict]:
    # 件数は 1〜LOGS_LIMIT_MAX に丸める（大きすぎる指定で固まらないように）
    return list_events(paper_id, limit=max(1, min(limit, LOGS_LIMIT_MAX)))


class LiveSessionRequest(BaseModel):
    """質問開始時にiPhoneから送られる，再生位置の秒数．

    どのセクションについて質問しているかを知るために使う．
    """
    position_sec: float


class QuestionLogRequest(BaseModel):
    """質問が終わった後にiPhoneから送られる，聞き取り結果と回答文．

    Live API の inputTranscription / outputTranscription で得たテキストを
    そのまま送ってもらう．既存のログ画面がこれまで通り動くようにする．
    """
    position_sec: float
    question: str
    answer: str


class AskRequest(BaseModel):
    """テキスト質問用．再生位置と質問文を送る．"""
    question: str
    position_sec: float


@app.post("/api/papers/{paper_id}/favorite")
def api_toggle_favorite(paper_id: str, _: None = Depends(require_token)) -> dict:
    """お気に入り(星)のON/OFFを切り替える．今の状態を反転させる．"""
    paper = get_paper(paper_id)
    if paper is None:
        raise HTTPException(status_code=404, detail="論文が見つかりません")
    new_value = not paper.favorite
    set_favorite(paper_id, new_value)
    return {"favorite": new_value}


@app.post("/api/papers/{paper_id}/ask")
def api_ask(paper_id: str, req: AskRequest, _: None = Depends(require_token)) -> dict:
    """テキストで質問する．再生位置のセクション本文をもとにLLMが答える．

    少し時間がかかるので，終わるまで待つ(アップロードと同じ方式)．
    """
    paper = get_paper(paper_id)
    if paper is None:
        raise HTTPException(status_code=404, detail="論文が見つかりません")
    section = find_section(paper_id, req.position_sec)
    if section is None:
        raise HTTPException(status_code=400, detail="対応するセクションが見つかりません")
    try:
        answer_text = llm.answer(section.source_text, req.question)
    except Exception as e:
        raise HTTPException(status_code=500, detail=f"エラーが発生しました: {str(e)}")
    add_event(
        paper_id,
        "question",
        {"position_sec": req.position_sec, "question": req.question, "answer": answer_text},
    )
    return {"answer": answer_text, "section_title": section.title}


@app.get("/api/stats")
def api_stats(_: None = Depends(require_token)) -> dict:
    """ホーム画面用．登録数・聴いた論文数・日ごとの再生回数を返す．"""
    papers = list_papers()
    listened_ids = set()
    for e in list_events(limit=100000):
        if e["kind"] == "listen":
            listened_ids.add(e["paper_id"])
    return {
        "total_papers": len(papers),
        "listened_papers": len(listened_ids),
        "by_day": stats_by_day(),
    }


@app.post("/api/papers/{paper_id}/live-session")
def api_live_session(paper_id: str, req: LiveSessionRequest, _: None = Depends(require_token)) -> dict:
    """Gemini Live API への直結に必要な情報(合鍵+論文本文)を返す．

    処理の流れ:
    再生位置から該当セクションを逆引き → 使い捨てトークンを発行 →
    「指示文 + セクション本文」を添えて返す．
    iPhone はこの応答を使って Google に直接 WebSocket 接続する．
    """
    paper = get_paper(paper_id)
    if paper is None:
        raise HTTPException(status_code=404, detail="論文が見つかりません")

    section = find_section(paper_id, req.position_sec)
    if section is None:
        raise HTTPException(status_code=400, detail="対応するセクションが見つかりません")

    try:
        token = live.issue_ephemeral_token()
    except live.LiveError as e:
        raise HTTPException(status_code=500, detail=str(e))

    # 音声で答える前提の指示文．llm.answer() のプロンプトと同じ構成にし，
    # 「話し言葉で・短く」の一文を加えている．
    system_instruction = (
        "あなたは英語論文を紹介するラジオ番組のアシスタントです．"
        "次は論文の該当箇所です．\n---\n"
        f"{section.source_text}\n---\n"
        "この内容を踏まえて，リスナーの質問に日本語で答えてください．"
        "音声で聞く前提なので，話し言葉で，1〜3文程度で簡潔に答えてください．"
    )
    return {
        "token": token,
        "model": settings.gemini_live_model,
        # 接続先はサーバー側から返す．
        # モデルやAPIバージョンを変えるときに2箇所直さなくて済むようにするため．
        # (メソッド名は BidiGenerateContentConstrained 固定．検証済みなので変えないこと)
        "ws_url": (
            "wss://generativelanguage.googleapis.com/ws/"
            "google.ai.generativelanguage.v1alpha.GenerativeService.BidiGenerateContentConstrained"
        ),
        "system_instruction": system_instruction,
        "section_title": section.title,
    }


@app.post("/api/papers/{paper_id}/question-log")
def api_question_log(paper_id: str, req: QuestionLogRequest, _: None = Depends(require_token)) -> dict:
    """Live API での質問・回答のテキストをログに記録するだけのエンドポイント．

    音声の送受信は iPhone と Google の間で直接行われるので，
    Mac 側は終わった後にテキストだけを受け取って保存する．
    """
    if get_paper(paper_id) is None:
        raise HTTPException(status_code=404, detail="論文が見つかりません")
    add_event(
        paper_id,
        "question",
        {"position_sec": req.position_sec, "question": req.question, "answer": req.answer},
    )
    return {"ok": True}


class FromExtractRequest(BaseModel):
    """sai-paper から受け取る，抽出済みMarkdownを含むリクエスト．"""
    paper_id: str
    title: str
    markdown: str
    mode: str = "podcast"
    # 原稿と音声の設定(sai-paper の settings.json の radio.script / radio.voice)．省略時は config/*.json．
    settings: dict | None = None
    # 音声を書く場所(論文フォルダの絶対パス)．省略時は data/audio/．
    audio_dir: str | None = None


@app.post("/api/papers/from-extract", status_code=201)
async def api_from_extract(
    req: FromExtractRequest,
    _: None = Depends(require_token),
) -> dict:
    """sai-paper から：抽出済みMarkdown + paper_id/title を受け取り，ポッドキャスト音声を生成する．

    sai-paper の `r` キーと sai-phone の「音声を作る」の両方がこの窓口を使う．
    処理には数分かかる．同期的に応答を待つ．
    """
    if req.mode != "podcast":
        raise HTTPException(status_code=400, detail="from-extract は mode=podcast のみ対応しています")

    try:
        paper = await ingest.ingest_from_markdown(
            req.markdown,
            "podcast",
            req.title,
            paper_id=req.paper_id,
            request_settings=req.settings,
            audio_dir=req.audio_dir,
        )
    except Exception as e:
        raise HTTPException(status_code=500, detail=f"エラーが発生しました: {str(e)}")

    return {"paper_id": paper.id, "title": paper.title, "mode": paper.mode}

# 画面は持たない(11-cleanup §3 A)．画面は sai-phone (Papers タブ → Radio) が持つ，
# GET / は 404 になる．
