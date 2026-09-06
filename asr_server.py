"""Compatible local STT endpoint: SenseVoice CPU or Qwen3-ASR vLLM."""
import io
import os
import re
import threading
import time
from contextlib import asynccontextmanager

import numpy as np
import soundfile as sf
from fastapi import FastAPI, File, Form, HTTPException, UploadFile
from starlette.concurrency import run_in_threadpool

BACKEND = os.environ.get("ASR_BACKEND", "sensevoice-cpu")
if BACKEND not in {"sensevoice-cpu", "qwen-vllm"}:
    raise ValueError("Unknown ASR_BACKEND")
MODEL_ID = "Qwen/Qwen3-ASR-0.6B" if BACKEND == "qwen-vllm" else "FunAudioLLM/SenseVoiceSmall"
DEVICE = "cpu" if BACKEND == "sensevoice-cpu" else "cuda:0"
MAX_BYTES = 20 * 1024 * 1024
MAX_SECONDS = 30
lock = threading.Lock()
model = None


@asynccontextmanager
async def lifespan(app):
    global model
    import torch
    torch.set_num_threads(int(os.environ.get("ASR_THREADS", "4")))
    if DEVICE != "cpu" and not torch.cuda.is_available():
        raise RuntimeError("CUDA unavailable; choose SenseVoice CPU mode")
    if BACKEND == "qwen-vllm":
        from qwen_asr import Qwen3ASRModel
        from pathlib import Path
        path = (Path(os.environ["VOICE_RUNTIME"]) / "qwen-model.path").read_text().strip()
        model = Qwen3ASRModel.LLM(
            model=path, gpu_memory_utilization=0.60,
            max_model_len=3072, max_num_seqs=1, max_new_tokens=1024,
            enforce_eager=True,
        )
    else:
        from funasr import AutoModel
        model = AutoModel(
            model=os.environ.get("ASR_MODEL", MODEL_ID),
            hub="hf", device=DEVICE, ncpu=4, disable_update=True,
            trust_remote_code=False,
        )
    # Complete lazy imports/JIT and a short inference before accepting requests.
    import librosa
    warm_audio = librosa.resample(np.zeros(24000, dtype=np.float32), orig_sr=24000, target_sr=16000)
    infer(warm_audio, "auto")
    yield
    model = None


app = FastAPI(title="Local speech recognition", lifespan=lifespan)


@app.get("/health")
def health():
    return {"ready": model is not None, "model": MODEL_ID,
            "backend": "vllm" if BACKEND == "qwen-vllm" else "funasr",
            "mode": BACKEND, "device": DEVICE}


def infer(samples, language):
    if BACKEND == "qwen-vllm":
        result = model.transcribe(audio=(samples, 16000),
                                 language={"auto": None, "zh": "Chinese", "en": "English"}[language])[0]
        return {"text": result.text, "language": result.language, "tags": [], "raw_text": result.text}
    result = model.generate(input=samples, cache={}, language=language,
                            use_itn=True, batch_size=1, disable_pbar=True)[0]
    raw = result["text"]
    tags = re.findall(r"<\|([^|]+)\|>", raw)
    detected = next((tag for tag in tags if tag in {"zh", "en", "yue", "ja", "ko", "nospeech"}), language)
    return {"text": re.sub(r"<\|[^|]+\|>", "", raw).strip(),
            "language": detected, "tags": tags, "raw_text": raw}


def transcribe(data, language):
    try:
        with sf.SoundFile(io.BytesIO(data)) as source:
            if source.frames <= 0 or source.samplerate <= 0:
                raise ValueError("Empty audio")
            if source.frames / source.samplerate > MAX_SECONDS:
                raise HTTPException(413, "Please use a recording of 30 seconds or less.")
            samples = source.read(dtype="float32", always_2d=True).mean(axis=1)
            sample_rate = source.samplerate
    except HTTPException:
        raise
    except Exception as exc:
        raise HTTPException(400, "Invalid audio. Upload a WAV, FLAC, or OGG file.") from exc
    if not np.isfinite(samples).all():
        raise HTTPException(400, "Audio contains invalid samples.")
    if not lock.acquire(blocking=False):
        raise HTTPException(429, "ASR is busy; try again shortly.")
    try:
        started = time.perf_counter()
        import librosa
        if sample_rate != 16000:
            samples = librosa.resample(samples, orig_sr=sample_rate, target_sr=16000)
        result = infer(samples, language)
        return {**result,
                "duration": round(len(samples) / 16000, 3),
                "inference_seconds": round(time.perf_counter() - started, 3)}
    finally:
        lock.release()


@app.post("/v1/audio/transcriptions")
async def transcription(file: UploadFile = File(...), language: str = Form("auto")):
    languages = {"auto": "auto", "zh": "zh", "en": "en"}
    if language not in languages:
        raise HTTPException(422, "language must be auto, zh, or en.")
    try:
        data = await file.read(MAX_BYTES + 1)
    finally:
        await file.close()
    if not data or len(data) > MAX_BYTES:
        raise HTTPException(413, "Upload a non-empty audio file under 20 MB.")
    return await run_in_threadpool(transcribe, data, languages[language])


if __name__ == "__main__":
    import uvicorn
    uvicorn.run(app, host="127.0.0.1", port=8001)
