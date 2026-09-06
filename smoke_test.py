"""Real bilingual TTS -> ASR check; saves audio and timings for inspection."""
import io
import json
import time
from pathlib import Path

import httpx
import numpy as np
import soundfile as sf

output = Path(__file__).resolve().parent / "outputs"
output.mkdir(exist_ok=True)
results = []
with httpx.Client(timeout=300, trust_env=False) as client:
    for port in (8001, 8002):
        response = client.get(f"http://127.0.0.1:{port}/health")
        response.raise_for_status()
        assert response.json()["ready"], response.text
    for lang, voice, text in [
        ("zh", "default", "你好，这是本地语音测试。今天我们尝试中文语音识别。"),
        ("en", "default", "Hello, this is a local voice test. Speech recognition runs on this computer."),
    ]:
        started = time.perf_counter()
        speech = client.post("http://127.0.0.1:8002/v1/audio/speech",
                             json={"input": text, "voice": voice})
        speech.raise_for_status()
        samples, rate = sf.read(io.BytesIO(speech.content))
        assert rate == 24000 and len(samples) > rate and np.isfinite(samples).all()
        assert float(np.max(np.abs(samples))) > 0.001, "Silent output"
        (output / f"tts-{lang}.wav").write_bytes(speech.content)
        endpoint = "http://127.0.0.1:8001/v1/audio/transcriptions" if lang == "zh" else "http://127.0.0.1:8002/test/transcribe"
        transcription = client.post(endpoint,
            files={"file": (f"{lang}.wav", speech.content, "audio/wav")},
            data={"language": lang})
        transcription.raise_for_status()
        item = {"language": lang, "input": text, "asr": transcription.json(),
                "tts_seconds": speech.headers.get("x-inference-seconds"),
                "audio_seconds": len(samples) / rate,
                "roundtrip_seconds": round(time.perf_counter() - started, 3)}
        results.append(item)
        print(json.dumps(item, ensure_ascii=False), flush=True)
        assert item["asr"]["text"].strip(), "Empty transcription"
    # Confirm invalid text and malformed audio are rejected without inference.
    assert client.post("http://127.0.0.1:8002/v1/audio/speech",
                       json={"input": "  "}).status_code == 422
    assert client.post("http://127.0.0.1:8001/v1/audio/transcriptions",
                       files={"file": ("bad.wav", b"invalid")}).status_code == 400
(output / "smoke-results.json").write_text(json.dumps(results, ensure_ascii=False, indent=2), encoding="utf-8")
