"""画面(静的ファイル)と PDF アップロードの窓口が main.py から消えていることを確認するテスト（壊れたら失敗する）．

考え方:
    11-cleanup §3 A で sai-radio は画面の無い裏方サーバーになった．
    - GET / が画面(StaticFiles の mount)になっていないこと（無いければ 404 になる）
    - POST /api/papers/upload が無いこと
    - 音声化の窓口 POST /api/papers/from-extract は残っていること

実行:
    PYTHONPATH=src python tests/test_main_routes.py
"""

from __future__ import annotations

import sys
from pathlib import Path

# src/ を読み込めるようにする（作業用コピーから実行するため）
ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / "src"))

from sai_radio import main  # noqa: E402


def run() -> int:
    # 登録されている全ルートのパスを集める
    paths = {getattr(route, "path", None) for route in main.app.routes}

    assert "/" not in paths, "画面(静的ファイル)の mount が残っている（GET / が 404 ではない）"
    assert "/api/papers/upload" not in paths, "PDF アップロードの窓口が残っている"
    assert "/api/papers/from-extract" in paths, "音声化の窓口 from-extract が無い"

    print("OK: 画面と PDF アップロードは削除済み（from-extract は残っている）")
    return 0


if __name__ == "__main__":
    sys.exit(run())
