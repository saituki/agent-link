"""Usage タブ（LLM の料金・制限）のデータ集め（実装計画 42 §3.3）．

集めるものは 2 つ:

- OpenRouter の残高・使用料・無料モデル回数 … 公開 API（キーは .env の環境変数）
- DeepSeek の残高 … 公開 API。使用料は pi の記録（``~/.pi/agent/sessions``）から集計する

外部 API は標準の ``urllib.request`` だけで呼ぶ（新しい依存を入れない）．タイムアウトは 10 秒．
API キーは返り値に含めない（画面にも出さない）．
"""

from __future__ import annotations

import json
import os
import time
import urllib.error
import urllib.request
from pathlib import Path

from sai_web import registry

# 外部 API のタイムアウト（秒・42 §3.3）
HTTP_TIMEOUT = 10
# 集計の覚え（秒・42 §3.3 の ponytail）
CACHE_TTL = 60
# Activity に残す日数（42 §3.3）
ACTIVITY_DAYS = 30
# 1 ページの件数（42 §3.3 の logs）
LOGS_PAGE = 50


# pi の provider とキーの対応を調べるファイル（42 §3.2）。
# 多人数モードでは、その人のひな形の HOME の下を見る（単独モードは今までどおり）。

def pi_agent_dir() -> Path:
    """pi の設定の置き場。多人数: `user_home()/.pi/agent`。単独: `~/.pi/agent`。"""
    if registry.multi_user() and registry.current_user.get() is not None:
        return registry.user_home() / ".pi" / "agent"
    return Path.home() / ".pi" / "agent"


def pi_models_path() -> Path:
    return pi_agent_dir() / "models.json"


def pi_auth_path() -> Path:
    return pi_agent_dir() / "auth.json"


def pi_session_dirs() -> list[Path]:
    """pi の記録（`.jsonl`）を読むフォルダ。

    - `PI_SESSIONS_DIR` があればそれ 1 つ（今までの差し替え方を保つ）。
    - 多人数モード: 今の利用者のプロジェクトごとの
      `<project_home>/.pi/agent/sessions` を全部。
    - 単独モード: `~/.pi/agent/sessions` 1 つ。
    """
    env = (os.getenv("PI_SESSIONS_DIR") or "").strip()
    if env:
        return [Path(env)]
    if registry.multi_user():
        out: list[Path] = []
        for project in registry.load_registry():
            name = str(project.get("name") or "")
            if not name:
                continue
            out.append(registry.project_home(name) / ".pi" / "agent" / "sessions")
        return out
    return [Path.home() / ".pi" / "agent" / "sessions"]


# 集計の覚え（利用者の名前 → {"at", "rows"}）。毎回 pi の全ファイルを読まないため
_pi_cache: dict[str, dict] = {}
# summary の覚え（利用者の名前 → {"at", "body"}）。外部 API を毎回呼ばないため
_summary_cache: dict[str, dict] = {}


# --- OpenRouter のアカウント設定（42 §3.2） ---


def openrouter_accounts() -> list[dict]:
    """`.env` の「名前=キーの入った環境変数名」を読む（未設定なら main=OPENROUTER_API_KEY）．

    値の形: ``SAI_WEB_OPENROUTER_ACCOUNTS=main=OPENROUTER_API_KEY,sub=OPENROUTER_API_KEY_2``
    名前と環境変数名だけを返す（キーそのものは返さない）．
    """
    raw = (os.getenv("SAI_WEB_OPENROUTER_ACCOUNTS") or "").strip()
    accounts: list[dict] = []
    for part in raw.split(","):
        part = part.strip()
        if not part or "=" not in part:
            continue
        name, env_var = part.split("=", 1)
        name = name.strip()
        env_var = env_var.strip()
        if name and env_var:
            accounts.append({"name": name, "env": env_var})
    # 未設定・書き方が壊れているときは既定に戻す（画面が空にならないように）
    return accounts or [{"name": "main", "env": "OPENROUTER_API_KEY"}]


def _key_for(env_var: str) -> str | None:
    """キーを読む（空文字は未設定として扱う）．

    多人数モードでは今の利用者の `secrets.env`（`containers.read_keys`）から読む。
    単独モードは今までどおり環境変数から読む。
    """
    if registry.multi_user():
        from sai_web import containers

        user = registry.current_user.get()
        if not isinstance(user, dict) or not user.get("name"):
            return None
        value = (containers.read_keys(str(user["name"])).get(env_var) or "").strip()
        return value or None
    value = (os.getenv(env_var) or "").strip()
    return value or None


# --- 外部 API ---


