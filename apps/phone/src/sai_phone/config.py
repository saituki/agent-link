"""sai-phone の設定値．すべて環境変数から読む．

開発者向けメモ: 値を変えたいときはコードを直さず，
環境変数（`.env` ファイルや `export`）を変更すること．
`.env` が無くても既定値で起動できる．
"""

from __future__ import annotations

import os
from dataclasses import dataclass
from pathlib import Path

from dotenv import load_dotenv

# プロジェクトルートの .env を読み込む（無くても動く）
load_dotenv(Path(__file__).resolve().parent.parent.parent / ".env")

# src/data 以下に SQLite を置く
DATA_DIR = Path(__file__).resolve().parent.parent / "data"
DB_PATH = DATA_DIR / "sai_phone.db"


@dataclass(frozen=True)
class Settings:
    # 待ち受けポート
    port: int = int(os.getenv("SAI_PHONE_PORT", "8200"))
    # 合言葉（未設定なら認証なし = 開発用）
    api_token: str | None = os.getenv("SAI_PHONE_TOKEN") or None
    # cc の記録置き場
    claude_sessions_dir: Path = Path(
        os.getenv("CLAUDE_SESSIONS_DIR", str(Path.home() / ".claude" / "projects"))
    )
    # pi の記録置き場
    pi_sessions_dir: Path = Path(
        os.getenv("PI_SESSIONS_DIR", str(Path.home() / ".pi" / "agent" / "sessions"))
    )
    # herdr の実行ファイル
    herdr_bin: str = os.getenv("HERDR_BIN", "herdr")
    # sai-link が実行ごとに書き出す JSON の置き場（00 §3.1）。
    # 手元確認では SAI_PHONE_RUNS_DIR で一時フォルダに差し替えられる。
    runs_dir: Path = Path(
        os.getenv("SAI_PHONE_RUNS_DIR", str(Path.home() / ".sai-moon" / "phone" / "runs"))
    )
    # sai-link CLI（Project タブ・Papers タブが呼ぶ・00 §8）
    sai_link_bin: str = os.getenv("SAI_LINK_BIN", "sai-moon-link")
    # sai-paper CLI（Papers タブが呼ぶ・PATH に無いため絶対パス・00 §8）
    sai_paper_bin: str = os.getenv(
        "SAI_PAPER_BIN", "/Users/touta/workdir/Sai/sai-moon/target/release/sai-moon-paper"
    )
    # sai-paper のルートは設定で持たない（プロジェクトごとに変わるため，
    # 論文の置き場があるプロジェクトの一覧から引く・計画 17 §3.2）
    # sai-radio の場所（Radio の中継先・00 §8）
    radio_url: str = os.getenv("SAI_RADIO_URL", "http://127.0.0.1:8201")
    # sai-radio に付ける合言葉（sai-radio 側に設定があるときだけ同じ値を書く・00 §8）
    radio_token: str | None = os.getenv("SAI_RADIO_TOKEN") or None
    # MLflow の場所（実験タブの中継先・読み取りだけ）
    mlflow_url: str = os.getenv("SAI_MLFLOW_URL", "http://127.0.0.1:5001")
    # MLflow の名前と合言葉（空なら付けない。今の MLflow は 127.0.0.1 では認証しない）
    mlflow_user: str = os.getenv("SAI_MLFLOW_USER", "")
    mlflow_password: str = os.getenv("SAI_MLFLOW_PASSWORD", "")


settings = Settings()

DATA_DIR.mkdir(parents=True, exist_ok=True)
