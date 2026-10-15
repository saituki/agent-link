"""プッシュ通知の送り手．agent-link-web 自身から iPhone に通知を出す．

必要な条件（iPhone 側）:
- HTTPS で開いていること（tailscale serve で用意する）
- ホーム画面に追加していること
- 通知を許可していること

暗号化は自前で書かない．pywebpush に任せる．
"""

from __future__ import annotations

import base64
import json
import logging
from pathlib import Path
from urllib.parse import quote

from agent_link_web.config import DATA_DIR

logger = logging.getLogger("agent_link_web")

# 鍵の保存場所．起動時に1度だけ作る．
_VAPID_FILE = DATA_DIR / "vapid.json"
# 送り主の連絡先（VAPID の sub）．Apple の通知サーバーは `@localhost` のような
# 形の正しくない宛先を 403 BadJwtToken で断るため，形の正しい宛先にする．
_VAPID_SUB = "mailto:agent-link-web@example.com"
# この状態コードが返ったら登録は無効（アプリの削除・通知の取り消し等）なので消す
_GONE_STATUS = (404, 410)


def _b64url(data: bytes) -> str:
    return base64.urlsafe_b64encode(data).decode().rstrip("=")


def get_public_key() -> str | None:
    """iPhone に配る公開鍵．まだ作っていなければ None．"""
    if not _VAPID_FILE.exists():
        return None
    try:
        return json.loads(_VAPID_FILE.read_text())["public_key"]
    except (OSError, json.JSONDecodeError, KeyError):
        return None


def ensure_keys() -> str:
    """鍵の組を用意する．無ければ作って保存し，公開鍵を返す．"""
    if _VAPID_FILE.exists():
        try:
            key = json.loads(_VAPID_FILE.read_text())["public_key"]
            if key:
                return key
        except (OSError, json.JSONDecodeError, KeyError):
            pass
    from py_vapid import Vapid
    from cryptography.hazmat.primitives import serialization

    v = Vapid()
    v.generate_keys()
    pub = v.public_key.public_bytes(
        serialization.Encoding.X962, serialization.PublicFormat.UncompressedPoint
    )
    priv_pem = v.private_pem()
    if isinstance(priv_pem, bytes):
        priv_pem = priv_pem.decode()
    data = {"public_key": _b64url(pub), "private_key": priv_pem}
    _VAPID_FILE.write_text(json.dumps(data))
    return data["public_key"]


def send_to(subscription: dict, title: str, body: str, url: str) -> str:
    """1つの登録先に通知を送る．

    返り値: "ok"（届いた）/ "gone"（登録が無効・消してよい）/ "error"（一時的な失敗など）．
    """
    if not _VAPID_FILE.exists():
        return "error"
    try:
        keys = json.loads(_VAPID_FILE.read_text())
    except (OSError, json.JSONDecodeError, KeyError):
        return "error"
    try:
        from py_vapid import Vapid
        from pywebpush import WebPushException, webpush

        # 鍵は PEM で保存しているので Vapid にしてから渡す
        # （文字のまま渡すと pywebpush が DER として読もうとして毎回失敗する）
        vapid = Vapid.from_pem(keys["private_key"].encode())
        webpush(
            subscription_info={
                "endpoint": subscription["endpoint"],
                "keys": subscription["keys"],
            },
            data=json.dumps({"title": title, "body": body, "url": url}),
            vapid_private_key=vapid,
            vapid_claims={"sub": _VAPID_SUB},
        )
        return "ok"
    except WebPushException as e:
        status = getattr(e.response, "status_code", None)
        detail = getattr(e.response, "text", "") or str(e)
        logger.warning(f"通知の送信に失敗: {status} {detail[:200]}")
        return "gone" if status in _GONE_STATUS else "error"
    except Exception as e:  # 鍵が壊れている等．失敗しても監視は止めない
        logger.warning(f"通知の送信に失敗: {e}")
        return "error"


def notify_heading(run: dict) -> str:
    """通知の見出し（＝プロジェクト名）．無いときだけ agent-link（計画書 A の U05・U17）．

    watch.py と questions.py の両方から使うのでここに置く．
    """
    return str(run.get("project_name") or "").strip() or "agent-link"


def run_title(run: dict) -> str:
    """通知に出す見出し（説明があればそれ，なければブロック名）．"""
    text = run.get("description") or run.get("block_name") or "実行"
    return f"{text} #{run.get('run_id', '')}"


def run_url(run: dict) -> str:
    """通知タップで開く実行画面の住所（02 §3.1）．"""
    project = quote(str(run.get("project_name") or ""), safe="")
    return f"/#/run/{project}/{run.get('run_id', '')}"


def broadcast(title: str, body: str, url: str) -> int:
    """登録済みの全 iPhone に送る．届いた件数を返す．"""
    from agent_link_web import store

    count = 0
    for sub in store.list_subscriptions():
        try:
            info = json.loads(sub["subscription_json"])
        except json.JSONDecodeError:
            continue
        result = send_to(info, title, body, url)
        if result == "ok":
            count += 1
        elif result == "gone":
            # 無効になった登録だけ消す（一時的な失敗で消すと，以後ずっと届かなくなるため）
            store.delete_subscription(sub["endpoint"])
    return count
