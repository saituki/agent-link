"""チャットの HTTP（C11）。sai-link の失敗を HTTP の状態に直す。"""

from fastapi import APIRouter, Body, HTTPException

from . import chat, linkcli

router = APIRouter()


def _call(fn, *args):
    try:
        return fn(*args)
    except linkcli.LinkError as e:
        raise HTTPException(status_code=e.status, detail=e.detail)


@router.post("/api/datasets/{ds}/chat/start")
def start(ds: str):
    return _call(chat.start, ds)


@router.post("/api/datasets/{ds}/chat/send")
def send(ds: str, body: dict = Body(...)):
    return _call(chat.send, ds, body.get("text"))


@router.get("/api/datasets/{ds}/chat/messages")
def messages(ds: str):
    return _call(chat.messages, ds)
