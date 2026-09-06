"""Exercise ASR with the official example recordings bundled in the model."""
import io
import json
import os
from pathlib import Path

import httpx
import soundfile as sf

root = Path(os.environ["MODELSCOPE_CACHE"])
results = []
with httpx.Client(timeout=180, trust_env=False) as client:
    for lang in ["zh", "en"]:
        source = next(root.rglob(f"example/{lang}.mp3"))
        samples, rate = sf.read(source)
        wav = io.BytesIO()
        sf.write(wav, samples, rate, format="WAV")
        response = client.post("http://127.0.0.1:8001/v1/audio/transcriptions",
            files={"file": (f"{lang}.wav", wav.getvalue(), "audio/wav")}, data={"language": "auto"})
        response.raise_for_status()
        result = response.json()
        assert result["text"].strip()
        results.append({"source": str(source), "result": result})
        print(json.dumps(results[-1], ensure_ascii=False), flush=True)
output = Path(__file__).resolve().parents[1] / "outputs"
output.mkdir(exist_ok=True)
(output / "asr-official-samples.json").write_text(json.dumps(results, ensure_ascii=False, indent=2), encoding="utf-8")
