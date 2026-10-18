"""On-demand local dictation editing; no transcript history is written to disk."""
import importlib.util
from pathlib import Path
import re
import threading
import time

from llama_runtime import LlamaServer

ROOT = Path(__file__).resolve().parent
MODEL = "AgenticASR-Refiner-Q4_K_M"
MODEL_PATH = ROOT / ".runtime/models/AgenticASR-Refiner-GGUF/AgenticASR-Refiner-Q4_K_M.gguf"
CONTEXT_TOKENS = 4096
MAX_OUTPUT_TOKENS = 3072
GPU_LAYERS = 0
IDLE_UNLOAD_SECONDS = 60

# Author's native protocol, pinned in tests/fixtures/dictation/specialist-protocols.json.
RULES = ("你是 ASR 文本纠错助手。保留原意，最小修改：去口癖/重复，修错字，补必要标点，"
         "规范数字、日期、术语和代码符号，处理自我修正。不要总结、扩写或解释。"
         "重要易错实体在末尾追加 <KEY>[词1、词2]；没有则不加。")


def build_messages(text, mode, terms):
    if mode not in {"clean", "notes"}:
        raise ValueError("Unknown text mode.")
    # Optional terms belong to ASR context. The refiner receives unchanged text,
    # without a reference dictionary, few-shot examples, or a JSON wrapper.
    return [{"role": "system", "content": RULES}, {"role": "user", "content": text}]


def edited_body(output):
    """Remove only the author's well-formed trailing metadata, never repair text."""
    if not isinstance(output, str) or not output.strip():
        raise RuntimeError("整理模型返回空内容；完整原文已保留。")
    if any(marker in output for marker in ("<think>", "</think>", "<|im_start|>", "<|im_end|>")):
        raise RuntimeError("整理模型返回了非正文内容；完整原文已保留。")
    if "<KEY>" in output:
        match = re.fullmatch(r"(.*?)<KEY>\[[^\[\]\r\n]*\]\s*", output, re.DOTALL)
        if output.count("<KEY>") != 1 or match is None:
            raise RuntimeError("整理模型返回了不完整的正文标记；完整原文已保留。")
        output = match.group(1)
    if not output.strip():
        raise RuntimeError("整理模型返回空正文；完整原文已保留。")
    return output.strip()


class TextProcessor:
    def __init__(self, idle_seconds=IDLE_UNLOAD_SECONDS):
        self.server = None
        self.device = "cpu"
        self.idle_seconds = idle_seconds
        self._operation_lock = threading.RLock()
        self._state_lock = threading.Lock()
        self._state = "idle"
        self._idle_timer = None
        self._idle_version = 0

    def health(self):
        with self._state_lock:
            installed = MODEL_PATH.is_file()
            ready = self.server is not None and self.server.ready
            state = self._state
            if state not in {"loading", "processing"}:
                state = "ready" if ready else ("idle" if installed else "missing")
            return {"installed": installed, "ready": ready, "state": state,
                    "model": MODEL, "device": self.device, "supports_terms": False,
                    "idle_unload_seconds": self.idle_seconds}

    def _cancel_idle(self):
        # Called with _operation_lock. A callback already waiting for the lock
        # sees an obsolete version after an operation has renewed the timer.
        self._idle_version += 1
        if self._idle_timer is not None:
            self._idle_timer.cancel()
            self._idle_timer = None

    def _schedule_idle(self):
        if self.server is None:
            return
        version = self._idle_version
        timer = threading.Timer(self.idle_seconds, self._unload_idle, args=(version,))
        timer.daemon = True
        self._idle_timer = timer
        timer.start()

    def _unload_idle(self, version):
        with self._operation_lock:
            if version == self._idle_version:
                self._cancel_idle()
                self._drop_server()

    def _drop_server(self):
        with self._state_lock:
            server, self.server = self.server, None
            try:
                if server is not None:
                    server.close()
            finally:
                # Report idle only after the process has actually exited.
                self._state = "idle"

    def _ensure_prepared(self, download=False):
        if self.server is not None and self.server.ready:
            return
        self._drop_server()
        with self._state_lock:
            self._state = "loading"
        try:
            if download:
                spec = importlib.util.spec_from_file_location("download_text_gguf", ROOT / "scripts/download-text-gguf.py")
                downloader = importlib.util.module_from_spec(spec)
                spec.loader.exec_module(downloader)
                downloader.download()
            if not MODEL_PATH.is_file():
                raise FileNotFoundError("未安装整理模型。请在设置中准备整理模型，或运行 scripts/download-text-gguf.py。")
            server = LlamaServer([
                "-m", str(MODEL_PATH), "--device", "none", "-ngl", "0", "-c", str(CONTEXT_TOKENS),
                "-np", "1", "-b", "256", "-ub", "256", "-t", "4", "-tb", "4",
                "-fa", "on", "--fit", "off", "--cache-ram", "0", "--no-cache-idle-slots",
                "--no-context-shift", "--jinja", "--reasoning-format", "auto", "--log-disable",
            ], load_timeout=60)
            with self._state_lock:
                self.server = server
                self._state = "ready"
        except BaseException:
            with self._state_lock:
                self._state = "idle"
            raise

    def prepare(self, download=False):
        with self._operation_lock:
            self._cancel_idle()
            try:
                self._ensure_prepared(download)
                return self.health()
            finally:
                self._schedule_idle()

    def process(self, text, mode, terms):
        with self._operation_lock:
            self._cancel_idle()
            started = time.perf_counter()
            try:
                self._ensure_prepared()
                with self._state_lock:
                    self._state = "processing"
                body = {"messages": build_messages(text, mode, terms), "stream": False,
                        "chat_template_kwargs": {"enable_thinking": False},
                        "temperature": 0, "seed": 42, "cache_prompt": False, "stop": ["<|im_end|>"]}
                try:
                    prompt = self.server.request("/apply-template", body, timeout=5)["prompt"]
                    prompt_tokens = len(self.server.request("/tokenize", {"content": prompt, "add_special": True}, timeout=5)["tokens"])
                    source_tokens = len(self.server.request("/tokenize", {"content": text}, timeout=5)["tokens"])
                    output_tokens = max(256, int(source_tokens * 1.5) + 128)
                    if output_tokens > MAX_OUTPUT_TOKENS or prompt_tokens + output_tokens + 32 > CONTEXT_TOKENS:
                        raise ValueError("文字超过本次整理的上下文或完整输出预算；完整原文已保留，请分段整理。")
                    body["max_tokens"] = output_tokens
                    result = self.server.request("/v1/chat/completions", body,
                                                 timeout=25 if mode == "clean" else 90)
                except ValueError:
                    raise
                except Exception:
                    # A timed-out generation must not keep occupying the local model.
                    self._drop_server()
                    raise
                choice = result["choices"][0]
                if choice.get("finish_reason") != "stop":
                    raise RuntimeError("整理输出未完整生成；完整原文已保留，请缩短内容后重试。")
                output = edited_body(choice["message"].get("content"))
                return {"text": output, "processing_seconds": round(time.perf_counter() - started, 3)}
            finally:
                with self._state_lock:
                    self._state = "ready" if self.server is not None else "idle"
                self._schedule_idle()

    def close(self):
        with self._operation_lock:
            self._cancel_idle()
            self._drop_server()
