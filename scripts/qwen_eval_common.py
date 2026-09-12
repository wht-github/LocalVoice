"""Shared NVML sampling and local HTTP helpers for the ASR experiments."""
import ctypes
import json
import threading
import time
import urllib.request


class MemorySampler:
    """NVML device-wide VRAM, including other applications; sampled every 20 ms."""
    class Info(ctypes.Structure):
        _fields_ = [('total', ctypes.c_ulonglong), ('free', ctypes.c_ulonglong), ('used', ctypes.c_ulonglong)]

    def __init__(self):
        self.lib = ctypes.WinDLL('nvml.dll')
        if self.lib.nvmlInit_v2() != 0:
            raise RuntimeError('NVML initialization failed')
        self.handle = ctypes.c_void_p()
        if self.lib.nvmlDeviceGetHandleByIndex_v2(0, ctypes.byref(self.handle)) != 0:
            raise RuntimeError('NVML GPU 0 unavailable')
        self.samples = []
        self.stop = threading.Event()

    def read(self):
        info = self.Info()
        if self.lib.nvmlDeviceGetMemoryInfo(self.handle, ctypes.byref(info)) != 0:
            raise RuntimeError('NVML memory query failed')
        return info.used / 2**20

    def start(self):
        def loop():
            while not self.stop.is_set():
                self.samples.append((time.perf_counter(), self.read()))
                self.stop.wait(0.02)
        self.thread = threading.Thread(target=loop, daemon=True)
        self.thread.start()

    def close(self):
        self.stop.set()
        self.thread.join()
        self.lib.nvmlShutdown()


def request_json(url, payload=None):
    data = None if payload is None else json.dumps(payload).encode()
    request = urllib.request.Request(url, data=data, headers={'Content-Type': 'application/json'})
    with urllib.request.urlopen(request, timeout=120) as response:
        return json.load(response)
