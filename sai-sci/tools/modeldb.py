#!/usr/bin/env python3
"""計算神経科学の公開モデル DB **ModelDB** のカタログを、手元の jsonl に写す道具。

研究の前に「既にあるモデルを探したい」「この論文にコードはあるか」を、**ネットを使わずに**
引けるようにするのが目的。カタログは 1 行 1 モデルの jsonl で、
`<sai_home>/refcode/modeldb/catalog.jsonl` に置く（`<sai_home>` は環境変数 `SAI_SAI_HOME`、
無ければ `~/.sai-moon`）。モデルごとに、説明・分類（脳領域・細胞種・電流・受容体・概念・
型）・論文（タイトル・著者・DOI・PubMed ID・アブストラクト）・ソースコードの URL を入れる。

呼び方（標準ライブラリだけ。Python 3.11 以上）:
    uv run --no-project --python ">=3.11" python "$SAI_PROJECT_ROOT/.sai/tools/modeldb.py" \
        sync [--limit N] [--ids 1,2,3] [--workers 4] [--json]   # カタログを nets から取る（差分更新）
        search [<語>...] [--doi D] [--region R] [--simulator S] [--celltype C] [--concept K] \
               [--limit 20] [--json]                              # カタログだけを読む（ネットを使わない）
        show <model_id> [--json]                                  # 1 件のレコードを出す
        status [--json]                                           # カタログの具合を出す

差分更新の考え方:
    - 取り直すのは、カタログに無い ID・`ver_number` が違う ID・前回 `error` が null でない ID。
    - `--ids` を渡すと、その ID だけを取る（`ver_number` が同じでも取り直す）。
    - `--limit` は、取り直す ID を昇順に並べた先頭 N 件に絞る。
    - サーバに無い ID は、カタログから外して `removed` に数える。

中断と書き込み:
    - 書き込みは全体を `catalog.jsonl.tmp` に書いて `os.replace` で入れ替える。途中で止めても、
      ファイルは常に前回か今回のどちらかの完全な jsonl になる（壊れた行を残さない）。
    - `Ctrl-C`（`KeyboardInterrupt`）を受けたら、新しい仕事を出さずに、終わった分だけ書き込み、
      終了コード 130 で終える。もう一度 `sync` すると続きから取る。
    - 1 モデルが失敗しても全体は止めない。失敗の理由はそのレコードの `error` に入る。
      記録が壊れた状態で `sync` が終わることはないので、終了コードは 0 のまま（失敗は `failed` で数える）。

アブストラクトの取り方:
    - ModelDB 側に abstract は無いので、論文の PubMed ID を 200 本ずつ PubMed（E-utilities）に
      問い合わせる。取れなかった論文で DOI があるものは、Crossref を 1 本ずつ問い合わせる。
      どちらでも取れなければ `abstract: null`・`abstract_source: "none"`。空文字は入れない。
    - 前回のカタログに同じ論文 ID のアブストラクトがあり、モデルの `ver_number` が変わっていなければ、
      それは网络的問い合わせなしに写して使う。

ネットの守り:
    - HTTP はすべて、このモジュールの関数 `http_get(url, timeout) -> (status, bytes)` 1 か所を通す。
      テストはここを偽物に差し替える。
    - `User-Agent: sai-moon-modeldb/1 (+https://modeldb.science/api)`。1 回の上限は 30 秒。
    - ホストごとに、要求の間を最低 0.3 秒あける（定数 `MIN_INTERVAL_S`。テストでは 0 にする）。
    - 再試行は 429・5xx・接続の失敗だけ（最大 3 回、待ちは 1・2・4 秒＝定数 `RETRY_SLEEPS`）。404 は再試行しない。
    - パスが `/download` か `/getModelFile` で始まるもの、`/search`・`/login` は、入口で例外を出して拒む。
      ソースコード本体の取得はしない（`refcode.py fetch modeldb:<id>` が受け持つ）。`/search` と
      `/login` は `robots.txt` で禁止されている。

終了コード: 0 = 成功、1 = 失敗（理由は標準エラー）、2 = 引数の誤り、130 = `sync` の中断。
`--json` のとき、標準出力は JSON 1 つだけ。
"""

from __future__ import annotations

import argparse
import concurrent.futures
import io
import json
import os
import re
import sys
import threading
import time
import urllib.error
import urllib.parse
import urllib.request
import xml.etree.ElementTree as ET
from datetime import datetime, timezone
from pathlib import Path

