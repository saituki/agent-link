#!/usr/bin/env python3
"""cc の PermissionRequest フックから呼ばれる承認の中継スクリプト．

cc が承認画面（Bash の実行やファイル編集の許可を求める画面）を出す直前に
呼ばれ，標準入力の JSON を sai-web サーバーへ転送するだけ（05 §3.3）．

方針（05 §3.3）:
- **何も出力せず，常に終了コード 0 で終える**．
  こうすると PC の端末には普段どおり承認画面が出る
  （フックが結果を返すまで画面が出ないため，フックで待ってはいけない）．
- サーバー停止中・通信エラー・JSON が読めない場合も同じ（cc の動作を壊さない）．
- 設定の登録は ~/.claude/settings.json の hooks.PermissionRequest から行う
  （matcher はツール全部）．

サーバーの場所:
- SAI_WEB_URL（例 http://127.0.0.1:8145）を最優先で読む．
- 無ければ SAI_WEB_PORT（省略時 8200）から組み立てる．
  （動作確認は別ポートで単体起動したサーバーに向けるため・05 §4）
- 観察用に SAI_WEB_HOOK_LOG を指定すると，受信した JSON をそのファイルへ
  追記する（通常は未設定）．
"""

from __future__ import annotations

import json
import os
import sys
import urllib.request


def server_base() -> str:
    """サーバーの場所（既定は 127.0.0.1:8200）．"""
    url = os.environ.get("SAI_WEB_URL")
    if url:
        return url.rstrip("/")
    port = os.environ.get("SAI_WEB_PORT") or "8200"
    return f"http://127.0.0.1:{port}"


def main() -> int:
    # ここで絶対に例外を外へ出さない（cc のフック実行を止めないため）
    try:
        raw_text = sys.stdin.read()
        try:
            data = json.loads(raw_text)
        except Exception:
            data = None
        # 観察用の控え（未設定なら何もしない）
        log_path = os.environ.get("SAI_WEB_HOOK_LOG")
        if log_path:
            try:
                with open(log_path, "a", encoding="utf-8") as f:
                    f.write(raw_text if raw_text.endswith("\n") else raw_text + "\n")
            except Exception:
                pass
        if not isinstance(data, dict) or not data.get("session_id"):
            return 0
        payload = dict(data)
        # どのペインの cc か（同じ cc_question.py の受け方と同じく環境変数から）
        payload["pane_id"] = os.environ.get("HERDR_PANE_ID") or ""
        body = json.dumps(payload).encode("utf-8")
        req = urllib.request.Request(
            f"{server_base()}/api/approvals",
            data=body,
            headers={"Content-Type": "application/json"},
            method="POST",
        )
        with urllib.request.urlopen(req, timeout=2) as res:
            res.read()
    except Exception:
        # サーバー停止中でも cc は普段どおり動く（何も出さず 0 で終える）
        pass
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
