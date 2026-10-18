"""One owned llama-server process, isolated from other local model servers."""
import ctypes
from ctypes import wintypes
import json
import os
from pathlib import Path
import secrets
import socket
import subprocess
import time
import urllib.error
import urllib.request


class WindowsJob:
    """Close the process tree when the parent is force-stopped by the desktop."""

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


class LlamaServer:
    def __init__(self, arguments, load_timeout=180):
        executable = Path(__file__).resolve().parent / ".runtime/llama/llama-server.exe"
        if not executable.is_file():
            raise FileNotFoundError("缺少 llama.cpp；请运行 scripts/setup-llama.ps1。")
        self.child = None
        self.job = None
        self.key = secrets.token_hex(32)
        self.http = urllib.request.build_opener(urllib.request.ProxyHandler({}))
        # A bind race fails authentication/startup; it never attaches to another server.
        with socket.socket() as sock:
            sock.bind(("127.0.0.1", 0))
            port = sock.getsockname()[1]
        self.url = f"http://127.0.0.1:{port}"
        env = os.environ.copy()
        env["LLAMA_API_KEY"] = self.key
        for name in ("GGML_CUDA_DISABLE_GRAPHS", "GGML_CUDA_GRAPH_OPT", "GGML_CUDA_PDL"):
            env.pop(name, None)
        command = [str(executable), "--host", "127.0.0.1", "--port", str(port), *arguments]
        try:
            self.job = WindowsJob()
            self.child = subprocess.Popen(command, env=env, stdin=subprocess.DEVNULL,
                                          creationflags=subprocess.CREATE_NO_WINDOW)
            self.job.assign(self.child)
            deadline = time.monotonic() + load_timeout
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

    @property
    def ready(self):
        return self.child is not None and self.child.poll() is None

    def close(self):
        try:
            if self.child is not None:
                if self.child.poll() is None:
                    self.child.terminate()
                try:
                    self.child.wait(timeout=10)
                except subprocess.TimeoutExpired:
                    self.child.kill()
                    self.child.wait(timeout=5)
        finally:
            self.child = None
            if self.job is not None:
                self.job.close()
                self.job = None