# ---- 場所と識別子 ------------------------------------------------------------------------
API_BASE = "https://modeldb.science/api/v1"
WEB_BASE = "https://modeldb.science"
GITHUB_BASE = "https://github.com/ModelDBRepository"
PUBMED_URL = "https://eutils.ncbi.nlm.nih.gov/entrez/eutils/efetch.fcgi"
CROSSREF_URL = "https://api.crossref.org/works"
# カタログの置き場（利用者全体で共有する。プロジェクトの .sai/ は見ない）
CATALOG_PARTS = ("refcode", "modeldb", "catalog.jsonl")

# ---- ネットの守り ----------------------------------------------------------------------
USER_AGENT = "sai-moon-modeldb/1 (+https://modeldb.science/api)"
HTTP_TIMEOUT = 30          # 1 回の要求の上限（秒）
MIN_INTERVAL_S = 0.3       # 同じホストの要求の間隔（秒）。テストでは 0 にする
MAX_RETRIES = 3            # 再試行する回数（初回は数えない。合計 4 回まで）
RETRY_SLEEPS = (1.0, 2.0, 4.0)  # 再試行の前の待ち（秒）。テストでは空にする
RETRY_STATUSES = (429, 500, 502, 503, 504)  # 再試行する応答。429 と 5xx だけ（404 はしない）
PUBMED_BATCH = 200         # PubMed への 1 回の問い合わせにまとめる論文数
# この道具では取りに行かないパス。`/download` と `/getModelFile` は本体の取得（refcode.py）の担当
BLOCKED_PATHS = ("/download", "/getModelFile", "/search", "/login")

# ファイル一覧のページにあるリンク。`&` が `&amp;` になっていることもある
FILE_HREF_RE = re.compile(r"""href=["'](/(\d+)\?tab=2&(?:amp;)?file=([^"'>]+))["']""")
DOI_URL_RE = re.compile(r"^\s*(?:https?://)?(?:dx\.)?doi\.org/", re.IGNORECASE)
TAG_RE = re.compile(r"<[^>]+>")
SPACE_RE = re.compile(r"\s+")


class ModelDBError(Exception):
    """利用者に見せる失敗（メッセージだけ出して終了コード 1）。"""


def sai_home() -> Path:
    """利用者全体の置き場（`SAI_SAI_HOME` があればそれ、無ければ `~/.sai-moon`）。"""
    value = os.environ.get("SAI_SAI_HOME")
    return Path(value) if value else Path.home() / ".sai-moon"


def catalog_path() -> Path:
    """カタログの置き場（`<sai_home>/refcode/modeldb/catalog.jsonl`）。"""
    return sai_home().joinpath(*CATALOG_PARTS)


def now_iso() -> str:
    """いまの UTC を `2026-10-03T06:00:00Z` の形で。"""
    return datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


# ==========================================================================================
# 1. HTTP（すべてここを通す。テストは関数ごと差し替える）
# ==========================================================================================

# ホストごとの「入れ物と最後に出した時刻」。スレッドをまたいで同じホストを通り抜けないようにする
_host_gates: dict[str, list] = {}
_host_gates_lock = threading.Lock()


def _host_gate(url: str) -> None:
    """同じホストへの要求を `MIN_INTERVAL_S` 以上あける。"""
    host = urllib.parse.urlsplit(url).netloc
    with _host_gates_lock:
        gate = _host_gates.get(host)
        if gate is None:
            gate = [threading.Lock(), 0.0]
            _host_gates[host] = gate
    with gate[0]:
        rest = MIN_INTERVAL_S - (time.monotonic() - gate[1])
        if rest > 0:
            time.sleep(rest)
        gate[1] = time.monotonic()


def http_get(url: str, timeout: float = HTTP_TIMEOUT) -> tuple[int, bytes]:
    """URL を取りに行く。`(status, 本文)` を返す。取りに行けない URL は例外。

    再試行するのは 429・5xx・接続の失敗だけ。最大 `MAX_RETRIES` 回まで、
    待ちは `RETRY_SLEEPS`（1・2・4 秒）。404 はそのまま返す。
    """
    path = urllib.parse.urlsplit(url).path
    for blocked in BLOCKED_PATHS:
        if path.startswith(blocked):
            raise ModelDBError(f"この道具では取りに行かない URL です（{blocked}）: {url}")
    reason = ""
    for attempt in range(MAX_RETRIES + 1):
        _host_gate(url)
        try:
            request = urllib.request.Request(url, headers={"User-Agent": USER_AGENT, "Accept": "*/*"})
            with urllib.request.urlopen(request, timeout=timeout) as response:
                return int(response.status), response.read()
        except urllib.error.HTTPError as e:
            body = b""
            try:
                body = e.read()
            except OSError:
                pass
            status = int(e.code)
            if status == 404 or status not in RETRY_STATUSES:
                return status, body
            reason = f"HTTP {status}"
        except (urllib.error.URLError, TimeoutError, OSError) as e:
            reason = str(e)
        if attempt < MAX_RETRIES and RETRY_SLEEPS:
            time.sleep(RETRY_SLEEPS[min(attempt, len(RETRY_SLEEPS) - 1)])
    raise ModelDBError(f"取りに行けませんでした（{reason}）: {url}")


