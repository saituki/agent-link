"""sai-radio の設定値．すべて環境変数から読む．

開発者向けメモ: 値を変えたいときはコードを直さず，環境変数
(`.env` ファイルや `export`) を変更すること．
"""

from __future__ import annotations

import os
from dataclasses import dataclass
from pathlib import Path

from dotenv import load_dotenv

# プロジェクトルートの .env を読み込む
load_dotenv(Path(__file__).resolve().parent.parent.parent / ".env")

# src/data 以下に音声ファイルとDBを置く
DATA_DIR = Path(__file__).resolve().parent.parent / "data"
AUDIO_DIR = DATA_DIR / "audio"
DB_PATH = DATA_DIR / "sai_radio.db"

# リポジトリルート/config 以下に podcast-creator 用の設定JSONを置く
CONFIG_DIR = Path(__file__).resolve().parent.parent.parent / "config"


@dataclass(frozen=True)
class Settings:
    # --- PDF抽出 ---
    docling_bin: str = os.getenv("DOCLING_BIN", "docling")
    docling_timeout_sec: float = float(os.getenv("DOCLING_TIMEOUT_SEC", "300"))

    # --- 要約・質問応答用LLM (esperanto経由) ---
    llm_provider: str = os.getenv("LLM_PROVIDER", "openrouter")
    # 空文字なら起動時にエラーで気づけるようにする
    llm_model: str = os.getenv("LLM_MODEL", "")

    # --- ポッドキャスト生成 (podcast-creator) ---
    podcast_episode_config_path: Path = Path(
        os.getenv("PODCAST_EPISODE_CONFIG", str(CONFIG_DIR / "episode_config.json"))
    )
    podcast_speakers_config_path: Path = Path(
        os.getenv("PODCAST_SPEAKERS_CONFIG", str(CONFIG_DIR / "speakers_config.json"))
    )
    podcast_profile_name: str = os.getenv("PODCAST_PROFILE_NAME", "sai_radio")

    # --- 音声圧縮 (フェーズ1: モバイル回線でも再生できるよう小さくする) ---
    # 解説モードの音声を AAC(m4a) にするときのビットレート．
    # 朗読音声なら 64k で十分聞き取れる．Wi-Fi が遅いときも詰まりにくくなる．
    audio_bitrate: str = os.getenv("AUDIO_BITRATE", "64k")

    # --- 質問機能 (フェーズ3: Gemini Live API に直接つなぐ) ---
    # APIキーは環境変数 GEMINI_API_KEY から読む．
    # `.env` に書いてもよいし，シェルの設定ファイル(~/.zshrc など)に書いてもよい．
    # どちらにも無い場合は，質問機能を使おうとしたときに日本語のエラーが出る．
    gemini_api_key: str | None = os.getenv("GEMINI_API_KEY")
    # Live API で使うモデル．日付固定版を指定する．
    # (-latest は中身が差し替わるため，動作確認した版に固定する)
    gemini_live_model: str = os.getenv(
        "GEMINI_LIVE_MODEL", "models/gemini-2.5-flash-native-audio-preview-12-2025"
    )
    # 使い捨てトークンの有効期限(秒)．既定30分．短めにして安全性を確保する．
    gemini_token_ttl_sec: int = int(os.getenv("GEMINI_TOKEN_TTL_SEC", "1800"))

    # --- 既存(変更なし) ---
    # AivisSpeech Engine (日本語音声合成)．`lib/AivisSpeech-Engine` を別途起動しておく．
    aivisspeech_base_url: str = os.getenv("AIVISSPEECH_BASE_URL", "http://127.0.0.1:10101")
    # 話者スタイルID．AivisSpeech Engine 起動後 `GET /speakers` で確認し，環境変数で指定する．
    aivisspeech_speaker: int = int(os.getenv("AIVISSPEECH_SPEAKER", "0"))

    # sai-radio サーバー自体の簡易トークン認証 (未設定なら認証なし = 開発用)
    api_token: str | None = os.getenv("SAI_RADIO_TOKEN")


settings = Settings()

AUDIO_DIR.mkdir(parents=True, exist_ok=True)
