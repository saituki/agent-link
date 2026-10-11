"""データの API（共有契約 C3）。"""

from __future__ import annotations

from fastapi import APIRouter

from . import dataset, spectrum

router = APIRouter()


@router.get("/api/datasets")
def list_datasets():
    return {"datasets": dataset.list_datasets()}


@router.get("/api/datasets/{ds}/records/{rec}/meta")
def meta(ds: str, rec: str):
    return dataset.load_meta(ds, rec)


@router.get("/api/datasets/{ds}/records/{rec}/m4")
def m4(ds: str, rec: str, vars: str, t0: float, t1: float, px: int):
    return dataset.m4(ds, rec, vars.split(","), t0, t1, px)


@router.get("/api/datasets/{ds}/records/{rec}/spec")
def spec(ds: str, rec: str, var: str, t0: float, t1: float, cols: int,
         fmin: float, fmax: float, win_ms: float):
    return spectrum.spectrogram(ds, rec, var, t0, t1, cols, fmin, fmax, win_ms)