def get_json(url: str, timeout: float = HTTP_TIMEOUT):
    """JSON を取りに行く。200 以外・JSON でなければ例外。"""
    status, body = http_get(url, timeout)
    if status != 200:
        raise ModelDBError(f"サーバが {status} を返しました: {url}")
    try:
        return json.loads(body.decode("utf-8"))
    except (UnicodeDecodeError, json.JSONDecodeError) as e:
        raise ModelDBError(f"返答が JSON として読めませんでした（{e}）: {url}") from e


# ==========================================================================================
# 2. カタログ（読み書き）
# ==========================================================================================

def load_catalog(path: Path | None = None) -> dict[int, dict]:
    """カタログを `{model_id: レコード}` として読む。無ければ空。読めない行は飛ばす。"""
    path = path or catalog_path()
    records: dict[int, dict] = {}
    if not path.is_file():
        return records
    try:
        text = path.read_text(encoding="utf-8")
    except OSError as e:
        raise ModelDBError(f"カタログを読めませんでした（{e}）: {path}") from e
    for line in text.splitlines():
        line = line.strip()
        if not line:
            continue
        try:
            record = json.loads(line)
        except json.JSONDecodeError:
            continue
        model_id = record.get("model_id")
        if isinstance(model_id, int):
            records[model_id] = record
    return records


def write_catalog(records: dict[int, dict], path: Path | None = None) -> Path:
    """全レコードを `model_id` の昇順で 1 行ずつ tmp に書き、`os.replace` で入れ替える。"""
    path = path or catalog_path()
    path.parent.mkdir(parents=True, exist_ok=True)
    tmp = path.with_name(path.name + ".tmp")
    with tmp.open("w", encoding="utf-8") as fh:
        for model_id in sorted(records):
            fh.write(json.dumps(records[model_id], ensure_ascii=False) + "\n")
    os.replace(tmp, path)
    return path


# ==========================================================================================
# 3. 1 モデルを取る
# ==========================================================================================

def _entries(node) -> list[dict]:
    """複数値の項目を `[{"id", "name"}]` にする。項目そのものが無いときは空。"""
    out: list[dict] = []
    if isinstance(node, str):
        return [{"id": None, "name": node}]
    if not isinstance(node, dict):
        return out
    values = node.get("value")
    if not isinstance(values, list):
        return out
    for item in values:
        if isinstance(item, dict):
            out.append({"id": item.get("object_id"), "name": item.get("object_name") or ""})
        elif isinstance(item, str):
            out.append({"id": None, "name": item})
    return out


def _names(node) -> list[str]:
    """複数値の項目から名前だけ取る。"""
    return [entry["name"] for entry in _entries(node) if entry.get("name")]


def _text(node) -> str | None:
    """`{"value": "..."}` の形から本文を取る。無ければ None。"""
    if isinstance(node, str):
        return node
    if isinstance(node, dict) and isinstance(node.get("value"), str):
        return node["value"]
    return None


def empty_record(model_id: int, ver_number, fetched_at: str) -> dict:
    """レコードの空の枠（キーは 1-b の形と同じで揃える）。"""
    return {
        "model_id": model_id,
        "model_name": "",
        "ver_number": ver_number,
        "notes": None,
        "simulators": [],
        "regions": [],
        "celltypes": [],
        "currents": [],
        "receptors": [],
        "model_concepts": [],
        "model_types": [],
        "implementers": [],
        "model_url": f"{WEB_BASE}/showmodel?model={model_id}",
        "papers": [],
        "source_zip_url": f"{WEB_BASE}/download/{model_id}",
        "source_github_url": f"{GITHUB_BASE}/{model_id}",
        "source_files_url": f"{WEB_BASE}/showmodel?model={model_id}&tab=2",
        "file_count": None,
        "fetched_at": fetched_at,
        "error": None,
    }


