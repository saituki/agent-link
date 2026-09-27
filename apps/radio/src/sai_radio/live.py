"""Gemini Live API 用の使い捨てトークン(エフェメラルトークン)を発行する．

なぜトークンが必要か:
iPhone のブラウザは Google に直接つなぐが，APIキーそのものを
iPhone 側に置くのは危険(盗まれると悪用される)．
そこで Mac 側で「1回だけ・短時間だけ使える合鍵」を作り，
iPhone にはその合鍵だけを渡す．

検証済みの注意点(計画書 1.3 節より．推測で変えないこと):
- 発行先は REST の v1alpha/auth_tokens
- ボディに liveConnectConstraints を付けてはいけない(400エラーになる)
- ブラウザからの接続先メソッド名は BidiGenerateContentConstrained
  (BidiGenerateContent だと 1008 エラーで切断される)
- APIバージョンは v1alpha 固定(v1beta は不可)
- 認証は ?access_token= クエリパラメータ(ブラウザはWebSocketにヘッダを付けられない)
"""

from __future__ import annotations

from datetime import datetime, timedelta, timezone

import httpx

from sai_radio.config import settings

# トークン発行の宛先．バージョンは v1alpha 固定(1.3 節の検証結果)．
_AUTH_TOKENS_URL = "https://generativelanguage.googleapis.com/v1alpha/auth_tokens"


class LiveError(RuntimeError):
    """トークン発行に失敗したときに投げるエラー．メッセージは日本語にする．"""


def _utc_now_iso(sec_from_now: int) -> str:
    """今から sec_from_now 秒後の時刻を ISO8601 の Z形式で返す．

    例: "2026-09-22T12:00:00Z"
    Google の auth_tokens はこの形式の時刻を受け付ける．
    """
    moment = datetime.now(timezone.utc) + timedelta(seconds=sec_from_now)
    return moment.strftime("%Y-%m-%dT%H:%M:%SZ")


def issue_ephemeral_token() -> str:
    """使い捨てトークンを1つ発行し，トークン名("auth_tokens/..."形式)を返す．

    有効期限は GEMINI_TOKEN_TTL_SEC(既定30分)．
    新しいセッションの受付は1分後まで(使い回しを防ぐため短め)．
    """
    api_key = settings.gemini_api_key
    if not api_key:
        raise LiveError(
            "GEMINI_API_KEY が設定されていません．"
            ".env かシェルの設定ファイル(~/.zshrc など)に設定してください．"
        )
    body = {
        # uses: 1 … この合鍵は1回しか使えない(安全性のため)
        "uses": 1,
        "expireTime": _utc_now_iso(settings.gemini_token_ttl_sec),
        "newSessionExpireTime": _utc_now_iso(60),
    }
    try:
        res = httpx.post(
            _AUTH_TOKENS_URL,
            params={"key": api_key},
            json=body,
            timeout=15.0,
        )
    except httpx.HTTPError as e:
        raise LiveError(f"トークン発行の通信に失敗しました: {e}") from e
    if res.status_code != 200:
        raise LiveError(
            f"トークンの発行に失敗しました(HTTP {res.status_code}): {res.text[:300]}"
        )
    data = res.json()
    name = data.get("name")
    if not name:
        raise LiveError("トークンの発行に失敗しました(応答にトークン名がありません)")
    return name
