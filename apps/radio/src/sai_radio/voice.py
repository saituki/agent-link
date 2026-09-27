"""音声を作る処理．

解説モード: セクションごとに AivisSpeech Engine で音声を作り，順番に
連結する．各セクションの開始・終了秒数を記録できるので，再生位置から
の逆引きがそのまま実現できる．

ポッドキャストモードは podcast.py で直接 podcast-creator ライブラリを
呼び出す．
"""

from __future__ import annotations

import io
import subprocess
import uuid
import wave
from pathlib import Path

import httpx

from sai_radio.config import AUDIO_DIR, settings
from sai_radio.sections import TextSection
from sai_radio.store import ManifestSection


class VoiceError(RuntimeError):
    pass


def synthesize(text: str, speaker: int | None = None) -> bytes:
    """1つの文章を音声(WAV bytes)にする．

    AivisSpeech Engine は VOICEVOX 互換で2段階になっている．
    1. `POST /audio_query` で読み上げ用クエリを作る
    2. そのクエリを `POST /synthesis` に渡して音声を得る
    """
    speaker = speaker if speaker is not None else settings.aivisspeech_speaker
    with httpx.Client(base_url=settings.aivisspeech_base_url, timeout=300.0) as client:
        query_resp = client.post(
            "/audio_query",
            params={"text": text, "speaker": speaker},
        )
        query_resp.raise_for_status()

        synth_resp = client.post(
            "/synthesis",
            params={"speaker": speaker},
            json=query_resp.json(),
        )
        synth_resp.raise_for_status()
        return synth_resp.content


def _wav_duration_sec(wav_bytes: bytes) -> float:
    with wave.open(io.BytesIO(wav_bytes)) as w:
        return w.getnframes() / w.getframerate()


def _concat_wavs(wav_list: list[bytes], out_path: Path) -> None:
    with wave.open(io.BytesIO(wav_list[0])) as first:
        params = first.getparams()

    with wave.open(str(out_path), "wb") as out:
        out.setparams(params)
        for wav_bytes in wav_list:
            with wave.open(io.BytesIO(wav_bytes)) as w:
                out.writeframes(w.readframes(w.getnframes()))


def _wav_to_m4a(wav_path: Path, m4a_path: Path, bitrate: str) -> None:
    """WAVファイルを AAC(m4a) に圧縮する．

    なぜ圧縮するか: 非圧縮WAVは1秒に約88KBも使うので，
    iPhoneのモバイル回線(4G)では途中で止まってしまう．
    AAC 64kbpsにすると約1/11のサイズになり，4Gでも再生できる．
    `-movflags +faststart` は，再生開始を早めるための必須オプション
    (ファイルの先頭に再生に必要な情報を置く)．
    """
    # tts_proxy.py と同じく，ffmpegは subprocess で呼び出す．
    result = subprocess.run(
        [
            "ffmpeg", "-y",
            "-i", str(wav_path),
            "-c:a", "aac", "-b:a", bitrate, "-ac", "1",
            "-movflags", "+faststart",
            str(m4a_path),
        ],
        capture_output=True,
        check=False,
    )
    if result.returncode != 0:
        # 失敗を黙って見過ごすとWAVのまま返してしまうので，必ず例外にする．
        raise VoiceError(f"音声の圧縮に失敗しました: {result.stderr.decode(errors='ignore')}")


def build_narration(sections: list[TextSection], paper_id: str) -> tuple[str, list[ManifestSection]]:
    """解説モード: セクションごとに音声を作って連結し，m4aに圧縮する．

    戻り値: (音声ファイルの相対パス, 対応表のセクション一覧)
    対応表の秒数はWAVの段階で計算しているが，圧縮しても長さは変わらないのでそのまま使う．
    """
    wav_clips: list[bytes] = []
    manifest: list[ManifestSection] = []
    cursor = 0.0

    for section in sections:
        wav_bytes = synthesize(section.body)
        duration = _wav_duration_sec(wav_bytes)
        manifest.append(
            ManifestSection(
                start=cursor,
                end=cursor + duration,
                title=section.title,
                source_text=section.body,
            )
        )
        wav_clips.append(wav_bytes)
        cursor += duration

    # まずWAVに連結し，その後ffmpegでm4aに圧縮してWAVは消す．
    wav_filename = f"{paper_id}.wav"
    wav_path = AUDIO_DIR / wav_filename
    _concat_wavs(wav_clips, wav_path)
    try:
        filename = f"{paper_id}.m4a"
        _wav_to_m4a(wav_path, AUDIO_DIR / filename, settings.audio_bitrate)
    finally:
        # 変換に成功してm4aができているときだけWAVを消す．
        # 失敗したらWAVを残して調査できるようにする．
        m4a_path = AUDIO_DIR / f"{paper_id}.m4a"
        if m4a_path.exists() and m4a_path.stat().st_size > 0:
            wav_path.unlink(missing_ok=True)
    return filename, manifest


def new_paper_id() -> str:
    return uuid.uuid4().hex[:12]
