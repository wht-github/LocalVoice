"""Small adapter for the pinned local llama.cpp server; no Torch model is loaded."""
import base64
import ctypes
from ctypes import wintypes
import io
import json
import os
from pathlib import Path
import secrets
import socket
import subprocess
import time
import urllib.error
import urllib.request

import numpy as np
import soundfile as sf

SIZES = {"qwen-llama-0.6b": "0.6B", "qwen-llama-1.7b": "1.7B"}


class WindowsJob:
    """Close the child process tree even when Rust force-stops this Python process."""

    def __init__(self):
        class Basic(ctypes.Structure):
            _fields_ = [("process_time", ctypes.c_int64), ("job_time", ctypes.c_int64),
                        ("flags", wintypes.DWORD), ("min_ws", ctypes.c_size_t),
                        ("max_ws", ctypes.c_size_t), ("active", wintypes.DWORD),
                        ("affinity", ctypes.c_size_t), ("priority", wintypes.DWORD),
                        ("scheduling", wintypes.DWORD)]

        class Extended(ctypes.Structure):
            _fields_ = [("basic", Basic), ("io", ctypes.c_uint64 * 6),
                        ("process_memory", ctypes.c_size_t), ("job_memory", ctypes.c_size_t),
                        ("peak_process", ctypes.c_size_t), ("peak_job", ctypes.c_size_t)]

        self.api = ctypes.WinDLL("kernel32", use_last_error=True)
        self.api.CreateJobObjectW.argtypes = [ctypes.c_void_p, wintypes.LPCWSTR]
        self.api.CreateJobObjectW.restype = wintypes.HANDLE
        self.api.SetInformationJobObject.argtypes = [wintypes.HANDLE, ctypes.c_int, ctypes.c_void_p, wintypes.DWORD]
        self.api.SetInformationJobObject.restype = wintypes.BOOL
        self.api.AssignProcessToJobObject.argtypes = [wintypes.HANDLE, wintypes.HANDLE]
        self.api.AssignProcessToJobObject.restype = wintypes.BOOL
        self.api.CloseHandle.argtypes = [wintypes.HANDLE]
        self.api.CloseHandle.restype = wintypes.BOOL
        self.handle = self.api.CreateJobObjectW(None, None)
        if not self.handle:
            raise ctypes.WinError(ctypes.get_last_error())
        limits = Extended()
        limits.basic.flags = 0x2000  # JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE
        if not self.api.SetInformationJobObject(self.handle, 9, ctypes.byref(limits), ctypes.sizeof(limits)):
            error = ctypes.WinError(ctypes.get_last_error())
            self.close()
            raise error

    def assign(self, child):
        if not self.api.AssignProcessToJobObject(self.handle, int(child._handle)):
            raise ctypes.WinError(ctypes.get_last_error())

    def close(self):
        if self.handle:
            self.api.CloseHandle(self.handle)
            self.handle = None


class LlamaASR:
    def __init__(self, mode):
        root = Path(__file__).resolve().parent
        size = SIZES[mode]
        directory = root / f".runtime/models/Qwen3-ASR-{size}-GGUF"
        decoder = directory / f"Qwen3-ASR-{size}-Q8_0.gguf"
        encoder = directory / f"mmproj-Qwen3-ASR-{size}-bf16.gguf"
        executable = root / ".runtime/llama-build/bin/llama-server.exe"
        for path in (executable, decoder, encoder):
            if not path.is_file():
                raise FileNotFoundError(f"缺少 {path}；请按 docs/qwen-llama-desktop.md 安装环境和模型。")
        self.child = None
        self.job = None
        self.key = secrets.token_hex(32)
        self.http = urllib.request.build_opener(urllib.request.ProxyHandler({}))
        # Let the OS choose a free loopback port. A bind race fails startup rather
        # than attaching to another server, whose API key will not match ours.
        with socket.socket() as sock:
            sock.bind(("127.0.0.1", 0))
            port = sock.getsockname()[1]
        self.url = f"http://127.0.0.1:{port}"
        env = os.environ.copy()
        env["PATH"] = str(root / ".venv-llama-build/Lib/site-packages/nvidia/cu13/bin/x86_64") + os.pathsep + env["PATH"]
        env["LLAMA_API_KEY"] = self.key
        for name in ("GGML_CUDA_DISABLE_GRAPHS", "GGML_CUDA_GRAPH_OPT", "GGML_CUDA_PDL"):
            env.pop(name, None)
        command = [str(executable), "-m", str(decoder), "--mmproj", str(encoder),
                   "--host", "127.0.0.1", "--port", str(port), "--device", "CUDA0", "-ngl", "99",
                   "-c", "2048", "-b", "256", "-ub", "256", "-np", "1", "-t", "4", "-tb", "4",
                   "-fa", "on", "-ctk", "f16", "-ctv", "f16", "--fit", "off",
                   "--cache-ram", "0", "--no-cache-idle-slots", "--no-context-shift"]
        try:
            self.job = WindowsJob()
            # Inherit the adapter's native-asr.log handles, never an undrained PIPE.
            self.child = subprocess.Popen(command, env=env, stdin=subprocess.DEVNULL,
                                          creationflags=subprocess.CREATE_NO_WINDOW)
            self.job.assign(self.child)
            deadline = time.monotonic() + 180
            while True:
                if self.child.poll() is not None:
                    raise RuntimeError(f"llama.cpp 启动失败（{self.child.returncode}），请查看 native-asr.log。")
                try:
                    if self.request("/health", timeout=2).get("status") == "ok":
                        break
                except (OSError, urllib.error.URLError):
                    pass
                if time.monotonic() >= deadline:
                    raise TimeoutError("llama.cpp 模型加载超时。")
                time.sleep(0.2)
        except BaseException:
            self.close()
            raise

    def request(self, path, data=None, timeout=90):
        request = urllib.request.Request(self.url + path,
            data=json.dumps(data).encode() if data is not None else None,
            headers={"Content-Type": "application/json", "Authorization": f"Bearer {self.key}"})
        with self.http.open(request, timeout=timeout) as response:
            return json.load(response)

    def transcribe(self, samples, language, warmup=False):
        if not warmup and not np.any(samples):
            return {"text": "", "language": language, "tags": [], "raw_text": ""}
        audio = io.BytesIO()
        sf.write(audio, samples, 16000, format="WAV", subtype="PCM_16")
        messages = [{"role": "user", "content": [{"type": "input_audio", "input_audio": {
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

    def close(self):
        if self.child is not None:
            if self.child.poll() is None:
                self.child.terminate()
            self.child.wait(timeout=10)
            self.child = None
        if self.job is not None:
            self.job.close()
            self.job = None
