"""Offline model smoke test and repeatable Mandarin/mixed reading samples."""
import asyncio
import json
import os
from pathlib import Path
import sys
import time

os.environ["TTS_BACKEND"] = "melo"
os.environ["HF_HUB_OFFLINE"] = "1"
os.environ["TRANSFORMERS_OFFLINE"] = "1"
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import tts_server as server

async def main():
    start = time.perf_counter()
    async with server.lifespan(server.app):
        results = {"startup_seconds": round(time.perf_counter() - start, 3), "samples": []}
        for name, text in [
            ("mandarin", "下午好。忙碌了一天，不妨给自己留一点安静的时间。打开窗，让微风吹进房间。接下来，我会用自然的语速，为你读完这段文字。"),
            ("mixed", "今天我们用 Rust 编写一个小工具，点击 Start 按钮就能开始。The application runs locally and keeps your data on this computer. 保存设置以后，继续完成今天的工作。"),
            ("english", "Take a short break and enjoy a quiet moment. The application reads each sentence in order, so you can focus on your work."),
        ]:
            response = server.speech(server.SpeechRequest(input=text, voice="melo_zh"))
            path = Path(server.__file__).parent / "outputs" / f"melo-{name}.wav"
            path.write_bytes(response.body)
            results["samples"].append({"name": name, "text": text,
                "inference_seconds": float(response.headers["x-inference-seconds"]),
                "audio_seconds": float(response.headers["x-audio-seconds"])})
        (Path(server.__file__).parent / "outputs/melo-samples.json").write_text(json.dumps(results, ensure_ascii=False, indent=2))
        print(json.dumps(results, ensure_ascii=False, indent=2))

asyncio.run(main())
