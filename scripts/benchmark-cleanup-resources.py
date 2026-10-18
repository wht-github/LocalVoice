"""Measure an explicit local text model with fixed dictation development cases.

Windows only; requires the existing pinned llama.cpp runtime, nvidia-smi and the
project Python environment. Starts and closes only its own random-port servers.
No microphone, user app controls, model downloads or app settings are involved.
"""
import argparse
import ctypes
from ctypes import wintypes
import hashlib
import json
import os
from pathlib import Path
import platform
import statistics
import subprocess
import sys
import threading
import time

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))
from llama_runtime import LlamaServer
from llama_text import build_messages

MIB = 1024 ** 2
KERNEL = ctypes.WinDLL("kernel32", use_last_error=True)
PSAPI = ctypes.WinDLL("psapi", use_last_error=True)

class ProcessEntry(ctypes.Structure):
    _fields_ = [("size", wintypes.DWORD), ("usage", wintypes.DWORD),
                ("pid", wintypes.DWORD), ("heap", ctypes.c_size_t),
                ("module", wintypes.DWORD), ("threads", wintypes.DWORD),
                ("parent", wintypes.DWORD), ("priority", wintypes.LONG),
                ("flags", wintypes.DWORD), ("exe", wintypes.WCHAR * 260)]

class MemoryCounters(ctypes.Structure):
    _fields_ = [("size", wintypes.DWORD), ("page_faults", wintypes.DWORD),
                ("peak_working_set", ctypes.c_size_t), ("working_set", ctypes.c_size_t),
                ("quota_peak_paged", ctypes.c_size_t), ("quota_paged", ctypes.c_size_t),
                ("quota_peak_nonpaged", ctypes.c_size_t), ("quota_nonpaged", ctypes.c_size_t),
                ("pagefile", ctypes.c_size_t), ("peak_pagefile", ctypes.c_size_t),
                ("private_usage", ctypes.c_size_t)]

KERNEL.CreateToolhelp32Snapshot.argtypes = [wintypes.DWORD, wintypes.DWORD]
KERNEL.CreateToolhelp32Snapshot.restype = wintypes.HANDLE
KERNEL.Process32FirstW.argtypes = [wintypes.HANDLE, ctypes.POINTER(ProcessEntry)]
KERNEL.Process32NextW.argtypes = [wintypes.HANDLE, ctypes.POINTER(ProcessEntry)]
KERNEL.OpenProcess.argtypes = [wintypes.DWORD, wintypes.BOOL, wintypes.DWORD]
KERNEL.OpenProcess.restype = wintypes.HANDLE
KERNEL.CloseHandle.argtypes = [wintypes.HANDLE]
PSAPI.GetProcessMemoryInfo.argtypes = [wintypes.HANDLE, ctypes.POINTER(MemoryCounters), wintypes.DWORD]
KERNEL.GetProcessTimes.argtypes = [wintypes.HANDLE, ctypes.POINTER(wintypes.FILETIME), ctypes.POINTER(wintypes.FILETIME), ctypes.POINTER(wintypes.FILETIME), ctypes.POINTER(wintypes.FILETIME)]

def owned_model_memory():
    snapshot = KERNEL.CreateToolhelp32Snapshot(2, 0)
    entries = []
    entry = ProcessEntry()
    entry.size = ctypes.sizeof(entry)
    try:
        ok = KERNEL.Process32FirstW(snapshot, ctypes.byref(entry))
        while ok:
            entries.append((entry.pid, entry.parent, entry.exe))
            ok = KERNEL.Process32NextW(snapshot, ctypes.byref(entry))
    finally:
        KERNEL.CloseHandle(snapshot)
    descendants = {os.getpid()}
    while True:
        found = descendants | {pid for pid, parent, _ in entries if parent in descendants}
        if found == descendants:
            break
        descendants = found
    processes = []
    for pid, _, exe in entries:
        if pid not in descendants or exe.lower() != "llama-server.exe":
            continue
        handle = KERNEL.OpenProcess(0x410, False, pid)
        if not handle:
            continue
        try:
            counters = MemoryCounters()
            counters.size = ctypes.sizeof(counters)
            if PSAPI.GetProcessMemoryInfo(handle, ctypes.byref(counters), counters.size):
                created, exited, kernel, user = (wintypes.FILETIME() for _ in range(4))
                KERNEL.GetProcessTimes(handle, ctypes.byref(created), ctypes.byref(exited), ctypes.byref(kernel), ctypes.byref(user))
                cpu_seconds = sum((value.dwHighDateTime << 32) + value.dwLowDateTime for value in (kernel, user)) / 10000000
                processes.append({"pid": pid, "working_set_mib": round(counters.working_set / MIB, 2),
                                  "private_commit_mib": round(counters.private_usage / MIB, 2),
                                  "cpu_seconds": round(cpu_seconds, 4),
                                  "os_lifetime_peak_working_set_mib": round(counters.peak_working_set / MIB, 2)})
        finally:
            KERNEL.CloseHandle(handle)
    return {"processes": processes,
            "working_set_mib": round(sum(p["working_set_mib"] for p in processes), 2),
            "private_commit_mib": round(sum(p["private_commit_mib"] for p in processes), 2)}