def _get_json(url: str, headers: dict) -> dict:
    """JSON を GET する（タイムアウト 10 秒）．"""
    req = urllib.request.Request(url, headers=headers)
    with urllib.request.urlopen(req, timeout=HTTP_TIMEOUT) as res:
        return json.loads(res.read().decode("utf-8"))


def _error_text(exc: Exception) -> str:
    """画面に出す短い失敗の理由（キーは含めない）．"""
    if isinstance(exc, urllib.error.HTTPError):
        return f"取得できませんでした（HTTP {exc.code}）"
    if isinstance(exc, urllib.error.URLError):
        return "取得できませんでした（接続できません）"
    return "取得できませんでした"


def openrouter_card(name: str, env_var: str) -> dict:
    """OpenRouter の 1 アカウント分（残高・使用料・無料モデル回数・キーの上限）．

    失敗したときは ``{"error": …}`` を返し，キーは返さない（42 §3.3）．
    """
    card: dict = {"provider": "openrouter", "account": name, "name": name, "env": env_var}
    key = _key_for(env_var)
    if not key:
        card["error"] = "キーが未設定です"
        return card
    headers = {"Authorization": f"Bearer {key}"}
    try:
        body = _get_json("https://openrouter.ai/api/v1/key", headers)
    except Exception as exc:  # noqa: BLE001 - 失敗の理由を画面に出す
        card["error"] = _error_text(exc)
        return card
    data = body.get("data") or {}
    card["usage"] = {
        "daily": data.get("usage_daily"),
        "weekly": data.get("usage_weekly"),
        "monthly": data.get("usage_monthly"),
        "total": data.get("usage"),
    }
    card["limit"] = data.get("limit")
    card["limit_remaining"] = data.get("limit_remaining")
    card["free_model_daily_requests"] = data.get("free_model_daily_requests")
    # 残高は /credits（取れなくても他の項目は出す）
    try:
        credits = (_get_json("https://openrouter.ai/api/v1/credits", headers).get("data")) or {}
        total_credits = credits.get("total_credits")
        total_usage = credits.get("total_usage")
        if total_credits is not None and total_usage is not None:
            card["total_credits"] = total_credits
            card["total_usage"] = total_usage
            card["balance"] = float(total_credits) - float(total_usage)
    except Exception:  # noqa: BLE001 - 残高だけ取れないときは諦める
        card["balance"] = None
    return card


def deepseek_balance() -> dict:
    """DeepSeek の残高（``/user/balance``）．失敗したときは ``{"error": …}``．"""
    card: dict = {"provider": "deepseek", "account": "deepseek", "name": "DeepSeek"}
    key = _key_for("DEEPSEEK_API_KEY")
    if not key:
        card["error"] = "キーが未設定です"
        return card
    try:
        body = _get_json(
            "https://api.deepseek.com/user/balance",
            {"Authorization": f"Bearer {key}"},
        )
    except Exception as exc:  # noqa: BLE001
        card["error"] = _error_text(exc)
        return card
    card["is_available"] = bool(body.get("is_available"))
    infos = body.get("balance_infos") or []
    info = infos[0] if infos else {}
    try:
        card["balance"] = float(info.get("total_balance"))
    except (TypeError, ValueError):
        card["balance"] = None
    card["currency"] = info.get("currency") or "USD"
    return card


# --- pi の記録（42 §3.3） ---


def read_pi_calls(
    base_dir: Path | None = None,
    now_ms: float | None = None,
    days: int = ACTIVITY_DAYS,
) -> list[dict]:
    """pi の記録を読み，assistant の行を 1 回分の使用に直す（新しい順は問わない）．

    ``base_dir`` はテスト用（既定は `pi_session_dirs()` の全部）．直近 ``days`` 日だけ残す．
    """
    roots = [Path(base_dir)] if base_dir is not None else pi_session_dirs()
    now_ms = time.time() * 1000 if now_ms is None else now_ms
    cutoff_ms = now_ms - days * 86400 * 1000
    rows: list[dict] = []
    for root in roots:
        for path in sorted(root.glob("*/*.jsonl")):
            try:
                fh = path.open("r", encoding="utf-8", errors="ignore")
            except OSError:
                continue
            with fh:
                for line in fh:
                    line = line.strip()
                    if not line:
                        continue
                    try:
                        obj = json.loads(line)
                    except json.JSONDecodeError:
                        continue
                    if obj.get("type") != "message":
                        continue
                    msg = obj.get("message")
                    if not isinstance(msg, dict) or msg.get("role") != "assistant":
                        continue
                    usage = msg.get("usage") or {}
                    ts = msg.get("timestamp") or obj.get("timestamp")
                    if not isinstance(ts, (int, float)) or ts < cutoff_ms:
                        continue
                    cost = (usage.get("cost") or {}).get("total") or 0
                    rows.append(
                        {
                            "timestamp": int(ts),
                            "provider": msg.get("provider") or "",
                            "model": msg.get("model") or "",
                            "input": int(usage.get("input") or 0),
                            "output": int(usage.get("output") or 0),
                            "cache_read": int(usage.get("cacheRead") or 0),
                            "cache_write": int(usage.get("cacheWrite") or 0),
                            "total": int(usage.get("totalTokens") or 0),
                            "cost": float(cost),
                        }
                    )
    rows.sort(key=lambda r: r["timestamp"])
    return rows


