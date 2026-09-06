"""Local CPU speech service. One backend is loaded per process."""
import io
import os
import threading
import time
from contextlib import asynccontextmanager
from typing import Literal
from pathlib import Path

import numpy as np
import soundfile as sf
from fastapi import FastAPI, HTTPException, Response, UploadFile, File, Form
from fastapi.responses import FileResponse
from pydantic import BaseModel, Field

BACKEND = os.environ.get("TTS_BACKEND", "kokoro")
VOICES = ({"melo_zh": "z"} if BACKEND == "melo" else
          {"zf_xiaobei": "z", "zm_yunxi": "z", "af_heart": "a", "am_michael": "a"})
pipelines = {}
lock = threading.Lock()


def bounded_parts(text, limit=180):
    """Bound even unpunctuated API input; the desktop already splits sentences."""
    while len(text) > limit:
        window = text[:limit]
        end = max((window.rfind(mark) + 1 for mark in "。！？!?；;，,\n "), default=0)
        if end < limit // 2:
            end = limit
        yield text[:end]
        text = text[end:]
    if text:
        yield text


@asynccontextmanager
async def lifespan(app):
    import torch
    torch.set_num_threads(int(os.environ.get("TTS_THREADS", "4")))
    if BACKEND == "melo":
        from melo.api import TTS
        model = TTS(language="ZH", device="cpu")
        pipelines["z"] = model
        import librosa
        warm_audio = model.tts_to_file("你好，Hello，欢迎使用本地语音。", model.hps.data.spk2id["ZH"], quiet=True)
        # Include resampling/JIT in startup so the first real request is not delayed.
        librosa.resample(warm_audio, orig_sr=model.hps.data.sampling_rate, target_sr=24000)
        yield
        pipelines.clear()
        return
    if BACKEND != "kokoro":
        raise ValueError(f"Unknown TTS_BACKEND: {BACKEND}")
    from kokoro import KPipeline
    en = KPipeline(lang_code="a", repo_id="hexgrad/Kokoro-82M", device="cpu")
    pipelines["a"] = en
    pipelines["z"] = KPipeline(lang_code="z", repo_id="hexgrad/Kokoro-82M",
                               model=en.model, device="cpu",
                               en_callable=lambda text: en.g2p(text)[0])
    for voice, lang in VOICES.items():
        pipelines[lang].load_voice(voice)
    # Warm both languages before reporting ready.
    for lang, text, voice in [("a", "Hello.", "af_heart"), ("z", "你好。", "zf_xiaobei")]:
        list(pipelines[lang](text, voice=voice))
    yield
    pipelines.clear()


app = FastAPI(title="Local Voice · CPU", lifespan=lifespan)


@app.get("/", include_in_schema=False)
def index():
    return FileResponse(Path(__file__).with_name("tester.html"))


@app.post("/test/transcribe", include_in_schema=False)
async def proxy_transcription(file: UploadFile = File(...), language: str = Form("auto")):
    import httpx
    try:
        data = await file.read(20 * 1024 * 1024 + 1)
    finally:
        await file.close()
    if not data or len(data) > 20 * 1024 * 1024:
        raise HTTPException(413, "Upload a non-empty audio file under 20 MB.")
    try:
        async with httpx.AsyncClient(timeout=120, trust_env=False) as client:
            result = await client.post("http://127.0.0.1:8001/v1/audio/transcriptions",
                files={"file": ("audio.wav", data, "application/octet-stream")},
                data={"language": language})
        return Response(result.content, status_code=result.status_code, media_type="application/json")
    except httpx.HTTPError as exc:
        raise HTTPException(503, "ASR service is unavailable or timed out.") from exc


class SpeechRequest(BaseModel):
    input: str = Field(min_length=1, max_length=2000)
    voice: str = "default"
    model: str | None = None
    speed: float = Field(default=1.0, ge=0.5, le=2.0)
    response_format: Literal["wav"] = "wav"


@app.get("/health")
def health():
    return {"ready": bool(pipelines), "model": "myshell-ai/MeloTTS-Chinese" if BACKEND == "melo" else "hexgrad/Kokoro-82M",
            "backend": BACKEND, "device": "cpu", "threads": int(os.environ.get("TTS_THREADS", "4"))}


@app.get("/v1/audio/voices")
def voices():
    return {"voices": [{"id": v, "name": "Melo · 中文 / 中英混读" if v == "melo_zh" else v,
                        "language": "zh" if lang == "z" else "en"}
                       for v, lang in VOICES.items()]}


@app.post("/v1/audio/speech")
def speech(request: SpeechRequest):
    if not request.input.strip():
        raise HTTPException(422, "Text cannot be blank.")
    voice = next(iter(VOICES)) if request.voice == "default" else request.voice
    if voice not in VOICES or request.model not in (None, BACKEND):
        raise HTTPException(422, "Unknown voice/model; query /v1/audio/voices for this backend.")
    if not lock.acquire(blocking=False):
        raise HTTPException(429, "TTS is busy; try again shortly.")
    try:
        started = time.perf_counter()
        pipeline = pipelines[VOICES[voice]]
        if BACKEND == "melo":
            import librosa
            chunks = []
            for part in bounded_parts(request.input):
                if not any(c.isalnum() for c in part):
                    continue
                audio = pipeline.tts_to_file(part, pipeline.hps.data.spk2id["ZH"],
                                             speed=request.speed, quiet=True)
                if len(audio):
                    chunks.append(librosa.resample(audio, orig_sr=pipeline.hps.data.sampling_rate, target_sr=24000))
        else:
            chunks = [audio.detach().cpu().numpy() for _, _, audio in
                      pipeline(request.input, voice=voice, speed=request.speed)
                      if audio is not None and len(audio)]
        if not chunks:
            raise HTTPException(422, "No speakable text was found.")
        audio = np.concatenate(chunks)
        output = io.BytesIO()
        sf.write(output, audio, 24000, format="WAV", subtype="PCM_16")
        return Response(output.getvalue(), media_type="audio/wav", headers={
            "X-Inference-Seconds": f"{time.perf_counter() - started:.3f}",
            "X-Audio-Seconds": f"{len(audio) / 24000:.3f}",
        })
    finally:
        lock.release()


if __name__ == "__main__":
    import uvicorn
    uvicorn.run(app, host="127.0.0.1", port=8002)