def gpu_memory():
    value = subprocess.check_output(["nvidia-smi", "--query-gpu=memory.used,memory.total", "--format=csv,noheader,nounits"],
                                    text=True, creationflags=subprocess.CREATE_NO_WINDOW).strip()
    used, total = value.splitlines()[0].split(",")
    return {"whole_gpu_used_mib": int(used), "whole_gpu_total_mib": int(total)}

class Sampler:
    def __init__(self):
        self.started = time.perf_counter()
        self.stage = "baseline"
        self.samples = []
        self.stop_event = threading.Event()
        self.thread = threading.Thread(target=self.run, daemon=True)
        self.thread.start()

    def run(self):
        next_gpu = 0
        gpu = {}
        while not self.stop_event.is_set():
            now = time.perf_counter()
            if now >= next_gpu:
                gpu = gpu_memory()
                next_gpu = now + 0.5
            self.samples.append({"elapsed_seconds": round(now - self.started, 3), "stage": self.stage,
                                 **owned_model_memory(), **gpu})
            self.stop_event.wait(0.2)

    def set_stage(self, stage):
        self.stage = stage

    def snapshot(self):
        return {**owned_model_memory(), **gpu_memory()}

    def close(self):
        self.stop_event.set()
        self.thread.join(timeout=5)

    def summary(self):
        result = {}
        for stage in dict.fromkeys(sample["stage"] for sample in self.samples):
            rows = [row for row in self.samples if row["stage"] == stage]
            result[stage] = {"samples": len(rows), "sampled_max_working_set_mib": max(row["working_set_mib"] for row in rows),
                             "sampled_max_private_commit_mib": max(row["private_commit_mib"] for row in rows),
                             "sampled_max_whole_gpu_used_mib": max(row["whole_gpu_used_mib"] for row in rows)}
        return result

def load_cases():
    seen = set()
    short, long = [], []
    directory = ROOT / "tests/fixtures/dictation"
    for name in ("cases.text.jsonl", "resource-cases.jsonl"):
        for line in (directory / name).read_text(encoding="utf-8").splitlines():
            if not line.strip():
                continue
            row = json.loads(line)
            if row["text"] in seen:
                continue
            seen.add(row["text"])
            (long if row.get("mode") == "notes" else short).append(row)
    return short, long

def messages_for(text, mode, without_example=False):
    messages = build_messages(text, mode, [])
    return [messages[0], messages[-1]] if without_example else messages


def generate(server, text, mode, context, without_example=False):
    body = {"messages": messages_for(text, mode, without_example), "stream": False,
            "chat_template_kwargs": {"enable_thinking": False}, "reasoning_effort": "none",
            "temperature": 0, "seed": 42, "cache_prompt": False}
    before_cpu = sum(row["cpu_seconds"] for row in owned_model_memory()["processes"])
    started = time.perf_counter()
    prompt = server.request("/apply-template", body, timeout=5)["prompt"]
    prompt_tokens = len(server.request("/tokenize", {"content": prompt, "add_special": True}, timeout=5)["tokens"])
    source_tokens = len(server.request("/tokenize", {"content": text}, timeout=5)["tokens"])
    output_tokens = max(256, int(source_tokens * 1.5) + 128)
    if output_tokens > 3072 or prompt_tokens + output_tokens + 32 > context:
        raise ValueError("Text exceeds existing complete-output context budget")
    body["max_tokens"] = output_tokens
    result = server.request("/v1/chat/completions", body, timeout=120)
    wall = time.perf_counter() - started
    cpu = sum(row["cpu_seconds"] for row in owned_model_memory()["processes"]) - before_cpu
    choice = result["choices"][0]
    return {"wall_seconds": round(wall, 3), "cpu_seconds": round(cpu, 4), "average_busy_logical_cores": round(cpu / wall, 3), "prompt_tokens": prompt_tokens,
            "source_tokens": source_tokens, "text": choice["message"].get("content"),
            "finish_reason": choice.get("finish_reason"), "usage": result.get("usage"),
            "timings": result.get("timings")}

