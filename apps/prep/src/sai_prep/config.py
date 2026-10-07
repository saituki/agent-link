"""sai-prep の設定の読み書き。

設定は `~/.config/sai-prep/config.json`（0600）に置く。作業の置き場は
`~/Library/Caches/sai-prep/jobs/<uuid>/`。どちらも環境変数で差し替えられる
（`SAI_PREP_CONFIG`・`SAI_PREP_JOBS`）。テストはこれを使って本物の HOME を汚さない。
"""

from __future__ import annotations

import json
import os
from dataclasses import asdict, dataclass, fields
from pathlib import Path
from urllib.parse import urlparse

# 翻訳に使う mlx-lm のモデル。`_immersive-translate` 版だけが `<<<source>>>` の
# 文字列プロンプトを受けられる（調査 R2）。
DEFAULT_MLX_MODEL = "mlx-community/translategemma-4b-it-4bit_immersive-translate"
DEFAULT_MLX_PORT = 8766
DEFAULT_DOCLING_DEVICE = "mps"


def config_path() -> Path:
    """設定ファイルのパス。`SAI_PREP_CONFIG` があればそれを使う。"""
    override = os.environ.get("SAI_PREP_CONFIG")
    if override:
        return Path(override).expanduser()
    return Path.home() / ".config" / "sai-prep" / "config.json"


def jobs_dir() -> Path:
    """作業の置き場の親。`SAI_PREP_JOBS` があればそれを使う。"""
    override = os.environ.get("SAI_PREP_JOBS")
    if override:
        return Path(override).expanduser()
    return Path.home() / "Library" / "Caches" / "sai-prep" / "jobs"


def allowed_server_url(url: str) -> bool:
    """サーバの住所として受け付けてよいか。

    `https://` はそのまま、`http://` は 127.0.0.1 か localhost のときだけ許す。
    個人のトークンを平文で外へ送らないため。
    """
    if not url:
        return False
    parsed = urlparse(url)
    if parsed.scheme == "https":
        return bool(parsed.netloc)
    if parsed.scheme == "http":
        return parsed.hostname in ("127.0.0.1", "localhost", "::1")
    return False


def normalize_server_url(url: str) -> str:
    """住所の末尾の `/` を落とす。"""
    return url.strip().rstrip("/")


@dataclass
class Config:
    """sai-prep の設定。"""

    server_url: str = ""
    token: str = ""
    project: str = ""
    translate: bool = True
    mlx_model: str = DEFAULT_MLX_MODEL
    mlx_port: int = DEFAULT_MLX_PORT
    docling_device: str = DEFAULT_DOCLING_DEVICE

    @classmethod
    def from_dict(cls, data: dict) -> "Config":
        known = {f.name for f in fields(cls)}
        clean = {k: v for k, v in data.items() if k in known}
        return cls(**clean)

    def to_dict(self) -> dict:
        return asdict(self)

    def with_server(self, server_url: str, token: str) -> "Config":
        self.server_url = normalize_server_url(server_url)
        self.token = token.strip()
        return self


def load() -> Config:
    """設定を読む。無ければ既定を返す。"""
    path = config_path()
    if not path.is_file():
        return Config()
    try:
        data = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError):
        return Config()
    if not isinstance(data, dict):
        return Config()
    return Config.from_dict(data)


def save(config: Config) -> Path:
    """設定を書く。ファイルは 0600、親フォルダは 0700。"""
    path = config_path()
    path.parent.mkdir(parents=True, exist_ok=True)
    try:
        os.chmod(path.parent, 0o700)
    except OSError:
        pass
    text = json.dumps(config.to_dict(), ensure_ascii=False, indent=2) + "\n"
    # 0600 で作る（他の利用者にトークンを見せない）。
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
    try:
        with os.fdopen(fd, "w", encoding="utf-8") as handle:
            handle.write(text)
    except Exception:
        raise
    try:
        os.chmod(path, 0o600)
    except OSError:
        pass
    return path
