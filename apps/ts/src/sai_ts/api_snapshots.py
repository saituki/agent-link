"""スナップショットの API（G3-C2）。HTTP から来た書き手は human。"""

from __future__ import annotations

from fastapi import APIRouter, HTTPException

from . import snapshots

router = APIRouter()


@router.get("/api/snapshots")
def list_all():
    return {"snapshots": snapshots.list_snapshots()}


@router.post("/api/datasets/{ds}/snapshots", status_code=201)
def create(ds: str, body: dict):
    try:
        return snapshots.create(ds, body.get("name"), body.get("records"), body.get("vars"),
                                body.get("categories"), "human")
    except FileExistsError as e:
        raise HTTPException(409, str(e))


@router.get("/api/snapshots/{name}")
def show(name: str):
    return snapshots.show(name)


@router.post("/api/snapshots/{name}/link")
def link(name: str, body: dict):
    try:
        return snapshots.link(name, body.get("node"), body.get("text"), "human")
    except snapshots.SciError as e:
        raise HTTPException(e.status, e.detail)