def pi_calls(force: bool = False) -> list[dict]:
    """pi の記録（60 秒だけ覚えておく・42 §3.3）．多人数モードでは利用者ごとに覚える。"""
    now = time.time()
    key = registry.user_key()
    hit = _pi_cache.get(key)
    if not force and hit is not None and hit["rows"] is not None and now - hit["at"] < CACHE_TTL:
        return hit["rows"]
    rows = read_pi_calls()
    _pi_cache[key] = {"at": now, "rows": rows}
    return rows


# --- pi の provider とアカウントの対応（42 §3.2） ---


def _env_name(ref) -> str:
    """``$OPENROUTER_API_KEY`` → ``OPENROUTER_API_KEY``．"""
    if not isinstance(ref, str):
        return ""
    return ref[1:] if ref.startswith("$") else ref


def provider_env_map() -> dict[str, str]:
    """pi の provider 名 → 参照している環境変数名（models.json と auth.json から）．"""
    out: dict[str, str] = {}
    # 個別の provider（openrouter-deepinfra など）は models.json
    try:
        models = json.loads(pi_models_path().read_text(encoding="utf-8"))
        for name, provider in (models.get("providers") or {}).items():
            if isinstance(provider, dict) and provider.get("apiKey"):
                out.setdefault(name, _env_name(provider["apiKey"]))
    except (OSError, json.JSONDecodeError, AttributeError):
        pass
    # pi 組み込みの provider（openrouter / deepseek / google）は auth.json
    try:
        auth = json.loads(pi_auth_path().read_text(encoding="utf-8"))
        for name, info in (auth or {}).items():
            if isinstance(info, dict) and info.get("key"):
                out[name] = _env_name(info["key"])
    except (OSError, json.JSONDecodeError, AttributeError):
        pass
    return out


def map_provider(
    provider: str, env_of: dict[str, str], accounts: list[dict]
) -> tuple[str, str] | None:
    """provider 名 → ``(種類, アカウント名)``．対応が分からなければ None．

    ``env_of`` は provider → 環境変数名（``provider_env_map``），
    ``accounts`` は ``openrouter_accounts`` の結果．
    """
    if provider == "deepseek":
        return ("deepseek", "deepseek")
    env_var = env_of.get(provider)
    for account in accounts:
        if env_var and env_var == account.get("env"):
            return ("openrouter", account["name"])
    return None


def account_for(provider: str) -> tuple[str, str] | None:
    """実際の設定ファイルを使って provider → アカウントを引く．"""
    return map_provider(provider, provider_env_map(), openrouter_accounts())


def filter_rows(
    rows: list[dict],
    provider: str,
    account: str,
    env_of: dict[str, str] | None = None,
    accounts: list[dict] | None = None,
) -> list[dict]:
    """そのアカウントに属する記録だけを残す（``env_of`` / ``accounts`` はテスト用）．"""
    env_of = provider_env_map() if env_of is None else env_of
    accounts = openrouter_accounts() if accounts is None else accounts
    want = (provider, account)
    return [r for r in rows if map_provider(r["provider"], env_of, accounts) == want]


def known_target(provider: str, account: str) -> bool:
    """外から来た ``provider`` / ``account`` が設定にあるか（42 §3.3・無ければ 404）．"""
    if provider == "openrouter":
        return account in {a["name"] for a in openrouter_accounts()}
    if provider == "deepseek":
        return account == "deepseek"
    return False


# --- 集計（42 §3.3） ---


def date_key(ts_ms: float) -> str:
    """時刻（ms）→ 端末の現地時刻の日付（``2026-09-24``）．"""
    return time.strftime("%Y-%m-%d", time.localtime(ts_ms / 1000))