def empty_paper(paper_id) -> dict:
    """論文の空の枠。値が無くてもキーは必ず持つ。"""
    return {
        "paper_id": paper_id,
        "title": None,
        "authors": [],
        "journal": None,
        "year": None,
        "doi": None,
        "pubmed_id": None,
        "abstract": None,
        "abstract_source": None,
    }


def fetch_model(model_id: int, ver_number) -> dict:
    """1 モデルを取る。`models/<id>`・各論文・ファイルの一覧。失敗してもレコードは返す。"""
    record = empty_record(model_id, ver_number, now_iso())
    status, body = http_get(f"{API_BASE}/models/{model_id}")
    if status != 200:
        record["error"] = f"models/{model_id} が {status} を返しました"
        return record
    try:
        data = json.loads(body.decode("utf-8"))
    except (UnicodeDecodeError, json.JSONDecodeError) as e:
        record["error"] = f"models/{model_id} の返答が読めませんでした（{e}）"
        return record
    if not isinstance(data, dict):
        record["error"] = f"models/{model_id} の返答が模型的ではありません"
        return record

    record["model_name"] = data.get("name") or ""
    own_ver = data.get("ver_number")
    record["ver_number"] = own_ver if isinstance(own_ver, int) else ver_number
    record["notes"] = _text(data.get("notes"))
    record["simulators"] = _names(data.get("modeling_application"))
    record["regions"] = _entries(data.get("region"))
    record["celltypes"] = _entries(data.get("neurons"))
    record["currents"] = _entries(data.get("currents"))
    record["receptors"] = _entries(data.get("receptors"))
    record["model_concepts"] = _entries(data.get("model_concept"))
    record["model_types"] = _entries(data.get("model_type"))
    record["implementers"] = _names(data.get("implemented_by"))

    seen: set = set()
    for entry in _entries(data.get("model_paper")):
        paper_id = entry.get("id")
        if not isinstance(paper_id, int) or paper_id in seen:
            continue
        seen.add(paper_id)
        record["papers"].append(fetch_paper(paper_id))

    # ファイル一覧は取れなくてもモデル自体は失敗にしない
    record["file_count"] = fetch_file_count(model_id)
    return record


def fetch_paper(paper_id: int) -> dict:
    """1 本の論文を取る。`papers/<pid>`。取れなければ中身の空の枠。"""
    paper = empty_paper(paper_id)
    try:
        status, body = http_get(f"{API_BASE}/papers/{paper_id}")
    except ModelDBError:
        return paper
    if status != 200:
        return paper
    try:
        data = json.loads(body.decode("utf-8"))
    except (UnicodeDecodeError, json.JSONDecodeError):
        return paper
    if not isinstance(data, dict):
        return paper
    paper["title"] = _text(data.get("title"))
    paper["authors"] = _names(data.get("authors"))
    paper["journal"] = _text(data.get("journal"))
    paper["year"] = _text(data.get("year"))
    paper["doi"] = _text(data.get("doi"))
    paper["pubmed_id"] = _text(data.get("pubmed_id"))
    return paper


def fetch_file_count(model_id: int) -> int | None:
    """ファイル一覧のページから、ファイルのリンクの数を数える（重なりは除く）。"""
    try:
        status, body = http_get(f"{WEB_BASE}/showmodel?model={model_id}&tab=2")
    except ModelDBError:
        return None
    if status != 200:
        return None
    found = set()
    for match in FILE_HREF_RE.finditer(body.decode("utf-8", "replace")):
        path = match.group(3).replace("&amp;", "&")
        found.add((match.group(2), path))
    return len(found)


# ==========================================================================================
# 4. アブストラクト（PubMed 200 本ずつ → Crossref → なし）
# ==========================================================================================

def join_abstract_text(nodes) -> str:
    """`AbstractText` の要素を 1 つの文字列にする。

    複数あれば `Label: 本文` の形にして改行でつなぐ。本文は入れ子のタグも取り込む。
    """
    parts: list[str] = []
    for node in nodes:
        text = SPACE_RE.sub(" ", "".join(node.itertext())).strip()
        if not text:
            continue
        label = (node.get("Label") or "").strip()
        parts.append(f"{label}: {text}" if label else text)
    return "\n".join(parts)


