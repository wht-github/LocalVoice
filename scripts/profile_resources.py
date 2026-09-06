"""Sample the two real services during concurrent local requests (run as root)."""
import concurrent.futures
import io
import json
import subprocess
import threading
import time
from pathlib import Path

import httpx
import numpy as np
import soundfile as sf

ROOT = Path(__file__).resolve().parents[1]
UNITS = ["local-voice-asr.service", "local-voice-tts.service"]
services = {}
for unit in UNITS:
    raw = subprocess.check_output(["systemctl", "show", unit, "-p", "MainPID", "-p", "ControlGroup"], text=True)
    props = dict(line.split("=", 1) for line in raw.splitlines())
    services[unit] = (int(props["MainPID"]), Path("/sys/fs/cgroup") / props["ControlGroup"].lstrip("/"))

def sample():
    result = {"time": time.monotonic(), "services": {}}
    for unit, (pid, cg) in services.items():
        smaps = dict((parts[0].rstrip(":"), int(parts[1])) for line in Path(f"/proc/{pid}/smaps_rollup").read_text().splitlines()[1:] if len(parts := line.split()) >= 2)
        cpu = dict(line.split() for line in (cg / "cpu.stat").read_text().splitlines())
        result["services"][unit] = {"rss_mib": smaps["Rss"] / 1024, "pss_mib": smaps["Pss"] / 1024,
            "cgroup_mib": int((cg / "memory.current").read_text()) / 2**20,
            "lifetime_peak_mib": int((cg / "memory.peak").read_text()) / 2**20,
            "cpu_usec": int(cpu["usage_usec"])}
    mem = dict((parts[0].rstrip(":"), int(parts[1])) for line in Path("/proc/meminfo").read_text().splitlines() if len(parts := line.split()) >= 2)
    result["linux_used_mib"] = (mem["MemTotal"] - mem["MemAvailable"]) / 1024
    return result

baseline = sample()
audio, rate = sf.read(ROOT / "outputs/tts-zh.wav")
audio = np.tile(audio, int(np.ceil(25 * rate / len(audio))))[:25 * rate]
buf = io.BytesIO()
sf.write(buf, audio, rate, format="WAV")
requests = []

def exercise(kind):
    with httpx.Client(timeout=180, trust_env=False) as client:
        for _ in range(3):
            start = time.monotonic()
            if kind == "asr":
                r = client.post("http://127.0.0.1:8001/v1/audio/transcriptions", files={"file": ("25s.wav", buf.getvalue(), "audio/wav")}, data={"language": "zh"})
            else:
                r = client.post("http://127.0.0.1:8002/v1/audio/speech", json={"voice": "default", "input": "这是一次本地语音服务的资源测试。我们同时运行语音识别与语音合成，观察内存和处理器的实际占用。Hello, welcome to the local voice assistant. 请保持自然的语速，清楚地读出每一句话。"})
            r.raise_for_status()
            requests.append({"kind": kind, "elapsed_seconds": round(time.monotonic()-start, 3)})

samples = [baseline]
with concurrent.futures.ThreadPoolExecutor(max_workers=2) as pool:
    jobs = [pool.submit(exercise, kind) for kind in ("asr", "tts")]
    while not all(job.done() for job in jobs):
        samples.append(sample())
        time.sleep(0.15)
    for job in jobs:
        job.result()
samples.append(sample())
result = {"sample_interval_seconds": 0.15, "asr_input_seconds": 25, "requests": requests, "baseline": baseline,
          "after": samples[-1], "max_linux_used_mib": max(s["linux_used_mib"] for s in samples), "peaks": {}}
for unit in UNITS:
    result["peaks"][unit] = {key: round(max(s["services"][unit][key] for s in samples), 1) for key in ("rss_mib", "pss_mib", "cgroup_mib", "lifetime_peak_mib")}
result["combined_peak_pss_mib"] = round(max(sum(v["pss_mib"] for v in s["services"].values()) for s in samples), 1)
result["peak_cpu_core_equivalents"] = round(max(sum(b["services"][u]["cpu_usec"] - a["services"][u]["cpu_usec"] for u in UNITS) / 1e6 / (b["time"] - a["time"]) for a,b in zip(samples,samples[1:])), 2)
result["average_cpu_core_equivalents"] = round(sum(samples[-1]["services"][u]["cpu_usec"] - baseline["services"][u]["cpu_usec"] for u in UNITS) / 1e6 / (samples[-1]["time"] - baseline["time"]), 2)
out = ROOT / "outputs/resource-profile.json"
out.write_text(json.dumps(result, ensure_ascii=False, indent=2), encoding="utf-8")
print(json.dumps(result, ensure_ascii=False, indent=2))
