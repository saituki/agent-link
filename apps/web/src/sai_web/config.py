"""sai-web の設定値．すべて環境変数から読む．

開発者向けメモ: 値を変えたいときはコードを直さず，
環境変数（`.env` ファイルや `export`）を変更すること．
`.env` が無くても既定値で起動できる．
"""

from __future__ import annotations

import os
from dataclasses import dataclass
from pathlib import Path

from dotenv import load_dotenv

from sai_web import registry

# プロジェクトルートの .env を読み込む（無くても動く）
load_dotenv(Path(__file__).resolve().parent.parent.parent / ".env")

# Web のデータ（DB・公開鍵・ログ）はソースのフォルダではなく利用者全体の置き場に置く。
# 置き場は `<SAI_HOME>/web/`（`SAI_HOME` が無ければ `~/.sai`。U17 の共有契約）。
# ソースのフォルダには置かない。
DATA_DIR = registry.sai_home() / "web"
DB_PATH = DATA_DIR / "sai_web.db"


@dataclass(frozen=True)
class Settings:
    # 待ち受けポート
    port: int = int(os.getenv("SAI_WEB_PORT", "8200"))
    # 合言葉（未設定なら認証なし = 開発用）
    api_token: str | None = os.getenv("SAI_WEB_TOKEN") or None
    # cc の記録置き場
    claude_sessions_dir: Path = Path(
        os.getenv("CLAUDE_SESSIONS_DIR", str(Path.home() / ".claude" / "projects"))
    )
    # pi の記録置き場
    pi_sessions_dir: Path = Path(
        os.getenv("PI_SESSIONS_DIR", str(Path.home() / ".pi" / "agent" / "sessions"))
    )
    # sai-link が実行ごとに書き出す JSON の置き場（00 §3.1）。
    # 手元確認では SAI_WEB_RUNS_DIR で一時フォルダに差し替えられる。
    runs_dir: Path = Path(
        os.getenv("SAI_WEB_RUNS_DIR", str(registry.sai_home() / "web" / "runs"))
    )
    # sai-link CLI（Project タブが呼ぶ・00 §8）
    sai_link_bin: str = os.getenv("SAI_LINK_BIN", "sai-link")


settings = Settings()

DATA_DIR.mkdir(parents=True, exist_ok=True)
