"""スナップショット（G3-C2）。変わらない一式を ts/snapshots/<name>/ に書き、research-tree の S か E に結ぶ。

作ったあとは中身を変えない（同じ名前は作り直さない）。結んだ記録だけ links.json に足す。
"""

from __future__ import annotations

import hashlib
import json
import os
import re
import secrets
import shutil
import subprocess
from pathlib import Path

from . import dataset, events, paths, runs

MANIFEST = "manifest.json"
LINKS = "links.json"
NODE_RE = re.compile(r"[SE][1-9][0-9]{0,8}")
SOURCE_SCRIPT_RE = re.compile(r"script:([^@]+)@([0-9a-f]{8})")
SCI_TIMEOUT = 60
_STATUS = {2: 400, 3: 404, 4: 409}  # sai-sci の終了コード → HTTP の状態（他は 502）


class SciError(Exception):
    def __init__(self, status: int, detail: str):
        super().__init__(detail)
        self.status = status
        self.detail = detail


def _strs(v, what: str) -> list[str]:
    if v is None:
        return []
    if not isinstance(v, list) or not all(isinstance(x, str) for x in v):
        raise ValueError(f"{what} は文字の配列にしてください")
    return list(dict.fromkeys(v))


def _read(path: Path):
    return json.loads(path.read_text(encoding="utf-8"))


def _links(d: Path) -> list[dict]:
    return _read(d / LINKS)["links"] if (d / LINKS).is_file() else []


def _dir(name: str) -> Path:
    paths.check_name(name)
    p = paths.inside(paths.snapshots_dir() / name)
    if not p.is_dir():
        raise KeyError(f"スナップショットがありません: {name}")
    return p


def _sha_file(path: Path) -> str:
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def _sha_files(files: dict) -> str:
    return hashlib.sha256(json.dumps(files, sort_keys=True, ensure_ascii=False).encode("utf-8")).hexdigest()


def _place(src: Path, dst: Path) -> None:
    # 元のファイルは os.replace で置き換わるので、hard link の中身は変わらない
    try:
        os.link(src, dst)
    except OSError:
        shutil.copy2(src, dst)