def days_series(
    rows: list[dict], now_ms: float | None = None, days: int = ACTIVITY_DAYS
) -> list[dict]:
    """直近 ``days`` 日の日ごと（費用・token・回数）．無い日も 0 で並べる．"""
    now_ms = time.time() * 1000 if now_ms is None else now_ms
    today = time.localtime(now_ms / 1000)
    # 今日の 0 時（現地時刻）を ms にする
    midnight = time.mktime(
        (today.tm_year, today.tm_mon, today.tm_mday, 0, 0, 0, 0, 0, -1)
    ) * 1000
    series: list[dict] = []
    for i in range(days - 1, -1, -1):
        day_ms = midnight - i * 86400 * 1000
        series.append({"date": date_key(day_ms), "cost": 0.0, "tokens": 0, "calls": 0})
    index = {entry["date"]: entry for entry in series}
    for row in rows:
        entry = index.get(date_key(row["timestamp"]))
        if entry is None:
            continue
        entry["cost"] += row["cost"]
        entry["tokens"] += row["total"]
        entry["calls"] += 1
    for entry in series:
        entry["cost"] = round(entry["cost"], 6)
    return series


def by_model(rows: list[dict]) -> list[dict]:
    """モデル別（回数・token・費用）．費用の多い順．"""
    agg: dict[str, dict] = {}
    for row in rows:
        model = row["model"] or "(不明)"
        item = agg.setdefault(model, {"model": model, "calls": 0, "tokens": 0, "cost": 0.0})
        item["calls"] += 1
        item["tokens"] += row["total"]
        item["cost"] += row["cost"]
    out = sorted(agg.values(), key=lambda x: (-x["cost"], -x["calls"], x["model"]))
    for item in out:
        item["cost"] = round(item["cost"], 6)
    return out


def usage_totals(rows: list[dict], now_ms: float | None = None) -> dict:
    """その日の使用と今月の使用（pi の記録から・42 §3.1 の DeepSeek カード）．"""
    now_ms = time.time() * 1000 if now_ms is None else now_ms
    today = date_key(now_ms)
    month = today[:7]
    out = {"today": 0.0, "month": 0.0, "today_tokens": 0, "month_tokens": 0, "calls": 0}
    for row in rows:
        key = date_key(row["timestamp"])
        if key == today:
            out["today"] += row["cost"]
            out["today_tokens"] += row["total"]
        if key[:7] == month:
            out["month"] += row["cost"]
            out["month_tokens"] += row["total"]
        out["calls"] += 1
    out["today"] = round(out["today"], 6)
    out["month"] = round(out["month"], 6)
    return out


def unknown_providers(rows: list[dict] | None = None) -> list[dict]:
    """設定に無い provider の集計（別枠で出す・42 §3.2）．"""
    rows = pi_calls() if rows is None else rows
    env_of = provider_env_map()
    accounts = openrouter_accounts()
    agg: dict[str, dict] = {}
    for row in rows:
        if map_provider(row["provider"], env_of, accounts) is not None:
            continue
        item = agg.setdefault(
            row["provider"], {"provider": row["provider"], "calls": 0, "tokens": 0, "cost": 0.0}
        )
        item["calls"] += 1
        item["tokens"] += row["total"]
        item["cost"] += row["cost"]
    out = sorted(agg.values(), key=lambda x: -x["cost"])
    for item in out:
        item["cost"] = round(item["cost"], 6)
    return out


# --- まとめ（API が返す形・42 §3.3） ---


def summary(refresh: bool = False) -> dict:
    """Usage タブのカード一覧（cc / OpenRouter 各アカウント / DeepSeek / 設定に無い provider）．

    外部 API の結果も 60 秒覚える（↻ の ``?refresh=1`` のときは取り直す）．
    """
    now = time.time()
    key = registry.user_key()
    hit = _summary_cache.get(key)
    if not refresh and hit is not None and hit["body"] is not None and now - hit["at"] < CACHE_TTL:
        return hit["body"]
    deepseek = deepseek_balance()
    deepseek["usage"] = usage_totals(filter_rows(pi_calls(), "deepseek", "deepseek"))
    body = {
        "openrouter": [
            openrouter_card(a["name"], a["env"]) for a in openrouter_accounts()
        ],
        "deepseek": deepseek,
        "unknown": unknown_providers(),
        "saved_at": int(now),
    }
    _summary_cache[key] = {"at": now, "body": body}
    return body


def activity(provider: str, account: str) -> dict:
    """日ごと（30 日）とモデル別の集計（42 §3.3）．"""
    rows = filter_rows(pi_calls(), provider, account)
    return {
        "provider": provider,
        "account": account,
        "days": days_series(rows),
        "by_model": by_model(rows),
        "totals": usage_totals(rows),
        "source": "pi",
    }


def logs(provider: str, account: str, offset: int = 0, limit: int = LOGS_PAGE) -> dict:
    """1 回ずつの一覧（新しい順・42 §3.3）．"""
    rows = sorted(
        filter_rows(pi_calls(), provider, account),
        key=lambda r: r["timestamp"],
        reverse=True,
    )
    return {
        "provider": provider,
        "account": account,
        "total": len(rows),
        "offset": offset,
        "limit": limit,
        "logs": rows[offset : offset + limit],
        "source": "pi",
    }
