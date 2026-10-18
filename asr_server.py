"""Local STT: SenseVoice CPU or Qwen ASR through official llama.cpp CUDA binaries."""
import io
import json
import os
import re
import threading
import time
from contextlib import asynccontextmanager
from pathlib import Path
from typing import Annotated, Literal

# Set caches before importing model/JIT libraries. Every clone owns its runtime.
CACHE = Path(__file__).resolve().parent / ".runtime/cache"
os.environ["HF_HOME"] = str(CACHE / "huggingface")
os.environ["HF_HUB_CACHE"] = str(CACHE / "huggingface/hub")
os.environ["MODELSCOPE_CACHE"] = str(CACHE / "modelscope")
os.environ["NUMBA_CACHE_DIR"] = str(CACHE / "numba")
os.environ["TORCH_HOME"] = str(CACHE / "torch")
os.environ.setdefault("HF_HUB_DISABLE_XET", "1")
os.environ.setdefault("HF_ENDPOINT", "https://hf-mirror.com")

import numpy as np
import soundfile as sf
from fastapi import FastAPI, File, Form, HTTPException, UploadFile
from pydantic import BaseModel, Field, StringConstraints, TypeAdapter, ValidationError
from starlette.concurrency import run_in_threadpool
from llama_text import TextProcessor

BACKEND = os.environ.get("ASR_BACKEND", "sensevoice-cpu")
LLAMA = BACKEND in {"qwen-llama-0.6b", "qwen-llama-1.7b"}
if not LLAMA and BACKEND != "sensevoice-cpu":
    raise ValueError("Unknown ASR_BACKEND")
MODEL_ID = "FunAudioLLM/SenseVoiceSmall"
SENSEVOICE_REVISION = "3847d57b6bdf2dd8875cb1508d2af43d80a16bf7"
if LLAMA:
    from llama_asr import SIZES
    MODEL_ID = f"Qwen3-ASR-{SIZES[BACKEND]}-Q8_0"
DEVICE = "cpu" if BACKEND == "sensevoice-cpu" else "cuda:0"
MAX_BYTES = 20 * 1024 * 1024
MAX_SECONDS = 30
lock = threading.Lock()
model = None
text_model = TextProcessor()
text_lock = threading.Lock()

TermName = Annotated[str, StringConstraints(strip_whitespace=True, min_length=1, max_length=80)]


class Term(BaseModel):
    text: TermName
    aliases: list[TermName] = Field(default_factory=list, max_length=8)
    description: Annotated[str, StringConstraints(strip_whitespace=True, max_length=160)] = ""


Terms = Annotated[list[Term], Field(max_length=64)]
terms_adapter = TypeAdapter(Terms)


def term_dicts(terms):
    result = [term.model_dump() for term in terms]
    if len(json.dumps(result, ensure_ascii=False)) > 12000:
        raise HTTPException(422, "术语表超过 12,000 字符，请减少本次术语或说明。")
    return result


class TextRequest(BaseModel):
    text: Annotated[str, StringConstraints(strip_whitespace=True, min_length=1, max_length=12000)]
    mode: Literal["clean", "notes"]
    terms: Terms = Field(default_factory=list)


class PrepareRequest(BaseModel):
    download: bool = False


@asynccontextmanager
async def asr_lifespan(app):
    global model
    if LLAMA:
        from llama_asr import LlamaASR
        model = LlamaASR(BACKEND)
        try:
            import librosa
            warm_audio = librosa.resample(np.zeros(24000, dtype=np.float32), orig_sr=24000, target_sr=16000)
            infer(warm_audio, "auto", warmup=True)
            yield
        finally:
            model.close()
            model = None
        return
    import torch
    torch.set_num_threads(int(os.environ.get("ASR_THREADS", "4")))
    from funasr import AutoModel
    from huggingface_hub import snapshot_download
    from huggingface_hub.errors import LocalEntryNotFoundError
    path = os.environ.get("ASR_MODEL")
    if not path:
        try:
            path = snapshot_download(MODEL_ID, revision=SENSEVOICE_REVISION, local_files_only=True)
        except LocalEntryNotFoundError:
            path = snapshot_download(MODEL_ID, revision=SENSEVOICE_REVISION)
    model = AutoModel(
        model=path, hub="hf", device="cpu", ncpu=4,
        disable_update=True, trust_remote_code=False,
    )
    # Complete lazy imports/JIT and a short inference before accepting requests.
    import librosa
    warm_audio = librosa.resample(np.zeros(24000, dtype=np.float32), orig_sr=24000, target_sr=16000)
    infer(warm_audio, "auto", warmup=True)
    try:
        yield
    finally:
        model = None


