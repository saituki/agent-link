"""podcast-creator ライブラリを直接呼び出す．open-notebookのPodcast機能の代替．"""

from __future__ import annotations

import shutil
import subprocess
import uuid
from pathlib import Path

from podcast_creator import configure, create_podcast

from sai_radio.config import AUDIO_DIR, settings
from sai_radio.store import ManifestSection

_configured = False


def _ensure_configured() -> None:
    global _configured
    if _configured:
        return
    configure("episode_config", str(settings.podcast_episode_config_path))
    configure("speakers_config", str(settings.podcast_speakers_config_path))
    _configured = True


def _audio_bitrate_kbps(audio_path: Path) -> float | None:
    """音声ファイルのビットレート(kbps)を調べる．調べられなければ None を返す．"""
    result = subprocess.run(
        [
            "ffprobe", "-v", "error",
            "-show_entries", "format=bit_rate",
            "-of", "default=noprint_wrappers=1:nokey=1",
            str(audio_path),
        ],
        capture_output=True,
        check=False,
    )
    if result.returncode != 0:
        return None
    try:
        return float(result.stdout.decode().strip()) / 1000.0
    except ValueError:
        return None


def _to_m4a(src_path: Path, dst_path: Path, bitrate: str) -> None:
    """音声ファイルを AAC(m4a, 64kbps)に変換する．

    なぜ変換するか: podcast-creatorが出すmp3はビットレートが高いことがあり，
    iPhoneのモバイル回線では再生が止まってしまう．
    `-movflags +faststart` は再生開始を早めるための必須オプション．
    """
    result = subprocess.run(
        [
            "ffmpeg", "-y",
            "-i", str(src_path),
            "-c:a", "aac", "-b:a", bitrate, "-ac", "1",
            "-movflags", "+faststart",
            str(dst_path),
        ],
        capture_output=True,
        check=False,
    )
    if result.returncode != 0:
        raise RuntimeError(f"音声の圧縮に失敗しました: {result.stderr.decode(errors='ignore')}")


# sai-paper から送られた設定で作る話者設定の名前(1回限りの設定として使う)．
REQUEST_SPEAKER_PROFILE = "sai_paper_request"


def _request_podcast_args(request_settings: dict) -> dict:
    """sai-paper の設定(script + voice)を podcast-creator の引数にする．

    話者は podcast-creator に名前付きの設定として登録してから名前で渡す．
    ponytail: 設定は podcast-creator 全体で共有されるため，別々の話者設定での
    同時生成は後勝ちになる．同時に作るようになったらリクエストごとに分ける．
    """
    script = request_settings.get("script", {})
    voice = request_settings.get("voice", {})
    configure(
        "speakers_config",
        {
            "profiles": {
                REQUEST_SPEAKER_PROFILE: {
                    "tts_provider": voice["tts_provider"],
                    "tts_model": voice["tts_model"],
                    "tts_config": {"base_url": voice["base_url"], "api_key": "unused"},
                    "speakers": voice["speakers"],
                }
            }
        },
    )
    return {
        "speaker_config": REQUEST_SPEAKER_PROFILE,
        "outline_provider": script["outline_provider"],
        "outline_model": script["outline_model"],
        "transcript_provider": script["transcript_provider"],
        "transcript_model": script["transcript_model"],
        "briefing": script["briefing"],
        "num_segments": script["num_segments"],
        "language": script["language"],
    }


async def build_podcast(
    full_text: str,
    paper_id: str,
    request_settings: dict | None = None,
    audio_dir: str | None = None,
) -> tuple[str, list[ManifestSection]]:
    """ポッドキャストモード: podcast-creatorに丸ごと生成させる．

    戻り値は voice.build_narration と同じ形 (音声ファイルのパス, 対応表)．
    mp3が既に十分小さい(64kbps以下)場合はそのまま使う．
    ビットレートが高い場合だけ m4a(64kbps)に変換して小さくする．

    - request_settings があれば，その原稿・音声の設定で作る(無ければ config/*.json)．
    - audio_dir があれば，そこに radio.m4a (または radio.mp3) を書き，絶対パスを返す．
      無ければ今までどおり AUDIO_DIR に <paper_id>.m4a を書き，相対パスを返す．
    """
    _ensure_configured()
    if request_settings:
        podcast_args = _request_podcast_args(request_settings)
    else:
        # 前のリクエストの話者設定が残っていることがあるので，config/ の設定に戻す．
        configure("speakers_config", str(settings.podcast_speakers_config_path))
        podcast_args = {"episode_profile": settings.podcast_profile_name}
    if audio_dir:
        out_dir = Path(audio_dir)
        out_dir.mkdir(parents=True, exist_ok=True)
        stem = "radio"
    else:
        out_dir = AUDIO_DIR
        stem = paper_id
    episode_name = f"sai-radio-{paper_id}"
    work_dir = AUDIO_DIR / f"_podcast_work_{uuid.uuid4().hex[:8]}"
    work_dir.mkdir(parents=True, exist_ok=True)
    try:
        result = await create_podcast(
            content=full_text,
            episode_name=episode_name,
            output_dir=str(work_dir),
            **podcast_args,
        )
        raw_audio_path = result.get("final_output_file_path")
        if not raw_audio_path or str(raw_audio_path).startswith("ERROR:"):
            raise RuntimeError(f"podcast-creator の音声生成に失敗しました: {raw_audio_path}")

        # 出てきたmp3が既に小さい(64kbps以下)ならそのまま使う．
        # 大きい場合だけ m4a に変換して小さくする．
        copied_path = out_dir / f"{stem}.mp3"
        m4a_path = out_dir / f"{stem}.m4a"
        shutil.copyfile(raw_audio_path, copied_path)
        bitrate = _audio_bitrate_kbps(copied_path)
        if bitrate is not None and bitrate <= 64.0:
            final_path = copied_path
            # 前に作った m4a が残っていると，そちらが先に見つかってしまう．
            m4a_path.unlink(missing_ok=True)
        else:
            _to_m4a(copied_path, m4a_path, settings.audio_bitrate)
            final_path = m4a_path
            # 変換できたことが確認できたら元のmp3は消す．
            if m4a_path.exists() and m4a_path.stat().st_size > 0:
                copied_path.unlink(missing_ok=True)
    finally:
        shutil.rmtree(work_dir, ignore_errors=True)

    manifest = [ManifestSection(start=0.0, end=None, title="全体", source_text=full_text)]
    # 論文フォルダに書いたときは絶対パス，AUDIO_DIR のときは今までどおりファイル名だけ．
    return (str(final_path.resolve()) if audio_dir else final_path.name), manifest