def clean_jats(raw: str) -> str:
    """Crossref の abstract（JATS の XML）を普通の文章にする。"""
    text = (raw or "")
    for entity, char in (("&lt;", "<"), ("&gt;", ">"), ("&quot;", '"'), ("&apos;", "'"), ("&amp;", "&")):
        text = text.replace(entity, char)
    text = TAG_RE.sub(" ", text)
    text = SPACE_RE.sub(" ", text).strip()
    return re.sub(r"^abstract\s*", "", text, count=1, flags=re.IGNORECASE).strip()


def fetch_pubmed(pmids: list[str]) -> dict[str, str]:
    """PubMed ID をまとめて取り、`{pmid: アブストラクト}` を返す。"""
    if not pmids:
        return {}
    url = (f"{PUBMED_URL}?db=pubmed&id={','.join(pmids)}"
           "&rettype=abstract&retmode=xml&tool=sai-moon")
    try:
        status, body = http_get(url, 60)
    except ModelDBError:
        return {}
    if status != 200:
        return {}
    try:
        root = ET.fromstring(body)
    except ET.ParseError:
        return {}
    found: dict[str, str] = {}
    for article in root.findall(".//PubmedArticle"):
        node = article.find("./MedlineCitation/PMID")
        pmid = (node.text or "").strip() if node is not None else ""
        if not pmid:
            continue
        text = join_abstract_text(article.findall(".//Abstract/AbstractText"))
        if text:
            found[pmid] = text
    return found


def fetch_crossref(doi: str) -> str:
    """DOI の 1 本を Crossref で取って、アブストラクトの文字列を返す（無ければ空）。"""
    if not doi:
        return ""
    url = f"{CROSSREF_URL}/{urllib.parse.quote(doi, safe='')}"
    try:
        status, body = http_get(url)
    except ModelDBError:
        return ""
    if status != 200:
        return ""
    try:
        data = json.loads(body.decode("utf-8"))
    except (UnicodeDecodeError, json.JSONDecodeError):
        return ""
    message = data.get("message") if isinstance(data, dict) else None
    if not isinstance(message, dict):
        return ""
    return clean_jats(message.get("abstract") or "")


def fill_abstracts(records: dict[int, dict], previous: dict[int, dict]) -> None:
    """全レコードの論文にアブストラクトを埋める（取得済みのものは動かさない）。"""
    # 1) 前回のカタログから写す（モデルの ver_number が変わっていなければ使える）
    for model_id, record in records.items():
        before = previous.get(model_id)
        if not before or before.get("ver_number") != record.get("ver_number"):
            continue
        olds = {p.get("paper_id"): p for p in (before.get("papers") or [])}
        for paper in record.get("papers") or []:
            if paper.get("abstract"):
                continue
            old = olds.get(paper.get("paper_id"))
            if old and old.get("abstract"):
                paper["abstract"] = old["abstract"]
                paper["abstract_source"] = old.get("abstract_source") or "pubmed"

    # 2) PubMed に 200 本ずつ
    wanted: dict[str, list[dict]] = {}
    for record in records.values():
        for paper in record.get("papers") or []:
            if paper.get("abstract"):
                continue
            pmid = str(paper.get("pubmed_id") or "").strip()
            if pmid:
                wanted.setdefault(pmid, []).append(paper)
    pmids = sorted(wanted)
    for start in range(0, len(pmids), PUBMED_BATCH):
        chunk = pmids[start:start + PUBMED_BATCH]
        for pmid, text in fetch_pubmed(chunk).items():
            for paper in wanted[pmid]:
                paper["abstract"] = text
                paper["abstract_source"] = "pubmed"

    # 3) 残りは DOI で Crossref を 1 本ずつ
    for record in records.values():
        for paper in record.get("papers") or []:
            if paper.get("abstract"):
                continue
            text = fetch_crossref(str(paper.get("doi") or "").strip())
            if text:
                paper["abstract"] = text
                paper["abstract_source"] = "crossref"

    # 4) どちらでも取れなければ null / none（空文字は入れない）
    for record in records.values():
        for paper in record.get("papers") or []:
            if not paper.get("abstract"):
                paper["abstract"] = None
                paper["abstract_source"] = "none"


# ==========================================================================================
# 5. sync
# ==========================================================================================

def _id_list(data, what: str) -> list:
    """`/models` と `/models/ver_number` の返答（配列か、配列を含む入れ物）を list にする。"""
    if isinstance(data, list):
        return data
    if isinstance(data, dict):
        for value in data.values():
            if isinstance(value, list):
                return value
    raise ModelDBError(f"{what} の返答が配列ではありません")


