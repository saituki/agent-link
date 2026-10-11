"""ch 設定（channels.json・共有契約 G1-C1）。並び・label・色・倍率・スペクトルを置く。表示と非表示は view.json の vars。"""

from __future__ import annotations

import json

from . import paths
from .events import COLOR_RE, is_num, write_json

FILE = "channels.json"
MAX_CHANNELS = 512
MAX_LABEL = 64
MAX_GAIN = 1000
MAX_WIN_MS = 10000
_TOP = ("version", "channels")
_ITEM = ("name", "label", "color", "gain", "spectrum")
_SPEC = ("on", "fmin", "fmax", "win_ms")


def load_channels(ds: str) -> dict:
    p = paths.dataset_dir(ds) / FILE
    if not p.is_file():
        return {"version": 1, "channels": []}
    return check_channels(json.loads(p.read_text(encoding="utf-8")))


def save_channels(ds: str, body: dict) -> dict:
    new = check_channels(body)
    write_json(paths.dataset_dir(ds) / FILE, new)
    return new


def _spectrum(s, n: str) -> dict:
    if not isinstance(s, dict):
        raise ValueError(f"spectrum: object にしてください: {n}")
    for k in s:
        if k not in _SPEC:
            raise ValueError(f"spectrum.{k} は spectrum の項目ではありません")
    if not isinstance(s.get("on"), bool):
        raise ValueError(f"spectrum.on: true か false にしてください: {n}")
    fmin, fmax, win = s.get("fmin"), s.get("fmax"), s.get("win_ms")
    if not is_num(fmin) or fmin < 0:
        raise ValueError(f"spectrum.fmin: 0 以上の数にしてください: {n}")
    if not is_num(fmax) or fmax <= fmin:
        raise ValueError(f"spectrum.fmax: fmin より大きい数にしてください: {n}")
    if not is_num(win) or not 0 < win <= MAX_WIN_MS:
        raise ValueError(f"spectrum.win_ms: 0 より大きく {MAX_WIN_MS} 以下の数にしてください: {n}")
    return {"on": s["on"], "fmin": fmin, "fmax": fmax, "win_ms": win}


def _item(c, seen: set) -> dict:
    if not isinstance(c, dict):
        raise ValueError("channels: object にしてください")
    for k in c:
        if k not in _ITEM:
            raise ValueError(f"{k} は channels の項目ではありません")
    n = c.get("name")
    if not paths.valid_name(n):
        raise ValueError(f"name: 名前の形が違います: {n}")
    if n in seen:
        raise ValueError(f"name: 重なっています: {n}")
    seen.add(n)
    label = c.get("label", "")
    if not isinstance(label, str) or len(label) > MAX_LABEL:
        raise ValueError(f"label: 文字列にしてください（{MAX_LABEL} 字まで）: {n}")
    color = c.get("color", "")
    if not isinstance(color, str) or (color and not COLOR_RE.fullmatch(color)):
        raise ValueError(f"color: #RRGGBB の形にしてください（空も可）: {n}")
    gain = c.get("gain", 1)
    if not is_num(gain) or not 0 < gain <= MAX_GAIN:
        raise ValueError(f"gain: 0 より大きく {MAX_GAIN} 以下の数にしてください: {n}")
    out = {"name": n, "label": label, "color": color, "gain": gain}
    if "spectrum" in c:
        out["spectrum"] = _spectrum(c["spectrum"], n)
    return out


def check_channels(body) -> dict:
    if not isinstance(body, dict):
        raise ValueError("本文は object にしてください")
    for k in body:
        if k not in _TOP:
            raise ValueError(f"{k} は channels の項目ではありません")
    if body.get("version", 1) != 1:
        raise ValueError("version: 1 にしてください")
    chs = body.get("channels", [])
    if not isinstance(chs, list):
        raise ValueError("channels: 配列にしてください")
    if len(chs) > MAX_CHANNELS:
        raise ValueError(f"channels: {MAX_CHANNELS} 件までです")
    seen: set = set()
    return {"version": 1, "channels": [_item(c, seen) for c in chs]}
