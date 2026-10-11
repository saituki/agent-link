"""イベント・スキーマ・画面の状態の API（C4・C6）。HTTP から来た変更の書き手は human。"""

from __future__ import annotations

from fastapi import APIRouter
from fastapi.responses import PlainTextResponse

from . import channels, events, view

router = APIRouter()


@router.get("/api/datasets/{ds}/events")
def list_events(ds: str, record: str | None = None, category: str | None = None,
                type: str | None = None):
    evs = events.list_events(ds, record, category, type)
    return {"events": evs, "count": len(evs)}


@router.post("/api/datasets/{ds}/events")
def add_event(ds: str, body: dict):
    return events.add_event(ds, body, "human")


@router.patch("/api/datasets/{ds}/events/{id}")
def update_event(ds: str, id: str, body: dict):
    return events.update_event(ds, id, body, "human")


@router.delete("/api/datasets/{ds}/events/{id}")
def delete_event(ds: str, id: str):
    events.delete_event(ds, id, "human")
    return {"id": id, "deleted": True}


@router.get("/api/datasets/{ds}/schema")
def get_schema(ds: str):
    return events.load_schema(ds)


@router.put("/api/datasets/{ds}/schema")
def put_schema(ds: str, body: dict):
    return events.save_schema(ds, body)


@router.get("/api/datasets/{ds}/channels")
def get_channels(ds: str):
    return channels.load_channels(ds)


@router.put("/api/datasets/{ds}/channels")
def put_channels(ds: str, body: dict):
    return channels.save_channels(ds, body)


@router.get("/api/datasets/{ds}/events.tsv")
def export_tsv(ds: str):
    return PlainTextResponse(
        events.export_tsv(ds),
        media_type="text/tab-separated-values",
        headers={"Content-Disposition": f'attachment; filename="{ds}-events.tsv"'},
    )


@router.get("/api/view")
def get_view():
    return view.load_view()


@router.put("/api/view")
def put_view(body: dict):
    return view.save_view(body, "human")