def fetch_index() -> dict[int, int]:
    """全モデルの `{id: ver_number}`。長さが違えば 1 回だけ取り直す。それでも違えば例外。"""
    for attempt in range(2):
        ids = _id_list(get_json(f"{API_BASE}/models"), "モデルの一覧")
        vers = _id_list(get_json(f"{API_BASE}/models/ver_number"), "版の一覧")
        if len(ids) == len(vers):
            index: dict[int, int] = {}
            for raw_id, raw_ver in zip(ids, vers):
                try:
                    index[int(raw_id)] = int(raw_ver)
                except (TypeError, ValueError):
                    continue
            return index
        if attempt == 0:
            # サーバの再起動やモデルの追加で順番がずれることがある。取り直すのは 1 回だけ
            print("モデルの一覧と版一覧の長さが合いません。もう一度取り直します", file=sys.stderr)
    raise ModelDBError("モデルの一覧と版一覧の長さが合いません")


def pick_targets(index: dict[int, int], old: dict[int, dict], ids, limit) -> list[int]:
    """取り直す ID を昇順で決める。"""
    if ids:
        targets = sorted({int(i) for i in ids})
    else:
        targets = []
        for model_id in sorted(index):
            before = old.get(model_id)
            if before is None or before.get("ver_number") != index[model_id] or before.get("error") is not None:
                targets.append(model_id)
    if limit and limit > 0:
        targets = targets[:limit]
    return targets


def _safe_fetch(model_id: int, ver_number, sink: list) -> dict | None:
    """1 モデルを取る。割り込み（KeyboardInterrupt）が来たら `sink` に残して None を返す。"""
    try:
        return fetch_model(model_id, ver_number)
    except KeyboardInterrupt:
        sink.append("interrupt")
        return None
    except Exception as e:  # 1 モデルで全体が止まらないように
        record = empty_record(model_id, ver_number, now_iso())
        record["error"] = f"取り損ねました（{e}）"
        return record


def cmd_sync(args) -> int:
    """カタログを差分更新する。"""
    path = catalog_path()
    old = load_catalog(path)
    index = fetch_index()

    records = dict(old)
    removed = 0
    for model_id in list(records):
        if model_id not in index:  # サーバに無いものは外す
            del records[model_id]
            removed += 1
    targets = pick_targets(index, old, args.ids, args.limit)
    versions = {mid: index.get(mid, old.get(mid, {}).get("ver_number")) for mid in targets}

    updated: set[int] = set()
    sink: list = []
    done = 0
    if targets:
        pool = concurrent.futures.ThreadPoolExecutor(max_workers=max(1, args.workers))
        try:
            futures = {pool.submit(_safe_fetch, mid, versions[mid], sink): mid for mid in targets}
            for future in concurrent.futures.as_completed(futures):
                model_id = futures[future]
                try:
                    record = future.result()
                except KeyboardInterrupt:
                    sink.append("interrupt")
                    break
                except Exception as e:
                    record = empty_record(model_id, versions[model_id], now_iso())
                    record["error"] = f"取り損ねました（{e}）"
                if record is None:  # 割り込み
                    break
                records[model_id] = record
                updated.add(model_id)
                done += 1
                if done % 50 == 0:
                    print(f"取り終えたモデル: {done}/{len(targets)}", file=sys.stderr)
        except KeyboardInterrupt:
            sink.append("interrupt")
        finally:
            pool.shutdown(wait=not sink, cancel_futures=bool(sink))

    if not sink:
        # アブストラクトはモデルを取り終えてからまとめて埋める
        fill_abstracts(records, old)
    write_catalog(records, path)

    fetched = sum(1 for mid in updated if records[mid].get("error") is None)
    failed = len(updated) - fetched
    result = {
        "total": len(index),
        "fetched": fetched,
        "skipped": max(0, len(records) - len(updated)),
        "failed": failed,
        "removed": removed,
        "path": str(path),
    }
    if args.json:
        print(json.dumps(result, ensure_ascii=False))
    else:
        print(f"取り直した {fetched + failed} 件（失敗 {failed}）・使い回した {result['skipped']} 件"
              f"・外した {removed} 件 / {path}")
    if sink:
        print("中断した。もう一度 sync すると続きから取る", file=sys.stderr)
        return 130
    return 0


# ==========================================================================================
# 6. search / show / status（ネットを使わない）
# ==========================================================================================

def _need_catalog() -> dict[int, dict]:
    path = catalog_path()
    if not path.is_file():
        print("カタログがありません。先に modeldb.py sync を打ってください", file=sys.stderr)
        raise LookupError(path)
    return load_catalog(path)


