"""Verify that force-stopping the adapter releases its real llama CUDA process."""
import ctypes
from ctypes import wintypes
import json
import os
from pathlib import Path
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))


def worker():
    from llama_asr import LlamaASR
    model = LlamaASR("qwen-llama-0.6b")
    try:
        print(json.dumps({"adapter_pid": os.getpid(), "server_pid": model.child.pid}), flush=True)
        input()  # Parent deliberately terminates this process, bypassing finally.
    finally:
        model.close()


def check():
    api = ctypes.WinDLL("kernel32", use_last_error=True)
    api.OpenProcess.argtypes = [wintypes.DWORD, wintypes.BOOL, wintypes.DWORD]
    api.OpenProcess.restype = wintypes.HANDLE
    api.TerminateProcess.argtypes = [wintypes.HANDLE, wintypes.UINT]
    api.TerminateProcess.restype = wintypes.BOOL
    api.WaitForSingleObject.argtypes = [wintypes.HANDLE, wintypes.DWORD]
    api.WaitForSingleObject.restype = wintypes.DWORD
    api.CloseHandle.argtypes = [wintypes.HANDLE]
    server = adapter = None
    child = subprocess.Popen([sys.executable, "-u", __file__, "--worker"],
                             stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True,
                             creationflags=subprocess.CREATE_NO_WINDOW)
    try:
        report = json.loads(child.stdout.readline())
        server = api.OpenProcess(0x100000, False, report["server_pid"])
        adapter = api.OpenProcess(1, False, report["adapter_pid"])
        if not server or not adapter or not api.TerminateProcess(adapter, 1):
            raise ctypes.WinError(ctypes.get_last_error())
        assert api.WaitForSingleObject(server, 10000) == 0, "llama-server survived adapter termination"
        child.wait(timeout=10)
        report["forced_termination_cleanup"] = True
        print(json.dumps(report))
    finally:
        for handle in (server, adapter):
            if handle:
                api.CloseHandle(handle)
        if child.poll() is None:
            subprocess.run(["taskkill", "/PID", str(child.pid), "/T", "/F"], check=False,
                           creationflags=subprocess.CREATE_NO_WINDOW)
            child.wait(timeout=10)


if __name__ == "__main__":
    worker() if "--worker" in sys.argv else check()
