"""open-notebook (openai_compatible TTS provider) と AivisSpeech Engine を繋ぐ変換サーバー．

open-notebook のポッドキャスト機能は TTS を OpenAI 互換API
(`POST /audio/speech`, MP3固定) でしか呼べない．AivisSpeech Engine は
VOICEVOX互換の2段階API(`/audio_query`→`/synthesis`)でWAVしか返さない．
このサーバーはその間に入り，リクエストをAivisSpeech向けに変換し，
出てきたWAVをffmpegでMP3に変換して返す．

起動:
    uv run uvicorn sai_radio.tts_proxy:app --port 8202

open-notebook 側の設定:
    Model registry に type=text_to_speech, provider=openai_compatible で登録し，
    base_url にこのサーバーのURL (例 http://host.docker.internal:8202) を指定する．
    Speaker Profile の各話者の voice_id には，AivisSpeech の話者スタイルID
    (`GET /speakers` で確認できる数値) を文字列で入れる．
"""

from __future__ import annotations

import subprocess

from fastapi import FastAPI, HTTPException
from fastapi.responses import Response
from pydantic import BaseModel

from sai_radio import voice

app = FastAPI(title="sai-radio TTS proxy (AivisSpeech bridge)")


class SpeechRequest(BaseModel):
    model: str
    voice: str
    input: str
    response_format: str = "mp3"


def _wav_to_mp3(wav_bytes: bytes) -> bytes:
    """ffmpeg で WAV バイト列を MP3 バイト列に変換する．"""
    result = subprocess.run(
        ["ffmpeg", "-f", "wav", "-i", "pipe:0", "-f", "mp3", "pipe:1"],
        input=wav_bytes,
        capture_output=True,
        check=False,
    )
    if result.returncode != 0:
        raise HTTPException(status_code=500, detail=f"ffmpeg 変換に失敗: {result.stderr.decode(errors='ignore')}")
    return result.stdout


@app.post("/audio/speech")
def audio_speech(req: SpeechRequest) -> Response:
    try:
        speaker_id = int(req.voice)
    except ValueError:
        raise HTTPException(status_code=400, detail=f"voice はAivisSpeechの話者ID(数値)にすること: {req.voice!r}")

    wav_bytes = voice.synthesize(req.input, speaker=speaker_id)
    mp3_bytes = _wav_to_mp3(wav_bytes)
    return Response(content=mp3_bytes, media_type="audio/mpeg")