def record_year(record: dict):
    """先頭の論文の年。"""
    papers = record.get("papers") or []
    return papers[0].get("year") if papers else None


def score_word(record: dict, word: str) -> int:
    """語の点数を返す（0 = 当たらなかった）。"""
    if word in (record.get("model_name") or "").lower():
        return 3
    for paper in record.get("papers") or []:
        if word in (paper.get("title") or "").lower():
            return 2
    haystacks = [record.get("notes") or ""]
    for paper in record.get("papers") or []:
        haystacks.append(paper.get("abstract") or "")
        haystacks.extend(paper.get("authors") or [])
    for key in ("regions", "celltypes", "currents", "receptors", "model_concepts", "model_types"):
        haystacks.extend(str(entry.get("name") or "") for entry in record.get(key) or [])
    return 1 if any(word in text.lower() for text in haystacks) else 0


def label_names(record: dict, key: str) -> list[str]:
    """`simulators`（名前だけ）と `regions` など（id と名前）の両方の形から名前を取り出す。"""
    values = record.get(key) or []
    names = []
    for value in values:
        if isinstance(value, dict):
            name = str(value.get("name") or "")
        else:
            name = str(value or "")
        if name:
            names.append(name)
    return names


def _matches_filter(record: dict, args) -> bool:
    """絞り込み（全部大文字と小文字を区別しない部分一致）。"""
    if args.doi:
        needle = DOI_URL_RE.sub("", args.doi).strip().lower()
        if not any(needle in str(p.get("doi") or "").lower() for p in record.get("papers") or []):
            return False
    for value, key in ((args.region, "regions"), (args.simulator, "simulators"),
                       (args.celltype, "celltypes"), (args.concept, "model_concepts")):
        if not value:
            continue
        needle = value.lower()
        if not any(needle in name.lower() for name in label_names(record, key)):
            return False
    return True


def cmd_search(args) -> int:
    """カタログだけから探す（ネットを使わない）。"""
    try:
        records = _need_catalog()
    except LookupError:
        return 1
    words = [w.strip().lower() for w in (args.words or []) if w.strip()]
    rows: list[tuple[int, dict]] = []
    for model_id in sorted(records):
        record = records[model_id]
        if not _matches_filter(record, args):
            continue
        score = 0
        for word in words:
            points = score_word(record, word)
            if points == 0:
                score = 0
                break
            score += points
        else:
            rows.append((score, record))
    rows.sort(key=lambda pair: (-pair[0], pair[1]["model_id"]))
    if args.limit and args.limit > 0:
        rows = rows[:args.limit]
    if args.json:
        print(json.dumps([
            {
                "model_id": record["model_id"],
                "model_name": record.get("model_name") or "",
                "year": record_year(record),
                "simulators": record.get("simulators") or [],
                "regions": [entry.get("name") or "" for entry in record.get("regions") or []],
                "model_url": record.get("model_url") or "",
                "source_github_url": record.get("source_github_url") or "",
                "score": score,
            }
            for score, record in rows
        ], ensure_ascii=False))
        return 0
    for score, record in rows:
        simulators = ", ".join(record.get("simulators") or [])
        print(f"{record['model_id']}\t{record_year(record) or ''}\t{simulators}\t"
              f"{record.get('model_name') or ''}")
    return 0


def show_text(record: dict) -> str:
    """`show` の `--json` を付けないときの読みやすい形。"""
    out = io.StringIO()
    print(f"モデル {record['model_id']}: {record.get('model_name') or ''}", file=out)
    print(f"版: {record.get('ver_number')}　取り出したとき: {record.get('fetched_at')}", file=out)
    for label, key in (("シミュレータ", "simulators"), ("脳領域", "regions"), ("細胞種", "celltypes"),
                       ("電流", "currents"), ("受容体", "receptors"), ("概念", "model_concepts"),
                       ("型", "model_types"), ("実装者", "implementers")):
        names = label_names(record, key)
        if names:
            print(f"{label}: {', '.join(names)}", file=out)
    if record.get("notes"):
        print(f"説明: {record['notes']}", file=out)
    for paper in record.get("papers") or []:
        print(f"論文 {paper.get('paper_id')}: {paper.get('title') or ''}", file=out)
        if paper.get("authors"):
            print(f"  著者: {', '.join(paper['authors'])}", file=out)
        mark = {"pubmed": "PubMed", "crossref": "Crossref", "none": "なし"}.get(
            paper.get("abstract_source") or "", "なし")
        if paper.get("doi"):
            print(f"  DOI: {paper['doi']}（アブストラクトの入手先: {mark}）", file=out)
        if paper.get("abstract"):
            print(f"  アブストラクト: {paper['abstract']}", file=out)
    print(f"解説ページ: {record.get('model_url')}", file=out)
    print(f"ファイル一覧: {record.get('source_files_url')}（{record.get('file_count')} 個）", file=out)
    print(f"ソース: {record.get('source_zip_url')} / GitHub: {record.get('source_github_url')}", file=out)
    if record.get("error"):
        print(f"注意: {record['error']}", file=out)
    return out.getvalue().rstrip()


