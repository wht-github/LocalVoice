"""Small adapter for the pinned local llama.cpp server; no Torch model is loaded."""
import base64
import io
from pathlib import Path

from llama_runtime import LlamaServer

import numpy as np
import soundfile as sf

SIZES = {"qwen-llama-0.6b": "0.6B", "qwen-llama-1.7b": "1.7B"}


class LlamaASR(LlamaServer):
    def __init__(self, mode):
        root = Path(__file__).resolve().parent
        size = SIZES[mode]
        directory = root / f".runtime/models/Qwen3-ASR-{size}-GGUF"
        decoder = directory / f"Qwen3-ASR-{size}-Q8_0.gguf"
        encoder = directory / f"mmproj-Qwen3-ASR-{size}-bf16.gguf"
        executable = root / ".runtime/llama/llama-server.exe"
        for path in (executable, decoder, encoder):
            if not path.is_file():
                raise FileNotFoundError(f"缺少 {path}；请按 docs/qwen-llama-desktop.md 安装环境和模型。")
        super().__init__(["-m", str(decoder), "--mmproj", str(encoder),
                   "--device", "CUDA0", "-ngl", "99",
                   "-c", "2048", "-b", "256", "-ub", "256", "-np", "1", "-t", "4", "-tb", "4",
                   "-fa", "on", "-ctk", "f16", "-ctv", "f16", "--fit", "off",
                   "--cache-ram", "0", "--no-cache-idle-slots", "--no-context-shift"])

    def transcribe(self, samples, language, warmup=False, terms=None):
        if not warmup and not np.any(samples):
            return {"text": "", "language": language, "tags": [], "raw_text": ""}
        audio = io.BytesIO()
        sf.write(audio, samples, 16000, format="WAV", subtype="PCM_16")
        # Official Qwen3-ASR uses a system message as its recognition context.
        # Only canonical names bias ASR; descriptions and aliases belong to editing.
        context = ", ".join(term["text"] for term in terms or [])
        if context:
            tokens = self.request("/tokenize", {"content": context}, timeout=5)["tokens"]
            if len(tokens) > 512:
                raise ValueError("术语上下文超过 512 tokens；请减少本次术语。")
        messages = [{"role": "system", "content": context}, {"role": "user", "content": [{"type": "input_audio", "input_audio": {
            "data": base64.b64encode(audio.getvalue()).decode(), "format": "wav"}}]}]
        body = {"messages": messages, "temperature": 0, "seed": 42, "max_tokens": 512,
                "cache_prompt": False, "stream": False}
        prefix = ""
        if language != "auto":
            prefix = "language " + {"zh": "Chinese", "en": "English"}[language] + "<asr_text>"
            messages.append({"role": "assistant", "content": prefix})
            body.update(continue_final_message=True, add_generation_prompt=False)
        result = self.request("/v1/chat/completions", body)
        choice = result["choices"][0]
        if choice["finish_reason"] != "stop":
            raise RuntimeError("识别输出超过限制，请缩短录音后重试。")
        raw = choice["message"]["content"]
        # llama.cpp can include the assistant prefill in the response.
        if prefix and not raw.startswith(prefix):
            raw = prefix + raw
        header, separator, text = raw.partition("<asr_text>")
        if not separator:
            raise RuntimeError("llama.cpp 返回了无法解析的识别结果。")
        detected = header.removeprefix("language ").strip() or language
        return {"text": text.strip() if np.any(samples) else "", "language": detected,
                "tags": [], "raw_text": raw}
