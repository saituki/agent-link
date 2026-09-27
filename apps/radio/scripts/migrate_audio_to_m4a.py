"""既存のWAV音声を AAC(m4a) に変換する移行スクリプト．

使い方:
    uv run python scripts/migrate_audio_to_m4a.py

やること:
1. DBに登録された論文のうち，音声が .wav のものを探す
2. ffmpeg で同名の .m4a (64kbps) を作る
3. DBの audio_path を .m4a に書き換える
4. 変換できたことが確認できたら元のWAVを消す
5. 変換前後のサイズを日本語で表示する

なぜ必要か: 非圧縮WAVはサイズが大きく(100MB超えも普通)，
iPhoneのモバイル回線では再生が止まってしまう．
m4aにすると約1/11になり，4Gでも再生できる．
"""

from __future__ import annotations

import subprocess
import sys
from pathlib import Path

# プロジェクトルートを基準に src を読み込めるようにする．
# (リポジトリ直下の scripts/ から実行する想定)
ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / "src"))

from sai_radio.config import AUDIO_DIR, settings  # noqa: E402
from sai_radio.store import init_db, list_papers, update_audio_path  # noqa: E402


def _wav_to_m4a(wav_path: Path, m4a_path: Path) -> None:
    """ffmpeg で WAV を m4a(AAC)に変換する．失敗したら例外を出す．"""
    result = subprocess.run(
        [
            "ffmpeg", "-y",
            "-i", str(wav_path),
            "-c:a", "aac", "-b:a", settings.audio_bitrate, "-ac", "1",
            "-movflags", "+faststart",  # 再生開始を早める必須オプション
            str(m4a_path),
        ],
        capture_output=True,
        check=False,
    )
    if result.returncode != 0:
        raise RuntimeError(f"ffmpeg変換に失敗: {result.stderr.decode(errors='ignore')}")


def _mb(path: Path) -> float:
    return path.stat().st_size / (1024 * 1024)


def main() -> None:
    init_db()
    papers = list_papers()
    targets = [p for p in papers if p.audio_path.endswith(".wav")]

    if not targets:
        print("変換対象のWAVはありませんでした．")
        return

    total_before = 0.0
    total_after = 0.0
    converted = 0

    for paper in targets:
        wav_path = AUDIO_DIR / paper.audio_path
        if not wav_path.exists():
            print(f"[{paper.id}] {wav_path.name} が見つからないため飛ばします．")
            continue
        m4a_name = wav_path.with_suffix(".m4a").name
        m4a_path = AUDIO_DIR / m4a_name

        before_mb = _mb(wav_path)
        print(f"[{paper.id}] {wav_path.name} ({before_mb:.1f}MB) を変換中...")
        _wav_to_m4a(wav_path, m4a_path)

        # 変換できたことが確認できてから元のWAVを消す．
        if not m4a_path.exists() or m4a_path.stat().st_size == 0:
            raise RuntimeError(f"{m4a_name} が作られなかったため中断します．WAVは削除しません．")
        after_mb = _mb(m4a_path)
        wav_path.unlink()

        # DBの参照先を .m4a に書き換える．
        update_audio_path(paper.id, m4a_name)
        print(f"  → {m4a_name} ({after_mb:.1f}MB) に変換しました．DBも更新済みです．")

        total_before += before_mb
        total_after += after_mb
        converted += 1

    print(f"\n{converted}件を変換しました．合計 {total_before:.1f}MB → {total_after:.1f}MB になりました．")


if __name__ == "__main__":
    main()
