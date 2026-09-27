"""sai-paper から送られる設定(settings)と音声の置き場所(audio_dir)の扱いを確かめるテスト（壊れたら失敗する）．

考え方:
    podcast-creator と ffmpeg/ffprobe を偽物に差し替え，build_podcast を 4 通りで呼ぶ．
    - settings あり: 送られた原稿・音声の設定が podcast-creator に渡る
    - settings なし: 今までどおり config/*.json のプロファイル名で作る
    - audio_dir あり: 論文フォルダに radio.m4a を書き，絶対パスを返す
    - audio_dir なし: 今までどおり data/audio/<paper_id>.m4a を書き，ファイル名だけを返す

実行:
    PYTHONPATH=src python tests/test_podcast_request_settings.py
"""

from __future__ import annotations

import asyncio
import sys
import tempfile
from pathlib import Path

# src/ を読み込めるようにする（作業用コピーから実行するため）
ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / "src"))

from sai_radio import podcast  # noqa: E402

SETTINGS = {
    "script": {
        "outline_provider": "openrouter",
        "outline_model": "outline-m",
        "transcript_provider": "openrouter",
        "transcript_model": "transcript-m",
        "briefing": "テスト番組",
        "num_segments": 3,
        "language": "ja",
    },
    "voice": {
        "tts_provider": "openai-compatible",
        "tts_model": "aivisspeech",
        "base_url": "http://127.0.0.1:9999",
        "speakers": [
            {"name": "A", "voice_id": "1", "backstory": "b", "personality": "p"},
            {"name": "B", "voice_id": "2", "backstory": "b", "personality": "p"},
        ],
    },
}


def _install_fakes(calls: dict, audio_dir: Path) -> None:
    """podcast-creator・ffmpeg・ffprobe・AUDIO_DIR を偽物にする．"""

    async def fake_create_podcast(**kwargs):
        calls["create"] = kwargs
        raw = Path(kwargs["output_dir"]) / "raw.mp3"
        raw.write_bytes(b"mp3")
        return {"final_output_file_path": str(raw)}

    def fake_configure(key, value=None):
        calls.setdefault("configure", []).append((key, value))

    def fake_to_m4a(src, dst, bitrate):
        Path(dst).write_bytes(b"m4a")

    podcast.create_podcast = fake_create_podcast
    podcast.configure = fake_configure
    podcast._configured = True
    podcast._audio_bitrate_kbps = lambda path: 128.0  # 高いので m4a に変換される
    podcast._to_m4a = fake_to_m4a
    podcast.AUDIO_DIR = audio_dir


def _run(settings: dict | None, audio_dir: str | None, base: Path) -> tuple[str, dict]:
    calls: dict = {}
    _install_fakes(calls, base / "audio")
    (base / "audio").mkdir(exist_ok=True)
    path, _ = asyncio.run(podcast.build_podcast("原稿", "proj-7", settings, audio_dir))
    return path, calls


def run() -> int:
    with tempfile.TemporaryDirectory() as tmp:
        base = Path(tmp)
        paper_dir = base / "papers" / "0007_Title"

        # 1. settings あり + audio_dir あり
        path, calls = _run(SETTINGS, str(paper_dir), base)
        assert path == str((paper_dir / "radio.m4a").resolve()), path
        assert (paper_dir / "radio.m4a").read_bytes() == b"m4a"
        assert not (paper_dir / "radio.mp3").exists(), "変換後の mp3 が残っている"
        create = calls["create"]
        assert create["speaker_config"] == podcast.REQUEST_SPEAKER_PROFILE
        assert create["outline_model"] == "outline-m"
        assert create["transcript_model"] == "transcript-m"
        assert create["num_segments"] == 3
        assert "episode_profile" not in create
        key, value = calls["configure"][0]
        profile = value["profiles"][podcast.REQUEST_SPEAKER_PROFILE]
        assert key == "speakers_config"
        assert profile["tts_config"]["base_url"] == "http://127.0.0.1:9999"
        assert len(profile["speakers"]) == 2

        # 2. settings あり + audio_dir なし
        path, calls = _run(SETTINGS, None, base)
        assert path == "proj-7.m4a", path
        assert (base / "audio" / "proj-7.m4a").exists()
        assert calls["create"]["speaker_config"] == podcast.REQUEST_SPEAKER_PROFILE

        # 3. settings なし + audio_dir あり
        path, calls = _run(None, str(paper_dir), base)
        assert path == str((paper_dir / "radio.m4a").resolve()), path
        assert calls["create"]["episode_profile"] == podcast.settings.podcast_profile_name
        assert calls["configure"] == [
            ("speakers_config", str(podcast.settings.podcast_speakers_config_path))
        ], "設定なしのときは config/ の話者設定に戻す"

        # 4. settings なし + audio_dir なし（今までどおり）
        path, calls = _run(None, None, base)
        assert path == "proj-7.m4a", path
        assert calls["create"]["episode_profile"] == podcast.settings.podcast_profile_name

    print("OK: settings / audio_dir の 4 通りとも期待どおり")
    return 0


if __name__ == "__main__":
    sys.exit(run())