def create(ds: str, name: str, records, vars, categories, by: str) -> dict:
    paths.check_name(name)
    recs_in = _strs(records, "records")
    vars_in = _strs(vars, "vars")
    cats_in = _strs(categories, "categories")
    root = paths.snapshots_dir()
    dst = paths.inside(root / name)
    if dst.exists():
        raise FileExistsError(f"同じ名前のスナップショットがあります: {name}")

    src = paths.dataset_dir(ds)
    rdir = src / "records"
    all_recs = sorted(p.name for p in rdir.iterdir() if p.is_dir()) if rdir.is_dir() else []
    recs = recs_in or all_recs
    for r in recs:
        paths.record_dir(ds, r)  # 名前と存在を確かめる

    root.mkdir(parents=True, exist_ok=True)
    tmp = root / f".tmp-{name}-{secrets.token_hex(4)}"
    tmp.mkdir()
    try:
        # 記録と ch
        vars_meta: list[dict] = []
        seen: set[str] = set()
        ch_names: set[str] = set()
        for rec in recs:
            meta = dataset.load_meta(ds, rec)
            have = {v["name"]: v for v in meta["vars"]}
            names = vars_in or [v["name"] for v in meta["vars"]]
            for n in names:
                if n not in have:
                    raise ValueError(f"記録 {rec} に変数 {n} がありません")
            keep = [have[n] for n in names]
            rdst = tmp / "records" / rec
            rdst.mkdir(parents=True)
            for v in keep:
                f = paths.inside(src / "records" / rec / v["file"])
                if not f.is_file():
                    raise ValueError(f"変数のファイルがありません: {rec}/{v['name']}")
                _place(f, rdst / v["file"])
                ch_names.add(v["name"])
                if v["name"] not in seen:
                    seen.add(v["name"])
                    vars_meta.append({"name": v["name"], "unit": v.get("unit", ""), "source": v.get("source"),
                                      "derived": v.get("derived")})
            body = {**meta, "vars": keep}
            (rdst / "meta.json").write_text(json.dumps(body, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")

        # イベント（入れた記録とカテゴリ。入れていない ch を含むものは除く）
        schema = events.load_schema(ds)
        cats = cats_in or [c["name"] for c in schema["categories"]]
        kept, skipped = [], 0
        for e in events.list_events(ds):
            if e["record"] not in recs or (cats_in and e["category"] not in cats_in):
                continue
            if not set(e.get("vars") or []) <= ch_names:
                skipped += 1
                continue
            kept.append(e)
        (tmp / "events.jsonl").write_text("".join(json.dumps(e, ensure_ascii=False) + "\n" for e in kept),
                                          encoding="utf-8")
        out_schema = {"version": 1,
                      "categories": [c for c in schema["categories"] if not cats_in or c["name"] in cats_in],
                      "metrics": schema["metrics"]}
        (tmp / "schema.json").write_text(json.dumps(out_schema, ensure_ascii=False, indent=2) + "\n",
                                         encoding="utf-8")

        # 実行の記録（派生変数の run と、イベントの source の script・sha8）
        run_ids = {v["derived"]["run"] for v in vars_meta if v.get("derived") and v["derived"].get("run")}
        sources = sorted({e["source"] for e in kept if e.get("source")})
        pairs = {m.groups() for s in sources if (m := SOURCE_SCRIPT_RE.fullmatch(s))}
        run_rows = [r for r in runs.list_runs(ds, limit=100000)
                    if r["id"] in run_ids or (r.get("algo"), (r.get("sha256") or "")[:8]) in pairs]

        files = {}
        for p in sorted(tmp.rglob("*")):
            rel = p.relative_to(tmp).as_posix()
            if p.is_file() and rel not in (MANIFEST, LINKS):
                files[rel] = _sha_file(p)
        sha = _sha_files(files)
        manifest = {"version": 1, "name": name, "ref": f"ts:{name}@{sha[:12]}", "sha256": sha,
                    "created_at": events.now(), "created_by": by, "dataset": ds, "records": recs,
                    "vars": vars_meta,
                    "events": {"categories": cats, "count": len(kept),
                               "sources": [s for s in sources if s]},
                    "runs": run_rows, "files": files}
        events.write_json(tmp / MANIFEST, manifest)
        os.rename(tmp, dst)
    except BaseException:
        shutil.rmtree(tmp, ignore_errors=True)
        raise
    return {**manifest, "skipped_events": skipped}


def list_snapshots() -> list[dict]:
    root = paths.snapshots_dir()
    out = []
    for d in sorted(root.iterdir()) if root.is_dir() else []:
        if not d.is_dir() or not paths.valid_name(d.name) or not (d / MANIFEST).is_file():
            continue
        m = _read(d / MANIFEST)
        out.append({"name": m["name"], "ref": m["ref"], "dataset": m["dataset"], "records": m["records"],
                    "vars": [v["name"] for v in m["vars"]], "events_count": m["events"]["count"],
                    "created_at": m["created_at"], "links": _links(d)})
    out.sort(key=lambda s: (s["created_at"], s["name"]), reverse=True)
    return out


def show(name: str) -> dict:
    d = _dir(name)
    return {"manifest": _read(d / MANIFEST), "links": _links(d)}


def _sci(op: dict) -> dict:
    root = paths.project_root()
    env = {**os.environ, "SAI_PROJECT_ROOT": str(root)}
    try:
        p = subprocess.run([os.environ.get("SAI_SCI_BIN", "sai-sci"), "apply", "--json"],
                           input=json.dumps(op, ensure_ascii=False), capture_output=True, text=True,
                           timeout=SCI_TIMEOUT, env=env, cwd=root)
    except FileNotFoundError:
        raise SciError(502, "sai-sci が見つかりません")
    except subprocess.TimeoutExpired:
        raise SciError(502, "sai-sci の時間切れです")
    if p.returncode != 0:
        lines = [ln for ln in p.stderr.splitlines() if ln.strip()]
        detail = lines[-1] if lines else f"sai-sci が終了コード {p.returncode} で失敗しました"
        raise SciError(_STATUS.get(p.returncode, 502), detail)
    try:
        return json.loads(p.stdout)
    except ValueError:
        raise SciError(502, "sai-sci の出力が JSON ではありません")


def link(name: str, node: str, text: str | None, by: str) -> dict:
    d = _dir(name)
    if not isinstance(node, str) or not NODE_RE.fullmatch(node):
        raise ValueError("node は S か E の ID にしてください（例 S3・E12）")
    ref = _read(d / MANIFEST)["ref"]
    if node[0] == "S":
        op = {"op": "add_e", "parent": node, "text": text or f"データセット {name}", "dataset": ref}
    else:
        op = {"op": "set_dataset", "node": node, "dataset": ref}
    e_id = str(_sci(op).get("result") or "")
    if not NODE_RE.fullmatch(e_id) or e_id[0] != "E":
        raise SciError(502, f"sai-sci の結果が E の ID ではありません: {e_id}")
    # ponytail: 同時に結ぶと後の書き込みが勝つ。結ぶ人が増えたら flock を足す
    links = _links(d) + [{"node": e_id, "op": op["op"], "at": events.now(), "by": by}]
    events.write_json(d / LINKS, {"links": links})
    return {"node": e_id, "op": op["op"], "ref": ref}