@asynccontextmanager
async def lifespan(app):
    try:
        async with asr_lifespan(app):
            yield
    finally:
        text_model.close()


app = FastAPI(title="Local speech recognition", lifespan=lifespan)


@app.get("/health")
def health():
    ready = model is not None and (not LLAMA or model.ready)
    return {"ready": ready, "model": MODEL_ID,
            "backend": "llama.cpp" if LLAMA else "funasr",
            "mode": BACKEND, "device": DEVICE, "supports_terms": LLAMA}


def infer(samples, language, warmup=False, terms=None):
    if LLAMA:
        return model.transcribe(samples, language, warmup, terms)
    result = model.generate(input=samples, cache={}, language=language,
                            use_itn=True, batch_size=1, disable_pbar=True)[0]
    raw = result["text"]
    tags = re.findall(r"<\|([^|]+)\|>", raw)
    detected = next((tag for tag in tags if tag in {"zh", "en", "yue", "ja", "ko", "nospeech"}), language)
    return {"text": re.sub(r"<\|[^|]+\|>", "", raw).strip(),
            "language": detected, "tags": tags, "raw_text": raw}


def transcribe(data, language, terms=None):
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
        try:
            result = infer(samples, language, terms=terms)
        except ValueError as exc:
            raise HTTPException(422, str(exc)) from exc
        return {**result,
                "terms_applied": bool(LLAMA and terms),
                "duration": round(len(samples) / 16000, 3),
                "inference_seconds": round(time.perf_counter() - started, 3)}
    finally:
        lock.release()


@app.post("/v1/audio/transcriptions")
async def transcription(file: UploadFile = File(...), language: str = Form("auto"),
                        terms: str = Form("[]")):
    languages = {"auto": "auto", "zh": "zh", "en": "en"}
    if language not in languages:
        raise HTTPException(422, "language must be auto, zh, or en.")
    try:
        parsed_terms = term_dicts(terms_adapter.validate_json(terms))
    except ValidationError as exc:
        raise HTTPException(422, "terms 必须是有效术语数组：每条包含 text、可选 aliases 和 description。") from exc
    try:
        data = await file.read(MAX_BYTES + 1)
    finally:
        await file.close()
    if not data or len(data) > MAX_BYTES:
        raise HTTPException(413, "Upload a non-empty audio file under 20 MB.")
    return await run_in_threadpool(transcribe, data, languages[language], parsed_terms)


@app.get("/v1/text/health")
def text_health():
    return text_model.health()


def text_operation(operation, *args):
    if not text_lock.acquire(blocking=False):
        raise HTTPException(429, "整理模型正在准备或处理文字，请稍后重试。")
    try:
        return operation(*args)
    except FileNotFoundError as exc:
        raise HTTPException(409, str(exc)) from exc
    except ValueError as exc:
        raise HTTPException(422, str(exc)) from exc
    except TimeoutError as exc:
        raise HTTPException(504, "本地整理超时；完整原文已保留。") from exc
    except RuntimeError as exc:
        raise HTTPException(503, str(exc)) from exc
    except Exception as exc:
        raise HTTPException(502, "本地整理服务暂不可用；完整原文已保留。") from exc
    finally:
        text_lock.release()


@app.post("/v1/text/prepare")
async def text_prepare(request: PrepareRequest = PrepareRequest()):
    return await run_in_threadpool(text_operation, text_model.prepare, request.download)


@app.post("/v1/text/process")
async def text_process(request: TextRequest):
    terms = term_dicts(request.terms)
    return await run_in_threadpool(text_operation, text_model.process, request.text, request.mode, terms)


if __name__ == "__main__":
    import uvicorn
    uvicorn.run(app, host=os.environ.get("ASR_HOST", "127.0.0.1"),
                port=int(os.environ.get("ASR_PORT", "8001")))
