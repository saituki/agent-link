"""拡張の HTTP（C8）。配信は Cache-Control: no-store（版が変わったら agent が読み直す）。"""

from fastapi import APIRouter, Body, HTTPException
from fastapi.responses import FileResponse

from . import ext

router = APIRouter()


@router.get("/api/ext")
def list_exts():
    return {"exts": ext.list_exts()}


@router.get("/ext/{name}/{path:path}")
def asset(name: str, path: str):
    p, media = ext.asset(name, path)
    return FileResponse(p, media_type=media, headers={"Cache-Control": "no-store"})


@router.post("/api/ext/{name}/call/{fn}")
def call(name: str, fn: str, body: dict = Body(default={})):
    try:
        return ext.call(name, fn, body)
    except ext.ExtError as e:
        raise HTTPException(status_code=e.status, detail=e.detail)