def cmd_show(args) -> int:
    """1 件のレコードを出す（ネットを使わない）。"""
    try:
        records = _need_catalog()
    except LookupError:
        return 1
    try:
        model_id = int(args.model_id)
    except (TypeError, ValueError):
        raise ModelDBError(f"モデル ID が読めませんでした: {args.model_id}") from None
    record = records.get(model_id)
    if record is None:
        print(f"モデル {model_id} はカタログにありません", file=sys.stderr)
        return 1
    print(json.dumps(record, ensure_ascii=False, indent=2) if args.json else show_text(record))
    return 0


def cmd_status(args) -> int:
    """カタログの具合を出す（ネットを使わない）。"""
    path = catalog_path()
    records = load_catalog(path)
    times = sorted(t for t in (r.get("fetched_at") for r in records.values()) if t)
    result = {
        "path": str(path),
        "count": len(records),
        "failed": sum(1 for r in records.values() if r.get("error") is not None),
        "oldest_fetched_at": times[0] if times else None,
        "newest_fetched_at": times[-1] if times else None,
    }
    if args.json:
        print(json.dumps(result, ensure_ascii=False))
    else:
        print(f"{result['count']} 件（失敗 {result['failed']} 件）: {path}")
        print(f"古い取り出し {result['oldest_fetched_at']} / 新しい取り出し {result['newest_fetched_at']}")
    return 0


# ==========================================================================================
# 7. 入口
# ==========================================================================================

def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description="ModelDB のカタログを写し、引けるようにする")
    sub = parser.add_subparsers(dest="verb", required=True)

    sync = sub.add_parser("sync", help="カタログを差分更新する（ネットを使う）")
    sync.add_argument("--limit", type=int, default=0, help="取り直す ID を先頭 N 件に絞る")
    sync.add_argument("--ids", default="", help="この ID だけ取り直す（カンマ区切り）")
    sync.add_argument("--workers", type=int, default=4, help="同時に取りに行く数（既定 4）")
    sync.add_argument("--json", action="store_true")
    sync.set_defaults(func=cmd_sync)

    search = sub.add_parser("search", help="カタログから探す（ネットを使わない）")
    search.add_argument("words", nargs="*", help="探す語（全部が含まれるものを出す）")
    search.add_argument("--doi", default="", help="論文の DOI で狭める")
    search.add_argument("--region", default="", help="脳領域で狭める")
    search.add_argument("--simulator", default="", help="シミュレータで狭める")
    search.add_argument("--celltype", default="", help="細胞種で狭める")
    search.add_argument("--concept", default="", help="概念で狭める")
    search.add_argument("--limit", type=int, default=20, help="出す件数（既定 20）")
    search.add_argument("--json", action="store_true")
    search.set_defaults(func=cmd_search)

    show = sub.add_parser("show", help="1 件のレコードを出す（ネットを使わない）")
    show.add_argument("model_id")
    show.add_argument("--json", action="store_true")
    show.set_defaults(func=cmd_show)

    status = sub.add_parser("status", help="カタログの具合を出す（ネットを使わない）")
    status.add_argument("--json", action="store_true")
    status.set_defaults(func=cmd_status)
    return parser


def parse_ids(raw: str) -> list[int]:
    """`--ids` を整数にする。読めないものがあれば引数の誤り。"""
    out: list[int] = []
    for piece in (raw or "").split(","):
        piece = piece.strip()
        if not piece:
            continue
        try:
            out.append(int(piece))
        except ValueError:
            raise ModelDBError(f"--ids が読めませんでした: {piece}") from None
    return out


def main(argv: list[str] | None = None) -> int:
    argv = list(sys.argv[1:] if argv is None else argv)
    args = build_parser().parse_args(argv)
    try:
        args.ids = parse_ids(getattr(args, "ids", ""))
        return args.func(args)
    except ModelDBError as e:
        print(f"エラー: {e}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