def save(report, output, sampler):
    report["resource_stage_summary"] = sampler.summary()
    report["resource_samples"] = sampler.samples
    if "text_pid" in report:
        memory = [process for row in sampler.samples for process in row["processes"] if process["pid"] == report["text_pid"]]
        if memory:
            report["text_process_sampled_max"] = {
                "working_set_mib": max(row["working_set_mib"] for row in memory),
                "private_commit_mib": max(row["private_commit_mib"] for row in memory),
                "os_lifetime_peak_working_set_mib": max(row["os_lifetime_peak_working_set_mib"] for row in memory)}
    output.write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, help="New JSON report path; an existing file is never overwritten.")
    parser.add_argument("--asr", choices=["none", "0.6b", "1.7b"], default="1.7b")
    parser.add_argument("--text-model", type=Path, help="Existing GGUF with adjacent manifest.json; omit to measure only ASR.")
    parser.add_argument("--device", choices=["cpu", "cuda"], default="cpu")
    parser.add_argument("--gpu-layers", type=int, default=0)
    parser.add_argument("--context", type=int, default=8192)
    parser.add_argument("--case-limit", type=int, default=0)
    parser.add_argument("--without-example", action="store_true", help="Single-factor ablation: remove only the middle user/assistant example.")
    parser.add_argument("--validate-cases", action="store_true", help="Validate fixed inputs without loading models or querying hardware.")
    args = parser.parse_args()
    short_cases, long_cases = load_cases()
    if args.validate_cases:
        print(json.dumps({"short_texts": len(short_cases), "long_notes": len(long_cases),
                          "input_sha256": hashlib.sha256(json.dumps(
                              [(row["id"], row["text"]) for row in short_cases + long_cases],
                              ensure_ascii=False).encode()).hexdigest()}, indent=2))
        return
    if args.output is None:
        parser.error("--output is required for a benchmark run")
    if args.output.exists():
        raise FileExistsError(args.output)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    report = {"started_utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
              "scope": "14 distinct short development text cases; not real-audio technical accuracy or frozen acceptance",
              "host": platform.platform(), "asr": args.asr, "device": args.device,
              "gpu_layers": args.gpu_layers, "context_tokens": args.context, "threads": 4,
              "terms": [], "cases": [], "prompt_sha256": hashlib.sha256(json.dumps(
                  {mode: messages_for("", mode, args.without_example) for mode in ("clean", "notes")},
                  ensure_ascii=False, sort_keys=True).encode()).hexdigest(),
              "measurement": "Owned llama-server process trees only; working set includes mapped/shared pages and is not additive physical memory. Private commit is not resident RAM. Process sampling ~200 ms plus GPU query cost; whole-card GPU readings refreshed ~500 ms and include other apps. Sample maxima are not continuous hardware peaks. Load is OS-cache-dependent, not reboot-cold. Request timeout 120 seconds; app clean/notes limits are 25/90 seconds.",
              "runtime_manifest": json.loads((ROOT / ".runtime/llama/release.json").read_text(encoding="utf-8-sig"))}
    report["benchmark_script_sha256"] = hashlib.sha256(Path(__file__).read_bytes()).hexdigest()
    report["prompt_variant"] = "without_example" if args.without_example else "production_boundary_v2"
    sampler = Sampler()
    asr = None
    text_server = None
    try:
        time.sleep(1)
        report["baseline_snapshot"] = sampler.snapshot()
        if args.asr != "none":
            from llama_asr import LlamaASR
            import soundfile as sf
            sampler.set_stage("asr_load")
            started = time.perf_counter()
            asr = LlamaASR("qwen-llama-" + args.asr)
            report["asr_pid"] = asr.child.pid
            report["asr_load_seconds"] = round(time.perf_counter() - started, 3)
            sampler.set_stage("asr_resident")
            time.sleep(1)
            report["asr_resident_snapshot"] = sampler.snapshot()
            samples, rate = sf.read(ROOT / "tests/fixtures/ascend-01219.wav", dtype="float32")
            assert rate == 16000
            sampler.set_stage("asr_inference")
            started = time.perf_counter()
            report["public_audio"] = asr.transcribe(samples, "auto", terms=[])
            report["asr_inference_seconds"] = round(time.perf_counter() - started, 3)
            sampler.set_stage("asr_after_inference_idle")
            time.sleep(1)
            report["asr_after_inference_snapshot"] = sampler.snapshot()
            print(json.dumps({"asr": args.asr, "load_seconds": report["asr_load_seconds"], "snapshot": report["asr_after_inference_snapshot"]}), flush=True)
        if args.text_model:
            model = args.text_model.resolve()
            report["text_model"] = str(model)
            report["model_bytes"] = model.stat().st_size
            report["model_manifest"] = json.loads(model.with_name("manifest.json").read_text(encoding="utf-8"))
            report["before_text_snapshot"] = sampler.snapshot()
            sampler.set_stage("text_load")
            started = time.perf_counter()
            text_server = LlamaServer([
                "-m", str(model), "--device", "CUDA0" if args.device == "cuda" else "none",
                "-ngl", str(args.gpu_layers), "-c", str(args.context), "-np", "1", "-b", "256", "-ub", "256",
                "-t", "4", "-tb", "4", "-fa", "on", "--fit", "off", "--cache-ram", "0", "--no-cache-idle-slots",
                "--no-context-shift", "--jinja", "--reasoning-format", "auto", "--log-disable"], load_timeout=180)
            report["text_pid"] = text_server.child.pid
            report["text_load_seconds"] = round(time.perf_counter() - started, 3)
            sampler.set_stage("text_resident")
            time.sleep(1)
            report["text_resident_snapshot"] = sampler.snapshot()
            cases = short_cases
            if args.case_limit:
                cases = cases[:args.case_limit]
            jobs = [(mode, case) for mode in ("clean", "notes") for case in cases]
            if not args.case_limit:
                jobs.extend(("notes", case) for case in long_cases)
            for mode, case in jobs:
                sampler.set_stage("text_inference_" + mode)
                try:
                    result = generate(text_server, case["text"], mode, args.context, args.without_example)
                    row = {**case, "mode": mode, "result": result,
                           "wall_seconds": result["wall_seconds"], "cpu_seconds": result["cpu_seconds"],
                           "average_busy_logical_cores": result["average_busy_logical_cores"]}
                    row["exceeds_app_timeout"] = result["wall_seconds"] > (25 if mode == "clean" else 90)
                except Exception as error:
                    row = {**case, "mode": mode, "error": str(error)}
                report["cases"].append(row)
                print(json.dumps({"model": model.name, "device": args.device, "id": case["id"], "mode": mode,
                                  "wall_seconds": row.get("wall_seconds"), "error": row.get("error"), "text": row.get("result", {}).get("text")}, ensure_ascii=False), flush=True)
                save(report, args.output, sampler)
                if "error" in row:
                    raise RuntimeError(row["error"])
            sampler.set_stage("both_idle_after_text")
            time.sleep(1)
            report["after_text_snapshot"] = sampler.snapshot()
            for mode in ("clean", "notes"):
                times = sorted(row["wall_seconds"] for row in report["cases"] if row["mode"] == mode and "wall_seconds" in row and row["id"] != "resource-long-notes-01")
                report[mode + "_latency"] = {"count": len(times), "median_seconds": statistics.median(times),
                                             "p95_nearest_rank_seconds": times[max(0, int(len(times) * .95 + .999999) - 1)],
                                             "max_seconds": max(times)}
        report["completed"] = True
    except Exception as error:
        report["error"] = str(error)
        raise
    finally:
        sampler.set_stage("cleanup")
        for server in (text_server, asr):
            if server:
                try:
                    server.close()
                except Exception as error:
                    report.setdefault("cleanup_errors", []).append(str(error))
        time.sleep(1)
        report["after_cleanup_snapshot"] = sampler.snapshot()
        sampler.close()
        save(report, args.output, sampler)
        print("REPORT " + str(args.output), flush=True)

if __name__ == "__main__":
    main()
